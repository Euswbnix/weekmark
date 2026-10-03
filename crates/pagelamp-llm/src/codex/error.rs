//! Errors of the Codex commands PageLamp starts. Codex's own output can echo course text or
//! account details, so it never goes into these messages or the logs: only a classified code.

use pagelamp_core::ai::ModelErrorKind;

use super::home::HomeError;

#[derive(Debug, thiserror::Error)]
pub enum CodexError {
    /// Another PageLamp process (or window) is running Codex, signing in or out.
    #[error("Codex is in use by another PageLamp window")]
    Busy,
    #[error("cancelled")]
    Cancelled,
    /// The binary couldn't be started (missing, blocked by antivirus or Smart App Control).
    #[error("could not start Codex ({0})")]
    Start(String),
    /// The tripwire: Codex showed output PageLamp doesn't allow (a tool item, or an item type
    /// this version doesn't know); the run was killed and its output discarded.
    #[error("PageLamp stopped Codex: unexpected output — update PageLamp")]
    Stopped,
    /// Codex ran and failed; `kind` is the classified reason.
    #[error("Codex failed ({})", kind.as_str())]
    Failed { kind: ModelErrorKind },
    #[error("{0}")]
    Io(String),
}

impl From<HomeError> for CodexError {
    fn from(err: HomeError) -> Self {
        match err {
            HomeError::Busy => CodexError::Busy,
            HomeError::Io(message) => CodexError::Io(message),
        }
    }
}

impl From<std::io::Error> for CodexError {
    fn from(err: std::io::Error) -> Self {
        CodexError::Io(err.to_string())
    }
}
