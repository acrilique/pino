// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::format::SupportedFormat;
use std::path::Path;

/// Convert `src` to `target`, keeping the embedded cover art where the target can hold it.
pub fn convert(src: &Path, dest: &Path, target: SupportedFormat) -> std::io::Result<()> {
    // WAV cannot carry a cover and ffmpeg does not write one into AIFF.
    let can_hold_cover = matches!(
        target,
        SupportedFormat::Mp3 | SupportedFormat::M4a | SupportedFormat::Flac
    );
    if can_hold_cover && run(src, dest, target, true).is_ok() {
        return Ok(());
    }
    // Also the fallback when the cover is in a format the target container rejects.
    run(src, dest, target, false)
}

fn run(src: &Path, dest: &Path, target: SupportedFormat, keep_cover: bool) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-i"]).arg(src).args(["-y", "-loglevel", "error"]);
    if keep_cover {
        cmd.args(["-map", "0:a", "-map", "0:v?", "-c:v", "copy"])
            .args(["-disposition:v", "attached_pic"]);
    } else {
        cmd.arg("-vn");
    }

    match target {
        SupportedFormat::Mp3 => {
            cmd.args(["-b:a", "320k"]);
        }
        SupportedFormat::M4a => {
            cmd.args(["-b:a", "256k"]);
        }
        _ => {}
    }

    cmd.arg(dest);

    let output = cmd.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(std::io::Error::other(format!(
            "ffmpeg failed: {}",
            stderr.trim()
        )));
    }
    Ok(())
}

pub fn check_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}
