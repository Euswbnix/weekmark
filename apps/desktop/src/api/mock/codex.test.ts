import { describe, expect, it } from "vitest";
import type { LoginEvent, RuntimeEvent } from "../ai";
import { createMockApi } from ".";
import { MOCK_CODEX_PIN, MOCK_DEVICE_CODE } from "./codex";

const fast = { latencyMs: 0, syncStepMs: 0 };
/** The ChatGPT plan offered, nothing installed yet. */
const notInstalled = { ...fast, scenario: "codex-not-installed" } as const;
const codex = { kind: "codex" } as const;
const DEMO101 = "folder:demo-courses/course/DEMO101";

describe("mock ChatGPT plan (Codex)", () => {
  it("starts not installed, with the download size, and not among the backends", async () => {
    const api = createMockApi(notInstalled);
    const status = await api.codexStatus();
    expect(status.runtime).toMatchObject({
      state: "not_installed",
      pinned_version: MOCK_CODEX_PIN,
    });
    expect(status.runtime.download_bytes).toBeGreaterThan(60_000_000);
    expect((await api.aiStatus()).backends).toEqual([]);
  });

  it("installs with progress, then signs in and asks for the disclosure", async () => {
    const api = createMockApi(notInstalled);
    const events: RuntimeEvent[] = [];
    const installed = await api.installCodex("install-1", (e) => events.push(e));
    expect(events.map((e) => e.type)).toEqual([
      "download_started",
      "progress",
      "progress",
      "progress",
      "progress",
      "verifying",
      "installing",
      "done",
    ]);
    expect(installed.runtime).toMatchObject({
      state: "installed",
      installed_version: MOCK_CODEX_PIN,
    });
    expect((await api.aiStatus()).backends[0]).toMatchObject({
      kind: "codex",
      state: "needs_setup",
    });

    const login: LoginEvent[] = [];
    const signedIn = await api.codexLogin("browser", (e) => login.push(e));
    expect(login.map((e) => e.type)).toEqual(["browser_opened", "waiting", "done"]);
    // `codex login status` doesn't say the plan type yet (A7).
    expect(signedIn.login).toEqual({ state: "chatgpt", plan_type: "unknown" });
    const [backend] = (await api.aiStatus()).backends;
    expect(backend).toMatchObject({ state: "needs_disclosure" });
    await api.acknowledgeAiDisclosure(codex, backend?.disclosure.version ?? 0);
    expect((await api.aiStatus()).backends[0]?.state).toBe("ready");
  });

  it("cancels a download (nothing installed) and a sign-in", async () => {
    const api = createMockApi({ ...notInstalled, syncStepMs: 5 });
    const install = api.installCodex("install-2", (e) => {
      if (e.type === "download_started") void api.cancelCodexInstall("install-2");
    });
    await expect(install).rejects.toMatchObject({ kind: "cancelled" });
    expect((await api.codexStatus()).runtime.state).toBe("not_installed");

    await api.installCodex("install-3", () => {});
    const login = api.codexLogin("device_code", (e) => {
      if (e.type === "device_code") {
        expect(e.user_code).toBe(MOCK_DEVICE_CODE);
        void api.cancelCodexLogin();
      }
    });
    await expect(login).rejects.toMatchObject({ kind: "cancelled" });
    expect((await api.codexStatus()).login.state).toBe("signed_out");
  });

  it("discloses training, credits and, for Edu, admin visibility", async () => {
    const plus = createMockApi({ ...fast, scenario: "codex-plus" });
    expect((await plus.aiStatus()).backends[0]?.disclosure).toMatchObject({
      training: { kind: "may_train" },
      cost: "plan_credits",
      admin_visibility: "no",
    });
    const edu = createMockApi({ ...fast, scenario: "codex-edu" });
    expect((await edu.aiStatus()).backends[0]).toMatchObject({
      state: "needs_disclosure",
      disclosure: { training: { kind: "no_training" }, admin_visibility: "yes" },
    });
  });

  it("counts runs against the weekly cap instead of money", async () => {
    const plus = createMockApi({ ...fast, scenario: "codex-plus" });
    const req = { feature: "weekly_explanation", course: DEMO101 } as const;
    expect(await plus.estimateGeneration(req)).toMatchObject({
      micro_usd_upper: null,
      would_block: null,
    });
    expect((await plus.usageSummary(null)).mode_a).toEqual({ runs_this_week: 12, weekly_cap: 40 });

    const capped = createMockApi({ ...fast, scenario: "codex-cap" });
    expect((await capped.estimateGeneration(req)).would_block).toBe("weekly_run_cap_reached");
    await capped.setModeAWeeklyCap(null);
    expect((await capped.estimateGeneration(req)).would_block).toBeNull();
    await expect(capped.setModeAWeeklyCap(0)).rejects.toMatchObject({ kind: "invalid" });
  });

  it("says what a RuntimeOutdated error means: install the pin, or update PageLamp", async () => {
    const old = createMockApi({ ...fast, scenario: "codex-outdated-pin" });
    expect((await old.codexStatus()).outdated_action).toBe("install_pin");
    expect(await old.testModel(codex, "gpt-6-luna")).toMatchObject({
      ok: false,
      error: "runtime_outdated",
    });
    const updated = await old.installCodex("install-4", () => {});
    expect(updated).toMatchObject({
      outdated_action: "none",
      runtime: { installed_version: MOCK_CODEX_PIN },
    });

    const app = createMockApi({ ...fast, scenario: "codex-outdated-app" });
    expect((await app.codexStatus()).outdated_action).toBe("update_pagelamp");
  });

  it("reports an API-key sign-in and a plan without codex exec", async () => {
    const key = createMockApi({ ...fast, scenario: "codex-api-key" });
    expect((await key.codexStatus()).login.state).toBe("api_key");
    // No plan type: the admin warning is conditional, never left out.
    expect((await key.aiStatus()).backends[0]?.disclosure.admin_visibility).toBe("unknown");
    const free = createMockApi({ ...fast, scenario: "codex-free" });
    expect(await free.codexStatus()).toMatchObject({
      exec_available: false,
      login: { plan_type: "free" },
    });
    expect((await free.aiStatus()).backends[0]?.state).toBe("needs_setup");
  });

  it("removes Codex and the features that used it; remove-all signs out", async () => {
    const api = createMockApi({ ...fast, scenario: "codex-plus" });
    await expect(api.setCodexSource("system")).rejects.toMatchObject({ kind: "invalid" });
    await api.removeAllAiData();
    expect((await api.codexStatus()).login.state).toBe("signed_out");

    const other = createMockApi({ ...fast, scenario: "codex-plus" });
    await other.removeCodex();
    const status = await other.aiStatus();
    expect(status.backends).toEqual([]);
    expect(status.features.every((f) => f.choice === null)).toBe(true);
  });

  it("reports what's missing, and refuses an unpriced acknowledgement for the plan", async () => {
    const api = createMockApi({ ...fast, scenario: "codex-signed-out" });
    expect((await api.aiStatus()).backends[0]).toMatchObject({
      state: "needs_setup",
      problems: ["not_signed_in"],
    });
    await expect(api.acknowledgeUnpricedModel(codex, "gpt-6-luna")).rejects.toMatchObject({
      kind: "invalid",
    });
    expect(await api.setCodexSource("managed")).toMatchObject({ runtime: { source: "managed" } });
  });
});
