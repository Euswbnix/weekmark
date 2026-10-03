// In-memory PageLamp for the preview build and tests, ported from the Tauri app's
// apps/desktop/src/api/mock/index.ts. It behaves like the facade where the UI can tell the
// difference: the same failure kinds, sync events streamed over time, `busy` while syncing.

import Foundation
import PageLampKit

/// Synthetic PageLamp: the preview app's default data. Never touches the disk (except a temp
/// "logs" folder), the keychain or the network.
public actor MockService: PageLampService {
    /// Simulated latency of every call and the delay between streamed sync events.
    public struct Timing: Sendable {
        public var latency: Duration
        public var syncStep: Duration
        /// Tests: the sync stops before every progress step until the gate lets it through
        /// (instead of waiting `syncStep`).
        public var gate: SyncStepGate?

        public init(latency: Duration, syncStep: Duration, gate: SyncStepGate? = nil) {
            self.latency = latency
            self.syncStep = syncStep
            self.gate = gate
        }

        /// Feels like the real thing in the preview app.
        public static let interactive = Timing(latency: .milliseconds(120), syncStep: .milliseconds(350))
        /// No waiting (tests, snapshots).
        public static let instant = Timing(latency: .zero, syncStep: .zero)
    }

    /// Where the mock pretends the bundled CLI lives (Connect shows it in snippets).
    public static let binaryPath = "/Applications/PageLamp Preview.app/Contents/MacOS/pagelamp"

    public nonisolated let scenario: MockScenario
    private let timing: Timing
    let now: @Sendable () -> Date
    let calendar: Calendar
    var db: MockDb
    private var syncing = false
    /// When the running sync started (`activity()`).
    private var syncStartedAt: Date?
    /// How often each call ran (tests).
    public private(set) var calls: [String: Int] = [:]

    public init(
        scenario: MockScenario = .preview,
        timing: Timing = .interactive,
        calendar: Calendar = .current,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.scenario = scenario
        self.timing = timing
        self.now = now
        self.calendar = calendar
        db = MockFixtures.database(scenario, now: now(), calendar: calendar)
        db.features.ai = MockAi.start(scenario, now: now(), calendar: calendar)
        // As in the Tauri mock: DEMO205's materials may not be shared with a cloud AI service.
        if let demo205 = db.courses.first(where: { $0.course.code == "DEMO205" }) {
            db.features.sharing[demo205.course.id] = .notAllowed
        }
        db.features.prepareNoteOnMonday = scenario == .weeklyNoteMonday
    }

    // MARK: - Debug controls (preview Debug menu, tests)

    /// Makes the Canvas token "expire": the next sync of Demo Canvas fails with `auth`.
    public func expireCanvasToken() {
        guard let index = db.sources.firstIndex(where: { $0.kind == .canvas }) else { return }
        db.sources[index].lastError = "Canvas rejected the access token (401). It may have expired or been revoked."
        db.sources[index].lastErrorKind = .authExpiredOrRevoked
    }

    /// Simulates the CLI holding the sync lock (every sync fails with `busy`).
    public func setExternalSyncRunning(_ running: Bool) {
        db.externalSyncRunning = running
    }

    /// Simulates a launch after an update from `version` (nil: from 0.1, which recorded none):
    /// `startupTasks` offers What's new until `acknowledgeWhatsNew`.
    public func simulateUpgrade(from version: String?) {
        db.updates.upgradedFrom = version
        db.updates.upgraded = true
        db.updates.whatsNewSeen = false
    }

    public func callCount(_ name: String) -> Int {
        calls[name, default: 0]
    }

    // MARK: - Helpers

    func respond(_ name: String) async {
        calls[name, default: 0] += 1
        await pause(timing.latency)
    }

    private func pause(_ duration: Duration) async {
        guard duration > .zero else { return }
        try? await Task.sleep(for: duration)
    }

    /// A model run's step: held at the tests' gate (at the run's id), else a sync step's wait.
    func generationStep(_ generationId: String, _ step: UInt32) async {
        if let gate = timing.gate {
            await gate.pass(SyncStepPosition(sourceId: generationId, step: step))
        } else {
            await pause(timing.syncStep)
        }
    }

    func courseIndex(_ reference: String) throws(PageLampFailure) -> Int {
        if let index = db.courses.firstIndex(where: { $0.course.id == reference || $0.course.code == reference }) {
            return index
        }
        throw PageLampFailure(kind: .notFound, message: "No course with id \(reference)")
    }

    private func sourceIndex(_ id: String) throws(PageLampFailure) -> Int {
        if let index = db.sources.firstIndex(where: { $0.id == id }) {
            return index
        }
        throw PageLampFailure(kind: .notFound, message: "No source with id \(id)")
    }

    private func sourceLabel(_ id: String) -> String {
        db.sources.first { $0.id == id }?.label ?? id
    }

    private func sourceSyncedAt(_ id: String) -> Date? {
        db.sources.first { $0.id == id }?.lastSyncedAt
    }

    private static func when(_ deadline: Deadline) -> Date? {
        deadline.event.dueAt ?? deadline.event.startsAt
    }

    func deadlines(in courses: [MockCourse], daysAhead: Int, daysBack: Int, at date: Date? = nil) -> [Deadline] {
        let t = date ?? now()
        let from = t.addingTimeInterval(-Double(daysBack) * 86_400)
        let to = t.addingTimeInterval(Double(daysAhead) * 86_400)
        return courses
            .flatMap(\.deadlines)
            .filter { deadline in
                guard let when = Self.when(deadline) else { return false }
                return when >= from && when <= to
            }
            .sorted { (Self.when($0) ?? .distantPast) < (Self.when($1) ?? .distantPast) }
    }

    /// "YYYY-MM-DD" of the day `days` after today, in the mock's calendar.
    func isoDay(daysFromToday days: Int) -> String {
        let day = calendar.date(byAdding: .day, value: days, to: calendar.startOfDay(for: now())) ?? now()
        return IsoDate.string(from: day, calendar: calendar)
    }

    static func aiMaterials(_ course: Course) -> AiMaterialsState {
        if course.aiPolicy == .prohibited { return .withheldByPolicy }
        if !course.aiAccess { return .turnedOff }
        return .readable
    }

    /// The course as the facade returns it: with the student's answer about sharing its materials
    /// (`setCourseMaterialSharing`, and DEMO205's from `init`), which the mock keeps apart from
    /// the fixture. Records are immutable, so a changed answer makes a new one.
    func courseRecord(_ course: MockCourse) -> Course {
        let record = course.course
        guard let answer = db.features.sharing[record.id], answer != record.materialSharing else { return record }
        return Course(
            id: record.id, sourceId: record.sourceId, externalId: record.externalId, code: record.code,
            name: record.name, termStart: record.termStart, termEnd: record.termEnd, termSource: record.termSource,
            url: record.url, aiPolicy: record.aiPolicy, aiPolicyNote: record.aiPolicyNote, aiAccess: record.aiAccess,
            materialSharing: answer, hidden: record.hidden, enrollmentActive: record.enrollmentActive,
            updatedAt: record.updatedAt
        )
    }

    /// Every week with a module or material, plus the current week, ascending (like Rust).
    private static func availableWeeks(_ course: MockCourse) -> [UInt32] {
        var weeks = Set(course.modules.compactMap(\.weekHint) + course.materials.compactMap(\.weekHint))
        if let current = course.timeline.currentWeek { weeks.insert(current) }
        return weeks.sorted()
    }

    func summary(_ course: MockCourse) -> CourseSummary {
        let upcoming = deadlines(in: [course], daysAhead: 21, daysBack: 0).filter { $0.event.kind != .classEvent }
        let aiMaterials = Self.aiMaterials(course.course)
        // Like the facade: "readable by your AI app" is 0 unless the AI may read materials.
        let readable = aiMaterials == .readable ? course.materials.filter { $0.textStatus == .ok }.count : 0
        return CourseSummary(
            course: courseRecord(course),
            aiMaterials: aiMaterials,
            timeline: course.timeline,
            lifecycle: MockCalendar.lifecycle(course.timeline, keptCurrentUntil: course.keptCurrentUntil),
            counts: CourseCounts(
                modules: UInt32(course.modules.count),
                materials: UInt32(course.materials.count),
                indexedMaterials: UInt32(readable),
                upcomingDeadlines: UInt32(upcoming.count)
            ),
            nextDeadline: upcoming.first,
            sourceLabel: sourceLabel(course.course.sourceId),
            lastSyncedAt: sourceSyncedAt(course.course.sourceId)
        )
    }

    private func makeStatus() -> AppStatus {
        let visible = db.courses.filter { !$0.course.hidden }
        let materials = db.courses.flatMap(\.materials)
        let indexed = materials.filter { $0.textStatus == .ok }
        return AppStatus(
            version: "0.1.0-mock",
            dataDir: db.dataDir,
            dbPath: "\(db.dataDir)/pagelamp.db",
            sources: db.sources.map(\.record),
            counts: StoreCounts(
                courses: UInt32(visible.count),
                hiddenCourses: UInt32(db.courses.count - visible.count),
                modules: UInt32(db.courses.reduce(0) { $0 + $1.modules.count }),
                materials: UInt32(materials.count),
                indexedMaterials: UInt32(indexed.count),
                chunks: indexed.reduce(0) { $0 + $1.chunkCount },
                events: UInt32(db.courses.reduce(0) { $0 + $1.deadlines.count }),
                studyPlans: db.studyPlan == nil ? 0 : 1
            ),
            lastSyncedAt: db.sources.compactMap(\.lastSyncedAt).max(),
            syncInProgress: syncing || db.externalSyncRunning
        )
    }

    // MARK: - PageLampService: shell

    public func status() async throws(PageLampFailure) -> AppStatus {
        await respond("status")
        return makeStatus()
    }

    public func listCourses() async throws(PageLampFailure) -> [CourseSummary] {
        await respond("listCourses")
        return db.courses.map(summary)
    }

    public func listSources() async throws(PageLampFailure) -> [SourceRecord] {
        await respond("listSources")
        return db.sources.map(\.record)
    }

    // MARK: This Week and course detail

    public func listDeadlines(course: String?, daysAhead: UInt32, daysBack: UInt32) async throws(PageLampFailure) -> [Deadline] {
        await respond("listDeadlines")
        let courses: [MockCourse]
        if let course {
            courses = [db.courses[try courseIndex(course)]]
        } else {
            courses = db.courses.filter { !$0.course.hidden }
        }
        return deadlines(in: courses, daysAhead: Int(daysAhead), daysBack: Int(daysBack))
    }

    public func latestStudyPlan() async throws(PageLampFailure) -> StoredStudyPlan? {
        await respond("latestStudyPlan")
        return db.studyPlan
    }

    public func courseOverview(course reference: String) async throws(PageLampFailure) -> CourseOverview {
        await respond("courseOverview")
        let course = db.courses[try courseIndex(reference)]
        let cutoff = now().addingTimeInterval(-14 * 86_400)
        let isRecent = { (material: MaterialView) in (material.publishedAt ?? .distantPast) >= cutoff }
        return CourseOverview(
            course: courseRecord(course),
            aiMaterials: Self.aiMaterials(course.course),
            timeline: course.timeline,
            lifecycle: MockCalendar.lifecycle(course.timeline, keptCurrentUntil: course.keptCurrentUntil),
            currentModules: course.modules.filter { course.timeline.currentModuleIds.contains($0.id) },
            recentMaterials: course.materials.filter(isRecent).sorted {
                ($0.publishedAt ?? .distantPast) > ($1.publishedAt ?? .distantPast)
            },
            upcomingDeadlines: deadlines(in: [course], daysAhead: 21, daysBack: 0),
            recentAnnouncements: course.announcements.filter(isRecent),
            sourceLabel: sourceLabel(course.course.sourceId),
            lastSyncedAt: sourceSyncedAt(course.course.sourceId),
            // Like the backend: what a course-wide download would fetch (all weeks).
            downloadableFiles: UInt32(course.materials.filter {
                $0.kind == .file && $0.textStatus == .notDownloaded && $0.downloadBlocked == nil
            }.count)
        )
    }

    public func weekMaterials(course reference: String, week: UInt32?) async throws(PageLampFailure) -> WeekMaterials {
        await respond("weekMaterials")
        let course = db.courses[try courseIndex(reference)]
        let aiMaterials = Self.aiMaterials(course.course)
        guard let shown = week ?? course.timeline.currentWeek else {
            let cutoff = now().addingTimeInterval(-14 * 86_400)
            return WeekMaterials(
                course: courseRecord(course),
                aiMaterials: aiMaterials,
                week: nil,
                requestedWeek: week,
                timeline: course.timeline,
                modules: [],
                materials: course.materials.filter { ($0.publishedAt ?? .distantPast) >= cutoff },
                availableWeeks: Self.availableWeeks(course),
                note: "Current week unknown — showing materials of the last 14 days.",
                noteKind: .currentWeekUnknown
            )
        }
        let materials = course.materials.filter { $0.weekHint == shown }
        return WeekMaterials(
            course: courseRecord(course),
            aiMaterials: aiMaterials,
            week: shown,
            requestedWeek: week,
            timeline: course.timeline,
            modules: course.modules.filter { $0.weekHint == shown },
            materials: materials,
            availableWeeks: Self.availableWeeks(course),
            note: materials.isEmpty ? "No modules or materials for week \(shown)." : nil,
            noteKind: materials.isEmpty ? .noMaterialsThisWeek : nil
        )
    }

    // MARK: Course weeks and the Past group (every mock course is Current or Unknown)

    public func courseTimeline(course reference: String) async throws(PageLampFailure) -> CourseTimeline {
        await respond("courseTimeline")
        return db.courses[try courseIndex(reference)].timeline
    }

    public func lifecycleSummary() async throws(PageLampFailure) -> LifecycleSummary {
        await respond("lifecycleSummary")
        let entries = db.courses.map { course in
            CourseLifecycleEntry(
                courseId: course.course.id, code: course.course.code, name: course.course.name,
                hidden: course.course.hidden,
                lifecycle: MockCalendar.lifecycle(course.timeline, keptCurrentUntil: course.keptCurrentUntil)
            )
        }
        let suggested = entries.filter(\.lifecycle.suggestRemoval).map(\.courseId)
        let snoozedUntil = db.bannerSnoozedUntil.flatMap { $0 >= isoDay(daysFromToday: 0) ? $0 : nil }
        return LifecycleSummary(
            courses: entries, suggested: suggested,
            showBanner: !suggested.isEmpty && snoozedUntil == nil, bannerSnoozedUntil: snoozedUntil
        )
    }

    public func keepCourseCurrent(course reference: String, until: String?) async throws(PageLampFailure) -> Course {
        await respond("keepCourseCurrent")
        let index = try courseIndex(reference)
        // The mock has no term dates: today + keepCurrentDays().
        db.courses[index].keptCurrentUntil = until ?? isoDay(daysFromToday: Int(keepCurrentDays()))
        return courseRecord(db.courses[index])
    }

    public func clearKeepCourseCurrent(course reference: String) async throws(PageLampFailure) -> Course {
        await respond("clearKeepCourseCurrent")
        let index = try courseIndex(reference)
        db.courses[index].keptCurrentUntil = nil
        return courseRecord(db.courses[index])
    }

    public func snoozeRemovalSuggestions(courses: [String], kind: SnoozeKind) async throws(PageLampFailure) {
        await respond("snoozeRemovalSuggestions")
        let until = kind == .keep ? keepForever() : isoDay(daysFromToday: Int(notNowDays()))
        for reference in courses {
            db.removalSnoozes[db.courses[try courseIndex(reference)].course.id] = until
        }
    }

    public func clearRemovalSnooze(courses: [String]) async throws(PageLampFailure) {
        await respond("clearRemovalSnooze")
        for reference in courses {
            db.removalSnoozes[db.courses[try courseIndex(reference)].course.id] = nil
        }
    }

    public func snoozeLifecycleBanner() async throws(PageLampFailure) {
        await respond("snoozeLifecycleBanner")
        db.bannerSnoozedUntil = isoDay(daysFromToday: Int(notNowDays()))
    }

    public func confirmCourseDates(course reference: String) async throws(PageLampFailure) -> CourseTimeline {
        await respond("confirmCourseDates")
        return db.courses[try courseIndex(reference)].timeline
    }

    // MARK: Updates and launch (like the facade: a fresh install, unless `simulateUpgrade`)

    public func startupTasks(now date: Date) async throws(PageLampFailure) -> StartupTasks {
        await respond("startupTasks")
        let updates = db.updates
        // The Mac app's What's new (the facade's Shell::Mac): never the update-check topic, which
        // is the Tauri app's.
        let whatsNew = updates.upgraded && !updates.whatsNewSeen
            ? WhatsNew(since: updates.upgradedFrom, topics: [.courseWeeks])
            : nil
        let checkIsOld = updates.lastCheck.map { date.timeIntervalSince($0.at) >= 24 * 3600 } ?? true
        return StartupTasks(
            whatsNew: whatsNew,
            updateCheckDue: updates.prefs.autoCheck && updates.disclosureSeen && whatsNew == nil && checkIsOld,
            updatedFrom: updates.upgraded ? updates.upgradedFrom : nil,
            // Monday's note, at the caller's moment (MockService+Note).
            prepareWeeklyNote: noteDue(at: date)
        )
    }

    public func updatePrefs() async throws(PageLampFailure) -> UpdatePrefs {
        await respond("updatePrefs")
        return db.updates.prefs
    }

    public func setUpdatePrefs(prefs: UpdatePrefs) async throws(PageLampFailure) {
        await respond("setUpdatePrefs")
        db.updates.prefs = prefs
    }

    public func effectiveUpdateChannel() async throws(PageLampFailure) -> UpdateChannel {
        await respond("effectiveUpdateChannel")
        // "0.1.0-mock" is a pre-release, like an alpha build: Beta unless the student chose.
        return db.updates.prefs.channel ?? (makeStatus().version.contains("-") ? .beta : .stable)
    }

    public func acknowledgeWhatsNew() async throws(PageLampFailure) {
        await respond("acknowledgeWhatsNew")
        db.updates.whatsNewSeen = true
        // The Mac app's acknowledgement is never the update disclosure (only the Tauri app's
        // update-check topic counts as that).
    }

    public func acknowledgeUpdateDisclosure() async throws(PageLampFailure) {
        await respond("acknowledgeUpdateDisclosure")
        db.updates.disclosureSeen = true
    }

    public func recordUpdateCheck(record: UpdateCheckRecord) async throws(PageLampFailure) {
        await respond("recordUpdateCheck")
        db.updates.lastCheck = record
    }

    public func lastUpdateCheck() async throws(PageLampFailure) -> UpdateCheckRecord? {
        await respond("lastUpdateCheck")
        return db.updates.lastCheck
    }

    public func activity() async throws(PageLampFailure) -> Activity {
        await respond("activity")
        let items = syncStartedAt.map { [ActivityItem(kind: .sync, sourceId: nil, startedAt: $0)] } ?? []
        return Activity(items: items, otherProcessSyncing: db.externalSyncRunning && items.isEmpty)
    }

    // MARK: Sources & Sync

    public func syncAll(request: SyncRequest, observer: any SyncObserver) async throws(PageLampFailure) -> SyncSummary {
        let startedAt = now()
        let results = try await runSync(db.sources.map(\.id), observer: observer)
        return SyncSummary(
            startedAt: startedAt,
            finishedAt: now(),
            ok: results.allSatisfy(\.ok),
            results: results
        )
    }

    public func syncSource(sourceId: String, request: SyncRequest, observer: any SyncObserver) async throws(PageLampFailure) -> SourceSyncResult {
        _ = try sourceIndex(sourceId)
        let results = try await runSync([sourceId], observer: observer)
        guard let result = results.first else {
            throw PageLampFailure(kind: .internal, message: "Sync produced no result")
        }
        return result
    }

    private func runSync(_ sourceIds: [String], observer: any SyncObserver) async throws(PageLampFailure) -> [SourceSyncResult] {
        calls["sync", default: 0] += 1
        if syncing || db.externalSyncRunning {
            await pause(timing.latency)
            throw PageLampFailure(kind: .busy, message: "Another PageLamp process is already syncing.")
        }
        syncing = true
        syncStartedAt = now()
        defer {
            syncing = false
            syncStartedAt = nil
        }
        var results: [SourceSyncResult] = []
        for sourceId in sourceIds {
            let index = try sourceIndex(sourceId)
            let source = db.sources[index]
            let startedAt = now()
            observer.onEvent(event: .sourceStarted(sourceId: source.id, label: source.label))
            seedCoursesOnFirstSync(source)
            let courses = db.courses.filter { $0.course.sourceId == source.id }
            let total = UInt32(max(courses.count, 1) * 3)
            var warnings: [String] = []
            for step in 1 ... total {
                if let gate = timing.gate {
                    await gate.pass(SyncStepPosition(sourceId: source.id, step: step))
                } else {
                    await pause(timing.syncStep)
                }
                observer.onEvent(event: .progress(
                    sourceId: source.id,
                    message: step < total ? "Indexing materials (\(step)/\(total))" : "Updating timelines",
                    current: step,
                    total: total,
                    stage: nil,
                    course: nil
                ))
                if source.kind == .folder, step == 2 {
                    let warning = "Skipped 'Week 3 lecture recording.mp4' — video files can't be read."
                    warnings.append(warning)
                    observer.onEvent(event: .warning(sourceId: source.id, message: warning))
                }
            }

            // Failures stay until fixed (token replaced, folder re-added).
            let stuck = source.lastErrorKind == .authExpiredOrRevoked || source.lastErrorKind == .notFound
            if stuck {
                observer.onEvent(event: .sourceFinished(
                    sourceId: source.id, ok: false, error: source.lastError ?? "Sync failed",
                    errorKind: source.lastErrorKind
                ))
            } else {
                db.sources[index].lastSyncedAt = now()
                db.sources[index].lastError = nil
                db.sources[index].lastErrorKind = nil
                observer.onEvent(event: .sourceFinished(sourceId: source.id, ok: true, error: nil, errorKind: nil))
            }
            results.append(SourceSyncResult(
                sourceId: source.id,
                label: source.label,
                kind: source.kind,
                ok: !stuck,
                error: stuck ? (source.lastError ?? "Sync failed") : nil,
                errorKind: stuck ? source.lastErrorKind : nil,
                startedAt: startedAt,
                finishedAt: now(),
                courses: UInt32(courses.count),
                modules: UInt32(courses.reduce(0) { $0 + $1.modules.count }),
                materials: UInt32(courses.reduce(0) { $0 + $1.materials.count }),
                filesDownloaded: stuck ? 0 : 2,
                filesIndexed: stuck ? 0 : 2,
                events: UInt32(courses.reduce(0) { $0 + $1.deadlines.count }),
                warnings: warnings,
                // Per-course details come from Canvas only.
                courseSummaries: source.kind == .canvas && !stuck
                    ? courses.map { course in
                        CourseSyncSummary(
                            course: course.course.code ?? course.course.name,
                            modules: UInt32(course.modules.count),
                            pages: UInt32(course.materials.filter { $0.kind == .page }.count),
                            files: UInt32(course.materials.filter { $0.kind == .file }.count),
                            events: UInt32(course.deadlines.count),
                            warnings: 0
                        )
                    }
                    : [],
                requests: nil
            ))
        }
        return results
    }

    /// First-run demo: in the "empty" scenario the first folder or Canvas source to sync "finds"
    /// the demo courses (M2's Add Source makes that reachable).
    private func seedCoursesOnFirstSync(_ source: MockSource) {
        guard scenario == .empty, source.kind != .ical, db.courses.isEmpty else { return }
        db.courses = MockFixtures.database(.demo, now: now(), calendar: calendar).courses
    }

    // MARK: Connect

    public func mcpClientConfigs(pagelampBinary: String) async throws(PageLampFailure) -> [McpClientConfig] {
        await respond("mcpClientConfigs")
        return MockFixtures.mcpClientConfigs(binary: pagelampBinary)
    }

    public func mcpLaunch(pagelampBinary: String) async throws(PageLampFailure) -> McpLaunch {
        await respond("mcpLaunch")
        return McpLaunch(command: pagelampBinary, args: ["mcp"], env: [:], temporaryLocation: nil)
    }

    public func doctor() async throws(PageLampFailure) -> DoctorReport {
        await respond("doctor")
        let status = makeStatus()
        return DoctorReport(
            version: status.version,
            os: "macos",
            arch: "aarch64",
            dataDir: db.dataDir,
            logsDir: "\(db.dataDir)/logs",
            schemaVersion: 7,
            databaseError: nil,
            keychainAvailable: true,
            keychainError: nil,
            sources: db.sources.map {
                DoctorSource(kind: $0.kind, ok: $0.lastErrorKind == nil, lastSyncedAt: $0.lastSyncedAt, lastErrorKind: $0.lastErrorKind)
            },
            courses: status.counts.courses,
            hiddenCourses: status.counts.hiddenCourses,
            materials: status.counts.materials,
            events: status.counts.events,
            // Claude Code already has a PageLamp entry, so Connect can preselect it.
            mcpClients: McpClientPresence(claudeDesktop: false, claudeCode: true, codex: false),
            lastCrash: db.lastCrash,
            extractWorker: ExtractWorkerCheck(status: .ok, spawnMs: 25),
            unreadableFiles: []
        )
    }

    // MARK: Diagnostics

    public func diagnosticReport() async throws(PageLampFailure) -> String {
        await respond("diagnosticReport")
        let status = makeStatus()
        let iso = ISO8601DateFormatter()
        let sources = status.sources.map { source in
            let synced = source.lastSyncedAt.map { iso.string(from: $0) } ?? "never"
            let error = source.lastErrorKind.map { "\($0)" } ?? "—"
            return "| \(source.kind) | \(source.lastErrorKind == nil ? "yes" : "no") | \(synced) | \(error) |"
        }
        let crash = db.lastCrash.map { crash in
            "\(iso.string(from: crash.time)) · \(crash.process) · \(crash.message)\(crash.location.map { " (\($0))" } ?? "")"
        } ?? "none"
        let stamp = iso.string(from: now())
        return ([
            "# PageLamp diagnostic report",
            "",
            "- Version: \(status.version)",
            "- OS: macOS 26.0 (aarch64)",
            "- Data folder: ~/Library/Application Support/dev.PageLamp.PageLamp",
            "- Database: ok",
            "- Keychain: available",
            "",
            "## Sources",
            "",
            "| kind | ok | last synced | last error |",
            "| --- | --- | --- | --- |",
        ] + (sources.isEmpty ? ["| — | — | — | — |"] : sources) + [
            "",
            "## Library",
            "",
            "\(status.counts.courses) courses (\(status.counts.hiddenCourses) hidden), \(status.counts.materials) materials, \(status.counts.events) events",
            "",
            "## Last crash",
            "",
            crash,
            "",
            "## Recent log (redacted)",
            "",
            "```",
            "\(stamp) INFO  pagelamp::sync: sync finished (course-1, course-2, course-3)",
            "\(stamp) DEBUG pagelamp::canvas: GET /api/v1/courses → 200 (token [redacted])",
            "```",
            "",
        ]).joined(separator: "\n")
    }

    public func logsDir() async throws(PageLampFailure) -> String {
        await respond("logsDir")
        // A real, empty folder so "Open Logs Folder" works; never the real data folder.
        let dir = FileManager.default.temporaryDirectory.appending(path: "PageLampPreview-mock-logs", directoryHint: .isDirectory)
        do {
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        } catch {
            throw PageLampFailure(kind: .internal, message: "Couldn't create \(dir.path): \(error)")
        }
        return dir.path(percentEncoded: false)
    }

    public func lastCrash() async throws(PageLampFailure) -> CrashReport? {
        await respond("lastCrash")
        return db.lastCrash
    }

    public func clearLastCrash() async throws(PageLampFailure) {
        await respond("clearLastCrash")
        db.lastCrash = nil
    }
}
