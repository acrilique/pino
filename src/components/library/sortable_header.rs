// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

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
