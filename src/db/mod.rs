//! The index: one SQLite database with the photos, their thumbnails, faces, people and the
//! user's corrections.
//!
//! Who owns what:
//! - `photos` rows (with their `thumbs` and `faces`, which cascade) are written by the scan;
//!   handlers only delete them through [`forget_photos`], which remembers named people first.
//! - `persons` are created by face grouping (`cluster`) and by the user ("Not them",
//!   naming). An unnamed person with no faces left is deleted; a named one is kept with its
//!   `face_memory` (average face), so the name returns when matching photos come back.
//! - `faces.rejected` marks faces the user placed ("Not them", "Same as"): grouping never
//!   moves them. `faces.grouped` is 0 until a grouping pass has seen the face.
//! - `persons.cover_face` is the face the user chose for the person's card, used while it
//!   still belongs to them.
//! - `folders` are the photo folders added in Settings; `excluded` are photos removed from
//!   the gallery (their files stay); `failures` are files that could not be read.

pub mod filters;
mod people;
pub mod photos;

pub use people::*;

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, params};

const SCHEMA: &str = "
-- photos removed from the gallery by the user (the files stay): scans skip them
CREATE TABLE IF NOT EXISTS excluded (
    path TEXT PRIMARY KEY
);

-- files that could not be indexed, with the reason; retried when the file changes
CREATE TABLE IF NOT EXISTS failures (
    path  TEXT PRIMARY KEY,
    mtime INTEGER NOT NULL,
    size  INTEGER NOT NULL,
    error TEXT NOT NULL
);

-- photo folders added from the UI (used when none are given on the command line)
CREATE TABLE IF NOT EXISTS folders (
    path TEXT PRIMARY KEY
);

CREATE TABLE IF NOT EXISTS places (
    id      INTEGER PRIMARY KEY,
    city    TEXT NOT NULL,
    region  TEXT NOT NULL,
    country TEXT NOT NULL,
    lat     REAL NOT NULL,
    lon     REAL NOT NULL,
    UNIQUE (city, region, country)
);

CREATE TABLE IF NOT EXISTS photos (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    path      TEXT NOT NULL UNIQUE,
    mtime     INTEGER NOT NULL,
    size      INTEGER NOT NULL,
    width     INTEGER NOT NULL,
    height    INTEGER NOT NULL,
    -- local capture time 'YYYY-MM-DD HH:MM:SS' (EXIF DateTimeOriginal, else file mtime)
    taken     TEXT NOT NULL,
    date_from_exif INTEGER NOT NULL,
    lat       REAL,
    lon       REAL,
    place_id  INTEGER REFERENCES places(id),
    -- 0 when indexed without face recognition, so a later run can add the faces
    faces_scanned INTEGER NOT NULL DEFAULT 1,
    -- BLAKE3 of the file, only for photos that share their size with another (duplicates)
    content_hash TEXT,
    -- goes up when the file is changed here (rotated), so browsers fetch the new thumbnail
    version INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS photos_taken ON photos(taken);
CREATE INDEX IF NOT EXISTS photos_md ON photos(substr(taken, 6, 5));
CREATE INDEX IF NOT EXISTS photos_place ON photos(place_id);
CREATE INDEX IF NOT EXISTS photos_size ON photos(size);

CREATE TABLE IF NOT EXISTS thumbs (
    photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
    data     BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS persons (
    id     INTEGER PRIMARY KEY,
    -- NULL until the user names the person; named persons keep their faces across re-clustering
    name   TEXT,
    hidden INTEGER NOT NULL DEFAULT 0,
    -- for named people: their average face, so new or restored photos of them rejoin the name
    -- even when none of their photos are left
    face_memory BLOB,
    -- the face the user picked for the person's card (used while it is still theirs)
    cover_face INTEGER
);

CREATE TABLE IF NOT EXISTS faces (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    photo_id  INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
    -- box relative to the photo, 0..1
    x REAL NOT NULL, y REAL NOT NULL, w REAL NOT NULL, h REAL NOT NULL,
    score     REAL NOT NULL,
    embedding BLOB NOT NULL,
    thumb     BLOB NOT NULL,
    person_id INTEGER REFERENCES persons(id) ON DELETE SET NULL,
    -- set when the user places a face (Not them: a group of its own; Same as: another
    -- person); clustering never moves it
    rejected  INTEGER NOT NULL DEFAULT 0,
    -- 0 until a grouping pass has looked at the face (new faces are grouped incrementally)
    grouped   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS faces_photo ON faces(photo_id);
CREATE INDEX IF NOT EXISTS faces_person ON faces(person_id, photo_id);
-- each person's cover face is their highest-scoring one
CREATE INDEX IF NOT EXISTS faces_person_score ON faces(person_id, score);
";

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 10000;
         PRAGMA cache_size = -65536;
         PRAGMA mmap_size = 1073741824;",
    )?;
    init(&conn)?;
    Ok(conn)
}

/// Creates the tables, or brings an index made by an older version up to date.
pub(crate) fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    migrate(conn)
}

/// A fresh index in memory, for tests.
#[cfg(test)]
pub(crate) fn open_in_memory() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory database");
    conn.execute_batch("PRAGMA foreign_keys = ON;").expect("pragmas");
    init(&conn).expect("schema");
    conn
}

/// Columns added after the first version, with their definitions: `migrate` adds the
/// missing ones to an existing index. New columns go at the end of this list.
const ADDED_COLUMNS: &[(&str, &str, &str)] = &[
    ("photos", "faces_scanned", "INTEGER NOT NULL DEFAULT 1"),
    // Faces in an existing index were grouped by the full passes of older versions.
    ("faces", "grouped", "INTEGER NOT NULL DEFAULT 1"),
    ("persons", "face_memory", "BLOB"),
    ("persons", "cover_face", "INTEGER"),
    ("photos", "version", "INTEGER NOT NULL DEFAULT 0"),
    ("photos", "content_hash", "TEXT"),
];

/// Upgrades indexes created by older versions in place.
fn migrate(conn: &Connection) -> Result<()> {
    for (table, column, definition) in ADDED_COLUMNS {
        let exists: bool = conn.query_row(
            &format!("SELECT COUNT(*) > 0 FROM pragma_table_info('{table}') WHERE name = ?"),
            [column],
            |r| r.get(0),
        )?;
        if !exists {
            conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"))?;
        }
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS photos_hash ON photos(content_hash) WHERE content_hash IS NOT NULL",
    )?;
    Ok(())
}

/// Files that could not be indexed, and photos removed from the gallery (their files kept).
pub fn problem_counts(conn: &Connection) -> Result<(i64, i64)> {
    Ok(conn.query_row("SELECT (SELECT COUNT(*) FROM failures), (SELECT COUNT(*) FROM excluded)", [], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?)
}

/// A file that could not be indexed.
#[derive(serde::Serialize)]
pub struct Failure {
    pub path: String,
    pub error: String,
}

/// The first `limit` files that could not be indexed, by path.
pub fn failures(conn: &Connection, limit: i64) -> Result<Vec<Failure>> {
    Ok(conn
        .prepare_cached("SELECT path, error FROM failures ORDER BY path LIMIT ?")?
        .query_map([limit], |r| Ok(Failure { path: r.get(0)?, error: r.get(1)? }))?
        .collect::<Result<_, _>>()?)
}

/// Forgets the files that could not be indexed, so the next scan tries them again.
pub fn forget_failures(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM failures", [])?;
    Ok(())
}

/// Brings back the photos removed from the gallery: the next scan indexes them again.
pub fn clear_excluded(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM excluded", [])?;
    Ok(())
}

/// Saves the average face of named people (all, or `only` these), so their photos rejoin the
/// name when they come back even if, by then, none of their photos are left.
pub fn remember_named_people(conn: &Connection, only: Option<&[i64]>) -> Result<()> {
    let filter =
        only.map(|ids| format!(" AND p.id IN ({})", ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")));
    if only.is_some_and(|ids| ids.is_empty()) {
        return Ok(());
    }
    let mut sums: std::collections::HashMap<i64, Vec<f32>> = std::collections::HashMap::new();
    {
        let mut stmt = conn.prepare(&format!(
            "SELECT f.person_id, f.embedding FROM faces f JOIN persons p ON p.id = f.person_id WHERE p.name IS NOT NULL{}",
            filter.unwrap_or_default()
        ))?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let embedding = crate::faces::embedding_from_bytes(&r.get::<_, Vec<u8>>(1)?);
            let sum = sums.entry(r.get(0)?).or_insert_with(|| vec![0f32; crate::faces::EMBEDDING_DIM]);
            sum.iter_mut().zip(&embedding).for_each(|(a, b)| *a += b);
        }
    }
    let mut save = conn.prepare("UPDATE persons SET face_memory = ? WHERE id = ?")?;
    for (person, mut sum) in sums {
        let norm = sum.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
        sum.iter_mut().for_each(|x| *x /= norm);
        save.execute(params![crate::faces::embedding_to_bytes(&sum), person])?;
    }
    Ok(())
}

/// Removes one photo from the index (its file is gone), remembering its named people first.
pub fn forget_photo(conn: &mut Connection, id: i64) -> Result<()> {
    forget_photos(conn, &[id])
}

/// [`forget_photo`] for many photos at once: the people are remembered and the empty
/// groups cleared once, not for every photo.
pub fn forget_photos(conn: &mut Connection, ids: &[i64]) -> Result<()> {
    forget_photos_with_progress(conn, ids, |_| {})
}

/// [`forget_photos`], deleting in batches and telling `progress` how many are done.
pub fn forget_photos_with_progress(conn: &mut Connection, ids: &[i64], mut progress: impl FnMut(usize)) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut people = Vec::new();
    {
        let mut query =
            conn.prepare("SELECT DISTINCT person_id FROM faces WHERE photo_id = ? AND person_id IS NOT NULL")?;
        for id in ids {
            for person in query.query_map([id], |r| r.get::<_, i64>(0))? {
                people.push(person?);
            }
        }
    }
    people.sort_unstable();
    people.dedup();
    remember_named_people(conn, Some(&people))?;
    let mut done = 0;
    for chunk in ids.chunks(500) {
        let tx = conn.transaction()?;
        {
            let mut delete = tx.prepare_cached("DELETE FROM photos WHERE id = ?")?;
            for id in chunk {
                delete.execute([id])?;
            }
        }
        tx.commit()?;
        done += chunk.len();
        progress(done);
    }
    let tx = conn.transaction()?;
    tx.execute(
        "DELETE FROM persons WHERE name IS NULL AND id NOT IN (SELECT DISTINCT person_id FROM faces WHERE person_id IS NOT NULL)",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_index_is_brought_up_to_date() {
        // The tables as the first versions created them, with a photo and a face in them.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE photos (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL UNIQUE,
                 mtime INTEGER NOT NULL, size INTEGER NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL,
                 taken TEXT NOT NULL, date_from_exif INTEGER NOT NULL, lat REAL, lon REAL, place_id INTEGER);
             CREATE TABLE persons (id INTEGER PRIMARY KEY, name TEXT, hidden INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE faces (id INTEGER PRIMARY KEY AUTOINCREMENT, photo_id INTEGER NOT NULL,
                 x REAL NOT NULL, y REAL NOT NULL, w REAL NOT NULL, h REAL NOT NULL, score REAL NOT NULL,
                 embedding BLOB NOT NULL, thumb BLOB NOT NULL, person_id INTEGER, rejected INTEGER NOT NULL DEFAULT 0);
             INSERT INTO photos VALUES (1, '/p/a.jpg', 1, 2, 3, 4, '2020-01-01 00:00:00', 1, NULL, NULL, NULL);
             INSERT INTO faces (photo_id, x, y, w, h, score, embedding, thumb) VALUES (1, 0, 0, 1, 1, 0.9, x'', x'');",
        )
        .unwrap();
        init(&conn).unwrap();
        init(&conn).unwrap(); // and again: nothing to do the second time

        let columns = |table: &str| -> Vec<String> {
            conn.prepare(&format!("SELECT name FROM pragma_table_info('{table}')"))
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        for (table, column, _) in ADDED_COLUMNS {
            assert!(columns(table).iter().any(|c| c == column), "{table}.{column}");
        }
        // Existing rows get the defaults that keep them as they were.
        let (scanned, version): (bool, i64) =
            conn.query_row("SELECT faces_scanned, version FROM photos", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((scanned, version), (true, 0));
        let grouped: bool = conn.query_row("SELECT grouped FROM faces", [], |r| r.get(0)).unwrap();
        assert!(grouped, "faces of an old index were already grouped");
        let index: i64 =
            conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name = 'photos_hash'", [], |r| r.get(0)).unwrap();
        assert_eq!(index, 1);
    }

    #[test]
    fn forgetting_photos_remembers_named_people() {
        let mut conn = crate::testutil::people_index();
        forget_photos(&mut conn, &[1]).unwrap();
        let photos: i64 = conn.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0)).unwrap();
        let faces: i64 = conn.query_row("SELECT COUNT(*) FROM faces", [], |r| r.get(0)).unwrap();
        assert_eq!((photos, faces), (0, 0));
        assert!(!crate::testutil::person_exists(&conn, 2), "unnamed and empty: gone");
        let memory: Option<Vec<u8>> =
            conn.query_row("SELECT face_memory FROM persons WHERE id = 1", [], |r| r.get(0)).unwrap();
        let memory = crate::faces::embedding_from_bytes(&memory.expect("Ana's face is remembered"));
        let norm: f32 = memory.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "normalised");
    }
}
