// What each "synced" line shows once PageLamp syncs by itself. A source has two clocks: its last
// FULL sync, and (later) the last time an automatic sync with nobody at the app read only its
// deadlines and announcements. No line may call the materials fresher, or the deadlines older,
// than they are; and a course only such a light sync has found says its materials are unread.

import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { createMockApi } from "@/api/mock";
import { SyncPill } from "@/components/layout/SyncPill";
import i18n from "@/i18n";
import { paths } from "@/lib/routes";
import { useUiStore } from "@/stores/ui";
import { renderRoute, renderWithProviders } from "@/test/render";

const DEMO404 = "canvas:canvas.demo.test/course/404";
const DEMO205 = "canvas:canvas.demo.test/course/205";

/** Canvas: full sync 13 hours ago, deadlines read 1 hour ago; DEMO404 found by that light read. */
const light = { scenario: "light-synced" } as const;

async function card(name: string): Promise<HTMLElement> {
  const item = (await screen.findByRole("heading", { name })).closest("li");
  if (!item) throw new Error(`no card for ${name}`);
  return item;
}

async function courseRow(code: RegExp): Promise<HTMLElement> {
  const row = (await screen.findByRole("link", { name: code })).closest("li");
  if (!row) throw new Error(`no row for ${code}`);
  return row;
}

/** A sentence is split by its <time> elements, so it is read off the whole page. */
async function pageSays(text: string) {
  await waitFor(() => expect(document.body).toHaveTextContent(text));
}

describe("after a light automatic sync", () => {
  it("the sidebar keeps the full sync's age: a light read doesn't make it look fresh", async () => {
    const api = createMockApi({ latencyMs: 0, ...light });
    // Against "Twice a day", 13 hours is old (the scenario itself has automatic sync off).
    await api.setSyncPrefs({ auto_sync: "twice_daily" });
    renderWithProviders(<SyncPill />, { api });
    const pill = await screen.findByRole("link", {
      name: "Sync status: Synced 13 hours ago. Open Sources & sync.",
    });
    expect(pill.querySelector('[data-sync="old"]')).not.toBeNull();
    expect(pill.querySelector('[data-sync="fresh"]')).toBeNull();
    // When the deadlines were read is on the source's card, not here.
    expect(pill).not.toHaveTextContent("Deadlines");
  });

  it("the Canvas card says both times; the other sources have one", async () => {
    renderRoute("/sources", light);
    const canvas = await card("Demo Canvas");
    await waitFor(() =>
      expect(canvas).toHaveTextContent("Deadlines and announcements checked 1 hour ago"),
    );
    expect(within(canvas).getByText("13 hours ago")).toBeInTheDocument();

    const folder = await card("Course folder");
    expect(within(folder).getByText("13 hours ago")).toBeInTheDocument();
    expect(folder).not.toHaveTextContent("Deadlines and announcements checked");
  });

  it("a Canvas course says when it was synced and when its deadlines were checked", async () => {
    renderRoute("/courses", light);
    expect(await courseRow(/DEMO205/)).toHaveTextContent(
      "Demo Canvas · synced 13 hours ago · deadlines checked 1 hour ago",
    );
  });

  it("a course it found says its materials haven't been read, not that it has none", async () => {
    renderRoute("/courses", light);
    const row = await courseRow(/DEMO404/);
    expect(row).toHaveTextContent(
      "Demo Canvas · materials not read yet · deadlines checked 1 hour ago",
    );
    expect(row).not.toHaveTextContent("synced 13 hours ago");
    expect(row).not.toHaveTextContent("No materials yet");
    // Its deadline was read.
    expect(row).toHaveTextContent("Next: Seminar proposal");
  });

  it("that course's page says the same in its header and on the Week tab", async () => {
    renderRoute(paths.course(DEMO404), light);
    await pageSays(
      "Data from Demo Canvas · materials not read yet · deadlines and announcements checked 1 hour ago",
    );
    expect(document.body).not.toHaveTextContent("synced 13 hours ago");
    expect(
      await screen.findByText("This course's materials haven't been read yet"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Its modules and materials are read by the next full sync\./),
    ).toBeInTheDocument();
    expect(screen.queryByText("No recent materials")).toBeNull();
    expect(screen.queryByText(/^No materials found/)).toBeNull();
  });

  it("a read course's page keeps its full sync's time and adds the deadlines' time", async () => {
    renderRoute(paths.course(DEMO205), light);
    await pageSays(
      "Data from Demo Canvas · synced 13 hours ago · deadlines and announcements checked 1 hour ago",
    );
  });

  it("says it in Chinese too", async () => {
    useUiStore.setState({ locale: "zh-CN" });
    await i18n.changeLanguage("zh-CN");
    renderRoute(paths.course(DEMO404), light);
    await pageSays("数据来源：Demo Canvas · 资料尚未读取 · 1小时前检查过截止日期和公告");
    expect(await screen.findByText("这门课的资料还没有读取")).toBeInTheDocument();
  });

  it("a full sync puts everything back on one clock", async () => {
    const api = createMockApi({ latencyMs: 0, syncStepMs: 0, ...light });
    await api.syncAll({}, () => {});
    renderRoute("/courses", { api });
    const row = await courseRow(/DEMO404/);
    expect(row).toHaveTextContent("Demo Canvas · synced 1 minute ago");
    expect(row).not.toHaveTextContent("materials not read yet");
    expect(row).not.toHaveTextContent("deadlines checked");
  });
});
