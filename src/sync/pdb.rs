// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use super::{SyncError, today};
use crate::db::Library;
use crate::format::SupportedFormat;
use rekordcrate::pdb::io::Database;
use rekordcrate::pdb::string::DeviceSQLString;
use rekordcrate::pdb::*;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

/// Generate a Pioneer PDB database from all tracks in the given library.
pub(super) fn generate_pdb(db: &Library, dest_dir: &Path) -> Result<(), SyncError> {
    let all = db.get_all_tracks_with_files()?;

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
    let today = today();
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
            Err(err) => return Err(err.into()),
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
