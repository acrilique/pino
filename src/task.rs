// Copyright (c) 2026 Lluc Simó Margalef <lluc.simo@protonmail.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy
// of the MPL was not distributed with this file, You can obtain one at
// http://mozilla.org/MPL/2.0/.
//
// SPDX-License-Identifier: MPL-2.0

/// Run a blocking closure on a dedicated thread and return its result asynchronously.
pub async fn spawn_blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, tokio::sync::oneshot::error::RecvError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.await
}

/// Open the local database and run a single operation on it in a background thread.
pub async fn db_op<T: Send + 'static>(
    f: impl FnOnce(&crate::db::Library) -> rusqlite::Result<T> + Send + 'static,
) -> Result<T, String> {
    let db_path = crate::paths::db_path();
    match spawn_blocking(move || crate::db::Library::open(&db_path).and_then(|lib| f(&lib))).await {
        Ok(Ok(val)) => Ok(val),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Background task panicked".to_string()),
    }
}

use dioxus::prelude::WritableExt;

/// Progress state that can be updated from a background thread via a channel.
pub struct ProgressHandle {
    pub phase: dioxus::prelude::Signal<String>,
    pub current: dioxus::prelude::Signal<u32>,
    pub total: dioxus::prelude::Signal<u32>,
}

impl ProgressHandle {
    pub fn new(
        phase: dioxus::prelude::Signal<String>,
        current: dioxus::prelude::Signal<u32>,
        total: dioxus::prelude::Signal<u32>,
    ) -> Self {
        Self {
            phase,
            current,
            total,
        }
    }

    pub fn reset(&mut self) {
        self.phase.set(String::new());
        self.current.set(0);
        self.total.set(0);
    }
}

/// Run an operation on a background thread while streaming progress updates to UI signals.
pub async fn run_with_progress<T: Send + 'static>(
    progress: &mut ProgressHandle,
    op: impl FnOnce(Box<dyn Fn(crate::sync::SyncProgress) + Send + Sync>) -> T + Send + 'static,
) -> Result<T, tokio::sync::oneshot::error::RecvError> {
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let (ptx, mut prx) = tokio::sync::mpsc::channel::<crate::sync::SyncProgress>(64);

    std::thread::spawn(move || {
        let callback = Box::new(move |p: crate::sync::SyncProgress| {
            let _ = ptx.blocking_send(p);
        });
        let result = op(callback);
        let _ = result_tx.send(result);
    });

    let mut phase = progress.phase;
    let mut current = progress.current;
    let mut total = progress.total;
    while let Some(p) = prx.recv().await {
        phase.set(p.phase.to_string());
        current.set(p.current);
        total.set(p.total);
    }

    result_rx.await
}
