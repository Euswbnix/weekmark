import { CalendarRange, FolderSync, type LucideIcon, RefreshCw } from "lucide-react";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  useAcknowledgeWhatsNew,
  useSetSyncPrefs,
  useSetUpdatePrefs,
  useStartupTasks,
  useSyncPrefs,
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

const TOPIC_ICON: Record<WhatsNewTopic, LucideIcon> = {
  update_check: RefreshCw,
  course_weeks: CalendarRange,
  auto_sync: FolderSync,
};

/**
 * One-time "What's new" for upgraders (from 0.1 or an earlier alpha), who never saw
 * onboarding. It explains the automatic update check and the automatic sync BEFORE the first
 * one runs, each with its switch right there. Closing it any way counts as read; the facade then
 * decides whether a check or a sync is due.
 */
export function WhatsNewSheet() {
  const tasks = useStartupTasks();
  const whatsNew = tasks.data?.whats_new;
  if (!whatsNew || whatsNew.topics.length === 0) return null;
  return <Sheet since={whatsNew.since ?? null} topics={whatsNew.topics} />;
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
  const current = autoCheck ?? prefs.data?.auto_check ?? true;
  const syncPrefs = useSyncPrefs();
  const setSyncPrefs = useSetSyncPrefs();
  // The same for automatic sync: null = untouched. How often is chosen on Sources & sync.
  const [autoSync, setAutoSync] = useState<boolean | null>(null);
  const syncSwitchId = useId();
  const syncWasOn = syncPrefs.data?.auto_sync !== "off";
  const syncOn = autoSync ?? syncWasOn;

  async function done() {
    setOpen(false);
    if (autoCheck !== null && prefs.data && autoCheck !== prefs.data.auto_check) {
      await setPrefs.mutateAsync({ auto_check: autoCheck, channel: prefs.data.channel ?? null });
    }
    // Saved before the acknowledgement: once that is in, a sync may be due.
    // Only a real change is saved. When the setting couldn't be read it counts as on (the
    // default), so "off" is still saved, and off-then-on writes nothing over a stored choice.
    if (autoSync !== null && autoSync !== syncWasOn) {
      await setSyncPrefs.mutateAsync({
        ...syncPrefs.data,
        auto_sync: autoSync ? "twice_daily" : "off",
      });
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
      <DialogContent className="sm:max-w-lg" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>{t("whatsNew.title")}</DialogTitle>
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
                  <p className="font-medium">{t(`whatsNew.topics.${topic}.title`)}</p>
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
                  {topic === "auto_sync" ? (
                    <div className="flex items-center gap-2 pt-1">
                      <Switch
                        id={syncSwitchId}
                        checked={syncOn}
                        onCheckedChange={(checked) => setAutoSync(checked)}
                      />
                      <Label htmlFor={syncSwitchId}>{t("whatsNew.autoSync")}</Label>
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
