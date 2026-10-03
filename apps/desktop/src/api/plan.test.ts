import { expect, it } from "vitest";
import en from "@/i18n/locales/en/plan.json";
import zh from "@/i18n/locales/zh-CN/plan.json";
import { PLAN_WARNING_CODES, UNSCHEDULED_REASONS } from "./plan";

// The facade's plan codes are shown in words: every one needs them in every language.
it("words every unscheduled reason and plan warning in en and zh-CN", () => {
  for (const messages of [en, zh]) {
    for (const reason of UNSCHEDULED_REASONS) {
      expect(messages.unscheduled.reason[reason], reason).toBeTruthy();
    }
    const warnings = messages.warnings as Record<string, string>;
    for (const code of PLAN_WARNING_CODES) {
      expect(warnings[`${code}_one`], code).toBeTruthy();
      expect(warnings[`${code}_other`], code).toBeTruthy();
    }
  }
});
