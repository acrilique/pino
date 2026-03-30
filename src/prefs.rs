// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::paths;
use std::path::PathBuf;

fn pref_path(name: &str) -> PathBuf {
    paths::data_dir().join(name)
}

fn load_pref(name: &str) -> Option<String> {
    std::fs::read_to_string(pref_path(name)).ok()
}

fn save_pref(name: &str, content: &str) {
    let path = pref_path(name);
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, content);
}

// --- Sort preferences ---

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
    let content = load_pref("sort_prefs.txt").unwrap_or_default();
    let mut parts = content.trim().splitn(2, '|');
    let key = match parts.next().unwrap_or("") {
        "title" => SortKey::Title,
        "album" => SortKey::Album,
        "duration" => SortKey::Duration,
        _ => SortKey::Artist,
    };
    let order = match parts.next().unwrap_or("") {
        "desc" => SortOrder::Desc,
        _ => SortOrder::Asc,
    };
    (key, order)
}

pub fn save_sort_prefs(key: SortKey, order: SortOrder) {
    let key_str = match key {
        SortKey::Title => "title",
        SortKey::Artist => "artist",
        SortKey::Album => "album",
        SortKey::Duration => "duration",
    };
    let order_str = match order {
        SortOrder::Asc => "asc",
        SortOrder::Desc => "desc",
    };
    save_pref("sort_prefs.txt", &format!("{key_str}|{order_str}"));
}

// --- Column widths ---

pub fn load_col_widths() -> Option<Vec<f64>> {
    let content = load_pref("col_widths.txt")?;
    let widths: Vec<f64> = content
        .trim()
        .split(',')
        .filter_map(|s| s.parse().ok())
        .collect();
    if widths.is_empty() {
        None
    } else {
        Some(widths)
    }
}

pub fn save_col_widths(widths: &str) {
    save_pref("col_widths.txt", widths);
}

// --- Destination directory ---

pub fn load_dest_dir() -> String {
    load_pref("dest_dir.txt")
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub fn save_dest_dir(dir: &str) {
    save_pref("dest_dir.txt", dir);
}
