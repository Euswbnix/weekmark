//! IPC contract, Rust half: replays every call the UI makes (recorded by
//! `src/api/tauri.contract.test.ts` into `fixtures/ipc-calls.json`) against the real commands,
//! backed by a real `App` in a temp data dir with in-memory secrets.
//!
//! A call passes when the command ran: it either succeeded or returned a facade `AppError`
//! (`{ kind, message }` — e.g. `not_found` for the made-up course ids). Anything else, such as
//! Tauri's "missing required key" or "command not found" strings, means the UI and the Rust
//! side disagree about the contract.

use std::path::Path;
use std::sync::Arc;

use pagelamp_app::App;
use pagelamp_app::trash::FileTrash;
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_desktop_lib::{Backend, with_commands};
use serde_json::Value;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{INVOKE_KEY, get_ipc_response, mock_builder, mock_context, noop_assets};
use tauri::webview::InvokeRequest;

/// Where the app's own page is served from, the only origin the ACL lets call app commands:
/// Tauri serves it from `http://tauri.localhost` on Windows and `tauri://localhost` elsewhere.
const APP_ORIGIN: &str = if cfg!(windows) {
    "http://tauri.localhost"
} else {
    "tauri://localhost"
};

/// Commands with real side effects on the machine running the tests.
/// (The updater ones would reach the network, install would restart the app, the local server
/// check connects to ports on this computer, and the Codex ones download it or start `codex`,
/// including one the student may have installed.)
const SKIPPED: &[&str] = &[
    "reveal_data_dir",
    "reveal_logs_dir",
    "updates_check",
    "updates_install",
    "detect_local_servers",
    "codex_status",
    "install_codex",
    "codex_login",
    "codex_logout",
];

/// The removal commands' Trash here: moves nothing, so a changed fixture can never reach this
/// computer's real Trash.
struct NoTrash;

impl FileTrash for NoTrash {
    fn trash(&self, _path: &Path) -> Result<(), String> {
        Err("the contract test's Trash moves nothing".into())
    }
}

#[test]
fn every_ui_call_reaches_its_command() {
    let data_dir = tempfile::tempdir().expect("temp dir");
    let facade = App::open_at_with_secrets(
        data_dir.path().to_path_buf(),
        Arc::new(MemorySecrets::new()),
    )
    .expect("open App in a temp dir");
    facade.set_trash(Arc::new(NoTrash));
    let app = with_commands(mock_builder())
        .manage(Backend::from_app(facade))
        .build(mock_context(noop_assets()))
        .expect("build mock app");
    let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("mock webview");

    let calls: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/ipc-calls.json")).expect("fixture JSON");
    assert!(calls.len() > 20, "the fixture should cover every command");

    for call in calls {
        let cmd = call["cmd"].as_str().expect("cmd").to_string();
        if SKIPPED.contains(&cmd.as_str()) {
            continue;
        }
        let response = get_ipc_response(
            &webview,
            InvokeRequest {
                cmd: cmd.clone(),
                callback: CallbackFn(0),
                error: CallbackFn(1),
                url: APP_ORIGIN.parse().expect("url"),
                body: InvokeBody::Json(call["args"].clone()),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        );
        if let Err(error) = response {
            let is_app_error = error.get("kind").and_then(Value::as_str).is_some()
                && error.get("message").and_then(Value::as_str).is_some();
            assert!(is_app_error, "`{cmd}` did not reach the facade: {error}");
        }
    }
}
