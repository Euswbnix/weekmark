//! One Codex run (design §2.3): `codex exec` with every tool off, the prompt on stdin (never in
//! argv) and stdin then closed, in an empty per-run folder, under the `CODEX_HOME` lock.
//!
//! - The answer is the JSONL `agent_message` item: no `-o` file, so PageLamp writes no
//!   course-derived text to disk (the per-run folder holds only the fixed instructions and the
//!   answer's JSON Schema, and is deleted afterwards).
//! - **Tripwire (an allow-list):** any item other than text, reasoning, the to-do list or a
//!   non-fatal error — a tool item (`command_execution`, `file_change`, `web_search`,
//!   `mcp_tool_call`, `collab_tool_call`) or a type this version doesn't know — and any unknown
//!   event kill the run and discard its output (`CodexError::Stopped`).
//! - **Errors** are plain strings in exec; they are classified into `ModelErrorKind` values and
//!   only the code and the stderr length are logged (stderr can echo course text).
//! - **Every run names its model** (`-m`), from the pin's supported list; never the account's
//!   default. A retired default falls back once to the pin's fallback model.
//! - **Cancel:** SIGINT, then kill after 3 s (TerminateProcess on Windows).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pagelamp_core::ai::{Effort, ModelErrorKind};
use pagelamp_core::ai_gate::RenderedPrompt;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::error::CodexError;
use super::home::{CodexHome, RUN_OVERRIDES, toml_string};
use super::process::{self, Purpose, RemoveOnDrop};
use crate::request::Usage;

/// No JSONL line for this long: the run is stuck.
const IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// After SIGINT, how long Codex gets to stop before it is killed (Unix only).
#[cfg(unix)]
const INTERRUPT_GRACE: Duration = Duration::from_secs(3);
/// Stdout beyond this is not an answer.
const MAX_STDOUT: usize = 16 << 20;
/// Kept only to classify an error; never logged.
const MAX_STDERR: usize = 64 << 10;
/// The item types a run may show (0.158.0 `exec/src/exec_events.rs`): the answer, reasoning,
/// the to-do list and non-fatal errors. Everything else stops the run.
const ALLOWED_ITEMS: [&str; 4] = ["agent_message", "reasoning", "todo_list", "error"];
/// The top-level events of 0.158.0; any other event stops the run.
const KNOWN_EVENTS: [&str; 8] = [
    "thread.started",
    "turn.started",
    "turn.completed",
    "turn.failed",
    "item.started",
    "item.updated",
    "item.completed",
    "error",
];

/// What to run.
pub struct ExecRequest<'a> {
    pub prompt: &'a RenderedPrompt,
    /// From the pin's supported list (`codex::pin`); checked by the caller.
    pub model: &'a str,
    pub effort: Effort,
    /// The JSON Schema of a structured answer (`--output-schema`), if any.
    pub output_schema: Option<&'a serde_json::Value>,
    /// Names the per-run folder (letters, digits, `-`, `_`).
    pub run_id: &'a str,
}

/// A finished run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecOutcome {
    /// The final `agent_message` (JSON text when a schema was given; not yet validated).
    pub text: String,
    pub usage: Usage,
    /// The model that answered (the fallback's, after a retired default).
    pub model: String,
}

/// The seam for Codex backends: only `Exec` (Stable) in v0.3.0; an app-server backend can come
/// when OpenAI marks it Stable (design §2.3).
pub trait CodexBackend {
    fn run(
        &self,
        request: &ExecRequest<'_>,
        cancel: &CancellationToken,
    ) -> impl std::future::Future<Output = Result<ExecOutcome, CodexError>> + Send;
}

/// `codex exec`.
#[derive(Clone, Debug)]
pub struct Exec {
    pub binary: PathBuf,
    pub home: CodexHome,
    /// `<local>/ai-runs`.
    pub runs_dir: PathBuf,
}

impl CodexBackend for Exec {
    async fn run(
        &self,
        request: &ExecRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<ExecOutcome, CodexError> {
        self.run_once(request, cancel).await
    }
}

impl Exec {
    /// Run with `request.model`; on `model_not_found` try `fallback` once (a retired default).
    pub async fn run_with_fallback(
        &self,
        request: &ExecRequest<'_>,
        fallback: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<ExecOutcome, CodexError> {
        match self.run_once(request, cancel).await {
            Err(CodexError::Failed {
                kind: ModelErrorKind::ModelNotFound,
            }) if fallback.is_some_and(|f| f != request.model) => {
                let model = fallback.unwrap_or_default();
                tracing::info!(target: "pagelamp::codex", "model not found; trying the fallback once");
                let retry = ExecRequest { model, ..*request };
                self.run_once(&retry, cancel).await
            }
            other => other,
        }
    }

    async fn run_once(
        &self,
        request: &ExecRequest<'_>,
        cancel: &CancellationToken,
    ) -> Result<ExecOutcome, CodexError> {
        if request.run_id.is_empty()
            || !request
                .run_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(CodexError::Io("invalid run id".into()));
        }
        let lock = self.home.lock()?;
        self.home.write_config(&lock)?;

        let run_dir = self.runs_dir.join(request.run_id);
        pagelamp_core::paths::create_private_dir_all(&run_dir)?;
        let _cleanup = RemoveOnDrop::new(run_dir.clone());
        let instructions = run_dir.join("instructions.md");
        std::fs::write(&instructions, request.prompt.instructions())?;
        let schema = match request.output_schema {
            Some(schema) => {
                let path = run_dir.join("schema.json");
                std::fs::write(&path, serde_json::to_vec(schema).unwrap_or_default())?;
                Some(path)
            }
            None => None,
        };
        let catalog = self.home.catalog_path()?;
        let args = argv(
            request.model,
            request.effort,
            schema.as_deref(),
            &instructions,
            &catalog,
        );

        let mut command = process::command(&self.binary, &self.home, Purpose::Other, &run_dir);
        command.args(&args).stdin(std::process::Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|err| CodexError::Start(err.kind().to_string()))?;

        // The prompt, then end of input: `codex exec` otherwise waits for more.
        let mut stdin = child.stdin.take().expect("piped stdin");
        let prompt = request.prompt.user_text().to_string();
        let writer = tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
            drop(stdin);
        });

        let mut stderr = child.stderr.take().expect("piped stderr");
        let stderr_task = tokio::spawn(async move {
            let mut kept = Vec::new();
            let mut buf = [0u8; 8192];
            let mut total = 0usize;
            loop {
                match stderr.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        total += n;
                        if kept.len() < MAX_STDERR {
                            kept.extend_from_slice(&buf[..n.min(MAX_STDERR - kept.len())]);
                        }
                    }
                }
            }
            (String::from_utf8_lossy(&kept).into_owned(), total)
        });

        let stdout = child.stdout.take().expect("piped stdout");
        let mut lines = BufReader::new(stdout).lines();
        let mut parser = EventParser::default();
        let mut read = 0usize;
        let mut stopped = false;
        let failure = loop {
            let next = tokio::select! {
                _ = cancel.cancelled() => {
                    interrupt(&mut child).await;
                    return Err(CodexError::Cancelled);
                }
                line = tokio::time::timeout(IDLE_TIMEOUT, lines.next_line()) => line,
            };
            let line = match next {
                Err(_) => break Some(ModelErrorKind::Timeout),
                Ok(Err(_)) | Ok(Ok(None)) => break None,
                Ok(Ok(Some(line))) => line,
            };
            read += line.len();
            if read > MAX_STDOUT {
                break Some(ModelErrorKind::BadOutput);
            }
            match parser.feed(&line) {
                Step::Continue => {}
                Step::Tripwire(what) => {
                    tracing::warn!(target: "pagelamp::codex", what, "tripwire: unexpected Codex output; run discarded");
                    stopped = true;
                    break Some(ModelErrorKind::BadOutput);
                }
            }
        };
        if failure.is_some() {
            let _ = child.kill().await;
        }
        let status = child.wait().await?;
        let _ = writer.await;
        let (stderr_text, stderr_len) = stderr_task.await.unwrap_or_default();
        drop(lock);
        if stopped {
            return Err(CodexError::Stopped);
        }

        let kind = failure
            .or(parser.failure)
            .or_else(|| (!status.success()).then(|| classify(&stderr_text)));
        if let Some(kind) = kind {
            tracing::info!(
                target: "pagelamp::codex",
                code = kind.as_str(),
                exit = ?status.code(),
                stderr_len,
                "codex exec failed"
            );
            return Err(CodexError::Failed { kind });
        }
        match parser.message {
            Some(text) => Ok(ExecOutcome {
                text,
                usage: parser.usage,
                model: request.model.to_string(),
            }),
            None => Err(CodexError::Failed {
                kind: ModelErrorKind::BadOutput,
            }),
        }
    }
}

/// The arguments of one run (design §2.3, with the 0.158.0 keys of `home::RUN_OVERRIDES`).
pub fn argv(
    model: &str,
    effort: Effort,
    schema: Option<&Path>,
    instructions: &Path,
    catalog: &Path,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "exec",
        "--json",
        "--ephemeral",
        "--skip-git-repo-check",
        "--strict-config",
        "--sandbox",
        "read-only",
        "--model",
        model,
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    if let Some(schema) = schema {
        args.push("--output-schema".into());
        args.push(schema.as_os_str().to_owned());
    }
    for over in RUN_OVERRIDES {
        args.push("-c".into());
        args.push((*over).into());
    }
    args.push("-c".into());
    args.push(format!("model_reasoning_effort=\"{}\"", reasoning_effort(effort)).into());
    args.push("-c".into());
    args.push(format!("model_instructions_file={}", toml_string(instructions)).into());
    args.push("-c".into());
    args.push(format!("model_catalog_json={}", toml_string(catalog)).into());
    // The prompt comes on stdin.
    args.push("-".into());
    args
}

/// Every model of the pin lists low, medium and high (0.158.0 `models.json`); "lowest" is low.
fn reasoning_effort(effort: Effort) -> &'static str {
    match effort {
        Effort::Lowest | Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
}

/// Unix: SIGINT, then a kill after `INTERRUPT_GRACE`. Windows: a kill (TerminateProcess) at
/// once.
async fn interrupt(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id().and_then(|id| i32::try_from(id).ok()) {
        // SAFETY: signalling our own child process by its pid.
        unsafe {
            libc::kill(pid, libc::SIGINT);
        }
        if tokio::time::timeout(INTERRUPT_GRACE, child.wait())
            .await
            .is_ok()
        {
            return;
        }
    }
    let _ = child.kill().await;
}

enum Step {
    Continue,
    Tripwire(String),
}

/// Reads `codex exec --json` (0.158.0 `exec/src/exec_events.rs`).
#[derive(Default)]
struct EventParser {
    message: Option<String>,
    usage: Usage,
    failure: Option<ModelErrorKind>,
}

impl EventParser {
    fn feed(&mut self, line: &str) -> Step {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            return Step::Continue; // not JSON: ignored (never logged)
        };
        let event_type = event["type"].as_str().unwrap_or_default();
        if !KNOWN_EVENTS.contains(&event_type) {
            return Step::Tripwire(format!("event:{}", short(event_type)));
        }
        match event_type {
            "item.started" | "item.updated" | "item.completed" => {
                let item = &event["item"];
                let kind = item["type"].as_str().unwrap_or_default();
                if !ALLOWED_ITEMS.contains(&kind) {
                    return Step::Tripwire(format!("item:{}", short(kind)));
                }
                if kind == "agent_message"
                    && event["type"] == "item.completed"
                    && let Some(text) = item["text"].as_str()
                {
                    self.message = Some(text.to_string());
                }
            }
            "turn.completed" => {
                let usage = &event["usage"];
                let n = |key: &str| usage[key].as_i64().map_or(0, |v| v.max(0) as u64);
                let cached = n("cached_input_tokens");
                self.usage = Usage {
                    input_uncached: n("input_tokens").saturating_sub(cached),
                    cache_read: cached,
                    cache_write: n("cache_write_input_tokens"),
                    output: n("output_tokens"),
                    reasoning: usage["reasoning_output_tokens"]
                        .as_i64()
                        .map(|v| v.max(0) as u64),
                    estimated: false,
                };
            }
            "turn.failed" => {
                self.failure = Some(classify(
                    event["error"]["message"].as_str().unwrap_or_default(),
                ));
            }
            "error" => {
                self.failure = Some(classify(event["message"].as_str().unwrap_or_default()));
            }
            _ => {}
        }
        Step::Continue
    }
}

/// A type name for the log (a short identifier, never content).
fn short(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
        .take(40)
        .collect()
}

/// exec errors are plain strings; the phrases below are OpenAI's and Codex's own.
pub fn classify(message: &str) -> ModelErrorKind {
    let m = message.to_ascii_lowercase();
    if m.contains("requires a newer version of codex") {
        ModelErrorKind::RuntimeOutdated
    } else if m.contains("usage limit") || m.contains("purchase more credits") {
        ModelErrorKind::UsageLimit
    } else if m.contains("not logged in")
        || m.contains("log in again")
        || m.contains("unauthorized")
        || m.contains("401")
    {
        ModelErrorKind::NotSignedIn
    } else if m.contains("model_not_found")
        || (m.contains("model")
            && (m.contains("does not exist")
                || m.contains("not found")
                || m.contains("is not supported")))
    {
        ModelErrorKind::ModelNotFound
    } else if m.contains("context window")
        || m.contains("context_length_exceeded")
        || m.contains("too long")
    {
        ModelErrorKind::ContextTooLong
    } else if m.contains("rate limit") || m.contains("429") {
        ModelErrorKind::RateLimited
    } else if m.contains("timed out") || m.contains("timeout") {
        ModelErrorKind::Timeout
    } else if m.contains("stream disconnected")
        || m.contains("error sending request")
        || m.contains("connection")
        || m.contains("network")
    {
        ModelErrorKind::Network
    } else if m.contains("overloaded")
        || m.contains("500")
        || m.contains("502")
        || m.contains("503")
        || m.contains("server error")
    {
        ModelErrorKind::Overloaded
    } else {
        ModelErrorKind::InvalidRequest
    }
}
