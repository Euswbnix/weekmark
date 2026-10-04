import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useMinute } from "./useMinute";

afterEach(() => {
  vi.useRealTimers();
});

describe("useMinute", () => {
  it("moves on with the clock while something reads it, and stops when nothing does", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 9, 5, 9, 0, 30));
    const first = renderHook(() => useMinute());
    const second = renderHook(() => useMinute());
    const start = first.result.current;
    expect(second.result.current).toBe(start);

    act(() => vi.advanceTimersByTime(60_000));
    expect(first.result.current).toBe(start + 1);
    expect(second.result.current).toBe(start + 1);

    const stopped = vi.spyOn(globalThis, "clearInterval");
    first.unmount();
    expect(stopped).not.toHaveBeenCalled();
    second.unmount();
    expect(stopped).toHaveBeenCalledTimes(1);

    // A later reader starts from the present, not from where the last one stopped.
    vi.setSystemTime(new Date(2026, 9, 5, 10, 0, 30));
    const third = renderHook(() => useMinute());
    expect(third.result.current).toBe(start + 60);
    third.unmount();
  });
});
