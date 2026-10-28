//! `imadive setup`: installs the gallery as a systemd service on a Linux server, with a
//! wizard that has an answer ready for every question. Run again, it updates the program,
//! changes the settings or uninstalls it.
//!
//! What it writes: the program with its face models and ONNX Runtime in `/opt/imadive`
//! (copied from the folder the running program is in, as the release archive unpacks), a
//! link in `/usr/local/bin`, the service in `/etc/systemd/system/imadive.service`, and the
//! answers in `/opt/imadive/setup.json`, which the next run starts from. Photos are only
//! read; the index goes in the data folder chosen.

use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

mod install;
mod self_update;
mod system;
mod units;
mod wizard;

use install::*;
pub use self_update::self_update;
use system::*;
use units::*;
use wizard::*;

const INSTALL_DIR: &str = "/opt/imadive";

const UNIT_PATH: &str = "/etc/systemd/system/imadive.service";

const LINK: &str = "/usr/local/bin/imadive";

const SETTINGS_PATH: &str = "/opt/imadive/setup.json";

/// Runs `imadive self-update` as root when the gallery asks for an update (see
/// [`render_update_units`]); the gallery's own service runs as a normal user.
const UPDATE_SERVICE_PATH: &str = "/etc/systemd/system/imadive-update.service";

/// The user created for the gallery when no other is chosen.
const SYSTEM_USER: &str = "imadive";

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(clap::Args)]
pub struct SetupArgs {
    /// Ask nothing: use the current settings (or the defaults) and the options given here.
    /// For scripts.
    #[arg(long)]
    yes: bool,
    /// A photo folder (repeatable). Folders can also be added later in the gallery's Settings.
    #[arg(long = "folder", value_name = "PATH")]
    folders: Vec<PathBuf>,
    /// The user the gallery runs as; it must be able to read the photos [default: the one
    /// running sudo, or a new system user called imadive]
    #[arg(long)]
    user: Option<String>,
    /// Where the index goes [default: imadive-data in that user's home, or /var/lib/imadive]
    #[arg(long)]
    data: Option<PathBuf>,
    /// The port the gallery is opened on [default: 7878]
    #[arg(long)]
    port: Option<u16>,
    /// Only this computer can open the gallery (by default, any device on the network)
    #[arg(long)]
    local: bool,
    /// No face recognition
    #[arg(long)]
    no_faces: bool,
    /// Remove the service and the program. Photos are never touched; the index stays unless
    /// --delete-data is given too.
    #[arg(long)]
    uninstall: bool,
    /// With --uninstall: delete the index too (people's names and the faces found)
    #[arg(long, requires = "uninstall")]
    delete_data: bool,
}

/// The answers, kept in setup.json for the next run.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
struct Settings {
    user: String,
    folders: Vec<PathBuf>,
    data: PathBuf,
    /// Any device on the network (0.0.0.0), or only this computer.
    network: bool,
    port: u16,
    faces: bool,
    allow_hosts: Vec<String>,
    face_threshold: f32,
    /// Nice and idle I/O, so indexing doesn't slow the server down.
    low_priority: bool,
    /// The service file as setup last wrote it: tells whether it was edited since.
    #[serde(default)]
    unit: String,
    /// It was edited in setup's editor: updates keep it as it is.
    #[serde(default)]
    edited: bool,
}

pub fn run(args: SetupArgs) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("imadive setup installs a Linux service; on this system run the program directly (see --help)");
    }
    if !Path::new("/run/systemd/system").is_dir() {
        bail!("this system doesn't run systemd, which setup needs; see docs/server.md to run it another way");
    }
    if !is_root() {
        bail!("setup changes system files: run it with sudo (sudo {} setup)", program_name());
    }
    let existing = installed_settings();
    let result = if args.uninstall {
        uninstall(existing.as_ref(), &args)
    } else if args.yes {
        let settings = from_args(existing.clone().unwrap_or_else(defaults), &args);
        let unchanged = existing.as_ref().is_some_and(|current| current == &settings);
        let unit = match existing.as_ref() {
            Some(current) if unchanged => update_unit(current),
            _ => render_unit(&settings),
        };
        apply(&settings, &unit, existing.as_ref())
    } else {
        wizard(existing, &args)
    };
    match result {
        Err(e) if is_interrupted(&e) => {
            cliclack::outro_cancel("Nothing was changed")?;
            Ok(())
        }
        other => other,
    }
}

fn defaults() -> Settings {
    let user = std::env::var("SUDO_USER").ok().filter(|u| !u.is_empty() && u != "root" && user_exists(u));
    let user = user.unwrap_or_else(|| SYSTEM_USER.into());
    Settings {
        data: default_data(&user),
        user,
        folders: Vec::new(),
        network: true,
        port: 7878,
        faces: true,
        allow_hosts: Vec::new(),
        face_threshold: imadive::DEFAULT_FACE_THRESHOLD,
        low_priority: true,
        unit: String::new(),
        edited: false,
    }
}

/// The answers given on the command line, over `base`.
fn from_args(mut s: Settings, args: &SetupArgs) -> Settings {
    if let Some(user) = &args.user {
        if user != &s.user && args.data.is_none() {
            s.data = default_data(user);
        }
        s.user = user.clone();
    }
    if !args.folders.is_empty() {
        s.folders = args.folders.iter().map(|f| std::path::absolute(f).unwrap_or_else(|_| f.clone())).collect();
    }
    if let Some(data) = &args.data {
        s.data = data.clone();
    }
    if let Some(port) = args.port {
        s.port = port;
    }
    if args.local {
        s.network = false;
    }
    if args.no_faces {
        s.faces = false;
    }
    s
}

/// `imadive-data` in the user's home, or /var/lib/imadive for the system user (or a user
/// without a home).
fn default_data(user: &str) -> PathBuf {
    if user != SYSTEM_USER
        && let Some(home) = home_of(user)
        && home.is_dir()
    {
        return home.join("imadive-data");
    }
    PathBuf::from("/var/lib/imadive")
}

fn installed_settings() -> Option<Settings> {
    serde_json::from_str(&fs::read_to_string(SETTINGS_PATH).ok()?).ok()
}

fn installed_version() -> Option<String> {
    let out = Command::new(format!("{INSTALL_DIR}/imadive")).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().nth(1).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings {
            user: "ana".into(),
            folders: vec!["/srv/photos".into(), "/mnt/nas/Family photos".into()],
            data: "/home/ana/imadive-data".into(),
            network: true,
            port: 7878,
            faces: true,
            allow_hosts: Vec::new(),
            face_threshold: imadive::DEFAULT_FACE_THRESHOLD,
            low_priority: true,
            unit: String::new(),
            edited: false,
        }
    }

    #[test]
    fn the_service_runs_the_installed_program_with_the_answers() {
        let unit = render_unit(&settings());
        let exec = unit.lines().find(|l| l.starts_with("ExecStart=")).unwrap();
        assert_eq!(
            exec,
            "ExecStart=/opt/imadive/imadive --host 0.0.0.0 --port 7878 --data /home/ana/imadive-data \
             --models /opt/imadive/models --onnxruntime /opt/imadive/onnxruntime /srv/photos \"/mnt/nas/Family photos\""
        );
        assert!(
            unit.contains("\nUser=ana\n")
                && unit.contains("\nNice=10\n")
                && unit.contains("WantedBy=multi-user.target")
        );
    }

    #[test]
    fn the_other_answers_change_the_service() {
        let s = Settings {
            network: false,
            faces: false,
            low_priority: false,
            allow_hosts: vec!["photos.home".into()],
            face_threshold: 0.5,
            folders: Vec::new(),
            ..settings()
        };
        let unit = render_unit(&s);
        let exec = unit.lines().find(|l| l.starts_with("ExecStart=")).unwrap();
        assert!(exec.contains("--host 127.0.0.1"));
        assert!(exec.ends_with("--no-faces --face-threshold 0.5 --allow-host photos.home"));
        assert!(!unit.contains("Nice="));
    }

    #[test]
    fn updates_asked_for_in_the_data_folder_run_self_update() {
        let (path_unit, service) = render_update_units(Path::new("/home/ana/100% photos"));
        assert!(path_unit.contains("\nPathChanged=/home/ana/100%% photos/update-request\n"), "{path_unit}");
        assert!(
            path_unit.contains("\nUnit=imadive-update.service\n") && path_unit.contains("WantedBy=multi-user.target")
        );
        assert!(service.contains("\nType=oneshot\nExecStart=/opt/imadive/imadive self-update\n"), "{service}");
        assert!(!service.contains("User="), "it runs as root");
    }

    #[test]
    fn arguments_are_quoted_as_systemd_reads_them() {
        assert_eq!(systemd_quote("/srv/photos"), "/srv/photos");
        assert_eq!(systemd_quote("/srv/My photos"), "\"/srv/My photos\"");
        assert_eq!(systemd_quote("/srv/100% \"best\""), "\"/srv/100%% \\\"best\\\"\"");
        assert_eq!(systemd_quote("/srv/$HOME"), "/srv/$$HOME");
    }

    #[test]
    fn lists_are_split_on_commas() {
        assert_eq!(split_paths(" /a , /b c ,, "), [PathBuf::from("/a"), PathBuf::from("/b c")]);
        assert!(split_list("").is_empty());
        assert_eq!(join_paths(&split_paths("/a, /b")), "/a, /b");
    }

    #[test]
    fn the_settings_survive_the_settings_file() {
        let s = settings();
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
