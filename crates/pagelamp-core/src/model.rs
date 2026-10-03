//! Shared data types. These are persisted in SQLite (see `store`) and several of them are
//! returned verbatim (as JSON) from MCP tools, so they derive `JsonSchema` and field names
//! are part of the public tool contract — rename with care.
//!
//! Conventions:
//! - IDs are opaque strings, stable across syncs. Format: `<source_id>/<kind>/<external_id>`
//!   for synced entities (e.g. `canvas:lms.example.edu/course/12345`), so the same Canvas
//!   object always maps to the same row.
//! - Instants are `DateTime<Utc>` (stored as RFC 3339 text). Calendar dates are `NaiveDate`.
//! - Week numbers are 1-based teaching weeks counted from the course's term start.

use chrono::{DateTime, NaiveDate, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type Timestamp = DateTime<Utc>;

// Course calendar and lifecycle types live next to their logic; they are model types too.
pub use crate::calendar::{CalendarWeek, CourseCalendar};
pub use crate::lifecycle::{CourseGroup, CourseLifecycle, LifecycleState, SnoozeKind};
pub use crate::term::evidence::{EvidenceCode, EvidenceItem, EvidenceParam, EvidenceSignal};
pub use crate::term::{
    AiLabel, BreakKind, CalendarBreak, CalendarOrigin, CalendarStatus, CoursePhase, DateSpan,
    RejectReason, RejectedDates, TeachingSegment, TermAnchorSource, TermResolution,
};

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// Where course data comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Canvas REST API with the student's own token (personal/dev use only — see README).
    Canvas,
    /// A local folder of course materials: `<root>/<COURSE>/...`. Works with any LMS.
    Folder,
    /// An iCalendar feed (e.g. Canvas "Calendar Feed" URL) providing deadlines/events.
    Ical,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Canvas => "canvas",
            SourceKind::Folder => "folder",
            SourceKind::Ical => "ical",
        }
    }
}

/// A configured data source. Secrets (Canvas token, calendar-feed URL) are NOT stored here;
/// they live in the OS keychain keyed by `id` (see `secrets`).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SourceRecord {
    /// e.g. `canvas:lms.example.edu`, `folder:3f2a…`, `ical:9b1c…`
    pub id: String,
    pub kind: SourceKind,
    /// Human label shown in status output, e.g. "Quercus" or "~/Courses".
    pub label: String,
    /// Non-secret configuration. Canvas: `{"base_url": "https://lms.example.edu"}`.
    /// Folder: `{"path": "/Users/me/Courses", "term_start": "2026-09-08"?}`.
    /// Ical: `{}` (the URL itself is a secret).
    pub config: serde_json::Value,
    pub last_synced_at: Option<Timestamp>,
    /// Error message of the most recent failed sync, cleared on success. Never contains secrets.
    pub last_error: Option<String>,
    /// Machine-readable class of `last_error` (UIs branch on this, never on the message).
    pub last_error_kind: Option<SourceErrorKind>,
}

/// Why a source failed to sync. Persisted as `sources.last_error_kind`.
///
/// Classification: Canvas 401 / invalid token and feed-URL 401/403 → `AuthExpiredOrRevoked`;
/// DNS/TLS/timeout/connection refused → `Network`; missing folder, feed 404 or Canvas host 404
/// → `NotFound`; Canvas throttling still failing after all retries → `RateLimited`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceErrorKind {
    AuthExpiredOrRevoked,
    Network,
    NotFound,
    RateLimited,
    Other,
}

impl SourceErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceErrorKind::AuthExpiredOrRevoked => "auth_expired_or_revoked",
            SourceErrorKind::Network => "network",
            SourceErrorKind::NotFound => "not_found",
            SourceErrorKind::RateLimited => "rate_limited",
            SourceErrorKind::Other => "other",
        }
    }
}

impl std::str::FromStr for SourceErrorKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "auth_expired_or_revoked" => SourceErrorKind::AuthExpiredOrRevoked,
            "network" => SourceErrorKind::Network,
            "not_found" => SourceErrorKind::NotFound,
            "rate_limited" => SourceErrorKind::RateLimited,
            "other" => SourceErrorKind::Other,
            other => return Err(format!("unknown source error kind '{other}'")),
        })
    }
}

// ---------------------------------------------------------------------------
// Courses
// ---------------------------------------------------------------------------

/// How the student is allowed to use generative AI in a course. Defaults to `Unknown`,
/// which consumers must treat like `LearningAid` for explanations but must never treat as
/// permission to produce graded work (UofT default: GenAI not permitted unless allowed).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AiPolicy {
    #[default]
    Unknown,
    /// No generative AI use permitted for this course's assessed work.
    Prohibited,
    /// AI may be used to learn/understand (explanations, study plans), not to produce graded work.
    LearningAid,
    /// AI may be used for assessed work with citation/disclosure.
    AllowedWithCitation,
    /// No restrictions stated by the instructor.
    Unrestricted,
}

impl AiPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            AiPolicy::Unknown => "unknown",
            AiPolicy::Prohibited => "prohibited",
            AiPolicy::LearningAid => "learning_aid",
            AiPolicy::AllowedWithCitation => "allowed_with_citation",
            AiPolicy::Unrestricted => "unrestricted",
        }
    }
}

impl std::str::FromStr for AiPolicy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "unknown" => AiPolicy::Unknown,
            "prohibited" => AiPolicy::Prohibited,
            "learning_aid" => AiPolicy::LearningAid,
            "allowed_with_citation" => AiPolicy::AllowedWithCitation,
            "unrestricted" => AiPolicy::Unrestricted,
            other => return Err(format!("unknown AI policy '{other}'")),
        })
    }
}

/// A course as produced by a source during sync. User-editable fields (AI policy, term
/// overrides, hidden flag) are NOT part of this type so that re-syncing never clobbers them.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CourseUpsert {
    pub id: String,
    pub source_id: String,
    pub external_id: String,
    /// Short code such as "CSC413H1". May be absent for folder courses (then derived from dir name).
    pub code: Option<String>,
    pub name: String,
    pub term_start: Option<NaiveDate>,
    pub term_end: Option<NaiveDate>,
    /// Link back to the course home page (for citations / "open in browser").
    pub url: Option<String>,
    /// Canvas syllabus HTML converted to text, if available (used later for AI-policy hints).
    pub syllabus_text: Option<String>,
    /// The LMS's own course and term facts (schema 3); all `None` for folder courses. Written on
    /// every upsert: the sync that upserts a course states them in full.
    pub lms: LmsCourseInfo,
}

/// What the LMS says about a course's dates and state, stored raw (schema 3, the `lms_*`
/// columns; docs/design/v0.3-course-calendar.md §3.1). Written only by sync. `None` means
/// unknown (not reported, or not an LMS course). Not part of the facade's `Course` type:
/// `core::term` / `core::lifecycle` read it through `Store::course_term_data`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LmsCourseInfo {
    /// `term.name`, e.g. "Fall 2026".
    pub term_name: Option<String>,
    /// Term dates as calendar dates in the course's time zone.
    pub term_start: Option<NaiveDate>,
    pub term_end: Option<NaiveDate>,
    /// `course.start_at` / `end_at` as calendar dates in the course's time zone.
    pub course_start: Option<NaiveDate>,
    pub course_end: Option<NaiveDate>,
    /// IANA time zone name of the course.
    pub time_zone: Option<String>,
    pub concluded: Option<bool>,
    pub workflow_state: Option<String>,
    /// Listed by the LMS but with `access_restricted_by_date` set.
    pub access_restricted: Option<bool>,
}

/// What happened to the copy of the database the last migration tried to make (settings key
/// `store::LAST_MIGRATION_BACKUP`). Codes only: no paths, no messages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MigrationBackupRecord {
    pub from_version: i64,
    pub to_version: i64,
    pub at: Timestamp,
    pub outcome: MigrationBackupOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum MigrationBackupOutcome {
    /// `pagelamp.db.v<from_version>.bak` was written.
    Ok,
    /// No copy: another process was migrating the same database at that moment.
    Skipped,
    /// No copy; the migration went ahead anyway. `code` is e.g. `storage_full`,
    /// `permission_denied`, `sqlite` or `other`.
    Failed { code: String },
}

/// A course's term and lifecycle inputs beyond `Course`: the LMS facts and the student's own
/// answers (schema 3). Read with `Store::course_term_data`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CourseTermData {
    pub lms: LmsCourseInfo,
    /// "I'm still taking this": keep the course current until this date (student; never
    /// touched by sync).
    pub keep_current_until: Option<NaiveDate>,
    /// "Not now" / "Keep" on the removal suggestion (student; `9999-12-31` = keep).
    pub removal_snoozed_until: Option<NaiveDate>,
    /// The synced `term_start`/`term_end` (before the student's overrides): for Canvas the
    /// "term first" merge of the LMS dates, for folders `course.toml` or the source's start.
    pub synced_term_start: Option<NaiveDate>,
    pub synced_term_end: Option<NaiveDate>,
    /// The student's own overrides as saved (`Course.term_*` merges them field by field).
    pub user_term_start: Option<NaiveDate>,
    pub user_term_end: Option<NaiveDate>,
    /// The school a folder course names in `course.toml` (`institution = "uoft"`): opts it into
    /// the session hint and the school's calendar (calendar design §6.3, §6.4). Canvas courses
    /// are known by their host instead.
    #[serde(default)]
    pub institution: Option<String>,
}

/// A course as read back from the store: synced fields + user overrides resolved.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Course {
    pub id: String,
    pub source_id: String,
    pub external_id: String,
    pub code: Option<String>,
    pub name: String,
    /// Effective term start = user override if set, else synced value.
    pub term_start: Option<NaiveDate>,
    pub term_end: Option<NaiveDate>,
    /// Where the effective term dates come from (`set_course_term(None, None)` clears the
    /// user override and falls back to the synced dates).
    pub term_source: TermSource,
    pub url: Option<String>,
    pub ai_policy: AiPolicy,
    /// Free text the student recorded about the policy (e.g. a quote from the syllabus).
    pub ai_policy_note: Option<String>,
    /// The student's switch "Let my AI app read this course's materials" (default on).
    /// Use `ai_materials()` for the effective state — a `prohibited` policy wins over it.
    pub ai_access: bool,
    /// The student's answer to "May this course's materials be shared with an AI service?"
    /// (question (b)). Only `not_allowed` keeps material text from cloud models PageLamp runs
    /// itself; the student's own AI app over MCP is unaffected.
    pub material_sharing: crate::ai::MaterialSharing,
    pub hidden: bool,
    /// False when the LMS no longer lists the course as active (e.g. the term ended). Such
    /// courses are kept with all their data; only the student removes them.
    pub enrollment_active: bool,
    pub updated_at: Timestamp,
}

/// Origin of a course's effective term dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TermSource {
    /// The student set a start and/or end date (overrides the synced dates).
    User,
    /// Dates come from the source (LMS term, course.toml, folder config).
    Synced,
    /// No term dates known.
    None,
}

/// Whether an AI app may read a course's material TEXT over MCP (docs/ARCHITECTURE.md §3
/// rule 8). Computed from `Course.ai_policy` and `Course.ai_access`, never stored.
/// Structure (titles, kinds, dates, weeks, URLs, counts), deadlines and study plans are
/// always available; only material text is withheld.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AiMaterialsState {
    Readable,
    /// The student switched AI access off for this course.
    TurnedOff,
    /// The student marked the course `ai_policy = prohibited` (wins over the switch).
    WithheldByPolicy,
}

impl AiMaterialsState {
    pub fn is_readable(self) -> bool {
        self == AiMaterialsState::Readable
    }
}

impl Course {
    /// Effective AI access to this course's material text: `prohibited` policy →
    /// `WithheldByPolicy` (whatever the switch says); else switch off → `TurnedOff`; else
    /// `Readable` (an `unknown` policy is readable).
    pub fn ai_materials(&self) -> AiMaterialsState {
        if self.ai_policy == AiPolicy::Prohibited {
            AiMaterialsState::WithheldByPolicy
        } else if !self.ai_access {
            AiMaterialsState::TurnedOff
        } else {
            AiMaterialsState::Readable
        }
    }

    /// "CSC413H1 — Neural Networks and Deep Learning" or just the name.
    pub fn display_name(&self) -> String {
        match &self.code {
            Some(code) if !self.name.starts_with(code.as_str()) => {
                format!("{code} — {}", self.name)
            }
            _ => self.name.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Modules / materials / chunks
// ---------------------------------------------------------------------------

/// A unit of course structure (Canvas module, or a sub-folder like "Week 3").
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Module {
    pub id: String,
    pub course_id: String,
    pub name: String,
    pub position: Option<i64>,
    pub unlock_at: Option<Timestamp>,
    /// Week number parsed from the name ("Week 3", "W03"), if any.
    pub week_hint: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    /// A downloadable file (PDF, PPTX, DOCX, notebook, text…).
    File,
    /// An LMS page (HTML content).
    Page,
    Announcement,
    Syllabus,
    /// A link to an external resource (course website, video). Title/URL only.
    ExternalLink,
}

impl MaterialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MaterialKind::File => "file",
            MaterialKind::Page => "page",
            MaterialKind::Announcement => "announcement",
            MaterialKind::Syllabus => "syllabus",
            MaterialKind::ExternalLink => "external_link",
        }
    }
}

/// Whether we have searchable text for a material.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextStatus {
    /// Not processed yet.
    Pending,
    /// Text extracted and indexed.
    Ok,
    /// File type we cannot extract (video, images, …).
    Unsupported,
    /// Known to exist but not downloaded (too large, download disabled, access denied).
    NotDownloaded,
    /// Extraction failed; see `text_error`.
    Error,
}

impl TextStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TextStatus::Pending => "pending",
            TextStatus::Ok => "ok",
            TextStatus::Unsupported => "unsupported",
            TextStatus::NotDownloaded => "not_downloaded",
            TextStatus::Error => "error",
        }
    }
}

/// Why the extraction worker (`pagelamp extract-worker`, v0.3 M0.5) could not read a file:
/// `materials.text_error_kind`, next to `text_status = error`. Same names as
/// `pagelamp_extract::worker::WorkerFailure`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TextErrorKind {
    /// Still running at the wall-clock limit.
    TimedOut,
    /// Used up its CPU-time budget.
    CpuLimit,
    /// Went past its memory cap.
    MemoryLimit,
    /// Died without an answer (a signal, an abort).
    Crashed,
    /// Answered something unreadable.
    BadOutput,
    /// Could not be started (e.g. blocked by antivirus).
    SpawnFailed,
    /// A worker binary from another version answered.
    ProtocolMismatch,
}

impl TextErrorKind {
    pub const ALL: [TextErrorKind; 7] = [
        TextErrorKind::TimedOut,
        TextErrorKind::CpuLimit,
        TextErrorKind::MemoryLimit,
        TextErrorKind::Crashed,
        TextErrorKind::BadOutput,
        TextErrorKind::SpawnFailed,
        TextErrorKind::ProtocolMismatch,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TextErrorKind::TimedOut => "timed_out",
            TextErrorKind::CpuLimit => "cpu_limit",
            TextErrorKind::MemoryLimit => "memory_limit",
            TextErrorKind::Crashed => "crashed",
            TextErrorKind::BadOutput => "bad_output",
            TextErrorKind::SpawnFailed => "spawn_failed",
            TextErrorKind::ProtocolMismatch => "protocol_mismatch",
        }
    }

    /// A limit or crash caused by the file itself: not tried again while the file's content,
    /// the worker protocol and the app version stay the same (`materials.text_error_fingerprint`).
    /// The other kinds are about the worker, not the file, and are retried on the next sync.
    pub fn is_hard(self) -> bool {
        matches!(
            self,
            TextErrorKind::TimedOut
                | TextErrorKind::CpuLimit
                | TextErrorKind::MemoryLimit
                | TextErrorKind::Crashed
        )
    }
}

/// Why a file that is recorded `NotDownloaded` cannot be downloaded by asking again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownloadBlock {
    /// The LMS locks the file for this student (e.g. not released yet).
    Locked,
    /// Larger than the download size limit.
    TooLarge,
}

impl DownloadBlock {
    pub fn as_str(self) -> &'static str {
        match self {
            DownloadBlock::Locked => "locked",
            DownloadBlock::TooLarge => "too_large",
        }
    }
}

/// A material as produced by a source during sync. Index state (hash, text status) is owned
/// by `ingest` and is preserved when an existing row is upserted.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct MaterialUpsert {
    pub id: String,
    pub course_id: String,
    pub module_id: Option<String>,
    pub kind: MaterialKind,
    pub title: String,
    pub url: Option<String>,
    pub local_path: Option<String>,
    pub mime: Option<String>,
    pub published_at: Option<Timestamp>,
    pub week_hint: Option<u32>,
}

/// A piece of course content. Text lives in `chunks` (see `Chunk`).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Material {
    pub id: String,
    pub course_id: String,
    pub module_id: Option<String>,
    pub kind: MaterialKind,
    pub title: String,
    /// Where the student can open the original (LMS URL or `file://` path) — used for citations.
    pub url: Option<String>,
    /// Absolute path of the cached/local copy, if any.
    pub local_path: Option<String>,
    pub mime: Option<String>,
    /// When the material was published/posted/unlocked (best available date).
    pub published_at: Option<Timestamp>,
    /// Teaching week this material belongs to, if known from structure (module/folder name).
    pub week_hint: Option<u32>,
    /// SHA-256 of the source bytes/HTML; used to skip re-extraction when unchanged.
    pub content_hash: Option<String>,
    pub text_status: TextStatus,
    pub text_error: Option<String>,
    /// Why the extraction worker failed on this content (with `text_status = error`).
    pub text_error_kind: Option<TextErrorKind>,
    /// The worker protocol and app version that failed (`ingest::failure_fingerprint`).
    pub text_error_fingerprint: Option<String>,
    /// Set (by the Canvas sync) when a `NotDownloaded` file cannot be downloaded on request.
    pub download_blocked: Option<DownloadBlock>,
    pub updated_at: Timestamp,
}

/// A searchable slice of a material's text.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Chunk {
    pub material_id: String,
    /// 0-based order within the material.
    pub ord: u32,
    /// Human-readable position for citations: "p. 3", "slide 5", "cell 12", "§ Backprop".
    pub locator: Option<String>,
    pub text: String,
}

// ---------------------------------------------------------------------------
// Events (deadlines, calendar)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    AssignmentDue,
    QuizDue,
    Exam,
    /// Lecture/tutorial/office hours etc.
    ClassEvent,
    /// Canvas planner/to-do item.
    PlannerItem,
    Other,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::AssignmentDue => "assignment_due",
            EventKind::QuizDue => "quiz_due",
            EventKind::Exam => "exam",
            EventKind::ClassEvent => "class_event",
            EventKind::PlannerItem => "planner_item",
            EventKind::Other => "other",
        }
    }
}

/// A dated item used for planning. PageLamp never fetches assignment instructions to solve
/// them; it only records *that* something is due and when.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub id: String,
    pub source_id: String,
    pub course_id: Option<String>,
    pub kind: EventKind,
    pub title: String,
    pub starts_at: Option<Timestamp>,
    pub ends_at: Option<Timestamp>,
    pub due_at: Option<Timestamp>,
    pub url: Option<String>,
    pub updated_at: Timestamp,
    /// The course text the source gave, e.g. the iCal "[DEMO101H1 F LEC0101]" suffix. Kept
    /// so `Store::relink_events` can link the event to a course that a later folder/Canvas
    /// sync creates. Internal: never serialised.
    #[serde(skip)]
    pub course_hint: Option<String>,
}

/// The course that a source's course text (e.g. the iCal "[DEMO101H1 F LEC0101]" suffix)
/// refers to: the course whose code is a prefix of the text, case-insensitive with spaces
/// ignored. The longest code wins ("DEMO1011" over "DEMO101"); between equally long codes a
/// visible course wins over a hidden one (e.g. last year's folder of the same course).
pub fn course_for_hint<'a>(hint: &str, courses: &'a [Course]) -> Option<&'a Course> {
    let target = squash(hint);
    courses
        .iter()
        .filter_map(|course| {
            let code = squash(course.code.as_deref()?);
            (!code.is_empty() && target.starts_with(&code)).then_some((code.len(), course))
        })
        .max_by_key(|(len, course)| (*len, !course.hidden))
        .map(|(_, course)| course)
}

/// Whether a course text (see `course_for_hint`) starts with `code` as a whole token,
/// case-insensitive with spaces ignored (e.g. a feed event of a removed course): "DEMO101 F"
/// and "demo 101" name DEMO101, "DEMO1011 S" doesn't ("MAT1" never hides MAT135).
pub fn hint_names_code(hint: &str, code: &str) -> bool {
    let code = squash(code);
    if code.is_empty() {
        return false;
    }
    let mut hint = hint.chars().flat_map(char::to_uppercase).peekable();
    for want in code.chars() {
        loop {
            match hint.next() {
                Some(c) if c.is_whitespace() => continue,
                Some(c) if c == want => break,
                _ => return false,
            }
        }
    }
    !hint.peek().is_some_and(|c| c.is_alphanumeric())
}

fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

impl Event {
    /// The instant used for sorting / range filtering: due_at, else starts_at.
    pub fn when(&self) -> Option<Timestamp> {
        self.due_at.or(self.starts_at)
    }
}

// ---------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchHit {
    pub material_id: String,
    pub material_title: String,
    pub course_id: String,
    pub course_code: Option<String>,
    pub chunk_ord: u32,
    pub locator: Option<String>,
    /// Short excerpt around the match (FTS5 snippet), with matches wrapped in «».
    pub snippet: String,
    pub url: Option<String>,
    pub week_hint: Option<u32>,
    /// Lower is better (FTS5 bm25).
    pub score: f64,
}

// ---------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

/// "Where is this course right now", with the evidence that led to the conclusion so an AI
/// client can explain (and a student can correct) it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CourseTimeline {
    pub as_of: NaiveDate,
    pub current_week: Option<u32>,
    /// Confidence of `current_week` (Low when it is None).
    pub confidence: Confidence,
    /// Human-readable reasons, e.g. "module 'Week 4: Backprop' unlocked 2026-09-29". English,
    /// for MCP and the CLI; UIs translate `evidence_items`. Never contains break labels, week
    /// topics or quotes.
    pub evidence: Vec<String>,
    /// Modules considered current (most recently unlocked / matching current week).
    pub current_module_ids: Vec<String>,
    /// True when the phase is `not_started` or `ended` (the exam period is not outside).
    pub outside_term: bool,
    pub phase: CoursePhase,
    pub phase_confidence: Confidence,
    /// The day teaching starts, when known (a weekend first class → the next Monday), for
    /// "Starts Jan 11"; surfaces show this instead of computing it.
    pub starts_on: Option<NaiveDate>,
    /// The week features and `week_materials` use by default: the current teaching week, the
    /// week before an unnumbered break, None in the exam period and outside the term.
    pub default_week: Option<u32>,
    /// During a break that doesn't count in the numbering: the last teaching week before it.
    pub break_after_week: Option<u32>,
    /// During the exam period: the last teaching week ("Exams (after week 12)").
    pub last_teaching_week: Option<u32>,
    /// During a break: its kind.
    pub current_break_kind: Option<BreakKind>,
    /// How far the professor's (non-bulk) week-numbered materials have got.
    pub notes_week: Option<u32>,
    /// The dates that count the weeks, and the dates that were not used.
    pub term: TermResolution,
    pub calendar: CalendarStatus,
    pub evidence_items: Vec<EvidenceItem>,
}

// ---------------------------------------------------------------------------
// Store statistics
// ---------------------------------------------------------------------------

/// Row counts for status screens / `sync_status`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoreCounts {
    /// Visible (not hidden) courses.
    pub courses: u32,
    pub hidden_courses: u32,
    pub modules: u32,
    pub materials: u32,
    /// Materials with text_status = ok and at least one chunk.
    pub indexed_materials: u32,
    pub chunks: u32,
    pub events: u32,
    pub study_plans: u32,
    /// Courses under "Removed courses" (their tombstones), not counted above.
    pub removed_courses: u32,
}

// ---------------------------------------------------------------------------
// Study plans (the only thing MCP clients may write)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StudyPlanItem {
    pub date: NaiveDate,
    /// Course id (or code) this task belongs to, if any.
    pub course_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    /// Materials to study for this item (ids from `week_materials` / `search_materials`).
    #[serde(default)]
    pub material_ids: Vec<String>,
    pub minutes: Option<u32>,
    #[serde(default)]
    pub done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StudyPlan {
    pub horizon_start: NaiveDate,
    pub horizon_end: NaiveDate,
    pub items: Vec<StudyPlanItem>,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct StoredStudyPlan {
    pub id: i64,
    pub created_at: Timestamp,
    pub plan: StudyPlan,
}

#[cfg(test)]
mod tests {
    use super::hint_names_code;

    #[test]
    fn a_hint_names_a_code_only_as_a_whole_token() {
        assert!(hint_names_code("DEMO101H1 F LEC0101", "DEMO101H1"));
        assert!(hint_names_code("demo 101 f", "DEMO101"));
        assert!(hint_names_code("DEMO101", "demo 101"));
        assert!(hint_names_code("DEMO101-LEC0101", "DEMO101"));
        assert!(!hint_names_code("DEMO1011 S LEC0101", "DEMO101"));
        assert!(!hint_names_code("MAT135 F", "MAT1"));
        assert!(!hint_names_code("MAT135", ""));
        assert!(!hint_names_code("MA", "MAT135"));
    }
}
