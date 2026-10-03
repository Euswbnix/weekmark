//! "When does this course teach": which dates count weeks, which were not used and why
//! (docs/design/v0.3-course-calendar.md §6). Pure functions only; callers pass `today`.
//!
//! The types here are part of the facade and MCP contract through `CourseTimeline.term`
//! (re-exported from `model`). Every type is expressible in UniFFI: no tuples, no `Result`
//! fields, no recursion, no free-form JSON.

pub mod evidence;
pub(crate) mod fit;
pub mod institution;
pub mod phase;
mod resolve;
pub mod session;

pub use resolve::{
    MAX_DAYS_BEFORE_ACTIVITY, MAX_DAYS_BEFORE_WINDOW, MAX_END_DAYS_OUTSIDE_WINDOW, MAX_TERM_DAYS,
    MAX_YEAR_TERM_DAYS, MIN_TERM_DAYS, ResolvedTerm, TermInput, resolve_term,
};

/// Settings key: ids of the courses whose student dates were saved or confirmed in PageLamp
/// 0.3 or later. The student dates of any other course were set in 0.1 and never checked
/// since: their `anchor_origin` is `Legacy`, and the v4 migration turns only those into
/// `legacy` calendars.
pub const CONFIRMED_DATES_KEY: &str = "course_dates.confirmed";

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{Confidence, Timestamp};

/// Where a course is in its term (design §6.6). Every exclusion rule uses the lifecycle
/// (`CourseLifecycle`), never the phase: the phase only drives labels and the week.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CoursePhase {
    /// Today is before week 1.
    NotStarted,
    /// Inside a teaching segment, not in a break week.
    Teaching,
    /// Today's week is a break week (at least 3 weekdays inside a break).
    Break,
    /// After the last class, up to the end of exams (or last class + 21 days).
    ExamPeriod,
    Ended,
    /// No usable dates; the week (if any) comes from module unlocks and materials.
    Unknown,
}

impl CoursePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            CoursePhase::NotStarted => "not_started",
            CoursePhase::Teaching => "teaching",
            CoursePhase::Break => "break",
            CoursePhase::ExamPeriod => "exam_period",
            CoursePhase::Ended => "ended",
            CoursePhase::Unknown => "unknown",
        }
    }
}

/// Which dates set week 1 (design §6.4; the first available one wins).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TermAnchorSource {
    /// The student's dates (the course dates form, an accepted proposal, or a v0.1 override).
    StudentConfirmed,
    /// The LMS course's own start and end.
    LmsCourseDates,
    /// The LMS term's start and end.
    LmsTerm,
    /// `course.toml` or the folder source's configured term start.
    FolderConfig,
    /// Exact dates of the school's published calendar (from alpha.2).
    InstitutionCalendar,
    /// Week 1 fitted from the week numbers and publish dates of the professor's materials.
    PublishedWeekLabels,
    /// No usable dates ("none" in JSON; named so Swift's `.none` stays Optional's).
    #[serde(rename = "none")]
    NoAnchor,
}

impl TermAnchorSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TermAnchorSource::StudentConfirmed => "student_confirmed",
            TermAnchorSource::LmsCourseDates => "lms_course_dates",
            TermAnchorSource::LmsTerm => "lms_term",
            TermAnchorSource::FolderConfig => "folder_config",
            TermAnchorSource::InstitutionCalendar => "institution_calendar",
            TermAnchorSource::PublishedWeekLabels => "published_week_labels",
            TermAnchorSource::NoAnchor => "none",
        }
    }
}

/// Why dates were not used (design §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    /// Longer than a teaching term (26 weeks; 36 with full-year evidence).
    LongerThanTeachingTerm,
    /// Shorter than 4 weeks.
    ShorterThanTeachingTerm,
    /// Starts more than 28 days before the course's first activity.
    StartsLongBeforeActivity,
    /// Starts more than 14 days before the session window.
    StartsBeforeSessionWindow,
    /// Ends more than 28 days after the session window (or, for a full-year section, more than
    /// 28 days before it): only the end is not used.
    EndOutsideSessionWindow,
    StartsAfterEnd,
    ConflictsWithStrongerSource,
}

impl RejectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RejectReason::LongerThanTeachingTerm => "longer_than_teaching_term",
            RejectReason::ShorterThanTeachingTerm => "shorter_than_teaching_term",
            RejectReason::StartsLongBeforeActivity => "starts_long_before_activity",
            RejectReason::StartsBeforeSessionWindow => "starts_before_session_window",
            RejectReason::EndOutsideSessionWindow => "end_outside_session_window",
            RejectReason::StartsAfterEnd => "starts_after_end",
            RejectReason::ConflictsWithStrongerSource => "conflicts_with_stronger_source",
        }
    }
}

/// Dates a source offered that were not used, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RejectedDates {
    pub source: TermAnchorSource,
    pub start: Option<NaiveDate>,
    pub end: Option<NaiveDate>,
    pub reason: RejectReason,
    /// True when only the end was not used (the start still counts weeks).
    pub end_only: bool,
}

impl RejectedDates {
    /// An LMS term too long to be a teaching term: an enrollment window like UofT's May–January
    /// "Fall 2026" (calendar design §6.3), never used to count weeks.
    pub fn is_enrollment_window(&self) -> bool {
        self.source == TermAnchorSource::LmsTerm
            && self.reason == RejectReason::LongerThanTeachingTerm
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BreakKind {
    ReadingWeek,
    Holiday,
    WinterBreak,
    Other,
}

impl BreakKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BreakKind::ReadingWeek => "reading_week",
            BreakKind::Holiday => "holiday",
            BreakKind::WinterBreak => "winter_break",
            BreakKind::Other => "other",
        }
    }
}

/// Where a student-confirmed calendar came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CalendarOrigin {
    /// The student typed the dates (course dates form, CLI).
    User,
    /// Dates set in PageLamp 0.1 that the student has not confirmed since.
    Legacy,
    Scan,
    Ai,
    AiApp,
    Restored,
}

impl CalendarOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            CalendarOrigin::User => "user",
            CalendarOrigin::Legacy => "legacy",
            CalendarOrigin::Scan => "scan",
            CalendarOrigin::Ai => "ai",
            CalendarOrigin::AiApp => "ai_app",
            CalendarOrigin::Restored => "restored",
        }
    }
}

/// "AI-generated · <backend> · <model> · <date>" for dates an AI read (from alpha.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AiLabel {
    pub backend_label: String,
    pub model: String,
    pub created_at: Timestamp,
    /// The model ran on this computer ("on-device model — check the dates", §7.12).
    #[serde(default)]
    pub on_device: bool,
}

/// One stretch of teaching: one for a one-term course, two for a full-year course.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TeachingSegment {
    pub first_class: NaiveDate,
    /// The last day of classes, when known.
    pub last_class: Option<NaiveDate>,
    /// The number of the segment's first week (0 is allowed: "Week 0").
    pub first_week_number: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DateSpan {
    pub start: NaiveDate,
    pub end: NaiveDate,
}

impl DateSpan {
    pub fn contains(&self, date: NaiveDate) -> bool {
        self.start <= date && date <= self.end
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CalendarBreak {
    pub kind: BreakKind,
    pub span: DateSpan,
    /// True when the break counts in the week numbering.
    pub numbered: bool,
    /// Short label from the course material (material text, rule 8: never in structure
    /// outputs such as MCP or `evidence`). At most 80 characters.
    pub label: String,
}

/// Whether the course has a calendar the student accepted (from alpha.2; always `None` in
/// alpha.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CalendarStatus {
    /// No calendar ("none" in JSON; named so Swift's `.none` stays Optional's).
    #[serde(rename = "none")]
    NoCalendar,
    Proposed,
    Accepted,
    AcceptedStale,
}

impl CalendarStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            CalendarStatus::NoCalendar => "none",
            CalendarStatus::Proposed => "proposed",
            CalendarStatus::Accepted => "accepted",
            CalendarStatus::AcceptedStale => "accepted_stale",
        }
    }
}

/// The dates that count the course's weeks, and the dates that were not used (design §3.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TermResolution {
    /// Monday of teaching week 1 (weeks run Monday to Sunday).
    pub week_one_monday: Option<NaiveDate>,
    pub teaching: Vec<TeachingSegment>,
    pub breaks: Vec<CalendarBreak>,
    pub exams_end: Option<NaiveDate>,
    pub anchor: TermAnchorSource,
    /// Low when `anchor` is `NoAnchor`.
    pub anchor_confidence: Confidence,
    /// For `StudentConfirmed`: where the student's dates came from.
    pub anchor_origin: Option<CalendarOrigin>,
    /// Set when the accepted calendar came from an AI (from alpha.3).
    pub ai_label: Option<AiLabel>,
    /// Bounds the course lies within: the LMS term (plausible or not) ∪ the session window.
    /// Never used to count weeks.
    pub outer_frame: Option<DateSpan>,
    pub not_used: Vec<RejectedDates>,
    /// The student's own first/last day of classes as saved (the raw overrides, whichever
    /// are set). The dates form prefills from `teaching` and sends these back unchanged for
    /// the fields the student didn't edit.
    pub student_start: Option<NaiveDate>,
    pub student_end: Option<NaiveDate>,
}

impl Default for TermResolution {
    fn default() -> Self {
        TermResolution {
            week_one_monday: None,
            teaching: Vec::new(),
            breaks: Vec::new(),
            exams_end: None,
            anchor: TermAnchorSource::NoAnchor,
            anchor_confidence: Confidence::Low,
            anchor_origin: None,
            ai_label: None,
            outer_frame: None,
            not_used: Vec::new(),
            student_start: None,
            student_end: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_json_names_match_as_str() {
        for source in [
            TermAnchorSource::StudentConfirmed,
            TermAnchorSource::LmsCourseDates,
            TermAnchorSource::LmsTerm,
            TermAnchorSource::FolderConfig,
            TermAnchorSource::InstitutionCalendar,
            TermAnchorSource::PublishedWeekLabels,
            TermAnchorSource::NoAnchor,
        ] {
            assert_eq!(serde_json::to_value(source).unwrap(), source.as_str());
        }
        for status in [
            CalendarStatus::NoCalendar,
            CalendarStatus::Proposed,
            CalendarStatus::Accepted,
            CalendarStatus::AcceptedStale,
        ] {
            assert_eq!(serde_json::to_value(status).unwrap(), status.as_str());
        }
        assert_eq!(TermAnchorSource::NoAnchor.as_str(), "none");
        assert_eq!(CalendarStatus::NoCalendar.as_str(), "none");
        for phase in [
            CoursePhase::NotStarted,
            CoursePhase::Teaching,
            CoursePhase::Break,
            CoursePhase::ExamPeriod,
            CoursePhase::Ended,
            CoursePhase::Unknown,
        ] {
            assert_eq!(serde_json::to_value(phase).unwrap(), phase.as_str());
        }
    }
}
