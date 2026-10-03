//! Machine-readable evidence for the week, phase and lifecycle, so the desktop app, the Swift
//! shell and the CLI can translate it (design §3.3 `EvidenceItem`).
//!
//! An item is a code plus string parameters. Parameter keys tell a UI how to format the value:
//!
//! | key                                                   | value                           |
//! |-------------------------------------------------------|---------------------------------|
//! | `date`, `start`, `end`, `since`, `until`, `monday`, `*_on` | ISO date "YYYY-MM-DD"      |
//! | `week`, `weeks`, `days`, `offset`, `count`, `*_week`  | integer                         |
//! | `source`                                              | a `TermAnchorSource` value      |
//! | `kind`                                                | a `BreakKind` value             |
//! | `reason`                                              | a `RejectReason` value          |
//! | `signal`                                              | an `EvidenceSignal` value       |
//! | `term_name`, `title`, `session`, `section`            | plain text, shown verbatim      |
//!
//! Text parameters are structure (titles, the LMS term name, a session code), never material
//! text: no break labels, week topics or quotes (design §7.11, rule 8). Titles are still
//! instructor-written: they are cut at 80 characters with control characters removed, and
//! UIs render them as plain text only.
//!
//! Optional parameters are simply absent. `english()` gives the English sentence the CLI and
//! the `evidence` strings (MCP) use.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{BreakKind, RejectReason, TermAnchorSource};

/// Longest text parameter, in characters (the `MAX_TITLE_CHARS` rule of `timeline`).
pub const MAX_TEXT_PARAM_CHARS: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceParam {
    pub key: String,
    pub value: String,
}

/// One reason, as a code (an `EvidenceCode` value) and its parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EvidenceItem {
    pub code: String,
    pub params: Vec<EvidenceParam>,
}

/// Which week signal an evidence item talks about (`signal` parameter).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSignal {
    ModuleUnlock,
    /// The course dates (whatever `TermResolution.anchor` is).
    Dates,
    RecentMaterials,
    LatestMaterial,
}

impl EvidenceSignal {
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceSignal::ModuleUnlock => "module_unlock",
            EvidenceSignal::Dates => "dates",
            EvidenceSignal::RecentMaterials => "recent_materials",
            EvidenceSignal::LatestMaterial => "latest_material",
        }
    }
}

macro_rules! evidence_codes {
    ($( $(#[$doc:meta])* $variant:ident = $text:literal, )*) => {
        /// Every `EvidenceItem.code`. Exported in the facade schema so UIs can check their
        /// translations are complete; the item itself carries the code as a string.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum EvidenceCode {
            $( $(#[$doc])* $variant, )*
        }

        impl EvidenceCode {
            pub const ALL: &'static [EvidenceCode] = &[$(EvidenceCode::$variant),*];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(EvidenceCode::$variant => $text,)*
                }
            }

            pub fn parse(code: &str) -> Option<EvidenceCode> {
                match code {
                    $($text => Some(EvidenceCode::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

evidence_codes! {
    // ----- the dates that count weeks -----
    /// `start?`, `end?`: the student's first and last day of classes are used.
    StudentDates = "student_dates",
    /// `start?`, `end?`: dates the student set in PageLamp 0.1 are used; they should check them.
    LegacyDates = "legacy_dates",
    /// `end`: only the student's last day of classes is used; week 1 comes from other dates.
    StudentEndUsed = "student_end_used",
    /// `start`, `end?`: the LMS course's own dates are used.
    LmsCourseDates = "lms_course_dates",
    /// `term_name?`, `start`, `end?`: the LMS term's dates are used.
    LmsTermDates = "lms_term_dates",
    /// `start`, `end?`: the folder's configured dates are used.
    FolderDates = "folder_dates",
    /// `session`, `start`, `end`: the school's published calendar is used (from alpha.2).
    InstitutionCalendar = "institution_calendar",
    /// `monday`, `weeks`: week 1 fitted to the week of `monday` from the week numbers of
    /// materials posted over `weeks` different weeks.
    WeekLabelsFit = "week_labels_fit",
    /// No usable course dates.
    NoCourseDates = "no_course_dates",

    // ----- dates not used -----
    /// `term_name?`, `start`, `end`, `weeks`: the LMS term is far longer than a teaching term
    /// (an enrollment window), so it doesn't count weeks.
    TermLooksLikeEnrollmentWindow = "term_looks_like_enrollment_window",
    /// `source`, `start?`, `end?`, `reason`: these dates are not used.
    DatesNotUsed = "dates_not_used",
    /// `source`, `end`, `until?`, `reason`: this end is not used; `until` is used instead
    /// (the session window's end), when given.
    EndNotUsed = "end_not_used",

    // ----- cross-checks between date sources -----
    /// `source`, `monday`: another source agrees on week 1.
    DatesAgree = "dates_agree",
    /// `source`, `monday`, `days`: another source puts week 1 `days` days away: the dates may
    /// be wrong, check them.
    DatesMayBeWrong = "dates_may_be_wrong",
    /// `session`, `section?`, `start`, `end`: the UofT session code bounds the course (bounds
    /// only, never week counts).
    SessionWindow = "session_window",
    /// `session`: the school's calendar in this version of PageLamp has no dates for the
    /// session (or the course's campus), so only the session window bounds the course.
    InstitutionCalendarMissing = "institution_calendar_missing",

    // ----- the current week -----
    /// `week`, `monday`: counted from week 1 starting `monday`.
    WeekFromDates = "week_from_dates",
    /// `week`, `title`, `date`: the most recently unlocked module.
    WeekFromModuleUnlock = "week_from_module_unlock",
    /// `week`, `title`, `date`, `days`: the highest week among materials of the last `days`
    /// days.
    WeekFromRecentMaterials = "week_from_recent_materials",
    /// `week`, `title`, `date`, `days`: the latest week-numbered material (nothing in the last
    /// `days` days).
    WeekFromLatestMaterial = "week_from_latest_material",
    /// `signal`, `week`: a weaker signal agrees.
    SignalAgrees = "signal_agrees",
    /// `signal`, `week`, `chosen_week`: a weaker signal disagrees; the stronger one wins.
    SignalDisagrees = "signal_disagrees",
    /// `title`, `count`, `date`, `from_week`, `to_week`: modules of several weeks unlocked on
    /// one day, so unlocks don't show the week.
    ModulesReleasedTogether = "modules_released_together",
    /// `title`, `count`, `date`, `days`: the latest unlock is too old to show the week.
    UnlockTooOld = "unlock_too_old",
    /// `title`, `count`, `date`: the latest unlocked module has no week number.
    UnlockWithoutWeek = "unlock_without_week",
    /// `title`, `count`, `from_week`, `to_week`, `start`, `end`: materials of many weeks were
    /// published together, so publish dates don't show the week.
    BulkPublish = "bulk_publish",
    /// `week`: materials of week `week` are already posted (one week ahead counts as agreeing).
    NotesAhead = "notes_ahead",
    /// `week`, `notes_week`: the professor's recent materials are at `notes_week` (from alpha.2).
    CalendarDisagreesWithNotes = "calendar_disagrees_with_notes",
    /// `offset`: materials are numbered `offset` weeks from the course dates (from alpha.2).
    NumberingOffset = "numbering_offset",
    /// Reading weeks and breaks are not known, so the week can run ahead of the teaching week.
    BreaksUnknown = "breaks_unknown",
    /// Nothing names a current week.
    NoWeekSignal = "no_week_signal",

    // ----- the phase -----
    /// `date`: classes start on `date`.
    StartsOn = "starts_on",
    /// `kind`, `start`, `end`: a break (from alpha.2).
    InBreak = "in_break",
    /// `kind`: no class today (a holiday shorter than a break week; from alpha.2).
    NoClassToday = "no_class_today",
    /// `start`, `end`: after the last class, exam period until `end`.
    ExamPeriod = "exam_period",
    /// `days`: no exam dates known, so the exam period is taken as `days` days after the last
    /// class.
    ExamPeriodEstimated = "exam_period_estimated",
    /// `date`: the course ended on `date`.
    EndedOn = "ended_on",
    /// `start`, `weeks`: only a start date is known and it was `weeks` weeks ago, longer than a
    /// term, so it no longer counts weeks.
    StartTooOld = "start_too_old",

    // ----- the lifecycle -----
    /// `until`: the student marked the course current until `until`.
    KeptCurrent = "kept_current",
    /// The LMS says the course is concluded.
    LmsConcluded = "lms_concluded",
    /// The LMS marks the course completed.
    LmsCompleted = "lms_completed",
    /// The LMS no longer lists the course as active.
    NoLongerListed = "no_longer_listed",
    /// `date`: the exams (or classes) of the student's dates ended on `date`.
    ExamsOver = "exams_over",
    /// `source`, `end`: the course dates ended on `end`.
    CourseEndPassed = "course_end_passed",
    /// `date`: by the course dates, the course ended on `date`.
    DatesEnded = "dates_ended",
    /// `term_name?`, `end`: the LMS term ended on `end` (the term itself isn't used for weeks).
    TermEndPassed = "term_end_passed",
    /// `session`, `end`: the UofT session ended on `end`.
    SessionEnded = "session_ended",
    /// `date`, `days`: last activity on `date`, `days` days ago.
    QuietSince = "quiet_since",
    /// No activity at all.
    NoActivity = "no_activity",
    /// `date`: recent activity (e.g. grade announcements) on `date`.
    RecentActivity = "recent_activity",
    /// `date`, `title`: a course event is still ahead.
    NextEvent = "next_event",
    /// `session`, `start`: the UofT session starts on `start`.
    SessionStarts = "session_starts",
    /// `days`: no dates at all and no activity in `days` days.
    NoDatesInactive = "no_dates_inactive",
    /// Some signs say the course may have ended, not enough to move it to Past.
    MayHaveEnded = "may_have_ended",
    /// `until`: removal suggestions for this course are snoozed until `until`.
    RemovalSnoozed = "removal_snoozed",
    /// The student chose to keep this course: it is never suggested for removal.
    RemovalKept = "removal_kept",
}

impl EvidenceItem {
    pub fn new(code: EvidenceCode) -> Self {
        EvidenceItem {
            code: code.as_str().to_string(),
            params: Vec::new(),
        }
    }

    /// The code as an enum (None for a code this build doesn't know).
    pub fn evidence_code(&self) -> Option<EvidenceCode> {
        EvidenceCode::parse(&self.code)
    }

    /// The value of parameter `key`, if present.
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.value.as_str())
    }

    fn with(mut self, key: &str, value: String) -> Self {
        self.params.push(EvidenceParam {
            key: key.to_string(),
            value,
        });
        self
    }

    pub fn date(self, key: &str, date: NaiveDate) -> Self {
        self.with(key, date.format("%Y-%m-%d").to_string())
    }

    pub fn opt_date(self, key: &str, date: Option<NaiveDate>) -> Self {
        match date {
            Some(date) => self.date(key, date),
            None => self,
        }
    }

    pub fn number(self, key: &str, value: impl Into<i64>) -> Self {
        self.with(key, value.into().to_string())
    }

    /// A plain-text parameter, cleaned by `plain_text`.
    pub fn text(self, key: &str, text: &str) -> Self {
        self.with(key, plain_text(text))
    }

    pub fn opt_text(self, key: &str, text: Option<&str>) -> Self {
        match text {
            Some(text) => self.text(key, text),
            None => self,
        }
    }

    pub fn source(self, source: TermAnchorSource) -> Self {
        self.with("source", source.as_str().to_string())
    }

    pub fn reason(self, reason: RejectReason) -> Self {
        self.with("reason", reason.as_str().to_string())
    }

    pub fn kind(self, kind: BreakKind) -> Self {
        self.with("kind", kind.as_str().to_string())
    }

    pub fn signal(self, signal: EvidenceSignal) -> Self {
        self.with("signal", signal.as_str().to_string())
    }

    /// The English sentence for this item (CLI, `CourseTimeline.evidence`).
    pub fn english(&self) -> String {
        english(self)
    }
}

/// Control characters and line breaks → spaces; cut after `MAX_TEXT_PARAM_CHARS` characters
/// with "…" (characters, not bytes, so Chinese text stays intact).
pub fn plain_text(text: &str) -> String {
    let mut clean: String = text
        .trim()
        .chars()
        .take(MAX_TEXT_PARAM_CHARS)
        .map(|c| {
            if c.is_control() || c.is_whitespace() {
                ' '
            } else {
                c
            }
        })
        .collect();
    if text.trim().chars().nth(MAX_TEXT_PARAM_CHARS).is_some() {
        clean.push('…');
    }
    clean
}

/// English name of a date source, for sentences.
fn source_name(source: &str) -> &'static str {
    match source {
        "student_confirmed" => "your course dates",
        "lms_course_dates" => "the LMS course dates",
        "lms_term" => "the LMS term",
        "folder_config" => "the folder's dates",
        "institution_calendar" => "the school calendar",
        "published_week_labels" => "the week numbers of posted materials",
        _ => "no source",
    }
}

fn signal_name(signal: &str) -> &'static str {
    match signal {
        "module_unlock" => "module unlock dates",
        "dates" => "the course dates",
        "recent_materials" => "recent materials",
        "latest_material" => "the latest material",
        _ => "another signal",
    }
}

fn reason_text(reason: &str) -> &'static str {
    match reason {
        "longer_than_teaching_term" => "longer than a teaching term",
        "shorter_than_teaching_term" => "shorter than a teaching term",
        "starts_long_before_activity" => "starts long before the course's first activity",
        "starts_before_session_window" => "starts long before the session",
        "end_outside_session_window" => "ends far outside the session",
        "starts_after_end" => "starts after it ends",
        "conflicts_with_stronger_source" => "conflicts with better dates",
        _ => "not plausible",
    }
}

fn kind_name(kind: &str) -> &'static str {
    match kind {
        "reading_week" => "reading week",
        "holiday" => "holiday",
        "winter_break" => "winter break",
        _ => "break",
    }
}

fn english(item: &EvidenceItem) -> String {
    let p = |key: &str| item.param(key).unwrap_or("?").to_string();
    let opt = |key: &str| item.param(key).map(str::to_string);
    let span = || match (opt("start"), opt("end")) {
        (Some(start), Some(end)) => format!("{start} → {end}"),
        (Some(start), None) => format!("from {start}"),
        (None, Some(end)) => format!("until {end}"),
        (None, None) => "no dates".to_string(),
    };
    // " 'Fall 2026'" or nothing, to follow "term".
    let term = || match opt("term_name") {
        Some(name) => format!(" '{name}'"),
        None => String::new(),
    };
    let Some(code) = item.evidence_code() else {
        return item.code.clone();
    };
    match code {
        EvidenceCode::StudentDates => format!("using your course dates ({})", span()),
        EvidenceCode::LegacyDates => format!(
            "using the term dates you set in PageLamp 0.1 ({}): check them",
            span()
        ),
        EvidenceCode::StudentEndUsed => {
            format!("using your last day of classes ({})", p("end"))
        }
        EvidenceCode::LmsCourseDates => {
            format!("using the course dates from the LMS ({})", span())
        }
        EvidenceCode::LmsTermDates => format!("using the LMS term{} ({})", term(), span()),
        EvidenceCode::FolderDates => format!("using the folder's course dates ({})", span()),
        EvidenceCode::InstitutionCalendar => format!(
            "using the school calendar for session {} ({})",
            p("session"),
            span()
        ),
        EvidenceCode::WeekLabelsFit => format!(
            "week 1 is the week of {}, fitted from the week numbers of materials posted over \
             {} weeks",
            p("monday"),
            p("weeks")
        ),
        EvidenceCode::NoCourseDates => "no usable course dates".to_string(),
        EvidenceCode::TermLooksLikeEnrollmentWindow => format!(
            "LMS term{} runs {} ({} weeks): longer than a teaching term, so it is not used to \
             count weeks",
            term(),
            span(),
            p("weeks")
        ),
        EvidenceCode::DatesNotUsed => format!(
            "{} ({}) not used: {}",
            source_name(&p("source")),
            span(),
            reason_text(&p("reason"))
        ),
        EvidenceCode::EndNotUsed => format!(
            "end {} of {} not used ({}){}",
            p("end"),
            source_name(&p("source")),
            reason_text(&p("reason")),
            opt("until")
                .map(|until| format!("; {until} is used instead"))
                .unwrap_or_default()
        ),
        EvidenceCode::DatesAgree => format!(
            "{} agree: week 1 is the week of {}",
            source_name(&p("source")),
            p("monday")
        ),
        EvidenceCode::DatesMayBeWrong => format!(
            "{} put week 1 on the week of {}, {} days away: these dates may be wrong — check \
             them",
            source_name(&p("source")),
            p("monday"),
            p("days")
        ),
        EvidenceCode::SessionWindow => format!(
            "UofT session {}{}: {} (bounds only)",
            p("session"),
            opt("section").map(|s| format!(" {s}")).unwrap_or_default(),
            span()
        ),
        EvidenceCode::InstitutionCalendarMissing => format!(
            "The school calendar in this version of PageLamp has no dates for session {}: only \
             its months bound the course",
            p("session")
        ),
        EvidenceCode::WeekFromDates => format!(
            "week {} counted from week 1 starting {}",
            p("week"),
            p("monday")
        ),
        EvidenceCode::WeekFromModuleUnlock => format!(
            "module '{}' unlocked {} (most recent unlock), so week {}",
            p("title"),
            p("date"),
            p("week")
        ),
        EvidenceCode::WeekFromRecentMaterials => format!(
            "material '{}' published {} has the highest week number ({}) among materials \
             published in the last {} days",
            p("title"),
            p("date"),
            p("week"),
            p("days")
        ),
        EvidenceCode::WeekFromLatestMaterial => format!(
            "latest week-numbered material '{}' was published {} (week {}); nothing \
             week-numbered was published in the last {} days",
            p("title"),
            p("date"),
            p("week"),
            p("days")
        ),
        EvidenceCode::SignalAgrees => {
            format!("{} agree (week {})", signal_name(&p("signal")), p("week"))
        }
        EvidenceCode::SignalDisagrees => format!(
            "{} say week {}, which disagrees with week {}; the stronger signal wins",
            signal_name(&p("signal")),
            p("week"),
            p("chosen_week")
        ),
        EvidenceCode::ModulesReleasedTogether => format!(
            "{} modules (e.g. '{}') all unlocked {} (weeks {}-{}), so unlock dates do not pin \
             down the current week",
            p("count"),
            p("title"),
            p("date"),
            p("from_week"),
            p("to_week")
        ),
        EvidenceCode::UnlockTooOld => format!(
            "the most recent unlock ('{}', {}) was {} days ago: too long ago to show the current \
             week",
            p("title"),
            p("date"),
            p("days")
        ),
        EvidenceCode::UnlockWithoutWeek => format!(
            "module '{}' unlocked {} (most recent unlock) without a week number in the name",
            p("title"),
            p("date")
        ),
        EvidenceCode::BulkPublish => format!(
            "{} materials (e.g. '{}', weeks {}-{}) were published {}, e.g. by a bulk upload, so \
             publish dates do not pin down the current week",
            p("count"),
            p("title"),
            p("from_week"),
            p("to_week"),
            match (opt("start"), opt("end")) {
                (Some(start), Some(end)) if start == end => format!("on {start}"),
                _ => format!("between {} and {}", p("start"), p("end")),
            }
        ),
        EvidenceCode::NotesAhead => format!("week {} materials are already posted", p("week")),
        EvidenceCode::CalendarDisagreesWithNotes => format!(
            "recent materials are at week {}, not week {}",
            p("notes_week"),
            p("week")
        ),
        EvidenceCode::NumberingOffset => format!(
            "materials are numbered {} weeks off the course dates",
            p("offset")
        ),
        EvidenceCode::BreaksUnknown => "reading weeks and breaks are not known, so the week can \
                                        run ahead of the teaching week"
            .to_string(),
        EvidenceCode::NoWeekSignal => "current week unknown: no module unlock, course dates or \
                                       published material names a current week"
            .to_string(),
        EvidenceCode::StartsOn => format!("classes start on {}", p("date")),
        EvidenceCode::InBreak => format!("{} ({})", kind_name(&p("kind")), span()),
        EvidenceCode::NoClassToday => format!("no class today ({})", kind_name(&p("kind"))),
        EvidenceCode::ExamPeriod => format!("after the last class: exam period ({})", span()),
        EvidenceCode::ExamPeriodEstimated => format!(
            "no exam dates known: the exam period is taken as {} days after the last class",
            p("days")
        ),
        EvidenceCode::EndedOn => format!("the course ended on {}", p("date")),
        EvidenceCode::StartTooOld => format!(
            "only a start date is known ({}), {} weeks ago: longer than a term, so it no longer \
             counts weeks",
            p("start"),
            p("weeks")
        ),
        EvidenceCode::KeptCurrent => {
            format!("you marked this course current until {}", p("until"))
        }
        EvidenceCode::LmsConcluded => "the LMS says the course is concluded".to_string(),
        EvidenceCode::LmsCompleted => "the LMS marks the course completed".to_string(),
        EvidenceCode::NoLongerListed => "the LMS no longer lists the course as active".to_string(),
        EvidenceCode::ExamsOver => format!("your course dates ended on {}", p("date")),
        EvidenceCode::CourseEndPassed => {
            format!("{} ended on {}", source_name(&p("source")), p("end"))
        }
        EvidenceCode::DatesEnded => format!("by the course dates it ended on {}", p("date")),
        EvidenceCode::TermEndPassed => {
            format!("the LMS term{} ended on {}", term(), p("end"))
        }
        EvidenceCode::SessionEnded => {
            format!("UofT session {} ended on {}", p("session"), p("end"))
        }
        EvidenceCode::QuietSince => {
            format!("last activity on {}, {} days ago", p("date"), p("days"))
        }
        EvidenceCode::NoActivity => "no activity".to_string(),
        EvidenceCode::RecentActivity => format!("recent activity on {}", p("date")),
        EvidenceCode::NextEvent => format!("'{}' is on {}", p("title"), p("date")),
        EvidenceCode::SessionStarts => {
            format!("UofT session {} starts on {}", p("session"), p("start"))
        }
        EvidenceCode::NoDatesInactive => {
            format!("no course dates and no activity in {} days", p("days"))
        }
        EvidenceCode::MayHaveEnded => "the course may have ended".to_string(),
        EvidenceCode::RemovalSnoozed => {
            format!("removal suggestions snoozed until {}", p("until"))
        }
        EvidenceCode::RemovalKept => "you chose to keep this course".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_match_their_json_names() {
        for code in EvidenceCode::ALL {
            assert_eq!(EvidenceCode::parse(code.as_str()), Some(*code));
            assert_eq!(
                serde_json::to_value(code).unwrap(),
                serde_json::json!(code.as_str())
            );
            let item = EvidenceItem::new(*code);
            assert!(!item.english().is_empty());
        }
        assert_eq!(EvidenceCode::parse("no_such_code"), None);
    }

    #[test]
    fn text_params_are_plain_and_short() {
        let item = EvidenceItem::new(EvidenceCode::WeekFromModuleUnlock)
            .number("week", 4u32)
            .text(
                "title",
                &format!("Week 4\n- forged line\r{}", "x".repeat(200)),
            )
            .date("date", NaiveDate::from_ymd_opt(2026, 9, 28).unwrap());
        let title = item.param("title").unwrap();
        assert!(!title.chars().any(char::is_control));
        assert_eq!(title.chars().count(), MAX_TEXT_PARAM_CHARS + 1);
        assert!(title.ends_with('…'));
        assert!(item.english().starts_with("module 'Week 4 - forged line x"));
        assert!(
            item.english()
                .ends_with("unlocked 2026-09-28 (most recent unlock), so week 4")
        );
    }

    #[test]
    fn enrollment_window_sentence() {
        let date = |m, d, y| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let item = EvidenceItem::new(EvidenceCode::TermLooksLikeEnrollmentWindow)
            .text("term_name", "Fall 2026")
            .date("start", date(5, 4, 2026))
            .date("end", date(1, 31, 2027))
            .number("weeks", 39);
        assert_eq!(
            item.english(),
            "LMS term 'Fall 2026' runs 2026-05-04 → 2027-01-31 (39 weeks): longer than a \
             teaching term, so it is not used to count weeks"
        );
    }
}
