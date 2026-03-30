// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use super::{SyncError, SyncProgress, new_id, open_remote_db, today, unique_path};
use crate::db::{Library, TrackFile, TrackWithFiles};
use crate::paths;
use std::collections::HashSet;
use std::path::Path;

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
