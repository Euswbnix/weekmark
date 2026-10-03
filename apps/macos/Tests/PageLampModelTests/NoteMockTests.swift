// Weekly notes in the mock, as the facade writes them: the events, the note from the week's
// structure, the last 5 kept, Delete, the gate (Monday's run never goes over the budget), the
// opt-in, the Monday rule with its one try a day (recorded as the run starts), Stop, and what
// removing AI data or a course's generated content takes with it.

import Foundation
import PageLamp
import PageLampKit
@testable import PageLampModel
import Synchronization
import Testing

/// A clock the test moves.
private final class TestTime: Sendable {
    private let value: Mutex<Date>
    init(_ date: Date = TestClock.now) { value = Mutex(date) }
    var now: Date { value.withLock { $0 } }
    func set(_ date: Date) { value.withLock { $0 = date } }
}

/// Monday 2026-09-28 at `hour` (Toronto), the Monday after the tests' Friday.
private func monday(_ hour: Int = 10, plusWeeks weeks: Int = 0) -> Date {
    TestClock.at(3 + 7 * weeks, hour)
}

private func mock(_ scenario: MockScenario, time: TestTime = TestTime(), gate: SyncStepGate? = nil) -> MockService {
    MockService(
        scenario: scenario, timing: MockService.Timing(latency: .zero, syncStep: .zero, gate: gate),
        calendar: TestClock.calendar, now: { time.now }
    )
}

private func options(automatic: Bool = false, overrideBudget: Bool = false) -> WeeklyNoteOptions {
    WeeklyNoteOptions(uiLanguage: "en", overrideBudget: overrideBudget, automatic: automatic)
}

/// The events of one run, by name.
private func names(_ stream: GenEventStream) async -> [String] {
    stream.finish()
    var all: [String] = []
    for await event in stream.events {
        let name = switch event {
        case .started(let id, let backend, let model, _): "started \(id) \(backend) \(model)"
        case .stage(let stage): AiCodes.name(stage)
        case .context(let summary, let tokens): "context \(summary.courses.count) \(tokens ?? 0)"
        case .usage: "usage"
        case .finished(let ok): "finished \(ok)"
        default: "other"
        }
        all.append(name)
    }
    return all
}

private func due(_ service: MockService, at date: Date) async throws -> Bool {
    try await service.startupTasks(now: date).prepareWeeklyNote
}

@discardableResult
private func write(
    _ service: MockService, _ id: String, automatic: Bool = false, overrideBudget: Bool = false
) async throws(PageLampFailure) -> WeeklyNote {
    try await service.writeWeeklyNote(
        generationId: id, options: options(automatic: automatic, overrideBudget: overrideBudget), observer: GenEventStream()
    )
}

private func failure(_ body: () async throws(PageLampFailure) -> Void) async -> PageLampFailure? {
    do {
        try await body()
        return nil
    } catch {
        return error
    }
}

/// How a run in a task ended, if it failed.
private func failure<T>(of task: Task<T, any Error>) async -> PageLampFailure? {
    do {
        _ = try await task.value
        return nil
    } catch {
        return error as? PageLampFailure
    }
}

@Suite("Weekly notes in the mock")
struct NoteMockTests {
    @Test("a note: the events in order, from the week's structure, never a material's text")
    func note() async throws {
        let time = TestTime(TestClock.at(4, 9))
        let service = mock(.aiKey, time: time)
        let stream = GenEventStream()
        let note = try await service.writeWeeklyNote(generationId: "n1", options: options(), observer: stream)
        #expect(await names(stream) == [
            "building_context", "context 3 2400", "started n1 OpenAI gpt-6-luna", "waiting_for_model", "usage",
            "validating", "finished true",
        ])
        #expect(note.meta.generationId == "n1" && note.meta.feature == .weeklyNote && !note.automatic)
        #expect(note.meta.createdAt == time.now && note.meta.estCostMicroUsd == 600)
        #expect(note.weekOf == "2026-09-28" && note.gradedWorkLeftOut == 0)
        // Hidden courses stay out; each course keeps its own AI state (No AI is structure only).
        let context = note.meta.context
        #expect(context.materialsIncluded == 0 && context.courses.allSatisfy { !$0.textIncluded })
        #expect(context.courses.map { $0.state } == [.readable, .readable, .withheldByPolicy])
        #expect(note.focus.count <= 3 && !note.focus.isEmpty && note.focus.allSatisfy { $0.courseId != nil })
        #expect(try await service.weeklyNotes().map { $0.meta.generationId } == ["n1"])
    }

    @Test("the week of a note is its Monday, whichever day the calendar's weeks start on")
    func weekOf() async throws {
        let time = TestTime(TestClock.at(2, 22))
        let service = mock(.aiKey, time: time)
        // Sunday 2026-09-27: the week of Monday the 21st.
        #expect(try await write(service, "sun").weekOf == "2026-09-21")
        time.set(monday(0))
        #expect(try await write(service, "mon").weekOf == "2026-09-28")
    }

    @Test("the last 5 are kept, newest first; Delete takes one, and then it isn't there")
    func kept() async throws {
        let time = TestTime()
        let service = mock(.aiKey, time: time)
        for index in 1...6 {
            time.set(TestClock.now.addingTimeInterval(Double(index) * 60))
            try await write(service, "n\(index)")
        }
        #expect(try await service.weeklyNotes().map { $0.meta.generationId } == ["n6", "n5", "n4", "n3", "n2"])
        try await service.deleteWeeklyNote(generationId: "n4")
        #expect(try await service.weeklyNotes().map { $0.meta.generationId } == ["n6", "n5", "n3", "n2"])
        let again = await failure { () async throws(PageLampFailure) in try await service.deleteWeeklyNote(generationId: "n4") }
        #expect(again?.kind == .notFound && again?.message == "There is no weekly note n4.")
    }

    @Test("refusals send nothing: no model, then nothing to write about, then the gate")
    func refusals() async throws {
        // No model: blocked, before anything else is looked at.
        let stream = GenEventStream()
        let noModel = await failure { () async throws(PageLampFailure) in
            _ = try await mock(.demo).writeWeeklyNote(generationId: "n1", options: options(), observer: stream)
        }
        #expect(noModel?.kind == .blocked && noModel?.blocked == .noModelChosen)
        #expect(await names(stream).isEmpty)
        // No course, deadline or plan: nothing to write about (once a model is set up).
        let empty = mock(.empty)
        _ = try await empty.addModelProvider(preset: "ollama", baseUrl: nil, apiKey: nil)
        try await empty.setFeatureModel(
            feature: .weeklyNote, choice: ModelChoice(backend: .provider(providerId: "ollama"), model: "qwen3.5:9b", effort: .lowest)
        )
        let nothing = await failure { () async throws(PageLampFailure) in try await write(empty, "n1") }
        #expect(nothing?.kind == .blocked && nothing?.blocked == .nothingToWrite)
        #expect(try await empty.weeklyNotes().isEmpty)
        // The gate: a changed disclosure; the budget, which a click may go over.
        let changed = await failure { () async throws(PageLampFailure) in try await write(mock(.aiDisclosureChanged), "n1") }
        #expect(changed?.blocked == .disclosureNotAcknowledged)
        let budget = mock(.aiKey)
        try await budget.setMonthlyBudget(microUsd: 1)
        let over = await failure { () async throws(PageLampFailure) in try await write(budget, "n1") }
        #expect(over?.blocked == .budgetReached)
        #expect(try await write(budget, "n2", overrideBudget: true).meta.generationId == "n2")
    }

    @Test("removed courses take their deadlines and plan items with them: then nothing to write about")
    func removedCourses() async throws {
        let service = mock(.aiKey)
        _ = try await service.removeCourses(
            courses: ["DEMO101", "DEMO205", "DEMO310"],
            options: RemoveOptions(reason: nil, keepDownloadedFiles: false, purgeNow: false, deletePreUpdateBackup: false)
        )
        let refused = await failure { () async throws(PageLampFailure) in try await write(service, "n1") }
        #expect(refused?.kind == .blocked && refused?.blocked == .nothingToWrite)
    }

    @Test("a week with nothing to write about isn't due on Monday, and keeps that Monday's try")
    func mondayNothingToWrite() async throws {
        let time = TestTime(monday())
        let service = mock(.aiKey, time: time)
        _ = try await service.setPrepareWeeklyNoteOnMonday(on: true)
        let report = try await service.removeCourses(
            courses: ["DEMO101", "DEMO205", "DEMO310"],
            options: RemoveOptions(reason: nil, keepDownloadedFiles: false, purgeNow: false, deletePreUpdateBackup: false)
        )
        #expect(try await !due(service, at: monday()))
        let refused = await failure { () async throws(PageLampFailure) in try await write(service, "auto-1", automatic: true) }
        #expect(refused?.kind == .invalid)
        // A course back later that Monday: due, with its try unused.
        let removed = try #require(report.removed.first)
        _ = try await service.restoreCourse(removedId: removed.removedId)
        #expect(try await due(service, at: monday(12)))
        #expect(try await write(service, "auto-2", automatic: true).automatic)
    }

    @Test("the opt-in: only a provider may prepare it; kept, and paused, when the model goes")
    func optIn() async throws {
        let none = mock(.demo)
        #expect(try await none.weeklyNoteSettings() == WeeklyNoteSettings(prepareOnMonday: false, prepareOnMondayAllowed: false))
        let refused = await failure { () async throws(PageLampFailure) in _ = try await none.setPrepareWeeklyNoteOnMonday(on: true) }
        #expect(refused?.kind == .invalid)
        let local = mock(.aiLocal)
        #expect(try await local.setPrepareWeeklyNoteOnMonday(on: true).prepareOnMondayAllowed)
        try await local.removeModelProvider(providerId: "ollama")
        #expect(try await local.weeklyNoteSettings() == WeeklyNoteSettings(prepareOnMonday: true, prepareOnMondayAllowed: false))
        #expect(try await !due(local, at: monday()))
        #expect(try await local.setPrepareWeeklyNoteOnMonday(on: false).prepareOnMonday == false)
    }

    @Test("due on a Monday with the opt-in, until a note is written or tried that day")
    func mondayRule() async throws {
        let time = TestTime(monday())
        let service = mock(.aiKey, time: time)
        #expect(try await !due(service, at: monday()))
        _ = try await service.setPrepareWeeklyNoteOnMonday(on: true)
        #expect(try await due(service, at: monday()))
        #expect(try await !due(service, at: monday().addingTimeInterval(86_400)))
        // A note the student wrote that day counts; deleting it makes Monday due again.
        try await write(service, "click")
        #expect(try await !due(service, at: monday(23)))
        try await service.deleteWeeklyNote(generationId: "click")
        #expect(try await due(service, at: monday(23)))
        // Monday's run: once; a second one that day is refused, a click still works.
        let note = try await write(service, "auto-1", automatic: true)
        #expect(note.automatic)
        #expect(try await !due(service, at: monday(12)))
        let second = await failure { () async throws(PageLampFailure) in try await write(service, "auto-2", automatic: true) }
        #expect(second?.kind == .invalid)
        #expect(try await !write(service, "click-2").automatic)
        // The next Monday is due again.
        #expect(try await due(service, at: monday(plusWeeks: 1)))
    }

    @Test("the try is recorded as Monday's run starts: a held run blocks a second one; a stopped one uses up the day")
    func tryAtStart() async throws {
        let gate = SyncStepGate()
        let time = TestTime(monday())
        let service = mock(.aiKey, time: time, gate: gate)
        _ = try await service.setPrepareWeeklyNoteOnMonday(on: true)
        let first = Task { try await write(service, "auto-1", automatic: true) }
        #expect(await gate.held() == SyncStepPosition(sourceId: "auto-1", step: 0))
        #expect(try await !due(service, at: monday(11)))
        let second = await failure { () async throws(PageLampFailure) in try await write(service, "auto-2", automatic: true) }
        #expect(second?.kind == .invalid)
        let stream = GenEventStream()
        let same = await failure { () async throws(PageLampFailure) in
            _ = try await service.writeWeeklyNote(generationId: "auto-1", options: options(automatic: true), observer: stream)
        }
        #expect(same?.kind == .busy)
        #expect(await names(stream).isEmpty)
        try await service.cancelGeneration(generationId: "auto-1")
        await gate.open()
        #expect(await failure(of: first)?.kind == .cancelled)
        #expect(try await service.weeklyNotes().isEmpty)
        #expect(try await !due(service, at: monday(22)))
    }

    @Test("Monday's run never goes over the budget, and the blocked try still counts")
    func automaticBudget() async throws {
        let service = mock(.aiKey, time: TestTime(monday()))
        _ = try await service.setPrepareWeeklyNoteOnMonday(on: true)
        try await service.setMonthlyBudget(microUsd: 1)
        let blocked = await failure { () async throws(PageLampFailure) in
            try await write(service, "auto-1", automatic: true, overrideBudget: true)
        }
        #expect(blocked?.blocked == .budgetReached)
        #expect(try await !due(service, at: monday(11)))
    }

    @Test("Stop at each step: cancelled, nothing kept; a stopped click leaves Monday due", arguments: [0, 1, 2] as [UInt32])
    func stop(at step: UInt32) async throws {
        let gate = SyncStepGate()
        let service = mock(.aiKey, time: TestTime(monday()), gate: gate)
        _ = try await service.setPrepareWeeklyNoteOnMonday(on: true)
        let stream = GenEventStream()
        let run = Task { try await service.writeWeeklyNote(generationId: "n1", options: options(), observer: stream) }
        await gate.advance(until: SyncStepPosition(sourceId: "n1", step: step))
        try await service.cancelGeneration(generationId: "n1")
        await gate.open()
        #expect(await failure(of: run)?.kind == .cancelled)
        #expect(await names(stream).last == "finished false")
        #expect(try await service.weeklyNotes().isEmpty)
        #expect(try await due(service, at: monday(11)))
        // The same id can run again.
        #expect(try await write(service, "n1").meta.generationId == "n1")
    }

    @Test("Monday in the calendar's zone, across the end of daylight saving time")
    func daylightSaving() async throws {
        let time = TestTime()
        let service = mock(.aiKey, time: time)
        _ = try await service.setPrepareWeeklyNoteOnMonday(on: true)
        // Toronto falls back on Sunday 2026-11-01: Monday the 2nd starts at 05:00 UTC.
        let utc = { (day: Int, hour: Int, minute: Int) -> Date in
            var components = DateComponents(year: 2026, month: 11, day: day, hour: hour, minute: minute)
            components.timeZone = .gmt
            return Calendar(identifier: .gregorian).date(from: components) ?? .distantPast
        }
        #expect(try await !due(service, at: utc(2, 4, 30)))
        #expect(try await due(service, at: utc(2, 5, 30)))
        time.set(utc(2, 4, 30))
        let early = await failure { () async throws(PageLampFailure) in try await write(service, "auto-1", automatic: true) }
        #expect(early?.kind == .invalid)
        time.set(utc(2, 5, 30))
        #expect(try await write(service, "auto-2", automatic: true).weekOf == "2026-11-02")
    }

    @Test("a course's generated content takes the notes that cover it; all of it, every note")
    func deleteGenerated() async throws {
        let service = mock(.aiKey)
        try await write(service, "n1")
        try await write(service, "n2")
        #expect(try await service.deleteGenerated(course: "DEMO099") == 0)
        #expect(try await service.deleteGenerated(course: "DEMO101") == 2)
        try await write(service, "n3")
        #expect(try await service.deleteGenerated(course: nil) == 1)
        #expect(try await service.weeklyNotes().isEmpty)
    }

    @Test("removing all AI data takes the notes, the opt-in and Monday's try")
    func removeAll() async throws {
        let service = mock(.aiKey, time: TestTime(monday()))
        _ = try await service.setPrepareWeeklyNoteOnMonday(on: true)
        try await write(service, "auto-1", automatic: true)
        #expect(try await service.removeAllAiData().generationsRemoved == 1)
        #expect(try await service.weeklyNotes().isEmpty)
        #expect(try await service.weeklyNoteSettings().prepareOnMonday == false)
        // Set up again the same Monday: due again (the try went with the rest).
        _ = try await service.addModelProvider(preset: "ollama", baseUrl: nil, apiKey: nil)
        try await service.setFeatureModel(
            feature: .weeklyNote, choice: ModelChoice(backend: .provider(providerId: "ollama"), model: "qwen3.5:9b", effort: .lowest)
        )
        _ = try await service.setPrepareWeeklyNoteOnMonday(on: true)
        #expect(try await due(service, at: monday(11)))
    }

    @Test("the Monday scenario: opted in, due any day, prepared once")
    func mondayScenario() async throws {
        let service = mock(.weeklyNoteMonday)
        #expect(try await service.weeklyNoteSettings() == WeeklyNoteSettings(prepareOnMonday: true, prepareOnMondayAllowed: true))
        // The tests' Friday counts as a Monday here.
        #expect(try await due(service, at: TestClock.now))
        #expect(try await write(service, "auto-1", automatic: true).automatic)
        #expect(try await !due(service, at: TestClock.now))
    }

    @Test("\"≈ $x\" for a note says when there's nothing to write about, with no amount")
    func estimate() async throws {
        #expect(try await mock(.aiKey).estimateGeneration(request: .weeklyNote).microUsdUpper == 900)
        let empty = mock(.empty)
        _ = try await empty.addModelProvider(preset: "ollama", baseUrl: nil, apiKey: nil)
        try await empty.setFeatureModel(
            feature: .weeklyNote, choice: ModelChoice(backend: .provider(providerId: "ollama"), model: "qwen3.5:9b", effort: .lowest)
        )
        let blocked = try await empty.estimateGeneration(request: .weeklyNote)
        #expect(blocked.wouldBlock == .nothingToWrite && blocked.microUsdUpper == nil && blocked.inputTokens == 0)
        // No model chosen comes first.
        #expect(try await mock(.empty).estimateGeneration(request: .weeklyNote).wouldBlock == .noModelChosen)
    }
}
