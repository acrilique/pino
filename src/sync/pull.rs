// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{
    SyncError, SyncProgress, SyncWarnings, device_relative_path, open_remote_library,
    reconcile_remote_track_ids, unique_path,
};
use crate::library::Library;
use crate::paths;
use std::collections::HashSet;
use std::path::Path;

/// Pull result including count plus any warnings.
pub struct PullResult {
    pub pulled: u32,
    pub warnings: Vec<String>,
}

/// Import tracks from the remote (USB) into the local library.
///
/// For each track on the remote that doesn't exist locally, copies the audio file from
/// `<USB>/Contents/` to `~/.local/share/pino/imported/` and registers it in the local library.
pub fn pull_from_remote(
    lib: &Library,
    dest_dir: &Path,
    on_progress: &(dyn Fn(SyncProgress) + Sync),
) -> Result<PullResult, SyncError> {
    let remote_lib = open_remote_library(dest_dir)?.ok_or(SyncError::NoRemoteDb)?;
    reconcile_remote_track_ids(lib, &remote_lib)?;

    let local_ids: HashSet<String> = lib.track_ids()?.into_iter().collect();
    let remote_tracks = remote_lib.all_tracks()?;

    let to_pull: Vec<_> = remote_tracks
        .iter()
        .filter(|tv| !local_ids.contains(&tv.id))
        .collect();

    if to_pull.is_empty() {
        return Ok(PullResult {
            pulled: 0,
            warnings: Vec::new(),
        });
    }

    let import_dir = paths::data_dir().join("imported");
    std::fs::create_dir_all(&import_dir)?;

    let contents_dir = dest_dir.join("Contents");
    let warnings = SyncWarnings::new();
    let total = u32::try_from(to_pull.len())?;
    let mut pulled = 0u32;

    for (i, tv) in to_pull.iter().enumerate() {
        on_progress(SyncProgress {
            phase: "Pulling from device",
            current: u32::try_from(i + 1)?,
            total,
        });

        // Copy each file from USB to the local imported directory, then import.
        let mut any_file_ok = false;
        for file_view in &tv.files {
            let remote_rel_path = device_relative_path(&file_view.file_path);
            let Some(filename) = remote_rel_path.file_name() else {
                warnings.push(format!("Invalid path on device: {}", file_view.file_path));
                continue;
            };
            let src = contents_dir.join(&remote_rel_path);
            if !src.exists() {
                warnings.push(format!("File not found on device: {}", src.display()));
                continue;
            }

            let dest = {
                let candidate = import_dir.join(filename);
                if candidate.exists() {
                    unique_path(&candidate)
                } else {
                    candidate
                }
            };

            if let Err(e) = std::fs::copy(&src, &dest) {
                warnings.push(format!("Copy failed: {e}"));
                continue;
            }

            // Import the copied file into the local library via aoide.
            match lib.import_file_variant(&dest, &tv.id, None, tv) {
                Ok((imported, import_warnings)) => {
                    for warning in import_warnings {
                        warnings.push(warning);
                    }
                    if imported > 0 {
                        any_file_ok = true;
                    } else {
                        let _ = std::fs::remove_file(&dest);
                    }
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&dest);
                    warnings.push(format!("Failed to import pulled file: {e}"));
                }
            }
        }

        if any_file_ok {
            pulled += 1;
        }
    }

    Ok(PullResult {
        pulled,
        warnings: warnings.into_vec(),
    })
}
