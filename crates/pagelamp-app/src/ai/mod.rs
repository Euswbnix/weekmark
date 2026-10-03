//! PageLamp's own model calls, as the facade offers them (v0.3 M1; design §3.8). The logic lives
//! here, so the Tauri app, the Swift shell and the CLI can't diverge (rule 12): disclosure facts,
//! policy questions, budgets and estimates are computed in Rust, and the UIs only render them.
//!
//! Model code is in `pagelamp-llm` (drivers) and `pagelamp_core::ai_gate` (the only producer of
//! prompt text).

mod estimate;
pub mod prompts;
mod providers;
mod settings;
mod types;
mod usage;

pub use types::*;

use std::time::Duration;

use chrono::NaiveDate;
use pagelamp_core::ai::{AiFeature, MaterialSharing};
use pagelamp_core::ingest::sha256_hex;
use pagelamp_llm::profile::{self, ProviderProfile, Retention, Training, Wire};
use pagelamp_llm::{Backend, CancellationToken, HttpDriver};

use crate::{App, AppError, AppErrorKind, Result};

/// Bumped whenever the fixed disclosure paragraphs (limitations and risks, ownership) change in
/// the UIs: part of every `DisclosureFacts::version`, so the student is asked again.
const DISCLOSURE_WORDING_VERSION: u32 = 1;

/// How long `detect_local_servers` waits for a local server.
const DETECT_TIMEOUT: Duration = Duration::from_secs(2);

/// The warning threshold of the monthly budget (owner decision D18).
pub const BUDGET_WARN_AT_PERCENT: u8 = 80;

/// The default monthly budget for API keys: US$5 (owner decision D18), in micro-USD.
pub const DEFAULT_MONTHLY_BUDGET_MICRO_USD: u64 = 5_000_000;

impl App {
    /// The providers PageLamp can set up, with their data-policy line (no network call).
    pub fn model_provider_presets(&self) -> Vec<ProviderPreset> {
        profile::presets().iter().map(provider_preset).collect()
    }

    /// Ollama and LM Studio on this computer: which are running (a quick loopback call each).
    pub async fn detect_local_servers(&self) -> Result<Vec<LocalServer>> {
        let mut servers = Vec::new();
        for (preset_id, kind) in [
            ("ollama", LocalServerKind::Ollama),
            ("lm_studio", LocalServerKind::LmStudio),
        ] {
            let Some(preset) = profile::preset(preset_id) else {
                continue;
            };
            let base_url = preset
                .base_url
                .as_ref()
                .map(|url| url.as_str().trim_end_matches('/').to_string())
                .unwrap_or_default();
            let running = match HttpDriver::new(preset.clone(), None) {
                Ok(driver) => {
                    let backend = Backend::Http(driver);
                    let cancel = CancellationToken::new();
                    matches!(
                        tokio::time::timeout(DETECT_TIMEOUT, backend.list_models(&cancel)).await,
                        Ok(Ok(_))
                    )
                }
                Err(_) => false,
            };
            servers.push(LocalServer {
                kind,
                base_url,
                running,
            });
        }
        Ok(servers)
    }

    // ----- status and providers -----------------------------------------------------------------

    /// Providers, feature routing, backends with their disclosure facts, and the budget. No
    /// network call (local servers: `detect_local_servers`).
    pub fn ai_status(&self) -> Result<AiStatus> {
        let store = self.read_store()?;
        let acks = settings::disclosures(&store)?;
        let mut providers = Vec::new();
        let mut backends = Vec::new();
        for (record, provider) in self.provider_records(&store)? {
            let backend = BackendRef::Provider {
                provider_id: record.provider_id.clone(),
            };
            let disclosure = disclosure_for(&provider.profile);
            let acknowledged = acks
                .get(&settings::backend_key(&backend))
                .map(|a| a.version);
            let mut problems = Vec::new();
            if provider.needs_key() && provider.key.is_none() {
                problems.push(BackendProblem::KeyMissing);
            }
            if acknowledged.is_some_and(|version| version != disclosure.version) {
                problems.push(BackendProblem::DisclosureChanged);
            }
            let state = if problems.contains(&BackendProblem::KeyMissing) {
                BackendState::NeedsSetup
            } else if acknowledged != Some(disclosure.version) {
                BackendState::NeedsDisclosure
            } else {
                BackendState::Ready
            };
            backends.push(AiBackendStatus {
                backend,
                label: record.label.clone(),
                kind: if record.on_device {
                    BackendKind::Local
                } else {
                    BackendKind::ApiKey
                },
                state,
                problems,
                disclosure,
                disclosure_acknowledged: acknowledged,
            });
            providers.push(record);
        }
        let routing = settings::routing(&store)?;
        let features = AiFeature::ALL
            .iter()
            .map(|feature| FeatureRouting {
                feature: *feature,
                choice: routing.0.get(feature).cloned(),
            })
            .collect();
        Ok(AiStatus {
            backends,
            providers,
            features,
            budget: usage::budget_status(&store)?,
        })
    }

    /// Add an API-key or local provider: checks the address (HTTPS unless this computer), refuses
    /// coding-plan endpoints and keys, validates the key with a free model-list call, and keeps
    /// the key in the keychain (`llm:<provider_id>`). Nothing is saved when a check fails.
    pub async fn add_model_provider(
        &self,
        preset: &str,
        base_url: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<ModelProviderRecord> {
        self.add_provider(preset, base_url, api_key).await
    }

    /// Replace a provider's key (validated first; the old key stays if the new one fails).
    pub async fn update_model_provider_key(
        &self,
        provider_id: &str,
        api_key: &str,
    ) -> Result<ModelProviderRecord> {
        self.replace_provider_key(provider_id, api_key).await
    }

    /// Remove a provider: its row, its keychain entry, and every feature routed to it.
    pub fn remove_model_provider(&self, provider_id: &str) -> Result<()> {
        self.delete_provider(provider_id)
    }

    /// The models a backend offers: its live list with PageLamp's price and capability data.
    pub async fn list_models(&self, backend: &BackendRef) -> Result<Vec<ModelInfo>> {
        let provider = self.provider_for(backend)?;
        let profile = provider.profile.clone();
        let local = profile.on_device();
        let models = provider
            .backend()?
            .list_models(&CancellationToken::new())
            .await?;
        Ok(models
            .into_iter()
            .map(|model| {
                let quirks = profile.model_quirks(&model.id);
                ModelInfo {
                    price_known: model.on_device
                        || pagelamp_llm::catalog::lookup(&profile.id, &model.id).is_some(),
                    runs_in_cloud: local && !model.on_device,
                    on_device: model.on_device,
                    context_window: model.context_window,
                    reasoning_always_on: quirks.thinking_always_on,
                    label: model.display_name,
                    suggested_for: Vec::new(),
                    id: model.id,
                }
            })
            .collect())
    }

    /// "Test": a tiny request with no course data. A model that answers badly or refuses the
    /// key is a report with `ok: false`, not an error; the last test is kept with the provider.
    pub async fn test_model(&self, backend: &BackendRef, model: &str) -> Result<ProbeReport> {
        let provider = self.provider_for(backend)?;
        let provider_id = provider.row.id.clone();
        let thinking = provider.profile.model_quirks(model).thinking_always_on;
        let started = std::time::Instant::now();
        let report = match provider
            .backend()?
            .probe(model, CancellationToken::new())
            .await
        {
            Ok(probe) => ProbeReport {
                ok: true,
                latency_ms: u32::try_from(probe.latency.as_millis()).unwrap_or(u32::MAX),
                structured_output_tier: Some(tier(probe.json_tier)),
                thinking_always_on: probe.thinking_always_on,
                error: None,
            },
            Err(pagelamp_llm::LlmError::Model(error)) => ProbeReport {
                ok: false,
                latency_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
                structured_output_tier: None,
                thinking_always_on: thinking,
                error: Some(error.kind),
            },
            Err(other) => return Err(other.into()),
        };
        let json = serde_json::to_string(&report).ok();
        self.write_store()?
            .set_model_provider_probe(&provider_id, json.as_deref())?;
        Ok(report)
    }

    // ----- choices and acknowledgements ------------------------------------------------------------

    /// Which model a feature uses (`None`: none). The backend must exist.
    pub fn set_feature_model(&self, feature: AiFeature, choice: Option<ModelChoice>) -> Result<()> {
        if let Some(choice) = &choice {
            self.provider_profile(&choice.backend)?;
            if choice.model.trim().is_empty() {
                return Err(AppError::new(AppErrorKind::Invalid, "Choose a model."));
            }
        }
        let store = self.write_store()?;
        let mut routing = settings::routing(&store)?;
        match choice {
            Some(choice) => routing.0.insert(feature, choice),
            None => routing.0.remove(&feature),
        };
        store.set_setting(settings::ROUTING, &routing)?;
        Ok(())
    }

    /// The student read a backend's disclosure. `version` must be the one currently shown:
    /// facts that changed since are asked again.
    pub fn acknowledge_ai_disclosure(&self, backend: &BackendRef, version: u32) -> Result<()> {
        let current = disclosure_for(&self.provider_profile(backend)?).version;
        if version != current {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                "What this service does with your data changed; please read it again.",
            ));
        }
        let store = self.write_store()?;
        let mut acks = settings::disclosures(&store)?;
        acks.insert(
            settings::backend_key(backend),
            settings::Acknowledgement {
                version,
                at: chrono::Utc::now(),
            },
        );
        store.set_setting(settings::DISCLOSURES, &acks)?;
        Ok(())
    }

    /// "The budget is not enforced for this model": needed before the first run of a model
    /// without a known price.
    pub fn acknowledge_unpriced_model(&self, backend: &BackendRef, model: &str) -> Result<()> {
        self.provider_profile(backend)?;
        let store = self.write_store()?;
        let mut acks = settings::unpriced(&store)?;
        let models = acks.entry(settings::backend_key(backend)).or_default();
        if !models.iter().any(|m| m == model) {
            models.push(model.to_string());
        }
        store.set_setting(settings::UNPRICED, &acks)?;
        Ok(())
    }

    /// The monthly soft cap for API keys, in micro-USD (`None`: no cap).
    pub fn set_monthly_budget(&self, micro_usd: Option<u64>) -> Result<()> {
        self.write_store()?.set_setting(
            settings::BUDGET,
            &settings::BudgetSetting {
                monthly_micro_usd: micro_usd,
            },
        )?;
        Ok(())
    }

    // ----- estimate and usage ---------------------------------------------------------------------

    /// "≈ $x" before Generate (an upper bound), and what would block the run. Local and cheap:
    /// no network call.
    pub fn estimate_generation(&self, request: &EstimateRequest) -> Result<CostEstimate> {
        self.estimate(request)
    }

    /// Token counts and estimated cost of a month (default: this month).
    pub fn usage_summary(&self, month: Option<NaiveDate>) -> Result<UsageSummary> {
        self.usage(month)
    }

    /// The student's answer to "may this course's materials be shared with an AI service?".
    pub fn set_course_material_sharing(&self, course: &str, answer: MaterialSharing) -> Result<()> {
        let store = self.write_store()?;
        let course = store.resolve_course_with(course, true)?;
        store.set_course_material_sharing(&course.id, answer)?;
        Ok(())
    }

    /// Keys, providers, generations, the usage ledger, AI settings and the pre-update backup
    /// (from schema 4 on it holds AI data too; the result says it was removed).
    pub fn remove_all_ai_data(&self) -> Result<RemoveAiDataReport> {
        let store = self.write_store()?;
        let keyed: Vec<String> = store
            .model_providers()?
            .into_iter()
            .filter(providers::uses_key)
            .map(|p| p.id)
            .collect();
        let removed = store.remove_all_ai_data()?;
        for id in &keyed {
            self.secrets.delete(&providers::key_account(id))?;
        }
        for key in settings::ALL_KEYS {
            store.remove_setting(key)?;
        }
        drop(store);
        let backup_removed = pagelamp_core::store::database_backup(&self.db_path()).is_some();
        if backup_removed {
            pagelamp_core::store::delete_database_backups(&self.db_path()).map_err(|err| {
                AppError::new(
                    AppErrorKind::Internal,
                    format!("could not delete the pre-update backup: {err}"),
                )
            })?;
        }
        Ok(RemoveAiDataReport {
            providers_removed: removed.providers,
            generations_removed: removed.generations,
            usage_rows_removed: removed.usage_rows,
            backup_removed,
        })
    }
}

fn tier(tier: pagelamp_llm::request::JsonTier) -> StructuredOutputTier {
    match tier {
        pagelamp_llm::request::JsonTier::NativeSchema => StructuredOutputTier::NativeSchema,
        pagelamp_llm::request::JsonTier::JsonObject => StructuredOutputTier::JsonObject,
        pagelamp_llm::request::JsonTier::PromptOnly => StructuredOutputTier::PromptOnly,
    }
}

fn provider_preset(preset: &ProviderProfile) -> ProviderPreset {
    ProviderPreset {
        id: preset.id.clone(),
        label: preset.display_name.clone(),
        wire: provider_wire(preset.wire),
        default_base_url: preset
            .base_url
            .as_ref()
            .map(|url| url.as_str().trim_end_matches('/').to_string()),
        needs_key: preset.auth != profile::Auth::None,
        base_url_editable: preset.base_url_editable,
        local: preset.local,
        data_policy: disclosure_for(preset),
    }
}

pub(crate) fn provider_wire(wire: Wire) -> ProviderWire {
    match wire {
        Wire::OpenAiResponses => ProviderWire::OpenaiResponses,
        Wire::OpenAiChat => ProviderWire::OpenaiChat,
        Wire::AnthropicMessages => ProviderWire::AnthropicMessages,
        Wire::OllamaNative => ProviderWire::OllamaNative,
    }
}

/// `doctor`'s AI facts (`AiDoctor`). Local only: the keychain is asked whether each key is
/// there, and servers on this computer whether they accept connections; nothing is sent.
pub(crate) fn doctor_checks(
    store: Option<&pagelamp_core::store::Store>,
    secrets: &dyn pagelamp_core::secrets::SecretBackend,
) -> AiDoctor {
    let rows = store
        .and_then(|store| store.model_providers().ok())
        .unwrap_or_default();
    let providers = rows
        .iter()
        .map(|row| {
            let profile = providers::profile_from_row(row).ok();
            let on_device = profile.as_ref().is_some_and(ProviderProfile::on_device);
            AiProviderCheck {
                preset: row.preset.clone(),
                on_device,
                key_present: providers::uses_key(row)
                    .then(|| matches!(secrets.get(&providers::key_account(&row.id)), Ok(Some(_)))),
                reachable: profile
                    .and_then(|profile| profile.base_url)
                    .filter(|_| on_device)
                    .map(|url| accepts_connections(&url)),
            }
        })
        .collect();
    let local_servers = [
        ("ollama", LocalServerKind::Ollama),
        ("lm_studio", LocalServerKind::LmStudio),
    ]
    .into_iter()
    .filter_map(|(preset_id, kind)| {
        let url = profile::preset(preset_id)?.base_url.clone()?;
        Some(LocalServer {
            kind,
            base_url: url.as_str().trim_end_matches('/').to_string(),
            running: accepts_connections(&url),
        })
    })
    .collect();
    AiDoctor {
        providers,
        local_servers,
    }
}

/// Whether something on this computer accepts connections at `url`'s port (loopback
/// addresses only; anything else is not contacted).
fn accepts_connections(url: &url::Url) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    if !pagelamp_llm::is_loopback(url) {
        return false;
    }
    let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (host, port).to_socket_addrs().is_ok_and(|mut addrs| {
        addrs.any(|addr| {
            addr.ip().is_loopback()
                && TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
        })
    })
}

/// The disclosure facts of an API-key or local provider.
pub(crate) fn disclosure_for(provider: &ProviderProfile) -> DisclosureFacts {
    let policy = &provider.data_policy;
    // By address, like the gate: a local app (Ollama, LM Studio) on another computer is not on
    // this device, and what happens to the data there is up to whoever runs it.
    let on_device = provider.on_device();
    let elsewhere = provider.local && !on_device;
    let facts = DisclosureFacts {
        version: 0,
        sends: vec![SentData::Structure, SentData::MaterialText],
        recipient: Recipient {
            name: provider.display_name.clone(),
            terms_url: policy.terms_url.clone(),
        },
        training: match policy.training {
            _ if elsewhere => TrainingFact::Unknown,
            Training::NoTraining => TrainingFact::NoTraining,
            Training::MayTrainFreeTier => TrainingFact::MayTrainFreeTier,
            Training::MayTrain => TrainingFact::MayTrain {
                how_to_turn_off_url: None,
            },
            Training::Unknown => TrainingFact::Unknown,
        },
        retention: match policy.retention {
            _ if elsewhere => RetentionFact::ProviderTerms,
            Retention::NotStored => RetentionFact::NotStored,
            Retention::StoredDays(days) => RetentionFact::StoredDays { days },
            Retention::ProviderTerms => RetentionFact::ProviderTerms,
            Retention::OnDevice => RetentionFact::OnDevice,
        },
        admin_visibility: false,
        min_age: policy.min_age,
        guardian_permission: policy.guardian_permission,
        cost: if on_device {
            CostKind::FreeOnDevice
        } else if elsewhere {
            CostKind::SelfHosted
        } else {
            CostKind::ApiBilling
        },
        on_device,
        location: policy.location.clone(),
    };
    with_version(facts)
}

/// `facts` with `version` set to a hash of everything else and of the fixed wording.
pub(crate) fn with_version(mut facts: DisclosureFacts) -> DisclosureFacts {
    facts.version = 0;
    let serialized = serde_json::to_string(&facts).unwrap_or_default();
    let digest = sha256_hex(format!("{DISCLOSURE_WORDING_VERSION}:{serialized}").as_bytes());
    facts.version = u32::from_str_radix(&digest[..8], 16).unwrap_or(0);
    facts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_carry_their_data_policy_and_a_stable_version() {
        let presets: Vec<ProviderPreset> = profile::presets().iter().map(provider_preset).collect();
        let openai = presets.iter().find(|p| p.id == "openai").unwrap();
        assert!(openai.needs_key && !openai.local);
        assert_eq!(
            openai.data_policy.retention,
            RetentionFact::StoredDays { days: 30 }
        );
        assert_eq!(openai.data_policy.min_age, Some(13));
        assert_eq!(openai.data_policy.cost, CostKind::ApiBilling);
        let gemini = presets.iter().find(|p| p.id == "gemini").unwrap();
        assert_eq!(gemini.data_policy.training, TrainingFact::MayTrainFreeTier);
        assert_eq!(gemini.data_policy.min_age, Some(18));
        let ollama = presets.iter().find(|p| p.id == "ollama").unwrap();
        assert!(!ollama.needs_key && ollama.local && ollama.data_policy.on_device);
        assert_eq!(ollama.data_policy.cost, CostKind::FreeOnDevice);
        // The version is a hash: the same facts give the same number, others another.
        let again = disclosure_for(profile::preset("openai").unwrap());
        assert_eq!(again.version, openai.data_policy.version);
        assert_ne!(openai.data_policy.version, gemini.data_policy.version);
        assert_ne!(openai.data_policy.version, 0);
    }
}
