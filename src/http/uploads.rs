//! Uploading photos and videos from the browser (the server only: the desktop app's photos
//! are already on its computer). They go into an `imaDive-uploads` folder inside one of the
//! gallery's folders, keeping the folders they came in; the page starts a scan afterwards.

use std::path::{Component, Path, PathBuf};

use anyhow::Context;
use axum::Json;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use tokio::io::AsyncWriteExt;

use super::{ApiError, ApiResult, Shared, db};
use crate::{imaging, library, video};

/// The folder uploads go into, inside the gallery folder chosen.
pub(super) const UPLOADS: &str = "imaDive-uploads";

/// File types the gallery indexes here (what an upload may be).
fn accepted(path: &Path) -> bool {
    imaging::is_supported(path) || video::is_video(path)
}

/// The extensions the page may send (lowercase), so it skips the rest without uploading them.
fn extensions() -> Vec<&'static str> {
    let all = [
        "jpg", "jpeg", "png", "webp", "tif", "tiff", "gif", "bmp", "heic", "heif", "mp4", "m4v", "mov", "3gp", "webm",
        "mkv", "avi", "wmv", "mpg", "mpeg", "mts", "m2ts",
    ];
    all.into_iter().filter(|e| accepted(Path::new(&format!("x.{e}")))).collect()
}

fn not_on_desktop() -> ApiError {
    ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "uploading is for the gallery on a server; in the desktop app, add the folder in Settings",
    )
}

/// Where uploads can go (the gallery's folders that can be reached now) and what they may be.
pub(super) async fn targets(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    if s.host.is_some() {
        return Err(not_on_desktop());
    }
    let scan_cfg = s.scan.clone();
    let folders = db(&s, move |conn| scan_cfg.roots(conn)).await?;
    let folders: Vec<String> =
        folders.into_iter().filter(|f| can_receive(f)).map(|f| f.to_string_lossy().into_owned()).collect();
    Ok(Json(json!({ "folders": folders, "subfolder": UPLOADS, "extensions": extensions() })))
}

#[derive(Deserialize)]
pub(super) struct UploadQuery {
    /// One of the gallery's folders.
    folder: String,
    /// The file's path as it was chosen: `name.jpg`, or `Trip/day 1/name.jpg` for a folder.
    path: String,
}

/// One file, as the request's body. It is written beside its place and renamed into it once
/// complete, so a broken upload leaves nothing half written. A file of the same name that is
/// the same file is kept (`"same"`); a different one keeps its name and the new one gets a
/// number (`"renamed"`).
pub(super) async fn upload(
    State(s): State<Shared>,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> ApiResult<(StatusCode, Json<JsonValue>)> {
    if s.host.is_some() {
        return Err(not_on_desktop());
    }
    let relative = clean_relative(&q.path).ok_or_else(|| ApiError::bad_request("not a valid file name"))?;
    if !accepted(&relative) {
        return Err(ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "not a photo or video the gallery shows"));
    }
    let scan_cfg = s.scan.clone();
    let roots = db(&s, move |conn| scan_cfg.roots(conn)).await?;
    let root = roots
        .into_iter()
        .find(|r| r.to_string_lossy() == q.folder)
        .ok_or_else(|| ApiError::forbidden("that folder isn't in the gallery"))?;
    if !can_receive(&root) {
        return Err(ApiError::conflict(format!("{} can't be reached right now", root.display())));
    }

    let wanted = root.join(UPLOADS).join(&relative);
    let dir = wanted.parent().context("a file without a folder")?.to_path_buf();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::new(StatusCode::FORBIDDEN, format!("can't create {}: {e}", dir.display())))?;
    let name = wanted.file_name().context("a file without a name")?.to_string_lossy().into_owned();
    let partial = dir.join(format!(".{name}.{}.part", std::process::id()));
    let written = receive(body, &partial).await;
    let size = match written {
        Ok(size) => size,
        Err(e) => {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(e);
        }
    };

    let (status, place) = tokio::task::spawn_blocking(move || place_file(&partial, &wanted, size)).await??;
    let saved = place.strip_prefix(&root).unwrap_or(&place).to_string_lossy().into_owned();
    let code = if status == "same" { StatusCode::OK } else { StatusCode::CREATED };
    Ok((code, Json(json!({ "status": status, "path": saved }))))
}

/// Whether uploads may go into this gallery folder now. Elsewhere an empty folder counts as an
/// unplugged drive (its photos are kept, see `library::reachable`), but a new gallery's folder
/// is often empty, waiting for its first uploads. What must not happen is writing into the
/// empty folder a drive is mounted on when it isn't: on the system's disk, hidden once the
/// drive is back. So an empty folder takes uploads unless /etc/fstab mounts a drive there.
fn can_receive(folder: &Path) -> bool {
    library::reachable(folder) || (folder.is_dir() && !fstab_mount_point(folder, "/etc/fstab"))
}

/// Whether `fstab` (a file in /etc/fstab's format) mounts something on `folder` or above it
/// (but not the root).
fn fstab_mount_point(folder: &Path, fstab: &str) -> bool {
    let Ok(table) = std::fs::read_to_string(fstab) else { return false };
    table
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| line.split_whitespace().nth(1))
        .map(|point| point.replace("\\040", " "))
        .any(|point| point != "/" && point != "none" && folder.starts_with(&point))
}

/// Streams the body into `path`; its size.
async fn receive(body: Body, path: &Path) -> Result<u64, ApiError> {
    let mut file = tokio::fs::File::create(path)
        .await
        .map_err(|e| ApiError::new(StatusCode::FORBIDDEN, format!("can't write in {}: {e}", display_dir(path))))?;
    let mut stream = body.into_data_stream();
    let mut size = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| ApiError::bad_request(format!("the upload was interrupted: {e}")))?;
        size += chunk.len() as u64;
        file.write_all(&chunk).await.context("writing the upload")?;
    }
    file.flush().await?;
    file.sync_all().await?;
    if size == 0 {
        return Err(ApiError::bad_request("the file is empty"));
    }
    Ok(size)
}

/// Moves the received file to its place: `"saved"` there, `"same"` when that file is already
/// there, `"renamed"` (with a number) when another file has the name.
fn place_file(partial: &Path, wanted: &Path, size: u64) -> anyhow::Result<(&'static str, PathBuf)> {
    let mut target = wanted.to_path_buf();
    let mut n = 1;
    loop {
        match std::fs::metadata(&target) {
            Err(_) => {
                std::fs::rename(partial, &target)?;
                return Ok((if n == 1 { "saved" } else { "renamed" }, target));
            }
            Ok(meta) if meta.len() == size && same_content(partial, &target)? => {
                std::fs::remove_file(partial)?;
                return Ok(("same", target));
            }
            Ok(_) => {
                n += 1;
                let stem = wanted.file_stem().unwrap_or_default().to_string_lossy();
                let name = match wanted.extension() {
                    Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
                    None => format!("{stem} ({n})"),
                };
                target = wanted.with_file_name(name);
            }
        }
    }
}

fn same_content(a: &Path, b: &Path) -> anyhow::Result<bool> {
    let hash = |p: &Path| -> anyhow::Result<blake3::Hash> {
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(std::fs::File::open(p)?)?;
        Ok(hasher.finalize())
    };
    Ok(hash(a)? == hash(b)?)
}

/// A path the browser sent, made safe to put under the uploads folder: plain names only
/// (no `..`, no root, no hidden or empty parts, no backslashes), at most 16 deep.
fn clean_relative(path: &str) -> Option<PathBuf> {
    if path.contains('\\') || path.contains('\0') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        let component = Path::new(part).components().next()?;
        if !matches!(component, Component::Normal(_)) || part.starts_with('.') || part.len() > 255 {
            return None;
        }
        out.push(part);
    }
    let depth = out.components().count();
    (1..=16).contains(&depth).then_some(out)
}

fn display_dir(path: &Path) -> String {
    path.parent().map(|p| p.display().to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_from_the_browser_stay_inside_the_uploads_folder() {
        assert_eq!(clean_relative("a.jpg"), Some(PathBuf::from("a.jpg")));
        assert_eq!(clean_relative("Trip/day 1/a.jpg"), Some(PathBuf::from("Trip/day 1/a.jpg")));
        assert_eq!(clean_relative("/Trip//a.jpg"), Some(PathBuf::from("Trip/a.jpg")));
        for bad in ["", "../a.jpg", "Trip/../../a.jpg", "./a.jpg", ".hidden/a.jpg", "a\\b.jpg", "a\0.jpg"] {
            assert_eq!(clean_relative(bad), None, "{bad:?}");
        }
        assert!(clean_relative(&format!("{}a.jpg", "d/".repeat(16))).is_none(), "too deep");
    }

    #[test]
    fn an_empty_folder_takes_uploads_unless_a_drive_is_mounted_there() {
        let dir = std::env::temp_dir().join(format!("imadive-fstab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fstab = dir.join("fstab");
        std::fs::write(
            &fstab,
            "# comment\nUUID=1 / ext4 defaults 0 1\n//nas/photos /mnt/nas cifs defaults 0 0\n/dev/sdb1 /media/My\\040Disk ext4 defaults 0 2\n",
        )
        .unwrap();
        let fstab = fstab.to_str().unwrap();
        assert!(fstab_mount_point(Path::new("/mnt/nas"), fstab));
        assert!(fstab_mount_point(Path::new("/mnt/nas/photos"), fstab));
        assert!(fstab_mount_point(Path::new("/media/My Disk/photos"), fstab));
        assert!(!fstab_mount_point(Path::new("/srv/photos"), fstab), "the root's entry doesn't count");
        assert!(!fstab_mount_point(Path::new("/mnt/nasty"), fstab));
        assert!(!fstab_mount_point(Path::new("/srv/photos"), "/no/such/fstab"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_already_there_is_kept_and_a_different_one_gets_a_number() {
        let dir = std::env::temp_dir().join(format!("imadive-upload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wanted = dir.join("a.jpg");
        let part = |content: &[u8]| {
            let p = dir.join(".part");
            std::fs::write(&p, content).unwrap();
            p
        };
        assert_eq!(place_file(&part(b"one"), &wanted, 3).unwrap(), ("saved", wanted.clone()));
        assert_eq!(place_file(&part(b"one"), &wanted, 3).unwrap(), ("same", wanted.clone()));
        assert_eq!(place_file(&part(b"two"), &wanted, 3).unwrap(), ("renamed", dir.join("a (2).jpg")));
        assert_eq!(place_file(&part(b"two"), &wanted, 3).unwrap(), ("same", dir.join("a (2).jpg")));
        assert!(!dir.join(".part").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
