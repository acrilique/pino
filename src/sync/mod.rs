// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

mod import;
mod pdb;
mod pull;
mod push;

pub use import::{import_files, import_folder};
pub use pull::{PullResult, pull_from_remote};
pub use push::{SyncConfig, SyncResult, sync};

use crate::format::SupportedFormat;
use crate::library::Library;
use crate::paths;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Thread-safe collector for non-fatal warnings during sync operations.
#[derive(Default)]
pub struct SyncWarnings(Mutex<Vec<String>>);

impl SyncWarnings {
    pub fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    pub fn push(&self, msg: impl Into<String>) {
        self.0.lock().unwrap().push(msg.into());
    }

    pub fn into_vec(self) -> Vec<String> {
        self.0.into_inner().unwrap()
    }
}

// ── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SyncError {
    Library(String),
    Io(std::io::Error),
    Image(image::ImageError),
    Pdb(rekordcrate::Error),
    NoRemoteDb,
    Overflow,
    Other(String),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Library(e) => write!(f, "Library error: {e}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Image(e) => write!(f, "Image processing error: {e}"),
            Self::Pdb(e) => write!(f, "PDB generation failed: {e}"),
            Self::NoRemoteDb => write!(f, "No pino database found on this device"),
            Self::Overflow => write!(f, "Numeric value exceeds supported range"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl From<anyhow::Error> for SyncError {
    fn from(e: anyhow::Error) -> Self {
        Self::Library(e.to_string())
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

impl From<std::num::TryFromIntError> for SyncError {
    fn from(_: std::num::TryFromIntError) -> Self {
        Self::Overflow
    }
}

impl From<image::ImageError> for SyncError {
    fn from(e: image::ImageError) -> Self {
        Self::Image(e)
    }
}

// ── Shared types ─────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct SyncProgress {
    pub phase: &'static str,
    pub current: u32,
    pub total: u32,
}

/// Counts of tracks that need pushing (local-only) and pulling (remote-only).
#[derive(Clone, Copy, Default, PartialEq)]
pub struct SyncStatus {
    pub to_push: u32,
    pub to_pull: u32,
    pub has_remote_db: bool,
}

// ── Status check ─────────────────────────────────────────────────────────────

/// Check sync status for a destination: how many tracks to push and pull.
pub fn check_sync_status(lib: &Library, dest_dir: &Path) -> Result<SyncStatus, SyncError> {
    let Some(remote_lib) = open_remote_library(dest_dir)? else {
        let local_ids: HashSet<String> = lib.track_ids()?.into_iter().collect();
        return Ok(SyncStatus {
            to_push: u32::try_from(local_ids.len())?,
            to_pull: 0,
            has_remote_db: false,
        });
    };

    reconcile_remote_track_ids(lib, &remote_lib)?;

    let local_ids: HashSet<String> = lib.track_ids()?.into_iter().collect();

    let remote_ids: HashSet<String> = remote_lib.track_ids()?.into_iter().collect();

    let to_push = u32::try_from(
        local_ids
            .iter()
            .filter(|id| !remote_ids.contains(*id))
            .count(),
    )?;
    let to_pull = u32::try_from(
        remote_ids
            .iter()
            .filter(|id| !local_ids.contains(*id))
            .count(),
    )?;

    Ok(SyncStatus {
        to_push,
        to_pull,
        has_remote_db: true,
    })
}

// ── Internal helpers shared across submodules ────────────────────────────────

pub(crate) fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub(crate) fn file_stem_string(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn find_file_in_formats<'a>(
    files: &'a [crate::bridge::TrackFileView],
    formats: &[SupportedFormat],
) -> Option<&'a crate::bridge::TrackFileView> {
    formats.iter().find_map(|wanted| {
        files.iter().find(|file| {
            SupportedFormat::try_from(file.format.as_str()).is_ok_and(|fmt| fmt == *wanted)
        })
    })
}

pub(crate) fn device_relative_path(file_path: &str) -> PathBuf {
    let path = Path::new(file_path);
    if !path.is_absolute() {
        return path.to_path_buf();
    }

    let mut relative = PathBuf::new();
    let mut after_contents = false;
    for component in path.components() {
        if after_contents {
            relative.push(component.as_os_str());
        } else if component.as_os_str() == "Contents" {
            after_contents = true;
        }
    }

    if !relative.as_os_str().is_empty() {
        return relative;
    }

    path.file_name().map_or_else(PathBuf::new, PathBuf::from)
}

pub(crate) fn converted_dir() -> PathBuf {
    paths::data_dir().join("converted")
}

pub(crate) fn remote_db_dir(dest_dir: &Path) -> PathBuf {
    dest_dir.join("PIONEER").join("pino")
}

pub(crate) fn open_remote_library(dest_dir: &Path) -> Result<Option<Library>, SyncError> {
    let dir = remote_db_dir(dest_dir);
    let db_file = dir.join("aoide.sqlite");
    if !db_file.exists() {
        return Ok(None);
    }
    Ok(Some(Library::open(&dir)?))
}

pub(crate) fn reconcile_remote_track_ids(
    local_lib: &Library,
    remote_lib: &Library,
) -> Result<(), SyncError> {
    let local_tracks = local_lib.all_tracks()?;
    let remote_tracks = remote_lib.all_tracks()?;

    let mut local_by_key: HashMap<String, Vec<&crate::bridge::TrackView>> = HashMap::new();
    for local in &local_tracks {
        if let Some(key) = track_identity_key(local) {
            local_by_key.entry(key).or_default().push(local);
        }
    }

    let local_ids: HashSet<&str> = local_tracks.iter().map(|track| track.id.as_str()).collect();
    let mut adopted_ids = HashSet::new();

    for remote in &remote_tracks {
        if local_ids.contains(remote.id.as_str()) {
            continue;
        }

        let Some(key) = track_identity_key(remote) else {
            continue;
        };
        let Some(candidates) = local_by_key.get(&key) else {
            continue;
        };
        if candidates.len() != 1 {
            continue;
        }

        let local = candidates[0];
        if !adopted_ids.insert(local.id.clone()) {
            continue;
        }

        remote_lib.reassign_track_id(&remote.id, &local.id)?;
    }

    Ok(())
}

pub(crate) fn unique_path(path: &Path) -> PathBuf {
    unique_path_batch(path, &mut HashSet::new())
}

fn track_identity_key(track: &crate::bridge::TrackView) -> Option<String> {
    let normalize = |value: &str| value.trim().to_ascii_lowercase();

    if !track.isrc.is_empty() {
        return Some(format!(
            "isrc:{}:{}",
            normalize(&track.isrc),
            track.duration_secs
        ));
    }

    if track.title.is_empty() || track.artist.is_empty() || track.duration_secs == 0 {
        return None;
    }

    Some(format!(
        "meta:{}:{}:{}:{}:{}:{}:{}:{}",
        normalize(&track.title),
        normalize(&track.artist),
        normalize(&track.album),
        normalize(&track.mix_name),
        track.duration_secs,
        track.track_number,
        track.disc_number,
        track.year
    ))
}

pub(crate) fn unique_path_batch(path: &Path, claimed: &mut HashSet<PathBuf>) -> PathBuf {
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
