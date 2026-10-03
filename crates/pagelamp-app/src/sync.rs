//! Sync orchestration: run the right source crate for each source, record the outcome on the
//! source row, and stream progress as `SyncEvent`s.
//!
//! Rules (docs/ARCHITECTURE.md §2, §4):
//! - `<data_dir>/sync.lock` is held for the whole run → `Busy` when another process syncs.
//! - One failing source never stops the others; its failure is recorded with
//!   `Store::record_sync` (message + `SourceErrorKind`) and reported as `ok: false`.
//! - Every returned future is `Send` (Tauri spawns them): no `Store` is held across `.await`.
//!   Blocking work (folder walk + text extraction) runs in `spawn_blocking` with its own
//!   `Store`; its progress travels back over a channel because the caller's `on_event` is not
//!   `'static`.

use std::collections::BTreeSet;

use chrono::{NaiveDate, Utc};
use pagelamp_canvas::{CanvasConfig, SyncOptions};
use pagelamp_core::ingest::Extractor;
use pagelamp_core::model::{MaterialKind, SourceKind, SourceRecord};
use pagelamp_core::paths;
use pagelamp_core::source::{CancelFlag, CourseSyncSummary, SourceError, SyncProgress};
use pagelamp_core::store::Store;
use tokio::sync::mpsc;

use crate::activity::ActivityKind;
use crate::lock::SyncLock;
use crate::{
    App, AppError, AppErrorKind, Result, SourceSyncResult, SyncEvent, SyncRequest, SyncSummary,
};

/// What a source sync produced (the counters of `SourceSyncResult`).
#[derive(Default)]
struct Counts {
    courses: usize,
    modules: usize,
    materials: usize,
    files_downloaded: usize,
    files_indexed: usize,
    events: usize,
    warnings: Vec<String>,
    course_summaries: Vec<CourseSyncSummary>,
    requests: Option<u32>,
}

impl App {
    /// Sync every source: course-creating sources (folder, Canvas) first, then calendar
    /// feeds, so feed events can be matched to their courses in the same run; otherwise in
    /// `list_sources` order. Holds `sync.lock` for the whole run (`Busy` if taken).
    pub async fn sync_all(
        &self,
        req: SyncRequest,
        on_event: impl Fn(SyncEvent) + Send + Sync,
    ) -> Result<SyncSummary> {
        let _lock = self.acquire_sync_lock()?;
        // S8: due purges first (and files a failed Trash move left).
        self.purge_due_at_sync_start();
        let _activity = self.begin_activity(ActivityKind::Sync, None);
        let cancel = self.begin_cancellable();
        let started_at = Utc::now();
        let mut sources = self.read_store()?.list_sources()?;
        // Stable: keeps the label order within each group.
        sources.sort_by_key(|source| source.kind == SourceKind::Ical);
        let mut results = Vec::with_capacity(sources.len());
        let extractor = self.extractor(cancel.flag());
        for source in &sources {
            results.push(
                self.sync_one(source, &req, None, &extractor, &on_event)
                    .await,
            );
            if cancel.flag().is_cancelled() {
                return Err(AppError::cancelled());
            }
        }
        Ok(SyncSummary {
            started_at,
            finished_at: Utc::now(),
            ok: results.iter().all(|r| r.ok),
            results,
        })
    }

    /// Sync one source (`NotFound` for an unknown id, `Busy` when another sync runs). A
    /// failing source is `Ok` with `ok: false`.
    pub async fn sync_source(
        &self,
        source_id: &str,
        req: SyncRequest,
        on_event: impl Fn(SyncEvent) + Send + Sync,
    ) -> Result<SourceSyncResult> {
        let _lock = self.acquire_sync_lock()?;
        // S8: due purges first (and files a failed Trash move left).
        self.purge_due_at_sync_start();
        let _activity = self.begin_activity(ActivityKind::Sync, Some(source_id));
        let cancel = self.begin_cancellable();
        let source = self.source(source_id)?;
        let extractor = self.extractor(cancel.flag());
        let result = self
            .sync_one(&source, &req, None, &extractor, &on_event)
            .await;
        cancel.result(result)
    }

    /// Explicit "download & index this course's files" action for an LMS course: a sync of
    /// the course's source restricted to that course with `download_files = true`. UIs must
    /// disclose first: "Downloading files through Canvas can count as viewing them (e.g.
    /// module 'must view' requirements)." Folder courses are always indexed → `Invalid`.
    pub async fn download_course_files(
        &self,
        course: &str,
        on_event: impl Fn(SyncEvent) + Send + Sync,
    ) -> Result<SourceSyncResult> {
        let _lock = self.acquire_sync_lock()?;
        // S8: due purges first (and files a failed Trash move left).
        self.purge_due_at_sync_start();
        let course = self.read_store()?.resolve_course_with(course, true)?;
        let _activity = self.begin_activity(ActivityKind::Download, Some(&course.source_id));
        let source = self.source(&course.source_id)?;
        if source.kind != SourceKind::Canvas {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                "Only Canvas courses have files to download; folder courses are always indexed.",
            ));
        }
        let req = SyncRequest {
            download_files: true,
            only_courses: vec![course.id],
            ..SyncRequest::default()
        };
        let cancel = self.begin_cancellable();
        let extractor = self.extractor(cancel.flag());
        let result = self
            .sync_one(&source, &req, None, &extractor, &on_event)
            .await;
        cancel.result(result)
    }

    /// Download chosen files of one Canvas course only (the syllabus candidates the student
    /// picked, calendar design D46): a sync of that course that downloads just these material
    /// ids. The same disclosure as `download_course_files` applies. `Invalid` for a folder
    /// course, an empty list, or an id that isn't one of the course's files.
    pub(crate) async fn download_chosen_files(
        &self,
        course: &str,
        material_ids: Vec<String>,
        on_event: impl Fn(SyncEvent) + Send + Sync,
    ) -> Result<SourceSyncResult> {
        let _lock = self.acquire_sync_lock()?;
        let (course, files) = {
            let store = self.read_store()?;
            let course = store.resolve_course_with(course, true)?;
            let files: BTreeSet<String> = store
                .list_materials(&course.id)?
                .into_iter()
                .filter(|m| m.kind == MaterialKind::File)
                .map(|m| m.id)
                .collect();
            (course, files)
        };
        if material_ids.is_empty() {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                "Choose a file to download.",
            ));
        }
        if let Some(stray) = material_ids.iter().find(|id| !files.contains(*id)) {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                format!("{stray} is not a file of {}.", course.display_name()),
            ));
        }
        let _activity = self.begin_activity(ActivityKind::Download, Some(&course.source_id));
        let source = self.source(&course.source_id)?;
        if source.kind != SourceKind::Canvas {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                "Only Canvas courses have files to download; folder courses are always indexed.",
            ));
        }
        let req = SyncRequest {
            download_files: true,
            only_courses: vec![course.id],
            ..SyncRequest::default()
        };
        let only: BTreeSet<String> = material_ids.into_iter().collect();
        let cancel = self.begin_cancellable();
        let extractor = self.extractor(cancel.flag());
        let result = self
            .sync_one(&source, &req, Some(&only), &extractor, &on_event)
            .await;
        cancel.result(result)
    }

    /// Stop the sync this app is running (`sync_all`, `sync_source`, `download_course_files`;
    /// mac request F4). It stops at the next file, course or download: a file being read in the
    /// extraction worker is abandoned at once. Courses finished so far stay synced; the source
    /// isn't marked failed; the call returns `AppErrorKind::Cancelled`. Does nothing when no
    /// sync runs in this app (a sync in another process, e.g. the CLI, can't be stopped here).
    pub fn cancel_sync(&self) {
        if let Some(flag) = self
            .state
            .sync_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            tracing::info!(target: "pagelamp::sync", "sync stop requested");
            flag.cancel();
        }
    }

    /// Register this sync's stop request until the returned guard is dropped.
    pub(crate) fn begin_cancellable(&self) -> CancelGuard<'_> {
        let flag = CancelFlag::new();
        *self
            .state
            .sync_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(flag.clone());
        CancelGuard { app: self, flag }
    }

    pub(crate) fn acquire_sync_lock(&self) -> Result<SyncLock> {
        SyncLock::acquire(&paths::sync_lock_path_in(self.data_dir()))
    }

    pub(crate) fn source(&self, source_id: &str) -> Result<SourceRecord> {
        self.read_store()?
            .get_source(source_id)?
            .ok_or_else(|| crate::unknown_source(source_id))
    }

    /// Run one source and record the outcome. Never fails: problems become `ok: false`.
    /// `extractor` reads the files of this sync (one per sync, see `ingest::Extractor`).
    pub(crate) async fn sync_one(
        &self,
        source: &SourceRecord,
        req: &SyncRequest,
        only_files: Option<&BTreeSet<String>>,
        extractor: &Extractor,
        on_event: &(dyn Fn(SyncEvent) + Send + Sync),
    ) -> SourceSyncResult {
        let started_at = Utc::now();
        // Before too: a course this sync removes or renames may already be in the logs.
        self.remember_course_names();
        tracing::info!(target: "pagelamp::sync", "{} sync started", source.kind.as_str());
        on_event(SyncEvent::SourceStarted {
            source_id: source.id.clone(),
            label: source.label.clone(),
        });
        let progress = |p: SyncProgress| on_event(progress_event(&source.id, p));
        let outcome = match source.kind {
            SourceKind::Folder => self.run_folder(source, extractor, &progress).await,
            SourceKind::Ical => self.run_ical(source, &progress).await,
            SourceKind::Canvas => {
                self.run_canvas(source, req, only_files, extractor, &progress)
                    .await
            }
        };
        if matches!(source.kind, SourceKind::Folder | SourceKind::Canvas) {
            self.relink_events();
            if outcome.is_ok() {
                self.scan_after_sync(&source.id);
            }
        }
        self.remember_course_names();
        let finished_at = Utc::now();
        if outcome.as_ref().is_err_and(|error| error.cancelled) {
            // Stopped by the student: not a failure, so nothing is recorded for the source.
            tracing::info!(target: "pagelamp::sync", "{} sync stopped", source.kind.as_str());
            on_event(SyncEvent::SourceFinished {
                source_id: source.id.clone(),
                ok: false,
                error: Some("The sync was stopped.".to_string()),
                error_kind: None,
            });
            return SourceSyncResult {
                ok: false,
                error: Some("The sync was stopped.".to_string()),
                ..self.empty_result(source, started_at)
            };
        }
        let error = outcome.as_ref().err().map(|e| (e.kind, e.message.clone()));
        // Log file: kind and counts at info; course/file names only at debug.
        match (&outcome, &error) {
            (Ok(counts), _) => {
                tracing::info!(
                    target: "pagelamp::sync",
                    "{} sync ok: {} courses, {} materials, {} events, {} warnings{} in {} ms",
                    source.kind.as_str(),
                    counts.courses,
                    counts.materials,
                    counts.events,
                    counts.warnings.len(),
                    counts.requests.map(|r| format!(", {r} requests")).unwrap_or_default(),
                    (finished_at - started_at).num_milliseconds()
                );
                for warning in &counts.warnings {
                    tracing::debug!(target: "pagelamp::sync", "warning: {warning}");
                }
            }
            (Err(_), Some((kind, message))) => tracing::warn!(
                target: "pagelamp::sync",
                "{} sync failed ({}): {message}",
                source.kind.as_str(),
                kind.as_str()
            ),
            (Err(_), None) => {}
        }
        let recorded = self.write_store().and_then(|store| {
            let error = error.as_ref().map(|(kind, msg)| (*kind, msg.as_str()));
            Ok(store.record_sync(&source.id, finished_at, error)?)
        });
        if let Err(err) = recorded {
            tracing::warn!(source = %source.id, "could not record sync outcome: {err}");
        }
        on_event(SyncEvent::SourceFinished {
            source_id: source.id.clone(),
            ok: error.is_none(),
            error: error.as_ref().map(|(_, msg)| msg.clone()),
            error_kind: error.as_ref().map(|(kind, _)| *kind),
        });
        let counts = outcome.unwrap_or_default();
        SourceSyncResult {
            source_id: source.id.clone(),
            label: source.label.clone(),
            kind: source.kind,
            ok: error.is_none(),
            error_kind: error.as_ref().map(|(kind, _)| *kind),
            error: error.map(|(_, msg)| msg),
            started_at,
            finished_at,
            courses: to_u32(counts.courses),
            modules: to_u32(counts.modules),
            materials: to_u32(counts.materials),
            files_downloaded: to_u32(counts.files_downloaded),
            files_indexed: to_u32(counts.files_indexed),
            events: to_u32(counts.events),
            warnings: counts.warnings,
            course_summaries: counts.course_summaries,
            requests: counts.requests,
        }
    }

    /// A result with no counts (a stopped sync).
    fn empty_result(
        &self,
        source: &SourceRecord,
        started_at: chrono::DateTime<Utc>,
    ) -> SourceSyncResult {
        SourceSyncResult {
            source_id: source.id.clone(),
            label: source.label.clone(),
            kind: source.kind,
            ok: false,
            error_kind: None,
            error: None,
            started_at,
            finished_at: Utc::now(),
            courses: 0,
            modules: 0,
            materials: 0,
            files_downloaded: 0,
            files_indexed: 0,
            events: 0,
            warnings: Vec::new(),
            course_summaries: Vec::new(),
            requests: None,
        }
    }

    /// Link calendar events synced before their course existed (local, no network). Runs
    /// after every folder/Canvas sync, even a failed one: it may still have added courses.
    fn relink_events(&self) {
        match self
            .write_store()
            .and_then(|store| Ok(store.relink_events()?))
        {
            Ok(0) => {}
            Ok(linked) => tracing::info!(
                target: "pagelamp::sync",
                "linked {linked} calendar events to their courses"
            ),
            Err(err) => tracing::warn!(
                target: "pagelamp::sync",
                "could not link calendar events to courses: {err}"
            ),
        }
    }

    async fn run_folder(
        &self,
        source: &SourceRecord,
        extractor: &Extractor,
        progress: &(dyn Fn(SyncProgress) + Send + Sync),
    ) -> std::result::Result<Counts, SourceError> {
        let root: std::path::PathBuf = source
            .config
            .get("path")
            .and_then(|p| p.as_str())
            .ok_or_else(|| SourceError::other("this folder source has no path configured"))?
            .into();
        let term_start = config_date(source, "term_start");
        let (db_path, source_id) = (self.db_path(), source.id.clone());
        let (tx, mut rx) = mpsc::unbounded_channel();
        let extractor = extractor.clone();
        let task = tokio::task::spawn_blocking(move || {
            let store = Store::open(&db_path)?;
            pagelamp_local::sync_folder(&store, &source_id, &root, term_start, &extractor, &|p| {
                let _ = tx.send(p);
            })
        });
        // Ends when the task finishes and drops the sender.
        while let Some(p) = rx.recv().await {
            progress(p);
        }
        let report = task
            .await
            .map_err(|err| SourceError::other(format!("folder sync crashed: {err}")))??;
        Ok(Counts {
            courses: report.courses,
            modules: report.modules,
            materials: report.materials,
            files_indexed: report.files_indexed,
            warnings: report.warnings,
            ..Counts::default()
        })
    }

    async fn run_ical(
        &self,
        source: &SourceRecord,
        progress: &(dyn Fn(SyncProgress) + Send + Sync),
    ) -> std::result::Result<Counts, SourceError> {
        let feed_url = self.stored_secret(source, "calendar feed URL")?;
        let report =
            pagelamp_local::sync_ical(&self.db_path(), &source.id, &feed_url, progress).await?;
        Ok(Counts {
            events: report.events,
            warnings: report.warnings,
            ..Counts::default()
        })
    }

    async fn run_canvas(
        &self,
        source: &SourceRecord,
        req: &SyncRequest,
        only_files: Option<&BTreeSet<String>>,
        extractor: &Extractor,
        progress: &(dyn Fn(SyncProgress) + Send + Sync),
    ) -> std::result::Result<Counts, SourceError> {
        let base_url = crate::canvas_base_url(source).map_err(|e| SourceError::other(e.message))?;
        let config = CanvasConfig {
            base_url,
            token: self.stored_secret(source, "Canvas access token")?,
        };
        let options = SyncOptions {
            download_files: req.download_files,
            max_file_bytes: u64::from(req.max_file_mb) * 1024 * 1024,
            files_dir: paths::files_dir_in(self.data_dir()),
            only_courses: req.only_courses.clone(),
            only_files: only_files.cloned(),
            extractor: extractor.clone(),
        };
        let report = pagelamp_canvas::sync(&self.db_path(), &config, &options, progress).await?;
        Ok(Counts {
            courses: report.courses,
            modules: report.modules,
            materials: report.materials,
            files_downloaded: report.files_downloaded,
            files_indexed: report.files_indexed,
            events: report.events,
            warnings: report.warnings,
            course_summaries: report.course_summaries,
            requests: Some(u32::try_from(report.requests).unwrap_or(u32::MAX)),
        })
    }

    /// The source's secret, or an auth error telling the student to enter it again.
    fn stored_secret(
        &self,
        source: &SourceRecord,
        what: &str,
    ) -> std::result::Result<String, SourceError> {
        match self.secrets.get(&source.id) {
            Ok(Some(secret)) => Ok(secret),
            Ok(None) => Err(SourceError::auth(format!(
                "No {what} is stored for '{}' — enter it again.",
                source.label
            ))),
            Err(err) => Err(SourceError::other(err.to_string())),
        }
    }
}

/// This sync's stop request, registered in the app until dropped.
pub(crate) struct CancelGuard<'a> {
    app: &'a App,
    flag: CancelFlag,
}

impl CancelGuard<'_> {
    pub(crate) fn flag(&self) -> CancelFlag {
        self.flag.clone()
    }

    /// `result`, or `Cancelled` if the sync was stopped.
    pub(crate) fn result(&self, result: SourceSyncResult) -> Result<SourceSyncResult> {
        if self.flag.is_cancelled() {
            Err(AppError::cancelled())
        } else {
            Ok(result)
        }
    }
}

impl Drop for CancelGuard<'_> {
    fn drop(&mut self) {
        let mut current = self
            .app
            .state
            .sync_cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *current = None;
    }
}

fn progress_event(source_id: &str, progress: SyncProgress) -> SyncEvent {
    match progress {
        SyncProgress::Step {
            message,
            current,
            total,
            stage,
            course,
        } => SyncEvent::Progress {
            source_id: source_id.to_string(),
            message,
            current,
            total,
            stage,
            course,
        },
        SyncProgress::Warning(message) => SyncEvent::Warning {
            source_id: source_id.to_string(),
            message,
        },
    }
}

/// A `YYYY-MM-DD` value from the source config, if present and valid.
fn config_date(source: &SourceRecord, key: &str) -> Option<NaiveDate> {
    let text = source.config.get(key)?.as_str()?;
    NaiveDate::parse_from_str(text, "%Y-%m-%d").ok()
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}
