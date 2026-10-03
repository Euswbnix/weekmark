//! Reminders and the weekly digest (model-access design §3.8, §5.3). No model. The logic lives
//! here, so the Tauri app, the CLI and the Swift shell only render:
//!
//! - `due_reminders(now)`: what to show now (catch-up after a time the app wasn't running, a
//!   maximum age per kind, dedupe against `reminders_shown`); the Tauri timer and the CLI;
//! - `reminders(from, to)`: what fires in a window, for Swift's calendar triggers (each with its
//!   wall-clock time and IANA time zone);
//! - `mark_reminders_shown` after a UI showed some;
//! - `reminder_settings` / `set_reminder_settings`;
//! - `weekly_digest`: the digest itself;
//! - `startup_tasks` (updates.rs) carries the due reminders at launch.
//!
//! Wall-clock times are in the computer's time zone (`AsOf::now_local`), so a "Monday 09:00"
//! digest stays at 09:00 local across a DST change and follows the student when they travel.

use std::collections::HashSet;

use chrono::Duration;
use pagelamp_core::dates::Tz;
use pagelamp_core::model::Timestamp;
use pagelamp_core::reminders::{self, CATCH_UP_DAYS, REMINDER_ID_PREFIXES, ReminderInputs};
pub use pagelamp_core::reminders::{DayOfWeek, Reminder, ReminderKind, ReminderSettings};
use pagelamp_core::store::Store;
use pagelamp_core::views::{self, AsOf, WeeklyDigest};

use crate::updates::Shell;
use crate::{App, AppError, AppErrorKind, Result};

/// The settings key of `ReminderSettings`.
const SETTINGS_KEY: &str = "reminder_settings";
/// The Mac app's own `run_in_background` (its "Remind me" consent), apart from the desktop
/// app's, which lives in `SETTINGS_KEY` (like What's new's `app.mac.*` keys). Absent: false.
const MAC_RUN_IN_BACKGROUND_KEY: &str = "app.mac.run_in_background";
/// The longest window `reminders(from, to)` accepts.
const MAX_WINDOW_DAYS: i64 = 62;

impl App {
    /// The weekly digest now (visible, not removed courses; model-access design §5.3).
    pub fn weekly_digest(&self) -> Result<WeeklyDigest> {
        let store = self.read_store()?;
        Ok(views::weekly_digest(
            &store,
            self.local_as_of(chrono::Utc::now()),
        )?)
    }

    /// The student's reminder settings (defaults when never set or unparseable). A failed read
    /// is an error, never the defaults: the shells keep the login item and the notifications
    /// as they are until the stored answer can be read. `run_in_background` is this shell's
    /// own (`Shell::Mac`: the Mac app's consent, false until given); the rest is shared.
    pub fn reminder_settings(&self) -> Result<ReminderSettings> {
        let store = self.read_store()?;
        let mut settings = settings_in(&store)?;
        if self.shell() == Shell::Mac {
            settings.run_in_background = store
                .setting_or_absent(MAC_RUN_IN_BACKGROUND_KEY)?
                .unwrap_or(false);
        }
        Ok(settings)
    }

    /// `Invalid` when a time isn't "HH:MM".
    pub fn set_reminder_settings(&self, settings: &ReminderSettings) -> Result<()> {
        settings.validate().map_err(|field| {
            AppError::new(
                AppErrorKind::Invalid,
                format!("{field} must be a time like 09:00."),
            )
        })?;
        let store = self.write_store()?;
        if self.shell() == Shell::Desktop {
            return Ok(store.set_setting(SETTINGS_KEY, settings)?);
        }
        // The Mac app: the shared fields, and its own consent; the desktop app's answer stays.
        Ok(store.in_transaction(|store| {
            let desktop = store
                .setting_or_absent::<ReminderSettings>(SETTINGS_KEY)?
                .unwrap_or_default()
                .run_in_background;
            store.set_setting(
                SETTINGS_KEY,
                &ReminderSettings {
                    run_in_background: desktop,
                    ..settings.clone()
                },
            )?;
            store.set_setting(MAC_RUN_IN_BACKGROUND_KEY, &settings.run_in_background)
        })?)
    }

    /// The reminders that fire in [`from`, `to`) and weren't shown yet, soonest first (Swift
    /// schedules them ahead). `Invalid` for a window that ends before it starts or is longer
    /// than 62 days.
    pub fn reminders(&self, from: Timestamp, to: Timestamp) -> Result<Vec<Reminder>> {
        if to < from || to - from > Duration::days(MAX_WINDOW_DAYS) {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                "Ask for reminders over a window of at most 62 days.",
            ));
        }
        let store = self.read_store()?;
        let shown = store.shown_reminders()?;
        Ok(schedule(&store, self.zone(), from, to)?
            .into_iter()
            .filter(|r| !shown.contains(&r.id))
            .collect())
    }

    /// What to show at `now` (the caller's clock): reminders that fired and weren't shown, not
    /// older than their kind allows (a digest 3 days, today's plan until midnight, a deadline
    /// until it is due, and only its latest reminder). Show them, then `mark_reminders_shown`.
    pub fn due_reminders(&self, now: Timestamp) -> Result<Vec<Reminder>> {
        let store = self.read_store()?;
        let tz = self.zone();
        let from = now - Duration::days(CATCH_UP_DAYS);
        let scheduled = schedule(&store, tz, from, now + Duration::seconds(1))?;
        let shown: HashSet<String> = store.shown_reminders()?;
        Ok(reminders::due(scheduled, now, tz, &shown))
    }

    /// The UI showed these reminders: they aren't due again. `Invalid` for an id that isn't a
    /// reminder's (nothing is recorded then).
    pub fn mark_reminders_shown(&self, ids: &[String]) -> Result<()> {
        if let Some(id) = ids
            .iter()
            .find(|id| !REMINDER_ID_PREFIXES.iter().any(|p| id.starts_with(p)))
        {
            return Err(AppError::new(
                AppErrorKind::Invalid,
                format!("'{id}' is not a reminder id."),
            ));
        }
        Ok(self
            .write_store()?
            .mark_reminders_shown(ids, chrono::Utc::now())?)
    }

    /// Use `zone` (an IANA name) for reminders and the digest instead of the computer's time
    /// zone; `None` goes back to the computer's. For tests and embedders that know better.
    /// `Invalid` for an unknown zone.
    #[doc(hidden)]
    pub fn set_time_zone(&self, zone: Option<&str>) -> Result<()> {
        let tz = zone
            .map(|name| {
                name.parse::<Tz>().map_err(|_| {
                    AppError::new(AppErrorKind::Invalid, format!("unknown time zone {name}"))
                })
            })
            .transpose()?;
        *self
            .state
            .zone
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = tz;
        Ok(())
    }

    /// The zone reminders use: `set_time_zone`'s, else the computer's, else UTC.
    fn zone(&self) -> Tz {
        let chosen = *self
            .state
            .zone
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        chosen.or_else(|| AsOf::now_local().tz).unwrap_or(Tz::UTC)
    }

    /// `now` with the student's date in the reminder zone.
    pub(crate) fn local_as_of(&self, now: Timestamp) -> AsOf {
        let tz = self.zone();
        AsOf {
            now,
            today: now.with_timezone(&tz).date_naive(),
            tz: Some(tz),
        }
    }
}

fn settings_in(store: &Store) -> Result<ReminderSettings> {
    Ok(store.setting_or_absent(SETTINGS_KEY)?.unwrap_or_default())
}

/// Every reminder that fires in [`from`, `to`) (shown or not).
fn schedule(store: &Store, tz: Tz, from: Timestamp, to: Timestamp) -> Result<Vec<Reminder>> {
    let settings = settings_in(store)?;
    // A deadline's reminders fire 48 h to 24 h before it; a digest counts the next 7 days.
    let deadlines = views::deadlines_between(store, from, to + Duration::days(7))?;
    // Hidden courses' items don't count, as in the digest.
    let plan = store.latest_visible_study_plan()?;
    let has_courses = !store.list_courses(false)?.is_empty();
    Ok(reminders::schedule(
        &settings,
        tz,
        from,
        to,
        &ReminderInputs {
            deadlines: &deadlines,
            plan: plan.as_ref().map(|stored| &stored.plan),
            has_courses,
        },
    ))
}
