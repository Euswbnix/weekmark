//! From a course's materials to a proposal that isn't stored yet (docs/design/
//! v0.3-course-calendar.md §7.2–§7.6). Every reader shares this path: the deterministic scan,
//! an AI reading and the student's AI app each hand back a `CalendarExtraction`, which is
//! checked against the course's own text (`validate`) and assembled (`assemble`).
//!
//! `reading_inputs` reads the store once: the candidates (§7.1) and their text, the year
//! hints the validator needs (V5, V6) and PageLamp's own evidence to cross-check with (V8).
//! Everything after that is pure.

use std::collections::HashMap;

use chrono::NaiveDate;

use super::CourseCalendar;
use super::assemble::{AssembleInput, Assembled, BadOutput, CrossChecks, assemble};
use super::candidates::{CandidateSignals, ScoredCandidate, candidates_in};
use super::extraction::CalendarExtraction;
use super::scan::scan;
use super::text::rebuild_parts;
use super::validate::{SourceMaterial, ValidationContext, validate};
use crate::ai_gate::ManifestEntry;
use crate::dates::course_date;
use crate::ingest::sha256_hex;
use crate::model::{Course, EventKind};
use crate::store::Store;
use crate::term::{BreakKind, CoursePhase};
use crate::views::{AsOf, CourseData};

/// What every reader of one course's calendar shares.
#[derive(Clone, Debug)]
pub struct ReadingInputs {
    pub course_id: String,
    pub candidates: Vec<ScoredCandidate>,
    /// The text of the candidates that are read, keyed by material id.
    pub sources: HashMap<String, SourceMaterial>,
    /// The candidates that are read: their content hash and every chunk.
    pub manifest: Vec<ManifestEntry>,
    pub validation: ValidationContext,
    pub checks: CrossChecks,
    /// Today's week and phase as the course shows them now (for "what accepting changes").
    pub current_week: Option<u32>,
    pub current_phase: CoursePhase,
    pub today: NaiveDate,
    pub full_year: bool,
}

/// Read what a calendar reading of `course` needs.
pub fn reading_inputs(
    store: &Store,
    course: &Course,
    at: AsOf,
    signals: &CandidateSignals,
) -> crate::Result<ReadingInputs> {
    let data = CourseData::load(store, course)?;
    // The week the views show today (none for a course that is over or hasn't started).
    let (resolved, timeline, _) = data.state(course, at);
    let candidates = candidates_in(store, &data, &resolved, signals)?;
    let mut sources = HashMap::new();
    let mut manifest = Vec::new();
    for scored in candidates.iter().filter(|c| c.candidate.included) {
        let id = &scored.candidate.material_id;
        let Some(material) = data.materials.iter().find(|m| &m.id == id) else {
            continue;
        };
        let chunks = store.get_chunks(id, 0, None)?;
        manifest.push(ManifestEntry {
            material_id: id.clone(),
            content_hash: material.content_hash.clone(),
            chunk_ords: chunks.iter().map(|c| c.ord).collect(),
        });
        sources.insert(
            id.clone(),
            SourceMaterial {
                material_id: id.clone(),
                title: material.title.clone(),
                url: material.url.clone(),
                published_at: material.published_at,
                parts: rebuild_parts(&chunks),
            },
        );
    }
    let resolution = &resolved.resolution;
    let lms = &data.term_data.lms;
    let first_class_event = data
        .events
        .iter()
        .filter(|event| event.kind == EventKind::ClassEvent)
        .filter_map(|event| event.starts_at.or(event.due_at))
        .map(|instant| course_date(instant, resolved.tz))
        .min();
    Ok(ReadingInputs {
        course_id: course.id.clone(),
        candidates,
        sources,
        manifest,
        validation: ValidationContext {
            today: Some(at.today),
            outer_frame: resolution.outer_frame,
            session_start: resolved.session.as_ref().map(|s| s.window.start),
            week_one_monday: resolution.week_one_monday,
            lms_term_start: lms.term_start.or(data.term_data.synced_term_start),
        },
        checks: CrossChecks {
            fit: resolved.fitted.map(|fit| (fit.monday, fit.weeks)),
            lms_course_start: resolved.plausible_lms_start,
            first_class_event,
            observations: resolved
                .observations
                .iter()
                .map(|o| (o.day, o.week))
                .collect(),
            school_reading_weeks: resolved
                .institution
                .iter()
                .flat_map(|term| &term.breaks)
                .filter(|b| b.kind == BreakKind::ReadingWeek)
                .map(|b| b.span)
                .collect(),
        },
        current_week: timeline.current_week,
        current_phase: timeline.phase,
        today: at.today,
        full_year: resolved.full_year,
    })
}

impl ReadingInputs {
    /// The candidate set's fingerprint: the same materials with the same text give the same
    /// one, so a dismissed proposal isn't made again from them.
    pub fn fingerprint(&self) -> String {
        let mut lines: Vec<String> = self
            .manifest
            .iter()
            .map(|entry| {
                format!(
                    "{}\t{}",
                    entry.material_id,
                    entry.content_hash.as_deref().unwrap_or("")
                )
            })
            .collect();
        lines.sort();
        sha256_hex(lines.join("\n").as_bytes())
    }

    /// Check `extraction` against `sources` (keyed by the handles or material ids it names)
    /// and assemble a proposal (V2–V11, §7.6); `BadOutput` when the run as a whole isn't
    /// usable (V10). `current` is the calendar in force, if any.
    pub fn propose(
        &self,
        extraction: &CalendarExtraction,
        sources: &HashMap<String, SourceMaterial>,
        current: Option<&CourseCalendar>,
    ) -> Result<Assembled, BadOutput> {
        let validated = validate(extraction, sources, &self.validation);
        assemble(&AssembleInput {
            validated: &validated,
            checks: self.checks.clone(),
            current,
            current_week: self.current_week,
            current_phase: self.current_phase,
            today: self.today,
            full_year: self.full_year,
        })
    }

    /// The deterministic scan of the candidates that are read (§7.2); `None` when it finds
    /// nothing usable (no first class).
    pub fn scan(&self, current: Option<&CourseCalendar>) -> Option<Assembled> {
        let extraction = scan(&self.sources);
        self.propose(&extraction, &self.sources, current).ok()
    }
}
