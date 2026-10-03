import { describe, expect, it } from "vitest";
import { createMockApi } from ".";

const NOW = new Date(2026, 8, 28, 10, 0); // 2026-09-28
const fast = { latencyMs: 0, syncStepMs: 0, now: () => NOW };
const ENDED = "canvas:canvas.demo.test/course/PHS150";
const INACTIVE = "canvas:canvas.demo.test/course/PHS190";

const removeOptions = {
  keep_downloaded_files: false,
  purge_now: false,
  delete_pre_update_backup: false,
};

describe("mock lifecycle summary", () => {
  it("suggests past courses and snoozes the banner and single courses", async () => {
    const api = createMockApi({ ...fast, scenario: "phases" });
    let summary = await api.lifecycleSummary();
    expect(summary.suggested.sort()).toEqual([ENDED, INACTIVE]);
    expect(summary.show_banner).toBe(true);

    await api.snoozeLifecycleBanner();
    summary = await api.lifecycleSummary();
    expect(summary.show_banner).toBe(false);
    expect(summary.banner_snoozed_until).toBe("2026-10-12");

    await api.snoozeRemovalSuggestions([INACTIVE], "keep");
    summary = await api.lifecycleSummary();
    expect(summary.suggested).toEqual([ENDED]);
    const overview = await api.courseOverview(INACTIVE);
    expect(overview.lifecycle.suggest_removal).toBe(false);
  });
});

describe("mock removal", () => {
  it("removes at once, lists the course as removed, and undoes it", async () => {
    const api = createMockApi({ ...fast, scenario: "phases" });
    const report = await api.removeCourses([ENDED], removeOptions);
    expect(report.removed).toHaveLength(1);
    expect(report.removed[0]).toMatchObject({
      state: "pending",
      reason: "ended",
      purge_in_days: 7,
    });
    expect((await api.listCourses()).some((c) => c.course.id === ENDED)).toBe(false);
    expect((await api.status()).counts.removed_courses).toBe(1);

    const outcome = await api.restoreCourse(report.removed[0]?.removed_id ?? "");
    expect(outcome).toMatchObject({ restored: true, course_id: ENDED });
    expect((await api.listCourses()).some((c) => c.course.id === ENDED)).toBe(true);
    expect(await api.removedCourses()).toEqual([]);
  });

  it("previews what removal deletes and keeps", async () => {
    const api = createMockApi({ ...fast, scenario: "uoft-fall" });
    const preview = await api.removalPreview(["canvas:canvas.demo.test/course/101"]);
    expect(preview.items[0]).toMatchObject({
      source_kind: "canvas",
      cannot_sync_again: true,
      own_folder_untouched: false,
    });
    expect(preview.backup).toMatchObject({ delete_by_default: true });
  });

  it("purges now, can't restore an access-restricted course, and forgets purged ones", async () => {
    const api = createMockApi({ ...fast, scenario: "uoft-fall" });
    const id = "canvas:canvas.demo.test/course/101";
    const { removed } = await api.removeCourses([id], { ...removeOptions, purge_now: true });
    const removedId = removed[0]?.removed_id ?? "";
    expect(removed[0]?.state).toBe("purged");
    expect(await api.restoreCourse(removedId)).toMatchObject({
      restored: false,
      failure: "access_restricted",
    });
    await api.forgetRemovedCourse(removedId);
    expect(await api.removedCourses()).toEqual([]);
  });

  it("only forgets purged courses, and refuses to remove during a sync", async () => {
    const api = createMockApi({ ...fast, scenario: "removed" });
    const rows = await api.removedCourses();
    expect(rows.map((r) => r.state).sort()).toEqual(["pending", "purged", "purged", "restoring"]);
    const pending = rows.find((r) => r.state === "pending");
    const error = await api.forgetRemovedCourse(pending?.removed_id ?? "").catch((e) => e);
    expect(error.kind).toBe("invalid");

    const busy = createMockApi({ ...fast, scenario: "busy" });
    const refused = await busy
      .removeCourses(["canvas:canvas.demo.test/course/99"], removeOptions)
      .catch((e) => e);
    expect(refused.kind).toBe("busy");
  });

  it("keys a removal by its course id, like the facade", async () => {
    const api = createMockApi({ ...fast, scenario: "phases" });
    const { removed } = await api.removeCourses([ENDED], removeOptions);
    expect(removed[0]?.removed_id).toBe(ENDED);
  });

  it("retries files waiting for the Trash with every due purge; only the flag deletes them", async () => {
    const api = createMockApi({ ...fast, scenario: "removed" });
    const waiting = (await api.removedCourses()).find((r) => r.files_pending);
    const id = waiting?.removed_id ?? "";
    let report = await api.purgeRemovedCourses(null, false);
    expect(report.files_pending).toEqual([id]);
    report = await api.purgeRemovedCourses([id], false);
    expect(report.files_pending).toEqual([id]);
    report = await api.purgeRemovedCourses([id], true);
    expect(report.files_pending).toEqual([]);
    expect((await api.removedCourses()).find((r) => r.removed_id === id)?.files_pending).toBe(
      false,
    );
  });

  it("follows the facade's rules for restoring, forgetting and the backup", async () => {
    const api = createMockApi({ ...fast, scenario: "removed" });
    const rows = await api.removedCourses();
    const restoring = rows.find((r) => r.state === "restoring");
    const waiting = rows.find((r) => r.files_pending);
    // A purge skips a restore in progress, even when asked for by id.
    const report = await api.purgeRemovedCourses([restoring?.removed_id ?? ""], false);
    expect(report.purged).toEqual([]);
    // Forgetting waits for the files.
    const refused = await api.forgetRemovedCourse(waiting?.removed_id ?? "").catch((e) => e);
    expect(refused.kind).toBe("invalid");

    // The backup goes with the purge: not at removal, then with "Delete now".
    const phases = createMockApi({ ...fast, scenario: "phases" });
    const removal = await phases.removeCourses([ENDED], {
      ...removeOptions,
      delete_pre_update_backup: true,
    });
    expect(removal.backup_deleted).toBe(false);
    const purge = await phases.purgeRemovedCourses([ENDED], false);
    expect(purge.backup_deleted).toBe(true);
  });

  it("deletes a pending course's data now", async () => {
    const api = createMockApi({ ...fast, scenario: "removed" });
    const pending = (await api.removedCourses()).find((r) => r.state === "pending");
    const report = await api.purgeRemovedCourses([pending?.removed_id ?? ""], false);
    expect(report.purged).toEqual([pending?.removed_id]);
    const after = (await api.removedCourses()).find((r) => r.removed_id === pending?.removed_id);
    expect(after?.state).toBe("purged");
  });
});

describe("mock course dates v2", () => {
  it("saves breaks, the exam period and a second part", async () => {
    const api = createMockApi({ ...fast, scenario: "uoft-fall" });
    const id = "canvas:canvas.demo.test/course/240";
    await api.setCourseDates(id, {
      first_class: "2026-09-08",
      last_class: "2026-12-08",
      exams_end: "2027-04-30",
      breaks: [{ kind: "reading_week", start: "2026-09-28", end: "2026-10-02", numbered: false }],
      second_segment: {
        first_class: "2027-01-05",
        last_class: "2027-04-06",
        restart_numbering: true,
      },
    });
    const { timeline } = await api.courseOverview(id);
    expect(timeline.phase).toBe("break");
    expect(timeline.break_after_week).toBe(3);
    expect(timeline.term.teaching).toHaveLength(2);
    expect(timeline.term.teaching[1]?.first_week_number).toBe(1);
    expect(timeline.term.exams_end).toBe("2027-04-30");

    const error = await api
      .setCourseDates(id, { first_class: "2026-12-01", last_class: "2026-09-01", breaks: [] })
      .catch((e) => e);
    expect(error.kind).toBe("invalid");

    await api.setCourseDates(id, null);
    expect((await api.courseOverview(id)).timeline.phase).toBe("unknown");
  });
});
