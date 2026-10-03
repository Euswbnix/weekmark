//! Tests of every public `Store` method on an in-memory database.
//! All data is synthetic ("DEMO101 Intro to Demo Studies" and friends).

use std::panic::{AssertUnwindSafe, catch_unwind};

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};
use pagelamp_core::Error;
use pagelamp_core::model::*;
use pagelamp_core::store::{
    MAX_PLAN_ID_CHARS, MAX_PLAN_ITEMS, MAX_PLAN_MATERIAL_IDS, MAX_PLAN_TEXT_CHARS,
    MAX_PLAN_TITLE_CHARS, MAX_SEARCH_LIMIT, MAX_STORED_STUDY_PLANS, Store,
};
use serde_json::json;

// ----- fixtures -----------------------------------------------------------------------------

const SOURCE: &str = "canvas:lms.example.edu";
const FEED: &str = "ical:demo-feed";

fn ts(text: &str) -> Timestamp {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn source(id: &str, kind: SourceKind, label: &str) -> SourceRecord {
    SourceRecord {
        id: id.to_string(),
        kind,
        label: label.to_string(),
        config: json!({ "base_url": "https://lms.example.edu" }),
        last_synced_at: None,
        last_error: None,
        last_error_kind: None,
    }
}

fn course_id(external_id: &str) -> String {
    format!("{SOURCE}/course/{external_id}")
}

fn course(external_id: &str, code: Option<&str>, name: &str) -> CourseUpsert {
    CourseUpsert {
        id: course_id(external_id),
        source_id: SOURCE.to_string(),
        external_id: external_id.to_string(),
        code: code.map(str::to_string),
        name: name.to_string(),
        term_start: Some(date("2026-09-08")),
        term_end: Some(date("2026-12-18")),
        url: Some(format!("https://lms.example.edu/courses/{external_id}")),
        syllabus_text: None,
        lms: Default::default(),
    }
}

fn module(id: &str, course_id: &str, name: &str, position: Option<i64>) -> Module {
    Module {
        id: id.to_string(),
        course_id: course_id.to_string(),
        name: name.to_string(),
        position,
        unlock_at: Some(ts("2026-09-08T08:00:00Z")),
        week_hint: Some(1),
    }
}

fn material(id: &str, course_id: &str, title: &str) -> MaterialUpsert {
    MaterialUpsert {
        id: id.to_string(),
        course_id: course_id.to_string(),
        module_id: None,
        kind: MaterialKind::File,
        title: title.to_string(),
        url: Some(format!("https://lms.example.edu/files/{title}")),
        local_path: Some(format!("/demo/files/{title}.pdf")),
        mime: Some("application/pdf".to_string()),
        published_at: Some(ts("2026-09-10T09:00:00Z")),
        week_hint: Some(1),
    }
}

fn chunk(material_id: &str, ord: u32, text: &str) -> Chunk {
    Chunk {
        material_id: material_id.to_string(),
        ord,
        locator: Some(format!("p. {}", ord + 1)),
        text: text.to_string(),
    }
}

fn event(id: &str, source_id: &str, course_id: Option<&str>, title: &str) -> Event {
    Event {
        id: id.to_string(),
        source_id: source_id.to_string(),
        course_id: course_id.map(str::to_string),
        kind: EventKind::AssignmentDue,
        title: title.to_string(),
        starts_at: None,
        ends_at: None,
        due_at: None,
        url: None,
        updated_at: ts("2026-09-20T00:00:00Z"),
        course_hint: None,
    }
}

fn plan_item(title: &str) -> StudyPlanItem {
    StudyPlanItem {
        date: date("2026-09-28"),
        course_id: Some(course_id("101")),
        title: title.to_string(),
        description: Some("Re-read the demo lecture notes".to_string()),
        material_ids: vec!["demo-material".to_string()],
        minutes: Some(45),
        done: false,
    }
}

fn plan(items: Vec<StudyPlanItem>) -> StudyPlan {
    StudyPlan {
        horizon_start: date("2026-09-28"),
        horizon_end: date("2026-10-04"),
        items,
        notes: Some("Focus on week 3".to_string()),
    }
}

/// In-memory store with one Canvas source and the course DEMO101.
fn demo_store() -> Store {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_source(&source(SOURCE, SourceKind::Canvas, "Demo LMS"))
        .unwrap();
    store
        .upsert_course(&course("101", Some("DEMO101"), "Intro to Demo Studies"))
        .unwrap();
    store
}

/// Material `id` in DEMO101 with the given chunk texts.
fn add_material_with_chunks(store: &Store, id: &str, texts: &[&str]) {
    store
        .upsert_material(&material(id, &course_id("101"), id))
        .unwrap();
    let chunks: Vec<Chunk> = texts
        .iter()
        .enumerate()
        .map(|(ord, text)| chunk(id, ord as u32, text))
        .collect();
    readable(store, id);
    store.replace_chunks(id, &chunks).unwrap();
}

/// Text only belongs to an `ok` material (`replace_chunks` refuses it otherwise).
fn readable(store: &Store, id: &str) {
    store
        .set_text_state(id, TextStatus::Ok, None, None)
        .unwrap();
}

/// Number of rows in the FTS index itself (not the content table).
fn fts_rows(store: &Store) -> i64 {
    store
        .conn()
        .query_row("SELECT COUNT(*) FROM chunks_fts_docsize", [], |row| {
            row.get(0)
        })
        .unwrap()
}

/// FTS5 verifies that its index matches the `chunks` content table.
fn assert_fts_consistent(store: &Store) {
    // `rank = 1` makes FTS5 also compare the index with the external content table.
    store
        .conn()
        .execute(
            "INSERT INTO chunks_fts(chunks_fts, rank) VALUES ('integrity-check', 1)",
            [],
        )
        .expect("FTS index out of sync with chunks");
}

fn raw_text(store: &Store, sql: &str) -> String {
    store.conn().query_row(sql, [], |row| row.get(0)).unwrap()
}

/// "YYYY-MM-DDTHH:MM:SSZ"
fn assert_fixed_instant_format(text: &str) {
    assert_eq!(text.len(), 20, "{text}");
    assert_eq!(&text[10..11], "T", "{text}");
    assert!(text.ends_with('Z'), "{text}");
    assert!(DateTime::parse_from_rfc3339(text).is_ok(), "{text}");
}

fn ids<T>(items: &[T], id: impl Fn(&T) -> &str) -> Vec<String> {
    items.iter().map(|item| id(item).to_string()).collect()
}

// ----- sources ------------------------------------------------------------------------------

#[test]
fn upsert_and_get_source() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.get_source(SOURCE).unwrap().is_none());

    store
        .upsert_source(&source(SOURCE, SourceKind::Canvas, "Demo LMS"))
        .unwrap();
    let stored = store.get_source(SOURCE).unwrap().unwrap();
    assert_eq!(stored.id, SOURCE);
    assert_eq!(stored.kind, SourceKind::Canvas);
    assert_eq!(stored.label, "Demo LMS");
    assert_eq!(
        stored.config,
        json!({ "base_url": "https://lms.example.edu" })
    );
    assert_eq!(stored.last_synced_at, None);
    assert_eq!(stored.last_error, None);
    assert_eq!(stored.last_error_kind, None);
}

#[test]
fn upsert_source_updates_config_but_not_sync_state() {
    let store = demo_store();
    store
        .record_sync(SOURCE, ts("2026-09-25T12:00:00Z"), None)
        .unwrap();
    store
        .record_sync(
            SOURCE,
            ts("2026-09-25T13:00:00Z"),
            Some((SourceErrorKind::Network, "timed out")),
        )
        .unwrap();

    let mut updated = source(SOURCE, SourceKind::Canvas, "Renamed LMS");
    updated.config = json!({ "base_url": "https://lms2.example.edu" });
    store.upsert_source(&updated).unwrap();

    let stored = store.get_source(SOURCE).unwrap().unwrap();
    assert_eq!(stored.label, "Renamed LMS");
    assert_eq!(stored.config["base_url"], "https://lms2.example.edu");
    assert_eq!(stored.last_synced_at, Some(ts("2026-09-25T12:00:00Z")));
    assert_eq!(stored.last_error.as_deref(), Some("timed out"));
    assert_eq!(stored.last_error_kind, Some(SourceErrorKind::Network));
}

#[test]
fn list_sources_is_ordered_by_label() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.list_sources().unwrap().is_empty());
    store
        .upsert_source(&source("folder:b", SourceKind::Folder, "Zeta folder"))
        .unwrap();
    store
        .upsert_source(&source(FEED, SourceKind::Ical, "Alpha feed"))
        .unwrap();
    store
        .upsert_source(&source(SOURCE, SourceKind::Canvas, "Middle LMS"))
        .unwrap();
    let labels: Vec<String> = store
        .list_sources()
        .unwrap()
        .into_iter()
        .map(|s| s.label)
        .collect();
    assert_eq!(labels, ["Alpha feed", "Middle LMS", "Zeta folder"]);
}

#[test]
fn record_sync_success_and_failure() {
    let store = demo_store();
    let first = ts("2026-09-25T12:00:00Z");
    store.record_sync(SOURCE, first, None).unwrap();
    let stored = store.get_source(SOURCE).unwrap().unwrap();
    assert_eq!(stored.last_synced_at, Some(first));
    assert_fixed_instant_format(&raw_text(&store, "SELECT last_synced_at FROM sources"));

    // Failure: error recorded, last successful sync time untouched.
    store
        .record_sync(
            SOURCE,
            ts("2026-09-26T12:00:00Z"),
            Some((SourceErrorKind::AuthExpiredOrRevoked, "token expired")),
        )
        .unwrap();
    let stored = store.get_source(SOURCE).unwrap().unwrap();
    assert_eq!(stored.last_synced_at, Some(first));
    assert_eq!(stored.last_error.as_deref(), Some("token expired"));
    assert_eq!(
        stored.last_error_kind,
        Some(SourceErrorKind::AuthExpiredOrRevoked)
    );

    // Success again: new time, error cleared.
    let second = ts("2026-09-27T12:00:00Z");
    store.record_sync(SOURCE, second, None).unwrap();
    let stored = store.get_source(SOURCE).unwrap().unwrap();
    assert_eq!(stored.last_synced_at, Some(second));
    assert_eq!(stored.last_error, None);
    assert_eq!(stored.last_error_kind, None);
}

#[test]
fn clear_source_error_keeps_last_synced_at() {
    let store = demo_store();
    let synced = ts("2026-09-25T12:00:00Z");
    store.record_sync(SOURCE, synced, None).unwrap();
    store
        .record_sync(
            SOURCE,
            ts("2026-09-26T12:00:00Z"),
            Some((SourceErrorKind::RateLimited, "throttled")),
        )
        .unwrap();
    store.clear_source_error(SOURCE).unwrap();
    let stored = store.get_source(SOURCE).unwrap().unwrap();
    assert_eq!(stored.last_synced_at, Some(synced));
    assert_eq!(stored.last_error, None);
    assert_eq!(stored.last_error_kind, None);
}

#[test]
fn source_updates_of_unknown_id_are_not_found() {
    let store = demo_store();
    let at = ts("2026-09-25T12:00:00Z");
    assert!(matches!(
        store.record_sync("canvas:nowhere", at, None),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        store.clear_source_error("canvas:nowhere"),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        store.remove_source("canvas:nowhere"),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn remove_source_cascades_to_everything() {
    let store = demo_store();
    let course = course_id("101");
    store
        .replace_modules(&course, &[module("mod-1", &course, "Week 1", Some(1))])
        .unwrap();
    add_material_with_chunks(&store, "mat-1", &["demo gradient text", "more demo text"]);
    let mut due = event("ev-1", SOURCE, Some(&course), "Demo quiz");
    due.due_at = Some(ts("2026-10-01T10:00:00Z"));
    store.replace_events(SOURCE, &[due]).unwrap();
    // A second source whose data must survive.
    store
        .upsert_source(&source(FEED, SourceKind::Ical, "Demo feed"))
        .unwrap();
    let mut feed_event = event("ev-feed", FEED, None, "Demo holiday");
    feed_event.starts_at = Some(ts("2026-10-12T00:00:00Z"));
    store.replace_events(FEED, &[feed_event]).unwrap();
    store
        .save_study_plan(&plan(vec![plan_item("Revise")]))
        .unwrap();
    assert_eq!(fts_rows(&store), 2);

    store.remove_source(SOURCE).unwrap();

    assert!(store.get_source(SOURCE).unwrap().is_none());
    let counts = store.counts().unwrap();
    assert_eq!(
        counts,
        StoreCounts {
            courses: 0,
            hidden_courses: 0,
            modules: 0,
            materials: 0,
            indexed_materials: 0,
            chunks: 0,
            events: 1,
            study_plans: 1,
            removed_courses: 0,
        }
    );
    assert_eq!(fts_rows(&store), 0);
    assert_fts_consistent(&store);
    assert!(store.search("gradient", None, 10).unwrap().is_empty());
    assert!(store.get_source(FEED).unwrap().is_some());
}

// ----- courses ------------------------------------------------------------------------------

#[test]
fn upsert_and_get_course() {
    let store = demo_store();
    let before = Utc::now() - chrono::TimeDelta::seconds(1);
    let mut upsert = course("202", Some("DEMO202"), "Advanced Demo Studies");
    upsert.syllabus_text = Some("Demo syllabus".to_string());
    store.upsert_course(&upsert).unwrap();
    let after = Utc::now();

    let stored = store.get_course(&course_id("202")).unwrap().unwrap();
    assert_eq!(stored.id, course_id("202"));
    assert_eq!(stored.source_id, SOURCE);
    assert_eq!(stored.external_id, "202");
    assert_eq!(stored.code.as_deref(), Some("DEMO202"));
    assert_eq!(stored.name, "Advanced Demo Studies");
    assert_eq!(stored.term_start, Some(date("2026-09-08")));
    assert_eq!(stored.term_end, Some(date("2026-12-18")));
    assert_eq!(stored.url, upsert.url);
    assert_eq!(stored.ai_policy, AiPolicy::Unknown);
    assert_eq!(stored.ai_policy_note, None);
    assert!(!stored.hidden);
    assert!(stored.updated_at >= before && stored.updated_at <= after);

    assert!(store.get_course("no-such-course").unwrap().is_none());
    let raw_date = raw_text(
        &store,
        "SELECT term_start FROM courses WHERE external_id = '202'",
    );
    assert_eq!(raw_date, "2026-09-08");
    assert_fixed_instant_format(&raw_text(
        &store,
        "SELECT updated_at FROM courses WHERE external_id = '202'",
    ));
}

#[test]
fn reupserting_a_course_preserves_user_overrides() {
    let store = demo_store();
    let id = course_id("101");
    store
        .set_course_policy(&id, AiPolicy::LearningAid, Some("syllabus §4"))
        .unwrap();
    store
        .set_course_term(&id, Some(date("2026-09-01")), None)
        .unwrap();
    store.set_course_hidden(&id, true).unwrap();

    // A later sync brings new synced values.
    let mut resynced = course("101", Some("DEMO101"), "Intro to Demo Studies (Fall)");
    resynced.term_start = Some(date("2026-09-10"));
    resynced.term_end = Some(date("2026-12-20"));
    store.upsert_course(&resynced).unwrap();

    let stored = store.get_course(&id).unwrap().unwrap();
    assert_eq!(stored.name, "Intro to Demo Studies (Fall)");
    assert_eq!(stored.ai_policy, AiPolicy::LearningAid);
    assert_eq!(stored.ai_policy_note.as_deref(), Some("syllabus §4"));
    assert!(stored.hidden);
    // Effective term: override for the start, synced value for the end.
    assert_eq!(stored.term_start, Some(date("2026-09-01")));
    assert_eq!(stored.term_end, Some(date("2026-12-20")));

    // Clearing the override falls back to the (new) synced start.
    store.set_course_term(&id, None, None).unwrap();
    let stored = store.get_course(&id).unwrap().unwrap();
    assert_eq!(stored.term_start, Some(date("2026-09-10")));
}

#[test]
fn list_courses_orders_by_code_and_filters_hidden() {
    let store = demo_store();
    store
        .upsert_course(&course("303", Some("DEMO303"), "Demo Lab"))
        .unwrap();
    store
        .upsert_course(&course("050", Some("DEMO050"), "Demo Basics"))
        .unwrap();
    store.set_course_hidden(&course_id("303"), true).unwrap();

    let visible = store.list_courses(false).unwrap();
    assert_eq!(
        ids(&visible, |c| &c.id),
        [course_id("050"), course_id("101")]
    );
    let all = store.list_courses(true).unwrap();
    assert_eq!(
        ids(&all, |c| &c.id),
        [course_id("050"), course_id("101"), course_id("303")]
    );
}

#[test]
fn resolve_course_rules() {
    let store = demo_store();
    store
        .upsert_course(&course("202", Some("DEMO202H1"), "Advanced Demo Studies"))
        .unwrap();
    store
        .upsert_course(&course("900", None, "Seminar on Examples"))
        .unwrap();

    // Exact id.
    assert_eq!(
        store.resolve_course(&course_id("202")).unwrap().external_id,
        "202"
    );
    // Exact code: case-insensitive, spaces ignored, surrounding whitespace trimmed.
    assert_eq!(store.resolve_course("demo 101").unwrap().external_id, "101");
    assert_eq!(
        store.resolve_course("  DEMO101 ").unwrap().external_id,
        "101"
    );
    // Unique code prefix.
    assert_eq!(store.resolve_course("demo202").unwrap().external_id, "202");
    // Unique name substring.
    assert_eq!(store.resolve_course("seminar").unwrap().external_id, "900");
    assert_eq!(
        store.resolve_course("ADVANCED demo").unwrap().external_id,
        "202"
    );
}

#[test]
fn resolve_course_exact_code_beats_prefix() {
    let store = demo_store();
    store
        .upsert_course(&course("1011", Some("DEMO1011"), "Demo Extension"))
        .unwrap();
    // "DEMO101" is an exact code AND a prefix of "DEMO1011": the exact match wins.
    assert_eq!(store.resolve_course("DEMO101").unwrap().external_id, "101");
}

#[test]
fn resolve_course_ambiguous_and_not_found() {
    let store = demo_store();
    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();

    match store.resolve_course("demo") {
        Err(Error::Ambiguous { query, candidates }) => {
            assert_eq!(query, "demo");
            assert_eq!(candidates.len(), 2);
            assert!(candidates[0].contains("DEMO101"));
            assert!(candidates[0].contains(&course_id("101")));
            assert!(candidates[1].contains("DEMO202"));
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
    // Name substring shared by both courses.
    assert!(matches!(
        store.resolve_course("demo studies"),
        Err(Error::Ambiguous { .. })
    ));

    match store.resolve_course("CHEM999") {
        Err(Error::NotFound(message)) => {
            assert!(message.contains("CHEM999"), "{message}");
            assert!(message.contains("DEMO101"), "{message}");
            assert!(message.contains("DEMO202"), "{message}");
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
    assert!(matches!(
        store.resolve_course("   "),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn resolve_course_hidden_only_with_include_hidden() {
    let store = demo_store();
    store.set_course_hidden(&course_id("101"), true).unwrap();
    assert!(matches!(
        store.resolve_course("DEMO101"),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        store.resolve_course(&course_id("101")),
        Err(Error::NotFound(_))
    ));
    let hidden = store.resolve_course_with("DEMO101", true).unwrap();
    assert!(hidden.hidden);
}

#[test]
fn prune_courses_deletes_only_unlisted_courses_of_that_source() {
    let store = demo_store();
    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();
    add_material_with_chunks(&store, "mat-1", &["demo text"]);
    // A course of another source with an unlisted id must survive.
    store
        .upsert_source(&source("folder:demo", SourceKind::Folder, "Demo folder"))
        .unwrap();
    let mut other = course("777", Some("DEMO777"), "Folder Demo");
    other.id = "folder:demo/course/777".to_string();
    other.source_id = "folder:demo".to_string();
    store.upsert_course(&other).unwrap();

    let deleted = store.prune_courses(SOURCE, &[course_id("202")]).unwrap();
    assert_eq!(deleted, 1);
    let remaining = store.list_courses(true).unwrap();
    assert_eq!(
        ids(&remaining, |c| &c.id),
        [course_id("202"), "folder:demo/course/777".to_string()]
    );
    // The pruned course's material and chunks cascaded away.
    assert!(store.get_material("mat-1").unwrap().is_none());
    assert_eq!(fts_rows(&store), 0);

    // Empty keep list removes all courses of the source.
    assert_eq!(store.prune_courses(SOURCE, &[]).unwrap(), 1);
    assert_eq!(store.list_courses(true).unwrap().len(), 1);
}

#[test]
fn course_settings() {
    let store = demo_store();
    let id = course_id("101");

    store
        .set_course_policy(&id, AiPolicy::Prohibited, Some("no AI at all"))
        .unwrap();
    let stored = store.get_course(&id).unwrap().unwrap();
    assert_eq!(stored.ai_policy, AiPolicy::Prohibited);
    assert_eq!(stored.ai_policy_note.as_deref(), Some("no AI at all"));
    store
        .set_course_policy(&id, AiPolicy::AllowedWithCitation, None)
        .unwrap();
    let stored = store.get_course(&id).unwrap().unwrap();
    assert_eq!(stored.ai_policy, AiPolicy::AllowedWithCitation);
    assert_eq!(stored.ai_policy_note, None);

    store
        .set_course_term(&id, Some(date("2026-09-01")), Some(date("2026-12-01")))
        .unwrap();
    let stored = store.get_course(&id).unwrap().unwrap();
    assert_eq!(stored.term_start, Some(date("2026-09-01")));
    assert_eq!(stored.term_end, Some(date("2026-12-01")));
    assert!(matches!(
        store.set_course_term(&id, Some(date("2026-12-02")), Some(date("2026-12-01"))),
        Err(Error::Invalid(_))
    ));

    store.set_course_hidden(&id, true).unwrap();
    assert!(store.get_course(&id).unwrap().unwrap().hidden);
    store.set_course_hidden(&id, false).unwrap();
    assert!(!store.get_course(&id).unwrap().unwrap().hidden);

    for result in [
        store.set_course_policy("nope", AiPolicy::Unknown, None),
        store.set_course_term("nope", None, None),
        store.set_course_hidden("nope", true),
    ] {
        assert!(matches!(result, Err(Error::NotFound(_))));
    }
}

#[test]
fn course_syllabus_text() {
    let store = demo_store();
    assert_eq!(store.course_syllabus_text(&course_id("101")).unwrap(), None);
    let mut with_syllabus = course("101", Some("DEMO101"), "Intro to Demo Studies");
    with_syllabus.syllabus_text = Some("Demo syllabus: AI may be used to study.".to_string());
    store.upsert_course(&with_syllabus).unwrap();
    assert_eq!(
        store
            .course_syllabus_text(&course_id("101"))
            .unwrap()
            .as_deref(),
        Some("Demo syllabus: AI may be used to study.")
    );
    assert_eq!(store.course_syllabus_text("nope").unwrap(), None);
}

#[test]
fn unknown_stored_values_are_errors_not_panics() {
    let store = demo_store();
    store
        .conn()
        .execute("UPDATE courses SET ai_policy = 'whatever'", [])
        .unwrap();
    let err = store.list_courses(true).unwrap_err();
    assert!(err.to_string().contains("whatever"), "{err}");

    let store = demo_store();
    store
        .conn()
        .execute("UPDATE courses SET updated_at = 'yesterday-ish'", [])
        .unwrap();
    assert!(matches!(
        store.get_course(&course_id("101")),
        Err(Error::Db(_))
    ));

    let store = demo_store();
    store
        .conn()
        .execute("UPDATE sources SET last_error_kind = 'mystery'", [])
        .unwrap();
    assert!(store.list_sources().is_err());
}

// ----- modules ------------------------------------------------------------------------------

#[test]
fn replace_modules_and_list_order() {
    let store = demo_store();
    let course = course_id("101");
    let mut unlocked = module("mod-c", &course, "Resources", None);
    unlocked.unlock_at = None;
    unlocked.week_hint = None;
    store
        .replace_modules(
            &course,
            &[
                module("mod-b", &course, "Week 2", Some(2)),
                unlocked,
                module("mod-a", &course, "Week 1", Some(1)),
            ],
        )
        .unwrap();
    let modules = store.list_modules(&course).unwrap();
    assert_eq!(ids(&modules, |m| &m.id), ["mod-a", "mod-b", "mod-c"]);
    assert_eq!(modules[0].unlock_at, Some(ts("2026-09-08T08:00:00Z")));
    assert_eq!(modules[0].week_hint, Some(1));
    assert_eq!(modules[2].position, None);
    assert_eq!(modules[2].unlock_at, None);
    assert!(store.list_modules("nope").unwrap().is_empty());
}

#[test]
fn replace_modules_unlinks_only_materials_of_removed_modules() {
    let store = demo_store();
    let course = course_id("101");
    store
        .replace_modules(
            &course,
            &[
                module("mod-1", &course, "Week 1", Some(1)),
                module("mod-2", &course, "Week 2", Some(2)),
            ],
        )
        .unwrap();
    let mut first = material("mat-1", &course, "Slides 1");
    first.module_id = Some("mod-1".to_string());
    let mut second = material("mat-2", &course, "Slides 2");
    second.module_id = Some("mod-2".to_string());
    store.upsert_material(&first).unwrap();
    store.upsert_material(&second).unwrap();

    // Next sync: module 2 is gone, module 1 renamed.
    store
        .replace_modules(
            &course,
            &[module("mod-1", &course, "Week 1 (updated)", Some(1))],
        )
        .unwrap();

    let modules = store.list_modules(&course).unwrap();
    assert_eq!(ids(&modules, |m| &m.id), ["mod-1"]);
    assert_eq!(modules[0].name, "Week 1 (updated)");
    let first = store.get_material("mat-1").unwrap().unwrap();
    assert_eq!(first.module_id.as_deref(), Some("mod-1"));
    let second = store.get_material("mat-2").unwrap().unwrap();
    assert_eq!(second.module_id, None);
}

#[test]
fn replace_modules_rejects_module_of_other_course() {
    let store = demo_store();
    let course = course_id("101");
    store
        .replace_modules(&course, &[module("mod-1", &course, "Week 1", Some(1))])
        .unwrap();
    let foreign = module("mod-x", "some/other/course", "Week 9", Some(9));
    assert!(matches!(
        store.replace_modules(&course, &[foreign]),
        Err(Error::Invalid(_))
    ));
    assert_eq!(store.list_modules(&course).unwrap().len(), 1);
}

#[test]
fn replace_modules_only_touches_the_given_course() {
    let store = demo_store();
    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();
    let (a, b) = (course_id("101"), course_id("202"));
    store
        .replace_modules(&b, &[module("mod-b1", &b, "Unit 1", Some(1))])
        .unwrap();
    store
        .replace_modules(&a, &[module("mod-a1", &a, "Week 1", Some(1))])
        .unwrap();
    store.replace_modules(&a, &[]).unwrap();

    assert!(store.list_modules(&a).unwrap().is_empty());
    assert_eq!(ids(&store.list_modules(&b).unwrap(), |m| &m.id), ["mod-b1"]);
}

// ----- materials & chunks -------------------------------------------------------------------

#[test]
fn upsert_material_inserts_pending() {
    let store = demo_store();
    let course = course_id("101");
    let upsert = material("mat-1", &course, "Lecture 1");
    store.upsert_material(&upsert).unwrap();

    let stored = store.get_material("mat-1").unwrap().unwrap();
    assert_eq!(stored.course_id, course);
    assert_eq!(stored.module_id, None);
    assert_eq!(stored.kind, MaterialKind::File);
    assert_eq!(stored.title, "Lecture 1");
    assert_eq!(stored.url, upsert.url);
    assert_eq!(stored.local_path, upsert.local_path);
    assert_eq!(stored.mime.as_deref(), Some("application/pdf"));
    assert_eq!(stored.published_at, Some(ts("2026-09-10T09:00:00Z")));
    assert_eq!(stored.week_hint, Some(1));
    assert_eq!(stored.content_hash, None);
    assert_eq!(stored.text_status, TextStatus::Pending);
    assert_eq!(stored.text_error, None);
    assert!(store.get_material("nope").unwrap().is_none());
}

#[test]
fn upsert_material_preserves_index_state_unless_local_path_changes() {
    let store = demo_store();
    let course = course_id("101");
    let mut upsert = material("mat-1", &course, "Lecture 1");
    store.upsert_material(&upsert).unwrap();
    store
        .set_text_state(
            "mat-1",
            TextStatus::Error,
            Some("demo failure"),
            Some("hash-1"),
        )
        .unwrap();

    // Same local_path: synced columns change, index state is kept.
    upsert.title = "Lecture 1 (revised)".to_string();
    upsert.kind = MaterialKind::Page;
    store.upsert_material(&upsert).unwrap();
    let stored = store.get_material("mat-1").unwrap().unwrap();
    assert_eq!(stored.title, "Lecture 1 (revised)");
    assert_eq!(stored.kind, MaterialKind::Page);
    assert_eq!(stored.text_status, TextStatus::Error);
    assert_eq!(stored.text_error.as_deref(), Some("demo failure"));
    assert_eq!(stored.content_hash.as_deref(), Some("hash-1"));

    // New local_path: back to pending (hash kept so ingest can still compare).
    upsert.local_path = Some("/demo/files/lecture-1-v2.pdf".to_string());
    store.upsert_material(&upsert).unwrap();
    let stored = store.get_material("mat-1").unwrap().unwrap();
    assert_eq!(stored.local_path, upsert.local_path);
    assert_eq!(stored.text_status, TextStatus::Pending);
    assert_eq!(stored.text_error, None);
    assert_eq!(stored.content_hash.as_deref(), Some("hash-1"));

    // local_path from Some to None also counts as a change.
    store
        .set_text_state("mat-1", TextStatus::Ok, None, None)
        .unwrap();
    upsert.local_path = None;
    store.upsert_material(&upsert).unwrap();
    let stored = store.get_material("mat-1").unwrap().unwrap();
    assert_eq!(stored.text_status, TextStatus::Pending);
}

#[test]
fn list_materials_order() {
    let store = demo_store();
    let course = course_id("101");
    let mut no_week = material("m-noweek", &course, "Aaa no week");
    no_week.week_hint = None;
    let mut week2 = material("m-week2", &course, "Week 2 notes");
    week2.week_hint = Some(2);
    let mut week1_late = material("m-week1-late", &course, "Aaa late");
    week1_late.published_at = Some(ts("2026-09-12T09:00:00Z"));
    let mut week1_b = material("m-week1-b", &course, "Bbb early");
    week1_b.published_at = Some(ts("2026-09-10T09:00:00Z"));
    let mut week1_a = material("m-week1-a", &course, "Aaa early");
    week1_a.published_at = Some(ts("2026-09-10T09:00:00Z"));
    for m in [&no_week, &week2, &week1_late, &week1_b, &week1_a] {
        store.upsert_material(m).unwrap();
    }
    let listed = store.list_materials(&course).unwrap();
    assert_eq!(
        ids(&listed, |m| &m.id),
        [
            "m-week1-a",
            "m-week1-b",
            "m-week1-late",
            "m-week2",
            "m-noweek"
        ]
    );
    assert!(store.list_materials("nope").unwrap().is_empty());
}

#[test]
fn prune_materials_deletes_unlisted_and_their_chunks() {
    let store = demo_store();
    add_material_with_chunks(&store, "keep-me", &["alpha demo"]);
    add_material_with_chunks(&store, "drop-me", &["beta demo", "gamma demo"]);
    assert_eq!(fts_rows(&store), 3);

    let deleted = store
        .prune_materials(&course_id("101"), &["keep-me".to_string()])
        .unwrap();
    assert_eq!(deleted, 1);
    assert!(store.get_material("drop-me").unwrap().is_none());
    assert!(store.get_material("keep-me").unwrap().is_some());
    assert_eq!(fts_rows(&store), 1);
    assert_fts_consistent(&store);
    assert!(store.search("beta", None, 10).unwrap().is_empty());
    assert_eq!(store.search("alpha", None, 10).unwrap().len(), 1);
}

#[test]
fn prune_materials_only_touches_the_given_course() {
    let store = demo_store();
    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();
    add_material_with_chunks(&store, "demo101-mat", &["alpha demo"]);
    let other = course_id("202");
    store
        .upsert_material(&material("demo202-mat", &other, "Other slides"))
        .unwrap();
    readable(&store, "demo202-mat");
    store
        .replace_chunks("demo202-mat", &[chunk("demo202-mat", 0, "omega demo")])
        .unwrap();

    // DEMO101 keeps nothing; DEMO202's material is not in `keep` but must survive.
    assert_eq!(store.prune_materials(&course_id("101"), &[]).unwrap(), 1);
    assert!(store.get_material("demo101-mat").unwrap().is_none());
    assert!(store.get_material("demo202-mat").unwrap().is_some());
    assert_eq!(store.chunk_count("demo202-mat").unwrap(), 1);
    assert_eq!(fts_rows(&store), 1);
    assert_fts_consistent(&store);
    assert_eq!(store.search("omega", None, 10).unwrap().len(), 1);
}

#[test]
fn set_text_state_updates_and_keeps_hash_when_none() {
    let store = demo_store();
    store
        .upsert_material(&material("mat-1", &course_id("101"), "Lecture 1"))
        .unwrap();
    store
        .set_text_state("mat-1", TextStatus::Ok, None, Some("hash-1"))
        .unwrap();
    store
        .set_text_state("mat-1", TextStatus::Unsupported, Some("video"), None)
        .unwrap();
    let stored = store.get_material("mat-1").unwrap().unwrap();
    assert_eq!(stored.text_status, TextStatus::Unsupported);
    assert_eq!(stored.text_error.as_deref(), Some("video"));
    assert_eq!(stored.content_hash.as_deref(), Some("hash-1"));
    assert!(matches!(
        store.set_text_state("nope", TextStatus::Ok, None, None),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn replace_chunks_get_chunks_and_count() {
    let store = demo_store();
    add_material_with_chunks(&store, "mat-1", &["zero", "one", "two", "three"]);
    assert_eq!(store.chunk_count("mat-1").unwrap(), 4);
    assert_eq!(store.chunk_count("nope").unwrap(), 0);

    let all = store.get_chunks("mat-1", 0, None).unwrap();
    assert_eq!(
        all.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
        ["zero", "one", "two", "three"]
    );
    assert_eq!(all[2].ord, 2);
    assert_eq!(all[2].locator.as_deref(), Some("p. 3"));
    assert_eq!(all[2].material_id, "mat-1");

    let window = store.get_chunks("mat-1", 1, Some(2)).unwrap();
    assert_eq!(window.iter().map(|c| c.ord).collect::<Vec<_>>(), [1, 2]);
    let tail = store.get_chunks("mat-1", 3, Some(10)).unwrap();
    assert_eq!(tail.len(), 1);
    assert!(store.get_chunks("mat-1", 4, None).unwrap().is_empty());
    assert!(store.get_chunks("mat-1", 0, Some(0)).unwrap().is_empty());

    // Replacing swaps the text and the FTS index.
    store
        .replace_chunks("mat-1", &[chunk("mat-1", 0, "replacement demo")])
        .unwrap();
    assert_eq!(store.chunk_count("mat-1").unwrap(), 1);
    assert_eq!(fts_rows(&store), 1);
    assert_fts_consistent(&store);
    assert!(store.search("three", None, 10).unwrap().is_empty());
    assert_eq!(store.search("replacement", None, 10).unwrap().len(), 1);

    // Replacing with nothing clears.
    store.replace_chunks("mat-1", &[]).unwrap();
    assert_eq!(store.chunk_count("mat-1").unwrap(), 0);
    assert_eq!(fts_rows(&store), 0);
}

#[test]
fn replace_chunks_is_all_or_nothing() {
    let store = demo_store();
    add_material_with_chunks(&store, "mat-1", &["original demo text"]);

    // Duplicate ord violates UNIQUE(material_id, ord) half-way through.
    let bad = [
        chunk("mat-1", 0, "new zero"),
        chunk("mat-1", 0, "duplicate zero"),
    ];
    assert!(matches!(
        store.replace_chunks("mat-1", &bad),
        Err(Error::Db(_))
    ));
    let chunks = store.get_chunks("mat-1", 0, None).unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].text, "original demo text");
    assert_fts_consistent(&store);

    // Chunks of another material are rejected up front.
    assert!(matches!(
        store.replace_chunks("mat-1", &[chunk("mat-2", 0, "x")]),
        Err(Error::Invalid(_))
    ));
}

// ----- events -------------------------------------------------------------------------------

#[test]
fn replace_events_upserts_and_deletes_missing() {
    let store = demo_store();
    store
        .upsert_source(&source(FEED, SourceKind::Ical, "Demo feed"))
        .unwrap();
    let course = course_id("101");
    let mut quiz = event("ev-quiz", SOURCE, Some(&course), "Demo quiz");
    quiz.due_at = Some(ts("2026-10-01T10:00:00Z"));
    let mut essay = event("ev-essay", SOURCE, Some(&course), "Demo essay");
    essay.due_at = Some(ts("2026-10-05T10:00:00Z"));
    let mut holiday = event("ev-holiday", FEED, None, "Demo holiday");
    holiday.starts_at = Some(ts("2026-10-12T00:00:00Z"));
    store
        .replace_events(SOURCE, &[quiz, essay.clone()])
        .unwrap();
    store.replace_events(FEED, &[holiday]).unwrap();

    // Next sync of SOURCE: quiz gone, essay moved.
    essay.title = "Demo essay (extended)".to_string();
    essay.due_at = Some(ts("2026-10-08T10:00:00Z"));
    essay.kind = EventKind::Exam;
    store.replace_events(SOURCE, &[essay]).unwrap();

    let events = store
        .list_events(ts("2026-01-01T00:00:00Z"), ts("2027-01-01T00:00:00Z"), None)
        .unwrap();
    assert_eq!(ids(&events, |e| &e.id), ["ev-essay", "ev-holiday"]);
    assert_eq!(events[0].title, "Demo essay (extended)");
    assert_eq!(events[0].kind, EventKind::Exam);
    assert_eq!(events[0].due_at, Some(ts("2026-10-08T10:00:00Z")));
    assert_eq!(events[0].updated_at, ts("2026-09-20T00:00:00Z"));
    assert_eq!(events[1].source_id, FEED);

    // Events of another source are rejected up front.
    assert!(matches!(
        store.replace_events(SOURCE, &[quiz_for(FEED)]),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn relink_events_links_hinted_events_to_courses_created_later() {
    let store = demo_store();
    store
        .upsert_source(&source(FEED, SourceKind::Ical, "Demo feed"))
        .unwrap();
    let hinted = |id: &str, hint: Option<&str>| {
        let mut event = event(id, FEED, None, id);
        event.due_at = Some(ts("2026-10-01T10:00:00Z"));
        event.course_hint = hint.map(str::to_string);
        event
    };
    store
        .replace_events(
            FEED,
            &[
                hinted("ev-202", Some("demo 202 F LEC0101")),
                hinted("ev-other", Some("OTHER999")),
                hinted("ev-plain", None),
            ],
        )
        .unwrap();
    assert_eq!(
        store.relink_events().unwrap(),
        0,
        "DEMO202 does not exist yet"
    );

    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();
    assert_eq!(store.relink_events().unwrap(), 1);
    assert_eq!(store.relink_events().unwrap(), 0, "nothing left to link");
    let events = store
        .list_events(ts("2026-01-01T00:00:00Z"), ts("2027-01-01T00:00:00Z"), None)
        .unwrap();
    let by_id = |id: &str| events.iter().find(|e| e.id == id).unwrap();
    assert_eq!(by_id("ev-202").course_id, Some(course_id("202")));
    assert_eq!(
        by_id("ev-202").course_hint.as_deref(),
        Some("demo 202 F LEC0101")
    );
    assert_eq!(by_id("ev-other").course_id, None);
    assert_eq!(by_id("ev-plain").course_id, None);
}

#[test]
fn lms_course_facts_follow_each_sync_while_student_answers_survive_it() {
    let store = demo_store();
    let id = course_id("101");
    assert_eq!(
        store.course_term_data(&id).unwrap(),
        Some(CourseTermData {
            synced_term_start: Some(date("2026-09-08")),
            synced_term_end: Some(date("2026-12-18")),
            ..CourseTermData::default()
        }),
        "no LMS facts or student answers yet"
    );
    let lms = LmsCourseInfo {
        term_name: Some("Fall 2026".into()),
        term_start: Some(date("2026-05-01")),
        term_end: Some(date("2026-12-31")),
        course_start: Some(date("2026-09-08")),
        course_end: Some(date("2026-12-18")),
        time_zone: Some("America/Toronto".into()),
        concluded: Some(false),
        workflow_state: Some("available".into()),
        access_restricted: None,
    };
    let mut upsert = course("101", Some("DEMO101"), "Intro to Demo Studies");
    upsert.lms = lms.clone();
    store.upsert_course(&upsert).unwrap();
    store
        .set_keep_current_until(&id, Some(date("2027-01-15")))
        .unwrap();
    store
        .set_removal_snoozed_until(&id, Some(date("9999-12-31")))
        .unwrap();
    let data = store.course_term_data(&id).unwrap().unwrap();
    assert_eq!(data.lms, lms);
    assert_eq!(data.keep_current_until, Some(date("2027-01-15")));
    assert_eq!(data.removal_snoozed_until, Some(date("9999-12-31")));

    // The next sync states the LMS facts anew; the student's answers stay.
    upsert.lms = LmsCourseInfo {
        concluded: Some(true),
        ..LmsCourseInfo::default()
    };
    store.upsert_course(&upsert).unwrap();
    store.set_course_access_restricted(&id, true).unwrap();
    let data = store.course_term_data(&id).unwrap().unwrap();
    assert_eq!(data.lms.concluded, Some(true));
    assert_eq!(data.lms.term_name, None);
    assert_eq!(data.lms.access_restricted, Some(true));
    assert_eq!(data.keep_current_until, Some(date("2027-01-15")));
    assert_eq!(data.removal_snoozed_until, Some(date("9999-12-31")));

    // The student's term overrides are read raw, apart from the synced dates.
    store
        .set_course_term(&id, None, Some(date("2026-12-08")))
        .unwrap();
    let data = store.course_term_data(&id).unwrap().unwrap();
    assert_eq!(
        (data.user_term_start, data.user_term_end),
        (None, Some(date("2026-12-08")))
    );
    assert_eq!(
        (data.synced_term_start, data.synced_term_end),
        (Some(date("2026-09-08")), Some(date("2026-12-18")))
    );

    // Clearing, the bulk read, and unknown ids.
    store.set_keep_current_until(&id, None).unwrap();
    let all = store.all_course_term_data().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[&id].keep_current_until, None);
    assert_eq!(store.course_term_data("nope").unwrap(), None);
    assert!(matches!(
        store.set_removal_snoozed_until("nope", None),
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        store.set_course_access_restricted("nope", true),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn calendar_events_go_to_the_visible_course_when_a_hidden_one_has_the_same_code() {
    let store = demo_store();
    store
        .upsert_source(&source(FEED, SourceKind::Ical, "Demo feed"))
        .unwrap();
    // Last year's DEMO101 (hidden) sorts after this year's; before, the last one won.
    store
        .upsert_course(&course(
            "999",
            Some("DEMO101"),
            "Intro to Demo Studies (old)",
        ))
        .unwrap();
    store.set_course_hidden(&course_id("999"), true).unwrap();
    let courses = store.list_courses(true).unwrap();
    assert_eq!(
        course_for_hint("DEMO101 F LEC0101", &courses).map(|c| c.id.as_str()),
        Some(course_id("101").as_str())
    );

    let mut quiz = event("ev-quiz", FEED, None, "Quiz 1");
    quiz.due_at = Some(ts("2026-10-01T10:00:00Z"));
    quiz.course_hint = Some("DEMO101 F LEC0101".into());
    store.replace_events(FEED, &[quiz]).unwrap();
    assert_eq!(store.relink_events().unwrap(), 1);
    let events = store
        .list_events(ts("2026-01-01T00:00:00Z"), ts("2027-01-01T00:00:00Z"), None)
        .unwrap();
    assert_eq!(events[0].course_id, Some(course_id("101")));
}

fn quiz_for(source_id: &str) -> Event {
    let mut quiz = event("ev-x", source_id, None, "Demo");
    quiz.due_at = Some(ts("2026-10-01T10:00:00Z"));
    quiz
}

#[test]
fn list_events_range_order_and_filter() {
    let store = demo_store();
    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();
    let a = course_id("101");
    let b = course_id("202");

    let mut on_start = event("e-start", SOURCE, Some(&a), "B due at range start");
    on_start.due_at = Some(ts("2026-10-01T10:00:00Z"));
    let mut same_time = event("e-same", SOURCE, Some(&b), "A same instant");
    // Different offset in, same instant: stored as UTC.
    same_time.due_at = Some(ts("2026-10-01T12:00:00+02:00"));
    let mut by_start = event("e-lecture", SOURCE, Some(&a), "Lecture");
    by_start.kind = EventKind::ClassEvent;
    by_start.starts_at = Some(ts("2026-10-02T09:00:00Z"));
    by_start.ends_at = Some(ts("2026-10-02T11:00:00Z"));
    let mut due_wins = event("e-due-wins", SOURCE, Some(&a), "Due wins");
    due_wins.starts_at = Some(ts("2026-09-01T00:00:00Z"));
    due_wins.due_at = Some(ts("2026-10-02T12:00:00Z"));
    let mut due_outside = event("e-due-outside", SOURCE, Some(&a), "Starts inside");
    due_outside.starts_at = Some(ts("2026-10-02T00:00:00Z"));
    due_outside.due_at = Some(ts("2026-11-01T00:00:00Z"));
    let undated = event("e-undated", SOURCE, Some(&a), "No date");
    let mut on_end = event("e-end", SOURCE, None, "At range end");
    on_end.due_at = Some(ts("2026-10-03T00:00:00Z"));
    store
        .replace_events(
            SOURCE,
            &[
                on_start,
                same_time,
                by_start,
                due_wins,
                due_outside,
                undated,
                on_end,
            ],
        )
        .unwrap();

    let from = ts("2026-10-01T10:00:00Z");
    let to = ts("2026-10-03T00:00:00Z");
    let all = store.list_events(from, to, None).unwrap();
    assert_eq!(
        ids(&all, |e| &e.id),
        ["e-same", "e-start", "e-lecture", "e-due-wins", "e-end"]
    );
    assert_eq!(all[2].ends_at, Some(ts("2026-10-02T11:00:00Z")));
    assert_eq!(all[2].kind, EventKind::ClassEvent);

    let only_a = store.list_events(from, to, Some(&a)).unwrap();
    assert_eq!(
        ids(&only_a, |e| &e.id),
        ["e-start", "e-lecture", "e-due-wins"]
    );
    assert!(store.list_events(to, from, None).unwrap().is_empty());
}

#[test]
fn list_events_accepts_open_ended_ranges() {
    // Regression: bounds past year 9999 used to be written as "+10000-…", which sorts before
    // every digit, so `list_events(now, DateTime::MAX_UTC)` silently returned nothing.
    let store = demo_store();
    let mut due = event("ev-1", SOURCE, Some(&course_id("101")), "Demo quiz");
    due.due_at = Some(ts("2026-09-28T12:00:00Z"));
    store.replace_events(SOURCE, &[due]).unwrap();

    let now = ts("2026-09-25T12:00:00Z");
    let (min, max) = (DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC);
    let listed = |from, to| ids(&store.list_events(from, to, None).unwrap(), |e| &e.id);
    assert_eq!(listed(min, max), ["ev-1"]);
    assert_eq!(listed(now, max), ["ev-1"]);
    assert_eq!(listed(now, now + TimeDelta::days(3_000_000)), ["ev-1"]);
    assert!(listed(min, now).is_empty());
}

#[test]
fn instants_outside_years_0000_to_9999_are_clamped_and_stay_readable() {
    // Regression: such instants used to be written in a form the reader could not parse,
    // which broke every later read of the course's modules/materials.
    let store = demo_store();
    let course = course_id("101");
    // A "no end" sentinel in a UTC-8 feed: 9999-12-31 23:00 local = year 10000 in UTC.
    let sentinel = ts("9999-12-31T23:00:00-08:00");
    let latest = ts("9999-12-31T23:59:59Z");
    let earliest = ts("0000-01-01T00:00:00Z");

    let mut week = module("mod-1", &course, "Week 1", Some(1));
    week.unlock_at = Some(sentinel);
    store.replace_modules(&course, &[week]).unwrap();
    assert_eq!(
        store.list_modules(&course).unwrap()[0].unlock_at,
        Some(latest)
    );

    let mut slides = material("mat-1", &course, "Slides");
    slides.published_at = Some(DateTime::<Utc>::MAX_UTC);
    store.upsert_material(&slides).unwrap();
    let stored = store.get_material("mat-1").unwrap().unwrap();
    assert_eq!(stored.published_at, Some(latest));
    assert_eq!(store.list_materials(&course).unwrap().len(), 1);

    let mut forever = event("ev-1", SOURCE, Some(&course), "Demo term");
    forever.starts_at = Some(DateTime::<Utc>::MIN_UTC);
    forever.ends_at = Some(sentinel);
    store.replace_events(SOURCE, &[forever]).unwrap();
    let events = store
        .list_events(DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC, None)
        .unwrap();
    assert_eq!(events[0].starts_at, Some(earliest));
    assert_eq!(events[0].ends_at, Some(latest));

    store
        .record_sync(SOURCE, DateTime::<Utc>::MAX_UTC, None)
        .unwrap();
    let synced = store.get_source(SOURCE).unwrap().unwrap();
    assert_eq!(synced.last_synced_at, Some(latest));

    for sql in [
        "SELECT unlock_at FROM modules",
        "SELECT published_at FROM materials",
        "SELECT starts_at FROM events",
        "SELECT ends_at FROM events",
        "SELECT last_synced_at FROM sources",
    ] {
        assert_fixed_instant_format(&raw_text(&store, sql));
    }
}

// ----- search -------------------------------------------------------------------------------

#[test]
fn search_finds_ranks_and_fills_hits() {
    let store = demo_store();
    add_material_with_chunks(
        &store,
        "mat-gd",
        &[
            "Unrelated introduction to the demo course.",
            "Gradient descent follows the gradient; each gradient step is small.",
        ],
    );
    add_material_with_chunks(&store, "mat-other", &["A single mention of gradient here."]);

    let hits = store.search("gradient", None, 10).unwrap();
    assert_eq!(hits.len(), 2);
    let best = &hits[0];
    assert_eq!(best.material_id, "mat-gd");
    assert_eq!(best.material_title, "mat-gd");
    assert_eq!(best.course_id, course_id("101"));
    assert_eq!(best.course_code.as_deref(), Some("DEMO101"));
    assert_eq!(best.chunk_ord, 1);
    assert_eq!(best.locator.as_deref(), Some("p. 2"));
    assert_eq!(best.week_hint, Some(1));
    assert_eq!(
        best.url.as_deref(),
        Some("https://lms.example.edu/files/mat-gd")
    );
    assert!(best.snippet.contains("«Gradient»"), "{}", best.snippet);
    assert!(hits[0].score <= hits[1].score);

    // Porter stemming: "steps" finds "step"; any of several words may match.
    assert_eq!(store.search("steps", None, 10).unwrap().len(), 1);
    assert_eq!(
        store
            .search("mention introduction", None, 10)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn search_excludes_hidden_courses_and_filters_by_course() {
    let store = demo_store();
    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();
    add_material_with_chunks(&store, "mat-101", &["shared keyword"]);
    store
        .upsert_material(&material("mat-202", &course_id("202"), "Other"))
        .unwrap();
    readable(&store, "mat-202");
    store
        .replace_chunks("mat-202", &[chunk("mat-202", 0, "shared keyword too")])
        .unwrap();

    assert_eq!(store.search("keyword", None, 10).unwrap().len(), 2);
    let only_202 = store
        .search("keyword", Some(&course_id("202")), 10)
        .unwrap();
    assert_eq!(ids(&only_202, |h| &h.material_id), ["mat-202"]);

    store.set_course_hidden(&course_id("202"), true).unwrap();
    let visible = store.search("keyword", None, 10).unwrap();
    assert_eq!(ids(&visible, |h| &h.material_id), ["mat-101"]);
    assert!(
        store
            .search("keyword", Some(&course_id("202")), 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn search_survives_hostile_queries() {
    let store = demo_store();
    add_material_with_chunks(
        &store,
        "mat-1",
        &["near the col x column, a or b; drop table demo"],
    );
    let long = "gradient ".repeat(5_000) + &"x".repeat(100_000);
    let queries = [
        "NEAR(",
        "NEAR(a b",
        "\"",
        "\"unterminated",
        "a OR",
        "OR",
        "AND NOT",
        "*",
        "demo*",
        "-",
        "-demo",
        "col:x",
        "text:demo",
        "^",
        "^demo",
        "",
        "   ",
        "!@#$%^&*()_+{}|:<>?~`-=[]\\;',./",
        "'; DROP TABLE chunks; --",
        "\") OR 1=1 --",
        "{a b}",
        "a + b",
        "\u{0}",
        long.as_str(),
    ];
    for query in queries {
        let result = store.search(query, None, 10);
        assert!(result.is_ok(), "query {query:?} failed: {result:?}");
    }
    // Operators are searched as ordinary words.
    assert_eq!(store.search("OR", None, 10).unwrap().len(), 1);
    assert_eq!(store.search("col:x", None, 10).unwrap().len(), 1);
    // The store is intact afterwards.
    assert_eq!(store.chunk_count("mat-1").unwrap(), 1);
}

#[test]
fn search_limit_is_clamped() {
    let store = demo_store();
    let texts: Vec<String> = (0..120).map(|i| format!("demo paragraph {i}")).collect();
    let text_refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    add_material_with_chunks(&store, "mat-1", &text_refs);

    assert_eq!(store.search("paragraph", None, 0).unwrap().len(), 1);
    assert_eq!(store.search("paragraph", None, 5).unwrap().len(), 5);
    assert_eq!(
        store.search("paragraph", None, u32::MAX).unwrap().len(),
        MAX_SEARCH_LIMIT as usize
    );
}

// ----- study plans --------------------------------------------------------------------------

#[test]
fn save_and_read_latest_study_plan() {
    let store = demo_store();
    assert!(store.latest_study_plan().unwrap().is_none());

    let before = Utc::now() - chrono::TimeDelta::seconds(1);
    let first = store
        .save_study_plan(&plan(vec![plan_item("Read chapter 1")]))
        .unwrap();
    let second = store
        .save_study_plan(&plan(vec![plan_item("Read chapter 2"), plan_item("Quiz")]))
        .unwrap();
    assert!(second.id > first.id);
    assert!(second.created_at >= before && second.created_at <= Utc::now());
    assert_eq!(second.created_at.timestamp_subsec_nanos(), 0);

    let latest = store.latest_study_plan().unwrap().unwrap();
    assert_eq!(latest.id, second.id);
    assert_eq!(latest.created_at, second.created_at);
    assert_eq!(latest.plan.items.len(), 2);
    assert_eq!(latest.plan.items[0].title, "Read chapter 2");
    assert_eq!(latest.plan.items[0].material_ids, ["demo-material"]);
    assert_eq!(latest.plan.items[0].minutes, Some(45));
    assert_eq!(latest.plan.notes.as_deref(), Some("Focus on week 3"));
    assert_eq!(latest.plan.horizon_end, date("2026-10-04"));
    assert_fixed_instant_format(&raw_text(
        &store,
        "SELECT created_at FROM study_plans ORDER BY id DESC LIMIT 1",
    ));

    // An empty plan (notes only) is fine; a one-day horizon too.
    let mut empty = plan(vec![]);
    empty.horizon_end = empty.horizon_start;
    store.save_study_plan(&empty).unwrap();
}

#[test]
fn a_model_never_reads_a_hidden_course_s_plan_items() {
    let store = demo_store();
    for (external, code, name) in [
        ("202", "DEMO 202", "Hidden Demo Studies"),
        ("203", "DEMO203", "Other Demo Studies"),
    ] {
        store
            .upsert_course(&course(external, Some(code), name))
            .unwrap();
    }
    let item = |course: Option<&str>, title: &str| StudyPlanItem {
        course_id: course.map(str::to_string),
        ..plan_item(title)
    };
    let hidden_id = course_id("202");
    store
        .save_study_plan(&plan(vec![
            item(Some(&course_id("101")), "Visible by id"),
            item(Some("demo101"), "Visible by code"),
            item(None, "No course"),
            item(Some("A course PageLamp doesn't know"), "Unknown course"),
            item(Some(&hidden_id), "Hidden by id"),
            // As an AI app may write it: the code in other case and spacing, the name.
            item(Some("demo202"), "Hidden by code"),
            item(Some("Hidden Demo Studies"), "Hidden by name"),
            // DEMO202 or DEMO203: it could be the hidden one.
            item(Some("DEMO20"), "Ambiguous"),
        ]))
        .unwrap();
    let titles = |plan: Option<StoredStudyPlan>| -> Vec<String> {
        plan.unwrap()
            .plan
            .items
            .into_iter()
            .map(|item| item.title)
            .collect()
    };
    // Nothing hidden yet: all of it.
    assert_eq!(titles(store.latest_visible_study_plan().unwrap()).len(), 8);

    store.set_course_hidden(&hidden_id, true).unwrap();
    assert_eq!(
        titles(store.latest_visible_study_plan().unwrap()),
        [
            "Visible by id",
            "Visible by code",
            "No course",
            "Unknown course"
        ]
    );
    // The student's own screens still show every item.
    assert_eq!(titles(store.latest_study_plan().unwrap()).len(), 8);
}

#[test]
fn a_removed_course_s_plan_items_are_hidden_by_id_name_or_code() {
    let store = demo_store();
    for (external, code, name) in [
        ("202", "DEMO 202", "Removed Demo Studies"),
        ("303", "DEMO303", "Another Demo Seminar"),
    ] {
        store
            .upsert_course(&course(external, Some(code), name))
            .unwrap();
    }
    let item = |course: &str, title: &str| StudyPlanItem {
        course_id: Some(course.to_string()),
        ..plan_item(title)
    };
    store
        .save_study_plan(&plan(vec![
            item(&course_id("101"), "Current by id"),
            item(&course_id("202"), "Removed by id"),
            item("demo202", "Removed by code"),
            item("Removed Demo Studies", "Removed by name"),
            item("DEMO303", "Current by code"),
        ]))
        .unwrap();
    store
        .remove_course(
            &course_id("202"),
            pagelamp_core::removal::RemovalReason::Ended,
            false,
            false,
            Utc::now(),
        )
        .unwrap();
    let titles = |plan: Option<StoredStudyPlan>| -> Vec<String> {
        plan.unwrap()
            .plan
            .items
            .into_iter()
            .map(|item| item.title)
            .collect()
    };
    for plan in [
        store.latest_study_plan().unwrap(),
        store.latest_visible_study_plan().unwrap(),
    ] {
        assert_eq!(titles(plan), ["Current by id", "Current by code"]);
    }

    // Ticking counts the items as shown: index 1 is DEMO303's, whatever the stored order.
    let id = store.latest_study_plan().unwrap().unwrap().id;
    let ticked = store.set_study_plan_item_done(id, 1, true).unwrap();
    let shown: Vec<(&str, bool)> = ticked
        .plan
        .items
        .iter()
        .map(|item| (item.title.as_str(), item.done))
        .collect();
    assert_eq!(shown, [("Current by id", false), ("Current by code", true)]);

    // A course PageLamp still has decides before a removed one with the same code.
    store
        .upsert_course(&course("204", Some("DEMO202"), "Demo Studies again"))
        .unwrap();
    assert_eq!(
        titles(store.latest_study_plan().unwrap()),
        ["Current by id", "Removed by code", "Current by code"]
    );
}

#[test]
fn an_ai_app_s_edit_keeps_the_items_it_could_not_see() {
    let store = demo_store();
    store
        .upsert_course(&course("202", Some("DEMO202"), "Hidden Demo Studies"))
        .unwrap();
    let item = |course: &str, title: &str, day: &str| StudyPlanItem {
        course_id: Some(course.to_string()),
        date: date(day),
        ..plan_item(title)
    };
    store
        .save_study_plan(&plan(vec![
            item(&course_id("101"), "Read chapter 1", "2026-09-28"),
            item("DEMO202", "Hidden course item", "2026-09-29"),
        ]))
        .unwrap();
    store.set_course_hidden(&course_id("202"), true).unwrap();

    // The app sees one item and sends back an edit of it: the hidden one stays, in date order.
    let seen = store.latest_visible_study_plan().unwrap().unwrap();
    assert_eq!(seen.plan.items.len(), 1);
    let mut edited = seen.plan.clone();
    edited.items[0].title = "Read chapter 1 and 2".into();
    edited
        .items
        .push(item(&course_id("101"), "Quiz yourself", "2026-09-30"));
    store.save_study_plan_keeping_unseen(&edited).unwrap();
    let stored: Vec<String> = store
        .latest_study_plan()
        .unwrap()
        .unwrap()
        .plan
        .items
        .into_iter()
        .map(|item| item.title)
        .collect();
    assert_eq!(
        stored,
        [
            "Read chapter 1 and 2",
            "Hidden course item",
            "Quiz yourself"
        ]
    );
    let seen = store.latest_visible_study_plan().unwrap().unwrap();
    assert_eq!(seen.plan.items.len(), 2);

    // Nothing hidden: the plan is saved as sent.
    store.set_course_hidden(&course_id("202"), false).unwrap();
    let only = plan(vec![item(&course_id("101"), "Only this", "2026-09-28")]);
    store.save_study_plan_keeping_unseen(&only).unwrap();
    assert_eq!(
        store.latest_study_plan().unwrap().unwrap().plan.items.len(),
        1
    );
    // And the limits apply to what was sent.
    let empty_title = plan(vec![item(&course_id("101"), " ", "2026-09-28")]);
    assert!(matches!(
        store.save_study_plan_keeping_unseen(&empty_title),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn save_study_plan_validation() {
    let store = demo_store();
    let invalid = |plan: StudyPlan| {
        let result = store.save_study_plan(&plan);
        assert!(matches!(result, Err(Error::Invalid(_))), "{result:?}");
    };

    let mut reversed = plan(vec![plan_item("x")]);
    reversed.horizon_start = date("2026-10-05");
    invalid(reversed);

    invalid(plan(vec![plan_item("x"); MAX_PLAN_ITEMS + 1]));
    invalid(plan(vec![plan_item("  ")]));
    invalid(plan(vec![plan_item(&"t".repeat(MAX_PLAN_TITLE_CHARS + 1))]));

    let mut long_description = plan_item("x");
    long_description.description = Some("d".repeat(MAX_PLAN_TEXT_CHARS + 1));
    invalid(plan(vec![long_description]));

    let mut long_notes = plan(vec![]);
    long_notes.notes = Some("n".repeat(MAX_PLAN_TEXT_CHARS + 1));
    invalid(long_notes);

    let mut many_ids = plan_item("x");
    many_ids.material_ids = vec!["m".to_string(); MAX_PLAN_MATERIAL_IDS + 1];
    invalid(plan(vec![many_ids]));

    let mut long_course = plan_item("x");
    long_course.course_id = Some("c".repeat(1_000));
    invalid(plan(vec![long_course]));

    let mut long_material_id = plan_item("x");
    long_material_id.material_ids = vec!["m".repeat(MAX_PLAN_ID_CHARS + 1)];
    invalid(plan(vec![long_material_id]));

    // Limits are inclusive; multi-byte characters count once.
    let at_limits = plan(vec![
        plan_item(&"é".repeat(MAX_PLAN_TITLE_CHARS));
        MAX_PLAN_ITEMS
    ]);
    store.save_study_plan(&at_limits).unwrap();
    assert_eq!(store.counts().unwrap().study_plans, 1);
}

#[test]
fn save_study_plan_rejects_oversized_plans() {
    // Regression: every field within its own limit, but JSON escapes each control character
    // as 6 bytes (\u0001), so this plan used to be stored as ~5 MB of JSON.
    let store = demo_store();
    let mut item = plan_item("x");
    item.description = Some("\u{1}".repeat(MAX_PLAN_TEXT_CHARS));
    let result = store.save_study_plan(&plan(vec![item; MAX_PLAN_ITEMS]));
    // (Only the error is printed on failure: the accepted plan would be megabytes long.)
    assert!(
        matches!(result, Err(Error::Invalid(_))),
        "{:?}",
        result.as_ref().err()
    );
    assert_eq!(store.counts().unwrap().study_plans, 0);
}

#[test]
fn save_study_plan_keeps_only_the_newest_plans() {
    let store = demo_store();
    let total = MAX_STORED_STUDY_PLANS + 3;
    let mut last_id = 0;
    for n in 1..=total {
        let saved = store
            .save_study_plan(&plan(vec![plan_item(&format!("Plan {n}"))]))
            .unwrap();
        last_id = saved.id;
    }
    assert_eq!(store.counts().unwrap().study_plans, MAX_STORED_STUDY_PLANS);
    let latest = store.latest_study_plan().unwrap().unwrap();
    assert_eq!(latest.id, last_id);
    assert_eq!(latest.plan.items[0].title, format!("Plan {total}"));
    // The oldest plans were the ones deleted.
    let oldest_title: String = store
        .conn()
        .query_row(
            "SELECT json_extract(plan_json, '$.items[0].title')
             FROM study_plans ORDER BY id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(oldest_title, "Plan 4");
}

// ----- counts -------------------------------------------------------------------------------

#[test]
fn counts_reflect_contents() {
    let store = demo_store();
    assert_eq!(
        Store::open_in_memory().unwrap().counts().unwrap(),
        StoreCounts::default()
    );

    let demo = course_id("101");
    store
        .upsert_course(&course("202", Some("DEMO202"), "Hidden Demo"))
        .unwrap();
    store.set_course_hidden(&course_id("202"), true).unwrap();
    store
        .replace_modules(
            &demo,
            &[
                module("mod-1", &demo, "Week 1", Some(1)),
                module("mod-2", &demo, "Week 2", Some(2)),
            ],
        )
        .unwrap();
    // Indexed: status ok + chunks.
    add_material_with_chunks(&store, "indexed", &["one", "two"]);
    store
        .set_text_state("indexed", TextStatus::Ok, None, Some("h"))
        .unwrap();
    // Status ok but no chunks (e.g. scanned PDF) → not indexed.
    store
        .upsert_material(&material("empty", &demo, "Scanned"))
        .unwrap();
    store
        .set_text_state("empty", TextStatus::Ok, Some("no extractable text"), None)
        .unwrap();
    // Not read yet → not indexed (and no chunks: text only belongs to `ok` materials).
    store
        .upsert_material(&material("pending", &demo, "Pending"))
        .unwrap();
    assert!(matches!(
        store.replace_chunks("pending", &[chunk("pending", 0, "three")]),
        Err(Error::Invalid(_))
    ));
    let mut due = event("ev-1", SOURCE, Some(&demo), "Demo quiz");
    due.due_at = Some(ts("2026-10-01T10:00:00Z"));
    store.replace_events(SOURCE, &[due]).unwrap();
    store
        .save_study_plan(&plan(vec![plan_item("Revise")]))
        .unwrap();

    assert_eq!(
        store.counts().unwrap(),
        StoreCounts {
            courses: 1,
            hidden_courses: 1,
            modules: 2,
            materials: 3,
            indexed_materials: 1,
            chunks: 2,
            events: 1,
            study_plans: 1,
            removed_courses: 0,
        }
    );
}

// ----- transactions -------------------------------------------------------------------------

#[test]
fn in_transaction_commits() {
    let store = demo_store();
    let value = store
        .in_transaction(|s| {
            s.upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))?;
            s.set_course_hidden(&course_id("202"), true)?;
            Ok(42)
        })
        .unwrap();
    assert_eq!(value, 42);
    assert!(store.get_course(&course_id("202")).unwrap().unwrap().hidden);
}

#[test]
fn in_transaction_rolls_back_on_error() {
    let store = demo_store();
    let result: pagelamp_core::Result<()> = store.in_transaction(|s| {
        s.upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))?;
        s.set_course_hidden("nope", true)?; // NotFound → whole transaction undone
        Ok(())
    });
    assert!(matches!(result, Err(Error::NotFound(_))));
    assert!(store.get_course(&course_id("202")).unwrap().is_none());
    assert!(store.conn().is_autocommit());
}

#[test]
fn in_transaction_rolls_back_on_panic() {
    let store = demo_store();
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let _ = store.in_transaction(|s| -> pagelamp_core::Result<()> {
            s.upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))?;
            panic!("demo panic inside a transaction");
        });
    }));
    assert!(outcome.is_err());
    assert!(store.conn().is_autocommit());
    assert!(store.get_course(&course_id("202")).unwrap().is_none());
    // The store is still usable.
    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();
}

#[test]
fn in_transaction_must_not_be_nested() {
    let store = demo_store();
    let result = store.in_transaction(|s| {
        s.upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))?;
        s.in_transaction(|_| Ok(()))
    });
    let Err(err @ Error::Db(_)) = result else {
        panic!("expected Error::Db, got {result:?}");
    };
    // The message names the rule, so a caller who nested by accident knows what to fix.
    assert!(err.to_string().contains("must not be nested"), "{err}");
    // The outer transaction was rolled back as a whole.
    assert!(store.conn().is_autocommit());
    assert!(store.get_course(&course_id("202")).unwrap().is_none());
}

#[test]
fn atomic_methods_share_an_outer_transaction() {
    let store = demo_store();
    add_material_with_chunks(&store, "mat-1", &["original demo text"]);

    // Savepoint-based methods inside a transaction that is rolled back afterwards.
    let result: pagelamp_core::Result<()> = store.in_transaction(|s| {
        s.replace_chunks("mat-1", &[chunk("mat-1", 0, "temporary text")])?;
        s.prune_materials(&course_id("101"), &[])?;
        Err(Error::Invalid("demo abort".to_string()))
    });
    assert!(result.is_err());
    let chunks = store.get_chunks("mat-1", 0, None).unwrap();
    assert_eq!(chunks[0].text, "original demo text");
    assert_fts_consistent(&store);

    // ... and committed together when the transaction succeeds.
    store
        .in_transaction(|s| {
            s.replace_chunks("mat-1", &[chunk("mat-1", 0, "committed text")])?;
            s.set_text_state("mat-1", TextStatus::Ok, None, Some("hash"))
        })
        .unwrap();
    assert_eq!(
        store.get_chunks("mat-1", 0, None).unwrap()[0].text,
        "committed text"
    );
    assert_eq!(store.counts().unwrap().indexed_materials, 1);
}

// ----- term source and AI access (docs/ARCHITECTURE.md §3 rule 8) --------------------------

#[test]
fn term_source_reports_where_effective_dates_come_from() {
    let store = demo_store();
    let id = course_id("101");
    assert_eq!(
        store.get_course(&id).unwrap().unwrap().term_source,
        TermSource::Synced
    );

    store
        .set_course_term(&id, None, Some(date("2026-12-01")))
        .unwrap();
    assert_eq!(
        store.get_course(&id).unwrap().unwrap().term_source,
        TermSource::User
    );

    store.set_course_term(&id, None, None).unwrap();
    assert_eq!(
        store.get_course(&id).unwrap().unwrap().term_source,
        TermSource::Synced
    );

    let mut no_term = course("202", Some("DEMO202"), "Advanced Demo Studies");
    no_term.term_start = None;
    no_term.term_end = None;
    store.upsert_course(&no_term).unwrap();
    assert_eq!(
        store
            .get_course(&course_id("202"))
            .unwrap()
            .unwrap()
            .term_source,
        TermSource::None
    );
}

#[test]
fn ai_access_defaults_on_survives_resync_and_rejects_unknown_ids() {
    let store = demo_store();
    let id = course_id("101");
    let stored = store.get_course(&id).unwrap().unwrap();
    assert!(stored.ai_access);
    assert_eq!(stored.ai_materials(), AiMaterialsState::Readable);

    store.set_course_ai_access(&id, false).unwrap();
    store
        .upsert_course(&course("101", Some("DEMO101"), "Intro to Demo Studies"))
        .unwrap();
    let stored = store.get_course(&id).unwrap().unwrap();
    assert!(!stored.ai_access, "a sync must never reset the switch");
    assert_eq!(stored.ai_materials(), AiMaterialsState::TurnedOff);

    assert!(matches!(
        store.set_course_ai_access("canvas:lms.example.edu/course/nope", true),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn prohibited_policy_withholds_text_and_the_switch_applies_again_afterwards() {
    let store = demo_store();
    let id = course_id("101");
    // Policy wins even with the switch on.
    store
        .set_course_policy(&id, AiPolicy::Prohibited, None)
        .unwrap();
    let stored = store.get_course(&id).unwrap().unwrap();
    assert!(stored.ai_access);
    assert_eq!(stored.ai_materials(), AiMaterialsState::WithheldByPolicy);

    // Switch off while prohibited, then relax the policy: the stored switch applies.
    store.set_course_ai_access(&id, false).unwrap();
    store
        .set_course_policy(&id, AiPolicy::LearningAid, None)
        .unwrap();
    assert_eq!(
        store.get_course(&id).unwrap().unwrap().ai_materials(),
        AiMaterialsState::TurnedOff
    );
    store.set_course_ai_access(&id, true).unwrap();
    assert_eq!(
        store.get_course(&id).unwrap().unwrap().ai_materials(),
        AiMaterialsState::Readable
    );

    // `unknown` policy is readable.
    store
        .set_course_policy(&id, AiPolicy::Unknown, None)
        .unwrap();
    assert_eq!(
        store.get_course(&id).unwrap().unwrap().ai_materials(),
        AiMaterialsState::Readable
    );
}

#[test]
fn ai_readable_search_excludes_turned_off_and_prohibited_courses() {
    let store = demo_store();
    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();
    add_material_with_chunks(&store, "m101", &["photosynthesis in demo plants"]);
    store
        .upsert_material(&material("m202", &course_id("202"), "m202"))
        .unwrap();
    readable(&store, "m202");
    store
        .replace_chunks(
            "m202",
            &[chunk("m202", 0, "photosynthesis in advanced demo plants")],
        )
        .unwrap();

    let codes = |hits: Vec<SearchHit>| -> Vec<String> {
        let mut codes: Vec<String> = hits.into_iter().filter_map(|h| h.course_code).collect();
        codes.sort();
        codes
    };
    assert_eq!(
        codes(
            store
                .search_ai_readable("photosynthesis", None, 10)
                .unwrap()
        ),
        ["DEMO101", "DEMO202"]
    );

    store
        .set_course_ai_access(&course_id("101"), false)
        .unwrap();
    assert_eq!(
        codes(
            store
                .search_ai_readable("photosynthesis", None, 10)
                .unwrap()
        ),
        ["DEMO202"]
    );
    store
        .set_course_policy(&course_id("202"), AiPolicy::Prohibited, None)
        .unwrap();
    assert!(
        store
            .search_ai_readable("photosynthesis", None, 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .search_ai_readable("photosynthesis", Some(&course_id("202")), 10)
            .unwrap()
            .is_empty()
    );
    // The student's own (non-AI) search still sees everything that is not hidden.
    assert_eq!(
        codes(store.search("photosynthesis", None, 10).unwrap()),
        ["DEMO101", "DEMO202"]
    );
}

#[test]
fn enrollment_is_marked_without_deleting_anything() {
    let store = demo_store();
    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();
    assert!(
        store
            .get_course(&course_id("202"))
            .unwrap()
            .unwrap()
            .enrollment_active
    );

    store
        .mark_enrollment_active(SOURCE, &[course_id("101")])
        .unwrap();
    let ended = store.get_course(&course_id("202")).unwrap().unwrap();
    assert!(
        !ended.enrollment_active,
        "no longer listed → inactive, but kept"
    );
    assert!(
        store
            .get_course(&course_id("101"))
            .unwrap()
            .unwrap()
            .enrollment_active
    );

    // A re-sync that lists it again (e.g. a new term) makes it active again, and upserts
    // never touch the flag.
    store
        .upsert_course(&course("202", Some("DEMO202"), "Advanced Demo Studies"))
        .unwrap();
    assert!(
        !store
            .get_course(&course_id("202"))
            .unwrap()
            .unwrap()
            .enrollment_active
    );
    store
        .mark_enrollment_active(SOURCE, &[course_id("101"), course_id("202")])
        .unwrap();
    assert!(
        store
            .get_course(&course_id("202"))
            .unwrap()
            .unwrap()
            .enrollment_active
    );
}
