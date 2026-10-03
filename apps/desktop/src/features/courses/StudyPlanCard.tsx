import { Clock, Sparkles } from "lucide-react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Link } from "react-router";
import { useCourses, useSetStudyPlanItemDone, useStudyPlan } from "@/api/queries";
import type { StoredStudyPlan } from "@/api/types";
import { AiGeneratedLabel } from "@/components/common/AiGeneratedLabel";
import { ErrorState } from "@/components/common/ErrorState";
import { SentenceWithTime, WHEN } from "@/components/common/SentenceWithTime";
import { Button } from "@/components/ui/button";
import { AI_SETUP_ENABLED } from "@/lib/features";
import { formatIsoDate } from "@/lib/format";
import { paths } from "@/lib/routes";
import { isPlanStale } from "./lib/plan";
import { Section } from "./parts/Section";
import { PlanSkeleton } from "./Skeletons";
import { StudyPlanBody } from "./StudyPlanBody";
import { StudyPlanEmpty } from "./StudyPlanEmpty";

/**
 * The latest study plan the student's AI app saved over MCP (`save_study_plan`). Read-only
 * in v0.1: to change it, the student asks their AI app.
 */
export function StudyPlanCard() {
  const { t } = useTranslation("courses");
  const plan = useStudyPlan();
  // For showing course codes next to plan items; the list below the card loads it anyway.
  const courses = useCourses();
  const tick = useSetStudyPlanItemDone();

  let description: ReactNode = null;
  let body: ReactNode;
  if (plan.isPending) {
    body = <PlanSkeleton />;
  } else if (plan.isError) {
    body = (
      <ErrorState
        error={plan.error}
        title={t("plan.errorTitle")}
        onRetry={() => void plan.refetch()}
      />
    );
  } else if (!plan.data) {
    body = <StudyPlanEmpty />;
  } else {
    description = <PlanMeta stored={plan.data} />;
    const planId = plan.data.id;
    body = (
      <>
        <StudyPlanBody
          plan={plan.data.plan}
          courses={courses.data}
          onToggle={
            AI_SETUP_ENABLED
              ? (itemIndex, done) => tick.mutate({ planId, itemIndex, done })
              : undefined
          }
        />
        {tick.isError ? (
          <p role="alert" className="mt-2 text-sm">
            {t("plan.tickFailed")}
          </p>
        ) : null}
      </>
    );
  }

  return (
    <Section title={t("plan.title")} description={description} actions={<PlanWithPageLamp />}>
      {body}
    </Section>
  );
}

/** M3: "Plan with PageLamp…" (design §7), next to the plan the student's AI app may have saved. */
function PlanWithPageLamp() {
  const { t } = useTranslation("plan");
  if (!AI_SETUP_ENABLED) return null;
  return (
    <Button asChild variant="outline" size="sm">
      <Link to={paths.plan}>
        <Sparkles aria-hidden />
        {t("entry")}
      </Link>
    </Button>
  );
}

/** "Made by your AI app 2 days ago · covers …" and, if needed, the out-of-date warning. */
function PlanMeta({ stored }: { stored: StoredStudyPlan }) {
  const { t, i18n } = useTranslation("courses");
  const { horizon_start: start, horizon_end: end } = stored.plan;
  return (
    <div className="space-y-1">
      <p>
        <SentenceWithTime
          iso={stored.created_at}
          text={t(stored.origin === "pagelamp" ? "plan.madeByPageLamp" : "plan.madeBy", {
            when: WHEN,
            start: formatIsoDate(start, i18n.language),
            end: formatIsoDate(end, i18n.language),
          })}
        />
      </p>
      {/* Written by PageLamp: the AI-generated line (Canvas §2E), kept with the saved plan. */}
      {stored.ai_label ? <AiGeneratedLabel meta={stored.ai_label} /> : null}
      {isPlanStale(stored) ? (
        <p className="flex items-center gap-1.5 text-foreground">
          <Clock className="size-4 shrink-0 text-warning" aria-hidden />
          {t(stored.origin === "pagelamp" ? "plan.stalePageLamp" : "plan.stale")}
        </p>
      ) : null}
    </div>
  );
}
