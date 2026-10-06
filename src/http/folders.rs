//! Photo folders: listing, adding, removing, and browsing the computer's folders.

use std::path::PathBuf;

use anyhow::Result;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};

use crate::library;
use crate::scan::{self};

use super::{ApiError, ApiResult, Shared, db};

pub(super) async fn folders(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let scan_cfg = s.scan.clone();
    let desktop = s.host.is_some();
    let list = db(&s, move |conn| {
        let mut out = Vec::new();
        for root in scan_cfg.roots(conn)? {
            let items = library::count_under(conn, &root)?;
            let fixed = scan_cfg.is_fixed(&root);
            out.push(
                json!({ "path": root.to_string_lossy(), "available": library::reachable(&root), "items": items, "fixed": fixed }),
            );
        }
        Ok(out)
    })
    .await?;
    Ok(Json(json!({ "folders": list, "desktop": desktop })))
}

#[derive(Deserialize)]
pub(super) struct FolderBody {
    path: String,
}

/// Saves a folder and indexes it.
pub(super) async fn save_folder(s: &Shared, path: PathBuf) -> ApiResult<Response> {
    let scan_cfg = s.scan.clone();
    let wanted = path.clone();
    let added = db(s, move |conn| library::add_folder(conn, &scan_cfg.fixed_roots, &wanted)).await?;
    match added {
        library::AddFolder::Added(path) => {
            scan::spawn(s.scan.clone(), s.status.clone());
            Ok(Json(json!({ "path": path.to_string_lossy() })).into_response())
        }
        library::AddFolder::Missing => Err(ApiError::bad_request(format!("{} does not exist", path.display()))),
        library::AddFolder::NotAFolder => Err(ApiError::bad_request(format!("{} is not a folder", path.display()))),
        library::AddFolder::AlreadyIn(outer) if dunce::canonicalize(&path).is_ok_and(|p| p == outer) => {
            Err(ApiError::conflict(format!("{} is already in the gallery", path.display())))
        }
        library::AddFolder::AlreadyIn(outer) => {
            Err(ApiError::conflict(format!("{} is already included: it is inside {}", path.display(), outer.display())))
        }
    }
}

pub(super) async fn add_folder(State(s): State<Shared>, Json(body): Json<FolderBody>) -> ApiResult<Response> {
    save_folder(&s, PathBuf::from(body.path.trim())).await
}

/// Opens the native folder picker (desktop app only).
pub(super) async fn pick_folder(State(s): State<Shared>) -> ApiResult<Response> {
    let Some(host) = s.host.clone() else { return Err(ApiError::desktop_only("open a folder picker")) };
    match tokio::task::spawn_blocking(move || host.pick_folder()).await? {
        Some(path) => save_folder(&s, path).await,
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

/// Ends a folder removal however it goes: scans may run again (and one picks up the
/// remaining folders).
pub(super) struct RemovalGuard {
    s: Shared,
}

impl Drop for RemovalGuard {
    fn drop(&mut self) {
        *self.s.status.removing.lock().unwrap() = None;
        self.s.status.hold.store(false, std::sync::atomic::Ordering::SeqCst);
        scan::spawn(self.s.scan.clone(), self.s.status.clone());
    }
}

pub(super) async fn remove_folder(State(s): State<Shared>, Query(body): Query<FolderBody>) -> ApiResult<Response> {
    use std::sync::atomic::Ordering::{Relaxed, SeqCst};
    if s.scan.is_fixed(std::path::Path::new(&body.path)) {
        return Err(ApiError::conflict("this folder is given on the command line; remove it there"));
    }
    {
        let mut removing = s.status.removing.lock().unwrap();
        if removing.is_some() {
            return Err(ApiError::conflict("a folder is being removed; wait for it to finish"));
        }
        *removing = Some(body.path.clone());
    }
    s.status.remove_done.store(0, Relaxed);
    s.status.remove_total.store(0, Relaxed);
    // A scan still running (the folder was just added, say) would put its photos back.
    s.status.hold.store(true, SeqCst);
    // Its own task: it finishes (and lets scans run again) even if the page is closed.
    let task = tokio::spawn(remove_folder_now(s.clone(), body.path));
    let (removed, kept) = task.await??;
    Ok(Json(json!({ "removed": removed, "kept": kept })).into_response())
}

pub(super) async fn remove_folder_now(s: Shared, path: String) -> ApiResult<(usize, usize)> {
    use std::sync::atomic::Ordering::{Relaxed, SeqCst};
    let _guard = RemovalGuard { s: s.clone() };
    while s.status.running.load(SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let scan_cfg = s.scan.clone();
    let status = s.status.clone();
    db(&s, move |conn| {
        let r = library::remove_folder(
            conn,
            &scan_cfg.fixed_roots,
            std::path::Path::new(&path),
            |total| status.remove_total.store(total as u64, Relaxed),
            |done| status.remove_done.store(done as u64, Relaxed),
        )?;
        tracing::info!("removed folder {path} ({} items, {} kept: in another folder)", r.removed, r.kept);
        Ok((r.removed, r.kept))
    })
    .await
}

#[derive(Deserialize)]
pub(super) struct BrowseQuery {
    path: Option<String>,
}

/// The folders inside `path` on the computer running Imadive, for choosing photo folders
/// from a browser. Without a path: next to the first photo folder, or the home folder.
pub(super) async fn browse_folders(State(s): State<Shared>, Query(q): Query<BrowseQuery>) -> ApiResult<Response> {
    let scan_cfg = s.scan.clone();
    let start = match q.path.filter(|p| !p.trim().is_empty()) {
        Some(p) => Some(PathBuf::from(p.trim())),
        None => {
            let roots = db(&s, move |conn| scan_cfg.roots(conn)).await?;
            roots
                .into_iter()
                .find_map(|r| r.parent().filter(|p| p.is_dir()).map(std::path::Path::to_path_buf))
                .or_else(|| std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from))
        }
    };
    let listing = tokio::task::spawn_blocking(move || list_dirs(start)).await?;
    listing.map(|v| Json(v).into_response()).map_err(|e| ApiError::bad_request(e.to_string()))
}

pub(super) fn list_dirs(path: Option<PathBuf>) -> Result<JsonValue> {
    let entry = |p: &std::path::Path, name: String| json!({ "name": name, "path": p.to_string_lossy() });
    // Windows without a path: the drives.
    let Some(path) = path.or_else(|| (!cfg!(windows)).then(|| PathBuf::from("/"))) else {
        let drives: Vec<JsonValue> = (b'A'..=b'Z')
            .map(|d| PathBuf::from(format!("{}:\\", d as char)))
            .filter(|p| p.is_dir())
            .map(|p| entry(&p, p.to_string_lossy().into_owned()))
            .collect();
        return Ok(json!({ "path": "", "parent": null, "dirs": drives }));
    };
    let path = dunce::canonicalize(&path).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    let mut dirs: Vec<(String, PathBuf)> = std::fs::read_dir(&path)
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(name, _)| !name.starts_with('.') && !name.starts_with('$'))
        .collect();
    dirs.sort_by_key(|(name, _)| name.to_lowercase());
    dirs.truncate(5000);
    // Windows: above a drive's root come the drives ("").
    let parent = path.parent().map(|p| p.to_string_lossy().into_owned()).or_else(|| cfg!(windows).then(String::new));
    Ok(json!({
        "path": path.to_string_lossy(),
        "parent": parent,
        "dirs": dirs.iter().map(|(name, p)| entry(p, name.clone())).collect::<Vec<_>>(),
    }))
}
