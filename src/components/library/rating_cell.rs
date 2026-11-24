// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use dioxus::prelude::*;

#[component]
pub fn RatingCell(value: u8, on_change: EventHandler<u8>) -> Element {
    rsx! {
        td { class: "rating-cell",
            for star in 1..=5u8 {
                span {
                    class: if star <= value { "star filled" } else { "star" },
                    onclick: move |_| {
                        // Clicking the current rating clears it.
                        on_change.call(if star == value { 0 } else { star });
                    },
                    "★"
                }
            }
        }
    }
}
