//! Weekly explanations (model-access design §5.2): one week of a course explained from its
//! materials' text, every paragraph citing the material it comes from.
//!
//! - `explain_week`: registered as a run from the start; gated (the course must be visible and
//!   `readable`; on a cloud backend question (b) `not_allowed` blocks it, `unanswered` and
//!   `not_sure` go ahead with the course's one-time reminder, D37 option 2 and D49); the
//!   materials that look like assessments are left out unless the student includes them. The
//!   answer is schema-constrained JSON: each paragraph's citation handles are resolved
//!   locally, unknown ones are dropped and counted, a paragraph left without one is dropped,
//!   and too many drops make it `bad_output`. Written in the output language (a setting: the
//!   UI's language or the course's).
//! - `saved_explanations`: the kept explanations (5 per course and week), each marked `stale`
//!   when its materials changed or new ones appeared that week.

use std::collections::HashSet;

use chrono::{SubsecRound, Utc};
use pagelamp_core::ai::{AiFeature, BlockReason, Destination, MaterialSharing, ModelErrorKind};
use pagelamp_core::ai_gate::{
    AnswerLanguage, ContextManifest, ContextSummary, GateError, GatedContext, LeftOutMaterial,
    assemble_in, week_changed, week_context_including,
};
use pagelamp_core::model::AiPolicy;
use pagelamp_core::store::{GenerationRecord, GenerationStatus};
use pagelamp_core::term::phase::week_starts_on;
use pagelamp_core::views::{self, AsOf};
use pagelamp_llm::OutputSpec;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::estimate::{EXPLANATION_MAX_OUTPUT, explanation_budget};
use super::prompts::{PROMPT_VERSION, WEEKLY_EXPLANATION};
use super::run::RunRequest;
use super::settings::{OUTPUT_LANGUAGE, backend_key};
use super::{GenEvent, GenNoticeCode, GenStage, GenerationMeta, feature_choice};
use crate::{App, AppError, AppErrorKind, Result};

/// The most check questions kept.
const MAX_CHECK_QUESTIONS: usize = 3;

/// The language explanations are written in (Settings → AI → Output language).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutputLanguage {
    /// The language the student uses PageLamp in (`ExplainOptions::ui_language`).
    #[default]
    Ui,
    /// The course materials' own language.
    Course,
}

/// Options of one explanation run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ExplainOptions {
    /// Materials the left-out list offered, to send this time ("include").
    pub include: Vec<String>,
    /// The UI's language, e.g. "en" or "zh-CN" (English for any other).
    pub ui_language: Option<String>,
    /// The student chose to go over the monthly budget for this run.
    pub override_budget: bool,
}

/// One week explained.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct WeeklyExplanation {
    pub meta: GenerationMeta,
    pub course_id: String,
    /// The week explained; `None` when the course's weeks are unknown (recent materials).
    pub week: Option<u32>,
    pub sections: Vec<ExplanationSection>,
    /// 2–3 short questions that check understanding.
    pub check_questions: Vec<String>,
    /// Materials not sent, and why ("include" sends one next time).
    pub left_out: Vec<LeftOutMaterial>,
    /// Its materials changed or new ones appeared since it was written: regenerate?
    pub stale: bool,
    /// This run carried the course's one-time question (b) reminder.
    pub sharing_reminder: bool,
    /// Citations of handles the materials don't have, dropped (with any paragraph left
    /// without a citation).
    pub dropped_citations: u32,
    /// The course's AI policy asks to cite AI use: the footer says so.
    pub cite_ai_use: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExplanationSection {
    pub heading: String,
    pub paragraphs: Vec<ExplanationParagraph>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExplanationParagraph {
    /// Markdown (a UI renders a subset, never raw HTML).
    pub text: String,
    /// At least one.
    pub citations: Vec<Citation>,
}

/// A material a paragraph comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Citation {
    pub handle: String,
    pub material_id: String,
    pub title: String,
    pub locator: Option<String>,
    pub url: Option<String>,
}

/// The model's answer (schema-constrained).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub(crate) struct ExplanationAnswer {
    pub sections: Vec<AnswerSection>,
    pub check_questions: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub(crate) struct AnswerSection {
    pub heading: String,
    pub paragraphs: Vec<AnswerParagraph>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub(crate) struct AnswerParagraph {
    pub text: String,
    /// Handles like "c3".
    pub citations: Vec<String>,
}

/// What a generation row keeps about what was sent (no text).
#[derive(Serialize, Deserialize)]
struct Summary {
    context: ContextSummary,
    manifest: ContextManifest,
}

impl App {
    /// Explain one week of `course` (`week`: default the course's default week). See the
    /// module docs. `Blocked` with the course's reason (hidden, prohibited, turned off,
    /// `material_sharing_not_allowed` on a cloud backend, no readable materials) or what stops
    /// a run; `NotFound` for an unknown course; `Busy` while `generation_id` runs.
    pub async fn explain_week(
        &self,
        course: &str,
        week: Option<u32>,
        generation_id: &str,
        options: ExplainOptions,
        on_event: impl Fn(GenEvent) + Send + Sync,
    ) -> Result<WeeklyExplanation> {
        let result = self
            .write_explanation(course, week, generation_id, &options, &on_event)
            .await;
        on_event(GenEvent::Finished { ok: result.is_ok() });
        result
    }

    /// The kept explanations of `course` for `week` (`None`: every week), newest first, each
    /// with `stale` computed now.
    pub fn saved_explanations(
        &self,
        course: &str,
        week: Option<u32>,
    ) -> Result<Vec<WeeklyExplanation>> {
        let store = self.read_store()?;
        let course = store.resolve_course_with(course, true)?;
        let at = AsOf::now_local();
        // The calendar in force now: a week it moved makes an explanation stale.
        let term = views::course_timeline(&store, &course, at)?.term;
        let mut out = Vec::new();
        for record in store.generations_of(
            AiFeature::WeeklyExplanation,
            &course.id,
            week,
            GenerationStatus::Accepted,
        )? {
            let Some(mut explanation) = record
                .output_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<WeeklyExplanation>(json).ok())
            else {
                continue;
            };
            let summary: Option<Summary> = record
                .summary_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok());
            let moved = record.week_starts_on.is_some()
                && record.week.and_then(|week| week_starts_on(&term, week))
                    != record.week_starts_on;
            explanation.stale = moved
                || match summary {
                    Some(summary) => {
                        let considered: Vec<String> = summary
                            .manifest
                            .materials
                            .iter()
                            .map(|entry| entry.material_id.clone())
                            .chain(
                                summary
                                    .context
                                    .left_out
                                    .iter()
                                    .map(|m| m.material_id.clone()),
                            )
                            .collect();
                        week_changed(
                            &store,
                            &course.id,
                            record.week,
                            &summary.manifest,
                            &considered,
                            at,
                        )?
                    }
                    None => true,
                };
            out.push(explanation);
        }
        Ok(out)
    }

    /// Delete one kept explanation (the history's "Delete"): its row, with the stored answer.
    /// `NotFound` for an unknown id or one that isn't an explanation.
    pub fn delete_explanation(&self, generation_id: &str) -> Result<()> {
        let store = self.write_store()?;
        let is_explanation = store
            .generation(generation_id)?
            .is_some_and(|record| record.feature == AiFeature::WeeklyExplanation);
        if !is_explanation || !store.delete_generation(generation_id)? {
            return Err(AppError::new(
                AppErrorKind::NotFound,
                format!("There is no explanation {generation_id}."),
            ));
        }
        Ok(())
    }

    /// The output-language setting (default: the UI's language; also when unparseable). A
    /// failed read is an error.
    pub fn ai_output_language(&self) -> Result<OutputLanguage> {
        Ok(self
            .read_store()?
            .setting_or_absent(OUTPUT_LANGUAGE)?
            .unwrap_or_default())
    }

    pub fn set_ai_output_language(&self, language: OutputLanguage) -> Result<()> {
        Ok(self
            .write_store()?
            .set_setting(OUTPUT_LANGUAGE, &language)?)
    }

    async fn write_explanation(
        &self,
        course: &str,
        week: Option<u32>,
        generation_id: &str,
        options: &ExplainOptions,
        on_event: &(dyn Fn(GenEvent) + Send + Sync),
    ) -> Result<WeeklyExplanation> {
        // Registered until the explanation is stored.
        let (_run, cancel) = self.register_run(generation_id, None)?;
        on_event(GenEvent::Stage {
            stage: GenStage::BuildingContext,
        });
        let at = AsOf::now_local();
        let started = Utc::now().trunc_subsecs(0);
        // Everything read before the model call; the store isn't held across it.
        let (course, week, week_starts, choice, destination, context, prompt, output) = {
            let store = self.read_store()?;
            let course = store.resolve_course_with(course, true)?;
            let Some(choice) = feature_choice(&store, AiFeature::WeeklyExplanation)? else {
                return Err(blocked(BlockReason::NoModelChosen));
            };
            let (profile, destination) = self.estimate_profile(&choice)?;
            // From here a change to the course's AI settings stops this run
            // (`stop_course_runs`); week_context_including reads them after that.
            self.run_reads_course(generation_id, &course.id, Some(destination));
            let context = match week_context_including(
                &store,
                &course.id,
                week,
                at,
                destination,
                explanation_budget(destination),
                &options.include,
            ) {
                Ok(context) => context,
                Err(GateError::Blocked(reason)) => return Err(blocked(reason)),
                Err(GateError::Store(err)) => return Err(err.into()),
            };
            let week = views::week_materials(&store, &course.id, week, true, at)?.week;
            let term = views::course_timeline(&store, &course, at)?.term;
            let week_starts = week.and_then(|week| week_starts_on(&term, week));
            let setting: OutputLanguage = store
                .setting_or_absent(OUTPUT_LANGUAGE)?
                .unwrap_or_default();
            let language = answer_language(setting, options.ui_language.as_deref());
            let prompt = assemble_in(WEEKLY_EXPLANATION, &context, None, Some(language));
            let output = explanation_output();
            if let Some(reason) = self.run_blocks(
                &store,
                &choice,
                &prompt,
                &output,
                EXPLANATION_MAX_OUTPUT,
                options.override_budget,
            )? {
                return Err(blocked(reason));
            }
            let estimate = pagelamp_llm::estimate::estimate(
                &profile,
                &choice.model,
                &prompt,
                &output,
                choice.effort,
                EXPLANATION_MAX_OUTPUT,
            );
            on_event(GenEvent::Context {
                summary: context.summary().clone(),
                input_tokens: Some(estimate.input_tokens),
            });
            (
                course,
                week,
                week_starts,
                choice,
                destination,
                context,
                prompt,
                output,
            )
        };
        let run = self
            .run_model(
                RunRequest {
                    generation_id,
                    cancel: &cancel,
                    feature: AiFeature::WeeklyExplanation,
                    choice: &choice,
                    prompt,
                    output,
                    max_output_tokens: EXPLANATION_MAX_OUTPUT,
                },
                on_event,
            )
            .await;
        let summary = serde_json::to_string(&Summary {
            context: context.summary().clone(),
            manifest: context.manifest().clone(),
        })
        .ok();
        let record = |status, output_json, error_kind: Option<&str>| GenerationRecord {
            id: generation_id.to_string(),
            feature: AiFeature::WeeklyExplanation,
            course_id: Some(course.id.clone()),
            week,
            backend: backend_key(&choice.backend),
            model: choice.model.clone(),
            status,
            created_at: started,
            prompt_version: PROMPT_VERSION,
            output_json,
            summary_json: summary.clone(),
            error_kind: error_kind.map(str::to_string),
            week_starts_on: week_starts,
        };
        let run = match run {
            Ok(run) => run,
            Err(err) => {
                let (status, kind) = match err.kind {
                    AppErrorKind::Cancelled => (GenerationStatus::Cancelled, None),
                    _ => (
                        GenerationStatus::Failed,
                        err.model_error.map(|kind| kind.as_str()),
                    ),
                };
                self.write_store()?
                    .record_generation(&record(status, None, kind))?;
                return Err(err);
            }
        };
        on_event(GenEvent::Stage {
            stage: GenStage::Validating,
        });
        let grounded = run
            .json
            .clone()
            .and_then(|json| serde_json::from_value::<ExplanationAnswer>(json).ok())
            .and_then(|answer| ground(answer, &context));
        let Some((sections, check_questions, dropped_citations)) = grounded else {
            let bad = ModelErrorKind::BadOutput;
            self.write_store()?.record_generation(&record(
                GenerationStatus::Failed,
                None,
                Some(bad.as_str()),
            ))?;
            return Err(AppError {
                model_error: Some(bad),
                ..AppError::new(
                    AppErrorKind::Model,
                    "The model's explanation didn't cite the week's materials.",
                )
            });
        };
        let store = self.write_store()?;
        // D37 option 2 and D49: the course's first cloud run with material text reminds once.
        let sharing_reminder = reminds_about_sharing(destination, course.material_sharing)
            && store.claim_sharing_reminder(&course.id, Utc::now())?;
        if sharing_reminder {
            on_event(GenEvent::Notice {
                code: GenNoticeCode::MaterialSharingReminder,
            });
        }
        let explanation = WeeklyExplanation {
            meta: GenerationMeta {
                generation_id: generation_id.to_string(),
                feature: AiFeature::WeeklyExplanation,
                backend_label: run.backend_label,
                model: run.model,
                on_device: run.on_device,
                created_at: started,
                usage: run.cost.usage,
                est_cost_micro_usd: run.cost.micro_usd,
                estimated: run.cost.estimated,
                context: context.summary().clone(),
                prompt_version: PROMPT_VERSION,
            },
            course_id: course.id.clone(),
            week,
            sections,
            check_questions,
            left_out: context.summary().left_out.clone(),
            stale: false,
            sharing_reminder,
            dropped_citations,
            cite_ai_use: course.ai_policy == AiPolicy::AllowedWithCitation,
        };
        store.record_generation(&record(
            GenerationStatus::Accepted,
            serde_json::to_string(&explanation).ok(),
            None,
        ))?;
        Ok(explanation)
    }
}

/// The answer with every citation resolved against `context` (see the module docs): the
/// sections, the check questions and how many citations were dropped. `None` when nothing is
/// left, or more citations were made up than resolved.
fn ground(
    answer: ExplanationAnswer,
    context: &GatedContext,
) -> Option<(Vec<ExplanationSection>, Vec<String>, u32)> {
    let (mut resolved, mut dropped) = (0u32, 0u32);
    let mut sections = Vec::new();
    for section in answer.sections {
        let mut paragraphs = Vec::new();
        for paragraph in section.paragraphs {
            let mut seen = HashSet::new();
            let citations: Vec<Citation> = paragraph
                .citations
                .iter()
                .map(|handle| handle.trim())
                .filter(|handle| seen.insert(handle.to_string()))
                .filter_map(|handle| match context.resolve_citation(handle) {
                    Some(target) => {
                        resolved += 1;
                        Some(Citation {
                            handle: handle.to_string(),
                            material_id: target.material_id.clone(),
                            title: target.title.clone(),
                            locator: target.locator.clone(),
                            url: target.url.clone(),
                        })
                    }
                    None => {
                        dropped += 1;
                        None
                    }
                })
                .collect();
            let text = paragraph.text.trim().to_string();
            if citations.is_empty() || text.is_empty() {
                continue;
            }
            paragraphs.push(ExplanationParagraph { text, citations });
        }
        if !paragraphs.is_empty() {
            sections.push(ExplanationSection {
                heading: section.heading.trim().to_string(),
                paragraphs,
            });
        }
    }
    if sections.is_empty() || dropped > resolved {
        return None;
    }
    let check_questions = answer
        .check_questions
        .into_iter()
        .map(|question| question.trim().to_string())
        .filter(|question| !question.is_empty())
        .take(MAX_CHECK_QUESTIONS)
        .collect();
    Some((sections, check_questions, dropped))
}

/// Whether a run with material text going to `destination` carries question (b)'s one-time
/// reminder (D37 option 2, D49): a cloud run of a course answered `unanswered` or `not_sure`.
/// (`not_allowed` never gets here on a cloud backend: the gate blocks it.)
fn reminds_about_sharing(destination: Destination, sharing: MaterialSharing) -> bool {
    destination == Destination::Cloud
        && matches!(
            sharing,
            MaterialSharing::Unanswered | MaterialSharing::NotSure
        )
}

/// The explanation's answer format (the estimate prices the same one).
pub(crate) fn explanation_output() -> OutputSpec {
    OutputSpec::for_type::<ExplanationAnswer>("weekly_explanation").unwrap_or(OutputSpec::Text)
}

/// The answer's language: the course's, or the UI's when it is one PageLamp speaks (English
/// otherwise).
pub(crate) fn answer_language(
    setting: OutputLanguage,
    ui_language: Option<&str>,
) -> AnswerLanguage {
    match setting {
        OutputLanguage::Course => AnswerLanguage::CourseLanguage,
        OutputLanguage::Ui => match ui_language.map(str::to_ascii_lowercase).as_deref() {
            Some("zh" | "zh-cn" | "zh-hans" | "zh-hans-cn") => AnswerLanguage::SimplifiedChinese,
            _ => AnswerLanguage::English,
        },
    }
}

fn blocked(reason: BlockReason) -> AppError {
    AppError::blocked(
        reason,
        match reason {
            BlockReason::CourseHidden => "This course is hidden: show it to explain its weeks.",
            BlockReason::CoursePolicyProhibited => {
                "This course's AI policy is \"prohibited\", so its materials aren't sent to a model."
            }
            BlockReason::CourseAiTurnedOff => "AI access is turned off for this course.",
            BlockReason::MaterialSharingNotAllowed => {
                "This course's materials may not be shared with an AI service: use a model on \
                 this computer."
            }
            BlockReason::NoReadableMaterials => "This week has no material PageLamp can read.",
            BlockReason::NoModelChosen => "Choose a model for explanations first.",
            _ => "PageLamp can't start this explanation.",
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ui_language_is_checked_against_what_pagelamp_speaks() {
        use AnswerLanguage::*;
        for (ui, expected) in [
            (Some("en"), English),
            (Some("zh-CN"), SimplifiedChinese),
            (Some("zh-Hans"), SimplifiedChinese),
            (Some("fr"), English),
            (Some("zh-TW"), English),
            (None, English),
        ] {
            assert_eq!(answer_language(OutputLanguage::Ui, ui), expected, "{ui:?}");
        }
        assert_eq!(
            answer_language(OutputLanguage::Course, Some("zh-CN")),
            CourseLanguage
        );
    }

    /// Question (b), D37 option 2 and D49: which runs carry the one-time reminder.
    #[test]
    fn only_cloud_runs_of_unanswered_or_unsure_courses_remind() {
        use MaterialSharing::*;
        for sharing in MaterialSharing::ALL {
            assert!(!reminds_about_sharing(Destination::OnDevice, sharing));
        }
        assert!(reminds_about_sharing(Destination::Cloud, Unanswered));
        assert!(reminds_about_sharing(Destination::Cloud, NotSure));
        assert!(!reminds_about_sharing(Destination::Cloud, Allowed));
    }
}
