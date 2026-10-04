import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/api/errors";
import type { SourceErrorKind, SyncSummary } from "@/api/types";
import { afterCurrentRun, useSyncStore } from "./sync";

/** A run of one source, "a", that ended with `kind` (null: it synced). */
function summaryOfA(kind: SourceErrorKind | null): SyncSummary {
  return {
    started_at: "2026-10-05T13:00:00Z",
    finished_at: "2026-10-05T13:00:01Z",
    ok: kind === null,
    results: [
      {
        source_id: "a",
        label: "Demo Canvas",
        kind: "canvas",
        ok: kind === null,
        error: kind === null ? null : "Synthetic failure",
        error_kind: kind,
        started_at: "2026-10-05T13:00:00Z",
        finished_at: "2026-10-05T13:00:01Z",
        courses: 1,
        modules: 0,
        materials: 0,
        files_downloaded: 0,
        files_indexed: 0,
        events: 0,
        warnings: [],
        course_summaries: [],
      },
    ],
  };
}

/** An automatic run of source "a" up to its end. */
function automaticRun(kind: SourceErrorKind | null) {
  const store = useSyncStore.getState();
  store.begin(1, null, "unattended");
  store.apply({ type: "source_started", source_id: "a", label: "Demo Canvas" });
  store.apply({
    type: "source_finished",
    source_id: "a",
    ok: kind === null,
    error: kind === null ? null : "Synthetic failure",
    error_kind: kind,
  });
}

const NO_RUN = { running: false, runError: null, lastSummary: null, order: [], automatic: null };

describe("sync store", () => {
  it("folds SyncEvents into per-source progress", () => {
    const store = useSyncStore.getState();
    store.begin(2);
    store.apply({ type: "source_started", source_id: "a", label: "Course folder" });
    store.apply({ type: "progress", source_id: "a", message: "Indexing", current: 2, total: 5 });
    store.apply({ type: "warning", source_id: "a", message: "Skipped a video" });
    store.apply({ type: "source_started", source_id: "b", label: "Demo Canvas" });
    store.apply({
      type: "source_finished",
      source_id: "b",
      ok: false,
      error: "401",
      error_kind: "auth_expired_or_revoked",
    });

    const s = useSyncStore.getState();
    expect(s.running).toBe(true);
    expect(s.total).toBe(2);
    expect(s.order).toEqual(["a", "b"]);
    expect(s.bySource.a).toMatchObject({
      label: "Course folder",
      current: 2,
      total: 5,
      warnings: ["Skipped a video"],
      result: null,
    });
    expect(s.bySource.b?.result).toEqual({
      ok: false,
      error: "401",
      errorKind: "auth_expired_or_revoked",
    });
  });

  it("marks sources that never finished as stopped when the run ends", () => {
    const store = useSyncStore.getState();
    store.begin(2);
    store.apply({ type: "source_started", source_id: "a", label: "Course folder" });
    store.apply({ type: "source_finished", source_id: "a", ok: true });
    store.apply({ type: "source_started", source_id: "b", label: "Demo Canvas" });
    store.finish(null, new ApiError("internal", "boom"));

    const s = useSyncStore.getState();
    expect(s.running).toBe(false);
    expect(s.bySource.a?.stopped).toBe(false);
    expect(s.bySource.b?.stopped).toBe(true);
  });

  it("keeps a dismissed run error dismissed until the next run", () => {
    const store = useSyncStore.getState();
    store.begin(1);
    store.finish(null, new ApiError("network", "offline"));
    store.dismissRunError();
    expect(useSyncStore.getState().runError).toBeNull();
    store.begin(1);
    store.finish(null, new ApiError("busy", "locked"));
    expect(useSyncStore.getState().runError?.kind).toBe("busy");
  });
});

afterEach(() => {
  vi.useRealTimers();
});

describe("a sync PageLamp started by itself", () => {
  it("counts the student as here for 30 seconds, and holds the next start for 30 minutes", () => {
    const now = new Date(2026, 9, 5, 9, 0).getTime();
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(now);
    const store = useSyncStore.getState();

    store.noteStudentAction();
    expect(useSyncStore.getState().attendedUntil).toBe(now + 30_000);

    store.begin(1, null, "unattended");
    expect(useSyncStore.getState().noAutomaticBefore).toBe(now + 1_800_000);
    store.finish(null, new ApiError("busy", "locked"));

    // The student's Stop holds it the same.
    vi.setSystemTime(now + 3_600_000);
    store.begin(1);
    store.finish(null, new ApiError("cancelled", "Cancelled"));
    expect(useSyncStore.getState().noAutomaticBefore).toBe(now + 3_600_000 + 1_800_000);
  });

  it("leaves the last run's result alone until it has something to show", () => {
    const store = useSyncStore.getState();
    // The student's sync failed: a row, and the reason for the whole run.
    store.begin(2);
    store.apply({ type: "source_started", source_id: "b", label: "Course folder" });
    store.finish(null, new ApiError("network", "offline"));
    const before = useSyncStore.getState();
    expect(before.runError?.kind).toBe("network");

    // An automatic run begins and is refused: nothing of the old result is touched.
    store.begin(3, null, "unattended");
    expect(useSyncStore.getState()).toMatchObject({ running: true, started: false });
    expect(useSyncStore.getState().runError).toBe(before.runError);
    expect(useSyncStore.getState().bySource).toBe(before.bySource);
    store.finish(null, new ApiError("busy", "locked"));
    expect(useSyncStore.getState()).toMatchObject({ running: false, automatic: null });
    expect(useSyncStore.getState().runError).toBe(before.runError);
    expect(useSyncStore.getState().order).toEqual(["b"]);

    // The next one gets going: from its first event it is the run on screen.
    store.begin(3, null, "unattended");
    store.apply({ type: "source_started", source_id: "a", label: "Demo Canvas" });
    expect(useSyncStore.getState()).toMatchObject({ started: true, runError: null, order: ["a"] });
  });

  it("ends like any other when every source synced", () => {
    automaticRun(null);
    useSyncStore.getState().finish(summaryOfA(null), null);
    const s = useSyncStore.getState();
    expect(s.lastSummary?.ok).toBe(true);
    expect(s.automatic).toBe("unattended");
    expect(s.order).toEqual(["a"]);
    expect(s.noAutomaticBefore).toBeGreaterThan(Date.now());
  });

  it("leaves nothing behind when it is refused, or wasn't due any more", () => {
    const store = useSyncStore.getState();
    store.begin(3, null, "attended");
    store.finish(null, new ApiError("busy", "locked"));
    expect(useSyncStore.getState()).toMatchObject({ ...NO_RUN, automaticProblem: false });
    // The start still counts for "not again so soon".
    expect(useSyncStore.getState().noAutomaticBefore).toBeGreaterThan(Date.now());

    store.begin(3, null, "unattended");
    store.finish(
      {
        started_at: "2026-10-05T13:00:00Z",
        finished_at: "2026-10-05T13:00:00Z",
        ok: true,
        results: [],
      },
      null,
    );
    expect(useSyncStore.getState()).toMatchObject({ ...NO_RUN, automaticProblem: false });
  });

  it("leaves nothing behind when a source failed, and notes what the facade recorded", () => {
    // Which failures are worth telling the student is the facade's call (it records them on
    // the source); the store doesn't judge by the kind of failure.
    automaticRun("auth_expired_or_revoked");
    useSyncStore.getState().finish(summaryOfA("auth_expired_or_revoked"), null);
    expect(useSyncStore.getState()).toMatchObject({ ...NO_RUN, automaticProblem: false });

    automaticRun("other");
    useSyncStore.getState().finish(summaryOfA("other"), null, true);
    expect(useSyncStore.getState()).toMatchObject({ ...NO_RUN, automaticProblem: true });

    // The next run starts clean.
    useSyncStore.getState().begin(1);
    expect(useSyncStore.getState().automaticProblem).toBe(false);
  });

  it("ends like any other when the student watches it, or stops it", () => {
    useSyncStore.setState({ watched: true });
    automaticRun("network");
    useSyncStore.getState().finish(summaryOfA("network"), null);
    expect(useSyncStore.getState().lastSummary?.ok).toBe(false);
    expect(useSyncStore.getState().order).toEqual(["a"]);

    useSyncStore.setState({ watched: false });
    const store = useSyncStore.getState();
    store.begin(1, null, "unattended");
    store.apply({ type: "source_started", source_id: "a", label: "Demo Canvas" });
    store.finish(null, new ApiError("cancelled", "Cancelled"));
    expect(useSyncStore.getState().stoppedByUser).toBe(true);
    expect(useSyncStore.getState().order).toEqual(["a"]);
  });

  it("keeps a manual run's failure, as before", () => {
    const store = useSyncStore.getState();
    store.begin(1);
    store.finish(null, new ApiError("busy", "locked"));
    expect(useSyncStore.getState().runError?.kind).toBe("busy");
    expect(useSyncStore.getState().noAutomaticBefore).toBe(0);
  });

  it("is quiet when refused even while the student is on an older run's capsule", () => {
    // Watching what is on screen isn't watching a run that never showed anything.
    useSyncStore.setState({ watched: true });
    const store = useSyncStore.getState();
    store.begin(3, null, "unattended");
    store.finish(null, new ApiError("busy", "locked"));
    expect(useSyncStore.getState()).toMatchObject(NO_RUN);
  });

  it("holds automatic starts for a while after the student stopped a sync", () => {
    const store = useSyncStore.getState();
    store.begin(1);
    store.apply({ type: "source_started", source_id: "a", label: "Demo Canvas" });
    store.finish(null, new ApiError("cancelled", "Cancelled"));
    expect(useSyncStore.getState().noAutomaticBefore).toBeGreaterThan(Date.now());
  });

  it("'Hide' clears the run but not what an automatic sync needs to remember", () => {
    automaticRun(null);
    useSyncStore.getState().finish(summaryOfA(null), null);
    useSyncStore.getState().noteStudentAction();
    useSyncStore.getState().hideRun();
    const s = useSyncStore.getState();
    expect(s).toMatchObject(NO_RUN);
    expect(s.noAutomaticBefore).toBeGreaterThan(Date.now());
    expect(s.attendedUntil).toBeGreaterThan(Date.now());
  });
});

describe("afterCurrentRun", () => {
  it("runs at once when nothing is running, else once after the run has ended", async () => {
    const now = vi.fn();
    afterCurrentRun(now);
    expect(now).toHaveBeenCalledTimes(1);

    const store = useSyncStore.getState();
    store.begin(1);
    // Whoever shows the run sees it end first: the action isn't run inside that notification.
    const running: boolean[] = [];
    const seenByOthers: boolean[] = [];
    afterCurrentRun(() => running.push(useSyncStore.getState().running));
    const unsubscribe = useSyncStore.subscribe((s) => seenByOthers.push(s.running));
    store.apply({ type: "source_started", source_id: "a", label: "Demo Canvas" });
    store.finish(null, null);
    expect(running).toEqual([]);
    expect(seenByOthers.at(-1)).toBe(false);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(running).toEqual([false]);

    store.begin(1);
    store.finish(null, null);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(running).toEqual([false]);
    unsubscribe();
  });
});
