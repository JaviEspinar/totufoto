// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod runtime;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;
use totufoto::{Config, Gallery, Host};

const FACE_THRESHOLD: f32 = 0.42;

struct DesktopHost {
    app: AppHandle,
}

impl Host for DesktopHost {
    fn pick_folder(&self) -> Option<PathBuf> {
        let mut dialog = self.app.dialog().file().set_title("Add a photo folder");
        if let Some(window) = self.app.get_webview_window("main") {
            dialog = dialog.set_parent(&window);
        }
        dialog.blocking_pick_folder().and_then(|p| p.into_path().ok())
    }

    fn open_url(&self, url: &str) {
        if let Err(e) = self.app.opener().open_url(url, None::<&str>) {
            tracing::warn!("opening {url}: {e}");
        }
    }

    fn reveal(&self, path: &std::path::Path) {
        if let Err(e) = self.app.opener().reveal_item_in_dir(path) {
            tracing::warn!("showing {}: {e}", path.display());
        }
    }
}

fn main() {
    totufoto::init_logging();
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--self-test") {
        std::process::exit(self_test(&args[2..]));
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // A second launch just brings the running window to the front.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Err(e) = start(app) {
                tracing::error!("{e:#}");
                app.dialog()
                    .message(format!("Totufoto could not start:\n\n{e:#}"))
                    .kind(MessageDialogKind::Error)
                    .title("Totufoto")
                    .blocking_show();
                std::process::exit(1);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Totufoto");
}

/// Starts the gallery server on a free local port and opens the window on it.
fn start(app: &mut tauri::App) -> Result<()> {
    let data_dir = app.path().app_data_dir().context("finding the app data folder")?;
    tracing::info!("data folder: {}", data_dir.display());
    let models = match runtime::install(&data_dir.join("runtime")) {
        Ok(rt) => totufoto::enable_faces(&rt.models, Some(&rt.onnxruntime)),
        Err(e) => {
            tracing::warn!("face recognition disabled: {e:#}");
            None
        }
    };
    let gallery = Gallery::open(Config {
        data_dir,
        folders: Vec::new(),
        models,
        face_threshold: FACE_THRESHOLD,
        host: Some(Arc::new(DesktopHost { app: app.handle().clone() })),
    })?;
    gallery.start_scan();

    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    tauri::async_runtime::spawn(async move {
        let served = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => gallery.serve(listener).await,
            Err(e) => Err(e.into()),
        };
        if let Err(e) = served {
            tracing::error!("gallery server stopped: {e:#}");
        }
    });

    let url = format!("http://127.0.0.1:{port}/").parse()?;
    WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
        .title("Totufoto")
        .inner_size(1400.0, 900.0)
        .min_inner_size(720.0, 480.0)
        .build()?;
    Ok(())
}

/// `totufoto-desktop --self-test [photos-folder] [report-file]`: checks, without a window,
/// that the embedded runtime loads and indexes photos. Used by CI on every platform.
fn self_test(args: &[String]) -> i32 {
    let report_path = args.get(1).map(|a| launch_relative(a));
    let (ok, report) = match run_self_test(args.first().map(|a| launch_relative(a))) {
        Ok((ok, report)) => (ok, report),
        Err(e) => (false, format!("self-test error: {e:#}")),
    };
    let report = format!("{report}\nresult: {}\n", if ok { "PASS" } else { "FAIL" });
    print!("{report}");
    if let Some(path) = report_path {
        let _ = std::fs::write(path, &report);
    }
    if ok { 0 } else { 1 }
}

/// Resolves a command-line path against the folder the app was launched from. The AppImage
/// launcher changes the working directory and passes the original one in `OWD`.
fn launch_relative(arg: &str) -> PathBuf {
    let path = PathBuf::from(arg);
    match std::env::var_os("OWD") {
        Some(owd) if path.is_relative() => PathBuf::from(owd).join(path),
        _ => path,
    }
}

fn run_self_test(photos: Option<PathBuf>) -> Result<(bool, String)> {
    let dir = std::env::temp_dir().join(format!("totufoto-self-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rt = runtime::install(&dir.join("runtime"))?;
    let mut report = format!(
        "version: {}\nos: {} {}\ncpu: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        totufoto::faces::cpu_features()
    );
    let Some(models) = totufoto::enable_faces(&rt.models, Some(&rt.onnxruntime)) else {
        report.push_str("face recognition: FAILED to load\n");
        return Ok((false, report));
    };
    report.push_str("face recognition: loaded\n");

    // Run the models once on a synthetic image, independent of any photos.
    let mut face_models = totufoto::faces::FaceModels::load(&models)?;
    let blank = image::RgbImage::from_pixel(640, 480, image::Rgb([128, 128, 128]));
    let detections = face_models.detect(&blank)?;
    report.push_str(&format!("synthetic image: {} faces (expected 0)\n", detections.len()));
    let mut ok = detections.is_empty();

    if let Some(photos) = photos {
        let gallery = Gallery::open(Config {
            data_dir: dir.join("data"),
            folders: vec![photos],
            models: Some(models),
            face_threshold: FACE_THRESHOLD,
            host: None,
        })?;
        gallery.scan_blocking()?;
        let s = gallery.status();
        report.push_str(&format!("photos: {} indexed, {} errors, {} faces\n", s.done, s.errors, s.faces));
        ok &= s.done > 0 && s.errors == 0 && s.faces > 0;
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok((ok, report))
}
