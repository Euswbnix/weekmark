import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { toast } from "sonner";
import { afterEach, beforeEach, vi } from "vitest";
import { useNoteRunStore } from "@/features/weekly-note/useWeeklyNote";
import i18n, { initI18n } from "@/i18n";
import { useCodexStore } from "@/stores/codex";
import { useSyncStore } from "@/stores/sync";
import { useUiStore } from "@/stores/ui";
import { useUpdateStore } from "@/stores/updates";

initI18n("en");
// findBy* and waitFor wait longer on CI runners (vite.config.ts).
configure({ asyncUtilTimeout: Number(import.meta.env.VITE_TEST_ASYNC_TIMEOUT ?? 1000) });

// jsdom lacks these browser APIs that Radix UI and our theme code use.
if (!window.matchMedia) {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
};
Element.prototype.scrollIntoView ??= () => {};
Element.prototype.hasPointerCapture ??= () => false;
Element.prototype.releasePointerCapture ??= () => {};
// Sonner's toasts (e.g. clicking a toast's Undo) capture the pointer on pointerdown.
Element.prototype.setPointerCapture ??= () => {};

const initialUi = useUiStore.getState();

beforeEach(async () => {
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: vi.fn().mockResolvedValue(undefined) },
  });
  if (i18n.language !== "en") await i18n.changeLanguage("en");
});

afterEach(() => {
  // Sonner replays still-visible toasts into the next <Toaster/>; don't leak them across tests.
  toast.dismiss();
  cleanup();
  localStorage.clear();
  useUiStore.setState(initialUi, true);
  useSyncStore.getState().reset();
  useUpdateStore.getState().reset();
  useCodexStore.setState({ install: { phase: "idle" } });
  useNoteRunStore.setState({ run: { phase: "idle" }, automaticProblem: null });
});
