//! The facade types of PageLamp's own model calls (v0.3 M1; design §3.8). Plain
//! `Serialize + Deserialize + JsonSchema` data with snake_case codes the UIs translate: they
//! render facts and codes, never prose from here.

use chrono::NaiveDate;
use pagelamp_core::ai::{AiFeature, BlockReason, Effort, ModelErrorKind};
use pagelamp_core::ai_gate::ContextSummary;
use pagelamp_core::model::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ----- backends and choices -------------------------------------------------------------------

/// Which backend a choice or an acknowledgement is about.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackendRef {
    /// The ChatGPT plan through official Codex (M2).
    Codex,
    /// The Claude plan through the student's Claude Code (builds with that feature only).
    ClaudeCode,
    /// An API-key or local provider the student added.
    Provider { provider_id: String },
}

/// The model a feature uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelChoice {
    pub backend: BackendRef,
    pub model: String,
    pub effort: Effort,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FeatureRouting {
    pub feature: AiFeature,
    pub choice: Option<ModelChoice>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    ApiKey,
    Local,
    Codex,
    ClaudeCode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackendState {
    Ready,
    NeedsSetup,
    /// Set up, but its disclosure (or a changed one) hasn't been acknowledged.
    NeedsDisclosure,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackendProblem {
    /// The provider's key is missing from the keychain.
    KeyMissing,
    /// The local server (Ollama, LM Studio) isn't running.
    ServerNotRunning,
    /// A feature is routed to a model the provider no longer lists.
    ModelMissing,
    /// The disclosure facts changed since they were acknowledged.
    DisclosureChanged,
    /// ChatGPT plan (Codex): not signed in, or signed in with something other than ChatGPT.
    NotSignedIn,
    /// ChatGPT plan (Codex): the runtime isn't installed.
    RuntimeMissing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AiBackendStatus {
    pub backend: BackendRef,
    pub label: String,
    pub kind: BackendKind,
    pub state: BackendState,
    pub problems: Vec<BackendProblem>,
    pub disclosure: DisclosureFacts,
    /// The disclosure version the student acknowledged, if any.
    pub disclosure_acknowledged: Option<u32>,
}

/// Everything the AI settings page shows (no network call).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AiStatus {
    /// This build offers the ChatGPT plan (`CHATGPT_PLAN_OFFERED`). False: no Codex backend is
    /// listed, and the UIs hide the ChatGPT card and every ChatGPT copy.
    pub chatgpt_plan_offered: bool,
    /// In priority order.
    pub backends: Vec<AiBackendStatus>,
    pub providers: Vec<ModelProviderRecord>,
    pub features: Vec<FeatureRouting>,
    pub budget: BudgetStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BudgetStatus {
    /// The monthly soft cap for API keys (`None`: no cap).
    pub monthly_micro_usd: Option<u64>,
    /// Estimated spend this month.
    pub spent_micro_usd: u64,
    /// Warn from this share of the cap.
    pub warn_at_percent: u8,
}

// ----- disclosure --------------------------------------------------------------------------------

/// What a backend is told and what happens to it (Canvas §2E items): codes and plain values the
/// UIs render. The UIs always add the fixed limitations-and-risks and ownership paragraphs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DisclosureFacts {
    /// A hash of all the facts and of the fixed paragraphs' wording: any change asks again.
    pub version: u32,
    pub sends: Vec<SentData>,
    pub recipient: Recipient,
    pub training: TrainingFact,
    pub retention: RetentionFact,
    /// Whether a school or company administrator can see what is sent (Edu, Enterprise and
    /// Business workspaces). `unknown` while the plan type isn't known: never understated.
    pub admin_visibility: AdminVisibility,
    pub min_age: Option<u8>,
    /// Under 18 needs a parent's or guardian's permission.
    pub guardian_permission: bool,
    pub cost: CostKind,
    pub on_device: bool,
    /// Where data is processed (ISO country code), if known.
    pub location: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AdminVisibility {
    /// No administrator sees it (a personal plan, an API key, this computer).
    No,
    /// Not known (e.g. the ChatGPT plan type): the UIs show the conditional warning.
    Unknown,
    /// A school or workspace administrator can see it.
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SentData {
    /// Titles, dates, week numbers, deadlines.
    Structure,
    /// The text of course materials (explanations, the course calendar).
    MaterialText,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Recipient {
    pub name: String,
    pub terms_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrainingFact {
    NoTraining,
    MayTrain {
        how_to_turn_off_url: Option<String>,
    },
    /// The free tier may train on inputs (and humans may review them).
    MayTrainFreeTier,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RetentionFact {
    NotStored,
    StoredDays { days: u32 },
    ProviderTerms,
    OnDevice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CostKind {
    /// Billed to the student's API account.
    ApiBilling,
    /// Counts against the student's ChatGPT / Claude plan.
    PlanCredits,
    FreeOnDevice,
    /// A cloud model served through the local app (Ollama cloud models).
    CloudViaLocal,
    /// A server someone else runs (a school's or a friend's Ollama or LM Studio): costs, if
    /// any, are set by whoever runs it.
    SelfHosted,
}

// ----- providers and models -----------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderWire {
    OpenaiResponses,
    OpenaiChat,
    AnthropicMessages,
    OllamaNative,
}

/// A provider PageLamp can set up.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderPreset {
    pub id: String,
    pub label: String,
    pub wire: ProviderWire,
    pub default_base_url: Option<String>,
    pub needs_key: bool,
    pub base_url_editable: bool,
    /// Runs on this computer (Ollama, LM Studio).
    pub local: bool,
    /// The data-policy line, shown before a key is entered.
    pub data_policy: DisclosureFacts,
}

/// A provider the student added (the key itself stays in the keychain).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelProviderRecord {
    pub provider_id: String,
    pub preset: String,
    pub label: String,
    pub wire: ProviderWire,
    pub base_url: String,
    /// The key's last 4 characters, for display.
    pub key_last4: Option<String>,
    pub on_device: bool,
    pub created_at: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocalServerKind {
    Ollama,
    LmStudio,
}

/// A local model server found on this computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LocalServer {
    pub kind: LocalServerKind,
    /// The preset to pass to `add_model_provider` to add it (`ollama`, `lm_studio`).
    pub preset: String,
    pub base_url: String,
    pub running: bool,
    /// The provider already added for it (the same preset and address), if any. Always
    /// `None` in `doctor`, which names no provider.
    pub provider_id: Option<String>,
}

/// `doctor`'s AI facts (M1): whether keys are there — never a key — and whether the model
/// servers on this computer answer. No provider address or id: doctor output is shared in
/// issues, and an address can name a school's server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AiDoctor {
    /// The providers the student added.
    pub providers: Vec<AiProviderCheck>,
    /// Ollama and LM Studio at their usual addresses on this computer.
    pub local_servers: Vec<LocalServer>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AiProviderCheck {
    /// The kind of provider (`openai`, `ollama`, `custom`, …).
    pub preset: String,
    pub on_device: bool,
    /// Whether its key is in the keychain (`None`: it needs none).
    pub key_present: Option<bool>,
    /// On this computer: whether its server accepts connections (`None`: not on this
    /// computer, so not contacted).
    pub reachable: Option<bool>,
}

/// A model a backend offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelInfo {
    pub id: String,
    pub label: Option<String>,
    pub on_device: bool,
    /// Served by a local app but run in the cloud (Ollama cloud models).
    pub runs_in_cloud: bool,
    /// Its price is in PageLamp's price list (else the budget can't be enforced for it).
    pub price_known: bool,
    pub context_window: Option<u32>,
    /// Thinking can't be turned off ("lowest" still thinks, and costs more).
    pub reasoning_always_on: bool,
    /// Features this release suggests the model for.
    pub suggested_for: Vec<AiFeature>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputTier {
    NativeSchema,
    JsonObject,
    PromptOnly,
}

/// What "Test" found out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProbeReport {
    pub ok: bool,
    pub latency_ms: u32,
    pub structured_output_tier: Option<StructuredOutputTier>,
    pub thinking_always_on: bool,
    pub error: Option<ModelErrorKind>,
}

// ----- estimate and usage -------------------------------------------------------------------------

/// What an estimate is for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "feature", rename_all = "snake_case")]
pub enum EstimateRequest {
    StudyPlan {
        horizon_days: Option<u32>,
        /// Course ids or codes (empty: the default set).
        courses: Vec<String>,
    },
    WeeklyExplanation {
        course: String,
        week: Option<u32>,
        /// Materials to send although they look like assessments (`ExplainOptions.include`):
        /// priced as the run with the same `include` sends them.
        #[serde(default)]
        include: Vec<String>,
    },
    WeeklyNote,
    CourseCalendar {
        courses: Vec<String>,
    },
}

/// "≈ $x" before Generate: an upper bound.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CostEstimate {
    /// `None` when the model's price is unknown.
    pub micro_usd_upper: Option<u64>,
    pub input_tokens: u64,
    pub max_output_tokens: u64,
    pub reasoning_allowance: u64,
    pub repair_possible: bool,
    pub price_known: bool,
    /// What would stop the run if started now (over budget, price not acknowledged, a course
    /// answered "not allowed", a weekly note with nothing to write about, a study plan with no
    /// course to plan for, …).
    pub would_block: Option<BlockReason>,
}

/// Token counts of a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TokenUsage {
    /// All input tokens, cached ones included.
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    /// All output tokens, reasoning included.
    pub output_tokens: u64,
    /// Of `output_tokens`, the ones spent thinking, if the provider says.
    pub reasoning_tokens: Option<u64>,
}

/// How a usage row's cost is known (the UIs label it: "≈ $x", "Free", "price unknown", "your
/// plan").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CostBasis {
    /// Priced from PageLamp's price list (an estimate of the provider's bill).
    Priced,
    /// Ran on this computer.
    FreeOnDevice,
    /// The model isn't in the price list: tokens only.
    Unpriced,
    /// Counted against a ChatGPT / Claude plan: no per-run cost.
    Plan,
}

/// One backend × model × feature of a month.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageRow {
    pub backend_label: String,
    pub model: String,
    pub feature: AiFeature,
    pub runs: u32,
    /// All input tokens, cached ones included.
    pub input_tokens: u64,
    /// All output tokens, reasoning included.
    pub output_tokens: u64,
    /// Of `output_tokens`, the ones spent thinking (0 when the provider doesn't say).
    pub reasoning_tokens: u64,
    pub cost_basis: CostBasis,
    /// Estimated cost: set for `priced` (and 0 for `free_on_device`), null for `unpriced` and
    /// `plan`.
    pub micro_usd: Option<u64>,
    /// Some counts are PageLamp's estimates (cancelled runs).
    pub estimated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageSummary {
    /// The first day of the month.
    pub month: NaiveDate,
    pub rows: Vec<UsageRow>,
    /// The priced rows' total (what the budget counts).
    pub total_micro_usd: u64,
    pub budget: BudgetStatus,
    /// The ChatGPT plan's runs this week and its cap, once the plan is set up or chosen.
    pub mode_a: Option<ModeAUsage>,
}

/// Mode A has no money budget: a weekly run cap instead (design §2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModeAUsage {
    pub runs_this_week: u32,
    /// `None`: no cap.
    pub weekly_cap: Option<u32>,
}

/// What "Remove all AI data" removed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemoveAiDataReport {
    pub providers_removed: u32,
    pub generations_removed: u32,
    pub usage_rows_removed: u32,
    /// The pre-update database backup was deleted too (it holds AI data from schema 4 on).
    pub backup_removed: bool,
}

// ----- generation (M3; the types exist from M1 for the FFI and UI stubs) -----------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GenStage {
    BuildingContext,
    WaitingForModel,
    Validating,
    Repairing,
    Scheduling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GenNoticeCode {
    ContextTrimmed,
    ThinkingAlwaysOn,
    JsonFallback,
    CoursesStructureOnly,
    MaterialsLeftOut,
    ApiKeyBilling,
    /// The course's first cloud run with question (b) unanswered or "not sure" (once per course).
    MaterialSharingReminder,
}

/// What a generation reports while it runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GenEvent {
    Started {
        generation_id: String,
        backend_label: String,
        model: String,
        on_device: bool,
    },
    Stage {
        stage: GenStage,
    },
    /// What the run sends, once its context is built (no text): the courses and materials
    /// (included, trimmed, left out), and the input tokens when they can be counted.
    Context {
        summary: pagelamp_core::ai_gate::ContextSummary,
        input_tokens: Option<u64>,
    },
    TextDelta {
        text: String,
    },
    Notice {
        code: GenNoticeCode,
    },
    Usage {
        usage: TokenUsage,
    },
    Finished {
        ok: bool,
    },
}

/// Provenance of every generated result (the "AI-generated · backend · model · date" label).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GenerationMeta {
    pub generation_id: String,
    pub feature: AiFeature,
    pub backend_label: String,
    pub model: String,
    /// The model ran on this computer.
    pub on_device: bool,
    pub created_at: Timestamp,
    pub usage: TokenUsage,
    pub est_cost_micro_usd: Option<u64>,
    /// Counts are estimates (the run was cancelled before the provider reported them).
    pub estimated: bool,
    pub context: ContextSummary,
    pub prompt_version: u32,
}

// ----- mode A: the ChatGPT plan through official Codex (M2; design §2.3, §3.8) ----------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodexRuntimeState {
    NotInstalled,
    Installed,
    /// Codex isn't offered for this computer's OS and CPU.
    UnsupportedPlatform,
}

/// Which Codex PageLamp runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodexSource {
    /// The pinned version PageLamp downloads and verifies (default).
    #[default]
    Managed,
    /// A Codex the student installed themselves, in PageLamp's tested range (D12).
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CodexRuntime {
    pub state: CodexRuntimeState,
    pub source: CodexSource,
    /// The version that runs (the managed one, or the system one when chosen).
    pub installed_version: Option<String>,
    pub pinned_version: String,
    /// The compressed download (for "≈70–80 MB").
    pub download_bytes: u64,
    /// Windows on Arm: runs, but untested (D14).
    pub untested_platform: bool,
}

/// What a `runtime_outdated` error means right now (the facade decides).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodexOutdatedAction {
    None,
    /// The installed runtime is older than the pin: install the pin ("updating…").
    InstallPin,
    /// The pin itself is too old: update PageLamp.
    UpdatePagelamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodexLoginState {
    SignedOut,
    /// Runs use the student's ChatGPT plan.
    Chatgpt,
    /// Signed in with an API key (or another non-plan method): runs bill that, not the plan.
    ApiKey,
}

/// The ChatGPT plan, when known (`codex login status` doesn't say; `unknown` until it can be
/// told).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChatGptPlanType {
    Free,
    Go,
    Plus,
    Pro,
    Business,
    Edu,
    Enterprise,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CodexLogin {
    pub state: CodexLoginState,
    pub plan_type: Option<ChatGptPlanType>,
}

/// A `codex` found on this computer (D12).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SystemCodex {
    pub version: String,
    pub in_tested_range: bool,
}

/// The ChatGPT-plan card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CodexStatus {
    /// This build offers the ChatGPT plan (`CHATGPT_PLAN_OFFERED`). False: hide the ChatGPT card
    /// and every ChatGPT copy; nothing was looked for or started, and the rest is neutral (not
    /// installed, signed out, no Codex of the student's own).
    pub chatgpt_plan_offered: bool,
    pub runtime: CodexRuntime,
    pub outdated_action: CodexOutdatedAction,
    pub login: CodexLogin,
    /// False when `codex exec` doesn't work on this plan (Free/Go, D10); `None` until known.
    pub exec_available: Option<bool>,
    /// Mode A's weekly run cap (`None`: no cap).
    pub weekly_cap: Option<u32>,
    pub runs_this_week: u32,
    pub system_codex: Option<SystemCodex>,
}

/// `install_codex` progress. A failure or cancel ends the call with an error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    DownloadStarted {
        total_bytes: u64,
    },
    Progress {
        downloaded_bytes: u64,
        total_bytes: u64,
    },
    Verifying,
    Installing,
    Done {
        version: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CodexLoginMethod {
    Browser,
    DeviceCode,
}

/// `codex_login` progress.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LoginEvent {
    /// Codex opened the browser; `url` is the same link, for when it didn't.
    BrowserOpened {
        url: Option<String>,
    },
    DeviceCode {
        verification_url: String,
        user_code: String,
        expires_in_secs: Option<u32>,
    },
    /// Waiting for the student to finish in the browser.
    Waiting,
    Done,
}
