//! Conversions, mirror coverage and the exported API end to end (temp data dirs, folder
//! sources and in-memory secrets only: nothing here touches the keychain or the real data dir).

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use chrono::{Datelike, Local, NaiveDate, TimeDelta, TimeZone, Utc};
use uniffi::FfiConverter;

use super::*;

type Tag = crate::UniFfiTag;

/// Lower to the FFI representation and lift back, as a call across the boundary does.
fn round_trip<T: FfiConverter<Tag>>(value: T) -> uniffi::Result<T> {
    T::try_lift(T::lower(value))
}

/// Poll an exported future on a throwaway single-thread runtime, like a foreign executor that
/// knows nothing about PageLamp's runtime.
fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime")
        .block_on(future)
}

// ----- custom types ------------------------------------------------------------------------------

#[test]
fn timestamps_cross_as_system_time() {
    let time =
        Utc.with_ymd_and_hms(2026, 9, 26, 12, 34, 56).unwrap() + TimeDelta::microseconds(789);
    let system: SystemTime = time.into();
    assert_eq!(
        system.duration_since(SystemTime::UNIX_EPOCH).unwrap(),
        Duration::from_micros(u64::try_from(time.timestamp_micros()).unwrap())
    );
    assert_eq!(round_trip::<Timestamp>(time).unwrap(), time);
    // Before 1970 too (Swift `Date` allows it).
    let old = Utc.with_ymd_and_hms(1969, 7, 20, 20, 17, 0).unwrap();
    assert_eq!(round_trip::<Timestamp>(old).unwrap(), old);
}

#[test]
fn iso_dates_cross_as_yyyy_mm_dd() {
    let date = NaiveDate::from_ymd_opt(2026, 9, 8).unwrap();
    assert_eq!(types::iso_date_to_string(date), "2026-09-08");
    assert_eq!(round_trip::<IsoDate>(date).unwrap(), date);
    assert_eq!(types::iso_date_from_string(" 2026-09-08 ").unwrap(), date);
    // What Swift would send: the string form, lifted as the custom type.
    let lowered = <String as FfiConverter<Tag>>::lower("2027-01-31".to_string());
    assert_eq!(
        <IsoDate as FfiConverter<Tag>>::try_lift(lowered).unwrap(),
        NaiveDate::from_ymd_opt(2027, 1, 31).unwrap()
    );
    // A bad date from Swift fails with `Invalid`, which UniFFI hands back as the thrown
    // `PageLampError` (it downcasts lift errors to the function's error type).
    for bad in [
        "2026-13-01",
        "2026-02-30",
        "26-09-08",
        "2026-9-8",
        "+2026-09-08",
        "yesterday",
        "",
    ] {
        let lowered = <String as FfiConverter<Tag>>::lower(bad.to_string());
        let err = <IsoDate as FfiConverter<Tag>>::try_lift(lowered).unwrap_err();
        assert!(
            matches!(
                err.downcast_ref::<PageLampError>(),
                Some(PageLampError::Invalid { .. })
            ),
            "{bad}: {err:?}"
        );
    }
}

#[test]
fn json_crosses_as_text() {
    let value =
        serde_json::json!({ "path": "/tmp/Courses", "term_start": "2026-09-08", "n": [1, 2] });
    let lowered = <JsonString as FfiConverter<Tag>>::lower(value.clone());
    let text = <String as FfiConverter<Tag>>::try_lift(lowered).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap(),
        value
    );
    assert_eq!(round_trip::<JsonString>(value.clone()).unwrap(), value);
    assert!(matches!(
        types::json_from_string("{not json"),
        Err(PageLampError::Invalid { .. })
    ));
}

#[test]
fn env_maps_cross_as_dictionaries() {
    let map = BTreeMap::from([
        ("PAGELAMP_HOME".to_string(), "/tmp/data dir".to_string()),
        ("B".to_string(), "2".to_string()),
    ]);
    let lowered = <EnvMap as FfiConverter<Tag>>::lower(map.clone());
    let dict = <HashMap<String, String> as uniffi::Lift<Tag>>::try_lift(lowered).unwrap();
    assert_eq!(dict.len(), 2);
    assert_eq!(dict["PAGELAMP_HOME"], "/tmp/data dir");
    assert_eq!(round_trip::<EnvMap>(map.clone()).unwrap(), map);
}

// ----- mirror coverage -------------------------------------------------------------------------

/// Every type in the facade's JSON Schema, i.e. every type that crosses the facade, and how
/// this crate carries it. Mirrors themselves are checked field by field by the compiler; this
/// list catches a NEW facade type that nobody mirrored yet.
const MIRRORED: &[&str] = &[
    // pagelamp-core model
    "SourceKind",
    "SourceErrorKind",
    "SourceRecord",
    "AiPolicy",
    "Course",
    "TermSource",
    "AiMaterialsState",
    "Module",
    "MaterialKind",
    "TextStatus",
    "DownloadBlock",
    "TextErrorKind",
    "EventKind",
    "Event",
    "SearchHit",
    "Confidence",
    "CourseTimeline",
    "CoursePhase",
    "TermAnchorSource",
    "RejectReason",
    "RejectedDates",
    "BreakKind",
    "CalendarOrigin",
    "AiLabel",
    "TeachingSegment",
    "DateSpan",
    "CalendarBreak",
    "CalendarStatus",
    "TermResolution",
    "EvidenceParam",
    "EvidenceItem",
    "EvidenceCode",
    "EvidenceSignal",
    "LifecycleState",
    "CourseGroup",
    "SnoozeKind",
    "CourseLifecycle",
    "StoreCounts",
    "StudyPlanItem",
    "StudyPlan",
    "StoredStudyPlan",
    // pagelamp-core views / source
    "Deadline",
    "CourseCounts",
    "CourseSummary",
    "MaterialView",
    "TextProblem",
    "CourseOverview",
    "WeekNoteKind",
    "WeekMaterials",
    "CourseSyncSummary",
    // pagelamp-app
    "AppStatus",
    "SyncRequest",
    "SyncEvent",
    "SyncStage",
    "SourceSyncResult",
    "SyncSummary",
    "McpClient",
    "InstallKind",
    "McpLaunch",
    "TemporaryLocation",
    "McpClientConfig",
    "McpNoteCode",
    "ProcessKind",
    "CrashReport",
    "DoctorSource",
    "McpClientPresence",
    "DoctorReport",
    "ExtractWorkerStatus",
    "ExtractWorkerCheck",
    "UnreadableFiles",
    "AiDoctor",
    "AiProviderCheck",
    "UpdateChannel",
    "UpdatePrefs",
    "WhatsNewTopic",
    "WhatsNew",
    "StartupTasks",
    "UpdateCheckOutcome",
    "UpdateCheckRecord",
    "ActivityKind",
    "ActivityItem",
    "Activity",
    // AI (v0.3 M1)
    "AiFeature",
    "BlockReason",
    "ModelErrorKind",
    "Effort",
    "MaterialSharing",
    "AiStatus",
    "AiBackendStatus",
    "BackendRef",
    "BackendKind",
    "BackendState",
    "BackendProblem",
    "ModelChoice",
    "FeatureRouting",
    "BudgetStatus",
    "DisclosureFacts",
    "SentData",
    "Recipient",
    "TrainingFact",
    "RetentionFact",
    "CostKind",
    "ProviderWire",
    "ProviderPreset",
    "ModelProviderRecord",
    "LocalServer",
    "AdminVisibility",
    "CodexRuntimeState",
    "CodexSource",
    "CodexRuntime",
    "CodexOutdatedAction",
    "CodexLoginState",
    "ChatGptPlanType",
    "CodexLogin",
    "SystemCodex",
    "CodexStatus",
    "RuntimeEvent",
    "CodexLoginMethod",
    "LoginEvent",
    "ModeAUsage",
    "LocalServerKind",
    "ModelInfo",
    "StructuredOutputTier",
    "ProbeReport",
    "EstimateRequest",
    "CostEstimate",
    "TokenUsage",
    "UsageRow",
    "CostBasis",
    "UsageSummary",
    "RemoveAiDataReport",
    "GenStage",
    "GenNoticeCode",
    "GenEvent",
    "GenerationMeta",
    "ContextSummary",
    "ContextCourse",
    "LeftOutMaterial",
    "LeftOutReason",
    // Course lifecycle and removal (v0.3 M0.10)
    "LifecycleSummary",
    "CourseLifecycleEntry",
    "RemovalReason",
    "LostAfterPurge",
    "BackupInfo",
    "RemovalPreviewItem",
    "RemovalPreview",
    "RemoveOptions",
    "TombstoneState",
    "RemovedCourse",
    "RemovalReport",
    "RestoreFailure",
    "RestoreOutcome",
    "PurgeReport",
    "CourseDatesInput",
    "BreakInput",
    "SegmentInput",
    // Course calendar proposals (v0.3 F3)
    "CourseCalendar",
    "CalendarWeek",
    "DateKind",
    "DateEvidence",
    "AlternativeDate",
    "ProposedDate",
    "ConflictCode",
    "CalendarConflict",
    "DropReason",
    "DropCount",
    "ChangeCode",
    "CalendarChange",
    "CalendarProposal",
    "AcceptedCalendar",
    "CandidateReason",
    "CandidateLeftOut",
    "CalendarCandidate",
    "CourseCalendarView",
    "SyllabusOffer",
    "ReadCalendarOptions",
    "CalendarRunOutcome",
    "CalendarBatchEvent",
    // Carried as `PageLampError` (error.rs).
    "AppError",
    "AppErrorKind",
];

#[test]
fn every_schema_type_is_mirrored() {
    let schema = pagelamp_app::json_schema();
    let defs = schema["$defs"]
        .as_object()
        .expect("the facade schema has $defs");
    let missing: Vec<&String> = defs
        .keys()
        .filter(|name| !MIRRORED.contains(&name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "facade types without a UniFFI mirror in crates/pagelamp-ffi/src/types.rs: {missing:?}"
    );
    // And the list is not stale: every name is still a facade type. (`Event` has no schema
    // entry of its own: `Deadline` flattens it.)
    let stale: Vec<&&str> = MIRRORED
        .iter()
        .filter(|name| !defs.contains_key(**name) && **name != "Event")
        .collect();
    assert!(
        stale.is_empty(),
        "no longer in the facade schema: {stale:?}"
    );
}

#[test]
fn sync_request_defaults_match_the_facade() {
    // The literals in the `SyncRequest` mirror's `#[uniffi(default …)]` attributes.
    let facade = SyncRequest::default();
    assert!(!facade.download_files);
    assert_eq!(facade.max_file_mb, 50);
    assert!(facade.only_courses.is_empty());
    let exported = default_sync_request();
    assert_eq!(exported.max_file_mb, facade.max_file_mb);
}

// ----- the exported API, end to end ------------------------------------------------------------

/// A course folder root with two courses, week folders and `course.toml`s whose term started
/// on last week's Monday (so the current week is 2: weeks run Monday to Sunday).
fn course_folder(root: &Path) -> NaiveDate {
    let today = Local::now().date_naive();
    let term_start = today - TimeDelta::days(i64::from(today.weekday().num_days_from_monday()) + 7);
    let write = |relative: &str, text: &str| {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "DEMO101 Intro to Demo Studies/course.toml",
        &format!(
            "code = \"DEMO101\"\nname = \"Intro to Demo Studies\"\nterm_start = {term_start}\n"
        ),
    );
    write(
        "DEMO101 Intro to Demo Studies/Week 1/lecture-01.md",
        "# Lecture 1\n\nPhotosynthesis turns light into chemical energy.\n",
    );
    write(
        "DEMO101 Intro to Demo Studies/Week 2/lecture-02.md",
        "# Lecture 2\n\nThe Calvin cycle fixes carbon dioxide.\n",
    );
    write(
        "DEMO202 Advanced Demo Studies/course.toml",
        &format!("code = \"DEMO202\"\nterm_start = {term_start}\n"),
    );
    write(
        "DEMO202 Advanced Demo Studies/Week 1/notes.txt",
        "Eigenvalues of a symmetric matrix are real.\n",
    );
    term_start
}

#[derive(Default)]
struct Collect {
    events: Mutex<Vec<(SyncEvent, Option<String>)>>,
}

impl SyncObserver for Collect {
    fn on_event(&self, event: SyncEvent) {
        let thread = std::thread::current().name().map(str::to_string);
        self.events.lock().unwrap().push((event, thread));
    }
}

#[test]
fn folder_source_sync_and_views() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let courses = temp.path().join("Courses");
    course_folder(&courses);

    let lamp = block_on(PageLamp::open_with_memory_secrets(path_string(&data))).unwrap();
    assert_eq!(block_on(lamp.data_dir()).unwrap(), path_string(&data));
    assert!(block_on(lamp.db_path()).unwrap().ends_with("pagelamp.db"));

    let source = block_on(lamp.add_folder_source(path_string(&courses), None, None)).unwrap();
    assert_eq!(source.kind, pagelamp_core::model::SourceKind::Folder);
    assert_eq!(source.label, "Courses");

    let observer = Arc::new(Collect::default());
    let summary = block_on(lamp.sync_all(SyncRequest::default(), observer.clone())).unwrap();
    assert!(summary.ok, "{summary:?}");
    assert_eq!(summary.results.len(), 1);
    assert_eq!(summary.results[0].courses, 2);

    let events = observer.events.lock().unwrap();
    assert!(matches!(
        events.first(),
        Some((SyncEvent::SourceStarted { .. }, _))
    ));
    assert!(matches!(
        events.last(),
        Some((SyncEvent::SourceFinished { ok: true, .. }, _))
    ));
    assert!(
        events
            .iter()
            .any(|(e, _)| matches!(e, SyncEvent::Progress { .. }))
    );
    // Delivered on PageLamp's own runtime, not the caller's thread.
    for (_, thread) in events.iter() {
        assert_eq!(thread.as_deref(), Some("pagelamp-rt"));
    }
    drop(events);

    let list = block_on(lamp.list_courses()).unwrap();
    let codes: Vec<_> = list.iter().filter_map(|c| c.course.code.clone()).collect();
    assert_eq!(codes, ["DEMO101", "DEMO202"]);
    assert_eq!(list[0].timeline.current_week, Some(2));

    let overview = block_on(lamp.course_overview("DEMO101".into())).unwrap();
    assert_eq!(overview.course.name, "Intro to Demo Studies");
    let week1 = block_on(lamp.week_materials("DEMO101".into(), Some(1))).unwrap();
    assert_eq!(week1.week, Some(1));
    assert_eq!(week1.materials.len(), 1);
    assert_eq!(week1.available_weeks, [1, 2]);

    let hits = block_on(lamp.search("photosynthesis".into(), None, 10)).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].course_code.as_deref(), Some("DEMO101"));

    assert!(
        block_on(lamp.list_deadlines(None, 7, 0))
            .unwrap()
            .is_empty()
    );
    assert!(block_on(lamp.latest_study_plan()).unwrap().is_none());

    block_on(lamp.set_course_policy("DEMO202".into(), AiPolicy::Prohibited, Some("no AI".into())))
        .unwrap();
    let start = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    block_on(lamp.set_course_term("DEMO202".into(), Some(start), None)).unwrap();
    block_on(lamp.set_course_ai_access("DEMO101".into(), false)).unwrap();
    block_on(lamp.set_course_hidden("DEMO101".into(), true)).unwrap();
    let list = block_on(lamp.list_courses()).unwrap();
    let demo101 = &list
        .iter()
        .find(|c| c.course.code.as_deref() == Some("DEMO101"))
        .unwrap()
        .course;
    let demo202 = &list
        .iter()
        .find(|c| c.course.code.as_deref() == Some("DEMO202"))
        .unwrap()
        .course;
    assert!(demo101.hidden && !demo101.ai_access);
    assert_eq!(demo202.ai_policy, AiPolicy::Prohibited);
    assert_eq!(demo202.term_start, Some(start));

    let status = block_on(lamp.status()).unwrap();
    assert_eq!(status.version, version());
    // DEMO101 is hidden now.
    assert_eq!(
        (status.counts.courses, status.counts.hidden_courses),
        (1, 1)
    );
    assert!(!status.sync_in_progress);
    assert!(status.last_synced_at.is_some());

    let launch =
        block_on(lamp.mcp_launch("/Applications/PageLamp.app/Contents/MacOS/pagelamp".into()))
            .unwrap();
    assert_eq!(launch.args, ["mcp"]);
    assert_eq!(launch.env.get("PAGELAMP_HOME"), Some(&path_string(&data)));
    let configs = block_on(
        lamp.mcp_client_configs("/Applications/PageLamp.app/Contents/MacOS/pagelamp".into()),
    )
    .unwrap();
    assert_eq!(configs.len(), 4);

    assert!(block_on(lamp.last_crash()).unwrap().is_none());
    block_on(lamp.clear_last_crash()).unwrap();
    let logs = block_on(lamp.logs_dir()).unwrap();
    assert!(Path::new(&logs).is_dir());

    // Removing is allowed now (no sync running) and takes the courses with it.
    block_on(lamp.remove_source(source.id.clone())).unwrap();
    assert!(block_on(lamp.list_courses()).unwrap().is_empty());
    assert!(block_on(lamp.list_sources()).unwrap().is_empty());
}

#[test]
fn errors_map_to_cases() {
    let temp = tempfile::tempdir().unwrap();
    let lamp = block_on(PageLamp::open_with_memory_secrets(path_string(temp.path()))).unwrap();
    assert!(matches!(
        block_on(lamp.course_overview("NOPE999".into())),
        Err(PageLampError::NotFound { .. })
    ));
    assert!(matches!(
        block_on(lamp.remove_source("folder:unknown".into())),
        Err(PageLampError::NotFound { .. })
    ));
    assert!(matches!(
        block_on(lamp.add_folder_source(path_string(&temp.path().join("missing")), None, None)),
        Err(PageLampError::Invalid { .. })
    ));
    assert!(matches!(
        block_on(lamp.sync_source(
            "folder:unknown".into(),
            SyncRequest::default(),
            Arc::new(Collect::default())
        )),
        Err(PageLampError::NotFound { .. })
    ));
    // Opening a data dir that is a file.
    let file = temp.path().join("not-a-dir");
    std::fs::write(&file, "x").unwrap();
    assert!(matches!(
        block_on(PageLamp::open_with_memory_secrets(path_string(&file))),
        Err(PageLampError::Internal { .. })
    ));
}

/// Holds the first sync inside its first event until the test releases it.
struct Gate {
    started: Mutex<Option<mpsc::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl SyncObserver for Gate {
    fn on_event(&self, _event: SyncEvent) {
        if let Some(started) = self.started.lock().unwrap().take() {
            started.send(()).unwrap();
            let _ = self.release.lock().unwrap().recv();
        }
    }
}

#[test]
fn overlapping_syncs_are_busy() {
    let temp = tempfile::tempdir().unwrap();
    let courses = temp.path().join("Courses");
    course_folder(&courses);
    let lamp = block_on(PageLamp::open_with_memory_secrets(path_string(
        &temp.path().join("data"),
    )))
    .unwrap();
    let source = block_on(lamp.add_folder_source(path_string(&courses), None, None)).unwrap();

    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let gate = Arc::new(Gate {
        started: Mutex::new(Some(started_tx)),
        release: Mutex::new(release_rx),
    });
    let first = {
        let lamp = lamp.clone();
        std::thread::spawn(move || block_on(lamp.sync_all(SyncRequest::default(), gate)))
    };
    started_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("first sync started");

    assert!(block_on(lamp.status()).unwrap().sync_in_progress);
    let activity = block_on(lamp.activity()).unwrap();
    assert_eq!(activity.items.len(), 1, "{activity:?}");
    assert!(
        !activity.other_process_syncing,
        "the sync runs in this process"
    );
    assert!(matches!(
        block_on(lamp.sync_all(SyncRequest::default(), Arc::new(Collect::default()))),
        Err(PageLampError::Busy { .. })
    ));
    assert!(matches!(
        block_on(lamp.remove_source(source.id.clone())),
        Err(PageLampError::Busy { .. })
    ));

    release_tx.send(()).unwrap();
    let summary = first.join().unwrap().unwrap();
    assert!(summary.ok);
    assert!(!block_on(lamp.status()).unwrap().sync_in_progress);
    assert!(block_on(lamp.activity()).unwrap().items.is_empty());
}

#[test]
fn update_settings_and_launch_tasks() {
    let temp = tempfile::tempdir().unwrap();
    let lamp = block_on(PageLamp::open_with_memory_secrets(path_string(
        &temp.path().join("data"),
    )))
    .unwrap();
    let prefs = block_on(lamp.update_prefs()).unwrap();
    assert!(prefs.auto_check && prefs.channel.is_none());
    block_on(lamp.set_update_prefs(UpdatePrefs {
        auto_check: true,
        channel: Some(UpdateChannel::Stable),
    }))
    .unwrap();
    assert_eq!(
        block_on(lamp.effective_update_channel()).unwrap(),
        UpdateChannel::Stable
    );

    // A fresh install: no What's new, and no check before the student saw the disclosure.
    let now = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
    let tasks = block_on(lamp.startup_tasks(now)).unwrap();
    assert!(tasks.whats_new.is_none() && tasks.updated_from.is_none());
    assert!(!tasks.update_check_due);
    block_on(lamp.acknowledge_update_disclosure()).unwrap();
    assert!(block_on(lamp.startup_tasks(now)).unwrap().update_check_due);

    let record = UpdateCheckRecord {
        at: now,
        channel: UpdateChannel::Stable,
        outcome: pagelamp_app::UpdateCheckOutcome::Available {
            version: "0.3.0".into(),
        },
    };
    block_on(lamp.record_update_check(record.clone())).unwrap();
    assert_eq!(block_on(lamp.last_update_check()).unwrap(), Some(record));
    assert!(!block_on(lamp.startup_tasks(now)).unwrap().update_check_due);
    let tomorrow = now + TimeDelta::hours(24);
    assert!(
        block_on(lamp.startup_tasks(tomorrow))
            .unwrap()
            .update_check_due
    );
    block_on(lamp.acknowledge_whats_new()).unwrap();

    let activity = block_on(lamp.activity()).unwrap();
    assert!(activity.items.is_empty() && !activity.other_process_syncing);
}

#[test]
fn course_lifecycle_calls_and_constants() {
    let temp = tempfile::tempdir().unwrap();
    let courses = temp.path().join("Courses");
    course_folder(&courses);
    let lamp = block_on(PageLamp::open_with_memory_secrets(path_string(
        &temp.path().join("data"),
    )))
    .unwrap();
    block_on(lamp.add_folder_source(path_string(&courses), None, None)).unwrap();
    let observer = Arc::new(Collect::default());
    assert!(
        block_on(lamp.sync_all(SyncRequest::default(), observer))
            .unwrap()
            .ok
    );

    assert_eq!(not_now_days(), 14);
    assert_eq!(keep_current_days(), 120);
    assert_eq!(
        keep_forever(),
        NaiveDate::from_ymd_opt(9999, 12, 31).unwrap()
    );

    let timeline = block_on(lamp.course_timeline("DEMO101".into())).unwrap();
    assert_eq!(timeline.current_week, Some(2));
    let summary = block_on(lamp.lifecycle_summary()).unwrap();
    assert_eq!(summary.courses.len(), 2);
    assert!(summary.suggested.is_empty() && !summary.show_banner);

    let kept_until = |lamp: &PageLamp, id: &str| {
        block_on(lamp.lifecycle_summary())
            .unwrap()
            .courses
            .into_iter()
            .find(|entry| entry.course_id == id)
            .unwrap()
            .lifecycle
            .kept_current_until
    };
    let until = Local::now().date_naive() + TimeDelta::days(30);
    let kept = block_on(lamp.keep_course_current("DEMO202".into(), Some(until))).unwrap();
    assert_eq!(kept.code.as_deref(), Some("DEMO202"));
    assert_eq!(kept_until(&lamp, &kept.id), Some(until));
    block_on(lamp.clear_keep_course_current("DEMO202".into())).unwrap();
    assert_eq!(kept_until(&lamp, &kept.id), None);

    // Current courses aren't suggested; the calls still reach the facade.
    let ids = vec![kept.id.clone()];
    block_on(lamp.snooze_removal_suggestions(ids.clone(), SnoozeKind::Keep)).unwrap();
    block_on(lamp.clear_removal_snooze(ids)).unwrap();
    block_on(lamp.snooze_lifecycle_banner()).unwrap();
    let confirmed = block_on(lamp.confirm_course_dates("DEMO101".into())).unwrap();
    assert_eq!(confirmed.current_week, Some(2));
    assert!(matches!(
        block_on(lamp.course_timeline("NOPE999".into())),
        Err(PageLampError::NotFound { .. })
    ));
}

#[test]
fn the_mac_app_has_its_own_whats_new() {
    // A folder used before (0.1 recorded no version), then opened by the Mac app.
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let courses = temp.path().join("Courses");
    course_folder(&courses);
    let desktop = || {
        pagelamp_app::App::open_at_with_secrets(data.clone(), Arc::new(MemorySecrets::new()))
            .unwrap()
    };
    desktop().add_folder_source(&courses, None, None).unwrap();
    let now = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();

    let lamp = block_on(PageLamp::open_with_memory_secrets(path_string(&data))).unwrap();
    let tasks = block_on(lamp.startup_tasks(now)).unwrap();
    assert!(tasks.whats_new.is_none(), "the Mac app's first run");
    // The desktop app's update from 0.1 is still there.
    assert!(desktop().startup_tasks(now).unwrap().whats_new.is_some());
}
