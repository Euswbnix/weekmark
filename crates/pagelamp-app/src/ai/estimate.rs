//! "≈ $x" before Generate, and everything that would stop the run (design §3.5, §4.1): the
//! gate's own blocks (question (b) on a cloud backend among them), the disclosure, an unpriced
//! model, and the monthly budget. Local and cheap: DB reads only, no network call.

use pagelamp_core::ai::{AiFeature, BlockReason, Destination};
use pagelamp_core::ai_gate::{
    ContextBudget, GateError, GatedContext, PlanScope, RenderedPrompt, assemble, note_context,
    plan_context, week_context,
};
use pagelamp_core::planner::PlanTasks;
use pagelamp_core::store::Store;
use pagelamp_core::views::AsOf;
use pagelamp_llm::OutputSpec;
use pagelamp_llm::profile::ProviderProfile;

use super::settings::{self, backend_key};
use super::{BackendRef, CostEstimate, EstimateRequest, prompts};
use crate::{App, AppError, AppErrorKind, Result};

/// Characters of material text an explanation may carry (design §4.2 starting value).
pub(crate) const EXPLANATION_CONTEXT_CHARS: usize = 200_000;
/// Output budgets per feature (tokens).
pub(crate) const EXPLANATION_MAX_OUTPUT: u32 = 6_000;
pub(crate) const PLAN_MAX_OUTPUT: u32 = 4_000;
pub(crate) const NOTE_MAX_OUTPUT: u32 = 1_000;
/// The default study-plan horizon (days).
pub(crate) const DEFAULT_PLAN_DAYS: u32 = 14;

impl App {
    pub(crate) fn estimate(&self, request: &EstimateRequest) -> Result<CostEstimate> {
        let feature = match request {
            EstimateRequest::StudyPlan { .. } => AiFeature::StudyPlan,
            EstimateRequest::WeeklyExplanation { .. } => AiFeature::WeeklyExplanation,
            EstimateRequest::WeeklyNote => AiFeature::WeeklyNote,
            EstimateRequest::CourseCalendar { .. } => {
                return Err(AppError::new(
                    AppErrorKind::Invalid,
                    "Reading course calendars with AI isn't available in this build yet.",
                ));
            }
        };
        let store = self.read_store()?;
        let routing = settings::routing(&store)?;
        let Some(choice) = routing.0.get(&feature).cloned() else {
            return Ok(blocked_estimate(BlockReason::NoModelChosen));
        };
        // The ChatGPT plan through Codex runs in OpenAI's cloud; its tokens are counted like
        // OpenAI's Responses wire. No key needed: an estimate never reads the keychain.
        let codex = choice.backend == BackendRef::Codex;
        let profile = if codex {
            pagelamp_llm::profile::preset("openai")
                .expect("the openai preset")
                .clone()
        } else {
            self.provider_profile(&choice.backend)?
        };
        let destination = if !codex && profile.on_device() {
            Destination::OnDevice
        } else {
            Destination::Cloud
        };
        let at = AsOf::now_local();

        // The gate decides what may be sent at all, question (b) included.
        let context = match request {
            EstimateRequest::StudyPlan {
                horizon_days,
                courses,
            } => {
                let scope = PlanScope {
                    courses: courses.clone(),
                    horizon_days: horizon_days.unwrap_or(DEFAULT_PLAN_DAYS),
                };
                plan_context(&store, &scope, at)
            }
            EstimateRequest::WeeklyExplanation { course, week } => {
                let budget = ContextBudget {
                    max_chars: EXPLANATION_CONTEXT_CHARS,
                };
                week_context(&store, course, *week, at, destination, budget)
            }
            EstimateRequest::WeeklyNote | EstimateRequest::CourseCalendar { .. } => {
                note_context(&store, at)
            }
        };
        let context = match context {
            Ok(context) => context,
            Err(GateError::Blocked(reason)) => return Ok(blocked_estimate(reason)),
            Err(GateError::Store(err)) => return Err(err.into()),
        };
        let (prompt, output, max_output) = request_shape(feature, &context);
        let estimate = pagelamp_llm::estimate::estimate(
            &profile,
            &choice.model,
            &prompt,
            &output,
            choice.effort,
            max_output,
        );
        if codex {
            // Runs count against the plan: no price and no money budget, a weekly run cap.
            return Ok(CostEstimate {
                micro_usd_upper: None,
                input_tokens: estimate.input_tokens,
                max_output_tokens: estimate.max_output_tokens,
                reasoning_allowance: estimate.reasoning_allowance,
                repair_possible: estimate.repair_possible,
                price_known: false,
                would_block: self.codex_blocks(&store)?,
            });
        }
        let would_block = self.other_blocks(
            &store,
            &choice.backend,
            &choice.model,
            &profile,
            estimate.micro_usd_upper,
            estimate.price_known,
        )?;
        Ok(CostEstimate {
            micro_usd_upper: estimate.micro_usd_upper,
            input_tokens: estimate.input_tokens,
            max_output_tokens: estimate.max_output_tokens,
            reasoning_allowance: estimate.reasoning_allowance,
            repair_possible: estimate.repair_possible,
            price_known: estimate.price_known,
            would_block,
        })
    }

    /// The ChatGPT plan: the disclosure, then the weekly run cap.
    fn codex_blocks(&self, store: &Store) -> Result<Option<BlockReason>> {
        // A Codex routing stored by an earlier build blocks here instead of running.
        if !self.chatgpt_plan_offered() {
            return Ok(Some(BlockReason::BackendDisabledInThisBuild));
        }
        let version = self.disclosure(&BackendRef::Codex)?.version;
        let acknowledged = settings::disclosures(store)?
            .get(&backend_key(&BackendRef::Codex))
            .is_some_and(|ack| ack.version == version);
        if !acknowledged {
            return Ok(Some(BlockReason::DisclosureNotAcknowledged));
        }
        if self.codex_cap_reached(store)? {
            return Ok(Some(BlockReason::WeeklyRunCapReached));
        }
        Ok(None)
    }

    /// The disclosure, an unpriced model and the budget, in that order.
    fn other_blocks(
        &self,
        store: &Store,
        backend: &super::BackendRef,
        model: &str,
        profile: &ProviderProfile,
        upper: Option<u64>,
        price_known: bool,
    ) -> Result<Option<BlockReason>> {
        let key = backend_key(backend);
        let facts = super::disclosure_for(profile);
        let acknowledged = settings::disclosures(store)?
            .get(&key)
            .is_some_and(|ack| ack.version == facts.version);
        if !acknowledged {
            return Ok(Some(BlockReason::DisclosureNotAcknowledged));
        }
        if profile.on_device() {
            return Ok(None); // free: no price, no budget
        }
        if !price_known
            && !settings::unpriced(store)?
                .get(&key)
                .is_some_and(|models| models.iter().any(|m| m == model))
        {
            return Ok(Some(BlockReason::PriceUnknownNotAcknowledged));
        }
        if let (Some(cap), Some(upper)) = (settings::monthly_budget(store)?, upper) {
            let spent = super::usage::spent_this_month(store)?;
            if spent.saturating_add(upper) > cap {
                return Ok(Some(BlockReason::BudgetReached));
            }
        }
        Ok(None)
    }
}

/// The prompt, answer format and output budget a feature's run would use.
fn request_shape(feature: AiFeature, context: &GatedContext) -> (RenderedPrompt, OutputSpec, u32) {
    match feature {
        AiFeature::StudyPlan => (
            assemble(prompts::STUDY_PLAN, context, None),
            OutputSpec::for_type::<PlanTasks>("study_plan_tasks").unwrap_or(OutputSpec::Text),
            PLAN_MAX_OUTPUT,
        ),
        AiFeature::WeeklyExplanation => (
            assemble(prompts::WEEKLY_EXPLANATION, context, None),
            OutputSpec::Text,
            EXPLANATION_MAX_OUTPUT,
        ),
        AiFeature::WeeklyNote | AiFeature::CourseCalendar => (
            assemble(prompts::WEEKLY_NOTE, context, None),
            OutputSpec::Text,
            NOTE_MAX_OUTPUT,
        ),
    }
}

/// An estimate for a run that can't start.
fn blocked_estimate(reason: BlockReason) -> CostEstimate {
    CostEstimate {
        micro_usd_upper: None,
        input_tokens: 0,
        max_output_tokens: 0,
        reasoning_allowance: 0,
        repair_possible: false,
        price_known: false,
        would_block: Some(reason),
    }
}
