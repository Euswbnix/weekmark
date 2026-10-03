// The selected tab and week live in the URL (?tab=policy&week=3), so they survive reloads and
// can be linked to directly. `replace` keeps them out of the back-button history.

import { useSearchParams } from "react-router";
import { AI_SETUP_ENABLED } from "@/lib/features";

const ALL_TABS = ["week", "explain", "timeline", "deadlines", "policy", "settings"] as const;
export type CourseTab = (typeof ALL_TABS)[number];

/** The tabs shown: Explain (M3) only where the AI screens are built (AI_SETUP_ENABLED). */
export const COURSE_TABS: readonly CourseTab[] = AI_SETUP_ENABLED
  ? ALL_TABS
  : ALL_TABS.filter((tab) => tab !== "explain");

export function isCourseTab(value: unknown): value is CourseTab {
  return COURSE_TABS.includes(value as CourseTab);
}

function useParam(name: string) {
  const [params, setParams] = useSearchParams();
  const set = (value: string | null) =>
    setParams(
      (prev) => {
        const next = new URLSearchParams(prev);
        if (value === null) next.delete(name);
        else next.set(name, value);
        return next;
      },
      { replace: true },
    );
  return [params.get(name), set] as const;
}

/** `?tab=` — defaults to "week" when missing or unknown. */
export function useCourseTab(): [CourseTab, (tab: CourseTab) => void] {
  const [value, set] = useParam("tab");
  return [isCourseTab(value) ? value : "week", (tab) => set(tab)];
}

/** `?week=` — null means "the course's current week". */
export function useSelectedWeek(): [number | null, (week: number | null) => void] {
  const [value, set] = useParam("week");
  // Teaching weeks start at 1; anything else in the URL means "current week".
  const week = value !== null && /^[1-9]\d{0,2}$/.test(value) ? Number(value) : null;
  return [week, (next) => set(next === null ? null : String(next))];
}
