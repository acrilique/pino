// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::prefs::{self, SortKey, SortOrder};
use dioxus::prelude::*;

#[component]
pub fn SortableHeader(
    label: &'static str,
    col_key: SortKey,
    mut sort_key: Signal<SortKey>,
    mut sort_order: Signal<SortOrder>,
    resizable: bool,
) -> Element {
    let is_active = sort_key() == col_key;
    rsx! {
        th {
            class: if is_active { "sortable active" } else { "sortable" },
            onclick: move |_| {
                if sort_key() == col_key {
                    sort_order.set(sort_order().toggle());
                } else {
                    sort_key.set(col_key);
                    sort_order.set(SortOrder::Asc);
                }
                prefs::save_sort_prefs(sort_key(), sort_order());
            },
            "{label}"
            if is_active {
                span { class: "sort-indicator", "{sort_order().indicator()}" }
            }
            if resizable {
                div {
                    class: "col-resizer",
                    onclick: |e: MouseEvent| e.stop_propagation(),
                }
            }
        }
    }
}
