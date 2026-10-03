/**
 * Mock mode shows the screens whose commands don't exist yet (AI_SETUP_ENABLED here, REMOVAL_UI,
 * CALENDAR_UI), so they can be built and tested. `?shipped` on a mock URL, or
 * VITE_PAGELAMP_SHIPPED=1, shows the mock as the release ships instead: screenshots for the owner
 * use it unless they show unreleased screens on purpose (scripts/shoot-lamplight.mjs).
 */
export function mockScreensEnabled(
  env: Record<string, unknown>,
  search: string = typeof location === "undefined" ? "" : location.search,
): boolean {
  if (env.VITE_API !== "mock") return false;
  return env.VITE_PAGELAMP_SHIPPED !== "1" && !new URLSearchParams(search).has("shipped");
}

/**
 * The AI screens: M1's setup (Settings → AI models and usage, question (b) on the course's policy
 * tab) and what M3 writes with it (Plan, the Explain tab, ticking plan items off, the output
 * language, onboarding's offer). They run against the mock until src-tauri has the AI commands
 * (plan M1, alpha.2), so a real build (alpha.1) doesn't show screens whose commands don't exist
 * yet. Set to true in the commit that wires the commands.
 */
export const AI_SETUP_ENABLED = mockScreensEnabled(import.meta.env);
