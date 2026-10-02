//! Startup shared by the command-line and desktop apps.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::faces::{self, ModelPaths};
use crate::scan::{self, ScanConfig, ScanStatus};
use crate::{db, guard, server};

/// Native services offered by the program embedding the gallery.
pub trait Host: Send + Sync + 'static {
    /// Shows a native folder picker; `None` when cancelled.
    fn pick_folder(&self) -> Option<PathBuf>;
    /// Opens a web page in the system browser.
    fn open_url(&self, url: &str);
    /// Shows a file in the system's file manager, selected.
    fn reveal(&self, path: &Path);
}

pub struct Config {
    /// Where the index database lives.
    pub data_dir: PathBuf,
    /// Folders to index. When empty, the folders added from the UI (saved in the index) are used.
    pub folders: Vec<PathBuf>,
    /// Face models, if face recognition is available (see [`enable_faces`]).
    pub models: Option<ModelPaths>,
    /// Cosine similarity needed to consider two faces the same person.
    pub face_threshold: f32,
    /// The desktop app's window services; `None` for the command-line app.
    pub host: Option<Arc<dyn Host>>,
    /// Host names the server answers to besides IP addresses, `localhost` and this
    /// computer's own names (`--allow-host`; see `guard`).
    pub allowed_names: Vec<String>,
}

pub struct Gallery {
    scan: Arc<ScanConfig>,
    status: Arc<ScanStatus>,
    host: Option<Arc<dyn Host>>,
    allowed_names: Vec<String>,
}

impl Gallery {
    pub fn open(cfg: Config) -> Result<Self> {
        std::fs::create_dir_all(&cfg.data_dir).with_context(|| format!("creating {}", cfg.data_dir.display()))?;
        let db_path = cfg.data_dir.join("index.sqlite");
        let moved = db::give_moved_faces_a_person(&mut db::open(&db_path)?)?;
        if moved > 0 {
            let faces = if moved == 1 { "face" } else { "faces" };
            tracing::info!("gave {moved} {faces} marked \"Not them\" a group of their own");
        }
        let folders = cfg
            .folders
            .iter()
            .map(|p| dunce::canonicalize(p).with_context(|| format!("photo folder {}", p.display())))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            scan: Arc::new(ScanConfig {
                fixed_roots: folders,
                db_path,
                models: cfg.models,
                cluster_threshold: cfg.face_threshold,
            }),
            status: Arc::new(ScanStatus::default()),
            host: cfg.host,
            allowed_names: cfg.allowed_names,
        })
    }

    /// Indexes the folders on the current thread.
    pub fn scan_blocking(&self) -> Result<()> {
        scan::run(&self.scan, &self.status)
    }

    /// Indexes the folders in the background.
    pub fn start_scan(&self) {
        scan::spawn(self.scan.clone(), self.status.clone());
    }

    /// Progress of the current or last scan.
    pub fn status(&self) -> scan::StatusView {
        self.status.view()
    }

    /// Serves the UI and API until the listener fails.
    pub async fn serve(self, listener: tokio::net::TcpListener) -> Result<()> {
        let names = guard::HostNames::with_own_names(&self.allowed_names);
        if !listener.local_addr()?.ip().is_loopback() {
            tracing::info!("answering to IP addresses, localhost and {:?}", names.names());
        }
        let app = server::router(
            server::AppState {
                pool: server::Pool::new(self.scan.db_path.clone()),
                status: self.status,
                scan: self.scan,
                host: self.host,
            },
            names,
        );
        axum::serve(listener, app).await?;
        Ok(())
    }
}

/// Loads ONNX Runtime and checks the face models. Face recognition is optional: when
/// something is missing the gallery still works, just without people.
pub fn enable_faces(models_dir: &Path, onnxruntime: Option<&Path>) -> Option<ModelPaths> {
    let paths = ModelPaths::in_dir(models_dir);
    if !paths.exist() {
        tracing::warn!("face models not found in {}; faces will be skipped", models_dir.display());
        return None;
    }
    tracing::info!("CPU: {}", faces::cpu_features());
    let loaded = faces::init_runtime(onnxruntime).and_then(|lib| faces::FaceModels::load(&paths).map(|_| lib));
    match loaded {
        Ok(lib) => {
            tracing::info!("face recognition enabled (ONNX Runtime: {})", lib.display());
            Some(paths)
        }
        Err(e) => {
            tracing::warn!("face recognition disabled: {e:#}");
            None
        }
    }
}

pub fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "imadive=info,imadive_desktop=info,ort=error,warn".into());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
