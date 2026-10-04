//! Automatic sync through the facade: the setting, `StartupTasks.sync_due`, and what a run
//! PageLamp starts by itself does differently (`SyncRequest.automatic`). Temporary data dirs,
//! in-memory secrets, a local mock server for the calendar feed; synthetic data only.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, TimeDelta, Utc};
use pagelamp_app::{
    App, AppErrorKind, AutoSync, AutoSyncTrigger, SyncDue, SyncEvent, SyncPrefs, SyncRequest,
};
use pagelamp_core::auto_sync::{
    AUTO_SYNC_ATTEMPTS_KEY, AutoSyncAttempts, LIGHT_SYNC_KEY, LightSync,
};
use pagelamp_core::model::SourceErrorKind;
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_core::store::Store;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn open(data: &Path) -> App {
    App::open_at_with_secrets(data.to_path_buf(), Arc::new(MemorySecrets::new())).unwrap()
}

fn data_dir(temp: &tempfile::TempDir) -> PathBuf {
    temp.path().join("data")
}

fn store(data: &Path) -> Store {
    Store::open(&data.join("pagelamp.db")).unwrap()
}

/// `<dir>/<name>/DEMO101 Intro/notes.txt`; returns the folder to add as a source.
fn course_folder(dir: &Path, name: &str) -> PathBuf {
    let root = dir.join(name);
    std::fs::create_dir_all(root.join("DEMO101 Intro")).unwrap();
    std::fs::write(root.join("DEMO101 Intro/notes.txt"), "demo").unwrap();
    root
}

/// The source's last successful sync was `hours` ago.
fn synced_hours_ago(data: &Path, source_id: &str, hours: i64) {
    store(data)
        .record_sync(source_id, Utc::now() - TimeDelta::hours(hours), None)
        .unwrap();
}

fn attempts(data: &Path) -> AutoSyncAttempts {
    store(data)
        .setting_or_absent(AUTO_SYNC_ATTEMPTS_KEY)
        .unwrap()
        .unwrap_or_default()
}

fn set(app: &App, auto_sync: AutoSync) {
    app.set_sync_prefs(SyncPrefs { auto_sync }).unwrap();
}

fn automatic() -> SyncRequest {
    SyncRequest {
        automatic: Some(AutoSyncTrigger::Unattended),
        ..SyncRequest::default()
    }
}

/// A calendar feed with one event, answering at `/feed.ics`.
const FEED: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Demo//EN\r\nBEGIN:VEVENT\r\n\
                    UID:ps1\r\nDTSTART:20300930T035900Z\r\nDTEND:20300930T035900Z\r\n\
                    SUMMARY:Problem Set 1\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

/// Whether an automatic sync is due from the timer at `at`. (For folders and calendar feeds,
/// which every run reads in full, the student's return answers the same until an attempt
/// didn't finish: each trigger waits after its own attempts only.)
fn due(app: &App, at: DateTime<Utc>) -> bool {
    app.startup_tasks(at).unwrap().sync_due.unattended
}

#[test]
fn the_setting_is_twice_a_day_until_chosen_and_a_failed_read_is_never_the_default() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    assert_eq!(app.sync_prefs().unwrap().auto_sync, AutoSync::TwiceDaily);
    assert_eq!(app.status().unwrap().auto_sync, AutoSync::TwiceDaily);
    for setting in [AutoSync::Off, AutoSync::Daily, AutoSync::TwiceDaily] {
        set(&app, setting);
        assert_eq!(open(&data).sync_prefs().unwrap().auto_sync, setting);
        assert_eq!(app.status().unwrap().auto_sync, setting);
    }
    // A value another version wrote reads as the default; a table that can't be read is an
    // error: the student's "off" must never come back as "on".
    store(&data)
        .set_setting("sync.prefs", &serde_json::json!({ "auto_sync": "hourly" }))
        .unwrap();
    assert_eq!(app.sync_prefs().unwrap(), SyncPrefs::default());
    set(&app, AutoSync::Off);
    let raw = rusqlite::Connection::open(data.join("pagelamp.db")).unwrap();
    raw.execute_batch("ALTER TABLE settings RENAME TO settings_away")
        .unwrap();
    assert!(app.sync_prefs().is_err());
    assert!(app.startup_tasks(Utc::now()).is_err());
    assert!(app.status().is_err());
}

#[test]
fn sync_due_follows_the_setting_the_clock_and_a_running_sync() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    let now = Utc::now();
    assert!(!due(&app, now), "no sources");

    let source = app
        .add_folder_source(&course_folder(temp.path(), "Courses"), None, None)
        .unwrap();
    assert!(due(&app, now), "never synced");
    synced_hours_ago(&data, &source.id, 11);
    assert!(!due(&app, now), "11 h old, twice a day");
    assert!(due(&app, now + TimeDelta::hours(1)), "12 h old");
    set(&app, AutoSync::Daily);
    assert!(
        !due(&app, now + TimeDelta::hours(12)),
        "23 h old, once a day"
    );
    assert!(due(&app, now + TimeDelta::hours(13)), "24 h old");
    set(&app, AutoSync::Off);
    assert!(!due(&app, now + TimeDelta::days(30)), "off");

    // Nothing while a sync runs, in this process or another (`sync.lock`).
    set(&app, AutoSync::TwiceDaily);
    synced_hours_ago(&data, &source.id, 13);
    assert!(due(&app, now));
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data.join("sync.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    assert!(!due(&app, now), "a sync is running");
    drop(lock);
    assert!(due(&app, now));
}

/// An upgrader reads What's new (where the row lets them turn it off) before the first
/// automatic run: nothing is due while it waits, a run asked for anyway is refused and
/// counted, and the setting can be changed meanwhile.
#[tokio::test]
async fn nothing_runs_while_whats_new_waits() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    open(&data)
        .add_folder_source(&course_folder(temp.path(), "Courses"), None, None)
        .unwrap();
    // 0.1 never recorded its version; a new process starts.
    store(&data).remove_setting("app.last_run_version").unwrap();
    let app = open(&data);
    let now = Utc::now();
    let tasks = app.startup_tasks(now).unwrap();
    assert!(tasks.whats_new.is_some());
    assert_eq!(
        tasks.sync_due,
        SyncDue::default(),
        "not while What's new waits"
    );

    let started = Mutex::new(0);
    let refused = app
        .sync_all(automatic(), |_| *started.lock().unwrap() += 1)
        .await
        .unwrap_err();
    assert_eq!(refused.kind, AppErrorKind::Invalid);
    assert_eq!(*started.lock().unwrap(), 0, "nothing started");
    assert_eq!(attempts(&data).failed, 1, "a refusal is counted");

    // The row's control works before the sheet is closed.
    set(&app, AutoSync::Off);
    app.acknowledge_whats_new().unwrap();
    assert!(!due(&app, now + TimeDelta::days(2)), "turned off there");
    set(&app, AutoSync::TwiceDaily);
    // (Counted after `now` was taken: a time before the attempt would read it as unknown.)
    let now = Utc::now();
    assert!(!due(&app, now), "the refusal's wait: an hour");
    assert!(due(&app, now + TimeDelta::minutes(61)));
}

#[tokio::test]
async fn an_automatic_run_is_counted_first_stays_quiet_and_leaves_what_needs_the_student() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
        .mount(&server)
        .await;
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    let folder = app
        .add_folder_source(&course_folder(temp.path(), "Courses"), None, None)
        .unwrap();
    let gone = app
        .add_folder_source(&course_folder(temp.path(), "Gone"), None, None)
        .unwrap();
    let feed = app
        .add_ical_source(&format!("{}/feed.ics", server.uri()), Some("Feed"))
        .await
        .unwrap();
    assert!(
        app.sync_all(SyncRequest::default(), |_| {})
            .await
            .unwrap()
            .ok
    );
    let now = Utc::now();
    assert!(!due(&app, now), "just synced");
    // A sync that isn't due does nothing and counts nothing.
    let nothing = app.sync_all(automatic(), |_| {}).await.unwrap();
    assert!(nothing.ok && nothing.results.is_empty());
    assert_eq!(attempts(&data), AutoSyncAttempts::default());

    // A folder disappears, and the student's own sync says so: it needs the student now.
    std::fs::remove_dir_all(temp.path().join("Gone")).unwrap();
    let failed = app
        .sync_source(&gone.id, SyncRequest::default(), |_| {})
        .await
        .unwrap();
    assert_eq!(failed.error_kind, Some(SourceErrorKind::NotFound));
    // Everything is 13 hours old, and the feed's server is having a bad day.
    for id in [&folder.id, &feed.id] {
        synced_hours_ago(&data, id, 13);
    }
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/feed.ics"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    assert!(due(&app, Utc::now()));

    let tried = Mutex::new(Vec::new());
    let summary = app
        .sync_all(automatic(), |event| {
            if let SyncEvent::SourceStarted { source_id, .. } = event {
                tried.lock().unwrap().push(source_id);
            }
        })
        .await
        .unwrap();
    // The missing folder was left alone; the feed failed and the folder synced.
    assert_eq!(*tried.lock().unwrap(), [folder.id.clone(), feed.id.clone()]);
    assert!(!summary.ok);
    let feed_result = &summary.results[1];
    assert_eq!(feed_result.error_kind, Some(SourceErrorKind::Network));
    // Quiet: the feed isn't marked failed (nothing asks for the student's attention), and it
    // is still as old as it was.
    let sources = app.list_sources().unwrap();
    let row = |id: &str| sources.iter().find(|s| s.id == id).unwrap();
    assert_eq!(
        (
            row(&feed.id).last_error.clone(),
            row(&feed.id).last_error_kind
        ),
        (None, None)
    );
    assert!(Utc::now() - row(&feed.id).last_synced_at.unwrap() >= TimeDelta::hours(13));
    assert!(Utc::now() - row(&folder.id).last_synced_at.unwrap() < TimeDelta::minutes(5));
    assert_eq!(
        row(&gone.id).last_error_kind,
        Some(SourceErrorKind::NotFound),
        "still shown on Sources"
    );

    // Counted, so nothing retries back to back: not due now, due again an hour later.
    let counted = attempts(&data);
    assert_eq!((counted.failed, counted.last_ok_at), (1, None));
    assert_eq!(counted.by_source.len(), 2, "the two it tried");
    let now = Utc::now();
    assert!(!due(&app, now));
    let again = app.sync_all(automatic(), |_| {}).await.unwrap();
    assert!(again.results.is_empty(), "not due: nothing ran");
    assert_eq!(attempts(&data).failed, 1, "and nothing was counted");
    assert!(!due(&app, now + TimeDelta::minutes(55)));
    assert!(due(&app, now + TimeDelta::minutes(61)));

    // A sync the student starts says what failed, as always.
    let feed_error = || {
        app.list_sources()
            .unwrap()
            .into_iter()
            .find(|s| s.id == feed.id)
            .unwrap()
            .last_error_kind
    };
    let manual = app
        .sync_source(&feed.id, SyncRequest::default(), |_| {})
        .await
        .unwrap();
    assert!(!manual.ok);
    assert_eq!(feed_error(), Some(SourceErrorKind::Network));

    // The feed's link is revoked: the next automatic run records that (only the student can
    // fix it), and after it the feed is left alone.
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/feed.ics"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    let wait_is_over = |data: &Path| {
        let mut record = attempts(data);
        record.last_at = Some(Utc::now() - TimeDelta::hours(13));
        store(data)
            .set_setting(AUTO_SYNC_ATTEMPTS_KEY, &record)
            .unwrap();
    };
    wait_is_over(&data);
    let revoked = app.sync_all(automatic(), |_| {}).await.unwrap();
    assert_eq!(revoked.results.len(), 1, "only the source that is due");
    assert_eq!(revoked.results[0].source_id, feed.id);
    assert_eq!(
        revoked.results[0].error_kind,
        Some(SourceErrorKind::AuthExpiredOrRevoked)
    );
    assert_eq!(feed_error(), Some(SourceErrorKind::AuthExpiredOrRevoked));
    wait_is_over(&data);
    assert!(
        !due(&app, Utc::now()),
        "the folder is fresh and the feed needs the student"
    );
    synced_hours_ago(&data, &folder.id, 13);
    let later = app.sync_all(automatic(), |_| {}).await.unwrap();
    assert!(later.ok);
    assert_eq!(later.results.len(), 1);
    assert_eq!(later.results[0].source_id, folder.id);
}

#[tokio::test]
async fn a_run_that_ends_well_clears_the_wait_and_one_refused_by_a_running_sync_is_counted() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    let folder = app
        .add_folder_source(&course_folder(temp.path(), "Courses"), None, None)
        .unwrap();

    // Another process is syncing: refused as busy, and counted before the lock was tried.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data.join("sync.lock"))
        .unwrap();
    lock.try_lock().unwrap();
    let busy = app.sync_all(automatic(), |_| {}).await.unwrap_err();
    assert_eq!(busy.kind, AppErrorKind::Busy);
    assert_eq!(attempts(&data).failed, 1);
    drop(lock);
    let now = Utc::now();
    assert!(!due(&app, now), "the refusal's wait");

    // An hour later the run happens and ends well: no wait is left.
    let mut record = attempts(&data);
    record.last_at = Some(now - TimeDelta::minutes(61));
    store(&data)
        .set_setting(AUTO_SYNC_ATTEMPTS_KEY, &record)
        .unwrap();
    let summary = app.sync_all(automatic(), |_| {}).await.unwrap();
    assert!(summary.ok);
    assert_eq!(summary.results.len(), 1);
    let record = attempts(&data);
    assert_eq!(record.failed, 0);
    assert!(record.last_ok_at.is_some());
    assert_eq!(record.by_source[&folder.id].len(), 2);
    assert!(!due(&app, Utc::now()), "fresh");
    assert!(due(&app, Utc::now() + TimeDelta::hours(12)));

    // Only `sync_all` runs by itself: the flag changes nothing for one source.
    let one = app
        .sync_source(&folder.id, automatic(), |_| {})
        .await
        .unwrap();
    assert!(one.ok);
    assert_eq!(attempts(&data), record);
}

/// A small Canvas: DEMO101 (a module with a link, an assignment) and, when `seminar`, DEMO404.
async fn canvas(server: &MockServer, seminar: bool, planner: serde_json::Value) {
    use serde_json::json;
    server.reset().await;
    let get = |at: &str, body: serde_json::Value| {
        Mock::given(method("GET"))
            .and(path(format!("/api/v1{at}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
    };
    let mut courses =
        vec![json!({"id": 101, "name": "Intro to Demo Studies", "course_code": "DEMO101"})];
    if seminar {
        courses.push(json!({"id": 404, "name": "Demo Seminar", "course_code": "DEMO404"}));
    }
    let due = (Utc::now() + TimeDelta::days(5)).to_rfc3339();
    for mock in [
        get("/users/self", json!({"id": 1, "name": "Demo Student"})),
        get("/courses", json!(courses)),
        get(
            "/courses/101/tabs",
            json!([{"id": "home"}, {"id": "modules"}]),
        ),
        get(
            "/courses/101/modules",
            json!([{"id": 1, "name": "Week 1", "items": [
                {"id": 13, "type": "ExternalUrl", "external_url": "https://video.example.edu/w1",
                 "title": "Lecture video"}]}]),
        ),
        get(
            "/courses/101/assignments",
            json!([{"id": 9, "name": "Problem Set 1", "due_at": due,
                    "html_url": "/courses/101/assignments/9"}]),
        ),
        get(
            "/courses/404/tabs",
            json!([{"id": "home"}, {"id": "modules"}]),
        ),
        get(
            "/courses/404/modules",
            json!([{"id": 2, "name": "Week 1", "items": [
                {"id": 23, "type": "ExternalUrl", "external_url": "https://video.example.edu/s1",
                 "title": "Seminar reading"}]}]),
        ),
        get("/courses/404/assignments", json!([])),
        get("/announcements", json!([])),
        get("/planner/items", planner),
    ] {
        mock.mount(server).await;
    }
}

/// The requests the server got since its last reset that name a course in their address.
async fn course_requests(server: &MockServer) -> Vec<String> {
    let requests = server.received_requests().await.unwrap();
    requests
        .iter()
        .map(|request| request.url.path().to_string())
        .filter(|path| path.starts_with("/api/v1/courses/"))
        .collect()
}

/// With nobody at the app, Canvas is read lightly: no request names a course, the source's
/// `last_synced_at` (its last full sync) stays, and a course found that way waits, said so,
/// for the full sync the student's return starts.
#[tokio::test]
async fn an_unattended_run_reads_canvas_lightly_and_each_scope_has_its_clock() {
    use serde_json::json;
    let server = MockServer::start().await;
    canvas(&server, false, json!([])).await;
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    let source = app
        .add_canvas_source(&server.uri(), "demo-not-a-real-token")
        .await
        .unwrap();
    let first = app.sync_all(SyncRequest::default(), |_| {}).await.unwrap();
    assert!(first.ok, "{first:?}");
    assert!(!course_requests(&server).await.is_empty(), "a full sync");
    let course = app.list_courses().unwrap().remove(0);
    assert!(!course.structure_pending);
    assert_eq!(course.deadlines_synced_at, course.last_synced_at);
    assert_eq!(course.counts.materials, 1);
    let now = Utc::now();
    assert_eq!(app.startup_tasks(now).unwrap().sync_due, SyncDue::default());

    // 13 hours later a second course is in Canvas and Problem Set 1 has moved.
    synced_hours_ago(&data, &source.id, 13);
    let both = SyncDue {
        unattended: true,
        attended: true,
    };
    assert_eq!(app.startup_tasks(Utc::now()).unwrap().sync_due, both);
    let moved = Utc::now() + TimeDelta::days(8);
    let planner = json!([{"plannable_id": 9, "plannable_type": "assignment", "course_id": 101,
        "plannable_date": moved.to_rfc3339(), "plannable": {"title": "Problem Set 1"}}]);
    canvas(&server, true, planner.clone()).await;

    // First the planner is down. A light run is only for deadlines and announcements, so one
    // that couldn't read them doesn't count as having read them: no stamp, a quiet failure
    // that waits like any other. The course it found is listed and already said to wait.
    Mock::given(method("GET"))
        .and(path("/api/v1/planner/items"))
        .respond_with(ResponseTemplate::new(500))
        .with_priority(1)
        .mount(&server)
        .await;
    let missed = app.sync_all(automatic(), |_| {}).await.unwrap();
    assert!(!missed.ok);
    assert_eq!(
        missed.results[0].error_kind,
        Some(SourceErrorKind::Network),
        "it may pass by itself"
    );
    assert!(app.status().unwrap().deadlines_synced_at.is_empty());
    let row = app.list_sources().unwrap().remove(0);
    assert_eq!((row.last_error, row.last_error_kind), (None, None), "quiet");
    assert_eq!(attempts(&data).failed, 1);
    // The timer waits after its own attempt; the student's return doesn't.
    let attended_first = SyncDue {
        unattended: false,
        attended: true,
    };
    assert_eq!(
        app.startup_tasks(Utc::now()).unwrap().sync_due,
        attended_first
    );
    let found = app.course_overview("DEMO404").unwrap();
    assert!(found.structure_pending, "said from the moment it is listed");
    assert_eq!(found.deadlines_synced_at, found.last_synced_at);
    let mut record = attempts(&data);
    record.last_at = Some(Utc::now() - TimeDelta::hours(2));
    store(&data)
        .set_setting(AUTO_SYNC_ATTEMPTS_KEY, &record)
        .unwrap();
    canvas(&server, true, planner.clone()).await;

    let light = app.sync_all(automatic(), |_| {}).await.unwrap();
    assert!(light.ok, "{light:?}");
    assert_eq!(light.results.len(), 1, "it ran");
    assert_eq!(course_requests(&server).await, Vec::<String>::new());

    // The full sync's clock didn't move; the deadlines' did.
    let full_at = |app: &App| app.list_sources().unwrap()[0].last_synced_at.unwrap();
    assert!(Utc::now() - full_at(&app) >= TimeDelta::hours(13));
    let status =
        pagelamp_core::views::sync_status(&store(&data), pagelamp_core::views::AsOf::now_local())
            .unwrap();
    assert!(!status.sources[0].stale, "13 hours isn't stale");
    let read_at = status.sources[0].deadlines_synced_at.unwrap();
    assert!(Utc::now() - read_at < TimeDelta::minutes(5));
    // The Sources screen gets the second clock too, only while it is the later one.
    assert_eq!(
        app.status().unwrap().deadlines_synced_at,
        std::collections::BTreeMap::from([(source.id.clone(), read_at)])
    );
    let courses = app.list_courses().unwrap();
    let by_code = |code: &str| {
        courses
            .iter()
            .find(|c| c.course.code.as_deref() == Some(code))
            .unwrap()
    };
    let (intro, seminar) = (by_code("DEMO101"), by_code("DEMO404"));
    assert_eq!(intro.last_synced_at, Some(full_at(&app)));
    assert_eq!(intro.deadlines_synced_at, Some(read_at));
    assert_eq!(intro.counts.materials, 1, "nothing was removed");
    assert!(!intro.structure_pending);
    assert_eq!(
        intro
            .next_deadline
            .as_ref()
            .unwrap()
            .event
            .due_at
            .unwrap()
            .timestamp(),
        moved.timestamp()
    );
    assert!(seminar.structure_pending, "found, not read yet");
    assert_eq!(seminar.counts.materials, 0);
    let overview = app.course_overview("DEMO404").unwrap();
    assert!(overview.structure_pending);
    assert_eq!(overview.deadlines_synced_at, Some(read_at));

    // The timer has nothing more to do; the student's return still starts a full sync, and
    // would for the new course alone.
    let attended_only = SyncDue {
        unattended: false,
        attended: true,
    };
    assert_eq!(
        app.startup_tasks(Utc::now()).unwrap().sync_due,
        attended_only
    );
    let again = app.sync_all(automatic(), |_| {}).await.unwrap();
    assert!(again.results.is_empty(), "not due from the timer");
    synced_hours_ago(&data, &source.id, 1);
    assert_eq!(
        app.startup_tasks(Utc::now()).unwrap().sync_due,
        attended_only,
        "a course waits for its first full sync"
    );

    let attended = SyncRequest {
        automatic: Some(AutoSyncTrigger::Attended),
        ..SyncRequest::default()
    };
    let full = app.sync_all(attended, |_| {}).await.unwrap();
    assert!(full.ok, "{full:?}");
    let asked = course_requests(&server).await;
    assert!(
        asked
            .iter()
            .any(|path| path == "/api/v1/courses/404/modules"),
        "{asked:?}"
    );
    assert!(Utc::now() - full_at(&app) < TimeDelta::minutes(5));
    let courses = app.list_courses().unwrap();
    let seminar = courses
        .iter()
        .find(|c| c.course.code.as_deref() == Some("DEMO404"))
        .unwrap();
    assert!(!seminar.structure_pending);
    assert_eq!(seminar.counts.materials, 1);
    assert_eq!(seminar.deadlines_synced_at, seminar.last_synced_at);
    assert!(app.status().unwrap().deadlines_synced_at.is_empty());
    assert_eq!(
        app.startup_tasks(Utc::now()).unwrap().sync_due,
        SyncDue::default()
    );
    let record = |data: &Path| -> LightSync {
        store(data)
            .setting_or_absent(LIGHT_SYNC_KEY)
            .unwrap()
            .unwrap_or_default()
    };
    assert_eq!(
        record(&data),
        LightSync::default(),
        "one clock again, and nothing waits"
    );

    // A course that leaves Canvas before any full sync read it doesn't keep one due for ever.
    let mut left = record(&data);
    left.structure_pending
        .insert(format!("{}/course/999", source.id));
    store(&data).set_setting(LIGHT_SYNC_KEY, &left).unwrap();
    assert_eq!(
        app.startup_tasks(Utc::now()).unwrap().sync_due,
        attended_only
    );
    assert!(
        app.sync_all(SyncRequest::default(), |_| {})
            .await
            .unwrap()
            .ok
    );
    assert!(record(&data).structure_pending.is_empty());

    // Removing the source forgets what the light runs left of it.
    let mut left = LightSync::default();
    left.light_run_ended(
        &source.id,
        Utc::now(),
        &[format!("{}/course/1000", source.id)],
    );
    store(&data).set_setting(LIGHT_SYNC_KEY, &left).unwrap();
    assert_ne!(record(&data), LightSync::default());
    app.remove_source(&source.id).unwrap();
    assert_eq!(record(&data), LightSync::default());
}

/// A student who stops a sync (the first one, say) doesn't get an automatic one at once: the
/// sources it didn't reach were never synced, which would make one due.
#[tokio::test]
async fn nothing_starts_by_itself_for_an_hour_after_the_student_stops_a_sync() {
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    app.add_folder_source(&course_folder(temp.path(), "Courses"), None, None)
        .unwrap();
    let before = Utc::now();
    assert!(due(&app, before), "never synced");

    // Stopped when the source starts (the event comes before any file is read).
    let remote = app.clone();
    let stopped = app
        .sync_all(SyncRequest::default(), move |event| {
            if matches!(event, SyncEvent::SourceStarted { .. }) {
                remote.cancel_sync();
            }
        })
        .await
        .unwrap_err();
    assert_eq!(stopped.kind, AppErrorKind::Cancelled);
    assert_eq!(app.list_sources().unwrap()[0].last_synced_at, None);

    let now = Utc::now();
    assert!(!due(&app, now), "the student pressed Stop");
    let nothing = app.sync_all(automatic(), |_| {}).await.unwrap();
    assert!(
        nothing.results.is_empty(),
        "an automatic run asked for anyway does nothing"
    );
    let record = attempts(&data);
    assert_eq!(
        (record.failed, record.last_at),
        (0, None),
        "and counts nothing"
    );
    assert!(record.stopped_at.is_some_and(|at| at >= before));
    assert!(!due(&app, now + TimeDelta::minutes(55)));
    assert!(
        due(&app, now + TimeDelta::minutes(61)),
        "an hour later it may"
    );

    // A sync that ends by itself records no stop.
    assert!(
        app.sync_all(SyncRequest::default(), |_| {})
            .await
            .unwrap()
            .ok
    );
    assert_eq!(attempts(&data).stopped_at, record.stopped_at);
}

/// A light run Canvas rejects (the token expired) is recorded on the source like any sync: only
/// the student can fix it. It stamps nothing, and the last full sync's time stays.
#[tokio::test]
async fn a_light_run_with_an_expired_token_is_recorded_and_stamps_nothing() {
    use serde_json::json;
    let server = MockServer::start().await;
    canvas(&server, false, json!([])).await;
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    let source = app
        .add_canvas_source(&server.uri(), "demo-not-a-real-token")
        .await
        .unwrap();
    assert!(
        app.sync_all(SyncRequest::default(), |_| {})
            .await
            .unwrap()
            .ok
    );
    synced_hours_ago(&data, &source.id, 13);
    let full_at = app.list_sources().unwrap()[0].last_synced_at;

    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"errors": [{"message": "Invalid access token."}]})),
        )
        .mount(&server)
        .await;
    let rejected = app.sync_all(automatic(), |_| {}).await.unwrap();
    assert_eq!(
        rejected.results[0].error_kind,
        Some(SourceErrorKind::AuthExpiredOrRevoked)
    );
    let row = app.list_sources().unwrap().remove(0);
    assert_eq!(
        row.last_error_kind,
        Some(SourceErrorKind::AuthExpiredOrRevoked)
    );
    assert_eq!(row.last_synced_at, full_at);
    assert!(app.status().unwrap().deadlines_synced_at.is_empty());
    let light: LightSync = store(&data)
        .setting_or_absent(LIGHT_SYNC_KEY)
        .unwrap()
        .unwrap_or_default();
    assert_eq!(light, LightSync::default());
    // It needs the student now: nothing is due by itself, from either trigger.
    assert_eq!(
        app.startup_tasks(Utc::now() + TimeDelta::days(2))
            .unwrap()
            .sync_due,
        SyncDue::default()
    );
}

/// The requests the server got since its last reset, as (path, User-Agent).
async fn requests(server: &MockServer) -> Vec<(String, String)> {
    let requests = server.received_requests().await.unwrap();
    requests
        .iter()
        .map(|request| {
            let agent = request.headers["user-agent"].to_str().unwrap().to_string();
            (request.url.path().to_string(), agent)
        })
        .collect()
}

/// DEMO101 also has a Files tab with one file (mounted over `canvas`).
async fn with_a_file(server: &MockServer) {
    use serde_json::json;
    for (at, body) in [
        (
            "/api/v1/courses/101/tabs",
            json!([{"id": "home"}, {"id": "modules"}, {"id": "files"}]),
        ),
        (
            "/api/v1/courses/101/files",
            json!([{"id": 501, "display_name": "slides.txt", "filename": "slides.txt",
                    "content-type": "text/plain", "size": 20,
                    "url": format!("{}/files/501/download?download_frd=1", server.uri()),
                    "updated_at": "2026-09-20T10:00:00Z"}]),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(at))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .with_priority(1)
            .mount(server)
            .await;
    }
}

/// What the automatic sync's rule and the "data as of" lines read about one source.
fn clocks(
    app: &App,
    data: &Path,
) -> (
    Option<DateTime<Utc>>,
    LightSync,
    bool,
    Option<DateTime<Utc>>,
) {
    let light: LightSync = store(data)
        .setting_or_absent(LIGHT_SYNC_KEY)
        .unwrap()
        .unwrap_or_default();
    let status =
        pagelamp_core::views::sync_status(&store(data), pagelamp_core::views::AsOf::now_local())
            .unwrap();
    (
        app.list_sources().unwrap()[0].last_synced_at,
        light,
        status.sources[0].stale,
        status.sources[0].deadlines_synced_at,
    )
}

/// "Download this course's files" and a sync limited to some courses read only those. They
/// must not make the whole source look freshly synced: not to the student, not to an AI app,
/// not to the automatic sync's clocks.
#[tokio::test]
async fn a_sync_limited_to_some_courses_leaves_the_sources_clocks_alone() {
    use serde_json::json;
    let server = MockServer::start().await;
    canvas(&server, true, json!([])).await;
    with_a_file(&server).await;
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    let source = app
        .add_canvas_source(&server.uri(), "demo-not-a-real-token")
        .await
        .unwrap();
    assert!(
        app.sync_all(SyncRequest::default(), |_| {})
            .await
            .unwrap()
            .ok
    );
    // The full sync is 30 hours old; a light run read the deadlines since.
    synced_hours_ago(&data, &source.id, 30);
    assert!(app.sync_all(automatic(), |_| {}).await.unwrap().ok);
    let before = clocks(&app, &data);
    assert!(before.2, "30 hours is stale");
    assert!(before.1.synced_at.contains_key(&source.id));
    let due = SyncDue {
        unattended: false,
        attended: true,
    };
    assert_eq!(app.startup_tasks(Utc::now()).unwrap().sync_due, due);

    // The student downloads one course's files.
    Mock::given(method("GET"))
        .and(path("/files/501/download"))
        .respond_with(ResponseTemplate::new(200).set_body_string("lambdaword"))
        .mount(&server)
        .await;
    let downloaded = app.download_course_files("DEMO101", |_| {}).await.unwrap();
    assert!(downloaded.ok, "{downloaded:?}");
    assert_eq!(downloaded.files_downloaded, 1);
    assert_eq!(clocks(&app, &data), before);
    assert_eq!(app.startup_tasks(Utc::now()).unwrap().sync_due, due);

    // A sync limited to one course: the same.
    let limited = SyncRequest {
        only_courses: vec!["DEMO101".to_string()],
        ..SyncRequest::default()
    };
    let summary = app.sync_all(limited.clone(), |_| {}).await.unwrap();
    assert!(summary.ok, "{summary:?}");
    assert_eq!(clocks(&app, &data), before);
    assert_eq!(app.startup_tasks(Utc::now()).unwrap().sync_due, due);

    // A good end still clears an error an earlier sync left on the source.
    store(&data)
        .record_sync(
            &source.id,
            Utc::now(),
            Some((SourceErrorKind::Other, "an earlier failure")),
        )
        .unwrap();
    assert!(app.sync_all(limited, |_| {}).await.unwrap().ok);
    let row = app.list_sources().unwrap().remove(0);
    assert_eq!((row.last_error, row.last_error_kind), (None, None));
    assert_eq!(row.last_synced_at, before.0);
}

/// A waiting course a full sync couldn't read keeps waiting, which is the truth for every
/// view and for an AI app, but it doesn't make full syncs follow one another.
#[tokio::test]
async fn a_waiting_course_a_full_sync_could_not_read_keeps_waiting() {
    use serde_json::json;
    let server = MockServer::start().await;
    canvas(&server, false, json!([])).await;
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    let source = app
        .add_canvas_source(&server.uri(), "demo-not-a-real-token")
        .await
        .unwrap();
    assert!(
        app.sync_all(SyncRequest::default(), |_| {})
            .await
            .unwrap()
            .ok
    );
    synced_hours_ago(&data, &source.id, 13);
    canvas(&server, true, json!([])).await;
    assert!(app.sync_all(automatic(), |_| {}).await.unwrap().ok);
    assert!(app.course_overview("DEMO404").unwrap().structure_pending);

    // The student comes back; Canvas answers 502 for the new course's modules.
    Mock::given(method("GET"))
        .and(path("/api/v1/courses/404/modules"))
        .respond_with(ResponseTemplate::new(502))
        .with_priority(1)
        .mount(&server)
        .await;
    let attended = SyncRequest {
        automatic: Some(AutoSyncTrigger::Attended),
        ..SyncRequest::default()
    };
    let full = app.sync_all(attended.clone(), |_| {}).await.unwrap();
    assert!(full.ok, "a course's modules failing is a warning: {full:?}");
    let seminar = app.course_overview("DEMO404").unwrap();
    assert!(seminar.structure_pending, "it wasn't read: still said so");
    assert_eq!(seminar.current_modules.len(), 0);
    assert!(!app.course_overview("DEMO101").unwrap().structure_pending);
    assert_eq!(
        app.startup_tasks(Utc::now()).unwrap().sync_due,
        SyncDue::default(),
        "tried: no full sync right after"
    );
    assert!(
        app.startup_tasks(Utc::now() + TimeDelta::hours(12))
            .unwrap()
            .sync_due
            .attended,
        "the regular one tries again"
    );

    // Canvas is well again and the full sync is 13 hours old: the course is read.
    canvas(&server, true, json!([])).await;
    synced_hours_ago(&data, &source.id, 13);
    assert!(app.sync_all(attended, |_| {}).await.unwrap().ok);
    let seminar = app.course_overview("DEMO404").unwrap();
    assert!(!seminar.structure_pending);
    let light: LightSync = store(&data)
        .setting_or_absent(LIGHT_SYNC_KEY)
        .unwrap()
        .unwrap_or_default();
    assert_eq!(light, LightSync::default());
}

/// A source that keeps failing brings the others no extra sync: a retry reads only what is
/// due. And what an automatic request can and can't change about a run.
#[tokio::test]
async fn a_retry_reads_only_the_sources_that_are_due() {
    use serde_json::json;
    let server = MockServer::start().await;
    canvas(&server, false, json!([])).await;
    with_a_file(&server).await;
    let feed_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.ics"))
        .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
        .mount(&feed_server)
        .await;
    let temp = tempfile::tempdir().unwrap();
    let data = data_dir(&temp);
    let app = open(&data);
    let source = app
        .add_canvas_source(&server.uri(), "demo-not-a-real-token")
        .await
        .unwrap();
    let feed = app
        .add_ical_source(&format!("{}/feed.ics", feed_server.uri()), Some("Feed"))
        .await
        .unwrap();
    assert!(
        app.sync_all(SyncRequest::default(), |_| {})
            .await
            .unwrap()
            .ok
    );
    // A sync the student started names itself plainly.
    let manual = requests(&server).await;
    assert!(
        !manual.is_empty()
            && manual
                .iter()
                .all(|(_, agent)| agent.ends_with("(read-only)")),
        "{manual:?}"
    );

    // The feed is 13 hours old and its server is down; Canvas is fresh.
    synced_hours_ago(&data, &feed.id, 13);
    feed_server.reset().await;
    Mock::given(method("GET"))
        .and(path("/feed.ics"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&feed_server)
        .await;
    canvas(&server, false, json!([])).await;
    with_a_file(&server).await;
    for _ in 0..2 {
        let run = app.sync_all(automatic(), |_| {}).await.unwrap();
        assert_eq!(run.results.len(), 1, "{run:?}");
        assert_eq!(run.results[0].source_id, feed.id);
        assert_eq!(requests(&server).await, Vec::new(), "no Canvas request");
        // The retry wait is over.
        let mut record = attempts(&data);
        record.last_at = Some(Utc::now() - TimeDelta::hours(13));
        store(&data)
            .set_setting(AUTO_SYNC_ATTEMPTS_KEY, &record)
            .unwrap();
    }
    assert!(!attempts(&data).by_source.contains_key(&source.id));

    // An automatic request can't ask for downloads or name courses: both are dropped, so the
    // run reads the whole source and downloads nothing. It says "automatic sync".
    synced_hours_ago(&data, &source.id, 13);
    let greedy = SyncRequest {
        automatic: Some(AutoSyncTrigger::Attended),
        download_files: true,
        only_courses: vec!["NO-SUCH-COURSE".to_string()],
        ..SyncRequest::default()
    };
    let run = app.sync_all(greedy, |_| {}).await.unwrap();
    let canvas_run = run
        .results
        .iter()
        .find(|result| result.source_id == source.id)
        .unwrap();
    assert!(canvas_run.ok, "{canvas_run:?}");
    assert_eq!(canvas_run.files_downloaded, 0);
    let seen = requests(&server).await;
    assert!(
        seen.iter()
            .any(|(path, _)| path == "/api/v1/courses/101/modules"),
        "{seen:?}"
    );
    assert!(seen.iter().all(|(path, _)| !path.contains("/download")));
    assert!(
        seen.iter()
            .all(|(_, agent)| agent.ends_with("(read-only; automatic sync)")),
        "{seen:?}"
    );
    let row = |app: &App| {
        app.list_sources()
            .unwrap()
            .into_iter()
            .find(|row| row.id == source.id)
            .unwrap()
    };
    assert!(Utc::now() - row(&app).last_synced_at.unwrap() < TimeDelta::minutes(5));

    // Only `sync_all` runs by itself. For one source the flag changes nothing: a full sync,
    // named plainly, and a failure that would be quiet in an automatic run is recorded.
    canvas(&server, false, json!([])).await;
    let one = app
        .sync_source(&source.id, automatic(), |_| {})
        .await
        .unwrap();
    assert!(one.ok, "{one:?}");
    let seen = requests(&server).await;
    assert!(
        seen.iter()
            .any(|(path, _)| path == "/api/v1/courses/101/modules")
    );
    assert!(seen.iter().all(|(_, agent)| agent.ends_with("(read-only)")));
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let failed = app
        .sync_source(&source.id, automatic(), |_| {})
        .await
        .unwrap();
    assert_eq!(failed.error_kind, Some(SourceErrorKind::Network));
    assert_eq!(row(&app).last_error_kind, Some(SourceErrorKind::Network));
}
