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
use std::collections::HashSet;
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
    let local_ids: HashSet<String> = lib.track_ids()?.into_iter().collect();

    let Some(remote_lib) = open_remote_library(dest_dir)? else {
        return Ok(SyncStatus {
            to_push: u32::try_from(local_ids.len())?,
            to_pull: 0,
            has_remote_db: false,
        });
    };

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
    files.iter().find(|f| {
        SupportedFormat::try_from(f.format.as_str()).is_ok_and(|fmt| formats.contains(&fmt))
    })
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

pub(crate) fn unique_path(path: &Path) -> PathBuf {
    unique_path_batch(path, &mut HashSet::new())
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
