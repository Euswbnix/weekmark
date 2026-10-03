import type {
  AiFeature,
  AiStatus,
  BackendRef,
  CodexLoginMethod,
  CodexSource,
  CodexStatus,
  CostEstimate,
  EstimateRequest,
  GenEvent,
  LocalServer,
  LoginEvent,
  MaterialSharing,
  ModelChoice,
  ModelInfo,
  ModelProviderRecord,
  ProbeReport,
  ProviderPreset,
  RemoveAiDataReport,
  RuntimeEvent,
  UsageSummary,
} from "./ai";
import type { ExplainOptions, OutputLanguage, WeeklyExplanation } from "./explain";
import type { GeneratedStudyPlan, PlanLimits, StudyPlanRequest } from "./plan";
import type {
  BackgroundStatus,
  NotificationText,
  Reminder,
  ReminderSettings,
  TrayLabels,
} from "./reminders";
import type {
  Activity,
  AiPolicy,
  AppStatus,
  CalendarBatchEvent,
  CalendarCandidate,
  CalendarProposal,
  CalendarRunOutcome,
  CourseCalendarView,
  CourseDatesInput,
  CourseOverview,
  CourseSummary,
  CrashReport,
  Deadline,
  DoctorReport,
  IsoDate,
  LifecycleSummary,
  McpClientConfig,
  PurgeReport,
  ReadCalendarOptions,
  RemovalPreview,
  RemovalReport,
  RemovedCourse,
  RemoveOptions,
  RestoreOutcome,
  SearchHit,
  SnoozeKind,
  SourceRecord,
  SourceSyncResult,
  StartupTasks,
  StoredStudyPlan,
  SyllabusOffer,
  SyncEvent,
  SyncRequest,
  SyncSummary,
  UpdateChannel,
  UpdateCheckRecord,
  UpdatePrefs,
  WeekMaterials,
} from "./types";
import type { WeeklyNote, WeeklyNoteOptions, WeeklyNoteSettings } from "./weeklyNote";

// ----- updater (desktop only: src-tauri's updates.rs, not the facade) -------------------------

/** How this build updates: in the app, or (deb/rpm) only by downloading the new package. */
export type UpdateInstallMode = "in_app" | "download_only";

export interface UpdaterStatus {
  /** The real version (CARGO_PKG_VERSION, e.g. "0.3.0-alpha.1"). */
  current_version: string;
  install: UpdateInstallMode;
  platform: "macos" | "windows" | "linux";
}

export interface AvailableUpdate {
  version: string;
  /** RFC 3339 publication date, when the manifest has one. */
  date?: string | null;
  /** Release notes (plain text / Markdown from the manifest). */
  notes?: string | null;
  /** Release page for "download only" installs (deb/rpm). */
  download_url?: string | null;
}

/** Progress of "Install and restart", streamed from Rust. */
export type UpdateEvent =
  | { type: "download_started"; total_bytes?: number | null }
  | { type: "progress"; downloaded_bytes: number; total_bytes?: number | null }
  | { type: "installing" }
  | { type: "restarting" };

/**
 * Everything the UI can ask of the backend. One method per facade method in
 * crates/pagelamp-app (docs/ARCHITECTURE.md §5), plus a few desktop-only helpers at the end.
 *
 * Two implementations:
 * - `tauri.ts` — calls the Rust commands in src-tauri (the real app).
 * - `mock/`    — in-memory synthetic data for the browser and tests (`pnpm dev:mock`).
 *
 * Every method rejects with an `ApiError` (see errors.ts) — branch on `error.kind`.
 * `course` parameters always take `course.id` (never a code).
 */
export interface PageLampApi {
  // ----- status & sources ------------------------------------------------------------------
  status(): Promise<AppStatus>;
  /** What the app is doing now; "Install and restart" waits for all of it. */
  activity(): Promise<Activity>;
  listSources(): Promise<SourceRecord[]>;
  /** Validates the token. The token is passed through and never stored by the UI. */
  addCanvasSource(baseUrl: string, token: string): Promise<SourceRecord>;
  addFolderSource(
    path: string,
    termStart?: IsoDate | null,
    label?: string | null,
  ): Promise<SourceRecord>;
  /** Validates the feed by fetching it. The URL is a secret, like a token. */
  addIcalSource(feedUrl: string, label?: string | null): Promise<SourceRecord>;
  /** Replace an expired Canvas token or a changed feed URL without removing the source. */
  updateSourceSecret(sourceId: string, secret: string): Promise<SourceRecord>;
  /** Removes the source, everything synced from it, and its stored secret. */
  removeSource(sourceId: string): Promise<void>;

  // ----- sync --------------------------------------------------------------------------------
  syncAll(req: SyncRequest, onEvent: (event: SyncEvent) => void): Promise<SyncSummary>;
  syncSource(
    sourceId: string,
    req: SyncRequest,
    onEvent: (event: SyncEvent) => void,
  ): Promise<SourceSyncResult>;

  /**
   * "Download & index this course's files" (Canvas only; folder courses → `invalid`). The UI
   * must first say that downloading through Canvas can count as viewing a file.
   */
  downloadCourseFiles(
    courseId: string,
    onEvent: (event: SyncEvent) => void,
  ): Promise<SourceSyncResult>;

  /**
   * Stop this app's running sync or download at the next file, course or download; the stopped
   * call rejects with `cancelled`. Does nothing when none runs here (not a CLI sync).
   */
  cancelSync(): Promise<void>;

  // ----- read views ----------------------------------------------------------------------------
  /** All courses, hidden ones included (check `course.hidden`). */
  listCourses(): Promise<CourseSummary[]>;
  courseOverview(courseId: string): Promise<CourseOverview>;
  /** `week` omitted/null = the course's current week. */
  weekMaterials(courseId: string, week?: number | null): Promise<WeekMaterials>;
  listDeadlines(courseId: string | null, daysAhead: number, daysBack: number): Promise<Deadline[]>;
  search(query: string, courseId: string | null, limit: number): Promise<SearchHit[]>;
  latestStudyPlan(): Promise<StoredStudyPlan | null>;

  // ----- course settings -------------------------------------------------------------------
  setCoursePolicy(courseId: string, policy: AiPolicy, note: string | null): Promise<void>;
  setCourseTerm(courseId: string, start: IsoDate | null, end: IsoDate | null): Promise<void>;
  setCourseHidden(courseId: string, hidden: boolean): Promise<void>;
  /**
   * "I'm still taking this": the course counts as current until `until` (null = the facade's
   * default: the end of the course's outer date frame, else today + 120 days).
   */
  keepCourseCurrent(courseId: string, until: IsoDate | null): Promise<void>;
  /** Undo "I'm still taking this". */
  clearKeepCourseCurrent(courseId: string): Promise<void>;
  /** "These dates are right": the student checked dates kept from version 0.1. */
  confirmCourseDates(courseId: string): Promise<void>;
  /**
   * The course dates form v2: first and last day of classes, end of exams, breaks and a second
   * part for full-year courses. null = clear the student's dates (the calendar in force).
   * Returns the course's calendar view as it is now.
   */
  setCourseDates(courseId: string, dates: CourseDatesInput | null): Promise<CourseCalendarView>;

  // ----- course lifecycle and removal (calendar design §8) ---------------------------------
  /** Every course's lifecycle, which ones are suggested for removal, and the banner state. */
  lifecycleSummary(): Promise<LifecycleSummary>;
  /** "Not now" on the "courses look finished" banner (14 days, computed by the facade). */
  snoozeLifecycleBanner(): Promise<void>;
  /** "Not now" (14 days) or "Keep" (never again) on some courses' removal suggestion. */
  snoozeRemovalSuggestions(courseIds: string[], kind: SnoozeKind): Promise<void>;
  /** "Not now" on the syllabus reading offers: 14 days, like the lifecycle banner. */
  snoozeCalendarOffers(): Promise<void>;
  clearRemovalSnooze(courseIds: string[]): Promise<void>;
  /** What removing these courses would delete and keep. */
  removalPreview(courseIds: string[]): Promise<RemovalPreview>;
  /** Stage 1 at once (and stage 2 with `purge_now`). `busy` while a sync runs. */
  removeCourses(courseIds: string[], options: RemoveOptions): Promise<RemovalReport>;
  removedCourses(): Promise<RemovedCourse[]>;
  /** Undo within 7 days, or restore by syncing again after the purge. */
  restoreCourse(removedId: string): Promise<RestoreOutcome>;
  /** "Delete now" (given ids) or every due purge (null). */
  purgeRemovedCourses(
    removedIds: string[] | null,
    permanentIfNoTrash: boolean,
  ): Promise<PurgeReport>;
  /** Purged courses only: the next sync brings the course back. */
  forgetRemovedCourse(removedId: string): Promise<void>;

  // ----- course calendar proposals (calendar design §7; F3) --------------------------------
  /** The calendar in force, pending proposals, candidates and why AI reading can't run. */
  courseCalendar(courseId: string): Promise<CourseCalendarView>;
  /** The student's add/remove of candidate materials. */
  setCalendarSources(
    courseId: string,
    include: string[],
    exclude: string[],
  ): Promise<CalendarCandidate[]>;
  /** Download chosen files only (counts as viewing them in Canvas; D46). */
  downloadMaterialFiles(
    courseId: string,
    materialIds: string[],
    onEvent: (event: SyncEvent) => void,
  ): Promise<SourceSyncResult>;
  /** The deterministic syllabus scan (no model). null = nothing new to propose. */
  scanCourseCalendar(courseId: string): Promise<CalendarProposal | null>;
  /** Accept a proposal, optionally with the student's edits and conflict choices. */
  acceptCalendarProposal(
    proposalId: number,
    edits: CourseDatesInput | null,
  ): Promise<CourseCalendarView>;
  /** Accept several proposals that have no conflicts and aren't low quality. */
  acceptPassingProposals(proposalIds: number[]): Promise<CourseCalendarView[]>;
  dismissCalendarProposal(proposalId: number): Promise<void>;
  /** The courses "Read syllabi for N courses" would read (the facade decides). */
  syllabusReadingOffers(): Promise<SyllabusOffer[]>;
  /** "Read the syllabus with AI" (design §7.3): gated like every AI run; makes a proposal. */
  readCourseCalendar(
    courseId: string,
    generationId: string,
    options: ReadCalendarOptions,
    onEvent: (event: GenEvent) => void,
  ): Promise<CalendarProposal>;
  /** "Read syllabi for N courses": one course after another, each gated on its own. */
  readCourseCalendars(
    courseIds: string[],
    batchId: string,
    options: ReadCalendarOptions,
    onEvent: (event: CalendarBatchEvent) => void,
  ): Promise<CalendarRunOutcome[]>;
  /** Stop a running generation or batch (by its id); the run ends with `cancelled`. */
  cancelGeneration(generationId: string): Promise<void>;

  // ----- weekly explanations (M3; design §5.2) ------------------------------------------------
  /**
   * Explains a course's week (null: its default week) from its materials, every paragraph cited:
   * a model run (`cancelGeneration(generationId)` stops it). No text arrives before the end.
   */
  explainWeek(
    courseId: string,
    week: number | null,
    generationId: string,
    options: ExplainOptions,
    onEvent: (event: GenEvent) => void,
  ): Promise<WeeklyExplanation>;
  /** The last 5 for the course and week, newest first (`stale` recomputed). */
  savedExplanations(courseId: string, week: number | null): Promise<WeeklyExplanation[]>;
  /** Deletes one explanation and its text (`not_found` for another feature's id). */
  deleteExplanation(generationId: string): Promise<void>;
  /** Whether explanations are written in PageLamp's language or the course's. */
  aiOutputLanguage(): Promise<OutputLanguage>;
  setAiOutputLanguage(language: OutputLanguage): Promise<void>;

  // ----- the weekly note (beta.2; design §5.3) --------------------------------------------------
  /**
   * Writes the weekly note from structure and plan progress: a model run (`cancelGeneration`
   * stops it; `activity` lists it). `automatic` when `startup_tasks` asked for it (Monday's
   * opt-in): never over the budget, and `invalid` once it isn't due.
   */
  writeWeeklyNote(
    generationId: string,
    options: WeeklyNoteOptions,
    onEvent: (event: GenEvent) => void,
  ): Promise<WeeklyNote>;
  /** The last 5 notes, newest first. */
  weeklyNotes(): Promise<WeeklyNote[]>;
  /** Deletes one note (`not_found` for another id). */
  deleteWeeklyNote(generationId: string): Promise<void>;
  /** "Prepare it when I open PageLamp on Monday", and whether the note's model allows it. */
  weeklyNoteSettings(): Promise<WeeklyNoteSettings>;
  /** Turning it on is `invalid` unless the note's model is an API key or a local model. */
  setPrepareWeeklyNoteOnMonday(on: boolean): Promise<WeeklyNoteSettings>;

  // ----- study plans written by PageLamp (M3; design §5.1) ------------------------------------
  /**
   * A draft plan: a model run like the syllabus reading (`cancelGeneration(generationId)` stops
   * it), its dates set by PageLamp's scheduler. Nothing is saved until `acceptStudyPlan`.
   */
  generateStudyPlan(
    request: StudyPlanRequest,
    generationId: string,
    onEvent: (event: GenEvent) => void,
  ): Promise<GeneratedStudyPlan>;
  /** What a request may ask for (horizon, weekly hours, note length): the limits it enforces. */
  planLimits(): Promise<PlanLimits>;
  /** Saves the draft as the latest plan (origin "pagelamp"). */
  acceptStudyPlan(generationId: string): Promise<StoredStudyPlan>;
  /** Ticks an item off (or back on); the index counts the items as `latestStudyPlan` lists them. */
  setStudyPlanItemDone(planId: number, itemIndex: number, done: boolean): Promise<StoredStudyPlan>;
  /** "Let my AI app read this course's materials" (§3 rule 8). "No AI" still wins over it. */
  setCourseAiAccess(courseId: string, allowed: boolean): Promise<void>;

  /** Question (b): may this course's materials be shared with an AI service? (D37) */
  setCourseMaterialSharing(courseId: string, answer: MaterialSharing): Promise<void>;

  // ----- AI setup (M1; design §3.8) -----------------------------------------------------------
  /** Backends in priority order with their disclosure facts, feature routing and budget. Local. */
  aiStatus(): Promise<AiStatus>;
  modelProviderPresets(): Promise<ProviderPreset[]>;
  /**
   * Validates the key with a free call, then stores it in the keychain. The key is passed
   * through and never stored by the UI; the record carries only its last 4 characters.
   */
  addModelProvider(
    preset: string,
    baseUrl: string | null,
    apiKey: string | null,
  ): Promise<ModelProviderRecord>;
  updateModelProviderKey(providerId: string, apiKey: string): Promise<ModelProviderRecord>;
  /** Removes the provider, its keychain entry and the feature choices that used it. */
  removeModelProvider(providerId: string): Promise<void>;
  /** Looks for Ollama and LM Studio on this computer (loopback only). */
  detectLocalServers(): Promise<LocalServer[]>;
  /** The live model list, joined with the price catalog and the on-device rule. */
  listModels(backend: BackendRef): Promise<ModelInfo[]>;
  /** A tiny real call: does this model answer, with structured output? */
  testModel(backend: BackendRef, model: string): Promise<ProbeReport>;
  setFeatureModel(feature: AiFeature, choice: ModelChoice | null): Promise<void>;
  /** The student read the disclosure sheet for this backend; `version` is the facts' hash. */
  acknowledgeAiDisclosure(backend: BackendRef, version: number): Promise<void>;
  /** "The budget is not enforced for this model" (no price in the catalog). */
  acknowledgeUnpricedModel(backend: BackendRef, model: string): Promise<void>;
  /** The soft monthly cap for API keys, in micro-USD; null = no cap. */
  setMonthlyBudget(microUsd: number | null): Promise<void>;
  /** "≈ $x" before Generate, and whether the run would be blocked. Local; call it debounced. */
  estimateGeneration(req: EstimateRequest): Promise<CostEstimate>;
  /** `month` = any day of the month (null = this month). Counts only, never content. */
  usageSummary(month: IsoDate | null): Promise<UsageSummary>;
  /** Keys, generated content, the usage ledger and AI settings; signs out of Codex first. */
  removeAllAiData(): Promise<RemoveAiDataReport>;

  // ----- mode A: the ChatGPT plan through official Codex (M2; design §2.3) -------------------
  /** The runtime, the sign-in, the weekly cap and what a RuntimeOutdated error means now. */
  codexStatus(): Promise<CodexStatus>;
  /**
   * Downloads the pinned Codex, verifies it and installs it (≈70 MB). `installId` (made by the
   * UI) lets cancelCodexInstall stop it before this resolves; a cancel rejects with `cancelled`.
   */
  installCodex(installId: string, onEvent: (event: RuntimeEvent) => void): Promise<CodexStatus>;
  /** Stops the download; the partial file is deleted, never resumed. */
  cancelCodexInstall(installId: string): Promise<void>;
  removeCodex(): Promise<void>;
  /** Codex's own sign-in; PageLamp never sees the credentials. */
  codexLogin(method: CodexLoginMethod, onEvent: (event: LoginEvent) => void): Promise<CodexStatus>;
  cancelCodexLogin(): Promise<void>;
  codexLogout(): Promise<CodexStatus>;
  /** Use PageLamp's own Codex or an installed one in the tested range (D12). */
  setCodexSource(source: CodexSource): Promise<CodexStatus>;
  /** PageLamp's ChatGPT-plan runs per week; null = no cap. */
  setModeAWeeklyCap(runs: number | null): Promise<void>;

  // ----- "connect your AI app" -------------------------------------------------------------
  /** The Rust side decides which `pagelamp` binary the snippets point at. */
  mcpClientConfigs(): Promise<McpClientConfig[]>;

  // ----- diagnostics (work even when the database can't be opened) ----------------------------
  /**
   * Markdown report for bug reports: versions, source states, recent log lines and the last
   * crash. Redacted and pseudonymised by the Rust side; the UI shows it before copying.
   */
  diagnosticReport(): Promise<string>;
  /** The setup check (file reader, unreadable files, database, keychain…), as data. */
  doctor(): Promise<DoctorReport>;
  /** The crash the panic hook recorded, until `clearLastCrash()`. */
  lastCrash(): Promise<CrashReport | null>;
  clearLastCrash(): Promise<void>;

  // ----- updates (facade: preferences, what's due now, the last check) ---------------------
  updatePrefs(): Promise<UpdatePrefs>;
  setUpdatePrefs(prefs: UpdatePrefs): Promise<void>;
  /** The chosen channel, else beta for a pre-release build, else stable (decision D3). */
  effectiveUpdateChannel(): Promise<UpdateChannel>;
  /** What to do at launch: the "What's new" sheet, an automatic check, the post-update banner. */
  startupTasks(): Promise<StartupTasks>;
  acknowledgeWhatsNew(): Promise<void>;
  /** The student saw (in onboarding) that PageLamp checks for updates. */
  acknowledgeUpdateDisclosure(): Promise<void>;
  lastUpdateCheck(): Promise<UpdateCheckRecord | null>;

  // ----- reminders (M3; design §5.3): what is due is the facade's, showing it the shell's ----
  reminderSettings(): Promise<ReminderSettings>;
  /**
   * Saves the settings; the shell then follows `run_in_background` (the tray, the login item and
   * the close button) and answers with how that went.
   */
  setReminderSettings(settings: ReminderSettings): Promise<BackgroundStatus>;
  backgroundStatus(): Promise<BackgroundStatus>;
  /** The tray menu in the student's language. */
  setTrayLabels(labels: TrayLabels): Promise<void>;
  /** What is due now: catch-up, dedupe and maximum age are the facade's. */
  dueReminders(): Promise<Reminder[]>;
  /** Shows these notifications, then marks their reminders shown. */
  showReminders(notifications: NotificationText[]): Promise<void>;
  /** Seen in the app instead (the catch-up card): they don't come back. */
  markRemindersShown(ids: string[]): Promise<void>;
  /**
   * The one notification when reminders are turned on: where the system asks whether PageLamp
   * may notify (desktop systems have no other way to ask). Marks nothing.
   */
  showRemindersOnNotice(title: string, body: string): Promise<void>;
  /** Opens the system's notification settings; false where there's no standard place (Linux). */
  openNotificationSettings(): Promise<boolean>;
  /** Calls `onCheck` whenever the shell asks for a delivery (every 15 min, after a sleep). */
  onReminderCheck(onCheck: () => void): () => void;

  // ----- desktop helpers (not part of the facade) --------------------------------------------
  /** Native folder picker. Resolves null when cancelled. */
  pickFolder(): Promise<string | null>;
  /** Open an http(s) link in the default browser. Other schemes are rejected. */
  openExternal(url: string): Promise<void>;
  /**
   * Open a material's local file with the system's app (material_local_file, Rust-side; the
   * page never sees the path). false: no document of it on this computer.
   */
  openMaterial(materialId: string): Promise<boolean>;
  /** Show a material's local file in Finder / Explorer. false: not on this computer. */
  revealMaterial(materialId: string): Promise<boolean>;
  /** Show the PageLamp data folder in Finder / Explorer. */
  revealDataDir(): Promise<void>;
  /**
   * Calls `onFocus` whenever the app window gains focus, e.g. after the student saved a study
   * plan in their AI app. Returns a function that stops listening.
   */
  onWindowFocus(onFocus: () => void): () => void;
  /** Show the folder with PageLamp's log files in Finder / Explorer. */
  revealLogsDir(): Promise<void>;
  /**
   * Write a UI crash (error-boundary) to the log: message and stack only, never app data.
   * Never rejects — logging must not cause a second error.
   */
  logUiError(message: string, stack: string | null): Promise<void>;
  updaterStatus(): Promise<UpdaterStatus>;
  /** Checks the effective channel; the result is recorded (codes only) for diagnostics. */
  checkForUpdate(): Promise<AvailableUpdate | null>;
  /**
   * Downloads, verifies and installs the update found by the last check, then restarts
   * PageLamp (on Windows the installer closes it). Only ever called after the student asked.
   */
  installUpdate(onEvent: (event: UpdateEvent) => void): Promise<void>;
}
