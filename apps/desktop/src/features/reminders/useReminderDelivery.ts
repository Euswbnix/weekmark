import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useApi } from "@/api/context";
import type { NotificationText } from "@/api/reminders";
import { REMINDERS_UI } from "./availability";
import { reminderText } from "./reminderText";

/**
 * Delivers due reminders (design §5.3): once at launch (the catch-up) and whenever the shell asks
 * (every 15 minutes, after a sleep). The facade says what is due; this words it and the shell
 * hands it to the system and marks it shown (one the shell couldn't hand over comes back next
 * time; the system doesn't report whether it showed one).
 * Only after the student said "Remind me" (run_in_background): the first notification is where
 * the system asks whether PageLamp may notify, and that must follow their choice, never a launch.
 * Also keeps the tray menu in the student's language. Mount once, in the app shell.
 */
export function useReminderDelivery(enabled: boolean = REMINDERS_UI) {
  const api = useApi();
  const { t, i18n } = useTranslation("reminders");
  const locale = i18n.language;

  useEffect(() => {
    if (!enabled) return;
    api.setTrayLabels({ open: t("tray.open"), quit: t("tray.quit") }).catch(() => {});
  }, [api, enabled, t]);

  useEffect(() => {
    if (!enabled) return;
    let stopped = false;
    let running: Promise<void> | null = null;
    let again = false;

    // One delivery at a time: two at once could both see a reminder as due and show it twice.
    const deliver = () => {
      if (running) {
        again = true;
        return;
      }
      running = (async () => {
        do {
          again = false;
          try {
            if (!(await api.reminderSettings()).run_in_background) continue;
            const due = await api.dueReminders();
            const notifications: NotificationText[] = due.map((r) => reminderText(r, t, locale));
            if (notifications.length > 0 && !stopped) await api.showReminders(notifications);
          } catch {
            // Unshown reminders stay due: the next check tries again.
          }
        } while (again && !stopped);
      })().finally(() => {
        running = null;
      });
    };

    deliver();
    const stop = api.onReminderCheck(deliver);
    return () => {
      stopped = true;
      stop();
    };
  }, [api, enabled, t, locale]);
}
