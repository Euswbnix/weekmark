//! Course lifecycle: is the course upcoming, current, finishing or over, and should PageLamp
//! suggest removing it (docs/design/v0.3-course-calendar.md §8.1)? Computed at every read,
//! never stored; pure functions, callers pass `today`.
//!
//! The lifecycle, not the phase, decides every exclusion: Ended, Inactive and Upcoming courses
//! leave the week-based views (D43), deadlines are never filtered by it (§8.2).

use chrono::{Datelike, NaiveDate};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dates::{add_days, course_date, days_between, week_one_monday};
use crate::model::{
    Confidence, Course, CourseTermData, CourseTimeline, Event, EventKind, TermResolution,
};
use crate::term::evidence::{EvidenceCode, EvidenceItem};
use crate::term::phase::EXAM_PERIOD_DAYS;
use crate::term::{CoursePhase, ResolvedTerm, TermAnchorSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Upcoming,
    Current,
    /// Over by its dates, but exams, recent activity or a future event keep it going.
    Finishing,
    Ended,
    /// No dates at all and no activity for months (orientation, club and training sites).
    Inactive,
    Unknown,
}

impl LifecycleState {
    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleState::Upcoming => "upcoming",
            LifecycleState::Current => "current",
            LifecycleState::Finishing => "finishing",
            LifecycleState::Ended => "ended",
            LifecycleState::Inactive => "inactive",
            LifecycleState::Unknown => "unknown",
        }
    }

    /// Whether the course takes part in the week-based views (design §8.2, D43): Ended,
    /// Inactive and Upcoming courses don't, so they have no current week.
    pub fn in_week_views(self) -> bool {
        !matches!(
            self,
            LifecycleState::Ended | LifecycleState::Inactive | LifecycleState::Upcoming
        )
    }

    /// Ended and Inactive → Past; Upcoming → Upcoming; everything else → Current.
    pub fn group(self) -> CourseGroup {
        match self {
            LifecycleState::Ended | LifecycleState::Inactive => CourseGroup::Past,
            LifecycleState::Upcoming => CourseGroup::Upcoming,
            LifecycleState::Current | LifecycleState::Finishing | LifecycleState::Unknown => {
                CourseGroup::Current
            }
        }
    }
}

/// How course lists group courses (the same in Tauri, Swift and the CLI).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CourseGroup {
    Current,
    Upcoming,
    Past,
}

impl CourseGroup {
    pub fn as_str(self) -> &'static str {
        match self {
            CourseGroup::Current => "current",
            CourseGroup::Upcoming => "upcoming",
            CourseGroup::Past => "past",
        }
    }
}

/// The answer to a removal suggestion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SnoozeKind {
    /// Don't suggest it for 14 days.
    NotNow,
    /// Never suggest it again.
    Keep,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CourseLifecycle {
    pub state: LifecycleState,
    pub group: CourseGroup,
    pub confidence: Confidence,
    /// Since when the state holds, when known (e.g. the day an Ended course ended).
    pub since: Option<NaiveDate>,
    /// When classes start (Upcoming courses: "Starts Jan 11"), when known.
    pub starts_on: Option<NaiveDate>,
    /// The latest material, announcement, event or module unlock date on or before today.
    pub last_activity: Option<NaiveDate>,
    /// The earliest course event (any kind but "other") from today to today + 30 days.
    pub next_event: Option<NaiveDate>,
    pub evidence_items: Vec<EvidenceItem>,
    /// Ended or Inactive, and the suggestion isn't snoozed. Hidden courses are suggested too.
    pub suggest_removal: bool,
    /// "I'm still taking this" until this date, when set.
    pub kept_current_until: Option<NaiveDate>,
    /// `is_active` on the day it was computed: the courses week-by-week features cover.
    pub is_active: bool,
}

impl CourseLifecycle {
    /// A course nothing is known about yet.
    pub fn unknown() -> Self {
        CourseLifecycle {
            state: LifecycleState::Unknown,
            group: LifecycleState::Unknown.group(),
            confidence: Confidence::Low,
            since: None,
            starts_on: None,
            last_activity: None,
            next_event: None,
            evidence_items: Vec::new(),
            suggest_removal: false,
            kept_current_until: None,
            is_active: true,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The rules (design §8.1)
// ---------------------------------------------------------------------------------------------

/// Rule 2: a plausible, unclipped LMS or folder end ends the course this many days later.
pub const END_GRACE_DAYS: i64 = 21;
/// Rule 2: the student's dates: exams end + 7 days, or (no exam date) last class + 28 days.
pub const AFTER_EXAMS_DAYS: i64 = 7;
pub const AFTER_LAST_CLASS_DAYS: i64 = 28;
/// Rules 3 and 4: "no activity in 21 days" / "activity in the last 21 days".
pub const QUIET_DAYS: i64 = 21;
/// Rule 3b: the session window alone needs 60 quiet days (or a past LMS term end).
pub const LONG_QUIET_DAYS: i64 = 60;
/// Rule 7: no dates at all and no activity for this long → Inactive.
pub const INACTIVE_DAYS: i64 = 120;
/// Rule 4: a course event within this many days keeps an ending course Finishing.
pub const NEXT_EVENT_DAYS: i64 = 30;
/// `is_active`: Upcoming courses count when they start within this many days.
pub const ACTIVE_BEFORE_START_DAYS: i64 = 14;

/// Everything the lifecycle reads about one course.
pub struct LifecycleInput<'a> {
    pub course: &'a Course,
    pub data: &'a CourseTermData,
    pub resolved: &'a ResolvedTerm,
    pub timeline: &'a CourseTimeline,
    /// The course's events (any date).
    pub events: &'a [Event],
    pub today: NaiveDate,
}

/// A reason the course may be over.
struct EndSignal {
    item: EvidenceItem,
    /// The day it ended, when known.
    since: Option<NaiveDate>,
}

/// The course's lifecycle on `input.today` (design §8.1; the first matching rule decides, and
/// an Ended result of rules 2–3b becomes Finishing while a course event is ahead).
pub fn course_lifecycle(input: &LifecycleInput<'_>) -> CourseLifecycle {
    let today = input.today;
    let data = input.data;
    let resolved = input.resolved;
    let term = &resolved.resolution;
    let timeline = input.timeline;
    let phase = timeline.phase;
    let tz = resolved.tz;
    let last_activity = resolved.last_activity;
    let days_quiet = last_activity.map(|last| days_between(last, today));
    let quiet_for = |days: i64| days_quiet.is_none_or(|quiet| quiet > days);
    let next_event = input
        .events
        .iter()
        .filter(|event| event.kind != EventKind::Other)
        .filter_map(|event| event.when().map(|when| (course_date(when, tz), event)))
        .filter(|(day, _)| *day >= today && days_between(today, *day) <= NEXT_EVENT_DAYS)
        .min_by_key(|(day, _)| *day);
    let starts_on = timeline
        .term
        .teaching
        .first()
        .map(|segment| {
            segment
                .first_class
                .max(week_one_monday(segment.first_class))
        })
        .filter(|_| phase == CoursePhase::NotStarted)
        .or_else(|| {
            resolved
                .session
                .as_ref()
                .map(|s| s.window.start)
                .filter(|start| *start > today)
        });

    let mut lifecycle = CourseLifecycle {
        last_activity,
        next_event: next_event.map(|(day, _)| day),
        starts_on,
        kept_current_until: data.keep_current_until.filter(|until| *until >= today),
        ..CourseLifecycle::unknown()
    };
    let activity_item = || match (last_activity, days_quiet) {
        (Some(date), Some(days)) => EvidenceItem::new(EvidenceCode::QuietSince)
            .date("date", date)
            .number("days", days),
        _ => EvidenceItem::new(EvidenceCode::NoActivity),
    };

    // Rule 1: "I'm still taking this".
    if let Some(until) = data.keep_current_until.filter(|until| *until >= today) {
        lifecycle.set(LifecycleState::Current, Confidence::High);
        lifecycle
            .evidence_items
            .push(EvidenceItem::new(EvidenceCode::KeptCurrent).date("until", until));
        return finish(lifecycle, data, today);
    }

    // Rule 2: strong end signals → Ended · High.
    let mut strong: Vec<EndSignal> = Vec::new();
    // The student's own dates: exams end + 7 days, or (no exam date) last class + 28.
    let own_end = term
        .exams_end
        .map(|end| (end, add_days(end, AFTER_EXAMS_DAYS)))
        .or_else(|| {
            resolved
                .student_last_class
                .map(|last| (last, add_days(last, AFTER_LAST_CLASS_DAYS)))
        });
    if let Some((end, over)) = own_end
        && over < today
    {
        strong.push(EndSignal {
            item: EvidenceItem::new(EvidenceCode::ExamsOver).date("date", end),
            since: Some(end),
        });
    }
    if data.lms.concluded == Some(true) {
        strong.push(EndSignal {
            item: EvidenceItem::new(EvidenceCode::LmsConcluded),
            since: None,
        });
    }
    if data.lms.workflow_state.as_deref() == Some("completed") {
        strong.push(EndSignal {
            item: EvidenceItem::new(EvidenceCode::LmsCompleted),
            since: None,
        });
    }
    if !input.course.enrollment_active {
        strong.push(EndSignal {
            item: EvidenceItem::new(EvidenceCode::NoLongerListed),
            since: None,
        });
    }
    // (The student's own end, when given, is the only end that counts.)
    if let Some((source, end)) = resolved.plausible_end
        && add_days(end, END_GRACE_DAYS) < today
        && own_end.is_none()
    {
        strong.push(EndSignal {
            item: EvidenceItem::new(EvidenceCode::CourseEndPassed)
                .source(source)
                .date("end", end),
            since: Some(end),
        });
    }
    if !strong.is_empty() {
        return ended(lifecycle, strong, Confidence::High, next_event, data, today);
    }

    // Rules 3 and 3b: weaker end signals, only when the phase says nothing (Ended or
    // Unknown, or from a Low anchor). Teaching or a break from a Medium+ anchor blocks them.
    // While the course is teaching (or on a break) and its last class is still ahead, no weak
    // signal ends it, whatever the anchor's confidence (a cross-check can make it Low).
    let last_class = term.teaching.last().and_then(|s| s.last_class);
    let teaching_now = matches!(phase, CoursePhase::Teaching | CoursePhase::Break)
        && last_class.is_none_or(|last| last >= today);
    let weak_phase = matches!(phase, CoursePhase::Ended | CoursePhase::Unknown)
        || (term.anchor_confidence == Confidence::Low && !teaching_now);
    if weak_phase {
        let mut weak: Vec<EndSignal> = Vec::new();
        // E1: dates that aren't the student's put the course past its end.
        if phase == CoursePhase::Ended && term.anchor != TermAnchorSource::StudentConfirmed {
            let since = timeline_end(term);
            weak.push(EndSignal {
                item: EvidenceItem::new(EvidenceCode::DatesEnded).opt_date("date", since),
                since,
            });
        }
        // E2: an LMS term that isn't used, but has ended.
        let e2 = term
            .not_used
            .iter()
            .filter(|r| r.source == TermAnchorSource::LmsTerm)
            // A clipped end counts only once the end that replaced it has passed too.
            .filter(|r| !r.end_only || last_class.is_some_and(|last| last < today))
            .filter_map(|r| r.end)
            .find(|end| *end < today);
        if let Some(end) = e2 {
            weak.push(EndSignal {
                item: EvidenceItem::new(EvidenceCode::TermEndPassed)
                    .opt_text("term_name", data.lms.term_name.as_deref())
                    .date("end", end),
                since: Some(end),
            });
        }
        // E3: the session window ended, which counts only with E2 or 60 quiet days.
        if let Some(session) = &resolved.session
            && add_days(session.window.end, END_GRACE_DAYS) < today
            && (e2.is_some() || quiet_for(LONG_QUIET_DAYS))
        {
            weak.push(EndSignal {
                item: EvidenceItem::new(EvidenceCode::SessionEnded)
                    .text("session", &session.session)
                    .date("end", session.window.end),
                since: Some(session.window.end),
            });
        }
        if !weak.is_empty() {
            if quiet_for(QUIET_DAYS) {
                lifecycle.evidence_items.push(activity_item());
                return ended(lifecycle, weak, Confidence::Medium, next_event, data, today);
            }
            // Rule 4: an end signal, but recent activity (e.g. grade announcements).
            lifecycle.set(LifecycleState::Finishing, Confidence::Medium);
            lifecycle.since = weak.iter().filter_map(|s| s.since).min();
            lifecycle
                .evidence_items
                .extend(weak.into_iter().map(|signal| signal.item));
            if let Some(last) = last_activity {
                lifecycle
                    .evidence_items
                    .push(EvidenceItem::new(EvidenceCode::RecentActivity).date("date", last));
            }
            return finish(lifecycle, data, today);
        }
    }

    // Rule 4: the exam period.
    if phase == CoursePhase::ExamPeriod {
        lifecycle.set(LifecycleState::Finishing, timeline.phase_confidence);
        lifecycle.since = term.teaching.last().and_then(|s| s.last_class);
        return finish(lifecycle, data, today);
    }
    // Rule 5: not started yet.
    let window_ahead = resolved.session.as_ref().filter(|s| s.window.start > today);
    if phase == CoursePhase::NotStarted || window_ahead.is_some() {
        let confidence = if phase == CoursePhase::NotStarted {
            timeline.phase_confidence
        } else {
            Confidence::Low
        };
        lifecycle.set(LifecycleState::Upcoming, confidence);
        match (phase, window_ahead) {
            (CoursePhase::NotStarted, _) | (_, None) => {
                if let Some(start) = lifecycle.starts_on {
                    lifecycle
                        .evidence_items
                        .push(EvidenceItem::new(EvidenceCode::StartsOn).date("date", start));
                }
            }
            (_, Some(session)) => lifecycle.evidence_items.push(
                EvidenceItem::new(EvidenceCode::SessionStarts)
                    .text("session", &session.session)
                    .date("start", session.window.start),
            ),
        }
        return finish(lifecycle, data, today);
    }
    // Rule 6: teaching, or a break.
    if matches!(phase, CoursePhase::Teaching | CoursePhase::Break) {
        lifecycle.set(LifecycleState::Current, timeline.phase_confidence);
        return finish(lifecycle, data, today);
    }
    // Rule 7: no dates at all, and nothing for months.
    let no_dates = term.anchor == TermAnchorSource::NoAnchor
        && resolved.session.is_none()
        && term.outer_frame.is_none()
        && data.lms.term_start.is_none()
        && data.lms.term_end.is_none();
    if no_dates && quiet_for(INACTIVE_DAYS) {
        lifecycle.set(LifecycleState::Inactive, Confidence::Medium);
        lifecycle
            .evidence_items
            .push(EvidenceItem::new(EvidenceCode::NoDatesInactive).number("days", INACTIVE_DAYS));
        lifecycle.evidence_items.push(activity_item());
        return finish(lifecycle, data, today);
    }
    // Rule 8: unknown; say so when there are weak signs of an end.
    lifecycle.set(LifecycleState::Unknown, Confidence::Low);
    let weak_end = resolved
        .session
        .as_ref()
        .is_some_and(|s| s.window.end < today)
        || term
            .not_used
            .iter()
            .any(|r| r.end.is_some_and(|end| end < today));
    if weak_end {
        lifecycle
            .evidence_items
            .push(EvidenceItem::new(EvidenceCode::MayHaveEnded));
    }
    finish(lifecycle, data, today)
}

/// Ended (or Finishing while a course event is ahead: rule 4).
fn ended(
    mut lifecycle: CourseLifecycle,
    signals: Vec<EndSignal>,
    confidence: Confidence,
    next_event: Option<(NaiveDate, &Event)>,
    data: &CourseTermData,
    today: NaiveDate,
) -> CourseLifecycle {
    lifecycle.since = signals.iter().filter_map(|s| s.since).min();
    let mut items: Vec<EvidenceItem> = signals.into_iter().map(|s| s.item).collect();
    items.append(&mut lifecycle.evidence_items);
    lifecycle.evidence_items = items;
    match next_event {
        Some((day, event)) => {
            lifecycle.set(LifecycleState::Finishing, confidence);
            lifecycle.evidence_items.push(
                EvidenceItem::new(EvidenceCode::NextEvent)
                    .date("date", day)
                    .text("title", &event.title),
            );
        }
        None => lifecycle.set(LifecycleState::Ended, confidence),
    }
    finish(lifecycle, data, today)
}

/// The removal suggestion (Ended or Inactive, unless snoozed), for every state.
fn finish(
    mut lifecycle: CourseLifecycle,
    data: &CourseTermData,
    today: NaiveDate,
) -> CourseLifecycle {
    let over = matches!(
        lifecycle.state,
        LifecycleState::Ended | LifecycleState::Inactive
    );
    let snoozed = data.removal_snoozed_until.filter(|until| *until >= today);
    lifecycle.suggest_removal = over && snoozed.is_none();
    lifecycle.is_active = is_active(&lifecycle, today);
    if over && let Some(until) = snoozed {
        let item = if until.year() >= 9999 {
            EvidenceItem::new(EvidenceCode::RemovalKept)
        } else {
            EvidenceItem::new(EvidenceCode::RemovalSnoozed).date("until", until)
        };
        lifecycle.evidence_items.push(item);
    }
    lifecycle
}

/// The day the resolved dates say the course ended (the exams' end, or last class + 21).
fn timeline_end(term: &TermResolution) -> Option<NaiveDate> {
    term.exams_end.or_else(|| {
        term.teaching
            .last()
            .and_then(|s| s.last_class)
            .map(|last| add_days(last, EXAM_PERIOD_DAYS))
    })
}

impl CourseLifecycle {
    fn set(&mut self, state: LifecycleState, confidence: Confidence) {
        self.state = state;
        self.group = state.group();
        self.confidence = confidence;
    }
}

/// "Active" for features that work week by week (study plans, reminders, the digest's week
/// sections): Current, Finishing or Unknown, and Upcoming courses that start within 14 days.
pub fn is_active(lifecycle: &CourseLifecycle, today: NaiveDate) -> bool {
    match lifecycle.state {
        LifecycleState::Current | LifecycleState::Finishing | LifecycleState::Unknown => true,
        LifecycleState::Upcoming => lifecycle
            .starts_on
            .is_some_and(|start| days_between(today, start) <= ACTIVE_BEFORE_START_DAYS),
        LifecycleState::Ended | LifecycleState::Inactive => false,
    }
}
