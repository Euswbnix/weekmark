// What's new in the mock: the facade's WHATS_NEW table (pagelamp-app updates.rs) and its
// topics_since, with semver's pre-release order (alpha.2 < alpha.10 < beta.1 < 0.3.0).

import type { WhatsNewTopic } from "../types";

/** Each topic with the version that introduced it, in the order the sheet lists them. */
export const WHATS_NEW: readonly (readonly [WhatsNewTopic, string])[] = [
  ["update_check", "0.3.0-alpha.1"],
  ["course_weeks", "0.3.0-alpha.1"],
  ["course_removal", "0.3.0-alpha.2"],
  ["syllabus_reading", "0.3.0-alpha.3"],
  ["ai_writing", "0.3.0-beta.1"],
  ["reminders", "0.3.0-beta.1"],
];

/** The topics introduced after `since` (null: from 0.1, which recorded no version: all). */
export function topicsSince(since: string | null): WhatsNewTopic[] {
  return WHATS_NEW.filter(
    ([, introduced]) => since === null || compareVersions(since, introduced) < 0,
  ).map(([topic]) => topic);
}

function parse(version: string) {
  const dash = version.indexOf("-");
  const core = (dash < 0 ? version : version.slice(0, dash)).split(".").map(Number);
  const pre = dash < 0 ? [] : version.slice(dash + 1).split(".");
  return { core, pre };
}

/** Semver precedence: < 0 when `a` comes before `b`. */
export function compareVersions(a: string, b: string): number {
  const x = parse(a);
  const y = parse(b);
  for (let i = 0; i < 3; i++) {
    const diff = (x.core[i] ?? 0) - (y.core[i] ?? 0);
    if (diff !== 0) return diff;
  }
  // A pre-release comes before its release.
  if (x.pre.length === 0 || y.pre.length === 0) return y.pre.length - x.pre.length;
  for (let i = 0; i < Math.max(x.pre.length, y.pre.length); i++) {
    const p = x.pre[i];
    const q = y.pre[i];
    if (p === undefined) return -1;
    if (q === undefined) return 1;
    if (p === q) continue;
    const numeric = /^\d+$/;
    if (numeric.test(p) && numeric.test(q)) return Number(p) - Number(q);
    // Numeric identifiers come before alphanumeric ones.
    if (numeric.test(p)) return -1;
    if (numeric.test(q)) return 1;
    return p < q ? -1 : 1;
  }
  return 0;
}

/** startup_tasks.whats_new for an upgrader from `since`. */
export function whatsNewSince(since: string | null): {
  since: string | null;
  topics: WhatsNewTopic[];
} {
  return { since, topics: topicsSince(since) };
}
