//! The context builders (design §4.2): the only code that reads course data for a prompt.
//!
//! - Course state is read in the same read transaction as the text (`in_read_transaction`),
//!   never cached: a policy change between two runs is always honoured.
//! - Text comes only from `chunks` of courses that are `readable` at that moment;
//!   `withheld_by_policy` and `turned_off` courses give structure only, hidden courses nothing.
//! - Question (b): a course answered `not_allowed` gives no text to a cloud model (a model on
//!   this computer still gets it). Structure is not material text and is unaffected.
//! - Structure = titles, kinds, dates, week numbers, deadlines (rule 8). Titles are course
//!   content too, so structure is wrapped as data like material text.
//! - Left out of an explanation by default (listed in the summary): materials that look like
//!   assessments (rule 4), external links, materials without text, and whatever doesn't fit
//!   the budget (each material gets a fair share).

use std::fmt::Write as _;

use chrono::{Duration, NaiveDate};

use super::{
    Block, CitationTarget, ContextCourse, GatedContext, LeftOutMaterial, LeftOutReason,
    ManifestEntry,
};
use crate::ai::{BlockReason, Destination};
use crate::calendar::candidates::{
    CandidateLeftOut, CandidateSignals, candidates_in, extra_chunks, select_chunks,
};
use crate::dates::course_date;
use crate::model::{AiMaterialsState, Chunk, Course, EventKind, MaterialKind, Module, TextStatus};
use crate::store::Store;
use crate::term::ResolvedTerm;
use crate::views::{self, AsOf, CourseData, MaterialView};

/// Words that mark a title as an assessment (whole words, any case) [tunable; open item].
const ASSESSMENT_WORDS: [&str; 10] = [
    "assignment",
    "homework",
    "hw",
    "problem set",
    "pset",
    "quiz",
    "exam",
    "midterm",
    "test",
    "lab report",
];

/// Words that make an assessment-looking title study material after all ("Midterm review",
/// "Practice quiz solutions", "Exam preparation notes").
const STUDY_WORDS: [&str; 8] = [
    "review",
    "practice",
    "solution",
    "solutions",
    "notes",
    "lecture",
    "slides",
    "preparation",
];

/// How much course text a context may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextBudget {
    /// Characters of material text, shared fairly among the materials.
    pub max_chars: usize,
}

/// Why no context was built.
#[derive(Debug, thiserror::Error)]
pub enum GateError {
    /// The course's state (or the request) doesn't allow it; nothing to send.
    #[error("blocked: {}", .0.as_str())]
    Blocked(BlockReason),
    #[error(transparent)]
    Store(#[from] crate::Error),
}

/// Which courses a study plan covers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanScope {
    /// Course ids or codes; empty: every visible course.
    pub courses: Vec<String>,
    /// Days ahead the plan covers (deadlines in this window are listed).
    pub horizon_days: u32,
}

/// Structure only, for a study plan: the courses in scope with their week, this and next
/// week's materials (titles and ids) and the deadlines in the horizon. Hidden courses are left
/// out.
pub fn plan_context(store: &Store, scope: &PlanScope, at: AsOf) -> Result<GatedContext, GateError> {
    store
        .in_read_transaction(|store| {
            let courses = scoped_courses(store, &scope.courses)?;
            let mut context = GatedContext::empty();
            for course in &courses {
                let text = course_structure(store, course, at, scope.horizon_days, &mut context)?;
                context.blocks.push(Block::Structure(text));
            }
            Ok(context)
        })
        .map_err(GateError::from)
}

/// Structure and study-plan progress, for the weekly note: every visible course, deadlines in
/// the next 7 days, and last week's and today's plan items.
pub fn note_context(store: &Store, at: AsOf) -> Result<GatedContext, GateError> {
    store
        .in_read_transaction(|store| {
            let courses = scoped_courses(store, &[])?;
            let mut context = GatedContext::empty();
            for course in &courses {
                let text = course_structure(store, course, at, 7, &mut context)?;
                context.blocks.push(Block::Structure(text));
            }
            if let Some(plan) = store.latest_study_plan()? {
                let week_ago = at.today - Duration::days(7);
                let last_week: Vec<_> = plan
                    .plan
                    .items
                    .iter()
                    .filter(|item| item.date >= week_ago && item.date < at.today)
                    .collect();
                let done = last_week.iter().filter(|item| item.done).count();
                let mut text = format!(
                    "Study plan progress: last 7 days {done} of {} items done.\nToday:",
                    last_week.len()
                );
                let today: Vec<_> = plan
                    .plan
                    .items
                    .iter()
                    .filter(|i| i.date == at.today)
                    .collect();
                if today.is_empty() {
                    text.push_str(" nothing planned.");
                }
                for item in today {
                    let _ = write!(
                        text,
                        "\n- {}{}",
                        item.title,
                        if item.done { " (done)" } else { "" }
                    );
                }
                context.blocks.push(Block::Structure(text));
            }
            Ok(context)
        })
        .map_err(GateError::from)
}

/// Material text of one course week, for an explanation: blocked unless the course is visible,
/// `readable`, its question-(b) answer allows `destination`, and it has readable materials that
/// week (`week`: default the current week).
pub fn week_context(
    store: &Store,
    course: &str,
    week: Option<u32>,
    at: AsOf,
    destination: Destination,
    budget: ContextBudget,
) -> Result<GatedContext, GateError> {
    let result = store.in_read_transaction(|store| {
        let course = store.resolve_course_with(course, true)?;
        if course.hidden {
            return Ok(Err(BlockReason::CourseHidden));
        }
        match course.ai_materials() {
            AiMaterialsState::Readable => {}
            AiMaterialsState::TurnedOff => return Ok(Err(BlockReason::CourseAiTurnedOff)),
            AiMaterialsState::WithheldByPolicy => {
                return Ok(Err(BlockReason::CoursePolicyProhibited));
            }
        }
        if !course.material_sharing.allows(destination) {
            return Ok(Err(BlockReason::MaterialSharingNotAllowed));
        }
        let listed = views::week_materials(store, &course.id, week, true, at)?;
        let mut context = GatedContext::empty();
        let mut candidates = Vec::new();
        for view in &listed.materials {
            if let Some(reason) = left_out(store, view)? {
                context.summary.left_out.push(LeftOutMaterial {
                    material_id: view.id.clone(),
                    title: view.title.clone(),
                    reason,
                });
                continue;
            }
            let chunks = store.get_chunks(&view.id, 0, None)?;
            let hash = store.get_material(&view.id)?.and_then(|m| m.content_hash);
            candidates.push((view, hash, chunks));
        }
        let sizes: Vec<usize> = candidates
            .iter()
            .map(|(_, _, chunks)| chunks.iter().map(|c| c.text.len()).sum())
            .collect();
        let shares = fair_shares(&sizes, budget.max_chars);

        let mut header = format!("Course: {}\n", course.display_name());
        match listed.week {
            Some(n) => {
                let _ = write!(header, "Week: {n}");
            }
            None => header.push_str("Week: unknown (recent materials)"),
        }
        context.blocks.push(Block::Structure(header));
        for ((view, hash, chunks), share) in candidates.iter().zip(shares) {
            let mut used = 0;
            let mut ords = Vec::new();
            for chunk in chunks {
                if used + chunk.text.len() > share {
                    break;
                }
                used += chunk.text.len();
                ords.push(chunk.ord);
                let handle = format!("c{}", context.handles() + 1);
                context.blocks.push(Block::Material {
                    handle,
                    target: CitationTarget {
                        material_id: view.id.clone(),
                        title: view.title.clone(),
                        locator: chunk.locator.clone(),
                        url: view.url.clone(),
                    },
                    text: chunk.text.clone(),
                });
            }
            if ords.is_empty() {
                context.summary.left_out.push(LeftOutMaterial {
                    material_id: view.id.clone(),
                    title: view.title.clone(),
                    reason: LeftOutReason::OverBudget,
                });
                continue;
            }
            context.summary.materials_included += 1;
            if ords.len() < chunks.len() {
                context.summary.materials_trimmed += 1;
            }
            context.manifest.materials.push(ManifestEntry {
                material_id: view.id.clone(),
                content_hash: hash.clone(),
                chunk_ords: ords,
            });
        }
        if context.summary.materials_included == 0 {
            return Ok(Err(BlockReason::NoReadableMaterials));
        }
        context.summary.courses.push(ContextCourse {
            course_id: course.id.clone(),
            state: AiMaterialsState::Readable,
            text_included: true,
        });
        Ok(Ok(context))
    })?;
    result.map_err(GateError::Blocked)
}

impl GatedContext {
    /// How many citation handles the context has so far.
    fn handles(&self) -> usize {
        self.blocks
            .iter()
            .filter(|block| matches!(block, Block::Material { .. }))
            .count()
    }
}

/// The visible courses among `wanted` (ids or codes; empty: all visible), in list order.
fn scoped_courses(store: &Store, wanted: &[String]) -> crate::Result<Vec<Course>> {
    if wanted.is_empty() {
        return store.list_courses(false);
    }
    let mut courses = Vec::new();
    for query in wanted {
        let course = store.resolve_course_with(query, true)?;
        if !course.hidden && !courses.iter().any(|c: &Course| c.id == course.id) {
            courses.push(course);
        }
    }
    Ok(courses)
}

/// The structure block of one course: its state, week, this and next week's materials and the
/// deadlines in the next `days` days. Adds the course to the summary and its listed materials to
/// the manifest (ids only).
fn course_structure(
    store: &Store,
    course: &Course,
    at: AsOf,
    days: u32,
    context: &mut GatedContext,
) -> crate::Result<String> {
    let state = course.ai_materials();
    context.summary.courses.push(ContextCourse {
        course_id: course.id.clone(),
        state,
        text_included: false,
    });
    let mut text = format!(
        "Course: {} [course_id: {}]\nMaterial text shared with AI: {}\n",
        course.display_name(),
        course.id,
        match state {
            AiMaterialsState::Readable => "yes",
            AiMaterialsState::TurnedOff => "no (turned off by the student)",
            AiMaterialsState::WithheldByPolicy => "no (the course does not allow AI use)",
        }
    );
    let this_week = views::week_materials(store, &course.id, None, true, at)?;
    match this_week.week {
        Some(n) => {
            let _ = writeln!(text, "Current week: {n}");
            list_materials(
                &mut text,
                &format!("Week {n} materials"),
                &this_week.materials,
                context,
            );
            let next = views::week_materials(store, &course.id, Some(n + 1), true, at)?;
            list_materials(
                &mut text,
                &format!("Week {} materials", n + 1),
                &next.materials,
                context,
            );
        }
        None => {
            text.push_str("Current week: unknown\n");
            list_materials(&mut text, "Recent materials", &this_week.materials, context);
        }
    }
    let deadlines = views::deadlines(store, Some(&course.id), days, 0, true, at)?;
    let _ = writeln!(text, "Deadlines in the next {days} days:");
    if deadlines.is_empty() {
        text.push_str("- none\n");
    }
    for deadline in deadlines {
        let when = deadline.event.when().map(|t| t.date_naive());
        let _ = writeln!(
            text,
            "- {} {} ({})",
            when.map(|d: NaiveDate| d.to_string()).unwrap_or_default(),
            deadline.event.title,
            deadline.event.kind.as_str()
        );
    }
    Ok(text)
}

fn list_materials(
    text: &mut String,
    heading: &str,
    materials: &[MaterialView],
    context: &mut GatedContext,
) {
    let _ = writeln!(text, "{heading}:");
    if materials.is_empty() {
        text.push_str("- none\n");
    }
    for material in materials {
        let _ = writeln!(
            text,
            "- [{}] {} ({})",
            material.id,
            material.title,
            material.kind.as_str()
        );
        if !context
            .manifest
            .materials
            .iter()
            .any(|m| m.material_id == material.id)
        {
            context.manifest.materials.push(ManifestEntry {
                material_id: material.id.clone(),
                content_hash: None,
                chunk_ords: Vec::new(),
            });
        }
    }
}

/// Why a week material is left out of an explanation, if it is.
fn left_out(store: &Store, view: &MaterialView) -> crate::Result<Option<LeftOutReason>> {
    if view.kind == MaterialKind::ExternalLink {
        return Ok(Some(LeftOutReason::ExternalLink));
    }
    if looks_like_assessment(&view.title) {
        return Ok(Some(LeftOutReason::LooksLikeAssessment));
    }
    let readable = store
        .get_material(&view.id)?
        .is_some_and(|m| m.text_status == TextStatus::Ok);
    if !readable || store.chunk_count(&view.id)? == 0 {
        return Ok(Some(LeftOutReason::NoText));
    }
    Ok(None)
}

/// A title with an assessment word and no study word (whole words, any case).
pub(crate) fn looks_like_assessment(title: &str) -> bool {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let joined = format!(" {} ", words.join(" "));
    let has = |phrase: &str| {
        let phrase = format!(" {phrase} ");
        // "hw3", "hw 3": a word that starts with "hw" followed by digits counts as "hw".
        joined.contains(&phrase)
            || (phrase == " hw "
                && words.iter().any(|w| {
                    w.strip_prefix("hw").is_some_and(|rest| {
                        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
                    })
                }))
    };
    ASSESSMENT_WORDS.iter().any(|w| has(w)) && !STUDY_WORDS.iter().any(|w| has(w))
}

/// Most week-numbered material titles in a calendar reading's structure block.
const CALENDAR_STRUCTURE_TITLES: usize = 40;
/// Most class events in a calendar reading's structure block.
const CALENDAR_STRUCTURE_EVENTS: usize = 20;

/// A syllabus reading's context (calendar design §7.3): gated like an explanation (visible,
/// `readable`, question (b) allowing `destination`), then the course's candidates (§7.1), each
/// with the chunks that say most about dates within a fair share of `budget`, up to 8 dated
/// chunks of other materials, and a structure block (term, session window, modules, week
/// numbers, class events) that only helps the model tell which year the materials are for.
/// `NoReadableMaterials` when no candidate has text.
pub fn calendar_context(
    store: &Store,
    course: &str,
    at: AsOf,
    destination: Destination,
    budget: ContextBudget,
    signals: &CandidateSignals,
) -> Result<GatedContext, GateError> {
    let result = store.in_read_transaction(|store| {
        let course = store.resolve_course_with(course, true)?;
        if course.hidden {
            return Ok(Err(BlockReason::CourseHidden));
        }
        match course.ai_materials() {
            AiMaterialsState::Readable => {}
            AiMaterialsState::TurnedOff => return Ok(Err(BlockReason::CourseAiTurnedOff)),
            AiMaterialsState::WithheldByPolicy => {
                return Ok(Err(BlockReason::CoursePolicyProhibited));
            }
        }
        if !course.material_sharing.allows(destination) {
            return Ok(Err(BlockReason::MaterialSharingNotAllowed));
        }
        let data = CourseData::load(store, &course)?;
        let (resolved, _) = data.timeline(&course, at);
        let candidates = candidates_in(store, &data, &resolved, signals)?;
        let mut context = GatedContext::empty();
        let mut read = Vec::new();
        for scored in &candidates {
            let candidate = &scored.candidate;
            let reason = match candidate.left_out {
                None => {
                    read.push(candidate);
                    continue;
                }
                Some(CandidateLeftOut::NoText | CandidateLeftOut::Scanned) => LeftOutReason::NoText,
                Some(CandidateLeftOut::OverBudget) => LeftOutReason::OverBudget,
                Some(CandidateLeftOut::ExcludedByStudent) => continue,
            };
            context.summary.left_out.push(LeftOutMaterial {
                material_id: candidate.material_id.clone(),
                title: candidate.title.clone(),
                reason,
            });
        }
        if read.is_empty() {
            return Ok(Err(BlockReason::NoReadableMaterials));
        }

        // Dated chunks of other materials (a schedule on the first lecture's slides).
        let read_ids: Vec<&str> = read.iter().map(|c| c.material_id.as_str()).collect();
        let mut others = Vec::new();
        for material in &data.materials {
            let usable = !read_ids.contains(&material.id.as_str())
                && material.kind != MaterialKind::ExternalLink
                && material.text_status == TextStatus::Ok
                && data.chunks_of(&material.id) > 0
                && !looks_like_assessment(&material.title);
            if usable {
                others.extend(store.get_chunks(&material.id, 0, None)?);
            }
        }
        let extras = extra_chunks(&others);
        let extra_chars: usize = others
            .iter()
            .filter(|chunk| {
                extras
                    .iter()
                    .any(|(id, ord)| *id == chunk.material_id && *ord == chunk.ord)
            })
            .map(|chunk| chunk.text.chars().count())
            .sum();

        context.blocks.push(Block::Structure(calendar_structure(
            &course, &data, &resolved, at,
        )));
        let texts = read
            .iter()
            .map(|c| store.get_chunks(&c.material_id, 0, None))
            .collect::<crate::Result<Vec<_>>>()?;
        let sizes: Vec<usize> = texts
            .iter()
            .map(|chunks| chunks.iter().map(|c| c.text.chars().count()).sum())
            .collect();
        let shares = fair_shares(&sizes, budget.max_chars.saturating_sub(extra_chars));
        for ((candidate, chunks), share) in read.iter().zip(&texts).zip(shares) {
            let selection = select_chunks(chunks, share);
            if selection.chosen.is_empty() {
                context.summary.left_out.push(LeftOutMaterial {
                    material_id: candidate.material_id.clone(),
                    title: candidate.title.clone(),
                    reason: LeftOutReason::OverBudget,
                });
                continue;
            }
            let material = data
                .materials
                .iter()
                .find(|m| m.id == candidate.material_id);
            push_chunks(
                &mut context,
                candidate.material_id.as_str(),
                &candidate.title,
                candidate.url.as_deref(),
                material.and_then(|m| m.content_hash.clone()),
                chunks
                    .iter()
                    .filter(|chunk| selection.chosen.contains(&chunk.ord)),
            );
            context.summary.materials_included += 1;
            if !selection.skipped.is_empty() {
                context.summary.materials_trimmed += 1;
            }
        }
        for material in &data.materials {
            let chosen: Vec<&Chunk> = others
                .iter()
                .filter(|chunk| chunk.material_id == material.id)
                .filter(|chunk| {
                    extras
                        .iter()
                        .any(|(id, ord)| *id == chunk.material_id && *ord == chunk.ord)
                })
                .collect();
            if chosen.is_empty() {
                continue;
            }
            push_chunks(
                &mut context,
                &material.id,
                &material.title,
                material.url.as_deref(),
                material.content_hash.clone(),
                chosen.into_iter(),
            );
            context.summary.materials_included += 1;
            context.summary.materials_trimmed += 1;
        }
        context.summary.courses.push(ContextCourse {
            course_id: course.id.clone(),
            state: AiMaterialsState::Readable,
            text_included: true,
        });
        Ok(Ok(context))
    })?;
    result.map_err(GateError::Blocked)
}

/// Add `chunks` of one material, each with its own handle, and its manifest entry.
fn push_chunks<'a>(
    context: &mut GatedContext,
    material_id: &str,
    title: &str,
    url: Option<&str>,
    content_hash: Option<String>,
    chunks: impl Iterator<Item = &'a Chunk>,
) {
    let mut ords = Vec::new();
    for chunk in chunks {
        ords.push(chunk.ord);
        let handle = format!("c{}", context.handles() + 1);
        context.blocks.push(Block::Material {
            handle,
            target: CitationTarget {
                material_id: material_id.to_string(),
                title: title.to_string(),
                locator: chunk.locator.clone(),
                url: url.map(str::to_string),
            },
            text: chunk.text.clone(),
        });
    }
    context.manifest.materials.push(ManifestEntry {
        material_id: material_id.to_string(),
        content_hash,
        chunk_ords: ords,
    });
}

/// The structure a syllabus reading gets (rule 8 structure, wrapped as data): it only helps
/// the model tell which year the materials are for.
fn calendar_structure(
    course: &Course,
    data: &CourseData,
    resolved: &ResolvedTerm,
    at: AsOf,
) -> String {
    let mut text = format!("Course: {}\nToday: {}\n", course.display_name(), at.today);
    let lms = &data.term_data.lms;
    if let Some(name) = &lms.term_name {
        let _ = write!(text, "LMS term: {name}");
        if let (Some(start), Some(end)) = (lms.term_start, lms.term_end) {
            let _ = write!(text, " ({start} to {end})");
        }
        text.push('\n');
    }
    if let Some(session) = &resolved.session {
        let _ = writeln!(
            text,
            "Session {}: {} to {}",
            session.session, session.window.start, session.window.end
        );
    }
    let unlocking: Vec<&Module> = data
        .modules
        .iter()
        .filter(|m| m.unlock_at.is_some())
        .collect();
    if !unlocking.is_empty() {
        text.push_str("Modules:\n");
        for module in unlocking {
            if let Some(unlock) = module.unlock_at {
                let _ = writeln!(
                    text,
                    "- {} (unlocks {})",
                    module.name,
                    course_date(unlock, resolved.tz)
                );
            }
        }
    }
    let mut numbered: Vec<(u32, NaiveDate, &str)> = data
        .materials
        .iter()
        .filter_map(|m| {
            Some((
                m.week_hint?,
                course_date(m.published_at?, resolved.tz),
                m.title.as_str(),
            ))
        })
        .collect();
    numbered.sort();
    if !numbered.is_empty() {
        text.push_str("Materials with week numbers:\n");
        for (week, posted, title) in numbered.iter().take(CALENDAR_STRUCTURE_TITLES) {
            let _ = writeln!(text, "- Week {week}: {title} (posted {posted})");
        }
    }
    let mut classes: Vec<(NaiveDate, &str)> = data
        .events
        .iter()
        .filter(|event| event.kind == EventKind::ClassEvent)
        .filter_map(|event| {
            let at = event.starts_at.or(event.due_at)?;
            Some((course_date(at, resolved.tz), event.title.as_str()))
        })
        .collect();
    classes.sort();
    if !classes.is_empty() {
        text.push_str("Class events:\n");
        for (day, title) in classes.iter().take(CALENDAR_STRUCTURE_EVENTS) {
            let _ = writeln!(text, "- {day}: {title}");
        }
    }
    text.trim_end().to_string()
}

/// Split `budget` characters fairly: every material gets the same share, and what a small one
/// doesn't need goes to the others.
fn fair_shares(sizes: &[usize], budget: usize) -> Vec<usize> {
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| sizes[i]);
    let mut shares = vec![0; sizes.len()];
    let mut remaining = budget;
    for (position, &i) in order.iter().enumerate() {
        let left = sizes.len() - position;
        let share = (remaining / left).min(sizes[i]);
        shares[i] = share;
        remaining -= share;
    }
    shares
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assessments_are_recognised_by_title_but_study_material_is_kept() {
        for title in [
            "Assignment 2",
            "Homework 3 (due Oct 14)",
            "HW4",
            "hw 5",
            "Problem Set 1",
            "Quiz 2",
            "Final Exam",
            "Midterm",
            "Lab report template",
        ] {
            assert!(looks_like_assessment(title), "{title}");
        }
        for title in [
            "Midterm review",
            "Practice quiz solutions",
            "Exam preparation notes",
            "Week 3 slides",
            "Lecture 5: testing hypotheses",
            "Homeworking tips",
            "Shows",
        ] {
            assert!(!looks_like_assessment(title), "{title}");
        }
    }

    #[test]
    fn shares_are_fair_and_small_materials_give_back() {
        assert_eq!(fair_shares(&[100, 100], 100), vec![50, 50]);
        assert_eq!(fair_shares(&[10, 1000, 1000], 310), vec![10, 150, 150]);
        assert_eq!(fair_shares(&[10, 20], 1000), vec![10, 20]);
        assert_eq!(fair_shares(&[], 1000), Vec::<usize>::new());
    }
}
