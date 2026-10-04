import { screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/api/errors";
import { createMockApi, type MockOptions } from "@/api/mock";
import i18n from "@/i18n";
import { useUiStore } from "@/stores/ui";
import { renderRoute } from "@/test/render";

function mockApi(options: MockOptions = {}) {
  return createMockApi({ latencyMs: 0, syncStepMs: 0, ...options });
}

const group = () => screen.findByRole("radiogroup", { name: "Automatic sync" });
const checked = (name: string) =>
  screen.getByRole("radio", { name }).getAttribute("aria-checked") === "true";

afterEach(() => {
  vi.useRealTimers();
});

describe("Sources & sync → Automatic sync", () => {
  it("shows the choice with what the student should know about it", async () => {
    renderRoute("/sources");
    const setting = await group();
    expect(
      within(setting)
        .getAllByRole("radio")
        .map((r) => r.textContent),
    ).toEqual(["Off", "Once a day", "Twice a day"]);
    expect(checked("Twice a day")).toBe(true);
    expect(setting).toHaveAccessibleDescription(/^Runs only while PageLamp is open\. /);
    expect(setting).toHaveAccessibleDescription(
      /Canvas may record that as your activity in each course\./,
    );
    expect(setting).toHaveAccessibleDescription(/without opening any course\.$/);
    // A full sync on coming back only when the last one is as old as the choice.
    expect(setting).toHaveAccessibleDescription(
      /come back to it and the last full sync is as old as your choice above, it syncs everything/,
    );
  });

  it("saves a choice at once, and clicking it again changes nothing", async () => {
    const api = mockApi();
    const save = vi.spyOn(api, "setSyncPrefs");
    const { user } = renderRoute("/sources", { api });
    await group();

    await user.click(screen.getByRole("radio", { name: "Once a day" }));
    await waitFor(() => expect(checked("Once a day")).toBe(true));
    expect(save).toHaveBeenCalledWith({ auto_sync: "daily" });
    expect(await api.syncPrefs()).toEqual({ auto_sync: "daily" });

    await user.click(screen.getByRole("radio", { name: "Once a day" }));
    expect(checked("Once a day")).toBe(true);
    expect(save).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("radio", { name: "Off" }));
    await waitFor(() => expect(checked("Off")).toBe(true));
    expect(save).toHaveBeenLastCalledWith({ auto_sync: "off" });
  });

  it("can be chosen with the keyboard", async () => {
    const api = mockApi();
    const save = vi.spyOn(api, "setSyncPrefs");
    const { user } = renderRoute("/sources", { api });
    await group();
    screen.getByRole("radio", { name: "Twice a day" }).focus();
    await user.keyboard("{ArrowLeft}");
    expect(screen.getByRole("radio", { name: "Once a day" })).toHaveFocus();
    await user.keyboard(" ");
    await waitFor(() => expect(save).toHaveBeenCalledWith({ auto_sync: "daily" }));
  });

  it("keeps the saved choice and says why when saving fails", async () => {
    const api = mockApi();
    vi.spyOn(api, "setSyncPrefs").mockRejectedValue(new ApiError("internal", "boom"));
    const { user } = renderRoute("/sources", { api });
    await group();
    await user.click(screen.getByRole("radio", { name: "Off" }));
    expect(await screen.findByText("Something unexpected went wrong.")).toBeInTheDocument();
    expect(checked("Twice a day")).toBe(true);
  });

  it("starts a sync that is due as soon as it is turned on", async () => {
    // Only the date is faked, so the launch is long past when the student changes the setting.
    const start = new Date(2026, 9, 5, 9, 0);
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(start);
    const api = mockApi({ scenario: "auto-sync-due" });
    await api.setSyncPrefs({ auto_sync: "off" });
    const sync = vi.spyOn(api, "syncAll");
    const { user } = renderRoute("/sources", { api });
    await group();
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(sync).not.toHaveBeenCalled();

    vi.setSystemTime(new Date(start.getTime() + 5 * 60 * 1000));
    await user.click(screen.getByRole("radio", { name: "Twice a day" }));
    await waitFor(() => expect(sync).toHaveBeenCalledTimes(1));
    // The student asked for it just now.
    expect(sync.mock.calls[0]?.[0]).toEqual({ automatic: "attended" });
  });

  it("isn't offered before there is a source", async () => {
    renderRoute("/sources", { scenario: "empty" });
    expect(await screen.findByText("No sources yet")).toBeInTheDocument();
    expect(screen.queryByRole("radiogroup", { name: "Automatic sync" })).toBeNull();
  });

  it("is in Chinese too", async () => {
    useUiStore.setState({ locale: "zh-CN" });
    await i18n.changeLanguage("zh-CN");
    renderRoute("/sources");
    const setting = await screen.findByRole("radiogroup", { name: "自动同步" });
    expect(
      within(setting)
        .getAllByRole("radio")
        .map((r) => r.textContent),
    ).toEqual(["关闭", "每天一次", "每天两次"]);
    expect(setting).toHaveAccessibleDescription(/^只有 PageLamp 开着的时候才会自动同步。/);
    expect(setting).toHaveAccessibleDescription(/Canvas 可能会把这记为你在各门课里的活动。/);
    expect(setting).toHaveAccessibleDescription(/如果上次完整同步已经超过上面选的间隔/);
  });
});

describe("adding a Canvas source", () => {
  async function openCanvasForm(options: MockOptions = {}, off = false) {
    useUiStore.setState({ aiDisclosureAcknowledgedAt: "2026-09-01T12:00:00.000Z" });
    const api = mockApi(options);
    if (off) await api.setSyncPrefs({ auto_sync: "off" });
    const { user } = renderRoute("/sources", { api });
    await user.click(await screen.findByRole("button", { name: "Add source" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a source" });
    await user.click(within(dialog).getByRole("radio", { name: /Canvas/ }));
    return dialog;
  }

  it("says that PageLamp syncs by itself and where to turn that off", async () => {
    const dialog = await openCanvasForm();
    expect(
      await within(dialog).findByText(
        "PageLamp syncs by itself while it's open. You can turn that off on Sources & sync.",
      ),
    ).toBeInTheDocument();
  });

  it("doesn't say so when automatic sync is off", async () => {
    const dialog = await openCanvasForm({}, true);
    await within(dialog).findByText(/Personal access tokens are for your own use only/);
    expect(within(dialog).queryByText(/syncs by itself/)).toBeNull();
  });
});
