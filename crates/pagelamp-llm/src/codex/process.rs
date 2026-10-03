//! How every Codex command is started (design §2.3, §3.6): an absolute, verified binary; a
//! cleared environment plus an allow-list (never `CODEX_API_KEY`, `OPENAI_API_KEY`,
//! `CODEX_ACCESS_TOKEN` or `PAGELAMP_SECRET_*`, so a run can't silently switch to API billing);
//! `CODEX_HOME` = PageLamp's own; a null stdin unless the prompt is written to it; no console
//! window on Windows; killed if PageLamp drops it.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;

use super::home::CodexHome;

/// Variables a Codex command may see (design §2.3), whatever else PageLamp's own environment
/// holds. Names are matched exactly; Windows is case-insensitive about them.
const ALLOWED: &[&str] = &[
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "TMP",
    "TEMP",
    "TMPDIR",
    "LANG",
    "PATH",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "SSL_CERT_FILE",
];

/// Extra variables for `codex login` on Linux and BSD, so Codex can open the browser.
const BROWSER: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "BROWSER",
];

/// What the command is for (only `Login` may open a browser).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    Login,
    Other,
}

/// The environment a Codex command gets: the allow-listed part of `current`, plus `CODEX_HOME`.
pub(crate) fn environment(
    current: impl IntoIterator<Item = (OsString, OsString)>,
    home: &CodexHome,
    purpose: Purpose,
) -> Vec<(OsString, OsString)> {
    let allowed = |name: &str| {
        ALLOWED.iter().any(|a| names_match(a, name))
            || (purpose == Purpose::Login && BROWSER.iter().any(|b| names_match(b, name)))
    };
    let mut env: Vec<(OsString, OsString)> = current
        .into_iter()
        .filter(|(name, _)| name.to_str().is_some_and(allowed))
        .collect();
    env.push(("CODEX_HOME".into(), home.dir().as_os_str().to_owned()));
    env
}

fn names_match(allowed: &str, name: &str) -> bool {
    if cfg!(windows) {
        allowed.eq_ignore_ascii_case(name)
    } else {
        allowed == name
    }
}

/// A Codex command with PageLamp's rules applied; stdin is null unless changed by the caller.
pub(crate) fn command(
    binary: &Path,
    home: &CodexHome,
    purpose: Purpose,
    cwd: &Path,
) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    command
        .env_clear()
        .envs(environment(std::env::vars_os(), home, purpose))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    command
}

/// Take an OS file lock without waiting for a real holder, but riding out the moment after one
/// releases it: `flock` locks belong to the open file, and a child another thread is starting
/// (between fork and exec) briefly shares every open file, so a lock just released can linger
/// for microseconds. Retries for up to `LOCK_GRACE`, then reports `WouldBlock`.
pub(crate) fn try_lock_briefly(file: &std::fs::File) -> Result<(), std::fs::TryLockError> {
    const LOCK_GRACE: std::time::Duration = std::time::Duration::from_millis(250);
    let started = std::time::Instant::now();
    loop {
        match file.try_lock() {
            Err(std::fs::TryLockError::WouldBlock) if started.elapsed() < LOCK_GRACE => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            other => return other,
        }
    }
}

/// Deletes a file or folder when dropped, unless disarmed.
pub(crate) struct RemoveOnDrop(std::path::PathBuf);

impl RemoveOnDrop {
    pub(crate) fn new(path: std::path::PathBuf) -> RemoveOnDrop {
        RemoveOnDrop(path)
    }

    pub(crate) fn disarm(self) {
        std::mem::forget(self);
    }
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if self.0.is_dir() {
            let _ = std::fs::remove_dir_all(&self.0);
        } else {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

/// The same rules for a blocking caller (`login::logout_blocking`): output discarded.
pub(crate) fn std_command(binary: &Path, home: &CodexHome, cwd: &Path) -> std::process::Command {
    let mut command = std::process::Command::new(binary);
    command
        .env_clear()
        .envs(environment(std::env::vars_os(), home, Purpose::Other))
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command
}

/// Remove ANSI escape sequences (Codex colours its prompts).
pub(crate) fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                // CSI: parameters and intermediates, then one final byte in @..~.
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_allow_listed_variables_reach_codex() {
        let home = CodexHome::new("/demo/local/codex-home");
        let current: Vec<(OsString, OsString)> = [
            ("PATH", "/usr/bin"),
            ("HOME", "/demo/home"),
            ("LANG", "en_CA.UTF-8"),
            ("HTTPS_PROXY", "http://proxy.demo:8080"),
            ("OPENAI_API_KEY", "sk-demo-not-a-real-key"),
            ("CODEX_API_KEY", "sk-demo-not-a-real-key"),
            ("CODEX_ACCESS_TOKEN", "demo-token"),
            ("CODEX_HOME", "/demo/home/.codex"),
            ("PAGELAMP_SECRET_LLM_OPENAI", "sk-demo-not-a-real-key"),
            ("PAGELAMP_HOME", "/demo/pagelamp"),
            ("DISPLAY", ":0"),
            ("RUST_LOG", "trace"),
        ]
        .iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .collect();
        let names = |env: Vec<(OsString, OsString)>| -> Vec<String> {
            env.into_iter()
                .map(|(k, _)| k.into_string().unwrap())
                .collect()
        };
        let other = environment(current.clone(), &home, Purpose::Other);
        assert!(
            other
                .iter()
                .any(|(k, v)| k == "CODEX_HOME" && v == "/demo/local/codex-home"),
            "PageLamp's own CODEX_HOME, never the student's"
        );
        assert_eq!(
            names(other),
            ["PATH", "HOME", "LANG", "HTTPS_PROXY", "CODEX_HOME"]
        );
        let login = names(environment(current, &home, Purpose::Login));
        assert!(login.contains(&"DISPLAY".to_string()), "the browser opens");
        for secret in [
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "CODEX_ACCESS_TOKEN",
            "PAGELAMP_SECRET_LLM_OPENAI",
        ] {
            assert!(!login.contains(&secret.to_string()), "{secret}");
        }
    }

    #[test]
    fn ansi_colours_are_removed() {
        assert_eq!(
            strip_ansi("   \u{1b}[94mhttps://auth.openai.com/codex/device\u{1b}[0m"),
            "   https://auth.openai.com/codex/device"
        );
        assert_eq!(
            strip_ansi("\u{1b}[1;90m(expires)\u{1b}[0m ok"),
            "(expires) ok"
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
