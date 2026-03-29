// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct LogEntry {
    pub message: String,
    pub class: &'static str,
}

impl LogEntry {
    pub fn info(msg: &str) -> Self {
        Self { message: msg.to_string(), class: "info" }
    }
    pub fn success(msg: &str) -> Self {
        Self { message: msg.to_string(), class: "success" }
    }
    pub fn error(msg: &str) -> Self {
        Self { message: msg.to_string(), class: "error" }
    }
}

#[component]
pub fn LogPanel(entries: Signal<Vec<LogEntry>>) -> Element {
    if entries.read().is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "log",
            for entry in entries() {
                p { class: entry.class, "{entry.message}" }
            }
        }
    }
}
