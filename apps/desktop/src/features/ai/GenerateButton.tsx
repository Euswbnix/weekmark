import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import {
  type AiStatus,
  type BlockReason,
  backendKey,
  type CostEstimate,
  type EstimateRequest,
} from "@/api/ai";
import {
  useAcknowledgeUnpricedModel,
  useAiStatus,
  useCodexStatus,
  useCostEstimate,
} from "@/api/ai-queries";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Spinner } from "@/components/ui/spinner";
import { paths } from "@/lib/routes";
import { estimateAmount, formatTokens } from "./lib/money";
import { useAiErrorText } from "./useAiErrorText";

/**
 * Whether the cost line applies. No model and the gate's blocks (the course's rules, question
 * (b), a weekly note with nothing to write about) carry no estimate; the others (disclosure,
 * unpriced model, weekly cap, budget) leave it complete, and over the budget the student decides
 * on the override with it.
 */
function showsCost(block: BlockReason | null): boolean {
  return (
    block === null ||
    block === "disclosure_not_acknowledged" ||
    block === "budget_reached" ||
    block === "price_unknown_not_acknowledged" ||
    block === "weekly_run_cap_reached"
  );
}

/** Blocks the student resolves in Settings → AI models. */
const SETTINGS_BLOCKS = new Set(["no_model_chosen", "disclosure_not_acknowledged"]);

/**
 * "≈ $x" and Generate (design §3.5, §7): the facade's upper-bound estimate for this request,
 * refreshed as the request changes, and the pre-flight blocks it reports (`would_block`). The
 * blocks the student can settle here are settled here: an unpriced model is acknowledged once,
 * and going over the budget is an explicit, per-run choice. Everything else disables Generate
 * with the reason.
 */
export function GenerateButton({
  request,
  onGenerate,
  label,
  describedBy,
  variant,
}: {
  /** null = the form isn't complete yet. */
  request: EstimateRequest | null;
  onGenerate: (options: { overrideBudget: boolean }) => void;
  label?: string;
  /** An element saying why the form isn't ready (added to the button's description). */
  describedBy?: string;
  /** "outline" where another button on the screen is the main one (Plan's Write again). */
  variant?: "default" | "outline";
}) {
  const { t, i18n } = useTranslation("ai");
  const estimate = useCostEstimate(request);
  const status = useAiStatus();
  const errorText = useAiErrorText();
  const [overrideBudget, setOverrideBudget] = useState(false);
  const ids = { line: useId(), override: useId(), reason: useId() };

  const data = estimate.data ?? null;
  const block = data?.would_block ?? null;
  // The tick answers this block only: once an estimate arrives without it (the budget changed
  // elsewhere), the tick goes, and a box that comes back isn't ticked already. A refetch keeps
  // the estimate on screen, so the same block keeps the tick.
  useEffect(() => {
    if (block !== "budget_reached") setOverrideBudget(false);
  }, [block]);
  const blocked = block !== null && !(block === "budget_reached" && overrideBudget);
  // Not while the estimate on screen is still the previous request's; a refetch of the same
  // request leaves the button as it is.
  const disabled = request === null || !data || blocked || estimate.settling;

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-3">
        <Button
          type="button"
          variant={variant}
          aria-disabled={disabled || undefined}
          aria-describedby={[block ? `${ids.line} ${ids.reason}` : ids.line, describedBy]
            .filter(Boolean)
            .join(" ")}
          className="aria-disabled:opacity-50"
          onClick={() => {
            if (!disabled) onGenerate({ overrideBudget: block === "budget_reached" });
          }}
        >
          {label ?? t("estimate.generate")}
        </Button>
        <p id={ids.line} className="text-sm text-muted-foreground" aria-live="polite">
          {estimate.isPending && request ? <Spinner aria-hidden /> : null}
          {data && request && showsCost(block) ? (
            <CostLine estimate={data} status={status.data ?? null} feature={request.feature} />
          ) : null}
        </p>
      </div>
      {data && request && data.input_tokens > 0 ? (
        <p className="text-xs text-muted-foreground">
          {t("estimate.details", {
            input: formatTokens(data.input_tokens, i18n.language),
            output: formatTokens(data.max_output_tokens + data.reasoning_allowance, i18n.language),
          })}
        </p>
      ) : null}
      {estimate.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {errorText(estimate.error)}
        </p>
      ) : null}
      {block === "budget_reached" ? (
        <div className="space-y-1.5">
          <p id={ids.reason} className="text-sm font-medium">
            {t("estimate.overBudget")}
          </p>
          <div className="flex items-center gap-2">
            <Checkbox
              id={ids.override}
              checked={overrideBudget}
              onCheckedChange={(v) => setOverrideBudget(v === true)}
            />
            <Label htmlFor={ids.override} className="font-normal">
              {t("estimate.overrideBudget")}
            </Label>
          </div>
        </div>
      ) : null}
      {block === "price_unknown_not_acknowledged" && request ? (
        <UnpricedAcknowledgement
          status={status.data ?? null}
          feature={request.feature}
          hintId={ids.reason}
        />
      ) : null}
      {block && block !== "budget_reached" && block !== "price_unknown_not_acknowledged" ? (
        <p id={ids.reason} className="text-sm">
          {t(`blocked.${block}`)}{" "}
          {SETTINGS_BLOCKS.has(block) ? (
            <Link to={paths.settings} className="underline underline-offset-4">
              {t("settings.title")}
            </Link>
          ) : null}
        </p>
      ) : null}
    </div>
  );
}

function CostLine({
  estimate,
  status,
  feature,
}: {
  estimate: CostEstimate;
  status: AiStatus | null;
  feature: EstimateRequest["feature"];
}) {
  const { t, i18n } = useTranslation("ai");
  const upper = estimate.micro_usd_upper ?? null;
  if (upper === 0) return <>{t("estimate.free")}</>;
  if (upper !== null) {
    const shown = estimateAmount(upper, i18n.language);
    return (
      <>
        <span className="sr-only">{t("estimate.label")}</span>{" "}
        {shown.kind === "lessThan"
          ? t("estimate.lessThan", { amount: shown.amount })
          : t("estimate.upTo", { amount: shown.amount })}
      </>
    );
  }
  const choice = status?.features.find((f) => f.feature === feature)?.choice ?? null;
  const backend = choice
    ? status?.backends.find((b) => backendKey(b.backend) === backendKey(choice.backend))
    : undefined;
  if (backend?.kind === "codex") return <CodexCostLine />;
  if (backend?.kind === "local") return <>{t("estimate.cloudNoPrice")}</>;
  if (backend?.kind === "api_key") return <>{t("estimate.noPrice")}</>;
  return null;
}

/** Mode A has no price: the plan, and this week's runs against the cap. */
function CodexCostLine() {
  const { t } = useTranslation("ai");
  const codex = useCodexStatus();
  const cap = codex.data?.weekly_cap ?? null;
  if (codex.data && cap !== null) {
    return <>{t("codex.costLineRuns", { runs: codex.data.runs_this_week, cap })}</>;
  }
  return <>{t("codex.costLine")}</>;
}

function UnpricedAcknowledgement({
  status,
  feature,
  hintId,
}: {
  status: AiStatus | null;
  feature: EstimateRequest["feature"];
  hintId: string;
}) {
  const { t } = useTranslation("ai");
  const acknowledge = useAcknowledgeUnpricedModel();
  const errorText = useAiErrorText();
  const choice = status?.features.find((f) => f.feature === feature)?.choice ?? null;
  if (!choice) return null;
  return (
    <div className="space-y-1.5">
      <p id={hintId} className="text-sm">
        {t("estimate.useUnpricedHint")}
      </p>
      <Button
        type="button"
        size="sm"
        variant="outline"
        disabled={acknowledge.isPending}
        onClick={() => acknowledge.mutate({ backend: choice.backend, model: choice.model })}
      >
        {t("estimate.useUnpriced")}
      </Button>
      {acknowledge.error ? (
        <p role="alert" className="text-sm text-destructive">
          {errorText(acknowledge.error)}
        </p>
      ) : null}
    </div>
  );
}
