import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import type { GenStage } from "@/api/ai";
import { useApi } from "@/api/context";
import { toApiError } from "@/api/errors";
import type { GeneratedStudyPlan, StudyPlanRequest } from "@/api/plan";
import { queryKeys } from "@/api/queries";

export type PlanRunState =
  | { phase: "idle" }
  | {
      phase: "running";
      backend: string | null;
      model: string | null;
      stage: GenStage | null;
      stopping: boolean;
    }
  /** A draft to review: nothing is saved until Accept. */
  | { phase: "draft"; draft: GeneratedStudyPlan; request: StudyPlanRequest }
  | { phase: "stopped" }
  | { phase: "failed"; error: unknown; request: StudyPlanRequest };

/**
 * One "Write my plan" run (design §5.1, §7): its GenEvents as state, Stop (cancel_generation), and
 * the draft; Regenerate is a new run with the same request. Usage and the budget refresh after
 * every run, whatever its end.
 */
export function usePlanGeneration() {
  const api = useApi();
  const client = useQueryClient();
  const [state, setState] = useState<PlanRunState>({ phase: "idle" });
  const runId = useRef<string | null>(null);

  // Leaving the page stops the run: nothing keeps writing (or costing) out of sight.
  useEffect(
    () => () => {
      const id = runId.current;
      if (id) void api.cancelGeneration(id).catch(() => {});
    },
    [api],
  );

  const start = useCallback(
    async (request: StudyPlanRequest) => {
      if (runId.current) return;
      const id = crypto.randomUUID();
      runId.current = id;
      setState({ phase: "running", backend: null, model: null, stage: null, stopping: false });
      try {
        const draft = await api.generateStudyPlan(request, id, (event) => {
          if (event.type === "started") {
            const { backend_label, model } = event;
            setState((s) => (s.phase === "running" ? { ...s, backend: backend_label, model } : s));
          } else if (event.type === "stage") {
            const { stage } = event;
            setState((s) => (s.phase === "running" ? { ...s, stage } : s));
          }
        });
        setState({ phase: "draft", draft, request });
      } catch (error) {
        setState(
          toApiError(error).kind === "cancelled"
            ? { phase: "stopped" }
            : { phase: "failed", error, request },
        );
      } finally {
        runId.current = null;
        await client.invalidateQueries({ queryKey: queryKeys.all });
      }
    },
    [api, client],
  );

  const stop = useCallback(async () => {
    const id = runId.current;
    if (!id) return;
    setState((s) => (s.phase === "running" ? { ...s, stopping: true } : s));
    await api.cancelGeneration(id);
  }, [api]);

  const discard = useCallback(() => setState({ phase: "idle" }), []);

  return { state, start, stop, discard };
}

/** Saves a draft as the latest study plan (the Courses page shows it at once). */
export function useAcceptStudyPlan() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (generationId: string) => api.acceptStudyPlan(generationId),
    onSuccess: (stored) => {
      client.setQueryData(queryKeys.studyPlan(), stored);
      void client.invalidateQueries({ queryKey: queryKeys.status() });
    },
  });
}
