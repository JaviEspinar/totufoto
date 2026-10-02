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

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

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

/// Moves a face out of its person into a new unnamed person of its own ("Not them").
/// Returns the new person's id, or `None` if the face doesn't exist.
pub fn move_face_to_new_person(conn: &mut Connection, face: i64) -> Result<Option<i64>> {
    let tx = conn.transaction()?;
    let Some(old) =
        tx.query_row("SELECT person_id FROM faces WHERE id = ?", [face], |r| r.get::<_, Option<i64>>(0)).optional()?
    else {
        return Ok(None);
    };
    tx.execute("INSERT INTO persons (name) VALUES (NULL)", [])?;
    let person = tx.last_insert_rowid();
    tx.execute("UPDATE faces SET person_id = ?, rejected = 1 WHERE id = ?", params![person, face])?;
    // The old group may be empty now.
    if let Some(old) = old {
        tx.execute(
            "DELETE FROM persons WHERE id = ?1 AND name IS NULL AND NOT EXISTS (SELECT 1 FROM faces WHERE person_id = ?1)",
            [old],
        )?;
    }
    tx.commit()?;
    Ok(Some(person))
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

/// Moves one face to an existing person ("Same as" in the photo viewer). The face stays
/// there: clustering never moves faces the user placed. Returns false if the face or the
/// person doesn't exist.
pub fn assign_face(conn: &mut Connection, face: i64, person: i64) -> Result<bool> {
    let tx = conn.transaction()?;
    let exists: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM persons WHERE id = ?)", [person], |r| r.get(0))?;
    let Some(old) =
        tx.query_row("SELECT person_id FROM faces WHERE id = ?", [face], |r| r.get::<_, Option<i64>>(0)).optional()?
    else {
        return Ok(false);
    };
    if !exists {
        return Ok(false);
    }
    tx.execute("UPDATE faces SET person_id = ?, rejected = 1 WHERE id = ?", params![person, face])?;
    if let Some(old) = old.filter(|&o| o != person) {
        tx.execute(
            "DELETE FROM persons WHERE id = ?1 AND name IS NULL AND NOT EXISTS (SELECT 1 FROM faces WHERE person_id = ?1)",
            [old],
        )?;
    }
    tx.commit()?;
    Ok(true)
}

/// Faces marked "Not them" before they got their own group (older versions left them
/// without a person) each get one now. Run once at startup.
pub fn give_moved_faces_a_person(conn: &mut Connection) -> Result<usize> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let faces: Vec<i64> = tx
        .prepare("SELECT id FROM faces WHERE rejected = 1 AND person_id IS NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for face in &faces {
        tx.execute("INSERT INTO persons (name) VALUES (NULL)", [])?;
        tx.execute("UPDATE faces SET person_id = ? WHERE id = ?", params![tx.last_insert_rowid(), face])?;
    }
    tx.commit()?;
    Ok(faces.len())
}

/// Names people so no two share a name (ignoring case): a taken name gets the first free
/// " (n)" suffix, as file managers do ("Ana", "Ana (1)", "Ana (2)"...). Returns the name
/// saved, `None` when the person was left unnamed.
pub fn rename_person(conn: &mut Connection, id: i64, name: Option<&str>) -> Result<Option<String>> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let name = name.map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    let name = match name {
        Some(wanted) => {
            // A named person with no photos left gives the name up to this one.
            let empty: Vec<(i64, String)> = tx
                .prepare(
                    "SELECT id, name FROM persons WHERE name IS NOT NULL AND id != ?
                     AND NOT EXISTS (SELECT 1 FROM faces WHERE person_id = persons.id)",
                )?
                .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?;
            for (other, other_name) in empty {
                if other_name.to_lowercase() == wanted.to_lowercase() {
                    tx.execute("DELETE FROM persons WHERE id = ?", [other])?;
                }
            }
            let taken: std::collections::HashSet<String> = tx
                .prepare("SELECT name FROM persons WHERE name IS NOT NULL AND id != ?")?
                .query_map([id], |r| r.get::<_, String>(0))?
                .map(|n| n.map(|n| n.to_lowercase()))
                .collect::<Result<_, _>>()?;
            Some(unique_name(&wanted, |candidate| taken.contains(&candidate.to_lowercase())))
        }
        None => None,
    };
    tx.execute("UPDATE persons SET name = ? WHERE id = ?", params![name, id])?;
    tx.commit()?;
    Ok(name)
}

/// `wanted` if free, else "<base> (n)" with the smallest free n, where base drops an
/// existing " (n)" suffix so "Ana (1)" becomes "Ana (2)" rather than "Ana (1) (1)".
fn unique_name(wanted: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(wanted) {
        return wanted.to_string();
    }
    let base = match wanted.strip_suffix(')').and_then(|w| w.rsplit_once(" (")) {
        Some((base, n)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => base,
        _ => wanted,
    };
    (1..).map(|n| format!("{base} ({n})")).find(|c| !taken(c)).expect("some number is free")
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
    fn unique_names() {
        let taken = |list: &'static [&'static str]| move |c: &str| list.iter().any(|t| t.eq_ignore_ascii_case(c));
        assert_eq!(unique_name("Ana", taken(&[])), "Ana");
        assert_eq!(unique_name("Ana", taken(&["ana"])), "Ana (1)");
        assert_eq!(unique_name("Ana", taken(&["Ana", "Ana (1)"])), "Ana (2)");
        assert_eq!(unique_name("Ana (1)", taken(&["Ana", "Ana (1)"])), "Ana (2)");
        assert_eq!(unique_name("Ana (x)", taken(&["Ana (x)"])), "Ana (x) (1)");
        assert_eq!(unique_name("Ana", taken(&["Ana", "Ana (2)"])), "Ana (1)");
    }
}
