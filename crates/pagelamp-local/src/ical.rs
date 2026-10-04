//! iCalendar feed source (see the crate docs): fetch → parse VEVENTs → `Event`s.
//!
//! The feed URL is a SECRET (Canvas feed URLs embed a private token): it is never logged and
//! never appears in an error message — reqwest errors are stripped of their URL.
//! RRULE recurrences are not expanded (v0.1): a recurring event contributes its first
//! occurrence only, which is enough for deadlines (Canvas emits one VEVENT per due date).

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use icalendar::{CalendarDateTime, Component, DatePerhapsTime};
use pagelamp_core::Store;
use pagelamp_core::ingest::sha256_hex;
use pagelamp_core::model::{Course, Event, EventKind, course_for_hint};
use pagelamp_core::source::{ProgressFn, SourceError, SyncProgress, SyncStage};
use regex::Regex;

use crate::IcalSyncReport;

/// Largest feed we accept.
pub(crate) const MAX_FEED_BYTES: usize = 10 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Canvas appends the course in brackets: "Assignment 1 [DEMO101 F LEC0101]".
static BRACKET_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\[([^\[\]]+)\]\s*$").expect("valid regex"));
static QUIZ: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(quiz|quizzes|test)\b").expect("valid regex"));
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(assignments?|due|homework|hw\d*|problem sets?|submissions?|submit)\b")
        .expect("valid regex")
});
static EXAM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(exams?|midterms?|finals?)\b").expect("valid regex"));

/// `webcal://` → `https://`; https required except http for localhost (tests). Errors are
/// `Other` with a message that never repeats the URL.
pub(crate) fn normalize_feed_url(input: &str) -> Result<String, SourceError> {
    let input = input.trim();
    let rewritten = match input.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("webcal") => format!("https://{rest}"),
        _ => input.to_string(),
    };
    let invalid = || {
        SourceError::other(
            "That is not a valid calendar feed URL (it should start with https:// or webcal://).",
        )
    };
    let url = url::Url::parse(&rewritten).map_err(|_| invalid())?;
    let local = is_loopback(&url);
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        _ => return Err(invalid()),
    }
    if url.host_str().is_none_or(str::is_empty) {
        return Err(invalid());
    }
    Ok(url.to_string())
}

/// Redirects to follow at most.
const MAX_REDIRECTS: usize = 5;

/// Why a redirect of the feed was not followed (carried inside reqwest's error, see
/// `request_error`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RedirectRefusal {
    TooMany,
    Insecure,
}

impl std::fmt::Display for RedirectRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            RedirectRefusal::TooMany => "too many redirects",
            RedirectRefusal::Insecure => "redirect to an insecure address",
        })
    }
}

impl std::error::Error for RedirectRefusal {}

/// Only this computer may be reached over plain http (tests); exact host names only.
fn is_loopback(url: &url::Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

/// Whether the feed may be redirected to `next`, given the URLs requested so far (the
/// original first): at most `MAX_REDIRECTS` redirects, always to https — except from plain
/// http on this computer to plain http on this computer (tests), so https never steps down.
pub(crate) fn redirect_refusal(next: &url::Url, previous: &[url::Url]) -> Option<RedirectRefusal> {
    if previous.len() > MAX_REDIRECTS {
        return Some(RedirectRefusal::TooMany);
    }
    let from_local_http = previous
        .last()
        .is_some_and(|p| p.scheme() == "http" && is_loopback(p));
    match next.scheme() {
        "https" => None,
        "http" if is_loopback(next) && from_local_http => None,
        _ => Some(RedirectRefusal::Insecure),
    }
}

/// Follow redirects per `redirect_refusal`. The feed URL is a secret, so no `Referer` is ever
/// sent either (reqwest would copy the full previous URL into it; see `fetch_ical`).
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        match redirect_refusal(attempt.url(), attempt.previous()) {
            None => attempt.follow(),
            Some(refusal) => attempt.error(refusal),
        }
    })
}

pub(crate) async fn fetch_ical(feed_url: &str) -> Result<String, SourceError> {
    let url = normalize_feed_url(feed_url)?;
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .redirect(redirect_policy())
        .referer(false)
        .user_agent(format!(
            "{}/{}",
            pagelamp_core::brand::PRODUCT_NAME,
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|err| {
            SourceError::other(format!(
                "could not start an HTTP client: {}",
                err.without_url()
            ))
        })?;
    let mut response = client.get(&url).send().await.map_err(request_error)?;
    let status = response.status();
    match status.as_u16() {
        200..=299 => {}
        401 | 403 => {
            return Err(SourceError::auth(
                "The calendar feed URL was rejected (it may have been reset). Copy a fresh feed URL and update it.",
            ));
        }
        404 | 410 => {
            return Err(SourceError::not_found(
                "The calendar feed no longer exists. Copy a fresh feed URL and update it.",
            ));
        }
        // The server is having trouble: it may pass by itself, like a network failure.
        code @ 500..=599 => {
            return Err(SourceError::network(format!(
                "The calendar server answered HTTP {code}."
            )));
        }
        code => {
            return Err(SourceError::other(format!(
                "The calendar server answered HTTP {code}."
            )));
        }
    }
    let too_large = || {
        SourceError::other(format!(
            "The calendar feed is larger than {} MB.",
            MAX_FEED_BYTES / (1024 * 1024)
        ))
    };
    if response
        .content_length()
        .is_some_and(|len| len > MAX_FEED_BYTES as u64)
    {
        return Err(too_large());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if body.len() + chunk.len() > MAX_FEED_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8_lossy(&body).into_owned();
    if !text.to_ascii_uppercase().contains("BEGIN:VCALENDAR") {
        return Err(SourceError::other(
            "This URL did not return a calendar (iCalendar) feed.",
        ));
    }
    Ok(text)
}

/// Classify a reqwest failure without ever including the URL.
fn request_error(err: reqwest::Error) -> SourceError {
    // A refused redirect: fixed texts (the error would name the URL), and the entered URL
    // is unusable rather than the server unreachable.
    let mut source = std::error::Error::source(&err);
    while let Some(inner) = source {
        if let Some(refusal) = inner.downcast_ref::<RedirectRefusal>() {
            return SourceError::invalid_input(match refusal {
                RedirectRefusal::TooMany => "The calendar feed redirected too many times.",
                RedirectRefusal::Insecure => {
                    "The calendar feed redirected to an insecure (http) address."
                }
            });
        }
        source = inner.source();
    }
    let network = err.is_timeout() || err.is_connect() || err.is_request();
    let detail = err.without_url().to_string();
    if network {
        SourceError::network(format!("Could not reach the calendar server ({detail})."))
    } else {
        SourceError::other(format!("Downloading the calendar feed failed ({detail})."))
    }
}

pub(crate) async fn sync_ical(
    db_path: &Path,
    source_id: &str,
    feed_url: &str,
    progress: ProgressFn<'_>,
) -> Result<IcalSyncReport, SourceError> {
    progress(SyncProgress::Step {
        message: "Downloading the calendar feed".into(),
        current: None,
        total: None,
        stage: Some(SyncStage::DownloadingFeed),
        course: None,
    });
    let ics = fetch_ical(feed_url).await?;
    let db = db_path.to_path_buf();
    let known = blocking(move || Ok(Store::open(&db)?.list_courses(true)?)).await?;
    let parsed = parse_with(source_id, &ics, &known, &Local)?;
    let mut report = IcalSyncReport {
        events: parsed.events.len(),
        matched_to_courses: parsed
            .events
            .iter()
            .filter(|e| e.course_id.is_some())
            .count(),
        warnings: Vec::new(),
    };
    if parsed.skipped_without_date > 0 {
        let message = format!(
            "{} calendar entries without a date were skipped",
            parsed.skipped_without_date
        );
        progress(SyncProgress::Warning(message.clone()));
        report.warnings.push(message);
    }
    progress(SyncProgress::Step {
        message: format!("Saving {} calendar events", parsed.events.len()),
        current: None,
        total: Some(u32::try_from(parsed.events.len()).unwrap_or(u32::MAX)),
        stage: Some(SyncStage::SavingEvents),
        course: None,
    });
    let (db, source) = (db_path.to_path_buf(), source_id.to_string());
    blocking(move || Ok(Store::open(&db)?.replace_events(&source, &parsed.events)?)).await?;
    Ok(report)
}

/// Run store work on the blocking pool (never hold a `Store` across `.await`).
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, SourceError> + Send + 'static,
) -> Result<T, SourceError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| SourceError::other(format!("calendar sync crashed: {err}")))?
}

/// Events parsed from a feed plus what was skipped.
pub(crate) struct Parsed {
    pub events: Vec<Event>,
    pub skipped_without_date: usize,
}

pub(crate) fn parse_ical(
    source_id: &str,
    ics: &str,
    known_courses: &[Course],
) -> Result<Vec<Event>, SourceError> {
    Ok(parse_with(source_id, ics, known_courses, &Local)?.events)
}

/// `parse_ical` with an explicit time zone for all-day and floating times (tests pass a
/// fixed offset so results don't depend on the machine).
pub(crate) fn parse_with<Tz: TimeZone>(
    source_id: &str,
    ics: &str,
    known_courses: &[Course],
    tz: &Tz,
) -> Result<Parsed, SourceError> {
    let calendar: icalendar::Calendar = ics.parse().map_err(|err| {
        SourceError::other(format!("The calendar feed could not be read ({err})."))
    })?;
    let now = Utc::now();
    let mut parsed = Parsed {
        events: Vec::new(),
        skipped_without_date: 0,
    };
    for event in calendar.events() {
        let raw_summary = event.get_summary().unwrap_or("").trim().to_string();
        let (title, bracket) = split_bracket(&raw_summary);
        let start = event.get_start();
        let end = event.get_end();
        let (starts_at, ends_at, due_at) = match (&start, &end) {
            (None, _) => {
                parsed.skipped_without_date += 1;
                continue;
            }
            (Some(DatePerhapsTime::Date(day)), _) => (None, None, Some(end_of_day(*day, tz))),
            (Some(start), end) => {
                let start_at = instant(start, tz);
                let end_at = end.as_ref().and_then(|e| instant(e, tz));
                match (start_at, end_at) {
                    (None, _) => {
                        parsed.skipped_without_date += 1;
                        continue;
                    }
                    // Canvas assignment events: DTSTART == DTEND (or no DTEND) → a deadline.
                    (Some(s), Some(e)) if s == e => (None, None, Some(s)),
                    (Some(s), None) => (None, None, Some(s)),
                    (Some(s), Some(e)) => (Some(s), Some(e), None),
                }
            }
        };
        let kind = classify(&raw_summary, event.get_description().unwrap_or(""));
        let id_part = match event.get_uid() {
            Some(uid) if !uid.trim().is_empty() => {
                // Overrides of a recurring event share the UID; keep them apart.
                match event.property_value("RECURRENCE-ID") {
                    Some(rid) => format!("{}#{rid}", uid.trim()),
                    None => uid.trim().to_string(),
                }
            }
            _ => {
                let dtstart = event.property_value("DTSTART").unwrap_or("");
                sha256_hex(format!("{raw_summary}\n{dtstart}").as_bytes())[..16].to_string()
            }
        };
        parsed.events.push(Event {
            id: format!("{source_id}/event/{id_part}"),
            source_id: source_id.to_string(),
            course_id: bracket
                .and_then(|b| course_for_hint(b, known_courses))
                .map(|c| c.id.clone()),
            kind,
            title: if title.is_empty() {
                "(untitled)".into()
            } else {
                title
            },
            starts_at,
            ends_at,
            due_at,
            url: event.get_url().map(str::to_string),
            updated_at: now,
            course_hint: bracket.map(str::to_string),
        });
    }
    Ok(parsed)
}

/// "Title [DEMO101 F LEC0101]" → ("Title", Some("DEMO101 F LEC0101")).
fn split_bracket(summary: &str) -> (String, Option<&str>) {
    match BRACKET_SUFFIX.captures(summary) {
        Some(captures) => {
            let whole = captures.get(0).expect("group 0");
            let inner = captures.get(1).expect("group 1").as_str();
            (summary[..whole.start()].trim().to_string(), Some(inner))
        }
        None => (summary.to_string(), None),
    }
}

/// Quiz → QuizDue; assignment/due/homework/submission → AssignmentDue; exam/midterm/final →
/// Exam; else ClassEvent. The description is only used for this classification — it is
/// never stored (docs/ARCHITECTURE.md §3 rule 4).
fn classify(summary: &str, description: &str) -> EventKind {
    if QUIZ.is_match(summary) {
        EventKind::QuizDue
    } else if ASSIGNMENT.is_match(summary) {
        EventKind::AssignmentDue
    } else if EXAM.is_match(summary) {
        EventKind::Exam
    } else if QUIZ.is_match(description) {
        EventKind::QuizDue
    } else if ASSIGNMENT.is_match(description) {
        EventKind::AssignmentDue
    } else if EXAM.is_match(description) {
        EventKind::Exam
    } else {
        EventKind::ClassEvent
    }
}

/// UTC instant of a DTSTART/DTEND value. Floating times and unknown TZIDs are read as `tz`.
fn instant<Tz: TimeZone>(value: &DatePerhapsTime, tz: &Tz) -> Option<DateTime<Utc>> {
    match value {
        DatePerhapsTime::Date(day) => local_to_utc(day.and_time(NaiveTime::MIN), tz),
        DatePerhapsTime::DateTime(date_time) => match date_time {
            CalendarDateTime::Utc(utc) => Some(*utc),
            CalendarDateTime::Floating(naive) => local_to_utc(*naive, tz),
            CalendarDateTime::WithTimezone {
                date_time: naive, ..
            } => date_time
                .try_into_utc()
                .or_else(|| local_to_utc(*naive, tz)),
        },
    }
}

/// End of `day` (23:59:59) in `tz`, as UTC.
fn end_of_day<Tz: TimeZone>(day: NaiveDate, tz: &Tz) -> DateTime<Utc> {
    let naive = day.and_hms_opt(23, 59, 59).expect("valid time");
    local_to_utc(naive, tz).unwrap_or_else(|| naive.and_utc())
}

/// Local wall-clock time → UTC. In a DST gap there is no such local time: use the time one
/// hour later; in a DST overlap the earlier instant is used.
fn local_to_utc<Tz: TimeZone>(naive: NaiveDateTime, tz: &Tz) -> Option<DateTime<Utc>> {
    tz.from_local_datetime(&naive)
        .earliest()
        .or_else(|| {
            tz.from_local_datetime(&(naive + chrono::TimeDelta::hours(1)))
                .earliest()
        })
        .map(|dt| dt.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;
    use pagelamp_core::model::AiPolicy;

    use super::*;

    const SOURCE: &str = "ical:demo";

    fn course(code: &str) -> Course {
        Course {
            id: format!("folder:demo/course/{code}"),
            source_id: "folder:demo".into(),
            external_id: code.into(),
            code: Some(code.into()),
            name: format!("{code} course"),
            term_start: None,
            term_end: None,
            term_source: pagelamp_core::model::TermSource::None,
            url: None,
            ai_policy: AiPolicy::Unknown,
            ai_policy_note: None,
            ai_access: true,
            enrollment_active: true,
            hidden: false,
            updated_at: Utc::now(),
        }
    }

    /// Toronto in September (UTC-4) as a fixed offset.
    fn toronto() -> FixedOffset {
        FixedOffset::west_opt(4 * 3600).unwrap()
    }

    fn ics(events: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Demo//Demo//EN\r\n{events}END:VCALENDAR\r\n"
        )
    }

    fn parse(events: &str, courses: &[Course]) -> Parsed {
        parse_with(SOURCE, &ics(events), courses, &toronto()).unwrap()
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn canvas_assignment_instant_is_a_deadline_matched_to_its_course() {
        let courses = [course("DEMO101"), course("DEMO1011"), course("DEMO202")];
        let parsed = parse(
            "BEGIN:VEVENT\r\nUID:event-assignment-1\r\nDTSTART:20260930T035900Z\r\n\
             DTEND:20260930T035900Z\r\nSUMMARY:Problem Set 1 [DEMO101 F LEC0101]\r\n\
             URL:https://lms.example.edu/courses/1/assignments/9\r\n\
             DESCRIPTION:Secret instructions that must never be stored\r\nEND:VEVENT\r\n",
            &courses,
        );
        let event = &parsed.events[0];
        assert_eq!(event.id, "ical:demo/event/event-assignment-1");
        assert_eq!(event.title, "Problem Set 1");
        assert_eq!(event.kind, EventKind::AssignmentDue);
        assert_eq!(event.due_at, Some(at("2026-09-30T03:59:00Z")));
        assert_eq!(event.starts_at, None);
        assert_eq!(
            event.course_id.as_deref(),
            Some("folder:demo/course/DEMO101")
        );
        assert_eq!(
            event.url.as_deref(),
            Some("https://lms.example.edu/courses/1/assignments/9")
        );
        let json = serde_json::to_string(event).unwrap();
        assert!(!json.contains("Secret instructions"));
    }

    #[test]
    fn longest_matching_code_wins_and_unknown_codes_stay_unmatched() {
        let courses = [course("DEMO101"), course("DEMO1011")];
        assert_eq!(
            course_for_hint("DEMO1011 S LEC0101", &courses).map(|c| c.id.as_str()),
            Some("folder:demo/course/DEMO1011")
        );
        assert_eq!(
            course_for_hint("demo 101 f", &courses).map(|c| c.id.as_str()),
            Some("folder:demo/course/DEMO101")
        );
        assert!(course_for_hint("OTHER999", &courses).is_none());
    }

    #[test]
    fn all_day_events_are_due_at_the_end_of_the_local_day() {
        let parsed = parse(
            "BEGIN:VEVENT\r\nUID:all-day\r\nDTSTART;VALUE=DATE:20261015\r\n\
             DTEND;VALUE=DATE:20261016\r\nSUMMARY:Midterm exam\r\nEND:VEVENT\r\n",
            &[],
        );
        let event = &parsed.events[0];
        assert_eq!(event.kind, EventKind::Exam);
        // 23:59:59 at UTC-4 = 03:59:59 UTC next day.
        assert_eq!(event.due_at, Some(at("2026-10-16T03:59:59Z")));
    }

    #[test]
    fn timed_events_with_tzid_floating_and_ranges() {
        let parsed = parse(
            "BEGIN:VEVENT\r\nUID:lecture\r\nDTSTART;TZID=America/Toronto:20261001T100000\r\n\
             DTEND;TZID=America/Toronto:20261001T113000\r\nSUMMARY:Lecture\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:floating\r\nDTSTART:20261002T090000\r\n\
             SUMMARY:Office hours\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:unknown-zone\r\nDTSTART;TZID=Mars/Olympus:20261003T090000\r\n\
             DTEND;TZID=Mars/Olympus:20261003T100000\r\nSUMMARY:Seminar\r\nEND:VEVENT\r\n",
            &[],
        );
        let by_id = |id: &str| {
            parsed
                .events
                .iter()
                .find(|e| e.id.ends_with(id))
                .unwrap()
                .clone()
        };
        let lecture = by_id("lecture");
        assert_eq!(lecture.kind, EventKind::ClassEvent);
        assert_eq!(lecture.starts_at, Some(at("2026-10-01T14:00:00Z")));
        assert_eq!(lecture.ends_at, Some(at("2026-10-01T15:30:00Z")));
        assert_eq!(lecture.due_at, None);
        // No DTEND: a point in time (read in the given zone).
        assert_eq!(by_id("floating").due_at, Some(at("2026-10-02T13:00:00Z")));
        // Unknown TZID falls back to the given zone instead of failing.
        assert_eq!(
            by_id("unknown-zone").starts_at,
            Some(at("2026-10-03T13:00:00Z"))
        );
    }

    #[test]
    fn missing_uid_gets_a_stable_hash_id_and_undated_events_are_skipped() {
        let events = "BEGIN:VEVENT\r\nDTSTART:20261001T100000Z\r\nSUMMARY:Quiz 2\r\nEND:VEVENT\r\n\
                      BEGIN:VEVENT\r\nUID:no-date\r\nSUMMARY:Someday\r\nEND:VEVENT\r\n";
        let first = parse(events, &[]);
        let second = parse(events, &[]);
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.skipped_without_date, 1);
        assert_eq!(first.events[0].id, second.events[0].id);
        assert!(first.events[0].id.starts_with("ical:demo/event/"));
        assert_eq!(first.events[0].kind, EventKind::QuizDue);
    }

    #[test]
    fn classification_prefers_the_summary() {
        assert_eq!(classify("Final project due", ""), EventKind::AssignmentDue);
        assert_eq!(classify("Final exam", ""), EventKind::Exam);
        assert_eq!(classify("Quiz 3", ""), EventKind::QuizDue);
        assert_eq!(classify("Tutorial", ""), EventKind::ClassEvent);
        assert_eq!(
            classify("Week 5 check-in", "submit your reflection"),
            EventKind::AssignmentDue
        );
        assert_eq!(
            classify("Lab 2", "latest submission"),
            EventKind::AssignmentDue
        );
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(parse_with(SOURCE, "not a calendar", &[], &toronto()).is_err());
        let parsed = parse_with(SOURCE, &ics(""), &[], &toronto()).unwrap();
        assert!(parsed.events.is_empty());
    }

    #[test]
    fn redirects_stay_on_https_and_stop_after_five() {
        let url = |text: &str| url::Url::parse(text).unwrap();
        let https = [url("https://calendar.example.edu/feed.ics")];
        let local_http = [url("http://127.0.0.1:8080/feed.ics")];
        let allowed =
            |next: &str, previous: &[url::Url]| redirect_refusal(&url(next), previous).is_none();
        assert!(allowed("https://cdn.example.net/x.ics", &https));
        assert!(allowed("https://cdn.example.net/x.ics", &local_http));
        assert!(allowed("http://localhost:9/x.ics", &local_http));
        assert!(allowed("http://[::1]:9/x.ics", &local_http));
        for (next, previous) in [
            ("http://calendar.example.edu/x.ics", &https[..]),
            ("http://127.0.0.1:9/x.ics", &https[..]), // https never steps down
            ("http://calendar.example.edu/x.ics", &local_http[..]),
            ("http://127.0.0.2/x.ics", &local_http[..]),
            ("http://0.0.0.0/x.ics", &local_http[..]),
            ("http://localhost.evil.com/x.ics", &local_http[..]),
            ("webcal://calendar.example.edu/x.ics", &https[..]),
            ("ftp://calendar.example.edu/x.ics", &https[..]),
            ("file:///etc/passwd", &https[..]),
        ] {
            assert_eq!(
                redirect_refusal(&url(next), previous),
                Some(RedirectRefusal::Insecure),
                "{next}"
            );
        }
        let chain: Vec<url::Url> = (0..6)
            .map(|i| url(&format!("https://calendar.example.edu/{i}.ics")))
            .collect();
        assert!(
            allowed("https://calendar.example.edu/x.ics", &chain[..5]),
            "5th hop"
        );
        assert_eq!(
            redirect_refusal(&url("https://calendar.example.edu/x.ics"), &chain),
            Some(RedirectRefusal::TooMany),
            "6th hop"
        );
    }

    #[test]
    fn feed_urls_are_normalised_and_never_echoed_in_errors() {
        assert_eq!(
            normalize_feed_url(" webcal://calendar.example.edu/feed.ics?token=abc ").unwrap(),
            "https://calendar.example.edu/feed.ics?token=abc"
        );
        assert!(normalize_feed_url("http://127.0.0.1:8080/feed.ics").is_ok());
        for bad in [
            "http://calendar.example.edu/feed?token=abc",
            "ftp://x.example.edu/f",
            "nonsense token=abc",
            "",
        ] {
            let err = normalize_feed_url(bad).unwrap_err();
            assert!(!err.message.contains("token=abc"), "{}", err.message);
        }
    }
}
