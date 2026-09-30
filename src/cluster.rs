//! Groups face embeddings into people.
//!
//! Faces of persons the user has named are fixed. Faces the user placed ("Not them" gives
//! a face its own group, "Same as" moves it to another person) are never moved; in an
//! unnamed group they attract nothing, and once the group is named they count as a named
//! person's faces. Every other face is first matched
//! against the named persons, and the rest are clustered by cosine similarity
//! (greedy centroid clustering followed by refinement passes). Unnamed clusters keep
//! their previous person id when most of their faces had it, so ids stay stable.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rayon::prelude::*;
use rusqlite::{Connection, params};

use crate::faces::{EMBEDDING_DIM, dot, embedding_from_bytes};

const MIN_CLUSTER_SIZE: usize = 2;
const REFINE_PASSES: usize = 2;

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

pub fn recluster(conn: &mut Connection, threshold: f32) -> Result<()> {
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

    // 1. Attach free faces to named persons.
    let named_ids: Vec<i64> = fixed.keys().copied().collect();
    let named_groups: Vec<Vec<usize>> = named_ids.iter().map(|id| fixed[id].clone()).collect();
    let named_centroids = centroids(&named_groups, &faces);
    let matches: Vec<Option<usize>> = free
        .par_iter()
        .map(|&i| best(&faces[i].embedding, &named_centroids).filter(|m| m.1 >= threshold).map(|m| m.0))
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

    // 2. Greedy clustering of the remaining faces (highest detection score first).
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut sums: Vec<Vec<f32>> = Vec::new();
    let mut unit: Vec<Vec<f32>> = Vec::new();
    for &i in &rest {
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
        let targets: Vec<Option<usize>> =
            rest.par_iter().map(|&i| best(&faces[i].embedding, &cents).filter(|h| h.1 >= threshold).map(|h| h.0)).collect();
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
    tx.execute(
        "DELETE FROM persons WHERE name IS NULL AND id NOT IN (SELECT DISTINCT person_id FROM faces WHERE person_id IS NOT NULL)",
        [],
    )?;
    tx.commit()?;
    tracing::info!("clustered {} faces into {} people", faces.len(), claimed.len() + named.len());
    Ok(())
}
