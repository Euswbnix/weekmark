import { screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { UsageSummary } from "@/api/ai";
import { createMockApi } from "@/api/mock";
import { renderRoute } from "@/test/render";

/**
 * The ChatGPT plan is offered only once OpenAI confirms in writing (the facade's
 * CHATGPT_PLAN_OFFERED, `chatgpt_plan_offered` in ai_status and codex_status). Until then nothing
 * names it: no card, no plan rows or weekly runs in usage, and every way into Codex refuses.
 */

const fast = { latencyMs: 0, syncStepMs: 0 };
const codex = { kind: "codex" } as const;
const CARD = "Use my ChatGPT plan (runs OpenAI Codex)";

afterEach(() => vi.restoreAllMocks());

async function section(title: string) {
  const heading = await screen.findByRole("heading", { level: 2, name: title });
  const region = heading.closest("section");
  if (!region) throw new Error(`no ${title} section`);
  return region;
}

/** A month with a ChatGPT-plan row and this week's plan runs (as an earlier build could leave). */
function withPlanUsage(summary: UsageSummary): UsageSummary {
  return {
    ...summary,
    rows: [
      ...summary.rows,
      {
        backend_label: "ChatGPT plan (through OpenAI Codex)",
        cost_basis: "plan",
        estimated: false,
        feature: "weekly_explanation",
        input_tokens: 12_000,
        micro_usd: null,
        model: "gpt-5.4",
        output_tokens: 900,
        reasoning_tokens: 0,
        runs: 3,
      },
    ],
    mode_a: { runs_this_week: 3, weekly_cap: 40 },
  };
}

describe("the ChatGPT plan while this build doesn't offer it (the default)", () => {
  it("the mock says so, lists no Codex backend, and refuses every way into Codex", async () => {
    const api = createMockApi(fast);
    const status = await api.aiStatus();
    expect(status.chatgpt_plan_offered).toBe(false);
    expect(status.backends.some((b) => b.backend.kind === "codex")).toBe(false);
    // codex_status still answers (for clean-up), says the same, and, like the facade, looked
    // for no Codex: neutral, with none of the student's own.
    expect(await api.codexStatus()).toMatchObject({
      chatgpt_plan_offered: false,
      runtime: { state: "not_installed", installed_version: null },
      login: { state: "signed_out" },
      system_codex: null,
    });
    const refusal = { kind: "blocked", blocked: "backend_disabled_in_this_build" };
    await expect(api.installCodex("i-1", () => {})).rejects.toMatchObject(refusal);
    await expect(api.codexLogin("browser", () => {})).rejects.toMatchObject(refusal);
    await expect(api.setCodexSource("managed")).rejects.toMatchObject(refusal);
    await expect(api.setModeAWeeklyCap(10)).rejects.toMatchObject(refusal);
    await expect(api.listModels(codex)).rejects.toMatchObject(refusal);
    await expect(api.testModel(codex, "gpt-5.4")).rejects.toMatchObject(refusal);
    await expect(
      api.setFeatureModel("weekly_explanation", {
        backend: codex,
        model: "gpt-5.4",
        effort: "lowest",
      }),
    ).rejects.toMatchObject(refusal);
    await expect(api.acknowledgeAiDisclosure(codex, 1)).rejects.toMatchObject(refusal);
    // Clean-up still works.
    await expect(api.codexLogout()).resolves.toBeDefined();
    await expect(api.removeCodex()).resolves.toBeUndefined();
    await expect(api.removeAllAiData()).resolves.toBeDefined();
  });

  it("Settings: no ChatGPT card, and the AI sections never name ChatGPT or Codex", async () => {
    const api = createMockApi(fast);
    vi.spyOn(api, "usageSummary").mockImplementation(async (month) =>
      withPlanUsage(await createMockApi(fast).usageSummary(month)),
    );
    renderRoute("/settings", { api });
    const models = await section("AI models");
    await within(models).findByText(/No model set up yet/);
    expect(within(models).queryByRole("heading", { name: CARD })).toBeNull();
    const usage = await section("AI usage");
    await waitFor(() => expect(within(usage).queryByText(/runs this week/)).toBeNull());
    await within(usage).findByRole("combobox");
    for (const region of [models, usage]) {
      expect(region.textContent).not.toMatch(/ChatGPT|Codex/);
    }
  });
});

describe("the ChatGPT plan where the build offers it (the codex-* scenarios)", () => {
  it("the mock offers it", async () => {
    const api = createMockApi({ ...fast, scenario: "codex-plus" });
    expect((await api.aiStatus()).chatgpt_plan_offered).toBe(true);
    expect((await api.codexStatus()).chatgpt_plan_offered).toBe(true);
    expect((await api.aiStatus()).backends.some((b) => b.backend.kind === "codex")).toBe(true);
  });

  it("Settings: the card, and the plan's rows and weekly runs in usage", async () => {
    const api = createMockApi({ ...fast, scenario: "codex-not-installed" });
    vi.spyOn(api, "usageSummary").mockImplementation(async (month) =>
      withPlanUsage(await createMockApi(fast).usageSummary(month)),
    );
    renderRoute("/settings", { api });
    const models = await section("AI models");
    expect(await within(models).findByRole("heading", { name: CARD })).toBeInTheDocument();
    const usage = await section("AI usage");
    expect(
      await within(usage).findByText("3 of 40 ChatGPT-plan runs this week"),
    ).toBeInTheDocument();
    expect(within(usage).getByText("ChatGPT plan (through OpenAI Codex)")).toBeInTheDocument();
  });
});
