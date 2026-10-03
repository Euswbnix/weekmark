//! The UofT session hint (docs/design/v0.3-course-calendar.md §6.3, D50).
//!
//! Quercus course names and codes end in a 5-digit session code `YYYYM` (M ∈ {1, 5, 9}), e.g.
//! "DEM332H5 F LEC0101 20269": course DEM332, half-credit (H), campus digit 5, section F,
//! session Fall 2026. The session gives a month window that bounds the course. It is only
//! ever a bound: it never counts weeks, and on its own it never ends a course.
//!
//! Only for courses synced from `q.utoronto.ca` (a 5-digit number means something else at
//! other schools), and folder courses whose `course.toml` says `institution = "uoft"` (the
//! schema v4 column `courses.institution`). The windows are month bounds [unverified; A15
//! confirms]:
//!
//! | Session | Section   | Window                            |
//! |---------|-----------|-----------------------------------|
//! | `YYYY9` | F or none | Sep 1 – Dec 31 of YYYY            |
//! | `YYYY9` | Y         | Sep 1 of YYYY – Apr 30 of YYYY+1  |
//! | `YYYY1` | S or none | Jan 1 – Apr 30 of YYYY            |
//! | `YYYY5` | F         | May 1 – Jun 30 of YYYY            |
//! | `YYYY5` | S         | Jul 1 – Aug 31 of YYYY            |
//! | `YYYY5` | Y or none | May 1 – Aug 31 of YYYY            |
//!
//! Any other combination gives no hint.

use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;

use super::DateSpan;
use crate::model::Course;

/// Canvas hosts whose courses carry UofT session codes.
const UOFT_CANVAS_SOURCES: &[&str] = &["canvas:q.utoronto.ca"];

/// A session code and the window it implies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionHint {
    /// The 5-digit code, e.g. "20269".
    pub session: String,
    /// 'F', 'S' or 'Y', when the course names one.
    pub section: Option<char>,
    /// The campus digit of the course code (1 St. George, 3 Scarborough, 5 Mississauga).
    pub campus: Option<u8>,
    pub window: DateSpan,
    /// A full-year (Y) section: terms up to 36 weeks are plausible.
    pub full_year: bool,
}

/// The session code at the very end: "… 20269".
static SESSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)((?:19|20)\d{2})([159])\s*$").expect("valid regex"));

/// A UofT course code with its section: "DEM332H5 F", "DEM137Y5 Y".
static CODE_SECTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u:\b)[A-Z]{3}[0-9A-Z]\d{2}([HY])(\d)(?:\s+([FSY])(?-u:\b))?")
        .expect("valid regex")
});

/// The hint for `course`, host-gated: a UofT Canvas host, or a folder course whose
/// `course.toml` names `institution` "uoft" (see the module docs).
pub fn session_hint(course: &Course, institution: Option<&str>) -> Option<SessionHint> {
    let uoft = UOFT_CANVAS_SOURCES.contains(&course.source_id.as_str())
        || (course.source_id.starts_with("folder:") && institution == Some("uoft"));
    if !uoft {
        return None;
    }
    parse_uoft_session(course.code.as_deref(), &course.name)
}

/// Parse a UofT session code from a course's code or name (no host check).
pub fn parse_uoft_session(code: Option<&str>, name: &str) -> Option<SessionHint> {
    let texts: Vec<&str> = code.into_iter().chain([name]).collect();
    let (year, month) = texts.iter().find_map(|text| {
        let captures = SESSION.captures(text.trim())?;
        let year: i32 = captures[1].parse().ok()?;
        let month: u32 = captures[2].parse().ok()?;
        Some((year, month))
    })?;
    let (weight, campus, section) = texts
        .iter()
        .find_map(|text| {
            let captures = CODE_SECTION.captures(text)?;
            let weight = captures[1].chars().next()?;
            let campus = captures[2].parse::<u8>().ok();
            let section = captures.get(3).and_then(|m| m.as_str().chars().next());
            Some((Some(weight), campus, section))
        })
        .unwrap_or((None, None, None));
    // A full-credit (Y) course without a section letter runs the whole session.
    let section = section.or(if weight == Some('Y') { Some('Y') } else { None });
    let window = window(year, month, section)?;
    Some(SessionHint {
        session: format!("{year}{month}"),
        section,
        campus,
        window,
        full_year: section == Some('Y') && month == 9,
    })
}

fn window(year: i32, month: u32, section: Option<char>) -> Option<DateSpan> {
    let span = |from: (i32, u32, u32), to: (i32, u32, u32)| {
        Some(DateSpan {
            start: NaiveDate::from_ymd_opt(from.0, from.1, from.2)?,
            end: NaiveDate::from_ymd_opt(to.0, to.1, to.2)?,
        })
    };
    match (month, section) {
        (9, Some('F') | None) => span((year, 9, 1), (year, 12, 31)),
        (9, Some('Y')) => span((year, 9, 1), (year + 1, 4, 30)),
        (1, Some('S') | None) => span((year, 1, 1), (year, 4, 30)),
        (5, Some('F')) => span((year, 5, 1), (year, 6, 30)),
        (5, Some('S')) => span((year, 7, 1), (year, 8, 31)),
        (5, Some('Y') | None) => span((year, 5, 1), (year, 8, 31)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AiPolicy, TermSource};

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn course(source: &str, code: Option<&str>, name: &str) -> Course {
        Course {
            id: format!("{source}/course/1"),
            source_id: source.into(),
            external_id: "1".into(),
            code: code.map(Into::into),
            name: name.into(),
            term_start: None,
            term_end: None,
            term_source: TermSource::None,
            url: None,
            ai_policy: AiPolicy::Unknown,
            ai_policy_note: None,
            ai_access: true,
            material_sharing: Default::default(),
            hidden: false,
            enrollment_active: true,
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn parses_session_section_and_campus() {
        let hint = parse_uoft_session(Some("DEM332H5 F LEC0101 20269"), "Demo Methods").unwrap();
        assert_eq!(hint.session, "20269");
        assert_eq!(hint.section, Some('F'));
        assert_eq!(hint.campus, Some(5));
        assert_eq!(
            hint.window,
            DateSpan {
                start: date(2026, 9, 1),
                end: date(2026, 12, 31)
            }
        );
        assert!(!hint.full_year);

        // The code may be in the name only; Y sections span two years.
        let y = parse_uoft_session(None, "DEM137Y5 Y LEC0101 20269").unwrap();
        assert_eq!(y.window.end, date(2027, 4, 30));
        assert!(y.full_year);
        // A full-credit course without a section letter is a Y section.
        let y = parse_uoft_session(Some("DEM137Y1 LEC0101 20269"), "x").unwrap();
        assert_eq!(y.section, Some('Y'));

        let winter = parse_uoft_session(Some("DEM210H1 S LEC0201 20271"), "x").unwrap();
        assert_eq!(winter.window.start, date(2027, 1, 1));
        assert_eq!(winter.window.end, date(2027, 4, 30));
    }

    #[test]
    fn summer_sections_have_their_own_windows() {
        let window = |code: &str| parse_uoft_session(Some(code), "x").map(|h| h.window);
        assert_eq!(
            window("DEM101H5 F LEC0101 20265").unwrap().end,
            date(2026, 6, 30)
        );
        assert_eq!(
            window("DEM101H5 S LEC0101 20265").unwrap().start,
            date(2026, 7, 1)
        );
        assert_eq!(
            window("DEM101Y5 Y LEC0101 20265").unwrap(),
            DateSpan {
                start: date(2026, 5, 1),
                end: date(2026, 8, 31)
            }
        );
        // Summer Y is not the 36-week Fall/Winter full year.
        assert!(
            !parse_uoft_session(Some("DEM101Y5 Y LEC0101 20265"), "x")
                .unwrap()
                .full_year
        );
    }

    #[test]
    fn other_numbers_and_combinations_give_no_hint() {
        for code in [
            "DEM332H5 F LEC0101",       // no session
            "DEM332H5 F LEC0101 20263", // month 3
            "DEM332H5 F LEC0101 12345", // not a year
            "DEM332H5 S LEC0101 20269", // S in a September code: not in the table
            "DEM332H5 F LEC0101 20269 extra",
            "Room 20269 booking notes",
        ] {
            assert_eq!(parse_uoft_session(Some(code), "Name"), None, "{code}");
        }
    }

    /// CAL-17 `session_hint_is_host_gated`: the same code on another host gives no hint.
    #[test]
    fn session_hint_is_host_gated() {
        let uoft = course(
            "canvas:q.utoronto.ca",
            Some("DEM332H5 F LEC0101 20269"),
            "Demo Methods",
        );
        assert!(session_hint(&uoft, None).is_some());
        for source in [
            "canvas:lms.example.edu",
            "canvas:q.utoronto.ca.example.com",
            "folder:3f2a",
        ] {
            let other = course(source, Some("DEM332H5 F LEC0101 20269"), "Demo Methods");
            assert!(session_hint(&other, None).is_none(), "{source}");
        }
        // A folder opts in with `institution = "uoft"` in course.toml; a Canvas host can't.
        let folder = course(
            "folder:3f2a",
            Some("DEM332H5 F LEC0101 20269"),
            "Demo Methods",
        );
        assert!(session_hint(&folder, Some("uoft")).is_some());
        assert!(session_hint(&folder, Some("other")).is_none());
        let canvas = course(
            "canvas:lms.example.edu",
            Some("DEM332H5 F LEC0101 20269"),
            "Demo Methods",
        );
        assert!(session_hint(&canvas, Some("uoft")).is_none());
    }
}
