//! Finding copies of the same photo or video to free disk space: the very same file, or the
//! same picture with other details, such as a copy uploaded from a phone's browser, which
//! Android hands over without its location.
//!
//! The fingerprint (BLAKE3) of a JPEG is that of its picture, leaving out the metadata
//! segments (EXIF, XMP...) but not the orientation, which changes how it looks; of an MP4 or
//! MOV, that of its media, leaving out the `moov` box (where its date and place are); of other
//! files, that of the whole file. Only files that could have a twin are read: those that share
//! their size with another, or their size in pixels and capture time. The fingerprint is
//! stored with the item and kept until the file changes. Of each set, the copy with a place
//! is kept, then the one with the oldest file date (ties: the first by path), and the others
//! can be moved to the bin.

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
    let _running = crate::scan::ClearOnDrop(&status.running);
    let result = fingerprint(db_path, status);
    *status.finished.lock().unwrap() = Some(chrono::Local::now().format("%Y-%m-%d %H:%M").to_string());
    result
}

fn fingerprint(db_path: &Path, status: &DupStatus) -> Result<()> {
    let conn = crate::db::open(db_path)?;
    let todo: Vec<(i64, String, i64, i64)> = conn
        .prepare(
            "SELECT id, path, mtime, size FROM items WHERE content_hash IS NULL
             AND (size IN (SELECT size FROM items GROUP BY size HAVING COUNT(*) > 1)
                  OR (width, height, taken) IN (SELECT width, height, taken FROM items
                                                GROUP BY width, height, taken HAVING COUNT(*) > 1))",
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
    let mut save = conn.prepare("UPDATE items SET content_hash = ? WHERE id = ?")?;
    conn.execute_batch("BEGIN")?;
    for (id, hash) in &hashes {
        save.execute(params![hash, id])?;
    }
    conn.execute_batch("COMMIT")?;
    tracing::info!("fingerprinted {} possible duplicates", hashes.len());
    Ok(())
}

/// The item's fingerprint: of its picture or media when it is a JPEG, MP4 or MOV, else of the
/// whole file (also when those can't be read as such).
pub(crate) fn hash_file(path: &Path) -> std::io::Result<String> {
    let ext = crate::imaging::extension(path);
    let mut hasher = blake3::Hasher::new();
    let mut file = std::io::BufReader::with_capacity(256 * 1024, File::open(path)?);
    let ok = match ext.as_str() {
        "jpg" | "jpeg" => hash_jpeg(&mut file, &mut hasher, path).is_ok(),
        "mp4" | "mov" | "m4v" => hash_media(&mut file, &mut hasher).is_ok(),
        _ => false,
    };
    if !ok {
        hasher = blake3::Hasher::new();
        let mut file = File::open(path)?;
        std::io::copy(&mut file, &mut hasher)?;
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// A JPEG without its metadata: every segment but APP0-APP15 and comments, then the picture
/// data from the start of the scan to the end, plus the EXIF orientation.
fn hash_jpeg(file: &mut impl std::io::BufRead, hasher: &mut blake3::Hasher, path: &Path) -> std::io::Result<()> {
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "not a JPEG");
    let mut marker = [0u8; 2];
    file.read_exact(&mut marker)?;
    if marker != [0xFF, 0xD8] {
        return Err(bad());
    }
    hasher.update(b"jpeg");
    loop {
        file.read_exact(&mut marker)?;
        if marker[0] != 0xFF {
            return Err(bad());
        }
        let kind = marker[1];
        if kind == 0xD8 || (0xD0..=0xD7).contains(&kind) || kind == 0x01 || kind == 0xFF {
            continue; // markers without a length
        }
        let mut len = [0u8; 2];
        file.read_exact(&mut len)?;
        let len = u16::from_be_bytes(len) as u64;
        if len < 2 {
            return Err(bad());
        }
        if (0xE0..=0xEF).contains(&kind) || kind == 0xFE {
            // Metadata: left out.
            std::io::copy(&mut file.by_ref().take(len - 2), &mut std::io::sink())?;
            continue;
        }
        hasher.update(&marker);
        hasher.update(&(len as u16).to_be_bytes());
        std::io::copy(&mut file.by_ref().take(len - 2), hasher)?;
        if kind == 0xDA {
            // The scan: the picture itself, to the end of the file.
            std::io::copy(file, hasher)?;
            break;
        }
    }
    // Two copies that only differ in their orientation look different: not the same photo.
    let orientation = std::fs::File::open(path).ok().and_then(|f| {
        let exif = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(f)).ok()?;
        exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?.value.get_uint(0)
    });
    hasher.update(&orientation.unwrap_or(1).to_le_bytes());
    Ok(())
}

/// An MP4 or MOV without its `moov` box, which holds its details (date, place, tracks' names)
/// as well as the tables describing the media: the media data and the other top-level boxes.
fn hash_media(file: &mut impl std::io::BufRead, hasher: &mut blake3::Hasher) -> std::io::Result<()> {
    hasher.update(b"media");
    let mut seen_media = false;
    loop {
        let mut head = [0u8; 8];
        match file.read_exact(&mut head) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e),
        }
        let kind = [head[4], head[5], head[6], head[7]];
        let (size, header) = match u32::from_be_bytes([head[0], head[1], head[2], head[3]]) {
            1 => {
                let mut large = [0u8; 8];
                file.read_exact(&mut large)?;
                (u64::from_be_bytes(large), 16)
            }
            0 => (u64::MAX, 8), // to the end of the file
            n => (n as u64, 8),
        };
        if size != u64::MAX && size < header {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not an MP4 box"));
        }
        let body = size.saturating_sub(header);
        let mut part = file.by_ref().take(body);
        if matches!(&kind, b"moov" | b"free" | b"skip" | b"wide" | b"udta" | b"meta" | b"uuid") {
            std::io::copy(&mut part, &mut std::io::sink())?;
        } else {
            seen_media |= &kind == b"mdat";
            hasher.update(&kind);
            std::io::copy(&mut part, hasher)?;
        }
        if size == u64::MAX {
            break;
        }
    }
    if seen_media { Ok(()) } else { Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "no media data")) }
}

/// The file is still the one that was indexed (same size and modification time).
fn unchanged(path: &Path, mtime: i64, size: i64) -> bool {
    std::fs::metadata(path).is_ok_and(|m| {
        m.len() as i64 == size
            && m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64)
                == Some(mtime)
    })
}

#[derive(Serialize, Clone)]
pub struct DupFile {
    pub id: i64,
    pub path: String,
    /// file modification time, unix seconds
    pub mtime: i64,
    pub size: i64,
    /// for the thumbnail's address (`/thumb/{id}/{version}`)
    pub version: i64,
}

#[derive(Serialize)]
pub struct DupGroup {
    pub keep: DupFile,
    pub remove: Vec<DupFile>,
}

/// Sets of copies of the same item, each with the copy kept (one with a place, then the oldest
/// file date, then the path) and the others; the largest savings first.
pub fn report(conn: &Connection) -> Result<Vec<DupGroup>> {
    let rows: Vec<(String, DupFile)> = conn
        .prepare(
            "SELECT content_hash, id, path, mtime, size, version FROM items
             WHERE content_hash IN (SELECT content_hash FROM items WHERE content_hash IS NOT NULL
                                    GROUP BY content_hash HAVING COUNT(*) > 1)
             ORDER BY content_hash, lat IS NULL, mtime, path",
        )?
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                DupFile { id: r.get(1)?, path: r.get(2)?, mtime: r.get(3)?, size: r.get(4)?, version: r.get(5)? },
            ))
        })?
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
        .map(|g| {
            (
                g.keep,
                g.remove
                    .into_iter()
                    .filter(|f| only.as_ref().is_none_or(|ids| ids.contains(&f.id)))
                    .collect::<Vec<_>>(),
            )
        })
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
            if !keep_ok || !unchanged(&path, file.mtime, file.size) || crate::library::root_of(roots, &path).is_none() {
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
    crate::db::forget_items(conn, &gone)?;
    tracing::info!(
        "duplicates: {} to the bin, {} deleted, {} bytes freed",
        result.binned,
        result.deleted,
        result.freed
    );
    Ok(result)
}

/// Summary for the status line: how many duplicate files and bytes.
pub fn totals(groups: &[DupGroup]) -> (usize, i64) {
    let files = groups.iter().map(|g| g.remove.len()).sum();
    let bytes = groups.iter().flat_map(|g| &g.remove).map(|f| f.size).sum();
    (files, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Library, Photo, set_mtime};

    /// Three copies of one photo (b the oldest file), a different photo, and the search run.
    fn library() -> Library {
        let lib = Library::new();
        let copy = Photo::default();
        for (rel, age) in [("a.jpg", 300), ("b.jpg", 100), ("sub/c.jpg", 200)] {
            set_mtime(&lib.add(rel, copy), age);
        }
        lib.add("other.jpg", Photo { color: [9, 9, 9], ..Photo::default() });
        lib.scan();
        run(&lib.cfg.db_path, &DupStatus::default()).unwrap();
        lib
    }

    fn names(files: &[DupFile]) -> Vec<String> {
        files.iter().map(|f| Path::new(&f.path).file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn the_oldest_copy_is_kept() {
        let lib = library();
        let groups = report(&lib.conn()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(names(std::slice::from_ref(&groups[0].keep)), ["b.jpg"]);
        assert_eq!(names(&groups[0].remove), ["c.jpg", "a.jpg"], "then by age");
        assert_eq!(totals(&groups), (2, 2 * groups[0].keep.size));
    }

    #[test]
    fn deleting_removes_the_copies_and_their_rows() {
        let lib = library();
        let result = delete(&mut lib.conn(), &[lib.root()], None, true, &DupStatus::default()).unwrap();
        assert_eq!((result.deleted, result.binned, result.skipped.len()), (2, 0, 0));
        assert!(lib.root().join("b.jpg").exists());
        assert!(!lib.root().join("a.jpg").exists() && !lib.root().join("sub/c.jpg").exists());
        assert!(lib.root().join("other.jpg").exists());
        let rows: i64 = lib.conn().query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 2);
        assert!(report(&lib.conn()).unwrap().is_empty());
    }

    #[test]
    fn only_the_chosen_copies_are_deleted() {
        let lib = library();
        let a = lib.id("a.jpg");
        let result = delete(&mut lib.conn(), &[lib.root()], Some(&[a]), true, &DupStatus::default()).unwrap();
        assert_eq!(result.deleted, 1);
        assert!(!lib.root().join("a.jpg").exists() && lib.root().join("sub/c.jpg").exists());
    }

    #[test]
    fn changed_or_outside_files_are_left_alone() {
        let lib = library();
        // c changed since the search; a is outside the folders the deletion may touch.
        std::fs::write(lib.root().join("sub/c.jpg"), Photo { color: [1, 1, 1], ..Photo::default() }.jpeg()).unwrap();
        let result = delete(&mut lib.conn(), &[lib.root().join("sub")], None, true, &DupStatus::default()).unwrap();
        assert_eq!(result.deleted, 0);
        assert_eq!(result.skipped.len(), 2);
        assert!(lib.root().join("a.jpg").exists() && lib.root().join("sub/c.jpg").exists());

        // If the copy that would be kept changed, nothing is deleted.
        let lib = library();
        std::fs::write(lib.root().join("b.jpg"), b"changed").unwrap();
        let result = delete(&mut lib.conn(), &[lib.root()], None, true, &DupStatus::default()).unwrap();
        assert_eq!((result.deleted, result.skipped.len()), (0, 2));
    }

    #[test]
    fn a_copy_without_its_place_is_the_same_photo_and_the_one_with_it_is_kept() {
        let lib = Library::new();
        let taken = Some("2025:08:01 10:00:00");
        // Uploaded from a phone's browser (Android leaves the place out), and older...
        set_mtime(&lib.add("upload.jpg", Photo { taken, ..Photo::default() }), 500);
        // ...and synced from the phone with its place, newer.
        set_mtime(&lib.add("synced.jpg", Photo { taken, gps: Some((40.4168, -3.7038)), ..Photo::default() }), 100);
        // The same picture, turned: it looks different, so it isn't a copy.
        lib.add("turned.jpg", Photo { taken, orientation: Some(6), ..Photo::default() });
        lib.scan();
        run(&lib.cfg.db_path, &DupStatus::default()).unwrap();
        let groups = report(&lib.conn()).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(names(std::slice::from_ref(&groups[0].keep)), ["synced.jpg"]);
        assert_eq!(names(&groups[0].remove), ["upload.jpg"]);
    }

    #[test]
    fn videos_are_compared_without_their_details() {
        let dir = crate::testutil::TempDir::new();
        let mp4 = |details: &[u8], media: &[u8]| {
            let boxed = |kind: &[u8], body: &[u8]| [&((body.len() + 8) as u32).to_be_bytes()[..], kind, body].concat();
            [boxed(b"ftyp", b"isom\0\0\0\0"), boxed(b"moov", details), boxed(b"mdat", media)].concat()
        };
        let write = |name: &str, bytes: Vec<u8>| {
            let path = dir.file(name);
            std::fs::write(&path, bytes).unwrap();
            hash_file(&path).unwrap()
        };
        let with_place = write("a.mp4", mp4(b"udta with a place", b"frames"));
        let without = write("b.mp4", mp4(b"udta", b"frames"));
        let other = write("c.mp4", mp4(b"udta", b"other frames"));
        assert_eq!(with_place, without);
        assert_ne!(without, other);
        // Not an MP4 inside: the whole file counts.
        assert_eq!(write("d.mp4", b"hello".to_vec()), blake3::hash(b"hello").to_hex().to_string());
    }
}
