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
    let (new, total): (i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(grouped = 0), 0), COUNT(*) FROM faces WHERE rejected = 0",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if full || (new > 0 && new as f64 > total as f64 * FULL_REGROUP_SHARE) {
        recluster(conn, threshold, progress)
    } else if new > 0 {
        group_new_faces(conn, threshold, progress)
    } else {
        delete_empty_people(conn)?;
        tracing::info!("no new faces to group");
        Ok(())
    }
}

fn delete_empty_people(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM persons WHERE name IS NULL AND id NOT IN (SELECT DISTINCT person_id FROM faces WHERE person_id IS NOT NULL)",
        [],
    )?;
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
    centroids
        .iter()
        .enumerate()
        .map(|(i, c)| (i, dot(embedding, c)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// Regroups every face from scratch (named people and faces placed by the user are kept).
pub fn recluster(conn: &mut Connection, threshold: f32, progress: &Progress) -> Result<()> {
    let named: HashSet<i64> = conn
        .prepare("SELECT id FROM persons WHERE name IS NOT NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;

    let mut fixed: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut free: Vec<usize> = Vec::new();
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

    // 1. Attach free faces to named persons.
    let named_ids: Vec<i64> = fixed.keys().copied().collect();
    let named_groups: Vec<Vec<usize>> = named_ids.iter().map(|id| fixed[id].clone()).collect();
    let named_centroids = centroids(&named_groups, &faces);
    let matches: Vec<Option<usize>> = free
        .par_iter()
        .map(|&i| {
            progress.add(1);
            best(&faces[i].embedding, &named_centroids).filter(|m| m.1 >= threshold).map(|m| m.0)
        })
        .collect();
    let mut assignment: HashMap<i64, Option<i64>> = HashMap::new();
    let mut rest = Vec::new();
    for (&i, m) in free.iter().zip(matches) {
        match m {
            Some(k) => {
                assignment.insert(faces[i].id, Some(named_ids[k]));
            }
            None => rest.push(i),
        }
    }

    progress.total.store((free.len() + rest.len() * (1 + REFINE_PASSES)) as u64, Ordering::Relaxed);

    // 2. Greedy clustering of the remaining faces (highest detection score first).
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut sums: Vec<Vec<f32>> = Vec::new();
    let mut unit: Vec<Vec<f32>> = Vec::new();
    for &i in &rest {
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

    // 3. Refinement: reassign every face to its closest centroid.
    for _ in 0..REFINE_PASSES {
        let cents = centroids(&groups, &faces);
        let targets: Vec<Option<usize>> = rest
            .par_iter()
            .map(|&i| {
                progress.add(1);
                best(&faces[i].embedding, &cents).filter(|h| h.1 >= threshold).map(|h| h.0)
            })
            .collect();
        let mut next: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
        for (&i, t) in rest.iter().zip(targets) {
            match t {
                Some(k) => next[k].push(i),
                None => next.push(vec![i]),
            }
        }
        next.retain(|g| !g.is_empty());
        groups = next;
    }

    // 4. Persist: reuse the previous unnamed person id where possible.
    let tx = conn.transaction()?;
    let mut claimed: HashSet<i64> = HashSet::new();
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    for group in &groups {
        if group.len() < MIN_CLUSTER_SIZE {
            for &i in group {
                assignment.insert(faces[i].id, None);
            }
            continue;
        }
        let mut votes: HashMap<i64, usize> = HashMap::new();
        for &i in group {
            if let Some(p) = faces[i].person.filter(|p| !named.contains(p) && !claimed.contains(p)) {
                *votes.entry(p).or_default() += 1;
            }
        }
        let person = match votes.into_iter().max_by_key(|v| v.1) {
            Some((p, _)) => p,
            None => {
                tx.execute("INSERT INTO persons (name) VALUES (NULL)", [])?;
                tx.last_insert_rowid()
            }
        };
        claimed.insert(person);
        for &i in group {
            assignment.insert(faces[i].id, Some(person));
        }
    }
    {
        let mut update = tx.prepare("UPDATE faces SET person_id = ? WHERE id = ?")?;
        for (face, person) in &assignment {
            update.execute(params![person, face])?;
        }
    }
    tx.execute("UPDATE faces SET grouped = 1 WHERE grouped = 0", [])?;
    delete_empty_people(&tx)?;
    tx.commit()?;
    tracing::info!("regrouped {} faces into {} people", faces.len(), claimed.len() + named.len());
    Ok(())
}

/// Places only the faces no grouping pass has seen yet. Each joins the closest existing
/// person when similar enough; the others are paired with each other and with earlier faces
/// that are still in no group, and every set of two or more becomes a new person. Much
/// cheaper than [`recluster`]: it compares the new faces only.
fn group_new_faces(conn: &mut Connection, threshold: f32, progress: &Progress) -> Result<()> {
    let named: HashSet<i64> = conn
        .prepare("SELECT id FROM persons WHERE name IS NOT NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    // Stream the faces: people's centroids are accumulated, only new and loose faces kept.
    let mut sums: HashMap<i64, Vec<f32>> = HashMap::new();
    let mut new_faces: Vec<(i64, Vec<f32>)> = Vec::new();
    let mut loose: Vec<(i64, Vec<f32>)> = Vec::new();
    {
        let mut stmt = conn.prepare("SELECT id, person_id, embedding, rejected, grouped FROM faces")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let (id, person, moved_by_user, grouped): (i64, Option<i64>, bool, bool) = (r.get(0)?, r.get(1)?, r.get(3)?, r.get(4)?);
            // Placed by the user in an unnamed group: attracts nothing.
            if moved_by_user && !person.is_some_and(|p| named.contains(&p)) {
                continue;
            }
            let embedding = embedding_from_bytes(&r.get::<_, Vec<u8>>(2)?);
            match person {
                Some(p) => sums.entry(p).or_insert_with(|| vec![0f32; EMBEDDING_DIM]).iter_mut().zip(&embedding).for_each(|(a, b)| *a += b),
                None if !grouped => new_faces.push((id, embedding)),
                None => loose.push((id, embedding)),
            }
        }
    }
    let (ids, mut units): (Vec<i64>, Vec<Vec<f32>>) = sums.into_iter().unzip();
    units.par_iter_mut().for_each(|u| normalize(u));
    progress.start(new_faces.len() * 2);

    // 1. New faces join the closest person.
    let matches: Vec<Option<i64>> = new_faces
        .par_iter()
        .map(|(_, e)| {
            progress.add(1);
            best(e, &units).filter(|m| m.1 >= threshold).map(|m| ids[m.0])
        })
        .collect();
    let mut assignment: Vec<(i64, i64)> = Vec::new();
    let mut rest: Vec<usize> = Vec::new();
    for (i, m) in matches.into_iter().enumerate() {
        match m {
            Some(p) => assignment.push((new_faces[i].0, p)),
            None => rest.push(i),
        }
    }

    // 2. The others pair up with each other and with loose faces (union-find over both).
    // Pool indices: 0..rest.len() are the unmatched new faces, then the loose faces.
    let pool = |k: usize| if k < rest.len() { &new_faces[rest[k]] } else { &loose[k - rest.len()] };
    let edges: Vec<Vec<usize>> = (0..rest.len())
        .into_par_iter()
        .map(|k| {
            progress.add(1);
            let e = &pool(k).1;
            (k + 1..rest.len() + loose.len()).filter(|&j| dot(e, &pool(j).1) >= threshold).collect()
        })
        .collect();
    progress.add((new_faces.len() - rest.len()) as u64);
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
        let members = components.entry(r).or_default();
        if !members.contains(&k) {
            members.push(k);
        }
    }

    let tx = conn.transaction()?;
    let mut new_people = 0;
    for members in components.values().filter(|m| m.len() >= MIN_CLUSTER_SIZE) {
        tx.execute("INSERT INTO persons (name) VALUES (NULL)", [])?;
        let person = tx.last_insert_rowid();
        new_people += 1;
        assignment.extend(members.iter().map(|&k| (pool(k).0, person)));
    }
    {
        let mut update = tx.prepare("UPDATE faces SET person_id = ? WHERE id = ?")?;
        for (face, person) in &assignment {
            update.execute(params![person, face])?;
        }
        let mut seen = tx.prepare("UPDATE faces SET grouped = 1 WHERE id = ?")?;
        for (id, _) in &new_faces {
            seen.execute([id])?;
        }
    }
    delete_empty_people(&tx)?;
    tx.commit()?;
    tracing::info!(
        "grouped {} new faces: {} joined existing people, {} new {}",
        new_faces.len(),
        assignment.len() - components.values().filter(|m| m.len() >= MIN_CLUSTER_SIZE).map(|m| m.len()).sum::<usize>(),
        new_people,
        if new_people == 1 { "person" } else { "people" }
    );
    Ok(())
}
