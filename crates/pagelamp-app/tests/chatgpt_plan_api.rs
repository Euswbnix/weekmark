//! The ChatGPT plan's build switch (`CHATGPT_PLAN_OFFERED`, off by default): PageLamp offers the
//! plan once OpenAI confirms in writing. Off, every way into Codex refuses with
//! `backend_disabled_in_this_build`, a Codex routing stored by an earlier build blocks instead of
//! running, the status says so in `chatgpt_plan_offered`, and clean-up still works. No process
//! is started. (`codex_api.rs` tests the plan with the switch on.)

use std::sync::Arc;

use chrono::{Local, TimeDelta};
use pagelamp_app::ai::{
    BackendRef, CHATGPT_PLAN_OFFERED, CodexLoginMethod, CodexSource, EstimateRequest, ModelChoice,
};
use pagelamp_app::{App, AppError, AppErrorKind};
use pagelamp_core::ai::{AiFeature, BlockReason, Effort};
use pagelamp_core::model::{CourseUpsert, SourceKind, SourceRecord};
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_core::store::Store;
use serde_json::json;
use std::collections::BTreeMap;

/// With the switch off, whatever `CHATGPT_PLAN_OFFERED` says (debug builds; the release build
/// is what the constant says).
fn app_in(dir: &std::path::Path) -> App {
    let app = App::open_at_with_secrets(dir.join("data"), Arc::new(MemorySecrets::new())).unwrap();
    app.set_chatgpt_plan_offered_for_tests(false);
    app
}

/// One course teaching now: something for a study plan to cover.
fn add_active_course(store: &Store) {
    let today = Local::now().date_naive();
    store
        .upsert_source(&SourceRecord {
            id: "canvas:lms.example.edu".into(),
            kind: SourceKind::Canvas,
            label: "lms.example.edu".into(),
            config: json!({ "base_url": "https://lms.example.edu" }),
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })
        .unwrap();
    store
        .upsert_course(&CourseUpsert {
            id: "canvas:lms.example.edu/course/101".into(),
            source_id: "canvas:lms.example.edu".into(),
            external_id: "101".into(),
            code: Some("DEMO101".into()),
            name: "Demo course 101".into(),
            term_start: Some(today - TimeDelta::days(14)),
            term_end: Some(today + TimeDelta::days(90)),
            url: None,
            syllabus_text: None,
            lms: Default::default(),
        })
        .unwrap();
}

fn codex_choice() -> ModelChoice {
    ModelChoice {
        backend: BackendRef::Codex,
        model: "gpt-6-luna".into(),
        effort: Effort::Lowest,
    }
}

fn not_offered<T: std::fmt::Debug>(result: Result<T, AppError>, what: &str) {
    let err = result.expect_err(what);
    assert_eq!(err.kind, AppErrorKind::Blocked, "{what}");
    assert_eq!(
        err.blocked,
        Some(BlockReason::BackendDisabledInThisBuild),
        "{what}"
    );
}

/// With no test setting, an app offers the plan exactly when the build does.
#[test]
fn an_app_follows_the_build_switch() {
    let temp = tempfile::tempdir().unwrap();
    let app = App::open_at_with_secrets(temp.path().join("data"), Arc::new(MemorySecrets::new()))
        .unwrap();
    assert_eq!(app.chatgpt_plan_offered(), CHATGPT_PLAN_OFFERED);
    assert_eq!(
        app.ai_status().unwrap().chatgpt_plan_offered,
        CHATGPT_PLAN_OFFERED
    );
}

#[tokio::test]
async fn every_way_into_codex_refuses_when_the_plan_is_not_offered() {
    let temp = tempfile::tempdir().unwrap();
    let app = app_in(temp.path());
    if !cfg!(debug_assertions) && CHATGPT_PLAN_OFFERED {
        return; // a release test run of a build that offers the plan: nothing to switch off
    }
    assert!(!app.chatgpt_plan_offered());
    assert!(!app.ai_status().unwrap().chatgpt_plan_offered);

    not_offered(
        app.set_feature_model(AiFeature::StudyPlan, Some(codex_choice())),
        "set_feature_model",
    );
    not_offered(
        app.acknowledge_ai_disclosure(&BackendRef::Codex, 1),
        "acknowledge_ai_disclosure",
    );
    not_offered(app.list_models(&BackendRef::Codex).await, "list_models");
    not_offered(
        app.test_model(&BackendRef::Codex, "gpt-6-luna").await,
        "test_model",
    );
    not_offered(
        app.install_codex("install-1", |_| {}).await,
        "install_codex",
    );
    not_offered(
        app.codex_login(CodexLoginMethod::DeviceCode, |_| {}).await,
        "codex_login",
    );
    not_offered(
        app.set_codex_source(CodexSource::System).await,
        "set_codex_source",
    );
    not_offered(app.set_mode_a_weekly_cap(Some(3)), "set_mode_a_weekly_cap");
    not_offered(app.require_chatgpt_plan(), "require_chatgpt_plan");
    // Nothing was stored on the way.
    assert!(
        app.ai_status()
            .unwrap()
            .features
            .iter()
            .all(|routing| routing.choice.is_none())
    );
}

/// A Codex routing an earlier (development) build stored blocks instead of running, and the
/// status lists no Codex backend; clean-up still works.
#[tokio::test]
async fn a_stored_codex_routing_blocks_and_clean_up_still_works() {
    if !cfg!(debug_assertions) && CHATGPT_PLAN_OFFERED {
        return; // a release test run of a build that offers the plan: nothing to switch off
    }
    let temp = tempfile::tempdir().unwrap();
    let app = app_in(temp.path());
    let routing = BTreeMap::from([(AiFeature::StudyPlan, codex_choice())]);
    let store = Store::open(&app.db_path()).unwrap();
    store.set_setting("ai.routing", &routing).unwrap();

    let status = app.ai_status().unwrap();
    assert!(!status.chatgpt_plan_offered);
    assert!(
        status
            .backends
            .iter()
            .all(|b| b.backend != BackendRef::Codex),
        "no ChatGPT card"
    );
    let plan = EstimateRequest::StudyPlan {
        horizon_days: None,
        courses: Vec::new(),
    };
    // What the plan covers is checked first, as for every feature: with no course, that's it.
    assert_eq!(
        app.estimate_generation(&plan).unwrap().would_block,
        Some(BlockReason::NoCourseToPlan)
    );
    add_active_course(&store);
    assert_eq!(
        app.estimate_generation(&plan).unwrap().would_block,
        Some(BlockReason::BackendDisabledInThisBuild)
    );
    drop(store);

    // Clean-up: the Codex this app may have installed, and everything AI.
    app.remove_codex().unwrap();
    app.remove_all_ai_data().unwrap();
    assert!(
        app.ai_status()
            .unwrap()
            .features
            .iter()
            .all(|routing| routing.choice.is_none())
    );
}

/// `codex_status` keeps answering with the switch off (the "Remove all AI data" dialog reads the
/// sign-in from it), and starts nothing: a managed Codex that notes every start is never run,
/// and no Codex sign-in folder appears. (It doesn't look on PATH either; the CLI test with a
/// Codex on PATH checks that.) Signing out is clean-up: with no sign-in folder nothing starts,
/// with one the managed Codex signs out.
#[tokio::test]
async fn codex_status_starts_nothing_when_the_plan_is_not_offered() {
    if !cfg!(debug_assertions) && CHATGPT_PLAN_OFFERED {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let app = app_in(temp.path());
    let data = temp.path().join("data");
    let mark = temp.path().join("codex-ran");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = data.join("runtimes").join("codex").join("0.1.0");
        std::fs::create_dir_all(&dir).unwrap();
        let codex = dir.join("codex");
        std::fs::write(
            &codex,
            format!("#!/bin/sh\necho \"$*\" >> '{}'\n", mark.display()),
        )
        .unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let started = || std::fs::read_to_string(&mark).unwrap_or_default();

    let status = app.codex_status().await.unwrap();
    assert!(!status.chatgpt_plan_offered);
    assert_eq!(
        status.login.state,
        pagelamp_app::ai::CodexLoginState::SignedOut
    );
    assert_eq!(status.system_codex, None);
    assert_eq!(status.runtime.installed_version, None);
    assert_eq!(
        status.outdated_action,
        pagelamp_app::ai::CodexOutdatedAction::None
    );
    assert_eq!(started(), "", "no Codex was started");
    assert!(!data.join("codex-home").exists());

    // Nothing to sign out of: nothing starts, no folder appears.
    app.codex_logout().await.unwrap();
    assert_eq!(started(), "");
    assert!(!data.join("codex-home").exists());
    // A sign-in folder left by an earlier build: the managed Codex signs out, nothing else runs.
    #[cfg(unix)]
    {
        std::fs::create_dir_all(data.join("codex-home")).unwrap();
        let status = app.codex_logout().await.unwrap();
        assert!(!status.chatgpt_plan_offered);
        assert_eq!(started(), "logout\n");
    }
}

/// The switch on (tests only): the same calls reach Codex's own checks again.
#[tokio::test]
async fn the_test_setting_offers_the_plan_in_a_debug_build() {
    let temp = tempfile::tempdir().unwrap();
    let app = app_in(temp.path());
    app.set_chatgpt_plan_offered_for_tests(true);
    assert_eq!(
        app.chatgpt_plan_offered(),
        cfg!(debug_assertions) || CHATGPT_PLAN_OFFERED
    );
    if cfg!(debug_assertions) {
        app.set_feature_model(AiFeature::StudyPlan, Some(codex_choice()))
            .unwrap();
        assert!(app.ai_status().unwrap().chatgpt_plan_offered);
    }
}
