import { describe, expect, it } from "vitest";
import type { ApiError } from "../errors";
import { createMockApi } from ".";

const NOW = new Date(2026, 8, 28, 10, 0); // Monday 2026-09-28
// "proposals" routes AI to an API key, so a plan can be written.
const fast = { latencyMs: 0, syncStepMs: 0, now: () => NOW, scenario: "proposals" as const };
const openai = { kind: "provider", provider_id: "openai" } as const;
const events = () => {
  const list: string[] = [];
  return { list, onEvent: (e: { type: string }) => list.push(e.type) };
};

describe("mock study plans", () => {
  it("checks the request like the facade", async () => {
    const api = createMockApi(fast);
    const { onEvent } = events();
    for (const request of [
      { horizon_days: 0 },
      { horizon_days: 57 },
      { hours_per_week: 81 },
      {
        days_off: [
          "monday",
          "tuesday",
          "wednesday",
          "thursday",
          "friday",
          "saturday",
          "sunday",
        ] as const,
      },
    ]) {
      await expect(
        api.generateStudyPlan(
          { ...request, days_off: [...(request.days_off ?? [])] },
          "g",
          onEvent,
        ),
      ).rejects.toMatchObject({ kind: "invalid" } satisfies Partial<ApiError>);
    }
  });

  it("answers the limits it checks", async () => {
    const api = createMockApi(fast);
    const { onEvent } = events();
    const limits = await api.planLimits();
    for (const request of [
      { horizon_days: limits.min_horizon_days - 1 },
      { horizon_days: limits.max_horizon_days + 1 },
      { hours_per_week: limits.min_hours_per_week - 1 },
      { hours_per_week: limits.max_hours_per_week + 1 },
    ]) {
      await expect(api.generateStudyPlan(request, "g", onEvent)).rejects.toMatchObject({
        kind: "invalid",
      } satisfies Partial<ApiError>);
    }
  });

  it("writes a draft on study days only, at most 4 hours a day, and saves it when accepted", async () => {
    const api = createMockApi(fast);
    const { list, onEvent } = events();
    const draft = await api.generateStudyPlan(
      { horizon_days: 14, hours_per_week: 40, days_off: ["saturday", "sunday"] },
      "plan-1",
      onEvent,
    );
    expect(list).toEqual(["started", "stage", "stage", "stage", "usage", "finished"]);
    expect(draft.plan.horizon_start).toBe("2026-09-28");
    expect(draft.plan.horizon_end).toBe("2026-10-11");
    const weekdays = draft.plan.items.map((i) => new Date(`${i.date}T12:00`).getDay());
    expect(weekdays.every((d) => d !== 0 && d !== 6)).toBe(true);
    const perDay = new Map<string, number>();
    for (const item of draft.plan.items) {
      perDay.set(item.date, (perDay.get(item.date) ?? 0) + (item.minutes ?? 0));
    }
    expect(Math.max(...perDay.values())).toBeLessThanOrEqual(240);
    expect(draft.warnings).toEqual([{ code: "graded_work_left_out", count: 1 }]);
    expect(await api.latestStudyPlan()).not.toMatchObject({ generation_id: "plan-1" });

    const stored = await api.acceptStudyPlan("plan-1");
    expect(stored).toMatchObject({
      origin: "pagelamp",
      generation_id: "plan-1",
      ai_label: { backend_label: draft.meta.backend_label, model: draft.meta.model },
    });
    expect(await api.latestStudyPlan()).toEqual(stored);
    await expect(api.acceptStudyPlan("plan-1")).rejects.toMatchObject({ kind: "invalid" });
    await expect(api.acceptStudyPlan("nope")).rejects.toMatchObject({ kind: "not_found" });

    const ticked = await api.setStudyPlanItemDone(stored.id, 0, true);
    expect(ticked.plan.items[0]?.done).toBe(true);
    await expect(api.setStudyPlanItemDone(stored.id, 999, true)).rejects.toMatchObject({
      kind: "not_found",
    });
  });

  it("stops at the next stage, and is listed as a generation while it runs", async () => {
    const api = createMockApi({ ...fast, syncStepMs: 20 });
    const running = api.generateStudyPlan({}, "plan-2", () => {});
    await new Promise((resolve) => setTimeout(resolve, 5));
    expect((await api.activity()).items).toMatchObject([
      { kind: "generation", generation_id: "plan-2" },
    ]);
    await api.cancelGeneration("plan-2");
    await expect(running).rejects.toMatchObject({ kind: "cancelled" });
    expect((await api.activity()).items).toEqual([]);
  });

  it("is blocked without a model", async () => {
    const api = createMockApi({ ...fast, scenario: "demo" });
    await expect(api.generateStudyPlan({}, "g", () => {})).rejects.toMatchObject({
      kind: "blocked",
    });
  });

  it("leaves a hidden course out of the default plan, and plans a course named twice once", async () => {
    const api = createMockApi(fast);
    const active = (await api.listCourses())
      .filter((c) => !c.course.hidden && c.lifecycle.is_active)
      .map((c) => c.course.id);
    const [hidden, kept] = active;
    expect(hidden && kept).toBeTruthy();
    await api.setCourseHidden(hidden ?? "", true);
    const draft = await api.generateStudyPlan({}, "p-1", () => {});
    const planned = draft.meta.context?.courses.map((c) => c.course_id) ?? [];
    expect(planned).toContain(kept);
    expect(planned).not.toContain(hidden);
    const twice = await api.generateStudyPlan(
      { courses: [kept ?? "", kept ?? ""] },
      "p-2",
      () => {},
    );
    expect(twice.meta.context?.courses.map((c) => c.course_id)).toEqual([kept]);
  });

  it("blocks no course to plan for before the click, after no model chosen, like the facade", async () => {
    // Only past courses: none is active.
    const api = createMockApi({ ...fast, scenario: "all-past" });
    const estimate = (courses: string[]) =>
      api.estimateGeneration({ feature: "study_plan", courses, horizon_days: 14 });
    expect((await estimate([])).would_block).toBe("no_model_chosen");
    await api.addModelProvider("openai", null, "sk-demo-key-7731");
    const version = (await api.aiStatus()).backends[0]?.disclosure.version ?? 0;
    await api.acknowledgeAiDisclosure(openai, version);
    await api.setFeatureModel("study_plan", {
      backend: openai,
      model: "gpt-5.4-mini",
      effort: "lowest",
    });
    expect(await estimate([])).toMatchObject({
      would_block: "no_course_to_plan",
      micro_usd_upper: null,
      input_tokens: 0,
    });
    await expect(api.generateStudyPlan({}, "g-1", () => {})).rejects.toMatchObject({
      kind: "blocked",
      blocked: "no_course_to_plan",
    } satisfies Partial<ApiError>);
    // A course named on purpose is planned although it has ended; hidden, it's left out.
    const [ended] = await api.listCourses();
    const id = ended?.course.id ?? "";
    expect((await estimate([id])).would_block).toBeNull();
    const draft = await api.generateStudyPlan({ courses: [id] }, "g-2", () => {});
    expect(draft.meta.context?.courses.map((c) => c.course_id)).toEqual([id]);
    await api.setCourseHidden(id, true);
    expect((await estimate([id])).would_block).toBe("no_course_to_plan");
    await expect(api.generateStudyPlan({ courses: [id] }, "g-3", () => {})).rejects.toMatchObject({
      kind: "blocked",
      blocked: "no_course_to_plan",
    } satisfies Partial<ApiError>);
    await expect(estimate(["no-such-course"])).rejects.toMatchObject({ kind: "not_found" });
    expect((await api.activity()).items).toEqual([]);
  });
});
