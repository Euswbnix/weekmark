// The weekly note's model on the mock: a click's note, Monday's (prepared when a read says so and
// nothing runs; quiet when it fails), when it asks (launch, the hour, 60 s after a wake with the
// hour restarted, an activation after 15 minutes or on a new day, a launch at login 60 s later),
// Delete and notes deleted elsewhere, another data source; GenerationRun's failure mapping; and
// the note's words in both languages.

import AppKit
import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Synchronization
import Testing

private let en = L10n(locale: Locale(identifier: "en_US"), table: .app)
private let zh = L10n(locale: Locale(identifier: "zh-Hans_CN"), table: .app)

/// A clock the test moves.
private final class TestTime: Sendable {
    private let value: Mutex<Date>
    init(_ date: Date = TestClock.now) { value = Mutex(date) }
    var now: Date { value.withLock { $0 } }
    func set(_ date: Date) { value.withLock { $0 = date } }
}

/// Counts `startupTasks` reads; can say Monday's note is due whatever the mock thinks.
private actor Reads {
    private(set) var count = 0
    var forceDue = false

    func record() { count += 1 }
    func force(_ on: Bool) { forceDue = on }
}

private struct CountingReads: ForwardingService {
    let base: any PageLampService
    let reads: Reads

    func startupTasks(now: Date) async throws(PageLampFailure) -> StartupTasks {
        await reads.record()
        let tasks = try await base.startupTasks(now: now)
        guard await reads.forceDue else { return tasks }
        return StartupTasks(whatsNew: tasks.whatsNew, updateCheckDue: tasks.updateCheckDue, updatedFrom: tasks.updatedFrom, prepareWeeklyNote: true)
    }
}

/// Counts cancels.
private actor Cancels {
    private(set) var count = 0
    func record() { count += 1 }
}

private struct CountingCancels: ForwardingService {
    let base: any PageLampService
    let cancels: Cancels

    /// Counted once the cancel has reached the service (a test may then let the run go on).
    func cancelGeneration(generationId: String) async throws(PageLampFailure) {
        try await base.cancelGeneration(generationId: generationId)
        await cancels.record()
    }
}

/// Counts note runs started (clicks and Monday's).
private actor Writes {
    private(set) var count = 0
    func record() { count += 1 }
}

private struct CountingWrites: ForwardingService {
    let base: any PageLampService
    let writes: Writes

    func writeWeeklyNote(generationId: String, options: WeeklyNoteOptions, observer: any GenObserver) async throws(PageLampFailure) -> WeeklyNote {
        await writes.record()
        return try await base.writeWeeklyNote(generationId: generationId, options: options, observer: observer)
    }
}

/// A run that finds the week empty as it starts (the race after Monday's check).
private struct EmptyWeekWrites: ForwardingService {
    let base: any PageLampService

    func writeWeeklyNote(generationId: String, options: WeeklyNoteOptions, observer: any GenObserver) async throws(PageLampFailure) -> WeeklyNote {
        throw PageLampFailure(kind: .blocked, message: "There is nothing to write about this week.", blocked: .nothingToWrite)
    }
}

/// Lets tasks the code under test started run (Monday's run starts in a task of its own).
@MainActor
private func settle() async {
    for _ in 0..<20 { await Task.yield() }
    try? await Task.sleep(for: .milliseconds(10))
}

/// Reads of the kept notes fail while the switch is on.
private actor NotesSwitch {
    private(set) var failing = false
    func set(_ on: Bool) { failing = on }
}

private struct FailingNotes: ForwardingService {
    let base: any PageLampService
    let notes: NotesSwitch

    func weeklyNotes() async throws(PageLampFailure) -> [WeeklyNote] {
        if await notes.failing { throw PageLampFailure(kind: .internal, message: "unreadable") }
        return try await base.weeklyNotes()
    }

    func deleteWeeklyNote(generationId: String) async throws(PageLampFailure) {
        if await notes.failing { throw PageLampFailure(kind: .internal, message: "locked") }
        try await base.deleteWeeklyNote(generationId: generationId)
    }
}

/// Monday 2026-09-28 at `hour` (Toronto).
private func monday(_ hour: Int = 10) -> Date {
    TestClock.at(3, hour)
}

private func mock(_ scenario: MockScenario, time: TestTime, gate: SyncStepGate? = nil) -> MockService {
    MockService(
        scenario: scenario, timing: MockService.Timing(latency: .zero, syncStep: .zero, gate: gate),
        calendar: TestClock.calendar, now: { time.now }
    )
}

/// A note model over `service`, its timers and notifications in the test's hands.
@MainActor
private func noteModel(
    _ service: any PageLampService,
    time: TestTime,
    timers: ManualTimers = ManualTimers(),
    app: NotificationCenter = NotificationCenter(),
    workspace: NotificationCenter = NotificationCenter(),
    atLogin: Bool = false
) -> WeeklyNoteModel {
    WeeklyNoteModel(
        service: service, clock: { time.now }, calendar: TestClock.calendar,
        timing: WeeklyNoteModel.Timing(sleep: timers.sleep), notificationCenter: app, workspaceCenter: workspace,
        startsAtLogin: { atLogin }, uiLanguage: { "en" }, debounce: .zero
    )
}

/// Waits (briefly) until `condition` holds.
@MainActor
private func eventually(_ condition: () async -> Bool) async -> Bool {
    for _ in 0..<2_000 {
        if await condition() { return true }
        try? await Task.sleep(for: .milliseconds(1))
    }
    return await condition()
}

@MainActor
private func finished(_ note: WeeklyNoteModel) -> WeeklyNote? {
    if case .finished(let written) = note.run.phase { return written }
    return nil
}

// MARK: - A run's failure

@Suite("Generation runs: where a failure ends")
@MainActor
struct GenerationRunFailureTests {
    @Test("a failure ends .failed, or wherever the caller maps it (never .failed then)")
    func mapped() async {
        let time = TestTime()
        let run = GenerationRun<WeeklyNote>(service: mock(.demo, time: time))
        let call: GenerationRun<WeeklyNote>.Call = { service, id, observer async throws(PageLampFailure) in
            try await service.writeWeeklyNote(generationId: id, options: WeeklyNoteOptions(), observer: observer)
        }
        await run.run(call)
        guard case .failed(let failure) = run.phase else {
            Issue.record("expected failed")
            return
        }
        #expect(failure.blocked == .noModelChosen)
        var seen: [PageLampFailure] = []
        await run.run(call, onFailure: { failure in
            seen.append(failure)
            return .idle
        })
        guard case .idle = run.phase else {
            Issue.record("expected idle")
            return
        }
        #expect(seen.count == 1 && !run.isRunning)
    }
}

// MARK: - The model

@Suite("Weekly note") @MainActor
struct WeeklyNoteModelTests {
    @Test("Write: the run's note shows, kept, newest first; Monday's line goes")
    func write() async throws {
        let time = TestTime()
        let note = noteModel(mock(.aiKey, time: time), time: time)
        await note.load()
        await note.estimate.settle()
        #expect(note.loaded && note.notes.isEmpty && note.shown == nil && note.estimate.canGenerate)
        await note.write(uiLanguage: "en")
        let first = try #require(finished(note))
        #expect(!first.automatic && !note.automatic && note.shown?.meta.generationId == first.meta.generationId)
        time.set(TestClock.now.addingTimeInterval(60))
        await note.write(uiLanguage: "en")
        let second = try #require(finished(note))
        #expect(note.history.map { $0.meta.generationId } == [second.meta.generationId, first.meta.generationId])
        note.show(first)
        #expect(note.shown?.meta.generationId == first.meta.generationId)
    }

    @Test("the week's key: the same a minute later, new when a deadline comes within 7 days or the day changes")
    func weekKey() async throws {
        let service = mock(.aiKey, time: TestTime())
        let courses = try await service.listCourses()
        let deadlines = try await service.listDeadlines(course: nil, daysAhead: 60, daysBack: 0)
        func key(_ now: Date) -> [String] {
            WeeklyNoteModel.weekKey(courses: courses, deadlines: deadlines, plan: nil, now: now, calendar: TestClock.calendar)
        }
        #expect(key(TestClock.now) == key(TestClock.now.addingTimeInterval(60)))
        // A deadline more than 7 days out joins the key once it is 7 days away.
        let week: TimeInterval = 7 * 86_400
        let later = try #require(
            deadlines.filter { ($0.event.dueAt ?? $0.event.startsAt).map { $0 > TestClock.now.addingTimeInterval(week + 3_600) } ?? false }
                .min { ($0.event.dueAt ?? $0.event.startsAt ?? .distantFuture) < ($1.event.dueAt ?? $1.event.startsAt ?? .distantFuture) }
        )
        let due = try #require(later.event.dueAt ?? later.event.startsAt)
        #expect(!key(due.addingTimeInterval(-week - 30)).contains(later.event.id))
        #expect(key(due.addingTimeInterval(-week + 30)).contains(later.event.id))
        // Midnight starts a new key, whatever else stays.
        #expect(key(TestClock.at(0, 23, 59)).first != key(TestClock.at(1, 0, 1)).first)
    }

    @Test("Monday's note: a read that says it's due prepares it once, marked automatic")
    func mondayNote() async throws {
        let time = TestTime()
        let writes = Writes()
        let note = noteModel(CountingWrites(base: mock(.weeklyNoteMonday, time: time), writes: writes), time: time)
        await note.load()
        await note.check()
        #expect(await eventually { finished(note) != nil })
        let written = try #require(finished(note))
        #expect(written.automatic && note.automatic && note.automaticProblem == nil)
        // Asked again: not due any more, nothing starts.
        await note.check()
        await settle()
        #expect(await writes.count == 1 && note.history.count == 1)
    }

    @Test("Monday's run never fails loudly: blocked leaves one line, \"not due\" says nothing")
    func quietFailures() async throws {
        let time = TestTime(monday())
        let base = mock(.aiKey, time: time)
        _ = try await base.setPrepareWeeklyNoteOnMonday(on: true)
        try await base.setMonthlyBudget(microUsd: 1)
        let note = noteModel(base, time: time)
        await note.check()
        #expect(await eventually { note.automaticProblem != nil })
        #expect(note.automaticProblem?.blocked == .budgetReached)
        guard case .idle = note.run.phase else {
            Issue.record("Monday's run should end idle, never failed")
            return
        }
        // Another read says it's due (another app tried meanwhile): the facade answers "not due",
        // which says nothing new.
        let reads = Reads()
        await reads.force(true)
        let forced = noteModel(CountingReads(base: base, reads: reads), time: time)
        await forced.check()
        #expect(await eventually { forced.automatic && !forced.run.isRunning })
        #expect(forced.automaticProblem == nil)
        guard case .idle = forced.run.phase else {
            Issue.record("expected idle")
            return
        }
        // A click clears the line, whatever its end.
        await note.load()
        await note.estimate.settle()
        #expect(note.estimate.block == .budgetReached)
        note.estimate.overrideBudget = true
        await note.write(uiLanguage: "en")
        #expect(note.automaticProblem == nil && finished(note) != nil)
    }

    @Test("an empty week: Write is off and says why before the click")
    func nothingToWrite() async throws {
        let time = TestTime()
        let empty = mock(.empty, time: time)
        _ = try await empty.addModelProvider(preset: "ollama", baseUrl: nil, apiKey: nil)
        try await empty.setFeatureModel(
            feature: .weeklyNote, choice: ModelChoice(backend: .provider(providerId: "ollama"), model: "qwen3.5:9b", effort: .lowest)
        )
        let note = noteModel(empty, time: time)
        await note.load()
        await note.estimate.settle()
        #expect(note.estimate.block == .nothingToWrite && !note.estimate.canGenerate && !note.estimate.showsCost)
        await note.write(uiLanguage: "en")
        guard case .idle = note.run.phase else {
            Issue.record("Write is off: nothing should start")
            return
        }
    }

    @Test("Monday's run that finds the week empty as it starts says nothing")
    func mondayEmptyIsQuiet() async throws {
        let time = TestTime(monday())
        let base = mock(.aiKey, time: time)
        _ = try await base.setPrepareWeeklyNoteOnMonday(on: true)
        let note = noteModel(EmptyWeekWrites(base: base), time: time)
        await note.check()
        #expect(await eventually { note.automatic && !note.run.isRunning })
        #expect(note.automaticProblem == nil)
        guard case .idle = note.run.phase else {
            Issue.record("expected idle")
            return
        }
    }

    @Test("coming back to the card estimates again, and going over the budget is chosen again")
    func revisit() async throws {
        let time = TestTime()
        let base = mock(.aiKey, time: time)
        let note = noteModel(base, time: time)
        await note.load()
        await note.estimate.settle()
        #expect(note.estimate.canGenerate && note.estimate.block == nil)
        // Meanwhile (an explanation elsewhere, Settings ▸ AI) the budget is all but spent.
        try await base.setMonthlyBudget(microUsd: 1)
        await note.load()
        #expect(note.estimate.block == .budgetReached && !note.estimate.canGenerate)
        note.estimate.overrideBudget = true
        #expect(note.estimate.canGenerate)
        // Away and back: the tick goes.
        await note.load()
        #expect(!note.estimate.overrideBudget && !note.estimate.canGenerate)
    }

    @Test("a click clears Monday's line even when it doesn't write a note (stopped)")
    func clickClearsLine() async throws {
        let gate = SyncStepGate()
        let time = TestTime(monday())
        let base = mock(.aiKey, time: time, gate: gate)
        _ = try await base.setPrepareWeeklyNoteOnMonday(on: true)
        try await base.setMonthlyBudget(microUsd: 1)
        let note = noteModel(base, time: time)
        await note.check()
        #expect(await eventually { note.automaticProblem != nil })
        // The budget raised: a click, stopped before it writes.
        try await base.setMonthlyBudget(microUsd: nil)
        await note.load()
        await note.estimate.settle()
        let click = Task { await note.write(uiLanguage: "en") }
        _ = await gate.held()
        await note.stop()
        await gate.open()
        await click.value
        guard case .stopped = note.run.phase else {
            Issue.record("expected stopped")
            return
        }
        #expect(note.automaticProblem == nil)
    }

    @Test("a read while a click's run goes starts nothing; the next read does (no answer is skipped for good)")
    func busy() async throws {
        let gate = SyncStepGate()
        let time = TestTime()
        let writes = Writes()
        let note = noteModel(CountingWrites(base: mock(.weeklyNoteMonday, time: time, gate: gate), writes: writes), time: time)
        await note.load()
        await note.estimate.settle()
        let click = Task { await note.write(uiLanguage: "en") }
        _ = await gate.held()
        await note.check()
        await settle()
        #expect(await writes.count == 1 && !note.automatic)
        await gate.open()
        await click.value
        #expect(await writes.count == 1 && !note.automatic)
        // The click's note was written today: in the scenario a note today isn't due (the facade's
        // rule), so delete it; the next read then prepares Monday's.
        let clicked = try #require(finished(note))
        #expect(await note.delete(clicked))
        await note.check()
        #expect(await eventually { finished(note)?.automatic == true })
    }

    @Test("it asks every hour; asleep, no hour runs; 60 s after a wake it asks, and the hour restarts")
    func hourAndWake() async throws {
        let time = TestTime()
        let timers = ManualTimers()
        let workspace = NotificationCenter()
        let reads = Reads()
        let note = noteModel(CountingReads(base: mock(.aiKey, time: time), reads: reads), time: time, timers: timers, workspace: workspace)
        note.start()
        await timers.waitForTimer(.seconds(3600))
        await timers.fire(.seconds(3600))
        #expect(await eventually { await reads.count == 1 })
        await timers.waitForTimer(.seconds(3600))
        // Going to sleep: the hour waiting is dropped (time asleep must not run it out).
        workspace.post(name: NSWorkspace.willSleepNotification, object: nil)
        #expect(await eventually { await timers.pending.isEmpty })
        // The wake: the next read is 60 s away.
        workspace.post(name: NSWorkspace.didWakeNotification, object: nil)
        await timers.waitForTimer(.seconds(60))
        #expect(await eventually { await timers.pending == [.seconds(60)] })
        await timers.fire(.seconds(60))
        #expect(await eventually { await reads.count == 2 })
        await timers.waitForTimer(.seconds(3600))
        note.close()
    }

    @Test("after a wake an activation waits for the settle read, even on a new day")
    func wakeSettles() async throws {
        let time = TestTime()
        let timers = ManualTimers()
        let app = NotificationCenter()
        let workspace = NotificationCenter()
        let reads = Reads()
        let service = CountingReads(base: mock(.aiKey, time: time), reads: reads)
        let note = noteModel(service, time: time, timers: timers, app: app, workspace: workspace)
        note.start()
        try await Task.sleep(for: .milliseconds(20))
        note.launched(try await service.startupTasks(now: time.now))
        #expect(await reads.count == 1)
        // Overnight asleep; Monday morning the Mac wakes and is unlocked at once.
        workspace.post(name: NSWorkspace.willSleepNotification, object: nil)
        time.set(TestClock.at(3, 7))
        workspace.post(name: NSWorkspace.didWakeNotification, object: nil)
        await timers.waitForTimer(.seconds(60))
        time.set(TestClock.at(3, 7).addingTimeInterval(5))
        app.post(name: NSApplication.didBecomeActiveNotification, object: nil)
        try await Task.sleep(for: .milliseconds(20))
        #expect(await reads.count == 1)
        // The settle read.
        time.set(TestClock.at(3, 7).addingTimeInterval(60))
        await timers.fire(.seconds(60))
        #expect(await eventually { await reads.count == 2 })
        note.close()
    }

    @Test("an activation asks again after 15 minutes or on a new day, never before the launch's read")
    func activation() async throws {
        let time = TestTime()
        let app = NotificationCenter()
        let reads = Reads()
        let service = CountingReads(base: mock(.aiKey, time: time), reads: reads)
        let note = noteModel(service, time: time, app: app)
        note.start()
        // The observers listen once the main actor is free.
        try await Task.sleep(for: .milliseconds(20))
        app.post(name: NSApplication.didBecomeActiveNotification, object: nil)
        try await Task.sleep(for: .milliseconds(20))
        #expect(await reads.count == 0)
        note.launched(try await service.startupTasks(now: time.now))
        #expect(await reads.count == 1)
        time.set(TestClock.now.addingTimeInterval(10 * 60))
        app.post(name: NSApplication.didBecomeActiveNotification, object: nil)
        try await Task.sleep(for: .milliseconds(20))
        #expect(await reads.count == 1)
        time.set(TestClock.now.addingTimeInterval(16 * 60))
        app.post(name: NSApplication.didBecomeActiveNotification, object: nil)
        #expect(await eventually { await reads.count == 2 })
        // A read just before midnight, then one 10 minutes on: a new day reads at once.
        time.set(TestClock.at(0, 23, 55))
        app.post(name: NSApplication.didBecomeActiveNotification, object: nil)
        #expect(await eventually { await reads.count == 3 })
        time.set(TestClock.at(1, 0, 5))
        app.post(name: NSApplication.didBecomeActiveNotification, object: nil)
        #expect(await eventually { await reads.count == 4 })
        note.close()
    }

    @Test("a launch whose read failed counts as a read: an activation 15 minutes on asks")
    func launchReadFailed() async throws {
        let time = TestTime()
        let app = NotificationCenter()
        let reads = Reads()
        let note = noteModel(CountingReads(base: mock(.aiKey, time: time), reads: reads), time: time, app: app)
        note.start()
        try await Task.sleep(for: .milliseconds(20))
        note.launched(nil)
        time.set(TestClock.now.addingTimeInterval(5 * 60))
        app.post(name: NSApplication.didBecomeActiveNotification, object: nil)
        try await Task.sleep(for: .milliseconds(20))
        #expect(await reads.count == 0)
        time.set(TestClock.now.addingTimeInterval(16 * 60))
        app.post(name: NSApplication.didBecomeActiveNotification, object: nil)
        #expect(await eventually { await reads.count == 1 })
        note.close()
    }

    @Test("a launch at login waits 60 s before Monday's note; an ordinary launch uses its answer")
    func launchAtLogin() async throws {
        let time = TestTime()
        let timers = ManualTimers()
        let service = mock(.weeklyNoteMonday, time: time)
        let tasks = try await service.startupTasks(now: time.now)
        #expect(tasks.prepareWeeklyNote)
        let login = noteModel(service, time: time, timers: timers, atLogin: true)
        login.launched(tasks)
        try await Task.sleep(for: .milliseconds(20))
        #expect(!login.run.isRunning && finished(login) == nil)
        await timers.waitForTimer(.seconds(60))
        await timers.fire(.seconds(60))
        #expect(await eventually { finished(login)?.automatic == true })
        login.close()
        // Not at login: at once (another mock, so Monday is due again).
        let other = mock(.weeklyNoteMonday, time: time)
        let ordinary = noteModel(other, time: time, atLogin: false)
        ordinary.launched(try await other.startupTasks(now: time.now))
        #expect(await eventually { finished(ordinary)?.automatic == true })
        ordinary.close()
    }

    @Test("Delete: gone at once; a note deleted elsewhere goes at the next read")
    func delete() async throws {
        let time = TestTime()
        let service = mock(.aiKey, time: time)
        let note = noteModel(service, time: time)
        await note.load()
        await note.estimate.settle()
        await note.write(uiLanguage: "en")
        time.set(TestClock.now.addingTimeInterval(60))
        await note.write(uiLanguage: "en")
        let newest = try #require(note.shown)
        #expect(await note.delete(newest))
        #expect(note.shown?.meta.generationId != newest.meta.generationId && note.history.count == 1)
        #expect(await note.delete(newest) == false && note.deleteFailure?.kind == .notFound)
        // Settings ▸ AI ▸ Remove All AI Data: the next read drops the run's note too.
        _ = try await service.removeAllAiData()
        await note.refresh()
        #expect(note.shown == nil && note.history.isEmpty)
    }

    @Test("a failed read of the notes keeps what was read; a failed Delete says so until a pick or a run")
    func failedReads() async throws {
        let time = TestTime()
        let notesSwitch = NotesSwitch()
        let note = noteModel(FailingNotes(base: mock(.aiKey, time: time), notes: notesSwitch), time: time)
        await note.load()
        await note.estimate.settle()
        await note.write(uiLanguage: "en")
        let written = try #require(finished(note))
        await notesSwitch.set(true)
        await note.refresh()
        #expect(note.shown?.meta.generationId == written.meta.generationId)
        #expect(note.history.count == 1 && note.deleted.isEmpty)
        #expect(await note.delete(written) == false && note.deleteFailure != nil)
        note.show(written)
        #expect(note.deleteFailure == nil)
        #expect(await note.delete(written) == false && note.deleteFailure != nil)
        await notesSwitch.set(false)
        time.set(TestClock.now.addingTimeInterval(60))
        await note.write(uiLanguage: "en")
        #expect(note.deleteFailure == nil)
        #expect(await note.delete(written))
    }

    @Test("another data source: the run in flight is cancelled, everything read anew")
    func replace() async throws {
        let gate = SyncStepGate()
        let time = TestTime()
        let cancels = Cancels()
        let first = mock(.aiKey, time: time, gate: gate)
        let note = noteModel(CountingCancels(base: first, cancels: cancels), time: time)
        await note.load()
        await note.estimate.settle()
        let run = Task { await note.write(uiLanguage: "en") }
        _ = await gate.held()
        let old = note.run
        note.replace(service: mock(.demo, time: time))
        #expect(note.run !== old && note.notes.isEmpty && !note.loaded && note.automaticProblem == nil)
        #expect(await eventually { await cancels.count == 1 })
        await gate.open()
        await run.value
        guard case .stopped = old.phase else {
            Issue.record("the old run should be cancelled")
            return
        }
        #expect(try await first.weeklyNotes().isEmpty)
    }

    @Test("the app's launch: Monday's note is prepared from the launch's read, and the hour is waiting")
    func appLaunch() async throws {
        let timers = ManualTimers()
        let model = AppModel(
            dataMode: .mock(.weeklyNoteMonday), strings: .app, settings: InMemorySettingsStore(language: .english),
            timing: AppModel.Timing(finishedCapsule: .seconds(60), failedCapsule: .seconds(60), mock: .instant, sleep: timers.sleep),
            calendar: TestClock.calendar, clock: { TestClock.now }, notificationCenter: NotificationCenter(),
            aiNote: true
        )
        let note = try #require(model.weeklyNote)
        await model.start()
        #expect(await eventually { finished(note)?.automatic == true })
        await timers.waitForTimer(.seconds(3600))
        note.close()
    }

    @Test("the app's model: on where it's turned on, following the data source; a scenario's Monday shows at once")
    func appModel() async throws {
        let (off, _) = makeModel(scenario: .aiKey)
        #expect(off.weeklyNote == nil)
        let model = AppModel(
            dataMode: .mock(.aiKey), strings: .app, settings: InMemorySettingsStore(language: .english),
            timing: AppModel.Timing(finishedCapsule: .seconds(60), failedCapsule: .seconds(60), mock: .instant),
            calendar: TestClock.calendar, clock: { TestClock.now }, notificationCenter: NotificationCenter(),
            aiNote: true
        )
        let note = try #require(model.weeklyNote)
        await model.useMock(.weeklyNoteMonday)
        #expect(model.weeklyNote === note)
        #expect(await eventually { finished(note)?.automatic == true })
        note.close()
    }
}

// MARK: - Words

@Suite("Weekly note text")
struct NoteTextTests {
    let text = NoteText(l10n: en, calendar: TestClock.calendar)

    func note(focus: [NoteFocus] = [], automatic: Bool = false, weekOf: String = "2026-09-21", graded: UInt32 = 0) -> WeeklyNote {
        WeeklyNote(
            meta: GenerationMeta(
                generationId: "n1", feature: .weeklyNote, backendLabel: "OpenAI", model: "gpt-6-luna", onDevice: false,
                createdAt: TestClock.now,
                usage: TokenUsage(inputTokens: 2_400, cachedInputTokens: 0, outputTokens: 420, reasoningTokens: nil),
                estCostMicroUsd: 600, estimated: false,
                context: ContextSummary(courses: [], materialsIncluded: 0, materialsTrimmed: 0, leftOut: []), promptVersion: 1
            ),
            weekOf: weekOf, text: "A calm week.", focus: focus, automatic: automatic, gradedWorkLeftOut: graded
        )
    }

    @Test("the run's headline and stages")
    func run() {
        var progress = GenerationRun<WeeklyNote>.Progress()
        #expect(text.headline(progress, automatic: false) == "Starting…")
        #expect(text.headline(progress, automatic: true) == "Preparing Monday's note")
        progress.backend = "OpenAI"
        progress.model = "gpt-6-luna"
        #expect(text.headline(progress, automatic: false) == "Writing with OpenAI · gpt-6-luna")
        #expect(text.stage(nil) == "Writing your note" && text.stage(.buildingContext) == "Gathering your week")
        #expect(NoteText(l10n: zh, calendar: TestClock.calendar).headline(progress, automatic: true) == "正在准备周一的笔记")
    }

    @Test("the week, Monday's mark, what was left out, an earlier week")
    func note() {
        #expect(text.description(note()) == "For the week of Sep 21, 2026")
        #expect(text.description(note(automatic: true)) == "For the week of Sep 21, 2026 · Prepared on Monday")
        #expect(text.regionLabel(note()) == "Weekly note for the week of Sep 21, 2026")
        #expect(text.gradedLeftOut(1) == "1 focus item was left out because it looked like an answer to graded work.")
        #expect(!text.isEarlierWeek(note(), now: TestClock.now))
        #expect(text.isEarlierWeek(note(weekOf: "2026-09-14"), now: TestClock.now))
        // Sunday still belongs to the week of the Monday before.
        #expect(!text.isEarlierWeek(note(), now: TestClock.at(2, 22)))
    }

    @Test("Copy: the note, the focus list with each course, then the AI label")
    func copy() {
        let focus = [NoteFocus(text: "Get ahead on Quiz 3.", courseId: "c1"), NoteFocus(text: "Rest.", courseId: nil)]
        let course: (String) -> String? = { $0 == "c1" ? "DEMO101" : nil }
        #expect(text.copyText(note(focus: focus), course: course) == """
        A calm week.

        Focus this week
        1. DEMO101: Get ahead on Quiz 3.
        2. Rest.

        AI-generated · OpenAI · gpt-6-luna · Sep 25, 2026 · 2,820 tokens
        """)
        #expect(text.copyText(note(), course: course) == "A calm week.\n\nAI-generated · OpenAI · gpt-6-luna · Sep 25, 2026 · 2,820 tokens")
    }

    @Test("the opt-in's cost line: on this computer by the model's facts, no price, or ≈ $x")
    func mondayCost() {
        typealias Cost = AiSettingsModel.NoteCost
        #expect(text.mondayCost(Cost(backend: "Ollama", model: "qwen3.5:9b", onDevice: true, priceKnown: true, upper: 0))
            == "With Ollama · qwen3.5:9b, on this computer.")
        // A cloud model priced at $0 is not "on this computer".
        #expect(text.mondayCost(Cost(backend: "Ollama", model: "gpt-oss:120b-cloud", onDevice: false, priceKnown: true, upper: 0))
            == "With Ollama · gpt-oss:120b-cloud: ≈ less than $0.01 each Monday, counted toward your monthly budget.")
        #expect(text.mondayCost(Cost(backend: "OpenAI", model: "x", onDevice: false, priceKnown: false, upper: nil))
            == "With OpenAI · x, counted toward your monthly budget (no price for this model).")
        #expect(text.mondayCost(Cost(backend: "OpenAI", model: "gpt-6-luna", onDevice: false, priceKnown: true, upper: 900))
            == "With OpenAI · gpt-6-luna: ≈ less than $0.01 each Monday, counted toward your monthly budget.")
        #expect(text.mondayCost(Cost(backend: "OpenAI", model: "gpt-6-luna", onDevice: false, priceKnown: true, upper: nil)) == nil)
        #expect(text.mondayCost(Cost(backend: "OpenAI", model: "gpt-6-luna", onDevice: false, priceKnown: true, upper: nil, weekEmpty: true))
            == "With OpenAI · gpt-6-luna, counted toward your monthly budget (what it costs depends on the week).")
    }
}

// MARK: - Settings ▸ AI ▸ Weekly note

@Suite("Settings: Monday's weekly note") @MainActor
struct WeeklyNoteSettingsTests {
    func loaded(_ scenario: MockScenario) async -> (AiSettingsModel, MockService) {
        let time = TestTime()
        let service = mock(scenario, time: time)
        let ai = AiSettingsModel(service: service, clock: { time.now }, calendar: TestClock.calendar)
        await ai.load()
        return (ai, service)
    }

    @Test("the opt-in: on with a provider, refused without one (and says why), paused once the model goes")
    func optIn() async throws {
        let (none, _) = await loaded(.demo)
        #expect(none.noteSettings == WeeklyNoteSettings(prepareOnMonday: false, prepareOnMondayAllowed: false))
        #expect(await none.setPrepareOnMonday(true) == false && none.noteSettingFailure?.kind == .invalid)
        let (ai, _) = await loaded(.aiKey)
        #expect(await ai.setPrepareOnMonday(true) && ai.noteSettings?.prepareOnMonday == true && ai.noteSettingFailure == nil)
        await ai.setModel(.weeklyNote, backend: nil, model: nil)
        #expect(ai.noteSettings == WeeklyNoteSettings(prepareOnMonday: true, prepareOnMondayAllowed: false))
        #expect(ai.noteCost == nil)
        // Paused, it can still be turned off.
        #expect(await ai.setPrepareOnMonday(false) && ai.noteSettings?.prepareOnMonday == false)
    }

    @Test("the cost: on this computer by the model's facts (a cloud model through Ollama isn't), else priced or not")
    func cost() async throws {
        let (local, _) = await loaded(.aiLocal)
        #expect(local.noteCost?.onDevice == true)
        await local.setModel(.weeklyNote, backend: .provider(providerId: "ollama"), model: "gpt-oss:120b-cloud")
        #expect(local.noteCost?.model == "gpt-oss:120b-cloud" && local.noteCost?.onDevice == false)
        let (key, _) = await loaded(.aiKey)
        let priced = try #require(key.noteCost)
        #expect(!priced.onDevice && priced.priceKnown && priced.upper == 900)
        await key.setModel(.weeklyNote, backend: .provider(providerId: "openai"), model: "gpt-6-preview-0929")
        #expect(key.noteCost?.priceKnown == false)
        let words = NoteText(l10n: en, calendar: TestClock.calendar)
        #expect(key.noteCost.flatMap(words.mondayCost) == "With OpenAI · gpt-6-preview-0929, counted toward your monthly budget (no price for this model).")
        // A week with nothing to write about: no amount, and no "no price" for a priced model.
        let (empty, service) = await loaded(.empty)
        _ = try await service.addModelProvider(preset: "openai", baseUrl: nil, apiKey: "sk-demo-key-7731")
        await empty.setModel(.weeklyNote, backend: .provider(providerId: "openai"), model: "gpt-5.4-mini")
        await empty.load()
        let cost = try #require(empty.noteCost)
        #expect(cost.priceKnown && cost.upper == nil && cost.weekEmpty)
        #expect(words.mondayCost(cost) == "With OpenAI · gpt-5.4-mini, counted toward your monthly budget (what it costs depends on the week).")
        // An unpriced model still says so first.
        await empty.setModel(.weeklyNote, backend: .provider(providerId: "openai"), model: "gpt-6-preview-0929")
        #expect(empty.noteCost.flatMap(words.mondayCost) == "With OpenAI · gpt-6-preview-0929, counted toward your monthly budget (no price for this model).")
    }
}
