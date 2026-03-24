// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

mod export;
mod ffmpeg;
mod format;
mod scan;

use dioxus::prelude::*;
use export::ExportConfig;
use format::SupportedFormat;
use std::path::PathBuf;

const FORMATS: [(SupportedFormat, &str); 5] = [
    (SupportedFormat::Mp3, "MP3"),
    (SupportedFormat::Wav, "WAV"),
    (SupportedFormat::Aiff, "AIFF"),
    (SupportedFormat::M4a, "AAC/M4A"),
    (SupportedFormat::Flac, "FLAC"),
];

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let input_dir = use_signal(String::new);
    let output_dir = use_signal(String::new);
    let mut format_enabled = use_signal(|| [true, true, true, true, false]);
    let mut convert_to_idx = use_signal(|| 0usize);
    let mut no_convert = use_signal(|| false);
    let mut log_entries = use_signal(Vec::<LogEntry>::new);
    let mut exporting = use_signal(|| false);

    rsx! {
        document::Stylesheet { href: asset!("/assets/main.css") }

        div { class: "container",
            h1 { "pino" }
            p { class: "subtitle", "Export audio to Pioneer-compatible USB" }

            DirField {
                label: "Input Directory".to_string(),
                value: input_dir,
                placeholder: "/path/to/music".to_string(),
            }
            DirField {
                label: "Output Directory".to_string(),
                value: output_dir,
                placeholder: "/path/to/usb".to_string(),
            }

            div { class: "field",
                label { "Supported Formats" }
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

            div { class: "field",
                label { class: "checkbox-label",
                    input {
                        r#type: "checkbox",
                        checked: no_convert(),
                        oninput: move |_| no_convert.set(!no_convert()),
                    }
                    "Skip unsupported files (no conversion)"
                }
            }

            if !no_convert() {
                div { class: "field",
                    label { "Convert unsupported files to" }
                    select {
                        oninput: move |e| {
                            if let Ok(idx) = e.value().parse::<usize>() {
                                convert_to_idx.set(idx);
                            }
                        },
                        for (i, (_, name)) in FORMATS.iter().enumerate() {
                            option {
                                value: "{i}",
                                selected: i == convert_to_idx(),
                                "{name}",
                            }
                        }
                    }
                }
            }

            button {
                class: "export-btn",
                disabled: exporting() || input_dir().is_empty() || output_dir().is_empty(),
                onclick: move |_| {
                    let enabled = *format_enabled.read();
                    let supported: Vec<SupportedFormat> = FORMATS
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| enabled[*i])
                        .map(|(_, (fmt, _))| *fmt)
                        .collect();
                    let convert_to = FORMATS[convert_to_idx()].0;
                    let nc = no_convert();
                    let input = PathBuf::from(input_dir());
                    let output = PathBuf::from(output_dir());

                    if supported.is_empty() {
                        log_entries
                            .write()
                            .push(LogEntry::error("Select at least one supported format."));
                        return;
                    }
                    if !nc && !supported.contains(&convert_to) {
                        log_entries.write().push(LogEntry::error(
                            "Conversion target format must be included in supported formats.",
                        ));
                        return;
                    }

                    let jobs = std::thread::available_parallelism()
                        .map(|n| n.get())
                        .unwrap_or(1);
                    let config = ExportConfig {
                        supported_formats: supported,
                        convert_to,
                        no_convert: nc,
                        jobs,
                    };

                    log_entries.write().clear();
                    log_entries
                        .write()
                        .push(LogEntry::info("Starting export..."));
                    exporting.set(true);

                    spawn(async move {
                        let (tx, rx) =
                            tokio::sync::oneshot::channel::<Result<(), String>>();
                        std::thread::spawn(move || {
                            let result = export::export(&input, &output, &config)
                                .map_err(|e| e.to_string());
                            let _ = tx.send(result);
                        });
                        match rx.await {
                            Ok(Ok(())) => {
                                log_entries
                                    .write()
                                    .push(LogEntry::success("Export completed successfully!"));
                            }
                            Ok(Err(e)) => {
                                log_entries
                                    .write()
                                    .push(LogEntry::error(&format!("Export failed: {e}")));
                            }
                            Err(_) => {
                                log_entries
                                    .write()
                                    .push(LogEntry::error("Export thread panicked."));
                            }
                        }
                        exporting.set(false);
                    });
                },
                if exporting() { "Exporting..." } else { "Export" }
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
                input {
                    r#type: "text",
                    value: "{value}",
                    placeholder: "{placeholder}",
                    oninput: move |e| value.set(e.value()),
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
