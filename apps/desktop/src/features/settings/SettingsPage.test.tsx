import { screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ApiError } from "@/api/errors";
import { createMockApi } from "@/api/mock";
import { MOCK_APP_VERSION } from "@/api/mock/fixtures";
import { brand, localized } from "@/brand";
import { REMINDERS_UI } from "@/features/reminders/availability";
import i18n from "@/i18n";
import { useUiStore } from "@/stores/ui";
import { renderRoute } from "@/test/render";

function mockApi(options: Parameters<typeof createMockApi>[0] = {}) {
  return createMockApi({ latencyMs: 0, syncStepMs: 0, ...options });
}

/** The settings card whose h2 is `title`. */
async function section(title: string) {
  const heading = await screen.findByRole("heading", { level: 2, name: title });
  const region = heading.closest("section");
  if (!region) throw new Error(`no section for ${title}`);
  return region;
}

describe("SettingsPage", () => {
  it("has one h1 and a card per section", async () => {
    renderRoute("/settings");
    expect(await screen.findByRole("heading", { level: 1, name: "Settings" })).toBeInTheDocument();
    expect(screen.getAllByRole("heading", { level: 1 })).toHaveLength(1);
    for (const name of ["Appearance", "Your data", "Privacy", "Help & feedback", "About"]) {
      expect(screen.getByRole("region", { name })).toBeInTheDocument();
    }
  });

  it("changes the theme and applies the dark class", async () => {
    const { user } = renderRoute("/settings");
    const theme = await screen.findByRole("radiogroup", { name: "Theme" });
    expect(within(theme).getByRole("radio", { name: "System" })).toHaveAttribute(
      "aria-checked",
      "true",
    );

    await user.click(within(theme).getByRole("radio", { name: "Dark" }));
    expect(useUiStore.getState().theme).toBe("dark");
    expect(document.documentElement).toHaveClass("dark");
    expect(within(theme).getByRole("radio", { name: "Dark" })).toHaveAttribute(
      "aria-checked",
      "true",
    );

    // Clicking the active option again must not clear the choice.
    await user.click(within(theme).getByRole("radio", { name: "Dark" }));
    expect(useUiStore.getState().theme).toBe("dark");

    await user.click(within(theme).getByRole("radio", { name: "Light" }));
    expect(useUiStore.getState().theme).toBe("light");
    expect(document.documentElement).not.toHaveClass("dark");
  });

  it("lets you pick a theme with the keyboard", async () => {
    const { user } = renderRoute("/settings");
    const theme = await screen.findByRole("radiogroup", { name: "Theme" });
    within(theme).getByRole("radio", { name: "System" }).focus();
    await user.keyboard("{ArrowRight}");
    const light = within(theme).getByRole("radio", { name: "Light" });
    expect(light).toHaveFocus();
    await user.keyboard(" ");
    expect(useUiStore.getState().theme).toBe("light");
    expect(light).toHaveAttribute("aria-checked", "true");
  });

  it("switches the language to Chinese and re-renders the headings", async () => {
    const { user } = renderRoute("/settings");
    await user.click(await screen.findByRole("combobox", { name: "Language" }));
    await user.click(await screen.findByRole("option", { name: "简体中文" }));

    expect(useUiStore.getState().locale).toBe("zh-CN");
    expect(await screen.findByRole("heading", { level: 1, name: "设置" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 2, name: "外观" })).toBeInTheDocument();
    const language = screen.getByRole("combobox", { name: "语言" });
    expect(language).toHaveTextContent("简体中文");
    // The chosen language's own name is marked up in that language for screen readers.
    expect(within(language).getByText("简体中文")).toHaveAttribute("lang", "zh-CN");
    expect(document.documentElement.lang).toBe("zh-CN");
    expect(i18n.language).toBe("zh-CN");
  });

  it("shows no placeholders for unfinished features", async () => {
    renderRoute("/settings");
    await section("Appearance");
    // Appearance ▸ Reduce transparency (Lamplight, §8), Updates ▸ automatic checks (M0.4), and
    // Reminders (M3) where it is built: off by default, so only its "Remind me" switch.
    const switches = ["Reduce transparency", "Check for updates automatically"];
    if (REMINDERS_UI) switches.push("Keep PageLamp in the tray and start it at login");
    else expect(screen.queryByRole("region", { name: "Reminders" })).toBeNull();
    for (const name of switches) expect(await screen.findByRole("switch", { name })).toBeVisible();
    expect(screen.queryAllByRole("switch")).toHaveLength(switches.length);
    expect(screen.queryByText("Coming soon")).toBeNull();
  });

  it("shows where the data lives and what is stored", async () => {
    const api = mockApi();
    const status = await api.status();
    const { user } = renderRoute("/settings", { api });
    const data = await section("Your data");

    expect(await within(data).findByText(status.data_dir)).toBeInTheDocument();
    expect(within(data).getByText(status.db_path)).toBeInTheDocument();
    const readable = within(data).getByText("Readable by your AI app").closest("div");
    expect(readable).toHaveTextContent(String(status.counts.indexed_materials));
    const hidden = within(data).getByText("Hidden courses").closest("div");
    expect(hidden).toHaveTextContent(String(status.counts.hidden_courses));

    const writeText = vi.spyOn(navigator.clipboard, "writeText");
    await user.click(within(data).getByRole("button", { name: "Copy data folder path" }));
    expect(writeText).toHaveBeenCalledWith(status.data_dir);
    expect(within(data).queryByText("Nothing synced yet.")).not.toBeInTheDocument();
  });

  it("opens the data folder with the desktop helper", async () => {
    const base = mockApi();
    const revealDataDir = vi.fn(base.revealDataDir);
    const { user } = renderRoute("/settings", { api: { ...base, revealDataDir } });
    const data = await section("Your data");
    await user.click(await within(data).findByRole("button", { name: "Show folder" }));
    expect(revealDataDir).toHaveBeenCalledTimes(1);
  });

  it("tells the student when the folder can't be opened", async () => {
    const base = mockApi();
    const revealDataDir = vi.fn().mockRejectedValue(new ApiError("not_found", "missing"));
    const { user } = renderRoute("/settings", { api: { ...base, revealDataDir } });
    const data = await section("Your data");
    await user.click(await within(data).findByRole("button", { name: "Show folder" }));
    expect(await screen.findByText("Couldn't open the folder")).toBeInTheDocument();
    expect(screen.getByText(i18n.t("errors.not_found"))).toBeInTheDocument();
  });

  it("points to Sources when nothing has been synced", async () => {
    renderRoute("/settings", { scenario: "empty" });
    const data = await section("Your data");
    expect(await within(data).findByText("Nothing synced yet.")).toBeInTheDocument();
    expect(within(data).getByRole("link", { name: "Add a source" })).toHaveAttribute(
      "href",
      "/sources",
    );
  });

  it("shows a loading placeholder while the status loads", async () => {
    const base = mockApi();
    const status = () => new Promise<Awaited<ReturnType<typeof base.status>>>(() => {});
    renderRoute("/settings", { api: { ...base, status } });
    const data = await section("Your data");
    expect(within(data).getByRole("status")).toHaveTextContent("Loading…");
    expect(within(data).queryByRole("button", { name: "Show folder" })).not.toBeInTheDocument();
  });

  it("shows an error with retry when the status can't load", async () => {
    const base = mockApi();
    const status = vi
      .fn()
      .mockRejectedValueOnce(new ApiError("internal", "database is locked"))
      .mockImplementation(base.status);
    const { user } = renderRoute("/settings", { api: { ...base, status } });
    const data = await section("Your data");
    const alert = await within(data).findByRole("alert");
    expect(within(alert).getByText("database is locked")).toBeInTheDocument();
    expect(within(await section("About")).getByText("Unavailable")).toBeInTheDocument();

    await user.click(within(alert).getByRole("button", { name: "Try again" }));
    const { data_dir } = await base.status();
    expect(await within(data).findByText(data_dir)).toBeInTheDocument();
  });

  it("shows the privacy promises and the AI disclosure", async () => {
    renderRoute("/settings");
    const privacy = await section("Privacy");
    expect(within(privacy).getByText(i18n.t("disclosure.full"))).toBeInTheDocument();
    expect(within(privacy).getByText(i18n.t("settings:privacy.keychain"))).toBeInTheDocument();
    expect(within(privacy).getByText(i18n.t("settings:privacy.readOnly"))).toBeInTheDocument();
  });

  it("says whether the AI disclosure was acknowledged and how to turn sharing off", async () => {
    renderRoute("/settings");
    const privacy = await section("Privacy");
    expect(
      within(privacy).getByText(
        "You haven't confirmed this yet. You'll be asked when you set up a source.",
      ),
    ).toBeInTheDocument();
    expect(within(privacy).getByText(/use the switch on its “AI policy” tab/)).toBeInTheDocument();

    useUiStore.getState().setAiDisclosureAcknowledged(true);
    expect(await within(privacy).findByText(/^You confirmed this on /)).toBeInTheDocument();
  });

  it("shows the version, license, tagline and the homepage link", async () => {
    renderRoute("/settings");
    const about = await section("About");
    expect(await within(about).findByText(MOCK_APP_VERSION)).toBeInTheDocument();
    expect(within(about).getByText("Apache-2.0")).toBeInTheDocument();
    expect(within(about).getByText(brand.productName)).toBeInTheDocument();
    expect(within(about).getByText(localized(brand.tagline, "en"))).toBeInTheDocument();

    // `help` and `issues` live in Help & feedback (features/diagnostics/diagnostics.test.tsx).
    const expected = (["homepage"] as const).filter((k) => brand.links[k]);
    const links = within(about).queryAllByRole("link");
    expect(links).toHaveLength(expected.length);
    for (const key of expected) {
      expect(
        within(about).getByRole("link", { name: i18n.t(`settings:about.${key}`) }),
      ).toHaveAttribute("href", brand.links[key]);
    }
  });
});
