// Query hooks for course calendar proposals, candidates and syllabus reading (calendar design
// §7; F3). Beside queries.ts so the course lane's additions stay in their own file; the keys
// live under `queryKeys.all`, so every sync and every mutation refreshes them.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useApi } from "./context";
import { queryKeys } from "./queries";
import type { CourseDatesInput, SyncEvent } from "./types";

export const proposalKeys = {
  courseCalendar: (courseId: string) => [...queryKeys.all, "course-calendar", courseId] as const,
  syllabusOffers: () => [...queryKeys.all, "syllabus-offers"] as const,
};

function useInvalidateAll() {
  const client = useQueryClient();
  return () => client.invalidateQueries({ queryKey: queryKeys.all });
}

export function useCourseCalendar(courseId: string, enabled = true) {
  const api = useApi();
  return useQuery({
    queryKey: proposalKeys.courseCalendar(courseId),
    queryFn: () => api.courseCalendar(courseId),
    enabled,
  });
}

export function useSyllabusReadingOffers(enabled = true) {
  const api = useApi();
  return useQuery({
    queryKey: proposalKeys.syllabusOffers(),
    queryFn: () => api.syllabusReadingOffers(),
    enabled,
  });
}

/**
 * "Not now" on the syllabus reading offers: 14 days for the courses offered now, like the
 * lifecycle banner. The facade then offers none of them (the card and startup_tasks alike).
 */
export function useSnoozeCalendarOffers() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: () => api.snoozeCalendarOffers(),
    onSuccess: () =>
      Promise.all([
        client.invalidateQueries({ queryKey: proposalKeys.syllabusOffers() }),
        client.invalidateQueries({ queryKey: queryKeys.startupTasks() }),
      ]),
  });
}

export function useSetCalendarSources() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { courseId: string; include: string[]; exclude: string[] }) =>
      api.setCalendarSources(v.courseId, v.include, v.exclude),
    onSuccess: invalidate,
  });
}

export function useDownloadMaterialFiles() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: {
      courseId: string;
      materialIds: string[];
      onEvent?: (event: SyncEvent) => void;
    }) => api.downloadMaterialFiles(v.courseId, v.materialIds, v.onEvent ?? (() => {})),
    onSuccess: invalidate,
  });
}

export function useScanCourseCalendar() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { courseId: string }) => api.scanCourseCalendar(v.courseId),
    onSuccess: invalidate,
  });
}

export function useAcceptCalendarProposal() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { proposalId: number; edits: CourseDatesInput | null }) =>
      api.acceptCalendarProposal(v.proposalId, v.edits),
    onSuccess: invalidate,
  });
}

export function useAcceptPassingProposals() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { proposalIds: number[] }) => api.acceptPassingProposals(v.proposalIds),
    onSuccess: invalidate,
  });
}

export function useDismissCalendarProposal() {
  const api = useApi();
  const invalidate = useInvalidateAll();
  return useMutation({
    mutationFn: (v: { proposalId: number }) => api.dismissCalendarProposal(v.proposalId),
    onSuccess: invalidate,
  });
}
