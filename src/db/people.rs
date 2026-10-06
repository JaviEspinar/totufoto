//! People: naming them, and moving faces between them ("Not them", "Same as").

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

/// A person as the People tab and the sidebar show them.
#[derive(Serialize)]
pub struct PersonSummary {
    pub id: i64,
    pub name: Option<String>,
    pub hidden: bool,
    /// Photos they are in.
    pub count: i64,
    /// The face on their card: the one chosen for it while it is still theirs, else their
    /// clearest one.
    pub face: i64,
}

/// Everyone with at least one face: named people first, then by number of photos.
pub fn list_people(conn: &Connection) -> Result<Vec<PersonSummary>> {
    Ok(conn
        .prepare_cached(
            "SELECT p.id, p.name, p.hidden, COUNT(DISTINCT f.item_id) AS n,
                    COALESCE((SELECT id FROM faces WHERE id = p.cover_face AND person_id = p.id),
                             (SELECT id FROM faces WHERE person_id = p.id ORDER BY score DESC LIMIT 1))
             FROM persons p JOIN faces f ON f.person_id = p.id
             GROUP BY p.id ORDER BY p.name IS NULL, n DESC, p.name",
        )?
        .query_map([], |r| {
            Ok(PersonSummary { id: r.get(0)?, name: r.get(1)?, hidden: r.get(2)?, count: r.get(3)?, face: r.get(4)? })
        })?
        .collect::<Result<_, _>>()?)
}

/// One person as `list_people` gives them, if they have a face.
pub fn person(conn: &Connection, id: i64) -> Result<Option<PersonSummary>> {
    Ok(conn
        .prepare_cached(
            "SELECT p.id, p.name, p.hidden, COUNT(DISTINCT f.item_id),
                    COALESCE((SELECT id FROM faces WHERE id = p.cover_face AND person_id = p.id),
                             (SELECT id FROM faces WHERE person_id = p.id ORDER BY score DESC LIMIT 1))
             FROM persons p JOIN faces f ON f.person_id = p.id
             WHERE p.id = ? GROUP BY p.id",
        )?
        .query_row([id], |r| {
            Ok(PersonSummary { id: r.get(0)?, name: r.get(1)?, hidden: r.get(2)?, count: r.get(3)?, face: r.get(4)? })
        })
        .optional()?)
}

pub fn set_hidden(conn: &Connection, person: i64, hidden: bool) -> Result<()> {
    conn.execute("UPDATE persons SET hidden = ? WHERE id = ?", params![hidden, person])?;
    Ok(())
}

/// Merges `from` into `into` ("Same as" in People): its faces join `into`, which takes its
/// name if it has none, and `from` goes.
pub fn merge_persons(conn: &mut Connection, from: i64, into: i64) -> Result<()> {
    if from == into {
        return Ok(());
    }
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE persons SET name = COALESCE(name, (SELECT name FROM persons WHERE id = ?1)) WHERE id = ?2",
        params![from, into],
    )?;
    tx.execute("UPDATE faces SET person_id = ? WHERE person_id = ?", params![into, from])?;
    tx.execute("DELETE FROM persons WHERE id = ?", [from])?;
    tx.commit()?;
    Ok(())
}

/// Shows this face on its person's card. Returns the person, `None` if the face doesn't
/// exist or belongs to no one.
pub fn set_cover_face(conn: &Connection, face: i64) -> Result<Option<i64>> {
    let person: Option<i64> =
        conn.query_row("SELECT person_id FROM faces WHERE id = ?", [face], |r| r.get(0)).optional()?.flatten();
    if let Some(person) = person {
        conn.execute("UPDATE persons SET cover_face = ? WHERE id = ?", [face, person])?;
    }
    Ok(person)
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

    fn person_of(conn: &Connection, face: i64) -> Option<i64> {
        conn.query_row("SELECT person_id FROM faces WHERE id = ?", [face], |r| r.get(0)).unwrap()
    }

    #[test]
    fn merging_moves_the_faces_and_keeps_a_name() {
        let mut conn = crate::testutil::people_index();
        // The unnamed person into Ana: Ana keeps her name and gets the face.
        merge_persons(&mut conn, 2, 1).unwrap();
        assert_eq!(person_of(&conn, 3), Some(1));
        assert!(!crate::testutil::person_exists(&conn, 2));
        // Ben into a new unnamed person: it takes his name.
        conn.execute("INSERT INTO persons (id, name) VALUES (9, NULL)", []).unwrap();
        merge_persons(&mut conn, 3, 9).unwrap();
        let name: Option<String> = conn.query_row("SELECT name FROM persons WHERE id = 9", [], |r| r.get(0)).unwrap();
        assert_eq!(name.as_deref(), Some("Ben"));
        merge_persons(&mut conn, 1, 1).unwrap();
        assert!(crate::testutil::person_exists(&conn, 1), "merging into oneself does nothing");
    }

    #[test]
    fn the_card_face_is_chosen_while_it_belongs_to_them() {
        let mut conn = crate::testutil::people_index();
        let face_of =
            |conn: &Connection, id: i64| list_people(conn).unwrap().into_iter().find(|p| p.id == id).unwrap().face;
        conn.execute("UPDATE faces SET score = 0.5 WHERE id = 2", []).unwrap();
        assert_eq!(face_of(&conn, 1), 1, "the clearest face");
        assert_eq!(set_cover_face(&conn, 2).unwrap(), Some(1));
        assert_eq!(face_of(&conn, 1), 2, "the chosen one");
        // Once the face is someone else's, the card goes back to the clearest.
        assign_face(&mut conn, 2, 3).unwrap();
        assert_eq!(face_of(&conn, 1), 1);
        assert_eq!(set_cover_face(&conn, 99).unwrap(), None);
    }

    #[test]
    fn listing_people() {
        let conn = crate::testutil::people_index();
        set_hidden(&conn, 3, true).unwrap();
        let people = list_people(&conn).unwrap();
        // Named first; Old Ana has no faces, so she isn't listed.
        let names: Vec<_> = people.iter().map(|p| (p.id, p.name.as_deref(), p.hidden)).collect();
        assert_eq!(names, [(1, Some("Ana"), false), (3, Some("Ben"), true), (2, None, false)]);
    }

    #[test]
    fn names_are_unique_ignoring_case() {
        let mut conn = crate::testutil::people_index();
        assert_eq!(rename_person(&mut conn, 2, Some("  ben ")).unwrap().as_deref(), Some("ben (1)"));
        assert_eq!(rename_person(&mut conn, 2, Some("Carla")).unwrap().as_deref(), Some("Carla"));
        assert_eq!(rename_person(&mut conn, 2, Some("   ")).unwrap(), None, "blank clears the name");
        // "Old Ana" has no photos left, so it gives its name up instead of making "Old Ana (1)".
        assert_eq!(rename_person(&mut conn, 2, Some("old ana")).unwrap().as_deref(), Some("old ana"));
        assert!(!crate::testutil::person_exists(&conn, 4));
    }

    #[test]
    fn same_as_moves_one_face_and_tidies_up() {
        let mut conn = crate::testutil::people_index();
        // The unnamed person's only face goes to Ana: the empty unnamed group goes.
        assert!(assign_face(&mut conn, 3, 1).unwrap());
        assert_eq!(person_of(&conn, 3), Some(1));
        assert!(!crate::testutil::person_exists(&conn, 2));
        // Ben's only face goes to Ana: Ben, named, stays (his name may come back).
        assert!(assign_face(&mut conn, 4, 1).unwrap());
        assert!(crate::testutil::person_exists(&conn, 3));
        let placed: bool = conn.query_row("SELECT rejected FROM faces WHERE id = 4", [], |r| r.get(0)).unwrap();
        assert!(placed, "grouping must never move a face the user placed");
        assert!(!assign_face(&mut conn, 99, 1).unwrap(), "no such face");
        assert!(!assign_face(&mut conn, 1, 99).unwrap(), "no such person");
        assert_eq!(person_of(&conn, 1), Some(1));
    }

    #[test]
    fn not_them_gives_the_face_a_group_of_its_own() {
        let mut conn = crate::testutil::people_index();
        let new = move_face_to_new_person(&mut conn, 3).unwrap().unwrap();
        assert_eq!(person_of(&conn, 3), Some(new));
        assert!(!crate::testutil::person_exists(&conn, 2), "the unnamed group it left is empty now");
        let new = move_face_to_new_person(&mut conn, 4).unwrap().unwrap();
        assert_eq!(person_of(&conn, 4), Some(new));
        assert!(crate::testutil::person_exists(&conn, 3), "Ben is named, so he stays");
        assert_eq!(move_face_to_new_person(&mut conn, 99).unwrap(), None);
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
