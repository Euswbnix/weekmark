import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { aiKeys } from "@/api/ai-queries";
import { createMockApi } from "@/api/mock";
import type { MockScenario } from "@/api/mock/fixtures";
import { renderRoute } from "@/test/render";

async function openDialog(scenario: MockScenario) {
  const api = createMockApi({ latencyMs: 0, scenario });
  const { user, queryClient } = renderRoute("/settings", { api });
  const button = await screen.findByRole("button", { name: "Remove all AI data" });
  // The dialog's words follow the ChatGPT sign-in: wait until it has been read.
  await waitFor(() => expect(queryClient.getQueryData(aiKeys.codex())).toBeDefined());
  await user.click(button);
  return screen.findByRole("alertdialog", { name: "Remove all AI data?" });
}

describe("Remove all AI data", () => {
  it("names the ChatGPT sign-out when there is one to undo", async () => {
    const dialog = await openDialog("codex-plus");
    expect(within(dialog).getByText(/signs PageLamp's Codex out of ChatGPT/)).toBeInTheDocument();
    expect(within(dialog).getByText(/everything PageLamp wrote with AI/)).toBeInTheDocument();
  });

  it.each(["ai-key", "codex-signed-out"] as const)(
    "doesn't mention ChatGPT without a ChatGPT sign-in (%s)",
    async (scenario) => {
      const dialog = await openDialog(scenario);
      expect(within(dialog).getByText(/everything PageLamp wrote with AI\./)).toBeInTheDocument();
      expect(within(dialog).queryByText(/ChatGPT|Codex/)).toBeNull();
      expect(within(dialog).queryByText(/every plan and explanation/)).toBeNull();
    },
  );
});
