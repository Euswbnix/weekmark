//! Course calendars: the dates a student accepted (or typed) for a course, as one validated
//! structure (docs/design/v0.3-course-calendar.md §3.3, §7). Pure functions only.
//!
//! alpha.2 (B6) brings the rest: the dates form, the validator and the resolver using an
//! accepted calendar as its first anchor. This module starts with the types and
//! `legacy_calendar`, which the schema-v4 migration calls for PageLamp 0.1 term overrides.

pub mod app_proposal;
pub mod assemble;
pub mod candidates;
pub mod extraction;
pub mod proposal;
pub mod reading;
pub mod scan;
pub mod text;
pub mod validate;

use chrono::{Datelike, NaiveDate, Weekday};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dates::{add_days, days_between};
use crate::term::phase::teaching_week_on;
use crate::term::{
    AiLabel, BreakKind, CalendarBreak, CalendarOrigin, DateSpan, MAX_YEAR_TERM_DAYS, MIN_TERM_DAYS,
    TeachingSegment, TermResolution,
};

/// An accepted or proposed course calendar (built by core, stored as `calendar_json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CourseCalendar {
    /// One for a one-term course, two for a full-year course.
    pub segments: Vec<TeachingSegment>,
    pub breaks: Vec<CalendarBreak>,
    pub exam_period: Option<DateSpan>,
    /// A fixed label only, never requirements (rule 4).
    pub final_exam_on: Option<NaiveDate>,
    /// Optional per-week rows from a schedule table.
    pub weeks: Vec<CalendarWeek>,
}

/// One row of a schedule table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CalendarWeek {
    pub number: u32,
    pub starts_on: NaiveDate,
    /// Plain text from the course material (rule 8: material text), at most 120 characters.
    pub topic: Option<String>,
}

/// The calendar in force for a course (its accepted `course_calendars` row), as the resolver
/// reads it: the student-confirmed anchor of highest priority (design §6.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarInForce {
    pub calendar: CourseCalendar,
    pub origin: CalendarOrigin,
    /// Set when the dates came from an AI (origin `ai` or `ai_app`).
    pub ai_label: Option<AiLabel>,
    /// The proposal's V8 cross-checks disagreed (stored with it): the calendar counts weeks at
    /// Medium, not High (design §7.5 V8).
    pub disagrees_with_notes: bool,
    /// A quoted material changed and a quote is gone (§7.8): still in force, at Medium.
    pub stale: bool,
}

/// The course dates form, as core validates it (the facade's `CourseDatesInput`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatesDraft {
    pub first_class: NaiveDate,
    pub last_class: Option<NaiveDate>,
    pub exams_end: Option<NaiveDate>,
    pub breaks: Vec<CalendarBreak>,
    pub second_segment: Option<SecondSegment>,
}

/// The second half of a full-year course.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecondSegment {
    pub first_class: NaiveDate,
    pub last_class: Option<NaiveDate>,
    /// True: its first week is week 1; false: the numbering continues after the first half.
    pub restart_numbering: bool,
}

/// What is wrong with a dates draft (codes for UIs; `message()` for people).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DatesProblem {
    /// The last day of classes is before the first.
    LastClassBeforeFirst,
    /// The end of exams is before the last day of classes.
    ExamsEndBeforeLastClass,
    /// An end of exams needs a last day of classes.
    ExamsEndWithoutLastClass,
    /// The exams end more than 8 weeks after the last class.
    ExamsTooLong,
    /// The course (both halves) is longer than 36 weeks.
    CourseTooLong,
    /// The second half starts before the first half ends (or the first has no end).
    SecondSegmentOverlaps,
    SecondSegmentLastClassBeforeFirst,
    /// A break ends before it starts.
    BreakReversed,
    /// A break lies outside the teaching dates.
    BreakOutsideCourse,
    /// A break is longer than 21 days (a winter break: 35).
    BreakTooLong,
    /// More than 4 breaks of 3 or more weekdays.
    TooManyBreaks,
}

impl DatesProblem {
    pub fn message(self) -> &'static str {
        match self {
            DatesProblem::LastClassBeforeFirst => "the last day of classes is before the first",
            DatesProblem::ExamsEndBeforeLastClass => {
                "the end of exams is before the last day of classes"
            }
            DatesProblem::ExamsEndWithoutLastClass => {
                "set the last day of classes before the end of exams"
            }
            DatesProblem::ExamsTooLong => "exams end more than 8 weeks after the last class",
            DatesProblem::CourseTooLong => "the course is longer than 36 weeks",
            DatesProblem::SecondSegmentOverlaps => {
                "the second half must start after the first half's last day of classes"
            }
            DatesProblem::SecondSegmentLastClassBeforeFirst => {
                "the second half's last day of classes is before its first"
            }
            DatesProblem::BreakReversed => "a break ends before it starts",
            DatesProblem::BreakOutsideCourse => "a break lies outside the course's dates",
            DatesProblem::BreakTooLong => {
                "a break is longer than 3 weeks (5 weeks for a winter break)"
            }
            DatesProblem::TooManyBreaks => "more than 4 breaks of 3 or more weekdays",
        }
    }
}

/// Longest exam period after the last class in the dates form (8 weeks).
const MAX_EXAM_DAYS: i64 = 56;
/// Longest break (21 days; a winter break 35).
const MAX_BREAK_DAYS: i64 = 21;
const MAX_WINTER_BREAK_DAYS: i64 = 35;
/// At most this many breaks of 3 or more weekdays (shorter holidays don't count).
const MAX_BREAKS: usize = 4;

/// The calendar a student's dates form describes (design §7.10), or every problem with it.
/// The student's own dates are trusted within these sanity checks; labels are cleaned
/// (control characters removed, 80 characters at most). A second half continues the week
/// numbering after the first half unless it restarts at 1.
pub fn calendar_from_dates(draft: &DatesDraft) -> Result<CourseCalendar, Vec<DatesProblem>> {
    let mut problems = Vec::new();
    let first = TeachingSegment {
        first_class: draft.first_class,
        last_class: draft.last_class,
        first_week_number: 1,
    };
    if draft
        .last_class
        .is_some_and(|last| last < draft.first_class)
    {
        problems.push(DatesProblem::LastClassBeforeFirst);
    }
    let mut segments = vec![first.clone()];
    if let Some(second) = &draft.second_segment {
        if first
            .last_class
            .is_none_or(|last| second.first_class <= last)
        {
            problems.push(DatesProblem::SecondSegmentOverlaps);
        }
        if second
            .last_class
            .is_some_and(|last| last < second.first_class)
        {
            problems.push(DatesProblem::SecondSegmentLastClassBeforeFirst);
        }
        segments.push(TeachingSegment {
            first_class: second.first_class,
            last_class: second.last_class,
            first_week_number: 1,
        });
    }
    let course_last = segments.last().and_then(|s| s.last_class);
    if let Some(last) = course_last
        && days_between(draft.first_class, last) + 1 > MAX_YEAR_TERM_DAYS
    {
        problems.push(DatesProblem::CourseTooLong);
    }
    let exam_period = match (draft.exams_end, course_last) {
        (None, _) => None,
        (Some(_), None) => {
            problems.push(DatesProblem::ExamsEndWithoutLastClass);
            None
        }
        (Some(end), Some(last)) => {
            if end < last {
                problems.push(DatesProblem::ExamsEndBeforeLastClass);
            } else if days_between(last, end) > MAX_EXAM_DAYS {
                problems.push(DatesProblem::ExamsTooLong);
            }
            Some(DateSpan {
                start: add_days(last, 1).min(end),
                end,
            })
        }
    };
    let course_span_end = draft.exams_end.or(course_last);
    let mut long_breaks = 0;
    let mut breaks = Vec::new();
    for b in &draft.breaks {
        if b.span.end < b.span.start {
            problems.push(DatesProblem::BreakReversed);
            continue;
        }
        let inside = b.span.start >= draft.first_class
            && course_span_end.is_none_or(|end| b.span.end <= end);
        if !inside {
            problems.push(DatesProblem::BreakOutsideCourse);
        }
        let max = if b.kind == BreakKind::WinterBreak {
            MAX_WINTER_BREAK_DAYS
        } else {
            MAX_BREAK_DAYS
        };
        if days_between(b.span.start, b.span.end) + 1 > max {
            problems.push(DatesProblem::BreakTooLong);
        }
        if weekdays_in(b.span) >= 3 {
            long_breaks += 1;
        }
        breaks.push(CalendarBreak {
            kind: b.kind,
            span: b.span,
            numbered: b.numbered,
            label: crate::term::evidence::plain_text(&b.label),
        });
    }
    if long_breaks > MAX_BREAKS {
        problems.push(DatesProblem::TooManyBreaks);
    }
    problems.dedup();
    if !problems.is_empty() {
        return Err(problems);
    }
    breaks.sort_by_key(|b| b.span.start);
    // A second half that continues the numbering starts after the first half's last week.
    if let Some(second) = &draft.second_segment
        && !second.restart_numbering
    {
        // Only breaks inside the first half count (the gap between the halves is the break
        // between them, not part of the first half's numbering).
        let first_last = segments[0].last_class;
        let first_only = TermResolution {
            teaching: vec![segments[0].clone()],
            breaks: breaks
                .iter()
                .filter(|b| first_last.is_some_and(|last| b.span.end <= last))
                .cloned()
                .collect(),
            ..TermResolution::default()
        };
        let last_week = segments[0]
            .last_class
            .and_then(|last| teaching_week_on(&first_only, last))
            .unwrap_or(0);
        segments[1].first_week_number = last_week + 1;
    }
    Ok(CourseCalendar {
        segments,
        breaks,
        exam_period,
        final_exam_on: None,
        weeks: Vec::new(),
    })
}

/// Monday-to-Friday days inside `span`.
fn weekdays_in(span: DateSpan) -> i64 {
    let days = days_between(span.start, span.end) + 1;
    (0..days.clamp(0, 400))
        .map(|i| add_days(span.start, i))
        .filter(|day| !matches!(day.weekday(), Weekday::Sat | Weekday::Sun))
        .count() as i64
}

/// The calendar for a term override set in PageLamp 0.1 (`user_term_start`, `user_term_end`)
/// that survived the v3 clean-up (design §3.2): the start becomes the first class, and the
/// end a last day of classes ("end only", §6.6).
///
/// The start is always kept (the student typed it). The end is dropped when start → end
/// isn't a plausible teaching span: it is before the start, shorter than `MIN_TERM_DAYS`, or
/// longer than `MAX_YEAR_TERM_DAYS` (the student's own dates are allowed full-year length;
/// the 0.1 prefill of a whole enrollment window is longer and was cleared by v3 anyway).
pub fn legacy_calendar(start: NaiveDate, end: Option<NaiveDate>) -> CourseCalendar {
    let end = end.filter(|end| {
        let days = days_between(start, *end) + 1;
        (MIN_TERM_DAYS..=MAX_YEAR_TERM_DAYS).contains(&days)
    });
    CourseCalendar {
        segments: vec![TeachingSegment {
            first_class: start,
            last_class: end,
            first_week_number: 1,
        }],
        breaks: Vec::new(),
        exam_period: None,
        final_exam_on: None,
        weeks: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn legacy_calendar_keeps_the_start_and_a_plausible_end() {
        let calendar = legacy_calendar(date(2026, 9, 8), Some(date(2026, 12, 8)));
        assert_eq!(
            calendar.segments,
            [TeachingSegment {
                first_class: date(2026, 9, 8),
                last_class: Some(date(2026, 12, 8)),
                first_week_number: 1,
            }]
        );
        assert!(calendar.breaks.is_empty() && calendar.weeks.is_empty());
        assert_eq!((calendar.exam_period, calendar.final_exam_on), (None, None));
        // A full-year span typed by the student is kept.
        let year = legacy_calendar(date(2026, 9, 8), Some(date(2027, 4, 9)));
        assert_eq!(year.segments[0].last_class, Some(date(2027, 4, 9)));
    }

    #[test]
    fn legacy_calendar_drops_an_implausible_end() {
        for end in [
            date(2026, 9, 1),  // before the start
            date(2026, 9, 20), // two weeks
            date(2027, 8, 31), // a year
        ] {
            let calendar = legacy_calendar(date(2026, 9, 8), Some(end));
            assert_eq!(calendar.segments[0].first_class, date(2026, 9, 8));
            assert_eq!(calendar.segments[0].last_class, None, "{end}");
        }
        assert_eq!(
            legacy_calendar(date(2026, 9, 8), None).segments[0].last_class,
            None
        );
    }

    #[test]
    fn calendar_json_round_trips() {
        let calendar = legacy_calendar(date(2026, 9, 8), Some(date(2026, 12, 8)));
        let json = serde_json::to_string(&calendar).unwrap();
        assert_eq!(
            json,
            r#"{"segments":[{"first_class":"2026-09-08","last_class":"2026-12-08","first_week_number":1}],"breaks":[],"exam_period":null,"final_exam_on":null,"weeks":[]}"#
        );
        let back: CourseCalendar = serde_json::from_str(&json).unwrap();
        assert_eq!(back, calendar);
    }
}
