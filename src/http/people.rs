//! People and faces: names, hiding, merging, "Not them", "Same as", card photos.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::db::PersonSummary;

use super::{ApiError, ApiResult, Shared, db};

pub(super) async fn people(State(s): State<Shared>) -> ApiResult<Json<Vec<PersonSummary>>> {
    Ok(Json(db(&s, |conn| crate::db::list_people(conn)).await?))
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
            crate::db::set_hidden(conn, id, hidden)?;
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
    db(&s, move |conn| crate::db::merge_persons(conn, id, body.into)).await?;
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
    match db(&s, move |conn| crate::db::set_cover_face(conn, id)).await? {
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
