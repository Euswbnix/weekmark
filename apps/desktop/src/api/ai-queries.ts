// TanStack Query hooks for the AI setup (M1), kept apart from queries.ts so that file stays
// readable. Same rules: screens read through these hooks, keys never hold a secret, and API keys
// travel only as mutation variables with `gcTime: 0` (dropped as soon as the call settles).

import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import type { CodexSource } from "./ai";
import {
  type AiFeature,
  type BackendRef,
  backendKey,
  type EstimateRequest,
  type MaterialSharing,
  type ModelChoice,
} from "./ai";
import { useApi } from "./context";
import { queryKeys } from "./queries";
import type { IsoDate } from "./types";

export const aiKeys = {
  all: [...queryKeys.all, "ai"] as const,
  status: () => [...aiKeys.all, "status"] as const,
  presets: () => [...aiKeys.all, "presets"] as const,
  localServers: () => [...aiKeys.all, "local-servers"] as const,
  models: (backend: BackendRef) => [...aiKeys.all, "models", backendKey(backend)] as const,
  estimate: (req: EstimateRequest) => [...aiKeys.all, "estimate", req] as const,
  usage: (month: IsoDate | null) => [...aiKeys.all, "usage", month] as const,
  codex: () => [...aiKeys.all, "codex"] as const,
};

function useInvalidateAi() {
  const client = useQueryClient();
  return () => client.invalidateQueries({ queryKey: aiKeys.all });
}

// ----- reads ----------------------------------------------------------------------------------

export function useAiStatus() {
  const api = useApi();
  return useQuery({ queryKey: aiKeys.status(), queryFn: () => api.aiStatus() });
}

export function useModelProviderPresets() {
  const api = useApi();
  return useQuery({
    queryKey: aiKeys.presets(),
    queryFn: () => api.modelProviderPresets(),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** Ollama / LM Studio on this computer; asked again whenever a screen that shows them mounts. */
export function useLocalServers() {
  const api = useApi();
  return useQuery({
    queryKey: aiKeys.localServers(),
    queryFn: () => api.detectLocalServers(),
    refetchOnMount: "always",
  });
}

/** The live model list of a backend (a network call): no automatic retries, show the error. */
export function useModels(backend: BackendRef | null) {
  const api = useApi();
  return useQuery({
    queryKey: backend ? aiKeys.models(backend) : [...aiKeys.all, "models", null],
    queryFn: () => api.listModels(backend as BackendRef),
    enabled: backend !== null,
    retry: false,
    staleTime: 5 * 60 * 1000,
  });
}

/** The model lists of several backends at once (one query each, same cache as useModels). */
export function useBackendModels(backends: BackendRef[]) {
  const api = useApi();
  return useQueries({
    queries: backends.map((backend) => ({
      queryKey: aiKeys.models(backend),
      queryFn: () => api.listModels(backend),
      retry: false,
      staleTime: 5 * 60 * 1000,
    })),
  });
}

/** `value`, once it stopped changing for `ms`. */
export function useDebouncedValue<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return settled;
}

/**
 * "≈ $x" before Generate. Debounced (the request changes as the student edits the form); the
 * previous estimate stays on screen meanwhile, and `settling` says it is still the previous
 * request's: the request changed and its own estimate hasn't arrived (a run must not start from
 * it). A refetch of the same request keeps its estimate and isn't settling. `req` null = nothing
 * to estimate yet.
 */
export function useCostEstimate(req: EstimateRequest | null) {
  const api = useApi();
  // Keyed by content, so a new object with the same fields doesn't restart the debounce.
  const current = req ? JSON.stringify(req) : null;
  const settled = useDebouncedValue(current, 300);
  const request = settled ? (JSON.parse(settled) as EstimateRequest) : null;
  const query = useQuery({
    queryKey: request ? aiKeys.estimate(request) : [...aiKeys.all, "estimate", null],
    queryFn: () => api.estimateGeneration(request as EstimateRequest),
    enabled: request !== null,
    placeholderData: (previous) => previous,
  });
  return { ...query, settling: current !== settled || query.isPlaceholderData };
}

/** `month` = the first day of a month; null = this month. */
export function useUsageSummary(month: IsoDate | null) {
  const api = useApi();
  return useQuery({
    queryKey: aiKeys.usage(month),
    queryFn: () => api.usageSummary(month),
    placeholderData: (previous) => previous,
  });
}

// ----- writes ---------------------------------------------------------------------------------

export function useAddModelProvider() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({
    mutationFn: (v: { preset: string; baseUrl: string | null; apiKey: string | null }) =>
      api.addModelProvider(v.preset, v.baseUrl, v.apiKey),
    onSuccess: invalidate,
    gcTime: 0,
  });
}

export function useUpdateModelProviderKey() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({
    mutationFn: (v: { providerId: string; apiKey: string }) =>
      api.updateModelProviderKey(v.providerId, v.apiKey),
    onSuccess: invalidate,
    gcTime: 0,
  });
}

export function useRemoveModelProvider() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({
    mutationFn: (providerId: string) => api.removeModelProvider(providerId),
    onSuccess: invalidate,
  });
}

/** "Test": a tiny real call to the model. Not cached; the result shows next to the button. */
export function useTestModel() {
  const api = useApi();
  return useMutation({
    mutationFn: (v: { backend: BackendRef; model: string }) => api.testModel(v.backend, v.model),
  });
}

export function useSetFeatureModel() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({
    mutationFn: (v: { feature: AiFeature; choice: ModelChoice | null }) =>
      api.setFeatureModel(v.feature, v.choice),
    onSuccess: invalidate,
  });
}

export function useAcknowledgeAiDisclosure() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({
    mutationFn: (v: { backend: BackendRef; version: number }) =>
      api.acknowledgeAiDisclosure(v.backend, v.version),
    onSuccess: invalidate,
  });
}

export function useAcknowledgeUnpricedModel() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({
    mutationFn: (v: { backend: BackendRef; model: string }) =>
      api.acknowledgeUnpricedModel(v.backend, v.model),
    onSuccess: invalidate,
  });
}

export function useSetMonthlyBudget() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({
    mutationFn: (microUsd: number | null) => api.setMonthlyBudget(microUsd),
    onSuccess: invalidate,
  });
}

/** Question (b). Changes the course views too, so it refreshes everything. */
export function useSetCourseMaterialSharing() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (v: { courseId: string; answer: MaterialSharing }) =>
      api.setCourseMaterialSharing(v.courseId, v.answer),
    onSuccess: () => client.invalidateQueries({ queryKey: queryKeys.all }),
  });
}

export function useRemoveAllAiData() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: () => api.removeAllAiData(),
    onSuccess: () => client.invalidateQueries({ queryKey: queryKeys.all }),
  });
}

// ----- mode A: the ChatGPT plan through Codex (M2) --------------------------------------------
// Installing (progress that outlives the screen) lives in stores/codex.ts; signing in lives in
// its dialog, whose closing cancels it.

export function useCodexStatus() {
  const api = useApi();
  return useQuery({ queryKey: aiKeys.codex(), queryFn: () => api.codexStatus() });
}

export function useCodexLogout() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({ mutationFn: () => api.codexLogout(), onSuccess: invalidate });
}

export function useRemoveCodex() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({ mutationFn: () => api.removeCodex(), onSuccess: invalidate });
}

export function useSetCodexSource() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({
    mutationFn: (source: CodexSource) => api.setCodexSource(source),
    onSuccess: invalidate,
  });
}

export function useSetModeAWeeklyCap() {
  const api = useApi();
  const invalidate = useInvalidateAi();
  return useMutation({
    mutationFn: (runs: number | null) => api.setModeAWeeklyCap(runs),
    onSuccess: invalidate,
  });
}
