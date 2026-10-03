//! Mode A: the student's ChatGPT plan through the official Codex CLI (design §2.3, §3.8; plan M2).
//! The runtime, the sign-in and the runs themselves are `pagelamp_llm::codex`; this is the facade
//! around them: which Codex runs (the pinned one, or the student's own in the tested range, D12),
//! status for the ChatGPT card, the weekly run cap, the disclosure and the error texts.
//!
//! **The build switch.** PageLamp offers the ChatGPT plan once OpenAI confirms in writing that
//! this use is allowed (the owner's decision; the README says so). Until then
//! `CHATGPT_PLAN_OFFERED` is false: every way into Codex refuses with
//! `BackendDisabledInThisBuild` (install, sign-in, the Codex choices, the disclosure, models,
//! "Test", and runs, a Codex routing stored by an earlier build included), and `codex_status` and
//! `ai_status` say so in `chatgpt_plan_offered`, so the UIs hide the ChatGPT card and copy.
//! `codex_status` then looks for no Codex and starts none. Clean-up still works: `remove_codex`,
//! `codex_logout` (only when a Codex sign-in folder is there), `forget_codex` and "Remove all AI
//! data".

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{Datelike, Duration, Local, TimeZone, Utc};
use pagelamp_core::ai::{AiFeature, Effort, ModelErrorKind};
use pagelamp_core::ai_gate::{GatedContext, assemble};
use pagelamp_core::paths;
use pagelamp_llm::CancellationToken;
use pagelamp_llm::codex::{
    self, CodexError, CodexHome, Exec, ExecRequest, InstallError, InstallEvent, Runtime, Version,
    login,
};

use super::settings::{self, OutdatedSeen, WeeklyCap};
use super::{
    AdminVisibility, ChatGptPlanType, CodexLogin, CodexLoginMethod, CodexLoginState,
    CodexOutdatedAction, CodexRuntime, CodexRuntimeState, CodexSource, CodexStatus, CostKind,
    DisclosureFacts, LoginEvent, ModelInfo, ProbeReport, Recipient, RetentionFact, RuntimeEvent,
    SentData, StructuredOutputTier, SystemCodex, TrainingFact, with_version,
};
use crate::activity::ActivityKind;
use crate::{App, AppError, AppErrorKind, Result};

/// Whether this build offers the ChatGPT plan (see the module docs). Turning it on is the one
/// release-prep change once OpenAI's written confirmation is in.
pub const CHATGPT_PLAN_OFFERED: bool = false;

/// Mode A's weekly run cap unless the student sets another (design §2.3).
pub const DEFAULT_WEEKLY_CAP: u32 = 40;
/// The `backend` of Codex runs in the usage ledger.
pub(crate) const USAGE_BACKEND: &str = "codex";
/// How the ChatGPT plan is named in `ai_status`, run events and AI labels.
pub(crate) const CODEX_LABEL: &str = "ChatGPT plan (through OpenAI Codex)";

/// "Test": a structured answer, and no course data.
const PROBE_INSTRUCTIONS: &str = "This is a connection test from PageLamp. Answer with the JSON \
    object {\"ok\": true} and nothing else.";

/// What the app keeps about Codex while it runs.
#[derive(Default)]
pub(crate) struct CodexState {
    /// Installs in progress, by install id.
    installs: Mutex<HashMap<String, CancellationToken>>,
    /// The sign-in in progress.
    login: Mutex<Option<CancellationToken>>,
    /// The last sign-in state Codex reported (for `ai_status`, which starts no process).
    last_login: Mutex<Option<CodexLoginState>>,
    /// One Codex command at a time in this process (other processes: the CODEX_HOME lock).
    runs: tokio::sync::Mutex<()>,
    /// `set_chatgpt_plan_offered_for_tests` (debug builds only): 0 unset, 1 off, 2 on.
    offered_for_tests: std::sync::atomic::AtomicU8,
}

/// The binary a Codex command runs.
struct Binary {
    path: PathBuf,
    version: Version,
    source: CodexSource,
}

/// The local data folder of an app using `data_dir` (see `App::local_dir`).
pub(crate) fn local_dir_for(data_dir: &std::path::Path) -> PathBuf {
    match paths::platform_local_data_dir() {
        Some(local) if crate::same_dir(Some(data_dir), paths::platform_data_dir().as_deref()) => {
            local
        }
        _ => data_dir.to_path_buf(),
    }
}

/// The Codex versions PageLamp installed for an app using `data_dir`, newest first (versions
/// only, for the diagnostic report).
pub(crate) fn installed_versions(data_dir: &std::path::Path) -> Vec<String> {
    Runtime::new(paths::codex_runtime_dir_in(&local_dir_for(data_dir)))
        .installed()
        .into_iter()
        .map(|installed| installed.version.to_string())
        .collect()
}

impl App {
    /// Where large, non-roaming data goes (`paths::local_data_dir`): the platform's local folder
    /// when this app uses the default data folder, else the data folder itself.
    pub(crate) fn local_dir(&self) -> PathBuf {
        local_dir_for(self.data_dir())
    }

    pub(crate) fn codex_home(&self) -> CodexHome {
        CodexHome::new(paths::codex_home_in(&self.local_dir()))
    }

    fn codex_runtime(&self) -> Runtime {
        Runtime::new(paths::codex_runtime_dir_in(&self.local_dir()))
    }

    fn codex_state(&self) -> &CodexState {
        &self.state.codex
    }

    // ----- the build switch ---------------------------------------------------------------------

    /// Whether this build offers the ChatGPT plan (`CHATGPT_PLAN_OFFERED`).
    pub fn chatgpt_plan_offered(&self) -> bool {
        let set = self
            .codex_state()
            .offered_for_tests
            .load(std::sync::atomic::Ordering::Relaxed);
        match set {
            1 if cfg!(debug_assertions) => false,
            2 if cfg!(debug_assertions) => true,
            _ => CHATGPT_PLAN_OFFERED,
        }
    }

    /// Offer the ChatGPT plan in this app or not, whatever the build says, so tests cover both
    /// states whichever way `CHATGPT_PLAN_OFFERED` is set. Debug builds only: a release build
    /// ignores it.
    #[doc(hidden)]
    pub fn set_chatgpt_plan_offered_for_tests(&self, offered: bool) {
        self.codex_state().offered_for_tests.store(
            if offered { 2 } else { 1 },
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// `Err(Blocked(BackendDisabledInThisBuild))` unless this build offers the ChatGPT plan (the
    /// CLI checks it first, so nothing is printed before the refusal).
    pub fn require_chatgpt_plan(&self) -> Result<()> {
        if self.chatgpt_plan_offered() {
            return Ok(());
        }
        Err(not_offered())
    }

    // ----- status -------------------------------------------------------------------------------

    /// The ChatGPT-plan card: runtime (managed or the student's own), sign-in, the weekly cap.
    /// Starts `codex login status` and `codex --version` (both local, a few milliseconds). In a
    /// build that doesn't offer the plan, nothing is looked for or started (no PATH lookup, no
    /// `codex`, no Codex sign-in folder created): the status says so, the rest neutral (not
    /// installed, signed out, no Codex of the student's own).
    pub async fn codex_status(&self) -> Result<CodexStatus> {
        let pin = codex::pin();
        let target = codex::running_target();
        let asset = target.and_then(|t| pin.asset(t));
        let store = self.read_store()?;
        let source: CodexSource = store.setting(settings::CODEX_SOURCE)?.unwrap_or_default();
        let weekly_cap = weekly_cap(&store)?;
        let runs_this_week = runs_this_week(&store)?;
        let outdated: Option<OutdatedSeen> = store.setting(settings::CODEX_OUTDATED)?;
        drop(store);

        let offered = self.chatgpt_plan_offered();
        let (system, binary, login) = if offered {
            let system = self.detect_system_codex().await;
            let binary = self.codex_binary().await.ok();
            let login = match &binary {
                Some(binary) => self.login_state(binary).await,
                None => None,
            };
            (system, binary, login)
        } else {
            (None, None, None)
        };
        let state = if asset.is_none() {
            CodexRuntimeState::UnsupportedPlatform
        } else if binary.is_some() {
            CodexRuntimeState::Installed
        } else {
            CodexRuntimeState::NotInstalled
        };
        let outdated_action = if offered {
            outdated_action(
                outdated.as_ref().map(|seen| seen.version.as_str()),
                binary.as_ref().map(|b| (&b.version, b.source)),
                &pin.version(),
            )
        } else {
            CodexOutdatedAction::None
        };
        Ok(CodexStatus {
            chatgpt_plan_offered: offered,
            runtime: CodexRuntime {
                state,
                source,
                installed_version: binary.as_ref().map(|b| b.version.to_string()),
                pinned_version: pin.version.clone(),
                download_bytes: asset.map_or(0, |a| a.size),
                untested_platform: target == Some("aarch64-pc-windows-msvc"),
            },
            outdated_action,
            login: CodexLogin {
                state: login.unwrap_or(CodexLoginState::SignedOut),
                plan_type: (login == Some(CodexLoginState::Chatgpt))
                    .then_some(ChatGptPlanType::Unknown),
            },
            exec_available: None,
            weekly_cap,
            runs_this_week,
            system_codex: system.map(|(_, version)| SystemCodex {
                in_tested_range: pin.in_tested_range(&version),
                version: version.to_string(),
            }),
        })
    }

    /// `codex login status`, remembered for `ai_status`. `None` when Codex is busy or failed.
    async fn login_state(&self, binary: &Binary) -> Option<CodexLoginState> {
        let _one_at_a_time = self.codex_state().runs.lock().await;
        let state = match login::status(&binary.path, &self.codex_home()).await {
            Ok(login::LoginState::SignedOut) => CodexLoginState::SignedOut,
            Ok(login::LoginState::ChatGpt) => CodexLoginState::Chatgpt,
            Ok(login::LoginState::ApiKey) => CodexLoginState::ApiKey,
            Err(_) => return *self.last_login(),
        };
        *self.last_login() = Some(state);
        Some(state)
    }

    fn last_login(&self) -> std::sync::MutexGuard<'_, Option<CodexLoginState>> {
        self.codex_state()
            .last_login
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The last sign-in state seen (no process started).
    pub(crate) fn codex_login_cached(&self) -> Option<CodexLoginState> {
        *self.last_login()
    }

    /// Whether a runtime is there to run (no process started): the managed pin, or the chosen
    /// system Codex.
    pub(crate) fn codex_installed(&self) -> bool {
        !self.codex_runtime().installed().is_empty()
    }

    /// The Codex to run: the student's own when chosen (and still in range), else the managed pin
    /// (the newest installed version, which may be older than the pin after an update).
    async fn codex_binary(&self) -> Result<Binary> {
        let source: CodexSource = self
            .read_store()?
            .setting(settings::CODEX_SOURCE)?
            .unwrap_or_default();
        if source == CodexSource::System {
            return match self.detect_system_codex().await {
                Some((path, version)) if codex::pin().in_tested_range(&version) => Ok(Binary {
                    path,
                    version,
                    source,
                }),
                _ => Err(AppError {
                    model_error: Some(ModelErrorKind::RuntimeMissing),
                    ..AppError::new(
                        AppErrorKind::Model,
                        "Your installed Codex is missing or not a version PageLamp tested. Use the \
                         Codex PageLamp installs instead.",
                    )
                }),
            };
        }
        match self.codex_runtime().installed().into_iter().next() {
            Some(installed) => Ok(Binary {
                path: installed.binary,
                version: installed.version,
                source,
            }),
            None => Err(AppError {
                model_error: Some(ModelErrorKind::RuntimeMissing),
                ..AppError::new(
                    AppErrorKind::Model,
                    "Codex isn't installed yet: set up \"Use my ChatGPT plan\" first.",
                )
            }),
        }
    }

    /// A `codex` on PATH and its version (D12). Only an absolute path found in PATH is run.
    async fn detect_system_codex(&self) -> Option<(PathBuf, Version)> {
        let path = find_on_path(if cfg!(windows) { "codex.exe" } else { "codex" })?;
        let version = login::version(&path, &self.codex_home()).await.ok()?;
        Some((path, version))
    }

    // ----- install --------------------------------------------------------------------------------

    /// Download, verify and install the pinned Codex (shows as activity `codex_install`).
    pub async fn install_codex(
        &self,
        install_id: &str,
        on_event: impl Fn(RuntimeEvent) + Send + Sync,
    ) -> Result<CodexStatus> {
        self.require_chatgpt_plan()?;
        let pin = codex::pin();
        let asset = codex::running_target()
            .and_then(|target| pin.asset(target))
            .ok_or_else(|| AppError {
                model_error: Some(ModelErrorKind::Unsupported),
                ..AppError::new(
                    AppErrorKind::Model,
                    "Codex isn't available for this computer.",
                )
            })?;
        let cancel = CancellationToken::new();
        self.codex_state()
            .installs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(install_id.to_string(), cancel.clone());
        let _activity = self.begin_activity(ActivityKind::CodexInstall, None);
        let result = self
            .codex_runtime()
            .install(pin, asset, install_id, &cancel, &|event| {
                on_event(runtime_event(event));
            })
            .await;
        self.codex_state()
            .installs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(install_id);
        result.map_err(install_error)?;
        // A newer runtime answers the "newer version" error.
        self.write_store()?
            .remove_setting(settings::CODEX_OUTDATED)?;
        self.codex_status().await
    }

    /// Stop an install; its partial download is deleted.
    pub fn cancel_codex_install(&self, install_id: &str) {
        if let Some(cancel) = self
            .codex_state()
            .installs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(install_id)
        {
            cancel.cancel();
        }
    }

    /// Delete the managed runtimes (the sign-in in PageLamp's CODEX_HOME stays; `codex_logout`
    /// ends it).
    pub fn remove_codex(&self) -> Result<()> {
        self.codex_runtime().remove_all().map_err(install_error)
    }

    // ----- sign-in --------------------------------------------------------------------------------

    /// Sign in through Codex itself (browser, or a one-time code).
    pub async fn codex_login(
        &self,
        method: CodexLoginMethod,
        on_event: impl Fn(LoginEvent) + Send + Sync,
    ) -> Result<CodexStatus> {
        self.require_chatgpt_plan()?;
        let binary = self.codex_binary().await?;
        let cancel = CancellationToken::new();
        *self
            .codex_state()
            .login
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(cancel.clone());
        let method = match method {
            CodexLoginMethod::Browser => login::LoginMethod::Browser,
            CodexLoginMethod::DeviceCode => login::LoginMethod::DeviceCode,
        };
        let result = {
            let _one_at_a_time = self.codex_state().runs.lock().await;
            login::login(
                &binary.path,
                &self.codex_home(),
                method,
                &cancel,
                &|event| {
                    on_event(login_event(event));
                },
            )
            .await
        };
        *self
            .codex_state()
            .login
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        result.map_err(codex_error)?;
        self.codex_status().await
    }

    /// Stop a sign-in in progress.
    pub fn cancel_codex_login(&self) {
        if let Some(cancel) = self
            .codex_state()
            .login
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            cancel.cancel();
        }
    }

    /// `codex logout` (Codex revokes and deletes its own credentials). In a build that doesn't
    /// offer the plan it is clean-up: with no Codex sign-in folder there is nothing to sign out
    /// of and nothing is started; with one, the Codex "Remove all AI data" would use signs out
    /// (never asked for its version).
    pub async fn codex_logout(&self) -> Result<CodexStatus> {
        let binary = if self.chatgpt_plan_offered() {
            Some(self.codex_binary().await?.path)
        } else if self.codex_home().dir().exists() {
            Some(self.cleanup_binary().ok_or_else(|| AppError {
                model_error: Some(ModelErrorKind::RuntimeMissing),
                ..AppError::new(
                    AppErrorKind::Model,
                    "There's no Codex to sign out with; \"Remove all AI data\" deletes its sign-in.",
                )
            })?)
        } else {
            None
        };
        if let Some(binary) = binary {
            let _one_at_a_time = self.codex_state().runs.lock().await;
            login::logout(&binary, &self.codex_home())
                .await
                .map_err(codex_error)?;
        }
        *self.last_login() = Some(CodexLoginState::SignedOut);
        self.codex_status().await
    }

    /// The Codex clean-up signs out with: the newest managed runtime, else a `codex` on PATH
    /// (never asked for its version).
    fn cleanup_binary(&self) -> Option<PathBuf> {
        self.codex_runtime()
            .installed()
            .into_iter()
            .next()
            .map(|installed| installed.binary)
            .or_else(|| find_on_path(if cfg!(windows) { "codex.exe" } else { "codex" }))
    }

    /// "Remove all AI data": `codex logout` (Codex revokes and deletes its own credentials), then
    /// PageLamp's `CODEX_HOME` and run folders are deleted. The runtime stays (`remove_codex`).
    /// `Busy` (nothing deleted) while Codex is running in any window.
    pub(crate) fn forget_codex(&self) -> Result<()> {
        let home = self.codex_home();
        if home.dir().exists() {
            if let Some(binary) = self.cleanup_binary() {
                // Signed out, or Codex can't run: the folder goes either way; Busy stops here.
                if let Err(CodexError::Busy) = login::logout_blocking(&binary, &home) {
                    return Err(codex_error(CodexError::Busy));
                }
            }
            std::fs::remove_dir_all(home.dir()).map_err(|err| {
                AppError::new(
                    AppErrorKind::Internal,
                    format!("could not delete Codex's folder: {err}"),
                )
            })?;
        }
        let runs = paths::ai_runs_dir_in(&self.local_dir());
        if runs.exists() {
            let _ = std::fs::remove_dir_all(runs);
        }
        *self.last_login() = None;
        Ok(())
    }

    // ----- choices --------------------------------------------------------------------------------

    /// Mode A's weekly run cap (`None`: no cap).
    pub fn set_mode_a_weekly_cap(&self, runs: Option<u32>) -> Result<()> {
        self.require_chatgpt_plan()?;
        self.write_store()?
            .set_setting(settings::MODE_A_WEEKLY_CAP, &WeeklyCap { runs })?;
        Ok(())
    }

    /// Run the managed pin, or the student's own Codex when its version is in the tested range.
    pub async fn set_codex_source(&self, source: CodexSource) -> Result<CodexStatus> {
        self.require_chatgpt_plan()?;
        if source == CodexSource::System {
            let pin = codex::pin();
            match self.detect_system_codex().await {
                None => {
                    return Err(AppError::new(
                        AppErrorKind::Invalid,
                        "No Codex was found on this computer (a `codex` command on PATH).",
                    ));
                }
                Some((_, version)) if !pin.in_tested_range(&version) => {
                    return Err(AppError::new(
                        AppErrorKind::Invalid,
                        format!(
                            "Your Codex is version {version}; PageLamp is tested with {} up to, \
                             not including, {}. Use the Codex PageLamp installs, or update yours.",
                            pin.tested_min, pin.tested_below
                        ),
                    ));
                }
                Some(_) => {}
            }
        }
        self.write_store()?
            .set_setting(settings::CODEX_SOURCE, &source)?;
        self.codex_status().await
    }

    // ----- models, test, disclosure, estimate --------------------------------------------------------

    /// The models a run may name (the pin's list).
    pub(crate) fn codex_models(&self) -> Vec<ModelInfo> {
        let pin = codex::pin();
        pin.models
            .supported
            .iter()
            .map(|id| ModelInfo {
                id: id.clone(),
                label: None,
                on_device: false,
                runs_in_cloud: false,
                // Runs count against the plan: no price, no money budget.
                price_known: false,
                context_window: None,
                reasoning_always_on: false,
                suggested_for: AiFeature::ALL
                    .into_iter()
                    .filter(|f| pin.models.default.get(*f) == id)
                    .collect(),
            })
            .collect()
    }

    /// "Test" with Codex: a structured answer, no course data. Counts as one run of the plan.
    pub(crate) async fn codex_test(&self, model: &str) -> Result<ProbeReport> {
        self.require_chatgpt_plan()?;
        check_codex_model(model)?;
        let binary = self.codex_binary().await?;
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "ok": { "type": "boolean" } },
            "required": ["ok"],
            "additionalProperties": false
        });
        let prompt = assemble(PROBE_INSTRUCTIONS, &GatedContext::empty(), None);
        let run_id = format!("probe-{}", std::process::id());
        let exec = self.codex_exec(&binary);
        let started = std::time::Instant::now();
        let request = ExecRequest {
            prompt: &prompt,
            model,
            effort: Effort::Lowest,
            output_schema: Some(&schema),
            run_id: &run_id,
        };
        let result = {
            let _one_at_a_time = self.codex_state().runs.lock().await;
            exec.run_with_fallback(&request, None, &CancellationToken::new())
                .await
        };
        let latency_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
        let failed = |kind| ProbeReport {
            ok: false,
            latency_ms,
            structured_output_tier: None,
            thinking_always_on: false,
            error: Some(kind),
        };
        Ok(match result {
            Ok(outcome) => {
                let valid = serde_json::from_str::<serde_json::Value>(&outcome.text)
                    .is_ok_and(|answer| answer["ok"] == true);
                if valid {
                    ProbeReport {
                        ok: true,
                        latency_ms,
                        structured_output_tier: Some(StructuredOutputTier::NativeSchema),
                        thinking_always_on: false,
                        error: None,
                    }
                } else {
                    failed(ModelErrorKind::BadOutput)
                }
            }
            Err(CodexError::Failed { kind }) => {
                self.note_outdated(kind, &binary)?;
                failed(kind)
            }
            Err(CodexError::Stopped) => failed(ModelErrorKind::BadOutput),
            Err(other) => return Err(codex_error(other)),
        })
    }

    /// One feature run through Codex (`run::App::run_model`): the pin's model for the feature
    /// with its fallback, one Codex command at a time, the answer's JSON parsed (Codex holds it
    /// to the schema), and one run of the weekly cap in the usage ledger.
    pub(crate) async fn codex_run(
        &self,
        request: &super::run::RunRequest<'_>,
        on_event: &(dyn Fn(super::GenEvent) + Send + Sync),
        cancel: &CancellationToken,
    ) -> Result<super::run::RunOutcome> {
        let model = request.choice.model.as_str();
        check_codex_model(model)?;
        let binary = self.codex_binary().await?;
        let exec = self.codex_exec(&binary);
        let schema = match &request.output {
            pagelamp_llm::OutputSpec::Json { schema, .. } => Some(schema),
            pagelamp_llm::OutputSpec::Text => None,
        };
        on_event(super::GenEvent::Started {
            generation_id: request.generation_id.to_string(),
            backend_label: CODEX_LABEL.to_string(),
            model: model.to_string(),
            on_device: false,
        });
        on_event(super::GenEvent::Stage {
            stage: super::GenStage::WaitingForModel,
        });
        let fallback = codex::pin().models.fallback.get(request.feature);
        let run_folder = super::run::run_folder_name(request.generation_id);
        let exec_request = ExecRequest {
            prompt: &request.prompt,
            model,
            effort: request.choice.effort,
            output_schema: schema,
            run_id: &run_folder,
        };
        let result = {
            let _one_at_a_time = self.codex_state().runs.lock().await;
            exec.run_with_fallback(
                &exec_request,
                (fallback != model).then_some(fallback),
                cancel,
            )
            .await
        };
        let plan = super::run::Billing::Plan;
        match result {
            Ok(outcome) => {
                let cost = self.record_run_usage(
                    USAGE_BACKEND,
                    &outcome.model,
                    request.feature,
                    outcome.usage,
                    plan,
                    "ok",
                )?;
                on_event(super::GenEvent::Usage { usage: cost.usage });
                let json = match schema {
                    Some(_) => Some(serde_json::from_str(&outcome.text).map_err(|_| {
                        model_error(
                            ModelErrorKind::BadOutput,
                            "Codex's answer wasn't the JSON PageLamp asked for.",
                        )
                    })?),
                    None => None,
                };
                Ok(super::run::RunOutcome {
                    json,
                    backend_label: CODEX_LABEL.to_string(),
                    model: outcome.model,
                    on_device: false,
                    cost,
                })
            }
            Err(err) => {
                // Busy and a runtime that couldn't start never reached OpenAI: no run to count.
                if !matches!(err, CodexError::Busy | CodexError::Start(_)) {
                    let outcome = if matches!(err, CodexError::Cancelled) {
                        "cancelled"
                    } else {
                        "failed"
                    };
                    self.record_run_usage(
                        USAGE_BACKEND,
                        model,
                        request.feature,
                        pagelamp_llm::Usage::default(),
                        plan,
                        outcome,
                    )?;
                }
                if let CodexError::Failed { kind } = err {
                    self.note_outdated(kind, &binary)?;
                }
                Err(codex_error(err))
            }
        }
    }

    fn codex_exec(&self, binary: &Binary) -> Exec {
        Exec {
            binary: binary.path.clone(),
            home: self.codex_home(),
            runs_dir: paths::ai_runs_dir_in(&self.local_dir()),
        }
    }

    /// Remember a "requires a newer version of Codex" error for `codex_status`.
    fn note_outdated(&self, kind: ModelErrorKind, binary: &Binary) -> Result<()> {
        if kind == ModelErrorKind::RuntimeOutdated {
            self.write_store()?.set_setting(
                settings::CODEX_OUTDATED,
                &OutdatedSeen {
                    version: binary.version.to_string(),
                    at: Utc::now(),
                },
            )?;
        }
        Ok(())
    }

    /// The plan's runs this week and its cap, once Codex is installed or chosen.
    pub(crate) fn mode_a_usage(
        &self,
        store: &pagelamp_core::store::Store,
    ) -> Result<Option<super::ModeAUsage>> {
        let chosen = settings::routing(store)?
            .0
            .values()
            .any(|choice| choice.backend == super::BackendRef::Codex);
        if !chosen && !self.codex_installed() {
            return Ok(None);
        }
        Ok(Some(super::ModeAUsage {
            runs_this_week: runs_this_week(store)?,
            weekly_cap: weekly_cap(store)?,
        }))
    }

    /// Whether the weekly cap stops another run now.
    pub(crate) fn codex_cap_reached(&self, store: &pagelamp_core::store::Store) -> Result<bool> {
        Ok(match weekly_cap(store)? {
            Some(cap) => runs_this_week(store)? >= cap,
            None => false,
        })
    }
}

/// What a "requires a newer version of Codex" error means now (design §2.3). `seen_with`: the
/// version that gave it; `running`: the Codex that runs now. The pin can help only when the
/// managed runtime that failed is older than the pin; otherwise only a PageLamp update brings a
/// newer Codex. An error seen with another version (e.g. before an install) no longer applies.
fn outdated_action(
    seen_with: Option<&str>,
    running: Option<(&Version, CodexSource)>,
    pin: &Version,
) -> CodexOutdatedAction {
    match (seen_with, running) {
        (Some(seen), Some((version, source))) if version.to_string() == seen => {
            if source == CodexSource::Managed && version < pin {
                CodexOutdatedAction::InstallPin
            } else {
                CodexOutdatedAction::UpdatePagelamp
            }
        }
        _ => CodexOutdatedAction::None,
    }
}

/// The disclosure facts of the ChatGPT plan through Codex (design §2.3).
pub(crate) fn codex_disclosure(plan: Option<ChatGptPlanType>) -> DisclosureFacts {
    with_version(DisclosureFacts {
        version: 0,
        sends: vec![SentData::Structure, SentData::MaterialText],
        recipient: Recipient {
            name: "OpenAI (your ChatGPT plan, through Codex)".to_string(),
            terms_url: Some("https://openai.com/policies/row-terms-of-use/".to_string()),
        },
        // Plus and Pro conversations may train OpenAI's models unless turned off.
        training: TrainingFact::MayTrain {
            how_to_turn_off_url: None,
        },
        retention: RetentionFact::ProviderTerms,
        admin_visibility: match plan {
            Some(
                ChatGptPlanType::Edu | ChatGptPlanType::Enterprise | ChatGptPlanType::Business,
            ) => AdminVisibility::Yes,
            Some(
                ChatGptPlanType::Free
                | ChatGptPlanType::Go
                | ChatGptPlanType::Plus
                | ChatGptPlanType::Pro,
            ) => AdminVisibility::No,
            Some(ChatGptPlanType::Unknown) | None => AdminVisibility::Unknown,
        },
        min_age: Some(13),
        guardian_permission: true,
        cost: CostKind::PlanCredits,
        on_device: false,
        location: None,
    })
}

/// A model a run may name (`Invalid` otherwise: runs never use the account's default).
pub(crate) fn check_codex_model(model: &str) -> Result<()> {
    if codex::pin().is_supported_model(model) {
        Ok(())
    } else {
        Err(AppError::new(
            AppErrorKind::Invalid,
            format!(
                "PageLamp's Codex runs these models: {}.",
                codex::pin().models.supported.join(", ")
            ),
        ))
    }
}

fn weekly_cap(store: &pagelamp_core::store::Store) -> Result<Option<u32>> {
    Ok(store
        .setting::<WeeklyCap>(settings::MODE_A_WEEKLY_CAP)?
        .map_or(Some(DEFAULT_WEEKLY_CAP), |cap| cap.runs))
}

/// Codex runs since Monday (local time) in the usage ledger.
fn runs_this_week(store: &pagelamp_core::store::Store) -> Result<u32> {
    let today = Local::now().date_naive();
    let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let from = Local
        .from_local_datetime(&monday.and_hms_opt(0, 0, 0).expect("midnight"))
        .earliest()
        .map_or_else(Utc::now, |t| t.with_timezone(&Utc));
    let runs = store
        .ai_usage_between(from, Utc::now() + Duration::days(1))?
        .iter()
        .filter(|record| record.backend == USAGE_BACKEND)
        .count();
    Ok(u32::try_from(runs).unwrap_or(u32::MAX))
}

/// The first `name` in PATH (an absolute path).
fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| is_file(candidate))
}

fn is_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

fn runtime_event(event: InstallEvent) -> RuntimeEvent {
    match event {
        InstallEvent::DownloadStarted { total_bytes } => {
            RuntimeEvent::DownloadStarted { total_bytes }
        }
        InstallEvent::Progress {
            downloaded_bytes,
            total_bytes,
        } => RuntimeEvent::Progress {
            downloaded_bytes,
            total_bytes,
        },
        InstallEvent::Verifying => RuntimeEvent::Verifying,
        InstallEvent::Installing => RuntimeEvent::Installing,
        InstallEvent::Done { version } => RuntimeEvent::Done { version },
    }
}

fn login_event(event: login::LoginEvent) -> LoginEvent {
    match event {
        login::LoginEvent::BrowserOpened { url } => LoginEvent::BrowserOpened { url },
        login::LoginEvent::DeviceCode {
            verification_url,
            user_code,
            expires_in_secs,
        } => LoginEvent::DeviceCode {
            verification_url,
            user_code,
            expires_in_secs,
        },
        login::LoginEvent::Waiting => LoginEvent::Waiting,
        login::LoginEvent::Done => LoginEvent::Done,
    }
}

fn model_error(kind: ModelErrorKind, message: &str) -> AppError {
    AppError {
        model_error: Some(kind),
        ..AppError::new(AppErrorKind::Model, message)
    }
}

/// Why nothing ChatGPT-plan works in this build (`CHATGPT_PLAN_OFFERED`).
pub(crate) fn not_offered() -> AppError {
    AppError::blocked(
        pagelamp_core::ai::BlockReason::BackendDisabledInThisBuild,
        "The ChatGPT plan isn't available in this version of PageLamp. Use an API key or a model \
         on this computer.",
    )
}

pub(crate) fn install_error(err: InstallError) -> AppError {
    match err {
        InstallError::Cancelled => AppError::cancelled(),
        InstallError::Busy => AppError::new(
            AppErrorKind::Busy,
            "Codex is being installed in another PageLamp window. Try again when it's done.",
        ),
        InstallError::UnsupportedPlatform => model_error(
            ModelErrorKind::Unsupported,
            "Codex isn't available for this computer.",
        ),
        InstallError::Network(detail) => AppError::new(
            AppErrorKind::Network,
            format!("The Codex download failed: {detail}"),
        ),
        InstallError::Verify(_) => model_error(
            ModelErrorKind::RuntimeVerifyFailed,
            "The downloaded Codex couldn't be verified, so it wasn't installed.",
        ),
        InstallError::Io(detail) => AppError::new(
            AppErrorKind::Internal,
            format!("Codex couldn't be installed: {detail}"),
        ),
    }
}

pub(crate) fn codex_error(err: CodexError) -> AppError {
    match err {
        CodexError::Busy => AppError::new(
            AppErrorKind::Busy,
            "Codex is in use by another PageLamp window or command. Try again when it has \
             finished.",
        ),
        CodexError::Cancelled => AppError::cancelled(),
        CodexError::Start(_) => model_error(
            ModelErrorKind::RuntimeMissing,
            "Codex couldn't be started (it may be blocked by antivirus or Smart App Control).",
        ),
        CodexError::Stopped => model_error(
            ModelErrorKind::BadOutput,
            "PageLamp stopped Codex: unexpected output — update PageLamp.",
        ),
        CodexError::Failed { kind } => model_error(kind, failure_text(kind)),
        CodexError::Io(detail) => AppError::new(
            AppErrorKind::Internal,
            format!("Codex couldn't be prepared: {detail}"),
        ),
    }
}

fn failure_text(kind: ModelErrorKind) -> &'static str {
    match kind {
        ModelErrorKind::RuntimeOutdated => {
            "This Codex is too old for the model: PageLamp needs a newer Codex."
        }
        ModelErrorKind::UsageLimit => "Your ChatGPT plan's usage limit is reached for now.",
        ModelErrorKind::NotSignedIn | ModelErrorKind::AuthRejected => {
            "Codex isn't signed in to ChatGPT."
        }
        ModelErrorKind::ModelNotFound => "Codex doesn't offer that model to this account.",
        ModelErrorKind::ContextTooLong => "Too much text for the model.",
        ModelErrorKind::RateLimited => "Too many requests; try again shortly.",
        ModelErrorKind::Network => "Codex couldn't reach OpenAI.",
        ModelErrorKind::Timeout => "Codex took too long.",
        ModelErrorKind::Overloaded => "OpenAI's service is overloaded or failing.",
        _ => "Codex couldn't answer.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_version_error_says_install_the_pin_or_update_pagelamp() {
        let v = |text: &str| Version::parse(text).unwrap();
        let pin = v("0.158.0");
        let action = |seen: Option<&str>, running: Option<(&str, CodexSource)>| {
            let running = running.map(|(version, source)| (v(version), source));
            outdated_action(seen, running.as_ref().map(|(v, s)| (v, *s)), &pin)
        };
        // An older managed runtime: the pin fixes it ("PageLamp needs a newer Codex; updating…").
        assert_eq!(
            action(Some("0.157.1"), Some(("0.157.1", CodexSource::Managed))),
            CodexOutdatedAction::InstallPin
        );
        // The pin itself, or the student's own Codex: only a PageLamp update helps.
        assert_eq!(
            action(Some("0.158.0"), Some(("0.158.0", CodexSource::Managed))),
            CodexOutdatedAction::UpdatePagelamp
        );
        assert_eq!(
            action(Some("0.158.2"), Some(("0.158.2", CodexSource::System))),
            CodexOutdatedAction::UpdatePagelamp
        );
        // Seen with a version that no longer runs (installed since), or never seen.
        assert_eq!(
            action(Some("0.157.1"), Some(("0.158.0", CodexSource::Managed))),
            CodexOutdatedAction::None
        );
        assert_eq!(
            action(None, Some(("0.158.0", CodexSource::Managed))),
            CodexOutdatedAction::None
        );
        assert_eq!(action(Some("0.158.0"), None), CodexOutdatedAction::None);
    }

    #[test]
    fn the_plan_disclosure_never_understates_admin_visibility() {
        assert_eq!(
            codex_disclosure(None).admin_visibility,
            AdminVisibility::Unknown
        );
        assert_eq!(
            codex_disclosure(Some(ChatGptPlanType::Unknown)).admin_visibility,
            AdminVisibility::Unknown
        );
        for plan in [
            ChatGptPlanType::Edu,
            ChatGptPlanType::Enterprise,
            ChatGptPlanType::Business,
        ] {
            assert_eq!(
                codex_disclosure(Some(plan)).admin_visibility,
                AdminVisibility::Yes
            );
        }
        assert_eq!(
            codex_disclosure(Some(ChatGptPlanType::Plus)).admin_visibility,
            AdminVisibility::No
        );
        // The facts differ, so a known plan type asks for the disclosure again.
        assert_ne!(
            codex_disclosure(None).version,
            codex_disclosure(Some(ChatGptPlanType::Edu)).version
        );
    }
}
