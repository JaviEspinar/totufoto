//! Face models and ONNX Runtime embedded in the executable, written to the app data
//! folder on first run so the app works without any setup.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

static DETECTOR: &[u8] = include_bytes!("../../models/det_500m.onnx");
static EMBEDDER: &[u8] = include_bytes!("../../models/w600k_mbf.onnx");

#[cfg(target_os = "windows")]
const ORT: (&str, &[u8]) = ("onnxruntime.dll", include_bytes!("../../onnxruntime/onnxruntime.dll"));
#[cfg(target_os = "macos")]
const ORT: (&str, &[u8]) = ("libonnxruntime.dylib", include_bytes!("../../onnxruntime/libonnxruntime.dylib"));
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const ORT: (&str, &[u8]) = ("libonnxruntime.so", include_bytes!("../../onnxruntime/libonnxruntime.so"));

/// Visual C++ runtime for onnxruntime.dll, so the app runs without the VC++ redistributable.
#[cfg(target_os = "windows")]
static VC_RUNTIME: &[(&str, &[u8])] = &[
    ("vcruntime140.dll", include_bytes!("../../onnxruntime/vcruntime140.dll")),
    ("vcruntime140_1.dll", include_bytes!("../../onnxruntime/vcruntime140_1.dll")),
    ("msvcp140.dll", include_bytes!("../../onnxruntime/msvcp140.dll")),
    ("msvcp140_1.dll", include_bytes!("../../onnxruntime/msvcp140_1.dll")),
];

pub struct Runtime {
    pub models: PathBuf,
    pub onnxruntime: PathBuf,
}

/// Extracts the embedded files into `base/<app version>/`, skipping files already there,
/// then deletes the folders of other versions (a few hundred MB after some updates).
pub fn install(base: &Path) -> Result<Runtime> {
    let dir = base.join(env!("CARGO_PKG_VERSION"));
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    write_once(&dir.join("det_500m.onnx"), DETECTOR)?;
    write_once(&dir.join("w600k_mbf.onnx"), EMBEDDER)?;
    #[cfg(target_os = "windows")]
    for (name, bytes) in VC_RUNTIME {
        write_once(&dir.join(name), bytes)?;
    }
    let onnxruntime = dir.join(ORT.0);
    write_once(&onnxruntime, ORT.1)?;
    remove_other_versions(base, &dir);
    Ok(Runtime { models: dir, onnxruntime })
}

/// Deletes every folder in `base` except `keep`. One that can't be deleted (in use by an
/// older copy of the app that is still running, say) is left for the next start.
fn remove_other_versions(base: &Path, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path == keep || !path.is_dir() {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => tracing::info!("deleted the files of an older version: {}", path.display()),
            Err(e) => tracing::debug!("couldn't delete {} yet: {e}", path.display()),
        }
    }
}

/// Writes the file unless it is already there with exactly these bytes. Comparing the
/// contents (tens of MB, read in milliseconds) also repairs a damaged or tampered file.
fn write_once(path: &Path, bytes: &[u8]) -> Result<()> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() == bytes.len() as u64)
        && std::fs::read(path).is_ok_and(|on_disk| on_disk == bytes)
    {
        return Ok(());
    }
    // Write next to the target and rename, so a crash never leaves a truncated file behind.
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("imadive-runtime-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_damaged_file_is_written_again() {
        let dir = temp("write");
        let file = dir.join("model.onnx");
        write_once(&file, b"model bytes").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"model bytes");
        // Same length, different contents: a length check alone would keep it.
        std::fs::write(&file, b"MODEL BYTES").unwrap();
        write_once(&file, b"model bytes").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"model bytes");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn older_versions_are_deleted() {
        let base = temp("versions");
        for v in ["0.1.9", "0.1.10"] {
            std::fs::create_dir_all(base.join(v)).unwrap();
            std::fs::write(base.join(v).join("libonnxruntime.so"), b"old").unwrap();
        }
        let rt = install(&base).unwrap();
        let left: Vec<_> = std::fs::read_dir(&base).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(left, vec![base.join(env!("CARGO_PKG_VERSION"))]);
        assert!(rt.onnxruntime.is_file());
        std::fs::remove_dir_all(&base).unwrap();
    }
}
