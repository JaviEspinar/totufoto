//! The project's releases on GitHub: which is the latest, and downloading its files, each
//! checked against the release's checksums.

use super::*;

/// GitHub, or a test server standing in for it (`IMADIVE_UPDATE_BASE`).
pub(super) fn base() -> String {
    std::env::var("IMADIVE_UPDATE_BASE").unwrap_or_else(|_| "https://github.com".into())
}

/// The page listing the releases.
pub fn releases_url() -> String {
    format!("https://github.com/{REPO}/releases")
}

/// "1.2.3" as numbers, to compare versions ("0.3.10" is newer than "0.3.9").
pub(super) fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// The version at the end of the address `releases/latest` redirects to (`.../tag/v0.3.3`).
pub(super) fn version_from_location(location: &str) -> Option<String> {
    let tag = location.trim_end_matches('/').rsplit('/').next()?;
    let version = tag.strip_prefix('v').unwrap_or(tag);
    parse_version(version).map(|_| version.to_string())
}

pub(super) fn agent(redirects: u32) -> ureq::Agent {
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

pub(super) fn latest_version_at(base: &str) -> Result<String> {
    let url = format!("{base}/{REPO}/releases/latest");
    let response = agent(0).get(&url).call().context("couldn't reach GitHub")?;
    let location = response
        .headers()
        .get("location")
        .and_then(|l| l.to_str().ok())
        .with_context(|| format!("GitHub didn't say which release is the latest ({})", response.status()))?;
    version_from_location(location).with_context(|| format!("couldn't tell the latest version from {location}"))
}

pub(super) fn release_file(base: &str, version: &str, name: &str) -> String {
    format!("{base}/{REPO}/releases/download/v{version}/{name}")
}

/// The checksum `SHA256SUMS` gives for `name` (lines as `sha256sum` writes them).
pub(super) fn checksum_for(sums: &str, name: &str) -> Option<String> {
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

pub(super) fn download_at(
    base: &str,
    version: &str,
    name: &str,
    dest: &Path,
    progress: &dyn Fn(u64, u64),
) -> Result<()> {
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
