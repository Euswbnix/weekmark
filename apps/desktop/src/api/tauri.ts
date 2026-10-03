// ─── Tauri boundary ────────────────────────────────────────────────────────────────────────
// This file is the ONLY place the UI talks to Rust. Each method invokes one command defined in
// src-tauri/src/commands.rs, which is a thin wrapper over `pagelamp_app::App`.
//
// Argument names: Tauri maps Rust snake_case parameters to camelCase keys, so the Rust
// parameter `base_url` is passed here as `baseUrl`.
// Errors: commands reject with a serialised `AppError`; `call` turns that into an ApiError.
// ───────────────────────────────────────────────────────────────────────────────────────────

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { isHttpUrl } from "@/lib/url";
import type { PageLampApi } from "./client";
import { ApiError, toApiError } from "./errors";
import type { SyncEvent } from "./types";

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toApiError(error);
  }
}

function eventChannel<E = SyncEvent>(onEvent: (event: E) => void): Channel<E> {
  const channel = new Channel<E>();
  channel.onmessage = onEvent;
  return channel;
}

export function createTauriApi(): PageLampApi {
  return {
    status: () => call("status"),
    activity: () => call("activity"),
    listSources: () => call("list_sources"),
    addCanvasSource: (baseUrl, token) => call("add_canvas_source", { baseUrl, token }),
    addFolderSource: (path, termStart, label) =>
      call("add_folder_source", { path, termStart: termStart ?? null, label: label ?? null }),
    addIcalSource: (feedUrl, label) => call("add_ical_source", { feedUrl, label: label ?? null }),
    updateSourceSecret: (sourceId, secret) => call("update_source_secret", { sourceId, secret }),
    removeSource: (sourceId) => call("remove_source", { sourceId }),

    syncAll: (req, onEvent) => call("sync_all", { req, onEvent: eventChannel(onEvent) }),
    syncSource: (sourceId, req, onEvent) =>
      call("sync_source", { sourceId, req, onEvent: eventChannel(onEvent) }),

    downloadCourseFiles: (courseId, onEvent) =>
      call("download_course_files", { course: courseId, onEvent: eventChannel(onEvent) }),
    cancelSync: () => call("cancel_sync"),

    listCourses: () => call("list_courses"),
    courseOverview: (courseId) => call("course_overview", { course: courseId }),
    weekMaterials: (courseId, week) =>
      call("week_materials", { course: courseId, week: week ?? null }),
    listDeadlines: (courseId, daysAhead, daysBack) =>
      call("list_deadlines", { course: courseId, daysAhead, daysBack }),
    search: (query, courseId, limit) => call("search", { query, course: courseId, limit }),
    latestStudyPlan: () => call("latest_study_plan"),

    setCoursePolicy: (courseId, policy, note) =>
      call("set_course_policy", { course: courseId, policy, note }),
    setCourseTerm: (courseId, start, end) =>
      call("set_course_term", { course: courseId, start, end }),
    setCourseHidden: (courseId, hidden) => call("set_course_hidden", { course: courseId, hidden }),
    keepCourseCurrent: (courseId, until) =>
      call("keep_course_current", { course: courseId, until }),
    clearKeepCourseCurrent: (courseId) => call("clear_keep_course_current", { course: courseId }),
    confirmCourseDates: (courseId) => call("confirm_course_dates", { course: courseId }),
    setCourseDates: (courseId, dates) => call("set_course_dates", { course: courseId, dates }),

    lifecycleSummary: () => call("lifecycle_summary"),
    snoozeLifecycleBanner: () => call("snooze_lifecycle_banner"),
    snoozeRemovalSuggestions: (courseIds, kind) =>
      call("snooze_removal_suggestions", { courses: courseIds, kind }),
    snoozeCalendarOffers: () => call("snooze_calendar_offers"),
    clearRemovalSnooze: (courseIds) => call("clear_removal_snooze", { courses: courseIds }),
    removalPreview: (courseIds) => call("removal_preview", { courses: courseIds }),
    removeCourses: (courseIds, options) => call("remove_courses", { courses: courseIds, options }),
    removedCourses: () => call("removed_courses"),
    restoreCourse: (removedId) => call("restore_course", { removedId }),
    purgeRemovedCourses: (removedIds, permanentIfNoTrash) =>
      call("purge_removed_courses", { removedIds, permanentIfNoTrash }),
    forgetRemovedCourse: (removedId) => call("forget_removed_course", { removedId }),

    courseCalendar: (courseId) => call("course_calendar", { course: courseId }),
    setCalendarSources: (courseId, include, exclude) =>
      call("set_calendar_sources", { course: courseId, include, exclude }),
    downloadMaterialFiles: (courseId, materialIds, onEvent) =>
      call("download_material_files", {
        course: courseId,
        materialIds,
        onEvent: eventChannel(onEvent),
      }),
    scanCourseCalendar: (courseId) => call("scan_course_calendar", { course: courseId }),
    acceptCalendarProposal: (proposalId, edits) =>
      call("accept_calendar_proposal", { proposalId, edits }),
    acceptPassingProposals: (proposalIds) => call("accept_passing_proposals", { proposalIds }),
    dismissCalendarProposal: (proposalId) => call("dismiss_calendar_proposal", { proposalId }),
    syllabusReadingOffers: () => call("syllabus_reading_offers"),
    readCourseCalendar: (courseId, generationId, options, onEvent) =>
      call("read_course_calendar", {
        course: courseId,
        generationId,
        options,
        onEvent: eventChannel(onEvent),
      }),
    readCourseCalendars: (courseIds, batchId, options, onEvent) =>
      call("read_course_calendars", {
        courses: courseIds,
        batchId,
        options,
        onEvent: eventChannel(onEvent),
      }),
    cancelGeneration: (generationId) => call("cancel_generation", { generationId }),
    explainWeek: (courseId, week, generationId, options, onEvent) =>
      call("explain_week", {
        course: courseId,
        week,
        generationId,
        options,
        onEvent: eventChannel(onEvent),
      }),
    savedExplanations: (courseId, week) => call("saved_explanations", { course: courseId, week }),
    deleteExplanation: (generationId) => call("delete_explanation", { generationId }),
    aiOutputLanguage: () => call("ai_output_language"),
    setAiOutputLanguage: (language) => call("set_ai_output_language", { language }),
    writeWeeklyNote: (generationId, options, onEvent) =>
      call("write_weekly_note", { generationId, options, onEvent: eventChannel(onEvent) }),
    weeklyNotes: () => call("weekly_notes"),
    deleteWeeklyNote: (generationId) => call("delete_weekly_note", { generationId }),
    weeklyNoteSettings: () => call("weekly_note_settings"),
    setPrepareWeeklyNoteOnMonday: (on) => call("set_prepare_weekly_note_on_monday", { on }),
    generateStudyPlan: (request, generationId, onEvent) =>
      call("generate_study_plan", {
        request,
        generationId,
        onEvent: eventChannel(onEvent),
      }),
    planLimits: () => call("plan_limits"),
    acceptStudyPlan: (generationId) => call("accept_study_plan", { generationId }),
    setStudyPlanItemDone: (planId, itemIndex, done) =>
      call("set_study_plan_item_done", { planId, itemIndex, done }),
    setCourseAiAccess: (courseId, allowed) =>
      call("set_course_ai_access", { course: courseId, allowed }),
    setCourseMaterialSharing: (courseId, answer) =>
      call("set_course_material_sharing", { course: courseId, answer }),

    // AI setup (M1). Most facade methods are stubs until schema v4 (after the alpha.1 tag), so
    // AI_SETUP_ENABLED keeps these screens to mock mode until the backend says they're live.
    aiStatus: () => call("ai_status"),
    modelProviderPresets: () => call("model_provider_presets"),
    addModelProvider: (preset, baseUrl, apiKey) =>
      call("add_model_provider", { preset, baseUrl, apiKey }),
    updateModelProviderKey: (providerId, apiKey) =>
      call("update_model_provider_key", { providerId, apiKey }),
    removeModelProvider: (providerId) => call("remove_model_provider", { providerId }),
    detectLocalServers: () => call("detect_local_servers"),
    // `backend` is the command's app-state parameter in Rust, so the BackendRef is `model_backend`.
    listModels: (backend) => call("list_models", { modelBackend: backend }),
    testModel: (backend, model) => call("test_model", { modelBackend: backend, model }),
    setFeatureModel: (feature, choice) => call("set_feature_model", { feature, choice }),
    acknowledgeAiDisclosure: (backend, version) =>
      call("acknowledge_ai_disclosure", { modelBackend: backend, version }),
    acknowledgeUnpricedModel: (backend, model) =>
      call("acknowledge_unpriced_model", { modelBackend: backend, model }),
    setMonthlyBudget: (microUsd) => call("set_monthly_budget", { microUsd }),
    estimateGeneration: (req) => call("estimate_generation", { request: req }),
    usageSummary: (month) => call("usage_summary", { month }),
    removeAllAiData: () => call("remove_all_ai_data"),
    codexStatus: () => call("codex_status"),
    installCodex: (installId, onEvent) =>
      call("install_codex", { installId, onEvent: eventChannel(onEvent) }),
    cancelCodexInstall: (installId) => call("cancel_codex_install", { installId }),
    removeCodex: () => call("remove_codex"),
    codexLogin: (method, onEvent) =>
      call("codex_login", { method, onEvent: eventChannel(onEvent) }),
    cancelCodexLogin: () => call("cancel_codex_login"),
    codexLogout: () => call("codex_logout"),
    setCodexSource: (source) => call("set_codex_source", { source }),
    setModeAWeeklyCap: (runs) => call("set_mode_a_weekly_cap", { runs }),

    mcpClientConfigs: () => call("mcp_client_configs"),

    updatePrefs: () => call("update_prefs"),
    setUpdatePrefs: (prefs) => call("set_update_prefs", { prefs }),
    effectiveUpdateChannel: () => call("effective_update_channel"),
    startupTasks: () => call("startup_tasks"),
    acknowledgeWhatsNew: () => call("acknowledge_whats_new"),
    acknowledgeUpdateDisclosure: () => call("acknowledge_update_disclosure"),
    lastUpdateCheck: () => call("last_update_check"),

    diagnosticReport: () => call("diagnostic_report"),
    doctor: () => call("doctor"),
    lastCrash: () => call("last_crash"),
    clearLastCrash: () => call("clear_last_crash"),

    // Plugin calls, like commands, reject only with an ApiError.
    pickFolder: async () => {
      try {
        // Needs capability `dialog:allow-open` (src-tauri/capabilities/default.json).
        const picked = await open({ directory: true, multiple: false });
        return typeof picked === "string" ? picked : null;
      } catch (error) {
        throw toApiError(error);
      }
    },
    openMaterial: (materialId) => call("open_material", { materialId }),
    revealMaterial: (materialId) => call("reveal_material", { materialId }),
    openExternal: async (url) => {
      // The opener capability is scoped to http(s) too; this check gives a clearer error.
      if (!isHttpUrl(url)) throw new ApiError("invalid", "Only web links can be opened");
      try {
        // Normalised (scheme/host lower-cased, spaces trimmed) so it matches the scope glob.
        await openUrl(new URL(url.trim()).href);
      } catch (error) {
        throw toApiError(error);
      }
    },
    reminderSettings: () => call("reminder_settings"),
    setReminderSettings: (settings) => call("set_reminder_settings", { settings }),
    backgroundStatus: () => call("background_status"),
    setTrayLabels: (labels) => call("set_tray_labels", { labels }),
    dueReminders: () => call("due_reminders"),
    showReminders: (notifications) => call("show_reminders", { notifications }),
    markRemindersShown: (ids) => call("mark_reminders_shown", { ids }),
    openNotificationSettings: () => call("open_notification_settings"),
    showRemindersOnNotice: (title, body) => call("show_reminders_on_notice", { title, body }),
    onReminderCheck: (onCheck) => {
      let unlisten: (() => void) | null = null;
      let stopped = false;
      // Needs core:event:default only. If listening fails, reminders still come at launch.
      listen("reminders:check", () => onCheck())
        .then((stop) => {
          if (stopped) stop();
          else unlisten = stop;
        })
        .catch(() => {});
      return () => {
        stopped = true;
        unlisten?.();
      };
    },
    revealDataDir: () => call("reveal_data_dir"),
    onWindowFocus: (onFocus) => {
      let unlisten: (() => void) | null = null;
      let stopped = false;
      // Events need no extra capability (core:event:default). If listening fails, the UI just
      // doesn't refresh on focus; that's not worth an error.
      Promise.resolve()
        .then(() =>
          getCurrentWindow().onFocusChanged(({ payload: focused }) => {
            if (focused) onFocus();
          }),
        )
        .then((stop) => {
          if (stopped) stop();
          else unlisten = stop;
        })
        .catch(() => {});
      return () => {
        stopped = true;
        unlisten?.();
      };
    },
    revealLogsDir: () => call("reveal_logs_dir"),
    updaterStatus: () => call("updates_status"),
    checkForUpdate: () => call("updates_check"),
    installUpdate: (onEvent) => call("updates_install", { onEvent: eventChannel(onEvent) }),
    logUiError: async (message, stack) => {
      try {
        await call("log_ui_error", { message, stack });
      } catch {
        // Nowhere left to report it; the UI already shows the original error.
      }
    },
  };
}
