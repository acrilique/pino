// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

mod editable_cell;
mod sortable_header;

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
    sort_key: Signal<SortKey>,
    sort_order: Signal<SortOrder>,
    on_sync: EventHandler,
) -> Element {
    let mut editing = use_signal(|| None::<(String, EditColumn)>);
    let edit_value = use_signal(String::new);
    let mut context_menu = use_signal(|| None::<ContextMenu>);
    let mut import_warnings: Signal<Vec<String>> = use_signal(Vec::new);
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
                class: "icon-btn",
                title: "Import folder",
                disabled: scanning(),
                onclick: move |_| {
                    spawn(async move {
                        let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await else {
                            return;
                        };
                        let input = folder.path().to_path_buf();
                        let db = paths::db_path();
                        scanning.set(true);

                        if let Ok(Ok(r)) = spawn_blocking(move || sync::import_folder(&db, &input)).await {
                            if !r.warnings.is_empty() {
                                import_warnings.set(r.warnings);
                            }
                            if r.imported > 0 {
                                refresh_tracks(&mut tracks);
                            }
                        }
                        scanning.set(false);
                    });
                },
                span {
                    svg {
                        width: "16",
                        height: "16",
                        view_box: "0 0 24 24",
                        fill: "none",
                        path {
                            d: "M13 7L11.8845 4.76892C11.5634 4.1268 11.4029 3.80573 11.1634 3.57116C10.9516 3.36373 10.6963 3.20597 10.4161 3.10931C10.0992 3 9.74021 3 9.02229 3H5.2C4.0799 3 3.51984 3 3.09202 3.21799C2.71569 3.40973 2.40973 3.71569 2.21799 4.09202C2 4.51984 2 5.0799 2 6.2V7M2 7H17.2C18.8802 7 19.7202 7 20.362 7.32698C20.9265 7.6146 21.3854 8.07354 21.673 8.63803C22 9.27976 22 10.1198 22 11.8V16.2C22 17.8802 22 18.7202 21.673 19.362C21.3854 19.9265 20.9265 20.3854 20.362 20.673C19.7202 21 18.8802 21 17.2 21H6.8C5.11984 21 4.27976 21 3.63803 20.673C3.07354 20.3854 2.6146 19.9265 2.32698 19.362C2 18.7202 2 17.8802 2 16.2V7ZM12 17V11M9 14H15",
                            stroke: "currentColor",
                            stroke_width: "2",
                            stroke_linecap: "round",
                            stroke_linejoin: "round"
                        }
                    }
                }
            }
            button {
                class: "icon-btn",
                title: "Import files",
                disabled: scanning(),
                onclick: move |_| {
                    spawn(async move {
                        let Some(files) = rfd::AsyncFileDialog::new()
                            .add_filter("Audio files", &[
                                "mp3", "wav", "aiff", "aif", "aac", "m4a",
                                "flac", "ogg", "wma", "opus",
                            ])
                            .pick_files()
                            .await
                        else {
                            return;
                        };
                        let paths: Vec<_> = files.iter().map(|f| f.path().to_path_buf()).collect();
                        let db = paths::db_path();
                        scanning.set(true);

                        if let Ok(Ok(r)) = spawn_blocking(move || sync::import_files(&db, paths)).await {
                            if !r.warnings.is_empty() {
                                import_warnings.set(r.warnings);
                            }
                            if r.imported > 0 {
                                refresh_tracks(&mut tracks);
                            }
                        }
                        scanning.set(false);
                    });
                },
                span {
                    svg {
                        width: "16",
                        height: "16",
                        view_box: "0 0 24 24",
                        fill: "none",
                        path {
                            d: "M14.5 18V5.58888C14.5 4.73166 14.5 4.30306 14.6805 4.04492C14.8382 3.81952 15.0817 3.669 15.3538 3.6288C15.6655 3.58276 16.0488 3.77444 16.8155 4.1578L20.5 6.00003M14.5 18C14.5 19.6569 13.1569 21 11.5 21C9.84315 21 8.5 19.6569 8.5 18C8.5 16.3432 9.84315 15 11.5 15C13.1569 15 14.5 16.3432 14.5 18ZM6.5 10V4.00003M3.5 7.00003H9.5",
                            stroke: "currentColor",
                            stroke_width: "2",
                            stroke_linecap: "round",
                            stroke_linejoin: "round"
                        }
                    }
                }
            }
            button {
                class: "icon-btn",
                title: "Sync to USB",
                onclick: move |_| on_sync.call(()),
                span {
                    svg {
                        width: "16",
                        height: "16",
                        view_box: "0 0 24 24",
                        fill: "none",
                        path {
                            d: "M10.4995 13.5002L20.9995 3.00017M10.6271 13.8282L13.2552 20.5862C13.4867 21.1816 13.6025 21.4793 13.7693 21.5662C13.9139 21.6415 14.0862 21.6416 14.2308 21.5664C14.3977 21.4797 14.5139 21.1822 14.7461 20.5871L21.3364 3.69937C21.5461 3.16219 21.6509 2.8936 21.5935 2.72197C21.5437 2.57292 21.4268 2.45596 21.2777 2.40616C21.1061 2.34883 20.8375 2.45364 20.3003 2.66327L3.41258 9.25361C2.8175 9.48584 2.51997 9.60195 2.43326 9.76886C2.35809 9.91354 2.35819 10.0858 2.43353 10.2304C2.52043 10.3972 2.81811 10.513 3.41345 10.7445L10.1715 13.3726C10.2923 13.4196 10.3527 13.4431 10.4036 13.4794C10.4487 13.5115 10.4881 13.551 10.5203 13.5961C10.5566 13.647 10.5801 13.7074 10.6271 13.8282Z",
                            stroke: "currentColor",
                            stroke_width: "2",
                            stroke_linecap: "round",
                            stroke_linejoin: "round"
                        }
                    }
                }
            }
            p { class: "track-count",
                if scanning() { "Scanning..." } else { "{tracks.read().len()} track(s) in library" }
            }
        }

        if !import_warnings.read().is_empty() {
            div {
                class: "modal-backdrop",
                onclick: move |_| import_warnings.write().clear(),
            }
            div { class: "modal",
                div { class: "modal-header",
                    h2 { "Import warnings" }
                    div { class: "modal-header-actions",
                        button {
                            class: "modal-close",
                            onclick: move |_| import_warnings.write().clear(),
                            "×"
                        }
                    }
                }
                div { class: "modal-body",
                    div { class: "log warnings-log",
                        for w in import_warnings() {
                            p { class: "warning", "{w}" }
                        }
                    }
                }
            }
        }

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
