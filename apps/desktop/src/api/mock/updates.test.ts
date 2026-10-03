import { describe, expect, it } from "vitest";
import type { UpdateEvent } from "../client";
import { createMockApi } from ".";
import { MOCK_APP_VERSION, MOCK_UPDATE_VERSION } from "./fixtures";

const fast = { latencyMs: 0, syncStepMs: 0 };

describe("mock updates", () => {
  it("uses beta by default for a pre-release build, and remembers a chosen channel", async () => {
    const api = createMockApi(fast);
    expect(await api.updatePrefs()).toEqual({ auto_check: true, channel: null });
    expect(await api.effectiveUpdateChannel()).toBe("beta");
    await api.setUpdatePrefs({ auto_check: false, channel: "stable" });
    expect(await api.updatePrefs()).toEqual({ auto_check: false, channel: "stable" });
    expect(await api.effectiveUpdateChannel()).toBe("stable");
  });

  it("has no stable release yet: a check there finds none and is recorded", async () => {
    const api = createMockApi({ ...fast, scenario: "update-available" });
    await api.setUpdatePrefs({ auto_check: true, channel: "stable" });
    await expect(api.checkForUpdate()).rejects.toMatchObject({ kind: "not_found" });
    expect(await api.lastUpdateCheck()).toMatchObject({
      channel: "stable",
      outcome: { kind: "error", code: "manifest" },
    });
  });

  it("shows upgraders 'What's new' first and only then makes a check due", async () => {
    const api = createMockApi({ ...fast, scenario: "upgrader" });
    const first = await api.startupTasks();
    // From alpha.0: every topic since, the update check first (the facade's table).
    expect(first.whats_new?.topics).toEqual([
      "update_check",
      "course_weeks",
      "course_removal",
      "syllabus_reading",
      "ai_writing",
      "reminders",
    ]);
    expect(first.whats_new?.since).toBe("0.3.0-alpha.0");
    expect(first.update_check_due).toBe(false);
    await api.acknowledgeWhatsNew();
    const after = await api.startupTasks();
    expect(after.whats_new).toBeNull();
    expect(after.update_check_due).toBe(true);
  });

  it("makes no check due on a fresh install until onboarding disclosed it", async () => {
    const api = createMockApi({ ...fast, scenario: "empty" });
    expect((await api.startupTasks()).update_check_due).toBe(false);
    await api.acknowledgeUpdateDisclosure();
    // Disclosed now, but the mock's last check was 2 h ago: the next one is due after a day.
    expect((await api.startupTasks()).update_check_due).toBe(false);
    expect(await api.lastUpdateCheck()).toMatchObject({ outcome: { kind: "up_to_date" } });
  });

  it("offers an update, records the check, and installs it with progress", async () => {
    const api = createMockApi({ ...fast, scenario: "update-available" });
    expect(await api.lastUpdateCheck()).toBeNull();
    expect((await api.startupTasks()).update_check_due).toBe(true);
    const update = await api.checkForUpdate();
    expect(update?.version).toBe(MOCK_UPDATE_VERSION);
    expect(await api.lastUpdateCheck()).toMatchObject({
      outcome: { kind: "available", version: MOCK_UPDATE_VERSION },
      channel: "beta",
    });
    const events: UpdateEvent[] = [];
    await api.installUpdate((event) => events.push(event));
    expect(events.map((e) => e.type)).toEqual([
      "download_started",
      "progress",
      "progress",
      "progress",
      "progress",
      "installing",
      "restarting",
    ]);
  });

  it("reports up to date otherwise", async () => {
    const api = createMockApi(fast);
    expect(await api.checkForUpdate()).toBeNull();
    expect(await api.lastUpdateCheck()).toMatchObject({ outcome: { kind: "up_to_date" } });
    expect((await api.updaterStatus()).current_version).toBe(MOCK_APP_VERSION);
  });

  it("only links to the download on deb/rpm installs", async () => {
    const api = createMockApi({ ...fast, scenario: "deb" });
    expect(await api.updaterStatus()).toMatchObject({
      install: "download_only",
      platform: "linux",
    });
    expect((await api.checkForUpdate())?.download_url).toContain(`v${MOCK_UPDATE_VERSION}`);
    await expect(api.installUpdate(() => {})).rejects.toMatchObject({ kind: "invalid" });
  });
});
