// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod runtime;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use imadive::{Config, Gallery, Host};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

struct DesktopHost {
    app: AppHandle,
    log: Option<PathBuf>,
}

impl Host for DesktopHost {
    fn pick_folder(&self) -> Option<PathBuf> {
        let mut dialog = self.app.dialog().file().set_title("Add a folder");
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

    fn log_file(&self) -> Option<PathBuf> {
        self.log.clone()
    }
}

fn main() {
    imadive::init_desktop_logging();
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
                    .message(format!("Imadive could not start:\n\n{e:#}"))
                    .kind(MessageDialogKind::Error)
                    .title("Imadive")
                    .blocking_show();
                std::process::exit(1);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Imadive");
}

/// Imadive was called Totufoto, whose folders were named after its old identifier. When
/// `folder` doesn't exist yet but the old one next to it does, the old one is moved to it,
/// so the library carries over. Returns the folder to use: the old one when it couldn't be
/// moved (an old copy of the app still running, say), so nothing is lost; the move is tried
/// again on the next start.
fn adopt_old_folder(folder: &Path) -> PathBuf {
    const OLD_IDENTIFIER: &str = "com.javiespinar.totufoto";
    let Some(old) = folder.parent().map(|p| p.join(OLD_IDENTIFIER)) else { return folder.to_path_buf() };
    if folder.exists() || !old.is_dir() {
        return folder.to_path_buf();
    }
    match std::fs::rename(&old, folder) {
        Ok(()) => {
            tracing::info!("moved {} to {}", old.display(), folder.display());
            folder.to_path_buf()
        }
        Err(e) => {
            tracing::warn!("couldn't move {} to {} ({e}); using it where it is", old.display(), folder.display());
            old
        }
    }
}

/// The window's address is always the same, so what the page saves (the language, the
/// theme...) is there next time: browsers keep it per address, port included. Only when
/// something else holds this port does the app take a free one, and those settings wait
/// until the next start.
const PORT: u16 = 47878;

/// Starts the gallery server on its local port and opens the window on it.
fn start(app: &mut tauri::App) -> Result<()> {
    let data_dir = adopt_old_folder(&app.path().app_data_dir().context("finding the app data folder")?);
    // On Windows the window's own data (saved preferences) is in a second folder.
    if let Ok(local) = app.path().app_local_data_dir() {
        adopt_old_folder(&local);
    }
    // In the data folder, which only exists from here on; earlier lines only reach the terminal.
    let log = data_dir.join("logs").join("imadive.log");
    let log = match imadive::log_to_file(&log) {
        Ok(()) => Some(log),
        Err(e) => {
            tracing::warn!("no log file: {e:#}");
            None
        }
    };
    tracing::info!("Imadive {} on {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS);
    tracing::info!("data folder: {}", data_dir.display());
    let models = match runtime::install(&data_dir.join("runtime")) {
        Ok(rt) => imadive::enable_faces(&rt.models, Some(&rt.onnxruntime)),
        Err(e) => {
            tracing::warn!("face recognition disabled: {e:#}");
            None
        }
    };
    let gallery = Gallery::open(Config {
        data_dir,
        folders: Vec::new(),
        models,
        face_threshold: imadive::DEFAULT_FACE_THRESHOLD,
        host: Some(Arc::new(DesktopHost { app: app.handle().clone(), log })),
        allowed_names: Vec::new(),
    })?;
    gallery.start_scan();

    let listener = match std::net::TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(listener) => listener,
        Err(e) => {
            tracing::warn!("port {PORT} is taken ({e}); saved settings won't apply this time");
            std::net::TcpListener::bind("127.0.0.1:0")?
        }
    };
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
        .title("Imadive")
        .inner_size(1400.0, 900.0)
        .min_inner_size(720.0, 480.0)
        .build()?;
    Ok(())
}

/// `imadive-desktop --self-test [photos-folder] [report-file]`: checks, without a window,
/// that the embedded runtime loads and indexes photos. Used by CI on every platform.
fn self_test(args: &[String]) -> i32 {
    let report_path = args.get(1).map(|a| launch_relative(a));
    let (ok, report) = match run_self_test(args.first().map(|a| launch_relative(a))) {
        Ok((ok, report)) => (ok, report),
        Err(e) => (false, format!("self-test error: {e:#}")),
    };
    let report = format!("{report}\nresult: {}\n", if ok { "PASS" } else { "FAIL" });
    print!("{report}");
    if let Some(path) = report_path
        && let Err(e) = std::fs::write(&path, &report)
    {
        // The report is also printed, but a Windows GUI app has nowhere to print to.
        eprintln!("couldn't write {}: {e}", path.display());
        return 2;
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
    let dir = std::env::temp_dir().join(format!("imadive-self-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rt = runtime::install(&dir.join("runtime"))?;
    let mut report = format!(
        "version: {}\nos: {} {}\ncpu: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        imadive::faces::cpu_features()
    );
    let Some(models) = imadive::enable_faces(&rt.models, Some(&rt.onnxruntime)) else {
        report.push_str("face recognition: FAILED to load\n");
        return Ok((false, report));
    };
    report.push_str("face recognition: loaded\n");

    // Run the models once on a synthetic image, independent of any photos.
    let mut face_models = imadive::faces::FaceModels::load(&models)?;
    let blank = image::RgbImage::from_pixel(640, 480, image::Rgb([128, 128, 128]));
    let detections = face_models.detect(&blank)?;
    report.push_str(&format!("synthetic image: {} faces (expected 0)\n", detections.len()));
    let mut ok = detections.is_empty();

    if let Some(photos) = photos {
        let gallery = Gallery::open(Config {
            data_dir: dir.join("data"),
            folders: vec![photos],
            models: Some(models),
            face_threshold: imadive::DEFAULT_FACE_THRESHOLD,
            host: None,
            allowed_names: Vec::new(),
        })?;
        gallery.scan_blocking()?;
        let s = gallery.status();
        report.push_str(&format!("photos: {} indexed, {} errors, {} faces\n", s.done, s.errors, s.faces));
        ok &= s.done > 0 && s.errors == 0 && s.faces > 0;
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok((ok, report))
}

#[cfg(test)]
mod tests {
    use super::adopt_old_folder;

    #[test]
    fn the_old_library_moves_to_the_new_name_once() {
        let base = std::env::temp_dir().join(format!("imadive-adopt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (old, new) = (base.join("com.javiespinar.totufoto"), base.join("com.waiting4timeout.imadive"));
        std::fs::create_dir_all(old.join("data")).unwrap();
        std::fs::write(old.join("data/index.sqlite"), b"library").unwrap();

        assert_eq!(adopt_old_folder(&new), new);
        assert_eq!(std::fs::read(new.join("data/index.sqlite")).unwrap(), b"library");
        assert!(!old.exists());

        // A folder of the new name is never replaced by an old one.
        std::fs::create_dir_all(&old).unwrap();
        assert_eq!(adopt_old_folder(&new), new);
        assert!(old.exists() && new.join("data/index.sqlite").exists());
        // Nothing to move: nothing happens.
        let other = base.join("fresh");
        std::fs::remove_dir_all(&old).unwrap();
        assert_eq!(adopt_old_folder(&other), other);
        assert!(!other.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
