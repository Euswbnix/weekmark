//! Removing finished courses in two stages (docs/design/v0.3-course-calendar.md §8.3–§8.7).
//!
//! - `removal_preview`: what the dialog shows per course, and the pre-update backup;
//! - `remove_courses` (stage 1, under `sync.lock`): hidden at once, a `pending` tombstone,
//!   purged after 7 days or at once with `purge_now`;
//! - `purge_removed_courses` (stage 2, under `sync.lock`; due purges also run at each sync's
//!   start): the database first (marked `files_pending` in the same transaction), then the
//!   course's downloaded Canvas files to the Trash (never files in the student's own folders;
//!   a failed move or a quit is retried, never deleted permanently unless the student asks),
//!   and the pre-update backup if the student chose so;
//! - `removed_courses`, `restore_course` (undo while pending; after a purge, the course is
//!   synced back and gets its settings back; a restore a quit interrupted is settled at the
//!   next sync or launch), `forget_removed_course`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use pagelamp_core::model::{
    CourseLifecycle, LifecycleState, MaterialKind, MigrationBackupOutcome, SourceErrorKind,
    SourceKind, Timestamp,
};
use pagelamp_core::paths;
use pagelamp_core::removal::Tombstone;
pub use pagelamp_core::removal::{RemovalReason, TombstoneState};
use pagelamp_core::store::Store;
use pagelamp_core::views::{self, AsOf};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::activity::ActivityKind;
use crate::trash::FileTrash;
use crate::{App, AppError, AppErrorKind, Result, SyncRequest};

/// What a later sync can't bring back once a course is purged (§8.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LostAfterPurge {
    /// Announcements older than the sync window (120 days).
    OldAnnouncements,
    /// Files the course may lock after it ends.
    LockedFiles,
    /// The LMS restricts the course by date: it can't be synced again at all.
    WholeCourse,
    /// Downloading the files again counts as viewing them in the LMS.
    RedownloadCountsAsViewing,
}

/// The pre-update backup (`pagelamp.db.v<N>.bak`) still holds the course's text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupInfo {
    pub age_days: u32,
    /// "Also delete the pre-update backup" is ticked by default when it is 14+ days old.
    pub delete_by_default: bool,
    /// `backup_old` (safe to delete) or `backup_recent` (the way back from a bad update).
    pub reason_code: String,
}

/// One course in the removal dialog.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemovalPreviewItem {
    pub course_id: String,
    pub code: Option<String>,
    pub name: String,
    pub source_kind: SourceKind,
    pub lifecycle: CourseLifecycle,
    pub materials: u32,
    pub downloaded_files: u32,
    pub downloaded_bytes: u64,
    pub deadlines: u32,
    pub generated_items: u32,
    /// The student changed the course's settings (AI policy, access, dates…).
    pub custom_settings: bool,
    /// A folder course: its files are never touched.
    pub own_folder_untouched: bool,
    /// The LMS restricts the course by date, so it can't be synced again.
    pub cannot_sync_again: bool,
    pub lost_after_purge: Vec<LostAfterPurge>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemovalPreview {
    pub items: Vec<RemovalPreviewItem>,
    pub backup: Option<BackupInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemoveOptions {
    /// None: from each course's lifecycle (Ended → ended, Inactive → inactive, else other).
    pub reason: Option<RemovalReason>,
    pub keep_downloaded_files: bool,
    /// Delete at once instead of in 7 days (no undo).
    pub purge_now: bool,
    /// Delete the pre-update backup with the purge (at once with `purge_now`); an undo keeps it.
    pub delete_pre_update_backup: bool,
}

/// A course in "Removed courses".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemovedCourse {
    /// What `restore_course` / `forget_removed_course` take.
    pub removed_id: String,
    pub source_id: String,
    pub source_kind: SourceKind,
    pub external_id: String,
    pub course_id: String,
    pub code: Option<String>,
    pub name: String,
    pub reason: RemovalReason,
    pub state: TombstoneState,
    pub removed_at: Timestamp,
    /// When the local data will be deleted (pending only).
    pub purge_after: Option<Timestamp>,
    pub purged_at: Option<Timestamp>,
    /// Whole days until the purge ("deleted in 3 days"); None once purged.
    pub purge_in_days: Option<u32>,
    pub keep_files: bool,
    /// Moving the downloaded files to the Trash failed; retried later.
    pub files_pending: bool,
}

/// What a purge would do (`purge_targets`), for a shell to show before it asks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgeTargets {
    /// Removals whose local data goes now, and purged courses whose downloaded files go to the
    /// Trash.
    pub courses: Vec<RemovedCourse>,
    /// One of the removals asked for the pre-update backup to go with it (`--delete-backup`).
    pub deletes_backup: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RemovalReport {
    pub removed: Vec<RemovedCourse>,
    pub purged_now: bool,
    /// The pre-update backup was deleted now (`purge_now`; otherwise it goes with the purge).
    pub backup_deleted: bool,
    /// Deleting the pre-update backup failed (the courses are removed all the same).
    pub backup_failed: bool,
}

/// Why a purged course didn't come back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestoreFailure {
    /// The LMS no longer lists the course.
    NotListed,
    /// The LMS restricts the course by date.
    AccessRestricted,
    Offline,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RestoreOutcome {
    pub restored: bool,
    pub course_id: Option<String>,
    pub failure: Option<RestoreFailure>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PurgeReport {
    /// `removed_id`s purged now.
    pub purged: Vec<String>,
    /// `removed_id`s whose files couldn't be moved to the Trash (kept; retried later).
    pub files_pending: Vec<String>,
    /// A purged course asked for the pre-update backup to go, and it did.
    pub backup_deleted: bool,
    /// Deleting the pre-update backup failed (the purge went ahead).
    pub backup_failed: bool,
}

/// A pre-update backup this old or older is deleted by default with a removal.
pub const BACKUP_DELETE_BY_DEFAULT_DAYS: i64 = 14;

impl App {
    /// What removing `courses` would take and keep (the removal dialog), plus the pre-update
    /// backup that still holds their text.
    pub fn removal_preview(&self, courses: Vec<String>) -> Result<RemovalPreview> {
        let store = self.read_store()?;
        let at = AsOf::now_local();
        let summaries = views::list_courses(&store, true, at)?;
        let files_dir = paths::files_dir_in(self.data_dir());
        let mut items = Vec::new();
        for reference in &courses {
            let course = store.resolve_course_with(reference, true)?;
            let summary = summaries
                .iter()
                .find(|s| s.course.id == course.id)
                .ok_or_else(|| AppError::new(AppErrorKind::NotFound, "course not found"))?;
            let source = self.source(&course.source_id)?;
            let materials = store.list_materials(&course.id)?;
            let (mut downloaded_files, mut downloaded_bytes) = (0u32, 0u64);
            if source.kind == SourceKind::Canvas {
                for material in materials.iter().filter(|m| m.kind == MaterialKind::File) {
                    let Some(path) = material.local_path.as_deref().map(Path::new) else {
                        continue;
                    };
                    if let Ok(meta) = std::fs::metadata(path)
                        && meta.is_file()
                        && path.starts_with(&files_dir)
                    {
                        downloaded_files += 1;
                        downloaded_bytes += meta.len();
                    }
                }
            }
            let deadlines = store
                .list_events(
                    DateTime::<Utc>::MIN_UTC,
                    DateTime::<Utc>::MAX_UTC,
                    Some(&course.id),
                )?
                .len();
            let restricted = store
                .course_term_data(&course.id)?
                .and_then(|data| data.lms.access_restricted)
                == Some(true);
            let lost_after_purge = match source.kind {
                SourceKind::Canvas => {
                    let mut lost = vec![
                        LostAfterPurge::OldAnnouncements,
                        LostAfterPurge::LockedFiles,
                    ];
                    if restricted {
                        lost.push(LostAfterPurge::WholeCourse);
                    }
                    if downloaded_files > 0 {
                        lost.push(LostAfterPurge::RedownloadCountsAsViewing);
                    }
                    lost
                }
                // Read again from the student's own files.
                SourceKind::Folder | SourceKind::Ical => Vec::new(),
            };
            items.push(RemovalPreviewItem {
                course_id: course.id.clone(),
                code: course.code.clone(),
                name: course.name.clone(),
                source_kind: source.kind,
                lifecycle: summary.lifecycle.clone(),
                materials: u32::try_from(materials.len()).unwrap_or(u32::MAX),
                downloaded_files,
                downloaded_bytes,
                deadlines: u32::try_from(deadlines).unwrap_or(u32::MAX),
                generated_items: store.course_generation_count(&course.id)?,
                custom_settings: store.course_settings(&course.id)?.customised(),
                own_folder_untouched: source.kind == SourceKind::Folder,
                cannot_sync_again: restricted,
                lost_after_purge,
            });
        }
        Ok(RemovalPreview {
            items,
            backup: backup_info(&store)?,
        })
    }

    /// Stage 1: remove `courses` (hidden at once; their data is deleted after 7 days, or now
    /// with `purge_now`). `Busy` while a sync runs; `NotFound` for an unknown or already
    /// removed course (nothing is removed then).
    pub async fn remove_courses(
        &self,
        courses: Vec<String>,
        options: RemoveOptions,
    ) -> Result<RemovalReport> {
        let _lock = self.acquire_sync_lock()?;
        // Their names may be in the logs: keep them for the report's pseudonymisation.
        self.remember_course_names();
        let store = self.write_store()?;
        let at = AsOf::now_local();
        let summaries = views::list_courses(&store, true, at)?;
        let resolved = courses
            .iter()
            .map(|c| store.resolve_course_with(c, true))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let now = Utc::now();
        let tombstones = store.in_transaction(|store| {
            resolved
                .iter()
                .map(|course| {
                    let reason = options.reason.unwrap_or_else(|| {
                        let state = summaries
                            .iter()
                            .find(|s| s.course.id == course.id)
                            .map(|s| s.lifecycle.state);
                        match state {
                            Some(LifecycleState::Ended) => RemovalReason::Ended,
                            Some(LifecycleState::Inactive) => RemovalReason::Inactive,
                            _ => RemovalReason::Other,
                        }
                    });
                    store.remove_course(
                        &course.id,
                        reason,
                        options.keep_downloaded_files,
                        options.delete_pre_update_backup,
                        now,
                    )
                })
                .collect::<pagelamp_core::Result<Vec<_>>>()
        })?;
        // The backup is deleted with the purge, never in this undoable stage.
        let purge = if options.purge_now {
            Some(self.purge_locked(&store, tombstones.clone(), false)?)
        } else {
            None
        };
        let removed = tombstones
            .iter()
            .filter_map(|t| store.tombstone(&t.course_id).ok().flatten())
            .map(|t| self.removed_course(&store, t))
            .collect::<Result<Vec<_>>>()?;
        Ok(RemovalReport {
            removed,
            purged_now: options.purge_now,
            backup_deleted: purge.as_ref().is_some_and(|p| p.backup_deleted),
            backup_failed: purge.as_ref().is_some_and(|p| p.backup_failed),
        })
    }

    /// "Removed courses": newest removal first.
    pub fn removed_courses(&self) -> Result<Vec<RemovedCourse>> {
        let store = self.read_store()?;
        store
            .tombstones()?
            .into_iter()
            .map(|t| self.removed_course(&store, t))
            .collect()
    }

    /// Undo a removal that isn't purged yet (everything comes back at once), or bring a purged
    /// course back by syncing its source: its settings and calendar come back, its Canvas
    /// files as not downloaded. A `restoring` course (a restore a quit interrupted) is tried
    /// again. `Busy` while a sync runs.
    pub async fn restore_course(&self, removed_id: &str) -> Result<RestoreOutcome> {
        let _lock = self.acquire_sync_lock()?;
        let tombstone = self
            .read_store()?
            .tombstone(removed_id)?
            .ok_or_else(|| no_removed_course(removed_id))?;
        if tombstone.state == TombstoneState::Pending {
            self.write_store()?.undo_removal(removed_id)?;
            return Ok(RestoreOutcome {
                restored: true,
                course_id: Some(tombstone.course_id),
                failure: None,
            });
        }
        // What can fail comes before the `restoring` mark; a mark a quit leaves behind is
        // settled at the next sync or launch (`settle_restores`).
        let source = self.source(&tombstone.source_id)?;
        let req = SyncRequest {
            only_courses: match source.kind {
                SourceKind::Canvas => vec![tombstone.course_id.clone()],
                SourceKind::Folder | SourceKind::Ical => Vec::new(),
            },
            ..SyncRequest::default()
        };
        // A sync of its source, listed like `sync_source` (an update waits for it).
        let _activity = self.begin_activity(ActivityKind::Sync, Some(&tombstone.source_id));
        self.write_store()?
            .set_tombstone_state(removed_id, TombstoneState::Restoring)?;
        let cancel = self.begin_cancellable();
        let extractor = self.extractor(cancel.flag());
        let result = self
            .sync_one(&source, &req, None, &extractor, &|_| {})
            .await;
        let store = self.write_store()?;
        if settle_restore(&store, removed_id)? {
            return Ok(RestoreOutcome {
                restored: true,
                course_id: Some(tombstone.course_id),
                failure: None,
            });
        }
        let failure = if result.error_kind == Some(SourceErrorKind::Network) {
            RestoreFailure::Offline
        } else if tombstone.settings.access_restricted {
            RestoreFailure::AccessRestricted
        } else if !result.ok {
            RestoreFailure::Other
        } else {
            RestoreFailure::NotListed
        };
        Ok(RestoreOutcome {
            restored: false,
            course_id: None,
            failure: Some(failure),
        })
    }

    /// Stage 2 now: the removals in `removed_ids` ("Delete now", or trying a failed Trash move
    /// again), or every due purge (`None`). `permanent_if_no_trash`: the student chose "Delete
    /// permanently" after the Trash failed. It is never true by default anywhere: the CLI sets
    /// it only with `--permanent`, a shell only from the student's explicit choice in the
    /// removal dialog, worded as something that can't be undone. `Busy` while a sync runs.
    pub async fn purge_removed_courses(
        &self,
        removed_ids: Option<Vec<String>>,
        permanent_if_no_trash: bool,
    ) -> Result<PurgeReport> {
        let _lock = self.acquire_sync_lock()?;
        let store = self.write_store()?;
        let targets = purge_targets_in(&store, removed_ids.as_deref())?;
        self.purge_locked(&store, targets, permanent_if_no_trash)
    }

    /// What `purge_removed_courses` with the same `removed_ids` would do, read-only, so a shell
    /// can show it and ask first (the CLI's `course purge`): the removals whose data it deletes,
    /// and the purged courses whose downloaded files it moves to the Trash. As in the purge, a
    /// `restoring` course is left alone, and so is a purged one with no files left to move.
    /// `NotFound` for an unknown id, as the purge.
    pub fn purge_targets(&self, removed_ids: Option<&[String]>) -> Result<PurgeTargets> {
        let store = self.read_store()?;
        let files_dir = paths::files_dir_in(self.data_dir());
        let mut targets = PurgeTargets {
            courses: Vec::new(),
            deletes_backup: false,
        };
        for tombstone in purge_targets_in(&store, removed_ids)? {
            let acts = match tombstone.state {
                TombstoneState::Pending => {
                    targets.deletes_backup |= tombstone.delete_backup;
                    true
                }
                TombstoneState::Purged => {
                    tombstone.files_pending
                        || (!tombstone.keep_files
                            && self.source(&tombstone.source_id)?.kind == SourceKind::Canvas
                            && !download_dirs(&store, &files_dir, &tombstone)?.is_empty())
                }
                TombstoneState::Restoring => false,
            };
            if acts {
                targets
                    .courses
                    .push(self.removed_course(&store, tombstone)?);
            }
        }
        Ok(targets)
    }

    /// Forget a purged course: the next sync brings it back. A removal that isn't purged yet
    /// is undone with `restore_course` instead, and files still waiting for the Trash are
    /// moved (or deleted) first (`Invalid`). `Busy` while a sync runs.
    pub fn forget_removed_course(&self, removed_id: &str) -> Result<()> {
        let _lock = self.acquire_sync_lock()?;
        let store = self.write_store()?;
        let tombstone = store
            .tombstone(removed_id)?
            .ok_or_else(|| no_removed_course(removed_id))?;
        if tombstone.state != TombstoneState::Purged {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                "Only a course whose data is already deleted can be forgotten; undo this removal instead.",
            ));
        }
        if tombstone.files_pending {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                "This course's downloaded files are still waiting for the Trash: try again, or \
                 delete them permanently, before forgetting it.",
            ));
        }
        Ok(store.delete_tombstone(removed_id)?)
    }

    /// Put in another Trash (tests; an embedder with its own).
    #[doc(hidden)]
    pub fn set_trash(&self, trash: Arc<dyn FileTrash>) {
        self.state.trash.set(trash);
    }

    /// S8, at a sync's start (the sync holds `sync.lock`): restores a quit interrupted are
    /// settled (before the sync reads the tombstones), then due purges run, and files a failed
    /// Trash move left. Problems are logged; they never stop the sync.
    pub(crate) fn purge_due_at_sync_start(&self) {
        let result = self
            .write_store()
            .and_then(|store| {
                settle_restores(&store)?;
                Ok((due_or_waiting(&store)?, store))
            })
            .and_then(|(targets, store)| self.purge_locked(&store, targets, false));
        match result {
            Ok(report) if !report.purged.is_empty() || !report.files_pending.is_empty() => {
                tracing::info!(
                    target: "pagelamp::removal",
                    "purged {} course(s); {} with files still waiting",
                    report.purged.len(),
                    report.files_pending.len()
                )
            }
            Ok(_) => {}
            Err(err) => tracing::warn!(target: "pagelamp::removal", "purge failed: {:?}", err.kind),
        }
    }

    /// At launch: settle a restore a quit interrupted (`settle_restores`), when there is one
    /// and `sync.lock` is free (a running restore holds it). Only a read when there is none;
    /// problems are logged.
    pub(crate) fn settle_restores_at_open(&self) {
        let interrupted = self.read_store().and_then(|store| {
            Ok(store
                .tombstones()?
                .iter()
                .any(|t| t.state == TombstoneState::Restoring))
        });
        if !matches!(interrupted, Ok(true)) {
            return;
        }
        let Ok(_lock) = self.acquire_sync_lock() else {
            return;
        };
        if let Err(err) = self.write_store().and_then(|store| settle_restores(&store)) {
            tracing::warn!(target: "pagelamp::removal", "settling a restore failed: {:?}", err.kind);
        }
    }

    /// Stage 2 for `targets` (`sync.lock` held): the database first, marked `files_pending`
    /// in the same transaction when downloaded Canvas files are left; then the files to the
    /// Trash (a failure, or a quit before the end, is retried; deleted permanently only with
    /// `permanent_if_no_trash`, and a failed delete is kept as pending too); `files_pending`
    /// is cleared only once every folder is handled. Then the pre-update backup, if a purged
    /// course asked for it: a failure is reported, never an error. A `restoring` course is
    /// left alone (its restore runs, or is settled at the next sync).
    fn purge_locked(
        &self,
        store: &Store,
        targets: Vec<Tombstone>,
        permanent_if_no_trash: bool,
    ) -> Result<PurgeReport> {
        let files_dir = paths::files_dir_in(self.data_dir());
        let trash = self.state.trash.get();
        let mut report = PurgeReport {
            purged: Vec::new(),
            files_pending: Vec::new(),
            backup_deleted: false,
            backup_failed: false,
        };
        let mut delete_backup = false;
        for tombstone in targets {
            if tombstone.state == TombstoneState::Restoring {
                continue;
            }
            let canvas = self.source(&tombstone.source_id)?.kind == SourceKind::Canvas;
            // Before the rows go: their local paths say which folders are the course's.
            let dirs = if canvas && !tombstone.keep_files {
                download_dirs(store, &files_dir, &tombstone)?
            } else {
                Vec::new()
            };
            if tombstone.state == TombstoneState::Pending {
                store.purge_course(&tombstone.course_id, Utc::now(), !dirs.is_empty())?;
                report.purged.push(tombstone.course_id.clone());
                delete_backup |= tombstone.delete_backup;
            } else if !dirs.is_empty() && !tombstone.files_pending {
                store.set_tombstone_files_pending(&tombstone.course_id, true)?;
            }
            let mut waiting = false;
            for dir in &dirs {
                let Err(reason) = trash.trash(dir) else {
                    continue;
                };
                if permanent_if_no_trash {
                    if let Err(err) = crate::remove_download_dir(dir) {
                        tracing::warn!(target: "pagelamp::removal", "deleting the downloaded files failed: {err}");
                        waiting = true;
                    }
                } else {
                    tracing::warn!(target: "pagelamp::removal", "moving to the Trash failed: {reason}");
                    waiting = true;
                }
            }
            // Also clears a mark whose folders are gone meanwhile (e.g. deleted by hand).
            store.set_tombstone_files_pending(&tombstone.course_id, waiting)?;
            if waiting {
                report.files_pending.push(tombstone.course_id);
            }
        }
        store.checkpoint_after_purge();
        if delete_backup {
            match pagelamp_core::store::delete_database_backups(&self.db_path()) {
                Ok(deleted) => report.backup_deleted = deleted > 0,
                Err(err) => {
                    tracing::warn!(target: "pagelamp::removal", "deleting the pre-update backup failed: {err}");
                    report.backup_failed = true;
                }
            }
        }
        Ok(report)
    }

    fn removed_course(&self, store: &Store, tombstone: Tombstone) -> Result<RemovedCourse> {
        let source_kind = store
            .get_source(&tombstone.source_id)?
            .map_or(SourceKind::Canvas, |s| s.kind);
        let purge_in_days = tombstone.purge_after.map(|after| {
            let seconds = (after - Utc::now()).num_seconds().max(0);
            u32::try_from((seconds + 86_399) / 86_400).unwrap_or(u32::MAX)
        });
        Ok(RemovedCourse {
            removed_id: tombstone.course_id.clone(),
            source_id: tombstone.source_id,
            source_kind,
            external_id: tombstone.external_id,
            course_id: tombstone.course_id,
            code: tombstone.code,
            name: tombstone.name,
            reason: tombstone.reason,
            state: tombstone.state,
            removed_at: tombstone.removed_at,
            purge_after: tombstone.purge_after,
            purged_at: tombstone.purged_at,
            purge_in_days,
            keep_files: tombstone.keep_files,
            files_pending: tombstone.files_pending,
        })
    }
}

/// What a purge acts on: the removals in `removed_ids` (`NotFound` for an unknown one), or every
/// due purge and purged course whose files still wait for the Trash (`None`).
fn purge_targets_in(store: &Store, removed_ids: Option<&[String]>) -> Result<Vec<Tombstone>> {
    match removed_ids {
        Some(ids) => ids
            .iter()
            .map(|id| store.tombstone(id)?.ok_or_else(|| no_removed_course(id)))
            .collect(),
        None => due_or_waiting(store),
    }
}

/// Due purges, then purged courses whose files still wait for the Trash.
fn due_or_waiting(store: &Store) -> Result<Vec<Tombstone>> {
    let mut targets = store.due_purges(Utc::now())?;
    targets.extend(
        store
            .tombstones()?
            .into_iter()
            .filter(|t| t.state == TombstoneState::Purged && t.files_pending),
    );
    Ok(targets)
}

/// The folders under `files_dir` holding a removed Canvas course's downloads: its own
/// `<CODE>-<id>` folder, the folders its materials' files are in, and the same course under
/// older codes (`…-<id>`). Every other course, of any source, owns what it uses: such a folder
/// is kept, and the older-code sweep is skipped when another course shares the Canvas id.
/// Only real folders that are direct children of `files_dir` (never a symbolic link, never
/// `..`) are returned.
fn download_dirs(store: &Store, files_dir: &Path, tombstone: &Tombstone) -> Result<Vec<PathBuf>> {
    let parents = |course_id: &str| -> Result<Vec<PathBuf>> {
        Ok(store
            .list_materials(course_id)?
            .into_iter()
            .filter_map(|m| crate::download_dir_of(files_dir, Path::new(m.local_path.as_deref()?)))
            .collect())
    };
    let suffix = crate::fold_name(&pagelamp_canvas::course_dir_suffix(&tombstone.external_id));
    let mut own = vec![pagelamp_canvas::course_files_dir(
        files_dir,
        tombstone.code.as_deref(),
        &tombstone.external_id,
    )];
    own.extend(parents(&tombstone.course_id)?);
    let mut kept = std::collections::HashSet::new();
    let mut shared_suffix = false;
    for other in store
        .list_all_courses()?
        .into_iter()
        .filter(|c| c.id != tombstone.course_id)
    {
        kept.insert(crate::dir_key(&pagelamp_canvas::course_files_dir(
            files_dir,
            other.code.as_deref(),
            &other.external_id,
        )));
        for dir in parents(&other.id)? {
            kept.insert(crate::dir_key(&dir));
        }
        shared_suffix |=
            crate::fold_name(&pagelamp_canvas::course_dir_suffix(&other.external_id)) == suffix;
    }
    if !shared_suffix && let Ok(entries) = std::fs::read_dir(files_dir) {
        for entry in entries.flatten() {
            if crate::fold_name(&entry.file_name().to_string_lossy()).ends_with(&suffix) {
                own.push(entry.path());
            }
        }
    }
    own.sort();
    own.dedup();
    own.retain(|dir| {
        !kept.contains(&crate::dir_key(dir))
            && crate::is_download_dir(files_dir, dir)
            && std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir())
    });
    Ok(own)
}

/// Settle every `restoring` tombstone (`sync.lock` held, no restore running): see
/// `settle_restore`.
fn settle_restores(store: &Store) -> Result<()> {
    for tombstone in store.tombstones()? {
        if tombstone.state == TombstoneState::Restoring {
            settle_restore(store, &tombstone.course_id)?;
        }
    }
    Ok(())
}

/// The end of a restore, also one a quit interrupted: the course is back (its row exists, so
/// the sync brought it) and gets the student's settings, or it goes back to `purged`. Whether
/// it is back.
fn settle_restore(store: &Store, removed_id: &str) -> Result<bool> {
    if store.get_course(removed_id)?.is_some() {
        store.apply_restored_settings(removed_id, Utc::now())?;
        Ok(true)
    } else {
        store.set_tombstone_state(removed_id, TombstoneState::Purged)?;
        Ok(false)
    }
}

/// The pre-update backup the removal dialog offers to delete, if there is one.
fn backup_info(store: &Store) -> Result<Option<BackupInfo>> {
    let Some(record) = store.last_migration_backup()? else {
        return Ok(None);
    };
    if record.outcome != MigrationBackupOutcome::Ok {
        return Ok(None);
    }
    let age_days = (Utc::now() - record.at).num_days().max(0);
    let old = age_days >= BACKUP_DELETE_BY_DEFAULT_DAYS;
    Ok(Some(BackupInfo {
        age_days: u32::try_from(age_days).unwrap_or(u32::MAX),
        delete_by_default: old,
        reason_code: if old { "backup_old" } else { "backup_recent" }.to_string(),
    }))
}

fn no_removed_course(id: &str) -> AppError {
    AppError::new(
        AppErrorKind::NotFound,
        format!("There is no removed course {id}."),
    )
}
