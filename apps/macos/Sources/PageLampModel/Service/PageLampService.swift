// What the Mac UI asks of PageLamp's core: every facade call the M1 screens need (spec §2.8),
// behind one protocol so the preview can run on synthetic data (MockService) or the real facade
// (LiveService).

import Foundation
import PageLampKit

/// PageLamp's core as the UI sees it. Every call is async and throws `PageLampFailure`.
///
/// Implementations: `LiveService` (the Rust facade through UniFFI), `MockService` (synthetic
/// demo data, the preview's default) and `UnavailableService` (the facade could not open; only
/// diagnostics work, spec S2).
///
/// Screens load their own data through `AppModel.service`; the shell data (status, courses,
/// sources, the This Week inputs) is loaded and kept by `AppModel`.
public protocol PageLampService: Sendable {
    // Shell (spec §2.8)
    func status() async throws(PageLampFailure) -> AppStatus
    /// All courses, hidden ones included (check `course.hidden`).
    func listCourses() async throws(PageLampFailure) -> [CourseSummary]
    func listSources() async throws(PageLampFailure) -> [SourceRecord]

    // This Week and course detail
    /// With a course: its events. Without: every visible course's events plus unlinked events.
    func listDeadlines(course: String?, daysAhead: UInt32, daysBack: UInt32) async throws(PageLampFailure) -> [Deadline]
    func latestStudyPlan() async throws(PageLampFailure) -> StoredStudyPlan?
    /// `course` is an id or a code.
    func courseOverview(course: String) async throws(PageLampFailure) -> CourseOverview
    /// The materials of `week` (nil: the current week).
    func weekMaterials(course: String, week: UInt32?) async throws(PageLampFailure) -> WeekMaterials

    // Course weeks, phases and the Past group (v0.3 course lane; `course` is an id or a code)
    /// Where one course is: week, phase, the dates used and not used, evidence.
    func courseTimeline(course: String) async throws(PageLampFailure) -> CourseTimeline
    /// Every course's lifecycle, the removal suggestions and whether the banner shows.
    func lifecycleSummary() async throws(PageLampFailure) -> LifecycleSummary
    /// "I'm still taking this" until `until` ("YYYY-MM-DD"; nil: the end of the course's term
    /// when that is ahead, else today + `keepCurrentDays()`).
    func keepCourseCurrent(course: String, until: String?) async throws(PageLampFailure) -> Course
    func clearKeepCourseCurrent(course: String) async throws(PageLampFailure) -> Course
    /// "Not now" (`notNowDays()`) or "Keep" (never again) on these courses' removal suggestion.
    func snoozeRemovalSuggestions(courses: [String], kind: SnoozeKind) async throws(PageLampFailure)
    func clearRemovalSnooze(courses: [String]) async throws(PageLampFailure)
    /// "Not now" on the "N courses look finished" banner.
    func snoozeLifecycleBanner() async throws(PageLampFailure)
    /// "These dates are right" for dates set in PageLamp 0.1.
    func confirmCourseDates(course: String) async throws(PageLampFailure) -> CourseTimeline

    // Updates and launch. Sparkle installs; the facade decides What's new and when to check.
    /// What's new (upgraders), whether the update check is due, the version this launch
    /// updated from. `now` is the caller's clock (the app asks again on a timer).
    func startupTasks(now: Date) async throws(PageLampFailure) -> StartupTasks
    func updatePrefs() async throws(PageLampFailure) -> UpdatePrefs
    func setUpdatePrefs(prefs: UpdatePrefs) async throws(PageLampFailure)
    /// The student's channel, else Beta for a pre-release build and Stable otherwise.
    func effectiveUpdateChannel() async throws(PageLampFailure) -> UpdateChannel
    func acknowledgeWhatsNew() async throws(PageLampFailure)
    /// The student saw what the update check sends.
    func acknowledgeUpdateDisclosure() async throws(PageLampFailure)
    func recordUpdateCheck(record: UpdateCheckRecord) async throws(PageLampFailure)
    func lastUpdateCheck() async throws(PageLampFailure) -> UpdateCheckRecord?
    /// Syncs and downloads running in this app, and whether another process syncs (ask before
    /// installing an update).
    func activity() async throws(PageLampFailure) -> Activity

    // AI (v0.3 M1–M3): model setup, costs, the ChatGPT plan through Codex, explanations and
    // study plans. A generation takes the caller's id; `cancelGeneration(generationId:)` stops it.
    // Progress goes to the observer (called off the run: it can't hold it up).
    /// The providers the student can add (OpenAI, Anthropic, Ollama, …), with their facts.
    func modelProviderPresets() async throws(PageLampFailure) -> [ProviderPreset]
    /// Model servers running on this computer (Ollama, LM Studio, …).
    func detectLocalServers() async throws(PageLampFailure) -> [LocalServer]
    /// Every backend, its state and disclosure, the feature routing and the budget.
    func aiStatus() async throws(PageLampFailure) -> AiStatus
    /// Adds a provider from a preset; the key goes to the secret store, never the database.
    func addModelProvider(preset: String, baseUrl: String?, apiKey: String?) async throws(PageLampFailure) -> ModelProviderRecord
    func updateModelProviderKey(providerId: String, apiKey: String) async throws(PageLampFailure) -> ModelProviderRecord
    func removeModelProvider(providerId: String) async throws(PageLampFailure)
    func listModels(backend: BackendRef) async throws(PageLampFailure) -> [ModelInfo]
    /// A tiny request to the model: does it answer, and in structured output?
    func testModel(backend: BackendRef, model: String) async throws(PageLampFailure) -> ProbeReport
    /// The model a feature uses (nil: none).
    func setFeatureModel(feature: AiFeature, choice: ModelChoice?) async throws(PageLampFailure)
    /// The student read what `backend` receives (`version`: the disclosure they saw).
    func acknowledgeAiDisclosure(backend: BackendRef, version: UInt32) async throws(PageLampFailure)
    /// The student accepts that `model` has no known price.
    func acknowledgeUnpricedModel(backend: BackendRef, model: String) async throws(PageLampFailure)
    /// The monthly budget in millionths of a US dollar (nil: none).
    func setMonthlyBudget(microUsd: UInt64?) async throws(PageLampFailure)
    func estimateGeneration(request: EstimateRequest) async throws(PageLampFailure) -> CostEstimate
    /// Use and cost in `month` (any day of it; nil: this month).
    func usageSummary(month: String?) async throws(PageLampFailure) -> UsageSummary
    /// Question (b): may a cloud model read this course's materials?
    func setCourseMaterialSharing(course: String, answer: MaterialSharing) async throws(PageLampFailure)
    /// Deletes generated explanations and plan drafts (one course, or all); returns how many.
    func deleteGenerated(course: String?) async throws(PageLampFailure) -> UInt32
    /// Everything AI: providers and their keys, choices, usage, generations.
    func removeAllAiData() async throws(PageLampFailure) -> RemoveAiDataReport
    func codexStatus() async throws(PageLampFailure) -> CodexStatus
    /// Downloads, verifies and installs the pinned Codex; `cancelCodexInstall(installId:)`
    /// stops it.
    func installCodex(installId: String, observer: any CodexInstallObserver) async throws(PageLampFailure) -> CodexStatus
    func cancelCodexInstall(installId: String) async throws(PageLampFailure)
    /// Removes the Codex PageLamp installed (never one the student installed themselves).
    func removeCodex() async throws(PageLampFailure)
    /// Signs in through Codex (browser or one-time code); `cancelCodexLogin()` stops it.
    func codexLogin(method: CodexLoginMethod, observer: any CodexLoginObserver) async throws(PageLampFailure) -> CodexStatus
    func cancelCodexLogin() async throws(PageLampFailure)
    func codexLogout() async throws(PageLampFailure) -> CodexStatus
    /// Runs per week on the ChatGPT plan (nil: the default cap).
    func setModeAWeeklyCap(runs: UInt32?) async throws(PageLampFailure)
    /// Which Codex to use: the one PageLamp installs, or the student's own.
    func setCodexSource(source: CodexSource) async throws(PageLampFailure) -> CodexStatus
    /// Explains a week of `course` (nil: the default week) from its readable materials, with
    /// citations; `cancelGeneration(generationId:)` stops it.
    func explainWeek(course: String, week: UInt32?, generationId: String, options: ExplainOptions, observer: any GenObserver) async throws(PageLampFailure) -> WeeklyExplanation
    /// The saved explanations of `course` (one week, or all), newest first.
    func savedExplanations(course: String, week: UInt32?) async throws(PageLampFailure) -> [WeeklyExplanation]
    func deleteExplanation(generationId: String) async throws(PageLampFailure)
    /// Explanations in the app's language or the course's.
    func aiOutputLanguage() async throws(PageLampFailure) -> OutputLanguage
    func setAiOutputLanguage(language: OutputLanguage) async throws(PageLampFailure)
    /// Writes this week's note from the courses' structure and the plan's progress (never
    /// material text); `cancelGeneration(generationId:)` stops it. `options.automatic` only
    /// when `startupTasks(now:).prepareWeeklyNote` said so.
    func writeWeeklyNote(generationId: String, options: WeeklyNoteOptions, observer: any GenObserver) async throws(PageLampFailure) -> WeeklyNote
    /// The kept weekly notes, newest first.
    func weeklyNotes() async throws(PageLampFailure) -> [WeeklyNote]
    func deleteWeeklyNote(generationId: String) async throws(PageLampFailure)
    /// "Prepare it when I open PageLamp on Monday", and whether the note's model allows it.
    func weeklyNoteSettings() async throws(PageLampFailure) -> WeeklyNoteSettings
    /// Turns "prepare it on Monday" on (an API key or a model on this computer only) or off.
    func setPrepareWeeklyNoteOnMonday(on: Bool) async throws(PageLampFailure) -> WeeklyNoteSettings
    /// Drafts a plan (not saved until `acceptStudyPlan(generationId:)`);
    /// `cancelGeneration(generationId:)` stops it.
    func generateStudyPlan(request: StudyPlanRequest, generationId: String, observer: any GenObserver) async throws(PageLampFailure) -> GeneratedStudyPlan
    /// Saves the draft `generationId` as the current study plan.
    func acceptStudyPlan(generationId: String) async throws(PageLampFailure) -> StoredStudyPlan
    func setStudyPlanItemDone(planId: Int64, itemIndex: UInt32, done: Bool) async throws(PageLampFailure) -> StoredStudyPlan
    /// Stops a running generation or batch by the id the caller gave it; it ends with `Cancelled`.
    /// Unknown or finished ids are fine.
    func cancelGeneration(generationId: String) async throws(PageLampFailure)

    // The rest of the course lane: calendars and reading the syllabus, removing finished
    // courses, reminders and the weekly digest, a material's file.
    /// The calendar in force, the proposals waiting, the candidates to read and why reading is
    /// blocked, if it is.
    func courseCalendar(course: String) async throws(PageLampFailure) -> CourseCalendarView
    /// The materials a syllabus reading would read, and why others are left out.
    func calendarCandidates(course: String) async throws(PageLampFailure) -> [CalendarCandidate]
    /// The student's picks: always read `include`, never `exclude` (material ids).
    func setCalendarSources(course: String, include: [String], exclude: [String]) async throws(PageLampFailure) -> [CalendarCandidate]
    /// Downloads these materials' files (Canvas) so they can be read; UIs disclose first.
    func downloadMaterialFiles(course: String, materialIds: [String], observer: any SyncObserver) async throws(PageLampFailure) -> SourceSyncResult
    /// The deterministic scan of the syllabus (no model); nil when it finds nothing new.
    func scanCourseCalendar(course: String) async throws(PageLampFailure) -> CalendarProposal?
    /// The student's own dates (nil: clear them, "Undo").
    func setCourseDates(course: String, dates: CourseDatesInput?) async throws(PageLampFailure) -> CourseCalendarView
    /// Accepts a proposal, optionally with the student's edits.
    func acceptCalendarProposal(proposalId: Int64, edits: CourseDatesInput?) async throws(PageLampFailure) -> CourseCalendarView
    /// Accepts proposals that have no conflicts ("Accept all that pass"); none when one doesn't
    /// pass (`Invalid`).
    func acceptPassingProposals(proposalIds: [Int64]) async throws(PageLampFailure) -> [CourseCalendarView]
    func dismissCalendarProposal(proposalId: Int64) async throws(PageLampFailure)
    /// The courses "Read syllabi for N courses" offers; empty while "Not now" covers them.
    func syllabusReadingOffers() async throws(PageLampFailure) -> [SyllabusOffer]
    /// "Not now" on the syllabus reading offers (`notNowDays()`).
    func snoozeCalendarOffers() async throws(PageLampFailure)
    /// Reads the syllabus with the chosen model into a proposal that changes nothing until
    /// accepted; `cancelGeneration(generationId:)` stops it.
    func readCourseCalendar(course: String, generationId: String, options: ReadCalendarOptions, observer: any GenObserver) async throws(PageLampFailure) -> CalendarProposal
    /// Reads several courses' syllabi, one after another; `cancelGeneration(generationId:)` with
    /// `batchId` stops the rest. A course that fails is an outcome, not an error.
    func readCourseCalendars(courses: [String], batchId: String, options: ReadCalendarOptions, observer: any CalendarBatchObserver) async throws(PageLampFailure) -> [CalendarRunOutcome]
    /// What removing `courses` would take away and keep.
    func removalPreview(courses: [String]) async throws(PageLampFailure) -> RemovalPreview
    /// Removes the courses (stage 1: undoable for 7 days).
    func removeCourses(courses: [String], options: RemoveOptions) async throws(PageLampFailure) -> RemovalReport
    func removedCourses() async throws(PageLampFailure) -> [RemovedCourse]
    func restoreCourse(removedId: String) async throws(PageLampFailure) -> RestoreOutcome
    /// Purges removed courses whose time is up (`removedIds`: these, nil: every due one).
    /// `permanentIfNoTrash`: delete files for good where there is no Trash; callers ask first,
    /// it is never implied.
    func purgeRemovedCourses(removedIds: [String]?, permanentIfNoTrash: Bool) async throws(PageLampFailure) -> PurgeReport
    /// Stops listing a removed course (its undo is gone).
    func forgetRemovedCourse(removedId: String) async throws(PageLampFailure)
    func weeklyDigest() async throws(PageLampFailure) -> WeeklyDigest
    func reminderSettings() async throws(PageLampFailure) -> ReminderSettings
    func setReminderSettings(settings: ReminderSettings) async throws(PageLampFailure)
    /// Every reminder from `from` to `to` (at most 62 days), for a schedule.
    func reminders(from: Date, to: Date) async throws(PageLampFailure) -> [Reminder]
    /// The reminders to show now (missed ones from the last 3 days included), not shown yet.
    func dueReminders(now: Date) async throws(PageLampFailure) -> [Reminder]
    func markRemindersShown(ids: [String]) async throws(PageLampFailure)
    /// The material's file on this computer for `purpose` (open or reveal), nil when there is none;
    /// never a path outside the data folder or the course folder.
    func materialLocalFile(materialId: String, purpose: LocalFileUse) async throws(PageLampFailure) -> String?
    /// Stops the running sync or download after its current step.
    func cancelSync() async throws(PageLampFailure)

    // Sources & Sync. Events go to `observer` (see `SyncEventStream`) while the call runs.
    /// Syncs every source; `.busy` when another sync runs. A failing source is reported in the
    /// summary (`ok: false`), not thrown.
    func syncAll(request: SyncRequest, observer: any SyncObserver) async throws(PageLampFailure) -> SyncSummary
    func syncSource(sourceId: String, request: SyncRequest, observer: any SyncObserver) async throws(PageLampFailure) -> SourceSyncResult

    // Connect your AI app (never writes an AI app's config)
    /// `pagelampBinary`: the bundled CLI (`AppModel.sidecarPath`).
    func mcpClientConfigs(pagelampBinary: String) async throws(PageLampFailure) -> [McpClientConfig]
    func mcpLaunch(pagelampBinary: String) async throws(PageLampFailure) -> McpLaunch
    /// Which AI apps are configured (`mcpClients`), keychain and database health, the version.
    func doctor() async throws(PageLampFailure) -> DoctorReport

    // Help and diagnostics (work in S2 too)
    /// Markdown, redacted and pseudonymised; always shown to the student before it is copied.
    func diagnosticReport() async throws(PageLampFailure) -> String
    /// The logs folder, created if missing.
    func logsDir() async throws(PageLampFailure) -> String
    func lastCrash() async throws(PageLampFailure) -> CrashReport?
    func clearLastCrash() async throws(PageLampFailure)
}
