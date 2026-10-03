//! The course dates form, version 2 (docs/design/v0.3-course-calendar.md §7.10): first and
//! last day of classes, end of exams, breaks and a second segment for full-year courses.
//!
//! The input types come first (the alpha.2 contract agreed with the desktop);
//! `set_course_dates` arrives with the schema-v4 calendars (B6). The facade computes the week
//! numbering (a second segment continues after the first or restarts at 1), so surfaces do no
//! week arithmetic.

use chrono::NaiveDate;
use pagelamp_core::model::BreakKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CourseDatesInput {
    pub first_class: Option<NaiveDate>,
    pub last_class: Option<NaiveDate>,
    pub exams_end: Option<NaiveDate>,
    pub breaks: Vec<BreakInput>,
    /// The second half of a full-year course.
    pub second_segment: Option<SegmentInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BreakInput {
    pub kind: BreakKind,
    pub start: NaiveDate,
    pub end: NaiveDate,
    /// The break counts in the week numbering.
    pub numbered: bool,
    /// Shown to the student only (never over MCP); at most 80 characters.
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SegmentInput {
    pub first_class: NaiveDate,
    pub last_class: Option<NaiveDate>,
    /// True: this segment's first week is week 1; false: numbering continues.
    pub restart_numbering: bool,
}

impl CourseDatesInput {
    /// The calendar these dates make (`calendar_from_dates` computes the week numbering);
    /// `Invalid` without a first day of classes, or listing every problem at once.
    pub(crate) fn to_calendar(&self) -> crate::Result<pagelamp_core::calendar::CourseCalendar> {
        use pagelamp_core::calendar::{DatesDraft, SecondSegment, calendar_from_dates};
        use pagelamp_core::model::{CalendarBreak, DateSpan};

        let first_class = self.first_class.ok_or_else(|| {
            crate::AppError::new(
                crate::AppErrorKind::Invalid,
                "Set the first day of classes.",
            )
        })?;
        let draft = DatesDraft {
            first_class,
            last_class: self.last_class,
            exams_end: self.exams_end,
            breaks: self
                .breaks
                .iter()
                .map(|b| CalendarBreak {
                    kind: b.kind,
                    span: DateSpan {
                        start: b.start,
                        end: b.end,
                    },
                    numbered: b.numbered,
                    label: b
                        .label
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .chars()
                        .take(pagelamp_core::calendar::validate::MAX_LABEL_CHARS)
                        .collect(),
                })
                .collect(),
            second_segment: self.second_segment.as_ref().map(|s| SecondSegment {
                first_class: s.first_class,
                last_class: s.last_class,
                restart_numbering: s.restart_numbering,
            }),
        };
        calendar_from_dates(&draft).map_err(|problems| {
            let text: Vec<&str> = problems.iter().map(|p| p.message()).collect();
            crate::AppError::new(crate::AppErrorKind::Invalid, text.join("; "))
        })
    }
}
