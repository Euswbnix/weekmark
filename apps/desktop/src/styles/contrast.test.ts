// Text contrast of the Lamplight tokens (docs/design/macos-shell.md §8, §10), computed from the
// generated tokens.css itself: every context the app can be in (light/dark × normal/increased
// contrast × no backdrop/Mica) resolves its --pl-* values the way the cascade does, and text must
// reach WCAG AA on every surface it sits on, glass and Mica included.

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(join(process.cwd(), "src/styles/tokens.css"), "utf8");

/** Declarations of one top-level block, e.g. `:root.dark` (not the @media copies). */
function block(selector: string): Record<string, string> {
  const start = css.indexOf(`\n${selector} {\n`);
  if (start < 0) throw new Error(`no block ${selector}`);
  const body = css.slice(start + selector.length + 4, css.indexOf("\n}\n", start));
  const decls: Record<string, string> = {};
  for (const m of body.matchAll(/--(pl-[a-z0-9-]+):\s*([^;]+);/g)) {
    decls[m[1] as string] = (m[2] as string).trim();
  }
  return decls;
}

type Ctx = { dark: boolean; more: boolean; mica: boolean };

/** The --pl-* values for a context, applying blocks in the cascade's order. */
function resolve(ctx: Ctx): (name: string) => string {
  const selectors = [":root"];
  if (ctx.dark) selectors.push(":root.dark");
  if (ctx.mica) selectors.push(':root[data-backdrop="mica"]');
  if (ctx.more) selectors.push(':root[data-contrast="more"]');
  if (ctx.dark && ctx.mica) selectors.push(':root.dark[data-backdrop="mica"]');
  if (ctx.dark && ctx.more) selectors.push(':root.dark[data-contrast="more"]');
  if (ctx.dark && ctx.mica && ctx.more)
    selectors.push(':root.dark[data-backdrop="mica"][data-contrast="more"]');
  const values: Record<string, string> = {};
  for (const s of selectors) Object.assign(values, block(s));
  const get = (name: string): string => {
    const v = values[name];
    if (!v) throw new Error(`--${name} unset`);
    const alias = /^var\(--(pl-[a-z0-9-]+)\)$/.exec(v);
    return alias ? get(alias[1] as string) : v;
  };
  return get;
}

type Rgba = [number, number, number, number];

type Oklab = [number, number, number, number];

/** An oklch() token as OKLab (L, a, b, alpha). */
function oklab(color: string): Oklab {
  const oklch = /^oklch\(([\d.]+) ([\d.]+) ([\d.]+)(?: \/ ([\d.]+))?\)$/.exec(color);
  if (!oklch) throw new Error(`not oklch: ${color}`);
  const [L, C, h, a] = [Number(oklch[1]), Number(oklch[2]), Number(oklch[3]), oklch[4]];
  return [
    L,
    C * Math.cos((h * Math.PI) / 180),
    C * Math.sin((h * Math.PI) / 180),
    a === undefined ? 1 : Number(a),
  ];
}

/** color-mix(in oklab, a p%, b) for opaque colours. */
function mixOklab(a: string, p: number, b: string): Rgba {
  const [x, y] = [oklab(a), oklab(b)];
  const w = p / 100;
  return fromOklab([
    x[0] * w + y[0] * (1 - w),
    x[1] * w + y[1] * (1 - w),
    x[2] * w + y[2] * (1 - w),
    1,
  ]);
}

/** CSS Color 4: OKLab → linear sRGB → sRGB (0…1), gamut-clipped. */
function fromOklab([L, A, B, alpha]: Oklab): Rgba {
  {
    const l = (L + 0.3963377774 * A + 0.2158037573 * B) ** 3;
    const m = (L - 0.1055613458 * A - 0.0638541728 * B) ** 3;
    const s = (L - 0.0894841775 * A - 1.291485548 * B) ** 3;
    const lin = [
      4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
      -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
      -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
    ];
    const enc = (x: number) => {
      const c = Math.min(1, Math.max(0, x));
      return c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055;
    };
    return [enc(lin[0] ?? 0), enc(lin[1] ?? 0), enc(lin[2] ?? 0), alpha];
  }
}

/** A token value as sRGB (oklch, rgb or hex). */
function parse(color: string): Rgba {
  if (color.startsWith("oklch(")) return fromOklab(oklab(color));
  const rgb = /^rgb\(([\d.]+) ([\d.]+) ([\d.]+)(?: \/ ([\d.]+))?\)$/.exec(color);
  if (rgb) {
    return [
      Number(rgb[1]) / 255,
      Number(rgb[2]) / 255,
      Number(rgb[3]) / 255,
      rgb[4] === undefined ? 1 : Number(rgb[4]),
    ];
  }
  const hex = /^#([0-9a-f]{6})$/i.exec(color);
  if (hex) {
    const n = Number.parseInt(hex[1] as string, 16);
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255, 1];
  }
  throw new Error(`can't parse ${color}`);
}

/** Source-over in sRGB, as browsers composite. */
function over(top: Rgba, bottom: Rgba): Rgba {
  const a = top[3];
  return [
    top[0] * a + bottom[0] * (1 - a),
    top[1] * a + bottom[1] * (1 - a),
    top[2] * a + bottom[2] * (1 - a),
    1,
  ];
}

function luminance([r, g, b]: Rgba): number {
  const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(text: Rgba, background: Rgba): number {
  const fg = text[3] < 1 ? over(text, background) : text;
  const [a, b] = [luminance(fg), luminance(background)].sort((x, y) => y - x) as [number, number];
  return (a + 0.05) / (b + 0.05);
}

const CONTEXTS: (Ctx & { name: string })[] = [];
for (const dark of [false, true]) {
  for (const more of [false, true]) {
    for (const mica of [false, true]) {
      CONTEXTS.push({
        dark,
        more,
        mica,
        name: `${dark ? "dark" : "light"}${more ? " · more" : ""}${mica ? " · mica" : ""}`,
      });
    }
  }
}

/** index.css: --destructive = color-mix(in oklab, status-danger 75%, text-primary). */
const DANGER_TEXT_MIX = 75;
/** index.css: --warning = color-mix(in oklab, status-warning 90%, text-primary) (glyphs only). */
const WARNING_MIX = 90;

/**
 * index.css: --input is 50 % ink; with increased contrast it is text.secondary (opaque).
 * Returned as the colour drawn over a surface.
 */
function INPUT_INK(ctx: Ctx, t: (name: string) => string): Rgba {
  if (ctx.more) return parse(t("pl-color-text-secondary"));
  const ink = parse(t("pl-color-text-primary"));
  return [ink[0], ink[1], ink[2], 0.5];
}

/** The dialog overlay (ui/dialog.tsx, alert-dialog.tsx: bg-black/10). */
const DIM: Rgba = [0, 0, 0, 0.1];

/** Mica approximations used by the spec's measurements (§8): #F3F3F3 / #202020. */
const MICA = { light: parse("#f3f3f3"), dark: parse("#202020") };

describe("Lamplight token contrast (WCAG AA)", () => {
  it.each(CONTEXTS)("$name: text on every surface", (ctx) => {
    const t = resolve(ctx);
    const text = {
      primary: parse(t("pl-color-text-primary")),
      secondary: parse(t("pl-color-text-secondary")),
    };
    const content = parse(t("pl-color-surface-content"));
    const surfaces: Record<string, Rgba> = {
      content,
      raised: parse(t("pl-color-surface-raised")),
      sidebar: parse(t("pl-color-surface-sidebar")),
      popover: parse(t("pl-color-surface-popover")),
    };
    // Glass: its tint over what is usually behind it (the content paper), blur aside.
    surfaces.glass = over(parse(t("pl-color-glass-tint")), content);
    // index.css's quiet fills: a 5–7 % wash of ink (notes, skeletons, hover) on a raised card.
    const ink = parse(t("pl-color-text-primary"));
    surfaces["ink wash on raised"] = over(
      [ink[0], ink[1], ink[2], 0.07],
      parse(t("pl-color-surface-raised")),
    );
    if (ctx.mica) {
      const mica = ctx.dark ? MICA.dark : MICA.light;
      surfaces["mica card"] = over(parse(t("pl-color-mica-layer-fill")), mica);
      surfaces["glass on mica"] = over(parse(t("pl-color-glass-tint")), mica);
    }
    const report: string[] = [];
    for (const [surface, bg] of Object.entries(surfaces)) {
      for (const [role, fg] of Object.entries(text)) {
        const ratio = contrast(fg, bg);
        if (ratio < 4.5) report.push(`${role} on ${surface}: ${ratio.toFixed(2)}`);
      }
    }
    expect(report).toEqual([]);
  });

  // The pairs the app actually renders (PR #4 review 5, 9, 14, 16), on every surface they sit
  // on: paper, callouts, popovers, glass, and the 5 % ink hover/selection wash on top of them.
  it.each(CONTEXTS)("$name: status text, controls and focus on their real surfaces", (ctx) => {
    const t = resolve(ctx);
    const content = parse(t("pl-color-surface-content"));
    const raised = parse(t("pl-color-surface-raised"));
    const ink = parse(t("pl-color-text-primary"));
    const wash = (base: Rgba, alpha = 0.05): Rgba => over([ink[0], ink[1], ink[2], alpha], base);
    const surfaces: Record<string, Rgba> = {
      content,
      raised,
      popover: parse(t("pl-color-surface-popover")),
      glass: over(parse(t("pl-color-glass-tint")), content),
      // A glass dialog over the dialog overlay (black 10 %) over the page.
      "dialog glass over dim": over(parse(t("pl-color-glass-tint")), over(DIM, content)),
      "hover on content": wash(content),
      "hover on raised": wash(raised),
      "selected row on content": wash(content, 0.09),
    };
    if (ctx.mica) {
      const card = over(parse(t("pl-color-mica-layer-fill")), ctx.dark ? MICA.dark : MICA.light);
      surfaces["mica card"] = card;
      // Callouts on Mica are a 5 % ink wash (content.css), hovers another 5 % on top.
      surfaces["mica callout"] = wash(card);
      surfaces["dialog glass over dim mica"] = over(
        parse(t("pl-color-glass-tint")),
        over(DIM, card),
      );
      surfaces["hover on mica callout"] = wash(wash(card));
    }
    // index.css: error text is the danger colour mixed a quarter of the way to ink.
    const dangerText = mixOklab(
      t("pl-color-status-danger"),
      DANGER_TEXT_MIX,
      t("pl-color-text-primary"),
    );
    const secondary = parse(t("pl-color-text-secondary"));
    const accent = parse(t("pl-color-accent"));
    const report: string[] = [];
    const need = (what: string, ratio: number, min: number) => {
      if (ratio < min) report.push(`${what}: ${ratio.toFixed(2)} < ${min}`);
    };
    for (const [surface, bg] of Object.entries(surfaces)) {
      need(`error text on ${surface}`, contrast(dangerText, bg), 4.5);
      // Inactive tabs and hints sit on the muted (5 % ink) fills.
      need(`secondary text on ${surface}`, contrast(secondary, bg), 4.5);
      // Control boundaries (WCAG 1.4.11): unchecked radios, checkboxes, switch tracks.
      need(`input border on ${surface}`, contrast(over([...INPUT_INK(ctx, t)], bg), bg), 3);
      // Focus: a solid accent outline or ring.
      need(`focus ring on ${surface}`, contrast(accent, bg), 3);
      // Status colours only on glyphs (non-text, 3:1); words stay ink.
      const glyphs: Record<string, Rgba> = {
        success: parse(t("pl-color-status-success")),
        warning: mixOklab(t("pl-color-status-warning"), WARNING_MIX, t("pl-color-text-primary")),
        danger: dangerText,
      };
      for (const [status, glyph] of Object.entries(glyphs)) {
        need(`${status} glyph on ${surface}`, contrast(glyph, bg), 3);
      }
    }
    expect(report).toEqual([]);
  });

  it.each(CONTEXTS)("$name: focus ring and lamp rule stand out (3:1)", (ctx) => {
    const t = resolve(ctx);
    const content = parse(t("pl-color-surface-content"));
    // The ring is the brand accent (index.css); tokens' accent is the default brand.
    expect(contrast(parse(t("pl-color-accent")), content)).toBeGreaterThanOrEqual(3);
    expect(contrast(parse(t("pl-color-lamp-rule")), content)).toBeGreaterThanOrEqual(3);
  });
});
