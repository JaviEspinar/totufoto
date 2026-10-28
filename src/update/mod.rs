//! Updates from the project's GitHub releases.
//!
//! The latest version is the tag GitHub's `releases/latest` page redirects to, as
//! `scripts/install.sh` finds it: no API call, so no rate limit. Every file downloaded is
//! checked against the release's `SHA256SUMS` before anything is replaced.
//!
//! How this copy updates depends on how it was installed ([`InstallKind`]): the desktop
//! app replaces its own file (Windows, AppImage) or installs the new package (.deb, asking
//! for the password), then restarts; a server installed with `imadive setup` asks a small
//! root service to install the new release (its service runs as a normal user, which can't
//! write to /opt; see `imadive self-update`); Docker and builds from source are told the
//! command to run.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::app::Host;

mod install;
mod kinds;
mod release;

pub use install::clean_up;
use install::*;
pub use kinds::{InstallKind, current_service, server_asset, switch_command};
pub use release::{download, is_newer, latest_version, releases_url};

/// The project on GitHub, where releases come from.
pub const REPO: &str = "waiting4timeout/imadive";

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Installed by `imadive setup`: the program, and the unit that installs updates asked for
/// from the gallery.
pub const SETUP_PROGRAM: &str = "/opt/imadive/imadive";

pub const HELPER_PATH_UNIT: &str = "/etc/systemd/system/imadive-update.path";

/// In the data folder: written by the gallery to ask for an update, and by the root service
/// to say how it went.
pub const REQUEST_FILE: &str = "update-request";

pub const STATUS_FILE: &str = "update-status.json";

/// What the page shows in Settings.
#[derive(Clone, Debug, Serialize)]
pub struct UpdateView {
    pub current: String,
    pub latest: Option<String>,
    /// When the last check ended (local time), if any.
    pub checked: Option<String>,
    pub available: bool,
    pub kind: InstallKind,
    pub can_install: bool,
    /// It checks once a day on its own (not with `--no-update-check`).
    pub automatic: bool,
    /// For a copy that can't update itself: what to run.
    pub command: Option<String>,
    pub releases: String,
    /// "idle", "checking", "downloading" (with `done` and `total` bytes), "installing",
    /// "restarting" or "error" (with `error`).
    pub state: &'static str,
    pub done: u64,
    pub total: u64,
    pub error: Option<String>,
}

#[derive(Default)]
struct Inner {
    latest: Option<String>,
    checked: Option<String>,
    state: Option<&'static str>,
    error: Option<String>,
}

/// Why an update can't start.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    CantUpdateItself,
    NothingNewer,
    Busy,
}

/// The update state of this copy, shared by the handlers and the background check.
pub struct Updates {
    kind: InstallKind,
    /// What to run instead of the kind's own command (a server built from source: switching
    /// to the ready-made build, with its settings).
    command: Mutex<Option<String>>,
    automatic: AtomicBool,
    /// Where the request and status files of a server's root service go.
    data_dir: PathBuf,
    inner: Mutex<Inner>,
    busy: AtomicBool,
    done: AtomicU64,
    total: AtomicU64,
}

impl Updates {
    pub fn new(kind: InstallKind, data_dir: PathBuf) -> Self {
        Self {
            kind,
            command: Mutex::new(None),
            automatic: AtomicBool::new(false),
            data_dir,
            inner: Mutex::default(),
            busy: AtomicBool::new(false),
            done: AtomicU64::new(0),
            total: AtomicU64::new(0),
        }
    }

    pub fn view(&self) -> UpdateView {
        let mut inner = self.inner.lock().unwrap();
        // A server's root service reports in a file.
        if self.kind == InstallKind::Setup
            && inner.state == Some("installing")
            && let Some(error) = helper_error(&self.data_dir)
        {
            inner.state = Some("error");
            inner.error = Some(error);
            self.busy.store(false, Ordering::SeqCst);
        }
        let available = inner.latest.as_deref().is_some_and(|l| is_newer(l, VERSION));
        UpdateView {
            current: VERSION.into(),
            latest: inner.latest.clone(),
            checked: inner.checked.clone(),
            available,
            kind: self.kind,
            can_install: self.kind.can_install(),
            automatic: self.automatic.load(Ordering::Relaxed),
            command: self.command.lock().unwrap().clone().or_else(|| self.kind.command()),
            releases: releases_url(),
            state: inner.state.unwrap_or("idle"),
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            error: inner.error.clone(),
        }
    }

    /// The command to show instead of the kind's own.
    pub fn set_command(&self, command: String) {
        *self.command.lock().unwrap() = Some(command);
    }

    pub fn kind(&self) -> InstallKind {
        self.kind
    }

    pub fn available(&self) -> bool {
        self.inner.lock().unwrap().latest.as_deref().is_some_and(|l| is_newer(l, VERSION))
    }

    fn set_state(&self, state: &'static str, error: Option<String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.state = Some(state);
        inner.error = error;
    }

    /// Asks GitHub for the latest version (blocking).
    pub fn check(&self) {
        if self.busy.load(Ordering::SeqCst) {
            return;
        }
        self.set_state("checking", None);
        let result = latest_version();
        let mut inner = self.inner.lock().unwrap();
        inner.checked = Some(chrono::Local::now().format("%Y-%m-%d %H:%M").to_string());
        match result {
            Ok(latest) => {
                if is_newer(&latest, VERSION) {
                    tracing::info!("Imadive {latest} is available (this is {VERSION})");
                }
                inner.latest = Some(latest);
                inner.state = None;
                inner.error = None;
            }
            Err(e) => {
                tracing::warn!("checking for updates: {e:#}");
                inner.state = Some("error");
                inner.error = Some(format!("{e:#}"));
            }
        }
    }

    /// Installs the latest version in the background. `Err` says why it can't start.
    pub fn install(self: &Arc<Self>, host: Option<Arc<dyn Host>>) -> std::result::Result<(), Refused> {
        if !self.kind.can_install() {
            return Err(Refused::CantUpdateItself);
        }
        let Some(latest) = self.inner.lock().unwrap().latest.clone().filter(|l| is_newer(l, VERSION)) else {
            return Err(Refused::NothingNewer);
        };
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(Refused::Busy);
        }
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        self.set_state(if self.kind == InstallKind::Setup { "installing" } else { "downloading" }, None);
        let me = self.clone();
        std::thread::spawn(move || {
            match me.install_now(&latest, host.as_deref()) {
                // A server's root service restarts it; the desktop app restarts itself.
                Ok(()) => {}
                Err(e) => {
                    tracing::error!("updating to {latest}: {e:#}");
                    me.set_state("error", Some(format!("{e:#}")));
                    me.busy.store(false, Ordering::SeqCst);
                }
            }
        });
        Ok(())
    }

    fn install_now(&self, version: &str, host: Option<&dyn Host>) -> Result<()> {
        tracing::info!("updating to Imadive {version}");
        let progress = |done: u64, total: u64| {
            self.done.store(done, Ordering::Relaxed);
            self.total.store(total, Ordering::Relaxed);
        };
        let asset = self.kind.asset(version).context("this copy can't update itself")?;
        match self.kind {
            InstallKind::WindowsExe => install::windows_exe(self, version, &asset, &progress)?,
            InstallKind::AppImage => install::app_image(self, version, &asset, &progress)?,
            InstallKind::Deb => install::deb(self, version, &asset, &progress)?,
            // The update service installs it and restarts the gallery.
            InstallKind::Setup => return install::ask_the_update_service(self, version),
            _ => bail!("this copy can't update itself"),
        }
        tracing::info!("installed; restarting");
        self.set_state("restarting", None);
        match host {
            Some(host) => host.restart(),
            None => tracing::warn!("no desktop app to restart"),
        }
        Ok(())
    }
}

/// Checks a minute after starting, then once a day.
pub fn spawn_checks(updates: Arc<Updates>) {
    updates.automatic.store(true, Ordering::Relaxed);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            let me = updates.clone();
            let _ = tokio::task::spawn_blocking(move || me.check()).await;
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::release::*;
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.3.10", "0.3.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.3.2", "0.3.2"));
        assert!(!is_newer("0.3.1", "0.3.2"));
        assert!(!is_newer("nonsense", "0.3.2"));
        assert_eq!(parse_version("v0.3.3"), Some((0, 3, 3)));
        assert_eq!(parse_version("0.3"), None);
        assert_eq!(parse_version("0.3.3.1"), None);
    }

    #[test]
    fn the_latest_version_is_in_the_redirect() {
        let at = |l: &str| version_from_location(l);
        assert_eq!(at("https://github.com/waiting4timeout/imadive/releases/tag/v0.3.3").as_deref(), Some("0.3.3"));
        assert_eq!(at("/waiting4timeout/imadive/releases/tag/v1.2.10/").as_deref(), Some("1.2.10"));
        assert_eq!(at("https://github.com/waiting4timeout/imadive/releases"), None);
    }

    #[test]
    fn checksums_are_found_by_file_name() {
        let sums = "ABC123  Imadive-0.3.3-windows-x64.exe\ndef456 *Imadive-0.3.3-linux-amd64.deb\n";
        assert_eq!(checksum_for(sums, "Imadive-0.3.3-windows-x64.exe").as_deref(), Some("abc123"));
        assert_eq!(checksum_for(sums, "Imadive-0.3.3-linux-amd64.deb").as_deref(), Some("def456"));
        assert_eq!(checksum_for(sums, "Imadive-0.3.3.exe"), None);
    }

    #[test]
    fn each_kind_installs_its_own_file() {
        assert_eq!(InstallKind::WindowsExe.asset("0.3.3").unwrap(), "Imadive-0.3.3-windows-x64.exe");
        assert_eq!(InstallKind::Deb.asset("0.3.3").unwrap(), "Imadive-0.3.3-linux-amd64.deb");
        assert_eq!(InstallKind::AppImage.asset("0.3.3").unwrap(), "Imadive-0.3.3-linux-x86_64.AppImage");
        assert!(InstallKind::Setup.asset("0.3.3").unwrap().starts_with("Imadive-server-0.3.3-linux-"));
        assert!(InstallKind::Docker.asset("0.3.3").is_none());
        assert!(InstallKind::Docker.command().unwrap().contains("docker pull ghcr.io/waiting4timeout/imadive"));
        assert!(InstallKind::Source.command().is_none(), "it gets switch_command, with its settings");
        assert!(!InstallKind::Source.can_install() && InstallKind::Setup.can_install());
    }

    #[test]
    fn a_server_built_from_source_switches_with_its_own_settings() {
        let command = switch_command(
            Some("imadive"),
            Some("caja"),
            Path::new("/home/caja/imadive-data"),
            &[PathBuf::from("/mnt/disk/Imágenes"), PathBuf::from("/srv/it's mine")],
            7878,
            false,
        );
        assert_eq!(
            command,
            "sudo systemctl disable --now imadive && curl -fsSL \
             https://raw.githubusercontent.com/waiting4timeout/imadive/main/scripts/install.sh | sh -s -- \
             --user caja --data /home/caja/imadive-data --folder '/mnt/disk/Imágenes' --folder '/srv/it'\\''s mine' --port 7878"
        );
        // Started by hand: nothing to stop, and only this computer.
        let command = switch_command(None, None, Path::new("/data"), &[], 8080, true);
        assert!(command.starts_with("curl ") && command.ends_with("--data /data --port 8080 --local"), "{command}");
    }

    #[cfg(unix)]
    #[test]
    fn a_running_program_is_replaced_by_renaming() {
        let dir = crate::testutil::TempDir::new();
        let (exe, fresh) = (dir.file("imadive.exe"), dir.file("imadive.exe.new"));
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&fresh, "new").unwrap();
        replace_running(&exe, &fresh).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(exe.with_extension("exe.old")).unwrap(), "old");
        assert!(!fresh.exists());
    }

    /// A stand-in for GitHub: release 9.9.9 with one file, `good`, and its checksum (plus a
    /// `bad` one whose checksum doesn't match). Answers on the address returned.
    fn fake_github() -> String {
        use axum::routing::get;
        let good = b"the new program".to_vec();
        let sum: String = Sha256::digest(&good).iter().map(|b| format!("{b:02x}")).collect();
        let sums = format!("{sum}  good\n{sum}  bad\n");
        let app = axum::Router::new()
            .route(
                "/waiting4timeout/imadive/releases/latest",
                get(|| async {
                    (
                        axum::http::StatusCode::FOUND,
                        [("location", "https://github.com/waiting4timeout/imadive/releases/tag/v9.9.9")],
                    )
                }),
            )
            .route("/waiting4timeout/imadive/releases/download/v9.9.9/SHA256SUMS", get(move || async move { sums }))
            .route("/waiting4timeout/imadive/releases/download/v9.9.9/good", get(move || async move { good }))
            .route("/waiting4timeout/imadive/releases/download/v9.9.9/bad", get(|| async { "something else" }));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        std::thread::spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(listener, app).await.unwrap();
            })
        });
        format!("http://{addr}")
    }

    #[test]
    fn releases_are_found_and_checked_before_use() {
        let base = fake_github();
        assert_eq!(latest_version_at(&base).unwrap(), "9.9.9");
        let dir = crate::testutil::TempDir::new();
        let dest = dir.file("good");
        let seen = AtomicU64::new(0);
        download_at(&base, "9.9.9", "good", &dest, &|done, _| seen.store(done, Ordering::Relaxed)).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"the new program");
        assert_eq!(seen.load(Ordering::Relaxed), 15);

        let bad = dir.file("bad");
        let err = download_at(&base, "9.9.9", "bad", &bad, &|_, _| {}).unwrap_err();
        assert!(err.to_string().contains("doesn't match its checksum"), "{err:#}");
        assert!(!bad.exists(), "a file that doesn't match is deleted");
        let err = download_at(&base, "9.9.9", "missing", &dir.file("missing"), &|_, _| {}).unwrap_err();
        assert!(err.to_string().contains("isn't in release 9.9.9"), "{err:#}");
    }
}
