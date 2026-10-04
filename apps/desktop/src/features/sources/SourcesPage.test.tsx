import { configure, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ApiError } from "@/api/errors";
import { createMockApi } from "@/api/mock";
import type { SyncEvent } from "@/api/types";
import i18n from "@/i18n";
import { useUiStore } from "@/stores/ui";
import { renderRoute } from "@/test/render";

// The mock adds a fixed 500 ms to token/feed validation; leave headroom on slow CI machines.
// (Per-file setting: Vitest isolates each test file.)
configure({ asyncUtilTimeout: 3000 });

// Synthetic values only. The mock accepts tokens of 8+ characters without "expired".
const NEW_TOKEN = "demo-replacement-token-7f3a91";

function storageDump(): string {
  const entries: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const key = localStorage.key(i) ?? "";
    entries.push(`${key}=${localStorage.getItem(key)}`);
  }
  return entries.join("\n");
}

describe("SourcesPage", () => {
  it("lists the demo sources with their kind, settings and status", async () => {
    renderRoute("/sources");

    expect(
      await screen.findByRole("heading", { level: 1, name: "Sources & sync" }),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    await waitFor(() =>
      expect(screen.getAllByRole("heading", { level: 2 }).map((h) => h.textContent)).toEqual([
        "Course folder",
        "Course calendar",
        "Demo Canvas",
      ]),
    );
    expect(screen.getByText("/Users/demo/Documents/Courses")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "https://canvas.demo.test" })).toBeInTheDocument();
    expect(screen.getByText("Private feed address (stored in your keychain)")).toBeInTheDocument();
    expect(screen.getAllByText("OK")).toHaveLength(3);
    expect(
      screen.getByRole("button", { name: "Replace token for Demo Canvas" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Replace feed address for Course calendar" }),
    ).toBeInTheDocument();
    // Folder sources have no secret to replace.
    expect(screen.queryByRole("button", { name: /Replace .* for Course folder/ })).toBeNull();
    expect(screen.getByText(/keeps your courses on this computer/)).toBeInTheDocument();
  });

  it("says which Canvas account a source is connected as", async () => {
    renderRoute("/sources");
    const canvas = (await screen.findByRole("heading", { level: 2, name: "Demo Canvas" })).closest(
      "[data-slot=card]",
    );
    if (!(canvas instanceof HTMLElement)) throw new Error("no Canvas card");
    expect(within(canvas).getByText("Canvas · Connected as Demo Student")).toBeInTheDocument();
    // Other kinds just name the kind.
    const folder = screen
      .getByRole("heading", { level: 2, name: "Course folder" })
      .closest("[data-slot=card]");
    if (!(folder instanceof HTMLElement)) throw new Error("no folder card");
    expect(within(folder).queryByText(/Connected as/)).toBeNull();
  });

  it("leaves out the account for Canvas sources added before it was recorded", async () => {
    const api = createMockApi({ latencyMs: 0, syncStepMs: 0 });
    const sources = await api.listSources();
    api.listSources = async () =>
      sources.map((s) =>
        s.kind === "canvas" ? { ...s, config: { base_url: s.config.base_url } } : s,
      );
    renderRoute("/sources", { api });
    const canvas = (await screen.findByRole("heading", { level: 2, name: "Demo Canvas" })).closest(
      "[data-slot=card]",
    );
    if (!(canvas instanceof HTMLElement)) throw new Error("no Canvas card");
    expect(within(canvas).getByText("Canvas")).toBeInTheDocument();
    expect(within(canvas).queryByText(/Connected as/)).toBeNull();
  });

  it("shows a skeleton while loading", async () => {
    renderRoute("/sources", { latencyMs: 20 });
    expect(screen.getByText("Loading your sources…")).toBeInTheDocument();
    expect(
      await screen.findByRole("heading", { level: 2, name: "Demo Canvas" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Loading your sources…")).not.toBeInTheDocument();
  });

  it("shows an error with a retry when the sources can't be loaded", async () => {
    const api = createMockApi({ latencyMs: 0, syncStepMs: 0 });
    const listSources = api.listSources;
    let fail = true;
    api.listSources = () =>
      fail ? Promise.reject(new ApiError("internal", "Database is locked")) : listSources();
    const { user } = renderRoute("/sources", { api });

    expect(await screen.findByText("Something went wrong")).toBeInTheDocument();
    expect(screen.getByText("Database is locked")).toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(
      await screen.findByRole("heading", { level: 2, name: "Demo Canvas" }),
    ).toBeInTheDocument();
  });

  it("shows the token-expired callout, and replacing the token clears the error", async () => {
    const { user, queryClient } = renderRoute("/sources", { scenario: "expired" });

    // The callout title and the card's status badge.
    expect(await screen.findAllByText("Access expired")).toHaveLength(2);
    expect(
      screen.getByText(
        "Your Canvas token has expired or was revoked. Create a new one in Canvas (Account → Settings → Approved Integrations → + New Access Token) and replace it here.",
      ),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Replace token for Demo Canvas" }));
    const dialog = await screen.findByRole("dialog", { name: "Replace token" });
    expect(
      within(dialog).getByText(
        "Personal access tokens are for your own use only, and they expire — Canvas shows the maximum when you create one (often 30–90 days).",
      ),
    ).toBeInTheDocument();
    await user.type(within(dialog).getByLabelText("New access token"), NEW_TOKEN);
    await user.click(within(dialog).getByRole("button", { name: "Replace" }));

    expect(await screen.findByText("Token replaced for Demo Canvas")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await waitFor(() => expect(screen.queryByText("Access expired")).not.toBeInTheDocument());
    // The button that opened the dialog went away with the callout: focus lands on the h1.
    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 1, name: "Sources & sync" })).toHaveFocus(),
    );

    // The token was handed to the backend and kept nowhere else.
    expect(document.body.innerHTML).not.toContain(NEW_TOKEN);
    expect(storageDump()).not.toContain(NEW_TOKEN);
    await waitFor(() =>
      expect(
        JSON.stringify(
          queryClient
            .getMutationCache()
            .getAll()
            .map((m) => m.state.variables),
        ),
      ).not.toContain(NEW_TOKEN),
    );
  });

  it("says plainly that a sync is already running when 'Sync now' can't start one", async () => {
    const api = createMockApi({ latencyMs: 0, syncStepMs: 0, scenario: "expired" });
    const realSyncAll = api.syncAll;
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    api.syncAll = async (req, onEvent: (event: SyncEvent) => void) => {
      await gate;
      return realSyncAll(req, onEvent);
    };
    const { user } = renderRoute("/sources", { api });
    // This window's own sync is in the way (the student's here; it could be an automatic one).
    await user.click(await screen.findByRole("button", { name: "Sync all" }));

    await user.click(screen.getByRole("button", { name: "Replace token for Demo Canvas" }));
    const dialog = await screen.findByRole("dialog", { name: "Replace token" });
    await user.type(within(dialog).getByLabelText("New access token"), NEW_TOKEN);
    await user.click(within(dialog).getByRole("button", { name: "Replace" }));
    await user.click(await screen.findByRole("button", { name: "Sync now" }));

    // Not "(maybe from the command line)": it is this window's.
    expect(
      await screen.findByText("A sync is already running. Try again when it finishes."),
    ).toBeInTheDocument();
    release();
  });

  it("keeps the replace dialog open and explains a rejected token", async () => {
    const { user } = renderRoute("/sources", { scenario: "expired" });
    await user.click(await screen.findByRole("button", { name: "Replace token for Demo Canvas" }));
    const dialog = await screen.findByRole("dialog", { name: "Replace token" });

    await user.click(within(dialog).getByRole("button", { name: "Replace" }));
    expect(within(dialog).getByText("Paste the new token first.")).toBeInTheDocument();

    await user.type(within(dialog).getByLabelText("New access token"), "demo-token-expired-01");
    await user.click(within(dialog).getByRole("button", { name: "Replace" }));
    expect(
      await within(dialog).findByText(
        "Canvas didn't accept this token. It may have expired or been revoked — create a new one and try again.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("describes other source errors by kind, with the backend's text under Technical details", async () => {
    const { user } = renderRoute("/sources", { scenario: "error" });
    expect(await screen.findByText("The last sync didn't work")).toBeInTheDocument();
    expect(screen.getByText("Not found")).toBeInTheDocument();
    const raw = "Folder /Users/demo/Documents/Courses was not found.";
    expect(screen.queryByText(raw)).not.toBeInTheDocument();

    const details = screen.getByRole("button", { name: "Technical details for Course folder" });
    expect(details).toHaveTextContent("Technical details");
    await user.click(details);
    expect(details).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText(raw).closest("[lang]")).toHaveAttribute("lang", "en");
    await user.click(
      screen.getByRole("button", { name: "Copy technical details for Course folder" }),
    );
    expect(await navigator.clipboard.readText()).toBe(raw);
  });

  it("keeps the backend's text behind Technical details in Chinese too", async () => {
    useUiStore.setState({ locale: "zh-CN" });
    await i18n.changeLanguage("zh-CN");
    const { user } = renderRoute("/sources", { scenario: "error" });
    expect(await screen.findByText("上次同步没有成功")).toBeInTheDocument();
    const raw = "Folder /Users/demo/Documents/Courses was not found.";
    expect(screen.queryByText(raw)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "技术详情：Course folder" }));
    expect(screen.getByText(raw)).toBeInTheDocument();
  });

  it("wraps a long source label so the status badge stays on the card", async () => {
    const label =
      "University of Example — Canvas (Faculty of Arts & Science, St. George campus, all sections)";
    const api = createMockApi({ latencyMs: 0, syncStepMs: 0 });
    await api.addFolderSource("/Users/demo/Documents/Long", null, label);
    // A source that has never synced makes an automatic sync due; this is about its "New" badge.
    await api.setSyncPrefs({ auto_sync: "off" });
    renderRoute("/sources", { api });

    const heading = await screen.findByRole("heading", { level: 2, name: label });
    // jsdom has no layout, so pin what does it: the title (the header grid's first column) may
    // shrink below the label's width, and the label wraps at any character instead of running on.
    expect(heading.closest('[data-slot="card-title"]')).toHaveClass("min-w-0");
    expect(heading).toHaveClass("min-w-0", "[overflow-wrap:anywhere]");
    expect(heading).not.toHaveClass("truncate");
    // The badge is in the same header, in the action column.
    const header = heading.closest('[data-slot="card-header"]') as HTMLElement;
    const action = header.querySelector('[data-slot="card-action"]') as HTMLElement;
    expect(within(action).getByText("New")).toBeInTheDocument();
  });

  it("removes a source after confirmation", async () => {
    const { user } = renderRoute("/sources");

    await user.click(await screen.findByRole("button", { name: "Remove Course calendar" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Remove Course calendar?" });
    // A feed only brings deadlines and events; it never creates courses.
    expect(within(dialog).getByText(/the deadlines and events synced from it/)).toBeInTheDocument();
    expect(within(dialog).getByText(/its stored feed address/)).toBeInTheDocument();
    expect(within(dialog).getByText(/Your courses aren't affected/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Remove source" }));

    expect(await screen.findByText("Removed Course calendar")).toBeInTheDocument();
    await waitFor(() =>
      expect(
        screen.queryByRole("heading", { level: 2, name: "Course calendar" }),
      ).not.toBeInTheDocument(),
    );
    expect(screen.getAllByRole("heading", { level: 2 })).toHaveLength(2);
    // The Remove button went away with its card, so focus moves to the page heading.
    await waitFor(() =>
      expect(screen.getByRole("heading", { level: 1, name: "Sources & sync" })).toHaveFocus(),
    );
  });

  it("says removing Canvas also deletes the downloaded course files", async () => {
    const { user } = renderRoute("/sources");
    await user.click(await screen.findByRole("button", { name: "Remove Demo Canvas" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Remove Demo Canvas?" });
    expect(within(dialog).getByText(/the course files you downloaded/)).toBeInTheDocument();
    expect(within(dialog).getByText(/Nothing in Canvas changes/)).toBeInTheDocument();
  });

  it("explains that removing a folder source leaves the files alone", async () => {
    const { user } = renderRoute("/sources");
    await user.click(await screen.findByRole("button", { name: "Remove Course folder" }));
    const dialog = await screen.findByRole("alertdialog", { name: "Remove Course folder?" });
    expect(within(dialog).getByText(/The files in your folder aren't touched/)).toBeInTheDocument();
    // A folder source has no token or feed address to delete.
    expect(within(dialog).queryByText(/token|feed address/)).toBeNull();
  });

  it("cancelling the remove dialog keeps the source", async () => {
    const { user } = renderRoute("/sources");
    await user.click(await screen.findByRole("button", { name: "Remove Demo Canvas" }));
    const dialog = await screen.findByRole("alertdialog");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(screen.getByRole("heading", { level: 2, name: "Demo Canvas" })).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Remove Demo Canvas" })).toHaveFocus(),
    );
  });

  it("replaces a calendar feed address and keeps it nowhere else", async () => {
    const feedUrl = "https://calendar.demo.test/feeds/private-replacement-5d19.ics";
    const { user } = renderRoute("/sources");
    const opener = await screen.findByRole("button", {
      name: "Replace feed address for Course calendar",
    });
    await user.click(opener);
    const dialog = await screen.findByRole("dialog", { name: "Replace feed address" });
    // The Canvas personal-use notice belongs to Canvas tokens only.
    expect(within(dialog).queryByText(/Personal access tokens/)).toBeNull();

    await user.type(within(dialog).getByLabelText("New calendar feed address"), feedUrl);
    await user.click(within(dialog).getByRole("button", { name: "Replace" }));

    expect(
      await screen.findByText("Feed address replaced for Course calendar"),
    ).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    // Opened from the card footer, which is still there: focus goes back to that button.
    await waitFor(() => expect(opener).toHaveFocus());
    expect(document.body.innerHTML).not.toContain("private-replacement-5d19");
    expect(storageDump()).not.toContain("private-replacement-5d19");
  });

  it("skips the disclosure in the dialog once it was acknowledged", async () => {
    useUiStore.setState({ aiDisclosureAcknowledgedAt: "2026-09-01T12:00:00.000Z" });
    const { user } = renderRoute("/sources");
    await user.click(await screen.findByRole("button", { name: "Add source" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a source" });
    expect(
      within(dialog).queryByRole("checkbox", { name: "I understand" }),
    ).not.toBeInTheDocument();
    expect(
      within(dialog).getByRole("radio", { name: "Course folder + calendar feed" }),
    ).toBeChecked();
  });

  it("returns focus to Add source when the dialog is cancelled", async () => {
    const { user } = renderRoute("/sources");
    const add = await screen.findByRole("button", { name: "Add source" });
    await user.click(add);
    const dialog = await screen.findByRole("dialog", { name: "Add a source" });
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await waitFor(() => expect(add).toHaveFocus());
  });

  it("shows the empty state and adds a first source from the dialog", async () => {
    const { user } = renderRoute("/sources", { scenario: "empty" });

    expect(await screen.findByText("No sources yet")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sync all" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    await user.click(screen.getByRole("button", { name: "Add your first source" }));

    const dialog = await screen.findByRole("dialog", { name: "Add a source" });
    // Onboarding was skipped, so the AI disclosure comes first and gates the forms.
    expect(within(dialog).getByText(/sends them to your AI provider/)).toBeInTheDocument();
    expect(within(dialog).queryByRole("radio")).not.toBeInTheDocument();
    await user.click(within(dialog).getByRole("checkbox", { name: "I understand" }));
    expect(
      within(dialog).getByRole("radio", { name: "Course folder + calendar feed" }),
    ).toBeChecked();
    await user.click(within(dialog).getByRole("button", { name: "Choose folder…" }));
    expect(
      await within(dialog).findByDisplayValue("/Users/demo/Documents/Courses"),
    ).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Add source" }));

    expect(await screen.findByText("Source added. Syncing now…")).toBeInTheDocument();
    expect(await screen.findByRole("heading", { level: 2, name: "Courses" })).toBeInTheDocument();
    expect(
      await screen.findByRole("heading", { level: 2, name: "Sync finished" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("No sources yet")).not.toBeInTheDocument();
  });

  it("shows live progress while syncing everything, then the result", async () => {
    const api = createMockApi({ latencyMs: 0, syncStepMs: 0 });
    const realSyncAll = api.syncAll;
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    // Emit the first events, then hold the run until the test has looked at it.
    api.syncAll = async (req, onEvent: (event: SyncEvent) => void) => {
      onEvent({ type: "source_started", source_id: "folder:demo-courses", label: "Course folder" });
      onEvent({
        type: "progress",
        source_id: "folder:demo-courses",
        message: "DEMO101: indexing files",
        current: 3,
        total: 12,
      });
      await gate;
      return realSyncAll(req, onEvent);
    };
    const { user } = renderRoute("/sources", { api });

    const syncAll = await screen.findByRole("button", { name: "Sync all" });
    await screen.findByRole("heading", { level: 2, name: "Demo Canvas" });
    await user.click(syncAll);

    expect(await screen.findByRole("heading", { level: 2, name: "Syncing…" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sync all" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    const bar = screen.getByRole("progressbar", { name: "Course folder progress" });
    expect(bar).toHaveAttribute("aria-valuenow", "25");
    expect(screen.getByText("DEMO101: indexing files")).toBeInTheDocument();
    expect(screen.getByText("Syncing")).toBeInTheDocument(); // the folder card's badge
    // No removing while a sync runs, even for a source that hasn't started yet.
    expect(screen.getByRole("button", { name: "Remove Demo Canvas" })).toBeDisabled();

    release();
    expect(
      await screen.findByRole("heading", { level: 2, name: "Sync finished" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sync all" })).toBeEnabled();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();

    // The folder source reported a skipped file: the count shows, the text is collapsed until
    // asked for.
    expect(screen.getByText("1 warning")).toBeInTheDocument();
    expect(screen.queryByText(/video files can't be read/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Technical details for Course folder" }));
    expect(await screen.findByText(/video files can't be read/)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Hide sync results" }));
    expect(screen.queryByRole("heading", { name: "Sync finished" })).not.toBeInTheDocument();
  });

  it("translates the step from its stage code, with the course as plain text", async () => {
    useUiStore.setState({ locale: "zh-CN" });
    await i18n.changeLanguage("zh-CN");
    const api = createMockApi({ latencyMs: 0, syncStepMs: 0 });
    const realSyncAll = api.syncAll;
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    api.syncAll = async (req, onEvent: (event: SyncEvent) => void) => {
      const source_id = "folder:demo-courses";
      onEvent({ type: "source_started", source_id, label: "Course folder" });
      onEvent({
        type: "progress",
        source_id,
        stage: "indexing_files",
        course: "DEMO{{x}}101",
        message: "DEMO{{x}}101: indexing files",
        current: 3,
        total: 12,
      });
      onEvent({
        type: "source_started",
        source_id: "ical:demo-calendar",
        label: "Course calendar",
      });
      // An older facade: English text and no stage, so a Chinese UI leaves it out.
      onEvent({
        type: "progress",
        source_id: "ical:demo-calendar",
        message: "Saving 12 calendar events",
      });
      await gate;
      return realSyncAll(req, onEvent);
    };
    const { user } = renderRoute("/sources", { api });

    await user.click(await screen.findByRole("button", { name: "全部同步" }));
    const row = (await screen.findByText("正在同步… · 3/12")).closest("li") as HTMLElement;
    expect(within(row).getByText("Course folder")).toBeInTheDocument();
    expect(within(row).getByText("正在为 DEMO{{x}}101 的文件建立索引")).toBeInTheDocument();
    expect(screen.queryByText("Saving 12 calendar events")).toBeNull();
    release();
    expect(await screen.findByRole("heading", { level: 2, name: "同步完成" })).toBeInTheDocument();
  });

  it("syncs a single source", async () => {
    const { user } = renderRoute("/sources", { scenario: "expired" });
    await user.click(await screen.findByRole("button", { name: "Sync Course calendar" }));
    expect(
      await screen.findByRole("heading", { level: 2, name: "Sync finished" }),
    ).toBeInTheDocument();
    const panel = screen.getByRole("region", { name: "Sync finished" });
    expect(within(panel).getByText("Course calendar")).toBeInTheDocument();
    expect(within(panel).queryByText("Demo Canvas")).not.toBeInTheDocument();
  });

  it("reports an expired token found during a sync", async () => {
    const { user } = renderRoute("/sources", { scenario: "expired" });
    await user.click(await screen.findByRole("button", { name: "Sync Demo Canvas" }));
    expect(
      await screen.findByRole("heading", { level: 2, name: "Sync finished with problems" }),
    ).toBeInTheDocument();
    const panel = screen.getByRole("region", { name: "Sync finished with problems" });
    expect(within(panel).getByText("Access expired")).toBeInTheDocument();
    expect(within(panel).getByText(/Access expired. Replace the token/)).toBeInTheDocument();
  });

  it("explains when another process is already syncing", async () => {
    const { user } = renderRoute("/sources", { scenario: "busy" });

    expect(await screen.findByText("Another sync is running")).toBeInTheDocument();
    expect(
      screen.getByText(
        "A sync is already running (maybe from the command line). Try again when it finishes.",
      ),
    ).toBeInTheDocument();

    // Syncing can't start while another process holds the lock: the button stays focusable
    // (aria-disabled) and clicking it starts nothing.
    const syncAll = screen.getByRole("button", { name: "Sync all" });
    expect(syncAll).toHaveAttribute("aria-disabled", "true");
    await user.click(syncAll);
    expect(syncAll).toHaveFocus();
    expect(
      screen.queryByRole("heading", { level: 2, name: "Sync failed" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("Another sync is running")).toBeInTheDocument();
    // Removing a source the other process may be syncing isn't offered either.
    expect(screen.getByRole("button", { name: "Remove Demo Canvas" })).toBeDisabled();
  });
});
