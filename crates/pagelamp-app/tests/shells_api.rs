//! Per shell: the desktop app and the Mac app share one data folder. Each has its own What's
//! new (only the desktop app's counts as the update disclosure) and its own "Remind me" answer
//! (`ReminderSettings::run_in_background`: the desktop's tray and login item, the Mac app's
//! notification consent); the other reminder settings are shared.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pagelamp_app::{App, ReminderSettings, Shell, WhatsNewTopic};
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_core::store::Store;

fn open(data: &Path) -> App {
    App::open_at_with_secrets(data.to_path_buf(), Arc::new(MemorySecrets::new())).unwrap()
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

fn data_dir(temp: &tempfile::TempDir) -> PathBuf {
    temp.path().join("data")
}

/// A course folder the app has synced before: what an existing (0.1) user has.
fn used_before(data: &Path, courses: &Path) {
    std::fs::create_dir_all(courses.join("DEMO101 Intro")).unwrap();
    std::fs::write(courses.join("DEMO101 Intro/notes.txt"), "demo").unwrap();
    open(data).add_folder_source(courses, None, None).unwrap();
}

fn set_last_run(data: &Path, version: &str) {
    Store::open(&data.join("pagelamp.db"))
        .unwrap()
        .set_setting("app.last_run_version", &version)
        .unwrap();
}

/// The Mac app on the same data folder (what the FFI opens).
fn open_mac(data: &Path) -> App {
    let app = open(data);
    app.set_shell(Shell::Mac);
    app
}

#[test]
fn the_mac_app_asking_first_leaves_the_desktop_app_its_whats_new_and_disclosure() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    used_before(&data, &temp.path().join("Courses"));
    let now = at("2026-10-01T09:00:00Z");

    // An update from 0.1 (no version recorded); the Mac app launches first: its own first run,
    // with nothing to show, although the folder has data.
    let mac = open_mac(&data);
    let tasks = mac.startup_tasks(now).unwrap();
    assert_eq!((tasks.whats_new, tasks.updated_from), (None, None));

    // The desktop app still gets its update from 0.1, the update-check topic included, and
    // closing it is the update disclosure.
    let desktop = open(&data);
    let whats_new = desktop
        .startup_tasks(now)
        .unwrap()
        .whats_new
        .expect("the desktop app's What's new");
    assert!(whats_new.topics.contains(&WhatsNewTopic::UpdateCheck));
    desktop.acknowledge_whats_new().unwrap();
    assert!(desktop.startup_tasks(now).unwrap().update_check_due);
}

#[test]
fn each_shell_acknowledges_only_its_own_whats_new() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    used_before(&data, &temp.path().join("Courses"));
    set_last_run(&data, "0.0.9");
    Store::open(&data.join("pagelamp.db"))
        .unwrap()
        .set_setting("app.mac.last_run_version", &"0.0.9")
        .unwrap();
    let now = at("2026-10-01T09:00:00Z");

    // The Mac app: every topic since 0.0.9 but the update check (it updates with Sparkle).
    let mac = open_mac(&data);
    let topics = mac
        .startup_tasks(now)
        .unwrap()
        .whats_new
        .expect("the Mac app's What's new")
        .topics;
    assert!(!topics.contains(&WhatsNewTopic::UpdateCheck), "{topics:?}");
    assert!(topics.contains(&WhatsNewTopic::CourseWeeks), "{topics:?}");
    mac.acknowledge_whats_new().unwrap();
    assert!(mac.startup_tasks(now).unwrap().whats_new.is_none());
    let disclosed: Option<bool> = Store::open(&data.join("pagelamp.db"))
        .unwrap()
        .setting("updates.disclosure_acknowledged")
        .unwrap();
    assert_eq!(
        disclosed, None,
        "the Mac app's sheet isn't the update disclosure"
    );

    // The desktop app's is still waiting, update check included.
    let desktop = open(&data);
    let tasks = desktop.startup_tasks(now).unwrap();
    assert_eq!(tasks.updated_from.as_deref(), Some("0.0.9"));
    assert!(
        tasks
            .whats_new
            .is_some_and(|w| w.topics.contains(&WhatsNewTopic::UpdateCheck))
    );
    assert!(!tasks.update_check_due);
    desktop.acknowledge_whats_new().unwrap();
    assert!(desktop.startup_tasks(now).unwrap().update_check_due);

    // Both launched this version: a new launch of either shows nothing.
    assert!(
        open_mac(&data)
            .startup_tasks(now)
            .unwrap()
            .whats_new
            .is_none()
    );
    assert!(open(&data).startup_tasks(now).unwrap().whats_new.is_none());
}

/// Consent given in the Mac app never gives the desktop app a tray and a login item, and the
/// other way round; every other reminder setting is shared.
#[test]
fn each_shell_has_its_own_remind_me_answer() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let desktop = open(&data);
    let mac = open_mac(&data);
    assert!(!desktop.reminder_settings().unwrap().run_in_background);
    assert!(!mac.reminder_settings().unwrap().run_in_background);

    // The Mac app says "Remind me" and changes a shared field.
    mac.set_reminder_settings(&ReminderSettings {
        run_in_background: true,
        digest_time: "18:30".into(),
        ..ReminderSettings::default()
    })
    .unwrap();
    let seen = desktop.reminder_settings().unwrap();
    assert!(
        !seen.run_in_background,
        "no tray or login item on the desktop"
    );
    assert_eq!(seen.digest_time, "18:30", "shared");
    assert!(mac.reminder_settings().unwrap().run_in_background);

    // The desktop app says "Remind me" too, then turns it off: the Mac app's answer stays.
    desktop
        .set_reminder_settings(&ReminderSettings {
            run_in_background: true,
            plan_today: true,
            ..seen.clone()
        })
        .unwrap();
    assert!(desktop.reminder_settings().unwrap().run_in_background);
    let mac_seen = mac.reminder_settings().unwrap();
    assert!(mac_seen.run_in_background && mac_seen.plan_today);
    desktop
        .set_reminder_settings(&ReminderSettings {
            run_in_background: false,
            ..desktop.reminder_settings().unwrap()
        })
        .unwrap();
    assert!(mac.reminder_settings().unwrap().run_in_background);

    // The Mac app turns its consent off: the desktop's answer is untouched by that too.
    desktop
        .set_reminder_settings(&ReminderSettings {
            run_in_background: true,
            ..desktop.reminder_settings().unwrap()
        })
        .unwrap();
    mac.set_reminder_settings(&ReminderSettings {
        run_in_background: false,
        ..mac.reminder_settings().unwrap()
    })
    .unwrap();
    assert!(desktop.reminder_settings().unwrap().run_in_background);
    assert!(!mac.reminder_settings().unwrap().run_in_background);
    let (d, m) = (
        desktop.reminder_settings().unwrap(),
        mac.reminder_settings().unwrap(),
    );
    assert_eq!(
        ReminderSettings {
            run_in_background: false,
            ..d
        },
        m,
        "every other field is shared"
    );
}
