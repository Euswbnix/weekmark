//! Sign-in through Codex itself (design §2.3): PageLamp starts `codex login` (browser, with a
//! local callback on port 1455) or `codex login --device-auth`, shows the link or the one-time
//! code Codex prints, and waits. It never sees or stores a credential: Codex keeps its own, in
//! the OS keychain or a 0600 file inside PageLamp's `CODEX_HOME`.
//!
//! `codex login status` is read by its exit code and one phrase ("Logged in using ChatGPT" /
//! "Logged in using …"); its output, like every other line Codex prints, is never logged.

use std::path::Path;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio_util::sync::CancellationToken;

use super::error::CodexError;
use super::home::CodexHome;
use super::pin::Version;
use super::process::{self, Purpose};
use pagelamp_core::ai::ModelErrorKind;

/// Commands other than a sign-in finish quickly; a hung one is killed.
const SHORT_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
/// Lines longer than this are cut (Codex's prompts are short).
const MAX_LINE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginMethod {
    /// The browser flow (Codex opens it; the link is also shown).
    Browser,
    /// A code entered at a link, for when the browser can't reach this computer (beta; the
    /// student enables it in ChatGPT's security settings, else Codex falls back to the browser).
    DeviceCode,
}

/// Progress of `login` (the facade maps it to its `LoginEvent`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginEvent {
    BrowserOpened {
        url: Option<String>,
    },
    DeviceCode {
        verification_url: String,
        user_code: String,
        expires_in_secs: Option<u32>,
    },
    /// Waiting for the student to finish in the browser.
    Waiting,
    Done,
}

/// How Codex is signed in, from `codex login status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginState {
    SignedOut,
    /// "Logged in using ChatGPT": runs use the student's plan.
    ChatGpt,
    /// Any other sign-in (an API key, an access token, …): runs bill that, not the plan.
    ApiKey,
}

/// Sign in. Holds the `CODEX_HOME` lock throughout (`Busy` if another window runs Codex).
pub async fn login(
    binary: &Path,
    home: &CodexHome,
    method: LoginMethod,
    cancel: &CancellationToken,
    on_event: &(dyn Fn(LoginEvent) + Send + Sync),
) -> Result<(), CodexError> {
    let lock = home.lock()?;
    home.write_config(&lock)?;
    let mut command = process::command(binary, home, Purpose::Login, home.dir());
    command.arg("login");
    if method == LoginMethod::DeviceCode {
        command.arg("--device-auth");
    }
    let mut child = command
        .spawn()
        .map_err(|err| CodexError::Start(err.kind().to_string()))?;
    // Each line with the stream it came from: the two are read concurrently, so their lines
    // interleave in any order, and each stream's prompts are parsed on their own.
    let (sender, mut lines) = tokio::sync::mpsc::unbounded_channel::<(Stream, String)>();
    for (from, stream) in [
        (
            Stream::Stdout,
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn AsyncRead + Unpin + Send>),
        ),
        (
            Stream::Stderr,
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn AsyncRead + Unpin + Send>),
        ),
    ] {
        let Some(stream) = stream else { continue };
        let sender = sender.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stream).lines();
            while let Ok(Some(mut line)) = reader.next_line().await {
                line.truncate(MAX_LINE);
                if sender.send((from, line)).is_err() {
                    break;
                }
            }
        });
    }
    drop(sender);

    let mut parser = LoginParser::default();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.kill().await;
                return Err(CodexError::Cancelled);
            }
            line = lines.recv() => match line {
                Some((from, line)) => parser.feed(from, &line).into_iter().for_each(on_event),
                None => break,
            },
        }
    }
    let status = tokio::select! {
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err(CodexError::Cancelled);
        }
        status = child.wait() => status?,
    };
    drop(lock);
    if status.success() {
        on_event(LoginEvent::Done);
        Ok(())
    } else {
        tracing::info!(target: "pagelamp::codex", code = ?status.code(), "codex login failed");
        Err(CodexError::Failed {
            kind: ModelErrorKind::AuthRejected,
        })
    }
}

/// `codex login status`.
pub async fn status(binary: &Path, home: &CodexHome) -> Result<LoginState, CodexError> {
    let lock = home.lock()?;
    home.write_config(&lock)?;
    let mut command = process::command(binary, home, Purpose::Other, home.dir());
    command.args(["login", "status"]);
    let output = run_short(command).await?;
    Ok(if !output.status.success() {
        LoginState::SignedOut
    } else {
        classify_status(&String::from_utf8_lossy(&output.stderr))
    })
}

/// `codex logout` (revokes and clears Codex's own credentials; succeeds when signed out).
pub async fn logout(binary: &Path, home: &CodexHome) -> Result<(), CodexError> {
    let lock = home.lock()?;
    home.write_config(&lock)?;
    let mut command = process::command(binary, home, Purpose::Other, home.dir());
    command.arg("logout");
    run_short(command).await?;
    Ok(())
}

/// `codex logout` for a blocking caller ("Remove all AI data" runs it before deleting
/// `CODEX_HOME`): the same lock and rules, killed after `SHORT_COMMAND_TIMEOUT`.
pub fn logout_blocking(binary: &Path, home: &CodexHome) -> Result<(), CodexError> {
    let lock = home.lock()?;
    home.write_config(&lock)?;
    let mut child = process::std_command(binary, home, home.dir())
        .arg("logout")
        .spawn()
        .map_err(|err| CodexError::Start(err.kind().to_string()))?;
    let started = std::time::Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        if started.elapsed() > SHORT_COMMAND_TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(CodexError::Failed {
                kind: ModelErrorKind::Timeout,
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `codex --version` (for "Use my installed Codex", D12). Needs no lock or config.
pub async fn version(binary: &Path, home: &CodexHome) -> Result<Version, CodexError> {
    let cwd = std::env::temp_dir();
    let mut command = process::command(binary, home, Purpose::Other, &cwd);
    command.arg("--version");
    let output = run_short(command).await?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(Version::parse)
        .filter(|_| output.status.success())
        .ok_or(CodexError::Failed {
            kind: ModelErrorKind::Unsupported,
        })
}

async fn run_short(
    mut command: tokio::process::Command,
) -> Result<std::process::Output, CodexError> {
    let child = command
        .spawn()
        .map_err(|err| CodexError::Start(err.kind().to_string()))?;
    match tokio::time::timeout(SHORT_COMMAND_TIMEOUT, child.wait_with_output()).await {
        Ok(output) => Ok(output?),
        // `kill_on_drop`: the timed-out child is killed with the future.
        Err(_) => Err(CodexError::Failed {
            kind: ModelErrorKind::Timeout,
        }),
    }
}

fn classify_status(stderr: &str) -> LoginState {
    if stderr.contains("Logged in using ChatGPT") {
        LoginState::ChatGpt
    } else if stderr.contains("Logged in using") {
        LoginState::ApiKey
    } else {
        LoginState::SignedOut
    }
}

/// Reads Codex's sign-in prompts (0.158.0 `cli/src/login.rs`, `login/src/device_code_auth.rs`):
/// - browser: "…navigate to this URL to authenticate:" then the URL;
/// - device code: "1. Open this link…" then the URL, "2. Enter this one-time code (expires in 15
///   minutes)" then the code; "Device code login is not enabled; falling back to browser login."
///   switches to the browser flow.
///
/// Codex may print these on stdout or stderr, and the two are read concurrently: a line on one
/// stream never clears what the other is expecting (an error line landing between "one-time
/// code" and the code lost the code). Only "announced" is shared.
#[derive(Default)]
struct LoginParser {
    streams: [PromptState; 2],
    announced: bool,
}

/// Which of Codex's output streams a line came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stream {
    Stdout,
    Stderr,
}

/// What one stream's prompt is waiting for.
#[derive(Default)]
struct PromptState {
    expecting: Option<Expecting>,
    device_url: Option<String>,
    expires_in_secs: Option<u32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Expecting {
    BrowserUrl,
    DeviceUrl,
    DeviceCode,
}

impl LoginParser {
    fn feed(&mut self, from: Stream, raw: &str) -> Vec<LoginEvent> {
        let announced = &mut self.announced;
        let state = &mut self.streams[from as usize];
        let line = process::strip_ansi(raw);
        let text = line.trim();
        if text.is_empty() {
            return Vec::new();
        }
        let lower = text.to_ascii_lowercase();
        if lower.contains("navigate to this url") {
            state.expecting = Some(Expecting::BrowserUrl);
            return Vec::new();
        }
        if lower.contains("open this link") {
            state.expecting = Some(Expecting::DeviceUrl);
            return Vec::new();
        }
        if lower.contains("one-time code") {
            state.expecting = Some(Expecting::DeviceCode);
            state.expires_in_secs = minutes_in(&lower).map(|m| m * 60);
            return Vec::new();
        }
        match state.expecting.take() {
            Some(Expecting::BrowserUrl) if text.starts_with("https://") && !*announced => {
                *announced = true;
                vec![
                    LoginEvent::BrowserOpened {
                        url: Some(text.to_string()),
                    },
                    LoginEvent::Waiting,
                ]
            }
            Some(Expecting::DeviceUrl) if text.starts_with("https://") => {
                state.device_url = Some(text.to_string());
                Vec::new()
            }
            Some(Expecting::DeviceCode) if !*announced => match state.device_url.take() {
                Some(verification_url) if is_code(text) => {
                    *announced = true;
                    vec![
                        LoginEvent::DeviceCode {
                            verification_url,
                            user_code: text.to_string(),
                            expires_in_secs: state.expires_in_secs,
                        },
                        LoginEvent::Waiting,
                    ]
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }
}

/// "(expires in 15 minutes)" → 15.
fn minutes_in(text: &str) -> Option<u32> {
    let after = text.split("expires in").nth(1)?;
    after
        .split(|c: char| !c.is_ascii_digit())
        .find(|part| !part.is_empty())?
        .parse()
        .ok()
}

/// A one-time code: letters, digits and dashes, short.
fn is_code(text: &str) -> bool {
    text.len() <= 32 && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(lines: &[&str]) -> Vec<LoginEvent> {
        let mut parser = LoginParser::default();
        lines
            .iter()
            .flat_map(|l| parser.feed(Stream::Stdout, l))
            .collect()
    }

    #[test]
    fn the_browser_link_is_read_from_codexs_prompt() {
        let events = feed_all(&[
            "Starting local login server on http://localhost:1455.",
            "If your browser did not open, navigate to this URL to authenticate:",
            "",
            "https://auth.openai.com/oauth/authorize?client_id=demo&state=demo",
            "",
            "On a remote or headless machine? Use `codex login --device-auth` instead.",
        ]);
        assert_eq!(
            events,
            [
                LoginEvent::BrowserOpened {
                    url: Some(
                        "https://auth.openai.com/oauth/authorize?client_id=demo&state=demo".into()
                    )
                },
                LoginEvent::Waiting
            ]
        );
    }

    #[test]
    fn the_device_code_and_its_expiry_are_read_through_the_colours() {
        let events = feed_all(&[
            "Welcome to Codex [v\u{1b}[90m0.158.0\u{1b}[0m]",
            "",
            "1. Open this link in your browser and sign in to your account",
            "   \u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m",
            "",
            "2. Enter this one-time code \u{1b}[90m(expires in 15 minutes)\u{1b}[0m",
            "   \u{1b}[94mDEMO-1234\u{1b}[0m",
        ]);
        assert_eq!(
            events,
            [
                LoginEvent::DeviceCode {
                    verification_url: "https://auth.openai.com/codex/device".into(),
                    user_code: "DEMO-1234".into(),
                    expires_in_secs: Some(900),
                },
                LoginEvent::Waiting
            ]
        );
        // Not enabled for the account: Codex falls back to the browser.
        let events = feed_all(&[
            "Device code login is not enabled; falling back to browser login.",
            "If your browser did not open, navigate to this URL to authenticate:",
            "https://auth.openai.com/oauth/authorize?state=demo",
        ]);
        assert!(matches!(events[0], LoginEvent::BrowserOpened { .. }));
    }

    #[test]
    fn a_line_on_the_other_stream_never_loses_the_code() {
        // stderr lines landing anywhere between stdout's prompts (the streams are read
        // concurrently; Windows CI saw the error line arrive between "one-time code" and the
        // code).
        let stdout = [
            "1. Open this link in your browser and sign in to your account",
            "   \u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m",
            "2. Enter this one-time code \u{1b}[90m(expires in 15 minutes)\u{1b}[0m",
            "   \u{1b}[94mDEMO-1234\u{1b}[0m",
        ];
        let expected = [
            LoginEvent::DeviceCode {
                verification_url: "https://auth.openai.com/codex/device".into(),
                user_code: "DEMO-1234".into(),
                expires_in_secs: Some(900),
            },
            LoginEvent::Waiting,
        ];
        for at in 0..=stdout.len() {
            let mut parser = LoginParser::default();
            let mut events = Vec::new();
            for (i, line) in stdout.iter().enumerate() {
                if i == at {
                    events.extend(parser.feed(
                        Stream::Stderr,
                        "Error logging in with device code: demo failure",
                    ));
                }
                events.extend(parser.feed(Stream::Stdout, line));
            }
            if at == stdout.len() {
                events.extend(parser.feed(Stream::Stderr, "Error logging in: demo failure"));
            }
            assert_eq!(events, expected, "stderr line before stdout line {at}");
        }
        // The same prompt on stderr works too, and a code is announced once.
        let mut parser = LoginParser::default();
        let mut events = Vec::new();
        for line in stdout {
            events.extend(parser.feed(Stream::Stderr, line));
        }
        for line in stdout {
            events.extend(parser.feed(Stream::Stdout, line));
        }
        assert_eq!(events, expected);
    }

    #[test]
    fn status_is_read_from_one_phrase() {
        assert_eq!(
            classify_status("Logged in using ChatGPT\n"),
            LoginState::ChatGpt
        );
        assert_eq!(
            classify_status("Logged in using an API key - sk-demo***\n"),
            LoginState::ApiKey
        );
        assert_eq!(
            classify_status("Logged in using access token\n"),
            LoginState::ApiKey
        );
        assert_eq!(classify_status("Not logged in\n"), LoginState::SignedOut);
    }
}
