// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU Affero General Public License as published by the Free
// Software Foundation, either version 3 of the License, or (at your option) any
// later version.
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::components::library::refresh_tracks;
use crate::components::log::{LogEntry, log_task_result};
use crate::task::{ProgressHandle, run_with_progress};
use crate::{db, paths, sync};
use dioxus::prelude::*;
use std::path::PathBuf;

use super::SyncState;

#[component]
pub fn PullSection(mut tracks: Signal<Vec<db::TrackWithFiles>>) -> Element {
    let state = use_context::<SyncState>();
    let dest_dir = state.dest_dir;
    let mut pulling = state.pulling;
    let mut log_entries = state.log_entries;
    let mut sync_status = state.sync_status;
    let progress_phase = state.progress_phase;
    let progress_current = state.progress_current;
    let progress_total = state.progress_total;

    rsx! {
        div { class: "pull-section",
            button {
                class: "pull-btn",
                disabled: pulling() || (state.syncing)(),
                onclick: move |_| {
                    let db = paths::db_path();
                    let dest = PathBuf::from(dest_dir());
                    pulling.set(true);
                    log_entries.write().clear();
                    log_entries.write().push(LogEntry::info("Pulling tracks from device..."));

                    let mut progress = ProgressHandle::new(
                        progress_phase, progress_current, progress_total,
                    );
                    progress.reset();

                    spawn(async move {
                        let result = run_with_progress(&mut progress, move |callback| {
                            sync::pull_from_remote(&db, &dest, &callback)
                        })
                        .await;

                        if log_task_result(
                            log_entries,
                            result,
                            |r: &sync::PullResult| format!("Pulled {} track(s) from device.", r.pulled),
                            "Pull",
                        )
                        .is_some_and(|r| {
                            for w in &r.warnings {
                                log_entries.write().push(LogEntry::warning(w));
                            }
                            r.pulled > 0
                        })
                        {
                            refresh_tracks(&mut tracks);
                            if let Some(mut s) = sync_status() {
                                s.to_pull = 0;
                                sync_status.set(Some(s));
                            }
                        }
                        pulling.set(false);
                    });
                },
                if pulling() { "Pulling..." } else { "Pull from device" }
            }
        }
    }
}
