// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::paths;
use serde::{Deserialize, Serialize};
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
        SortKey::Title => "title".to_string(),
        SortKey::Artist => "artist".to_string(),
        SortKey::Album => "album".to_string(),
        SortKey::Duration => "duration".to_string(),
    };
    prefs.sort_order = match order {
        SortOrder::Asc => "asc".to_string(),
        SortOrder::Desc => "desc".to_string(),
    };
    prefs.save();
}

// ── Column widths ─────────────────────────────────────────────────────────────

pub fn load_col_widths() -> Option<Vec<f64>> {
    let prefs = Prefs::load();
    if prefs.col_widths.is_empty() {
        None
    } else {
        Some(prefs.col_widths)
    }
}

pub fn save_col_widths(widths: &str) {
    let parsed: Vec<f64> = widths.split(',').filter_map(|s| s.parse().ok()).collect();
    let mut prefs = Prefs::load();
    prefs.col_widths = parsed;
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
