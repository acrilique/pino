// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::components::library::refresh_tracks;
use crate::components::log::{LogEntry, log_task_result};
use crate::components::select::{Select, SelectList, SelectOption, SelectTrigger};
use crate::format::SupportedFormat;
use crate::task::{ProgressHandle, run_with_progress};
use crate::{db, ffmpeg, paths, sync};
use dioxus::prelude::*;
use std::path::PathBuf;

use super::{FORMATS, SyncState, enabled_formats, format_label};

#[component]
pub fn PushSection(
    mut tracks: Signal<Vec<db::TrackWithFiles>>,
    need_conversion: Memo<u32>,
) -> Element {
    let state = use_context::<SyncState>();
    let dest_dir = state.dest_dir;
    let mut format_enabled = state.format_enabled;
    let mut convert_to = state.convert_to;
    let mut auto_convert = state.auto_convert;
    let mut syncing = state.syncing;
    let mut log_entries = state.log_entries;
    let mut sync_status = state.sync_status;
    let mut jobs = state.jobs;
    let progress_phase = state.progress_phase;
    let progress_current = state.progress_current;
    let progress_total = state.progress_total;

    let mut ffmpeg_missing = use_signal(|| false);
    let mut start_sync = move |convert_fmt: Option<SupportedFormat>| {
        let enabled = *format_enabled.read();
        let supported = enabled_formats(enabled);
        let dest = PathBuf::from(dest_dir());

        let config = sync::SyncConfig {
            supported_formats: supported,
            convert_to: convert_fmt,
            jobs: jobs(),
        };

        let db = paths::db_path();
        log_entries.write().clear();
        ffmpeg_missing.set(false);
        syncing.set(true);

        let mut progress = ProgressHandle::new(progress_phase, progress_current, progress_total);
        progress.reset();

        spawn(async move {
            let result = run_with_progress(&mut progress, move |callback| {
                sync::sync(&db, &dest, &config, &callback)
            })
            .await;

            if let Some(result) = log_task_result(
                log_entries,
                result,
                |r: &sync::SyncResult| r.to_string(),
                "Sync",
            ) {
                for w in &result.warnings {
                    log_entries.write().push(LogEntry::warning(w));
                }
                if result.converted > 0 {
                    refresh_tracks(&mut tracks);
                }
                if let Some(mut s) = sync_status() {
                    s.to_push = 0;
                    sync_status.set(Some(s));
                }
            }
            syncing.set(false);
        });
    };

    rsx! {
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

        if need_conversion() > 0 && !ffmpeg_missing() {
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
                        Select::<SupportedFormat> {
                            on_value_change: move |val: Option<SupportedFormat>| {
                                convert_to.set(val);
                            },
                            SelectTrigger {
                                {format_label(convert_to())}
                            }
                            SelectList {
                                for (i, (fmt, name)) in FORMATS.iter().enumerate() {
                                    if format_enabled.read()[i] {
                                        SelectOption::<SupportedFormat> {
                                            value: *fmt,
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

        if ffmpeg_missing() && !syncing() {
            p { class: "error",
                "ffmpeg is required for audio conversion but was not found in PATH."
            }
        }

        button {
            class: "export-btn",
            disabled: syncing() || (state.pulling)(),
            onclick: move |_| {
                if ffmpeg_missing() {
                    ffmpeg_missing.set(false);
                    start_sync(None);
                    return;
                }

                let enabled = *format_enabled.read();
                let supported = enabled_formats(enabled);

                if supported.is_empty() {
                    log_entries
                        .write()
                        .push(LogEntry::error("Select at least one allowed format."));
                    return;
                }

                let convert_fmt = if auto_convert() && need_conversion() > 0 {
                    let Some(fmt) = convert_to() else {
                        log_entries.write().push(LogEntry::error(
                            "Select a conversion target format.",
                        ));
                        return;
                    };
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

                if convert_fmt.is_some() && !ffmpeg::check_available() {
                    ffmpeg_missing.set(true);
                } else {
                    start_sync(convert_fmt);
                }
            },
            if syncing() {
                "Pushing..."
            } else if ffmpeg_missing() {
                {format!("Push without conversion (skip {} files)", need_conversion())}
            } else {
                "Push to device"
            }
        }
    }
}
