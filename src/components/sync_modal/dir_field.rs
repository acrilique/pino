// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

use crate::components::input::Input;
use dioxus::prelude::*;

#[component]
pub fn DirField(label: String, mut value: Signal<String>, placeholder: String) -> Element {
    let mut local = use_signal(&*value);

    use_effect(move || {
        local.set(value());
    });

    rsx! {
        div { class: "field",
            label { "{label}" }
            div { class: "dir-row",
                Input {
                    r#type: "text",
                    value: "{local}",
                    placeholder: "{placeholder}",
                    oninput: move |e: FormEvent| local.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter {
                            value.set(local());
                        }
                    },
                    onblur: move |_| {
                        value.set(local());
                    },
                }
                button {
                    onclick: move |_| {
                        spawn(async move {
                            if let Some(folder) =
                                rfd::AsyncFileDialog::new().pick_folder().await
                            {
                                value.set(
                                    folder.path().to_string_lossy().to_string(),
                                );
                            }
                        });
                    },
                    "Browse"
                }
            }
        }
    }
}
