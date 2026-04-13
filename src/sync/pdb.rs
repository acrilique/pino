// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::{SyncError, today};
use crate::bridge::TrackView;
use crate::format::SupportedFormat;
use crate::library::Library;
use rekordcrate::pdb::io::Database;
use rekordcrate::pdb::string::DeviceSQLString;
use rekordcrate::pdb::{
    Album, Artist, Artwork, DatabaseType, Genre, History, Key, Label, PageType, PlainPageType,
    PlainRow, Row, Subtype,
};
use rekordcrate::util::ColorIndex;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

type ArtistMap = HashMap<String, u32>;
type AlbumMap = HashMap<(String, u32), u32>;
type GenreMap = HashMap<String, u32>;
type KeyMap = HashMap<String, u32>;
type LabelMap = HashMap<String, u32>;
type ArtworkMap = HashMap<String, u32>;

struct IdMaps {
    artists: ArtistMap,
    albums: AlbumMap,
    genres: GenreMap,
    keys: KeyMap,
    labels: LabelMap,
    artworks: ArtworkMap,
}

/// Generate a Pioneer PDB database from all tracks in the given library.
pub(super) fn generate_pdb(lib: &Library, dest_dir: &Path) -> Result<(), SyncError> {
    let all = lib
        .all_tracks()
        .map_err(|e| SyncError::Other(e.to_string()))?;

    if all.is_empty() {
        return Ok(());
    }

    let rekordbox_dir = dest_dir.join("PIONEER").join("rekordbox");
    std::fs::create_dir_all(&rekordbox_dir)?;

    let maps = build_id_maps(&all);

    copy_artwork_to_device(&all, &maps.artworks, dest_dir)?;

    let mut pdb = create_pdb(&rekordbox_dir)?;

    let today = today();
    let exported_count = insert_tracks(&mut pdb, &all, &maps, &today)?;
    insert_artists(&mut pdb, &maps.artists)?;
    insert_albums(&mut pdb, &maps.albums)?;
    insert_genres(&mut pdb, &maps.genres)?;
    insert_keys(&mut pdb, &maps.keys)?;
    insert_labels(&mut pdb, &maps.labels)?;
    insert_artworks(&mut pdb, &maps.artworks)?;

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
        "PDB: {} track(s), {} artist(s), {} album(s), {} genre(s), {} key(s), {} label(s), {} artwork(s)",
        exported_count,
        maps.artists.len(),
        maps.albums.len(),
        maps.genres.len(),
        maps.keys.len(),
        maps.labels.len(),
        maps.artworks.len(),
    );

    Ok(())
}

/// Build sequential ID maps for artists, albums, genres, keys, and labels from the track list.
///
/// Composers and remixers are inserted into the artist map (they get their own artist IDs).
fn build_id_maps(all: &[TrackView]) -> IdMaps {
    let mut artist_map: ArtistMap = HashMap::new();
    let mut next_artist_id: u32 = 1;
    let mut album_map: AlbumMap = HashMap::new();
    let mut next_album_id: u32 = 1;
    let mut genre_map: GenreMap = HashMap::new();
    let mut next_genre_id: u32 = 1;
    let mut key_map: KeyMap = HashMap::new();
    let mut next_key_id: u32 = 1;
    let mut label_map: LabelMap = HashMap::new();
    let mut next_label_id: u32 = 1;
    let mut artwork_map: ArtworkMap = HashMap::new();
    let mut next_artwork_id: u32 = 1;

    for tv in all {
        if !tv.artist.is_empty() && !artist_map.contains_key(&tv.artist) {
            artist_map.insert(tv.artist.clone(), next_artist_id);
            next_artist_id += 1;
        }

        if !tv.composer.is_empty() && !artist_map.contains_key(&tv.composer) {
            artist_map.insert(tv.composer.clone(), next_artist_id);
            next_artist_id += 1;
        }

        if !tv.remixer.is_empty() && !artist_map.contains_key(&tv.remixer) {
            artist_map.insert(tv.remixer.clone(), next_artist_id);
            next_artist_id += 1;
        }

        if !tv.album.is_empty() {
            let artist_id = if tv.artist.is_empty() {
                0
            } else {
                artist_map[&tv.artist]
            };
            let key = (tv.album.clone(), artist_id);
            if let std::collections::hash_map::Entry::Vacant(entry) = album_map.entry(key) {
                entry.insert(next_album_id);
                next_album_id += 1;
            }
        }

        if !tv.genre.is_empty() && !genre_map.contains_key(&tv.genre) {
            genre_map.insert(tv.genre.clone(), next_genre_id);
            next_genre_id += 1;
        }

        if !tv.key.is_empty() && !key_map.contains_key(&tv.key) {
            key_map.insert(tv.key.clone(), next_key_id);
            next_key_id += 1;
        }

        if !tv.label.is_empty() && !label_map.contains_key(&tv.label) {
            label_map.insert(tv.label.clone(), next_label_id);
            next_label_id += 1;
        }

        if !tv.artwork_path.is_empty() && !artwork_map.contains_key(&tv.artwork_path) {
            artwork_map.insert(tv.artwork_path.clone(), next_artwork_id);
            next_artwork_id += 1;
        }
    }

    IdMaps {
        artists: artist_map,
        albums: album_map,
        genres: genre_map,
        keys: key_map,
        labels: label_map,
        artworks: artwork_map,
    }
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
    all: &[TrackView],
    maps: &IdMaps,
    today: &str,
) -> Result<u32, SyncError> {
    let mut exported_count: u32 = 0;

    for tv in all {
        let Some(file) = tv.files.first() else {
            continue;
        };

        let track_id = exported_count + 1;
        let artist_id = if tv.artist.is_empty() {
            0
        } else {
            maps.artists[&tv.artist]
        };
        let album_id = if tv.album.is_empty() {
            0
        } else {
            maps.albums[&(tv.album.clone(), artist_id)]
        };
        let genre_id = if tv.genre.is_empty() {
            0
        } else {
            maps.genres[&tv.genre]
        };
        let key_id = if tv.key.is_empty() {
            0
        } else {
            maps.keys[&tv.key]
        };
        let label_id = if tv.label.is_empty() {
            0
        } else {
            maps.labels[&tv.label]
        };
        let composer_id = if tv.composer.is_empty() {
            0
        } else {
            maps.artists[&tv.composer]
        };
        let remixer_id = if tv.remixer.is_empty() {
            0
        } else {
            maps.artists[&tv.remixer]
        };
        let artwork_id = if tv.artwork_path.is_empty() {
            0
        } else {
            maps.artworks[&tv.artwork_path]
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
            .title(tv.title.parse()?)
            .artist_id(artist_id)
            .album_id(album_id)
            .genre_id(genre_id)
            .key_id(key_id)
            .label_id(label_id)
            .composer_id(composer_id)
            .remixer_id(remixer_id)
            .artwork_id(artwork_id)
            .file_path(pioneer_path.parse()?)
            .filename(file.file_path.parse()?)
            .sample_rate(file.sample_rate)
            .sample_depth(16)
            .bitrate(file.bitrate)
            .duration(tv.duration_secs)
            .file_size(file.file_size)
            .file_type(file_type.into())
            .tempo(tv.tempo)
            .year(tv.year)
            .track_number(tv.track_number)
            .disc_number(tv.disc_number)
            .rating(tv.rating)
            .color(color_index_from_u8(tv.color))
            .comment(tv.comment.parse()?)
            .isrc(tv.isrc.parse()?)
            .lyricist(tv.lyricist.parse()?)
            .mix_name(tv.mix_name.parse()?)
            .release_date(tv.release_date.parse()?)
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

/// Insert artist rows into the PDB, sorted by their assigned IDs.
fn insert_artists(pdb: &mut Database<File>, artist_map: &ArtistMap) -> Result<(), SyncError> {
    let mut artists_sorted: Vec<_> = artist_map.iter().collect();
    artists_sorted.sort_by_key(|&(_, &id)| id);
    for (name, &id) in artists_sorted {
        let artist = Artist::builder().id(id).name(name.parse()?).build();
        pdb.add_row(Row::Plain(PlainRow::Artist(artist)))?;
    }
    Ok(())
}

/// Insert album rows into the PDB, sorted by their assigned IDs.
fn insert_albums(pdb: &mut Database<File>, album_map: &AlbumMap) -> Result<(), SyncError> {
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

/// Insert genre rows into the PDB, sorted by their assigned IDs.
fn insert_genres(pdb: &mut Database<File>, genre_map: &GenreMap) -> Result<(), SyncError> {
    let mut sorted: Vec<_> = genre_map.iter().collect();
    sorted.sort_by_key(|&(_, &id)| id);
    for (name, &id) in sorted {
        let genre = Genre::builder().id(id).name(name.parse()?).build();
        pdb.add_row(Row::Plain(PlainRow::Genre(genre)))?;
    }
    Ok(())
}

/// Insert key rows into the PDB, sorted by their assigned IDs.
fn insert_keys(pdb: &mut Database<File>, key_map: &KeyMap) -> Result<(), SyncError> {
    let mut sorted: Vec<_> = key_map.iter().collect();
    sorted.sort_by_key(|&(_, &id)| id);
    for (name, &id) in sorted {
        let key = Key::builder().id(id).name(name.parse()?).build();
        pdb.add_row(Row::Plain(PlainRow::Key(key)))?;
    }
    Ok(())
}

/// Insert label rows into the PDB, sorted by their assigned IDs.
fn insert_labels(pdb: &mut Database<File>, label_map: &LabelMap) -> Result<(), SyncError> {
    let mut sorted: Vec<_> = label_map.iter().collect();
    sorted.sort_by_key(|&(_, &id)| id);
    for (name, &id) in sorted {
        let label = Label::builder().id(id).name(name.parse()?).build();
        pdb.add_row(Row::Plain(PlainRow::Label(label)))?;
    }
    Ok(())
}

/// Copy artwork files to PIONEER/Artwork/ on the destination device.
///
/// Each unique `artwork_path` (local cache file) is resized and written twice:
/// - `{id}.jpg`   — 80×80 thumbnail
/// - `{id}_m.jpg` — 240×240 medium image
///
/// Both files are always JPEG regardless of the source format.
fn copy_artwork_to_device(
    all: &[TrackView],
    artwork_map: &ArtworkMap,
    dest_dir: &Path,
) -> Result<(), SyncError> {
    use image::ImageReader;
    use image::imageops::FilterType;

    if artwork_map.is_empty() {
        return Ok(());
    }

    let art_dest = dest_dir.join("PIONEER").join("Artwork");
    std::fs::create_dir_all(&art_dest)?;

    let mut copied = std::collections::HashSet::new();
    for tv in all {
        let art = &tv.artwork_path;
        if art.is_empty() || copied.contains(art) {
            continue;
        }

        let src = std::path::Path::new(art);
        if !src.exists() {
            eprintln!("  Warning: artwork file not found: {art}");
            continue;
        }

        let reader = match ImageReader::open(src) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  Warning: failed to open artwork {art}: {e}");
                continue;
            }
        };
        let img = match reader.decode() {
            Ok(i) => i,
            Err(e) => {
                eprintln!("  Warning: failed to decode artwork {art}: {e}");
                continue;
            }
        };

        let id = artwork_map[art];

        // 80x80 thumbnail
        let thumb = img.resize_to_fill(80, 80, FilterType::Lanczos3);
        thumb.to_rgb8().save(art_dest.join(format!("{id}.jpg")))?;

        // 240x240 medium
        let medium = img.resize_to_fill(240, 240, FilterType::Lanczos3);
        medium
            .to_rgb8()
            .save(art_dest.join(format!("{id}_m.jpg")))?;

        copied.insert(art.clone());
    }

    Ok(())
}

/// Insert artwork rows into the PDB, sorted by their assigned IDs.
///
/// The PDB path always references the 80×80 thumbnail (`{id}.jpg`).
fn insert_artworks(pdb: &mut Database<File>, artwork_map: &ArtworkMap) -> Result<(), SyncError> {
    let mut sorted: Vec<_> = artwork_map.iter().collect();
    sorted.sort_by_key(|&(_, &id)| id);
    for (_, &id) in sorted {
        let device_path = format!("/Artwork/{id}.jpg");
        let artwork = Artwork::builder().id(id).path(device_path.parse()?).build();
        pdb.add_row(Row::Plain(PlainRow::Artwork(artwork)))?;
    }
    Ok(())
}

/// Map a `u8` color value to a [`ColorIndex`].
fn color_index_from_u8(value: u8) -> ColorIndex {
    match value {
        1 => ColorIndex::Pink,
        2 => ColorIndex::Red,
        3 => ColorIndex::Orange,
        4 => ColorIndex::Yellow,
        5 => ColorIndex::Green,
        6 => ColorIndex::Aqua,
        7 => ColorIndex::Blue,
        8 => ColorIndex::Purple,
        _ => ColorIndex::None,
    }
}
