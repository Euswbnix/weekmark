import { useSyncStore } from "@/stores/sync";

/**
 * Where the latest sync run stands, derived from the live sync store:
 * - idle: nothing has run since the app started (or the result was dismissed)
 *
 * An automatic sync counts from its first event. Before that the outcome is still the last
 * run's, so a start the facade refuses (another sync took the lock, it isn't due any more)
 * never shows or is announced, and never wipes a failure the student hasn't dealt with.
 * - failed: the whole run stopped (e.g. `busy`); per-source failures are `doneWithErrors`
 * - stopped: the student stopped it (Stop); not a failure
 */
export type SyncOutcome = "idle" | "running" | "done" | "doneWithErrors" | "failed" | "stopped";

export function useSyncOutcome(): SyncOutcome {
  const runError = useSyncStore((s) => s.runError);
  const summary = useSyncStore((s) => s.lastSummary);
  const stoppedByUser = useSyncStore((s) => s.stoppedByUser);
  // An automatic sync shows from its first event; until then the last run's result stands.
  const shown = useSyncStore((s) => s.running && s.started);
  const anyFailed = useSyncStore((s) =>
    Object.values(s.bySource).some((p) => p.result !== null && !p.result.ok),
  );
  if (shown) return "running";
  if (stoppedByUser) return "stopped";
  if (runError) return "failed";
  if (summary) return summary.ok && !anyFailed ? "done" : "doneWithErrors";
  return "idle";
}
