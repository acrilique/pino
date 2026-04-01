// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

mod import;
mod pdb;
mod pull;
mod push;

pub use import::{import_files, import_folder};
pub use pull::{PullResult, pull_from_remote};
pub use push::{SyncConfig, SyncResult, sync};

use crate::db::{Library, TrackFile};
use crate::ffmpeg;
use crate::format::SupportedFormat;
use crate::paths;
use lofty::prelude::*;
use lofty::tag::ItemKey;
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

// ── Warnings collector ───────────────────────────────────────────────────────

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
    Db(rusqlite::Error),
    Io(std::io::Error),
    Pdb(rekordcrate::Error),
    NoRemoteDb,
    Overflow,
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Db(e) => write!(f, "Database error: {e}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Pdb(e) => write!(f, "PDB generation failed: {e}"),
            Self::NoRemoteDb => write!(f, "No pino database found on this device"),
            Self::Overflow => write!(f, "Numeric value exceeds supported range"),
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

impl From<std::num::TryFromIntError> for SyncError {
    fn from(_: std::num::TryFromIntError) -> Self {
        Self::Overflow
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
pub fn check_sync_status(db_path: &Path, dest_dir: &Path) -> Result<SyncStatus, SyncError> {
    let local_db = Library::open(db_path)?;
    let local_ids: HashSet<String> = local_db.get_track_ids()?.into_iter().collect();

    let Some(remote_db) = open_remote_db(dest_dir)? else {
        return Ok(SyncStatus {
            to_push: u32::try_from(local_ids.len())?,
            to_pull: 0,
            has_remote_db: false,
        });
    };

    let remote_ids: HashSet<String> = remote_db.get_track_ids()?.into_iter().collect();

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

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

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
    files: &'a [TrackFile],
    formats: &[SupportedFormat],
) -> Option<&'a TrackFile> {
    files.iter().find(|f| {
        SupportedFormat::try_from(f.format.as_str()).is_ok_and(|fmt| formats.contains(&fmt))
    })
}

pub(crate) fn converted_dir() -> PathBuf {
    paths::data_dir().join("converted")
}

pub(crate) fn remote_db_path(dest_dir: &Path) -> PathBuf {
    dest_dir.join("PIONEER").join("pino").join("library.db")
}

pub(crate) fn open_remote_db(dest_dir: &Path) -> Result<Option<Library>, SyncError> {
    let path = remote_db_path(dest_dir);
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(Library::open(&path)?))
}

pub(crate) fn file_size_on_disk(path: &Path) -> u32 {
    std::fs::metadata(path)
        .map(|m| u32::try_from(m.len()).unwrap_or(u32::MAX))
        .unwrap_or(0)
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

// ── Metadata reading ─────────────────────────────────────────────────────────

pub(crate) struct AudioMeta {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub composer: String,
    pub label: String,
    pub remixer: String,
    pub key: String,
    pub comment: String,
    pub isrc: String,
    pub lyricist: String,
    pub mix_name: String,
    pub release_date: String,
    pub year: u16,
    pub track_number: u32,
    pub disc_number: u16,
    pub rating: u8,
    pub color: u8,
    pub artwork_path: String,
    pub duration_secs: u16,
    pub sample_rate: u32,
    pub bitrate: u32,
}

impl AudioMeta {
    /// Fill in zero properties from fallback values (useful for converted files).
    pub fn with_fallback_properties(mut self, sample_rate: u32, bitrate: u32) -> Self {
        if self.sample_rate == 0 {
            self.sample_rate = sample_rate;
        }
        if self.bitrate == 0 {
            self.bitrate = bitrate;
        }
        self
    }
}

/// Extract the first front-cover (or first available) picture from a lofty tag,
/// write it to the artwork cache directory using a content-hash filename, and
/// return the cache path. Returns an empty string if no picture is found.
fn extract_artwork(tag: Option<&lofty::tag::Tag>, warnings: &SyncWarnings) -> String {
    use lofty::picture::PictureType;
    use sha2::{Digest, Sha256};

    let Some(tag) = tag else { return String::new() };

    let pictures = tag.pictures();
    if pictures.is_empty() {
        return String::new();
    }

    // Prefer front cover, fall back to first available.
    let picture = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .unwrap_or(&pictures[0]);

    let data = picture.data();
    if data.is_empty() {
        return String::new();
    }

    let ext = picture
        .mime_type()
        .and_then(lofty::picture::MimeType::ext)
        .unwrap_or("jpg");

    let hash = {
        let digest = Sha256::digest(data);
        digest.iter().fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        })
    };
    let filename = format!("{hash}.{ext}");

    let art_dir = crate::paths::artwork_dir();
    if let Err(e) = std::fs::create_dir_all(&art_dir) {
        warnings.push(format!("Failed to create artwork dir: {e}"));
        return String::new();
    }

    let dest = art_dir.join(&filename);
    if dest.exists() {
        return dest.to_string_lossy().into_owned();
    }

    if let Err(e) = std::fs::write(&dest, data) {
        warnings.push(format!("Failed to write artwork {filename}: {e}"));
        return String::new();
    }

    dest.to_string_lossy().into_owned()
}

/// Read audio metadata from a file (lofty with ffprobe fallback).
#[allow(clippy::too_many_lines)]
pub(crate) fn read_metadata(
    path: &Path,
    fallback_title: &str,
    warnings: &SyncWarnings,
) -> AudioMeta {
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

            let get_str = |key: ItemKey| -> String {
                tag.and_then(|t| t.get_string(key).map(std::string::ToString::to_string))
                    .unwrap_or_default()
            };

            let genre = get_str(ItemKey::Genre);
            let composer = get_str(ItemKey::Composer);
            let label = get_str(ItemKey::Label);
            let remixer = get_str(ItemKey::Remixer);
            let key = get_str(ItemKey::InitialKey);
            let comment = get_str(ItemKey::Comment);
            let isrc = get_str(ItemKey::Isrc);
            let lyricist = get_str(ItemKey::Lyricist);
            let mix_name = get_str(ItemKey::ContentGroup);
            let release_date = get_str(ItemKey::ReleaseDate);

            let year = tag
                .and_then(|t| t.get_string(ItemKey::Year))
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(0);
            let track_number = tag.and_then(lofty::tag::Accessor::track).unwrap_or(0);
            let disc_number = tag
                .and_then(lofty::tag::Accessor::disk)
                .and_then(|d| u16::try_from(d).ok())
                .unwrap_or(0);
            let rating = tag
                .and_then(|t| t.get_string(ItemKey::Popularimeter))
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0);
            let color = tag
                .and_then(|t| t.get_string(ItemKey::Color))
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0);

            let properties = tagged_file.properties();

            let artwork_path = extract_artwork(tag, warnings);

            AudioMeta {
                title,
                artist,
                album,
                genre,
                composer,
                label,
                remixer,
                key,
                comment,
                isrc,
                lyricist,
                mix_name,
                release_date,
                year,
                track_number,
                disc_number,
                rating,
                color,
                artwork_path,
                duration_secs: u16::try_from(properties.duration().as_secs()).unwrap_or(u16::MAX),
                sample_rate: properties.sample_rate().unwrap_or(44100),
                bitrate: properties.overall_bitrate().unwrap_or(320),
            }
        }
        Err(e) => {
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            warnings.push(format!("lofty could not read metadata for {filename}: {e}"));
            match ffmpeg::probe_metadata(path) {
                Ok(meta) => {
                    warnings.push(format!("{filename}: using ffprobe metadata fallback"));
                    AudioMeta {
                        title: meta.title.unwrap_or_else(|| fallback_title.to_string()),
                        artist: meta.artist.unwrap_or_default(),
                        album: meta.album.unwrap_or_default(),
                        genre: meta.genre.unwrap_or_default(),
                        composer: String::new(),
                        label: String::new(),
                        remixer: String::new(),
                        key: String::new(),
                        comment: meta.comment.unwrap_or_default(),
                        isrc: String::new(),
                        lyricist: String::new(),
                        mix_name: String::new(),
                        release_date: String::new(),
                        year: meta.year,
                        track_number: meta.track_number,
                        disc_number: 0,
                        rating: 0,
                        color: 0,
                        artwork_path: String::new(),
                        duration_secs: meta.duration_secs,
                        sample_rate: meta.sample_rate,
                        bitrate: meta.bitrate,
                    }
                }
                Err(probe_err) => {
                    warnings.push(format!(
                        "{filename}: ffprobe fallback also failed ({probe_err}), using defaults. You might need to fill in the metadata manually."
                    ));
                    AudioMeta {
                        title: fallback_title.to_string(),
                        artist: String::new(),
                        album: String::new(),
                        genre: String::new(),
                        composer: String::new(),
                        label: String::new(),
                        remixer: String::new(),
                        key: String::new(),
                        comment: String::new(),
                        isrc: String::new(),
                        lyricist: String::new(),
                        mix_name: String::new(),
                        release_date: String::new(),
                        year: 0,
                        track_number: 0,
                        disc_number: 0,
                        rating: 0,
                        color: 0,
                        artwork_path: String::new(),
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
pub(crate) fn read_audio_properties(
    path: &Path,
    fallback_sample_rate: u32,
    fallback_bitrate: u32,
    warnings: &SyncWarnings,
) -> AudioMeta {
    read_metadata(path, &file_stem_string(path), warnings)
        .with_fallback_properties(fallback_sample_rate, fallback_bitrate)
}
