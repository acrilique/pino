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
mod scan;
mod sync;

use components::input::Input;
use components::select::*;
use dioxus::prelude::*;
use format::SupportedFormat;
use std::path::PathBuf;

const FORMATS: [(SupportedFormat, &str); 5] = [
    (SupportedFormat::Mp3, "MP3"),
    (SupportedFormat::Wav, "WAV"),
    (SupportedFormat::Aiff, "AIFF"),
    (SupportedFormat::M4a, "AAC/M4A"),
    (SupportedFormat::Flac, "FLAC"),
];

#[derive(Clone, Copy, PartialEq)]
enum SortKey {
    Title,
    Artist,
    Album,
    Duration,
}

#[derive(Clone, Copy, PartialEq)]
enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    fn toggle(self) -> Self {
        match self {
            SortOrder::Asc => SortOrder::Desc,
            SortOrder::Desc => SortOrder::Asc,
        }
    }

    fn indicator(self) -> &'static str {
        match self {
            SortOrder::Asc => " ▲",
            SortOrder::Desc => " ▼",
        }
    }
}

/// Identifies a cell being edited: (track index, column key).
#[derive(Clone, Copy, PartialEq)]
enum EditColumn {
    Title,
    Artist,
    Album,
}

#[derive(Clone, PartialEq)]
struct ContextMenu {
    x: f64,
    y: f64,
    target: ContextTarget,
}

#[derive(Clone, PartialEq)]
enum ContextTarget {
    File {
        file_id: String,
        track_id: String,
        format: String,
    },
    Track {
        track_id: String,
    },
}

fn format_label(idx: Option<usize>) -> &'static str {
    match idx {
        Some(i) => FORMATS.get(i).map_or("Select a format", |(_, name)| name),
        None => "Select a format",
    }
}

fn main() {
    let custom_head = format!(
        "<style>{}</style><style>{}</style><style>{}</style><style>{}</style>",
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
    let mut tracks = use_signal(Vec::<db::TrackWithFiles>::new);
    let mut sync_open = use_signal(|| false);

    // Library state.
    let scanning = use_signal(|| false);
    let lib_log_entries = use_signal(Vec::<LogEntry>::new);
    let sort_key = use_signal(|| SortKey::Artist);
    let sort_order = use_signal(|| SortOrder::Asc);

    // Sync state.
    let dest_dir = use_signal(String::new);
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

    // Check sync status when destination changes.
    use_effect(move || {
        let dir = dest_dir();
        if dir.is_empty() {
            sync_status.set(None);
            return;
        }
        let db = paths::db_path();
        let dest = PathBuf::from(dir);
        checking.set(true);
        spawn(async move {
            let (tx, rx) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                let _ = tx.send(sync::check_sync_status(&db, &dest));
            });
            if let Ok(Ok(status)) = rx.await {
                sync_status.set(Some(status));
            }
            checking.set(false);
        });
    });

    // Load tracks from DB on startup.
    let db = paths::db_path();
    use_effect(move || {
        let db = db.clone();
        spawn(async move {
            let (tx, rx) = tokio::sync::oneshot::channel();
            std::thread::spawn(move || {
                let result = db::Library::open(&db)
                    .and_then(|lib| lib.get_all_tracks_with_files())
                    .unwrap_or_default();
                let _ = tx.send(result);
            });
            if let Ok(t) = rx.await {
                tracks.set(t);
            }
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
                    on_close: move |_| sync_open.set(false),
                    on_refresh: move |_| {
                        let dir = dest_dir();
                        if dir.is_empty() {
                            return;
                        }
                        let db = paths::db_path();
                        let dest = PathBuf::from(dir);
                        checking.set(true);
                        spawn(async move {
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            std::thread::spawn(move || {
                                let _ = tx.send(sync::check_sync_status(&db, &dest));
                            });
                            if let Ok(Ok(status)) = rx.await {
                                sync_status.set(Some(status));
                            }
                            checking.set(false);
                        });
                    },
                }
            }
        }
    }
}

fn refresh_tracks(tracks: &mut Signal<Vec<db::TrackWithFiles>>) {
    let mut tracks = *tracks;
    let db = paths::db_path();
    spawn(async move {
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let result = db::Library::open(&db)
                .and_then(|lib| lib.get_all_tracks_with_files())
                .unwrap_or_default();
            let _ = tx.send(result);
        });
        if let Ok(t) = rx.await {
            tracks.set(t);
        }
    });
}

#[component]
fn Library(
    mut tracks: Signal<Vec<db::TrackWithFiles>>,
    mut scanning: Signal<bool>,
    mut log_entries: Signal<Vec<LogEntry>>,
    sort_key: Signal<SortKey>,
    sort_order: Signal<SortOrder>,
    on_sync: EventHandler,
) -> Element {
    // Which cell is being edited: (track_id, column).
    let mut editing = use_signal(|| None::<(String, EditColumn)>);
    let edit_value = use_signal(String::new);

    // Context menu state: position and target.
    let mut context_menu = use_signal(|| None::<ContextMenu>);

    let sorted_tracks = use_memo(move || {
        let mut list = tracks();
        let key = sort_key();
        let order = sort_order();
        list.sort_by(|a, b| {
            let cmp = match key {
                SortKey::Title => a
                    .track
                    .title
                    .to_lowercase()
                    .cmp(&b.track.title.to_lowercase()),
                SortKey::Artist => a
                    .track
                    .artist
                    .to_lowercase()
                    .cmp(&b.track.artist.to_lowercase()),
                SortKey::Album => a
                    .track
                    .album
                    .to_lowercase()
                    .cmp(&b.track.album.to_lowercase()),
                SortKey::Duration => a.track.duration_secs.cmp(&b.track.duration_secs),
            };
            match order {
                SortOrder::Asc => cmp,
                SortOrder::Desc => cmp.reverse(),
            }
        });
        list
    });

    let mut commit_edit = move || {
        let current = editing.read().clone();
        let Some((track_id, col)) = current else {
            return;
        };
        let new_val = edit_value().trim().to_string();

        // Find the track and apply locally.
        let mut w = tracks.write();
        if let Some(twf) = w.iter_mut().find(|t| t.track.id == track_id) {
            match col {
                EditColumn::Title => twf.track.title = new_val.clone(),
                EditColumn::Artist => twf.track.artist = new_val.clone(),
                EditColumn::Album => twf.track.album = new_val.clone(),
            }
            let title = twf.track.title.clone();
            let artist = twf.track.artist.clone();
            let album = twf.track.album.clone();
            let tempo = twf.track.tempo;
            let track_id = track_id.clone();
            drop(w);

            // Persist to DB in background.
            let db = paths::db_path();
            let scroll_id = track_id.clone();
            spawn(async move {
                let (tx, rx) = tokio::sync::oneshot::channel();
                std::thread::spawn(move || {
                    let _ = tx.send(db::Library::open(&db).and_then(|lib| {
                        lib.update_track(&track_id, &title, &artist, &album, tempo)
                    }));
                });
                let _ = rx.await;
            });

            // Scroll to the track's new position after re-sort.
            let js = format!(
                "setTimeout(() => document.getElementById('track-{}')?.scrollIntoView({{block:'nearest',behavior:'smooth'}}), 50)",
                scroll_id
            );
            document::eval(&js);
        }
        editing.set(None);
    };

    rsx! {
        div { class: "tab-content",

        div { class: "library-header",
            button {
                class: "add-btn",
                title: "Import tracks",
                disabled: scanning(),
                onclick: move |_| {
                    spawn(async move {
                        let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await else {
                            return;
                        };
                        let input = folder.path().to_path_buf();
                        let db = paths::db_path();
                        scanning.set(true);
                        log_entries.write().clear();
                        log_entries.write().push(LogEntry::info("Scanning..."));

                        let (tx, rx) = tokio::sync::oneshot::channel();
                        std::thread::spawn(move || {
                            let _ = tx.send(sync::import_folder(&db, &input));
                        });
                        match rx.await {
                            Ok(Ok(n)) => {
                                log_entries.write().push(LogEntry::success(
                                    &format!("Imported {n} new track(s)."),
                                ));
                                refresh_tracks(&mut tracks);
                            }
                            Ok(Err(e)) => {
                                log_entries.write().push(LogEntry::error(
                                    &format!("Import failed: {e}"),
                                ));
                            }
                            Err(_) => {
                                log_entries.write().push(LogEntry::error("Import thread panicked."));
                            }
                        }
                        scanning.set(false);
                    });
                },
                if scanning() { "..." } else { "+" }
            }
            button {
                class: "sync-btn",
                title: "Sync to USB",
                onclick: move |_| on_sync.call(()),
                "➤"
            }
            p { class: "track-count", "{tracks.read().len()} track(s) in library" }
        }

        if !log_entries.read().is_empty() {
            div { class: "log",
                for entry in log_entries() {
                    p { class: entry.class, "{entry.message}" }
                }
            }
        }

        if !tracks.read().is_empty() {
            div {
                class: "track-list",
                id: "track-list",
                onmounted: |_| {
                    document::eval(r#"
                        (function() {
                            const el = document.getElementById('track-list');
                            if (!el) return;
                            function resize() {
                                const top = el.getBoundingClientRect().top;
                                el.style.height = (window.innerHeight - top - 34) + 'px';
                            }
                            resize();
                            window.__pino_resize = resize;
                            window.addEventListener('resize', resize);
                        })()
                    "#);
                },
                table {
                    thead {
                        tr {
                            SortableHeader { label: "Title", col_key: SortKey::Title, sort_key, sort_order }
                            SortableHeader { label: "Artist", col_key: SortKey::Artist, sort_key, sort_order }
                            SortableHeader { label: "Album", col_key: SortKey::Album, sort_key, sort_order }
                            th { "Formats" }
                            SortableHeader { label: "Duration", col_key: SortKey::Duration, sort_key, sort_order }
                        }
                    }
                    tbody {
                        for twf in sorted_tracks() {
                            {
                                let track_id = twf.track.id.clone();
                                rsx! {
                                    tr {
                                        id: "track-{track_id}",
                                        oncontextmenu: {
                                            let track_id = track_id.clone();
                                            move |e: MouseEvent| {
                                                e.prevent_default();
                                                context_menu.set(Some(ContextMenu {
                                                    x: e.page_coordinates().x,
                                                    y: e.page_coordinates().y,
                                                    target: ContextTarget::Track {
                                                        track_id: track_id.clone(),
                                                    },
                                                }));
                                            }
                                        },
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Title,
                                            value: twf.track.title.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |_| commit_edit(),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Artist,
                                            value: twf.track.artist.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |_| commit_edit(),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Album,
                                            value: twf.track.album.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |_| commit_edit(),
                                        }
                                        td { class: "formats-cell",
                                            for file in &twf.files {
                                                {
                                                    let file_id = file.id.clone();
                                                    let track_id_for_file = track_id.clone();
                                                    let fmt = file.format.clone();
                                                    rsx! {
                                                        span {
                                                            class: "format-badge",
                                                            oncontextmenu: move |e: MouseEvent| {
                                                                e.prevent_default();
                                                                context_menu.set(Some(ContextMenu {
                                                                    x: e.page_coordinates().x,
                                                                    y: e.page_coordinates().y,
                                                                    target: ContextTarget::File {
                                                                        file_id: file_id.clone(),
                                                                        track_id: track_id_for_file.clone(),
                                                                        format: fmt.clone(),
                                                                    },
                                                                }));
                                                            },
                                                            "{file.format}"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        td { "{format_duration(twf.track.duration_secs)}" }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Context menu overlay.
            if let Some(menu) = context_menu() {
                div {
                    class: "context-overlay",
                    onclick: move |_| context_menu.set(None),
                    oncontextmenu: move |e: MouseEvent| {
                        e.prevent_default();
                        context_menu.set(None);
                    },
                }
                div {
                    class: "context-menu",
                    style: "left: {menu.x}px; top: {menu.y}px;",
                    match menu.target.clone() {
                        ContextTarget::File { file_id, track_id, format } => rsx! {
                            button {
                                class: "context-item danger",
                                onclick: move |_| {
                                    let file_id = file_id.clone();
                                    let track_id = track_id.clone();
                                    context_menu.set(None);

                                    // Remove from local state.
                                    let mut w = tracks.write();
                                    if let Some(twf) = w.iter_mut().find(|t| t.track.id == track_id) {
                                        twf.files.retain(|f| f.id != file_id);
                                    }
                                    drop(w);

                                    // Persist.
                                    let db = paths::db_path();
                                    spawn(async move {
                                        let (tx, rx) = tokio::sync::oneshot::channel();
                                        std::thread::spawn(move || {
                                            let _ = tx.send(
                                                db::Library::open(&db)
                                                    .and_then(|lib| lib.delete_file(&file_id)),
                                            );
                                        });
                                        let _ = rx.await;
                                    });
                                },
                                "Remove {format} file"
                            }
                        },
                        ContextTarget::Track { track_id } => rsx! {
                            button {
                                class: "context-item danger",
                                onclick: move |_| {
                                    let track_id = track_id.clone();
                                    context_menu.set(None);

                                    tracks.write().retain(|t| t.track.id != track_id);

                                    let db = paths::db_path();
                                    spawn(async move {
                                        let (tx, rx) = tokio::sync::oneshot::channel();
                                        std::thread::spawn(move || {
                                            let _ = tx.send(
                                                db::Library::open(&db)
                                                    .and_then(|lib| lib.delete_track(&track_id)),
                                            );
                                        });
                                        let _ = rx.await;
                                    });
                                },
                                "Remove track"
                            }
                        },
                    }
                }
            }
        }
        } // tab-content
    }
}

#[component]
fn SortableHeader(
    label: &'static str,
    col_key: SortKey,
    mut sort_key: Signal<SortKey>,
    mut sort_order: Signal<SortOrder>,
) -> Element {
    let is_active = sort_key() == col_key;
    rsx! {
        th {
            class: if is_active { "sortable active" } else { "sortable" },
            onclick: move |_| {
                if sort_key() == col_key {
                    sort_order.set(sort_order().toggle());
                } else {
                    sort_key.set(col_key);
                    sort_order.set(SortOrder::Asc);
                }
            },
            "{label}"
            if is_active {
                span { class: "sort-indicator", "{sort_order().indicator()}" }
            }
        }
    }
}

#[component]
fn EditableCell(
    track_id: String,
    column: EditColumn,
    value: String,
    mut editing: Signal<Option<(String, EditColumn)>>,
    mut edit_value: Signal<String>,
    on_commit: EventHandler,
) -> Element {
    let col_suffix = match column {
        EditColumn::Title => "title",
        EditColumn::Artist => "artist",
        EditColumn::Album => "album",
    };
    let input_id = format!("edit-{track_id}-{col_suffix}");

    let is_editing = editing
        .read()
        .as_ref()
        .is_some_and(|(id, col)| id == &track_id && *col == column);

    if is_editing {
        rsx! {
            td { class: "editing-cell",
                input {
                    r#type: "text",
                    id: "{input_id}",
                    class: "cell-edit-input",
                    value: "{edit_value}",
                    autofocus: true,
                    oninput: move |e: FormEvent| edit_value.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter {
                            on_commit.call(());
                        } else if e.key() == Key::Escape {
                            editing.set(None);
                        }
                    },
                    onblur: move |_| {
                        on_commit.call(());
                    },
                }
            }
        }
    } else {
        let tid = track_id.clone();
        let val = value.clone();
        let focus_id = input_id.clone();
        rsx! {
            td {
                class: "editable-cell",
                onclick: move |_| {
                    editing.set(Some((tid.clone(), column)));
                    edit_value.set(val.clone());

                    let js = format!(
                        "setTimeout(() => {{ const el = document.getElementById('{}'); if (el) {{ el.focus(); el.select?.(); }} }}, 0)",
                        focus_id
                    );
                    document::eval(&js);
                },
                "{value}"
            }
        }
    }
}

fn format_duration(secs: u16) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

#[component]
fn SyncModal(
    mut tracks: Signal<Vec<db::TrackWithFiles>>,
    dest_dir: Signal<String>,
    mut format_enabled: Signal<[bool; 5]>,
    mut convert_to_idx: Signal<Option<usize>>,
    mut auto_convert: Signal<bool>,
    mut syncing: Signal<bool>,
    mut pulling: Signal<bool>,
    mut log_entries: Signal<Vec<LogEntry>>,
    mut progress_phase: Signal<String>,
    mut progress_current: Signal<u32>,
    mut progress_total: Signal<u32>,
    mut jobs: Signal<usize>,
    mut sync_status: Signal<Option<sync::SyncStatus>>,
    checking: Signal<bool>,
    on_close: EventHandler,
    on_refresh: EventHandler,
) -> Element {
    // Count tracks that need conversion for the currently selected formats.
    let need_conversion = use_memo(move || {
        let enabled = *format_enabled.read();
        let supported: Vec<SupportedFormat> = FORMATS
            .iter()
            .enumerate()
            .filter(|(i, _)| enabled[*i])
            .map(|(_, (fmt, _))| *fmt)
            .collect();

        if supported.is_empty() {
            return 0u32;
        }

        tracks
            .read()
            .iter()
            .filter(|twf| {
                !twf.files.iter().any(|f| {
                    SupportedFormat::try_from(f.format.as_str())
                        .is_ok_and(|fmt| supported.contains(&fmt))
                })
            })
            .count() as u32
    });

    let has_dest = !dest_dir().is_empty();
    let status = sync_status();
    let to_push = status.map_or(0, |s| s.to_push);
    let to_pull = status.map_or(0, |s| s.to_pull);
    let up_to_date = status.is_some_and(|s| s.to_push == 0 && s.to_pull == 0);

    let busy = syncing() || pulling();

    rsx! {
        div {
            class: "modal-backdrop",
            onclick: move |_| {
                if !busy {
                    on_close.call(());
                }
            },
        }
        div { class: "modal",
            div { class: "modal-header",
                h2 { "Sync to USB" }
                div { class: "modal-header-actions",
                    button {
                        class: "modal-refresh",
                        disabled: busy || checking(),
                        title: "Re-check device",
                        onclick: move |_| on_refresh.call(()),
                        "↻"
                    }
                    button {
                        class: "modal-close",
                        disabled: busy,
                        onclick: move |_| on_close.call(()),
                        "×"
                    }
                }
            }
            div { class: "modal-body",

        DirField {
            label: "Destination".to_string(),
            value: dest_dir,
            placeholder: "/path/to/usb".to_string(),
        }

        if has_dest {
            if checking() {
                div { class: "sync-status checking",
                    span { class: "status-icon", "⟳" }
                    p { "Checking device..." }
                }
            } else if up_to_date && !busy && log_entries.read().is_empty() {
                div { class: "sync-status up-to-date",
                    span { class: "status-icon", "✓" }
                    p { "Everything is up to date." }
                }
            } else if status.is_some() && !up_to_date {
                div { class: "sync-status has-changes",
                    if to_push > 0 {
                        div { class: "status-row",
                            span { class: "status-icon push", "↑" }
                            p { "{to_push} track(s) to push to device" }
                        }
                    }
                    if to_pull > 0 {
                        div { class: "status-row",
                            span { class: "status-icon pull", "↓" }
                            p { "{to_pull} track(s) on device not in your library" }
                        }
                    }
                }
            }
        }

        if has_dest && to_pull > 0 && !checking() {
            div { class: "pull-section",
                button {
                    class: "pull-btn",
                    disabled: pulling() || syncing(),
                    onclick: move |_| {
                        let db = paths::db_path();
                        let dest = PathBuf::from(dest_dir());
                        pulling.set(true);
                        log_entries.write().clear();
                        log_entries.write().push(LogEntry::info("Pulling tracks from device..."));
                        progress_phase.set(String::new());
                        progress_current.set(0);
                        progress_total.set(0);

                        spawn(async move {
                            let (result_tx, result_rx) = tokio::sync::oneshot::channel();
                            let (ptx, mut prx) = tokio::sync::mpsc::channel::<sync::SyncProgress>(64);

                            std::thread::spawn(move || {
                                let callback = move |p: sync::SyncProgress| {
                                    let _ = ptx.blocking_send(p);
                                };
                                let result = sync::pull_from_remote(&db, &dest, &callback);
                                let _ = result_tx.send(result);
                            });

                            while let Some(p) = prx.recv().await {
                                progress_phase.set(p.phase.to_string());
                                progress_current.set(p.current);
                                progress_total.set(p.total);
                            }

                            match result_rx.await {
                                Ok(Ok(n)) => {
                                    log_entries.write().push(LogEntry::success(
                                        &format!("Pulled {n} track(s) from device."),
                                    ));
                                    refresh_tracks(&mut tracks);
                                    // Clear the pull count.
                                    if let Some(mut s) = sync_status() {
                                        s.to_pull = 0;
                                        sync_status.set(Some(s));
                                    }
                                }
                                Ok(Err(e)) => {
                                    log_entries.write().push(LogEntry::error(
                                        &format!("Pull failed: {e}"),
                                    ));
                                }
                                Err(_) => {
                                    log_entries.write().push(LogEntry::error("Pull thread panicked."));
                                }
                            }
                            pulling.set(false);
                        });
                    },
                    if pulling() { "Pulling..." } else { "Pull from device" }
                }
            }
        }

        if has_dest && to_push > 0 && !checking() {
            div { class: "field",
                label { "Allowed Formats" }
                div { class: "formats",
                    for (i, (_, name)) in FORMATS.iter().enumerate() {
                        label { class: "checkbox-label",
                            input {
                                r#type: "checkbox",
                                checked: format_enabled.read()[i],
                                oninput: move |_| {
                                    let current = format_enabled.read()[i];
                                    format_enabled.write()[i] = !current;
                                },
                            }
                            "{name}"
                        }
                    }
                }
            }

            if need_conversion() > 0 {
                div { class: "warning",
                    p {
                        "{need_conversion()} track(s) have no file in any of the selected formats."
                    }
                    div { class: "field",
                        label { class: "checkbox-label",
                            input {
                                r#type: "checkbox",
                                checked: auto_convert(),
                                oninput: move |_| auto_convert.set(!auto_convert()),
                            }
                            "Convert them during sync to"
                        }
                        if auto_convert() {
                            Select::<usize> {
                                on_value_change: move |val: Option<usize>| {
                                    convert_to_idx.set(val);
                                },
                                SelectTrigger {
                                    {format_label(convert_to_idx())}
                                }
                                SelectList {
                                    for (i, (_, name)) in FORMATS.iter().enumerate() {
                                        if format_enabled.read()[i] {
                                            SelectOption::<usize> {
                                                value: i,
                                                index: i,
                                                text_value: name.to_string(),
                                                "{name}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if auto_convert() {
                        div { class: "field",
                            label { "Parallel jobs" }
                            div { class: "dir-row",
                                input {
                                    r#type: "number",
                                    min: "1",
                                    max: "32",
                                    value: "{jobs}",
                                    class: "jobs-input",
                                    oninput: move |e: FormEvent| {
                                        if let Ok(n) = e.value().parse::<usize>() {
                                            jobs.set(n.clamp(1, 32));
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
            }

            button {
                class: "export-btn",
                disabled: syncing() || pulling(),
                onclick: move |_| {
                    let enabled = *format_enabled.read();
                    let supported: Vec<SupportedFormat> = FORMATS
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| enabled[*i])
                        .map(|(_, (fmt, _))| *fmt)
                        .collect();
                    let dest = PathBuf::from(dest_dir());

                    if supported.is_empty() {
                        log_entries
                            .write()
                            .push(LogEntry::error("Select at least one allowed format."));
                        return;
                    }

                    let convert_to = if auto_convert() && need_conversion() > 0 {
                        let Some(idx) = convert_to_idx() else {
                            log_entries.write().push(LogEntry::error(
                                "Select a conversion target format.",
                            ));
                            return;
                        };
                        let fmt = FORMATS[idx].0;
                        if !supported.contains(&fmt) {
                            log_entries.write().push(LogEntry::error(
                                "Conversion target must be one of the allowed formats.",
                            ));
                            return;
                        }
                        Some(fmt)
                    } else {
                        None
                    };

                    let config = sync::SyncConfig {
                        supported_formats: supported,
                        convert_to,
                        jobs: jobs(),
                    };

                    let db = paths::db_path();
                    log_entries.write().clear();
                    progress_phase.set(String::new());
                    progress_current.set(0);
                    progress_total.set(0);
                    syncing.set(true);

                    spawn(async move {
                        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
                        let (ptx, mut prx) = tokio::sync::mpsc::channel::<sync::SyncProgress>(64);

                        std::thread::spawn(move || {
                            let callback = move |p: sync::SyncProgress| {
                                let _ = ptx.blocking_send(p);
                            };
                            let result = sync::sync(&db, &dest, &config, &callback);
                            let _ = result_tx.send(result);
                        });

                        // Process progress updates until the channel closes.
                        while let Some(p) = prx.recv().await {
                            progress_phase.set(p.phase.to_string());
                            progress_current.set(p.current);
                            progress_total.set(p.total);
                        }

                        match result_rx.await {
                            Ok(Ok(result)) => {
                                log_entries
                                    .write()
                                    .push(LogEntry::success(&result.to_string()));
                                if result.converted > 0 {
                                    refresh_tracks(&mut tracks);
                                }
                                // Clear the push count.
                                if let Some(mut s) = sync_status() {
                                    s.to_push = 0;
                                    sync_status.set(Some(s));
                                }
                            }
                            Ok(Err(e)) => {
                                log_entries
                                    .write()
                                    .push(LogEntry::error(&format!("Sync failed: {e}")));
                            }
                            Err(_) => {
                                log_entries
                                    .write()
                                    .push(LogEntry::error("Sync thread panicked."));
                            }
                        }
                        syncing.set(false);
                    });
                },
                if syncing() { "Pushing..." } else { "Push to device" }
            }
        }

        if (syncing() || pulling()) && progress_total() > 0 {
            div { class: "progress-section",
                p { class: "progress-label", "{progress_phase()}" }
                div { class: "progress-bar",
                    div {
                        class: "progress-fill",
                        width: "{progress_current() as f64 / progress_total() as f64 * 100.0}%",
                    }
                }
                p { class: "progress-count", "{progress_current()}/{progress_total()}" }
            }
        }

        if !log_entries.read().is_empty() {
            div { class: "log",
                for entry in log_entries() {
                    p { class: entry.class, "{entry.message}" }
                }
            }
        }

        } // modal-body
        } // modal
    }
}

#[derive(Clone, PartialEq)]
struct LogEntry {
    message: String,
    class: &'static str,
}

impl LogEntry {
    fn info(msg: &str) -> Self {
        Self {
            message: msg.to_string(),
            class: "info",
        }
    }
    fn success(msg: &str) -> Self {
        Self {
            message: msg.to_string(),
            class: "success",
        }
    }
    fn error(msg: &str) -> Self {
        Self {
            message: msg.to_string(),
            class: "error",
        }
    }
}

#[component]
fn DirField(label: String, mut value: Signal<String>, placeholder: String) -> Element {
    rsx! {
        div { class: "field",
            label { "{label}" }
            div { class: "dir-row",
                Input {
                    r#type: "text",
                    value: "{value}",
                    placeholder: "{placeholder}",
                    oninput: move |e: FormEvent| value.set(e.value()),
                }
                button {
                    onclick: move |_| {
                        spawn(async move {
                            if let Some(folder) =
                                rfd::AsyncFileDialog::new().pick_folder().await
                            {
                                value.set(
                                    folder.path().to_string_lossy().to_string(),
                                );
                            }
                        });
                    },
                    "Browse"
                }
            }
        }
    }
}
