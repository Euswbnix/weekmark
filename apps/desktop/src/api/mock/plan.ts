// Study plans written by PageLamp in the mock (M3; design §5.1): the request's limits, the AI gate,
// the run's stages (Stop between them), a draft laid out like the scheduler's (study days only, at
// most 4 h a day, the rest unscheduled), and accepting and ticking the saved plan.

import type { GenEvent } from "../ai";
import type { PageLampApi } from "../client";
import { ApiError } from "../errors";
import type {
  DayOfWeek,
  GeneratedStudyPlan,
  PlanLimits,
  StudyPlanRequest,
  UnscheduledTask,
} from "../plan";
import type { StoredStudyPlan, StudyPlanItem } from "../types";
import { aiMaterialsState } from "../types";
import type { MockActivity } from "./activity";
import { isoOf } from "./calendar";
import type { MockCourse, MockDb } from "./fixtures";
import type { MockAiRun } from "./proposals";

type PlanApi = Pick<
  PageLampApi,
  "generateStudyPlan" | "planLimits" | "acceptStudyPlan" | "setStudyPlanItemDone"
>;

const DAYS: readonly DayOfWeek[] = [
  "sunday",
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
];
const MAX_MINUTES_PER_DAY = 240;

/** The facade's limits on a request (`plan_limits`). */
const LIMITS: PlanLimits = {
  min_horizon_days: 1,
  max_horizon_days: 56,
  default_horizon_days: 14,
  min_hours_per_week: 1,
  max_hours_per_week: 80,
  default_hours_per_week: 10,
  student_note_max_chars: 500,
};

export function createPlanMock(deps: {
  db: MockDb;
  now: () => Date;
  respond: <T>(value: T | (() => T), extraLatency?: number) => Promise<T>;
  step: () => Promise<void>;
  activity: MockActivity;
  /**
   * The AI gate for a study plan with the request's courses: who the run goes to, or ApiError
   * `blocked` (`no_course_to_plan` after `no_model_chosen`, as the facade).
   */
  gate: (courses: string[], horizonDays: number, overrideBudget: boolean) => Promise<MockAiRun>;
  /** The courses the plan covers (`planCourses` in the mock's index). */
  planCourses: (wanted: readonly string[]) => MockCourse[];
}): PlanApi & { cancel: (generationId: string) => void } {
  const { db, now, respond } = deps;
  const drafts = new Map<string, GeneratedStudyPlan>();
  const accepted = new Set<string>();
  const cancelled = new Set<string>();
  const running = new Set<string>();

  async function write(
    request: StudyPlanRequest,
    generationId: string,
    onEvent: (event: GenEvent) => void,
  ): Promise<GeneratedStudyPlan> {
    const horizon = request.horizon_days ?? LIMITS.default_horizon_days;
    const hours = request.hours_per_week ?? LIMITS.default_hours_per_week;
    const daysOff = new Set(request.days_off ?? []);
    if (horizon < LIMITS.min_horizon_days || horizon > LIMITS.max_horizon_days) {
      throw new ApiError("invalid", "The plan covers 1 to 56 days.");
    }
    if (hours < LIMITS.min_hours_per_week || hours > LIMITS.max_hours_per_week) {
      throw new ApiError("invalid", "Study hours per week are 1 to 80.");
    }
    if (daysOff.size >= 7) throw new ApiError("invalid", "Leave at least one study day.");
    const wanted = request.courses ?? [];
    const run = await deps.gate(wanted, horizon, request.override_budget ?? false);
    const courses = deps.planCourses(wanted);
    const stop = () => {
      if (cancelled.has(generationId)) {
        onEvent({ type: "finished", ok: false });
        throw new ApiError("cancelled", "The plan was stopped.");
      }
    };
    onEvent({ type: "started", generation_id: generationId, ...run });
    for (const stage of ["building_context", "waiting_for_model", "scheduling"] as const) {
      onEvent({ type: "stage", stage });
      await deps.step();
      stop();
    }
    const usage = { input_tokens: 9_800, cached_input_tokens: 0, output_tokens: 1_600 };
    onEvent({ type: "usage", usage });
    onEvent({ type: "finished", ok: true });

    const today = now();
    const start = isoOf(today);
    const end = isoOf(addDays(today, horizon - 1));
    const studyDays = Array.from({ length: horizon }, (_, i) => addDays(today, i)).filter(
      (d) => !daysOff.has(DAYS[d.getDay()] as DayOfWeek),
    );
    const perDay = Math.min(MAX_MINUTES_PER_DAY, Math.round((hours * 60) / (7 - daysOff.size)));

    // The "model's" tasks: each course's materials to read, then a review before its deadline.
    const tasks = courses.flatMap((c) => [
      ...c.materials.slice(0, 3).map((m) => ({
        course_id: c.course.id,
        title: `Read ${m.title}`,
        material_ids: [m.id],
        minutes: 45,
      })),
      { course_id: c.course.id, title: "Review this week's notes", material_ids: [], minutes: 30 },
    ]);
    const items: StudyPlanItem[] = [];
    const unscheduled: UnscheduledTask[] = [];
    const used = new Map<string, number>();
    let day = 0;
    for (const task of tasks) {
      while (
        day < studyDays.length &&
        (used.get(isoOf(studyDays[day] as Date)) ?? 0) + task.minutes > perDay
      ) {
        day += 1;
      }
      const date = studyDays[day];
      if (!date) {
        unscheduled.push({
          course_id: task.course_id,
          title: task.title,
          reason: studyDays.length === 0 ? "no_study_days" : "no_time_before_latest",
        });
        continue;
      }
      const iso = isoOf(date);
      used.set(iso, (used.get(iso) ?? 0) + task.minutes);
      items.push({ ...task, date: iso, done: false });
    }

    const plan: GeneratedStudyPlan = {
      meta: {
        backend_label: run.backend_label,
        model: run.model,
        on_device: run.on_device,
        created_at: today.toISOString(),
        generation_id: generationId,
        feature: "study_plan",
        estimated: false,
        prompt_version: 1,
        usage,
        est_cost_micro_usd: run.on_device ? null : 4_200,
        context: {
          courses: courses.map((c) => {
            const state = aiMaterialsState(c.course);
            return { course_id: c.course.id, state, text_included: state === "readable" };
          }),
          left_out: [],
          materials_included: courses
            .filter((c) => aiMaterialsState(c.course) === "readable")
            .reduce((n, c) => n + Math.min(3, c.materials.length), 0),
          materials_trimmed: 0,
        },
      },
      plan: { horizon_start: start, horizon_end: end, items },
      unscheduled,
      // The "model" also proposed doing an assignment itself: dropped, only counted.
      warnings: [{ code: "graded_work_left_out", count: 1 }],
    };
    drafts.set(generationId, plan);
    return plan;
  }

  return {
    planLimits: () => respond({ ...LIMITS }),
    generateStudyPlan: async (request, generationId, onEvent) => {
      if (running.has(generationId)) throw new ApiError("busy", "This plan is being written.");
      running.add(generationId);
      try {
        return await deps.activity.during("generation", { generation_id: generationId }, () =>
          write(request, generationId, onEvent),
        );
      } finally {
        running.delete(generationId);
        cancelled.delete(generationId);
      }
    },
    acceptStudyPlan: (generationId) =>
      respond(() => {
        const draft = drafts.get(generationId);
        if (!draft)
          throw new ApiError("not_found", `There is no study plan draft ${generationId}.`);
        if (accepted.has(generationId)) {
          throw new ApiError("invalid", "This draft is already your plan.");
        }
        accepted.add(generationId);
        const stored: StoredStudyPlan = {
          id: (db.studyPlan?.id ?? 0) + 1,
          created_at: now().toISOString(),
          origin: "pagelamp",
          generation_id: generationId,
          // Kept with the plan, like a calendar's (also after "Remove all AI data").
          ai_label: {
            backend_label: draft.meta.backend_label,
            model: draft.meta.model,
            created_at: draft.meta.created_at,
            on_device: draft.meta.on_device,
          },
          plan: structuredClone(draft.plan),
        };
        db.studyPlan = stored;
        return stored;
      }),
    setStudyPlanItemDone: (planId, itemIndex, done) =>
      respond(() => {
        const plan = db.studyPlan;
        const item = plan?.id === planId ? plan.plan.items[itemIndex] : undefined;
        if (!plan || !item) throw new ApiError("not_found", "No such plan item.");
        item.done = done;
        return plan;
      }),
    cancel: (generationId) => {
      if (running.has(generationId)) cancelled.add(generationId);
    },
  };
}

function addDays(date: Date, days: number): Date {
  const next = new Date(date);
  next.setDate(next.getDate() + days);
  return next;
}
