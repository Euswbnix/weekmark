import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WeeklyExplanation } from "@/api/explain";
import { createMockApi } from "@/api/mock";
import { renderWithProviders } from "@/test/render";
import { useSavedExplanations } from "./useExplanation";

const COURSE = "canvas:canvas.demo.test/course/205";

function Saved({ week }: { week: number | null }) {
  const saved = useSavedExplanations(COURSE, week);
  if (!saved.data) return null;
  return (
    <ul aria-label="saved">
      {saved.data.map((e) => (
        <li key={e.meta.generation_id}>{e.meta.generation_id}</li>
      ))}
    </ul>
  );
}

const explanation = (id: string, week: number | null) =>
  ({ meta: { generation_id: id }, week }) as WeeklyExplanation;

describe("saved explanations without a week", () => {
  it("keeps only those of the recent materials from the facade's every-week answer", async () => {
    const api = createMockApi({ latencyMs: 0 });
    const saved = vi
      .spyOn(api, "savedExplanations")
      .mockResolvedValue([explanation("week-4", 4), explanation("recent", null)]);
    renderWithProviders(<Saved week={null} />, { api });
    const list = await screen.findByRole("list", { name: "saved" });
    expect(list).toHaveTextContent("recent");
    expect(list).not.toHaveTextContent("week-4");
    expect(saved).toHaveBeenCalledWith(COURSE, null);
  });

  it("keeps a week's list as the facade gives it", async () => {
    const api = createMockApi({ latencyMs: 0 });
    vi.spyOn(api, "savedExplanations").mockResolvedValue([explanation("week-4", 4)]);
    renderWithProviders(<Saved week={4} />, { api });
    await waitFor(() =>
      expect(screen.getByRole("list", { name: "saved" })).toHaveTextContent("week-4"),
    );
  });
});
