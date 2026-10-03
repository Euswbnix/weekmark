//! Teaching weeks and the phase on a date (docs/design/v0.3-course-calendar.md §6.6).
//!
//! - A teaching week inside a segment = the segment's `first_week_number` + whole weeks from
//!   the Monday of its first class to the Monday of the date, skipping break weeks that don't
//!   count in the numbering.
//! - A break week has at least 3 weekdays inside a break; shorter holidays don't change the
//!   phase.
//! - After the last class: the exam period until `exams_end` (without one: last class + 21
//!   days, Low), then Ended.
//! - Only a start: Teaching for up to 16 weeks (30 with full-year evidence), then Unknown.

use chrono::NaiveDate;

use super::{BreakKind, CalendarBreak, CoursePhase, TeachingSegment, TermResolution};
use crate::dates::{add_days, days_between, monday_of, week_one_monday};
use crate::model::Confidence;

/// Without exam dates, the exam period lasts this long after the last class.
pub const EXAM_PERIOD_DAYS: i64 = 21;
/// A start with no end counts weeks for this long…
pub const MAX_OPEN_TEACHING_WEEKS: u32 = 16;
/// …or this long with full-year evidence.
pub const MAX_OPEN_TEACHING_WEEKS_FULL_YEAR: u32 = 30;
/// Weekdays inside a break that make its week a break week.
const BREAK_WEEKDAYS: u32 = 3;

/// The phase on one day, with the numbers the timeline shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PhaseState {
    pub phase: CoursePhase,
    pub confidence: Confidence,
    /// Today's teaching week: Teaching, or a break that counts in the numbering.
    pub week: Option<u32>,
    pub break_after_week: Option<u32>,
    pub last_teaching_week: Option<u32>,
    pub break_kind: Option<BreakKind>,
    /// When classes start (NotStarted).
    pub starts_on: Option<NaiveDate>,
    /// Exam period: the last class. Ended: the day it ended.
    pub since: Option<NaiveDate>,
    /// Exam period: its last day; `estimated` when there is no exam date.
    pub until: Option<NaiveDate>,
    pub estimated: bool,
    /// Unknown because only an old start is known: (start, weeks since).
    pub start_too_old: Option<(NaiveDate, u32)>,
}

impl PhaseState {
    fn new(phase: CoursePhase, confidence: Confidence) -> Self {
        PhaseState {
            phase,
            confidence,
            week: None,
            break_after_week: None,
            last_teaching_week: None,
            break_kind: None,
            starts_on: None,
            since: None,
            until: None,
            estimated: false,
            start_too_old: None,
        }
    }
}

/// The phase of a course on `today`. `confidence` is the anchor's (after cross-checks);
/// `end_clipped` (an end replaced by the session window) makes the phase Low. `end_only` is a
/// last day of classes known without any start (the student's): Unknown up to it, then the
/// exam period (Low) for 21 days, then Ended.
pub(crate) fn phase_on(
    term: &TermResolution,
    confidence: Confidence,
    today: NaiveDate,
    full_year: bool,
    end_clipped: bool,
    end_only: Option<NaiveDate>,
) -> PhaseState {
    let confidence = if end_clipped {
        Confidence::Low
    } else {
        confidence
    };
    let Some(first) = term.teaching.first() else {
        return match end_only {
            Some(last) if today > last => {
                let end = add_days(last, EXAM_PERIOD_DAYS);
                if today <= end {
                    let mut state = PhaseState::new(CoursePhase::ExamPeriod, Confidence::Low);
                    state.since = Some(last);
                    state.until = Some(end);
                    state.estimated = true;
                    state
                } else {
                    let mut state = PhaseState::new(CoursePhase::Ended, Confidence::Low);
                    state.since = Some(end);
                    state
                }
            }
            _ => PhaseState::new(CoursePhase::Unknown, Confidence::Low),
        };
    };
    if today < week_one_monday(first.first_class) {
        let mut state = PhaseState::new(CoursePhase::NotStarted, confidence);
        state.starts_on = Some(first.first_class.max(week_one_monday(first.first_class)));
        return state;
    }
    let segments = &term.teaching;
    for (index, segment) in segments.iter().enumerate() {
        let next = segments.get(index + 1);
        let in_segment = segment.last_class.is_none_or(|last| today <= last);
        if in_segment {
            return inside_segment(term, segment, confidence, today, full_year);
        }
        if let Some(next) = next
            && today < week_one_monday(next.first_class)
        {
            // Between two segments of a full-year course.
            let mut state = PhaseState::new(CoursePhase::Break, confidence);
            state.break_kind = Some(BreakKind::WinterBreak);
            state.break_after_week = segment
                .last_class
                .and_then(|last| teaching_week_in(term, segment, last));
            return state;
        }
        if next.is_some() {
            continue;
        }
        // After the last class of the last segment.
        let last = segment
            .last_class
            .expect("in_segment is false only with a last class");
        let last_week = teaching_week_in(term, segment, last);
        let (end, estimated) = match term.exams_end {
            Some(end) if end >= last => (end, false),
            _ => (add_days(last, EXAM_PERIOD_DAYS), true),
        };
        if today <= end {
            let mut state = PhaseState::new(
                CoursePhase::ExamPeriod,
                if estimated {
                    Confidence::Low
                } else {
                    confidence
                },
            );
            state.last_teaching_week = last_week;
            state.since = Some(last);
            state.until = Some(end);
            state.estimated = estimated;
            return state;
        }
        let mut state = PhaseState::new(CoursePhase::Ended, confidence);
        state.since = Some(end);
        state.last_teaching_week = last_week;
        return state;
    }
    PhaseState::new(CoursePhase::Unknown, Confidence::Low)
}

fn inside_segment(
    term: &TermResolution,
    segment: &TeachingSegment,
    confidence: Confidence,
    today: NaiveDate,
    full_year: bool,
) -> PhaseState {
    let week = teaching_week_in(term, segment, today);
    if segment.last_class.is_none() {
        let limit = if full_year {
            MAX_OPEN_TEACHING_WEEKS_FULL_YEAR
        } else {
            MAX_OPEN_TEACHING_WEEKS
        };
        let since = weeks_since(segment.first_class, today);
        if since > limit {
            let mut state = PhaseState::new(CoursePhase::Unknown, Confidence::Low);
            state.start_too_old = Some((segment.first_class, since));
            return state;
        }
    }
    if let Some(found) = break_of_week(&term.breaks, monday_of(today)) {
        let mut state = PhaseState::new(CoursePhase::Break, confidence);
        state.break_kind = Some(found.kind);
        if found.numbered {
            state.week = week;
        } else {
            state.break_after_week = last_teaching_week_before(term, segment, monday_of(today));
        }
        return state;
    }
    let mut state = PhaseState::new(CoursePhase::Teaching, confidence);
    state.week = week;
    state
}

/// Whole weeks from week 1 (starting `start`) to the week of `date`, plus one.
fn weeks_since(start: NaiveDate, date: NaiveDate) -> u32 {
    let weeks = days_between(week_one_monday(start), monday_of(date)) / 7 + 1;
    u32::try_from(weeks.max(0)).unwrap_or(u32::MAX)
}

/// The teaching week of `date` in `segment` (None before the segment's first week, or in a
/// break week that doesn't count).
fn teaching_week_in(
    term: &TermResolution,
    segment: &TeachingSegment,
    date: NaiveDate,
) -> Option<u32> {
    let first = week_one_monday(segment.first_class);
    let target = monday_of(date);
    if target < first {
        return None;
    }
    if break_of_week(&term.breaks, target).is_some_and(|b| !b.numbered) {
        return None;
    }
    let weeks = days_between(first, target) / 7;
    let skipped = (0..weeks)
        .filter(|i| {
            let monday = add_days(first, i * 7);
            break_of_week(&term.breaks, monday).is_some_and(|b| !b.numbered)
        })
        .count();
    let weeks = u32::try_from(weeks).ok()?;
    let skipped = u32::try_from(skipped).ok()?;
    Some(segment.first_week_number + weeks - skipped)
}

/// The last teaching week before the week starting `monday`.
fn last_teaching_week_before(
    term: &TermResolution,
    segment: &TeachingSegment,
    monday: NaiveDate,
) -> Option<u32> {
    let mut week = add_days(monday, -7);
    while week >= week_one_monday(segment.first_class) {
        if let Some(number) = teaching_week_in(term, segment, week) {
            return Some(number);
        }
        week = add_days(week, -7);
    }
    None
}

/// The break that makes the week starting `monday` a break week, if any.
fn break_of_week(breaks: &[CalendarBreak], monday: NaiveDate) -> Option<&CalendarBreak> {
    breaks.iter().find(|b| {
        (0..5)
            .map(|i| add_days(monday, i))
            .filter(|day| b.span.contains(*day))
            .count() as u32
            >= BREAK_WEEKDAYS
    })
}

/// The teaching week that `date` falls in, by the resolution's segments (None outside them or
/// in an unnumbered break). Used to place materials without a week number.
pub fn teaching_week_on(term: &TermResolution, date: NaiveDate) -> Option<u32> {
    let segment = term
        .teaching
        .iter()
        .rev()
        .find(|s| week_one_monday(s.first_class) <= date)?;
    if segment.last_class.is_some_and(|last| date > last) {
        return None;
    }
    teaching_week_in(term, segment, date)
}

/// The Monday week `week` starts on, by the resolution's segments and breaks (the inverse of
/// `teaching_week_on`); None when no Monday of the term falls in that week. Saved explanations
/// keep it (`generations.week_starts_on`): a calendar that moves the week makes them stale.
pub fn week_starts_on(term: &TermResolution, week: u32) -> Option<NaiveDate> {
    let first = term
        .teaching
        .iter()
        .map(|segment| week_one_monday(segment.first_class))
        .min()?;
    // A year and a half of Mondays covers every term (a full-year course has at most 36
    // weeks of teaching plus its breaks).
    (0..80)
        .map(|n| first + chrono::Duration::weeks(n))
        .find(|monday| teaching_week_on(term, *monday) == Some(week))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{DateSpan, TermAnchorSource};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn term(first: NaiveDate, last: Option<NaiveDate>) -> TermResolution {
        TermResolution {
            week_one_monday: Some(monday_of(first)),
            teaching: vec![TeachingSegment {
                first_class: first,
                last_class: last,
                first_week_number: 1,
            }],
            anchor: TermAnchorSource::LmsCourseDates,
            anchor_confidence: Confidence::Medium,
            ..TermResolution::default()
        }
    }

    fn phase(term: &TermResolution, today: NaiveDate) -> PhaseState {
        phase_on(term, Confidence::Medium, today, false, false, None)
    }

    #[test]
    fn weeks_are_monday_aligned() {
        // Classes start on a Thursday: that week is week 1, the next Monday starts week 2.
        let t = term(date(2026, 9, 10), Some(date(2026, 12, 8)));
        assert_eq!(phase(&t, date(2026, 9, 7)).week, Some(1));
        assert_eq!(phase(&t, date(2026, 9, 13)).week, Some(1));
        assert_eq!(phase(&t, date(2026, 9, 14)).week, Some(2));
        assert_eq!(phase(&t, date(2026, 9, 28)).week, Some(4));
        let before = phase(&t, date(2026, 9, 6));
        assert_eq!(before.phase, CoursePhase::NotStarted);
        assert_eq!(before.starts_on, Some(date(2026, 9, 10)));
    }

    /// CAL-9 `end_only_anchor_gives_exam_period`: a plausible end and no exam dates.
    #[test]
    fn end_only_anchor_gives_exam_period() {
        let t = term(date(2026, 9, 8), Some(date(2026, 12, 8)));
        let exams = phase(&t, date(2026, 12, 16));
        assert_eq!(exams.phase, CoursePhase::ExamPeriod);
        assert_eq!(exams.confidence, Confidence::Low);
        assert_eq!(exams.last_teaching_week, Some(14));
        assert_eq!(exams.until, Some(date(2026, 12, 29)));
        assert!(exams.estimated);
        let ended = phase(&t, date(2026, 12, 30));
        assert_eq!(ended.phase, CoursePhase::Ended);
        assert_eq!(ended.since, Some(date(2026, 12, 29)));
        assert_eq!(phase(&t, date(2026, 12, 8)).phase, CoursePhase::Teaching);
    }

    #[test]
    fn exam_dates_end_the_exam_period() {
        let mut t = term(date(2026, 9, 8), Some(date(2026, 12, 8)));
        t.exams_end = Some(date(2026, 12, 22));
        let exams = phase_on(&t, Confidence::High, date(2026, 12, 15), false, false, None);
        assert_eq!(
            (exams.phase, exams.confidence),
            (CoursePhase::ExamPeriod, Confidence::High)
        );
        assert_eq!(phase(&t, date(2026, 12, 23)).phase, CoursePhase::Ended);
    }

    #[test]
    fn a_start_alone_stops_counting_after_sixteen_weeks() {
        let t = term(date(2026, 9, 7), None);
        assert_eq!(phase(&t, date(2026, 12, 21)).week, Some(16));
        let unknown = phase(&t, date(2026, 12, 28));
        assert_eq!(unknown.phase, CoursePhase::Unknown);
        assert_eq!(unknown.start_too_old, Some((date(2026, 9, 7), 17)));
        // Thirty weeks with full-year evidence.
        let y = phase_on(&t, Confidence::Medium, date(2027, 3, 1), true, false, None);
        assert_eq!((y.phase, y.week), (CoursePhase::Teaching, Some(26)));
    }

    #[test]
    fn a_clipped_end_makes_the_phase_low() {
        let t = term(date(2026, 9, 8), Some(date(2026, 12, 31)));
        let state = phase_on(&t, Confidence::Medium, date(2026, 10, 1), false, true, None);
        assert_eq!(
            (state.phase, state.confidence),
            (CoursePhase::Teaching, Confidence::Low)
        );
    }

    #[test]
    fn break_weeks_need_three_weekdays_and_may_not_count() {
        let mut t = term(date(2026, 9, 8), Some(date(2026, 12, 8)));
        t.breaks = vec![
            CalendarBreak {
                kind: BreakKind::ReadingWeek,
                span: DateSpan {
                    start: date(2026, 10, 26),
                    end: date(2026, 10, 30),
                },
                numbered: false,
                label: String::new(),
            },
            CalendarBreak {
                kind: BreakKind::Holiday,
                span: DateSpan {
                    start: date(2026, 10, 12),
                    end: date(2026, 10, 12),
                },
                numbered: false,
                label: String::new(),
            },
        ];
        // One-day holiday: still Teaching, week 6.
        let holiday = phase(&t, date(2026, 10, 12));
        assert_eq!(
            (holiday.phase, holiday.week),
            (CoursePhase::Teaching, Some(6))
        );
        // Reading week: Break after week 7.
        let reading = phase(&t, date(2026, 10, 28));
        assert_eq!(reading.phase, CoursePhase::Break);
        assert_eq!(reading.week, None);
        assert_eq!(reading.break_after_week, Some(7));
        assert_eq!(reading.break_kind, Some(BreakKind::ReadingWeek));
        // The week after is week 8; numbered, it would be week 9.
        assert_eq!(phase(&t, date(2026, 11, 2)).week, Some(8));
        t.breaks[0].numbered = true;
        assert_eq!(phase(&t, date(2026, 11, 2)).week, Some(9));
        assert_eq!(phase(&t, date(2026, 10, 28)).week, Some(8));
        assert_eq!(teaching_week_on(&t, date(2026, 11, 3)), Some(9));
        assert_eq!(teaching_week_on(&t, date(2026, 12, 9)), None);
    }

    #[test]
    fn no_segment_is_unknown() {
        let t = TermResolution::default();
        assert_eq!(phase(&t, date(2026, 9, 28)).phase, CoursePhase::Unknown);
        assert_eq!(teaching_week_on(&t, date(2026, 9, 28)), None);
    }

    #[test]
    fn a_week_starts_on_its_monday_and_moves_with_the_calendar() {
        let term = term(date(2026, 9, 9), Some(date(2026, 12, 4)));
        assert_eq!(week_starts_on(&term, 1), Some(date(2026, 9, 7)));
        assert_eq!(week_starts_on(&term, 3), Some(date(2026, 9, 21)));
        assert_eq!(week_starts_on(&term, 40), None);
        let later = super::tests::term(date(2026, 9, 16), Some(date(2026, 12, 4)));
        assert_eq!(week_starts_on(&later, 3), Some(date(2026, 9, 28)));
    }
}
