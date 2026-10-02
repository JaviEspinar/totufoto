//! People and faces: names, hiding, merging, "Not them", "Same as", card photos.

use anyhow::Result;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};

use super::{ApiError, ApiResult, Shared, db};

pub(super) async fn people(State(s): State<Shared>) -> ApiResult<Json<JsonValue>> {
    let rows = db(&s, |conn| {
        let rows: Vec<JsonValue> = conn
            .prepare_cached(
                "SELECT p.id, p.name, p.hidden, COUNT(DISTINCT f.photo_id) AS n,
                        COALESCE((SELECT id FROM faces WHERE id = p.cover_face AND person_id = p.id),
                                 (SELECT id FROM faces WHERE person_id = p.id ORDER BY score DESC LIMIT 1))
                 FROM persons p JOIN faces f ON f.person_id = p.id
                 GROUP BY p.id ORDER BY p.name IS NULL, n DESC, p.name",
            )?
            .query_map([], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "name": r.get::<_, Option<String>>(1)?,
                    "hidden": r.get::<_, bool>(2)?,
                    "count": r.get::<_, i64>(3)?,
                    "face": r.get::<_, i64>(4)?,
                }))
            })?
            .collect::<Result<_, _>>()?;
        Ok(rows)
    })
    .await?;
    Ok(Json(rows.into()))
}

#[derive(Deserialize)]
pub(super) struct PersonUpdate {
    #[serde(default, deserialize_with = "serde_with_null::deserialize")]
    name: Option<Option<String>>,
    hidden: Option<bool>,
}

/// Distinguishes a missing `name` (keep) from `"name": null` (forget the name).
mod serde_with_null {
    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
        Ok(Some(Option::<String>::deserialize(d)?))
    }
}

pub(super) async fn update_person(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<PersonUpdate>,
) -> ApiResult<Response> {
    let saved = db(&s, move |conn| {
        if let Some(hidden) = body.hidden {
            conn.execute("UPDATE persons SET hidden = ? WHERE id = ?", params![hidden, id])?;
        }
        // Names are kept unique; the response says which name was saved.
        body.name.map(|name| crate::db::rename_person(conn, id, name.as_deref())).transpose()
    })
    .await?;
    Ok(match saved {
        Some(name) => Json(json!({ "name": name })).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}

#[derive(Deserialize)]
pub(super) struct Merge {
    into: i64,
}

pub(super) async fn merge_person(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<Merge>,
) -> ApiResult<StatusCode> {
    if body.into == id {
        return Ok(StatusCode::NO_CONTENT);
    }
    db(&s, move |conn| {
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE persons SET name = COALESCE(name, (SELECT name FROM persons WHERE id = ?1)) WHERE id = ?2",
            params![id, body.into],
        )?;
        tx.execute("UPDATE faces SET person_id = ? WHERE person_id = ?", params![body.into, id])?;
        tx.execute("DELETE FROM persons WHERE id = ?", [id])?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// "Not them": moves the face into a new unnamed person, which can be renamed, hidden or
/// merged later. Clustering never moves it back.
pub(super) async fn reject_face(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    match db(&s, move |conn| crate::db::move_face_to_new_person(conn, id)).await? {
        Some(person) => Ok(Json(json!({ "person": person })).into_response()),
        None => Err(ApiError::not_found("face")),
    }
}

/// Shows this face on its person's card. It stays the card's face while it belongs to them.
pub(super) async fn cover_face(State(s): State<Shared>, Path(id): Path<i64>) -> ApiResult<Response> {
    let person: Option<i64> = db(&s, move |conn| {
        let person: Option<i64> =
            conn.query_row("SELECT person_id FROM faces WHERE id = ?", [id], |r| r.get(0)).optional()?.flatten();
        if let Some(person) = person {
            conn.execute("UPDATE persons SET cover_face = ? WHERE id = ?", [id, person])?;
        }
        Ok(person)
    })
    .await?;
    match person {
        Some(person) => Ok(Json(json!({ "person": person })).into_response()),
        None => Err(ApiError::not_found("face in a group")),
    }
}

#[derive(Deserialize)]
pub(super) struct AssignBody {
    person: i64,
}

/// "Same as" in the photo viewer: moves this one face to another person.
pub(super) async fn assign_face(
    State(s): State<Shared>,
    Path(id): Path<i64>,
    Json(body): Json<AssignBody>,
) -> ApiResult<StatusCode> {
    let done = db(&s, move |conn| crate::db::assign_face(conn, id, body.person)).await?;
    if done { Ok(StatusCode::NO_CONTENT) } else { Err(ApiError::not_found("face or person")) }
}
