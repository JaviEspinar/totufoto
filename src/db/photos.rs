//! Photo queries for the page: lists, group cards, details, places and pictures. Each
//! returns what the page reads, ready to be sent as JSON.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde::Serialize;

use super::filters::{PhotoQuery, date_range_filter, people_filter, photo_filters};

/// A photo or video in a list: `[id, width, height, taken, place, version, duration]`, where
/// `duration` is null for photos and a video's length in seconds (0 when unknown). An array
/// rather than an object, since lists can have hundreds of thousands of photos.
#[derive(Serialize)]
pub struct PhotoRow(pub i64, pub i64, pub i64, pub String, pub Option<i64>, pub i64, pub Option<f64>);

#[derive(Serialize)]
pub struct PhotoList {
    pub photos: Vec<PhotoRow>,
    /// With `upcoming`: the days (`MM-DD`) it covers, from today.
    pub days: Vec<String>,
}

/// The photos matching the query, newest first unless `sort` is "asc".
pub fn list_photos(conn: &Connection, q: &PhotoQuery) -> Result<PhotoList> {
    let (filters, args, days) = photo_filters(q);
    let order = if newest_first(q) { "taken DESC, id DESC" } else { "taken ASC, id ASC" };
    let sql = format!(
        "SELECT id, width, height, taken, place_id, version, duration FROM photos WHERE 1 = 1{filters} ORDER BY {order}"
    );
    let photos = conn
        .prepare_cached(&sql)?
        .query_map(params_from_iter(args), |r| {
            Ok(PhotoRow(r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
        })?
        .collect::<Result<_, _>>()?;
    Ok(PhotoList { photos, days })
}

fn newest_first(q: &PhotoQuery) -> bool {
    q.sort.as_deref() != Some("asc")
}

#[derive(Clone, Copy, PartialEq)]
pub enum GroupBy {
    Year,
    Month,
    Day,
    Place,
}

impl GroupBy {
    pub fn parse(by: Option<&str>) -> Option<Self> {
        match by? {
            "year" => Some(Self::Year),
            "month" => Some(Self::Month),
            "day" => Some(Self::Day),
            "place" => Some(Self::Place),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Year => "substr(taken, 1, 4)",
            Self::Month => "substr(taken, 1, 7)",
            Self::Day => "substr(taken, 1, 10)",
            Self::Place => "COALESCE(place_id, 0)",
        }
    }
}

/// A date ("2024", "2024-10", "2024-10-05"), or a place id (0: no location).
#[derive(Serialize)]
#[serde(untagged)]
pub enum GroupKey {
    Date(String),
    Place(i64),
}

#[derive(Serialize)]
pub struct Group {
    pub key: GroupKey,
    pub count: i64,
    /// The cover photo: the newest, or the oldest when sorting oldest first.
    pub cover: i64,
    /// The cover's version, for its thumbnail's URL.
    #[serde(rename = "v")]
    pub version: i64,
    /// How many of `count` are videos.
    pub videos: i64,
}

#[derive(Serialize)]
pub struct Groups {
    pub groups: Vec<Group>,
    /// Photos and videos in all the groups.
    pub total: i64,
    /// How many of `total` are videos.
    pub videos: i64,
}

/// One line per group of the photos matching the query, for the group cards. Dates follow
/// the sort; places come with the most photos first.
pub fn groups(conn: &Connection, q: &PhotoQuery, by: GroupBy) -> Result<Groups> {
    let (filters, args, _) = photo_filters(q);
    let desc = newest_first(q);
    // SQLite takes the other columns from the row that has the MAX/MIN: the cover.
    let pick = if desc { "MAX(taken)" } else { "MIN(taken)" };
    let order = if by == GroupBy::Place {
        "n DESC, k"
    } else if desc {
        "k DESC"
    } else {
        "k ASC"
    };
    let key = by.key();
    let sql = format!(
        "SELECT {key} AS k, COUNT(*) AS n, id, {pick}, version, COUNT(duration) FROM photos WHERE 1 = 1{filters}
         GROUP BY k ORDER BY {order}"
    );
    let groups: Vec<Group> = conn
        .prepare_cached(&sql)?
        .query_map(params_from_iter(args), |r| {
            let key = if by == GroupBy::Place { GroupKey::Place(r.get(0)?) } else { GroupKey::Date(r.get(0)?) };
            Ok(Group { key, count: r.get(1)?, cover: r.get(2)?, version: r.get(4)?, videos: r.get(5)? })
        })?
        .collect::<Result<_, _>>()?;
    let total = groups.iter().map(|g| g.count).sum();
    let videos = groups.iter().map(|g| g.videos).sum();
    Ok(Groups { groups, total, videos })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoDetail {
    pub id: i64,
    pub path: String,
    pub taken: String,
    /// False when the date is the file's, for lack of EXIF data.
    pub date_from_exif: bool,
    pub width: i64,
    pub height: i64,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub version: i64,
    pub rotatable: bool,
    /// Videos: the length in seconds (0 when unknown); null for photos.
    pub duration: Option<f64>,
    pub faces: Vec<FaceBox>,
}

#[derive(Serialize)]
pub struct FaceBox {
    pub id: i64,
    /// x, y, width, height, relative to the photo (0..1).
    #[serde(rename = "box")]
    pub bbox: [f64; 4],
    pub person: Option<i64>,
    pub name: Option<String>,
}

/// A photo's details and faces (left to right), for the viewer.
pub fn photo_detail(conn: &Connection, id: i64) -> Result<Option<PhotoDetail>> {
    let photo = conn
        .prepare_cached(
            "SELECT p.path, p.taken, p.date_from_exif, p.width, p.height, p.lat, p.lon, pl.city, pl.region,
                    pl.country, p.version, p.duration
             FROM photos p LEFT JOIN places pl ON pl.id = p.place_id WHERE p.id = ?",
        )?
        .query_row([id], |r| {
            let path: String = r.get(0)?;
            Ok(PhotoDetail {
                id,
                rotatable: crate::rotate::can_rotate(std::path::Path::new(&path)),
                path,
                taken: r.get(1)?,
                date_from_exif: r.get(2)?,
                width: r.get(3)?,
                height: r.get(4)?,
                lat: r.get(5)?,
                lon: r.get(6)?,
                city: r.get(7)?,
                region: r.get(8)?,
                country: r.get(9)?,
                version: r.get(10)?,
                duration: r.get(11)?,
                faces: Vec::new(),
            })
        })
        .optional()?;
    let Some(mut photo) = photo else { return Ok(None) };
    photo.faces = conn
        .prepare_cached(
            "SELECT f.id, f.x, f.y, f.w, f.h, f.person_id, p.name FROM faces f
             LEFT JOIN persons p ON p.id = f.person_id WHERE f.photo_id = ? ORDER BY f.x",
        )?
        .query_map([id], |r| {
            Ok(FaceBox {
                id: r.get(0)?,
                bbox: [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?],
                person: r.get(5)?,
                name: r.get(6)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(Some(photo))
}

#[derive(Serialize)]
pub struct Place {
    pub id: i64,
    pub city: String,
    pub region: String,
    pub country: String,
    pub count: i64,
    /// The newest photo taken there.
    pub cover: i64,
}

/// The places of the photos with these people (as in the photo query) taken in this date
/// range, with the most photos first.
pub fn places(
    conn: &Connection,
    people: Option<&str>,
    match_mode: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<Place>> {
    let filter = people_filter("p.id", people, match_mode);
    let (range, args) = date_range_filter("p.taken", from, to);
    Ok(conn
        .prepare_cached(&format!(
            "SELECT pl.id, pl.city, pl.region, pl.country, COUNT(*) AS n, MAX(p.id)
             FROM photos p JOIN places pl ON pl.id = p.place_id
             WHERE 1 = 1{filter}{range}
             GROUP BY pl.id ORDER BY n DESC, pl.city"
        ))?
        .query_map(params_from_iter(args), |r| {
            Ok(Place {
                id: r.get(0)?,
                city: r.get(1)?,
                region: r.get(2)?,
                country: r.get(3)?,
                count: r.get(4)?,
                cover: r.get(5)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

/// The file of a photo in the gallery.
pub fn photo_path(conn: &Connection, id: i64) -> Result<Option<String>> {
    Ok(conn.prepare_cached("SELECT path FROM photos WHERE id = ?")?.query_row([id], |r| r.get(0)).optional()?)
}

/// A picture for the grid: a photo's thumbnail, or the sign that it is a video (which has
/// none; the server draws a generic one).
pub enum Thumbnail {
    Jpeg(Vec<u8>),
    Video,
}

/// A photo's or video's picture for the grid, and its current version.
pub fn thumbnail(conn: &Connection, photo: i64) -> Result<Option<(Thumbnail, i64)>> {
    let row: Option<(Option<Vec<u8>>, i64, bool)> = conn
        .prepare_cached(
            "SELECT t.data, p.version, p.duration IS NOT NULL FROM photos p
             LEFT JOIN thumbs t ON t.photo_id = p.id WHERE p.id = ?",
        )?
        .query_row([photo], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    Ok(row.and_then(|(data, version, video)| match (data, video) {
        (Some(jpeg), _) => Some((Thumbnail::Jpeg(jpeg), version)),
        (None, true) => Some((Thumbnail::Video, version)),
        (None, false) => None,
    }))
}

/// A face's picture (JPEG).
pub fn face_picture(conn: &Connection, face: i64) -> Result<Option<Vec<u8>>> {
    Ok(conn.prepare_cached("SELECT thumb FROM faces WHERE id = ?")?.query_row([face], |r| r.get(0)).optional()?)
}
