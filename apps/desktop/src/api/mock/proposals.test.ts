import { describe, expect, it } from "vitest";
import { createMockApi } from ".";

const NOW = new Date(2026, 8, 28, 10, 0); // Monday 2026-09-28
const fast = { latencyMs: 0, syncStepMs: 0, now: () => NOW, scenario: "proposals" as const };
const FITTED = "canvas:canvas.demo.test/course/332";
const UNLABELLED = "canvas:canvas.demo.test/course/240";
const LEGACY = "canvas:canvas.demo.test/course/205";

describe("mock calendar proposals", () => {
  it("shows an AI proposal with a conflict and alternatives, and the candidates", async () => {
    const api = createMockApi(fast);
    const view = await api.courseCalendar(FITTED);
    expect(view.status).toBe("proposed");
    const [proposal] = view.proposals;
    expect(proposal).toMatchObject({ origin: "ai", passing: false, resulting_week_today: 4 });
    expect(proposal?.conflicts[0]?.options).toHaveLength(2);
    // The schedule page's date, and PageLamp's own (no label, no quote).
    expect(proposal?.dates[0]?.alternatives).toHaveLength(2);
    expect(proposal?.conflicts.map((c) => c.options.length)).toEqual([2, 0]);
    expect(view.candidates.map((c) => c.reason).sort()).toEqual(["syllabus", "title_schedule"]);
    expect(view.blocked ?? null).toBeNull();
  });

  it("accepts with edits, which puts the calendar in force", async () => {
    const api = createMockApi(fast);
    const { proposals } = await api.courseCalendar(UNLABELLED);
    const scan = proposals[0];
    expect(scan?.origin).toBe("scan");
    const view = await api.acceptCalendarProposal(scan?.id ?? 0, null);
    expect(view.status).toBe("accepted");
    expect(view.proposals).toEqual([]);
    const { timeline } = await api.courseOverview(UNLABELLED);
    expect(timeline.current_week).toBe(4);
    expect(timeline.term.anchor).toBe("student_confirmed");
    expect(timeline.term.anchor_origin).toBe("scan");
    expect(timeline.calendar).toBe("accepted");
  });

  it("refuses a batch accept of a proposal with conflicts, and dismisses", async () => {
    const api = createMockApi(fast);
    const id = (await api.courseCalendar(FITTED)).proposals[0]?.id ?? 0;
    const error = await api.acceptPassingProposals([id]).catch((e) => e);
    expect(error.kind).toBe("invalid");
    await api.dismissCalendarProposal(id);
    expect((await api.courseCalendar(FITTED)).status).toBe("none");
  });

  it("marks a stale accepted calendar and offers only courses without one", async () => {
    const api = createMockApi(fast);
    const view = await api.courseCalendar(LEGACY);
    expect(view.status).toBe("accepted_stale");
    expect(view.accepted).toMatchObject({
      stale: true,
      changed_materials: ["Course outline (updated)"],
    });
    const offers = (await api.syllabusReadingOffers()).map((o) => o.course_id).sort();
    expect(offers).toEqual([UNLABELLED, FITTED].sort());
  });

  it("hides the offers for 14 days after Not now, on the card and at startup", async () => {
    let clock = new Date(2026, 8, 28, 10, 0);
    const api = createMockApi({ ...fast, now: () => clock });
    expect((await api.syllabusReadingOffers()).length).toBeGreaterThan(0);
    await api.snoozeCalendarOffers();
    expect(await api.syllabusReadingOffers()).toEqual([]);
    expect((await api.startupTasks()).calendar_offers).toEqual([]);
    clock = new Date(2026, 9, 13, 10, 0); // 15 days later
    expect((await api.syllabusReadingOffers()).length).toBeGreaterThan(0);
    expect((await api.startupTasks()).calendar_offers_total).toBeGreaterThan(0);
  });

  it("downloads one outline file, and lets the student exclude a candidate", async () => {
    const api = createMockApi(fast);
    const outline = (await api.courseCalendar(UNLABELLED)).candidates.find((c) => c.downloadable);
    expect(outline?.title).toBe("Syllabus.pdf");
    await api.downloadMaterialFiles(UNLABELLED, [outline?.material_id ?? ""], () => {});
    let candidates = (await api.courseCalendar(UNLABELLED)).candidates;
    expect(candidates.find((c) => c.title === "Syllabus.pdf")).toMatchObject({
      has_text: true,
      included: true,
    });

    candidates = await api.setCalendarSources(UNLABELLED, [], [outline?.material_id ?? ""]);
    expect(candidates.find((c) => c.title === "Syllabus.pdf")).toMatchObject({
      included: false,
      left_out: "excluded_by_student",
    });
  });

  it("scans a course once", async () => {
    const api = createMockApi({ ...fast, scenario: "uoft-fall" });
    // No outline in plain uoft-fall: nothing to scan.
    expect(await api.scanCourseCalendar(FITTED)).toBeNull();
  });
});
