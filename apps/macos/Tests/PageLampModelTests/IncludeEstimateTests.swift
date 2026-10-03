// "≈ $x" for Include It and Write Again: the mock prices an explanation's include only where its
// run lifts it (a graded-looking material of that week it reads; DEMO101's Assignment 4 is the
// week's second readable material once included); an unknown id, a material read anyway or one
// left over the length limit adds nothing. The Explain section runs Include It and
// Write Again from estimates of exactly what they send.

import Foundation
import PageLamp
import PageLampKit
@testable import PageLampModel
import Testing

private func mock(_ scenario: MockScenario, gate: SyncStepGate? = nil) -> MockService {
    MockService(
        scenario: scenario, timing: MockService.Timing(latency: .zero, syncStep: .zero, gate: gate),
        calendar: TestClock.calendar, now: { TestClock.now }
    )
}

/// DEMO101's week 4 materials by title.
private func week4(_ service: MockService) async throws -> [String: MaterialView] {
    let materials = try await service.weekMaterials(course: "DEMO101", week: 4).materials
    return Dictionary(uniqueKeysWithValues: materials.map { ($0.title, $0) })
}

private func material(_ id: String, _ title: String) -> MaterialView {
    MaterialView(
        id: id, courseId: "c", title: title, kind: .file, moduleId: nil, moduleName: nil, weekHint: 4,
        publishedAt: nil, url: nil, textStatus: .ok, textError: nil, downloadBlocked: nil, chunkCount: 3
    )
}

/// The options of every explanation run started.
private actor SentOptions {
    private(set) var all: [ExplainOptions] = []
    func record(_ options: ExplainOptions) { all.append(options) }
}

private struct RecordingExplain: ForwardingService {
    let base: any PageLampService
    let sent: SentOptions

    func explainWeek(
        course: String, week: UInt32?, generationId: String, options: ExplainOptions, observer: any GenObserver
    ) async throws(PageLampFailure) -> WeeklyExplanation {
        await sent.record(options)
        return try await base.explainWeek(course: course, week: week, generationId: generationId, options: options, observer: observer)
    }
}

@Suite("Include priced in the mock")
struct IncludePricingTests {
    @Test("only what the run lifts is priced: an unknown id or a material read anyway adds nothing")
    func nothingLifted() async throws {
        let service = mock(.aiKey)
        let week = try await week4(service)
        let base = try await service.estimateGeneration(request: .weeklyExplanation(course: "DEMO101", week: 4))
        let slides = try #require(week["Week 4 slides — Sampling and Surveys"])
        let practice = try #require(week["Week 4 practice questions"])
        // An id that names nothing; the slides (read anyway); the practice questions (not graded
        // work: nothing to lift).
        for include in [["nope"], [slides.id], [practice.id]] {
            let priced = try await service.estimateGeneration(
                request: .weeklyExplanation(course: "DEMO101", week: 4, include: include)
            )
            #expect(priced.inputTokens == base.inputTokens && priced.microUsdUpper == base.microUsdUpper, "include \(include)")
        }
    }

    @Test("the assignment included is read, so priced: one more material's worth, unknown ids beside it add nothing")
    func assignmentLifted() async throws {
        let service = mock(.aiKey)
        let assignment = try #require(try await week4(service)["Assignment 4 — Survey Simulation"])
        let base = try await service.estimateGeneration(request: .weeklyExplanation(course: "DEMO101", week: 4))
        let priced = try await service.estimateGeneration(
            request: .weeklyExplanation(course: "DEMO101", week: 4, include: [assignment.id, "nope"])
        )
        #expect(priced.inputTokens == base.inputTokens + 15_000)
        #expect((priced.microUsdUpper ?? 0) > (base.microUsdUpper ?? 0))
    }

    @Test("the run is priced as its estimate: a budget between the two stops only the run with the include")
    func runPricedLikeEstimate() async throws {
        let service = mock(.aiKey)
        let assignment = try #require(try await week4(service)["Assignment 4 — Survey Simulation"])
        let plain = EstimateRequest.weeklyExplanation(course: "DEMO101", week: 4)
        let included = EstimateRequest.weeklyExplanation(course: "DEMO101", week: 4, include: [assignment.id])
        let base = try #require(try await service.estimateGeneration(request: plain).microUsdUpper)
        let more = try #require(try await service.estimateGeneration(request: included).microUsdUpper)
        let spent = try await service.aiStatus().budget.spentMicroUsd
        try await service.setMonthlyBudget(microUsd: spent + (base + more) / 2)
        #expect(try await service.estimateGeneration(request: included).wouldBlock == .budgetReached)
        #expect(try await service.estimateGeneration(request: plain).wouldBlock == nil)
        do {
            _ = try await service.explainWeek(
                course: "DEMO101", week: 4, generationId: "ex-included", options: ExplainOptions(include: [assignment.id]),
                observer: GenEventStream()
            )
            Issue.record("expected the run with the include to be over the budget")
        } catch {
            #expect(error.kind == .blocked && error.blocked == .budgetReached)
        }
        let written = try await service.explainWeek(
            course: "DEMO101", week: 4, generationId: "ex-plain", options: ExplainOptions(), observer: GenEventStream()
        )
        #expect(written.meta.generationId == "ex-plain")
    }

    @Test("lifted: a graded-looking material read; one left over the length limit isn't, nor is study material")
    func lifted() {
        let test = material("t", "Midterm test")
        let notes = material("n", "Week 4 notes")
        let slides = material("s", "Week 4 slides")
        #expect(MockService.liftedIncludes([test, notes], ["t", "n", "nope"]) == ["t"])
        // Included but third in the week: the two materials that fit are read, the test is over
        // the limit (so not among what's lifted).
        let (read, leftOut) = MockService.explanationSelection([notes, slides, test], include: ["t"])
        #expect(read.map { $0.id } == ["n", "s"] && leftOut.map { $0.reason } == [.overBudget])
        let request = EstimateRequest.weeklyExplanation(course: "c", week: 4, include: ["t"])
        #expect(MockAiFixtures.workload(request, lifted: 0).input == 45_000)
        #expect(MockAiFixtures.workload(request, lifted: 2).input == 75_000)
    }
}

@Suite("Include It and Write Again") @MainActor
struct IncludeEstimateModelTests {
    func model(_ service: any PageLampService, newId: @escaping @Sendable () -> String = randomGenerationId) async -> ExplainModel {
        let explain = ExplainModel(courseId: "DEMO101", service: service, debounce: .zero, newId: newId)
        await explain.load(week: 4)
        await explain.estimate.settle()
        return explain
    }

    @Test("Include It is priced with what it sends and sends it; Write Again sends the same include again")
    func includeThenAgain() async throws {
        let base = mock(.aiKey)
        let sent = SentOptions()
        let assignment = try #require(try await week4(base)["Assignment 4 — Survey Simulation"])
        let explain = await model(RecordingExplain(base: base, sent: sent))
        await explain.generate(week: 4, uiLanguage: "en")
        let first = try #require(explain.shown(week: 4))
        #expect(explain.includeIds(first) == [assignment.id] && explain.againIds(first).isEmpty)
        explain.prepare(for: first, week: 4)
        await explain.includeEstimate.settle()
        #expect(explain.includeEstimate.request == .weeklyExplanation(course: "DEMO101", week: 4, include: [assignment.id]))
        #expect(explain.againEstimate.request == nil && explain.againEstimate(for: first) === explain.estimate)
        #expect(explain.includeEstimate.canGenerate)
        await explain.includeAndWriteAgain(first, week: 4, uiLanguage: "en")
        let included = try #require(explain.shown(week: 4))
        let afterInclude = await sent.all.map { $0.include }
        #expect(afterInclude == [[], [assignment.id]])
        // Its Write Again: the same include, from its own "≈ $x".
        #expect(explain.againIds(included) == [assignment.id] && explain.againEstimate(for: included) === explain.againEstimate)
        explain.prepare(for: included, week: 4)
        await explain.againEstimate.settle()
        #expect(explain.againEstimate.request == .weeklyExplanation(course: "DEMO101", week: 4, include: [assignment.id]))
        await explain.writeAgain(included, week: 4, uiLanguage: "en")
        let afterAgain = await sent.all.map { $0.include }
        #expect(afterAgain == [[], [assignment.id], [assignment.id]])
        // An explanation written without include: Write Again sends none, from the week's "≈ $x".
        await explain.writeAgain(first, week: 4, uiLanguage: "en")
        let last = await sent.all.last
        #expect(last?.include == [])
    }

    @Test("Include It adds to what the explanation's run included")
    func addsToSent() async throws {
        let base = mock(.aiKey)
        let assignment = try #require(try await week4(base)["Assignment 4 — Survey Simulation"])
        let explain = await model(base, newId: { "ex-1" })
        explain.prepare(for: nil, week: 4)
        #expect(explain.includeEstimate.request == nil && explain.againEstimate.request == nil)
        await explain.generate(week: 4, uiLanguage: "en")
        let first = try #require(explain.shown(week: 4))
        // A run here that included the assignment (ex-1 again), and another left-out material the
        // facade would bring back.
        explain.prepare(for: first, week: 4)
        await explain.includeEstimate.settle()
        await explain.includeAndWriteAgain(first, week: 4, uiLanguage: "en")
        let written = try #require(explain.shown(week: 4))
        let other = LeftOutMaterial(materialId: "other", title: "Quiz 4", reason: .looksLikeAssessment, includable: true)
        let later = WeeklyExplanation(
            meta: written.meta, courseId: written.courseId, week: written.week, sections: written.sections,
            checkQuestions: written.checkQuestions, leftOut: written.leftOut + [other], stale: false,
            sharingReminder: false, droppedCitations: 0, citeAiUse: false
        )
        #expect(explain.includeIds(later) == [assignment.id, "other"])
    }

    @Test("\"not allowed\" for sharing prices Include It again at once: blocked, not left enabled")
    func sharingRefusedBlocksInclude() async throws {
        let explain = await model(mock(.aiKey))
        await explain.generate(week: 4, uiLanguage: "en")
        let first = try #require(explain.shown(week: 4))
        explain.prepare(for: first, week: 4)
        await explain.includeEstimate.settle()
        #expect(explain.includeEstimate.canGenerate)
        #expect(await explain.answerSharing(.notAllowed, for: first))
        #expect(explain.includeEstimate.block == .materialSharingNotAllowed && !explain.includeEstimate.canGenerate)
    }

    @Test("a run takes every line's tick with it: none rides on a later run")
    func ticksGo() async throws {
        let base = mock(.aiBudget)
        try await base.setMonthlyBudget(microUsd: 4_962_000)
        let explain = await model(base)
        explain.estimate.overrideBudget = true
        await explain.generate(week: 4, uiLanguage: "en")
        let first = try #require(explain.shown(week: 4))
        explain.prepare(for: first, week: 4)
        await explain.includeEstimate.settle()
        // Both lines ticked; Include It runs.
        explain.estimate.overrideBudget = true
        explain.includeEstimate.overrideBudget = true
        await explain.includeAndWriteAgain(first, week: 4, uiLanguage: "en")
        #expect(explain.estimate.block == .budgetReached)
        #expect(!explain.estimate.overrideBudget && !explain.includeEstimate.overrideBudget && !explain.againEstimate.overrideBudget)
    }

    @Test("Include It over the budget: off until its own tick, for that run only")
    func overBudget() async throws {
        let base = mock(.aiBudget)
        try await base.setMonthlyBudget(microUsd: 4_962_000)
        let sent = SentOptions()
        let explain = await model(RecordingExplain(base: base, sent: sent))
        explain.estimate.overrideBudget = true
        await explain.generate(week: 4, uiLanguage: "en")
        let first = try #require(explain.shown(week: 4))
        explain.prepare(for: first, week: 4)
        await explain.includeEstimate.settle()
        #expect(explain.includeEstimate.block == .budgetReached && !explain.includeEstimate.canGenerate)
        await explain.includeAndWriteAgain(first, week: 4, uiLanguage: "en")
        let blocked = await sent.all
        #expect(blocked.count == 1)
        explain.includeEstimate.overrideBudget = true
        await explain.includeAndWriteAgain(first, week: 4, uiLanguage: "en")
        let over = await sent.all
        #expect(over.count == 2 && over.last?.overrideBudget == true)
        #expect(!explain.includeEstimate.overrideBudget)
    }
}
