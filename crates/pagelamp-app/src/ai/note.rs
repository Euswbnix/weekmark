//! The AI weekly note (model-access design §5.3; beta.2): a few sentences on the week and the
//! three things to focus on, from the courses' structure and the study plan's progress only
//! (`note_context`: never material text, for every course whatever its AI state).
//!
//! - `write_weekly_note`: started by a click, registered as a run and gated like every run;
//!   the answer is schema-constrained JSON. Focus items that would produce graded work are left
//!   out, a course id the context doesn't have is dropped, and an empty note is `bad_output`.
//!   The note is stored with its AI label (`meta`); the latest 5 runs are kept.
//! - "Prepare it when I open PageLamp on Monday" (`set_prepare_weekly_note_on_monday`): an
//!   opt-in for the student's own API key or a model on this computer only (modes C and D).
//!   The ChatGPT and Claude plans never run in the background (plan D27, revised 2026-09-29).
//!   `startup_tasks` says when to prepare it: the opt-in is on, the note's model allows it,
//!   it is Monday in the reminder zone (wall-clock, so a DST change doesn't move it), no
//!   automatic note was tried yet that Monday, there is no note of that day, and there is
//!   something to write about. Every surface then calls `write_weekly_note` with `automatic`,
//!   which checks the same again and records the try as it starts: one try a Monday whatever
//!   its outcome (a surface that asks again every hour never repeats a failed, paid run). A
//!   week with nothing to write about records no try, so a course synced later that Monday
//!   still gets its note. An automatic run never goes over the budget.
//! - Nothing to write about (no active course, no deadline in the next 7 days, no plan item)
//!   is `Blocked(NothingToWrite)`, which `estimate_generation` already says before the click.
//! - `weekly_notes`, `delete_weekly_note`; `delete_generated` covers notes too.

use chrono::{Datelike, Duration, NaiveDate, SubsecRound, Utc, Weekday};
use pagelamp_core::ai::{AiFeature, BlockReason, ModelErrorKind};
use pagelamp_core::ai_gate::{
    AnswerLanguage, ContextManifest, ContextSummary, GateError, GatedContext, assemble_in,
    note_context,
};
use pagelamp_core::model::Timestamp;
use pagelamp_core::planner::text_produces_graded_work;
use pagelamp_core::store::{GenerationRecord, GenerationStatus, Store};
use pagelamp_core::views::AsOf;
use pagelamp_llm::OutputSpec;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::estimate::NOTE_MAX_OUTPUT;
use super::prompts::{PROMPT_VERSION, WEEKLY_NOTE};
use super::run::RunRequest;
use super::settings::{NOTE_ON_MONDAY, NOTE_TRIED_ON, backend_key};
use super::{BackendRef, GenEvent, GenStage, GenerationMeta, ModelChoice, feature_choice};
use crate::{App, AppError, AppErrorKind, Result};

/// The most focus items kept.
pub const MAX_FOCUS_ITEMS: usize = 3;
/// The longest note kept (characters); a longer one is cut at a sentence end when it can be.
const MAX_NOTE_CHARS: usize = 1_200;

/// Options of one weekly note run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WeeklyNoteOptions {
    /// The UI's language, e.g. "en" or "zh-CN" (English for any other).
    pub ui_language: Option<String>,
    /// The student chose to go over the monthly budget for this run (never for `automatic`).
    pub override_budget: bool,
    /// Started because `startup_tasks().prepare_weekly_note` said so, not by a click: refused
    /// unless that still holds.
    pub automatic: bool,
}

/// A weekly note. `meta` is its AI label (backend, model, when, on this computer or not)
/// and what was sent.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct WeeklyNote {
    pub meta: GenerationMeta,
    /// The Monday of the week the note is for (the student's date).
    pub week_of: NaiveDate,
    /// 3–5 sentences, plain text.
    pub text: String,
    /// The things to focus on this week, most important first (at most 3).
    pub focus: Vec<NoteFocus>,
    /// Prepared at launch on Monday (the opt-in), not by a click.
    pub automatic: bool,
    /// Focus items left out because they would produce graded work.
    pub graded_work_left_out: u32,
}

/// One thing to focus on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoteFocus {
    pub text: String,
    /// The course it is about, when it names one of the note's courses.
    pub course_id: Option<String>,
}

/// The weekly note's settings (Settings → AI).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WeeklyNoteSettings {
    /// "Prepare it when I open PageLamp on Monday", as the student set it.
    pub prepare_on_monday: bool,
    /// The note's model allows it now: an API key or a model on this computer (modes C and
    /// D). False with the ChatGPT or Claude plan, or no model chosen: then nothing is
    /// prepared, whatever `prepare_on_monday` says.
    pub prepare_on_monday_allowed: bool,
}

/// The model's answer (schema-constrained).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub(crate) struct NoteAnswer {
    /// 3–5 sentences.
    pub note: String,
    /// The three things to focus on, most important first.
    pub focus: Vec<AnswerFocus>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub(crate) struct AnswerFocus {
    pub text: String,
    /// A course_id from the context, or null.
    pub course_id: Option<String>,
}

/// What a generation row keeps about what was sent (no text).
#[derive(Serialize)]
struct Summary<'a> {
    context: &'a ContextSummary,
    manifest: &'a ContextManifest,
}

impl App {
    /// Write a weekly note (see the module docs). `Blocked` for what stops a run (no model
    /// chosen, nothing to write about, the disclosure, the budget…); `Invalid` for an
    /// `automatic` run that isn't due; `Busy` while a run with `generation_id` goes on.
    pub async fn write_weekly_note(
        &self,
        generation_id: &str,
        options: WeeklyNoteOptions,
        on_event: impl Fn(GenEvent) + Send + Sync,
    ) -> Result<WeeklyNote> {
        self.write_weekly_note_at(Utc::now(), generation_id, options, on_event)
            .await
    }

    /// `write_weekly_note` as if it were `now` (tests: a Monday's automatic run).
    #[doc(hidden)]
    pub async fn write_weekly_note_at(
        &self,
        now: Timestamp,
        generation_id: &str,
        options: WeeklyNoteOptions,
        on_event: impl Fn(GenEvent) + Send + Sync,
    ) -> Result<WeeklyNote> {
        let result = self
            .write_note(now, generation_id, &options, &on_event)
            .await;
        on_event(GenEvent::Finished { ok: result.is_ok() });
        result
    }

    /// The kept weekly notes, newest first. A stored note that can't be read is skipped, with
    /// a warning that names its id (never its text).
    pub fn weekly_notes(&self) -> Result<Vec<WeeklyNote>> {
        let store = self.read_store()?;
        Ok(store
            .course_free_generations(AiFeature::WeeklyNote, Some(GenerationStatus::Accepted))?
            .into_iter()
            .filter_map(|record| {
                let note = record
                    .output_json
                    .as_deref()
                    .and_then(|json| serde_json::from_str(json).ok());
                if note.is_none() {
                    tracing::warn!(target: "pagelamp::ai", id = %record.id, "unreadable weekly note skipped");
                }
                note
            })
            .collect())
    }

    /// Delete one kept note. `NotFound` for an unknown id or one that isn't a weekly note.
    pub fn delete_weekly_note(&self, generation_id: &str) -> Result<()> {
        let store = self.write_store()?;
        let is_note = store
            .generation(generation_id)?
            .is_some_and(|record| record.feature == AiFeature::WeeklyNote);
        if !is_note || !store.delete_generation(generation_id)? {
            return Err(AppError::new(
                AppErrorKind::NotFound,
                format!("There is no weekly note {generation_id}."),
            ));
        }
        Ok(())
    }

    /// The weekly note's settings. A failed read is an error.
    pub fn weekly_note_settings(&self) -> Result<WeeklyNoteSettings> {
        let store = self.read_store()?;
        Ok(WeeklyNoteSettings {
            prepare_on_monday: store.setting_or_absent(NOTE_ON_MONDAY)?.unwrap_or(false),
            prepare_on_monday_allowed: allows_background_runs(
                feature_choice(&store, AiFeature::WeeklyNote)?.as_ref(),
            ),
        })
    }

    /// Turn "prepare it when I open PageLamp on Monday" on or off. Turning it on is `Invalid`
    /// unless the note's model is an API key or a model on this computer (modes C and D, plan
    /// D27); turning it off always works.
    pub fn set_prepare_weekly_note_on_monday(&self, on: bool) -> Result<WeeklyNoteSettings> {
        {
            let store = self.write_store()?;
            if on
                && !allows_background_runs(feature_choice(&store, AiFeature::WeeklyNote)?.as_ref())
            {
                return Err(AppError::new(
                    AppErrorKind::Invalid,
                    "Preparing the note on Monday needs an API key or a model on this computer \
                     for weekly notes.",
                ));
            }
            store.set_setting(NOTE_ON_MONDAY, &on)?;
        }
        self.weekly_note_settings()
    }

    /// Whether a surface should prepare the weekly note now, at `now` (see the module docs).
    pub(crate) fn weekly_note_due(&self, now: Timestamp) -> Result<bool> {
        self.note_due_in(&self.read_store()?, self.local_as_of(now))
    }

    fn note_due_in(&self, store: &Store, at: AsOf) -> Result<bool> {
        Ok(at.today.weekday() == Weekday::Mon
            && allows_background_runs(feature_choice(store, AiFeature::WeeklyNote)?.as_ref())
            && self.monday_open(store, at.today)?
            // Last, as the heaviest: only on an opted-in Monday before its try.
            && has_something_to_write(store, at)?)
    }

    /// The opt-in is on, no automatic note was tried `today`, and none was written today (a
    /// note the student wrote first is this week's).
    fn monday_open(&self, store: &Store, today: NaiveDate) -> pagelamp_core::Result<bool> {
        let opted_in: bool = store.setting_or_absent(NOTE_ON_MONDAY)?.unwrap_or(false);
        let tried_on: Option<NaiveDate> = store.setting_or_absent(NOTE_TRIED_ON)?;
        if !opted_in || tried_on == Some(today) {
            return Ok(false);
        }
        let latest = store
            .course_free_generations(AiFeature::WeeklyNote, Some(GenerationStatus::Accepted))?;
        Ok(!latest
            .first()
            .is_some_and(|record| self.local_as_of(record.created_at).today == today))
    }

    /// An automatic run starts: `Invalid` unless it is due, else its try is recorded. The
    /// re-check and the record are one write transaction, so two runs (or two apps) never both
    /// start one.
    fn start_automatic_note(&self, at: AsOf) -> Result<()> {
        let today = at.today;
        let not_due = || {
            AppError::new(
                AppErrorKind::Invalid,
                "The weekly note isn't due to be prepared now.",
            )
        };
        {
            let store = self.read_store()?;
            // The cheap checks first: a Monday, and a model that may run in the background.
            if today.weekday() != Weekday::Mon
                || !allows_background_runs(feature_choice(&store, AiFeature::WeeklyNote)?.as_ref())
            {
                return Err(not_due());
            }
            // Nothing to write about: not due, and no try is recorded, so a course synced later
            // that Monday still gets its note. Outside the write transaction, since
            // `note_context` reads in its own. Should the week empty between here and the run's
            // own check, the run stops with `Blocked(NothingToWrite)`: nothing is sent, its try
            // is recorded, and the shells say nothing.
            if !has_something_to_write(&store, at)? {
                return Err(not_due());
            }
        }
        let store = self.write_store()?;
        let started = store.in_transaction(|store| {
            let open = self.monday_open(store, today)?;
            if open {
                store.set_setting(NOTE_TRIED_ON, &today)?;
            }
            Ok(open)
        })?;
        if !started {
            return Err(not_due());
        }
        Ok(())
    }

    async fn write_note(
        &self,
        now: Timestamp,
        generation_id: &str,
        options: &WeeklyNoteOptions,
        on_event: &(dyn Fn(GenEvent) + Send + Sync),
    ) -> Result<WeeklyNote> {
        // Registered until the note is stored.
        let (_run, cancel) = self.register_run(generation_id, None)?;
        if options.automatic {
            self.start_automatic_note(self.local_as_of(now))?;
        }
        on_event(GenEvent::Stage {
            stage: GenStage::BuildingContext,
        });
        let at = self.local_as_of(now);
        let started = now.trunc_subsecs(0);
        let week_of =
            at.today - Duration::days(i64::from(at.today.weekday().num_days_from_monday()));
        // Everything read before the model call; the store isn't held across it.
        let (choice, context, prompt, output) = {
            let store = self.read_store()?;
            let Some(choice) = feature_choice(&store, AiFeature::WeeklyNote)? else {
                return Err(blocked(BlockReason::NoModelChosen));
            };
            let (profile, _) = self.estimate_profile(&choice)?;
            let context = match writable_note_context(&store, at) {
                Ok(context) => context,
                Err(GateError::Blocked(reason)) => return Err(blocked(reason)),
                Err(GateError::Store(err)) => return Err(err.into()),
            };
            let language = note_language(options.ui_language.as_deref());
            let prompt = assemble_in(WEEKLY_NOTE, &context, None, Some(language));
            let output = note_output();
            // An automatic run never goes over the budget.
            let override_budget = options.override_budget && !options.automatic;
            if let Some(reason) = self.run_blocks(
                &store,
                &choice,
                &prompt,
                &output,
                NOTE_MAX_OUTPUT,
                override_budget,
            )? {
                return Err(blocked(reason));
            }
            let estimate = pagelamp_llm::estimate::estimate(
                &profile,
                &choice.model,
                &prompt,
                &output,
                choice.effort,
                NOTE_MAX_OUTPUT,
            );
            on_event(GenEvent::Context {
                summary: context.summary().clone(),
                input_tokens: Some(estimate.input_tokens),
            });
            (choice, context, prompt, output)
        };
        let run = self
            .run_model(
                RunRequest {
                    generation_id,
                    cancel: &cancel,
                    feature: AiFeature::WeeklyNote,
                    choice: &choice,
                    prompt,
                    output,
                    max_output_tokens: NOTE_MAX_OUTPUT,
                },
                on_event,
            )
            .await;
        let summary = serde_json::to_string(&Summary {
            context: context.summary(),
            manifest: context.manifest(),
        })
        .ok();
        let record = |status, output_json, error_kind: Option<&str>| GenerationRecord {
            id: generation_id.to_string(),
            feature: AiFeature::WeeklyNote,
            course_id: None,
            week: None,
            backend: backend_key(&choice.backend),
            model: choice.model.clone(),
            status,
            created_at: started,
            prompt_version: PROMPT_VERSION,
            output_json,
            summary_json: summary.clone(),
            error_kind: error_kind.map(str::to_string),
            week_starts_on: None,
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
        let checked = run
            .json
            .clone()
            .and_then(|json| serde_json::from_value::<NoteAnswer>(json).ok())
            .and_then(|answer| checked_answer(answer, &context));
        let Some((text, focus, graded_work_left_out)) = checked else {
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
                    "The model's answer wasn't a weekly note.",
                )
            });
        };
        let note = WeeklyNote {
            meta: GenerationMeta {
                generation_id: generation_id.to_string(),
                feature: AiFeature::WeeklyNote,
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
            week_of,
            text,
            focus,
            automatic: options.automatic,
            graded_work_left_out,
        };
        self.write_store()?.record_generation(&record(
            GenerationStatus::Accepted,
            serde_json::to_string(&note).ok(),
            None,
        ))?;
        Ok(note)
    }
}

/// The note's answer format.
pub(crate) fn note_output() -> OutputSpec {
    OutputSpec::for_type::<NoteAnswer>("weekly_note").unwrap_or(OutputSpec::Text)
}

/// Whether `choice` may run in the background: the student's own API key or a model on this
/// computer (modes C and D). The ChatGPT and Claude plans may not (plan D27).
fn allows_background_runs(choice: Option<&ModelChoice>) -> bool {
    matches!(
        choice.map(|choice| &choice.backend),
        Some(BackendRef::Provider { .. })
    )
}

/// The note text, the focus items and how many were left out as graded work; `None` when no
/// note or no focus item is left.
fn checked_answer(
    answer: NoteAnswer,
    context: &GatedContext,
) -> Option<(String, Vec<NoteFocus>, u32)> {
    let text = cut(answer.note.trim(), MAX_NOTE_CHARS);
    let mut graded = 0u32;
    let mut focus = Vec::new();
    for item in answer.focus {
        let item_text = item.text.trim();
        if item_text.is_empty() {
            continue;
        }
        if text_produces_graded_work(item_text, None) {
            graded += 1;
            continue;
        }
        let course_id = item.course_id.map(|id| id.trim().to_string()).filter(|id| {
            context
                .summary()
                .courses
                .iter()
                .any(|course| course.course_id == *id)
        });
        focus.push(NoteFocus {
            text: item_text.to_string(),
            course_id,
        });
    }
    focus.truncate(MAX_FOCUS_ITEMS);
    if text.is_empty() || (focus.is_empty() && graded == 0) {
        return None;
    }
    Some((text, focus, graded))
}

/// `text` cut to `max` characters, at the last sentence end inside them when there is one.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    match head.rfind(['.', '!', '?', '。', '！', '？']) {
        Some(end) => head[..end + head[end..].chars().next().map_or(1, char::len_utf8)].to_string(),
        None => head,
    }
}

/// The note's language: the UI's when PageLamp speaks it, else English.
pub(crate) fn note_language(ui_language: Option<&str>) -> AnswerLanguage {
    match ui_language.map(str::to_ascii_lowercase).as_deref() {
        Some("zh" | "zh-cn" | "zh-hans" | "zh-hans-cn") => AnswerLanguage::SimplifiedChinese,
        _ => AnswerLanguage::English,
    }
}

/// The note's context, or `Blocked(NothingToWrite)` when there is nothing to write about (no
/// active course, no deadline in the next 7 days, no plan item): the one rule the estimate, a
/// run and `startup_tasks` share.
pub(crate) fn writable_note_context(
    store: &Store,
    at: AsOf,
) -> std::result::Result<GatedContext, GateError> {
    let context = note_context(store, at)?;
    if context.is_empty() {
        return Err(GateError::Blocked(BlockReason::NothingToWrite));
    }
    Ok(context)
}

/// Whether the note has something to write about at `at` (`writable_note_context`).
fn has_something_to_write(store: &Store, at: AsOf) -> Result<bool> {
    match writable_note_context(store, at) {
        Ok(_) => Ok(true),
        Err(GateError::Blocked(_)) => Ok(false),
        Err(GateError::Store(err)) => Err(err.into()),
    }
}

fn blocked(reason: BlockReason) -> AppError {
    AppError::blocked(
        reason,
        match reason {
            BlockReason::NoModelChosen => "Choose a model for weekly notes first.",
            BlockReason::NothingToWrite => {
                "There is nothing to write about this week: no active course, no deadline in the \
                 next 7 days and no study plan item."
            }
            _ => "PageLamp can't start this weekly note.",
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_api_keys_and_local_models_may_run_in_the_background() {
        let choice = |backend| ModelChoice {
            backend,
            model: "m".into(),
            effort: pagelamp_core::ai::Effort::Lowest,
        };
        assert!(!allows_background_runs(None));
        assert!(!allows_background_runs(Some(&choice(BackendRef::Codex))));
        assert!(!allows_background_runs(Some(&choice(
            BackendRef::ClaudeCode
        ))));
        assert!(allows_background_runs(Some(&choice(
            BackendRef::Provider {
                provider_id: "ollama".into()
            }
        ))));
    }

    #[test]
    fn a_long_note_is_cut_at_a_sentence_end() {
        assert_eq!(cut("Short.", 10), "Short.");
        assert_eq!(cut("One. Two three four.", 12), "One.");
        assert_eq!(cut("第一句。第二句很长很长", 8), "第一句。");
        assert_eq!(cut("no end at all here", 7), "no end ");
    }
}
