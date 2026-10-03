//! From validated claims to a proposed course calendar (docs/design/v0.3-course-calendar.md
//! §7.5 V7–V9, §7.6). Pure.
//!
//! - V9: one field given different dates by different materials keeps them all as
//!   alternatives; the later-published material's date is the default (the student chooses).
//! - Assembly: first-class dates more than 3 weeks apart make the two segments of a
//!   full-year course; last days of classes go to the segment they follow; the week table
//!   decides each segment's first week number (a restart in segment 2, or "Week 0") and each
//!   break's `numbered` (a break that has a row of its own counts).
//! - V7: a claim inconsistent by itself (an over-long break or exam period) is dropped; a
//!   violation between claims (a last class before the first, exams that start far from the
//!   last class, a final exam outside the exam period, a break outside the course, a segment
//!   of the wrong length) becomes a conflict that keeps both, and the proposal isn't passing.
//! - V8: cross-checks with deterministic evidence: the week-1 fit from the professor's
//!   materials (≥ 3 weeks, 7+ days away), plausible LMS course dates (outside −7/+21 days),
//!   the earliest class event (more than ±3 days), and the week numbers of posted materials
//!   (under 50% agreement, ≥ 3 samples). Conflicts are shown, never dropped; once accepted the
//!   calendar is at most Medium (`disagrees_with_notes`).

use std::collections::BTreeMap;

use chrono::{Datelike, NaiveDate, Weekday};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::extraction::ClaimKind;
use super::validate::{DateEvidence, DropCount, DropReason, ValidDate, ValidWeek, Validated};
use super::{CalendarWeek, CourseCalendar};
use crate::dates::{add_days, days_between, week_one_monday};
use crate::model::Confidence;
use crate::term::evidence::EvidenceParam;
use crate::term::phase::{phase_on, teaching_week_on};
use crate::term::{
    BreakKind, CalendarBreak, CoursePhase, DateSpan, TeachingSegment, TermResolution,
};

/// First-class dates further apart than this start a second segment.
const SEGMENT_GAP_DAYS: i64 = 21;
/// V7 limits.
const MIN_SEGMENT_WEEKS: i64 = 4;
const MAX_SEGMENT_WEEKS: i64 = 20;
const MAX_COURSE_WEEKS: i64 = 36;
const MAX_BREAK_DAYS: i64 = 21;
const MAX_WINTER_BREAK_DAYS: i64 = 35;
const MAX_LONG_BREAKS: usize = 4;
const EXAMS_FROM_LAST_CLASS: std::ops::RangeInclusive<i64> = -3..=28;
const MAX_EXAM_DAYS: i64 = 28;
/// V8 limits.
const FIT_DISAGREE_DAYS: i64 = 7;
const MIN_FIT_WEEKS: usize = 3;
const LMS_BEFORE_DAYS: i64 = 7;
const LMS_AFTER_DAYS: i64 = 21;
const CLASS_EVENT_DAYS: i64 = 3;
const MIN_NOTE_SAMPLES: usize = 3;

/// What a proposed date is.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DateKind {
    FirstClass,
    LastClass,
    /// A break (a span; "break" is a keyword in Swift).
    BreakSpan,
    ExamPeriod,
    FinalExam,
    /// The start of a numbered week from a schedule table.
    WeekStart,
}

/// Another reading of the same field (V9), or an option in a conflict. Flat: no nesting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AlternativeDate {
    pub date: NaiveDate,
    pub end: Option<NaiveDate>,
    /// Material text (rule 8); empty for dates from PageLamp's own evidence.
    pub label: String,
    pub evidence: Vec<DateEvidence>,
}

/// One date of a proposal, with its quotes and the other readings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProposedDate {
    pub kind: DateKind,
    /// 0, or 1 for the second half of a full-year course.
    pub segment: u32,
    pub date: NaiveDate,
    pub end: Option<NaiveDate>,
    pub label: String,
    pub evidence: Vec<DateEvidence>,
    pub alternatives: Vec<AlternativeDate>,
    /// For `week_start`.
    pub week: Option<u32>,
    /// For `break_span`.
    pub break_kind: Option<BreakKind>,
    pub numbered: Option<bool>,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ConflictCode {
    /// A weekday or the stated term fits another year (V5): never recommended.
    SyllabusFromAnotherYear,
    /// Two or more claims contradict each other (V7).
    Inconsistent,
    /// The professor's week-numbered materials disagree (V8).
    DisagreesWithNotes,
    DisagreesWithLmsDates,
    DisagreesWithClassEvent,
    DiffersFromInstitutionCalendar,
}

/// "A choice between two": the options the student picks from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CalendarConflict {
    pub code: ConflictCode,
    pub kind: DateKind,
    pub segment: u32,
    pub options: Vec<AlternativeDate>,
}

/// What accepting would change (UIs translate the code; params follow the evidence-param
/// conventions, plus `from_phase` / `to_phase` holding `CoursePhase` values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeCode {
    /// No calendar in force yet.
    NewCalendar,
    /// `from`, `to`.
    FirstClassMoved,
    /// `from?`, `to`.
    LastClassMoved,
    /// `kind`, `start`, `end`.
    BreakAdded,
    /// `kind`, `start`, `end`.
    BreakRemoved,
    /// `from?`, `to`.
    ExamsEndMoved,
    /// `from?`, `to?` (weeks).
    WeekTodayChanges,
    /// `from_phase`, `to_phase`.
    PhaseChanges,
}

impl ChangeCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeCode::NewCalendar => "new_calendar",
            ChangeCode::FirstClassMoved => "first_class_moved",
            ChangeCode::LastClassMoved => "last_class_moved",
            ChangeCode::BreakAdded => "break_added",
            ChangeCode::BreakRemoved => "break_removed",
            ChangeCode::ExamsEndMoved => "exams_end_moved",
            ChangeCode::WeekTodayChanges => "week_today_changes",
            ChangeCode::PhaseChanges => "phase_changes",
        }
    }
}

/// One line of "what accepting would change": a code and its params (like `EvidenceItem`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CalendarChange {
    pub code: ChangeCode,
    pub params: Vec<EvidenceParam>,
}

impl CalendarChange {
    fn new(code: ChangeCode) -> Self {
        CalendarChange {
            code,
            params: Vec::new(),
        }
    }

    fn with(mut self, key: &str, value: impl ToString) -> Self {
        self.params.push(EvidenceParam {
            key: key.to_string(),
            value: value.to_string(),
        });
        self
    }

    fn opt(self, key: &str, value: Option<impl ToString>) -> Self {
        match value {
            Some(value) => self.with(key, value),
            None => self,
        }
    }
}

/// PageLamp's own evidence to cross-check against (V8).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CrossChecks {
    /// The week-1 Monday fitted from the professor's materials, and from how many weeks.
    pub fit: Option<(NaiveDate, usize)>,
    /// The LMS course's start, when plausible.
    pub lms_course_start: Option<NaiveDate>,
    /// The earliest class event linked to the course.
    pub first_class_event: Option<NaiveDate>,
    /// Week-numbered material observations: (posting day, week).
    pub observations: Vec<(NaiveDate, u32)>,
    /// The reading weeks of the school's calendar (§6.4 anchor 5), when it has the course's
    /// session.
    pub school_reading_weeks: Vec<DateSpan>,
}

/// What the assembler needs besides the validated claims.
#[derive(Clone, Debug)]
pub struct AssembleInput<'a> {
    pub validated: &'a Validated,
    pub checks: CrossChecks,
    /// The calendar in force, if any (for `changes`).
    pub current: Option<&'a CourseCalendar>,
    /// Today's week and phase as the course shows them now.
    pub current_week: Option<u32>,
    pub current_phase: CoursePhase,
    pub today: NaiveDate,
    pub full_year: bool,
}

/// A proposal ready to store and show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Assembled {
    pub calendar: CourseCalendar,
    pub dates: Vec<ProposedDate>,
    pub conflicts: Vec<CalendarConflict>,
    pub dropped: Vec<DropCount>,
    /// Many claims dropped, the week table dropped, or the stated term from another year.
    pub low_quality: bool,
    /// No conflicts and not low quality: "accept all that pass" may take it.
    pub passing: bool,
    /// V8 found disagreement: once accepted, the calendar is at most Medium.
    pub disagrees_with_notes: bool,
    pub resulting_week_today: Option<u32>,
    pub resulting_phase: CoursePhase,
    pub changes: Vec<CalendarChange>,
}

/// The run as a whole failed (V10): the reader's output is not usable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadOutput;

/// Group candidate dates of one field (V9): the default and the other readings.
#[derive(Clone, Debug)]
struct Field {
    date: NaiveDate,
    end: Option<NaiveDate>,
    label: String,
    evidence: Vec<DateEvidence>,
    alternatives: Vec<AlternativeDate>,
}

impl Field {
    fn from_dates(mut dates: Vec<&ValidDate>) -> Option<Field> {
        // The later-published material's reading is the default.
        dates.sort_by_key(|d| std::cmp::Reverse(d.published_at));
        let first = *dates.first()?;
        let mut field = Field {
            date: first.date,
            end: first.end,
            label: first.label.clone(),
            evidence: vec![first.evidence.clone()],
            alternatives: Vec::new(),
        };
        for other in &dates[1..] {
            if (other.date, other.end) == (field.date, field.end) {
                field.evidence.push(other.evidence.clone());
            } else if let Some(alt) = field
                .alternatives
                .iter_mut()
                .find(|a| (a.date, a.end) == (other.date, other.end))
            {
                alt.evidence.push(other.evidence.clone());
            } else {
                field.alternatives.push(AlternativeDate {
                    date: other.date,
                    end: other.end,
                    label: other.label.clone(),
                    evidence: vec![other.evidence.clone()],
                });
            }
        }
        Some(field)
    }

    fn as_option(&self) -> AlternativeDate {
        AlternativeDate {
            date: self.date,
            end: self.end,
            label: self.label.clone(),
            evidence: self.evidence.clone(),
        }
    }

    fn proposed(&self, kind: DateKind, segment: u32) -> ProposedDate {
        ProposedDate {
            kind,
            segment,
            date: self.date,
            end: self.end,
            label: self.label.clone(),
            evidence: self.evidence.clone(),
            alternatives: self.alternatives.clone(),
            week: None,
            break_kind: None,
            numbered: None,
        }
    }
}

/// Assemble a proposal from `input.validated` (§7.6), or `BadOutput` (V10).
pub fn assemble(input: &AssembleInput<'_>) -> Result<Assembled, BadOutput> {
    let validated = input.validated;
    if validated.bad_output {
        return Err(BadOutput);
    }
    let mut dropped: BTreeMap<DropReason, u32> = validated
        .dropped
        .iter()
        .map(|d| (d.reason, d.count))
        .collect();
    let mut conflicts: Vec<CalendarConflict> = Vec::new();
    let mut dates_out: Vec<ProposedDate> = Vec::new();
    let of_kind = |kind: ClaimKind| -> Vec<&ValidDate> {
        validated.dates.iter().filter(|d| d.kind == kind).collect()
    };

    // V5 findings are conflicts that are never recommended.
    for finding in &validated.another_year {
        conflicts.push(CalendarConflict {
            code: ConflictCode::SyllabusFromAnotherYear,
            kind: finding.kind.map_or(DateKind::WeekStart, date_kind),
            segment: 0,
            options: Vec::new(),
        });
    }
    if validated.stated_term_another_year {
        conflicts.push(CalendarConflict {
            code: ConflictCode::SyllabusFromAnotherYear,
            kind: DateKind::FirstClass,
            segment: 0,
            options: Vec::new(),
        });
    }

    // Segments: first classes, clustered (dates within 3 weeks are one field, V9).
    let mut firsts = of_kind(ClaimKind::FirstClass);
    firsts.sort_by_key(|d| d.date);
    let mut first_fields: Vec<Field> = Vec::new();
    let mut cluster: Vec<&ValidDate> = Vec::new();
    for date in firsts {
        if let Some(last) = cluster.last()
            && days_between(last.date, date.date) > SEGMENT_GAP_DAYS
        {
            first_fields.extend(Field::from_dates(std::mem::take(&mut cluster)));
        }
        cluster.push(date);
    }
    first_fields.extend(Field::from_dates(cluster));
    // No first-class claim: the first dated week 0/1 row (V10 guarantees one).
    let weeks = &validated.weeks;
    if first_fields.is_empty() {
        let row = weeks
            .iter()
            .filter(|w| w.week <= 1)
            .find(|w| w.starts_on.is_some())
            .ok_or(BadOutput)?;
        first_fields.push(Field {
            date: row.starts_on.ok_or(BadOutput)?,
            end: None,
            label: String::new(),
            evidence: vec![row.evidence.clone()],
            alternatives: Vec::new(),
        });
    }
    if first_fields.len() > 2 {
        for extra in first_fields.split_off(2) {
            conflicts.push(CalendarConflict {
                code: ConflictCode::Inconsistent,
                kind: DateKind::FirstClass,
                segment: 1,
                options: vec![first_fields[1].as_option(), extra.as_option()],
            });
        }
    }

    // Last classes go to the segment they follow.
    let segment_of = |date: NaiveDate| -> usize {
        first_fields
            .iter()
            .rposition(|f| f.date <= date)
            .unwrap_or(0)
    };
    let mut last_by_segment: Vec<Vec<&ValidDate>> = vec![Vec::new(); first_fields.len()];
    for last in of_kind(ClaimKind::LastClass) {
        last_by_segment[segment_of(last.date)].push(last);
    }
    let last_fields: Vec<Option<Field>> =
        last_by_segment.into_iter().map(Field::from_dates).collect();

    let exam_field = Field::from_dates(of_kind(ClaimKind::ExamPeriod));
    let final_field = Field::from_dates(of_kind(ClaimKind::FinalExam));

    // Breaks: overlapping spans are one field (V9: identical ones merge, different ones are
    // alternatives, the later-published default); over-long ones are dropped (V7, by itself).
    let mut break_claims = of_kind(ClaimKind::Break);
    break_claims.sort_by_key(|d| d.date);
    let mut break_groups: Vec<(NaiveDate, NaiveDate, Vec<&ValidDate>)> = Vec::new();
    for claim in break_claims {
        let span = (claim.date, claim.end.unwrap_or(claim.date));
        let kind = break_kind(&claim.label, span.0, span.1);
        let max = if kind == BreakKind::WinterBreak {
            MAX_WINTER_BREAK_DAYS
        } else {
            MAX_BREAK_DAYS
        };
        if days_between(span.0, span.1) + 1 > max {
            *dropped.entry(DropReason::Inconsistent).or_default() += 1;
            continue;
        }
        match break_groups
            .iter_mut()
            .find(|(start, end, _)| span.0 <= *end && *start <= span.1)
        {
            Some((start, end, group)) => {
                *start = (*start).min(span.0);
                *end = (*end).max(span.1);
                group.push(claim);
            }
            None => break_groups.push((span.0, span.1, vec![claim])),
        }
    }
    let break_fields: Vec<Field> = break_groups
        .into_iter()
        .filter_map(|(_, _, group)| Field::from_dates(group))
        .collect();

    // V7, by itself: an over-long exam period is dropped.
    let exam_field = exam_field.filter(|exam| {
        let long = exam
            .end
            .is_some_and(|end| days_between(exam.date, end) + 1 > MAX_EXAM_DAYS);
        if long {
            *dropped.entry(DropReason::Inconsistent).or_default() += 1;
        }
        !long
    });

    // The week table: one restart allowed (segment 2), dates at least 7 days apart.
    let (weeks, table_ok) = check_weeks(weeks);
    if !table_ok {
        *dropped.entry(DropReason::TableDropped).or_default() +=
            u32::try_from(validated.weeks.len()).unwrap_or(u32::MAX);
    }

    // Segments with their first week numbers.
    let mut segments: Vec<TeachingSegment> = Vec::new();
    for (index, first) in first_fields.iter().enumerate() {
        let last_class = last_fields
            .get(index)
            .and_then(|f| f.as_ref())
            .map(|f| f.date);
        let first_week_number = weeks
            .iter()
            .filter_map(|w| Some((w, w.starts_on?)))
            .find(|(_, starts)| week_one_monday(first.date) == crate::dates::monday_of(*starts))
            .map_or(1, |(w, _)| w.week);
        segments.push(TeachingSegment {
            first_class: first.date,
            last_class,
            first_week_number,
        });
    }
    // A second segment without its own row: continue the numbering.
    if segments.len() == 2
        && !weeks.iter().any(|w| {
            w.starts_on.is_some_and(|s| {
                crate::dates::monday_of(s) == week_one_monday(segments[1].first_class)
            })
        })
    {
        let first_half = TermResolution {
            teaching: vec![segments[0].clone()],
            ..TermResolution::default()
        };
        let last_week = segments[0]
            .last_class
            .and_then(|last| teaching_week_on(&first_half, last))
            .unwrap_or(0);
        segments[1].first_week_number = last_week + 1;
    }

    // Breaks, `numbered` from the table.
    let breaks: Vec<CalendarBreak> = break_fields
        .iter()
        .map(|field| {
            let span = DateSpan {
                start: field.date,
                end: field.end.unwrap_or(field.date),
            };
            CalendarBreak {
                kind: break_kind(&field.label, span.start, span.end),
                span,
                numbered: weeks
                    .iter()
                    .any(|w| w.starts_on.is_some_and(|s| span.contains(s))),
                label: field.label.clone(),
            }
        })
        .collect();

    // V7 between claims: conflicts.
    for (index, (first, last)) in first_fields.iter().zip(&last_fields).enumerate() {
        let segment_no = u32::try_from(index).unwrap_or(0);
        if let Some(last) = last {
            let weeks_long = (days_between(first.date, last.date) + 1 + 6) / 7;
            if last.date <= first.date
                || !(MIN_SEGMENT_WEEKS..=MAX_SEGMENT_WEEKS).contains(&weeks_long)
            {
                conflicts.push(CalendarConflict {
                    code: ConflictCode::Inconsistent,
                    kind: DateKind::LastClass,
                    segment: segment_no,
                    options: vec![first.as_option(), last.as_option()],
                });
            }
        }
    }
    if let (Some(first), Some(last)) =
        (segments.first(), segments.last().and_then(|s| s.last_class))
        && (days_between(first.first_class, last) + 1 + 6) / 7 > MAX_COURSE_WEEKS
    {
        conflicts.push(CalendarConflict {
            code: ConflictCode::Inconsistent,
            kind: DateKind::LastClass,
            segment: u32::try_from(segments.len() - 1).unwrap_or(0),
            options: Vec::new(),
        });
    }
    let course_last = last_fields.iter().rev().flatten().next();
    if let (Some(exam), Some(last)) = (&exam_field, course_last) {
        let start_after_last = days_between(last.date, exam.date);
        let end = exam.end.unwrap_or(exam.date);
        if !EXAMS_FROM_LAST_CLASS.contains(&start_after_last) || end < last.date {
            conflicts.push(CalendarConflict {
                code: ConflictCode::Inconsistent,
                kind: DateKind::ExamPeriod,
                segment: u32::try_from(last_fields.len().saturating_sub(1)).unwrap_or(0),
                options: vec![last.as_option(), exam.as_option()],
            });
        }
    }
    if let (Some(final_exam), Some(exam)) = (&final_field, &exam_field) {
        let span = DateSpan {
            start: exam.date,
            end: exam.end.unwrap_or(exam.date),
        };
        if !span.contains(final_exam.date) {
            conflicts.push(CalendarConflict {
                code: ConflictCode::Inconsistent,
                kind: DateKind::FinalExam,
                segment: 0,
                options: vec![exam.as_option(), final_exam.as_option()],
            });
        }
    }
    let course_span = DateSpan {
        start: segments[0].first_class,
        end: exam_field
            .as_ref()
            .and_then(|e| e.end)
            .or(course_last.map(|l| l.date))
            .unwrap_or(NaiveDate::MAX),
    };
    let mut long_breaks = 0;
    for (field, calendar_break) in break_fields.iter().zip(&breaks) {
        if !(course_span.contains(calendar_break.span.start)
            && course_span.contains(calendar_break.span.end))
        {
            conflicts.push(CalendarConflict {
                code: ConflictCode::Inconsistent,
                kind: DateKind::BreakSpan,
                segment: 0,
                options: vec![field.as_option()],
            });
        }
        if weekdays(calendar_break.span) >= 3 {
            long_breaks += 1;
        }
    }
    if long_breaks > MAX_LONG_BREAKS {
        conflicts.push(CalendarConflict {
            code: ConflictCode::Inconsistent,
            kind: DateKind::BreakSpan,
            segment: 0,
            options: Vec::new(),
        });
    }

    let calendar = CourseCalendar {
        segments,
        breaks,
        exam_period: exam_field.as_ref().map(|e| DateSpan {
            start: e.date,
            end: e.end.unwrap_or(e.date),
        }),
        final_exam_on: final_field.as_ref().map(|f| f.date),
        weeks: weeks
            .iter()
            .filter_map(|w| {
                Some(CalendarWeek {
                    number: w.week,
                    starts_on: w.starts_on?,
                    topic: w.topic.clone(),
                })
            })
            .collect(),
    };

    // V8 cross-checks.
    let term = resolution_of(&calendar);
    let week_one = term.week_one_monday.unwrap_or(input.today);
    let mut disagrees = false;
    let first_option = || vec![first_fields[0].as_option()];
    if let Some((fit_monday, fit_weeks)) = input.checks.fit
        && fit_weeks >= MIN_FIT_WEEKS
        && days_between(week_one, fit_monday).abs() >= FIT_DISAGREE_DAYS
    {
        disagrees = true;
        conflicts.push(CalendarConflict {
            code: ConflictCode::DisagreesWithNotes,
            kind: DateKind::FirstClass,
            segment: 0,
            options: [first_option(), vec![own_option(fit_monday)]].concat(),
        });
    }
    if let Some(start) = input.checks.lms_course_start {
        let offset = days_between(start, calendar.segments[0].first_class);
        if !(-LMS_BEFORE_DAYS..=LMS_AFTER_DAYS).contains(&offset) {
            conflicts.push(CalendarConflict {
                code: ConflictCode::DisagreesWithLmsDates,
                kind: DateKind::FirstClass,
                segment: 0,
                options: [first_option(), vec![own_option(start)]].concat(),
            });
        }
    }
    if let Some(event) = input.checks.first_class_event
        && days_between(event, calendar.segments[0].first_class).abs() > CLASS_EVENT_DAYS
    {
        conflicts.push(CalendarConflict {
            code: ConflictCode::DisagreesWithClassEvent,
            kind: DateKind::FirstClass,
            segment: 0,
            options: [first_option(), vec![own_option(event)]].concat(),
        });
    }
    let samples: Vec<bool> = input
        .checks
        .observations
        .iter()
        .filter_map(|(day, week)| {
            let expected = teaching_week_on(&term, *day)?;
            Some(*week == expected || *week == expected + 1)
        })
        .collect();
    if samples.len() >= MIN_NOTE_SAMPLES
        && samples.iter().filter(|agrees| **agrees).count() * 2 < samples.len()
        && !conflicts
            .iter()
            .any(|c| c.code == ConflictCode::DisagreesWithNotes)
    {
        disagrees = true;
        conflicts.push(CalendarConflict {
            code: ConflictCode::DisagreesWithNotes,
            kind: DateKind::WeekStart,
            segment: 0,
            options: Vec::new(),
        });
    }

    // A reading week the school's calendar puts elsewhere: shown with both options.
    let proposed_reading: Vec<&CalendarBreak> = calendar
        .breaks
        .iter()
        .filter(|b| b.kind == BreakKind::ReadingWeek)
        .collect();
    let course_end = calendar
        .exam_period
        .map(|period| period.end)
        .or_else(|| calendar.segments.last().and_then(|s| s.last_class));
    for school in &input.checks.school_reading_weeks {
        let inside = school.start >= calendar.segments[0].first_class
            && course_end.is_none_or(|end| school.end <= end);
        let matched = proposed_reading
            .iter()
            .any(|b| b.span.start <= school.end && school.start <= b.span.end);
        let Some(nearest) = proposed_reading
            .iter()
            .min_by_key(|b| days_between(b.span.start, school.start).abs())
        else {
            break; // the proposal names no reading week: nothing to compare
        };
        if !inside || matched {
            continue;
        }
        disagrees = true;
        let segment = calendar
            .segments
            .iter()
            .rposition(|s| s.first_class <= school.start)
            .unwrap_or(0);
        conflicts.push(CalendarConflict {
            code: ConflictCode::DiffersFromInstitutionCalendar,
            kind: DateKind::BreakSpan,
            segment: u32::try_from(segment).unwrap_or(0),
            options: vec![
                AlternativeDate {
                    date: nearest.span.start,
                    end: Some(nearest.span.end),
                    label: nearest.label.clone(),
                    evidence: Vec::new(),
                },
                AlternativeDate {
                    end: Some(school.end),
                    ..own_option(school.start)
                },
            ],
        });
    }

    // The proposal's dates, as the student reviews them.
    for (index, field) in first_fields.iter().enumerate() {
        dates_out.push(field.proposed(DateKind::FirstClass, u32::try_from(index).unwrap_or(0)));
    }
    for (index, field) in last_fields.iter().enumerate() {
        if let Some(field) = field {
            dates_out.push(field.proposed(DateKind::LastClass, u32::try_from(index).unwrap_or(0)));
        }
    }
    for (field, calendar_break) in break_fields.iter().zip(&calendar.breaks) {
        let mut date = field.proposed(DateKind::BreakSpan, 0);
        date.break_kind = Some(calendar_break.kind);
        date.numbered = Some(calendar_break.numbered);
        dates_out.push(date);
    }
    if let Some(exam) = &exam_field {
        dates_out.push(exam.proposed(DateKind::ExamPeriod, 0));
    }
    if let Some(final_exam) = &final_field {
        dates_out.push(final_exam.proposed(DateKind::FinalExam, 0));
    }
    for row in &weeks {
        let Some(starts_on) = row.starts_on else {
            continue;
        };
        dates_out.push(ProposedDate {
            kind: DateKind::WeekStart,
            segment: 0,
            date: starts_on,
            end: None,
            label: String::new(),
            evidence: vec![row.evidence.clone()],
            alternatives: Vec::new(),
            week: Some(row.week),
            break_kind: None,
            numbered: None,
        });
    }

    // Today, if accepted.
    let ProposalOutcome {
        resulting_week_today,
        resulting_phase,
        changes,
    } = outcome_of(
        &calendar,
        &CurrentCalendar {
            calendar: input.current,
            week: input.current_week,
            phase: input.current_phase,
        },
        input.today,
        input.full_year,
    );

    let dropped: Vec<DropCount> = dropped
        .into_iter()
        .map(|(reason, count)| DropCount { reason, count })
        .collect();
    let lost: u32 = dropped.iter().map(|d| d.count).sum();
    let total = u32::try_from(validated.dates.len()).unwrap_or(u32::MAX) + lost;
    let low_quality = validated.table_dropped
        || !table_ok
        || validated.stated_term_another_year
        || (total > 0 && lost * 3 > total);
    conflicts.sort_by_key(|c| (c.code, c.kind, c.segment));
    conflicts.dedup_by(|a, b| {
        a.code == b.code && a.kind == b.kind && a.segment == b.segment && a.options == b.options
    });
    Ok(Assembled {
        passing: conflicts.is_empty() && !low_quality,
        calendar,
        dates: dates_out,
        conflicts,
        dropped,
        low_quality,
        disagrees_with_notes: disagrees,
        resulting_week_today,
        resulting_phase,
        changes,
    })
}

/// The table's rows in date order, or none when the dated rows break V7 (week numbers
/// increase with one restart allowed for a full-year course; dates at least 7 days apart).
/// Rows without a date follow, by week.
fn check_weeks(rows: &[ValidWeek]) -> (Vec<ValidWeek>, bool) {
    let mut dated: Vec<ValidWeek> = rows
        .iter()
        .filter(|r| r.starts_on.is_some())
        .cloned()
        .collect();
    dated.sort_by_key(|row| (row.starts_on, row.week));
    let mut restarts = 0;
    for pair in dated.windows(2) {
        if pair[1].week <= pair[0].week {
            restarts += 1;
        }
        if let (Some(a), Some(b)) = (pair[0].starts_on, pair[1].starts_on)
            && days_between(a, b) < 7
        {
            return (Vec::new(), false);
        }
    }
    if restarts > 1 {
        return (Vec::new(), false);
    }
    let mut undated: Vec<ValidWeek> = rows
        .iter()
        .filter(|r| r.starts_on.is_none())
        .cloned()
        .collect();
    undated.sort_by_key(|row| row.week);
    dated.extend(undated);
    (dated, true)
}

/// What kind of break a label names (material text in any of the supported languages).
fn break_kind(label: &str, start: NaiveDate, end: NaiveDate) -> BreakKind {
    let label = label.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| label.contains(w));
    if has(&["winter break", "holiday break", "寒假"]) {
        BreakKind::WinterBreak
    } else if has(&[
        "reading week",
        "study break",
        "fall break",
        "reading break",
        "spring break",
        "阅读周",
        "閱讀週",
    ]) {
        BreakKind::ReadingWeek
    } else if weekdays(DateSpan { start, end }) <= 2 {
        BreakKind::Holiday
    } else {
        BreakKind::Other
    }
}

fn weekdays(span: DateSpan) -> i64 {
    let days = days_between(span.start, span.end) + 1;
    (0..days.clamp(0, 400))
        .map(|i| add_days(span.start, i))
        .filter(|d| !matches!(d.weekday(), Weekday::Sat | Weekday::Sun))
        .count() as i64
}

fn date_kind(kind: ClaimKind) -> DateKind {
    match kind {
        ClaimKind::FirstClass | ClaimKind::TermStart => DateKind::FirstClass,
        ClaimKind::LastClass | ClaimKind::TermEnd => DateKind::LastClass,
        ClaimKind::Break => DateKind::BreakSpan,
        ClaimKind::ExamPeriod => DateKind::ExamPeriod,
        ClaimKind::FinalExam => DateKind::FinalExam,
    }
}

/// An option from PageLamp's own evidence (no quote).
fn own_option(date: NaiveDate) -> AlternativeDate {
    AlternativeDate {
        date,
        end: None,
        label: String::new(),
        evidence: Vec::new(),
    }
}

/// The resolution a calendar gives (for phases and weeks).
pub fn resolution_of(calendar: &CourseCalendar) -> TermResolution {
    TermResolution {
        week_one_monday: calendar
            .segments
            .first()
            .map(|s| week_one_monday(s.first_class)),
        teaching: calendar.segments.clone(),
        breaks: calendar.breaks.clone(),
        exams_end: calendar.exam_period.map(|p| p.end),
        anchor: crate::term::TermAnchorSource::StudentConfirmed,
        anchor_confidence: Confidence::High,
        ..TermResolution::default()
    }
}

/// The calendar in force and what the course shows today under it.
#[derive(Clone, Copy, Debug)]
pub struct CurrentCalendar<'a> {
    pub calendar: Option<&'a CourseCalendar>,
    pub week: Option<u32>,
    pub phase: CoursePhase,
}

/// What accepting a proposal would mean today.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalOutcome {
    pub resulting_week_today: Option<u32>,
    pub resulting_phase: CoursePhase,
    pub changes: Vec<CalendarChange>,
}

/// §7.6: today's week and phase under `calendar`, and what changes compared with `current`.
/// Recomputed whenever a proposal is shown, so it follows a calendar accepted meanwhile.
pub fn outcome_of(
    calendar: &CourseCalendar,
    current: &CurrentCalendar<'_>,
    today: NaiveDate,
    full_year: bool,
) -> ProposalOutcome {
    let state = phase_on(
        &resolution_of(calendar),
        Confidence::High,
        today,
        full_year || calendar.segments.len() == 2,
        false,
        None,
    );
    ProposalOutcome {
        resulting_week_today: state.week,
        resulting_phase: state.phase,
        changes: changes(current, calendar, state.week, state.phase),
    }
}

fn changes(
    input: &CurrentCalendar<'_>,
    calendar: &CourseCalendar,
    week: Option<u32>,
    phase: CoursePhase,
) -> Vec<CalendarChange> {
    let mut out = Vec::new();
    match input.calendar {
        None => out.push(CalendarChange::new(ChangeCode::NewCalendar)),
        Some(current) => {
            let first = |c: &CourseCalendar| c.segments.first().map(|s| s.first_class);
            let last = |c: &CourseCalendar| c.segments.last().and_then(|s| s.last_class);
            if first(current) != first(calendar)
                && let Some(to) = first(calendar)
            {
                out.push(
                    CalendarChange::new(ChangeCode::FirstClassMoved)
                        .opt("from", first(current))
                        .with("to", to),
                );
            }
            if last(current) != last(calendar)
                && let Some(to) = last(calendar)
            {
                out.push(
                    CalendarChange::new(ChangeCode::LastClassMoved)
                        .opt("from", last(current))
                        .with("to", to),
                );
            }
            let exams = |c: &CourseCalendar| c.exam_period.map(|p| p.end);
            if exams(current) != exams(calendar)
                && let Some(to) = exams(calendar)
            {
                out.push(
                    CalendarChange::new(ChangeCode::ExamsEndMoved)
                        .opt("from", exams(current))
                        .with("to", to),
                );
            }
            for added in calendar
                .breaks
                .iter()
                .filter(|b| !current.breaks.iter().any(|c| c.span == b.span))
            {
                out.push(
                    CalendarChange::new(ChangeCode::BreakAdded)
                        .with("kind", added.kind.as_str())
                        .with("start", added.span.start)
                        .with("end", added.span.end),
                );
            }
            for removed in current
                .breaks
                .iter()
                .filter(|b| !calendar.breaks.iter().any(|c| c.span == b.span))
            {
                out.push(
                    CalendarChange::new(ChangeCode::BreakRemoved)
                        .with("kind", removed.kind.as_str())
                        .with("start", removed.span.start)
                        .with("end", removed.span.end),
                );
            }
        }
    }
    if week != input.week {
        out.push(
            CalendarChange::new(ChangeCode::WeekTodayChanges)
                .opt("from", input.week)
                .opt("to", week),
        );
    }
    if phase != input.phase {
        out.push(
            CalendarChange::new(ChangeCode::PhaseChanges)
                .with("from_phase", input.phase.as_str())
                .with("to_phase", phase.as_str()),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::calendar::extraction::WeekKind;
    use crate::model::Timestamp;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn at(y: i32, m: u32, d: u32) -> Option<Timestamp> {
        Some(date(y, m, d).and_hms_opt(12, 0, 0).unwrap().and_utc())
    }

    fn evidence(material: &str) -> DateEvidence {
        DateEvidence {
            material_id: material.into(),
            title: material.into(),
            locator: None,
            quote: Some("a quote from the material".into()),
            url: None,
            derived: false,
        }
    }

    fn claim(kind: ClaimKind, start: NaiveDate, end: Option<NaiveDate>, label: &str) -> ValidDate {
        ValidDate {
            kind,
            date: start,
            end,
            label: label.into(),
            evidence: evidence("outline"),
            published_at: at(2026, 8, 20),
        }
    }

    /// CAL-20's fall course as claims.
    fn fall() -> Vec<ValidDate> {
        vec![
            claim(
                ClaimKind::FirstClass,
                date(2026, 9, 8),
                None,
                "Classes begin",
            ),
            claim(
                ClaimKind::Break,
                date(2026, 10, 26),
                Some(date(2026, 10, 30)),
                "Reading week",
            ),
            claim(
                ClaimKind::LastClass,
                date(2026, 12, 8),
                None,
                "Last day of classes",
            ),
            claim(
                ClaimKind::ExamPeriod,
                date(2026, 12, 10),
                Some(date(2026, 12, 22)),
                "Final exam period",
            ),
        ]
    }

    fn validated(dates: Vec<ValidDate>, weeks: Vec<ValidWeek>) -> Validated {
        Validated {
            dates,
            weeks,
            ..Validated::default()
        }
    }

    fn input(validated: &Validated, today: NaiveDate) -> AssembleInput<'_> {
        AssembleInput {
            validated,
            checks: CrossChecks::default(),
            current: None,
            current_week: None,
            current_phase: CoursePhase::Unknown,
            today,
            full_year: false,
        }
    }

    #[test]
    fn a_fall_outline_becomes_a_passing_calendar() {
        let v = validated(fall(), Vec::new());
        let a = assemble(&input(&v, date(2026, 10, 28))).unwrap();
        assert!(a.passing, "{:?}", a.conflicts);
        assert_eq!(
            a.calendar.segments,
            [TeachingSegment {
                first_class: date(2026, 9, 8),
                last_class: Some(date(2026, 12, 8)),
                first_week_number: 1,
            }]
        );
        assert_eq!(a.calendar.breaks.len(), 1);
        assert_eq!(a.calendar.breaks[0].kind, BreakKind::ReadingWeek);
        assert!(!a.calendar.breaks[0].numbered);
        assert_eq!(
            a.calendar.exam_period,
            Some(DateSpan {
                start: date(2026, 12, 10),
                end: date(2026, 12, 22)
            })
        );
        // Today (10-28) would be the reading week.
        assert_eq!(a.resulting_phase, CoursePhase::Break);
        assert_eq!(a.resulting_week_today, None);
        let codes: Vec<&str> = a.changes.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(codes, ["new_calendar", "phase_changes"]);
        let kinds: Vec<DateKind> = a.dates.iter().map(|d| d.kind).collect();
        assert_eq!(
            kinds,
            [
                DateKind::FirstClass,
                DateKind::LastClass,
                DateKind::BreakSpan,
                DateKind::ExamPeriod
            ]
        );
        assert_eq!(a.dates[2].break_kind, Some(BreakKind::ReadingWeek));
        assert_eq!(a.dates[2].numbered, Some(false));
        let week = assemble(&input(&v, date(2026, 11, 2))).unwrap();
        assert_eq!(week.resulting_week_today, Some(8));
    }

    /// CAL-38: "Last day of classes: Oct 15" against "Final exam period: Dec 10–22".
    #[test]
    fn multi_claim_violation_becomes_conflict() {
        let mut dates = fall();
        dates[2].date = date(2026, 10, 15);
        let v = validated(dates, Vec::new());
        let a = assemble(&input(&v, date(2026, 9, 28))).unwrap();
        assert!(!a.passing);
        let conflict = a
            .conflicts
            .iter()
            .find(|c| c.kind == DateKind::ExamPeriod)
            .expect("exam period conflict");
        assert_eq!(conflict.code, ConflictCode::Inconsistent);
        let options: Vec<NaiveDate> = conflict.options.iter().map(|o| o.date).collect();
        assert_eq!(options, [date(2026, 10, 15), date(2026, 12, 10)]);
        // Both dates are still in the proposal for the student to choose.
        assert!(a.dates.iter().any(|d| d.date == date(2026, 10, 15)));
        assert!(a.dates.iter().any(|d| d.date == date(2026, 12, 10)));
        // An injected "Final exam: October 1" outside the exam period is a conflict too.
        let mut dates = fall();
        dates.push(claim(
            ClaimKind::FinalExam,
            date(2026, 10, 1),
            None,
            "Final exam",
        ));
        let v = validated(dates, Vec::new());
        let a = assemble(&input(&v, date(2026, 9, 28))).unwrap();
        assert!(a.conflicts.iter().any(|c| c.kind == DateKind::FinalExam));
        assert!(!a.passing);
    }

    /// V9: the syllabus and a later schedule page disagree about the reading week.
    #[test]
    fn different_materials_give_alternatives_with_the_later_one_first() {
        let mut dates = fall();
        let mut schedule = claim(
            ClaimKind::Break,
            date(2026, 10, 27),
            Some(date(2026, 10, 31)),
            "Reading week",
        );
        schedule.evidence = evidence("schedule");
        schedule.published_at = at(2026, 9, 10);
        dates.push(schedule);
        // The same date from a second material only adds evidence.
        let mut copy = fall()[0].clone();
        copy.evidence = evidence("announcement");
        dates.push(copy);
        let v = validated(dates, Vec::new());
        let a = assemble(&input(&v, date(2026, 9, 28))).unwrap();
        let reading = a
            .dates
            .iter()
            .find(|d| d.kind == DateKind::BreakSpan)
            .unwrap();
        assert_eq!(
            reading.date,
            date(2026, 10, 27),
            "the later-published schedule wins"
        );
        assert_eq!(reading.alternatives.len(), 1);
        assert_eq!(reading.alternatives[0].date, date(2026, 10, 26));
        let first = a
            .dates
            .iter()
            .find(|d| d.kind == DateKind::FirstClass)
            .unwrap();
        assert_eq!(first.evidence.len(), 2);
        assert!(first.alternatives.is_empty());
    }

    fn row(week: u32, starts: NaiveDate, kind: WeekKind) -> ValidWeek {
        ValidWeek {
            week,
            starts_on: Some(starts),
            kind,
            topic: None,
            evidence: evidence("schedule"),
        }
    }

    /// CAL-22 (the assembly part): two first classes make a full-year course; the table
    /// restarts or continues the numbering.
    #[test]
    fn two_first_classes_make_a_full_year_course() {
        let dates = vec![
            claim(
                ClaimKind::FirstClass,
                date(2026, 9, 8),
                None,
                "Fall classes begin",
            ),
            claim(
                ClaimKind::LastClass,
                date(2026, 12, 8),
                None,
                "Fall classes end",
            ),
            claim(
                ClaimKind::FirstClass,
                date(2027, 1, 11),
                None,
                "Winter classes begin",
            ),
            claim(
                ClaimKind::LastClass,
                date(2027, 4, 9),
                None,
                "Winter classes end",
            ),
        ];
        // No table: the numbering continues (the first half has 14 weeks).
        let v = validated(dates.clone(), Vec::new());
        let a = assemble(&input(&v, date(2027, 1, 20))).unwrap();
        assert_eq!(a.calendar.segments.len(), 2);
        assert_eq!(a.calendar.segments[1].first_week_number, 15);
        assert_eq!(a.calendar.segments[0].last_class, Some(date(2026, 12, 8)));
        assert_eq!(a.calendar.segments[1].last_class, Some(date(2027, 4, 9)));
        assert_eq!(a.resulting_week_today, Some(16));
        assert!(a.passing, "{:?}", a.conflicts);
        // A table that restarts at week 1 in January.
        let weeks = vec![
            row(1, date(2026, 9, 8), WeekKind::Teaching),
            row(2, date(2026, 9, 15), WeekKind::Teaching),
            row(1, date(2027, 1, 11), WeekKind::Teaching),
            row(2, date(2027, 1, 18), WeekKind::Teaching),
        ];
        let v = validated(dates, weeks);
        let a = assemble(&input(&v, date(2027, 1, 20))).unwrap();
        assert_eq!(a.calendar.segments[1].first_week_number, 1);
        assert_eq!(a.resulting_week_today, Some(2));
        assert_eq!(a.calendar.weeks.len(), 4);
    }

    #[test]
    fn the_table_decides_week_zero_and_numbered_breaks() {
        let weeks = vec![
            row(0, date(2026, 9, 8), WeekKind::Teaching),
            row(1, date(2026, 9, 15), WeekKind::Teaching),
            row(7, date(2026, 10, 26), WeekKind::Break),
            row(8, date(2026, 11, 2), WeekKind::Teaching),
        ];
        let v = validated(fall(), weeks);
        let a = assemble(&input(&v, date(2026, 9, 28))).unwrap();
        assert_eq!(a.calendar.segments[0].first_week_number, 0);
        assert!(a.calendar.breaks[0].numbered);
        assert_eq!(a.resulting_week_today, Some(3));
        assert!(
            a.dates
                .iter()
                .any(|d| d.kind == DateKind::WeekStart && d.week == Some(7))
        );
        // Two restarts: the table goes.
        let bad = vec![
            row(1, date(2026, 9, 8), WeekKind::Teaching),
            row(1, date(2026, 9, 15), WeekKind::Teaching),
            row(1, date(2026, 9, 22), WeekKind::Teaching),
        ];
        let v = validated(fall(), bad);
        let a = assemble(&input(&v, date(2026, 9, 28))).unwrap();
        assert!(a.calendar.weeks.is_empty());
        assert!(a.low_quality && !a.passing);
    }

    #[test]
    fn a_break_that_is_too_long_by_itself_is_dropped() {
        let mut dates = fall();
        dates.push(claim(
            ClaimKind::Break,
            date(2026, 11, 2),
            Some(date(2026, 11, 30)),
            "Project weeks",
        ));
        let v = validated(dates, Vec::new());
        let a = assemble(&input(&v, date(2026, 9, 28))).unwrap();
        assert_eq!(a.calendar.breaks.len(), 1);
        assert!(
            a.dropped
                .iter()
                .any(|d| d.reason == DropReason::Inconsistent && d.count == 1)
        );
    }

    /// V8: PageLamp's own evidence disagrees.
    /// V8: a reading week the school's calendar puts elsewhere is a conflict with both options
    /// (the school's without a label), and the calendar is at most Medium once accepted; the
    /// same week agrees.
    #[test]
    fn a_reading_week_the_school_puts_elsewhere_is_a_conflict() {
        let v = validated(fall(), Vec::new());
        let span = |start, end| DateSpan {
            start: date(2026, 10, start),
            end: date(2026, 10, end),
        };
        let mut i = input(&v, date(2026, 10, 5));
        i.checks.school_reading_weeks = vec![span(26, 30)];
        let a = assemble(&i).unwrap();
        assert!(
            !a.conflicts
                .iter()
                .any(|c| c.code == ConflictCode::DiffersFromInstitutionCalendar)
        );
        i.checks.school_reading_weeks = vec![span(19, 23)];
        let a = assemble(&i).unwrap();
        let conflict = a
            .conflicts
            .iter()
            .find(|c| c.code == ConflictCode::DiffersFromInstitutionCalendar)
            .expect("a conflict");
        assert_eq!(conflict.kind, DateKind::BreakSpan);
        let options: Vec<(NaiveDate, Option<NaiveDate>)> =
            conflict.options.iter().map(|o| (o.date, o.end)).collect();
        assert_eq!(
            options,
            [
                (date(2026, 10, 26), Some(date(2026, 10, 30))),
                (date(2026, 10, 19), Some(date(2026, 10, 23)))
            ]
        );
        assert!(
            conflict.options[1].label.is_empty(),
            "no label from the school"
        );
        assert!(a.disagrees_with_notes && !a.passing);
    }

    #[test]
    fn cross_checks_become_conflicts_that_are_shown() {
        let v = validated(fall(), Vec::new());
        let mut i = input(&v, date(2026, 10, 5));
        i.checks = CrossChecks {
            fit: Some((date(2026, 9, 14), 3)),
            // 29 days before the first class: outside −7/+21.
            lms_course_start: Some(date(2026, 8, 10)),
            first_class_event: Some(date(2026, 9, 15)),
            observations: Vec::new(),
            school_reading_weeks: Vec::new(),
        };
        let a = assemble(&i).unwrap();
        let codes: Vec<ConflictCode> = a.conflicts.iter().map(|c| c.code).collect();
        assert_eq!(
            codes,
            [
                ConflictCode::DisagreesWithNotes,
                ConflictCode::DisagreesWithLmsDates,
                ConflictCode::DisagreesWithClassEvent
            ]
        );
        assert!(a.disagrees_with_notes);
        assert!(!a.passing);
        assert_eq!(
            a.calendar.segments[0].first_class,
            date(2026, 9, 8),
            "never dropped"
        );
        // Materials a week ahead agree; most materials two weeks off disagree.
        let mut i = input(&v, date(2026, 10, 5));
        i.checks.observations = vec![
            (date(2026, 9, 21), 3),
            (date(2026, 9, 28), 5),
            (date(2026, 10, 5), 5),
        ];
        assert!(assemble(&i).unwrap().passing);
        i.checks.observations = vec![
            (date(2026, 9, 21), 6),
            (date(2026, 9, 28), 7),
            (date(2026, 10, 5), 5),
        ];
        let a = assemble(&i).unwrap();
        assert!(a.disagrees_with_notes && !a.passing);
    }

    #[test]
    fn another_year_and_bad_output() {
        let mut v = validated(fall(), Vec::new());
        v.stated_term_another_year = true;
        let a = assemble(&input(&v, date(2026, 9, 28))).unwrap();
        assert!(
            a.conflicts
                .iter()
                .any(|c| c.code == ConflictCode::SyllabusFromAnotherYear)
        );
        assert!(!a.passing);
        v.bad_output = true;
        assert_eq!(assemble(&input(&v, date(2026, 9, 28))), Err(BadOutput));
    }

    #[test]
    fn changes_against_the_calendar_in_force() {
        let current = assemble(&input(&validated(fall(), Vec::new()), date(2026, 9, 28)))
            .unwrap()
            .calendar;
        let mut dates = fall();
        dates[2].date = date(2026, 12, 4);
        dates.push(claim(
            ClaimKind::Break,
            date(2026, 10, 12),
            Some(date(2026, 10, 12)),
            "Thanksgiving",
        ));
        let v = validated(dates, Vec::new());
        let mut i = input(&v, date(2026, 9, 28));
        i.current = Some(&current);
        i.current_week = Some(4);
        i.current_phase = CoursePhase::Teaching;
        let a = assemble(&i).unwrap();
        let codes: Vec<&str> = a.changes.iter().map(|c| c.code.as_str()).collect();
        assert_eq!(codes, ["last_class_moved", "break_added"]);
        let holiday = a
            .calendar
            .breaks
            .iter()
            .find(|b| b.span.start == date(2026, 10, 12))
            .unwrap();
        assert_eq!(holiday.kind, BreakKind::Holiday);
    }
}
