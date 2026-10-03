// Type-checks translation keys against the English files: `t("nav.courses")` compiles,
// `t("nav.typo")` does not. Add a line here when you add a new namespace file.

import "i18next";
import type ai from "./locales/en/ai.json";
import type calendar from "./locales/en/calendar.json";
import type chrome from "./locales/en/chrome.json";
import type common from "./locales/en/common.json";
import type connect from "./locales/en/connect.json";
import type course from "./locales/en/course.json";
import type courses from "./locales/en/courses.json";
import type explain from "./locales/en/explain.json";
import type onboarding from "./locales/en/onboarding.json";
import type plan from "./locales/en/plan.json";
import type proposals from "./locales/en/proposals.json";
import type reminders from "./locales/en/reminders.json";
import type removal from "./locales/en/removal.json";
import type settings from "./locales/en/settings.json";
import type sources from "./locales/en/sources.json";
import type updates from "./locales/en/updates.json";
import type weeklyNote from "./locales/en/weeklyNote.json";

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "common";
    resources: {
      ai: typeof ai;
      calendar: typeof calendar;
      chrome: typeof chrome;
      common: typeof common;
      connect: typeof connect;
      course: typeof course;
      courses: typeof courses;
      explain: typeof explain;
      onboarding: typeof onboarding;
      plan: typeof plan;
      proposals: typeof proposals;
      reminders: typeof reminders;
      weeklyNote: typeof weeklyNote;
      removal: typeof removal;
      settings: typeof settings;
      sources: typeof sources;
      updates: typeof updates;
    };
    returnNull: false;
  }
}
