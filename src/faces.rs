//! Face detection (SCRFD) and face embeddings (ArcFace / MobileFaceNet) on ONNX Runtime.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use image::RgbImage;
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::value::Tensor;

const DET_SIZE: u32 = 640;
const DET_THRESHOLD: f32 = 0.55;
const NMS_IOU: f32 = 0.4;
/// Faces smaller than this (in pixels of the working image) give unreliable embeddings.
pub const MIN_FACE_PX: f32 = 28.0;
pub const EMBEDDING_DIM: usize = 512;

/// Landmark template used by ArcFace for a 112x112 aligned crop.
const ARCFACE_DST: [[f32; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.5014],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.2041],
];

#[derive(Debug, Clone)]
pub struct Detection {
    /// x1, y1, x2, y2 in image pixels.
    pub bbox: [f32; 4],
    pub score: f32,
    pub kps: [[f32; 2]; 5],
}

pub struct FaceModels {
    detector: Session,
    embedder: Session,
}

#[derive(Clone)]
pub struct ModelPaths {
    pub detector: PathBuf,
    pub embedder: PathBuf,
}

impl ModelPaths {
    pub fn in_dir(dir: &Path) -> Self {
        Self { detector: dir.join("det_500m.onnx"), embedder: dir.join("w600k_mbf.onnx") }
    }

    pub fn exist(&self) -> bool {
        self.detector.is_file() && self.embedder.is_file()
    }
}

#[cfg(target_os = "windows")]
const ORT_LIBRARY: &str = "onnxruntime.dll";
#[cfg(target_os = "macos")]
const ORT_LIBRARY: &str = "libonnxruntime.dylib";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const ORT_LIBRARY: &str = "libonnxruntime.so";

/// Places searched for the ONNX Runtime library, most specific first.
/// A path given explicitly is the only one tried.
fn runtime_candidates(explicit: Option<&Path>) -> Vec<PathBuf> {
    if let Some(p) = explicit {
        return vec![if p.is_dir() { p.join(ORT_LIBRARY) } else { p.to_path_buf() }];
    }
    let mut out = Vec::new();
    if let Some(p) = std::env::var_os("ORT_DYLIB_PATH") {
        out.push(PathBuf::from(p));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        out.push(dir.join(ORT_LIBRARY));
        out.push(dir.join("onnxruntime").join(ORT_LIBRARY));
    }
    // Where scripts/fetch-onnxruntime.* puts it when running from the project folder.
    out.push(PathBuf::from("onnxruntime").join(ORT_LIBRARY));
    out
}

/// Loads the ONNX Runtime shared library. Must succeed before any model is loaded.
pub fn init_runtime(explicit: Option<&Path>) -> Result<PathBuf> {
    let candidates = runtime_candidates(explicit);
    let path = candidates
        .iter()
        .find(|p| p.is_file())
        .ok_or_else(|| anyhow!("{ORT_LIBRARY} not found (looked in: {})", display_list(&candidates)))?;
    let committed = ort::init_from(path).map_err(|e| anyhow!("loading {}: {e}", path.display()))?.commit();
    if !committed {
        tracing::debug!("ONNX Runtime environment was already configured");
    }
    Ok(path.clone())
}

fn display_list(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
}

/// SIMD level the CPU offers, for the startup log.
pub fn cpu_features() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        let f = [
            ("avx512f", std::arch::is_x86_feature_detected!("avx512f")),
            ("avx2", std::arch::is_x86_feature_detected!("avx2")),
            ("fma", std::arch::is_x86_feature_detected!("fma")),
            ("avx", std::arch::is_x86_feature_detected!("avx")),
            ("sse4.1", std::arch::is_x86_feature_detected!("sse4.1")),
        ];
        f.iter().map(|(n, on)| format!("{n}={}", if *on { "yes" } else { "no" })).collect::<Vec<_>>().join(" ")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        std::env::consts::ARCH.to_string()
    }
}

fn session(path: &Path) -> Result<Session> {
    Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| anyhow!("{e}"))?
        // Parallelism comes from processing many photos at once, one session per worker thread.
        .with_intra_threads(1)
        .map_err(|e| anyhow!("{e}"))?
        .commit_from_file(path)
        .with_context(|| format!("loading model {}", path.display()))
}

impl FaceModels {
    pub fn load(paths: &ModelPaths) -> Result<Self> {
        Ok(Self { detector: session(&paths.detector)?, embedder: session(&paths.embedder)? })
    }

    pub fn detect(&mut self, img: &RgbImage) -> Result<Vec<Detection>> {
        let (w, h) = img.dimensions();
        // Letterbox into DET_SIZE x DET_SIZE, keeping the aspect ratio (padding right/bottom).
        let scale = (DET_SIZE as f32 / w as f32).min(DET_SIZE as f32 / h as f32);
        let nw = ((w as f32 * scale).round() as u32).clamp(1, DET_SIZE);
        let nh = ((h as f32 * scale).round() as u32).clamp(1, DET_SIZE);
        let resized = crate::imaging::resize_rgb(img, nw, nh)?;

        let plane = (DET_SIZE * DET_SIZE) as usize;
        let mut input = vec![-127.5f32 / 128.0; 3 * plane];
        for (x, y, p) in resized.enumerate_pixels() {
            let i = (y * DET_SIZE + x) as usize;
            for c in 0..3 {
                input[c * plane + i] = (p[c] as f32 - 127.5) / 128.0;
            }
        }
        let tensor = Tensor::from_array(([1usize, 3, DET_SIZE as usize, DET_SIZE as usize], input))?;
        let outputs = self.detector.run(ort::inputs![tensor])?;

        let mut dets = Vec::new();
        for (level, stride) in [8u32, 16, 32].into_iter().enumerate() {
            let (_, scores) = outputs[level].try_extract_tensor::<f32>()?;
            let (_, boxes) = outputs[level + 3].try_extract_tensor::<f32>()?;
            let (_, kps) = outputs[level + 6].try_extract_tensor::<f32>()?;
            let fw = DET_SIZE / stride;
            let s = stride as f32;
            for (idx, &score) in scores.iter().enumerate() {
                if score < DET_THRESHOLD {
                    continue;
                }
                let cell = idx as u32 / 2; // two anchors per location
                let cx = (cell % fw) as f32 * s;
                let cy = (cell / fw) as f32 * s;
                let b = &boxes[idx * 4..idx * 4 + 4];
                let k = &kps[idx * 10..idx * 10 + 10];
                let mut landmarks = [[0f32; 2]; 5];
                for (j, lm) in landmarks.iter_mut().enumerate() {
                    *lm = [(cx + k[2 * j] * s) / scale, (cy + k[2 * j + 1] * s) / scale];
                }
                dets.push(Detection {
                    bbox: [
                        (cx - b[0] * s) / scale,
                        (cy - b[1] * s) / scale,
                        (cx + b[2] * s) / scale,
                        (cy + b[3] * s) / scale,
                    ],
                    score,
                    kps: landmarks,
                });
            }
        }
        Ok(nms(dets))
    }

    /// Returns one L2-normalised embedding per detection.
    pub fn embed(&mut self, img: &RgbImage, dets: &[Detection]) -> Result<Vec<Vec<f32>>> {
        if dets.is_empty() {
            return Ok(Vec::new());
        }
        const S: usize = 112;
        let plane = S * S;
        let mut input = vec![0f32; dets.len() * 3 * plane];
        for (n, det) in dets.iter().enumerate() {
            let crop = align(img, &det.kps);
            let base = n * 3 * plane;
            for (x, y, p) in crop.enumerate_pixels() {
                let i = y as usize * S + x as usize;
                for c in 0..3 {
                    input[base + c * plane + i] = (p[c] as f32 - 127.5) / 127.5;
                }
            }
        }
        let tensor = Tensor::from_array(([dets.len(), 3, S, S], input))?;
        let outputs = self.embedder.run(ort::inputs![tensor])?;
        let (_, data) = outputs[0].try_extract_tensor::<f32>()?;
        Ok(data
            .chunks_exact(EMBEDDING_DIM)
            .map(|v| {
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
                v.iter().map(|x| x / norm).collect()
            })
            .collect())
    }
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let area = |r: &[f32; 4]| (r[2] - r[0]) * (r[3] - r[1]);
    inter / (area(a) + area(b) - inter).max(1e-6)
}

fn nms(mut dets: Vec<Detection>) -> Vec<Detection> {
    dets.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut keep: Vec<Detection> = Vec::new();
    for d in dets {
        if keep.iter().all(|k| iou(&k.bbox, &d.bbox) < NMS_IOU) {
            keep.push(d);
        }
    }
    keep
}

/// Warps the face into the 112x112 ArcFace template with a least-squares similarity transform.
fn align(img: &RgbImage, kps: &[[f32; 2]; 5]) -> RgbImage {
    let n = kps.len() as f32;
    let (mut msx, mut msy, mut mdx, mut mdy) = (0f32, 0f32, 0f32, 0f32);
    for (s, d) in kps.iter().zip(ARCFACE_DST.iter()) {
        msx += s[0];
        msy += s[1];
        mdx += d[0];
        mdy += d[1];
    }
    msx /= n;
    msy /= n;
    mdx /= n;
    mdy /= n;
    // dst = [[a, -b], [b, a]] * src + t  (scale * rotation, solved in closed form)
    let (mut num_a, mut num_b, mut var) = (0f32, 0f32, 0f32);
    for (s, d) in kps.iter().zip(ARCFACE_DST.iter()) {
        let (sx, sy) = (s[0] - msx, s[1] - msy);
        let (dx, dy) = (d[0] - mdx, d[1] - mdy);
        num_a += sx * dx + sy * dy;
        num_b += sx * dy - sy * dx;
        var += sx * sx + sy * sy;
    }
    let var = var.max(1e-6);
    let (a, b) = (num_a / var, num_b / var);
    let (tx, ty) = (mdx - (a * msx - b * msy), mdy - (b * msx + a * msy));
    // Inverse mapping: src = M^-1 (dst - t)
    let det = (a * a + b * b).max(1e-12);
    let (w, h) = img.dimensions();
    RgbImage::from_fn(112, 112, |u, v| {
        let (dx, dy) = (u as f32 - tx, v as f32 - ty);
        let sx = (a * dx + b * dy) / det;
        let sy = (-b * dx + a * dy) / det;
        bilinear(img, sx, sy, w, h)
    })
}

fn bilinear(img: &RgbImage, x: f32, y: f32, w: u32, h: u32) -> image::Rgb<u8> {
    if x < 0.0 || y < 0.0 || x > (w - 1) as f32 || y > (h - 1) as f32 {
        return image::Rgb([0, 0, 0]);
    }
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let (p00, p10, p01, p11) = (img.get_pixel(x0, y0), img.get_pixel(x1, y0), img.get_pixel(x0, y1), img.get_pixel(x1, y1));
    let mut out = [0u8; 3];
    for c in 0..3 {
        let top = p00[c] as f32 * (1.0 - fx) + p10[c] as f32 * fx;
        let bottom = p01[c] as f32 * (1.0 - fx) + p11[c] as f32 * fx;
        out[c] = (top * (1.0 - fy) + bottom * fy).round() as u8;
    }
    image::Rgb(out)
}

pub fn embedding_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn embedding_from_bytes(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
