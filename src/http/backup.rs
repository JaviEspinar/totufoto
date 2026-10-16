//! Backups of the index (see `crate::backup`): made as a download in a browser or saved
//! where the desktop app's dialog says, and restored from a file sent (or picked) and checked
//! first, so the page can say what it holds before anything is replaced.

use std::path::PathBuf;
use std::sync::atomic::Ordering::SeqCst;

use anyhow::Context;
use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
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

/// A backup to download (in a browser). It is made in the data folder first, and the last one
/// stays there until the next.
pub(super) async fn download(State(s): State<Shared>, request: axum::extract::Request) -> ApiResult<Response> {
    use tower::ServiceExt;
    let out = data_dir(&s).join(format!("backup-download.{}", backup::EXTENSION));
    let (db_path, fixed) = (s.scan.db_path.clone(), s.scan.fixed_roots.clone());
    let made = out.clone();
    tokio::task::spawn_blocking(move || backup::create(&db_path, &fixed, &made)).await??;
    let service = tower_http::services::ServeFile::new_with_mime(&out, &"application/octet-stream".parse()?);
    let mut response = service.oneshot(request).await?.map(Body::new);
    response.headers_mut().insert(header::CONTENT_DISPOSITION, super::items::attachment(&backup::file_name()).parse()?);
    response.headers_mut().insert(header::CACHE_CONTROL, "no-store".parse()?);
    Ok(response)
}

/// A backup saved where the desktop app's save dialog says; `204` when it was cancelled.
pub(super) async fn save(State(s): State<Shared>) -> ApiResult<Response> {
    let Some(host) = s.host.clone() else { return Err(ApiError::desktop_only("open a save dialog")) };
    let Some(out) = tokio::task::spawn_blocking(move || host.save_file(&backup::file_name())).await? else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let (db_path, fixed) = (s.scan.db_path.clone(), s.scan.fixed_roots.clone());
    let made = out.clone();
    tokio::task::spawn_blocking(move || backup::create(&db_path, &fixed, &made)).await??;
    tracing::info!("backup saved in {}", out.display());
    Ok(Json(json!({ "path": out.to_string_lossy() })).into_response())
}

#[derive(Deserialize, Default)]
pub(super) struct InspectBody {
    /// Choose the file with the desktop app's open dialog.
    #[serde(default)]
    pick: bool,
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

/// A backup to restore, chosen with the desktop app's open dialog (copied, so the file chosen
/// is never changed); `204` when it was cancelled.
pub(super) async fn inspect_pick(State(s): State<Shared>, body: Option<Json<InspectBody>>) -> ApiResult<Response> {
    if !body.map(|Json(b)| b).unwrap_or_default().pick {
        return Err(ApiError::bad_request("send the backup as the body, or pick it in the desktop app"));
    }
    let Some(host) = s.host.clone() else { return Err(ApiError::desktop_only("open a file dialog")) };
    let Some(chosen) = tokio::task::spawn_blocking(move || host.pick_file()).await? else {
        return Ok(StatusCode::NO_CONTENT.into_response());
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
