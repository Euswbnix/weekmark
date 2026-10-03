import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ApiError } from "@/api/errors";
import { createMockApi } from "@/api/mock";
import { queryKeys } from "@/api/queries";
import { paths } from "@/lib/routes";
import { renderRoute } from "@/test/render";
import { useNoteRunStore } from "./useWeeklyNote";

const MONDAY = new Date(2026, 8, 28, 10, 0); // Monday 2026-09-28
const TUESDAY = new Date(2026, 8, 29, 10, 0);

function mockApi(options: Parameters<typeof createMockApi>[0] = {}) {
  return createMockApi({ latencyMs: 0, syncStepMs: 0, scenario: "ai-key", ...options });
}

async function card() {
  return screen.findByRole("region", { name: "Weekly note" });
}

/** Write, once the estimate has come back. */
async function writeButton(name = "Write my weekly note") {
  const button = await within(await card()).findByRole("button", { name });
  await waitFor(() => expect(button).not.toHaveAttribute("aria-disabled"));
  return button;
}

/** OpenAI with a key, its disclosure read, chosen for the weekly note. */
async function withNoteModel(api: ReturnType<typeof createMockApi>) {
  const openai = { kind: "provider", provider_id: "openai" } as const;
  await api.addModelProvider("openai", null, "sk-demo-key-7731");
  const version = (await api.aiStatus()).backends[0]?.disclosure.version ?? 0;
  await api.acknowledgeAiDisclosure(openai, version);
  await api.setFeatureModel("weekly_note", {
    backend: openai,
    model: "gpt-5.4-mini",
    effort: "lowest",
  });
}

describe("Courses → Weekly note", () => {
  it("says before the click when there's nothing to write about this week", async () => {
    const api = mockApi({ scenario: "all-past", now: () => TUESDAY });
    await withNoteModel(api);
    const write = vi.spyOn(api, "writeWeeklyNote");
    renderRoute("/courses", { api });
    const region = await card();
    expect(
      await within(region).findByText(/nothing to write about this week yet/),
    ).toBeInTheDocument();
    expect(within(region).getByRole("button", { name: "Write my weekly note" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    expect(within(region).queryByText(/Please check what you entered/)).toBeNull();
    expect(write).not.toHaveBeenCalled();
  });

  it("reads its estimate again when the window comes back (a sync elsewhere filled the week)", async () => {
    const api = mockApi({ scenario: "all-past", now: () => TUESDAY });
    await withNoteModel(api);
    renderRoute("/courses", { api });
    const region = await card();
    await within(region).findByText(/nothing to write about this week yet/);
    // Elsewhere, a course counts as current again.
    const [course] = await api.listCourses();
    if (!course) throw new Error("all-past has courses");
    await api.keepCourseCurrent(course.course.id, null);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(await writeButton()).toBeInTheDocument();
    expect(within(region).queryByText(/nothing to write about this week yet/)).toBeNull();
  });

  it("sits between the week's deadlines and the study plan", async () => {
    renderRoute("/courses", { api: mockApi({ now: () => TUESDAY }) });
    await card();
    const titles = screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent);
    const week = titles.findIndex((t) => /This week/.test(t ?? ""));
    const note = titles.indexOf("Weekly note");
    const plan = titles.findIndex((t) => /study plan/i.test(t ?? ""));
    expect(week).toBeGreaterThanOrEqual(0);
    expect(note).toBeGreaterThan(week);
    expect(plan).toBeGreaterThan(note);
  });

  it("writes a note with ≈ $x first, Stop while it runs, then the note with its focus", async () => {
    const api = mockApi({ syncStepMs: 100, now: () => TUESDAY });
    const write = vi.spyOn(api, "writeWeeklyNote");
    const { user } = renderRoute("/courses", { api });
    const button = await writeButton();
    expect(button).toHaveAccessibleDescription(/≈/);
    await user.click(button);
    const stop = await screen.findByRole("button", { name: "Stop" });
    await waitFor(() => expect(stop).toHaveFocus());
    expect(stop).toHaveAccessibleDescription(/doesn't stop it/);
    const note = await screen.findByRole("article", { name: /^Weekly note for the week of/ });
    await waitFor(() => expect(note).toHaveFocus());
    expect(write).toHaveBeenCalledWith(
      expect.any(String),
      expect.objectContaining({ automatic: false, override_budget: false, ui_language: "en" }),
      expect.any(Function),
    );
    expect(within(note).getByRole("heading", { name: "Focus this week" })).toBeInTheDocument();
    expect(within(note).getAllByRole("listitem").length).toBeGreaterThan(0);
    expect(within(note).getByText(/^AI-generated · /)).toBeInTheDocument();
    expect(screen.getByText("Your weekly note is ready.")).toHaveAttribute("role", "status");
  });

  it("keeps a run going when the student leaves the page; only Stop stops it", async () => {
    const api = mockApi({ syncStepMs: 150, now: () => TUESDAY });
    const cancel = vi.spyOn(api, "cancelGeneration");
    const { user, router } = renderRoute("/courses", { api });
    await user.click(await writeButton());
    await screen.findByRole("button", { name: "Stop" });
    await act(() => router.navigate(paths.settings));
    await act(() => router.navigate(paths.courses));
    expect(cancel).not.toHaveBeenCalled();
    await screen.findByRole("article", { name: /^Weekly note for the week of/ });
    expect(await api.weeklyNotes()).toHaveLength(1);
  });

  it("a run that is no longer the app's run changes nothing when it ends", async () => {
    const api = mockApi({ syncStepMs: 100, now: () => TUESDAY });
    const { user } = renderRoute("/courses", { api });
    await user.click(await writeButton());
    await screen.findByRole("button", { name: "Stop" });
    // The run store starts over (as it does between tests) while that run goes on.
    act(() => useNoteRunStore.setState({ run: { phase: "idle" }, automaticProblem: null }));
    // The run ends and its note is saved (the list shows it)...
    await screen.findByRole("article", { name: /^Weekly note for the week of/ });
    // ...but it doesn't become the store's run again.
    expect(useNoteRunStore.getState().run).toEqual({ phase: "idle" });
    expect(screen.queryByText("Your weekly note is ready.")).toBeNull();
  });

  it("stops with Stop, says so, and saves nothing", async () => {
    const api = mockApi({ syncStepMs: 150, now: () => TUESDAY });
    const { user } = renderRoute("/courses", { api });
    await user.click(await writeButton());
    await user.click(await screen.findByRole("button", { name: "Stop" }));
    expect(await screen.findByText("Stopped. Nothing was saved.")).toHaveFocus();
    expect(await api.weeklyNotes()).toEqual([]);
  });

  it("sends one cancel per Stop; a failed cancel lets Stop be pressed again", async () => {
    const api = mockApi({ syncStepMs: 300, now: () => TUESDAY });
    const realCancel = api.cancelGeneration.bind(api);
    const cancel = vi
      .spyOn(api, "cancelGeneration")
      .mockRejectedValueOnce(new ApiError("internal", "The cancel didn't get through."));
    const { user } = renderRoute("/courses", { api });
    await user.click(await writeButton());
    await user.click(await screen.findByRole("button", { name: "Stop" }));
    // The failed cancel: the run goes on, and Stop is back.
    const again = await screen.findByRole("button", { name: "Stop" });
    expect(cancel).toHaveBeenCalledTimes(1);
    let release = () => {};
    cancel.mockImplementationOnce(
      (id) =>
        new Promise<void>((resolve) => {
          release = () => void realCancel(id).then(resolve);
        }),
    );
    await user.click(again);
    const stopping = await screen.findByRole("button", { name: "Stopping…" });
    await user.click(stopping);
    expect(cancel).toHaveBeenCalledTimes(2);
    release();
    expect(await screen.findByText("Stopped. Nothing was saved.")).toBeInTheDocument();
    expect(cancel).toHaveBeenCalledTimes(2);
  });

  it("deletes a note after asking, and the focus goes on", async () => {
    const api = mockApi({ now: () => TUESDAY });
    await api.writeWeeklyNote("n-1", {}, () => {});
    const { user } = renderRoute("/courses", { api });
    const note = await screen.findByRole("article", { name: /^Weekly note for the week of/ });
    await user.click(within(note).getByRole("button", { name: "Delete…" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Delete this note?" });
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    expect(await screen.findByText("Note deleted.")).toBeInTheDocument();
    expect(await api.weeklyNotes()).toEqual([]);
    await waitFor(() =>
      expect(screen.queryByRole("article", { name: /^Weekly note for the week of/ })).toBeNull(),
    );
    // Nothing is left to show: the focus goes to Write's controls.
    const write = within(await card()).getByRole("button", { name: "Write my weekly note" });
    await waitFor(() => expect(write.closest("[tabindex]")).toHaveFocus());
  });

  it("copies the note with its focus list and the AI-generated label", async () => {
    const api = mockApi({ now: () => TUESDAY });
    const written = await api.writeWeeklyNote("n-1", {}, () => {});
    const { user } = renderRoute("/courses", { api });
    const note = await screen.findByRole("article", { name: /^Weekly note for the week of/ });
    // user-event installs its own clipboard stub, so read it back.
    await user.click(within(note).getByRole("button", { name: "Copy" }));
    const copied = await navigator.clipboard.readText();
    expect(copied.startsWith(written.text)).toBe(true);
    expect(copied).toContain("Focus this week\n1. ");
    expect(copied).toMatch(/AI-generated · /);
  });

  it("shows an earlier note from the list, and the focus goes to it", async () => {
    const api = mockApi({ now: () => TUESDAY });
    await api.writeWeeklyNote("n-1", {}, () => {});
    await api.writeWeeklyNote("n-2", {}, () => {});
    const { user } = renderRoute("/courses", { api });
    const earlier = await screen.findByRole("region", { name: "Earlier notes" });
    expect(within(earlier).getByText("Showing")).toBeInTheDocument();
    await user.click(within(earlier).getByRole("button", { name: /^Show / }));
    const note = screen.getByRole("article", { name: /^Weekly note for the week of/ });
    await waitFor(() => expect(note).toHaveFocus());
    // The other one can be shown again.
    expect(within(earlier).getByRole("button", { name: /^Show / })).toBeInTheDocument();
  });

  it("says when a note was left without graded work, and when it's for an earlier week", async () => {
    const api = mockApi({ now: () => TUESDAY });
    const note = await api.writeWeeklyNote("n-1", {}, () => {});
    vi.spyOn(api, "weeklyNotes").mockResolvedValue([
      { ...note, graded_work_left_out: 1, week_of: "2026-09-14" },
    ]);
    renderRoute("/courses", { api });
    expect(
      await screen.findByText(
        "1 focus item was left out because it looked like an answer to graded work.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("This note is for an earlier week.")).toBeInTheDocument();
  });
});

describe("Monday's note (the opt-in)", () => {
  it("is prepared at launch, on its own: no focus moves, never over the budget", async () => {
    const api = mockApi({ scenario: "weekly-note-monday", syncStepMs: 100, now: () => MONDAY });
    const write = vi.spyOn(api, "writeWeeklyNote");
    renderRoute("/courses", { api });
    expect(await screen.findByText("Preparing Monday's note")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop" })).not.toHaveFocus();
    await screen.findByRole("article", { name: /^Weekly note for the week of/ });
    expect(screen.getByText(/Prepared on Monday/)).toBeInTheDocument();
    expect(write).toHaveBeenCalledTimes(1);
    expect(write.mock.calls[0]?.[1]).toMatchObject({ automatic: true, override_budget: false });
  });

  it("runs again on the next Monday of the same launch (the tray keeps the app open)", async () => {
    let clock = MONDAY;
    const api = mockApi({ scenario: "ai-key", now: () => clock });
    await api.setPrepareWeeklyNoteOnMonday(true);
    const write = vi.spyOn(api, "writeWeeklyNote");
    const { queryClient } = renderRoute("/courses", { api });
    await waitFor(() => expect(write).toHaveBeenCalledTimes(1));
    await screen.findByRole("article", { name: /^Weekly note for the week of/ });
    // The hourly refetch on a Tuesday: nothing to do.
    clock = TUESDAY;
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    expect(write).toHaveBeenCalledTimes(1);
    clock = new Date(2026, 9, 5, 9, 0); // the next Monday
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await waitFor(() => expect(write).toHaveBeenCalledTimes(2));
    expect(write.mock.calls[1]?.[1]).toMatchObject({ automatic: true });
  });

  it("doesn't start while a click's run is going", async () => {
    const clock = MONDAY;
    const api = mockApi({ scenario: "ai-key", syncStepMs: 200, now: () => clock });
    const write = vi.spyOn(api, "writeWeeklyNote");
    const { user, queryClient } = renderRoute("/courses", { api });
    await user.click(await writeButton());
    await screen.findByRole("button", { name: "Stop" });
    await api.setPrepareWeeklyNoteOnMonday(true);
    await act(() => queryClient.invalidateQueries({ queryKey: queryKeys.startupTasks() }));
    await waitFor(async () => expect((await api.startupTasks()).prepare_weekly_note).toBe(true));
    expect(write).toHaveBeenCalledTimes(1);
    expect(write.mock.calls[0]?.[1]).toMatchObject({ automatic: false });
    // The click's run ends here, not during a later test.
    await screen.findByRole("article", { name: /^Weekly note for the week of/ });
  });

  it("says why when it was blocked, with no dialog; not due is silent", async () => {
    const api = mockApi({ scenario: "weekly-note-monday", now: () => MONDAY });
    vi.spyOn(api, "writeWeeklyNote").mockRejectedValue(
      new ApiError("blocked", "The AI gate stopped this run.", { blocked: "budget_reached" }),
    );
    renderRoute("/courses", { api });
    expect(await screen.findByText(/^Monday's note wasn't prepared: /)).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("alertdialog")).toBeNull();
    // The normal button is there below it.
    expect(await writeButton()).toBeInTheDocument();
  });

  it("stays silent when the week had nothing to write about as it started", async () => {
    const api = mockApi({ scenario: "weekly-note-monday", now: () => MONDAY });
    const write = vi.spyOn(api, "writeWeeklyNote").mockRejectedValue(
      new ApiError("blocked", "There is nothing to write about this week.", {
        blocked: "nothing_to_write",
      }),
    );
    renderRoute("/courses", { api });
    await waitFor(() => expect(write).toHaveBeenCalled());
    await writeButton();
    expect(screen.queryByText(/^Monday's note wasn't prepared/)).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("stays silent when it isn't due any more", async () => {
    const api = mockApi({ scenario: "weekly-note-monday", now: () => MONDAY });
    vi.spyOn(api, "writeWeeklyNote").mockRejectedValue(
      new ApiError("invalid", "The weekly note isn't due."),
    );
    renderRoute("/courses", { api });
    await writeButton();
    expect(screen.queryByText(/^Monday's note wasn't prepared/)).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("Settings → Weekly note", () => {
  async function prepareSwitch() {
    return screen.findByRole("switch", {
      name: "Prepare my weekly note when I open PageLamp on Monday",
    });
  }

  it("says what it costs before it's turned on, and turns it on", async () => {
    const api = mockApi();
    const set = vi.spyOn(api, "setPrepareWeeklyNoteOnMonday");
    const { user } = renderRoute("/settings", { api });
    const toggle = await prepareSwitch();
    await waitFor(() =>
      expect(toggle).toHaveAccessibleDescription(/each Monday, counted toward your monthly budget/),
    );
    await user.click(toggle);
    expect(set).toHaveBeenCalledWith(true);
    await waitFor(() => expect(toggle).toBeChecked());
  });

  it('says the cost depends on the week, with no amount and no "no price", when there\'s nothing to write about', async () => {
    const api = mockApi({ scenario: "all-past" });
    await withNoteModel(api);
    renderRoute("/settings", { api });
    const toggle = await prepareSwitch();
    await waitFor(() =>
      expect(toggle).toHaveAccessibleDescription(
        "With OpenAI · gpt-5.4-mini, counted toward your monthly budget (what it costs depends on the week).",
      ),
    );
  });

  it("isn't offered with the ChatGPT plan", async () => {
    const api = mockApi({ scenario: "codex-plus" });
    const settings = vi.spyOn(api, "weeklyNoteSettings");
    renderRoute("/settings", { api });
    await waitFor(() => expect(settings).toHaveBeenCalled());
    await screen.findByRole("heading", { level: 2, name: "AI models" });
    expect(screen.queryByRole("switch", { name: /^Prepare my weekly note/ })).toBeNull();
  });

  it("stays, paused, when it's on and the note's model no longer allows it", async () => {
    const api = mockApi({ scenario: "codex-plus" });
    vi.spyOn(api, "weeklyNoteSettings").mockResolvedValue({
      prepare_on_monday: true,
      prepare_on_monday_allowed: false,
    });
    const set = vi.spyOn(api, "setPrepareWeeklyNoteOnMonday");
    const { user } = renderRoute("/settings", { api });
    const toggle = await prepareSwitch();
    expect(toggle).toHaveAccessibleDescription(/^Paused: /);
    await user.click(toggle);
    expect(set).toHaveBeenCalledWith(false);
  });
});
