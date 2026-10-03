import type { QueryClient } from "@tanstack/react-query";
import { act, configure, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockApi } from "@/api/mock";
import { brand } from "@/brand";
import i18n from "@/i18n";
import { useSyncStore } from "@/stores/sync";
import { useUiStore } from "@/stores/ui";
import { renderRoute } from "@/test/render";

// The mock adds a fixed 500 ms to token/feed validation; leave headroom on slow CI machines.
// (Per-file setting: Vitest isolates each test file.)
configure({ asyncUtilTimeout: 3000 });

// Synthetic secrets. The mock validates them and drops them.
const FEED_SECRET = "private-feed-4c8e21";
const FEED_URL = `https://calendar.demo.test/feeds/${FEED_SECRET}.ics`;
const CANVAS_TOKEN = "demo-canvas-token-9b72d0";

/** Everything the UI could have kept the secret in besides the input itself. */
async function expectSecretNotKept(secret: string, queryClient: QueryClient) {
  const stored: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const key = localStorage.key(i) ?? "";
    stored.push(`${key}=${localStorage.getItem(key)}`);
  }
  expect(stored.join("\n")).not.toContain(secret);
  expect(document.body.innerHTML).not.toContain(secret);
  expect(JSON.stringify(useUiStore.getState())).not.toContain(secret);
  expect(JSON.stringify(useSyncStore.getState())).not.toContain(secret);
  expect(
    JSON.stringify(
      queryClient
        .getQueryCache()
        .getAll()
        .map((q) => [q.queryKey, q.state.data]),
    ),
  ).not.toContain(secret);
  await waitFor(() =>
    expect(
      JSON.stringify(
        queryClient
          .getMutationCache()
          .getAll()
          .map((m) => m.state.variables),
      ),
    ).not.toContain(secret),
  );
}

async function goToSourceStep(user: ReturnType<typeof renderRoute>["user"]) {
  await user.click(await screen.findByRole("checkbox", { name: "I understand" }));
  await user.click(screen.getByRole("button", { name: "Get started" }));
  await screen.findByRole("heading", { level: 1, name: "Where are your courses?" });
}

describe("OnboardingPage", () => {
  it("welcomes the student with what the app does and the AI disclosure", async () => {
    renderRoute("/welcome", { scenario: "empty" });

    expect(
      await screen.findByRole("heading", { level: 1, name: `Welcome to ${brand.productName}` }),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    expect(screen.getByText("Step 1 of 3")).toBeInTheDocument();
    expect(screen.getByRole("listitem", { current: "step" })).toHaveTextContent("Welcome");
    expect(screen.getByText(/never solves or submits assignments/)).toBeInTheDocument();
    expect(
      screen.getByText(
        `When you ask your AI app about a course, it reads that course's materials from ${brand.productName} and sends them to your AI provider under your own account. ${brand.productName} itself stores nothing remotely. You're responsible for following each course's AI policy — and you can turn sharing off for any course.`,
      ),
    ).toBeInTheDocument();
    // Onboarding is full-window: no sidebar navigation.
    expect(screen.queryByRole("navigation")).not.toBeInTheDocument();
  });

  it("goes from folder + calendar feed to the first sync and the connect CTA", async () => {
    const { user, queryClient } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);

    const heading = screen.getByRole("heading", { level: 1, name: "Where are your courses?" });
    await waitFor(() => expect(heading).toHaveFocus());
    expect(screen.getByText("Step 2 of 3")).toBeInTheDocument();
    expect(screen.getByRole("listitem", { current: "step" })).toHaveTextContent("Add a source");
    expect(screen.getByRole("radio", { name: "Course folder + calendar feed" })).toBeChecked();
    expect(screen.getByText("Recommended · works with any LMS")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Choose folder…" }));
    expect(await screen.findByDisplayValue("/Users/demo/Documents/Courses")).toBeInTheDocument();
    await user.type(screen.getByLabelText("Term start (optional)"), "2026-09-08");
    await user.type(screen.getByLabelText("Calendar feed address (optional)"), FEED_URL);
    await user.click(screen.getByRole("button", { name: "Add and continue" }));

    expect(await screen.findByText("Step 3 of 3")).toBeInTheDocument();
    expect(
      await screen.findByRole("heading", { level: 1, name: "Your courses are ready" }),
    ).toBeInTheDocument();
    // Folder + feed: everything really is on this computer, so no Canvas download note.
    expect(screen.getByText(/Everything is on this computer now/)).toBeInTheDocument();
    expect(screen.queryByText(/Canvas files aren't downloaded/)).toBeNull();
    // One row per new source (the folder is named after its last path segment).
    const panel = screen.getByRole("region", { name: "Sync finished" });
    expect(within(panel).getByText("Courses")).toBeInTheDocument();
    expect(within(panel).getByText("Calendar feed")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { level: 2, name: `What ${brand.productName} found` }),
    ).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Connect your AI app" })).toHaveAttribute(
      "href",
      "/connect",
    );
    expect(screen.getByRole("link", { name: "Go to my courses" })).toHaveAttribute(
      "href",
      "/courses",
    );
    // Once the courses are in, the one AI question (no model is set up in the mock).
    expect(
      await screen.findByRole("region", {
        name: `Let ${brand.productName} write plans and explanations`,
      }),
    ).toBeInTheDocument();

    await expectSecretNotKept(FEED_SECRET, queryClient);
  });

  it("asks for a folder or a feed before submitting", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);
    await user.click(screen.getByRole("button", { name: "Add and continue" }));
    expect(
      screen.getByText("Choose a folder or paste a calendar feed address — or both."),
    ).toBeInTheDocument();
    expect(screen.getByText("Step 2 of 3")).toBeInTheDocument();
  });

  it("doesn't offer to leave while its sync is only queued behind another run", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty", syncStepMs: 60 });
    await goToSourceStep(user);
    await user.type(screen.getByLabelText("Folder path"), "/Users/demo/Courses");
    // Another run in this window is still going when the step starts ours.
    act(() => useSyncStore.setState({ running: true }));
    await user.click(screen.getByRole("button", { name: "Add and continue" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: "Syncing your courses" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Continue in the background" })).toBeNull();

    // Once that run ends, ours starts, and leaving is safe again.
    act(() => useSyncStore.setState({ running: false }));
    expect(
      await screen.findByRole("link", { name: "Continue in the background" }),
    ).toBeInTheDocument();
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false), { timeout: 5000 });
  });

  it("warns before connecting when PageLamp runs from the disk image", async () => {
    const api = createMockApi({ scenario: "empty", latencyMs: 0, syncStepMs: 0 });
    const real = await api.mcpClientConfigs();
    api.mcpClientConfigs = async () =>
      real.map((c) => ({
        ...c,
        launch: {
          ...c.launch,
          command: "/Volumes/PageLamp/PageLamp.app/Contents/MacOS/pagelamp",
          temporary_location: "disk_image" as const,
        },
        note_codes: ["run_from_temporary_location" as const, ...c.note_codes],
        notes: ["temporary", ...c.notes],
      }));
    const { user } = renderRoute("/welcome", { api });
    await goToSourceStep(user);
    await user.type(screen.getByLabelText("Folder path"), "/Users/demo/Courses");
    await user.click(screen.getByRole("button", { name: "Add and continue" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: "Your courses are ready" }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText("Move PageLamp to Applications before connecting"),
    ).toBeInTheDocument();
  });

  it("lets the student leave a long first sync running in the background", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty", syncStepMs: 60 });
    await goToSourceStep(user);
    await user.type(screen.getByLabelText("Folder path"), "/Users/demo/Courses");
    await user.click(screen.getByRole("button", { name: "Add and continue" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: "Syncing your courses" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Back" })).toBeDisabled();

    await user.click(screen.getByRole("link", { name: "Continue in the background" }));
    expect(await screen.findByRole("heading", { level: 1, name: "Courses" })).toBeInTheDocument();
    expect(useSyncStore.getState().running).toBe(true);
    // It finishes on its own, and the courses it found show up.
    await waitFor(() => expect(useSyncStore.getState().running).toBe(false), { timeout: 5000 });
    expect((await screen.findAllByText("DEMO101")).length).toBeGreaterThan(0);
  });

  it("names a new feed in the app's language, not the backend's English default", async () => {
    const api = createMockApi({ scenario: "empty", latencyMs: 0, syncStepMs: 0 });
    const addFeed = vi.spyOn(api, "addIcalSource");
    const { user } = renderRoute("/welcome", { api });
    await goToSourceStep(user);
    await user.type(screen.getByLabelText("Folder path"), "/Users/demo/Courses");
    await user.type(screen.getByLabelText("Calendar feed address (optional)"), FEED_URL);
    await user.click(screen.getByRole("button", { name: "Add and continue" }));
    await screen.findByText("Step 3 of 3");
    expect(addFeed).toHaveBeenCalledWith(FEED_URL, i18n.t("sourceKind.ical"));
    expect(i18n.t("sourceKind.ical", { lng: "zh-CN" })).toBe("日历订阅");
  });

  it("shows per-item errors and doesn't add the working part twice", async () => {
    const { user, api } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);

    await user.type(screen.getByLabelText("Folder path"), "/Users/demo/missing-folder");
    await user.type(screen.getByLabelText("Calendar feed address (optional)"), FEED_URL);
    await user.click(screen.getByRole("button", { name: "Add and continue" }));

    expect(
      await screen.findByText("We couldn't find that folder. Choose it again or check the path."),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Folder /Users/demo/missing-folder was not found."),
    ).toBeInTheDocument();
    expect(await screen.findByText("Calendar feed added")).toBeInTheDocument();
    expect(screen.getByLabelText("Folder path")).toHaveAttribute("aria-invalid", "true");

    await user.clear(screen.getByLabelText("Folder path"));
    await user.type(screen.getByLabelText("Folder path"), "/Users/demo/Courses");
    await user.click(screen.getByRole("button", { name: "Add and continue" }));

    expect(await screen.findByText("Step 3 of 3")).toBeInTheDocument();
    const sources = await api.listSources();
    expect(sources.map((s) => s.kind).sort()).toEqual(["folder", "ical"]);
  });

  it("explains an unreachable calendar feed", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);
    await user.type(
      screen.getByLabelText("Calendar feed address (optional)"),
      "https://offline.demo.test/feed.ics",
    );
    await user.click(screen.getByRole("button", { name: "Add and continue" }));
    expect(
      await screen.findByText(
        "Couldn't reach that address. Check your internet connection and try again.",
      ),
    ).toBeInTheDocument();
  });

  it("shows the Canvas notice verbatim and explains a rejected token", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);
    await user.click(screen.getByRole("radio", { name: "Canvas access token" }));

    expect(
      screen.getByText(
        "Personal access tokens are for your own use only, and they expire — Canvas shows the maximum when you create one (often 30–90 days).",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Personal use")).toBeInTheDocument();
    expect(
      screen.getByText(
        /^In Canvas: Account → Settings → Approved Integrations → \+ New Access Token\./,
      ),
    ).toBeInTheDocument();

    await user.type(screen.getByLabelText("Canvas address"), "https://canvas.demo.test");
    await user.type(screen.getByLabelText("Access token"), "demo-token-expired-0001");
    await user.click(screen.getByRole("button", { name: "Add and continue" }));

    // While Canvas checks the token (the mock takes 500 ms), the button says so and is busy.
    const checking = screen.getByRole("button", { name: "Checking your token…" });
    expect(checking).toBeDisabled();
    expect(checking).toHaveAttribute("aria-busy", "true");

    expect(
      await screen.findByText(
        "Canvas didn't accept this token. It may have expired or been revoked — create a new one and try again.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Canvas rejected this token. It may have expired or been revoked."),
    ).toBeInTheDocument();
    // Focus goes back to the field to fix, not to <body>.
    expect(screen.getByLabelText("Access token")).toHaveFocus();
    expect(screen.getByText("Step 2 of 3")).toBeInTheDocument();
  });

  it("explains an unreachable Canvas address", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);
    await user.click(screen.getByRole("radio", { name: "Canvas access token" }));
    await user.type(screen.getByLabelText("Canvas address"), "https://offline.demo.test");
    await user.type(screen.getByLabelText("Access token"), CANVAS_TOKEN);
    await user.click(screen.getByRole("button", { name: "Add and continue" }));
    expect(
      await screen.findByText(
        "Couldn't reach that Canvas address. Check the address and your internet connection.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("Step 2 of 3")).toBeInTheDocument();
  });

  it("rejects a pasted course link and returns to the address", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);
    await user.click(screen.getByRole("radio", { name: "Canvas access token" }));
    await user.type(screen.getByLabelText("Canvas address"), "canvas.demo.test/courses/1");
    await user.type(screen.getByLabelText("Access token"), CANVAS_TOKEN);
    await user.click(screen.getByRole("button", { name: "Add and continue" }));
    expect(
      await screen.findByText(
        "That is a link to a page, not a Canvas address. Enter just the address, like https://canvas.demo.test",
      ),
    ).toBeInTheDocument();
    // The message names the host, never the whole pasted link.
    expect(screen.queryByText(/courses\/1/)).toBeNull();
    expect(screen.getByLabelText("Canvas address")).toHaveFocus();
    expect(screen.getByText("Step 2 of 3")).toBeInTheDocument();
  });

  it("requires both Canvas fields", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);
    await user.click(screen.getByRole("radio", { name: "Canvas access token" }));
    await user.click(screen.getByRole("button", { name: "Add and continue" }));
    expect(screen.getByText("Enter your Canvas address.")).toBeInTheDocument();
    expect(screen.getByText("Paste your access token.")).toBeInTheDocument();
  });

  it("adds Canvas with a valid token, then syncs without keeping the token", async () => {
    const { user, queryClient } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);
    await user.click(screen.getByRole("radio", { name: "Canvas access token" }));
    // No scheme typed: the form adds https:// for the student.
    await user.type(screen.getByLabelText("Canvas address"), "canvas.demo.test");
    await user.type(screen.getByLabelText("Access token"), CANVAS_TOKEN);
    await user.click(screen.getByRole("button", { name: "Add and continue" }));

    expect(
      await screen.findByRole("heading", { level: 1, name: "Your courses are ready" }),
    ).toBeInTheDocument();
    expect(screen.getByText("canvas.demo.test")).toBeInTheDocument();
    // Canvas files aren't downloaded by a sync: don't claim everything is on this computer.
    expect(screen.queryByText(/Everything is on this computer now/)).toBeNull();
    expect(screen.getByText(/Canvas files aren't downloaded automatically/)).toHaveTextContent(
      "“Download files…” at the top of its page",
    );
    await expectSecretNotKept(CANVAS_TOKEN, queryClient);
  });

  it("warns about an explicit http:// Canvas address instead of rewriting it", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);
    await user.click(screen.getByRole("radio", { name: "Canvas access token" }));
    const address = screen.getByLabelText("Canvas address");
    await user.type(address, "http://canvas.demo.test");
    expect(address).toHaveValue("http://canvas.demo.test");
    expect(address).toHaveAccessibleDescription(/Canvas addresses use https/);
    await user.clear(address);
    await user.type(address, "https://canvas.demo.test");
    expect(screen.queryByText(/Canvas addresses use https/)).not.toBeInTheDocument();
  });

  it("offers a retry when another process is already syncing", async () => {
    const { user } = renderRoute("/welcome", { scenario: "busy" });
    await goToSourceStep(user);
    await user.click(screen.getByRole("button", { name: "Choose folder…" }));
    await screen.findByDisplayValue("/Users/demo/Documents/Courses");
    await user.click(screen.getByRole("button", { name: "Add and continue" }));

    expect(
      await screen.findByRole("heading", { level: 1, name: "The sync didn't run" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "A sync is already running (maybe from the command line). Try again when it finishes.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Go to my courses" })).toBeInTheDocument();
  });

  it("moves back between steps", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty" });
    await goToSourceStep(user);
    await user.click(screen.getByRole("button", { name: "Back" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: `Welcome to ${brand.productName}` }),
    ).toBeInTheDocument();
    expect(screen.getByText("Step 1 of 3")).toBeInTheDocument();
  });

  it("asks the student to acknowledge the AI disclosure before continuing", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty" });
    const start = await screen.findByRole("button", { name: "Get started" });
    expect(start).toHaveAttribute("aria-disabled", "true");

    await user.click(start);
    expect(screen.getByRole("alert")).toHaveTextContent("Tick “I understand” to continue.");
    expect(start).toHaveFocus(); // still focusable: the hint is read, focus isn't lost
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Welcome to");

    await user.click(screen.getByRole("checkbox", { name: "I understand" }));
    expect(useUiStore.getState().aiDisclosureAcknowledgedAt).not.toBeNull();
    expect(localStorage.getItem("pagelamp.ui")).toContain("aiDisclosureAcknowledgedAt");
    expect(screen.getByText(/^You confirmed this on /)).toBeInTheDocument();

    await user.click(start);
    expect(
      await screen.findByRole("heading", { level: 1, name: "Where are your courses?" }),
    ).toBeInTheDocument();
  });

  it("remembers an earlier acknowledgement", async () => {
    useUiStore.setState({ aiDisclosureAcknowledgedAt: "2026-09-01T12:00:00.000Z" });
    renderRoute("/welcome", { scenario: "empty" });
    expect(await screen.findByRole("checkbox", { name: "I understand" })).toBeChecked();
    expect(screen.getByRole("button", { name: "Get started" })).not.toHaveAttribute(
      "aria-disabled",
      "true",
    );
    expect(screen.getByText("You confirmed this on Sep 1, 2026.")).toBeInTheDocument();
  });

  it("'Skip for now' remembers the choice and opens the courses", async () => {
    const { user, router } = renderRoute("/welcome", { scenario: "empty" });
    await user.click(await screen.findByRole("button", { name: "Skip for now" }));
    expect(useUiStore.getState().onboardingSkipped).toBe(true);
    await waitFor(() => expect(router.state.location.pathname).toBe("/courses"));
  });

  it("switches the language from the welcome screen", async () => {
    const { user } = renderRoute("/welcome", { scenario: "empty" });
    await user.click(await screen.findByRole("button", { name: "简体中文" }));
    expect(
      await screen.findByRole("heading", { level: 1, name: `欢迎使用 ${brand.productName}` }),
    ).toBeInTheDocument();
    expect(useUiStore.getState().locale).toBe("zh-CN");
  });
});
