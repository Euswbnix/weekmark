//! Course dates, phases and lifecycle: the facade half (docs/design/v0.3-course-calendar.md §4,
//! §8). The pure computation lives in `pagelamp_core::term` / `lifecycle`; this module holds the
//! writes and every default a surface would otherwise have to compute ("I'm still taking this"
//! until when, how long "Not now" lasts), so Tauri, Swift and the CLI only render (plan §1.3).
//!
//! Settings keys (schema 3 `settings` table):
//!
//! | key                               | value                                             |
//! |-----------------------------------|---------------------------------------------------|
//! | `course_dates.confirmed`          | ids of courses whose student dates are confirmed  |
//! | `lifecycle.banner_snoozed`        | `BannerSnooze`: until when, for which courses     |

pub(crate) mod calendar;
pub(crate) mod dates;
pub(crate) mod removal;

use std::collections::BTreeSet;

use chrono::NaiveDate;
use pagelamp_core::dates::add_days;
use pagelamp_core::model::{Course, CourseLifecycle, CourseTimeline, SnoozeKind};
use pagelamp_core::store::Store;
use pagelamp_core::term::CONFIRMED_DATES_KEY;
use pagelamp_core::views::{self, AsOf};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{App, AppError, AppErrorKind, Result};

/// "Not now" on a removal suggestion or the banner lasts this many days.
pub const NOT_NOW_DAYS: i64 = 14;
/// "I'm still taking this" without an outer frame lasts this many days.
pub const KEEP_CURRENT_DAYS: i64 = 120;
/// `removal_snoozed_until` for "Keep" (9999-12-31): never suggest again.
pub fn keep_forever() -> NaiveDate {
    NaiveDate::from_ymd_opt(9999, 12, 31).expect("9999-12-31 is a date")
}

const BANNER_KEY: &str = "lifecycle.banner_snoozed";
/// "Not now" on the syllabus reading offers (`snooze_calendar_offers`).
const OFFERS_KEY: &str = "calendar.offers_snoozed";

/// Every course's lifecycle, and whether the Courses page shows "N courses look finished".
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct LifecycleSummary {
    /// Every course, hidden ones included (hidden past courses are most of the backlog).
    pub courses: Vec<CourseLifecycleEntry>,
    /// Ids of the courses suggested for removal (`lifecycle.suggest_removal`).
    pub suggested: Vec<String>,
    /// True when some suggested course is not covered by a snoozed banner.
    pub show_banner: bool,
    /// Until when "Not now" on the banner holds, if it was pressed and hasn't expired.
    pub banner_snoozed_until: Option<NaiveDate>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CourseLifecycleEntry {
    pub course_id: String,
    pub code: Option<String>,
    pub name: String,
    pub hidden: bool,
    pub lifecycle: CourseLifecycle,
}

/// The banner's "Not now": until `until`, for the courses suggested when it was pressed.
/// (The syllabus reading offers use the same shape.)
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct BannerSnooze {
    until: Option<NaiveDate>,
    courses: Vec<String>,
}

impl App {
    /// Where one course is: week, phase, the dates used and not used, evidence. Hidden courses
    /// are addressable, as everywhere in the facade.
    pub fn course_timeline(&self, course: &str) -> Result<CourseTimeline> {
        let store = self.read_store()?;
        let course = store.resolve_course_with(course, true)?;
        Ok(views::course_timeline(&store, &course, AsOf::now_local())?)
    }

    /// Every course's lifecycle, the removal suggestions and the banner state.
    pub fn lifecycle_summary(&self) -> Result<LifecycleSummary> {
        let store = self.read_store()?;
        lifecycle_summary(&store, AsOf::now_local())
    }

    /// "I'm still taking this": the course counts as current until `until`, by default the end
    /// of its outer frame (the LMS term ∪ the session window) when that is still ahead, else
    /// today + 120 days.
    pub fn keep_course_current(&self, course: &str, until: Option<NaiveDate>) -> Result<Course> {
        let store = self.write_store()?;
        let at = AsOf::now_local();
        let course = store.resolve_course_with(course, true)?;
        let until = match until {
            Some(until) if until < at.today => {
                return Err(AppError::new(
                    AppErrorKind::Invalid,
                    format!("{until} is in the past: choose today or a later day"),
                ));
            }
            Some(until) => until,
            None => {
                let timeline = views::course_timeline(&store, &course, at)?;
                default_keep_until(&timeline, at.today)
            }
        };
        store.set_keep_current_until(&course.id, Some(until))?;
        Ok(course)
    }

    /// Undo "I'm still taking this".
    pub fn clear_keep_course_current(&self, course: &str) -> Result<Course> {
        let store = self.write_store()?;
        let course = store.resolve_course_with(course, true)?;
        store.set_keep_current_until(&course.id, None)?;
        Ok(course)
    }

    /// "Not now" (14 days) or "Keep" (never again) on the removal suggestion of `courses`.
    pub fn snooze_removal_suggestions(&self, courses: Vec<String>, kind: SnoozeKind) -> Result<()> {
        let store = self.write_store()?;
        let today = AsOf::now_local().today;
        let until = match kind {
            SnoozeKind::NotNow => add_days(today, NOT_NOW_DAYS),
            SnoozeKind::Keep => keep_forever(),
        };
        store.in_transaction(|store| {
            for course in &courses {
                let course = store.resolve_course_with(course, true)?;
                store.set_removal_snoozed_until(&course.id, Some(until))?;
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Undo "Not now" / "Keep": the courses may be suggested again.
    pub fn clear_removal_snooze(&self, courses: Vec<String>) -> Result<()> {
        let store = self.write_store()?;
        store.in_transaction(|store| {
            for course in &courses {
                let course = store.resolve_course_with(course, true)?;
                store.set_removal_snoozed_until(&course.id, None)?;
            }
            Ok(())
        })?;
        Ok(())
    }

    /// "Not now" on the banner: hidden for 14 days, until another course becomes a suggestion.
    /// "Not now" on the syllabus reading offers (`syllabus_reading_offers`, which
    /// `startup_tasks().calendar_offers` lists): silent for 14 days for the courses offered
    /// now; a course offered later shows them again.
    pub fn snooze_calendar_offers(&self) -> Result<()> {
        let offered: Vec<String> = self
            .all_syllabus_reading_offers()?
            .into_iter()
            .map(|offer| offer.course_id)
            .collect();
        let today = AsOf::now_local().today;
        self.write_store()?.set_setting(
            OFFERS_KEY,
            &BannerSnooze {
                until: Some(add_days(today, NOT_NOW_DAYS)),
                courses: offered,
            },
        )?;
        Ok(())
    }

    /// The offers to show now: every current offer when one of them isn't covered by an
    /// unexpired "Not now" (like the lifecycle banner), else none.
    pub(crate) fn unsnoozed_calendar_offers(&self) -> Result<Vec<calendar::SyllabusOffer>> {
        let offers = self.all_syllabus_reading_offers()?;
        let snooze: BannerSnooze = self
            .read_store()?
            .setting_or_absent(OFFERS_KEY)?
            .unwrap_or_default();
        let today = AsOf::now_local().today;
        let quiet = snooze.until.is_some_and(|until| until >= today)
            && offers
                .iter()
                .all(|offer| snooze.courses.contains(&offer.course_id));
        Ok(if quiet { Vec::new() } else { offers })
    }

    pub fn snooze_lifecycle_banner(&self) -> Result<()> {
        let store = self.write_store()?;
        let at = AsOf::now_local();
        let summary = lifecycle_summary(&store, at)?;
        store.set_setting(
            BANNER_KEY,
            &BannerSnooze {
                until: Some(add_days(at.today, NOT_NOW_DAYS)),
                courses: summary.suggested,
            },
        )?;
        Ok(())
    }

    /// "These dates are right": the course's student dates (set in PageLamp 0.1) stop being
    /// `legacy`. Saving or clearing dates with `set_course_term` confirms them too.
    pub fn confirm_course_dates(&self, course: &str) -> Result<CourseTimeline> {
        let store = self.write_store()?;
        let course = store.resolve_course_with(course, true)?;
        store.in_transaction(|store| set_dates_confirmed(store, &course.id, true))?;
        Ok(views::course_timeline(&store, &course, AsOf::now_local())?)
    }
}

/// Add (`true`) or remove the course from the confirmed-dates list. A failed read fails the
/// call before anything is written, so the other courses' confirmations are never lost; a
/// value that doesn't parse counts as an empty list.
pub(crate) fn set_dates_confirmed(
    store: &Store,
    course_id: &str,
    confirmed: bool,
) -> pagelamp_core::Result<()> {
    let mut ids: BTreeSet<String> = store
        .setting_or_absent::<BTreeSet<String>>(CONFIRMED_DATES_KEY)?
        .unwrap_or_default();
    let changed = if confirmed {
        ids.insert(course_id.to_string())
    } else {
        ids.remove(course_id)
    };
    if changed {
        store.set_setting(CONFIRMED_DATES_KEY, &ids)?;
    }
    Ok(())
}

fn default_keep_until(timeline: &CourseTimeline, today: NaiveDate) -> NaiveDate {
    timeline
        .term
        .outer_frame
        .map(|frame| frame.end)
        .filter(|end| *end > today)
        .unwrap_or_else(|| add_days(today, KEEP_CURRENT_DAYS))
}

fn lifecycle_summary(store: &Store, at: AsOf) -> Result<LifecycleSummary> {
    let courses: Vec<CourseLifecycleEntry> = views::list_courses(store, true, at)?
        .into_iter()
        .map(|summary| CourseLifecycleEntry {
            course_id: summary.course.id,
            code: summary.course.code,
            name: summary.course.name,
            hidden: summary.course.hidden,
            lifecycle: summary.lifecycle,
        })
        .collect();
    let suggested: Vec<String> = courses
        .iter()
        .filter(|entry| entry.lifecycle.suggest_removal)
        .map(|entry| entry.course_id.clone())
        .collect();
    let snooze: BannerSnooze = store
        .setting_or_absent::<BannerSnooze>(BANNER_KEY)?
        .unwrap_or_default();
    let banner_snoozed_until = snooze.until.filter(|until| *until >= at.today);
    let show_banner = match banner_snoozed_until {
        Some(_) => suggested.iter().any(|id| !snooze.courses.contains(id)),
        None => !suggested.is_empty(),
    };
    Ok(LifecycleSummary {
        courses,
        suggested,
        show_banner,
        banner_snoozed_until,
    })
}
