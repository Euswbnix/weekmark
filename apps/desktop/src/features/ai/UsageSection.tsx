import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { UsageRow, UsageSummary } from "@/api/ai";
import { useAiStatus, useUsageSummary } from "@/api/ai-queries";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { SettingsSection } from "@/features/settings/SettingsSection";
import { useToday } from "@/lib/useToday";
import { formatTokens, formatUsd } from "./lib/money";
import { useAiErrorText } from "./useAiErrorText";

/** The ledger keeps 13 months (design §3.5). */
const MONTHS_KEPT = 13;

/** "YYYY-MM-01" for this month and the 12 before it, newest first. */
export function recentMonths(today: string): string[] {
  const [y, m] = today.split("-").map(Number) as [number, number];
  return Array.from({ length: MONTHS_KEPT }, (_, i) => {
    const date = new Date(y, m - 1 - i, 1);
    return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-01`;
  });
}

function monthLabel(month: string, locale: string): string {
  const [y, m] = month.split("-").map(Number) as [number, number];
  return new Intl.DateTimeFormat(locale, { year: "numeric", month: "long" }).format(
    new Date(y, m - 1, 1),
  );
}

/**
 * Settings → AI usage (design §7): runs, tokens and estimated cost per backend, model and feature
 * for a month, from the local ledger (counts only, never content), with the budget.
 */
export function UsageSection() {
  const { t, i18n } = useTranslation("ai");
  const today = useToday();
  const months = recentMonths(today);
  const [chosen, setChosen] = useState<string | null>(null);
  const month = chosen && months.includes(chosen) ? chosen : (months[0] ?? null);
  const usage = useUsageSummary(month);
  // Without the ChatGPT plan in this build, nothing here names it (no plan rows, no weekly runs).
  const offered = useAiStatus().data?.chatgpt_plan_offered ?? false;
  const errorText = useAiErrorText();
  const monthId = useId();

  return (
    <SettingsSection title={t("usage.title")} description={t("usage.description")}>
      <div className="flex items-center gap-3">
        <label htmlFor={monthId} className="text-sm font-medium">
          {t("usage.month")}
        </label>
        <Select value={month ?? ""} onValueChange={setChosen}>
          <SelectTrigger id={monthId} className="w-48">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {months.map((m) => (
              <SelectItem key={m} value={m}>
                {monthLabel(m, i18n.language)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {usage.isPending ? (
        <Skeleton className="h-24 w-full" />
      ) : usage.isError ? (
        <p role="alert" className="text-sm text-destructive">
          {errorText(usage.error)}
        </p>
      ) : (
        <>
          <UsageTable summary={offered ? usage.data : withoutPlan(usage.data)} />
          {month === months[0] ? (
            <BudgetLine summary={offered ? usage.data : withoutPlan(usage.data)} />
          ) : null}
        </>
      )}
    </SettingsSection>
  );
}

/** The summary without the ChatGPT plan's rows and weekly runs (the plan isn't offered). */
function withoutPlan(summary: UsageSummary): UsageSummary {
  return { ...summary, rows: summary.rows.filter((r) => r.cost_basis !== "plan"), mode_a: null };
}

function UsageTable({ summary }: { summary: UsageSummary }) {
  const { t, i18n } = useTranslation("ai");
  const locale = i18n.language;
  const label = monthLabel(summary.month, locale);
  if (summary.rows.length === 0) {
    return <p className="text-sm text-muted-foreground">{t("usage.empty", { month: label })}</p>;
  }
  const estimated = summary.rows.some((r) => r.estimated);
  return (
    <div className="space-y-2">
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <caption className="sr-only">{t("usage.caption", { month: label })}</caption>
          <thead className="text-left text-muted-foreground">
            <tr className="border-b">
              <th scope="col" className="py-2 pr-4 font-medium">
                {t("usage.columns.model")}
              </th>
              <th scope="col" className="py-2 pr-4 font-medium">
                {t("usage.columns.feature")}
              </th>
              <th scope="col" className="py-2 pr-4 text-right font-medium">
                {t("usage.columns.runs")}
              </th>
              <th scope="col" className="py-2 pr-4 text-right font-medium">
                {t("usage.columns.tokens")}
              </th>
              <th scope="col" className="py-2 text-right font-medium">
                {t("usage.columns.cost")}
              </th>
            </tr>
          </thead>
          <tbody>
            {summary.rows.map((row) => (
              <tr
                key={`${row.backend_label}/${row.model}/${row.feature}`}
                className="border-b last:border-0"
              >
                <th scope="row" className="py-2 pr-4 text-left font-normal">
                  <span className="block">{row.model}</span>
                  <span className="block text-xs text-muted-foreground">{row.backend_label}</span>
                </th>
                <td className="py-2 pr-4">{t(`features.name.${row.feature}`)}</td>
                <td className="py-2 pr-4 text-right tabular-nums">{row.runs}</td>
                <td className="py-2 pr-4 text-right tabular-nums">
                  {t("usage.tokens", {
                    input: formatTokens(row.input_tokens, locale),
                    output: formatTokens(row.output_tokens, locale),
                  })}
                </td>
                <td className="py-2 text-right tabular-nums">
                  <Cost row={row} />
                </td>
              </tr>
            ))}
          </tbody>
          <tfoot>
            <tr>
              <th scope="row" colSpan={4} className="pt-2 pr-4 text-left font-medium">
                {t("usage.total")}
              </th>
              <td className="pt-2 text-right font-medium tabular-nums">
                {estimated ? "≈ " : ""}
                {formatUsd(summary.total_micro_usd, locale)}
              </td>
            </tr>
          </tfoot>
        </table>
      </div>
      {estimated ? (
        <p className="text-xs text-muted-foreground">{t("usage.estimatedNote")}</p>
      ) : null}
    </div>
  );
}

function Cost({ row }: { row: UsageRow }) {
  const { t, i18n } = useTranslation("ai");
  switch (row.cost_basis) {
    case "free_on_device":
      return <>{t("usage.free")}</>;
    case "unpriced":
      return <>{t("usage.noPrice")}</>;
    case "plan":
      return <>{t("usage.plan")}</>;
    case "priced":
      return (
        <>
          {row.estimated ? "≈ " : ""}
          {formatUsd(row.micro_usd ?? 0, i18n.language)}
        </>
      );
  }
}

/** This month only: how much of the API-key budget is used. */
function BudgetLine({ summary }: { summary: UsageSummary }) {
  const { t, i18n } = useTranslation("ai");
  const { monthly_micro_usd: budget, spent_micro_usd: spent } = summary.budget;
  const modeA = summary.mode_a ?? null;
  return (
    <>
      {budget === null || budget === undefined ? null : (
        <p className="text-sm">
          {t("budget.used", {
            spent: formatUsd(spent, i18n.language),
            budget: formatUsd(budget, i18n.language),
          })}
        </p>
      )}
      {/* Mode A has no money budget; its runs per week are capped instead (M2). */}
      {modeA ? (
        <p className="text-sm">
          {modeA.weekly_cap === null || modeA.weekly_cap === undefined
            ? t("usage.modeANoCap", { runs: modeA.runs_this_week })
            : t("usage.modeA", { runs: modeA.runs_this_week, cap: modeA.weekly_cap })}
        </p>
      ) : null}
    </>
  );
}
