import { useSyncExternalStore } from "react";

const MINUTE = 60_000;

// One timer for every reader, and none while nothing reads it.
const listeners = new Set<() => void>();
let timer: ReturnType<typeof setInterval> | null = null;
let minute = Math.floor(Date.now() / MINUTE);

function tick() {
  const next = Math.floor(Date.now() / MINUTE);
  if (next === minute) return;
  minute = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  if (timer === null) {
    // The stored minute is as old as the last reader.
    minute = Math.floor(Date.now() / MINUTE);
    timer = setInterval(tick, MINUTE);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  };
}

/**
 * Re-renders its caller once a minute, so "Synced 5 minutes ago" keeps counting while PageLamp
 * sits open. Returns the current minute (minutes since the epoch); read the time itself with
 * `new Date()`.
 */
export function useMinute(): number {
  return useSyncExternalStore(subscribe, () => minute);
}
