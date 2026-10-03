// Reminders in the mock (M3; design §5.3): settings with the facade's defaults and "HH:MM"
// check, what is due in the reminders-due scenario (each shown once, like reminders_shown), and
// the tray and login item that follow run_in_background.

import type { PageLampApi } from "../client";
import { ApiError } from "../errors";
import type { BackgroundStatus, Reminder, ReminderSettings } from "../reminders";
import type { MockCourse, MockScenario } from "./fixtures";

type RemindersApi = Pick<
  PageLampApi,
  | "reminderSettings"
  | "setReminderSettings"
  | "backgroundStatus"
  | "setTrayLabels"
  | "dueReminders"
  | "showReminders"
  | "markRemindersShown"
  | "openNotificationSettings"
  | "showRemindersOnNotice"
  | "onReminderCheck"
>;

/** The facade's defaults (pagelamp-core ReminderSettings::default). */
export const DEFAULT_REMINDER_SETTINGS: ReminderSettings = {
  deadline_soon: true,
  weekly_digest: true,
  digest_day: "monday",
  digest_time: "09:00",
  plan_today: false,
  plan_today_time: "08:00",
  run_in_background: false,
};

/** A page event that stands in for the shell's "reminders:check" (tests and the demo). */
export const MOCK_REMINDER_CHECK_EVENT = "pagelamp:reminders-check";

const HH_MM = /^([01]\d|2[0-3]):[0-5]\d$/;

export function createRemindersMock(deps: {
  scenario: MockScenario;
  now: () => Date;
  respond: <T>(value: T | (() => T), extraLatency?: number) => Promise<T>;
  courses: () => MockCourse[];
}): RemindersApi & { dueNow: () => Reminder[] } {
  const { scenario, now, respond } = deps;
  const noTray = scenario === "reminders-no-tray";
  let settings: ReminderSettings = {
    ...DEFAULT_REMINDER_SETTINGS,
    run_in_background: noTray,
  };
  const shown = new Set<string>();

  function background(): BackgroundStatus {
    const on = settings.run_in_background;
    return {
      run_in_background: on,
      tray: on && !noTray,
      tray_unavailable: on && noTray,
      login_item: on,
    };
  }

  function due(): Reminder[] {
    if (scenario !== "reminders-due") return [];
    const at = now();
    const course = deps.courses().find((c) => !c.course.hidden)?.course;
    const fire = (minutesAgo: number) => new Date(at.getTime() - minutesAgo * 60_000);
    const local = (d: Date) => {
      const pad = (n: number) => String(n).padStart(2, "0");
      return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
    };
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
    const deadlineFire = fire(10);
    const all: Reminder[] = [
      {
        id: "deadline_soon:demo-ps3:24",
        kind: "deadline_soon",
        local_time: local(deadlineFire),
        time_zone: zone,
        fire_at: deadlineFire.toISOString(),
        title: "Problem set 3",
        course_id: course?.id ?? null,
        course_code: course?.code ?? null,
        course_name: course?.name ?? null,
        due_at: new Date(deadlineFire.getTime() + 24 * 3_600_000).toISOString(),
        hours_before: 24,
        count: null,
      },
      {
        id: "weekly_digest:2026-W40",
        kind: "weekly_digest",
        local_time: local(fire(90)),
        time_zone: zone,
        fire_at: fire(90).toISOString(),
        count: 2,
      },
    ];
    if (settings.plan_today) {
      all.push({
        id: "plan_today:2026-09-29",
        kind: "plan_today",
        local_time: local(fire(30)),
        time_zone: zone,
        fire_at: fire(30).toISOString(),
        count: 3,
      });
    }
    return all
      .filter((r) => !shown.has(r.id))
      .filter((r) =>
        r.kind === "deadline_soon"
          ? settings.deadline_soon
          : r.kind === "weekly_digest"
            ? settings.weekly_digest
            : settings.plan_today,
      );
  }

  return {
    reminderSettings: () => respond(() => ({ ...settings })),
    setReminderSettings: (next) =>
      respond(() => {
        if (!HH_MM.test(next.digest_time) || !HH_MM.test(next.plan_today_time)) {
          throw new ApiError("invalid", "A reminder time must be HH:MM.");
        }
        settings = { ...next };
        return background();
      }),
    backgroundStatus: () => respond(background),
    setTrayLabels: async () => {},
    dueReminders: () => respond(due),
    showReminders: (notifications) =>
      respond(() => {
        for (const n of notifications) shown.add(n.id);
      }),
    markRemindersShown: (ids) =>
      respond(() => {
        for (const id of ids) shown.add(id);
      }),
    showRemindersOnNotice: async () => {},
    openNotificationSettings: () => respond(true),
    dueNow: due,
    onReminderCheck: (onCheck) => {
      const handler = () => onCheck();
      window.addEventListener(MOCK_REMINDER_CHECK_EVENT, handler);
      return () => window.removeEventListener(MOCK_REMINDER_CHECK_EVENT, handler);
    },
  };
}
