//! Store methods of `generations` (schema 4; model-access design §5.4): one row per run of a
//! feature, with the validated answer and a summary of what was sent (no text). A row is
//! written once, when the run ends (the store is never held across a model call).
//!
//! Retention: the latest 5 rows per feature, course and week.

use rusqlite::{Row, params};

use super::{Store, TextValue, get_value, ts_text};
use crate::Result;
use crate::ai::AiFeature;
use crate::model::Timestamp;

/// Rows kept per feature, course and week.
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
}

const GENERATION_COLUMNS: &str = "id, feature, course_id, week, backend, model, status, \
     created_at, prompt_version, output_json, summary_json, error_kind";

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
    })
}

impl Store {
    /// Write a finished run (replacing a row with the same id), then keep the latest 5 of its
    /// feature, course and week.
    pub fn record_generation(&self, record: &GenerationRecord) -> Result<()> {
        self.atomic(|| {
            self.conn.execute(
                "INSERT OR REPLACE INTO generations
                     (id, feature, course_id, week, backend, model, status, created_at,
                      prompt_version, output_json, summary_json, error_kind)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
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
                ],
            )?;
            // Rows a calendar still points to are kept (course_calendars.generation_id).
            self.conn.execute(
                "DELETE FROM generations
                 WHERE feature = ?1 AND course_id IS ?2 AND week IS ?3
                   AND id NOT IN (SELECT generation_id FROM course_calendars
                                  WHERE generation_id IS NOT NULL)
                   AND id NOT IN (
                       SELECT id FROM generations
                       WHERE feature = ?1 AND course_id IS ?2 AND week IS ?3
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
        // A failed run replaces nothing else and carries its error.
        let mut failed = run("gen-7", 7);
        failed.status = GenerationStatus::Failed;
        failed.output_json = None;
        failed.error_kind = Some("bad_output".into());
        store.record_generation(&failed).unwrap();
        assert_eq!(store.generation("gen-7").unwrap().unwrap(), failed);
        assert!(store.generation("gen-2").unwrap().is_none());
        // Another course's runs are counted apart.
        let mut elsewhere = run("other", 8);
        elsewhere.course_id = None;
        store.record_generation(&elsewhere).unwrap();
        assert!(store.generation("gen-3").unwrap().is_some());
    }
}
