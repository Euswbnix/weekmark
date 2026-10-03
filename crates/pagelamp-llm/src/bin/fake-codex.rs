//! TEST-ONLY stand-in for the Codex CLI, used by pagelamp-llm's tests. It is never shipped: no
//! install or packaging script copies it, and release builds of the CLI and the desktop app
//! don't build other crates' binaries. Keep it out of every packaging step.
//!
//! PageLamp clears a Codex command's environment except for an allow-list, so the only channel
//! into this program is `$CODEX_HOME`:
//! - it reads what to do from `$CODEX_HOME/fake-codex.json`: an object keyed by the command
//!   (`"login"`, `"login --device-auth"`, `"login status"`, `"logout"`, `"--version"`, `"exec"`,
//!   or `"exec:<model>"` for a run with `--model <model>`),
//!   each `{ "stdout": [lines], "stderr": [lines], "exit": code, "sleep_ms": ms,
//!   "wait_for_stdin_eof": bool }`;
//! - it appends what it got to `$CODEX_HOME/fake-codex-observed.jsonl`: its arguments, the names
//!   of its environment variables, its working directory and everything read from stdin.

use std::io::{Read, Write};
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let home = PathBuf::from(std::env::var_os("CODEX_HOME").expect("CODEX_HOME"));
    let script: serde_json::Value = std::fs::read(home.join("fake-codex.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let key = match args.as_slice() {
        [first, second, ..]
            if first == "login" && (second == "status" || second == "--device-auth") =>
        {
            format!("{first} {second}")
        }
        [first, ..] => first.clone(),
        [] => String::new(),
    };
    let model = args
        .iter()
        .position(|a| a == "--model" || a == "-m")
        .and_then(|i| args.get(i + 1));
    let step = match model {
        Some(model) if key == "exec" && !script[format!("exec:{model}")].is_null() => {
            &script[format!("exec:{model}")]
        }
        _ => &script[&key],
    };

    let mut stdin = String::new();
    if step["wait_for_stdin_eof"]
        .as_bool()
        .unwrap_or(key == "exec")
    {
        // Like `codex exec`: read until end of input (never returns while stdin stays open).
        let _ = std::io::stdin().read_to_string(&mut stdin);
    }
    let mut env: Vec<String> = std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect();
    env.sort();
    let observed = serde_json::json!({
        "args": args,
        "env": env,
        "cwd": std::env::current_dir().ok(),
        "stdin": stdin,
    });
    // One write for the whole line, so a reader never sees half of it (Windows CI). Another
    // process (an indexer, antivirus) may hold the new file briefly: retry, and fail loudly
    // rather than leave the tests an empty record.
    let record = format!("{observed}\n");
    let mut recorded = false;
    for _ in 0..20 {
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(home.join("fake-codex-observed.jsonl"))
            .and_then(|mut file| file.write_all(record.as_bytes()));
        if written.is_ok() {
            recorded = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if !recorded {
        eprintln!("fake-codex: could not record what it observed");
        std::process::exit(97);
    }

    let lines = |name: &str| -> Vec<String> {
        step[name]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let delay = std::time::Duration::from_millis(step["sleep_ms"].as_u64().unwrap_or(0));
    for line in lines("stderr") {
        eprintln!("{line}");
    }
    let mut out = std::io::stdout().lock();
    for line in lines("stdout") {
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
    drop(out);
    std::thread::sleep(delay);
    std::process::exit(step["exit"].as_i64().unwrap_or(0) as i32);
}
