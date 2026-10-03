//! The course calendar in the facade (docs/design/v0.3-course-calendar.md §4, §7): the calendar
//! in force, proposals, candidate materials and reading the syllabus with AI.
//!
//! - Proposals and the calendar in force live in `course_calendars` (schema v4). A proposal
//!   changes nothing until the student accepts it; accepting or the dates form puts one
//!   calendar in force and mirrors it into the student's term dates for older readers.
//! - Readers: the deterministic scan (no model) and "Read the syllabus with AI": gated like
//!   every model run (`ai_gate::calendar_context`, question (b), the disclosure, the budget or
//!   weekly cap), run on the chosen backend, checked against the course's own text and stored
//!   as a proposal with its AI label. The course's first cloud run shows the question (b)
//!   reminder once (D37, D49).
//! - The student can download chosen Canvas files (D46), e.g. a syllabus candidate that isn't
//!   downloaded yet.

use std::collections::BTreeSet;

use chrono::Utc;
use pagelamp_core::ai::{AiFeature, BlockReason, Destination, MaterialSharing, ModelErrorKind};
use pagelamp_core::ai_gate::{GateError, GatedContext, assemble, calendar_context};
use pagelamp_core::calendar::assemble::{Assembled, CurrentCalendar, outcome_of};
use pagelamp_core::calendar::candidates::{CalendarCandidate, ScoredCandidate, course_candidates};
use pagelamp_core::calendar::extraction::CalendarExtraction;
use pagelamp_core::calendar::proposal::{AcceptedCalendar, CalendarProposal};
use pagelamp_core::calendar::reading::reading_inputs;
use pagelamp_core::model::{
    AiLabel, AiMaterialsState, CalendarOrigin, Course, CourseGroup, CoursePhase,
};
use pagelamp_core::store::{
    CalendarChecks, CalendarProvenance, CalendarRow, CalendarState, GenerationRecord,
    GenerationStatus, NewCalendarRow, Staleness, Store,
};
use pagelamp_core::term::CalendarStatus;
use pagelamp_core::views::{self, AsOf};
use pagelamp_llm::OutputSpec;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::dates::CourseDatesInput;
use crate::ai::prompts::{COURSE_CALENDAR, PROMPT_VERSION};
use crate::ai::run::RunRequest;
use crate::ai::{
    GenEvent, GenNoticeCode, GenStage, backend_key, calendar_budget, feature_choice, request_shape,
};
use crate::{App, AppError, AppErrorKind, Result, SourceSyncResult, SyncEvent};

/// `SyllabusOffer::reason_code` for a course without a calendar in force.
pub const OFFER_NO_CALENDAR: &str = "no_calendar";

/// Everything the Timeline tab shows about a course's calendar.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CourseCalendarView {
    pub course_id: String,
    /// The calendar in force, if the student accepted or typed one.
    pub accepted: Option<AcceptedCalendar>,
    /// Pending proposals, at most one per origin.
    pub proposals: Vec<CalendarProposal>,
    pub status: CalendarStatus,
    /// The materials a reading would use, and why (read first, then by score).
    pub candidates: Vec<CalendarCandidate>,
    /// Why AI reading can't run for this course now.
    pub blocked: Option<BlockReason>,
}

/// A course "Read syllabi for N courses" would read (the facade decides which).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SyllabusOffer {
    pub course_id: String,
    /// Why it is offered: `no_calendar`.
    pub reason_code: String,
    /// How many candidate materials it has.
    pub candidates: u32,
    /// Some candidate has text to read.
    pub has_text: bool,
}

/// Options of an AI reading run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReadCalendarOptions {
    /// The student chose to go over the monthly budget for this run.
    pub override_budget: bool,
}

/// How one course of "Read syllabi for N courses" ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CalendarRunOutcome {
    pub course_id: String,
    /// The proposal the run made; `None` when it was blocked, failed, stopped or found no dates.
    pub proposal_id: Option<i64>,
    /// The proposal has no conflicts and isn't low quality (`accept_passing_proposals`).
    pub passing: bool,
    /// The gate stopped this course (each course is gated on its own).
    pub blocked: Option<BlockReason>,
    /// The run failed, or was stopped (`cancelled`).
    pub error: Option<AppErrorKind>,
}

/// Progress of `read_course_calendars`: one course after another, each with its `GenEvent`s.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CalendarBatchEvent {
    CourseStarted {
        course_id: String,
        /// 0-based.
        index: u32,
        total: u32,
    },
    Gen {
        course_id: String,
        event: GenEvent,
    },
    CourseFinished {
        outcome: CalendarRunOutcome,
    },
}

impl App {
    /// The calendar in force, pending proposals, the candidates and why AI reading can't run.
    /// Hidden courses are addressable.
    pub fn course_calendar(&self, course: &str) -> Result<CourseCalendarView> {
        let store = self.read_store()?;
        let course = store.resolve_course_with(course, true)?;
        self.calendar_view(&store, &course, AsOf::now_local())
    }

    /// The materials a syllabus reading would use, why, and whether each has text (§7.1).
    pub fn calendar_candidates(&self, course: &str) -> Result<Vec<CalendarCandidate>> {
        let store = self.read_store()?;
        let course = store.resolve_course_with(course, true)?;
        Ok(candidates_of(&store, &course, AsOf::now_local())?
            .into_iter()
            .map(|c| c.candidate)
            .collect())
    }

    /// The student's add (`include`) and remove (`exclude`) of candidate materials; a material
    /// in neither list keeps its earlier choice. Ids must be the course's materials.
    pub fn set_calendar_sources(
        &self,
        course: &str,
        include: Vec<String>,
        exclude: Vec<String>,
    ) -> Result<Vec<CalendarCandidate>> {
        let store = self.write_store()?;
        let course = store.resolve_course_with(course, true)?;
        let own: BTreeSet<String> = store
            .list_materials(&course.id)?
            .into_iter()
            .map(|m| m.id)
            .collect();
        if let Some(stray) = include.iter().chain(&exclude).find(|id| !own.contains(*id)) {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                format!("{stray} is not a material of {}.", course.display_name()),
            ));
        }
        let mut choices = store.calendar_signals(&course.id)?.choices;
        for id in include {
            choices.insert(id, true);
        }
        for id in exclude {
            choices.insert(id, false);
        }
        store.set_calendar_sources(&course.id, &choices)?;
        Ok(candidates_of(&store, &course, AsOf::now_local())?
            .into_iter()
            .map(|c| c.candidate)
            .collect())
    }

    /// Download the chosen files only (it counts as viewing them in Canvas; D46), e.g. the
    /// syllabus candidates marked downloadable. Holds the sync lock like any sync.
    pub async fn download_material_files(
        &self,
        course: &str,
        material_ids: Vec<String>,
        on_event: impl Fn(SyncEvent) + Send + Sync,
    ) -> Result<SourceSyncResult> {
        self.download_chosen_files(course, material_ids, on_event)
            .await
    }

    /// The deterministic syllabus scan (no model, §7.2): a proposal from the candidates' own
    /// words, or `None` when they state no first day of classes or the student dismissed a scan
    /// of the same materials.
    pub fn scan_course_calendar(&self, course: &str) -> Result<Option<CalendarProposal>> {
        let store = self.write_store()?;
        let course = store.resolve_course_with(course, true)?;
        scan_course(&store, &course, AsOf::now_local(), false)
    }

    /// S10: after a sync, scan the source's courses (hidden and withheld ones too: the scan
    /// sends nothing anywhere) and propose what changed. Problems never fail the sync.
    pub(crate) fn scan_after_sync(&self, source_id: &str) {
        let result = (|| -> Result<usize> {
            let store = self.write_store()?;
            let at = AsOf::now_local();
            let mut proposed = 0;
            for course in store.list_courses(true)? {
                if course.source_id == source_id
                    && scan_course(&store, &course, at, true)?.is_some()
                {
                    proposed += 1;
                }
            }
            Ok(proposed)
        })();
        match result {
            Ok(n) => {
                tracing::info!(target: "pagelamp::calendar", "scan after sync: {n} proposal(s)")
            }
            Err(err) => tracing::warn!(
                target: "pagelamp::calendar",
                "scan after sync failed: {:?}",
                err.kind
            ),
        }
    }

    /// The course dates form: `Some` puts the student's own calendar in force (the week
    /// numbering is computed here), `None` clears it and the student's term dates ("Undo").
    pub fn set_course_dates(
        &self,
        course: &str,
        dates: Option<CourseDatesInput>,
    ) -> Result<CourseCalendarView> {
        let store = self.write_store()?;
        let course = store.resolve_course_with(course, true)?;
        let calendar = dates
            .as_ref()
            .map(CourseDatesInput::to_calendar)
            .transpose()?;
        store.set_student_calendar(&course.id, calendar.as_ref(), Utc::now())?;
        super::set_dates_confirmed(&store, &course.id, calendar.is_some())?;
        self.calendar_view(&store, &course, AsOf::now_local())
    }

    /// Accept a proposal, optionally with the student's edits and conflict choices (as the
    /// dates form). The calendar in force becomes superseded.
    pub fn accept_calendar_proposal(
        &self,
        proposal_id: i64,
        edits: Option<CourseDatesInput>,
    ) -> Result<CourseCalendarView> {
        let store = self.write_store()?;
        let row = pending(&store, proposal_id)?;
        let edited = edits
            .as_ref()
            .map(CourseDatesInput::to_calendar)
            .transpose()?
            .map(|calendar| (calendar, row.dates.clone()));
        let accepted = store.accept_calendar_proposal(proposal_id, edited, Utc::now())?;
        super::set_dates_confirmed(&store, &accepted.course_id, true)?;
        let course = store.resolve_course_with(&accepted.course_id, true)?;
        self.calendar_view(&store, &course, AsOf::now_local())
    }

    /// Accept several proposals that have no conflicts and aren't low quality; `Invalid`
    /// (accepting none) if one of them isn't passing.
    pub fn accept_passing_proposals(
        &self,
        proposal_ids: Vec<i64>,
    ) -> Result<Vec<CourseCalendarView>> {
        {
            let store = self.read_store()?;
            for id in &proposal_ids {
                if !pending(&store, *id)?.checks.passing {
                    return Err(AppError::new(
                        AppErrorKind::Invalid,
                        "This proposal has conflicts to choose between first.",
                    ));
                }
            }
        }
        proposal_ids
            .into_iter()
            .map(|id| self.accept_calendar_proposal(id, None))
            .collect()
    }

    /// Dismiss a proposal: the same materials aren't proposed again for 30 days.
    pub fn dismiss_calendar_proposal(&self, proposal_id: i64) -> Result<()> {
        let store = self.write_store()?;
        pending(&store, proposal_id)?;
        Ok(store.dismiss_calendar_proposal(proposal_id, Utc::now())?)
    }

    /// The courses "Read syllabi for N courses" would read: current, upcoming or unknown
    /// courses, not hidden, without a calendar in force, whose materials AI may read and that
    /// have a candidate to read. Model setup doesn't matter here (the button then says "Set up
    /// AI to read syllabi"). Empty while "Not now" covers every one of them
    /// (`snooze_calendar_offers`); a course offered later brings all of them back. The Courses
    /// page and `startup_tasks().calendar_offers` show the same offers.
    pub fn syllabus_reading_offers(&self) -> Result<Vec<SyllabusOffer>> {
        self.unsnoozed_calendar_offers()
    }

    /// Every offer, snoozed or not (what "Not now" records).
    pub(crate) fn all_syllabus_reading_offers(&self) -> Result<Vec<SyllabusOffer>> {
        let store = self.read_store()?;
        let at = AsOf::now_local();
        let mut offers = Vec::new();
        for summary in views::list_courses(&store, false, at)? {
            let course = &summary.course;
            let without_calendar = matches!(
                summary.timeline.calendar,
                CalendarStatus::NoCalendar | CalendarStatus::Proposed
            );
            if summary.lifecycle.group == CourseGroup::Past
                || !without_calendar
                || !course.ai_materials().is_readable()
            {
                continue;
            }
            let candidates = candidates_of(&store, course, at)?;
            if !candidates.iter().any(|c| c.candidate.included) {
                continue;
            }
            offers.push(SyllabusOffer {
                course_id: course.id.clone(),
                reason_code: OFFER_NO_CALENDAR.to_string(),
                candidates: u32::try_from(candidates.len()).unwrap_or(u32::MAX),
                has_text: candidates.iter().any(|c| c.candidate.has_text),
            });
        }
        Ok(offers)
    }

    /// "Read the syllabus with AI" (§7.3): gated like every model run, then read, checked
    /// against the course's own text (§7.5) and stored as a proposal that changes nothing
    /// until the student accepts it.
    pub async fn read_course_calendar(
        &self,
        course: &str,
        generation_id: &str,
        options: ReadCalendarOptions,
        on_event: impl Fn(GenEvent) + Send + Sync,
    ) -> Result<CalendarProposal> {
        let result = self
            .read_one_calendar(course, generation_id, options, None, &on_event)
            .await;
        on_event(GenEvent::Finished { ok: result.is_ok() });
        result
    }

    /// "Read syllabi for N courses": one course after another, each gated on its own. Stopping
    /// the batch (`cancel_generation(batch_id)`) stops the course being read and the rest.
    pub async fn read_course_calendars(
        &self,
        courses: Vec<String>,
        batch_id: &str,
        options: ReadCalendarOptions,
        on_event: impl Fn(CalendarBatchEvent) + Send + Sync,
    ) -> Result<Vec<CalendarRunOutcome>> {
        let resolved = {
            let store = self.read_store()?;
            courses
                .iter()
                .map(|course| store.resolve_course_with(course, true))
                .collect::<std::result::Result<Vec<Course>, _>>()?
        };
        let (_batch, batch_cancel) = self.register_run(batch_id, None)?;
        let total = u32::try_from(resolved.len()).unwrap_or(u32::MAX);
        let mut outcomes = Vec::with_capacity(resolved.len());
        for (index, course) in resolved.iter().enumerate() {
            if batch_cancel.is_cancelled() {
                break;
            }
            let index = u32::try_from(index).unwrap_or(u32::MAX);
            on_event(CalendarBatchEvent::CourseStarted {
                course_id: course.id.clone(),
                index,
                total,
            });
            let generation_id = format!("{batch_id}-{index}");
            let forward = |event: GenEvent| {
                on_event(CalendarBatchEvent::Gen {
                    course_id: course.id.clone(),
                    event,
                })
            };
            let result = self
                .read_one_calendar(
                    &course.id,
                    &generation_id,
                    options,
                    Some(&batch_cancel),
                    &forward,
                )
                .await;
            forward(GenEvent::Finished { ok: result.is_ok() });
            let outcome = match result {
                Ok(proposal) => CalendarRunOutcome {
                    course_id: course.id.clone(),
                    proposal_id: Some(proposal.id),
                    passing: proposal.passing,
                    blocked: None,
                    error: None,
                },
                Err(err) => CalendarRunOutcome {
                    course_id: course.id.clone(),
                    proposal_id: None,
                    passing: false,
                    blocked: err.blocked,
                    error: (err.kind != AppErrorKind::Blocked).then_some(err.kind),
                },
            };
            on_event(CalendarBatchEvent::CourseFinished {
                outcome: outcome.clone(),
            });
            let stop = outcome.error == Some(AppErrorKind::Cancelled);
            outcomes.push(outcome);
            if stop {
                break;
            }
        }
        Ok(outcomes)
    }

    /// Stop a running generation or batch by its id; it ends with `cancelled`. Unknown or
    /// finished ids are fine (nothing to stop).
    pub fn cancel_generation(&self, generation_id: &str) -> Result<()> {
        self.cancel_run(generation_id);
        Ok(())
    }

    /// One syllabus reading (see `read_course_calendar`); `parent`: the batch it belongs to.
    async fn read_one_calendar(
        &self,
        course: &str,
        generation_id: &str,
        options: ReadCalendarOptions,
        parent: Option<&pagelamp_llm::CancellationToken>,
        on_event: &(dyn Fn(GenEvent) + Send + Sync),
    ) -> Result<CalendarProposal> {
        // Registered until the proposal is stored: an update waits for the whole reading.
        let (_run, cancel) = self.register_run(generation_id, parent)?;
        on_event(GenEvent::Stage {
            stage: GenStage::BuildingContext,
        });
        let at = AsOf::now_local();
        // Everything read before the model call; the store isn't held across it.
        let (course, choice, destination, context, prompt, inputs, by_handle) = {
            let store = self.read_store()?;
            let course = store.resolve_course_with(course, true)?;
            // From here a change to the course's AI settings stops this reading; the gate
            // reads them again after that, so no change slips between the two.
            self.run_reads_course(generation_id, &course.id, None);
            let course = store.resolve_course_with(&course.id, true)?;
            // The gate's order: the course's own state before the model setup.
            if let Some(reason) = course_block(&course) {
                return Err(blocked(reason));
            }
            let Some(choice) = feature_choice(&store, AiFeature::CourseCalendar)? else {
                return Err(blocked(BlockReason::NoModelChosen));
            };
            let (_, destination) = self.estimate_profile(&choice)?;
            // Before calendar_context reads question (b) (`stop_course_runs`, cloud only).
            self.run_reads_course(generation_id, &course.id, Some(destination));
            let signals = store.calendar_signals(&course.id)?;
            let context = match calendar_context(
                &store,
                &course.id,
                at,
                destination,
                calendar_budget(destination),
                &signals,
            ) {
                Ok(context) => context,
                Err(GateError::Blocked(reason)) => return Err(blocked(reason)),
                Err(GateError::Store(err)) => return Err(err.into()),
            };
            let (prompt, output, max_output) = request_shape(AiFeature::CourseCalendar, &context);
            if let Some(reason) = self.run_blocks(
                &store,
                &choice,
                &prompt,
                &output,
                max_output,
                options.override_budget,
            )? {
                return Err(blocked(reason));
            }
            let inputs = reading_inputs(&store, &course, at, &signals)?;
            let by_handle = inputs.sources_by_handle(&store, &context)?;
            (
                course,
                choice,
                destination,
                context,
                prompt,
                inputs,
                by_handle,
            )
        };
        let (_, output, max_output) = request_shape(AiFeature::CourseCalendar, &context);
        let started = Utc::now();
        let run = self
            .run_model(
                RunRequest {
                    generation_id,
                    cancel: &cancel,
                    feature: AiFeature::CourseCalendar,
                    choice: &choice,
                    prompt,
                    output,
                    max_output_tokens: max_output,
                },
                on_event,
            )
            .await;
        let generation = |status, output_json, error_kind: Option<&str>| GenerationRecord {
            id: generation_id.to_string(),
            feature: AiFeature::CourseCalendar,
            course_id: Some(course.id.clone()),
            week: None,
            backend: backend_key(&choice.backend),
            model: choice.model.clone(),
            status,
            created_at: started,
            prompt_version: PROMPT_VERSION,
            output_json,
            summary_json: serde_json::to_string(&GenerationSummary {
                context: context.summary(),
                manifest: context.manifest(),
            })
            .ok(),
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
                    .record_generation(&generation(status, None, kind))?;
                return Err(err);
            }
        };
        on_event(GenEvent::Stage {
            stage: GenStage::Validating,
        });
        let store = self.write_store()?;
        let extraction: Option<CalendarExtraction> = run
            .json
            .clone()
            .and_then(|json| serde_json::from_value(json).ok());
        let current = store.accepted_calendar(&course.id)?.map(|row| row.calendar);
        let assembled = extraction.as_ref().and_then(|extraction| {
            inputs
                .propose(extraction, &by_handle, current.as_ref())
                .ok()
        });
        let Some(assembled) = assembled else {
            store.record_generation(&generation(
                GenerationStatus::Failed,
                None,
                Some(ModelErrorKind::BadOutput.as_str()),
            ))?;
            return Err(AppError {
                model_error: Some(ModelErrorKind::BadOutput),
                ..AppError::new(
                    AppErrorKind::Model,
                    "The model's answer had no dates PageLamp could check against the \
                     course's materials. Try again, or set the dates yourself.",
                )
            });
        };
        store.record_generation(&generation(
            GenerationStatus::Accepted,
            serde_json::to_string(&extraction).ok(),
            None,
        ))?;
        let sharing_reminder = destination == Destination::Cloud
            && matches!(
                course.material_sharing,
                MaterialSharing::Unanswered | MaterialSharing::NotSure
            )
            && store.claim_sharing_reminder(&course.id, Utc::now())?;
        if sharing_reminder {
            on_event(GenEvent::Notice {
                code: GenNoticeCode::MaterialSharingReminder,
            });
        }
        let row = NewCalendarRow {
            course_id: course.id.clone(),
            origin: CalendarOrigin::Ai,
            dates: assembled.dates.clone(),
            checks: checks_of(&assembled, sharing_reminder),
            calendar: assembled.calendar,
            manifest: context.manifest().materials.clone(),
            fingerprint: inputs.fingerprint(),
            provenance: Some(CalendarProvenance {
                generation_id: Some(generation_id.to_string()),
                backend_label: run.backend_label,
                model: run.model,
                prompt_version: PROMPT_VERSION,
                on_device: run.on_device,
            }),
        };
        let id = store.insert_calendar_proposal(&row, Utc::now())?;
        store
            .calendar_row(id)?
            .map(proposal_of)
            .ok_or_else(|| no_proposal(id))
    }

    /// The view of one course's calendars at `at`.
    fn calendar_view(
        &self,
        store: &Store,
        course: &Course,
        at: AsOf,
    ) -> Result<CourseCalendarView> {
        let timeline = views::course_timeline(store, course, at)?;
        let accepted_row = store.accepted_calendar(&course.id)?;
        // What accepting each proposal would change, against the calendar in force now (another
        // proposal may have been accepted since it was made).
        let current = CurrentCalendar {
            calendar: accepted_row.as_ref().map(|row| &row.calendar),
            week: timeline.current_week,
            phase: timeline.phase,
        };
        let proposals = store
            .calendar_proposals(&course.id)?
            .into_iter()
            .map(|row| {
                let outcome = outcome_of(&row.calendar, &current, at.today, false);
                CalendarProposal {
                    resulting_week_today: outcome.resulting_week_today,
                    resulting_phase: outcome.resulting_phase,
                    changes: outcome.changes,
                    ..proposal_of(row)
                }
            })
            .collect();
        let status = timeline.calendar;
        let accepted = match accepted_row {
            Some(row) => {
                let staleness = store.calendar_staleness(&row)?;
                Some(accepted_of(row, staleness))
            }
            None => None,
        };
        let candidates = candidates_of(store, course, at)?;
        let blocked = self.reading_block(store, course, &candidates)?;
        Ok(CourseCalendarView {
            course_id: course.id.clone(),
            accepted,
            proposals,
            status,
            candidates: candidates.into_iter().map(|c| c.candidate).collect(),
            blocked,
        })
    }

    /// Why AI reading can't run for `course` now, in the gate's order: the course's own state,
    /// no model, question (b), the disclosure or the weekly cap, and no readable candidate.
    /// (A budget only shows in the estimate: it depends on the run's size.)
    fn reading_block(
        &self,
        store: &Store,
        course: &Course,
        candidates: &[ScoredCandidate],
    ) -> Result<Option<BlockReason>> {
        if let Some(reason) = course_block(course) {
            return Ok(Some(reason));
        }
        let Some(choice) = feature_choice(store, AiFeature::CourseCalendar)? else {
            return Ok(Some(BlockReason::NoModelChosen));
        };
        let (_, destination) = self.estimate_profile(&choice)?;
        if !course.material_sharing.allows(destination) {
            return Ok(Some(BlockReason::MaterialSharingNotAllowed));
        }
        if let Some(reason) = self.run_blocks(
            store,
            &choice,
            &assemble(COURSE_CALENDAR, &GatedContext::empty(), None),
            &OutputSpec::Text,
            0,
            true,
        )? {
            return Ok(Some(reason));
        }
        if !candidates.iter().any(|c| c.candidate.included) {
            return Ok(Some(BlockReason::NoReadableMaterials));
        }
        Ok(None)
    }
}

/// Why a course's own state stops AI reading: hidden, or its materials not readable.
fn course_block(course: &Course) -> Option<BlockReason> {
    if course.hidden {
        return Some(BlockReason::CourseHidden);
    }
    match course.ai_materials() {
        AiMaterialsState::WithheldByPolicy => Some(BlockReason::CoursePolicyProhibited),
        AiMaterialsState::TurnedOff => Some(BlockReason::CourseAiTurnedOff),
        AiMaterialsState::Readable => None,
    }
}

/// The deterministic scan of one course (§7.2). Nothing new → `None`: the student dismissed a
/// scan of these materials, or the calendar in force came from them or says the same. The
/// student's own "Scan" returns a waiting scan proposal of the same materials again;
/// `automatic` (after a sync) doesn't.
fn scan_course(
    store: &Store,
    course: &Course,
    at: AsOf,
    automatic: bool,
) -> Result<Option<CalendarProposal>> {
    let signals = store.calendar_signals(&course.id)?;
    let inputs = reading_inputs(store, course, at, &signals)?;
    if inputs.sources.is_empty() {
        return Ok(None);
    }
    let fingerprint = inputs.fingerprint();
    if store.calendar_fingerprint_dismissed(&course.id, CalendarOrigin::Scan, &fingerprint)? {
        return Ok(None);
    }
    let waiting = store
        .calendar_proposals(&course.id)?
        .into_iter()
        .find(|row| row.origin == CalendarOrigin::Scan && row.fingerprint == fingerprint);
    if let Some(row) = waiting {
        return Ok((!automatic).then(|| proposal_of(row)));
    }
    let current = store.accepted_calendar(&course.id)?;
    if current
        .as_ref()
        .is_some_and(|row| row.fingerprint == fingerprint)
    {
        return Ok(None);
    }
    let current = current.map(|row| row.calendar);
    let Some(assembled) = inputs.scan(current.as_ref()) else {
        return Ok(None);
    };
    if current.as_ref() == Some(&assembled.calendar) {
        return Ok(None);
    }
    let row = NewCalendarRow {
        course_id: course.id.clone(),
        origin: CalendarOrigin::Scan,
        dates: assembled.dates.clone(),
        checks: checks_of(&assembled, false),
        calendar: assembled.calendar,
        manifest: inputs.manifest.clone(),
        fingerprint,
        provenance: None,
    };
    let id = store.insert_calendar_proposal(&row, Utc::now())?;
    Ok(store.calendar_row(id)?.map(proposal_of))
}

/// What a generation row keeps about what was sent (no text).
#[derive(Serialize)]
struct GenerationSummary<'a> {
    context: &'a pagelamp_core::ai_gate::ContextSummary,
    manifest: &'a pagelamp_core::ai_gate::ContextManifest,
}

/// The course's candidates with the signals stored for it.
fn candidates_of(store: &Store, course: &Course, at: AsOf) -> Result<Vec<ScoredCandidate>> {
    let signals = store.calendar_signals(&course.id)?;
    Ok(course_candidates(store, course, at, &signals)?)
}

/// A pending proposal row, or `NotFound`.
fn pending(store: &Store, id: i64) -> Result<CalendarRow> {
    store
        .calendar_row(id)?
        .filter(|row| row.state == CalendarState::Proposed)
        .ok_or_else(|| no_proposal(id))
}

/// The checks a proposal is stored with.
fn checks_of(assembled: &Assembled, sharing_reminder: bool) -> CalendarChecks {
    CalendarChecks {
        conflicts: assembled.conflicts.clone(),
        dropped: assembled.dropped.clone(),
        low_quality: assembled.low_quality,
        passing: assembled.passing,
        disagrees_with_notes: assembled.disagrees_with_notes,
        sharing_reminder,
        resulting_week_today: assembled.resulting_week_today,
        resulting_phase: Some(assembled.resulting_phase),
        changes: assembled.changes.clone(),
    }
}

fn ai_label_of(row: &CalendarRow) -> Option<AiLabel> {
    row.provenance.as_ref().map(|p| AiLabel {
        backend_label: p.backend_label.clone(),
        model: p.model.clone(),
        created_at: row.created_at,
        on_device: p.on_device,
    })
}

/// A stored proposal as the facade shows it.
fn proposal_of(row: CalendarRow) -> CalendarProposal {
    let ai_label = ai_label_of(&row);
    CalendarProposal {
        id: row.id,
        course_id: row.course_id,
        origin: row.origin,
        calendar: row.calendar,
        dates: row.dates,
        conflicts: row.checks.conflicts,
        dropped: row.checks.dropped,
        low_quality: row.checks.low_quality,
        passing: row.checks.passing,
        ai_label,
        sharing_reminder: row.checks.sharing_reminder,
        resulting_week_today: row.checks.resulting_week_today,
        resulting_phase: row.checks.resulting_phase.unwrap_or(CoursePhase::Unknown),
        changes: row.checks.changes,
        created_at: row.created_at,
    }
}

/// The calendar in force as the facade shows it.
fn accepted_of(row: CalendarRow, staleness: Staleness) -> AcceptedCalendar {
    let ai_label = ai_label_of(&row);
    AcceptedCalendar {
        id: row.id,
        origin: row.origin,
        calendar: row.calendar,
        dates: row.dates,
        ai_label,
        accepted_at: row.decided_at.unwrap_or(row.created_at),
        stale: staleness.stale,
        // materials.updated_at moves with every sync, so no date says when the text changed.
        stale_since: None,
        changed_materials: staleness.changed_materials,
    }
}

fn blocked(reason: BlockReason) -> AppError {
    AppError::blocked(reason, block_message(reason))
}

fn block_message(reason: BlockReason) -> &'static str {
    match reason {
        BlockReason::CourseHidden => "This course is hidden: show it to read its syllabus.",
        BlockReason::CoursePolicyProhibited => {
            "This course's AI policy is \"prohibited\", so its materials aren't sent to a model."
        }
        BlockReason::CourseAiTurnedOff => "AI access is turned off for this course.",
        BlockReason::NoModelChosen => "Choose a model for reading syllabi first.",
        BlockReason::MaterialSharingNotAllowed => {
            "You said this course's materials may not be shared with an AI service."
        }
        BlockReason::NoReadableMaterials => "This course has no syllabus or outline to read.",
        BlockReason::DisclosureNotAcknowledged => "Review what is sent and to whom first.",
        BlockReason::WeeklyRunCapReached => "This week's ChatGPT-plan runs are used up.",
        BlockReason::BudgetReached => "This would go past your monthly AI budget.",
        BlockReason::PriceUnknownNotAcknowledged => {
            "This model has no known price: confirm that the budget can't be enforced for it."
        }
        _ => "Reading the syllabus with AI isn't available right now.",
    }
}

fn no_proposal(id: i64) -> AppError {
    AppError::new(
        AppErrorKind::NotFound,
        format!("There is no calendar proposal {id}."),
    )
}
