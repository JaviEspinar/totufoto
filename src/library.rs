//! The library: which folders hold the items (photos and videos), and what may happen to them.
//!
//! The folders are the ones given on the command line plus the ones saved in Settings. A
//! item belongs to the library when its path is inside one of them; only such items may be
//! deleted from disk, rotated, or have duplicates deleted. The HTTP handlers map the
//! outcomes here to answers; the rules live here so they can be tested without a server.

use std::path::{Path, PathBuf};

use anyhow::Result;
use rusqlite::{Connection, params};

/// The command-line folders, then the saved ones. A folder inside another one is left out:
/// the outer one already includes it.
pub fn roots(conn: &Connection, fixed: &[PathBuf]) -> Result<Vec<PathBuf>> {
    roots_without(conn, fixed, None)
}

/// [`roots`] as they would be without the saved folder `left_out`. Not the same as leaving
/// it out of [`roots`]: a folder it contained (a command-line one, say) counts again.
fn roots_without(conn: &Connection, fixed: &[PathBuf], left_out: Option<&Path>) -> Result<Vec<PathBuf>> {
    let saved: Vec<PathBuf> = conn
        .prepare("SELECT path FROM folders ORDER BY path")?
        .query_map([], |r| r.get::<_, String>(0))?
        .map(|r| r.map(PathBuf::from))
        .filter(|p| p.as_ref().map_or(true, |p| Some(p.as_path()) != left_out))
        .collect::<Result<_, _>>()?;
    let all: Vec<PathBuf> = fixed.iter().cloned().chain(saved).collect();
    let mut roots: Vec<PathBuf> = Vec::new();
    for (i, root) in all.iter().enumerate() {
        let covered =
            all.iter().enumerate().any(|(j, other)| j != i && root.starts_with(other) && (root != other || j < i));
        if !covered {
            roots.push(root.clone());
        }
    }
    Ok(roots)
}

/// The folder of `roots` that `path` is in, if any (compared by path components, so
/// `/photos2/a.jpg` is not in `/photos`).
pub fn root_of<'a, R: AsRef<Path>>(roots: &'a [R], path: &Path) -> Option<&'a R> {
    roots.iter().find(|r| path.starts_with(r.as_ref()))
}

/// Whether a folder can be read now. A folder that is missing, or there but
/// completely empty, is taken for unplugged: on Linux an unmounted drive leaves its mount
/// point behind as an empty folder, and its items must not be taken for deleted.
pub fn reachable(folder: &Path) -> bool {
    std::fs::read_dir(folder).is_ok_and(|mut entries| entries.next().is_some())
}

/// The indexed items (id, path) whose files are inside `folder`.
pub fn items_under(conn: &Connection, folder: &Path) -> Result<Vec<(i64, String)>> {
    let (len, prefix) = prefix(folder);
    Ok(conn
        .prepare_cached("SELECT id, path FROM items WHERE substr(path, 1, ?1) = ?2")?
        .query_map(params![len, prefix], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?)
}

/// How many indexed items are inside `folder`.
pub fn count_under(conn: &Connection, folder: &Path) -> Result<i64> {
    let (len, prefix) = prefix(folder);
    Ok(conn
        .prepare_cached("SELECT COUNT(*) FROM items WHERE substr(path, 1, ?1) = ?2")?
        .query_row(params![len, prefix], |r| r.get(0))?)
}

/// "<folder><separator>" and its length in characters (as SQLite's substr counts), so
/// `/photos` doesn't also match `/photos2`.
fn prefix(folder: &Path) -> (i64, String) {
    let prefix = folder.join("").to_string_lossy().into_owned();
    (prefix.chars().count() as i64, prefix)
}

pub enum AddFolder {
    /// Saved (canonical path); saved folders inside it were replaced by it.
    Added(PathBuf),
    Missing,
    NotAFolder,
    /// Already in the gallery: this folder itself, or the folder it is inside.
    AlreadyIn(PathBuf),
}

/// Saves a folder from Settings.
pub fn add_folder(conn: &Connection, fixed: &[PathBuf], path: &Path) -> Result<AddFolder> {
    let Ok(path) = dunce::canonicalize(path) else { return Ok(AddFolder::Missing) };
    if !path.is_dir() {
        return Ok(AddFolder::NotAFolder);
    }
    if let Some(outer) = root_of(&roots(conn, fixed)?, &path) {
        return Ok(AddFolder::AlreadyIn(outer.clone()));
    }
    // Saved folders inside the new one aren't needed any more (their items stay).
    let saved: Vec<String> =
        conn.prepare("SELECT path FROM folders")?.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for inner in saved.iter().filter(|p| Path::new(p).starts_with(&path)) {
        conn.execute("DELETE FROM folders WHERE path = ?", [inner])?;
    }
    conn.execute("INSERT OR IGNORE INTO folders (path) VALUES (?)", [path.to_string_lossy()])?;
    Ok(AddFolder::Added(path))
}

pub struct RemovedFolder {
    pub removed: usize,
    /// Photos left in the gallery because another folder still includes them.
    pub kept: usize,
}

/// Removes a saved folder and its items from the gallery (the files stay), except items
/// another folder still includes. `total` is told how many items go, `progress` how many
/// are gone so far. The folder leaves the list only once its items are gone: if removing
/// them fails, it stays listed and can be removed again.
pub fn remove_folder(
    conn: &mut Connection,
    fixed: &[PathBuf],
    folder: &Path,
    total: impl FnOnce(usize),
    progress: impl FnMut(usize),
) -> Result<RemovedFolder> {
    let remaining = roots_without(conn, fixed, Some(folder))?;
    let (kept, gone): (Vec<_>, Vec<_>) =
        items_under(conn, folder)?.into_iter().partition(|(_, path)| root_of(&remaining, Path::new(path)).is_some());
    let gone: Vec<i64> = gone.into_iter().map(|(id, _)| id).collect();
    total(gone.len());
    crate::db::forget_items_with_progress(conn, &gone, progress)?;
    conn.execute("DELETE FROM folders WHERE path = ?", [folder.to_string_lossy()])?;
    Ok(RemovedFolder { removed: gone.len(), kept: kept.len() })
}

pub enum CheckItem {
    /// The file is there.
    Present(String),
    /// The file is gone but its folder is there, so the item was removed from the gallery
    /// (`None`: it wasn't in the gallery to begin with).
    Removed(Option<String>),
    /// Its folder can't be reached (an unplugged drive, say): nothing was removed.
    Unavailable { path: String, folder: PathBuf },
}

/// What happened to an item that can't be opened. An item whose file is gone while its
/// folder is there is removed from the gallery.
pub fn check_item(conn: &mut Connection, roots: &[PathBuf], id: i64) -> Result<CheckItem> {
    let Some(path) = crate::db::items::item_path(conn, id)? else { return Ok(CheckItem::Removed(None)) };
    let file = PathBuf::from(&path);
    if file.is_file() {
        return Ok(CheckItem::Present(path));
    }
    let root = root_of(roots, &file);
    let folder_there = match root {
        Some(r) => reachable(r),
        None => file.parent().is_some_and(|p| p.is_dir()),
    };
    if !folder_there {
        let folder = root.cloned().unwrap_or_else(|| file.parent().map(PathBuf::from).unwrap_or_default());
        return Ok(CheckItem::Unavailable { path, folder });
    }
    crate::db::forget_item(conn, id)?;
    tracing::info!("{path} is gone: removed from the gallery");
    Ok(CheckItem::Removed(Some(path)))
}

pub enum RemoveFrom {
    /// Out of the gallery; the file stays and scans leave it out from then on.
    Gallery,
    /// The file goes to the bin; deleted for good when `permanently`.
    Disk { permanently: bool },
}

pub enum RemoveItem {
    Removed(String),
    Binned(String),
    Deleted(String),
    /// The file's drive has no bin: nothing was done, and only `permanently` would delete it.
    NoBin {
        path: String,
        error: String,
    },
    NotFound,
    /// Not inside the folders, so it isn't deleted.
    Outside,
}

/// Removes an item from the gallery or from disk. Only files inside the folders are
/// deleted.
pub fn remove_item(conn: &mut Connection, roots: &[PathBuf], id: i64, from: RemoveFrom) -> Result<RemoveItem> {
    let Some(path) = crate::db::items::item_path(conn, id)? else { return Ok(RemoveItem::NotFound) };
    let permanently = match from {
        RemoveFrom::Gallery => {
            conn.execute("INSERT OR IGNORE INTO excluded (path) VALUES (?)", [&path])?;
            crate::db::forget_item(conn, id)?;
            tracing::info!("{path}: removed from the gallery (file kept)");
            return Ok(RemoveItem::Removed(path));
        }
        RemoveFrom::Disk { permanently } => permanently,
    };
    let file = PathBuf::from(&path);
    if root_of(roots, &file).is_none() {
        return Ok(RemoveItem::Outside);
    }
    if file.exists() {
        if permanently {
            std::fs::remove_file(&file)?;
            tracing::info!("{path}: deleted permanently");
        } else if let Err(e) = trash::delete(&file) {
            tracing::warn!("{path}: can't move to the bin: {e}");
            return Ok(RemoveItem::NoBin { path, error: e.to_string() });
        } else {
            tracing::info!("{path}: moved to the bin");
        }
    }
    crate::db::forget_item(conn, id)?;
    Ok(if permanently { RemoveItem::Deleted(path) } else { RemoveItem::Binned(path) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{Library, Photo};

    #[test]
    fn root_of_compares_whole_folder_names() {
        let roots = [PathBuf::from("/photos"), PathBuf::from("/more/photos")];
        assert_eq!(root_of(&roots, Path::new("/photos/a.jpg")), Some(&roots[0]));
        assert_eq!(root_of(&roots, Path::new("/more/photos/x/b.jpg")), Some(&roots[1]));
        assert_eq!(root_of(&roots, Path::new("/photos2/a.jpg")), None);
        assert_eq!(root_of(&roots, Path::new("/more/a.jpg")), None);
    }

    #[test]
    fn items_under_a_folder_not_its_namesakes() {
        use crate::testutil::native;
        // Paths as the scan stores them: absolute, with this system's separator.
        let base = std::env::temp_dir();
        let conn = crate::db::open_in_memory();
        for (id, rel) in [(1, "photos/a.jpg"), (2, "photos/sub/b.jpg"), (3, "photos2/c.jpg"), (4, "x/d.jpg")] {
            conn.execute(
                "INSERT INTO items (id, path, mtime, size, width, height, taken, date_from_exif) VALUES (?, ?, 0, 1, 1, 1, '', 0)",
                params![id, base.join(native(rel)).to_string_lossy()],
            )
            .unwrap();
        }
        let ids: Vec<i64> = items_under(&conn, &base.join("photos")).unwrap().into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, [1, 2]);
        assert_eq!(count_under(&conn, &base.join("photos")).unwrap(), 2);
        assert_eq!(count_under(&conn, &base.join(native("photos/sub"))).unwrap(), 1);
    }

    #[test]
    fn adding_folders() {
        let lib = Library::new();
        let conn = lib.conn();
        let fixed = std::slice::from_ref(&lib.cfg.fixed_roots[0]);
        let other = lib.dir.file("other/.keep").parent().unwrap().to_path_buf();
        std::fs::create_dir_all(other.join("inner")).unwrap();
        std::fs::write(lib.dir.path().join("file.txt"), "").unwrap();

        assert!(matches!(add_folder(&conn, fixed, &lib.dir.path().join("nope")).unwrap(), AddFolder::Missing));
        assert!(matches!(add_folder(&conn, fixed, &lib.dir.path().join("file.txt")).unwrap(), AddFolder::NotAFolder));
        assert!(matches!(add_folder(&conn, fixed, &lib.root()).unwrap(), AddFolder::AlreadyIn(r) if r == lib.root()));
        assert!(matches!(add_folder(&conn, fixed, &other.join("inner")).unwrap(), AddFolder::Added(_)));
        assert!(matches!(add_folder(&conn, fixed, &other.join("inner")).unwrap(), AddFolder::AlreadyIn(_)));
        // The outer folder replaces the saved one inside it.
        assert!(matches!(add_folder(&conn, fixed, &other).unwrap(), AddFolder::Added(_)));
        assert_eq!(roots(&conn, fixed).unwrap(), [lib.root(), other]);
    }

    #[test]
    fn removing_a_folder_keeps_items_another_one_includes() {
        // A saved folder that contains the command-line one: removing it must not drop the
        // command-line folder's items.
        let lib = Library::new();
        lib.add("a.jpg", Photo::default());
        let outer = lib.dir.path().to_path_buf();
        Photo { color: [1, 2, 3], ..Photo::default() }.write(&lib.dir.file("b.jpg"));
        let mut conn = lib.conn();
        let fixed = lib.cfg.fixed_roots.clone();
        assert!(matches!(add_folder(&conn, &fixed, &outer).unwrap(), AddFolder::Added(_)));
        lib.scan();
        let a = lib.id("a.jpg");

        let mut told = 0;
        let mut progress = Vec::new();
        let r = remove_folder(&mut conn, &fixed, &outer, |n| told = n, |n| progress.push(n)).unwrap();
        assert_eq!((r.removed, r.kept, told), (1, 1, 1));
        assert_eq!(progress.last(), Some(&1));
        assert_eq!(lib.id("a.jpg"), a, "kept as it was, not indexed again");
        assert_eq!(roots(&conn, &fixed).unwrap(), fixed);
    }

    #[test]
    fn checking_an_item_that_cannot_be_opened() {
        let lib = Library::new();
        lib.add("a.jpg", Photo::default());
        lib.add("sub/b.jpg", Photo { color: [1, 2, 3], ..Photo::default() });
        lib.scan();
        let mut conn = lib.conn();
        let roots = lib.cfg.fixed_roots.clone();
        let (a, b) = (lib.id("a.jpg"), lib.id("sub/b.jpg"));

        assert!(matches!(check_item(&mut conn, &roots, a).unwrap(), CheckItem::Present(_)));
        std::fs::remove_file(lib.root().join("a.jpg")).unwrap();
        assert!(matches!(check_item(&mut conn, &roots, a).unwrap(), CheckItem::Removed(Some(_))));
        assert!(matches!(check_item(&mut conn, &roots, a).unwrap(), CheckItem::Removed(None)));
        // The whole library folder is gone (an unplugged drive): nothing is removed.
        std::fs::remove_dir_all(lib.root()).unwrap();
        assert!(matches!(check_item(&mut conn, &roots, b).unwrap(), CheckItem::Unavailable { .. }));
        assert_eq!(lib.id("sub/b.jpg"), b);
    }

    #[test]
    fn removing_items() {
        let lib = Library::new();
        lib.add("a.jpg", Photo::default());
        lib.add("b.jpg", Photo { color: [1, 2, 3], ..Photo::default() });
        lib.scan();
        let mut conn = lib.conn();
        let roots = lib.cfg.fixed_roots.clone();
        let (a, b) = (lib.id("a.jpg"), lib.id("b.jpg"));

        assert!(matches!(remove_item(&mut conn, &roots, a, RemoveFrom::Gallery).unwrap(), RemoveItem::Removed(_)));
        assert!(lib.root().join("a.jpg").exists());
        assert!(matches!(remove_item(&mut conn, &roots, a, RemoveFrom::Gallery).unwrap(), RemoveItem::NotFound));
        let elsewhere = [lib.dir.path().join("elsewhere")];
        let disk = || RemoveFrom::Disk { permanently: true };
        assert!(matches!(remove_item(&mut conn, &elsewhere, b, disk()).unwrap(), RemoveItem::Outside));
        assert!(lib.root().join("b.jpg").exists());
        assert!(matches!(remove_item(&mut conn, &roots, b, disk()).unwrap(), RemoveItem::Deleted(_)));
        assert!(!lib.root().join("b.jpg").exists());
    }
}
