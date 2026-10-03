import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "react-router";
import { toast } from "sonner";
import type { StudyPlanRequest } from "@/api/plan";
import { PageHeader } from "@/components/common/PageHeader";
import { Button } from "@/components/ui/button";
import { useAiErrorText } from "@/features/ai/useAiErrorText";
import { paths } from "@/lib/routes";
import { useApiErrorText } from "@/lib/useApiErrorText";
import { useFocusOnMount } from "@/lib/useFocusOnMount";
import { PlanDraft } from "./PlanDraft";
import { PlanForm } from "./PlanForm";
import { type PlanRunState, useAcceptStudyPlan, usePlanGeneration } from "./usePlanGeneration";

/**
 * "Plan your study" (design §7): the request, the run with its stages and Stop, then the draft
 * to accept, write again or discard. Accepting saves it as the study plan the Courses page shows.
 */
export function PlanPage() {
  const { t } = useTranslation("plan");
  const navigate = useNavigate();
  const run = usePlanGeneration();
  const accept = useAcceptStudyPlan();
  const acceptErrorText = useApiErrorText();
  const [lastRequest, setLastRequest] = useState<StudyPlanRequest | null>(null);
  const [discarded, setDiscarded] = useState(false);
  const { state } = run;

  function start(request: StudyPlanRequest) {
    setLastRequest(request);
    setDiscarded(false);
    accept.reset();
    void run.start(request);
  }

  return (
    <div className="max-w-3xl">
      <PageHeader title={t("title")} description={t("description")} />
      <div className="space-y-6">
        {state.phase === "running" ? (
          <PlanProgress state={state} onStop={() => void run.stop()} />
        ) : state.phase === "draft" ? (
          <PlanDraft
            draft={state.draft}
            accepting={accept.isPending}
            onAccept={() =>
              accept.mutate(state.draft.meta.generation_id, {
                onSuccess: () => {
                  toast.success(t("draft.accepted"));
                  navigate(paths.courses);
                },
              })
            }
            regenerate={{
              feature: "study_plan",
              courses: state.request.courses ?? [],
              horizon_days: state.request.horizon_days,
            }}
            // A new run: going over the budget is chosen again, next to its estimate.
            onRegenerate={(overrideBudget) =>
              start({ ...state.request, override_budget: overrideBudget })
            }
            onDiscard={() => {
              run.discard();
              setDiscarded(true);
            }}
          />
        ) : (
          <PlanForm initial={lastRequest} onGenerate={start} />
        )}
        <PlanOutcome state={state} discarded={discarded} />
        {accept.error ? (
          <div role="alert" className="text-sm">
            <p className="font-medium">{t("draft.acceptFailed")}</p>
            <p className="text-muted-foreground">{acceptErrorText(accept.error)}</p>
          </div>
        ) : null}
      </div>
    </div>
  );
}

/** "Writing with <backend> · <model>", the stage, and Stop. */
function PlanProgress({
  state,
  onStop,
}: {
  state: Extract<PlanRunState, { phase: "running" }>;
  onStop: () => void;
}) {
  const { t } = useTranslation("plan");
  // The button that started the run is gone: Stop takes the focus.
  const stopRef = useFocusOnMount<HTMLButtonElement>();
  const leaveId = useId();
  return (
    <div className="space-y-2 text-sm">
      <p className="font-medium">
        {state.backend && state.model
          ? t("running.withModel", { backend: state.backend, model: state.model })
          : t("running.starting")}
      </p>
      {state.stage ? (
        <p className="text-muted-foreground" aria-hidden>
          {t(`running.stage.${state.stage}`)}
        </p>
      ) : null}
      <Button
        ref={stopRef}
        type="button"
        size="sm"
        variant="outline"
        onClick={onStop}
        aria-disabled={state.stopping || undefined}
        aria-describedby={leaveId}
        className="aria-disabled:opacity-50"
      >
        {state.stopping ? t("running.stopping") : t("running.stop")}
      </Button>
      <p id={leaveId} className="text-xs text-muted-foreground">
        {t("running.leaveHint")}
      </p>
    </div>
  );
}

/**
 * The live region (design §7, accessibility): the stages and how the run ended, never more; a
 * failure is an alert. When a run stops, fails or its draft is discarded, the form comes back
 * and the focus goes to what happened.
 */
function PlanOutcome({ state, discarded }: { state: PlanRunState; discarded: boolean }) {
  const { t } = useTranslation("plan");
  const errorText = useAiErrorText();
  const statusRef = useRef<HTMLParagraphElement>(null);
  const alertRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (state.phase === "failed") alertRef.current?.focus();
    else if (state.phase === "stopped" || (state.phase === "idle" && discarded)) {
      statusRef.current?.focus();
    }
  }, [state.phase, discarded]);
  const text =
    state.phase === "running"
      ? state.stage
        ? t(`running.stage.${state.stage}`)
        : t("running.writingNow")
      : state.phase === "draft"
        ? t("draft.ready")
        : state.phase === "stopped"
          ? t("draft.stopped")
          : discarded
            ? t("draft.discarded")
            : "";
  const visible = state.phase === "stopped" || (state.phase === "idle" && discarded);
  return (
    <>
      <p
        ref={statusRef}
        tabIndex={-1}
        role="status"
        className={visible ? "text-sm outline-none" : "sr-only"}
      >
        {text}
      </p>
      {state.phase === "failed" ? (
        <div ref={alertRef} tabIndex={-1} role="alert" className="text-sm outline-none">
          <p className="font-medium">{t("draft.failed")}</p>
          <p className="text-muted-foreground">{errorText(state.error)}</p>
        </div>
      ) : null}
    </>
  );
}
