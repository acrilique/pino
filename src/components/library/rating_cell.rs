// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

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
