import type { TFunction } from "i18next";
import type { NotificationText, Reminder } from "@/api/reminders";

/**
 * One due reminder as a notification in the student's language (design §5.3): course codes and
 * titles only, never material text. Times are shown in the reminder's own time zone.
 */
export function reminderText(
  reminder: Reminder,
  t: TFunction<"reminders">,
  locale: string,
): NotificationText {
  const { id } = reminder;
  switch (reminder.kind) {
    case "deadline_soon": {
      const title = reminder.title ?? "";
      const code = reminder.course_code ?? reminder.course_name ?? null;
      return {
        id,
        title: code ? t("notify.deadlineTitle", { code, title }) : title,
        body: reminder.due_at
          ? t("notify.deadlineBody", {
              when: when(reminder.due_at, reminder.time_zone, locale, t),
            })
          : "",
      };
    }
    case "weekly_digest": {
      const count = reminder.count ?? 0;
      return {
        id,
        title: t("notify.digestTitle"),
        body: count > 0 ? t("notify.digestBody", { count }) : t("notify.digestBodyNone"),
      };
    }
    case "plan_today":
      return {
        id,
        title: t("notify.planTitle"),
        body: t("notify.planBody", { count: reminder.count ?? 0 }),
      };
  }
}

/**
 * "Tuesday 23:59" in the student's language, in `timeZone` when the system knows it. The day and
 * the time are formatted apart: CLDR's zh pattern runs them together ("星期二23:59").
 */
function when(at: string, timeZone: string, locale: string, t: TFunction<"reminders">): string {
  const format = (options: Intl.DateTimeFormatOptions) => {
    try {
      return new Intl.DateTimeFormat(locale, { ...options, timeZone }).format(new Date(at));
    } catch {
      // An IANA name this system doesn't know: its own zone is the best guess left.
      return new Intl.DateTimeFormat(locale, options).format(new Date(at));
    }
  };
  return t("notify.when", {
    weekday: format({ weekday: "long" }),
    time: format({ hour: "2-digit", minute: "2-digit" }),
  });
}
