// The real PageLamp core: the Rust facade through the UniFFI bindings (PageLampKit).

import Foundation
import PageLampKit
import Synchronization

/// `PageLampService` over the Rust facade. Every call runs on the core's own worker threads
/// (`spawn_blocking` behind UniFFI's async), so the main actor never blocks on the database.
///
/// Opening it with `openDefault()` uses the **real** data folder and keychain, shared with the
/// installed PageLamp app and its CLI. The preview app reaches it only from the Debug menu after
/// a confirmation; tests build it with `init(core:)` over `PageLamp.openWithMemorySecrets` in a
/// temp folder.
public final class LiveService: PageLampService {
    public let core: PageLamp

    public init(core: PageLamp) {
        self.core = core
    }

    /// Opens the default data folder (`~/Library/Application Support/dev.PageLamp.PageLamp`) with
    /// the keychain, after starting the core's diagnostics (logs, panic hook) once per process.
    /// Syncs read files in the bundled `pagelamp` executable's resource-limited worker
    /// (`bundledExtractWorker`); without one (`swift run`) they read them in this process.
    public static func openDefault() async throws(PageLampFailure) -> LiveService {
        try startDiagnostics()
        do {
            let core = try await PageLamp.open(dataDir: nil)
            try await core.setExtractWorker(path: bundledExtractWorker)
            return LiveService(core: core)
        } catch {
            throw PageLampFailure.from(error)
        }
    }

    /// The bundled CLI (`Contents/MacOS/pagelamp`), which runs `pagelamp extract-worker`.
    public static var bundledExtractWorker: String? {
        Bundle.main.url(forAuxiliaryExecutable: "pagelamp")?.path(percentEncoded: false)
    }

    /// `initDiagnostics` for the default data folder; later calls do nothing (like the core's).
    public static func startDiagnostics() throws(PageLampFailure) {
        let first = diagnosticsStarted.withLock { started in
            defer { started = true }
            return !started
        }
        guard first else { return }
        do {
            try initDiagnostics(verbose: false)
        } catch {
            throw PageLampFailure.from(error)
        }
    }

    private static let diagnosticsStarted = Mutex(false)

    /// Runs one facade call and maps its error.
    private func call<T>(_ body: () async throws -> T) async throws(PageLampFailure) -> T {
        do {
            return try await body()
        } catch {
            throw PageLampFailure.from(error)
        }
    }

    public func status() async throws(PageLampFailure) -> AppStatus {
        try await call { try await core.status() }
    }

    public func listCourses() async throws(PageLampFailure) -> [CourseSummary] {
        try await call { try await core.listCourses() }
    }

    public func listSources() async throws(PageLampFailure) -> [SourceRecord] {
        try await call { try await core.listSources() }
    }

    public func listDeadlines(course: String?, daysAhead: UInt32, daysBack: UInt32) async throws(PageLampFailure) -> [Deadline] {
        try await call { try await core.listDeadlines(course: course, daysAhead: daysAhead, daysBack: daysBack) }
    }

    public func latestStudyPlan() async throws(PageLampFailure) -> StoredStudyPlan? {
        try await call { try await core.latestStudyPlan() }
    }

    public func courseOverview(course: String) async throws(PageLampFailure) -> CourseOverview {
        try await call { try await core.courseOverview(course: course) }
    }

    public func weekMaterials(course: String, week: UInt32?) async throws(PageLampFailure) -> WeekMaterials {
        try await call { try await core.weekMaterials(course: course, week: week) }
    }

    public func courseTimeline(course: String) async throws(PageLampFailure) -> CourseTimeline {
        try await call { try await core.courseTimeline(course: course) }
    }

    public func lifecycleSummary() async throws(PageLampFailure) -> LifecycleSummary {
        try await call { try await core.lifecycleSummary() }
    }

    public func keepCourseCurrent(course: String, until: String?) async throws(PageLampFailure) -> Course {
        try await call { try await core.keepCourseCurrent(course: course, until: until) }
    }

    public func clearKeepCourseCurrent(course: String) async throws(PageLampFailure) -> Course {
        try await call { try await core.clearKeepCourseCurrent(course: course) }
    }

    public func snoozeRemovalSuggestions(courses: [String], kind: SnoozeKind) async throws(PageLampFailure) {
        try await call { try await core.snoozeRemovalSuggestions(courses: courses, kind: kind) }
    }

    public func clearRemovalSnooze(courses: [String]) async throws(PageLampFailure) {
        try await call { try await core.clearRemovalSnooze(courses: courses) }
    }

    public func snoozeLifecycleBanner() async throws(PageLampFailure) {
        try await call { try await core.snoozeLifecycleBanner() }
    }

    public func confirmCourseDates(course: String) async throws(PageLampFailure) -> CourseTimeline {
        try await call { try await core.confirmCourseDates(course: course) }
    }

    public func startupTasks(now: Date) async throws(PageLampFailure) -> StartupTasks {
        try await call { try await core.startupTasks(now: now) }
    }

    public func updatePrefs() async throws(PageLampFailure) -> UpdatePrefs {
        try await call { try await core.updatePrefs() }
    }

    public func setUpdatePrefs(prefs: UpdatePrefs) async throws(PageLampFailure) {
        try await call { try await core.setUpdatePrefs(prefs: prefs) }
    }

    public func effectiveUpdateChannel() async throws(PageLampFailure) -> UpdateChannel {
        try await call { try await core.effectiveUpdateChannel() }
    }

    public func acknowledgeWhatsNew() async throws(PageLampFailure) {
        try await call { try await core.acknowledgeWhatsNew() }
    }

    public func acknowledgeUpdateDisclosure() async throws(PageLampFailure) {
        try await call { try await core.acknowledgeUpdateDisclosure() }
    }

    public func recordUpdateCheck(record: UpdateCheckRecord) async throws(PageLampFailure) {
        try await call { try await core.recordUpdateCheck(record: record) }
    }

    public func lastUpdateCheck() async throws(PageLampFailure) -> UpdateCheckRecord? {
        try await call { try await core.lastUpdateCheck() }
    }

    public func activity() async throws(PageLampFailure) -> Activity {
        try await call { try await core.activity() }
    }

    public func modelProviderPresets() async throws(PageLampFailure) -> [ProviderPreset] {
        try await call { try await core.modelProviderPresets() }
    }

    public func detectLocalServers() async throws(PageLampFailure) -> [LocalServer] {
        try await call { try await core.detectLocalServers() }
    }

    public func aiStatus() async throws(PageLampFailure) -> AiStatus {
        try await call { try await core.aiStatus() }
    }

    public func addModelProvider(preset: String, baseUrl: String?, apiKey: String?) async throws(PageLampFailure) -> ModelProviderRecord {
        try await call { try await core.addModelProvider(preset: preset, baseUrl: baseUrl, apiKey: apiKey) }
    }

    public func updateModelProviderKey(providerId: String, apiKey: String) async throws(PageLampFailure) -> ModelProviderRecord {
        try await call { try await core.updateModelProviderKey(providerId: providerId, apiKey: apiKey) }
    }

    public func removeModelProvider(providerId: String) async throws(PageLampFailure) {
        try await call { try await core.removeModelProvider(providerId: providerId) }
    }

    public func listModels(backend: BackendRef) async throws(PageLampFailure) -> [ModelInfo] {
        try await call { try await core.listModels(backend: backend) }
    }

    public func testModel(backend: BackendRef, model: String) async throws(PageLampFailure) -> ProbeReport {
        try await call { try await core.testModel(backend: backend, model: model) }
    }

    public func setFeatureModel(feature: AiFeature, choice: ModelChoice?) async throws(PageLampFailure) {
        try await call { try await core.setFeatureModel(feature: feature, choice: choice) }
    }

    public func acknowledgeAiDisclosure(backend: BackendRef, version: UInt32) async throws(PageLampFailure) {
        try await call { try await core.acknowledgeAiDisclosure(backend: backend, version: version) }
    }

    public func acknowledgeUnpricedModel(backend: BackendRef, model: String) async throws(PageLampFailure) {
        try await call { try await core.acknowledgeUnpricedModel(backend: backend, model: model) }
    }

    public func setMonthlyBudget(microUsd: UInt64?) async throws(PageLampFailure) {
        try await call { try await core.setMonthlyBudget(microUsd: microUsd) }
    }

    public func estimateGeneration(request: EstimateRequest) async throws(PageLampFailure) -> CostEstimate {
        try await call { try await core.estimateGeneration(request: request) }
    }

    public func usageSummary(month: String?) async throws(PageLampFailure) -> UsageSummary {
        try await call { try await core.usageSummary(month: month) }
    }

    public func setCourseMaterialSharing(course: String, answer: MaterialSharing) async throws(PageLampFailure) {
        try await call { try await core.setCourseMaterialSharing(course: course, answer: answer) }
    }

    public func deleteGenerated(course: String?) async throws(PageLampFailure) -> UInt32 {
        try await call { try await core.deleteGenerated(course: course) }
    }

    public func removeAllAiData() async throws(PageLampFailure) -> RemoveAiDataReport {
        try await call { try await core.removeAllAiData() }
    }

    public func codexStatus() async throws(PageLampFailure) -> CodexStatus {
        try await call { try await core.codexStatus() }
    }

    public func installCodex(installId: String, observer: any CodexInstallObserver) async throws(PageLampFailure) -> CodexStatus {
        try await call { try await core.installCodex(installId: installId, observer: observer) }
    }

    public func cancelCodexInstall(installId: String) async throws(PageLampFailure) {
        try await call { try await core.cancelCodexInstall(installId: installId) }
    }

    public func removeCodex() async throws(PageLampFailure) {
        try await call { try await core.removeCodex() }
    }

    public func codexLogin(method: CodexLoginMethod, observer: any CodexLoginObserver) async throws(PageLampFailure) -> CodexStatus {
        try await call { try await core.codexLogin(method: method, observer: observer) }
    }

    public func cancelCodexLogin() async throws(PageLampFailure) {
        try await call { try await core.cancelCodexLogin() }
    }

    public func codexLogout() async throws(PageLampFailure) -> CodexStatus {
        try await call { try await core.codexLogout() }
    }

    public func setModeAWeeklyCap(runs: UInt32?) async throws(PageLampFailure) {
        try await call { try await core.setModeAWeeklyCap(runs: runs) }
    }

    public func setCodexSource(source: CodexSource) async throws(PageLampFailure) -> CodexStatus {
        try await call { try await core.setCodexSource(source: source) }
    }

    public func explainWeek(course: String, week: UInt32?, generationId: String, options: ExplainOptions, observer: any GenObserver) async throws(PageLampFailure) -> WeeklyExplanation {
        try await call { try await core.explainWeek(course: course, week: week, generationId: generationId, options: options, observer: observer) }
    }

    public func savedExplanations(course: String, week: UInt32?) async throws(PageLampFailure) -> [WeeklyExplanation] {
        try await call { try await core.savedExplanations(course: course, week: week) }
    }

    public func deleteExplanation(generationId: String) async throws(PageLampFailure) {
        try await call { try await core.deleteExplanation(generationId: generationId) }
    }

    public func aiOutputLanguage() async throws(PageLampFailure) -> OutputLanguage {
        try await call { try await core.aiOutputLanguage() }
    }

    public func setAiOutputLanguage(language: OutputLanguage) async throws(PageLampFailure) {
        try await call { try await core.setAiOutputLanguage(language: language) }
    }

    public func writeWeeklyNote(generationId: String, options: WeeklyNoteOptions, observer: any GenObserver) async throws(PageLampFailure) -> WeeklyNote {
        try await call { try await core.writeWeeklyNote(generationId: generationId, options: options, observer: observer) }
    }

    public func weeklyNotes() async throws(PageLampFailure) -> [WeeklyNote] {
        try await call { try await core.weeklyNotes() }
    }

    public func deleteWeeklyNote(generationId: String) async throws(PageLampFailure) {
        try await call { try await core.deleteWeeklyNote(generationId: generationId) }
    }

    public func weeklyNoteSettings() async throws(PageLampFailure) -> WeeklyNoteSettings {
        try await call { try await core.weeklyNoteSettings() }
    }

    public func setPrepareWeeklyNoteOnMonday(on: Bool) async throws(PageLampFailure) -> WeeklyNoteSettings {
        try await call { try await core.setPrepareWeeklyNoteOnMonday(on: on) }
    }

    public func generateStudyPlan(request: StudyPlanRequest, generationId: String, observer: any GenObserver) async throws(PageLampFailure) -> GeneratedStudyPlan {
        try await call { try await core.generateStudyPlan(request: request, generationId: generationId, observer: observer) }
    }

    public func acceptStudyPlan(generationId: String) async throws(PageLampFailure) -> StoredStudyPlan {
        try await call { try await core.acceptStudyPlan(generationId: generationId) }
    }

    public func setStudyPlanItemDone(planId: Int64, itemIndex: UInt32, done: Bool) async throws(PageLampFailure) -> StoredStudyPlan {
        try await call { try await core.setStudyPlanItemDone(planId: planId, itemIndex: itemIndex, done: done) }
    }

    public func cancelGeneration(generationId: String) async throws(PageLampFailure) {
        try await call { try await core.cancelGeneration(generationId: generationId) }
    }

    public func courseCalendar(course: String) async throws(PageLampFailure) -> CourseCalendarView {
        try await call { try await core.courseCalendar(course: course) }
    }

    public func calendarCandidates(course: String) async throws(PageLampFailure) -> [CalendarCandidate] {
        try await call { try await core.calendarCandidates(course: course) }
    }

    public func setCalendarSources(course: String, include: [String], exclude: [String]) async throws(PageLampFailure) -> [CalendarCandidate] {
        try await call { try await core.setCalendarSources(course: course, include: include, exclude: exclude) }
    }

    public func downloadMaterialFiles(course: String, materialIds: [String], observer: any SyncObserver) async throws(PageLampFailure) -> SourceSyncResult {
        try await call { try await core.downloadMaterialFiles(course: course, materialIds: materialIds, observer: observer) }
    }

    public func scanCourseCalendar(course: String) async throws(PageLampFailure) -> CalendarProposal? {
        try await call { try await core.scanCourseCalendar(course: course) }
    }

    public func setCourseDates(course: String, dates: CourseDatesInput?) async throws(PageLampFailure) -> CourseCalendarView {
        try await call { try await core.setCourseDates(course: course, dates: dates) }
    }

    public func acceptCalendarProposal(proposalId: Int64, edits: CourseDatesInput?) async throws(PageLampFailure) -> CourseCalendarView {
        try await call { try await core.acceptCalendarProposal(proposalId: proposalId, edits: edits) }
    }

    public func acceptPassingProposals(proposalIds: [Int64]) async throws(PageLampFailure) -> [CourseCalendarView] {
        try await call { try await core.acceptPassingProposals(proposalIds: proposalIds) }
    }

    public func dismissCalendarProposal(proposalId: Int64) async throws(PageLampFailure) {
        try await call { try await core.dismissCalendarProposal(proposalId: proposalId) }
    }

    public func syllabusReadingOffers() async throws(PageLampFailure) -> [SyllabusOffer] {
        try await call { try await core.syllabusReadingOffers() }
    }

    public func snoozeCalendarOffers() async throws(PageLampFailure) {
        try await call { try await core.snoozeCalendarOffers() }
    }

    public func readCourseCalendar(course: String, generationId: String, options: ReadCalendarOptions, observer: any GenObserver) async throws(PageLampFailure) -> CalendarProposal {
        try await call { try await core.readCourseCalendar(course: course, generationId: generationId, options: options, observer: observer) }
    }

    public func readCourseCalendars(courses: [String], batchId: String, options: ReadCalendarOptions, observer: any CalendarBatchObserver) async throws(PageLampFailure) -> [CalendarRunOutcome] {
        try await call { try await core.readCourseCalendars(courses: courses, batchId: batchId, options: options, observer: observer) }
    }

    public func removalPreview(courses: [String]) async throws(PageLampFailure) -> RemovalPreview {
        try await call { try await core.removalPreview(courses: courses) }
    }

    public func removeCourses(courses: [String], options: RemoveOptions) async throws(PageLampFailure) -> RemovalReport {
        try await call { try await core.removeCourses(courses: courses, options: options) }
    }

    public func removedCourses() async throws(PageLampFailure) -> [RemovedCourse] {
        try await call { try await core.removedCourses() }
    }

    public func restoreCourse(removedId: String) async throws(PageLampFailure) -> RestoreOutcome {
        try await call { try await core.restoreCourse(removedId: removedId) }
    }

    public func purgeRemovedCourses(removedIds: [String]?, permanentIfNoTrash: Bool) async throws(PageLampFailure) -> PurgeReport {
        try await call { try await core.purgeRemovedCourses(removedIds: removedIds, permanentIfNoTrash: permanentIfNoTrash) }
    }

    public func forgetRemovedCourse(removedId: String) async throws(PageLampFailure) {
        try await call { try await core.forgetRemovedCourse(removedId: removedId) }
    }

    public func weeklyDigest() async throws(PageLampFailure) -> WeeklyDigest {
        try await call { try await core.weeklyDigest() }
    }

    public func reminderSettings() async throws(PageLampFailure) -> ReminderSettings {
        try await call { try await core.reminderSettings() }
    }

    public func setReminderSettings(settings: ReminderSettings) async throws(PageLampFailure) {
        try await call { try await core.setReminderSettings(settings: settings) }
    }

    public func reminders(from: Date, to: Date) async throws(PageLampFailure) -> [Reminder] {
        try await call { try await core.reminders(from: from, to: to) }
    }

    public func dueReminders(now: Date) async throws(PageLampFailure) -> [Reminder] {
        try await call { try await core.dueReminders(now: now) }
    }

    public func markRemindersShown(ids: [String]) async throws(PageLampFailure) {
        try await call { try await core.markRemindersShown(ids: ids) }
    }

    public func materialLocalFile(materialId: String, purpose: LocalFileUse) async throws(PageLampFailure) -> String? {
        try await call { try await core.materialLocalFile(materialId: materialId, purpose: purpose) }
    }

    public func cancelSync() async throws(PageLampFailure) {
        try await call { try await core.cancelSync() }
    }

    public func syncAll(request: SyncRequest, observer: any SyncObserver) async throws(PageLampFailure) -> SyncSummary {
        try await call { try await core.syncAll(request: request, observer: observer) }
    }

    public func syncSource(sourceId: String, request: SyncRequest, observer: any SyncObserver) async throws(PageLampFailure) -> SourceSyncResult {
        try await call { try await core.syncSource(sourceId: sourceId, request: request, observer: observer) }
    }

    public func mcpClientConfigs(pagelampBinary: String) async throws(PageLampFailure) -> [McpClientConfig] {
        try await call { try await core.mcpClientConfigs(pagelampBinary: pagelampBinary) }
    }

    public func mcpLaunch(pagelampBinary: String) async throws(PageLampFailure) -> McpLaunch {
        try await call { try await core.mcpLaunch(pagelampBinary: pagelampBinary) }
    }

    public func doctor() async throws(PageLampFailure) -> DoctorReport {
        try await call { try await core.doctor() }
    }

    public func diagnosticReport() async throws(PageLampFailure) -> String {
        try await call { try await core.diagnosticReport() }
    }

    public func logsDir() async throws(PageLampFailure) -> String {
        try await call { try await core.logsDir() }
    }

    public func lastCrash() async throws(PageLampFailure) -> CrashReport? {
        try await call { try await core.lastCrash() }
    }

    public func clearLastCrash() async throws(PageLampFailure) {
        try await call { try await core.clearLastCrash() }
    }
}

/// The facade could not open (spec S2): data calls fail with the opening error; diagnostics
/// still work through the core's free functions, which use the default data folder.
public struct UnavailableService: PageLampService {
    public let failure: PageLampFailure

    public init(failure: PageLampFailure) {
        self.failure = failure
    }

    public func status() async throws(PageLampFailure) -> AppStatus { throw failure }
    public func listCourses() async throws(PageLampFailure) -> [CourseSummary] { throw failure }
    public func listSources() async throws(PageLampFailure) -> [SourceRecord] { throw failure }

    public func listDeadlines(course: String?, daysAhead: UInt32, daysBack: UInt32) async throws(PageLampFailure) -> [Deadline] {
        throw failure
    }

    public func latestStudyPlan() async throws(PageLampFailure) -> StoredStudyPlan? { throw failure }
    public func courseOverview(course: String) async throws(PageLampFailure) -> CourseOverview { throw failure }

    public func weekMaterials(course: String, week: UInt32?) async throws(PageLampFailure) -> WeekMaterials {
        throw failure
    }

    public func courseTimeline(course: String) async throws(PageLampFailure) -> CourseTimeline { throw failure }
    public func lifecycleSummary() async throws(PageLampFailure) -> LifecycleSummary { throw failure }

    public func keepCourseCurrent(course: String, until: String?) async throws(PageLampFailure) -> Course {
        throw failure
    }

    public func clearKeepCourseCurrent(course: String) async throws(PageLampFailure) -> Course { throw failure }
    public func snoozeRemovalSuggestions(courses: [String], kind: SnoozeKind) async throws(PageLampFailure) { throw failure }
    public func clearRemovalSnooze(courses: [String]) async throws(PageLampFailure) { throw failure }
    public func snoozeLifecycleBanner() async throws(PageLampFailure) { throw failure }
    public func confirmCourseDates(course: String) async throws(PageLampFailure) -> CourseTimeline { throw failure }

    public func startupTasks(now: Date) async throws(PageLampFailure) -> StartupTasks { throw failure }
    public func updatePrefs() async throws(PageLampFailure) -> UpdatePrefs { throw failure }
    public func setUpdatePrefs(prefs: UpdatePrefs) async throws(PageLampFailure) { throw failure }
    public func effectiveUpdateChannel() async throws(PageLampFailure) -> UpdateChannel { throw failure }
    public func acknowledgeWhatsNew() async throws(PageLampFailure) { throw failure }
    public func acknowledgeUpdateDisclosure() async throws(PageLampFailure) { throw failure }
    public func recordUpdateCheck(record: UpdateCheckRecord) async throws(PageLampFailure) { throw failure }
    public func lastUpdateCheck() async throws(PageLampFailure) -> UpdateCheckRecord? { throw failure }
    public func activity() async throws(PageLampFailure) -> Activity { throw failure }

    public func modelProviderPresets() async throws(PageLampFailure) -> [ProviderPreset] { throw failure }
    public func detectLocalServers() async throws(PageLampFailure) -> [LocalServer] { throw failure }
    public func aiStatus() async throws(PageLampFailure) -> AiStatus { throw failure }
    public func addModelProvider(preset: String, baseUrl: String?, apiKey: String?) async throws(PageLampFailure) -> ModelProviderRecord { throw failure }
    public func updateModelProviderKey(providerId: String, apiKey: String) async throws(PageLampFailure) -> ModelProviderRecord { throw failure }
    public func removeModelProvider(providerId: String) async throws(PageLampFailure) { throw failure }
    public func listModels(backend: BackendRef) async throws(PageLampFailure) -> [ModelInfo] { throw failure }
    public func testModel(backend: BackendRef, model: String) async throws(PageLampFailure) -> ProbeReport { throw failure }
    public func setFeatureModel(feature: AiFeature, choice: ModelChoice?) async throws(PageLampFailure) { throw failure }
    public func acknowledgeAiDisclosure(backend: BackendRef, version: UInt32) async throws(PageLampFailure) { throw failure }
    public func acknowledgeUnpricedModel(backend: BackendRef, model: String) async throws(PageLampFailure) { throw failure }
    public func setMonthlyBudget(microUsd: UInt64?) async throws(PageLampFailure) { throw failure }
    public func estimateGeneration(request: EstimateRequest) async throws(PageLampFailure) -> CostEstimate { throw failure }
    public func usageSummary(month: String?) async throws(PageLampFailure) -> UsageSummary { throw failure }
    public func setCourseMaterialSharing(course: String, answer: MaterialSharing) async throws(PageLampFailure) { throw failure }
    public func deleteGenerated(course: String?) async throws(PageLampFailure) -> UInt32 { throw failure }
    public func removeAllAiData() async throws(PageLampFailure) -> RemoveAiDataReport { throw failure }
    public func codexStatus() async throws(PageLampFailure) -> CodexStatus { throw failure }
    public func installCodex(installId: String, observer: any CodexInstallObserver) async throws(PageLampFailure) -> CodexStatus { throw failure }
    public func cancelCodexInstall(installId: String) async throws(PageLampFailure) { throw failure }
    public func removeCodex() async throws(PageLampFailure) { throw failure }
    public func codexLogin(method: CodexLoginMethod, observer: any CodexLoginObserver) async throws(PageLampFailure) -> CodexStatus { throw failure }
    public func cancelCodexLogin() async throws(PageLampFailure) { throw failure }
    public func codexLogout() async throws(PageLampFailure) -> CodexStatus { throw failure }
    public func setModeAWeeklyCap(runs: UInt32?) async throws(PageLampFailure) { throw failure }
    public func setCodexSource(source: CodexSource) async throws(PageLampFailure) -> CodexStatus { throw failure }
    public func explainWeek(course: String, week: UInt32?, generationId: String, options: ExplainOptions, observer: any GenObserver) async throws(PageLampFailure) -> WeeklyExplanation { throw failure }
    public func savedExplanations(course: String, week: UInt32?) async throws(PageLampFailure) -> [WeeklyExplanation] { throw failure }
    public func deleteExplanation(generationId: String) async throws(PageLampFailure) { throw failure }
    public func aiOutputLanguage() async throws(PageLampFailure) -> OutputLanguage { throw failure }
    public func setAiOutputLanguage(language: OutputLanguage) async throws(PageLampFailure) { throw failure }
    public func writeWeeklyNote(generationId: String, options: WeeklyNoteOptions, observer: any GenObserver) async throws(PageLampFailure) -> WeeklyNote { throw failure }
    public func weeklyNotes() async throws(PageLampFailure) -> [WeeklyNote] { throw failure }
    public func deleteWeeklyNote(generationId: String) async throws(PageLampFailure) { throw failure }
    public func weeklyNoteSettings() async throws(PageLampFailure) -> WeeklyNoteSettings { throw failure }
    public func setPrepareWeeklyNoteOnMonday(on: Bool) async throws(PageLampFailure) -> WeeklyNoteSettings { throw failure }
    public func generateStudyPlan(request: StudyPlanRequest, generationId: String, observer: any GenObserver) async throws(PageLampFailure) -> GeneratedStudyPlan { throw failure }
    public func acceptStudyPlan(generationId: String) async throws(PageLampFailure) -> StoredStudyPlan { throw failure }
    public func setStudyPlanItemDone(planId: Int64, itemIndex: UInt32, done: Bool) async throws(PageLampFailure) -> StoredStudyPlan { throw failure }
    public func cancelGeneration(generationId: String) async throws(PageLampFailure) { throw failure }
    public func courseCalendar(course: String) async throws(PageLampFailure) -> CourseCalendarView { throw failure }
    public func calendarCandidates(course: String) async throws(PageLampFailure) -> [CalendarCandidate] { throw failure }
    public func setCalendarSources(course: String, include: [String], exclude: [String]) async throws(PageLampFailure) -> [CalendarCandidate] { throw failure }
    public func downloadMaterialFiles(course: String, materialIds: [String], observer: any SyncObserver) async throws(PageLampFailure) -> SourceSyncResult { throw failure }
    public func scanCourseCalendar(course: String) async throws(PageLampFailure) -> CalendarProposal? { throw failure }
    public func setCourseDates(course: String, dates: CourseDatesInput?) async throws(PageLampFailure) -> CourseCalendarView { throw failure }
    public func acceptCalendarProposal(proposalId: Int64, edits: CourseDatesInput?) async throws(PageLampFailure) -> CourseCalendarView { throw failure }
    public func acceptPassingProposals(proposalIds: [Int64]) async throws(PageLampFailure) -> [CourseCalendarView] { throw failure }
    public func dismissCalendarProposal(proposalId: Int64) async throws(PageLampFailure) { throw failure }
    public func syllabusReadingOffers() async throws(PageLampFailure) -> [SyllabusOffer] { throw failure }
    public func snoozeCalendarOffers() async throws(PageLampFailure) { throw failure }
    public func readCourseCalendar(course: String, generationId: String, options: ReadCalendarOptions, observer: any GenObserver) async throws(PageLampFailure) -> CalendarProposal { throw failure }
    public func readCourseCalendars(courses: [String], batchId: String, options: ReadCalendarOptions, observer: any CalendarBatchObserver) async throws(PageLampFailure) -> [CalendarRunOutcome] { throw failure }
    public func removalPreview(courses: [String]) async throws(PageLampFailure) -> RemovalPreview { throw failure }
    public func removeCourses(courses: [String], options: RemoveOptions) async throws(PageLampFailure) -> RemovalReport { throw failure }
    public func removedCourses() async throws(PageLampFailure) -> [RemovedCourse] { throw failure }
    public func restoreCourse(removedId: String) async throws(PageLampFailure) -> RestoreOutcome { throw failure }
    public func purgeRemovedCourses(removedIds: [String]?, permanentIfNoTrash: Bool) async throws(PageLampFailure) -> PurgeReport { throw failure }
    public func forgetRemovedCourse(removedId: String) async throws(PageLampFailure) { throw failure }
    public func weeklyDigest() async throws(PageLampFailure) -> WeeklyDigest { throw failure }
    public func reminderSettings() async throws(PageLampFailure) -> ReminderSettings { throw failure }
    public func setReminderSettings(settings: ReminderSettings) async throws(PageLampFailure) { throw failure }
    public func reminders(from: Date, to: Date) async throws(PageLampFailure) -> [Reminder] { throw failure }
    public func dueReminders(now: Date) async throws(PageLampFailure) -> [Reminder] { throw failure }
    public func markRemindersShown(ids: [String]) async throws(PageLampFailure) { throw failure }
    public func materialLocalFile(materialId: String, purpose: LocalFileUse) async throws(PageLampFailure) -> String? { throw failure }
    public func cancelSync() async throws(PageLampFailure) { throw failure }

    public func syncAll(request: SyncRequest, observer: any SyncObserver) async throws(PageLampFailure) -> SyncSummary {
        throw failure
    }

    public func syncSource(sourceId: String, request: SyncRequest, observer: any SyncObserver) async throws(PageLampFailure) -> SourceSyncResult {
        throw failure
    }

    public func mcpClientConfigs(pagelampBinary: String) async throws(PageLampFailure) -> [McpClientConfig] {
        throw failure
    }

    public func mcpLaunch(pagelampBinary: String) async throws(PageLampFailure) -> McpLaunch { throw failure }

    public func doctor() async throws(PageLampFailure) -> DoctorReport {
        do { return try await diagnosticsDoctor() } catch { throw PageLampFailure.from(error) }
    }

    public func diagnosticReport() async throws(PageLampFailure) -> String {
        do { return try await diagnosticsReport() } catch { throw PageLampFailure.from(error) }
    }

    public func logsDir() async throws(PageLampFailure) -> String {
        do { return try await diagnosticsLogsDir() } catch { throw PageLampFailure.from(error) }
    }

    public func lastCrash() async throws(PageLampFailure) -> CrashReport? {
        do { return try await diagnosticsLastCrash() } catch { throw PageLampFailure.from(error) }
    }

    public func clearLastCrash() async throws(PageLampFailure) {
        do { try await diagnosticsClearLastCrash() } catch { throw PageLampFailure.from(error) }
    }
}
