import { act, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type { EstimateRequest } from "@/api/ai";
import { createMockApi } from "@/api/mock";
import { DEMO101, DEMO205 } from "@/features/course/testing";
import { renderWithProviders } from "@/test/render";
import { GenerateButton } from "./GenerateButton";
import { MaterialSharingReminder, SharingNotAllowedNotice } from "./MaterialSharingNotices";

const explain = (course: string): EstimateRequest => ({ feature: "weekly_explanation", course });

function generate() {
  return screen.getByRole("button", { name: "Generate" });
}

describe("≈ $x before Generate", () => {
  it("shows the upper bound, rounded up to the cent, and generates", async () => {
    const onGenerate = vi.fn();
    const { user } = renderWithProviders(
      <GenerateButton request={explain(DEMO101)} onGenerate={onGenerate} />,
      { scenario: "ai-key" },
    );
    expect(await screen.findByText("≈ $0.07 at most")).toBeInTheDocument();
    expect(generate()).toHaveAccessibleDescription("Estimated cost: ≈ $0.07 at most");
    expect(screen.getByText("Up to 45K tokens in and 8K out")).toBeInTheDocument();
    await user.click(generate());
    expect(onGenerate).toHaveBeenCalledWith({ overrideBudget: false });
  });

  it("goes over the budget only when the student says so, for this run", async () => {
    const onGenerate = vi.fn();
    const { user } = renderWithProviders(
      <GenerateButton request={explain(DEMO101)} onGenerate={onGenerate} />,
      { scenario: "ai-budget" },
    );
    expect(await screen.findByText("This would go over this month's budget.")).toBeInTheDocument();
    expect(generate()).toHaveAttribute("aria-disabled", "true");
    await user.click(generate());
    expect(onGenerate).not.toHaveBeenCalled();
    await user.click(screen.getByLabelText("Go over the budget this time"));
    await user.click(generate());
    expect(onGenerate).toHaveBeenCalledWith({ overrideBudget: true });
  });

  it("keeps the tick through a refetch of the same request with the same block", async () => {
    const onGenerate = vi.fn();
    const { user, api, queryClient } = renderWithProviders(
      <GenerateButton request={explain(DEMO101)} onGenerate={onGenerate} />,
      { scenario: "ai-budget" },
    );
    const estimate = vi.spyOn(api, "estimateGeneration");
    expect(await screen.findByText("This would go over this month's budget.")).toBeInTheDocument();
    await user.click(screen.getByLabelText("Go over the budget this time"));
    expect(generate()).not.toHaveAttribute("aria-disabled");
    await act(async () => {
      await queryClient.invalidateQueries();
    });
    expect(estimate).toHaveBeenCalled();
    expect(screen.getByLabelText("Go over the budget this time")).toBeChecked();
    expect(generate()).not.toHaveAttribute("aria-disabled");
    await user.click(generate());
    expect(onGenerate).toHaveBeenCalledWith({ overrideBudget: true });
  });

  it("waits for a changed request's own estimate before it runs", async () => {
    const onGenerate = vi.fn();
    function Switching() {
      const [include, setInclude] = useState<string[]>([]);
      return (
        <>
          <button type="button" onClick={() => setInclude(["no-such-material"])}>
            Change
          </button>
          <GenerateButton
            request={{ feature: "weekly_explanation", course: DEMO101, include }}
            onGenerate={onGenerate}
          />
        </>
      );
    }
    const { user } = renderWithProviders(<Switching />, { scenario: "ai-key" });
    expect(await screen.findByText("≈ $0.07 at most")).toBeInTheDocument();
    await vi.waitFor(() => expect(generate()).not.toHaveAttribute("aria-disabled"));
    await user.click(screen.getByRole("button", { name: "Change" }));
    // The previous request's "≈ $x" is still shown, but nothing runs from it.
    expect(generate()).toHaveAttribute("aria-disabled", "true");
    await user.click(generate());
    expect(onGenerate).not.toHaveBeenCalled();
    await vi.waitFor(() => expect(generate()).not.toHaveAttribute("aria-disabled"));
  });

  it("asks once before using a model with no price", async () => {
    const { user, api } = renderWithProviders(
      <GenerateButton request={explain(DEMO101)} onGenerate={() => {}} />,
      { scenario: "ai-unpriced" },
    );
    const acknowledge = vi.spyOn(api, "acknowledgeUnpricedModel");
    expect(await screen.findByText("No price for this model")).toBeInTheDocument();
    expect(generate()).toHaveAttribute("aria-disabled", "true");
    await user.click(screen.getByRole("button", { name: "Use it anyway" }));
    expect(acknowledge).toHaveBeenCalledWith(
      { kind: "provider", provider_id: "openai" },
      "gpt-6-preview-0929",
    );
    await vi.waitFor(() => expect(generate()).not.toHaveAttribute("aria-disabled"));
  });

  it("is free on this computer", async () => {
    renderWithProviders(<GenerateButton request={explain(DEMO205)} onGenerate={() => {}} />, {
      scenario: "ai-local",
    });
    expect(await screen.findByText("Free, on this computer")).toBeInTheDocument();
  });

  it("explains why it can't run, and where to fix it", async () => {
    renderWithProviders(
      <GenerateButton request={{ feature: "weekly_note" }} onGenerate={() => {}} />,
    );
    expect(await screen.findByText(/Choose a model for this/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "AI models" })).toHaveAttribute("href", "/settings");
    expect(generate()).toHaveAttribute("aria-disabled", "true");
  });

  it("says there's nothing to write about, with no cost, no tokens and no Settings link", async () => {
    const api = createMockApi({ latencyMs: 0, syncStepMs: 0, scenario: "ai-key" });
    vi.spyOn(api, "estimateGeneration").mockResolvedValue({
      micro_usd_upper: null,
      input_tokens: 0,
      max_output_tokens: 0,
      reasoning_allowance: 0,
      repair_possible: false,
      price_known: false,
      would_block: "nothing_to_write",
    });
    const onGenerate = vi.fn();
    const { user } = renderWithProviders(
      <GenerateButton request={{ feature: "weekly_note" }} onGenerate={onGenerate} />,
      { api },
    );
    expect(await screen.findByText(/nothing to write about this week yet/)).toBeInTheDocument();
    expect(generate()).toHaveAttribute("aria-disabled", "true");
    expect(screen.queryByRole("link", { name: "AI models" })).toBeNull();
    // No cost line at all: not even "No price for this model", which the blocked estimate's empty
    // price would read as if the cost line showed for it.
    expect(screen.queryByText(/No price for this model/)).toBeNull();
    await user.click(generate());
    expect(onGenerate).not.toHaveBeenCalled();
  });

  it("never sends a course whose materials may not be shared to a cloud model", async () => {
    renderWithProviders(<GenerateButton request={explain(DEMO205)} onGenerate={() => {}} />, {
      scenario: "ai-key",
    });
    expect(
      await screen.findByText(/may not be shared with AI services, so nothing was sent/),
    ).toBeInTheDocument();
    expect(generate()).toHaveAttribute("aria-disabled", "true");
  });
});

describe("question (b) notices", () => {
  it("reminds once, without blocking, and records the answer given inline", async () => {
    const onClose = vi.fn();
    const { user, api } = renderWithProviders(
      <MaterialSharingReminder
        courseId={DEMO101}
        courseName="DEMO101"
        service="OpenAI"
        onClose={onClose}
      />,
    );
    const save = vi.spyOn(api, "setCourseMaterialSharing");
    expect(
      screen.getByRole("heading", {
        name: "Check whether your instructor allows sharing course materials with AI services",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText(/sent text from DEMO101's materials to OpenAI/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Yes, it's allowed" }));
    expect(save).toHaveBeenCalledWith(DEMO101, "allowed");
    await vi.waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("offers what still works when sharing isn't allowed", async () => {
    const onUseLocal = vi.fn();
    const { user } = renderWithProviders(
      <SharingNotAllowedNotice courseId={DEMO205} courseName="DEMO205" onUseLocal={onUseLocal} />,
    );
    expect(screen.getByText(/DEMO205's materials may not be shared/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /structure only/ })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Use a model on this computer" }));
    expect(onUseLocal).toHaveBeenCalled();
    expect(screen.getByRole("link", { name: "Change the answer" })).toHaveAttribute(
      "href",
      `/courses/${encodeURIComponent(DEMO205)}?tab=policy`,
    );
  });
});
