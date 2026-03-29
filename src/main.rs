// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

mod components;
mod db;
mod ffmpeg;
mod format;
mod paths;
mod prefs;
mod scan;
mod sync;
mod task;

use components::library::Library;
use components::log::LogEntry;
use components::sync_modal::SyncModal;
use dioxus::prelude::*;
use std::path::PathBuf;
use task::spawn_blocking;

fn main() {
    let custom_head = format!(
        "<style>{}</style><style>{}</style><style>{}</style><style>{}</style><style>{}</style>",
        include_str!("../assets/theme.css"),
        include_str!("../assets/main.css"),
        include_str!("../assets/dx-components-theme.css"),
        include_str!("components/input/style.css"),
        include_str!("components/select/style.css"),
    );

    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pino");

    dioxus::LaunchBuilder::new()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_menu(None)
                .with_data_directory(data_dir)
                .with_custom_head(custom_head),
        )
        .launch(App);
}

#[component]
fn App() -> Element {
    // Load tracks from DB on startup (synchronous to avoid flicker).
    let initial_tracks = db::Library::open(&paths::db_path())
        .and_then(|lib| lib.get_all_tracks_with_files())
        .unwrap_or_default();
    let tracks = use_signal(|| initial_tracks);
    let mut sync_open = use_signal(|| false);

    // Library state.
    let scanning = use_signal(|| false);
    let lib_log_entries = use_signal(Vec::<LogEntry>::new);
    let (initial_key, initial_order) = prefs::load_sort_prefs();
    let sort_key = use_signal(|| initial_key);
    let sort_order = use_signal(|| initial_order);

    // Sync state.
    let initial_dest = prefs::load_dest_dir();
    let dest_dir = use_signal(|| initial_dest);
    let format_enabled = use_signal(|| [true, true, true, true, false]);
    let sync_convert_to_idx = use_signal(|| None::<usize>);
    let auto_convert = use_signal(|| true);
    let syncing = use_signal(|| false);
    let pulling = use_signal(|| false);
    let sync_log_entries = use_signal(Vec::<LogEntry>::new);
    let progress_phase = use_signal(String::new);
    let progress_current = use_signal(|| 0u32);
    let progress_total = use_signal(|| 0u32);
    let jobs = use_signal(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    });
    let mut sync_status = use_signal(|| None::<sync::SyncStatus>);
    let mut checking = use_signal(|| false);
    let mut dest_error = use_signal(|| None::<String>);

    // Check sync status when destination changes.
    use_effect(move || {
        let dir = dest_dir();
        if dir.is_empty() {
            sync_status.set(None);
            dest_error.set(None);
            return;
        }
        prefs::save_dest_dir(&dir);
        let dest = PathBuf::from(&dir);
        if !dest.is_dir() {
            sync_status.set(None);
            dest_error.set(Some(format!("Cannot access \"{dir}\".")));
            checking.set(false);
            return;
        }
        dest_error.set(None);
        let db = paths::db_path();
        checking.set(true);
        spawn(async move {
            match spawn_blocking(move || sync::check_sync_status(&db, &dest)).await {
                Ok(Ok(status)) => sync_status.set(Some(status)),
                Ok(Err(e)) => {
                    sync_status.set(None);
                    dest_error.set(Some(format!("Error checking device: {e}")));
                }
                Err(_) => {
                    sync_status.set(None);
                    dest_error.set(Some("Check thread panicked.".to_string()));
                }
            }
            checking.set(false);
        });
    });

    rsx! {
        div { class: "container",
            oncontextmenu: move |e: MouseEvent| e.prevent_default(),
            h1 { "pino" }
            p { class: "subtitle", "acrilique's USB exporter - powered by rekordcrate" }

            Library {
                tracks,
                scanning,
                log_entries: lib_log_entries,
                sort_key,
                sort_order,
                on_sync: move |_| sync_open.set(true),
            }

            if sync_open() {
                SyncModal {
                    tracks,
                    dest_dir,
                    format_enabled,
                    convert_to_idx: sync_convert_to_idx,
                    auto_convert,
                    syncing,
                    pulling,
                    log_entries: sync_log_entries,
                    progress_phase,
                    progress_current,
                    progress_total,
                    jobs,
                    sync_status,
                    checking,
                    dest_error,
                    on_close: move |_| sync_open.set(false),
                }
            }
        }
    }
}
