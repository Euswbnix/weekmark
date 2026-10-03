// Mock course weeks, phases and lifecycle (calendar design §6, §8.1; F1). Builders for the
// fixtures and a much simplified stand-in for the backend resolver, so the dates form, "I'm
// still taking this" and "These dates are right" behave plausibly in `pnpm dev:mock`. The real
// rules live in pagelamp-core; nothing here is meant to match them beyond what the UI shows.

import type { Confidence, CourseDatesInput, CourseTimeline } from "../generated";
import type {
  CourseGroup,
  CourseLifecycle,
  EvidenceCode,
  EvidenceItem,
  LifecycleState,
  TermResolution,
} from "../types";

export type CalendarFields = Pick<
  CourseTimeline,
  | "phase"
  | "phase_confidence"
  | "default_week"
  | "break_after_week"
  | "last_teaching_week"
  | "current_break_kind"
  | "notes_week"
  | "starts_on"
  | "term"
  | "calendar"
  | "evidence_items"
>;

const DAY = 24 * 60 * 60 * 1000;

/** "YYYY-MM-DD" of a local date. */
export function isoOf(date: Date): string {
  const m = String(date.getMonth() + 1).padStart(2, "0");
  const d = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${m}-${d}`;
}

function parseIso(iso: string): Date {
  const [y, m, d] = iso.split("-").map(Number) as [number, number, number];
  return new Date(y, m - 1, d);
}

/** Local calendar date `days` from `now`. */
export function dayFrom(now: Date, days: number): string {
  return isoOf(new Date(now.getFullYear(), now.getMonth(), now.getDate() + days));
}

export function addDays(iso: string, days: number): string {
  const d = parseIso(iso);
  return isoOf(new Date(d.getFullYear(), d.getMonth(), d.getDate() + days));
}

/** The Monday of the week containing `iso`. */
export function mondayOf(iso: string): string {
  const d = parseIso(iso);
  const back = (d.getDay() + 6) % 7; // Monday = 0
  return addDays(iso, -back);
}

/** The day teaching starts: the date itself, or the next Monday for a Saturday or Sunday. */
export function teachingStart(iso: string): string {
  const day = parseIso(iso).getDay(); // Sunday = 0, Saturday = 6
  return day === 6 ? addDays(iso, 2) : day === 0 ? addDays(iso, 1) : iso;
}

/** The Monday `weeks` weeks from this week's Monday (negative = earlier). */
export function mondayFrom(now: Date, weeks: number): string {
  return addDays(mondayOf(isoOf(now)), weeks * 7);
}

function daysBetween(from: string, to: string): number {
  return Math.round((parseIso(to).getTime() - parseIso(from).getTime()) / DAY);
}

/** An evidence item; numbers become strings, as the facade sends them. */
export function ev(code: EvidenceCode, params: Record<string, string | number> = {}): EvidenceItem {
  return {
    code,
    params: Object.entries(params).map(([key, value]) => ({ key, value: String(value) })),
  };
}

export function resolution(partial: Partial<TermResolution> = {}): TermResolution {
  return {
    week_one_monday: null,
    teaching: [],
    breaks: [],
    exams_end: null,
    anchor: "none",
    anchor_confidence: "low",
    anchor_origin: null,
    ai_label: null,
    outer_frame: null,
    not_used: [],
    student_start: null,
    student_end: null,
    ...partial,
  };
}

export function calendarFields(
  partial: Partial<CalendarFields> & Pick<CalendarFields, "phase">,
): CalendarFields {
  return {
    phase_confidence: "medium",
    default_week: null,
    break_after_week: null,
    last_teaching_week: null,
    current_break_kind: null,
    notes_week: null,
    starts_on: null,
    term: resolution(),
    calendar: "none",
    evidence_items: [],
    ...partial,
  };
}

const GROUP_OF: Record<LifecycleState, CourseGroup> = {
  upcoming: "upcoming",
  current: "current",
  finishing: "current",
  unknown: "current",
  ended: "past",
  inactive: "past",
};

/** The facade's `is_active` on `today`: Current, Finishing or Unknown, or Upcoming and
 * starting within 14 days. */
export function activeOn(state: LifecycleState, startsOn: string | null, today: string): boolean {
  if (state === "upcoming") return startsOn !== null && startsOn <= addDays(today, 14);
  return state === "current" || state === "finishing" || state === "unknown";
}

export function lifecycle(
  partial: Partial<CourseLifecycle> & Pick<CourseLifecycle, "state">,
): CourseLifecycle {
  const past = GROUP_OF[partial.state] === "past";
  const startsOn = partial.starts_on ?? null;
  return {
    group: GROUP_OF[partial.state],
    confidence: "medium",
    since: null,
    starts_on: null,
    last_activity: null,
    next_event: null,
    evidence_items: [],
    suggest_removal: past,
    kept_current_until: null,
    is_active: activeOn(partial.state, startsOn, isoOf(new Date())),
    ...partial,
  };
}

/** What the facade would report with "I'm still taking this" set until `until`. */
export function keptCurrent(base: CourseLifecycle, until: string): CourseLifecycle {
  return {
    ...base,
    state: "current",
    group: "current",
    confidence: "high",
    suggest_removal: false,
    kept_current_until: until,
    is_active: true,
    evidence_items: [ev("kept_current", { until })],
  };
}

/** The facade's default for "I'm still taking this": the outer frame's end, else today + 120. */
export function defaultKeepUntil(timeline: CourseTimeline, today: string): string {
  const end = timeline.term.outer_frame?.end;
  return end && end > today ? end : addDays(today, 120);
}

/**
 * The timeline and lifecycle after the student saves the dates form (alpha.1: first and last
 * day of classes; the last day is an end-only anchor, design §6.6). Simplified: no breaks, no
 * other signals.
 */
export function withStudentDates(
  base: CourseTimeline,
  start: string | null,
  end: string | null,
  today: string,
): { timeline: CourseTimeline; lifecycle: CourseLifecycle } {
  const monday = start ? mondayOf(start) : (base.term.week_one_monday ?? null);
  const term = resolution({
    ...base.term,
    week_one_monday: monday,
    teaching: monday
      ? [{ first_class: start ?? monday, last_class: end, first_week_number: 1 }]
      : [],
    anchor: "student_confirmed",
    anchor_confidence: "high",
    anchor_origin: "user",
    student_start: start,
    student_end: end,
  });
  const evidence = [
    ev("student_dates", { ...(start ? { start } : {}), ...(end ? { end } : {}) }),
    ...base.evidence_items.filter((item) => item.code === "term_looks_like_enrollment_window"),
  ];
  const week =
    monday && today >= monday ? Math.floor(daysBetween(monday, mondayOf(today)) / 7) + 1 : null;
  // The English lines MCP shows (`evidence`), in the v0.1 style.
  const lines = start
    ? [`Term start set to ${start} by you${week ? ` → week ${week}` : ""}`]
    : [`Last day of classes set to ${end} by you`];
  const fields = (partial: Partial<CalendarFields> & Pick<CalendarFields, "phase">) => ({
    ...base,
    evidence: lines,
    current_week: null,
    outside_term: false,
    ...calendarFields({ term, evidence_items: evidence, phase_confidence: "high", ...partial }),
  });

  if (monday && today < monday) {
    // Like the facade: a weekend first class starts teaching the next Monday.
    const startsOn = start ? teachingStart(start) : monday;
    return {
      timeline: { ...fields({ phase: "not_started", starts_on: startsOn }), outside_term: true },
      lifecycle: lifecycle({ state: "upcoming", starts_on: startsOn }),
    };
  }
  if (end && today > end) {
    const lastWeek = monday ? Math.floor(daysBetween(monday, mondayOf(end)) / 7) + 1 : null;
    if (daysBetween(end, today) <= 21) {
      return {
        timeline: fields({
          phase: "exam_period",
          phase_confidence: "low",
          last_teaching_week: lastWeek,
        }),
        lifecycle: lifecycle({
          state: "finishing",
          evidence_items: [ev("exam_period_estimated", { days: 21 })],
        }),
      };
    }
    return {
      timeline: { ...fields({ phase: "ended" }), outside_term: true },
      lifecycle: lifecycle({
        state: "ended",
        confidence: "high",
        since: addDays(end, 21),
        evidence_items: [ev("exams_over", { date: addDays(end, 21) })],
      }),
    };
  }
  if (week === null) {
    return {
      timeline: fields({ phase: "unknown", phase_confidence: "low" }),
      lifecycle: lifecycle({ state: "unknown", suggest_removal: false }),
    };
  }
  const confidence: Confidence = "high";
  return {
    timeline: {
      ...fields({ phase: "teaching", default_week: week }),
      current_week: week,
      confidence,
    },
    lifecycle: lifecycle({ state: "current" }),
  };
}

/**
 * The timeline and lifecycle once the student's own dates are cleared: what the course's
 * source gave, minus any student dates (a v0.1 override the fixture started with).
 */
export function withoutStudentDates(
  synced: CourseTimeline,
  syncedLifecycle: CourseLifecycle,
): { timeline: CourseTimeline; lifecycle: CourseLifecycle } {
  const { term } = synced;
  if (!term.student_start && !term.student_end) {
    return { timeline: synced, lifecycle: syncedLifecycle };
  }
  return {
    timeline: {
      ...synced,
      current_week: null,
      confidence: "low",
      outside_term: false,
      ...calendarFields({
        phase: "unknown",
        phase_confidence: "low",
        term: resolution({ outer_frame: term.outer_frame, not_used: term.not_used }),
        evidence_items: synced.evidence_items.filter(
          (item) => !["student_dates", "legacy_dates", "student_end_used"].includes(item.code),
        ),
      }),
    },
    lifecycle: lifecycle({ state: "unknown", confidence: "low", suggest_removal: false }),
  };
}

/**
 * The timeline and lifecycle after the student saves the dates form v2: the first part as
 * `withStudentDates`, plus the end of exams, breaks and a second part (full-year courses). Only
 * what the UI shows is simulated: breaks give the Break phase, a later exams end extends the
 * exam period, and a second part continues or restarts the week numbers.
 */
export function withCourseDates(
  base: CourseTimeline,
  input: CourseDatesInput,
  today: string,
): { timeline: CourseTimeline; lifecycle: CourseLifecycle } {
  const second = input.second_segment ?? null;
  const lastDay = second ? (second.last_class ?? null) : (input.last_class ?? null);
  const next = withStudentDates(base, input.first_class ?? null, lastDay, today);
  const monday = next.timeline.term.week_one_monday ?? null;
  const first = next.timeline.term.teaching[0];
  const firstWeeks =
    monday && input.last_class
      ? Math.floor(daysBetween(monday, mondayOf(input.last_class)) / 7) + 1
      : 0;
  const teaching = first
    ? [
        { ...first, last_class: input.last_class ?? null },
        ...(second
          ? [
              {
                first_class: second.first_class,
                last_class: second.last_class ?? null,
                first_week_number: second.restart_numbering ? 1 : firstWeeks + 1,
              },
            ]
          : []),
      ]
    : [];
  const breaks = input.breaks.map((b) => ({
    kind: b.kind,
    span: { start: b.start, end: b.end },
    numbered: b.numbered,
    label: b.label ?? "",
  }));
  let timeline: CourseTimeline = {
    ...next.timeline,
    term: {
      ...next.timeline.term,
      teaching,
      breaks,
      exams_end: input.exams_end ?? null,
      anchor_origin: "user",
    },
  };
  let life = next.lifecycle;
  const inBreak = input.breaks.find(
    (b) => b.start <= addDays(today, 4) && b.end >= mondayOf(today),
  );
  if (inBreak && timeline.phase === "teaching") {
    timeline = {
      ...timeline,
      phase: "break",
      current_break_kind: inBreak.kind,
      current_week: inBreak.numbered ? timeline.current_week : null,
      break_after_week: inBreak.numbered ? null : (timeline.current_week ?? 1) - 1,
      default_week: inBreak.numbered ? timeline.current_week : (timeline.current_week ?? 1) - 1,
    };
  }
  if (input.exams_end && lastDay && today > lastDay && today <= input.exams_end) {
    timeline = { ...timeline, phase: "exam_period", phase_confidence: "high", outside_term: false };
    life = lifecycle({ state: "finishing", confidence: "high" });
  }
  return { timeline, lifecycle: life };
}
