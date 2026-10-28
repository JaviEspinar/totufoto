//! Groups face embeddings into people.
//!
//! Faces of persons the user has named are fixed. Faces the user placed ("Not them" gives
//! a face its own group, "Same as" moves it to another person) are never moved; in an
//! unnamed group they attract nothing, and once the group is named they count as a named
//! person's faces. Every other face is first matched
//! against the named persons, and the rest are clustered by cosine similarity
//! (greedy centroid clustering followed by refinement passes). Unnamed clusters keep
//! their previous person id when most of their faces had it, so ids stay stable.
//!
//! Regrouping every face is slow on large libraries (minutes for 100,000+ faces), so a
//! normal scan only places the new faces: see [`update_groups`].

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use rayon::prelude::*;
use rusqlite::{Connection, params};

use crate::faces::{EMBEDDING_DIM, dot, embedding_from_bytes};

const MIN_CLUSTER_SIZE: usize = 2;
const REFINE_PASSES: usize = 2;
/// Above this share of new faces, regrouping everything gives better groups than placing
/// the new ones (for example on the first index of a library).
const FULL_REGROUP_SHARE: f64 = 0.5;

/// Where grouping reports how far it is (shown in Settings).
pub struct Progress<'a> {
    pub done: &'a AtomicU64,
    pub total: &'a AtomicU64,
}

impl Progress<'_> {
    fn start(&self, total: usize) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total as u64, Ordering::Relaxed);
    }
    fn add(&self, n: u64) {
        self.done.fetch_add(n, Ordering::Relaxed);
    }
}

/// Brings people groups up to date after a scan: regroups everything when asked to (or when
/// most faces are new), places only the new faces otherwise, and does nothing else when no
/// faces were added.
pub fn update_groups(conn: &mut Connection, threshold: f32, full: bool, progress: &Progress) -> Result<()> {
    let (new, total): (i64, i64) =
        conn.query_row("SELECT COALESCE(SUM(grouped = 0), 0), COUNT(*) FROM faces WHERE rejected = 0", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    if full || (new > 0 && new as f64 > total as f64 * FULL_REGROUP_SHARE) {
        recluster(conn, threshold, progress)?;
    } else if new > 0 {
        group_new_faces(conn, threshold, progress)?;
    } else {
        delete_empty_people(conn)?;
        tracing::info!("no new faces to group");
    }
    crate::db::remember_named_people(conn, None)
}

/// The ids of the people the user has named.
fn named_people(conn: &Connection) -> Result<HashSet<i64>> {
    Ok(conn
        .prepare("SELECT id FROM persons WHERE name IS NOT NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?)
}

/// The face remembered for each named person, which still attracts their faces when none of
/// their photos are in the index.
fn face_memories(conn: &Connection) -> Result<Vec<(i64, Vec<f32>)>> {
    let rows: Vec<(i64, Vec<u8>)> = conn
        .prepare("SELECT id, face_memory FROM persons WHERE name IS NOT NULL AND face_memory IS NOT NULL")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    Ok(rows.into_iter().map(|(id, bytes)| (id, embedding_from_bytes(&bytes))).collect())
}

fn delete_empty_people(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM persons WHERE name IS NULL AND id NOT IN (SELECT DISTINCT person_id FROM faces WHERE person_id IS NOT NULL)",
        [],
    )?;
    Ok(())
}

fn new_person(conn: &Connection) -> Result<i64> {
    conn.execute("INSERT INTO persons (name) VALUES (NULL)", [])?;
    Ok(conn.last_insert_rowid())
}

/// Saves each face's person (`None`: in no group).
fn set_people(conn: &Connection, assignment: impl IntoIterator<Item = (i64, Option<i64>)>) -> Result<()> {
    let mut update = conn.prepare("UPDATE faces SET person_id = ? WHERE id = ?")?;
    for (face, person) in assignment {
        update.execute(params![person, face])?;
    }
    Ok(())
}

struct Face {
    id: i64,
    person: Option<i64>,
    embedding: Vec<f32>,
    /// The user placed this face ("Not them" or "Same as").
    moved_by_user: bool,
}

fn normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter_mut().for_each(|x| *x /= n);
}

fn centroids(groups: &[Vec<usize>], faces: &[Face]) -> Vec<Vec<f32>> {
    groups
        .par_iter()
        .map(|members| {
            let mut c = vec![0f32; EMBEDDING_DIM];
            for &i in members {
                c.iter_mut().zip(&faces[i].embedding).for_each(|(a, b)| *a += b);
            }
            normalize(&mut c);
            c
        })
        .collect()
}

fn best(embedding: &[f32], centroids: &[Vec<f32>]) -> Option<(usize, f32)> {
    centroids.iter().enumerate().map(|(i, c)| (i, dot(embedding, c))).max_by(|a, b| a.1.total_cmp(&b.1))
}

/// For each embedding, the closest of `centroids` when similar enough. One step of progress
/// each.
fn closest_each(
    embeddings: &[&[f32]],
    centroids: &[Vec<f32>],
    threshold: f32,
    progress: &Progress,
) -> Vec<Option<usize>> {
    embeddings
        .par_iter()
        .map(|e| {
            progress.add(1);
            best(e, centroids).filter(|m| m.1 >= threshold).map(|m| m.0)
        })
        .collect()
}

/// Regroups every face from scratch (named people and faces placed by the user are kept).
pub fn recluster(conn: &mut Connection, threshold: f32, progress: &Progress) -> Result<()> {
    let named = named_people(conn)?;
    let faces: Vec<Face> = conn
        .prepare("SELECT id, person_id, embedding, rejected FROM faces ORDER BY score DESC")?
        .query_map([], |r| {
            Ok(Face {
                id: r.get(0)?,
                person: r.get(1)?,
                embedding: embedding_from_bytes(&r.get::<_, Vec<u8>>(2)?),
                moved_by_user: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    let regrouped = regroup(&faces, &named, face_memories(conn)?, threshold, progress);
    let previous = previous_ids(&regrouped.groups, &faces, &named);

    let tx = conn.transaction()?;
    let mut assignment = regrouped.assignment;
    for (group, previous) in regrouped.groups.iter().zip(previous) {
        let person = match previous {
            Some(person) => person,
            None => new_person(&tx)?,
        };
        assignment.extend(group.iter().map(|&i| (faces[i].id, Some(person))));
    }
    set_people(&tx, assignment)?;
    tx.execute("UPDATE faces SET grouped = 1 WHERE grouped = 0", [])?;
    delete_empty_people(&tx)?;
    tx.commit()?;
    tracing::info!("regrouped {} faces into {} people", faces.len(), regrouped.groups.len() + named.len());
    Ok(())
}

/// What regrouping decided, before anything is saved.
struct Regrouped {
    /// Faces matched to a named person, or left in no group, by face id.
    assignment: HashMap<i64, Option<i64>>,
    /// The new unnamed groups, as indices into the faces, largest first.
    groups: Vec<Vec<usize>>,
}

/// Regrouping without the database: free faces are matched to the named people, and the rest
/// clustered. Faces of named people and faces the user placed are in no result: they stay.
fn regroup(
    faces: &[Face],
    named: &HashSet<i64>,
    memories: Vec<(i64, Vec<f32>)>,
    threshold: f32,
    progress: &Progress,
) -> Regrouped {
    let mut fixed: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut free: Vec<usize> = Vec::new();
    for (i, f) in faces.iter().enumerate() {
        match f.person {
            Some(p) if named.contains(&p) => fixed.entry(p).or_default().push(i),
            // Placed by the user: stays where it is, attracts nothing until its group is named.
            _ if f.moved_by_user => {}
            _ => free.push(i),
        }
    }

    // Work: one step per free face to match named people, then (for the faces left) one
    // for the greedy pass and one per refinement pass; the total is corrected once known.
    progress.start(free.len() * (2 + REFINE_PASSES));

    // 1. Attach free faces to named persons (also those with no photos left, by the face
    // remembered for them).
    let (mut ids, named_groups): (Vec<i64>, Vec<Vec<usize>>) = fixed.into_iter().unzip();
    let mut named_centroids = centroids(&named_groups, faces);
    let with_faces: HashSet<i64> = ids.iter().copied().collect();
    for (person, memory) in memories.into_iter().filter(|(person, _)| !with_faces.contains(person)) {
        ids.push(person);
        named_centroids.push(memory);
    }
    let embeddings: Vec<&[f32]> = free.iter().map(|&i| faces[i].embedding.as_slice()).collect();
    let mut assignment: HashMap<i64, Option<i64>> = HashMap::new();
    let mut rest = Vec::new();
    for (&i, m) in free.iter().zip(closest_each(&embeddings, &named_centroids, threshold, progress)) {
        match m {
            Some(k) => {
                assignment.insert(faces[i].id, Some(ids[k]));
            }
            None => rest.push(i),
        }
    }

    progress.total.store((free.len() + rest.len() * (1 + REFINE_PASSES)) as u64, Ordering::Relaxed);

    // 2. Greedy clustering of the remaining faces, then refinement passes.
    let mut groups = greedy(&rest, faces, threshold, progress);
    for _ in 0..REFINE_PASSES {
        groups = refine(&groups, &rest, faces, threshold, progress);
    }

    // 3. Groups too small to be a person leave their faces in no group.
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let (groups, alone): (Vec<_>, Vec<_>) = groups.into_iter().partition(|g| g.len() >= MIN_CLUSTER_SIZE);
    assignment.extend(alone.iter().flatten().map(|&i| (faces[i].id, None)));
    Regrouped { assignment, groups }
}

/// Each face, highest detection score first, joins the closest group when similar enough,
/// or starts a new one.
fn greedy(rest: &[usize], faces: &[Face], threshold: f32, progress: &Progress) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut sums: Vec<Vec<f32>> = Vec::new();
    let mut unit: Vec<Vec<f32>> = Vec::new();
    for &i in rest {
        progress.add(1);
        let e = &faces[i].embedding;
        let hit = if unit.len() > 256 {
            unit.par_iter().enumerate().map(|(k, c)| (k, dot(e, c))).max_by(|a, b| a.1.total_cmp(&b.1))
        } else {
            best(e, &unit)
        };
        match hit.filter(|h| h.1 >= threshold) {
            Some((k, _)) => {
                groups[k].push(i);
                sums[k].iter_mut().zip(e).for_each(|(a, b)| *a += b);
                unit[k].copy_from_slice(&sums[k]);
                normalize(&mut unit[k]);
            }
            None => {
                groups.push(vec![i]);
                sums.push(e.clone());
                unit.push(e.clone());
            }
        }
    }
    groups
}

/// Reassigns every face to its closest group's centroid; faces close to none start their own.
fn refine(
    groups: &[Vec<usize>],
    rest: &[usize],
    faces: &[Face],
    threshold: f32,
    progress: &Progress,
) -> Vec<Vec<usize>> {
    let cents = centroids(groups, faces);
    let embeddings: Vec<&[f32]> = rest.iter().map(|&i| faces[i].embedding.as_slice()).collect();
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
    for (&i, t) in rest.iter().zip(closest_each(&embeddings, &cents, threshold, progress)) {
        match t {
            Some(k) => next[k].push(i),
            None => next.push(vec![i]),
        }
    }
    next.retain(|g| !g.is_empty());
    next
}

/// The id each group keeps, so ids stay stable: the unnamed person most of its faces had,
/// unless a larger group took it first. `None`: a new person.
fn previous_ids(groups: &[Vec<usize>], faces: &[Face], named: &HashSet<i64>) -> Vec<Option<i64>> {
    let mut claimed: HashSet<i64> = HashSet::new();
    groups
        .iter()
        .map(|group| {
            let mut votes: HashMap<i64, usize> = HashMap::new();
            for &i in group {
                if let Some(p) = faces[i].person.filter(|p| !named.contains(p) && !claimed.contains(p)) {
                    *votes.entry(p).or_default() += 1;
                }
            }
            let person = votes.into_iter().max_by_key(|v| v.1).map(|v| v.0);
            claimed.extend(person);
            person
        })
        .collect()
}

/// What placing new faces needs: each person's centroid, the new faces, and the earlier
/// faces still in no group ("loose"), as (face id, embedding).
struct ToPlace {
    people: Vec<i64>,
    centroids: Vec<Vec<f32>>,
    new_faces: Vec<(i64, Vec<f32>)>,
    loose: Vec<(i64, Vec<f32>)>,
}

/// Where placing put the new faces, before anything is saved.
struct Placed {
    /// (face id, person) for the new faces that joined someone.
    joined: Vec<(i64, i64)>,
    /// The face ids of each new person.
    new_people: Vec<Vec<i64>>,
}

/// Places only the faces no grouping pass has seen yet. Each joins the closest existing
/// person when similar enough; the others are paired with each other and with earlier faces
/// that are still in no group, and every set of two or more becomes a new person. Much
/// cheaper than [`recluster`]: it compares the new faces only.
fn group_new_faces(conn: &mut Connection, threshold: f32, progress: &Progress) -> Result<()> {
    let to_place = faces_to_place(conn)?;
    let placed = place(&to_place, threshold, progress);

    let tx = conn.transaction()?;
    let mut assignment: Vec<(i64, Option<i64>)> = placed.joined.iter().map(|&(face, p)| (face, Some(p))).collect();
    for members in &placed.new_people {
        let person = new_person(&tx)?;
        assignment.extend(members.iter().map(|&face| (face, Some(person))));
    }
    set_people(&tx, assignment)?;
    {
        let mut seen = tx.prepare("UPDATE faces SET grouped = 1 WHERE id = ?")?;
        for (id, _) in &to_place.new_faces {
            seen.execute([id])?;
        }
    }
    delete_empty_people(&tx)?;
    tx.commit()?;
    let new_people = placed.new_people.len();
    tracing::info!(
        "grouped {} new faces: {} joined existing people, {} new {}",
        to_place.new_faces.len(),
        placed.joined.len(),
        new_people,
        if new_people == 1 { "person" } else { "people" }
    );
    Ok(())
}

/// Streams the faces: people's centroids are accumulated, only new and loose faces kept.
fn faces_to_place(conn: &Connection) -> Result<ToPlace> {
    let named = named_people(conn)?;
    let mut sums: HashMap<i64, Vec<f32>> = HashMap::new();
    let mut new_faces: Vec<(i64, Vec<f32>)> = Vec::new();
    let mut loose: Vec<(i64, Vec<f32>)> = Vec::new();
    {
        let mut stmt = conn.prepare("SELECT id, person_id, embedding, rejected, grouped FROM faces")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let (id, person, moved_by_user, grouped): (i64, Option<i64>, bool, bool) =
                (r.get(0)?, r.get(1)?, r.get(3)?, r.get(4)?);
            // Placed by the user in an unnamed group: attracts nothing.
            if moved_by_user && !person.is_some_and(|p| named.contains(&p)) {
                continue;
            }
            let embedding = embedding_from_bytes(&r.get::<_, Vec<u8>>(2)?);
            match person {
                Some(p) => sums
                    .entry(p)
                    .or_insert_with(|| vec![0f32; EMBEDDING_DIM])
                    .iter_mut()
                    .zip(&embedding)
                    .for_each(|(a, b)| *a += b),
                None if !grouped => new_faces.push((id, embedding)),
                None => loose.push((id, embedding)),
            }
        }
    }
    let with_faces: HashSet<i64> = sums.keys().copied().collect();
    let (mut people, mut centroids): (Vec<i64>, Vec<Vec<f32>>) = sums.into_iter().unzip();
    centroids.par_iter_mut().for_each(|c| normalize(c));
    // Named people with no photos left still attract their faces.
    for (person, memory) in face_memories(conn)?.into_iter().filter(|(person, _)| !with_faces.contains(person)) {
        people.push(person);
        centroids.push(memory);
    }
    Ok(ToPlace { people, centroids, new_faces, loose })
}

/// Placing without the database: new faces join the closest person, and the others pair up.
fn place(to_place: &ToPlace, threshold: f32, progress: &Progress) -> Placed {
    progress.start(to_place.new_faces.len() * 2);
    let embeddings: Vec<&[f32]> = to_place.new_faces.iter().map(|(_, e)| e.as_slice()).collect();
    let mut joined: Vec<(i64, i64)> = Vec::new();
    let mut rest: Vec<&(i64, Vec<f32>)> = Vec::new();
    for (face, m) in to_place.new_faces.iter().zip(closest_each(&embeddings, &to_place.centroids, threshold, progress))
    {
        match m {
            Some(k) => joined.push((face.0, to_place.people[k])),
            None => rest.push(face),
        }
    }
    progress.add(joined.len() as u64);
    let new_people = pair_up(&rest, &to_place.loose, threshold, progress);
    Placed { joined, new_people }
}

/// The new faces that joined no one pair up with each other and with loose faces (union-find
/// over both). Every set of two or more, as face ids, becomes a new person; loose faces like
/// no new face stay as they are.
fn pair_up(rest: &[&(i64, Vec<f32>)], loose: &[(i64, Vec<f32>)], threshold: f32, progress: &Progress) -> Vec<Vec<i64>> {
    // Pool indices: 0..rest.len() are the unmatched new faces, then the loose faces.
    let pool = |k: usize| if k < rest.len() { rest[k] } else { &loose[k - rest.len()] };
    let edges: Vec<Vec<usize>> = (0..rest.len())
        .into_par_iter()
        .map(|k| {
            progress.add(1);
            let e = &pool(k).1;
            (k + 1..rest.len() + loose.len()).filter(|&j| dot(e, &pool(j).1) >= threshold).collect()
        })
        .collect();
    let mut parent: Vec<usize> = (0..rest.len() + loose.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (k, targets) in edges.iter().enumerate() {
        for &j in targets {
            let (a, b) = (root(&mut parent, k), root(&mut parent, j));
            if a != b {
                parent[a] = b;
            }
        }
    }
    let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
    for k in (0..rest.len()).chain(edges.iter().flatten().copied()) {
        let r = root(&mut parent, k);
        components.entry(r).or_default().push(k);
    }
    components
        .into_values()
        .map(|mut members| {
            members.sort_unstable();
            members.dedup();
            members
        })
        .filter(|members| members.len() >= MIN_CLUSTER_SIZE)
        .map(|members| members.into_iter().map(|k| pool(k).0).collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::faces::embedding_to_bytes;

    /// A face embedding close to axis `k`; `j` makes each one slightly different.
    fn near(k: usize, j: usize) -> Vec<f32> {
        let mut v = vec![0f32; EMBEDDING_DIM];
        v[k] = 1.0;
        v[200 + j] = 0.15;
        normalize(&mut v);
        v
    }

    fn index() -> Connection {
        let conn = crate::db::open_in_memory();
        conn.execute(
            "INSERT INTO items (id, path, mtime, size, width, height, taken, date_from_exif)
             VALUES (1, '/p/1.jpg', 0, 1, 1, 1, '2020-01-01 00:00:00', 1)",
            [],
        )
        .unwrap();
        conn
    }

    fn add_face(conn: &Connection, id: i64, embedding: &[f32]) {
        conn.execute(
            "INSERT INTO faces (id, item_id, x, y, w, h, score, embedding, thumb) VALUES (?, 1, 0, 0, 1, 1, 0.9, ?, x'')",
            params![id, embedding_to_bytes(embedding)],
        )
        .unwrap();
    }

    fn person(conn: &Connection, face: i64) -> Option<i64> {
        conn.query_row("SELECT person_id FROM faces WHERE id = ?", [face], |r| r.get(0)).unwrap()
    }

    fn group(conn: &mut Connection, full: bool) {
        let (done, total) = (AtomicU64::new(0), AtomicU64::new(0));
        update_groups(conn, 0.42, full, &Progress { done: &done, total: &total }).unwrap();
    }

    fn quietly<R>(f: impl FnOnce(&Progress) -> R) -> R {
        let (done, total) = (AtomicU64::new(0), AtomicU64::new(0));
        f(&Progress { done: &done, total: &total })
    }

    fn face(id: i64, person: Option<i64>, embedding: Vec<f32>) -> Face {
        Face { id, person, embedding, moved_by_user: false }
    }

    #[test]
    fn regrouping_matches_named_people_and_clusters_the_rest() {
        let named = HashSet::from([10]);
        let faces = vec![
            face(1, Some(10), near(0, 0)),
            face(2, Some(10), near(0, 1)),
            face(3, None, near(0, 2)),
            face(4, Some(30), near(1, 3)),
            face(5, None, near(1, 4)),
            face(6, None, near(2, 5)),
            Face { moved_by_user: true, ..face(7, Some(20), near(1, 6)) },
            face(8, None, near(3, 7)),
        ];
        // Person 11 is named but has no photos left: its remembered face still attracts.
        let memories = vec![(11, near(3, 8))];
        let r = quietly(|p| regroup(&faces, &named, memories, 0.42, p));
        assert_eq!(r.assignment.get(&3), Some(&Some(10)), "joins the named person");
        assert_eq!(r.assignment.get(&8), Some(&Some(11)), "joins by the remembered face");
        assert_eq!(r.assignment.get(&6), Some(&None), "alone, in no group");
        assert!(!r.assignment.contains_key(&1) && !r.assignment.contains_key(&7), "fixed faces stay");
        let ids: Vec<Vec<i64>> = r.groups.iter().map(|g| g.iter().map(|&i| faces[i].id).collect()).collect();
        assert_eq!(ids, vec![vec![4, 5]]);
        assert_eq!(previous_ids(&r.groups, &faces, &named), vec![Some(30)], "keeps its id");
    }

    #[test]
    fn a_previous_id_goes_to_the_largest_group_only() {
        let faces = vec![face(1, Some(5), near(0, 0)), face(2, Some(5), near(0, 1)), face(3, Some(5), near(1, 2))];
        let groups = vec![vec![0, 1], vec![2]];
        assert_eq!(previous_ids(&groups, &faces, &HashSet::new()), vec![Some(5), None]);
    }

    #[test]
    fn unmatched_new_faces_pair_up_with_each_other_and_loose_faces() {
        let (a, b, c) = ((1, near(0, 0)), (2, near(0, 1)), (3, near(1, 2)));
        let loose = vec![(4, near(1, 3)), (5, near(2, 4))];
        let mut people = quietly(|p| pair_up(&[&a, &b, &c], &loose, 0.42, p));
        people.iter_mut().for_each(|m| m.sort_unstable());
        people.sort();
        assert_eq!(people, vec![vec![1, 2], vec![3, 4]], "face 5 is like no new face: it stays loose");
    }

    /// Faces 1-3 look alike, 4-5 look alike, 6 like nobody else.
    fn two_people_and_a_stranger() -> Connection {
        let mut conn = index();
        for (id, k, j) in [(1, 0, 0), (2, 0, 1), (3, 0, 2), (4, 1, 3), (5, 1, 4), (6, 2, 5)] {
            add_face(&conn, id, &near(k, j));
        }
        group(&mut conn, true);
        conn
    }

    #[test]
    fn similar_faces_become_one_person_and_a_single_face_none() {
        let conn = two_people_and_a_stranger();
        let a = person(&conn, 1).expect("faces 1-3 grouped");
        let b = person(&conn, 4).expect("faces 4-5 grouped");
        assert_ne!(a, b);
        assert_eq!((person(&conn, 2), person(&conn, 3)), (Some(a), Some(a)));
        assert_eq!(person(&conn, 5), Some(b));
        assert_eq!(person(&conn, 6), None, "one face alone is no one yet");
    }

    #[test]
    fn new_faces_join_their_person_or_pair_up() {
        let mut conn = two_people_and_a_stranger();
        let a = person(&conn, 1).unwrap();
        // Few new faces, so only they are placed: one like the first person, and one like
        // the stranger, which together make a new person.
        add_face(&conn, 7, &near(0, 6));
        add_face(&conn, 8, &near(2, 7));
        group(&mut conn, false);
        assert_eq!(person(&conn, 7), Some(a));
        let c = person(&conn, 8).expect("a new person");
        assert_eq!(person(&conn, 6), Some(c));
        assert_ne!(c, a);
    }

    #[test]
    fn regrouping_keeps_names_ids_and_faces_placed_by_hand() {
        let mut conn = two_people_and_a_stranger();
        let (a, b) = (person(&conn, 1).unwrap(), person(&conn, 4).unwrap());
        conn.execute("UPDATE persons SET name = 'Ana' WHERE id = ?", [a]).unwrap();
        // "Not them" on face 3: it gets a person of its own, which grouping must not undo.
        let alone = crate::db::move_face_to_new_person(&mut conn, 3).unwrap().unwrap();
        group(&mut conn, true);
        assert_eq!((person(&conn, 1), person(&conn, 2)), (Some(a), Some(a)), "Ana keeps her faces");
        assert_eq!(person(&conn, 4), Some(b), "an unnamed person keeps its id");
        assert_eq!(person(&conn, 3), Some(alone), "a face placed by hand stays");
    }
}
