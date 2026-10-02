//! Photos: lists, groups, details, pictures, rotating, removing.

use std::path::PathBuf;

use anyhow::Result;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params_from_iter};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};

use crate::{imaging, library};

use super::filters::{PhotoQuery, date_range_filter, people_filter, photo_filters};
use super::{ApiError, ApiResult, IMMUTABLE, Shared, db};

pub(super) async fn photos(State(s): State<Shared>, Query(q): Query<PhotoQuery>) -> ApiResult<Json<JsonValue>> {
    let result = db(&s, move |conn| {
        let (filters, args, days) = photo_filters(&q);
        let desc = q.sort.as_deref() != Some("asc");
        let order = if desc { "taken DESC, id DESC" } else { "taken ASC, id ASC" };
        let sql = format!(
            "SELECT id, width, height, taken, place_id, version FROM photos WHERE 1 = 1{filters} ORDER BY {order}"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows: Vec<JsonValue> = stmt
            .query_map(params_from_iter(args), |r| {
                Ok(json!([
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<i64>>(4)?,
                    r.get::<_, i64>(5)?
                ]))
            })?
            .collect::<Result<_, _>>()?;
        Ok(json!({ "photos": rows, "days": days }))
    })
    .await?;
    Ok(Json(result))
}

/// One line per group of the photos matching the filters, for the group cards: the key
/// (year "2024", month "2024-10", day "2024-10-05", or place id with 0 for no location), the
/// number of photos, and a cover photo (the newest, or the oldest when sorting oldest
/// first). Dates follow the sort; places come with the most photos first.
pub(super) async fn groups(State(s): State<Shared>, Query(q): Query<PhotoQuery>) -> ApiResult<Response> {
    let key = match q.by.as_deref() {
        Some("year") => "substr(taken, 1, 4)",
        Some("month") => "substr(taken, 1, 7)",
        Some("day") => "substr(taken, 1, 10)",
        Some("place") => "COALESCE(place_id, 0)",
        _ => return Err(ApiError::bad_request("by must be year, month, day or place")),
    };
    let by_place = q.by.as_deref() == Some("place");
    let result = db(&s, move |conn| {
        let (filters, args, _) = photo_filters(&q);
        let desc = q.sort.as_deref() != Some("asc");
        // SQLite takes the other columns from the row that has the MAX/MIN: the cover.
        let pick = if desc { "MAX(taken)" } else { "MIN(taken)" };
        let order = if by_place { "n DESC, k" } else if desc { "k DESC" } else { "k ASC" };
        let sql = format!("SELECT {key} AS k, COUNT(*) AS n, id, {pick}, version FROM photos WHERE 1 = 1{filters} GROUP BY k ORDER BY {order}");
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut total = 0i64;
        let rows: Vec<JsonValue> = stmt
            .query_map(params_from_iter(args), |r| {
                let key: JsonValue = if by_place { r.get::<_, i64>(0)?.into() } else { r.get::<_, String>(0)?.into() };
                Ok((key, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(4)?))
            })?
            .map(|row| {
                row.map(|(key, count, cover, version)| {
                    total += count;
                    json!({ "key": key, "count": count, "cover": cover, "v": version })
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(json!({ "groups": rows, "total": total }))
    })
    .await?;
    Ok(Json(result).into_response())
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

pub(super) async fn photo_detail(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Json<JsonValue>> {
    let result = db(&s, move |conn| {
        let photo = conn
            .query_row(
                "SELECT p.path, p.taken, p.date_from_exif, p.width, p.height, p.lat, p.lon, pl.city, pl.region, pl.country, p.version
                 FROM photos p LEFT JOIN places pl ON pl.id = p.place_id WHERE p.id = ?",
                [id],
                |r| {
                    Ok(json!({
                        "id": id,
                        "path": r.get::<_, String>(0)?,
                        "taken": r.get::<_, String>(1)?,
                        "dateFromExif": r.get::<_, bool>(2)?,
                        "width": r.get::<_, i64>(3)?,
                        "height": r.get::<_, i64>(4)?,
                        "lat": r.get::<_, Option<f64>>(5)?,
                        "lon": r.get::<_, Option<f64>>(6)?,
                        "city": r.get::<_, Option<String>>(7)?,
                        "region": r.get::<_, Option<String>>(8)?,
                        "country": r.get::<_, Option<String>>(9)?,
                        "version": r.get::<_, i64>(10)?,
                        "rotatable": crate::rotate::can_rotate(std::path::Path::new(&r.get::<_, String>(0)?)),
                    }))
                },
            )
            .optional()?;
        let Some(mut photo) = photo else { return Ok(JsonValue::Null) };
        let faces: Vec<JsonValue> = conn
            .prepare_cached(
                "SELECT f.id, f.x, f.y, f.w, f.h, f.person_id, p.name FROM faces f
                 LEFT JOIN persons p ON p.id = f.person_id WHERE f.photo_id = ? ORDER BY f.x",
            )?
            .query_map([id], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "box": [r.get::<_, f64>(1)?, r.get::<_, f64>(2)?, r.get::<_, f64>(3)?, r.get::<_, f64>(4)?],
                    "person": r.get::<_, Option<i64>>(5)?,
                    "name": r.get::<_, Option<String>>(6)?,
                }))
            })?
            .collect::<Result<_, _>>()?;
        photo["faces"] = faces.into();
        Ok(photo)
    })
    .await?;
    if result.is_null() {
        return Err(ApiError::not_found("photo"));
    }
    Ok(Json(result))
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

pub(super) async fn places(State(s): State<Shared>, Query(q): Query<PlacesQuery>) -> ApiResult<Json<JsonValue>> {
    let rows = db(&s, move |conn| {
        let filter = people_filter("p.id", q.people.as_deref(), q.match_mode.as_deref());
        let (range, args) = date_range_filter("p.taken", q.from.as_deref(), q.to.as_deref());
        let rows: Vec<JsonValue> = conn
            .prepare_cached(&format!(
                "SELECT pl.id, pl.city, pl.region, pl.country, COUNT(*) AS n, MAX(p.id)
                 FROM photos p JOIN places pl ON pl.id = p.place_id
                 WHERE 1 = 1{filter}{range}
                 GROUP BY pl.id ORDER BY n DESC, pl.city"
            ))?
            .query_map(params_from_iter(args), |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "city": r.get::<_, String>(1)?,
                    "region": r.get::<_, String>(2)?,
                    "country": r.get::<_, String>(3)?,
                    "count": r.get::<_, i64>(4)?,
                    "cover": r.get::<_, i64>(5)?,
                }))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    })
    .await?;
    Ok(Json(rows.into()))
}

pub(super) fn jpeg(bytes: Vec<u8>, cache: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, cache)], bytes).into_response()
}

pub(super) async fn blob(s: &Shared, sql: &'static str, id: i64) -> ApiResult<Response> {
    let data: Option<Vec<u8>> =
        db(s, move |conn| Ok(conn.prepare_cached(sql)?.query_row([id], |r| r.get(0)).optional()?)).await?;
    data.map(|d| jpeg(d, IMMUTABLE)).ok_or_else(|| ApiError::not_found("picture"))
}

pub(super) async fn thumb(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    blob(&s, "SELECT data FROM thumbs WHERE photo_id = ?", id).await
}

pub(super) async fn face_thumb(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    blob(&s, "SELECT thumb FROM faces WHERE id = ?", id).await
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
    let path: Option<String> = db(&s, move |conn| {
        Ok(conn.query_row("SELECT path FROM photos WHERE id = ?", [id], |r| r.get(0)).optional()?)
    })
    .await?;
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
    let path: Option<String> = db(&s, move |conn| {
        Ok(conn.query_row("SELECT path FROM photos WHERE id = ?", [id], |r| r.get(0)).optional()?)
    })
    .await?;
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
