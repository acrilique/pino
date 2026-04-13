// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use rusqlite::{Connection, Row, params};
use std::collections::HashSet;
use std::path::Path;

// track_number starts with track, so clippy complains
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::struct_field_names)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub composer: String,
    pub label: String,
    pub remixer: String,
    pub key: String,
    pub comment: String,
    pub isrc: String,
    pub lyricist: String,
    pub mix_name: String,
    pub release_date: String,
    pub duration_secs: u16,
    pub tempo: u32,
    pub year: u16,
    pub track_number: u32,
    pub disc_number: u16,
    pub rating: u8,
    pub color: u8,
    pub artwork_path: String,
    pub added_at: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackFile {
    pub id: String,
    pub track_id: String,
    pub format: String,
    pub file_path: String,
    pub file_size: u32,
    pub sample_rate: u32,
    pub bitrate: u32,
    pub added_at: String,
}

fn track_from_row(row: &Row) -> rusqlite::Result<Track> {
    Ok(Track {
        id: row.get(0)?,
        title: row.get(1)?,
        artist: row.get(2)?,
        album: row.get(3)?,
        genre: row.get(4)?,
        composer: row.get(5)?,
        label: row.get(6)?,
        remixer: row.get(7)?,
        key: row.get(8)?,
        comment: row.get(9)?,
        isrc: row.get(10)?,
        lyricist: row.get(11)?,
        mix_name: row.get(12)?,
        release_date: row.get(13)?,
        duration_secs: row.get(14)?,
        tempo: row.get(15)?,
        year: row.get(16)?,
        track_number: row.get(17)?,
        disc_number: row.get(18)?,
        rating: row.get(19)?,
        color: row.get(20)?,
        artwork_path: row.get(21)?,
        added_at: row.get(22)?,
    })
}

fn track_file_from_row(row: &Row) -> rusqlite::Result<TrackFile> {
    Ok(TrackFile {
        id: row.get(0)?,
        track_id: row.get(1)?,
        format: row.get(2)?,
        file_path: row.get(3)?,
        file_size: row.get(4)?,
        sample_rate: row.get(5)?,
        bitrate: row.get(6)?,
        added_at: row.get(7)?,
    })
}

/// A track together with all its available files (different formats).
#[derive(Debug, Clone, PartialEq)]
pub struct TrackWithFiles {
    pub track: Track,
    pub files: Vec<TrackFile>,
}

pub struct Library {
    conn: Connection,
}

impl Library {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let conn = Connection::open(path)?;
        let lib = Self { conn };
        lib.migrate()?;
        Ok(lib)
    }

    // There's a bunch of SQL statements here so it makes sense to allow the long function
    #[allow(clippy::too_many_lines)]
    fn migrate(&self) -> rusqlite::Result<()> {
        let has_files_table: bool = self.conn.query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='files'",
            [],
            |row| row.get(0),
        )?;

        if has_files_table {
            self.migrate_extended_metadata()?;
            self.migrate_artwork_path()?;
            return Ok(());
        }

        // Check if the old single-table schema exists (has format column on tracks).
        let has_old_tracks: bool = self.conn.query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='tracks'",
            [],
            |row| row.get(0),
        )?;

        if has_old_tracks {
            // Read file data from old table before dropping it.
            let mut stmt = self.conn.prepare(
                "SELECT id, format, file_path, file_size, sample_rate, bitrate, added_at FROM tracks",
            )?;
            let old_files: Vec<(String, String, String, u32, u32, u32, String)> = stmt
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(stmt);

            self.conn.execute_batch(
                "CREATE TABLE files (
                    id TEXT PRIMARY KEY,
                    track_id TEXT NOT NULL REFERENCES tracks_new(id),
                    format TEXT NOT NULL,
                    file_path TEXT NOT NULL UNIQUE,
                    file_size INTEGER NOT NULL DEFAULT 0,
                    sample_rate INTEGER NOT NULL DEFAULT 44100,
                    bitrate INTEGER NOT NULL DEFAULT 0,
                    added_at TEXT NOT NULL
                );

                CREATE TABLE tracks_new (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    artist TEXT NOT NULL DEFAULT '',
                    album TEXT NOT NULL DEFAULT '',
                    genre TEXT NOT NULL DEFAULT '',
                    composer TEXT NOT NULL DEFAULT '',
                    label TEXT NOT NULL DEFAULT '',
                    remixer TEXT NOT NULL DEFAULT '',
                    key TEXT NOT NULL DEFAULT '',
                    comment TEXT NOT NULL DEFAULT '',
                    isrc TEXT NOT NULL DEFAULT '',
                    lyricist TEXT NOT NULL DEFAULT '',
                    mix_name TEXT NOT NULL DEFAULT '',
                    release_date TEXT NOT NULL DEFAULT '',
                    duration_secs INTEGER NOT NULL DEFAULT 0,
                    tempo INTEGER NOT NULL DEFAULT 0,
                    year INTEGER NOT NULL DEFAULT 0,
                    track_number INTEGER NOT NULL DEFAULT 0,
                    disc_number INTEGER NOT NULL DEFAULT 0,
                    rating INTEGER NOT NULL DEFAULT 0,
                    color INTEGER NOT NULL DEFAULT 0,
                    artwork_path TEXT NOT NULL DEFAULT '',
                    added_at TEXT NOT NULL
                );

                INSERT INTO tracks_new (id, title, artist, album, duration_secs, tempo, added_at)
                    SELECT id, title, artist, album, duration_secs, tempo, added_at FROM tracks;

                DROP TABLE tracks;
                ALTER TABLE tracks_new RENAME TO tracks;",
            )?;

            // Insert file rows with generated UUIDs.
            for (track_id, format, file_path, file_size, sample_rate, bitrate, added_at) in
                &old_files
            {
                let file_id = uuid::Uuid::new_v4().to_string();
                self.conn.execute(
                    "INSERT OR IGNORE INTO files (id, track_id, format, file_path, file_size, sample_rate, bitrate, added_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![file_id, track_id, format, file_path, file_size, sample_rate, bitrate, added_at],
                )?;
            }
        } else {
            // Fresh install — create both tables.
            self.conn.execute_batch(
                "CREATE TABLE tracks (
                    id TEXT PRIMARY KEY,
                    title TEXT NOT NULL DEFAULT '',
                    artist TEXT NOT NULL DEFAULT '',
                    album TEXT NOT NULL DEFAULT '',
                    genre TEXT NOT NULL DEFAULT '',
                    composer TEXT NOT NULL DEFAULT '',
                    label TEXT NOT NULL DEFAULT '',
                    remixer TEXT NOT NULL DEFAULT '',
                    key TEXT NOT NULL DEFAULT '',
                    comment TEXT NOT NULL DEFAULT '',
                    isrc TEXT NOT NULL DEFAULT '',
                    lyricist TEXT NOT NULL DEFAULT '',
                    mix_name TEXT NOT NULL DEFAULT '',
                    release_date TEXT NOT NULL DEFAULT '',
                    duration_secs INTEGER NOT NULL DEFAULT 0,
                    tempo INTEGER NOT NULL DEFAULT 0,
                    year INTEGER NOT NULL DEFAULT 0,
                    track_number INTEGER NOT NULL DEFAULT 0,
                    disc_number INTEGER NOT NULL DEFAULT 0,
                    rating INTEGER NOT NULL DEFAULT 0,
                    color INTEGER NOT NULL DEFAULT 0,
                    artwork_path TEXT NOT NULL DEFAULT '',
                    added_at TEXT NOT NULL
                );

                CREATE TABLE files (
                    id TEXT PRIMARY KEY,
                    track_id TEXT NOT NULL REFERENCES tracks(id),
                    format TEXT NOT NULL,
                    file_path TEXT NOT NULL UNIQUE,
                    file_size INTEGER NOT NULL DEFAULT 0,
                    sample_rate INTEGER NOT NULL DEFAULT 44100,
                    bitrate INTEGER NOT NULL DEFAULT 0,
                    added_at TEXT NOT NULL
                );",
            )?;
        }

        Ok(())
    }

    /// Add the extended metadata columns if they don't exist yet (v2 → v3 migration).
    fn migrate_extended_metadata(&self) -> rusqlite::Result<()> {
        let has_genre: bool = self.conn.query_row(
            "SELECT count(*) > 0 FROM pragma_table_info('tracks') WHERE name='genre'",
            [],
            |row| row.get(0),
        )?;
        if has_genre {
            return Ok(());
        }
        self.conn.execute_batch(
            "ALTER TABLE tracks ADD COLUMN genre TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN composer TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN label TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN remixer TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN key TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN comment TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN isrc TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN lyricist TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN mix_name TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN release_date TEXT NOT NULL DEFAULT '';
             ALTER TABLE tracks ADD COLUMN year INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE tracks ADD COLUMN track_number INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE tracks ADD COLUMN disc_number INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE tracks ADD COLUMN rating INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE tracks ADD COLUMN color INTEGER NOT NULL DEFAULT 0;",
        )?;
        Ok(())
    }

    /// Add the `artwork_path` column if it doesn't exist yet (v3 → v4 migration).
    fn migrate_artwork_path(&self) -> rusqlite::Result<()> {
        let has_artwork_path: bool = self.conn.query_row(
            "SELECT count(*) > 0 FROM pragma_table_info('tracks') WHERE name='artwork_path'",
            [],
            |row| row.get(0),
        )?;
        if has_artwork_path {
            return Ok(());
        }
        self.conn.execute_batch(
            "ALTER TABLE tracks ADD COLUMN artwork_path TEXT NOT NULL DEFAULT '';",
        )?;
        Ok(())
    }

    pub fn add_track(&self, track: &Track) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO tracks (
                id, title, artist, album, genre, composer, label, remixer,
                key, comment, isrc, lyricist, mix_name, release_date,
                duration_secs, tempo, year, track_number, disc_number,
                rating, color, artwork_path, added_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19,
                ?20, ?21, ?22, ?23
             )",
            params![
                track.id,
                track.title,
                track.artist,
                track.album,
                track.genre,
                track.composer,
                track.label,
                track.remixer,
                track.key,
                track.comment,
                track.isrc,
                track.lyricist,
                track.mix_name,
                track.release_date,
                track.duration_secs,
                track.tempo,
                track.year,
                track.track_number,
                track.disc_number,
                track.rating,
                track.color,
                track.artwork_path,
                track.added_at,
            ],
        )?;
        Ok(())
    }

    pub fn add_file(&self, file: &TrackFile) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO files (id, track_id, format, file_path, file_size, sample_rate, bitrate, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                file.id,
                file.track_id,
                file.format,
                file.file_path,
                file.file_size,
                file.sample_rate,
                file.bitrate,
                file.added_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_all_tracks_with_files(&self) -> rusqlite::Result<Vec<TrackWithFiles>> {
        let mut track_stmt = self.conn.prepare(
            "SELECT id, title, artist, album, genre, composer, label, remixer,
                    key, comment, isrc, lyricist, mix_name, release_date,
                    duration_secs, tempo, year, track_number, disc_number,
                    rating, color, artwork_path, added_at
             FROM tracks ORDER BY artist, album, title",
        )?;
        let tracks: Vec<Track> = track_stmt
            .query_map([], track_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut file_stmt = self.conn.prepare(
            "SELECT id, track_id, format, file_path, file_size, sample_rate, bitrate, added_at
             FROM files WHERE track_id = ?1",
        )?;

        let mut result = Vec::with_capacity(tracks.len());
        for track in tracks {
            let files: Vec<TrackFile> = file_stmt
                .query_map(params![track.id], track_file_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            result.push(TrackWithFiles { track, files });
        }
        Ok(result)
    }

    pub fn get_track_ids(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM tracks")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(ids)
    }

    pub fn has_file_path(&self, path: &str) -> rusqlite::Result<bool> {
        let count: u32 = self.conn.query_row(
            "SELECT COUNT(*) FROM files WHERE file_path = ?1",
            params![path],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Update all metadata fields for a track.
    pub fn update_track_from(&self, track: &Track) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE tracks SET
                title = ?1, artist = ?2, album = ?3, genre = ?4,
                composer = ?5, label = ?6, remixer = ?7, key = ?8,
                comment = ?9, isrc = ?10, lyricist = ?11, mix_name = ?12,
                release_date = ?13, duration_secs = ?14, tempo = ?15,
                year = ?16, track_number = ?17, disc_number = ?18,
                rating = ?19, color = ?20, artwork_path = ?21
             WHERE id = ?22",
            params![
                track.title,
                track.artist,
                track.album,
                track.genre,
                track.composer,
                track.label,
                track.remixer,
                track.key,
                track.comment,
                track.isrc,
                track.lyricist,
                track.mix_name,
                track.release_date,
                track.duration_secs,
                track.tempo,
                track.year,
                track.track_number,
                track.disc_number,
                track.rating,
                track.color,
                track.artwork_path,
                track.id,
            ],
        )?;
        Ok(())
    }

    /// Delete a single file entry by its id.
    pub fn delete_file(&self, file_id: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE id = ?1", params![file_id])?;
        Ok(())
    }

    /// Delete a track and all its associated files.
    pub fn delete_track(&self, track_id: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM files WHERE track_id = ?1", params![track_id])?;
        self.conn
            .execute("DELETE FROM tracks WHERE id = ?1", params![track_id])?;
        Ok(())
    }

    /// Get all `file_path` values in the DB (for deduplication on the remote side).
    pub fn used_filenames(&self) -> rusqlite::Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT file_path FROM files")?;
        let mut set = HashSet::new();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            set.insert(row?);
        }
        Ok(set)
    }
}
