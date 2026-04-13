// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

mod dir_field;
mod pull_section;
mod push_section;

use crate::components::log::{LogEntry, LogPanel};
use crate::format::SupportedFormat;
use crate::library::Library;
use crate::task::spawn_blocking;
use crate::{prefs, sync};
use dioxus::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

use dir_field::DirField;
use pull_section::PullSection;
use push_section::PushSection;

pub const FORMATS: [(SupportedFormat, &str); 5] = [
    (SupportedFormat::Mp3, "MP3"),
    (SupportedFormat::Wav, "WAV"),
    (SupportedFormat::Aiff, "AIFF"),
    (SupportedFormat::M4a, "AAC/M4A"),
    (SupportedFormat::Flac, "FLAC"),
];

pub fn enabled_formats(enabled: [bool; 5]) -> Vec<SupportedFormat> {
    FORMATS
        .iter()
        .enumerate()
        .filter(|(i, _)| enabled[*i])
        .map(|(_, (fmt, _))| *fmt)
        .collect()
}

pub fn format_label(fmt: Option<SupportedFormat>) -> &'static str {
    match fmt {
        Some(f) => FORMATS
            .iter()
            .find(|(sf, _)| *sf == f)
            .map_or("Select a format", |(_, name)| name),
        None => "Select a format",
    }
}

// ── Sync state (lives in App, accessed via context) ──────────────────────────

#[derive(Clone, Copy)]
pub struct SyncState {
    pub dest_dir: Signal<String>,
    pub format_enabled: Signal<[bool; 5]>,
    pub convert_to: Signal<Option<SupportedFormat>>,
    pub auto_convert: Signal<bool>,
    pub syncing: Signal<bool>,
    pub pulling: Signal<bool>,
    pub log_entries: Signal<Vec<LogEntry>>,
    pub progress_phase: Signal<String>,
    pub progress_current: Signal<u32>,
    pub progress_total: Signal<u32>,
    pub jobs: Signal<usize>,
    pub sync_status: Signal<Option<sync::SyncStatus>>,
    pub checking: Signal<bool>,
    pub dest_error: Signal<Option<String>>,
}

impl SyncState {
    pub fn new() -> Self {
        Self {
            dest_dir: Signal::new(prefs::load_dest_dir()),
            format_enabled: Signal::new([true, true, true, true, false]),
            convert_to: Signal::new(None),
            auto_convert: Signal::new(true),
            syncing: Signal::new(false),
            pulling: Signal::new(false),
            log_entries: Signal::new(Vec::new()),
            progress_phase: Signal::new(String::new()),
            progress_current: Signal::new(0),
            progress_total: Signal::new(0),
            jobs: Signal::new(
                std::thread::available_parallelism()
                    .map(std::num::NonZero::get)
                    .unwrap_or(4),
            ),
            sync_status: Signal::new(None),
            checking: Signal::new(false),
            dest_error: Signal::new(None),
        }
    }
}

// ── Device check (shared between App use_effect and SyncModal refresh) ───────

/// Trigger a sync-status check against the destination.
pub fn check_device(state: &SyncState) {
    let dir = (state.dest_dir)();
    let mut sync_status = state.sync_status;
    let mut dest_error = state.dest_error;
    let mut checking = state.checking;

    if dir.is_empty() {
        sync_status.set(None);
        dest_error.set(None);
        return;
    }
    let dest = PathBuf::from(&dir);
    if !dest.is_dir() {
        sync_status.set(None);
        dest_error.set(Some(format!("Cannot access \"{dir}\".")));
        checking.set(false);
        return;
    }
    dest_error.set(None);
    let lib = consume_context::<Arc<Library>>();
    checking.set(true);
    spawn(async move {
        match spawn_blocking(move || sync::check_sync_status(&lib, &dest)).await {
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
}

// ── Component ────────────────────────────────────────────────────────────────

#[component]
pub fn SyncModal(
    mut tracks: Signal<Vec<crate::bridge::TrackView>>,
    on_close: EventHandler,
) -> Element {
    let state = use_context::<SyncState>();
    let dest_dir = state.dest_dir;
    let format_enabled = state.format_enabled;
    let syncing = state.syncing;
    let pulling = state.pulling;
    let log_entries = state.log_entries;
    let sync_status = state.sync_status;
    let checking = state.checking;
    let dest_error = state.dest_error;
    let progress_phase = state.progress_phase;
    let progress_current = state.progress_current;
    let progress_total = state.progress_total;

    let need_conversion = use_memo(move || {
        let enabled = *format_enabled.read();
        let supported = enabled_formats(enabled);

        if supported.is_empty() {
            return 0u32;
        }

        u32::try_from(
            tracks
                .read()
                .iter()
                .filter(|twf| {
                    !twf.files.iter().any(|f| {
                        SupportedFormat::try_from(f.format.as_str())
                            .is_ok_and(|fmt| supported.contains(&fmt))
                    })
                })
                .count(),
        )
        .unwrap_or(u32::MAX)
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
                        onclick: move |_| {
                            check_device(&state);
                        },
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
            if let Some(err) = dest_error() {
                div { class: "sync-status error-status",
                    span { class: "status-icon", "✕" }
                    p { "{err}" }
                }
            } else if checking() {
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
            PullSection { tracks }
        }

        if has_dest && to_push > 0 && !checking() {
            PushSection { tracks, need_conversion }
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

        LogPanel { entries: log_entries }

        } // modal-body
        } // modal
    }
}
