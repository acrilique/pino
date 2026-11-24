// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::SyncError;
use crate::library::Library;
use crate::sync::SyncProgress;
use std::path::{Path, PathBuf};

/// Extensions accepted for import: every [`SupportedFormat`](crate::format::SupportedFormat)
/// plus formats that can only reach a device through conversion.
fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp3" | "wav" | "aiff" | "aif" | "aac" | "m4a" | "flac" | "ogg" | "wma" | "opus"
            )
        })
}

fn find_audio_files(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            find_audio_files(&path, files)?;
        } else if is_audio_file(&path) {
            files.push(path);
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
    mut paths: Vec<PathBuf>,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> ImportResult {
    paths.sort();

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
    let mut audio_files = Vec::new();
    find_audio_files(input_dir, &mut audio_files)?;

    Ok(import_tracks(lib, audio_files, on_progress))
}

/// Import specific audio files into the library.
pub fn import_files(
    lib: &Library,
    paths: Vec<PathBuf>,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> ImportResult {
    let audio_files = paths.into_iter().filter(|p| is_audio_file(p)).collect();
    import_tracks(lib, audio_files, on_progress)
}
