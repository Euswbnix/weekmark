import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import type { GenStage } from "@/api/ai";
import { useApi } from "@/api/context";
import { toApiError } from "@/api/errors";
import type { OutputLanguage, WeeklyExplanation } from "@/api/explain";
import { queryKeys } from "@/api/queries";

export const explainKeys = {
  saved: (courseId: string, week: number | null) =>
    [...queryKeys.all, "explanations", courseId, week] as const,
  language: () => [...queryKeys.all, "ai-output-language"] as const,
};

export type ExplainRunState =
  | { phase: "idle" }
  | {
      phase: "running";
      backend: string | null;
      model: string | null;
      stage: GenStage | null;
      /** The model runs on this computer (the `started` event); null until then. */
      onDevice: boolean | null;
      /** Materials whose text is read (the `context` event). */
      materials: number | null;
      stopping: boolean;
    }
  | { phase: "done"; explanation: WeeklyExplanation }
  | { phase: "stopped" }
  | { phase: "failed"; error: unknown };

/**
 * One "Explain week N" run (design §5.2, §7): its GenEvents as state (no text arrives before
 * the end), Stop (cancel_generation), and the saved list refreshed after every run. It also
 * remembers what each of its runs included (`sentInclude`): Regenerate sends that again and
 * Include adds to it. Explanations saved before don't say, so they count as none.
 */
export function useExplanation(courseId: string) {
  const api = useApi();
  const client = useQueryClient();
  const [state, setState] = useState<ExplainRunState>({ phase: "idle" });
  const [sent, setSent] = useState<ReadonlyMap<string, string[]>>(new Map());
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
    async (
      week: number | null,
      options: { overrideBudget: boolean; include: string[]; uiLanguage: string },
    ) => {
      if (runId.current) return;
      const id = crypto.randomUUID();
      runId.current = id;
      setState({
        phase: "running",
        backend: null,
        model: null,
        stage: null,
        onDevice: null,
        materials: null,
        stopping: false,
      });
      try {
        const explanation = await api.explainWeek(
          courseId,
          week,
          id,
          {
            include: options.include,
            override_budget: options.overrideBudget,
            ui_language: options.uiLanguage,
          },
          (event) => {
            if (event.type === "started") {
              const { backend_label, model, on_device } = event;
              setState((s) =>
                s.phase === "running"
                  ? { ...s, backend: backend_label, model, onDevice: on_device }
                  : s,
              );
            } else if (event.type === "stage") {
              const { stage } = event;
              setState((s) => (s.phase === "running" ? { ...s, stage } : s));
            } else if (event.type === "context") {
              const materials = event.summary.materials_included;
              setState((s) => (s.phase === "running" ? { ...s, materials } : s));
            }
          },
        );
        setSent((before) => new Map(before).set(explanation.meta.generation_id, options.include));
        setState({ phase: "done", explanation });
      } catch (error) {
        setState(
          toApiError(error).kind === "cancelled"
            ? { phase: "stopped" }
            : { phase: "failed", error },
        );
      } finally {
        runId.current = null;
        await client.invalidateQueries({ queryKey: queryKeys.all });
      }
    },
    [api, client, courseId],
  );

  const stop = useCallback(async () => {
    const id = runId.current;
    if (!id) return;
    setState((s) => (s.phase === "running" ? { ...s, stopping: true } : s));
    // A failed cancel leaves the run going: its Stop stays, and it ends as it would.
    await api.cancelGeneration(id).catch(() => {});
  }, [api]);

  /** What the run that wrote `generationId` included (none for one it didn't write). */
  const sentInclude = useCallback((generationId: string) => sent.get(generationId) ?? [], [sent]);

  return { state, start, stop, sentInclude };
}

/**
 * The last 5 explanations of the course's week, newest first. Without a week, those of the
 * recent materials: the facade answers null with every week's (each with its staleness worked
 * out), so the rest are left out here. That costs a read of every week, only while a course has
 * no week; a facade call for the recent ones alone would save it.
 */
export function useSavedExplanations(courseId: string, week: number | null) {
  const api = useApi();
  return useQuery({
    queryKey: explainKeys.saved(courseId, week),
    queryFn: () => api.savedExplanations(courseId, week),
    select: (list) => (week === null ? list.filter((e) => e.week == null) : list),
  });
}

/** Deletes an explanation; the course's saved lists refresh. */
export function useDeleteExplanation(courseId: string) {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (generationId: string) => api.deleteExplanation(generationId),
    onSuccess: () =>
      client.invalidateQueries({ queryKey: [...queryKeys.all, "explanations", courseId] }),
  });
}

export function useOutputLanguage() {
  const api = useApi();
  return useQuery({ queryKey: explainKeys.language(), queryFn: () => api.aiOutputLanguage() });
}

export function useSetOutputLanguage() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (language: OutputLanguage) => api.setAiOutputLanguage(language),
    onSuccess: (_result, language) => client.setQueryData(explainKeys.language(), language),
  });
}
