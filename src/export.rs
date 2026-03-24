// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::ffmpeg;
use crate::format::SupportedFormat;
use crate::scan;
use lofty::prelude::*;
use rekordcrate::pdb::io::Database;
use rekordcrate::pdb::string::DeviceSQLString;
use rekordcrate::pdb::*;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct ExportConfig {
    pub supported_formats: Vec<SupportedFormat>,
    pub convert_to: SupportedFormat,
    pub no_convert: bool,
    pub jobs: usize,
}

struct PreparedTrack {
    src_path: PathBuf,
    dest_filename: String,
    dest_format: SupportedFormat,
    needs_conversion: bool,
    title: String,
    artist_name: String,
    album_name: String,
    duration_secs: u16,
    sample_rate: u32,
    bitrate: u32,
    tempo: u32,
}

struct TrackInfo {
    dest_filename: String,
    title: String,
    artist_name: String,
    album_name: String,
    duration_secs: u16,
    sample_rate: u32,
    bitrate: u32,
    file_size: u32,
    tempo: u32,
    file_type: SupportedFormat,
}

pub fn export(
    input_dir: &Path,
    output_dir: &Path,
    config: &ExportConfig,
) -> rekordcrate::Result<()> {
    // Check FFmpeg availability if conversion is enabled.
    if !config.no_convert && !ffmpeg::check_available() {
        eprintln!("Error: ffmpeg is required for audio conversion but was not found in PATH.");
        eprintln!("Install ffmpeg or use --no-convert to skip unsupported files.");
        std::process::exit(1);
    }

    let rekordbox_dir = output_dir.join("PIONEER").join("rekordbox");
    let contents_dir = output_dir.join("Contents");
    std::fs::create_dir_all(&rekordbox_dir)?;
    std::fs::create_dir_all(&contents_dir)?;

    // Scan for audio files recursively.
    let mut audio_files = Vec::new();
    scan::find_audio_files(
        input_dir,
        &config.supported_formats,
        !config.no_convert,
        &mut audio_files,
    )?;
    audio_files.sort_by(|(a, _), (b, _)| a.cmp(b));

    if audio_files.is_empty() {
        eprintln!("No audio files found in {}", input_dir.display());
        return Ok(());
    }

    let supported_count = audio_files.iter().filter(|(_, f)| f.is_some()).count();
    let convert_count = audio_files.len() - supported_count;
    println!(
        "Found {} audio file(s) ({} supported, {} to convert)",
        audio_files.len(),
        supported_count,
        convert_count,
    );

    // === Phase 1: Read metadata and prepare tracks (sequential) ===
    let mut artist_map: HashMap<String, u32> = HashMap::new();
    let mut next_artist_id: u32 = 1;
    let mut album_map: HashMap<(String, u32), u32> = HashMap::new();
    let mut next_album_id: u32 = 1;

    let mut prepared: Vec<PreparedTrack> = Vec::new();
    let mut used_filenames: HashMap<String, u32> = HashMap::new();

    for (src_path, src_format) in &audio_files {
        let src_format = *src_format;
        let needs_conversion = src_format.is_none();
        // Determine destination format and filename with dedup.
        let original_stem = src_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let dest_format = src_format.unwrap_or(config.convert_to);
        let dest_ext: &str = dest_format.into();
        let dest_filename = {
            let base = format!("{}.{}", original_stem, dest_ext);
            let count = used_filenames.entry(base.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                base
            } else {
                format!("{}_{}.{}", original_stem, count, dest_ext)
            }
        };

        // Read metadata from source (with ffprobe fallback for problematic files).
        let (title, artist_name, album_name, duration_secs, sample_rate, bitrate);
        match lofty::read_from_path(src_path) {
            Ok(tagged_file) => {
                let tag = tagged_file
                    .primary_tag()
                    .or_else(|| tagged_file.first_tag());

                title = tag
                    .and_then(|t| t.title().map(|s| s.to_string()))
                    .unwrap_or_else(|| original_stem.clone());
                artist_name = tag
                    .and_then(|t| t.artist().map(|s| s.to_string()))
                    .unwrap_or_default();
                album_name = tag
                    .and_then(|t| t.album().map(|s| s.to_string()))
                    .unwrap_or_default();

                let properties = tagged_file.properties();
                duration_secs = properties.duration().as_secs() as u16;
                sample_rate = properties.sample_rate().unwrap_or(44100);
                bitrate = properties.overall_bitrate().unwrap_or(320);
            }
            Err(e) => {
                let filename = src_path.file_name().unwrap_or_default().to_string_lossy();
                eprintln!("  Warning: lofty could not read metadata for {filename}: {e}");
                match ffmpeg::probe_metadata(src_path) {
                    Some(meta) => {
                        eprintln!("    Using ffprobe metadata fallback");
                        title = meta.title.unwrap_or_else(|| original_stem.clone());
                        artist_name = meta.artist.unwrap_or_default();
                        album_name = meta.album.unwrap_or_default();
                        duration_secs = meta.duration_secs;
                        sample_rate = meta.sample_rate;
                        bitrate = meta.bitrate;
                    }
                    None => {
                        eprintln!("    ffprobe fallback also failed, skipping file");
                        continue;
                    }
                }
            }
        };

        // Setting tempo here to the actual value doesn't change at all the behavior of my CDJ-350
        // players, which have tempo detection. In the future it makes sense to fill this with a
        // real value and also to generate valid ANLZ files for each track.
        let tempo = 0;

        // Register artist.
        if !artist_name.is_empty() && !artist_map.contains_key(&artist_name) {
            artist_map.insert(artist_name.clone(), next_artist_id);
            next_artist_id += 1;
        }

        // Register album (keyed by album name + artist ID for uniqueness).
        if !album_name.is_empty() {
            let artist_id = if artist_name.is_empty() {
                0
            } else {
                artist_map[&artist_name]
            };
            let key = (album_name.clone(), artist_id);
            if let std::collections::hash_map::Entry::Vacant(entry) = album_map.entry(key) {
                entry.insert(next_album_id);
                next_album_id += 1;
            }
        }

        prepared.push(PreparedTrack {
            src_path: src_path.clone(),
            dest_filename,
            dest_format,
            needs_conversion,
            title,
            artist_name,
            album_name,
            duration_secs,
            sample_rate,
            bitrate,
            tempo,
        });
    }

    // === Phase 2: Convert files in parallel to a local temp directory ===
    // Writing to local disk avoids saturating slow destination I/O (e.g. USB drives).
    let total = prepared.len();
    let width = total.to_string().len();
    let convert_count_actual = prepared.iter().filter(|t| t.needs_conversion).count();

    // Only create temp dir if there are files to convert.
    let temp_dir = if convert_count_actual > 0 {
        Some(std::env::temp_dir().join(format!("pino-{}", std::process::id())))
    } else {
        None
    };
    if let Some(ref td) = temp_dir {
        std::fs::create_dir_all(td)?;
    }

    // Each slot: true = conversion succeeded, false = failed.
    let convert_ok: Vec<Mutex<bool>> = (0..total).map(|_| Mutex::new(false)).collect();
    let next_idx = AtomicUsize::new(0);
    let converted = AtomicUsize::new(0);

    // Only convert files in parallel (skip copies, those go straight to destination later).
    let items_to_convert: Vec<usize> = prepared
        .iter()
        .enumerate()
        .filter(|(_, t)| t.needs_conversion)
        .map(|(i, _)| i)
        .collect();
    let convert_total = items_to_convert.len();

    if convert_total > 0 {
        std::thread::scope(|s| {
            let num_workers = config.jobs.min(convert_total).max(1);
            for _ in 0..num_workers {
                s.spawn(|| {
                    loop {
                        let work_idx = next_idx.fetch_add(1, Ordering::Relaxed);
                        if work_idx >= convert_total {
                            break;
                        }
                        let idx = items_to_convert[work_idx];
                        let track = &prepared[idx];
                        let temp_path = temp_dir.as_ref().unwrap().join(&track.dest_filename);
                        let src_name = track
                            .src_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy();
                        let n = converted.fetch_add(1, Ordering::Relaxed) + 1;

                        println!("  [{n:>width$}/{convert_total}] Converting {src_name}...");
                        match ffmpeg::convert(&track.src_path, &temp_path, config.convert_to) {
                            Ok(()) => {
                                *convert_ok[idx].lock().unwrap() = true;
                            }
                            Err(e) => {
                                eprintln!("  Warning: conversion failed for {src_name}: {e}");
                                let _ = std::fs::remove_file(&temp_path);
                            }
                        }
                    }
                });
            }
        });
    }

    // === Phase 3: Transfer files sequentially to the destination ===
    let mut track_infos: Vec<TrackInfo> = Vec::new();
    for (i, track) in prepared.into_iter().enumerate() {
        let dest_path = contents_dir.join(&track.dest_filename);
        let src_name = track
            .src_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let file_size = if track.needs_conversion {
            if !*convert_ok[i].lock().unwrap() {
                continue;
            }
            let temp_path = temp_dir.as_ref().unwrap().join(&track.dest_filename);
            println!(
                "  [{:>width$}/{total}] Moving {}...",
                track_infos.len() + 1,
                track.dest_filename,
            );
            match std::fs::rename(&temp_path, &dest_path)
                .or_else(|_| std::fs::copy(&temp_path, &dest_path).map(|_| ()))
            {
                Ok(()) => {
                    let _ = std::fs::remove_file(&temp_path);
                    std::fs::metadata(&dest_path)?.len() as u32
                }
                Err(e) => {
                    eprintln!("  Warning: failed to move converted file {src_name}: {e}");
                    let _ = std::fs::remove_file(&temp_path);
                    continue;
                }
            }
        } else {
            println!(
                "  [{:>width$}/{total}] Copying {src_name}...",
                track_infos.len() + 1
            );
            match std::fs::copy(&track.src_path, &dest_path) {
                Ok(size) => size as u32,
                Err(e) => {
                    eprintln!("  Warning: copy failed for {src_name}: {e}");
                    continue;
                }
            }
        };

        track_infos.push(TrackInfo {
            dest_filename: track.dest_filename,
            title: track.title,
            artist_name: track.artist_name,
            album_name: track.album_name,
            duration_secs: track.duration_secs,
            sample_rate: track.sample_rate,
            bitrate: track.bitrate,
            file_size,
            tempo: track.tempo,
            file_type: track.dest_format,
        });
    }

    // Clean up temp directory.
    if let Some(ref td) = temp_dir {
        let _ = std::fs::remove_dir_all(td);
    }

    // Create the PDB database.
    // Use the same layout as real rekordbox exports, including Unknown table types.
    let table_page_types = vec![
        PageType::Plain(PlainPageType::Tracks),
        PageType::Plain(PlainPageType::Genres),
        PageType::Plain(PlainPageType::Artists),
        PageType::Plain(PlainPageType::Albums),
        PageType::Plain(PlainPageType::Labels),
        PageType::Plain(PlainPageType::Keys),
        PageType::Plain(PlainPageType::Colors),
        PageType::Plain(PlainPageType::PlaylistTree),
        PageType::Plain(PlainPageType::PlaylistEntries),
        PageType::Unknown(9),
        PageType::Unknown(10),
        PageType::Plain(PlainPageType::HistoryPlaylists),
        PageType::Plain(PlainPageType::HistoryEntries),
        PageType::Plain(PlainPageType::Artwork),
        PageType::Unknown(14),
        PageType::Unknown(15),
        PageType::Plain(PlainPageType::Columns),
        PageType::Plain(PlainPageType::Menu),
        PageType::Unknown(18),
        PageType::Plain(PlainPageType::History),
    ];

    let pdb_path = rekordbox_dir.join("export.pdb");
    let pdb_file = File::create(&pdb_path)?;
    let mut db = Database::create(pdb_file, DatabaseType::Plain, &table_page_types)?;

    // Insert track rows.
    let mut exported_track_count: u32 = 0;
    for info in &track_infos {
        let track_id = exported_track_count + 1;
        let artist_id = if info.artist_name.is_empty() {
            0
        } else {
            artist_map[&info.artist_name]
        };
        let album_id = if info.album_name.is_empty() {
            0
        } else {
            album_map[&(info.album_name.clone(), artist_id)]
        };

        // Pioneer uses forward-slash paths relative to the USB root.
        let pioneer_path = format!("/Contents/{}", info.dest_filename);

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        let track = Track::builder()
            .id(track_id)
            .title(info.title.parse()?)
            .artist_id(artist_id)
            .album_id(album_id)
            .file_path(pioneer_path.parse()?)
            .filename(info.dest_filename.parse()?)
            .sample_rate(info.sample_rate)
            .sample_depth(16)
            .bitrate(info.bitrate)
            .duration(info.duration_secs)
            .file_size(info.file_size)
            .file_type(info.file_type.into())
            .tempo(info.tempo)
            .autoload_hotcues("ON".parse()?)
            .date_added(today.parse()?)
            .build();
        match db.add_row(Row::Plain(PlainRow::Track(track))) {
            Ok(_) => {
                exported_track_count += 1;
            }
            Err(err @ rekordcrate::Error::TrackRowTooSmall { .. }) => {
                eprintln!(
                    "  Warning: track '{}' won't be exported: {err}",
                    info.dest_filename
                );
                let dest_path = contents_dir.join(&info.dest_filename);
                let _ = std::fs::remove_file(&dest_path);
                continue;
            }
            Err(err) => return Err(err),
        }

        println!(
            "  [{}] {} - {}{}",
            track_id,
            if info.artist_name.is_empty() {
                "(unknown)"
            } else {
                &info.artist_name
            },
            info.title,
            if info.album_name.is_empty() {
                String::new()
            } else {
                format!(" [{}]", info.album_name)
            },
        );
    }

    // Insert artist rows (sorted by ID for deterministic output).
    let mut artists_sorted: Vec<_> = artist_map.iter().collect();
    artists_sorted.sort_by_key(|&(_, &id)| id);
    for (name, &id) in artists_sorted {
        let artist = Artist::builder().id(id).name(name.parse()?).build();
        db.add_row(Row::Plain(PlainRow::Artist(artist)))?;
    }

    // Insert album rows (sorted by ID for deterministic output).
    let mut albums_sorted: Vec<_> = album_map.iter().collect();
    albums_sorted.sort_by_key(|&(_, &id)| id);
    for ((album_name, artist_id), &id) in albums_sorted {
        let album = Album::builder()
            .id(id)
            .artist_id(*artist_id)
            .name(album_name.parse()?)
            .build();
        db.add_row(Row::Plain(PlainRow::Album(album)))?;
    }

    rekordcrate::pdb::defaults::insert_default_colors(&mut db)?;
    rekordcrate::pdb::defaults::insert_default_columns(&mut db)?;
    rekordcrate::pdb::defaults::insert_default_menus(&mut db)?;

    // Insert history sync row with current date.
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    db.add_row(Row::Plain(PlainRow::History(History {
        subtype: Subtype(640),
        index_shift: 0,
        num_tracks: exported_track_count,
        date: today.parse()?,
        version: "1000".parse()?,
        label: DeviceSQLString::empty(),
    })))?;

    // Write the database.
    db.close()?;

    println!(
        "\nExported {} track(s), {} artist(s), and {} album(s) to {}",
        exported_track_count,
        artist_map.len(),
        album_map.len(),
        pdb_path.display(),
    );

    Ok(())
}
