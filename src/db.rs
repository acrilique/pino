// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use rusqlite::{Connection, Row, params};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u16,
    pub tempo: u32,
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
        duration_secs: row.get(4)?,
        tempo: row.get(5)?,
        added_at: row.get(6)?,
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

    fn migrate(&self) -> rusqlite::Result<()> {
        let has_files_table: bool = self.conn.query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='files'",
            [],
            |row| row.get(0),
        )?;

        if has_files_table {
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
                    duration_secs INTEGER NOT NULL DEFAULT 0,
                    tempo INTEGER NOT NULL DEFAULT 0,
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
                    duration_secs INTEGER NOT NULL DEFAULT 0,
                    tempo INTEGER NOT NULL DEFAULT 0,
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

    pub fn add_track(&self, track: &Track) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO tracks (id, title, artist, album, duration_secs, tempo, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                track.id,
                track.title,
                track.artist,
                track.album,
                track.duration_secs,
                track.tempo,
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
            "SELECT id, title, artist, album, duration_secs, tempo, added_at
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

    /// Update the metadata fields (title, artist, album, tempo) for a track.
    pub fn update_track(
        &self,
        id: &str,
        title: &str,
        artist: &str,
        album: &str,
        tempo: u32,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE tracks SET title = ?1, artist = ?2, album = ?3, tempo = ?4 WHERE id = ?5",
            params![title, artist, album, tempo, id],
        )?;
        Ok(())
    }

    /// Update metadata from an existing `Track` struct.
    pub fn update_track_from(&self, track: &Track) -> rusqlite::Result<()> {
        self.update_track(
            &track.id,
            &track.title,
            &track.artist,
            &track.album,
            track.tempo,
        )
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
