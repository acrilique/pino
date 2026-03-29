// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::components::log::{LogEntry, LogPanel};
use crate::prefs::{self, SortKey, SortOrder};
use crate::task::spawn_blocking;
use crate::{db, paths, sync};
use dioxus::prelude::*;

/// Identifies a cell being edited: (track index, column key).
#[derive(Clone, Copy, PartialEq)]
pub enum EditColumn {
    Title,
    Artist,
    Album,
}

#[derive(Clone, PartialEq)]
pub struct ContextMenu {
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

pub fn refresh_tracks(tracks: &mut Signal<Vec<db::TrackWithFiles>>) {
    let mut tracks = *tracks;
    let db = paths::db_path();
    spawn(async move {
        let result = spawn_blocking(move || {
            db::Library::open(&db)
                .and_then(|lib| lib.get_all_tracks_with_files())
                .unwrap_or_default()
        })
        .await;
        if let Ok(t) = result {
            tracks.set(t);
        }
    });
}

fn format_duration(secs: u16) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

// --- Inline JS ---

const JS_TRACK_LIST_INIT: &str = r#"
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

    var resizing = null;
    el.addEventListener('mousedown', function(e) {
        if (!e.target.classList.contains('col-resizer')) return;
        e.preventDefault();
        var th = e.target.closest('th');
        if (!th) return;
        var nextTh = th.nextElementSibling;
        if (!nextTh) return;
        var allThs = Array.from(th.parentElement.children);
        allThs.forEach(function(c) { c.style.width = c.offsetWidth + 'px'; });
        resizing = {
            th: th,
            handle: e.target,
            startX: e.pageX,
            startWidth: th.offsetWidth,
            nextTh: nextTh,
            nextWidth: nextTh ? nextTh.offsetWidth : 0
        };
        e.target.classList.add('active');
        document.body.style.cursor = 'col-resize';
        document.body.style.userSelect = 'none';
    });
    document.addEventListener('mousemove', function(e) {
        if (!resizing) return;
        e.preventDefault();
        var delta = e.pageX - resizing.startX;
        var maxGrow = resizing.nextTh ? resizing.nextWidth - 40 : 0;
        var maxShrink = resizing.startWidth - 40;
        delta = Math.max(-maxShrink, Math.min(delta, maxGrow));
        resizing.th.style.width = (resizing.startWidth + delta) + 'px';
        if (resizing.nextTh) {
            resizing.nextTh.style.width = (resizing.nextWidth - delta) + 'px';
        }
    });
    document.addEventListener('mouseup', function() {
        if (resizing) {
            resizing.handle.classList.remove('active');
            resizing = null;
            document.body.style.cursor = '';
            document.body.style.userSelect = '';
            var table = el.querySelector('table');
            if (table) {
                var ths = Array.from(table.querySelector('thead tr').children);
                var tableW = table.offsetWidth;
                if (tableW > 0) {
                    var pcts = ths.map(function(th) {
                        return (th.offsetWidth / tableW * 100).toFixed(2);
                    });
                    if (window.__pino_save_widths) window.__pino_save_widths(pcts.join(','));
                }
            }
        }
    });
})()
"#;

const JS_COL_WIDTH_LISTENER: &str =
    r#"window.__pino_save_widths = function(csv) { dioxus.send(csv); };"#;

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

            let db = paths::db_path();
            let scroll_id = track_id.clone();
            spawn(async move {
                let _ = spawn_blocking(move || {
                    db::Library::open(&db).and_then(|lib| {
                        lib.update_track(&track_id, &title, &artist, &album, tempo)
                    })
                })
                .await;
            });

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

                        match spawn_blocking(move || sync::import_folder(&db, &input)).await {
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

                                    let mut w = tracks.write();
                                    if let Some(twf) = w.iter_mut().find(|t| t.track.id == track_id) {
                                        twf.files.retain(|f| f.id != file_id);
                                    }
                                    drop(w);

                                    let db = paths::db_path();
                                    spawn(async move {
                                        let _ = spawn_blocking(move || {
                                            db::Library::open(&db)
                                                .and_then(|lib| lib.delete_file(&file_id))
                                        })
                                        .await;
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
                                        let _ = spawn_blocking(move || {
                                            db::Library::open(&db)
                                                .and_then(|lib| lib.delete_track(&track_id))
                                        })
                                        .await;
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
    resizable: bool,
    #[props(default)] initial_width: Option<String>,
) -> Element {
    let is_active = sort_key() == col_key;
    rsx! {
        th {
            class: if is_active { "sortable active" } else { "sortable" },
            width: initial_width,
            onclick: move |_| {
                if sort_key() == col_key {
                    sort_order.set(sort_order().toggle());
                } else {
                    sort_key.set(col_key);
                    sort_order.set(SortOrder::Asc);
                }
                prefs::save_sort_prefs(sort_key(), sort_order());
            },
            "{label}"
            if is_active {
                span { class: "sort-indicator", "{sort_order().indicator()}" }
            }
            if resizable {
                div {
                    class: "col-resizer",
                    onclick: |e: MouseEvent| e.stop_propagation(),
                }
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
