//! Automatic sync: the student's setting and the record of automatic attempts.
//!
//! Only data and the rule's arithmetic live here, so the MCP server (which depends on this
//! crate alone and never syncs) can say whether PageLamp refreshes by itself and when it last
//! did. Starting a sync is the facade's job (`pagelamp-app`); nothing an MCP client can write
//! is read here.
//!
//! | key                  | value                                                        |
//! |----------------------|--------------------------------------------------------------|
//! | `sync.prefs`         | `SyncPrefs` (absent: automatic sync twice a day)             |
//! | `sync.auto_attempts` | `AutoSyncAttempts`                                           |
//! | `sync.light`         | `LightSync`                                                  |
//!
//! Two scopes, each with its own clock (`SyncScope`): a sync nobody is at the app for asks
//! Canvas for nothing with a course in its path, since Canvas may record such a request as
//! the student's activity in that course; a sync started while the student is at the app
//! reads everything, like pressing Sync. A source's
//! `last_synced_at` always means its last full sync.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{TimeDelta, Timelike};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Result;
use crate::model::{SourceErrorKind, SourceKind, SourceRecord, Timestamp};
use crate::store::Store;
use crate::views::STALE_AFTER_HOURS;

/// The setting's key in the `settings` table.
pub const SYNC_PREFS_KEY: &str = "sync.prefs";
/// The attempts record's key in the `settings` table.
pub const AUTO_SYNC_ATTEMPTS_KEY: &str = "sync.auto_attempts";
/// The light sync record's key in the `settings` table.
pub const LIGHT_SYNC_KEY: &str = "sync.light";

/// The first retry after an automatic attempt that didn't finish: 1 hour, doubling with each
/// further one (1, 2, 4, 8 h…) up to the setting's interval.
pub const FIRST_RETRY: TimeDelta = TimeDelta::hours(1);
/// The most automatic attempts one source gets in any 24 hours, whatever the backoff says.
pub const MAX_ATTEMPTS_PER_SOURCE_PER_DAY: usize = 6;

/// How often PageLamp syncs by itself while it runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoSync {
    /// Only when the student starts a sync.
    Off,
    /// When the last successful sync is 24 hours old.
    Daily,
    /// When the last successful sync is 12 hours old (the default).
    #[default]
    TwiceDaily,
}

impl AutoSync {
    pub fn as_str(self) -> &'static str {
        match self {
            AutoSync::Off => "off",
            AutoSync::Daily => "daily",
            AutoSync::TwiceDaily => "twice_daily",
        }
    }

    /// How old the last successful sync must be before the next automatic one; `None`: off.
    pub fn interval(self) -> Option<TimeDelta> {
        match self {
            AutoSync::Off => None,
            AutoSync::Daily => Some(TimeDelta::hours(24)),
            AutoSync::TwiceDaily => Some(TimeDelta::hours(12)),
        }
    }

    /// After how long without a successful sync the data counts as stale: the interval and
    /// half of it again (an automatic sync may be an hour late and retry), and never less than
    /// 24 hours. So "once a day" isn't stale every morning.
    pub fn stale_after(self) -> TimeDelta {
        let at_least = TimeDelta::hours(STALE_AFTER_HOURS);
        match self.interval() {
            Some(interval) => (interval + interval / 2).max(at_least),
            None => at_least,
        }
    }
}

/// Why PageLamp starts a sync by itself. The shell says which; what each one syncs is the
/// facade's decision (the shells never choose it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AutoSyncTrigger {
    /// The app's timer, with nobody known to be at the app. From a Canvas source such a run
    /// reads only the course list, deadlines and announcements.
    Unattended,
    /// The student just opened PageLamp, brought its window to the front or closed What's new.
    /// Such a run reads everything, like pressing Sync (it never downloads files).
    Attended,
}

/// What an automatic sync reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyncScope {
    /// Everything a sync the student starts reads (never downloading files).
    Everything,
    /// From a Canvas source, only what it serves without a course in the path: the course
    /// list, planner items (deadlines) and announcements. Canvas may record a request for a
    /// course's modules, files, pages or assignments as the student's activity in that course;
    /// these requests aren't made. Folder and calendar-feed sources are read in full.
    UserLevel,
}

/// What the light (`SyncScope::UserLevel`) runs left behind. A source's `last_synced_at` is
/// never touched by one: it keeps meaning the last full sync.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LightSync {
    /// Per Canvas source id: when a light run last succeeded (its deadlines and announcements
    /// are at least this fresh).
    pub synced_at: BTreeMap<String, Timestamp>,
    /// Courses a light run saw for the first time: listed, with deadlines and announcements,
    /// but their modules and materials wait for a full sync.
    pub structure_pending: BTreeSet<String>,
    /// The waiting courses a full sync has tried and couldn't read (its module listing failed
    /// in a way that may pass). They keep waiting, which is the truth for every view, but they
    /// no longer make a full sync due by themselves: the next regular one tries again.
    pub structure_tried: BTreeSet<String>,
}

impl LightSync {
    /// When `source`'s deadlines and announcements were last read: the newer of its last full
    /// sync and its last light run.
    pub fn deadlines_synced_at(&self, source: &SourceRecord) -> Option<Timestamp> {
        source
            .last_synced_at
            .max(self.synced_at.get(&source.id).copied())
    }

    /// Per source a light run read after its last full sync: when that was. A source without
    /// an entry has one clock (`last_synced_at`).
    pub fn later_than_full(&self, sources: &[SourceRecord]) -> BTreeMap<String, Timestamp> {
        sources
            .iter()
            .filter_map(|source| {
                let at = *self.synced_at.get(&source.id)?;
                source
                    .last_synced_at
                    .is_none_or(|full| at > full)
                    .then(|| (source.id.clone(), at))
            })
            .collect()
    }

    /// When `source` was last read as far as `scope` reads it.
    pub fn synced_at(&self, source: &SourceRecord, scope: SyncScope) -> Option<Timestamp> {
        match scope {
            SyncScope::UserLevel if source.kind == SourceKind::Canvas => {
                self.deadlines_synced_at(source)
            }
            SyncScope::UserLevel | SyncScope::Everything => source.last_synced_at,
        }
    }

    /// Whether a course of `source` waits for a full sync that no full sync has tried yet
    /// (the one case that makes a full sync due whatever the interval).
    pub fn has_untried(&self, source: &SourceRecord) -> bool {
        let prefix = format!("{}/", source.id);
        self.structure_pending
            .iter()
            .any(|course| course.starts_with(&prefix) && !self.structure_tried.contains(course))
    }

    /// A light run of the source ended well at `at` and found `new_courses`. The time is kept
    /// to the whole second, as the sources table keeps a full sync's, so the two compare.
    pub fn light_run_ended(&mut self, source_id: &str, at: Timestamp, new_courses: &[String]) {
        let at = at.with_nanosecond(0).unwrap_or(at);
        self.synced_at.insert(source_id.to_string(), at);
        self.structure_pending.extend(new_courses.iter().cloned());
    }

    /// A full sync of the source ended well.
    ///
    /// - `only`: the courses whose structure it read, when it was limited to some. Those wait
    ///   no more.
    /// - Otherwise (`None`) every course of the source is released, also one that has left
    ///   Canvas's list since it was found (no sync will read it, and it must not keep a full
    ///   sync due for ever), and the light run's stamp goes: the source has one clock again.
    /// - `unread`: courses it selected and couldn't read the structure of. One that was
    ///   waiting keeps waiting, and is marked as tried.
    ///
    /// Returns whether anything changed.
    pub fn full_sync_read(
        &mut self,
        source_id: &str,
        only: Option<&[String]>,
        unread: &[String],
    ) -> bool {
        let before = self.clone();
        match only {
            Some(courses) => {
                for course in courses {
                    self.structure_pending.remove(course);
                }
            }
            None => {
                let prefix = format!("{source_id}/");
                self.structure_pending
                    .retain(|course| !course.starts_with(&prefix) || unread.contains(course));
                self.synced_at.remove(source_id);
            }
        }
        for course in unread {
            if self.structure_pending.contains(course) {
                self.structure_tried.insert(course.clone());
            }
        }
        let waiting = &self.structure_pending;
        self.structure_tried
            .retain(|course| waiting.contains(course));
        *self != before
    }

    /// The source was removed: nothing of it is remembered. Returns whether anything changed.
    pub fn forget_source(&mut self, source_id: &str) -> bool {
        self.full_sync_read(source_id, None, &[])
    }
}

/// The student's sync settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SyncPrefs {
    /// How often PageLamp syncs by itself while it runs. Default twice a day.
    pub auto_sync: AutoSync,
}

/// What PageLamp remembers about its automatic sync attempts. An attempt is counted when it
/// is asked for, before anything else: one that is refused (another sync runs, What's new is
/// waiting) or fails counts as much as one that ran, so nothing retries back to back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoSyncAttempts {
    /// When the last automatic attempt from the timer (`SyncScope::UserLevel`) was asked for.
    pub last_at: Option<Timestamp>,
    /// Such attempts in a row that didn't end with every source it could sync synced.
    pub failed: u32,
    /// The same two for the attempts made while the student is at the app
    /// (`SyncScope::Everything`). Each scope waits after its own attempts only: a source that
    /// keeps failing for the timer's runs must not keep the full sync from ever being due.
    pub full_last_at: Option<Timestamp>,
    pub full_failed: u32,
    /// When an automatic run last ended with every source it could sync synced (a run from
    /// the timer reads only deadlines and announcements from Canvas).
    pub last_ok_at: Option<Timestamp>,
    /// Per source id: when its automatic attempts of the last 24 hours started.
    pub by_source: BTreeMap<String, Vec<Timestamp>>,
    /// When the student last stopped a sync (one they started or an automatic one). Nothing
    /// starts by itself for `FIRST_RETRY` after it: a student who pressed Stop, for example
    /// during the first sync, doesn't want one to begin again at once.
    pub stopped_at: Option<Timestamp>,
}

impl AutoSyncAttempts {
    /// The earliest moment the next automatic attempt of `scope` may start: after a failed or
    /// refused one of that scope, 1 h, 2 h, 4 h… later, at most the setting's interval;
    /// `None`: no wait. (An attempt recorded after `now` is one the clock was set forward for:
    /// it tells nothing, see `known`.)
    pub fn retry_not_before(
        &self,
        interval: TimeDelta,
        scope: SyncScope,
        now: Timestamp,
    ) -> Option<Timestamp> {
        let (last, failed) = match scope {
            SyncScope::UserLevel => (known(self.last_at, now)?, self.failed),
            SyncScope::Everything => (known(self.full_last_at, now)?, self.full_failed),
        };
        if failed == 0 {
            return None;
        }
        let doubled = FIRST_RETRY
            .checked_mul(1 << (failed - 1).min(16))
            .unwrap_or(interval);
        Some(last + doubled.min(interval))
    }

    /// Whether the student stopped a sync less than `FIRST_RETRY` before `now`.
    pub fn stopped_recently(&self, now: Timestamp) -> bool {
        known(self.stopped_at, now).is_some_and(|stopped| now < stopped + FIRST_RETRY)
    }

    /// How many automatic attempts `source_id` had in the 24 hours before `now`.
    pub fn attempts_in_last_day(&self, source_id: &str, now: Timestamp) -> usize {
        let since = now - TimeDelta::hours(24);
        self.by_source.get(source_id).map_or(0, |starts| {
            starts
                .iter()
                .filter(|at| **at > since && **at <= now)
                .count()
        })
    }

    /// Count an attempt of `scope` on `sources` at `now` (and forget starts older than 24
    /// hours). The cap per source is shared by both scopes.
    pub fn count(&mut self, sources: &[&SourceRecord], now: Timestamp, scope: SyncScope) {
        let since = now - TimeDelta::hours(24);
        self.by_source.retain(|_, starts| {
            starts.retain(|at| *at > since && *at <= now);
            !starts.is_empty()
        });
        for source in sources {
            self.by_source
                .entry(source.id.clone())
                .or_default()
                .push(now);
        }
        match scope {
            SyncScope::UserLevel => {
                self.last_at = Some(now);
                self.failed = self.failed.saturating_add(1);
            }
            SyncScope::Everything => {
                self.full_last_at = Some(now);
                self.full_failed = self.full_failed.saturating_add(1);
            }
        }
    }

    /// The attempt of `scope` counted last ended with every source it could sync synced. A
    /// full run that ends well read everything the timer's run reads: both waits are over.
    pub fn succeeded(&mut self, at: Timestamp, scope: SyncScope) {
        self.failed = 0;
        if scope == SyncScope::Everything {
            self.full_failed = 0;
        }
        self.last_ok_at = Some(at);
    }
}

/// The student's sync settings (the defaults when never set or unreadable; a failed read is
/// an error, never the default).
pub fn sync_prefs(store: &Store) -> Result<SyncPrefs> {
    Ok(store.setting_or_absent(SYNC_PREFS_KEY)?.unwrap_or_default())
}

/// How often PageLamp syncs by itself (`sync_prefs`).
pub fn auto_sync(store: &Store) -> Result<AutoSync> {
    Ok(sync_prefs(store)?.auto_sync)
}

/// The attempts record (empty when never written or unreadable; a failed read is an error).
pub fn attempts(store: &Store) -> Result<AutoSyncAttempts> {
    Ok(store
        .setting_or_absent(AUTO_SYNC_ATTEMPTS_KEY)?
        .unwrap_or_default())
}

/// The light sync record (empty when never written or unreadable; a failed read is an error).
pub fn light_sync(store: &Store) -> Result<LightSync> {
    Ok(store.setting_or_absent(LIGHT_SYNC_KEY)?.unwrap_or_default())
}

/// Whether an automatic sync may try `source` at all: not while its last sync failed for a
/// reason only the student can fix (an expired or revoked token or link, a missing folder or
/// address). Replacing the token, link or folder, or a sync the student starts, clears that.
pub fn needs_the_student(source: &SourceRecord) -> bool {
    matches!(
        source.last_error_kind,
        Some(SourceErrorKind::AuthExpiredOrRevoked | SourceErrorKind::NotFound)
    )
}

/// The sources an automatic sync would try at `now`: those that don't need the student and
/// are under the daily cap.
pub fn eligible<'a>(
    sources: &'a [SourceRecord],
    attempts: &AutoSyncAttempts,
    now: Timestamp,
) -> Vec<&'a SourceRecord> {
    sources
        .iter()
        .filter(|source| !needs_the_student(source))
        .filter(|source| {
            attempts.attempts_in_last_day(&source.id, now) < MAX_ATTEMPTS_PER_SOURCE_PER_DAY
        })
        .collect()
}

/// What the clock rule reads besides the setting.
#[derive(Clone, Copy)]
pub struct Clocks<'a> {
    pub sources: &'a [SourceRecord],
    pub attempts: &'a AutoSyncAttempts,
    pub light: &'a LightSync,
}

/// A stored time, unless it is after `now`. That happens when the computer's clock was set
/// forward, something was recorded and the clock was corrected: such a time tells nothing, and
/// reading it as it is would hold everything back until the clock catches up. A source "last
/// read" in the future counts as never read (one extra sync, which records the right time); a
/// wait that "started" in the future isn't one.
pub fn known(at: Option<Timestamp>, now: Timestamp) -> Option<Timestamp> {
    at.filter(|at| *at <= now)
}

/// The sources an automatic sync of `scope` reads at `now` by the clock alone (the facade adds
/// what only it knows: a sync running, What's new waiting). Empty when nothing is due: the
/// setting is off, a retry wait of this scope is running, or the student stopped a sync in the
/// last hour. Otherwise every source that can be tried and that was last read (as far as
/// `scope` reads) the interval ago or never. For a full sync also a source with a course that
/// waits for its first one and that no full sync has tried, whatever the interval.
///
/// Only these are read: a source that keeps failing brings the others no extra sync.
pub fn due_now<'a>(
    setting: AutoSync,
    clocks: Clocks<'a>,
    scope: SyncScope,
    now: Timestamp,
) -> Vec<&'a SourceRecord> {
    let Some(interval) = setting.interval() else {
        return Vec::new();
    };
    let waiting = clocks.attempts.stopped_recently(now)
        || clocks
            .attempts
            .retry_not_before(interval, scope, now)
            .is_some_and(|not_before| now < not_before);
    if waiting {
        return Vec::new();
    }
    eligible(clocks.sources, clocks.attempts, now)
        .into_iter()
        .filter(|source| {
            known(clocks.light.synced_at(source, scope), now).is_none_or(|at| now - at >= interval)
                || (scope == SyncScope::Everything && clocks.light.has_untried(source))
        })
        .collect()
}

/// Whether an automatic sync of `scope` is due at `now` by the clock alone (`due_now`).
pub fn due_by_the_clock(
    setting: AutoSync,
    clocks: Clocks<'_>,
    scope: SyncScope,
    now: Timestamp,
) -> bool {
    !due_now(setting, clocks, scope, now).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SourceKind;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn source(id: &str, synced: Option<&str>, error: Option<SourceErrorKind>) -> SourceRecord {
        SourceRecord {
            id: id.into(),
            kind: SourceKind::Canvas,
            label: id.into(),
            config: serde_json::json!({}),
            last_synced_at: synced.map(at),
            last_error: error.map(|_| "failed".to_string()),
            last_error_kind: error,
        }
    }

    #[test]
    fn the_setting_names_its_interval_and_when_data_is_stale() {
        assert_eq!(AutoSync::default(), AutoSync::TwiceDaily);
        assert_eq!(AutoSync::Off.interval(), None);
        assert_eq!(AutoSync::Daily.interval(), Some(TimeDelta::hours(24)));
        assert_eq!(AutoSync::TwiceDaily.interval(), Some(TimeDelta::hours(12)));
        // Never stale within the interval: once a day is stale after 36 h, not 24.
        assert_eq!(AutoSync::Off.stale_after(), TimeDelta::hours(24));
        assert_eq!(AutoSync::TwiceDaily.stale_after(), TimeDelta::hours(24));
        assert_eq!(AutoSync::Daily.stale_after(), TimeDelta::hours(36));
        for setting in [AutoSync::Off, AutoSync::Daily, AutoSync::TwiceDaily] {
            let json = serde_json::to_string(&setting).unwrap();
            assert_eq!(json, format!("\"{}\"", setting.as_str()));
        }
    }

    #[test]
    fn due_when_the_stalest_source_that_can_be_tried_is_an_interval_old() {
        let now = at("2026-10-03T12:00:00Z");
        let none = AutoSyncAttempts::default();
        let fresh = source("a", Some("2026-10-03T03:00:00Z"), None);
        let old = source("b", Some("2026-10-02T23:59:00Z"), None);
        let never = source("c", None, None);
        let light = LightSync::default();
        let due = |setting, sources: &[SourceRecord]| {
            let clocks = Clocks {
                sources,
                attempts: &none,
                light: &light,
            };
            // With no light run recorded, both scopes read the same clock.
            let everything = due_by_the_clock(setting, clocks, SyncScope::Everything, now);
            assert_eq!(
                everything,
                due_by_the_clock(setting, clocks, SyncScope::UserLevel, now)
            );
            everything
        };
        assert!(!due(AutoSync::TwiceDaily, &[]), "no sources");
        assert!(
            !due(AutoSync::TwiceDaily, std::slice::from_ref(&fresh)),
            "9 h old"
        );
        assert!(due(AutoSync::TwiceDaily, &[fresh.clone(), old.clone()]));
        assert!(!due(AutoSync::Daily, &[fresh.clone(), old.clone()]));
        assert!(
            due(AutoSync::Daily, std::slice::from_ref(&never)),
            "never synced"
        );
        assert!(!due(AutoSync::Off, &[old.clone(), never.clone()]));
        // A source only the student can fix is neither tried nor a reason to try.
        for kind in [
            SourceErrorKind::AuthExpiredOrRevoked,
            SourceErrorKind::NotFound,
        ] {
            let blocked = source("d", Some("2026-09-01T00:00:00Z"), Some(kind));
            assert!(needs_the_student(&blocked));
            assert!(!due(AutoSync::TwiceDaily, std::slice::from_ref(&blocked)));
            assert!(!due(AutoSync::TwiceDaily, &[blocked, fresh.clone()]));
        }
        // A failure that may pass by itself is tried again.
        let flaky = source(
            "e",
            Some("2026-09-01T00:00:00Z"),
            Some(SourceErrorKind::Network),
        );
        assert!(due(AutoSync::TwiceDaily, &[flaky]));
    }

    #[test]
    fn a_light_run_satisfies_the_light_clock_only_and_a_new_course_asks_for_a_full_sync() {
        let now = at("2026-10-03T12:00:00Z");
        let none = AutoSyncAttempts::default();
        // The last full sync of Canvas was 30 hours ago; a light run finished an hour ago.
        let canvas = source("canvas:lms", Some("2026-10-02T06:00:00Z"), None);
        let mut folder = source("folder:demo", Some("2026-10-03T11:00:00Z"), None);
        folder.kind = SourceKind::Folder;
        let sources = [canvas.clone(), folder.clone()];
        let mut light = LightSync::default();
        light
            .synced_at
            .insert(canvas.id.clone(), at("2026-10-03T11:00:00Z"));
        let due = |light: &LightSync, sources: &[SourceRecord], scope| {
            let clocks = Clocks {
                sources,
                attempts: &none,
                light,
            };
            due_by_the_clock(AutoSync::TwiceDaily, clocks, scope, now)
        };
        assert!(
            !due(&light, &sources, SyncScope::UserLevel),
            "deadlines are fresh"
        );
        assert!(
            due(&light, &sources, SyncScope::Everything),
            "materials aren't"
        );
        assert_eq!(
            light.deadlines_synced_at(&canvas),
            Some(at("2026-10-03T11:00:00Z"))
        );
        assert_eq!(
            light.synced_at(&canvas, SyncScope::Everything),
            canvas.last_synced_at
        );
        assert_eq!(
            light.later_than_full(&sources),
            BTreeMap::from([(canvas.id.clone(), at("2026-10-03T11:00:00Z"))])
        );
        // A full sync after the light run: one clock again.
        let synced_since = source("canvas:lms", Some("2026-10-03T11:30:00Z"), None);
        assert!(light.later_than_full(&[synced_since]).is_empty());
        // A light stamp means nothing for a folder: it is always read in full.
        light
            .synced_at
            .insert(folder.id.clone(), at("2026-10-03T11:59:00Z"));
        folder.last_synced_at = Some(at("2026-10-02T06:00:00Z"));
        assert!(due(
            &light,
            std::slice::from_ref(&folder),
            SyncScope::UserLevel
        ));

        // Everything is fresh, but a light run found a new course: a full sync is due (the
        // light clock isn't moved by it).
        let fresh = source("canvas:lms", Some("2026-10-03T10:00:00Z"), None);
        let mut found = LightSync::default();
        assert!(!due(
            &found,
            std::slice::from_ref(&fresh),
            SyncScope::Everything
        ));
        found
            .structure_pending
            .insert("canvas:lms/course/7".to_string());
        assert!(found.has_untried(&fresh));
        assert!(due(
            &found,
            std::slice::from_ref(&fresh),
            SyncScope::Everything
        ));
        assert!(!due(
            &found,
            std::slice::from_ref(&fresh),
            SyncScope::UserLevel
        ));
        // (Another source's course doesn't count.)
        let other = source("canvas:lms2", Some("2026-10-03T10:00:00Z"), None);
        assert!(!found.has_untried(&other));
        assert!(!due(
            &found,
            std::slice::from_ref(&other),
            SyncScope::Everything
        ));

        // A full sync limited to other courses leaves it waiting; a full sync of the whole
        // source releases it, also when the course has left Canvas's list meanwhile.
        found
            .structure_pending
            .insert("canvas:lms2/course/1".to_string());
        assert!(!found.full_sync_read(
            "canvas:lms",
            Some(&["canvas:lms/course/8".to_string()]),
            &[]
        ));
        assert!(found.has_untried(&fresh));
        assert!(found.full_sync_read("canvas:lms", None, &[]));
        assert!(!found.has_untried(&fresh) && found.has_untried(&other));
        assert!(
            !found.full_sync_read("canvas:lms", None, &[]),
            "nothing left to change"
        );
        // A light run's time is kept to the second, like a full sync's in the sources table;
        // a full sync of the whole source drops it (one clock again), a limited one doesn't.
        let ended: Timestamp = "2026-10-03T11:00:00.755Z".parse().unwrap();
        found.light_run_ended("canvas:lms", ended, &["canvas:lms/course/9".to_string()]);
        assert_eq!(found.synced_at["canvas:lms"], at("2026-10-03T11:00:00Z"));
        assert!(found.full_sync_read(
            "canvas:lms",
            Some(&["canvas:lms/course/9".to_string()]),
            &[]
        ));
        assert!(found.synced_at.contains_key("canvas:lms") && !found.has_untried(&fresh));
        assert!(found.full_sync_read("canvas:lms", None, &[]));
        assert!(!found.synced_at.contains_key("canvas:lms"));
        // Removing a source forgets its stamp and its waiting courses.
        found
            .synced_at
            .insert("canvas:lms2".to_string(), at("2026-10-03T11:00:00Z"));
        assert!(found.forget_source("canvas:lms2"));
        assert_eq!(found, LightSync::default());
    }

    #[test]
    fn a_failed_or_refused_attempt_waits_longer_each_time_up_to_the_interval() {
        let old = [source("a", Some("2026-10-01T00:00:00Z"), None)];
        let refs: Vec<&SourceRecord> = old.iter().collect();
        let mut attempts = AutoSyncAttempts::default();
        let start = at("2026-10-03T00:00:00Z");
        let twice = AutoSync::TwiceDaily;
        let light = LightSync::default();
        let full = SyncScope::Everything;
        let due_for = |attempts: &AutoSyncAttempts, hours: i64, scope| {
            let clocks = Clocks {
                sources: &old,
                attempts,
                light: &light,
            };
            let now = start + TimeDelta::hours(hours);
            due_by_the_clock(twice, clocks, scope, now)
        };
        let due = |attempts: &AutoSyncAttempts, hours: i64| due_for(attempts, hours, full);
        assert!(due(&attempts, 0));
        // Counted when asked for: nothing retries back to back.
        attempts.count(&refs, start, full);
        assert_eq!(attempts.full_failed, 1);
        assert!(!due(&attempts, 0));
        assert!(
            !due(&attempts, 0) && due(&attempts, 1),
            "1 h after the first"
        );
        attempts.count(&refs, start + TimeDelta::hours(1), full);
        assert!(
            !due(&attempts, 2) && due(&attempts, 3),
            "2 h after the second"
        );
        attempts.count(&refs, start + TimeDelta::hours(3), full);
        assert!(
            !due(&attempts, 6) && due(&attempts, 7),
            "4 h after the third"
        );
        attempts.count(&refs, start + TimeDelta::hours(7), full);
        assert!(
            !due(&attempts, 14) && due(&attempts, 15),
            "8 h after the fourth"
        );
        attempts.count(&refs, start + TimeDelta::hours(15), full);
        // 16 h would be next: capped at the interval (12 h).
        assert!(!due(&attempts, 26) && due(&attempts, 27));

        // Each scope waits after its own attempts only. All of these were full ones, so the
        // timer's run was due all along; and a source that keeps failing for the timer's runs
        // doesn't keep the full sync from being due when the student comes back.
        assert_eq!((attempts.failed, attempts.last_at), (0, None));
        assert!(due_for(&attempts, 16, SyncScope::UserLevel));
        let mut timer = AutoSyncAttempts::default();
        for hour in [0, 1, 3] {
            timer.count(&refs, start + TimeDelta::hours(hour), SyncScope::UserLevel);
        }
        assert_eq!((timer.failed, timer.full_failed), (3, 0));
        assert!(
            !due_for(&timer, 4, SyncScope::UserLevel),
            "4 h after the third"
        );
        assert!(
            due_for(&timer, 4, full),
            "the full sync has no wait of its own"
        );
        // A light run that ends well ends the timer's wait only; a full one ends both.
        attempts.count(&refs, start + TimeDelta::hours(27), SyncScope::UserLevel);
        attempts.succeeded(start + TimeDelta::hours(27), SyncScope::UserLevel);
        assert_eq!((attempts.failed, attempts.full_failed), (0, 5));
        attempts.count(&refs, start + TimeDelta::hours(28), SyncScope::UserLevel);
        attempts.succeeded(start + TimeDelta::hours(28), full);
        assert_eq!(
            (
                attempts.failed,
                attempts.full_failed,
                attempts.retry_not_before(TimeDelta::hours(12), full, start + TimeDelta::hours(28)),
                attempts.retry_not_before(
                    TimeDelta::hours(12),
                    SyncScope::UserLevel,
                    start + TimeDelta::hours(28)
                )
            ),
            (0, 0, None, None)
        );
        // The student stopped a sync: nothing starts by itself for an hour, with no attempt
        // counted and whatever the retry wait says.
        let stopped = AutoSyncAttempts {
            stopped_at: Some(start),
            ..AutoSyncAttempts::default()
        };
        assert!(!due(&stopped, 0) && due(&stopped, 1));
        assert!(!due_for(&stopped, 0, SyncScope::UserLevel));
        assert!(stopped.stopped_recently(start + TimeDelta::minutes(59)));
        assert!(!stopped.stopped_recently(start + TimeDelta::minutes(60)));
    }

    /// A course that waits for its first full sync makes one due at once, but never past the
    /// retry wait or a source's cap for the day.
    #[test]
    fn a_waiting_course_doesnt_get_past_the_wait_or_the_daily_cap() {
        let start = at("2026-10-03T12:00:00Z");
        let fresh = [source("canvas:lms", Some("2026-10-03T11:00:00Z"), None)];
        let refs: Vec<&SourceRecord> = fresh.iter().collect();
        let mut light = LightSync::default();
        light
            .structure_pending
            .insert("canvas:lms/course/7".to_string());
        let full = SyncScope::Everything;
        let due = |attempts: &AutoSyncAttempts, now| {
            let clocks = Clocks {
                sources: &fresh,
                attempts,
                light: &light,
            };
            due_by_the_clock(AutoSync::TwiceDaily, clocks, full, now)
        };
        let mut attempts = AutoSyncAttempts::default();
        assert!(due(&attempts, start), "the course waits");
        // A full attempt that didn't finish: the wait holds, course or not.
        attempts.count(&refs, start, full);
        assert!(!due(&attempts, start + TimeDelta::minutes(59)));
        assert!(due(&attempts, start + TimeDelta::minutes(61)));
        // Six attempts in a day (of either scope): the source isn't tried again, course or not.
        let mut capped = AutoSyncAttempts::default();
        for hour in 0..3 {
            capped.count(&refs, start + TimeDelta::hours(hour), SyncScope::UserLevel);
            capped.count(&refs, start + TimeDelta::hours(hour), full);
        }
        capped.succeeded(start + TimeDelta::hours(3), full);
        assert_eq!(
            capped.attempts_in_last_day("canvas:lms", start + TimeDelta::hours(4)),
            6
        );
        assert!(!due(&capped, start + TimeDelta::hours(4)), "at its cap");
        assert!(due(&capped, start + TimeDelta::hours(25)));
    }

    /// A waiting course a full sync tried and couldn't read keeps waiting (every view says
    /// so), but no longer makes a full sync due outside the interval.
    #[test]
    fn a_course_a_full_sync_could_not_read_keeps_waiting_without_making_one_due() {
        let now = at("2026-10-03T12:00:00Z");
        let fresh = source("canvas:lms", Some("2026-10-03T11:00:00Z"), None);
        let none = AutoSyncAttempts::default();
        let course = "canvas:lms/course/7".to_string();
        let gone = "canvas:lms/course/8".to_string();
        let mut light = LightSync::default();
        light.light_run_ended(&fresh.id, now, &[course.clone(), gone.clone()]);
        let due = |light: &LightSync, now| {
            let clocks = Clocks {
                sources: std::slice::from_ref(&fresh),
                attempts: &none,
                light,
            };
            due_by_the_clock(AutoSync::TwiceDaily, clocks, SyncScope::Everything, now)
        };
        assert!(due(&light, now));
        // The full sync couldn't read course 7 (and course 8 has left Canvas's list).
        assert!(light.full_sync_read(&fresh.id, None, std::slice::from_ref(&course)));
        assert_eq!(light.structure_pending, BTreeSet::from([course.clone()]));
        assert_eq!(light.structure_tried, BTreeSet::from([course.clone()]));
        assert!(!light.has_untried(&fresh));
        assert!(!due(&light, now), "tried: not due before the interval");
        assert!(
            due(&light, now + TimeDelta::hours(12)),
            "the regular one tries again"
        );
        // Still unread then: nothing changes. Read: it waits no more.
        assert!(!light.full_sync_read(&fresh.id, None, std::slice::from_ref(&course)));
        assert!(light.full_sync_read(&fresh.id, None, &[]));
        assert_eq!(light, LightSync::default());
        // A sync limited to it that couldn't read it marks it tried as well.
        light.light_run_ended(&fresh.id, now, std::slice::from_ref(&course));
        assert!(light.full_sync_read(&fresh.id, Some(&[]), std::slice::from_ref(&course)));
        assert!(light.structure_pending.contains(&course) && !light.has_untried(&fresh));
        assert!(
            light.synced_at.contains_key(&fresh.id),
            "a limited sync keeps the stamp"
        );
    }

    /// Only the sources that are due are read: one that keeps failing brings the others no
    /// extra sync.
    #[test]
    fn only_the_sources_that_are_due_are_read() {
        let now = at("2026-10-03T12:00:00Z");
        let canvas = source("canvas:lms", Some("2026-10-03T10:00:00Z"), None);
        let mut feed = source("ical:feed", Some("2026-10-02T20:00:00Z"), None);
        feed.kind = SourceKind::Ical;
        let never = source("canvas:other", None, None);
        let sources = [canvas.clone(), feed.clone(), never.clone()];
        let none = AutoSyncAttempts::default();
        let mut light = LightSync::default();
        let ids = |light: &LightSync, scope| -> Vec<String> {
            let clocks = Clocks {
                sources: &sources,
                attempts: &none,
                light,
            };
            due_now(AutoSync::TwiceDaily, clocks, scope, now)
                .iter()
                .map(|source| source.id.clone())
                .collect()
        };
        for scope in [SyncScope::UserLevel, SyncScope::Everything] {
            assert_eq!(ids(&light, scope), ["ical:feed", "canvas:other"]);
        }
        // A course of the fresh Canvas source waits: a full sync reads that source too.
        light
            .structure_pending
            .insert("canvas:lms/course/7".to_string());
        assert_eq!(
            ids(&light, SyncScope::Everything),
            ["canvas:lms", "ical:feed", "canvas:other"]
        );
        assert_eq!(
            ids(&light, SyncScope::UserLevel),
            ["ical:feed", "canvas:other"]
        );
    }

    /// The clock was set forward, something was recorded, the clock was corrected: a time
    /// after now tells nothing, and must not hold everything back until the clock catches up.
    #[test]
    fn a_time_recorded_in_the_future_holds_nothing_back() {
        let now = at("2026-10-03T12:00:00Z");
        let later = "2027-01-01T00:00:00Z";
        let ahead = [source("a", Some(later), None)];
        let refs: Vec<&SourceRecord> = ahead.iter().collect();
        let light = LightSync::default();
        let due = |attempts: &AutoSyncAttempts| {
            let clocks = Clocks {
                sources: &ahead,
                attempts,
                light: &light,
            };
            due_by_the_clock(AutoSync::TwiceDaily, clocks, SyncScope::Everything, now)
        };
        // "Last synced" next year: read as never synced, so one sync sets it right.
        assert!(due(&AutoSyncAttempts::default()));
        // A stop, a failed attempt and six attempts next year: none of them counts today.
        let mut attempts = AutoSyncAttempts {
            stopped_at: Some(at(later)),
            ..AutoSyncAttempts::default()
        };
        for _ in 0..6 {
            attempts.count(&refs, at(later), SyncScope::Everything);
        }
        assert!(!attempts.stopped_recently(now));
        assert_eq!(
            attempts.retry_not_before(TimeDelta::hours(12), SyncScope::Everything, now),
            None
        );
        assert_eq!(attempts.attempts_in_last_day("a", now), 0);
        assert!(due(&attempts));
        // The next count drops them.
        attempts.count(&refs, now, SyncScope::Everything);
        assert_eq!(attempts.by_source["a"], [now]);
        assert_eq!(known(Some(now), now), Some(now));
        assert_eq!(known(Some(at(later)), now), None);
    }

    #[test]
    fn a_source_gets_at_most_six_automatic_attempts_a_day() {
        let sources = [source("a", None, None), source("b", None, None)];
        let only_a = [&sources[0]];
        let mut attempts = AutoSyncAttempts::default();
        let start = at("2026-10-03T00:00:00Z");
        for hour in 0..6 {
            attempts.count(
                &only_a,
                start + TimeDelta::hours(hour),
                SyncScope::UserLevel,
            );
        }
        let now = start + TimeDelta::hours(6);
        assert_eq!(attempts.attempts_in_last_day("a", now), 6);
        let ids = |attempts: &AutoSyncAttempts, now| -> Vec<String> {
            eligible(&sources, attempts, now)
                .iter()
                .map(|s| s.id.clone())
                .collect()
        };
        assert_eq!(ids(&attempts, now), ["b"], "a is at its cap");
        // 24 h after its first attempt, a may be tried again.
        let later = start + TimeDelta::hours(24) + TimeDelta::minutes(1);
        assert_eq!(ids(&attempts, later), ["a", "b"]);
        // Counting forgets what is older than a day.
        attempts.count(&[], later, SyncScope::Everything);
        assert_eq!(attempts.by_source["a"].len(), 5);
    }
}
