// TanStack Query hooks — the way screens read and change data. Screens never call `useApi()`
// methods directly for reads; they use these hooks so caching and invalidation stay consistent.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import { useApi } from "./context";
import type { AiPolicy, IsoDate, StartupTasks, StoredStudyPlan, UpdatePrefs } from "./types";

export const queryKeys = {
  all: ["pagelamp"] as const,
  status: () => [...queryKeys.all, "status"] as const,
  activity: () => [...queryKeys.all, "activity"] as const,
  sources: () => [...queryKeys.all, "sources"] as const,
  courses: () => [...queryKeys.all, "courses"] as const,
  course: (courseId: string) => [...queryKeys.all, "course", courseId] as const,
  week: (courseId: string, week: number | null) =>
    [...queryKeys.all, "course", courseId, "week", week] as const,
  deadlines: (courseId: string | null, daysAhead: number, daysBack: number) =>
    [...queryKeys.all, "deadlines", courseId, daysAhead, daysBack] as const,
  studyPlan: () => [...queryKeys.all, "study-plan"] as const,
  planLimits: () => [...queryKeys.all, "plan-limits"] as const,
  mcpConfigs: () => [...queryKeys.all, "mcp-configs"] as const,
  lastCrash: () => [...queryKeys.all, "last-crash"] as const,
  doctor: () => [...queryKeys.all, "doctor"] as const,
  // Deliberately outside `all`: a sync finishing (which invalidates `all`) must not swap the
  // text the student is reviewing before they copy it.
  diagnosticReport: () => ["diagnostic-report"] as const,
  updatePrefs: () => [...queryKeys.all, "update-prefs"] as const,
  updateChannel: () => [...queryKeys.all, "update-channel"] as const,
  lastUpdateCheck: () => [...queryKeys.all, "last-update-check"] as const,
  // Once per launch, outside `all`: a sync finishing must not bring back "What's new" or start
  // another automatic check.
  startupTasks: () => ["startup-tasks"] as const,
  updaterStatus: () => ["updater-status"] as const,
};

// ----- reads ----------------------------------------------------------------------------------

export function useStatus() {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.status(),
    queryFn: () => api.status(),
    // While another process (e.g. the CLI) is syncing, poll so "busy" clears by itself.
    refetchInterval: (query) => (query.state.data?.sync_in_progress ? 3000 : false),
  });
}

/**
 * What the app is doing (App::activity), polled while `watching`: the install dialog, so
 * "Install and restart" comes back by itself when a reading or a Codex download ends.
 */
export function useActivity(watching: boolean) {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.activity(),
    queryFn: () => api.activity(),
    enabled: watching,
    refetchInterval: watching ? 2000 : false,
  });
}

export function useSources() {
  const api = useApi();
  return useQuery({ queryKey: queryKeys.sources(), queryFn: () => api.listSources() });
}

/** All courses including hidden ones — filter on `course.hidden` in the UI. */
export function useCourses() {
  const api = useApi();
  return useQuery({ queryKey: queryKeys.courses(), queryFn: () => api.listCourses() });
}

export function useCourseOverview(courseId: string) {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.course(courseId),
    queryFn: () => api.courseOverview(courseId),
  });
}

/** `week` null = the course's current week. */
export function useWeekMaterials(courseId: string, week: number | null) {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.week(courseId, week),
    queryFn: () => api.weekMaterials(courseId, week),
    placeholderData: (previous) => previous,
  });
}

/** `day` (e.g. from useToday) only keys the cache, so the window moves on at midnight. */
export function useDeadlines(
  courseId: string | null,
  daysAhead: number,
  daysBack = 0,
  day?: string,
) {
  const api = useApi();
  return useQuery({
    queryKey: [...queryKeys.deadlines(courseId, daysAhead, daysBack), day ?? null],
    queryFn: () => api.listDeadlines(courseId, daysAhead, daysBack),
  });
}

export function useStudyPlan() {
  const api = useApi();
  return useQuery({ queryKey: queryKeys.studyPlan(), queryFn: () => api.latestStudyPlan() });
}

/** The facade's limits on a study plan request (fixed for the app's life: asked once). */
export function usePlanLimits() {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.planLimits(),
    queryFn: () => api.planLimits(),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/** Ticks a plan item off or back on (M3), shown at once and put back if saving fails. */
export function useSetStudyPlanItemDone() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (v: { planId: number; itemIndex: number; done: boolean }) =>
      api.setStudyPlanItemDone(v.planId, v.itemIndex, v.done),
    onMutate: async ({ itemIndex, done }) => {
      await client.cancelQueries({ queryKey: queryKeys.studyPlan() });
      const previous = client.getQueryData<StoredStudyPlan | null>(queryKeys.studyPlan());
      if (previous) {
        const items = previous.plan.items.map((item, i) =>
          i === itemIndex ? { ...item, done } : item,
        );
        client.setQueryData(queryKeys.studyPlan(), {
          ...previous,
          plan: { ...previous.plan, items },
        });
      }
      return { previous };
    },
    onSuccess: (stored) => client.setQueryData(queryKeys.studyPlan(), stored),
    onError: (_error, _v, context) => client.setQueryData(queryKeys.studyPlan(), context?.previous),
  });
}

export function useMcpClientConfigs() {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.mcpConfigs(),
    queryFn: () => api.mcpClientConfigs(),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/**
 * The AI app saves study plans (and the CLI may sync) while PageLamp's window is in the
 * background, so refresh what they change when the student comes back to the window.
 * Queries refetch only if something shows them. Mount once, in the app shell.
 */
export function useRefreshOnWindowFocus() {
  const api = useApi();
  const client = useQueryClient();
  useEffect(
    () =>
      api.onWindowFocus(() => {
        // The weekly note's estimate too (ai-queries' aiKeys.estimate key, spelled out: importing
        // it here would be circular): a sync elsewhere can give a week with nothing to write
        // about something to write about, and only that one keeps Write off. The other estimates
        // stay as they are, so their buttons don't change on every focus.
        const noteEstimate = [...queryKeys.all, "ai", "estimate", { feature: "weekly_note" }];
        for (const queryKey of [
          queryKeys.studyPlan(),
          queryKeys.courses(),
          queryKeys.status(),
          noteEstimate,
        ]) {
          void client.invalidateQueries({ queryKey });
        }
      }),
    [api, client],
  );
}

/** The setup check; refreshed after syncs (unreadable-file counts change). */
export function useDoctor() {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.doctor(),
    queryFn: () => api.doctor(),
    staleTime: 60_000,
  });
}

/** What the panic hook recorded last time (null = nothing to report). */
export function useLastCrash() {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.lastCrash(),
    queryFn: () => api.lastCrash(),
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/**
 * The diagnostic report, fetched while `enabled` (the preview is open). `gcTime: 0` drops it as
 * soon as the preview closes, so every preview shows a fresh report and none stays in memory.
 */
export function useDiagnosticReport(enabled: boolean) {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.diagnosticReport(),
    queryFn: () => api.diagnosticReport(),
    enabled,
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 0,
  });
}

// ----- writes ---------------------------------------------------------------------------------
//
// Secrets (tokens, feed URLs) are passed straight through as mutation variables and are never
// put in a query key or cache. `gcTime: 0` drops the mutation (and its variables) from the
// mutation cache as soon as it settles.
//
// Every write invalidates all queries and awaits the refetch in onSuccess. If a component
// remounts on that refetch (e.g. a form keyed on saved values), per-call
// `mutate(vars, { onSuccess })` callbacks never fire — use `await mutateAsync(vars)` inside
// try/catch and show the toast afterwards.

function useInvalidateAll() {
  const client = useQueryClient();
  return () => client.invalidateQueries({ queryKey: queryKeys.all });
}

export function useAddCanvasSource() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { baseUrl: string; token: string }) => api.addCanvasSource(v.baseUrl, v.token),
    onSuccess: invalidate,
    gcTime: 0,
  });
}

export function useAddFolderSource() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { path: string; termStart: IsoDate | null; label: string | null }) =>
      api.addFolderSource(v.path, v.termStart, v.label),
    onSuccess: invalidate,
  });
}

export function useAddIcalSource() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { feedUrl: string; label: string | null }) =>
      api.addIcalSource(v.feedUrl, v.label),
    onSuccess: invalidate,
    gcTime: 0,
  });
}

export function useUpdateSourceSecret() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { sourceId: string; secret: string }) =>
      api.updateSourceSecret(v.sourceId, v.secret),
    onSuccess: invalidate,
    gcTime: 0,
  });
}

export function useRemoveSource() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (sourceId: string) => api.removeSource(sourceId),
    onSuccess: invalidate,
  });
}

export function useSetCoursePolicy() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { courseId: string; policy: AiPolicy; note: string | null }) =>
      api.setCoursePolicy(v.courseId, v.policy, v.note),
    onSuccess: invalidate,
  });
}

export function useSetCourseTerm() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { courseId: string; start: IsoDate | null; end: IsoDate | null }) =>
      api.setCourseTerm(v.courseId, v.start, v.end),
    onSuccess: invalidate,
  });
}

export function useSetCourseAiAccess() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { courseId: string; allowed: boolean }) =>
      api.setCourseAiAccess(v.courseId, v.allowed),
    onSuccess: invalidate,
  });
}

export function useSetCourseHidden() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { courseId: string; hidden: boolean }) =>
      api.setCourseHidden(v.courseId, v.hidden),
    onSuccess: invalidate,
  });
}

/** "I'm still taking this" (`until` null = the facade's default date). */
export function useKeepCourseCurrent() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { courseId: string; until: IsoDate | null }) =>
      api.keepCourseCurrent(v.courseId, v.until),
    onSuccess: invalidate,
  });
}

export function useClearKeepCourseCurrent() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { courseId: string }) => api.clearKeepCourseCurrent(v.courseId),
    onSuccess: invalidate,
  });
}

/** "These dates are right" for dates kept from version 0.1. */
export function useConfirmCourseDates() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { courseId: string }) => api.confirmCourseDates(v.courseId),
    onSuccess: invalidate,
  });
}

export function useClearLastCrash() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: () => api.clearLastCrash(),
    onSuccess: () => client.setQueryData(queryKeys.lastCrash(), null),
  });
}

// ----- updates (M0.4) ---------------------------------------------------------------------------

export function useUpdatePrefs() {
  const api = useApi();
  return useQuery({ queryKey: queryKeys.updatePrefs(), queryFn: () => api.updatePrefs() });
}

/** The channel actually used (the choice, else the default for this build). */
export function useEffectiveUpdateChannel() {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.updateChannel(),
    queryFn: () => api.effectiveUpdateChannel(),
  });
}

export function useSetUpdatePrefs() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (prefs: UpdatePrefs) => api.setUpdatePrefs(prefs),
    onSuccess: () =>
      Promise.all([
        client.invalidateQueries({ queryKey: queryKeys.updatePrefs() }),
        client.invalidateQueries({ queryKey: queryKeys.updateChannel() }),
      ]),
  });
}

export function useLastUpdateCheck() {
  const api = useApi();
  return useQuery({ queryKey: queryKeys.lastUpdateCheck(), queryFn: () => api.lastUpdateCheck() });
}

/**
 * What to do now (the facade decides, the UI only renders): asked at launch, then hourly while
 * PageLamp runs, even with the window hidden (it may live in the tray for days), and again after
 * each acknowledgement.
 */
export function useStartupTasks() {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.startupTasks(),
    queryFn: () => api.startupTasks(),
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: Number.POSITIVE_INFINITY,
    refetchInterval: 60 * 60 * 1000,
    refetchIntervalInBackground: true,
    retry: false,
  });
}

/** This build's version and how it updates; fixed for the whole launch. */
export function useUpdaterStatus() {
  const api = useApi();
  return useQuery({
    queryKey: queryKeys.updaterStatus(),
    queryFn: () => api.updaterStatus(),
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
}

/**
 * The student has read "What's new": it's done for good (and so is the update disclosure). The
 * sheet closes at once; then the facade is asked again whether an update check is now due.
 */
export function useAcknowledgeWhatsNew() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: () => api.acknowledgeWhatsNew(),
    onSuccess: () => {
      client.setQueryData<StartupTasks>(queryKeys.startupTasks(), (tasks) =>
        tasks ? { ...tasks, whats_new: null } : tasks,
      );
      return client.invalidateQueries({ queryKey: queryKeys.startupTasks() });
    },
  });
}

export function useAcknowledgeUpdateDisclosure() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: () => api.acknowledgeUpdateDisclosure(),
    onSuccess: () => client.invalidateQueries({ queryKey: queryKeys.startupTasks() }),
  });
}
