import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockApi } from "@/api/mock";
import { brand } from "@/brand";
import { useUiStore } from "@/stores/ui";
import { renderRoute, renderWithProviders } from "@/test/render";
import { AiOfferCard } from "./AiOfferCard";

const TITLE = `Let ${brand.productName} write plans and explanations`;

describe("AiOfferCard", () => {
  it("points to Settings → AI models, and counts that as the answer", async () => {
    const { user, router } = renderWithProviders(<AiOfferCard />);
    expect(await screen.findByRole("region", { name: TITLE })).toBeInTheDocument();
    const setUp = screen.getByRole("link", { name: "Set up a model" });
    expect(setUp).toHaveAttribute("href", "/settings#ai-models");

    await user.click(setUp);
    expect(router.state.location.pathname).toBe("/settings");
    expect(router.state.location.hash).toBe("#ai-models");
    expect(useUiStore.getState().aiOfferAnswered).toBe(true);
  });

  it("Not now says where to find it later, takes the focus, and is never asked again", async () => {
    const first = renderWithProviders(<AiOfferCard />);
    await first.user.click(await screen.findByRole("button", { name: "Not now" }));
    const note = screen.getByRole("status");
    expect(note).toHaveTextContent("You can set one up any time in Settings → AI models.");
    expect(note).toHaveFocus();
    expect(useUiStore.getState().aiOfferAnswered).toBe(true);
    first.unmount();

    const api = createMockApi({ latencyMs: 0 });
    const status = vi.spyOn(api, "aiStatus");
    renderWithProviders(<AiOfferCard />, { api });
    await waitFor(() => expect(status).toHaveBeenCalled());
    expect(screen.queryByRole("region", { name: TITLE })).toBeNull();
  });

  it("isn't offered to a student who already has a model ready", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const real = await api.aiStatus();
    const backend = {
      ...real.backends[0],
      state: "ready" as const,
    } as (typeof real.backends)[number];
    const status = vi.spyOn(api, "aiStatus").mockResolvedValue({ ...real, backends: [backend] });
    renderWithProviders(<AiOfferCard />, { api });
    await waitFor(() => expect(status).toHaveBeenCalled());
    await waitFor(() => expect(screen.queryByRole("button", { name: "Not now" })).toBeNull());
    expect(screen.queryByRole("region", { name: TITLE })).toBeNull();
  });
});

describe("Settings at #ai-models", () => {
  it("focuses the AI models heading", async () => {
    renderRoute("/settings#ai-models");
    const heading = await screen.findByRole("heading", { level: 2, name: "AI models" });
    await waitFor(() => expect(heading).toHaveFocus());
  });
});
