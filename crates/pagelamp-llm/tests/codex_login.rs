//! Codex sign-in, status, sign-out and version against the test-only `fake-codex` binary
//! (`src/bin/fake-codex.rs`): what PageLamp starts, with which environment and stdin, and how it
//! reads the answers. No real Codex and no network.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pagelamp_core::ai::ModelErrorKind;
use pagelamp_llm::CancellationToken;
use pagelamp_llm::codex::{
    CodexError, CodexHome, LoginEvent, LoginMethod, LoginState, Version, login,
};
use serde_json::{Value, json};

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-codex"))
}

/// A `CODEX_HOME` whose fake Codex answers `script`.
fn home_with(script: Value) -> (tempfile::TempDir, CodexHome) {
    let temp = tempfile::tempdir().unwrap();
    let home = CodexHome::new(temp.path().join("codex-home"));
    std::fs::create_dir_all(home.dir()).unwrap();
    std::fs::write(home.dir().join("fake-codex.json"), script.to_string()).unwrap();
    (temp, home)
}

/// What the fake Codex recorded, one JSON line per run. Only whole lines: a run still writing
/// (or killed while writing) can leave a last line without its newline.
fn observed(home: &Path) -> Vec<Value> {
    std::fs::read_to_string(home.join("fake-codex-observed.jsonl"))
        .unwrap_or_default()
        .split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .map(|line| serde_json::from_str(line.trim_end()).unwrap())
        .collect()
}

#[tokio::test]
async fn the_browser_sign_in_shows_codexs_link_and_waits() {
    let (_temp, home) = home_with(json!({
        "login": {
            "stderr": [
                "Starting local login server on http://localhost:1455.",
                "If your browser did not open, navigate to this URL to authenticate:",
                "",
                "https://auth.openai.com/oauth/authorize?state=demo",
                "Successfully logged in"
            ],
            "sleep_ms": 50
        }
    }));
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    login::login(
        &fake(),
        &home,
        LoginMethod::Browser,
        &CancellationToken::new(),
        &move |e| seen.lock().unwrap().push(e),
    )
    .await
    .unwrap();
    assert_eq!(
        *events.lock().unwrap(),
        [
            LoginEvent::BrowserOpened {
                url: Some("https://auth.openai.com/oauth/authorize?state=demo".into())
            },
            LoginEvent::Waiting,
            LoginEvent::Done
        ]
    );
    let runs = observed(home.dir());
    assert_eq!(runs[0]["args"], json!(["login"]));
    // The login command gets PageLamp's config and a null stdin, and never a key.
    assert!(
        std::fs::read_to_string(home.dir().join("config.toml"))
            .unwrap()
            .contains("shell_tool = false")
    );
    assert_eq!(runs[0]["stdin"], "");
    let env: Vec<String> = serde_json::from_value(runs[0]["env"].clone()).unwrap();
    assert!(env.contains(&"CODEX_HOME".to_string()));
    for secret in ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"] {
        assert!(!env.contains(&secret.to_string()), "{secret}");
    }
    assert!(env.iter().all(|name| !name.starts_with("PAGELAMP_SECRET_")));
}

#[tokio::test]
async fn the_device_code_sign_in_and_its_failure() {
    let (_temp, home) = home_with(json!({
        "login --device-auth": {
            "stdout": [
                "1. Open this link in your browser and sign in to your account",
                "   \u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m",
                "2. Enter this one-time code \u{1b}[90m(expires in 15 minutes)\u{1b}[0m",
                "   \u{1b}[94mDEMO-1234\u{1b}[0m"
            ],
            "stderr": ["Error logging in with device code: demo failure"],
            "exit": 1
        }
    }));
    let events = Arc::new(Mutex::new(Vec::new()));
    let seen = events.clone();
    let err = login::login(
        &fake(),
        &home,
        LoginMethod::DeviceCode,
        &CancellationToken::new(),
        &move |e| seen.lock().unwrap().push(e),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(
            err,
            CodexError::Failed {
                kind: ModelErrorKind::AuthRejected
            }
        ),
        "{err}"
    );
    // Codex's own words never reach the error.
    assert!(!err.to_string().contains("demo failure"));
    assert_eq!(
        events.lock().unwrap()[0],
        LoginEvent::DeviceCode {
            verification_url: "https://auth.openai.com/codex/device".into(),
            user_code: "DEMO-1234".into(),
            expires_in_secs: Some(900),
        }
    );
    assert_eq!(
        observed(home.dir())[0]["args"],
        json!(["login", "--device-auth"])
    );
}

#[tokio::test]
async fn a_sign_in_can_be_cancelled_and_holds_the_lock_meanwhile() {
    let (_temp, home) = home_with(json!({
        "login": { "stderr": ["navigate to this URL to authenticate:", "https://auth.openai.com/x"], "sleep_ms": 30000 }
    }));
    let cancel = CancellationToken::new();
    let task = {
        let (home, cancel) = (home.clone(), cancel.clone());
        tokio::spawn(async move {
            login::login(&fake(), &home, LoginMethod::Browser, &cancel, &|_| {}).await
        })
    };
    // While the sign-in runs, every other Codex command in any window is Busy.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !home.is_locked() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the sign-in never started"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(matches!(
        login::status(&fake(), &home).await,
        Err(CodexError::Busy)
    ));
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(CodexError::Cancelled)));
    released(|| home.is_locked()).await;
}

#[tokio::test]
async fn status_sign_out_and_version() {
    let (_temp, home) = home_with(json!({
        "login status": { "stderr": ["Logged in using ChatGPT"] },
        "logout": { "stderr": ["Successfully logged out"] },
        "--version": { "stdout": ["codex-cli 0.158.0"] }
    }));
    assert_eq!(
        login::status(&fake(), &home).await.unwrap(),
        LoginState::ChatGpt
    );
    login::logout(&fake(), &home).await.unwrap();
    assert_eq!(
        login::version(&fake(), &home).await.unwrap(),
        Version::parse("0.158.0").unwrap()
    );
    let runs = observed(home.dir());
    let args: Vec<Value> = runs.iter().map(|r| r["args"].clone()).collect();
    assert_eq!(
        args,
        [
            json!(["login", "status"]),
            json!(["logout"]),
            json!(["--version"])
        ]
    );
    assert!(
        runs.iter().all(|r| r["stdin"] == ""),
        "a null stdin for every non-exec command"
    );

    let (_temp, home) = home_with(json!({
        "login status": { "stderr": ["Logged in using an API key - sk-demo***"] }
    }));
    assert_eq!(
        login::status(&fake(), &home).await.unwrap(),
        LoginState::ApiKey
    );
    let (_temp, home) =
        home_with(json!({ "login status": { "stderr": ["Not logged in"], "exit": 1 } }));
    assert_eq!(
        login::status(&fake(), &home).await.unwrap(),
        LoginState::SignedOut
    );

    let missing = Path::new("/nonexistent/pagelamp-demo/codex");
    assert!(matches!(
        login::status(missing, &home).await,
        Err(CodexError::Start(_))
    ));
}

/// The lock is released (a child another test thread is starting can hold a just-released
/// `flock` for a moment, so poll briefly).
async fn released(is_locked: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while is_locked() {
        assert!(
            std::time::Instant::now() < deadline,
            "the lock is still held"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
