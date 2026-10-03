// A service that wraps another one (`FixtureService`, test doubles): every call goes to `base`
// unless the wrapper implements it itself, so wrappers write only what they change.

import Foundation
import PageLampKit

/// A `PageLampService` that forwards every call it doesn't implement to `base`.
public protocol ForwardingService: PageLampService {
    var base: any PageLampService { get }
}

extension ForwardingService {
    public func status() async throws(PageLampFailure) -> AppStatus {
        try await base.status()
    }

    public func listCourses() async throws(PageLampFailure) -> [CourseSummary] {
        try await base.listCourses()
    }

    public func listSources() async throws(PageLampFailure) -> [SourceRecord] {
        try await base.listSources()
    }

    public func listDeadlines(course: String?, daysAhead: UInt32, daysBack: UInt32) async throws(PageLampFailure) -> [Deadline] {
        try await base.listDeadlines(course: course, daysAhead: daysAhead, daysBack: daysBack)
    }

    public func latestStudyPlan() async throws(PageLampFailure) -> StoredStudyPlan? {
        try await base.latestStudyPlan()
    }

    public func courseOverview(course: String) async throws(PageLampFailure) -> CourseOverview {
        try await base.courseOverview(course: course)
    }

    public func weekMaterials(course: String, week: UInt32?) async throws(PageLampFailure) -> WeekMaterials {
        try await base.weekMaterials(course: course, week: week)
    }

    public func courseTimeline(course: String) async throws(PageLampFailure) -> CourseTimeline {
        try await base.courseTimeline(course: course)
    }

    public func lifecycleSummary() async throws(PageLampFailure) -> LifecycleSummary {
        try await base.lifecycleSummary()
    }

    public func keepCourseCurrent(course: String, until: String?) async throws(PageLampFailure) -> Course {
        try await base.keepCourseCurrent(course: course, until: until)
    }

    public func clearKeepCourseCurrent(course: String) async throws(PageLampFailure) -> Course {
        try await base.clearKeepCourseCurrent(course: course)
    }

    public func snoozeRemovalSuggestions(courses: [String], kind: SnoozeKind) async throws(PageLampFailure) {
        try await base.snoozeRemovalSuggestions(courses: courses, kind: kind)
    }

    public func clearRemovalSnooze(courses: [String]) async throws(PageLampFailure) {
        try await base.clearRemovalSnooze(courses: courses)
    }

    public func snoozeLifecycleBanner() async throws(PageLampFailure) {
        try await base.snoozeLifecycleBanner()
    }

    public func confirmCourseDates(course: String) async throws(PageLampFailure) -> CourseTimeline {
        try await base.confirmCourseDates(course: course)
    }

    public func startupTasks(now: Date) async throws(PageLampFailure) -> StartupTasks {
        try await base.startupTasks(now: now)
    }

    public func updatePrefs() async throws(PageLampFailure) -> UpdatePrefs {
        try await base.updatePrefs()
    }

    public func setUpdatePrefs(prefs: UpdatePrefs) async throws(PageLampFailure) {
        try await base.setUpdatePrefs(prefs: prefs)
    }

    public func effectiveUpdateChannel() async throws(PageLampFailure) -> UpdateChannel {
        try await base.effectiveUpdateChannel()
    }

    public func acknowledgeWhatsNew() async throws(PageLampFailure) {
        try await base.acknowledgeWhatsNew()
    }

    public func acknowledgeUpdateDisclosure() async throws(PageLampFailure) {
        try await base.acknowledgeUpdateDisclosure()
    }

    public func recordUpdateCheck(record: UpdateCheckRecord) async throws(PageLampFailure) {
        try await base.recordUpdateCheck(record: record)
    }

    public func lastUpdateCheck() async throws(PageLampFailure) -> UpdateCheckRecord? {
        try await base.lastUpdateCheck()
    }

    public func activity() async throws(PageLampFailure) -> Activity {
        try await base.activity()
    }

    public func modelProviderPresets() async throws(PageLampFailure) -> [ProviderPreset] {
        try await base.modelProviderPresets()
    }

    public func detectLocalServers() async throws(PageLampFailure) -> [LocalServer] {
        try await base.detectLocalServers()
    }

    public func aiStatus() async throws(PageLampFailure) -> AiStatus {
        try await base.aiStatus()
    }

    public func addModelProvider(preset: String, baseUrl: String?, apiKey: String?) async throws(PageLampFailure) -> ModelProviderRecord {
        try await base.addModelProvider(preset: preset, baseUrl: baseUrl, apiKey: apiKey)
    }

    public func updateModelProviderKey(providerId: String, apiKey: String) async throws(PageLampFailure) -> ModelProviderRecord {
        try await base.updateModelProviderKey(providerId: providerId, apiKey: apiKey)
    }

    public func removeModelProvider(providerId: String) async throws(PageLampFailure) {
        try await base.removeModelProvider(providerId: providerId)
    }

    public func listModels(backend: BackendRef) async throws(PageLampFailure) -> [ModelInfo] {
        try await base.listModels(backend: backend)
    }

    public func testModel(backend: BackendRef, model: String) async throws(PageLampFailure) -> ProbeReport {
        try await base.testModel(backend: backend, model: model)
    }

    public func setFeatureModel(feature: AiFeature, choice: ModelChoice?) async throws(PageLampFailure) {
        try await base.setFeatureModel(feature: feature, choice: choice)
    }

    public func acknowledgeAiDisclosure(backend: BackendRef, version: UInt32) async throws(PageLampFailure) {
        try await base.acknowledgeAiDisclosure(backend: backend, version: version)
    }

    public func acknowledgeUnpricedModel(backend: BackendRef, model: String) async throws(PageLampFailure) {
        try await base.acknowledgeUnpricedModel(backend: backend, model: model)
    }

    public func setMonthlyBudget(microUsd: UInt64?) async throws(PageLampFailure) {
        try await base.setMonthlyBudget(microUsd: microUsd)
    }

    public func estimateGeneration(request: EstimateRequest) async throws(PageLampFailure) -> CostEstimate {
        try await base.estimateGeneration(request: request)
    }

    public func usageSummary(month: String?) async throws(PageLampFailure) -> UsageSummary {
        try await base.usageSummary(month: month)
    }

    public func setCourseMaterialSharing(course: String, answer: MaterialSharing) async throws(PageLampFailure) {
        try await base.setCourseMaterialSharing(course: course, answer: answer)
    }

    public func deleteGenerated(course: String?) async throws(PageLampFailure) -> UInt32 {
        try await base.deleteGenerated(course: course)
    }

    public func removeAllAiData() async throws(PageLampFailure) -> RemoveAiDataReport {
        try await base.removeAllAiData()
    }

    public func codexStatus() async throws(PageLampFailure) -> CodexStatus {
        try await base.codexStatus()
    }

    public func installCodex(installId: String, observer: any CodexInstallObserver) async throws(PageLampFailure) -> CodexStatus {
        try await base.installCodex(installId: installId, observer: observer)
    }

    public func cancelCodexInstall(installId: String) async throws(PageLampFailure) {
        try await base.cancelCodexInstall(installId: installId)
    }

    public func removeCodex() async throws(PageLampFailure) {
        try await base.removeCodex()
    }

    public func codexLogin(method: CodexLoginMethod, observer: any CodexLoginObserver) async throws(PageLampFailure) -> CodexStatus {
        try await base.codexLogin(method: method, observer: observer)
    }

    public func cancelCodexLogin() async throws(PageLampFailure) {
        try await base.cancelCodexLogin()
    }

    public func codexLogout() async throws(PageLampFailure) -> CodexStatus {
        try await base.codexLogout()
    }

    public func setModeAWeeklyCap(runs: UInt32?) async throws(PageLampFailure) {
        try await base.setModeAWeeklyCap(runs: runs)
    }

    public func setCodexSource(source: CodexSource) async throws(PageLampFailure) -> CodexStatus {
        try await base.setCodexSource(source: source)
    }

    public func explainWeek(course: String, week: UInt32?, generationId: String, options: ExplainOptions, observer: any GenObserver) async throws(PageLampFailure) -> WeeklyExplanation {
        try await base.explainWeek(course: course, week: week, generationId: generationId, options: options, observer: observer)
    }

    public func savedExplanations(course: String, week: UInt32?) async throws(PageLampFailure) -> [WeeklyExplanation] {
        try await base.savedExplanations(course: course, week: week)
    }

    public func deleteExplanation(generationId: String) async throws(PageLampFailure) {
        try await base.deleteExplanation(generationId: generationId)
    }

    public func aiOutputLanguage() async throws(PageLampFailure) -> OutputLanguage {
        try await base.aiOutputLanguage()
    }

    public func setAiOutputLanguage(language: OutputLanguage) async throws(PageLampFailure) {
        try await base.setAiOutputLanguage(language: language)
    }

    public func writeWeeklyNote(generationId: String, options: WeeklyNoteOptions, observer: any GenObserver) async throws(PageLampFailure) -> WeeklyNote {
        try await base.writeWeeklyNote(generationId: generationId, options: options, observer: observer)
    }

    public func weeklyNotes() async throws(PageLampFailure) -> [WeeklyNote] {
        try await base.weeklyNotes()
    }

    public func deleteWeeklyNote(generationId: String) async throws(PageLampFailure) {
        try await base.deleteWeeklyNote(generationId: generationId)
    }

    public func weeklyNoteSettings() async throws(PageLampFailure) -> WeeklyNoteSettings {
        try await base.weeklyNoteSettings()
    }

    public func setPrepareWeeklyNoteOnMonday(on: Bool) async throws(PageLampFailure) -> WeeklyNoteSettings {
        try await base.setPrepareWeeklyNoteOnMonday(on: on)
    }

    public func generateStudyPlan(request: StudyPlanRequest, generationId: String, observer: any GenObserver) async throws(PageLampFailure) -> GeneratedStudyPlan {
        try await base.generateStudyPlan(request: request, generationId: generationId, observer: observer)
    }

    public func acceptStudyPlan(generationId: String) async throws(PageLampFailure) -> StoredStudyPlan {
        try await base.acceptStudyPlan(generationId: generationId)
    }

    public func setStudyPlanItemDone(planId: Int64, itemIndex: UInt32, done: Bool) async throws(PageLampFailure) -> StoredStudyPlan {
        try await base.setStudyPlanItemDone(planId: planId, itemIndex: itemIndex, done: done)
    }

    public func cancelGeneration(generationId: String) async throws(PageLampFailure) {
        try await base.cancelGeneration(generationId: generationId)
    }

    public func courseCalendar(course: String) async throws(PageLampFailure) -> CourseCalendarView {
        try await base.courseCalendar(course: course)
    }

    public func calendarCandidates(course: String) async throws(PageLampFailure) -> [CalendarCandidate] {
        try await base.calendarCandidates(course: course)
    }

    public func setCalendarSources(course: String, include: [String], exclude: [String]) async throws(PageLampFailure) -> [CalendarCandidate] {
        try await base.setCalendarSources(course: course, include: include, exclude: exclude)
    }

    public func downloadMaterialFiles(course: String, materialIds: [String], observer: any SyncObserver) async throws(PageLampFailure) -> SourceSyncResult {
        try await base.downloadMaterialFiles(course: course, materialIds: materialIds, observer: observer)
    }

    public func scanCourseCalendar(course: String) async throws(PageLampFailure) -> CalendarProposal? {
        try await base.scanCourseCalendar(course: course)
    }

    public func setCourseDates(course: String, dates: CourseDatesInput?) async throws(PageLampFailure) -> CourseCalendarView {
        try await base.setCourseDates(course: course, dates: dates)
    }

    public func acceptCalendarProposal(proposalId: Int64, edits: CourseDatesInput?) async throws(PageLampFailure) -> CourseCalendarView {
        try await base.acceptCalendarProposal(proposalId: proposalId, edits: edits)
    }

    public func acceptPassingProposals(proposalIds: [Int64]) async throws(PageLampFailure) -> [CourseCalendarView] {
        try await base.acceptPassingProposals(proposalIds: proposalIds)
    }

    public func dismissCalendarProposal(proposalId: Int64) async throws(PageLampFailure) {
        try await base.dismissCalendarProposal(proposalId: proposalId)
    }

    public func syllabusReadingOffers() async throws(PageLampFailure) -> [SyllabusOffer] {
        try await base.syllabusReadingOffers()
    }

    public func snoozeCalendarOffers() async throws(PageLampFailure) {
        try await base.snoozeCalendarOffers()
    }

    public func readCourseCalendar(course: String, generationId: String, options: ReadCalendarOptions, observer: any GenObserver) async throws(PageLampFailure) -> CalendarProposal {
        try await base.readCourseCalendar(course: course, generationId: generationId, options: options, observer: observer)
    }

    public func readCourseCalendars(courses: [String], batchId: String, options: ReadCalendarOptions, observer: any CalendarBatchObserver) async throws(PageLampFailure) -> [CalendarRunOutcome] {
        try await base.readCourseCalendars(courses: courses, batchId: batchId, options: options, observer: observer)
    }

    public func removalPreview(courses: [String]) async throws(PageLampFailure) -> RemovalPreview {
        try await base.removalPreview(courses: courses)
    }

    public func removeCourses(courses: [String], options: RemoveOptions) async throws(PageLampFailure) -> RemovalReport {
        try await base.removeCourses(courses: courses, options: options)
    }

    public func removedCourses() async throws(PageLampFailure) -> [RemovedCourse] {
        try await base.removedCourses()
    }

    public func restoreCourse(removedId: String) async throws(PageLampFailure) -> RestoreOutcome {
        try await base.restoreCourse(removedId: removedId)
    }

    public func purgeRemovedCourses(removedIds: [String]?, permanentIfNoTrash: Bool) async throws(PageLampFailure) -> PurgeReport {
        try await base.purgeRemovedCourses(removedIds: removedIds, permanentIfNoTrash: permanentIfNoTrash)
    }

    public func forgetRemovedCourse(removedId: String) async throws(PageLampFailure) {
        try await base.forgetRemovedCourse(removedId: removedId)
    }

    public func weeklyDigest() async throws(PageLampFailure) -> WeeklyDigest {
        try await base.weeklyDigest()
    }

    public func reminderSettings() async throws(PageLampFailure) -> ReminderSettings {
        try await base.reminderSettings()
    }

    public func setReminderSettings(settings: ReminderSettings) async throws(PageLampFailure) {
        try await base.setReminderSettings(settings: settings)
    }

    public func reminders(from: Date, to: Date) async throws(PageLampFailure) -> [Reminder] {
        try await base.reminders(from: from, to: to)
    }

    public func dueReminders(now: Date) async throws(PageLampFailure) -> [Reminder] {
        try await base.dueReminders(now: now)
    }

    public func markRemindersShown(ids: [String]) async throws(PageLampFailure) {
        try await base.markRemindersShown(ids: ids)
    }

    public func materialLocalFile(materialId: String, purpose: LocalFileUse) async throws(PageLampFailure) -> String? {
        try await base.materialLocalFile(materialId: materialId, purpose: purpose)
    }

    public func cancelSync() async throws(PageLampFailure) {
        try await base.cancelSync()
    }

    public func syncAll(request: SyncRequest, observer: any SyncObserver) async throws(PageLampFailure) -> SyncSummary {
        try await base.syncAll(request: request, observer: observer)
    }

    public func syncSource(sourceId: String, request: SyncRequest, observer: any SyncObserver) async throws(PageLampFailure) -> SourceSyncResult {
        try await base.syncSource(sourceId: sourceId, request: request, observer: observer)
    }

    public func mcpClientConfigs(pagelampBinary: String) async throws(PageLampFailure) -> [McpClientConfig] {
        try await base.mcpClientConfigs(pagelampBinary: pagelampBinary)
    }

    public func mcpLaunch(pagelampBinary: String) async throws(PageLampFailure) -> McpLaunch {
        try await base.mcpLaunch(pagelampBinary: pagelampBinary)
    }

    public func doctor() async throws(PageLampFailure) -> DoctorReport {
        try await base.doctor()
    }

    public func diagnosticReport() async throws(PageLampFailure) -> String {
        try await base.diagnosticReport()
    }

    public func logsDir() async throws(PageLampFailure) -> String {
        try await base.logsDir()
    }

    public func lastCrash() async throws(PageLampFailure) -> CrashReport? {
        try await base.lastCrash()
    }

    public func clearLastCrash() async throws(PageLampFailure) {
        try await base.clearLastCrash()
    }
}
