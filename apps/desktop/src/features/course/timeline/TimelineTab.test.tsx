import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { todayIso } from "@/lib/format";
import { DEMO101, DEMO310, openCourse } from "../testing";

// These tests cover the dates form a real alpha.1 build shows (v1: first and last day of
// classes). Form v2 is behind DATES_V2_UI (CourseDatesForm.test.tsx).
vi.mock("./availability", () => ({ DATES_V2_UI: false, datesV2UiEnabled: () => false }));

/** "YYYY-MM-DD" shifted by whole days (UTC arithmetic, so no DST surprises). */
function shiftIso(date: string, days: number): string {
  const [y, m, d] = date.split("-").map(Number) as [number, number, number];
  return new Date(Date.UTC(y, m - 1, d + days)).toISOString().slice(0, 10);
}

// Courses of the uoft-fall and phases mock scenarios (src/api/mock/courseScenarios.ts).
const FITTED = "canvas:canvas.demo.test/course/332"; // week from the professor's posts
const UNLABELLED = "canvas:canvas.demo.test/course/240"; // wide Canvas term, no week numbers
const LEGACY = "canvas:canvas.demo.test/course/205"; // a v0.1 start date to check
const ENDED = "canvas:canvas.demo.test/course/PHS150";

async function openTimeline(courseId: string, scenario?: "uoft-fall" | "phases") {
  const result = await openCourse(courseId, { query: "tab=timeline", scenario });
  const overview = await result.api.courseOverview(courseId);
  return {
    ...result,
    overview,
    panel: screen.getByRole("tabpanel", { name: "Timeline" }),
    start: screen.getByLabelText("First day of classes"),
    end: screen.getByLabelText("Last day of classes (optional)"),
  };
}

describe("Timeline tab — where this course is", () => {
  it("explains the week with translated evidence and the dates used", async () => {
    const { panel } = await openTimeline(DEMO101);
    const card = within(panel).getByRole("region", { name: "Where this course is" });
    expect(within(card).getByText("Week 4")).toBeInTheDocument();
    expect(
      within(card).getByText("This is very likely right. The clues are listed below."),
    ).toBeInTheDocument();
    const evidence = within(card).getAllByRole("list", { name: "How we worked this out" })[0];
    expect(
      within(evidence as HTMLElement).getByText(
        /^Module “Week 4: Sampling and Surveys” unlocked .+ → week 4$/,
      ),
    ).toBeInTheDocument();
    expect(within(card).getByText("Source")).toBeInTheDocument();
    expect(within(card).getByText("Course folder settings")).toBeInTheDocument();
    expect(within(card).getByRole("region", { name: "Course status" })).toHaveTextContent(
      "Current",
    );
  });

  it("sets a wide Canvas term aside and never prefills the form with it (CAL-2)", async () => {
    const { panel, start, end, overview } = await openTimeline(UNLABELLED, "uoft-fall");
    expect(within(panel).getAllByText("Week unknown").length).toBeGreaterThan(0);
    const notUsed = within(panel).getByRole("list", { name: "Dates not used" });
    expect(within(notUsed).getByText(/^Canvas term dates: /)).toBeInTheDocument();
    expect(
      within(notUsed).getByText(
        "Longer than a teaching term, so it looks like an enrollment window rather than class dates.",
      ),
    ).toBeInTheDocument();
    // The Canvas window is what v0.1 prefilled; now the fields stay empty.
    expect(overview.course.term_start).toBeTruthy();
    expect(start).toHaveValue("");
    expect(end).toHaveValue("");
    expect(screen.queryByText("Filled in with the dates PageLamp is using now.")).toBeNull();
  });

  it("prefills the dates the resolver uses, not the Canvas window", async () => {
    const { start, overview } = await openTimeline(FITTED, "uoft-fall");
    expect(start).toHaveValue(overview.timeline.term.week_one_monday);
    expect(start).not.toHaveValue(overview.course.term_start);
    expect(screen.getByText("Filled in with the dates PageLamp is using now.")).toBeInTheDocument();
  });
});

describe("Timeline tab — course dates form", () => {
  it("saves only the field the student changed", async () => {
    const { user, api, end, start, overview } = await openTimeline(FITTED, "uoft-fall");
    const setTerm = vi.spyOn(api, "setCourseTerm");
    const monday = overview.timeline.term.week_one_monday ?? todayIso();
    const lastDay = shiftIso(monday, 12 * 7 + 4);
    await user.type(end, lastDay);
    await user.click(screen.getByRole("button", { name: "Save dates" }));

    // The prefilled start was not touched, so it is not saved as the student's own date.
    expect(setTerm).toHaveBeenCalledWith(FITTED, null, lastDay);
    expect(await screen.findByText("Course dates saved")).toBeInTheDocument();
    expect(end).toHaveValue(lastDay);
    expect(start).toHaveValue(monday);
  });

  it("keeps the student's own untouched date when saving the other one", async () => {
    const { user, api, end, overview } = await openTimeline(LEGACY, "uoft-fall");
    const setTerm = vi.spyOn(api, "setCourseTerm");
    const own = overview.timeline.term.student_start;
    expect(own).toBeTruthy();
    const lastDay = shiftIso(own ?? todayIso(), 12 * 7);
    await user.type(end, lastDay);
    await user.click(screen.getByRole("button", { name: "Save dates" }));
    expect(setTerm).toHaveBeenCalledWith(LEGACY, own, lastDay);
  });

  it("saves with Enter and keeps focus in the date field", async () => {
    const { user, start } = await openTimeline(DEMO101);
    const newStart = shiftIso(todayIso(), -14);
    await user.clear(start);
    await user.type(start, `${newStart}{Enter}`);

    expect(await screen.findByText("Course dates saved")).toBeInTheDocument();
    expect(start).toHaveValue(newStart);
    expect(start).toHaveFocus();
    expect(screen.getByRole("button", { name: "Save dates" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });

  it("shows an inline error when classes end before they start", async () => {
    const { user, start, end } = await openTimeline(DEMO101);
    await user.clear(end);
    await user.type(end, "2000-01-01");
    expect(start).not.toHaveValue("");
    await user.click(screen.getByRole("button", { name: "Save dates" }));

    expect(
      await screen.findByText("These dates don't work. Check that classes end after they start."),
    ).toBeInTheDocument();
    expect(end).toHaveAttribute("aria-invalid", "true");
    expect(start).toHaveAttribute("aria-invalid", "true");
  });

  it("offers to clear only the student's own dates", async () => {
    const { user, api, start } = await openTimeline(DEMO310);
    const setTerm = vi.spyOn(api, "setCourseTerm");
    expect(screen.queryByRole("button", { name: "Clear my dates" })).not.toBeInTheDocument();

    await user.type(start, shiftIso(todayIso(), -21));
    await user.click(screen.getByRole("button", { name: "Save dates" }));
    await user.click(await screen.findByRole("button", { name: "Clear my dates" }));

    expect(setTerm).toHaveBeenLastCalledWith(DEMO310, null, null);
    expect(await screen.findByText("Your dates were cleared")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Clear my dates" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save dates" })).toHaveFocus();
  });

  it("shows a course that hasn't started as upcoming", async () => {
    const { user, start, panel } = await openTimeline(DEMO310);
    await user.type(start, shiftIso(todayIso(), 30));
    await user.click(screen.getByRole("button", { name: "Save dates" }));
    const status = await within(panel).findByRole("region", { name: "Course status" });
    await waitFor(() => expect(status).toHaveTextContent("Upcoming"));
    expect(within(panel).getAllByText(/^Starts /).length).toBeGreaterThan(0);
  });
});

describe("Timeline tab — checking dates kept from version 0.1", () => {
  it("confirms the dates and moves focus to the card", async () => {
    const { user, api, panel } = await openTimeline(LEGACY, "uoft-fall");
    const confirm = vi.spyOn(api, "confirmCourseDates");
    expect(within(panel).getByText("Check this course's dates")).toBeInTheDocument();
    expect(
      within(panel).getByText("Set by you in an earlier version of PageLamp"),
    ).toBeInTheDocument();

    await user.click(within(panel).getByRole("button", { name: "These dates are right" }));
    expect(confirm).toHaveBeenCalledWith(LEGACY);
    expect(await screen.findByText("Dates confirmed")).toBeInTheDocument();
    expect(within(panel).queryByText("Check this course's dates")).toBeNull();
    expect(within(panel).getByText("Set by you")).toBeInTheDocument();
    await waitFor(() =>
      expect(within(panel).getByRole("heading", { name: "Where this course is" })).toHaveFocus(),
    );
  });

  it("goes to the form with Edit dates", async () => {
    const { user, start } = await openTimeline(LEGACY, "uoft-fall");
    await user.click(screen.getByRole("button", { name: "Edit dates" }));
    expect(start).toHaveFocus();
  });
});

describe("Timeline tab — I'm still taking this", () => {
  it("keeps a past course current, then undoes it", async () => {
    const { user, api, panel } = await openTimeline(ENDED, "phases");
    const keep = vi.spyOn(api, "keepCourseCurrent");
    const clear = vi.spyOn(api, "clearKeepCourseCurrent");
    const status = within(panel).getByRole("region", { name: "Course status" });
    expect(status).toHaveTextContent("Ended");

    await user.click(within(status).getByRole("button", { name: "I'm still taking this" }));
    expect(keep).toHaveBeenCalledWith(ENDED, null);
    expect(await screen.findByText("Moved to your current courses")).toBeInTheDocument();
    expect(
      within(status).getByText(/^You marked this course as current until /),
    ).toBeInTheDocument();
    expect(status).toHaveTextContent("Current");

    await user.click(within(status).getByRole("button", { name: "Undo" }));
    expect(clear).toHaveBeenCalledWith(ENDED);
    expect(
      await within(status).findByRole("button", { name: "I'm still taking this" }),
    ).toBeInTheDocument();
  });

  it("refuses a date in the past inline", async () => {
    const { user, panel } = await openTimeline(ENDED, "phases");
    const status = within(panel).getByRole("region", { name: "Course status" });
    await user.click(within(status).getByRole("button", { name: "I'm still taking this" }));
    await user.click(await within(status).findByRole("button", { name: "Change date" }));
    const until = within(status).getByLabelText("Current until");
    expect(until).toHaveFocus();
    await user.clear(until);
    await user.type(until, "2000-01-01");
    await user.click(within(status).getByRole("button", { name: "Save date" }));
    expect(await within(status).findByText("Pick a date from today on.")).toBeInTheDocument();
    expect(until).toHaveAttribute("aria-invalid", "true");
  });
});
