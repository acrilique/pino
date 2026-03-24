// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::db::{Library, Track, TrackFile, TrackWithFiles};
use crate::ffmpeg;
use crate::format::SupportedFormat;
use crate::scan;
use lofty::prelude::*;
use rekordcrate::pdb::io::Database;
use rekordcrate::pdb::string::DeviceSQLString;
use rekordcrate::pdb::*;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

pub struct SyncConfig {
    pub supported_formats: Vec<SupportedFormat>,
    /// Format to convert to when a track has no file in an allowed format. `None` = skip those.
    pub convert_to: Option<SupportedFormat>,
}

pub struct SyncResult {
    pub synced: u32,
    pub converted: u32,
    pub skipped: u32,
}

impl std::fmt::Display for SyncResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.synced == 0 && self.skipped == 0 {
            return write!(f, "Everything is up to date.");
        }
        write!(f, "Synced {} track(s)", self.synced)?;
        if self.converted > 0 {
            write!(f, " ({} converted)", self.converted)?;
        }
        if self.skipped > 0 {
            write!(f, ", {} skipped (no matching format)", self.skipped)?;
        }
        write!(f, ".")
    }
}

/// Directory where pino stores locally converted files.
fn converted_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pino")
        .join("converted")
}

/// Scan a folder and import all audio files into the local database.
///
/// Returns the number of newly imported tracks.
pub fn import_folder(db_path: &Path, input_dir: &Path) -> Result<u32, String> {
    let db = Library::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;

    let all_formats = vec![
        SupportedFormat::Mp3,
        SupportedFormat::Wav,
        SupportedFormat::Aiff,
        SupportedFormat::M4a,
        SupportedFormat::Flac,
    ];
    let mut audio_files = Vec::new();
    scan::find_audio_files(input_dir, &all_formats, true, &mut audio_files)
        .map_err(|e| format!("Scan failed: {e}"))?;
    audio_files.sort_by(|(a, _), (b, _)| a.cmp(b));

    let mut imported = 0u32;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    for (src_path, src_format) in &audio_files {
        let path_str = src_path.to_string_lossy().to_string();
        if db.has_file_path(&path_str).unwrap_or(false) {
            continue;
        }

        let original_stem = src_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let format_str: String = match src_format {
            Some(fmt) => <SupportedFormat as Into<&str>>::into(*fmt).to_string(),
            None => src_path
                .extension()
                .unwrap_or_default()
                .to_str()
                .unwrap_or("")
                .to_ascii_lowercase(),
        };

        let (title, artist, album, duration_secs, sample_rate, bitrate) =
            read_metadata(src_path, &original_stem);

        let file_size = std::fs::metadata(src_path)
            .map(|m| m.len() as u32)
            .unwrap_or(0);

        let track_id = uuid::Uuid::new_v4().to_string();
        let track = Track {
            id: track_id.clone(),
            title,
            artist,
            album,
            duration_secs,
            tempo: 0,
            added_at: today.clone(),
        };

        let file = TrackFile {
            id: uuid::Uuid::new_v4().to_string(),
            track_id,
            format: format_str,
            file_path: path_str,
            file_size,
            sample_rate,
            bitrate,
            added_at: today.clone(),
        };

        if let Err(e) = db.add_track(&track) {
            eprintln!("  Warning: failed to add track: {e}");
            continue;
        }
        if let Err(e) = db.add_file(&file) {
            eprintln!("  Warning: failed to add file: {e}");
            continue;
        }
        imported += 1;
    }

    Ok(imported)
}

/// Convert specific tracks in the local library to a target format.
///
/// For each track, takes the first available file and converts it. The converted file is saved
/// in `~/.local/share/pino/converted/` and registered in the local DB.
///
/// Returns the number of successfully converted tracks.
pub fn convert_tracks(
    db_path: &Path,
    track_ids: &[String],
    target: SupportedFormat,
) -> Result<u32, String> {
    if !ffmpeg::check_available() {
        return Err("ffmpeg is required for audio conversion but was not found in PATH".into());
    }

    let db = Library::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;
    let conv_dir = converted_dir();
    std::fs::create_dir_all(&conv_dir).map_err(|e| e.to_string())?;

    let target_ext: &str = target.into();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut converted = 0u32;

    for track_id in track_ids {
        let files = db
            .get_files_for_track(track_id)
            .map_err(|e| e.to_string())?;

        // Skip if already has a file in the target format.
        if files.iter().any(|f| f.format == target_ext) {
            continue;
        }

        // Pick source file (prefer a supported format).
        let Some(source) = files.first() else {
            continue;
        };
        let src_path = PathBuf::from(&source.file_path);
        if !src_path.exists() {
            eprintln!("  Warning: source file not found: {}", source.file_path);
            continue;
        }

        let stem = src_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let dest_path = conv_dir.join(format!("{stem}.{target_ext}"));

        // Avoid overwriting an existing file with a unique name.
        let dest_path = unique_path(&dest_path);

        if let Err(e) = ffmpeg::convert(&src_path, &dest_path, target) {
            eprintln!("  Warning: conversion failed for {}: {e}", source.file_path);
            continue;
        }

        let (_, sample_rate, bitrate) =
            read_audio_properties(&dest_path, 0, source.sample_rate, source.bitrate);
        let file_size = std::fs::metadata(&dest_path)
            .map(|m| m.len() as u32)
            .unwrap_or(0);

        let file = TrackFile {
            id: uuid::Uuid::new_v4().to_string(),
            track_id: track_id.clone(),
            format: target_ext.to_string(),
            file_path: dest_path.to_string_lossy().to_string(),
            file_size,
            sample_rate,
            bitrate,
            added_at: today.clone(),
        };

        if let Err(e) = db.add_file(&file) {
            eprintln!("  Warning: failed to register converted file: {e}");
            continue;
        }
        converted += 1;
    }

    Ok(converted)
}

/// Synchronize local library to a remote destination (additive-only).
///
/// For each local track not yet on the remote:
/// - If a file in one of the allowed formats exists, copy it.
/// - Otherwise, if `convert_to` is set, convert and also save the new file locally.
/// - Otherwise, skip the track.
pub fn sync(db_path: &Path, dest_dir: &Path, config: &SyncConfig) -> Result<SyncResult, String> {
    if config.convert_to.is_some() && !ffmpeg::check_available() {
        return Err("ffmpeg is required for audio conversion but was not found in PATH".into());
    }

    let local_db = Library::open(db_path).map_err(|e| format!("Failed to open local DB: {e}"))?;

    let pino_dir = dest_dir.join("PIONEER").join("pino");
    std::fs::create_dir_all(&pino_dir).map_err(|e| e.to_string())?;
    let remote_db_path = pino_dir.join("library.db");
    let remote_db =
        Library::open(&remote_db_path).map_err(|e| format!("Failed to open remote DB: {e}"))?;

    let contents_dir = dest_dir.join("Contents");
    std::fs::create_dir_all(&contents_dir).map_err(|e| e.to_string())?;

    let local_tracks = local_db
        .get_all_tracks_with_files()
        .map_err(|e| e.to_string())?;
    let remote_ids: std::collections::HashSet<String> = remote_db
        .get_track_ids()
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();

    let to_sync: Vec<&TrackWithFiles> = local_tracks
        .iter()
        .filter(|t| !remote_ids.contains(&t.track.id))
        .collect();

    if to_sync.is_empty() {
        generate_pdb(&remote_db, dest_dir)?;
        return Ok(SyncResult {
            synced: 0,
            converted: 0,
            skipped: 0,
        });
    }

    let mut used_filenames = remote_db.used_filenames().map_err(|e| e.to_string())?;
    let conv_dir = converted_dir();
    std::fs::create_dir_all(&conv_dir).map_err(|e| e.to_string())?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    let total = to_sync.len();
    let mut synced = 0u32;
    let mut converted = 0u32;
    let mut skipped = 0u32;

    for (i, twf) in to_sync.iter().enumerate() {
        let track = &twf.track;

        // Try to find a file in one of the allowed formats.
        let matching_file = twf.files.iter().find(|f| {
            SupportedFormat::try_from(f.format.as_str())
                .is_ok_and(|fmt| config.supported_formats.contains(&fmt))
        });

        let (src_file, dest_format, needs_conversion) = if let Some(f) = matching_file {
            let fmt = SupportedFormat::try_from(f.format.as_str()).unwrap();
            (f, fmt, false)
        } else if let Some(convert_to) = config.convert_to {
            // No matching file — need conversion. Pick the first available source.
            let Some(f) = twf.files.first() else {
                skipped += 1;
                continue;
            };
            (f, convert_to, true)
        } else {
            skipped += 1;
            continue;
        };

        let src_path = PathBuf::from(&src_file.file_path);
        if !src_path.exists() {
            eprintln!("  Warning: source file not found: {}", src_file.file_path);
            skipped += 1;
            continue;
        }

        let dest_ext: &str = dest_format.into();
        let original_stem = src_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        let dest_filename = {
            let base = format!("{original_stem}.{dest_ext}");
            let count = used_filenames.entry(base.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                base
            } else {
                format!("{original_stem}_{}.{dest_ext}", count)
            }
        };
        let dest_path = contents_dir.join(&dest_filename);

        println!("  [{}/{}] {}", i + 1, total, dest_filename);

        if needs_conversion {
            // Convert to local cache first, then copy to USB.
            let local_conv_path =
                unique_path(&conv_dir.join(format!("{original_stem}.{dest_ext}")));

            if let Err(e) = ffmpeg::convert(&src_path, &local_conv_path, dest_format) {
                eprintln!(
                    "  Warning: conversion failed for {}: {e}",
                    src_file.file_path
                );
                skipped += 1;
                continue;
            }

            // Save converted file in local DB.
            let (_, sample_rate, bitrate) =
                read_audio_properties(&local_conv_path, 0, src_file.sample_rate, src_file.bitrate);
            let local_file_size = std::fs::metadata(&local_conv_path)
                .map(|m| m.len() as u32)
                .unwrap_or(0);

            let local_file = TrackFile {
                id: uuid::Uuid::new_v4().to_string(),
                track_id: track.id.clone(),
                format: dest_ext.to_string(),
                file_path: local_conv_path.to_string_lossy().to_string(),
                file_size: local_file_size,
                sample_rate,
                bitrate,
                added_at: today.clone(),
            };
            local_db.add_file(&local_file).ok();

            // Copy to destination.
            if let Err(e) = std::fs::copy(&local_conv_path, &dest_path) {
                eprintln!("  Warning: copy failed for converted file: {e}");
                skipped += 1;
                continue;
            }
            converted += 1;
        } else if let Err(e) = std::fs::copy(&src_path, &dest_path) {
            eprintln!("  Warning: copy failed for {}: {e}", src_file.file_path);
            skipped += 1;
            continue;
        }

        let file_size = std::fs::metadata(&dest_path)
            .map(|m| m.len() as u32)
            .unwrap_or(0);

        // Determine audio properties for the remote entry.
        let (sample_rate, bitrate) = if needs_conversion {
            let (_, sr, br) =
                read_audio_properties(&dest_path, 0, src_file.sample_rate, src_file.bitrate);
            (sr, br)
        } else {
            (src_file.sample_rate, src_file.bitrate)
        };

        // Write track + file to remote DB.
        let remote_track = Track {
            id: track.id.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_secs: track.duration_secs,
            tempo: track.tempo,
            added_at: today.clone(),
        };

        let remote_file = TrackFile {
            id: uuid::Uuid::new_v4().to_string(),
            track_id: track.id.clone(),
            format: dest_ext.to_string(),
            file_path: dest_filename,
            file_size,
            sample_rate,
            bitrate,
            added_at: today.clone(),
        };

        remote_db.add_track(&remote_track).ok();
        if let Err(e) = remote_db.add_file(&remote_file) {
            eprintln!("  Warning: failed to add file to remote DB: {e}");
            continue;
        }

        synced += 1;
    }

    generate_pdb(&remote_db, dest_dir)?;

    Ok(SyncResult {
        synced,
        converted,
        skipped,
    })
}

/// Count how many local tracks have no file in any of the given formats.
pub fn count_needing_conversion(
    db_path: &Path,
    formats: &[SupportedFormat],
) -> Result<u32, String> {
    let db = Library::open(db_path).map_err(|e| format!("Failed to open database: {e}"))?;
    let tracks = db.get_all_tracks_with_files().map_err(|e| e.to_string())?;

    let count = tracks
        .iter()
        .filter(|twf| {
            !twf.files.iter().any(|f| {
                SupportedFormat::try_from(f.format.as_str()).is_ok_and(|fmt| formats.contains(&fmt))
            })
        })
        .count() as u32;

    Ok(count)
}

/// Generate a unique file path by appending `_2`, `_3`, etc. if the path already exists.
fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut n = 2u32;
    loop {
        let candidate = parent.join(format!("{stem}_{n}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// Read audio metadata from a file (lofty with ffprobe fallback).
fn read_metadata(path: &Path, fallback_title: &str) -> (String, String, String, u16, u32, u32) {
    match lofty::read_from_path(path) {
        Ok(tagged_file) => {
            let tag = tagged_file
                .primary_tag()
                .or_else(|| tagged_file.first_tag());

            let title = tag
                .and_then(|t| t.title().map(|s| s.to_string()))
                .unwrap_or_else(|| fallback_title.to_string());
            let artist = tag
                .and_then(|t| t.artist().map(|s| s.to_string()))
                .unwrap_or_default();
            let album = tag
                .and_then(|t| t.album().map(|s| s.to_string()))
                .unwrap_or_default();

            let properties = tagged_file.properties();
            let duration_secs = properties.duration().as_secs() as u16;
            let sample_rate = properties.sample_rate().unwrap_or(44100);
            let bitrate = properties.overall_bitrate().unwrap_or(320);

            (title, artist, album, duration_secs, sample_rate, bitrate)
        }
        Err(e) => {
            let filename = path.file_name().unwrap_or_default().to_string_lossy();
            eprintln!("  Warning: lofty could not read metadata for {filename}: {e}");
            match ffmpeg::probe_metadata(path) {
                Some(meta) => {
                    eprintln!("    Using ffprobe metadata fallback");
                    (
                        meta.title.unwrap_or_else(|| fallback_title.to_string()),
                        meta.artist.unwrap_or_default(),
                        meta.album.unwrap_or_default(),
                        meta.duration_secs,
                        meta.sample_rate,
                        meta.bitrate,
                    )
                }
                None => {
                    eprintln!("    ffprobe fallback also failed, using defaults");
                    (
                        fallback_title.to_string(),
                        String::new(),
                        String::new(),
                        0,
                        44100,
                        0,
                    )
                }
            }
        }
    }
}

/// Read audio properties from a destination file (for converted tracks).
fn read_audio_properties(
    path: &Path,
    fallback_duration: u16,
    fallback_sample_rate: u32,
    fallback_bitrate: u32,
) -> (u16, u32, u32) {
    match lofty::read_from_path(path) {
        Ok(tagged) => {
            let props = tagged.properties();
            (
                props.duration().as_secs() as u16,
                props.sample_rate().unwrap_or(fallback_sample_rate),
                props.overall_bitrate().unwrap_or(fallback_bitrate),
            )
        }
        Err(_) => match ffmpeg::probe_metadata(path) {
            Some(meta) => (meta.duration_secs, meta.sample_rate, meta.bitrate),
            None => (fallback_duration, fallback_sample_rate, fallback_bitrate),
        },
    }
}

/// Generate a Pioneer PDB database from all tracks in the given library.
fn generate_pdb(db: &Library, dest_dir: &Path) -> Result<(), String> {
    generate_pdb_inner(db, dest_dir).map_err(|e| e.to_string())
}

fn generate_pdb_inner(db: &Library, dest_dir: &Path) -> rekordcrate::Result<()> {
    let all = db
        .get_all_tracks_with_files()
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    if all.is_empty() {
        return Ok(());
    }

    let rekordbox_dir = dest_dir.join("PIONEER").join("rekordbox");
    std::fs::create_dir_all(&rekordbox_dir)?;

    // Build artist and album maps with sequential IDs.
    let mut artist_map: HashMap<String, u32> = HashMap::new();
    let mut next_artist_id: u32 = 1;
    let mut album_map: HashMap<(String, u32), u32> = HashMap::new();
    let mut next_album_id: u32 = 1;

    for twf in &all {
        let track = &twf.track;
        if !track.artist.is_empty() && !artist_map.contains_key(&track.artist) {
            artist_map.insert(track.artist.clone(), next_artist_id);
            next_artist_id += 1;
        }
        if !track.album.is_empty() {
            let artist_id = if track.artist.is_empty() {
                0
            } else {
                artist_map[&track.artist]
            };
            let key = (track.album.clone(), artist_id);
            if let std::collections::hash_map::Entry::Vacant(entry) = album_map.entry(key) {
                entry.insert(next_album_id);
                next_album_id += 1;
            }
        }
    }

    // PDB table layout matching real rekordbox exports.
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
    let mut pdb = Database::create(pdb_file, DatabaseType::Plain, &table_page_types)?;

    let mut exported_count: u32 = 0;
    for twf in &all {
        let track = &twf.track;
        // Use the first file entry (remote DB has exactly one per track).
        let Some(file) = twf.files.first() else {
            continue;
        };

        let track_id = exported_count + 1;
        let artist_id = if track.artist.is_empty() {
            0
        } else {
            artist_map[&track.artist]
        };
        let album_id = if track.album.is_empty() {
            0
        } else {
            album_map[&(track.album.clone(), artist_id)]
        };

        let pioneer_path = format!("/Contents/{}", file.file_path);
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        let Ok(file_type) = SupportedFormat::try_from(file.format.as_str()) else {
            eprintln!(
                "  Warning: unsupported format '{}' for PDB, skipping {}",
                file.format, file.file_path
            );
            continue;
        };

        let pdb_track = rekordcrate::pdb::Track::builder()
            .id(track_id)
            .title(track.title.parse()?)
            .artist_id(artist_id)
            .album_id(album_id)
            .file_path(pioneer_path.parse()?)
            .filename(file.file_path.parse()?)
            .sample_rate(file.sample_rate)
            .sample_depth(16)
            .bitrate(file.bitrate)
            .duration(track.duration_secs)
            .file_size(file.file_size)
            .file_type(file_type.into())
            .tempo(track.tempo)
            .autoload_hotcues("ON".parse()?)
            .date_added(today.parse()?)
            .build();

        match pdb.add_row(Row::Plain(PlainRow::Track(pdb_track))) {
            Ok(_) => exported_count += 1,
            Err(rekordcrate::Error::TrackRowTooSmall { .. }) => {
                eprintln!(
                    "  Warning: track '{}' skipped in PDB: row too small",
                    file.file_path
                );
                continue;
            }
            Err(err) => return Err(err),
        }
    }

    // Insert artist rows.
    let mut artists_sorted: Vec<_> = artist_map.iter().collect();
    artists_sorted.sort_by_key(|&(_, &id)| id);
    for (name, &id) in artists_sorted {
        let artist = Artist::builder().id(id).name(name.parse()?).build();
        pdb.add_row(Row::Plain(PlainRow::Artist(artist)))?;
    }

    // Insert album rows.
    let mut albums_sorted: Vec<_> = album_map.iter().collect();
    albums_sorted.sort_by_key(|&(_, &id)| id);
    for ((album_name, artist_id), &id) in albums_sorted {
        let album = Album::builder()
            .id(id)
            .artist_id(*artist_id)
            .name(album_name.parse()?)
            .build();
        pdb.add_row(Row::Plain(PlainRow::Album(album)))?;
    }

    rekordcrate::pdb::defaults::insert_default_colors(&mut pdb)?;
    rekordcrate::pdb::defaults::insert_default_columns(&mut pdb)?;
    rekordcrate::pdb::defaults::insert_default_menus(&mut pdb)?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    pdb.add_row(Row::Plain(PlainRow::History(History {
        subtype: Subtype(640),
        index_shift: 0,
        num_tracks: exported_count,
        date: today.parse()?,
        version: "1000".parse()?,
        label: DeviceSQLString::empty(),
    })))?;

    pdb.close()?;

    println!(
        "PDB: {} track(s), {} artist(s), {} album(s)",
        exported_count,
        artist_map.len(),
        album_map.len(),
    );

    Ok(())
}
