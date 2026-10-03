//! Which reminders were shown (`reminders_shown`, schema 4; model-access design §5.3, §5.4):
//! reminder ids are kept 60 days. The table also holds one-time notices of other kinds (the
//! question (b) reminder, `material_sharing:<course>`), which these methods never touch.

use std::collections::HashSet;

use chrono::Duration;
use rusqlite::params;

use super::{Store, ts_text};
use crate::Result;
use crate::model::Timestamp;
use crate::reminders::REMINDER_ID_PREFIXES;

/// How long a shown reminder's id is kept.
pub const REMINDERS_SHOWN_DAYS: i64 = 60;

/// `id LIKE …` for every reminder prefix ('_' escaped: it is a LIKE wildcard).
fn reminder_filter() -> String {
    REMINDER_ID_PREFIXES
        .iter()
        .map(|prefix| format!("id LIKE '{}%' ESCAPE '\\'", prefix.replace('_', "\\_")))
        .collect::<Vec<_>>()
        .join(" OR ")
}

impl Store {
    /// The ids of shown reminders (not other notices).
    pub fn shown_reminders(&self) -> Result<HashSet<String>> {
        Ok(self
            .query_list(
                &format!("SELECT id FROM reminders_shown WHERE {}", reminder_filter()),
                [],
                |row| row.get::<_, String>(0),
            )?
            .into_iter()
            .collect())
    }

    /// Record `ids` as shown at `now` (again: no change), and forget reminder ids older than
    /// 60 days. The caller checks the ids are reminder ids.
    pub fn mark_reminders_shown(&self, ids: &[String], now: Timestamp) -> Result<()> {
        self.atomic(|| {
            for id in ids {
                self.conn.execute(
                    "INSERT OR IGNORE INTO reminders_shown (id, shown_at) VALUES (?1, ?2)",
                    params![id, ts_text(now)],
                )?;
            }
            self.conn.execute(
                &format!(
                    "DELETE FROM reminders_shown WHERE shown_at < ?1 AND ({})",
                    reminder_filter()
                ),
                [ts_text(now - Duration::days(REMINDERS_SHOWN_DAYS))],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;

    #[test]
    fn reminder_ids_are_kept_60_days_and_other_notices_stay() {
        let store = Store::open_in_memory().unwrap();
        let old = Utc.with_ymd_and_hms(2026, 7, 1, 9, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 11, 2, 9, 0, 0).unwrap();
        store
            .mark_reminders_shown(&["weekly_digest:2026-06-29".to_string()], old)
            .unwrap();
        assert!(
            store
                .claim_sharing_reminder("folder:x/course/A", old)
                .unwrap()
        );
        store
            .mark_reminders_shown(&["weekly_digest:2026-11-02".to_string()], now)
            .unwrap();
        assert_eq!(
            store.shown_reminders().unwrap(),
            HashSet::from(["weekly_digest:2026-11-02".to_string()])
        );
        // The question (b) reminder is still claimed.
        assert!(
            !store
                .claim_sharing_reminder("folder:x/course/A", now)
                .unwrap()
        );
    }
}
