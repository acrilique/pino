// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

mod bridge;
mod components;
mod ffmpeg;
mod format;
mod library;
mod paths;
mod prefs;
mod sync;
mod task;

use std::sync::{Arc, OnceLock};

use components::library::Library as LibraryView;
use components::sync_modal::{SyncModal, SyncState, check_device};
use dioxus::prelude::*;

// Both initialized in main() before Dioxus (and its Tokio runtime) starts, so
// Library's internal block_on never races with an outer async context.
static LIBRARY: OnceLock<Arc<library::Library>> = OnceLock::new();
static INITIAL_TRACKS: OnceLock<Vec<bridge::TrackView>> = OnceLock::new();

fn main() {
    let lib = Arc::new(library::Library::open(&paths::db_dir()).expect("open library"));
    let initial_tracks = lib.all_tracks().unwrap_or_default();
    LIBRARY.set(lib).ok();
    INITIAL_TRACKS.set(initial_tracks).ok();

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
    // Share the already-opened library via Dioxus context.
    let lib = LIBRARY
        .get()
        .expect("library initialized before launch")
        .clone();
    use_context_provider({
        let lib = lib.clone();
        move || lib.clone()
    });

    // Load tracks on startup — fetched in main() before Dioxus's runtime starts.
    let initial_tracks = INITIAL_TRACKS.get().cloned().unwrap_or_default();
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

            LibraryView {
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
