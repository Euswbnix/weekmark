//! UniFFI bindings of the PageLamp facade (`pagelamp-app`) for the native macOS app.
//!
//! This crate is plumbing, not a second facade: every method forwards to one `App` method
//! (`docs/design/macos-shell.md` §13). It adds only what the language boundary needs:
//!
//! - **Mirrors** of the facade types (`types.rs`), checked against the real definitions at
//!   compile time, plus custom types for chrono / JSON / `BTreeMap`.
//! - **`PageLampError`** (`error.rs`): `AppError` as a Swift-`switch`able enum.
//! - **Threading**: every facade call is `async` and returns `Result`; only the cheap helpers
//!   `version`, `default_sync_request`, `init_diagnostics`, `log_ui_error` and the facade's
//!   constants (`not_now_days`, `keep_current_days`, `keep_forever`) are synchronous
//!   (and only those that cannot fail are non-throwing: Swift traps on a panic in a
//!   non-throwing export). Blocking facade calls (SQLite, file system) run with
//!   `spawn_blocking`, async ones (sync, Canvas/iCal checks) with `spawn`, both on this
//!   crate's own multi-thread tokio runtime (threads named
//!   `pagelamp-rt`). The Swift caller therefore never blocks its actor, and the returned
//!   futures do not care which executor polls them. Swift `Task` cancellation does not stop
//!   the Rust work (UniFFI has no cancellation); the facade has no cancel either.
//! - **Progress**: `SyncObserver` is implemented in Swift; `on_event` is called on a
//!   `pagelamp-rt` thread for every `SyncEvent` and must return quickly (hand the event to an
//!   `AsyncStream`: `SyncEventStream` in apps/macos/Sources/PageLampKit).
//! - **Panics** in facade code surface as `PageLampError::Panic` (caught at the task
//!   boundary), never as a trap: keep `panic = "unwind"` (the workspace release profile does).
//!
//! Secrets: `PageLamp::open` uses the OS keychain (production). `open_with_memory_secrets`
//! keeps secrets in memory only — for tests and SwiftUI previews; it never touches the keychain.

uniffi::setup_scaffolding!();

mod error;
mod types;

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use pagelamp_app::diagnostics::{CrashReport, DoctorReport, ProcessKind};
use pagelamp_app::{
    Activity, App, AppError, AppStatus, LifecycleSummary, McpClientConfig, McpLaunch, Shell,
    SourceSyncResult, StartupTasks, SyncEvent, SyncPrefs, SyncRequest, SyncSummary, UpdateChannel,
    UpdateCheckRecord, UpdatePrefs,
};
use pagelamp_core::lifecycle::SnoozeKind;
use pagelamp_core::model::{
    AiPolicy, Course, CourseTimeline, SearchHit, SourceRecord, StoredStudyPlan,
};
use pagelamp_core::secrets::MemorySecrets;
use pagelamp_core::views::{CourseOverview, CourseSummary, Deadline, WeekMaterials};

pub use crate::error::PageLampError;
use crate::error::join_error;
pub use crate::types::{EnvMap, IsoDate, JsonString, Timestamp};

type Result<T, E = PageLampError> = std::result::Result<T, E>;

// ---------------------------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------------------------

/// PageLamp's own runtime; lives for the whole process.
static RUNTIME: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("pagelamp-rt")
        .build()
        .expect("PageLamp could not start its worker threads")
});

/// Run a blocking facade call on the runtime's blocking pool.
async fn blocking<T, F>(call: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> std::result::Result<T, AppError> + Send + 'static,
{
    RUNTIME
        .spawn_blocking(call)
        .await
        .map_err(join_error)?
        .map_err(PageLampError::from)
}

/// Run an async facade call on the runtime.
async fn spawned<T, F>(call: F) -> Result<T>
where
    T: Send + 'static,
    F: Future<Output = std::result::Result<T, AppError>> + Send + 'static,
{
    RUNTIME
        .spawn(call)
        .await
        .map_err(join_error)?
        .map_err(PageLampError::from)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------------------------
// Sync progress (implemented in Swift)
// ---------------------------------------------------------------------------------------------

/// Receives the progress of `sync_all`, `sync_source` and `download_course_files`.
#[uniffi::export(foreign)]
pub trait SyncObserver: Send + Sync {
    /// Called on a PageLamp worker thread, in order, for every event of the run. Return
    /// quickly: the sync waits while this runs.
    fn on_event(&self, event: SyncEvent);
}

fn forward_to(observer: Arc<dyn SyncObserver>) -> impl Fn(SyncEvent) + Send + Sync + 'static {
    move |event| observer.on_event(event)
}

// ---------------------------------------------------------------------------------------------
// The facade object
// ---------------------------------------------------------------------------------------------

/// The PageLamp facade (`pagelamp_app::App`). Cheap to share: every call opens its own
/// short-lived database connection, so one instance serves the whole app from any thread.
#[derive(uniffi::Object)]
pub struct PageLamp {
    app: App,
}

#[uniffi::export]
impl PageLamp {
    /// Opens (creating and migrating) the data directory: `data_dir`, or the default one
    /// (`PAGELAMP_HOME`, else the platform folder) when nil. Secrets live in the OS keychain.
    #[uniffi::constructor]
    pub async fn open(data_dir: Option<String>) -> Result<Arc<Self>> {
        let app = blocking(move || match data_dir {
            Some(dir) => App::open_at(PathBuf::from(dir)),
            None => App::open(),
        })
        .await?;
        // This crate serves the Mac app: its own What's new (`pagelamp_app::Shell`).
        app.set_shell(Shell::Mac);
        Ok(Arc::new(Self { app }))
    }

    /// Like `open` with an explicit data directory, but secrets (Canvas tokens, feed URLs)
    /// are kept in memory only and never touch the keychain. For tests and previews.
    #[uniffi::constructor]
    pub async fn open_with_memory_secrets(data_dir: String) -> Result<Arc<Self>> {
        let app = blocking(move || {
            App::open_at_with_secrets(PathBuf::from(data_dir), Arc::new(MemorySecrets::new()))
        })
        .await?;
        app.set_shell(Shell::Mac);
        Ok(Arc::new(Self { app }))
    }

    // ----- paths ---------------------------------------------------------------------------

    pub async fn data_dir(&self) -> Result<String> {
        Ok(path_string(self.app.data_dir()))
    }

    /// `<data_dir>/pagelamp.db`
    pub async fn db_path(&self) -> Result<String> {
        Ok(path_string(&self.app.db_path()))
    }

    /// Where `pagelamp extract-worker` is (the bundled `pagelamp` executable,
    /// `Bundle.main.url(forAuxiliaryExecutable:)`): syncs then read every file in a separate,
    /// resource-limited process. Nil reads files in this process. Set it once after `open`.
    pub async fn set_extract_worker(&self, path: Option<String>) -> Result<()> {
        self.app.set_extract_worker(path.map(PathBuf::from));
        Ok(())
    }

    /// The executable set with `set_extract_worker`.
    pub async fn extract_worker(&self) -> Result<Option<String>> {
        Ok(self.app.extract_worker().as_deref().map(path_string))
    }

    // ----- status & sources ----------------------------------------------------------------

    pub async fn status(&self) -> Result<AppStatus> {
        let app = self.app.clone();
        blocking(move || app.status()).await
    }

    pub async fn list_sources(&self) -> Result<Vec<SourceRecord>> {
        let app = self.app.clone();
        blocking(move || app.list_sources()).await
    }

    /// Validates the token with the Canvas server, stores it in the secret store and creates
    /// the source `canvas:<host>`. Personal use only: callers must show the notice first.
    pub async fn add_canvas_source(&self, base_url: String, token: String) -> Result<SourceRecord> {
        let app = self.app.clone();
        spawned(async move { app.add_canvas_source(&base_url, &token).await }).await
    }

    /// Adds a local course folder (`<root>/<COURSE>/…`); adding the same folder again returns
    /// the existing source (updating its label/term start when given).
    pub async fn add_folder_source(
        &self,
        path: String,
        term_start: Option<IsoDate>,
        label: Option<String>,
    ) -> Result<SourceRecord> {
        let app = self.app.clone();
        blocking(move || app.add_folder_source(Path::new(&path), term_start, label.as_deref()))
            .await
    }

    /// Validates the feed by fetching it and stores its URL in the secret store.
    pub async fn add_ical_source(
        &self,
        feed_url: String,
        label: Option<String>,
    ) -> Result<SourceRecord> {
        let app = self.app.clone();
        spawned(async move { app.add_ical_source(&feed_url, label.as_deref()).await }).await
    }

    /// Removes the source, everything synced from it, its downloaded files and its secret.
    /// `Busy` while a sync runs.
    pub async fn remove_source(&self, source_id: String) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.remove_source(&source_id)).await
    }

    /// Replaces an expired token / changed feed URL in place (validated like `add_*`).
    pub async fn update_source_secret(
        &self,
        source_id: String,
        secret: String,
    ) -> Result<SourceRecord> {
        let app = self.app.clone();
        spawned(async move { app.update_source_secret(&source_id, &secret).await }).await
    }

    // ----- sync ----------------------------------------------------------------------------

    /// Syncs every source (`Busy` when another sync runs). A failing source is reported in
    /// the summary (`ok: false`), not thrown.
    pub async fn sync_all(
        &self,
        request: SyncRequest,
        observer: Arc<dyn SyncObserver>,
    ) -> Result<SyncSummary> {
        let app = self.app.clone();
        spawned(async move { app.sync_all(request, forward_to(observer)).await }).await
    }

    /// Syncs one source (`NotFound` for an unknown id, `Busy` when another sync runs).
    pub async fn sync_source(
        &self,
        source_id: String,
        request: SyncRequest,
        observer: Arc<dyn SyncObserver>,
    ) -> Result<SourceSyncResult> {
        let app = self.app.clone();
        spawned(async move {
            app.sync_source(&source_id, request, forward_to(observer))
                .await
        })
        .await
    }

    /// "Download & index this course's files" for a Canvas course (`Invalid` for folder
    /// courses). UIs must disclose first that downloading can count as viewing the files.
    pub async fn download_course_files(
        &self,
        course: String,
        observer: Arc<dyn SyncObserver>,
    ) -> Result<SourceSyncResult> {
        let app = self.app.clone();
        spawned(async move {
            app.download_course_files(&course, forward_to(observer))
                .await
        })
        .await
    }

    // ----- read views (hidden courses included/addressable) --------------------------------

    /// All courses, hidden ones included (check `course.hidden`).
    pub async fn list_courses(&self) -> Result<Vec<CourseSummary>> {
        let app = self.app.clone();
        blocking(move || app.list_courses()).await
    }

    /// `course` is an id or a code.
    pub async fn course_overview(&self, course: String) -> Result<CourseOverview> {
        let app = self.app.clone();
        blocking(move || app.course_overview(&course)).await
    }

    /// The materials of `week` (nil: the current week).
    pub async fn week_materials(&self, course: String, week: Option<u32>) -> Result<WeekMaterials> {
        let app = self.app.clone();
        blocking(move || app.week_materials(&course, week)).await
    }

    /// With a course: its events. Without: every non-hidden course's events plus events not
    /// linked to a course.
    pub async fn list_deadlines(
        &self,
        course: Option<String>,
        days_ahead: u32,
        days_back: u32,
    ) -> Result<Vec<Deadline>> {
        let app = self.app.clone();
        blocking(move || app.list_deadlines(course.as_deref(), days_ahead, days_back)).await
    }

    /// The student's own search over every non-hidden course.
    pub async fn search(
        &self,
        query: String,
        course: Option<String>,
        limit: u32,
    ) -> Result<Vec<SearchHit>> {
        let app = self.app.clone();
        blocking(move || app.search(&query, course.as_deref(), limit)).await
    }

    pub async fn latest_study_plan(&self) -> Result<Option<StoredStudyPlan>> {
        let app = self.app.clone();
        blocking(move || app.latest_study_plan()).await
    }

    // ----- course settings (course = id or code) --------------------------------------------

    pub async fn set_course_policy(
        &self,
        course: String,
        policy: AiPolicy,
        note: Option<String>,
    ) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_course_policy(&course, policy, note.as_deref())).await
    }

    /// Sets the student's term override; nil/nil falls back to the synced dates.
    pub async fn set_course_term(
        &self,
        course: String,
        start: Option<IsoDate>,
        end: Option<IsoDate>,
    ) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_course_term(&course, start, end)).await
    }

    /// "Let my AI app read this course's materials".
    pub async fn set_course_ai_access(&self, course: String, allowed: bool) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_course_ai_access(&course, allowed)).await
    }

    pub async fn set_course_hidden(&self, course: String, hidden: bool) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_course_hidden(&course, hidden)).await
    }

    // ----- course weeks, phases and the Past group --------------------------------------------

    /// Where one course is: week, phase, the dates used and not used, evidence. Hidden courses
    /// are addressable.
    pub async fn course_timeline(&self, course: String) -> Result<CourseTimeline> {
        let app = self.app.clone();
        blocking(move || app.course_timeline(&course)).await
    }

    /// Every course's lifecycle, the removal suggestions and whether the banner shows.
    pub async fn lifecycle_summary(&self) -> Result<LifecycleSummary> {
        let app = self.app.clone();
        blocking(move || app.lifecycle_summary()).await
    }

    /// "I'm still taking this" until `until` (nil: the end of the course's outer frame when
    /// that is still ahead, else today + `keep_current_days()`).
    pub async fn keep_course_current(
        &self,
        course: String,
        until: Option<IsoDate>,
    ) -> Result<Course> {
        let app = self.app.clone();
        blocking(move || app.keep_course_current(&course, until)).await
    }

    /// Undo "I'm still taking this".
    pub async fn clear_keep_course_current(&self, course: String) -> Result<Course> {
        let app = self.app.clone();
        blocking(move || app.clear_keep_course_current(&course)).await
    }

    /// "Not now" (`not_now_days()`) or "Keep" (never again) on the removal suggestion of
    /// `courses`.
    pub async fn snooze_removal_suggestions(
        &self,
        courses: Vec<String>,
        kind: SnoozeKind,
    ) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.snooze_removal_suggestions(courses, kind)).await
    }

    /// Undo "Not now" / "Keep": the courses may be suggested again.
    pub async fn clear_removal_snooze(&self, courses: Vec<String>) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.clear_removal_snooze(courses)).await
    }

    /// "Not now" on the banner: hidden for `not_now_days()`, until another course becomes a
    /// suggestion.
    pub async fn snooze_lifecycle_banner(&self) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.snooze_lifecycle_banner()).await
    }

    /// "These dates are right": the course's dates from PageLamp 0.1 stop being `legacy`.
    pub async fn confirm_course_dates(&self, course: String) -> Result<CourseTimeline> {
        let app = self.app.clone();
        blocking(move || app.confirm_course_dates(&course)).await
    }

    // ----- updates and launch -----------------------------------------------------------------

    /// The student's update settings (defaults when never set).
    pub async fn update_prefs(&self) -> Result<UpdatePrefs> {
        let app = self.app.clone();
        blocking(move || app.update_prefs()).await
    }

    pub async fn set_update_prefs(&self, prefs: UpdatePrefs) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_update_prefs(prefs)).await
    }

    /// The student's sync settings: how often PageLamp syncs by itself while it runs (twice a
    /// day unless chosen otherwise).
    pub async fn sync_prefs(&self) -> Result<SyncPrefs> {
        let app = self.app.clone();
        blocking(move || app.sync_prefs()).await
    }

    /// Change them (also while What's new is waiting: its row carries the control).
    pub async fn set_sync_prefs(&self, prefs: SyncPrefs) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_sync_prefs(prefs)).await
    }

    /// The student's channel, else Beta for a pre-release build and Stable otherwise.
    pub async fn effective_update_channel(&self) -> Result<UpdateChannel> {
        let app = self.app.clone();
        blocking(move || app.effective_update_channel()).await
    }

    /// What to do at launch and on the app's timer: What's new (upgraders), whether the
    /// automatic update check is due, the version this launch updated from. `now` is the
    /// caller's clock.
    pub async fn startup_tasks(&self, now: Timestamp) -> Result<StartupTasks> {
        let app = self.app.clone();
        blocking(move || app.startup_tasks(now)).await
    }

    /// The student closed What's new (its update-check topic counts as the disclosure).
    pub async fn acknowledge_whats_new(&self) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.acknowledge_whats_new()).await
    }

    /// The student saw what the update check sends (onboarding, fresh installs).
    pub async fn acknowledge_update_disclosure(&self) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.acknowledge_update_disclosure()).await
    }

    /// Remember how an update check ended (`startup_tasks` and diagnostic reports use it).
    pub async fn record_update_check(&self, record: UpdateCheckRecord) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.record_update_check(record)).await
    }

    pub async fn last_update_check(&self) -> Result<Option<UpdateCheckRecord>> {
        let app = self.app.clone();
        blocking(move || app.last_update_check()).await
    }

    /// What this app is doing right now (syncs, downloads), and whether another process is
    /// syncing. Ask it before installing an update.
    pub async fn activity(&self) -> Result<Activity> {
        let app = self.app.clone();
        blocking(move || Ok(app.activity())).await
    }

    // ----- diagnostics (this data dir) -------------------------------------------------------

    /// `<data_dir>/logs`, created if missing ("Open Logs Folder").
    pub async fn logs_dir(&self) -> Result<String> {
        let app = self.app.clone();
        blocking(move || app.logs_dir().map(|dir| path_string(&dir))).await
    }

    pub async fn last_crash(&self) -> Result<Option<CrashReport>> {
        let app = self.app.clone();
        blocking(move || app.last_crash()).await
    }

    pub async fn clear_last_crash(&self) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.clear_last_crash()).await
    }

    /// Probes the secret store (the keychain after `open`) and reads which AI apps have a
    /// PageLamp entry in their config (presence only).
    pub async fn doctor(&self) -> Result<DoctorReport> {
        let app = self.app.clone();
        blocking(move || app.doctor()).await
    }

    /// Markdown for an issue, redacted and pseudonymised; show it before it is shared.
    pub async fn diagnostic_report(&self) -> Result<String> {
        let app = self.app.clone();
        blocking(move || app.diagnostic_report()).await
    }

    // ----- "connect your AI app" -------------------------------------------------------------

    /// One config per AI app for launching `<pagelamp_binary> mcp` (the bundled CLI:
    /// `Bundle.main.url(forAuxiliaryExecutable: "pagelamp")`). Never writes any config.
    pub async fn mcp_client_configs(
        &self,
        pagelamp_binary: String,
    ) -> Result<Vec<McpClientConfig>> {
        let app = self.app.clone();
        blocking(move || Ok(app.mcp_client_configs(Path::new(&pagelamp_binary)))).await
    }

    /// How an AI app launches this data dir's MCP server.
    pub async fn mcp_launch(&self, pagelamp_binary: String) -> Result<McpLaunch> {
        let app = self.app.clone();
        blocking(move || Ok(app.mcp_launch(Path::new(&pagelamp_binary)))).await
    }
}

// ---------------------------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------------------------

/// Version of this PageLamp build (the facade's `AppStatus.version`).
#[uniffi::export]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// The facade's default sync options (same as Swift's `SyncRequest()`).
#[uniffi::export]
pub fn default_sync_request() -> SyncRequest {
    SyncRequest::default()
}

/// Days "Not now" lasts on a removal suggestion, the lifecycle banner or the syllabus offers.
#[uniffi::export]
pub fn not_now_days() -> i64 {
    pagelamp_app::NOT_NOW_DAYS
}

/// Days "I'm still taking this" lasts when the course has no outer frame ahead.
#[uniffi::export]
pub fn keep_current_days() -> i64 {
    pagelamp_app::KEEP_CURRENT_DAYS
}

/// The date "Keep" stores as `removal_snoozed_until` (9999-12-31): never suggested again.
#[uniffi::export]
pub fn keep_forever() -> IsoDate {
    pagelamp_app::keep_forever()
}

/// Sets up log files (`<data dir>/logs`), redacted stderr logging and the crash hook for this
/// process. Call once, first thing, before `PageLamp.open`; later calls do nothing. `data_dir`
/// nil = the default data dir (`PAGELAMP_HOME`, else the platform folder), as `open(nil)`.
#[uniffi::export(default(data_dir = None))]
pub fn init_diagnostics(verbose: bool, data_dir: Option<String>) -> Result<()> {
    match data_dir {
        Some(dir) => {
            pagelamp_core::diagnostics::init(Some(Path::new(&dir)), ProcessKind::App, verbose)
        }
        None => pagelamp_app::diagnostics::init(ProcessKind::App, verbose),
    }
    Ok(())
}

/// Logs one ERROR line from the UI (message + optional details, capped and redacted).
#[uniffi::export]
pub fn log_ui_error(message: String, stack: Option<String>) {
    pagelamp_app::diagnostics::log_ui_error(&message, stack.as_deref());
}

// Diagnostics for the DEFAULT data dir without an open `PageLamp`: they work when `open`
// failed (a locked, damaged or too new database), which is when a report is needed.

/// `<default data dir>/logs`, created if missing.
#[uniffi::export]
pub async fn diagnostics_logs_dir() -> Result<String> {
    blocking(|| pagelamp_app::diagnostics::logs_dir().map(|dir| path_string(&dir))).await
}

#[uniffi::export]
pub async fn diagnostics_last_crash() -> Result<Option<CrashReport>> {
    blocking(pagelamp_app::diagnostics::last_crash).await
}

#[uniffi::export]
pub async fn diagnostics_clear_last_crash() -> Result<()> {
    blocking(pagelamp_app::diagnostics::clear_last_crash).await
}

/// `doctor` for the default data dir (probes the OS keychain).
#[uniffi::export]
pub async fn diagnostics_doctor() -> Result<DoctorReport> {
    blocking(pagelamp_app::diagnostics::doctor).await
}

/// `diagnostic_report` for the default data dir (probes the OS keychain).
#[uniffi::export]
pub async fn diagnostics_report() -> Result<String> {
    blocking(pagelamp_app::diagnostics::diagnostic_report).await
}

#[cfg(test)]
mod tests;
