//! Installing a release, for each kind of copy that can update itself.

use super::*;

#[cfg(unix)]
pub(super) fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// The Windows app: the new program downloaded next to it, then put in its place.
pub(super) fn windows_exe(updates: &Updates, version: &str, asset: &str, progress: &dyn Fn(u64, u64)) -> Result<()> {
    let exe = std::env::current_exe()?;
    let fresh = exe.with_extension("exe.new");
    download(version, asset, &fresh, progress)?;
    updates.set_state("installing", None);
    replace_running(&exe, &fresh)
}

/// The AppImage: the new one downloaded next to it, then renamed over it.
pub(super) fn app_image(updates: &Updates, version: &str, asset: &str, progress: &dyn Fn(u64, u64)) -> Result<()> {
    let target = PathBuf::from(std::env::var_os("APPIMAGE").context("APPIMAGE isn't set")?);
    let fresh = target.with_extension("AppImage.new");
    download(version, asset, &fresh, progress)?;
    updates.set_state("installing", None);
    make_executable(&fresh)?;
    std::fs::rename(&fresh, &target).with_context(|| format!("replacing {}", target.display()))
}

/// The .deb: the new package downloaded to the temporary folder and installed.
pub(super) fn deb(updates: &Updates, version: &str, asset: &str, progress: &dyn Fn(u64, u64)) -> Result<()> {
    let package = std::env::temp_dir().join(asset);
    download(version, asset, &package, progress)?;
    updates.set_state("installing", None);
    install_deb(&package)?;
    let _ = std::fs::remove_file(&package);
    Ok(())
}

/// A server installed with `imadive setup`: the root update service is asked to install it
/// (see `imadive self-update`), and reports in [`STATUS_FILE`].
pub(super) fn ask_the_update_service(updates: &Updates, version: &str) -> Result<()> {
    let _ = std::fs::remove_file(updates.data_dir.join(STATUS_FILE));
    std::fs::write(updates.data_dir.join(REQUEST_FILE), chrono::Local::now().to_rfc3339())
        .context("asking the update service")?;
    tracing::info!("asked the update service to install {version}");
    Ok(())
}

/// Puts `fresh` in place of the running program `exe`. Windows can't overwrite a running
/// program, but it can rename it: the old one becomes `.old`, deleted on the next start.
pub(super) fn replace_running(exe: &Path, fresh: &Path) -> Result<()> {
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

/// Installs a .deb with apt, which asks for the password in a system dialog (pkexec).
pub(super) fn install_deb(package: &Path) -> Result<()> {
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
pub(super) fn helper_error(data_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(data_dir.join(STATUS_FILE)).ok()?;
    let status: serde_json::Value = serde_json::from_str(&text).ok()?;
    (status["state"] == "error").then(|| status["error"].as_str().unwrap_or("the update failed").to_string())
}
