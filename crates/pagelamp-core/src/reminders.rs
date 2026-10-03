//! Weekly progress reminders (model-access design §5.3): deterministic, no model.
//!
//! - Three kinds: `deadline_soon` (48 h and 24 h before a deadline of a visible, not removed
//!   course, whatever its lifecycle), `weekly_digest` (default Monday 09:00) and `plan_today`
//!   (off by default).
//! - Each reminder carries its local wall-clock time and IANA time zone as well as the instant
//!   it fires, so "Monday 09:00" stays 09:00 local across a DST change (Swift schedules
//!   calendar triggers from the wall-clock time).
//! - Their text is titles and codes only, never material text.
//! - `schedule` lists the reminders of a time window; `due` picks what to show now: catch-up
//!   after a time the app wasn't running, a maximum age per kind, an earlier deadline reminder
//!   replaced by a later one, and nothing already shown.

use std::collections::{BTreeMap, HashSet};

use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc, Weekday,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dates::Tz;
use crate::model::{EventKind, StudyPlan, Timestamp};
use crate::views::Deadline;

/// Hours before a deadline its reminders fire, earliest first.
pub const DEADLINE_HOURS: [u32; 2] = [48, 24];
/// A weekly digest not shown within this long is dropped.
pub const DIGEST_MAX_AGE_DAYS: i64 = 3;
/// How far back `due` looks: the longest maximum age of any kind.
pub const CATCH_UP_DAYS: i64 = DIGEST_MAX_AGE_DAYS;
/// The id prefixes of reminders (`reminders_shown` also holds other one-time notices).
pub const REMINDER_ID_PREFIXES: [&str; 3] = ["deadline_soon:", "weekly_digest:", "plan_today:"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReminderKind {
    /// 48 h and 24 h before a deadline.
    DeadlineSoon,
    /// The weekly digest (`weekly_digest`).
    WeeklyDigest,
    /// Today's open study plan items.
    PlanToday,
}

/// A day of the week (the weekly digest's day, a study plan's days off).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DayOfWeek {
    #[default]
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl DayOfWeek {
    pub fn weekday(self) -> Weekday {
        match self {
            DayOfWeek::Monday => Weekday::Mon,
            DayOfWeek::Tuesday => Weekday::Tue,
            DayOfWeek::Wednesday => Weekday::Wed,
            DayOfWeek::Thursday => Weekday::Thu,
            DayOfWeek::Friday => Weekday::Fri,
            DayOfWeek::Saturday => Weekday::Sat,
            DayOfWeek::Sunday => Weekday::Sun,
        }
    }
}

/// Which reminders the student wants, and when (Settings → Reminders). Times are local
/// wall-clock times, "HH:MM" (24-hour); a kind turned off keeps its time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReminderSettings {
    pub deadline_soon: bool,
    pub weekly_digest: bool,
    pub digest_day: DayOfWeek,
    pub digest_time: String,
    pub plan_today: bool,
    pub plan_today_time: String,
    /// The onboarding answer "Remind me", per app shell, the only field that isn't shared. The
    /// desktop app keeps its tray and login item in line with it. The Mac app treats it as its
    /// consent to schedule notifications, stored apart so that neither app's answer changes
    /// the other's. Reminders are computed the same either way.
    pub run_in_background: bool,
}

impl Default for ReminderSettings {
    fn default() -> Self {
        ReminderSettings {
            deadline_soon: true,
            weekly_digest: true,
            digest_day: DayOfWeek::Monday,
            digest_time: "09:00".to_string(),
            plan_today: false,
            plan_today_time: "08:00".to_string(),
            run_in_background: false,
        }
    }
}

impl ReminderSettings {
    /// `Err` with the field's name when a time isn't "HH:MM".
    pub fn validate(&self) -> Result<(), &'static str> {
        if parse_time(&self.digest_time).is_none() {
            return Err("digest_time");
        }
        if parse_time(&self.plan_today_time).is_none() {
            return Err("plan_today_time");
        }
        Ok(())
    }
}

/// "HH:MM" (24-hour, two digits each).
fn parse_time(text: &str) -> Option<NaiveTime> {
    let (hours, minutes) = text.split_once(':')?;
    if hours.len() != 2 || minutes.len() != 2 {
        return None;
    }
    NaiveTime::from_hms_opt(hours.parse().ok()?, minutes.parse().ok()?, 0)
}

/// One reminder. A UI builds its text from `kind` and these fields (translated), never from
/// material text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Reminder {
    /// Stable: the same reminder has the same id in every call (`mark_reminders_shown`).
    pub id: String,
    pub kind: ReminderKind,
    /// The wall-clock time it fires at in `time_zone`, "YYYY-MM-DDTHH:MM".
    pub local_time: String,
    /// IANA time zone of `local_time` (the computer's zone; "UTC" if it is unknown).
    pub time_zone: String,
    pub fire_at: Timestamp,
    /// `deadline_soon`: the deadline's title.
    pub title: Option<String>,
    /// `deadline_soon`: its course (none for a feed event not linked to a course).
    pub course_id: Option<String>,
    pub course_code: Option<String>,
    pub course_name: Option<String>,
    /// `deadline_soon`: when it is due, and how many hours before that this fires.
    pub due_at: Option<Timestamp>,
    pub hours_before: Option<u32>,
    /// `weekly_digest`: the deadlines of the 7 days from `fire_at`; `plan_today`: the plan's
    /// open items that day.
    pub count: Option<u32>,
}

/// What the reminders are built from.
pub struct ReminderInputs<'a> {
    /// Deadlines due within [from, to + 7 days] (`views::deadlines_between`): those 48 h and
    /// 24 h ahead, and the digest's week.
    pub deadlines: &'a [Deadline],
    /// The latest study plan, if any.
    pub plan: Option<&'a StudyPlan>,
    /// Whether there is any visible course (no digest without one).
    pub has_courses: bool,
}

/// Event kinds that are deadlines (not lectures or other calendar entries).
fn is_deadline(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::AssignmentDue | EventKind::QuizDue | EventKind::Exam | EventKind::PlannerItem
    )
}

/// The reminders that fire in [`from`, `to`), soonest first.
pub fn schedule(
    settings: &ReminderSettings,
    tz: Tz,
    from: Timestamp,
    to: Timestamp,
    inputs: &ReminderInputs<'_>,
) -> Vec<Reminder> {
    let mut out = Vec::new();
    let in_window = |t: Timestamp| from <= t && t < to;
    if settings.deadline_soon {
        for deadline in inputs.deadlines {
            let event = &deadline.event;
            let Some(due) = event.when().filter(|_| is_deadline(event.kind)) else {
                continue;
            };
            for hours in DEADLINE_HOURS {
                let fire_at = due - Duration::hours(i64::from(hours));
                if !in_window(fire_at) {
                    continue;
                }
                out.push(Reminder {
                    id: format!("deadline_soon:{hours}h:{}@{}", event.id, due.timestamp()),
                    kind: ReminderKind::DeadlineSoon,
                    local_time: local_text(fire_at, tz),
                    time_zone: tz.name().to_string(),
                    fire_at,
                    title: Some(event.title.clone()),
                    course_id: event.course_id.clone(),
                    course_code: deadline.course_code.clone(),
                    course_name: deadline.course_name.clone(),
                    due_at: Some(due),
                    hours_before: Some(hours),
                    count: None,
                });
            }
        }
    }
    let digest_time = parse_time(&settings.digest_time);
    let plan_time = parse_time(&settings.plan_today_time);
    let open_items: BTreeMap<NaiveDate, u32> = inputs
        .plan
        .map(|plan| {
            plan.items
                .iter()
                .filter(|item| !item.done)
                .fold(BTreeMap::new(), |mut days, item| {
                    *days.entry(item.date).or_default() += 1;
                    days
                })
        })
        .unwrap_or_default();
    // Every local date the window touches (a day either side for the zone's offset).
    let first = from.with_timezone(&tz).date_naive() - Duration::days(1);
    let last = to.with_timezone(&tz).date_naive() + Duration::days(1);
    for date in first.iter_days().take_while(|date| *date <= last) {
        if settings.weekly_digest
            && inputs.has_courses
            && date.weekday() == settings.digest_day.weekday()
            && let Some(fire_at) = digest_time.and_then(|time| at_local(tz, date, time))
            && in_window(fire_at)
        {
            let week_end = fire_at + Duration::days(7);
            let due_this_week = inputs
                .deadlines
                .iter()
                .filter(|d| is_deadline(d.event.kind))
                .filter_map(|d| d.event.when())
                .filter(|due| fire_at <= *due && *due < week_end)
                .count();
            let count = u32::try_from(due_this_week).unwrap_or(u32::MAX);
            out.push(simple(
                ReminderKind::WeeklyDigest,
                date,
                fire_at,
                tz,
                Some(count),
            ));
        }
        if settings.plan_today
            && let Some(&count) = open_items.get(&date)
            && let Some(fire_at) = plan_time.and_then(|time| at_local(tz, date, time))
            && in_window(fire_at)
        {
            out.push(simple(
                ReminderKind::PlanToday,
                date,
                fire_at,
                tz,
                Some(count),
            ));
        }
    }
    out.sort_by(|a, b| a.fire_at.cmp(&b.fire_at).then_with(|| a.id.cmp(&b.id)));
    out
}

/// What to show at `now` from `scheduled` (see the module docs): fired, not older than its
/// kind's maximum age, not replaced by a later reminder of the same deadline, not in `shown`.
pub fn due(
    scheduled: Vec<Reminder>,
    now: Timestamp,
    tz: Tz,
    shown: &HashSet<String>,
) -> Vec<Reminder> {
    let fired: Vec<Reminder> = scheduled.into_iter().filter(|r| r.fire_at <= now).collect();
    // The latest reminder that fired for each deadline (its event at its due time).
    let mut latest: BTreeMap<&str, Timestamp> = BTreeMap::new();
    for reminder in fired
        .iter()
        .filter(|r| r.kind == ReminderKind::DeadlineSoon)
    {
        let entry = latest
            .entry(deadline_key(&reminder.id))
            .or_insert(reminder.fire_at);
        *entry = (*entry).max(reminder.fire_at);
    }
    let latest: BTreeMap<String, Timestamp> = latest
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    fired
        .into_iter()
        .filter(|r| !shown.contains(&r.id))
        .filter(|r| match r.kind {
            ReminderKind::DeadlineSoon => {
                r.due_at.is_some_and(|due| now < due)
                    && latest.get(deadline_key(&r.id)) == Some(&r.fire_at)
            }
            ReminderKind::WeeklyDigest => now - r.fire_at < Duration::days(DIGEST_MAX_AGE_DAYS),
            // Until the end of that day.
            ReminderKind::PlanToday => {
                r.fire_at.with_timezone(&tz).date_naive() == now.with_timezone(&tz).date_naive()
            }
        })
        .collect()
}

/// The deadline a `deadline_soon` id is about: "<event id>@<due, Unix seconds>" (the part after
/// "deadline_soon:<hours>h:"), the same for its 48 h and 24 h reminders.
fn deadline_key(id: &str) -> &str {
    id.split_once("h:").map_or(id, |(_, rest)| rest)
}

fn simple(
    kind: ReminderKind,
    date: NaiveDate,
    fire_at: Timestamp,
    tz: Tz,
    count: Option<u32>,
) -> Reminder {
    let name = match kind {
        ReminderKind::WeeklyDigest => "weekly_digest",
        ReminderKind::PlanToday => "plan_today",
        ReminderKind::DeadlineSoon => "deadline_soon",
    };
    Reminder {
        id: format!("{name}:{date}"),
        kind,
        local_time: local_text(fire_at, tz),
        time_zone: tz.name().to_string(),
        fire_at,
        title: None,
        course_id: None,
        course_code: None,
        course_name: None,
        due_at: None,
        hours_before: None,
        count,
    }
}

/// `date` at wall-clock `time` in `tz`: the first of two times when the clock goes back, an
/// hour later when that time doesn't exist (the clock goes forward).
fn at_local(tz: Tz, date: NaiveDate, time: NaiveTime) -> Option<Timestamp> {
    let local = date.and_time(time);
    let resolved = match tz.from_local_datetime(&local) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t),
        LocalResult::None => tz
            .from_local_datetime(&(local + Duration::hours(1)))
            .earliest(),
    };
    resolved.map(|t| t.with_timezone(&Utc))
}

fn local_text(at: DateTime<Utc>, tz: Tz) -> String {
    at.with_timezone(&tz).format("%Y-%m-%dT%H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_hh_mm() {
        assert!(parse_time("09:00").is_some() && parse_time("23:59").is_some());
        for bad in ["9:00", "24:00", "09:60", "0900", "09:00:00", ""] {
            assert!(parse_time(bad).is_none(), "{bad}");
        }
        let settings = ReminderSettings {
            digest_time: "9am".into(),
            ..ReminderSettings::default()
        };
        assert_eq!(settings.validate(), Err("digest_time"));
    }

    #[test]
    fn a_deadline_s_reminders_share_its_key() {
        // Feed ids can hold "h:" and "@" themselves.
        let a = "deadline_soon:48h:ical:x/uid-h:1@example.edu@1793282400";
        let b = "deadline_soon:24h:ical:x/uid-h:1@example.edu@1793282400";
        assert_eq!(deadline_key(a), "ical:x/uid-h:1@example.edu@1793282400");
        assert_eq!(deadline_key(a), deadline_key(b));
    }

    #[test]
    fn a_missing_local_time_moves_an_hour_later() {
        let tz: Tz = "America/Toronto".parse().unwrap();
        // 2026-03-08 02:30 doesn't exist in Toronto (the clock jumps from 02:00 to 03:00).
        let date = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let at = at_local(tz, date, NaiveTime::from_hms_opt(2, 30, 0).unwrap()).unwrap();
        assert_eq!(local_text(at, tz), "2026-03-08T03:30");
    }
}
