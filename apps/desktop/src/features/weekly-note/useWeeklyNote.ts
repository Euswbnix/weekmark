import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { create } from "zustand";
import type { GenStage } from "@/api/ai";
import { useApi } from "@/api/context";
import { toApiError } from "@/api/errors";
import { queryKeys, useStartupTasks } from "@/api/queries";
import type { StartupTasks } from "@/api/types";
import type { WeeklyNote } from "@/api/weeklyNote";
import { AI_SETUP_ENABLED } from "@/lib/features";

export const weeklyNoteKeys = {
  notes: () => [...queryKeys.all, "weekly-notes"] as const,
  settings: () => [...queryKeys.all, "weekly-note-settings"] as const,
};

export type NoteRunState =
  | { phase: "idle" }
  | {
      phase: "running";
      id: string;
      /** Monday's note, started by the app (no focus moves, no alert). */
      automatic: boolean;
      backend: string | null;
      model: string | null;
      stage: GenStage | null;
      stopping: boolean;
    }
  | { phase: "done"; note: WeeklyNote; automatic: boolean }
  | { phase: "stopped"; automatic: boolean }
  | { phase: "failed"; error: unknown };

interface NoteRunStore {
  run: NoteRunState;
  /** Monday's note wasn't prepared (blocked or failed): the card says why, until the next run. */
  automaticProblem: unknown;
  setRun: (run: NoteRunState | ((run: NoteRunState) => NoteRunState)) => void;
  setAutomaticProblem: (error: unknown) => void;
}

/**
 * The one weekly note run of the app, started by a click or by Monday's opt-in: the card shows
 * either, with its stages and Stop, wherever it's mounted. Leaving the page doesn't stop it (the
 * note is saved when it ends); only Stop does.
 */
export const useNoteRunStore = create<NoteRunStore>((set) => ({
  run: { phase: "idle" },
  automaticProblem: null,
  setRun: (run) => set((s) => ({ run: typeof run === "function" ? run(s.run) : run })),
  setAutomaticProblem: (automaticProblem) => set({ automaticProblem }),
}));

/** A note run is going (a click's or Monday's). */
export function noteRunInFlight(): boolean {
  return useNoteRunStore.getState().run.phase === "running";
}

/** Starts and stops the app's weekly note run. */
export function useWeeklyNoteRun() {
  const api = useApi();
  const client = useQueryClient();
  const run = useNoteRunStore((s) => s.run);

  const start = useCallback(
    async (options: { automatic: boolean; overrideBudget?: boolean; uiLanguage: string }) => {
      const store = useNoteRunStore.getState();
      if (store.run.phase === "running") return;
      const { automatic } = options;
      const id = crypto.randomUUID();
      store.setRun({
        phase: "running",
        id,
        automatic,
        backend: null,
        model: null,
        stage: null,
        stopping: false,
      });
      if (!automatic) store.setAutomaticProblem(null);
      // Only while this run is still the app's run: a run that ended or was replaced (the store
      // reset between tests, say) changes nothing when its events or its end arrive late.
      const isCurrent = (r: NoteRunState) => r.phase === "running" && r.id === id;
      const update = (change: (r: Extract<NoteRunState, { phase: "running" }>) => NoteRunState) =>
        store.setRun((r) => (r.phase === "running" && r.id === id ? change(r) : r));
      const end = (next: NoteRunState, problem?: unknown) => {
        if (!isCurrent(useNoteRunStore.getState().run)) return;
        store.setRun(next);
        if (problem !== undefined) store.setAutomaticProblem(problem);
      };
      try {
        const note = await api.writeWeeklyNote(
          id,
          {
            automatic,
            // Monday's note never goes over the budget (the facade refuses it anyway).
            override_budget: automatic ? false : (options.overrideBudget ?? false),
            ui_language: options.uiLanguage,
          },
          (event) => {
            if (event.type === "started") {
              const { backend_label, model } = event;
              update((r) => ({ ...r, backend: backend_label, model }));
            } else if (event.type === "stage") {
              const { stage } = event;
              update((r) => ({ ...r, stage }));
            }
          },
        );
        end({ phase: "done", note, automatic }, null);
      } catch (error) {
        const { kind, blocked } = toApiError(error);
        if (kind === "cancelled") {
          end({ phase: "stopped", automatic });
        } else if (automatic) {
          // Monday's note: no alert, no dialog. Not due any more, or nothing to write about (the
          // week emptied as the run started; the card's Write already says so), is nothing to
          // say; anything else leaves one line on the card.
          const quiet = kind === "invalid" || blocked === "nothing_to_write";
          end({ phase: "idle" }, quiet ? undefined : error);
        } else {
          end({ phase: "failed", error });
        }
      } finally {
        await client.invalidateQueries({ queryKey: weeklyNoteKeys.notes() });
      }
    },
    [api, client],
  );

  const stop = useCallback(async () => {
    const store = useNoteRunStore.getState();
    const current = store.run;
    // One cancel per run: a second click while it stops sends nothing.
    if (current.phase !== "running" || current.stopping) return;
    const { id } = current;
    store.setRun((r) => (r.phase === "running" && r.id === id ? { ...r, stopping: true } : r));
    // A failed cancel leaves the run going, and Stop can be pressed again.
    await api.cancelGeneration(id).catch(() => {
      store.setRun((r) => (r.phase === "running" && r.id === id ? { ...r, stopping: false } : r));
    });
  }, [api]);

  return { run, start, stop };
}

/**
 * Monday's note (the opt-in): whenever an answer of `startup_tasks` says to prepare it and no note
 * run is going, start one, automatic. The app can live in the tray for days and the answer is
 * asked again hourly, so each answer is handled once (the facade says so at most once a Monday).
 * Mount once, in the app shell.
 */
export function useWeeklyNotePreparation(enabled: boolean = AI_SETUP_ENABLED) {
  const tasks = useStartupTasks();
  const { start } = useWeeklyNoteRun();
  const { i18n } = useTranslation();
  const handled = useRef<StartupTasks | null>(null);
  const data = tasks.data;
  useEffect(() => {
    // Query results keep their identity when nothing changed, so each answer is handled once.
    if (!enabled || !data || handled.current === data) return;
    handled.current = data;
    if (!data.prepare_weekly_note || noteRunInFlight()) return;
    void start({ automatic: true, uiLanguage: i18n.language });
  }, [enabled, data, start, i18n.language]);
}

/** The last 5 notes, newest first. */
export function useWeeklyNotes() {
  const api = useApi();
  return useQuery({ queryKey: weeklyNoteKeys.notes(), queryFn: () => api.weeklyNotes() });
}

/** Deletes a note; the list refreshes. */
export function useDeleteWeeklyNote() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (generationId: string) => api.deleteWeeklyNote(generationId),
    onSuccess: () => client.invalidateQueries({ queryKey: weeklyNoteKeys.notes() }),
  });
}

/** "Prepare it when I open PageLamp on Monday", and whether the note's model allows it. */
export function useWeeklyNoteSettings() {
  const api = useApi();
  return useQuery({ queryKey: weeklyNoteKeys.settings(), queryFn: () => api.weeklyNoteSettings() });
}

export function useSetPrepareOnMonday() {
  const api = useApi();
  const client = useQueryClient();
  return useMutation({
    mutationFn: (on: boolean) => api.setPrepareWeeklyNoteOnMonday(on),
    onSuccess: (settings) => client.setQueryData(weeklyNoteKeys.settings(), settings),
  });
}
