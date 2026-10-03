import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockApi } from "@/api/mock";
import { renderRoute } from "@/test/render";

// "proposals": three current courses, AI routed to an API key.
function mockApi(options: Parameters<typeof createMockApi>[0] = {}) {
  return createMockApi({ latencyMs: 0, syncStepMs: 0, scenario: "proposals", ...options });
}

async function planForm() {
  await screen.findByRole("heading", { level: 1, name: "Plan your study" });
  return ready();
}

/** Write my plan, once the estimate for the form as it is now has come back. */
async function ready() {
  const button = await screen.findByRole("button", { name: "Write my plan" });
  await waitFor(() => expect(button).not.toHaveAttribute("aria-disabled"));
  return button;
}

describe("Plan your study", () => {
  it("writes a draft from the form, then saves it as the study plan to tick off", async () => {
    const api = mockApi();
    const generate = vi.spyOn(api, "generateStudyPlan");
    const { user, router } = renderRoute("/plan", { api });
    const write = await planForm();

    await user.click(screen.getByRole("button", { name: "Sat" }));
    await user.click(screen.getByRole("button", { name: "Sun" }));
    const hours = screen.getByLabelText("Study hours per week");
    await user.clear(hours);
    await user.type(hours, "20");
    await user.type(screen.getByLabelText("Anything to focus on? (optional)"), "the midterm");
    expect(write).toBeInTheDocument();
    await user.click(await ready());

    const draft = await screen.findByRole("region", { name: "Your draft plan" });
    expect(generate).toHaveBeenCalledWith(
      expect.objectContaining({
        horizon_days: 14,
        hours_per_week: 20,
        days_off: ["saturday", "sunday"],
        note: "the midterm",
        override_budget: false,
      }),
      expect.any(String),
      expect.any(Function),
    );
    expect(within(draft).getByText(/^AI-generated · /)).toBeInTheDocument();
    expect(
      within(draft).getByText(
        "1 task was left out because it looked like an answer to graded work.",
      ),
    ).toBeInTheDocument();
    expect(within(draft).getByRole("table", { name: "Your draft plan" })).toBeInTheDocument();
    expect(screen.getByText("Your draft plan is ready.")).toHaveAttribute("role", "status");

    await user.click(within(draft).getByRole("button", { name: "Use this plan" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/courses"));
    const plan = (
      await screen.findByRole("heading", { level: 2, name: "Your study plan" })
    ).closest("section");
    if (!plan) throw new Error("no plan section");
    expect(await within(plan).findByText(/^Made by PageLamp /)).toBeInTheDocument();
    expect(within(plan).getByText(/^AI-generated · /)).toBeInTheDocument();

    const [first] = await within(plan).findAllByRole("checkbox");
    if (!first) throw new Error("no plan item");
    const tick = vi.spyOn(api, "setStudyPlanItemDone");
    await user.click(first);
    await waitFor(() => expect(first).toBeChecked());
    expect(tick).toHaveBeenCalledWith(expect.any(Number), 0, true);
  });

  it("keeps the focus where the run is: Stop while it runs, then the draft's heading", async () => {
    const { user } = renderRoute("/plan", { api: mockApi({ syncStepMs: 100 }) });
    await user.click(await planForm());
    // The focus moves in an effect after the render: wait for it, not just for the element.
    const stop = await screen.findByRole("button", { name: "Stop" });
    await waitFor(() => expect(stop).toHaveFocus());
    const heading = await screen.findByRole("heading", { level: 2, name: "Your draft plan" });
    await waitFor(() => expect(heading).toHaveFocus());
  });

  it("stops a run, and nothing is saved", async () => {
    const api = mockApi({ syncStepMs: 200 });
    const accept = vi.spyOn(api, "acceptStudyPlan");
    const { user } = renderRoute("/plan", { api });
    await user.click(await planForm());
    await user.click(await screen.findByRole("button", { name: "Stop" }));
    expect(await screen.findByText("Stopped. Nothing was saved.")).toHaveFocus();
    expect(screen.getByRole("button", { name: "Write my plan" })).toBeInTheDocument();
    expect(accept).not.toHaveBeenCalled();
  });

  it("stops a run when the student leaves the page, and says so beforehand", async () => {
    const api = mockApi({ syncStepMs: 300 });
    const cancel = vi.spyOn(api, "cancelGeneration");
    const { user, router } = renderRoute("/plan", { api });
    await user.click(await planForm());
    expect(await screen.findByRole("button", { name: "Stop" })).toHaveAccessibleDescription(
      "Leaving this page stops it; nothing is saved.",
    );
    await act(() => router.navigate("/courses"));
    await waitFor(() => expect(cancel).toHaveBeenCalledTimes(1));
  });

  it("shows Write again's estimate before a second run", async () => {
    const api = mockApi();
    const generate = vi.spyOn(api, "generateStudyPlan");
    const { user } = renderRoute("/plan", { api });
    await user.click(await planForm());
    await screen.findByRole("region", { name: "Your draft plan" });
    const again = screen.getByRole("button", { name: "Write again" });
    await waitFor(() => expect(again).not.toHaveAttribute("aria-disabled"));
    expect(again).toHaveAccessibleDescription(/≈/);
    await user.click(again);
    await waitFor(() => expect(generate).toHaveBeenCalledTimes(2));
  });

  it("discards a draft back to the form", async () => {
    const { user } = renderRoute("/plan", { api: mockApi() });
    await user.click(await planForm());
    const draft = await screen.findByRole("region", { name: "Your draft plan" });
    await user.click(within(draft).getByRole("button", { name: "Discard" }));
    expect(await screen.findByText("Draft discarded.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Write my plan" })).toBeInTheDocument();
  });

  it("says what's missing before anything is sent", async () => {
    const { user } = renderRoute("/plan", { api: mockApi() });
    await planForm();
    const days = screen.getByLabelText("Days to plan");
    await user.clear(days);
    await user.type(days, "90");
    expect(screen.getByText("Plan 1 to 56 days.")).toBeInTheDocument();
    expect(days).toHaveAttribute("aria-invalid", "true");
    await user.clear(days);
    await user.type(days, "14");
    for (const course of screen.getAllByRole("checkbox")) await user.click(course);
    expect(screen.getByText("Choose at least one course.")).toBeInTheDocument();
    // The disabled button says why, too.
    expect(screen.getByRole("button", { name: "Write my plan" })).toHaveAccessibleDescription(
      /Choose at least one course\./,
    );
  });

  it("with no active course, says so instead of offering to write a plan", async () => {
    renderRoute("/plan", { scenario: "all-past" });
    expect(await screen.findByText(/No course is active right now/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "See your courses" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Write my plan" })).toBeNull();
  });

  it("without a model, leads to setting one up instead", async () => {
    renderRoute("/plan", { scenario: "demo" });
    await screen.findByRole("heading", { level: 1, name: "Plan your study" });
    expect(await screen.findByRole("link", { name: /Set up|AI models/i })).toBeInTheDocument();
  });
});
