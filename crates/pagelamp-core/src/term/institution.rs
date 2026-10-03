//! The school's published calendar (docs/design/v0.3-course-calendar.md §6.4, anchor 5; owner
//! action A15): the first and last day of classes, the reading weeks and the exam period of each
//! session and campus, from `data/institution/uoft.toml`, shipped with PageLamp.
//!
//! - Only dates checked against UofT's published sessional dates are shipped, never made-up
//!   ones. Until A15's dates arrive the file holds no year, and every course falls back to the
//!   session window (§6.3). Tests bring their own files, marked `fixture = true`.
//! - Used only where the session hint applies (the UofT host, or `institution = "uoft"` in a
//!   folder's `course.toml`; §6.3, D50), when the course's session and campus parse and the file
//!   has that session for that campus.
//! - A full-year (Y) course takes the Fall session and the Winter one after it: two segments,
//!   the numbering continuing (§6.6; a syllabus or the student may say otherwise).
//! - Breaks are kinds only: the file has no labels, so nothing here is material text (§7.11).

use std::sync::LazyLock;

use chrono::NaiveDate;
use serde::Deserialize;

use super::session::SessionHint;
use super::{BreakKind, CalendarBreak, DateSpan, TeachingSegment};
use crate::calendar::{DatesDraft, SecondSegment, calendar_from_dates};

/// The shipped file.
const UOFT: &str = include_str!("../../data/institution/uoft.toml");

/// The campuses a year covers: St. George, Scarborough, Mississauga.
pub const CAMPUSES: [u8; 3] = [1, 3, 5];

/// The shipped calendar, parsed once. A file that doesn't parse counts as empty (a test checks
/// the shipped one parses and is complete).
pub fn shipped() -> &'static InstitutionCalendar {
    static SHIPPED: LazyLock<InstitutionCalendar> = LazyLock::new(|| {
        InstitutionCalendar::parse(UOFT).unwrap_or_else(|err| {
            tracing::error!(target: "pagelamp::term", "the school calendar doesn't parse: {err}");
            InstitutionCalendar::default()
        })
    });
    &SHIPPED
}

/// One school's calendar file.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstitutionCalendar {
    pub format: u32,
    pub institution: String,
    /// A test fixture's dates: never in the shipped file.
    #[serde(default)]
    pub fixture: bool,
    #[serde(default)]
    pub years: Vec<AcademicYear>,
}

/// One academic year: its Fall, Winter and Summer sessions.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcademicYear {
    /// "2026-27": Fall 2026, Winter 2027 and Summer 2027.
    pub year: String,
    /// Where and when the dates were checked.
    pub source: String,
    /// Campuses (`CAMPUSES`) this year has no dates for.
    #[serde(default)]
    pub missing_campuses: Vec<u8>,
    #[serde(default)]
    pub sessions: Vec<SessionDates>,
}

/// One session at one campus.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDates {
    /// The 5-digit session code: "20269".
    pub session: String,
    pub campus: u8,
    /// Summer "F" or "S"; none: the whole session.
    #[serde(default)]
    pub section: Option<String>,
    pub first_class: NaiveDate,
    pub last_class: NaiveDate,
    #[serde(default)]
    pub breaks: Vec<BreakDates>,
    pub exams_start: Option<NaiveDate>,
    pub exams_end: Option<NaiveDate>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BreakDates {
    pub kind: BreakKind,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

/// A course's dates from the school's calendar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstitutionTerm {
    /// One segment, two for a full-year course.
    pub segments: Vec<TeachingSegment>,
    /// Kinds and dates only: no labels.
    pub breaks: Vec<CalendarBreak>,
    pub exams_end: Option<NaiveDate>,
    /// What they were built from (`starting`).
    draft: DatesDraft,
}

impl InstitutionTerm {
    /// The same term from the student's own first class (a start without an end, §6.4 row 1):
    /// the school's end, breaks and exam period, and the numbering counted from that start.
    /// `None` when that start doesn't fit the term (after its first segment ends).
    pub fn starting(&self, first_class: NaiveDate) -> Option<InstitutionTerm> {
        let mut draft = self.draft.clone();
        draft.first_class = first_class;
        draft.breaks.retain(|b| b.span.start >= first_class);
        built(draft)
    }

    pub fn first_class(&self) -> NaiveDate {
        self.segments[0].first_class
    }

    pub fn last_class(&self) -> Option<NaiveDate> {
        self.segments.last().and_then(|segment| segment.last_class)
    }
}

impl InstitutionCalendar {
    /// Parse and check a calendar file: format 1, "uoft", and every session consistent (codes,
    /// campuses, classes before exams, breaks inside the classes).
    pub fn parse(text: &str) -> Result<InstitutionCalendar, String> {
        let calendar: InstitutionCalendar =
            toml::from_str(text).map_err(|err| format!("not a calendar file: {err}"))?;
        if calendar.format != 1 {
            return Err(format!("format {} isn't known", calendar.format));
        }
        if calendar.institution != "uoft" {
            return Err(format!(
                "institution {:?} isn't known",
                calendar.institution
            ));
        }
        for year in &calendar.years {
            for dates in &year.sessions {
                check_session(&year.year, dates)
                    .map_err(|err| format!("{} session {}: {err}", year.year, dates.session))?;
            }
        }
        Ok(calendar)
    }

    /// Every year either has dates for each campus or names the ones it lacks: the problems.
    pub fn completeness_problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for year in &self.years {
            for campus in CAMPUSES {
                let listed = year.sessions.iter().any(|s| s.campus == campus);
                let missing = year.missing_campuses.contains(&campus);
                if listed == missing {
                    problems.push(format!(
                        "{}: campus {campus} {}",
                        year.year,
                        if listed {
                            "has dates and is also listed as missing"
                        } else {
                            "has no dates and isn't listed as missing"
                        }
                    ));
                }
            }
        }
        problems
    }

    /// Whether the file has any dates of the academic year a session belongs to.
    pub fn has_year_of(&self, hint: &SessionHint) -> bool {
        academic_year(&hint.session)
            .is_some_and(|year| self.years.iter().any(|entry| entry.year == year))
    }

    /// The course's dates, when the file has its session for its campus (a full-year course:
    /// its Fall and Winter sessions). `None` otherwise: the session window bounds the course.
    pub fn term(&self, hint: &SessionHint) -> Option<InstitutionTerm> {
        let campus = hint.campus?;
        let (year, month) = session_parts(&hint.session)?;
        let section = hint.section.filter(|s| *s == 'F' || *s == 'S');
        let draft = if hint.full_year {
            let fall = self.find(&hint.session, campus, None)?;
            let winter = self.find(&format!("{}1", year + 1), campus, None)?;
            DatesDraft {
                first_class: fall.first_class,
                last_class: Some(fall.last_class),
                exams_end: winter.exams_end,
                breaks: breaks_of(fall).chain(breaks_of(winter)).collect(),
                second_segment: Some(SecondSegment {
                    first_class: winter.first_class,
                    last_class: Some(winter.last_class),
                    restart_numbering: false,
                }),
            }
        } else {
            // Summer F and S courses have their own dates; the rest take the whole session.
            let summer_section = if month == 5 { section } else { None };
            let dates = self
                .find(&hint.session, campus, summer_section)
                .or_else(|| self.find(&hint.session, campus, None))?;
            DatesDraft {
                first_class: dates.first_class,
                last_class: Some(dates.last_class),
                exams_end: dates.exams_end,
                breaks: breaks_of(dates).collect(),
                second_segment: None,
            }
        };
        built(draft)
    }

    fn find(&self, session: &str, campus: u8, section: Option<char>) -> Option<&SessionDates> {
        self.years
            .iter()
            .flat_map(|year| &year.sessions)
            .find(|dates| {
                dates.session == session
                    && dates.campus == campus
                    && dates.section.as_deref().and_then(|s| s.chars().next()) == section
            })
    }
}

/// The term a draft describes. The file was checked when parsed; a combination that still fails
/// the calendar checks (§7.10) gives no dates.
fn built(draft: DatesDraft) -> Option<InstitutionTerm> {
    let calendar = calendar_from_dates(&draft).ok()?;
    Some(InstitutionTerm {
        segments: calendar.segments,
        breaks: calendar.breaks,
        exams_end: calendar.exam_period.map(|period| period.end),
        draft,
    })
}

fn breaks_of(dates: &SessionDates) -> impl Iterator<Item = CalendarBreak> + '_ {
    dates.breaks.iter().map(|b| CalendarBreak {
        kind: b.kind,
        span: DateSpan {
            start: b.start,
            end: b.end,
        },
        numbered: false,
        label: String::new(),
    })
}

/// "20269" → (2026, 9).
fn session_parts(session: &str) -> Option<(i32, u32)> {
    if session.len() != 5 || !session.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let year = session[..4].parse().ok()?;
    let month = session[4..].parse().ok()?;
    [1, 5, 9].contains(&month).then_some((year, month))
}

/// The academic year a session belongs to: Fall 2026, Winter 2027 and Summer 2027 → "2026-27".
fn academic_year(session: &str) -> Option<String> {
    let (year, month) = session_parts(session)?;
    let start = if month == 9 { year } else { year - 1 };
    Some(format!("{start}-{:02}", (start + 1) % 100))
}

fn check_session(year: &str, dates: &SessionDates) -> Result<(), String> {
    if academic_year(&dates.session).as_deref() != Some(year) {
        return Err(format!("isn't a session of {year}"));
    }
    if !CAMPUSES.contains(&dates.campus) {
        return Err(format!("campus {} isn't 1, 3 or 5", dates.campus));
    }
    if let Some(section) = &dates.section
        && (section != "F" && section != "S" || !dates.session.ends_with('5'))
    {
        return Err("a section is only for a Summer session, \"F\" or \"S\"".to_string());
    }
    if dates.last_class <= dates.first_class {
        return Err("the last class is not after the first".to_string());
    }
    match (dates.exams_start, dates.exams_end) {
        (Some(start), Some(end)) if start <= dates.last_class || end < start => {
            return Err("the exams don't follow the last class".to_string());
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err("give both exams_start and exams_end, or neither".to_string());
        }
        _ => {}
    }
    for b in &dates.breaks {
        if b.end < b.start || b.start < dates.first_class || b.end > dates.last_class {
            return Err(format!(
                "a {} break isn't inside the classes",
                b.kind.as_str()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped file parses, holds no test fixture, and every year it has covers all three
    /// campuses or names the ones it lacks.
    #[test]
    fn the_shipped_calendar_is_checked_dates_only() {
        let calendar = InstitutionCalendar::parse(UOFT).expect("the shipped file parses");
        assert!(
            !calendar.fixture,
            "a fixture's made-up dates must never ship"
        );
        assert!(
            !UOFT.contains("fixture"),
            "no fixture marker in the shipped file"
        );
        assert_eq!(calendar.completeness_problems(), Vec::<String>::new());
        for year in &calendar.years {
            assert!(
                !year.source.trim().is_empty(),
                "{}: say where it was checked",
                year.year
            );
        }
    }

    #[test]
    fn sessions_belong_to_their_academic_year() {
        assert_eq!(academic_year("20269").as_deref(), Some("2026-27"));
        assert_eq!(academic_year("20271").as_deref(), Some("2026-27"));
        assert_eq!(academic_year("20275").as_deref(), Some("2026-27"));
        assert_eq!(academic_year("20263"), None);
        assert_eq!(academic_year("2026"), None);
    }

    #[test]
    fn a_calendar_file_is_checked_when_parsed() {
        let file = |session: &str| {
            format!(
                "format = 1\ninstitution = \"uoft\"\nfixture = true\n[[years]]\nyear = \"2026-27\"\n\
                 source = \"test\"\n[[years.sessions]]\nsession = \"{session}\"\ncampus = 5\n\
                 first_class = \"2026-09-08\"\nlast_class = \"2026-12-04\"\n"
            )
        };
        assert!(InstitutionCalendar::parse(&file("20269")).is_ok());
        assert!(
            InstitutionCalendar::parse(&file("20279")).is_err(),
            "another year"
        );
        assert!(
            InstitutionCalendar::parse("format = 2\ninstitution = \"uoft\"\n").is_err(),
            "unknown format"
        );
        assert!(
            InstitutionCalendar::parse("format = 1\ninstitution = \"elsewhere\"\n").is_err(),
            "unknown school"
        );
        assert!(
            InstitutionCalendar::parse("format = 1\ninstitution = \"uoft\"\nextra = 1\n").is_err(),
            "unknown key"
        );
    }
}
