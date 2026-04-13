// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use dioxus::prelude::*;

#[derive(Clone, PartialEq)]
pub struct LogEntry {
    pub message: String,
    pub class: &'static str,
}

impl LogEntry {
    pub fn info(msg: &str) -> Self {
        Self {
            message: msg.to_string(),
            class: "info",
        }
    }
    pub fn success(msg: &str) -> Self {
        Self {
            message: msg.to_string(),
            class: "success",
        }
    }
    pub fn warning(msg: &str) -> Self {
        Self {
            message: msg.to_string(),
            class: "warning",
        }
    }
    pub fn error(msg: &str) -> Self {
        Self {
            message: msg.to_string(),
            class: "error",
        }
    }
}

/// Log the result of a background task and return the success value (if any).
pub fn log_task_result<T>(
    mut log: Signal<Vec<LogEntry>>,
    result: Result<Result<T, impl std::fmt::Display>, impl std::fmt::Display>,
    success_msg: impl FnOnce(&T) -> String,
    fail_context: &str,
) -> Option<T> {
    match result {
        Ok(Ok(val)) => {
            log.write().push(LogEntry::success(&success_msg(&val)));
            Some(val)
        }
        Ok(Err(e)) => {
            log.write()
                .push(LogEntry::error(&format!("{fail_context} failed: {e}")));
            None
        }
        Err(e) => {
            log.write()
                .push(LogEntry::error(&format!("{fail_context} failed: {e}")));
            None
        }
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
