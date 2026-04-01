// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

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

pub struct ProbeMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub comment: Option<String>,
    pub year: u16,
    pub track_number: u32,
    pub duration_secs: u16,
    pub sample_rate: u32,
    pub bitrate: u32,
}

/// Extract metadata via ffprobe as a fallback when lofty fails.
#[allow(clippy::too_many_lines)]
pub fn probe_metadata(path: &Path) -> Result<ProbeMetadata, String> {
    let output = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries"])
        .arg("format=duration,bit_rate")
        .args([
            "--show_entries",
            "format_tags=title,artist,album,genre,comment,date,track",
        ])
        .args(["-show_entries", "stream=sample_rate"])
        .args(["-select_streams", "a:0"])
        .args(["-of", "default=noprint_wrappers=1:nokey=0"])
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffprobe exited with error: {}", stderr.trim()));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut title = None;
    let mut artist = None;
    let mut album = None;
    let mut genre = None;
    let mut comment = None;
    let mut year: u16 = 0;
    let mut track_number: u32 = 0;
    let mut duration_secs: u16 = 0;
    let mut sample_rate: u32 = 44100;
    let mut bitrate: u32 = 0;

    for line in text.lines() {
        if let Some((key, val)) = line.split_once('=') {
            match key.trim().to_ascii_lowercase().as_str() {
                "tag:title" | "title" => {
                    let v = val.trim();
                    if !v.is_empty() {
                        title = Some(v.to_string());
                    }
                }
                "tag:artist" | "artist" => {
                    let v = val.trim();
                    if !v.is_empty() {
                        artist = Some(v.to_string());
                    }
                }
                "tag:album" | "album" => {
                    let v = val.trim();
                    if !v.is_empty() {
                        album = Some(v.to_string());
                    }
                }
                "tag:genre" | "genre" => {
                    let v = val.trim();
                    if !v.is_empty() {
                        genre = Some(v.to_string());
                    }
                }
                "tag:comment" | "comment" => {
                    let v = val.trim();
                    if !v.is_empty() {
                        comment = Some(v.to_string());
                    }
                }
                "tag:date" | "date" => {
                    let v = val.trim();
                    if let Some(y) = v.get(..4).and_then(|s| s.parse::<u16>().ok()) {
                        year = y;
                    }
                }
                "tag:track" | "track" => {
                    let v = val.trim().split('/').next().unwrap_or("");
                    if let Ok(n) = v.parse::<u32>() {
                        track_number = n;
                    }
                }
                "duration" => {
                    if let Ok(d) = val.trim().parse::<f64>() {
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        {
                            duration_secs = u16::try_from(d.clamp(0.0, f64::from(u16::MAX)) as u64)
                                .unwrap_or(u16::MAX);
                        }
                    }
                }
                "sample_rate" => {
                    if let Ok(sr) = val.trim().parse::<u32>() {
                        sample_rate = sr;
                    }
                }
                "bit_rate" => {
                    if let Ok(br) = val.trim().parse::<u64>() {
                        bitrate = u32::try_from(br / 1000).unwrap_or(u32::MAX);
                    }
                }
                _ => {}
            }
        }
    }

    Ok(ProbeMetadata {
        title,
        artist,
        album,
        genre,
        comment,
        year,
        track_number,
        duration_secs,
        sample_rate,
        bitrate,
    })
}
