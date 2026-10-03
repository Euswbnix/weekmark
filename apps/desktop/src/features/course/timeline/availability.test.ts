import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { DATES_V2_UI, datesV2UiEnabled } from "./availability";

// Vitest runs from apps/desktop (vite.config.ts), where the .env files live.
const desktopDir = process.cwd();

describe("the dates form v2 switch", () => {
  it("is off in a production build and on in mock mode", () => {
    expect(datesV2UiEnabled({ MODE: "production", PROD: true })).toBe(false);
    expect(datesV2UiEnabled({ MODE: "development", VITE_API: "tauri" })).toBe(false);
    expect(datesV2UiEnabled({ MODE: "mock", VITE_API: "mock" })).toBe(true);
    expect(datesV2UiEnabled({ MODE: "production", VITE_PAGELAMP_DATES_V2_UI: "1" })).toBe(true);
    // The removal switch no longer turns it on.
    expect(datesV2UiEnabled({ MODE: "production", VITE_PAGELAMP_REMOVAL_UI: "1" })).toBe(false);
    expect(DATES_V2_UI).toBe(true);
  });

  it("is off in the mock as the release ships it (?shipped)", () => {
    expect(datesV2UiEnabled({ VITE_API: "mock" }, "?shipped")).toBe(false);
    expect(datesV2UiEnabled({ VITE_API: "mock", VITE_PAGELAMP_SHIPPED: "1" }, "")).toBe(false);
    expect(datesV2UiEnabled({ VITE_API: "mock" }, "?scenario=demo")).toBe(true);
  });

  it("isn't turned on by any env file a real build reads", () => {
    const envFiles = readdirSync(desktopDir).filter((f) => f.startsWith(".env"));
    for (const file of envFiles.filter((f) => f !== ".env.mock")) {
      const text = readFileSync(join(desktopDir, file), "utf8");
      expect({ file, on: /^\s*VITE_PAGELAMP_DATES_V2_UI\s*=\s*1/m.test(text) }).toEqual({
        file,
        on: false,
      });
    }
  });
});
