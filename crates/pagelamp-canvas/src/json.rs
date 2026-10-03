//! The subset of Canvas REST JSON we read. Only `id`s are strict; every other field is
//! optional AND lenient: a missing, null or oddly shaped value (e.g. a date-only timestamp)
//! becomes `None` instead of making the whole item unreadable — an unreadable item would look
//! like a deleted one. Assignment descriptions have no field here, so they are never kept
//! (they exist only transiently in the raw JSON page) — docs/ARCHITECTURE.md §3 rule 4.

use chrono::{DateTime, NaiveDate, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

/// An optional field whose wrong-shaped value becomes `None` (see module docs).
fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Option<T>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// A timestamp: RFC 3339, or a bare `YYYY-MM-DD` (midnight UTC); anything else → `None`.
fn lenient_time<'de, D: Deserializer<'de>>(d: D) -> Result<Option<DateTime<Utc>>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    let Some(serde_json::Value::String(text)) = value else {
        return Ok(None);
    };
    let text = text.trim();
    Ok(DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .map(|d| d.and_hms_opt(0, 0, 0).expect("midnight exists").and_utc())
        }))
}

/// A course or term date as Canvas sent it: an instant, or a bare `YYYY-MM-DD` date. A bare
/// date is already a calendar date and must not be moved by the course's time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Moment {
    Instant(DateTime<Utc>),
    Date(NaiveDate),
}

/// Like `lenient_time`, but keeps a bare date as a date (see `Moment`).
fn lenient_moment<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Moment>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    let Some(serde_json::Value::String(text)) = value else {
        return Ok(None);
    };
    let text = text.trim();
    Ok(DateTime::parse_from_rfc3339(text)
        .map(|t| Moment::Instant(t.with_timezone(&Utc)))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .map(Moment::Date)
        }))
}

/// A Canvas object id. Canvas sends numbers (sometimes strings, e.g. sharded "123~456").
/// Only ASCII digits and `~` are accepted, so an id can be put into a URL path safely.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct CanvasId(pub String);

impl<'de> Deserialize<'de> for CanvasId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let text = match value {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s,
            _ => return Err(serde::de::Error::custom("id must be a number or string")),
        };
        if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit() || c == '~') {
            return Err(serde::de::Error::custom("invalid Canvas id"));
        }
        Ok(CanvasId(text))
    }
}

impl std::fmt::Display for CanvasId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `GET /users/self`
#[derive(Debug, Deserialize)]
pub(crate) struct User {
    /// Required: an answer without a user id is not Canvas (see `probe_error`).
    #[allow(dead_code)]
    pub id: CanvasId,
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub short_name: Option<String>,
}

/// `GET /courses?include[]=term&include[]=syllabus_body&include[]=concluded`
#[derive(Debug, Deserialize)]
pub(crate) struct Course {
    pub id: CanvasId,
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub course_code: Option<String>,
    #[serde(default, deserialize_with = "lenient_moment")]
    pub start_at: Option<Moment>,
    #[serde(default, deserialize_with = "lenient_moment")]
    pub end_at: Option<Moment>,
    #[serde(default, deserialize_with = "lenient")]
    pub term: Option<Term>,
    #[serde(default, deserialize_with = "lenient")]
    pub syllabus_body: Option<String>,
    /// Canvas returns a stub `{id, access_restricted_by_date: true}` for courses outside
    /// their dates; those are not upserted (only `lms_access_restricted` is recorded).
    #[serde(default, deserialize_with = "lenient")]
    pub access_restricted_by_date: Option<bool>,
    /// The course's IANA time zone, e.g. "America/Toronto".
    #[serde(default, deserialize_with = "lenient")]
    pub time_zone: Option<String>,
    /// `unpublished`, `available`, `completed` or `deleted`.
    #[serde(default, deserialize_with = "lenient")]
    pub workflow_state: Option<String>,
    /// Sent with `include[]=concluded`: the course (or its term) has ended for the student.
    #[serde(default, deserialize_with = "lenient")]
    pub concluded: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Term {
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient_moment")]
    pub start_at: Option<Moment>,
    #[serde(default, deserialize_with = "lenient_moment")]
    pub end_at: Option<Moment>,
}

/// `GET /courses/:id/tabs`
#[derive(Debug, Deserialize)]
pub(crate) struct Tab {
    pub id: String,
    #[serde(default, deserialize_with = "lenient")]
    pub hidden: Option<bool>,
}

/// `GET /courses/:id/modules?include[]=items&include[]=content_details`
#[derive(Debug, Deserialize)]
pub(crate) struct Module {
    pub id: CanvasId,
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub position: Option<i64>,
    #[serde(default, deserialize_with = "lenient_time")]
    pub unlock_at: Option<DateTime<Utc>>,
    /// Omitted by Canvas when a module has many items (then use the items endpoint).
    #[serde(default, deserialize_with = "lenient")]
    pub items: Option<Vec<serde_json::Value>>,
    #[serde(default, deserialize_with = "lenient")]
    pub items_count: Option<u64>,
}

/// A module item (parsed from `Module::items` one by one so a bad item is skipped alone).
#[derive(Debug, Deserialize)]
pub(crate) struct ModuleItem {
    pub id: CanvasId,
    #[serde(default, deserialize_with = "lenient")]
    pub title: Option<String>,
    #[serde(rename = "type", default, deserialize_with = "lenient")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub content_id: Option<CanvasId>,
    #[serde(default, deserialize_with = "lenient")]
    pub page_url: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub external_url: Option<String>,
}

/// `GET /courses/:id/files` and `/courses/:id/files/:file_id`
#[derive(Debug, Deserialize)]
pub(crate) struct File {
    pub id: CanvasId,
    #[serde(default, deserialize_with = "lenient")]
    pub display_name: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub filename: Option<String>,
    #[serde(rename = "content-type", default, deserialize_with = "lenient")]
    pub content_type: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub size: Option<u64>,
    /// Download URL (Canvas host; redirects to file storage).
    #[serde(default, deserialize_with = "lenient")]
    pub url: Option<String>,
    #[serde(default, deserialize_with = "lenient_time")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "lenient_time")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "lenient")]
    pub locked_for_user: Option<bool>,
}

/// `GET /courses/:id/pages` (no body) and `/courses/:id/pages/:url_or_id` (with body)
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Page {
    #[serde(default, deserialize_with = "lenient")]
    pub page_id: Option<CanvasId>,
    /// The course's front page (S4). Whether the pages list carries it is unverified; when it
    /// doesn't, the signal is simply absent.
    #[serde(default, deserialize_with = "lenient")]
    pub front_page: Option<bool>,
    /// The page's slug (used to fetch it).
    #[serde(default, deserialize_with = "lenient")]
    pub url: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub html_url: Option<String>,
    #[serde(default, deserialize_with = "lenient_time")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "lenient_time")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "lenient")]
    pub body: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub locked_for_user: Option<bool>,
}

/// `GET /courses/:id/assignments` — deliberately WITHOUT `description`: assignment
/// instructions are dropped with the raw page and never stored.
#[derive(Debug, Deserialize)]
pub(crate) struct Assignment {
    pub id: CanvasId,
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient_time")]
    pub due_at: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "lenient")]
    pub html_url: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub submission_types: Option<Vec<String>>,
}

/// `GET /announcements?context_codes[]=course_:id`
#[derive(Debug, Deserialize)]
pub(crate) struct Announcement {
    pub id: CanvasId,
    #[serde(default, deserialize_with = "lenient")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub message: Option<String>,
    #[serde(default, deserialize_with = "lenient_time")]
    pub posted_at: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "lenient")]
    pub html_url: Option<String>,
}

/// `GET /planner/items`
#[derive(Debug, Deserialize)]
pub(crate) struct PlannerItem {
    #[serde(default, deserialize_with = "lenient")]
    pub plannable_id: Option<CanvasId>,
    #[serde(default, deserialize_with = "lenient")]
    pub plannable_type: Option<String>,
    #[serde(default, deserialize_with = "lenient_time")]
    pub plannable_date: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "lenient")]
    pub course_id: Option<CanvasId>,
    #[serde(default, deserialize_with = "lenient")]
    pub html_url: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub plannable: Option<Plannable>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Plannable {
    #[serde(default, deserialize_with = "lenient")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "lenient")]
    pub name: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn ids_accept_numbers_and_sharded_strings_only() {
        let id: CanvasId = serde_json::from_value(json!(12345)).unwrap();
        assert_eq!(id.0, "12345");
        let id: CanvasId = serde_json::from_value(json!("1234~56")).unwrap();
        assert_eq!(id.0, "1234~56");
        for bad in [
            json!("../../users/1"),
            json!("12/34"),
            json!(""),
            json!(null),
            json!(1.5),
        ] {
            assert!(
                serde_json::from_value::<CanvasId>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn odd_field_values_become_none_instead_of_dropping_the_item() {
        let course: Course = serde_json::from_value(json!({
            "id": 202, "name": "Advanced Demo Studies", "start_at": "2026-09-07",
            "end_at": "not a date", "access_restricted_by_date": "yes", "term": 5
        }))
        .unwrap();
        assert_eq!(
            course.start_at,
            Some(Moment::Date(NaiveDate::from_ymd_opt(2026, 9, 7).unwrap()))
        );
        assert!(course.end_at.is_none() && course.term.is_none());
        assert!(course.access_restricted_by_date.is_none());
        let file: File =
            serde_json::from_value(json!({"id": 1, "size": "big", "display_name": 42})).unwrap();
        assert!(file.size.is_none() && file.display_name.is_none());
    }

    /// CAL-16 `concluded_field_is_lenient`: a missing, wrongly typed or true `concluded`
    /// (and the other new course fields) never make the course unreadable.
    #[test]
    fn concluded_field_is_lenient() {
        let course =
            |value: serde_json::Value| -> Course { serde_json::from_value(value).unwrap() };
        assert_eq!(course(json!({"id": 1})).concluded, None);
        assert_eq!(
            course(json!({"id": 1, "concluded": "true"})).concluded,
            None
        );
        assert_eq!(course(json!({"id": 1, "concluded": 1})).concluded, None);
        assert_eq!(course(json!({"id": 1, "concluded": null})).concluded, None);
        assert_eq!(
            course(json!({"id": 1, "concluded": true})).concluded,
            Some(true)
        );
        assert_eq!(
            course(json!({"id": 1, "concluded": false})).concluded,
            Some(false)
        );
        let odd = course(json!({
            "id": 1, "time_zone": 5, "workflow_state": ["completed"],
            "term": {"name": 2026, "start_at": "2026-05-04T04:00:00Z"}
        }));
        assert!(odd.time_zone.is_none() && odd.workflow_state.is_none());
        let term = odd.term.unwrap();
        assert!(term.name.is_none());
        assert!(matches!(term.start_at, Some(Moment::Instant(_))));
    }

    #[test]
    fn missing_and_null_fields_are_tolerated() {
        let course: Course = serde_json::from_value(json!({"id": 1, "name": null})).unwrap();
        assert!(course.name.is_none() && course.term.is_none());
        let assignment: Assignment = serde_json::from_value(json!({
            "id": 9, "name": "PS1", "description": "<p>secret instructions</p>", "due_at": null
        }))
        .unwrap();
        assert_eq!(assignment.name.as_deref(), Some("PS1"));
    }
}
