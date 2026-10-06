//! Items (photos and videos): lists, groups, details, pictures, rotating, removing.

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
use crate::db::filters::ItemQuery;
use crate::db::items::{self, GroupBy, Groups, ItemDetail, ItemList, Place};

pub(super) async fn items(State(s): State<Shared>, Query(q): Query<ItemQuery>) -> ApiResult<Json<ItemList>> {
    Ok(Json(db(&s, move |conn| items::list_items(conn, &q)).await?))
}

/// One line per group of the items matching the filters, for the group cards.
pub(super) async fn groups(State(s): State<Shared>, Query(q): Query<ItemQuery>) -> ApiResult<Json<Groups>> {
    let Some(by) = GroupBy::parse(q.by.as_deref()) else {
        return Err(ApiError::bad_request("by must be year, month, day or place"));
    };
    Ok(Json(db(&s, move |conn| items::groups(conn, &q, by)).await?))
}

/// Called when an item can't be opened. If its file is gone but its folder is there, the
/// item is removed from the index ("removed"); if the whole folder can't be reached (an
/// unplugged drive, say) nothing is removed ("unavailable"); "present" if the file is there.
pub(super) async fn check_item(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Json<JsonValue>> {
    let scan_cfg = s.scan.clone();
    let checked = db(&s, move |conn| {
        let roots = scan_cfg.roots(conn)?;
        library::check_item(conn, &roots, id)
    })
    .await?;
    Ok(Json(match checked {
        library::CheckItem::Present(path) => json!({ "status": "present", "path": path }),
        library::CheckItem::Removed(path) => json!({ "status": "removed", "path": path }),
        library::CheckItem::Unavailable { path, folder } => {
            json!({ "status": "unavailable", "path": path, "folder": folder.to_string_lossy() })
        }
    }))
}

#[derive(Deserialize)]
pub(super) struct RemoveQuery {
    /// "gallery": out of the gallery, the file stays; "disk": the file goes to the bin
    from: String,
    /// with "disk": delete for good when there is no bin to move the file to
    #[serde(default)]
    permanently: bool,
}

/// Removes an item. From the gallery: the file stays, and scans leave it out from then on.
/// From disk: the file is moved to the bin of the computer running the gallery; if that
/// isn't possible the answer is "no-bin", and the file is only deleted for good when asked
/// again with `permanently`. Only files inside the library folders can be deleted.
pub(super) async fn remove_item(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Query(body): Query<RemoveQuery>,
) -> ApiResult<Response> {
    use library::RemoveItem;
    let from = match body.from.as_str() {
        "gallery" => library::RemoveFrom::Gallery,
        "disk" => library::RemoveFrom::Disk { permanently: body.permanently },
        _ => return Err(ApiError::bad_request("from must be gallery or disk")),
    };
    let scan_cfg = s.scan.clone();
    let removed = db(&s, move |conn| {
        let roots = scan_cfg.roots(conn)?;
        library::remove_item(conn, &roots, id, from)
    })
    .await?;
    let ok = |status: &str, path: String| Ok(Json(json!({ "status": status, "path": path })).into_response());
    match removed {
        RemoveItem::Removed(path) => ok("removed", path),
        RemoveItem::Binned(path) => ok("binned", path),
        RemoveItem::Deleted(path) => ok("deleted", path),
        // Not an error: the page asks whether to delete it for good instead.
        RemoveItem::NoBin { path, error } => {
            Ok((StatusCode::CONFLICT, Json(json!({ "status": "no-bin", "error": error, "path": path })))
                .into_response())
        }
        RemoveItem::NotFound => Err(ApiError::not_found("photo")),
        RemoveItem::Outside => Err(ApiError::forbidden("the file is outside the photo folders")),
    }
}

pub(super) async fn item_detail(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Json<ItemDetail>> {
    db(&s, move |conn| items::item_detail(conn, id)).await?.map(Json).ok_or_else(|| ApiError::not_found("photo"))
}

#[derive(Deserialize)]
pub(super) struct PlacesQuery {
    /// Only count items of these people (comma separated ids), as in /api/items.
    people: Option<String>,
    #[serde(rename = "match")]
    match_mode: Option<String>,
    /// Only count items taken in this range (YYYY-MM-DD, both days included).
    from: Option<String>,
    to: Option<String>,
}

pub(super) async fn places(State(s): State<Shared>, Query(q): Query<PlacesQuery>) -> ApiResult<Json<Vec<Place>>> {
    Ok(Json(
        db(&s, move |conn| {
            items::places(conn, q.people.as_deref(), q.match_mode.as_deref(), q.from.as_deref(), q.to.as_deref())
        })
        .await?,
    ))
}

pub(super) fn jpeg(bytes: Vec<u8>, cache: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, cache)], bytes).into_response()
}

/// An item's thumbnail at `/thumb/{id}/{version}`. The version in the address is what makes
/// it safe to keep for good: a new version (the item was rotated, or its file changed) has
/// a new address. An address with another version gets the current picture, but not to keep.
pub(super) async fn thumb(State(s): State<Shared>, Path((id, version)): Path<(i64, i64)>) -> ApiResult<Response> {
    let data = db(&s, move |conn| items::thumbnail(conn, id)).await?;
    let (picture, current) = data.ok_or_else(|| ApiError::not_found("picture"))?;
    // A video whose thumbnail the page hasn't made yet: nothing, and the page draws its own.
    let Some(bytes) = picture else {
        return Ok(([(header::CACHE_CONTROL, "no-store")], StatusCode::NO_CONTENT).into_response());
    };
    Ok(jpeg(bytes, if current == version { IMMUTABLE } else { "no-cache" }))
}

#[derive(Deserialize)]
pub(super) struct ThumbQuery {
    /// The video's version the frame was taken from.
    v: i64,
}

/// A video's thumbnail, made by the page from a frame of the video (the server can't decode
/// videos). It is made over like a photo's, which also refuses anything but a JPEG image.
pub(super) async fn set_video_thumb(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Query(q): Query<ThumbQuery>,
    body: axum::body::Bytes,
) -> ApiResult<Json<JsonValue>> {
    let thumb = tokio::task::spawn_blocking(move || -> Result<Option<Vec<u8>>> {
        let Ok(frame) = image::load_from_memory_with_format(&body, image::ImageFormat::Jpeg) else { return Ok(None) };
        Ok(Some(crate::scan::thumbnail(&frame.into_rgb8())?))
    })
    .await??;
    let Some(thumb) = thumb else { return Err(ApiError::bad_request("the picture must be a JPEG image")) };
    match db(&s, move |conn| items::set_video_thumbnail(conn, id, q.v, &thumb)).await? {
        items::SetThumbnail::Saved(version) => Ok(Json(json!({ "version": version }))),
        items::SetThumbnail::NotFound => Err(ApiError::not_found("photo")),
        items::SetThumbnail::NotAVideo => Err(ApiError::conflict("only videos get their thumbnail from the page")),
        items::SetThumbnail::Changed => Err(ApiError::conflict("the video changed; its thumbnail will be made again")),
    }
}

pub(super) async fn face_thumb(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    let data = db(&s, move |conn| items::face_picture(conn, id)).await?;
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

/// Shows the item's file in the file manager (desktop app only; 501 otherwise).
pub(super) async fn reveal_item(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    let path = db(&s, move |conn| items::item_path(conn, id)).await?;
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
    /// `silent=1`: a video without its sound, for when the sound stops the browser playing it
    silent: Option<String>,
}

impl OriginalQuery {
    fn download(&self) -> bool {
        is_on(&self.download)
    }

    fn silent(&self) -> bool {
        is_on(&self.silent)
    }
}

/// A flag in the query: on when present, unless it is `0` or `false`.
fn is_on(flag: &Option<String>) -> bool {
    flag.as_deref().is_some_and(|v| !matches!(v, "0" | "false"))
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
    request: axum::extract::Request,
) -> ApiResult<Response> {
    let path = db(&s, move |conn| items::item_path(conn, id)).await?;
    let Some(path) = path.map(PathBuf::from) else { return Err(ApiError::not_found("photo")) };
    if !path.is_file() {
        // Deleted outside the gallery (or its drive is unplugged): the page asks /check.
        return Err(ApiError::not_found("file in its folder"));
    }
    if crate::video::is_video(&path) {
        return serve_video(&path, q.download(), q.silent(), request).await;
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

/// A video, streamed from disk with range requests: the browser's player asks for the parts
/// it needs, so it can start at once and seek, and nothing is read into memory.
/// A video, in the parts the browser asks for (Range). `silent`: with its sound tracks
/// relabelled as filler as the bytes go out (see `video::sound_track_types`).
async fn serve_video(
    path: &std::path::Path,
    download: bool,
    silent: bool,
    request: axum::extract::Request,
) -> ApiResult<Response> {
    use tower::ServiceExt;
    let service = tower_http::services::ServeFile::new_with_mime(path, &crate::video::mime(path).parse()?);
    let mut response = service.oneshot(request).await?.map(axum::body::Body::new);
    if silent {
        let owned = path.to_path_buf();
        let at = tokio::task::spawn_blocking(move || crate::video::sound_track_types(&owned)).await??;
        response = without_sound(response, at);
    }
    response.headers_mut().insert(header::CACHE_CONTROL, "public, max-age=3600".parse()?);
    if download {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "video".into());
        response.headers_mut().insert(header::CONTENT_DISPOSITION, attachment(&name).parse()?);
    }
    Ok(response)
}

/// The response with `free` written over the four bytes at each of `at` (file offsets)
/// that it carries. A part (206) starts where its Content-Range says.
fn without_sound(response: Response, at: Vec<u64>) -> Response {
    use futures_util::StreamExt;
    if at.is_empty() {
        return response;
    }
    let mut offset = match response.status() {
        StatusCode::OK => 0,
        StatusCode::PARTIAL_CONTENT => {
            let range = response.headers().get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok());
            match range.and_then(|r| r.strip_prefix("bytes ")?.split('-').next()?.parse::<u64>().ok()) {
                Some(start) => start,
                None => return response,
            }
        }
        _ => return response,
    };
    let (parts, body) = response.into_parts();
    let patched = body.into_data_stream().map(move |chunk| {
        chunk.map(|bytes| {
            let end = offset + bytes.len() as u64;
            let inside = |pos: u64| (offset..end).contains(&pos);
            let out = if at.iter().any(|&p| (p..p + 4).any(inside)) {
                let mut copy = bytes.to_vec();
                for &p in &at {
                    for (pos, b) in (p..p + 4).zip(b"free") {
                        if inside(pos) {
                            copy[(pos - offset) as usize] = *b;
                        }
                    }
                }
                axum::body::Bytes::from(copy)
            } else {
                bytes
            };
            offset = end;
            out
        })
    });
    Response::from_parts(parts, axum::body::Body::from_stream(patched))
}
