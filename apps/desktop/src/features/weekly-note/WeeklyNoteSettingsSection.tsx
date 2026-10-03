import { useId } from "react";
import { useTranslation } from "react-i18next";
import { backendKey } from "@/api/ai";
import { useAiStatus, useCostEstimate } from "@/api/ai-queries";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { estimateAmount } from "@/features/ai/lib/money";
import { SettingsSection } from "@/features/settings/SettingsSection";
import { useSetPrepareOnMonday, useWeeklyNoteSettings } from "./useWeeklyNote";

/**
 * Settings → Weekly note (design §5.3): "Prepare my weekly note when I open PageLamp on Monday",
 * only where the note's model allows it (the student's own API key or a model on this computer,
 * never a subscription plan: decision D27; the facade decides). What it costs is said before it's
 * turned on. Turned on and no longer allowed, it stays, paused, so it can be turned off.
 */
export function WeeklyNoteSettingsSection() {
  const { t } = useTranslation("weeklyNote");
  const settings = useWeeklyNoteSettings();
  const save = useSetPrepareOnMonday();
  const id = useId();
  const hintId = useId();
  if (!settings.data) return null;
  const { prepare_on_monday: on, prepare_on_monday_allowed: allowed } = settings.data;
  if (!on && !allowed) return null;

  return (
    <SettingsSection title={t("settings.title")}>
      <div className="flex items-start gap-3">
        <Switch
          id={id}
          checked={on}
          aria-describedby={hintId}
          onCheckedChange={(next) => save.mutate(next)}
        />
        <div className="min-w-0 flex-1 space-y-0.5">
          <Label htmlFor={id}>{t("settings.prepare")}</Label>
          <p id={hintId} className="text-xs text-muted-foreground">
            {allowed ? <CostLine /> : t("settings.paused")}
          </p>
        </div>
      </div>
      {save.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {t("settings.saveFailed")}
        </p>
      ) : null}
    </SettingsSection>
  );
}

/** The note's model, and what each Monday's note costs (or that it runs on this computer). */
function CostLine() {
  const { t, i18n } = useTranslation("weeklyNote");
  const { t: tai } = useTranslation("ai");
  const status = useAiStatus();
  const estimate = useCostEstimate({ feature: "weekly_note" });
  const choice = status.data?.features.find((f) => f.feature === "weekly_note")?.choice;
  const backend = choice
    ? status.data?.backends.find((b) => backendKey(b.backend) === backendKey(choice.backend))
    : undefined;
  if (!choice || !backend) return null;
  const names = { backend: backend.label, model: choice.model };
  if (backend.kind === "local") return t("settings.costLocal", names);
  // A week with nothing to write about has no estimate: no amount, and no "no price" either.
  if (estimate.data?.would_block === "nothing_to_write") {
    return t("settings.costWeekEmpty", names);
  }
  const upper = estimate.data?.micro_usd_upper ?? null;
  if (upper === null) {
    return estimate.data ? t("settings.costUnpriced", names) : null;
  }
  const shown = estimateAmount(upper, i18n.language);
  const cost =
    shown.kind === "lessThan"
      ? tai("estimate.lessThan", { amount: shown.amount })
      : tai("estimate.upTo", { amount: shown.amount });
  return t("settings.costKey", { ...names, cost });
}
