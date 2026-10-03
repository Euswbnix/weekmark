// Rules jsdom can't lay out or render, pinned in the CSS itself (PR #4 review 4, 6, 11, 18).
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const read = (path: string) => readFileSync(join(process.cwd(), path), "utf8");
const shell = read("src/styles/shell.css");

/** The body of the first rule whose selector and at-rule context match. */
function block(css: string, pattern: RegExp): string {
  const match = css.match(pattern);
  expect(match, String(pattern)).not.toBeNull();
  return match?.[0] ?? "";
}

describe("shell.css", () => {
  it("keeps focused elements clear of the toolbar row and the accessory bar (WCAG 2.4.11)", () => {
    const content = block(shell, /\.pl-content \{[^}]*\}/);
    expect(content).toContain("scroll-padding-top: calc(var(--pl-size-toolbar-row) + 8px)");
    expect(content).toContain("scroll-padding-bottom: calc(var(--pl-size-accessory-height)");
  });

  it("paints Canvas under a Mica window in contrast themes", () => {
    const forced = block(
      shell,
      /@media \(forced-colors: active\) \{\s*:root\[data-backdrop="mica"\] body,[^}]*\}/,
    );
    expect(forced).toContain(".pl-sidebar");
    expect(forced).toContain(".pl-content");
    expect(forced).toContain("background: Canvas");
  });

  it("stops spinners turning with reduced motion; they fade instead", () => {
    const reduced = block(
      shell,
      /@media \(prefers-reduced-motion: reduce\) \{\s*\.animate-spin \{[^}]*\}/,
    );
    expect(reduced).toContain("animation: pl-busy-fade");
    expect(block(shell, /@keyframes pl-busy-fade \{[^}]*\}/)).toContain("opacity:");
  });

  it("turns popovers, menus and dialogs into crossfades with reduced motion", () => {
    const reduced = block(
      shell,
      /@media \(prefers-reduced-motion: reduce\) \{\s*:is\([^{]*\{[^}]*\}/,
    );
    for (const slot of [
      "popover-content",
      "select-content",
      "dialog-content",
      "alert-dialog-content",
    ]) {
      expect(reduced).toContain(`[data-slot="${slot}"]`);
    }
    expect(reduced).toContain("--tw-enter-scale: 1");
    expect(reduced).toContain("--tw-enter-translate-y: 0");
  });
});

describe("floating surfaces (popovers, menus, selects, dialogs)", () => {
  const components = [
    "src/components/ui/popover.tsx",
    "src/components/ui/select.tsx",
    "src/components/ui/dropdown-menu.tsx",
    "src/components/ui/dialog.tsx",
    "src/components/ui/alert-dialog.tsx",
  ];

  it("are glass that floats", () => {
    for (const file of components) expect(read(file)).toContain("pl-glass pl-float");
  });

  it("are opaque on Linux and on Windows without Mica, and over an open dialog", () => {
    const opaque = block(
      shell,
      /:root:not\(\[data-platform="macos"\]\):not\(\[data-backdrop="mica"\]\) \.pl-float,[^{]*\{[^}]*\}/,
    );
    expect(opaque).toContain(':root:has([data-slot="dialog-content"]');
    expect(opaque).toContain("backdrop-filter: none");
    expect(opaque).toContain("background: var(--pl-color-surface-popover)");
  });

  it("keep dialogs fixed in the window (.pl-glass sets position: relative)", () => {
    expect(block(shell, /\.pl-float:is\(\[data-slot="dialog-content"\][^{]*\{[^}]*\}/)).toContain(
      "position: fixed",
    );
  });
});

describe("dialogs", () => {
  it("never blur the window behind them (§8: no animated blur; transparency settings)", () => {
    for (const file of ["src/components/ui/dialog.tsx", "src/components/ui/alert-dialog.tsx"]) {
      expect(read(file)).not.toMatch(/backdrop-blur|backdrop-filter/);
    }
  });
});
