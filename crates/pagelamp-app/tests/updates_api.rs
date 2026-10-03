//! The update facade (v0.3 M0.4): prefs, the launch classification, What's new, update-check
//! records, and `activity()`. Temporary data dirs, in-memory secrets, synthetic data only.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeDelta, Utc};
use pagelamp_app::{
    ActivityKind, App, SyncEvent, SyncRequest, UpdateChannel, UpdateCheckOutcome,
    UpdateCheckRecord, UpdatePrefs, WhatsNewTopic,
};
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

#[test]
fn update_prefs_default_on_and_are_stored() {
    let temp = tempfile::tempdir().unwrap();
    let app = open(&data_dir(&temp));
    assert_eq!(app.update_prefs().unwrap(), UpdatePrefs::default());
    assert!(app.update_prefs().unwrap().auto_check);
    let default = if env!("CARGO_PKG_VERSION").contains('-') {
        UpdateChannel::Beta
    } else {
        UpdateChannel::Stable
    };
    assert_eq!(app.effective_update_channel().unwrap(), default);
    let prefs = UpdatePrefs {
        auto_check: false,
        channel: Some(UpdateChannel::Beta),
    };
    app.set_update_prefs(prefs.clone()).unwrap();
    assert_eq!(open(&data_dir(&temp)).update_prefs().unwrap(), prefs);
    assert_eq!(app.effective_update_channel().unwrap(), UpdateChannel::Beta);
}

#[test]
fn a_fresh_install_checks_daily_once_the_disclosure_was_seen() {
    let temp = tempfile::tempdir().unwrap();
    let app = open(&data_dir(&temp));
    let now = at("2026-10-01T09:00:00Z");
    let tasks = app.startup_tasks(now).unwrap();
    assert_eq!(tasks.whats_new, None, "nothing is new on a fresh install");
    assert_eq!(tasks.updated_from, None);
    assert!(
        !tasks.update_check_due,
        "not before the onboarding disclosure"
    );

    app.acknowledge_update_disclosure().unwrap();
    assert!(
        app.startup_tasks(now).unwrap().update_check_due,
        "seen at once"
    );
    app.record_update_check(UpdateCheckRecord {
        at: now,
        channel: UpdateChannel::Stable,
        outcome: UpdateCheckOutcome::UpToDate,
    })
    .unwrap();
    assert!(!app.startup_tasks(now).unwrap().update_check_due);
    // The same app, still running a day later (in the tray): due again.
    let later = now + TimeDelta::hours(25);
    assert!(app.startup_tasks(later).unwrap().update_check_due);

    // Turned off: never due.
    app.set_update_prefs(UpdatePrefs {
        auto_check: false,
        channel: None,
    })
    .unwrap();
    assert!(!app.startup_tasks(later).unwrap().update_check_due);
}

#[test]
fn a_new_user_who_adds_a_source_before_startup_tasks_sees_no_whats_new() {
    // The desktop's first-run screens add a source, then the shell asks for its startup tasks.
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let courses = temp.path().join("Courses");
    std::fs::create_dir_all(courses.join("DEMO101 Intro")).unwrap();
    std::fs::write(courses.join("DEMO101 Intro/notes.txt"), "demo").unwrap();
    let app = open(&data);
    app.add_folder_source(&courses, None, None).unwrap();
    let tasks = app.startup_tasks(at("2026-10-01T09:00:00Z")).unwrap();
    assert_eq!(tasks.whats_new, None, "a new user, not an upgrade");
    assert_eq!(tasks.updated_from, None);
    let acknowledged: Option<String> = Store::open(&data.join("pagelamp.db"))
        .unwrap()
        .setting("app.whats_new_acknowledged")
        .unwrap();
    assert_eq!(acknowledged.as_deref(), Some(env!("CARGO_PKG_VERSION")));
}

#[test]
fn the_same_version_again_shows_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    used_before(&data, &temp.path().join("Courses"));
    set_last_run(&data, env!("CARGO_PKG_VERSION"));
    let tasks = open(&data)
        .startup_tasks(at("2026-10-01T09:00:00Z"))
        .unwrap();
    assert_eq!((tasks.whats_new, tasks.updated_from), (None, None));
}

#[test]
fn an_upgrade_from_0_1_shows_whats_new_before_the_first_check() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    used_before(&data, &temp.path().join("Courses"));
    // 0.1 never recorded its version; a new process starts.
    Store::open(&data.join("pagelamp.db"))
        .unwrap()
        .remove_setting("app.last_run_version")
        .unwrap();
    let app = open(&data);
    let now = at("2026-10-01T09:00:00Z");
    let tasks = app.startup_tasks(now).unwrap();
    let whats_new = tasks.whats_new.expect("an upgrader sees What's new");
    assert_eq!(whats_new.since, None, "0.1 didn't record its version");
    assert_eq!(
        whats_new.topics,
        [
            WhatsNewTopic::UpdateCheck,
            WhatsNewTopic::CourseWeeks,
            WhatsNewTopic::CourseRemoval,
        ]
    );
    assert_eq!(tasks.updated_from, None);
    assert!(!tasks.update_check_due, "not while What's new waits");

    app.acknowledge_whats_new().unwrap();
    let tasks = app.startup_tasks(now).unwrap();
    assert_eq!(tasks.whats_new, None, "gone at once, in the same process");
    assert!(
        tasks.update_check_due,
        "the update-check topic counts as the disclosure"
    );

    // The next launch of the same version is an ordinary one.
    let next = open(&data).startup_tasks(now).unwrap();
    assert_eq!((next.whats_new, next.updated_from), (None, None));
}

#[test]
fn an_upgrade_between_versions_reports_the_old_one_for_the_whole_launch() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    used_before(&data, &temp.path().join("Courses"));
    set_last_run(&data, "0.0.9");
    let app = open(&data);
    let now = at("2026-10-01T09:00:00Z");
    for _ in 0..2 {
        let tasks = app.startup_tasks(now).unwrap();
        assert_eq!(tasks.updated_from.as_deref(), Some("0.0.9"));
        assert_eq!(
            tasks.whats_new.map(|w| w.since),
            Some(Some("0.0.9".to_string()))
        );
    }
    // A clone shares the launch; a new process doesn't see an update any more.
    let clone = app.clone();
    assert_eq!(
        clone.startup_tasks(now).unwrap().updated_from.as_deref(),
        Some("0.0.9")
    );
    assert_eq!(open(&data).startup_tasks(now).unwrap().updated_from, None);
}

#[test]
fn the_last_update_check_is_kept_and_reported_as_codes() {
    let temp = tempfile::tempdir().unwrap();
    let app = open(&data_dir(&temp));
    assert_eq!(app.last_update_check().unwrap(), None);
    let record = UpdateCheckRecord {
        at: at("2026-10-01T09:00:00Z"),
        channel: UpdateChannel::Beta,
        outcome: UpdateCheckOutcome::Error {
            code: "network".into(),
        },
    };
    app.record_update_check(record.clone()).unwrap();
    assert_eq!(app.last_update_check().unwrap(), Some(record));
    let report = app.diagnostic_report().unwrap();
    assert!(
        report.contains("- Last update check: 2026-10-01 09:00 UTC (beta): error network"),
        "{report}"
    );
}

#[tokio::test]
async fn activity_shows_a_running_sync_and_another_process_holding_the_lock() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let courses = temp.path().join("Courses");
    std::fs::create_dir_all(courses.join("DEMO101 Intro")).unwrap();
    std::fs::write(courses.join("DEMO101 Intro/notes.txt"), "demo").unwrap();
    let app = open(&data);
    let source = app.add_folder_source(&courses, None, None).unwrap();
    assert!(app.activity().items.is_empty());
    assert!(!app.activity().other_process_syncing);

    // Seen from inside the sync (its progress callback runs while it works).
    let seen = Mutex::new(Vec::new());
    let observer = app.clone();
    app.sync_source(&source.id, SyncRequest::default(), |event| {
        if let SyncEvent::SourceStarted { .. } = event {
            seen.lock().unwrap().push(observer.activity());
        }
    })
    .await
    .unwrap();
    let during = seen.lock().unwrap().pop().expect("observed");
    assert_eq!(during.items.len(), 1);
    assert_eq!(during.items[0].kind, ActivityKind::Sync);
    assert_eq!(
        during.items[0].source_id.as_deref(),
        Some(source.id.as_str())
    );
    assert!(!during.other_process_syncing, "the lock is ours");
    assert!(app.activity().items.is_empty(), "deregistered afterwards");

    // Another process holds sync.lock.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data.join("sync.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(app.activity().other_process_syncing);
}

#[test]
fn a_failed_read_of_the_update_settings_is_an_error_never_the_defaults() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    app.set_update_prefs(UpdatePrefs {
        auto_check: false,
        channel: None,
    })
    .unwrap();
    let raw = rusqlite::Connection::open(data.join("pagelamp.db")).unwrap();

    // The table can't be read (busy past the timeout, damaged, …): the student's "off" must
    // never come back as the default "on".
    raw.execute_batch("ALTER TABLE settings RENAME TO settings_away")
        .unwrap();
    assert!(app.update_prefs().is_err());
    assert!(app.startup_tasks(at("2026-10-01T09:00:00Z")).is_err());
    assert!(app.last_update_check().is_err());
    raw.execute_batch("ALTER TABLE settings_away RENAME TO settings")
        .unwrap();
    assert!(!app.update_prefs().unwrap().auto_check);

    // Another version's shape: the defaults, as before.
    raw.execute(
        "UPDATE settings SET value = '{\"auto_check\": \"sometimes\"}' WHERE key = 'updates.prefs'",
        [],
    )
    .unwrap();
    assert_eq!(app.update_prefs().unwrap(), UpdatePrefs::default());
}
