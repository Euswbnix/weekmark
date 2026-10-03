//! The desktop's core flow against the real backend, through the same Tauri commands the UI
//! calls: add a (synthetic) course folder → sync → course list → course page → AI access switch
//! → "connect your AI app". Asserts the fields the screens rely on, so a backend change that
//! would blank a screen fails here before anyone opens the window.

use std::path::Path;
use std::sync::Arc;

use pagelamp_app::App;
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_desktop_lib::{Backend, with_commands};
use serde_json::{Value, json};
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{Manager, WebviewWindow};

/// Where the app's own page is served from, the only origin the ACL lets call app commands:
/// Tauri serves it from `http://tauri.localhost` on Windows and `tauri://localhost` elsewhere.
const APP_ORIGIN: &str = if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

fn invoke(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
    get_ipc_response(
        webview,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: APP_ORIGIN.parse().expect("url"),
            body: InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|body| body.deserialize::<Value>().expect("JSON response"))
}

fn ok(webview: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Value {
    invoke(webview, cmd, args).unwrap_or_else(|err| panic!("`{cmd}` failed: {err}"))
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

/// Two synthetic courses with week folders (made-up content only).
fn course_folder(root: &Path) {
    for (dir, topic) in [
        ("DEMO101 Intro to Demo Studies", "sampling frames"),
        ("DEMO205 Foundations of Sample Data", "joining tables"),
    ] {
        write(
            &root.join(dir).join("course.toml"),
            "term_start = 2026-09-01\n",
        );
        for week in 1..=4 {
            write(
                &root
                    .join(dir)
                    .join(format!("Week {week}"))
                    .join(format!("Week {week} notes.md")),
                &format!("# Week {week}\n\nThis week is about {topic}. Synthetic test notes.\n"),
            );
        }
        write(
            &root.join(dir).join("Week 4").join("Lecture recording.mp4"),
            "not a video",
        );
    }
}

#[test]
fn folder_source_flow_through_the_ui_commands() {
    let data = tempfile::tempdir().expect("data dir");
    let folder = tempfile::tempdir().expect("course folder");
    course_folder(folder.path());

    let facade =
        App::open_at_with_secrets(data.path().to_path_buf(), Arc::new(MemorySecrets::new()))
            .expect("open App");
    let app = with_commands(mock_builder())
        .manage(Backend::from_app(facade))
        .build(mock_context(noop_assets()))
        .expect("mock app");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("webview");

    // Onboarding: add the folder (args exactly as src/api/tauri.ts sends them), then sync.
    let source = ok(
        &webview,
        "add_folder_source",
        json!({ "path": folder.path(), "termStart": null, "label": "Smoke test courses" }),
    );
    assert_eq!(source["kind"], "folder");
    assert_eq!(source["label"], "Smoke test courses");

    let summary = ok(
        &webview,
        "sync_all",
        json!({ "req": {}, "onEvent": "__CHANNEL__:0" }),
    );
    assert_eq!(summary["ok"], true, "sync failed: {summary}");
    assert_eq!(summary["results"][0]["courses"], 2);

    // Courses screen.
    let courses = ok(&webview, "list_courses", json!({}));
    let courses = courses.as_array().expect("course list");
    assert_eq!(courses.len(), 2);
    let demo101 = courses
        .iter()
        .find(|c| c["course"]["code"] == "DEMO101")
        .expect("DEMO101 listed");
    assert_eq!(demo101["ai_materials"], "readable");
    assert_eq!(demo101["course"]["term_source"], "synced");
    assert_eq!(demo101["source_label"], "Smoke test courses");
    assert!(demo101["counts"]["materials"].as_u64().unwrap() >= 5);
    assert!(demo101["counts"]["indexed_materials"].as_u64().unwrap() >= 4);
    let course_id = demo101["course"]["id"].as_str().expect("id").to_string();

    // Course page: overview and a week the switcher can show.
    let overview = ok(&webview, "course_overview", json!({ "course": course_id }));
    let name = overview["course"]["name"].as_str().expect("name");
    assert!(
        name.contains("Intro to Demo Studies"),
        "course name: {name}"
    );
    let week = ok(
        &webview,
        "week_materials",
        json!({ "course": course_id, "week": 4 }),
    );
    assert_eq!(week["week"], 4);
    assert!(
        week["available_weeks"]
            .as_array()
            .unwrap()
            .contains(&json!(4))
    );
    let statuses: Vec<&str> = week["materials"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["text_status"].as_str().unwrap())
        .collect();
    assert!(
        statuses.contains(&"ok"),
        "week 4 has readable notes: {statuses:?}"
    );
    assert!(
        statuses.contains(&"unsupported"),
        "the .mp4 is unsupported: {statuses:?}"
    );

    // AI policy tab: the switch, then "No AI" wins over it.
    ok(
        &webview,
        "set_course_ai_access",
        json!({ "course": course_id, "allowed": false }),
    );
    let overview = ok(&webview, "course_overview", json!({ "course": course_id }));
    assert_eq!(overview["ai_materials"], "turned_off");
    ok(
        &webview,
        "set_course_ai_access",
        json!({ "course": course_id, "allowed": true }),
    );
    ok(
        &webview,
        "set_course_policy",
        json!({ "course": course_id, "policy": "prohibited", "note": null }),
    );
    let overview = ok(&webview, "course_overview", json!({ "course": course_id }));
    assert_eq!(overview["ai_materials"], "withheld_by_policy");
    assert_eq!(overview["course"]["ai_access"], true);

    // Hidden courses stay reachable from the desktop (list + course page).
    ok(
        &webview,
        "set_course_hidden",
        json!({ "course": course_id, "hidden": true }),
    );
    ok(&webview, "course_overview", json!({ "course": course_id }));

    // Connect page: every config points at a pagelamp binary and sets the non-default data dir.
    let configs = ok(&webview, "mcp_client_configs", json!({}));
    let configs = configs.as_array().expect("configs");
    assert!(configs.iter().any(|c| c["client"] == "claude_desktop"));
    for config in configs {
        assert_eq!(
            config["notes"].as_array().unwrap().len(),
            config["note_codes"].as_array().unwrap().len(),
            "notes and note codes are paired",
        );
        assert!(config["launch"]["env"]["PAGELAMP_HOME"].is_string());
    }
}

/// Checks the folder that `pnpm run smoke` generates: every file but the videos becomes readable
/// (including its generated PDF). Run with
/// `PAGELAMP_SMOKE_COURSES=<temp>/pagelamp-smoke/Courses cargo test -p pagelamp-desktop -- --ignored`.
#[test]
#[ignore = "needs the folder generated by `pnpm run smoke --no-launch`"]
fn smoke_folder_is_fully_indexed() {
    let Ok(folder) = std::env::var("PAGELAMP_SMOKE_COURSES") else {
        panic!("set PAGELAMP_SMOKE_COURSES to the smoke course folder");
    };
    let data = tempfile::tempdir().expect("data dir");
    let facade =
        App::open_at_with_secrets(data.path().to_path_buf(), Arc::new(MemorySecrets::new()))
            .expect("open App");
    let app = with_commands(mock_builder())
        .manage(Backend::from_app(facade))
        .build(mock_context(noop_assets()))
        .expect("mock app");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("webview");

    ok(
        &webview,
        "add_folder_source",
        json!({ "path": folder, "termStart": null, "label": null }),
    );
    let summary = ok(
        &webview,
        "sync_all",
        json!({ "req": {}, "onEvent": "__CHANNEL__:0" }),
    );
    assert_eq!(summary["ok"], true, "sync failed: {summary}");
    for course in ok(&webview, "list_courses", json!({})).as_array().unwrap() {
        let id = course["course"]["id"].as_str().unwrap();
        let week = ok(
            &webview,
            "week_materials",
            json!({ "course": id, "week": 4 }),
        );
        for m in week["materials"].as_array().unwrap() {
            let title = m["title"].as_str().unwrap();
            let expected = if title.ends_with(".mp4") {
                "unsupported"
            } else {
                "ok"
            };
            assert_eq!(m["text_status"], expected, "{title}: {m}");
        }
        println!(
            "{} — week {}: {} materials",
            course["course"]["code"],
            week["week"],
            week["materials"].as_array().unwrap().len()
        );
    }
}

/// While an update installs (`updates_install` holds the gate), nothing the restart would kill
/// can start: no sync, no file download, no model run, no course removal, restore or purge.
/// Afterwards they run again.
#[test]
fn no_sync_starts_while_an_update_installs() {
    let data = tempfile::tempdir().expect("data dir");
    let folder = tempfile::tempdir().expect("course folder");
    course_folder(folder.path());
    let facade =
        App::open_at_with_secrets(data.path().to_path_buf(), Arc::new(MemorySecrets::new()))
            .expect("open App");
    let app = with_commands(mock_builder())
        .manage(Backend::from_app(facade))
        .build(mock_context(noop_assets()))
        .expect("mock app");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("webview");
    let source = ok(
        &webview,
        "add_folder_source",
        json!({ "path": folder.path(), "termStart": null, "label": "Smoke test courses" }),
    );
    let source_id = source["id"].as_str().expect("source id");
    let course = format!("{source_id}/course/DEMO101");

    let backend = app.state::<Backend>();
    let gate = backend.hold_work_for_install().expect("no install yet");
    for (cmd, args) in [
        ("sync_all", json!({ "req": {}, "onEvent": "__CHANNEL__:0" })),
        (
            "sync_source",
            json!({ "sourceId": source_id, "req": {}, "onEvent": "__CHANNEL__:0" }),
        ),
        (
            "download_course_files",
            json!({ "course": course, "onEvent": "__CHANNEL__:0" }),
        ),
        (
            "download_material_files",
            json!({ "course": course, "materialIds": [], "onEvent": "__CHANNEL__:0" }),
        ),
        (
            "read_course_calendar",
            json!({
                "course": course,
                "generationId": "install-gate-test",
                "options": { "override_budget": false },
                "onEvent": "__CHANNEL__:0",
            }),
        ),
        (
            "read_course_calendars",
            json!({
                "courses": [course],
                "batchId": "install-gate-test",
                "options": { "override_budget": false },
                "onEvent": "__CHANNEL__:0",
            }),
        ),
    ] {
        let err = invoke(&webview, cmd, args).expect_err(cmd);
        assert_eq!(err["kind"], "busy", "`{cmd}`: {err}");
    }
    assert!(
        ok(&webview, "list_courses", json!({}))
            .as_array()
            .expect("course list")
            .is_empty(),
        "nothing synced"
    );

    drop(gate);
    let summary = ok(
        &webview,
        "sync_all",
        json!({ "req": {}, "onEvent": "__CHANNEL__:0" }),
    );
    assert_eq!(summary["ok"], true, "sync failed: {summary}");

    // Removals move files and a restore syncs: refused too, and nothing changes.
    let remove = |course: &str| {
        json!({
            "courses": [course],
            "options": {
                "reason": null,
                "keep_downloaded_files": false,
                "purge_now": false,
                "delete_pre_update_backup": false,
            },
        })
    };
    let report = ok(&webview, "remove_courses", remove("DEMO101"));
    let removed_id = report["removed"][0]["removed_id"]
        .as_str()
        .expect("removed id")
        .to_string();
    let gate = backend.hold_work_for_install().expect("no install yet");
    for (cmd, args) in [
        ("remove_courses", remove("DEMO205")),
        ("restore_course", json!({ "removedId": removed_id })),
        (
            "purge_removed_courses",
            json!({ "removedIds": [removed_id], "permanentIfNoTrash": false }),
        ),
    ] {
        let err = invoke(&webview, cmd, args).expect_err(cmd);
        assert_eq!(err["kind"], "busy", "`{cmd}`: {err}");
    }
    let codes: Vec<Value> = ok(&webview, "list_courses", json!({}))
        .as_array()
        .expect("course list")
        .iter()
        .map(|summary| summary["course"]["code"].clone())
        .collect();
    assert_eq!(
        codes,
        [json!("DEMO205")],
        "DEMO205 kept, DEMO101 still removed"
    );
    let removed = ok(&webview, "removed_courses", json!({}));
    assert_eq!(removed[0]["state"], "pending", "not purged: {removed}");
    drop(gate);
    let restored = ok(
        &webview,
        "restore_course",
        json!({ "removedId": removed_id }),
    );
    assert_eq!(restored["restored"], true, "{restored}");
}
