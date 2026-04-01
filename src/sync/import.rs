// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use super::{SyncError, SyncWarnings, file_size_on_disk, file_stem_string, read_metadata};
use crate::db::{Library, Track, TrackFile};
use crate::format::SupportedFormat;
use std::path::{Path, PathBuf};

fn is_known_audio_extension(ext: &str) -> bool {
    matches!(
        ext,
        "mp3" | "wav" | "aiff" | "aif" | "aac" | "m4a" | "flac" | "ogg" | "wma" | "opus"
    )
}

fn find_audio_files(
    dir: &Path,
    supported: &[SupportedFormat],
    convert: bool,
    files: &mut Vec<(PathBuf, Option<SupportedFormat>)>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            find_audio_files(&path, supported, convert, files)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            let ext_lower = ext.to_ascii_lowercase();
            if let Ok(fmt) = SupportedFormat::try_from(ext_lower.as_str()) {
                if supported.contains(&fmt) {
                    files.push((path, Some(fmt)));
                } else if convert {
                    files.push((path, None));
                }
            } else if convert && is_known_audio_extension(&ext_lower) {
                files.push((path, None));
            }
        }
    }
    Ok(())
}

/// Import result including count plus any warnings.
pub struct ImportResult {
    pub imported: u32,
    pub warnings: Vec<String>,
}

fn import_tracks(db: &Library, audio_files: &[(PathBuf, Option<SupportedFormat>)]) -> ImportResult {
    let mut imported = 0u32;
    let warnings = SyncWarnings::new();

    for (src_path, src_format) in audio_files {
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

        let meta = read_metadata(src_path, &original_stem, &warnings);

        let file_size = file_size_on_disk(src_path);

        let track_id = super::new_id();
        let track = Track {
            id: track_id.clone(),
            title: meta.title,
            artist: meta.artist,
            album: meta.album,
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
            duration_secs: meta.duration_secs,
            tempo: 0,
            year: 0,
            track_number: 0,
            disc_number: 0,
            rating: 0,
            color: 0,
            added_at: super::today(),
        };

        let file = TrackFile {
            id: super::new_id(),
            track_id,
            format: format_str,
            file_path: path_str,
            file_size,
            sample_rate: meta.sample_rate,
            bitrate: meta.bitrate,
            added_at: super::today(),
        };

        if let Err(e) = db.add_track(&track) {
            warnings.push(format!("Failed to add track: {e}"));
            continue;
        }
        if let Err(e) = db.add_file(&file) {
            warnings.push(format!("Failed to add file: {e}"));
            continue;
        }
        imported += 1;
    }

    ImportResult {
        imported,
        warnings: warnings.into_vec(),
    }
}

fn classify_file(path: PathBuf) -> Option<(PathBuf, Option<SupportedFormat>)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if let Ok(fmt) = SupportedFormat::try_from(ext.as_str()) {
        Some((path, Some(fmt)))
    } else if is_known_audio_extension(&ext) {
        Some((path, None))
    } else {
        None
    }
}

/// Scan a folder and import all audio files into the local database.
///
/// Returns the number of newly imported tracks.
pub fn import_folder(db_path: &Path, input_dir: &Path) -> Result<ImportResult, SyncError> {
    let db = Library::open(db_path)?;

    let all_formats = SupportedFormat::ALL.to_vec();
    let mut audio_files = Vec::new();
    find_audio_files(input_dir, &all_formats, true, &mut audio_files)?;
    audio_files.sort_by(|(a, _), (b, _)| a.cmp(b));

    Ok(import_tracks(&db, &audio_files))
}

/// Import specific audio files into the local database.
pub fn import_files(db_path: &Path, paths: Vec<PathBuf>) -> Result<ImportResult, SyncError> {
    let db = Library::open(db_path)?;

    let mut audio_files: Vec<_> = paths.into_iter().filter_map(classify_file).collect();
    audio_files.sort_by(|(a, _), (b, _)| a.cmp(b));

    Ok(import_tracks(&db, &audio_files))
}
