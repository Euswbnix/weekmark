// Synthetic demo data for mock mode and tests. EVERYTHING here is made up (docs/ARCHITECTURE.md
// §3.7): no real courses, people, schools or tokens. Dates are relative to `now` so the demo
// always looks "live" (a deadline in 2 days, week 4 of term, …).

import type { CourseWithSharing, MaterialSharing } from "../ai";
import type {
  AiPolicy,
  AppStatus,
  Confidence,
  Course,
  CourseLifecycle,
  CourseTimeline,
  CrashReport,
  Deadline,
  DownloadBlock,
  EventKind,
  MaterialKind,
  MaterialView,
  McpClientConfig,
  Module,
  SourceErrorKind,
  SourceRecord,
  StoredStudyPlan,
  TextProblem,
  TextStatus,
} from "../types";
import {
  type CalendarFields,
  calendarFields,
  dayFrom,
  ev,
  lifecycle,
  mondayOf,
  resolution,
} from "./calendar";

export type MockScenario =
  | "demo"
  | "empty"
  | "expired"
  | "error"
  | "busy"
  | "crashed"
  // Updates (M0.4): an update is offered / an upgrader from 0.1 sees "What's new" / the first
  // launch after an update / a deb or rpm install (download link only).
  | "update-available"
  | "upgrader"
  | "upgrader-from-01"
  | "updated"
  | "deb"
  // The file reader (extraction worker) is blocked, e.g. by antivirus (M0.5).
  | "worker-blocked"
  // AI setup (M1): an API key at 40% of the budget / Ollama on this computer / a model with no
  // price / 99% of the budget / a disclosure that changed since it was acknowledged / a custom
  // endpoint whose model list and test fail. The demo has no model set up.
  | "ai-key"
  | "ai-local"
  | "ai-unpriced"
  | "ai-budget"
  | "ai-disclosure-changed"
  | "ai-errors"
  // The ChatGPT plan through Codex (M2): installed and signed out / Plus (12 of 40 runs this week)
  // / an Edu workspace / signed in with an API key / an installed Codex older than the pin /
  // RuntimeOutdated at the pin (update PageLamp) / Free without `codex exec` / the weekly cap
  // reached. The demo has no Codex installed.
  | "codex-signed-out"
  | "codex-plus"
  | "codex-edu"
  | "codex-api-key"
  | "codex-outdated-pin"
  | "codex-outdated-app"
  | "codex-free"
  | "codex-cap"
  // Course weeks and lifecycle (M0.10): the owner's UofT-style case / one course per phase /
  // only past courses.
  | "uoft-fall"
  | "phases"
  | "all-past"
  // Removal (F2): some courses already removed, one waiting to be purged.
  | "removed"
  // Calendar proposals (F3): an AI proposal with a conflict, a scan proposal, a stale calendar.
  | "proposals";

export const MOCK_SCENARIOS: readonly MockScenario[] = [
  "demo",
  "empty",
  "expired",
  "error",
  "busy",
  "crashed",
  "update-available",
  "upgrader",
  "upgrader-from-01",
  "updated",
  "deb",
  "worker-blocked",
  "ai-key",
  "ai-local",
  "ai-unpriced",
  "ai-budget",
  "ai-disclosure-changed",
  "ai-errors",
  "codex-signed-out",
  "codex-plus",
  "codex-edu",
  "codex-api-key",
  "codex-outdated-pin",
  "codex-outdated-app",
  "codex-free",
  "codex-cap",
  "uoft-fall",
  "phases",
  "all-past",
  "removed",
  "proposals",
];

/** The version mock mode reports (a pre-release, so its default update channel is beta). */
export const MOCK_APP_VERSION = "0.3.0-alpha.1";
/** The earlier version the "upgrader" and "updated" scenarios come from (0.1 recorded none). */
export const MOCK_PREVIOUS_VERSION = "0.3.0-alpha.0";
/** The version mock mode offers as an update. */
export const MOCK_UPDATE_VERSION = "0.3.0-alpha.2";

/** One course with everything the views need. */
export interface MockCourse {
  course: Course;
  timeline: CourseTimeline;
  /** The lifecycle without "I'm still taking this" (the mock applies that at read time). */
  lifecycle: CourseLifecycle;
  /** "I'm still taking this" until this date. */
  keptCurrentUntil: string | null;
  /** What the source reported, restored when the student clears their term override. */
  synced: {
    termStart: string | null;
    termEnd: string | null;
    timeline: CourseTimeline;
    lifecycle: CourseLifecycle;
  };
  modules: Module[];
  materials: MaterialView[];
  announcements: MaterialView[];
  deadlines: Deadline[];
}

export interface MockDb {
  dataDir: string;
  sources: SourceRecord[];
  courses: MockCourse[];
  studyPlan: StoredStudyPlan | null;
  /** Simulates another process (the CLI) holding sync.lock. */
  externalSyncRunning: boolean;
  /** What the panic hook recorded last time ("crashed" scenario). */
  lastCrash: CrashReport | null;
}

export const MOCK_BINARY_PATH = "/Users/demo/PageLamp/target/debug/pagelamp";

// Calendar arithmetic (not "+ N × 24 h"), so dates stay right across daylight-saving changes.
export function at(now: Date, days: number, hour = 12, minute = 0): string {
  return new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() + days,
    hour,
    minute,
  ).toISOString();
}

export function dateOnly(now: Date, days: number): string {
  const d = new Date(now.getFullYear(), now.getMonth(), now.getDate() + days);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

// ---------------------------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------------------------

export const SOURCE_FOLDER = "folder:demo-courses";
export const SOURCE_ICAL = "ical:demo-calendar";
export const SOURCE_CANVAS = "canvas:canvas.demo.test";

function sources(now: Date, scenario: MockScenario): SourceRecord[] {
  const ok = new Date(now.getTime() - 2 * 60 * 60 * 1000).toISOString();
  const failed = (kind: SourceErrorKind, message: string) => ({
    last_error: message,
    last_error_kind: kind,
  });
  return [
    {
      id: SOURCE_FOLDER,
      kind: "folder",
      label: "Course folder",
      config: { path: "/Users/demo/Documents/Courses", term_start: dateOnly(now, -23) },
      last_synced_at: ok,
      ...(scenario === "error"
        ? failed("not_found", "Folder /Users/demo/Documents/Courses was not found.")
        : { last_error: null, last_error_kind: null }),
    },
    {
      id: SOURCE_ICAL,
      kind: "ical",
      label: "Course calendar",
      config: {},
      last_synced_at: ok,
      last_error: null,
      last_error_kind: null,
    },
    {
      id: SOURCE_CANVAS,
      kind: "canvas",
      label: "Demo Canvas",
      config: { base_url: "https://canvas.demo.test", account_name: "Demo Student" },
      last_synced_at: scenario === "expired" ? at(now, -9, 18) : ok,
      ...(scenario === "expired"
        ? failed(
            "auth_expired_or_revoked",
            "Canvas rejected the access token (401). It may have expired or been revoked.",
          )
        : { last_error: null, last_error_kind: null }),
    },
  ];
}

// ---------------------------------------------------------------------------------------------
// Courses
// ---------------------------------------------------------------------------------------------

export interface CourseSpec {
  id: string;
  sourceId: string;
  code: string;
  name: string;
  policy: AiPolicy;
  policyNote: string | null;
  hidden: boolean;
  aiAccess?: boolean;
  /** False = Canvas no longer lists the course as active (term over). */
  enrollmentActive?: boolean;
  /** Question (b) (M1); unanswered when left out. */
  materialSharing?: MaterialSharing;
  termStartDays: number | null;
  week: number | null;
  confidence: Confidence;
  evidence: string[];
  url: string | null;
  /** Phase, term resolution and evidence items (defaults: from `week` and the term start). */
  calendar?: Partial<CalendarFields>;
  /** The student's own dates (term_source "user"). */
  userTerm?: boolean;
}

export function course(spec: CourseSpec, now: Date): Course {
  const c: CourseWithSharing = {
    id: spec.id,
    source_id: spec.sourceId,
    external_id: spec.code,
    code: spec.code,
    name: spec.name,
    term_start: spec.termStartDays === null ? null : dateOnly(now, spec.termStartDays),
    term_end: spec.termStartDays === null ? null : dateOnly(now, spec.termStartDays + 12 * 7 + 4),
    url: spec.url,
    ai_policy: spec.policy,
    ai_policy_note: spec.policyNote,
    ai_access: spec.aiAccess ?? true,
    material_sharing: spec.materialSharing ?? "unanswered",
    enrollment_active: spec.enrollmentActive ?? true,
    term_source: spec.userTerm ? "user" : spec.termStartDays === null ? "none" : "synced",
    hidden: spec.hidden,
    updated_at: at(now, -1, 9),
  };
  return c;
}

/** Assemble a MockCourse, remembering the synced term so an override can be undone. */
export function mockCourse(
  parts: Omit<MockCourse, "synced" | "keptCurrentUntil"> & { keptCurrentUntil?: string | null },
): MockCourse {
  return {
    ...parts,
    keptCurrentUntil: parts.keptCurrentUntil ?? null,
    synced: {
      termStart: parts.course.term_start ?? null,
      termEnd: parts.course.term_end ?? null,
      timeline: parts.timeline,
      lifecycle: parts.lifecycle,
    },
  };
}

/** The calendar fields a v0.1-style spec implies: teaching week N from the term start. */
function defaultCalendar(spec: CourseSpec, now: Date): CalendarFields {
  if (spec.week === null || spec.termStartDays === null) {
    return calendarFields({
      phase: "unknown",
      phase_confidence: "low",
      evidence_items: [ev("no_week_signal")],
    });
  }
  const start = dayFrom(now, spec.termStartDays);
  return calendarFields({
    phase: "teaching",
    phase_confidence: spec.confidence,
    default_week: spec.week,
    term: resolution({
      week_one_monday: mondayOf(start),
      teaching: [
        {
          first_class: start,
          last_class: dayFrom(now, spec.termStartDays + 12 * 7 + 4),
          first_week_number: 1,
        },
      ],
      anchor: spec.sourceId === SOURCE_FOLDER ? "folder_config" : "lms_course_dates",
      anchor_confidence: "medium",
    }),
  });
}

export function timeline(spec: CourseSpec, now: Date, moduleIds: string[]): CourseTimeline {
  const fields = { ...defaultCalendar(spec, now), ...spec.calendar };
  return {
    as_of: dateOnly(now, 0),
    current_week: spec.week,
    confidence: spec.confidence,
    evidence: spec.evidence,
    current_module_ids: moduleIds,
    outside_term: fields.phase === "not_started" || fields.phase === "ended",
    ...fields,
  };
}

let materialSeq = 0;
export function material(
  courseId: string,
  title: string,
  kind: MaterialKind,
  week: number | null,
  publishedDays: number,
  now: Date,
  opts: {
    status?: TextStatus;
    chunks?: number;
    module?: Module;
    error?: string;
    /** Why there is no text (the facade's MaterialView.text_problem). */
    problem?: TextProblem;
    url?: string;
    /** A Canvas file that can't be downloaded on request. */
    blocked?: DownloadBlock;
  } = {},
): MaterialView {
  materialSeq += 1;
  return {
    id: `${courseId}/material/${materialSeq}`,
    course_id: courseId,
    title,
    kind,
    module_id: opts.module?.id ?? null,
    module_name: opts.module?.name ?? null,
    week_hint: week,
    published_at: at(now, publishedDays, 9, 30),
    url: opts.url ?? `https://canvas.demo.test/files/${materialSeq}`,
    text_status: opts.status ?? "ok",
    text_error: opts.error ?? null,
    text_problem: opts.problem ?? null,
    download_blocked: opts.blocked ?? null,
    chunk_count: (opts.status ?? "ok") === "ok" ? (opts.chunks ?? 8) : 0,
  };
}

let eventSeq = 0;
export function deadline(
  c: Course,
  title: string,
  kind: EventKind,
  dueDays: number,
  now: Date,
  hour = 23,
  minute = 59,
): Deadline {
  eventSeq += 1;
  const when = at(now, dueDays, hour, minute);
  const isDue = kind === "assignment_due" || kind === "quiz_due" || kind === "planner_item";
  return {
    id: `${SOURCE_ICAL}/event/${eventSeq}`,
    source_id: SOURCE_ICAL,
    course_id: c.id,
    course_code: c.code ?? null,
    course_name: c.name,
    kind,
    title,
    due_at: isDue ? when : null,
    starts_at: isDue ? null : when,
    ends_at: null,
    url: `https://canvas.demo.test/calendar#event-${eventSeq}`,
    updated_at: at(now, -1, 8),
  };
}

export function weekModules(courseId: string, names: string[], now: Date, termStartDays: number) {
  return names.map<Module>((name, i) => ({
    id: `${courseId}/module/${i + 1}`,
    course_id: courseId,
    name,
    position: i + 1,
    unlock_at: at(now, termStartDays + i * 7, 8),
    week_hint: i + 1,
  }));
}

function demo101(now: Date): MockCourse {
  const spec: CourseSpec = {
    id: `${SOURCE_FOLDER}/course/DEMO101`,
    sourceId: SOURCE_FOLDER,
    code: "DEMO101",
    name: "Intro to Demo Studies",
    policy: "learning_aid",
    policyNote:
      "Syllabus §5: AI tools may be used to review and understand material, not to write graded work.",
    hidden: false,
    termStartDays: -23,
    week: 4,
    confidence: "high",
    evidence: [
      `Module 'Week 4: Sampling and Surveys' unlocked ${dateOnly(now, -2)}`,
      `Term started ${dateOnly(now, -23)} (course folder settings) → week 4, which agrees`,
      "Reading week is not modelled in v0.1",
    ],
    url: null,
    calendar: {
      notes_week: 4,
      evidence_items: [
        ev("week_from_module_unlock", {
          title: "Week 4: Sampling and Surveys",
          date: dateOnly(now, -2),
          week: 4,
        }),
        ev("signal_agrees", { signal: "dates", week: 4 }),
        ev("breaks_unknown"),
      ],
    },
  };
  const c = course(spec, now);
  const modules = weekModules(
    c.id,
    [
      "Week 1: What Is a Demo?",
      "Week 2: Placeholder Data",
      "Week 3: Measuring Nothing Carefully",
      "Week 4: Sampling and Surveys",
    ],
    now,
    -23,
  );
  const [m1, m2, m3, m4] = modules as [Module, Module, Module, Module];
  const materials = [
    material(c.id, "Week 1 slides — What Is a Demo?", "file", 1, -23, now, {
      module: m1,
      chunks: 24,
    }),
    material(c.id, "Course syllabus", "syllabus", null, -24, now, { chunks: 6 }),
    material(c.id, "Week 2 slides — Placeholder Data", "file", 2, -16, now, {
      module: m2,
      chunks: 28,
    }),
    material(c.id, "Reading: Chapter 2, Making Up Numbers Responsibly", "file", 2, -16, now, {
      module: m2,
      chunks: 14,
    }),
    material(c.id, "Week 3 slides — Measuring Nothing Carefully", "file", 3, -9, now, {
      module: m3,
      chunks: 31,
    }),
    material(c.id, "Lab 3 notebook", "file", 3, -9, now, { module: m3, chunks: 12 }),
    material(c.id, "Week 3 lecture recording", "file", 3, -8, now, {
      module: m3,
      status: "unsupported",
    }),
    material(c.id, "Week 4 slides — Sampling and Surveys", "file", 4, -2, now, {
      module: m4,
      chunks: 32,
    }),
    material(c.id, "Reading: Chapter 4, Who Gets Asked", "file", 4, -2, now, {
      module: m4,
      chunks: 18,
    }),
    material(c.id, "Lab 4 notebook — Survey Simulation", "file", 4, -2, now, {
      module: m4,
      chunks: 15,
    }),
    material(c.id, "Survey dataset (large archive)", "file", 4, -2, now, {
      module: m4,
      status: "not_downloaded",
    }),
    material(c.id, "Week 4 practice questions", "page", 4, -1, now, { module: m4, chunks: 3 }),
    // A scan: read without errors, but no text in it (like pagelamp-core's ingest).
    material(c.id, "Scanned handout — sampling frames", "file", 4, -1, now, {
      module: m4,
      chunks: 0,
      error: "no extractable text (scanned?)",
      problem: "no_text",
    }),
  ];
  const announcements = [
    material(c.id, "Office hours move to Thursday this week", "announcement", null, -1, now, {
      chunks: 1,
    }),
    material(c.id, "Problem Set 2 is posted", "announcement", null, -6, now, { chunks: 1 }),
  ];
  const deadlines = [
    deadline(c, "Reading response 3", "assignment_due", -3, now),
    deadline(c, "Problem Set 2", "assignment_due", 2, now),
    deadline(c, "Quiz 3 — Sampling", "quiz_due", 5, now, 10, 0),
    deadline(c, "Lecture 9", "class_event", 1, now, 10, 0),
    deadline(c, "Midterm test", "exam", 12, now, 18, 0),
  ];
  return mockCourse({
    course: c,
    timeline: timeline(spec, now, [m4.id]),
    lifecycle: lifecycle({
      state: "current",
      confidence: "high",
      last_activity: dateOnly(now, -1),
      next_event: dateOnly(now, 1),
    }),
    modules,
    materials,
    announcements,
    deadlines,
  });
}

function demo205(now: Date): MockCourse {
  const spec: CourseSpec = {
    id: `${SOURCE_CANVAS}/course/205`,
    sourceId: SOURCE_CANVAS,
    code: "DEMO205",
    name: "Foundations of Sample Data",
    policy: "unknown",
    policyNote: null,
    hidden: false,
    materialSharing: "not_allowed",
    termStartDays: -24,
    week: 4,
    confidence: "medium",
    evidence: [`Term started ${dateOnly(now, -24)} (from Canvas) → week 4`],
    url: "https://canvas.demo.test/courses/205",
    calendar: {
      notes_week: 3,
      evidence_items: [
        ev("lms_course_dates", { start: dateOnly(now, -24), end: dateOnly(now, -24 + 12 * 7 + 4) }),
        ev("week_from_dates", { week: 4, monday: dateOnly(now, -24) }),
        ev("breaks_unknown"),
      ],
    },
  };
  const c = course(spec, now);
  const modules = weekModules(
    c.id,
    ["Unit A: Tables", "Unit B: Columns", "Unit C: Rows"],
    now,
    -24,
  );
  const [ma, mb, mc] = modules as [Module, Module, Module];
  const materials = [
    material(c.id, "Unit A notes", "page", 1, -24, now, { module: ma, chunks: 9 }),
    material(c.id, "Unit B notes", "page", 2, -17, now, { module: mb, chunks: 11 }),
    material(c.id, "Unit C notes", "page", 3, -10, now, { module: mc, chunks: 10 }),
    // Canvas files stay "not downloaded" until the student asks (download_course_files).
    material(c.id, "Unit C worked examples", "file", 4, -3, now, {
      module: mc,
      status: "not_downloaded",
    }),
    // Over the download limit: listed, but asking for a download won't help.
    material(c.id, "Unit C lecture recording", "file", 4, -3, now, {
      module: mc,
      status: "not_downloaded",
      blocked: "too_large",
    }),
    material(c.id, "Course website", "external_link", null, -24, now, { status: "unsupported" }),
  ];
  const deadlines = [
    deadline(c, "Exercise set 3", "assignment_due", 6, now),
    deadline(c, "Exercise set 4", "assignment_due", 13, now),
  ];
  return mockCourse({
    course: c,
    timeline: timeline(spec, now, [mc.id]),
    lifecycle: lifecycle({
      state: "current",
      last_activity: dateOnly(now, -3),
      next_event: dateOnly(now, 6),
    }),
    modules,
    materials,
    announcements: [],
    deadlines,
  });
}

function demo310(now: Date): MockCourse {
  const spec: CourseSpec = {
    id: `${SOURCE_FOLDER}/course/DEMO310`,
    sourceId: SOURCE_FOLDER,
    code: "DEMO310",
    name: "Seminar in Example Analysis",
    policy: "prohibited",
    policyNote: "Syllabus p. 2: no generative AI for any part of this course.",
    hidden: false,
    materialSharing: "not_sure",
    termStartDays: null,
    week: null,
    confidence: "low",
    evidence: ["No term dates and no week-numbered folders — set the term start to fix this"],
    url: null,
  };
  const c = course(spec, now);
  const materials = [
    material(c.id, "Seminar reading list", "file", null, -20, now, { chunks: 4 }),
    material(c.id, "Discussion guide", "file", null, -5, now, { chunks: 6 }),
  ];
  return mockCourse({
    course: c,
    timeline: timeline(spec, now, []),
    lifecycle: lifecycle({ state: "unknown", confidence: "low", last_activity: dateOnly(now, -5) }),
    modules: [],
    materials,
    announcements: [],
    deadlines: [deadline(c, "Seminar presentation", "assignment_due", 9, now, 14, 0)],
  });
}

function demo099(now: Date): MockCourse {
  const spec: CourseSpec = {
    id: `${SOURCE_CANVAS}/course/99`,
    sourceId: SOURCE_CANVAS,
    code: "DEMO099",
    name: "Orientation Placeholder",
    policy: "unknown",
    policyNote: null,
    hidden: true,
    materialSharing: "allowed",
    enrollmentActive: false,
    termStartDays: -30,
    week: 5,
    confidence: "medium",
    evidence: [`Term started ${dateOnly(now, -30)} (from Canvas) → week 5`],
    url: "https://canvas.demo.test/courses/99",
  };
  const c = course(spec, now);
  return mockCourse({
    course: c,
    // Its materials had reached week 5 when it ended (a past course shows no such line).
    timeline: { ...timeline(spec, now, []), notes_week: 5 },
    lifecycle: lifecycle({
      state: "ended",
      confidence: "high",
      since: dateOnly(now, -3),
      last_activity: dateOnly(now, -30),
      evidence_items: [ev("no_longer_listed")],
    }),
    modules: [],
    materials: [material(c.id, "Welcome page", "page", 1, -30, now, { chunks: 2 })],
    announcements: [],
    deadlines: [],
  });
}

// ---------------------------------------------------------------------------------------------
// Study plan (as if the student's AI app saved it via MCP `save_study_plan`)
// ---------------------------------------------------------------------------------------------

function studyPlan(now: Date, courses: MockCourse[]): StoredStudyPlan {
  const [c101, c205] = courses.map((c) => c.course.id) as [string, string];
  return {
    id: 1,
    created_at: at(now, -2, 20, 15),
    plan: {
      horizon_start: dateOnly(now, -1),
      horizon_end: dateOnly(now, 12),
      notes: "Front-load Problem Set 2, then shift to midterm review from next week.",
      items: [
        {
          date: dateOnly(now, -1),
          course_id: c101,
          title: "Skim Week 4 slides",
          minutes: 30,
          done: true,
        },
        {
          date: dateOnly(now, 0),
          course_id: c101,
          title: "Read Chapter 4 and summarise sampling frames",
          minutes: 60,
          done: false,
        },
        {
          date: dateOnly(now, 0),
          course_id: c205,
          title: "Work through Unit C examples",
          minutes: 45,
          done: false,
        },
        {
          date: dateOnly(now, 1),
          course_id: c101,
          title: "Problem Set 2 — questions 1–3",
          minutes: 90,
          done: false,
        },
        {
          date: dateOnly(now, 2),
          course_id: c101,
          title: "Problem Set 2 — finish and check",
          minutes: 60,
          done: false,
        },
        {
          date: dateOnly(now, 4),
          course_id: c101,
          title: "Quiz 3 practice questions",
          minutes: 40,
          done: false,
        },
        {
          date: dateOnly(now, 6),
          course_id: c205,
          title: "Exercise set 3",
          minutes: 75,
          done: false,
        },
        {
          date: dateOnly(now, 8),
          course_id: c101,
          title: "Midterm review: weeks 1–2",
          minutes: 90,
          done: false,
        },
      ],
    },
  };
}

// ---------------------------------------------------------------------------------------------
// "Connect your AI app"
// ---------------------------------------------------------------------------------------------

export function mcpClientConfigs(binary: string): McpClientConfig[] {
  const launch = { command: binary, args: ["mcp"], env: {} };
  return [
    {
      client: "claude_desktop",
      title: "Claude Desktop",
      install_kind: "json_snippet",
      config_path_hint: "~/Library/Application Support/Claude/claude_desktop_config.json",
      content: JSON.stringify(
        { mcpServers: { pagelamp: { command: binary, args: ["mcp"] } } },
        null,
        2,
      ),
      notes: [
        "Works on every Claude plan, including Free.",
        "On Team, Enterprise and Education plans an admin can turn extensions off.",
        "Quit Claude Desktop before editing its config: it rewrites the file when it quits.",
      ],
      note_codes: [
        "works_on_all_claude_plans",
        "admins_may_disable_extensions",
        "quit_before_editing",
      ],
      launch,
    },
    {
      client: "claude_code",
      title: "Claude Code",
      install_kind: "shell_command",
      config_path_hint: null,
      content: `claude mcp add --scope user pagelamp -- ${binary} mcp`,
      notes: [
        "Claude Code needs a paid Claude plan (Pro or higher).",
        "Run this once in a terminal; new Claude Code sessions then have the server.",
      ],
      note_codes: ["needs_paid_claude_plan", "restart_client_after_change"],
      launch,
    },
    {
      client: "codex",
      title: "Codex / ChatGPT desktop (Work/Codex mode)",
      install_kind: "toml_snippet",
      config_path_hint: "~/.codex/config.toml",
      content: `[mcp_servers.pagelamp]\ncommand = "${binary}"\nargs = ["mcp"]\n`,
      notes: [
        "The ChatGPT desktop app (Work/Codex mode) reads the same ~/.codex/config.toml.",
        "Documented for ChatGPT Plus and higher, and for Edu.",
        "Support on the Free and Go plans isn't documented.",
        "Restart the app after changing its config.",
      ],
      note_codes: [
        "codex_config_shared_with_chatgpt_desktop",
        "codex_plus_and_edu_documented",
        "free_go_undocumented",
        "restart_client_after_change",
      ],
      launch,
    },
    {
      client: "generic",
      title: "Other MCP clients",
      install_kind: "json_snippet",
      config_path_hint: null,
      // A bare server definition (no "mcpServers" wrapper), like the backend's.
      content: JSON.stringify({ command: binary, args: ["mcp"] }, null, 2),
      notes: [
        "Any MCP client that can launch a local (stdio) server can use PageLamp: adapt this command, arguments and environment to that client's config format.",
      ],
      note_codes: ["generic_stdio_client"],
      launch,
    },
  ];
}

// ---------------------------------------------------------------------------------------------

export function buildMockDb(now: Date, scenario: MockScenario): MockDb {
  materialSeq = 0;
  eventSeq = 0;
  const dataDir = "/Users/demo/Library/Application Support/dev.PageLamp.PageLamp";
  if (scenario === "empty") {
    return {
      dataDir,
      sources: [],
      courses: [],
      studyPlan: null,
      externalSyncRunning: false,
      lastCrash: null,
    };
  }
  const courses = [demo101(now), demo205(now), demo310(now), demo099(now)];
  return {
    dataDir,
    sources: sources(now, scenario),
    courses,
    studyPlan: studyPlan(now, courses),
    externalSyncRunning: scenario === "busy",
    lastCrash: scenario === "crashed" ? crash(now) : null,
  };
}

function crash(now: Date): CrashReport {
  return {
    time: at(now, -1, 21, 14),
    version: MOCK_APP_VERSION,
    process: "app",
    message: "called `Option::unwrap()` on a `None` value",
    location: "crates/pagelamp-app/src/sync.rs:212:31",
  };
}

/**
 * Stand-in for the Rust diagnostic report (pagelamp_app::diagnostics): same kind of content,
 * made-up values. The real format is decided by the backend; the UI only shows the text.
 */
export function diagnosticReport(status: AppStatus, lastCrash: CrashReport | null, now: Date) {
  const sources = status.sources.map(
    (s) =>
      `| ${s.kind} | ${s.last_error_kind ? "no" : "yes"} | ${s.last_synced_at ?? "never"} | ${s.last_error_kind ?? "—"} |`,
  );
  const crashLine = lastCrash
    ? `${lastCrash.time} · ${lastCrash.process} · ${lastCrash.message}${lastCrash.location ? ` (${lastCrash.location})` : ""}`
    : "none";
  return [
    "# PageLamp diagnostic report",
    "",
    `- Version: ${status.version}`,
    "- OS: macOS 15.5 (aarch64)",
    "- Data folder: ~/Library/Application Support/dev.PageLamp.PageLamp",
    "- Database: ok",
    "- Keychain: available",
    "",
    "## Sources",
    "",
    "| kind | ok | last synced | last error |",
    "| --- | --- | --- | --- |",
    ...(sources.length > 0 ? sources : ["| — | — | — | — |"]),
    "",
    "## Library",
    "",
    `${status.counts.courses} courses (${status.counts.hidden_courses} hidden), ${status.counts.materials} materials, ${status.counts.events} events`,
    "",
    "## Last crash",
    "",
    crashLine,
    "",
    "## Recent log (redacted)",
    "",
    "```",
    `${now.toISOString()} INFO  pagelamp::sync: sync finished (course-1, course-2, course-3)`,
    `${now.toISOString()} DEBUG pagelamp::canvas: GET /api/v1/courses → 200 (token [redacted])`,
    "```",
    "",
  ].join("\n");
}
