//! Weekly explanations (model-access design §5.2) through the facade: a grounded explanation
//! from a local mock model (citations resolved, unknown handles and uncited paragraphs
//! dropped), the left-out list and "include", the output language, saved explanations and
//! staleness, the gate matrix with question (b), and bad output. Synthetic data only; no run
//! ever leaves this computer (a cloud backend is only used where the gate blocks first).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDate, SubsecRound, TimeDelta, Utc};
use pagelamp_app::ai::{
    BackendRef, ExplainOptions, GenEvent, GenStage, ModelChoice, OutputLanguage,
};
use pagelamp_app::{App, AppErrorKind};
use pagelamp_core::ai::{
    AiFeature, BlockReason, Effort, MaterialSharing, ModelErrorKind, ProviderRow,
};
use pagelamp_core::ai_gate::LeftOutReason;
use pagelamp_core::model::*;
use pagelamp_core::secrets::{MemorySecrets, SecretBackend};
use pagelamp_core::store::{GenerationStatus, Store};
use pagelamp_core::views::{self, AsOf};
use serde_json::json;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const SOURCE: &str = "canvas:lms.example.edu";

fn course_id(external: &str) -> String {
    format!("{SOURCE}/course/{external}")
}

fn day(offset: i64) -> NaiveDate {
    Local::now().date_naive() + TimeDelta::days(offset)
}

/// The Monday two weeks before this week's: a term that starts then is in week 3 today, whatever
/// the weekday. (`day(-14)` falls on a weekend on Saturdays and Sundays, and a weekend start
/// moves week one to the next Monday, so today would be week 2.)
fn week_three_start() -> NaiveDate {
    week_three_start_on(Local::now().date_naive())
}

fn week_three_start_on(today: NaiveDate) -> NaiveDate {
    today - TimeDelta::days(i64::from(today.weekday().num_days_from_monday()) + 14)
}

/// `week_three_start` holds on every day of the week, by the default week `explain_week(…,
/// None, …)` resolves (`views::week_materials`); 14 days back gives week 2 on a Saturday and a
/// Sunday, which is what failed on a weekend.
#[test]
fn the_fixture_term_is_in_week_three_every_day_of_the_week() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::open(&temp.path().join("pagelamp.db")).unwrap();
    store
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
    let monday = NaiveDate::from_ymd_opt(2026, 9, 28).unwrap();
    for offset in 0..7 {
        let today = monday + TimeDelta::days(offset);
        let at = AsOf::at(today.and_hms_opt(12, 0, 0).unwrap().and_utc(), today);
        let week = |start: NaiveDate| {
            store
                .upsert_course(&CourseUpsert {
                    id: course_id("101"),
                    source_id: SOURCE.into(),
                    external_id: "101".into(),
                    code: Some("DEMO101".into()),
                    name: "Demo course 101".into(),
                    term_start: Some(start),
                    term_end: Some(today + TimeDelta::days(90)),
                    url: None,
                    syllabus_text: None,
                    lms: Default::default(),
                })
                .unwrap();
            views::week_materials(&store, "DEMO101", None, true, at)
                .unwrap()
                .week
        };
        assert_eq!(week(week_three_start_on(today)), Some(3), "{today}");
        let weekend = today.weekday().num_days_from_monday() >= 5;
        assert_eq!(
            week(today - TimeDelta::days(14)),
            Some(if weekend { 2 } else { 3 }),
            "{today}"
        );
    }
}

fn material(store: &Store, external: &str, name: &str, title: &str, text: &str) -> String {
    let id = format!("{SOURCE}/file/{external}-{name}");
    store
        .upsert_material(&MaterialUpsert {
            id: id.clone(),
            course_id: course_id(external),
            module_id: None,
            kind: MaterialKind::File,
            title: title.into(),
            url: Some(format!("https://lms.example.edu/files/{name}")),
            local_path: None,
            mime: None,
            published_at: None,
            week_hint: Some(3),
        })
        .unwrap();
    if !text.is_empty() {
        store
            .set_text_state(&id, TextStatus::Ok, None, Some("hash-1"))
            .unwrap();
        store
            .replace_chunks(
                &id,
                &[Chunk {
                    material_id: id.clone(),
                    ord: 0,
                    locator: Some("slide 2".into()),
                    text: text.into(),
                }],
            )
            .unwrap();
    }
    id
}

/// Every course teaches now; week 3 is the one explained. DEMO101 readable (slides and an
/// assignment sheet), DEMO202 prohibited, DEMO303 turned off, DEMO404 hidden, DEMO505 readable
/// and "not allowed" (question (b)), DEMO606 readable without text.
fn app_with_courses(dir: &std::path::Path) -> (App, Arc<MemorySecrets>) {
    let secrets = Arc::new(MemorySecrets::new());
    let app = App::open_at_with_secrets(dir.join("data"), secrets.clone()).unwrap();
    let store = Store::open(&app.db_path()).unwrap();
    store
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
    for external in ["101", "202", "303", "404", "505", "606"] {
        store
            .upsert_course(&CourseUpsert {
                id: course_id(external),
                source_id: SOURCE.into(),
                external_id: external.into(),
                code: Some(format!("DEMO{external}")),
                name: format!("Demo course {external}"),
                term_start: Some(week_three_start()),
                term_end: Some(day(90)),
                url: None,
                syllabus_text: None,
                lms: Default::default(),
            })
            .unwrap();
    }
    material(
        &store,
        "101",
        "slides",
        "Week 3 slides",
        "Stomata open in light.",
    );
    material(
        &store,
        "101",
        "a2",
        "Assignment 2",
        "Question 1: explain guard cells.",
    );
    for external in ["202", "303", "404", "505"] {
        material(
            &store,
            external,
            "slides",
            "Week 3 slides",
            &format!("Canary {external} text."),
        );
    }
    material(&store, "606", "slides", "Week 3 slides", "");
    drop(store);
    app.set_course_policy("DEMO202", AiPolicy::Prohibited, None)
        .unwrap();
    app.set_course_ai_access("DEMO303", false).unwrap();
    app.set_course_hidden("DEMO404", true).unwrap();
    app.set_course_material_sharing("DEMO505", MaterialSharing::NotAllowed)
        .unwrap();
    (app, secrets)
}

/// A model on this computer (an Ollama mock), chosen for explanations and disclosed.
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
    choose(app, backend.clone());
    acknowledge(app, &backend);
    server
}

fn choose(app: &App, backend: BackendRef) {
    app.set_feature_model(
        AiFeature::WeeklyExplanation,
        Some(ModelChoice {
            backend,
            model: "local-model".into(),
            effort: Effort::Lowest,
        }),
    )
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
               "done_reason": "stop", "prompt_eval_count": 500, "eval_count": 120}),
    ];
    let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/x-ndjson")
        .set_body_string(body)
}

fn explanation() -> serde_json::Value {
    json!({
        "sections": [
            {"heading": "Stomata", "paragraphs": [
                {"text": "Stomata open in light.", "citations": ["c1", "c7"]},
                {"text": "An invented fact.", "citations": ["c9"]},
                {"text": "Guard cells swell.", "citations": ["c1", "c1"]}
            ]},
            {"heading": "Uncited", "paragraphs": [
                {"text": "Nothing to back this.", "citations": []}
            ]}
        ],
        "check_questions": ["Why do stomata open?", "What do guard cells do?", " ", "Q3?", "Q4?"]
    })
}

fn collect() -> (Arc<Mutex<Vec<GenEvent>>>, impl Fn(GenEvent) + Send + Sync) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    (events, move |event| sink.lock().unwrap().push(event))
}

async fn sent(server: &MockServer, n: usize) -> String {
    String::from_utf8(server.received_requests().await.unwrap()[n].body.clone()).unwrap()
}

#[tokio::test]
async fn an_explanation_is_grounded_in_the_week_s_materials() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&explanation()))
        .mount(&server)
        .await;
    let (events, on_event) = collect();
    let result = app
        .explain_week(
            "DEMO101",
            Some(3),
            "explain-1",
            ExplainOptions {
                ui_language: Some("zh-CN".into()),
                ..ExplainOptions::default()
            },
            on_event,
        )
        .await
        .unwrap();

    // Every paragraph cites a material of the week; made-up handles and uncited paragraphs go.
    assert_eq!(result.week, Some(3));
    let paragraphs: Vec<(&str, Vec<&str>)> = result
        .sections
        .iter()
        .flat_map(|s| &s.paragraphs)
        .map(|p| {
            (
                p.text.as_str(),
                p.citations.iter().map(|c| c.handle.as_str()).collect(),
            )
        })
        .collect();
    assert_eq!(
        paragraphs,
        [
            ("Stomata open in light.", vec!["c1"]),
            ("Guard cells swell.", vec!["c1"])
        ]
    );
    assert_eq!(
        result.sections.len(),
        1,
        "a section left without paragraphs goes"
    );
    let citation = &result.sections[0].paragraphs[0].citations[0];
    assert_eq!(
        (
            citation.material_id.as_str(),
            citation.title.as_str(),
            citation.locator.as_deref(),
            citation.url.as_deref()
        ),
        (
            format!("{SOURCE}/file/101-slides").as_str(),
            "Week 3 slides",
            Some("slide 2"),
            Some("https://lms.example.edu/files/slides")
        )
    );
    assert_eq!(result.dropped_citations, 2);
    assert_eq!(
        result.check_questions.len(),
        3,
        "2–3 questions, empty ones dropped"
    );
    assert_eq!(result.left_out.len(), 1);
    assert_eq!(
        (
            result.left_out[0].title.as_str(),
            result.left_out[0].reason,
            result.left_out[0].includable
        ),
        ("Assignment 2", LeftOutReason::LooksLikeAssessment, true)
    );
    assert!(!result.stale && !result.sharing_reminder && !result.cite_ai_use);
    assert_eq!(result.meta.backend_label, "Ollama");

    // The stages the Explain tab shows.
    let events = events.lock().unwrap().clone();
    let context = events.iter().find_map(|e| match e {
        GenEvent::Context {
            summary,
            input_tokens,
        } => Some((summary.materials_included, *input_tokens)),
        _ => None,
    });
    assert!(
        matches!(context, Some((1, Some(tokens))) if tokens > 0),
        "{context:?}"
    );
    assert!(events.contains(&GenEvent::Stage {
        stage: GenStage::Validating
    }));
    assert_eq!(events.last(), Some(&GenEvent::Finished { ok: true }));

    // Policy golden: only the readable course's week text, the assignment sheet left out,
    // the answer's language as fixed wording.
    let body = sent(&server, 0).await;
    assert!(body.contains("Stomata open in light."));
    assert!(
        !body.contains("guard cells"),
        "the left-out assignment sheet"
    );
    for external in ["202", "303", "404", "505"] {
        assert!(!body.contains(&format!("Canary {external}")), "{external}");
    }
    assert!(body.contains("Simplified Chinese"));

    // "Include" sends the assignment sheet next time; the course language setting.
    app.set_ai_output_language(OutputLanguage::Course).unwrap();
    assert_eq!(app.ai_output_language().unwrap(), OutputLanguage::Course);
    let second = app
        .explain_week(
            "DEMO101",
            None,
            "explain-2",
            ExplainOptions {
                include: vec![format!("{SOURCE}/file/101-a2")],
                ..ExplainOptions::default()
            },
            |_| {},
        )
        .await
        .unwrap();
    // No week given: this week, which is week 3 every day of the week.
    assert_eq!(second.week, Some(3));
    assert!(second.left_out.is_empty());
    let body = sent(&server, 1).await;
    assert!(body.contains("guard cells"));
    assert!(body.contains("the language the course materials are written in"));
}

#[tokio::test]
async fn include_never_sends_a_material_without_readable_text() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&explanation()))
        .mount(&server)
        .await;
    // "Quiz 3" was read once and is not downloaded now (locked, its copy gone).
    let quiz = {
        let store = Store::open(&app.db_path()).unwrap();
        let id = material(&store, "101", "quiz3", "Quiz 3", "Quiz 3 answer key: 42.");
        store
            .set_text_state(&id, TextStatus::NotDownloaded, None, None)
            .unwrap();
        id
    };
    for (n, include) in [(0, vec![]), (1, vec![quiz.clone()])] {
        let result = app
            .explain_week(
                "DEMO101",
                Some(3),
                &format!("explain-{n}"),
                ExplainOptions {
                    include,
                    ..ExplainOptions::default()
                },
                |_| {},
            )
            .await
            .unwrap();
        // No text is decided before "looks like an assessment", and include can't lift it.
        let left = result
            .left_out
            .iter()
            .find(|left| left.material_id == quiz)
            .map(|left| (left.reason, left.includable));
        assert_eq!(left, Some((LeftOutReason::NoText, false)), "run {n}");
        assert!(!sent(&server, n).await.contains("answer key"), "run {n}");
    }
}

#[tokio::test]
async fn turning_a_course_s_ai_off_elsewhere_stops_its_explanation() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&explanation()).set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;
    let task = {
        let app = app.clone();
        tokio::spawn(async move {
            app.explain_week(
                "DEMO101",
                Some(3),
                "explain-off",
                ExplainOptions::default(),
                |_| {},
            )
            .await
        })
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while server.received_requests().await.unwrap().is_empty() {
        assert!(Instant::now() < deadline, "the run never reached the model");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Another window turns the course's AI access off while the model is being asked.
    app.set_course_ai_access("DEMO101", false).unwrap();
    let err = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::Cancelled);
}

#[tokio::test]
async fn saved_explanations_turn_stale_when_the_week_changes() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&explanation()))
        .mount(&server)
        .await;
    app.explain_week(
        "DEMO101",
        Some(3),
        "explain-1",
        ExplainOptions::default(),
        |_| {},
    )
    .await
    .unwrap();
    app.explain_week(
        "DEMO101",
        Some(3),
        "explain-2",
        ExplainOptions::default(),
        |_| {},
    )
    .await
    .unwrap();
    let saved = app.saved_explanations("DEMO101", Some(3)).unwrap();
    assert_eq!(
        saved
            .iter()
            .map(|e| (e.meta.generation_id.as_str(), e.stale))
            .collect::<Vec<_>>(),
        [("explain-2", false), ("explain-1", false)]
    );
    assert_eq!(app.saved_explanations("DEMO101", None).unwrap().len(), 2);
    assert!(
        app.saved_explanations("DEMO101", Some(4))
            .unwrap()
            .is_empty()
    );

    // The slides changed.
    let store = Store::open(&app.db_path()).unwrap();
    store
        .set_text_state(
            &format!("{SOURCE}/file/101-slides"),
            TextStatus::Ok,
            None,
            Some("hash-2"),
        )
        .unwrap();
    assert!(app.saved_explanations("DEMO101", Some(3)).unwrap()[0].stale);
    store
        .set_text_state(
            &format!("{SOURCE}/file/101-slides"),
            TextStatus::Ok,
            None,
            Some("hash-1"),
        )
        .unwrap();
    assert!(!app.saved_explanations("DEMO101", Some(3)).unwrap()[0].stale);
    // A new material that week.
    material(&store, "101", "notes", "Week 3 notes", "More notes.");
    assert!(app.saved_explanations("DEMO101", Some(3)).unwrap()[0].stale);
}

/// The gate matrix (design §4.1), question (b) included (D37 option 2).
#[tokio::test]
async fn explanations_are_blocked_for_the_course_s_reason() {
    let temp = tempfile::tempdir().unwrap();
    let (app, secrets) = app_with_courses(temp.path());
    let err = app
        .explain_week("DEMO101", Some(3), "x", ExplainOptions::default(), |_| {})
        .await
        .unwrap_err();
    assert_eq!(err.blocked, Some(BlockReason::NoModelChosen));
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&explanation()))
        .mount(&server)
        .await;
    for (course, reason) in [
        ("DEMO202", BlockReason::CoursePolicyProhibited),
        ("DEMO303", BlockReason::CourseAiTurnedOff),
        ("DEMO404", BlockReason::CourseHidden),
        ("DEMO606", BlockReason::NoReadableMaterials),
    ] {
        let err = app
            .explain_week(course, Some(3), "x", ExplainOptions::default(), |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.blocked, Some(reason), "{course}");
    }
    // "Not allowed" stays on this computer: a local model may read it.
    let local = app
        .explain_week(
            "DEMO505",
            Some(3),
            "local-505",
            ExplainOptions::default(),
            |_| {},
        )
        .await
        .unwrap();
    assert!(!local.sharing_reminder);
    // A cloud backend is refused before anything is sent.
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
    );
    let (events, on_event) = collect();
    let err = app
        .explain_week(
            "DEMO505",
            Some(3),
            "cloud-505",
            ExplainOptions::default(),
            on_event,
        )
        .await
        .unwrap_err();
    assert_eq!(err.blocked, Some(BlockReason::MaterialSharingNotAllowed));
    assert!(
        !events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, GenEvent::Started { .. })),
        "no run started"
    );
}

#[tokio::test]
async fn an_answer_citing_nothing_real_is_bad_output() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&json!({
            "sections": [{"heading": "Made up", "paragraphs": [
                {"text": "Invented.", "citations": ["c8", "c9"]}
            ]}],
            "check_questions": []
        })))
        .mount(&server)
        .await;
    let err = app
        .explain_week(
            "DEMO101",
            Some(3),
            "explain-bad",
            ExplainOptions::default(),
            |_| {},
        )
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind, err.model_error),
        (AppErrorKind::Model, Some(ModelErrorKind::BadOutput))
    );
    let row = Store::open(&app.db_path())
        .unwrap()
        .generation("explain-bad")
        .unwrap()
        .unwrap();
    assert_eq!(row.status, GenerationStatus::Failed);
    assert!(
        app.saved_explanations("DEMO101", Some(3))
            .unwrap()
            .is_empty()
    );
}

/// "Delete generated content per course" (design §8), then everything.
#[tokio::test]
async fn generated_content_is_deleted_per_course_or_all() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&explanation()))
        .mount(&server)
        .await;
    for (course, id) in [
        ("DEMO101", "e-101-a"),
        ("DEMO101", "e-101-b"),
        ("DEMO505", "e-505"),
    ] {
        app.explain_week(course, Some(3), id, ExplainOptions::default(), |_| {})
            .await
            .unwrap();
    }
    assert_eq!(app.delete_generated(Some("DEMO101")).unwrap(), 2);
    assert!(app.saved_explanations("DEMO101", None).unwrap().is_empty());
    assert_eq!(app.saved_explanations("DEMO505", None).unwrap().len(), 1);
    assert_eq!(app.delete_generated(None).unwrap(), 1);
    assert!(app.saved_explanations("DEMO505", None).unwrap().is_empty());
    assert_eq!(
        app.delete_generated(Some("NOPE999")).unwrap_err().kind,
        AppErrorKind::NotFound
    );
}

/// A calendar that moves the week makes a saved explanation stale (calendar design §7.8, B11),
/// and one explanation can be deleted from the history.
#[tokio::test]
async fn a_moved_week_makes_explanations_stale_and_one_can_be_deleted() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&explanation()))
        .mount(&server)
        .await;
    app.explain_week("DEMO101", Some(3), "e-1", ExplainOptions::default(), |_| {})
        .await
        .unwrap();
    app.explain_week("DEMO101", Some(3), "e-2", ExplainOptions::default(), |_| {})
        .await
        .unwrap();
    let store = Store::open(&app.db_path()).unwrap();
    assert!(
        store
            .generation("e-1")
            .unwrap()
            .unwrap()
            .week_starts_on
            .is_some()
    );
    assert!(!app.saved_explanations("DEMO101", Some(3)).unwrap()[0].stale);
    // The student's dates start the term a week earlier: week 3 is another week now.
    app.set_course_term(
        "DEMO101",
        Some(week_three_start() - TimeDelta::days(7)),
        Some(day(90)),
    )
    .unwrap();
    assert!(app.saved_explanations("DEMO101", Some(3)).unwrap()[0].stale);

    app.delete_explanation("e-1").unwrap();
    let left: Vec<String> = app
        .saved_explanations("DEMO101", None)
        .unwrap()
        .into_iter()
        .map(|e| e.meta.generation_id)
        .collect();
    assert_eq!(left, ["e-2"]);
    assert_eq!(
        app.delete_explanation("e-1").unwrap_err().kind,
        AppErrorKind::NotFound
    );
}

/// A failed read of the output language fails the explanation before anything is sent: never
/// an answer in a language the student didn't choose. Another version's shape is the default.
#[tokio::test]
async fn a_failed_read_of_the_output_language_sends_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let (app, _) = app_with_courses(temp.path());
    let server = with_local_model(&app).await;
    Mock::given(method("POST"))
        .respond_with(answer(&explanation()))
        .mount(&server)
        .await;
    app.set_ai_output_language(OutputLanguage::Course).unwrap();
    let raw = rusqlite::Connection::open(app.db_path()).unwrap();

    raw.execute(
        "UPDATE settings SET value = CAST(value AS BLOB) WHERE key = 'ai.output_language'",
        [],
    )
    .unwrap();
    assert!(app.ai_output_language().is_err());
    let result = app
        .explain_week(
            "DEMO101",
            Some(3),
            "explain-1",
            ExplainOptions::default(),
            |_| {},
        )
        .await;
    assert!(result.is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
    raw.execute(
        "UPDATE settings SET value = CAST(value AS TEXT) WHERE key = 'ai.output_language'",
        [],
    )
    .unwrap();
    assert_eq!(app.ai_output_language().unwrap(), OutputLanguage::Course);

    raw.execute(
        "UPDATE settings SET value = '\"klingon\"' WHERE key = 'ai.output_language'",
        [],
    )
    .unwrap();
    assert_eq!(app.ai_output_language().unwrap(), OutputLanguage::Ui);
}
