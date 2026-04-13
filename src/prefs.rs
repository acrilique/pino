// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

const PREFS_FILE: &str = "prefs.toml";

// ── Serialized prefs ──────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Default)]
struct Prefs {
    #[serde(rename = "sortKey", default)]
    sort_key: String,
    #[serde(rename = "sortOrder", default)]
    sort_order: String,
    #[serde(rename = "colWidths", default)]
    col_widths: Vec<f64>,
    #[serde(rename = "destDir", default)]
    dest_dir: String,
    #[serde(rename = "hiddenColumns", default)]
    hidden_columns: Vec<String>,
}

impl Prefs {
    fn load() -> Self {
        let path = pref_path();
        toml::from_str(&std::fs::read_to_string(&path).unwrap_or_default()).unwrap_or_default()
    }

    fn save(&self) {
        let path = pref_path();
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        let _ = std::fs::write(&path, toml::to_string(self).unwrap_or_default());
    }
}

fn pref_path() -> PathBuf {
    paths::data_dir().join(PREFS_FILE)
}

// ── Sort preferences ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum SortKey {
    Title,
    Artist,
    Album,
    Duration,
    Genre,
    Composer,
    Label,
    Remixer,
    Key,
    Comment,
    Isrc,
    Lyricist,
    MixName,
    ReleaseDate,
    Bpm,
    Year,
    TrackNumber,
    DiscNumber,
    Rating,
    Color,
    AddedAt,
    FileName,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    pub fn toggle(self) -> Self {
        match self {
            SortOrder::Asc => SortOrder::Desc,
            SortOrder::Desc => SortOrder::Asc,
        }
    }

    pub fn indicator(self) -> &'static str {
        match self {
            SortOrder::Asc => " ▲",
            SortOrder::Desc => " ▼",
        }
    }
}

pub fn load_sort_prefs() -> (SortKey, SortOrder) {
    let prefs = Prefs::load();
    let key = match prefs.sort_key.as_str() {
        "title" => SortKey::Title,
        "album" => SortKey::Album,
        "duration" => SortKey::Duration,
        "genre" => SortKey::Genre,
        "composer" => SortKey::Composer,
        "label" => SortKey::Label,
        "remixer" => SortKey::Remixer,
        "key" => SortKey::Key,
        "comment" => SortKey::Comment,
        "isrc" => SortKey::Isrc,
        "lyricist" => SortKey::Lyricist,
        "mix_name" => SortKey::MixName,
        "release_date" => SortKey::ReleaseDate,
        "bpm" => SortKey::Bpm,
        "year" => SortKey::Year,
        "track_number" => SortKey::TrackNumber,
        "disc_number" => SortKey::DiscNumber,
        "rating" => SortKey::Rating,
        "color" => SortKey::Color,
        "added_at" => SortKey::AddedAt,
        "file_name" => SortKey::FileName,
        _ => SortKey::Artist,
    };
    let order = match prefs.sort_order.as_str() {
        "desc" => SortOrder::Desc,
        _ => SortOrder::Asc,
    };
    (key, order)
}

pub fn save_sort_prefs(key: SortKey, order: SortOrder) {
    let mut prefs = Prefs::load();
    prefs.sort_key = match key {
        SortKey::Title => "title",
        SortKey::Artist => "artist",
        SortKey::Album => "album",
        SortKey::Duration => "duration",
        SortKey::Genre => "genre",
        SortKey::Composer => "composer",
        SortKey::Label => "label",
        SortKey::Remixer => "remixer",
        SortKey::Key => "key",
        SortKey::Comment => "comment",
        SortKey::Isrc => "isrc",
        SortKey::Lyricist => "lyricist",
        SortKey::MixName => "mix_name",
        SortKey::ReleaseDate => "release_date",
        SortKey::Bpm => "bpm",
        SortKey::Year => "year",
        SortKey::TrackNumber => "track_number",
        SortKey::DiscNumber => "disc_number",
        SortKey::Rating => "rating",
        SortKey::Color => "color",
        SortKey::AddedAt => "added_at",
        SortKey::FileName => "file_name",
    }
    .to_string();
    prefs.sort_order = match order {
        SortOrder::Asc => "asc".to_string(),
        SortOrder::Desc => "desc".to_string(),
    };
    prefs.save();
}

// ── Destination directory ─────────────────────────────────────────────────────

pub fn load_dest_dir() -> String {
    Prefs::load().dest_dir
}

pub fn save_dest_dir(dir: &str) {
    let mut prefs = Prefs::load();
    prefs.dest_dir = dir.to_string();
    prefs.save();
}

// ── Column visibility ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Column {
    Title,
    Artist,
    Album,
    Formats,
    Duration,
    Genre,
    Composer,
    Label,
    Remixer,
    Key,
    Comment,
    Isrc,
    Lyricist,
    MixName,
    ReleaseDate,
    Bpm,
    Year,
    TrackNumber,
    DiscNumber,
    Rating,
    Color,
    AddedAt,
    FileName,
}

impl Column {
    pub const ALL: &[Column] = &[
        Column::Title,
        Column::Artist,
        Column::Album,
        Column::Formats,
        Column::Duration,
        Column::Genre,
        Column::Composer,
        Column::Label,
        Column::Remixer,
        Column::Key,
        Column::Comment,
        Column::Isrc,
        Column::Lyricist,
        Column::MixName,
        Column::ReleaseDate,
        Column::Bpm,
        Column::Year,
        Column::TrackNumber,
        Column::DiscNumber,
        Column::Rating,
        Column::Color,
        Column::AddedAt,
        Column::FileName,
    ];

    /// Columns that are hidden unless the user previously toggled them.
    const HIDDEN_BY_DEFAULT: &[Column] = &[
        Column::Genre,
        Column::Composer,
        Column::Label,
        Column::Remixer,
        Column::Key,
        Column::Comment,
        Column::Isrc,
        Column::Lyricist,
        Column::MixName,
        Column::ReleaseDate,
        Column::Bpm,
        Column::Year,
        Column::TrackNumber,
        Column::DiscNumber,
        Column::Rating,
        Column::Color,
        Column::AddedAt,
        Column::FileName,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Column::Title => "Title",
            Column::Artist => "Artist",
            Column::Album => "Album",
            Column::Formats => "Formats",
            Column::Duration => "Duration",
            Column::Genre => "Genre",
            Column::Composer => "Composer",
            Column::Label => "Label",
            Column::Remixer => "Remixer",
            Column::Key => "Key",
            Column::Comment => "Comment",
            Column::Isrc => "ISRC",
            Column::Lyricist => "Lyricist",
            Column::MixName => "Mix Name",
            Column::ReleaseDate => "Release Date",
            Column::Bpm => "BPM",
            Column::Year => "Year",
            Column::TrackNumber => "Track #",
            Column::DiscNumber => "Disc #",
            Column::Rating => "Rating",
            Column::Color => "Color",
            Column::AddedAt => "Added At",
            Column::FileName => "File Name",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Column::Title => "title",
            Column::Artist => "artist",
            Column::Album => "album",
            Column::Formats => "formats",
            Column::Duration => "duration",
            Column::Genre => "genre",
            Column::Composer => "composer",
            Column::Label => "label",
            Column::Remixer => "remixer",
            Column::Key => "key",
            Column::Comment => "comment",
            Column::Isrc => "isrc",
            Column::Lyricist => "lyricist",
            Column::MixName => "mix_name",
            Column::ReleaseDate => "release_date",
            Column::Bpm => "bpm",
            Column::Year => "year",
            Column::TrackNumber => "track_number",
            Column::DiscNumber => "disc_number",
            Column::Rating => "rating",
            Column::Color => "color",
            Column::AddedAt => "added_at",
            Column::FileName => "file_name",
        }
    }

    fn from_str(s: &str) -> Option<Column> {
        match s {
            "title" => Some(Column::Title),
            "artist" => Some(Column::Artist),
            "album" => Some(Column::Album),
            "formats" => Some(Column::Formats),
            "duration" => Some(Column::Duration),
            "genre" => Some(Column::Genre),
            "composer" => Some(Column::Composer),
            "label" => Some(Column::Label),
            "remixer" => Some(Column::Remixer),
            "key" => Some(Column::Key),
            "comment" => Some(Column::Comment),
            "isrc" => Some(Column::Isrc),
            "lyricist" => Some(Column::Lyricist),
            "mix_name" => Some(Column::MixName),
            "release_date" => Some(Column::ReleaseDate),
            "bpm" => Some(Column::Bpm),
            "year" => Some(Column::Year),
            "track_number" => Some(Column::TrackNumber),
            "disc_number" => Some(Column::DiscNumber),
            "rating" => Some(Column::Rating),
            "color" => Some(Column::Color),
            "added_at" => Some(Column::AddedAt),
            "file_name" => Some(Column::FileName),
            _ => None,
        }
    }
}

pub fn load_hidden_columns() -> HashSet<Column> {
    let prefs = Prefs::load();
    if prefs.hidden_columns.is_empty() {
        // First launch / no explicit config: hide new columns by default.
        Column::HIDDEN_BY_DEFAULT.iter().copied().collect()
    } else {
        prefs
            .hidden_columns
            .iter()
            .filter_map(|s| Column::from_str(s))
            .collect()
    }
}

pub fn save_hidden_columns(hidden: &HashSet<Column>) {
    let mut prefs = Prefs::load();
    prefs.hidden_columns = hidden.iter().map(|c| c.as_str().to_string()).collect();
    prefs.save();
}
