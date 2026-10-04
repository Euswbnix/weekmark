//! UniFFI mirrors of every facade type the Swift app sees.
//!
//! `#[uniffi::remote(..)]` mirrors restate the real definitions from `pagelamp-app` /
//! `pagelamp-core` (which do not depend on UniFFI). The generated converters construct and
//! destructure the real types field by field and match every enum variant, so a mirror that
//! drifts from the facade (a field or variant added, removed, renamed or retyped) is a
//! compile error in this crate. `tests::every_schema_type_is_mirrored` additionally fails
//! when the facade's JSON Schema (`pagelamp schema`) gains a type nobody mirrored yet.
//!
//! Types UniFFI cannot carry become custom types (Swift sees the builtin on the right):
//!
//! | Rust (facade)                 | alias here   | Swift                  |
//! |-------------------------------|--------------|------------------------|
//! | `chrono::DateTime<Utc>`       | `Timestamp`  | `Date`                 |
//! | `chrono::NaiveDate`           | `IsoDate`    | `String`, "YYYY-MM-DD" |
//! | `serde_json::Value`           | `JsonString` | `String` (JSON text)   |
//! | `BTreeMap<String, String>`    | `EnvMap`     | `[String: String]`     |
//!
//! `PathBuf`/`&Path` never cross the boundary as such: the exported methods take and return
//! `String`s (see `lib.rs`).

use std::collections::{BTreeMap, HashMap};
use std::time::SystemTime;

use pagelamp_app::ai::{
    AiBackendStatus, AiStatus, BackendKind, BackendProblem, BackendRef, BackendState, BudgetStatus,
    CostBasis, CostEstimate, CostKind, DisclosureFacts, EstimateRequest, FeatureRouting, GenEvent,
    GenNoticeCode, GenStage, GenerationMeta, LocalServer, LocalServerKind, ModelChoice, ModelInfo,
    ModelProviderRecord, ProbeReport, ProviderPreset, ProviderWire, Recipient, RemoveAiDataReport,
    RetentionFact, SentData, StructuredOutputTier, TokenUsage, TrainingFact, UsageRow,
    UsageSummary,
};
use pagelamp_app::diagnostics::{
    CrashReport, DoctorReport, DoctorSource, ExtractWorkerCheck, ExtractWorkerStatus,
    McpClientPresence, ProcessKind, UnreadableFiles,
};
use pagelamp_app::{
    Activity, ActivityItem, ActivityKind, AppErrorKind, AppStatus, AutoSync, AutoSyncTrigger,
    BackupInfo, BreakInput, CalendarBatchEvent, CalendarRunOutcome, CourseCalendarView,
    CourseDatesInput, CourseLifecycleEntry, InstallKind, LifecycleSummary, LostAfterPurge,
    McpClient, McpClientConfig, McpLaunch, McpNoteCode, PurgeReport, ReadCalendarOptions,
    RemovalPreview, RemovalPreviewItem, RemovalReason, RemovalReport, RemoveOptions, RemovedCourse,
    RestoreFailure, RestoreOutcome, SegmentInput, SourceSyncResult, StartupTasks, SyllabusOffer,
    SyncDue, SyncEvent, SyncPrefs, SyncRequest, SyncSummary, TemporaryLocation, TombstoneState,
    UpdateChannel, UpdateCheckOutcome, UpdateCheckRecord, UpdatePrefs, WhatsNew, WhatsNewTopic,
};
use pagelamp_core::ai::{AiFeature, BlockReason, Effort, MaterialSharing, ModelErrorKind};
use pagelamp_core::ai_gate::{ContextCourse, ContextSummary, LeftOutMaterial, LeftOutReason};
use pagelamp_core::calendar::assemble::{
    AlternativeDate, CalendarChange, CalendarConflict, ChangeCode, ConflictCode, DateKind,
    ProposedDate,
};
use pagelamp_core::calendar::candidates::{CalendarCandidate, CandidateLeftOut, CandidateReason};
use pagelamp_core::calendar::proposal::{AcceptedCalendar, CalendarProposal};
use pagelamp_core::calendar::validate::{DateEvidence, DropCount, DropReason};
use pagelamp_core::calendar::{CalendarWeek, CourseCalendar};
use pagelamp_core::model::{
    AiLabel, AiMaterialsState, AiPolicy, BreakKind, CalendarBreak, CalendarOrigin, CalendarStatus,
    Confidence, Course, CourseGroup, CourseLifecycle, CoursePhase, CourseTimeline, DateSpan,
    DownloadBlock, Event, EventKind, EvidenceCode, EvidenceItem, EvidenceParam, EvidenceSignal,
    LifecycleState, MaterialKind, Module, RejectReason, RejectedDates, SearchHit, SnoozeKind,
    SourceErrorKind, SourceKind, SourceRecord, StoreCounts, StoredStudyPlan, StudyPlan,
    StudyPlanItem, TeachingSegment, TermAnchorSource, TermResolution, TermSource, TextErrorKind,
    TextStatus,
};
use pagelamp_core::source::{CourseSyncSummary, SyncStage};
use pagelamp_core::views::{
    CourseCounts, CourseOverview, CourseSummary, Deadline, MaterialView, TextProblem,
    WeekMaterials, WeekNoteKind,
};

use crate::PageLampError;

// ---------------------------------------------------------------------------------------------
// Custom types
// ---------------------------------------------------------------------------------------------

/// A point in time (UTC). Swift: `Date`.
pub type Timestamp = chrono::DateTime<chrono::Utc>;
uniffi::custom_type!(Timestamp, SystemTime, {
    remote,
    lower: |time| SystemTime::from(time),
    try_lift: |time| Ok(Timestamp::from(time)),
});

/// A calendar date without a time zone. Swift: `String`, always "YYYY-MM-DD".
pub type IsoDate = chrono::NaiveDate;
uniffi::custom_type!(IsoDate, String, {
    remote,
    lower: |date| iso_date_to_string(date),
    try_lift: |text| Ok(iso_date_from_string(&text)?),
});

/// Free-form JSON (`SourceRecord.config`). Swift: `String` holding JSON text.
pub type JsonString = serde_json::Value;
uniffi::custom_type!(JsonString, String, {
    remote,
    lower: |value| value.to_string(),
    try_lift: |text| Ok(json_from_string(&text)?),
});

/// Environment variables (`McpLaunch.env`). Swift: `[String: String]`. UniFFI has no ordered
/// map; the order of environment variables carries no meaning.
pub type EnvMap = BTreeMap<String, String>;
uniffi::custom_type!(EnvMap, HashMap<String, String>, {
    remote,
    lower: |map| map.into_iter().collect(),
    try_lift: |map| Ok(map.into_iter().collect()),
});

/// Times by source id (`AppStatus.deadlines_synced_at`). Swift: `[String: Date]`.
pub type SyncTimes = BTreeMap<String, Timestamp>;
uniffi::custom_type!(SyncTimes, HashMap<String, Timestamp>, {
    remote,
    lower: |map| map.into_iter().collect(),
    try_lift: |map| Ok(map.into_iter().collect()),
});

/// "YYYY-MM-DD".
pub(crate) fn iso_date_to_string(date: IsoDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// Parses exactly "YYYY-MM-DD" (surrounding whitespace ignored); anything else is `Invalid`,
/// so a bad date from Swift throws `PageLampError.invalid`, not an internal error.
pub(crate) fn iso_date_from_string(text: &str) -> Result<IsoDate, PageLampError> {
    let trimmed = text.trim();
    let shaped = trimmed.len() == 10
        && trimmed.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        });
    shaped
        .then(|| IsoDate::parse_from_str(trimmed, "%Y-%m-%d").ok())
        .flatten()
        .ok_or_else(|| PageLampError::Invalid {
            message: format!("'{text}' is not a date (expected YYYY-MM-DD)"),
        })
}

pub(crate) fn json_from_string(text: &str) -> Result<JsonString, PageLampError> {
    serde_json::from_str(text).map_err(|err| PageLampError::Invalid {
        message: format!("not valid JSON: {err}"),
    })
}

// ---------------------------------------------------------------------------------------------
// pagelamp-core: model
// ---------------------------------------------------------------------------------------------

#[uniffi::remote(Enum)]
pub enum SourceKind {
    Canvas,
    Folder,
    Ical,
}

/// Why a source's last sync failed; UIs branch on this, never on the message.
#[uniffi::remote(Enum)]
pub enum SourceErrorKind {
    AuthExpiredOrRevoked,
    Network,
    NotFound,
    RateLimited,
    Other,
}

/// A configured source. `config` is JSON text: Canvas `{base_url, account_name}`, folder
/// `{path, term_start?}`, iCal `{}`.
#[uniffi::remote(Record)]
pub struct SourceRecord {
    pub id: String,
    pub kind: SourceKind,
    pub label: String,
    pub config: JsonString,
    pub last_synced_at: Option<Timestamp>,
    pub last_error: Option<String>,
    pub last_error_kind: Option<SourceErrorKind>,
}

#[uniffi::remote(Enum)]
pub enum AiPolicy {
    Unknown,
    Prohibited,
    LearningAid,
    AllowedWithCitation,
    Unrestricted,
}

#[uniffi::remote(Record)]
pub struct Course {
    pub id: String,
    pub source_id: String,
    pub external_id: String,
    pub code: Option<String>,
    pub name: String,
    pub term_start: Option<IsoDate>,
    pub term_end: Option<IsoDate>,
    pub term_source: TermSource,
    pub url: Option<String>,
    pub ai_policy: AiPolicy,
    pub ai_policy_note: Option<String>,
    pub ai_access: bool,
    pub hidden: bool,
    pub enrollment_active: bool,
    pub updated_at: Timestamp,
}

#[uniffi::remote(Enum)]
pub enum TermSource {
    User,
    Synced,
    None,
}

#[uniffi::remote(Enum)]
pub enum AiMaterialsState {
    Readable,
    TurnedOff,
    WithheldByPolicy,
}

#[uniffi::remote(Record)]
pub struct Module {
    pub id: String,
    pub course_id: String,
    pub name: String,
    pub position: Option<i64>,
    pub unlock_at: Option<Timestamp>,
    pub week_hint: Option<u32>,
}

#[uniffi::remote(Enum)]
pub enum MaterialKind {
    File,
    Page,
    Announcement,
    Syllabus,
    ExternalLink,
}

#[uniffi::remote(Enum)]
pub enum TextStatus {
    Pending,
    Ok,
    Unsupported,
    NotDownloaded,
    Error,
}

#[uniffi::remote(Enum)]
pub enum DownloadBlock {
    Locked,
    TooLarge,
}

/// Why the extraction worker could not read a file.
#[uniffi::remote(Enum)]
pub enum TextErrorKind {
    TimedOut,
    CpuLimit,
    MemoryLimit,
    Crashed,
    BadOutput,
    SpawnFailed,
    ProtocolMismatch,
}

#[uniffi::remote(Enum)]
pub enum EventKind {
    AssignmentDue,
    QuizDue,
    Exam,
    ClassEvent,
    PlannerItem,
    Other,
}

/// `course_hint` is internal to syncing (never serialised by the facade); it is always nil in
/// values the facade returns.
#[uniffi::remote(Record)]
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
    pub course_hint: Option<String>,
}

#[uniffi::remote(Record)]
pub struct SearchHit {
    pub material_id: String,
    pub material_title: String,
    pub course_id: String,
    pub course_code: Option<String>,
    pub chunk_ord: u32,
    pub locator: Option<String>,
    pub snippet: String,
    pub url: Option<String>,
    pub week_hint: Option<u32>,
    pub score: f64,
}

#[uniffi::remote(Enum)]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[uniffi::remote(Record)]
pub struct CourseTimeline {
    pub as_of: IsoDate,
    pub current_week: Option<u32>,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
    pub current_module_ids: Vec<String>,
    pub outside_term: bool,
    pub phase: CoursePhase,
    pub phase_confidence: Confidence,
    pub starts_on: Option<IsoDate>,
    pub default_week: Option<u32>,
    pub break_after_week: Option<u32>,
    pub last_teaching_week: Option<u32>,
    pub current_break_kind: Option<BreakKind>,
    pub notes_week: Option<u32>,
    pub term: TermResolution,
    pub calendar: CalendarStatus,
    pub evidence_items: Vec<EvidenceItem>,
}

// ----- course calendar (pagelamp-core term) -----

#[uniffi::remote(Enum)]
pub enum CoursePhase {
    NotStarted,
    Teaching,
    Break,
    ExamPeriod,
    Ended,
    Unknown,
}

#[uniffi::remote(Enum)]
pub enum TermAnchorSource {
    StudentConfirmed,
    LmsCourseDates,
    LmsTerm,
    FolderConfig,
    InstitutionCalendar,
    PublishedWeekLabels,
    NoAnchor,
}

#[uniffi::remote(Enum)]
pub enum RejectReason {
    LongerThanTeachingTerm,
    ShorterThanTeachingTerm,
    StartsLongBeforeActivity,
    StartsBeforeSessionWindow,
    EndOutsideSessionWindow,
    StartsAfterEnd,
    ConflictsWithStrongerSource,
}

#[uniffi::remote(Record)]
pub struct RejectedDates {
    pub source: TermAnchorSource,
    pub start: Option<IsoDate>,
    pub end: Option<IsoDate>,
    pub reason: RejectReason,
    pub end_only: bool,
}

#[uniffi::remote(Enum)]
pub enum BreakKind {
    ReadingWeek,
    Holiday,
    WinterBreak,
    Other,
}

#[uniffi::remote(Enum)]
pub enum CalendarOrigin {
    User,
    Legacy,
    Scan,
    Ai,
    AiApp,
    Restored,
}

#[uniffi::remote(Record)]
pub struct AiLabel {
    pub backend_label: String,
    pub model: String,
    pub created_at: Timestamp,
}

#[uniffi::remote(Record)]
pub struct TeachingSegment {
    pub first_class: IsoDate,
    pub last_class: Option<IsoDate>,
    pub first_week_number: u32,
}

#[uniffi::remote(Record)]
pub struct DateSpan {
    pub start: IsoDate,
    pub end: IsoDate,
}

#[uniffi::remote(Record)]
pub struct CalendarBreak {
    pub kind: BreakKind,
    pub span: DateSpan,
    pub numbered: bool,
    pub label: String,
}

#[uniffi::remote(Enum)]
pub enum CalendarStatus {
    NoCalendar,
    Proposed,
    Accepted,
    AcceptedStale,
}

#[uniffi::remote(Record)]
pub struct TermResolution {
    pub week_one_monday: Option<IsoDate>,
    pub teaching: Vec<TeachingSegment>,
    pub breaks: Vec<CalendarBreak>,
    pub exams_end: Option<IsoDate>,
    pub anchor: TermAnchorSource,
    pub anchor_confidence: Confidence,
    pub anchor_origin: Option<CalendarOrigin>,
    pub ai_label: Option<AiLabel>,
    pub outer_frame: Option<DateSpan>,
    pub not_used: Vec<RejectedDates>,
    pub student_start: Option<IsoDate>,
    pub student_end: Option<IsoDate>,
}

#[uniffi::remote(Record)]
pub struct EvidenceParam {
    pub key: String,
    pub value: String,
}

#[uniffi::remote(Record)]
pub struct EvidenceItem {
    pub code: String,
    pub params: Vec<EvidenceParam>,
}

/// Every `EvidenceItem.code` (the item carries the code as a string; this enum lets Swift check
/// its translations are complete).
#[uniffi::remote(Enum)]
pub enum EvidenceCode {
    StudentDates,
    LegacyDates,
    StudentEndUsed,
    LmsCourseDates,
    LmsTermDates,
    FolderDates,
    InstitutionCalendar,
    WeekLabelsFit,
    NoCourseDates,
    TermLooksLikeEnrollmentWindow,
    DatesNotUsed,
    EndNotUsed,
    DatesAgree,
    DatesMayBeWrong,
    SessionWindow,
    WeekFromDates,
    WeekFromModuleUnlock,
    WeekFromRecentMaterials,
    WeekFromLatestMaterial,
    SignalAgrees,
    SignalDisagrees,
    ModulesReleasedTogether,
    UnlockTooOld,
    UnlockWithoutWeek,
    BulkPublish,
    NotesAhead,
    CalendarDisagreesWithNotes,
    NumberingOffset,
    BreaksUnknown,
    NoWeekSignal,
    StartsOn,
    InBreak,
    NoClassToday,
    ExamPeriod,
    ExamPeriodEstimated,
    EndedOn,
    StartTooOld,
    KeptCurrent,
    LmsConcluded,
    LmsCompleted,
    NoLongerListed,
    ExamsOver,
    CourseEndPassed,
    DatesEnded,
    TermEndPassed,
    SessionEnded,
    QuietSince,
    NoActivity,
    RecentActivity,
    NextEvent,
    SessionStarts,
    NoDatesInactive,
    MayHaveEnded,
    RemovalSnoozed,
    RemovalKept,
}

#[uniffi::remote(Enum)]
pub enum EvidenceSignal {
    ModuleUnlock,
    Dates,
    RecentMaterials,
    LatestMaterial,
}

// ----- course lifecycle (pagelamp-core lifecycle) -----

#[uniffi::remote(Enum)]
pub enum LifecycleState {
    Upcoming,
    Current,
    Finishing,
    Ended,
    Inactive,
    Unknown,
}

#[uniffi::remote(Enum)]
pub enum CourseGroup {
    Current,
    Upcoming,
    Past,
}

#[uniffi::remote(Enum)]
pub enum SnoozeKind {
    NotNow,
    Keep,
}

#[uniffi::remote(Record)]
pub struct CourseLifecycle {
    pub state: LifecycleState,
    pub group: CourseGroup,
    pub confidence: Confidence,
    pub since: Option<IsoDate>,
    pub starts_on: Option<IsoDate>,
    pub last_activity: Option<IsoDate>,
    pub next_event: Option<IsoDate>,
    pub evidence_items: Vec<EvidenceItem>,
    pub suggest_removal: bool,
    pub kept_current_until: Option<IsoDate>,
}

#[uniffi::remote(Record)]
pub struct StoreCounts {
    pub courses: u32,
    pub hidden_courses: u32,
    pub modules: u32,
    pub materials: u32,
    pub indexed_materials: u32,
    pub chunks: u32,
    pub events: u32,
    pub study_plans: u32,
}

#[uniffi::remote(Record)]
pub struct StudyPlanItem {
    pub date: IsoDate,
    pub course_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub material_ids: Vec<String>,
    pub minutes: Option<u32>,
    pub done: bool,
}

#[uniffi::remote(Record)]
pub struct StudyPlan {
    pub horizon_start: IsoDate,
    pub horizon_end: IsoDate,
    pub items: Vec<StudyPlanItem>,
    pub notes: Option<String>,
}

#[uniffi::remote(Record)]
pub struct StoredStudyPlan {
    pub id: i64,
    pub created_at: Timestamp,
    pub plan: StudyPlan,
}

// ---------------------------------------------------------------------------------------------
// pagelamp-core: views and source
// ---------------------------------------------------------------------------------------------

/// A deadline: the event (flattened in JSON, nested here) plus its course's code and name.
#[uniffi::remote(Record)]
pub struct Deadline {
    pub event: Event,
    pub course_code: Option<String>,
    pub course_name: Option<String>,
}

#[uniffi::remote(Record)]
pub struct CourseCounts {
    pub modules: u32,
    pub materials: u32,
    pub indexed_materials: u32,
    pub upcoming_deadlines: u32,
}

#[uniffi::remote(Record)]
pub struct CourseSummary {
    pub course: Course,
    pub ai_materials: AiMaterialsState,
    pub timeline: CourseTimeline,
    pub lifecycle: CourseLifecycle,
    pub counts: CourseCounts,
    pub next_deadline: Option<Deadline>,
    pub source_label: String,
    pub last_synced_at: Option<Timestamp>,
    #[uniffi(default = None)]
    pub deadlines_synced_at: Option<Timestamp>,
    #[uniffi(default)]
    pub structure_pending: bool,
}

#[uniffi::remote(Record)]
pub struct MaterialView {
    pub id: String,
    pub course_id: String,
    pub title: String,
    pub kind: MaterialKind,
    pub module_id: Option<String>,
    pub module_name: Option<String>,
    pub week_hint: Option<u32>,
    pub published_at: Option<Timestamp>,
    pub url: Option<String>,
    pub text_status: TextStatus,
    pub text_error: Option<String>,
    pub download_blocked: Option<DownloadBlock>,
    pub chunk_count: u32,
    #[uniffi(default)]
    pub text_problem: Option<TextProblem>,
}

/// Why a material's text can't be read.
#[uniffi::remote(Enum)]
pub enum TextProblem {
    NoText,
    TooLarge,
    PasswordProtected,
    Malformed,
    TimedOut,
    CpuLimit,
    MemoryLimit,
    Crashed,
    BadOutput,
    SpawnFailed,
    ProtocolMismatch,
    Other,
}

#[uniffi::remote(Record)]
pub struct CourseOverview {
    pub course: Course,
    pub ai_materials: AiMaterialsState,
    pub timeline: CourseTimeline,
    pub lifecycle: CourseLifecycle,
    pub current_modules: Vec<Module>,
    pub recent_materials: Vec<MaterialView>,
    pub upcoming_deadlines: Vec<Deadline>,
    pub recent_announcements: Vec<MaterialView>,
    pub source_label: String,
    pub last_synced_at: Option<Timestamp>,
    pub downloadable_files: u32,
    #[uniffi(default = None)]
    pub deadlines_synced_at: Option<Timestamp>,
    #[uniffi(default)]
    pub structure_pending: bool,
}

#[uniffi::remote(Enum)]
pub enum WeekNoteKind {
    CurrentWeekUnknown,
    OutsideTerm,
    NoMaterialsThisWeek,
    ExamPeriod,
    Break,
}

#[uniffi::remote(Record)]
pub struct WeekMaterials {
    pub course: Course,
    pub ai_materials: AiMaterialsState,
    pub week: Option<u32>,
    pub requested_week: Option<u32>,
    pub timeline: CourseTimeline,
    pub modules: Vec<Module>,
    pub materials: Vec<MaterialView>,
    pub available_weeks: Vec<u32>,
    pub note: Option<String>,
    pub note_kind: Option<WeekNoteKind>,
}

#[uniffi::remote(Record)]
pub struct CourseSyncSummary {
    pub course: String,
    pub modules: u32,
    pub pages: u32,
    pub files: u32,
    pub events: u32,
    pub warnings: u32,
}

// ---------------------------------------------------------------------------------------------
// pagelamp-app: status, sync, "connect your AI app"
// ---------------------------------------------------------------------------------------------

#[uniffi::remote(Record)]
pub struct AppStatus {
    pub version: String,
    pub data_dir: String,
    pub db_path: String,
    pub sources: Vec<SourceRecord>,
    pub counts: StoreCounts,
    pub last_synced_at: Option<Timestamp>,
    pub sync_in_progress: bool,
    pub auto_sync: AutoSync,
    pub deadlines_synced_at: SyncTimes,
}

/// How often PageLamp syncs by itself while it runs.
#[uniffi::remote(Enum)]
pub enum AutoSync {
    Off,
    Daily,
    TwiceDaily,
}

/// The student's sync settings.
#[uniffi::remote(Record)]
pub struct SyncPrefs {
    pub auto_sync: AutoSync,
}

/// Why PageLamp starts a sync by itself.
#[uniffi::remote(Enum)]
pub enum AutoSyncTrigger {
    Unattended,
    Attended,
}

/// Whether an automatic sync is due, by trigger.
#[uniffi::remote(Record)]
pub struct SyncDue {
    pub unattended: bool,
    pub attended: bool,
}

/// Options for a sync run; `SyncRequest()` in Swift equals the facade's `SyncRequest::default()`
/// (checked by `tests::sync_request_defaults_match_the_facade` and, through the generated
/// initialiser, by the Swift tests against `defaultSyncRequest()`).
#[uniffi::remote(Record)]
pub struct SyncRequest {
    #[uniffi(default)]
    pub download_files: bool,
    #[uniffi(default = 50)]
    pub max_file_mb: u32,
    #[uniffi(default)]
    pub only_courses: Vec<String>,
    #[uniffi(default = None)]
    pub automatic: Option<AutoSyncTrigger>,
}

/// What a sync step is doing (translate it; `SyncEvent.progress`'s message is English).
#[uniffi::remote(Enum)]
pub enum SyncStage {
    CheckingAccess,
    ListingCourses,
    ReadingCourse,
    DownloadingFiles,
    ScanningFiles,
    IndexingFiles,
    DownloadingFeed,
    SavingEvents,
}

/// Progress of a sync run, delivered to `SyncObserver.on_event`.
#[uniffi::remote(Enum)]
pub enum SyncEvent {
    SourceStarted {
        source_id: String,
        label: String,
    },
    Progress {
        source_id: String,
        message: String,
        current: Option<u32>,
        total: Option<u32>,
        stage: Option<SyncStage>,
        course: Option<String>,
    },
    Warning {
        source_id: String,
        message: String,
    },
    SourceFinished {
        source_id: String,
        ok: bool,
        error: Option<String>,
        error_kind: Option<SourceErrorKind>,
    },
}

#[uniffi::remote(Record)]
pub struct SourceSyncResult {
    pub source_id: String,
    pub label: String,
    pub kind: SourceKind,
    pub ok: bool,
    pub error: Option<String>,
    pub error_kind: Option<SourceErrorKind>,
    pub started_at: Timestamp,
    pub finished_at: Timestamp,
    pub courses: u32,
    pub modules: u32,
    pub materials: u32,
    pub files_downloaded: u32,
    pub files_indexed: u32,
    pub events: u32,
    pub warnings: Vec<String>,
    pub course_summaries: Vec<CourseSyncSummary>,
    pub requests: Option<u32>,
}

#[uniffi::remote(Record)]
pub struct SyncSummary {
    pub started_at: Timestamp,
    pub finished_at: Timestamp,
    pub ok: bool,
    pub results: Vec<SourceSyncResult>,
}

#[uniffi::remote(Enum)]
pub enum McpClient {
    ClaudeDesktop,
    ClaudeCode,
    Codex,
    Generic,
}

#[uniffi::remote(Enum)]
pub enum InstallKind {
    JsonSnippet,
    ShellCommand,
    TomlSnippet,
}

#[uniffi::remote(Record)]
pub struct McpLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub env: EnvMap,
    pub temporary_location: Option<TemporaryLocation>,
}

#[uniffi::remote(Enum)]
pub enum TemporaryLocation {
    DiskImage,
    Translocated,
    AppImage,
}

#[uniffi::remote(Record)]
pub struct McpClientConfig {
    pub client: McpClient,
    pub title: String,
    pub install_kind: InstallKind,
    pub config_path_hint: Option<String>,
    pub content: String,
    pub notes: Vec<String>,
    pub note_codes: Vec<McpNoteCode>,
    pub launch: McpLaunch,
}

#[uniffi::remote(Enum)]
pub enum McpNoteCode {
    WorksOnAllClaudePlans,
    AdminsMayDisableExtensions,
    NeedsPaidClaudePlan,
    CodexConfigSharedWithChatgptDesktop,
    CodexPlusAndEduDocumented,
    FreeGoUndocumented,
    RestartClientAfterChange,
    QuitBeforeEditing,
    CustomDataDir,
    GenericStdioClient,
    RunFromTemporaryLocation,
}

// ---------------------------------------------------------------------------------------------
// pagelamp-app: diagnostics
// ---------------------------------------------------------------------------------------------

#[uniffi::remote(Enum)]
pub enum ProcessKind {
    App,
    Mcp,
}

#[uniffi::remote(Record)]
pub struct CrashReport {
    pub time: Timestamp,
    pub version: String,
    pub process: ProcessKind,
    pub message: String,
    pub location: Option<String>,
}

#[uniffi::remote(Record)]
pub struct DoctorSource {
    pub kind: SourceKind,
    pub ok: bool,
    pub last_synced_at: Option<Timestamp>,
    pub last_error_kind: Option<SourceErrorKind>,
}

#[uniffi::remote(Record)]
pub struct McpClientPresence {
    pub claude_desktop: bool,
    pub claude_code: bool,
    pub codex: bool,
}

#[uniffi::remote(Record)]
pub struct DoctorReport {
    pub version: String,
    pub os: String,
    pub arch: String,
    pub data_dir: String,
    pub logs_dir: String,
    pub schema_version: Option<i64>,
    pub database_error: Option<String>,
    pub keychain_available: bool,
    pub keychain_error: Option<String>,
    pub sources: Vec<DoctorSource>,
    pub courses: u32,
    pub hidden_courses: u32,
    pub materials: u32,
    pub events: u32,
    pub mcp_clients: McpClientPresence,
    pub last_crash: Option<CrashReport>,
    pub extract_worker: ExtractWorkerCheck,
    pub unreadable_files: Vec<UnreadableFiles>,
}

/// Whether the extraction worker works (`spawn_failed`: blocked by antivirus or Smart App
/// Control, or missing).
#[uniffi::remote(Enum)]
pub enum ExtractWorkerStatus {
    Ok,
    NotSet,
    SpawnFailed,
    ProtocolMismatch,
    Failed,
}

#[uniffi::remote(Record)]
pub struct ExtractWorkerCheck {
    pub status: ExtractWorkerStatus,
    pub spawn_ms: Option<u32>,
}

#[uniffi::remote(Record)]
pub struct UnreadableFiles {
    pub kind: TextErrorKind,
    pub count: u32,
}

// ----- updates and activity (v0.3 M0.4; `PageLamp::startup_tasks` and friends) ----------------

#[uniffi::remote(Enum)]
pub enum UpdateChannel {
    Stable,
    Beta,
}

#[uniffi::remote(Record)]
pub struct UpdatePrefs {
    pub auto_check: bool,
    pub channel: Option<UpdateChannel>,
}

#[uniffi::remote(Enum)]
pub enum WhatsNewTopic {
    UpdateCheck,
    CourseWeeks,
    AutoSync,
}

#[uniffi::remote(Record)]
pub struct WhatsNew {
    pub since: Option<String>,
    pub topics: Vec<WhatsNewTopic>,
}

#[uniffi::remote(Record)]
pub struct StartupTasks {
    pub whats_new: Option<WhatsNew>,
    pub update_check_due: bool,
    pub updated_from: Option<String>,
    pub sync_due: SyncDue,
}

#[uniffi::remote(Enum)]
pub enum UpdateCheckOutcome {
    UpToDate,
    Available { version: String },
    Error { code: String },
}

#[uniffi::remote(Record)]
pub struct UpdateCheckRecord {
    pub at: Timestamp,
    pub channel: UpdateChannel,
    pub outcome: UpdateCheckOutcome,
}

#[uniffi::remote(Enum)]
pub enum ActivityKind {
    Sync,
    Download,
}

#[uniffi::remote(Record)]
pub struct ActivityItem {
    pub kind: ActivityKind,
    pub source_id: Option<String>,
    pub started_at: Timestamp,
}

#[uniffi::remote(Record)]
pub struct Activity {
    pub items: Vec<ActivityItem>,
    pub other_process_syncing: bool,
}

// ----- AI (v0.3 M1) ---------------------------------------------------------------------------

#[uniffi::remote(Enum)]
pub enum AiFeature {
    StudyPlan,
    WeeklyExplanation,
    WeeklyNote,
    CourseCalendar,
}

/// Why a model call was refused before anything was sent.
#[uniffi::remote(Enum)]
pub enum BlockReason {
    CoursePolicyProhibited,
    CourseAiTurnedOff,
    CourseHidden,
    NoReadableMaterials,
    MaterialSharingNotAllowed,
    CodingPlanKey,
    DisclosureNotAcknowledged,
    NoModelChosen,
    BudgetReached,
    PriceUnknownNotAcknowledged,
    WeeklyRunCapReached,
    BackendDisabledInThisBuild,
}

/// Why a model call failed.
#[uniffi::remote(Enum)]
pub enum ModelErrorKind {
    NotSignedIn,
    AuthRejected,
    BillingOrQuota,
    UsageLimit,
    RateLimited,
    Overloaded,
    InvalidRequest,
    ModelNotFound,
    ContextTooLong,
    Refused,
    ContentFiltered,
    Network,
    Timeout,
    BadOutput,
    RuntimeMissing,
    RuntimeVerifyFailed,
    RuntimeOutdated,
    Unsupported,
}

#[uniffi::remote(Enum)]
pub enum Effort {
    Lowest,
    Low,
    Medium,
    High,
}

#[uniffi::remote(Enum)]
pub enum MaterialSharing {
    Unanswered,
    Allowed,
    NotSure,
    NotAllowed,
}

#[uniffi::remote(Enum)]
pub enum BackendRef {
    Codex,
    ClaudeCode,
    Provider { provider_id: String },
}

#[uniffi::remote(Record)]
pub struct ModelChoice {
    pub backend: BackendRef,
    pub model: String,
    pub effort: Effort,
}

#[uniffi::remote(Record)]
pub struct FeatureRouting {
    pub feature: AiFeature,
    pub choice: Option<ModelChoice>,
}

#[uniffi::remote(Enum)]
pub enum BackendKind {
    ApiKey,
    Local,
    Codex,
    ClaudeCode,
}

#[uniffi::remote(Enum)]
pub enum BackendState {
    Ready,
    NeedsSetup,
    NeedsDisclosure,
    Unavailable,
}

#[uniffi::remote(Enum)]
pub enum BackendProblem {
    KeyMissing,
    ServerNotRunning,
    ModelMissing,
    DisclosureChanged,
}

#[uniffi::remote(Record)]
pub struct AiBackendStatus {
    pub backend: BackendRef,
    pub label: String,
    pub kind: BackendKind,
    pub state: BackendState,
    pub problems: Vec<BackendProblem>,
    pub disclosure: DisclosureFacts,
    pub disclosure_acknowledged: Option<u32>,
}

#[uniffi::remote(Record)]
pub struct AiStatus {
    pub backends: Vec<AiBackendStatus>,
    pub providers: Vec<ModelProviderRecord>,
    pub features: Vec<FeatureRouting>,
    pub budget: BudgetStatus,
}

#[uniffi::remote(Record)]
pub struct BudgetStatus {
    pub monthly_micro_usd: Option<u64>,
    pub spent_micro_usd: u64,
    pub warn_at_percent: u8,
}

#[uniffi::remote(Record)]
pub struct DisclosureFacts {
    pub version: u32,
    pub sends: Vec<SentData>,
    pub recipient: Recipient,
    pub training: TrainingFact,
    pub retention: RetentionFact,
    pub admin_visibility: bool,
    pub min_age: Option<u8>,
    pub guardian_permission: bool,
    pub cost: CostKind,
    pub on_device: bool,
    pub location: Option<String>,
}

#[uniffi::remote(Enum)]
pub enum SentData {
    Structure,
    MaterialText,
}

#[uniffi::remote(Record)]
pub struct Recipient {
    pub name: String,
    pub terms_url: Option<String>,
}

#[uniffi::remote(Enum)]
pub enum TrainingFact {
    NoTraining,
    MayTrain { how_to_turn_off_url: Option<String> },
    MayTrainFreeTier,
    Unknown,
}

#[uniffi::remote(Enum)]
pub enum RetentionFact {
    NotStored,
    StoredDays { days: u32 },
    ProviderTerms,
    OnDevice,
}

#[uniffi::remote(Enum)]
pub enum CostKind {
    ApiBilling,
    PlanCredits,
    FreeOnDevice,
    CloudViaLocal,
}

#[uniffi::remote(Enum)]
pub enum ProviderWire {
    OpenaiResponses,
    OpenaiChat,
    AnthropicMessages,
    OllamaNative,
}

#[uniffi::remote(Record)]
pub struct ProviderPreset {
    pub id: String,
    pub label: String,
    pub wire: ProviderWire,
    pub default_base_url: Option<String>,
    pub needs_key: bool,
    pub base_url_editable: bool,
    pub local: bool,
    pub data_policy: DisclosureFacts,
}

#[uniffi::remote(Record)]
pub struct ModelProviderRecord {
    pub provider_id: String,
    pub preset: String,
    pub label: String,
    pub wire: ProviderWire,
    pub base_url: String,
    pub key_last4: Option<String>,
    pub on_device: bool,
    pub created_at: Timestamp,
}

#[uniffi::remote(Enum)]
pub enum LocalServerKind {
    Ollama,
    LmStudio,
}

#[uniffi::remote(Record)]
pub struct LocalServer {
    pub kind: LocalServerKind,
    pub base_url: String,
    pub running: bool,
}

#[uniffi::remote(Record)]
pub struct ModelInfo {
    pub id: String,
    pub label: Option<String>,
    pub on_device: bool,
    pub runs_in_cloud: bool,
    pub price_known: bool,
    pub context_window: Option<u32>,
    pub reasoning_always_on: bool,
    pub suggested_for: Vec<AiFeature>,
}

#[uniffi::remote(Enum)]
pub enum StructuredOutputTier {
    NativeSchema,
    JsonObject,
    PromptOnly,
}

#[uniffi::remote(Record)]
pub struct ProbeReport {
    pub ok: bool,
    pub latency_ms: u32,
    pub structured_output_tier: Option<StructuredOutputTier>,
    pub thinking_always_on: bool,
    pub error: Option<ModelErrorKind>,
}

#[uniffi::remote(Enum)]
pub enum EstimateRequest {
    StudyPlan {
        horizon_days: Option<u32>,
        courses: Vec<String>,
    },
    WeeklyExplanation {
        course: String,
        week: Option<u32>,
    },
    WeeklyNote,
    CourseCalendar {
        courses: Vec<String>,
    },
}

#[uniffi::remote(Record)]
pub struct CostEstimate {
    pub micro_usd_upper: Option<u64>,
    pub input_tokens: u64,
    pub max_output_tokens: u64,
    pub reasoning_allowance: u64,
    pub repair_possible: bool,
    pub price_known: bool,
    pub would_block: Option<BlockReason>,
}

#[uniffi::remote(Record)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: Option<u64>,
}

#[uniffi::remote(Enum)]
pub enum CostBasis {
    Priced,
    FreeOnDevice,
    Unpriced,
    Plan,
}

#[uniffi::remote(Record)]
pub struct UsageRow {
    pub backend_label: String,
    pub model: String,
    pub feature: AiFeature,
    pub runs: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_basis: CostBasis,
    pub micro_usd: Option<u64>,
    pub estimated: bool,
}

#[uniffi::remote(Record)]
pub struct UsageSummary {
    pub month: IsoDate,
    pub rows: Vec<UsageRow>,
    pub total_micro_usd: u64,
    pub budget: BudgetStatus,
}

#[uniffi::remote(Record)]
pub struct RemoveAiDataReport {
    pub providers_removed: u32,
    pub generations_removed: u32,
    pub usage_rows_removed: u32,
    pub backup_removed: bool,
}

#[uniffi::remote(Enum)]
pub enum GenStage {
    BuildingContext,
    WaitingForModel,
    Validating,
    Repairing,
    Scheduling,
}

#[uniffi::remote(Enum)]
pub enum GenNoticeCode {
    ContextTrimmed,
    ThinkingAlwaysOn,
    JsonFallback,
    CoursesStructureOnly,
    MaterialsLeftOut,
    ApiKeyBilling,
    MaterialSharingReminder,
}

#[uniffi::remote(Enum)]
pub enum GenEvent {
    Started {
        generation_id: String,
        backend_label: String,
        model: String,
        on_device: bool,
    },
    Stage {
        stage: GenStage,
    },
    TextDelta {
        text: String,
    },
    Notice {
        code: GenNoticeCode,
    },
    Usage {
        usage: TokenUsage,
    },
    Finished {
        ok: bool,
    },
}

#[uniffi::remote(Record)]
pub struct GenerationMeta {
    pub generation_id: String,
    pub feature: AiFeature,
    pub backend_label: String,
    pub model: String,
    pub created_at: Timestamp,
    pub usage: TokenUsage,
    pub est_cost_micro_usd: Option<u64>,
    pub estimated: bool,
    pub context: ContextSummary,
    pub prompt_version: u32,
}

#[uniffi::remote(Record)]
pub struct ContextSummary {
    pub courses: Vec<ContextCourse>,
    pub materials_included: u32,
    pub materials_trimmed: u32,
    pub left_out: Vec<LeftOutMaterial>,
}

#[uniffi::remote(Record)]
pub struct ContextCourse {
    pub course_id: String,
    pub state: AiMaterialsState,
    pub text_included: bool,
}

#[uniffi::remote(Record)]
pub struct LeftOutMaterial {
    pub material_id: String,
    pub title: String,
    pub reason: LeftOutReason,
}

#[uniffi::remote(Enum)]
pub enum LeftOutReason {
    LooksLikeAssessment,
    ExternalLink,
    NoText,
    OverBudget,
}

// ---------------------------------------------------------------------------------------------
// pagelamp-app: course lifecycle
// ---------------------------------------------------------------------------------------------

#[uniffi::remote(Record)]
pub struct LifecycleSummary {
    pub courses: Vec<CourseLifecycleEntry>,
    pub suggested: Vec<String>,
    pub show_banner: bool,
    pub banner_snoozed_until: Option<IsoDate>,
}

#[uniffi::remote(Record)]
pub struct CourseLifecycleEntry {
    pub course_id: String,
    pub code: Option<String>,
    pub name: String,
    pub hidden: bool,
    pub lifecycle: CourseLifecycle,
}

// ---------------------------------------------------------------------------------------------
// pagelamp-app: removing courses, the dates form v2 (alpha.2 types)
// ---------------------------------------------------------------------------------------------

#[uniffi::remote(Enum)]
pub enum RemovalReason {
    Ended,
    Inactive,
    NotMine,
    Other,
}

#[uniffi::remote(Enum)]
pub enum LostAfterPurge {
    OldAnnouncements,
    LockedFiles,
    WholeCourse,
    RedownloadCountsAsViewing,
}

#[uniffi::remote(Record)]
pub struct BackupInfo {
    pub age_days: u32,
    pub delete_by_default: bool,
    pub reason_code: String,
}

#[uniffi::remote(Record)]
pub struct RemovalPreviewItem {
    pub course_id: String,
    pub code: Option<String>,
    pub name: String,
    pub source_kind: SourceKind,
    pub lifecycle: CourseLifecycle,
    pub materials: u32,
    pub downloaded_files: u32,
    pub downloaded_bytes: u64,
    pub deadlines: u32,
    pub generated_items: u32,
    pub custom_settings: bool,
    pub own_folder_untouched: bool,
    pub cannot_sync_again: bool,
    pub lost_after_purge: Vec<LostAfterPurge>,
}

#[uniffi::remote(Record)]
pub struct RemovalPreview {
    pub items: Vec<RemovalPreviewItem>,
    pub backup: Option<BackupInfo>,
}

#[uniffi::remote(Record)]
pub struct RemoveOptions {
    pub reason: Option<RemovalReason>,
    pub keep_downloaded_files: bool,
    pub purge_now: bool,
    pub delete_pre_update_backup: bool,
}

#[uniffi::remote(Enum)]
pub enum TombstoneState {
    Pending,
    Purged,
    Restoring,
}

#[uniffi::remote(Record)]
pub struct RemovedCourse {
    pub removed_id: String,
    pub source_id: String,
    pub source_kind: SourceKind,
    pub external_id: String,
    pub course_id: String,
    pub code: Option<String>,
    pub name: String,
    pub reason: RemovalReason,
    pub state: TombstoneState,
    pub removed_at: Timestamp,
    pub purge_after: Option<Timestamp>,
    pub purged_at: Option<Timestamp>,
    pub purge_in_days: Option<u32>,
    pub keep_files: bool,
    pub files_pending: bool,
}

#[uniffi::remote(Record)]
pub struct RemovalReport {
    pub removed: Vec<RemovedCourse>,
    pub purged_now: bool,
    pub backup_deleted: bool,
}

#[uniffi::remote(Enum)]
pub enum RestoreFailure {
    NotListed,
    AccessRestricted,
    Offline,
    Other,
}

#[uniffi::remote(Record)]
pub struct RestoreOutcome {
    pub restored: bool,
    pub course_id: Option<String>,
    pub failure: Option<RestoreFailure>,
}

#[uniffi::remote(Record)]
pub struct PurgeReport {
    pub purged: Vec<String>,
    pub files_pending: Vec<String>,
}

#[uniffi::remote(Record)]
pub struct CourseDatesInput {
    pub first_class: Option<IsoDate>,
    pub last_class: Option<IsoDate>,
    pub exams_end: Option<IsoDate>,
    pub breaks: Vec<BreakInput>,
    pub second_segment: Option<SegmentInput>,
}

#[uniffi::remote(Record)]
pub struct BreakInput {
    pub kind: BreakKind,
    pub start: IsoDate,
    pub end: IsoDate,
    pub numbered: bool,
    pub label: Option<String>,
}

#[uniffi::remote(Record)]
pub struct SegmentInput {
    pub first_class: IsoDate,
    pub last_class: Option<IsoDate>,
    pub restart_numbering: bool,
}

// ---------------------------------------------------------------------------------------------
// Course calendar proposals (v0.3 F3; calendar design §4, §7)
// ---------------------------------------------------------------------------------------------

/// The kind of a facade error, where a record carries one (`CalendarRunOutcome.error`); errors
/// themselves arrive as `PageLampError`.
#[uniffi::remote(Enum)]
pub enum AppErrorKind {
    Auth,
    Network,
    Invalid,
    NotFound,
    Ambiguous,
    Busy,
    SchemaTooNew,
    SchemaTooOld,
    Blocked,
    Model,
    Cancelled,
    Internal,
}

#[uniffi::remote(Record)]
pub struct CourseCalendar {
    pub segments: Vec<TeachingSegment>,
    pub breaks: Vec<CalendarBreak>,
    pub exam_period: Option<DateSpan>,
    pub final_exam_on: Option<IsoDate>,
    pub weeks: Vec<CalendarWeek>,
}

#[uniffi::remote(Record)]
pub struct CalendarWeek {
    pub number: u32,
    pub starts_on: IsoDate,
    pub topic: Option<String>,
}

#[uniffi::remote(Enum)]
pub enum DateKind {
    FirstClass,
    LastClass,
    BreakSpan,
    ExamPeriod,
    FinalExam,
    WeekStart,
}

#[uniffi::remote(Record)]
pub struct DateEvidence {
    pub material_id: String,
    pub title: String,
    pub locator: Option<String>,
    pub quote: Option<String>,
    pub url: Option<String>,
    pub derived: bool,
}

#[uniffi::remote(Record)]
pub struct AlternativeDate {
    pub date: IsoDate,
    pub end: Option<IsoDate>,
    pub label: String,
    pub evidence: Vec<DateEvidence>,
}

#[uniffi::remote(Record)]
pub struct ProposedDate {
    pub kind: DateKind,
    pub segment: u32,
    pub date: IsoDate,
    pub end: Option<IsoDate>,
    pub label: String,
    pub evidence: Vec<DateEvidence>,
    pub alternatives: Vec<AlternativeDate>,
    pub week: Option<u32>,
    pub break_kind: Option<BreakKind>,
    pub numbered: Option<bool>,
}

#[uniffi::remote(Enum)]
pub enum ConflictCode {
    SyllabusFromAnotherYear,
    Inconsistent,
    DisagreesWithNotes,
    DisagreesWithLmsDates,
    DisagreesWithClassEvent,
    DiffersFromInstitutionCalendar,
}

#[uniffi::remote(Record)]
pub struct CalendarConflict {
    pub code: ConflictCode,
    pub kind: DateKind,
    pub segment: u32,
    pub options: Vec<AlternativeDate>,
}

#[uniffi::remote(Enum)]
pub enum DropReason {
    UnknownSource,
    UnsupportedQuote,
    DateNotInQuote,
    AmbiguousYear,
    OutsideFrame,
    Inconsistent,
    TableDropped,
}

#[uniffi::remote(Record)]
pub struct DropCount {
    pub reason: DropReason,
    pub count: u32,
}

#[uniffi::remote(Enum)]
pub enum ChangeCode {
    NewCalendar,
    FirstClassMoved,
    LastClassMoved,
    BreakAdded,
    BreakRemoved,
    ExamsEndMoved,
    WeekTodayChanges,
    PhaseChanges,
}

#[uniffi::remote(Record)]
pub struct CalendarChange {
    pub code: ChangeCode,
    pub params: Vec<EvidenceParam>,
}

#[uniffi::remote(Record)]
pub struct CalendarProposal {
    pub id: i64,
    pub course_id: String,
    pub origin: CalendarOrigin,
    pub calendar: CourseCalendar,
    pub dates: Vec<ProposedDate>,
    pub conflicts: Vec<CalendarConflict>,
    pub dropped: Vec<DropCount>,
    pub low_quality: bool,
    pub passing: bool,
    pub ai_label: Option<AiLabel>,
    pub sharing_reminder: bool,
    pub resulting_week_today: Option<u32>,
    pub resulting_phase: CoursePhase,
    pub changes: Vec<CalendarChange>,
    pub created_at: Timestamp,
}

#[uniffi::remote(Record)]
pub struct AcceptedCalendar {
    pub id: i64,
    pub origin: CalendarOrigin,
    pub calendar: CourseCalendar,
    pub dates: Vec<ProposedDate>,
    pub ai_label: Option<AiLabel>,
    pub accepted_at: Timestamp,
    pub stale: bool,
    pub stale_since: Option<IsoDate>,
    pub changed_materials: Vec<String>,
}

#[uniffi::remote(Enum)]
pub enum CandidateReason {
    Syllabus,
    LinkedFromSyllabus,
    TitleOutline,
    TitleSchedule,
    TitleInfo,
    FrontPage,
    StartModule,
    Announcement,
    NamedInCourseToml,
    StudentAdded,
}

#[uniffi::remote(Enum)]
pub enum CandidateLeftOut {
    NoText,
    Scanned,
    OverBudget,
    ExcludedByStudent,
}

#[uniffi::remote(Record)]
pub struct CalendarCandidate {
    pub material_id: String,
    pub title: String,
    pub kind: MaterialKind,
    pub reason: CandidateReason,
    pub included: bool,
    pub student_choice: Option<bool>,
    pub has_text: bool,
    pub downloadable: bool,
    pub left_out: Option<CandidateLeftOut>,
    pub url: Option<String>,
}

#[uniffi::remote(Record)]
pub struct CourseCalendarView {
    pub course_id: String,
    pub accepted: Option<AcceptedCalendar>,
    pub proposals: Vec<CalendarProposal>,
    pub status: CalendarStatus,
    pub candidates: Vec<CalendarCandidate>,
    pub blocked: Option<BlockReason>,
}

#[uniffi::remote(Record)]
pub struct SyllabusOffer {
    pub course_id: String,
    pub reason_code: String,
    pub candidates: u32,
    pub has_text: bool,
}

#[uniffi::remote(Record)]
pub struct ReadCalendarOptions {
    #[uniffi(default)]
    pub override_budget: bool,
}

#[uniffi::remote(Record)]
pub struct CalendarRunOutcome {
    pub course_id: String,
    pub proposal_id: Option<i64>,
    pub passing: bool,
    pub blocked: Option<BlockReason>,
    pub error: Option<AppErrorKind>,
}

#[uniffi::remote(Enum)]
pub enum CalendarBatchEvent {
    CourseStarted {
        course_id: String,
        index: u32,
        total: u32,
    },
    Gen {
        course_id: String,
        event: GenEvent,
    },
    CourseFinished {
        outcome: CalendarRunOutcome,
    },
}
