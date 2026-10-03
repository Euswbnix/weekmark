import { act, screen, waitFor, within } from "@testing-library/react";
import { toast } from "sonner";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { GenEvent } from "@/api/ai";
import { ApiError } from "@/api/errors";
import { createMockApi } from "@/api/mock";
import { renderRoute } from "@/test/render";
import { openCourse } from "../testing";

// Courses of the "proposals" mock scenario (src/api/mock/proposals.ts); it routes AI to an
// OpenAI key (gpt-6-luna), so AI reading can run.
const FITTED = "canvas:canvas.demo.test/course/332"; // AI proposal already; outline readable
const UNLABELLED = "canvas:canvas.demo.test/course/240"; // scan proposal already
const LEGACY = "canvas:canvas.demo.test/course/205"; // accepted calendar gone stale

beforeEach(() => {
  toast.dismiss();
});

function mockApi() {
  return createMockApi({ latencyMs: 0, syncStepMs: 0, scenario: "proposals" });
}

async function openReading(courseId: string, api = mockApi()) {
  const result = await openCourse(courseId, { query: "tab=timeline", scenario: "proposals", api });
  const section = await screen.findByRole("region", { name: "Read with AI" });
  return { ...result, section };
}

describe("Read the syllabus with AI", () => {
  it("shows the cost first, runs, labels the proposal and asks question (b) once", async () => {
    const { user, section } = await openReading(UNLABELLED);
    const read = within(section).getByRole("button", { name: "Read the syllabus with AI" });
    // "≈ $x" before the run.
    await waitFor(() => expect(within(section).getByText(/^Up to|^Less than/)).toBeInTheDocument());

    await user.click(read);
    expect(await within(section).findByText(/^Dates read\./)).toBeInTheDocument();
    // The new AI proposal, labelled like every AI output.
    const proposals = await screen.findByRole("region", { name: "Proposed dates" });
    expect(
      within(proposals).getByText(/^AI-generated · OpenAI · gpt-6-luna · /),
    ).toBeInTheDocument();
    // The course's first cloud run with question (b) unanswered: the one-time reminder.
    const reminder = within(section).getByRole("region", {
      name: "Check whether your instructor allows sharing course materials with AI services",
    });
    await user.click(within(reminder).getByRole("button", { name: "Not sure" }));
    await waitFor(() => expect(within(section).queryByRole("region")).toBeNull());

    // A second run: no reminder again.
    await user.click(within(section).getByRole("button", { name: "Read the syllabus with AI" }));
    expect(await within(section).findByText(/^Dates read\./)).toBeInTheDocument();
    expect(within(section).queryByRole("region")).toBeNull();
  });

  it("shows who reads it and the stage, and Stop ends it without changes", async () => {
    const api = mockApi();
    let finish: (error: unknown) => void = () => {};
    api.readCourseCalendar = vi.fn(async (_c, id, _o, onEvent: (e: GenEvent) => void) => {
      onEvent({
        type: "started",
        generation_id: id,
        backend_label: "OpenAI",
        model: "gpt-6-luna",
        on_device: false,
      });
      onEvent({ type: "stage", stage: "waiting_for_model" });
      return new Promise<never>((_, reject) => {
        finish = reject;
      });
    });
    const cancel = vi.spyOn(api, "cancelGeneration");
    const { user, section } = await openReading(FITTED, api);

    await user.click(within(section).getByRole("button", { name: "Read the syllabus with AI" }));
    expect(
      await within(section).findByText("Reading with OpenAI · gpt-6-luna"),
    ).toBeInTheDocument();
    expect(within(section).getByText("Waiting for the model…")).toBeInTheDocument();
    expect(within(section).queryByRole("button", { name: "Read the syllabus with AI" })).toBeNull();

    await user.click(within(section).getByRole("button", { name: "Stop" }));
    expect(cancel).toHaveBeenCalledWith(expect.any(String));
    expect(within(section).getByRole("button", { name: "Stopping…" })).toBeInTheDocument();
    await act(async () => finish(new ApiError("cancelled", "Stopped")));
    expect(await within(section).findByText("Stopped. Nothing was changed.")).toBeInTheDocument();
    expect(
      within(section).getByRole("button", { name: "Read the syllabus with AI" }),
    ).toBeVisible();
  });

  it("says why it can't run for a No AI course, and respects question (b)", async () => {
    const api = mockApi();
    await api.setCoursePolicy(FITTED, "prohibited", null);
    const first = await openReading(FITTED, api);
    expect(within(first.section).getByText(/^This course is marked “No AI”/)).toBeInTheDocument();
    expect(within(first.section).queryByRole("button", { name: /Read the syllabus/ })).toBeNull();
    first.unmount();

    const other = mockApi();
    await other.setCourseMaterialSharing(UNLABELLED, "not_allowed");
    const second = await openReading(UNLABELLED, other);
    expect(
      await within(second.section).findByRole("region", { name: "Not sent to an AI service" }),
    ).toBeInTheDocument();
    expect(within(second.section).queryByRole("button", { name: /Read the syllabus/ })).toBeNull();
  });

  it("finds dates without AI, or says there's nothing new", async () => {
    const { user } = await openReading(FITTED);
    await user.click(screen.getByRole("button", { name: "Find dates without AI" }));
    expect(await screen.findByText(/^Found dates\./)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Find dates without AI" }));
    expect(await screen.findByText("No new dates found in these materials.")).toBeInTheDocument();
  });
});

describe("Read the syllabus with AI: setup and staleness", () => {
  it("without a model, offers the setup and nothing more (and no batch on the home screen)", async () => {
    const api = mockApi();
    await api.setFeatureModel("course_calendar", null);
    const { section, unmount } = await openReading(UNLABELLED, api);
    const setUp = await within(section).findByRole("link", { name: "Set up AI to read syllabi" });
    expect(setUp).toHaveAttribute("href", "/settings");
    expect(within(section).queryByRole("button", { name: /Read the syllabus/ })).toBeNull();
    unmount();

    renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 2, name: "This week" });
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Read syllabi with AI" })).toBeNull(),
    );
  });

  it("says when the reading stays on this computer", async () => {
    const api = mockApi();
    api.readCourseCalendar = vi.fn(async (_c, id, _o, onEvent: (e: GenEvent) => void) => {
      onEvent({
        type: "started",
        generation_id: id,
        backend_label: "Ollama",
        model: "qwen3.5:9b",
        on_device: true,
      });
      return new Promise<never>(() => {});
    });
    const { user, section } = await openReading(FITTED, api);
    await user.click(within(section).getByRole("button", { name: "Read the syllabus with AI" }));
    expect(
      await within(section).findByText("Reading on this computer with Ollama · qwen3.5:9b"),
    ).toBeInTheDocument();
  });

  it('takes a stale calendar\'s "read again" to the reading section', async () => {
    const { user } = await openCourse(LEGACY, { query: "tab=timeline", scenario: "proposals" });
    await user.click(await screen.findByRole("button", { name: "Read the syllabus again" }));
    expect(screen.getByRole("heading", { level: 3, name: "Read with AI" })).toHaveFocus();
  });
});

describe("Read syllabi for N courses", () => {
  it("reads each offered course, lists the outcomes and accepts the passing ones", async () => {
    const api = mockApi();
    const offers = await api.syllabusReadingOffers();
    expect(offers.length).toBeGreaterThan(1);
    const { user } = renderRoute("/courses", { api });
    const region = await screen.findByRole("region", { name: "Read syllabi with AI" });
    const read = await within(region).findByRole("button", {
      name: `Read syllabi for ${offers.length} courses`,
    });
    await waitFor(() => expect(within(region).getByText(/^Up to|^Less than/)).toBeInTheDocument());

    await user.click(read);
    expect(
      await within(region).findByText(
        new RegExp(`^Done: ${offers.length} courses have dates to check\\.$`),
      ),
    ).toBeInTheDocument();
    const rows = within(region).getAllByRole("listitem");
    expect(rows).toHaveLength(offers.length);
    expect(within(rows[0] as HTMLElement).getByRole("link")).toHaveAttribute(
      "href",
      expect.stringContaining("tab=timeline"),
    );

    const accept = within(region).getByRole("button", { name: /^Accept the \d+ that passed/ });
    await user.click(accept);
    expect(await within(region).findByText(/^Dates accepted for \d+ course/)).toBeInTheDocument();
  });

  it("goes away for two weeks with Not now, and focus moves to the heading", async () => {
    const api = mockApi();
    const snooze = vi.spyOn(api, "snoozeCalendarOffers");
    const { user } = renderRoute("/courses", { api });
    const region = await screen.findByRole("region", { name: "Read syllabi with AI" });
    await user.click(
      within(region).getByRole("button", { name: "Not now: reading syllabi with AI" }),
    );
    expect(await screen.findByText("We'll offer it again in two weeks.")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: "Read syllabi with AI" })).toBeNull(),
    );
    expect(screen.getByRole("heading", { level: 1 })).toHaveFocus();
    expect(snooze).toHaveBeenCalledTimes(1);
    // The facade offers none of them now, on the card and at startup alike.
    expect(await api.syllabusReadingOffers()).toEqual([]);
    expect((await api.startupTasks()).calendar_offers_total).toBe(0);
  });

  it("stops the rest when asked", async () => {
    const api = mockApi();
    const offers = await api.syllabusReadingOffers();
    const real = api.readCourseCalendars;
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    api.readCourseCalendars = vi.fn(async (ids, batchId, options, onEvent) => {
      onEvent({ type: "course_started", course_id: ids[0] as string, index: 0, total: ids.length });
      await gate;
      return real(ids, batchId, options, onEvent);
    });
    const { user } = renderRoute("/courses", { api });
    const region = await screen.findByRole("region", { name: "Read syllabi with AI" });
    const read = await within(region).findByRole("button", { name: /^Read syllabi for/ });
    await waitFor(() => expect(read).not.toHaveAttribute("aria-disabled"));
    await user.click(read);
    expect(
      await within(region).findByText(new RegExp(`^Reading 1 of ${offers.length} · `)),
    ).toBeInTheDocument();

    await user.click(within(region).getByRole("button", { name: "Stop the rest" }));
    release();
    expect(await within(region).findByText(/^Stopped\. 0 courses have dates/)).toBeInTheDocument();
  });
});
