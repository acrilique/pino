// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::components::input::Input;
use crate::components::library::refresh_tracks;
use crate::components::log::{LogEntry, LogPanel};
use crate::components::select::*;
use crate::format::SupportedFormat;
use crate::task::spawn_blocking;
use crate::{db, paths, sync};
use dioxus::prelude::*;
use std::path::PathBuf;

pub const FORMATS: [(SupportedFormat, &str); 5] = [
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

/// Progress signals shared by sync/pull operations.
struct ProgressSignals {
    phase: Signal<String>,
    current: Signal<u32>,
    total: Signal<u32>,
}

impl ProgressSignals {
    fn reset(&mut self) {
        self.phase.set(String::new());
        self.current.set(0);
        self.total.set(0);
    }
}

/// Run a sync operation on a background thread with progress, draining progress updates
/// into the UI signals and returning the final result.
async fn run_with_progress<T: Send + 'static>(
    progress: &mut ProgressSignals,
    op: impl FnOnce(Box<dyn Fn(sync::SyncProgress) + Send + Sync>) -> T + Send + 'static,
) -> Result<T, tokio::sync::oneshot::error::RecvError> {
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let (ptx, mut prx) = tokio::sync::mpsc::channel::<sync::SyncProgress>(64);

    std::thread::spawn(move || {
        let callback = Box::new(move |p: sync::SyncProgress| {
            let _ = ptx.blocking_send(p);
        });
        let result = op(callback);
        let _ = result_tx.send(result);
    });

    let mut phase = progress.phase;
    let mut current = progress.current;
    let mut total = progress.total;
    while let Some(p) = prx.recv().await {
        phase.set(p.phase.to_string());
        current.set(p.current);
        total.set(p.total);
    }

    result_rx.await
}

/// Trigger a sync-status check against the destination.
fn check_device(
    dest_dir: &str,
    mut sync_status: Signal<Option<sync::SyncStatus>>,
    mut dest_error: Signal<Option<String>>,
    mut checking: Signal<bool>,
) {
    if dest_dir.is_empty() {
        sync_status.set(None);
        dest_error.set(None);
        return;
    }
    let dest = PathBuf::from(dest_dir);
    if !dest.is_dir() {
        sync_status.set(None);
        dest_error.set(Some(format!("Cannot access \"{dest_dir}\".")));
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
}

#[component]
pub fn SyncModal(
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
    dest_error: Signal<Option<String>>,
    on_close: EventHandler,
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
                        onclick: move |_| {
                            check_device(&dest_dir(), sync_status, dest_error, checking);
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

                        let mut progress = ProgressSignals {
                            phase: progress_phase,
                            current: progress_current,
                            total: progress_total,
                        };
                        progress.reset();

                        spawn(async move {
                            let result = run_with_progress(&mut progress, move |callback| {
                                sync::pull_from_remote(&db, &dest, &callback)
                            })
                            .await;

                            match result {
                                Ok(Ok(n)) => {
                                    log_entries.write().push(LogEntry::success(
                                        &format!("Pulled {n} track(s) from device."),
                                    ));
                                    refresh_tracks(&mut tracks);
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
                    syncing.set(true);

                    let mut progress = ProgressSignals {
                        phase: progress_phase,
                        current: progress_current,
                        total: progress_total,
                    };
                    progress.reset();

                    spawn(async move {
                        let result = run_with_progress(&mut progress, move |callback| {
                            sync::sync(&db, &dest, &config, &callback)
                        })
                        .await;

                        match result {
                            Ok(Ok(result)) => {
                                log_entries
                                    .write()
                                    .push(LogEntry::success(&result.to_string()));
                                if result.converted > 0 {
                                    refresh_tracks(&mut tracks);
                                }
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

        LogPanel { entries: log_entries }

        } // modal-body
        } // modal
    }
}

#[component]
fn DirField(label: String, mut value: Signal<String>, placeholder: String) -> Element {
    let mut local = use_signal(&*value);

    use_effect(move || {
        local.set(value());
    });

    rsx! {
        div { class: "field",
            label { "{label}" }
            div { class: "dir-row",
                Input {
                    r#type: "text",
                    value: "{local}",
                    placeholder: "{placeholder}",
                    oninput: move |e: FormEvent| local.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter {
                            value.set(local());
                        }
                    },
                    onblur: move |_| {
                        value.set(local());
                    },
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
