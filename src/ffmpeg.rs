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

pub fn convert(src: &Path, dest: &Path, target: SupportedFormat) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-i"])
        .arg(src)
        .args(["-y", "-vn", "-loglevel", "error"]);

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
