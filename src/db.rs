use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

const SCHEMA: &str = "
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
    faces_scanned INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS photos_taken ON photos(taken);
CREATE INDEX IF NOT EXISTS photos_md ON photos(substr(taken, 6, 5));
CREATE INDEX IF NOT EXISTS photos_place ON photos(place_id);

CREATE TABLE IF NOT EXISTS thumbs (
    photo_id INTEGER PRIMARY KEY REFERENCES photos(id) ON DELETE CASCADE,
    data     BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS persons (
    id     INTEGER PRIMARY KEY,
    -- NULL until the user names the person; named persons keep their faces across re-clustering
    name   TEXT,
    hidden INTEGER NOT NULL DEFAULT 0
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
    -- set when the user moves a face out of a person (Not them): the face then has its own
    -- group, and clustering never moves it
    rejected  INTEGER NOT NULL DEFAULT 0
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
