//! Doing it: the program and its files, the services, libheif, and uninstalling.

use super::*;

/// Installs (or updates) the program, the service and its folders, then starts it and waits
/// for the gallery to answer.
pub(super) fn apply(settings: &Settings, unit: &str, existing: Option<&Settings>) -> Result<()> {
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
    // iPhone photos (HEIC) are decoded with the system's libheif: installed when missing, if
    // the system has apt. Not having it only leaves those photos out, so a failure here doesn't
    // stop the install.
    if cfg!(target_os = "linux") && !imadive::heic_libheif_available() && on_path("apt-get") {
        let _ = step("Installing libheif, for iPhone photos (HEIC)", &mut install_libheif);
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
    step("Writing the update service", &mut || {
        let (path_unit, service) = render_update_units(&settings.data);
        fs::write(imadive::update::HELPER_PATH_UNIT, path_unit)?;
        fs::write(UPDATE_SERVICE_PATH, service)?;
        run_cmd(Command::new("systemctl").arg("daemon-reload"))?;
        run_cmd(Command::new("systemctl").args(["enable", "--now", "imadive-update.path"]))
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

pub(super) fn uninstall(existing: Option<&Settings>, args: &SetupArgs) -> Result<()> {
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
    let _ = Command::new("systemctl").args(["disable", "--now", "imadive", "imadive-update.path"]).status();
    for unit in [UNIT_PATH, imadive::update::HELPER_PATH_UNIT, UPDATE_SERVICE_PATH] {
        let _ = fs::remove_file(unit);
    }
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

/// The folder the running program is in: an unpacked release archive, or the install itself.
pub(super) fn program_dir() -> Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    Ok(exe.parent().context("the program has no folder")?.to_path_buf())
}

/// Copies the program, its models and ONNX Runtime into the install folder. The program is
/// written beside and renamed over the old one, which a running copy may still have open.
pub(super) fn install_files(source: &Path) -> Result<()> {
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
    #[cfg(unix)]
    std::os::unix::fs::symlink(target.join("imadive"), LINK).with_context(|| format!("linking {LINK}"))?;
    Ok(())
}

/// Installs libheif with apt, with its HEVC plugin where the system splits it out (Ubuntu
/// 24.04 and newer, Debian 13); the package lists are refreshed if the first try fails.
pub(super) fn install_libheif() -> Result<()> {
    let install = || -> Result<()> {
        let plugin = Command::new("apt-cache")
            .args(["show", "libheif-plugin-libde265"])
            .output()
            .is_ok_and(|o| o.status.success() && !o.stdout.is_empty());
        let mut cmd = Command::new("apt-get");
        cmd.env("DEBIAN_FRONTEND", "noninteractive").args(["install", "-y", "-qq", "libheif1"]);
        if plugin {
            cmd.arg("libheif-plugin-libde265");
        }
        run_cmd(&mut cmd)
    };
    install().or_else(|_| {
        run_cmd(Command::new("apt-get").args(["update", "-qq"]))?;
        install()
    })
}

/// Waits up to a minute for the gallery to answer on this computer.
pub(super) fn wait_until_answering(port: u16) -> Result<()> {
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
