//! Library indexing: walks the photo folders and, in parallel, extracts metadata,
//! builds thumbnails and detects + embeds faces for new or modified files.
//!
//! # What runs at the same time
//!
//! All background work reports through [`ScanStatus`], which the HTTP handlers read and set:
//!
//! - **Scans**: one scan thread at most (`spawned`, taken atomically in [`spawn`]). A scan
//!   asked for while one runs sets `rerun`, and the same thread scans again afterwards.
//!   `running` is set for the length of one pass, and cleared by a guard even on a panic.
//! - **Identical files**: searched on the scan thread after each scan, or on a thread of
//!   their own when asked (`dups.running`); deleted by one request at a time
//!   (`dups.deleting`, refused with 409 while set).
//! - **Removing a folder**: one at a time (`removing`). It sets `hold`, which makes a
//!   running scan stop between photos and new scans end at once, waits for `running` to
//!   clear, removes the photos, then clears both and starts a scan for the other folders.
//! - **Rotating** is refused while `running` is set, so a scan never reads a file that is
//!   being rewritten.
//!
//! Database access is safe throughout: SQLite in WAL mode lets the request handlers read
//! while the scan's single writer thread commits its batches.

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
use crate::library::root_of;
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
    /// A scan thread exists (see `spawn`).
    spawned: AtomicBool,
    /// Regroup every face at the end of the next scan, not just the new ones.
    regroup: AtomicBool,
    /// The search for identical files, run after each scan.
    pub dups: Arc<crate::duplicates::DupStatus>,
    /// While set (a folder is being removed), a running scan stops between photos and new
    /// scans end at once, so none puts back photos that are being removed.
    pub hold: AtomicBool,
    /// The folder being removed, with how many of its photos are done.
    pub removing: Mutex<Option<String>>,
    pub remove_done: AtomicU64,
    pub remove_total: AtomicU64,
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
    pub removing: Option<String>,
    pub remove_done: u64,
    pub remove_total: u64,
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
            removing: self.removing.lock().unwrap().clone(),
            remove_done: self.remove_done.load(Ordering::Relaxed),
            remove_total: self.remove_total.load(Ordering::Relaxed),
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
    /// Folders given on the command line: always part of the library, next to the folders
    /// saved from Settings.
    pub fixed_roots: Vec<PathBuf>,
    pub db_path: PathBuf,
    pub models: Option<ModelPaths>,
    pub cluster_threshold: f32,
}

impl ScanConfig {
    /// The command-line folders, then the saved ones. A folder inside another one is left
    /// out: the outer one already includes it.
    pub fn roots(&self, conn: &Connection) -> Result<Vec<PathBuf>> {
        crate::library::roots(conn, &self.fixed_roots)
    }

    pub fn is_fixed(&self, path: &Path) -> bool {
        self.fixed_roots.iter().any(|r| r == path)
    }
}

/// Scans in a background thread. If a scan is already running, another one follows it.
pub fn spawn(cfg: Arc<ScanConfig>, status: Arc<ScanStatus>) {
    // One scan thread at a time: the flag is taken atomically, so two requests at the same
    // moment can't start two threads.
    if status.spawned.swap(true, Ordering::SeqCst) {
        status.rerun.store(true, Ordering::SeqCst);
        return;
    }
    std::thread::spawn(move || {
        loop {
            // A panic must not end the thread with the flag still taken: no scan would ever
            // run again.
            let work = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                loop {
                    if let Err(e) = run(&cfg, &status) {
                        tracing::error!("scan failed: {e:#}");
                    }
                    if !status.rerun.swap(false, Ordering::SeqCst) {
                        break;
                    }
                }
                // Then look for identical files among what's new.
                if let Err(e) = crate::duplicates::run(&cfg.db_path, &status.dups) {
                    tracing::error!("duplicate search failed: {e:#}");
                }
            }));
            if work.is_err() {
                tracing::error!("the scan crashed; the next one starts from where the index is");
            }
            status.spawned.store(false, Ordering::SeqCst);
            // A scan asked for while this thread was finishing would be lost: run it here,
            // unless another thread took the flag meanwhile.
            if !(status.rerun.load(Ordering::SeqCst) && !status.spawned.swap(true, Ordering::SeqCst)) {
                break;
            }
            status.rerun.store(false, Ordering::SeqCst);
        }
    });
}

struct FileEntry {
    path: PathBuf,
    mtime: i64,
    size: i64,
    /// The photo's id when the file was indexed before (it changed): it is updated in place,
    /// so the photo keeps its id.
    known: Option<i64>,
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

/// Clears a busy flag when dropped, also when the work panics or its request is dropped,
/// so a failure never leaves the gallery stuck as busy until a restart.
pub(crate) struct ClearOnDrop<'a>(pub &'a AtomicBool);

impl Drop for ClearOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub fn run(cfg: &ScanConfig, status: &ScanStatus) -> Result<()> {
    if status.running.swap(true, Ordering::SeqCst) {
        status.rerun.store(true, Ordering::SeqCst);
        return Ok(());
    }
    let _running = ClearOnDrop(&status.running);
    // Until the scan says otherwise: what a panic leaves behind.
    status.set_phase("failed");
    let result = scan(cfg, status);
    status.set_phase(match &result {
        Ok(()) => "idle",
        Err(_) => "failed",
    });
    result
}

fn scan(cfg: &ScanConfig, status: &ScanStatus) -> Result<()> {
    let started = Instant::now();
    for counter in [&status.total, &status.done, &status.errors, &status.faces, &status.group_done, &status.group_total]
    {
        counter.store(0, Ordering::Relaxed);
    }
    status.set_phase("listing files");
    let mut conn = crate::db::open(&cfg.db_path)?;
    let roots = cfg.roots(&conn)?;
    if roots.is_empty() {
        // Nothing to compare against: never treat "no folders" as "every photo was deleted".
        // Removing a folder from the UI removes its photos itself.
        tracing::info!("no photo folders yet; add one in Settings");
        return Ok(());
    }
    let files = list_files(&roots);
    if status.hold.load(Ordering::SeqCst) {
        tracing::info!("scan stopped: a folder is being removed");
        return Ok(());
    }
    // Photos on a folder that is currently unavailable (say, an unplugged drive) are kept.
    let offline: Vec<&PathBuf> = roots.iter().filter(|r| !crate::library::reachable(r)).collect();
    for r in &offline {
        tracing::warn!("folder {} is not available; keeping its photos", r.display());
    }

    let models = cfg.models.clone().filter(ModelPaths::exist);
    let known: HashMap<String, (i64, i64, i64, bool)> = {
        let mut stmt = conn.prepare("SELECT path, id, mtime, size, faces_scanned FROM photos")?;
        stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))?
            .collect::<Result<_, _>>()?
    };

    // Photos the user removed from the gallery stay out of it.
    let excluded: HashSet<String> =
        conn.prepare("SELECT path FROM excluded")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    // Files that failed before are only retried once they change.
    let failed: HashMap<String, (i64, i64)> = conn
        .prepare("SELECT path, mtime, size FROM failures")?
        .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
        .collect::<Result<_, _>>()?;

    let present: HashSet<String> = files.iter().map(|f| f.path.to_string_lossy().into_owned()).collect();
    {
        let mut forget = conn.prepare("DELETE FROM failures WHERE path = ?")?;
        for path in failed.keys().filter(|p| !present.contains(*p) && root_of(&offline, Path::new(p)).is_none()) {
            forget.execute([path])?;
        }
    }
    // With folders given on the command line, the folders (those and the saved ones) are the
    // whole library, so photos outside them go. Folders managed only from Settings remove
    // their own photos when removed, so a scan only drops photos inside the current folders
    // whose files are gone.
    let whole_library = !cfg.fixed_roots.is_empty();
    let stale: Vec<i64> = known
        .iter()
        .filter(|(p, _)| {
            let path = Path::new(p);
            !present.contains(*p)
                && root_of(&offline, path).is_none()
                && (whole_library || root_of(&roots, path).is_some())
        })
        .map(|(_, v)| v.0)
        .collect();
    let removed = stale.len();
    let todo: Vec<FileEntry> = files
        .into_iter()
        .filter_map(|mut f| match known.get(f.path.to_string_lossy().as_ref()) {
            // Changed files, and photos indexed while face recognition was unavailable: read
            // again, and updated in place.
            Some(&(id, mtime, size, faces_scanned))
                if mtime != f.mtime || size != f.size || (models.is_some() && !faces_scanned) =>
            {
                f.known = Some(id);
                Some(f)
            }
            Some(_) => None,
            None => {
                let path = f.path.to_string_lossy();
                let wanted = !excluded.contains(path.as_ref()) && failed.get(path.as_ref()) != Some(&(f.mtime, f.size));
                wanted.then_some(f)
            }
        })
        .collect();
    let updated = todo.iter().filter(|f| f.known.is_some()).count();

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
    tracing::info!("{} photos to index ({updated} changed), {removed} removed", todo.len());

    let (sender, receiver) = mpsc::sync_channel::<Outcome>(256);
    // Set when saving fails: the remaining files aren't read for nothing.
    let saving_failed = AtomicBool::new(false);
    std::thread::scope(|s| -> Result<()> {
        s.spawn(|| {
            todo.into_par_iter().for_each_with(sender, |sender, file| {
                if status.hold.load(Ordering::Relaxed) || saving_failed.load(Ordering::Relaxed) {
                    return; // stopped: what is done so far is kept, the rest waits for the next scan
                }
                let copy = FileEntry { path: file.path.clone(), mtime: file.mtime, size: file.size, known: file.known };
                // A file that crashes a decoder (or the face models) is a failed file, not a failed scan.
                let processed =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| process(file, models.as_ref())));
                let outcome = match processed {
                    Ok(Ok(p)) => Outcome::Indexed(p),
                    Ok(Err((file, e))) => {
                        tracing::warn!("{}: {e:#}", file.path.display());
                        Outcome::Failed(file, format!("{e:#}"))
                    }
                    Err(panic) => {
                        let why = panic
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "unknown error".into());
                        tracing::error!("{}: crashed while reading it: {why}", copy.path.display());
                        Outcome::Failed(copy, format!("crashed while reading it: {why}"))
                    }
                };
                if sender.send(outcome).is_err() {
                    saving_failed.store(true, Ordering::Relaxed);
                }
            });
        });
        write_results(&mut conn, receiver, status)
    })?;

    if status.hold.load(Ordering::SeqCst) {
        tracing::info!("scan stopped: a folder is being removed");
        return Ok(());
    }
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
                    Some(FileEntry { path: e.into_path(), mtime, size: meta.len() as i64, known: None })
                })
        })
        .collect()
}

enum Outcome {
    Indexed(Processed),
    /// The file couldn't be indexed; saved with the reason and retried when it changes.
    Failed(FileEntry, String),
}

/// The gallery's thumbnail of a decoded (upright) photo.
pub(crate) fn thumbnail(img: &image::RgbImage) -> Result<Vec<u8>> {
    let work = imaging::fit(img, WORK_MAX_SIDE)?;
    imaging::encode_jpeg(&imaging::fit_box(&work, THUMB_MAX_W, THUMB_MAX_H)?, 78)
}

fn process(file: FileEntry, models: Option<&ModelPaths>) -> Result<Processed, (FileEntry, anyhow::Error)> {
    let copy = FileEntry { path: file.path.clone(), mtime: file.mtime, size: file.size, known: file.known };
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
    let thumb = thumbnail(&work)?;

    let exif_date = exif.as_ref().and_then(crate::metadata::taken);
    let taken = exif_date.clone().unwrap_or_else(|| {
        let t = DateTime::from_timestamp(file.mtime, 0).unwrap_or_default().with_timezone(&Local);
        t.format("%Y-%m-%d %H:%M:%S").to_string()
    });
    let gps = exif.as_ref().and_then(crate::metadata::position);

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
                    // A photo whose file changed and can no longer be read leaves the gallery
                    // (its named people are remembered first).
                    if let Some(id) = file.known {
                        let people: Vec<i64> = tx
                            .prepare_cached(
                                "SELECT DISTINCT person_id FROM faces WHERE photo_id = ? AND person_id IS NOT NULL",
                            )?
                            .query_map([id], |r| r.get(0))?
                            .collect::<Result<_, _>>()?;
                        crate::db::remember_named_people(&tx, Some(&people))?;
                        tx.execute("DELETE FROM photos WHERE id = ?", [id])?;
                    }
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
            let photo_id = match p.file.known {
                // Changed: the same photo with what the file says now. Its faces are found
                // again; the version tells browsers to fetch the new thumbnail.
                Some(id) => {
                    tx.execute(
                        "UPDATE photos SET mtime = ?, size = ?, width = ?, height = ?, taken = ?, date_from_exif = ?,
                             lat = ?, lon = ?, place_id = ?, faces_scanned = ?, content_hash = NULL, version = version + 1
                         WHERE id = ?",
                        params![
                            p.file.mtime,
                            p.file.size,
                            p.width,
                            p.height,
                            p.taken,
                            p.from_exif,
                            p.gps.map(|g| g.0),
                            p.gps.map(|g| g.1),
                            place_id,
                            p.faces_scanned,
                            id
                        ],
                    )?;
                    tx.execute("DELETE FROM thumbs WHERE photo_id = ?", [id])?;
                    tx.execute("DELETE FROM faces WHERE photo_id = ?", [id])?;
                    id
                }
                None => {
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
                    tx.last_insert_rowid()
                }
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_flag_clears_when_the_work_panics() {
        let busy = AtomicBool::new(true);
        let result = std::panic::catch_unwind(|| {
            let _busy = ClearOnDrop(&busy);
            panic!("a decoder crashed");
        });
        assert!(result.is_err());
        assert!(!busy.load(Ordering::SeqCst));
    }

    #[test]
    fn a_failed_scan_is_not_left_running() {
        // A database that can't be opened (its folder is a file): the scan fails at once.
        let not_a_folder = std::env::temp_dir().join(format!("imadive-test-{}", std::process::id()));
        std::fs::write(&not_a_folder, b"").unwrap();
        let cfg = ScanConfig {
            fixed_roots: vec![std::env::temp_dir()],
            db_path: not_a_folder.join("index.sqlite"),
            models: None,
            cluster_threshold: 0.42,
        };
        let status = ScanStatus::default();
        let result = run(&cfg, &status);
        std::fs::remove_file(&not_a_folder).unwrap();
        assert!(result.is_err());
        assert!(!status.running.load(Ordering::SeqCst));
        assert_eq!(status.view().phase, "failed");
    }

    use crate::testutil::{Photo, TempDir};

    /// A library in `dir/photos` (given on the command line when `fixed`) with its index in
    /// `dir/data`.
    fn library(dir: &TempDir, fixed: bool) -> ScanConfig {
        let photos = dir.file("photos/.keep").parent().unwrap().to_path_buf();
        ScanConfig {
            fixed_roots: if fixed { vec![photos] } else { Vec::new() },
            db_path: dir.file("data/index.sqlite"),
            models: None,
            cluster_threshold: 0.42,
        }
    }

    fn scan_now(cfg: &ScanConfig) {
        run(cfg, &ScanStatus::default()).expect("scan");
    }

    /// Indexed paths relative to `dir`, sorted.
    fn indexed(cfg: &ScanConfig, dir: &TempDir) -> Vec<String> {
        let conn = crate::db::open(&cfg.db_path).unwrap();
        let mut paths: Vec<String> = conn
            .prepare("SELECT path FROM photos")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|p| p.unwrap().strip_prefix(&*dir.path().to_string_lossy()).unwrap().replace('\\', "/"))
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn roots_leave_out_folders_inside_others() {
        let conn = crate::db::open_in_memory();
        let cfg = |fixed: &[&str]| ScanConfig {
            fixed_roots: fixed.iter().map(PathBuf::from).collect(),
            db_path: PathBuf::new(),
            models: None,
            cluster_threshold: 0.42,
        };
        let save = |paths: &[&str]| {
            conn.execute("DELETE FROM folders", []).unwrap();
            for p in paths {
                conn.execute("INSERT INTO folders (path) VALUES (?)", [p]).unwrap();
            }
        };
        let roots = |c: &ScanConfig| {
            c.roots(&conn).unwrap().into_iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>()
        };

        save(&["/a/b"]);
        assert_eq!(roots(&cfg(&["/a"])), ["/a"], "a saved folder inside a command-line one");
        save(&["/a/b", "/a"]);
        assert_eq!(roots(&cfg(&[])), ["/a"], "a saved folder inside another saved one");
        save(&["/a"]);
        assert_eq!(roots(&cfg(&["/a"])), ["/a"], "the same folder twice");
        save(&["/ab"]);
        assert_eq!(roots(&cfg(&["/a"])), ["/a", "/ab"], "/ab is not inside /a");
        save(&[]);
        assert!(roots(&cfg(&[])).is_empty());
    }

    #[test]
    fn indexes_dates_and_places() {
        let dir = TempDir::new();
        let cfg = library(&dir, true);
        Photo { taken: Some("2019:07:04 18:30:00"), gps: Some((40.4168, -3.7038)), ..Photo::default() }
            .write(&dir.file("photos/madrid.jpg"));
        Photo { color: [10, 20, 30], ..Photo::default() }.write(&dir.file("photos/sub/no-exif.jpg"));
        std::fs::write(dir.file("photos/notes.txt"), "not a photo").unwrap();
        scan_now(&cfg);

        assert_eq!(indexed(&cfg, &dir), ["/photos/madrid.jpg", "/photos/sub/no-exif.jpg"]);
        let conn = crate::db::open(&cfg.db_path).unwrap();
        let (taken, from_exif, city, w, h): (String, bool, String, u32, u32) = conn
            .query_row(
                "SELECT taken, date_from_exif, city, width, height FROM photos JOIN places ON places.id = place_id
                 WHERE path LIKE '%madrid.jpg'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!((taken.as_str(), from_exif, city.as_str(), w, h), ("2019-07-04 18:30:00", true, "Madrid", 64, 48));
        let (from_exif, place): (bool, Option<i64>) = conn
            .query_row("SELECT date_from_exif, place_id FROM photos WHERE path LIKE '%no-exif.jpg'", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((from_exif, place), (false, None), "file date, no place");
        let thumbs: i64 = conn.query_row("SELECT COUNT(*) FROM thumbs", [], |r| r.get(0)).unwrap();
        assert_eq!(thumbs, 2);
    }

    #[test]
    fn a_rescan_prunes_deleted_files_and_adds_new_ones() {
        let dir = TempDir::new();
        let cfg = library(&dir, true);
        Photo::default().write(&dir.file("photos/a.jpg"));
        Photo { color: [1, 2, 3], ..Photo::default() }.write(&dir.file("photos/b.jpg"));
        scan_now(&cfg);
        std::fs::remove_file(dir.file("photos/a.jpg")).unwrap();
        Photo { color: [4, 5, 6], ..Photo::default() }.write(&dir.file("photos/c.jpg"));
        scan_now(&cfg);
        assert_eq!(indexed(&cfg, &dir), ["/photos/b.jpg", "/photos/c.jpg"]);
    }

    #[test]
    fn photos_on_a_missing_folder_are_kept() {
        // An unplugged drive: its folder is gone, but its photos stay in the gallery.
        let dir = TempDir::new();
        let cfg = library(&dir, true);
        Photo::default().write(&dir.file("photos/a.jpg"));
        scan_now(&cfg);
        std::fs::remove_dir_all(dir.path().join("photos")).unwrap();
        scan_now(&cfg);
        assert_eq!(indexed(&cfg, &dir), ["/photos/a.jpg"]);
    }

    #[test]
    fn folders_from_settings_only_prune_inside_themselves() {
        // With command-line folders, they are the whole library: photos outside them go.
        let dir = TempDir::new();
        let mut cfg = library(&dir, true);
        Photo::default().write(&dir.file("photos/a.jpg"));
        Photo { color: [1, 2, 3], ..Photo::default() }.write(&dir.file("other/b.jpg"));
        cfg.fixed_roots.push(dir.path().join("other"));
        scan_now(&cfg);
        cfg.fixed_roots.pop();
        scan_now(&cfg);
        assert_eq!(indexed(&cfg, &dir), ["/photos/a.jpg"]);

        // With folders from Settings only, removing a folder removes its own photos, so a
        // scan never drops photos outside the current folders.
        let dir = TempDir::new();
        let cfg = library(&dir, false);
        Photo::default().write(&dir.file("photos/a.jpg"));
        Photo { color: [1, 2, 3], ..Photo::default() }.write(&dir.file("other/b.jpg"));
        let conn = crate::db::open(&cfg.db_path).unwrap();
        for f in ["photos", "other"] {
            conn.execute("INSERT INTO folders (path) VALUES (?)", [dir.path().join(f).to_string_lossy()]).unwrap();
        }
        scan_now(&cfg);
        conn.execute("DELETE FROM folders WHERE path LIKE '%other'", []).unwrap();
        scan_now(&cfg);
        assert_eq!(indexed(&cfg, &dir), ["/other/b.jpg", "/photos/a.jpg"]);
    }

    #[test]
    fn no_folders_never_empties_the_index() {
        let dir = TempDir::new();
        let cfg = library(&dir, false);
        Photo::default().write(&dir.file("photos/a.jpg"));
        let conn = crate::db::open(&cfg.db_path).unwrap();
        conn.execute("INSERT INTO folders (path) VALUES (?)", [dir.path().join("photos").to_string_lossy()]).unwrap();
        scan_now(&cfg);
        conn.execute("DELETE FROM folders", []).unwrap();
        scan_now(&cfg);
        assert_eq!(indexed(&cfg, &dir), ["/photos/a.jpg"]);
    }

    #[test]
    fn removed_photos_stay_out_and_unreadable_files_wait_for_a_change() {
        let dir = TempDir::new();
        let cfg = library(&dir, true);
        Photo::default().write(&dir.file("photos/keep.jpg"));
        Photo { color: [1, 2, 3], ..Photo::default() }.write(&dir.file("photos/removed.jpg"));
        std::fs::write(dir.file("photos/broken.jpg"), b"not really a JPEG").unwrap();
        let conn = crate::db::open(&cfg.db_path).unwrap();
        conn.execute(
            "INSERT INTO excluded (path) VALUES (?)",
            [dir.path().join(crate::testutil::native("photos/removed.jpg")).to_string_lossy()],
        )
        .unwrap();

        let status = ScanStatus::default();
        run(&cfg, &status).unwrap();
        assert_eq!(indexed(&cfg, &dir), ["/photos/keep.jpg"]);
        assert_eq!(status.view().errors, 1);
        let failures: i64 = conn.query_row("SELECT COUNT(*) FROM failures", [], |r| r.get(0)).unwrap();
        assert_eq!(failures, 1);

        // Unchanged, the broken file is not read again.
        let status = ScanStatus::default();
        run(&cfg, &status).unwrap();
        assert_eq!((status.view().total, status.view().errors), (0, 0));

        // Fixed (a new size), it is read and indexed, and its failure is forgotten.
        Photo { color: [7, 8, 9], ..Photo::default() }.write(&dir.file("photos/broken.jpg"));
        scan_now(&cfg);
        assert_eq!(indexed(&cfg, &dir), ["/photos/broken.jpg", "/photos/keep.jpg"]);
        let failures: i64 = conn.query_row("SELECT COUNT(*) FROM failures", [], |r| r.get(0)).unwrap();
        assert_eq!(failures, 0);
    }

    #[test]
    fn an_empty_photo_folder_counts_as_unplugged() {
        // On Linux an unmounted drive leaves its mount point as an empty folder; its photos
        // must not be taken for deleted.
        let lib = crate::testutil::Library::new();
        lib.add("a.jpg", Photo::default());
        lib.add("sub/b.jpg", Photo { color: [1, 2, 3], ..Photo::default() });
        lib.scan();
        std::fs::remove_dir_all(lib.root()).unwrap();
        std::fs::create_dir(lib.root()).unwrap();
        lib.scan();
        let conn = lib.conn();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
        // A folder that only lost some files is pruned as usual.
        lib.add("c.jpg", Photo { color: [4, 5, 6], ..Photo::default() });
        lib.scan();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn a_changed_file_keeps_its_id() {
        let lib = crate::testutil::Library::new();
        lib.add("a.jpg", Photo { width: 64, height: 48, ..Photo::default() });
        lib.scan();
        let id = lib.id("a.jpg");
        lib.add("a.jpg", Photo { width: 30, height: 20, taken: Some("2018:02:03 04:05:06"), ..Photo::default() });
        let status = lib.scan();
        assert_eq!(status.total, 1);
        assert_eq!(lib.id("a.jpg"), id, "same photo, so links to it keep working");
        let (w, h, taken, version): (u32, u32, String, i64) = lib
            .conn()
            .query_row("SELECT width, height, taken, version FROM photos WHERE id = ?", [id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .unwrap();
        assert_eq!((w, h, taken.as_str()), (30, 20, "2018-02-03 04:05:06"));
        assert_eq!(version, 1, "browsers fetch the new thumbnail");
        let thumbs: i64 = lib.conn().query_row("SELECT COUNT(*) FROM thumbs", [], |r| r.get(0)).unwrap();
        assert_eq!(thumbs, 1);
    }

    #[test]
    fn a_changed_file_that_breaks_leaves_the_gallery() {
        let lib = crate::testutil::Library::new();
        lib.add("a.jpg", Photo::default());
        lib.scan();
        std::fs::write(lib.root().join("a.jpg"), b"broken now").unwrap();
        let status = lib.scan();
        assert_eq!(status.errors, 1);
        let conn = lib.conn();
        let (photos, failures): (i64, i64) = conn
            .query_row("SELECT (SELECT COUNT(*) FROM photos), (SELECT COUNT(*) FROM failures)", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((photos, failures), (0, 1));
    }

    #[test]
    fn scans_asked_for_at_once_run_one_after_another() {
        let lib = crate::testutil::Library::new();
        lib.add("a.jpg", Photo::default());
        let cfg = Arc::new(ScanConfig {
            fixed_roots: lib.cfg.fixed_roots.clone(),
            db_path: lib.cfg.db_path.clone(),
            models: None,
            cluster_threshold: 0.42,
        });
        let status = Arc::new(ScanStatus::default());
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (cfg, status) = (cfg.clone(), status.clone());
                std::thread::spawn(move || spawn(cfg, status))
            })
            .collect();
        threads.into_iter().for_each(|t| t.join().unwrap());
        let start = std::time::Instant::now();
        while status.spawned.load(Ordering::SeqCst) || status.running.load(Ordering::SeqCst) {
            assert!(start.elapsed().as_secs() < 30, "the scans never finished");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!status.rerun.load(Ordering::SeqCst), "no scan left waiting");
        assert_eq!(lib.scan().total, 0, "a.jpg was indexed");
    }
}
