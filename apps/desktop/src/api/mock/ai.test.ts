import { describe, expect, it } from "vitest";
import { materialSharing } from "../ai";
import { createMockApi } from ".";

const fast = { latencyMs: 0, syncStepMs: 0 };
const openai = { kind: "provider", provider_id: "openai" } as const;

async function courseId(api: ReturnType<typeof createMockApi>, code: string) {
  const courses = await api.listCourses();
  const found = courses.find((c) => c.course.code === code);
  if (!found) throw new Error(`no ${code}`);
  return found.course;
}

describe("mock AI setup", () => {
  it("starts with nothing set up, the default budget, and no model to estimate with", async () => {
    const api = createMockApi(fast);
    const status = await api.aiStatus();
    expect(status.backends).toEqual([]);
    expect(status.features.map((f) => f.choice)).toEqual([null, null, null, null]);
    expect(status.budget).toEqual({
      monthly_micro_usd: 5_000_000,
      spent_micro_usd: 0,
      warn_at_percent: 80,
    });
    const estimate = await api.estimateGeneration({ feature: "weekly_note" });
    expect(estimate.would_block).toBe("no_model_chosen");
  });

  it("keeps only the key's last 4 characters and asks for the disclosure first", async () => {
    const api = createMockApi(fast);
    const key = "sk-demo-secret-value-9f3A";
    const record = await api.addModelProvider("openai", null, key);
    expect(record).toMatchObject({ provider_id: "openai", key_last4: "9f3A", on_device: false });
    const status = await api.aiStatus();
    expect(JSON.stringify(status)).not.toContain("secret-value");
    const [backend] = status.backends;
    expect(backend).toMatchObject({ state: "needs_disclosure", disclosure_acknowledged: null });
    await expect(
      api.acknowledgeAiDisclosure(openai, (backend?.disclosure.version ?? 0) + 1),
    ).rejects.toMatchObject({ kind: "invalid" });
    await api.acknowledgeAiDisclosure(openai, backend?.disclosure.version ?? 0);
    expect((await api.aiStatus()).backends[0]?.state).toBe("ready");
  });

  it("refuses coding-plan keys, rejected keys and plain http to another computer", async () => {
    const api = createMockApi(fast);
    await expect(
      api.addModelProvider("custom", "https://api.z.ai/api/coding/paas/v4", "demo-key"),
    ).rejects.toMatchObject({ kind: "blocked", blocked: "coding_plan_key" });
    await expect(api.addModelProvider("openai", null, "sk-sp-demo")).rejects.toMatchObject({
      blocked: "coding_plan_key",
    });
    await expect(api.addModelProvider("openai", null, "sk-bad-key")).rejects.toMatchObject({
      kind: "model",
      model_error: "auth_rejected",
    });
    await expect(
      api.addModelProvider("custom", "http://llm.example.edu/v1", "demo-key"),
    ).rejects.toMatchObject({ kind: "invalid" });
    await expect(api.addModelProvider("openai", null, "  ")).rejects.toMatchObject({
      kind: "invalid",
    });
    await expect(api.addModelProvider("no-such-preset", null, "k")).rejects.toMatchObject({
      kind: "not_found",
    });
    // A local server needs no key, and http is fine on this computer.
    const local = await api.addModelProvider("ollama", "http://127.0.0.1:11434", null);
    expect(local).toMatchObject({ on_device: true, key_last4: null });
    // One provider per preset.
    await expect(
      api.addModelProvider("ollama", "http://127.0.0.1:11434", null),
    ).rejects.toMatchObject({ kind: "invalid" });
  });

  it("estimates an upper bound and applies the course gates, question (b) included", async () => {
    const api = createMockApi({ ...fast, scenario: "ai-key" });
    const demo101 = await courseId(api, "DEMO101");
    const ok = await api.estimateGeneration({ feature: "weekly_explanation", course: demo101.id });
    expect(ok.would_block).toBeNull();
    expect(ok.price_known).toBe(true);
    expect(ok.micro_usd_upper).toBeGreaterThan(0);
    expect(ok.reasoning_allowance).toBe(2_000); // effort "low"

    const demo205 = await courseId(api, "DEMO205");
    expect(materialSharing(demo205)).toBe("not_allowed");
    const shared = await api.estimateGeneration({
      feature: "weekly_explanation",
      course: demo205.id,
    });
    // A blocked estimate carries no amount and no tokens.
    expect(shared).toMatchObject({
      would_block: "material_sharing_not_allowed",
      micro_usd_upper: null,
      input_tokens: 0,
    });

    const demo310 = await courseId(api, "DEMO310");
    const prohibited = await api.estimateGeneration({
      feature: "weekly_explanation",
      course: demo310.id,
    });
    expect(prohibited.would_block).toBe("course_policy_prohibited");

    // Answering "allowed" lifts the question-(b) block.
    await api.setCourseMaterialSharing(demo205.id, "allowed");
    expect(materialSharing(await courseId(api, "DEMO205"))).toBe("allowed");
    const after = await api.estimateGeneration({
      feature: "weekly_explanation",
      course: demo205.id,
    });
    expect(after.would_block).toBeNull();
  });

  it("prices what Include sends, so a budget can stop only the run that includes", async () => {
    const api = createMockApi({ ...fast, scenario: "ai-key" });
    const demo101 = await courseId(api, "DEMO101");
    const { materials } = await api.weekMaterials(demo101.id);
    const assignment = materials.find((m) => m.title.startsWith("Assignment 4"));
    if (!assignment) throw new Error("week 4 has Assignment 4");
    const plain = { feature: "weekly_explanation", course: demo101.id } as const;
    const withAssignment = { ...plain, include: [assignment.id] };
    const base = await api.estimateGeneration(plain);
    // An id the week doesn't have is sent nowhere, so it costs nothing (like the facade).
    const unknown = await api.estimateGeneration({ ...plain, include: ["no-such-material"] });
    expect(unknown.input_tokens).toBe(base.input_tokens);
    const included = await api.estimateGeneration(withAssignment);
    expect(included.input_tokens).toBeGreaterThan(base.input_tokens);
    const [low, high] = [base.micro_usd_upper ?? 0, included.micro_usd_upper ?? 0];
    expect(high).toBeGreaterThan(low + 1);
    const { spent_micro_usd } = (await api.aiStatus()).budget;
    await api.setMonthlyBudget(spent_micro_usd + Math.floor((low + high) / 2));
    expect((await api.estimateGeneration(plain)).would_block).toBeNull();
    expect((await api.estimateGeneration(withAssignment)).would_block).toBe("budget_reached");
    // The run with the same include stops the same way, and the plain one doesn't.
    const options = { include: [assignment.id], override_budget: false, ui_language: "en" };
    await expect(
      api.explainWeek(demo101.id, null, "explain-include", options, () => {}),
    ).rejects.toMatchObject({ kind: "blocked", blocked: "budget_reached" });
  });

  it("prices only what Include lifts: a material read anyway or not graded-looking adds nothing", async () => {
    const api = createMockApi({ ...fast, scenario: "ai-key" });
    const demo101 = await courseId(api, "DEMO101");
    const { materials } = await api.weekMaterials(demo101.id);
    const slides = materials.find((m) => m.title.startsWith("Week 4 slides"));
    const practice = materials.find((m) => m.title.startsWith("Week 4 practice"));
    if (!slides || !practice) throw new Error("week 4 has its slides and practice questions");
    const plain = { feature: "weekly_explanation", course: demo101.id } as const;
    const base = await api.estimateGeneration(plain);
    for (const id of [slides.id, practice.id]) {
      const priced = await api.estimateGeneration({ ...plain, include: [id] });
      expect(priced.input_tokens).toBe(base.input_tokens);
      expect(priced.micro_usd_upper).toBe(base.micro_usd_upper);
    }
  });

  it("blocks a run over the budget until the cap is raised or removed", async () => {
    const api = createMockApi({ ...fast, scenario: "ai-budget" });
    expect((await api.aiStatus()).budget.spent_micro_usd).toBe(4_960_000);
    const demo101 = await courseId(api, "DEMO101");
    const req = { feature: "weekly_explanation", course: demo101.id } as const;
    // Over budget keeps its amount: the student decides on the per-run override with it.
    const over = await api.estimateGeneration(req);
    expect(over.would_block).toBe("budget_reached");
    expect(over.micro_usd_upper).toBeGreaterThan(0);
    await api.setMonthlyBudget(null);
    expect((await api.estimateGeneration(req)).would_block).toBeNull();
    await expect(api.setMonthlyBudget(-1)).rejects.toMatchObject({ kind: "invalid" });
  });

  it("asks once before using a model with no price", async () => {
    const api = createMockApi({ ...fast, scenario: "ai-unpriced" });
    const demo101 = await courseId(api, "DEMO101");
    const req = { feature: "weekly_explanation", course: demo101.id } as const;
    const first = await api.estimateGeneration(req);
    expect(first).toMatchObject({
      price_known: false,
      micro_usd_upper: null,
      would_block: "price_unknown_not_acknowledged",
    });
    expect(first.input_tokens).toBeGreaterThan(0);
    await api.acknowledgeUnpricedModel(openai, "gpt-6-preview-0929");
    expect((await api.estimateGeneration(req)).would_block).toBeNull();
  });

  it("costs nothing on this computer, where question (b) doesn't apply", async () => {
    const api = createMockApi({ ...fast, scenario: "ai-local" });
    const [backend] = (await api.aiStatus()).backends;
    expect(backend).toMatchObject({ kind: "local", state: "ready" });
    expect(backend?.disclosure).toMatchObject({ on_device: true, cost: "free_on_device" });
    const demo205 = await courseId(api, "DEMO205");
    const estimate = await api.estimateGeneration({
      feature: "weekly_explanation",
      course: demo205.id,
    });
    expect(estimate).toMatchObject({ micro_usd_upper: 0, would_block: null });
    const models = await api.listModels({ kind: "provider", provider_id: "ollama" });
    expect(models.filter((m) => !m.on_device).map((m) => m.id)).toEqual(["gpt-oss:120b-cloud"]);
  });

  it("asks again when the disclosure facts changed", async () => {
    const api = createMockApi({ ...fast, scenario: "ai-disclosure-changed" });
    const [backend] = (await api.aiStatus()).backends;
    expect(backend).toMatchObject({ state: "needs_disclosure", problems: ["disclosure_changed"] });
    const estimate = await api.estimateGeneration({ feature: "weekly_note" });
    // Fully estimated: only the acknowledgement stops the run.
    expect(estimate).toMatchObject({
      would_block: "disclosure_not_acknowledged",
      price_known: true,
    });
    expect(estimate.input_tokens).toBeGreaterThan(0);
    expect(estimate.micro_usd_upper).toBeGreaterThan(0);
  });

  it("reports model errors with their codes", async () => {
    const api = createMockApi({ ...fast, scenario: "ai-errors" });
    const [backend] = (await api.aiStatus()).backends;
    if (!backend) throw new Error("no backend");
    await expect(api.listModels(backend.backend)).rejects.toMatchObject({
      kind: "model",
      model_error: "network",
    });
    // Like the facade: a model error in "Test" is a failed probe, not a thrown error.
    expect(await api.testModel(backend.backend, "demo-model-large")).toMatchObject({
      ok: false,
      error: "rate_limited",
    });
    await expect(
      api.testModel({ kind: "provider", provider_id: "nope" }, "m"),
    ).rejects.toMatchObject({ kind: "not_found" });
  });

  it("sums usage per month and forgets everything on remove all", async () => {
    const now = () => new Date("2026-10-14T12:00:00Z");
    const api = createMockApi({ ...fast, now, scenario: "ai-key" });
    const month = await api.usageSummary(null);
    expect(month).toMatchObject({ month: "2026-10-01", total_micro_usd: 2_000_000 });
    expect((await api.usageSummary("2026-09-01")).rows).toHaveLength(2);
    expect((await api.usageSummary("2026-08-01")).rows).toEqual([]);
    await api.removeAllAiData();
    expect((await api.aiStatus()).backends).toEqual([]);
    expect((await api.usageSummary(null)).rows).toEqual([]);
  });

  it("removing a provider clears the features that used it", async () => {
    const api = createMockApi({ ...fast, scenario: "ai-key" });
    await api.removeModelProvider("openai");
    const status = await api.aiStatus();
    expect(status.backends).toEqual([]);
    expect(status.features.every((f) => f.choice === null)).toBe(true);
  });
});
