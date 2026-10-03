// Weekly explanations in the mock (M3; design §5.2): the course's rules first (a course that isn't
// readable is blocked with its reason), the AI gate, the run's events (no text before the end),
// an explanation of the week's readable materials with every paragraph cited, and the last 5 kept
// per course and week, newest first. Like the facade: `week: null` explains the week it resolves
// to, else the materials of the last 14 days (weeks unknown); saved(null) is every week; what
// looks like graded work is left out unless included, and nothing brings back one over budget.

import { type GenEvent, materialSharing } from "../ai";
import type { PageLampApi } from "../client";
import { ApiError } from "../errors";
import type { WeeklyExplanation } from "../explain";
import type { LeftOutMaterial, LeftOutReason } from "../plan";
import { aiMaterialsState, type MaterialView } from "../types";
import type { MockActivity } from "./activity";
import type { MockCourse } from "./fixtures";
import type { MockAiRun } from "./proposals";

type ExplainApi = Pick<
  PageLampApi,
  | "explainWeek"
  | "savedExplanations"
  | "deleteExplanation"
  | "aiOutputLanguage"
  | "setAiOutputLanguage"
>;

const KEEP = 5;
const DAY = 24 * 60 * 60 * 1000;

/** pagelamp-core's looks_like_assessment: an assessment word, and no study word. */
export const ASSESSMENT_WORDS = [
  "assignment",
  "homework",
  "hw",
  "problem set",
  "pset",
  "quiz",
  "exam",
  "midterm",
  "test",
  "lab report",
];
export const STUDY_WORDS = [
  "review",
  "practice",
  "solution",
  "solutions",
  "notes",
  "lecture",
  "slides",
  "preparation",
];

export function looksLikeAssessment(title: string): boolean {
  const words = title
    .toLowerCase()
    .split(/[^\p{L}\p{N}]+/u)
    .filter((w) => w !== "");
  const joined = ` ${words.join(" ")} `;
  const has = (phrase: string) =>
    joined.includes(` ${phrase} `) || (phrase === "hw" && words.some((w) => /^hw\d+$/.test(w)));
  return ASSESSMENT_WORDS.some(has) && !STUDY_WORDS.some(has);
}

/**
 * What an explanation reads and leaves out, in pagelamp-core's order (left_out): no readable
 * text first (whatever the title says), then what looks like graded work unless included;
 * the first two readable materials fit the budget and the rest are left out for it, which
 * `include` doesn't change.
 */
/**
 * The one left-out reason `ExplainOptions.include` brings back (the facade's
 * `LeftOutReason::includable`): a material that looks like graded work, once the student says it
 * isn't. Materials over the length limit stay out whatever `include` says.
 */
export const INCLUDABLE_REASON: LeftOutReason = "looks_like_assessment";

export function selectMaterials(
  materials: MaterialView[],
  include: string[],
): { read: MaterialView[]; leftOut: LeftOutMaterial[] } {
  const leftOut: LeftOutMaterial[] = [];
  const readable: MaterialView[] = [];
  for (const m of materials) {
    if (m.text_status !== "ok") {
      leftOut.push({ material_id: m.id, title: m.title, reason: "no_text", includable: false });
    } else if (looksLikeAssessment(m.title) && !include.includes(m.id)) {
      leftOut.push({
        material_id: m.id,
        title: m.title,
        reason: INCLUDABLE_REASON,
        includable: true,
      });
    } else {
      readable.push(m);
    }
  }
  // The first two, in the week's order, stand in for the facade's length budget. An included
  // material gets no priority there (every candidate gets a fair share), so here neither: it
  // can come back over the length limit like any other.
  const read = readable.slice(0, 2);
  for (const m of readable.slice(2)) {
    leftOut.push({ material_id: m.id, title: m.title, reason: "over_budget", includable: false });
  }
  return { read, leftOut };
}

/** A graded-looking material the student included: the only kind `include` lifts. */
function lifts(m: MaterialView, include: string[]): boolean {
  return include.includes(m.id) && looksLikeAssessment(m.title);
}

/**
 * The week an explanation reads, as pagelamp-core's week_materials picks it: `week`, else the
 * course's default week, else the last 14 days.
 */
export function explanationWeek(
  c: MockCourse,
  requestedWeek: number | null,
  now: Date,
): { week: number | null; materials: MaterialView[] } {
  const week = requestedWeek ?? c.timeline.default_week ?? null;
  if (week !== null) return { week, materials: c.materials.filter((m) => m.week_hint === week) };
  const until = now.getTime();
  const since = until - 14 * DAY;
  const materials = c.materials.filter((m) => {
    const at = m.published_at ? Date.parse(m.published_at) : Number.NaN;
    return at >= since && at <= until;
  });
  return { week, materials };
}

/**
 * What an explanation of that week sends of `include`: the graded-looking materials it lifts and
 * reads (pagelamp-core's week_context_including). The estimate prices these, as the run does;
 * an id that names nothing there adds nothing.
 */
export function liftedIncludes(
  c: MockCourse,
  requestedWeek: number | null,
  include: string[],
  now: Date,
): string[] {
  const { materials } = explanationWeek(c, requestedWeek, now);
  return liftedOf(selectMaterials(materials, include).read, include);
}

function liftedOf(read: MaterialView[], include: string[]): string[] {
  return read.filter((m) => lifts(m, include)).map((m) => m.id);
}

export function createExplainMock(deps: {
  now: () => Date;
  respond: <T>(value: T | (() => T), extraLatency?: number) => Promise<T>;
  step: () => Promise<void>;
  activity: MockActivity;
  /** `include`: the materials the run sends although they look like assessments. */
  gate: (
    courseId: string,
    week: number | null,
    include: string[],
    overrideBudget: boolean,
  ) => Promise<MockAiRun>;
  findCourse: (courseId: string) => MockCourse;
}): ExplainApi & { cancel: (generationId: string) => void } {
  const { now, respond } = deps;
  const saved = new Map<string, WeeklyExplanation[]>();
  const cancelled = new Set<string>();
  const running = new Set<string>();
  const reminded = new Set<string>();
  let language: "ui" | "course" = "ui";

  const key = (courseId: string, week: number | null) => `${courseId}#${week ?? "recent"}`;

  async function write(
    courseId: string,
    requestedWeek: number | null,
    generationId: string,
    include: string[],
    overrideBudget: boolean,
    onEvent: (event: GenEvent) => void,
  ): Promise<WeeklyExplanation> {
    const c = deps.findCourse(courseId);
    const state = aiMaterialsState(c.course);
    if (c.course.hidden) {
      throw new ApiError("blocked", "The course is hidden.", { blocked: "course_hidden" });
    }
    if (state === "withheld_by_policy") {
      throw new ApiError("blocked", "The course's AI policy withholds its materials.", {
        blocked: "course_policy_prohibited",
      });
    }
    if (state === "turned_off") {
      throw new ApiError("blocked", "AI access is off for this course.", {
        blocked: "course_ai_turned_off",
      });
    }
    const { week, materials } = explanationWeek(c, requestedWeek, now());
    const { read, leftOut } = selectMaterials(materials, include);
    if (read.length === 0) {
      throw new ApiError("blocked", "No readable materials this week.", {
        blocked: "no_readable_materials",
      });
    }
    // Priced with what the run sends, as the facade's run checks its own prompt.
    const run = await deps.gate(c.course.id, week, liftedOf(read, include), overrideBudget);
    const stop = () => {
      if (cancelled.has(generationId)) {
        onEvent({ type: "finished", ok: false });
        throw new ApiError("cancelled", "The explanation was stopped.");
      }
    };

    onEvent({ type: "stage", stage: "building_context" });
    await deps.step();
    stop();
    const summary = {
      courses: [{ course_id: c.course.id, state, text_included: true }],
      left_out: leftOut,
      materials_included: read.length,
      materials_trimmed: 0,
    };
    onEvent({ type: "context", summary, input_tokens: 6_400 * read.length });
    onEvent({ type: "started", generation_id: generationId, ...run });
    onEvent({ type: "stage", stage: "waiting_for_model" });
    await deps.step();
    stop();
    const usage = {
      input_tokens: 6_400 * read.length,
      cached_input_tokens: 0,
      output_tokens: 1_100,
    };
    onEvent({ type: "usage", usage });
    onEvent({ type: "stage", stage: "validating" });
    await deps.step();
    stop();
    onEvent({ type: "finished", ok: true });

    const answer = materialSharing(c.course);
    const reminder =
      !run.on_device &&
      (answer === "unanswered" || answer === "not_sure") &&
      !reminded.has(c.course.id);
    if (reminder) reminded.add(c.course.id);
    const explanation: WeeklyExplanation = {
      meta: {
        backend_label: run.backend_label,
        model: run.model,
        on_device: run.on_device,
        created_at: now().toISOString(),
        generation_id: generationId,
        feature: "weekly_explanation",
        estimated: false,
        prompt_version: 1,
        usage,
        est_cost_micro_usd: run.on_device ? null : 3_100,
        context: summary,
      },
      course_id: c.course.id,
      week,
      sections: read.map((m, i) => ({
        heading: m.title,
        paragraphs: [
          {
            text: `The key idea of **${m.title}** is how this week's topic builds on the last one.`,
            citations: [cite(m, i + 1, "p. 2")],
          },
          {
            text: "Work through the example at the end before the next lecture.",
            citations: [cite(m, i + 1, "p. 5")],
          },
        ],
      })),
      check_questions: read.map((m) => `What is the main point of ${m.title}?`),
      left_out: leftOut,
      stale: false,
      sharing_reminder: reminder,
      dropped_citations: 0,
      cite_ai_use: c.course.ai_policy === "allowed_with_citation",
    };
    const list = saved.get(key(c.course.id, week)) ?? [];
    saved.set(key(c.course.id, week), [explanation, ...list].slice(0, KEEP));
    return explanation;
  }

  return {
    explainWeek: async (courseId, week, generationId, options, onEvent) => {
      if (running.has(generationId))
        throw new ApiError("busy", "This explanation is being written.");
      running.add(generationId);
      try {
        return await deps.activity.during("generation", { generation_id: generationId }, () =>
          write(
            courseId,
            week,
            generationId,
            options.include ?? [],
            options.override_budget ?? false,
            onEvent,
          ),
        );
      } finally {
        running.delete(generationId);
        cancelled.delete(generationId);
      }
    },
    savedExplanations: (courseId, week) =>
      respond(() => {
        const c = deps.findCourse(courseId);
        if (week !== null && week !== undefined) return saved.get(key(c.course.id, week)) ?? [];
        // Like the facade: every week, newest first.
        return [...saved.entries()]
          .filter(([k]) => k.startsWith(`${c.course.id}#`))
          .flatMap(([, list]) => list)
          .sort((a, b) => b.meta.created_at.localeCompare(a.meta.created_at));
      }),
    deleteExplanation: (generationId) =>
      respond(() => {
        for (const [k, list] of saved) {
          const kept = list.filter((e) => e.meta.generation_id !== generationId);
          if (kept.length < list.length) {
            saved.set(k, kept);
            return;
          }
        }
        throw new ApiError("not_found", `There is no explanation ${generationId}.`);
      }),
    aiOutputLanguage: () => respond(language),
    setAiOutputLanguage: (next) =>
      respond(() => {
        language = next;
      }),
    cancel: (generationId) => {
      if (running.has(generationId)) cancelled.add(generationId);
    },
  };
}

function cite(material: MaterialView, n: number, locator: string) {
  return {
    handle: `M${n}`,
    material_id: material.id,
    title: material.title,
    locator,
    url: material.url ?? null,
  };
}
