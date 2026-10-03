import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockApi } from "@/api/mock";
import i18n from "@/i18n";
import { paths } from "@/lib/routes";
import { useUiStore } from "@/stores/ui";
import { renderRoute } from "@/test/render";

const READABLE = "canvas:canvas.demo.test/course/205"; // "proposals": week 4 has one material
const WITHHELD = "folder:demo-courses/course/DEMO310"; // "demo": the AI policy withholds it

function mockApi(options: Parameters<typeof createMockApi>[0] = {}) {
  return createMockApi({ latencyMs: 0, syncStepMs: 0, scenario: "proposals", ...options });
}

async function explainButton() {
  const button = await screen.findByRole("button", { name: "Explain week 4" });
  await waitFor(() => expect(button).not.toHaveAttribute("aria-disabled"));
  return button;
}

describe("Course → Explain", () => {
  it("explains the week with cited paragraphs, check questions and the AI-generated line", async () => {
    const api = mockApi();
    const explain = vi.spyOn(api, "explainWeek");
    const { user } = renderRoute(`${paths.course(READABLE)}?tab=explain`, { api });
    await user.click(await explainButton());

    expect(await screen.findByText("The explanation is ready.")).toHaveAttribute("role", "status");
    expect(screen.getByRole("region", { name: "Explanation of week 4" })).toHaveFocus();
    expect(explain).toHaveBeenCalledWith(
      READABLE,
      4,
      expect.any(String),
      expect.objectContaining({ include: [], override_budget: false, ui_language: "en" }),
      expect.any(Function),
    );
    const article = screen.getByRole("article");
    expect(within(article).getByText(/^AI-generated · /)).toBeInTheDocument();
    const sources = within(article).getAllByRole("list", { name: "Sources" });
    expect(sources.length).toBeGreaterThan(0);
    expect(
      within(article).getByRole("heading", { name: "Check your understanding" }),
    ).toBeInTheDocument();
    // The course's first cloud run: the question (b) reminder.
    expect(screen.getByText(/Check whether your instructor allows sharing/)).toBeInTheDocument();
  });

  it("opens a citation's file on this computer", async () => {
    const api = mockApi();
    const open = vi.spyOn(api, "openMaterial");
    const { user } = renderRoute(`${paths.course(READABLE)}?tab=explain`, { api });
    await user.click(await explainButton());
    const [chip] = within(await screen.findByRole("article")).getAllByRole("button", {
      name: /, p\. \d$/,
    });
    if (!chip) throw new Error("no citation");
    await user.click(chip);
    expect(open).toHaveBeenCalledWith(expect.any(String));
  });

  it("deletes an explanation after asking", async () => {
    const api = mockApi();
    const { user } = renderRoute(`${paths.course(READABLE)}?tab=explain`, { api });
    await user.click(await explainButton());
    await user.click(await screen.findByRole("button", { name: "Delete…" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Delete this explanation?" });
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    expect(await screen.findByText("Explanation deleted.")).toBeInTheDocument();
    expect(await api.savedExplanations(READABLE, 4)).toEqual([]);
    await waitFor(() => expect(screen.queryByRole("article")).toBeNull());
  });

  it("stops a run, and nothing is saved", async () => {
    const api = mockApi({ syncStepMs: 200 });
    const { user } = renderRoute(`${paths.course(READABLE)}?tab=explain`, { api });
    await user.click(await explainButton());
    const stop = await screen.findByRole("button", { name: "Stop" });
    expect(stop).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(await screen.findByText("Stopped. Nothing was saved.")).toBeInTheDocument();
    expect(await api.savedExplanations(READABLE, 4)).toEqual([]);
  });

  it("says why a course's weeks can't be explained, and leads to its AI policy", async () => {
    const { user } = renderRoute(`${paths.course(WITHHELD)}?tab=explain`, { scenario: "demo" });
    expect(
      await screen.findByText(/This course's AI policy doesn't allow AI to read its materials/),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Explain week/ })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Open the AI policy tab" }));
    expect(
      await screen.findByRole("tab", { name: "AI policy", selected: true }),
    ).toBeInTheDocument();
    // The button went with its tab: the Policy panel takes the focus.
    await waitFor(() => expect(screen.getByRole("tabpanel", { name: "AI policy" })).toHaveFocus());
  });

  it("keeps a run going on another tab, and stops it when the course is left", async () => {
    const api = mockApi({ syncStepMs: 300 });
    const cancel = vi.spyOn(api, "cancelGeneration");
    const { user, router } = renderRoute(`${paths.course(READABLE)}?tab=explain`, { api });
    await user.click(await explainButton());
    expect(await screen.findByRole("button", { name: "Stop" })).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "This week" }));
    await user.click(screen.getByRole("tab", { name: "Explain" }));
    expect(screen.getByRole("button", { name: "Stop" })).toHaveAccessibleDescription(
      /leaving this course stops it/,
    );
    expect(cancel).not.toHaveBeenCalled();
    await act(() => router.navigate(paths.courses));
    await waitFor(() => expect(cancel).toHaveBeenCalledTimes(1));
  });

  it("moves the focus to what Show shows, and to the heading when the last one is deleted", async () => {
    const api = mockApi();
    const { user } = renderRoute(`${paths.course(READABLE)}?tab=explain`, { api });
    await user.click(await explainButton());
    await screen.findByRole("region", { name: "Explanation of week 4" });
    await user.click(await explainButton());
    await screen.findByText("The explanation is ready.");
    const show = await screen.findByRole("button", { name: /^Show / });
    await user.click(show);
    await waitFor(() =>
      expect(screen.getByRole("region", { name: "Explanation of week 4" })).toHaveFocus(),
    );
    for (let i = 0; i < 2; i++) {
      await user.click(await screen.findByRole("button", { name: "Delete…" }));
      const dialog = await screen.findByRole("alertdialog", { name: "Delete this explanation?" });
      await user.click(within(dialog).getByRole("button", { name: "Delete" }));
      await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
      await waitFor(() =>
        expect(document.activeElement).toBe(
          screen.queryByRole("region", { name: "Explanation of week 4" }) ??
            screen.getByRole("heading", { level: 3, name: "Explain a week" }),
        ),
      );
    }
    expect(screen.queryByRole("region", { name: "Explanation of week 4" })).toBeNull();
  });
});

describe("Course → Explain: what the facade does", () => {
  const DEMO101 = "folder:demo-courses/course/DEMO101"; // "ai-key": week 4 has one graded-looking

  it("offers to include what looks like graded work, and sends only that", async () => {
    const api = mockApi({ scenario: "ai-key" });
    const explain = vi.spyOn(api, "explainWeek");
    const { user } = renderRoute(`${paths.course(DEMO101)}?tab=explain`, { api });
    const button = await screen.findByRole("button", { name: "Explain week 4" });
    await waitFor(() => expect(button).not.toHaveAttribute("aria-disabled"));
    await user.click(button);
    const article = await screen.findByRole("article");
    expect(within(article).getByText(/Assignment 4 — Survey Simulation/)).toHaveTextContent(
      "(looks like graded work)",
    );
    // Over the length limit stays out whatever include says: no button for it.
    expect(within(article).getByText(/Week 4 practice questions/)).toHaveTextContent(
      "(over the length limit)",
    );
    const includeButton = within(article).getByRole("button", {
      name: "Not graded work? Include it and write again",
    });
    // Its own "≈ $x", and what PageLamp does with it, said right there and read with the button.
    await waitFor(() => expect(includeButton).not.toHaveAttribute("aria-disabled"));
    expect(includeButton).toHaveAccessibleDescription(
      /≈.*PageLamp explains concepts; it doesn't answer assignments, quizzes or exams\.$/,
    );
    await user.click(includeButton);
    await waitFor(() => expect(explain).toHaveBeenCalledTimes(2));
    const include = explain.mock.calls[1]?.[3].include ?? [];
    expect(include).toHaveLength(1);
    const assignment = (await api.weekMaterials(DEMO101, 4)).materials.find((m) =>
      m.title.startsWith("Assignment 4"),
    );
    expect(include).toEqual([assignment?.id]);
  });

  const TICK = "Go over the budget this time";
  const INCLUDE = "Not graded work? Include it and write again";
  /** The GenerateButton line a button sits in (its own "≈ $x" and tick). */
  const lineOf = (button: HTMLElement) => button.closest("div.space-y-2") as HTMLElement;
  async function assignmentOf(api: ReturnType<typeof mockApi>) {
    const found = (await api.weekMaterials(DEMO101, 4)).materials.find((m) =>
      m.title.startsWith("Assignment 4"),
    );
    if (!found) throw new Error("week 4 has Assignment 4");
    return found;
  }
  /** Ticks the week's line and explains it (a budget every run goes over). */
  async function explainOverBudget(user: ReturnType<typeof renderRoute>["user"]) {
    const button = await screen.findByRole("button", { name: "Explain week 4" });
    await waitFor(() => expect(button).toHaveAttribute("aria-disabled", "true"));
    await user.click(within(lineOf(button)).getByRole("checkbox", { name: TICK }));
    await waitFor(() => expect(button).not.toHaveAttribute("aria-disabled"));
    await user.click(button);
    await waitFor(() => expect(screen.queryByRole("button", { name: "Stop" })).toBeNull(), {
      timeout: 5000,
    });
  }

  it("prices Include with what it sends: blocked on its own line until its own tick, used once", async () => {
    const api = mockApi({ scenario: "ai-key" });
    await api.setMonthlyBudget(1);
    const assignment = await assignmentOf(api);
    const explain = vi.spyOn(api, "explainWeek");
    const estimate = vi.spyOn(api, "estimateGeneration");
    const { user } = renderRoute(`${paths.course(DEMO101)}?tab=explain`, { api });
    await explainOverBudget(user);
    const article = await screen.findByRole("article");
    const include = within(article).getByRole("button", { name: INCLUDE });
    await waitFor(() =>
      expect(estimate).toHaveBeenCalledWith(
        expect.objectContaining({
          feature: "weekly_explanation",
          week: 4,
          include: [assignment.id],
        }),
      ),
    );
    // Blocked on its own line ("≈ $x" and the reason there), and a click does nothing.
    await waitFor(() => expect(include).toHaveAttribute("aria-disabled", "true"));
    const includeTick = within(lineOf(include)).getByRole("checkbox", { name: TICK });
    expect(
      within(lineOf(include)).getByText("This would go over this month's budget."),
    ).toBeInTheDocument();
    await user.click(include);
    expect(explain).toHaveBeenCalledTimes(1);
    // Ticking Include's box leaves the week's alone.
    const week = screen.getByRole("button", { name: "Explain week 4" });
    const weekTick = within(lineOf(week)).getByRole("checkbox", { name: TICK });
    expect(weekTick).not.toBeChecked();
    await user.click(includeTick);
    expect(weekTick).not.toBeChecked();
    await waitFor(() => expect(include).not.toHaveAttribute("aria-disabled"));
    await user.click(include);
    await waitFor(() => expect(explain).toHaveBeenCalledTimes(2));
    expect(explain.mock.calls[1]?.[3]).toMatchObject({
      include: [assignment.id],
      override_budget: true,
    });
    await waitFor(() => expect(screen.queryByRole("button", { name: "Stop" })).toBeNull());
    // The week's line, back after the run, waits for a tick of its own.
    const weekAfter = screen.getByRole("button", { name: "Explain week 4" });
    expect(within(lineOf(weekAfter)).getByRole("checkbox", { name: TICK })).not.toBeChecked();
  });

  it("a stopped Include leaves no tick behind: going over the budget is chosen for each run", async () => {
    const api = mockApi({ scenario: "ai-key", syncStepMs: 400 });
    await api.setMonthlyBudget(1);
    const explain = vi.spyOn(api, "explainWeek");
    const { user } = renderRoute(`${paths.course(DEMO101)}?tab=explain`, { api });
    await explainOverBudget(user);
    const include = within(await screen.findByRole("article")).getByRole("button", {
      name: INCLUDE,
    });
    await user.click(within(lineOf(include)).getByRole("checkbox", { name: TICK }));
    await waitFor(() => expect(include).not.toHaveAttribute("aria-disabled"));
    await user.click(include);
    // While it runs, Include isn't offered.
    await user.click(await screen.findByRole("button", { name: "Stop" }));
    expect(within(screen.getByRole("article")).queryByRole("button", { name: INCLUDE })).toBeNull();
    await screen.findByText("Stopped. Nothing was saved.", undefined, { timeout: 5000 });
    expect(explain).toHaveBeenCalledTimes(2);
    // The same explanation and its Include are back, and Include waits for a tick again.
    const again = within(await screen.findByRole("article")).getByRole("button", { name: INCLUDE });
    await waitFor(() => expect(again).toHaveAttribute("aria-disabled", "true"));
    expect(within(lineOf(again)).getByRole("checkbox", { name: TICK })).not.toBeChecked();
  });

  it("a tick answers one request: Show another explanation, and Include's box starts unticked", async () => {
    const api = mockApi({ scenario: "ai-key" });
    await api.setMonthlyBudget(1);
    const original = api.explainWeek.bind(api);
    let plain = 0;
    // The second explanation leaves out one more graded-looking material: its Include asks for more.
    const explain = vi.spyOn(api, "explainWeek").mockImplementation(async (...args) => {
      const written = await original(...args);
      if ((args[3].include?.length ?? 0) > 0 || ++plain !== 2) return written;
      return {
        ...written,
        left_out: [
          ...written.left_out,
          {
            material_id: "quiz-5",
            title: "Quiz 5",
            reason: "looks_like_assessment",
            includable: true,
          },
        ],
      };
    });
    const { user } = renderRoute(`${paths.course(DEMO101)}?tab=explain`, { api });
    await explainOverBudget(user);
    await explainOverBudget(user);
    // B's Include, ticked against B's "≈ $x".
    const includeB = within(await screen.findByRole("article")).getByRole("button", {
      name: "Not graded work? Include these 2 and write again",
    });
    await user.click(within(lineOf(includeB)).getByRole("checkbox", { name: TICK }));
    // Show A: another request, so another line, with nothing ticked.
    await user.click(screen.getByRole("button", { name: /^Show / }));
    const includeA = within(await screen.findByRole("article")).getByRole("button", {
      name: INCLUDE,
    });
    await waitFor(() => expect(includeA).toHaveAttribute("aria-disabled", "true"));
    expect(within(lineOf(includeA)).getByRole("checkbox", { name: TICK })).not.toBeChecked();
    await user.click(includeA);
    expect(explain).toHaveBeenCalledTimes(2);
  });

  it("Include adds to what the explanation's run included", async () => {
    const api = mockApi({ scenario: "ai-key" });
    const assignment = await assignmentOf(api);
    const original = api.explainWeek.bind(api);
    // The included run's explanation leaves out one more graded-looking material (the facade's
    // answer after the course changed).
    const explain = vi.spyOn(api, "explainWeek").mockImplementation(async (...args) => {
      const written = await original(...args);
      if (args[3].include?.length !== 1) return written;
      return {
        ...written,
        left_out: [
          ...written.left_out,
          {
            material_id: "quiz-5",
            title: "Quiz 5",
            reason: "looks_like_assessment",
            includable: true,
          },
        ],
      };
    });
    const estimate = vi.spyOn(api, "estimateGeneration");
    const { user } = renderRoute(`${paths.course(DEMO101)}?tab=explain`, { api });
    const button = await screen.findByRole("button", { name: "Explain week 4" });
    await waitFor(() => expect(button).not.toHaveAttribute("aria-disabled"));
    await user.click(button);
    const first = within(await screen.findByRole("article")).getByRole("button", { name: INCLUDE });
    await waitFor(() => expect(first).not.toHaveAttribute("aria-disabled"));
    await user.click(first);
    await waitFor(() => expect(explain).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Stop" })).toBeNull());
    // The second Include is priced and sent with the assignment again and the quiz.
    const second = within(await screen.findByRole("article")).getByRole("button", {
      name: INCLUDE,
    });
    await waitFor(() =>
      expect(estimate).toHaveBeenCalledWith(
        expect.objectContaining({ include: [assignment.id, "quiz-5"] }),
      ),
    );
    await waitFor(() => expect(second).not.toHaveAttribute("aria-disabled"));
    await user.click(second);
    await waitFor(() => expect(explain).toHaveBeenCalledTimes(3));
    expect(explain.mock.calls[2]?.[3].include).toEqual([assignment.id, "quiz-5"]);
  });

  it("Regenerate of an explanation whose run included materials sends them again, at its own price", async () => {
    const api = mockApi({ scenario: "ai-key" });
    const assignment = await assignmentOf(api);
    const original = api.explainWeek.bind(api);
    const explain = vi.spyOn(api, "explainWeek").mockImplementation(async (...args) => {
      const written = await original(...args);
      return (args[3].include?.length ?? 0) > 0 ? { ...written, stale: true } : written;
    });
    const estimate = vi.spyOn(api, "estimateGeneration");
    const { user } = renderRoute(`${paths.course(DEMO101)}?tab=explain`, { api });
    const button = await screen.findByRole("button", { name: "Explain week 4" });
    await waitFor(() => expect(button).not.toHaveAttribute("aria-disabled"));
    await user.click(button);
    const include = within(await screen.findByRole("article")).getByRole("button", {
      name: INCLUDE,
    });
    await waitFor(() => expect(include).not.toHaveAttribute("aria-disabled"));
    // From the click on, Include's line is gone: an estimate with the include is Regenerate's own.
    estimate.mockClear();
    await user.click(include);
    await waitFor(() => expect(explain).toHaveBeenCalledTimes(2));
    const regenerate = await screen.findByRole("button", { name: "Write again" });
    await waitFor(() =>
      expect(estimate).toHaveBeenCalledWith(expect.objectContaining({ include: [assignment.id] })),
    );
    await waitFor(() => expect(regenerate).not.toHaveAttribute("aria-disabled"));
    expect(regenerate).toHaveAccessibleDescription(/≈/);
    await user.click(regenerate);
    await waitFor(() => expect(explain).toHaveBeenCalledTimes(3));
    expect(explain.mock.calls[2]?.[3].include).toEqual([assignment.id]);
  });

  it("Regenerate's tick goes with its run: the next stale explanation's line starts unticked", async () => {
    const api = mockApi({ scenario: "ai-key" });
    await api.setMonthlyBudget(1);
    const assignment = await assignmentOf(api);
    const original = api.explainWeek.bind(api);
    const explain = vi.spyOn(api, "explainWeek").mockImplementation(async (...args) => {
      const written = await original(...args);
      return (args[3].include?.length ?? 0) > 0 ? { ...written, stale: true } : written;
    });
    const { user } = renderRoute(`${paths.course(DEMO101)}?tab=explain`, { api });
    await explainOverBudget(user);
    const include = within(await screen.findByRole("article")).getByRole("button", {
      name: INCLUDE,
    });
    await user.click(within(lineOf(include)).getByRole("checkbox", { name: TICK }));
    await waitFor(() => expect(include).not.toHaveAttribute("aria-disabled"));
    await user.click(include);
    await waitFor(() => expect(explain).toHaveBeenCalledTimes(2));
    const regenerate = await screen.findByRole("button", { name: "Write again" });
    await user.click(within(lineOf(regenerate)).getByRole("checkbox", { name: TICK }));
    await waitFor(() => expect(regenerate).not.toHaveAttribute("aria-disabled"));
    await user.click(regenerate);
    await waitFor(() => expect(explain).toHaveBeenCalledTimes(3));
    expect(explain.mock.calls[2]?.[3]).toMatchObject({
      include: [assignment.id],
      override_budget: true,
    });
    // Its result is stale too: a new Regenerate line, waiting for a tick of its own.
    await waitFor(() => expect(screen.queryByRole("button", { name: "Stop" })).toBeNull());
    const next = await screen.findByRole("button", { name: "Write again" });
    await waitFor(() => expect(next).toHaveAttribute("aria-disabled", "true"));
    expect(within(lineOf(next)).getByRole("checkbox", { name: TICK })).not.toBeChecked();
  });

  it("says “include it” for one graded-looking material in Chinese too (one plural form)", async () => {
    useUiStore.setState({ locale: "zh-CN" });
    await i18n.changeLanguage("zh-CN");
    const { user } = renderRoute(`${paths.course(DEMO101)}?tab=explain`, {
      api: mockApi({ scenario: "ai-key" }),
    });
    const button = await screen.findByRole("button", { name: "讲解第 4 周" });
    await waitFor(() => expect(button).not.toHaveAttribute("aria-disabled"));
    await user.click(button);
    const article = await screen.findByRole("article");
    expect(
      within(article).getByRole("button", { name: "不是计分作业？纳入它并重新生成" }),
    ).toBeInTheDocument();
    expect(within(article).queryByText(/纳入这 1 份/)).toBeNull();
  });

  it("stops a run when AI access is turned off from the policy tab", async () => {
    const api = mockApi({ syncStepMs: 300 });
    const explain = vi.spyOn(api, "explainWeek");
    const cancel = vi.spyOn(api, "cancelGeneration");
    const { user } = renderRoute(`${paths.course(READABLE)}?tab=explain`, { api });
    await user.click(await explainButton());
    await screen.findByRole("button", { name: "Stop" });
    await user.click(screen.getByRole("tab", { name: "AI policy" }));
    await user.click(
      await screen.findByRole("switch", { name: "Let my AI app read this course's materials" }),
    );
    // The run that was started, and no other.
    await waitFor(() => expect(cancel).toHaveBeenCalledTimes(1));
    expect(cancel).toHaveBeenCalledWith(explain.mock.calls[0]?.[2]);
  });

  it("stops a cloud run when sharing the materials is answered “not allowed”", async () => {
    // Slow enough to outlast the answer's save (it waits 600 ms for a change of mind).
    const api = mockApi({ syncStepMs: 1500 });
    const explain = vi.spyOn(api, "explainWeek");
    const cancel = vi.spyOn(api, "cancelGeneration");
    const { user } = renderRoute(`${paths.course(READABLE)}?tab=explain`, { api });
    await user.click(await explainButton());
    // The API key's model runs in the cloud (the `started` event says so).
    expect(
      await screen.findByText(/^Writing with /, undefined, { timeout: 5000 }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("tab", { name: "AI policy" }));
    await user.click(await screen.findByRole("radio", { name: /^No, it's not allowed/ }));
    await waitFor(() => expect(cancel).toHaveBeenCalledWith(explain.mock.calls[0]?.[2]), {
      timeout: 5000,
    });
  });

  it("explains the recent materials of a course with no week, and keeps them", async () => {
    const NO_WEEK = "canvas:canvas.demo.test/course/240"; // "proposals": weeks unknown
    const api = mockApi();
    const explain = vi.spyOn(api, "explainWeek");
    const first = renderRoute(`${paths.course(NO_WEEK)}?tab=explain`, { api });
    const button = await screen.findByRole("button", { name: "Explain the recent materials" });
    await waitFor(() => expect(button).not.toHaveAttribute("aria-disabled"));
    expect(
      screen.getByText(
        "This course's weeks aren't known, so PageLamp explains its materials of the last 14 days.",
      ),
    ).toBeInTheDocument();
    await first.user.click(button);
    expect(
      await screen.findByRole("region", { name: "Explanation of the recent materials" }),
    ).toBeInTheDocument();
    expect(explain).toHaveBeenCalledWith(
      NO_WEEK,
      null,
      expect.any(String),
      expect.anything(),
      expect.any(Function),
    );
    first.unmount();

    // Saved and billed: still there when the course is opened again.
    renderRoute(`${paths.course(NO_WEEK)}?tab=explain`, { api });
    expect(
      await screen.findByRole("region", { name: "Explanation of the recent materials" }),
    ).toBeInTheDocument();
  });

  it("tells same-day explanations apart by their time", async () => {
    let clock = new Date(2026, 8, 29, 10, 5);
    const api = mockApi({ now: () => clock });
    const { user } = renderRoute(`${paths.course(READABLE)}?tab=explain`, { api });
    await user.click(await explainButton());
    await screen.findByRole("region", { name: "Explanation of week 4" });
    clock = new Date(2026, 8, 29, 14, 40);
    await user.click(await explainButton());
    await screen.findByText("The explanation is ready.");
    const show = await screen.findByRole("button", { name: /^Show / });
    expect(show.getAttribute("aria-label")).toMatch(/10:05/);
  });
});

describe("Settings → Language of AI explanations", () => {
  it("switches to the course materials' language", async () => {
    const api = mockApi();
    const { user } = renderRoute("/settings", { api });
    const heading = await screen.findByRole("heading", {
      level: 2,
      name: "Language of AI explanations",
    });
    const section = heading.closest("section") as HTMLElement;
    const course = await within(section).findByRole("radio", {
      name: "The course materials' language",
    });
    await user.click(course);
    await waitFor(async () => expect(await api.aiOutputLanguage()).toBe("course"));
  });
});
