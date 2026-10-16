//! Backups of the index: everything the gallery knows that the photos don't say themselves
//! (people and their names, face groups, the user's corrections, hidden and removed items,
//! the folders), to put back on this computer or another one.
//!
//! A backup is a copy of the index (SQLite) without the photos' thumbnails, which are made
//! again from the photos (see `scan::fill_thumbnails`), so it stays small. Videos keep theirs:
//! only a browser that plays the video can make one. A `backup_info` table says what it is.
//!
//! Restoring writes the backup into the live index with SQLite's backup API, so the
//! connections already open see the new contents and no file is swapped under them. The
//! index it replaces is kept beside it (`index.before-restore.sqlite`), once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Serialize;

/// The file name extension of backups.
pub const EXTENSION: &str = "imadive";
/// The layout of `backup_info`; a backup with a higher one is refused.
const FORMAT: i64 = 1;
/// The copy of the index a restore replaced.
pub const BEFORE_RESTORE: &str = "index.before-restore.sqlite";

/// A backup's file name for today: `Imadive backup 2026-10-16.imadive`.
pub fn file_name() -> String {
    format!("Imadive backup {}.{EXTENSION}", chrono::Local::now().format("%Y-%m-%d"))
}

/// Progress of the backup being made, for the page to follow: rows copied of all the rows,
/// and the file it went to (or why it failed) once it is done.
#[derive(Default)]
pub struct Job {
    pub running: AtomicBool,
    pub done: AtomicU64,
    pub total: AtomicU64,
    outcome: Mutex<Option<Result<String, String>>>,
}

#[derive(Serialize)]
pub struct JobView {
    pub running: bool,
    pub done: u64,
    pub total: u64,
    /// Where the last backup went, once it is done.
    pub path: Option<String>,
    pub error: Option<String>,
}

impl Job {
    pub fn view(&self) -> JobView {
        let outcome = self.outcome.lock().unwrap().clone();
        JobView {
            running: self.running.load(Ordering::SeqCst),
            done: self.done.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            path: outcome.as_ref().and_then(|o| o.as_ref().ok().cloned()),
            error: outcome.and_then(|o| o.err()),
        }
    }

    /// Makes a backup of `db_path` at `out` on a thread of its own. `false` when one is
    /// already being made.
    pub fn start(self: &Arc<Self>, db_path: PathBuf, fixed_roots: Vec<PathBuf>, out: PathBuf) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.done.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        *self.outcome.lock().unwrap() = None;
        let job = self.clone();
        std::thread::spawn(move || {
            let result = create(&db_path, &fixed_roots, &out, &job);
            let outcome = match result {
                Ok(()) => {
                    tracing::info!("backup saved in {}", out.display());
                    Ok(out.to_string_lossy().into_owned())
                }
                Err(e) => {
                    tracing::error!("making a backup in {}: {e:#}", out.display());
                    Err(format!("{e:#}"))
                }
            };
            *job.outcome.lock().unwrap() = Some(outcome);
            job.running.store(false, Ordering::SeqCst);
        });
        true
    }
}

/// `name` in `folder`, or `name (2)`, `name (3)`... when a file already has it.
pub fn unique_path(folder: &Path, name: &str) -> PathBuf {
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let mut path = folder.join(name);
    let mut n = 2;
    while path.exists() {
        path = folder.join(format!("{stem} ({n}).{ext}"));
        n += 1;
    }
    path
}

/// The tables a backup copies, with the rows it leaves out: the photos' thumbnails.
const TABLES: &[(&str, &str)] = &[
    ("excluded", ""),
    ("failures", ""),
    ("folders", ""),
    ("places", ""),
    ("items", ""),
    ("thumbs", "AND item_id IN (SELECT id FROM src.items WHERE duration IS NOT NULL)"),
    ("persons", ""),
    ("faces", ""),
];
/// Rows copied at a time, so the progress moves steadily.
const CHUNK: i64 = 2000;

/// Writes a backup of the index at `db_path` to `out`. The command-line folders
/// (`fixed_roots`) go in it too, so a restore elsewhere has every folder. Only what the backup
/// keeps is read and written, inside one read of the index (a consistent copy, while the
/// gallery keeps reading and scanning). `progress` counts the rows.
pub fn create(db_path: &Path, fixed_roots: &[PathBuf], out: &Path, progress: &Job) -> Result<()> {
    let partial = out.with_extension("partial");
    let _ = std::fs::remove_file(&partial);
    let result = (|| -> Result<()> {
        if let Some(ms) = std::env::var("IMADIVE_TEST_BACKUP_DELAY_MS").ok().and_then(|v| v.parse().ok()) {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
        let mut copy = Connection::open(&partial).with_context(|| format!("writing {}", partial.display()))?;
        crate::db::init(&copy)?;
        copy.execute("ATTACH DATABASE ? AS src", [db_path.to_string_lossy()])?;
        copy.execute_batch("PRAGMA src.busy_timeout = 10000;")?;
        let tx = copy.transaction()?;
        let schema: i64 = tx.query_row("PRAGMA src.user_version", [], |r| r.get(0))?;
        tx.execute_batch(&format!("PRAGMA main.user_version = {schema};"))?;
        let mut total = 0;
        for (table, only) in TABLES {
            total +=
                tx.query_row(&format!("SELECT COUNT(*) FROM src.{table} WHERE 1 {only}"), [], |r| r.get::<_, i64>(0))?;
        }
        progress.total.store(total as u64, Ordering::Relaxed);
        for (table, only) in TABLES {
            // By name: an index upgraded from an older version has its later columns in
            // another order than a new one.
            let columns: Vec<String> = tx
                .prepare(&format!("SELECT name FROM pragma_table_info('{table}', 'main')"))?
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            let columns = columns.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
            let (low, high): (Option<i64>, Option<i64>) =
                tx.query_row(&format!("SELECT MIN(rowid), MAX(rowid) FROM src.{table}"), [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
            let (Some(low), Some(high)) = (low, high) else { continue };
            let mut from = low;
            while from <= high {
                let copied = tx.execute(
                    &format!(
                        "INSERT INTO main.{table} ({columns}) SELECT {columns} FROM src.{table} WHERE rowid BETWEEN ?1 AND ?2 {only}"
                    ),
                    params![from, from + CHUNK - 1],
                )?;
                progress.done.fetch_add(copied as u64, Ordering::Relaxed);
                from += CHUNK;
            }
        }
        let fixed: Vec<String> = fixed_roots.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        tx.execute_batch("CREATE TABLE backup_info (key TEXT PRIMARY KEY, value TEXT NOT NULL);")?;
        for (key, value) in [
            ("app", "imadive".to_string()),
            ("format", FORMAT.to_string()),
            ("schema", schema.to_string()),
            ("version", env!("CARGO_PKG_VERSION").to_string()),
            ("created", chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()),
            ("fixed_folders", serde_json::to_string(&fixed)?),
        ] {
            tx.execute("INSERT INTO backup_info (key, value) VALUES (?, ?)", params![key, value])?;
        }
        tx.commit()?;
        copy.execute_batch("DETACH DATABASE src;")?;
        drop(copy);
        std::fs::rename(&partial, out).with_context(|| format!("saving {}", out.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

/// What a backup holds, shown before it is restored.
#[derive(Debug, Serialize)]
pub struct Summary {
    /// When it was made (local time) and by which version.
    pub created: String,
    pub version: String,
    pub photos: i64,
    pub videos: i64,
    pub people: i64,
    pub named: i64,
    pub folders: Vec<BackupFolder>,
}

#[derive(Debug, Serialize)]
pub struct BackupFolder {
    pub path: String,
    /// The folder exists on this computer. Its photos are kept either way; one that isn't
    /// here shows as unavailable until it's back or moved ("Moved to…").
    pub here: bool,
}

/// Why a file can't be restored.
#[derive(Debug)]
pub enum Refused {
    NotABackup,
    /// Made by a newer Imadive (this version).
    Newer(String),
}

/// Checks that `path` is a backup this version can restore, brings one made by an older
/// version up to date (it is changed in place), and says what it holds.
pub fn inspect(path: &Path) -> Result<Result<Summary, Refused>> {
    let info = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(anyhow::Error::from)
        .and_then(|conn| read_info(&conn));
    let Ok(info) = info else { return Ok(Err(Refused::NotABackup)) };
    if info.get("app").map(String::as_str) != Some("imadive") {
        return Ok(Err(Refused::NotABackup));
    }
    let number = |key: &str| info.get(key).and_then(|v| v.parse::<i64>().ok()).unwrap_or(i64::MAX);
    let version = info.get("version").cloned().unwrap_or_default();
    if number("format") > FORMAT || number("schema") > crate::db::SCHEMA_VERSION {
        return Ok(Err(Refused::Newer(version)));
    }
    let conn = crate::db::open(path)?;
    let count = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
    let mut folders: Vec<String> = conn
        .prepare("SELECT path FROM folders ORDER BY path")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    folders.extend(fixed_folders(&info));
    folders.sort();
    folders.dedup();
    Ok(Ok(Summary {
        created: info.get("created").cloned().unwrap_or_default(),
        version,
        photos: count("SELECT COUNT(*) FROM items WHERE duration IS NULL")?,
        videos: count("SELECT COUNT(*) FROM items WHERE duration IS NOT NULL")?,
        people: count("SELECT COUNT(*) FROM persons WHERE id IN (SELECT person_id FROM faces) OR name IS NOT NULL")?,
        named: count("SELECT COUNT(*) FROM persons WHERE name IS NOT NULL")?,
        folders: folders
            .into_iter()
            .map(|path| {
                let here = Path::new(&path).is_dir();
                BackupFolder { path, here }
            })
            .collect(),
    }))
}

/// Replaces the index at `db_path` with the backup at `backup` (checked with [`inspect`]).
/// The backup's command-line folders become saved folders here, unless they are command-line
/// folders here too (`fixed_roots`). Scans must not run meanwhile.
pub fn restore(db_path: &Path, fixed_roots: &[PathBuf], backup: &Path) -> Result<()> {
    let mut live = crate::db::open(db_path)?;
    // The one way back from a restore that wasn't wanted.
    let before = db_path.with_file_name(BEFORE_RESTORE);
    let _ = std::fs::remove_file(&before);
    live.execute("VACUUM INTO ?", [before.to_string_lossy()])?;
    live.restore(rusqlite::MAIN_DB, backup, None::<fn(rusqlite::backup::Progress)>)?;
    let info = read_info(&live)?;
    let tx = live.transaction()?;
    for folder in fixed_folders(&info) {
        if !fixed_roots.iter().any(|r| r == Path::new(&folder)) {
            tx.execute("INSERT OR IGNORE INTO folders (path) VALUES (?)", [&folder])?;
        }
    }
    tx.execute_batch("DROP TABLE backup_info;")?;
    tx.commit()?;
    // The backup was made by this version or brought up to date by `inspect`; this checks.
    crate::db::init(&live)?;
    Ok(())
}

fn read_info(conn: &Connection) -> Result<std::collections::HashMap<String, String>> {
    let rows = conn
        .prepare("SELECT key, value FROM backup_info")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn fixed_folders(info: &std::collections::HashMap<String, String>) -> Vec<String> {
    info.get("fixed_folders").and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default()
}

/// The photos (not videos) without a thumbnail, as after a restore: id and path.
pub fn photos_without_thumbnail(conn: &Connection) -> Result<Vec<(i64, String)>> {
    Ok(conn
        .prepare(
            "SELECT id, path FROM items WHERE duration IS NULL
             AND NOT EXISTS (SELECT 1 FROM thumbs WHERE item_id = items.id) ORDER BY taken DESC",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?)
}

/// Makes the thumbnail of photo `id` from its file and stores it. `None` when it isn't a
/// photo of the index.
pub fn make_thumbnail(conn: &Connection, id: i64) -> Result<Option<Vec<u8>>> {
    let path: Option<String> =
        conn.query_row("SELECT path FROM items WHERE id = ? AND duration IS NULL", [id], |r| r.get(0)).optional()?;
    let Some(path) = path else { return Ok(None) };
    let thumb = crate::scan::photo_thumbnail(Path::new(&path))?;
    conn.execute("INSERT OR IGNORE INTO thumbs (item_id, data) VALUES (?, ?)", params![id, thumb])?;
    Ok(Some(thumb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Library, Photo, TempDir};

    fn library_with_people() -> Library {
        let lib = Library::new();
        lib.add("2024/a.jpg", Photo::default());
        lib.add("2024/b.jpg", Photo::default());
        lib.scan();
        let conn = lib.conn();
        conn.execute_batch(
            "INSERT INTO persons (id, name) VALUES (1, 'Ana'), (2, NULL);
             INSERT INTO folders (path) VALUES ('/somewhere/else');",
        )
        .unwrap();
        lib
    }

    #[test]
    fn a_restore_brings_back_names_and_folders_without_photo_thumbnails() {
        let lib = library_with_people();
        let out = lib.dir.file("backup.imadive");
        create(&lib.cfg.db_path, &lib.cfg.fixed_roots, &out, &Job::default()).unwrap();

        // Things change after the backup...
        lib.conn().execute_batch("UPDATE persons SET name = 'Someone' WHERE id = 1; DELETE FROM folders;").unwrap();

        let summary = inspect(&out).unwrap().unwrap();
        assert_eq!((summary.photos, summary.videos, summary.named), (2, 0, 1));
        assert_eq!(summary.version, env!("CARGO_PKG_VERSION"));
        let root = lib.root().to_string_lossy().into_owned();
        let paths: Vec<_> = summary.folders.iter().map(|f| (f.path.as_str(), f.here)).collect();
        assert!(paths.contains(&(root.as_str(), true)), "{paths:?}");
        assert!(paths.contains(&("/somewhere/else", false)), "{paths:?}");

        // ...and come back. Restored on a gallery without command-line folders, the backup's
        // command-line folder becomes a saved one.
        restore(&lib.cfg.db_path, &[], &out).unwrap();
        let conn = lib.conn();
        let name: Option<String> = conn.query_row("SELECT name FROM persons WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(name.as_deref(), Some("Ana"));
        let mut folders: Vec<String> = conn
            .prepare("SELECT path FROM folders")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        folders.sort();
        let mut expected = vec![root.clone(), "/somewhere/else".to_string()];
        expected.sort();
        assert_eq!(folders, expected);
        let thumbs: i64 = conn.query_row("SELECT COUNT(*) FROM thumbs", [], |r| r.get(0)).unwrap();
        assert_eq!(thumbs, 0);
        let info: i64 =
            conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'backup_info'", [], |r| r.get(0)).unwrap();
        assert_eq!(info, 0);
        // The index it replaced is kept beside it.
        assert!(lib.cfg.db_path.with_file_name(BEFORE_RESTORE).is_file());

        // The thumbnails are made again from the photos.
        let missing = photos_without_thumbnail(&conn).unwrap();
        assert_eq!(missing.len(), 2);
        let jpeg = make_thumbnail(&conn, missing[0].0).unwrap().unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8]));
        assert_eq!(photos_without_thumbnail(&conn).unwrap().len(), 1);
    }

    #[test]
    fn command_line_folders_here_stay_command_line_folders() {
        let lib = library_with_people();
        let out = lib.dir.file("backup.imadive");
        create(&lib.cfg.db_path, &lib.cfg.fixed_roots, &out, &Job::default()).unwrap();
        inspect(&out).unwrap().unwrap();
        restore(&lib.cfg.db_path, &lib.cfg.fixed_roots, &out).unwrap();
        let folders: Vec<String> = lib
            .conn()
            .prepare("SELECT path FROM folders")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(folders, vec!["/somewhere/else".to_string()]);
    }

    #[test]
    fn an_index_upgraded_from_an_old_version_is_copied_column_by_column() {
        // Before 0.2 the index had fewer columns; upgrading added them at the end, in another
        // order than a new index has them (version before content_hash).
        let dir = TempDir::new();
        let old = dir.file("data/index.sqlite");
        Connection::open(&old)
            .unwrap()
            .execute_batch(
                "CREATE TABLE items (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL UNIQUE,
                    mtime INTEGER NOT NULL, size INTEGER NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL,
                    taken TEXT NOT NULL, date_from_exif INTEGER NOT NULL, lat REAL, lon REAL, place_id INTEGER);",
            )
            .unwrap();
        crate::db::open(&old)
            .unwrap()
            .execute_batch(
                "INSERT INTO items (path, mtime, size, width, height, taken, date_from_exif, version, content_hash)
                 VALUES ('/p/a.jpg', 1, 2, 3, 4, '2020-01-01 10:00:00', 1, 7, 'abc');",
            )
            .unwrap();
        let out = dir.file("backup.imadive");
        create(&old, &[], &out, &Job::default()).unwrap();
        inspect(&out).unwrap().unwrap();
        let copy = Connection::open(&out).unwrap();
        let (version, hash): (i64, String) =
            copy.query_row("SELECT version, content_hash FROM items", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((version, hash.as_str()), (7, "abc"));
    }

    #[test]
    fn a_second_backup_the_same_day_gets_a_number() {
        let dir = TempDir::new();
        let first = unique_path(dir.path(), "Imadive backup 2026-10-16.imadive");
        assert!(first.ends_with("Imadive backup 2026-10-16.imadive"));
        std::fs::write(&first, "").unwrap();
        assert!(
            unique_path(dir.path(), "Imadive backup 2026-10-16.imadive")
                .ends_with("Imadive backup 2026-10-16 (2).imadive")
        );
    }

    #[test]
    fn other_files_and_newer_backups_are_refused() {
        let dir = TempDir::new();
        let text = dir.file("notes.imadive");
        std::fs::write(&text, "hello").unwrap();
        assert!(matches!(inspect(&text).unwrap(), Err(Refused::NotABackup)));

        // An index, but not a backup.
        let index = dir.file("index.sqlite");
        crate::db::open(&index).unwrap();
        assert!(matches!(inspect(&index).unwrap(), Err(Refused::NotABackup)));

        let lib = library_with_people();
        let out = lib.dir.file("backup.imadive");
        create(&lib.cfg.db_path, &lib.cfg.fixed_roots, &out, &Job::default()).unwrap();
        Connection::open(&out)
            .unwrap()
            .execute("UPDATE backup_info SET value = '99' WHERE key = 'schema'", [])
            .unwrap();
        assert!(matches!(inspect(&out).unwrap(), Err(Refused::Newer(_))));
    }
}
