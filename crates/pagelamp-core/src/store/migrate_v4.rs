//! The data step of the schema-4 migration (calendar design §3.2, as confirmed 2026-09-29):
//! PageLamp 0.1 term overrides that survived the v3 clean-up become accepted course calendars.
//!
//! - One row per course with `user_term_start` set: state `accepted`, origin `user` when the
//!   student confirmed the dates (`course_dates.confirmed` setting, a JSON array of course
//!   ids), else `legacy` (the Timeline tab then asks "Check this course's dates").
//! - A course with only `user_term_end` gets no row (the resolver keeps it as the student's
//!   last day of classes). `user_term_*` stay: they mirror the calendar for v3 readers.

use chrono::{NaiveDate, SubsecRound, Utc};
use rusqlite::params;

use super::Store;
use crate::Result;

/// Settings key listing the courses whose dates the student confirmed.
pub const COURSE_DATES_CONFIRMED: &str = "course_dates.confirmed";

/// The fingerprint of the rows this step writes. The views know them by it: these rows restate
/// the 0.1 overrides that `user_term_*` still hold, and the resolver keeps reading those (with
/// their plausibility checks and a borrowed end) until the student saves or accepts new dates.
pub const MIGRATED_FINGERPRINT: &str = "legacy";

impl Store {
    /// Runs inside the migration transaction, right after `SCHEMA_V4`.
    pub(super) fn migrate_legacy_calendars(&self) -> Result<()> {
        let confirmed: Vec<String> = self.setting(COURSE_DATES_CONFIRMED)?.unwrap_or_default();
        let overrides: Vec<(String, NaiveDate, Option<NaiveDate>)> = self.query_list(
            "SELECT id, user_term_start, user_term_end FROM courses
             WHERE user_term_start IS NOT NULL ORDER BY id",
            [],
            |row| {
                Ok((
                    row.get("id")?,
                    super::get_value(row, "user_term_start")?,
                    super::get_opt_value(row, "user_term_end")?,
                ))
            },
        )?;
        let now = Utc::now().trunc_subsecs(0).to_rfc3339();
        for (course_id, start, end) in overrides {
            let origin = if confirmed.contains(&course_id) {
                "user"
            } else {
                "legacy"
            };
            self.conn.execute(
                "INSERT INTO course_calendars
                     (course_id, origin, state, calendar_json, evidence_json, checks_json,
                      manifest_json, fingerprint, created_at, decided_at)
                 VALUES (?1, ?2, 'accepted', ?3, '[]', '{}', '[]', ?4, ?5, ?5)",
                params![
                    course_id,
                    origin,
                    legacy_calendar_json(start, end),
                    MIGRATED_FINGERPRINT,
                    now
                ],
            )?;
        }
        Ok(())
    }
}

/// The calendar JSON of a 0.1 override: the course lane's `legacy_calendar` (the start is
/// always kept; the end only when start → end spans 4 to 36 weeks).
fn legacy_calendar_json(start: NaiveDate, end: Option<NaiveDate>) -> String {
    serde_json::to_string(&crate::calendar::legacy_calendar(start, end))
        .expect("a course calendar serialises")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn a_legacy_override_becomes_the_lanes_calendar_json() {
        // The shape the migration stored before the lane merged (it must not change).
        assert_eq!(
            legacy_calendar_json(date(2026, 9, 8), Some(date(2026, 12, 8))),
            r#"{"segments":[{"first_class":"2026-09-08","last_class":"2026-12-08","first_week_number":1}],"breaks":[],"exam_period":null,"final_exam_on":null,"weeks":[]}"#
        );
        for end in [date(2026, 9, 1), date(2026, 9, 20), date(2027, 8, 31)] {
            assert!(
                legacy_calendar_json(date(2026, 9, 8), Some(end)).contains(r#""last_class":null"#)
            );
        }
        assert!(
            legacy_calendar_json(date(2026, 9, 8), Some(date(2027, 4, 9)))
                .contains(r#""last_class":"2027-04-09""#)
        );
    }
}
