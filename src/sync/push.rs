// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use super::{
    SyncError, SyncProgress, converted_dir, file_size_on_disk, file_stem_string,
    find_file_in_formats, new_id, pdb, read_audio_properties, remote_db_path, today,
    unique_path_batch,
};
use crate::db::{Library, Track, TrackFile, TrackWithFiles};
use crate::ffmpeg;
use crate::format::SupportedFormat;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

// ── Public types ─────────────────────────────────────────────────────────────

pub struct SyncConfig {
    pub supported_formats: Vec<SupportedFormat>,
    /// Format to convert to when a track has no file in an allowed format. `None` = skip those.
    pub convert_to: Option<SupportedFormat>,
    pub jobs: usize,
}

pub struct SyncResult {
    pub synced: u32,
    pub converted: u32,
    pub skipped: u32,
    pub updated: u32,
}

impl std::fmt::Display for SyncResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.synced == 0 && self.updated == 0 && self.skipped == 0 {
            return write!(f, "Everything is up to date.");
        }
        write!(f, "Synced {} track(s)", self.synced)?;
        if self.converted > 0 {
            write!(f, " ({} converted)", self.converted)?;
        }
        if self.updated > 0 {
            write!(f, ", {} updated", self.updated)?;
        }
        if self.skipped > 0 {
            write!(f, ", {} skipped (no matching format)", self.skipped)?;
        }
        write!(f, ".")
    }
}

// ── Internal types ───────────────────────────────────────────────────────────

struct PreparedItem<'a> {
    track: &'a Track,
    src_file: &'a TrackFile,
    src_path: PathBuf,
    dest_filename: String,
    dest_format: SupportedFormat,
    needs_conversion: bool,
    local_conv_path: PathBuf,
}

struct ConvertJob<'a> {
    src: &'a Path,
    dest: &'a Path,
    format: SupportedFormat,
}

// ── Sync entry point ─────────────────────────────────────────────────────────

/// Synchronize local library to a remote destination (additive-only).
///
/// Runs in four phases:
/// 1. **Metadata update** — push local metadata changes to tracks already on the remote.
/// 2. **Converting files** — parallel conversion for tracks without a matching format.
/// 3. **Copying files** — sequential copy to destination + register in remote DB.
/// 4. **Writing database** — generate the Pioneer PDB.
pub fn sync(
    db_path: &Path,
    dest_dir: &Path,
    config: &SyncConfig,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> Result<SyncResult, SyncError> {
    if config.convert_to.is_some() && !ffmpeg::check_available() {
        return Err(SyncError::FfmpegNotFound);
    }

    let local_db = Library::open(db_path)?;

    let rdb_path = remote_db_path(dest_dir);
    if let Some(parent) = rdb_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let remote_db = Library::open(&rdb_path)?;

    let contents_dir = dest_dir.join("Contents");
    std::fs::create_dir_all(&contents_dir)?;

    let local_tracks = local_db.get_all_tracks_with_files()?;
    let remote_ids: HashSet<String> = remote_db.get_track_ids()?.into_iter().collect();

    let to_sync: Vec<&TrackWithFiles> = local_tracks
        .iter()
        .filter(|t| !remote_ids.contains(&t.track.id))
        .collect();

    let updated = update_remote_metadata(&local_tracks, &remote_db)?;

    if to_sync.is_empty() {
        pdb::generate_pdb(&remote_db, dest_dir)?;
        return Ok(SyncResult {
            synced: 0,
            converted: 0,
            skipped: 0,
            updated,
        });
    }

    let mut used_filenames: HashMap<String, u32> = remote_db
        .used_filenames()?
        .into_iter()
        .map(|name| (name, 1))
        .collect();
    let conv_dir = converted_dir();
    std::fs::create_dir_all(&conv_dir)?;

    let (prepared, mut skipped) =
        prepare_sync_items(&to_sync, config, &mut used_filenames, &conv_dir);
    let convert_ok = convert_prepared_items(&prepared, config.jobs, on_progress);
    let (synced, converted, copy_skipped) = copy_to_destination(
        &prepared,
        &convert_ok,
        &contents_dir,
        &local_db,
        &remote_db,
        on_progress,
    );
    skipped += copy_skipped;

    // === PDB Phase ===
    on_progress(SyncProgress {
        phase: "Writing database",
        current: 0,
        total: 1,
    });
    pdb::generate_pdb(&remote_db, dest_dir)?;

    Ok(SyncResult {
        synced,
        converted,
        skipped,
        updated,
    })
}

// ── Internals ────────────────────────────────────────────────────────────────

/// Push local metadata changes to tracks already on the remote.
fn update_remote_metadata(
    local_tracks: &[TrackWithFiles],
    remote_db: &Library,
) -> Result<u32, SyncError> {
    let remote_tracks = remote_db.get_all_tracks_with_files()?;
    let remote_by_id: HashMap<&str, &Track> = remote_tracks
        .iter()
        .map(|twf| (twf.track.id.as_str(), &twf.track))
        .collect();

    let mut updated = 0u32;
    for local_twf in local_tracks {
        let local = &local_twf.track;
        if let Some(remote) = remote_by_id.get(local.id.as_str())
            && (local.title != remote.title
                || local.artist != remote.artist
                || local.album != remote.album
                || local.tempo != remote.tempo)
        {
            remote_db
                .update_track(
                    &local.id,
                    &local.title,
                    &local.artist,
                    &local.album,
                    local.tempo,
                )
                .ok();
            updated += 1;
        }
    }
    Ok(updated)
}

/// For each track to sync, pick the best source file, assign a destination filename,
/// and determine whether a conversion is needed.
fn prepare_sync_items<'a>(
    to_sync: &[&'a TrackWithFiles],
    config: &SyncConfig,
    used_filenames: &mut HashMap<String, u32>,
    conv_dir: &Path,
) -> (Vec<PreparedItem<'a>>, u32) {
    let mut prepared: Vec<PreparedItem> = Vec::new();
    let mut skipped = 0u32;
    let mut claimed_conv_paths: HashSet<PathBuf> = HashSet::new();

    for twf in to_sync {
        let track = &twf.track;

        let matching_file = find_file_in_formats(&twf.files, &config.supported_formats);

        let (src_file, dest_format, needs_conversion) = if let Some(f) = matching_file {
            let fmt = SupportedFormat::try_from(f.format.as_str()).unwrap();
            (f, fmt, false)
        } else if let Some(convert_to) = config.convert_to {
            let Some(f) = twf.files.first() else {
                skipped += 1;
                continue;
            };
            (f, convert_to, true)
        } else {
            skipped += 1;
            continue;
        };

        let src_path = PathBuf::from(&src_file.file_path);
        if !src_path.exists() {
            eprintln!("  Warning: source file not found: {}", src_file.file_path);
            skipped += 1;
            continue;
        }

        let dest_ext: &str = dest_format.into();
        let original_stem = file_stem_string(&src_path);

        let dest_filename = {
            let base = format!("{original_stem}.{dest_ext}");
            let count = used_filenames.entry(base.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                base
            } else {
                format!("{original_stem}_{}.{dest_ext}", count)
            }
        };

        let local_conv_path = if needs_conversion {
            let base = conv_dir.join(format!("{original_stem}.{dest_ext}"));
            unique_path_batch(&base, &mut claimed_conv_paths)
        } else {
            PathBuf::new()
        };

        prepared.push(PreparedItem {
            track,
            src_file,
            src_path,
            dest_filename,
            dest_format,
            needs_conversion,
            local_conv_path,
        });
    }

    (prepared, skipped)
}

/// Run ffmpeg conversions in parallel, returning which jobs succeeded.
fn run_conversions(
    jobs: &[ConvertJob],
    num_workers: usize,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> Vec<bool> {
    let total = jobs.len();
    if total == 0 {
        return vec![];
    }
    let results: Vec<std::sync::Mutex<bool>> =
        (0..total).map(|_| std::sync::Mutex::new(false)).collect();
    let next_idx = AtomicUsize::new(0);
    let done_count = AtomicU32::new(0);

    std::thread::scope(|s| {
        let workers = num_workers.min(total).max(1);
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let idx = next_idx.fetch_add(1, Ordering::Relaxed);
                    if idx >= total {
                        break;
                    }
                    let job = &jobs[idx];
                    match ffmpeg::convert(job.src, job.dest, job.format) {
                        Ok(()) => *results[idx].lock().unwrap() = true,
                        Err(e) => {
                            eprintln!(
                                "  Warning: conversion failed for {}: {e}",
                                job.src.display()
                            );
                        }
                    }
                    let done = done_count.fetch_add(1, Ordering::Relaxed) + 1;
                    on_progress(SyncProgress {
                        phase: "Converting files",
                        current: done,
                        total: total as u32,
                    });
                }
            });
        }
    });

    results
        .into_iter()
        .map(|m| m.into_inner().unwrap())
        .collect()
}

/// Run parallel ffmpeg conversions for items that need them.
///
/// Returns a per-item boolean indicating whether the conversion succeeded.
fn convert_prepared_items(
    prepared: &[PreparedItem],
    jobs: usize,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> Vec<bool> {
    let items_to_convert: Vec<usize> = prepared
        .iter()
        .enumerate()
        .filter(|(_, p)| p.needs_conversion)
        .map(|(i, _)| i)
        .collect();
    let conv_jobs: Vec<ConvertJob> = items_to_convert
        .iter()
        .map(|&i| ConvertJob {
            src: &prepared[i].src_path,
            dest: &prepared[i].local_conv_path,
            format: prepared[i].dest_format,
        })
        .collect();
    let conv_results = run_conversions(&conv_jobs, jobs, on_progress);
    let mut convert_ok = vec![false; prepared.len()];
    for (job_idx, &prep_idx) in items_to_convert.iter().enumerate() {
        convert_ok[prep_idx] = conv_results[job_idx];
    }
    convert_ok
}

/// Copy (or move converted) files to the destination and register them in both databases.
///
/// Returns `(synced, converted, skipped)`.
fn copy_to_destination(
    prepared: &[PreparedItem],
    convert_ok: &[bool],
    contents_dir: &Path,
    local_db: &Library,
    remote_db: &Library,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> (u32, u32, u32) {
    let today = today();
    let copy_total = prepared.len() as u32;
    let mut synced = 0u32;
    let mut converted = 0u32;
    let mut skipped = 0u32;

    for (i, item) in prepared.iter().enumerate() {
        on_progress(SyncProgress {
            phase: "Copying files",
            current: (i + 1) as u32,
            total: copy_total,
        });

        let dest_path = contents_dir.join(&item.dest_filename);
        let dest_ext: &str = item.dest_format.into();

        let (sample_rate, bitrate);
        if item.needs_conversion {
            if !convert_ok[i] {
                skipped += 1;
                continue;
            }

            let conv_meta = read_audio_properties(
                &item.local_conv_path,
                item.src_file.sample_rate,
                item.src_file.bitrate,
            );
            sample_rate = conv_meta.sample_rate;
            bitrate = conv_meta.bitrate;

            // Register converted file in local DB.
            let local_file = TrackFile {
                id: new_id(),
                track_id: item.track.id.clone(),
                format: dest_ext.to_string(),
                file_path: item.local_conv_path.to_string_lossy().to_string(),
                file_size: file_size_on_disk(&item.local_conv_path),
                sample_rate,
                bitrate,
                added_at: today.clone(),
            };
            local_db.add_file(&local_file).ok();

            // Copy converted file to destination.
            if let Err(e) = std::fs::copy(&item.local_conv_path, &dest_path) {
                eprintln!("  Warning: copy failed for converted file: {e}");
                skipped += 1;
                continue;
            }
            converted += 1;
        } else {
            sample_rate = item.src_file.sample_rate;
            bitrate = item.src_file.bitrate;

            if let Err(e) = std::fs::copy(&item.src_path, &dest_path) {
                eprintln!(
                    "  Warning: copy failed for {}: {e}",
                    item.src_file.file_path
                );
                skipped += 1;
                continue;
            }
        }

        let file_size = file_size_on_disk(&dest_path);

        // Write track + file to remote DB.
        let remote_track = Track {
            id: item.track.id.clone(),
            title: item.track.title.clone(),
            artist: item.track.artist.clone(),
            album: item.track.album.clone(),
            duration_secs: item.track.duration_secs,
            tempo: item.track.tempo,
            added_at: today.clone(),
        };

        let remote_file = TrackFile {
            id: new_id(),
            track_id: item.track.id.clone(),
            format: dest_ext.to_string(),
            file_path: item.dest_filename.clone(),
            file_size,
            sample_rate,
            bitrate,
            added_at: today.clone(),
        };

        remote_db.add_track(&remote_track).ok();
        if let Err(e) = remote_db.add_file(&remote_file) {
            eprintln!("  Warning: failed to add file to remote DB: {e}");
            continue;
        }

        synced += 1;
    }

    (synced, converted, skipped)
}
