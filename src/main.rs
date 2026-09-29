mod cluster;
mod db;
mod faces;
mod geo;
mod imaging;
mod scan;
mod server;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;

use crate::faces::ModelPaths;
use crate::scan::{ScanConfig, ScanStatus};

/// Fast local photo gallery with timeline, places and face grouping.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Folders containing photos (scanned recursively)
    #[arg(required = true)]
    library: Vec<PathBuf>,
    /// Where the index database lives
    #[arg(long, default_value = "totufoto-data")]
    data: PathBuf,
    /// Folder with det_500m.onnx and w600k_mbf.onnx
    #[arg(long, default_value = "models")]
    models: PathBuf,
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 7878)]
    port: u16,
    /// Cosine similarity needed to consider two faces the same person (higher = stricter)
    #[arg(long, default_value_t = 0.42)]
    face_threshold: f32,
    /// ONNX Runtime library (file or folder); by default looked up next to the
    /// executable and in ./onnxruntime (see scripts/fetch-onnxruntime.sh)
    #[arg(long)]
    onnxruntime: Option<PathBuf>,
    /// Skip face detection
    #[arg(long)]
    no_faces: bool,
    /// Index the library and exit, without starting the web server
    #[arg(long)]
    scan_only: bool,
}

/// Face recognition is optional: when the models or ONNX Runtime can't be loaded the
/// gallery still works, just without people.
fn face_models(args: &Args) -> Option<ModelPaths> {
    let paths = ModelPaths::in_dir(&args.models);
    if !paths.exist() {
        tracing::warn!(
            "face models not found in {}; run scripts/fetch-models.sh (faces will be skipped)",
            args.models.display()
        );
        return None;
    }
    tracing::info!("CPU: {}", faces::cpu_features());
    let loaded = faces::init_runtime(args.onnxruntime.as_deref())
        .and_then(|lib| faces::FaceModels::load(&paths).map(|_| lib));
    match loaded {
        Ok(lib) => {
            tracing::info!("face recognition enabled (ONNX Runtime: {})", lib.display());
            Some(paths)
        }
        Err(e) => {
            tracing::warn!("face recognition disabled: {e:#}. Run scripts/fetch-onnxruntime.sh to install ONNX Runtime");
            None
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "totufoto=info,ort=error,warn".into()),
        )
        .init();
    let args = Args::parse();

    std::fs::create_dir_all(&args.data).with_context(|| format!("creating {}", args.data.display()))?;
    let db_path = args.data.join("index.sqlite");
    db::open(&db_path)?;

    let models = if args.no_faces { None } else { face_models(&args) };
    let library = args
        .library
        .iter()
        .map(|p| p.canonicalize().with_context(|| format!("library folder {}", p.display())))
        .collect::<Result<Vec<_>>>()?;

    let scan = Arc::new(ScanConfig {
        roots: library,
        db_path: db_path.clone(),
        models,
        cluster_threshold: args.face_threshold,
    });
    let status = Arc::new(ScanStatus::default());

    if args.scan_only {
        return tokio::task::spawn_blocking(move || scan::run(&scan, &status)).await?;
    }

    server::spawn_scan(scan.clone(), status.clone());
    let app = server::router(server::AppState { pool: server::Pool::new(db_path), status, scan });
    let listener = tokio::net::TcpListener::bind((args.host.as_str(), args.port)).await?;
    tracing::info!("gallery ready at http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
