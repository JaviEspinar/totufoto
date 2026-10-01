//! HTTP API and embedded web UI.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::extract::{ConnectInfo, Path, Query, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Datelike, Duration, Local};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use tower_http::compression::CompressionLayer;

use crate::app::Host;
use crate::imaging;
use crate::scan::{self, ScanConfig, ScanStatus};

const INDEX_HTML: &str = include_str!("../web/index.html");
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

/// Tiny SQLite connection pool: connections are cheap, but reusing them keeps the page cache warm.
pub struct Pool {
    path: PathBuf,
    idle: Mutex<Vec<Connection>>,
}

impl Pool {
    pub fn new(path: PathBuf) -> Self {
        Self { path, idle: Mutex::new(Vec::new()) }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let conn = self.idle.lock().unwrap().pop();
        let mut conn = match conn {
            Some(c) => c,
            None => crate::db::open(&self.path)?,
        };
        let out = f(&mut conn);
        self.idle.lock().unwrap().push(conn);
        out
    }
}

pub struct AppState {
    pub pool: Pool,
    pub status: Arc<ScanStatus>,
    pub scan: Arc<ScanConfig>,
    pub host: Option<Arc<dyn Host>>,
}

type Shared = Arc<AppState>;

struct ApiError(anyhow::Error);

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        tracing::error!("{:#}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, format!("{:#}", self.0)).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

async fn db<T: Send + 'static>(
    state: &Shared,
    f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
) -> ApiResult<T> {
    let state = state.clone();
    Ok(tokio::task::spawn_blocking(move || state.pool.with(f)).await??)
}

pub fn router(state: AppState, allowed_hosts: Option<Vec<String>>) -> Router {
    let router = Router::new()
        .route("/", get(|| async { Html(INDEX_HTML) }))
        .route("/api/status", get(status))
        .route("/api/scan", post(start_scan))
        .route("/api/regroup", post(regroup))
        .route("/api/failures", get(failures))
        .route("/api/failures/retry", post(retry_failures))
        .route("/api/folders", get(folders).post(add_folder))
        .route("/api/folders/pick", post(pick_folder))
        .route("/api/folders/remove", post(remove_folder))
        .route("/api/open", post(open_url))
        .route("/api/photos", get(photos))
        .route("/api/photos/{id}", get(photo_detail))
        .route("/api/photos/{id}/check", post(check_photo))
        .route("/api/photos/{id}/remove", post(remove_photo))
        .route("/api/photos/{id}/reveal", post(reveal_photo))
        .route("/api/excluded/clear", post(clear_excluded))
        .route("/api/duplicates", get(duplicates_report))
        .route("/api/duplicates/search", post(duplicates_search))
        .route("/api/duplicates/delete", post(duplicates_delete))
        .route("/api/groups", get(groups))
        .route("/api/places", get(places))
        .route("/api/people", get(people))
        .route("/api/people/{id}", post(update_person))
        .route("/api/people/{id}/merge", post(merge_person))
        .route("/api/faces/{id}/reject", post(reject_face))
        .route("/api/faces/{id}/assign", post(assign_face))
        .route("/thumb/{id}", get(thumb))
        .route("/face/{id}", get(face_thumb))
        .route("/original/{id}", get(original))
        .layer(CompressionLayer::new())
        .with_state(Arc::new(state));
    match allowed_hosts {
        Some(hosts) => router.layer(middleware::from_fn_with_state(Arc::new(hosts), check_host)),
        None => router,
    }
}

async fn check_host(State(allowed): State<Arc<Vec<String>>>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok());
    if host.is_some_and(|h| allowed.iter().any(|a| a.eq_ignore_ascii_case(h))) {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "unexpected Host header").into_response()
    }
}

async fn status(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let view = s.status.view();
    // Files that could not be indexed, saved across scans (see /api/failures).
    let (failed, excluded): (i64, i64) = db(&s, |conn| {
        Ok(conn.query_row("SELECT (SELECT COUNT(*) FROM failures), (SELECT COUNT(*) FROM excluded)", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?)
    })
    .await?;
    let mut value = serde_json::to_value(view)?;
    value["failed"] = failed.into();
    // Photos removed from the gallery (their files kept); see /api/excluded/clear.
    value["excluded"] = excluded.into();
    Ok(Json(value))
}

/// Files that could not be indexed, with the reason (first 500).
async fn failures(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let rows = db(&s, |conn| {
        let rows: Vec<JsonValue> = conn
            .prepare_cached("SELECT path, error FROM failures ORDER BY path LIMIT 500")?
            .query_map([], |r| Ok(json!({ "path": r.get::<_, String>(0)?, "error": r.get::<_, String>(1)? })))?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    })
    .await?;
    Ok(Json(rows.into()))
}

/// Forgets the failures and scans again, so those files are tried once more.
async fn retry_failures(State(s): State<Shared>) -> ApiResult<StatusCode> {
    db(&s, |conn| Ok(conn.execute("DELETE FROM failures", [])?)).await?;
    scan::spawn(s.scan.clone(), s.status.clone());
    Ok(StatusCode::ACCEPTED)
}

/// Scans and then regroups every face from scratch (named people are kept).
async fn regroup(State(s): State<Shared>) -> StatusCode {
    s.status.request_regroup();
    scan::spawn(s.scan.clone(), s.status.clone());
    StatusCode::ACCEPTED
}

async fn start_scan(State(s): State<Shared>) -> StatusCode {
    scan::spawn(s.scan.clone(), s.status.clone());
    StatusCode::ACCEPTED
}

async fn folders(State(s): State<Shared>, ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>) -> ApiResult<Json<JsonValue>> {
    let scan_cfg = s.scan.clone();
    let managed = s.scan.fixed_roots.is_empty();
    let desktop = s.host.is_some();
    // Folders can only be opened for someone on the computer that has the photos.
    let local = desktop || peer.ip().is_loopback();
    let list = db(&s, move |conn| {
        let mut count = conn.prepare_cached("SELECT COUNT(*) FROM photos WHERE substr(path, 1, ?1) = ?2")?;
        let mut out = Vec::new();
        for root in scan_cfg.roots(conn)? {
            // Match "<root><separator>" so /photos doesn't also count /photos2.
            let prefix = root.join("").to_string_lossy().into_owned();
            let photos: i64 = count.query_row(params![prefix.chars().count() as i64, prefix], |r| r.get(0))?;
            out.push(json!({ "path": root.to_string_lossy(), "available": root.is_dir(), "photos": photos }));
        }
        Ok(out)
    })
    .await?;
    Ok(Json(json!({ "folders": list, "managed": managed, "desktop": desktop, "local": local })))
}

#[derive(Deserialize)]
struct FolderBody {
    path: String,
}

/// Saves a folder and indexes it. Only when folders aren't fixed on the command line.
async fn save_folder(s: &Shared, path: PathBuf) -> ApiResult<Response> {
    if !s.scan.fixed_roots.is_empty() {
        return Ok((StatusCode::CONFLICT, "folders are set on the command line").into_response());
    }
    let Ok(path) = dunce::canonicalize(&path) else {
        return Ok((StatusCode::BAD_REQUEST, format!("{} does not exist", path.display())).into_response());
    };
    if !path.is_dir() {
        return Ok((StatusCode::BAD_REQUEST, format!("{} is not a folder", path.display())).into_response());
    }
    let stored = path.to_string_lossy().into_owned();
    db(s, move |conn| {
        conn.execute("INSERT OR IGNORE INTO folders (path) VALUES (?)", [stored])?;
        Ok(())
    })
    .await?;
    scan::spawn(s.scan.clone(), s.status.clone());
    Ok(Json(json!({ "path": path.to_string_lossy() })).into_response())
}

async fn add_folder(State(s): State<Shared>, Json(body): Json<FolderBody>) -> ApiResult<Response> {
    save_folder(&s, PathBuf::from(body.path.trim())).await
}

/// Opens the native folder picker (desktop app only).
async fn pick_folder(State(s): State<Shared>) -> ApiResult<Response> {
    let Some(host) = s.host.clone() else { return Ok(StatusCode::NOT_IMPLEMENTED.into_response()) };
    match tokio::task::spawn_blocking(move || host.pick_folder()).await? {
        Some(path) => save_folder(&s, path).await,
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

async fn remove_folder(State(s): State<Shared>, Json(body): Json<FolderBody>) -> ApiResult<StatusCode> {
    if !s.scan.fixed_roots.is_empty() {
        return Ok(StatusCode::CONFLICT);
    }
    db(&s, move |conn| {
        // Match "<folder><separator>" so removing /photos doesn't also remove /photos2.
        let prefix = PathBuf::from(&body.path).join("").to_string_lossy().into_owned();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM folders WHERE path = ?", [&body.path])?;
        let removed = tx.execute(
            "DELETE FROM photos WHERE substr(path, 1, ?1) = ?2",
            params![prefix.chars().count() as i64, prefix],
        )?;
        tx.execute(
            "DELETE FROM persons WHERE name IS NULL AND id NOT IN (SELECT DISTINCT person_id FROM faces WHERE person_id IS NOT NULL)",
            [],
        )?;
        tx.commit()?;
        tracing::info!("removed folder {} ({removed} photos)", body.path);
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct OpenBody {
    url: String,
}

/// Opens a map link in the system browser (desktop app only). Limited to the map site
/// the UI links to, so the endpoint can't be used to launch arbitrary URLs.
async fn open_url(State(s): State<Shared>, Json(body): Json<OpenBody>) -> StatusCode {
    match &s.host {
        Some(host) if body.url.starts_with("https://www.openstreetmap.org/") => {
            host.open_url(&body.url);
            StatusCode::NO_CONTENT
        }
        Some(_) => StatusCode::FORBIDDEN,
        None => StatusCode::NOT_IMPLEMENTED,
    }
}

#[derive(Deserialize)]
struct PhotoQuery {
    /// "asc" or "desc" (default) by capture date
    sort: Option<String>,
    place: Option<i64>,
    /// comma separated person ids
    people: Option<String>,
    /// "all" (default): every selected person appears, "any": at least one,
    /// "only": all of them and no other known person
    #[serde(rename = "match")]
    match_mode: Option<String>,
    /// YYYY, YYYY-MM or YYYY-MM-DD prefix of the capture date
    date: Option<String>,
    /// capture date range, YYYY-MM-DD, both days included
    from: Option<String>,
    to: Option<String>,
    /// photos from previous years whose anniversary falls within the next `upcoming` days
    upcoming: Option<u32>,
    /// /api/groups only: "year", "month", "day" or "place"
    by: Option<String>,
}

fn parse_ids(s: &str) -> Vec<i64> {
    s.split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

/// SQL condition (starting with " AND") and its arguments keeping photos whose `taken`
/// column falls in the range, both days included. Either end may be missing; invalid
/// dates are ignored, and a reversed range is put the right way round.
fn date_range_filter(taken: &str, from: Option<&str>, to: Option<&str>) -> (String, Vec<Value>) {
    let parse = |d: Option<&str>| d.and_then(|d| chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").ok());
    let (mut from, mut to) = (parse(from), parse(to));
    if let (Some(f), Some(t)) = (from, to)
        && f > t
    {
        (from, to) = (Some(t), Some(f));
    }
    let mut sql = String::new();
    let mut args = Vec::new();
    // `taken` is "YYYY-MM-DD HH:MM:SS", so plain text comparison orders it by time.
    if let Some(f) = from {
        sql.push_str(&format!(" AND {taken} >= ?"));
        args.push(Value::from(f.format("%Y-%m-%d").to_string()));
    }
    if let Some(t) = to.and_then(|t| t.succ_opt()) {
        sql.push_str(&format!(" AND {taken} < ?"));
        args.push(Value::from(t.format("%Y-%m-%d").to_string()));
    }
    (sql, args)
}

/// SQL condition (starting with " AND") keeping photos, by their `photo_id` column, where
/// the given people appear. `match_mode`: "all" (default) all of them together, "any" at
/// least one, "only" all of them and no other known person. Empty without people.
fn people_filter(photo_id: &str, people: Option<&str>, match_mode: Option<&str>) -> String {
    let people = people.map(parse_ids).unwrap_or_default();
    if people.is_empty() {
        return String::new();
    }
    // ids are parsed integers, so inlining them is safe
    let list = people.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    match match_mode.unwrap_or("all") {
        "any" => format!(" AND {photo_id} IN (SELECT photo_id FROM faces WHERE person_id IN ({list}))"),
        mode => {
            let mut sql = format!(
                " AND {photo_id} IN (SELECT photo_id FROM faces WHERE person_id IN ({list})
                                     GROUP BY photo_id HAVING COUNT(DISTINCT person_id) = {})",
                people.len()
            );
            if mode == "only" {
                sql.push_str(&format!(
                    " AND {photo_id} NOT IN (SELECT photo_id FROM faces f JOIN persons pe ON pe.id = f.person_id
                                             WHERE pe.hidden = 0 AND f.person_id NOT IN ({list}))"
                ));
            }
            sql
        }
    }
}

/// `MM-DD` for today and the following days, and the current year.
fn upcoming_days(days: u32) -> (Vec<String>, i32) {
    let today = Local::now().date_naive();
    let list = (0..days.clamp(1, 366)).map(|d| (today + Duration::days(d as i64)).format("%m-%d").to_string()).collect();
    (list, today.year())
}

/// The filters of a photo query as SQL conditions (each starting with " AND") and their
/// arguments, shared by the photo list and the group summaries so both always agree. Also
/// returns the upcoming days (`MM-DD`) when `upcoming` is set.
fn photo_filters(q: &PhotoQuery) -> (String, Vec<Value>, Vec<String>) {
    let mut sql = String::new();
    let mut args: Vec<Value> = Vec::new();
    match q.place {
        // 0: photos without a location
        Some(0) => sql.push_str(" AND place_id IS NULL"),
        Some(place) => {
            sql.push_str(" AND place_id = ?");
            args.push(place.into());
        }
        None => {}
    }
    if let Some(date) = q.date.as_deref().filter(|d| !d.is_empty()) {
        sql.push_str(" AND taken LIKE ? || '%'");
        args.push(date.to_string().into());
    }
    let (range, range_args) = date_range_filter("taken", q.from.as_deref(), q.to.as_deref());
    sql.push_str(&range);
    args.extend(range_args);
    let mut days = Vec::new();
    if let Some(n) = q.upcoming {
        let (list, year) = upcoming_days(n);
        sql.push_str(&format!(
            " AND substr(taken, 6, 5) IN ({}) AND CAST(substr(taken, 1, 4) AS INTEGER) < ?",
            vec!["?"; list.len()].join(",")
        ));
        args.extend(list.iter().cloned().map(Value::from));
        args.push((year as i64).into());
        days = list;
    }
    sql.push_str(&people_filter("id", q.people.as_deref(), q.match_mode.as_deref()));
    (sql, args, days)
}

async fn photos(State(s): State<Shared>, Query(q): Query<PhotoQuery>) -> ApiResult<Json<JsonValue>> {
    let result = db(&s, move |conn| {
        let (filters, args, days) = photo_filters(&q);
        let desc = q.sort.as_deref() != Some("asc");
        let order = if desc { "taken DESC, id DESC" } else { "taken ASC, id ASC" };
        let sql = format!("SELECT id, width, height, taken, place_id FROM photos WHERE 1 = 1{filters} ORDER BY {order}");
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows: Vec<JsonValue> = stmt
            .query_map(params_from_iter(args), |r| {
                Ok(json!([
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<i64>>(4)?
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
async fn groups(State(s): State<Shared>, Query(q): Query<PhotoQuery>) -> ApiResult<Response> {
    let key = match q.by.as_deref() {
        Some("year") => "substr(taken, 1, 4)",
        Some("month") => "substr(taken, 1, 7)",
        Some("day") => "substr(taken, 1, 10)",
        Some("place") => "COALESCE(place_id, 0)",
        _ => return Ok((StatusCode::BAD_REQUEST, "by must be year, month, day or place").into_response()),
    };
    let by_place = q.by.as_deref() == Some("place");
    let result = db(&s, move |conn| {
        let (filters, args, _) = photo_filters(&q);
        let desc = q.sort.as_deref() != Some("asc");
        // SQLite takes the other columns from the row that has the MAX/MIN: the cover.
        let pick = if desc { "MAX(taken)" } else { "MIN(taken)" };
        let order = if by_place { "n DESC, k" } else if desc { "k DESC" } else { "k ASC" };
        let sql = format!("SELECT {key} AS k, COUNT(*) AS n, id, {pick} FROM photos WHERE 1 = 1{filters} GROUP BY k ORDER BY {order}");
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut total = 0i64;
        let rows: Vec<JsonValue> = stmt
            .query_map(params_from_iter(args), |r| {
                let key: JsonValue = if by_place { r.get::<_, i64>(0)?.into() } else { r.get::<_, String>(0)?.into() };
                Ok((key, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))
            })?
            .map(|row| {
                row.map(|(key, count, cover)| {
                    total += count;
                    json!({ "key": key, "count": count, "cover": cover })
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
async fn check_photo(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Json<JsonValue>> {
    let scan_cfg = s.scan.clone();
    let result = db(&s, move |conn| {
        let Some(path) = conn.query_row("SELECT path FROM photos WHERE id = ?", [id], |r| r.get::<_, String>(0)).optional()? else {
            return Ok(json!({ "status": "removed" }));
        };
        let file = PathBuf::from(&path);
        if file.is_file() {
            return Ok(json!({ "status": "present", "path": path }));
        }
        let root = scan_cfg.roots(conn)?.into_iter().find(|r| file.starts_with(r));
        let folder_there = match &root {
            Some(r) => r.is_dir(),
            None => file.parent().is_some_and(|p| p.is_dir()),
        };
        if !folder_there {
            let folder = root.unwrap_or_else(|| file.parent().map(PathBuf::from).unwrap_or_default());
            return Ok(json!({ "status": "unavailable", "path": path, "folder": folder.to_string_lossy() }));
        }
        crate::db::forget_photo(conn, id)?;
        tracing::info!("{path} is gone: removed from the gallery");
        Ok(json!({ "status": "removed", "path": path }))
    })
    .await?;
    Ok(Json(result))
}

#[derive(Deserialize)]
struct RemoveBody {
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
async fn remove_photo(State(s): State<Shared>, Path(id): Path<i64>, Json(body): Json<RemoveBody>) -> ApiResult<Response> {
    let scan_cfg = s.scan.clone();
    let result = db(&s, move |conn| {
        let Some(path) = conn.query_row("SELECT path FROM photos WHERE id = ?", [id], |r| r.get::<_, String>(0)).optional()? else {
            return Ok((StatusCode::NOT_FOUND, json!({ "error": "no such photo" })));
        };
        match body.from.as_str() {
            "gallery" => {
                conn.execute("INSERT OR IGNORE INTO excluded (path) VALUES (?)", [&path])?;
                crate::db::forget_photo(conn, id)?;
                tracing::info!("{path}: removed from the gallery (file kept)");
                Ok((StatusCode::OK, json!({ "status": "removed", "path": path })))
            }
            "disk" => {
                let file = PathBuf::from(&path);
                if !scan_cfg.roots(conn)?.iter().any(|r| file.starts_with(r)) {
                    return Ok((StatusCode::FORBIDDEN, json!({ "error": "the file is outside the photo folders" })));
                }
                if file.exists() {
                    if body.permanently {
                        std::fs::remove_file(&file)?;
                        tracing::info!("{path}: deleted permanently");
                    } else if let Err(e) = trash::delete(&file) {
                        tracing::warn!("{path}: can't move to the bin: {e}");
                        return Ok((StatusCode::CONFLICT, json!({ "status": "no-bin", "error": e.to_string(), "path": path })));
                    } else {
                        tracing::info!("{path}: moved to the bin");
                    }
                }
                crate::db::forget_photo(conn, id)?;
                Ok((StatusCode::OK, json!({ "status": if body.permanently { "deleted" } else { "binned" }, "path": path })))
            }
            _ => Ok((StatusCode::BAD_REQUEST, json!({ "error": "from must be gallery or disk" }))),
        }
    })
    .await?;
    Ok((result.0, Json(result.1)).into_response())
}

/// Identical files and the search progress. While the scan or the search runs, the report
/// may still be incomplete.
async fn duplicates_report(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
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
        "files": files,
        "bytes": bytes,
        "groups": groups,
    })))
}

async fn duplicates_search(State(s): State<Shared>) -> StatusCode {
    crate::duplicates::spawn(s.scan.db_path.clone(), s.status.dups.clone());
    StatusCode::ACCEPTED
}

#[derive(Deserialize)]
struct DupDelete {
    /// only these photo ids (default: every duplicate copy)
    ids: Option<Vec<i64>>,
    #[serde(default)]
    permanently: bool,
}

/// Moves the duplicate copies to the bin (or deletes them for good when asked), keeping the
/// oldest file of each set.
async fn duplicates_delete(State(s): State<Shared>, Json(body): Json<DupDelete>) -> ApiResult<Json<JsonValue>> {
    let scan_cfg = s.scan.clone();
    let result = db(&s, move |conn| {
        let roots = scan_cfg.roots(conn)?;
        crate::duplicates::delete(conn, &roots, body.ids.as_deref(), body.permanently)
    })
    .await?;
    Ok(Json(serde_json::to_value(result)?))
}

/// Brings back the photos removed from the gallery (they are indexed again).
async fn clear_excluded(State(s): State<Shared>) -> ApiResult<StatusCode> {
    db(&s, |conn| Ok(conn.execute("DELETE FROM excluded", [])?)).await?;
    scan::spawn(s.scan.clone(), s.status.clone());
    Ok(StatusCode::ACCEPTED)
}

async fn photo_detail(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Json<JsonValue>> {
    let result = db(&s, move |conn| {
        let photo = conn
            .query_row(
                "SELECT p.path, p.taken, p.date_from_exif, p.width, p.height, p.lat, p.lon, pl.city, pl.region, pl.country
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
        return Err(anyhow::anyhow!("photo {id} not found").into());
    }
    Ok(Json(result))
}

#[derive(Deserialize)]
struct PlacesQuery {
    /// Only count photos of these people (comma separated ids), as in /api/photos.
    people: Option<String>,
    #[serde(rename = "match")]
    match_mode: Option<String>,
    /// Only count photos taken in this range (YYYY-MM-DD, both days included).
    from: Option<String>,
    to: Option<String>,
}

async fn places(State(s): State<Shared>, Query(q): Query<PlacesQuery>) -> ApiResult<Json<JsonValue>> {
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

async fn people(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let rows = db(&s, |conn| {
        let rows: Vec<JsonValue> = conn
            .prepare_cached(
                "SELECT p.id, p.name, p.hidden, COUNT(DISTINCT f.photo_id) AS n,
                        (SELECT id FROM faces WHERE person_id = p.id ORDER BY score DESC LIMIT 1)
                 FROM persons p JOIN faces f ON f.person_id = p.id
                 GROUP BY p.id ORDER BY p.name IS NULL, n DESC, p.name",
            )?
            .query_map([], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "name": r.get::<_, Option<String>>(1)?,
                    "hidden": r.get::<_, bool>(2)?,
                    "count": r.get::<_, i64>(3)?,
                    "face": r.get::<_, i64>(4)?,
                }))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    })
    .await?;
    Ok(Json(rows.into()))
}

#[derive(Deserialize)]
struct PersonUpdate {
    #[serde(default, deserialize_with = "serde_with_null::deserialize")]
    name: Option<Option<String>>,
    hidden: Option<bool>,
}

/// Distinguishes a missing `name` (keep) from `"name": null` (forget the name).
mod serde_with_null {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
        Ok(Some(Option::<String>::deserialize(d)?))
    }
}

async fn update_person(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<PersonUpdate>,
) -> ApiResult<Response> {
    let saved = db(&s, move |conn| {
        if let Some(hidden) = body.hidden {
            conn.execute("UPDATE persons SET hidden = ? WHERE id = ?", params![hidden, id])?;
        }
        // Names are kept unique; the response says which name was saved.
        body.name.map(|name| crate::db::rename_person(conn, id, name.as_deref())).transpose()
    })
    .await?;
    Ok(match saved {
        Some(name) => Json(json!({ "name": name })).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

#[derive(Deserialize)]
struct Merge {
    into: i64,
}

async fn merge_person(State(s): State<Shared>, Path(id): Path<i64>, Json(body): Json<Merge>) -> ApiResult<StatusCode> {
    if body.into == id {
        return Ok(StatusCode::NO_CONTENT);
    }
    db(&s, move |conn| {
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE persons SET name = COALESCE(name, (SELECT name FROM persons WHERE id = ?1)) WHERE id = ?2",
            params![id, body.into],
        )?;
        tx.execute("UPDATE faces SET person_id = ? WHERE person_id = ?", params![body.into, id])?;
        tx.execute("DELETE FROM persons WHERE id = ?", [id])?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// "Not them": moves the face into a new unnamed person, which can be renamed, hidden or
/// merged later. Clustering never moves it back.
async fn reject_face(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    Ok(match db(&s, move |conn| crate::db::move_face_to_new_person(conn, id)).await? {
        Some(person) => Json(json!({ "person": person })).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    })
}

#[derive(Deserialize)]
struct AssignBody {
    person: i64,
}

/// "Same as" in the photo viewer: moves this one face to another person.
async fn assign_face(State(s): State<Shared>, Path(id): Path<i64>, Json(body): Json<AssignBody>) -> ApiResult<StatusCode> {
    let done = db(&s, move |conn| crate::db::assign_face(conn, id, body.person)).await?;
    Ok(if done { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND })
}

fn jpeg(bytes: Vec<u8>, cache: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, cache)], bytes).into_response()
}

async fn blob(s: &Shared, sql: &'static str, id: i64) -> ApiResult<Response> {
    let data: Option<Vec<u8>> =
        db(s, move |conn| Ok(conn.prepare_cached(sql)?.query_row([id], |r| r.get(0)).optional()?)).await?;
    Ok(match data {
        Some(d) => jpeg(d, IMMUTABLE),
        None => StatusCode::NOT_FOUND.into_response(),
    })
}

async fn thumb(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    blob(&s, "SELECT data FROM thumbs WHERE photo_id = ?", id).await
}

async fn face_thumb(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    blob(&s, "SELECT thumb FROM faces WHERE id = ?", id).await
}

/// Shows the photo's file in the file manager of the computer that has it: through the
/// desktop app, or for a browser on that same computer. Others get 409.
async fn reveal_photo(
    State(s): State<Shared>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    Path(id): Path<i64>,
) -> ApiResult<Response> {
    let path: Option<String> =
        db(&s, move |conn| Ok(conn.query_row("SELECT path FROM photos WHERE id = ?", [id], |r| r.get(0)).optional()?)).await?;
    let Some(path) = path.map(PathBuf::from) else { return Ok(StatusCode::NOT_FOUND.into_response()) };
    if let Some(host) = &s.host {
        host.reveal(&path);
    } else if peer.ip().is_loopback() {
        reveal_in_file_manager(&path)?;
    } else {
        return Ok((StatusCode::CONFLICT, "the photos are on another computer").into_response());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    Command::new("open").arg("-R").arg(path).spawn()?;
    #[cfg(target_os = "windows")]
    Command::new("explorer").arg(format!("/select,{}", path.display())).spawn()?;
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Ask the file manager to select the file; otherwise just open its folder.
        let uri = format!("file://{}", path.to_string_lossy().replace('%', "%25").replace(' ', "%20").replace('#', "%23"));
        let selected = Command::new("dbus-send")
            .args(["--session", "--dest=org.freedesktop.FileManager1", "--type=method_call", "/org/freedesktop/FileManager1",
                   "org.freedesktop.FileManager1.ShowItems", &format!("array:string:{uri}"), "string:"])
            .status()
            .is_ok_and(|s| s.success());
        if !selected {
            Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn()?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct OriginalQuery {
    /// `download=1`: save as a file (Content-Disposition: attachment) instead of showing it
    download: Option<String>,
}

impl OriginalQuery {
    fn download(&self) -> bool {
        self.download.as_deref().is_some_and(|v| !matches!(v, "0" | "false"))
    }
}

/// `attachment; filename=...` with the file's own name (ASCII fallback plus UTF-8).
fn attachment(name: &str) -> String {
    let ascii: String = name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    let encoded: String = name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

async fn original(State(s): State<Shared>, Path(id): Path<i64>, Query(q): Query<OriginalQuery>) -> ApiResult<Response> {
    let path: Option<String> =
        db(&s, move |conn| Ok(conn.query_row("SELECT path FROM photos WHERE id = ?", [id], |r| r.get(0)).optional()?))
            .await?;
    let Some(path) = path.map(PathBuf::from) else { return Ok(StatusCode::NOT_FOUND.into_response()) };
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
        let mut response = ([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "public, max-age=3600")], bytes).into_response();
        if q.download() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("photo-{id}.{ext}"));
            response.headers_mut().insert(header::CONTENT_DISPOSITION, attachment(&name).parse()?);
        }
        return Ok(response);
    }
    // HEIC, TIFF...: convert to a large JPEG on the fly, and save it as the JPEG it is now.
    let jpeg_name = format!("{}.jpg", path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("photo-{id}")));
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
