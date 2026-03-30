// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::db::{Library, Track, TrackFile, TrackWithFiles};
use crate::ffmpeg;
use crate::format::SupportedFormat;
use crate::paths;
use crate::scan;
use lofty::prelude::*;
use rekordcrate::pdb::io::Database;
use rekordcrate::pdb::string::DeviceSQLString;
use rekordcrate::pdb::*;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

#[derive(Debug)]
pub enum SyncError {
    Db(rusqlite::Error),
    Io(std::io::Error),
    Pdb(rekordcrate::Error),
    FfmpegNotFound,
    NoRemoteDb,
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Db(e) => write!(f, "Database error: {e}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Pdb(e) => write!(f, "PDB generation failed: {e}"),
            Self::FfmpegNotFound => {
                write!(
                    f,
                    "ffmpeg is required for audio conversion but was not found in PATH"
                )
            }
            Self::NoRemoteDb => write!(f, "No pino database found on this device"),
        }
    }
}

impl From<rusqlite::Error> for SyncError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e)
    }
}

impl From<std::io::Error> for SyncError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<rekordcrate::Error> for SyncError {
    fn from(e: rekordcrate::Error) -> Self {
        Self::Pdb(e)
    }
}

impl From<rekordcrate::pdb::string::StringError> for SyncError {
    fn from(e: rekordcrate::pdb::string::StringError) -> Self {
        Self::Pdb(e.into())
    }
}

pub struct SyncConfig {
    pub supported_formats: Vec<SupportedFormat>,
    /// Format to convert to when a track has no file in an allowed format. `None` = skip those.
    pub convert_to: Option<SupportedFormat>,
    pub jobs: usize,
}

#[derive(Clone)]
pub struct SyncProgress {
    pub phase: &'static str,
    pub current: u32,
    pub total: u32,
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

/// Directory where pino stores locally converted files.
fn converted_dir() -> PathBuf {
    paths::data_dir().join("converted")
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn file_stem_string(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn find_file_in_formats<'a>(
    files: &'a [TrackFile],
    formats: &[SupportedFormat],
) -> Option<&'a TrackFile> {
    files.iter().find(|f| {
        SupportedFormat::try_from(f.format.as_str()).is_ok_and(|fmt| formats.contains(&fmt))
    })
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn remote_db_path(dest_dir: &Path) -> PathBuf {
    dest_dir.join("PIONEER").join("pino").join("library.db")
}

fn open_remote_db(dest_dir: &Path) -> Result<Option<Library>, SyncError> {
    let path = remote_db_path(dest_dir);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(Library::open(&path)?))
}

fn file_size_on_disk(path: &Path) -> u32 {
    std::fs::metadata(path).map(|m| m.len() as u32).unwrap_or(0)
}

struct ConvertJob<'a> {
    src: &'a Path,
    dest: &'a Path,
    format: SupportedFormat,
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

/// Scan a folder and import all audio files into the local database.
///
/// Returns the number of newly imported tracks.
pub fn import_folder(db_path: &Path, input_dir: &Path) -> Result<u32, SyncError> {
    let db = Library::open(db_path)?;

    let all_formats = SupportedFormat::ALL.to_vec();
    let mut audio_files = Vec::new();
    scan::find_audio_files(input_dir, &all_formats, true, &mut audio_files)?;
    audio_files.sort_by(|(a, _), (b, _)| a.cmp(b));

    let mut imported = 0u32;
    let today = today();

    for (src_path, src_format) in &audio_files {
        let path_str = src_path.to_string_lossy().to_string();
        if db.has_file_path(&path_str).unwrap_or(false) {
            continue;
        }

        let original_stem = file_stem_string(src_path);

        let format_str: String = match src_format {
            Some(fmt) => <SupportedFormat as Into<&str>>::into(*fmt).to_string(),
            None => src_path
                .extension()
                .unwrap_or_default()
                .to_str()
                .unwrap_or("")
                .to_ascii_lowercase(),
        };

        let meta = read_metadata(src_path, &original_stem);

        let file_size = file_size_on_disk(src_path);

        let track_id = new_id();
        let track = Track {
            id: track_id.clone(),
            title: meta.title,
            artist: meta.artist,
            album: meta.album,
            duration_secs: meta.duration_secs,
            tempo: 0,
            added_at: today.clone(),
        };

        let file = TrackFile {
            id: new_id(),
            track_id,
            format: format_str,
            file_path: path_str,
            file_size,
            sample_rate: meta.sample_rate,
            bitrate: meta.bitrate,
            added_at: today.clone(),
        };

        if let Err(e) = db.add_track(&track) {
            eprintln!("  Warning: failed to add track: {e}");
            continue;
        }
        if let Err(e) = db.add_file(&file) {
            eprintln!("  Warning: failed to add file: {e}");
            continue;
        }
        imported += 1;
    }

    Ok(imported)
}

struct PreparedItem<'a> {
    track: &'a Track,
    src_file: &'a TrackFile,
    src_path: PathBuf,
    dest_filename: String,
    dest_format: SupportedFormat,
    needs_conversion: bool,
    local_conv_path: PathBuf,
}

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
        generate_pdb(&remote_db, dest_dir)?;
        return Ok(SyncResult {
            synced: 0,
            converted: 0,
            skipped: 0,
            updated,
        });
    }

    let mut used_filenames = remote_db.used_filenames()?;
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
    generate_pdb(&remote_db, dest_dir)?;

    Ok(SyncResult {
        synced,
        converted,
        skipped,
        updated,
    })
}

/// Count how many local tracks have no file in any of the given formats.
#[allow(dead_code)]
pub fn count_needing_conversion(
    db_path: &Path,
    formats: &[SupportedFormat],
) -> Result<u32, SyncError> {
    let db = Library::open(db_path)?;
    let tracks = db.get_all_tracks_with_files()?;

    let count = tracks
        .iter()
        .filter(|twf| find_file_in_formats(&twf.files, formats).is_none())
        .count() as u32;

    Ok(count)
}

/// Counts of tracks that need pushing (local-only) and pulling (remote-only).
#[derive(Clone, Copy, Default, PartialEq)]
pub struct SyncStatus {
    pub to_push: u32,
    pub to_pull: u32,
    pub has_remote_db: bool,
}

/// Check sync status for a destination: how many tracks to push and pull.
pub fn check_sync_status(db_path: &Path, dest_dir: &Path) -> Result<SyncStatus, SyncError> {
    let local_db = Library::open(db_path)?;
    let local_ids: HashSet<String> = local_db.get_track_ids()?.into_iter().collect();

    let Some(remote_db) = open_remote_db(dest_dir)? else {
        return Ok(SyncStatus {
            to_push: local_ids.len() as u32,
            to_pull: 0,
            has_remote_db: false,
        });
    };

    let remote_ids: HashSet<String> = remote_db.get_track_ids()?.into_iter().collect();

    let to_push = local_ids
        .iter()
        .filter(|id| !remote_ids.contains(*id))
        .count() as u32;
    let to_pull = remote_ids
        .iter()
        .filter(|id| !local_ids.contains(*id))
        .count() as u32;

    Ok(SyncStatus {
        to_push,
        to_pull,
        has_remote_db: true,
    })
}

/// Import tracks from the remote (USB) into the local library.
///
/// For each track on the remote that doesn't exist locally, copies the audio file from
/// `<USB>/Contents/` to `~/.local/share/pino/imported/` and registers it in the local DB.
pub fn pull_from_remote(
    db_path: &Path,
    dest_dir: &Path,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> Result<u32, SyncError> {
    let local_db = Library::open(db_path)?;
    let remote_db = open_remote_db(dest_dir)?.ok_or(SyncError::NoRemoteDb)?;

    let local_ids: HashSet<String> = local_db.get_track_ids()?.into_iter().collect();
    let remote_tracks = remote_db.get_all_tracks_with_files()?;

    let to_pull: Vec<&TrackWithFiles> = remote_tracks
        .iter()
        .filter(|twf| !local_ids.contains(&twf.track.id))
        .collect();

    if to_pull.is_empty() {
        return Ok(0);
    }

    let import_dir = paths::data_dir().join("imported");
    std::fs::create_dir_all(&import_dir)?;

    let contents_dir = dest_dir.join("Contents");
    let today = today();
    let total = to_pull.len() as u32;
    let mut pulled = 0u32;

    for (i, twf) in to_pull.iter().enumerate() {
        on_progress(SyncProgress {
            phase: "Pulling from device",
            current: (i + 1) as u32,
            total,
        });

        // Insert the track metadata.
        if let Err(e) = local_db.add_track(&twf.track) {
            eprintln!("  Warning: failed to add track: {e}");
            continue;
        }

        // Copy each file from USB to the local imported directory.
        let mut any_file_ok = false;
        for remote_file in &twf.files {
            let src = contents_dir.join(&remote_file.file_path);
            if !src.exists() {
                eprintln!(
                    "  Warning: file not found on device: {}",
                    remote_file.file_path
                );
                continue;
            }

            let dest = import_dir.join(&remote_file.file_path);
            // Avoid overwriting existing files with the same name.
            let dest = if dest.exists() {
                unique_path(&dest)
            } else {
                dest
            };

            if let Err(e) = std::fs::copy(&src, &dest) {
                eprintln!("  Warning: copy failed: {e}");
                continue;
            }

            let local_file = TrackFile {
                id: new_id(),
                track_id: twf.track.id.clone(),
                format: remote_file.format.clone(),
                file_path: dest.to_string_lossy().to_string(),
                file_size: remote_file.file_size,
                sample_rate: remote_file.sample_rate,
                bitrate: remote_file.bitrate,
                added_at: today.clone(),
            };

            if let Err(e) = local_db.add_file(&local_file) {
                eprintln!("  Warning: failed to register file: {e}");
                continue;
            }
            any_file_ok = true;
        }

        if any_file_ok {
            pulled += 1;
        }
    }

    Ok(pulled)
}

/// Generate a unique file path by appending `_2`, `_3`, etc. if the path already exists.
#[allow(dead_code)]
fn unique_path(path: &Path) -> PathBuf {
    unique_path_batch(path, &mut HashSet::new())
}

/// Like `unique_path`, but also avoids collisions with paths already claimed in the current batch.
fn unique_path_batch(path: &Path, claimed: &mut HashSet<PathBuf>) -> PathBuf {
    if !path.exists() && !claimed.contains(path) {
        claimed.insert(path.to_path_buf());
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut n = 2u32;
    loop {
        let candidate = parent.join(format!("{stem}_{n}.{ext}"));
        if !candidate.exists() && !claimed.contains(&candidate) {
            claimed.insert(candidate.clone());
            return candidate;
        }
        n += 1;
    }
}

struct AudioMeta {
    title: String,
    artist: String,
    album: String,
    duration_secs: u16,
    sample_rate: u32,
    bitrate: u32,
}

/// Read audio metadata from a file (lofty with ffprobe fallback).
fn read_metadata(path: &Path, fallback_title: &str) -> AudioMeta {
    match lofty::read_from_path(path) {
        Ok(tagged_file) => {
            let tag = tagged_file
                .primary_tag()
                .or_else(|| tagged_file.first_tag());

            let title = tag
                .and_then(|t| t.title().map(|s| s.to_string()))
                .unwrap_or_else(|| fallback_title.to_string());
            let artist = tag
                .and_then(|t| t.artist().map(|s| s.to_string()))
                .unwrap_or_default();
            let album = tag
                .and_then(|t| t.album().map(|s| s.to_string()))
                .unwrap_or_default();

            let properties = tagged_file.properties();

            AudioMeta {
                title,
                artist,
                album,
                duration_secs: properties.duration().as_secs() as u16,
                sample_rate: properties.sample_rate().unwrap_or(44100),
                bitrate: properties.overall_bitrate().unwrap_or(320),
            }
        }
        Err(e) => {
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            eprintln!("  Warning: lofty could not read metadata for {filename}: {e}");
            match ffmpeg::probe_metadata(path) {
                Some(meta) => {
                    eprintln!("    Using ffprobe metadata fallback");
                    AudioMeta {
                        title: meta.title.unwrap_or_else(|| fallback_title.to_string()),
                        artist: meta.artist.unwrap_or_default(),
                        album: meta.album.unwrap_or_default(),
                        duration_secs: meta.duration_secs,
                        sample_rate: meta.sample_rate,
                        bitrate: meta.bitrate,
                    }
                }
                None => {
                    eprintln!("    ffprobe fallback also failed, using defaults");
                    AudioMeta {
                        title: fallback_title.to_string(),
                        artist: String::new(),
                        album: String::new(),
                        duration_secs: 0,
                        sample_rate: 44100,
                        bitrate: 0,
                    }
                }
            }
        }
    }
}

/// Read audio properties from a destination file (for converted tracks).
fn read_audio_properties(
    path: &Path,
    fallback_sample_rate: u32,
    fallback_bitrate: u32,
) -> AudioMeta {
    read_metadata(path, &file_stem_string(path))
        .with_fallback_properties(fallback_sample_rate, fallback_bitrate)
}

impl AudioMeta {
    /// Fill in zero properties from fallback values (useful for converted files).
    fn with_fallback_properties(mut self, sample_rate: u32, bitrate: u32) -> Self {
        if self.sample_rate == 0 {
            self.sample_rate = sample_rate;
        }
        if self.bitrate == 0 {
            self.bitrate = bitrate;
        }
        self
    }
}

/// Generate a Pioneer PDB database from all tracks in the given library.
fn generate_pdb(db: &Library, dest_dir: &Path) -> Result<(), SyncError> {
    let all = db.get_all_tracks_with_files()?;

    if all.is_empty() {
        return Ok(());
    }

    let rekordbox_dir = dest_dir.join("PIONEER").join("rekordbox");
    std::fs::create_dir_all(&rekordbox_dir)?;

    // Build artist and album maps with sequential IDs.
    let mut artist_map: HashMap<String, u32> = HashMap::new();
    let mut next_artist_id: u32 = 1;
    let mut album_map: HashMap<(String, u32), u32> = HashMap::new();
    let mut next_album_id: u32 = 1;

    for twf in &all {
        let track = &twf.track;
        if !track.artist.is_empty() && !artist_map.contains_key(&track.artist) {
            artist_map.insert(track.artist.clone(), next_artist_id);
            next_artist_id += 1;
        }
        if !track.album.is_empty() {
            let artist_id = if track.artist.is_empty() {
                0
            } else {
                artist_map[&track.artist]
            };
            let key = (track.album.clone(), artist_id);
            if let std::collections::hash_map::Entry::Vacant(entry) = album_map.entry(key) {
                entry.insert(next_album_id);
                next_album_id += 1;
            }
        }
    }

    // PDB table layout matching real rekordbox exports.
    let table_page_types = vec![
        PageType::Plain(PlainPageType::Tracks),
        PageType::Plain(PlainPageType::Genres),
        PageType::Plain(PlainPageType::Artists),
        PageType::Plain(PlainPageType::Albums),
        PageType::Plain(PlainPageType::Labels),
        PageType::Plain(PlainPageType::Keys),
        PageType::Plain(PlainPageType::Colors),
        PageType::Plain(PlainPageType::PlaylistTree),
        PageType::Plain(PlainPageType::PlaylistEntries),
        PageType::Unknown(9),
        PageType::Unknown(10),
        PageType::Plain(PlainPageType::HistoryPlaylists),
        PageType::Plain(PlainPageType::HistoryEntries),
        PageType::Plain(PlainPageType::Artwork),
        PageType::Unknown(14),
        PageType::Unknown(15),
        PageType::Plain(PlainPageType::Columns),
        PageType::Plain(PlainPageType::Menu),
        PageType::Unknown(18),
        PageType::Plain(PlainPageType::History),
    ];

    let pdb_path = rekordbox_dir.join("export.pdb");
    let pdb_file = File::create(&pdb_path)?;
    let mut pdb = Database::create(pdb_file, DatabaseType::Plain, &table_page_types)?;

    let mut exported_count: u32 = 0;
    let today = today();
    for twf in &all {
        let track = &twf.track;
        // Use the first file entry (remote DB has exactly one per track).
        let Some(file) = twf.files.first() else {
            continue;
        };

        let track_id = exported_count + 1;
        let artist_id = if track.artist.is_empty() {
            0
        } else {
            artist_map[&track.artist]
        };
        let album_id = if track.album.is_empty() {
            0
        } else {
            album_map[&(track.album.clone(), artist_id)]
        };

        let pioneer_path = format!("/Contents/{}", file.file_path);

        let Ok(file_type) = SupportedFormat::try_from(file.format.as_str()) else {
            eprintln!(
                "  Warning: unsupported format '{}' for PDB, skipping {}",
                file.format, file.file_path
            );
            continue;
        };

        let pdb_track = rekordcrate::pdb::Track::builder()
            .id(track_id)
            .title(track.title.parse()?)
            .artist_id(artist_id)
            .album_id(album_id)
            .file_path(pioneer_path.parse()?)
            .filename(file.file_path.parse()?)
            .sample_rate(file.sample_rate)
            .sample_depth(16)
            .bitrate(file.bitrate)
            .duration(track.duration_secs)
            .file_size(file.file_size)
            .file_type(file_type.into())
            .tempo(track.tempo)
            .autoload_hotcues("ON".parse()?)
            .date_added(today.parse()?)
            .build();

        match pdb.add_row(Row::Plain(PlainRow::Track(pdb_track))) {
            Ok(_) => exported_count += 1,
            Err(rekordcrate::Error::TrackRowTooSmall { .. }) => {
                eprintln!(
                    "  Warning: track '{}' skipped in PDB: row too small",
                    file.file_path
                );
                continue;
            }
            Err(err) => return Err(err.into()),
        }
    }

    // Insert artist rows.
    let mut artists_sorted: Vec<_> = artist_map.iter().collect();
    artists_sorted.sort_by_key(|&(_, &id)| id);
    for (name, &id) in artists_sorted {
        let artist = Artist::builder().id(id).name(name.parse()?).build();
        pdb.add_row(Row::Plain(PlainRow::Artist(artist)))?;
    }

    // Insert album rows.
    let mut albums_sorted: Vec<_> = album_map.iter().collect();
    albums_sorted.sort_by_key(|&(_, &id)| id);
    for ((album_name, artist_id), &id) in albums_sorted {
        let album = Album::builder()
            .id(id)
            .artist_id(*artist_id)
            .name(album_name.parse()?)
            .build();
        pdb.add_row(Row::Plain(PlainRow::Album(album)))?;
    }

    rekordcrate::pdb::defaults::insert_default_colors(&mut pdb)?;
    rekordcrate::pdb::defaults::insert_default_columns(&mut pdb)?;
    rekordcrate::pdb::defaults::insert_default_menus(&mut pdb)?;

    pdb.add_row(Row::Plain(PlainRow::History(History {
        subtype: Subtype(640),
        index_shift: 0,
        num_tracks: exported_count,
        date: today.parse()?,
        version: "1000".parse()?,
        label: DeviceSQLString::empty(),
    })))?;

    pdb.close()?;

    println!(
        "PDB: {} track(s), {} artist(s), {} album(s)",
        exported_count,
        artist_map.len(),
        album_map.len(),
    );

    Ok(())
}
