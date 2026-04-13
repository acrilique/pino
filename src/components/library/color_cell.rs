// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use dioxus::prelude::*;

pub const TRACK_COLORS: &[(u8, &str, &str)] = &[
    (0, "None", "transparent"),
    (1, "Pink", "#ff5cb4"),
    (2, "Red", "#e0334b"),
    (3, "Orange", "#ff8c00"),
    (4, "Yellow", "#e5d84e"),
    (5, "Green", "#10b981"),
    (6, "Aqua", "#22d3ee"),
    (7, "Blue", "#3b82f6"),
    (8, "Purple", "#a855f7"),
];

#[component]
pub fn ColorCell(
    track_id: String,
    value: u8,
    on_change: EventHandler<u8>,
    #[props(default)] hidden: bool,
) -> Element {
    if hidden {
        return rsx! {};
    }

    let mut open = use_signal(|| None::<(f64, f64)>);

    let current = TRACK_COLORS
        .iter()
        .find(|(v, _, _)| *v == value)
        .unwrap_or(&TRACK_COLORS[0]);

    rsx! {
        td { class: "color-cell",
            div {
                class: "color-swatch-btn",
                onclick: move |e: MouseEvent| {
                    if open().is_some() {
                        open.set(None);
                    } else {
                        open.set(Some((e.page_coordinates().x, e.page_coordinates().y)));
                    }
                },
                span {
                    class: "color-swatch",
                    style: "background: {current.2};",
                }
            }
            if let Some((x, y)) = open() {
                div {
                    class: "color-dropdown-overlay",
                    onclick: move |_| open.set(None),
                }
                div {
                    class: "color-dropdown",
                    style: "left: {x}px; top: {y}px;",
                    for &(val, name, hex) in TRACK_COLORS {
                        button {
                            class: if val == value { "color-option active" } else { "color-option" },
                            onclick: move |_| {
                                on_change.call(val);
                                open.set(None);
                            },
                            span {
                                class: "color-swatch",
                                style: "background: {hex};",
                            }
                            "{name}"
                        }
                    }
                }
            }
        }
    }
}
