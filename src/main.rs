// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

mod components;
mod db;
mod ffmpeg;
mod format;
mod paths;
mod prefs;
mod sync;
mod task;

use components::library::Library;
use components::sync_modal::{SyncModal, SyncState, check_device};
use dioxus::prelude::*;

fn main() {
    let custom_head = format!(
        "<style>{}</style><style>{}</style><style>{}</style><style>{}</style><style>{}</style>",
        include_str!("../assets/theme.css"),
        include_str!("../assets/main.css"),
        include_str!("../assets/dx-components-theme.css"),
        include_str!("components/input/style.css"),
        include_str!("components/select/style.css"),
    );

    let data_dir = paths::data_dir();

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
    let (initial_key, initial_order) = prefs::load_sort_prefs();
    let sort_key = use_signal(|| initial_key);
    let sort_order = use_signal(|| initial_order);

    // Sync state shared via context.
    use_context_provider(SyncState::new);
    let state = use_context::<SyncState>();

    // Check sync status when destination changes.
    use_effect(move || {
        let dir = (state.dest_dir)();
        if !dir.is_empty() {
            prefs::save_dest_dir(&dir);
        }
        check_device(&state);
    });

    rsx! {
        div { class: "container",
            oncontextmenu: move |e: MouseEvent| e.prevent_default(),
            h1 { "pino" }
            p { class: "subtitle", "acrilique's USB exporter - powered by rekordcrate" }

            Library {
                tracks,
                scanning,
                sort_key,
                sort_order,
                on_sync: move |()| sync_open.set(true),
            }

            if sync_open() {
                SyncModal {
                    tracks,
                    on_close: move |()| sync_open.set(false),
                }
            }
        }
    }
}
