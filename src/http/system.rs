//! Library status, scans, duplicates, and opening links (desktop app).

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};

use crate::scan::{self};

use super::{ApiError, ApiResult, Shared, db};

pub(super) async fn status(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let view = s.status.view();
    // Files that could not be indexed, saved across scans (see /api/failures).
    let (failed, excluded) = db(&s, |conn| crate::db::problem_counts(conn)).await?;
    let mut value = serde_json::to_value(view)?;
    value["failed"] = failed.into();
    // Photos removed from the gallery (their files kept); see /api/excluded/clear.
    value["excluded"] = excluded.into();
    value["version"] = env!("CARGO_PKG_VERSION").into();
    value["project"] = super::PROJECT_URL.into();
    Ok(Json(value))
}

/// Files that could not be indexed, with the reason (first 500).
pub(super) async fn failures(State(s): State<Shared>) -> ApiResult<Json<Vec<crate::db::Failure>>> {
    Ok(Json(db(&s, |conn| crate::db::failures(conn, 500)).await?))
}

/// Forgets the failures and scans again, so those files are tried once more.
pub(super) async fn retry_failures(State(s): State<Shared>) -> ApiResult<StatusCode> {
    db(&s, |conn| crate::db::forget_failures(conn)).await?;
    scan::spawn(s.scan.clone(), s.status.clone());
    Ok(StatusCode::ACCEPTED)
}

/// Scans and then regroups every face from scratch (named people are kept).
pub(super) async fn regroup(State(s): State<Shared>) -> StatusCode {
    s.status.request_regroup();
    scan::spawn(s.scan.clone(), s.status.clone());
    StatusCode::ACCEPTED
}

pub(super) async fn start_scan(State(s): State<Shared>) -> StatusCode {
    scan::spawn(s.scan.clone(), s.status.clone());
    StatusCode::ACCEPTED
}

#[derive(Deserialize)]
pub(super) struct OpenBody {
    url: String,
}

/// Opens a link in the system browser (desktop app only). Limited to the sites the UI links
/// to (maps, and the project's pages from Settings), so the endpoint can't be used to launch
/// arbitrary URLs.
pub(super) async fn open_url(State(s): State<Shared>, Json(body): Json<OpenBody>) -> ApiResult<StatusCode> {
    match &s.host {
        Some(host)
            if body.url.starts_with("https://www.openstreetmap.org/")
                || body.url.starts_with(&format!("{}/", super::PROJECT_URL)) =>
        {
            host.open_url(&body.url);
            Ok(StatusCode::NO_CONTENT)
        }
        Some(_) => Err(ApiError::forbidden("only map links can be opened")),
        None => Err(ApiError::desktop_only("open links")),
    }
}

/// Identical files and the search progress. While the scan or the search runs, the report
/// may still be incomplete.
pub(super) async fn duplicates_report(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let dups = s.status.dups.clone();
    let scanning = s.status.running.load(std::sync::atomic::Ordering::Relaxed);
    let groups = db(&s, |conn| crate::duplicates::report(conn)).await?;
    let (files, bytes) = crate::duplicates::totals(&groups);
    use std::sync::atomic::Ordering::Relaxed;
    Ok(Json(json!({
        "scanning": scanning,
        "running": dups.running.load(Relaxed),
        "done": dups.done.load(Relaxed),
        "total": dups.total.load(Relaxed),
        "finished": *dups.finished.lock().unwrap(),
        "deleting": dups.deleting.load(Relaxed),
        "files": files,
        "bytes": bytes,
        "groups": groups,
    })))
}

pub(super) async fn duplicates_search(State(s): State<Shared>) -> StatusCode {
    crate::duplicates::spawn(s.scan.db_path.clone(), s.status.dups.clone());
    StatusCode::ACCEPTED
}

#[derive(Deserialize)]
pub(super) struct DupDelete {
    /// only these photo ids (default: every duplicate copy)
    ids: Option<Vec<i64>>,
    #[serde(default)]
    permanently: bool,
}

/// Moves the duplicate copies to the bin (or deletes them for good when asked), keeping the
/// oldest file of each set.
pub(super) async fn duplicates_delete(State(s): State<Shared>, Json(body): Json<DupDelete>) -> ApiResult<Response> {
    let dups = s.status.dups.clone();
    if dups.deleting.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err(ApiError::conflict("the duplicates are already being deleted"));
    }
    let scan_cfg = s.scan.clone();
    let progress = dups.clone();
    let result = db(&s, move |conn| {
        // Cleared when the work ends, even if the page that asked for it is gone by then.
        let _deleting = crate::scan::ClearOnDrop(&progress.deleting);
        let roots = scan_cfg.roots(conn)?;
        crate::duplicates::delete(conn, &roots, body.ids.as_deref(), body.permanently, &progress)
    })
    .await;
    Ok(Json(serde_json::to_value(result?)?).into_response())
}

/// How far the deletion has got; cheap (no database), for polling while it runs.
pub(super) async fn duplicates_progress(State(s): State<Shared>) -> Json<JsonValue> {
    use std::sync::atomic::Ordering::Relaxed;
    let dups = &s.status.dups;
    Json(json!({
        "deleting": dups.deleting.load(Relaxed),
        "done": dups.delete_done.load(Relaxed),
        "total": dups.delete_total.load(Relaxed),
        "freed": dups.delete_freed.load(Relaxed),
    }))
}

/// Brings back the photos removed from the gallery (they are indexed again).
pub(super) async fn clear_excluded(State(s): State<Shared>) -> ApiResult<StatusCode> {
    db(&s, |conn| crate::db::clear_excluded(conn)).await?;
    scan::spawn(s.scan.clone(), s.status.clone());
    Ok(StatusCode::ACCEPTED)
}
