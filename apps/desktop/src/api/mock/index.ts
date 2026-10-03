// In-memory implementation of PageLampApi for `pnpm dev:mock` and tests.
//
// It behaves like the real facade where the UI can tell the difference: validation errors use
// the same AppError kinds, sync streams SyncEvents over time, and settings persist for the
// session. Pick a state to look at with `?scenario=` in the URL, e.g.
//   http://localhost:1420/?scenario=expired#/sources
// Scenarios: demo (default) · empty · expired · error · busy · crashed; updates (M0.4):
// update-available · upgrader · upgrader-from-01 · upgrader-from-alpha1 · updated · deb; worker-blocked (M0.5); AI setup
// (M1): ai-key · ai-local · ai-unpriced · ai-budget · ai-disclosure-changed · ai-errors; the
// ChatGPT plan (M2), offered only in these: codex-not-installed · codex-signed-out · codex-plus ·
// codex-edu · codex-api-key · codex-outdated-pin · codex-outdated-app · codex-free · codex-cap
// (elsewhere the plan isn't offered, as in every build until OpenAI confirms in writing);
// course weeks and lifecycle (M0.10): uoft-fall · phases · all-past (see courseScenarios.ts);
// reminders (M3): reminders-due · reminders-no-tray (see reminders.ts).
// weekly note (beta.2): weekly-note-monday (opted in, an API key, Monday; see weeklyNote.ts).
//
// Secrets passed to this mock (tokens, feed URLs) are validated and then dropped — never stored,
// never logged.

import { type EstimateRequest, sameBackend } from "../ai";
import type { AvailableUpdate, PageLampApi } from "../client";
import { ApiError } from "../errors";
import {
  type AppStatus,
  aiMaterialsState,
  type CourseSummary,
  type CourseTimeline,
  type Deadline,
  type MaterialView,
  type SourceKind,
  type SourceRecord,
  type SourceSyncResult,
  type SyncEvent,
  type SyncRequest,
  type SyncSummary,
  type UpdateChannel,
  type UpdateCheckRecord,
} from "../types";
import { createMockActivity } from "./activity";
import { createMockAi } from "./ai";
import {
  addDays,
  defaultKeepUntil,
  isoOf,
  withCourseDates,
  withoutStudentDates,
  withStudentDates,
} from "./calendar";
import { buildCalendarScenarioDb } from "./courseScenarios";
import { createExplainMock } from "./explain";
import {
  buildMockDb,
  diagnosticReport,
  MOCK_APP_VERSION,
  MOCK_BINARY_PATH,
  MOCK_PREVIOUS_VERSION,
  MOCK_UPDATE_VERSION,
  type MockCourse,
  type MockDb,
  type MockScenario,
  mcpClientConfigs,
} from "./fixtures";
import { createPlanMock } from "./plan";
import { createProposalsMock, type MockAiRun } from "./proposals";
import { createRemindersMock } from "./reminders";
import { createLifecycleMock } from "./removal";
import { createWeeklyNoteMock } from "./weeklyNote";
import { whatsNewSince } from "./whatsNew";

export { MOCK_SCENARIOS, type MockScenario } from "./fixtures";

export interface MockOptions {
  scenario?: MockScenario;
  /** Simulated latency of every call in ms (0 in tests). */
  latencyMs?: number;
  /** Delay between streamed sync events in ms. */
  syncStepMs?: number;
  /** Fixed clock for deterministic tests. */
  now?: () => Date;
}

const DAY = 24 * 60 * 60 * 1000;
/** startup_tasks lists at most this many offers and suggestions (the facade's STARTUP_LIST_MAX). */
const STARTUP_LIST_MAX = 20;

function sleep(ms: number): Promise<void> {
  return ms > 0 ? new Promise((resolve) => setTimeout(resolve, ms)) : Promise.resolve();
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

function whenOf(d: Deadline): number | null {
  const iso = d.due_at ?? d.starts_at;
  return iso ? Date.parse(iso) : null;
}

function parseHttpUrl(value: string, allowWebcal = false): URL | null {
  try {
    const url = new URL(value.trim());
    const ok =
      url.protocol === "https:" ||
      url.protocol === "http:" ||
      (allowWebcal && url.protocol === "webcal:");
    return ok ? url : null;
  } catch {
    return null;
  }
}

/**
 * Mirrors the backend's normalize_base_url (pagelamp-canvas), in the same order: a pasted token
 * ("7~AbC…") or a single word ("canvas", "7", "canvas.") is refused before anything else, then a
 * page link (path, query or fragment), then anything but https (http only for localhost). The
 * messages never repeat the input; at most the host.
 */
function canvasAddress(input: string): URL {
  const trimmed = input.trim();
  const invalid = () =>
    new ApiError(
      "invalid",
      "That is not a Canvas address. Enter just the address, like https://lms.example.edu",
    );
  if (trimmed.includes("~")) throw invalid();
  const withScheme = trimmed.includes("://") ? trimmed : `https://${trimmed}`;
  // The host as typed, before the URL parser turns "7" into 0.0.0.7: after "://", up to the
  // first / ? #, without user info, without a port (unless IPv6), without a trailing dot.
  let typed = withScheme.slice(withScheme.indexOf("://") + 3).split(/[/?#]/)[0] ?? "";
  typed = typed.split("@").pop() ?? "";
  if (!typed.startsWith("[")) typed = typed.split(":")[0] ?? "";
  typed = typed.replace(/\.+$/, "");
  if (!typed.includes(".") && !typed.startsWith("[") && typed !== "localhost") throw invalid();
  let url: URL;
  try {
    url = new URL(withScheme);
  } catch {
    throw invalid();
  }
  const host = url.hostname;
  if (!host || (!host.includes(".") && !host.includes(":") && host !== "localhost")) {
    throw invalid();
  }
  if (url.username || url.password) throw invalid();
  if ((url.pathname !== "/" && url.pathname !== "") || url.search || url.hash) {
    throw new ApiError(
      "invalid",
      `That is a link to a page, not a Canvas address. Enter just the address, like https://${url.host}`,
    );
  }
  const local = ["localhost", "127.0.0.1", "[::1]"].includes(host);
  if (!(url.protocol === "https:" || (url.protocol === "http:" && local))) throw invalid();
  return url;
}

export function createMockApi(options: MockOptions = {}): PageLampApi {
  const scenario = options.scenario ?? "demo";
  const latency = options.latencyMs ?? 250;
  const syncStep = options.syncStepMs ?? 350;
  const now = options.now ?? (() => new Date());
  const db: MockDb = buildCalendarScenarioDb(now(), scenario) ?? buildMockDb(now(), scenario);
  let syncing = false;
  const activity = createMockActivity(now);
  /** Set by cancelSync: the running sync stops at its next step. */
  let cancelRequested = false;
  let nextId = 1;

  // Updates (M0.4). A fresh install ("empty") hasn't seen the update-check disclosure yet; an
  // upgrader hasn't seen "What's new"; some scenarios have never checked. 0.1 never recorded its
  // version, so upgraders from it have none.
  const upgradedFrom =
    scenario === "upgrader"
      ? MOCK_PREVIOUS_VERSION
      : scenario === "upgrader-from-alpha1"
        ? "0.3.0-alpha.1"
        : null;
  const offersUpdate = scenario === "update-available" || scenario === "deb";
  const updates = {
    prefs: { auto_check: true, channel: null as UpdateChannel | null },
    disclosureSeen: scenario !== "empty",
    whatsNewSeen:
      scenario !== "upgrader" &&
      scenario !== "upgrader-from-01" &&
      scenario !== "upgrader-from-alpha1",
    lastCheck: (offersUpdate || scenario === "upgrader" || scenario === "upgrader-from-01"
      ? null
      : {
          at: new Date(now().getTime() - 2 * 60 * 60 * 1000).toISOString(),
          channel: "beta",
          outcome: { kind: "up_to_date" },
        }) as UpdateCheckRecord | null,
  };
  function effectiveChannel(): UpdateChannel {
    return updates.prefs.channel ?? (MOCK_APP_VERSION.includes("-") ? "beta" : "stable");
  }
  function mockUpdate(): AvailableUpdate {
    return {
      version: MOCK_UPDATE_VERSION,
      date: new Date(now().getTime() - DAY).toISOString(),
      notes:
        "- Every course shows its week and phase.\n- Finished courses move to a “Past” group.\n- Fixes for syncing large course folders.",
      download_url:
        scenario === "deb"
          ? `https://github.com/Euswbnix/pagelamp/releases/tag/v${MOCK_UPDATE_VERSION}`
          : null,
    };
  }

  async function respond<T>(value: T | (() => T), extraLatency = 0): Promise<T> {
    await sleep(latency + extraLatency);
    const result = typeof value === "function" ? (value as () => T)() : value;
    return clone(result);
  }

  function findCourse(courseId: string): MockCourse {
    const found = db.courses.find((c) => c.course.id === courseId);
    if (!found) throw new ApiError("not_found", `No course with id ${courseId}`);
    return found;
  }

  function findSource(sourceId: string): SourceRecord {
    const found = db.sources.find((s) => s.id === sourceId);
    if (!found) throw new ApiError("not_found", `No source with id ${sourceId}`);
    return found;
  }

  function sourceLabel(sourceId: string) {
    return db.sources.find((s) => s.id === sourceId)?.label ?? sourceId;
  }

  function sourceSyncedAt(sourceId: string) {
    return db.sources.find((s) => s.id === sourceId)?.last_synced_at ?? null;
  }

  function deadlinesWithin(courses: MockCourse[], daysAhead: number, daysBack: number) {
    const t = now().getTime();
    return courses
      .flatMap((c) => c.deadlines)
      .filter((d) => {
        const w = whenOf(d);
        return w !== null && w >= t - daysBack * DAY && w <= t + daysAhead * DAY;
      })
      .sort((a, b) => (whenOf(a) ?? 0) - (whenOf(b) ?? 0));
  }

  /** Every week with a module or material, plus the current week, ascending (like Rust). */
  function availableWeeks(c: MockCourse): number[] {
    const weeks = new Set<number>();
    for (const m of [...c.modules, ...c.materials]) if (m.week_hint) weeks.add(m.week_hint);
    const timeline = timelineOf(c);
    if (timeline.current_week) weeks.add(timeline.current_week);
    if (timeline.default_week) weeks.add(timeline.default_week);
    return [...weeks].sort((a, b) => a - b);
  }

  /** The lifecycle with "I'm still taking this" applied, as the facade computes it per read. */
  // Lifecycle, removal suggestions and removal (removal.ts). lifecycleOf applies "I'm still
  // taking this" and snoozes, as the facade computes them per read.
  const courseLifecycle = createLifecycleMock({ db, scenario, now, respond, findCourse });
  const lifecycleOf = courseLifecycle.lifecycleOf;

  /** Ended, Inactive and Upcoming courses leave the week-based views (the facade's D43 rule). */
  function inWeekViews(c: MockCourse): boolean {
    const state = lifecycleOf(c).state;
    return state !== "ended" && state !== "inactive" && state !== "upcoming";
  }
  /**
   * The timeline every view shows, as the facade computes it: a course that is over, inactive
   * or not started has no current week, default week or current modules.
   */
  function timelineOf(c: MockCourse): CourseTimeline {
    if (inWeekViews(c)) return c.timeline;
    return {
      ...c.timeline,
      current_week: null,
      default_week: null,
      current_module_ids: [],
      confidence: "low",
    };
  }
  /**
   * The courses a study plan covers (the facade's plan scope): without a list, every visible,
   * active course; with one, the named courses that aren't hidden (a course that has ended is
   * still planned; an unknown one isn't found). None: blocked with no_course_to_plan.
   */
  function planCourses(wanted: readonly string[]): MockCourse[] {
    if (wanted.length === 0) {
      return db.courses.filter((c) => !c.course.hidden && lifecycleOf(c).is_active);
    }
    const named = wanted.map(findCourse).filter((c) => !c.course.hidden);
    return named.filter((c, i) => named.findIndex((o) => o.course.id === c.course.id) === i);
  }

  /**
   * The weekly note has nothing to write about (the facade's writable_note_context): no visible
   * active course, no deadline in the next 7 days, and no plan item of the last 7 days or today
   * (a hidden or removed course's left out; one of a course the mock doesn't know counts).
   */
  function noteHasNothingToWrite(): boolean {
    const visible = db.courses.filter((c) => !c.course.hidden);
    const today = isoOf(now());
    const weekAgo = addDays(today, -7);
    const leftOut = new Set([
      ...db.courses.filter((c) => c.course.hidden).map((c) => c.course.id),
      ...courseLifecycle.removedIds(),
    ]);
    const planItems = (db.studyPlan?.plan.items ?? []).filter(
      (item) =>
        item.date >= weekAgo &&
        item.date <= today &&
        !(item.course_id != null && leftOut.has(item.course_id)),
    );
    return (
      !visible.some((c) => lifecycleOf(c).is_active) &&
      deadlinesWithin(visible, 7, 0).length === 0 &&
      planItems.length === 0
    );
  }
  /**
   * The AI gate of one run, as in the facade: the AI mock's estimate (its blocks; the student may
   * override a reached budget), then who the feature's model runs on. `ai` is created below;
   * this is only called later.
   */
  async function aiGate(request: EstimateRequest, overrideBudget: boolean): Promise<MockAiRun> {
    const estimate = await ai.estimateGeneration(request);
    const block = estimate.would_block ?? null;
    if (block && !(block === "budget_reached" && overrideBudget)) {
      throw new ApiError("blocked", "The AI gate stopped this run.", { blocked: block });
    }
    const status = await ai.aiStatus();
    const choice = status.features.find((f) => f.feature === request.feature)?.choice;
    const backend = choice
      ? status.backends.find((b) => sameBackend(b.backend, choice.backend))
      : undefined;
    if (!choice || !backend) {
      throw new ApiError("blocked", "No model chosen.", { blocked: "no_model_chosen" });
    }
    return {
      backend_label: backend.label,
      model: choice.model,
      on_device: backend.kind === "local",
    };
  }

  // Reminders, their settings, the tray and the login item (reminders.ts).
  const { dueNow: dueReminders, ...remindersApi } = createRemindersMock({
    scenario,
    now,
    respond,
    courses: () => db.courses,
  });

  // Study plans written by PageLamp (plan.ts).
  const studyPlans = createPlanMock({
    db,
    now,
    respond,
    step: () => sleep(syncStep),
    activity,
    gate: (courses, horizonDays, overrideBudget) =>
      aiGate({ feature: "study_plan", courses, horizon_days: horizonDays }, overrideBudget),
    planCourses,
  });

  // Weekly explanations (explain.ts).
  const explanations = createExplainMock({
    now,
    respond,
    step: () => sleep(syncStep),
    activity,
    gate: (courseId, week, include, overrideBudget) =>
      aiGate({ feature: "weekly_explanation", course: courseId, week, include }, overrideBudget),
    findCourse,
  });

  // The AI weekly note and "Prepare it on Monday" (weeklyNote.ts).
  const weeklyNotes = createWeeklyNoteMock({
    scenario,
    now,
    respond,
    step: () => sleep(syncStep),
    activity,
    courses: () => db.courses,
    lifecycleOf: (c) => lifecycleOf(c),
    deadlinesWithin,
    nothingToWrite: noteHasNothingToWrite,
    gate: (overrideBudget) => aiGate({ feature: "weekly_note" }, overrideBudget),
    noteBackend: async () => {
      const status = await ai.aiStatus();
      const choice = status.features.find((f) => f.feature === "weekly_note")?.choice;
      if (!choice) return null;
      return status.backends.find((b) => sameBackend(b.backend, choice.backend))?.kind ?? null;
    },
  });

  // Calendar proposals, candidates and syllabus reading (proposals.ts).
  const courseProposals = createProposalsMock({
    db,
    scenario,
    now,
    activity,
    respond,
    findCourse,
    step: () => sleep(syncStep),
    // The AI mock's estimate is the gate, as in the facade (created below; called later).
    aiGate: (courseId, overrideBudget) =>
      aiGate({ feature: "course_calendar", courses: [courseId] }, overrideBudget),
    applyCalendar: (c, input, origin, aiLabel) => {
      const next = withCourseDates(c.timeline, input, isoOf(now()));
      c.timeline = {
        ...next.timeline,
        term: { ...next.timeline.term, anchor_origin: origin, ai_label: aiLabel },
      };
      c.lifecycle = next.lifecycle;
      c.course.term_start = input.first_class ?? c.course.term_start;
      c.course.term_source = "user";
    },
  });

  function summary(c: MockCourse): CourseSummary {
    const upcoming = deadlinesWithin([c], 21, 0).filter((d) => d.kind !== "class_event");
    const aiMaterials = aiMaterialsState(c.course);
    return {
      course: c.course,
      timeline: timelineOf(c),
      lifecycle: lifecycleOf(c),
      ai_materials: aiMaterials,
      counts: {
        modules: c.modules.length,
        materials: c.materials.length,
        // Like the facade: "readable by your AI app" is 0 unless the AI may read materials.
        indexed_materials: aiMaterials === "readable" ? c.materials.filter(hasText).length : 0,
        upcoming_deadlines: upcoming.length,
      },
      next_deadline: upcoming[0] ?? null,
      source_label: sourceLabel(c.course.source_id),
      last_synced_at: sourceSyncedAt(c.course.source_id),
    };
  }

  function status(): AppStatus {
    const visible = db.courses.filter((c) => !c.course.hidden);
    const materials = db.courses.flatMap((c) => c.materials);
    const indexed = materials.filter(hasText);
    const synced = db.sources
      .map((s) => s.last_synced_at)
      .filter((v): v is string => !!v)
      .sort();
    return {
      version: MOCK_APP_VERSION,
      data_dir: db.dataDir,
      db_path: `${db.dataDir}/pagelamp.db`,
      sources: db.sources,
      counts: {
        courses: visible.length,
        hidden_courses: db.courses.length - visible.length,
        modules: db.courses.reduce((n, c) => n + c.modules.length, 0),
        materials: materials.length,
        indexed_materials: indexed.length,
        chunks: indexed.reduce((n, m) => n + m.chunk_count, 0),
        events: db.courses.reduce((n, c) => n + c.deadlines.length, 0),
        study_plans: db.studyPlan ? 1 : 0,
        removed_courses: courseLifecycle.removedCount(),
      },
      last_synced_at: synced.at(-1) ?? null,
      sync_in_progress: syncing || db.externalSyncRunning,
    };
  }

  function addSource(kind: SourceKind, id: string, label: string, config: Record<string, unknown>) {
    if (db.sources.some((s) => s.id === id)) {
      throw new ApiError("invalid", `${label} is already added`);
    }
    const record: SourceRecord = {
      id,
      kind,
      label,
      config,
      last_synced_at: null,
      last_error: null,
      last_error_kind: null,
    };
    db.sources.push(record);
    return record;
  }

  // Shared by add_canvas_source and update_source_secret: what a real Canvas would say.
  function validateCanvasToken(token: string) {
    const t = token.trim();
    if (t.length < 8)
      throw new ApiError("invalid", "That doesn't look like a Canvas access token.");
    if (/expired|revoked|bad/i.test(t)) {
      throw new ApiError(
        "auth",
        "Canvas rejected this token. It may have expired or been revoked.",
      );
    }
  }

  function validateFeedUrl(feedUrl: string) {
    const url = parseHttpUrl(feedUrl, true);
    if (!url) throw new ApiError("invalid", "Paste the full calendar feed address (https://…).");
    if (url.pathname.includes("404")) {
      throw new ApiError("not_found", "The calendar feed address returned 404 Not Found.");
    }
    if (url.hostname.includes("offline")) {
      throw new ApiError("network", `Couldn't reach ${url.hostname}.`);
    }
  }

  // First-run demo: in the "empty" scenario the first folder/Canvas source to sync "finds" the
  // demo courses, so onboarding → courses can be walked through end to end.
  function seedCoursesOnFirstSync(source: SourceRecord) {
    if (scenario !== "empty" || source.kind === "ical" || db.courses.length > 0) return;
    db.courses = buildMockDb(now(), "demo").courses.map((c) => ({
      ...c,
      course: { ...c.course, source_id: source.id },
    }));
  }

  /** A sync or a download, listed in activity() while it runs. */
  function runSync(
    sourceIds: string[],
    onEvent: (event: SyncEvent) => void,
    work: { kind: "sync" | "download"; source_id?: string } = { kind: "sync" },
  ): Promise<SourceSyncResult[]> {
    return activity.during(work.kind, { source_id: work.source_id }, () =>
      syncSources(sourceIds, onEvent),
    );
  }

  async function syncSources(
    sourceIds: string[],
    onEvent: (event: SyncEvent) => void,
  ): Promise<SourceSyncResult[]> {
    if (syncing || db.externalSyncRunning) {
      await sleep(latency);
      throw new ApiError("busy", "Another PageLamp process is already syncing.");
    }
    syncing = true;
    cancelRequested = false;
    const results: SourceSyncResult[] = [];
    try {
      for (const sourceId of sourceIds) {
        const source = findSource(sourceId);
        const startedAt = now().toISOString();
        onEvent({ type: "source_started", source_id: source.id, label: source.label });
        seedCoursesOnFirstSync(source);
        const courses = db.courses.filter((c) => c.course.source_id === source.id);
        const total = Math.max(courses.length, 1) * 3;
        const warnings: string[] = [];
        for (let step = 1; step <= total; step++) {
          await sleep(syncStep);
          if (cancelRequested) {
            // Like the facade: the source isn't recorded as failed.
            const stopped = "The sync was stopped.";
            onEvent({
              type: "source_finished",
              source_id: source.id,
              ok: false,
              error: stopped,
              error_kind: null,
            });
            throw new ApiError("cancelled", stopped);
          }
          onEvent({
            type: "progress",
            source_id: source.id,
            ...stepOf(source, courses[Math.floor((step - 1) / 3)]?.course.code),
            current: step,
            total,
          });
          if (source.kind === "folder" && step === 2) {
            const w = "Skipped 'Week 3 lecture recording.mp4' — video files can't be read.";
            warnings.push(w);
            onEvent({ type: "warning", source_id: source.id, message: w });
          }
        }

        // Failure scenarios stay failed until fixed (token replaced, folder re-added).
        const failure =
          source.last_error_kind === "auth_expired_or_revoked" ||
          source.last_error_kind === "not_found"
            ? { error: source.last_error ?? "Sync failed", kind: source.last_error_kind }
            : null;
        if (failure) {
          onEvent({
            type: "source_finished",
            source_id: source.id,
            ok: false,
            error: failure.error,
            error_kind: failure.kind,
          });
        } else {
          source.last_synced_at = now().toISOString();
          source.last_error = null;
          source.last_error_kind = null;
          onEvent({ type: "source_finished", source_id: source.id, ok: true });
        }
        results.push({
          source_id: source.id,
          label: source.label,
          kind: source.kind,
          ok: !failure,
          error: failure?.error ?? null,
          error_kind: failure?.kind ?? null,
          started_at: startedAt,
          finished_at: now().toISOString(),
          courses: courses.length,
          modules: courses.reduce((n, c) => n + c.modules.length, 0),
          materials: courses.reduce((n, c) => n + c.materials.length, 0),
          files_downloaded: failure ? 0 : 2,
          files_indexed: failure ? 0 : 2,
          events: courses.reduce((n, c) => n + c.deadlines.length, 0),
          warnings,
          // Per-course details come from Canvas only.
          course_summaries:
            source.kind === "canvas" && !failure
              ? courses.map((c) => ({
                  course: c.course.code ?? c.course.name,
                  modules: c.modules.length,
                  pages: c.materials.filter((m) => m.kind === "page").length,
                  files: c.materials.filter((m) => m.kind === "file").length,
                  events: c.deadlines.length,
                  warnings: 0,
                }))
              : [],
        });
      }
    } finally {
      syncing = false;
    }
    return results;
  }

  const ai = createMockAi({
    scenario,
    now,
    activity,
    // The extra delay imitates slow network calls in the demo; tests (latency 0) skip it, so a
    // loaded machine can't push them past their timeouts.
    delay: (extra = 0) => sleep(latency > 0 ? latency + extra : 0),
    stepMs: syncStep,
    courses: () => db.courses,
    findCourse,
    noteHasNothingToWrite,
    planCourses,
  });

  /** A material with a local file on this computer (and, to open it, a document type). */
  function localDocument(materialId: string, open: boolean): boolean {
    const m = db.courses.flatMap((c) => c.materials).find((x) => x.id === materialId);
    if (!m || (m.kind !== "file" && m.kind !== "syllabus")) return false;
    if (m.text_status === "not_downloaded") return false;
    return !open || m.text_status !== "unsupported";
  }

  /** The student's dates cleared: back to what the source reported (like the facade). */
  function clearStudentDates(c: MockCourse) {
    c.course.term_start = c.synced.termStart;
    c.course.term_end = c.synced.termEnd;
    c.course.term_source = c.synced.termStart || c.synced.termEnd ? "synced" : "none";
    const synced = withoutStudentDates(c.synced.timeline, c.synced.lifecycle);
    c.timeline = synced.timeline;
    c.lifecycle = synced.lifecycle;
  }

  return {
    ...ai,
    ...courseLifecycle.api,
    ...courseProposals.api,
    // One Stop for every run: a syllabus reading, a study plan, an explanation or a note.
    cancelGeneration: async (generationId) => {
      studyPlans.cancel(generationId);
      explanations.cancel(generationId);
      weeklyNotes.cancel(generationId);
      await courseProposals.api.cancelGeneration(generationId);
    },
    writeWeeklyNote: weeklyNotes.writeWeeklyNote,
    weeklyNotes: weeklyNotes.weeklyNotes,
    deleteWeeklyNote: weeklyNotes.deleteWeeklyNote,
    weeklyNoteSettings: weeklyNotes.weeklyNoteSettings,
    setPrepareWeeklyNoteOnMonday: weeklyNotes.setPrepareWeeklyNoteOnMonday,
    explainWeek: explanations.explainWeek,
    savedExplanations: explanations.savedExplanations,
    deleteExplanation: explanations.deleteExplanation,
    aiOutputLanguage: explanations.aiOutputLanguage,
    setAiOutputLanguage: explanations.setAiOutputLanguage,
    generateStudyPlan: studyPlans.generateStudyPlan,
    planLimits: studyPlans.planLimits,
    acceptStudyPlan: studyPlans.acceptStudyPlan,
    setStudyPlanItemDone: studyPlans.setStudyPlanItemDone,
    ...remindersApi,

    downloadMaterialFiles: async (courseId, materialIds, onEvent) => {
      const c = findCourse(courseId);
      const source = findSource(c.course.source_id);
      if (source.kind !== "canvas") {
        await sleep(latency);
        throw new ApiError("invalid", "Only Canvas courses have files to download.");
      }
      const [result] = await runSync([source.id], onEvent, {
        kind: "download",
        source_id: source.id,
      });
      if (!result) throw new ApiError("internal", "Sync produced no result");
      let downloaded = 0;
      for (const m of c.materials) {
        if (!materialIds.includes(m.id) || m.text_status !== "not_downloaded") continue;
        m.text_status = "ok";
        m.chunk_count = 6;
        downloaded += 1;
      }
      return clone({ ...result, files_downloaded: downloaded, files_indexed: downloaded });
    },
    status: () => respond(status),
    activity: () =>
      respond(() => ({
        items: activity.items(),
        other_process_syncing: db.externalSyncRunning && !syncing,
      })),
    listSources: () => respond(() => db.sources),

    addCanvasSource: async (baseUrl, token) => {
      await sleep(latency + 500);
      const url = canvasAddress(baseUrl);
      if (url.hostname.includes("offline")) {
        throw new ApiError("network", `Couldn't reach ${url.hostname}.`);
      }
      validateCanvasToken(token);
      return clone(
        addSource("canvas", `canvas:${url.hostname}`, url.hostname, {
          base_url: url.origin,
          account_name: "Demo Student",
        }),
      );
    },

    addFolderSource: async (path, termStart, label) => {
      await sleep(latency);
      const p = path.trim();
      if (!p) throw new ApiError("invalid", "Choose a folder first.");
      if (p.includes("missing")) throw new ApiError("not_found", `Folder ${p} was not found.`);
      if (termStart && !/^\d{4}-\d{2}-\d{2}$/.test(termStart)) {
        throw new ApiError("invalid", "Term start must be a date like 2026-09-08.");
      }
      const name = label?.trim() || p.split("/").filter(Boolean).at(-1) || p;
      return clone(
        addSource("folder", `folder:mock-${nextId++}`, name, {
          path: p,
          ...(termStart ? { term_start: termStart } : {}),
        }),
      );
    },

    addIcalSource: async (feedUrl, label) => {
      await sleep(latency + 500);
      validateFeedUrl(feedUrl);
      return clone(
        addSource("ical", `ical:mock-${nextId++}`, label?.trim() || "Calendar feed", {}),
      );
    },

    updateSourceSecret: async (sourceId, secret) => {
      await sleep(latency + 500);
      const source = findSource(sourceId);
      if (source.kind === "canvas") validateCanvasToken(secret);
      else if (source.kind === "ical") validateFeedUrl(secret);
      else throw new ApiError("invalid", "Folder sources have no secret.");
      source.last_error = null;
      source.last_error_kind = null;
      return clone(source);
    },

    removeSource: async (sourceId) => {
      await sleep(latency);
      findSource(sourceId);
      db.sources = db.sources.filter((s) => s.id !== sourceId);
      db.courses = db.courses.filter((c) => c.course.source_id !== sourceId);
    },

    cancelSync: async () => {
      if (syncing) cancelRequested = true;
    },

    syncAll: async (_req: SyncRequest, onEvent) => {
      const startedAt = now().toISOString();
      const results = await runSync(
        db.sources.map((s) => s.id),
        onEvent,
      );
      const out: SyncSummary = {
        started_at: startedAt,
        finished_at: now().toISOString(),
        ok: results.every((r) => r.ok),
        results,
      };
      return clone(out);
    },

    syncSource: async (sourceId, _req, onEvent) => {
      findSource(sourceId);
      const [result] = await runSync([sourceId], onEvent, { kind: "sync", source_id: sourceId });
      if (!result) throw new ApiError("internal", "Sync produced no result");
      return clone(result);
    },

    downloadCourseFiles: async (courseId, onEvent) => {
      const c = findCourse(courseId);
      const source = findSource(c.course.source_id);
      if (source.kind !== "canvas") {
        await sleep(latency);
        throw new ApiError(
          "invalid",
          "Only Canvas courses have files to download; folder courses are always indexed.",
        );
      }
      const [result] = await runSync([source.id], onEvent, {
        kind: "download",
        source_id: source.id,
      });
      if (!result) throw new ApiError("internal", "Sync produced no result");
      let downloaded = 0;
      const warnings = [...result.warnings];
      if (result.ok) {
        for (const m of c.materials) {
          if (m.kind !== "file" || m.text_status !== "not_downloaded") continue;
          if (m.download_blocked) {
            // Like the backend: skipped, and only explained in the run's warnings.
            warnings.push(
              m.download_blocked === "locked"
                ? `${c.course.code}: ${m.title} skipped (locked in Canvas)`
                : `${c.course.code}: ${m.title} skipped (larger than the download limit)`,
            );
            continue;
          }
          m.text_status = "ok";
          m.chunk_count = 6;
          downloaded += 1;
        }
      }
      return clone({
        ...result,
        files_downloaded: downloaded,
        files_indexed: downloaded,
        warnings,
      });
    },

    listCourses: () => respond(() => db.courses.map(summary)),

    courseOverview: (courseId) =>
      respond(() => {
        const c = findCourse(courseId);
        const t = now().getTime();
        const recent = (m: { published_at?: string | null }) =>
          !!m.published_at && Date.parse(m.published_at) >= t - 14 * DAY;
        const timeline = timelineOf(c);
        return {
          course: c.course,
          timeline,
          lifecycle: lifecycleOf(c),
          current_modules: c.modules.filter((m) => timeline.current_module_ids.includes(m.id)),
          recent_materials: c.materials
            .filter(recent)
            .sort((a, b) => Date.parse(b.published_at ?? "") - Date.parse(a.published_at ?? "")),
          upcoming_deadlines: deadlinesWithin([c], 21, 0),
          recent_announcements: c.announcements.filter(recent),
          source_label: sourceLabel(c.course.source_id),
          last_synced_at: sourceSyncedAt(c.course.source_id),
          ai_materials: aiMaterialsState(c.course),
          // Like the backend: what a course-wide download would fetch (all weeks).
          downloadable_files: c.materials.filter(
            (m) => m.kind === "file" && m.text_status === "not_downloaded" && !m.download_blocked,
          ).length,
        };
      }),

    weekMaterials: (courseId, week) =>
      respond(() => {
        const c = findCourse(courseId);
        const timeline = timelineOf(c);
        const shown = week ?? timeline.default_week ?? timeline.current_week ?? null;
        if (shown === null) {
          const t = now().getTime();
          return {
            course: c.course,
            week: null,
            requested_week: week ?? null,
            ai_materials: aiMaterialsState(c.course),
            available_weeks: availableWeeks(c),
            timeline,
            modules: [],
            materials: c.materials.filter(
              (m) => !!m.published_at && Date.parse(m.published_at) >= t - 14 * DAY,
            ),
            // A course outside the week views has no current week to be unknown.
            ...(inWeekViews(c)
              ? {
                  note: "Current week unknown — showing materials of the last 14 days.",
                  note_kind: "current_week_unknown" as const,
                }
              : {
                  note: "The course is over, inactive or hasn't started, so it has no current week; showing materials published in the last 14 days.",
                  note_kind: "outside_term" as const,
                }),
          };
        }
        return {
          course: c.course,
          week: shown,
          requested_week: week ?? null,
          ai_materials: aiMaterialsState(c.course),
          available_weeks: availableWeeks(c),
          timeline,
          modules: c.modules.filter((m) => m.week_hint === shown),
          materials: c.materials.filter((m) => m.week_hint === shown),
          ...(c.materials.some((m) => m.week_hint === shown)
            ? { note: null, note_kind: null }
            : {
                note: `No modules or materials for week ${shown}.`,
                note_kind: "no_materials_this_week" as const,
              }),
        };
      }),

    listDeadlines: (courseId, daysAhead, daysBack) =>
      respond(() => {
        const courses = courseId
          ? [findCourse(courseId)]
          : db.courses.filter((c) => !c.course.hidden);
        return deadlinesWithin(courses, daysAhead, daysBack);
      }),

    search: (query, courseId, limit) =>
      respond(() => {
        const q = query.trim().toLowerCase();
        if (!q) return [];
        const courses = courseId ? [findCourse(courseId)] : db.courses;
        return courses
          .flatMap((c) =>
            c.materials
              .filter((m) => m.text_status === "ok" && m.title.toLowerCase().includes(q))
              .map((m, i) => ({
                material_id: m.id,
                material_title: m.title,
                course_id: c.course.id,
                course_code: c.course.code ?? null,
                chunk_ord: 0,
                locator: "p. 1",
                snippet: `…«${query.trim()}» appears in ${m.title}…`,
                url: m.url ?? null,
                week_hint: m.week_hint ?? null,
                score: -1 - i,
              })),
          )
          .slice(0, limit);
      }),

    latestStudyPlan: () => respond(() => db.studyPlan),

    setCoursePolicy: async (courseId, policy, note) => {
      await sleep(latency);
      const c = findCourse(courseId);
      c.course.ai_policy = policy;
      c.course.ai_policy_note = note?.trim() ? note.trim() : null;
    },

    setCourseTerm: async (courseId, start, end) => {
      await sleep(latency);
      const isDate = (v: string | null) => v === null || /^\d{4}-\d{2}-\d{2}$/.test(v);
      if (!isDate(start) || !isDate(end))
        throw new ApiError("invalid", "Use dates like 2026-09-08.");
      if (start && end && end < start) {
        throw new ApiError("invalid", "The term can't end before it starts.");
      }
      const c = findCourse(courseId);
      const today = isoOf(now());
      if (start === null && end === null) {
        clearStudentDates(c);
        return;
      }
      // COALESCE(user, synced), per field; the student's end is the last day of classes.
      c.course.term_start = start ?? c.synced.termStart;
      c.course.term_end = end ?? c.synced.termEnd;
      c.course.term_source = "user";
      const next = withStudentDates(c.timeline, start, end, today);
      c.timeline = next.timeline;
      c.lifecycle = next.lifecycle;
    },

    setCourseHidden: async (courseId, hidden) => {
      await sleep(latency);
      findCourse(courseId).course.hidden = hidden;
    },

    keepCourseCurrent: async (courseId, until) => {
      await sleep(latency);
      const c = findCourse(courseId);
      const today = isoOf(now());
      if (until !== null && (!/^\d{4}-\d{2}-\d{2}$/.test(until) || until < today)) {
        throw new ApiError("invalid", "Pick a date from today on.");
      }
      c.keptCurrentUntil = until ?? defaultKeepUntil(c.timeline, today);
    },

    clearKeepCourseCurrent: async (courseId) => {
      await sleep(latency);
      findCourse(courseId).keptCurrentUntil = null;
    },

    confirmCourseDates: async (courseId) => {
      await sleep(latency);
      const c = findCourse(courseId);
      if (c.timeline.term.anchor_origin === "legacy") {
        c.timeline = { ...c.timeline, term: { ...c.timeline.term, anchor_origin: "user" } };
      }
    },

    setCourseDates: async (courseId, dates) => {
      await sleep(latency);
      const c = findCourse(courseId);
      if (dates === null) {
        clearStudentDates(c);
        return courseProposals.api.courseCalendar(courseId);
      }
      const inOrder = (...dates: (string | null | undefined)[]) => {
        const set = dates.filter((d): d is string => !!d);
        return set.every((d, i) => i === 0 || (set[i - 1] ?? d) <= d);
      };
      const segment = dates.second_segment;
      if (
        !inOrder(
          dates.first_class,
          dates.last_class,
          segment?.first_class,
          segment?.last_class,
          dates.exams_end,
        ) ||
        dates.breaks.some((b) => b.end < b.start)
      ) {
        throw new ApiError("invalid", "These dates are out of order.");
      }
      const next = withCourseDates(c.timeline, dates, isoOf(now()));
      c.course.term_start = dates.first_class ?? c.synced.termStart;
      c.course.term_end =
        dates.exams_end ?? segment?.last_class ?? dates.last_class ?? c.synced.termEnd;
      c.course.term_source = "user";
      c.timeline = next.timeline;
      c.lifecycle = next.lifecycle;
      return courseProposals.api.courseCalendar(courseId);
    },

    setCourseAiAccess: async (courseId, allowed) => {
      await sleep(latency);
      findCourse(courseId).course.ai_access = allowed;
    },

    mcpClientConfigs: () => respond(() => mcpClientConfigs(MOCK_BINARY_PATH)),

    updatePrefs: () => respond(() => ({ ...updates.prefs })),
    effectiveUpdateChannel: () => respond(effectiveChannel),
    setUpdatePrefs: (prefs) =>
      respond(() => {
        updates.prefs = { auto_check: prefs.auto_check, channel: prefs.channel ?? null };
      }),
    startupTasks: async () => {
      // Monday's note (weeklyNote.ts): asks who runs the note's model, so it's read first.
      const prepareWeeklyNote = await weeklyNotes.prepareNow();
      return respond(() => {
        const last = updates.lastCheck ? Date.parse(updates.lastCheck.at) : null;
        return {
          prepare_weekly_note: prepareWeeklyNote,
          whats_new: updates.whatsNewSeen ? null : whatsNewSince(upgradedFrom),
          update_check_due:
            updates.prefs.auto_check &&
            updates.disclosureSeen &&
            updates.whatsNewSeen &&
            (last === null || last <= now().getTime() - DAY),
          // The facade's launch.updated_from, which also gives What's new its `since`.
          updated_from: upgradedFrom ?? (scenario === "updated" ? MOCK_PREVIOUS_VERSION : null),
          // What came due since the last launch (reminders-due); the purge: none in the mock yet.
          due_reminders: dueReminders(),
          purge_due: false,
          removed_files_waiting: 0,
          // The same offers as the Courses page's card, "Not now" applied (proposals.ts).
          calendar_offers: courseProposals.offersNow().slice(0, STARTUP_LIST_MAX),
          calendar_offers_total: courseProposals.offersNow().length,
          removal_suggestions: [],
          removal_suggestions_total: 0,
        };
      });
    },
    acknowledgeWhatsNew: () =>
      respond(() => {
        updates.whatsNewSeen = true;
        updates.disclosureSeen = true;
      }),
    acknowledgeUpdateDisclosure: () =>
      respond(() => {
        updates.disclosureSeen = true;
      }),
    lastUpdateCheck: () => respond(() => updates.lastCheck),

    diagnosticReport: () => respond(() => diagnosticReport(status(), db.lastCrash, now())),
    doctor: () =>
      respond(() => {
        const s = status();
        const blocked = scenario === "worker-blocked";
        return {
          version: s.version,
          os: "macos",
          arch: "aarch64",
          data_dir: "~/Library/Application Support/dev.PageLamp.PageLamp",
          logs_dir: "~/Library/Application Support/dev.PageLamp.PageLamp/logs",
          schema_version: 3,
          database_error: null,
          keychain_available: true,
          keychain_error: null,
          sources: s.sources.map((source) => ({
            kind: source.kind,
            ok: !source.last_error_kind,
            last_synced_at: source.last_synced_at ?? null,
            last_error_kind: source.last_error_kind ?? null,
          })),
          courses: s.counts.courses,
          hidden_courses: s.counts.hidden_courses,
          materials: s.counts.materials,
          events: s.counts.events,
          mcp_clients: { claude_desktop: true, claude_code: false, codex: false },
          last_crash: db.lastCrash,
          extract_worker: blocked
            ? { status: "spawn_failed" as const, spawn_ms: null }
            : { status: "ok" as const, spawn_ms: 24 },
          unreadable_files: blocked
            ? [
                { kind: "spawn_failed" as const, count: 3 },
                { kind: "timed_out" as const, count: 1 },
              ]
            : [],
          enrollment_window_terms: 0,
          removals_waiting: 0,
        };
      }),
    lastCrash: () => respond(() => db.lastCrash),
    clearLastCrash: () =>
      respond(() => {
        db.lastCrash = null;
      }),

    pickFolder: () => respond("/Users/demo/Documents/Courses"),
    openExternal: async () => {
      // Mock mode never leaves the page: demo links point at *.demo.test.
    },
    revealDataDir: async () => {},
    // Like material_local_file: a downloaded document of a file material (never a page or link).
    openMaterial: async (materialId) => {
      await sleep(latency);
      return localDocument(materialId, true);
    },
    revealMaterial: async (materialId) => {
      await sleep(latency);
      return localDocument(materialId, false);
    },
    onWindowFocus: (onFocus) => {
      // The browser tab's focus stands in for the desktop window's.
      const handler = () => onFocus();
      window.addEventListener("focus", handler);
      return () => window.removeEventListener("focus", handler);
    },
    revealLogsDir: async () => {},
    updaterStatus: () =>
      respond(() => ({
        current_version: MOCK_APP_VERSION,
        install: scenario === "deb" ? ("download_only" as const) : ("in_app" as const),
        platform: scenario === "deb" ? ("linux" as const) : ("macos" as const),
      })),
    checkForUpdate: async () => {
      await sleep(latency + 300);
      const at = now().toISOString();
      const channel = effectiveChannel();
      // Stable has no release yet: every release so far is a test version, like this mock's.
      if (channel === "stable") {
        updates.lastCheck = { at, channel, outcome: { kind: "error", code: "manifest" } };
        throw new ApiError("not_found", "Could not fetch a valid release JSON from the remote");
      }
      if (offersUpdate) {
        updates.lastCheck = {
          at,
          channel,
          outcome: { kind: "available", version: MOCK_UPDATE_VERSION },
        };
        return clone(mockUpdate());
      }
      updates.lastCheck = { at, channel, outcome: { kind: "up_to_date" } };
      return null;
    },
    installUpdate: async (onEvent) => {
      await sleep(latency);
      if (scenario === "deb") {
        throw new ApiError("invalid", "This install updates by downloading the new package.");
      }
      if (!offersUpdate) throw new ApiError("not_found", "No update to install.");
      const total = 14_500_000;
      onEvent({ type: "download_started", total_bytes: total });
      for (const part of [0.25, 0.5, 0.75, 1]) {
        await sleep(syncStep);
        onEvent({
          type: "progress",
          downloaded_bytes: Math.round(total * part),
          total_bytes: total,
        });
      }
      onEvent({ type: "installing" });
      await sleep(syncStep);
      // The real app restarts here; mock mode stays on the "Restarting…" state.
      onEvent({ type: "restarting" });
    },
    logUiError: async () => {},
  };
}

/** A step like the facade's: its stage, its course, and the English text for the CLI and logs. */
function stepOf(
  source: SourceRecord,
  courseCode: string | null | undefined,
): Pick<Extract<SyncEvent, { type: "progress" }>, "message" | "stage" | "course"> {
  const course = courseCode ?? source.label;
  switch (source.kind) {
    case "folder":
      return { stage: "indexing_files", course, message: `${course}: indexing files` };
    case "canvas":
      return { stage: "reading_course", course, message: `${course}: reading` };
    case "ical":
      return { stage: "downloading_feed", course: null, message: "Downloading the calendar feed" };
  }
}

/** Text to read, like the facade's count: indexed ("ok") and at least one chunk (not a scan). */
function hasText(m: MaterialView): boolean {
  return m.text_status === "ok" && m.chunk_count > 0;
}
