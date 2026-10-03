//! The ChatGPT plan's build switch (`CHATGPT_PLAN_OFFERED`, off by default): PageLamp offers the
//! plan once OpenAI confirms in writing. Off, every way into Codex refuses with
//! `backend_disabled_in_this_build`, a Codex routing stored by an earlier build blocks instead of
//! running, the status says so in `chatgpt_plan_offered`, and clean-up still works. No process
//! is started. (`codex_api.rs` tests the plan with the switch on.)

use std::sync::Arc;

use pagelamp_app::ai::{
    BackendRef, CHATGPT_PLAN_OFFERED, CodexLoginMethod, CodexSource, EstimateRequest, ModelChoice,
};
use pagelamp_app::{App, AppError, AppErrorKind};
use pagelamp_core::ai::{AiFeature, BlockReason, Effort};
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_core::store::Store;
use std::collections::BTreeMap;

/// With the switch off, whatever `CHATGPT_PLAN_OFFERED` says (debug builds; the release build
/// is what the constant says).
fn app_in(dir: &std::path::Path) -> App {
    let app = App::open_at_with_secrets(dir.join("data"), Arc::new(MemorySecrets::new())).unwrap();
    app.set_chatgpt_plan_offered_for_tests(false);
    app
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
    Store::open(&app.db_path())
        .unwrap()
        .set_setting("ai.routing", &routing)
        .unwrap();

    let status = app.ai_status().unwrap();
    assert!(!status.chatgpt_plan_offered);
    assert!(
        status
            .backends
            .iter()
            .all(|b| b.backend != BackendRef::Codex),
        "no ChatGPT card"
    );
    let estimate = app
        .estimate_generation(&EstimateRequest::StudyPlan {
            horizon_days: None,
            courses: Vec::new(),
        })
        .unwrap();
    assert_eq!(
        estimate.would_block,
        Some(BlockReason::BackendDisabledInThisBuild)
    );

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
/// sign-in from it). It looks for a `codex` on PATH, so it only runs where there is none: these
/// tests never start a Codex found on this computer.
#[tokio::test]
async fn codex_status_still_answers_when_the_plan_is_not_offered() {
    let exe = if cfg!(windows) { "codex.exe" } else { "codex" };
    let on_path = std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(exe).is_file()));
    if on_path {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let app = app_in(temp.path());
    let status = app.codex_status().await.unwrap();
    assert!(!status.chatgpt_plan_offered);
    assert_eq!(
        status.login.state,
        pagelamp_app::ai::CodexLoginState::SignedOut
    );
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
