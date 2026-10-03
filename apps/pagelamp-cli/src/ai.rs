//! `pagelamp ai …`: the model providers PageLamp itself calls (API keys and local servers),
//! which model each feature uses, the estimate, usage and the monthly budget. Thin wrappers
//! over the facade (`pagelamp_app::ai`): every rule — the gate, question (b), the disclosure,
//! the budget — is enforced there, so the CLI can't skip it. Keys are read like `canvas add`:
//! without echo, or from stdin when piped; never from arguments.

use chrono::{Datelike, NaiveDate};
use clap::{Subcommand, ValueEnum};
use pagelamp_app::App;
use pagelamp_app::ai::{
    AiBackendStatus, BackendRef, BackendState, CodexLoginMethod, CodexLoginState,
    CodexOutdatedAction, CodexRuntimeState, CodexSource, CodexStatus, CostBasis, CostEstimate,
    EstimateRequest, LoginEvent, ModelChoice, RuntimeEvent, StructuredOutputTier,
};
use pagelamp_core::ai::{AiFeature, Effort};
use pagelamp_core::brand::CLI_NAME;

use crate::{confirm, print_json, read_secret, text};

#[derive(Subcommand)]
pub enum AiCommand {
    /// Providers, the model each feature uses, and the budget (default).
    Status,
    /// The kinds of provider PageLamp can set up.
    Presets,
    /// Add a provider. Its key is read without echo (or from stdin when piped).
    Add {
        /// The kind of provider, e.g. openai, anthropic, ollama (see `ai presets`).
        preset: String,
        /// The provider's address: required for `custom`; for a local server on another port.
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Replace a provider's key (read like `add`).
    UpdateKey {
        /// The provider's id, as shown by `ai status`.
        provider: String,
    },
    /// Remove a provider and its key; or --all: every AI setting, key and usage record.
    Remove {
        /// The provider's id, as shown by `ai status`.
        #[arg(required_unless_present = "all", conflicts_with = "all")]
        provider: Option<String>,
        /// Remove all AI data: providers, keys, choices, acknowledgements, usage and the
        /// pre-update database backup.
        #[arg(long)]
        all: bool,
        /// With --all: don't ask first.
        #[arg(long)]
        yes: bool,
    },
    /// The models a provider offers (asks the provider; free).
    Models {
        /// The provider's id, as shown by `ai status`.
        provider: String,
    },
    /// Send a tiny test request to a model (a few tokens with an API key).
    Test {
        /// The provider's id, as shown by `ai status`.
        provider: String,
        /// The model's id (see `ai models`).
        model: String,
    },
    /// Choose the model a feature uses (`off`: none). Shows what the provider is sent first.
    Use {
        /// The feature.
        feature: FeatureArg,
        /// The provider's id (see `ai status`), or `off`.
        provider: String,
        /// The model's id (see `ai models <provider>`).
        model: Option<String>,
        /// How hard the model thinks (lowest: the cheapest setting the model allows).
        #[arg(long, value_enum, default_value_t = EffortArg::Lowest)]
        effort: EffortArg,
        /// Accept what is shown (the disclosure; for a model without a known price, that the
        /// budget can't limit it) without asking.
        #[arg(long)]
        yes: bool,
        /// Don't ask the provider for its model list (for a model it doesn't list).
        #[arg(long)]
        no_check: bool,
    },
    /// The ChatGPT plan through OpenAI's Codex: install, sign in, the weekly cap (default: status).
    Codex {
        #[command(subcommand)]
        command: Option<CodexCommand>,
    },
    /// Tokens and estimated cost of a month.
    Usage {
        /// The month, YYYY-MM (default: this month).
        #[arg(long, value_parser = parse_month)]
        month: Option<NaiveDate>,
    },
    /// Show or set the monthly budget for API keys, in US dollars (`off`: no budget).
    Budget {
        /// e.g. 5 or 2.50; `off` for none.
        amount: Option<String>,
    },
    /// What a run would cost at most, and whether anything would stop it (nothing is sent).
    Estimate {
        /// The feature.
        #[arg(long)]
        feature: FeatureArg,
        /// The course (code, name or id): one for weekly-explanation; any number for
        /// study-plan and course-calendar (default: all).
        #[arg(long = "course")]
        courses: Vec<String>,
        /// The course week (weekly-explanation; default: this week).
        #[arg(long)]
        week: Option<u32>,
        /// Days ahead (study-plan; default: 14).
        #[arg(long)]
        days: Option<u32>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum FeatureArg {
    #[value(alias = "study_plan")]
    StudyPlan,
    #[value(alias = "weekly_explanation")]
    WeeklyExplanation,
    #[value(alias = "weekly_note")]
    WeeklyNote,
    #[value(alias = "course_calendar")]
    CourseCalendar,
}

impl From<FeatureArg> for AiFeature {
    fn from(arg: FeatureArg) -> Self {
        match arg {
            FeatureArg::StudyPlan => AiFeature::StudyPlan,
            FeatureArg::WeeklyExplanation => AiFeature::WeeklyExplanation,
            FeatureArg::WeeklyNote => AiFeature::WeeklyNote,
            FeatureArg::CourseCalendar => AiFeature::CourseCalendar,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EffortArg {
    Lowest,
    Low,
    Medium,
    High,
}

impl From<EffortArg> for Effort {
    fn from(arg: EffortArg) -> Self {
        match arg {
            EffortArg::Lowest => Effort::Lowest,
            EffortArg::Low => Effort::Low,
            EffortArg::Medium => Effort::Medium,
            EffortArg::High => Effort::High,
        }
    }
}

fn parse_month(text: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(&format!("{text}-01"), "%Y-%m-%d")
        .map_err(|_| format!("expected YYYY-MM, got '{text}'"))
}

/// A feature as the CLI names it (`study-plan`).
fn feature_name(feature: AiFeature) -> String {
    feature.as_str().replace('_', "-")
}

/// A provider id, or `codex` / `claude-code`.
fn backend(id: &str) -> BackendRef {
    match id {
        "codex" => BackendRef::Codex,
        "claude-code" | "claude_code" => BackendRef::ClaudeCode,
        _ => BackendRef::Provider {
            provider_id: id.to_string(),
        },
    }
}

fn backend_name(backend: &BackendRef) -> &str {
    match backend {
        BackendRef::Codex => "codex",
        BackendRef::ClaudeCode => "claude-code",
        BackendRef::Provider { provider_id } => provider_id,
    }
}

pub async fn run(app: &App, command: AiCommand, json: bool) -> anyhow::Result<()> {
    match command {
        AiCommand::Status => status(app, json),
        AiCommand::Presets => {
            let presets = app.model_provider_presets();
            if json {
                return print_json(&presets);
            }
            for preset in &presets {
                let how = match (preset.local, preset.needs_key) {
                    (true, _) => "on this computer, no key",
                    (false, true) if preset.default_base_url.is_none() => "key and --base-url",
                    (false, true) => "key",
                    (false, false) => "no key",
                };
                println!("{:<12} {} ({how})", preset.id, preset.label);
            }
            Ok(())
        }
        AiCommand::Add { preset, base_url } => {
            let needs_key = app
                .model_provider_presets()
                .iter()
                .find(|p| p.id == preset)
                .map(|p| p.needs_key);
            let key = match needs_key {
                Some(true) => Some(read_secret("API key: ")?),
                // Unknown presets fail in the facade, before anything is read or sent.
                _ => None,
            };
            let record = app
                .add_model_provider(&preset, base_url.as_deref(), key.as_deref())
                .await?;
            if json {
                return print_json(&record);
            }
            println!("Added {} ({}).", record.label, record.provider_id);
            eprintln!(
                "Next: `{CLI_NAME} ai models {id}`, then `{CLI_NAME} ai use <feature> {id} <model>`.",
                id = record.provider_id
            );
            Ok(())
        }
        AiCommand::UpdateKey { provider } => {
            let key = read_secret("New API key: ")?;
            let record = app.update_model_provider_key(&provider, &key).await?;
            if json {
                return print_json(&record);
            }
            println!("Updated the key of {}.", record.label);
            Ok(())
        }
        AiCommand::Remove { provider, all, yes } => {
            if all {
                confirm(text::REMOVE_ALL_AI_DATA, yes)?;
                let report = app.remove_all_ai_data()?;
                if json {
                    return print_json(&report);
                }
                println!(
                    "Removed {} provider(s), {} usage record(s) and {} generated text(s){}.",
                    report.providers_removed,
                    report.usage_rows_removed,
                    report.generations_removed,
                    if report.backup_removed {
                        ", and the pre-update database backup"
                    } else {
                        ""
                    }
                );
                return Ok(());
            }
            let provider = provider.unwrap_or_default();
            app.remove_model_provider(&provider)?;
            if json {
                return print_json(&serde_json::json!({ "removed": provider }));
            }
            println!("Removed {provider} and its key.");
            Ok(())
        }
        AiCommand::Models { provider } => {
            let models = app.list_models(&backend(&provider)).await?;
            if json {
                return print_json(&models);
            }
            if models.is_empty() {
                println!("{provider} lists no models.");
            }
            let width = models
                .iter()
                .map(|m| m.id.chars().count())
                .max()
                .unwrap_or(0);
            for model in &models {
                let mut facts = vec![if model.on_device && !model.runs_in_cloud {
                    "on this computer".to_string()
                } else {
                    "cloud".to_string()
                }];
                if !model.on_device {
                    facts.push(
                        if model.price_known {
                            "price known"
                        } else {
                            "price unknown"
                        }
                        .into(),
                    );
                }
                if let Some(window) = model.context_window {
                    facts.push(format!("{}k context", window / 1000));
                }
                if model.reasoning_always_on {
                    facts.push("always thinks".into());
                }
                if !model.suggested_for.is_empty() {
                    let features: Vec<String> = model
                        .suggested_for
                        .iter()
                        .map(|f| feature_name(*f))
                        .collect();
                    facts.push(format!("suggested for {}", features.join(", ")));
                }
                println!("{:<width$}  {}", model.id, facts.join(" · "));
            }
            Ok(())
        }
        AiCommand::Test { provider, model } => {
            let report = app.test_model(&backend(&provider), &model).await?;
            if json {
                print_json(&report)?;
            } else if report.ok {
                println!(
                    "OK: answered in {} ms{}{}.",
                    report.latency_ms,
                    match report.structured_output_tier {
                        Some(StructuredOutputTier::NativeSchema) =>
                            ", structured answers supported",
                        Some(StructuredOutputTier::JsonObject) =>
                            ", JSON answers (checked by PageLamp)",
                        Some(StructuredOutputTier::PromptOnly) => ", answers checked by PageLamp",
                        None => "",
                    },
                    if report.thinking_always_on {
                        "; it always thinks (costs more)"
                    } else {
                        ""
                    }
                );
            }
            match (report.ok, report.error) {
                (true, _) => Ok(()),
                (false, Some(kind)) => anyhow::bail!(
                    "the test failed: {} ({})",
                    text::model_error(kind),
                    kind.as_str()
                ),
                (false, None) => anyhow::bail!("the test failed"),
            }
        }
        AiCommand::Use {
            feature,
            provider,
            model,
            effort,
            yes,
            no_check,
        } => {
            let feature = AiFeature::from(feature);
            match (provider.as_str(), model) {
                ("off", None) => {
                    app.set_feature_model(feature, None)?;
                    if json {
                        return print_json(
                            &serde_json::json!({ "feature": feature, "choice": null }),
                        );
                    }
                    println!("{} uses no model now.", feature_name(feature));
                    Ok(())
                }
                (_, Some(model)) => {
                    let choice = ModelChoice {
                        backend: backend(&provider),
                        model,
                        effort: effort.into(),
                    };
                    use_model(app, feature, choice, yes, no_check).await?;
                    if json {
                        return print_json(&app.ai_status()?.features);
                    }
                    Ok(())
                }
                (_, None) => {
                    anyhow::bail!("name the model (see `{CLI_NAME} ai models {provider}`)")
                }
            }
        }
        AiCommand::Codex { command } => {
            codex(app, command.unwrap_or(CodexCommand::Status), json).await
        }
        AiCommand::Usage { month } => {
            let summary = app.usage_summary(month)?;
            if json {
                return print_json(&summary);
            }
            let label = format!("{}-{:02}", summary.month.year(), summary.month.month());
            if summary.rows.is_empty() {
                println!("No AI use in {label}.");
            }
            for row in &summary.rows {
                let cost = match (row.cost_basis, row.micro_usd) {
                    (CostBasis::FreeOnDevice, _) => "free (on this computer)".to_string(),
                    (CostBasis::Plan, _) => "your plan".to_string(),
                    (CostBasis::Unpriced, _) | (_, None) => "price unknown".to_string(),
                    (CostBasis::Priced, Some(micro)) => format!("≈ {}", usd(micro)),
                };
                println!(
                    "{label} · {} · {} · {}: {} run(s), {} tokens in, {} out{} · {cost}{}",
                    row.backend_label,
                    row.model,
                    feature_name(row.feature),
                    row.runs,
                    thousands(row.input_tokens),
                    thousands(row.output_tokens),
                    if row.reasoning_tokens > 0 {
                        format!(" ({} thinking)", thousands(row.reasoning_tokens))
                    } else {
                        String::new()
                    },
                    if row.estimated {
                        " (some counts estimated)"
                    } else {
                        ""
                    }
                );
            }
            println!(
                "Total ≈ {} · {}",
                usd(summary.total_micro_usd),
                budget_line(
                    summary.budget.monthly_micro_usd,
                    summary.budget.spent_micro_usd
                )
            );
            Ok(())
        }
        AiCommand::Budget { amount } => {
            if let Some(amount) = amount {
                let cap = match amount.trim().to_ascii_lowercase().as_str() {
                    "off" | "none" => None,
                    other => Some(parse_usd(other)?),
                };
                app.set_monthly_budget(cap)?;
            }
            let budget = app.ai_status()?.budget;
            if json {
                return print_json(&budget);
            }
            println!(
                "{}",
                budget_line(budget.monthly_micro_usd, budget.spent_micro_usd)
            );
            Ok(())
        }
        AiCommand::Estimate {
            feature,
            courses,
            week,
            days,
        } => {
            let request = match feature {
                FeatureArg::StudyPlan => EstimateRequest::StudyPlan {
                    horizon_days: days,
                    courses,
                },
                FeatureArg::WeeklyExplanation => {
                    let [course] = <[String; 1]>::try_from(courses).map_err(|_| {
                        anyhow::anyhow!("weekly-explanation needs exactly one --course")
                    })?;
                    EstimateRequest::WeeklyExplanation {
                        course,
                        week,
                        include: Vec::new(),
                    }
                }
                FeatureArg::WeeklyNote => EstimateRequest::WeeklyNote,
                FeatureArg::CourseCalendar => EstimateRequest::CourseCalendar { courses },
            };
            let estimate = app.estimate_generation(&request)?;
            if json {
                print_json(&estimate)?;
            } else if estimate.input_tokens > 0 {
                println!("{}", estimate_line(&estimate));
            }
            match estimate.would_block {
                // The facade's own code, so scripts and tests can match it.
                Some(reason) => anyhow::bail!(
                    "blocked: {} — {}",
                    reason.as_str(),
                    text::block_reason(reason)
                ),
                None => Ok(()),
            }
        }
    }
}

#[derive(Subcommand)]
pub enum CodexCommand {
    /// The runtime, the sign-in and this week's runs (default).
    Status,
    /// Download and verify the Codex version PageLamp is tested with (≈70–80 MB).
    Install,
    /// Delete the downloaded Codex (the sign-in stays until `codex logout`).
    Remove,
    /// Sign in to ChatGPT through Codex (opens the browser).
    Login {
        /// Use a one-time code instead of the browser (enable it in ChatGPT's security settings).
        #[arg(long)]
        device_code: bool,
    },
    /// Sign out of ChatGPT in PageLamp's Codex.
    Logout,
    /// Show or set the weekly run cap (a number, or `off`).
    Cap { runs: Option<String> },
    /// Run PageLamp's Codex (`managed`) or your own installed one in the tested range (`system`).
    Use { source: SourceArg },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum SourceArg {
    Managed,
    System,
}

async fn codex(app: &App, command: CodexCommand, json: bool) -> anyhow::Result<()> {
    // Status (which then starts nothing) and sign-out (clean-up) work in every build; the rest
    // needs the ChatGPT plan.
    if !matches!(command, CodexCommand::Status | CodexCommand::Logout) {
        app.require_chatgpt_plan()?;
    }
    let status = match command {
        CodexCommand::Status => app.codex_status().await?,
        CodexCommand::Install => {
            let before = app.codex_status().await?;
            eprintln!(
                "Downloading Codex {} from OpenAI (≈{} MB)…",
                before.runtime.pinned_version,
                before.runtime.download_bytes.div_ceil(1_000_000)
            );
            app.install_codex(
                &format!("cli-{}", std::process::id()),
                |event| match event {
                    RuntimeEvent::Progress {
                        downloaded_bytes,
                        total_bytes,
                    } if total_bytes > 0 => {
                        eprint!("\r  {:>3}%", downloaded_bytes * 100 / total_bytes);
                    }
                    RuntimeEvent::Verifying => eprintln!("\r  checking it is OpenAI's…"),
                    RuntimeEvent::Installing => eprintln!("  installing…"),
                    _ => {}
                },
            )
            .await?
        }
        CodexCommand::Remove => {
            app.remove_codex()?;
            app.codex_status().await?
        }
        CodexCommand::Login { device_code } => {
            let method = if device_code {
                CodexLoginMethod::DeviceCode
            } else {
                CodexLoginMethod::Browser
            };
            app.codex_login(method, |event| match event {
                LoginEvent::BrowserOpened { url } => {
                    eprintln!("Sign in to ChatGPT in your browser.");
                    if let Some(url) = url {
                        eprintln!("If it didn't open, go to: {url}");
                    }
                }
                LoginEvent::DeviceCode {
                    verification_url,
                    user_code,
                    ..
                } => eprintln!("Go to {verification_url} and enter the code {user_code}"),
                LoginEvent::Waiting => eprintln!("Waiting for you to finish signing in…"),
                LoginEvent::Done => eprintln!("Signed in."),
            })
            .await?
        }
        CodexCommand::Logout => app.codex_logout().await?,
        CodexCommand::Cap { runs } => {
            if let Some(runs) = runs {
                let cap = match runs.trim().to_ascii_lowercase().as_str() {
                    "off" | "none" => None,
                    other => Some(
                        other
                            .parse::<u32>()
                            .map_err(|_| anyhow::anyhow!("expected a number of runs, or off"))?,
                    ),
                };
                app.set_mode_a_weekly_cap(cap)?;
            }
            app.codex_status().await?
        }
        CodexCommand::Use { source } => {
            app.set_codex_source(match source {
                SourceArg::Managed => CodexSource::Managed,
                SourceArg::System => CodexSource::System,
            })
            .await?
        }
    };
    if json {
        return print_json(&status);
    }
    print_codex_status(&status);
    Ok(())
}

fn print_codex_status(status: &CodexStatus) {
    // Nothing else to show: no Codex was looked for, and every hint below names a command that
    // refuses in this build.
    if !status.chatgpt_plan_offered {
        println!("The ChatGPT plan isn't available in this version of PageLamp.");
        return;
    }
    let runtime = &status.runtime;
    let installed = match (runtime.state, &runtime.installed_version) {
        (CodexRuntimeState::UnsupportedPlatform, _) => {
            "not available for this computer".to_string()
        }
        (CodexRuntimeState::Installed, Some(version)) => format!(
            "Codex {version} ({})",
            match runtime.source {
                CodexSource::Managed => "installed by PageLamp",
                CodexSource::System => "your own",
            }
        ),
        _ => format!("not installed (`{CLI_NAME} ai codex install`)"),
    };
    println!(
        "Runtime: {installed}; PageLamp is tested with {}",
        runtime.pinned_version
    );
    if runtime.untested_platform {
        println!("  (Windows on Arm: runs, but untested)");
    }
    match status.outdated_action {
        CodexOutdatedAction::InstallPin => {
            println!("  PageLamp needs a newer Codex: run `{CLI_NAME} ai codex install`.")
        }
        CodexOutdatedAction::UpdatePagelamp => {
            println!("  Update PageLamp to keep using your ChatGPT plan.")
        }
        CodexOutdatedAction::None => {}
    }
    println!(
        "Sign-in: {}",
        match status.login.state {
            CodexLoginState::Chatgpt => "ChatGPT".to_string(),
            CodexLoginState::ApiKey =>
                "an API key or token: runs bill that, not your ChatGPT plan".to_string(),
            CodexLoginState::SignedOut => format!("signed out (`{CLI_NAME} ai codex login`)"),
        }
    );
    println!(
        "This week: {} run(s){}",
        status.runs_this_week,
        status
            .weekly_cap
            .map(|cap| format!(" of {cap}"))
            .unwrap_or_else(|| " (no cap)".into())
    );
    if let Some(system) = &status.system_codex {
        println!(
            "Your own Codex: {}{}",
            system.version,
            if system.in_tested_range {
                format!(" (can be used: `{CLI_NAME} ai codex use system`)")
            } else {
                " (not a version PageLamp tested)".to_string()
            }
        );
    }
}

fn status(app: &App, json: bool) -> anyhow::Result<()> {
    let status = app.ai_status()?;
    if json {
        return print_json(&status);
    }
    if status.providers.is_empty() {
        println!(
            "No model providers yet. Add one with `{CLI_NAME} ai add <preset>` (see `{CLI_NAME} ai presets`)."
        );
    } else {
        println!("Providers:");
    }
    for record in &status.providers {
        let id = BackendRef::Provider {
            provider_id: record.provider_id.clone(),
        };
        let mut facts = vec![record.label.clone(), record.base_url.clone()];
        if let Some(last4) = &record.key_last4 {
            facts.push(format!("key …{last4}"));
        }
        facts.push(
            if record.on_device {
                "on this computer"
            } else {
                "cloud"
            }
            .into(),
        );
        if let Some(backend) = status.backends.iter().find(|b| b.backend == id) {
            facts.push(state_text(backend));
        }
        println!("  {}  {}", record.provider_id, facts.join(" · "));
    }
    println!("Features:");
    for routing in &status.features {
        let choice = match &routing.choice {
            Some(choice) => format!(
                "{} on {} (effort {})",
                choice.model,
                backend_name(&choice.backend),
                choice.effort.as_str()
            ),
            None => "no model chosen".into(),
        };
        println!("  {:<19} {choice}", feature_name(routing.feature));
    }
    println!(
        "{}",
        budget_line(
            status.budget.monthly_micro_usd,
            status.budget.spent_micro_usd
        )
    );
    Ok(())
}

fn state_text(backend: &AiBackendStatus) -> String {
    match backend.state {
        BackendState::Ready => "ready".into(),
        BackendState::NeedsSetup => format!(
            "key missing (`{CLI_NAME} ai update-key {}`)",
            backend_name(&backend.backend)
        ),
        BackendState::NeedsDisclosure => "not accepted yet (`ai use` shows what it is sent)".into(),
        BackendState::Unavailable => "unavailable".into(),
    }
}

/// `ai use`: the disclosure (and, for a model without a known price, the budget note) first,
/// then the choice and the acknowledgements. Nothing is saved unless the student agrees.
async fn use_model(
    app: &App,
    feature: AiFeature,
    choice: ModelChoice,
    yes: bool,
    no_check: bool,
) -> anyhow::Result<()> {
    let name = backend_name(&choice.backend).to_string();
    let status = app.ai_status()?;
    let Some(backend) = status.backends.iter().find(|b| b.backend == choice.backend) else {
        if choice.backend == BackendRef::Codex {
            // In a build that doesn't offer the plan, its own refusal (no setup copy).
            app.require_chatgpt_plan()?;
            anyhow::bail!(
                "set up the ChatGPT plan first: `{CLI_NAME} ai codex install`, then `{CLI_NAME} ai codex login`"
            );
        }
        // Unknown providers and Claude Code: the facade's error says why.
        app.set_feature_model(feature, Some(choice))?;
        anyhow::bail!("'{name}' is not set up; see `{CLI_NAME} ai status`");
    };
    let on_device = backend.disclosure.on_device;
    // Whether the model's price is known (`None`: not checked).
    let price_known = if no_check {
        None
    } else {
        let models = app.list_models(&choice.backend).await?;
        let Some(info) = models.iter().find(|m| m.id == choice.model) else {
            anyhow::bail!(
                "{name} doesn't list '{}' (see `{CLI_NAME} ai models {name}`, or use --no-check)",
                choice.model
            );
        };
        Some(info.price_known || (info.on_device && !info.runs_in_cloud))
    };
    let disclose = backend.disclosure_acknowledged != Some(backend.disclosure.version);
    // The ChatGPT plan has no price to accept (a weekly run cap instead).
    let unpriced = !on_device && price_known != Some(true) && choice.backend != BackendRef::Codex;
    if disclose {
        eprintln!("{}\n", text::disclosure(&backend.disclosure));
    }
    if unpriced {
        eprintln!("{}\n", text::unpriced_model(price_known.is_none()));
    }
    if disclose || unpriced {
        confirm("Continue?", yes)?;
    }
    app.set_feature_model(feature, Some(choice.clone()))?;
    if disclose {
        app.acknowledge_ai_disclosure(&choice.backend, backend.disclosure.version)?;
    }
    if unpriced {
        app.acknowledge_unpriced_model(&choice.backend, &choice.model)?;
    }
    eprintln!(
        "{} now uses {} on {name} (effort {}).",
        feature_name(feature),
        choice.model,
        choice.effort.as_str()
    );
    Ok(())
}

fn estimate_line(estimate: &CostEstimate) -> String {
    let cost = match estimate.micro_usd_upper {
        Some(0) => "Free".to_string(),
        Some(micro) => format!("≈ {} at most", usd(micro)),
        None => "Price unknown".to_string(),
    };
    let mut tokens = format!(
        "{} tokens in, up to {} out",
        thousands(estimate.input_tokens),
        thousands(estimate.max_output_tokens)
    );
    if estimate.reasoning_allowance > 0 {
        tokens.push_str(&format!(
            " + {} thinking",
            thousands(estimate.reasoning_allowance)
        ));
    }
    if estimate.repair_possible {
        tokens.push_str("; one repair possible");
    }
    format!("{cost} ({tokens})")
}

fn budget_line(cap: Option<u64>, spent: u64) -> String {
    match cap {
        Some(cap) => format!("Budget: ≈ {} of {} this month.", usd(spent), usd(cap)),
        None => format!("Budget: none (≈ {} this month).", usd(spent)),
    }
}

/// Micro-USD as dollars ("< $0.01" for less than half a cent).
fn usd(micro: u64) -> String {
    match micro {
        0 => "$0.00".into(),
        1..5_000 => "< $0.01".into(),
        _ => {
            let cents = (micro + 5_000) / 10_000;
            format!("${}.{:02}", cents / 100, cents % 100)
        }
    }
}

/// "5", "2.50", "$0.75" → micro-USD.
fn parse_usd(text: &str) -> anyhow::Result<u64> {
    let text = text.trim().trim_start_matches('$');
    let invalid = || anyhow::anyhow!("expected an amount in US dollars like 5 or 2.50, or off");
    let (dollars, fraction) = text.split_once('.').unwrap_or((text, ""));
    if dollars.is_empty() && fraction.is_empty()
        || fraction.len() > 6
        || !dollars
            .chars()
            .chain(fraction.chars())
            .all(|c| c.is_ascii_digit())
    {
        return Err(invalid());
    }
    let dollars: u64 = if dollars.is_empty() {
        0
    } else {
        dollars.parse().map_err(|_| invalid())?
    };
    let fraction: u64 = format!("{fraction:0<6}").parse().map_err(|_| invalid())?;
    dollars
        .checked_mul(1_000_000)
        .and_then(|micro| micro.checked_add(fraction))
        .ok_or_else(invalid)
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_parse_and_print_in_dollars() {
        assert_eq!(parse_usd("5").unwrap(), 5_000_000);
        assert_eq!(parse_usd("$2.50").unwrap(), 2_500_000);
        assert_eq!(parse_usd(".75").unwrap(), 750_000);
        assert_eq!(parse_usd("0.000001").unwrap(), 1);
        for bad in [
            "",
            ".",
            "-1",
            "1.2.3",
            "abc",
            "1.0000001",
            "99999999999999999",
        ] {
            assert!(parse_usd(bad).is_err(), "{bad}");
        }
        assert_eq!(usd(0), "$0.00");
        assert_eq!(usd(4_999), "< $0.01");
        assert_eq!(usd(5_000), "$0.01");
        assert_eq!(usd(2_500_000), "$2.50");
        assert_eq!(usd(1_234_567_890), "$1234.57");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(
            parse_month("2026-09").unwrap(),
            NaiveDate::from_ymd_opt(2026, 9, 1).unwrap()
        );
        assert!(parse_month("2026-13").is_err());
    }
}
