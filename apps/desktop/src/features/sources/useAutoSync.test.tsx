import { act, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PageLampApi } from "@/api/client";
import { ApiError } from "@/api/errors";
import { createMockApi, type MockOptions } from "@/api/mock";
import { queryKeys } from "@/api/queries";
import type { SourceErrorKind, SourceSyncResult, SyncEvent, SyncSummary } from "@/api/types";
import { useSyncStore } from "@/stores/sync";
import { useUpdateStore } from "@/stores/updates";
import { renderRoute } from "@/test/render";

const HOUR = 60 * 60 * 1000;
const MINUTE = 60 * 1000;
const CANVAS = "canvas:canvas.demo.test";
const DUE = { unattended: true, attended: true };

function mockApi(options: MockOptions = {}) {
  return createMockApi({ latencyMs: 0, syncStepMs: 0, ...options });
}

/** Long enough for an answer to arrive and a sync to start, if one were going to. */
const settle = (ms = 50) => new Promise((resolve) => setTimeout(resolve, ms));

/** The facade keeps saying a sync is due, whatever happened. */
function alwaysDue(api: PageLampApi) {
  const tasks = api.startupTasks.bind(api);
  api.startupTasks = async () => ({ ...(await tasks()), sync_due: DUE });
}

/** Only the date is faked: timers, and so `waitFor`, keep working. */
function startClock(): Date {
  const start = new Date(2026, 9, 5, 9, 0);
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(start);
  return start;
}

function later(start: Date, ms: number) {
  vi.setSystemTime(new Date(start.getTime() + ms));
}

function failedCanvas(kind: SourceErrorKind): SourceSyncResult {
  return {
    source_id: CANVAS,
    label: "Demo Canvas",
    kind: "canvas",
    ok: false,
    error: "Synthetic failure",
    error_kind: kind,
    started_at: "2026-10-05T13:00:00Z",
    finished_at: "2026-10-05T13:00:01Z",
    courses: 0,
    modules: 0,
    materials: 0,
    files_downloaded: 0,
    files_indexed: 0,
    events: 0,
    warnings: [],
    course_summaries: [],
  };
}

/** An automatic run in which Canvas fails with `kind`. */
function failingWith(api: PageLampApi, kind: SourceErrorKind) {
  return vi
    .spyOn(api, "syncAll")
    .mockImplementation(async (_req, onEvent: (event: SyncEvent) => void) => {
      onEvent({ type: "source_started", source_id: CANVAS, label: "Demo Canvas" });
      onEvent({
        type: "source_finished",
        source_id: CANVAS,
        ok: false,
        error: "Synthetic failure",
        error_kind: kind,
      });
      const summary: SyncSummary = {
        started_at: "2026-10-05T13:00:00Z",
        finished_at: "2026-10-05T13:00:01Z",
        ok: false,
        results: [failedCanvas(kind)],
      };
      return summary;
    });
}

const announced = () =>
  screen
    .getAllByRole("status")
    .map((s) => s.textContent)
    .join(" ");

function expectNoTrace() {
  expect(screen.queryByRole("button", { name: "Sync failed" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Sync finished with problems" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();
  expect(screen.queryByText("Sync failed")).toBeNull();
  expect(document.querySelector("[data-sonner-toast]")).toBeNull();
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.queryByRole("alertdialog")).toBeNull();
  expect(useSyncStore.getState()).toMatchObject({
    running: false,
    runError: null,
    lastSummary: null,
    order: [],
  });
}

afterEach(async () => {
  vi.useRealTimers();
  // A sync still in flight would end inside the next test, in the store they share.
  await waitFor(() => expect(useSyncStore.getState().running).toBe(false), { timeout: 5000 });
});

describe("automatic sync", () => {
  it("starts one attended sync at launch when one is due, and asks again afterwards", async () => {
    const api = mockApi({ scenario: "auto-sync-due" });
    const sync = vi.spyOn(api, "syncAll");
    const tasks = vi.spyOn(api, "startupTasks");
    renderRoute("/courses", { api });

    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "attended" });
    expect(await screen.findByRole("button", { name: "Sync finished" })).toBeInTheDocument();
    // No dialog, no message, and focus stays where it was.
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(document.querySelector("[data-sonner-toast]")).toBeNull();
    expect(document.body).toHaveFocus();

    // Once at launch, once after the run: the cached answer no longer says "due".
    await waitFor(() => expect(tasks).toHaveBeenCalledTimes(2));
    await settle();
    expect(sync).toHaveBeenCalledTimes(1);
    expect((await api.startupTasks()).sync_due).toEqual({ unattended: false, attended: false });
  });

  it("looks like any sync while it runs: the capsule names the source it is on", async () => {
    const api = mockApi({ scenario: "auto-sync-due" });
    const real = api.syncAll;
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    api.syncAll = async (req, onEvent: (event: SyncEvent) => void) => {
      onEvent({ type: "source_started", source_id: "folder:demo-courses", label: "Course folder" });
      await gate;
      return real(req, onEvent);
    };
    renderRoute("/courses", { api });

    // No count: only the facade knows which sources this run syncs.
    expect(await screen.findByRole("button", { name: "Syncing · Course folder" })).toBeVisible();
    expect(announced()).toContain("Syncing…");
    release();
    expect(await screen.findByRole("button", { name: "Sync finished" })).toBeInTheDocument();
  });

  it("does nothing when no sync is due", async () => {
    const api = mockApi();
    const sync = vi.spyOn(api, "syncAll");
    const tasks = vi.spyOn(api, "startupTasks");
    renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await waitFor(() => expect(tasks).toHaveBeenCalledTimes(1));
    await settle();
    expect(sync).not.toHaveBeenCalled();
    expect(tasks).toHaveBeenCalledTimes(1);
  });

  it("does nothing when automatic sync is off", async () => {
    const api = mockApi({ scenario: "auto-sync-due" });
    await api.setSyncPrefs({ auto_sync: "off" });
    const sync = vi.spyOn(api, "syncAll");
    renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await settle();
    expect(sync).not.toHaveBeenCalled();
  });

  it("starts an unattended sync from the hourly re-read, and no second one an hour later", async () => {
    const start = startClock();
    const api = mockApi();
    const sync = vi.spyOn(api, "syncAll");
    const { queryClient } = renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await settle();
    expect(sync).not.toHaveBeenCalled();

    // The sources were synced at 07:00; at 20:00 the re-read finds them 13 hours old.
    later(start, 11 * HOUR);
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    // Nobody is known to be at the app: the launch was 11 hours ago.
    expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "unattended" });
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false));

    later(start, 12 * HOUR);
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await settle();
    expect(sync).toHaveBeenCalledTimes(1);

    // Half a day on, the next one.
    later(start, 24 * HOUR);
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(2));
  });

  it("leaves the full sync for the student's return after the timer's light one", async () => {
    const start = startClock();
    const api = mockApi();
    const sync = vi.spyOn(api, "syncAll");
    const { queryClient } = renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await settle();

    later(start, 11 * HOUR);
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "unattended" });
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false));
    // Canvas was only read lightly, so a full sync is still due; the timer never runs that one.
    await settle();
    expect((await api.startupTasks()).sync_due).toEqual({ unattended: false, attended: true });
    expect(sync).toHaveBeenCalledTimes(1);

    // The student comes back, more than half an hour after that run.
    later(start, 11 * HOUR + 40 * MINUTE);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(2));
    expect(sync.mock.calls[1]?.[0]).toEqual({ automatic: "attended" });
  });

  it("asks again when the student comes back to the window, and that sync is attended", async () => {
    const start = startClock();
    const api = mockApi();
    const sync = vi.spyOn(api, "syncAll");
    const tasks = vi.spyOn(api, "startupTasks");
    renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await waitFor(() => expect(tasks).toHaveBeenCalledTimes(1));

    // A few minutes later the answer is still young: nothing is asked.
    later(start, 5 * MINUTE);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await settle();
    expect(tasks).toHaveBeenCalledTimes(1);

    later(start, 11 * HOUR);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "attended" });
  });

  it("never starts while the student's own sync runs, and asks again when it has ended", async () => {
    const start = startClock();
    // Slow enough to be caught in flight: while it runs the mock, like the facade, says "not due".
    const api = mockApi({ syncStepMs: 30 });
    const all = vi.spyOn(api, "syncAll");
    const tasks = vi.spyOn(api, "startupTasks");
    const { user, queryClient } = renderRoute("/sources", { api });
    await screen.findByRole("heading", { level: 2, name: "Course calendar" });

    // 13 hours on, the student syncs one source; the others stay as old as they were.
    later(start, 13 * HOUR);
    await user.click(screen.getByRole("button", { name: "Sync Course calendar" }));
    await waitFor(() => expect(useSyncStore.getState().running).toBe(true));
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    expect((await tasks.mock.results.at(-1)?.value)?.sync_due).toEqual({
      unattended: false,
      attended: false,
    });
    expect(all).not.toHaveBeenCalled();

    // It ends: PageLamp asks again, and now syncs the rest by itself.
    await waitFor(() => expect(all).toHaveBeenCalledTimes(1), { timeout: 3000 });
    expect(all.mock.calls[0]?.[0]).toEqual({ automatic: "unattended" });
  });

  it("doesn't start while another process syncs", async () => {
    const api = mockApi({ scenario: "busy" });
    alwaysDue(api);
    const sync = vi.spyOn(api, "syncAll");
    renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await settle();
    expect(sync).not.toHaveBeenCalled();
    expect(screen.queryByText("Sync failed")).toBeNull();
  });

  it("starts no second sync within half an hour, even after leaving the shell and coming back", async () => {
    const api = mockApi({ scenario: "auto-sync-due" });
    alwaysDue(api);
    const sync = vi.spyOn(api, "syncAll");
    const { router, queryClient } = renderRoute("/courses", { api });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false));
    await settle();
    expect(sync).toHaveBeenCalledTimes(1);

    // Without the half-hour hold, so only "this answer was dealt with" can stop a second one.
    act(() => useSyncStore.setState({ noAutomaticBefore: 0 }));
    await act(() => router.navigate("/welcome"));
    await screen.findByRole("heading", { level: 1, name: "Welcome to PageLamp" });
    await act(() => router.navigate("/courses"));
    await screen.findByRole("heading", { level: 1, name: "Courses" });
    await settle();
    // The answer in the cache was dealt with by the shell that read it.
    expect(sync).toHaveBeenCalledTimes(1);

    // And with the hold back, a new "due" answer finds the last start too recent.
    act(() => useSyncStore.setState({ noAutomaticBefore: Date.now() + 60_000 }));
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await settle();
    expect(sync).toHaveBeenCalledTimes(1);
  });

  it("keeps half an hour between two automatic starts: not at 29 minutes, again at 31", async () => {
    const start = startClock();
    const api = mockApi({ scenario: "auto-sync-due" });
    alwaysDue(api);
    const sync = vi.spyOn(api, "syncAll");
    const { queryClient } = renderRoute("/courses", { api });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false));
    await settle();

    later(start, 29 * MINUTE);
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await settle();
    expect(sync).toHaveBeenCalledTimes(1);

    later(start, 31 * MINUTE);
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(2));
  });

  it.each([
    [29, "attended"],
    [31, "unattended"],
  ] as const)(
    "counts an answer %i seconds after the student came back as %s",
    async (seconds, trigger) => {
      const start = startClock();
      const api = mockApi();
      const sync = vi.spyOn(api, "syncAll");
      const { queryClient } = renderRoute("/courses", { api });
      await screen.findByRole("heading", { level: 1 });
      await settle();

      // An hour on, the student comes back. The answer read then is young and nothing is due.
      later(start, HOUR);
      await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
      act(() => {
        window.dispatchEvent(new Event("focus"));
      });
      await settle();
      expect(sync).not.toHaveBeenCalled();

      alwaysDue(api);
      later(start, HOUR + seconds * 1000);
      await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
      await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
      expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: trigger });
    },
  );

  it("doesn't take a clock set back for the student still being here", async () => {
    const api = mockApi();
    const sync = vi.spyOn(api, "syncAll");
    const { queryClient } = renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await settle();

    // As after the clock went back ten minutes: both marks lie far in the future.
    act(() =>
      useSyncStore.setState({
        attendedUntil: Date.now() + 10 * MINUTE,
        noAutomaticBefore: Date.now() + 2 * HOUR,
      }),
    );
    alwaysDue(api);
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "unattended" });
  });

  it("asks from the hourly timer itself, also with the window out of sight", async () => {
    const start = new Date(2026, 9, 5, 9, 0);
    // The query's own timer is faked; the waits below use real ones.
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    vi.setSystemTime(start);
    const api = mockApi();
    const sync = vi.spyOn(api, "syncAll");
    const tasks = vi.spyOn(api, "startupTasks");
    renderRoute("/courses", { api });
    await vi.waitFor(() => expect(tasks).toHaveBeenCalledTimes(1));
    await settle();
    expect(sync).not.toHaveBeenCalled();

    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    try {
      // Ten hours on; then the hour that makes the timer fire, 13 hours after the last sync.
      vi.setSystemTime(new Date(start.getTime() + 10 * HOUR));
      await act(() => vi.advanceTimersByTimeAsync(HOUR));
      // Asked again by the timer (and once more after the run it started).
      await vi.waitFor(() => expect(tasks.mock.calls.length).toBeGreaterThanOrEqual(2));
      await vi.waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
      expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "unattended" });
    } finally {
      Reflect.deleteProperty(document, "visibilityState");
    }
  });

  it("doesn't take a start the student never saw for the student opening PageLamp", async () => {
    const start = startClock();
    const api = mockApi({ scenario: "auto-sync-due", startedHidden: true });
    const sync = vi.spyOn(api, "syncAll");
    renderRoute("/courses", { api });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "unattended" });
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false));
    await settle();
    expect(sync).toHaveBeenCalledTimes(1);

    // The first time the window gains focus is the student arriving.
    later(start, 40 * MINUTE);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(2));
    expect(sync.mock.calls[1]?.[0]).toEqual({ automatic: "attended" });
  });

  it("doesn't start one right after the student stopped a sync", async () => {
    // What Stop leaves in the store (stores/sync.test.ts): no automatic start for a while.
    useSyncStore.setState({ noAutomaticBefore: Date.now() + 60_000 });
    const api = mockApi({ scenario: "auto-sync-due" });
    const sync = vi.spyOn(api, "syncAll");
    renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await settle();
    expect(sync).not.toHaveBeenCalled();
  });

  it("picks the trigger from what is due: unattended at launch when only that is", async () => {
    const api = mockApi({ scenario: "auto-sync-due" });
    const tasks = api.startupTasks.bind(api);
    api.startupTasks = async () => ({
      ...(await tasks()),
      sync_due: { unattended: true, attended: false },
    });
    const sync = vi.spyOn(api, "syncAll");
    renderRoute("/courses", { api });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "unattended" });
  });

  it("never runs an attended sync from the hourly re-read", async () => {
    const start = startClock();
    const api = mockApi();
    const sync = vi.spyOn(api, "syncAll");
    const { queryClient } = renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await settle();

    // Only a full sync is due, and nobody is known to be here.
    const tasks = api.startupTasks.bind(api);
    api.startupTasks = async () => ({
      ...(await tasks()),
      sync_due: { unattended: false, attended: true },
    });
    later(start, 11 * HOUR);
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await settle();
    expect(sync).not.toHaveBeenCalled();

    // The student comes back: now it runs, as attended.
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "attended" });
  });

  it("leaves no trace when the start is refused", async () => {
    const api = mockApi({ scenario: "auto-sync-due" });
    let refuse: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      refuse = resolve;
    });
    const sync = vi.spyOn(api, "syncAll").mockImplementation(async () => {
      await gate;
      throw new ApiError("busy", "Another PageLamp window is syncing.");
    });
    const { router } = renderRoute("/courses", { api });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    // The start is in flight and has shown nothing yet: no capsule, nothing announced.
    await settle();
    expect(useSyncStore.getState().running).toBe(true);
    expect(screen.queryByRole("button", { name: /^Syncing/ })).toBeNull();
    expect(announced()).not.toContain("Syncing");

    refuse();
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false));
    await settle();
    expectNoTrace();
    expect(announced()).not.toContain("Syncing");

    await act(() => router.navigate("/sources"));
    await screen.findByRole("heading", { level: 1, name: "Sources & sync" });
    expect(screen.queryByRole("heading", { name: "Sync failed" })).toBeNull();
  });

  it("leaves no trace when the facade says it isn't due any more", async () => {
    // The answer said "due", but by the time the run asks, the mock's data is fresh.
    const api = mockApi();
    alwaysDue(api);
    const sync = vi.spyOn(api, "syncAll");
    renderRoute("/courses", { api });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false));
    await settle();
    expectNoTrace();
    expect(screen.queryByRole("button", { name: "Sync finished" })).toBeNull();
    expect(announced()).not.toContain("Syncing");
    expect(announced()).not.toContain("Sync finished");
  });

  it("stays quiet when a source can't be reached", async () => {
    const api = mockApi({ scenario: "auto-sync-due" });
    const sync = failingWith(api, "network");
    renderRoute("/courses", { api });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false));
    await settle();
    expectNoTrace();
    expect(announced()).not.toContain("problems");
    expect(document.body).toHaveFocus();
  });

  it("says once to screen readers when the run left a problem on a source", async () => {
    const api = mockApi({ scenario: "auto-sync-due" });
    const sync = failingWith(api, "auth_expired_or_revoked");
    // What counts is what the facade recorded on the source: here, Canvas's expired token.
    const status = api.status.bind(api);
    api.status = async () => {
      const now = await status();
      if (sync.mock.calls.length === 0) return now;
      return {
        ...now,
        sources: now.sources.map((source) =>
          source.id === CANVAS
            ? {
                ...source,
                last_error: "Synthetic failure",
                last_error_kind: "auth_expired_or_revoked" as const,
              }
            : source,
        ),
      };
    };
    renderRoute("/courses", { api });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(announced()).toContain("Sync finished with problems"));
    // Heard, not shown: no capsule to dismiss, nothing opened, focus unmoved.
    expectNoTrace();
    expect(document.body).toHaveFocus();
  });

  it("says nothing when the facade recorded nothing, whatever the run reported", async () => {
    // The same failed result, but the source's row is as it was: the facade kept it quiet.
    const api = mockApi({ scenario: "auto-sync-due" });
    const sync = failingWith(api, "auth_expired_or_revoked");
    renderRoute("/courses", { api });
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false));
    await settle();
    expectNoTrace();
    expect(announced()).not.toContain("problems");
  });

  it("waits for 'Got it' on What's new, then syncs as attended", async () => {
    const start = startClock();
    const api = mockApi({ scenario: "upgrader" });
    const sync = vi.spyOn(api, "syncAll");
    // Opened 13 hours after the last sync.
    later(start, 11 * HOUR);
    const { user } = renderRoute("/courses", { api });

    const sheet = await screen.findByRole("dialog", { name: "What's new in PageLamp" });
    await settle();
    expect(sync).not.toHaveBeenCalled();

    // Read at leisure: by now only "Got it" says the student is here.
    later(start, 11 * HOUR + 5 * MINUTE);
    await user.click(within(sheet).getByRole("button", { name: "Got it" }));
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "attended" });
  });

  it("doesn't ask for a sync while an update is being installed, and asks again afterwards", async () => {
    useUpdateStore.setState({ install: { phase: "downloading", downloaded: 0, total: null } });
    const api = mockApi({ scenario: "auto-sync-due" });
    const sync = vi.spyOn(api, "syncAll");
    renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await settle();
    expect(sync).not.toHaveBeenCalled();

    for (const phase of ["installing", "restarting", "held"] as const) {
      act(() => useUpdateStore.setState({ install: { phase } }));
      await settle(20);
      expect(sync).not.toHaveBeenCalled();
    }

    // The install didn't happen after all.
    act(() => useUpdateStore.setState({ install: { phase: "idle" } }));
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
  });

  it("doesn't start under an open dialog; it starts when the dialog closes", async () => {
    const start = startClock();
    const api = mockApi();
    const sync = vi.spyOn(api, "syncAll");
    const { user, queryClient } = renderRoute("/sources", { api });
    await user.click(await screen.findByRole("button", { name: "Add source" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a source" });

    later(start, 13 * HOUR);
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await settle();
    expect(sync).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
  });

  it("still asks again when a dialog was opened while it waited for a sync to end", async () => {
    const start = startClock();
    const api = mockApi({ syncStepMs: 30 });
    const all = vi.spyOn(api, "syncAll");
    const { user, queryClient } = renderRoute("/sources", { api });
    await screen.findByRole("heading", { level: 2, name: "Course calendar" });

    later(start, 13 * HOUR);
    await user.click(screen.getByRole("button", { name: "Sync Course calendar" }));
    await waitFor(() => expect(useSyncStore.getState().running).toBe(true));
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    // The student opens a dialog before that sync ends.
    await user.click(screen.getByRole("button", { name: "Add source" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a source" });
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false), { timeout: 3000 });
    await settle();
    expect(all).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(all).toHaveBeenCalledTimes(1));
  });

  it("never runs during onboarding", async () => {
    const api = mockApi({ scenario: "empty" });
    alwaysDue(api);
    const tasks = vi.spyOn(api, "startupTasks");
    const sync = vi.spyOn(api, "syncAll");
    renderRoute("/welcome", { api });
    await screen.findByRole("heading", { level: 1, name: "Welcome to PageLamp" });
    await settle();
    expect(tasks).not.toHaveBeenCalled();
    expect(sync).not.toHaveBeenCalled();
  });

  it("does nothing without sources", async () => {
    const api = mockApi({ scenario: "empty" });
    alwaysDue(api);
    const sync = vi.spyOn(api, "syncAll");
    renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await settle();
    expect(sync).not.toHaveBeenCalled();
  });

  it("syncs a source added during an automatic run as soon as that run ends", async () => {
    const api = mockApi({ scenario: "auto-sync-due" });
    const real = api.syncAll;
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    api.syncAll = async (req, onEvent: (event: SyncEvent) => void) => {
      onEvent({ type: "source_started", source_id: "folder:demo-courses", label: "Course folder" });
      await gate;
      return real(req, onEvent);
    };
    const one = vi.spyOn(api, "syncSource");
    const { user } = renderRoute("/sources", { api });
    await screen.findByRole("button", { name: "Syncing · Course folder" });

    await user.click(screen.getByRole("button", { name: "Add source" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a source" });
    await user.click(within(dialog).getByRole("checkbox", { name: "I understand" }));
    await user.click(within(dialog).getByRole("button", { name: "Choose folder…" }));
    await within(dialog).findByDisplayValue("/Users/demo/Documents/Courses");
    await user.click(within(dialog).getByRole("button", { name: "Add source" }));
    expect(
      await screen.findByText("Source added. It will be included in the next sync."),
    ).toBeInTheDocument();
    expect(one).not.toHaveBeenCalled();

    release();
    await waitFor(() => expect(one).toHaveBeenCalledTimes(1));
  });
});
