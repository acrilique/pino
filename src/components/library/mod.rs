// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

mod color_cell;
mod editable_cell;
mod rating_cell;
mod sortable_header;
mod tag_cell;

use std::collections::HashSet;
use std::sync::Arc;

use dioxus_core::Task;

use crate::bridge::{self, TrackField};
use crate::library::Library as Lib;
use crate::prefs::{self, Column, DEFAULT_PAGE_SIZE, SortKey, SortOrder};
use crate::sync;
use crate::task::{self, spawn_blocking};
use dioxus::prelude::*;

use color_cell::ColorCell;
pub use editable_cell::EditColumn;
use editable_cell::EditableCell;
use rating_cell::RatingCell;
use sortable_header::SortableHeader;
use tag_cell::TagCell;

pub fn refresh_tracks(tracks: &mut Signal<Vec<bridge::TrackView>>) {
    refresh_tracks_with_query(tracks, "");
}

pub fn refresh_tracks_with_query(tracks: &mut Signal<Vec<bridge::TrackView>>, query: &str) {
    let lib = dioxus::prelude::consume_context::<Arc<Lib>>();
    let mut tracks = *tracks;
    let query = query.to_owned();
    spawn(async move {
        let t = spawn_blocking(move || {
            if query.is_empty() {
                lib.all_tracks().unwrap_or_default()
            } else {
                lib.search_tracks(&query).unwrap_or_default()
            }
        })
        .await
        .unwrap_or_default();
        tracks.set(t);
    });
}

fn format_duration(secs: u16) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

fn file_basename(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
}

const JS_TRACK_LIST_INIT: &str = include_str!("../../../assets/track-list.js");

#[derive(Clone, PartialEq)]
struct ContextMenu {
    x: f64,
    y: f64,
    target: ContextTarget,
}

#[derive(Clone, PartialEq)]
enum ContextTarget {
    Header,
    File {
        file_path: String,
        track_id: String,
        format: String,
    },
    Track {
        track_ids: Vec<String>,
    },
}

#[component]
pub fn Library(
    mut tracks: Signal<Vec<bridge::TrackView>>,
    mut scanning: Signal<bool>,
    sort_key: Signal<SortKey>,
    sort_order: Signal<SortOrder>,
    on_sync: EventHandler,
) -> Element {
    let mut editing = use_signal(|| None::<(String, EditColumn)>);
    let edit_value = use_signal(String::new);
    let mut context_menu = use_signal(|| None::<ContextMenu>);
    let mut debounce_task: Signal<Option<Task>> = use_signal(|| None);
    let mut import_warnings: Signal<Vec<String>> = use_signal(Vec::new);
    let mut hidden_cols = use_signal(prefs::load_hidden_columns);
    let mut selected_tracks: Signal<HashSet<String>> = use_signal(HashSet::new);
    let mut last_clicked: Signal<Option<String>> = use_signal(|| None);

    let mut visible_count = use_signal(prefs::load_page_size);

    // Reset visible count when sort or track list changes.
    use_effect(move || {
        let _ = sort_key();
        let _ = sort_order();
        let _ = tracks.read().len();
        visible_count.set(prefs::load_page_size());
    });

    let progress_phase = use_signal(String::new);
    let progress_current = use_signal(|| 0u32);
    let progress_total = use_signal(|| 0u32);

    let sorted_tracks = use_memo(move || {
        let mut list = tracks();
        let key = sort_key();
        let order = sort_order();
        list.sort_by(|a, b| {
            let cmp = match key {
                SortKey::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
                SortKey::Artist => a.artist.to_lowercase().cmp(&b.artist.to_lowercase()),
                SortKey::Album => a.album.to_lowercase().cmp(&b.album.to_lowercase()),
                SortKey::Duration => a.duration_secs.cmp(&b.duration_secs),
                SortKey::Genre => a.genre.to_lowercase().cmp(&b.genre.to_lowercase()),
                SortKey::Composer => a.composer.to_lowercase().cmp(&b.composer.to_lowercase()),
                SortKey::Label => a.label.to_lowercase().cmp(&b.label.to_lowercase()),
                SortKey::Remixer => a.remixer.to_lowercase().cmp(&b.remixer.to_lowercase()),
                SortKey::Key => a.key.to_lowercase().cmp(&b.key.to_lowercase()),
                SortKey::Comment => a.comment.to_lowercase().cmp(&b.comment.to_lowercase()),
                SortKey::Isrc => a.isrc.cmp(&b.isrc),
                SortKey::Lyricist => a.lyricist.to_lowercase().cmp(&b.lyricist.to_lowercase()),
                SortKey::MixName => a.mix_name.to_lowercase().cmp(&b.mix_name.to_lowercase()),
                SortKey::ReleaseDate => a.release_date.cmp(&b.release_date),
                SortKey::Bpm => a.tempo.cmp(&b.tempo),
                SortKey::Year => a.year.cmp(&b.year),
                SortKey::TrackNumber => a.track_number.cmp(&b.track_number),
                SortKey::DiscNumber => a.disc_number.cmp(&b.disc_number),
                SortKey::Rating => a.rating.cmp(&b.rating),
                SortKey::Color => a.color.cmp(&b.color),
                SortKey::AddedAt => a.added_at.cmp(&b.added_at),
                SortKey::FileName => {
                    let a_name = a
                        .files
                        .first()
                        .map(|f| file_basename(&f.file_path))
                        .unwrap_or_default();
                    let b_name = b
                        .files
                        .first()
                        .map(|f| file_basename(&f.file_path))
                        .unwrap_or_default();
                    a_name.to_lowercase().cmp(&b_name.to_lowercase())
                }
                SortKey::Tags => {
                    let a_tags = a.tags.join(", ").to_lowercase();
                    let b_tags = b.tags.join(", ").to_lowercase();
                    a_tags.cmp(&b_tags)
                }
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
        if let Some(twf) = w.iter_mut().find(|t| t.id == track_id) {
            match col {
                EditColumn::Title => twf.title.clone_from(&new_val),
                EditColumn::Artist => twf.artist.clone_from(&new_val),
                EditColumn::Album => twf.album.clone_from(&new_val),
                EditColumn::Genre => twf.genre.clone_from(&new_val),
                EditColumn::Composer => twf.composer.clone_from(&new_val),
                EditColumn::Label => twf.label.clone_from(&new_val),
                EditColumn::Remixer => twf.remixer.clone_from(&new_val),
                EditColumn::Key => twf.key.clone_from(&new_val),
                EditColumn::Comment => twf.comment.clone_from(&new_val),
                EditColumn::Isrc => twf.isrc.clone_from(&new_val),
                EditColumn::Lyricist => twf.lyricist.clone_from(&new_val),
                EditColumn::MixName => twf.mix_name.clone_from(&new_val),
                EditColumn::ReleaseDate => twf.release_date.clone_from(&new_val),
                EditColumn::Bpm => {
                    twf.tempo = new_val.parse().unwrap_or(twf.tempo);
                }
                EditColumn::Year => {
                    twf.year = new_val.parse().unwrap_or(twf.year);
                }
                EditColumn::TrackNumber => {
                    twf.track_number = new_val.parse().unwrap_or(twf.track_number);
                }
                EditColumn::DiscNumber => {
                    twf.disc_number = new_val.parse().unwrap_or(twf.disc_number);
                }
                EditColumn::AddedAt => twf.added_at.clone_from(&new_val),
                EditColumn::Tags => return, // tags are committed by TagCell directly
            }
            let track_uid = twf.id.clone();
            drop(w);

            let field = match col {
                EditColumn::Title => TrackField::Title(new_val.clone()),
                EditColumn::Artist => TrackField::Artist(new_val.clone()),
                EditColumn::Album => TrackField::Album(new_val.clone()),
                EditColumn::Genre => TrackField::Genre(new_val.clone()),
                EditColumn::Composer => TrackField::Composer(new_val.clone()),
                EditColumn::Label => TrackField::Label(new_val.clone()),
                EditColumn::Remixer => TrackField::Remixer(new_val.clone()),
                EditColumn::Key => TrackField::Key(new_val.clone()),
                EditColumn::Comment => TrackField::Comment(new_val.clone()),
                EditColumn::Isrc => TrackField::Isrc(new_val.clone()),
                EditColumn::Lyricist => TrackField::Lyricist(new_val.clone()),
                EditColumn::MixName => TrackField::MixName(new_val.clone()),
                EditColumn::ReleaseDate => TrackField::ReleaseDate(new_val.clone()),
                EditColumn::Bpm => TrackField::Tempo(new_val.parse().unwrap_or(0)),
                EditColumn::Year => TrackField::Year(new_val.parse().unwrap_or(0)),
                EditColumn::TrackNumber => TrackField::TrackNumber(new_val.parse().unwrap_or(0)),
                EditColumn::DiscNumber => TrackField::DiscNumber(new_val.parse().unwrap_or(0)),
                EditColumn::AddedAt | EditColumn::Tags => return, // added_at not editable via aoide, tags are committed by TagCell directly
            };

            let scroll_id = track_uid.clone();
            let lib = consume_context::<Arc<Lib>>();
            spawn(async move {
                let _ = spawn_blocking(move || lib.update_track(&track_uid, &field)).await;
            });

            let js = format!(
                "setTimeout(() => document.getElementById('track-{scroll_id}')?.scrollIntoView({{block:'nearest',behavior:'smooth'}}), 50)"
            );
            document::eval(&js);
        }
        editing.set(None);
    };

    let mut update_rating = move |track_id: String, new_val: u8| {
        let mut w = tracks.write();
        if let Some(twf) = w.iter_mut().find(|t| t.id == track_id) {
            twf.rating = new_val;
            drop(w);
            let lib = consume_context::<Arc<Lib>>();
            spawn(async move {
                let _ = spawn_blocking(move || {
                    lib.update_track(&track_id, &TrackField::Rating(new_val))
                })
                .await;
            });
        }
    };

    let mut update_color = move |track_id: String, new_val: u8| {
        let mut w = tracks.write();
        if let Some(twf) = w.iter_mut().find(|t| t.id == track_id) {
            twf.color = new_val;
            drop(w);
            let lib = consume_context::<Arc<Lib>>();
            spawn(async move {
                let _ = spawn_blocking(move || {
                    lib.update_track(&track_id, &TrackField::Color(new_val))
                })
                .await;
            });
        }
    };

    let mut update_tags = move |track_id: String, new_val: Vec<String>| {
        let mut w = tracks.write();
        if let Some(twf) = w.iter_mut().find(|t| t.id == track_id) {
            twf.tags.clone_from(&new_val);
            drop(w);
            let lib = consume_context::<Arc<Lib>>();
            spawn(async move {
                let _ =
                    spawn_blocking(move || lib.update_track(&track_id, &TrackField::Tags(new_val)))
                        .await;
            });
        }
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
                        let lib = consume_context::<Arc<Lib>>();
                        scanning.set(true);
                        let mut progress = task::ProgressHandle::new(
                            progress_phase,
                            progress_current,
                            progress_total,
                        );

                        let result = task::run_with_progress(&mut progress, move |on_progress| {
                            sync::import_folder(&lib, &input, &*on_progress)
                        })
                        .await;

                        if let Ok(Ok(r)) = result {
                            if !r.warnings.is_empty() {
                                import_warnings.set(r.warnings);
                            }
                            if r.imported > 0 {
                                document::eval("document.getElementById('search-input').value = ''");
                                refresh_tracks(&mut tracks);
                            }
                        }
                        progress.reset();
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
                        let lib = consume_context::<Arc<Lib>>();
                        scanning.set(true);
                        let mut progress = task::ProgressHandle::new(
                            progress_phase,
                            progress_current,
                            progress_total,
                        );

                        let result = task::run_with_progress(&mut progress, move |on_progress| {
                            sync::import_files(&lib, paths, &*on_progress)
                        })
                        .await;

                        if let Ok(r) = result {
                            if !r.warnings.is_empty() {
                                import_warnings.set(r.warnings);
                            }
                            if r.imported > 0 {
                                document::eval("document.getElementById('search-input').value = ''");
                                refresh_tracks(&mut tracks);
                            }
                        }
                        progress.reset();
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
                if scanning() {
                    {
                        let phase = progress_phase();
                        let current = progress_current();
                        let total = progress_total();
                        if total > 0 {
                            format!("{phase}… {current}/{total}")
                        } else if phase.is_empty() {
                            "Scanning…".to_string()
                        } else {
                            format!("{phase}…")
                        }
                    }
                } else { "{tracks.read().len()} track(s) in library" }
            }
        }

        // Search input
        div { class: "search-bar",
            input {
                id: "search-input",
                class: "input search-input",
                r#type: "text",
                placeholder: "Search tracks…",
                oninput: move |e: FormEvent| {
                    let q = e.value();
                    visible_count.set(prefs::load_page_size());
                    if let Some(task) = debounce_task.take() {
                        Task::cancel(task);
                    }
                    debounce_task.set(Some(spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                        refresh_tracks_with_query(&mut tracks, &q);
                    })));
                },
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
                },
                onscroll: move |_| {
                    // Load more rows when scrolled near bottom.
                    let total = sorted_tracks.read().len();
                    let current = visible_count();
                    if current < total {
                        let fut = document::eval(
                            r"let el = document.getElementById('track-list');
                               el ? (el.scrollTop + el.clientHeight >= el.scrollHeight - 200) : false",
                        );
                        spawn(async move {
                            if let Ok(near_bottom) = fut.await
                                && near_bottom == true {
                                    let n = visible_count();
                                    let t = sorted_tracks.read().len();
                                    if n < t {
                                        visible_count.set((n + DEFAULT_PAGE_SIZE).min(t));
                                    }
                                }
                        });
                    }
                },
                table {
                    thead {
                        tr {
                            oncontextmenu: move |e: MouseEvent| {
                                e.prevent_default();
                                context_menu.set(Some(ContextMenu {
                                    x: e.page_coordinates().x,
                                    y: e.page_coordinates().y,
                                    target: ContextTarget::Header,
                                }));
                            },
                            if !hidden_cols.read().contains(&Column::Title) {
                                SortableHeader { label: "Title", col_key: SortKey::Title, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Artist) {
                                SortableHeader { label: "Artist", col_key: SortKey::Artist, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Album) {
                                SortableHeader { label: "Album", col_key: SortKey::Album, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Formats) {
                                th {
                                    "Formats"
                                    div {
                                        class: "col-resizer",
                                        onclick: |e: MouseEvent| e.stop_propagation(),
                                    }
                                }
                            }
                            if !hidden_cols.read().contains(&Column::Duration) {
                                SortableHeader { label: "Duration", col_key: SortKey::Duration, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Genre) {
                                SortableHeader { label: "Genre", col_key: SortKey::Genre, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Composer) {
                                SortableHeader { label: "Composer", col_key: SortKey::Composer, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Label) {
                                SortableHeader { label: "Label", col_key: SortKey::Label, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Remixer) {
                                SortableHeader { label: "Remixer", col_key: SortKey::Remixer, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Key) {
                                SortableHeader { label: "Key", col_key: SortKey::Key, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Comment) {
                                SortableHeader { label: "Comment", col_key: SortKey::Comment, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Isrc) {
                                SortableHeader { label: "ISRC", col_key: SortKey::Isrc, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Lyricist) {
                                SortableHeader { label: "Lyricist", col_key: SortKey::Lyricist, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::MixName) {
                                SortableHeader { label: "Mix Name", col_key: SortKey::MixName, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::ReleaseDate) {
                                SortableHeader { label: "Release Date", col_key: SortKey::ReleaseDate, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Bpm) {
                                SortableHeader { label: "BPM", col_key: SortKey::Bpm, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Year) {
                                SortableHeader { label: "Year", col_key: SortKey::Year, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::TrackNumber) {
                                SortableHeader { label: "Track #", col_key: SortKey::TrackNumber, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::DiscNumber) {
                                SortableHeader { label: "Disc #", col_key: SortKey::DiscNumber, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Rating) {
                                SortableHeader { label: "Rating", col_key: SortKey::Rating, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Color) {
                                SortableHeader { label: "Color", col_key: SortKey::Color, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::AddedAt) {
                                SortableHeader { label: "Added At", col_key: SortKey::AddedAt, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::FileName) {
                                SortableHeader { label: "File Name", col_key: SortKey::FileName, sort_key, sort_order, resizable: true }
                            }
                            if !hidden_cols.read().contains(&Column::Tags) {
                                SortableHeader { label: "Tags", col_key: SortKey::Tags, sort_key, sort_order, resizable: true }
                            }
                        }
                    }
                    tbody {
                        for (row_idx, twf) in sorted_tracks().into_iter().take(visible_count()).enumerate() {
                            {
                                let track_id = twf.id.clone();
                                let is_selected = selected_tracks.read().contains(&track_id);
                                rsx! {
                                    tr {
                                        id: "track-{track_id}",
                                        class: if is_selected { "selected" },
                                        onclick: {
                                            let track_id = track_id.clone();
                                            move |e: MouseEvent| {
                                                let modifiers = e.modifiers();
                                                if modifiers.contains(Modifiers::SHIFT) {
                                                    e.prevent_default();
                                                }
                                                if modifiers.contains(Modifiers::CONTROL) || modifiers.contains(Modifiers::META) {
                                                    // Ctrl/Cmd+click: toggle individual row
                                                    let mut sel = selected_tracks.write();
                                                    if sel.contains(&track_id) {
                                                        sel.remove(&track_id);
                                                    } else {
                                                        sel.insert(track_id.clone());
                                                    }
                                                    last_clicked.set(Some(track_id.clone()));
                                                } else if modifiers.contains(Modifiers::SHIFT) {
                                                    // Shift+click: range select
                                                    let sorted = sorted_tracks();
                                                    let anchor = last_clicked().and_then(|id| {
                                                        sorted.iter().position(|t| t.id == id)
                                                    }).unwrap_or(0);
                                                    let (start, end) = if anchor <= row_idx {
                                                        (anchor, row_idx)
                                                    } else {
                                                        (row_idx, anchor)
                                                    };
                                                    let mut sel = selected_tracks.write();
                                                    for t in &sorted[start..=end] {
                                                        sel.insert(t.id.clone());
                                                    }
                                                } else {
                                                    // Plain click: select only this row
                                                    let mut sel = selected_tracks.write();
                                                    sel.clear();
                                                    sel.insert(track_id.clone());
                                                    last_clicked.set(Some(track_id.clone()));
                                                }
                                            }
                                        },
                                        oncontextmenu: {
                                            let track_id = track_id.clone();
                                            move |e: MouseEvent| {
                                                e.prevent_default();
                                                // If right-clicked track is already selected, operate on whole selection
                                                // Otherwise, select just this track
                                                if !selected_tracks.read().contains(&track_id) {
                                                    let mut sel = selected_tracks.write();
                                                    sel.clear();
                                                    sel.insert(track_id.clone());
                                                    last_clicked.set(Some(track_id.clone()));
                                                }
                                                let ids: Vec<String> = selected_tracks.read().iter().cloned().collect();
                                                context_menu.set(Some(ContextMenu {
                                                    x: e.page_coordinates().x,
                                                    y: e.page_coordinates().y,
                                                    target: ContextTarget::Track {
                                                        track_ids: ids,
                                                    },
                                                }));
                                            }
                                        },
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Title,
                                            value: twf.title.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Title),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Artist,
                                            value: twf.artist.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Artist),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Album,
                                            value: twf.album.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Album),
                                        }
                                        if !hidden_cols.read().contains(&Column::Formats) {
                                            td { class: "formats-cell",
                                                for file in &twf.files {
                                                    {
                                                        let file_path_ctx = file.file_path.clone();
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
                                                                            file_path: file_path_ctx.clone(),
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
                                        }
                                        if !hidden_cols.read().contains(&Column::Duration) {
                                            td { "{format_duration(twf.duration_secs)}" }
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Genre,
                                            value: twf.genre.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Genre),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Composer,
                                            value: twf.composer.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Composer),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Label,
                                            value: twf.label.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Label),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Remixer,
                                            value: twf.remixer.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Remixer),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Key,
                                            value: twf.key.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Key),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Comment,
                                            value: twf.comment.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Comment),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Isrc,
                                            value: twf.isrc.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Isrc),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Lyricist,
                                            value: twf.lyricist.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Lyricist),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::MixName,
                                            value: twf.mix_name.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::MixName),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::ReleaseDate,
                                            value: twf.release_date.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::ReleaseDate),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Bpm,
                                            value: if twf.tempo == 0 { String::new() } else { twf.tempo.to_string() },
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Bpm),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::Year,
                                            value: if twf.year == 0 { String::new() } else { twf.year.to_string() },
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::Year),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::TrackNumber,
                                            value: if twf.track_number == 0 { String::new() } else { twf.track_number.to_string() },
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::TrackNumber),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::DiscNumber,
                                            value: if twf.disc_number == 0 { String::new() } else { twf.disc_number.to_string() },
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::DiscNumber),
                                        }
                                        RatingCell {
                                            track_id: track_id.clone(),
                                            value: twf.rating,
                                            on_change: {
                                                let tid = track_id.clone();
                                                move |v: u8| update_rating(tid.clone(), v)
                                            },
                                            hidden: hidden_cols.read().contains(&Column::Rating),
                                        }
                                        ColorCell {
                                            track_id: track_id.clone(),
                                            value: twf.color,
                                            on_change: {
                                                let tid = track_id.clone();
                                                move |v: u8| update_color(tid.clone(), v)
                                            },
                                            hidden: hidden_cols.read().contains(&Column::Color),
                                        }
                                        EditableCell {
                                            track_id: track_id.clone(),
                                            column: EditColumn::AddedAt,
                                            value: twf.added_at.clone(),
                                            editing,
                                            edit_value,
                                            on_commit: move |()| commit_edit(),
                                            hidden: hidden_cols.read().contains(&Column::AddedAt),
                                        }
                                        if !hidden_cols.read().contains(&Column::FileName) {
                                            td {
                                                "{twf.files.first().map(|f| file_basename(&f.file_path)).unwrap_or_default()}"
                                            }
                                        }
                                        TagCell {
                                            track_id: track_id.clone(),
                                            tags: twf.tags.clone(),
                                            editing,
                                            on_change: {
                                                let tid = track_id.clone();
                                                move |v: Vec<String>| update_tags(tid.clone(), v)
                                            },
                                            hidden: hidden_cols.read().contains(&Column::Tags),
                                        }
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
                        ContextTarget::Header => rsx! {
                            for &col in Column::ALL {
                                {
                                    let visible = !hidden_cols.read().contains(&col);
                                    rsx! {
                                        button {
                                            class: "context-item col-toggle",
                                            onclick: move |_| {
                                                let mut h = hidden_cols.write();
                                                if h.contains(&col) {
                                                    h.remove(&col);
                                                } else {
                                                    h.insert(col);
                                                }
                                                prefs::save_hidden_columns(&h);
                                            },
                                            span { class: "col-check", if visible { "✓" } }
                                            "{col.label()}"
                                        }
                                    }
                                }
                            }
                        },
                        ContextTarget::File { file_path, track_id, format } => rsx! {
                            button {
                                class: "context-item danger",
                                onclick: move |_| {
                                    let file_path = file_path.clone();
                                    let track_id = track_id.clone();
                                    context_menu.set(None);

                                    let mut w = tracks.write();
                                    if let Some(twf) = w.iter_mut().find(|t| t.id == track_id) {
                                        twf.files.retain(|f| f.file_path != file_path);
                                    }
                                    drop(w);

                                    let lib = consume_context::<Arc<Lib>>();
                                    spawn(async move {
                                        let _ = spawn_blocking(move || lib.delete_track_by_path(&file_path)).await;
                                    });
                                },
                                "Remove {format} file"
                            }
                        },
                        ContextTarget::Track { track_ids } => {
                            let remove_label = if track_ids.len() == 1 {
                                "Remove track".to_string()
                            } else {
                                format!("Remove {} tracks", track_ids.len())
                            };
                            let play_label = if track_ids.len() == 1 {
                                "Play track".to_string()
                            } else {
                                format!("Play {} tracks", track_ids.len())
                            };
                            let play_ids = track_ids.clone();
                            rsx! {
                                button {
                                    class: "context-item",
                                    onclick: move |_| {
                                        let ids = play_ids.clone();
                                        context_menu.set(None);
                                        let r = tracks.read();
                                        for id in &ids {
                                            if let Some(twf) = r.iter().find(|t| t.id == *id)
                                                && let Some(file) = twf.files.first() {
                                                    let _ = open::that(&file.file_path);
                                                }
                                        }
                                    },
                                    "{play_label}"
                                }
                                button {
                                    class: "context-item danger",
                                    onclick: move |_| {
                                        let ids = track_ids.clone();
                                        context_menu.set(None);
                                        selected_tracks.write().clear();

                                        let id_set: HashSet<String> = ids.iter().cloned().collect();
                                        tracks.write().retain(|t| !id_set.contains(&t.id));

                                        spawn(async move {
                                            for id in ids {
                                                let lib = consume_context::<Arc<Lib>>();
                                                let _ = spawn_blocking(move || lib.delete_track(&id)).await;
                                            }
                                        });
                                    },
                                    "{remove_label}"
                                }
                            }
                        }
                    }
                }
            }
        }
        } // tab-content
    }
}
