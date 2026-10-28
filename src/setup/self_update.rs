//! `imadive self-update`: the latest release installed as root, when the gallery asks.

use super::*;

/// `imadive self-update`, run by imadive-update.service: downloads the latest server release
/// for this computer, checks it, and installs it with its own `setup --yes`, which keeps the
/// settings and restarts the gallery. How it went is written to `update-status.json` in the
/// data folder, for the gallery to show.
pub fn self_update() -> Result<()> {
    if !is_root() {
        bail!("self-update installs a release, so it runs as root (imadive-update.service starts it)");
    }
    let settings = installed_settings().context("Imadive isn't installed here with setup")?;
    let status_file = settings.data.join(imadive::update::STATUS_FILE);
    let report = |state: &str, error: Option<String>| {
        let _ = fs::write(&status_file, serde_json::json!({ "state": state, "error": error }).to_string());
    };
    report("installing", None);
    let work = std::env::temp_dir().join(format!("imadive-update-{}", std::process::id()));
    let result = (|| -> Result<Option<String>> {
        let latest = imadive::update::latest_version()?;
        if !imadive::update::is_newer(&latest, VERSION) {
            return Ok(None);
        }
        fs::create_dir_all(&work)?;
        let name = imadive::update::server_asset(&latest);
        let archive = work.join(&name);
        println!("Downloading {name}...");
        imadive::update::download(&latest, &name, &archive, &|_, _| {})?;
        run_cmd(Command::new("tar").arg("-xzf").arg(&archive).arg("-C").arg(&work))?;
        let program = work.join(format!("imadive-{latest}")).join("imadive");
        run_cmd(Command::new(&program).args(["setup", "--yes"]))?;
        Ok(Some(latest))
    })();
    let _ = fs::remove_dir_all(&work);
    match result {
        Ok(Some(version)) => {
            println!("Updated to {version}");
            report("done", None);
            Ok(())
        }
        Ok(None) => {
            println!("{VERSION} is the latest version");
            report("done", None);
            Ok(())
        }
        Err(e) => {
            report("error", Some(format!("{e:#}")));
            Err(e)
        }
    }
}
