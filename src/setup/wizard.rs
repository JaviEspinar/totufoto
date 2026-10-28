//! The wizard's questions, its summary of what it will do, and editing the service file.

use super::*;

pub(super) fn wizard(existing: Option<Settings>, args: &SetupArgs) -> Result<()> {
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
pub(super) fn ask(base: Settings) -> Result<Settings> {
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
pub(super) fn confirm_and_apply(mut settings: Settings, existing: Option<&Settings>, update: bool) -> Result<()> {
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

pub(super) fn summary(s: &Settings, update: bool) -> String {
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

/// Lets the user change the service file in their editor ($VISUAL, $EDITOR, nano or vi).
pub(super) fn edit(unit: &str) -> Result<String> {
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

pub(super) fn is_interrupted(e: &anyhow::Error) -> bool {
    e.downcast_ref::<io::Error>().is_some_and(|e| e.kind() == io::ErrorKind::Interrupted)
}

pub(super) fn split_list(text: &str) -> Vec<String> {
    text.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

pub(super) fn split_paths(text: &str) -> Vec<PathBuf> {
    split_list(text).into_iter().map(PathBuf::from).collect()
}

pub(super) fn join_paths(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>().join(", ")
}
