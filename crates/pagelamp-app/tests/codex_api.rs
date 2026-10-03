//! The ChatGPT plan (mode A) in the facade, without starting any process: routing, the
//! disclosure, `ai_status`, the estimate with its weekly run cap, and the errors before Codex is
//! installed. (The runtime, sign-in and runs are tested against a fake Codex in pagelamp-llm;
//! these tests never run a `codex` found on this computer.)

use std::sync::Arc;

use chrono::Utc;
use pagelamp_app::ai::{
    AdminVisibility, BackendProblem, BackendRef, BackendState, CostKind, DEFAULT_WEEKLY_CAP,
    EstimateRequest, ModelChoice,
};
use pagelamp_app::{App, AppErrorKind};
use pagelamp_core::ai::{
    AiFeature, BlockReason, Effort, MaterialSharing, ModelErrorKind, UsageRecord,
};
use pagelamp_core::model::*;
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_core::store::Store;
use serde_json::json;

const COURSE: &str = "folder:demo/course/DEMO101";

/// With the ChatGPT plan offered (this build switch is off by default: `chatgpt_plan_api.rs`).
fn app_in(dir: &std::path::Path) -> App {
    let app = App::open_at_with_secrets(dir.join("data"), Arc::new(MemorySecrets::new())).unwrap();
    app.set_chatgpt_plan_offered_for_tests(true);
    app
}

/// DEMO101 with a readable week-3 material.
fn seed_course(app: &App) {
    let store = Store::open(&app.db_path()).unwrap();
    store
        .upsert_source(&SourceRecord {
            id: "folder:demo".into(),
            kind: SourceKind::Folder,
            label: "Demo".into(),
            config: json!({ "path": "/demo/courses" }),
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })
        .unwrap();
    store
        .upsert_course(&CourseUpsert {
            id: COURSE.into(),
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
    let material = format!("{COURSE}/material/slides");
    store
        .upsert_material(&MaterialUpsert {
            id: material.clone(),
            course_id: COURSE.into(),
            module_id: None,
            kind: MaterialKind::File,
            title: "Week 3 slides".into(),
            url: None,
            local_path: None,
            mime: None,
            published_at: None,
            week_hint: Some(3),
        })
        .unwrap();
    store
        .set_text_state(&material, TextStatus::Ok, None, Some("hash"))
        .unwrap();
    store
        .replace_chunks(
            &material,
            &[Chunk {
                material_id: material.clone(),
                ord: 0,
                locator: None,
                text: "Stomata open in light. ".repeat(50),
            }],
        )
        .unwrap();
}

fn codex_choice(model: &str) -> ModelChoice {
    ModelChoice {
        backend: BackendRef::Codex,
        model: model.into(),
        effort: Effort::Lowest,
    }
}

fn codex_run() -> UsageRecord {
    UsageRecord {
        at: Utc::now(),
        backend: "codex".into(),
        model: "gpt-6-luna".into(),
        feature: AiFeature::WeeklyExplanation,
        input_uncached: 1000,
        cache_read: 0,
        cache_write: 0,
        output: 200,
        reasoning: None,
        micro_usd: None,
        cost_basis: "plan".into(),
        estimated: false,
        outcome: "ok".into(),
    }
}

#[tokio::test]
async fn codex_runs_name_only_pinned_models_and_need_no_price() {
    let temp = tempfile::tempdir().unwrap();
    let app = app_in(temp.path());
    let models = app.list_models(&BackendRef::Codex).await.unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert!(ids.contains(&"gpt-6-luna"), "{ids:?}");
    let luna = models.iter().find(|m| m.id == "gpt-6-luna").unwrap();
    assert_eq!(
        luna.suggested_for.len(),
        AiFeature::ALL.len(),
        "the default for every feature"
    );
    assert!(models.iter().all(|m| !m.on_device && !m.price_known));

    let err = app
        .set_feature_model(AiFeature::StudyPlan, Some(codex_choice("gpt-9-unreleased")))
        .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::Invalid);
    app.set_feature_model(AiFeature::StudyPlan, Some(codex_choice("gpt-6-luna")))
        .unwrap();
    let unpriced = app
        .acknowledge_unpriced_model(&BackendRef::Codex, "gpt-6-luna")
        .unwrap_err();
    assert_eq!(unpriced.kind, AppErrorKind::Invalid);

    // Not installed: a test says so before anything runs.
    let err = app
        .test_model(&BackendRef::Codex, "gpt-6-luna")
        .await
        .unwrap_err();
    assert_eq!(
        (err.kind, err.model_error),
        (AppErrorKind::Model, Some(ModelErrorKind::RuntimeMissing))
    );
}

#[tokio::test]
async fn status_shows_the_plan_first_with_a_disclosure_that_never_understates() {
    let temp = tempfile::tempdir().unwrap();
    let app = app_in(temp.path());
    // Nothing set up: the ChatGPT plan isn't listed.
    assert!(app.ai_status().unwrap().backends.is_empty());

    app.set_feature_model(
        AiFeature::WeeklyExplanation,
        Some(codex_choice("gpt-6-luna")),
    )
    .unwrap();
    let status = app.ai_status().unwrap();
    let codex = &status.backends[0];
    assert_eq!(codex.backend, BackendRef::Codex);
    assert_eq!(codex.state, BackendState::NeedsSetup);
    assert_eq!(
        codex.problems,
        [BackendProblem::RuntimeMissing, BackendProblem::NotSignedIn]
    );
    let facts = &codex.disclosure;
    assert_eq!(
        facts.admin_visibility,
        AdminVisibility::Unknown,
        "plan type unknown"
    );
    assert_eq!(facts.cost, CostKind::PlanCredits);
    assert!(!facts.on_device);
    // Its version is what the student acknowledges.
    app.acknowledge_ai_disclosure(&BackendRef::Codex, facts.version)
        .unwrap();
    assert_eq!(
        app.ai_status().unwrap().backends[0].disclosure_acknowledged,
        Some(facts.version)
    );
}

#[tokio::test]
async fn the_estimate_counts_tokens_and_the_weekly_cap_stops_runs() {
    let temp = tempfile::tempdir().unwrap();
    let app = app_in(temp.path());
    seed_course(&app);
    app.set_feature_model(
        AiFeature::WeeklyExplanation,
        Some(codex_choice("gpt-6-luna")),
    )
    .unwrap();
    let request = EstimateRequest::WeeklyExplanation {
        course: "DEMO101".into(),
        week: Some(3),
        include: Vec::new(),
    };
    let first = app.estimate_generation(&request).unwrap();
    assert_eq!(
        first.would_block,
        Some(BlockReason::DisclosureNotAcknowledged)
    );
    assert!(first.input_tokens > 0, "tokens are counted");
    assert_eq!((first.micro_usd_upper, first.price_known), (None, false));

    let version = app.ai_status().unwrap().backends[0].disclosure.version;
    app.acknowledge_ai_disclosure(&BackendRef::Codex, version)
        .unwrap();
    assert_eq!(app.estimate_generation(&request).unwrap().would_block, None);
    // Question (b): Codex runs in OpenAI's cloud.
    app.set_course_material_sharing("DEMO101", MaterialSharing::NotAllowed)
        .unwrap();
    assert_eq!(
        app.estimate_generation(&request).unwrap().would_block,
        Some(BlockReason::MaterialSharingNotAllowed)
    );
    app.set_course_material_sharing("DEMO101", MaterialSharing::NotSure)
        .unwrap();

    // The weekly cap (default 40) counts Codex runs since Monday.
    assert_eq!(DEFAULT_WEEKLY_CAP, 40);
    app.set_mode_a_weekly_cap(Some(2)).unwrap();
    let store = Store::open(&app.db_path()).unwrap();
    store.record_ai_usage(&codex_run()).unwrap();
    assert_eq!(app.estimate_generation(&request).unwrap().would_block, None);
    store.record_ai_usage(&codex_run()).unwrap();
    assert_eq!(
        app.estimate_generation(&request).unwrap().would_block,
        Some(BlockReason::WeeklyRunCapReached)
    );
    let usage = app.usage_summary(None).unwrap().mode_a.unwrap();
    assert_eq!((usage.runs_this_week, usage.weekly_cap), (2, Some(2)));
    app.set_mode_a_weekly_cap(None).unwrap();
    assert_eq!(
        app.estimate_generation(&request).unwrap().would_block,
        None,
        "no cap"
    );
}

#[tokio::test]
async fn removing_all_ai_data_forgets_codex_and_its_settings() {
    let temp = tempfile::tempdir().unwrap();
    let app = app_in(temp.path());
    app.set_mode_a_weekly_cap(Some(3)).unwrap();
    app.set_feature_model(AiFeature::StudyPlan, Some(codex_choice("gpt-6-luna")))
        .unwrap();
    // Codex's folder as a sign-in would leave it (nothing here runs Codex: no runtime).
    let codex_home = temp.path().join("data/codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    std::fs::write(codex_home.join("auth.json"), b"{}").unwrap();
    app.remove_all_ai_data().unwrap();
    assert!(!codex_home.exists());
    assert!(
        app.ai_status().unwrap().backends.is_empty(),
        "routing forgotten"
    );
    // The report never mentions Codex's folder.
    assert!(!app.diagnostic_report().unwrap().contains("codex-home"));
}
