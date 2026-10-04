//! Application services — the ONE facade that user interfaces call.
//!
//! Both `apps/pagelamp-cli` and the desktop app (`apps/desktop/src-tauri`, Tauri commands)
//! call only this crate (plus `pagelamp_core::model` / `pagelamp_core::views` types). This
//! keeps the backend/frontend contract in one place: if the desktop app needs something, it
//! is added here first.
//!
//! All public types returned here derive `Serialize + JsonSchema`; `pagelamp schema` (CLI)
//! prints them as one JSON Schema document (`json_schema()`) so the frontend can generate
//! TypeScript types (json-schema-to-typescript) instead of hand-writing them.
//!
//! Responsibilities:
//! - source management: add/remove Canvas (validates token, stores it in the keychain),
//!   folder, iCal (feed URL in keychain); removing a source deletes its secret;
//!   `update_source_secret` replaces an expired token / changed feed URL in place.
//! - sync orchestration: `sync_all` / `sync_source` run the right source crate, record the
//!   outcome with `Store::record_sync`, and emit progress events. An advisory lock file
//!   (`<data_dir>/sync.lock`) prevents CLI and desktop app from syncing concurrently
//!   (`AppErrorKind::Busy`).
//! - read views shared with the MCP server — implemented in `pagelamp_core::views`.
//! - course settings: AI policy, term override, hidden.
//! - MCP client configuration snippets for Claude Desktop, Claude Code, Codex (and generic).
//!
//! Every method returns `Result<T, AppError>`; `AppError` serialises as
//! `{ "kind": "...", "message": "..." }` and its message never contains a secret.

mod activity;
pub mod ai;
mod auto_sync;
mod course;
pub mod diagnostics;
mod lock;
mod mcp_config;
mod sync;
mod updates;

pub use activity::{Activity, ActivityItem, ActivityKind};
pub use course::calendar::{
    CalendarBatchEvent, CalendarRunOutcome, CourseCalendarView, OFFER_NO_CALENDAR,
    ReadCalendarOptions, SyllabusOffer,
};
pub use course::dates::{BreakInput, CourseDatesInput, SegmentInput};
pub use course::removal::{
    BackupInfo, LostAfterPurge, PurgeReport, RemovalPreview, RemovalPreviewItem, RemovalReason,
    RemovalReport, RemoveOptions, RemovedCourse, RestoreFailure, RestoreOutcome, TombstoneState,
};
pub use course::{
    CourseLifecycleEntry, KEEP_CURRENT_DAYS, LifecycleSummary, NOT_NOW_DAYS, keep_forever,
};
pub use updates::{
    Shell, StartupTasks, SyncDue, UpdateChannel, UpdateCheckOutcome, UpdateCheckRecord,
    UpdatePrefs, WhatsNew, WhatsNewTopic,
};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::NaiveDate;
use pagelamp_canvas::CanvasConfig;
pub use pagelamp_core::auto_sync::{AutoSync, AutoSyncTrigger, SyncPrefs};
use pagelamp_core::model::{
    AiMaterialsState, AiPolicy, Course, SearchHit, SourceErrorKind, SourceKind, SourceRecord,
    StoreCounts, StoredStudyPlan, TermSource, Timestamp,
};
use pagelamp_core::paths;
use pagelamp_core::secrets::{KeychainSecrets, SecretBackend};
pub use pagelamp_core::source::SyncStage;
use pagelamp_core::source::{CourseSyncSummary, SourceError};
use pagelamp_core::store::Store;
use pagelamp_core::views::{self, AsOf, CourseOverview, CourseSummary, Deadline, WeekMaterials};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::lock::SyncLock;

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

/// What kind of failure happened; UIs branch on this, never on `message`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AppErrorKind {
    /// Token/feed URL rejected (expired, revoked, wrong).
    Auth,
    /// Could not reach the server (DNS, TLS, timeout, refused).
    Network,
    /// Bad input from the user (malformed URL/date, folder is not a directory, …).
    Invalid,
    /// Course/source/material does not exist.
    NotFound,
    /// A course reference matched several courses; `message` lists them.
    Ambiguous,
    /// Another process (CLI or desktop app) is syncing (holds `sync.lock`).
    Busy,
    /// The database was written by a newer PageLamp than this one: update PageLamp.
    SchemaTooNew,
    /// The database was written by an older PageLamp and not updated yet: open the app once
    /// (or run any `pagelamp` command), which updates it.
    SchemaTooOld,
    /// A model call was refused before anything was sent; see `blocked`.
    Blocked,
    /// A model call failed; see `model_error` (and `retry_after_secs`).
    Model,
    /// The caller cancelled (a sync, a generation, a download).
    Cancelled,
    /// Anything else (database, keychain, I/O, bugs).
    Internal,
}

/// Error returned by every facade method. Serialised as-is by the Tauri commands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AppError {
    pub kind: AppErrorKind,
    /// User-presentable; never contains a secret.
    pub message: String,
    /// Why a model call was refused (kind `blocked`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<pagelamp_core::ai::BlockReason>,
    /// Why a model call failed (kind `model`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_error: Option<pagelamp_core::ai::ModelErrorKind>,
    /// How long the provider asked to wait before trying again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u32>,
}

impl AppError {
    pub fn new(kind: AppErrorKind, message: impl Into<String>) -> Self {
        AppError {
            kind,
            message: message.into(),
            blocked: None,
            model_error: None,
            retry_after_secs: None,
        }
    }

    /// A model call refused before anything was sent.
    pub fn blocked(reason: pagelamp_core::ai::BlockReason, message: impl Into<String>) -> Self {
        AppError {
            blocked: Some(reason),
            ..AppError::new(AppErrorKind::Blocked, message)
        }
    }

    pub fn cancelled() -> Self {
        AppError::new(AppErrorKind::Cancelled, "Cancelled.")
    }
}

impl From<pagelamp_llm::LlmError> for AppError {
    fn from(err: pagelamp_llm::LlmError) -> Self {
        match err {
            pagelamp_llm::LlmError::Cancelled => AppError::cancelled(),
            pagelamp_llm::LlmError::Blocked { reason, message } => {
                AppError::blocked(reason, message)
            }
            pagelamp_llm::LlmError::Model(error) => AppError {
                model_error: Some(error.kind),
                retry_after_secs: error
                    .retry_after
                    .map(|wait| u32::try_from(wait.as_secs()).unwrap_or(u32::MAX)),
                ..AppError::new(AppErrorKind::Model, error.message)
            },
        }
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

impl From<pagelamp_core::Error> for AppError {
    fn from(err: pagelamp_core::Error) -> Self {
        use pagelamp_core::Error as E;
        let kind = match &err {
            E::NotFound(_) | E::NotInitialised(_) => AppErrorKind::NotFound,
            E::Ambiguous { .. } => AppErrorKind::Ambiguous,
            E::SchemaTooNew { .. } => AppErrorKind::SchemaTooNew,
            E::SchemaTooOld { .. } => AppErrorKind::SchemaTooOld,
            E::Invalid(_) => AppErrorKind::Invalid,
            E::Cancelled => AppErrorKind::Cancelled,
            E::Db(_) | E::Json(_) | E::Io(_) | E::NoDataDir | E::Secret(_) => {
                AppErrorKind::Internal
            }
        };
        // Core error messages never contain secrets (the store never sees them and
        // `secrets` redacts keychain errors).
        AppError::new(kind, err.to_string())
    }
}

impl From<SourceError> for AppError {
    fn from(err: SourceError) -> Self {
        if err.invalid_input {
            return AppError::new(AppErrorKind::Invalid, err.message);
        }
        let kind = match err.kind {
            SourceErrorKind::AuthExpiredOrRevoked => AppErrorKind::Auth,
            SourceErrorKind::Network | SourceErrorKind::RateLimited => AppErrorKind::Network,
            SourceErrorKind::NotFound => AppErrorKind::NotFound,
            SourceErrorKind::Other => AppErrorKind::Internal,
        };
        AppError::new(kind, err.message)
    }
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

// ---------------------------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AppStatus {
    /// Version of this PageLamp build.
    pub version: String,
    pub data_dir: String,
    pub db_path: String,
    pub sources: Vec<SourceRecord>,
    pub counts: StoreCounts,
    /// Most recent successful sync over all sources.
    pub last_synced_at: Option<Timestamp>,
    /// True while any process holds `sync.lock`.
    pub sync_in_progress: bool,
    /// How often PageLamp syncs by itself while it runs.
    pub auto_sync: AutoSync,
    /// Per source id: when its deadlines and announcements were last read, for a source an
    /// automatic sync has read lightly since its last full sync (`sources[].last_synced_at`
    /// keeps meaning the full sync). A source without an entry has that one clock.
    pub deadlines_synced_at: BTreeMap<String, Timestamp>,
}

// ---------------------------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------------------------

/// Options for a sync run. All fields have defaults, so `{}` is a valid request.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SyncRequest {
    /// Download LMS (Canvas) files and index their text. Default FALSE: downloading a file
    /// through Canvas counts as viewing it (it can complete "must view" module requirements
    /// and shows up in instructor analytics), so files are listed as `not_downloaded` unless
    /// the student explicitly asks — see `App::download_course_files`. Folder sources are
    /// local and always indexed.
    pub download_files: bool,
    /// Skip LMS files larger than this many megabytes.
    pub max_file_mb: u32,
    /// Only sync these courses (ids or codes); empty = all.
    pub only_courses: Vec<String>,
    /// PageLamp started this sync by itself (`StartupTasks.sync_due`), not the student, and
    /// why. Only `sync_all` takes it: the run happens only if it is still due, is counted as an
    /// attempt first, leaves out the sources that need the student, never downloads files (the
    /// fields above are ignored) and keeps a failure that may pass by itself quiet
    /// (`auto_sync`). `None`: the student started it.
    pub automatic: Option<AutoSyncTrigger>,
}

impl Default for SyncRequest {
    fn default() -> Self {
        SyncRequest {
            download_files: false,
            max_file_mb: 50,
            only_courses: Vec::new(),
            automatic: None,
        }
    }
}

/// Progress stream of a sync run (desktop forwards these through a `tauri::ipc::Channel`).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncEvent {
    SourceStarted {
        source_id: String,
        label: String,
    },
    Progress {
        source_id: String,
        /// English, for the CLI and logs; the UIs translate `stage` when it is set.
        message: String,
        current: Option<u32>,
        total: Option<u32>,
        stage: Option<SyncStage>,
        /// The course the step is about (its code, else its name).
        course: Option<String>,
    },
    Warning {
        source_id: String,
        message: String,
    },
    SourceFinished {
        source_id: String,
        ok: bool,
        error: Option<String>,
        error_kind: Option<SourceErrorKind>,
    },
}

/// Outcome of syncing one source. A failing source is `ok: false` (not an `Err`), so
/// `sync_all` can report partial success.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SourceSyncResult {
    pub source_id: String,
    pub label: String,
    pub kind: SourceKind,
    pub ok: bool,
    pub error: Option<String>,
    pub error_kind: Option<SourceErrorKind>,
    pub started_at: Timestamp,
    pub finished_at: Timestamp,
    pub courses: u32,
    pub modules: u32,
    pub materials: u32,
    pub files_downloaded: u32,
    pub files_indexed: u32,
    pub events: u32,
    pub warnings: Vec<String>,
    /// Per-course details (Canvas; empty for other sources).
    pub course_summaries: Vec<CourseSyncSummary>,
    /// HTTP requests made (Canvas; None for other sources).
    pub requests: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SyncSummary {
    pub started_at: Timestamp,
    pub finished_at: Timestamp,
    /// True when every source synced successfully.
    pub ok: bool,
    pub results: Vec<SourceSyncResult>,
}

// ---------------------------------------------------------------------------------------------
// "Connect your AI app"
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpClient {
    ClaudeDesktop,
    ClaudeCode,
    Codex,
    Generic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    /// Merge `content` into a JSON config file.
    JsonSnippet,
    /// Run `content` in a terminal.
    ShellCommand,
    /// Append `content` to a TOML config file.
    TomlSnippet,
}

/// How an MCP client launches the server — the single source every snippet (and, later,
/// a `.mcpb` Desktop Extension manifest) is generated from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct McpLaunch {
    /// Absolute path of the `pagelamp` binary.
    pub command: String,
    pub args: Vec<String>,
    /// Extra environment (only `PAGELAMP_HOME` when the data dir is not the default).
    pub env: BTreeMap<String, String>,
    /// Set when `command` lives somewhere it won't be found later; every config then starts
    /// with a `RunFromTemporaryLocation` note.
    pub temporary_location: Option<TemporaryLocation>,
}

/// A place the `pagelamp` binary runs from that won't exist (or move) later, so an AI app
/// configured with that path loses the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TemporaryLocation {
    /// macOS: opened from the downloaded disk image (a read-only volume under `/Volumes`).
    #[serde(rename = "disk_image")]
    DiskImage,
    /// macOS: a randomised read-only copy (App Translocation) of an app that was not moved
    /// out of Downloads yet.
    #[serde(rename = "translocated")]
    Translocated,
    /// Linux: inside an AppImage, which is mounted at a new path every launch.
    #[serde(rename = "appimage")]
    AppImage,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct McpClientConfig {
    pub client: McpClient,
    /// e.g. "Claude Desktop".
    pub title: String,
    pub install_kind: InstallKind,
    /// Where the snippet goes, e.g. "~/Library/Application Support/Claude/claude_desktop_config.json".
    pub config_path_hint: Option<String>,
    /// The snippet / command to copy.
    pub content: String,
    /// Facts the student should know (plan availability, restart the app, …), in English.
    pub notes: Vec<String>,
    /// Machine-readable code of each entry of `notes` (same length, same order) so UIs can
    /// localise.
    pub note_codes: Vec<McpNoteCode>,
    pub launch: McpLaunch,
}

/// Stable codes for `McpClientConfig.notes` (one code per note).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpNoteCode {
    /// Claude Desktop: works on every Claude plan, including Free.
    WorksOnAllClaudePlans,
    /// Claude Desktop: admins of Team/Enterprise/Education workspaces can disable extensions.
    AdminsMayDisableExtensions,
    /// Claude Code: needs a paid Claude plan (Pro or higher).
    NeedsPaidClaudePlan,
    /// Codex: the ChatGPT desktop app (Work/Codex mode) reads the same ~/.codex/config.toml.
    CodexConfigSharedWithChatgptDesktop,
    /// Codex: documented for ChatGPT Plus and higher, plus Edu.
    CodexPlusAndEduDocumented,
    /// Codex: Free/Go support is undocumented.
    FreeGoUndocumented,
    /// Restart / reload the AI app after changing its config.
    RestartClientAfterChange,
    /// Claude Desktop: quit it completely before editing its config (it rewrites the file when
    /// it quits), then paste, save and open it again.
    QuitBeforeEditing,
    /// The snippet sets PAGELAMP_HOME because a non-default data directory is in use.
    CustomDataDir,
    /// Generic stdio MCP client: adapt the command/args/env to that client's config format.
    GenericStdioClient,
    /// PageLamp runs from a place its binary won't be found at later (macOS: the disk image
    /// or an App Translocation copy → move it to Applications and copy again; Linux: inside an
    /// AppImage → use the .deb/.rpm or the command-line archive). Always the first note.
    RunFromTemporaryLocation,
}

// ---------------------------------------------------------------------------------------------
// The facade
// ---------------------------------------------------------------------------------------------

/// Cheap to clone; holds the data directory and the secret backend. Every call opens its
/// own short-lived SQLite connection (see docs/ARCHITECTURE.md §4), so one `App` can be shared
/// by all threads/tasks without a mutex.
#[derive(Clone)]
pub struct App {
    data_dir: PathBuf,
    secrets: Arc<dyn SecretBackend>,
    /// Per-process state shared by every clone.
    state: Arc<AppState>,
}

/// What an `App` and its clones share within one process.
#[derive(Default)]
pub(crate) struct AppState {
    /// This launch's classification (`updates`), computed once.
    launch: std::sync::Mutex<Option<updates::LaunchClass>>,
    /// Which shell opened the app (`App::set_shell`); What's new is per shell.
    shell: std::sync::Mutex<updates::Shell>,
    /// Running syncs and downloads (`activity`).
    activity: activity::Registry,
    /// The `pagelamp` executable that runs extraction workers (`set_extract_worker`).
    extract_worker: std::sync::RwLock<Option<PathBuf>>,
    /// The stop request of the sync this app is running, if any (`cancel_sync`).
    sync_cancel: std::sync::Mutex<Option<pagelamp_core::source::CancelFlag>>,
    /// Whether the data dir had been used when this app opened it (sources, or a
    /// pre-migration backup), for the launch classification (`updates`): read at open, since
    /// the first-run screens add a source before the shell asks for its startup tasks.
    used_before_at_open: bool,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("data_dir", &self.data_dir)
            .finish_non_exhaustive()
    }
}

impl App {
    /// Default data dir (`PAGELAMP_HOME` respected). Creates it and migrates the DB.
    pub fn open() -> Result<App> {
        App::open_at(paths::data_dir()?)
    }

    /// Explicit data dir (tests, portable installs). Creates it and migrates the DB.
    pub fn open_at(data_dir: PathBuf) -> Result<App> {
        App::open_at_with_secrets(data_dir, Arc::new(KeychainSecrets))
    }

    /// Like `open_at`, with a custom secret backend — for tests and embedders
    /// (`pagelamp_core::secrets::MemorySecrets` keeps everything in memory).
    pub fn open_at_with_secrets(data_dir: PathBuf, secrets: Arc<dyn SecretBackend>) -> Result<App> {
        paths::ensure_dirs_in(&data_dir).map_err(|err| {
            AppError::new(
                AppErrorKind::Internal,
                format!(
                    "could not create the data folder {}: {err}",
                    data_dir.display()
                ),
            )
        })?;
        let db_path = paths::db_path_in(&data_dir);
        // Create + migrate, then only read: the launch itself is classified (and the running
        // version recorded) by the first `startup_tasks`, never by a CLI or MCP process.
        let store = Store::open(&db_path)?;
        let used_before_at_open =
            !store.list_sources()?.is_empty() || store.last_migration_backup()?.is_some();
        drop(store);
        let app = App {
            data_dir,
            secrets,
            state: Arc::new(AppState {
                used_before_at_open,
                ..AppState::default()
            }),
        };
        // Courses synced by an older version are in its logs but not remembered yet.
        app.remember_course_names();
        Ok(app)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Where `pagelamp extract-worker` is (v0.3 M0.5). Syncs then read every file in a
    /// separate, resource-limited process started from it, so a hostile or broken file costs
    /// that process, never the app. `None` (the default) reads files in this process. Each
    /// surface sets it once at start-up: the desktop app its `pagelamp` sidecar, the CLI
    /// `current_exe()`, the macOS app `Bundle.main.url(forAuxiliaryExecutable:)`. Shared by
    /// every clone of this `App`.
    pub fn set_extract_worker(&self, path: Option<PathBuf>) {
        *self
            .state
            .extract_worker
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = path;
    }

    /// The executable set with `set_extract_worker`.
    pub fn extract_worker(&self) -> Option<PathBuf> {
        self.state
            .extract_worker
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// A fresh extractor for one sync (`ingest::Extractor`).
    /// A fresh extractor for one sync, stopped by `cancel` (`ingest::Extractor`).
    pub(crate) fn extractor(
        &self,
        cancel: pagelamp_core::source::CancelFlag,
    ) -> pagelamp_core::ingest::Extractor {
        let extractor = match self.extract_worker() {
            Some(exe) => pagelamp_core::ingest::Extractor::worker(exe),
            None => pagelamp_core::ingest::Extractor::in_process(),
        };
        extractor.cancellable(cancel)
    }

    /// `<data_dir>/pagelamp.db`
    pub fn db_path(&self) -> PathBuf {
        paths::db_path_in(&self.data_dir)
    }

    fn read_store(&self) -> Result<Store> {
        Ok(Store::open_read_only(&self.db_path())?)
    }

    fn write_store(&self) -> Result<Store> {
        Ok(Store::open(&self.db_path())?)
    }

    // ----- status & sources ----------------------------------------------------------------

    pub fn status(&self) -> Result<AppStatus> {
        let store = self.read_store()?;
        let sources = store.list_sources()?;
        Ok(AppStatus {
            version: env!("CARGO_PKG_VERSION").to_string(),
            data_dir: self.data_dir.display().to_string(),
            db_path: self.db_path().display().to_string(),
            last_synced_at: sources.iter().filter_map(|s| s.last_synced_at).max(),
            counts: store.counts()?,
            auto_sync: pagelamp_core::auto_sync::auto_sync(&store)?,
            deadlines_synced_at: pagelamp_core::auto_sync::light_sync(&store)?
                .later_than_full(&sources),
            sources,
            sync_in_progress: lock::is_locked(&paths::sync_lock_path_in(&self.data_dir)),
        })
    }

    pub fn list_sources(&self) -> Result<Vec<SourceRecord>> {
        Ok(self.read_store()?.list_sources()?)
    }

    /// Validates the token (`GET /api/v1/users/self`), stores it in the keychain, creates the
    /// source `canvas:<host>`. Personal-use only — callers must show the notice.
    pub async fn add_canvas_source(&self, base_url: &str, token: &str) -> Result<SourceRecord> {
        let base_url = pagelamp_canvas::normalize_base_url(base_url)
            .map_err(|err| AppError::new(AppErrorKind::Invalid, err.message))?;
        let config = CanvasConfig {
            base_url: base_url.clone(),
            token: non_empty_secret(token, "Canvas access token")?,
        };
        // The display name only (never email or ids) — shown as "Connected as …".
        let account_name = pagelamp_canvas::check_token(&config).await?;
        let id = pagelamp_canvas::source_id(&base_url);
        let label = base_url
            .split_once("://")
            .map_or(base_url.as_str(), |(_, host)| host)
            .to_string();
        self.save_source_with_secret(
            SourceRecord {
                id,
                kind: SourceKind::Canvas,
                label,
                config: json!({ "base_url": base_url, "account_name": account_name }),
                last_synced_at: None,
                last_error: None,
                last_error_kind: None,
            },
            &config.token,
        )
    }

    /// Adds a local course folder (`<root>/<COURSE>/…`). Adding the same folder again returns
    /// the existing source (updating its label/term start when given).
    pub fn add_folder_source(
        &self,
        path: &Path,
        term_start: Option<NaiveDate>,
        label: Option<&str>,
    ) -> Result<SourceRecord> {
        let invalid = || {
            AppError::new(
                AppErrorKind::Invalid,
                format!("'{}' is not a folder", path.display()),
            )
        };
        if !path.is_dir() {
            return Err(invalid());
        }
        let root = std::fs::canonicalize(path).map_err(|_| invalid())?;
        let root_text = root.to_string_lossy().to_string();
        let id = format!("folder:{}", short_hash(&root_text));
        let store = self.write_store()?;
        let existing = store.get_source(&id)?;
        let mut config = existing
            .as_ref()
            .map_or_else(|| json!({}), |s| s.config.clone());
        config["path"] = json!(root_text);
        if let Some(start) = term_start {
            config["term_start"] = json!(start.format("%Y-%m-%d").to_string());
        }
        let label = match (label.map(str::trim).filter(|l| !l.is_empty()), &existing) {
            (Some(label), _) => label.to_string(),
            (None, Some(existing)) => existing.label.clone(),
            // The folder's own name: short, and doesn't put the local path (user name) into
            // labels that the MCP server shows to the AI app. The full path is in `config`.
            (None, None) => root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| display_path(&root)),
        };
        store.upsert_source(&SourceRecord {
            id: id.clone(),
            kind: SourceKind::Folder,
            label,
            config,
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })?;
        Ok(store.get_source(&id)?.expect("source was just saved"))
    }

    /// Validates the feed by fetching it, stores the URL in the keychain.
    pub async fn add_ical_source(
        &self,
        feed_url: &str,
        label: Option<&str>,
    ) -> Result<SourceRecord> {
        let feed_url = normalized_feed_url(feed_url)?;
        pagelamp_local::fetch_ical(&feed_url).await?;
        let label = label
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .unwrap_or("Calendar feed")
            .to_string();
        self.save_source_with_secret(
            SourceRecord {
                id: format!("ical:{}", short_hash(&feed_url)),
                kind: SourceKind::Ical,
                label,
                config: json!({}),
                last_synced_at: None,
                last_error: None,
                last_error_kind: None,
            },
            &feed_url,
        )
    }

    /// Removes the source, everything synced from it, the files downloaded for its Canvas
    /// courses and its keychain secret. A folder source has no secret, and the student's own
    /// folder is never touched. `Busy` while a sync runs (it could write those files).
    pub fn remove_source(&self, source_id: &str) -> Result<()> {
        let _lock = SyncLock::acquire(&paths::sync_lock_path_in(self.data_dir()))?;
        let store = self.write_store()?;
        let Some(source) = store.get_source(source_id)? else {
            return Err(unknown_source(source_id));
        };
        // Its course names may be in the logs: keep them for the report's pseudonymisation.
        self.remember_course_names();
        if source.kind == SourceKind::Canvas {
            // Files first: if deleting fails, the source stays and removing can be retried.
            self.remove_downloaded_files(&store, source_id)?;
        }
        // Before the row goes: if this fails the source stays and removing can be retried.
        self.forget_light_sync(&store, source_id)?;
        store.remove_source(source_id)?;
        if source.kind != SourceKind::Folder {
            self.secrets.delete(source_id)?;
        }
        // The pre-update backup holds course text too: it goes with the last source.
        if store.list_sources()?.is_empty() {
            pagelamp_core::store::delete_database_backups(&self.db_path())
                .map_err(pagelamp_core::Error::from)?;
        }
        Ok(())
    }

    /// Record the current course names for the diagnostic report (`diagnostics`). Failing
    /// only weakens the report's pseudonymisation, so it is logged, not returned.
    pub(crate) fn remember_course_names(&self) {
        let remembered = self
            .read_store()
            .and_then(|store| Ok(store.list_courses(true)?))
            .and_then(|courses| {
                diagnostics::remember_courses(self.data_dir(), &courses)
                    .map_err(|err| AppError::new(AppErrorKind::Internal, err.to_string()))
            });
        if let Err(err) = remembered {
            tracing::warn!("could not update the course alias list ({:?})", err.kind);
        }
    }

    /// Delete the download directories of every course of this Canvas source, all direct
    /// children of `files/`: its current `<CODE>-<id>/`, the directories its materials' local
    /// copies are in, and any other `…-<id>/` (the same course under an older code) unless
    /// another source has a course with that id. A directory another source's course uses is
    /// kept (compared case-insensitively and in NFC, as APFS and NTFS do).
    fn remove_downloaded_files(&self, store: &Store, source_id: &str) -> Result<()> {
        let files_dir = paths::files_dir_in(self.data_dir());
        let dirs_of = |course: &Course| -> Result<Vec<PathBuf>> {
            let mut dirs = vec![pagelamp_canvas::course_files_dir(
                &files_dir,
                course.code.as_deref(),
                &course.external_id,
            )];
            for material in store.list_materials(&course.id)? {
                let parent = material
                    .local_path
                    .as_deref()
                    .and_then(|p| Path::new(p).parent())
                    .filter(|parent| parent.parent() == Some(files_dir.as_path()));
                if let Some(parent) = parent {
                    dirs.push(parent.to_path_buf());
                }
            }
            Ok(dirs)
        };
        let suffix_of =
            |course: &Course| fold_name(&pagelamp_canvas::course_dir_suffix(&course.external_id));
        let mut own = Vec::new();
        let mut own_suffixes = std::collections::HashSet::new();
        let mut kept = std::collections::HashSet::new();
        let mut other_suffixes = std::collections::HashSet::new();
        for course in store.list_courses(true)? {
            let dirs = dirs_of(&course)?;
            if course.source_id == source_id {
                own.extend(dirs);
                own_suffixes.insert(suffix_of(&course));
            } else {
                kept.extend(dirs.iter().map(|dir| dir_key(dir)));
                other_suffixes.insert(suffix_of(&course));
            }
        }
        // The same courses under older codes: `<OLDCODE>-<id>`.
        own_suffixes.retain(|suffix| !other_suffixes.contains(suffix));
        if let Ok(entries) = std::fs::read_dir(&files_dir) {
            for entry in entries.flatten() {
                let name = fold_name(&entry.file_name().to_string_lossy());
                if own_suffixes
                    .iter()
                    .any(|suffix| name.ends_with(suffix.as_str()))
                {
                    own.push(entry.path());
                }
            }
        }
        own.sort();
        own.dedup();
        for dir in own.iter().filter(|dir| !kept.contains(&dir_key(dir))) {
            remove_download_dir(dir).map_err(|err| {
                AppError::new(
                    AppErrorKind::Internal,
                    format!(
                        "Could not delete the downloaded course files in {}: {err}",
                        pagelamp_core::diagnostics::shorten_home(&dir.display().to_string())
                    ),
                )
            })?;
        }
        Ok(())
    }

    /// Replace an expired/revoked Canvas token or a changed feed URL without removing the
    /// source (remove would cascade to courses + user overrides). Validates like `add_*`,
    /// overwrites the keychain entry, clears last_error/last_error_kind.
    pub async fn update_source_secret(
        &self,
        source_id: &str,
        secret: &str,
    ) -> Result<SourceRecord> {
        let source = self
            .read_store()?
            .get_source(source_id)?
            .ok_or_else(|| unknown_source(source_id))?;
        let mut secret = non_empty_secret(secret, "secret")?;
        let mut account_name = None;
        match source.kind {
            SourceKind::Folder => {
                return Err(AppError::new(
                    AppErrorKind::Invalid,
                    "folder sources have no secret",
                ));
            }
            SourceKind::Canvas => {
                let base_url = canvas_base_url(&source)?;
                // The new token may belong to another account: refresh "Connected as …".
                account_name = Some(
                    pagelamp_canvas::check_token(&CanvasConfig {
                        base_url,
                        token: secret.clone(),
                    })
                    .await?,
                );
            }
            SourceKind::Ical => {
                secret = normalized_feed_url(&secret)?;
                pagelamp_local::fetch_ical(&secret).await?;
            }
        }
        self.secrets.set(source_id, &secret)?;
        let store = self.write_store()?;
        if let Some(name) = account_name {
            let mut updated = source;
            updated.config["account_name"] = json!(name);
            store.upsert_source(&updated)?; // label/config only; sync state untouched
        }
        store.clear_source_error(source_id)?;
        store
            .get_source(source_id)?
            .ok_or_else(|| unknown_source(source_id))
    }

    /// Save `source` and its secret: secret first, and removed again if saving the source
    /// fails, so a secret never exists without its source row.
    fn save_source_with_secret(&self, source: SourceRecord, secret: &str) -> Result<SourceRecord> {
        self.secrets.set(&source.id, secret)?;
        let saved = self.write_store().and_then(|store| {
            store.upsert_source(&source)?;
            Ok(store
                .get_source(&source.id)?
                .expect("source was just saved"))
        });
        if saved.is_err() {
            // Best effort; the original error is the one to report.
            let _ = self.secrets.delete(&source.id);
        }
        saved
    }

    // ----- read views ------------------------------------------------------------------------
    // Hidden courses are included/addressable here (the desktop app shows them behind a
    // toggle); the MCP server calls the views with `include_hidden = false`.

    /// All courses INCLUDING hidden ones (check `course.hidden`).
    pub fn list_courses(&self) -> Result<Vec<CourseSummary>> {
        Ok(views::list_courses(
            &self.read_store()?,
            true,
            AsOf::now_local(),
        )?)
    }

    pub fn course_overview(&self, course: &str) -> Result<CourseOverview> {
        Ok(views::course_overview(
            &self.read_store()?,
            course,
            true,
            AsOf::now_local(),
        )?)
    }

    pub fn week_materials(&self, course: &str, week: Option<u32>) -> Result<WeekMaterials> {
        Ok(views::week_materials(
            &self.read_store()?,
            course,
            week,
            true,
            AsOf::now_local(),
        )?)
    }

    /// With a course: that course's events (hidden courses too). Without: every non-hidden
    /// course's events plus events not linked to a course.
    pub fn list_deadlines(
        &self,
        course: Option<&str>,
        days_ahead: u32,
        days_back: u32,
    ) -> Result<Vec<Deadline>> {
        let store = self.read_store()?;
        Ok(views::deadlines(
            &store,
            course,
            days_ahead,
            days_back,
            true,
            AsOf::now_local(),
        )?)
    }

    /// The student's own search over every non-hidden course (whatever its AI access).
    pub fn search(&self, query: &str, course: Option<&str>, limit: u32) -> Result<Vec<SearchHit>> {
        Ok(views::search(&self.read_store()?, query, course, limit)?)
    }

    pub fn latest_study_plan(&self) -> Result<Option<StoredStudyPlan>> {
        Ok(self.read_store()?.latest_study_plan()?)
    }

    // ----- course settings (course = id or code; hidden courses are addressable) -------------

    pub fn set_course_policy(
        &self,
        course: &str,
        policy: AiPolicy,
        note: Option<&str>,
    ) -> Result<()> {
        let store = self.write_store()?;
        let course = store.resolve_course_with(course, true)?;
        let note = note.map(str::trim).filter(|n| !n.is_empty());
        Ok(store.set_course_policy(&course.id, policy, note)?)
    }

    /// Set/clear the student's term override (`None, None` falls back to the synced dates).
    /// `start` is the first day of classes and `end` the last day of classes. Saving dates
    /// confirms them (they are no longer `legacy`, calendar design §3.2).
    pub fn set_course_term(
        &self,
        course: &str,
        start: Option<NaiveDate>,
        end: Option<NaiveDate>,
    ) -> Result<()> {
        let store = self.write_store()?;
        let course = store.resolve_course_with(course, true)?;
        store.in_transaction(|store| {
            store.set_course_term(&course.id, start, end)?;
            course::set_dates_confirmed(store, &course.id, start.is_some() || end.is_some())
        })?;
        Ok(())
    }

    /// The per-course switch "Let my AI app read this course's materials" (docs/ARCHITECTURE.md
    /// §3 rule 8). A `prohibited` AI policy withholds text regardless of this switch.
    pub fn set_course_ai_access(&self, course: &str, allowed: bool) -> Result<()> {
        let store = self.write_store()?;
        let course = store.resolve_course_with(course, true)?;
        Ok(store.set_course_ai_access(&course.id, allowed)?)
    }

    pub fn set_course_hidden(&self, course: &str, hidden: bool) -> Result<()> {
        let store = self.write_store()?;
        let course = store.resolve_course_with(course, true)?;
        Ok(store.set_course_hidden(&course.id, hidden)?)
    }

    // ----- diagnostics (see the `diagnostics` module; these use this App's data dir) ----------

    /// `<data_dir>/logs` (created if missing) — for "Open logs folder".
    pub fn logs_dir(&self) -> Result<PathBuf> {
        diagnostics::logs_dir_in(&self.data_dir)
    }

    pub fn last_crash(&self) -> Result<Option<diagnostics::CrashReport>> {
        Ok(pagelamp_core::diagnostics::last_crash(&self.data_dir)?)
    }

    pub fn clear_last_crash(&self) -> Result<()> {
        Ok(pagelamp_core::diagnostics::clear_last_crash(
            &self.data_dir,
        )?)
    }

    /// What the last database update did about its backup copy (`None`: never updated, or
    /// not readable). Shown by `doctor` and in diagnostic reports.
    pub fn last_migration_backup(&self) -> Option<pagelamp_core::model::MigrationBackupRecord> {
        diagnostics::last_migration_backup_in(self.data_dir())
    }

    /// Facts for helping a student, including a check of the extraction worker (it is started
    /// once, which takes a moment).
    pub fn doctor(&self) -> Result<diagnostics::DoctorReport> {
        Ok(diagnostics::doctor_in(
            &self.data_dir,
            self.secrets.as_ref(),
            self.extract_worker().as_deref(),
        ))
    }

    /// Markdown for an issue (doctor + last crash + recent log lines), redacted and with
    /// course names pseudonymised. Shown to the student before they share it.
    pub fn diagnostic_report(&self) -> Result<String> {
        Ok(diagnostics::report_in(
            &self.data_dir,
            self.secrets.as_ref(),
            self.extract_worker().as_deref(),
        ))
    }

    // ----- "connect your AI app" ------------------------------------------------------------

    /// One config per client (claude_desktop, claude_code, codex, generic) for launching
    /// `<pagelamp_binary> mcp`. `PAGELAMP_HOME` is included only when this App's data dir
    /// is not the platform default.
    pub fn mcp_client_configs(&self, pagelamp_binary: &Path) -> Vec<McpClientConfig> {
        mcp_config::client_configs(
            &self.mcp_launch(pagelamp_binary),
            mcp_config::Shell::current(),
            mcp_config::claude_desktop_config_hint(),
        )
    }

    /// How an MCP client launches this App's server (the source of every snippet).
    pub fn mcp_launch(&self, pagelamp_binary: &Path) -> McpLaunch {
        let command =
            std::path::absolute(pagelamp_binary).unwrap_or_else(|_| pagelamp_binary.to_path_buf());
        let mut env = BTreeMap::new();
        if !same_dir(Some(&self.data_dir), paths::platform_data_dir().as_deref()) {
            env.insert(
                paths::HOME_ENV.to_string(),
                self.data_dir.display().to_string(),
            );
        }
        let temporary_location =
            mcp_config::temporary_location(&command, &mcp_config::LaunchEnv::current());
        McpLaunch {
            command: command.display().to_string(),
            args: vec!["mcp".to_string()],
            env,
            temporary_location,
        }
    }
}

// ----- facade helpers ---------------------------------------------------------------------------

/// A download directory's name as the file system compares it: case-insensitive, NFC.
fn dir_key(dir: &Path) -> String {
    dir.file_name()
        .map(|name| fold_name(&name.to_string_lossy()))
        .unwrap_or_default()
}

/// `name` in NFC and lower case.
fn fold_name(name: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    name.nfc().collect::<String>().to_lowercase()
}

/// Remove one download directory: a directory with everything in it, a symbolic link itself
/// (never what it points to); anything else (a stray file) is left alone.
fn remove_download_dir(dir: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(dir) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let removed = if metadata.file_type().is_symlink() {
        std::fs::remove_file(dir)
    } else if metadata.is_dir() {
        std::fs::remove_dir_all(dir)
    } else {
        return Ok(());
    };
    match removed {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn unknown_source(source_id: &str) -> AppError {
    AppError::new(AppErrorKind::NotFound, format!("no source '{source_id}'"))
}

/// Trimmed secret, or `Invalid` when empty. The message names the field, never the value.
fn non_empty_secret(secret: &str, what: &str) -> Result<String> {
    let secret = secret.trim();
    if secret.is_empty() {
        return Err(AppError::new(
            AppErrorKind::Invalid,
            format!("the {what} is empty"),
        ));
    }
    Ok(secret.to_string())
}

/// A usable feed URL (`webcal://` → `https://`), or `Invalid` (the message never repeats it).
fn normalized_feed_url(input: &str) -> Result<String> {
    let input = non_empty_secret(input, "calendar feed URL")?;
    pagelamp_local::normalize_feed_url(&input)
        .map_err(|err| AppError::new(AppErrorKind::Invalid, err.message))
}

fn canvas_base_url(source: &SourceRecord) -> Result<String> {
    source
        .config
        .get("base_url")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| {
            AppError::new(
                AppErrorKind::Internal,
                format!("source '{}' has no base_url", source.id),
            )
        })
}

/// First 12 hex chars of SHA-256 — short, stable source ids that don't reveal the input
/// (feed URLs are secrets).
fn short_hash(text: &str) -> String {
    pagelamp_core::ingest::sha256_hex(text.as_bytes())[..12].to_string()
}

/// `path` with the home directory shortened to `~` (labels only).
fn display_path(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Same directory, comparing canonical forms when they exist.
fn same_dir(a: Option<&Path>, b: Option<&Path>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            a == b
                || matches!(
                    (std::fs::canonicalize(a), std::fs::canonicalize(b)),
                    (Ok(a), Ok(b)) if a == b
                )
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------------------------
// JSON Schema export (`pagelamp schema`)
// ---------------------------------------------------------------------------------------------

/// Container whose only purpose is to pull every facade type into one schema document
/// (`$defs`), so json-schema-to-typescript emits one TS file with all of them.
#[allow(dead_code)]
#[derive(JsonSchema)]
#[schemars(title = "PageLampAppTypes")]
struct AppTypes {
    app_error: AppError,
    app_status: AppStatus,
    source_record: SourceRecord,
    source_error_kind: SourceErrorKind,
    sync_request: SyncRequest,
    sync_event: SyncEvent,
    sync_summary: SyncSummary,
    source_sync_result: SourceSyncResult,
    course_summary: CourseSummary,
    course_overview: CourseOverview,
    week_materials: WeekMaterials,
    deadline: Deadline,
    search_hit: SearchHit,
    stored_study_plan: StoredStudyPlan,
    ai_policy: AiPolicy,
    ai_materials_state: AiMaterialsState,
    term_source: TermSource,
    mcp_client_config: McpClientConfig,
    doctor_report: diagnostics::DoctorReport,
    extract_worker_check: diagnostics::ExtractWorkerCheck,
    extract_worker_status: diagnostics::ExtractWorkerStatus,
    unreadable_files: diagnostics::UnreadableFiles,
    text_error_kind: pagelamp_core::model::TextErrorKind,
    crash_report: diagnostics::CrashReport,
    process_kind: diagnostics::ProcessKind,
    course_sync_summary: CourseSyncSummary,
    update_prefs: UpdatePrefs,
    update_channel: UpdateChannel,
    startup_tasks: StartupTasks,
    whats_new: WhatsNew,
    whats_new_topic: WhatsNewTopic,
    update_check_record: UpdateCheckRecord,
    update_check_outcome: UpdateCheckOutcome,
    sync_prefs: SyncPrefs,
    auto_sync: AutoSync,
    auto_sync_trigger: AutoSyncTrigger,
    sync_due: SyncDue,
    activity: Activity,
    activity_item: ActivityItem,
    activity_kind: ActivityKind,
    // AI (v0.3 M1)
    ai_feature: pagelamp_core::ai::AiFeature,
    block_reason: pagelamp_core::ai::BlockReason,
    model_error_kind: pagelamp_core::ai::ModelErrorKind,
    effort: pagelamp_core::ai::Effort,
    material_sharing: pagelamp_core::ai::MaterialSharing,
    ai_status: ai::AiStatus,
    ai_backend_status: ai::AiBackendStatus,
    backend_ref: ai::BackendRef,
    backend_kind: ai::BackendKind,
    backend_state: ai::BackendState,
    backend_problem: ai::BackendProblem,
    model_choice: ai::ModelChoice,
    feature_routing: ai::FeatureRouting,
    budget_status: ai::BudgetStatus,
    disclosure_facts: ai::DisclosureFacts,
    sent_data: ai::SentData,
    recipient: ai::Recipient,
    training_fact: ai::TrainingFact,
    retention_fact: ai::RetentionFact,
    cost_kind: ai::CostKind,
    provider_wire: ai::ProviderWire,
    provider_preset: ai::ProviderPreset,
    model_provider_record: ai::ModelProviderRecord,
    local_server: ai::LocalServer,
    local_server_kind: ai::LocalServerKind,
    model_info: ai::ModelInfo,
    structured_output_tier: ai::StructuredOutputTier,
    probe_report: ai::ProbeReport,
    estimate_request: ai::EstimateRequest,
    cost_estimate: ai::CostEstimate,
    token_usage: ai::TokenUsage,
    usage_row: ai::UsageRow,
    cost_basis: ai::CostBasis,
    usage_summary: ai::UsageSummary,
    remove_ai_data_report: ai::RemoveAiDataReport,
    gen_stage: ai::GenStage,
    gen_notice_code: ai::GenNoticeCode,
    gen_event: ai::GenEvent,
    generation_meta: ai::GenerationMeta,
    // Course (v0.3 M0.10)
    course_timeline: pagelamp_core::model::CourseTimeline,
    lifecycle_summary: LifecycleSummary,
    snooze_kind: pagelamp_core::model::SnoozeKind,
    evidence_code: pagelamp_core::model::EvidenceCode,
    evidence_signal: pagelamp_core::model::EvidenceSignal,
    // alpha.2 (types first; methods with schema v4)
    removal_preview: RemovalPreview,
    remove_options: RemoveOptions,
    removal_report: RemovalReport,
    restore_outcome: RestoreOutcome,
    purge_report: PurgeReport,
    course_dates_input: CourseDatesInput,
    // alpha.3 (F3: types first; AI reading and storage on the v4 line)
    course_calendar_view: CourseCalendarView,
    calendar_candidate: pagelamp_core::calendar::candidates::CalendarCandidate,
    syllabus_offer: SyllabusOffer,
    read_calendar_options: ReadCalendarOptions,
    calendar_run_outcome: CalendarRunOutcome,
    calendar_batch_event: CalendarBatchEvent,
    calendar_proposal: pagelamp_core::calendar::proposal::CalendarProposal,
}

/// JSON Schema (draft 2020-12) of every type crossing the facade, as one document.
pub fn json_schema() -> serde_json::Value {
    let mut settings = schemars::generate::SchemaSettings::draft2020_12();
    settings.meta_schema = Some("https://json-schema.org/draft/2020-12/schema".into());
    let schema = settings.into_generator().into_root_schema_for::<AppTypes>();
    schema.to_value()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The desktop app and the CLI both use `App::open()`, i.e. `paths::data_dir()`; config
    /// snippets for the default location must therefore NOT pin PAGELAMP_HOME, so the MCP
    /// server started by an AI app resolves the same folder by itself.
    #[test]
    fn default_data_dir_needs_no_pagelamp_home() {
        let temp = tempfile::tempdir().unwrap();
        let custom = temp.path().join("data");
        std::fs::create_dir_all(&custom).unwrap();
        assert!(same_dir(Some(&custom), Some(&custom)));
        assert!(same_dir(Some(&custom), Some(&temp.path().join("data/./"))));
        assert!(!same_dir(Some(&custom), Some(temp.path())));
        assert!(!same_dir(Some(&custom), None));

        let app = App {
            data_dir: custom.clone(),
            secrets: Arc::new(pagelamp_core::secrets::MemorySecrets::new()),
            state: Arc::default(),
        };
        let launch = app.mcp_launch(Path::new("/demo/pagelamp"));
        let expected_env = !same_dir(Some(&custom), paths::platform_data_dir().as_deref());
        assert_eq!(launch.env.contains_key(paths::HOME_ENV), expected_env);
    }
}
