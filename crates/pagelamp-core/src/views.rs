//! Read views shared by the App facade (desktop app, CLI) and the MCP server.
//!
//! Every function takes an open `Store` (read-only is enough) plus an `AsOf` ("now" and the
//! student's local calendar date) so results are deterministic in tests. Field names are part
//! of both the frontend contract (via `pagelamp schema`) and the MCP tool output — rename
//! with care and tell the frontend.
//!
//! Views return raw data. Presentation concerns that only MCP needs (the
//! `<course_material>` wrappers, the `guidance` string, output caps in characters) live in
//! `pagelamp-mcp`.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Local, NaiveDate, TimeDelta, Utc};
use pagelamp_extract::FailureKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::auto_sync::{self, AutoSync, LightSync};
use crate::dates::{Tz, course_date, time_zone};
use crate::lifecycle::{self, LifecycleInput};
use crate::model::*;
use crate::store::Store;
use crate::term::phase::teaching_week_on;
use crate::term::{CONFIRMED_DATES_KEY, ResolvedTerm, TermInput, resolve_term};
use crate::timeline;
use crate::{Error, Result};

mod digest;

pub use digest::{DigestCourse, DigestPlan, WeeklyDigest, weekly_digest};

/// Window for "recent" materials/announcements in overviews.
pub const RECENT_DAYS: u32 = 14;
/// Window for "upcoming" deadlines in overviews and course summaries.
pub const UPCOMING_DAYS: u32 = 21;
/// A source whose last successful sync is older than this is reported as stale, unless the
/// automatic sync's interval asks for longer (`AutoSync::stale_after`).
pub const STALE_AFTER_HOURS: i64 = 24;

/// The moment a view is computed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AsOf {
    pub now: Timestamp,
    /// The student's local calendar date (drives timeline/week inference).
    pub today: NaiveDate,
    /// The machine's time zone: the dates of courses without one of their own (folder
    /// courses) are taken in it, so they agree with `today`. None → UTC.
    pub tz: Option<Tz>,
}

impl AsOf {
    /// Current instant; `today` and `tz` in the machine's time zone.
    ///
    /// Both come from the operating system's zone setting (`iana-time-zone`: CoreFoundation on
    /// macOS, `/etc/localtime` on Linux, the Windows API), never from a `TZ` environment
    /// variable, so the desktop app and every `pagelamp mcp` an AI app starts (possibly with
    /// its own environment) compute the same `today` and the same course dates. Only when the
    /// system zone can't be determined does `today` fall back to chrono's `Local`.
    pub fn now_local() -> Self {
        let now: DateTime<Utc> = Utc::now();
        let tz = local_time_zone();
        let today = match tz {
            Some(tz) => now.with_timezone(&tz).date_naive(),
            None => now.with_timezone(&Local).date_naive(),
        };
        AsOf { now, today, tz }
    }

    /// A fixed moment (tests, previews): `today` as given, UTC for courses without a zone.
    pub fn at(now: Timestamp, today: NaiveDate) -> Self {
        AsOf {
            now,
            today,
            tz: None,
        }
    }
}

/// The operating system's IANA time zone, if it can be determined (see `AsOf::now_local`).
fn local_time_zone() -> Option<Tz> {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| time_zone(&name))
}

/// A deadline/event enriched with its course's code and name (all `Event` fields are
/// flattened into the same JSON object).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Deadline {
    #[serde(flatten)]
    pub event: Event,
    pub course_code: Option<String>,
    pub course_name: Option<String>,
}

/// Per-course counts shown in course lists.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct CourseCounts {
    pub modules: u32,
    pub materials: u32,
    /// Materials whose text the student's AI app can read: indexed materials, but 0 unless
    /// the course's `ai_materials` state is `readable`.
    pub indexed_materials: u32,
    /// Deadlines due in the next `UPCOMING_DAYS` days.
    pub upcoming_deadlines: u32,
}

/// One row of the course list ("where is each course this week").
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CourseSummary {
    pub course: Course,
    /// Effective AI access to this course's material text (`Course::ai_materials`).
    pub ai_materials: AiMaterialsState,
    pub timeline: CourseTimeline,
    /// Upcoming / current / finishing / ended, the Past group and removal suggestions.
    pub lifecycle: CourseLifecycle,
    pub counts: CourseCounts,
    pub next_deadline: Option<Deadline>,
    /// Label of the source this course came from (e.g. "Quercus", "~/Courses").
    pub source_label: String,
    /// When that source last synced in full (the freshness of the course's modules and
    /// materials).
    pub last_synced_at: Option<Timestamp>,
    /// When the source's deadlines and announcements were last read: `last_synced_at`, or a
    /// later automatic sync that read only those (`auto_sync::LightSync`).
    pub deadlines_synced_at: Option<Timestamp>,
    /// An automatic sync found this course, and no full sync has read it yet: it is listed
    /// with its deadlines and announcements, but its modules and materials are still missing
    /// (not empty). The next full sync reads them.
    pub structure_pending: bool,
}

/// A material as listed in views (no text; use `read_material` for text).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MaterialView {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub kind: MaterialKind,
    pub module_id: Option<String>,
    pub module_name: Option<String>,
    pub week_hint: Option<u32>,
    pub published_at: Option<Timestamp>,
    /// Where the student can open the original (LMS URL or `file://` URL).
    pub url: Option<String>,
    pub text_status: TextStatus,
    pub text_error: Option<String>,
    /// Why a `NotDownloaded` file cannot be downloaded on request (locked in the LMS, too
    /// large); `None` when a download may work.
    pub download_blocked: Option<DownloadBlock>,
    /// Number of text chunks (pages/slides/sections) available via `read_material`.
    pub chunk_count: u32,
    /// Why there is no text to read, in a word the app can show; `None` when the text is
    /// there or was never tried (`pending`, `unsupported`, `not_downloaded`).
    pub text_problem: Option<TextProblem>,
}

/// Why a material's text can't be read (`MaterialView::text_problem`). It is worked out when
/// the view is built, from `text_status`, the chunk count, `text_error_kind` and the
/// extractor's message, so it also covers failures that earlier versions recorded.
/// `text_error` keeps the full message for the CLI, logs and MCP.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextProblem {
    /// Read, but there is no text in it (e.g. a scanned PDF).
    NoText,
    /// Over a size, page or part limit.
    TooLarge,
    /// A PDF that needs a password to open.
    PasswordProtected,
    /// Damaged, not the format its name says, or something the reader can't handle.
    Malformed,
    /// The extraction worker was still running at its time limit.
    TimedOut,
    /// The extraction worker used up its CPU-time budget.
    CpuLimit,
    /// The extraction worker went past its memory cap.
    MemoryLimit,
    /// The extraction worker died without an answer.
    Crashed,
    /// The extraction worker answered something unreadable.
    BadOutput,
    /// The extraction worker could not be started (e.g. blocked by antivirus).
    SpawnFailed,
    /// An extraction worker from another version answered.
    ProtocolMismatch,
    /// Any other failure; `text_error` says what.
    Other,
}

impl TextProblem {
    /// The problem with `material`'s text, which has `chunk_count` chunks.
    pub fn of(material: &Material, chunk_count: u32) -> Option<TextProblem> {
        match material.text_status {
            TextStatus::Ok if chunk_count == 0 => Some(TextProblem::NoText),
            TextStatus::Error => Some(match material.text_error_kind {
                Some(kind) => TextProblem::from(kind),
                None => match material
                    .text_error
                    .as_deref()
                    .and_then(pagelamp_extract::failure_kind)
                {
                    Some(FailureKind::TooLarge) => TextProblem::TooLarge,
                    Some(FailureKind::PasswordProtected) => TextProblem::PasswordProtected,
                    Some(FailureKind::Malformed) => TextProblem::Malformed,
                    None => TextProblem::Other,
                },
            }),
            TextStatus::Ok
            | TextStatus::Pending
            | TextStatus::Unsupported
            | TextStatus::NotDownloaded => None,
        }
    }
}

impl From<TextErrorKind> for TextProblem {
    fn from(kind: TextErrorKind) -> Self {
        match kind {
            TextErrorKind::TimedOut => TextProblem::TimedOut,
            TextErrorKind::CpuLimit => TextProblem::CpuLimit,
            TextErrorKind::MemoryLimit => TextProblem::MemoryLimit,
            TextErrorKind::Crashed => TextProblem::Crashed,
            TextErrorKind::BadOutput => TextProblem::BadOutput,
            TextErrorKind::SpawnFailed => TextProblem::SpawnFailed,
            TextErrorKind::ProtocolMismatch => TextProblem::ProtocolMismatch,
        }
    }
}

/// Everything needed to answer "what's going on in this course right now".
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CourseOverview {
    pub course: Course,
    /// Effective AI access to this course's material text (`Course::ai_materials`).
    pub ai_materials: AiMaterialsState,
    pub timeline: CourseTimeline,
    pub lifecycle: CourseLifecycle,
    pub current_modules: Vec<Module>,
    /// Materials published in the last `RECENT_DAYS` days, newest first (announcements excluded).
    pub recent_materials: Vec<MaterialView>,
    /// Deadlines due in the next `UPCOMING_DAYS` days, soonest first.
    pub upcoming_deadlines: Vec<Deadline>,
    /// Announcements posted in the last `RECENT_DAYS` days, newest first (titles + ids only).
    pub recent_announcements: Vec<MaterialView>,
    pub source_label: String,
    /// When the course's source last synced in full (modules and materials).
    pub last_synced_at: Option<Timestamp>,
    /// Files a "download this course's files" action would fetch: kind `file`, text status
    /// `not_downloaded` and no `download_blocked` reason (all weeks).
    pub downloadable_files: u32,
    /// When its deadlines and announcements were last read (`CourseSummary`).
    pub deadlines_synced_at: Option<Timestamp>,
    /// Its modules and materials haven't been read yet (`CourseSummary`).
    pub structure_pending: bool,
}

/// Materials of one teaching week.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct WeekMaterials {
    pub course: Course,
    /// Effective AI access to this course's material text (`Course::ai_materials`).
    pub ai_materials: AiMaterialsState,
    /// The week actually shown (None when no week could be determined; see `note`).
    pub week: Option<u32>,
    /// The week the caller asked for (None = "current week").
    pub requested_week: Option<u32>,
    pub timeline: CourseTimeline,
    /// Modules belonging to this week.
    pub modules: Vec<Module>,
    pub materials: Vec<MaterialView>,
    /// Every week that has at least one module or material (plus the current week), ascending —
    /// drives the ‹ Week › switcher.
    pub available_weeks: Vec<u32>,
    /// Explains any fallback, e.g. "current week unknown — showing materials of the last 14 days".
    pub note: Option<String>,
    /// Machine-readable class of `note` (UIs localise from this; `note` stays English).
    pub note_kind: Option<WeekNoteKind>,
}

/// Why `WeekMaterials.note` is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WeekNoteKind {
    /// No week requested and the current week could not be inferred — showing the materials
    /// of the last `RECENT_DAYS` days instead (`week` is None).
    CurrentWeekUnknown,
    /// Today is before the term start or after the term end; or the course is over, inactive
    /// or not started by its lifecycle, whatever its dates (`note` then says so).
    OutsideTerm,
    /// The week is known but has no modules or materials.
    NoMaterialsThisWeek,
    /// No week requested and the course is in its exam period (no teaching week): showing the
    /// materials of the last `RECENT_DAYS` days (`week` is None).
    ExamPeriod,
    /// No week requested and the course is in a break: showing the week before it.
    Break,
}

/// A window of a material's text chunks (pagination via `next_chunk`).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MaterialText {
    pub material: MaterialView,
    pub course_code: Option<String>,
    /// Effective AI access for the material's course. When not `readable`, `chunks` is empty
    /// (docs/ARCHITECTURE.md §3 rule 8).
    pub ai_materials: AiMaterialsState,
    pub chunks: Vec<Chunk>,
    pub from_chunk: u32,
    /// Pass as `from_chunk` to continue; None when the end was reached.
    pub next_chunk: Option<u32>,
    pub total_chunks: u32,
    /// True when the single chunk returned was longer than the limit and was cut.
    pub truncated: bool,
}

/// An announcement with its text.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Announcement {
    pub material: MaterialView,
    /// Effective AI access for the course. When not `readable`, `text` is empty.
    pub ai_materials: AiMaterialsState,
    pub text: String,
    /// True when `text` was cut to the caller's limit.
    pub truncated: bool,
}

/// Search results for an AI client: only courses whose material text is readable
/// (docs/ARCHITECTURE.md §3 rule 8).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AiSearchResults {
    pub hits: Vec<SearchHit>,
    /// State of the requested course (only when a course filter was given). When it is not
    /// `readable`, `hits` is empty.
    pub course_ai_materials: Option<AiMaterialsState>,
    /// Codes (or names, when a course has no code) of visible courses left out because their
    /// material text is not readable (only when no course filter was given).
    pub excluded_courses: Vec<String>,
}

/// A source with a freshness verdict.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SourceStatus {
    #[serde(flatten)]
    pub source: SourceRecord,
    /// Never synced, last successful sync older than the stale threshold (24 hours, or more
    /// when the automatic sync's interval asks for it), or the last sync failed.
    pub stale: bool,
    /// When the source's deadlines and announcements were last read: its last full sync
    /// (`last_synced_at`, which is what `stale` is about), or a later automatic sync that read
    /// only those.
    pub deadlines_synced_at: Option<Timestamp>,
    /// `deadlines_synced_at` is older than the stale threshold too (or there is none). False
    /// with `stale` true: the modules and materials are old, the deadlines and announcements
    /// aren't.
    pub deadlines_stale: bool,
}

/// Data freshness overview (MCP `sync_status`, desktop status screen).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SyncStatus {
    pub sources: Vec<SourceStatus>,
    pub counts: StoreCounts,
    /// Most recent successful sync over all sources.
    pub last_synced_at: Option<Timestamp>,
    /// True when there are no sources or any source is stale / failing.
    pub stale: bool,
    /// How often PageLamp syncs by itself while it runs (`Off`: only when the student starts
    /// a sync).
    pub auto_sync: AutoSync,
    /// When an automatic sync last ended with every source it could sync synced.
    pub last_automatic_sync_at: Option<Timestamp>,
}

/// Bounds for `read_material`'s `max_chars` (smaller/larger requests are clamped).
pub const MIN_READ_CHARS: usize = 200;
pub const MAX_READ_CHARS: usize = 50_000;

// ---------------------------------------------------------------------------------------------
// View functions
// ---------------------------------------------------------------------------------------------

/// All courses (hidden ones included when `include_hidden`; the desktop app shows them with a
/// toggle, the MCP server never does), ordered like `Store::list_courses`.
pub fn list_courses(store: &Store, include_hidden: bool, at: AsOf) -> Result<Vec<CourseSummary>> {
    let sources = SourceIndex::load(store)?;
    let upcoming = store.list_events(at.now, add_days(at.now, UPCOMING_DAYS), None)?;
    let mut term_data = store.all_course_term_data()?;
    let confirmed = confirmed_courses(store)?;
    let mut summaries = Vec::new();
    for course in store.list_courses(include_hidden)? {
        let data = CourseData::load_with(
            store,
            &course,
            term_data.remove(&course.id).unwrap_or_default(),
            &confirmed,
        )?;
        let (_, timeline, lifecycle) = data.state(&course, at);
        let ai_materials = course.ai_materials();
        let course_deadlines: Vec<&Event> = upcoming
            .iter()
            .filter(|event| event.course_id.as_deref() == Some(course.id.as_str()))
            .collect();
        let indexed = data
            .materials
            .iter()
            .filter(|m| m.text_status == TextStatus::Ok && data.chunks_of(&m.id) > 0)
            .count();
        let counts = CourseCounts {
            modules: count_u32(data.modules.len()),
            materials: count_u32(data.materials.len()),
            indexed_materials: if ai_materials.is_readable() {
                count_u32(indexed)
            } else {
                0
            },
            upcoming_deadlines: count_u32(course_deadlines.len()),
        };
        let next_deadline = course_deadlines
            .first()
            .map(|event| deadline(event, Some(&course)));
        let synced = sources.info(&course);
        summaries.push(CourseSummary {
            ai_materials,
            timeline,
            lifecycle,
            counts,
            next_deadline,
            source_label: synced.label,
            last_synced_at: synced.last_synced_at,
            deadlines_synced_at: synced.deadlines_synced_at,
            structure_pending: synced.structure_pending,
            course,
        });
    }
    Ok(summaries)
}

/// Timeline of one course: its resolved dates, phase and week (see `timeline`), without a
/// week when the course's lifecycle leaves the week-based views (`CourseData::state`).
pub fn course_timeline(store: &Store, course: &Course, at: AsOf) -> Result<CourseTimeline> {
    Ok(CourseData::load(store, course)?.state(course, at).1)
}

/// "What's going on in this course right now". `course` is resolved with
/// `Store::resolve_course_with(course, include_hidden)`: the desktop app passes `true` (a
/// hidden course's page still works), the MCP server `false`.
pub fn course_overview(
    store: &Store,
    course: &str,
    include_hidden: bool,
    at: AsOf,
) -> Result<CourseOverview> {
    let course = store.resolve_course_with(course, include_hidden)?;
    let data = CourseData::load(store, &course)?;
    let (_, timeline, lifecycle) = data.state(&course, at);
    let current_modules = data
        .modules
        .iter()
        .filter(|module| timeline.current_module_ids.contains(&module.id))
        .cloned()
        .collect();
    let since = sub_days(at.now, RECENT_DAYS);
    let recent = |announcements: bool| -> Vec<MaterialView> {
        let mut recent: Vec<&Material> = data
            .materials
            .iter()
            .filter(|m| (m.kind == MaterialKind::Announcement) == announcements)
            .filter(|m| m.published_at.is_some_and(|p| p >= since && p <= at.now))
            .collect();
        recent.sort_by(|a, b| {
            b.published_at
                .cmp(&a.published_at)
                .then(a.title.cmp(&b.title))
        });
        recent.into_iter().map(|m| data.view(m)).collect()
    };
    let upcoming_deadlines = store
        .list_events(at.now, add_days(at.now, UPCOMING_DAYS), Some(&course.id))?
        .iter()
        .map(|event| deadline(event, Some(&course)))
        .collect();
    let synced = SourceIndex::load(store)?.info(&course);
    let downloadable_files = data
        .materials
        .iter()
        .filter(|m| {
            m.kind == MaterialKind::File
                && m.text_status == TextStatus::NotDownloaded
                && m.download_blocked.is_none()
        })
        .count();
    Ok(CourseOverview {
        ai_materials: course.ai_materials(),
        timeline,
        lifecycle,
        current_modules,
        recent_materials: recent(false),
        upcoming_deadlines,
        recent_announcements: recent(true),
        source_label: synced.label,
        last_synced_at: synced.last_synced_at,
        deadlines_synced_at: synced.deadlines_synced_at,
        structure_pending: synced.structure_pending,
        downloadable_files: u32::try_from(downloadable_files).unwrap_or(u32::MAX),
        course,
    })
}

/// Materials of `week` (default: the inferred current week). A material belongs to week N
/// when its `week_hint` is N; or it has no week_hint and its module's week_hint is N; or it
/// has neither and the course term start is known and it was published during week N.
/// Announcements are not week materials (see `course_overview` / `announcements`).
///
/// `course` is resolved with `include_hidden` like `course_overview`. When no week is given
/// and the current week is unknown, the materials of the last `RECENT_DAYS` days are shown
/// instead (`note_kind = CurrentWeekUnknown`; `OutsideTerm` for a course that is over, inactive
/// or hasn't started, which has no current week).
pub fn week_materials(
    store: &Store,
    course: &str,
    week: Option<u32>,
    include_hidden: bool,
    at: AsOf,
) -> Result<WeekMaterials> {
    let course = store.resolve_course_with(course, include_hidden)?;
    let data = CourseData::load(store, &course)?;
    let (resolved, timeline, lifecycle) = data.state(&course, at);
    let no_current_week = has_no_current_week(&timeline, &lifecycle);
    let content: Vec<&Material> = data
        .materials
        .iter()
        .filter(|m| m.kind != MaterialKind::Announcement)
        .collect();

    let mut weeks: BTreeSet<u32> = data.modules.iter().filter_map(|m| m.week_hint).collect();
    weeks.extend(content.iter().filter_map(|m| data.week_of(&resolved, m)));
    weeks.extend(timeline.current_week);
    weeks.extend(timeline.default_week);
    let available_weeks: Vec<u32> = weeks.into_iter().collect();

    let (shown_week, modules, materials, note_kind) = match week.or(timeline.default_week) {
        Some(n) => {
            let modules: Vec<Module> = data
                .modules
                .iter()
                .filter(|m| m.week_hint == Some(n))
                .cloned()
                .collect();
            let materials: Vec<MaterialView> = content
                .iter()
                .filter(|m| data.week_of(&resolved, m) == Some(n))
                .map(|m| data.view(m))
                .collect();
            let note_kind = if modules.is_empty() && materials.is_empty() {
                Some(WeekNoteKind::NoMaterialsThisWeek)
            } else if week.is_none() && timeline.phase == CoursePhase::Break {
                Some(WeekNoteKind::Break)
            } else if week.is_none() && timeline.outside_term {
                Some(WeekNoteKind::OutsideTerm)
            } else {
                None
            };
            (Some(n), modules, materials, note_kind)
        }
        None => {
            let since = sub_days(at.now, RECENT_DAYS);
            let mut recent: Vec<&&Material> = content
                .iter()
                .filter(|m| m.published_at.is_some_and(|p| p >= since && p <= at.now))
                .collect();
            recent.sort_by(|a, b| {
                b.published_at
                    .cmp(&a.published_at)
                    .then(a.title.cmp(&b.title))
            });
            let materials = recent.into_iter().map(|m| data.view(m)).collect();
            let note_kind = match timeline.phase {
                _ if no_current_week => WeekNoteKind::OutsideTerm,
                CoursePhase::ExamPeriod => WeekNoteKind::ExamPeriod,
                CoursePhase::NotStarted | CoursePhase::Ended => WeekNoteKind::OutsideTerm,
                _ => WeekNoteKind::CurrentWeekUnknown,
            };
            (None, Vec::new(), materials, Some(note_kind))
        }
    };
    let note = note_kind.map(|kind| match kind {
        // The real reason: such a course may be inside its term dates, or have none.
        WeekNoteKind::OutsideTerm if no_current_week => no_current_week_note(),
        _ => week_note_text(kind, shown_week),
    });
    Ok(WeekMaterials {
        ai_materials: course.ai_materials(),
        week: shown_week,
        requested_week: week,
        timeline,
        modules,
        materials,
        available_weeks,
        note,
        note_kind,
        course,
    })
}

/// Whether the course's lifecycle leaves it without a current week (`CourseData::state`): Ended,
/// Inactive and Upcoming courses, with one exception. The student's own dates are the highest
/// authority: when they put the course in a teaching or break week, it keeps that week even if
/// its lifecycle is Upcoming, which only a session code naming a later term can make it (rule
/// 5). Its lifecycle isn't changed here: that would regroup the course, which is a larger change
/// than the week this rule is about.
fn has_no_current_week(timeline: &CourseTimeline, lifecycle: &CourseLifecycle) -> bool {
    let own_dates_say_teaching = lifecycle.state == LifecycleState::Upcoming
        && timeline.term.anchor == TermAnchorSource::StudentConfirmed
        && matches!(timeline.phase, CoursePhase::Teaching | CoursePhase::Break);
    !lifecycle.state.in_week_views() && !own_dates_say_teaching
}

/// Events with `when()` in [now - days_back, now + days_ahead], soonest first, optionally for
/// one course. Without a course filter, events of hidden courses are excluded (events not
/// linked to any course are kept). With a filter, `course` is resolved with
/// `include_hidden` (desktop: `true`, MCP: `false`).
pub fn deadlines(
    store: &Store,
    course: Option<&str>,
    days_ahead: u32,
    days_back: u32,
    include_hidden: bool,
    at: AsOf,
) -> Result<Vec<Deadline>> {
    let from = sub_days(at.now, days_back);
    let to = add_days(at.now, days_ahead);
    if let Some(course) = course {
        let course = store.resolve_course_with(course, include_hidden)?;
        let events = store.list_events(from, to, Some(&course.id))?;
        return Ok(events.iter().map(|e| deadline(e, Some(&course))).collect());
    }
    let courses: HashMap<String, Course> = store
        .list_courses(true)?
        .into_iter()
        .map(|c| (c.id.clone(), c))
        .collect();
    let mut result = Vec::new();
    for event in store.list_events(from, to, None)? {
        let course = event.course_id.as_ref().and_then(|id| courses.get(id));
        if course.is_some_and(|c| c.hidden) {
            continue;
        }
        result.push(deadline(&event, course));
    }
    Ok(result)
}

/// Announcements of the last `days` days, newest first; each text capped at `max_chars`
/// characters. `course` is resolved with `Store::resolve_course` (hidden excluded — this view
/// serves the MCP server). For a course whose material text is not readable (rule 8) the
/// announcements are listed with EMPTY text.
pub fn announcements(
    store: &Store,
    course: &str,
    days: u32,
    max_chars: usize,
    at: AsOf,
) -> Result<Vec<Announcement>> {
    let course = store.resolve_course(course)?;
    let ai_materials = course.ai_materials();
    let data = CourseData::load(store, &course)?;
    let since = sub_days(at.now, days);
    let mut items: Vec<&Material> = data
        .materials
        .iter()
        .filter(|m| m.kind == MaterialKind::Announcement)
        .filter(|m| m.published_at.is_some_and(|p| p >= since && p <= at.now))
        .collect();
    items.sort_by(|a, b| {
        b.published_at
            .cmp(&a.published_at)
            .then(a.title.cmp(&b.title))
    });
    let mut result = Vec::new();
    for item in items {
        let (text, truncated) = if ai_materials.is_readable() {
            let full: Vec<String> = store
                .get_chunks(&item.id, 0, None)?
                .into_iter()
                .map(|c| c.text)
                .collect();
            truncate_chars(&full.join("\n\n"), max_chars)
        } else {
            (String::new(), false)
        };
        result.push(Announcement {
            material: data.view(item),
            ai_materials,
            text,
            truncated,
        });
    }
    Ok(result)
}

/// Chunks starting at `from_chunk` until adding the next chunk would exceed `max_chars`
/// (clamped to `MIN_READ_CHARS..=MAX_READ_CHARS`). When the first chunk alone is longer, it
/// is returned cut to `max_chars` characters with `truncated = true`, so the limit always
/// holds. Materials of hidden courses → `NotFound`. For a course whose material text is not
/// readable (rule 8) no chunks are returned.
pub fn read_material(
    store: &Store,
    material_id: &str,
    from_chunk: u32,
    max_chars: usize,
) -> Result<MaterialText> {
    let not_found = || Error::NotFound(format!("material '{material_id}'"));
    let material = store.get_material(material_id)?.ok_or_else(not_found)?;
    let course = store
        .get_course(&material.course_id)?
        .filter(|c| !c.hidden)
        .ok_or_else(not_found)?;
    let data = CourseData::load(store, &course)?;
    let ai_materials = course.ai_materials();
    let total_chunks = data.chunks_of(&material.id);
    let mut text = MaterialText {
        material: data.view(&material),
        course_code: course.code.clone(),
        ai_materials,
        chunks: Vec::new(),
        from_chunk,
        next_chunk: None,
        total_chunks,
        truncated: false,
    };
    if !ai_materials.is_readable() {
        return Ok(text);
    }

    let budget = max_chars.clamp(MIN_READ_CHARS, MAX_READ_CHARS);
    let mut used = 0usize;
    let mut next = from_chunk;
    'batches: loop {
        let batch = store.get_chunks(&material.id, next, Some(READ_BATCH))?;
        if batch.is_empty() {
            break;
        }
        for mut chunk in batch {
            let len = chunk.text.chars().count();
            if text.chunks.is_empty() && len > budget {
                chunk.text = truncate_chars(&chunk.text, budget).0;
                text.truncated = true;
            } else if used + len > budget {
                break 'batches;
            }
            used += chunk.text.chars().count();
            next = chunk.ord + 1;
            text.chunks.push(chunk);
            if text.truncated {
                break 'batches;
            }
        }
    }
    text.next_chunk = (next < total_chunks && !text.chunks.is_empty()).then_some(next);
    Ok(text)
}

/// The student's own full-text search (desktop app): every non-hidden course, whatever its
/// AI access. `course` (optional) is resolved with `Store::resolve_course`.
pub fn search(
    store: &Store,
    query: &str,
    course: Option<&str>,
    limit: u32,
) -> Result<Vec<SearchHit>> {
    let course_id = course
        .map(|c| store.resolve_course(c).map(|c| c.id))
        .transpose()?;
    store.search(query, course_id.as_deref(), limit)
}

/// Search for an AI client: only courses whose material text is readable (rule 8). With a
/// course filter on a non-readable course the hits are empty and `course_ai_materials` says
/// why; without a filter, non-readable courses are excluded inside the SQL and listed in
/// `excluded_courses`.
pub fn search_for_ai(
    store: &Store,
    query: &str,
    course: Option<&str>,
    limit: u32,
) -> Result<AiSearchResults> {
    if let Some(course) = course {
        let course = store.resolve_course(course)?;
        let state = course.ai_materials();
        let hits = if state.is_readable() {
            store.search_ai_readable(query, Some(&course.id), limit)?
        } else {
            Vec::new()
        };
        return Ok(AiSearchResults {
            hits,
            course_ai_materials: Some(state),
            excluded_courses: Vec::new(),
        });
    }
    let excluded_courses = store
        .list_courses(false)?
        .into_iter()
        .filter(|c| !c.ai_materials().is_readable())
        .map(|c| c.code.unwrap_or(c.name))
        .collect();
    Ok(AiSearchResults {
        hits: store.search_ai_readable(query, None, limit)?,
        course_ai_materials: None,
        excluded_courses,
    })
}

/// Sources with freshness verdicts, store counts and the latest successful sync.
pub fn sync_status(store: &Store, at: AsOf) -> Result<SyncStatus> {
    let auto_sync = auto_sync::auto_sync(store)?;
    let stale_before = at.now - auto_sync.stale_after();
    let light = auto_sync::light_sync(store)?;
    let sources: Vec<SourceStatus> = store
        .list_sources()?
        .into_iter()
        .map(|source| {
            let deadlines_synced_at = light.deadlines_synced_at(&source);
            // (A time after now tells nothing: the clock was set forward when it was recorded.)
            let stale_at =
                |synced| auto_sync::known(synced, at.now).is_none_or(|at| at < stale_before);
            SourceStatus {
                stale: source.last_error.is_some() || stale_at(source.last_synced_at),
                deadlines_stale: stale_at(deadlines_synced_at),
                deadlines_synced_at,
                source,
            }
        })
        .collect();
    let last_synced_at = sources.iter().filter_map(|s| s.source.last_synced_at).max();
    let stale = sources.is_empty() || sources.iter().any(|s| s.stale);
    Ok(SyncStatus {
        counts: store.counts()?,
        sources,
        last_synced_at,
        stale,
        auto_sync,
        last_automatic_sync_at: auto_sync::attempts(store)?.last_ok_at,
    })
}

// ---------------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------------

/// Chunks fetched per query by `read_material`.
const READ_BATCH: u32 = 64;

/// Everything one course view needs, loaded once.
/// One course's rows, loaded once for its views (and the calendar's candidates).
pub(crate) struct CourseData {
    pub(crate) modules: Vec<Module>,
    pub(crate) materials: Vec<Material>,
    /// Every event of the course (deadlines, class events), any date.
    pub(crate) events: Vec<Event>,
    pub(crate) term_data: CourseTermData,
    dates_confirmed: bool,
    module_names: HashMap<String, String>,
    module_weeks: HashMap<String, u32>,
    chunk_counts: HashMap<String, u32>,
}

/// The courses whose student dates are confirmed (`CONFIRMED_DATES_KEY`). A value that
/// doesn't parse counts as none; a failed read is an error.
fn confirmed_courses(store: &Store) -> Result<BTreeSet<String>> {
    Ok(store
        .setting_or_absent::<BTreeSet<String>>(CONFIRMED_DATES_KEY)?
        .unwrap_or_default())
}

impl CourseData {
    pub(crate) fn load(store: &Store, course: &Course) -> Result<Self> {
        let term_data = store.course_term_data(&course.id)?.unwrap_or_default();
        Self::load_with(store, course, term_data, &confirmed_courses(store)?)
    }

    fn load_with(
        store: &Store,
        course: &Course,
        term_data: CourseTermData,
        confirmed: &BTreeSet<String>,
    ) -> Result<Self> {
        let modules = store.list_modules(&course.id)?;
        let module_names = modules
            .iter()
            .map(|m| (m.id.clone(), m.name.clone()))
            .collect();
        let module_weeks = modules
            .iter()
            .filter_map(|m| Some((m.id.clone(), m.week_hint?)))
            .collect();
        Ok(CourseData {
            materials: store.list_materials(&course.id)?,
            events: store.list_events(
                DateTime::<Utc>::MIN_UTC,
                DateTime::<Utc>::MAX_UTC,
                Some(&course.id),
            )?,
            dates_confirmed: confirmed.contains(&course.id),
            term_data,
            chunk_counts: store.material_chunk_counts(&course.id)?,
            modules,
            module_names,
            module_weeks,
        })
    }

    fn input<'a>(&'a self, course: &'a Course, at: AsOf) -> TermInput<'a> {
        TermInput {
            fallback_tz: at.tz,
            course,
            data: &self.term_data,
            dates_confirmed: self.dates_confirmed,
            // Accepted calendars live in schema v4 (alpha.2); none before that.
            calendar: None,
            modules: &self.modules,
            materials: &self.materials,
            events: &self.events,
            today: at.today,
        }
    }

    /// The resolved dates, the timeline and the lifecycle, as every view shows them. A course
    /// that is over, inactive or hasn't started (by its lifecycle) has no current week, no
    /// default week and no current modules, whatever the week signals say (design §8.2, D43):
    /// without usable dates, the week number of a material posted years ago would otherwise
    /// stay "the current week". The signals stay in the evidence, and "I'm still taking this"
    /// (a Current lifecycle) brings the week back.
    pub(crate) fn state(
        &self,
        course: &Course,
        at: AsOf,
    ) -> (ResolvedTerm, CourseTimeline, CourseLifecycle) {
        let (resolved, mut timeline) = self.timeline(course, at);
        // The lifecycle reads the phase and the dates, never the week.
        let lifecycle = self.lifecycle(course, &resolved, &timeline, at);
        if has_no_current_week(&timeline, &lifecycle) {
            timeline.current_week = None;
            timeline.default_week = None;
            timeline.current_module_ids.clear();
            timeline.confidence = Confidence::Low;
        }
        (resolved, timeline, lifecycle)
    }

    /// The resolved dates and the timeline (`term::resolve_term`, `timeline::infer_timeline`),
    /// before the lifecycle is applied: views use `state`.
    pub(crate) fn timeline(&self, course: &Course, at: AsOf) -> (ResolvedTerm, CourseTimeline) {
        let input = self.input(course, at);
        let resolved = resolve_term(&input);
        let timeline = timeline::infer_timeline(&input, &resolved);
        (resolved, timeline)
    }

    pub(crate) fn chunks_of(&self, material_id: &str) -> u32 {
        self.chunk_counts.get(material_id).copied().unwrap_or(0)
    }

    /// The course's lifecycle (`lifecycle::course_lifecycle`).
    fn lifecycle(
        &self,
        course: &Course,
        resolved: &ResolvedTerm,
        timeline: &CourseTimeline,
        at: AsOf,
    ) -> CourseLifecycle {
        lifecycle::course_lifecycle(&LifecycleInput {
            course,
            data: &self.term_data,
            resolved,
            timeline,
            events: &self.events,
            today: at.today,
        })
    }

    /// Teaching week of a material per the `week_materials` membership rule: its own week
    /// number, its module's, else the teaching week it was published in by the resolved dates.
    fn week_of(&self, resolved: &ResolvedTerm, material: &Material) -> Option<u32> {
        material
            .week_hint
            .or_else(|| self.module_weeks.get(material.module_id.as_ref()?).copied())
            .or_else(|| {
                let published = course_date(material.published_at?, resolved.tz);
                teaching_week_on(&resolved.resolution, published)
            })
    }

    fn view(&self, material: &Material) -> MaterialView {
        let chunk_count = self.chunks_of(&material.id);
        MaterialView {
            id: material.id.clone(),
            course_id: material.course_id.clone(),
            title: material.title.clone(),
            kind: material.kind,
            module_id: material.module_id.clone(),
            module_name: material
                .module_id
                .as_ref()
                .and_then(|id| self.module_names.get(id).cloned()),
            week_hint: material.week_hint,
            published_at: material.published_at,
            url: material.url.clone(),
            text_status: material.text_status,
            text_error: material.text_error.clone(),
            download_blocked: material.download_blocked,
            chunk_count,
            text_problem: TextProblem::of(material, chunk_count),
        }
    }
}

/// Source labels and sync times by source id, with what the light automatic syncs left.
struct SourceIndex {
    sources: HashMap<String, SourceRecord>,
    light: LightSync,
}

/// A course's source and how fresh the course's data is.
struct SourceInfo {
    label: String,
    last_synced_at: Option<Timestamp>,
    deadlines_synced_at: Option<Timestamp>,
    structure_pending: bool,
}

impl SourceIndex {
    fn load(store: &Store) -> Result<Self> {
        Ok(SourceIndex {
            sources: store
                .list_sources()?
                .into_iter()
                .map(|s| (s.id.clone(), s))
                .collect(),
            light: auto_sync::light_sync(store)?,
        })
    }

    fn info(&self, course: &Course) -> SourceInfo {
        let structure_pending = self.light.structure_pending.contains(&course.id);
        match self.sources.get(&course.source_id) {
            Some(source) => SourceInfo {
                label: source.label.clone(),
                last_synced_at: source.last_synced_at,
                deadlines_synced_at: self.light.deadlines_synced_at(source),
                structure_pending,
            },
            None => SourceInfo {
                label: course.source_id.clone(),
                last_synced_at: None,
                deadlines_synced_at: None,
                structure_pending,
            },
        }
    }
}

fn deadline(event: &Event, course: Option<&Course>) -> Deadline {
    Deadline {
        event: event.clone(),
        course_code: course.and_then(|c| c.code.clone()),
        course_name: course.map(|c| c.name.clone()),
    }
}

/// `n` days as a duration (u32 days always fit).
fn days(n: u32) -> TimeDelta {
    TimeDelta::days(i64::from(n))
}

/// `now + n days`, saturating at the largest representable instant (the store clamps
/// out-of-range bounds, so huge windows simply mean "everything").
fn add_days(now: Timestamp, n: u32) -> Timestamp {
    now.checked_add_signed(days(n))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// `now - n days`, saturating at the smallest representable instant.
fn sub_days(now: Timestamp, n: u32) -> Timestamp {
    now.checked_sub_signed(days(n))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

fn count_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// First `max_chars` characters of `text` (never splits a UTF-8 character) and whether
/// anything was cut.
fn truncate_chars(text: &str, max_chars: usize) -> (String, bool) {
    match text.char_indices().nth(max_chars) {
        Some((byte, _)) => (text[..byte].to_string(), true),
        None => (text.to_string(), false),
    }
}

/// The note of a course whose lifecycle leaves it without a current week (`OutsideTerm`).
fn no_current_week_note() -> String {
    format!(
        "The course is over, inactive or hasn't started, so it has no current week; showing \
         materials published in the last {RECENT_DAYS} days."
    )
}

/// English text for `WeekMaterials.note` (UIs localise from `note_kind`).
fn week_note_text(kind: WeekNoteKind, week: Option<u32>) -> String {
    match kind {
        WeekNoteKind::CurrentWeekUnknown => format!(
            "The current week could not be determined; showing materials published in the last {RECENT_DAYS} days."
        ),
        WeekNoteKind::OutsideTerm => match week {
            Some(n) => format!("Today is outside the course's term; showing week {n}."),
            None => "Today is outside the course's term.".to_string(),
        },
        WeekNoteKind::NoMaterialsThisWeek => match week {
            Some(n) => format!("No modules or materials are assigned to week {n}."),
            None => "No modules or materials found.".to_string(),
        },
        WeekNoteKind::ExamPeriod => format!(
            "The course is in its exam period; showing materials published in the last {RECENT_DAYS} days."
        ),
        WeekNoteKind::Break => match week {
            Some(n) => format!("The course is on a break; showing week {n}."),
            None => "The course is on a break.".to_string(),
        },
    }
}
