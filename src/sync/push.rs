// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    SyncError, SyncProgress, SyncWarnings, converted_dir, device_relative_path, file_stem_string,
    find_file_in_formats, pdb, reconcile_remote_track_ids, remote_db_dir, unique_path_batch,
};
use crate::bridge::TrackView;
use crate::ffmpeg;
use crate::format::SupportedFormat;
use crate::library::Library;
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
    pub pdb_skipped: u32,
    pub updated: u32,
    pub warnings: Vec<String>,
}

impl std::fmt::Display for SyncResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.synced == 0 && self.updated == 0 && self.skipped == 0 && self.pdb_skipped == 0 {
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
        if self.pdb_skipped > 0 {
            write!(f, ", {} skipped (PDB row too small)", self.pdb_skipped)?;
        }
        write!(f, ".")
    }
}

// ── Internal types ───────────────────────────────────────────────────────────

struct PreparedItem<'a> {
    track: &'a TrackView,
    src_file: &'a crate::bridge::TrackFileView,
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
    lib: &Library,
    dest_dir: &Path,
    config: &SyncConfig,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> Result<SyncResult, SyncError> {
    let warnings = SyncWarnings::new();

    let rdb_dir = remote_db_dir(dest_dir);
    std::fs::create_dir_all(&rdb_dir)?;
    let remote_lib = Library::open(&rdb_dir)?;

    let contents_dir = dest_dir.join("Contents");
    std::fs::create_dir_all(&contents_dir)?;

    let local_tracks = lib.all_tracks()?;

    reconcile_remote_track_ids(lib, &remote_lib)?;

    let remote_tracks = remote_lib.all_tracks()?;
    let remote_ids: HashSet<String> = remote_tracks.iter().map(|t| t.id.clone()).collect();

    let to_sync: Vec<&TrackView> = local_tracks
        .iter()
        .filter(|t| !remote_ids.contains(&t.id))
        .collect();

    let updated = update_remote_metadata(&local_tracks, &remote_lib, &remote_tracks)?;

    if to_sync.is_empty() {
        let pdb_skipped = pdb::generate_pdb(&remote_lib, dest_dir, &warnings)?;
        return Ok(SyncResult {
            synced: 0,
            converted: 0,
            skipped: 0,
            pdb_skipped,
            updated,
            warnings: warnings.into_vec(),
        });
    }

    // Seed dedup map with just the bare filenames already on the remote.
    let mut used_filenames: HashMap<String, u32> = remote_tracks
        .iter()
        .flat_map(|tv| tv.files.iter())
        .filter_map(|f| {
            device_relative_path(&f.file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|name| (name.to_owned(), 1u32))
        })
        .collect();
    let conv_dir = converted_dir();
    std::fs::create_dir_all(&conv_dir)?;

    let (prepared, mut skipped) =
        prepare_sync_items(&to_sync, config, &mut used_filenames, &conv_dir, &warnings);
    let convert_ok = convert_prepared_items(&prepared, config.jobs, on_progress, &warnings)?;
    let (synced, converted, copy_skipped) = copy_to_destination(
        &prepared,
        &convert_ok,
        &contents_dir,
        lib,
        &remote_lib,
        on_progress,
        &warnings,
    )?;
    skipped += copy_skipped;

    // === PDB Phase ===
    on_progress(SyncProgress {
        phase: "Writing database",
        current: 0,
        total: 1,
    });
    let pdb_skipped = pdb::generate_pdb(&remote_lib, dest_dir, &warnings)?;

    Ok(SyncResult {
        synced,
        converted,
        skipped,
        pdb_skipped,
        updated,
        warnings: warnings.into_vec(),
    })
}

// ── Internals ────────────────────────────────────────────────────────────────

/// Push local metadata changes to tracks already on the remote.
fn update_remote_metadata(
    local_tracks: &[TrackView],
    remote_lib: &Library,
    remote_tracks: &[TrackView],
) -> Result<u32, SyncError> {
    let remote_by_id: HashMap<&str, &TrackView> = remote_tracks
        .iter()
        .map(|tv| (tv.id.as_str(), tv))
        .collect();

    let mut updated = 0u32;
    for local in local_tracks {
        if let Some(remote) = remote_by_id.get(local.id.as_str())
            && metadata_differs(local, remote)
        {
            remote_lib.overwrite_track_fields(&local.id, local)?;
            updated += 1;
        }
    }
    Ok(updated)
}

/// Return `true` if any metadata field differs between `local` and `remote`.
fn metadata_differs(local: &TrackView, remote: &TrackView) -> bool {
    local.title != remote.title
        || local.artist != remote.artist
        || local.album != remote.album
        || local.genre != remote.genre
        || local.composer != remote.composer
        || local.label != remote.label
        || local.remixer != remote.remixer
        || local.key != remote.key
        || local.comment != remote.comment
        || local.isrc != remote.isrc
        || local.lyricist != remote.lyricist
        || local.mix_name != remote.mix_name
        || local.release_date != remote.release_date
        || local.tempo != remote.tempo
        || local.year != remote.year
        || local.track_number != remote.track_number
        || local.disc_number != remote.disc_number
        || local.rating != remote.rating
        || local.color != remote.color
}

/// For each track to sync, pick the best source file, assign a destination filename,
/// and determine whether a conversion is needed.
fn prepare_sync_items<'a>(
    to_sync: &[&'a TrackView],
    config: &SyncConfig,
    used_filenames: &mut HashMap<String, u32>,
    conv_dir: &Path,
    warnings: &SyncWarnings,
) -> (Vec<PreparedItem<'a>>, u32) {
    let mut prepared: Vec<PreparedItem> = Vec::new();
    let mut skipped = 0u32;
    let mut claimed_conv_paths: HashSet<PathBuf> = HashSet::new();

    for tv in to_sync {
        let matching_file = find_file_in_formats(&tv.files, &config.supported_formats);

        let (src_file, dest_format, needs_conversion) = if let Some(f) = matching_file {
            let fmt = SupportedFormat::try_from(f.format.as_str()).unwrap();
            (f, fmt, false)
        } else if let Some(convert_to) = config.convert_to {
            let Some(f) = tv.files.first() else {
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
            warnings.push(format!("Source file not found: {}", src_file.file_path));
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
                format!("{original_stem}_{count}.{dest_ext}")
            }
        };

        let local_conv_path = if needs_conversion {
            let base = conv_dir.join(format!("{original_stem}.{dest_ext}"));
            unique_path_batch(&base, &mut claimed_conv_paths)
        } else {
            PathBuf::new()
        };

        prepared.push(PreparedItem {
            track: tv,
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
    warnings: &SyncWarnings,
) -> Result<Vec<bool>, SyncError> {
    let total = jobs.len();
    if total == 0 {
        return Ok(vec![]);
    }
    let total_u32 = u32::try_from(total)?;
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
                            warnings
                                .push(format!("Conversion failed for {}: {e}", job.src.display()));
                        }
                    }
                    let done = done_count.fetch_add(1, Ordering::Relaxed) + 1;
                    on_progress(SyncProgress {
                        phase: "Converting files",
                        current: done,
                        total: total_u32,
                    });
                }
            });
        }
    });

    Ok(results
        .into_iter()
        .map(|m| m.into_inner().unwrap())
        .collect())
}

/// Run parallel ffmpeg conversions for items that need them.
///
/// Returns a per-item boolean indicating whether the conversion succeeded.
fn convert_prepared_items(
    prepared: &[PreparedItem],
    jobs: usize,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
    warnings: &SyncWarnings,
) -> Result<Vec<bool>, SyncError> {
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
    let conv_results = run_conversions(&conv_jobs, jobs, on_progress, warnings)?;
    let mut convert_ok = vec![false; prepared.len()];
    for (job_idx, &prep_idx) in items_to_convert.iter().enumerate() {
        convert_ok[prep_idx] = conv_results[job_idx];
    }
    Ok(convert_ok)
}

/// Copy (or move converted) files to the destination and register them in the remote library.
///
/// Returns `(synced, converted, skipped)`.
fn copy_to_destination(
    prepared: &[PreparedItem],
    convert_ok: &[bool],
    contents_dir: &Path,
    local_lib: &Library,
    remote_lib: &Library,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
    warnings: &SyncWarnings,
) -> Result<(u32, u32, u32), SyncError> {
    let copy_total = u32::try_from(prepared.len())?;
    let mut synced = 0u32;
    let mut converted = 0u32;
    let mut skipped = 0u32;

    for (i, item) in prepared.iter().enumerate() {
        on_progress(SyncProgress {
            phase: "Copying files",
            current: u32::try_from(i + 1)?,
            total: copy_total,
        });

        let dest_path = contents_dir.join(&item.dest_filename);

        if item.needs_conversion {
            if !convert_ok[i] {
                skipped += 1;
                continue;
            }

            match local_lib.import_file_variant(
                &item.local_conv_path,
                &item.track.id,
                None,
                item.track,
            ) {
                Ok((_imported, import_warnings)) => {
                    for warning in import_warnings {
                        warnings.push(format!(
                            "Failed to register converted local file {}: {warning}",
                            item.local_conv_path.display()
                        ));
                    }
                }
                Err(e) => warnings.push(format!(
                    "Failed to register converted local file {}: {e}",
                    item.local_conv_path.display()
                )),
            }

            if let Err(e) = std::fs::copy(&item.local_conv_path, &dest_path) {
                warnings.push(format!("Copy failed for converted file: {e}"));
                skipped += 1;
                continue;
            }
            converted += 1;
        } else if let Err(e) = std::fs::copy(&item.src_path, &dest_path) {
            warnings.push(format!("Copy failed for {}: {e}", item.src_file.file_path));
            skipped += 1;
            continue;
        }

        // Import the copied file into the remote aoide library.
        match remote_lib.import_file_variant(
            &dest_path,
            &item.track.id,
            Some(item.dest_filename.clone()),
            item.track,
        ) {
            Ok((imported, import_warnings)) => {
                for warning in import_warnings {
                    warnings.push(format!(
                        "Failed to import into remote library {}: {warning}",
                        item.dest_filename
                    ));
                }
                if imported == 0 {
                    let _ = std::fs::remove_file(&dest_path);
                    skipped += 1;
                    continue;
                }
            }
            Err(e) => {
                let _ = std::fs::remove_file(&dest_path);
                warnings.push(format!("Failed to import into remote library: {e}"));
                skipped += 1;
                continue;
            }
        }

        synced += 1;
    }

    Ok((synced, converted, skipped))
}
