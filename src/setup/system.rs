//! Small questions to the system: commands, users, ports, addresses, the firewall.

use super::*;

pub(super) fn run_cmd(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("running {:?}", cmd.get_program()))?;
    if !out.status.success() {
        bail!("{:?} failed: {}", cmd.get_program(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

pub(super) fn is_root() -> bool {
    Command::new("id").arg("-u").output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
}

pub(super) fn user_exists(name: &str) -> bool {
    !name.is_empty() && Command::new("id").arg("-u").arg(name).output().is_ok_and(|o| o.status.success())
}

pub(super) fn home_of(user: &str) -> Option<PathBuf> {
    let out = Command::new("getent").args(["passwd", user]).output().ok()?;
    let line = String::from_utf8_lossy(&out.stdout);
    line.trim().split(':').nth(5).filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// Whether `user` can list and read `folder` (checked as that user, with runuser).
pub(super) fn readable_by(user: &str, folder: &Path) -> bool {
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

pub(super) fn port_free(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_ok()
}

pub(super) fn local_addresses() -> Vec<String> {
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

pub(super) fn ufw_active() -> bool {
    Command::new("ufw")
        .arg("status")
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("Status: active"))
}

pub(super) fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

pub(super) fn program_name() -> String {
    std::env::args().next().unwrap_or_else(|| "imadive".into())
}
