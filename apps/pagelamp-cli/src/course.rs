//! `pagelamp courses`, `course timeline`, `course calendar`, `course keep` and removing courses
//! (`course remove`, `removed`, `restore`, `purge`, `forget`): course weeks, phases, calendars,
//! the Past group and removal (docs/design/v0.3-course-calendar.md §7.13). Every default and
//! grouping comes from the facade; this module only renders.

use chrono::NaiveDate;
use pagelamp_app::ai::GenEvent;
use pagelamp_app::{
    App, CourseCalendarView, PurgeTargets, ReadCalendarOptions, RemovalPreview, RemoveOptions,
    RestoreFailure, TombstoneState,
};
use pagelamp_core::ai::AiFeature;
use pagelamp_core::calendar::CourseCalendar;
use pagelamp_core::calendar::assemble::DateKind;
use pagelamp_core::calendar::candidates::CandidateLeftOut;
use pagelamp_core::model::{
    AiLabel, CalendarOrigin, Confidence, CourseGroup, CourseLifecycle, CoursePhase, CourseTimeline,
    EvidenceItem, LifecycleState, TermAnchorSource,
};
use pagelamp_core::views::CourseSummary;

use crate::{ai_materials, confidence, confirm, print_json};

/// Which lifecycle groups `pagelamp courses` shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Groups {
    /// Current and Upcoming, with a count of the past courses.
    Active,
    Past,
    All,
}

impl Groups {
    fn shows(self, group: CourseGroup) -> bool {
        match self {
            Groups::Active => group != CourseGroup::Past,
            Groups::Past => group == CourseGroup::Past,
            Groups::All => true,
        }
    }
}

/// `pagelamp courses [--past|--all] [-v]`
pub fn list(app: &App, groups: Groups, evidence: bool, json: bool) -> anyhow::Result<()> {
    let all = app.list_courses()?;
    let courses: Vec<&CourseSummary> = all
        .iter()
        .filter(|summary| groups.shows(summary.lifecycle.group))
        .collect();
    if json {
        return print_json(&courses);
    }
    if all.is_empty() {
        println!(
            "No courses yet: add a source, then run `{} sync`.",
            pagelamp_core::brand::CLI_NAME
        );
        return Ok(());
    }
    for (group, title) in [
        (CourseGroup::Current, "Current"),
        (CourseGroup::Upcoming, "Upcoming"),
        (CourseGroup::Past, "Past"),
    ] {
        if !groups.shows(group) {
            continue;
        }
        let in_group: Vec<&CourseSummary> = courses
            .iter()
            .copied()
            .filter(|summary| summary.lifecycle.group == group)
            .collect();
        if in_group.is_empty() {
            continue;
        }
        println!("{title} ({})", in_group.len());
        for summary in in_group {
            println!("  {}", line(summary));
            if evidence {
                for text in summary
                    .timeline
                    .evidence
                    .iter()
                    .cloned()
                    .chain(lifecycle_lines(&summary.lifecycle))
                {
                    println!("      - {text}");
                }
            }
        }
    }
    let past = all
        .iter()
        .filter(|summary| summary.lifecycle.group == CourseGroup::Past)
        .count();
    if groups == Groups::Active && past > 0 {
        println!(
            "Past courses: {past} (`{} courses --past` lists them).",
            pagelamp_core::brand::CLI_NAME
        );
    }
    Ok(())
}

/// One course line: name, week or phase, next deadline, AI policy and access.
fn line(summary: &CourseSummary) -> String {
    let course = &summary.course;
    let next = summary
        .next_deadline
        .as_ref()
        .and_then(|d| {
            d.event.when().map(|w| {
                format!(
                    "  next: {} {}",
                    d.event.title,
                    w.with_timezone(&chrono::Local).format("%b %-d")
                )
            })
        })
        .unwrap_or_default();
    format!(
        "{:<40} {:<22} ai_policy={:<21} ai_materials={:<18}{}{}",
        course.display_name(),
        week_label(&summary.timeline, &summary.lifecycle),
        course.ai_policy.as_str(),
        ai_materials(summary.ai_materials),
        next,
        if course.hidden { "  [hidden]" } else { "" }
    )
}

/// A break week by the student's own dates. An upcoming course keeps its place in the week
/// views there, as in a teaching week of those dates (the facade's rule in
/// `pagelamp_core::views`): it has no current week then, so the week alone doesn't tell.
fn own_break(timeline: &CourseTimeline) -> bool {
    timeline.phase == CoursePhase::Break
        && timeline.term.anchor == TermAnchorSource::StudentConfirmed
}

/// "week 4 (medium)", "reading week", "exams (after week 12)", "ended", "starts 2027-01-11"…
/// A course that is over, inactive or hasn't started has no week: its lifecycle says which.
pub fn week_label(timeline: &CourseTimeline, lifecycle: &CourseLifecycle) -> String {
    let starts = || match lifecycle.starts_on {
        Some(start) => format!("starts {start}"),
        None => "not started".to_string(),
    };
    match lifecycle.state {
        LifecycleState::Ended => return "ended".to_string(),
        LifecycleState::Inactive => return "inactive".to_string(),
        // (With a week, or in a break of its own dates: the student's dates put an upcoming
        // course there, and it is shown like a running one.)
        LifecycleState::Upcoming if timeline.current_week.is_none() && !own_break(timeline) => {
            return starts();
        }
        LifecycleState::Upcoming
        | LifecycleState::Current
        | LifecycleState::Finishing
        | LifecycleState::Unknown => {}
    }
    let week = |week: u32, c: Confidence| format!("week {week} ({})", confidence(c));
    match timeline.phase {
        CoursePhase::Teaching | CoursePhase::Unknown => match timeline.current_week {
            Some(n) => week(n, timeline.confidence),
            None => "week ?".to_string(),
        },
        CoursePhase::Break => {
            let name = timeline
                .current_break_kind
                .map_or("break", |kind| match kind.as_str() {
                    "reading_week" => "reading week",
                    "winter_break" => "winter break",
                    "holiday" => "holiday",
                    _ => "break",
                });
            match (timeline.current_week, timeline.break_after_week) {
                (Some(n), _) => format!("{name} (week {n})"),
                (None, Some(after)) => format!("{name} (after week {after})"),
                (None, None) => name.to_string(),
            }
        }
        CoursePhase::ExamPeriod => match timeline.last_teaching_week {
            Some(after) => format!("exams (after week {after})"),
            None => "exams".to_string(),
        },
        CoursePhase::Ended => "ended".to_string(),
        CoursePhase::NotStarted => starts(),
    }
}

fn lifecycle_lines(lifecycle: &CourseLifecycle) -> impl Iterator<Item = String> + '_ {
    lifecycle.evidence_items.iter().map(EvidenceItem::english)
}

/// `pagelamp course timeline <course>`: where the course is, the dates used and not used,
/// and the evidence.
pub fn timeline(app: &App, course: &str, json: bool) -> anyhow::Result<()> {
    let overview = app.course_overview(course)?;
    if json {
        return print_json(&serde_json::json!({
            "course": overview.course,
            "timeline": overview.timeline,
            "lifecycle": overview.lifecycle,
        }));
    }
    let timeline = &overview.timeline;
    let lifecycle = &overview.lifecycle;
    println!("{}", overview.course.display_name());
    let phase = match timeline.phase {
        // The label names the other phases already ("exams (after week 12)", "ended"), and a
        // course outside the week views by its lifecycle ("inactive").
        CoursePhase::Teaching | CoursePhase::Unknown
            if lifecycle.state.in_week_views() || timeline.current_week.is_some() =>
        {
            format!(" · {}", timeline.phase.as_str())
        }
        _ => String::new(),
    };
    println!(
        "  Now:       {}{phase} (phase {})",
        week_label(timeline, lifecycle),
        confidence(timeline.phase_confidence)
    );
    println!(
        "  Lifecycle: {} ({}){}",
        lifecycle.state.as_str(),
        confidence(lifecycle.confidence),
        if lifecycle.suggest_removal {
            " — suggested for removal"
        } else {
            ""
        }
    );
    println!("  Dates:     {}", dates_used(timeline));
    if let Some(label) = &timeline.term.ai_label {
        println!(
            "             read by {} · {} on {}, confirmed by you",
            label.backend_label,
            label.model,
            label.created_at.format("%Y-%m-%d")
        );
    }
    for rejected in &timeline.term.not_used {
        let span = match (rejected.start, rejected.end) {
            (Some(start), Some(end)) => format!("{start} → {end}"),
            (Some(start), None) => format!("from {start}"),
            (None, Some(end)) if rejected.end_only => format!("end {end}"),
            (None, Some(end)) => format!("until {end}"),
            (None, None) => "no dates".to_string(),
        };
        println!(
            "  Not used:  {} ({span}): {}",
            source_name(rejected.source),
            rejected.reason.as_str().replace('_', " ")
        );
    }
    println!("  Evidence:");
    for text in timeline
        .evidence
        .iter()
        .cloned()
        .chain(lifecycle_lines(lifecycle))
    {
        println!("    - {text}");
    }
    Ok(())
}

fn dates_used(timeline: &CourseTimeline) -> String {
    let term = &timeline.term;
    let Some(first) = term.teaching.first() else {
        return "none known — set them with `course term <course> --start … --end …`".to_string();
    };
    let end = term
        .exams_end
        .or(term.teaching.last().and_then(|s| s.last_class))
        .map_or_else(|| "?".to_string(), |end| end.to_string());
    let origin = match term.anchor_origin {
        Some(CalendarOrigin::Legacy) => " — set in PageLamp 0.1, check them",
        _ => "",
    };
    format!(
        "{} → {end} from {} ({}){origin}",
        first.first_class,
        source_name(term.anchor),
        confidence(term.anchor_confidence)
    )
}

fn source_name(source: TermAnchorSource) -> &'static str {
    match source {
        TermAnchorSource::StudentConfirmed => "your dates",
        TermAnchorSource::LmsCourseDates => "the LMS course dates",
        TermAnchorSource::LmsTerm => "the LMS term",
        TermAnchorSource::FolderConfig => "the folder's dates",
        TermAnchorSource::InstitutionCalendar => "the school calendar",
        TermAnchorSource::PublishedWeekLabels => "the week numbers of posted materials",
        TermAnchorSource::NoAnchor => "nothing",
    }
}

/// `pagelamp course keep <course> [--until D | --clear]`
pub fn keep(
    app: &App,
    course: &str,
    until: Option<NaiveDate>,
    clear: bool,
    json: bool,
) -> anyhow::Result<()> {
    let course = if clear {
        app.clear_keep_course_current(course)?
    } else {
        app.keep_course_current(course, until)?
    };
    let lifecycle = app.course_overview(&course.id)?.lifecycle;
    if json {
        return print_json(&serde_json::json!({
            "course_id": course.id,
            "kept_current_until": lifecycle.kept_current_until,
            "lifecycle": lifecycle.state,
        }));
    }
    match lifecycle.kept_current_until {
        Some(until) => println!("{} counts as current until {until}.", course.display_name()),
        None => println!(
            "{} is back to its own dates ({}).",
            course.display_name(),
            lifecycle.state.as_str()
        ),
    }
    Ok(())
}

/// What `pagelamp course calendar` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarAction {
    Show,
    Scan,
    Read { over_budget: bool },
    Accept(i64),
    Dismiss(i64),
}

/// `pagelamp course calendar <course> [--scan|--read|--accept N|--dismiss N]`
pub async fn calendar(
    app: &App,
    course: &str,
    action: CalendarAction,
    json: bool,
) -> anyhow::Result<()> {
    match action {
        CalendarAction::Show => {}
        CalendarAction::Scan => match app.scan_course_calendar(course)? {
            Some(proposal) if !json => {
                println!("Proposal #{} from the syllabus scan.", proposal.id)
            }
            Some(_) => {}
            None if !json => println!("The scan found nothing new to propose."),
            None => {}
        },
        CalendarAction::Read { over_budget } => {
            crate::features::refuse_unattended_plan_run(app, AiFeature::CourseCalendar)?;
            let generation_id = format!("cli-{}", std::process::id());
            let options = ReadCalendarOptions {
                override_budget: over_budget,
            };
            let proposal = app
                .read_course_calendar(course, &generation_id, options, |event| {
                    if let GenEvent::Started {
                        backend_label,
                        model,
                        ..
                    } = event
                    {
                        eprintln!("Reading with {backend_label} · {model}…");
                    }
                })
                .await?;
            if !json {
                println!("Proposal #{} read by AI.", proposal.id);
                if proposal.sharing_reminder {
                    println!("{}", crate::text::sharing_reminder_note());
                }
            }
        }
        CalendarAction::Accept(id) => {
            app.accept_calendar_proposal(id, None)?;
        }
        CalendarAction::Dismiss(id) => app.dismiss_calendar_proposal(id)?,
    }
    let view = app.course_calendar(course)?;
    if json {
        return print_json(&view);
    }
    print_calendar_view(&view);
    Ok(())
}

fn print_calendar_view(view: &CourseCalendarView) {
    match &view.accepted {
        Some(accepted) => {
            let stale = if accepted.stale {
                " — a quoted material changed; read it again?"
            } else {
                ""
            };
            println!(
                "In force: {}{stale}",
                provenance(accepted.origin, accepted.ai_label.as_ref())
            );
            print_course_calendar(&accepted.calendar, "  ");
        }
        None => println!("No calendar in force."),
    }
    for proposal in &view.proposals {
        let verdict = if proposal.passing {
            "no conflicts".to_string()
        } else {
            format!("{} conflict(s) to choose between", proposal.conflicts.len())
        };
        let week = proposal
            .resulting_week_today
            .map_or_else(|| "no week".to_string(), |week| format!("week {week}"));
        println!(
            "Proposal #{}: {}; {verdict}; after accepting: {week}, {}",
            proposal.id,
            provenance(proposal.origin, proposal.ai_label.as_ref()),
            phase_word(proposal.resulting_phase)
        );
        for date in &proposal.dates {
            let when = match date.end {
                Some(end) => format!("{} – {end}", date.date),
                None => date.date.to_string(),
            };
            println!("  {:<11} {when}", kind_word(date.kind));
            for evidence in &date.evidence {
                if let Some(quote) = &evidence.quote {
                    let place = evidence
                        .locator
                        .as_deref()
                        .map_or_else(String::new, |l| format!(", {l}"));
                    println!("      \"{quote}\" — {}{place}", evidence.title);
                }
            }
        }
        let dropped: u32 = proposal.dropped.iter().map(|d| d.count).sum();
        if dropped > 0 {
            println!("  {dropped} item(s) left out: their words weren't in the materials");
        }
    }
    println!("Materials a reading uses:");
    for candidate in &view.candidates {
        let state = match candidate.left_out {
            None => "read".to_string(),
            Some(reason) => format!("not read: {}", left_out_word(reason)),
        };
        let download = if candidate.downloadable {
            " (can be downloaded)"
        } else {
            ""
        };
        println!("  [{state}] {}{download}", candidate.title);
    }
    if let Some(reason) = view.blocked {
        println!("Reading with AI: not now ({})", reason.as_str());
    }
}

fn print_course_calendar(calendar: &CourseCalendar, indent: &str) {
    for segment in &calendar.segments {
        let end = segment
            .last_class
            .map_or_else(|| "?".to_string(), |d| d.to_string());
        println!(
            "{indent}Classes {} – {end} (weeks from {})",
            segment.first_class, segment.first_week_number
        );
    }
    for item in &calendar.breaks {
        println!(
            "{indent}Break {} – {}{}",
            item.span.start,
            item.span.end,
            if item.numbered { " (numbered)" } else { "" }
        );
    }
    if let Some(exams) = calendar.exam_period {
        println!("{indent}Exams {} – {}", exams.start, exams.end);
    }
}

fn provenance(origin: CalendarOrigin, label: Option<&AiLabel>) -> String {
    match (origin, label) {
        (_, Some(label)) => format!(
            "read by AI · {} · {}{}",
            label.backend_label,
            label.model,
            if label.on_device {
                " (on this computer — check the dates)"
            } else {
                ""
            }
        ),
        (CalendarOrigin::Scan, None) => "from the syllabus scan".to_string(),
        (CalendarOrigin::User, None) => "your dates".to_string(),
        (CalendarOrigin::Legacy, None) => "set in PageLamp 0.1, check them".to_string(),
        (other, None) => other.as_str().to_string(),
    }
}

fn kind_word(kind: DateKind) -> &'static str {
    match kind {
        DateKind::FirstClass => "first class",
        DateKind::LastClass => "last class",
        DateKind::BreakSpan => "break",
        DateKind::ExamPeriod => "exams",
        DateKind::FinalExam => "final exam",
        DateKind::WeekStart => "week",
    }
}

fn left_out_word(reason: CandidateLeftOut) -> &'static str {
    match reason {
        CandidateLeftOut::NoText => "no text",
        CandidateLeftOut::Scanned => "scanned, no text",
        CandidateLeftOut::OverBudget => "over the limit",
        CandidateLeftOut::ExcludedByStudent => "you removed it",
    }
}

fn phase_word(phase: CoursePhase) -> &'static str {
    match phase {
        CoursePhase::NotStarted => "not started",
        CoursePhase::Teaching => "teaching",
        CoursePhase::Break => "break",
        CoursePhase::ExamPeriod => "exams",
        CoursePhase::Ended => "ended",
        CoursePhase::Unknown => "phase unknown",
    }
}

/// `pagelamp course remove <courses…> [--dry-run] [--now] [--keep-files] [--delete-backup]
/// [--yes]`: shows what goes, then asks (design §7.13); `--dry-run` only shows it, `--yes`
/// neither shows nor asks. Without a terminal and without `--yes`, nothing changes.
pub async fn remove(
    app: &App,
    courses: Vec<String>,
    dry_run: bool,
    options: RemoveOptions,
    yes: bool,
    json: bool,
) -> anyhow::Result<()> {
    if dry_run {
        let preview = app.removal_preview(courses)?;
        if json {
            return print_json(&preview);
        }
        print!("{}", removal_preview_text(&preview, &options));
        return Ok(());
    }
    let courses = if yes {
        courses
    } else {
        // Before the question, on stderr: stdout keeps only the result (`--json` too). An
        // unknown or already removed course fails here, before anything is asked.
        let preview = app.removal_preview(courses)?;
        eprint!("{}", removal_preview_text(&preview, &options));
        confirm(&removal_question(&preview, &options), false)?;
        // Exactly what was shown, by id: a course removed or pruned while the question waits
        // is then not found, and a code can't land on another course.
        preview
            .items
            .iter()
            .map(|item| item.course_id.clone())
            .collect()
    };
    let report = app.remove_courses(courses, options).await?;
    if json {
        return print_json(&report);
    }
    for removed in &report.removed {
        let label = removed.code.as_deref().unwrap_or(&removed.name);
        match removed.purge_in_days {
            Some(days) if !report.purged_now => println!(
                "Removed {label}. Its data is deleted in {days} days; undo with `{} course restore \"{}\"`.",
                pagelamp_core::brand::CLI_NAME,
                removed.removed_id
            ),
            _ => println!("Deleted {label}. Restoring it means syncing it again."),
        }
        if removed.files_pending {
            println!("  Its downloaded files couldn't be moved to the Trash; they are kept.");
        }
    }
    if report.backup_deleted {
        println!("The pre-update backup was deleted.");
    } else if report.backup_failed {
        println!("The pre-update backup couldn't be deleted; it is still in the data folder.");
    }
    Ok(())
}

/// What a removal takes and keeps, per course, as `--dry-run` shows it and as the question
/// before a removal follows (with the options already chosen).
fn removal_preview_text(preview: &RemovalPreview, options: &RemoveOptions) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for item in &preview.items {
        let label = item.code.as_deref().unwrap_or(&item.name);
        let _ = writeln!(
            text,
            "{label} — {} materials, {} deadlines",
            item.materials, item.deadlines
        );
        if item.downloaded_files > 0 {
            let (files, kb) = (item.downloaded_files, item.downloaded_bytes.div_ceil(1024));
            let _ = if options.keep_downloaded_files {
                writeln!(
                    text,
                    "  {files} downloaded files ({kb} KB) are kept (--keep-files)"
                )
            } else {
                writeln!(
                    text,
                    "  {files} downloaded files ({kb} KB) go to the Trash (--keep-files keeps them)"
                )
            };
        }
        if item.own_folder_untouched {
            text.push_str("  Your folder isn't changed; PageLamp stops reading it.\n");
        }
        if item.custom_settings {
            text.push_str("  Your settings for it are kept for a restore.\n");
        }
        if item.cannot_sync_again {
            text.push_str("  Canvas restricts this course: it can't be synced again.\n");
        }
    }
    if let Some(backup) = &preview.backup {
        let _ = writeln!(
            text,
            "The pre-update backup ({} days old) still holds their text{}.",
            backup.age_days,
            if options.delete_pre_update_backup && backup.delete_by_default {
                "; it is deleted with them (--delete-backup)"
            } else if options.delete_pre_update_backup {
                "; it is deleted with them (--delete-backup), though it is the way back if the \
                 update went wrong"
            } else if backup.delete_by_default {
                "; consider --delete-backup"
            } else {
                ": keep it until you know the update works"
            }
        );
    }
    text
}

/// The question after the preview: when the data goes, and whether it can be undone.
fn removal_question(preview: &RemovalPreview, options: &RemoveOptions) -> String {
    let labels: Vec<&str> = preview
        .items
        .iter()
        .map(|item| item.code.as_deref().unwrap_or(&item.name))
        .collect();
    let labels = labels.join(", ");
    if options.purge_now {
        format!("Remove {labels} and delete the local data now? This can't be undone.")
    } else {
        format!(
            "Remove {labels}? The local data is deleted in {} days; `{} course restore` undoes it until then.",
            pagelamp_core::removal::PURGE_AFTER_DAYS,
            pagelamp_core::brand::CLI_NAME
        )
    }
}

/// `pagelamp course removed`
pub fn removed(app: &App, json: bool) -> anyhow::Result<()> {
    let removed = app.removed_courses()?;
    if json {
        return print_json(&removed);
    }
    if removed.is_empty() {
        println!("No removed courses.");
    }
    for course in &removed {
        let label = course.code.as_deref().unwrap_or(&course.name);
        let state = match (course.state, course.purge_in_days) {
            (TombstoneState::Pending, Some(days)) => {
                format!("deleted in {days} days (undo possible)")
            }
            (TombstoneState::Restoring, _) => "being restored".to_string(),
            _ => "deleted; restoring means syncing again".to_string(),
        };
        let files = if course.files_pending {
            "; downloaded files wait for the Trash"
        } else {
            ""
        };
        println!("{label} — {state}{files}  [{}]", course.removed_id);
    }
    Ok(())
}

/// `pagelamp course restore <removed id>`
pub async fn restore(app: &App, removed_id: &str, json: bool) -> anyhow::Result<()> {
    let outcome = app.restore_course(removed_id).await?;
    if json {
        return print_json(&outcome);
    }
    match (outcome.restored, outcome.failure) {
        (true, _) => println!("Restored."),
        (false, Some(RestoreFailure::Offline)) => println!("Not restored: you seem to be offline."),
        (false, Some(RestoreFailure::AccessRestricted)) => {
            println!("Not restored: Canvas restricts access to this course.")
        }
        (false, Some(RestoreFailure::NotListed)) => {
            println!("Not restored: Canvas no longer lists this course.")
        }
        (false, _) => println!("Not restored: the sync didn't bring it back."),
    }
    Ok(())
}

/// `pagelamp course purge [<removed ids…>] [--permanent] [--yes]`: shows what goes, then asks;
/// `--yes` neither shows nor asks. Nothing to delete: nothing is asked. Without a terminal and
/// without `--yes`, nothing changes.
pub async fn purge(
    app: &App,
    removed_ids: Vec<String>,
    permanent: bool,
    yes: bool,
    json: bool,
) -> anyhow::Result<()> {
    let ids = (!removed_ids.is_empty()).then_some(removed_ids);
    let ids = if yes {
        ids
    } else {
        // The facade's own choice of what a purge does; an unknown id fails here.
        let targets = app.purge_targets(ids.as_deref())?;
        if !targets.courses.is_empty() {
            eprint!("{}", purge_preview_text(&targets, permanent));
            confirm(&purge_question(&targets, permanent), false)?;
        }
        // Exactly what was shown: a removal that falls due while the question waits isn't added.
        Some(
            targets
                .courses
                .iter()
                .map(|course| course.removed_id.clone())
                .collect(),
        )
    };
    let report = app.purge_removed_courses(ids, permanent).await?;
    if json {
        return print_json(&report);
    }
    println!("Deleted {} course(s).", report.purged.len());
    if !report.files_pending.is_empty() {
        println!(
            "{} course(s)' downloaded files couldn't be moved to the Trash and are kept (--permanent deletes them).",
            report.files_pending.len()
        );
    }
    if report.backup_deleted {
        println!("The pre-update backup was deleted.");
    } else if report.backup_failed {
        println!("The pre-update backup couldn't be deleted; it is still in the data folder.");
    }
    Ok(())
}

/// What a purge does, per removed course.
fn purge_preview_text(targets: &PurgeTargets, permanent: bool) -> String {
    use std::fmt::Write as _;
    let mut text = String::new();
    for course in &targets.courses {
        let label = course.code.as_deref().unwrap_or(&course.name);
        let what = match course.state {
            TombstoneState::Pending => "its local data is deleted now, and its undo ends",
            _ => "its downloaded files still on this computer go to the Trash",
        };
        let _ = writeln!(text, "{label} — {what}");
    }
    if targets.deletes_backup {
        text.push_str("The pre-update backup is deleted too (asked for with --delete-backup).\n");
    }
    if permanent {
        text.push_str(
            "Where the Trash can't take downloaded files, they are deleted permanently (--permanent).\n",
        );
    }
    text
}

/// The question after the purge preview.
fn purge_question(targets: &PurgeTargets, permanent: bool) -> String {
    let n = targets.courses.len();
    let deletes = targets
        .courses
        .iter()
        .any(|course| course.state == TombstoneState::Pending);
    if permanent || deletes || targets.deletes_backup {
        format!("Delete the data of {n} removed course(s) now? This can't be undone.")
    } else {
        format!("Move the downloaded files of {n} removed course(s) to the Trash?")
    }
}
