//! HTTP API and embedded web UI.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::extract::{Path, Query, Request, State};
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
        .route("/api/folders", get(folders).post(add_folder))
        .route("/api/folders/pick", post(pick_folder))
        .route("/api/folders/remove", post(remove_folder))
        .route("/api/open", post(open_url))
        .route("/api/photos", get(photos))
        .route("/api/photos/{id}", get(photo_detail))
        .route("/api/places", get(places))
        .route("/api/people", get(people))
        .route("/api/people/{id}", post(update_person))
        .route("/api/people/{id}/merge", post(merge_person))
        .route("/api/faces/{id}/reject", post(reject_face))
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

async fn status(State(s): State<Shared>) -> Json<scan::StatusView> {
    Json(s.status.view())
}

async fn start_scan(State(s): State<Shared>) -> StatusCode {
    scan::spawn(s.scan.clone(), s.status.clone());
    StatusCode::ACCEPTED
}

async fn folders(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let scan_cfg = s.scan.clone();
    let managed = s.scan.fixed_roots.is_empty();
    let desktop = s.host.is_some();
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
    Ok(Json(json!({ "folders": list, "managed": managed, "desktop": desktop })))
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
    /// photos from previous years whose anniversary falls within the next `upcoming` days
    upcoming: Option<u32>,
}

fn parse_ids(s: &str) -> Vec<i64> {
    s.split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

/// `MM-DD` for today and the following days, and the current year.
fn upcoming_days(days: u32) -> (Vec<String>, i32) {
    let today = Local::now().date_naive();
    let list = (0..days.clamp(1, 366)).map(|d| (today + Duration::days(d as i64)).format("%m-%d").to_string()).collect();
    (list, today.year())
}

async fn photos(State(s): State<Shared>, Query(q): Query<PhotoQuery>) -> ApiResult<Json<JsonValue>> {
    let result = db(&s, move |conn| {
        let mut sql = String::from("SELECT id, width, height, taken, place_id FROM photos WHERE 1 = 1");
        let mut args: Vec<Value> = Vec::new();
        if let Some(place) = q.place {
            sql.push_str(" AND place_id = ?");
            args.push(place.into());
        }
        if let Some(date) = q.date.filter(|d| !d.is_empty()) {
            sql.push_str(" AND taken LIKE ? || '%'");
            args.push(date.into());
        }
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
        let people = q.people.as_deref().map(parse_ids).unwrap_or_default();
        if !people.is_empty() {
            // ids are parsed integers, so inlining them is safe
            let list = people.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            match q.match_mode.as_deref().unwrap_or("all") {
                "any" => sql.push_str(&format!(" AND id IN (SELECT photo_id FROM faces WHERE person_id IN ({list}))")),
                mode => {
                    sql.push_str(&format!(
                        " AND id IN (SELECT photo_id FROM faces WHERE person_id IN ({list})
                                     GROUP BY photo_id HAVING COUNT(DISTINCT person_id) = {})",
                        people.len()
                    ));
                    if mode == "only" {
                        sql.push_str(&format!(
                            " AND id NOT IN (SELECT photo_id FROM faces f JOIN persons p ON p.id = f.person_id
                                             WHERE p.hidden = 0 AND f.person_id NOT IN ({list}))"
                        ));
                    }
                }
            }
        }
        let desc = q.sort.as_deref() != Some("asc");
        sql.push_str(if desc { " ORDER BY taken DESC, id DESC" } else { " ORDER BY taken ASC, id ASC" });

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

async fn places(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let rows = db(&s, |conn| {
        let rows: Vec<JsonValue> = conn
            .prepare_cached(
                "SELECT pl.id, pl.city, pl.region, pl.country, COUNT(*) AS n, MAX(p.id)
                 FROM photos p JOIN places pl ON pl.id = p.place_id
                 GROUP BY pl.id ORDER BY n DESC, pl.city",
            )?
            .query_map([], |r| {
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
) -> ApiResult<StatusCode> {
    db(&s, move |conn| {
        if let Some(name) = body.name {
            let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
            conn.execute("UPDATE persons SET name = ? WHERE id = ?", params![name, id])?;
        }
        if let Some(hidden) = body.hidden {
            conn.execute("UPDATE persons SET hidden = ? WHERE id = ?", params![hidden, id])?;
        }
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
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

async fn original(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
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
        return Ok(([(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, "public, max-age=3600")], bytes).into_response());
    }
    // HEIC, TIFF...: convert to a large JPEG on the fly.
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let raw = std::fs::read(&path)?;
        let img = imaging::decode(&path, &raw)?.into_rgb8();
        imaging::encode_jpeg(&imaging::fit(&img, 2560)?, 88)
    })
    .await??;
    Ok(jpeg(bytes, "public, max-age=3600"))
}
