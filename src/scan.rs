// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::format::SupportedFormat;
use std::path::{Path, PathBuf};

fn is_known_audio_extension(ext: &str) -> bool {
    matches!(
        ext,
        "mp3" | "wav" | "aiff" | "aif" | "aac" | "m4a" | "flac" | "ogg" | "wma" | "opus"
    )
}

pub fn find_audio_files(
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
