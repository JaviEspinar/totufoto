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
    conn.execute_batch(SCHEMA)?;
    migrate(&conn)?;
    Ok(conn)
}

/// Upgrades indexes created by older versions in place.
fn migrate(conn: &Connection) -> Result<()> {
    let has_column: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_table_info('photos') WHERE name = 'faces_scanned'",
        [],
        |r| r.get(0),
    )?;
    if !has_column {
        conn.execute_batch("ALTER TABLE photos ADD COLUMN faces_scanned INTEGER NOT NULL DEFAULT 1")?;
    }
    let has_grouped: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_table_info('faces') WHERE name = 'grouped'",
        [],
        |r| r.get(0),
    )?;
    if !has_grouped {
        // Faces in an existing index were grouped by the full passes of older versions.
        conn.execute_batch("ALTER TABLE faces ADD COLUMN grouped INTEGER NOT NULL DEFAULT 1")?;
    }
    let has_memory: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_table_info('persons') WHERE name = 'face_memory'",
        [],
        |r| r.get(0),
    )?;
    if !has_memory {
        conn.execute_batch("ALTER TABLE persons ADD COLUMN face_memory BLOB")?;
    }
    let has_cover: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_table_info('persons') WHERE name = 'cover_face'",
        [],
        |r| r.get(0),
    )?;
    if !has_cover {
        conn.execute_batch("ALTER TABLE persons ADD COLUMN cover_face INTEGER")?;
    }
    let has_version: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_table_info('photos') WHERE name = 'version'",
        [],
        |r| r.get(0),
    )?;
    if !has_version {
        conn.execute_batch("ALTER TABLE photos ADD COLUMN version INTEGER NOT NULL DEFAULT 0")?;
    }
    let has_hash: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_table_info('photos') WHERE name = 'content_hash'",
        [],
        |r| r.get(0),
    )?;
    if !has_hash {
        conn.execute_batch("ALTER TABLE photos ADD COLUMN content_hash TEXT")?;
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS photos_hash ON photos(content_hash) WHERE content_hash IS NOT NULL")?;
    Ok(())
}

/// Moves a face out of its person into a new unnamed person of its own ("Not them").
/// Returns the new person's id, or `None` if the face doesn't exist.
pub fn move_face_to_new_person(conn: &mut Connection, face: i64) -> Result<Option<i64>> {
    let tx = conn.transaction()?;
    let Some(old) = tx
        .query_row("SELECT person_id FROM faces WHERE id = ?", [face], |r| r.get::<_, Option<i64>>(0))
        .optional()?
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
    let filter = only.map(|ids| format!(" AND p.id IN ({})", ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")));
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
    if ids.is_empty() {
        return Ok(());
    }
    let mut people = Vec::new();
    {
        let mut query = conn.prepare("SELECT DISTINCT person_id FROM faces WHERE photo_id = ? AND person_id IS NOT NULL")?;
        for id in ids {
            for person in query.query_map([id], |r| r.get::<_, i64>(0))? {
                people.push(person?);
            }
        }
    }
    people.sort_unstable();
    people.dedup();
    remember_named_people(conn, Some(&people))?;
    let tx = conn.transaction()?;
    {
        let mut delete = tx.prepare("DELETE FROM photos WHERE id = ?")?;
        for id in ids {
            delete.execute([id])?;
        }
    }
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
    let Some(old) = tx
        .query_row("SELECT person_id FROM faces WHERE id = ?", [face], |r| r.get::<_, Option<i64>>(0))
        .optional()?
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
    use super::unique_name;

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
