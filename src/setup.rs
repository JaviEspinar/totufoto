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

const INSTALL_DIR: &str = "/opt/imadive";
const UNIT_PATH: &str = "/etc/systemd/system/imadive.service";
const LINK: &str = "/usr/local/bin/imadive";
const SETTINGS_PATH: &str = "/opt/imadive/setup.json";
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

fn wizard(existing: Option<Settings>, args: &SetupArgs) -> Result<()> {
    cliclack::intro(format!(" Imadive {VERSION} setup "))?;
    let mut settings = match &existing {
        Some(current) => {
            let installed = installed_version().unwrap_or_else(|| "an earlier version".into());
            let action = cliclack::select(format!("Imadive is installed ({installed}). What would you like to do?"))
                .item("update", format!("Install {VERSION}, keeping the settings"), "")
                .item("change", "Change the settings", "")
                .item("uninstall", "Uninstall", "photos are never touched")
                .item("cancel", "Cancel", "")
                .interact()?;
            match action {
                "update" => return confirm_and_apply(current.clone(), existing.as_ref(), true),
                "uninstall" => return uninstall(existing.as_ref(), args),
                "cancel" => {
                    cliclack::outro("Nothing was changed")?;
                    return Ok(());
                }
                _ => current.clone(),
            }
        }
        None => {
            cliclack::note(
                "Before you start",
                "The gallery has no login: anyone who can reach it can see, rotate and delete \
                 photos. Share it only on a network you trust, and never forward its port on \
                 your router.\n\nEvery question has an answer ready: press Enter to keep it.",
            )?;
            from_args(defaults(), args)
        }
    };
    settings = ask(settings)?;
    confirm_and_apply(settings, existing.as_ref(), false)
}

/// The questions, each starting from the answer in `base`.
fn ask(base: Settings) -> Result<Settings> {
    let mut s = base;
    s.user = cliclack::input("Run the gallery as which user?")
        .default_input(&s.user)
        .validate(|name: &String| {
            if name == SYSTEM_USER || user_exists(name) {
                Ok(())
            } else {
                Err(format!("there is no user {name} (type {SYSTEM_USER} to create a system user for it)"))
            }
        })
        .interact()?;
    if s.user == SYSTEM_USER && !user_exists(SYSTEM_USER) {
        cliclack::log::remark(format!(
            "A system user {SYSTEM_USER} will be created. It needs read access to the photo folders."
        ))?;
    }

    let folders: String = cliclack::input("Photo folders, separated by commas")
        .placeholder("leave it empty to add them later in the gallery's Settings")
        .default_input(&join_paths(&s.folders))
        .required(false)
        .validate(|text: &String| match split_paths(text).into_iter().find(|p| !p.is_dir()) {
            Some(missing) => Err(format!("{} isn't a folder", missing.display())),
            None => Ok(()),
        })
        .interact()?;
    s.folders = split_paths(&folders);
    for folder in s.folders.iter().filter(|f| !readable_by(&s.user, f)) {
        cliclack::log::warning(format!(
            "{} can't read {}: its photos won't show until it can.",
            s.user,
            folder.display()
        ))?;
    }

    let data_default = if s.data.as_os_str().is_empty() { default_data(&s.user) } else { s.data.clone() };
    let data: String = cliclack::input("Where should the index go?")
        .default_input(&data_default.to_string_lossy())
        .validate(|text: &String| if Path::new(text).is_absolute() { Ok(()) } else { Err("an absolute path, please") })
        .interact()?;
    s.data = PathBuf::from(data);

    s.network = cliclack::select("Who can open the gallery?")
        .item(true, "Any device on this network", "listens on 0.0.0.0, with no login")
        .item(false, "Only this computer", "127.0.0.1")
        .initial_value(s.network)
        .interact()?;

    let current_port = s.port;
    let port: String = cliclack::input("Port")
        .default_input(&s.port.to_string())
        .validate(move |text: &String| match text.parse::<u16>() {
            Ok(0) | Err(_) => Err("a number from 1 to 65535".to_string()),
            Ok(p) if p != current_port && !port_free(p) => Err(format!("something else is using port {p}")),
            Ok(_) => Ok(()),
        })
        .interact()?;
    s.port = port.parse()?;

    s.faces = cliclack::confirm("Find and group the people in the photos?").initial_value(s.faces).interact()?;

    if cliclack::confirm("Change the advanced options?").initial_value(false).interact()? {
        let hosts: String = cliclack::input("Other names the gallery is opened by, separated by commas")
            .placeholder("for example photos.home; IP addresses and this computer's name always work")
            .default_input(&s.allow_hosts.join(", "))
            .required(false)
            .interact()?;
        s.allow_hosts = split_list(&hosts);
        let threshold: String = cliclack::input("How alike two faces must be to be the same person (0 to 1)")
            .default_input(&s.face_threshold.to_string())
            .validate(|text: &String| match text.parse::<f32>() {
                Ok(v) if (0.0..=1.0).contains(&v) => Ok(()),
                _ => Err("a number from 0 to 1 (higher is stricter)"),
            })
            .interact()?;
        s.face_threshold = threshold.parse()?;
        s.low_priority = cliclack::confirm("Index at low priority, so the server stays responsive?")
            .initial_value(s.low_priority)
            .interact()?;
    }
    Ok(s)
}

/// The summary, then installing with the generated service file or one edited first.
fn confirm_and_apply(mut settings: Settings, existing: Option<&Settings>, update: bool) -> Result<()> {
    let mut unit = match existing {
        Some(current) if update => {
            if hand_edited(current) || current.edited {
                cliclack::log::info(format!("{UNIT_PATH} was edited, so it is kept as it is."))?;
            }
            update_unit(current)
        }
        _ => render_unit(&settings),
    };
    if !update
        && let Some(current) = existing
        && (hand_edited(current) || current.edited)
    {
        cliclack::log::warning(format!("{UNIT_PATH} was edited: these settings replace it (you can edit it again)."))?;
    }
    // An edit (by hand or in setup) stays marked, so later updates keep it too.
    settings.edited = update && existing.is_some_and(|c| c.edited || hand_edited(c));
    loop {
        cliclack::note("Ready to install", summary(&settings, update))?;
        let choice = cliclack::select("Go ahead?")
            .item("install", if update { "Update" } else { "Install" }, "")
            .item("show", "Show the service file", UNIT_PATH)
            .item("edit", "Edit the service file first", "opens it in your editor")
            .item("cancel", "Cancel", "")
            .interact()?;
        match choice {
            "install" => return apply(&settings, &unit, existing),
            "show" => cliclack::note(UNIT_PATH, unit.trim_end())?,
            "edit" => {
                let edited = edit(&unit)?;
                settings.edited |= edited != unit;
                unit = edited;
            }
            _ => {
                cliclack::outro("Nothing was changed")?;
                return Ok(());
            }
        }
    }
}

/// Installs (or updates) the program, the service and its folders, then starts it and waits
/// for the gallery to answer.
fn apply(settings: &Settings, unit: &str, existing: Option<&Settings>) -> Result<()> {
    let source = program_dir()?;
    let step = |what: &str, f: &mut dyn FnMut() -> Result<()>| -> Result<()> {
        let spinner = cliclack::spinner();
        spinner.start(what);
        match f() {
            Ok(()) => {
                spinner.stop(what);
                Ok(())
            }
            Err(e) => {
                spinner.error(format!("{what}: {e:#}"));
                Err(e)
            }
        }
    };
    if settings.user == SYSTEM_USER && !user_exists(SYSTEM_USER) {
        step(&format!("Creating the user {SYSTEM_USER}"), &mut || {
            let shell = ["/usr/sbin/nologin", "/sbin/nologin"].into_iter().find(|s| Path::new(s).exists());
            run_cmd(
                Command::new("useradd")
                    .args(["--system", "--no-create-home", "--home-dir", "/var/lib/imadive"])
                    .args(["--shell", shell.unwrap_or("/bin/false"), SYSTEM_USER]),
            )
        })?;
    }
    if existing.is_some() {
        // The program can't be replaced while it runs.
        let _ = Command::new("systemctl").args(["stop", "imadive"]).status();
    }
    if source != Path::new(INSTALL_DIR) {
        step(&format!("Copying the program to {INSTALL_DIR}"), &mut || install_files(&source))?;
    }
    step("Preparing the index folder", &mut || {
        fs::create_dir_all(&settings.data).with_context(|| format!("creating {}", settings.data.display()))?;
        run_cmd(Command::new("chown").arg(format!("{0}:", settings.user)).arg(&settings.data))
    })?;
    step("Writing the service", &mut || {
        fs::write(UNIT_PATH, unit).with_context(|| format!("writing {UNIT_PATH}"))?;
        let saved = Settings { unit: unit.to_string(), ..settings.clone() };
        fs::write(SETTINGS_PATH, serde_json::to_string_pretty(&saved)?)?;
        run_cmd(Command::new("systemctl").arg("daemon-reload"))?;
        run_cmd(Command::new("systemctl").args(["enable", "imadive"]))
    })?;
    step("Starting the gallery", &mut || {
        run_cmd(Command::new("systemctl").args(["restart", "imadive"]))?;
        wait_until_answering(settings.port)
    })
    .inspect_err(|_| {
        if let Ok(out) = Command::new("journalctl").args(["-u", "imadive", "-n", "15", "--no-pager"]).output() {
            let _ = cliclack::log::error(String::from_utf8_lossy(&out.stdout).trim_end());
        }
    })?;

    let addresses = if settings.network { local_addresses() } else { vec!["127.0.0.1".into()] };
    let urls: Vec<String> = addresses.iter().map(|a| format!("  http://{a}:{}", settings.port)).collect();
    let mut next = format!("Open the gallery at:\n{}\n", urls.join("\n"));
    if settings.folders.is_empty() {
        next.push_str("\nAdd your photo folders in its Settings (the gear at the top right).\n");
    }
    if settings.network && ufw_active() {
        next.push_str(&format!(
            "\nThe firewall (ufw) is on. To let your network in (with its own range):\n  sudo ufw allow from 192.168.0.0/24 to any port {} proto tcp\n",
            settings.port
        ));
    }
    next.push_str("\nLog: journalctl -u imadive -f\nChange the settings, update or uninstall: sudo imadive setup");
    cliclack::note("The gallery is running", next)?;
    cliclack::outro("Done")?;
    Ok(())
}

fn uninstall(existing: Option<&Settings>, args: &SetupArgs) -> Result<()> {
    if existing.is_none() && !Path::new(UNIT_PATH).exists() {
        bail!("Imadive isn't installed as a service here");
    }
    let data = existing.map(|s| s.data.clone());
    let delete_data = if args.yes {
        args.delete_data
    } else {
        if !cliclack::confirm("Uninstall Imadive? Your photos are not touched.").initial_value(false).interact()? {
            cliclack::outro("Nothing was changed")?;
            return Ok(());
        }
        match &data {
            Some(dir) => cliclack::confirm(format!(
                "Also delete the index in {}? People's names and the faces found would be lost.",
                dir.display()
            ))
            .initial_value(false)
            .interact()?,
            None => false,
        }
    };
    let _ = Command::new("systemctl").args(["disable", "--now", "imadive"]).status();
    let _ = fs::remove_file(UNIT_PATH);
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    if fs::read_link(LINK).is_ok_and(|target| target.starts_with(INSTALL_DIR)) {
        let _ = fs::remove_file(LINK);
    }
    let _ = fs::remove_dir_all(INSTALL_DIR);
    let mut done = "Imadive was uninstalled.".to_string();
    match (&data, delete_data) {
        (Some(dir), true) => {
            fs::remove_dir_all(dir).with_context(|| format!("deleting {}", dir.display()))?;
        }
        (Some(dir), false) => done.push_str(&format!(" The index is still in {}.", dir.display())),
        (None, _) => {}
    }
    if user_exists(SYSTEM_USER) {
        done.push_str(&format!(" The user {SYSTEM_USER} stays (sudo userdel {SYSTEM_USER} removes it)."));
    }
    if args.yes {
        println!("{done}");
    } else {
        cliclack::outro(done)?;
    }
    Ok(())
}

/// The service file an update installs: the one on disk when it was edited (by hand or in
/// setup), or else a fresh one for this version.
fn update_unit(current: &Settings) -> String {
    match fs::read_to_string(UNIT_PATH) {
        Ok(on_disk) if hand_edited(current) || current.edited => on_disk,
        _ => render_unit(current),
    }
}

/// Whether the service file was changed by hand after setup wrote it.
fn hand_edited(current: &Settings) -> bool {
    fs::read_to_string(UNIT_PATH).is_ok_and(|on_disk| !current.unit.is_empty() && on_disk != current.unit)
}

/// The service file for these settings.
fn render_unit(s: &Settings) -> String {
    let mut exec = vec![
        format!("{INSTALL_DIR}/imadive"),
        "--host".into(),
        if s.network { "0.0.0.0" } else { "127.0.0.1" }.into(),
        "--port".into(),
        s.port.to_string(),
        "--data".into(),
        s.data.to_string_lossy().into_owned(),
        "--models".into(),
        format!("{INSTALL_DIR}/models"),
        "--onnxruntime".into(),
        format!("{INSTALL_DIR}/onnxruntime"),
    ];
    if !s.faces {
        exec.push("--no-faces".into());
    }
    if (s.face_threshold - imadive::DEFAULT_FACE_THRESHOLD).abs() > f32::EPSILON {
        exec.extend(["--face-threshold".into(), s.face_threshold.to_string()]);
    }
    for host in &s.allow_hosts {
        exec.extend(["--allow-host".into(), host.clone()]);
    }
    exec.extend(s.folders.iter().map(|f| f.to_string_lossy().into_owned()));
    let exec: Vec<String> = exec.iter().map(|a| systemd_quote(a)).collect();
    let priority = if s.low_priority {
        "# Indexing uses every core; this keeps the server responsive.\nNice=10\nIOSchedulingClass=idle\n"
    } else {
        ""
    };
    format!(
        "# Written by `imadive setup`; run it again to change it. An update\n\
         # keeps edits made here; changing the settings in setup replaces them.\n\
         [Unit]\n\
         Description=Imadive photo gallery\n\
         # Waits for the network, and for network drives with photos on them.\n\
         After=network-online.target remote-fs.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         User={user}\n\
         WorkingDirectory={INSTALL_DIR}\n\
         ExecStart={exec}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         {priority}\
         # Small protections that don't get in the way: no privilege changes,\n\
         # its own /tmp, and /usr, /boot and /etc read-only.\n\
         NoNewPrivileges=true\n\
         PrivateTmp=true\n\
         ProtectSystem=full\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        user = s.user,
        exec = exec.join(" "),
    )
}

/// One ExecStart argument as systemd reads it: `%` and `$` doubled, and in quotes when it has
/// spaces or quotes.
fn systemd_quote(arg: &str) -> String {
    let escaped = arg.replace('%', "%%").replace('$', "$$");
    if escaped.is_empty() || escaped.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '\\') {
        format!("\"{}\"", escaped.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        escaped
    }
}

fn summary(s: &Settings, update: bool) -> String {
    let mut lines = Vec::new();
    if update {
        lines.push(format!("Version      {VERSION} (settings kept)"));
    }
    lines.push(format!("User         {}", s.user));
    lines.push(format!(
        "Photos       {}",
        if s.folders.is_empty() { "added later in Settings".into() } else { join_paths(&s.folders) }
    ));
    lines.push(format!("Index        {}", s.data.display()));
    lines.push(format!(
        "Open from    {}, port {}",
        if s.network { "any device on this network" } else { "this computer only" },
        s.port
    ));
    lines.push(format!("Faces        {}", if s.faces { "found and grouped" } else { "off" }));
    if !s.allow_hosts.is_empty() {
        lines.push(format!("Also named   {}", s.allow_hosts.join(", ")));
    }
    lines.push(format!("Program      {INSTALL_DIR}, service {UNIT_PATH}"));
    lines.join("\n")
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

/// The folder the running program is in: an unpacked release archive, or the install itself.
fn program_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    Ok(exe.parent().context("the program has no folder")?.to_path_buf())
}

/// Copies the program, its models and ONNX Runtime into the install folder. The program is
/// written beside and renamed over the old one, which a running copy may still have open.
fn install_files(source: &Path) -> Result<()> {
    let target = Path::new(INSTALL_DIR);
    fs::create_dir_all(target)?;
    let fresh = target.join("imadive.new");
    fs::copy(source.join("imadive"), &fresh).context("copying the program")?;
    fs::rename(&fresh, target.join("imadive"))?;
    for dir in ["models", "onnxruntime"] {
        let from = source.join(dir);
        if from.is_dir() {
            let to = target.join(dir);
            fs::create_dir_all(&to)?;
            for entry in fs::read_dir(&from)? {
                let entry = entry?;
                fs::copy(entry.path(), to.join(entry.file_name()))?;
            }
        }
    }
    for file in ["LICENSE", "THIRD_PARTY.md"] {
        if source.join(file).is_file() {
            fs::copy(source.join(file), target.join(file))?;
        }
    }
    if fs::symlink_metadata(LINK).is_ok() {
        fs::remove_file(LINK)?;
    }
    std::os::unix::fs::symlink(target.join("imadive"), LINK).with_context(|| format!("linking {LINK}"))?;
    Ok(())
}

/// Lets the user change the service file in their editor ($VISUAL, $EDITOR, nano or vi).
fn edit(unit: &str) -> Result<String> {
    let path = std::env::temp_dir().join(format!("imadive-{}.service", std::process::id()));
    fs::write(&path, unit)?;
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .ok()
        .filter(|e| !e.is_empty())
        .or_else(|| ["nano", "vi"].into_iter().find(|e| on_path(e)).map(str::to_string))
        .context("no editor found: set EDITOR")?;
    let status = Command::new("sh").arg("-c").arg(format!("{editor} \"$1\"")).arg("sh").arg(&path).status()?;
    let edited = fs::read_to_string(&path)?;
    let _ = fs::remove_file(&path);
    if !status.success() {
        bail!("the editor ended with {status}");
    }
    if edited != unit {
        cliclack::log::info("The service file will be installed as you edited it.")?;
    }
    Ok(edited)
}

/// Waits up to a minute for the gallery to answer on this computer.
fn wait_until_answering(port: u16) -> Result<()> {
    let start = Instant::now();
    loop {
        if let Ok(mut stream) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(2)) {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let request = format!("GET /api/status HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
            let mut answer = String::new();
            if stream.write_all(request.as_bytes()).is_ok()
                && stream.read_to_string(&mut answer).is_ok()
                && answer.starts_with("HTTP/1.")
                && answer.split_whitespace().nth(1) == Some("200")
            {
                return Ok(());
            }
        }
        if start.elapsed() > Duration::from_secs(60) {
            bail!("the gallery didn't answer on port {port} within a minute");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn run_cmd(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("running {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{:?} failed: {}", cmd.get_program(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn is_root() -> bool {
    Command::new("id").arg("-u").output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
}

fn user_exists(name: &str) -> bool {
    !name.is_empty() && Command::new("id").arg("-u").arg(name).output().is_ok_and(|o| o.status.success())
}

fn home_of(user: &str) -> Option<PathBuf> {
    let out = Command::new("getent").args(["passwd", user]).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout);
    line.trim().split(':').nth(5).filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// Whether `user` can list and read `folder` (checked as that user, with runuser).
fn readable_by(user: &str, folder: &Path) -> bool {
    if !user_exists(user) || !on_path("runuser") {
        return true; // can't tell yet (a user still to be created): don't warn
    }
    Command::new("runuser")
        .args(["-u", user, "--", "test", "-r"])
        .arg(folder)
        .args(["-a", "-x"])
        .arg(folder)
        .status()
        .is_ok_and(|s| s.success())
}

fn port_free(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_ok()
}

fn local_addresses() -> Vec<String> {
    let found = Command::new("hostname").arg("-I").output().ok().map(|o| {
        String::from_utf8_lossy(&o.stdout)
            .split_whitespace()
            .filter(|a| !a.contains(':')) // IPv4 is what people type
            .map(str::to_string)
            .collect::<Vec<_>>()
    });
    match found {
        Some(list) if !list.is_empty() => list,
        _ => vec!["<this server's address>".into()],
    }
}

fn ufw_active() -> bool {
    Command::new("ufw")
        .arg("status")
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("Status: active"))
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

fn program_name() -> String {
    std::env::args().next().unwrap_or_else(|| "imadive".into())
}

fn is_interrupted(e: &anyhow::Error) -> bool {
    e.downcast_ref::<io::Error>().is_some_and(|e| e.kind() == io::ErrorKind::Interrupted)
}

fn split_list(text: &str) -> Vec<String> {
    text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

fn split_paths(text: &str) -> Vec<PathBuf> {
    split_list(text).into_iter().map(PathBuf::from).collect()
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
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
