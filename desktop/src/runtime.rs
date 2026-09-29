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

/// Extracts the embedded files into `base/<app version>/`, skipping files already there.
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
    Ok(Runtime { models: dir, onnxruntime })
}

fn write_once(path: &Path, bytes: &[u8]) -> Result<()> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() == bytes.len() as u64) {
        return Ok(());
    }
    // Write next to the target and rename, so a crash never leaves a truncated file behind.
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}
