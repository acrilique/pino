// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use dioxus::prelude::*;

/// Chip/pill tag editor cell for the track table.
///
/// Displays tags as removable chips. Click the cell to enter edit mode:
/// type a tag name and press Enter/comma to add, Backspace on empty input
/// to remove last tag, Escape to cancel.
#[component]
pub fn TagCell(
    track_id: String,
    tags: Vec<String>,
    on_change: EventHandler<Vec<String>>,
    #[props(default)] hidden: bool,
) -> Element {
    if hidden {
        return rsx! {};
    }

    let mut is_editing = use_signal(|| false);
    let mut draft = use_signal(String::new);
    let mut local_tags = use_signal(|| tags.clone());
    let input_id = format!("tags-{track_id}");

    // Sync external tags when they change (e.g. after undo or refresh).
    use_effect(use_reactive!(|tags| {
        local_tags.set(tags);
    }));

    let mut commit_tag = move || {
        let val = draft().trim().to_string();
        if !val.is_empty() {
            let mut t = local_tags.write();
            if !t.contains(&val) {
                t.push(val);
            }
            drop(t);
            draft.set(String::new());
            on_change.call(local_tags());
        }
    };

    let mut remove_tag = move |idx: usize| {
        local_tags.write().remove(idx);
        on_change.call(local_tags());
    };

    if is_editing() {
        rsx! {
            td { class: "tag-cell editing",
                onclick: move |e: MouseEvent| e.stop_propagation(),
                div { class: "tag-chips",
                    for (idx, tag) in local_tags().into_iter().enumerate() {
                        span { class: "tag-chip",
                            "{tag}"
                            button {
                                class: "tag-remove",
                                tabindex: -1,
                                onclick: move |e: MouseEvent| {
                                    e.stop_propagation();
                                    remove_tag(idx);
                                },
                                "×"
                            }
                        }
                    }
                    input {
                        r#type: "text",
                        id: "{input_id}",
                        class: "tag-input",
                        placeholder: "Add tag…",
                        value: "{draft}",
                        autofocus: true,
                        oninput: move |e: FormEvent| {
                            let v = e.value();
                            // Comma acts as separator.
                            if v.contains(',') {
                                let parts: Vec<&str> = v.split(',').collect();
                                for part in &parts[..parts.len() - 1] {
                                    let trimmed = part.trim().to_string();
                                    if !trimmed.is_empty() {
                                        let mut t = local_tags.write();
                                        if !t.contains(&trimmed) {
                                            t.push(trimmed);
                                        }
                                    }
                                }
                                draft.set(parts.last().unwrap_or(&"").trim().to_string());
                                on_change.call(local_tags());
                            } else {
                                draft.set(v);
                            }
                        },
                        onkeydown: move |e: KeyboardEvent| {
                            if e.key() == Key::Enter {
                                e.prevent_default();
                                commit_tag();
                            } else if e.key() == Key::Escape {
                                is_editing.set(false);
                                draft.set(String::new());
                            } else if e.key() == Key::Backspace && draft().is_empty() {
                                let mut t = local_tags.write();
                                if !t.is_empty() {
                                    t.pop();
                                    drop(t);
                                    on_change.call(local_tags());
                                }
                            }
                        },
                        onblur: move |_| {
                            commit_tag();
                            is_editing.set(false);
                        },
                    }
                }
            }
        }
    } else {
        let focus_id = input_id.clone();
        rsx! {
            td {
                class: "tag-cell editable-cell",
                onclick: move |_| {
                    is_editing.set(true);
                    let js = format!(
                        "setTimeout(() => {{ const el = document.getElementById('{focus_id}'); if (el) el.focus(); }}, 0)"
                    );
                    document::eval(&js);
                },
                div { class: "tag-chips",
                    for tag in local_tags() {
                        span { class: "tag-chip readonly", "{tag}" }
                    }
                    if local_tags().is_empty() {
                        span { class: "tag-placeholder" }
                    }
                }
            }
        }
    }
}
