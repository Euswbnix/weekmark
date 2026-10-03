//! The AI weekly note through the facade (model-access design §5.3; beta.2): the courses'
//! structure and the plan's progress only (the policy golden), the answer checked (graded work
//! and made-up course ids dropped, bad output), kept notes and their deletion, a run listed in
//! `activity()`, and "prepare it when I open PageLamp on Monday" for API keys and local models
//! only (plan D27): once a Monday whatever the outcome, never over the budget, on the student's
//! Monday across a DST change. Synthetic data only; no run leaves this computer.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate, SubsecRound, TimeZone, Utc};
use pagelamp_app::ai::{
    BackendRef, EstimateRequest, GenEvent, GenStage, ModelChoice, WeeklyNoteOptions,
};
use pagelamp_app::{App, AppErrorKind, BreakInput, CourseDatesInput};
use pagelamp_core::ai::{AiFeature, BlockReason, Effort, ModelErrorKind, ProviderRow};
use pagelamp_core::model::*;
use pagelamp_core::secrets::{MemorySecrets, SecretBackend};
use pagelamp_core::store::{GenerationRecord, GenerationStatus, Store};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SOURCE: &str = "canvas:lms.example.edu";

fn course_id(external: &str) -> String {
    format!("{SOURCE}/course/{external}")
}

fn day(offset: i64) -> NaiveDate {
    Local::now().date_naive() + Duration::days(offset)
}

/// This week's Monday on the computer's clock (the note's `week_of`).
fn this_monday() -> NaiveDate {
    let today = Local::now().date_naive();
    today - Duration::days(i64::from(today.weekday().num_days_from_monday()))
}

/// Next Monday at 14:00 UTC: a Monday morning in Toronto, inside every fixture term.
fn next_monday() -> DateTime<Utc> {
    (this_monday() + Duration::days(7))
        .and_hms_opt(14, 0, 0)
        .unwrap()
        .and_utc()
}

fn add_course(store: &Store, external: &str, start: NaiveDate, end: NaiveDate) {
    store
        .upsert_course(&CourseUpsert {
            id: course_id(external),
            source_id: SOURCE.into(),
            external_id: external.into(),
            code: Some(format!("DEMO{external}")),
            name: format!("Demo course {external}"),
            term_start: Some(start),
            term_end: Some(end),
            url: None,
            syllabus_text: None,
            lms: Default::default(),
        })
        .unwrap();
}

/// A material of `external` whose text is a canary: it must never reach a note's prompt.
fn canary_material(store: &Store, external: &str) {
    let id = format!("{SOURCE}/file/{external}-slides");
    store
        .upsert_material(&MaterialUpsert {
            id: id.clone(),
            course_id: course_id(external),
            module_id: None,
            kind: MaterialKind::File,
            title: "Lecture slides".into(),
            url: None,
            local_path: None,
            mime: None,
            published_at: None,
            week_hint: None,
        })
        .unwrap();
    store
        .set_text_state(&id, TextStatus::Ok, None, Some("hash-1"))
        .unwrap();
    store
        .replace_chunks(
            &id,
            &[Chunk {
                material_id: id.clone(),
                ord: 0,
                locator: None,
                text: format!("Canary {external} text."),
            }],
        )
        .unwrap();
}

fn open(dir: &std::path::Path) -> (App, Arc<MemorySecrets>) {
    let secrets = Arc::new(MemorySecrets::new());
    let app = App::open_at_with_secrets(dir.join("data"), secrets.clone()).unwrap();
    Store::open(&app.db_path())
        .unwrap()
        .upsert_source(&SourceRecord {
            id: SOURCE.into(),
            kind: SourceKind::Canvas,
            label: "lms.example.edu".into(),
            config: json!({ "base_url": "https://lms.example.edu" }),
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })
        .unwrap();
    (app, secrets)
}

fn deadline(id: &str, external: &str, title: &str, days: i64) -> Event {
    Event {
        id: format!("{SOURCE}/assignment/{id}"),
        source_id: SOURCE.into(),
        course_id: Some(course_id(external)),
        kind: EventKind::AssignmentDue,
        title: title.into(),
        starts_at: None,
        ends_at: None,
        due_at: Some(Utc::now() + Duration::days(days)),
        url: None,
        updated_at: Utc::now(),
        course_hint: None,
    }
}

/// DEMO101 readable (in a reading week the student labelled), DEMO202 prohibited, DEMO303
/// turned off, DEMO404 hidden, DEMO505 over long ago, DEMO606 starting in two months (not
/// active) but with a deadline this week; every course has canary text. A deadline in two
/// days and a study plan with last week's progress.
fn app_with_courses(dir: &std::path::Path) -> (App, Arc<MemorySecrets>) {
    let (app, secrets) = open(dir);
    let store = Store::open(&app.db_path()).unwrap();
    for external in ["101", "202", "303", "404"] {
        add_course(&store, external, day(-30), day(60));
        canary_material(&store, external);
    }
    add_course(&store, "505", day(-400), day(-300));
    add_course(&store, "606", day(60), day(150));
    for external in ["505", "606"] {
        canary_material(&store, external);
    }
    store
        .replace_events(
            SOURCE,
            &[
                deadline("ps2", "101", "Problem Set 2", 2),
                deadline("survey", "606", "Pre-course survey", 3),
            ],
        )
        .unwrap();
    let item = |date: NaiveDate, title: &str, done: bool| StudyPlanItem {
        date,
        course_id: Some(course_id("101")),
        title: title.into(),
        description: None,
        material_ids: Vec::new(),
        minutes: Some(30),
        done,
    };
    store
        .save_study_plan(&StudyPlan {
            horizon_start: day(-7),
            horizon_end: day(7),
            items: vec![
                item(day(-3), "Review week 2", true),
                item(day(-2), "Practice problems", false),
                item(day(0), "Read chapter 4", false),
                // DEMO404's, by id and by code: hidden below, so never sent.
                StudyPlanItem {
                    course_id: Some(course_id("404")),
                    ..item(day(-1), "Canary plan item 404 by id", true)
                },
                StudyPlanItem {
                    course_id: Some("DEMO404".into()),
                    ..item(day(0), "Canary plan item 404 by code", false)
                },
            ],
            notes: None,
        })
        .unwrap();
    drop(store);
    app.set_course_dates(
        "DEMO101",
        Some(CourseDatesInput {
            first_class: Some(this_monday() - Duration::days(21)),
            last_class: Some(this_monday() + Duration::days(60)),
            exams_end: None,
            breaks: vec![BreakInput {
                kind: BreakKind::ReadingWeek,
                start: this_monday(),
                end: this_monday() + Duration::days(4),
                numbered: false,
                label: Some("Canary break label".into()),
            }],
            second_segment: None,
        }),
    )
    .unwrap();
    app.set_course_policy("DEMO202", AiPolicy::Prohibited, None)
        .unwrap();
    app.set_course_ai_access("DEMO303", false).unwrap();
    app.set_course_hidden("DEMO404", true).unwrap();
    (app, secrets)
}

/// A model on this computer (an Ollama mock, mode D), chosen for weekly notes and disclosed.
async fn with_local_model(app: &App) -> MockServer {
    let server = MockServer::start().await;
    Store::open(&app.db_path())
        .unwrap()
        .insert_model_provider(&ProviderRow {
            id: "ollama".into(),
            preset: "ollama".into(),
            label: "Ollama".into(),
            wire: "ollama_native".into(),
            base_url: server.uri(),
            created_at: Utc::now().trunc_subsecs(0),
            last_probe_json: None,
        })
        .unwrap();
    let backend = BackendRef::Provider {
        provider_id: "ollama".into(),
    };
    choose(app, backend.clone(), "local-model");
    acknowledge(app, &backend);
    server
}

fn choose(app: &App, backend: BackendRef, model: &str) {
    app.set_feature_model(
        AiFeature::WeeklyNote,
        Some(ModelChoice {
            backend,
            model: model.into(),
            effort: Effort::Lowest,
        }),
    )
    .unwrap();
}

/// A choice `set_feature_model` can't make in this build (the plans), written as stored.
fn route_to(app: &App, backend: BackendRef) {
    let routing = BTreeMap::from([(
        AiFeature::WeeklyNote,
        ModelChoice {
            backend,
            model: "plan-model".into(),
            effort: Effort::Lowest,
        },
    )]);
    Store::open(&app.db_path())
        .unwrap()
        .set_setting("ai.routing", &routing)
        .unwrap();
}

fn acknowledge(app: &App, backend: &BackendRef) {
    let version = app
        .ai_status()
        .unwrap()
        .backends
        .iter()
        .find(|b| &b.backend == backend)
        .unwrap()
        .disclosure
        .version;
    app.acknowledge_ai_disclosure(backend, version).unwrap();
}

fn answer(answer: &serde_json::Value) -> ResponseTemplate {
    let lines = [
        json!({"model": "local-model", "created_at": "2026-09-29T10:00:00Z",
               "message": {"role": "assistant", "content": answer.to_string()}, "done": false}),
        json!({"model": "local-model", "created_at": "2026-09-29T10:00:01Z",
               "message": {"role": "assistant", "content": ""}, "done": true,
               "done_reason": "stop", "prompt_eval_count": 300, "eval_count": 80}),
    ];
    let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/x-ndjson")
        .set_body_string(body)
}

fn a_note() -> serde_json::Value {
    json!({
        "note": "  It is reading week for DEMO101, and Problem Set 2 is due soon. Last week you \
                 did one of two planned items.  ",
        "focus": [
            {"text": "Re-read the week 3 slides before Problem Set 2", "course_id": course_id("101")},
            {"text": "Write Problem Set 2's answers", "course_id": course_id("101")},
            {"text": "Catch up on DEMO303", "course_id": "canvas:made.up/course/9"},
            {"text": "Plan next week", "course_id": null},
            {"text": "A fourth thing", "course_id": null}
        ]
    })
}

fn click() -> WeeklyNoteOptions {
    WeeklyNoteOptions {
        ui_language: Some("en".into()),
        ..WeeklyNoteOptions::default()
    }
}

async fn sent(server: &MockServer, n: usize) -> String {
    String::from_utf8(server.received_requests().await.unwrap()[n].body.clone()).unwrap()
}

/// Policy golden: the prompt holds every visible, active course's structure (titles, weeks,
/// phase, deadlines) and the plan's progress, and no material text of any course; prohibited
/// and turned-off courses appear as structure only, hidden and ended ones not at all (a hidden
/// course's plan items neither, nor in the progress count), and a break by its kind, never its
/// label.
#[tokio::test]
async fn a_note_is_written_from_structure_and_progress_only() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&a_note()))
        .mount(&server)
        .await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let note = app
        .write_weekly_note("note-1", click(), move |event| {
            sink.lock().unwrap().push(event)
        })
        .await
        .unwrap();

    let body = sent(&server, 0).await;
    for external in ["101", "202", "303", "404", "505", "606"] {
        assert!(
            !body.contains(&format!("Canary {external}")),
            "{external}: {body}"
        );
    }
    assert!(!body.contains("Canary break label"), "{body}");
    assert!(!body.contains("Canary plan item 404"), "hidden: {body}");
    for external in ["101", "202", "303"] {
        assert!(
            body.contains(&format!("Demo course {external}")),
            "{external}"
        );
    }
    assert!(!body.contains("Demo course 404"), "hidden");
    assert!(!body.contains("Demo course 505"), "ended, no deadline");
    // Not active, but a deadline this week: its deadlines only, like the digest.
    assert!(body.contains("Demo course 606"), "{body}");
    assert!(body.contains("not active: its deadlines only"), "{body}");
    assert!(body.contains("Pre-course survey"), "{body}");
    assert!(body.contains("no (the course does not allow AI use)"));
    assert!(body.contains("no (turned off by the student)"));
    assert!(body.contains("Phase: break (reading_week)"), "{body}");
    assert!(body.contains("Problem Set 2"));
    assert!(body.contains("last 7 days 1 of 2 items done"), "{body}");
    assert!(body.contains("Read chapter 4"));

    // The answer, checked: graded work and a made-up course id go, at most three items.
    assert_eq!(
        note.text,
        "It is reading week for DEMO101, and Problem Set 2 is due soon. Last week you did one \
         of two planned items."
    );
    let focus: Vec<(&str, Option<&str>)> = note
        .focus
        .iter()
        .map(|item| (item.text.as_str(), item.course_id.as_deref()))
        .collect();
    assert_eq!(
        focus,
        [
            (
                "Re-read the week 3 slides before Problem Set 2",
                Some(course_id("101").as_str())
            ),
            ("Catch up on DEMO303", None),
            ("Plan next week", None),
        ]
    );
    assert_eq!(note.graded_work_left_out, 1);
    assert_eq!(note.week_of, this_monday());
    assert!(!note.automatic);
    assert_eq!(note.meta.feature, AiFeature::WeeklyNote);
    assert!(note.meta.on_device);
    let courses: Vec<&str> = note
        .meta
        .context
        .courses
        .iter()
        .map(|c| c.course_id.as_str())
        .collect();
    assert_eq!(
        courses,
        [
            course_id("101"),
            course_id("202"),
            course_id("303"),
            course_id("606")
        ]
    );
    assert!(note.meta.context.courses.iter().all(|c| !c.text_included));
    assert_eq!(
        events.lock().unwrap().last(),
        Some(&GenEvent::Finished { ok: true })
    );

    // Kept with its AI label.
    let kept = app.weekly_notes().unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].meta.generation_id, "note-1");
    assert_eq!(kept[0].meta.backend_label, "Ollama");
}

#[tokio::test]
async fn an_empty_answer_is_bad_output_and_nothing_to_write_about_is_blocked() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&json!({"note": " ", "focus": []})))
        .mount(&server)
        .await;
    let err = app
        .write_weekly_note("note-bad", click(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::Model);
    assert_eq!(err.model_error, Some(ModelErrorKind::BadOutput));
    assert!(app.weekly_notes().unwrap().is_empty());

    // Only a course that ended long ago, no deadline and no plan: nothing to write about,
    // nothing sent.
    let other = tempfile::tempdir().unwrap();
    let (app, _) = open(other.path());
    add_course(
        &Store::open(&app.db_path()).unwrap(),
        "505",
        day(-400),
        day(-300),
    );
    let server = with_local_model(&app).await;
    let err = app
        .write_weekly_note("note-none", click(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind, err.blocked),
        (AppErrorKind::Blocked, Some(BlockReason::NothingToWrite))
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

/// A week with nothing to write about is known before the click: the estimate says so (after
/// no model chosen, as the run checks), a click is blocked before anything is sent, and
/// Monday's note isn't due, so that Monday's try stays for a course synced later that day.
#[tokio::test]
async fn nothing_to_write_about_is_known_before_the_click_and_spends_no_monday() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = open(temp.path());
    app.set_time_zone(Some("America/Toronto")).unwrap();
    let estimate = || {
        app.estimate_generation(&EstimateRequest::WeeklyNote)
            .unwrap()
    };
    assert_eq!(estimate().would_block, Some(BlockReason::NoModelChosen));
    let server = with_local_model(&app).await;
    let blocked = estimate();
    assert_eq!(
        (
            blocked.would_block,
            blocked.input_tokens,
            blocked.micro_usd_upper
        ),
        (Some(BlockReason::NothingToWrite), 0, None)
    );

    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let err = app
        .write_weekly_note("note-none", click(), move |event| {
            sink.lock().unwrap().push(event)
        })
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind, err.blocked),
        (AppErrorKind::Blocked, Some(BlockReason::NothingToWrite))
    );
    assert_eq!(
        *events.lock().unwrap(),
        [
            GenEvent::Stage {
                stage: GenStage::BuildingContext
            },
            GenEvent::Finished { ok: false }
        ]
    );
    assert!(app.weekly_notes().unwrap().is_empty());

    // Monday 2026-11-02, 09:00 in Toronto, opted in: not due, and an automatic run records no
    // try.
    app.set_prepare_weekly_note_on_monday(true).unwrap();
    let monday = Utc.with_ymd_and_hms(2026, 11, 2, 14, 0, 0).unwrap();
    let prepare = |at| app.startup_tasks(at).unwrap().prepare_weekly_note;
    assert!(!prepare(monday), "nothing to write about");
    let automatic = WeeklyNoteOptions {
        automatic: true,
        ..click()
    };
    let err = app
        .write_weekly_note_at(monday, "auto-empty", automatic.clone(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::Invalid);
    let tried: Option<NaiveDate> = Store::open(&app.db_path())
        .unwrap()
        .setting_or_absent("ai.weekly_note_tried_on")
        .unwrap();
    assert_eq!(tried, None, "no try recorded");
    assert!(server.received_requests().await.unwrap().is_empty());

    // A course synced later that Monday: due, and prepared then.
    add_course(
        &Store::open(&app.db_path()).unwrap(),
        "101",
        NaiveDate::from_ymd_opt(2026, 9, 8).unwrap(),
        NaiveDate::from_ymd_opt(2026, 12, 18).unwrap(),
    );
    let later = monday + Duration::hours(1);
    assert!(prepare(later));
    Mock::given(method("POST"))
        .respond_with(answer(&a_note()))
        .mount(&server)
        .await;
    let note = app
        .write_weekly_note_at(later, "auto-later", automatic, |_| {})
        .await
        .unwrap();
    assert!(note.automatic);
    assert!(!prepare(later + Duration::hours(1)));
}

/// A study plan item today is something to write about, with no course at all.
#[tokio::test]
async fn a_plan_item_alone_is_something_to_write_about() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = open(temp.path());
    let _server = with_local_model(&app).await;
    Store::open(&app.db_path())
        .unwrap()
        .save_study_plan(&StudyPlan {
            horizon_start: day(0),
            horizon_end: day(6),
            items: vec![StudyPlanItem {
                date: day(0),
                course_id: None,
                title: "Read chapter 1".into(),
                description: None,
                material_ids: Vec::new(),
                minutes: Some(30),
                done: false,
            }],
            notes: None,
        })
        .unwrap();
    let estimate = app
        .estimate_generation(&EstimateRequest::WeeklyNote)
        .unwrap();
    assert_eq!(estimate.would_block, None);
    assert!(estimate.input_tokens > 0);
}

/// The latest 5 are kept; deleting a course's generated content deletes the notes that
/// covered it; one note can be deleted.
#[tokio::test]
async fn notes_are_kept_and_deleted_with_the_courses_they_cover() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&a_note()))
        .mount(&server)
        .await;
    for n in 0..6 {
        app.write_weekly_note(&format!("note-{n}"), click(), |_| {})
            .await
            .unwrap();
    }
    let kept: Vec<String> = app
        .weekly_notes()
        .unwrap()
        .into_iter()
        .map(|note| note.meta.generation_id)
        .collect();
    assert_eq!(kept.len(), 5);
    assert!(!kept.contains(&"note-0".to_string()), "the oldest goes");

    // DEMO505 ended: no note covered it.
    assert_eq!(app.delete_generated(Some("DEMO505")).unwrap(), 0);
    assert_eq!(app.weekly_notes().unwrap().len(), 5);
    app.delete_weekly_note("note-5").unwrap();
    assert_eq!(
        app.delete_weekly_note("note-5").unwrap_err().kind,
        AppErrorKind::NotFound
    );
    // DEMO303 (turned off) was in every note as structure.
    assert_eq!(app.delete_generated(Some("DEMO303")).unwrap(), 4);
    assert!(app.weekly_notes().unwrap().is_empty());
}

/// Failed runs never push kept notes out (a Monday whose local model isn't running adds a
/// failed row every week).
#[tokio::test]
async fn failed_runs_never_push_kept_notes_out() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&a_note()))
        .mount(&server)
        .await;
    for n in 0..5 {
        app.write_weekly_note(&format!("note-{n}"), click(), |_| {})
            .await
            .unwrap();
    }
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    for n in 0..3 {
        assert!(
            app.write_weekly_note(&format!("failed-{n}"), click(), |_| {})
                .await
                .is_err()
        );
    }
    assert_eq!(app.weekly_notes().unwrap().len(), 5);
}

/// Plan D27 (revised 2026-09-29): "prepare it when I open PageLamp on Monday" is for the
/// student's own API key (mode C) and a model on this computer (mode D); the ChatGPT plan
/// (mode A) and the Claude plan (mode B) never run in the background.
#[tokio::test]
async fn preparing_on_monday_is_for_api_keys_and_local_models_only() {
    let temp = tempfile::tempdir().unwrap();
    let (app, secrets) = open(temp.path());
    app.set_time_zone(Some("America/Toronto")).unwrap();
    // A course with fixed dates around those Mondays: the note has something to write about.
    add_course(
        &Store::open(&app.db_path()).unwrap(),
        "101",
        NaiveDate::from_ymd_opt(2026, 9, 8).unwrap(),
        NaiveDate::from_ymd_opt(2026, 12, 18).unwrap(),
    );
    // Monday 2026-11-02, 09:00 in Toronto; the Tuesday after.
    let monday = Utc.with_ymd_and_hms(2026, 11, 2, 14, 0, 0).unwrap();
    let tuesday = monday + Duration::days(1);
    let prepare = |at| app.startup_tasks(at).unwrap().prepare_weekly_note;

    // No model chosen yet: not offered.
    let settings = app.weekly_note_settings().unwrap();
    assert!(!settings.prepare_on_monday && !settings.prepare_on_monday_allowed);
    assert_eq!(
        app.set_prepare_weekly_note_on_monday(true)
            .unwrap_err()
            .kind,
        AppErrorKind::Invalid
    );

    // Mode D: Ollama on this computer.
    let _server = with_local_model(&app).await;
    assert!(!prepare(monday), "not opted in");
    let settings = app.set_prepare_weekly_note_on_monday(true).unwrap();
    assert!(settings.prepare_on_monday && settings.prepare_on_monday_allowed);
    assert!(prepare(monday));
    assert!(!prepare(tuesday));

    // Mode C: the student's own API key.
    Store::open(&app.db_path())
        .unwrap()
        .insert_model_provider(&ProviderRow {
            id: "openai".into(),
            preset: "openai".into(),
            label: "OpenAI".into(),
            wire: "openai_responses".into(),
            base_url: "https://api.openai.com/v1".into(),
            created_at: Utc::now().trunc_subsecs(0),
            last_probe_json: None,
        })
        .unwrap();
    secrets.set("llm:openai", "sk-demo-not-a-real-key").unwrap();
    choose(
        &app,
        BackendRef::Provider {
            provider_id: "openai".into(),
        },
        "gpt-demo",
    );
    assert!(
        app.weekly_note_settings()
            .unwrap()
            .prepare_on_monday_allowed
    );
    assert!(prepare(monday));

    // Modes A and B: the plans. The stored answer stays, but nothing is prepared, and it
    // can't be turned on again; turning it off always works.
    for plan in [BackendRef::Codex, BackendRef::ClaudeCode] {
        route_to(&app, plan.clone());
        let settings = app.weekly_note_settings().unwrap();
        assert!(
            settings.prepare_on_monday && !settings.prepare_on_monday_allowed,
            "{plan:?}"
        );
        assert!(!prepare(monday), "{plan:?}");
        assert_eq!(
            app.set_prepare_weekly_note_on_monday(true)
                .unwrap_err()
                .kind,
            AppErrorKind::Invalid,
            "{plan:?}"
        );
        // An automatic run is refused before anything else.
        let err = app
            .write_weekly_note(
                "auto",
                WeeklyNoteOptions {
                    automatic: true,
                    ..click()
                },
                |_| {},
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind, AppErrorKind::Invalid, "{plan:?}");
    }
    assert!(
        !app.set_prepare_weekly_note_on_monday(false)
            .unwrap()
            .prepare_on_monday
    );

    // A note the student wrote that Monday is the week's: nothing more is prepared. One that
    // failed doesn't count (only an automatic try does, see below).
    choose(
        &app,
        BackendRef::Provider {
            provider_id: "ollama".into(),
        },
        "local-model",
    );
    app.set_prepare_weekly_note_on_monday(true).unwrap();
    let record = |id: &str, status| GenerationRecord {
        id: id.into(),
        feature: AiFeature::WeeklyNote,
        course_id: None,
        week: None,
        backend: "provider:ollama".into(),
        model: "local-model".into(),
        status,
        created_at: monday - Duration::hours(1),
        prompt_version: 4,
        output_json: None,
        summary_json: None,
        error_kind: None,
        week_starts_on: None,
    };
    let store = Store::open(&app.db_path()).unwrap();
    store
        .record_generation(&record("failed", GenerationStatus::Failed))
        .unwrap();
    assert!(prepare(monday));
    store
        .record_generation(&record("written", GenerationStatus::Accepted))
        .unwrap();
    assert!(!prepare(monday));
    assert!(prepare(monday + Duration::days(7)), "the next Monday");
}

/// An automatic note is tried once a Monday whatever its outcome: it is recorded as it
/// starts, so a surface that asks again every hour never repeats a failed (paid) run.
#[tokio::test]
async fn an_automatic_note_is_tried_once_a_monday_whatever_the_outcome() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    app.set_time_zone(Some("America/Toronto")).unwrap();
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&json!({"note": "", "focus": []})))
        .mount(&server)
        .await;
    app.set_prepare_weekly_note_on_monday(true).unwrap();
    let monday = next_monday();
    let prepare = |at| app.startup_tasks(at).unwrap().prepare_weekly_note;
    let automatic = || WeeklyNoteOptions {
        automatic: true,
        ..click()
    };

    assert!(prepare(monday));
    let err = app
        .write_weekly_note_at(monday, "auto-1", automatic(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err.model_error, Some(ModelErrorKind::BadOutput));
    assert!(!prepare(monday + Duration::hours(1)), "tried once");
    let again = app
        .write_weekly_note_at(monday + Duration::hours(1), "auto-2", automatic(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(again.kind, AppErrorKind::Invalid);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    // A click still works that Monday.
    server.reset().await;
    Mock::given(method("POST"))
        .respond_with(answer(&a_note()))
        .mount(&server)
        .await;
    let note = app
        .write_weekly_note_at(monday + Duration::hours(2), "click", click(), |_| {})
        .await
        .unwrap();
    assert!(!note.automatic);

    // The next Monday: prepared once, and it says so.
    let next = monday + Duration::days(7);
    assert!(prepare(next));
    let note = app
        .write_weekly_note_at(next, "auto-3", automatic(), |_| {})
        .await
        .unwrap();
    assert!(note.automatic);
    assert!(
        !prepare(next + Duration::hours(1)),
        "this week's note exists"
    );
}

/// An automatic run never goes over the monthly budget, whatever its options say.
#[tokio::test]
async fn an_automatic_note_never_goes_over_the_budget() {
    let temp = tempfile::tempdir().unwrap();
    let (app, secrets) = app_with_courses(temp.path());
    app.set_time_zone(Some("America/Toronto")).unwrap();
    Store::open(&app.db_path())
        .unwrap()
        .insert_model_provider(&ProviderRow {
            id: "openai".into(),
            preset: "openai".into(),
            label: "OpenAI".into(),
            wire: "openai_responses".into(),
            base_url: "https://api.openai.com/v1".into(),
            created_at: Utc::now().trunc_subsecs(0),
            last_probe_json: None,
        })
        .unwrap();
    secrets.set("llm:openai", "sk-demo-not-a-real-key").unwrap();
    let backend = BackendRef::Provider {
        provider_id: "openai".into(),
    };
    choose(&app, backend.clone(), "gpt-6-luna");
    acknowledge(&app, &backend);
    app.set_prepare_weekly_note_on_monday(true).unwrap();
    app.set_monthly_budget(Some(1)).unwrap();
    let err = app
        .write_weekly_note_at(
            next_monday(),
            "auto-over",
            WeeklyNoteOptions {
                automatic: true,
                override_budget: true,
                ..click()
            },
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(err.blocked, Some(BlockReason::BudgetReached));
    // The try counts: nothing more that Monday.
    assert!(
        !app.startup_tasks(next_monday() + Duration::hours(1))
            .unwrap()
            .prepare_weekly_note
    );
}

/// A note run is a generation in `activity()` (the install gate waits for it) until it ends.
#[tokio::test]
async fn a_note_run_is_listed_in_activity_while_it_runs() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&a_note()).set_delay(std::time::Duration::from_millis(800)))
        .mount(&server)
        .await;
    let task = {
        let app = app.clone();
        tokio::spawn(async move { app.write_weekly_note("note-busy", click(), |_| {}).await })
    };
    let listed = |app: &App| {
        app.activity()
            .items
            .iter()
            .any(|item| item.generation_id.as_deref() == Some("note-busy"))
    };
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    while !listed(&app) {
        assert!(
            Instant::now() < deadline,
            "the run never showed in activity()"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    task.await.unwrap().unwrap();
    assert!(!listed(&app), "gone once it ends");
}

/// "Monday" is the student's wall-clock Monday in the reminder zone, as for the digest: Toronto
/// leaves DST on 2026-11-01.
#[tokio::test]
async fn monday_is_the_student_s_monday_across_the_end_of_dst() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = open(temp.path());
    app.set_time_zone(Some("America/Toronto")).unwrap();
    let server = with_local_model(&app).await;
    app.set_prepare_weekly_note_on_monday(true).unwrap();
    // A course with fixed dates, so the note has something to say on those Mondays.
    add_course(
        &Store::open(&app.db_path()).unwrap(),
        "101",
        NaiveDate::from_ymd_opt(2026, 9, 8).unwrap(),
        NaiveDate::from_ymd_opt(2026, 12, 18).unwrap(),
    );
    let prepare = |y, m, d, h, min| {
        app.startup_tasks(Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap())
            .unwrap()
            .prepare_weekly_note
    };
    // After the change (UTC-5): 04:30 UTC is still Sunday 23:30, 05:30 UTC is Monday 00:30.
    assert!(!prepare(2026, 11, 2, 4, 30));
    assert!(prepare(2026, 11, 2, 5, 30));
    // The Monday before (UTC-4): 04:30 UTC is already Monday 00:30.
    assert!(!prepare(2026, 10, 26, 3, 30));
    assert!(prepare(2026, 10, 26, 4, 30));

    // The automatic run itself checks the same Monday: refused on Sunday 23:30, run at Monday
    // 00:30.
    Mock::given(method("POST"))
        .respond_with(answer(&a_note()))
        .mount(&server)
        .await;
    let automatic = WeeklyNoteOptions {
        automatic: true,
        ..click()
    };
    let sunday_night = Utc.with_ymd_and_hms(2026, 11, 2, 4, 30, 0).unwrap();
    let err = app
        .write_weekly_note_at(sunday_night, "auto-sunday", automatic.clone(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::Invalid);
    assert!(server.received_requests().await.unwrap().is_empty());
    let monday_night = Utc.with_ymd_and_hms(2026, 11, 2, 5, 30, 0).unwrap();
    let note = app
        .write_weekly_note_at(monday_night, "auto-monday", automatic, |_| {})
        .await
        .unwrap();
    assert!(note.automatic);
    assert_eq!(note.week_of, NaiveDate::from_ymd_opt(2026, 11, 2).unwrap());
}
