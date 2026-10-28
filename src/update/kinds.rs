//! How this copy was installed, which decides how it updates, and the command to show when it
//! can't update itself.

use super::*;

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
    pub(super) fn asset(self, version: &str) -> Option<String> {
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
pub(super) fn shell_quote(word: &str) -> String {
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
