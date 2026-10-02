use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use imadive::{Config, Gallery};

/// Fast local photo gallery with timeline, places and face grouping.
#[derive(Parser)]
#[command(version)]
struct Args {
    /// Folders containing photos (scanned recursively). Without folders, the ones added
    /// from the web UI are used.
    library: Vec<PathBuf>,
    /// Where the index database lives [default: imadive-data, or totufoto-data when only
    /// that one exists (Imadive was called Totufoto)]
    #[arg(long)]
    data: Option<PathBuf>,
    /// Folder with det_500m.onnx and w600k_mbf.onnx
    #[arg(long, default_value = "models")]
    models: PathBuf,
    /// Address to listen on. The default only accepts this computer; 0.0.0.0 shares the
    /// gallery with your network. There is no login: anyone who can reach it can see,
    /// rotate and delete photos, so only share it on a network you trust.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// A host name to accept in the address bar (repeatable), when the server is reached
    /// through a name other than an IP address or this computer's name, for example
    /// photos.home. Other names are refused, which protects against DNS rebinding.
    #[arg(long = "allow-host", value_name = "NAME")]
    allow_host: Vec<String>,
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
    imadive::init_logging();
    let args = Args::parse();
    let models = if args.no_faces { None } else { imadive::enable_faces(&args.models, args.onnxruntime.as_deref()) };
    let gallery = Gallery::open(Config {
        data_dir: args.data.unwrap_or_else(default_data_dir),
        folders: args.library,
        models,
        face_threshold: args.face_threshold,
        host: None,
        allowed_names: args.allow_host,
    })?;

    if args.scan_only {
        return tokio::task::spawn_blocking(move || gallery.scan_blocking()).await?;
    }
    gallery.start_scan();
    let listener = tokio::net::TcpListener::bind((args.host.as_str(), args.port)).await?;
    tracing::info!("gallery ready at http://{}", listener.local_addr()?);
    gallery.serve(listener).await
}

/// `imadive-data`, unless only the folder of the app's old name exists: then that one, so an
/// existing index keeps working after the rename.
fn default_data_dir() -> PathBuf {
    let (now, old) = (PathBuf::from("imadive-data"), PathBuf::from("totufoto-data"));
    if !now.exists() && old.is_dir() {
        tracing::info!("using the index in ./totufoto-data (rename it to imadive-data whenever you like)");
        return old;
    }
    now
}
