//! Mode A: the student's ChatGPT plan through the official, unmodified Codex CLI (design §2.3,
//! plan M2). PageLamp never touches ChatGPT credentials: Codex signs in and runs on its own, in a
//! dedicated `CODEX_HOME`, with every tool off.
//!
//! - `pin`: the pinned version, its models and the verified asset per target.
//! - `runtime`: download, verify and install that asset (at most two versions kept).
//! - `home`: the dedicated `CODEX_HOME`, its config and the cross-process lock.
//! - `login`: sign-in, status and sign-out through Codex itself.
//! - `process`: how every Codex command is started (environment allow-list, null stdin).
//! - `exec`: one run (`codex exec`), its JSONL answer, the tripwire and the error mapping.

mod error;
pub mod exec;
pub mod home;
pub mod login;
pub mod pin;
mod process;
pub mod runtime;

pub use error::CodexError;
pub use exec::{CodexBackend, Exec, ExecOutcome, ExecRequest};
pub use home::{CodexHome, HomeError, HomeLock};
pub use login::{LoginEvent, LoginMethod, LoginState};
pub use pin::{Pin, PinAsset, Version, pin, running_target};
pub use runtime::{InstallError, InstallEvent, Installed, Runtime};
