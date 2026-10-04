import { act, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createMockApi } from "@/api/mock";
import { useSyncStore } from "@/stores/sync";
import { renderWithProviders } from "@/test/render";
import { SyncPill } from "./SyncPill";

afterEach(() => {
  vi.useRealTimers();
});

// The pill on its own, not a whole route: rendering /settings and searching its accessible
// names took over 5 s on the Windows runner.
describe("SyncPill", () => {
  it("keeps the last state while this window syncs (the accessory bar has the progress)", async () => {
    renderWithProviders(<SyncPill />);
    const pill = await screen.findByRole("link", { name: /^Sync status: Synced .+ ago\./ });
    act(() => {
      useSyncStore.getState().begin(3);
      useSyncStore
        .getState()
        .apply({ type: "source_started", source_id: "canvas", label: "Demo Canvas" });
    });
    expect(pill).toHaveAccessibleName(/^Sync status: Synced .+ ago\./);
    expect(pill).not.toHaveTextContent("Syncing");
  });

  it("says another process is syncing", async () => {
    renderWithProviders(<SyncPill />, { api: createMockApi({ latencyMs: 0, scenario: "busy" }) });
    expect(
      await screen.findByRole("link", { name: "Sync status: Syncing…. Open Sources & sync." }),
    ).toBeInTheDocument();
  });

  it("marks a recent sync with the check", async () => {
    renderWithProviders(<SyncPill />);
    const pill = await screen.findByRole("link", { name: /^Sync status: Synced 2 hours ago\./ });
    expect(pill.querySelector('[data-sync="fresh"]')).not.toBeNull();
    expect(pill.querySelector('[data-sync="old"]')).toBeNull();
  });

  it("stops looking fresh once the last sync is older than the chosen interval", async () => {
    // 13 hours ago, with "Twice a day": old, but nothing is wrong.
    renderWithProviders(<SyncPill />, {
      api: createMockApi({ latencyMs: 0, scenario: "auto-sync-due" }),
    });
    const pill = await screen.findByRole("link", { name: /^Sync status: Synced 13 hours ago\./ });
    expect(pill.querySelector('[data-sync="old"]')).not.toBeNull();
    expect(pill.querySelector('[data-sync="fresh"]')).toBeNull();
    expect(pill).not.toHaveTextContent("Needs attention");
    expect(pill).not.toHaveClass("text-destructive");
  });

  it("measures 'old' by the setting: 13 hours is recent for once a day", async () => {
    const api = createMockApi({ latencyMs: 0, scenario: "auto-sync-due" });
    await api.setSyncPrefs({ auto_sync: "daily" });
    renderWithProviders(<SyncPill />, { api });
    const pill = await screen.findByRole("link", { name: /^Sync status: Synced 13 hours ago\./ });
    expect(pill.querySelector('[data-sync="fresh"]')).not.toBeNull();
  });

  it("shows the oldest source: one synced a minute ago doesn't make the others fresh", async () => {
    // The folder and the feed 2 hours ago, Canvas 5 days ago, nothing wrong with any of them.
    renderWithProviders(<SyncPill />, {
      api: createMockApi({ latencyMs: 0, scenario: "canvas-old" }),
    });
    const pill = await screen.findByRole("link", { name: /^Sync status: Synced 5 days ago\./ });
    expect(pill.querySelector('[data-sync="old"]')).not.toBeNull();
    expect(pill.querySelector('[data-sync="fresh"]')).toBeNull();
    expect(pill).not.toHaveTextContent("Needs attention");
  });

  it("is old while a source has never synced", async () => {
    const api = createMockApi({ latencyMs: 0 });
    await api.addFolderSource("/Users/demo/Documents/New", null, "New folder");
    renderWithProviders(<SyncPill />, { api });
    const pill = await screen.findByRole("link", { name: /^Sync status: Synced 2 hours ago\./ });
    expect(pill.querySelector('[data-sync="old"]')).not.toBeNull();
  });

  // Canvas there is 9 days old and its token expired: the problem is said, not the age.
  it("keeps 'Needs attention' for problems, however old the data", async () => {
    renderWithProviders(<SyncPill />, {
      api: createMockApi({ latencyMs: 0, scenario: "expired" }),
    });
    const pill = await screen.findByRole("link", {
      name: "Sync status: Needs attention. Open Sources & sync.",
    });
    expect(pill.querySelector("[data-sync]")).toBeNull();
  });

  it("keeps counting while it sits there, and turns old when the interval has passed", async () => {
    const start = new Date(2026, 9, 5, 9, 0);
    vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] });
    vi.setSystemTime(start);
    renderWithProviders(<SyncPill />);
    const pill = await screen.findByRole("link", { name: /^Sync status: Synced 2 hours ago\./ });

    // Nothing is fetched again: only the minute goes by.
    vi.setSystemTime(new Date(start.getTime() + 11 * 60 * 60 * 1000));
    act(() => vi.advanceTimersByTime(60_000));
    expect(pill).toHaveAccessibleName(/^Sync status: Synced 13 hours ago\./);
    expect(pill.querySelector('[data-sync="old"]')).not.toBeNull();
  });
});
