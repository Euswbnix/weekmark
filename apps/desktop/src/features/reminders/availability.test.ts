import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { REMINDERS_UI, remindersUiEnabled } from "./availability";

// Vitest runs from apps/desktop (vite.config.ts), where the .env files live.
const desktopDir = process.cwd();

describe("the reminders switch (M3)", () => {
  it("is off in a production build and on in mock mode", () => {
    expect(remindersUiEnabled({ MODE: "production", PROD: true })).toBe(false);
    expect(remindersUiEnabled({ MODE: "development", VITE_API: "tauri" })).toBe(false);
    expect(remindersUiEnabled({ MODE: "mock", VITE_API: "mock" })).toBe(true);
    expect(remindersUiEnabled({ MODE: "production", VITE_PAGELAMP_REMINDERS_UI: "1" })).toBe(true);
    // Tests run against the mock (vite.config.ts), so the screens are tested.
    expect(REMINDERS_UI).toBe(true);
  });

  it("is off in the mock as the release ships it (?shipped)", () => {
    expect(remindersUiEnabled({ VITE_API: "mock" }, "?shipped")).toBe(false);
    expect(remindersUiEnabled({ VITE_API: "mock" }, "?scenario=demo&shipped=&platform=macos")).toBe(
      false,
    );
    expect(remindersUiEnabled({ VITE_API: "mock", VITE_PAGELAMP_SHIPPED: "1" }, "")).toBe(false);
    expect(remindersUiEnabled({ VITE_API: "mock" }, "?scenario=demo")).toBe(true);
  });

  it("isn't turned on by any env file a real build reads", () => {
    const envFiles = readdirSync(desktopDir).filter((f) => f.startsWith(".env"));
    expect(envFiles).toContain(".env.mock");
    for (const file of envFiles.filter((f) => f !== ".env.mock")) {
      const text = readFileSync(join(desktopDir, file), "utf8");
      expect({ file, mock: /^\s*VITE_API\s*=\s*mock/m.test(text) }).toEqual({ file, mock: false });
      expect({ file, on: /^\s*VITE_PAGELAMP_REMINDERS_UI\s*=\s*1/m.test(text) }).toEqual({
        file,
        on: false,
      });
    }
  });
});
