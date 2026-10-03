import { type RefObject, useEffect, useRef } from "react";

/**
 * Moves focus to the element once it mounts: where a run's progress, result or outcome replaces
 * the button that started it, so keyboard and screen-reader users aren't left on the page body.
 */
export function useFocusOnMount<T extends HTMLElement>(enabled = true): RefObject<T | null> {
  const ref = useRef<T>(null);
  // Only as it mounts: `enabled` says whether this mount moves the focus (a run the app started
  // on its own moves nothing).
  const focusNow = useRef(enabled);
  useEffect(() => {
    if (focusNow.current) ref.current?.focus();
  }, []);
  return ref;
}
