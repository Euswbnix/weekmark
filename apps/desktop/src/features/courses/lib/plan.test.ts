import { describe, expect, it } from "vitest";
import type { StoredStudyPlan, StudyPlanItem } from "@/api/types";
import { addDays, groupPlanByDate, isInFocus, isPlanStale } from "./plan";

// Fixed local clock: Friday 2026-09-25, 10:00.
const NOW = new Date(2026, 8, 25, 10, 0);

function stored(createdAt: Date, horizonEnd: string): StoredStudyPlan {
  return {
    id: 1,
    created_at: createdAt.toISOString(),
    origin: "ai_app",
    plan: { horizon_start: "2026-09-20", horizon_end: horizonEnd, items: [] },
  };
}

function daysBefore(days: number, hours = 0): Date {
  return new Date(NOW.getTime() - (days * 24 + hours) * 60 * 60 * 1000);
}

describe("isPlanStale", () => {
  it("is fresh when recent and the horizon has not ended", () => {
    expect(isPlanStale(stored(daysBefore(2), "2026-10-07"), NOW)).toBe(false);
  });

  it("is fresh on the horizon's last day", () => {
    expect(isPlanStale(stored(daysBefore(1), "2026-09-25"), NOW)).toBe(false);
  });

  it("is stale once the horizon has ended", () => {
    expect(isPlanStale(stored(daysBefore(1), "2026-09-24"), NOW)).toBe(true);
  });

  it("is stale when made more than 7 days ago, even if the horizon is still open", () => {
    expect(isPlanStale(stored(daysBefore(7, 1), "2026-10-20"), NOW)).toBe(true);
  });

  it("is not stale at exactly 7 days old", () => {
    expect(isPlanStale(stored(daysBefore(7), "2026-10-20"), NOW)).toBe(false);
  });
});

describe("addDays", () => {
  it("crosses month and year boundaries", () => {
    expect(addDays("2026-09-30", 2)).toBe("2026-10-02");
    expect(addDays("2026-12-31", 1)).toBe("2027-01-01");
    expect(addDays("2026-03-01", -1)).toBe("2026-02-28");
  });
});

describe("groupPlanByDate", () => {
  const item = (date: string, title: string): StudyPlanItem => ({ date, title });

  it("groups by date, earliest first, keeping plan order within a day", () => {
    const days = groupPlanByDate([
      item("2026-09-27", "C"),
      item("2026-09-25", "A"),
      item("2026-09-27", "D"),
      item("2026-09-25", "B"),
    ]);
    expect(days.map((d) => [d.date, d.entries.map((e) => e.item.title)])).toEqual([
      ["2026-09-25", ["A", "B"]],
      ["2026-09-27", ["C", "D"]],
    ]);
    // Positions refer to the saved plan, so they stay stable keys.
    expect(days[1]?.entries.map((e) => e.position)).toEqual([0, 2]);
  });

  it("returns nothing for an empty plan", () => {
    expect(groupPlanByDate([])).toEqual([]);
  });
});

describe("isInFocus", () => {
  it("covers today and the next 2 days only", () => {
    const today = "2026-09-30";
    expect(isInFocus("2026-09-29", today)).toBe(false);
    expect(isInFocus("2026-09-30", today)).toBe(true);
    expect(isInFocus("2026-10-02", today)).toBe(true);
    expect(isInFocus("2026-10-03", today)).toBe(false);
  });
});
