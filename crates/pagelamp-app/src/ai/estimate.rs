//! "≈ $x" before Generate, and everything that would stop the run (design §3.5, §4.1): the
//! gate's own blocks (question (b) on a cloud backend among them, and a weekly note with
//! nothing to write about), the disclosure, an unpriced model, and the monthly budget. Local
//! and cheap: DB reads only, no network call.

use pagelamp_core::ai::{AiFeature, BlockReason, Destination};
use pagelamp_core::ai_gate::{
    AnswerLanguage, ContextBudget, GateError, GatedContext, PlanScope, RenderedPrompt, StudentNote,
    assemble_in, calendar_context, week_context_including,
};
use pagelamp_core::calendar::extraction::CalendarExtraction;
use pagelamp_core::planner::PlanTasks;
use pagelamp_core::store::Store;
use pagelamp_core::views::AsOf;
use pagelamp_llm::OutputSpec;
use pagelamp_llm::profile::ProviderProfile;

use super::settings::{self, backend_key};
use super::{BackendRef, CostEstimate, EstimateRequest, ModelChoice, prompts};
use crate::{App, AppError, AppErrorKind, Result};

/// Characters of material text an explanation may carry (design §4.2 starting value).
pub(crate) const EXPLANATION_CONTEXT_CHARS: usize = 200_000;
/// The same for a model on this computer (a smaller context window, design §4.2).
pub(crate) const EXPLANATION_LOCAL_CONTEXT_CHARS: usize = 24_000;
/// Output budgets per feature (tokens).
pub(crate) const EXPLANATION_MAX_OUTPUT: u32 = 6_000;
pub(crate) const PLAN_MAX_OUTPUT: u32 = 4_000;
pub(crate) const NOTE_MAX_OUTPUT: u32 = 1_000;
/// A syllabus reading (calendar design §7.3).
pub(crate) const CALENDAR_MAX_OUTPUT: u32 = 4_000;
/// Characters of material text a syllabus reading may carry, in the cloud and on this computer.
pub(crate) const CALENDAR_CONTEXT_CHARS: usize = 60_000;
pub(crate) const CALENDAR_LOCAL_CONTEXT_CHARS: usize = 24_000;
/// The default study-plan horizon (days).
pub(crate) const DEFAULT_PLAN_DAYS: u32 = 14;

impl App {
    pub(crate) fn estimate(&self, request: &EstimateRequest) -> Result<CostEstimate> {
        let feature = match request {
            EstimateRequest::StudyPlan { .. } => AiFeature::StudyPlan,
            EstimateRequest::WeeklyExplanation { .. } => AiFeature::WeeklyExplanation,
            EstimateRequest::WeeklyNote => AiFeature::WeeklyNote,
            EstimateRequest::CourseCalendar { courses } => {
                return self.calendar_estimate(courses);
            }
        };
        let store = self.read_store()?;
        let Some(choice) = feature_choice(&store, feature)? else {
            return Ok(blocked_estimate(BlockReason::NoModelChosen));
        };
        let (profile, destination) = self.estimate_profile(&choice)?;
        let codex = choice.backend == BackendRef::Codex;
        // The note's day is the reminder zone's, as in its run and `startup_tasks`.
        let at = if feature == AiFeature::WeeklyNote {
            self.local_as_of(chrono::Utc::now())
        } else {
            AsOf::now_local()
        };

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
                // No course to plan for blocks before the click, as the run refuses it.
                super::plan::writable_plan_context(&store, &scope, at)
            }
            // The run's own builder, so an include is priced as the run sends it.
            EstimateRequest::WeeklyExplanation {
                course,
                week,
                include,
            } => week_context_including(
                &store,
                course,
                *week,
                at,
                destination,
                explanation_budget(destination),
                include,
            ),
            // Nothing to write about blocks before the click, as the run refuses it.
            EstimateRequest::WeeklyNote | EstimateRequest::CourseCalendar { .. } => {
                super::note::writable_note_context(&store, at)
            }
        };
        let context = match context {
            Ok(context) => context,
            Err(GateError::Blocked(reason)) => return Ok(blocked_estimate(reason)),
            Err(GateError::Store(err)) => return Err(err.into()),
        };
        let language = estimate_language(&store, feature)?;
        let (prompt, output, max_output) = request_shape_in(feature, &context, None, language);
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

    /// "Read syllabi for N courses" (or one): each course's context gated on its own and
    /// counted; blocked only when no course can be read, or by what stops every run.
    fn calendar_estimate(&self, courses: &[String]) -> Result<CostEstimate> {
        if courses.is_empty() {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                "Choose at least one course.",
            ));
        }
        let store = self.read_store()?;
        let Some(choice) = feature_choice(&store, AiFeature::CourseCalendar)? else {
            return Ok(blocked_estimate(BlockReason::NoModelChosen));
        };
        let (profile, destination) = self.estimate_profile(&choice)?;
        let at = AsOf::now_local();
        let mut total: Option<pagelamp_llm::estimate::Estimate> = None;
        let mut first_block = None;
        for course in courses {
            let course = store.resolve_course_with(course, true)?;
            let signals = store.calendar_signals(&course.id)?;
            let budget = calendar_budget(destination);
            match calendar_context(&store, &course.id, at, destination, budget, &signals) {
                Ok(context) => {
                    let (prompt, output, max_output) =
                        request_shape(AiFeature::CourseCalendar, &context);
                    let one = pagelamp_llm::estimate::estimate(
                        &profile,
                        &choice.model,
                        &prompt,
                        &output,
                        choice.effort,
                        max_output,
                    );
                    total = Some(match total {
                        None => one,
                        Some(sum) => pagelamp_llm::estimate::Estimate {
                            input_tokens: sum.input_tokens + one.input_tokens,
                            max_output_tokens: sum.max_output_tokens + one.max_output_tokens,
                            reasoning_allowance: sum.reasoning_allowance + one.reasoning_allowance,
                            repair_possible: sum.repair_possible || one.repair_possible,
                            micro_usd_upper: sum
                                .micro_usd_upper
                                .zip(one.micro_usd_upper)
                                .map(|(a, b)| a + b),
                            price_known: sum.price_known && one.price_known,
                        },
                    });
                }
                Err(GateError::Blocked(reason)) => {
                    first_block.get_or_insert(reason);
                }
                Err(GateError::Store(err)) => return Err(err.into()),
            }
        }
        let Some(estimate) = total else {
            return Ok(blocked_estimate(
                first_block.unwrap_or(BlockReason::NoReadableMaterials),
            ));
        };
        let codex = choice.backend == BackendRef::Codex;
        let would_block = if codex {
            self.codex_blocks(&store)?
        } else {
            self.other_blocks(
                &store,
                &choice.backend,
                &choice.model,
                &profile,
                estimate.micro_usd_upper,
                estimate.price_known,
            )?
        };
        Ok(CostEstimate {
            micro_usd_upper: if codex {
                None
            } else {
                estimate.micro_usd_upper
            },
            input_tokens: estimate.input_tokens,
            max_output_tokens: estimate.max_output_tokens,
            reasoning_allowance: estimate.reasoning_allowance,
            repair_possible: estimate.repair_possible,
            price_known: !codex && estimate.price_known,
            would_block,
        })
    }

    /// The profile tokens are counted with, and where the model runs. The ChatGPT plan through
    /// Codex runs in OpenAI's cloud; its tokens are counted like OpenAI's Responses wire. No key
    /// needed: an estimate never reads the keychain.
    pub(crate) fn estimate_profile(
        &self,
        choice: &ModelChoice,
    ) -> Result<(ProviderProfile, Destination)> {
        if choice.backend == BackendRef::Codex {
            let profile = pagelamp_llm::profile::preset("openai")
                .expect("the openai preset")
                .clone();
            return Ok((profile, Destination::Cloud));
        }
        let profile = self.provider_profile(&choice.backend)?;
        let destination = if profile.on_device() {
            Destination::OnDevice
        } else {
            Destination::Cloud
        };
        Ok((profile, destination))
    }

    /// What stops a run of `choice` with this prompt besides the gate: the disclosure, then the
    /// ChatGPT plan's weekly cap, or an unpriced model and the monthly budget (which the
    /// student may go past for this run with `override_budget`).
    pub(crate) fn run_blocks(
        &self,
        store: &Store,
        choice: &ModelChoice,
        prompt: &RenderedPrompt,
        output: &OutputSpec,
        max_output: u32,
        override_budget: bool,
    ) -> Result<Option<BlockReason>> {
        if choice.backend == BackendRef::Codex {
            return self.codex_blocks(store);
        }
        let (profile, _) = self.estimate_profile(choice)?;
        let estimate = pagelamp_llm::estimate::estimate(
            &profile,
            &choice.model,
            prompt,
            output,
            choice.effort,
            max_output,
        );
        let block = self.other_blocks(
            store,
            &choice.backend,
            &choice.model,
            &profile,
            estimate.micro_usd_upper,
            estimate.price_known,
        )?;
        Ok(block.filter(|reason| !(override_budget && *reason == BlockReason::BudgetReached)))
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

/// The model a feature is routed to, if any.
pub(crate) fn feature_choice(store: &Store, feature: AiFeature) -> Result<Option<ModelChoice>> {
    Ok(settings::routing(store)?.0.get(&feature).cloned())
}

/// How much course text an explanation may carry: ≈ 200k characters in the cloud, ≈ 24k on
/// this computer.
pub(crate) fn explanation_budget(destination: Destination) -> ContextBudget {
    ContextBudget {
        max_chars: match destination {
            Destination::Cloud => EXPLANATION_CONTEXT_CHARS,
            Destination::OnDevice => EXPLANATION_LOCAL_CONTEXT_CHARS,
        },
    }
}

/// How much course text a syllabus reading may carry: ≈ 60k characters in the cloud, ≈ 24k on
/// this computer (calendar design §7.3).
pub(crate) fn calendar_budget(destination: Destination) -> ContextBudget {
    ContextBudget {
        max_chars: match destination {
            Destination::Cloud => CALENDAR_CONTEXT_CHARS,
            Destination::OnDevice => CALENDAR_LOCAL_CONTEXT_CHARS,
        },
    }
}

/// The prompt, answer format and output budget a feature's run would use.
pub(crate) fn request_shape(
    feature: AiFeature,
    context: &GatedContext,
) -> (RenderedPrompt, OutputSpec, u32) {
    request_shape_in(feature, context, None, None)
}

/// `request_shape` with the student's own note (a study plan's "focus on the midterm"), sent
/// as data.
pub(crate) fn request_shape_with_note(
    feature: AiFeature,
    context: &GatedContext,
    note: Option<&StudentNote>,
) -> (RenderedPrompt, OutputSpec, u32) {
    request_shape_in(feature, context, note, None)
}

/// `request_shape_with_note` with the answer-language line the run adds (an explanation's and
/// the note's), in the answer format the run asks for.
pub(crate) fn request_shape_in(
    feature: AiFeature,
    context: &GatedContext,
    note: Option<&StudentNote>,
    language: Option<AnswerLanguage>,
) -> (RenderedPrompt, OutputSpec, u32) {
    match feature {
        AiFeature::StudyPlan => (
            assemble_in(prompts::STUDY_PLAN, context, note, language),
            OutputSpec::for_type::<PlanTasks>("study_plan_tasks").unwrap_or(OutputSpec::Text),
            PLAN_MAX_OUTPUT,
        ),
        AiFeature::WeeklyExplanation => (
            assemble_in(prompts::WEEKLY_EXPLANATION, context, None, language),
            super::explain::explanation_output(),
            EXPLANATION_MAX_OUTPUT,
        ),
        AiFeature::WeeklyNote => (
            assemble_in(prompts::WEEKLY_NOTE, context, None, language),
            super::note::note_output(),
            NOTE_MAX_OUTPUT,
        ),
        AiFeature::CourseCalendar => (
            assemble_in(prompts::COURSE_CALENDAR, context, None, language),
            OutputSpec::for_type::<CalendarExtraction>("course_calendar")
                .unwrap_or(OutputSpec::Text),
            CALENDAR_MAX_OUTPUT,
        ),
    }
}

/// The answer-language line the estimate assumes for a run of `feature`, never shorter than the
/// run's: an explanation in the course's language gets that line exactly; one in the UI's
/// language, and the note (whose language is the UI's too), get the longest line the run could
/// add, since the estimate doesn't know the UI's language. A failed read is an error, as in
/// the run.
fn estimate_language(store: &Store, feature: AiFeature) -> Result<Option<AnswerLanguage>> {
    Ok(match feature {
        AiFeature::WeeklyExplanation => {
            let setting: super::OutputLanguage = store
                .setting_or_absent(settings::OUTPUT_LANGUAGE)?
                .unwrap_or_default();
            Some(match setting {
                super::OutputLanguage::Course => AnswerLanguage::CourseLanguage,
                super::OutputLanguage::Ui => LONGEST_UI_LANGUAGE,
            })
        }
        AiFeature::WeeklyNote => Some(LONGEST_UI_LANGUAGE),
        AiFeature::StudyPlan | AiFeature::CourseCalendar => None,
    })
}

/// Of the languages a run in the UI's language can use (English, Simplified Chinese), the one
/// with the longer instruction.
const LONGEST_UI_LANGUAGE: AnswerLanguage = AnswerLanguage::SimplifiedChinese;

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

#[cfg(test)]
mod tests {
    use pagelamp_core::ai::Effort;

    use super::super::OutputLanguage;
    use super::super::explain::{answer_language, explanation_output};
    use super::super::note::{note_language, note_output};
    use super::*;

    /// For every output-language setting and UI language, the estimate's prompt costs at least
    /// what the run's does, on a model whose structured answers may need a second, repaired
    /// call (`structured_output: false` in the catalog): the estimate and the run's own check
    /// (`run_blocks`) agree on the answer format, and the estimate's language line is never the
    /// shorter one.
    #[test]
    fn the_estimate_is_never_below_the_run() {
        let profile = pagelamp_llm::profile::preset("openai").unwrap();
        let model = "gpt-5.2-pro";
        let price = |prompt: &RenderedPrompt, output: &OutputSpec, max_output: u32| {
            pagelamp_llm::estimate::estimate(
                profile,
                model,
                prompt,
                output,
                Effort::Lowest,
                max_output,
            )
        };
        let context = GatedContext::empty();
        let store = Store::open_in_memory().unwrap();
        for setting in [OutputLanguage::Ui, OutputLanguage::Course] {
            store
                .set_setting(settings::OUTPUT_LANGUAGE, &setting)
                .unwrap();
            let language = estimate_language(&store, AiFeature::WeeklyExplanation).unwrap();
            let (prompt, output, max_output) =
                request_shape_in(AiFeature::WeeklyExplanation, &context, None, language);
            let estimated = price(&prompt, &output, max_output);
            assert!(estimated.repair_possible, "{setting:?}: the run's format");
            for ui in [None, Some("en"), Some("zh-CN")] {
                let run = assemble_in(
                    prompts::WEEKLY_EXPLANATION,
                    &context,
                    None,
                    Some(answer_language(setting, ui)),
                );
                let run = price(&run, &explanation_output(), EXPLANATION_MAX_OUTPUT);
                assert!(
                    estimated.micro_usd_upper >= run.micro_usd_upper,
                    "{setting:?} {ui:?}: {estimated:?} < {run:?}"
                );
            }
        }
        let language = estimate_language(&store, AiFeature::WeeklyNote).unwrap();
        let (prompt, output, max_output) =
            request_shape_in(AiFeature::WeeklyNote, &context, None, language);
        let estimated = price(&prompt, &output, max_output);
        for ui in [None, Some("en"), Some("zh-CN")] {
            let run = assemble_in(
                prompts::WEEKLY_NOTE,
                &context,
                None,
                Some(note_language(ui)),
            );
            let run = price(&run, &note_output(), NOTE_MAX_OUTPUT);
            assert!(
                estimated.micro_usd_upper >= run.micro_usd_upper,
                "note {ui:?}: {estimated:?} < {run:?}"
            );
        }
    }
}
