//! One Codex run against the test-only `fake-codex` binary (`src/bin/fake-codex.rs`): the
//! argv, environment and stdin PageLamp gives `codex exec`, and how it reads the JSONL answer
//! (plan M2 DoD 1). No real Codex and no network.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pagelamp_core::ai::{Effort, ModelErrorKind};
use pagelamp_core::ai_gate::RenderedPrompt;
use pagelamp_llm::CancellationToken;
use pagelamp_llm::codex::exec::{Exec, ExecRequest};
use pagelamp_llm::codex::{CodexError, CodexHome, pin};
use serde_json::{Value, json};

fn fake() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fake-codex"))
}

/// An `Exec` whose fake Codex answers `script` (keyed `exec`, or `exec:<model>`).
fn exec_with(script: Value) -> (tempfile::TempDir, Exec) {
    let temp = tempfile::tempdir().unwrap();
    let home = CodexHome::new(temp.path().join("codex-home"));
    std::fs::create_dir_all(home.dir()).unwrap();
    std::fs::write(home.dir().join("fake-codex.json"), script.to_string()).unwrap();
    let exec = Exec {
        binary: fake(),
        home,
        runs_dir: temp.path().join("ai-runs"),
    };
    (temp, exec)
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

fn jsonl(events: &[Value]) -> Vec<String> {
    events.iter().map(Value::to_string).collect()
}

fn answer(text: &str) -> Vec<String> {
    jsonl(&[
        json!({"type": "thread.started", "thread_id": "demo"}),
        json!({"type": "turn.started"}),
        json!({"type": "item.completed", "item": {"id": "r1", "type": "reasoning", "text": "thinking"}}),
        json!({"type": "item.completed", "item": {"id": "m1", "type": "agent_message", "text": text}}),
        json!({"type": "turn.completed", "usage": {"input_tokens": 1200, "cached_input_tokens": 200, "cache_write_input_tokens": 0, "output_tokens": 300, "reasoning_output_tokens": 100}}),
    ])
}

fn prompt() -> RenderedPrompt {
    RenderedPrompt::for_tests(
        "Explain only from the course text.",
        "Course: DEMO101\nWeek 3: stomata open in light.",
    )
}

fn request<'a>(prompt: &'a RenderedPrompt, model: &'a str) -> ExecRequest<'a> {
    ExecRequest {
        prompt,
        model,
        effort: Effort::Lowest,
        output_schema: None,
        run_id: "run-1",
    }
}

#[tokio::test]
async fn a_run_names_its_model_sends_the_prompt_on_stdin_and_reads_the_answer() {
    let (temp, exec) = exec_with(json!({ "exec": { "stdout": answer("{\"ok\":true}") } }));
    let prompt = prompt();
    let schema =
        json!({"type": "object", "properties": {"ok": {"type": "boolean"}}, "required": ["ok"]});
    let mut req = request(&prompt, "gpt-6-luna");
    req.output_schema = Some(&schema);
    let outcome = exec
        .run_with_fallback(&req, None, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(outcome.text, "{\"ok\":true}");
    assert_eq!(outcome.model, "gpt-6-luna");
    assert_eq!(
        (
            outcome.usage.input_uncached,
            outcome.usage.cache_read,
            outcome.usage.output,
            outcome.usage.reasoning
        ),
        (1000, 200, 300, Some(100))
    );

    let run = &observed(exec.home.dir())[0];
    // The prompt went on stdin (the golden: exactly the user part) and never into argv.
    assert_eq!(run["stdin"], prompt.user_text());
    let args: Vec<String> = serde_json::from_value(run["args"].clone()).unwrap();
    assert!(args.iter().all(|a| !a.contains("stomata")));
    assert_eq!(args.last().map(String::as_str), Some("-"));
    // An explicit model from the pin's supported list; tools off; no -o file.
    let model = args[args.iter().position(|a| a == "--model").unwrap() + 1].clone();
    assert!(pin().is_supported_model(&model));
    for flag in [
        "--json",
        "--ephemeral",
        "--strict-config",
        "--skip-git-repo-check",
        "--output-schema",
    ] {
        assert!(args.contains(&flag.to_string()), "{flag}");
    }
    assert!(args.windows(2).any(|w| w == ["--sandbox", "read-only"]));
    assert!(args.contains(&"features.shell_tool=false".to_string()));
    // Codex gets PageLamp's model catalog: no code mode, no shell, for every pinned model.
    let catalog_arg = args
        .iter()
        .find_map(|a| a.strip_prefix("model_catalog_json="))
        .unwrap();
    let catalog_path = toml::from_str::<toml::Table>(&format!("v = {catalog_arg}")).unwrap()["v"]
        .as_str()
        .unwrap()
        .to_string();
    let catalog: Value =
        serde_json::from_str(&std::fs::read_to_string(&catalog_path).unwrap()).unwrap();
    assert!(
        catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["tool_mode"] == "direct" && m["shell_type"] == "disabled")
    );
    assert!(
        !args
            .iter()
            .any(|a| a == "-o" || a == "--output-last-message")
    );
    // The environment: PageLamp's CODEX_HOME, no keys.
    let env: Vec<String> = serde_json::from_value(run["env"].clone()).unwrap();
    assert!(env.contains(&"CODEX_HOME".to_string()));
    for secret in ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"] {
        assert!(!env.contains(&secret.to_string()), "{secret}");
    }
    // It ran in an empty per-run folder, deleted afterwards.
    assert!(run["cwd"].as_str().unwrap().ends_with("run-1"));
    assert!(!temp.path().join("ai-runs/run-1").exists());
}

#[tokio::test]
async fn a_tool_item_trips_the_wire_and_the_run_is_discarded() {
    let mut stdout = answer("never shown");
    stdout.insert(2, json!({"type": "item.started", "item": {"id": "c1", "type": "command_execution", "command": "ls", "aggregated_output": "", "exit_code": null, "status": "in_progress"}}).to_string());
    let (_temp, exec) = exec_with(json!({ "exec": { "stdout": stdout, "sleep_ms": 30000 } }));
    let prompt = prompt();
    let started = Instant::now();
    let err = exec
        .run_with_fallback(
            &request(&prompt, "gpt-6-luna"),
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, CodexError::Stopped), "{err}");
    assert_eq!(
        err.to_string(),
        "PageLamp stopped Codex: unexpected output — update PageLamp"
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "killed, not waited for"
    );

    // Anything this version doesn't know stops the run too: an item type, or an event.
    for unexpected in [
        json!({"type": "item.started", "item": {"id": "x1", "type": "image_view", "path": "/demo"}}),
        json!({"type": "session.configured", "tools": ["exec"]}),
    ] {
        let mut stdout = answer("never shown");
        stdout.insert(2, unexpected.to_string());
        let (_temp, exec) = exec_with(json!({ "exec": { "stdout": stdout } }));
        let err = exec
            .run_with_fallback(
                &request(&prompt, "gpt-6-luna"),
                None,
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CodexError::Stopped), "{unexpected}: {err}");
    }
}

#[tokio::test]
async fn errors_are_classified_and_a_retired_default_falls_back_once() {
    let failed =
        |message: &str| jsonl(&[json!({"type": "turn.failed", "error": {"message": message}})]);
    // "requires a newer version of Codex" is RuntimeOutdated and never retried.
    let (_temp, exec) = exec_with(
        json!({ "exec": { "stdout": failed("unexpected status 400 Bad Request: The 'gpt-6-astra' model requires a newer version of Codex. Please upgrade to the latest app or CLI and try again."), "exit": 1 } }),
    );
    let prompt = prompt();
    let err = exec
        .run_with_fallback(
            &request(&prompt, "gpt-6-astra"),
            Some("gpt-5.6-luna"),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            CodexError::Failed {
                kind: ModelErrorKind::RuntimeOutdated
            }
        ),
        "{err}"
    );
    assert!(
        !err.to_string().contains("gpt-6-astra"),
        "Codex's words stay out of errors"
    );
    assert_eq!(observed(exec.home.dir()).len(), 1);

    // model_not_found for the default → the pin's fallback, once.
    let (_temp, exec) = exec_with(json!({
        "exec:gpt-6-luna": { "stdout": failed("model_not_found: The model `gpt-6-luna` does not exist"), "exit": 1 },
        "exec:gpt-5.6-luna": { "stdout": answer("from the fallback") }
    }));
    let outcome = exec
        .run_with_fallback(
            &request(&prompt, "gpt-6-luna"),
            Some("gpt-5.6-luna"),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        (outcome.text.as_str(), outcome.model.as_str()),
        ("from the fallback", "gpt-5.6-luna")
    );
    assert_eq!(observed(exec.home.dir()).len(), 2);

    for (message, kind) in [
        (
            "You've hit your usage limit. Upgrade to Pro or try again later.",
            ModelErrorKind::UsageLimit,
        ),
        (
            "Not logged in. Run codex login",
            ModelErrorKind::NotSignedIn,
        ),
        (
            "stream disconnected before completion: error sending request",
            ModelErrorKind::Network,
        ),
    ] {
        let (_temp, exec) = exec_with(json!({ "exec": { "stdout": failed(message), "exit": 1 } }));
        let err = exec
            .run_with_fallback(
                &request(&prompt, "gpt-6-luna"),
                None,
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, CodexError::Failed { kind: k } if k == kind),
            "{message}: {err}"
        );
    }
    // Exit 0 without an answer.
    let (_temp, exec) = exec_with(json!({ "exec": { "stdout": [] } }));
    let err = exec
        .run_with_fallback(
            &request(&prompt, "gpt-6-luna"),
            None,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            CodexError::Failed {
                kind: ModelErrorKind::BadOutput
            }
        ),
        "{err}"
    );
}

#[tokio::test]
async fn a_run_can_be_cancelled_and_holds_the_lock_meanwhile() {
    let (_temp, exec) = exec_with(json!({ "exec": { "stdout": [], "sleep_ms": 30000 } }));
    let cancel = CancellationToken::new();
    let task = {
        let (exec, cancel) = (exec.clone(), cancel.clone());
        tokio::spawn(async move {
            let prompt = prompt();
            exec.run_with_fallback(&request(&prompt, "gpt-6-luna"), None, &cancel)
                .await
        })
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while observed(exec.home.dir()).is_empty() {
        assert!(Instant::now() < deadline, "the run never started");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // A second run (any window) is Busy, never parallel.
    let prompt = prompt();
    assert!(matches!(
        exec.run_with_fallback(
            &request(&prompt, "gpt-6-luna"),
            None,
            &CancellationToken::new()
        )
        .await,
        Err(CodexError::Busy)
    ));
    let started = Instant::now();
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(CodexError::Cancelled)));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "SIGINT, then a kill after 3 s"
    );
    released(|| exec.home.is_locked()).await;
}

#[tokio::test]
async fn stdin_is_closed_so_a_codex_that_reads_to_the_end_finishes() {
    // The fake reads stdin to the end, like `codex exec` (which otherwise prints "Reading
    // additional input from stdin..." and waits).
    let (_temp, exec) =
        exec_with(json!({ "exec": { "stdout": answer("done"), "wait_for_stdin_eof": true } }));
    let prompt = prompt();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        exec.run_with_fallback(
            &request(&prompt, "gpt-6-luna"),
            None,
            &CancellationToken::new(),
        ),
    )
    .await
    .expect("stdin was closed")
    .unwrap();
    assert_eq!(outcome.text, "done");
}

/// Every file under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

/// The canary check of design §4.4 for Codex runs: after a run, no file PageLamp keeps for Codex
/// — `CODEX_HOME` (the config, the catalog, and whatever Codex adds there: its `*.sqlite`
/// databases with their `-wal`/`-shm` files) and the per-run folders — holds course text. With a
/// real Codex and a real sign-in, owner test A7 greps the same files.
#[tokio::test]
async fn no_course_text_stays_on_disk_after_a_run() {
    const CANARY: &str = "stomatacanary7731";
    let (temp, exec) = exec_with(json!({ "exec": { "stdout": answer("an answer") } }));
    // Codex keeps its own databases in CODEX_HOME; stand-ins, so the scan covers that shape.
    std::fs::write(exec.home.dir().join("state_5.sqlite"), b"SQLite format 3\0").unwrap();
    std::fs::write(exec.home.dir().join("logs_2.sqlite-wal"), b"").unwrap();
    let prompt = RenderedPrompt::for_tests(
        "Explain only from the course text.",
        &format!("Course: DEMO101\nWeek 3: {CANARY} stomata open in light."),
    );
    exec.run_with_fallback(
        &request(&prompt, "gpt-6-luna"),
        None,
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let files = files_under(temp.path());
    assert!(
        files.iter().any(|f| f.ends_with("config.toml")),
        "{files:?}"
    );
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        if name.starts_with("fake-codex") {
            continue; // the test double's own record of what it was given
        }
        let bytes = std::fs::read(&file).unwrap();
        assert!(
            !bytes.windows(CANARY.len()).any(|w| w == CANARY.as_bytes()),
            "course text in {}",
            file.display()
        );
    }
    // The fake did get it, on stdin: the scan would have seen a leak.
    assert!(
        observed(exec.home.dir())[0]["stdin"]
            .as_str()
            .unwrap()
            .contains(CANARY)
    );
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
