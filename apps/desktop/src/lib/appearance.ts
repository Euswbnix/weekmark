// Appearance contexts on <html> for the Lamplight tokens (docs/design/macos-shell.md §8,
// design/tokens/README.md): data-platform and data-backdrop are fixed for the window's life and
// set before the first render; data-transparency, data-contrast and data-window-active follow the
// student's settings and the window's focus (app/useAppearance.ts).

export type Platform = "macos" | "windows" | "linux";
export type Backdrop = "none" | "mica";

declare global {
  interface Window {
    /** What the Rust side built the window with (its initialization script sets it). */
    __PAGELAMP_WINDOW__?: { backdrop?: string; hidden?: boolean };
  }
}

export function platformFromUserAgent(userAgent: string): Platform {
  if (/Windows/i.test(userAgent)) return "windows";
  if (/Macintosh|Mac OS X/i.test(userAgent)) return "macos";
  return "linux";
}

/**
 * The window's fixed contexts. In mock mode, `?platform=` and `?backdrop=` simulate another
 * system (screenshots, tests); a real window only gets Mica when Rust built it with Mica.
 */
export function staticAppearance(env: {
  mock: boolean;
  search: string;
  userAgent: string;
  windowBackdrop?: string;
}): { platform: Platform; backdrop: Backdrop } {
  const params = new URLSearchParams(env.search);
  const asked = env.mock ? params.get("platform") : null;
  const platform: Platform =
    asked === "macos" || asked === "windows" || asked === "linux"
      ? asked
      : platformFromUserAgent(env.userAgent);
  const backdropAsked = env.mock ? params.get("backdrop") : env.windowBackdrop;
  // Mica exists only on Windows 11 (Rust decides; the page just follows).
  const backdrop: Backdrop = backdropAsked === "mica" && platform === "windows" ? "mica" : "none";
  return { platform, backdrop };
}

/**
 * Sets data-platform and data-backdrop; call once, before the first render. `simulateMica` (mock
 * mode) paints a Mica stand-in behind the page, since a browser has no Mica.
 */
export function applyStaticAppearance(
  root: HTMLElement,
  appearance: { platform: Platform; backdrop: Backdrop },
  simulateMica = false,
) {
  root.dataset.platform = appearance.platform;
  root.dataset.backdrop = appearance.backdrop;
  if (simulateMica && appearance.backdrop === "mica") root.dataset.micaSim = "";
}

/** The student's appearance preferences (stores/ui.ts). */
export interface AppearancePreferences {
  theme: "system" | "light" | "dark";
  transparency: "auto" | "reduced";
  contrast: "auto" | "more";
}

/**
 * The preferences on <html> before the first render (usePreferences and useAppearance keep them
 * in step afterwards), so the first frame already has the student's theme and transparency.
 */
export function applyPreferences(
  root: HTMLElement,
  prefs: AppearancePreferences,
  systemDark: boolean,
): void {
  const dark = prefs.theme === "dark" || (prefs.theme === "system" && systemDark);
  root.classList.toggle("dark", dark);
  root.style.colorScheme = dark ? "dark" : "light";
  root.dataset.transparency = prefs.transparency;
  root.dataset.contrast = prefs.contrast;
}

let startupDark: boolean | null = null;

/** Records the system's colour scheme at startup, before anything calls setTheme. */
export function recordStartupScheme(): boolean {
  startupDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  return startupDark;
}

/**
 * The native theme for "system" after an explicit light or dark this session. On Linux, tao
 * turns setTheme(null) into "prefer light" (overriding the portal's dark preference), so the
 * system value seen at startup goes back instead; tao's portal listener follows later changes.
 * macOS and Windows go back to following the system with null.
 */
export function systemNativeTheme(platform: string | undefined): "light" | "dark" | null {
  if (platform !== "linux") return null;
  return startupDark ? "dark" : "light";
}
