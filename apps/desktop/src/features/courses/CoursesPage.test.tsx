import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { PageLampApi } from "@/api/client";
import { ApiError } from "@/api/errors";
import { createMockApi } from "@/api/mock";
import { SOURCE_CANVAS } from "@/api/mock/fixtures";
import { brand } from "@/brand";
import i18n from "@/i18n";
import { AI_SETUP_ENABLED } from "@/lib/features";
import { paths } from "@/lib/routes";
import { useSyncStore } from "@/stores/sync";
import { useUiStore } from "@/stores/ui";
import { renderRoute } from "@/test/render";

/** Queries that skip visually hidden live-region copies of on-screen text. */
const VISIBLE_ONLY = "script, style, .sr-only";

function mockApi(options: Parameters<typeof createMockApi>[0] = {}): PageLampApi {
  return createMockApi({ latencyMs: 0, syncStepMs: 0, ...options });
}

function coursesRegion() {
  return screen.findByRole("region", { name: "Your courses" });
}

/** The card of a course, found through its (only) link. */
function card(list: HTMLElement, code: string): HTMLElement {
  const link = within(list).getByRole("link", { name: new RegExp(`^${code}\\b`) });
  const article = link.closest("article");
  if (!article) throw new Error(`No card for ${code}`);
  return article;
}

describe("CoursesPage — course list", () => {
  it("lists visible courses with week, AI policy, next deadline and readability", async () => {
    renderRoute("/courses");
    expect(screen.getByRole("heading", { level: 1, name: "Courses" })).toBeInTheDocument();
    const list = await coursesRegion();

    const c101 = within(card(list, "DEMO101"));
    expect(c101.getByText("Intro to Demo Studies")).toBeInTheDocument();
    expect(c101.getByText("Week 4")).toBeInTheDocument();
    expect(c101.getByText("Learning aid only")).toBeInTheDocument();
    expect(c101.getByText(/^Next: Problem Set 2 ·/)).toBeInTheDocument();
    expect(c101.getByText("10 of 13 materials readable by your AI app")).toBeInTheDocument();
    expect(c101.getByText(/^Course folder · synced/)).toBeInTheDocument();

    const c205 = within(card(list, "DEMO205"));
    expect(c205.getByText("Not set")).toBeInTheDocument();
    expect(c205.getByText("Set policy")).toBeInTheDocument();
    expect(c205.getByText(/^Next: Exercise set 3 ·/)).toBeInTheDocument();

    const c310 = within(card(list, "DEMO310"));
    expect(c310.getByText("No AI")).toBeInTheDocument();
    expect(c310.getByText("Week unknown")).toBeInTheDocument();
    expect(c310.getByText("Set the first day of classes to fix this")).toBeInTheDocument();

    // Sorted by code; the hidden DEMO099 is not shown.
    const links = within(list).getAllByRole("link");
    expect(links.map((l) => l.textContent?.slice(0, 7))).toEqual(["DEMO101", "DEMO205", "DEMO310"]);
    expect(within(list).queryByText("DEMO099")).toBeNull();
  });

  it("reveals hidden courses with the switch", async () => {
    const { user } = renderRoute("/courses");
    const list = await coursesRegion();

    await user.click(within(list).getByRole("switch", { name: "Show hidden courses (1)" }));
    // DEMO099 looks finished, so it is under the (collapsed) Past courses.
    await user.click(within(list).getByRole("button", { name: "Past courses (1)" }));

    const hidden = within(card(list, "DEMO099"));
    expect(hidden.getByText("Hidden")).toBeInTheDocument();
    expect(useUiStore.getState().showHiddenCourses).toBe(true);
    // Past courses go after the current ones.
    expect(within(list).getAllByRole("link").at(-1)?.textContent).toMatch(/^DEMO099/);
  });

  it("shows each course's AI access to materials, with icon and text", async () => {
    const api = mockApi();
    await api.setCourseAiAccess("folder:demo-courses/course/DEMO101", false);
    renderRoute("/courses", { api });
    const list = await coursesRegion();

    expect(
      within(card(list, "DEMO101")).getByText("AI access to materials is off"),
    ).toBeInTheDocument();
    expect(
      within(card(list, "DEMO205")).getByText("3 of 6 materials readable by your AI app"),
    ).toBeInTheDocument();
    // DEMO310 is a "No AI" course: its materials are withheld whatever the switch says.
    expect(
      within(card(list, "DEMO310")).getByText("Materials not shared (No AI course)"),
    ).toBeInTheDocument();
  });

  it("groups courses that look finished under Past courses, collapsed until opened", async () => {
    useUiStore.setState({ showHiddenCourses: true });
    const { user } = renderRoute("/courses");
    const list = await coursesRegion();
    expect(within(list).getByRole("heading", { level: 3, name: "Current" })).toBeInTheDocument();
    const toggle = within(list).getByRole("button", { name: "Past courses (1)" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(within(list).queryByRole("link", { name: /^DEMO099\b/ })).toBeNull();

    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(useUiStore.getState().showPastCourses).toBe(true);
    expect(within(card(list, "DEMO099")).getByText("Past course")).toBeInTheDocument();
    expect(within(card(list, "DEMO101")).queryByText("Past course")).not.toBeInTheDocument();
    expect(within(list).getByText(/Their deadlines are still listed/)).toBeInTheDocument();
  });

  it("puts a hidden course back in the list from its card", async () => {
    useUiStore.setState({ showHiddenCourses: true, showPastCourses: true });
    const { user, api } = renderRoute("/courses");
    const setHidden = vi.spyOn(api, "setCourseHidden");
    const list = await coursesRegion();
    const hidden = card(list, "DEMO099");
    // The action is a sibling of the card's link, not nested inside it.
    const show = within(hidden).getByRole("button", { name: "Show in course list" });
    expect(show.closest("a")).toBeNull();

    await user.click(show);
    expect(setHidden).toHaveBeenCalledWith("canvas:canvas.demo.test/course/99", false);
    expect(await screen.findByText("DEMO099 is back in your course list")).toBeInTheDocument();
    await waitFor(() =>
      expect(within(list).getByRole("link", { name: /^DEMO099\b/ })).toHaveFocus(),
    );
    expect(within(card(list, "DEMO099")).queryByText("Hidden")).not.toBeInTheDocument();
  });

  it("opens a course's page when its card is clicked", async () => {
    const { user, router } = renderRoute("/courses");
    const list = await coursesRegion();

    await user.click(within(list).getByRole("link", { name: /^DEMO205\b/ }));

    expect(router.state.location.pathname).toBe(paths.course(`${SOURCE_CANVAS}/course/205`));
  });

  it("explains how to see courses when every course is hidden", async () => {
    const api = mockApi();
    const real = api.listCourses;
    api.listCourses = async () => (await real()).filter((c) => c.course.hidden);
    const { user } = renderRoute("/courses", { api });
    const list = await coursesRegion();

    expect(
      within(list).getByText(
        'All of your courses are hidden. Turn on "Show hidden courses" to see them.',
      ),
    ).toBeInTheDocument();
    await user.click(within(list).getByRole("switch", { name: "Show hidden courses (1)" }));
    expect(
      within(list).getByText("No current courses. Finished ones are under Past courses."),
    ).toBeInTheDocument();
    await user.click(within(list).getByRole("button", { name: "Past courses (1)" }));
    expect(card(list, "DEMO099")).toBeInTheDocument();
  });

  it("shows loading placeholders until the data arrives", async () => {
    renderRoute("/courses");
    const main = within(screen.getByRole("main"));

    // One page-shaped skeleton first; each section then shows its own until it has loaded.
    expect(main.getByText("Loading…")).toBeInTheDocument();
    expect(main.queryByRole("region", { name: "Your courses" })).toBeNull();
    await coursesRegion();
    await waitFor(() => expect(main.queryByText("Loading…")).toBeNull());
  });
});

describe("CoursesPage — this week", () => {
  it("groups the next 7 days by day and counts real deadlines", async () => {
    renderRoute("/courses");
    const week = await screen.findByRole("region", { name: "This week" });

    expect(await within(week).findByText("Problem Set 2")).toBeInTheDocument();
    expect(within(week).getByText("3 deadlines in the next 7 days")).toBeInTheDocument();
    // The class tomorrow is listed (quietly) under its day; the midterm in 12 days is not.
    const tomorrow = within(week).getByRole("heading", { level: 3, name: /^Tomorrow/ });
    expect(
      within(tomorrow.closest("li") as HTMLElement).getByText("Lecture 9"),
    ).toBeInTheDocument();
    expect(within(week).queryByText("Midterm test")).toBeNull();
    expect(within(week).getByText("Quiz 3 — Sampling")).toBeInTheDocument();
  });

  it("says when nothing is due", async () => {
    const api = mockApi();
    api.listDeadlines = vi.fn().mockResolvedValue([]);
    renderRoute("/courses", { api });

    const week = await screen.findByRole("region", { name: "This week" });
    expect(await within(week).findByText("Nothing due in the next 7 days")).toBeInTheDocument();
  });

  it("shows an error with retry when deadlines fail to load", async () => {
    const api = mockApi();
    const real = api.listDeadlines;
    api.listDeadlines = vi
      .fn<PageLampApi["listDeadlines"]>()
      .mockRejectedValueOnce(new ApiError("internal", "Synthetic failure"))
      .mockImplementation(real);
    const { user } = renderRoute("/courses", { api });

    const week = await screen.findByRole("region", { name: "This week" });
    expect(
      await within(week).findByText("Couldn't load this week's deadlines"),
    ).toBeInTheDocument();
    await user.click(within(week).getByRole("button", { name: "Try again" }));
    expect(await within(week).findByText("Problem Set 2")).toBeInTheDocument();
  });

  it("links only http(s) addresses", async () => {
    const api = mockApi();
    const real = api.listDeadlines;
    api.listDeadlines = async (...args) =>
      (await real(...args)).map((d) =>
        d.title === "Problem Set 2" ? { ...d, url: "file:///Users/demo/Courses/ps2.pdf" } : d,
      );
    renderRoute("/courses", { api });

    const week = await screen.findByRole("region", { name: "This week" });
    expect(await within(week).findByText("Problem Set 2")).toBeInTheDocument();
    expect(within(week).queryByRole("link", { name: "Problem Set 2" })).toBeNull();
    expect(within(week).getByRole("link", { name: "Quiz 3 — Sampling" })).toHaveAttribute(
      "href",
      expect.stringMatching(/^https:\/\//),
    );
  });
});

describe("CoursesPage — study plan", () => {
  it("shows notes and the next few days, and the full plan on request", async () => {
    const { user } = renderRoute("/courses");
    const plan = await screen.findByRole("region", { name: "Your study plan" });

    expect(await within(plan).findByText(/^Front-load Problem Set 2/)).toBeInTheDocument();
    expect(within(plan).getByText(/^Made by your AI app/)).toBeInTheDocument();
    expect(within(plan).queryByText(/may be out of date/)).toBeNull();

    const today = within(plan).getByRole("heading", { level: 3, name: /^Today/ });
    const todayItems = within(today.closest("li") as HTMLElement);
    expect(
      todayItems.getByText("Read Chapter 4 and summarise sampling frames"),
    ).toBeInTheDocument();
    expect(todayItems.getByText("DEMO205")).toBeInTheDocument();
    expect(todayItems.getByText("60 min")).toBeInTheDocument();

    // Collapsed: yesterday's done task and next week's review are not shown.
    expect(within(plan).queryByText("Skim Week 4 slides")).toBeNull();
    expect(within(plan).queryByText("Midterm review: weeks 1–2")).toBeNull();

    const toggle = within(plan).getByRole("button", { name: "Show full plan" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(within(plan).getByRole("button", { name: "Show less" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );

    const doneRow = within(plan).getByText("Skim Week 4 slides").closest("li") as HTMLElement;
    expect(within(doneRow).getByText("Done")).toBeInTheDocument();
    expect(within(plan).getByText("Midterm review: weeks 1–2")).toBeInTheDocument();
    if (AI_SETUP_ENABLED) {
      // M3: items are ticked off here, each checkbox named after its task.
      expect(within(doneRow).getByRole("checkbox", { name: /Skim Week 4 slides$/ })).toBeChecked();
    } else {
      // Read-only: no checkboxes.
      expect(within(plan).queryByRole("checkbox")).toBeNull();
    }
  });

  it("flags a plan whose horizon has ended", async () => {
    const api = mockApi();
    const real = await api.latestStudyPlan();
    if (!real) throw new Error("demo has a plan");
    api.latestStudyPlan = vi.fn().mockResolvedValue({
      ...real,
      created_at: "2026-01-05T20:00:00Z",
      plan: {
        ...real.plan,
        horizon_start: "2026-01-05",
        horizon_end: "2026-01-18",
        items: real.plan.items.map((item) => ({ ...item, date: "2026-01-06" })),
      },
    });
    renderRoute("/courses", { api });

    const plan = await screen.findByRole("region", { name: "Your study plan" });
    expect(
      await within(plan).findByText(
        "This plan may be out of date — ask your AI app for a new one.",
      ),
    ).toBeInTheDocument();
    expect(
      within(plan).getByText("Nothing planned for today or the next 2 days."),
    ).toBeInTheDocument();
  });

  it("shows a task's description", async () => {
    const api = mockApi();
    const real = await api.latestStudyPlan();
    if (!real) throw new Error("demo has a plan");
    const [first, ...rest] = real.plan.items.filter((item) => !item.done);
    if (!first) throw new Error("demo plan has open tasks");
    api.latestStudyPlan = vi.fn().mockResolvedValue({
      ...real,
      plan: { ...real.plan, items: [{ ...first, description: "Sections 4.1 to 4.3" }, ...rest] },
    });
    renderRoute("/courses", { api });

    const plan = await screen.findByRole("region", { name: "Your study plan" });
    const row = (await within(plan).findByText(first.title)).closest("li") as HTMLElement;
    expect(within(row).getByText("Sections 4.1 to 4.3")).toBeInTheDocument();
  });

  it("shows a plan the AI app saved while the window was in the background", async () => {
    const api = mockApi();
    const saved = await api.latestStudyPlan();
    const latest = vi.fn<PageLampApi["latestStudyPlan"]>().mockResolvedValue(null);
    api.latestStudyPlan = latest;
    renderRoute("/courses", { api });
    const plan = await screen.findByRole("region", { name: "Your study plan" });
    expect(await within(plan).findByText("No study plan yet")).toBeInTheDocument();

    // The student saves a plan in their AI app, then comes back to PageLamp.
    latest.mockResolvedValue(saved);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    const open = saved?.plan.items.find((item) => !item.done);
    if (!open) throw new Error("demo plan has open tasks");
    expect(await within(plan).findByText(open.title)).toBeInTheDocument();
    expect(within(plan).queryByText("No study plan yet")).toBeNull();
  });

  it("explains how to get a plan when there is none", async () => {
    const api = mockApi();
    api.latestStudyPlan = vi.fn().mockResolvedValue(null);
    const { user } = renderRoute("/courses", { api });

    const plan = await screen.findByRole("region", { name: "Your study plan" });
    expect(await within(plan).findByText("No study plan yet")).toBeInTheDocument();
    expect(
      within(plan).getByText(/Make me a study plan for the next two weeks\./),
    ).toBeInTheDocument();
    expect(within(plan).getByRole("link", { name: "Connect your AI app" })).toHaveAttribute(
      "href",
      paths.connect,
    );

    await user.click(within(plan).getByRole("button", { name: "Copy the prompt" }));
    // user-event installs its own clipboard stub, so read it back.
    expect(await navigator.clipboard.readText()).toBe(
      "Make me a study plan for the next two weeks.",
    );
  });
});

describe("CoursesPage — sync", () => {
  it("syncs on 'Sync now' and shows progress until it finishes", async () => {
    const api = mockApi();
    const realSyncAll = api.syncAll;
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const syncAll = vi.fn<PageLampApi["syncAll"]>(async (req, onEvent) => {
      await gate;
      return realSyncAll(req, onEvent);
    });
    api.syncAll = syncAll;
    const { user } = renderRoute("/courses", { api });

    const button = await screen.findByRole("button", { name: "Sync now" });
    await user.click(button);

    expect(syncAll).toHaveBeenCalledTimes(1);
    expect(button).toHaveAttribute("aria-disabled", "true");
    // Progress shows in the accessory bar at the foot of the window.
    expect(await screen.findByRole("button", { name: /^Syncing/ })).toBeInTheDocument();

    release();
    await waitFor(() => expect(button).not.toHaveAttribute("aria-disabled"));
    expect(screen.getByRole("button", { name: "Sync finished" })).toBeInTheDocument();
  });

  it("explains why a sync could not run, until dismissed", async () => {
    const api = mockApi();
    api.syncAll = vi.fn().mockRejectedValue(new ApiError("network", "Synthetic failure"));
    const { user } = renderRoute("/courses", { api });

    await user.click(await screen.findByRole("button", { name: "Sync now" }));

    // The accessory bar says (and announces) "Sync failed"; the page's box explains why, and
    // announces only the why, so nothing is heard twice.
    expect(await screen.findByRole("button", { name: "Sync failed" })).toBeInTheDocument();
    const page = within(screen.getByRole("main"));
    expect(page.getByText("Sync failed", { ignore: VISIBLE_ONLY })).toBeInTheDocument();
    const why = "Couldn't reach the server. Check your internet connection and the address.";
    // (The course list's syllabus batch has a live region of its own.)
    expect(page.getAllByRole("status").some((s) => s.textContent === why)).toBe(true);
    expect(page.getByText(why, { ignore: VISIBLE_ONLY })).toBeInTheDocument();
    await user.click(page.getByRole("button", { name: "Close" }));
    expect(page.queryByText("Sync failed", { ignore: VISIBLE_ONLY })).toBeNull();
  });

  it("disables 'Sync now' while another process is syncing, and checks again", async () => {
    const api = createMockApi({ latencyMs: 0, syncStepMs: 0, scenario: "busy" });
    const status = vi.spyOn(api, "status");
    const { user } = renderRoute("/courses", { api });

    expect(
      await screen.findByText("Another sync is running", { ignore: VISIBLE_ONLY }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sync now" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );

    const before = status.mock.calls.length;
    await user.click(screen.getByRole("button", { name: "Check again" }));
    await waitFor(() => expect(status.mock.calls.length).toBeGreaterThan(before));
  });

  it("explains a source that couldn't sync", async () => {
    renderRoute("/courses", { scenario: "error" });

    expect(await screen.findByText("Course folder couldn't sync: Not found")).toBeInTheDocument();
    expect(screen.getByText("Courses from this source may be out of date.")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Open Sources & sync" })).toHaveAttribute(
      "href",
      paths.sources,
    );
    const list = await coursesRegion();
    expect(within(card(list, "DEMO101")).getByText("Not found")).toBeInTheDocument();
    expect(within(card(list, "DEMO205")).queryByText("Not found")).toBeNull();
  });

  it("explains an expired Canvas token and marks the affected courses", async () => {
    renderRoute("/courses", { scenario: "expired" });

    expect(
      await screen.findByText("Your Canvas token for Demo Canvas stopped working"),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Replace token" })).toHaveAttribute(
      "href",
      paths.sources,
    );
    const list = await coursesRegion();
    expect(within(card(list, "DEMO205")).getByText("Access expired")).toBeInTheDocument();
    expect(within(card(list, "DEMO101")).queryByText("Access expired")).toBeNull();
  });
});

describe("CoursesPage — empty and error", () => {
  it("shows the empty state with 'Add a source' when there are no sources", async () => {
    renderRoute("/courses", { scenario: "empty" });

    expect(await screen.findByText("No courses yet")).toBeInTheDocument();
    expect(
      screen.getByText(`Add a source so ${brand.productName} can find your courses.`),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Add a source" })).toHaveAttribute(
      "href",
      paths.welcome,
    );
    expect(screen.queryByRole("button", { name: "Sync now" })).toBeNull();
  });

  it("offers one 'Sync now' when sources exist but no courses came in", async () => {
    const api = mockApi();
    api.listCourses = vi.fn().mockResolvedValue([]);
    const syncAll = vi.spyOn(api, "syncAll");
    const { user } = renderRoute("/courses", { api });

    expect(await screen.findByText("No courses yet")).toBeInTheDocument();
    const buttons = await screen.findAllByRole("button", { name: "Sync now" });
    expect(buttons).toHaveLength(1);
    await user.click(buttons[0] as HTMLElement);
    expect(syncAll).toHaveBeenCalledTimes(1);
  });

  it("says the courses are on their way while a sync runs, not 'No courses yet'", async () => {
    const api = mockApi();
    api.listCourses = vi.fn().mockResolvedValue([]);
    renderRoute("/courses", { api });
    expect(await screen.findByText("No courses yet")).toBeInTheDocument();

    // E.g. the first sync, left running in the background from onboarding.
    act(() => useSyncStore.setState({ running: true }));
    expect(
      await screen.findByText("Your courses appear here when the sync finishes."),
    ).toBeInTheDocument();
    expect(screen.queryByText("No courses yet")).toBeNull();
    expect(screen.queryByRole("button", { name: "Sync now" })).toBeNull();

    act(() => useSyncStore.setState({ running: false }));
    expect(await screen.findByText("No courses yet")).toBeInTheDocument();
  });

  it("also waits for a sync another process (the CLI) is running", async () => {
    const api = mockApi({ scenario: "busy" });
    api.listCourses = vi.fn().mockResolvedValue([]);
    renderRoute("/courses", { api });
    expect(
      await screen.findByText("Your courses appear here when the sync finishes."),
    ).toBeInTheDocument();
  });

  it("shows an error with retry when courses fail to load", async () => {
    const api = mockApi();
    const real = api.listCourses;
    api.listCourses = vi
      .fn<PageLampApi["listCourses"]>()
      .mockRejectedValueOnce(new ApiError("internal", "Synthetic failure"))
      .mockImplementation(real);
    const { user } = renderRoute("/courses", { api });

    expect(await screen.findByText("Couldn't load your courses")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(await coursesRegion()).toBeInTheDocument();
  });
});

describe("CoursesPage — zh-CN", () => {
  it("renders the key headings in Chinese", async () => {
    useUiStore.setState({ locale: "zh-CN" });
    await i18n.changeLanguage("zh-CN");
    renderRoute("/courses");

    expect(await screen.findByRole("heading", { level: 1, name: "课程" })).toBeInTheDocument();
    expect(await screen.findByRole("heading", { level: 2, name: "本周" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 2, name: "我的学习计划" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 2, name: "我的课程" })).toBeInTheDocument();
    expect(await screen.findByText("未来 7 天有 3 个截止日期")).toBeInTheDocument();
    expect(await screen.findByRole("heading", { level: 3, name: /^今天/ })).toBeInTheDocument();
  });
});
