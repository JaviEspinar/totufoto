//! HTTP API and embedded web UI: the router, the connection pool, errors, and the page.

mod folders;
mod people;
mod photos;
mod system;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Result;
use axum::http::{StatusCode, header};
use axum::middleware;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::Connection;
use serde_json::json;
use tower_http::compression::CompressionLayer;

use crate::app::Host;
use crate::scan::{ScanConfig, ScanStatus};

const INDEX_HTML: &str = include_str!("../../web/index.html");

const APP_CSS: &str = include_str!("../../web/app.css");

/// The page's script, split by area, in the order the page loads it: as each file loads it
/// may use what the earlier ones define, but not the later ones.
const SCRIPTS: [(&str, &str); 10] = [
    ("core", include_str!("../../web/js/core.js")),
    ("sidebar", include_str!("../../web/js/sidebar.js")),
    ("photos", include_str!("../../web/js/photos.js")),
    ("optimization", include_str!("../../web/js/optimization.js")),
    ("people", include_str!("../../web/js/people.js")),
    ("views", include_str!("../../web/js/views.js")),
    ("viewer", include_str!("../../web/js/viewer.js")),
    ("settings", include_str!("../../web/js/settings.js")),
    ("status", include_str!("../../web/js/status.js")),
    ("main", include_str!("../../web/js/main.js")),
];

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// The project's pages, linked from Settings. Update it when the repository moves.
const PROJECT_URL: &str = "https://github.com/JaviEspinar/totufoto";

/// The page, with its stylesheet and scripts linked by a hash of their contents
/// (`/js/core.js?v=...`): browsers may keep them for good, and still fetch the new ones
/// after an upgrade, since the hash changes with them.
fn index_html() -> &'static str {
    static PAGE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PAGE.get_or_init(|| {
        let mut hasher = blake3::Hasher::new();
        hasher.update(APP_CSS.as_bytes());
        for (_, script) in SCRIPTS {
            hasher.update(script.as_bytes());
        }
        let hash = hasher.finalize().to_hex();
        let hash = &hash[..12];
        let scripts: String =
            SCRIPTS.iter().map(|(name, _)| format!("<script src=\"/js/{name}.js?v={hash}\"></script>\n")).collect();
        INDEX_HTML.replace("{{scripts}}", &scripts).replace("{{assets}}", hash)
    })
}

async fn script(axum::extract::Path(file): axum::extract::Path<String>) -> Response {
    let name = file.strip_suffix(".js").unwrap_or(&file);
    match SCRIPTS.iter().find(|(n, _)| *n == name) {
        Some((_, body)) => asset("text/javascript; charset=utf-8", body),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn asset(content_type: &'static str, body: &'static str) -> Response {
    ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, IMMUTABLE)], body).into_response()
}

/// Tiny SQLite connection pool: reusing connections keeps their page cache warm, and the
/// number in use at once is limited (a page asks for dozens of thumbnails at a time, and
/// each connection may cache up to 16 MB).
pub struct Pool {
    path: PathBuf,
    idle: Mutex<Vec<Connection>>,
    permits: Arc<tokio::sync::Semaphore>,
}

impl Pool {
    pub fn new(path: PathBuf) -> Self {
        let size = std::thread::available_parallelism().map_or(4, |n| n.get() * 2).max(4);
        Self { path, idle: Mutex::new(Vec::new()), permits: Arc::new(tokio::sync::Semaphore::new(size)) }
    }

    fn with<T>(&self, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
        let conn = self.idle.lock().unwrap_or_else(PoisonError::into_inner).pop();
        let mut conn = match conn {
            Some(c) => c,
            None => {
                let conn = crate::db::open(&self.path)?;
                // Less than the scan's connection: these mostly read small rows and thumbnails.
                conn.execute_batch("PRAGMA cache_size = -16384;")?;
                conn
            }
        };
        let out = f(&mut conn);
        self.idle.lock().unwrap_or_else(PoisonError::into_inner).push(conn);
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
    // Held until the work ends, even if the request that asked for it is gone.
    let permit = state.pool.permits.clone().acquire_owned().await?;
    Ok(tokio::task::spawn_blocking(move || {
        let _permit = permit;
        state.pool.with(f)
    })
    .await??)
}

pub fn router(state: AppState, names: crate::guard::HostNames) -> Router {
    let router = Router::new()
        .route("/", get(|| async { Html(index_html()) }))
        .route("/app.css", get(|| async { asset("text/css; charset=utf-8", APP_CSS) }))
        .route("/js/{file}", get(script))
        .route("/api/status", get(system::status))
        .route("/api/scan", post(system::start_scan))
        .route("/api/regroup", post(system::regroup))
        .route("/api/failures", get(system::failures))
        .route("/api/failures/retry", post(system::retry_failures))
        .route("/api/folders", get(folders::folders).post(folders::add_folder))
        .route("/api/folders/pick", post(folders::pick_folder))
        .route("/api/folders/remove", post(folders::remove_folder))
        .route("/api/folders/browse", get(folders::browse_folders))
        .route("/api/open", post(system::open_url))
        .route("/api/logs/reveal", post(system::reveal_logs))
        .route("/api/photos", get(photos::photos))
        .route("/api/photos/{id}", get(photos::photo_detail))
        .route("/api/photos/{id}/check", post(photos::check_photo))
        .route("/api/photos/{id}/remove", post(photos::remove_photo))
        .route("/api/photos/{id}/reveal", post(photos::reveal_photo))
        .route("/api/photos/{id}/rotate", post(photos::rotate_photo))
        .route("/api/excluded/clear", post(system::clear_excluded))
        .route("/api/duplicates", get(system::duplicates_report))
        .route("/api/duplicates/search", post(system::duplicates_search))
        .route("/api/duplicates/delete", post(system::duplicates_delete))
        .route("/api/duplicates/progress", get(system::duplicates_progress))
        .route("/api/groups", get(photos::groups))
        .route("/api/places", get(photos::places))
        .route("/api/people", get(people::people))
        .route("/api/people/{id}", post(people::update_person))
        .route("/api/people/{id}/merge", post(people::merge_person))
        .route("/api/faces/{id}/reject", post(people::reject_face))
        .route("/api/faces/{id}/cover", post(people::cover_face))
        .route("/api/faces/{id}/assign", post(people::assign_face))
        .route("/thumb/{id}", get(photos::thumb))
        .route("/face/{id}", get(photos::face_thumb))
        .route("/original/{id}", get(photos::original))
        .layer(CompressionLayer::new())
        .with_state(Arc::new(state));
    // On every address: no Host other than this server's, no changes from other sites.
    router.layer(middleware::from_fn_with_state(Arc::new(names), crate::guard::check))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value as JsonValue;

    /// The gallery's router on a test library (a.jpg indexed), without the desktop app.
    fn app(lib: &crate::testutil::Library) -> Router {
        app_with_status(lib).0
    }

    /// The app, and its status to set busy flags on.
    fn app_with_status(lib: &crate::testutil::Library) -> (Router, Arc<ScanStatus>) {
        let status = Arc::new(ScanStatus::default());
        let state = AppState {
            pool: Pool::new(lib.cfg.db_path.clone()),
            status: status.clone(),
            scan: Arc::new(ScanConfig {
                fixed_roots: lib.cfg.fixed_roots.clone(),
                db_path: lib.cfg.db_path.clone(),
                models: None,
                cluster_threshold: 0.42,
            }),
            host: None,
        };
        (router(state, crate::guard::HostNames::default()), status)
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

    /// The field names of a JSON object, sorted.
    fn keys(v: &JsonValue) -> Vec<&str> {
        let mut k: Vec<&str> = v.as_object().expect("an object").keys().map(String::as_str).collect();
        k.sort();
        k
    }

    #[tokio::test]
    async fn responses_keep_the_shape_the_page_reads() {
        let lib = crate::testutil::Library::new();
        lib.add(
            "madrid.jpg",
            crate::testutil::Photo {
                taken: Some("2019:07:04 18:30:00"),
                gps: Some((40.4168, -3.7038)),
                ..Default::default()
            },
        );
        lib.add("plain.jpg", crate::testutil::Photo { color: [1, 2, 3], ..Default::default() });
        std::fs::write(lib.root().join("broken.jpg"), b"not a photo").unwrap();
        lib.scan();
        let madrid = lib.id("madrid.jpg");
        lib.conn()
            .execute_batch(&format!(
                "INSERT INTO persons (id, name) VALUES (1, 'Ana'), (2, NULL);
                 INSERT INTO faces (id, photo_id, x, y, w, h, score, embedding, thumb, person_id) VALUES
                     (1, {madrid}, 0.1, 0.2, 0.3, 0.4, 0.9, x'', x'01', 1), (2, {madrid}, 0.5, 0.5, 0.1, 0.1, 0.8, x'', x'02', 2);"
            ))
            .unwrap();
        let app = app(&lib);
        let get = |uri: String| {
            let app = app.clone();
            async move { call(&app, "GET", &uri, None).await }
        };

        let (_, v) = get("/api/photos".into()).await;
        assert_eq!(keys(&v), ["days", "photos"]);
        let rows = v["photos"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        let row = rows.iter().find(|r| r[0] == madrid).unwrap().as_array().unwrap();
        assert_eq!(row.len(), 6, "[id, width, height, taken, place, version]");
        assert_eq!(
            (&row[1], &row[2], &row[3], &row[5]),
            (&json!(64), &json!(48), &json!("2019-07-04 18:30:00"), &json!(0))
        );
        assert!(row[4].is_i64(), "a place");

        let (_, v) = get("/api/groups?by=month".into()).await;
        assert_eq!(keys(&v), ["groups", "total"]);
        assert_eq!(v["total"], 2);
        assert_eq!(keys(&v["groups"][0]), ["count", "cover", "key", "v"]);
        let (_, v) = get("/api/groups?by=place".into()).await;
        assert!(v["groups"].as_array().unwrap().iter().any(|g| g["key"] == 0), "no location is key 0");

        let (_, v) = get(format!("/api/photos/{madrid}")).await;
        assert_eq!(
            keys(&v),
            [
                "city",
                "country",
                "dateFromExif",
                "faces",
                "height",
                "id",
                "lat",
                "lon",
                "path",
                "region",
                "rotatable",
                "taken",
                "version",
                "width"
            ]
        );
        assert_eq!((&v["city"], &v["dateFromExif"], &v["rotatable"]), (&json!("Madrid"), &json!(true), &json!(true)));
        assert_eq!(keys(&v["faces"][0]), ["box", "id", "name", "person"]);
        assert_eq!((&v["faces"][0]["name"], &v["faces"][0]["box"]), (&json!("Ana"), &json!([0.1, 0.2, 0.3, 0.4])));

        let (_, v) = get("/api/places".into()).await;
        assert_eq!(keys(&v[0]), ["city", "count", "country", "cover", "id", "region"]);
        assert_eq!(v[0]["cover"], madrid);

        let (_, v) = get("/api/people".into()).await;
        assert_eq!(keys(&v[0]), ["count", "face", "hidden", "id", "name"]);
        assert_eq!((&v[0]["name"], &v[1]["name"]), (&json!("Ana"), &JsonValue::Null), "named first");

        let (_, v) = get("/api/status".into()).await;
        for k in ["excluded", "failed", "phase", "running", "total", "done", "faces", "removing", "version", "logs"] {
            assert!(v.get(k).is_some(), "status.{k}");
        }
        assert_eq!(v["failed"], 1);
        let (_, v) = get("/api/failures".into()).await;
        assert_eq!(keys(&v[0]), ["error", "path"]);

        // People updates answer as the page expects.
        let (status, v) = call(&app, "POST", "/api/people/2", Some(json!({ "name": "ana" }))).await;
        assert_eq!((status, &v["name"]), (StatusCode::OK, &json!("ana (1)")));
        let (status, _) = call(&app, "POST", "/api/people/2", Some(json!({ "hidden": true }))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, v) = call(&app, "POST", "/api/faces/2/cover", None).await;
        assert_eq!((status, &v["person"]), (StatusCode::OK, &json!(2)));
        let (status, _) = call(&app, "POST", "/api/people/2/merge", Some(json!({ "into": 1 }))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, v) = get("/api/people".into()).await;
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!((&v[0]["name"], &v[0]["count"]), (&json!("Ana"), &json!(1)));
        assert_eq!(get("/face/1".into()).await.0, StatusCode::OK);
        assert_eq!(get(format!("/thumb/{madrid}")).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn the_page_links_its_stylesheet_and_script_by_content() {
        use tower::ServiceExt;
        let (lib, _) = library_with_a_photo();
        let app = app(&lib);
        let get = |uri: &str| {
            axum::http::Request::builder()
                .uri(uri)
                .header("host", "127.0.0.1:7878")
                .body(axum::body::Body::empty())
                .unwrap()
        };
        let page = app.clone().oneshot(get("/")).await.unwrap();
        let page = String::from_utf8(axum::body::to_bytes(page.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
        assert!(!page.contains("{{assets}}"));
        let scripts: Vec<&str> = page.split("<script src=\"").skip(1).map(|s| s.split('"').next().unwrap()).collect();
        let order: Vec<String> = SCRIPTS.iter().map(|(n, _)| format!("/js/{n}.js")).collect();
        assert_eq!(scripts.iter().map(|s| s.split('?').next().unwrap()).collect::<Vec<_>>(), order, "all, in order");
        let hash = scripts[0].split("?v=").nth(1).unwrap();
        assert_eq!(hash.len(), 12);
        assert!(page.contains(&format!("/app.css?v={hash}")), "the stylesheet has the same hash");
        let mut uris: Vec<(&str, &str)> = scripts.iter().map(|s| (*s, "text/javascript")).collect();
        uris.push(("/app.css", "text/css"));
        for (uri, kind) in uris {
            let res = app.clone().oneshot(get(uri)).await.unwrap();
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            assert!(res.headers()[header::CONTENT_TYPE].to_str().unwrap().starts_with(kind));
        }
        assert_eq!(app.clone().oneshot(get("/js/nothing.js")).await.unwrap().status(), StatusCode::NOT_FOUND);
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
        // Another file, so the folder isn't left empty (which would count as unplugged).
        std::fs::write(lib.root().join("notes.txt"), "").unwrap();
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
    async fn busy_work_refuses_a_second_start_and_frees_itself() {
        use std::sync::atomic::Ordering::SeqCst;
        let (lib, id) = library_with_a_photo();
        let (app, status) = app_with_status(&lib);

        // A deletion of duplicates already running: a second one is refused, and the
        // refusal doesn't clear the running one's flag.
        status.dups.deleting.store(true, SeqCst);
        let (code, body) = call(&app, "POST", "/api/duplicates/delete", Some(json!({}))).await;
        assert_eq!(code, StatusCode::CONFLICT);
        assert!(body["error"].as_str().is_some());
        assert!(status.dups.deleting.load(SeqCst));
        // Once it ends, a new one runs, and frees the flag when done.
        status.dups.deleting.store(false, SeqCst);
        let (code, _) = call(&app, "POST", "/api/duplicates/delete", Some(json!({}))).await;
        assert_eq!(code, StatusCode::OK);
        assert!(!status.dups.deleting.load(SeqCst));

        // Rotating waits for the scan.
        status.running.store(true, SeqCst);
        let (code, _) = call(&app, "POST", &format!("/api/photos/{id}/rotate"), Some(json!({ "turns": 1 }))).await;
        assert_eq!(code, StatusCode::CONFLICT);
        status.running.store(false, SeqCst);

        // One folder removal at a time.
        *status.removing.lock().unwrap() = Some("/elsewhere".into());
        let (code, body) = call(&app, "POST", "/api/folders/remove", Some(json!({ "path": "/saved/folder" }))).await;
        assert_eq!(code, StatusCode::CONFLICT);
        assert!(body["error"].as_str().unwrap().contains("being removed"), "{body}");
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
}
