// The AI weekly note in the mock (beta.2; design §5.3), with the facade's rules: structure and plan
// progress only (no material text, so no course is blocked); without an active course, a deadline
// in the next 7 days or a plan item it is blocked with nothing_to_write (the estimate says so
// first, after no model chosen); the run's events and Stop; the last 5 notes kept, newest first.
// "Prepare it on Monday" is allowed only for an API key or a model on this computer; an automatic
// run is refused unless it's due (a week with nothing to write about isn't), records its try as it
// starts, and never goes over the budget.

import type { BackendKind, GenEvent } from "../ai";
import type { PageLampApi } from "../client";
import { ApiError } from "../errors";
import type { CourseLifecycle, Deadline } from "../types";
import type { WeeklyNote, WeeklyNoteSettings } from "../weeklyNote";
import type { MockActivity } from "./activity";
import type { MockCourse, MockScenario } from "./fixtures";
import type { MockAiRun } from "./proposals";

type WeeklyNoteApi = Pick<
  PageLampApi,
  | "writeWeeklyNote"
  | "weeklyNotes"
  | "deleteWeeklyNote"
  | "weeklyNoteSettings"
  | "setPrepareWeeklyNoteOnMonday"
>;

const KEEP = 5;

/** YYYY-MM-DD of `date` in local time. */
function localDay(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

/** The Monday of `date`'s week, local time. */
function mondayOf(date: Date): string {
  const back = (date.getDay() + 6) % 7;
  return localDay(new Date(date.getFullYear(), date.getMonth(), date.getDate() - back));
}

export function createWeeklyNoteMock(deps: {
  scenario: MockScenario;
  now: () => Date;
  respond: <T>(value: T | (() => T), extraLatency?: number) => Promise<T>;
  step: () => Promise<void>;
  activity: MockActivity;
  courses: () => MockCourse[];
  lifecycleOf: (c: MockCourse) => CourseLifecycle;
  deadlinesWithin: (courses: MockCourse[], daysAhead: number, daysBack: number) => Deadline[];
  /** The week has nothing to write about (the facade's writable_note_context). */
  nothingToWrite: () => boolean;
  /** The AI gate for the note (the student may go over a reached budget, never automatically). */
  gate: (overrideBudget: boolean) => Promise<MockAiRun>;
  /** Who runs the note's model now (null: no model chosen). */
  noteBackend: () => Promise<BackendKind | null>;
}): WeeklyNoteApi & { cancel: (generationId: string) => void; prepareNow: () => Promise<boolean> } {
  const { now, respond } = deps;
  let notes: WeeklyNote[] = [];
  let prepareOnMonday = deps.scenario === "weekly-note-monday";
  /** The day of the last automatic try (recorded as it starts, whatever its outcome). */
  let triedOn: string | null = null;
  const cancelled = new Set<string>();
  const running = new Set<string>();

  async function allowed(): Promise<boolean> {
    const kind = await deps.noteBackend();
    return kind === "api_key" || kind === "local";
  }

  /** It's Monday (the "weekly-note-monday" scenario pretends so), in the mock's clock. */
  function isMonday(): boolean {
    return deps.scenario === "weekly-note-monday" || now().getDay() === 1;
  }

  /** What startup_tasks().prepare_weekly_note says. */
  async function prepareNow(): Promise<boolean> {
    const today = localDay(now());
    return (
      prepareOnMonday &&
      isMonday() &&
      triedOn !== today &&
      !notes.some((n) => localDay(new Date(n.meta.created_at)) === today) &&
      (await allowed()) &&
      // Last, like the facade: nothing to write about isn't due, so no try is recorded.
      !deps.nothingToWrite()
    );
  }

  async function write(
    generationId: string,
    automatic: boolean,
    overrideBudget: boolean,
    onEvent: (event: GenEvent) => void,
  ): Promise<WeeklyNote> {
    if (automatic) {
      if (!(await prepareNow())) {
        throw new ApiError("invalid", "The weekly note isn't due.");
      }
      triedOn = localDay(now());
    }
    const visible = deps.courses().filter((c) => !c.course.hidden);
    const active = visible.filter((c) => deps.lifecycleOf(c).is_active);
    const deadlines = deps.deadlinesWithin(visible, 7, 0);
    // The gate refuses no model chosen, then nothing to write about (from the estimate, as the
    // facade does), then the rest. Automatic runs never go over the budget.
    const run = await deps.gate(automatic ? false : overrideBudget);
    const stop = () => {
      if (cancelled.has(generationId)) {
        onEvent({ type: "finished", ok: false });
        throw new ApiError("cancelled", "The weekly note was stopped.");
      }
    };
    onEvent({ type: "stage", stage: "building_context" });
    await deps.step();
    stop();
    const summary = {
      courses: active.map((c) => ({
        course_id: c.course.id,
        state: "readable" as const,
        text_included: false,
      })),
      left_out: [],
      materials_included: 0,
      materials_trimmed: 0,
    };
    onEvent({ type: "context", summary, input_tokens: 2_400 });
    onEvent({ type: "started", generation_id: generationId, ...run });
    onEvent({ type: "stage", stage: "waiting_for_model" });
    await deps.step();
    stop();
    const usage = { input_tokens: 2_400, cached_input_tokens: 0, output_tokens: 420 };
    onEvent({ type: "usage", usage });
    onEvent({ type: "stage", stage: "validating" });
    await deps.step();
    stop();
    onEvent({ type: "finished", ok: true });

    const byCourse = new Map(visible.map((c) => [c.course.id, c]));
    const focus = deadlines.slice(0, 3).map((d) => {
      const course = d.course_id ? byCourse.get(d.course_id) : undefined;
      return {
        course_id: course ? course.course.id : null,
        text: `Get ahead on ${d.title} before it's due.`,
      };
    });
    const note: WeeklyNote = {
      automatic,
      focus,
      graded_work_left_out: 0,
      meta: {
        backend_label: run.backend_label,
        model: run.model,
        on_device: run.on_device,
        created_at: now().toISOString(),
        generation_id: generationId,
        feature: "weekly_note",
        estimated: false,
        prompt_version: 1,
        usage,
        est_cost_micro_usd: run.on_device ? null : 600,
        context: summary,
      },
      text:
        `This week you have ${deadlines.length} deadlines across ${active.length} active courses. ` +
        "Start with the earliest one, and keep an hour for this week's readings. " +
        "Your study plan is on track so far.",
      week_of: mondayOf(now()),
    };
    notes = [note, ...notes].slice(0, KEEP);
    return note;
  }

  const settings = async (): Promise<WeeklyNoteSettings> => ({
    prepare_on_monday: prepareOnMonday,
    prepare_on_monday_allowed: await allowed(),
  });

  return {
    writeWeeklyNote: async (generationId, options, onEvent) => {
      if (running.has(generationId)) throw new ApiError("busy", "This note is being written.");
      running.add(generationId);
      try {
        return await deps.activity.during("generation", { generation_id: generationId }, () =>
          write(
            generationId,
            options.automatic ?? false,
            options.override_budget ?? false,
            onEvent,
          ),
        );
      } finally {
        running.delete(generationId);
        cancelled.delete(generationId);
      }
    },
    weeklyNotes: () => respond(() => notes),
    deleteWeeklyNote: (generationId) =>
      respond(() => {
        const kept = notes.filter((n) => n.meta.generation_id !== generationId);
        if (kept.length === notes.length) {
          throw new ApiError("not_found", `There is no weekly note ${generationId}.`);
        }
        notes = kept;
      }),
    weeklyNoteSettings: async () => respond(await settings()),
    setPrepareWeeklyNoteOnMonday: async (on) => {
      if (on && !(await allowed())) {
        throw new ApiError(
          "invalid",
          "Preparing the note on Monday needs an API key or a model on this computer.",
        );
      }
      prepareOnMonday = on;
      return respond(await settings());
    },
    cancel: (generationId) => {
      if (running.has(generationId)) cancelled.add(generationId);
    },
    prepareNow,
  };
}
