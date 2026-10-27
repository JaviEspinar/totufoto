//! Backups of the index (see `crate::backup`): made as a download in a browser or saved
//! where the desktop app's dialog says, and restored from a file sent (or picked) and checked
//! first, so the page can say what it holds before anything is replaced.

use std::path::PathBuf;
use std::sync::atomic::Ordering::SeqCst;

use anyhow::Context;
use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use tokio::io::AsyncWriteExt;

use crate::backup;
use crate::scan;

use super::{ApiError, ApiResult, Shared};

fn data_dir(s: &Shared) -> PathBuf {
    s.scan.db_path.parent().map(PathBuf::from).unwrap_or_default()
}

/// The backup chosen to restore, kept until it is restored or another one is chosen.
fn pending(s: &Shared) -> PathBuf {
    data_dir(s).join(format!("restore-pending.{}", backup::EXTENSION))
}

fn remove_with_journal(path: &std::path::Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(name));
    }
}

#[derive(Deserialize, Default)]
pub(super) struct SaveBody {
    /// The folder on the server to save it in (a browser on the server's gallery). Without
    /// it, the desktop app asks where with its save dialog.
    folder: Option<String>,
}

/// Starts making a backup, in a folder of the server or where the desktop app's save dialog
/// says, and answers at once (`202` with the `path` it goes to; `204` when the dialog was
/// cancelled). `/api/backup/progress` follows it.
pub(super) async fn save(State(s): State<Shared>, body: Option<Json<SaveBody>>) -> ApiResult<Response> {
    if s.status.backup.running.load(SeqCst) {
        return Err(ApiError::conflict("a backup is already being made"));
    }
    let out = match body.and_then(|Json(b)| b.folder).filter(|f| !f.trim().is_empty()) {
        Some(folder) => {
            let folder = PathBuf::from(folder.trim());
            if !folder.is_dir() {
                return Err(ApiError::bad_request(format!("{} is not a folder", folder.display())));
            }
            // Said now, rather than once the backup has failed.
            let probe = folder.join(".imadive-write-check");
            if let Err(e) = std::fs::write(&probe, b"") {
                return Err(ApiError::forbidden(format!("can't write in {}: {e}", folder.display())));
            }
            let _ = std::fs::remove_file(&probe);
            backup::unique_path(&folder, &backup::file_name())
        }
        None => {
            let Some(host) = s.host.clone() else {
                return Err(ApiError::bad_request("choose the folder to save the backup in"));
            };
            match tokio::task::spawn_blocking(move || host.save_file(&backup::file_name())).await? {
                Some(path) => path,
                None => return Ok(StatusCode::NO_CONTENT.into_response()),
            }
        }
    };
    if !s.status.backup.start(s.scan.db_path.clone(), s.scan.fixed_roots.clone(), out.clone()) {
        return Err(ApiError::conflict("a backup is already being made"));
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "path": out.to_string_lossy() }))).into_response())
}

/// How the backup being made (or the last one) is going.
pub(super) async fn progress(State(s): State<Shared>) -> Json<backup::JobView> {
    Json(s.status.backup.view())
}

#[derive(Deserialize, Default)]
pub(super) struct InspectBody {
    /// Choose the file with the desktop app's open dialog.
    #[serde(default)]
    pick: bool,
    /// A backup file on the server (chosen in the page's folder browser).
    path: Option<String>,
}

/// A backup to restore, sent as the body: checked and described, not restored yet.
pub(super) async fn inspect_upload(State(s): State<Shared>, body: Body) -> ApiResult<Json<JsonValue>> {
    let target = pending(&s);
    remove_with_journal(&target);
    let mut file = tokio::fs::File::create(&target).await.context("saving the backup")?;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ApiError::bad_request(format!("the upload was interrupted: {e}")))?;
        file.write_all(&chunk).await.context("saving the backup")?;
    }
    file.flush().await?;
    drop(file);
    describe(target).await
}

/// A backup to restore: a file on the server, or one chosen with the desktop app's open
/// dialog (`204` when it was cancelled). It is copied, so the file chosen is never changed.
pub(super) async fn inspect_pick(State(s): State<Shared>, body: Option<Json<InspectBody>>) -> ApiResult<Response> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let chosen = match body.path.filter(|p| !p.trim().is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path.trim());
            if !path.is_file() {
                return Err(ApiError::bad_request(format!("{} is not a file", path.display())));
            }
            path
        }
        None if body.pick => {
            let Some(host) = s.host.clone() else { return Err(ApiError::desktop_only("open a file dialog")) };
            match tokio::task::spawn_blocking(move || host.pick_file()).await? {
                Some(path) => path,
                None => return Ok(StatusCode::NO_CONTENT.into_response()),
            }
        }
        None => return Err(ApiError::bad_request("choose the backup file to restore")),
    };
    let target = pending(&s);
    remove_with_journal(&target);
    let copy = target.clone();
    tokio::task::spawn_blocking(move || std::fs::copy(&chosen, &copy)).await?.context("reading the backup")?;
    Ok(describe(target).await?.into_response())
}

async fn describe(target: PathBuf) -> ApiResult<Json<JsonValue>> {
    let checked = target.clone();
    let refused = match tokio::task::spawn_blocking(move || backup::inspect(&checked)).await? {
        Ok(Ok(summary)) => return Ok(Json(serde_json::to_value(summary)?)),
        Ok(Err(backup::Refused::NotABackup)) => ApiError::bad_request("this file isn't an Imadive backup"),
        Ok(Err(backup::Refused::Empty)) => {
            ApiError::bad_request("this file is empty: the backup wasn't saved completely")
        }
        Ok(Err(backup::Refused::NotADatabase)) => ApiError::bad_request(
            "this file isn't an Imadive backup (it isn't even a database: maybe an incomplete download)",
        ),
        Ok(Err(backup::Refused::AnIndex)) => {
            ApiError::bad_request("this file is an Imadive index, not a backup: make a backup with Back up… instead")
        }
        Ok(Err(backup::Refused::Unreadable(why))) => {
            tracing::warn!("reading a backup: {why}");
            ApiError::bad_request(format!("this backup can't be read: {why}"))
        }
        Ok(Err(backup::Refused::Newer(version))) => {
            ApiError::bad_request(format!("this backup was made by a newer Imadive ({version}): update this one first"))
        }
        Err(e) => e.into(),
    };
    remove_with_journal(&target);
    Err(refused)
}

/// Replaces the index with the backup checked last. Scans wait meanwhile, and one starts
/// afterwards: for files changed since the backup, and the photos' thumbnails.
pub(super) async fn restore(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let source = pending(&s);
    if !source.is_file() {
        return Err(ApiError::conflict("choose the backup to restore first"));
    }
    if s.status.backup.running.load(SeqCst) {
        return Err(ApiError::conflict("a backup is being made; wait for it to finish"));
    }
    if s.status.removing.lock().unwrap().is_some() || s.status.dups.deleting.load(SeqCst) {
        return Err(ApiError::conflict("the gallery is busy removing files; wait for it to finish"));
    }
    if s.status.hold.swap(true, SeqCst) {
        return Err(ApiError::conflict("a folder is being removed or moved; wait for it to finish"));
    }
    s.status.restoring.store(true, SeqCst);
    let _guard = RestoreGuard { s: s.clone() };
    while s.status.running.load(SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let (db_path, fixed, from) = (s.scan.db_path.clone(), s.scan.fixed_roots.clone(), source.clone());
    tokio::task::spawn_blocking(move || backup::restore(&db_path, &fixed, &from)).await??;
    remove_with_journal(&source);
    tracing::info!("restored a backup; the index it replaced is in {}", backup::BEFORE_RESTORE);
    Ok(Json(json!({ "restored": true })))
}

/// Ends a restore however it goes: scans may run again, and one starts.
struct RestoreGuard {
    s: Shared,
}

impl Drop for RestoreGuard {
    fn drop(&mut self) {
        self.s.status.restoring.store(false, SeqCst);
        self.s.status.hold.store(false, SeqCst);
        scan::spawn(self.s.scan.clone(), self.s.status.clone());
    }
}
