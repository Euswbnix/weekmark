//! Store methods of course removal (schema 4, `course_tombstones`; calendar design §8.3–§8.5).
//!
//! - Stage 1 (`remove_course`) writes the tombstone and sets `hidden = 1` in one transaction,
//!   so v3 readers already leave the course out; new readers leave tombstoned courses out of
//!   every course list (`Store::list_courses`).
//! - Stage 2 (`purge_course`) deletes the course's rows (its LMS events explicitly, the rest by
//!   cascade) with `secure_delete` on (freed pages are zeroed), compacts the full-text index
//!   and marks the tombstone `purged`, with `files_pending` set in the same transaction when
//!   downloaded files are left to move. The caller then truncates the WAL
//!   (`checkpoint_after_purge`, outside a transaction), moves the files to the Trash and only
//!   then clears `files_pending`.
//! - The tombstone matches `(source_id, external_id)`; a sync skips `pending` and `purged` ones.

use std::collections::HashMap;

use chrono::Duration;
use rusqlite::{Row, params};

use super::{Store, TextValue, expect_changed, get_opt_value, get_value, opt_date_text, ts_text};
use crate::Result;
use crate::calendar::CourseCalendar;
use crate::model::Timestamp;
use crate::removal::{
    CourseSettings, PURGE_AFTER_DAYS, RemovalReason, Tombstone, TombstoneState, dates_only,
};

impl TextValue for RemovalReason {
    fn parse_text(text: &str) -> Option<Self> {
        RemovalReason::ALL.into_iter().find(|r| r.as_str() == text)
    }
}

impl TextValue for TombstoneState {
    fn parse_text(text: &str) -> Option<Self> {
        TombstoneState::ALL.into_iter().find(|s| s.as_str() == text)
    }
}

const TOMBSTONE_COLUMNS: &str = "source_id, external_id, course_id, code, name, reason, state, \
     removed_at, purge_after, purged_at, keep_files, files_pending, delete_backup, settings_json";

fn tombstone_from_row(row: &Row<'_>) -> rusqlite::Result<Tombstone> {
    let settings: String = row.get("settings_json")?;
    Ok(Tombstone {
        source_id: row.get("source_id")?,
        external_id: row.get("external_id")?,
        course_id: row.get("course_id")?,
        code: row.get("code")?,
        name: row.get("name")?,
        reason: get_value(row, "reason")?,
        state: get_value(row, "state")?,
        removed_at: get_value(row, "removed_at")?,
        purge_after: get_opt_value(row, "purge_after")?,
        purged_at: get_opt_value(row, "purged_at")?,
        keep_files: row.get("keep_files")?,
        files_pending: row.get("files_pending")?,
        delete_backup: row.get("delete_backup")?,
        // An unreadable snapshot only loses what a restore would put back.
        settings: serde_json::from_str(&settings).unwrap_or_default(),
    })
}

impl Store {
    /// Stage 1: remove `course_id` (hide it, write a `pending` tombstone purged after 7 days).
    /// `keep_files` and `delete_backup` are the student's choices for the purge. `NotFound` for
    /// an unknown course; `Invalid` if it is already removed.
    pub fn remove_course(
        &self,
        course_id: &str,
        reason: RemovalReason,
        keep_files: bool,
        delete_backup: bool,
        now: Timestamp,
    ) -> Result<Tombstone> {
        self.atomic(|| {
            let course = self
                .get_course(course_id)?
                .ok_or_else(|| crate::Error::NotFound(format!("course '{course_id}'")))?;
            if self.tombstone(course_id)?.is_some() {
                return Err(crate::Error::Invalid(format!(
                    "{} is already removed",
                    course.display_name()
                )));
            }
            let settings = self.course_settings(course_id)?;
            let tombstone = Tombstone {
                source_id: course.source_id.clone(),
                external_id: course.external_id.clone(),
                course_id: course.id.clone(),
                code: course.code.clone(),
                name: course.name.clone(),
                reason,
                state: TombstoneState::Pending,
                removed_at: now,
                purge_after: Some(now + Duration::days(PURGE_AFTER_DAYS)),
                purged_at: None,
                keep_files,
                files_pending: false,
                delete_backup,
                settings,
            };
            self.conn.execute(
                &format!(
                    "INSERT INTO course_tombstones ({TOMBSTONE_COLUMNS})
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, 0, ?11, ?12)"
                ),
                params![
                    tombstone.source_id,
                    tombstone.external_id,
                    tombstone.course_id,
                    tombstone.code,
                    tombstone.name,
                    reason.as_str(),
                    TombstoneState::Pending.as_str(),
                    ts_text(now),
                    tombstone.purge_after.map(ts_text),
                    keep_files,
                    delete_backup,
                    serde_json::to_string(&tombstone.settings).expect("settings serialise"),
                ],
            )?;
            self.set_course_hidden(course_id, true)?;
            Ok(tombstone)
        })
    }

    /// The student's own settings of a course, as a tombstone keeps them (no material text).
    /// `NotFound` for an unknown course.
    pub fn course_settings(&self, course_id: &str) -> Result<CourseSettings> {
        let course = self
            .get_course(course_id)?
            .ok_or_else(|| crate::Error::NotFound(format!("course '{course_id}'")))?;
        let data = self.course_term_data(course_id)?.unwrap_or_default();
        Ok(CourseSettings {
            ai_policy: Some(course.ai_policy),
            ai_policy_note: course.ai_policy_note,
            ai_access: Some(course.ai_access),
            material_sharing: Some(course.material_sharing),
            hidden: course.hidden,
            user_term_start: data.user_term_start,
            user_term_end: data.user_term_end,
            keep_current_until: data.keep_current_until,
            calendar: self.accepted_calendar_json(course_id)?.map(dates_only),
            access_restricted: data.lms.access_restricted == Some(true),
        })
    }

    /// How many generated items (explanations, notes, readings) a course has.
    pub fn course_generation_count(&self, course_id: &str) -> Result<u32> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM generations WHERE course_id = ?1",
            [course_id],
            |row| row.get(0),
        )?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// The accepted calendar's `calendar_json`, if the course has one.
    fn accepted_calendar_json(&self, course_id: &str) -> Result<Option<CourseCalendar>> {
        let text: Option<String> = self.query_opt(
            "SELECT calendar_json FROM course_calendars
             WHERE course_id = ?1 AND state = 'accepted'",
            [course_id],
            |row| row.get(0),
        )?;
        Ok(text.and_then(|text| serde_json::from_str(&text).ok()))
    }

    /// Every tombstone, newest removal first.
    pub fn tombstones(&self) -> Result<Vec<Tombstone>> {
        self.query_list(
            &format!(
                "SELECT {TOMBSTONE_COLUMNS} FROM course_tombstones
                 ORDER BY removed_at DESC, course_id"
            ),
            [],
            tombstone_from_row,
        )
    }

    /// The tombstone of `course_id`, if it was removed.
    pub fn tombstone(&self, course_id: &str) -> Result<Option<Tombstone>> {
        self.query_opt(
            &format!("SELECT {TOMBSTONE_COLUMNS} FROM course_tombstones WHERE course_id = ?1"),
            [course_id],
            tombstone_from_row,
        )
    }

    /// A source's tombstones by external id (what its sync compares against).
    pub fn tombstone_states(&self, source_id: &str) -> Result<HashMap<String, TombstoneState>> {
        let pairs: Vec<(String, TombstoneState)> = self.query_list(
            "SELECT external_id, state FROM course_tombstones WHERE source_id = ?1",
            [source_id],
            |row| Ok((row.get("external_id")?, get_value(row, "state")?)),
        )?;
        Ok(pairs.into_iter().collect())
    }

    /// `pending` tombstones whose purge is due at `now`.
    pub fn due_purges(&self, now: Timestamp) -> Result<Vec<Tombstone>> {
        self.query_list(
            &format!(
                "SELECT {TOMBSTONE_COLUMNS} FROM course_tombstones
                 WHERE state = 'pending' AND purge_after <= ?1
                 ORDER BY purge_after, course_id"
            ),
            [ts_text(now)],
            tombstone_from_row,
        )
    }

    /// Undo stage 1: the course's previous `hidden` comes back and the tombstone goes.
    /// `Invalid` unless the removal is still `pending`.
    pub fn undo_removal(&self, course_id: &str) -> Result<()> {
        self.atomic(|| {
            let tombstone = self.require_tombstone(course_id)?;
            if tombstone.state != TombstoneState::Pending {
                return Err(crate::Error::Invalid(
                    "only a removal that isn't purged yet can be undone".to_string(),
                ));
            }
            self.set_course_hidden(course_id, tombstone.settings.hidden)?;
            self.delete_tombstone(course_id)
        })
    }

    /// Stage 2, the database part: delete the course's LMS events (by its own source; feed
    /// events only lose their link), the course row with everything that cascades (modules,
    /// materials, chunks and their index entries, calendars, generations), compact the
    /// full-text index, and mark the tombstone `purged`, with `files_pending` (downloaded files
    /// are left to move) in the same transaction. `NotFound` without a tombstone.
    pub fn purge_course(&self, course_id: &str, now: Timestamp, files_pending: bool) -> Result<()> {
        // For this connection: pages the deletes free are overwritten with zeros.
        self.conn.execute_batch("PRAGMA secure_delete = ON;")?;
        self.atomic(|| {
            let tombstone = self.require_tombstone(course_id)?;
            self.conn.execute(
                "DELETE FROM events WHERE course_id = ?1 AND source_id = ?2",
                params![course_id, tombstone.source_id],
            )?;
            self.conn
                .execute("DELETE FROM courses WHERE id = ?1", [course_id])?;
            // The delete triggers only add FTS5 delete keys; merge the segments so the old
            // terms go.
            self.conn
                .execute("INSERT INTO chunks_fts(chunks_fts) VALUES('optimize')", [])?;
            self.conn.execute(
                "UPDATE course_tombstones
                 SET state = 'purged', purged_at = ?2, purge_after = NULL, files_pending = ?3
                 WHERE course_id = ?1",
                params![course_id, ts_text(now), files_pending],
            )?;
            Ok(())
        })
    }

    /// After a purge, outside any transaction: copy the zeroed pages into the database and
    /// empty the WAL, so no older copy of the deleted rows stays in it. Best effort: a reader
    /// holding the WAL open only delays it to a later checkpoint.
    pub fn checkpoint_after_purge(&self) {
        let _ = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    }

    /// Set a tombstone's state (a restore: `restoring`, then back to `purged` if the course
    /// didn't come back).
    pub fn set_tombstone_state(&self, course_id: &str, state: TombstoneState) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE course_tombstones SET state = ?2 WHERE course_id = ?1",
            params![course_id, state.as_str()],
        )?;
        expect_changed(changed, "removed course", course_id)
    }

    /// Whether the downloaded files still wait for the Trash (cleared only once every folder
    /// is handled).
    pub fn set_tombstone_files_pending(&self, course_id: &str, pending: bool) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE course_tombstones SET files_pending = ?2 WHERE course_id = ?1",
            params![course_id, pending],
        )?;
        expect_changed(changed, "removed course", course_id)
    }

    /// Forget a tombstone (the next sync brings the course back).
    pub fn delete_tombstone(&self, course_id: &str) -> Result<()> {
        let changed = self.conn.execute(
            "DELETE FROM course_tombstones WHERE course_id = ?1",
            [course_id],
        )?;
        expect_changed(changed, "removed course", course_id)
    }

    /// A restored course gets the student's settings back: AI policy and note, access, the
    /// materials question, hidden, term dates, "I'm still taking this", and the calendar as a
    /// `restored` calendar in force. Then the tombstone goes.
    pub fn apply_restored_settings(&self, course_id: &str, now: Timestamp) -> Result<()> {
        self.atomic(|| {
            let tombstone = self.require_tombstone(course_id)?;
            let s = &tombstone.settings;
            if let Some(policy) = s.ai_policy {
                self.set_course_policy(course_id, policy, s.ai_policy_note.as_deref())?;
            }
            if let Some(access) = s.ai_access {
                self.set_course_ai_access(course_id, access)?;
            }
            if let Some(sharing) = s.material_sharing {
                self.set_course_material_sharing(course_id, sharing)?;
            }
            self.set_course_hidden(course_id, s.hidden)?;
            self.conn.execute(
                "UPDATE courses SET user_term_start = ?2, user_term_end = ?3,
                     keep_current_until = ?4
                 WHERE id = ?1",
                params![
                    course_id,
                    opt_date_text(s.user_term_start),
                    opt_date_text(s.user_term_end),
                    opt_date_text(s.keep_current_until)
                ],
            )?;
            if let Some(calendar) = &s.calendar {
                self.conn.execute(
                    "INSERT INTO course_calendars
                         (course_id, origin, state, calendar_json, evidence_json, checks_json,
                          manifest_json, fingerprint, created_at, decided_at)
                     VALUES (?1, 'restored', 'accepted', ?2, '[]', '{}', '[]', 'restored', ?3, ?3)",
                    params![
                        course_id,
                        serde_json::to_string(calendar).expect("calendars serialise"),
                        ts_text(now)
                    ],
                )?;
            }
            self.delete_tombstone(course_id)
        })
    }

    fn require_tombstone(&self, course_id: &str) -> Result<Tombstone> {
        self.tombstone(course_id)?
            .ok_or_else(|| crate::Error::NotFound(format!("removed course '{course_id}'")))
    }
}
