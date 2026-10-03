import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { openCourse } from "../testing";

// The dates form v2 (behind DATES_V2_UI, which is on in tests). Courses of the uoft-fall and
// phases mock scenarios.
const UNLABELLED = "canvas:canvas.demo.test/course/240";
const FULL_YEAR = "canvas:canvas.demo.test/course/PHS180";

async function openForm(courseId: string, scenario: "uoft-fall" | "phases") {
  const result = await openCourse(courseId, { query: "tab=timeline", scenario });
  const form = screen.getByRole("form", { name: "Course dates" });
  return { ...result, form };
}

describe("course dates form v2", () => {
  it("saves classes, the end of exams and a break", async () => {
    const { user, api, form } = await openForm(UNLABELLED, "uoft-fall");
    const setDates = vi.spyOn(api, "setCourseDates");
    const f = within(form);
    // Nothing to prefill: the Canvas window was set aside.
    expect(f.getByLabelText("First day of classes")).toHaveValue("");

    await user.type(f.getByLabelText("First day of classes"), "2026-09-08");
    await user.type(f.getByLabelText("Last day of classes (optional)"), "2026-12-08");
    await user.type(f.getByLabelText("End of exams (optional)"), "2026-12-22");
    expect(f.getByText("No breaks added.")).toBeInTheDocument();
    await user.click(f.getByRole("button", { name: "Add a break" }));
    const row = f.getByRole("group", { name: "Break 1" });
    await waitFor(() => expect(within(row).getByLabelText("First day")).toHaveFocus());
    await user.type(within(row).getByLabelText("First day"), "2026-10-12");
    await user.type(within(row).getByLabelText("Last day"), "2026-10-16");
    await user.type(within(row).getByLabelText("Name (optional)"), "Reading Week");
    await user.click(f.getByRole("button", { name: "Save dates" }));

    expect(setDates).toHaveBeenCalledWith(UNLABELLED, {
      first_class: "2026-09-08",
      last_class: "2026-12-08",
      exams_end: "2026-12-22",
      breaks: [
        {
          kind: "reading_week",
          start: "2026-10-12",
          end: "2026-10-16",
          numbered: false,
          label: "Reading Week",
        },
      ],
      second_segment: null,
    });
    expect(await screen.findByText("Course dates saved")).toBeInTheDocument();
    expect(f.getByRole("button", { name: "Clear my dates" })).toBeInTheDocument();
  });

  it("refuses dates out of order before saving", async () => {
    const { user, api, form } = await openForm(UNLABELLED, "uoft-fall");
    const setDates = vi.spyOn(api, "setCourseDates");
    const f = within(form);
    await user.type(f.getByLabelText("First day of classes"), "2026-12-08");
    await user.type(f.getByLabelText("Last day of classes (optional)"), "2026-09-08");
    await user.click(f.getByRole("button", { name: "Save dates" }));
    expect(setDates).not.toHaveBeenCalled();
    expect(f.getByText(/^These dates are out of order\./)).toBeInTheDocument();
    expect(f.getByLabelText("First day of classes")).toHaveAttribute("aria-invalid", "true");
  });

  it("removes a break and returns focus to Add a break", async () => {
    const { user, form } = await openForm(UNLABELLED, "uoft-fall");
    const f = within(form);
    await user.click(f.getByRole("button", { name: "Add a break" }));
    await user.click(f.getByRole("button", { name: "Remove break 1" }));
    expect(f.queryByRole("group", { name: "Break 1" })).toBeNull();
    expect(f.getByRole("button", { name: "Add a break" })).toHaveFocus();
  });

  it("prefills a full-year course's second part and saves how its weeks are numbered", async () => {
    const { user, api, form } = await openForm(FULL_YEAR, "phases");
    const setDates = vi.spyOn(api, "setCourseDates");
    const f = within(form);
    expect(f.getByRole("checkbox", { name: /has a second part/ })).toBeChecked();
    const second = f.getByRole("group", { name: "Second part" });
    expect(within(second).getByLabelText("First day of classes")).not.toHaveValue("");
    // The fixture continues the numbering (week 13 after 12 weeks).
    expect(
      within(second).getByRole("radio", { name: "Continue from the first part" }),
    ).toBeChecked();
    expect(f.getByRole("group", { name: "Break 1" })).toBeInTheDocument();

    await user.click(within(second).getByRole("radio", { name: "Start again at week 1" }));
    await user.click(f.getByRole("button", { name: "Save dates" }));
    expect(setDates).toHaveBeenCalledWith(
      FULL_YEAR,
      expect.objectContaining({
        second_segment: expect.objectContaining({ restart_numbering: true }),
      }),
    );
  });

  it("clears the student's calendar", async () => {
    const { user, api, form } = await openForm(FULL_YEAR, "phases");
    const setDates = vi.spyOn(api, "setCourseDates");
    await user.click(within(form).getByRole("button", { name: "Clear my dates" }));
    expect(setDates).toHaveBeenCalledWith(FULL_YEAR, null);
    expect(await screen.findByText("Your dates were cleared")).toBeInTheDocument();
  });
});
