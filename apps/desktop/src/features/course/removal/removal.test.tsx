import { screen, waitFor, within } from "@testing-library/react";
import { toast } from "sonner";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createMockApi, type MockScenario } from "@/api/mock";
import { paths } from "@/lib/routes";
import { useUiStore } from "@/stores/ui";
import { renderRoute } from "@/test/render";
import { openCourse } from "../testing";

const ENDED = "canvas:canvas.demo.test/course/PHS150";
const INACTIVE = "canvas:canvas.demo.test/course/PHS190";

beforeEach(() => {
  // Sonner replays toasts to every new <Toaster/>; start each test without old ones.
  toast.dismiss();
});

function api(scenario: MockScenario) {
  return createMockApi({ latencyMs: 0, syncStepMs: 0, scenario });
}

async function openBanner(scenario: "phases" | "busy" = "phases") {
  const mock = api(scenario);
  const result = renderRoute(paths.courses, { api: mock });
  const banner = await screen.findByRole("region", { name: /courses? looks? finished$/ });
  return { ...result, banner };
}

describe("the 'courses look finished' banner", () => {
  it("reviews the suggested courses and removes them, with Undo", async () => {
    const { user, banner, api: mock } = await openBanner();
    expect(within(banner).getByText("2 courses look finished")).toBeInTheDocument();
    const removeCourses = vi.spyOn(mock, "removeCourses");

    await user.click(within(banner).getByRole("button", { name: "Review" }));
    const dialog = await screen.findByRole("dialog", { name: "Remove past courses" });
    const list = within(dialog).getByRole("group", { name: "Courses to remove" });
    expect(within(list).getByRole("checkbox", { name: /^PHS150/ })).toBeChecked();
    expect(within(list).getByRole("checkbox", { name: /^PHS190/ })).toBeChecked();
    // Each row explains itself with the lifecycle's evidence.
    expect(within(list).getByText(/^Ended · By the course dates, it ended /)).toBeInTheDocument();
    // What goes, what stays, and what a later sync can't bring back.
    expect(await within(dialog).findByText("What will be deleted")).toBeInTheDocument();
    expect(
      within(dialog).getByText(
        "The course name and code stay in Removed courses, so sync doesn't add it back.",
      ),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText("downloading files again counts as viewing them in Canvas"),
    ).toBeInTheDocument();
    // A recent pre-update backup is kept by default, with the reason.
    const backup = within(dialog).getByRole("checkbox", {
      name: "Also delete the pre-update backup",
    });
    expect(backup).not.toBeChecked();
    expect(
      within(dialog).getByText(/it's your way back if an update goes wrong/),
    ).toBeInTheDocument();
    // It goes with the purge, not with the removal.
    expect(backup).toHaveAccessibleDescription(/Undo keeps it\.$/);

    await user.click(within(dialog).getByRole("button", { name: "Remove 2 courses" }));
    expect(removeCourses).toHaveBeenCalledWith([ENDED, INACTIVE], {
      reason: null,
      keep_downloaded_files: false,
      purge_now: false,
      delete_pre_update_backup: false,
    });
    expect(await screen.findByText("2 courses removed")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect((await mock.listCourses()).some((c) => c.course.id === ENDED)).toBe(false);

    await user.click(screen.getByRole("button", { name: "Undo" }));
    expect(await screen.findByText("Removal undone")).toBeInTheDocument();
    expect((await mock.listCourses()).some((c) => c.course.id === ENDED)).toBe(true);
  });

  it("keeps a course out of the suggestions, and only removes what stays ticked", async () => {
    const { user, banner, api: mock } = await openBanner();
    const snooze = vi.spyOn(mock, "snoozeRemovalSuggestions");
    await user.click(within(banner).getByRole("button", { name: "Review" }));
    const dialog = await screen.findByRole("dialog");

    await user.click(
      within(dialog).getByRole("button", { name: "Keep PHS190 and stop suggesting it" }),
    );
    expect(snooze).toHaveBeenCalledWith([INACTIVE], "keep");
    expect(await screen.findByText("PHS190 won't be suggested again")).toBeInTheDocument();
    expect(within(dialog).queryByRole("checkbox", { name: /^PHS190/ })).toBeNull();

    await user.click(within(dialog).getByRole("checkbox", { name: /^PHS150/ }));
    expect(within(dialog).getByText("Tick at least one course to remove.")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Remove 0 courses" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });

  it("deletes now without an Undo when asked", async () => {
    const { user, banner, api: mock } = await openBanner();
    const removeCourses = vi.spyOn(mock, "removeCourses");
    const purge = vi.spyOn(mock, "purgeRemovedCourses");
    await user.click(within(banner).getByRole("button", { name: "Review" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(
      within(dialog).getByRole("checkbox", { name: "Delete now instead of in 7 days" }),
    );
    await user.click(within(dialog).getByRole("button", { name: "Remove 2 courses" }));
    expect(
      await screen.findByText("2 courses deleted. Restoring means syncing them again."),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
    // Deleting now is stage 2 through remove_courses: the Trash, never a permanent delete.
    expect(removeCourses.mock.calls[0]?.[1]).toMatchObject({ purge_now: true });
    expect(purge).not.toHaveBeenCalled();
  });

  it("says so when a sync is running", async () => {
    const { user, banner } = await openBanner("busy");
    await user.click(within(banner).getByRole("button", { name: "Review" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Remove 1 course" }));
    expect(
      await within(dialog).findByText("A sync is running. Try again when it finishes."),
    ).toBeInTheDocument();
  });

  it("goes away for two weeks with Not now, and focus moves to the heading", async () => {
    const { user, banner } = await openBanner();
    await user.click(within(banner).getByRole("button", { name: "Not now" }));
    expect(await screen.findByText("We'll ask again in two weeks.")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: /look finished$/ })).toBeNull(),
    );
    expect(screen.getByRole("heading", { level: 1 })).toHaveFocus();
  });
});

describe("Review past courses", () => {
  it("lists every past course with the suggested ones ticked", async () => {
    useUiStore.setState({ showPastCourses: true });
    const mock = api("phases");
    await mock.snoozeRemovalSuggestions([INACTIVE], "not_now");
    const { user } = renderRoute(paths.courses, { api: mock });
    await user.click(await screen.findByRole("button", { name: "Review past courses…" }));
    const dialog = await screen.findByRole("dialog", { name: "Remove past courses" });
    expect(within(dialog).getByRole("checkbox", { name: /^PHS150/ })).toBeChecked();
    expect(within(dialog).getByRole("checkbox", { name: /^PHS190/ })).not.toBeChecked();
  });
});

describe("removing one course from its settings", () => {
  it("explains hide vs remove, removes it and goes back to the course list", async () => {
    const {
      user,
      router,
      api: mock,
    } = await openCourse(ENDED, {
      query: "tab=settings",
      scenario: "phases",
    });
    const section = screen.getByRole("region", { name: "Remove from PageLamp" });
    expect(
      within(section).getByText(
        "Hide keeps the data and keeps syncing; your AI app doesn't see the course.",
      ),
    ).toBeInTheDocument();
    await user.click(within(section).getByRole("button", { name: "Remove from PageLamp…" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Remove PHS150 from PageLamp?",
    });
    await user.click(within(dialog).getByRole("button", { name: "Remove 1 course" }));
    expect(await screen.findByText("PHS150 removed")).toBeInTheDocument();
    await waitFor(() => expect(router.state.location.pathname).toBe(paths.courses));
    expect((await mock.removedCourses()).map((r) => r.course_id)).toEqual([ENDED]);
  });
});

describe("Removed courses in Settings", () => {
  async function openRemoved() {
    const mock = api("removed");
    const result = renderRoute(paths.settings, { api: mock });
    const list = await screen.findByRole("list", { name: "Removed courses" });
    const row = (code: string) => {
      const item = within(list)
        .getAllByRole("listitem")
        .find((li) => li.textContent?.startsWith(code));
      if (!item) throw new Error(`No row for ${code}`);
      return item;
    };
    return { ...result, list, row, api: mock };
  }

  it("lists pending and purged courses with the right actions", async () => {
    const { row } = await openRemoved();
    const pending = within(row("DEM101H5"));
    expect(pending.getByText(/its data is deleted in 3 days$/)).toBeInTheDocument();
    expect(pending.getByRole("button", { name: "Undo" })).toBeInTheDocument();
    expect(pending.getByRole("button", { name: "Delete now" })).toBeInTheDocument();

    const purged = within(row("DEM236H5"));
    expect(purged.getByText(/^Data deleted /)).toBeInTheDocument();
    expect(
      purged.getByText(
        "Its downloaded files couldn't be moved to the Trash. PageLamp tries again at each sync.",
      ),
    ).toBeInTheDocument();
    expect(purged.getByRole("button", { name: /^Try again: move DEM236H5/ })).toBeInTheDocument();
    expect(purged.getByRole("button", { name: "Delete files permanently" })).toBeInTheDocument();
    expect(purged.getByRole("button", { name: "Restore" })).toBeInTheDocument();
    // Forgetting waits until its files are in the Trash or deleted.
    expect(purged.queryByRole("button", { name: "Forget" })).toBeNull();
    expect(within(row("DEM150H5")).getByRole("button", { name: "Forget" })).toBeInTheDocument();
    // The data summary counts them.
    expect(screen.getAllByText("Removed courses").length).toBeGreaterThan(1);
  });

  it("offers Delete files permanently only after the Trash failed, and asks first", async () => {
    const { user, row, api: mock } = await openRemoved();
    const purge = vi.spyOn(mock, "purgeRemovedCourses");
    // Never by default: not while the data waits its 7 days, nor once the files are gone.
    expect(within(row("DEM101H5")).queryByRole("button", { name: /permanently/ })).toBeNull();
    expect(within(row("DEM150H5")).queryByRole("button", { name: /permanently/ })).toBeNull();

    const stuck = row("DEM236H5");
    await user.click(within(stuck).getByRole("button", { name: "Delete files permanently" }));
    let confirm = await screen.findByRole("alertdialog", { name: /files permanently\?$/ });
    expect(
      within(confirm).getByText(
        "They couldn't be moved to the Trash, so they would be deleted for good. You can't undo this.",
      ),
    ).toBeInTheDocument();
    await user.click(within(confirm).getByRole("button", { name: "Cancel" }));
    expect(purge).not.toHaveBeenCalled();

    await user.click(within(stuck).getByRole("button", { name: "Delete files permanently" }));
    confirm = await screen.findByRole("alertdialog", { name: /files permanently\?$/ });
    await user.click(within(confirm).getByRole("button", { name: "Delete files permanently" }));
    expect(await screen.findByText(/files were deleted permanently$/)).toBeInTheDocument();
    const removedId = (await mock.removedCourses()).find((r) =>
      r.code?.startsWith("DEM236"),
    )?.removed_id;
    expect(purge.mock.calls).toEqual([[[removedId], true]]);
    await waitFor(() =>
      expect(within(row("DEM236H5")).queryByText(/couldn't be moved to the Trash/)).toBeNull(),
    );
  });

  it("never deletes files for good from Delete now or Try again", async () => {
    const { user, row, api: mock } = await openRemoved();
    const purge = vi.spyOn(mock, "purgeRemovedCourses");

    // The mock's Trash still refuses these files: they stay waiting, and it says so.
    await user.click(
      within(row("DEM236H5")).getByRole("button", { name: /^Try again: move DEM236H5/ }),
    );
    expect(await screen.findByText(/^Still couldn't move DEM236H5/)).toBeInTheDocument();
    expect(within(row("DEM236H5")).getByText(/couldn't be moved to the Trash/)).toBeInTheDocument();

    await user.click(within(row("DEM101H5")).getByRole("button", { name: "Delete now" }));
    const confirm = await screen.findByRole("alertdialog", { name: /^Delete DEM101H5/ });
    await user.click(within(confirm).getByRole("button", { name: "Delete now" }));
    expect(await screen.findByText(/data was deleted$/)).toBeInTheDocument();

    expect(purge.mock.calls.map(([, permanent]) => permanent)).toEqual([false, false]);
  });

  it("offers Restore again for a restore that didn't finish", async () => {
    const { user, row, api: mock } = await openRemoved();
    const restore = vi.spyOn(mock, "restoreCourse");
    const stuck = within(row("PHS150"));
    expect(
      stuck.getByText(
        "The restore didn't finish. PageLamp sorts it out at the next sync, or you can restore it again.",
      ),
    ).toBeInTheDocument();
    expect(stuck.queryByRole("button", { name: "Forget" })).toBeNull();
    await user.click(stuck.getByRole("button", { name: "Restore again" }));
    expect(await screen.findByText("PHS150 is back in your courses")).toBeInTheDocument();
    expect(restore).toHaveBeenCalledWith("canvas:canvas.demo.test/course/PHS150");
  });

  it("says when the pre-update backup couldn't be deleted with the data", async () => {
    const { user, row, api: mock } = await openRemoved();
    vi.spyOn(mock, "purgeRemovedCourses").mockResolvedValue({
      purged: ["x"],
      files_pending: [],
      backup_deleted: false,
      backup_failed: true,
    });
    await user.click(within(row("DEM101H5")).getByRole("button", { name: "Delete now" }));
    const confirm = await screen.findByRole("alertdialog", { name: /^Delete DEM101H5/ });
    await user.click(within(confirm).getByRole("button", { name: "Delete now" }));
    expect(
      await screen.findByText(
        "The pre-update backup couldn't be deleted. It's still in the data folder.",
      ),
    ).toBeInTheDocument();
  });

  it("undoes a pending removal", async () => {
    const { user, row, api: mock } = await openRemoved();
    await user.click(within(row("DEM101H5")).getByRole("button", { name: "Undo" }));
    expect(
      await screen.findByText("DEM101H5 F LEC0101 20245 is back in your courses"),
    ).toBeInTheDocument();
    expect((await mock.listCourses()).some((c) => c.course.code?.startsWith("DEM101"))).toBe(true);
  });

  it("says why a purged course can't be restored", async () => {
    const { user, row } = await openRemoved();
    await user.click(within(row("DEM150H5")).getByRole("button", { name: "Restore" }));
    expect(
      await screen.findByText(
        "Couldn't restore DEM150H5 S LEC0101 20261: Canvas restricts access to it.",
      ),
    ).toBeInTheDocument();
  });

  it("forgets a purged course after asking", async () => {
    const { user, row, list } = await openRemoved();
    await user.click(within(row("DEM150H5")).getByRole("button", { name: "Forget" }));
    const confirm = await screen.findByRole("alertdialog", { name: /^Forget DEM150H5/ });
    expect(
      within(confirm).getByText(
        "PageLamp stops remembering it, so the next sync brings the course back.",
      ),
    ).toBeInTheDocument();
    await user.click(within(confirm).getByRole("button", { name: "Forget" }));
    expect(
      await screen.findByText(/was forgotten\. The next sync brings it back\.$/),
    ).toBeInTheDocument();
    await waitFor(() => expect(within(list).queryByText(/^DEM150H5/)).toBeNull());
  });
});
