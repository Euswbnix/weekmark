import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { useApi } from "@/api/context";
import { queryKeys, useStartupTasks, useStatus } from "@/api/queries";
import type { AutoSyncTrigger, StartupTasks } from "@/api/types";
import {
  ATTENDED_WINDOW_MS,
  AUTO_SYNC_MIN_GAP_MS,
  useStartSync,
  useSyncStore,
} from "@/stores/sync";
import { useUpdateStore } from "@/stores/updates";

/** Coming back to the window asks again once the last answer is this old. */
export const FOCUS_REREAD_MS = 10 * 60 * 1000;

function dialogOpen(): boolean {
  return document.querySelector('[role="dialog"], [role="alertdialog"]') !== null;
}

/**
 * PageLamp syncs by itself while it is open. The facade decides when one is due
 * (`startup_tasks.sync_due`: the setting, how old the data is, retries); this hook only asks and
 * starts the same sync as the Sync button, marked with what triggered it:
 *
 * - attended: the student just did something here (opened PageLamp, came back to its window,
 *   closed "What's new", changed the setting);
 * - unattended: the hourly re-read, always, also with the window in front; and a launch the
 *   student didn't see (the window started hidden), until the window first gains focus.
 *
 * It never starts while something else is going on (a sync, an update being installed, a dialog,
 * "What's new"); it asks again when that is over and lets the new answer decide. It never opens
 * anything, shows no message and takes no focus; a run that goes wrong stays quiet (stores/sync).
 * Nothing else may start a sync without the student: no link, argument or event.
 *
 * Mount once, in the app shell (so never during onboarding, which runs the first sync itself).
 */
export function useAutoSync() {
  const api = useApi();
  const client = useQueryClient();
  // Keeps the question asked: at launch, hourly, after each acknowledgement.
  useStartupTasks();
  const status = useStatus();
  const startSync = useStartSync();
  const running = useSyncStore((s) => s.running);
  const installing = useUpdateStore(
    (s) => s.install.phase !== "idle" && s.install.phase !== "failed",
  );
  const loaded = status.data !== undefined;
  const noSources = status.data?.sources.length === 0;
  const otherProcess = status.data?.sync_in_progress === true;

  // Answers are told apart by their number, not by what they say or when they came: an answer
  // equal to the one before it keeps its identity, and two can arrive in the same millisecond.
  const answers = useCallback(
    () => client.getQueryState(queryKeys.startupTasks())?.dataUpdateCount ?? 0,
    [client],
  );
  const [answer, setAnswer] = useState(answers);
  useEffect(() => {
    const [key] = queryKeys.startupTasks();
    const unsubscribe = client.getQueryCache().subscribe((event) => {
      if (
        event.type === "updated" &&
        event.action.type === "success" &&
        event.query.queryKey[0] === key
      ) {
        setAnswer(event.query.state.dataUpdateCount);
      }
    });
    setAnswer(answers());
    return unsubscribe;
  }, [client, answers]);

  // An answer already in the cache when the shell mounts (back from /welcome) was dealt with by
  // the shell that read it. With none, this is the launch: the student has just opened PageLamp.
  const [atMount] = useState(answer);
  const handled = useRef<{ count: number; whatsNew: boolean }>({
    count: atMount,
    whatsNew: false,
  });
  // A due answer found something in the way; ask again when it is gone.
  const waiting = useRef(false);
  const dialogs = useRef<MutationObserver | null>(null);
  // A dialog closed: look at what is in the way again.
  const [closed, setClosed] = useState(0);

  const reread = useCallback(
    () => client.invalidateQueries({ queryKey: queryKeys.startupTasks() }),
    [client],
  );

  useEffect(() => {
    // A window started hidden (at login) wasn't opened by the student.
    if (atMount === 0 && !api.startedHidden()) useSyncStore.getState().noteStudentAction();
    return () => dialogs.current?.disconnect();
  }, [atMount, api]);

  // The student comes back to the window: ask again if the answer is old, or if it said a sync
  // was due (the student's own sync may have changed that since).
  useEffect(
    () =>
      api.onWindowFocus(() => {
        useSyncStore.getState().noteStudentAction();
        const state = client.getQueryState<StartupTasks>(queryKeys.startupTasks());
        const due = state?.data?.sync_due;
        if (
          !state?.dataUpdatedAt ||
          Date.now() - state.dataUpdatedAt > FOCUS_REREAD_MS ||
          due?.attended ||
          due?.unattended
        ) {
          void reread();
        }
        // The setting may have been changed elsewhere (the command line).
        void client.invalidateQueries({ queryKey: queryKeys.syncPrefs() });
      }),
    [api, client, reread],
  );

  // biome-ignore lint/correctness/useExhaustiveDependencies: `closed` only re-runs the check.
  useEffect(() => {
    const data = client.getQueryData<StartupTasks>(queryKeys.startupTasks());
    // The total in the capsule and "no sources" come from the status: wait for it.
    if (!data || !loaded) return;
    const blocked = running || otherProcess || installing || dialogOpen();
    // Dialogs are portalled into <body>; nothing else says when the last one closes.
    const watchDialogs = () => {
      if (!dialogOpen() || dialogs.current) return;
      const observer = new MutationObserver(() => {
        if (dialogOpen()) return;
        observer.disconnect();
        dialogs.current = null;
        setClosed((n) => n + 1);
      });
      observer.observe(document.body, { childList: true });
      dialogs.current = observer;
    };
    const seen = handled.current;
    if (seen.count === answer) {
      // Nothing new was answered; at most, what was in the way is gone, or something else is
      // in the way now (a dialog opened while a sync was running).
      if (waiting.current && !blocked) {
        waiting.current = false;
        void reread();
      } else if (waiting.current) {
        watchDialogs();
      }
      return;
    }
    handled.current = { count: answer, whatsNew: !!data.whats_new };
    waiting.current = false;
    const store = useSyncStore.getState();
    // "Got it" closed What's new: the student is here.
    if (seen.whatsNew && !data.whats_new) store.noteStudentAction();
    if (data.whats_new || noSources) return;

    // Bounded both ways: a clock set back after the student's action mustn't keep it "just now".
    const left = useSyncStore.getState().attendedUntil - Date.now();
    const attended = left > 0 && left <= ATTENDED_WINDOW_MS;
    const trigger: AutoSyncTrigger | null =
      attended && data.sync_due.attended
        ? "attended"
        : data.sync_due.unattended
          ? "unattended"
          : null;
    if (!trigger) {
      // While any sync holds the lock the facade answers "not due", whatever is stale: ask
      // again when it has ended (a sync of one source leaves the others as old as they were).
      if (running || otherProcess) waiting.current = true;
      return;
    }
    if (blocked) {
      waiting.current = true;
      watchDialogs();
      return;
    }
    // A backstop next to the facade's own clock: not so soon after the last automatic start,
    // nor right after the student stopped a sync. (Bounded like the window above.)
    const hold = store.noAutomaticBefore - Date.now();
    if (hold > 0 && hold <= AUTO_SYNC_MIN_GAP_MS) return;
    // Afterwards the cached answer must stop saying "due".
    void startSync(undefined, { automatic: trigger }).then(reread);
  }, [
    answer,
    loaded,
    noSources,
    running,
    otherProcess,
    installing,
    closed,
    client,
    startSync,
    reread,
  ]);
}
