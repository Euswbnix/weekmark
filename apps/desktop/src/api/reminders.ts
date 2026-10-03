// Reminders (v0.3 M3; design §5.3): the facade's types (generated.ts) and the desktop shell's
// (background.rs, reminders.rs).

import type { DayOfWeek, ReminderSettings as StoredReminderSettings } from "./generated";

export type { Reminder, ReminderKind } from "./generated";

export type Weekday = DayOfWeek;
export const WEEKDAYS: readonly Weekday[] = [
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
  "sunday",
];

/**
 * `reminder_settings()` fills in every field (the facade's defaults for missing ones), so the UI
 * reads and writes them whole. Times are "HH:MM", local; a kind turned off keeps its time.
 */
export type ReminderSettings = Required<StoredReminderSettings>;

/** The tray and the login item as they are now (background.rs). */
export interface BackgroundStatus {
  run_in_background: boolean;
  tray: boolean;
  /** On, but this system can't show a tray icon (Linux without an AppIndicator library). */
  tray_unavailable: boolean;
  /** The login item exists; the student may have removed it in the system's settings. */
  login_item: boolean;
}

/** A notification as the page words it; Rust shows it and marks the reminder shown. */
export interface NotificationText {
  id: string;
  title: string;
  body: string;
}

/** The tray menu's words. */
export interface TrayLabels {
  open: string;
  quit: string;
}
