//! Photos: lists, groups, details, pictures, rotating, removing.

use std::path::PathBuf;

use anyhow::Result;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};

use crate::{imaging, library};

use super::{ApiError, ApiResult, IMMUTABLE, Shared, db};
use crate::db::filters::PhotoQuery;
use crate::db::photos::{self, GroupBy, Groups, PhotoDetail, PhotoList, Place};

pub(super) async fn photos(State(s): State<Shared>, Query(q): Query<PhotoQuery>) -> ApiResult<Json<PhotoList>> {
    Ok(Json(db(&s, move |conn| photos::list_photos(conn, &q)).await?))
}

/// One line per group of the photos matching the filters, for the group cards.
pub(super) async fn groups(State(s): State<Shared>, Query(q): Query<PhotoQuery>) -> ApiResult<Json<Groups>> {
    let Some(by) = GroupBy::parse(q.by.as_deref()) else {
        return Err(ApiError::bad_request("by must be year, month, day or place"));
    };
    Ok(Json(db(&s, move |conn| photos::groups(conn, &q, by)).await?))
}

/// Called when a photo can't be opened. If its file is gone but its folder is there, the
/// photo is removed from the index ("removed"); if the whole folder can't be reached (an
/// unplugged drive, say) nothing is removed ("unavailable"); "present" if the file is there.
pub(super) async fn check_photo(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Json<JsonValue>> {
    let scan_cfg = s.scan.clone();
    let checked = db(&s, move |conn| {
        let roots = scan_cfg.roots(conn)?;
        library::check_photo(conn, &roots, id)
    })
    .await?;
    Ok(Json(match checked {
        library::CheckPhoto::Present(path) => json!({ "status": "present", "path": path }),
        library::CheckPhoto::Removed(path) => json!({ "status": "removed", "path": path }),
        library::CheckPhoto::Unavailable { path, folder } => {
            json!({ "status": "unavailable", "path": path, "folder": folder.to_string_lossy() })
        }
    }))
}

#[derive(Deserialize)]
pub(super) struct RemoveBody {
    /// "gallery": out of the gallery, the file stays; "disk": the file goes to the bin
    from: String,
    /// with "disk": delete for good when there is no bin to move the file to
    #[serde(default)]
    permanently: bool,
}

/// Removes a photo. From the gallery: the file stays, and scans leave it out from then on.
/// From disk: the file is moved to the bin of the computer running the gallery; if that
/// isn't possible the answer is "no-bin", and the file is only deleted for good when asked
/// again with `permanently`. Only files inside the library folders can be deleted.
pub(super) async fn remove_photo(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<RemoveBody>,
) -> ApiResult<Response> {
    use library::RemovePhoto;
    let from = match body.from.as_str() {
        "gallery" => library::RemoveFrom::Gallery,
        "disk" => library::RemoveFrom::Disk { permanently: body.permanently },
        _ => return Err(ApiError::bad_request("from must be gallery or disk")),
    };
    let scan_cfg = s.scan.clone();
    let removed = db(&s, move |conn| {
        let roots = scan_cfg.roots(conn)?;
        library::remove_photo(conn, &roots, id, from)
    })
    .await?;
    let ok = |status: &str, path: String| Ok(Json(json!({ "status": status, "path": path })).into_response());
    match removed {
        RemovePhoto::Removed(path) => ok("removed", path),
        RemovePhoto::Binned(path) => ok("binned", path),
        RemovePhoto::Deleted(path) => ok("deleted", path),
        // Not an error: the page asks whether to delete it for good instead.
        RemovePhoto::NoBin { path, error } => {
            Ok((StatusCode::CONFLICT, Json(json!({ "status": "no-bin", "error": error, "path": path })))
                .into_response())
        }
        RemovePhoto::NotFound => Err(ApiError::not_found("photo")),
        RemovePhoto::Outside => Err(ApiError::forbidden("the file is outside the photo folders")),
    }
}

pub(super) async fn photo_detail(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Json<PhotoDetail>> {
    db(&s, move |conn| photos::photo_detail(conn, id)).await?.map(Json).ok_or_else(|| ApiError::not_found("photo"))
}

#[derive(Deserialize)]
pub(super) struct PlacesQuery {
    /// Only count photos of these people (comma separated ids), as in /api/photos.
    people: Option<String>,
    #[serde(rename = "match")]
    match_mode: Option<String>,
    /// Only count photos taken in this range (YYYY-MM-DD, both days included).
    from: Option<String>,
    to: Option<String>,
}

pub(super) async fn places(State(s): State<Shared>, Query(q): Query<PlacesQuery>) -> ApiResult<Json<Vec<Place>>> {
    Ok(Json(
        db(&s, move |conn| {
            photos::places(conn, q.people.as_deref(), q.match_mode.as_deref(), q.from.as_deref(), q.to.as_deref())
        })
        .await?,
    ))
}

pub(super) fn jpeg(bytes: Vec<u8>, cache: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, cache)], bytes).into_response()
}

pub(super) async fn thumb(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    let data = db(&s, move |conn| photos::thumbnail(conn, id)).await?;
    data.map(|d| jpeg(d, IMMUTABLE)).ok_or_else(|| ApiError::not_found("picture"))
}

pub(super) async fn face_thumb(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    let data = db(&s, move |conn| photos::face_picture(conn, id)).await?;
    data.map(|d| jpeg(d, IMMUTABLE)).ok_or_else(|| ApiError::not_found("picture"))
}

#[derive(Deserialize)]
pub(super) struct RotateBody {
    /// quarter turns clockwise (negative: counterclockwise)
    turns: i32,
}

/// Turns the photo in its file (JPEG: its EXIF orientation, so nothing is recompressed;
/// PNG: rewritten turned) and in the gallery.
pub(super) async fn rotate_photo(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<RotateBody>,
) -> ApiResult<Response> {
    if s.status.running.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(ApiError::conflict("the photos are being scanned; try again when the scan ends"));
    }
    let turns = body.turns.rem_euclid(4) as u8;
    let scan_cfg = s.scan.clone();
    let outcome = db(&s, move |conn| {
        let roots = scan_cfg.roots(conn)?;
        crate::rotate::rotate_photo(conn, &roots, id, turns)
    })
    .await?;
    use crate::rotate::Outcome;
    match outcome {
        Outcome::Rotated { width, height, version } => {
            Ok(Json(json!({ "width": width, "height": height, "version": version })).into_response())
        }
        Outcome::NotFound => Err(ApiError::not_found("photo")),
        Outcome::Unsupported => {
            Err(ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "only JPEG and PNG photos can be rotated"))
        }
        Outcome::Outside => Err(ApiError::forbidden("the file is not in the photo folders")),
        Outcome::Changed => Err(ApiError::conflict("the file changed since it was indexed; rescan first")),
    }
}

/// Shows the photo's file in the file manager (desktop app only; 409 otherwise).
pub(super) async fn reveal_photo(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    let path = db(&s, move |conn| photos::photo_path(conn, id)).await?;
    let Some(path) = path.map(PathBuf::from) else { return Err(ApiError::not_found("photo")) };
    let Some(host) = &s.host else {
        return Err(ApiError::desktop_only("open folders"));
    };
    host.reveal(&path);
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[derive(Deserialize)]
pub(super) struct OriginalQuery {
    /// `download=1`: save as a file (Content-Disposition: attachment) instead of showing it
    download: Option<String>,
}

impl OriginalQuery {
    fn download(&self) -> bool {
        self.download.as_deref().is_some_and(|v| !matches!(v, "0" | "false"))
    }
}

/// `attachment; filename=...` with the file's own name (ASCII fallback plus UTF-8).
pub(super) fn attachment(name: &str) -> String {
    let ascii: String =
        name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    let encoded: String = name
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

pub(super) async fn original(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Query(q): Query<OriginalQuery>,
) -> ApiResult<Response> {
    let path = db(&s, move |conn| photos::photo_path(conn, id)).await?;
    let Some(path) = path.map(PathBuf::from) else { return Err(ApiError::not_found("photo")) };
    if !path.is_file() {
        // Deleted outside the gallery (or its drive is unplugged): the page asks /check.
        return Err(ApiError::not_found("file in its folder"));
    }
    let ext = imaging::extension(&path);
    if imaging::BROWSER_EXTENSIONS.contains(&ext.as_str()) {
        let mime = match ext.as_str() {
            "png" => "image/png",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "bmp" => "image/bmp",
            _ => "image/jpeg",
        };
        let bytes = tokio::fs::read(&path).await?;
        let mut response =
            ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "public, max-age=3600")], bytes).into_response();
        if q.download() {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("photo-{id}.{ext}"));
            response.headers_mut().insert(header::CONTENT_DISPOSITION, attachment(&name).parse()?);
        }
        return Ok(response);
    }
    // HEIC, TIFF...: convert to a large JPEG on the fly, and save it as the JPEG it is now.
    let jpeg_name = format!(
        "{}.jpg",
        path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("photo-{id}"))
    );
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let raw = std::fs::read(&path)?;
        let img = imaging::decode(&path, &raw)?.into_rgb8();
        imaging::encode_jpeg(&imaging::fit(&img, 2560)?, 88)
    })
    .await??;
    let mut response = jpeg(bytes, "public, max-age=3600");
    if q.download() {
        response.headers_mut().insert(header::CONTENT_DISPOSITION, attachment(&jpeg_name).parse()?);
    }
    Ok(response)
}
