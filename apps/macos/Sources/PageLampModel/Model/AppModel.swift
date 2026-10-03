// The app's state on the main actor (spec §2.2–§2.4, §6.2): shell data, navigation, sync and the
// status capsule, the data mode, and the language.

import AppKit
import Foundation
import Observation
import PageLampKit

/// Where the app's data comes from.
public enum DataMode: Equatable, Sendable {
    /// Synthetic demo data (`MockService`): the preview build's default.
    case mock(MockScenario)
    /// The real facade over the default data folder, shared with the installed PageLamp app and
    /// its CLI. Reached only from the Debug menu, after a confirmation.
    case live
}

/// The diagnostic report sheet (Help ▸ Copy Diagnostic Report…, S2): always a preview first.
public enum DiagnosticReportState: Equatable, Sendable {
    case loading
    case loaded(String)
    case failed(PageLampFailure)
}

/// The window that presents the diagnostic report sheet: the main window (Help menu, S2) or
/// Settings (Settings ▸ Help), so the sheet opens where the student asked for it.
public enum DiagnosticReportHost: Equatable, Sendable {
    case main
    case settings
}

/// The sidebar footer's persistent, low-emphasis state (spec §2.3; never counts).
public enum FooterStatus: Equatable, Sendable {
    case syncing
    case needsAttention
    case synced(Date)
    case neverSynced
}

/// A source row on Sources & Sync to scroll to and briefly highlight (a fix or "Open Sources &
/// Sync" led there). Each request is new, so asking again for the same source highlights again.
public struct SourceHighlight: Equatable, Sendable {
    public let sourceId: String
    public let request: Int
}

/// A menu command that shows its result in the main window, which may be closed (only Settings
/// open): the command opens the main window first (spec §2.7).
public enum MainWindowCommand: Equatable, Sendable {
    /// View ▸ This Week ⌘1 · Sources & Sync ⌘2 · Connect AI App ⌘3.
    case show(Destination)
    /// Go ▸ Previous Week ⌘[ (-1) · Next Week ⌘] (+1).
    case stepWeek(Int)
    /// Go ▸ Current Week ⇧⌘T.
    case currentWeek
    /// Help ▸ Copy Diagnostic Report… (the preview sheet is the main window's).
    case diagnosticReport
    /// Debug ▸ Data Source ▸ Live Data… (the confirmation is the main window's).
    case liveData
}

/// The notification centers the reminders use: the system's for live data (in the app bundle),
/// one in memory for mock data, so synthetic reminders never reach Notification Center.
public struct ReminderCenters: Sendable {
    public var live: @Sendable () -> any NotificationCenterClient
    public var mock: any NotificationCenterClient

    public init(live: @escaping @Sendable () -> any NotificationCenterClient, mock: any NotificationCenterClient) {
        self.live = live
        self.mock = mock
    }

    public static var standard: ReminderCenters {
        ReminderCenters(
            live: {
                if let system = SystemNotificationCenter.shared() { return system }
                return RecordingNotificationCenter()
            },
            mock: RecordingNotificationCenter()
        )
    }
}

@Observable @MainActor
public final class AppModel {
    /// How long transient states stay, how fast the mock answers, and how the model waits.
    public struct Timing: Sendable {
        /// Waits for a duration; returns early when the waiting task is cancelled. Every timer of
        /// the model goes through it, so tests can fire timers themselves instead of sleeping.
        public typealias Sleep = @Sendable (Duration) async -> Void

        public var finishedCapsule: Duration
        public var failedCapsule: Duration
        public var mock: MockService.Timing
        /// How often to look again while another process syncs (spec §2.2, S17).
        public var busyPoll: Duration
        /// How long a source row stays highlighted after a fix led to it.
        public var sourceHighlight: Duration
        public var sleep: Sleep

        public init(
            finishedCapsule: Duration,
            failedCapsule: Duration,
            mock: MockService.Timing,
            busyPoll: Duration = .seconds(5),
            sourceHighlight: Duration = .seconds(2),
            sleep: @escaping Sleep = { try? await Task.sleep(for: $0) }
        ) {
            self.finishedCapsule = finishedCapsule
            self.failedCapsule = failedCapsule
            self.mock = mock
            self.busyPoll = busyPoll
            self.sourceHighlight = sourceHighlight
            self.sleep = sleep
        }

        /// The app: "Sync finished" for 4 s (spec §6.2).
        public static let standard = Timing(finishedCapsule: .seconds(4), failedCapsule: .seconds(6), mock: .interactive)
    }

    /// Whether the shell has data (S1 loading, S2 backend unavailable).
    public enum Phase: Equatable, Sendable {
        case loading
        case ready
        case unavailable(PageLampFailure)
    }

    /// Shell data that failed to load while `status()` worked (sections show S14 errors).
    public enum ShellPart: Hashable, Sendable {
        case courses
        case deadlines
        case studyPlan
    }

    // MARK: Environment

    public let strings: StringTable
    /// "Now" for everything the UI derives from time (injectable for tests and snapshots).
    @ObservationIgnored public let clock: @Sendable () -> Date
    @ObservationIgnored public let calendar: Calendar
    @ObservationIgnored private let settings: any SettingsStore
    @ObservationIgnored private let timing: Timing
    @ObservationIgnored private let notificationCenter: NotificationCenter
    @ObservationIgnored private let preferredLanguages: @Sendable () -> [String]
    /// The preferred languages at launch (menus keep them until the app reopens).
    @ObservationIgnored private let launchLanguages: [String]

    // MARK: Data source

    public private(set) var dataMode: DataMode
    /// The core. Screens load their own data through it (errors are `PageLampFailure`).
    public private(set) var service: any PageLampService {
        didSet {
            serviceGeneration += 1
            weeklyNote?.replace(service: service)
        }
    }
    /// Bumped whenever `service` is replaced (Debug ▸ Data Source, the live facade opening):
    /// screens holding their own model of the service rebuild on it.
    public private(set) var serviceGeneration = 0

    // MARK: Shell data (spec §2.8)

    public private(set) var phase: Phase = .loading
    public private(set) var status: AppStatus?
    /// Every course, hidden ones included; the sidebar shows `visibleCourses`.
    public private(set) var courses: [CourseSummary] = []
    public private(set) var sources: [SourceRecord] = []
    /// The This Week inputs: `list_deadlines(nil, 8, 0)` and the latest plan.
    public private(set) var upcomingDeadlines: [Deadline] = []
    public private(set) var studyPlan: StoredStudyPlan?
    public private(set) var lastCrash: CrashReport?
    /// Set while the app runs from a temporary location (Connect warns, spec S15).
    public private(set) var temporaryLocation: TemporaryLocation?
    public private(set) var sectionErrors: [ShellPart: PageLampFailure] = [:]
    /// How many refreshes started (each refresh's sequence number; diagnostics and tests).
    public private(set) var refreshCount = 0
    /// The sequence number of the last refresh whose results were applied (older refreshes that
    /// finish after a newer one started are dropped; tests).
    public private(set) var appliedRefresh = 0

    // MARK: Navigation (spec §2.4)

    public var destination: Destination = .thisWeek
    /// The destination whose page (and toolbar) the detail column shows. DetailColumn moves it to
    /// `destination` at once, or two display frames later while the sidebar capsule leads (a
    /// Debug comparison, spec §2.3 "Motion order").
    public private(set) var pageDestination: Destination = .thisWeek
    public var inspectorShown = false
    /// Whether a main window restored its stored destination and inspector in this run. Only the
    /// first one does: a window reopened later (say by ⌘2 with only Settings open) shows where
    /// the model is now, which the command just set, not what an older window stored.
    @ObservationIgnored public var restoredWindowState = false
    @ObservationIgnored private var courseStates: [String: CourseUIState] = [:]

    /// The detail column caught up with `destination`.
    public func showDestinationPage() {
        if pageDestination != destination { pageDestination = destination }
    }

    // MARK: Sync and the capsule (spec §6.2)

    public private(set) var isSyncing = false
    public private(set) var syncProgress: SyncProgress?
    /// The last run's results, shown on Sources & Sync until Hide Results.
    public private(set) var lastRun: SyncRun?
    public private(set) var capsule: CapsuleState = .hidden
    @ObservationIgnored private var dismissedAttention: Set<String> = []
    @ObservationIgnored private var capsuleTimer: Task<Void, Never>?
    /// The source row Sources & Sync scrolls to and highlights (spec §3.3; the fix bubble).
    public private(set) var sourceHighlight: SourceHighlight?
    @ObservationIgnored private var highlightTimer: Task<Void, Never>?
    @ObservationIgnored private var highlightRequests = 0
    /// Why the crash notice couldn't be dismissed (S6), shown under it.
    public private(set) var crashDismissFailure: PageLampFailure?

    // MARK: Presentation requests (commands → views)

    public var diagnosticReport: DiagnosticReportState?
    /// Where the diagnostic report sheet is presented.
    public private(set) var diagnosticReportHost: DiagnosticReportHost = .main
    /// The Debug menu asked to switch to live data; the root view asks for confirmation.
    public var confirmingLiveData = false
    /// What's new after an update, until the student closes it (the main window's sheet).
    public private(set) var whatsNew: WhatsNewPresentation?

    // MARK: Settings

    public var language: AppLanguage {
        didSet {
            settings.language = language
            L10n.current = l10n
        }
    }

    public var appearance: AppAppearance {
        didSet { settings.appearance = appearance }
    }

    // MARK: AI (M3; preview builds until it ships)

    /// Settings ▸ AI: the student's models, keys, budget and usage (shared with the Tauri app).
    public let aiSettings: Bool
    /// This Week ▸ Plan with PageLamp…: a study plan written by the student's model.
    public let aiPlan: Bool
    /// A course's Explain section: a week explained by the student's model.
    public let aiExplain: Bool
    /// This Week's weekly note and Monday's (Settings ▸ AI); nil where it's off.
    public private(set) var weeklyNote: WeeklyNoteModel?

    /// A plan written by PageLamp was saved: This Week shows it at once, then everything is read
    /// again (a refresh already under way can't put the old plan back; reminders and the menu
    /// bar's week follow the new plan).
    public func studyPlanSaved(_ stored: StoredStudyPlan) async {
        studyPlan = stored
        sectionErrors[.studyPlan] = nil
        await refresh()
    }

    // MARK: Reminders (M3; preview builds until they ship)

    /// Reminders as notifications and the catch-up card; nil where reminders aren't shown yet.
    public private(set) var reminderDelivery: ReminderDelivery?
    /// The menu bar extra's week (`weekly_digest()`), read with each refresh where reminders are
    /// shown; nil until read.
    public private(set) var menuBarWeek: MenuBarWeek?
    /// Why the last `weekly_digest()` failed (the menu says so and offers Try Again).
    public private(set) var menuBarWeekFailure: PageLampFailure?
    /// Reads of the menu bar's week started: a late, older one is dropped.
    @ObservationIgnored private var menuBarLoads = 0
    @ObservationIgnored private let reminderCenters: ReminderCenters
    @ObservationIgnored private var reminderTasks: [Task<Void, Never>] = []
    @ObservationIgnored private var reminderResponder: ReminderResponder?
    /// Opens the main window, which may be closed. A view sets it: `openWindow` lives in
    /// SwiftUI's environment (a click on a notification uses it).
    @ObservationIgnored public var openMainWindow: (() -> Void)?

    @ObservationIgnored private var activationTask: Task<Void, Never>?
    @ObservationIgnored private var busyPoll: Task<Void, Never>?
    @ObservationIgnored private var generation = 0

    /// - Parameters:
    ///   - dataMode: `.mock(…)` (default). `.live` is reached with `useLive()`, never at init.
    ///   - service: a ready-made service (tests); by default the mock for `dataMode`.
    public init(
        dataMode: DataMode = .mock(.preview),
        strings: StringTable,
        settings: any SettingsStore = UserDefaultsSettingsStore(),
        timing: Timing = .standard,
        calendar: Calendar = .current,
        clock: @escaping @Sendable () -> Date = { Date() },
        notificationCenter: NotificationCenter = .default,
        preferredLanguages: @escaping @Sendable () -> [String] = { Locale.preferredLanguages },
        service: (any PageLampService)? = nil,
        reminders: Bool = false,
        reminderCenters: ReminderCenters = .standard,
        aiSettings: Bool = false,
        aiPlan: Bool = false,
        aiExplain: Bool = false,
        aiNote: Bool = false
    ) {
        self.strings = strings
        self.aiSettings = aiSettings
        self.aiPlan = aiPlan
        self.aiExplain = aiExplain
        self.reminderCenters = reminderCenters
        self.settings = settings
        self.timing = timing
        self.calendar = calendar
        self.clock = clock
        self.notificationCenter = notificationCenter
        self.preferredLanguages = preferredLanguages
        launchLanguages = preferredLanguages()
        language = settings.language
        appearance = settings.appearance
        let mode: DataMode = service == nil && dataMode == .live ? .mock(.preview) : dataMode
        self.dataMode = mode
        if let service {
            self.service = service
        } else if case .mock(let scenario) = mode {
            self.service = MockService(scenario: scenario, timing: timing.mock, calendar: calendar, now: clock)
        } else {
            self.service = MockService(scenario: .preview, timing: timing.mock, calendar: calendar, now: clock)
        }
        L10n.current = l10n
        if aiNote {
            weeklyNote = WeeklyNoteModel(
                service: self.service, clock: clock, calendar: calendar,
                timing: WeeklyNoteModel.Timing(sleep: timing.sleep), notificationCenter: notificationCenter,
                uiLanguage: { [weak self] in self?.localization ?? "en" }
            )
        }
        if reminders {
            reminderDelivery = ReminderDelivery(
                service: self.service, center: reminderCenters.mock, record: InMemorySettingsStore(),
                clock: clock, l10n: { [weak self, strings] in self?.l10n ?? L10n(locale: .current, table: strings) }
            )
            // The delegate is set before launch finishes, so a click that launched the app arrives.
            if SystemNotificationCenter.isAvailable {
                let responder = ReminderResponder(model: self)
                reminderResponder = responder
                SystemNotificationCenter.install(delegate: responder)
            }
        }
    }

    /// The student clicked a reminder's notification: its course (a deadline's), else This Week.
    public func openFromReminder(courseId: String?) {
        if let courseId, course(id: courseId) != nil {
            destination = .course(courseId)
        } else {
            destination = .thisWeek
        }
        NSApp.activate()
        openMainWindow?()
    }

    isolated deinit {
        activationTask?.cancel()
        reminderTasks.forEach { $0.cancel() }
        capsuleTimer?.cancel()
        busyPoll?.cancel()
        highlightTimer?.cancel()
    }

    // MARK: - Language

    /// The shipped localization in use ("en" / "zh-Hans").
    public var localization: String {
        language.localization(preferredLanguages: preferredLanguages())
    }

    /// The locale views format with (`.environment(\.locale, model.locale)`).
    public var locale: Locale {
        language.locale(preferredLanguages: preferredLanguages(), region: Locale.current.region)
    }

    /// The strings of the current language.
    public var l10n: L10n {
        L10n(locale: locale, table: strings)
    }

    /// Strings for menus and window titles. Like the system's own menu items they follow
    /// `AppleLanguages` as it was at launch, and switch only after Reopen Now (spec §7.3), so the
    /// menu bar never mixes two languages.
    public var menuL10n: L10n {
        L10n(
            locale: AppLanguage.system.locale(preferredLanguages: launchLanguages, region: Locale.current.region),
            table: strings
        )
    }

    /// Makes menus and system dialogs follow the language after a relaunch (Reopen Now).
    public func applyLanguageToMenus() {
        settings.appleLanguages = language.appleLanguages
    }

    // MARK: - Lifecycle

    /// Loads the shell data and refreshes it whenever the app becomes active (spec §2.2).
    /// Idempotent; the root view calls it once.
    public func start() async {
        let firstStart = activationTask == nil
        if activationTask == nil {
            let center = notificationCenter
            activationTask = Task { [weak self] in
                for await _ in center.notifications(named: NSApplication.didBecomeActiveNotification) {
                    await self?.refresh()
                }
            }
        }
        if firstStart { weeklyNote?.start() }
        if firstStart, reminderDelivery != nil {
            // A new zone or a wake: the week ahead is scheduled again.
            let center = notificationCenter
            let workspace = NSWorkspace.shared.notificationCenter
            reminderTasks = [
                Task { [weak self] in
                    for await _ in center.notifications(named: .NSSystemTimeZoneDidChange) {
                        self?.reminderDelivery?.requestPass()
                    }
                },
                Task { [weak self] in
                    for await _ in workspace.notifications(named: NSWorkspace.didWakeNotification) {
                        self?.reminderDelivery?.requestPass()
                    }
                },
            ]
        }
        await refresh()
        if firstStart { await loadWhatsNew() }
    }

    /// Reloads status, courses, sources and the This Week inputs.
    ///
    /// Refreshes overlap (activation, the busy poll, the one after each sync, Try Again), and
    /// the facade answers in any order: only the latest refresh to start applies its results. An
    /// older one that finishes later (say an activation refresh that read the data before our
    /// sync finished) is dropped instead of putting stale data back.
    public func refresh() async {
        let generation = self.generation
        let service = self.service
        let sidecar = sidecarPath
        refreshCount += 1
        let sequence = refreshCount

        let status: AppStatus
        do throws(PageLampFailure) {
            status = try await service.status()
        } catch {
            guard isLatestRefresh(sequence, generation) else { return }
            busyPoll?.cancel()
            busyPoll = nil
            phase = .unavailable(error)
            appliedRefresh = sequence
            return
        }

        async let courses = Self.load { () async throws(PageLampFailure) in try await service.listCourses() }
        async let deadlines = Self.load { () async throws(PageLampFailure) in
            try await service.listDeadlines(course: nil, daysAhead: ThisWeekDigest.fetchDaysAhead, daysBack: 0)
        }
        async let plan = Self.load { () async throws(PageLampFailure) in try await service.latestStudyPlan() }
        async let crash = Self.load { () async throws(PageLampFailure) in try await service.lastCrash() }
        async let launch = Self.load { () async throws(PageLampFailure) in try await service.mcpLaunch(pagelampBinary: sidecar) }
        let loaded = await (courses, deadlines, plan, crash, launch)
        guard isLatestRefresh(sequence, generation) else { return }

        self.status = status
        sources = status.sources
        var errors: [ShellPart: PageLampFailure] = [:]
        switch loaded.0 {
        case .success(let value): self.courses = value
        case .failure(let error): errors[.courses] = error
        }
        switch loaded.1 {
        case .success(let value): upcomingDeadlines = value
        case .failure(let error): errors[.deadlines] = error
        }
        switch loaded.2 {
        case .success(let value): studyPlan = value
        case .failure(let error): errors[.studyPlan] = error
        }
        if case .success(let value) = loaded.3 { lastCrash = value }
        if case .success(let value) = loaded.4 { temporaryLocation = value.temporaryLocation }
        sectionErrors = errors
        phase = .ready
        updateAttention()
        scheduleBusyPoll()
        appliedRefresh = sequence
        // After a sync, a plan saved elsewhere, a new day: the week ahead again.
        reminderDelivery?.requestPass()
        if reminderDelivery != nil { await loadMenuBarWeek() }
    }

    /// Reads the digest for the menu bar extra.
    public func loadMenuBarWeek() async {
        let generation = self.generation
        let service = self.service
        let now = clock()
        menuBarLoads += 1
        let load = menuBarLoads
        // The plan with the digest: its tasks carry its AI-generated line (a plan PageLamp wrote).
        // The week and the label are one pair, read together: both are new, or neither.
        async let plan = Self.load { () async throws(PageLampFailure) in try await service.latestStudyPlan() }
        do throws(PageLampFailure) {
            let digest = try await service.weeklyDigest()
            let stored = try await plan.get()
            guard generation == self.generation, load == menuBarLoads else { return }
            menuBarWeek = MenuBarWeek(digest: digest, now: now, aiLabel: stored?.aiLabel)
            menuBarWeekFailure = nil
        } catch {
            guard generation == self.generation, load == menuBarLoads else { return }
            // Either read failed: the week shown stays with its own label (they were read
            // together); the failure is kept for the next look.
            menuBarWeekFailure = error
        }
    }

    /// Whether the refresh numbered `sequence` (of the service generation `generation`) is still
    /// the latest to have started.
    private func isLatestRefresh(_ sequence: Int, _ generation: Int) -> Bool {
        sequence == refreshCount && generation == self.generation
    }

    /// While another process (the CLI) holds the sync lock, look again every few seconds, so
    /// the sync buttons come back when it finishes (spec §2.2, S17).
    private func scheduleBusyPoll() {
        busyPoll?.cancel()
        busyPoll = nil
        guard externalSyncRunning else { return }
        let delay = timing.busyPoll
        let sleep = timing.sleep
        busyPoll = Task { [weak self] in
            await sleep(delay)
            guard !Task.isCancelled else { return }
            await self?.refresh()
        }
    }

    /// Whether a look-again is scheduled (S17; tests).
    public var isBusyPollScheduled: Bool {
        busyPoll != nil
    }

    private nonisolated static func load<T: Sendable>(
        _ body: @Sendable () async throws(PageLampFailure) -> T
    ) async -> Result<T, PageLampFailure> {
        do throws(PageLampFailure) {
            return .success(try await body())
        } catch {
            return .failure(error)
        }
    }

    // MARK: - Derived shell state

    /// Sidebar courses: visible and active, by code (hidden and past courses are M2 sections).
    public var visibleCourses: [CourseSummary] {
        courses
            .filter { !$0.course.hidden && $0.course.enrollmentActive }
            .sorted { lhs, rhs in
                (lhs.course.code ?? lhs.course.name).localizedStandardCompare(rhs.course.code ?? rhs.course.name)
                    == .orderedAscending
            }
    }

    public func course(id: String) -> CourseSummary? {
        courses.first { $0.course.id == id }
    }

    /// Sources whose last sync failed, in list order.
    public var failingSources: [SourceRecord] {
        sources.filter { $0.lastErrorKind != nil }
    }

    public func isSourceFailing(_ sourceId: String) -> Bool {
        sources.contains { $0.id == sourceId && $0.lastErrorKind != nil }
    }

    public var footerStatus: FooterStatus {
        if isSyncing || status?.syncInProgress == true { return .syncing }
        if !failingSources.isEmpty { return .needsAttention }
        if let last = status?.lastSyncedAt { return .synced(last) }
        return .neverSynced
    }

    /// This Week's grouping of `upcomingDeadlines` and the plan, as of `clock()`.
    public var thisWeek: ThisWeekDigest {
        ThisWeekDigest(deadlines: upcomingDeadlines, plan: studyPlan, now: clock(), calendar: calendar)
    }

    /// The bundled CLI that AI apps launch (`Contents/MacOS/pagelamp`); the mock's made-up path in
    /// mock mode.
    public var sidecarPath: String {
        switch dataMode {
        case .mock: MockService.binaryPath
        case .live: Bundle.main.url(forAuxiliaryExecutable: "pagelamp")?.path(percentEncoded: false) ?? "pagelamp"
        }
    }

    /// Another process (usually the CLI) holds the sync lock while this app isn't syncing (S17).
    public var externalSyncRunning: Bool {
        !isSyncing && status?.syncInProgress == true
    }

    /// Whether a sync can start now: the data is loaded and no sync runs, here or elsewhere
    /// (S17: every sync button is disabled while the CLI syncs).
    public var canStartSync: Bool {
        phase == .ready && !isSyncing && !externalSyncRunning
    }

    /// Sync Now / Sync All: a sync can start and there is something to sync.
    public var canSync: Bool {
        canStartSync && !sources.isEmpty
    }

    // MARK: - Courses and weeks

    /// The session state (section, week) of a course; created on first use.
    public func ui(for courseId: String) -> CourseUIState {
        if let state = courseStates[courseId] { return state }
        let state = CourseUIState()
        courseStates[courseId] = state
        return state
    }

    /// The course on screen, if the detail shows one.
    public var selectedCourseId: String? {
        if case .course(let id) = destination { return id }
        return nil
    }

    /// Loads the displayed week of a course and remembers its available and current weeks
    /// (for Go ▸ Previous/Next Week and the toolbar).
    public func weekMaterials(for courseId: String) async throws(PageLampFailure) -> WeekMaterials {
        let state = ui(for: courseId)
        let week = try await service.weekMaterials(course: courseId, week: state.selectedWeek)
        state.update(availableWeeks: week.availableWeeks, currentWeek: week.timeline.currentWeek)
        return week
    }

    public func canStepWeek(by delta: Int) -> Bool {
        guard let id = selectedCourseId else { return false }
        let state = ui(for: id)
        return state.section == .week && (delta < 0 ? state.previousWeek : state.nextWeek) != nil
    }

    public func stepWeek(by delta: Int) {
        guard canStepWeek(by: delta), let id = selectedCourseId else { return }
        ui(for: id).step(by: delta)
    }

    /// Go ▸ Current Week: back to the current week (or to "Recent materials" when the current
    /// week is unknown) from a week the student stepped to or from another section.
    public var canShowCurrentWeek: Bool {
        guard let id = selectedCourseId else { return false }
        let state = ui(for: id)
        return state.isAwayFromDefault || (state.section != .week && state.currentWeek != nil)
    }

    public func showCurrentWeek() {
        guard let id = selectedCourseId else { return }
        ui(for: id).showCurrentWeek()
    }

    // MARK: - Sync

    public func syncAll() async {
        await runSync(sourceId: nil)
    }

    public func syncSource(_ sourceId: String) async {
        await runSync(sourceId: sourceId)
    }

    public func hideResults() {
        lastRun = nil
    }

    private func runSync(sourceId: String?) async {
        guard phase == .ready, !isSyncing else { return }
        isSyncing = true
        capsuleTimer?.cancel()
        let observer = SyncEventStream()
        let progress = SyncProgress(sourceCount: sourceId == nil ? max(sources.count, 1) : 1)
        syncProgress = progress
        capsule = .syncing(progress.capsule)

        // Events arrive on the core's threads; the stream hands them to the main actor in order.
        let pump = Task { [weak self] in
            for await event in observer.events {
                guard let self, var progress = self.syncProgress else { return }
                progress.apply(event)
                self.syncProgress = progress
                if case .syncing = self.capsule { self.capsule = .syncing(progress.capsule) }
            }
        }

        let service = self.service
        let outcome: Result<[SourceSyncResult], PageLampFailure>
        do throws(PageLampFailure) {
            if let sourceId {
                outcome = .success([try await service.syncSource(sourceId: sourceId, request: SyncRequest(), observer: observer)])
            } else {
                outcome = .success(try await service.syncAll(request: SyncRequest(), observer: observer).results)
            }
        } catch {
            outcome = .failure(error)
        }
        observer.finish()
        await pump.value

        isSyncing = false
        syncProgress = nil
        switch outcome {
        case .success(let results):
            lastRun = SyncRun(finishedAt: clock(), results: results)
            finishSync(results)
        case .failure(let error):
            capsule = .failed(error.kind)
            hideCapsule(after: timing.failedCapsule)
        }
        await refresh()
    }

    private func finishSync(_ results: [SourceSyncResult]) {
        if let rejected = results.first(where: { $0.errorKind == .authExpiredOrRevoked }) {
            // A new rejection is a new problem, even if the student dismissed the last one.
            dismissedAttention.remove(rejected.sourceId)
            capsule = .attention(.init(
                sourceId: rejected.sourceId,
                sourceLabel: rejected.label,
                fix: rejected.kind == .ical ? .replaceFeed : .replaceToken
            ))
        } else {
            capsule = .finished(.init(problems: results.filter { !$0.ok }.count))
            hideCapsule(after: timing.finishedCapsule)
        }
    }

    private func hideCapsule(after delay: Duration) {
        capsuleTimer?.cancel()
        let shown = capsule
        let sleep = timing.sleep
        capsuleTimer = Task { [weak self] in
            await sleep(delay)
            guard !Task.isCancelled, let self, self.capsule == shown else { return }
            self.capsule = .hidden
            self.updateAttention()
        }
    }

    /// Shows the first rejected secret the student hasn't dismissed (S7), and clears an
    /// attention whose source was fixed.
    private func updateAttention() {
        if case .attention(let attention) = capsule,
           !sources.contains(where: { $0.id == attention.sourceId && $0.lastErrorKind == .authExpiredOrRevoked }) {
            capsule = .hidden
        }
        guard capsule == .hidden, !isSyncing else { return }
        if let attention = sources.lazy
            .compactMap(CapsuleState.Attention.forSource)
            .first(where: { !dismissedAttention.contains($0.sourceId) }) {
            capsule = .attention(attention)
        }
    }

    /// The capsule's × (and Esc): hide this attention until the source fails again.
    public func dismissAttention() {
        guard case .attention(let attention) = capsule else { return }
        dismissedAttention.insert(attention.sourceId)
        capsule = .hidden
    }

    /// The capsule's tinted fix bubble.
    public func performCapsuleFix() {
        guard case .attention(let attention) = capsule else { return }
        fixSource(attention.sourceId)
    }

    /// Replace Token… / Replace Feed Address… for a source whose secret was rejected: the one
    /// request every fix button (the capsule's bubble, the course header) makes. M1 has no
    /// Replace sheet, so it does the visible next best thing: Sources & Sync, scrolled to the
    /// source, whose problem callout explains the fix. M2 presents the Replace sheet here.
    public func fixSource(_ sourceId: String) {
        showSource(sourceId)
    }

    /// Opens Sources & Sync scrolled to `sourceId`, which is highlighted for a moment
    /// (`sourceHighlight`, cleared after `Timing.sourceHighlight`).
    public func showSource(_ sourceId: String) {
        destination = .sources
        highlightRequests += 1
        let highlight = SourceHighlight(sourceId: sourceId, request: highlightRequests)
        sourceHighlight = highlight
        highlightTimer?.cancel()
        let (sleep, delay) = (timing.sleep, timing.sourceHighlight)
        highlightTimer = Task { [weak self] in
            await sleep(delay)
            guard !Task.isCancelled else { return }
            self?.endSourceHighlight(highlight)
        }
    }

    /// Ends `highlight` (if it is still the current one).
    public func endSourceHighlight(_ highlight: SourceHighlight) {
        if sourceHighlight == highlight { sourceHighlight = nil }
    }

    // MARK: - Menu commands

    /// Runs a menu command whose result shows in the main window. The main window may be closed
    /// (only Settings open): `openMainWindow` (the scene's `openWindow(id: "main")`) runs first,
    /// which also brings an open window to the front.
    public func perform(_ command: MainWindowCommand, openMainWindow: () -> Void) async {
        // A window this opens shows the command's result, never an older stored destination.
        restoredWindowState = true
        openMainWindow()
        switch command {
        case .show(let destination):
            self.destination = destination
        case .stepWeek(let delta):
            stepWeek(by: delta)
        case .currentWeek:
            showCurrentWeek()
        case .diagnosticReport:
            await showDiagnosticReport(in: .main)
        case .liveData:
            if dataMode != .live { confirmingLiveData = true }
        }
    }

    // MARK: - Crash notice (S6)

    /// The crash notice's Dismiss: clears the record in the core (for the CLI's crashes too).
    public func dismissCrash() async {
        let generation = self.generation
        do throws(PageLampFailure) {
            try await service.clearLastCrash()
            guard generation == self.generation else { return }
            lastCrash = nil
            crashDismissFailure = nil
        } catch {
            guard generation == self.generation else { return }
            crashDismissFailure = error
        }
    }

    // MARK: - What's new

    /// Asks the facade whether this launch follows an update of the Mac app (it answers once per
    /// launch and shell, until acknowledged) and shows the topics this build can show. With none
    /// left it acknowledges at once: nothing should wait on a sheet that never shows. When the
    /// state can't be read (the settings aren't readable), it shows nothing and writes nothing.
    ///
    /// The same answer carries the reminders that came due since the last launch: the catch-up
    /// card shows them when reminders are off here (`ReminderDelivery.launch`).
    public func loadWhatsNew() async {
        let generation = self.generation
        let tasks: StartupTasks
        do throws(PageLampFailure) {
            tasks = try await service.startupTasks(now: clock())
        } catch {
            if generation == self.generation { weeklyNote?.launched(nil) }
            return
        }
        guard generation == self.generation else { return }
        weeklyNote?.launched(tasks)
        await reminderDelivery?.launch(due: tasks.dueReminders)
        guard generation == self.generation, let offered = tasks.whatsNew else { return }
        let l10n = self.l10n
        let items = WhatsNewCatalog.items(for: offered.topics) { l10n.has($0) }
        if items.isEmpty {
            await acknowledgeWhatsNew()
        } else {
            whatsNew = WhatsNewPresentation(since: offered.since, items: items)
        }
    }

    /// The sheet closed, any way (Got It, Esc): this version's What's new counts as read. The
    /// update disclosure is never acknowledged here; that is the Tauri app's.
    public func acknowledgeWhatsNew() async {
        whatsNew = nil
        // A failure is harmless: the facade offers What's new only on the first launch after an
        // update, so it doesn't come back next time either.
        try? await service.acknowledgeWhatsNew()
    }

    // MARK: - Data mode

    /// Switches to synthetic data (never touches the real data folder).
    public func useMock(_ scenario: MockScenario) async {
        guard !isSyncing else { return }
        replaceService(
            MockService(scenario: scenario, timing: timing.mock, calendar: calendar, now: clock),
            mode: .mock(scenario)
        )
        await refresh()
        // A scenario's Monday shows without waiting for the hour.
        await weeklyNote?.check()
    }

    /// Opens the real facade (default data folder, keychain). Call only after the student
    /// confirmed that the preview shares data with the installed PageLamp.
    public func useLive() async {
        guard !isSyncing else { return }
        replaceService(UnavailableService(failure: PageLampFailure(kind: .internal, message: "opening")), mode: .live)
        let generation = self.generation
        do throws(PageLampFailure) {
            let live = try await LiveService.openDefault()
            guard generation == self.generation else { return }
            service = live
            rewireReminders()
        } catch {
            guard generation == self.generation else { return }
            service = UnavailableService(failure: error)
            rewireReminders()
            phase = .unavailable(error)
            return
        }
        await refresh()
        await loadWhatsNew()
    }

    private func replaceService(_ service: any PageLampService, mode: DataMode) {
        generation += 1
        capsuleTimer?.cancel()
        busyPoll?.cancel()
        self.service = service
        dataMode = mode
        rewireReminders()
        phase = .loading
        status = nil
        courses = []
        sources = []
        upcomingDeadlines = []
        studyPlan = nil
        lastCrash = nil
        temporaryLocation = nil
        sectionErrors = [:]
        lastRun = nil
        syncProgress = nil
        capsule = .hidden
        dismissedAttention = []
        crashDismissFailure = nil
        highlightTimer?.cancel()
        sourceHighlight = nil
        courseStates = [:]
        whatsNew = nil
        menuBarWeek = nil
        menuBarWeekFailure = nil
        if case .course = destination { destination = .thisWeek }
    }

    /// The reminders follow the data source: live data uses the system's notification center and
    /// the app's record of what it handed over; mock data a center and a record in memory.
    private func rewireReminders() {
        guard let reminderDelivery else { return }
        switch dataMode {
        case .live:
            reminderDelivery.replace(service: service, center: reminderCenters.live(), record: settings)
        case .mock:
            reminderDelivery.replace(service: service, center: reminderCenters.mock, record: InMemorySettingsStore())
        }
    }

    /// Debug ▸ a mock sync in which Demo Canvas's token is rejected (attention state).
    public func runMockSyncWithRejectedToken() async {
        guard let mock = service as? MockService else { return }
        await mock.expireCanvasToken()
        await syncAll()
    }

    // MARK: - Diagnostics

    /// Help ▸ Copy Diagnostic Report…: loads the report for the preview sheet, presented by
    /// `host` (the main window, or Settings from Settings ▸ Help).
    public func showDiagnosticReport(in host: DiagnosticReportHost = .main) async {
        diagnosticReportHost = host
        diagnosticReport = .loading
        do throws(PageLampFailure) {
            let report = try await service.diagnosticReport()
            if diagnosticReport != nil { diagnosticReport = .loaded(report) }
        } catch {
            if diagnosticReport != nil { diagnosticReport = .failed(error) }
        }
    }

    public func dismissDiagnosticReport() {
        diagnosticReport = nil
    }
}
