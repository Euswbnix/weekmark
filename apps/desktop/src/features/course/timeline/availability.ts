import { mockScreensEnabled } from "@/lib/features";

/**
 * The course dates form v2 (breaks, the end of exams, a second part for full-year courses). It
 * runs against the mock until set_course_dates is real on main (feat/course-ai, alpha.2), so a
 * real build before that keeps the v1 form (first and last day of classes).
 * VITE_PAGELAMP_DATES_V2_UI=1 turns it on in a real build for testing. Delete this switch in the
 * commit that makes the form real on main. `?shipped` hides it in the mock (lib/features.ts).
 */
export function datesV2UiEnabled(env: Record<string, unknown>, search?: string): boolean {
  return mockScreensEnabled(env, search) || env.VITE_PAGELAMP_DATES_V2_UI === "1";
}

export const DATES_V2_UI = datesV2UiEnabled(import.meta.env);
