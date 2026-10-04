import { describe, expect, it } from "vitest";
import { calendarFields, lifecycle } from "@/api/mock/calendar";
import type { CourseSummary } from "@/api/types";
import { courseLabelFor, sortCourses, sourceErrors } from "./courses";

function summary(id: string, code: string | null, hidden = false): CourseSummary {
  return {
    course: {
      id,
      code,
      name: `Course ${id}`,
      external_id: id,
      source_id: "folder:test",
      hidden,
      ai_access: true,
      term_source: "none",
      enrollment_active: true,
      ai_policy: "unknown",
      updated_at: "2026-09-24T09:00:00Z",
    },
    timeline: {
      as_of: "2026-09-25",
      confidence: "low",
      current_module_ids: [],
      evidence: [],
      outside_term: false,
      ...calendarFields({ phase: "unknown" }),
    },
    lifecycle: lifecycle({ state: "current" }),
    ai_materials: "readable",
    counts: { indexed_materials: 0, materials: 0, modules: 0, upcoming_deadlines: 0 },
    source_label: "Course folder",
    structure_pending: false,
  };
}

describe("sortCourses", () => {
  it("sorts visible courses by code (numbers in order), hidden ones last", () => {
    const sorted = sortCourses([
      summary("a", "DEMO310"),
      summary("b", "DEMO099", true),
      summary("c", "DEMO20"),
      summary("d", "DEMO101"),
    ]);
    expect(sorted.map((s) => s.course.code)).toEqual(["DEMO20", "DEMO101", "DEMO310", "DEMO099"]);
  });

  it("falls back to the name when a course has no code", () => {
    const sorted = sortCourses([summary("z", "ZZZ"), summary("x", null)]);
    expect(sorted.map((s) => s.course.id)).toEqual(["x", "z"]);
  });
});

describe("courseLabelFor", () => {
  const courses = [summary("folder:test/course/DEMO101", "DEMO101"), summary("f:t/c/2", null)];

  it("finds a course by id or by code", () => {
    expect(courseLabelFor("folder:test/course/DEMO101", courses)).toBe("DEMO101");
    expect(courseLabelFor("DEMO101", courses)).toBe("DEMO101");
  });

  it("uses the name when the course has no code", () => {
    expect(courseLabelFor("f:t/c/2", courses)).toBe("Course f:t/c/2");
  });

  it("hides unknown ids but keeps unknown bare codes", () => {
    expect(courseLabelFor("canvas:host/course/42", courses)).toBeNull();
    expect(courseLabelFor("DEMO999", courses)).toBe("DEMO999");
    expect(courseLabelFor(null, courses)).toBeNull();
  });
});

describe("sourceErrors", () => {
  it("maps only failing sources", () => {
    const map = sourceErrors([
      {
        id: "a",
        kind: "canvas",
        label: "A",
        config: {},
        last_error_kind: "auth_expired_or_revoked",
      },
      { id: "b", kind: "folder", label: "B", config: {}, last_error_kind: null },
    ]);
    expect([...map]).toEqual([["a", "auth_expired_or_revoked"]]);
  });
});
