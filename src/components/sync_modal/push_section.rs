// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::components::library::refresh_tracks;
use crate::components::log::{LogEntry, log_task_result};
use crate::components::select::{Select, SelectList, SelectOption, SelectTrigger};
use crate::format::SupportedFormat;
use crate::task::{ProgressHandle, run_with_progress};
use crate::{db, paths, sync};
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

        button {
            class: "export-btn",
            disabled: syncing() || (state.pulling)(),
            onclick: move |_| {
                let enabled = *format_enabled.read();
                let supported = enabled_formats(enabled);
                let dest = PathBuf::from(dest_dir());

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

                let config = sync::SyncConfig {
                    supported_formats: supported,
                    convert_to: convert_fmt,
                    jobs: jobs(),
                };

                let db = paths::db_path();
                log_entries.write().clear();
                syncing.set(true);

                let mut progress = ProgressHandle::new(
                    progress_phase, progress_current, progress_total,
                );
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
            },
            if syncing() { "Pushing..." } else { "Push to device" }
        }
    }
}
