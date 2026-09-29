use std::path::Path;

fn main() {
    // The face models and ONNX Runtime are embedded in the executable (see src/runtime.rs).
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let ort = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => "onnxruntime/onnxruntime.dll",
        Ok("macos") => "onnxruntime/libonnxruntime.dylib",
        _ => "onnxruntime/libonnxruntime.so",
    };
    let mut files: Vec<String> = ["models/det_500m.onnx", "models/w600k_mbf.onnx", ort].map(String::from).into();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // Visual C++ runtime needed by onnxruntime.dll (copied by scripts/fetch-onnxruntime.ps1).
        for dll in ["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "msvcp140_1.dll"] {
            files.push(format!("onnxruntime/{dll}"));
        }
    }
    for file in &files {
        let path = root.join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        if !path.is_file() {
            panic!(
                "{file} is missing. Run scripts/fetch-models.sh and scripts/fetch-onnxruntime.sh \
                 (or the .ps1 versions on Windows) from the project folder first."
            );
        }
    }
    tauri_build::build();
}
