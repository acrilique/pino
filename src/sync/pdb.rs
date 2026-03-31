// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use super::{SyncError, today};
use crate::db::{Library, TrackWithFiles};
use crate::format::SupportedFormat;
use rekordcrate::pdb::io::Database;
use rekordcrate::pdb::string::DeviceSQLString;
use rekordcrate::pdb::{
    Album, Artist, DatabaseType, History, PageType, PlainPageType, PlainRow, Row, Subtype,
};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

type ArtistMap = HashMap<String, u32>;
type AlbumMap = HashMap<(String, u32), u32>;

/// Generate a Pioneer PDB database from all tracks in the given library.
pub(super) fn generate_pdb(db: &Library, dest_dir: &Path) -> Result<(), SyncError> {
    let all = db.get_all_tracks_with_files()?;

    if all.is_empty() {
        return Ok(());
    }

    let rekordbox_dir = dest_dir.join("PIONEER").join("rekordbox");
    std::fs::create_dir_all(&rekordbox_dir)?;

    let (artist_map, album_map) = build_id_maps(&all);

    let mut pdb = create_pdb(&rekordbox_dir)?;

    let today = today();
    let exported_count = insert_tracks(&mut pdb, &all, &artist_map, &album_map, &today)?;
    insert_artists_and_albums(&mut pdb, &artist_map, &album_map)?;

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

/// Build sequential ID maps for artists and albums from the track list.
fn build_id_maps(all: &[TrackWithFiles]) -> (ArtistMap, AlbumMap) {
    let mut artist_map: ArtistMap = HashMap::new();
    let mut next_artist_id: u32 = 1;
    let mut album_map: AlbumMap = HashMap::new();
    let mut next_album_id: u32 = 1;

    for twf in all {
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

    (artist_map, album_map)
}

/// Create the PDB file with the standard rekordbox table layout.
fn create_pdb(rekordbox_dir: &Path) -> Result<Database<File>, SyncError> {
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
    Ok(Database::create(
        pdb_file,
        DatabaseType::Plain,
        &table_page_types,
    )?)
}

/// Insert track rows into the PDB, returning the number of successfully exported tracks.
fn insert_tracks(
    pdb: &mut Database<File>,
    all: &[TrackWithFiles],
    artist_map: &ArtistMap,
    album_map: &AlbumMap,
    today: &str,
) -> Result<u32, SyncError> {
    let mut exported_count: u32 = 0;

    for twf in all {
        let track = &twf.track;
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
            }
            Err(err) => return Err(err.into()),
        }
    }

    Ok(exported_count)
}

/// Insert artist and album rows into the PDB, sorted by their assigned IDs.
fn insert_artists_and_albums(
    pdb: &mut Database<File>,
    artist_map: &ArtistMap,
    album_map: &AlbumMap,
) -> Result<(), SyncError> {
    let mut artists_sorted: Vec<_> = artist_map.iter().collect();
    artists_sorted.sort_by_key(|&(_, &id)| id);
    for (name, &id) in artists_sorted {
        let artist = Artist::builder().id(id).name(name.parse()?).build();
        pdb.add_row(Row::Plain(PlainRow::Artist(artist)))?;
    }

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

    Ok(())
}
