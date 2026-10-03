//! Store methods of `generations` (schema 4; model-access design §5.4): one row per run of a
//! feature, with the validated answer and a summary of what was sent (no text). A row is
//! written once, when the run ends (the store is never held across a model call).
//!
//! Retention, per feature, course and week: the latest 5 accepted rows, the latest 5 drafts
//! and the latest 5 failed or cancelled runs, each counted apart, so a failure never deletes
//! a result in use or one waiting to be accepted. A row a calendar points to is always kept.

use rusqlite::{Row, params};

use super::{Store, TextValue, get_opt_value, get_value, opt_date_text, ts_text};
use crate::Result;
use crate::ai::AiFeature;
use crate::model::Timestamp;

/// Rows kept per feature, course and week in each of: accepted, drafts, failed or cancelled.
pub const KEEP_GENERATIONS: usize = 5;

/// How a run ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GenerationStatus {
    /// A result waiting for the student (a study-plan draft).
    Draft,
    /// A result in use (an explanation, a note, a calendar proposal).
    Accepted,
    Failed,
    Cancelled,
}

impl GenerationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            GenerationStatus::Draft => "draft",
            GenerationStatus::Accepted => "accepted",
            GenerationStatus::Failed => "failed",
            GenerationStatus::Cancelled => "cancelled",
        }
    }
}

impl TextValue for GenerationStatus {
    fn parse_text(text: &str) -> Option<Self> {
        [
            GenerationStatus::Draft,
            GenerationStatus::Accepted,
            GenerationStatus::Failed,
            GenerationStatus::Cancelled,
        ]
        .into_iter()
        .find(|status| status.as_str() == text)
    }
}

/// One run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationRecord {
    /// The caller's generation id.
    pub id: String,
    pub feature: AiFeature,
    pub course_id: Option<String>,
    pub week: Option<u32>,
    /// `codex`, `claude_code` or `provider:<id>`.
    pub backend: String,
    pub model: String,
    pub status: GenerationStatus,
    pub created_at: Timestamp,
    pub prompt_version: u32,
    /// The validated answer (JSON).
    pub output_json: Option<String>,
    /// What was sent, without text: the context summary and manifest (JSON).
    pub summary_json: Option<String>,
    /// A `ModelErrorKind` or `bad_output` code.
    pub error_kind: Option<String>,
    /// The Monday `week` started on when it was written (explanations): a calendar that moves
    /// the week makes the result stale.
    pub week_starts_on: Option<chrono::NaiveDate>,
}

const GENERATION_COLUMNS: &str = "id, feature, course_id, week, backend, model, status, \
     created_at, prompt_version, output_json, summary_json, error_kind, week_starts_on";

fn generation_from_row(row: &Row<'_>) -> rusqlite::Result<GenerationRecord> {
    Ok(GenerationRecord {
        id: row.get("id")?,
        feature: get_value(row, "feature")?,
        course_id: row.get("course_id")?,
        week: row.get("week")?,
        backend: row.get("backend")?,
        model: row.get("model")?,
        status: get_value(row, "status")?,
        created_at: get_value(row, "created_at")?,
        prompt_version: row.get("prompt_version")?,
        output_json: row.get("output_json")?,
        summary_json: row.get("summary_json")?,
        error_kind: row.get("error_kind")?,
        week_starts_on: get_opt_value(row, "week_starts_on")?,
    })
}

impl Store {
    /// Write a finished run (replacing a row with the same id), then keep the latest 5 accepted
    /// rows, the latest 5 drafts and the latest 5 failed or cancelled runs of its feature,
    /// course and week.
    pub fn record_generation(&self, record: &GenerationRecord) -> Result<()> {
        self.atomic(|| {
            self.conn.execute(
                "INSERT OR REPLACE INTO generations
                     (id, feature, course_id, week, backend, model, status, created_at,
                      prompt_version, output_json, summary_json, error_kind, week_starts_on)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    record.id,
                    record.feature.as_str(),
                    record.course_id,
                    record.week,
                    record.backend,
                    record.model,
                    record.status.as_str(),
                    ts_text(record.created_at),
                    record.prompt_version,
                    record.output_json,
                    record.summary_json,
                    record.error_kind,
                    opt_date_text(record.week_starts_on),
                ],
            )?;
            // Rows a calendar still points to are kept (course_calendars.generation_id).
            // Accepted rows, drafts and failures are counted apart: a failed or cancelled run
            // never pushes out a result in use or a draft waiting to be accepted.
            self.conn.execute(
                "DELETE FROM generations
                 WHERE feature = ?1 AND course_id IS ?2 AND week IS ?3
                   AND id NOT IN (SELECT generation_id FROM course_calendars
                                  WHERE generation_id IS NOT NULL)
                   AND id NOT IN (
                       SELECT id FROM generations
                       WHERE feature = ?1 AND course_id IS ?2 AND week IS ?3
                         AND status = 'accepted'
                       ORDER BY created_at DESC, rowid DESC LIMIT ?4)
                   AND id NOT IN (
                       SELECT id FROM generations
                       WHERE feature = ?1 AND course_id IS ?2 AND week IS ?3
                         AND status = 'draft'
                       ORDER BY created_at DESC, rowid DESC LIMIT ?4)
                   AND id NOT IN (
                       SELECT id FROM generations
                       WHERE feature = ?1 AND course_id IS ?2 AND week IS ?3
                         AND status NOT IN ('accepted', 'draft')
                       ORDER BY created_at DESC, rowid DESC LIMIT ?4)",
                params![
                    record.feature.as_str(),
                    record.course_id,
                    record.week,
                    KEEP_GENERATIONS as i64
                ],
            )?;
            Ok(())
        })
    }

    /// One run by id.
    pub fn generation(&self, id: &str) -> Result<Option<GenerationRecord>> {
        self.query_opt(
            &format!("SELECT {GENERATION_COLUMNS} FROM generations WHERE id = ?1"),
            [id],
            generation_from_row,
        )
    }

    /// Delete the runs of `course_id` (explanations, calendar readings, and the weekly notes
    /// that covered it) or, with `None`, every run (study plan drafts included); how many. A
    /// calendar that came from a run keeps its dates and its own label (`generation_id`
    /// becomes NULL); an accepted plan keeps its label.
    pub fn delete_generations(&self, course_id: Option<&str>) -> Result<u32> {
        self.atomic(|| {
            let mut removed = self.conn.execute(
                "DELETE FROM generations WHERE ?1 IS NULL OR course_id = ?1",
                [course_id],
            )?;
            if let Some(course_id) = course_id {
                // A note covers several courses (`course_id` NULL): its summary lists them.
                for note in self.course_free_generations(AiFeature::WeeklyNote, None)? {
                    if summary_lists_course(note.summary_json.as_deref(), course_id) {
                        removed += self
                            .conn
                            .execute("DELETE FROM generations WHERE id = ?1", [&note.id])?;
                    }
                }
            }
            self.conn.execute(
                "UPDATE study_plans SET generation_id = NULL
                 WHERE generation_id IS NOT NULL
                   AND generation_id NOT IN (SELECT id FROM generations)",
                [],
            )?;
            Ok(u32::try_from(removed).unwrap_or(u32::MAX))
        })
    }

    /// Delete run `id`; whether there was one.
    pub fn delete_generation(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM generations WHERE id = ?1", [id])?
            == 1)
    }

    /// How many kept runs failed, by error kind (`ModelErrorKind` codes), most first: counts
    /// only, for the diagnostic report.
    pub fn failed_generation_kinds(&self) -> Result<Vec<(String, u32)>> {
        self.query_list(
            "SELECT COALESCE(error_kind, 'unknown'), COUNT(*) FROM generations
             WHERE status = 'failed' GROUP BY 1 ORDER BY 2 DESC, 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }

    /// A feature's runs that belong to no single course (weekly notes), with `status` or
    /// (`None`) any, newest first.
    pub fn course_free_generations(
        &self,
        feature: AiFeature,
        status: Option<GenerationStatus>,
    ) -> Result<Vec<GenerationRecord>> {
        self.query_list(
            &format!(
                "SELECT {GENERATION_COLUMNS} FROM generations
                 WHERE feature = ?1 AND course_id IS NULL AND (?2 IS NULL OR status = ?2)
                 ORDER BY created_at DESC, rowid DESC"
            ),
            params![feature.as_str(), status.map(GenerationStatus::as_str)],
            generation_from_row,
        )
    }

    /// A feature's runs with `status` for a course, of one week or (`None`) all, newest first.
    pub fn generations_of(
        &self,
        feature: AiFeature,
        course_id: &str,
        week: Option<u32>,
        status: GenerationStatus,
    ) -> Result<Vec<GenerationRecord>> {
        self.query_list(
            &format!(
                "SELECT {GENERATION_COLUMNS} FROM generations
                 WHERE feature = ?1 AND course_id = ?2 AND (?3 IS NULL OR week = ?3)
                   AND status = ?4
                 ORDER BY created_at DESC, rowid DESC"
            ),
            params![feature.as_str(), course_id, week, status.as_str()],
            generation_from_row,
        )
    }
}

/// Whether a run's summary (`{"context": {"courses": [{"course_id": …}]}}`) lists
/// `course_id`. A summary that can't be read lists every course, so it is deleted too.
fn summary_lists_course(summary_json: Option<&str>, course_id: &str) -> bool {
    let Some(summary) =
        summary_json.and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
    else {
        return true;
    };
    match summary
        .pointer("/context/courses")
        .and_then(|courses| courses.as_array())
    {
        Some(courses) => courses
            .iter()
            .any(|course| course.get("course_id").and_then(|id| id.as_str()) == Some(course_id)),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use serde_json::json;

    use super::*;
    use crate::model::{CourseUpsert, SourceKind, SourceRecord};

    const COURSE: &str = "canvas:lms.example.edu/course/101";

    fn at(minutes: i64) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-24T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
            + Duration::minutes(minutes)
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

    fn run(id: &str, minutes: i64) -> GenerationRecord {
        GenerationRecord {
            id: id.into(),
            feature: AiFeature::CourseCalendar,
            course_id: Some(COURSE.into()),
            week: None,
            backend: "codex".into(),
            model: "gpt-6-luna".into(),
            status: GenerationStatus::Accepted,
            created_at: at(minutes),
            prompt_version: 1,
            output_json: Some("{}".into()),
            summary_json: Some(r#"{"materials_included":1}"#.into()),
            error_kind: None,
            week_starts_on: None,
        }
    }

    #[test]
    fn runs_are_recorded_and_the_latest_five_kept() {
        let store = demo_store();
        for i in 0..7 {
            store
                .record_generation(&run(&format!("gen-{i}"), i))
                .unwrap();
        }
        assert!(store.generation("gen-0").unwrap().is_none());
        assert!(store.generation("gen-1").unwrap().is_none());
        let latest = store.generation("gen-6").unwrap().unwrap();
        assert_eq!(latest, run("gen-6", 6));
        // A failed run carries its error and pushes no accepted row out.
        let mut failed = run("gen-7", 7);
        failed.status = GenerationStatus::Failed;
        failed.output_json = None;
        failed.error_kind = Some("bad_output".into());
        store.record_generation(&failed).unwrap();
        assert_eq!(store.generation("gen-7").unwrap().unwrap(), failed);
        assert!(store.generation("gen-2").unwrap().is_some());
        // Another course's runs are counted apart.
        let mut elsewhere = run("other", 8);
        elsewhere.course_id = None;
        store.record_generation(&elsewhere).unwrap();
        assert!(store.generation("gen-3").unwrap().is_some());
    }

    /// Failures never push results out: accepted rows, drafts and failures have 5 places each.
    #[test]
    fn failed_runs_never_push_accepted_results_out() {
        let store = demo_store();
        for i in 0..5 {
            store
                .record_generation(&run(&format!("ok-{i}"), i))
                .unwrap();
        }
        // A study plan draft on screen, waiting for Accept, then 6 failed regenerations.
        let draft = GenerationRecord {
            feature: AiFeature::StudyPlan,
            course_id: None,
            status: GenerationStatus::Draft,
            ..run("draft", 5)
        };
        store.record_generation(&draft).unwrap();
        let failed_plan = |id: &str, minutes| GenerationRecord {
            feature: AiFeature::StudyPlan,
            course_id: None,
            status: GenerationStatus::Failed,
            output_json: None,
            ..run(id, minutes)
        };
        for i in 0..6 {
            store
                .record_generation(&failed_plan(&format!("plan-failed-{i}"), 30 + i))
                .unwrap();
        }
        assert_eq!(store.generation("draft").unwrap(), Some(draft));
        assert!(store.generation("plan-failed-0").unwrap().is_none());
        let failed = |id: &str, minutes| GenerationRecord {
            status: GenerationStatus::Failed,
            output_json: None,
            error_kind: Some("timeout".into()),
            ..run(id, minutes)
        };
        for i in 0..6 {
            store
                .record_generation(&failed(&format!("failed-{i}"), 10 + i))
                .unwrap();
        }
        for i in 0..5 {
            assert!(
                store.generation(&format!("ok-{i}")).unwrap().is_some(),
                "ok-{i}"
            );
        }
        // The others keep their own latest 5.
        assert!(store.generation("failed-0").unwrap().is_none());
        assert!(store.generation("failed-5").unwrap().is_some());
        // A new result pushes the oldest result out, not a failure.
        store.record_generation(&run("ok-5", 20)).unwrap();
        assert!(store.generation("ok-0").unwrap().is_none());
        assert!(store.generation("failed-1").unwrap().is_some());
    }

    /// A note covers several courses: deleting one course's runs deletes the notes whose
    /// summary lists it (and any whose summary can't be read), not the others.
    #[test]
    fn a_course_s_runs_include_the_notes_that_covered_it() {
        let store = demo_store();
        let note = |id: &str, minutes, summary: Option<&str>| GenerationRecord {
            feature: AiFeature::WeeklyNote,
            course_id: None,
            week: None,
            summary_json: summary.map(str::to_string),
            ..run(id, minutes)
        };
        let covers = |ids: &[&str]| {
            json!({"context": {"courses": ids.iter().map(|id| json!({"course_id": id})).collect::<Vec<_>>()}})
                .to_string()
        };
        store
            .record_generation(&note("with", 1, Some(&covers(&[COURSE, "other"]))))
            .unwrap();
        store
            .record_generation(&note("without", 2, Some(&covers(&["other"]))))
            .unwrap();
        store
            .record_generation(&note("unreadable", 3, Some("{")))
            .unwrap();
        assert_eq!(
            store
                .course_free_generations(AiFeature::WeeklyNote, None)
                .unwrap()
                .len(),
            3
        );
        assert_eq!(store.delete_generations(Some(COURSE)).unwrap(), 2);
        let left: Vec<String> = store
            .course_free_generations(AiFeature::WeeklyNote, Some(GenerationStatus::Accepted))
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(left, ["without"]);
    }
}
