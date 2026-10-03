import { screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { createMockApi } from "@/api/mock";
import { renderRoute } from "@/test/render";

it("runs the app-start purge once when the facade says it's due", async () => {
  const api = createMockApi({ latencyMs: 0, syncStepMs: 0, scenario: "removed" });
  const tasks = api.startupTasks.bind(api);
  api.startupTasks = async () => ({ ...(await tasks()), purge_due: true });
  const purge = vi.spyOn(api, "purgeRemovedCourses");
  renderRoute("/courses", { api });
  await screen.findByRole("heading", { level: 1 });
  await waitFor(() => expect(purge).toHaveBeenCalledWith(null, false));
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(purge).toHaveBeenCalledTimes(1);
});

it("doesn't purge when nothing is due", async () => {
  const api = createMockApi({ latencyMs: 0, syncStepMs: 0, scenario: "removed" });
  const purge = vi.spyOn(api, "purgeRemovedCourses");
  renderRoute("/courses", { api });
  await screen.findByRole("heading", { level: 1 });
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(purge).not.toHaveBeenCalled();
});
