// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Input directory containing audio files.
    #[arg(value_name = "INPUT_DIR")]
    pub input_dir: PathBuf,
    /// Output directory for the Pioneer export.
    #[arg(value_name = "OUTPUT_DIR")]
    pub output_dir: PathBuf,
    /// Comma-separated list of supported audio formats (mp3,wav,aiff,aac,flac).
    #[arg(short = 'f', long, default_value = "mp3,wav,aiff,aac")]
    pub formats: String,
    /// Target format for converting unsupported files (default: mp3).
    #[arg(
        short = 'c',
        long,
        default_value = "mp3",
        conflicts_with = "no_convert"
    )]
    pub convert_to: String,
    /// Do not convert unsupported files; ignore them instead.
    #[arg(short = 'n', long)]
    pub no_convert: bool,
    /// Number of parallel jobs for file conversion/copying.
    #[arg(short = 'j', long)]
    pub jobs: Option<usize>,
}
