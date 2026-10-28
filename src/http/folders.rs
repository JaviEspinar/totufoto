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
use crate::scan::{self, Work, WorkGuard};

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
#[derive(Deserialize, Default)]
pub(super) struct PickBody {
    /// false: only answer the folder chosen (`{"path"}`), to tell where a folder was moved.
    save: Option<bool>,
}

pub(super) async fn pick_folder(State(s): State<Shared>, body: Option<Json<PickBody>>) -> ApiResult<Response> {
    let Some(host) = s.host.clone() else { return Err(ApiError::desktop_only("open a folder picker")) };
    let save = body.map(|Json(b)| b).unwrap_or_default().save.unwrap_or(true);
    match tokio::task::spawn_blocking(move || host.pick_folder()).await? {
        Some(path) if save => save_folder(&s, path).await,
        Some(path) => Ok(Json(json!({ "path": path.to_string_lossy() })).into_response()),
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

#[derive(Deserialize)]
pub(super) struct MoveBody {
    from: String,
    to: String,
}

/// A saved folder moved elsewhere (or renamed, or copied to another drive): its items keep
/// everything under their new paths instead of being indexed again (see
/// `library::move_folder`). Scans wait while it runs; one starts afterwards, for files that
/// are new or changed in the new place.
pub(super) async fn move_folder(State(s): State<Shared>, Json(body): Json<MoveBody>) -> ApiResult<Response> {
    if s.scan.is_fixed(std::path::Path::new(&body.from)) {
        return Err(ApiError::conflict("this folder is given on the command line: change it there"));
    }
    let _work = s.status.begin(Work::MovingFolder, &s.scan).map_err(super::busy)?;
    super::scan_stopped(&s).await;
    let scan_cfg = s.scan.clone();
    let (from, to) = (PathBuf::from(&body.from), PathBuf::from(&body.to));
    let moved = db(&s, move |conn| library::move_folder(conn, &scan_cfg.fixed_roots, &from, &to)).await?;
    match moved {
        library::MoveFolder::Moved(r, to) => {
            tracing::info!(
                "moved folder {} to {} ({} items kept, {} changed, {} missing)",
                body.from,
                to.display(),
                r.moved,
                r.changed,
                r.missing
            );
            let to = to.to_string_lossy();
            Ok(Json(json!({ "path": to, "moved": r.moved, "changed": r.changed, "missing": r.missing }))
                .into_response())
        }
        library::MoveFolder::NotSaved => Err(ApiError::conflict("that folder isn't in the gallery")),
        library::MoveFolder::Missing => Err(ApiError::bad_request(format!("{} does not exist", body.to))),
        library::MoveFolder::NotAFolder => Err(ApiError::bad_request(format!("{} is not a folder", body.to))),
        library::MoveFolder::Overlaps(other) => {
            let other = other.display();
            Err(ApiError::conflict(format!("{} overlaps {other}, which is already in the gallery", body.to)))
        }
    }
}

pub(super) async fn remove_folder(State(s): State<Shared>, Query(body): Query<FolderBody>) -> ApiResult<Response> {
    use std::sync::atomic::Ordering::Relaxed;
    if s.scan.is_fixed(std::path::Path::new(&body.path)) {
        return Err(ApiError::conflict("this folder is given on the command line; remove it there"));
    }
    // Scans are held: one still running (the folder was just added, say) would put its photos back.
    let work = s.status.begin(Work::RemovingFolder(body.path.clone()), &s.scan).map_err(super::busy)?;
    s.status.remove_done.store(0, Relaxed);
    s.status.remove_total.store(0, Relaxed);
    // Its own task: it finishes (and lets scans run again) even if the page is closed.
    let task = tokio::spawn(remove_folder_now(s.clone(), body.path, work));
    let (removed, kept) = task.await??;
    Ok(Json(json!({ "removed": removed, "kept": kept })).into_response())
}

pub(super) async fn remove_folder_now(s: Shared, path: String, _work: WorkGuard) -> ApiResult<(usize, usize)> {
    use std::sync::atomic::Ordering::Relaxed;
    super::scan_stopped(&s).await;
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
    /// Also list the files with this extension (`imadive`: backups), to choose one.
    files: Option<String>,
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
    let ext = q.files.filter(|e| !e.is_empty());
    let listing = tokio::task::spawn_blocking(move || {
        let mut listing = list_dirs(start)?;
        if let Some(ext) = ext
            && let Some(dir) = listing["path"].as_str().filter(|p| !p.is_empty()).map(PathBuf::from)
        {
            listing["files"] = list_files(&dir, &ext).into();
        }
        Ok::<_, anyhow::Error>(listing)
    })
    .await?;
    listing.map(|v| Json(v).into_response()).map_err(|e| ApiError::bad_request(e.to_string()))
}

/// The files in `dir` ending in `.ext`, by name: `[{"name", "path"}]`.
fn list_files(dir: &std::path::Path, ext: &str) -> Vec<JsonValue> {
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                .filter(|(name, path)| {
                    !name.starts_with('.') && path.is_file() && crate::imaging::extension(path) == ext
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|(name, _)| name.to_lowercase());
    files.iter().map(|(name, p)| json!({ "name": name, "path": p.to_string_lossy() })).collect()
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
