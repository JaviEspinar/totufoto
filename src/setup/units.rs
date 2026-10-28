//! The systemd units setup writes: the gallery's service, and the units that install updates.

use super::*;

/// The service file an update installs: the one on disk when it was edited (by hand or in
/// setup), or else a fresh one for this version.
pub(super) fn update_unit(current: &Settings) -> String {
    match fs::read_to_string(UNIT_PATH) {
        Ok(on_disk) if hand_edited(current) || current.edited => on_disk,
        _ => render_unit(current),
    }
}

/// Whether the service file was changed by hand after setup wrote it.
pub(super) fn hand_edited(current: &Settings) -> bool {
    fs::read_to_string(UNIT_PATH).is_ok_and(|on_disk| !current.unit.is_empty() && on_disk != current.unit)
}

/// The service file for these settings.
pub(super) fn render_unit(s: &Settings) -> String {
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

/// The units that install an update when the gallery asks: the gallery writes
/// `update-request` in its data folder, which starts `imadive self-update` as root. The
/// request says nothing: root only ever installs the latest release, checked against its
/// checksums.
pub(super) fn render_update_units(data: &Path) -> (String, String) {
    // systemd is Linux-only: the path always takes `/` (also when the tests run on Windows).
    let request = format!("{}/{}", data.display(), imadive::update::REQUEST_FILE).replace('%', "%%");
    let path_unit = format!(
        "# Written by `imadive setup`: installs an update when the gallery asks for one.\n\
         [Unit]\n\
         Description=Imadive updates asked for from the gallery\n\
         \n\
         [Path]\n\
         PathChanged={request}\n\
         Unit=imadive-update.service\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n"
    );
    let service = format!(
        "# Written by `imadive setup`: installs the latest release (see imadive-update.path).\n\
         [Unit]\n\
         Description=Imadive update: installs the latest release\n\
         \n\
         [Service]\n\
         Type=oneshot\n\
         ExecStart={INSTALL_DIR}/imadive self-update\n"
    );
    (path_unit, service)
}

/// One ExecStart argument as systemd reads it: `%` and `$` doubled, and in quotes when it has
/// spaces or quotes.
pub(super) fn systemd_quote(arg: &str) -> String {
    let escaped = arg.replace('%', "%%").replace('$', "$$");
    if escaped.is_empty() || escaped.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '\\') {
        format!("\"{}\"", escaped.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        escaped
    }
}
