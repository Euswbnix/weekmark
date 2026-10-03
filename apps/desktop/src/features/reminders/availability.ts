import { mockScreensEnabled } from "@/lib/features";

/**
 * The reminders screens and delivery (M3): Settings → Reminders, onboarding's "Remind me", the
 * catch-up card and the notifications. src-tauri has the commands (tray, login item, ticker);
 * real builds keep the screens off until the release turns them on (beta.1), like
 * AI_SETUP_ENABLED. VITE_PAGELAMP_REMINDERS_UI=1 turns them on in a real build for testing.
 * `?shipped` hides them in the mock (lib/features.ts).
 */
export function remindersUiEnabled(env: Record<string, unknown>, search?: string): boolean {
  return mockScreensEnabled(env, search) || env.VITE_PAGELAMP_REMINDERS_UI === "1";
}

export const REMINDERS_UI = remindersUiEnabled(import.meta.env);
