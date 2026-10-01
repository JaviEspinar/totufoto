//! Library indexing: walks the photo folders and, in parallel, extracts metadata,
//! builds thumbnails and detects + embeds faces for new or modified files.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Instant, UNIX_EPOCH};

use anyhow::Result;
use chrono::{DateTime, Local};
use rayon::prelude::*;
use rusqlite::{Connection, params};
use serde::Serialize;
use walkdir::WalkDir;

use crate::faces::{FaceModels, MIN_FACE_PX, ModelPaths};
use crate::{cluster, geo, imaging};

const WORK_MAX_SIDE: u32 = 1600;
/// Thumbnails are sized for rows up to ~220 CSS px tall on 2x screens.
const THUMB_MAX_W: u32 = 1100;
const THUMB_MAX_H: u32 = 440;
const FACE_THUMB: u32 = 128;

#[derive(Default)]
pub struct ScanStatus {
    pub running: AtomicBool,
    pub total: AtomicU64,
    pub done: AtomicU64,
    pub errors: AtomicU64,
    pub faces: AtomicU64,
    pub phase: Mutex<String>,
    /// Progress of the "grouping faces" step.
    pub group_done: AtomicU64,
    pub group_total: AtomicU64,
    /// Another scan was requested while one was running (for example a folder was added).
    rerun: AtomicBool,
    /// Regroup every face at the end of the next scan, not just the new ones.
    regroup: AtomicBool,
}

#[derive(Serialize)]
pub struct StatusView {
    pub running: bool,
    pub phase: String,
    pub total: u64,
    pub done: u64,
    pub errors: u64,
    pub faces: u64,
    pub group_done: u64,
    pub group_total: u64,
}

impl ScanStatus {
    pub fn view(&self) -> StatusView {
        StatusView {
            running: self.running.load(Ordering::Relaxed),
            phase: self.phase.lock().unwrap().clone(),
            total: self.total.load(Ordering::Relaxed),
            done: self.done.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            faces: self.faces.load(Ordering::Relaxed),
            group_done: self.group_done.load(Ordering::Relaxed),
            group_total: self.group_total.load(Ordering::Relaxed),
        }
    }

    /// Makes the next scan regroup every face from scratch.
    pub fn request_regroup(&self) {
        self.regroup.store(true, Ordering::SeqCst);
    }

    fn set_phase(&self, phase: &str) {
        *self.phase.lock().unwrap() = phase.to_string();
    }
}

pub struct ScanConfig {
    /// Folders given on the command line. When empty, the folders saved in the index are used.
    pub fixed_roots: Vec<PathBuf>,
    pub db_path: PathBuf,
    pub models: Option<ModelPaths>,
    pub cluster_threshold: f32,
}

impl ScanConfig {
    pub fn roots(&self, conn: &Connection) -> Result<Vec<PathBuf>> {
        if !self.fixed_roots.is_empty() {
            return Ok(self.fixed_roots.clone());
        }
        let roots = conn
            .prepare("SELECT path FROM folders ORDER BY path")?
            .query_map([], |r| r.get::<_, String>(0))?
            .map(|r| r.map(PathBuf::from))
            .collect::<Result<_, _>>()?;
        Ok(roots)
    }
}

/// Scans in a background thread. If a scan is already running, another one follows it.
pub fn spawn(cfg: Arc<ScanConfig>, status: Arc<ScanStatus>) {
    if status.running.load(Ordering::SeqCst) {
        status.rerun.store(true, Ordering::SeqCst);
        return;
    }
    std::thread::spawn(move || {
        loop {
            if let Err(e) = run(&cfg, &status) {
                tracing::error!("scan failed: {e:#}");
            }
            if !status.rerun.swap(false, Ordering::SeqCst) {
                break;
            }
        }
    });
}

struct FileEntry {
    path: PathBuf,
    mtime: i64,
    size: i64,
}

struct FaceOut {
    rel: [f32; 4],
    score: f32,
    embedding: Vec<f32>,
    thumb: Vec<u8>,
}

struct Processed {
    file: FileEntry,
    width: u32,
    height: u32,
    taken: String,
    from_exif: bool,
    gps: Option<(f64, f64)>,
    thumb: Vec<u8>,
    faces: Vec<FaceOut>,
    faces_scanned: bool,
}

thread_local! {
    static MODELS: RefCell<Option<FaceModels>> = const { RefCell::new(None) };
}

pub fn run(cfg: &ScanConfig, status: &ScanStatus) -> Result<()> {
    if status.running.swap(true, Ordering::SeqCst) {
        status.rerun.store(true, Ordering::SeqCst);
        return Ok(());
    }
    let result = scan(cfg, status);
    status.running.store(false, Ordering::SeqCst);
    status.set_phase(match &result {
        Ok(()) => "idle",
        Err(_) => "failed",
    });
    result
}

fn scan(cfg: &ScanConfig, status: &ScanStatus) -> Result<()> {
    let started = Instant::now();
    for counter in [&status.total, &status.done, &status.errors, &status.faces, &status.group_done, &status.group_total] {
        counter.store(0, Ordering::Relaxed);
    }
    status.set_phase("listing files");
    let mut conn = crate::db::open(&cfg.db_path)?;
    let roots = cfg.roots(&conn)?;
    if roots.is_empty() {
        // Nothing to compare against: never treat "no folders" as "every photo was deleted".
        // Removing a folder from the UI removes its photos itself.
        tracing::info!("no photo folders yet; add one from the Folders button");
        return Ok(());
    }
    let files = list_files(&roots);
    // Photos on a folder that is currently unavailable (say, an unplugged drive) are kept.
    let offline: Vec<&PathBuf> = roots.iter().filter(|r| !r.is_dir()).collect();
    for r in &offline {
        tracing::warn!("folder {} is not available; keeping its photos", r.display());
    }

    let models = cfg.models.clone().filter(ModelPaths::exist);
    let known: HashMap<String, (i64, i64, i64, bool)> = {
        let mut stmt = conn.prepare("SELECT path, id, mtime, size, faces_scanned FROM photos")?;
        stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))?
            .collect::<Result<_, _>>()?
    };

    // Files that failed before are only retried once they change.
    let failed: HashMap<String, (i64, i64)> = conn
        .prepare("SELECT path, mtime, size FROM failures")?
        .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
        .collect::<Result<_, _>>()?;

    let present: HashSet<String> = files.iter().map(|f| f.path.to_string_lossy().into_owned()).collect();
    {
        let mut forget = conn.prepare("DELETE FROM failures WHERE path = ?")?;
        for path in failed.keys().filter(|p| !present.contains(*p) && !offline.iter().any(|r| Path::new(p).starts_with(r))) {
            forget.execute([path])?;
        }
    }
    // Folders given on the command line define the whole library, so photos outside them go.
    // Folders managed from the UI remove their own photos when removed, so a scan only drops
    // photos inside the current folders whose files are gone.
    let whole_library = !cfg.fixed_roots.is_empty();
    let mut stale: Vec<i64> = known
        .iter()
        .filter(|(p, _)| {
            let path = Path::new(p);
            !present.contains(*p)
                && !offline.iter().any(|r| path.starts_with(r))
                && (whole_library || roots.iter().any(|r| path.starts_with(r)))
        })
        .map(|(_, v)| v.0)
        .collect();
    let removed = stale.len();
    let todo: Vec<FileEntry> = files
        .into_iter()
        .filter(|f| match known.get(f.path.to_string_lossy().as_ref()) {
            // Changed files, and photos indexed while face recognition was unavailable.
            Some(&(id, mtime, size, faces_scanned))
                if mtime != f.mtime || size != f.size || (models.is_some() && !faces_scanned) =>
            {
                stale.push(id);
                true
            }
            Some(_) => false,
            None => failed.get(f.path.to_string_lossy().as_ref()) != Some(&(f.mtime, f.size)),
        })
        .collect();

    if !stale.is_empty() {
        // Named people whose photos are going keep their face, to rejoin them if they return.
        crate::db::remember_named_people(&conn, None)?;
        let tx = conn.transaction()?;
        for id in &stale {
            tx.execute("DELETE FROM photos WHERE id = ?", [id])?;
        }
        tx.commit()?;
    }

    status.total.store(todo.len() as u64, Ordering::Relaxed);
    status.set_phase("indexing photos");
    tracing::info!("{} photos to index ({} updated), {} removed", todo.len(), stale.len() - removed, removed);

    let (sender, receiver) = mpsc::sync_channel::<Outcome>(256);
    std::thread::scope(|s| -> Result<()> {
        s.spawn(|| {
            todo.into_par_iter().for_each_with(sender, |sender, file| {
                let outcome = match process(file, models.as_ref()) {
                    Ok(p) => Outcome::Indexed(p),
                    Err((file, e)) => {
                        tracing::warn!("{}: {e:#}", file.path.display());
                        Outcome::Failed(file, format!("{e:#}"))
                    }
                };
                let _ = sender.send(outcome);
            });
        });
        write_results(&mut conn, receiver, status)
    })?;

    if models.is_some() {
        status.set_phase("grouping faces");
        let progress = cluster::Progress { done: &status.group_done, total: &status.group_total };
        let full = status.regroup.swap(false, Ordering::SeqCst);
        cluster::update_groups(&mut conn, cfg.cluster_threshold, full, &progress)?;
    }
    conn.execute_batch("PRAGMA optimize;")?;
    tracing::info!(
        "scan finished in {:.1}s: {} photos, {} faces, {} errors",
        started.elapsed().as_secs_f32(),
        status.done.load(Ordering::Relaxed),
        status.faces.load(Ordering::Relaxed),
        status.errors.load(Ordering::Relaxed)
    );
    Ok(())
}

fn list_files(roots: &[PathBuf]) -> Vec<FileEntry> {
    roots
        .par_iter()
        .flat_map_iter(|root| {
            WalkDir::new(root)
                .follow_links(true)
                .into_iter()
                .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .filter(|e| imaging::is_supported(e.path()))
                .filter_map(|e| {
                    let meta = e.metadata().ok()?;
                    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
                    Some(FileEntry { path: e.into_path(), mtime, size: meta.len() as i64 })
                })
        })
        .collect()
}

enum Outcome {
    Indexed(Processed),
    /// The file couldn't be indexed; saved with the reason and retried when it changes.
    Failed(FileEntry, String),
}

fn process(file: FileEntry, models: Option<&ModelPaths>) -> Result<Processed, (FileEntry, anyhow::Error)> {
    let copy = FileEntry { path: file.path.clone(), mtime: file.mtime, size: file.size };
    process_inner(file, models).map_err(|e| (copy, e))
}

fn process_inner(file: FileEntry, models: Option<&ModelPaths>) -> Result<Processed> {
    let bytes = std::fs::read(&file.path)?;
    let exif = exif::Reader::new().read_from_container(&mut Cursor::new(&bytes)).ok();

    let img = imaging::decode(&file.path, &bytes)?.into_rgb8();
    drop(bytes);
    let (width, height) = img.dimensions();
    let work = imaging::fit(&img, WORK_MAX_SIDE)?;
    drop(img);
    let thumb = imaging::encode_jpeg(&imaging::fit_box(&work, THUMB_MAX_W, THUMB_MAX_H)?, 78)?;

    let exif_date = exif.as_ref().and_then(exif_datetime);
    let taken = exif_date.clone().unwrap_or_else(|| {
        let t = DateTime::from_timestamp(file.mtime, 0).unwrap_or_default().with_timezone(&Local);
        t.format("%Y-%m-%d %H:%M:%S").to_string()
    });
    let gps = exif.as_ref().and_then(exif_gps);

    let faces = match models {
        Some(paths) => detect_faces(&work, paths)?,
        None => Vec::new(),
    };

    let faces_scanned = models.is_some();
    Ok(Processed { file, width, height, taken, from_exif: exif_date.is_some(), gps, thumb, faces, faces_scanned })
}

fn detect_faces(work: &image::RgbImage, paths: &ModelPaths) -> Result<Vec<FaceOut>> {
    MODELS.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(FaceModels::load(paths)?);
        }
        let models = slot.as_mut().unwrap();
        let dets: Vec<_> = models
            .detect(work)?
            .into_iter()
            .filter(|d| (d.bbox[2] - d.bbox[0]).min(d.bbox[3] - d.bbox[1]) >= MIN_FACE_PX)
            .collect();
        let embeddings = models.embed(work, &dets)?;
        let (w, h) = (work.width() as f32, work.height() as f32);
        dets.into_iter()
            .zip(embeddings)
            .map(|(d, embedding)| {
                let [x1, y1, x2, y2] = [d.bbox[0].max(0.0), d.bbox[1].max(0.0), d.bbox[2].min(w), d.bbox[3].min(h)];
                Ok(FaceOut {
                    rel: [x1 / w, y1 / h, (x2 - x1) / w, (y2 - y1) / h],
                    score: d.score,
                    thumb: imaging::face_thumbnail(work, d.bbox, FACE_THUMB)?,
                    embedding,
                })
            })
            .collect()
    })
}

fn write_results(conn: &mut Connection, receiver: mpsc::Receiver<Outcome>, status: &ScanStatus) -> Result<()> {
    let mut places: HashMap<(String, String, String), i64> = HashMap::new();
    let mut batch = Vec::with_capacity(64);
    let flush = |conn: &mut Connection, batch: &mut Vec<Outcome>, places: &mut HashMap<_, _>| -> Result<()> {
        let tx = conn.transaction()?;
        for outcome in batch.drain(..) {
            let p = match outcome {
                Outcome::Indexed(p) => p,
                Outcome::Failed(file, error) => {
                    let error: String = error.chars().take(300).collect();
                    tx.execute(
                        "INSERT OR REPLACE INTO failures (path, mtime, size, error) VALUES (?, ?, ?, ?)",
                        params![file.path.to_string_lossy(), file.mtime, file.size, error],
                    )?;
                    status.errors.fetch_add(1, Ordering::Relaxed);
                    status.done.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };
            tx.execute("DELETE FROM failures WHERE path = ?", [p.file.path.to_string_lossy()])?;
            let place_id = match p.gps {
                Some((lat, lon)) => Some(place_id(&tx, places, lat, lon)?),
                None => None,
            };
            tx.execute(
                "INSERT INTO photos (path, mtime, size, width, height, taken, date_from_exif, lat, lon, place_id, faces_scanned)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    p.file.path.to_string_lossy(),
                    p.file.mtime,
                    p.file.size,
                    p.width,
                    p.height,
                    p.taken,
                    p.from_exif,
                    p.gps.map(|g| g.0),
                    p.gps.map(|g| g.1),
                    place_id,
                    p.faces_scanned
                ],
            )?;
            let photo_id = tx.last_insert_rowid();
            tx.execute("INSERT INTO thumbs (photo_id, data) VALUES (?, ?)", params![photo_id, p.thumb])?;
            for f in &p.faces {
                tx.execute(
                    "INSERT INTO faces (photo_id, x, y, w, h, score, embedding, thumb, grouped) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 0)",
                    params![
                        photo_id,
                        f.rel[0],
                        f.rel[1],
                        f.rel[2],
                        f.rel[3],
                        f.score,
                        crate::faces::embedding_to_bytes(&f.embedding),
                        f.thumb
                    ],
                )?;
            }
            status.faces.fetch_add(p.faces.len() as u64, Ordering::Relaxed);
            status.done.fetch_add(1, Ordering::Relaxed);
        }
        tx.commit()?;
        Ok(())
    };
    for p in receiver {
        batch.push(p);
        if batch.len() >= 64 {
            flush(conn, &mut batch, &mut places)?;
        }
    }
    flush(conn, &mut batch, &mut places)
}

fn place_id(conn: &Connection, cache: &mut HashMap<(String, String, String), i64>, lat: f64, lon: f64) -> Result<i64> {
    let city = geo::lookup(lat, lon);
    let key = (city.name.clone(), city.region.clone(), city.country.clone());
    if let Some(&id) = cache.get(&key) {
        return Ok(id);
    }
    conn.execute(
        "INSERT OR IGNORE INTO places (city, region, country, lat, lon) VALUES (?, ?, ?, ?, ?)",
        params![key.0, key.1, key.2, city.lat, city.lon],
    )?;
    let id = conn.query_row(
        "SELECT id FROM places WHERE city = ? AND region = ? AND country = ?",
        params![key.0, key.1, key.2],
        |r| r.get(0),
    )?;
    cache.insert(key, id);
    Ok(id)
}

fn exif_datetime(exif: &exif::Exif) -> Option<String> {
    [exif::Tag::DateTimeOriginal, exif::Tag::DateTimeDigitized, exif::Tag::DateTime].into_iter().find_map(|tag| {
        let field = exif.get_field(tag, exif::In::PRIMARY)?;
        let exif::Value::Ascii(ref parts) = field.value else { return None };
        let dt = exif::DateTime::from_ascii(parts.first()?).ok()?;
        if dt.year < 1900 || dt.month == 0 || dt.day == 0 {
            return None;
        }
        Some(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
        ))
    })
}

fn exif_gps(exif: &exif::Exif) -> Option<(f64, f64)> {
    let coord = |value_tag, ref_tag, negative: u8| -> Option<f64> {
        let field = exif.get_field(value_tag, exif::In::PRIMARY)?;
        let exif::Value::Rational(ref v) = field.value else { return None };
        if v.len() < 3 || v.iter().any(|r| r.denom == 0) {
            return None;
        }
        let deg = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
        let sign = match exif.get_field(ref_tag, exif::In::PRIMARY).map(|f| &f.value) {
            Some(exif::Value::Ascii(r)) if r.first().and_then(|s| s.first()) == Some(&negative) => -1.0,
            _ => 1.0,
        };
        Some(deg * sign)
    };
    let lat = coord(exif::Tag::GPSLatitude, exif::Tag::GPSLatitudeRef, b'S')?;
    let lon = coord(exif::Tag::GPSLongitude, exif::Tag::GPSLongitudeRef, b'W')?;
    // 0,0 is what many cameras write when they have no fix.
    let valid = lat.abs() <= 90.0 && lon.abs() <= 180.0 && (lat.abs() > 1e-6 || lon.abs() > 1e-6);
    valid.then_some((lat, lon))
}
