//! What the app is doing right now (`App::activity`), so a UI can hold back "Install and
//! restart" while anything runs (model-access design §3.7): syncs, file downloads, Codex installs
//! and model runs started through this `App` (every clone shares the registry).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{SubsecRound, Utc};
use pagelamp_core::model::Timestamp;
use pagelamp_core::paths;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{App, AppState, lock};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    /// `sync_all` / `sync_source`.
    Sync,
    /// `download_course_files`.
    Download,
    /// `install_codex`: downloading and verifying the Codex runtime.
    CodexInstall,
    /// A model run, from its gate to its stored answer (`read_course_calendar`, and every
    /// feature that runs a model); a batch (`read_course_calendars`) is one item.
    Generation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActivityItem {
    pub kind: ActivityKind,
    /// The one source it works on; `None` for `sync_all` and a generation.
    pub source_id: Option<String>,
    /// A generation's id (a batch's for a batch): `cancel_generation` stops it.
    #[serde(default)]
    pub generation_id: Option<String>,
    pub started_at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Activity {
    /// Work this app is doing, oldest first.
    pub items: Vec<ActivityItem>,
    /// Another process (the CLI, another window) is syncing: `sync.lock` is held, but not by
    /// this app.
    pub other_process_syncing: bool,
}

/// The running work of one `App` (shared by its clones).
#[derive(Default)]
pub(crate) struct Registry {
    next: AtomicU64,
    items: Mutex<BTreeMap<u64, ActivityItem>>,
}

/// Registered work; dropping it (the work ended, failed or was dropped) deregisters it.
pub(crate) struct ActivityGuard {
    state: Arc<AppState>,
    id: u64,
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        let mut items = self
            .state
            .activity
            .items
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        items.remove(&self.id);
    }
}

impl App {
    /// What this app is doing right now, and whether another process is syncing.
    pub fn activity(&self) -> Activity {
        let items: Vec<ActivityItem> = self
            .state
            .activity
            .items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        // Syncs and downloads both hold sync.lock (a Codex install doesn't).
        let syncing_here = items
            .iter()
            .any(|item| matches!(item.kind, ActivityKind::Sync | ActivityKind::Download));
        let locked = lock::is_locked(&paths::sync_lock_path_in(self.data_dir()));
        Activity {
            other_process_syncing: locked && !syncing_here,
            items,
        }
    }

    /// Register work until the returned guard is dropped.
    pub(crate) fn begin_activity(
        &self,
        kind: ActivityKind,
        source_id: Option<&str>,
    ) -> ActivityGuard {
        self.register_activity(kind, source_id, None)
    }

    /// Register generation `generation_id` until the returned guard is dropped
    /// (`App::register_run`).
    pub(crate) fn begin_generation(&self, generation_id: &str) -> ActivityGuard {
        self.register_activity(ActivityKind::Generation, None, Some(generation_id))
    }

    fn register_activity(
        &self,
        kind: ActivityKind,
        source_id: Option<&str>,
        generation_id: Option<&str>,
    ) -> ActivityGuard {
        let id = self.state.activity.next.fetch_add(1, Ordering::Relaxed);
        self.state
            .activity
            .items
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id,
                ActivityItem {
                    kind,
                    source_id: source_id.map(str::to_string),
                    generation_id: generation_id.map(str::to_string),
                    started_at: Utc::now().trunc_subsecs(0),
                },
            );
        ActivityGuard {
            state: Arc::clone(&self.state),
            id,
        }
    }
}
