import { screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/api/errors";
import { createMockApi } from "@/api/mock";
import { MOCK_DEVICE_CODE } from "@/api/mock/codex";
import { DEMO101 } from "@/features/course/testing";
import { renderRoute, renderWithProviders } from "@/test/render";
import { GenerateButton } from "../GenerateButton";

const TITLE = "Use my ChatGPT plan (runs OpenAI Codex)";

async function card() {
  const heading = await screen.findByRole("heading", { level: 3, name: TITLE });
  const region = heading.closest("section");
  if (!region) throw new Error("no ChatGPT card");
  return region;
}

afterEach(() => vi.restoreAllMocks());

describe("Use my ChatGPT plan (Codex)", () => {
  it("says what the download is, installs it and offers to sign in", async () => {
    const { user } = renderRoute("/settings", { scenario: "codex-not-installed" });
    const region = await card();
    expect(
      await within(region).findByText(/≈70–80 MB, taking up to ≈330 MB once installed/),
    ).toBeInTheDocument();
    await user.click(within(region).getByRole("button", { name: "Download Codex" }));
    expect(await within(region).findByText("Codex 0.158.0 is installed.")).toBeInTheDocument();
    expect(
      within(region).getByRole("button", { name: "Sign in with ChatGPT" }),
    ).toBeInTheDocument();
  });

  it("says when another window is installing Codex (not a sync)", async () => {
    const api = createMockApi({ latencyMs: 0, syncStepMs: 0, scenario: "codex-not-installed" });
    vi.spyOn(api, "installCodex").mockRejectedValue(new ApiError("busy", "Installing elsewhere."));
    const { user } = renderRoute("/settings", { api });
    const region = await card();
    await user.click(await within(region).findByRole("button", { name: "Download Codex" }));
    const alert = await within(region).findByRole("alert");
    expect(alert).toHaveTextContent("Codex is being installed in another PageLamp window.");
    expect(alert).not.toHaveTextContent(/sync/i);
    expect(within(region).getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("cancels a download without calling it a failure", async () => {
    const api = createMockApi({ latencyMs: 0, syncStepMs: 40, scenario: "codex-not-installed" });
    const { user } = renderRoute("/settings", { api });
    const region = await card();
    await user.click(await within(region).findByRole("button", { name: "Download Codex" }));
    await user.click(await within(region).findByRole("button", { name: "Cancel download" }));
    expect(
      await within(region).findByRole("button", { name: "Download Codex" }),
    ).toBeInTheDocument();
    expect(within(region).queryByRole("alert")).toBeNull();
    expect((await api.codexStatus()).runtime.state).toBe("not_installed");
  });

  it("signs in through the browser, then shows the Codex disclosure sheet", async () => {
    const { user } = renderRoute("/settings", { scenario: "codex-signed-out" });
    const region = await card();
    await user.click(await within(region).findByRole("button", { name: "Sign in with ChatGPT" }));
    const dialog = await screen.findByRole("dialog", { name: "Sign in with ChatGPT" });
    expect(within(dialog).getByRole("radio", { name: "In your browser" })).toBeChecked();
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));

    const sheet = await screen.findByRole("dialog", {
      name: "Before PageLamp uses ChatGPT plan (through OpenAI Codex)",
    });
    expect(
      within(sheet).getByText(
        /It goes to OpenAI through Codex, signed in with your ChatGPT account/,
      ),
    ).toBeInTheDocument();
    expect(
      within(sheet).getByRole("link", { name: /How to turn training off/ }),
    ).toBeInTheDocument();
    expect(
      within(sheet).getByText(/Past your plan limit, runs may use your ChatGPT credits/),
    ).toBeInTheDocument();
    await user.click(within(sheet).getByLabelText("I meet OpenAI's age requirement."));
    await user.click(
      within(sheet).getByRole("button", { name: "Turn on ChatGPT plan (through OpenAI Codex)" }),
    );
    await waitFor(() => expect(region).toHaveFocus());
    // The plan type isn't known after a sign-in yet (A7), so admin visibility is "unknown".
    expect(await within(region).findByText("Signed in with ChatGPT")).toBeInTheDocument();
    expect(within(region).getByText(/If this is an Edu or workspace account/)).toBeInTheDocument();
  });

  it("shows a device code to enter, and cancels the sign-in when closed", async () => {
    const api = createMockApi({ latencyMs: 0, syncStepMs: 200, scenario: "codex-signed-out" });
    const cancel = vi.spyOn(api, "cancelCodexLogin");
    const { user } = renderRoute("/settings", { api });
    const region = await card();
    await user.click(await within(region).findByRole("button", { name: "Sign in with ChatGPT" }));
    const dialog = await screen.findByRole("dialog", { name: "Sign in with ChatGPT" });
    await user.click(within(dialog).getByRole("radio", { name: "With a code" }));
    expect(within(dialog).getByRole("radio", { name: "With a code" })).toHaveAccessibleDescription(
      /turn on device code sign-in in ChatGPT's security settings/,
    );
    await user.click(within(dialog).getByRole("button", { name: "Continue" }));
    expect(await within(dialog).findByText(MOCK_DEVICE_CODE)).toBeInTheDocument();
    expect(within(dialog).getByText("The code expires in 15 minutes.")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(cancel).toHaveBeenCalled();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(
      within(region).getByRole("button", { name: "Sign in with ChatGPT" }),
    ).toBeInTheDocument();
  });

  it("warns that an Edu workspace's admins can see what is sent", async () => {
    renderRoute("/settings", { scenario: "codex-edu" });
    const region = await card();
    expect(
      await within(region).findByText(/its administrators can see what PageLamp sends/),
    ).toBeInTheDocument();
    expect(within(region).getByRole("button", { name: "Review and turn on" })).toBeInTheDocument();
  });

  it("warns that an API-key sign-in bills that key", async () => {
    renderRoute("/settings", { scenario: "codex-api-key" });
    const region = await card();
    expect(
      await within(region).findByText(/runs are billed to that key, not to your ChatGPT plan/),
    ).toBeInTheDocument();
    // The plan type is unknown: the admin warning says "may".
    expect(within(region).getByText(/If this is an Edu or workspace account/)).toBeInTheDocument();
  });

  it("points a plan without codex exec to what still works (D10)", async () => {
    renderRoute("/settings", { scenario: "codex-free" });
    const region = await card();
    expect(await within(region).findByText(/doesn't let other apps run Codex/)).toBeInTheDocument();
    expect(within(region).getByRole("link", { name: "Connect your AI app" })).toHaveAttribute(
      "href",
      "/connect",
    );
    expect(within(region).queryByRole("button", { name: "Review and turn on" })).toBeNull();
  });

  it("installs the pinned Codex on its own when the installed one is older", async () => {
    renderRoute("/settings", { scenario: "codex-outdated-pin" });
    const region = await card();
    expect(await within(region).findByText("Codex 0.157.1 is installed.")).toBeInTheDocument();
    expect(within(region).getByText("PageLamp needs a newer Codex; updating…")).toBeInTheDocument();
    expect(await within(region).findByText("Codex 0.158.0 is installed.")).toBeInTheDocument();
    expect(within(region).queryByText(/needs a newer Codex/)).toBeNull();
  });

  it("asks to update PageLamp when the pinned Codex is already installed", async () => {
    const { user } = renderRoute("/settings", { scenario: "codex-outdated-app" });
    const region = await card();
    expect(
      await within(region).findByText("Update PageLamp to keep using your ChatGPT plan."),
    ).toBeInTheDocument();
    await user.click(within(region).getByRole("button", { name: "Check for updates" }));
    expect(await within(region).findByText(/No update yet/)).toBeInTheDocument();
  });

  it("caps the weekly runs, shows them in usage, and can remove Codex", async () => {
    const { user } = renderRoute("/settings", { scenario: "codex-plus" });
    const region = await card();
    expect(await within(region).findByText("12 of 40 runs this week")).toBeInTheDocument();
    expect(await screen.findByText("12 of 40 ChatGPT-plan runs this week")).toBeInTheDocument();
    await user.click(within(region).getByLabelText("No limit"));
    await user.click(within(region).getByRole("button", { name: "Save limit" }));
    expect(await within(region).findByText("12 runs this week")).toBeInTheDocument();

    await user.click(within(region).getByRole("button", { name: "Remove Codex" }));
    const confirm = await screen.findByRole("alertdialog", { name: "Remove Codex?" });
    await user.click(within(confirm).getByRole("button", { name: "Remove Codex" }));
    expect(
      await within(region).findByRole("button", { name: "Download Codex" }),
    ).toBeInTheDocument();
  });

  it("before Generate: the plan instead of a price, and the weekly cap", async () => {
    renderWithProviders(
      <GenerateButton
        request={{ feature: "weekly_explanation", course: DEMO101 }}
        onGenerate={() => {}}
      />,
      { scenario: "codex-plus" },
    );
    expect(
      await screen.findByText("Uses your ChatGPT plan · 12 of 40 runs this week"),
    ).toBeInTheDocument();
  });

  it("blocks a run once the weekly cap is reached", async () => {
    renderWithProviders(
      <GenerateButton
        request={{ feature: "weekly_explanation", course: DEMO101 }}
        onGenerate={() => {}}
      />,
      { scenario: "codex-cap" },
    );
    expect(await screen.findByText("You've reached this week's run limit.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Generate" })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });
});
