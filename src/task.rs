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
