use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use totufoto::{Config, Gallery};

/// Fast local photo gallery with timeline, places and face grouping.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Folders containing photos (scanned recursively). Without folders, the ones added
    /// from the web UI are used.
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

#[tokio::main]
async fn main() -> Result<()> {
    totufoto::init_logging();
    let args = Args::parse();
    let models = if args.no_faces { None } else { totufoto::enable_faces(&args.models, args.onnxruntime.as_deref()) };
    let gallery = Gallery::open(Config {
        data_dir: args.data,
        folders: args.library,
        models,
        face_threshold: args.face_threshold,
        host: None,
    })?;

    if args.scan_only {
        return tokio::task::spawn_blocking(move || gallery.scan_blocking()).await?;
    }
    gallery.start_scan();
    let listener = tokio::net::TcpListener::bind((args.host.as_str(), args.port)).await?;
    tracing::info!("gallery ready at http://{}", listener.local_addr()?);
    gallery.serve(listener).await
}
