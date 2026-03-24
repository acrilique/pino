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

fn format_label(idx: Option<usize>) -> &'static str {
    match idx {
        Some(i) => FORMATS.get(i).map_or("Select a format", |(_, name)| name),
        None => "Select a format",
    }
}

fn db_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("pino")
        .join("library.db")
}

fn main() {
    dioxus::launch(App);
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Library,
    Sync,
}

#[component]
fn App() -> Element {
    let mut tab = use_signal(|| Tab::Library);
    let mut tracks = use_signal(Vec::<db::TrackWithFiles>::new);

    // Load tracks from DB on startup.
    let db = db_path();
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
        document::Stylesheet { href: asset!("/assets/main.css") }
        document::Stylesheet { href: asset!("/assets/dx-components-theme.css") }

        div { class: "container",
            h1 { "pino" }
            p { class: "subtitle", "Pioneer-compatible USB music manager" }

            div { class: "tabs",
                button {
                    class: if tab() == Tab::Library { "tab active" } else { "tab" },
                    onclick: move |_| tab.set(Tab::Library),
                    "Library"
                }
                button {
                    class: if tab() == Tab::Sync { "tab active" } else { "tab" },
                    onclick: move |_| tab.set(Tab::Sync),
                    "Sync"
                }
            }

            match tab() {
                Tab::Library => rsx! { LibraryTab { tracks } },
                Tab::Sync => rsx! { SyncTab { tracks } },
            }
        }
    }
}

fn refresh_tracks(tracks: &mut Signal<Vec<db::TrackWithFiles>>) {
    let mut tracks = *tracks;
    let db = db_path();
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
fn LibraryTab(mut tracks: Signal<Vec<db::TrackWithFiles>>) -> Element {
    let scan_dir = use_signal(String::new);
    let mut scanning = use_signal(|| false);
    let mut convert_to_idx = use_signal(|| None::<usize>);
    let mut converting = use_signal(|| false);
    let mut log_entries = use_signal(Vec::<LogEntry>::new);

    rsx! {
        DirField {
            label: "Folder to import".to_string(),
            value: scan_dir,
            placeholder: "/path/to/music".to_string(),
        }

        button {
            class: "export-btn",
            disabled: scanning() || scan_dir().is_empty(),
            onclick: move |_| {
                let input = PathBuf::from(scan_dir());
                let db = db_path();
                scanning.set(true);
                log_entries.write().clear();
                log_entries.write().push(LogEntry::info("Scanning..."));

                spawn(async move {
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
            if scanning() { "Scanning..." } else { "Import" }
        }

        // Conversion section.
        div { class: "field convert-section",
            label { "Convert all tracks to" }
            div { class: "dir-row",
                Select::<usize> {
                    on_value_change: move |val: Option<usize>| {
                        convert_to_idx.set(val);
                    },
                    SelectTrigger {
                        {format_label(convert_to_idx())}
                    }
                    SelectList {
                        for (i, (_, name)) in FORMATS.iter().enumerate() {
                            SelectOption::<usize> {
                                value: i,
                                index: i,
                                text_value: name.to_string(),
                                "{name}"
                            }
                        }
                    }
                }
                button {
                    disabled: converting() || convert_to_idx().is_none() || tracks.read().is_empty(),
                    onclick: move |_| {
                        let Some(idx) = convert_to_idx() else { return };
                        let target = FORMATS[idx].0;
                        let db = db_path();
                        let track_ids: Vec<String> = tracks
                            .read()
                            .iter()
                            .map(|twf| twf.track.id.clone())
                            .collect();

                        converting.set(true);
                        log_entries.write().clear();
                        log_entries.write().push(LogEntry::info("Converting..."));

                        spawn(async move {
                            let (tx, rx) = tokio::sync::oneshot::channel();
                            std::thread::spawn(move || {
                                let _ = tx.send(sync::convert_tracks(&db, &track_ids, target));
                            });
                            match rx.await {
                                Ok(Ok(n)) => {
                                    log_entries.write().push(LogEntry::success(
                                        &format!("Converted {n} track(s)."),
                                    ));
                                    refresh_tracks(&mut tracks);
                                }
                                Ok(Err(e)) => {
                                    log_entries.write().push(LogEntry::error(
                                        &format!("Conversion failed: {e}"),
                                    ));
                                }
                                Err(_) => {
                                    log_entries
                                        .write()
                                        .push(LogEntry::error("Conversion thread panicked."));
                                }
                            }
                            converting.set(false);
                        });
                    },
                    if converting() { "Converting..." } else { "Convert" }
                }
            }
        }

        if !log_entries.read().is_empty() {
            div { class: "log",
                for entry in log_entries() {
                    p { class: entry.class, "{entry.message}" }
                }
            }
        }

        p { class: "track-count", "{tracks.read().len()} track(s) in library" }

        if !tracks.read().is_empty() {
            div { class: "track-list",
                table {
                    thead {
                        tr {
                            th { "Title" }
                            th { "Artist" }
                            th { "Album" }
                            th { "Formats" }
                            th { "Duration" }
                        }
                    }
                    tbody {
                        for twf in tracks() {
                            tr {
                                td { "{twf.track.title}" }
                                td { "{twf.track.artist}" }
                                td { "{twf.track.album}" }
                                td { class: "formats-cell",
                                    for file in &twf.files {
                                        span { class: "format-badge", "{file.format}" }
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
}

fn format_duration(secs: u16) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

#[component]
fn SyncTab(mut tracks: Signal<Vec<db::TrackWithFiles>>) -> Element {
    let dest_dir = use_signal(String::new);
    let mut format_enabled = use_signal(|| [true, true, true, true, false]);
    let mut convert_to_idx = use_signal(|| None::<usize>);
    let mut auto_convert = use_signal(|| true);
    let mut syncing = use_signal(|| false);
    let mut log_entries = use_signal(Vec::<LogEntry>::new);

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

    rsx! {
        DirField {
            label: "Destination".to_string(),
            value: dest_dir,
            placeholder: "/path/to/usb".to_string(),
        }

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
            }
        }

        button {
            class: "export-btn",
            disabled: syncing() || dest_dir().is_empty(),
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

                let convert_to = if auto_convert() {
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
                };

                let db = db_path();
                log_entries.write().clear();
                log_entries.write().push(LogEntry::info("Syncing..."));
                syncing.set(true);

                spawn(async move {
                    let (tx, rx) = tokio::sync::oneshot::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(sync::sync(&db, &dest, &config));
                    });
                    match rx.await {
                        Ok(Ok(result)) => {
                            log_entries
                                .write()
                                .push(LogEntry::success(&result.to_string()));
                            // Refresh tracks since sync may have added converted files locally.
                            if result.converted > 0 {
                                refresh_tracks(&mut tracks);
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
            if syncing() { "Syncing..." } else { "Sync" }
        }

        if !log_entries.read().is_empty() {
            div { class: "log",
                for entry in log_entries() {
                    p { class: entry.class, "{entry.message}" }
                }
            }
        }
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
