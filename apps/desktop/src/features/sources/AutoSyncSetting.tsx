import { useId } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useSetSyncPrefs, useSyncPrefs } from "@/api/queries";
import type { AutoSync } from "@/api/types";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { useApiErrorText } from "@/lib/useApiErrorText";
import { useSyncStore } from "@/stores/sync";

const OPTIONS = ["off", "daily", "twice_daily"] as const satisfies readonly AutoSync[];

function isAutoSync(value: string): value is AutoSync {
  return OPTIONS.some((option) => option === value);
}

/**
 * How often PageLamp syncs by itself while it is open: off, once or twice a day. Saved as soon
 * as it is chosen; the facade is then asked again what is due, so turning it on can start a due
 * sync right away. The hint says what each kind of automatic sync fetches, and that Canvas may
 * record a full one as the student's activity.
 */
export function AutoSyncSetting() {
  const { t } = useTranslation("sources");
  const prefs = useSyncPrefs();
  const save = useSetSyncPrefs();
  const errorText = useApiErrorText();
  const labelId = useId();
  const hintId = useId();
  const current = prefs.data;
  if (!current) return null;
  const chosen: AutoSync = current.auto_sync ?? "twice_daily";

  async function choose(value: string) {
    // A single ToggleGroup reports "" when the active item is clicked again; ignore that.
    if (!current || !isAutoSync(value) || value === chosen || save.isPending) return;
    // The student chose this just now: a sync that becomes due counts as attended.
    useSyncStore.getState().noteStudentAction();
    try {
      await save.mutateAsync({ ...current, auto_sync: value });
    } catch (error) {
      toast.error(errorText(error));
    }
  }

  return (
    <div className="space-y-2">
      <div id={labelId} className="text-sm font-medium">
        {t("autoSync.label")}
      </div>
      <ToggleGroup
        type="single"
        variant="outline"
        spacing={0}
        value={chosen}
        onValueChange={(value) => void choose(value)}
        aria-labelledby={labelId}
        aria-describedby={hintId}
      >
        {OPTIONS.map((option) => (
          <ToggleGroupItem key={option} value={option} className="px-3">
            {t(`autoSync.option.${option}`)}
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
      <p id={hintId} className="text-sm text-muted-foreground">
        {t("autoSync.hint")}
      </p>
    </div>
  );
}
