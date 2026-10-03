import { BellRing } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { useFocusOnMount } from "@/lib/useFocusOnMount";
import { useReminderSettings, useSetReminderSettings } from "./queries";

/**
 * Onboarding's one explicit question (design §5.3): "Remind me (keep PageLamp in the tray and
 * start it at login)". Yes turns on both, No leaves both off; either can change in Settings.
 * Shown while the first sync runs, so answering costs no extra step.
 */
export function RemindMeCard() {
  const { t } = useTranslation("reminders");
  const settings = useReminderSettings();
  const save = useSetReminderSettings();
  const [answer, setAnswer] = useState<boolean | null>(null);
  const titleId = useId();
  if (!settings.data) return null;
  const current = settings.data;

  function choose(yes: boolean) {
    setAnswer(yes);
    save.mutate({ ...current, run_in_background: yes });
  }

  return (
    <section aria-labelledby={titleId}>
      <Card>
        <CardHeader>
          <h2 id={titleId} className="flex items-center gap-2 font-heading text-base font-medium">
            <BellRing className="size-4" aria-hidden />
            {t("remind.title")}
          </h2>
        </CardHeader>
        <CardContent className="space-y-3">
          {answer === null || save.isError ? (
            <>
              <p className="text-sm">{t("remind.question")}</p>
              <div className="flex flex-wrap gap-2">
                <Button type="button" onClick={() => choose(true)}>
                  {t("remind.yes")}
                </Button>
                <Button type="button" variant="outline" onClick={() => choose(false)}>
                  {t("remind.no")}
                </Button>
              </div>
            </>
          ) : (
            <Answered text={answer ? t("remind.onDone") : t("remind.offDone")} />
          )}
          {save.isError ? <SaveFailed text={t("settings.saveFailed")} /> : null}
        </CardContent>
      </Card>
    </section>
  );
}

/** Replaces the buttons, so it takes the focus. */
function Answered({ text }: { text: string }) {
  const ref = useFocusOnMount<HTMLParagraphElement>();
  return (
    <p ref={ref} tabIndex={-1} role="status" className="text-sm text-muted-foreground outline-none">
      {text}
    </p>
  );
}

/** Replaces the note that had the focus (the buttons come back), so it takes the focus. */
function SaveFailed({ text }: { text: string }) {
  const ref = useFocusOnMount<HTMLParagraphElement>();
  return (
    <p ref={ref} tabIndex={-1} role="alert" className="text-sm text-destructive outline-none">
      {text}
    </p>
  );
}
