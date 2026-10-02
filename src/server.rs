//! HTTP API and embedded web UI.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::middleware;
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
use crate::scan::{self, ScanConfig, ScanStatus};
use crate::{imaging, library};

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

/// An error answer: a status code and a message for the person using the gallery, sent as
/// `{"error": "..."}`. Anything that isn't one of the expected cases (a database or file
/// error, say) becomes a 500 and is logged.
#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
    fn not_found(what: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, format!("no such {what}"))
    }
    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }
    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }
    /// Something only the desktop app can do.
    fn desktop_only(what: &str) -> Self {
        Self::new(StatusCode::NOT_IMPLEMENTED, format!("only the desktop app can {what}"))
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(e: E) -> Self {
        let e = e.into();
        tracing::error!("{e:#}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
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

pub fn router(state: AppState, names: crate::guard::HostNames) -> Router {
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
        .route("/api/folders/browse", get(browse_folders))
        .route("/api/open", post(open_url))
        .route("/api/photos", get(photos))
        .route("/api/photos/{id}", get(photo_detail))
        .route("/api/photos/{id}/check", post(check_photo))
        .route("/api/photos/{id}/remove", post(remove_photo))
        .route("/api/photos/{id}/reveal", post(reveal_photo))
        .route("/api/photos/{id}/rotate", post(rotate_photo))
        .route("/api/excluded/clear", post(clear_excluded))
        .route("/api/duplicates", get(duplicates_report))
        .route("/api/duplicates/search", post(duplicates_search))
        .route("/api/duplicates/delete", post(duplicates_delete))
        .route("/api/duplicates/progress", get(duplicates_progress))
        .route("/api/groups", get(groups))
        .route("/api/places", get(places))
        .route("/api/people", get(people))
        .route("/api/people/{id}", post(update_person))
        .route("/api/people/{id}/merge", post(merge_person))
        .route("/api/faces/{id}/reject", post(reject_face))
        .route("/api/faces/{id}/cover", post(cover_face))
        .route("/api/faces/{id}/assign", post(assign_face))
        .route("/thumb/{id}", get(thumb))
        .route("/face/{id}", get(face_thumb))
        .route("/original/{id}", get(original))
        .layer(CompressionLayer::new())
        .with_state(Arc::new(state));
    // On every address: no Host other than this server's, no changes from other sites.
    router.layer(middleware::from_fn_with_state(Arc::new(names), crate::guard::check))
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

async fn folders(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let scan_cfg = s.scan.clone();
    let desktop = s.host.is_some();
    let list = db(&s, move |conn| {
        let mut out = Vec::new();
        for root in scan_cfg.roots(conn)? {
            let photos = library::count_under(conn, &root)?;
            let fixed = scan_cfg.is_fixed(&root);
            out.push(
                json!({ "path": root.to_string_lossy(), "available": root.is_dir(), "photos": photos, "fixed": fixed }),
            );
        }
        Ok(out)
    })
    .await?;
    Ok(Json(json!({ "folders": list, "desktop": desktop })))
}

#[derive(Deserialize)]
struct FolderBody {
    path: String,
}

/// Saves a folder and indexes it.
async fn save_folder(s: &Shared, path: PathBuf) -> ApiResult<Response> {
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

async fn add_folder(State(s): State<Shared>, Json(body): Json<FolderBody>) -> ApiResult<Response> {
    save_folder(&s, PathBuf::from(body.path.trim())).await
}

/// Opens the native folder picker (desktop app only).
async fn pick_folder(State(s): State<Shared>) -> ApiResult<Response> {
    let Some(host) = s.host.clone() else { return Err(ApiError::desktop_only("open a folder picker")) };
    match tokio::task::spawn_blocking(move || host.pick_folder()).await? {
        Some(path) => save_folder(&s, path).await,
        None => Ok(StatusCode::NO_CONTENT.into_response()),
    }
}

/// Ends a folder removal however it goes: scans may run again (and one picks up the
/// remaining folders).
struct RemovalGuard {
    s: Shared,
}

impl Drop for RemovalGuard {
    fn drop(&mut self) {
        *self.s.status.removing.lock().unwrap() = None;
        self.s.status.hold.store(false, std::sync::atomic::Ordering::SeqCst);
        scan::spawn(self.s.scan.clone(), self.s.status.clone());
    }
}

async fn remove_folder(State(s): State<Shared>, Json(body): Json<FolderBody>) -> ApiResult<Response> {
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

async fn remove_folder_now(s: Shared, path: String) -> ApiResult<(usize, usize)> {
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
        tracing::info!("removed folder {path} ({} photos, {} kept: in another folder)", r.removed, r.kept);
        Ok((r.removed, r.kept))
    })
    .await
}

#[derive(Deserialize)]
struct BrowseQuery {
    path: Option<String>,
}

/// The folders inside `path` on the computer running Totufoto, for choosing photo folders
/// from a browser. Without a path: next to the first photo folder, or the home folder.
async fn browse_folders(State(s): State<Shared>, Query(q): Query<BrowseQuery>) -> ApiResult<Response> {
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

fn list_dirs(path: Option<PathBuf>) -> Result<JsonValue> {
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

#[derive(Deserialize)]
struct OpenBody {
    url: String,
}

/// Opens a map link in the system browser (desktop app only). Limited to the map site
/// the UI links to, so the endpoint can't be used to launch arbitrary URLs.
async fn open_url(State(s): State<Shared>, Json(body): Json<OpenBody>) -> ApiResult<StatusCode> {
    match &s.host {
        Some(host) if body.url.starts_with("https://www.openstreetmap.org/") => {
            host.open_url(&body.url);
            Ok(StatusCode::NO_CONTENT)
        }
        Some(_) => Err(ApiError::forbidden("only map links can be opened")),
        None => Err(ApiError::desktop_only("open links")),
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
    let list =
        (0..days.clamp(1, 366)).map(|d| (today + Duration::days(d as i64)).format("%m-%d").to_string()).collect();
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
async fn groups(State(s): State<Shared>, Query(q): Query<PhotoQuery>) -> ApiResult<Response> {
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
async fn check_photo(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Json<JsonValue>> {
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
async fn remove_photo(
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
        "deleting": dups.deleting.load(Relaxed),
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
async fn duplicates_delete(State(s): State<Shared>, Json(body): Json<DupDelete>) -> ApiResult<Response> {
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
async fn duplicates_progress(State(s): State<Shared>) -> Json<JsonValue> {
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
async fn clear_excluded(State(s): State<Shared>) -> ApiResult<StatusCode> {
    db(&s, |conn| Ok(conn.execute("DELETE FROM excluded", [])?)).await?;
    scan::spawn(s.scan.clone(), s.status.clone());
    Ok(StatusCode::ACCEPTED)
}

async fn photo_detail(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Json<JsonValue>> {
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
                        COALESCE((SELECT id FROM faces WHERE id = p.cover_face AND person_id = p.id),
                                 (SELECT id FROM faces WHERE person_id = p.id ORDER BY score DESC LIMIT 1))
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
    match db(&s, move |conn| crate::db::move_face_to_new_person(conn, id)).await? {
        Some(person) => Ok(Json(json!({ "person": person })).into_response()),
        None => Err(ApiError::not_found("face")),
    }
}

/// Shows this face on its person's card. It stays the card's face while it belongs to them.
async fn cover_face(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    let person: Option<i64> = db(&s, move |conn| {
        let person: Option<i64> =
            conn.query_row("SELECT person_id FROM faces WHERE id = ?", [id], |r| r.get(0)).optional()?.flatten();
        if let Some(person) = person {
            conn.execute("UPDATE persons SET cover_face = ? WHERE id = ?", [id, person])?;
        }
        Ok(person)
    })
    .await?;
    match person {
        Some(person) => Ok(Json(json!({ "person": person })).into_response()),
        None => Err(ApiError::not_found("face in a group")),
    }
}

#[derive(Deserialize)]
struct AssignBody {
    person: i64,
}

/// "Same as" in the photo viewer: moves this one face to another person.
async fn assign_face(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<AssignBody>,
) -> ApiResult<StatusCode> {
    let done = db(&s, move |conn| crate::db::assign_face(conn, id, body.person)).await?;
    if done { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::not_found("face or person")) }
}

fn jpeg(bytes: Vec<u8>, cache: &'static str) -> Response {
    ([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, cache)], bytes).into_response()
}

async fn blob(s: &Shared, sql: &'static str, id: i64) -> ApiResult<Response> {
    let data: Option<Vec<u8>> =
        db(s, move |conn| Ok(conn.prepare_cached(sql)?.query_row([id], |r| r.get(0)).optional()?)).await?;
    data.map(|d| jpeg(d, IMMUTABLE)).ok_or_else(|| ApiError::not_found("picture"))
}

async fn thumb(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    blob(&s, "SELECT data FROM thumbs WHERE photo_id = ?", id).await
}

async fn face_thumb(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    blob(&s, "SELECT thumb FROM faces WHERE id = ?", id).await
}

#[derive(Deserialize)]
struct RotateBody {
    /// quarter turns clockwise (negative: counterclockwise)
    turns: i32,
}

/// Turns the photo in its file (JPEG: its EXIF orientation, so nothing is recompressed;
/// PNG: rewritten turned) and in the gallery.
async fn rotate_photo(
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
async fn reveal_photo(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
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

async fn original(State(s): State<Shared>, Path(id): Path<i64>, Query(q): Query<OriginalQuery>) -> ApiResult<Response> {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Six photos and four people (Carl hidden):
    /// 1 Ana + Ben, 2 Ana, 3 Ben, 4 Ana + Carl, 5 nobody, 6 Ana + Dan.
    fn library() -> Connection {
        let conn = crate::db::open_in_memory();
        conn.execute_batch(
            "INSERT INTO persons (id, name, hidden) VALUES (1, 'Ana', 0), (2, 'Ben', 0), (3, 'Carl', 1), (4, NULL, 0);
             INSERT INTO photos (id, path, mtime, size, width, height, taken, date_from_exif) VALUES
                 (1, '/p/1.jpg', 0, 1, 1, 1, '2020-03-10 12:00:00', 1),
                 (2, '/p/2.jpg', 0, 1, 1, 1, '2020-03-11 00:00:00', 1),
                 (3, '/p/3.jpg', 0, 1, 1, 1, '2021-05-01 08:00:00', 1),
                 (4, '/p/4.jpg', 0, 1, 1, 1, '2021-05-02 23:59:59', 1),
                 (5, '/p/5.jpg', 0, 1, 1, 1, '2022-01-01 10:00:00', 1),
                 (6, '/p/6.jpg', 0, 1, 1, 1, '2022-06-01 10:00:00', 1);",
        )
        .unwrap();
        for (photo, person) in [(1, 1), (1, 2), (2, 1), (3, 2), (4, 1), (4, 3), (6, 1), (6, 4)] {
            conn.execute(
                "INSERT INTO faces (photo_id, x, y, w, h, score, embedding, thumb, person_id) VALUES (?, 0, 0, 1, 1, 1, x'', x'', ?)",
                [photo, person],
            )
            .unwrap();
        }
        conn
    }

    /// The photos a query (as the UI sends it) keeps.
    fn ids(conn: &Connection, query: JsonValue) -> Vec<i64> {
        let q: PhotoQuery = serde_json::from_value(query).unwrap();
        let (filters, args, _) = photo_filters(&q);
        conn.prepare(&format!("SELECT id FROM photos WHERE 1 = 1{filters} ORDER BY id"))
            .unwrap()
            .query_map(params_from_iter(args), |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// The gallery's router on a test library (a.jpg indexed), without the desktop app.
    fn app(lib: &crate::testutil::Library) -> Router {
        let state = AppState {
            pool: Pool::new(lib.cfg.db_path.clone()),
            status: Arc::new(ScanStatus::default()),
            scan: Arc::new(ScanConfig {
                fixed_roots: lib.cfg.fixed_roots.clone(),
                db_path: lib.cfg.db_path.clone(),
                models: None,
                cluster_threshold: 0.42,
            }),
            host: None,
        };
        router(state, crate::guard::HostNames::default())
    }

    /// Sends a request as the gallery's own page would, and returns the status and the body.
    async fn call(app: &Router, method: &str, uri: &str, body: Option<JsonValue>) -> (StatusCode, JsonValue) {
        use tower::ServiceExt;
        let mut req = axum::http::Request::builder().method(method).uri(uri).header("host", "127.0.0.1:7878");
        if method != "GET" {
            req = req.header("sec-fetch-site", "same-origin");
        }
        let req = match body {
            Some(b) => req.header("content-type", "application/json").body(axum::body::Body::from(b.to_string())),
            None => req.body(axum::body::Body::empty()),
        }
        .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(JsonValue::Null))
    }

    fn library_with_a_photo() -> (crate::testutil::Library, i64) {
        let lib = crate::testutil::Library::new();
        lib.add("a.jpg", crate::testutil::Photo::default());
        lib.scan();
        let id = lib.id("a.jpg");
        (lib, id)
    }

    #[tokio::test]
    async fn errors_have_the_right_status_and_a_message() {
        let (lib, id) = library_with_a_photo();
        let app = app(&lib);
        let error =
            |(status, body): (StatusCode, JsonValue)| (status, body["error"].as_str().unwrap_or("").to_string());

        let (status, body) = call(&app, "GET", &format!("/api/photos/{id}"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["width"], 64);
        assert_eq!(
            error(call(&app, "GET", "/api/photos/9999", None).await),
            (StatusCode::NOT_FOUND, "no such photo".into())
        );
        assert_eq!(call(&app, "GET", "/thumb/9999", None).await.0, StatusCode::NOT_FOUND);
        assert_eq!(call(&app, "GET", "/api/groups", None).await.0, StatusCode::BAD_REQUEST);
        let (status, msg) =
            error(call(&app, "POST", &format!("/api/photos/{id}/rotate"), Some(json!({ "turns": 1 }))).await);
        assert_eq!(status, StatusCode::OK, "{msg}");
        let (status, msg) = error(call(&app, "POST", "/api/folders/pick", None).await);
        assert_eq!(
            (status, msg.as_str()),
            (StatusCode::NOT_IMPLEMENTED, "only the desktop app can open a folder picker")
        );
        let (status, _) = call(&app, "POST", "/api/folders", Some(json!({ "path": "/nonexistent/photos" }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, msg) = error(call(&app, "POST", "/api/folders", Some(json!({ "path": lib.root() }))).await);
        assert_eq!((status, msg.contains("already in the gallery")), (StatusCode::CONFLICT, true), "{msg}");
    }

    #[tokio::test]
    async fn a_photo_whose_file_is_gone_is_404_then_removed_by_check() {
        let (lib, id) = library_with_a_photo();
        let app = app(&lib);
        std::fs::remove_file(lib.root().join("a.jpg")).unwrap();
        assert_eq!(call(&app, "GET", &format!("/original/{id}"), None).await.0, StatusCode::NOT_FOUND);
        let (_, body) = call(&app, "POST", &format!("/api/photos/{id}/check"), None).await;
        assert_eq!(body["status"], "removed");
        assert_eq!(call(&app, "GET", &format!("/api/photos/{id}"), None).await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn removing_from_the_gallery_keeps_the_file_and_disk_stays_inside_the_folders() {
        let (lib, id) = library_with_a_photo();
        let app = app(&lib);
        let (status, body) =
            call(&app, "POST", &format!("/api/photos/{id}/remove"), Some(json!({ "from": "nowhere" }))).await;
        assert_eq!((status, body["error"].as_str()), (StatusCode::BAD_REQUEST, Some("from must be gallery or disk")));
        let (status, body) =
            call(&app, "POST", &format!("/api/photos/{id}/remove"), Some(json!({ "from": "gallery" }))).await;
        assert_eq!((status, body["status"].as_str()), (StatusCode::OK, Some("removed")));
        assert!(lib.root().join("a.jpg").exists());
        assert_eq!(lib.scan().total, 0, "it stays out of the gallery");

        // A photo outside the folders can't be deleted from disk.
        let outside = lib.dir.file("elsewhere/b.jpg");
        crate::testutil::Photo::default().write(&outside);
        let conn = lib.conn();
        conn.execute(
            "INSERT INTO photos (path, mtime, size, width, height, taken, date_from_exif) VALUES (?, 0, 1, 1, 1, '2020-01-01 00:00:00', 0)",
            [outside.to_string_lossy()],
        )
        .unwrap();
        let b = conn.last_insert_rowid();
        let (status, _) = call(
            &app,
            "POST",
            &format!("/api/photos/{b}/remove"),
            Some(json!({ "from": "disk", "permanently": true })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(outside.exists());
    }

    #[tokio::test]
    async fn the_guard_applies_to_every_route() {
        use tower::ServiceExt;
        let (lib, _) = library_with_a_photo();
        let app = app(&lib);
        let req = |host: &str, site: Option<&str>| {
            let mut b = axum::http::Request::builder().method("POST").uri("/api/scan").header("host", host);
            if let Some(site) = site {
                b = b.header("sec-fetch-site", site);
            }
            b.body(axum::body::Body::empty()).unwrap()
        };
        let status =
            |r: axum::http::Request<axum::body::Body>| async { app.clone().oneshot(r).await.unwrap().status() };
        assert_eq!(status(req("evil.example:7878", Some("same-origin"))).await, StatusCode::FORBIDDEN);
        assert_eq!(status(req("127.0.0.1:7878", Some("cross-site"))).await, StatusCode::FORBIDDEN);
        assert_eq!(status(req("127.0.0.1:7878", Some("same-origin"))).await, StatusCode::ACCEPTED);
    }

    #[test]
    fn people_together_any_or_only() {
        let conn = library();
        assert_eq!(ids(&conn, json!({ "people": "1,2" })), [1], "together is the default");
        assert_eq!(ids(&conn, json!({ "people": "1,2", "match": "all" })), [1]);
        assert_eq!(ids(&conn, json!({ "people": "1,2", "match": "any" })), [1, 2, 3, 4, 6]);
        // Only Ana: hidden Carl doesn't count, unnamed person 4 does.
        assert_eq!(ids(&conn, json!({ "people": "1", "match": "only" })), [2, 4]);
        assert_eq!(ids(&conn, json!({ "people": "1,2", "match": "only" })), [1]);
        assert_eq!(ids(&conn, json!({ "people": "1, x,,2" })), [1], "junk in the list is ignored");
        assert_eq!(ids(&conn, json!({ "people": "" })), [1, 2, 3, 4, 5, 6], "no people, no filter");
    }

    #[test]
    fn date_ranges_include_both_days() {
        let conn = library();
        assert_eq!(ids(&conn, json!({ "from": "2020-03-11", "to": "2021-05-02" })), [2, 3, 4]);
        assert_eq!(ids(&conn, json!({ "from": "2021-05-02", "to": "2020-03-11" })), [2, 3, 4], "reversed");
        assert_eq!(ids(&conn, json!({ "from": "2021-05-01" })), [3, 4, 5, 6]);
        assert_eq!(ids(&conn, json!({ "to": "2020-03-10" })), [1]);
        assert_eq!(ids(&conn, json!({ "from": "2021-13-40", "to": "2020-03-10" })), [1], "an invalid day is ignored");
        assert_eq!(ids(&conn, json!({ "date": "2021" })), [3, 4], "a year");
        assert_eq!(ids(&conn, json!({ "date": "2020-03-10" })), [1], "a day");
        assert_eq!(ids(&conn, json!({ "people": "1", "date": "2021" })), [4], "filters combine");
    }

    #[test]
    fn upcoming_is_past_years_only() {
        let conn = library();
        let today = Local::now().date_naive();
        let on = |years_ago: i32, days_ahead: i64| {
            let day = today + Duration::days(days_ahead);
            format!("{}-{} 12:00:00", today.year() - years_ago, day.format("%m-%d"))
        };
        conn.execute_batch("DELETE FROM photos").unwrap();
        for (id, taken) in [(10, on(3, 0)), (11, on(1, 5)), (12, on(0, 0)), (13, on(2, 40))] {
            conn.execute(
                "INSERT INTO photos (id, path, mtime, size, width, height, taken, date_from_exif) VALUES (?, ?, 0, 1, 1, 1, ?, 1)",
                params![id, format!("/p/{id}.jpg"), taken],
            )
            .unwrap();
        }
        // Today three years ago and in five days a year ago; not this year's, not in 40 days.
        assert_eq!(ids(&conn, json!({ "upcoming": 30 })), [10, 11]);
        assert_eq!(ids(&conn, json!({ "upcoming": 1 })), [10]);
        assert_eq!(upcoming_days(0).0.len(), 1, "at least today");
        assert_eq!(upcoming_days(10_000).0.len(), 366, "at most a year");
    }
}
