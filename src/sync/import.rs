// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::SyncError;
use crate::format::SupportedFormat;
use crate::library::Library;
use crate::sync::SyncProgress;
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

/// Import a list of audio file paths into the library using aoide-media-file.
fn import_tracks(
    lib: &Library,
    audio_files: &[(PathBuf, Option<SupportedFormat>)],
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> ImportResult {
    let paths: Vec<PathBuf> = audio_files.iter().map(|(p, _)| p.clone()).collect();

    let progress_cb = |current: u32, total: u32| {
        on_progress(SyncProgress {
            phase: "Importing tracks",
            current,
            total,
        });
    };

    match lib.import_files_with_progress(&paths, Some(&progress_cb)) {
        Ok((imported, warnings)) => ImportResult { imported, warnings },
        Err(e) => ImportResult {
            imported: 0,
            warnings: vec![format!("Import failed: {e}")],
        },
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

/// Scan a folder and import all audio files into the library.
pub fn import_folder(
    lib: &Library,
    input_dir: &Path,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> Result<ImportResult, SyncError> {
    on_progress(SyncProgress {
        phase: "Finding files",
        current: 0,
        total: 0,
    });
    let all_formats = SupportedFormat::ALL.to_vec();
    let mut audio_files = Vec::new();
    find_audio_files(input_dir, &all_formats, true, &mut audio_files)?;
    audio_files.sort_by(|(a, _), (b, _)| a.cmp(b));

    Ok(import_tracks(lib, &audio_files, on_progress))
}

/// Import specific audio files into the library.
pub fn import_files(
    lib: &Library,
    paths: Vec<PathBuf>,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> ImportResult {
    let mut audio_files: Vec<_> = paths.into_iter().filter_map(classify_file).collect();
    audio_files.sort_by(|(a, _), (b, _)| a.cmp(b));

    import_tracks(lib, &audio_files, on_progress)
}
