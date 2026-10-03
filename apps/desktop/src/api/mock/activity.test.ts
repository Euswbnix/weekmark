import { describe, expect, it } from "vitest";
import type { ApiError } from "../errors";
import { createMockApi } from ".";

const NOW = new Date(2026, 8, 28, 10, 0); // Monday 2026-09-28
const FITTED = "canvas:canvas.demo.test/course/332"; // "proposals": AI reading can run
// Steps long enough that a run is still going when activity() answers.
const slow = { latencyMs: 0, syncStepMs: 20, now: () => NOW };

describe("mock activity", () => {
  it("lists a reading while it runs, a batch as one item, and drops a stopped one", async () => {
    const api = createMockApi({ ...slow, scenario: "proposals" });
    const options = { override_budget: false };

    const reading = api.readCourseCalendar(FITTED, "gen-1", options, () => {});
    expect((await api.activity()).items).toMatchObject([
      { kind: "generation", generation_id: "gen-1", source_id: null },
    ]);
    await reading;
    expect((await api.activity()).items).toEqual([]);

    const batch = api.readCourseCalendars([FITTED], "batch-1", options, () => {});
    expect((await api.activity()).items).toMatchObject([
      { kind: "generation", generation_id: "batch-1" },
    ]);
    await batch;
    expect((await api.activity()).items).toEqual([]);

    const stopped = api.readCourseCalendar(FITTED, "gen-2", options, () => {});
    await api.cancelGeneration("gen-2");
    await expect(stopped).rejects.toMatchObject({ kind: "cancelled" } satisfies Partial<ApiError>);
    expect((await api.activity()).items).toEqual([]);
  });

  it("lists syncs while they run", async () => {
    const api = createMockApi(slow);
    const [source] = await api.listSources();
    if (!source) throw new Error("the demo has sources");

    const all = api.syncAll({}, () => {});
    expect((await api.activity()).items).toMatchObject([{ kind: "sync", source_id: null }]);
    await all;
    const one = api.syncSource(source.id, {}, () => {});
    expect((await api.activity()).items).toMatchObject([{ kind: "sync", source_id: source.id }]);
    await one;
    expect(await api.activity()).toEqual({ items: [], other_process_syncing: false });
  });

  it("lists a Codex download while it runs, and never a refused one", async () => {
    // The demo doesn't offer the ChatGPT plan: the install is refused before any activity.
    const demo = createMockApi(slow);
    await expect(demo.installCodex("install-0", () => {})).rejects.toMatchObject({
      kind: "blocked",
      blocked: "backend_disabled_in_this_build",
    } satisfies Partial<ApiError>);
    expect((await demo.activity()).items).toEqual([]);

    const api = createMockApi({ ...slow, scenario: "codex-not-installed" });
    const codex = api.installCodex("install-1", () => {});
    expect((await api.activity()).items).toMatchObject([{ kind: "codex_install" }]);
    await codex;
    expect(await api.activity()).toEqual({ items: [], other_process_syncing: false });
  });
});
