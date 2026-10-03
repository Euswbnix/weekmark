//! A calendar the student's own AI app proposes over MCP (calendar design §7.9, D48; beta.1,
//! cuttable): `pagelamp mcp` has no model and no network, so the AI app reads the syllabus
//! with the read tools and hands back a `CalendarExtraction`; this checks it with the same
//! validator as every reader and stores a proposal with origin `ai_app`, which only PageLamp
//! can accept.
//!
//! - Refused for a course whose materials aren't readable: otherwise "quote found / not found"
//!   would be an oracle for withheld text. Hidden and removed courses aren't found at all.
//! - The reply carries counts and reason codes only, never quotes.
//! - At most `MAX_PER_DAY` calls per course and day (counted in `settings`, with the proposal
//!   in one transaction).

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::extraction::CalendarExtraction;
use super::reading::reading_inputs;
use super::text::rebuild_parts;
use super::validate::{DropCount, SourceMaterial};
use crate::ai::BlockReason;
use crate::ai_gate::ManifestEntry;
use crate::model::{AiMaterialsState, Course, Timestamp};
use crate::store::{CalendarChecks, NewCalendarRow, Store};
use crate::term::CalendarOrigin;
use crate::views::AsOf;

/// Proposals an AI app may send per course and day [estimate, design §7.9].
pub const MAX_PER_DAY: u32 = 3;
/// `settings` key of the day's counts.
const COUNTS_KEY: &str = "calendar.app_proposals";

/// What `propose_course_calendar` tells the AI app: counts and codes, no text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AppProposalReply {
    /// The proposal waiting in PageLamp for the student to accept.
    pub proposal_id: i64,
    /// Dates that were found in the materials' own words.
    pub dates_kept: u32,
    /// Claims and rows left out, by reason (e.g. `unsupported_quote`).
    pub dropped: Vec<DropCount>,
    /// Places where claims disagree: the student chooses.
    pub conflicts: u32,
    /// No conflicts and not low quality.
    pub passing: bool,
    /// Proposals this course may still get today.
    pub left_today: u32,
}

/// Why a proposal isn't stored.
#[derive(Debug, thiserror::Error)]
pub enum AppProposalError {
    /// The course's materials aren't shared with AI (`CoursePolicyProhibited`,
    /// `CourseAiTurnedOff`).
    #[error("blocked: {}", .0.as_str())]
    Blocked(BlockReason),
    #[error("this course already got {MAX_PER_DAY} proposals today")]
    LimitReached,
    /// No claim could be checked against the course's materials (V10).
    #[error("no date could be checked against the course's materials")]
    BadOutput,
    #[error(transparent)]
    Store(#[from] crate::Error),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DayCounts {
    date: Option<NaiveDate>,
    counts: BTreeMap<String, u32>,
}

/// Check `extraction` against `course`'s materials and store the proposal (see the module
/// docs). The materials are named by their ids (the read tools list them).
pub fn propose_from_ai_app(
    store: &Store,
    course: &Course,
    extraction: &CalendarExtraction,
    at: AsOf,
    now: Timestamp,
) -> Result<AppProposalReply, AppProposalError> {
    match course.ai_materials() {
        AiMaterialsState::Readable => {}
        AiMaterialsState::TurnedOff => {
            return Err(AppProposalError::Blocked(BlockReason::CourseAiTurnedOff));
        }
        AiMaterialsState::WithheldByPolicy => {
            return Err(AppProposalError::Blocked(
                BlockReason::CoursePolicyProhibited,
            ));
        }
    }
    store.in_transaction(|store| {
        // Every call counts, also one that ends as bad output. A failed read fails the call:
        // taking it for "no calls yet" would lift the daily limit.
        let mut day: DayCounts = store.setting_or_absent(COUNTS_KEY)?.unwrap_or_default();
        if day.date != Some(at.today) {
            day = DayCounts {
                date: Some(at.today),
                counts: BTreeMap::new(),
            };
        }
        let used = day.counts.entry(course.id.clone()).or_default();
        if *used >= MAX_PER_DAY {
            return Ok(Err(AppProposalError::LimitReached));
        }
        *used += 1;
        let left_today = MAX_PER_DAY - *used;
        store.set_setting(COUNTS_KEY, &day)?;

        let signals = store.calendar_signals(&course.id)?;
        let inputs = reading_inputs(store, course, at, &signals)?;
        let sources = course_sources(store, &course.id)?;
        let current = store.accepted_calendar(&course.id)?.map(|row| row.calendar);
        let Ok(assembled) = inputs.propose(extraction, &sources, current.as_ref()) else {
            return Ok(Err(AppProposalError::BadOutput));
        };
        // The materials its dates quote: a change to one makes the proposal stale.
        let mut quoted: Vec<&str> = assembled
            .dates
            .iter()
            .flat_map(|date| date.evidence.iter().map(|e| e.material_id.as_str()))
            .collect();
        quoted.sort_unstable();
        quoted.dedup();
        let manifest = quoted
            .iter()
            .map(|id| -> crate::Result<ManifestEntry> {
                Ok(ManifestEntry {
                    material_id: (*id).to_string(),
                    content_hash: store.get_material(id)?.and_then(|m| m.content_hash),
                    chunk_ords: store
                        .get_chunks(id, 0, None)?
                        .iter()
                        .map(|c| c.ord)
                        .collect(),
                })
            })
            .collect::<crate::Result<Vec<_>>>()?;
        let reply = |proposal_id| AppProposalReply {
            proposal_id,
            dates_kept: u32::try_from(assembled.dates.len()).unwrap_or(u32::MAX),
            dropped: assembled.dropped.clone(),
            conflicts: u32::try_from(assembled.conflicts.len()).unwrap_or(u32::MAX),
            passing: assembled.passing,
            left_today,
        };
        let id = store.insert_calendar_proposal(
            &NewCalendarRow {
                course_id: course.id.clone(),
                origin: CalendarOrigin::AiApp,
                calendar: assembled.calendar.clone(),
                dates: assembled.dates.clone(),
                checks: CalendarChecks {
                    conflicts: assembled.conflicts.clone(),
                    dropped: assembled.dropped.clone(),
                    low_quality: assembled.low_quality,
                    passing: assembled.passing,
                    disagrees_with_notes: assembled.disagrees_with_notes,
                    sharing_reminder: false,
                    resulting_week_today: assembled.resulting_week_today,
                    resulting_phase: Some(assembled.resulting_phase),
                    changes: assembled.changes.clone(),
                },
                manifest,
                fingerprint: inputs.fingerprint(),
                provenance: None,
            },
            now,
        )?;
        Ok(Ok(reply(id)))
    })?
}

/// Every material of the course with text, keyed by its id (what the AI app names).
fn course_sources(
    store: &Store,
    course_id: &str,
) -> crate::Result<HashMap<String, SourceMaterial>> {
    let mut sources = HashMap::new();
    for material in store.list_materials(course_id)? {
        let chunks = store.get_chunks(&material.id, 0, None)?;
        if chunks.is_empty() {
            continue;
        }
        sources.insert(
            material.id.clone(),
            SourceMaterial {
                material_id: material.id.clone(),
                title: material.title.clone(),
                url: material.url.clone(),
                published_at: material.published_at,
                parts: rebuild_parts(&chunks),
            },
        );
    }
    Ok(sources)
}
