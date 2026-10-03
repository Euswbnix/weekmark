//! Plain data types of PageLamp's own model calls (v0.3 M1; docs/design/v0.3-model-access.md
//! §3.4, §3.8, §4.1). They cross the facade, so they are `Serialize + JsonSchema` with stable
//! snake_case names that the UIs translate; UIs branch on these codes, never on messages.
//!
//! The model code itself lives in `pagelamp-llm` (network) and `ai_gate` (the only producer of
//! prompt text); this module has no logic beyond names.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Why a model call was refused before anything was sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BlockReason {
    /// The course's `ai_policy` is `prohibited`.
    CoursePolicyProhibited,
    /// The student switched AI access off for the course.
    CourseAiTurnedOff,
    CourseHidden,
    NoReadableMaterials,
    /// A cloud backend, and the student answered "may not be shared" for the course.
    MaterialSharingNotAllowed,
    /// A coding-plan endpoint or key: its terms forbid use from other applications.
    CodingPlanKey,
    DisclosureNotAcknowledged,
    NoModelChosen,
    /// The run would go past the monthly budget (API keys only).
    BudgetReached,
    /// An API-key model without a known price, and the student hasn't acknowledged that the
    /// budget can't be enforced for it.
    PriceUnknownNotAcknowledged,
    /// The weekly run cap of the ChatGPT-plan mode.
    WeeklyRunCapReached,
    BackendDisabledInThisBuild,
    /// The weekly note has nothing to write about this week: no active course, no deadline in
    /// the next 7 days and no study plan item.
    NothingToWrite,
    /// A study plan has no course to plan for: no visible, active course, or every course the
    /// request names is hidden.
    NoCourseToPlan,
}

impl BlockReason {
    pub const ALL: [BlockReason; 14] = [
        BlockReason::CoursePolicyProhibited,
        BlockReason::CourseAiTurnedOff,
        BlockReason::CourseHidden,
        BlockReason::NoReadableMaterials,
        BlockReason::MaterialSharingNotAllowed,
        BlockReason::CodingPlanKey,
        BlockReason::DisclosureNotAcknowledged,
        BlockReason::NoModelChosen,
        BlockReason::BudgetReached,
        BlockReason::PriceUnknownNotAcknowledged,
        BlockReason::WeeklyRunCapReached,
        BlockReason::BackendDisabledInThisBuild,
        BlockReason::NothingToWrite,
        BlockReason::NoCourseToPlan,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            BlockReason::CoursePolicyProhibited => "course_policy_prohibited",
            BlockReason::CourseAiTurnedOff => "course_ai_turned_off",
            BlockReason::CourseHidden => "course_hidden",
            BlockReason::NoReadableMaterials => "no_readable_materials",
            BlockReason::MaterialSharingNotAllowed => "material_sharing_not_allowed",
            BlockReason::CodingPlanKey => "coding_plan_key",
            BlockReason::DisclosureNotAcknowledged => "disclosure_not_acknowledged",
            BlockReason::NoModelChosen => "no_model_chosen",
            BlockReason::BudgetReached => "budget_reached",
            BlockReason::PriceUnknownNotAcknowledged => "price_unknown_not_acknowledged",
            BlockReason::WeeklyRunCapReached => "weekly_run_cap_reached",
            BlockReason::BackendDisabledInThisBuild => "backend_disabled_in_this_build",
            BlockReason::NothingToWrite => "nothing_to_write",
            BlockReason::NoCourseToPlan => "no_course_to_plan",
        }
    }
}

/// Why a model call failed (the facade's `SourceErrorKind` for models).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelErrorKind {
    /// ChatGPT / Claude plan modes: not signed in.
    NotSignedIn,
    /// The key or sign-in was refused (401/403).
    AuthRejected,
    /// Out of credit or over a spend limit (402, quota 429s): never retried.
    BillingOrQuota,
    /// A plan's usage limit (Codex / Claude Code).
    UsageLimit,
    /// Too many requests; see `retry_after_secs`.
    RateLimited,
    /// The service is overloaded or failing (5xx, 529).
    Overloaded,
    /// The service rejected the request (400).
    InvalidRequest,
    ModelNotFound,
    ContextTooLong,
    /// The model declined to answer.
    Refused,
    /// The provider's content filter stopped the answer.
    ContentFiltered,
    Network,
    Timeout,
    /// The answer could not be used (unparseable, failed validation after one repair).
    BadOutput,
    /// The local runtime (Codex) is not installed.
    RuntimeMissing,
    /// A downloaded runtime failed its checksum or signature check.
    RuntimeVerifyFailed,
    /// Codex: "requires a newer version of Codex" (design §2.3).
    RuntimeOutdated,
    /// The backend or model can't do what was asked.
    Unsupported,
}

impl ModelErrorKind {
    pub const ALL: [ModelErrorKind; 18] = [
        ModelErrorKind::NotSignedIn,
        ModelErrorKind::AuthRejected,
        ModelErrorKind::BillingOrQuota,
        ModelErrorKind::UsageLimit,
        ModelErrorKind::RateLimited,
        ModelErrorKind::Overloaded,
        ModelErrorKind::InvalidRequest,
        ModelErrorKind::ModelNotFound,
        ModelErrorKind::ContextTooLong,
        ModelErrorKind::Refused,
        ModelErrorKind::ContentFiltered,
        ModelErrorKind::Network,
        ModelErrorKind::Timeout,
        ModelErrorKind::BadOutput,
        ModelErrorKind::RuntimeMissing,
        ModelErrorKind::RuntimeVerifyFailed,
        ModelErrorKind::RuntimeOutdated,
        ModelErrorKind::Unsupported,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ModelErrorKind::NotSignedIn => "not_signed_in",
            ModelErrorKind::AuthRejected => "auth_rejected",
            ModelErrorKind::BillingOrQuota => "billing_or_quota",
            ModelErrorKind::UsageLimit => "usage_limit",
            ModelErrorKind::RateLimited => "rate_limited",
            ModelErrorKind::Overloaded => "overloaded",
            ModelErrorKind::InvalidRequest => "invalid_request",
            ModelErrorKind::ModelNotFound => "model_not_found",
            ModelErrorKind::ContextTooLong => "context_too_long",
            ModelErrorKind::Refused => "refused",
            ModelErrorKind::ContentFiltered => "content_filtered",
            ModelErrorKind::Network => "network",
            ModelErrorKind::Timeout => "timeout",
            ModelErrorKind::BadOutput => "bad_output",
            ModelErrorKind::RuntimeMissing => "runtime_missing",
            ModelErrorKind::RuntimeVerifyFailed => "runtime_verify_failed",
            ModelErrorKind::RuntimeOutdated => "runtime_outdated",
            ModelErrorKind::Unsupported => "unsupported",
        }
    }
}

/// How hard the model should think. Mapped per wire: "lowest" is the cheapest setting the
/// model allows (`none` / `minimal`, or `low` where thinking can't be turned off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    #[default]
    Lowest,
    Low,
    Medium,
    High,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Lowest => "lowest",
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
        }
    }
}

/// A feature that runs a model.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AiFeature {
    StudyPlan,
    WeeklyExplanation,
    WeeklyNote,
    CourseCalendar,
}

impl AiFeature {
    pub const ALL: [AiFeature; 4] = [
        AiFeature::StudyPlan,
        AiFeature::WeeklyExplanation,
        AiFeature::WeeklyNote,
        AiFeature::CourseCalendar,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            AiFeature::StudyPlan => "study_plan",
            AiFeature::WeeklyExplanation => "weekly_explanation",
            AiFeature::WeeklyNote => "weekly_note",
            AiFeature::CourseCalendar => "course_calendar",
        }
    }

    /// Whether the feature sends course material text (else structure only).
    pub fn sends_material_text(self) -> bool {
        matches!(
            self,
            AiFeature::WeeklyExplanation | AiFeature::CourseCalendar
        )
    }
}

/// The student's answer to "May this course's materials be shared with an AI service?"
/// (design §4.1, question (b); `courses.material_sharing`, schema v4). Only `not_allowed` stops
/// material text from going to a cloud backend (owner decision D37, option 2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaterialSharing {
    #[default]
    Unanswered,
    Allowed,
    NotSure,
    NotAllowed,
}

impl MaterialSharing {
    pub const ALL: [MaterialSharing; 4] = [
        MaterialSharing::Unanswered,
        MaterialSharing::Allowed,
        MaterialSharing::NotSure,
        MaterialSharing::NotAllowed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MaterialSharing::Unanswered => "unanswered",
            MaterialSharing::Allowed => "allowed",
            MaterialSharing::NotSure => "not_sure",
            MaterialSharing::NotAllowed => "not_allowed",
        }
    }

    /// Whether material text may go to a model at `destination`: everything but `not_allowed`
    /// with a cloud model.
    pub fn allows(self, destination: Destination) -> bool {
        !(self == MaterialSharing::NotAllowed && destination == Destination::Cloud)
    }
}

/// Where the model that receives a context runs. Question (b) limits only cloud models; a model
/// on this computer (a local server) sends nothing anywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Destination {
    OnDevice,
    Cloud,
}

impl Destination {
    pub const ALL: [Destination; 2] = [Destination::OnDevice, Destination::Cloud];
}

/// A provider the student added, as stored (`model_providers`; the key is in the keychain).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderRow {
    pub id: String,
    pub preset: String,
    pub label: String,
    /// `pagelamp_llm::Wire::as_str`.
    pub wire: String,
    pub base_url: String,
    pub created_at: crate::model::Timestamp,
    /// The last "Test" as JSON (no key, no text).
    pub last_probe_json: Option<String>,
}

/// One model call in the usage ledger (`ai_usage`): counts and cost only, never content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageRecord {
    pub at: crate::model::Timestamp,
    /// `codex`, `claude_code` or `provider:<id>`.
    pub backend: String,
    pub model: String,
    pub feature: AiFeature,
    pub input_uncached: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub output: u64,
    pub reasoning: Option<u64>,
    /// `None`: the price is unknown, or the run counted against a plan.
    pub micro_usd: Option<u64>,
    /// `priced`, `free_on_device`, `unpriced` or `plan`.
    pub cost_basis: String,
    /// Counts estimated by PageLamp (a cancelled run).
    pub estimated: bool,
    /// `ok`, `failed` or `cancelled`.
    pub outcome: String,
}

/// What `Store::remove_all_ai_data` deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AiDataRemoved {
    pub providers: u32,
    pub generations: u32,
    pub usage_rows: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_their_json_names() {
        for reason in BlockReason::ALL {
            assert_eq!(serde_json::to_value(reason).unwrap(), reason.as_str());
        }
        for kind in ModelErrorKind::ALL {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
        }
        for effort in [Effort::Lowest, Effort::Low, Effort::Medium, Effort::High] {
            assert_eq!(serde_json::to_value(effort).unwrap(), effort.as_str());
        }
        for feature in AiFeature::ALL {
            assert_eq!(serde_json::to_value(feature).unwrap(), feature.as_str());
        }
        for answer in MaterialSharing::ALL {
            assert_eq!(serde_json::to_value(answer).unwrap(), answer.as_str());
        }
    }
}
