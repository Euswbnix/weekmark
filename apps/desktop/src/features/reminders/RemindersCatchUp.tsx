import { BellRing, X } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { useApi } from "@/api/context";
import { useStartupTasks } from "@/api/queries";
import type { Reminder } from "@/api/reminders";
import { Alert, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { focusPageHeading } from "@/lib/focus";
import { paths, settingsSections } from "@/lib/routes";
import { REMINDERS_UI } from "./availability";
import { useReminderSettings } from "./queries";
import { reminderText } from "./reminderText";

/**
 * Reminders off ("Not now"): what came due since the last launch shows here instead of as
 * notifications (design §5.3): the course code, title and due time.
 * Opening one, or dismissing the card, marks them shown, so they don't come back.
 */
export function RemindersCatchUp() {
  // Behind the switch before any query: a build without the reminder screens asks nothing.
  return REMINDERS_UI ? <CatchUp /> : null;
}

function CatchUp() {
  const { t, i18n } = useTranslation("reminders");
  const api = useApi();
  const tasks = useStartupTasks();
  const settings = useReminderSettings();
  const [seen, setSeen] = useState<ReadonlySet<string>>(new Set());
  const titleId = useId();
  const titleRef = useRef<HTMLDivElement>(null);
  // Decided by the setting at launch: with reminders on then, what was due went out as
  // notifications, and turning them off later mustn't bring those back here.
  const [offAtLaunch, setOffAtLaunch] = useState<boolean | null>(null);
  useEffect(() => {
    if (offAtLaunch === null && settings.data) setOffAtLaunch(!settings.data.run_in_background);
  }, [offAtLaunch, settings.data]);
  if (!offAtLaunch || settings.data?.run_in_background !== false) return null;
  const due = (tasks.data?.due_reminders ?? []).filter((r) => !seen.has(r.id));
  if (due.length === 0) return null;

  function markShown(ids: string[]) {
    setSeen((before) => new Set([...before, ...ids]));
    api.markRemindersShown(ids).catch(() => {});
    // The row (or the whole card) goes. A new page takes the focus itself (AppShell); on the
    // same page it goes to the card's title, else the page's heading.
    requestAnimationFrame(() => {
      const active = document.activeElement;
      if (active && active !== document.body) return;
      if (titleRef.current?.isConnected) titleRef.current.focus();
      else focusPageHeading();
    });
  }

  return (
    <Alert role="region" aria-labelledby={titleId} className="mb-6 px-4 py-3">
      <BellRing aria-hidden />
      <AlertTitle id={titleId} ref={titleRef} tabIndex={-1} className="outline-none">
        {t("catchUp.title")}
      </AlertTitle>
      <div className="col-start-2 space-y-3">
        <ul className="divide-y text-sm">
          {due.map((reminder) => {
            const text = reminderText(reminder, t, i18n.language);
            return (
              <li key={reminder.id} className="flex items-baseline justify-between gap-3 py-1.5">
                <span className="min-w-0">
                  <span className="font-medium">{text.title}</span>
                  {text.body ? <span className="text-muted-foreground"> · {text.body}</span> : null}
                </span>
                <Button asChild variant="ghost" size="sm" className="shrink-0">
                  <Link
                    to={destination(reminder)}
                    // "Open" alone, once per row, says nothing about which one.
                    aria-label={`${t("catchUp.open")} ${text.title}`}
                    onClick={() => markShown([reminder.id])}
                  >
                    {t("catchUp.open")}
                  </Link>
                </Button>
              </li>
            );
          })}
        </ul>
        <div className="flex flex-wrap items-center gap-3">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => markShown(due.map((r) => r.id))}
          >
            <X aria-hidden />
            {t("catchUp.dismiss")}
          </Button>
          <Link
            to={`${paths.settings}#${settingsSections.reminders}`}
            className="text-sm underline underline-offset-2"
          >
            {t("catchUp.turnOn")}
          </Link>
        </div>
      </div>
    </Alert>
  );
}

/** A deadline opens its course; the week and today's plan open Courses (the plan is there). */
function destination(reminder: Reminder): string {
  return reminder.kind === "deadline_soon" && reminder.course_id
    ? paths.course(reminder.course_id)
    : paths.courses;
}
