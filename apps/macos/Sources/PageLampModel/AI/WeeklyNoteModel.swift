// The weekly note (design §5.3, §7; the Tauri app's WeeklyNoteCard and useWeeklyNote): one run for
// the whole app, written on a click or prepared on Monday, the kept notes and which one shows,
// Delete, "≈ $x", and Monday's line when its run didn't work. The run lives here, not in a view:
// leaving This Week never stops it, and Monday's run starts with no view open.
//
// Monday's note: the facade says when it's due (`startup_tasks().prepare_weekly_note`) and records
// one try a Monday in the same transaction as the run's start, so this asks as often as it likes
// and starts a run whenever the answer says so and nothing runs. It asks at launch (60 s later
// when PageLamp opens at login, before the network or a local model may be up), every hour while
// the app runs, 60 s after a wake (the hour stops when the Mac goes to sleep and restarts at the
// wake, so an hour that passed asleep never fires then), and on activation at most every 15
// minutes or on a new day. Monday's run never
// goes over the budget, never moves focus, and ends quietly: "not due any more" and "nothing to
// write about" (the week emptied as the run started; Write already says so) say nothing, anything
// else leaves one line on the card.

import AppKit
import Foundation
import Observation
import PageLampKit
import ServiceManagement

@Observable @MainActor
public final class WeeklyNoteModel {
    /// When Monday's note is asked about.
    public struct Timing: Sendable {
        /// Between two reads while the app runs.
        public var interval: Duration
        /// After a wake, or a launch at login, before the read.
        public var settle: Duration
        /// An activation reads again only this long after the last read (or on a new day).
        public var activationGap: TimeInterval
        public var sleep: AppModel.Timing.Sleep

        public init(
            interval: Duration = .seconds(3600),
            settle: Duration = .seconds(60),
            activationGap: TimeInterval = 15 * 60,
            sleep: @escaping AppModel.Timing.Sleep = { try? await Task.sleep(for: $0) }
        ) {
            self.interval = interval
            self.settle = settle
            self.activationGap = activationGap
            self.sleep = sleep
        }
    }

    public private(set) var run: GenerationRun<WeeklyNote>
    /// "≈ $x" for Write (the note has no week or course to choose).
    public private(set) var estimate: CostEstimateModel
    /// The kept notes, newest first, as last read.
    public private(set) var notes: [WeeklyNote] = []
    /// The notes have been read at least once (until then the card shows no "empty" line).
    public private(set) var loaded = false
    /// The run in flight, or the last one, is Monday's.
    public private(set) var automatic = false
    /// Why Monday's run didn't write a note (one line on the card), until a click or a note.
    public private(set) var automaticProblem: PageLampFailure?
    /// The note picked in the history; nil = the run's new one, else the newest.
    public private(set) var shownId: String?
    /// Deleted here, or gone from a later read: never shown again.
    public private(set) var deleted: Set<String> = []
    public private(set) var deleting = false
    public private(set) var deleteFailure: PageLampFailure?

    @ObservationIgnored private var service: any PageLampService
    @ObservationIgnored private let clock: @Sendable () -> Date
    @ObservationIgnored private let calendar: Calendar
    @ObservationIgnored private let timing: Timing
    @ObservationIgnored private let debounce: Duration
    @ObservationIgnored private let newId: @Sendable () -> String
    @ObservationIgnored private let notificationCenter: NotificationCenter
    @ObservationIgnored private let workspaceCenter: NotificationCenter
    @ObservationIgnored private let startsAtLogin: @MainActor () -> Bool
    @ObservationIgnored private let uiLanguage: @MainActor () -> String
    @ObservationIgnored private var lastRead: Date?
    /// Until then (60 s after a wake), an activation doesn't ask: the settle read does.
    @ObservationIgnored private var settleUntil: Date?
    @ObservationIgnored private var loop: Task<Void, Never>?
    @ObservationIgnored private var observers: [Task<Void, Never>] = []
    @ObservationIgnored private var mondayRun: Task<Void, Never>?
    @ObservationIgnored private var mondayRuns = 0
    @ObservationIgnored private var noteLoads = 0
    /// Bumped with each data source: an answer from the one before is dropped.
    @ObservationIgnored private var generation = 0

    /// - Parameters:
    ///   - notificationCenter: the app's (activation); `workspaceCenter`: NSWorkspace's (wake).
    ///   - startsAtLogin: whether PageLamp opens at login (the launch read waits then).
    ///   - uiLanguage: the answers' language for Monday's run ("en" / "zh-Hans").
    public init(
        service: any PageLampService,
        clock: @escaping @Sendable () -> Date,
        calendar: Calendar,
        timing: Timing = Timing(),
        notificationCenter: NotificationCenter = .default,
        workspaceCenter: NotificationCenter = NSWorkspace.shared.notificationCenter,
        startsAtLogin: @escaping @MainActor () -> Bool = { SMAppService.mainApp.status == .enabled },
        uiLanguage: @escaping @MainActor () -> String,
        debounce: Duration = .milliseconds(300),
        newId: @escaping @Sendable () -> String = randomGenerationId
    ) {
        self.service = service
        self.clock = clock
        self.calendar = calendar
        self.timing = timing
        self.notificationCenter = notificationCenter
        self.workspaceCenter = workspaceCenter
        self.startsAtLogin = startsAtLogin
        self.uiLanguage = uiLanguage
        self.debounce = debounce
        self.newId = newId
        run = GenerationRun(service: service, newId: newId)
        estimate = CostEstimateModel(service: service, debounce: debounce)
    }

    isolated deinit {
        close()
    }

    // MARK: - The notes

    /// Reads the kept notes and "≈ $x" (the card shows, the data source changed, or something
    /// else may have deleted notes: Settings ▸ AI, another app). The note's request never changes,
    /// so "≈ $x" is read anew each time (a run elsewhere may have spent the budget, a sync grown the
    /// week), and going over the budget is chosen again on each visit.
    public func load() async {
        estimate.overrideBudget = false
        if estimate.request == .weeklyNote {
            await estimate.refresh()
        } else {
            estimate.update(.weeklyNote)
        }
        await loadNotes()
    }

    /// Reads the notes and "≈ $x" again (back from Settings ▸ AI or another app).
    public func refresh() async {
        await estimate.refresh()
        await loadNotes()
    }

    private func loadNotes() async {
        noteLoads += 1
        let (load, generation, service) = (noteLoads, self.generation, self.service)
        let list: [WeeklyNote]
        do throws(PageLampFailure) {
            list = try await service.weeklyNotes()
        } catch {
            // Unread: what was read before stays (a failed read says nothing about what's gone;
            // Write still works).
            guard load == noteLoads, generation == self.generation else { return }
            loaded = true
            return
        }
        guard load == noteLoads, generation == self.generation else { return }
        notes = list
        loaded = true
        // The run's note, gone from a read after it was kept (removed with all AI data, or a
        // course's generated content): not shown again.
        if case .finished(let fresh) = run.phase, !list.contains(where: { $0.meta.generationId == fresh.meta.generationId }) {
            deleted.insert(fresh.meta.generationId)
        }
    }

    /// The kept notes, without those deleted here.
    public var history: [WeeklyNote] {
        notes.filter { !deleted.contains($0.meta.generationId) }
    }

    /// The note on the card: the one picked in the history, else the run's new one, else the
    /// newest kept.
    public var shown: WeeklyNote? {
        let history = self.history
        if let shownId, let picked = history.first(where: { $0.meta.generationId == shownId }) {
            return picked
        }
        if case .finished(let fresh) = run.phase, !deleted.contains(fresh.meta.generationId) {
            return fresh
        }
        return history.first
    }

    public func show(_ note: WeeklyNote) {
        shownId = note.meta.generationId
        deleteFailure = nil
    }

    // MARK: - Writing

    /// Write my weekly note (a click): from Write's own "≈ $x"; going over the budget is chosen
    /// again next to it for each run.
    public func write(uiLanguage: String) async {
        guard estimate.canGenerate, !run.isRunning else { return }
        let options = WeeklyNoteOptions(uiLanguage: uiLanguage, overrideBudget: estimate.goesOverBudget, automatic: false)
        let generation = self.generation
        automatic = false
        automaticProblem = nil
        deleteFailure = nil
        shownId = nil
        estimate.overrideBudget = false
        await run.run { service, id, observer async throws(PageLampFailure) in
            try await service.writeWeeklyNote(generationId: id, options: options, observer: observer)
        }
        await ended(generation)
    }

    /// Monday's note, when the facade said it's due and nothing runs. It never goes over the
    /// budget; "not due any more" (another app, or an earlier read, got there first) and
    /// "nothing to write about" end silently, anything else leaves `automaticProblem`.
    private func writeMonday() async {
        guard !run.isRunning else { return }
        let options = WeeklyNoteOptions(uiLanguage: uiLanguage(), overrideBudget: false, automatic: true)
        let generation = self.generation
        automatic = true
        // A tick left from an earlier visit never rides on a later click.
        estimate.overrideBudget = false
        await run.run(
            { service, id, observer async throws(PageLampFailure) in
                try await service.writeWeeklyNote(generationId: id, options: options, observer: observer)
            },
            onFailure: { [weak self] failure in
                // A run of the data source before says nothing about this one.
                let quiet = failure.kind == .invalid || failure.blocked == .nothingToWrite
                if let self, self.generation == generation, !quiet { self.automaticProblem = failure }
                return .idle
            }
        )
        await ended(generation)
    }

    /// After any end: a note clears Monday's line; the list and "≈ $x" are read again (a stopped
    /// or failed run is billed too). Nothing, when the data source changed meanwhile.
    private func ended(_ generation: Int) async {
        guard generation == self.generation else { return }
        if case .finished = run.phase {
            automaticProblem = nil
            deleteFailure = nil
        }
        await loadNotes()
        await estimate.refresh()
    }

    public func stop() async {
        await run.stop()
    }

    // MARK: - Delete

    /// Deletes a note (asked first). Returns whether it's gone; `deleteFailure` says why not.
    public func delete(_ note: WeeklyNote) async -> Bool {
        guard !deleting else { return false }
        deleting = true
        deleteFailure = nil
        defer { deleting = false }
        let (id, generation, service) = (note.meta.generationId, self.generation, self.service)
        do throws(PageLampFailure) {
            try await service.deleteWeeklyNote(generationId: id)
        } catch {
            guard generation == self.generation else { return false }
            deleteFailure = error
            return false
        }
        guard generation == self.generation else { return false }
        deleted.insert(id)
        shownId = nil
        await loadNotes()
        return true
    }

    // MARK: - Monday

    /// Starts asking about Monday's note: every `interval`, after a wake and on activation.
    /// Idempotent (the app calls it once at start).
    public func start() {
        guard observers.isEmpty else { return }
        let center = notificationCenter
        let workspace = workspaceCenter
        observers = [
            Task { [weak self] in
                for await _ in center.notifications(named: NSApplication.didBecomeActiveNotification) {
                    await self?.activated()
                }
            },
            // Asleep, no countdown runs: the timers count time asleep, and an hour that ran out
            // then would fire at the wake, before the network or a local model is up.
            Task { [weak self] in
                for await _ in workspace.notifications(named: NSWorkspace.willSleepNotification) {
                    self?.pause()
                }
            },
            Task { [weak self] in
                for await _ in workspace.notifications(named: NSWorkspace.didWakeNotification) {
                    self?.woke()
                }
            },
        ]
        if loop == nil { schedule(first: timing.interval) }
    }

    /// The launch's answer (read with What's new; nil when that read failed): used at once,
    /// unless PageLamp opened at login, when the network or a local model may not be up yet: then
    /// it asks again shortly. A failed read counts as a read (the next activation 15 minutes on,
    /// or the hour, asks again).
    public func launched(_ tasks: StartupTasks?) {
        lastRead = clock()
        if startsAtLogin() {
            schedule(first: timing.settle)
        } else if let tasks {
            prepare(tasks)
            schedule(first: timing.interval)
        }
    }

    /// Asks the facade now (a turn of the hour, a wake, an activation, another data source).
    public func check() async {
        let now = clock()
        lastRead = now
        let (generation, service) = (self.generation, self.service)
        let tasks: StartupTasks
        do throws(PageLampFailure) {
            tasks = try await service.startupTasks(now: now)
        } catch {
            return
        }
        guard generation == self.generation else { return }
        prepare(tasks)
    }

    /// Starts Monday's run when the answer says it's due and nothing runs (a click's run going:
    /// the next read decides).
    private func prepare(_ tasks: StartupTasks) {
        guard tasks.prepareWeeklyNote, !run.isRunning, mondayRun == nil else { return }
        mondayRuns += 1
        let token = mondayRuns
        mondayRun = Task { [weak self] in
            await self?.writeMonday()
            if self?.mondayRuns == token { self?.mondayRun = nil }
        }
    }

    /// The wake: the read comes once things are up (60 s), and the hour restarts from it. An
    /// activation meanwhile (unlocking, a Dock click, a notification clicked at the wake) waits
    /// for that read rather than spending Monday's one try on a network still coming back.
    private func woke() {
        settleUntil = clock().addingTimeInterval(Self.seconds(timing.settle))
        schedule(first: timing.settle)
    }

    private func activated() async {
        guard let last = lastRead else { return }
        let now = clock()
        if let settleUntil, now < settleUntil { return }
        if now.timeIntervalSince(last) >= timing.activationGap || !calendar.isDate(now, inSameDayAs: last) {
            await check()
        }
    }

    private static func seconds(_ duration: Duration) -> TimeInterval {
        let parts = duration.components
        return TimeInterval(parts.seconds) + TimeInterval(parts.attoseconds) / 1e18
    }

    /// The Mac goes to sleep: no countdown until the wake starts one.
    private func pause() {
        loop?.cancel()
        loop = nil
    }

    /// (Re)starts the countdown: the first read after `first`, then every `interval`.
    private func schedule(first: Duration) {
        loop?.cancel()
        let (sleep, interval) = (timing.sleep, timing.interval)
        loop = Task { [weak self] in
            var delay = first
            while !Task.isCancelled {
                await sleep(delay)
                guard !Task.isCancelled, let self else { return }
                await self.check()
                delay = interval
            }
        }
    }

    // MARK: - The data source

    /// Another data source (Debug ▸ Data Source, the live facade opening): the run in flight is
    /// cancelled and everything is read from the new one.
    public func replace(service: any PageLampService) {
        run.cancelInFlight()
        mondayRun?.cancel()
        mondayRun = nil
        mondayRuns += 1
        generation += 1
        self.service = service
        run = GenerationRun(service: service, newId: newId)
        estimate = CostEstimateModel(service: service, debounce: debounce)
        notes = []
        loaded = false
        automatic = false
        automaticProblem = nil
        shownId = nil
        deleted = []
        deleteFailure = nil
    }

    /// Stops asking (the app's model goes away).
    public func close() {
        loop?.cancel()
        loop = nil
        observers.forEach { $0.cancel() }
        observers = []
        mondayRun?.cancel()
    }
}

extension WeeklyNoteModel {
    /// What the note writes about, as This Week knows it at `now`: the day, the courses that show
    /// and whether each is active, the deadlines due within the next 7 days and the plan. It
    /// changes when one of those does (a sync, a hidden course, a deadline coming within 7 days,
    /// midnight), not every minute, and the card reads "≈ $x" again then. `load()` and the estimate
    /// never change it, so that can't loop.
    nonisolated public static func weekKey(
        courses: [CourseSummary], deadlines: [Deadline], plan: StoredStudyPlan?, now: Date, calendar: Calendar
    ) -> [String] {
        let day = calendar.startOfDay(for: now)
        let horizon = now.addingTimeInterval(7 * 86_400)
        let due = deadlines.filter { deadline in
            guard let when = deadline.event.dueAt ?? deadline.event.startsAt else { return false }
            return when >= now && when <= horizon
        }
        return ["day:\(day.timeIntervalSinceReferenceDate)"]
            + courses.map { "\($0.course.id):\($0.course.hidden):\($0.lifecycle.isActive)" }
            + due.map(\.event.id)
            + [plan.map { "plan:\($0.id):\($0.plan.items.count)" } ?? "plan:none"]
    }
}
