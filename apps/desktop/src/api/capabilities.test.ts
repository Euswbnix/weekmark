// The webview may never install or redirect updates itself: the updater is driven from Rust
// commands only (M0.4). Guard the capability file against an `updater:*` permission creeping in.

import { expect, it } from "vitest";
import capabilities from "../../src-tauri/capabilities/default.json";

it("grants the webview no updater permission", () => {
  const ids = capabilities.permissions.map((p) => (typeof p === "string" ? p : p.identifier));
  expect(ids.length).toBeGreaterThan(0);
  expect(ids.filter((id) => id.startsWith("updater"))).toEqual([]);
});

// Course material files are opened and revealed Rust-side (open_material / reveal_material), after
// the facade checked the file: the webview gets no permission to open or reveal local paths.
it("grants the webview no way to open or reveal local files", () => {
  const ids = capabilities.permissions.map((p) => (typeof p === "string" ? p : p.identifier));
  expect(ids).not.toContain("opener:default");
  expect(ids.filter((id) => /^opener:allow-(open-path|reveal-item-in-dir)/.test(id))).toEqual([]);
});
