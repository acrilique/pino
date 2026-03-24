// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

mod cli;
mod export;
mod ffmpeg;
mod format;
mod scan;

use clap::Parser;
use cli::Cli;
use export::ExportConfig;
use format::SupportedFormat;

fn main() {
    let cli = Cli::parse();

    let supported_formats: Vec<SupportedFormat> = cli
        .formats
        .split(',')
        .map(|s| {
            SupportedFormat::try_from(s.trim()).unwrap_or_else(|e| {
                eprintln!("Error: {e}");
                eprintln!("Supported formats: mp3, wav, aiff, aac, flac");
                std::process::exit(1);
            })
        })
        .collect();

    let convert_to = SupportedFormat::try_from(cli.convert_to.trim()).unwrap_or_else(|e| {
        eprintln!("Error: {e}");
        eprintln!("Supported formats: mp3, wav, aiff, aac, flac");
        std::process::exit(1);
    });

    if !cli.no_convert && !supported_formats.contains(&convert_to) {
        eprintln!(
            "Error: conversion target '{}' is not in the supported formats list",
            convert_to
        );
        std::process::exit(1);
    }

    let jobs = cli.jobs.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    let config = ExportConfig {
        supported_formats,
        convert_to,
        no_convert: cli.no_convert,
        jobs,
    };

    if let Err(e) = export::export(&cli.input_dir, &cli.output_dir, &config) {
        eprintln!("Error: {e}");
    }
}
