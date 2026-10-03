//! The study-plan scheduler (v0.3; design §5.1): PURE and deterministic. The model proposes
//! tasks from structure only; this puts them on days within the student's capacity. Dates never
//! come from the model's say-so alone: every task lands inside its own window and the horizon,
//! or is listed as unscheduled with a reason.
//!
//! - Capacity: study hours per week ÷ study days per week, at most 4 hours a day.
//! - Order: by the latest allowed day, then priority, then kind (deadline preparation and
//!   reviews first), then the model's order.
//! - Placement: on the study day in the task's window with the most time left (ties: the
//!   earliest), which spreads the work; a task too long for any single day is split across days;
//!   a task that doesn't fit at all is unscheduled.
//! - Limits of a saved plan (`store::MAX_PLAN_*`) are enforced; material and course ids the
//!   context didn't contain are dropped.

use std::collections::{BTreeMap, HashSet};

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::model::{StudyPlan, StudyPlanItem};
use crate::store::{
    MAX_PLAN_ID_CHARS, MAX_PLAN_ITEMS, MAX_PLAN_MATERIAL_IDS, MAX_PLAN_TEXT_CHARS,
    MAX_PLAN_TITLE_CHARS,
};

/// The longest plan (days).
pub const MAX_HORIZON_DAYS: u32 = 56;
/// The most study time on one day (minutes).
pub const MAX_MINUTES_PER_DAY: u32 = 240;
/// The shortest task or task part (minutes).
pub const MIN_TASK_MINUTES: u32 = 15;

/// The model's answer for a study plan: tasks, no dates beyond each task's window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanTasks {
    pub tasks: Vec<PlanTask>,
}

/// One task the model proposes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanTask {
    /// The `course_id` from the context, or null for general study.
    pub course_id: Option<String>,
    pub kind: TaskKind,
    /// What to do, e.g. "Re-read the week 4 slides before A2 is due". Never "write A2's
    /// answers": no task produces graded work.
    pub title: String,
    pub description: Option<String>,
    /// Material ids from the context.
    pub material_ids: Vec<String>,
    /// Estimated minutes.
    pub minutes: u32,
    pub priority: TaskPriority,
    /// Not before this day (e.g. when a material is released).
    pub earliest: Option<NaiveDate>,
    /// Not after this day (e.g. the day before a deadline or exam).
    pub latest: Option<NaiveDate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Read,
    Review,
    Practice,
    PrepareDeadline,
    CatchUp,
}

impl TaskKind {
    /// Scheduling order among tasks with the same latest day and priority.
    fn rank(self) -> u8 {
        match self {
            TaskKind::PrepareDeadline => 0,
            TaskKind::Review => 1,
            TaskKind::Practice => 2,
            TaskKind::Read => 3,
            TaskKind::CatchUp => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    High,
    Normal,
    Low,
}

impl TaskPriority {
    fn rank(self) -> u8 {
        match self {
            TaskPriority::High => 0,
            TaskPriority::Normal => 1,
            TaskPriority::Low => 2,
        }
    }
}

/// What the scheduler needs besides the tasks.
#[derive(Clone, Debug)]
pub struct PlannerInput<'a> {
    /// The plan's first day (usually today).
    pub start: NaiveDate,
    /// Days the plan covers (1–56).
    pub horizon_days: u32,
    pub hours_per_week: u32,
    pub days_off: &'a [Weekday],
    /// Material ids that were in the context (others are dropped).
    pub known_material_ids: &'a HashSet<String>,
    /// Course ids that were in the context (others become general study).
    pub known_course_ids: &'a HashSet<String>,
}

/// Why a task isn't in the plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnscheduledReason {
    /// Its window doesn't overlap the plan's dates.
    OutsideHorizon,
    /// Its window has no study day (all days off).
    NoStudyDays,
    /// Not enough study time left in its window.
    NoTimeBeforeLatest,
    /// The plan is full (`MAX_PLAN_ITEMS`).
    TooManyItems,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UnscheduledTask {
    pub title: String,
    pub course_id: Option<String>,
    pub reason: UnscheduledReason,
}

/// The scheduled plan.
#[derive(Clone, Debug)]
pub struct PlannerOutput {
    pub plan: StudyPlan,
    pub unscheduled: Vec<UnscheduledTask>,
    /// Material ids the model gave that weren't in the context.
    pub dropped_material_ids: u32,
}

/// Put `tasks` on days (see the module docs).
pub fn schedule(tasks: &[PlanTask], input: &PlannerInput<'_>) -> PlannerOutput {
    let horizon = input.horizon_days.clamp(1, MAX_HORIZON_DAYS);
    let start = input.start;
    let end = start + Duration::days(i64::from(horizon) - 1);
    let days_off: HashSet<Weekday> = input.days_off.iter().copied().collect();
    let study_days_per_week = (7 - days_off.len().min(7)) as u32;
    let per_day = (input.hours_per_week.saturating_mul(60))
        .checked_div(study_days_per_week)
        .unwrap_or(0)
        .min(MAX_MINUTES_PER_DAY);
    let mut capacity: BTreeMap<NaiveDate, u32> = (0..horizon)
        .map(|offset| start + Duration::days(i64::from(offset)))
        .filter(|day| !days_off.contains(&day.weekday()))
        .map(|day| (day, per_day))
        .collect();

    let mut order: Vec<usize> = (0..tasks.len()).collect();
    order.sort_by_key(|&i| {
        let task = &tasks[i];
        (
            task.latest.unwrap_or(end).min(end),
            task.priority.rank(),
            task.kind.rank(),
            i,
        )
    });

    let mut placed: Vec<(NaiveDate, usize, StudyPlanItem)> = Vec::new();
    let mut unscheduled = Vec::new();
    let mut dropped_material_ids = 0;
    for &index in &order {
        let task = &tasks[index];
        let course_id = task
            .course_id
            .clone()
            .filter(|id| input.known_course_ids.contains(id));
        let unscheduled_as = |reason| UnscheduledTask {
            title: clip(&task.title, MAX_PLAN_TITLE_CHARS),
            course_id: course_id.clone(),
            reason,
        };
        let low = task.earliest.map_or(start, |d| d.max(start));
        let high = task.latest.map_or(end, |d| d.min(end));
        if low > high {
            unscheduled.push(unscheduled_as(UnscheduledReason::OutsideHorizon));
            continue;
        }
        let window: Vec<NaiveDate> = capacity.range(low..=high).map(|(day, _)| *day).collect();
        if window.is_empty() || per_day == 0 {
            unscheduled.push(unscheduled_as(UnscheduledReason::NoStudyDays));
            continue;
        }
        let minutes = task.minutes.clamp(MIN_TASK_MINUTES, MAX_MINUTES_PER_DAY);
        let free: u32 = window.iter().map(|day| capacity[day]).sum();
        if free < minutes {
            unscheduled.push(unscheduled_as(UnscheduledReason::NoTimeBeforeLatest));
            continue;
        }
        // Whole on the emptiest day, or split across the emptiest days.
        let mut parts: Vec<(NaiveDate, u32)> = Vec::new();
        let mut left = minutes;
        while left > 0 {
            let Some(day) = window
                .iter()
                .filter(|day| capacity[*day] >= MIN_TASK_MINUTES.min(left))
                .max_by_key(|day| (capacity[*day], std::cmp::Reverse(**day)))
                .copied()
            else {
                break;
            };
            let take = left.min(capacity[&day]);
            parts.push((day, take));
            left -= take;
            *capacity.get_mut(&day).expect("window days have capacity") -= take;
        }
        if left > 0 {
            // Only crumbs smaller than a task part were left: give the time back.
            for (day, take) in parts {
                *capacity.get_mut(&day).expect("window days have capacity") += take;
            }
            unscheduled.push(unscheduled_as(UnscheduledReason::NoTimeBeforeLatest));
            continue;
        }
        let known: Vec<String> = task
            .material_ids
            .iter()
            .filter(|id| input.known_material_ids.contains(*id))
            .take(MAX_PLAN_MATERIAL_IDS)
            .map(|id| clip(id, MAX_PLAN_ID_CHARS))
            .collect();
        dropped_material_ids += (task.material_ids.len() - known.len()) as u32;
        let split = parts.len() > 1;
        parts.sort_by_key(|(day, _)| *day);
        for (part, (day, take)) in parts.into_iter().enumerate() {
            let title = if split {
                format!("{} (part {})", task.title, part + 1)
            } else {
                task.title.clone()
            };
            placed.push((
                day,
                placed.len(),
                StudyPlanItem {
                    date: day,
                    course_id: course_id.clone().map(|id| clip(&id, MAX_PLAN_ID_CHARS)),
                    title: clip(&title, MAX_PLAN_TITLE_CHARS),
                    description: task
                        .description
                        .as_deref()
                        .map(|text| clip(text, MAX_PLAN_TEXT_CHARS)),
                    material_ids: known.clone(),
                    minutes: Some(take),
                    done: false,
                },
            ));
        }
    }
    placed.sort_by_key(|(day, order, _)| (*day, *order));
    let mut items: Vec<StudyPlanItem> = placed.into_iter().map(|(_, _, item)| item).collect();
    if items.len() > MAX_PLAN_ITEMS {
        for item in items.split_off(MAX_PLAN_ITEMS) {
            unscheduled.push(UnscheduledTask {
                title: item.title,
                course_id: item.course_id,
                reason: UnscheduledReason::TooManyItems,
            });
        }
    }
    PlannerOutput {
        plan: StudyPlan {
            horizon_start: start,
            horizon_end: end,
            items,
            notes: None,
        },
        unscheduled,
        dropped_material_ids,
    }
}

/// `text` cut to at most `max` characters.
fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// Words naming graded work ("A2", "HW3", "PS1" and "Q4" count too).
const GRADED_WORDS: [&str; 14] = [
    "assignment",
    "assignments",
    "homework",
    "hw",
    "pset",
    "quiz",
    "exam",
    "midterm",
    "test",
    "lab",
    "project",
    "essay",
    "report",
    "paper",
];
/// Verbs that produce work when their object is an answer, a solution or a write-up.
const PRODUCE_VERBS: [&str; 9] = [
    "write", "draft", "produce", "compose", "complete", "finish", "do", "fill", "submit",
];
/// What producing graded work makes.
const OUTPUT_NOUNS: [&str; 12] = [
    "answer",
    "answers",
    "solution",
    "solutions",
    "response",
    "responses",
    "essay",
    "report",
    "code",
    "submission",
    "questions",
    "problems",
];
/// Words that make it study (practice material, going over posted solutions).
const PRACTICE_WORDS: [&str; 5] = ["practice", "sample", "past", "mock", "old"];
const STUDY_VERBS: [&str; 6] = ["review", "reviewing", "read", "reread", "study", "check"];

/// Whether a task produces graded work: it names graded work ("A2", "quiz", "essay"…) and asks
/// for answers, solutions or a write-up of it ("Write A2's answers", "Solve HW3", "Quiz 4
/// solutions"). Practice material ("Solve the practice midterm") and going over posted
/// solutions ("Review the quiz 3 solutions") are study (design §5.1; M3 DoD 1). The plan never
/// contains such a task, whatever the model proposed.
pub fn produces_graded_work(task: &PlanTask) -> bool {
    text_produces_graded_work(&task.title, task.description.as_deref())
}

/// `produces_graded_work` for any title and description (a weekly note's focus items too).
pub fn text_produces_graded_work(title: &str, description: Option<&str>) -> bool {
    let text = format!("{title} {}", description.unwrap_or(""));
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let numbered = |w: &str, prefix: &str| {
        w.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    };
    let graded = words.iter().any(|w| {
        GRADED_WORDS.contains(&w.as_str())
            || ["a", "hw", "ps", "q"]
                .iter()
                .any(|prefix| numbered(w, prefix))
    }) || text.to_lowercase().contains("problem set");
    if !graded || words.iter().any(|w| PRACTICE_WORDS.contains(&w.as_str())) {
        return false;
    }
    // The first cue that asks for the work itself.
    let cue = words.iter().enumerate().position(|(i, w)| {
        // "answer(s)", "solution(s)" themselves, or "solve".
        OUTPUT_NOUNS[..4].contains(&w.as_str())
            || w == "solve"
            || (PRODUCE_VERBS.contains(&w.as_str())
                && words[i + 1..]
                    .iter()
                    .take(4)
                    .any(|next| OUTPUT_NOUNS.contains(&next.as_str())))
    });
    // Going over something first ("Review the posted solutions") is study.
    cue.is_some_and(|at| {
        !words[..at]
            .iter()
            .any(|w| STUDY_VERBS.contains(&w.as_str()))
    })
}

/// `tasks` without the ones that produce graded work (`produces_graded_work`), and how many
/// were left out.
pub fn without_graded_work(tasks: Vec<PlanTask>) -> (Vec<PlanTask>, u32) {
    let before = tasks.len();
    let kept: Vec<PlanTask> = tasks
        .into_iter()
        .filter(|task| !produces_graded_work(task))
        .collect();
    let left_out = u32::try_from(before - kept.len()).unwrap_or(u32::MAX);
    (kept, left_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    fn task(title: &str, minutes: u32, latest: Option<NaiveDate>) -> PlanTask {
        PlanTask {
            course_id: Some("c1".into()),
            kind: TaskKind::Read,
            title: title.into(),
            description: None,
            material_ids: vec!["m1".into(), "invented".into()],
            minutes,
            priority: TaskPriority::Normal,
            earliest: None,
            latest,
        }
    }

    struct Known {
        materials: HashSet<String>,
        courses: HashSet<String>,
    }

    impl Known {
        fn new() -> Known {
            Known {
                materials: ["m1".to_string()].into(),
                courses: ["c1".to_string()].into(),
            }
        }

        fn input(&self, hours: u32, days_off: &'static [Weekday]) -> PlannerInput<'_> {
            PlannerInput {
                // 2026-10-05 is a Monday.
                start: day(5),
                horizon_days: 7,
                hours_per_week: hours,
                days_off,
                known_material_ids: &self.materials,
                known_course_ids: &self.courses,
            }
        }
    }

    #[test]
    fn tasks_land_in_their_window_within_daily_capacity() {
        let known = Known::new();
        // 10 h over 5 study days = 120 min a day.
        let input = known.input(10, &[Weekday::Sat, Weekday::Sun]);
        let tasks = vec![
            task("Read week 4 slides", 60, Some(day(6))),
            task("Review for the quiz", 90, Some(day(6))),
            task("Practice problems", 60, None),
        ];
        let output = schedule(&tasks, &input);
        assert!(output.unscheduled.is_empty(), "{:?}", output.unscheduled);
        let mut per_day: BTreeMap<NaiveDate, u32> = BTreeMap::new();
        for item in &output.plan.items {
            assert!(item.date >= day(5) && item.date <= day(11));
            assert!(!matches!(item.date.weekday(), Weekday::Sat | Weekday::Sun));
            *per_day.entry(item.date).or_default() += item.minutes.unwrap();
        }
        assert!(per_day.values().all(|&m| m <= 120), "{per_day:?}");
        // Both deadline tasks are on or before their latest day.
        for item in output
            .plan
            .items
            .iter()
            .filter(|i| i.title != "Practice problems")
        {
            assert!(item.date <= day(6), "{item:?}");
        }
        // Invented material ids are dropped, known ones kept.
        assert!(
            output
                .plan
                .items
                .iter()
                .all(|i| i.material_ids == vec!["m1".to_string()])
        );
        assert_eq!(output.dropped_material_ids, 3);
    }

    #[test]
    fn long_tasks_split_and_impossible_ones_are_unscheduled_with_a_reason() {
        let known = Known::new();
        let input = known.input(5, &[]); // 300 min / 7 days = 42 min a day
        let output = schedule(&[task("Big reading", 120, None)], &input);
        let parts: Vec<&StudyPlanItem> = output.plan.items.iter().collect();
        assert!(parts.len() >= 3, "{parts:?}");
        assert!(parts[0].title.ends_with("(part 1)"));
        assert_eq!(parts.iter().map(|p| p.minutes.unwrap()).sum::<u32>(), 120);

        let tasks = vec![
            task("Due yesterday", 30, Some(day(4))),
            task("Too much for one day", 240, Some(day(5))),
            PlanTask {
                earliest: Some(day(20)),
                ..task("After the plan", 30, None)
            },
        ];
        let output = schedule(&tasks, &input);
        let reasons: Vec<UnscheduledReason> = output.unscheduled.iter().map(|u| u.reason).collect();
        assert_eq!(
            reasons,
            vec![
                UnscheduledReason::OutsideHorizon,
                UnscheduledReason::NoTimeBeforeLatest,
                UnscheduledReason::OutsideHorizon
            ]
        );
        assert!(output.plan.items.is_empty());
        // Every day off: nothing can be placed.
        let all_off = known.input(
            10,
            &[
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
                Weekday::Sat,
                Weekday::Sun,
            ],
        );
        let output = schedule(&[task("Anything", 30, None)], &all_off);
        assert_eq!(output.unscheduled[0].reason, UnscheduledReason::NoStudyDays);
    }

    #[test]
    fn urgent_and_high_priority_work_is_placed_first() {
        let known = Known::new();
        let input = known.input(2, &[]); // 17 min a day: one small task a day
        let low = PlanTask {
            priority: TaskPriority::Low,
            minutes: 15,
            ..task("Low", 15, Some(day(5)))
        };
        let high = PlanTask {
            priority: TaskPriority::High,
            minutes: 15,
            ..task("High", 15, Some(day(5)))
        };
        let output = schedule(&[low, high], &input);
        assert_eq!(output.plan.items.len(), 1);
        assert_eq!(output.plan.items[0].title, "High");
        assert_eq!(output.unscheduled[0].title, "Low");
    }

    #[test]
    fn unknown_courses_become_general_study_and_limits_hold() {
        let known = Known::new();
        let input = PlannerInput {
            horizon_days: 56,
            hours_per_week: 40,
            ..known.input(40, &[])
        };
        let mut many = Vec::new();
        for n in 0..250 {
            many.push(PlanTask {
                course_id: Some("made-up".into()),
                title: "x".repeat(400) + &n.to_string(),
                ..task("", 15, None)
            });
        }
        let output = schedule(&many, &input);
        assert_eq!(output.plan.items.len(), MAX_PLAN_ITEMS);
        assert!(output.plan.items.iter().all(|i| i.course_id.is_none()));
        assert!(
            output
                .plan
                .items
                .iter()
                .all(|i| i.title.chars().count() <= MAX_PLAN_TITLE_CHARS)
        );
        assert_eq!(
            output
                .unscheduled
                .iter()
                .filter(|u| u.reason == UnscheduledReason::TooManyItems)
                .count(),
            50
        );
        assert_eq!(output.plan.horizon_end, day(5) + Duration::days(55));
    }

    fn titled(title: &str) -> PlanTask {
        PlanTask {
            course_id: None,
            kind: TaskKind::PrepareDeadline,
            title: title.into(),
            description: None,
            material_ids: Vec::new(),
            minutes: 30,
            priority: TaskPriority::Normal,
            earliest: None,
            latest: None,
        }
    }

    /// Prompt fixtures: what a plan may and may not contain (design §5.1; M3 DoD 1).
    #[test]
    fn tasks_that_produce_graded_work_are_left_out() {
        for title in [
            "Write A2's answers",
            "A2 answers: questions 1-4",
            "Solve HW3 problems 2 and 5",
            "Draft the essay for Assignment 1",
            "Quiz 4 solutions",
            "Complete the A3 questions",
            "Write the lab report",
            "Finish problem set 2 answers",
            "Answer the midterm questions",
        ] {
            assert!(produces_graded_work(&titled(title)), "{title}");
        }
        for title in [
            "Start A2 (due Oct 14): re-read week 4 slides",
            "Prepare for quiz 3: review lectures 5-6",
            "Solve the practice midterm",
            "Review the posted quiz 3 solutions",
            "Read chapter 4 before the lab",
            "Work through past exam questions",
            "Catch up on week 2 readings",
            "Review lecture notes before writing the lab report",
            "Practice problems on derivatives",
        ] {
            assert!(!produces_graded_work(&titled(title)), "{title}");
        }
        // The description counts too.
        let mut sneaky = titled("Assignment 2");
        sneaky.description = Some("Write the answers to questions 1-3.".into());
        assert!(produces_graded_work(&sneaky));
        let (kept, left_out) = without_graded_work(vec![
            titled("Write A2's answers"),
            titled("Re-read week 4 slides"),
        ]);
        assert_eq!((kept.len(), left_out), (1, 1));
    }

    /// A tiny deterministic generator (xorshift64*), so the property test needs no crate.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n.max(1)
        }
    }

    /// Scheduler properties over random tasks and settings: every item lies inside the horizon,
    /// its task's window and a study day; no day holds more than its capacity; limits hold; every
    /// task is either placed in full or unscheduled.
    #[test]
    fn random_plans_keep_every_date_inside_the_horizon_and_capacity() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let weekdays = [
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ];
        let known_materials: HashSet<String> = ["m1", "m2", "m3"].map(String::from).into();
        let known_courses: HashSet<String> = ["c1", "c2"].map(String::from).into();
        for round in 0..300 {
            let start = day(1) + Duration::days(rng.below(30) as i64);
            let horizon_days = 1 + rng.below(u64::from(MAX_HORIZON_DAYS)) as u32;
            let hours_per_week = rng.below(40) as u32;
            let days_off: Vec<Weekday> = weekdays
                .iter()
                .copied()
                .filter(|_| rng.below(4) == 0)
                .collect();
            let tasks: Vec<PlanTask> = (0..rng.below(30))
                .map(|i| {
                    let earliest = (rng.below(3) == 0)
                        .then(|| start + Duration::days(rng.below(70) as i64 - 7));
                    let latest = (rng.below(2) == 0)
                        .then(|| start + Duration::days(rng.below(70) as i64 - 7));
                    PlanTask {
                        course_id: ["c1", "c2", "c9"]
                            .get(rng.below(4) as usize)
                            .map(|c| (*c).to_string()),
                        kind: [
                            TaskKind::Read,
                            TaskKind::Review,
                            TaskKind::Practice,
                            TaskKind::PrepareDeadline,
                            TaskKind::CatchUp,
                        ][rng.below(5) as usize],
                        title: format!("Task {i}."),
                        description: None,
                        material_ids: vec!["m1".into(), "m9".into()],
                        minutes: rng.below(600) as u32,
                        priority: [TaskPriority::High, TaskPriority::Normal, TaskPriority::Low]
                            [rng.below(3) as usize],
                        earliest,
                        latest,
                    }
                })
                .collect();
            let input = PlannerInput {
                start,
                horizon_days,
                hours_per_week,
                days_off: &days_off,
                known_material_ids: &known_materials,
                known_course_ids: &known_courses,
            };
            let out = schedule(&tasks, &input);
            let end = start + Duration::days(i64::from(horizon_days) - 1);
            let study_days = 7 - days_off.len() as u32;
            let per_day = (hours_per_week * 60)
                .checked_div(study_days)
                .unwrap_or(0)
                .min(MAX_MINUTES_PER_DAY);
            let mut used: BTreeMap<NaiveDate, u32> = BTreeMap::new();
            for item in &out.plan.items {
                assert!(
                    start <= item.date && item.date <= end,
                    "round {round}: {item:?}"
                );
                assert!(!days_off.contains(&item.date.weekday()), "round {round}");
                *used.entry(item.date).or_default() += item.minutes.unwrap_or(0);
                assert!(
                    item.material_ids
                        .iter()
                        .all(|m| known_materials.contains(m))
                );
                assert!(
                    item.course_id
                        .as_ref()
                        .is_none_or(|c| known_courses.contains(c))
                );
                // Inside its own task's window.
                let task = tasks
                    .iter()
                    .find(|t| item.title.starts_with(&t.title))
                    .expect("an item comes from a task");
                assert!(
                    task.earliest.is_none_or(|e| e <= item.date),
                    "round {round}"
                );
                assert!(task.latest.is_none_or(|l| item.date <= l), "round {round}");
            }
            for (date, minutes) in used {
                assert!(minutes <= per_day, "round {round}: {date} holds {minutes}");
            }
            assert!(out.plan.items.len() <= MAX_PLAN_ITEMS);
            assert_eq!((out.plan.horizon_start, out.plan.horizon_end), (start, end));
        }
    }
}
