//! `pagelamp ai …` and `pagelamp course sharing` end to end, with a temporary PAGELAMP_HOME,
//! synthetic course folders and a local stand-in for LM Studio on 127.0.0.1. Only key-less
//! providers are used, so nothing reads or writes the real OS keychain, and nothing leaves
//! this computer.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{SubsecRound, Utc};
use pagelamp_app::App;
use pagelamp_app::ai::{BackendRef, CodexSource, ModelChoice};
use pagelamp_core::ai::{AiFeature, Effort, ProviderRow};
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_core::store::Store;
use serde_json::Value;

/// Like `tests/cli.rs`: every location `pagelamp` could fall back to points into `home`.
fn pagelamp(home: &Path, args: &[&str]) -> Output {
    pagelamp_with_path(home, args, None)
}

/// `pagelamp` with `PATH` set to `path` when given.
fn pagelamp_with_path(home: &Path, args: &[&str], path: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pagelamp"));
    if let Some(path) = path {
        command.env("PATH", path);
    }
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
    // `explain` itself is refused before anything is sent (M3 DoD 1, the CLI entry point).
    let refused = failed(&pagelamp(&home, &["explain", "DEMO101", "--week", "1"]));
    assert!(
        refused.contains("blocked: material_sharing_not_allowed"),
        "{refused}"
    );
    ok(&pagelamp(
        &home,
        &["course", "sharing", "DEMO101", "not-sure"],
    ));
    ok(&pagelamp(&home, &estimate));
}

#[test]
fn remind_is_quiet_until_something_is_due_and_plan_and_note_need_a_model() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    synced_demo_course(&home, &temp.path().join("Courses"));
    // Nothing due (the digest's Monday reminder may be, so ask on a fresh week only when it
    // isn't): `--digest` always prints the digest.
    let digest = ok(&pagelamp(&home, &["remind", "--digest"]));
    assert!(digest.contains("This week"), "{digest}");
    assert!(digest.contains("DEMO101"), "{digest}");
    let output = pagelamp(&home, &["--json", "remind", "--digest"]);
    ok(&output);
    let body = json_out(&output);
    assert!(
        body["due"].is_array() && body["digest"]["courses"].is_array(),
        "{body}"
    );
    // Shown reminders don't come back.
    let again = pagelamp(&home, &["--json", "remind"]);
    ok(&again);
    assert_eq!(json_out(&again)["due"], serde_json::json!([]));

    let plan = failed(&pagelamp(&home, &["plan"]));
    assert!(plan.contains("blocked: no_model_chosen"), "{plan}");
    assert!(
        ok(&pagelamp(&home, &["explain", "DEMO101", "--saved"])).contains("No saved explanations.")
    );
    let note = failed(&pagelamp(&home, &["note"]));
    assert!(note.contains("blocked: no_model_chosen"), "{note}");
    assert!(ok(&pagelamp(&home, &["note", "--saved"])).contains("No saved weekly notes."));
}

/// PageLamp runs the ChatGPT and Claude plans only when the student starts the run (plan
/// D27): with no terminal (cron, a script; a spawned test has none), the model commands refuse
/// them first, before any other check. A Codex on PATH that leaves a mark if anything starts it guards that
/// nothing was run. An API key or a local model is not refused.
#[test]
fn plan_runs_without_a_terminal_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    synced_demo_course(&home, &temp.path().join("Courses"));
    let choice = |backend| ModelChoice {
        backend,
        model: "plan-model".into(),
        effort: Effort::Lowest,
    };
    let mut routing = BTreeMap::from([
        (AiFeature::StudyPlan, choice(BackendRef::Codex)),
        (AiFeature::WeeklyExplanation, choice(BackendRef::ClaudeCode)),
        (AiFeature::WeeklyNote, choice(BackendRef::Codex)),
        (AiFeature::CourseCalendar, choice(BackendRef::ClaudeCode)),
    ]);
    let store = Store::open(&home.join("pagelamp.db")).unwrap();
    store.set_setting("ai.routing", &routing).unwrap();
    // The student's own Codex on PATH, its disclosure read: the refusal is what stops the run.
    store
        .set_setting("ai.codex_source", &CodexSource::System)
        .unwrap();
    let app = App::open_at_with_secrets(home.clone(), Arc::new(MemorySecrets::new())).unwrap();
    // The ChatGPT plan's build switch is off: acknowledging needs it on (in this process only;
    // the spawned CLI refuses before it would matter).
    app.set_chatgpt_plan_offered_for_tests(true);
    let version = app
        .ai_status()
        .unwrap()
        .backends
        .iter()
        .find(|b| b.backend == BackendRef::Codex)
        .unwrap()
        .disclosure
        .version;
    app.acknowledge_ai_disclosure(&BackendRef::Codex, version)
        .unwrap();
    let bin = temp.path().join("bin");
    let path = cfg!(unix).then_some(bin.as_path());
    std::fs::create_dir_all(&bin).unwrap();
    let mark = temp.path().join("codex-ran");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let codex = bin.join("codex");
        // Shell builtins only: PATH is just `bin`, so `touch` would never be found.
        std::fs::write(
            &codex,
            format!("#!/bin/sh\necho \"$*\" >> '{}'\nexit 1\n", mark.display()),
        )
        .unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    for args in [
        &["plan"][..],
        &["explain", "DEMO101"],
        &["note"],
        &["course", "calendar", "DEMO101", "--read"],
        &["--json", "note"],
    ] {
        let err = failed(&pagelamp_with_path(&home, args, path));
        assert!(
            err.contains("refused: unattended_plan_run"),
            "{args:?}: {err}"
        );
        assert!(
            err.contains(
                "PageLamp runs the ChatGPT and Claude plans only when you start the run yourself."
            ),
            "{args:?}: {err}"
        );
        assert!(
            err.contains(
                "For scheduled runs (cron), choose an API key or a model on this computer"
            ),
            "{args:?}: {err}"
        );
    }
    assert!(!mark.exists(), "no Codex was started");
    // What runs no model is unaffected.
    assert!(
        ok(&pagelamp_with_path(&home, &["note", "--saved"], path))
            .contains("No saved weekly notes.")
    );

    // An API key or a local model may run on a schedule (here the provider doesn't exist, so
    // the run fails later, for that reason).
    routing.insert(
        AiFeature::WeeklyNote,
        choice(BackendRef::Provider {
            provider_id: "lm_studio".into(),
        }),
    );
    store.set_setting("ai.routing", &routing).unwrap();
    let err = failed(&pagelamp_with_path(&home, &["note"], path));
    assert!(!err.contains("unattended_plan_run"), "{err}");
}

/// A build that doesn't offer the ChatGPT plan (`CHATGPT_PLAN_OFFERED`): `ai codex …` refuses
/// with the facade's message before anything is printed or started (status and sign-out still
/// work: see the next test).
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
        &["ai", "use", "study-plan", "codex", "gpt-6-luna", "--yes"],
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

/// With the plan not offered, `ai codex status` says so and starts nothing: neither a Codex on
/// PATH nor a managed one runs, and no Codex sign-in folder appears. Signing out is clean-up:
/// with no sign-in folder nothing starts; with one, the Codex "Remove all AI data" would use
/// signs out, and nothing else runs.
#[cfg(unix)]
#[test]
fn codex_status_starts_nothing_when_the_plan_is_not_offered() {
    use std::os::unix::fs::PermissionsExt;
    if pagelamp_app::ai::CHATGPT_PLAN_OFFERED {
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let bin = temp.path().join("bin");
    let mark = temp.path().join("codex-ran");
    // Codexes that note every start, with shell builtins only (PATH is just `bin`).
    let fake = |dir: &Path, who: &str| {
        std::fs::create_dir_all(dir).unwrap();
        let codex = dir.join("codex");
        std::fs::write(
            &codex,
            format!("#!/bin/sh\necho \"{who} $*\" >> '{}'\n", mark.display()),
        )
        .unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    fake(&bin, "path");
    let started = || std::fs::read_to_string(&mark).unwrap_or_default();
    let run = |args: &[&str]| pagelamp_with_path(&home, args, Some(&bin));

    let text = ok(&run(&["ai", "codex", "status"]));
    assert_eq!(
        text.trim(),
        "The ChatGPT plan isn't available in this version of PageLamp."
    );
    let status = json_out(&run(&["--json", "ai", "codex", "status"]));
    assert_eq!(status["chatgpt_plan_offered"], false);
    assert_eq!(status["system_codex"], Value::Null);
    assert_eq!(status["login"]["state"], "signed_out");
    assert_eq!(started(), "", "no Codex was started");
    assert!(!home.join("codex-home").exists());

    // A managed Codex as well: status still starts nothing, and with no sign-in folder neither
    // does sign-out.
    fake(
        &home.join("runtimes").join("codex").join("0.1.0"),
        "managed",
    );
    ok(&run(&["ai", "codex", "status"]));
    ok(&run(&["ai", "codex", "logout"]));
    assert_eq!(started(), "");
    assert!(!home.join("codex-home").exists());

    // A sign-in folder left by an earlier build: the managed Codex signs out, and only that.
    std::fs::create_dir_all(home.join("codex-home")).unwrap();
    let text = ok(&run(&["ai", "codex", "logout"]));
    assert!(text.contains("isn't available"), "{text}");
    assert_eq!(started(), "managed logout\n");
}

/// A home with no course and a model on this computer (never asked: nothing listens there)
/// chosen for `feature`.
fn home_with_local_model(feature: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    ok(&pagelamp(&home, &["ai"]));
    Store::open(&home.join("pagelamp.db"))
        .unwrap()
        .insert_model_provider(&ProviderRow {
            id: "local-llm".into(),
            preset: "lm_studio".into(),
            label: "Local LLM".into(),
            wire: "openai_chat".into(),
            base_url: "http://127.0.0.1:9/v1".into(),
            created_at: Utc::now().trunc_subsecs(0),
            last_probe_json: None,
        })
        .unwrap();
    ok(&pagelamp(
        &home,
        &[
            "ai",
            "use",
            feature,
            "local-llm",
            "local-model",
            "--yes",
            "--no-check",
        ],
    ));
    (temp, home)
}

/// A week with nothing to write about: `ai estimate --feature weekly-note` and `note` both say
/// so with the facade's code (exit 1), before anything is sent. The model is on this computer
/// and is never asked.
#[test]
fn nothing_to_write_about_is_said_before_the_note_runs() {
    let (_temp, home) = home_with_local_model("weekly-note");
    for args in [
        &["ai", "estimate", "--feature", "weekly-note"][..],
        &["note"],
    ] {
        let output = pagelamp(&home, args);
        let err = failed(&output);
        assert!(
            err.contains("blocked: nothing_to_write — there is nothing to write about this week"),
            "{args:?}: {err}"
        );
        assert!(stdout(&output).is_empty(), "{args:?}: no estimate line");
    }
}

/// No course to plan for: `ai estimate --feature study-plan` and `plan` both say so with the
/// facade's code (exit 1), before anything is sent.
#[test]
fn no_course_to_plan_for_is_said_before_the_plan_runs() {
    let (_temp, home) = home_with_local_model("study-plan");
    for args in [
        &["ai", "estimate", "--feature", "study-plan"][..],
        &["plan"],
    ] {
        let output = pagelamp(&home, args);
        let err = failed(&output);
        assert!(
            err.contains("blocked: no_course_to_plan — there is no active course to plan for"),
            "{args:?}: {err}"
        );
        assert!(
            err.contains("hidden courses are skipped") && err.contains("course show <course>"),
            "{args:?}: {err}"
        );
        assert!(stdout(&output).is_empty(), "{args:?}: no estimate line");
    }
}
