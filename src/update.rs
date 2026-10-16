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

/// GitHub, or a test server standing in for it (`IMADIVE_UPDATE_BASE`).
fn base() -> String {
    std::env::var("IMADIVE_UPDATE_BASE").unwrap_or_else(|_| "https://github.com".into())
}

/// The page listing the releases.
pub fn releases_url() -> String {
    format!("https://github.com/{REPO}/releases")
}

/// How this copy was installed, which decides how it updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    /// The Windows desktop app: one .exe, replaced in place.
    WindowsExe,
    /// The Linux desktop app as an AppImage ($APPIMAGE), replaced in place.
    AppImage,
    /// The Linux desktop app from the .deb package, updated with apt.
    Deb,
    /// A server installed with `imadive setup`, with the root service that installs updates.
    Setup,
    /// A server installed with an older `imadive setup`, before that service existed.
    SetupWithoutHelper,
    Docker,
    /// Anything else: built from source, or a program started by hand.
    Source,
}

impl InstallKind {
    /// The desktop app's.
    pub fn desktop() -> Self {
        if cfg!(windows) {
            Self::WindowsExe
        } else if std::env::var_os("APPIMAGE").is_some() {
            Self::AppImage
        } else if cfg!(target_os = "linux") {
            Self::Deb
        } else {
            Self::Source
        }
    }

    /// The command-line app's (a server).
    pub fn server() -> Self {
        if std::env::var("IMADIVE_INSTALL").is_ok_and(|v| v == "docker") {
            return Self::Docker;
        }
        let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
        if exe.as_deref() == Some(Path::new(SETUP_PROGRAM)) {
            return if Path::new(HELPER_PATH_UNIT).exists() { Self::Setup } else { Self::SetupWithoutHelper };
        }
        Self::Source
    }

    pub fn can_install(self) -> bool {
        matches!(self, Self::WindowsExe | Self::AppImage | Self::Deb | Self::Setup)
    }

    /// What to run to update a copy that can't update itself.
    pub fn command(self) -> Option<String> {
        match self {
            Self::SetupWithoutHelper => {
                Some(format!("curl -fsSL https://raw.githubusercontent.com/{REPO}/main/scripts/install.sh | sh"))
            }
            Self::Docker => Some(format!("docker pull ghcr.io/{REPO}:latest")),
            // A server built from source gets one to switch to the ready-made build, with its
            // own settings (see `switch_command`).
            _ => None,
        }
    }

    /// The release file this kind installs.
    fn asset(self, version: &str) -> Option<String> {
        match self {
            Self::WindowsExe => Some(format!("Imadive-{version}-windows-x64.exe")),
            Self::AppImage => Some(format!("Imadive-{version}-linux-x86_64.AppImage")),
            Self::Deb => Some(format!("Imadive-{version}-linux-amd64.deb")),
            Self::Setup | Self::SetupWithoutHelper => Some(server_asset(version)),
            Self::Docker | Self::Source => None,
        }
    }
}

/// Single quotes for the shell, unless the word needs none.
fn shell_quote(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "/._-:@=+,".contains(c)) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// What a server built from source runs once to become the ready-made one, which then updates
/// from Settings: its service stopped (setup puts its own in place), then the install script,
/// whose setup starts from this server's settings (the user it runs as, the index, the
/// folders, the port) so nothing is indexed again.
pub fn switch_command(
    service: Option<&str>,
    user: Option<&str>,
    data: &Path,
    folders: &[PathBuf],
    port: u16,
    local: bool,
) -> String {
    let mut setup: Vec<String> = Vec::new();
    if let Some(user) = user {
        setup.extend(["--user".into(), shell_quote(user)]);
    }
    setup.extend(["--data".into(), shell_quote(&data.to_string_lossy())]);
    for folder in folders {
        setup.extend(["--folder".into(), shell_quote(&folder.to_string_lossy())]);
    }
    setup.extend(["--port".into(), port.to_string()]);
    if local {
        setup.push("--local".into());
    }
    let install = format!(
        "curl -fsSL https://raw.githubusercontent.com/{REPO}/main/scripts/install.sh | sh -s -- {}",
        setup.join(" ")
    );
    match service {
        Some(service) => format!("sudo systemctl disable --now {} && {install}", shell_quote(service)),
        None => install,
    }
}

/// The systemd service this program runs in, if any (from /proc/self/cgroup).
pub fn current_service() -> Option<String> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    cgroup.lines().find_map(|line| {
        let unit = line.rsplit('/').next()?;
        unit.strip_suffix(".service").filter(|name| !name.contains('@')).map(str::to_string)
    })
}

/// The server archive for this computer's processor.
pub fn server_asset(version: &str) -> String {
    format!("Imadive-server-{version}-linux-{}.tar.gz", std::env::consts::ARCH)
}

/// "1.2.3" as numbers, to compare versions ("0.3.10" is newer than "0.3.9").
fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// The version at the end of the address `releases/latest` redirects to (`.../tag/v0.3.3`).
fn version_from_location(location: &str) -> Option<String> {
    let tag = location.trim_end_matches('/').rsplit('/').next()?;
    let version = tag.strip_prefix('v').unwrap_or(tag);
    parse_version(version).map(|_| version.to_string())
}

fn agent(redirects: u32) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
    ureq::Agent::config_builder()
        .max_redirects(redirects)
        .http_status_as_error(redirects > 0)
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(30)))
        .tls_config(tls)
        .user_agent(format!("imadive/{VERSION}"))
        .build()
        .into()
}

/// The latest release's version.
pub fn latest_version() -> Result<String> {
    latest_version_at(&base())
}

fn latest_version_at(base: &str) -> Result<String> {
    let url = format!("{base}/{REPO}/releases/latest");
    let response = agent(0).get(&url).call().context("couldn't reach GitHub")?;
    let location = response
        .headers()
        .get("location")
        .and_then(|l| l.to_str().ok())
        .with_context(|| format!("GitHub didn't say which release is the latest ({})", response.status()))?;
    version_from_location(location).with_context(|| format!("couldn't tell the latest version from {location}"))
}

fn release_file(base: &str, version: &str, name: &str) -> String {
    format!("{base}/{REPO}/releases/download/v{version}/{name}")
}

/// The checksum `SHA256SUMS` gives for `name` (lines as `sha256sum` writes them).
fn checksum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        (file.trim().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
    })
}

/// Downloads release file `name` of `version` to `dest`, checked against the release's
/// checksums; `progress` gets the bytes done and the total. A file that doesn't match is
/// deleted.
pub fn download(version: &str, name: &str, dest: &Path, progress: &dyn Fn(u64, u64)) -> Result<()> {
    download_at(&base(), version, name, dest, progress)
}

fn download_at(base: &str, version: &str, name: &str, dest: &Path, progress: &dyn Fn(u64, u64)) -> Result<()> {
    let agent = agent(10);
    let sums = agent
        .get(&release_file(base, version, "SHA256SUMS"))
        .call()
        .context("couldn't download the release's checksums")?
        .into_body()
        .read_to_string()?;
    let expected = checksum_for(&sums, name).with_context(|| format!("{name} isn't in release {version}"))?;
    let response =
        agent.get(&release_file(base, version, name)).call().with_context(|| format!("couldn't download {name}"))?;
    let total = response.body().content_length().unwrap_or(0);
    let mut reader = response.into_body().into_reader();
    let mut file = std::fs::File::create(dest).with_context(|| format!("writing {}", dest.display()))?;
    let mut hasher = Sha256::new();
    let (mut buf, mut done) = (vec![0u8; 256 * 1024], 0u64);
    let copied = (|| -> Result<()> {
        loop {
            let n = reader.read(&mut buf).context("the download was interrupted")?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            done += n as u64;
            progress(done, total);
        }
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    let checked = copied.and_then(|()| {
        if actual != expected {
            bail!("{name} doesn't match its checksum; it wasn't installed");
        }
        Ok(())
    });
    if checked.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    checked
}

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
        let restart = |what: &str| {
            tracing::info!("{what}; restarting");
            self.set_state("restarting", None);
            match host {
                Some(host) => host.restart(),
                None => tracing::warn!("no desktop app to restart"),
            }
        };
        match self.kind {
            InstallKind::WindowsExe => {
                let exe = std::env::current_exe()?;
                let fresh = exe.with_extension("exe.new");
                download(version, &asset, &fresh, &progress)?;
                self.set_state("installing", None);
                replace_running(&exe, &fresh)?;
                restart("installed");
            }
            InstallKind::AppImage => {
                let target = PathBuf::from(std::env::var_os("APPIMAGE").context("APPIMAGE isn't set")?);
                let fresh = target.with_extension("AppImage.new");
                download(version, &asset, &fresh, &progress)?;
                self.set_state("installing", None);
                make_executable(&fresh)?;
                std::fs::rename(&fresh, &target).with_context(|| format!("replacing {}", target.display()))?;
                restart("installed");
            }
            InstallKind::Deb => {
                let package = std::env::temp_dir().join(&asset);
                download(version, &asset, &package, &progress)?;
                self.set_state("installing", None);
                install_deb(&package)?;
                let _ = std::fs::remove_file(&package);
                restart("installed");
            }
            InstallKind::Setup => {
                let _ = std::fs::remove_file(self.data_dir.join(STATUS_FILE));
                std::fs::write(self.data_dir.join(REQUEST_FILE), chrono::Local::now().to_rfc3339())
                    .context("asking the update service")?;
                tracing::info!("asked the update service to install {version}");
            }
            _ => bail!("this copy can't update itself"),
        }
        Ok(())
    }
}

/// Puts `fresh` in place of the running program `exe`. Windows can't overwrite a running
/// program, but it can rename it: the old one becomes `.old`, deleted on the next start.
fn replace_running(exe: &Path, fresh: &Path) -> Result<()> {
    let old = exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).with_context(|| format!("moving {} aside", exe.display()))?;
    if let Err(e) = std::fs::rename(fresh, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(e).with_context(|| format!("putting the new {} in place", exe.display()));
    }
    Ok(())
}

/// Deletes what an update on Windows left behind (the program it replaced).
pub fn clean_up() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Installs a .deb with apt, which asks for the password in a system dialog (pkexec).
fn install_deb(package: &Path) -> Result<()> {
    let status = std::process::Command::new("pkexec")
        .args(["apt-get", "install", "-y", "--allow-downgrades"])
        .arg(package)
        .status();
    match status {
        Ok(s) if s.success() => Ok(()),
        // pkexec: 126 when the password dialog was dismissed, 127 when not allowed.
        Ok(s) if matches!(s.code(), Some(126 | 127)) => bail!("the update was cancelled"),
        Ok(s) => bail!("apt couldn't install the package ({s}); it is in {}", package.display()),
        Err(e) => bail!("couldn't ask for the password ({e}); install {} yourself", package.display()),
    }
}

/// An error the root service reported, if it failed.
fn helper_error(data_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(data_dir.join(STATUS_FILE)).ok()?;
    let status: serde_json::Value = serde_json::from_str(&text).ok()?;
    (status["state"] == "error").then(|| status["error"].as_str().unwrap_or("the update failed").to_string())
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
