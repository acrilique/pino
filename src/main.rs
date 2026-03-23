// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use clap::Parser;
use lofty::prelude::*;
use rekordcrate::pdb::io::Database;
use rekordcrate::pdb::string::DeviceSQLString;
use rekordcrate::pdb::*;
use rekordcrate::util::FileType;
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportedFormat {
    Mp3,
    Wav,
    Aiff,
    M4a,
    Flac,
}

impl TryFrom<&str> for SupportedFormat {
    type Error = String;

    fn try_from(ext: &str) -> Result<Self, Self::Error> {
        match ext.to_ascii_lowercase().as_str() {
            "mp3" => Ok(Self::Mp3),
            "wav" => Ok(Self::Wav),
            "aiff" | "aif" => Ok(Self::Aiff),
            "aac" | "m4a" => Ok(Self::M4a),
            "flac" => Ok(Self::Flac),
            _ => Err(format!("unknown format '{ext}'")),
        }
    }
}

impl From<SupportedFormat> for &'static str {
    fn from(fmt: SupportedFormat) -> Self {
        match fmt {
            SupportedFormat::Mp3 => "mp3",
            SupportedFormat::Wav => "wav",
            SupportedFormat::Aiff => "aiff",
            SupportedFormat::M4a => "m4a",
            SupportedFormat::Flac => "flac",
        }
    }
}

impl From<SupportedFormat> for FileType {
    fn from(fmt: SupportedFormat) -> Self {
        match fmt {
            SupportedFormat::Mp3 => FileType::Mp3,
            SupportedFormat::Wav => FileType::Wav,
            SupportedFormat::Aiff => FileType::Aiff,
            SupportedFormat::M4a => FileType::M4a,
            SupportedFormat::Flac => FileType::Flac,
        }
    }
}

impl fmt::Display for SupportedFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str((*self).into())
    }
}

fn is_known_audio_extension(ext: &str) -> bool {
    matches!(
        ext,
        "mp3" | "wav" | "aiff" | "aif" | "aac" | "m4a" | "flac" | "ogg" | "wma" | "opus"
    )
}

fn find_audio_files(
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

fn ffmpeg_convert(src: &Path, dest: &Path, target: SupportedFormat) -> std::io::Result<()> {
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args(["-i"]).arg(src).args(["-y", "-loglevel", "error"]);

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

fn check_ffmpeg() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

struct FfprobeMetadata {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    duration_secs: u16,
    sample_rate: u32,
    bitrate: u32,
}

/// Extract metadata via ffprobe as a fallback when lofty fails.
fn ffprobe_metadata(path: &Path) -> Option<FfprobeMetadata> {
    let output = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries"])
        .arg("format=duration,bit_rate")
        .args(["-show_entries", "format_tags=title,artist,album"])
        .args(["-show_entries", "stream=sample_rate"])
        .args(["-select_streams", "a:0"])
        .args(["-of", "default=noprint_wrappers=1:nokey=0"])
        .arg(path)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut title = None;
    let mut artist = None;
    let mut album = None;
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
                "duration" => {
                    if let Ok(d) = val.trim().parse::<f64>() {
                        duration_secs = d as u16;
                    }
                }
                "sample_rate" => {
                    if let Ok(sr) = val.trim().parse::<u32>() {
                        sample_rate = sr;
                    }
                }
                "bit_rate" => {
                    if let Ok(br) = val.trim().parse::<u64>() {
                        bitrate = (br / 1000) as u32;
                    }
                }
                _ => {}
            }
        }
    }

    Some(FfprobeMetadata {
        title,
        artist,
        album,
        duration_secs,
        sample_rate,
        bitrate,
    })
}

struct ExportConfig {
    supported_formats: Vec<SupportedFormat>,
    convert_to: SupportedFormat,
    no_convert: bool,
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Input directory containing audio files.
    #[arg(value_name = "INPUT_DIR")]
    input_dir: PathBuf,
    /// Output directory for the Pioneer export.
    #[arg(value_name = "OUTPUT_DIR")]
    output_dir: PathBuf,
    /// Comma-separated list of supported audio formats (mp3,wav,aiff,aac,flac).
    #[arg(short = 'f', long, default_value = "mp3,wav,aiff,aac")]
    formats: String,
    /// Target format for converting unsupported files (default: mp3).
    #[arg(
        short = 'c',
        long,
        default_value = "mp3",
        conflicts_with = "no_convert"
    )]
    convert_to: String,
    /// Do not convert unsupported files; ignore them instead.
    #[arg(short = 'n', long)]
    no_convert: bool,
}

fn export(input_dir: &Path, output_dir: &Path, config: &ExportConfig) -> rekordcrate::Result<()> {
    // Check FFmpeg availability if conversion is enabled.
    if !config.no_convert && !check_ffmpeg() {
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
    find_audio_files(
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

    // Collect unique artists and albums, and assign IDs.
    let mut artist_map: HashMap<String, u32> = HashMap::new();
    let mut next_artist_id: u32 = 1;
    let mut album_map: HashMap<(String, u32), u32> = HashMap::new();
    let mut next_album_id: u32 = 1;

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

    let mut track_infos = Vec::new();
    let mut used_filenames: HashMap<String, u32> = HashMap::new();

    for (src_path, src_format) in &audio_files {
        let src_format = *src_format;
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
                match ffprobe_metadata(src_path) {
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

        // Copy or convert file to destination.
        let dest_path = contents_dir.join(&dest_filename);
        if src_format.is_some() {
            println!(
                "  Copying {}...",
                src_path.file_name().unwrap_or_default().to_string_lossy()
            );
            std::fs::copy(src_path, &dest_path)?;
        } else {
            println!(
                "  Converting {}...",
                src_path.file_name().unwrap_or_default().to_string_lossy()
            );
            match ffmpeg_convert(src_path, &dest_path, config.convert_to) {
                Ok(()) => {}
                Err(e) => {
                    eprintln!(
                        "  Warning: conversion failed for {}: {e}",
                        src_path.file_name().unwrap_or_default().to_string_lossy()
                    );
                    let _ = std::fs::remove_file(&dest_path);
                    continue;
                }
            }
        }

        let file_size = std::fs::metadata(&dest_path)?.len() as u32;

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

        track_infos.push(TrackInfo {
            dest_filename,
            title,
            artist_name,
            album_name,
            duration_secs,
            sample_rate,
            bitrate,
            file_size,
            tempo,
            file_type: dest_format,
        });
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

    let config = ExportConfig {
        supported_formats,
        convert_to,
        no_convert: cli.no_convert,
    };

    if let Err(e) = export(&cli.input_dir, &cli.output_dir, &config) {
        eprintln!("Error: {e}");
    }
}
