//! `pagelamp plan`, `explain`, `note` and `remind` (model-access design §7, CLI): study plans,
//! weekly explanations and weekly notes PageLamp writes itself, and the reminders and weekly
//! digest (cron-friendly). Every policy is the facade's; this only prints.

use std::collections::HashMap;

use chrono::Utc;
use clap::ValueEnum;
use pagelamp_app::ai::{
    BackendRef, ExplainOptions, GenEvent, GeneratedStudyPlan, PlanWarningCode, StudyPlanRequest,
    WeeklyExplanation, WeeklyNote, WeeklyNoteOptions,
};
use pagelamp_app::{App, DayOfWeek, Reminder, ReminderKind};
use pagelamp_core::ai::AiFeature;
use pagelamp_core::ai_gate::LeftOutReason;
use pagelamp_core::model::{PlanOrigin, StoredStudyPlan, StudyPlan};
use pagelamp_core::planner::UnscheduledReason;
use pagelamp_core::views::WeeklyDigest;
use serde::Serialize;

use crate::print_json;

/// A weekday on the command line.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum DayArg {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl From<DayArg> for DayOfWeek {
    fn from(day: DayArg) -> Self {
        match day {
            DayArg::Mon => DayOfWeek::Monday,
            DayArg::Tue => DayOfWeek::Tuesday,
            DayArg::Wed => DayOfWeek::Wednesday,
            DayArg::Thu => DayOfWeek::Thursday,
            DayArg::Fri => DayOfWeek::Friday,
            DayArg::Sat => DayOfWeek::Saturday,
            DayArg::Sun => DayOfWeek::Sunday,
        }
    }
}

/// What `pagelamp plan` does.
pub struct PlanArgs {
    pub days: Option<u32>,
    pub hours: Option<u32>,
    pub days_off: Vec<DayArg>,
    pub courses: Vec<String>,
    pub note: Option<String>,
    pub over_budget: bool,
    pub save: bool,
    pub accept: Option<String>,
}

/// A facade error as the CLI says it: a run that can't start gives the facade's code and how
/// to fix it (like `ai estimate`).
fn said(err: pagelamp_app::AppError) -> anyhow::Error {
    match err.blocked {
        Some(reason) => anyhow::anyhow!(
            "blocked: {} — {}",
            reason.as_str(),
            crate::text::block_reason(reason)
        ),
        None => err.into(),
    }
}

/// PageLamp starts runs on the ChatGPT and Claude plans only when the student starts them (plan
/// D27), so the CLI refuses them when no terminal is attached: a run of `feature` on one of
/// them with neither stdin nor stderr a terminal (cron, a script) is refused before anything
/// starts, and the message points to an API key or a model on this computer, which may run on
/// a schedule. It stops an accidental crontab; it is not a lock (a faked terminal passes).
pub(crate) fn refuse_unattended_plan_run(app: &App, feature: AiFeature) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() || std::io::stderr().is_terminal() {
        return Ok(());
    }
    let backend = app
        .ai_status()?
        .features
        .into_iter()
        .find(|routing| routing.feature == feature)
        .and_then(|routing| routing.choice)
        .map(|choice| choice.backend);
    if matches!(backend, Some(BackendRef::Codex | BackendRef::ClaudeCode)) {
        let what = match feature {
            AiFeature::StudyPlan => "study plans",
            AiFeature::WeeklyExplanation => "explanations",
            AiFeature::WeeklyNote => "weekly notes",
            AiFeature::CourseCalendar => "reading syllabi",
        };
        anyhow::bail!(
            "refused: unattended_plan_run — PageLamp runs the ChatGPT and Claude plans only \
             when you start the run yourself. For scheduled runs (cron), choose an API key or \
             a model on this computer for {what}."
        );
    }
    Ok(())
}

/// A generation id for one CLI run.
fn generation_id(what: &str) -> String {
    format!(
        "cli-{what}-{}-{}",
        std::process::id(),
        Utc::now().timestamp()
    )
}

fn progress(event: GenEvent) {
    if let GenEvent::Started {
        backend_label,
        model,
        ..
    } = event
    {
        eprintln!("Asking {backend_label} · {model}…");
    }
}

/// Course codes by id, for printing.
fn codes(app: &App) -> HashMap<String, String> {
    app.list_courses()
        .map(|courses| {
            courses
                .into_iter()
                .filter_map(|summary| Some((summary.course.id, summary.course.code?)))
                .collect()
        })
        .unwrap_or_default()
}

/// `pagelamp plan`
pub async fn plan(app: &App, args: PlanArgs, json: bool) -> anyhow::Result<()> {
    if let Some(id) = &args.accept {
        let stored = app.accept_study_plan(id)?;
        if json {
            return print_json(&stored);
        }
        println!("Saved as your study plan.");
        print_stored(app, &stored);
        return Ok(());
    }
    refuse_unattended_plan_run(app, AiFeature::StudyPlan)?;
    let id = generation_id("plan");
    let request = StudyPlanRequest {
        horizon_days: args.days,
        hours_per_week: args.hours,
        days_off: args.days_off.into_iter().map(DayOfWeek::from).collect(),
        courses: args.courses,
        note: args.note,
        override_budget: args.over_budget,
    };
    let draft = app
        .generate_study_plan(request, &id, progress)
        .await
        .map_err(said)?;
    if args.save {
        let stored = app.accept_study_plan(&id)?;
        if json {
            return print_json(&stored);
        }
        println!("Saved as your study plan.");
        print_stored(app, &stored);
        return Ok(());
    }
    if json {
        return print_json(&draft);
    }
    print_draft(app, &draft);
    println!(
        "\nKeep it: `{} plan --accept {id}`",
        pagelamp_core::brand::CLI_NAME
    );
    Ok(())
}

fn print_draft(app: &App, draft: &GeneratedStudyPlan) {
    println!(
        "Draft plan · AI-generated · {} · {} · {}",
        draft.meta.backend_label,
        draft.meta.model,
        draft.meta.created_at.format("%Y-%m-%d")
    );
    print_plan(app, &draft.plan);
    for warning in &draft.warnings {
        match warning.code {
            PlanWarningCode::GradedWorkLeftOut => println!(
                "Left out {} task(s) that would have done graded work for you.",
                warning.count
            ),
            PlanWarningCode::UnknownMaterialsDropped => println!(
                "Dropped {} material id(s) the model made up.",
                warning.count
            ),
        }
    }
    for task in &draft.unscheduled {
        let why = match task.reason {
            UnscheduledReason::OutsideHorizon => "outside the plan's days",
            UnscheduledReason::NoStudyDays => "no study day in its window",
            UnscheduledReason::NoTimeBeforeLatest => "no time left before it's due",
            UnscheduledReason::TooManyItems => "the plan is full",
        };
        println!("Not scheduled: {} ({why})", task.title);
    }
}

fn print_stored(app: &App, stored: &StoredStudyPlan) {
    match (&stored.origin, &stored.ai_label) {
        (PlanOrigin::PageLamp, Some(label)) => println!(
            "AI-generated · {} · {} · {}",
            label.backend_label,
            label.model,
            label.created_at.format("%Y-%m-%d")
        ),
        (PlanOrigin::PageLamp, None) => println!("AI-generated by PageLamp"),
        (PlanOrigin::AiApp, _) => println!("Saved by your AI app"),
    }
    print_plan(app, &stored.plan);
}

fn print_plan(app: &App, plan: &StudyPlan) {
    let codes = codes(app);
    let mut day = None;
    for item in &plan.items {
        if day != Some(item.date) {
            day = Some(item.date);
            println!("\n{}", item.date.format("%a %Y-%m-%d"));
        }
        let course = item
            .course_id
            .as_ref()
            .and_then(|id| codes.get(id))
            .map(|code| format!(" [{code}]"))
            .unwrap_or_default();
        let minutes = item
            .minutes
            .map(|m| format!("{m} min "))
            .unwrap_or_default();
        let done = if item.done { "✓" } else { "•" };
        println!("  {done} {minutes}{}{course}", item.title);
    }
    if plan.items.is_empty() {
        println!("(no items)");
    }
}

/// What `pagelamp explain` does.
pub struct ExplainArgs {
    pub course: String,
    pub week: Option<u32>,
    pub include: Vec<String>,
    pub over_budget: bool,
    pub saved: bool,
}

/// `pagelamp explain`
pub async fn explain(app: &App, args: ExplainArgs, json: bool) -> anyhow::Result<()> {
    if args.saved {
        let saved = app.saved_explanations(&args.course, args.week)?;
        if json {
            return print_json(&saved);
        }
        if saved.is_empty() {
            println!("No saved explanations.");
        }
        for explanation in &saved {
            println!(
                "{} · week {} · {} section(s){}",
                explanation.meta.created_at.format("%Y-%m-%d %H:%M"),
                explanation
                    .week
                    .map_or_else(|| "unknown".to_string(), |w| w.to_string()),
                explanation.sections.len(),
                if explanation.stale {
                    " · materials changed since: run it again?"
                } else {
                    ""
                }
            );
        }
        return Ok(());
    }
    refuse_unattended_plan_run(app, AiFeature::WeeklyExplanation)?;
    let id = generation_id("explain");
    let options = ExplainOptions {
        include: args.include,
        // The command line speaks English.
        ui_language: Some("en".to_string()),
        override_budget: args.over_budget,
    };
    let explanation = app
        .explain_week(&args.course, args.week, &id, options, progress)
        .await
        .map_err(said)?;
    if json {
        return print_json(&explanation);
    }
    print_explanation(&explanation);
    Ok(())
}

fn print_explanation(explanation: &WeeklyExplanation) {
    if let Some(week) = explanation.week {
        println!("# Week {week}\n");
    }
    // Citations become numbered notes.
    let mut notes: Vec<String> = Vec::new();
    for section in &explanation.sections {
        println!("## {}\n", section.heading);
        for paragraph in &section.paragraphs {
            let marks: Vec<String> = paragraph
                .citations
                .iter()
                .map(|citation| {
                    let note = match &citation.locator {
                        Some(locator) => format!("{}, {locator}", citation.title),
                        None => citation.title.clone(),
                    };
                    let n = notes.iter().position(|n| *n == note).unwrap_or_else(|| {
                        notes.push(note);
                        notes.len() - 1
                    });
                    format!("[{}]", n + 1)
                })
                .collect();
            println!("{} {}\n", paragraph.text, marks.join(""));
        }
    }
    if !explanation.check_questions.is_empty() {
        println!("Check yourself:");
        for (n, question) in explanation.check_questions.iter().enumerate() {
            println!("  {}. {question}", n + 1);
        }
        println!();
    }
    for (n, note) in notes.iter().enumerate() {
        println!("[{}] {note}", n + 1);
    }
    for material in &explanation.left_out {
        let why = match material.reason {
            LeftOutReason::LooksLikeAssessment => "looks like graded work",
            LeftOutReason::ExternalLink => "a link",
            LeftOutReason::NoText => "no text",
            LeftOutReason::OverBudget => "no room",
        };
        println!(
            "Left out: {} ({why}; include it with --include {})",
            material.title, material.material_id
        );
    }
    println!(
        "\nAI-generated · {} · {} · {}",
        explanation.meta.backend_label,
        explanation.meta.model,
        explanation.meta.created_at.format("%Y-%m-%d")
    );
    if explanation.cite_ai_use {
        println!("This course asks you to cite AI use: follow its rules.");
    }
    if explanation.sharing_reminder {
        println!("{}", crate::text::sharing_reminder_note());
    }
}

/// `pagelamp note`: write a weekly note, or (`saved`) list the kept ones.
pub async fn note(app: &App, over_budget: bool, saved: bool, json: bool) -> anyhow::Result<()> {
    if saved {
        let notes = app.weekly_notes()?;
        if json {
            return print_json(&notes);
        }
        if notes.is_empty() {
            println!("No saved weekly notes.");
        }
        for note in &notes {
            print_note(note, &codes(app));
            println!();
        }
        return Ok(());
    }
    refuse_unattended_plan_run(app, AiFeature::WeeklyNote)?;
    let id = generation_id("note");
    let options = WeeklyNoteOptions {
        // The command line speaks English.
        ui_language: Some("en".to_string()),
        override_budget: over_budget,
        automatic: false,
    };
    let note = app
        .write_weekly_note(&id, options, progress)
        .await
        .map_err(said)?;
    if json {
        return print_json(&note);
    }
    print_note(&note, &codes(app));
    Ok(())
}

fn print_note(note: &WeeklyNote, codes: &HashMap<String, String>) {
    println!("# Week of {}\n", note.week_of);
    println!("{}\n", note.text);
    if !note.focus.is_empty() {
        println!("Focus on:");
        for (n, item) in note.focus.iter().enumerate() {
            match item.course_id.as_ref().and_then(|id| codes.get(id)) {
                Some(code) => println!("  {}. {} ({code})", n + 1, item.text),
                None => println!("  {}. {}", n + 1, item.text),
            }
        }
    }
    println!(
        "\nAI-generated · {} · {} · {}",
        note.meta.backend_label,
        note.meta.model,
        note.meta.created_at.format("%Y-%m-%d")
    );
}

#[derive(Serialize)]
struct RemindOutput<'a> {
    due: &'a [Reminder],
    digest: Option<&'a WeeklyDigest>,
}

/// `pagelamp remind`: the reminders due now (then marked shown, so no surface shows them
/// again), and the weekly digest when its reminder is due or `--digest` asks. Prints nothing
/// when nothing is due (cron-friendly).
pub fn remind(app: &App, digest: bool, json: bool) -> anyhow::Result<()> {
    let due = app.due_reminders(Utc::now())?;
    let digest_due = digest || due.iter().any(|r| r.kind == ReminderKind::WeeklyDigest);
    let weekly = if digest_due {
        Some(app.weekly_digest()?)
    } else {
        None
    };
    if json {
        print_json(&RemindOutput {
            due: &due,
            digest: weekly.as_ref(),
        })?;
    } else {
        for reminder in &due {
            println!("{}", reminder_text(reminder));
        }
        if let Some(weekly) = &weekly {
            print_digest(weekly);
        }
    }
    let shown: Vec<String> = due.into_iter().map(|r| r.id).collect();
    if !shown.is_empty() {
        app.mark_reminders_shown(&shown)?;
    }
    Ok(())
}

fn reminder_text(reminder: &Reminder) -> String {
    let course = reminder
        .course_code
        .as_deref()
        .map(|code| format!(" ({code})"))
        .unwrap_or_default();
    match reminder.kind {
        ReminderKind::DeadlineSoon => format!(
            "Due in {} h: {}{course}",
            reminder.hours_before.unwrap_or(24),
            reminder.title.as_deref().unwrap_or("a deadline")
        ),
        ReminderKind::WeeklyDigest => format!(
            "Your week: {} deadline(s) in the next 7 days.",
            reminder.count.unwrap_or(0)
        ),
        ReminderKind::PlanToday => format!(
            "Today's study plan: {} item(s) to do.",
            reminder.count.unwrap_or(0)
        ),
    }
}

fn print_digest(digest: &WeeklyDigest) {
    println!("\nThis week");
    for course in &digest.courses {
        let name = course.code.as_deref().unwrap_or(&course.name);
        match (course.active, course.week) {
            (true, Some(week)) => println!(
                "  {name} · week {week} · {} material(s){}",
                course.material_count,
                if course.material_titles.is_empty() {
                    String::new()
                } else {
                    format!(": {}", course.material_titles.join(", "))
                }
            ),
            _ => println!("  {name}"),
        }
        for deadline in &course.deadlines {
            let when = deadline
                .event
                .when()
                .map(|t| t.format("%a %m-%d").to_string())
                .unwrap_or_default();
            println!("    due {when}: {}", deadline.event.title);
        }
    }
    if let Some(plan) = &digest.plan {
        println!(
            "  Study plan: {} of {} done last week; {} item(s) today.",
            plan.last_week_done,
            plan.last_week_planned,
            plan.today.len()
        );
    }
}
