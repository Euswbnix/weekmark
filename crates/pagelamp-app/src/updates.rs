//! Update preferences, the first-run / upgrade state and update-check records (v0.3 M0.4).
//!
//! The logic lives here, in the facade (plan §1.3, rule 12): the desktop app, the Swift shell
//! and the CLI only render `startup_tasks` and call the setters. Everything is stored in the
//! schema-3 `settings` table:
//!
//! | key                          | value                                                    |
//! |------------------------------|----------------------------------------------------------|
//! | `updates.prefs`              | `UpdatePrefs`                                            |
//! | `updates.disclosure_acknowledged` | `true` once the student saw what the update check sends |
//! | `updates.last_check`         | `UpdateCheckRecord`                                      |
//! | `app.last_run_version`       | the version that last ran `startup_tasks`                |
//! | `app.whats_new_acknowledged` | the version whose What's new the student closed          |
//! | `app.mac.last_run_version`, `app.mac.whats_new_acknowledged` | the same for the Mac app |
//!
//! `startup_tasks(now)` classifies the launch once per process (fresh install, upgrade and
//! from which version) and records the running version; everything else is recomputed on every
//! call, because the app may run for days and callers poll it on a timer.
//!
//! What's new is per shell (`Shell`): the desktop app and the Mac app share the data folder,
//! and whichever asked first would otherwise take the other's upgrade. The desktop app keeps
//! the keys above, unchanged; the Mac app has its own, its first run shows nothing, and it never
//! gets the update-check topic (it updates itself with Sparkle), so only the desktop app's
//! What's new counts as the update disclosure.

use chrono::TimeDelta;
use pagelamp_core::model::Timestamp;
use pagelamp_core::store::Store;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{App, Reminder, Result};

const PREFS_KEY: &str = "updates.prefs";
const DISCLOSURE_KEY: &str = "updates.disclosure_acknowledged";
const LAST_CHECK_KEY: &str = "updates.last_check";
const LAST_RUN_KEY: &str = "app.last_run_version";
const WHATS_NEW_ACK_KEY: &str = "app.whats_new_acknowledged";
const MAC_LAST_RUN_KEY: &str = "app.mac.last_run_version";
const MAC_WHATS_NEW_ACK_KEY: &str = "app.mac.whats_new_acknowledged";

/// Which app shell opened the data folder (`App::set_shell`): What's new is per shell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shell {
    /// The desktop app, and every caller that doesn't say (the CLI, tests): the keys alpha.1
    /// wrote, the update-check topic and the update disclosure.
    #[default]
    Desktop,
    /// The native Mac app: its own keys, and no update-check topic (Sparkle updates it).
    Mac,
}

impl Shell {
    fn last_run_key(self) -> &'static str {
        match self {
            Shell::Desktop => LAST_RUN_KEY,
            Shell::Mac => MAC_LAST_RUN_KEY,
        }
    }

    fn whats_new_ack_key(self) -> &'static str {
        match self {
            Shell::Desktop => WHATS_NEW_ACK_KEY,
            Shell::Mac => MAC_WHATS_NEW_ACK_KEY,
        }
    }
}

/// How often the automatic update check runs.
const CHECK_INTERVAL: TimeDelta = TimeDelta::hours(24);

/// This build's What's new topics, each with the version that introduced it. A student coming
/// from an older version sees the topics introduced after that version.
const WHATS_NEW: &[(WhatsNewTopic, &str)] = &[
    (WhatsNewTopic::UpdateCheck, "0.3.0-alpha.1"),
    (WhatsNewTopic::CourseWeeks, "0.3.0-alpha.1"),
    (WhatsNewTopic::CourseRemoval, "0.3.0-alpha.2"),
    (WhatsNewTopic::SyllabusReading, "0.3.0-alpha.3"),
    (WhatsNewTopic::AiWriting, "0.3.0-beta.1"),
    (WhatsNewTopic::Reminders, "0.3.0-beta.1"),
];

/// Where updates come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Stable,
    Beta,
}

/// The student's update settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UpdatePrefs {
    /// Check for updates automatically (daily). Default on (D2).
    pub auto_check: bool,
    /// The chosen channel; `None` = the default for this build (`effective_update_channel`).
    pub channel: Option<UpdateChannel>,
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        UpdatePrefs {
            auto_check: true,
            channel: None,
        }
    }
}

/// A topic of the one-time What's new sheet shown after an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WhatsNewTopic {
    /// PageLamp now checks for updates (what it sends, how to turn it off).
    UpdateCheck,
    /// Course weeks, phases and the Past group.
    CourseWeeks,
    /// Removing finished courses: 7 days to undo, the student's own folders untouched.
    CourseRemoval,
    /// AI reads a syllabus into cited date proposals; setting up a model (including the
    /// ChatGPT plan) comes with it.
    SyllabusReading,
    /// Study plans and weekly explanations with the student's own model.
    AiWriting,
    /// Deadline and weekly reminders; the tray and starting at login (opt-in).
    Reminders,
}

/// What's new since `since` (`None`: an update from 0.1, which didn't record its version).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WhatsNew {
    pub since: Option<String>,
    pub topics: Vec<WhatsNewTopic>,
}

/// What the app should do now (M0 subset; `startup_tasks`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StartupTasks {
    /// Show What's new (upgraders only) until `acknowledge_whats_new`.
    pub whats_new: Option<WhatsNew>,
    /// Run the automatic update check now: it is on, the student saw the disclosure (onboarding
    /// or What's new), no What's new is waiting, and the last check is 24 h or more ago.
    pub update_check_due: bool,
    /// The version this launch updated from (`None`: not an update, or an update from 0.1).
    /// Shows the "quit and reopen your AI app" banner.
    pub updated_from: Option<String>,
    /// Reminders to show now (`due_reminders`); `mark_reminders_shown` once shown.
    pub due_reminders: Vec<Reminder>,
    /// Removed courses wait for their purge (it is due, or a Trash move left files): run
    /// `purge_removed_courses(None)` (the app-start purge, calendar design §8.3).
    pub purge_due: bool,
    /// Removed courses whose downloaded files still wait for the Trash.
    pub removed_files_waiting: u32,
    /// Courses whose syllabus PageLamp could read with AI (only offered, D47): at most
    /// `STARTUP_LIST_MAX`, empty while "Not now" covers them (`snooze_calendar_offers`).
    pub calendar_offers: Vec<crate::SyllabusOffer>,
    pub calendar_offers_total: u32,
    /// Ids of the courses that look finished (`lifecycle_summary().suggested`): at most
    /// `STARTUP_LIST_MAX`, empty while the banner's "Not now" covers them.
    pub removal_suggestions: Vec<String>,
    pub removal_suggestions_total: u32,
    /// Prepare the weekly note now (`write_weekly_note` with `automatic`): the student opted
    /// in, the note's model is an API key or a model on this computer (never the ChatGPT or
    /// Claude plan, plan D27), it is Monday in the reminder zone, no automatic note was tried
    /// yet that Monday (one try, whatever its outcome), no note was written that day, and there
    /// is something to write about (an active course, a deadline in the next 7 days or a plan
    /// item).
    pub prepare_weekly_note: bool,
}

/// The most items of each list in `StartupTasks` (the totals say how many there are).
pub const STARTUP_LIST_MAX: usize = 20;

/// One update check and how it ended (`record_update_check`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UpdateCheckRecord {
    pub at: Timestamp,
    pub channel: UpdateChannel,
    pub outcome: UpdateCheckOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum UpdateCheckOutcome {
    UpToDate,
    Available {
        version: String,
    },
    /// A short code (e.g. `network`, `signature`, `manifest`), never a message or URL.
    Error {
        code: String,
    },
}

/// How this process's launch was classified (computed once, by the first `startup_tasks`).
#[derive(Clone, Debug)]
pub(crate) struct LaunchClass {
    /// An update from an older version (including 0.1).
    upgrade: bool,
    /// The version it updated from; `None` for 0.1 or when not an update.
    updated_from: Option<String>,
}

impl App {
    /// The shell that opened this app (default `Desktop`); set it before the first
    /// `startup_tasks`. Only the Mac app's FFI calls it.
    pub fn set_shell(&self, shell: Shell) {
        *self.state.shell.lock().unwrap_or_else(|e| e.into_inner()) = shell;
        // Classified again with this shell's keys.
        *self.state.launch.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub(crate) fn shell(&self) -> Shell {
        *self.state.shell.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The student's update settings (defaults when never set or unparseable; a failed read is
    /// an error, never the defaults).
    pub fn update_prefs(&self) -> Result<UpdatePrefs> {
        prefs_in(&self.read_store()?)
    }

    pub fn set_update_prefs(&self, prefs: UpdatePrefs) -> Result<()> {
        Ok(self.write_store()?.set_setting(PREFS_KEY, &prefs)?)
    }

    /// The channel updates come from: the student's choice, else Beta for a pre-release build
    /// (e.g. 0.3.0-alpha.1) and Stable otherwise.
    pub fn effective_update_channel(&self) -> Result<UpdateChannel> {
        Ok(self
            .update_prefs()?
            .channel
            .unwrap_or_else(|| default_channel(env!("CARGO_PKG_VERSION"))))
    }

    /// What to do at launch and on the app's timer (see the module docs). `now` is the caller's
    /// clock, so the answer is testable and follows a long-running app.
    pub fn startup_tasks(&self, now: Timestamp) -> Result<StartupTasks> {
        let launch = self.launch_class()?;
        let shell = self.shell();
        let store = self.read_store()?;
        let whats_new = if launch.upgrade && !whats_new_acknowledged(&store, shell)? {
            let topics = topics_for(shell, launch.updated_from.as_deref());
            (!topics.is_empty()).then(|| WhatsNew {
                since: launch.updated_from.clone(),
                topics,
            })
        } else {
            None
        };
        // A failed read fails the call (the app asks again on its timer): it must never look like
        // "checks on", "disclosed" or "never checked".
        let prefs = prefs_in(&store)?;
        let disclosed: bool = store.setting_or_absent(DISCLOSURE_KEY)?.unwrap_or(false);
        let last_check: Option<UpdateCheckRecord> = store.setting_or_absent(LAST_CHECK_KEY)?;
        let check_is_old = last_check.is_none_or(|check| now - check.at >= CHECK_INTERVAL);
        let tombstones = store.tombstones()?;
        let removed_files_waiting = tombstones.iter().filter(|t| t.files_pending).count();
        let purge_due = removed_files_waiting > 0 || !store.due_purges(now)?.is_empty();
        drop(store);
        // Reminders, offers and suggestions never keep the rest from the shell.
        let due_reminders = self.due_reminders(now).unwrap_or_else(|err| {
            tracing::warn!(target: "pagelamp::reminders", "due reminders failed: {:?}", err.kind);
            Vec::new()
        });
        let mut calendar_offers = self.unsnoozed_calendar_offers().unwrap_or_else(|err| {
            tracing::warn!(target: "pagelamp::calendar", "offers failed: {:?}", err.kind);
            Vec::new()
        });
        let mut removal_suggestions = match self.lifecycle_summary() {
            Ok(summary) if summary.show_banner => summary.suggested,
            Ok(_) => Vec::new(),
            Err(err) => {
                tracing::warn!(target: "pagelamp::lifecycle", "suggestions failed: {:?}", err.kind);
                Vec::new()
            }
        };
        let prepare_weekly_note = self.weekly_note_due(now).unwrap_or_else(|err| {
            tracing::warn!(target: "pagelamp::ai", "weekly note check failed: {:?}", err.kind);
            false
        });
        let calendar_offers_total = u32::try_from(calendar_offers.len()).unwrap_or(u32::MAX);
        let removal_suggestions_total =
            u32::try_from(removal_suggestions.len()).unwrap_or(u32::MAX);
        calendar_offers.truncate(STARTUP_LIST_MAX);
        removal_suggestions.truncate(STARTUP_LIST_MAX);
        Ok(StartupTasks {
            update_check_due: prefs.auto_check && disclosed && whats_new.is_none() && check_is_old,
            whats_new,
            updated_from: launch.updated_from.clone(),
            due_reminders,
            purge_due,
            removed_files_waiting: u32::try_from(removed_files_waiting).unwrap_or(u32::MAX),
            calendar_offers,
            calendar_offers_total,
            removal_suggestions,
            removal_suggestions_total,
            prepare_weekly_note,
        })
    }

    /// The student closed this shell's What's new. The desktop app's update-check topic counts
    /// as the update disclosure (the Mac app never shows that topic).
    pub fn acknowledge_whats_new(&self) -> Result<()> {
        let launch = self.launch_class()?;
        let shell = self.shell();
        let store = self.write_store()?;
        store.set_setting(shell.whats_new_ack_key(), &env!("CARGO_PKG_VERSION"))?;
        if topics_for(shell, launch.updated_from.as_deref()).contains(&WhatsNewTopic::UpdateCheck) {
            store.set_setting(DISCLOSURE_KEY, &true)?;
        }
        Ok(())
    }

    /// The student saw what the update check sends (onboarding, fresh installs).
    pub fn acknowledge_update_disclosure(&self) -> Result<()> {
        Ok(self.write_store()?.set_setting(DISCLOSURE_KEY, &true)?)
    }

    /// Remember how an update check ended (for `startup_tasks` and diagnostic reports).
    pub fn record_update_check(&self, record: UpdateCheckRecord) -> Result<()> {
        Ok(self.write_store()?.set_setting(LAST_CHECK_KEY, &record)?)
    }

    pub fn last_update_check(&self) -> Result<Option<UpdateCheckRecord>> {
        Ok(self.read_store()?.setting_or_absent(LAST_CHECK_KEY)?)
    }

    /// Classify this launch once per process: a fresh install gets no What's new (its
    /// acknowledgement is set to this version), an upgrade does; the running version is
    /// recorded for the next launch.
    fn launch_class(&self) -> Result<LaunchClass> {
        let mut cached = self.state.launch.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(launch) = cached.as_ref() {
            return Ok(launch.clone());
        }
        let current = env!("CARGO_PKG_VERSION");
        let shell = self.shell();
        let store = self.write_store()?;
        // Before any write: a failed read leaves the launch unclassified, and the next call
        // tries again.
        let last_run: Option<String> = store.setting_or_absent(shell.last_run_key())?;
        let launch = match last_run {
            Some(last) if last == current => LaunchClass {
                upgrade: false,
                updated_from: None,
            },
            Some(last) => LaunchClass {
                upgrade: true,
                updated_from: Some(last),
            },
            // 0.1 never recorded a version: for the desktop app it is an update if 0.1 left
            // data behind, as it was when this app opened the data dir (a new user's onboarding
            // adds a source before the shell asks for its startup tasks). The Mac app never ran
            // before: its first run, with nothing to show.
            None => {
                let upgrade = shell == Shell::Desktop && self.state.used_before_at_open;
                if !upgrade {
                    store.set_setting(shell.whats_new_ack_key(), &current)?;
                }
                LaunchClass {
                    upgrade,
                    updated_from: None,
                }
            }
        };
        store.set_setting(shell.last_run_key(), &current)?;
        *cached = Some(launch.clone());
        Ok(launch)
    }
}

/// Beta for pre-release builds, Stable otherwise.
fn default_channel(version: &str) -> UpdateChannel {
    match semver::Version::parse(version) {
        Ok(v) if v.pre.is_empty() => UpdateChannel::Stable,
        Ok(_) => UpdateChannel::Beta,
        Err(_) => UpdateChannel::Stable,
    }
}

/// The What's new topics introduced after `since` (`None`: from 0.1, i.e. all of them).
fn topics_since(since: Option<&str>) -> Vec<WhatsNewTopic> {
    let since = since.and_then(|v| semver::Version::parse(v).ok());
    WHATS_NEW
        .iter()
        .filter(|(_, introduced)| {
            let introduced = semver::Version::parse(introduced).expect("valid version");
            since.as_ref().is_none_or(|since| *since < introduced)
        })
        .map(|(topic, _)| *topic)
        .collect()
}

/// `topics_since` as `shell` shows them: the Mac app never gets the update-check topic.
fn topics_for(shell: Shell, since: Option<&str>) -> Vec<WhatsNewTopic> {
    let mut topics = topics_since(since);
    if shell != Shell::Desktop {
        topics.retain(|topic| *topic != WhatsNewTopic::UpdateCheck);
    }
    topics
}

fn whats_new_acknowledged(store: &Store, shell: Shell) -> Result<bool> {
    Ok(store
        .setting_or_absent::<String>(shell.whats_new_ack_key())?
        .is_some_and(|version| version == env!("CARGO_PKG_VERSION")))
}

/// The student's update settings in `store` (see `App::update_prefs`).
fn prefs_in(store: &Store) -> Result<UpdatePrefs> {
    Ok(store.setting_or_absent(PREFS_KEY)?.unwrap_or_default())
}

/// "2026-10-01 14:03 UTC (beta): up_to_date | available 0.3.0-alpha.2 | error network".
pub(crate) fn describe_check(record: &UpdateCheckRecord) -> String {
    let channel = match record.channel {
        UpdateChannel::Stable => "stable",
        UpdateChannel::Beta => "beta",
    };
    let outcome = match &record.outcome {
        UpdateCheckOutcome::UpToDate => "up_to_date".to_string(),
        UpdateCheckOutcome::Available { version } => format!("available {version}"),
        UpdateCheckOutcome::Error { code } => format!("error {code}"),
    };
    format!(
        "{} ({channel}): {outcome}",
        record.at.format("%Y-%m-%d %H:%M UTC")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_release_builds_default_to_beta() {
        assert_eq!(default_channel("0.3.0-alpha.1"), UpdateChannel::Beta);
        assert_eq!(default_channel("0.3.0"), UpdateChannel::Stable);
        assert_eq!(default_channel("not a version"), UpdateChannel::Stable);
    }

    #[test]
    fn topics_are_the_ones_introduced_after_the_old_version() {
        use WhatsNewTopic::*;
        let all = [
            UpdateCheck,
            CourseWeeks,
            CourseRemoval,
            SyllabusReading,
            AiWriting,
            Reminders,
        ];
        assert_eq!(topics_since(None), all);
        assert_eq!(topics_since(Some("0.1.0")), all);
        // Pre-releases order as semver does: alpha.1 < alpha.2 < alpha.3 < beta.1 < 0.3.0.
        assert_eq!(topics_since(Some("0.3.0-alpha.1")), all[2..]);
        assert_eq!(topics_since(Some("0.3.0-alpha.2")), all[3..]);
        assert_eq!(topics_since(Some("0.3.0-alpha.3")), [AiWriting, Reminders]);
        assert!(topics_since(Some("0.3.0-beta.1")).is_empty());
        assert!(topics_since(Some("0.3.0")).is_empty());
    }

    /// Every topic has a row (the version that introduced it) and desktop copy in both
    /// languages: a build never announces a topic without words, nor forgets one.
    #[test]
    fn every_topic_has_a_row_and_desktop_copy() {
        fn names(value: &serde_json::Value, out: &mut Vec<String>) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, value) in map {
                        match (key.as_str(), value) {
                            ("const", serde_json::Value::String(name)) => out.push(name.clone()),
                            ("enum", serde_json::Value::Array(list)) => out
                                .extend(list.iter().filter_map(|v| v.as_str().map(str::to_string))),
                            _ => names(value, out),
                        }
                    }
                }
                serde_json::Value::Array(items) => items.iter().for_each(|item| names(item, out)),
                _ => {}
            }
        }
        let schema = serde_json::to_value(schemars::schema_for!(WhatsNewTopic)).unwrap();
        let mut topics = Vec::new();
        names(&schema, &mut topics);
        topics.sort();
        topics.dedup();
        assert!(topics.len() >= 3, "{schema}");
        let mut rows: Vec<String> = WHATS_NEW
            .iter()
            .map(|(topic, _)| {
                serde_json::to_value(topic)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        rows.sort();
        assert_eq!(rows, topics, "one WHATS_NEW row per topic");
        for language in ["en", "zh-CN"] {
            let path = format!(
                "{}/../../apps/desktop/src/i18n/locales/{language}/updates.json",
                env!("CARGO_MANIFEST_DIR")
            );
            let copy: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            for topic in &topics {
                for field in ["title", "body"] {
                    let text = copy["whatsNew"]["topics"][topic][field].as_str();
                    assert!(
                        text.is_some_and(|text| !text.trim().is_empty()),
                        "{language}: whatsNew.topics.{topic}.{field}"
                    );
                }
            }
        }
    }
}
