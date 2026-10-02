//! The photo query the UI sends, and the SQL conditions it turns into.

use chrono::{Datelike, Duration, Local};
use rusqlite::types::Value;
use serde::Deserialize;

#[derive(Deserialize)]
pub(super) struct PhotoQuery {
    /// "asc" or "desc" (default) by capture date
    pub(super) sort: Option<String>,
    place: Option<i64>,
    /// comma separated person ids
    people: Option<String>,
    /// "all" (default): every selected person appears, "any": at least one,
    /// "only": all of them and no other known person
    #[serde(rename = "match")]
    match_mode: Option<String>,
    /// YYYY, YYYY-MM or YYYY-MM-DD prefix of the capture date
    date: Option<String>,
    /// capture date range, YYYY-MM-DD, both days included
    from: Option<String>,
    to: Option<String>,
    /// photos from previous years whose anniversary falls within the next `upcoming` days
    upcoming: Option<u32>,
    /// /api/groups only: "year", "month", "day" or "place"
    pub(super) by: Option<String>,
}

pub(super) fn parse_ids(s: &str) -> Vec<i64> {
    s.split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

/// SQL condition (starting with " AND") and its arguments keeping photos whose `taken`
/// column falls in the range, both days included. Either end may be missing; invalid
/// dates are ignored, and a reversed range is put the right way round.
pub(super) fn date_range_filter(taken: &str, from: Option<&str>, to: Option<&str>) -> (String, Vec<Value>) {
    let parse = |d: Option<&str>| d.and_then(|d| chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d").ok());
    let (mut from, mut to) = (parse(from), parse(to));
    if let (Some(f), Some(t)) = (from, to)
        && f > t
    {
        (from, to) = (Some(t), Some(f));
    }
    let mut sql = String::new();
    let mut args = Vec::new();
    // `taken` is "YYYY-MM-DD HH:MM:SS", so plain text comparison orders it by time.
    if let Some(f) = from {
        sql.push_str(&format!(" AND {taken} >= ?"));
        args.push(Value::from(f.format("%Y-%m-%d").to_string()));
    }
    if let Some(t) = to.and_then(|t| t.succ_opt()) {
        sql.push_str(&format!(" AND {taken} < ?"));
        args.push(Value::from(t.format("%Y-%m-%d").to_string()));
    }
    (sql, args)
}

/// SQL condition (starting with " AND") keeping photos, by their `photo_id` column, where
/// the given people appear. `match_mode`: "all" (default) all of them together, "any" at
/// least one, "only" all of them and no other known person. Empty without people.
pub(super) fn people_filter(photo_id: &str, people: Option<&str>, match_mode: Option<&str>) -> String {
    let people = people.map(parse_ids).unwrap_or_default();
    if people.is_empty() {
        return String::new();
    }
    // ids are parsed integers, so inlining them is safe
    let list = people.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    match match_mode.unwrap_or("all") {
        "any" => format!(" AND {photo_id} IN (SELECT photo_id FROM faces WHERE person_id IN ({list}))"),
        mode => {
            let mut sql = format!(
                " AND {photo_id} IN (SELECT photo_id FROM faces WHERE person_id IN ({list})
                                     GROUP BY photo_id HAVING COUNT(DISTINCT person_id) = {})",
                people.len()
            );
            if mode == "only" {
                sql.push_str(&format!(
                    " AND {photo_id} NOT IN (SELECT photo_id FROM faces f JOIN persons pe ON pe.id = f.person_id
                                             WHERE pe.hidden = 0 AND f.person_id NOT IN ({list}))"
                ));
            }
            sql
        }
    }
}

/// `MM-DD` for today and the following days, and the current year.
pub(super) fn upcoming_days(days: u32) -> (Vec<String>, i32) {
    let today = Local::now().date_naive();
    let list =
        (0..days.clamp(1, 366)).map(|d| (today + Duration::days(d as i64)).format("%m-%d").to_string()).collect();
    (list, today.year())
}

/// The filters of a photo query as SQL conditions (each starting with " AND") and their
/// arguments, shared by the photo list and the group summaries so both always agree. Also
/// returns the upcoming days (`MM-DD`) when `upcoming` is set.
pub(super) fn photo_filters(q: &PhotoQuery) -> (String, Vec<Value>, Vec<String>) {
    let mut sql = String::new();
    let mut args: Vec<Value> = Vec::new();
    match q.place {
        // 0: photos without a location
        Some(0) => sql.push_str(" AND place_id IS NULL"),
        Some(place) => {
            sql.push_str(" AND place_id = ?");
            args.push(place.into());
        }
        None => {}
    }
    if let Some(date) = q.date.as_deref().filter(|d| !d.is_empty()) {
        sql.push_str(" AND taken LIKE ? || '%'");
        args.push(date.to_string().into());
    }
    let (range, range_args) = date_range_filter("taken", q.from.as_deref(), q.to.as_deref());
    sql.push_str(&range);
    args.extend(range_args);
    let mut days = Vec::new();
    if let Some(n) = q.upcoming {
        let (list, year) = upcoming_days(n);
        sql.push_str(&format!(
            " AND substr(taken, 6, 5) IN ({}) AND CAST(substr(taken, 1, 4) AS INTEGER) < ?",
            vec!["?"; list.len()].join(",")
        ));
        args.extend(list.iter().cloned().map(Value::from));
        args.push((year as i64).into());
        days = list;
    }
    sql.push_str(&people_filter("id", q.people.as_deref(), q.match_mode.as_deref()));
    (sql, args, days)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params, params_from_iter};
    use serde_json::{Value as JsonValue, json};

    /// Six photos and four people (Carl hidden):
    /// 1 Ana + Ben, 2 Ana, 3 Ben, 4 Ana + Carl, 5 nobody, 6 Ana + Dan.
    fn library() -> Connection {
        let conn = crate::db::open_in_memory();
        conn.execute_batch(
            "INSERT INTO persons (id, name, hidden) VALUES (1, 'Ana', 0), (2, 'Ben', 0), (3, 'Carl', 1), (4, NULL, 0);
             INSERT INTO photos (id, path, mtime, size, width, height, taken, date_from_exif) VALUES
                 (1, '/p/1.jpg', 0, 1, 1, 1, '2020-03-10 12:00:00', 1),
                 (2, '/p/2.jpg', 0, 1, 1, 1, '2020-03-11 00:00:00', 1),
                 (3, '/p/3.jpg', 0, 1, 1, 1, '2021-05-01 08:00:00', 1),
                 (4, '/p/4.jpg', 0, 1, 1, 1, '2021-05-02 23:59:59', 1),
                 (5, '/p/5.jpg', 0, 1, 1, 1, '2022-01-01 10:00:00', 1),
                 (6, '/p/6.jpg', 0, 1, 1, 1, '2022-06-01 10:00:00', 1);",
        )
        .unwrap();
        for (photo, person) in [(1, 1), (1, 2), (2, 1), (3, 2), (4, 1), (4, 3), (6, 1), (6, 4)] {
            conn.execute(
                "INSERT INTO faces (photo_id, x, y, w, h, score, embedding, thumb, person_id) VALUES (?, 0, 0, 1, 1, 1, x'', x'', ?)",
                [photo, person],
            )
            .unwrap();
        }
        conn
    }

    /// The photos a query (as the UI sends it) keeps.
    fn ids(conn: &Connection, query: JsonValue) -> Vec<i64> {
        let q: PhotoQuery = serde_json::from_value(query).unwrap();
        let (filters, args, _) = photo_filters(&q);
        conn.prepare(&format!("SELECT id FROM photos WHERE 1 = 1{filters} ORDER BY id"))
            .unwrap()
            .query_map(params_from_iter(args), |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn people_together_any_or_only() {
        let conn = library();
        assert_eq!(ids(&conn, json!({ "people": "1,2" })), [1], "together is the default");
        assert_eq!(ids(&conn, json!({ "people": "1,2", "match": "all" })), [1]);
        assert_eq!(ids(&conn, json!({ "people": "1,2", "match": "any" })), [1, 2, 3, 4, 6]);
        // Only Ana: hidden Carl doesn't count, unnamed person 4 does.
        assert_eq!(ids(&conn, json!({ "people": "1", "match": "only" })), [2, 4]);
        assert_eq!(ids(&conn, json!({ "people": "1,2", "match": "only" })), [1]);
        assert_eq!(ids(&conn, json!({ "people": "1, x,,2" })), [1], "junk in the list is ignored");
        assert_eq!(ids(&conn, json!({ "people": "" })), [1, 2, 3, 4, 5, 6], "no people, no filter");
    }

    #[test]
    fn date_ranges_include_both_days() {
        let conn = library();
        assert_eq!(ids(&conn, json!({ "from": "2020-03-11", "to": "2021-05-02" })), [2, 3, 4]);
        assert_eq!(ids(&conn, json!({ "from": "2021-05-02", "to": "2020-03-11" })), [2, 3, 4], "reversed");
        assert_eq!(ids(&conn, json!({ "from": "2021-05-01" })), [3, 4, 5, 6]);
        assert_eq!(ids(&conn, json!({ "to": "2020-03-10" })), [1]);
        assert_eq!(ids(&conn, json!({ "from": "2021-13-40", "to": "2020-03-10" })), [1], "an invalid day is ignored");
        assert_eq!(ids(&conn, json!({ "date": "2021" })), [3, 4], "a year");
        assert_eq!(ids(&conn, json!({ "date": "2020-03-10" })), [1], "a day");
        assert_eq!(ids(&conn, json!({ "people": "1", "date": "2021" })), [4], "filters combine");
    }

    #[test]
    fn upcoming_is_past_years_only() {
        let conn = library();
        let today = Local::now().date_naive();
        let on = |years_ago: i32, days_ahead: i64| {
            let day = today + Duration::days(days_ahead);
            format!("{}-{} 12:00:00", today.year() - years_ago, day.format("%m-%d"))
        };
        conn.execute_batch("DELETE FROM photos").unwrap();
        for (id, taken) in [(10, on(3, 0)), (11, on(1, 5)), (12, on(0, 0)), (13, on(2, 40))] {
            conn.execute(
                "INSERT INTO photos (id, path, mtime, size, width, height, taken, date_from_exif) VALUES (?, ?, 0, 1, 1, 1, ?, 1)",
                params![id, format!("/p/{id}.jpg"), taken],
            )
            .unwrap();
        }
        // Today three years ago and in five days a year ago; not this year's, not in 40 days.
        assert_eq!(ids(&conn, json!({ "upcoming": 30 })), [10, 11]);
        assert_eq!(ids(&conn, json!({ "upcoming": 1 })), [10]);
        assert_eq!(upcoming_days(0).0.len(), 1, "at least today");
        assert_eq!(upcoming_days(10_000).0.len(), 366, "at most a year");
    }
}
