// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

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
