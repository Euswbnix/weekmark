//! Reminders and the weekly digest through the facade (model-access design §5.3): wall-clock
//! times across a DST change, catch-up, dedupe, maximum age, which deadlines remind, today's
//! plan, settings and the launch tasks. Synthetic data only; the zone is fixed to Toronto.

use std::sync::Arc;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use pagelamp_app::{App, AppErrorKind, ReminderKind, ReminderSettings, RemoveOptions};
use pagelamp_core::model::*;
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_core::store::Store;
use pagelamp_core::term::CoursePhase;
use serde_json::json;

const SOURCE: &str = "canvas:lms.example.edu";

fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
}

fn app(temp: &tempfile::TempDir) -> App {
    let app = App::open_at_with_secrets(temp.path().join("data"), Arc::new(MemorySecrets::new()))
        .unwrap();
    app.set_time_zone(Some("America/Toronto")).unwrap();
    let store = Store::open(&app.db_path()).unwrap();
    store
        .upsert_source(&SourceRecord {
            id: SOURCE.into(),
            kind: SourceKind::Canvas,
            label: "Demo LMS".into(),
            config: json!({ "base_url": "https://lms.example.edu" }),
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })
        .unwrap();
    app
}

/// A course of `SOURCE` whose term runs from `start` to `end`.
fn course(app: &App, external: &str, start: NaiveDate, end: NaiveDate) -> String {
    let id = format!("{SOURCE}/course/{external}");
    Store::open(&app.db_path())
        .unwrap()
        .upsert_course(&CourseUpsert {
            id: id.clone(),
            source_id: SOURCE.into(),
            external_id: external.into(),
            code: Some(format!("DEMO{external}")),
            name: format!("Demo course {external}"),
            term_start: Some(start),
            term_end: Some(end),
            url: None,
            syllabus_text: None,
            lms: Default::default(),
        })
        .unwrap();
    id
}

fn fall_course(app: &App, external: &str) -> String {
    course(
        app,
        external,
        NaiveDate::from_ymd_opt(2026, 9, 8).unwrap(),
        NaiveDate::from_ymd_opt(2026, 12, 18).unwrap(),
    )
}

fn deadline(id: &str, course_id: &str, kind: EventKind, due: DateTime<Utc>) -> Event {
    Event {
        id: format!("{SOURCE}/assignment/{id}"),
        source_id: SOURCE.into(),
        course_id: Some(course_id.into()),
        kind,
        title: format!("Essay {id}"),
        starts_at: None,
        ends_at: None,
        due_at: Some(due),
        url: None,
        updated_at: Utc::now(),
        course_hint: None,
    }
}

fn set_events(app: &App, events: &[Event]) {
    Store::open(&app.db_path())
        .unwrap()
        .replace_events(SOURCE, events)
        .unwrap();
}

fn kinds(reminders: &[pagelamp_app::Reminder]) -> Vec<(ReminderKind, String)> {
    reminders
        .iter()
        .map(|r| (r.kind, r.local_time.clone()))
        .collect()
}

#[test]
fn a_monday_digest_stays_at_nine_across_the_end_of_dst() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    fall_course(&app, "101");
    // Canada leaves DST on Sunday 2026-11-01.
    let week = app
        .reminders(utc(2026, 10, 25, 0, 0), utc(2026, 11, 8, 0, 0))
        .unwrap();
    let digests: Vec<_> = week
        .iter()
        .filter(|r| r.kind == ReminderKind::WeeklyDigest)
        .collect();
    assert_eq!(
        digests
            .iter()
            .map(|r| (r.local_time.as_str(), r.fire_at, r.time_zone.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                "2026-10-26T09:00",
                utc(2026, 10, 26, 13, 0),
                "America/Toronto"
            ),
            (
                "2026-11-02T09:00",
                utc(2026, 11, 2, 14, 0),
                "America/Toronto"
            ),
        ]
    );
    assert_eq!(digests[1].id, "weekly_digest:2026-11-02");
}

#[test]
fn a_missed_digest_catches_up_once_and_goes_stale() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    let course = fall_course(&app, "101");
    set_events(
        &app,
        &[deadline(
            "1",
            &course,
            EventKind::AssignmentDue,
            utc(2026, 11, 6, 17, 0),
        )],
    );
    // PageLamp wasn't running on Monday morning: the digest shows on Tuesday.
    let tuesday = app.due_reminders(utc(2026, 11, 3, 15, 0)).unwrap();
    let digest: Vec<_> = tuesday
        .iter()
        .filter(|r| r.kind == ReminderKind::WeeklyDigest)
        .collect();
    assert_eq!(digest.len(), 1);
    assert_eq!(digest[0].count, Some(1), "one deadline this week");
    app.mark_reminders_shown(&[digest[0].id.clone()]).unwrap();
    let again = app.due_reminders(utc(2026, 11, 3, 16, 0)).unwrap();
    assert!(
        again.iter().all(|r| r.kind != ReminderKind::WeeklyDigest),
        "shown once"
    );
    // The next Monday's, opened 3 days and an hour late: too old.
    let late = app.due_reminders(utc(2026, 11, 12, 15, 0)).unwrap();
    assert!(
        late.iter().all(|r| r.kind != ReminderKind::WeeklyDigest),
        "{late:?}"
    );
}

#[test]
fn a_deadline_reminds_48_and_24_hours_ahead_and_never_after() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    app.set_reminder_settings(&ReminderSettings {
        weekly_digest: false,
        ..ReminderSettings::default()
    })
    .unwrap();
    let course = fall_course(&app, "101");
    let due = utc(2026, 11, 10, 17, 0);
    set_events(
        &app,
        &[deadline("1", &course, EventKind::AssignmentDue, due)],
    );

    let early = app.due_reminders(due - Duration::hours(47)).unwrap();
    assert_eq!(early.len(), 1);
    let first = &early[0];
    assert_eq!(
        (
            first.kind,
            first.hours_before,
            first.due_at,
            first.local_time.as_str()
        ),
        (
            ReminderKind::DeadlineSoon,
            Some(48),
            Some(due),
            "2026-11-08T12:00"
        )
    );
    assert_eq!(
        (
            first.title.as_deref(),
            first.course_code.as_deref(),
            first.course_name.as_deref()
        ),
        (Some("Essay 1"), Some("DEMO101"), Some("Demo course 101"))
    );
    // Not shown in time: once the 24 h reminder fires, only that one is due.
    let later = app.due_reminders(due - Duration::hours(23)).unwrap();
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].hours_before, Some(24));
    app.mark_reminders_shown(&[later[0].id.clone()]).unwrap();
    assert!(
        app.due_reminders(due - Duration::hours(1))
            .unwrap()
            .is_empty()
    );
    // Due: nothing is shown any more, even what wasn't.
    app.mark_reminders_shown(&[]).unwrap();
    let past = app.due_reminders(due + Duration::hours(1)).unwrap();
    assert!(past.is_empty(), "{past:?}");
}

#[test]
fn deadlines_remind_whatever_the_lifecycle_but_not_for_hidden_or_removed_courses() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    app.set_reminder_settings(&ReminderSettings {
        weekly_digest: false,
        ..ReminderSettings::default()
    })
    .unwrap();
    let current = fall_course(&app, "101");
    // Ended in April: a late deadline still reminds.
    let ended = course(
        &app,
        "202",
        NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
        NaiveDate::from_ymd_opt(2026, 4, 30).unwrap(),
    );
    let hidden = fall_course(&app, "303");
    let removed = fall_course(&app, "404");
    Store::open(&app.db_path())
        .unwrap()
        .set_course_hidden(&hidden, true)
        .unwrap();
    let due = utc(2026, 11, 10, 17, 0);
    set_events(
        &app,
        &[
            deadline("1", &current, EventKind::AssignmentDue, due),
            deadline("2", &ended, EventKind::QuizDue, due),
            deadline("3", &hidden, EventKind::AssignmentDue, due),
            deadline("4", &removed, EventKind::AssignmentDue, due),
            // A lecture is not a deadline.
            deadline("5", &current, EventKind::ClassEvent, due),
        ],
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(app.remove_courses(
        vec!["DEMO404".into()],
        RemoveOptions {
            reason: None,
            keep_downloaded_files: false,
            purge_now: false,
            delete_pre_update_backup: false,
        },
    ))
    .unwrap();
    let due_now = app.due_reminders(due - Duration::hours(47)).unwrap();
    let mut titles: Vec<&str> = due_now.iter().filter_map(|r| r.title.as_deref()).collect();
    titles.sort_unstable();
    assert_eq!(titles, ["Essay 1", "Essay 2"]);
}

#[test]
fn today_s_plan_reminds_until_midnight_when_turned_on() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    let course = fall_course(&app, "101");
    let hidden = fall_course(&app, "303");
    let day = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
    let item = |title: &str, done: bool| StudyPlanItem {
        date: day,
        course_id: Some(course.clone()),
        title: title.into(),
        description: None,
        material_ids: Vec::new(),
        minutes: Some(30),
        done,
    };
    Store::open(&app.db_path())
        .unwrap()
        .save_study_plan(&StudyPlan {
            horizon_start: day,
            horizon_end: day + Duration::days(6),
            items: vec![
                item("Read ch. 3", false),
                item("Problems", false),
                item("Done", true),
                // A hidden course's item doesn't count.
                StudyPlanItem {
                    course_id: Some(hidden.clone()),
                    ..item("Hidden course reading", false)
                },
            ],
            notes: None,
        })
        .unwrap();
    Store::open(&app.db_path())
        .unwrap()
        .set_course_hidden(&hidden, true)
        .unwrap();
    let plan_reminders = |now| -> Vec<pagelamp_app::Reminder> {
        app.due_reminders(now)
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == ReminderKind::PlanToday)
            .collect()
    };
    assert!(
        plan_reminders(utc(2026, 11, 4, 20, 0)).is_empty(),
        "off by default"
    );
    app.set_reminder_settings(&ReminderSettings {
        plan_today: true,
        plan_today_time: "07:30".into(),
        ..ReminderSettings::default()
    })
    .unwrap();
    let today = plan_reminders(utc(2026, 11, 4, 20, 0));
    assert_eq!(
        kinds(&today),
        [(ReminderKind::PlanToday, "2026-11-04T07:30".to_string())]
    );
    assert_eq!(today[0].count, Some(2));
    // Past local midnight (05:00 UTC in November): gone.
    assert!(plan_reminders(utc(2026, 11, 5, 5, 30)).is_empty());
}

#[test]
fn settings_are_checked_and_ids_must_be_reminders() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    assert_eq!(
        app.reminder_settings().unwrap(),
        ReminderSettings::default()
    );
    let err = app
        .set_reminder_settings(&ReminderSettings {
            digest_time: "9:00".into(),
            ..ReminderSettings::default()
        })
        .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::Invalid);
    let chosen = ReminderSettings {
        digest_day: pagelamp_app::DayOfWeek::Sunday,
        digest_time: "18:30".into(),
        run_in_background: true,
        ..ReminderSettings::default()
    };
    app.set_reminder_settings(&chosen).unwrap();
    assert_eq!(app.reminder_settings().unwrap(), chosen);
    // The question (b) notice is not a reminder: it can't be marked from here.
    let err = app
        .mark_reminders_shown(&["material_sharing:x".to_string()])
        .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::Invalid);
    let err = app
        .reminders(utc(2026, 11, 1, 0, 0), utc(2027, 1, 15, 0, 0))
        .unwrap_err();
    assert_eq!(err.kind, AppErrorKind::Invalid, "at most 62 days");
}

/// A failed read of the reminder settings is an error, never the defaults: the student's
/// "off" never turns into reminders, and the shells leave the login item and notifications
/// as they are until the stored answer can be read. Another version's shape is the defaults.
#[test]
fn a_failed_read_of_the_reminder_settings_is_an_error_never_the_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    fall_course(&app, "101");
    let chosen = ReminderSettings {
        weekly_digest: false,
        run_in_background: true,
        ..ReminderSettings::default()
    };
    app.set_reminder_settings(&chosen).unwrap();
    let raw = rusqlite::Connection::open(app.db_path()).unwrap();
    let monday = utc(2026, 11, 2, 15, 0);

    // The value can't be read (here it is stored as a blob).
    raw.execute(
        "UPDATE settings SET value = CAST(value AS BLOB) WHERE key = 'reminder_settings'",
        [],
    )
    .unwrap();
    assert!(app.reminder_settings().is_err());
    assert!(app.due_reminders(monday).is_err());
    assert!(app.reminders(monday, monday + Duration::days(7)).is_err());
    assert!(app.startup_tasks(monday).unwrap().due_reminders.is_empty());
    raw.execute(
        "UPDATE settings SET value = CAST(value AS TEXT) WHERE key = 'reminder_settings'",
        [],
    )
    .unwrap();
    assert_eq!(app.reminder_settings().unwrap(), chosen);
    assert!(app.due_reminders(monday).unwrap().is_empty(), "digest off");

    raw.execute(
        "UPDATE settings SET value = '{\"weekly_digest\": \"sometimes\"}' WHERE key = 'reminder_settings'",
        [],
    )
    .unwrap();
    assert_eq!(
        app.reminder_settings().unwrap(),
        ReminderSettings::default()
    );
}

#[test]
fn launch_tasks_carry_due_reminders_and_a_due_purge() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    fall_course(&app, "101");
    fall_course(&app, "202");
    let monday = utc(2026, 11, 2, 15, 0);
    let tasks = app.startup_tasks(monday).unwrap();
    assert_eq!(
        kinds(&tasks.due_reminders),
        [(ReminderKind::WeeklyDigest, "2026-11-02T09:00".to_string())]
    );
    assert!(!tasks.purge_due);
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(app.remove_courses(
        vec!["DEMO202".into()],
        RemoveOptions {
            reason: None,
            keep_downloaded_files: false,
            purge_now: false,
            delete_pre_update_backup: false,
        },
    ))
    .unwrap();
    // The removal is timed by the real clock: due 7 days from now.
    assert!(
        !app.startup_tasks(Utc::now()).unwrap().purge_due,
        "not due for 7 days"
    );
    let later = Utc::now() + Duration::days(8);
    let tasks = app.startup_tasks(later).unwrap();
    assert!(tasks.purge_due);
    assert_eq!(tasks.removed_files_waiting, 0);
}

#[test]
fn the_digest_lists_active_courses_by_week_and_others_only_for_deadlines() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    let today = Utc::now().date_naive();
    let current = course(
        &app,
        "101",
        today - Duration::days(20),
        today + Duration::days(60),
    );
    // Starts in 30 days (not active yet), with a deadline before that.
    let upcoming_with_deadline = course(
        &app,
        "202",
        today + Duration::days(30),
        today + Duration::days(130),
    );
    // Ended, nothing due: left out.
    course(
        &app,
        "303",
        today - Duration::days(200),
        today - Duration::days(90),
    );
    set_events(
        &app,
        &[
            deadline(
                "1",
                &current,
                EventKind::AssignmentDue,
                Utc::now() + Duration::days(2),
            ),
            deadline(
                "2",
                &upcoming_with_deadline,
                EventKind::AssignmentDue,
                Utc::now() + Duration::days(3),
            ),
        ],
    );
    let digest = app.weekly_digest().unwrap();
    let rows: Vec<(Option<&str>, bool, bool, usize)> = digest
        .courses
        .iter()
        .map(|c| {
            (
                c.code.as_deref(),
                c.active,
                c.week.is_some(),
                c.deadlines.len(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (Some("DEMO101"), true, true, 1),
            (Some("DEMO202"), false, false, 1),
        ]
    );
    // The phase line: codes the UIs word ("Week 3", "Reading week — catch up").
    assert_eq!(
        (digest.courses[0].phase, digest.courses[1].phase),
        (CoursePhase::Teaching, CoursePhase::NotStarted)
    );
}

/// Policy golden: a prohibited or turned-off course's material text never reaches the digest
/// or a reminder (they carry titles and codes only).
#[test]
fn no_material_text_reaches_the_digest_or_reminders() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    let today = Utc::now().date_naive();
    let store = Store::open(&app.db_path()).unwrap();
    for (external, policy, access) in [
        ("101", AiPolicy::Prohibited, true),
        ("202", AiPolicy::Unrestricted, false),
    ] {
        let id = course(
            &app,
            external,
            today - Duration::days(20),
            today + Duration::days(60),
        );
        store.set_course_policy(&id, policy, None).unwrap();
        store.set_course_ai_access(&id, access).unwrap();
        let material = format!("{id}/file/1");
        store
            .upsert_material(&MaterialUpsert {
                id: material.clone(),
                course_id: id.clone(),
                module_id: None,
                kind: MaterialKind::File,
                title: "Week notes".into(),
                url: None,
                local_path: None,
                mime: None,
                published_at: None,
                week_hint: None,
            })
            .unwrap();
        // Readable text (text only belongs to an `ok` material): still never in the digest.
        store
            .set_text_state(&material, TextStatus::Ok, None, Some("h"))
            .unwrap();
        store
            .replace_chunks(
                &material,
                &[Chunk {
                    material_id: material.clone(),
                    ord: 0,
                    locator: None,
                    text: "Stomata secret body text".into(),
                }],
            )
            .unwrap();
    }
    let digest = serde_json::to_string(&app.weekly_digest().unwrap()).unwrap();
    let reminders = serde_json::to_string(
        &app.reminders(Utc::now(), Utc::now() + Duration::days(14))
            .unwrap(),
    )
    .unwrap();
    for text in [digest, reminders] {
        assert!(!text.contains("Stomata"), "{text}");
    }
}

/// A current course whose syllabus PageLamp could read (an offer, D47).
fn with_syllabus(app: &App, course_id: &str) {
    let store = Store::open(&app.db_path()).unwrap();
    let id = format!("{course_id}/syllabus");
    store
        .upsert_material(&MaterialUpsert {
            id: id.clone(),
            course_id: course_id.into(),
            module_id: None,
            kind: MaterialKind::Syllabus,
            title: "Syllabus".into(),
            url: None,
            local_path: None,
            mime: None,
            published_at: None,
            week_hint: None,
        })
        .unwrap();
    store
        .set_text_state(&id, TextStatus::Ok, None, Some("h"))
        .unwrap();
    store
        .replace_chunks(
            &id,
            &[Chunk {
                material_id: id.clone(),
                ord: 0,
                locator: None,
                text: "Classes begin in September. ".repeat(12),
            }],
        )
        .unwrap();
}

/// Launch tasks offer syllabus readings and list finished courses: at most 20 of each with
/// the totals, and silent while each list's "Not now" covers it (M3, calendar design §4).
#[test]
fn launch_tasks_list_offers_and_suggestions_until_not_now() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    let today = Utc::now().date_naive();
    let current = course(
        &app,
        "101",
        today - Duration::days(20),
        today + Duration::days(60),
    );
    with_syllabus(&app, &current);
    for n in 0..25 {
        course(
            &app,
            &format!("9{n:02}"),
            today - Duration::days(400),
            today - Duration::days(300),
        );
    }
    let tasks = app.startup_tasks(Utc::now()).unwrap();
    assert_eq!(
        tasks
            .calendar_offers
            .iter()
            .map(|o| o.course_id.as_str())
            .collect::<Vec<_>>(),
        [current.as_str()]
    );
    assert_eq!(tasks.calendar_offers_total, 1);
    assert_eq!(
        (
            tasks.removal_suggestions.len(),
            tasks.removal_suggestions_total
        ),
        (20, 25)
    );

    app.snooze_calendar_offers().unwrap();
    app.snooze_lifecycle_banner().unwrap();
    let quiet = app.startup_tasks(Utc::now()).unwrap();
    assert!(quiet.calendar_offers.is_empty() && quiet.removal_suggestions.is_empty());
    // The Courses page's card reads the same offers.
    assert!(app.syllabus_reading_offers().unwrap().is_empty());

    // A course offered later brings the offers back, all of them.
    let another = course(
        &app,
        "102",
        today - Duration::days(20),
        today + Duration::days(60),
    );
    with_syllabus(&app, &another);
    let back = app.startup_tasks(Utc::now()).unwrap();
    assert_eq!(back.calendar_offers_total, 2);
    assert_eq!(app.syllabus_reading_offers().unwrap().len(), 2);
    assert!(back.removal_suggestions.is_empty(), "still snoozed");
}

/// A failed read of the offers' "Not now" never brings the offers back: the Courses page's
/// card fails and the launch tasks show none. Another version's shape counts as no "Not now".
#[test]
fn a_failed_read_of_not_now_never_brings_the_offers_back() {
    let temp = tempfile::tempdir().unwrap();
    let app = app(&temp);
    let today = Utc::now().date_naive();
    let current = course(
        &app,
        "101",
        today - Duration::days(20),
        today + Duration::days(60),
    );
    with_syllabus(&app, &current);
    app.snooze_calendar_offers().unwrap();
    let raw = rusqlite::Connection::open(app.db_path()).unwrap();

    raw.execute(
        "UPDATE settings SET value = CAST(value AS BLOB) WHERE key = 'calendar.offers_snoozed'",
        [],
    )
    .unwrap();
    assert!(app.syllabus_reading_offers().is_err());
    let tasks = app.startup_tasks(Utc::now()).unwrap();
    assert!(tasks.calendar_offers.is_empty());
    raw.execute(
        "UPDATE settings SET value = CAST(value AS TEXT) WHERE key = 'calendar.offers_snoozed'",
        [],
    )
    .unwrap();
    assert!(app.syllabus_reading_offers().unwrap().is_empty());

    raw.execute(
        "UPDATE settings SET value = '[3]' WHERE key = 'calendar.offers_snoozed'",
        [],
    )
    .unwrap();
    assert_eq!(app.syllabus_reading_offers().unwrap().len(), 1);
}
