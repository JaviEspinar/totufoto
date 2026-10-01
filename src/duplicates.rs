//! Finding identical photo files (the very same bytes) to free disk space.
//!
//! Only files that share their size with another can be identical, so only those are read
//! and fingerprinted (BLAKE3); the fingerprint is stored with the photo and kept until the
//! file changes. Of each set of identical files the one with the oldest file date is kept
//! (ties: the first by path), and the others can be moved to the bin.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use anyhow::Result;
use rayon::prelude::*;
use rusqlite::{Connection, params};
use serde::Serialize;

/// Files read at the same time: fingerprinting is limited by the disk, not the CPU.
const READERS: usize = 4;

#[derive(Default)]
pub struct DupStatus {
    pub running: AtomicBool,
    pub done: AtomicU64,
    pub total: AtomicU64,
    /// When the last search finished (local time), if any.
    pub finished: Mutex<Option<String>>,
    rerun: AtomicBool,
    /// Deleting the duplicates: one deletion at a time, with its progress.
    pub deleting: AtomicBool,
    pub delete_done: AtomicU64,
    pub delete_total: AtomicU64,
    pub delete_freed: AtomicU64,
}

/// Searches in a background thread; one asked for while searching runs right after.
pub fn spawn(db_path: PathBuf, status: Arc<DupStatus>) {
    if status.running.load(Ordering::SeqCst) {
        status.rerun.store(true, Ordering::SeqCst);
        return;
    }
    std::thread::spawn(move || {
        loop {
            if let Err(e) = run(&db_path, &status) {
                tracing::error!("duplicate search failed: {e:#}");
            }
            if !status.rerun.swap(false, Ordering::SeqCst) {
                break;
            }
        }
    });
}

/// Fingerprints the photos that might have an identical twin and haven't been yet.
pub fn run(db_path: &Path, status: &DupStatus) -> Result<()> {
    if status.running.swap(true, Ordering::SeqCst) {
        status.rerun.store(true, Ordering::SeqCst);
        return Ok(());
    }
    let result = fingerprint(db_path, status);
    status.running.store(false, Ordering::SeqCst);
    *status.finished.lock().unwrap() = Some(chrono::Local::now().format("%Y-%m-%d %H:%M").to_string());
    result
}

fn fingerprint(db_path: &Path, status: &DupStatus) -> Result<()> {
    let conn = crate::db::open(db_path)?;
    let todo: Vec<(i64, String, i64, i64)> = conn
        .prepare(
            "SELECT id, path, mtime, size FROM photos WHERE content_hash IS NULL
             AND size IN (SELECT size FROM photos GROUP BY size HAVING COUNT(*) > 1)",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    status.done.store(0, Ordering::Relaxed);
    status.total.store(todo.len() as u64, Ordering::Relaxed);
    if todo.is_empty() {
        return Ok(());
    }
    let pool = rayon::ThreadPoolBuilder::new().num_threads(READERS).build()?;
    let hashes: Vec<(i64, String)> = pool.install(|| {
        todo.par_iter()
            .filter_map(|(id, path, mtime, size)| {
                let hash = unchanged(Path::new(path), *mtime, *size).then(|| hash_file(Path::new(path)).ok()).flatten();
                status.done.fetch_add(1, Ordering::Relaxed);
                hash.map(|h| (*id, h))
            })
            .collect()
    });
    let mut save = conn.prepare("UPDATE photos SET content_hash = ? WHERE id = ?")?;
    conn.execute_batch("BEGIN")?;
    for (id, hash) in &hashes {
        save.execute(params![hash, id])?;
    }
    conn.execute_batch("COMMIT")?;
    tracing::info!("fingerprinted {} possible duplicates", hashes.len());
    Ok(())
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// The file is still the one that was indexed (same size and modification time).
fn unchanged(path: &Path, mtime: i64, size: i64) -> bool {
    std::fs::metadata(path).is_ok_and(|m| {
        m.len() as i64 == size
            && m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64) == Some(mtime)
    })
}

#[derive(Serialize, Clone)]
pub struct DupFile {
    pub id: i64,
    pub path: String,
    /// file modification time, unix seconds
    pub mtime: i64,
    pub size: i64,
}

#[derive(Serialize)]
pub struct DupGroup {
    pub keep: DupFile,
    pub remove: Vec<DupFile>,
}

/// Sets of identical files, each with the copy kept (oldest file date, then path) and the
/// others; the largest savings first.
pub fn report(conn: &Connection) -> Result<Vec<DupGroup>> {
    let rows: Vec<(String, DupFile)> = conn
        .prepare(
            "SELECT content_hash, id, path, mtime, size FROM photos
             WHERE content_hash IN (SELECT content_hash FROM photos WHERE content_hash IS NOT NULL
                                    GROUP BY content_hash HAVING COUNT(*) > 1)
             ORDER BY content_hash, mtime, path",
        )?
        .query_map([], |r| Ok((r.get(0)?, DupFile { id: r.get(1)?, path: r.get(2)?, mtime: r.get(3)?, size: r.get(4)? })))?
        .collect::<Result<_, _>>()?;
    let mut by_hash: Vec<(String, Vec<DupFile>)> = Vec::new();
    for (hash, file) in rows {
        match by_hash.last_mut() {
            Some((h, files)) if *h == hash => files.push(file),
            _ => by_hash.push((hash, vec![file])),
        }
    }
    let mut groups: Vec<DupGroup> = by_hash
        .into_iter()
        .filter(|(_, files)| files.len() > 1)
        .map(|(_, mut files)| {
            let keep = files.remove(0);
            DupGroup { keep, remove: files }
        })
        .collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.remove.iter().map(|f| f.size).sum::<i64>()));
    Ok(groups)
}

#[derive(Serialize, Default)]
pub struct DeleteResult {
    pub binned: usize,
    pub deleted: usize,
    pub freed: i64,
    /// files whose drive has no bin: deleted only when asked again with `permanently`
    pub no_bin: Vec<i64>,
    /// files left alone because something changed since the search
    pub skipped: Vec<String>,
}

/// Removes the duplicate copies (all, or only `only`), keeping the oldest of each set. Each
/// file is checked first: it and the copy that is kept must still be there and unchanged,
/// and it must be inside the photo folders.
/// Progress goes to `status` (files done of total, bytes freed).
pub fn delete(
    conn: &mut Connection,
    roots: &[PathBuf],
    only: Option<&[i64]>,
    permanently: bool,
    status: &DupStatus,
) -> Result<DeleteResult> {
    let mut result = DeleteResult::default();
    let mut gone = Vec::new();
    let only: Option<std::collections::HashSet<i64>> = only.map(|ids| ids.iter().copied().collect());
    let groups: Vec<(DupFile, Vec<DupFile>)> = report(conn)?
        .into_iter()
        .map(|g| (g.keep, g.remove.into_iter().filter(|f| only.as_ref().is_none_or(|ids| ids.contains(&f.id))).collect::<Vec<_>>()))
        .filter(|(_, remove)| !remove.is_empty())
        .collect();
    status.delete_done.store(0, Ordering::Relaxed);
    status.delete_freed.store(0, Ordering::Relaxed);
    status.delete_total.store(groups.iter().map(|(_, r)| r.len() as u64).sum(), Ordering::Relaxed);
    for (keep, remove) in &groups {
        let keep_ok = unchanged(Path::new(&keep.path), keep.mtime, keep.size);
        for file in remove {
            status.delete_done.fetch_add(1, Ordering::Relaxed);
            let path = PathBuf::from(&file.path);
            if !keep_ok || !unchanged(&path, file.mtime, file.size) || !roots.iter().any(|r| path.starts_with(r)) {
                result.skipped.push(file.path.clone());
                continue;
            }
            if permanently {
                match std::fs::remove_file(&path) {
                    Ok(()) => result.deleted += 1,
                    Err(e) => {
                        tracing::warn!("{}: {e}", file.path);
                        result.skipped.push(file.path.clone());
                        continue;
                    }
                }
            } else if let Err(e) = trash::delete(&path) {
                tracing::warn!("{}: can't move to the bin: {e}", file.path);
                result.no_bin.push(file.id);
                continue;
            } else {
                result.binned += 1;
            }
            result.freed += file.size;
            status.delete_freed.fetch_add(file.size as u64, Ordering::Relaxed);
            gone.push(file.id);
        }
    }
    // Their faces are the same as the kept copy's, so the people only lose the duplicates.
    crate::db::forget_photos(conn, &gone)?;
    tracing::info!("duplicates: {} to the bin, {} deleted, {} bytes freed", result.binned, result.deleted, result.freed);
    Ok(result)
}

/// Summary for the status line: how many duplicate files and bytes.
pub fn totals(groups: &[DupGroup]) -> (usize, i64) {
    let files = groups.iter().map(|g| g.remove.len()).sum();
    let bytes = groups.iter().flat_map(|g| &g.remove).map(|f| f.size).sum();
    (files, bytes)
}
