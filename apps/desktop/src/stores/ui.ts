// UI preferences (theme, language, list toggles). Persisted to localStorage because they are
// harmless conveniences. NEVER put tokens, feed URLs or course content in this store.

import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";
import type { Locale } from "@/brand";

export type ThemePreference = "system" | "light" | "dark";
/** "reduced" = solid backgrounds instead of glass (in addition to the system setting). */
export type TransparencyPreference = "auto" | "reduced";
/** "more" = Increase contrast (Linux, where the web view can't see the system setting). */
export type ContrastPreference = "auto" | "more";

interface UiState {
  theme: ThemePreference;
  /** null = the brand's default language. */
  locale: Locale | null;
  showHiddenCourses: boolean;
  /** "Past courses" group expanded on the course list (collapsed by default). */
  showPastCourses: boolean;
  /** The student dismissed onboarding without adding a source. */
  onboardingSkipped: boolean;
  /** When the student confirmed the AI disclosure (ISO instant), null = not yet. */
  aiDisclosureAcknowledgedAt: string | null;
  /** Onboarding's "Let PageLamp write plans and explanations" was answered: never shown again. */
  aiOfferAnswered: boolean;
  transparency: TransparencyPreference;
  contrast: ContrastPreference;
  setTheme: (theme: ThemePreference) => void;
  setLocale: (locale: Locale) => void;
  setShowHiddenCourses: (show: boolean) => void;
  setShowPastCourses: (show: boolean) => void;
  setOnboardingSkipped: (skipped: boolean) => void;
  /** Tick/untick "I understand" under the AI disclosure. */
  setAiDisclosureAcknowledged: (acknowledged: boolean) => void;
  setAiOfferAnswered: () => void;
  setTransparency: (transparency: TransparencyPreference) => void;
  setContrast: (contrast: ContrastPreference) => void;
}

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      theme: "system",
      locale: null,
      showHiddenCourses: false,
      showPastCourses: false,
      onboardingSkipped: false,
      aiDisclosureAcknowledgedAt: null,
      aiOfferAnswered: false,
      transparency: "auto",
      contrast: "auto",
      setTheme: (theme) => set({ theme }),
      setLocale: (locale) => set({ locale }),
      setShowHiddenCourses: (showHiddenCourses) => set({ showHiddenCourses }),
      setShowPastCourses: (showPastCourses) => set({ showPastCourses }),
      setOnboardingSkipped: (onboardingSkipped) => set({ onboardingSkipped }),
      setAiDisclosureAcknowledged: (acknowledged) =>
        set({ aiDisclosureAcknowledgedAt: acknowledged ? new Date().toISOString() : null }),
      setAiOfferAnswered: () => set({ aiOfferAnswered: true }),
      setTransparency: (transparency) => set({ transparency }),
      setContrast: (contrast) => set({ contrast }),
    }),
    {
      name: "pagelamp.ui",
      version: 1,
      storage: createJSONStorage(() => localStorage),
      // Explicit allow-list: only these keys are ever written to disk.
      partialize: (s) => ({
        theme: s.theme,
        locale: s.locale,
        showHiddenCourses: s.showHiddenCourses,
        showPastCourses: s.showPastCourses,
        onboardingSkipped: s.onboardingSkipped,
        aiDisclosureAcknowledgedAt: s.aiDisclosureAcknowledgedAt,
        aiOfferAnswered: s.aiOfferAnswered,
        transparency: s.transparency,
        contrast: s.contrast,
      }),
    },
  ),
);
