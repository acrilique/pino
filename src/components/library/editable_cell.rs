// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum EditColumn {
    Title,
    Artist,
    Album,
    Genre,
    Composer,
    Label,
    Remixer,
    Key,
    Comment,
    Isrc,
    Lyricist,
    MixName,
    ReleaseDate,
    Bpm,
    Year,
    TrackNumber,
    DiscNumber,
    AddedAt,
    Tags,
}

impl EditColumn {
    /// Returns `None` for free-text columns, or a validation function for numeric ones.
    fn validate(self, input: &str) -> bool {
        match self {
            EditColumn::Bpm | EditColumn::TrackNumber => {
                input.is_empty() || input.parse::<u32>().is_ok()
            }
            EditColumn::Year | EditColumn::DiscNumber => {
                input.is_empty() || input.parse::<u16>().is_ok()
            }
            _ => true,
        }
    }

    fn is_numeric(self) -> bool {
        matches!(
            self,
            EditColumn::Bpm | EditColumn::Year | EditColumn::TrackNumber | EditColumn::DiscNumber
        )
    }
}

#[component]
pub fn EditableCell(
    track_id: String,
    column: EditColumn,
    value: String,
    mut editing: Signal<Option<(String, EditColumn)>>,
    mut edit_value: Signal<String>,
    on_commit: EventHandler,
    #[props(default)] hidden: bool,
) -> Element {
    if hidden {
        return rsx! {};
    }

    let col_suffix = match column {
        EditColumn::Title => "title",
        EditColumn::Artist => "artist",
        EditColumn::Album => "album",
        EditColumn::Genre => "genre",
        EditColumn::Composer => "composer",
        EditColumn::Label => "label",
        EditColumn::Remixer => "remixer",
        EditColumn::Key => "key",
        EditColumn::Comment => "comment",
        EditColumn::Isrc => "isrc",
        EditColumn::Lyricist => "lyricist",
        EditColumn::MixName => "mix_name",
        EditColumn::ReleaseDate => "release_date",
        EditColumn::Bpm => "bpm",
        EditColumn::Year => "year",
        EditColumn::TrackNumber => "track_number",
        EditColumn::DiscNumber => "disc_number",
        EditColumn::AddedAt => "added_at",
        EditColumn::Tags => "tags",
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
                    inputmode: if column.is_numeric() { "numeric" },
                    id: "{input_id}",
                    class: "cell-edit-input",
                    value: "{edit_value}",
                    autofocus: true,
                    oninput: move |e: FormEvent| {
                        let v = e.value();
                        if column.validate(&v) {
                            edit_value.set(v);
                        }
                    },
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
