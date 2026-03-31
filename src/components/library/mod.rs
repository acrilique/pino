// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

mod editable_cell;
mod sortable_header;

use crate::components::log::{LogEntry, LogPanel, log_task_result};
use crate::prefs::{self, SortKey, SortOrder};
use crate::task::{db_op, spawn_blocking};
use crate::{db, paths, sync};
use dioxus::prelude::*;

pub use editable_cell::EditColumn;
use editable_cell::EditableCell;
use sortable_header::SortableHeader;

pub fn refresh_tracks(tracks: &mut Signal<Vec<db::TrackWithFiles>>) {
    let mut tracks = *tracks;
    spawn(async move {
        let t = db_op(db::Library::get_all_tracks_with_files)
            .await
            .unwrap_or_default();
        tracks.set(t);
    });
}

fn format_duration(secs: u16) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

const JS_TRACK_LIST_INIT: &str = include_str!("../../../assets/track-list.js");

const JS_COL_WIDTH_LISTENER: &str =
    r"window.__pino_save_widths = function(csv) { dioxus.send(csv); };";

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

#[component]
pub fn Library(
    mut tracks: Signal<Vec<db::TrackWithFiles>>,
    mut scanning: Signal<bool>,
    mut log_entries: Signal<Vec<LogEntry>>,
    sort_key: Signal<SortKey>,
    sort_order: Signal<SortOrder>,
    on_sync: EventHandler,
) -> Element {
    let mut editing = use_signal(|| None::<(String, EditColumn)>);
    let edit_value = use_signal(String::new);
    let mut context_menu = use_signal(|| None::<ContextMenu>);
    let col_widths = prefs::load_col_widths().unwrap_or_default();

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

        let mut w = tracks.write();
        if let Some(twf) = w.iter_mut().find(|t| t.track.id == track_id) {
            match col {
                EditColumn::Title => twf.track.title.clone_from(&new_val),
                EditColumn::Artist => twf.track.artist.clone_from(&new_val),
                EditColumn::Album => twf.track.album.clone_from(&new_val),
            }
            let track = twf.track.clone();
            drop(w);

            let scroll_id = track.id.clone();
            spawn(async move {
                let _ = db_op(move |lib| lib.update_track_from(&track)).await;
            });

            let js = format!(
                "setTimeout(() => document.getElementById('track-{scroll_id}')?.scrollIntoView({{block:'nearest',behavior:'smooth'}}), 50)"
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

                        if log_task_result(
                            log_entries,
                            spawn_blocking(move || sync::import_folder(&db, &input)).await,
                            |r: &sync::ImportResult| format!("Imported {} new track(s).", r.imported),
                            "Import",
                        )
                        .is_some_and(|r| {
                            for w in &r.warnings {
                                log_entries.write().push(LogEntry::warning(w));
                            }
                            r.imported > 0
                        })
                        {
                            refresh_tracks(&mut tracks);
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

        LogPanel { entries: log_entries }

        if !tracks.read().is_empty() {
            div {
                class: "track-list",
                id: "track-list",
                onmounted: |_| {
                    document::eval(JS_TRACK_LIST_INIT);

                    let mut col_eval = document::eval(JS_COL_WIDTH_LISTENER);
                    spawn(async move {
                        while let Ok(widths) = col_eval.recv::<String>().await {
                            prefs::save_col_widths(&widths);
                        }
                    });
                },
                table {
                    thead {
                        tr {
                            SortableHeader { label: "Title", col_key: SortKey::Title, sort_key, sort_order, resizable: true, initial_width: col_widths.first().map(|w| format!("{w}%")) }
                            SortableHeader { label: "Artist", col_key: SortKey::Artist, sort_key, sort_order, resizable: true, initial_width: col_widths.get(1).map(|w| format!("{w}%")) }
                            SortableHeader { label: "Album", col_key: SortKey::Album, sort_key, sort_order, resizable: true, initial_width: col_widths.get(2).map(|w| format!("{w}%")) }
                            th {
                                width: col_widths.get(3).map(|w| format!("{w}%")),
                                "Formats"
                                div {
                                    class: "col-resizer",
                                    onclick: |e: MouseEvent| e.stop_propagation(),
                                }
                            }
                            SortableHeader { label: "Duration", col_key: SortKey::Duration, sort_key, sort_order, resizable: false, initial_width: col_widths.get(4).map(|w| format!("{w}%")) }
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
                                            on_commit: move |()| commit_edit(),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Artist,
                                            value: twf.track.artist.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Album,
                                            value: twf.track.album.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
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

                                    let mut w = tracks.write();
                                    if let Some(twf) = w.iter_mut().find(|t| t.track.id == track_id) {
                                        twf.files.retain(|f| f.id != file_id);
                                    }
                                    drop(w);

                                    spawn(async move {
                                        let _ = db_op(move |lib| lib.delete_file(&file_id)).await;
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

                                    spawn(async move {
                                        let _ = db_op(move |lib| lib.delete_track(&track_id)).await;
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
