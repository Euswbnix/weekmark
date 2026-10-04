use thiserror::Error;

use crate::brand::{CLI_NAME, PRODUCT_NAME};
use crate::paths::HOME_ENV;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The database was created by a newer PageLamp than this binary understands.
    #[error(
        "database schema version {found} is newer than supported version {supported}; please update {PRODUCT_NAME}"
    )]
    SchemaTooNew { found: i64, supported: i64 },

    /// The database was written by an older PageLamp and must be migrated by a read-write
    /// open (the app or any CLI command) before it can be read (e.g. by the MCP server, when
    /// its own migration attempt failed).
    #[error(
        "the {PRODUCT_NAME} database (schema {found}) needs updating to schema {supported}; open the {PRODUCT_NAME} app once or run `{CLI_NAME} status`"
    )]
    SchemaTooOld { found: i64, supported: i64 },

    /// The database has not been initialised yet (read-only open of a missing/empty DB).
    #[error("{PRODUCT_NAME} has no data yet at {0}; run `{CLI_NAME} sync` first")]
    NotInitialised(String),

    /// Neither `PAGELAMP_HOME` nor a platform data directory is available (no home dir).
    #[error("Could not determine a data directory; set {HOME_ENV}")]
    NoDataDir,

    #[error("not found: {0}")]
    NotFound(String),

    /// A course query matched more than one course.
    #[error("'{query}' matches several courses: {candidates:?}")]
    Ambiguous {
        query: String,
        candidates: Vec<String>,
    },

    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("keychain error: {0}")]
    Secret(String),

    /// The work was stopped on request (`source::CancelFlag`).
    #[error("cancelled")]
    Cancelled,
}

impl Error {
    /// The database is held by another connection or process (SQLite's busy or locked), after
    /// the busy timeout: the same call can work a moment later.
    pub fn is_database_busy(&self) -> bool {
        matches!(
            self,
            Error::Db(rusqlite::Error::SqliteFailure(failure, _))
                if matches!(
                    failure.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
                )
        )
    }
}
