//! Schema 4 (v0.3 M1): the migration from a real version-3 database, the AI tables and the
//! course lane's tables. Synthetic data only.

use std::path::{Path, PathBuf};

use chrono::{Duration, TimeZone, Utc};
use pagelamp_core::ai::{AiFeature, MaterialSharing, ProviderRow, UsageRecord};
use pagelamp_core::model::*;
use pagelamp_core::store::{
    COURSE_DATES_CONFIRMED, SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_VERSION, Store, backup_path,
};
use tempfile::TempDir;

fn temp_db() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pagelamp.db");
    (dir, path)
}

/// A version-3 database with four courses: one with confirmed dates, one with a 0.1
/// override, one with only an end override and one without overrides.
fn version_3_db(path: &Path) {
    let plain = rusqlite::Connection::open(path).unwrap();
    for schema in [SCHEMA_V1, SCHEMA_V2, SCHEMA_V3] {
        plain.execute_batch(schema).unwrap();
    }
    plain.pragma_update(None, "user_version", 3).unwrap();
    plain
        .execute(
            "INSERT INTO sources (id, kind, label) VALUES ('folder:demo', 'folder', 'Demo')",
            [],
        )
        .unwrap();
    for (id, start, end) in [
        (
            "folder:demo/course/CONFIRMED",
            Some("2026-09-08"),
            Some("2026-12-08"),
        ),
        (
            "folder:demo/course/LEGACY",
            Some("2026-09-08"),
            Some("2027-08-31"),
        ),
        ("folder:demo/course/ENDONLY", None, Some("2026-12-08")),
        ("folder:demo/course/NONE", None, None),
    ] {
        plain
            .execute(
                "INSERT INTO courses (id, source_id, external_id, code, name, user_term_start,
                                      user_term_end, updated_at)
                 VALUES (?1, 'folder:demo', ?1, ?1, 'Demo', ?2, ?3, '2026-09-20T00:00:00Z')",
                rusqlite::params![id, start, end],
            )
            .unwrap();
    }
    plain
        .execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, '2026-09-20T00:00:00Z')",
            rusqlite::params![
                COURSE_DATES_CONFIRMED,
                r#"["folder:demo/course/CONFIRMED"]"#
            ],
        )
        .unwrap();
}

#[test]
fn a_version_3_database_migrates_its_term_overrides_into_calendars() {
    let (_dir, path) = temp_db();
    version_3_db(&path);
    let store = Store::open(&path).unwrap();
    assert_eq!(SCHEMA_VERSION, 4);
    // Additive: a still-running v3 reader keeps working.
    assert_eq!(store.min_reader_version().unwrap(), Some(3));
    assert!(backup_path(&path, 3).exists(), "backed up before migrating");

    let rows: Vec<(String, String, String, String)> = {
        let mut statement = store
            .conn()
            .prepare(
                "SELECT course_id, origin, state, calendar_json FROM course_calendars
                 ORDER BY course_id",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(
        rows.len(),
        2,
        "only courses with a start override: {rows:?}"
    );
    let (confirmed, legacy) = (&rows[0], &rows[1]);
    assert_eq!(
        (
            confirmed.0.as_str(),
            confirmed.1.as_str(),
            confirmed.2.as_str()
        ),
        ("folder:demo/course/CONFIRMED", "user", "accepted")
    );
    assert!(
        confirmed.3.contains(r#""last_class":"2026-12-08""#),
        "{}",
        confirmed.3
    );
    assert_eq!(
        (legacy.0.as_str(), legacy.1.as_str()),
        ("folder:demo/course/LEGACY", "legacy")
    );
    // A year-long span is implausible for 0.1's form: the end is dropped, the start kept.
    assert!(
        legacy.3.contains(r#""first_class":"2026-09-08""#),
        "{}",
        legacy.3
    );
    assert!(legacy.3.contains(r#""last_class":null"#), "{}", legacy.3);
    // The overrides stay for v3 readers; the new question starts unanswered.
    let course = store
        .get_course("folder:demo/course/LEGACY")
        .unwrap()
        .unwrap();
    assert_eq!(course.term_source, TermSource::User);
    assert_eq!(course.material_sharing, MaterialSharing::Unanswered);
}

fn demo_store() -> Store {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_source(&SourceRecord {
            id: "folder:demo".into(),
            kind: SourceKind::Folder,
            label: "Demo".into(),
            config: serde_json::json!({ "path": "/demo/courses" }),
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })
        .unwrap();
    store
        .upsert_course(&CourseUpsert {
            id: "folder:demo/course/DEMO101".into(),
            source_id: "folder:demo".into(),
            external_id: "DEMO101".into(),
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

#[test]
fn the_material_sharing_answer_survives_sync() {
    let store = demo_store();
    let id = "folder:demo/course/DEMO101";
    store
        .set_course_material_sharing(id, MaterialSharing::NotAllowed)
        .unwrap();
    // A sync upserts the course again.
    store
        .upsert_course(&CourseUpsert {
            id: id.into(),
            source_id: "folder:demo".into(),
            external_id: "DEMO101".into(),
            code: Some("DEMO101".into()),
            name: "Renamed by the LMS".into(),
            term_start: None,
            term_end: None,
            url: None,
            syllabus_text: None,
            lms: Default::default(),
        })
        .unwrap();
    let course = store.get_course(id).unwrap().unwrap();
    assert_eq!(course.material_sharing, MaterialSharing::NotAllowed);
    assert!(
        store
            .set_course_material_sharing("nope", MaterialSharing::Allowed)
            .is_err()
    );
}

fn provider(id: &str) -> ProviderRow {
    ProviderRow {
        id: id.into(),
        preset: "openai".into(),
        label: "OpenAI".into(),
        wire: "openai_responses".into(),
        base_url: "https://api.openai.com/v1".into(),
        created_at: Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap(),
        last_probe_json: None,
    }
}

#[test]
fn providers_are_stored_without_keys() {
    let store = demo_store();
    store.insert_model_provider(&provider("openai")).unwrap();
    assert!(
        store.insert_model_provider(&provider("openai")).is_err(),
        "unique ids"
    );
    store
        .set_model_provider_probe("openai", Some(r#"{"ok":true}"#))
        .unwrap();
    let rows = store.model_providers().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].last_probe_json.as_deref(), Some(r#"{"ok":true}"#));
    assert_eq!(
        store.model_provider("openai").unwrap().unwrap().label,
        "OpenAI"
    );
    assert!(store.delete_model_provider("openai").unwrap());
    assert!(!store.delete_model_provider("openai").unwrap());
    assert!(store.set_model_provider_probe("openai", None).is_err());
}

fn usage(at: chrono::DateTime<Utc>, micro_usd: Option<u64>) -> UsageRecord {
    UsageRecord {
        at,
        backend: "provider:openai".into(),
        model: "gpt-6-luna".into(),
        feature: AiFeature::WeeklyExplanation,
        input_uncached: 1200,
        cache_read: 300,
        cache_write: 0,
        output: 400,
        reasoning: Some(100),
        micro_usd,
        cost_basis: if micro_usd.is_some() {
            "priced"
        } else {
            "unpriced"
        }
        .into(),
        estimated: false,
        outcome: "ok".into(),
    }
}

#[test]
fn the_usage_ledger_keeps_counts_and_cost_and_prunes_old_rows() {
    let store = demo_store();
    let now = Utc.with_ymd_and_hms(2026, 10, 15, 12, 0, 0).unwrap();
    store.record_ai_usage(&usage(now, Some(250))).unwrap();
    store
        .record_ai_usage(&usage(now - Duration::days(40), None))
        .unwrap();
    store
        .record_ai_usage(&usage(now - Duration::days(500), Some(10)))
        .unwrap();
    let month = store
        .ai_usage_between(now - Duration::days(30), now + Duration::days(1))
        .unwrap();
    assert_eq!(month, vec![usage(now, Some(250))]);
    assert_eq!(store.prune_ai_usage(now - Duration::days(400)).unwrap(), 1);
    assert_eq!(
        store
            .ai_usage_between(now - Duration::days(1000), now + Duration::days(1))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn removing_all_ai_data_empties_the_ai_tables_and_keeps_course_data() {
    let store = demo_store();
    store.insert_model_provider(&provider("openai")).unwrap();
    store.record_ai_usage(&usage(Utc::now(), Some(1))).unwrap();
    store
        .conn()
        .execute(
            "INSERT INTO generations (id, feature, course_id, backend, model, status, created_at,
                                      prompt_version)
             VALUES ('g1', 'weekly_explanation', 'folder:demo/course/DEMO101', 'provider:openai',
                     'gpt-6-luna', 'accepted', '2026-10-01T00:00:00Z', 1)",
            [],
        )
        .unwrap();
    // A plan PageLamp generated keeps its label (compliance item 4); only the run link goes.
    let label = r#"{"backend_label":"OpenAI","model":"gpt-6-luna","created_at":"2026-10-01T00:00:00Z","on_device":false}"#;
    store
        .conn()
        .execute(
            "INSERT INTO study_plans (created_at, plan_json, origin, generation_id, ai_label_json)
             VALUES ('2026-10-01T00:00:00Z', '{}', 'pagelamp', 'g1', ?1)",
            [label],
        )
        .unwrap();
    let removed = store.remove_all_ai_data().unwrap();
    assert_eq!(
        (removed.providers, removed.generations, removed.usage_rows),
        (1, 1, 1)
    );
    let (generation_id, kept): (Option<String>, Option<String>) = store
        .conn()
        .query_row(
            "SELECT generation_id, ai_label_json FROM study_plans",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((generation_id, kept.as_deref()), (None, Some(label)));
    assert!(store.model_providers().unwrap().is_empty());
    assert_eq!(
        store.list_courses(true).unwrap().len(),
        1,
        "course data stays"
    );
    assert_eq!(store.remove_all_ai_data().unwrap().providers, 0);
}
