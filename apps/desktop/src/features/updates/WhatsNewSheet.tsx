import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  useAcknowledgeWhatsNew,
  useSetUpdatePrefs,
  useStartupTasks,
  useUpdatePrefs,
} from "@/api/queries";
import type { WhatsNewTopic } from "@/api/types";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { shownTopics, TOPIC_ICON } from "./whatsNewTopics";

/**
 * One-time "What's new" for upgraders: the topics introduced since their version (all of them
 * from 0.1, which never saw onboarding). It explains the automatic update check BEFORE the first
 * one runs, with the switch right there. Closing it any way counts as read; the facade then
 * decides whether a check is due. Each topic is a heading; the focus starts on the title so the
 * sheet is read from the top, and a long list scrolls inside the window.
 *
 * The update check waits for the acknowledgement, so a topic this build has no copy or icon for
 * is left out, and with none left the sheet is acknowledged without being shown.
 */
export function WhatsNewSheet() {
  const tasks = useStartupTasks();
  const { i18n } = useTranslation("updates");
  const whatsNew = tasks.data?.whats_new;
  if (!whatsNew) return null;
  const topics = shownTopics(whatsNew.topics, (key) => i18n.exists(key, { ns: "updates" }));
  if (topics.length === 0) return <AcknowledgeUnshown />;
  return <Sheet since={whatsNew.since ?? null} topics={topics} />;
}

/** Nothing to show: still count What's new as read, once, so the update check isn't held. */
function AcknowledgeUnshown() {
  const acknowledge = useAcknowledgeWhatsNew();
  const sent = useRef(false);
  useEffect(() => {
    if (sent.current) return;
    sent.current = true;
    acknowledge.mutate();
  }, [acknowledge]);
  return null;
}

function Sheet({ since, topics }: { since: string | null; topics: WhatsNewTopic[] }) {
  const { t } = useTranslation("updates");
  const prefs = useUpdatePrefs();
  const setPrefs = useSetUpdatePrefs();
  const acknowledge = useAcknowledgeWhatsNew();
  const [open, setOpen] = useState(true);
  // null = untouched: keep whatever the preference is.
  const [autoCheck, setAutoCheck] = useState<boolean | null>(null);
  const switchId = useId();
  const titleRef = useRef<HTMLHeadingElement>(null);
  const current = autoCheck ?? prefs.data?.auto_check ?? true;

  async function done() {
    setOpen(false);
    if (autoCheck !== null && prefs.data && autoCheck !== prefs.data.auto_check) {
      await setPrefs.mutateAsync({ auto_check: autoCheck, channel: prefs.data.channel ?? null });
    }
    await acknowledge.mutateAsync();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) void done();
      }}
    >
      <DialogContent
        className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-lg"
        showCloseButton={false}
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          titleRef.current?.focus();
        }}
      >
        <DialogHeader>
          <DialogTitle ref={titleRef} tabIndex={-1} className="outline-none">
            {t("whatsNew.title")}
          </DialogTitle>
          {since ? (
            <DialogDescription>{t("whatsNew.since", { version: since })}</DialogDescription>
          ) : null}
        </DialogHeader>
        <ul className="space-y-4">
          {topics.map((topic) => {
            const Icon = TOPIC_ICON[topic];
            return (
              <li key={topic} className="flex gap-3">
                <Icon className="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden />
                <div className="min-w-0 space-y-1">
                  <h3 className="font-medium">{t(`whatsNew.topics.${topic}.title`)}</h3>
                  <p className="text-muted-foreground">{t(`whatsNew.topics.${topic}.body`)}</p>
                  {topic === "update_check" ? (
                    <div className="flex items-center gap-2 pt-1">
                      <Switch
                        id={switchId}
                        checked={current}
                        onCheckedChange={(checked) => setAutoCheck(checked)}
                      />
                      <Label htmlFor={switchId}>{t("settings.autoCheck")}</Label>
                    </div>
                  ) : null}
                </div>
              </li>
            );
          })}
        </ul>
        <DialogFooter>
          <Button type="button" onClick={() => void done()}>
            {t("whatsNew.done")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
