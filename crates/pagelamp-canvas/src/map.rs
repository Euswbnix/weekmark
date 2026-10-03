//! Canvas JSON → PageLamp model, as pure functions (tested on JSON fixtures, no HTTP).
//!
//! Ids follow `pagelamp_core::model` conventions: `<source>/course/<id>`, `<source>/module/<id>`,
//! `<source>/file/<id>`, `<source>/page/<id>`, `<source>/link/<module item id>`,
//! `<source>/announcement/<id>`, `<source>/syllabus/<course id>`, `<source>/assignment/<id>`,
//! `<source>/planner/<type>/<id>`.

use chrono::{DateTime, NaiveDate, Utc};
use pagelamp_core::dates::{self, Tz};
use pagelamp_core::model::{
    CourseUpsert, Event, EventKind, LmsCourseInfo, MaterialKind, MaterialUpsert, Module,
};
use pagelamp_core::timeline::parse_week_hint;
use url::Url;

use crate::json::{self, CanvasId};

/// Builds ids for one source.
#[derive(Clone, Copy)]
pub(crate) struct Ids<'a> {
    pub source: &'a str,
}

impl Ids<'_> {
    pub(crate) fn course(&self, id: &CanvasId) -> String {
        format!("{}/course/{id}", self.source)
    }
    pub(crate) fn module(&self, id: &CanvasId) -> String {
        format!("{}/module/{id}", self.source)
    }
    pub(crate) fn file(&self, id: &CanvasId) -> String {
        format!("{}/file/{id}", self.source)
    }
    pub(crate) fn page(&self, id: &CanvasId) -> String {
        format!("{}/page/{id}", self.source)
    }
    pub(crate) fn link(&self, item: &CanvasId) -> String {
        format!("{}/link/{item}", self.source)
    }
    pub(crate) fn announcement(&self, id: &CanvasId) -> String {
        format!("{}/announcement/{id}", self.source)
    }
    pub(crate) fn syllabus(&self, course: &CanvasId) -> String {
        format!("{}/syllabus/{course}", self.source)
    }
}

/// Where a material sits in the module structure.
#[derive(Clone, Debug, Default)]
pub(crate) struct Placement {
    pub module_id: Option<String>,
    pub module_week: Option<u32>,
}

/// A course, or None for date-restricted stubs / nameless entries.
///
/// The LMS facts go to `lms` raw (calendar design §5, S2/S3): course and term dates apart, as
/// dates in the course's time zone. `term_start`/`term_end` keep v0.1's "term first" merge,
/// with the same conversion.
pub(crate) fn course(ids: Ids<'_>, base: &Url, course: &json::Course) -> Option<CourseUpsert> {
    if course.access_restricted_by_date == Some(true) {
        return None;
    }
    let name = course
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())?;
    let tz = course.time_zone.as_deref().and_then(dates::time_zone);
    let day = |moment: Option<json::Moment>| moment.map(|m| moment_date(m, tz));
    let term = course.term.as_ref();
    let lms = LmsCourseInfo {
        term_name: term.and_then(|t| non_empty(t.name.as_deref())),
        term_start: day(term.and_then(|t| t.start_at)),
        term_end: day(term.and_then(|t| t.end_at)),
        course_start: day(course.start_at),
        course_end: day(course.end_at),
        time_zone: tz.map(|tz| tz.name().to_string()),
        concluded: course.concluded,
        workflow_state: non_empty(course.workflow_state.as_deref()),
        // A course upserted here was listed with its full fields, so not restricted.
        access_restricted: Some(false),
    };
    let start = lms.term_start.or(lms.course_start);
    let end = lms.term_end.or(lms.course_end);
    Some(CourseUpsert {
        id: ids.course(&course.id),
        source_id: ids.source.to_string(),
        external_id: course.id.0.clone(),
        code: course
            .course_code
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string),
        name: name.to_string(),
        term_start: start,
        term_end: end,
        url: Some(canvas_url(base, &["courses", &course.id.0])),
        syllabus_text: course
            .syllabus_body
            .as_deref()
            .map(html_to_text)
            .filter(|t| !t.is_empty()),
        lms,
    })
}

/// The course calendar date of a Canvas course or term date (a bare date stays as it is).
fn moment_date(moment: json::Moment, tz: Option<Tz>) -> NaiveDate {
    match moment {
        json::Moment::Instant(instant) => dates::course_date(instant, tz),
        json::Moment::Date(date) => date,
    }
}

/// `text` trimmed, or None when that leaves nothing.
fn non_empty(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

pub(crate) fn module(ids: Ids<'_>, course_id: &str, module: &json::Module) -> Module {
    let name = module
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| format!("Module {}", module.id));
    Module {
        id: ids.module(&module.id),
        course_id: course_id.to_string(),
        week_hint: parse_week_hint(&name),
        name,
        position: module.position,
        unlock_at: module.unlock_at,
    }
}

/// A course file. `title` overrides the file name (module items carry the display title).
pub(crate) fn file(
    ids: Ids<'_>,
    base: &Url,
    course: &CanvasId,
    course_id: &str,
    file: &json::File,
    title: Option<&str>,
    placement: &Placement,
) -> MaterialUpsert {
    let title = title
        .or(file.display_name.as_deref())
        .or(file.filename.as_deref())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("File {}", file.id));
    MaterialUpsert {
        id: ids.file(&file.id),
        course_id: course_id.to_string(),
        module_id: placement.module_id.clone(),
        kind: MaterialKind::File,
        week_hint: parse_week_hint(&title).or(placement.module_week),
        title,
        url: Some(canvas_url(
            base,
            &["courses", &course.0, "files", &file.id.0],
        )),
        local_path: None,
        mime: file.content_type.clone().filter(|m| !m.is_empty()),
        published_at: file.updated_at.or(file.created_at),
    }
}

pub(crate) fn page(
    ids: Ids<'_>,
    base: &Url,
    course_id: &str,
    page_id: &CanvasId,
    page: &json::Page,
    placement: &Placement,
) -> MaterialUpsert {
    let title = page
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| format!("Page {page_id}"));
    MaterialUpsert {
        id: ids.page(page_id),
        course_id: course_id.to_string(),
        module_id: placement.module_id.clone(),
        kind: MaterialKind::Page,
        week_hint: parse_week_hint(&title).or(placement.module_week),
        title,
        url: page.html_url.as_deref().and_then(|u| absolute_url(base, u)),
        local_path: None,
        mime: Some("text/html".into()),
        published_at: page.updated_at.or(page.created_at),
    }
}

/// An `ExternalUrl` module item (title + link only).
pub(crate) fn link(
    ids: Ids<'_>,
    course_id: &str,
    item: &json::ModuleItem,
    placement: &Placement,
) -> Option<MaterialUpsert> {
    let url = item
        .external_url
        .as_deref()
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))?;
    let title = item
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| url.to_string());
    Some(MaterialUpsert {
        id: ids.link(&item.id),
        course_id: course_id.to_string(),
        module_id: placement.module_id.clone(),
        kind: MaterialKind::ExternalLink,
        week_hint: parse_week_hint(&title).or(placement.module_week),
        title,
        url: Some(url.to_string()),
        local_path: None,
        mime: None,
        published_at: None,
    })
}

pub(crate) fn announcement(
    ids: Ids<'_>,
    base: &Url,
    course_id: &str,
    announcement: &json::Announcement,
) -> MaterialUpsert {
    let title = announcement
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Announcement".into());
    MaterialUpsert {
        id: ids.announcement(&announcement.id),
        course_id: course_id.to_string(),
        module_id: None,
        kind: MaterialKind::Announcement,
        week_hint: None,
        title,
        url: announcement
            .html_url
            .as_deref()
            .and_then(|u| absolute_url(base, u)),
        local_path: None,
        mime: Some("text/html".into()),
        published_at: announcement.posted_at,
    }
}

pub(crate) fn syllabus(
    ids: Ids<'_>,
    base: &Url,
    course: &CanvasId,
    course_id: &str,
) -> MaterialUpsert {
    MaterialUpsert {
        id: ids.syllabus(course),
        course_id: course_id.to_string(),
        module_id: None,
        kind: MaterialKind::Syllabus,
        week_hint: None,
        title: "Syllabus".into(),
        url: Some(canvas_url(
            base,
            &["courses", &course.0, "assignments", "syllabus"],
        )),
        local_path: None,
        mime: Some("text/html".into()),
        published_at: None,
    }
}

/// A due date (title + date + link ONLY — docs/ARCHITECTURE.md §3 rule 4). None without a
/// due date.
pub(crate) fn assignment(
    ids: Ids<'_>,
    base: &Url,
    course_id: &str,
    assignment: &json::Assignment,
    now: DateTime<Utc>,
) -> Option<Event> {
    let due_at = assignment.due_at?;
    let quiz = assignment
        .submission_types
        .as_ref()
        .is_some_and(|types| types.iter().any(|t| t == "online_quiz"));
    Some(Event {
        id: format!("{}/assignment/{}", ids.source, assignment.id),
        source_id: ids.source.to_string(),
        course_id: Some(course_id.to_string()),
        kind: if quiz {
            EventKind::QuizDue
        } else {
            EventKind::AssignmentDue
        },
        title: assignment
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("Assignment {}", assignment.id)),
        starts_at: None,
        ends_at: None,
        due_at: Some(due_at),
        url: assignment
            .html_url
            .as_deref()
            .and_then(|u| absolute_url(base, u)),
        updated_at: now,
        course_hint: None,
    })
}

/// Planner item types that are NOT already covered by the assignments endpoint.
const PLANNER_TYPES: [&str; 4] = [
    "calendar_event",
    "planner_note",
    "wiki_page",
    "discussion_topic",
];

/// A planner item → event (title + date + link). `course_id` maps Canvas course ids to ours;
/// items of courses outside it are skipped (not synced this time), items without a course
/// (personal notes) are kept.
pub(crate) fn planner_item(
    ids: Ids<'_>,
    base: &Url,
    item: &json::PlannerItem,
    course_id: impl Fn(&CanvasId) -> Option<String>,
    now: DateTime<Utc>,
) -> Option<Event> {
    let kind = item.plannable_type.as_deref()?;
    if !PLANNER_TYPES.contains(&kind) {
        return None;
    }
    let id = item.plannable_id.as_ref()?;
    let date = item.plannable_date?;
    let course = match &item.course_id {
        Some(canvas_course) => Some(course_id(canvas_course)?),
        None => None,
    };
    let plannable = item.plannable.as_ref();
    let title = plannable
        .and_then(|p| p.title.clone().or_else(|| p.name.clone()))
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "Planner item".into());
    Some(Event {
        id: format!("{}/planner/{kind}/{id}", ids.source),
        source_id: ids.source.to_string(),
        course_id: course,
        kind: if kind == "calendar_event" {
            EventKind::ClassEvent
        } else {
            EventKind::PlannerItem
        },
        title,
        starts_at: None,
        ends_at: None,
        due_at: Some(date),
        url: item.html_url.as_deref().and_then(|u| absolute_url(base, u)),
        updated_at: now,
        course_hint: None,
    })
}

/// LMS HTML → plain text (for the course's syllabus_text field).
pub(crate) fn html_to_text(html: &str) -> String {
    pagelamp_extract::extract_html(html)
        .into_iter()
        .map(|segment| segment.text)
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string()
}

/// Files of this course the syllabus links to (S5): `/courses/<course>/files/<file>` anywhere
/// in its HTML (links, download links, API endpoints), as material ids.
pub(crate) fn syllabus_file_links(
    ids: Ids<'_>,
    course: &CanvasId,
    html: &str,
) -> std::collections::BTreeSet<String> {
    static FILE_LINK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"/courses/([0-9~]+)/files/([0-9~]+)").expect("valid regex")
    });
    FILE_LINK
        .captures_iter(html)
        .filter(|captures| captures[1] == course.0)
        .map(|captures| ids.file(&CanvasId(captures[2].to_string())))
        .collect()
}

/// `base` + path segments (each percent-encoded).
fn canvas_url(base: &Url, segments: &[&str]) -> String {
    let mut url = base.clone();
    url.set_query(None);
    if let Ok(mut path) = url.path_segments_mut() {
        path.clear().extend(segments);
    }
    url.to_string()
}

/// Make a Canvas link absolute; only http(s) links are kept.
fn absolute_url(base: &Url, link: &str) -> Option<String> {
    let url = base.join(link).ok()?;
    matches!(url.scheme(), "https" | "http").then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn base() -> Url {
        Url::parse("https://lms.example.edu").unwrap()
    }

    fn ids() -> Ids<'static> {
        Ids {
            source: "canvas:lms.example.edu",
        }
    }

    fn from<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> T {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn courses_use_term_dates_and_skip_restricted_stubs() {
        let course: json::Course = from(json!({
            "id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101H1 F",
            "start_at": "2026-08-01T00:00:00Z",
            "term": {"start_at": "2026-09-07T04:00:00Z", "end_at": "2026-12-18T05:00:00Z"},
            "syllabus_body": "<h1>Syllabus</h1><p>Weekly quizzes.</p><script>x()</script>"
        }));
        let upsert = super::course(ids(), &base(), &course).unwrap();
        assert_eq!(upsert.id, "canvas:lms.example.edu/course/101");
        assert_eq!(upsert.code.as_deref(), Some("DEMO101H1 F"));
        assert_eq!(upsert.term_start.unwrap().to_string(), "2026-09-07");
        assert_eq!(upsert.term_end.unwrap().to_string(), "2026-12-18");
        assert_eq!(
            upsert.url.as_deref(),
            Some("https://lms.example.edu/courses/101")
        );
        let syllabus = upsert.syllabus_text.unwrap();
        assert!(syllabus.contains("Weekly quizzes.") && !syllabus.contains("x()"));

        // No time zone: UTC dates, as in v0.1; the LMS facts are kept raw.
        assert_eq!(upsert.lms.term_start, upsert.term_start);
        assert_eq!(upsert.lms.course_start.unwrap().to_string(), "2026-08-01");
        assert_eq!(upsert.lms.time_zone, None);
        assert_eq!(upsert.lms.access_restricted, Some(false));

        let stub: json::Course = from(json!({"id": 5, "access_restricted_by_date": true}));
        assert!(super::course(ids(), &base(), &stub).is_none());
        let nameless: json::Course = from(json!({"id": 6, "name": "  "}));
        assert!(super::course(ids(), &base(), &nameless).is_none());
    }

    #[test]
    fn courses_keep_raw_lms_dates_in_the_course_time_zone() {
        // A UofT-style enrollment-window term (synthetic dates) and course dates that end at
        // 23:59 local time, which is the next day in UTC.
        let course: json::Course = from(json!({
            "id": 332, "name": "Demo Methods", "course_code": "DEM332H5 F LEC0101 20269",
            "time_zone": "America/Toronto", "workflow_state": " available ",
            "concluded": false,
            "start_at": "2026-09-08T13:00:00Z", "end_at": "2026-12-09T04:59:00Z",
            "term": {"name": " Fall 2026 ", "start_at": "2026-05-04T04:00:00Z",
                     "end_at": "2027-02-01T04:59:00Z"}
        }));
        let upsert = super::course(ids(), &base(), &course).unwrap();
        let date = |text: &str| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok();
        assert_eq!(
            upsert.lms,
            LmsCourseInfo {
                term_name: Some("Fall 2026".into()),
                term_start: date("2026-05-04"),
                term_end: date("2027-01-31"),
                course_start: date("2026-09-08"),
                course_end: date("2026-12-08"),
                time_zone: Some("America/Toronto".into()),
                concluded: Some(false),
                workflow_state: Some("available".into()),
                access_restricted: Some(false),
            }
        );
        // The merged dates use the same conversion (term first, as in v0.1).
        assert_eq!(upsert.term_start, date("2026-05-04"));
        assert_eq!(upsert.term_end, date("2027-01-31"));

        // Unknown time zone → UTC; bare dates are never shifted; no term → course dates.
        let odd: json::Course = from(json!({
            "id": 333, "name": "Demo Seminar", "time_zone": "Mars/Olympus_Mons",
            "concluded": "yes", "start_at": "2026-09-08", "end_at": "2026-12-09T04:59:00Z"
        }));
        let upsert = super::course(ids(), &base(), &odd).unwrap();
        assert_eq!(upsert.lms.time_zone, None);
        assert_eq!(upsert.lms.concluded, None);
        assert_eq!(upsert.lms.term_name, None);
        assert_eq!(upsert.lms.course_start, date("2026-09-08"));
        assert_eq!(upsert.lms.course_end, date("2026-12-09"));
        assert_eq!(
            (upsert.term_start, upsert.term_end),
            (date("2026-09-08"), date("2026-12-09"))
        );
    }

    #[test]
    fn files_take_week_from_title_then_module() {
        let f: json::File = from(json!({
            "id": 77, "display_name": "Lecture slides.pdf", "content-type": "application/pdf",
            "size": 1234, "url": "https://lms.example.edu/files/77/download",
            "updated_at": "2026-09-20T10:00:00Z"
        }));
        let placement = Placement {
            module_id: Some("canvas:lms.example.edu/module/3".into()),
            module_week: Some(3),
        };
        let m = file(
            ids(),
            &base(),
            &CanvasId("101".into()),
            "c",
            &f,
            None,
            &placement,
        );
        assert_eq!(m.id, "canvas:lms.example.edu/file/77");
        assert_eq!(m.week_hint, Some(3));
        assert_eq!(
            m.url.as_deref(),
            Some("https://lms.example.edu/courses/101/files/77")
        );
        assert_eq!(m.mime.as_deref(), Some("application/pdf"));
        let titled = file(
            ids(),
            &base(),
            &CanvasId("101".into()),
            "c",
            &f,
            Some("Week 5 reading"),
            &placement,
        );
        assert_eq!(titled.week_hint, Some(5));
        assert_eq!(titled.title, "Week 5 reading");
    }

    #[test]
    fn assignments_keep_only_title_date_and_link() {
        let now = Utc::now();
        let a: json::Assignment = from(json!({
            "id": 9, "name": "Problem Set 1", "due_at": "2026-09-30T03:59:00Z",
            "html_url": "/courses/101/assignments/9",
            "description": "<p>Secret instructions</p>", "submission_types": ["online_upload"]
        }));
        let event = assignment(ids(), &base(), "c", &a, now).unwrap();
        assert_eq!(event.kind, EventKind::AssignmentDue);
        assert_eq!(
            event.url.as_deref(),
            Some("https://lms.example.edu/courses/101/assignments/9")
        );
        assert!(!serde_json::to_string(&event).unwrap().contains("Secret"));
        let quiz: json::Assignment = from(json!({
            "id": 10, "name": "Quiz 1", "due_at": "2026-09-30T03:59:00Z",
            "submission_types": ["online_quiz"]
        }));
        assert_eq!(
            assignment(ids(), &base(), "c", &quiz, now).unwrap().kind,
            EventKind::QuizDue
        );
        let undated: json::Assignment = from(json!({"id": 11, "name": "Someday"}));
        assert!(assignment(ids(), &base(), "c", &undated, now).is_none());
    }

    #[test]
    fn planner_items_skip_assignments_and_unsynced_courses() {
        let now = Utc::now();
        let map = |id: &CanvasId| (id.0 == "101").then(|| "ours/101".to_string());
        let item = |value: serde_json::Value| -> json::PlannerItem { from(value) };
        let note = item(json!({"plannable_id": 1, "plannable_type": "planner_note",
            "plannable_date": "2026-10-01T12:00:00Z", "plannable": {"title": "Buy a lab coat"}}));
        let event = planner_item(ids(), &base(), &note, map, now).unwrap();
        assert_eq!(
            (event.kind, event.course_id.clone()),
            (EventKind::PlannerItem, None)
        );
        let lecture = item(
            json!({"plannable_id": 2, "plannable_type": "calendar_event", "course_id": 101,
            "plannable_date": "2026-10-01T12:00:00Z", "plannable": {"title": "Guest lecture"},
            "html_url": "/courses/101/calendar_events/2"}),
        );
        let event = planner_item(ids(), &base(), &lecture, map, now).unwrap();
        assert_eq!(event.kind, EventKind::ClassEvent);
        assert_eq!(event.course_id.as_deref(), Some("ours/101"));
        let assignment = item(json!({"plannable_id": 3, "plannable_type": "assignment",
            "plannable_date": "2026-10-01T12:00:00Z"}));
        assert!(planner_item(ids(), &base(), &assignment, map, now).is_none());
        let other_course = item(
            json!({"plannable_id": 4, "plannable_type": "wiki_page", "course_id": 999,
            "plannable_date": "2026-10-01T12:00:00Z"}),
        );
        assert!(planner_item(ids(), &base(), &other_course, map, now).is_none());
    }

    #[test]
    fn links_and_urls_are_http_only() {
        let item: json::ModuleItem = from(
            json!({"id": 5, "type": "ExternalUrl", "title": "Week 2 video",
            "external_url": "https://video.example.edu/w2"}),
        );
        let m = link(ids(), "c", &item, &Placement::default()).unwrap();
        assert_eq!((m.kind, m.week_hint), (MaterialKind::ExternalLink, Some(2)));
        let js: json::ModuleItem =
            from(json!({"id": 6, "type": "ExternalUrl", "external_url": "javascript:alert(1)"}));
        assert!(link(ids(), "c", &js, &Placement::default()).is_none());
        assert_eq!(absolute_url(&base(), "javascript:alert(1)"), None);
    }
}
