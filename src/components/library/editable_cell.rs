// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum EditColumn {
    Title,
    Artist,
    Album,
}

#[component]
pub fn EditableCell(
    track_id: String,
    column: EditColumn,
    value: String,
    mut editing: Signal<Option<(String, EditColumn)>>,
    mut edit_value: Signal<String>,
    on_commit: EventHandler,
) -> Element {
    let col_suffix = match column {
        EditColumn::Title => "title",
        EditColumn::Artist => "artist",
        EditColumn::Album => "album",
    };
    let input_id = format!("edit-{track_id}-{col_suffix}");

    let is_editing = editing
        .read()
        .as_ref()
        .is_some_and(|(id, col)| id == &track_id && *col == column);

    if is_editing {
        rsx! {
            td { class: "editing-cell",
                input {
                    r#type: "text",
                    id: "{input_id}",
                    class: "cell-edit-input",
                    value: "{edit_value}",
                    autofocus: true,
                    oninput: move |e: FormEvent| edit_value.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter {
                            on_commit.call(());
                        } else if e.key() == Key::Escape {
                            editing.set(None);
                        }
                    },
                    onblur: move |_| {
                        on_commit.call(());
                    },
                }
            }
        }
    } else {
        let tid = track_id.clone();
        let val = value.clone();
        let focus_id = input_id.clone();
        rsx! {
            td {
                class: "editable-cell",
                onclick: move |_| {
                    editing.set(Some((tid.clone(), column)));
                    edit_value.set(val.clone());

                    let js = format!(
                        "setTimeout(() => {{ const el = document.getElementById('{focus_id}'); if (el) {{ el.focus(); el.select?.(); }} }}, 0)"
                    );
                    document::eval(&js);
                },
                "{value}"
            }
        }
    }
}
