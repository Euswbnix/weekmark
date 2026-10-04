import { describe, expect, it } from "vitest";
import type { SyncEvent } from "../types";
import { createMockApi } from ".";

const fast = { latencyMs: 0, syncStepMs: 0 };
const HOUR = 60 * 60 * 1000;
const NOT_DUE = { unattended: false, attended: false };
const DUE = { unattended: true, attended: true };

describe("mock automatic sync", () => {
  it("is on, twice a day, and not due while the data is fresh", async () => {
    const api = createMockApi(fast);
    expect(await api.syncPrefs()).toEqual({ auto_sync: "twice_daily" });
    expect((await api.status()).auto_sync).toBe("twice_daily");
    expect((await api.startupTasks()).sync_due).toEqual(NOT_DUE);
  });

  it("is due once the sources are older than the interval, until a sync of any kind", async () => {
    const automatic = createMockApi({ ...fast, scenario: "auto-sync-due" });
    expect((await automatic.startupTasks()).sync_due).toEqual(DUE);
    const summary = await automatic.syncAll({ automatic: "attended" }, () => {});
    expect(summary.ok).toBe(true);
    expect(summary.results).toHaveLength(3);
    expect((await automatic.startupTasks()).sync_due).toEqual(NOT_DUE);

    // The student's own sync counts just the same.
    const manual = createMockApi({ ...fast, scenario: "auto-sync-due" });
    await manual.syncAll({}, () => {});
    expect((await manual.startupTasks()).sync_due).toEqual(NOT_DUE);
  });

  it("follows the setting: off is never due, once a day waits for 24 hours", async () => {
    let clock = new Date(2026, 9, 5, 9, 0);
    const api = createMockApi({ ...fast, scenario: "auto-sync-due", now: () => clock });
    await api.setSyncPrefs({ auto_sync: "off" });
    expect((await api.startupTasks()).sync_due).toEqual(NOT_DUE);
    expect((await api.status()).auto_sync).toBe("off");

    await api.setSyncPrefs({ auto_sync: "daily" });
    // 13 hours old.
    expect((await api.startupTasks()).sync_due).toEqual(NOT_DUE);
    clock = new Date(clock.getTime() + 11 * HOUR);
    expect((await api.startupTasks()).sync_due).toEqual(DUE);
  });

  it("is not due with no sources, before What's new is dismissed, or while a sync runs", async () => {
    let clock = new Date(2026, 9, 5, 9, 0);
    const later = () => {
      clock = new Date(clock.getTime() + 13 * HOUR);
    };

    expect((await createMockApi({ ...fast, scenario: "empty" }).startupTasks()).sync_due).toEqual(
      NOT_DUE,
    );

    const upgrader = createMockApi({ ...fast, scenario: "upgrader", now: () => clock });
    later();
    expect((await upgrader.startupTasks()).sync_due).toEqual(NOT_DUE);
    await upgrader.acknowledgeWhatsNew();
    expect((await upgrader.startupTasks()).sync_due).toEqual(DUE);

    const busy = createMockApi({ ...fast, scenario: "busy", now: () => clock });
    later();
    expect((await busy.startupTasks()).sync_due).toEqual(NOT_DUE);
  });

  it("answers an automatic call that isn't due with an empty summary and no events", async () => {
    const api = createMockApi(fast);
    const events: SyncEvent[] = [];
    const summary = await api.syncAll({ automatic: "unattended" }, (event) => events.push(event));
    expect(summary).toMatchObject({ ok: true, results: [] });
    expect(events).toEqual([]);
  });

  it("leaves out a source only the student can fix", async () => {
    let clock = new Date(2026, 9, 5, 9, 0);
    const api = createMockApi({ ...fast, scenario: "expired", now: () => clock });
    // Canvas (expired token) is 9 days old; the two others are fresh.
    expect((await api.startupTasks()).sync_due).toEqual(NOT_DUE);
    clock = new Date(clock.getTime() + 13 * HOUR);
    expect((await api.startupTasks()).sync_due).toEqual(DUE);

    const summary = await api.syncAll({ automatic: "unattended" }, () => {});
    expect(summary.ok).toBe(true);
    expect(summary.results.map((r) => r.kind).sort()).toEqual(["folder", "ical"]);
    expect((await api.startupTasks()).sync_due).toEqual(NOT_DUE);
  });

  it("waits an hour after an automatic run the student stopped", async () => {
    let clock = new Date(2026, 9, 5, 9, 0);
    const api = createMockApi({
      latencyMs: 0,
      syncStepMs: 5,
      scenario: "auto-sync-due",
      now: () => clock,
    });
    const run = api.syncAll({ automatic: "attended" }, () => {});
    await api.cancelSync();
    await expect(run).rejects.toMatchObject({ kind: "cancelled" });
    expect((await api.startupTasks()).sync_due).toEqual(NOT_DUE);

    clock = new Date(clock.getTime() + HOUR);
    expect((await api.startupTasks()).sync_due).toEqual(DUE);
  });

  it("reads Canvas lightly from the timer: the full sync stays due for the student's return", async () => {
    const api = createMockApi({ ...fast, scenario: "auto-sync-due" });
    const before = (await api.status()).sources.find((s) => s.kind === "canvas");
    const summary = await api.syncAll({ automatic: "unattended" }, () => {});
    expect(summary.ok).toBe(true);
    expect(summary.results).toHaveLength(3);

    const status = await api.status();
    const canvas = status.sources.find((s) => s.kind === "canvas");
    // The full sync's time didn't move; the deadlines have their own.
    expect(canvas?.last_synced_at).toBe(before?.last_synced_at);
    expect(Object.keys(status.deadlines_synced_at)).toEqual([canvas?.id]);
    expect((await api.startupTasks()).sync_due).toEqual({ unattended: false, attended: true });

    await api.syncAll({ automatic: "attended" }, () => {});
    expect((await api.status()).deadlines_synced_at).toEqual({});
    expect((await api.startupTasks()).sync_due).toEqual(NOT_DUE);
  });

  it("shows a light sync's leftovers in the light-synced scenario, with automatic sync off", async () => {
    const api = createMockApi({ ...fast, scenario: "light-synced" });
    expect(await api.syncPrefs()).toEqual({ auto_sync: "off" });
    expect((await api.startupTasks()).sync_due).toEqual(NOT_DUE);
    const status = await api.status();
    expect(Object.keys(status.deadlines_synced_at)).toEqual(["canvas:canvas.demo.test"]);
    const pending = (await api.listCourses()).filter((c) => c.structure_pending);
    expect(pending.map((c) => c.course.code)).toEqual(["DEMO404"]);
    expect(pending[0]?.counts.materials).toBe(0);
  });

  it("syncs only the sources that are due", async () => {
    // The folder and the feed 2 hours ago, Canvas 5 days ago.
    const api = createMockApi({ ...fast, scenario: "canvas-old" });
    await api.setSyncPrefs({ auto_sync: "twice_daily" });
    const summary = await api.syncAll({ automatic: "attended" }, () => {});
    expect(summary.results.map((r) => r.kind)).toEqual(["canvas"]);
  });

  it("leaves the source's last sync alone when one course's files are downloaded", async () => {
    const api = createMockApi({ ...fast, scenario: "auto-sync-due" });
    const canvas = () => api.status().then((s) => s.sources.find((x) => x.kind === "canvas"));
    const before = (await canvas())?.last_synced_at;
    await api.downloadCourseFiles("canvas:canvas.demo.test/course/205", () => {});
    expect((await canvas())?.last_synced_at).toBe(before);
    // So a full sync is as due as it was.
    expect((await api.startupTasks()).sync_due).toEqual(DUE);
  });
});
