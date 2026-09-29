//! Startup shared by the command-line and desktop apps.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};

use crate::faces::{self, ModelPaths};
use crate::scan::{self, ScanConfig, ScanStatus};
use crate::{db, server};

/// Native services offered by the program embedding the gallery.
pub trait Host: Send + Sync + 'static {
    /// Shows a native folder picker; `None` when cancelled.
    fn pick_folder(&self) -> Option<PathBuf>;
    /// Opens a web page in the system browser.
    fn open_url(&self, url: &str);
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
    pub host: Option<Arc<dyn Host>>,
}

pub struct Gallery {
    scan: Arc<ScanConfig>,
    status: Arc<ScanStatus>,
    host: Option<Arc<dyn Host>>,
}

impl Gallery {
    pub fn open(cfg: Config) -> Result<Self> {
        std::fs::create_dir_all(&cfg.data_dir).with_context(|| format!("creating {}", cfg.data_dir.display()))?;
        let db_path = cfg.data_dir.join("index.sqlite");
        db::open(&db_path)?;
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
        let addr = listener.local_addr()?;
        let app = server::router(
            server::AppState { pool: server::Pool::new(self.scan.db_path.clone()), status: self.status, scan: self.scan, host: self.host },
            allowed_hosts(addr),
        );
        axum::serve(listener, app).await?;
        Ok(())
    }
}

/// On loopback, only accept requests addressed to this server, so web pages can't reach
/// the API through DNS rebinding. Other interfaces are an explicit choice to share it.
fn allowed_hosts(addr: SocketAddr) -> Option<Vec<String>> {
    addr.ip().is_loopback().then(|| {
        let port = addr.port();
        vec![format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")]
    })
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
        .unwrap_or_else(|_| "totufoto=info,totufoto_desktop=info,ort=error,warn".into());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}
