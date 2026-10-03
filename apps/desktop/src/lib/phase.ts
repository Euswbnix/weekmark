// What to call the point a course is at ("Week 4", "Reading week (after week 7)", "Exams",
// "Starts Jan 11", …). The facade decides the phase and the numbers (calendar design §6.6);
// this only picks the words for them.

import type { BreakKind, Confidence, CourseLifecycle, CourseTimeline } from "@/api/types";

export type PhaseLabel =
  | { kind: "teaching"; week: number; confidence: Confidence }
  | { kind: "breakNumbered"; week: number; breakKind: BreakKind }
  | { kind: "breakAfter"; week: number; breakKind: BreakKind }
  | { kind: "break"; breakKind: BreakKind }
  | { kind: "examsAfter"; week: number }
  | { kind: "exams" }
  | { kind: "ended" }
  | { kind: "inactive" }
  | { kind: "startsOn"; date: string }
  | { kind: "notStarted" }
  | { kind: "unknown" };

/**
 * A course that is over, inactive or hasn't started has no current week (the facade clears it):
 * its lifecycle says where it stands, whatever the phase (the CLI's `week_label` does the same).
 */
export function outsideWeekViews(timeline: CourseTimeline, lifecycle: CourseLifecycle): boolean {
  return (
    lifecycle.state === "ended" ||
    lifecycle.state === "inactive" ||
    // The student's own dates can put an upcoming course in a week, or in a break between
    // weeks (which has no current week): it keeps its place, as the facade does.
    (lifecycle.state === "upcoming" &&
      (timeline.current_week ?? null) === null &&
      !(timeline.phase === "break" && timeline.term.anchor === "student_confirmed"))
  );
}

export function describePhase(timeline: CourseTimeline, lifecycle?: CourseLifecycle): PhaseLabel {
  const week = timeline.current_week ?? null;
  if (lifecycle && outsideWeekViews(timeline, lifecycle)) {
    if (lifecycle.state === "ended") return { kind: "ended" };
    if (lifecycle.state === "inactive") return { kind: "inactive" };
    const start = lifecycle.starts_on ?? timeline.starts_on ?? null;
    return start ? { kind: "startsOn", date: start } : { kind: "notStarted" };
  }
  switch (timeline.phase) {
    case "teaching": {
      const shown = week ?? timeline.default_week ?? null;
      return shown === null
        ? { kind: "unknown" }
        : { kind: "teaching", week: shown, confidence: timeline.confidence };
    }
    case "break": {
      const breakKind = timeline.current_break_kind ?? "other";
      if (week !== null) return { kind: "breakNumbered", week, breakKind };
      const after = timeline.break_after_week ?? null;
      return after !== null
        ? { kind: "breakAfter", week: after, breakKind }
        : { kind: "break", breakKind };
    }
    case "exam_period": {
      const last = timeline.last_teaching_week ?? null;
      return last !== null ? { kind: "examsAfter", week: last } : { kind: "exams" };
    }
    case "ended":
      return { kind: "ended" };
    case "not_started": {
      // The facade's day teaching starts (a weekend first class gives the next Monday).
      const start = timeline.starts_on ?? null;
      return start ? { kind: "startsOn", date: start } : { kind: "notStarted" };
    }
    default:
      // Unknown: the v0.1 week signals may still give a week (at their own confidence).
      return week !== null
        ? { kind: "teaching", week, confidence: timeline.confidence }
        : { kind: "unknown" };
  }
}

/** "Jan 11" this year, "Jan 11, 2027" in another year (no time zone shift). */
export function formatShortDate(date: string, locale: string, today: string): string {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  const sameYear = today.slice(0, 4) === date.slice(0, 4);
  return new Intl.DateTimeFormat(locale, {
    ...(sameYear ? {} : { year: "numeric" }),
    month: "short",
    day: "numeric",
  }).format(new Date(y, m - 1, d));
}
