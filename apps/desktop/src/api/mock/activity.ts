// What the mock is doing right now, like the facade's registry (`App::activity`): syncs,
// downloads, a Codex install and model runs register while they run.

import type { ActivityItem, ActivityKind } from "../types";

export interface MockActivity {
  /** Run `work`, listed as `kind` until it ends, fails or stops. */
  during<T>(
    kind: ActivityKind,
    ids: { source_id?: string; generation_id?: string },
    work: () => Promise<T>,
  ): Promise<T>;
  /** The running work, oldest first. */
  items(): ActivityItem[];
}

export function createMockActivity(now: () => Date): MockActivity {
  const running = new Map<number, ActivityItem>();
  let next = 0;
  return {
    async during(kind, ids, work) {
      const id = next++;
      running.set(id, {
        kind,
        source_id: ids.source_id ?? null,
        generation_id: ids.generation_id ?? null,
        started_at: now().toISOString(),
      });
      try {
        return await work();
      } finally {
        running.delete(id);
      }
    },
    items: () => [...running.values()],
  };
}
