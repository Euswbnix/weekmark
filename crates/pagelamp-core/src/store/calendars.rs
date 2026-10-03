//! Store methods of course calendars (schema 4, `course_calendars`; docs/design/
//! v0.3-course-calendar.md §3.2, §7.10): proposals, the calendar in force, and what the
//! student's accept, dismiss or dates form does to them.
//!
//! - One accepted row per course (the calendar in force) and at most one proposed row per
//!   course and origin: a newer proposal from the same reader replaces the older one.
//! - Accepting (or the dates form) supersedes the calendar in force and mirrors the first class
//!   and the end of exams (else the last day of classes) into `user_term_start/end`, so v3
//!   readers still see the right span (§3.2 "Version skew").
//! - Retention: the latest 3 superseded rows per course (for undo) and dismissed rows for 30
//!   days (their fingerprint stops the same materials from being proposed again).
//!
//! Rows hold material text (labels, topics, quotes): local display only (§7.11).

use chrono::Duration;
use rusqlite::{Row, params};
use serde::{Deserialize, Serialize};

use super::{Store, TextValue, expect_changed, get_opt_value, get_value, opt_date_text, ts_text};
use crate::Result;
use crate::ai_gate::ManifestEntry;
use crate::calendar::CourseCalendar;
use crate::calendar::assemble::{CalendarChange, CalendarConflict, ProposedDate};
use crate::calendar::candidates::CandidateSignals;
use crate::calendar::text::{find_quote, rebuild_parts};
use crate::calendar::validate::DropCount;
use crate::model::Timestamp;
use crate::term::{CalendarOrigin, CoursePhase};

/// Superseded rows kept per course (for undo).
pub const KEEP_SUPERSEDED: usize = 3;
/// Days a dismissed row is kept.
pub const KEEP_DISMISSED_DAYS: i64 = 30;

/// Where a row is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CalendarState {
    Proposed,
    /// The calendar in force.
    Accepted,
    Dismissed,
    /// Was in force; replaced by a later accept or dates form.
    Superseded,
}

impl CalendarState {
    pub fn as_str(self) -> &'static str {
        match self {
            CalendarState::Proposed => "proposed",
            CalendarState::Accepted => "accepted",
            CalendarState::Dismissed => "dismissed",
            CalendarState::Superseded => "superseded",
        }
    }
}

impl TextValue for CalendarState {
    fn parse_text(text: &str) -> Option<Self> {
        [
            CalendarState::Proposed,
            CalendarState::Accepted,
            CalendarState::Dismissed,
            CalendarState::Superseded,
        ]
        .into_iter()
        .find(|state| state.as_str() == text)
    }
}

impl TextValue for CalendarOrigin {
    fn parse_text(text: &str) -> Option<Self> {
        [
            CalendarOrigin::User,
            CalendarOrigin::Legacy,
            CalendarOrigin::Scan,
            CalendarOrigin::Ai,
            CalendarOrigin::AiApp,
            CalendarOrigin::Restored,
        ]
        .into_iter()
        .find(|origin| origin.as_str() == text)
    }
}

/// What the checks found, stored with a row (`checks_json`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarChecks {
    pub conflicts: Vec<CalendarConflict>,
    pub dropped: Vec<DropCount>,
    pub low_quality: bool,
    pub passing: bool,
    /// V8 found disagreement: once accepted, the calendar counts weeks at Medium.
    pub disagrees_with_notes: bool,
    /// Show the one-time question (b) reminder with this proposal (D37 option 2).
    pub sharing_reminder: bool,
    /// Today's week and phase once accepted, and what accepting changes, as computed when the
    /// proposal was made.
    pub resulting_week_today: Option<u32>,
    pub resulting_phase: Option<CoursePhase>,
    pub changes: Vec<CalendarChange>,
}

/// Where an AI-read calendar came from (the "AI-generated · backend · model · date" label).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarProvenance {
    pub generation_id: Option<String>,
    /// As the label shows it ("ChatGPT plan (through OpenAI Codex)").
    pub backend_label: String,
    pub model: String,
    pub prompt_version: u32,
    /// The model ran on this computer (stored in `checks_json`, next to the checks).
    pub on_device: bool,
}

/// A row to write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewCalendarRow {
    pub course_id: String,
    pub origin: CalendarOrigin,
    pub calendar: CourseCalendar,
    /// The dates with their quotes (`evidence_json`); empty for the student's own dates.
    pub dates: Vec<ProposedDate>,
    pub checks: CalendarChecks,
    pub manifest: Vec<ManifestEntry>,
    pub fingerprint: String,
    pub provenance: Option<CalendarProvenance>,
}

/// A stored row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarRow {
    pub id: i64,
    pub course_id: String,
    pub origin: CalendarOrigin,
    pub state: CalendarState,
    pub calendar: CourseCalendar,
    pub dates: Vec<ProposedDate>,
    pub checks: CalendarChecks,
    pub manifest: Vec<ManifestEntry>,
    pub fingerprint: String,
    pub provenance: Option<CalendarProvenance>,
    pub created_at: Timestamp,
    pub decided_at: Option<Timestamp>,
}

const CALENDAR_COLUMNS: &str = "id, course_id, origin, state, calendar_json, evidence_json, \
     checks_json, manifest_json, fingerprint, generation_id, backend, model, prompt_version, \
     created_at, decided_at";

fn json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("calendar rows serialise")
}

/// A JSON column, or a conversion error naming it (never a panic on a bad row).
fn from_json<T: serde::de::DeserializeOwned>(row: &Row<'_>, column: &str) -> rusqlite::Result<T> {
    let text: String = row.get(column)?;
    serde_json::from_str(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("bad {column}: {error}").into(),
        )
    })
}

/// `checks_json` as stored: the checks, plus whether the reading model ran on this computer.
#[derive(Serialize, Deserialize)]
struct StoredChecks {
    #[serde(flatten)]
    checks: CalendarChecks,
    #[serde(default)]
    on_device: bool,
}

fn calendar_from_row(row: &Row<'_>) -> rusqlite::Result<CalendarRow> {
    let backend: Option<String> = row.get("backend")?;
    let model: Option<String> = row.get("model")?;
    let stored: StoredChecks = from_json(row, "checks_json")?;
    let provenance = match (backend, model) {
        (Some(backend_label), Some(model)) => Some(CalendarProvenance {
            generation_id: row.get("generation_id")?,
            backend_label,
            model,
            prompt_version: row
                .get::<_, Option<u32>>("prompt_version")?
                .unwrap_or_default(),
            on_device: stored.on_device,
        }),
        _ => None,
    };
    Ok(CalendarRow {
        id: row.get("id")?,
        course_id: row.get("course_id")?,
        origin: get_value(row, "origin")?,
        state: get_value(row, "state")?,
        calendar: from_json(row, "calendar_json")?,
        dates: from_json(row, "evidence_json")?,
        checks: stored.checks,
        manifest: from_json(row, "manifest_json")?,
        fingerprint: row.get("fingerprint")?,
        provenance,
        created_at: get_value(row, "created_at")?,
        decided_at: get_opt_value(row, "decided_at")?,
    })
}

/// The span a calendar mirrors into `user_term_start/end`: the first class, and the end of
/// exams, else the last segment's last day of classes.
fn mirrored_span(
    calendar: &CourseCalendar,
) -> (Option<chrono::NaiveDate>, Option<chrono::NaiveDate>) {
    let start = calendar.segments.first().map(|s| s.first_class);
    let end = calendar
        .exam_period
        .map(|span| span.end)
        .or_else(|| calendar.segments.last().and_then(|s| s.last_class));
    (start, end)
}

/// Whether an accepted calendar's quotes are still in its materials (§7.8; computed when read,
/// never stored).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Staleness {
    /// A quoted material changed and some quote is no longer in it (or it is gone). The
    /// calendar stays in force: it is still the best evidence.
    pub stale: bool,
    /// The quoted materials whose text changed, whether or not their quotes are still there.
    pub changed_materials: Vec<String>,
}

impl Store {
    /// §7.8 for `row`: a quoted material whose content hash changed is searched again for each
    /// of its quotes. A calendar without quotes (the student's own, a legacy one) never goes
    /// stale.
    pub fn calendar_staleness(&self, row: &CalendarRow) -> Result<Staleness> {
        let mut staleness = Staleness::default();
        for entry in &row.manifest {
            let current = self.get_material(&entry.material_id)?;
            if current.as_ref().map(|m| &m.content_hash) == Some(&entry.content_hash) {
                continue;
            }
            staleness.changed_materials.push(entry.material_id.clone());
            let quotes: Vec<&str> = row
                .dates
                .iter()
                .flat_map(|date| {
                    date.evidence
                        .iter()
                        .chain(date.alternatives.iter().flat_map(|alt| &alt.evidence))
                })
                .filter(|evidence| evidence.material_id == entry.material_id)
                .filter_map(|evidence| evidence.quote.as_deref())
                .collect();
            if quotes.is_empty() {
                continue;
            }
            let parts = match current {
                Some(_) => rebuild_parts(&self.get_chunks(&entry.material_id, 0, None)?),
                None => Vec::new(),
            };
            if quotes
                .iter()
                .any(|quote| find_quote(&parts, quote).is_none())
            {
                staleness.stale = true;
            }
        }
        Ok(staleness)
    }

    /// The candidate signals schema 4 stores for a course: files the syllabus links to, the
    /// front page, the outline a folder's `course.toml` names (all sync-written) and the
    /// student's own adds and removes.
    pub fn calendar_signals(&self, course_id: &str) -> Result<CandidateSignals> {
        let mut signals = CandidateSignals::default();
        let flagged: Vec<(String, bool, bool, bool)> = self.query_list(
            "SELECT id, linked_from_syllabus, is_front_page, named_outline FROM materials
             WHERE course_id = ?1
               AND (linked_from_syllabus = 1 OR is_front_page = 1 OR named_outline = 1)",
            [course_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        for (id, linked, front, named) in flagged {
            if linked {
                signals.linked_from_syllabus.insert(id.clone());
            }
            if front {
                signals.front_page.insert(id.clone());
            }
            if named {
                signals.named_in_course_toml.insert(id);
            }
        }
        let stored: Option<String> = self
            .query_opt(
                "SELECT calendar_sources FROM courses WHERE id = ?1",
                [course_id],
                |row| row.get(0),
            )?
            .flatten();
        if let Some(text) = stored {
            // An unreadable value is treated as no choices (it is rewritten on the next edit).
            signals.choices = serde_json::from_str(&text).unwrap_or_default();
        }
        Ok(signals)
    }

    /// The sync's calendar signals for a course's materials (S4, S5): the files its syllabus
    /// links to, and its front page. `None` leaves that flag as it is (the sync couldn't tell);
    /// `Some(None)` for the front page: there is none.
    pub fn set_calendar_links(
        &self,
        course_id: &str,
        linked_from_syllabus: Option<&std::collections::BTreeSet<String>>,
        front_page: Option<Option<&str>>,
    ) -> Result<()> {
        if let Some(linked) = linked_from_syllabus {
            let ids = serde_json::to_string(linked).expect("ids serialise");
            self.conn.execute(
                "UPDATE materials SET linked_from_syllabus =
                     (id IN (SELECT value FROM json_each(?2)))
                 WHERE course_id = ?1",
                params![course_id, ids],
            )?;
        }
        if let Some(front) = front_page {
            self.conn.execute(
                "UPDATE materials SET is_front_page = (id IS ?2) WHERE course_id = ?1",
                params![course_id, front],
            )?;
        }
        Ok(())
    }

    /// The student's adds (`true`) and removes (`false`) of candidate materials.
    pub fn set_calendar_sources(
        &self,
        course_id: &str,
        choices: &std::collections::BTreeMap<String, bool>,
    ) -> Result<()> {
        let value = (!choices.is_empty()).then(|| json(choices));
        let changed = self.conn.execute(
            "UPDATE courses SET calendar_sources = ?2 WHERE id = ?1",
            params![course_id, value],
        )?;
        expect_changed(changed, "course", course_id)
    }

    /// Store a proposal (state `proposed`); a proposal from the same origin for the course is
    /// replaced. Returns its id.
    pub fn insert_calendar_proposal(&self, row: &NewCalendarRow, now: Timestamp) -> Result<i64> {
        self.atomic(|| {
            self.conn.execute(
                "DELETE FROM course_calendars
                 WHERE course_id = ?1 AND origin = ?2 AND state = 'proposed'",
                params![row.course_id, row.origin.as_str()],
            )?;
            self.insert_calendar_row(row, CalendarState::Proposed, now, None)
        })
    }

    fn insert_calendar_row(
        &self,
        row: &NewCalendarRow,
        state: CalendarState,
        now: Timestamp,
        decided_at: Option<Timestamp>,
    ) -> Result<i64> {
        let provenance = row.provenance.as_ref();
        self.conn.execute(
            "INSERT INTO course_calendars
                 (course_id, origin, state, calendar_json, evidence_json, checks_json,
                  manifest_json, fingerprint, generation_id, backend, model, prompt_version,
                  created_at, decided_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                row.course_id,
                row.origin.as_str(),
                state.as_str(),
                json(&row.calendar),
                json(&row.dates),
                json(&StoredChecks {
                    checks: row.checks.clone(),
                    on_device: provenance.is_some_and(|p| p.on_device),
                }),
                json(&row.manifest),
                row.fingerprint,
                provenance.and_then(|p| p.generation_id.as_deref()),
                provenance.map(|p| p.backend_label.as_str()),
                provenance.map(|p| p.model.as_str()),
                provenance.map(|p| p.prompt_version),
                ts_text(now),
                decided_at.map(ts_text),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// One row by id, in any state.
    pub fn calendar_row(&self, id: i64) -> Result<Option<CalendarRow>> {
        self.query_opt(
            &format!("SELECT {CALENDAR_COLUMNS} FROM course_calendars WHERE id = ?1"),
            [id],
            calendar_from_row,
        )
    }

    /// The course's pending proposals, newest first.
    pub fn calendar_proposals(&self, course_id: &str) -> Result<Vec<CalendarRow>> {
        self.query_list(
            &format!(
                "SELECT {CALENDAR_COLUMNS} FROM course_calendars
                 WHERE course_id = ?1 AND state = 'proposed'
                 ORDER BY created_at DESC, id DESC"
            ),
            [course_id],
            calendar_from_row,
        )
    }

    /// The course's calendar in force.
    pub fn accepted_calendar(&self, course_id: &str) -> Result<Option<CalendarRow>> {
        self.query_opt(
            &format!(
                "SELECT {CALENDAR_COLUMNS} FROM course_calendars
                 WHERE course_id = ?1 AND state = 'accepted'"
            ),
            [course_id],
            calendar_from_row,
        )
    }

    /// Accept proposal `id`, optionally with the student's edits (a new calendar and its
    /// dates): the calendar in force becomes superseded, and the span is mirrored into
    /// `user_term_*`. `NotFound` unless `id` is a pending proposal.
    pub fn accept_calendar_proposal(
        &self,
        id: i64,
        edited: Option<(CourseCalendar, Vec<ProposedDate>)>,
        now: Timestamp,
    ) -> Result<CalendarRow> {
        self.atomic(|| {
            let row = self
                .calendar_row(id)?
                .filter(|row| row.state == CalendarState::Proposed)
                .ok_or_else(|| crate::Error::NotFound(format!("calendar proposal '{id}'")))?;
            self.supersede_calendar(&row.course_id, now)?;
            let now_text = ts_text(now);
            match edited {
                Some((calendar, dates)) => self.conn.execute(
                    "UPDATE course_calendars
                     SET state = 'accepted', decided_at = ?2, calendar_json = ?3,
                         evidence_json = ?4
                     WHERE id = ?1",
                    params![id, now_text, json(&calendar), json(&dates)],
                )?,
                None => self.conn.execute(
                    "UPDATE course_calendars SET state = 'accepted', decided_at = ?2 WHERE id = ?1",
                    params![id, now_text],
                )?,
            };
            let accepted = self
                .calendar_row(id)?
                .ok_or_else(|| crate::Error::NotFound(format!("calendar proposal '{id}'")))?;
            self.mirror_calendar(&accepted.course_id, Some(&accepted.calendar))?;
            self.prune_superseded(&accepted.course_id)?;
            Ok(accepted)
        })
    }

    /// The student's own dates (the dates form): `Some` puts a `user` calendar in force, `None`
    /// clears the calendar in force and `user_term_*` ("Undo"). Returns the new row.
    pub fn set_student_calendar(
        &self,
        course_id: &str,
        calendar: Option<&CourseCalendar>,
        now: Timestamp,
    ) -> Result<Option<CalendarRow>> {
        self.atomic(|| {
            self.supersede_calendar(course_id, now)?;
            self.mirror_calendar(course_id, calendar)?;
            let Some(calendar) = calendar else {
                return Ok(None);
            };
            let id = self.insert_calendar_row(
                &NewCalendarRow {
                    course_id: course_id.to_string(),
                    origin: CalendarOrigin::User,
                    calendar: calendar.clone(),
                    dates: Vec::new(),
                    checks: CalendarChecks {
                        passing: true,
                        ..CalendarChecks::default()
                    },
                    manifest: Vec::new(),
                    fingerprint: "user".to_string(),
                    provenance: None,
                },
                CalendarState::Accepted,
                now,
                Some(now),
            )?;
            self.prune_superseded(course_id)?;
            self.calendar_row(id)
        })
    }

    /// Dismiss proposal `id` (kept 30 days, so its materials aren't proposed again).
    pub fn dismiss_calendar_proposal(&self, id: i64, now: Timestamp) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE course_calendars SET state = 'dismissed', decided_at = ?2
             WHERE id = ?1 AND state = 'proposed'",
            params![id, ts_text(now)],
        )?;
        expect_changed(changed, "calendar proposal", &id.to_string())
    }

    /// Whether the student dismissed a proposal of `origin` made from these materials
    /// (`fingerprint`) in the last 30 days.
    pub fn calendar_fingerprint_dismissed(
        &self,
        course_id: &str,
        origin: CalendarOrigin,
        fingerprint: &str,
    ) -> Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM course_calendars
             WHERE course_id = ?1 AND origin = ?2 AND fingerprint = ?3 AND state = 'dismissed'",
            params![course_id, origin.as_str(), fingerprint],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    /// Delete dismissed rows older than 30 days.
    pub fn prune_dismissed_calendars(&self, now: Timestamp) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM course_calendars WHERE state = 'dismissed' AND decided_at < ?1",
            [ts_text(now - Duration::days(KEEP_DISMISSED_DAYS))],
        )?)
    }

    /// The calendar in force becomes superseded.
    fn supersede_calendar(&self, course_id: &str, now: Timestamp) -> Result<()> {
        self.conn.execute(
            "UPDATE course_calendars SET state = 'superseded', decided_at = ?2
             WHERE course_id = ?1 AND state = 'accepted'",
            params![course_id, ts_text(now)],
        )?;
        Ok(())
    }

    /// Mirror `calendar`'s span into `user_term_*` (`None` clears them).
    fn mirror_calendar(&self, course_id: &str, calendar: Option<&CourseCalendar>) -> Result<()> {
        let (start, end) = calendar.map(mirrored_span).unwrap_or((None, None));
        let changed = self.conn.execute(
            "UPDATE courses SET user_term_start = ?2, user_term_end = ?3 WHERE id = ?1",
            params![course_id, opt_date_text(start), opt_date_text(end)],
        )?;
        expect_changed(changed, "course", course_id)
    }

    /// Keep the latest 3 superseded rows of the course.
    fn prune_superseded(&self, course_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM course_calendars
             WHERE course_id = ?1 AND state = 'superseded' AND id NOT IN (
                 SELECT id FROM course_calendars
                 WHERE course_id = ?1 AND state = 'superseded'
                 ORDER BY decided_at DESC, id DESC LIMIT ?2)",
            params![course_id, KEEP_SUPERSEDED as i64],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, NaiveDate, Utc};
    use serde_json::json;

    use super::*;
    use crate::calendar::legacy_calendar;
    use crate::model::{CourseUpsert, SourceKind, SourceRecord};
    use crate::term::DateSpan;

    const COURSE: &str = "canvas:lms.example.edu/course/101";

    fn date(text: &str) -> NaiveDate {
        NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("{text}T12:00:00Z"))
            .unwrap()
            .with_timezone(&Utc)
    }

    fn demo_store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_source(&SourceRecord {
                id: "canvas:lms.example.edu".into(),
                kind: SourceKind::Canvas,
                label: "Demo LMS".into(),
                config: json!({}),
                last_synced_at: None,
                last_error: None,
                last_error_kind: None,
            })
            .unwrap();
        store
            .upsert_course(&CourseUpsert {
                id: COURSE.into(),
                source_id: "canvas:lms.example.edu".into(),
                external_id: "101".into(),
                code: Some("DEMO101".into()),
                name: "Intro to Demo Studies".into(),
                term_start: None,
                term_end: None,
                url: None,
                syllabus_text: None,
                lms: Default::default(),
            })
            .unwrap();
        store
    }

    fn proposal(origin: CalendarOrigin, first_class: &str, fingerprint: &str) -> NewCalendarRow {
        let mut calendar = legacy_calendar(date(first_class), Some(date("2026-12-08")));
        calendar.exam_period = Some(DateSpan {
            start: date("2026-12-10"),
            end: date("2026-12-21"),
        });
        NewCalendarRow {
            course_id: COURSE.into(),
            origin,
            calendar,
            dates: Vec::new(),
            checks: CalendarChecks {
                passing: true,
                sharing_reminder: origin == CalendarOrigin::Ai,
                ..CalendarChecks::default()
            },
            manifest: vec![ManifestEntry {
                material_id: "m1".into(),
                content_hash: Some("h1".into()),
                chunk_ords: vec![0, 1],
            }],
            fingerprint: fingerprint.into(),
            provenance: (origin == CalendarOrigin::Ai).then(|| CalendarProvenance {
                generation_id: None,
                backend_label: "ChatGPT plan (through OpenAI Codex)".into(),
                model: "gpt-6-luna".into(),
                prompt_version: 1,
                on_device: false,
            }),
        }
    }

    fn user_term(store: &Store) -> (Option<NaiveDate>, Option<NaiveDate>) {
        let data = store.course_term_data(COURSE).unwrap().unwrap();
        (data.user_term_start, data.user_term_end)
    }

    #[test]
    fn a_newer_proposal_replaces_the_same_readers_older_one() {
        let store = demo_store();
        let scan = store
            .insert_calendar_proposal(
                &proposal(CalendarOrigin::Scan, "2026-09-08", "f1"),
                at("2026-09-20"),
            )
            .unwrap();
        let ai = store
            .insert_calendar_proposal(
                &proposal(CalendarOrigin::Ai, "2026-09-09", "f1"),
                at("2026-09-21"),
            )
            .unwrap();
        let rescan = store
            .insert_calendar_proposal(
                &proposal(CalendarOrigin::Scan, "2026-09-10", "f2"),
                at("2026-09-22"),
            )
            .unwrap();
        let ids: Vec<i64> = store
            .calendar_proposals(COURSE)
            .unwrap()
            .iter()
            .map(|row| row.id)
            .collect();
        assert_eq!(ids, [rescan, ai]);
        assert!(store.calendar_row(scan).unwrap().is_none());
        let ai = store.calendar_row(ai).unwrap().unwrap();
        assert_eq!(ai.state, CalendarState::Proposed);
        assert_eq!(ai.provenance.unwrap().model, "gpt-6-luna");
        assert!(ai.checks.sharing_reminder);
        assert_eq!(ai.manifest[0].chunk_ords, [0, 1]);
        assert!(store.accepted_calendar(COURSE).unwrap().is_none());
    }

    #[test]
    fn accepting_puts_one_calendar_in_force_and_mirrors_its_span() {
        let store = demo_store();
        let first = store
            .insert_calendar_proposal(
                &proposal(CalendarOrigin::Scan, "2026-09-08", "f1"),
                at("2026-09-20"),
            )
            .unwrap();
        let second = store
            .insert_calendar_proposal(
                &proposal(CalendarOrigin::Ai, "2026-09-09", "f1"),
                at("2026-09-20"),
            )
            .unwrap();
        let accepted = store
            .accept_calendar_proposal(first, None, at("2026-09-21"))
            .unwrap();
        assert_eq!(accepted.state, CalendarState::Accepted);
        assert_eq!(accepted.decided_at, Some(at("2026-09-21")));
        // v3 readers see the first class and the end of exams.
        assert_eq!(
            user_term(&store),
            (Some(date("2026-09-08")), Some(date("2026-12-21")))
        );
        // The other proposal stays until the student decides; accepting it (edited) replaces
        // the calendar in force.
        let mut edited = store.calendar_row(second).unwrap().unwrap().calendar;
        edited.exam_period = None;
        let second = store
            .accept_calendar_proposal(second, Some((edited, Vec::new())), at("2026-09-22"))
            .unwrap();
        assert_eq!(
            store.accepted_calendar(COURSE).unwrap().unwrap().id,
            second.id
        );
        assert_eq!(second.calendar.exam_period, None);
        assert_eq!(
            user_term(&store),
            (Some(date("2026-09-09")), Some(date("2026-12-08")))
        );
        assert_eq!(
            store.calendar_row(first).unwrap().unwrap().state,
            CalendarState::Superseded
        );
        // Only pending proposals can be accepted.
        assert!(matches!(
            store.accept_calendar_proposal(first, None, at("2026-09-23")),
            Err(crate::Error::NotFound(_))
        ));
    }

    #[test]
    fn the_dates_form_and_undo() {
        let store = demo_store();
        let calendar = legacy_calendar(date("2026-09-08"), Some(date("2026-12-08")));
        let row = store
            .set_student_calendar(COURSE, Some(&calendar), at("2026-09-21"))
            .unwrap()
            .unwrap();
        assert_eq!(
            (row.origin, row.state),
            (CalendarOrigin::User, CalendarState::Accepted)
        );
        assert_eq!(
            user_term(&store),
            (Some(date("2026-09-08")), Some(date("2026-12-08")))
        );
        assert!(
            store
                .set_student_calendar(COURSE, None, at("2026-09-22"))
                .unwrap()
                .is_none()
        );
        assert!(store.accepted_calendar(COURSE).unwrap().is_none());
        assert_eq!(user_term(&store), (None, None));
        assert_eq!(
            store.calendar_row(row.id).unwrap().unwrap().state,
            CalendarState::Superseded
        );
    }

    #[test]
    fn three_superseded_calendars_are_kept_for_undo() {
        let store = demo_store();
        let mut ids = Vec::new();
        for day in 10..16 {
            let calendar = legacy_calendar(date(&format!("2026-09-{day}")), None);
            let row = store
                .set_student_calendar(COURSE, Some(&calendar), at(&format!("2026-09-{day}")))
                .unwrap()
                .unwrap();
            ids.push(row.id);
        }
        let kept: Vec<i64> = ids
            .iter()
            .copied()
            .filter(|id| store.calendar_row(*id).unwrap().is_some())
            .collect();
        // The one in force and the latest 3 before it.
        assert_eq!(kept, ids[2..]);
    }

    #[test]
    fn dismissed_materials_are_remembered_for_thirty_days() {
        let store = demo_store();
        let id = store
            .insert_calendar_proposal(
                &proposal(CalendarOrigin::Scan, "2026-09-08", "f1"),
                at("2026-09-20"),
            )
            .unwrap();
        store
            .dismiss_calendar_proposal(id, at("2026-09-21"))
            .unwrap();
        assert!(store.calendar_proposals(COURSE).unwrap().is_empty());
        assert!(
            store
                .calendar_fingerprint_dismissed(COURSE, CalendarOrigin::Scan, "f1")
                .unwrap()
        );
        assert!(
            !store
                .calendar_fingerprint_dismissed(COURSE, CalendarOrigin::Scan, "f2")
                .unwrap()
        );
        assert!(matches!(
            store.dismiss_calendar_proposal(id, at("2026-09-21")),
            Err(crate::Error::NotFound(_))
        ));
        assert_eq!(
            store.prune_dismissed_calendars(at("2026-10-20")).unwrap(),
            0
        );
        assert_eq!(
            store.prune_dismissed_calendars(at("2026-10-22")).unwrap(),
            1
        );
        assert!(
            !store
                .calendar_fingerprint_dismissed(COURSE, CalendarOrigin::Scan, "f1")
                .unwrap()
        );
    }

    fn add_material(store: &Store, id: &str, hash: &str, text: &str) {
        store
            .upsert_material(&crate::model::MaterialUpsert {
                id: id.into(),
                course_id: COURSE.into(),
                module_id: None,
                kind: crate::model::MaterialKind::Syllabus,
                title: "Syllabus".into(),
                url: None,
                local_path: None,
                mime: None,
                published_at: None,
                week_hint: None,
            })
            .unwrap();
        store
            .set_text_state(id, crate::model::TextStatus::Ok, None, Some(hash))
            .unwrap();
        store
            .replace_chunks(
                id,
                &[crate::model::Chunk {
                    material_id: id.into(),
                    ord: 0,
                    locator: None,
                    text: text.into(),
                }],
            )
            .unwrap();
    }

    #[test]
    fn a_changed_syllabus_makes_the_calendar_stale_only_when_a_quote_is_gone() {
        use crate::calendar::assemble::DateKind;
        use crate::calendar::validate::DateEvidence;

        let store = demo_store();
        add_material(
            &store,
            "m1",
            "h1",
            "Welcome. Classes begin September 8, 2026.",
        );
        let mut row = proposal(CalendarOrigin::Scan, "2026-09-08", "f1");
        row.dates = vec![ProposedDate {
            kind: DateKind::FirstClass,
            segment: 0,
            date: date("2026-09-08"),
            end: None,
            label: String::new(),
            evidence: vec![DateEvidence {
                material_id: "m1".into(),
                title: "Syllabus".into(),
                locator: None,
                quote: Some("Classes begin September 8, 2026.".into()),
                url: None,
                derived: false,
            }],
            alternatives: Vec::new(),
            week: None,
            break_kind: None,
            numbered: None,
        }];
        let id = store
            .insert_calendar_proposal(&row, at("2026-09-20"))
            .unwrap();
        let accepted = store
            .accept_calendar_proposal(id, None, at("2026-09-21"))
            .unwrap();
        assert_eq!(
            store.calendar_staleness(&accepted).unwrap(),
            Staleness::default()
        );

        // The syllabus is edited but still says it: "updated, dates unchanged".
        add_material(
            &store,
            "m1",
            "h2",
            "Welcome back! Classes begin September 8, 2026.",
        );
        let staleness = store.calendar_staleness(&accepted).unwrap();
        assert_eq!(
            (staleness.stale, staleness.changed_materials),
            (false, vec!["m1".to_string()])
        );
        // The words are gone: stale (the calendar stays in force).
        add_material(
            &store,
            "m1",
            "h3",
            "Welcome back! Classes begin September 9, 2026.",
        );
        assert!(store.calendar_staleness(&accepted).unwrap().stale);

        // The student's own dates carry no quotes and never go stale.
        let user = store
            .set_student_calendar(COURSE, Some(&accepted.calendar), at("2026-09-22"))
            .unwrap()
            .unwrap();
        assert_eq!(
            store.calendar_staleness(&user).unwrap(),
            Staleness::default()
        );
    }

    #[test]
    fn a_legacy_row_from_the_v4_migration_reads_back() {
        let store = demo_store();
        store
            .conn
            .execute(
                "INSERT INTO course_calendars
                     (course_id, origin, state, calendar_json, evidence_json, checks_json,
                      manifest_json, fingerprint, created_at, decided_at)
                 VALUES (?1, 'legacy', 'accepted', ?2, '[]', '{}', '[]', 'legacy', ?3, ?3)",
                params![
                    COURSE,
                    json(&legacy_calendar(date("2026-09-08"), None)),
                    ts_text(at("2026-09-01"))
                ],
            )
            .unwrap();
        let row = store.accepted_calendar(COURSE).unwrap().unwrap();
        assert_eq!(row.origin, CalendarOrigin::Legacy);
        assert_eq!(row.checks, CalendarChecks::default());
        assert!(row.provenance.is_none() && row.dates.is_empty());
    }
}
