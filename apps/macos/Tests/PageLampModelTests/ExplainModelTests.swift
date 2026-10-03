// A course's Explain section: its words (in both languages), the Markdown subset, which sections
// the picker offers, and the section's model on the mock: the week, the run and its end, what's
// shown, Delete, the sharing question, and stopping a run the course no longer allows.

import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Testing

private let en = L10n(locale: Locale(identifier: "en_US"), table: .app)
private let zh = L10n(locale: Locale(identifier: "zh-Hans_CN"), table: .app)

private func mock(_ scenario: MockScenario, gate: SyncStepGate? = nil) -> MockService {
    MockService(
        scenario: scenario, timing: MockService.Timing(latency: .zero, syncStep: .zero, gate: gate),
        calendar: TestClock.calendar, now: { TestClock.now }
    )
}

/// Counts cancels.
private actor Cancels {
    private(set) var count = 0

    func record() {
        count += 1
    }
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

/// ICU's narrow and no-break spaces ("10:05\u{202F}AM") as plain spaces.
private func plain(_ value: String) -> String {
    value.replacingOccurrences(of: "\u{202F}", with: " ").replacingOccurrences(of: "\u{00A0}", with: " ")
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

private func meta(created: Date = TestClock.now) -> GenerationMeta {
    GenerationMeta(
        generationId: "ex-1", feature: .weeklyExplanation, backendLabel: "OpenAI", model: "gpt-5.4-mini", onDevice: false,
        createdAt: created,
        usage: TokenUsage(inputTokens: 12_800, cachedInputTokens: 0, outputTokens: 1_100, reasoningTokens: nil),
        estCostMicroUsd: 3_100, estimated: false,
        context: ContextSummary(courses: [], materialsIncluded: 2, materialsTrimmed: 0, leftOut: []), promptVersion: 1
    )
}

// MARK: - Words

@Suite("Explain text")
struct ExplainTextTests {
    let text = ExplainText(l10n: en, calendar: TestClock.calendar)

    @Test("the week, Generate, the run's headline, materials and stages")
    func run() {
        #expect(text.weekOption(4, current: 4) == "Week 4 (this week)")
        #expect(text.weekOption(3, current: 4) == "Week 3")
        #expect(text.generate(4) == "Explain Week 4" && text.generate(nil) == "Explain the Recent Materials")
        var progress = GenerationRun<WeeklyExplanation>.Progress()
        #expect(text.headline(progress) == "Starting…")
        progress.backend = "OpenAI"
        progress.model = "gpt-5.4-mini"
        #expect(text.headline(progress) == "Writing with OpenAI · gpt-5.4-mini")
        #expect(text.reading(1) == "Reading 1 material" && text.reading(2) == "Reading 2 materials")
        #expect(text.stage(nil) == "Writing the explanation" && text.stage(.validating) == "Checking the citations")
        let zhText = ExplainText(l10n: zh, calendar: TestClock.calendar)
        #expect(zhText.generate(4) == "讲解第 4 周" && zhText.weekOption(4, current: 4) == "第 4 周（本周）")
    }

    @Test("the result's labels, sources, reasons and Include")
    func result() {
        #expect(text.regionLabel(4) == "Explanation of week 4" && text.regionLabel(nil) == "Explanation of the recent materials")
        let cited = Citation(handle: "M1", materialId: "m", title: "Week 4 slides", locator: "p. 2", url: nil)
        let bare = Citation(handle: "M1", materialId: "m", title: "Week 4 slides", locator: nil, url: nil)
        #expect(text.citation(cited) == "Week 4 slides, p. 2" && text.citation(bare) == "Week 4 slides")
        #expect(ExplainText(l10n: zh, calendar: TestClock.calendar).citation(cited) == "Week 4 slides，p. 2")
        for reason in [LeftOutReason.looksLikeAssessment, .externalLink, .noText, .overBudget] {
            #expect(en.has("explain.result.leftOutReason.\(AiCodes.name(reason))"))
        }
        #expect(text.reason(.looksLikeAssessment) == "looks like graded work")
        #expect(text.include(1) == "Not Graded Work? Include It and Write Again")
        #expect(text.include(2) == "Not Graded Work? Include These 2 and Write Again")
        #expect(text.droppedCitations(1) == "1 citation the model gave didn't match your materials and was removed.")
        #expect(en("mac.explain.reason", ["reason": "no readable text"]) == " (no readable text)")
        #expect(zh("mac.explain.reason", ["reason": "x"]) == "（x）")
    }

    @Test("the history's time and its button for VoiceOver")
    func history() throws {
        let date = try #require(TestClock.calendar.date(from: DateComponents(year: 2026, month: 9, day: 25, hour: 10, minute: 5)))
        #expect(plain(text.savedAt(date)) == "Fri, Sep 25 at 10:05 AM")
        #expect(plain(text.showFrom(date)) == "Show the explanation from Fri, Sep 25 at 10:05 AM")
    }

    @Test("Copy: headings, paragraphs with their sources, the questions, then the AI label")
    func copy() {
        let explanation = WeeklyExplanation(
            meta: meta(), courseId: "c", week: 4,
            sections: [
                ExplanationSection(heading: "Sampling", paragraphs: [
                    ExplanationParagraph(text: "The **key** idea.", citations: [
                        Citation(handle: "M1", materialId: "a", title: "Week 4 slides", locator: "p. 2", url: nil),
                        Citation(handle: "M2", materialId: "b", title: "Reading", locator: nil, url: nil),
                    ]),
                    ExplanationParagraph(text: "No source.", citations: []),
                ]),
            ],
            checkQuestions: ["What is a frame?", "Why sample?"], leftOut: [], stale: false, sharingReminder: false,
            droppedCitations: 0, citeAiUse: false
        )
        #expect(text.copyText(explanation) == """
        Sampling

        The **key** idea. [Week 4 slides, p. 2; Reading]

        No source.

        1. What is a frame?
        2. Why sample?

        AI-generated · OpenAI · gpt-5.4-mini · Sep 25, 2026 · 13,900 tokens
        """)
    }
}

@Suite("Inline Markdown")
struct InlineMarkdownTests {
    @Test("strong, emphasis and code, and nothing else (the Tauri app's cases)")
    func runs() {
        #expect(InlineMarkdown.runs("A **key** idea, *in short*, is `f(x)`.") == [
            .init("A ", .plain), .init("key", .strong), .init(" idea, ", .plain), .init("in short", .emphasis),
            .init(", is ", .plain), .init("f(x)", .code), .init(".", .plain),
        ])
        // Markup from a model stays text; links and images are not rendered.
        let markup = #"<img src=x onerror="alert(1)"> [a](https://x.test)"#
        #expect(InlineMarkdown.runs(markup) == [.init(markup, .plain)])
        #expect(InlineMarkdown.runs("2 * 3 * 4") == [.init("2 * 3 * 4", .plain)])
        #expect(InlineMarkdown.runs("").isEmpty)
    }
}

@Suite("Course sections")
struct CourseSectionsTests {
    @Test("Explain is the fourth section where it's on; elsewhere it falls back to This Week")
    func shown() {
        #expect(CourseSection.shown(explain: true) == [.week, .deadlines, .timeline, .explain])
        #expect(CourseSection.shown(explain: false) == [.week, .deadlines, .timeline])
        #expect(CourseSection.explain.shown(explain: false) == .week)
        #expect(CourseSection.explain.shown(explain: true) == .explain)
        #expect(CourseSection.deadlines.shown(explain: false) == .deadlines)
    }
}

// MARK: - The section's model

@Suite("Explain section") @MainActor
struct ExplainModelTests {
    func course(_ service: any PageLampService, _ code: String) async throws -> CourseSummary {
        try #require(try await service.listCourses().first { $0.course.code == code })
    }

    func model(_ service: any PageLampService, _ summary: CourseSummary) async -> (ExplainModel, UInt32?) {
        let explain = ExplainModel(courseId: summary.course.id, service: service, debounce: .zero)
        let week = explain.week(summary.timeline)
        await explain.load(week: week)
        await explain.estimate.settle()
        return (explain, week)
    }

    @Test("what can't be explained, and why: hidden first, then what the AI may read")
    func blocks() async throws {
        let service = mock(.aiKey)
        #expect(ExplainModel.block(try await course(service, "DEMO101")) == nil)
        #expect(ExplainModel.block(try await course(service, "DEMO310")) == .withheldByPolicy)
        let off = FixtureService(base: service, aiAccessOff: true)
        #expect(ExplainModel.block(try await course(off, "DEMO101")) == .turnedOff)
    }

    @Test("the week: the course's default, or a picked one")
    func weeks() async throws {
        let service = mock(.aiKey)
        let summary = try await course(service, "DEMO101")
        let (explain, week) = await model(service, summary)
        #expect(week == summary.timeline.defaultWeek && week != nil)
        // A course with a default week has no "Recent materials" choice.
        #expect(ExplainModel.choices(summary.timeline, weeks: [1, 2, 3, 4]) == [.week(1), .week(2), .week(3), .week(4)])
        explain.pick(.week(2))
        #expect(explain.week(summary.timeline) == 2)
    }

    @Test("Explain: the run's end shows the new explanation, kept; a second one goes to the history")
    func explain() async throws {
        let service = mock(.aiKey)
        let summary = try await course(service, "DEMO101")
        let (explain, week) = await model(service, summary)
        #expect(explain.estimate.canGenerate && explain.shown(week: week) == nil)
        await explain.generate(week: week, uiLanguage: "en")
        guard case .finished(let first) = explain.run.phase else {
            Issue.record("expected an explanation")
            return
        }
        #expect(first.week == week && explain.shown(week: week)?.meta.generationId == first.meta.generationId)
        #expect(explain.history.map { $0.meta.generationId } == [first.meta.generationId])
        await explain.generate(week: week, uiLanguage: "en")
        guard case .finished(let second) = explain.run.phase else {
            Issue.record("expected a second explanation")
            return
        }
        #expect(explain.history.map { $0.meta.generationId } == [second.meta.generationId, first.meta.generationId])
        #expect(explain.shown(week: week)?.meta.generationId == second.meta.generationId)
        explain.show(first)
        #expect(explain.shown(week: week)?.meta.generationId == first.meta.generationId)
        // Another week: nothing picked, nothing of that week to show.
        explain.pick(.week(1))
        await explain.load(week: 1)
        #expect(explain.shownId == nil && explain.shown(week: 1) == nil)
    }

    @Test("Delete: gone at once and from the facade; the next one shows")
    func delete() async throws {
        let service = mock(.aiKey)
        let summary = try await course(service, "DEMO101")
        let (explain, week) = await model(service, summary)
        await explain.generate(week: week, uiLanguage: "en")
        await explain.generate(week: week, uiLanguage: "en")
        let newest = try #require(explain.shown(week: week))
        #expect(await explain.delete(newest, week: week))
        #expect(explain.history.count == 1 && explain.shown(week: week)?.meta.generationId != newest.meta.generationId)
        #expect(try await service.savedExplanations(course: summary.course.id, week: week).count == 1)
        // Deleting it again: the facade says not found, and the reason is kept.
        #expect(await explain.delete(newest, week: week) == false && explain.deleteFailure?.kind == .notFound)
    }

    @Test("the sharing question: on the first cloud run, closed by an answer or Dismiss")
    func sharing() async throws {
        let service = mock(.aiKey)
        let summary = try await course(service, "DEMO101")
        let (explain, week) = await model(service, summary)
        await explain.generate(week: week, uiLanguage: "en")
        let shown = try #require(explain.shown(week: week))
        #expect(shown.sharingReminder && explain.asksAboutSharing(shown))
        #expect(await explain.answerSharing(.allowed, for: shown))
        #expect(!explain.asksAboutSharing(shown))
        #expect(try await course(service, "DEMO101").course.materialSharing == .allowed)
        // Dismissing closes it too, without an answer.
        let other = ExplainModel(courseId: summary.course.id, service: service, debounce: .zero)
        other.dismissSharingQuestion(shown)
        #expect(!other.asksAboutSharing(shown))
    }

    /// A run of DEMO101's week held at its first step (before `started`), counting cancels.
    private func heldRun(
        _ scenario: MockScenario
    ) async throws -> (ExplainModel, MockService, SyncStepGate, Cancels, CourseSummary, Task<Void, Never>) {
        let gate = SyncStepGate()
        let cancels = Cancels()
        let base = mock(scenario, gate: gate)
        let service = CountingCancels(base: base, cancels: cancels)
        let summary = try await course(service, "DEMO101")
        let (explain, week) = await model(service, summary)
        let run = Task { await explain.generate(week: week, uiLanguage: "en") }
        _ = await gate.held()
        #expect(await eventually { explain.run.isRunning })
        return (explain, base, gate, cancels, summary, run)
    }

    @Test("materials that may not be shared stop a cloud run once it says it goes to the cloud, not before")
    func sharingRefused() async throws {
        let (explain, base, gate, cancels, summary, run) = try await heldRun(.aiKey)
        // "Not allowed" arrives (from another app) before the run has said where it goes.
        try await base.setCourseMaterialSharing(course: summary.course.id, answer: .notAllowed)
        let refused = try await course(base, "DEMO101")
        #expect(refused.course.materialSharing == .notAllowed && explain.runOnDevice == nil)
        await explain.stopIfRefused(refused)
        #expect(await cancels.count == 0)
        // `started` says the cloud: the course page checks again (its onChange of runOnDevice).
        await gate.advance()
        _ = await gate.held()
        #expect(await eventually { explain.runOnDevice == false })
        await explain.stopIfRefused(refused)
        #expect(await cancels.count == 1)
        await gate.open()
        await run.value
        guard case .stopped = explain.run.phase else {
            Issue.record("expected the run to stop")
            return
        }
        #expect(explain.runOnDevice == nil)
    }

    @Test("a model on this computer goes on whatever the answer about sharing")
    func sharingRefusedOnDevice() async throws {
        let (explain, base, gate, cancels, summary, run) = try await heldRun(.aiLocal)
        try await base.setCourseMaterialSharing(course: summary.course.id, answer: .notAllowed)
        let refused = try await course(base, "DEMO101")
        await gate.advance()
        _ = await gate.held()
        #expect(await eventually { explain.runOnDevice == true })
        await explain.stopIfRefused(refused)
        #expect(await cancels.count == 0)
        await gate.open()
        await run.value
        guard case .finished = explain.run.phase else {
            Issue.record("expected the run to finish")
            return
        }
    }

    @Test("a course turned off or hidden stops the run at once, wherever it goes")
    func courseRefused() async throws {
        let (explain, base, gate, cancels, _, run) = try await heldRun(.aiKey)
        let off = try await course(FixtureService(base: base, aiAccessOff: true), "DEMO101")
        #expect(off.aiMaterials == .turnedOff && explain.runOnDevice == nil)
        await explain.stopIfRefused(off)
        #expect(await cancels.count == 1)
        await gate.open()
        await run.value
        guard case .stopped = explain.run.phase else {
            Issue.record("expected the run to stop")
            return
        }
    }

    @Test("going over the budget: off until ticked, for one run only")
    func overBudget() async throws {
        let service = mock(.aiBudget)
        try await service.setMonthlyBudget(microUsd: 4_962_000)
        let summary = try await course(service, "DEMO101")
        let (explain, week) = await model(service, summary)
        #expect(explain.estimate.block == .budgetReached && !explain.estimate.canGenerate)
        await explain.generate(week: week, uiLanguage: "en")
        guard case .idle = explain.run.phase else {
            Issue.record("nothing should run without the tick")
            return
        }
        explain.estimate.overrideBudget = true
        #expect(explain.estimate.canGenerate)
        await explain.generate(week: week, uiLanguage: "en")
        guard case .finished = explain.run.phase else {
            Issue.record("the run should go over the budget")
            return
        }
        #expect(explain.history.count == 1)
        // That run only: the next is blocked again until the tick is chosen again.
        #expect(!explain.estimate.overrideBudget && explain.estimate.block == .budgetReached && !explain.estimate.canGenerate)
        await explain.generate(week: week, uiLanguage: "en")
        #expect(try await service.savedExplanations(course: summary.course.id, week: week).count == 1)
    }

    @Test("Recent materials isn't kept: once the course knows its weeks, its default week shows")
    func recentNotKept() {
        let explain = ExplainModel(courseId: "c", service: mock(.aiKey), debounce: .zero)
        let unknown = testTimeline(week: nil)
        #expect(ExplainModel.choices(unknown, weeks: []) == [.recent])
        explain.pick(.recent)
        #expect(explain.picked == nil && explain.week(unknown) == nil)
        // Term dates set, or a sync: the course now has a default week.
        let known = testTimeline(week: 3)
        #expect(explain.week(known) == 3)
        explain.pick(.week(2))
        #expect(explain.week(known) == 2)
    }

    @Test("a list read for a week no longer shown is dropped")
    func staleList() async throws {
        let gate = SyncStepGate()
        let service = mock(.aiKey, gate: gate)
        let summary = try await course(service, "DEMO101")
        let (explain, week) = await model(service, summary)
        let run = Task { await explain.generate(week: week, uiLanguage: "en") }
        _ = await gate.held()
        // The section moves to week 3 while week 4 is written.
        await explain.load(week: 3)
        #expect(explain.saved.isEmpty)
        await gate.open()
        await run.value
        // The run's end reads week 4's list: not week 3's.
        #expect(explain.saved.isEmpty)
        #expect(try await service.savedExplanations(course: summary.course.id, week: week).count == 1)
    }

    @Test("the course keeps one section model; leaving the course stops its run and drops it")
    func leave() async throws {
        let gate = SyncStepGate()
        let cancels = Cancels()
        let (app, _) = makeModel(scenario: .aiKey, gate: gate) { CountingCancels(base: $0, cancels: cancels) }
        let summary = try await course(app.service, "DEMO101")
        let detail = CourseDetailModel(courseId: summary.course.id)
        let explain = detail.explainModel(using: app)
        #expect(detail.explainModel(using: app) === explain)
        await explain.load(week: 4)
        await explain.estimate.settle()
        let run = Task { await explain.generate(week: 4, uiLanguage: "en") }
        _ = await gate.held()
        #expect(await eventually { explain.run.isRunning })
        detail.leaveExplain()
        #expect(await eventually { await cancels.count == 1 })
        #expect(detail.explain == nil && detail.explainModel(using: app) !== explain)
        await gate.open()
        await run.value
        guard case .stopped = explain.run.phase else {
            Issue.record("expected the run to stop")
            return
        }
    }
}
