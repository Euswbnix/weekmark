import { describe, expect, it } from "vitest";
import { type CalendarFields, calendarFields, lifecycle, resolution } from "@/api/mock/calendar";
import type { CourseTimeline } from "@/api/types";
import { describePhase, formatShortDate, outsideWeekViews } from "./phase";

function timeline(
  fields: Partial<CalendarFields> & Pick<CalendarFields, "phase">,
  week: number | null = null,
): CourseTimeline {
  return {
    as_of: "2026-09-28",
    confidence: "medium",
    current_module_ids: [],
    current_week: week,
    evidence: [],
    outside_term: false,
    ...calendarFields(fields),
  };
}

describe("describePhase with the lifecycle", () => {
  // The facade clears the week of a course that is over, inactive or not started; without the
  // lifecycle these would read "Week unknown" for a course PageLamp knows is over.
  it("names an ended or inactive course by its lifecycle, whatever the phase", () => {
    const concluded = timeline({ phase: "teaching" });
    expect(describePhase(concluded)).toEqual({ kind: "unknown" });
    expect(describePhase(concluded, lifecycle({ state: "ended" }))).toEqual({ kind: "ended" });
    const oldSite = timeline({ phase: "unknown" });
    expect(describePhase(oldSite, lifecycle({ state: "inactive" }))).toEqual({
      kind: "inactive",
    });
    expect(
      describePhase(timeline({ phase: "exam_period" }), lifecycle({ state: "ended" })),
    ).toEqual({ kind: "ended" });
  });

  it("says when an upcoming course starts, from the lifecycle or the timeline", () => {
    const unknown = timeline({ phase: "unknown" });
    expect(
      describePhase(unknown, lifecycle({ state: "upcoming", starts_on: "2027-01-01" })),
    ).toEqual({ kind: "startsOn", date: "2027-01-01" });
    expect(
      describePhase(
        timeline({ phase: "not_started", starts_on: "2027-01-11" }),
        lifecycle({ state: "upcoming" }),
      ),
    ).toEqual({ kind: "startsOn", date: "2027-01-11" });
    expect(describePhase(unknown, lifecycle({ state: "upcoming" }))).toEqual({
      kind: "notStarted",
    });
  });

  it("keeps a week for the lifecycles that have one", () => {
    const week4 = timeline({ phase: "teaching", default_week: 4 }, 4);
    const taught = { kind: "teaching", week: 4, confidence: "medium" };
    for (const state of ["current", "finishing", "unknown"] as const) {
      expect(describePhase(week4, lifecycle({ state }))).toEqual(taught);
      expect(outsideWeekViews(week4, lifecycle({ state }))).toBe(false);
    }
    // The student's own dates put an upcoming course in a week: it keeps it.
    expect(describePhase(week4, lifecycle({ state: "upcoming" }))).toEqual(taught);
    expect(outsideWeekViews(week4, lifecycle({ state: "upcoming" }))).toBe(false);
    expect(outsideWeekViews(timeline({ phase: "unknown" }), lifecycle({ state: "upcoming" }))).toBe(
      true,
    );
    expect(outsideWeekViews(week4, lifecycle({ state: "ended" }))).toBe(true);
    // A break by the student's own dates has no current week, and keeps the course's place
    // too; a break by any other dates doesn't.
    const upcoming = lifecycle({ state: "upcoming", starts_on: "2027-01-01" });
    const ownBreak = timeline({
      phase: "break",
      default_week: 6,
      current_break_kind: "reading_week",
      term: resolution({ anchor: "student_confirmed" }),
    });
    expect(outsideWeekViews(ownBreak, upcoming)).toBe(false);
    expect(describePhase(ownBreak, upcoming).kind).toBe("break");
    const otherBreak = timeline({ phase: "break", term: resolution({ anchor: "lms_term" }) });
    expect(outsideWeekViews(otherBreak, upcoming)).toBe(true);
    expect(describePhase(otherBreak, upcoming)).toEqual({ kind: "startsOn", date: "2027-01-01" });
  });
});

describe("describePhase", () => {
  it("names a teaching week with its confidence", () => {
    expect(describePhase(timeline({ phase: "teaching", default_week: 4 }, 4))).toEqual({
      kind: "teaching",
      week: 4,
      confidence: "medium",
    });
  });

  it("names numbered and unnumbered breaks", () => {
    expect(
      describePhase(
        timeline({ phase: "break", current_break_kind: "reading_week", default_week: 7 }, 7),
      ),
    ).toEqual({ kind: "breakNumbered", week: 7, breakKind: "reading_week" });
    expect(
      describePhase(
        timeline({ phase: "break", current_break_kind: "reading_week", break_after_week: 6 }),
      ),
    ).toEqual({ kind: "breakAfter", week: 6, breakKind: "reading_week" });
    expect(describePhase(timeline({ phase: "break" }))).toEqual({
      kind: "break",
      breakKind: "other",
    });
  });

  it("names the exam period, with the last teaching week when known", () => {
    expect(describePhase(timeline({ phase: "exam_period", last_teaching_week: 12 }))).toEqual({
      kind: "examsAfter",
      week: 12,
    });
    expect(describePhase(timeline({ phase: "exam_period" }))).toEqual({ kind: "exams" });
  });

  it("gives the facade's start day before a course starts", () => {
    const course = timeline({ phase: "not_started" });
    expect(describePhase({ ...course, starts_on: "2027-01-11" })).toEqual({
      kind: "startsOn",
      date: "2027-01-11",
    });
    // Teaching dates alone don't decide it: the facade may move a weekend start to Monday.
    const term = resolution({
      teaching: [{ first_class: "2027-01-09", last_class: null, first_week_number: 1 }],
    });
    expect(describePhase(timeline({ phase: "not_started", term }))).toEqual({ kind: "notStarted" });
  });

  it("never shows a week for an ended course", () => {
    expect(describePhase(timeline({ phase: "ended" }, 22))).toEqual({ kind: "ended" });
  });

  it("keeps a week from the older signals when the phase is unknown", () => {
    expect(describePhase(timeline({ phase: "unknown" }, 3))).toEqual({
      kind: "teaching",
      week: 3,
      confidence: "medium",
    });
    expect(describePhase(timeline({ phase: "unknown" }))).toEqual({ kind: "unknown" });
  });
});

describe("formatShortDate", () => {
  it("leaves out the year when it is this year", () => {
    expect(formatShortDate("2026-10-12", "en", "2026-09-28")).toBe("Oct 12");
    expect(formatShortDate("2027-01-11", "en", "2026-09-28")).toBe("Jan 11, 2027");
    expect(formatShortDate("2027-01-11", "zh-CN", "2026-09-28")).toBe("2027年1月11日");
  });
});
