import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/api/errors";
import type { SourceErrorKind, SyncSummary } from "@/api/types";
import { paths } from "@/lib/routes";
import { useSyncStore } from "@/stores/sync";
import { renderRoute } from "@/test/render";
import { FINISHED_MS } from "./AccessoryBar";

const summary = (ok: boolean): SyncSummary => ({
  started_at: "2026-09-28T12:00:00Z",
  finished_at: "2026-09-28T12:01:00Z",
  ok,
  results: [],
});

function startRun() {
  act(() => {
    const store = useSyncStore.getState();
    store.begin(2);
    store.apply({ type: "source_started", source_id: "canvas", label: "Demo Canvas" });
  });
}

function finishCanvas(ok: boolean) {
  act(() =>
    useSyncStore.getState().apply({
      type: "source_finished",
      source_id: "canvas",
      ok,
      error: ok ? null : "Synthetic failure",
      error_kind: ok ? null : "network",
    }),
  );
}

/** The summary of a run in which Canvas ended with `kind` (null: it synced). */
function canvasSummary(kind: SourceErrorKind | null): SyncSummary {
  return {
    ...summary(kind === null),
    results: [
      {
        source_id: "canvas",
        label: "Demo Canvas",
        kind: "canvas",
        ok: kind === null,
        error: kind === null ? null : "Synthetic failure",
        error_kind: kind,
        started_at: "2026-09-28T12:00:00Z",
        finished_at: "2026-09-28T12:01:00Z",
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

/** A sync PageLamp started by itself, up to Canvas ending with `kind`. */
function automaticRun(kind: SourceErrorKind | null) {
  act(() => {
    const store = useSyncStore.getState();
    store.begin(2, null, "unattended");
    store.apply({ type: "source_started", source_id: "canvas", label: "Demo Canvas" });
  });
  act(() =>
    useSyncStore.getState().apply({
      type: "source_finished",
      source_id: "canvas",
      ok: kind === null,
      error: kind === null ? null : "Synthetic failure",
      error_kind: kind,
    }),
  );
}

async function renderIdle() {
  const result = renderRoute("/settings");
  await screen.findByRole("heading", { level: 1, name: "Settings" });
  return result;
}

const announced = () =>
  screen
    .getAllByRole("status")
    .map((s) => s.textContent)
    .join(" ");

afterEach(() => {
  vi.useRealTimers();
});

describe("AccessoryBar", () => {
  it("stays out of sight (and out of reach) when nothing is syncing", async () => {
    await renderIdle();
    expect(screen.queryByRole("button", { name: /sync/i })).toBeNull();
  });

  it("counts sources while this window syncs, and announces only the start", async () => {
    await renderIdle();
    startRun();
    expect(screen.getByRole("button", { name: "Syncing 0 of 2 · Demo Canvas" })).toBeVisible();
    expect(announced()).toContain("Syncing…");

    finishCanvas(true);
    act(() =>
      useSyncStore
        .getState()
        .apply({ type: "source_started", source_id: "folder", label: "Course folder" }),
    );
    expect(screen.getByRole("button", { name: "Syncing 1 of 2 · Course folder" })).toBeVisible();
    expect(announced()).not.toContain("Course folder");

    // Every source done, the run not over yet.
    act(() =>
      useSyncStore
        .getState()
        .apply({ type: "source_finished", source_id: "folder", ok: true, error: null }),
    );
    expect(screen.getByRole("button", { name: "Syncing 2/2…" })).toBeVisible();
  });

  it("says 'Sync finished' for 4 seconds, then leaves", async () => {
    await renderIdle();
    startRun();
    vi.useFakeTimers();
    finishCanvas(true);
    act(() => useSyncStore.getState().finish(summary(true), null));
    expect(screen.getByRole("button", { name: "Sync finished" })).toBeInTheDocument();
    expect(announced()).toContain("Sync finished");

    act(() => vi.advanceTimersByTime(FINISHED_MS - 1));
    expect(screen.getByRole("button", { name: "Sync finished" })).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(1));
    expect(screen.queryByRole("button", { name: "Sync finished" })).toBeNull();
  });

  it("keeps a problem until it is dismissed with × or Esc", async () => {
    await renderIdle();
    startRun();
    vi.useFakeTimers();
    finishCanvas(false);
    act(() => useSyncStore.getState().finish(summary(false), null));
    expect(announced()).toContain("Sync finished with problems");
    act(() => vi.advanceTimersByTime(FINISHED_MS * 2));
    expect(screen.getByRole("button", { name: "Sync finished with problems" })).toBeInTheDocument();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("button", { name: "Sync finished with problems" })).toBeNull();

    vi.useRealTimers();
    startRun();
    act(() => useSyncStore.getState().finish(null, new ApiError("network", "Synthetic failure")));
    expect(screen.getByRole("button", { name: "Sync failed" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("button", { name: "Sync failed" })).toBeNull();
  });

  it("leaves quietly when the student stopped the sync, but says so to screen readers", async () => {
    await renderIdle();
    startRun();
    act(() => useSyncStore.getState().finish(null, new ApiError("cancelled", "Cancelled")));
    expect(screen.queryByRole("button", { name: /sync/i })).toBeNull();
    expect(announced()).toContain("Sync stopped");
  });

  it("waits while focused, and hands focus to the page when it leaves", async () => {
    await renderIdle();
    startRun();
    vi.useFakeTimers();
    finishCanvas(true);
    act(() => useSyncStore.getState().finish(summary(true), null));
    const capsule = screen.getByRole("button", { name: "Sync finished" });
    act(() => capsule.focus());
    act(() => vi.advanceTimersByTime(FINISHED_MS * 2));
    expect(capsule).toHaveFocus();

    // A problem dismissed with ×: focus goes to the page, not to <body>.
    vi.useRealTimers();
    startRun();
    act(() => useSyncStore.getState().finish(null, new ApiError("network", "Synthetic failure")));
    const dismiss = screen.getByRole("button", { name: "Dismiss" });
    act(() => dismiss.focus());
    fireEvent.click(dismiss);
    expect(screen.getByRole("main")).toHaveFocus();
  });

  it("opens per-source progress with Stop and a way to Sources & sync", async () => {
    const { user } = await renderIdle();
    startRun();
    await user.click(screen.getByRole("button", { name: "Syncing 0 of 2 · Demo Canvas" }));

    const details = await screen.findByRole("dialog", { name: "Sync details" });
    await waitFor(() => expect(details).toHaveFocus());
    expect(within(details).getByText("Demo Canvas")).toBeInTheDocument();
    expect(within(details).getByRole("button", { name: "Stop" })).toBeInTheDocument();
    const link = within(details).getByRole("link", { name: "Open Sources & sync" });
    expect(link).toHaveAttribute("href", paths.sources);

    await user.click(link);
    expect(await screen.findByRole("heading", { level: 1, name: "Sources & sync" })).toBeVisible();
    expect(screen.queryByRole("dialog", { name: "Sync details" })).toBeNull();
  });

  describe("for a sync PageLamp started by itself", () => {
    it("shows nothing until the run's first event, then the same capsule and 'Sync finished'", async () => {
      await renderIdle();
      act(() => useSyncStore.getState().begin(2, null, "unattended"));
      expect(screen.queryByRole("button", { name: /^Syncing/ })).toBeNull();
      expect(announced()).not.toContain("Syncing");

      act(() =>
        useSyncStore
          .getState()
          .apply({ type: "source_started", source_id: "canvas", label: "Demo Canvas" }),
      );
      expect(screen.getByRole("button", { name: "Syncing 0 of 2 · Demo Canvas" })).toBeVisible();
      expect(announced()).toContain("Syncing…");

      finishCanvas(true);
      act(() => useSyncStore.getState().finish(canvasSummary(null), null));
      expect(screen.getByRole("button", { name: "Sync finished" })).toBeInTheDocument();
      expect(announced()).toContain("Sync finished");
    });

    it("leaves focus on the page's body when 'Sync finished' goes", async () => {
      await renderIdle();
      automaticRun(null);
      vi.useFakeTimers();
      act(() => useSyncStore.getState().finish(canvasSummary(null), null));
      act(() => vi.advanceTimersByTime(FINISHED_MS));
      expect(screen.queryByRole("button", { name: "Sync finished" })).toBeNull();
      expect(document.body).toHaveFocus();
    });

    it("just leaves when the run went wrong: no problem to dismiss, nothing said", async () => {
      await renderIdle();
      automaticRun("network");
      act(() => useSyncStore.getState().finish(canvasSummary("network"), null));
      expect(screen.queryByRole("button", { name: /sync/i })).toBeNull();
      expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull();
      expect(announced()).not.toContain("problems");
      expect(announced()).not.toContain("Sync failed");
      expect(document.body).toHaveFocus();

      automaticRun(null);
      act(() => useSyncStore.getState().finish(null, new ApiError("network", "Synthetic failure")));
      expect(screen.queryByRole("button", { name: /sync/i })).toBeNull();
      expect(announced()).not.toContain("Sync failed");
    });

    it("says it once to screen readers when the run left a problem on a source", async () => {
      await renderIdle();
      automaticRun("auth_expired_or_revoked");
      act(() =>
        useSyncStore.getState().finish(canvasSummary("auth_expired_or_revoked"), null, true),
      );
      expect(screen.queryByRole("button", { name: /sync/i })).toBeNull();
      expect(announced()).toContain("Sync finished with problems");
    });

    it("names the source without a count: only the facade knows how many are due", async () => {
      await renderIdle();
      act(() => {
        const store = useSyncStore.getState();
        store.begin(null, null, "unattended");
        store.apply({ type: "source_started", source_id: "canvas", label: "Demo Canvas" });
      });
      expect(screen.getByRole("button", { name: "Syncing · Demo Canvas" })).toBeVisible();
    });

    it("keeps the problem on screen when the student is in the capsule", async () => {
      await renderIdle();
      act(() => {
        const store = useSyncStore.getState();
        store.begin(2, null, "unattended");
        store.apply({ type: "source_started", source_id: "canvas", label: "Demo Canvas" });
      });
      act(() => screen.getByRole("button", { name: "Syncing 0 of 2 · Demo Canvas" }).focus());
      finishCanvas(false);
      act(() => useSyncStore.getState().finish(canvasSummary("network"), null));
      expect(
        screen.getByRole("button", { name: "Sync finished with problems" }),
      ).toBeInTheDocument();

      // Dismissed by the student from inside it: focus goes to the page, as after any run.
      fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
      expect(screen.queryByRole("button", { name: "Sync finished with problems" })).toBeNull();
      expect(document.getElementById("main")).toHaveFocus();
    });

    it("leaves the student's failed sync on screen when its own start is refused", async () => {
      await renderIdle();
      startRun();
      act(() => useSyncStore.getState().finish(null, new ApiError("network", "Synthetic failure")));
      expect(screen.getByRole("button", { name: "Sync failed" })).toBeInTheDocument();

      // PageLamp tries one by itself and is refused: the failure and its details are untouched.
      act(() => useSyncStore.getState().begin(2, null, "unattended"));
      expect(screen.getByRole("button", { name: "Sync failed" })).toBeInTheDocument();
      act(() => useSyncStore.getState().finish(null, new ApiError("busy", "Synthetic refusal")));
      expect(screen.getByRole("button", { name: "Sync failed" })).toBeInTheDocument();
      expect(useSyncStore.getState().runError?.kind).toBe("network");
      expect(useSyncStore.getState().order).toEqual(["canvas"]);
    });

    it("keeps saying what it said while it fades out after a run that went wrong", async () => {
      const { container } = await renderIdle();
      automaticRun("network");
      const capsule = container.querySelector(".pl-accessory");
      expect(capsule).toHaveTextContent("Syncing 1/2…");
      act(() => useSyncStore.getState().finish(canvasSummary("network"), null));
      // Hidden now; the store holds no run, and the text must not fall back to "Syncing…".
      expect(capsule).toHaveAttribute("data-state", "hidden");
      expect(capsule).toHaveTextContent("Syncing 1/2…");
    });

    it("never opens its details by itself, also after the student closed the last run from them", async () => {
      const { user } = await renderIdle();
      startRun();
      await user.click(screen.getByRole("button", { name: "Syncing 0 of 2 · Demo Canvas" }));
      const details = await screen.findByRole("dialog", { name: "Sync details" });
      // Stopped from its details: the capsule leaves with them still open.
      act(() => useSyncStore.getState().finish(null, new ApiError("cancelled", "Cancelled")));
      await waitFor(() => expect(details).not.toBeInTheDocument());

      act(() => {
        const store = useSyncStore.getState();
        store.begin(2, null, "unattended");
        store.apply({ type: "source_started", source_id: "canvas", label: "Demo Canvas" });
      });
      expect(screen.getByRole("button", { name: "Syncing 0 of 2 · Demo Canvas" })).toBeVisible();
      await new Promise((resolve) => setTimeout(resolve, 20));
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });
});
