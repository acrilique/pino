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
pub fn RatingCell(
    track_id: String,
    value: u8,
    on_change: EventHandler<u8>,
    #[props(default)] hidden: bool,
) -> Element {
    if hidden {
        return rsx! {};
    }

    rsx! {
        td { class: "rating-cell",
            for star in 1..=5u8 {
                span {
                    class: if star <= value { "star filled" } else { "star" },
                    onclick: {
                        let tid = track_id.clone();
                        move |_| {
                            let _ = tid.as_str(); // keep clone alive
                            if star == value {
                                on_change.call(0);
                            } else {
                                on_change.call(star);
                            }
                        }
                    },
                    "★"
                }
            }
        }
    }
}
