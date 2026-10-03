//! Which dates count a course's weeks (docs/design/v0.3-course-calendar.md §6.2–§6.4).
//!
//! Sources, in priority order (the first usable one is the anchor):
//! 1. the student's dates (`user_term_*`; `Legacy` until confirmed in 0.3, see
//!    `CONFIRMED_DATES_KEY`), always used; a legacy end is dropped when start → end fails a
//!    plausibility check;
//! 2. the LMS course's own dates, 3. the LMS term, 4. the folder's dates, when plausible;
//! 5. the school's published calendar (`institution`; UofT, where the session hint applies);
//! 6. week 1 fitted from the professor's week-numbered materials (`fit`).
//!
//! Plausibility (§6.3): a span is not used when it starts after it ends, is longer than 26
//! weeks (36 with full-year evidence), shorter than 4 weeks, starts more than 28 days before
//! the course's first activity, or starts more than 14 days before the session window. Only
//! the end is not used (replaced by the window end) when it lies more than 28 days after the
//! window, or, for a full-year section, more than 28 days before its end.
//!
//! Lower sources that put week 1 at least 7 days away from the anchor are noted; the anchor's
//! confidence then drops one level unless the student confirmed it.

use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;

use super::evidence::{EvidenceCode, EvidenceItem};
use super::fit::{self, Observation};
use super::institution::{self, InstitutionCalendar, InstitutionTerm};
use super::session::{SessionHint, session_hint};
use super::{
    CalendarOrigin, CalendarStatus, DateSpan, RejectReason, RejectedDates, TeachingSegment,
    TermAnchorSource, TermResolution,
};
use crate::calendar::CalendarInForce;
use crate::dates::{
    Tz, add_days, course_date, days_between, monday_of, time_zone, week_one_monday,
};
use crate::model::{Confidence, Course, CourseTermData, Event, Material, Module};

/// Longest plausible teaching term, in days (26 weeks).
pub const MAX_TERM_DAYS: i64 = 182;
/// Longest plausible term with full-year evidence (36 weeks).
pub const MAX_YEAR_TERM_DAYS: i64 = 252;
/// Shortest plausible term (4 weeks).
pub const MIN_TERM_DAYS: i64 = 28;
/// A start this long before the first activity is an enrollment window, not a term.
pub const MAX_DAYS_BEFORE_ACTIVITY: i64 = 28;
/// A start this long before the session window is not this session's.
pub const MAX_DAYS_BEFORE_WINDOW: i64 = 14;
/// An end this far outside the session window is replaced by the window end.
pub const MAX_END_DAYS_OUTSIDE_WINDOW: i64 = 28;
/// Without an outer frame, the fit accepts week-1 Mondays within this many days of today.
const FALLBACK_FRAME_DAYS: i64 = 240;
/// Another source this far from the anchor's week 1 "disagrees".
const DISAGREE_DAYS: i64 = 7;

/// A full-year term name: "Full Year 2026-27", "Fall/Winter".
static FULL_YEAR_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)full[- ]?year|fall\s*/\s*winter").expect("valid regex"));

/// Everything the resolver reads about one course (all of it from the store).
#[derive(Clone, Copy)]
pub struct TermInput<'a> {
    /// The machine's time zone, for courses without their own (folder courses, Canvas rows
    /// synced before schema 3). None → UTC.
    pub fallback_tz: Option<Tz>,
    pub course: &'a Course,
    pub data: &'a CourseTermData,
    /// The course is in the `CONFIRMED_DATES_KEY` list.
    pub dates_confirmed: bool,
    /// The course's accepted calendar (schema v4), which outranks everything else.
    pub calendar: Option<&'a CalendarInForce>,
    pub modules: &'a [Module],
    pub materials: &'a [Material],
    pub events: &'a [Event],
    pub today: NaiveDate,
    /// The school's calendar to use (§6.4 anchor 5); `None`: the one shipped with PageLamp.
    pub institution: Option<&'a InstitutionCalendar>,
}

impl TermInput<'_> {
    /// The course's time zone; for a folder course the machine's (None → UTC dates). A Canvas
    /// course without a zone keeps UTC: its LMS dates were stored as UTC dates, and one course
    /// uses one zone everywhere (§6.1).
    pub fn time_zone(&self) -> Option<Tz> {
        let canvas = self.course.source_id.starts_with("canvas:");
        self.data
            .lms
            .time_zone
            .as_deref()
            .and_then(time_zone)
            .or(self.fallback_tz.filter(|_| !canvas))
    }
}

/// The resolver's result: the public `TermResolution` plus what the timeline and the lifecycle
/// need from the same pass.
#[derive(Clone, Debug)]
pub struct ResolvedTerm {
    pub resolution: TermResolution,
    pub tz: Option<Tz>,
    pub session: Option<SessionHint>,
    /// Full-year evidence (Y section, or a full-year term name).
    pub full_year: bool,
    /// The anchor's end was replaced by the session window end.
    pub end_clipped: bool,
    /// The first plausible, unclipped end of the LMS or folder dates (lifecycle rule 2), with
    /// its source, whether or not those dates are the anchor.
    pub plausible_end: Option<(TermAnchorSource, NaiveDate)>,
    /// Earliest and latest activity on or before today (materials, unlocks, events).
    pub first_activity: Option<NaiveDate>,
    pub last_activity: Option<NaiveDate>,
    /// The last day of classes the student gave (their calendar or their own term end), for
    /// the lifecycle's rule 2; None when the end came from elsewhere.
    pub student_last_class: Option<NaiveDate>,
    /// Accepted when a calendar is in force.
    pub calendar: CalendarStatus,
    /// The school's dates for this course's session and campus, whether or not they are the
    /// anchor (a calendar proposal's V8 check compares its reading weeks).
    pub institution: Option<InstitutionTerm>,
    /// Non-bulk per-day week observations (§6.5), oldest first.
    pub(crate) observations: Vec<Observation>,
    /// Week 1 fitted from the professor's materials, whether or not it is the anchor.
    pub(crate) fitted: Option<fit::Fit>,
    /// The LMS course's own start, when plausible (a calendar proposal's V8 check).
    pub plausible_lms_start: Option<NaiveDate>,
    pub notes_week: Option<u32>,
    /// Evidence about the dates, in order.
    pub evidence: Vec<EvidenceItem>,
    /// The English lines worth showing in `CourseTimeline.evidence` (not every item has one:
    /// the dates used are implied by the week line).
    pub english: Vec<String>,
}

/// A source's dates, before plausibility checks.
#[derive(Clone, Debug)]
struct Offer {
    source: TermAnchorSource,
    start: Option<NaiveDate>,
    end: Option<NaiveDate>,
}

/// A usable anchor.
#[derive(Clone, Debug)]
struct Anchor {
    source: TermAnchorSource,
    start: NaiveDate,
    end: Option<NaiveDate>,
    confidence: Confidence,
    origin: Option<CalendarOrigin>,
    end_clipped: bool,
    /// For the fit: how many weeks' materials agree.
    fit_weeks: Option<usize>,
}

/// What a plausibility check says about a span.
enum Verdict {
    Use {
        end: Option<NaiveDate>,
        /// The end was replaced: (the original end, why).
        clipped: Option<(NaiveDate, RejectReason)>,
    },
    Reject(RejectReason),
}

struct Context<'a> {
    first_activity: Option<NaiveDate>,
    session: Option<&'a SessionHint>,
    full_year: bool,
}

pub fn resolve_term(input: &TermInput<'_>) -> ResolvedTerm {
    let tz = input.time_zone();
    let data = input.data;
    let lms = &data.lms;
    let session = session_hint(input.course, data.institution.as_deref());
    let full_year = session.as_ref().is_some_and(|s| s.full_year)
        || lms
            .term_name
            .as_deref()
            .is_some_and(|name| FULL_YEAR_NAME.is_match(name));
    let (first_activity, last_activity) = activity(input, tz);
    let ctx = Context {
        first_activity,
        session: session.as_ref(),
        full_year,
    };
    let mut evidence = Vec::new();
    let mut english = Vec::new();
    let mut not_used = Vec::new();

    // The LMS term, raw; Canvas rows synced before schema 3 only have the merged dates.
    let is_canvas = input.course.source_id.starts_with("canvas:");
    let has_lms_facts = lms.term_start.is_some()
        || lms.term_end.is_some()
        || lms.course_start.is_some()
        || lms.course_end.is_some();
    let (lms_term_start, lms_term_end) = if has_lms_facts {
        (lms.term_start, lms.term_end)
    } else if is_canvas {
        (data.synced_term_start, data.synced_term_end)
    } else {
        (None, None)
    };

    // Outer frame: the LMS term (plausible or not) ∪ the session window.
    let term_span = match (lms_term_start, lms_term_end) {
        (Some(start), Some(end)) if start <= end => Some(DateSpan { start, end }),
        _ => None,
    };
    let outer_frame = union(term_span, session.as_ref().map(|s| s.window));
    if let Some(session) = &session {
        let item = EvidenceItem::new(EvidenceCode::SessionWindow)
            .text("session", &session.session)
            .opt_text("section", session.section.map(String::from).as_deref())
            .date("start", session.window.start)
            .date("end", session.window.end);
        evidence.push(item);
    }

    // Offers from the LMS and the folder, in priority order.
    let mut offers = Vec::new();
    if is_canvas {
        if has_lms_facts {
            offers.push(Offer {
                source: TermAnchorSource::LmsCourseDates,
                start: lms.course_start,
                end: lms.course_end,
            });
        }
        offers.push(Offer {
            source: TermAnchorSource::LmsTerm,
            start: lms_term_start,
            end: lms_term_end,
        });
    } else {
        offers.push(Offer {
            source: TermAnchorSource::FolderConfig,
            start: data.synced_term_start,
            end: data.synced_term_end,
        });
    }
    let mut plausible: Vec<Anchor> = Vec::new();
    for offer in &offers {
        let Some(start) = offer.start else { continue };
        match check(start, offer.end, &ctx) {
            Verdict::Use { end, clipped } => {
                if let Some((original, reason)) = clipped {
                    not_used.push(RejectedDates {
                        source: offer.source,
                        start: None,
                        end: Some(original),
                        reason,
                        end_only: true,
                    });
                }
                plausible.push(Anchor {
                    source: offer.source,
                    start,
                    end,
                    confidence: Confidence::Medium,
                    origin: None,
                    end_clipped: clipped.is_some(),
                    fit_weeks: None,
                });
            }
            Verdict::Reject(reason) => not_used.push(RejectedDates {
                source: offer.source,
                start: Some(start),
                end: offer.end,
                reason,
                end_only: false,
            }),
        }
    }

    // The school's published calendar (§6.4 anchor 5), below the LMS and folder dates.
    let school = input.institution.unwrap_or_else(|| institution::shipped());
    let school_term = session.as_ref().and_then(|hint| school.term(hint));
    if let Some(term) = &school_term {
        plausible.push(Anchor {
            source: TermAnchorSource::InstitutionCalendar,
            start: term.first_class(),
            end: term.last_class(),
            confidence: Confidence::Medium,
            origin: None,
            end_clipped: false,
            fit_weeks: None,
        });
    } else if let Some(hint) = &session
        && hint.campus.is_some()
        && !school.years.is_empty()
    {
        // The file has other years (or campuses) but not this one: the window bounds it.
        evidence.push(
            EvidenceItem::new(EvidenceCode::InstitutionCalendarMissing)
                .text("session", &hint.session),
        );
    }

    // The fit from the professor's materials.
    let observations = fit::observations(input.modules, input.materials, tz, input.today);
    let frame = outer_frame.unwrap_or(DateSpan {
        start: add_days(input.today, -FALLBACK_FRAME_DAYS),
        end: add_days(input.today, FALLBACK_FRAME_DAYS),
    });
    // Week-1 Mondays are checked against the frame from its own week's Monday (a frame that
    // starts on a Tuesday must still admit that week).
    let frame = DateSpan {
        start: monday_of(frame.start),
        end: frame.end,
    };
    let fitted = fit::fit(&observations, frame);
    if let Some(fitted) = fitted {
        plausible.push(Anchor {
            source: TermAnchorSource::PublishedWeekLabels,
            start: fitted.monday,
            end: None,
            confidence: fitted.confidence,
            origin: None,
            end_clipped: false,
            fit_weeks: Some(fitted.weeks),
        });
    }

    // The student's dates come first.
    let origin = if input.dates_confirmed {
        CalendarOrigin::User
    } else {
        CalendarOrigin::Legacy
    };
    let student = match input.calendar {
        Some(in_force) => calendar_anchor(in_force),
        None => student_anchor(data, origin, &ctx, &mut not_used),
    };
    let lower_index = usize::from(student.is_none());
    let mut anchor = match (student, plausible.first()) {
        (Some(student), _) => Some(student),
        (None, Some(first)) => Some(first.clone()),
        (None, None) => None,
    };

    // A student end alone replaces only the end of the anchor below.
    if input.calendar.is_none()
        && data.user_term_start.is_none()
        && let (Some(end), Some(anchor)) = (data.user_term_end, anchor.as_mut())
        && end >= anchor.start
    {
        anchor.end = Some(end);
        anchor.end_clipped = false;
        evidence.push(EvidenceItem::new(EvidenceCode::StudentEndUsed).date("end", end));
    }
    // A student start without an end borrows a plausible end from below (a calendar's
    // segments say what they mean). From the school's calendar it takes its breaks and exam
    // period too (the same term), from the student's own start.
    let mut borrowed_school: Option<InstitutionTerm> = None;
    if let Some(anchor) = anchor.as_mut()
        && input.calendar.is_none()
        && anchor.source == TermAnchorSource::StudentConfirmed
        && anchor.end.is_none()
        && let Some(lower) = plausible.iter().find(|lower| {
            lower
                .end
                .is_some_and(|end| days_between(anchor.start, end) >= MIN_TERM_DAYS)
        })
    {
        anchor.end = lower.end;
        anchor.end_clipped = lower.end_clipped;
        if lower.source == TermAnchorSource::InstitutionCalendar {
            borrowed_school = school_term
                .as_ref()
                .and_then(|term| term.starting(anchor.start));
        }
        evidence.push(anchor_item(lower, lms.term_name.as_deref()));
    }

    // Cross-check the anchor against the lower sources.
    if let Some(anchor) = anchor.as_mut() {
        evidence.insert(0, anchor_item(anchor, lms.term_name.as_deref()));
        if anchor.origin == Some(CalendarOrigin::Legacy) {
            english.push(evidence[0].english());
        }
        let mut disagreed = false;
        let lower = plausible
            .iter()
            .skip(lower_index)
            .filter(|lower| lower.source != anchor.source);
        for lower in lower {
            let monday = week_one_monday(lower.start);
            let days = days_between(week_one_monday(anchor.start), monday).abs();
            let item = if days >= DISAGREE_DAYS {
                disagreed = true;
                EvidenceItem::new(EvidenceCode::DatesMayBeWrong)
                    .source(lower.source)
                    .date("monday", monday)
                    .number("days", days)
            } else {
                EvidenceItem::new(EvidenceCode::DatesAgree)
                    .source(lower.source)
                    .date("monday", monday)
            };
            if days >= DISAGREE_DAYS {
                english.push(item.english());
            }
            evidence.push(item);
        }
        if disagreed && anchor.source != TermAnchorSource::StudentConfirmed {
            anchor.confidence = one_level_down(anchor.confidence);
        }
    } else {
        evidence.insert(0, EvidenceItem::new(EvidenceCode::NoCourseDates));
        // A last day of classes with nothing else: an end-only date (§6.6).
        if input.calendar.is_none()
            && let Some(end) = data.user_term_end
        {
            evidence.push(EvidenceItem::new(EvidenceCode::StudentEndUsed).date("end", end));
        }
    }

    // What was not used, and why.
    for rejected in &not_used {
        let item = rejected_item(rejected, lms.term_name.as_deref(), outer_until(&ctx));
        english.push(item.english());
        evidence.push(item);
    }

    let resolution = match (&anchor, input.calendar) {
        (Some(anchor), Some(in_force)) => TermResolution {
            week_one_monday: Some(week_one_monday(anchor.start)),
            teaching: in_force.calendar.segments.clone(),
            breaks: in_force.calendar.breaks.clone(),
            exams_end: in_force.calendar.exam_period.map(|period| period.end),
            anchor: anchor.source,
            anchor_confidence: anchor.confidence,
            anchor_origin: anchor.origin,
            ai_label: in_force.ai_label.clone(),
            outer_frame,
            not_used,
            student_start: data.user_term_start,
            student_end: data.user_term_end,
        },
        (Some(anchor), None) => {
            // The school's segments, breaks and exam period when its calendar is the anchor or
            // lent the student's start its end.
            let school_dates = match anchor.source {
                TermAnchorSource::InstitutionCalendar => school_term.clone(),
                _ => borrowed_school,
            };
            let (teaching, breaks, exams_end) = match school_dates {
                Some(term) => (term.segments, term.breaks, term.exams_end),
                None => (
                    vec![TeachingSegment {
                        first_class: anchor.start,
                        last_class: anchor.end,
                        first_week_number: 1,
                    }],
                    Vec::new(),
                    None,
                ),
            };
            TermResolution {
                week_one_monday: Some(week_one_monday(anchor.start)),
                teaching,
                breaks,
                exams_end,
                anchor: anchor.source,
                anchor_confidence: anchor.confidence,
                anchor_origin: anchor.origin,
                ai_label: None,
                outer_frame,
                not_used,
                student_start: data.user_term_start,
                student_end: data.user_term_end,
            }
        }
        (None, _) => TermResolution {
            outer_frame,
            not_used,
            student_start: data.user_term_start,
            student_end: data.user_term_end,
            ..TermResolution::default()
        },
    };
    let plausible_lms_start = plausible
        .iter()
        .find(|a| a.source == TermAnchorSource::LmsCourseDates)
        .map(|a| a.start);
    let plausible_end = plausible
        .iter()
        .filter(|a| a.source != TermAnchorSource::PublishedWeekLabels && !a.end_clipped)
        .find_map(|a| Some((a.source, a.end?)));
    let student_last_class = match input.calendar {
        Some(in_force) => in_force
            .calendar
            .segments
            .last()
            .and_then(|segment| segment.last_class),
        // The student's own end, when it is the end in force (a legacy end may be dropped), or
        // the only date there is (an end-only date).
        None => data.user_term_end.filter(|end| match &anchor {
            Some(anchor) => anchor.end == Some(*end),
            None => true,
        }),
    };
    ResolvedTerm {
        calendar: if input.calendar.is_some() {
            CalendarStatus::Accepted
        } else {
            CalendarStatus::NoCalendar
        },
        student_last_class,
        resolution,
        tz,
        plausible_end,
        end_clipped: anchor.as_ref().is_some_and(|a| a.end_clipped),
        session,
        full_year,
        first_activity,
        last_activity,
        notes_week: fit::notes_week(&observations),
        institution: school_term,
        observations,
        fitted,
        plausible_lms_start,
        evidence,
        english,
    }
}

/// The student's dates as an anchor (they always win when a start is set). A legacy end that
/// fails a plausibility check is dropped and noted.
fn student_anchor(
    data: &CourseTermData,
    origin: CalendarOrigin,
    ctx: &Context<'_>,
    not_used: &mut Vec<RejectedDates>,
) -> Option<Anchor> {
    let start = data.user_term_start?;
    let mut end = data.user_term_end;
    if origin == CalendarOrigin::Legacy
        && let Some(legacy_end) = end
        && let Some(reason) = end_problem(start, legacy_end, ctx)
    {
        not_used.push(RejectedDates {
            source: TermAnchorSource::StudentConfirmed,
            start: None,
            end: Some(legacy_end),
            reason,
            end_only: true,
        });
        end = None;
    }
    Some(Anchor {
        source: TermAnchorSource::StudentConfirmed,
        start,
        end,
        confidence: Confidence::High,
        origin: Some(origin),
        end_clipped: false,
        fit_weeks: None,
    })
}

/// The accepted calendar as the anchor: the student confirmed it, whoever read the dates.
fn calendar_anchor(in_force: &CalendarInForce) -> Option<Anchor> {
    let segments = &in_force.calendar.segments;
    let first = segments.first()?;
    Some(Anchor {
        source: TermAnchorSource::StudentConfirmed,
        start: first.first_class,
        end: segments.last().and_then(|s| s.last_class),
        // The student confirmed it, whoever read the dates (a scan too); V8 disagreement at
        // acceptance, or a quote gone from a changed material since, caps it at Medium.
        confidence: if in_force.disagrees_with_notes || in_force.stale {
            Confidence::Medium
        } else {
            Confidence::High
        },
        origin: Some(in_force.origin),
        end_clipped: false,
        fit_weeks: None,
    })
}

/// The plausibility checks of §6.3.
fn check(start: NaiveDate, end: Option<NaiveDate>, ctx: &Context<'_>) -> Verdict {
    if let Some(end) = end {
        if start > end {
            return Verdict::Reject(RejectReason::StartsAfterEnd);
        }
        let days = days_between(start, end) + 1;
        let max = if ctx.full_year {
            MAX_YEAR_TERM_DAYS
        } else {
            MAX_TERM_DAYS
        };
        if days > max {
            return Verdict::Reject(RejectReason::LongerThanTeachingTerm);
        }
        if days < MIN_TERM_DAYS {
            return Verdict::Reject(RejectReason::ShorterThanTeachingTerm);
        }
    }
    if let Some(first) = ctx.first_activity
        && days_between(start, first) > MAX_DAYS_BEFORE_ACTIVITY
    {
        return Verdict::Reject(RejectReason::StartsLongBeforeActivity);
    }
    if let Some(session) = ctx.session
        && days_between(start, session.window.start) > MAX_DAYS_BEFORE_WINDOW
    {
        return Verdict::Reject(RejectReason::StartsBeforeSessionWindow);
    }
    let clipped = match (end, ctx.session) {
        (Some(end), Some(session)) if end_outside_window(end, session) => {
            Some((end, RejectReason::EndOutsideSessionWindow))
        }
        _ => None,
    };
    let end = match clipped {
        Some(_) => ctx.session.map(|s| s.window.end),
        None => end,
    };
    Verdict::Use { end, clipped }
}

/// §6.3 end clause: more than 28 days after the window, or (full-year section) more than 28
/// days before its end.
fn end_outside_window(end: NaiveDate, session: &SessionHint) -> bool {
    let after = days_between(session.window.end, end);
    after > MAX_END_DAYS_OUTSIDE_WINDOW
        || (session.full_year && -after > MAX_END_DAYS_OUTSIDE_WINDOW)
}

/// Why a legacy start → end fails (the start itself is always kept).
fn end_problem(start: NaiveDate, end: NaiveDate, ctx: &Context<'_>) -> Option<RejectReason> {
    if start > end {
        return Some(RejectReason::StartsAfterEnd);
    }
    let days = days_between(start, end) + 1;
    let max = if ctx.full_year {
        MAX_YEAR_TERM_DAYS
    } else {
        MAX_TERM_DAYS
    };
    if days > max {
        return Some(RejectReason::LongerThanTeachingTerm);
    }
    if days < MIN_TERM_DAYS {
        return Some(RejectReason::ShorterThanTeachingTerm);
    }
    match ctx.session {
        Some(session) if end_outside_window(end, session) => {
            Some(RejectReason::EndOutsideSessionWindow)
        }
        _ => None,
    }
}

/// The end a clipped span uses instead (the session window's end).
fn outer_until(ctx: &Context<'_>) -> Option<NaiveDate> {
    ctx.session.map(|s| s.window.end)
}

fn one_level_down(confidence: Confidence) -> Confidence {
    match confidence {
        Confidence::High => Confidence::Medium,
        Confidence::Medium | Confidence::Low => Confidence::Low,
    }
}

fn union(a: Option<DateSpan>, b: Option<DateSpan>) -> Option<DateSpan> {
    match (a, b) {
        (Some(a), Some(b)) => Some(DateSpan {
            start: a.start.min(b.start),
            end: a.end.max(b.end),
        }),
        (a, b) => a.or(b),
    }
}

/// Earliest and latest activity on or before today: materials (incl. announcements), module
/// unlocks and events, as course-local dates.
fn activity(input: &TermInput<'_>, tz: Option<Tz>) -> (Option<NaiveDate>, Option<NaiveDate>) {
    let dates = input
        .materials
        .iter()
        .filter_map(|m| m.published_at)
        .chain(input.modules.iter().filter_map(|m| m.unlock_at))
        .chain(input.events.iter().filter_map(|e| e.when()))
        .map(|instant| course_date(instant, tz))
        .filter(|date| *date <= input.today);
    let mut first = None;
    let mut last = None;
    for date in dates {
        first = Some(first.map_or(date, |f: NaiveDate| f.min(date)));
        last = Some(last.map_or(date, |l: NaiveDate| l.max(date)));
    }
    (first, last)
}

/// The item naming the dates used.
fn anchor_item(anchor: &Anchor, term_name: Option<&str>) -> EvidenceItem {
    let end = anchor.end;
    match anchor.source {
        TermAnchorSource::StudentConfirmed => {
            let code = if anchor.origin == Some(CalendarOrigin::Legacy) {
                EvidenceCode::LegacyDates
            } else {
                EvidenceCode::StudentDates
            };
            EvidenceItem::new(code)
                .date("start", anchor.start)
                .opt_date("end", end)
        }
        TermAnchorSource::LmsCourseDates => EvidenceItem::new(EvidenceCode::LmsCourseDates)
            .date("start", anchor.start)
            .opt_date("end", end),
        TermAnchorSource::LmsTerm => EvidenceItem::new(EvidenceCode::LmsTermDates)
            .opt_text("term_name", term_name)
            .date("start", anchor.start)
            .opt_date("end", end),
        TermAnchorSource::PublishedWeekLabels => EvidenceItem::new(EvidenceCode::WeekLabelsFit)
            .date("monday", anchor.start)
            .number(
                "weeks",
                i64::try_from(anchor.fit_weeks.unwrap_or_default()).unwrap_or_default(),
            ),
        TermAnchorSource::FolderConfig
        | TermAnchorSource::InstitutionCalendar
        | TermAnchorSource::NoAnchor => EvidenceItem::new(EvidenceCode::FolderDates)
            .date("start", anchor.start)
            .opt_date("end", end),
    }
}

fn rejected_item(
    rejected: &RejectedDates,
    term_name: Option<&str>,
    until: Option<NaiveDate>,
) -> EvidenceItem {
    if rejected.end_only {
        return EvidenceItem::new(EvidenceCode::EndNotUsed)
            .source(rejected.source)
            .opt_date("end", rejected.end)
            .opt_date(
                "until",
                until.filter(|_| {
                    rejected.reason == RejectReason::EndOutsideSessionWindow
                        && rejected.source != TermAnchorSource::StudentConfirmed
                }),
            )
            .reason(rejected.reason);
    }
    if rejected.is_enrollment_window()
        && let (Some(start), Some(end)) = (rejected.start, rejected.end)
    {
        let weeks = (days_between(start, end) + 1 + 6) / 7;
        return EvidenceItem::new(EvidenceCode::TermLooksLikeEnrollmentWindow)
            .opt_text("term_name", term_name)
            .date("start", start)
            .date("end", end)
            .number("weeks", weeks);
    }
    EvidenceItem::new(EvidenceCode::DatesNotUsed)
        .source(rejected.source)
        .opt_date("start", rejected.start)
        .opt_date("end", rejected.end)
        .reason(rejected.reason)
}
