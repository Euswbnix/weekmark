//! `pagelamp ai …` and `pagelamp course sharing` end to end, with a temporary PAGELAMP_HOME,
//! synthetic course folders and a local stand-in for LM Studio on 127.0.0.1. Only key-less
//! providers are used, so nothing reads or writes the real OS keychain, and nothing leaves
//! this computer.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use chrono::{SubsecRound, Utc};
use pagelamp_core::ai::ProviderRow;
use pagelamp_core::store::Store;
use serde_json::Value;

/// Like `tests/cli.rs`: every location `pagelamp` could fall back to points into `home`.
fn pagelamp(home: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pagelamp"));
    command.env("PAGELAMP_HOME", home);
    for var in [
        "HOME",
        "USERPROFILE",
        "CODEX_HOME",
        "APPDATA",
        "LOCALAPPDATA",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
    ] {
        command.env(var, home);
    }
    for var in ["RUST_LOG", "PAGELAMP_LOG", "APPIMAGE", "APPDIR"] {
        command.env_remove(var);
    }
    // Piped stdin: `ai use` can't ask, so it needs --yes.
    let child = command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn ok(output: &Output) -> String {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(output),
        stderr(output)
    );
    stdout(output)
}

fn failed(output: &Output) -> String {
    assert!(!output.status.success(), "stdout: {}", stdout(output));
    stderr(output)
}

fn json_out(output: &Output) -> Value {
    serde_json::from_str(&ok(output)).unwrap()
}

/// DEMO101 with a week-1 note, synced from a folder.
fn synced_demo_course(home: &Path, root: &Path) {
    let file = root.join("DEMO101 Intro to Demo Studies/Week 1/notes.md");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "# Basics\nphotosynthesis converts light").unwrap();
    ok(&pagelamp(
        home,
        &[
            "folder",
            "add",
            root.to_str().unwrap(),
            "--term-start",
            "2026-09-07",
        ],
    ));
    ok(&pagelamp(home, &["sync"]));
}

/// A stand-in for LM Studio: lists one model and answers every chat request with `{"ok":true}`.
fn local_model_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).is_err() {
                continue;
            }
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let (content_type, reply) = if request_line.starts_with("GET /v1/models") {
                (
                    "application/json",
                    r#"{"object":"list","data":[{"id":"local-model","object":"model"}]}"#
                        .to_string(),
                )
            } else {
                (
                    "text/event-stream",
                    "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"ok\\\":true}\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n"
                        .to_string(),
                )
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
        }
    });
    format!("http://{address}/v1")
}

#[test]
fn status_presets_budget_and_usage_without_a_provider() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let status = ok(&pagelamp(&home, &["ai"]));
    assert!(status.contains("No model providers yet"), "{status}");
    assert!(
        status.contains("weekly-explanation  no model chosen"),
        "{status}"
    );
    assert!(
        status.contains("Budget: ≈ $0.00 of $5.00 this month."),
        "{status}"
    );
    let presets = ok(&pagelamp(&home, &["ai", "presets"]));
    assert!(
        presets.contains("openai") && presets.contains("ollama") && presets.contains("custom"),
        "{presets}"
    );

    let set = ok(&pagelamp(&home, &["ai", "budget", "2.50"]));
    assert!(set.contains("of $2.50"), "{set}");
    let budget = json_out(&pagelamp(&home, &["--json", "ai", "budget"]));
    assert_eq!(budget["monthly_micro_usd"], 2_500_000);
    ok(&pagelamp(&home, &["ai", "budget", "off"]));
    assert!(ok(&pagelamp(&home, &["ai", "budget"])).contains("Budget: none"));
    assert!(failed(&pagelamp(&home, &["ai", "budget", "five"])).contains("US dollars"));

    assert!(
        ok(&pagelamp(&home, &["ai", "usage", "--month", "2026-08"]))
            .contains("No AI use in 2026-08.")
    );
    let blocked = failed(&pagelamp(
        &home,
        &["ai", "estimate", "--feature", "weekly-note"],
    ));
    assert!(blocked.contains("blocked: no_model_chosen"), "{blocked}");
    assert!(failed(&pagelamp(&home, &["ai", "add", "nope"])).contains("unknown provider type"));
    // Nothing to remove without a provider; --all asks first (and can't, when piped).
    assert!(failed(&pagelamp(&home, &["ai", "remove", "openai"])).contains("no model provider"));
    assert!(failed(&pagelamp(&home, &["ai", "remove", "--all"])).contains("--yes"));
    let removed = json_out(&pagelamp(
        &home,
        &["--json", "ai", "remove", "--all", "--yes"],
    ));
    assert_eq!(removed["providers_removed"], 0);
}

#[test]
fn a_local_server_is_added_tested_chosen_and_removed() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    synced_demo_course(&home, &temp.path().join("Courses"));
    let url = local_model_server();

    let added = json_out(&pagelamp(
        &home,
        &["--json", "ai", "add", "lm_studio", "--base-url", &url],
    ));
    let id = added["provider_id"].as_str().unwrap().to_string();
    assert!(id.starts_with("lm_studio-"), "{added}");
    assert_eq!(added["on_device"], true);
    let models = ok(&pagelamp(&home, &["ai", "models", &id]));
    assert!(models.contains("local-model  on this computer"), "{models}");
    let test = ok(&pagelamp(&home, &["ai", "test", &id, "local-model"]));
    assert!(test.starts_with("OK: answered in"), "{test}");

    // The disclosure comes first, and nothing is saved without consent.
    let asked = pagelamp(
        &home,
        &["ai", "use", "weekly-explanation", &id, "local-model"],
    );
    let shown = failed(&asked);
    assert!(
        shown.contains("runs on this computer: nothing leaves it"),
        "{shown}"
    );
    assert!(shown.contains("Generative AI can be wrong"), "{shown}");
    assert!(ok(&pagelamp(&home, &["ai"])).contains("weekly-explanation  no model chosen"));
    let missing = failed(&pagelamp(
        &home,
        &[
            "ai",
            "use",
            "weekly-explanation",
            &id,
            "no-such-model",
            "--yes",
        ],
    ));
    assert!(
        missing.contains("doesn't list 'no-such-model'"),
        "{missing}"
    );
    ok(&pagelamp(
        &home,
        &[
            "ai",
            "use",
            "weekly-explanation",
            &id,
            "local-model",
            "--yes",
        ],
    ));
    let status = ok(&pagelamp(&home, &["ai"]));
    assert!(status.contains("on this computer · ready"), "{status}");
    assert!(
        status.contains(&format!(
            "weekly-explanation  local-model on {id} (effort lowest)"
        )),
        "{status}"
    );

    // Free on this computer, even for a course whose materials may not go to a cloud service.
    let estimate = [
        "ai",
        "estimate",
        "--feature",
        "weekly-explanation",
        "--course",
        "DEMO101",
        "--week",
        "1",
    ];
    assert!(ok(&pagelamp(&home, &estimate)).starts_with("Free ("));
    let sharing = pagelamp(&home, &["course", "sharing", "DEMO101", "not-allowed"]);
    assert!(stderr(&sharing).contains("won't send this course's material text"));
    ok(&sharing);
    assert!(ok(&pagelamp(&home, &estimate)).starts_with("Free ("));

    ok(&pagelamp(
        &home,
        &["ai", "use", "weekly-explanation", "off"],
    ));
    assert!(failed(&pagelamp(&home, &estimate)).contains("blocked: no_model_chosen"));
    assert!(ok(&pagelamp(&home, &["ai", "remove", &id])).contains("Removed"));
    assert!(ok(&pagelamp(&home, &["ai"])).contains("No model providers yet"));
}

#[test]
fn a_not_allowed_course_is_blocked_for_a_cloud_model_from_the_cli() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    synced_demo_course(&home, &temp.path().join("Courses"));
    // A key-less server that isn't this computer: a cloud model as far as question (b) goes.
    // Written directly, so no request is ever made to it.
    Store::open(&home.join("pagelamp.db"))
        .unwrap()
        .insert_model_provider(&ProviderRow {
            id: "school-llm".into(),
            preset: "lm_studio".into(),
            label: "School LLM".into(),
            wire: "openai_chat".into(),
            base_url: "https://llm.example.invalid/v1".into(),
            created_at: Utc::now().trunc_subsecs(0),
            last_probe_json: None,
        })
        .unwrap();
    let chosen = pagelamp(
        &home,
        &[
            "ai",
            "use",
            "weekly-explanation",
            "school-llm",
            "demo-model",
            "--yes",
            "--no-check",
        ],
    );
    ok(&chosen);
    assert!(stderr(&chosen).contains("It goes to LM Studio under your account"));
    assert!(stderr(&chosen).contains("Costs, if any, are set by whoever runs this server"));
    assert!(stderr(&chosen).contains("If PageLamp doesn't know this model's price"));

    let estimate = [
        "ai",
        "estimate",
        "--feature",
        "weekly-explanation",
        "--course",
        "DEMO101",
        "--week",
        "1",
    ];
    assert!(ok(&pagelamp(&home, &estimate)).starts_with("Price unknown ("));
    ok(&pagelamp(
        &home,
        &["course", "sharing", "DEMO101", "not_allowed"],
    ));
    let blocked = failed(&pagelamp(&home, &estimate));
    assert!(
        blocked.contains("blocked: material_sharing_not_allowed"),
        "{blocked}"
    );
    let mut json = vec!["--json"];
    json.extend(estimate);
    let output = pagelamp(&home, &json);
    failed(&output);
    let body: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(body["would_block"], "material_sharing_not_allowed");
    assert_eq!(body["input_tokens"], 0, "nothing would be sent");
    ok(&pagelamp(
        &home,
        &["course", "sharing", "DEMO101", "not-sure"],
    ));
    ok(&pagelamp(&home, &estimate));
}

/// A build that doesn't offer the ChatGPT plan (`CHATGPT_PLAN_OFFERED`): `ai codex …` refuses
/// with the facade's message before anything is printed or started (status and sign-out, which
/// clean up, still work; they aren't run here because they would look for a `codex` on PATH).
#[test]
fn codex_commands_refuse_when_the_plan_is_not_offered() {
    if pagelamp_app::ai::CHATGPT_PLAN_OFFERED {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    for args in [
        &["ai", "codex", "install"][..],
        &["ai", "codex", "login"],
        &["ai", "codex", "use", "system"],
        &["ai", "codex", "cap", "5"],
    ] {
        let output = pagelamp(&home, args);
        let err = failed(&output);
        assert!(
            err.contains("The ChatGPT plan isn't available in this version of PageLamp"),
            "{args:?}: {err}"
        );
        assert!(
            stdout(&output).is_empty(),
            "{args:?}: nothing printed first"
        );
        assert!(!err.contains("Downloading"), "{args:?}: {err}");
    }
}
