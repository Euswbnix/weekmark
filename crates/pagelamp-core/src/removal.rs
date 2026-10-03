//! Removing a course from PageLamp, in two stages (docs/design/v0.3-course-calendar.md §8.3–
//! §8.5): the plain data a tombstone keeps. The store methods are in `store::tombstones`, the
//! facade in `pagelamp_app::course::removal`.
//!
//! - Stage 1 hides the course and writes a `pending` tombstone: nothing is deleted, and undo
//!   puts everything back.
//! - Stage 2 (after 7 days, or "Delete now") deletes the course's local data; the tombstone
//!   stays `purged`, with the name and the student's settings, so a sync doesn't add the
//!   course back and a restore can put the settings back.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ai::MaterialSharing;
use crate::calendar::CourseCalendar;
use crate::model::{AiPolicy, Timestamp};

/// Days between stage 1 and the purge.
pub const PURGE_AFTER_DAYS: i64 = 7;

/// Why a course was removed (display only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RemovalReason {
    Ended,
    Inactive,
    NotMine,
    Other,
}

impl RemovalReason {
    pub const ALL: [RemovalReason; 4] = [
        RemovalReason::Ended,
        RemovalReason::Inactive,
        RemovalReason::NotMine,
        RemovalReason::Other,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RemovalReason::Ended => "ended",
            RemovalReason::Inactive => "inactive",
            RemovalReason::NotMine => "not_mine",
            RemovalReason::Other => "other",
        }
    }
}

/// Where a removed course is in the two stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TombstoneState {
    /// Removed; local data kept until `purge_after` (undo possible).
    Pending,
    /// Local data deleted; the name stays so sync doesn't add it back.
    Purged,
    /// Being synced back after a purge.
    Restoring,
}

impl TombstoneState {
    pub const ALL: [TombstoneState; 3] = [
        TombstoneState::Pending,
        TombstoneState::Purged,
        TombstoneState::Restoring,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TombstoneState::Pending => "pending",
            TombstoneState::Purged => "purged",
            TombstoneState::Restoring => "restoring",
        }
    }

    /// Whether a sync leaves the course out (a restore syncs it back).
    pub fn skipped_by_sync(self) -> bool {
        self != TombstoneState::Restoring
    }
}

/// The student's own settings of a course, kept with its tombstone (`settings_json`) so a
/// restore can put them back. No material text: the calendar keeps its dates only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CourseSettings {
    pub ai_policy: Option<AiPolicy>,
    pub ai_policy_note: Option<String>,
    pub ai_access: Option<bool>,
    pub material_sharing: Option<MaterialSharing>,
    /// Hidden before the removal (undo puts it back).
    pub hidden: bool,
    pub user_term_start: Option<NaiveDate>,
    pub user_term_end: Option<NaiveDate>,
    pub keep_current_until: Option<NaiveDate>,
    /// The calendar in force, without labels or topics (material text).
    pub calendar: Option<CourseCalendar>,
    /// Canvas listed the course as restricted by date when it was removed: a sync can't bring
    /// it back (a restore says so).
    pub access_restricted: bool,
}

impl CourseSettings {
    /// Whether the student changed anything from the defaults ("has custom settings").
    pub fn customised(&self) -> bool {
        self.ai_policy.is_some_and(|p| p != AiPolicy::Unknown)
            || self.ai_policy_note.is_some()
            || self.ai_access == Some(false)
            || self
                .material_sharing
                .is_some_and(|s| s != MaterialSharing::Unanswered)
            || self.user_term_start.is_some()
            || self.user_term_end.is_some()
            || self.keep_current_until.is_some()
            || self.calendar.is_some()
    }
}

/// `calendar` without its material text (break labels, week topics).
pub fn dates_only(mut calendar: CourseCalendar) -> CourseCalendar {
    for item in &mut calendar.breaks {
        item.label.clear();
    }
    for week in &mut calendar.weeks {
        week.topic = None;
    }
    calendar
}

/// A removed course (`course_tombstones`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tombstone {
    pub source_id: String,
    pub external_id: String,
    pub course_id: String,
    pub code: Option<String>,
    pub name: String,
    pub reason: RemovalReason,
    pub state: TombstoneState,
    pub removed_at: Timestamp,
    pub purge_after: Option<Timestamp>,
    pub purged_at: Option<Timestamp>,
    /// "Keep downloaded files": the purge leaves them.
    pub keep_files: bool,
    /// The downloaded files still wait for the Trash: set with the purge itself and cleared
    /// only once every folder is handled, so a quit or a failed move is retried later.
    pub files_pending: bool,
    /// "Also delete the pre-update backup": done with the purge (stage 2), never at stage 1.
    pub delete_backup: bool,
    pub settings: CourseSettings,
}
