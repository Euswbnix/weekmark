import { act, screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createMockApi } from "@/api/mock";
import { MOCK_REMINDER_CHECK_EVENT } from "@/api/mock/reminders";
import { renderRoute, renderWithProviders } from "@/test/render";
import { RemindMeCard } from "./RemindMeCard";

function mockApi(options: Parameters<typeof createMockApi>[0] = {}) {
  return createMockApi({ latencyMs: 0, syncStepMs: 0, ...options });
}

async function remindersSection() {
  const heading = await screen.findByRole("heading", { level: 2, name: "Reminders" });
  const region = heading.closest("section");
  if (!region) throw new Error("no Reminders section");
  return region;
}

const REMIND_ME = "Keep PageLamp in the tray and start it at login";

describe("Settings → Reminders", () => {
  it("is off by default; turning it on shows the kinds and sends the one notice", async () => {
    const api = mockApi();
    const save = vi.spyOn(api, "setReminderSettings");
    const notice = vi.spyOn(api, "showRemindersOnNotice");
    const { user } = renderRoute("/settings", { api });
    const section = await remindersSection();

    const remindMe = await within(section).findByRole("switch", { name: REMIND_ME });
    expect(remindMe).not.toBeChecked();
    expect(within(section).queryByRole("switch", { name: "Your week" })).toBeNull();

    await user.click(remindMe);
    await waitFor(() => expect(notice).toHaveBeenCalledTimes(1));
    expect(notice).toHaveBeenCalledWith(
      "Reminders are on",
      "PageLamp will remind you here about deadlines and your week.",
    );
    expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ run_in_background: true }));
    expect(within(section).getByRole("switch", { name: "Deadlines coming up" })).toBeChecked();
    expect(within(section).getByRole("switch", { name: "Your week" })).toBeChecked();
    expect(within(section).getByRole("switch", { name: "Today's study plan" })).not.toBeChecked();
    // Whether notifications are allowed can't be read: say where to look.
    expect(within(section).getByText(/Not seeing reminders\?/)).toBeInTheDocument();
    const settings = vi.spyOn(api, "openNotificationSettings");
    await user.click(within(section).getByRole("button", { name: "Open notification settings" }));
    expect(settings).toHaveBeenCalled();

    // Another change keeps it on and sends no second notice.
    await user.click(within(section).getByRole("switch", { name: "Deadlines coming up" }));
    await waitFor(() =>
      expect(save).toHaveBeenLastCalledWith(
        expect.objectContaining({ run_in_background: true, deadline_soon: false }),
      ),
    );
    expect(notice).toHaveBeenCalledTimes(1);
  });

  it("saves the digest time when a valid time is left, and says what a valid one looks like", async () => {
    const api = mockApi();
    await api.setReminderSettings({ ...(await api.reminderSettings()), run_in_background: true });
    const save = vi.spyOn(api, "setReminderSettings");
    const { user } = renderRoute("/settings", { api });
    const section = await remindersSection();

    const time = await within(section).findByLabelText("Time for Your week");
    expect(time).toHaveValue("09:00");
    await user.clear(time);
    expect(within(section).getByText("Use a time like 09:00.")).toBeInTheDocument();
    expect(time).toHaveAttribute("aria-invalid", "true");
    await user.type(time, "07:30");
    await user.tab();
    await waitFor(() =>
      expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ digest_time: "07:30" })),
    );
    expect((await api.reminderSettings()).digest_time).toBe("07:30");
  });

  it("says when the student removed the login item in the system's settings", async () => {
    const api = mockApi();
    await api.setReminderSettings({ ...(await api.reminderSettings()), run_in_background: true });
    api.backgroundStatus = async () => ({
      run_in_background: true,
      tray: true,
      tray_unavailable: false,
      login_item: false,
    });
    renderRoute("/settings", { api });
    const section = await remindersSection();
    expect(
      await within(section).findByText(/Starting at login is turned off in your system's settings/),
    ).toBeInTheDocument();
  });

  it("says when this system can't show a tray icon", async () => {
    renderRoute("/settings", { scenario: "reminders-no-tray" });
    const section = await remindersSection();
    expect(
      await within(section).findByText(
        /The tray isn't available on this system; install libayatana-appindicator3-1/,
      ),
    ).toBeInTheDocument();
  });
});

describe("onboarding: Remind me", () => {
  it("turns running in the background on with Yes, and says where to change it", async () => {
    const api = mockApi();
    const notice = vi.spyOn(api, "showRemindersOnNotice");
    const { user } = renderWithProviders(<RemindMeCard />, { api });
    await user.click(await screen.findByRole("button", { name: "Yes, remind me" }));
    const answered = await screen.findByRole("status");
    expect(answered).toHaveTextContent(
      "PageLamp will remind you. You can change this in Settings → Reminders.",
    );
    // It replaces the buttons, so it takes the focus.
    expect(answered).toHaveFocus();
    expect((await api.reminderSettings()).run_in_background).toBe(true);
    expect(notice).toHaveBeenCalledTimes(1);
  });

  it("puts the focus on the error when the answer couldn't be saved", async () => {
    const api = mockApi();
    vi.spyOn(api, "setReminderSettings").mockRejectedValue(new Error("disk full"));
    const { user } = renderWithProviders(<RemindMeCard />, { api });
    await user.click(await screen.findByRole("button", { name: "Yes, remind me" }));
    expect(await screen.findByRole("alert")).toHaveFocus();
  });

  it("leaves it off with Not now, and sends no notification", async () => {
    const api = mockApi();
    const notice = vi.spyOn(api, "showRemindersOnNotice");
    const { user } = renderWithProviders(<RemindMeCard />, { api });
    await user.click(await screen.findByRole("button", { name: "Not now" }));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "No reminders. You can turn them on in Settings → Reminders.",
    );
    expect((await api.reminderSettings()).run_in_background).toBe(false);
    expect(notice).not.toHaveBeenCalled();
  });
});

describe("reminder delivery", () => {
  it("shows nothing until the student said Remind me", async () => {
    const api = mockApi({ scenario: "reminders-due" });
    const show = vi.spyOn(api, "showReminders");
    const due = vi.spyOn(api, "dueReminders");
    renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    act(() => {
      window.dispatchEvent(new Event(MOCK_REMINDER_CHECK_EVENT));
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(due).not.toHaveBeenCalled();
    expect(show).not.toHaveBeenCalled();
  });

  it("shows what is due at launch, worded, and each reminder only once", async () => {
    const api = mockApi({ scenario: "reminders-due" });
    await api.setReminderSettings({ ...(await api.reminderSettings()), run_in_background: true });
    const show = vi.spyOn(api, "showReminders");
    const due = vi.spyOn(api, "dueReminders");
    renderRoute("/courses", { api });

    await waitFor(() => expect(show).toHaveBeenCalledTimes(1));
    const [shown] = show.mock.calls[0] ?? [];
    expect(shown?.map((n) => n.id)).toEqual([
      "deadline_soon:demo-ps3:24",
      "weekly_digest:2026-W40",
    ]);
    expect(shown?.[0]?.title).toMatch(/^[A-Z]+\d+: Problem set 3$/);
    expect(shown?.[1]).toMatchObject({
      title: "Your week in PageLamp",
      body: "2 deadlines in the next 7 days",
    });

    // The shell asks again: nothing new is due, so nothing is shown twice.
    act(() => {
      window.dispatchEvent(new Event(MOCK_REMINDER_CHECK_EVENT));
    });
    await waitFor(() => expect(due).toHaveBeenCalledTimes(2));
    expect(show).toHaveBeenCalledTimes(1);
  });
});

describe("reminders while they're off: the catch-up card", () => {
  async function card() {
    return screen.findByRole("region", { name: "Since you last opened PageLamp" });
  }

  it("lists what came due, and Dismiss marks them all shown", async () => {
    const api = mockApi({ scenario: "reminders-due" });
    const mark = vi.spyOn(api, "markRemindersShown");
    const { user } = renderRoute("/courses", { api });
    const region = await card();
    expect(within(region).getByText(/^[A-Z]+\d+: Problem set 3$/)).toBeInTheDocument();
    expect(within(region).getByText("Your week in PageLamp")).toBeInTheDocument();

    expect(
      within(region).getByRole("link", { name: "Get these as notifications" }),
    ).toHaveAttribute("href", "/settings#reminders");

    await user.click(within(region).getByRole("button", { name: "Dismiss" }));
    expect(mark).toHaveBeenCalledWith(["deadline_soon:demo-ps3:24", "weekly_digest:2026-W40"]);
    expect(screen.queryByRole("region", { name: "Since you last opened PageLamp" })).toBeNull();
    // The card went with the focused button: the page's heading takes the focus.
    await waitFor(() => expect(screen.getByRole("heading", { level: 1 })).toHaveFocus());
    expect((await api.startupTasks()).due_reminders).toEqual([]);
  });

  it("opens a deadline's course and marks only that one", async () => {
    const api = mockApi({ scenario: "reminders-due" });
    const mark = vi.spyOn(api, "markRemindersShown");
    const { user, router } = renderRoute("/courses", { api });
    const region = await card();
    const [open] = within(region).getAllByRole("link", { name: /^Open [A-Z]+\d+: Problem set 3$/ });
    if (!open) throw new Error("no Open link");
    await user.click(open);
    expect(mark).toHaveBeenCalledWith(["deadline_soon:demo-ps3:24"]);
    await waitFor(() => expect(router.state.location.pathname).toMatch(/^\/courses\/.+/));
  });

  it("keeps the focus in the card when an Open on this page leaves other rows", async () => {
    const { user } = renderRoute("/courses", { scenario: "reminders-due" });
    const region = await card();
    await user.click(within(region).getByRole("link", { name: /^Open Your week in PageLamp$/ }));
    await waitFor(() =>
      expect(within(region).getByText("Since you last opened PageLamp")).toHaveFocus(),
    );
  });

  it("isn't brought back by turning reminders off after a launch with them on", async () => {
    const api = mockApi({ scenario: "reminders-due" });
    await api.setReminderSettings({ ...(await api.reminderSettings()), run_in_background: true });
    const { user } = renderRoute("/settings", { api });
    const on = await screen.findByRole("switch", {
      name: "Keep PageLamp in the tray and start it at login",
    });
    await waitFor(() => expect(on).toBeChecked());
    await user.click(on);
    await waitFor(() => expect(on).not.toBeChecked());
    expect(screen.queryByRole("region", { name: "Since you last opened PageLamp" })).toBeNull();
  });

  it("isn't shown when reminders come as notifications", async () => {
    const api = mockApi({ scenario: "reminders-due" });
    await api.setReminderSettings({ ...(await api.reminderSettings()), run_in_background: true });
    renderRoute("/courses", { api });
    await screen.findByRole("heading", { level: 1 });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(screen.queryByRole("region", { name: "Since you last opened PageLamp" })).toBeNull();
  });
});
