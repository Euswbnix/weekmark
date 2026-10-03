// Weekly explanations in the mock, like the Tauri mock's explain.ts: the course's rules, then the
// gate, refused before any event; the stages with Stop between them; what is read and left out,
// and why; the reminder about sharing, once per course; the last 5 kept per course and week. Plus
// the facade's rules the Tauri mock lacks: links and materials without chunks are left out, "≈ $x"
// refuses a week with nothing to read, and removing AI data drops the explanations.

import Foundation
import PageLampKit
@testable import PageLampModel
import Testing

private func mock(_ scenario: MockScenario, gate: SyncStepGate? = nil) -> MockService {
    MockService(
        scenario: scenario, timing: MockService.Timing(latency: .zero, syncStep: .zero, gate: gate),
        calendar: TestClock.calendar, now: { TestClock.now }
    )
}

/// The events of one run, in order.
private func events(_ stream: GenEventStream) async -> [GenEvent] {
    stream.finish()
    var all: [GenEvent] = []
    for await event in stream.events { all.append(event) }
    return all
}

/// One short line per event.
private func names(_ events: [GenEvent]) -> [String] {
    events.map { event -> String in
        switch event {
        case .stage(let stage): AiCodes.name(stage)
        case .context(let summary, let tokens): "context \(summary.materialsIncluded) \(tokens ?? 0)"
        case .started(let id, let backend, let model, _): "started \(id) \(backend) \(model)"
        case .usage: "usage"
        case .finished(let ok): "finished \(ok)"
        default: "other"
        }
    }
}

/// A whole run of "ex-1" on DEMO101's week 4 with the aiKey scenario's model.
private let wholeRun = [
    "building_context", "context 2 12800", "started ex-1 OpenAI gpt-5.4-mini", "waiting_for_model", "usage",
    "validating", "finished true",
]

/// An explanation of DEMO101's week (4 unless given).
private func explain(
    _ service: MockService, _ id: String, week: UInt32? = 4, options: ExplainOptions = ExplainOptions(),
    observer: GenEventStream = GenEventStream()
) async throws -> WeeklyExplanation {
    try await service.explainWeek(course: "DEMO101", week: week, generationId: id, options: options, observer: observer)
}

/// The ids of a course's kept explanations, in the order the mock gives them.
private func saved(_ service: MockService, _ course: String = "DEMO101", week: UInt32?) async throws -> [String] {
    try await service.savedExplanations(course: course, week: week).map(\.meta.generationId)
}

/// DEMO101's materials of `week` by title (the fixtures number ids with one counter for every
/// course, so tests look them up).
private func byTitle(_ service: MockService, week: UInt32 = 4) async throws -> [String: MaterialView] {
    let materials = try await service.weekMaterials(course: "DEMO101", week: week).materials
    return Dictionary(uniqueKeysWithValues: materials.map { ($0.title, $0) })
}

private func left(_ material: MaterialView, _ reason: LeftOutReason, includable: Bool = false) -> LeftOutMaterial {
    LeftOutMaterial(materialId: material.id, title: material.title, reason: reason, includable: includable)
}

/// DEMO101's week 4.
private enum Title {
    static let slides = "Week 4 slides — Sampling and Surveys"
    static let reading = "Reading: Chapter 4, Who Gets Asked"
    static let assignment = "Assignment 4 — Survey Simulation"
    static let archive = "Survey dataset (large archive)"
    static let practice = "Week 4 practice questions"
    static let scan = "Scanned handout — sampling frames"
}

/// A clock the test sets (the mock reads it on its own actor).
private final class SetClock: @unchecked Sendable {
    private let lock = NSLock()
    private var current = TestClock.now

    var now: Date {
        lock.withLock { current }
    }

    func set(_ date: Date) {
        lock.withLock { current = date }
    }
}

@Suite("Explanations in the mock")
struct ExplainMockTests {
    @Test("an explanation: the events in order, the week's first two readable materials cited, kept")
    func explanation() async throws {
        let service = mock(.aiKey)
        let week = try await byTitle(service)
        let slides = try #require(week[Title.slides])
        let reading = try #require(week[Title.reading])
        let assignment = try #require(week[Title.assignment])
        let archive = try #require(week[Title.archive])
        let practice = try #require(week[Title.practice])
        let scan = try #require(week[Title.scan])
        let stream = GenEventStream()
        // No week: the course's default week (4).
        let explanation = try await service.explainWeek(
            course: "DEMO101", week: nil, generationId: "ex-1", options: ExplainOptions(), observer: stream
        )
        let seen = await events(stream)
        #expect(names(seen) == wholeRun)
        #expect(explanation.week == 4 && explanation.courseId == slides.courseId)
        #expect(explanation.sections.map(\.heading) == [Title.slides, Title.reading])
        #expect(explanation.sections.allSatisfy { $0.paragraphs.count == 2 })
        let citations = explanation.sections.flatMap(\.paragraphs).flatMap(\.citations)
        #expect(citations.map(\.handle) == ["M1", "M1", "M2", "M2"])
        #expect(citations.map(\.locator) == ["p. 2", "p. 5", "p. 2", "p. 5"])
        #expect(citations.map(\.materialId) == [slides.id, slides.id, reading.id, reading.id])
        #expect(citations.map(\.url) == [slides.url, slides.url, reading.url, reading.url])
        #expect(slides.url == "https://canvas.demo.test/files/8")
        #expect(explanation.checkQuestions == [
            "What is the main point of \(Title.slides)?", "What is the main point of \(Title.reading)?",
        ])
        #expect(explanation.leftOut == [
            left(assignment, .looksLikeAssessment, includable: true), left(archive, .noText), left(scan, .noText),
            left(practice, .overBudget),
        ])
        let usage = TokenUsage(inputTokens: 12_800, cachedInputTokens: 0, outputTokens: 1_100, reasoningTokens: nil)
        let context = ContextSummary(
            courses: [ContextCourse(courseId: slides.courseId, state: .readable, textIncluded: true)],
            materialsIncluded: 2, materialsTrimmed: 0, leftOut: explanation.leftOut
        )
        #expect(explanation.meta == GenerationMeta(
            generationId: "ex-1", feature: .weeklyExplanation, backendLabel: "OpenAI", model: "gpt-5.4-mini",
            onDevice: false, createdAt: TestClock.now, usage: usage, estCostMicroUsd: 3_100, estimated: false,
            context: context, promptVersion: 1
        ))
        #expect(seen.contains(.context(summary: context, inputTokens: 12_800)) && seen.contains(.usage(usage: usage)))
        #expect(!explanation.stale && explanation.sharingReminder && explanation.droppedCitations == 0)
        #expect(!explanation.citeAiUse)
        #expect(try await saved(service, week: 4) == ["ex-1"])
    }

    @Test("left out in the facade's order; include brings back only what looks like graded work")
    func leftOut() async throws {
        let service = mock(.aiKey)
        let week = try await byTitle(service)
        let assignment = try #require(week[Title.assignment])
        let archive = try #require(week[Title.archive])
        let practice = try #require(week[Title.practice])
        let scan = try #require(week[Title.scan])
        let usual = [
            left(assignment, .looksLikeAssessment, includable: true), left(archive, .noText), left(scan, .noText),
            left(practice, .overBudget),
        ]
        // No text and over the length limit stay out whatever include says.
        for (index, include) in [[], [archive.id], [practice.id], [scan.id]].enumerated() {
            let explanation = try await explain(service, "ex-\(index)", options: ExplainOptions(include: include))
            #expect(explanation.leftOut == usual, "include \(include)")
            #expect(explanation.sections.map(\.heading) == [Title.slides, Title.reading])
        }
        // Included, the assignment is read in its own place (the week's second readable material),
        // and the reading is over the limit instead.
        let reading = try #require(week[Title.reading])
        let included = try await explain(service, "ex-included", options: ExplainOptions(include: [assignment.id]))
        #expect(included.leftOut == [
            left(archive, .noText), left(scan, .noText), left(reading, .overBudget), left(practice, .overBudget),
        ])
        #expect(included.sections.map(\.heading) == [Title.slides, Title.assignment])
        #expect(included.meta.context.leftOut == included.leftOut)

        // Week 3: the recording has no text. Week 1: one material, one section, one question.
        let three = try await explain(service, "ex-week-3", week: 3)
        #expect(three.sections.map(\.heading) == ["Week 3 slides — Measuring Nothing Carefully", "Lab 3 notebook"])
        #expect(three.leftOut.map(\.title) == ["Week 3 lecture recording"] && three.leftOut.map(\.reason) == [.noText])
        let one = try await explain(service, "ex-week-1", week: 1)
        #expect(one.sections.count == 1 && one.checkQuestions.count == 1 && one.leftOut.isEmpty)
        #expect(one.meta.usage.inputTokens == 6_400 && one.meta.context.materialsIncluded == 1)
    }

    @Test("the facade's rules the Tauri mock lacks: a link first, and a material without chunks has no text")
    func facadeRules() {
        func material(_ id: String, _ title: String, kind: MaterialKind = .file, chunks: UInt32 = 4) -> MaterialView {
            MaterialView(
                id: id, courseId: "c", title: title, kind: kind, moduleId: nil, moduleName: nil, weekHint: 4,
                publishedAt: TestClock.now, url: nil, textStatus: .ok, textError: nil, downloadBlocked: nil,
                chunkCount: chunks
            )
        }
        let link = material("link", "Quiz 1", kind: .externalLink)
        let empty = material("empty", "Problem Set 3", chunks: 0)
        let problems = material("ps", "Problem Set 2")
        let notes = material("notes", "Week 4 notes")
        let slides = material("slides", "Week 4 slides")
        // Included, graded-looking work is read; a link or a material without text never is.
        let (read, leftOut) = MockService.explanationSelection(
            [link, empty, problems, notes, slides], include: ["link", "empty", "ps"]
        )
        #expect(read.map(\.id) == ["ps", "notes"])
        #expect(leftOut == [left(link, .externalLink), left(empty, .noText), left(slides, .overBudget)])
        let (usual, usualLeftOut) = MockService.explanationSelection([problems, notes, slides], include: [])
        #expect(usual.map(\.id) == ["notes", "slides"])
        #expect(usualLeftOut == [left(problems, .looksLikeAssessment, includable: true)])
    }

    @Test("no week: the materials of the last 14 days, newest first, then by title")
    func recent() {
        func material(_ title: String, days: Int?) -> MaterialView {
            MaterialView(
                id: title, courseId: "c", title: title, kind: .file, moduleId: nil, moduleName: nil, weekHint: nil,
                publishedAt: days.map { TestClock.at($0, 9) }, url: nil, textStatus: .ok, textError: nil,
                downloadBlocked: nil, chunkCount: 4
            )
        }
        let materials = [
            material("Older", days: -20), material("B", days: -1), material("Undated", days: nil),
            material("C", days: -3), material("A", days: -1), material("Tomorrow", days: 1),
        ]
        #expect(MockService.recentMaterials(materials, now: TestClock.now).map(\.title) == ["A", "B", "C"])
    }

    @Test("the course's rules, then the gate: refused before any event, nothing kept")
    func refusals() async throws {
        let cases: [(MockScenario, String, UInt32?, BlockReason?)] = [
            (.aiKey, "DEMO310", nil, .coursePolicyProhibited),
            (.aiKey, "DEMO099", nil, .courseHidden),
            (.aiKey, "DEMO101", 5, .noReadableMaterials),
            // The Tauri mock's order: nothing to read comes before the answer about sharing.
            (.aiKey, "DEMO205", 4, .noReadableMaterials),
            (.aiKey, "DEMO205", 1, .materialSharingNotAllowed),
            (.demo, "DEMO101", nil, .noModelChosen),
            (.aiUnpriced, "DEMO101", nil, .priceUnknownNotAcknowledged),
            (.aiErrors, "DEMO101", nil, .priceUnknownNotAcknowledged),
            (.aiDisclosureChanged, "DEMO101", nil, .disclosureNotAcknowledged),
            (.aiBudget, "DEMO101", nil, .budgetReached),
            (.aiKey, "nope", nil, nil),
            (.empty, "DEMO101", nil, nil),
        ]
        for (scenario, course, week, reason) in cases {
            let service = mock(scenario)
            let stream = GenEventStream()
            do {
                _ = try await service.explainWeek(
                    course: course, week: week, generationId: "ex-1", options: ExplainOptions(), observer: stream
                )
                Issue.record("expected a refusal: \(scenario) \(course)")
            } catch {
                if let reason {
                    #expect(error.kind == .blocked && error.blocked == reason, "\(scenario) \(course)")
                } else {
                    #expect(error.kind == .notFound, "\(scenario) \(course)")
                }
            }
            #expect(await events(stream).isEmpty, "\(scenario) \(course)")
            #expect(await service.callCount("explainWeek") == 1)
            if reason != nil {
                #expect(try await service.savedExplanations(course: course, week: nil).isEmpty)
            }
        }
    }

    @Test("the reminder about sharing: once per course, for a cloud model and no answer or 'not sure'")
    func sharingReminder() async throws {
        let key = mock(.aiKey)
        #expect(try await explain(key, "ex-1").sharingReminder)
        #expect(try await !explain(key, "ex-2").sharingReminder)
        // Each course has its own: DEMO205, answered "not sure" now.
        try await key.setCourseMaterialSharing(course: "DEMO205", answer: .notSure)
        let other = try await key.explainWeek(
            course: "DEMO205", week: 1, generationId: "ex-3", options: ExplainOptions(), observer: GenEventStream()
        )
        #expect(other.sharingReminder)
        // On this computer nothing is shared.
        #expect(try await !explain(mock(.aiLocal), "ex-1").sharingReminder)
        // Answered yes: no reminder; "not sure": one.
        let allowed = mock(.aiKey)
        try await allowed.setCourseMaterialSharing(course: "DEMO101", answer: .allowed)
        #expect(try await !explain(allowed, "ex-1").sharingReminder)
        let notSure = mock(.aiKey)
        try await notSure.setCourseMaterialSharing(course: "DEMO101", answer: .notSure)
        #expect(try await explain(notSure, "ex-1").sharingReminder)
    }

    @Test("DEMO205's materials may not go to a cloud model; on this computer they may")
    func notShared() async throws {
        do {
            _ = try await mock(.aiKey).explainWeek(
                course: "DEMO205", week: 1, generationId: "ex-1", options: ExplainOptions(), observer: GenEventStream()
            )
            Issue.record("expected blocked")
        } catch {
            #expect(error.kind == .blocked && error.blocked == .materialSharingNotAllowed)
        }
        let local = try await mock(.aiLocal).explainWeek(
            course: "DEMO205", week: 1, generationId: "ex-1", options: ExplainOptions(), observer: GenEventStream()
        )
        #expect(local.meta.onDevice && local.meta.estCostMicroUsd == nil && !local.sharingReminder)
        #expect(local.meta.backendLabel == "Ollama" && local.meta.model == "qwen3.5:9b")
        #expect(local.sections.map(\.heading) == ["Unit A notes"])
    }

    @Test("the course records carry the student's answer about sharing; not allowed refuses a cloud run")
    func courseRecords() async throws {
        let service = mock(.aiKey)
        let courses = try await service.listCourses()
        #expect(courses.first { $0.course.code == "DEMO205" }?.course.materialSharing == .notAllowed)
        #expect(courses.first { $0.course.code == "DEMO101" }?.course.materialSharing == .unanswered)
        try await service.setCourseMaterialSharing(course: "DEMO101", answer: .notAllowed)
        #expect(try await service.listCourses().first { $0.course.code == "DEMO101" }?.course.materialSharing == .notAllowed)
        #expect(try await service.courseOverview(course: "DEMO101").course.materialSharing == .notAllowed)
        #expect(try await service.weekMaterials(course: "DEMO101", week: nil).course.materialSharing == .notAllowed)
        #expect(try await service.keepCourseCurrent(course: "DEMO101", until: nil).materialSharing == .notAllowed)
        do {
            _ = try await explain(service, "ex-1")
            Issue.record("expected blocked")
        } catch {
            #expect((error as? PageLampFailure)?.blocked == .materialSharingNotAllowed)
        }
    }

    @Test("over the budget only when chosen for this run")
    func overBudget() async throws {
        let service = mock(.aiBudget)
        do {
            _ = try await service.explainWeek(
                course: "DEMO101", week: 4, generationId: "ex-1", options: ExplainOptions(), observer: GenEventStream()
            )
            Issue.record("expected blocked")
        } catch {
            #expect(error.kind == .blocked && error.blocked == .budgetReached)
        }
        let explanation = try await explain(service, "ex-1", options: ExplainOptions(overrideBudget: true))
        #expect(explanation.meta.estCostMicroUsd == 3_100)
    }

    @Test("kept: newest first, 5 per course and week; every week's without one")
    func kept() async throws {
        let service = mock(.aiKey)
        for number in 1...6 {
            _ = try await explain(service, "ex-\(number)")
        }
        #expect(try await saved(service, week: 4) == ["ex-6", "ex-5", "ex-4", "ex-3", "ex-2"])
        _ = try await explain(service, "ex-7", week: 3)
        #expect(try await saved(service, week: 3) == ["ex-7"])
        // The clock stands still: the one kept later comes first.
        #expect(try await saved(service, week: nil) == ["ex-7", "ex-6", "ex-5", "ex-4", "ex-3", "ex-2"])
        #expect(try await saved(service, "DEMO205", week: nil).isEmpty)
    }

    @Test("every week's: the newest first, whatever order they were kept in")
    func newestFirst() async throws {
        let clock = SetClock()
        let service = MockService(
            scenario: .aiKey, timing: MockService.Timing(latency: .zero, syncStep: .zero),
            calendar: TestClock.calendar, now: { clock.now }
        )
        clock.set(TestClock.now.addingTimeInterval(120))
        _ = try await explain(service, "ex-later", week: 3)
        clock.set(TestClock.now.addingTimeInterval(60))
        _ = try await explain(service, "ex-earlier")
        #expect(try await saved(service, week: nil) == ["ex-later", "ex-earlier"])
    }

    @Test("delete: gone from every list; unknown ids and courses are not found")
    func delete() async throws {
        let service = mock(.aiKey)
        _ = try await explain(service, "ex-1")
        _ = try await explain(service, "ex-2")
        try await service.deleteExplanation(generationId: "ex-1")
        #expect(try await saved(service, week: 4) == ["ex-2"])
        #expect(try await saved(service, week: nil) == ["ex-2"])
        do {
            try await service.deleteExplanation(generationId: "ex-1")
            Issue.record("expected not found")
        } catch {
            #expect(error.kind == .notFound && error.message == "There is no explanation ex-1.")
        }
        do {
            _ = try await service.savedExplanations(course: "nope", week: nil)
            Issue.record("expected not found")
        } catch {
            #expect(error.kind == .notFound)
        }
    }

    @Test("Stop: the run ends at its next step, cancelled, nothing kept; the id runs again", arguments: [0, 1, 2] as [UInt32])
    func stop(at step: UInt32) async throws {
        let gate = SyncStepGate()
        let service = mock(.aiKey, gate: gate)
        let stream = GenEventStream()
        let run = Task { try await explain(service, "ex-1", observer: stream) }
        await gate.advance(until: SyncStepPosition(sourceId: "ex-1", step: step))
        try await service.cancelGeneration(generationId: "ex-1")
        await gate.open()
        do {
            _ = try await run.value
            Issue.record("expected cancelled")
        } catch {
            #expect((error as? PageLampFailure)?.kind == .cancelled)
        }
        // The events up to the step it was held at, then the end.
        let reached = [1, 4, 6][Int(step)]
        let seen = await events(stream)
        #expect(names(seen) == Array(wholeRun.prefix(reached)) + ["finished false"])
        #expect(try await saved(service, week: nil).isEmpty)
        // No longer running: the same id goes again (the gate is open now).
        #expect(try await explain(service, "ex-1").meta.generationId == "ex-1")
    }

    @Test("one run per id: the same id while it runs is busy, with no events")
    func busy() async throws {
        let gate = SyncStepGate()
        let service = mock(.aiKey, gate: gate)
        let first = Task { try await explain(service, "ex-1") }
        #expect(await gate.held() == SyncStepPosition(sourceId: "ex-1", step: 0))
        let stream = GenEventStream()
        do {
            _ = try await service.explainWeek(
                course: "DEMO101", week: 4, generationId: "ex-1", options: ExplainOptions(), observer: stream
            )
            Issue.record("expected busy")
        } catch {
            #expect(error.kind == .busy)
        }
        #expect(await events(stream).isEmpty)
        await gate.open()
        #expect(try await first.value.meta.generationId == "ex-1")
    }

    @Test("looks like graded work: the facade's examples")
    func gradedWork() {
        for title in [
            "Assignment 2", "Homework 3 (due Oct 14)", "HW4", "hw 5", "Problem Set 1", "Quiz 2", "Final Exam", "Midterm",
            "Lab report template",
        ] {
            #expect(MockService.looksLikeAssessment(title), "\(title)")
        }
        for title in [
            "Midterm review", "Practice quiz solutions", "Exam preparation notes", "Week 3 slides",
            "Lecture 5: testing hypotheses", "Homeworking tips", "Shows",
        ] {
            #expect(!MockService.looksLikeAssessment(title), "\(title)")
        }
    }

    @Test("≈ $x: a week with nothing to read is refused with no amount, like the run")
    func estimate() async throws {
        let key = mock(.aiKey)
        let empty = try await key.estimateGeneration(request: .weeklyExplanation(course: "DEMO101", week: 5))
        #expect(empty.wouldBlock == .noReadableMaterials && empty.microUsdUpper == nil && empty.inputTokens == 0)
        #expect(try await key.estimateGeneration(request: .weeklyExplanation(course: "DEMO101", week: nil)).microUsdUpper == 69_750)
        // The course's rules come first: for a cloud model, DEMO205's answer about sharing.
        #expect(try await key.estimateGeneration(request: .weeklyExplanation(course: "DEMO205", week: 4)).wouldBlock == .materialSharingNotAllowed)
        // On this computer, its week 4 has nothing to read, and its week 1 is free.
        let local = mock(.aiLocal)
        #expect(try await local.estimateGeneration(request: .weeklyExplanation(course: "DEMO205", week: nil)).wouldBlock == .noReadableMaterials)
        let free = try await local.estimateGeneration(request: .weeklyExplanation(course: "DEMO205", week: 1))
        #expect(free.wouldBlock == nil && free.microUsdUpper == 0)
        // No model comes before everything.
        #expect(try await mock(.demo).estimateGeneration(request: .weeklyExplanation(course: "DEMO101", week: 5)).wouldBlock == .noModelChosen)
    }

    @Test("the answers' language stays; removing all AI data drops the explanations and the reminders, not the answers")
    func removeAllAiData() async throws {
        let service = mock(.aiKey)
        #expect(try await service.aiOutputLanguage() == .ui)
        try await service.setAiOutputLanguage(language: .course)
        #expect(try await service.aiOutputLanguage() == .course)
        #expect(try await explain(service, "ex-1").sharingReminder)
        _ = try await explain(service, "ex-2", week: 3)
        #expect(try await service.removeAllAiData().generationsRemoved == 2)
        #expect(try await service.aiOutputLanguage() == .ui)
        #expect(try await saved(service, week: nil).isEmpty)
        // Set up again: the reminder shows again; DEMO205's answer stayed.
        let openai = BackendRef.provider(providerId: "openai")
        _ = try await service.addModelProvider(preset: "openai", baseUrl: nil, apiKey: "sk-demo-1234")
        try await service.acknowledgeAiDisclosure(backend: openai, version: 3101)
        try await service.setFeatureModel(
            feature: .weeklyExplanation, choice: ModelChoice(backend: openai, model: "gpt-6-luna", effort: .lowest)
        )
        #expect(try await explain(service, "ex-3").sharingReminder)
        #expect(try await service.listCourses().first { $0.course.code == "DEMO205" }?.course.materialSharing == .notAllowed)
    }

    @Test("deleting a course's generated content drops its explanations only")
    func deleteGenerated() async throws {
        let service = mock(.aiLocal)
        _ = try await explain(service, "ex-1")
        _ = try await service.explainWeek(
            course: "DEMO205", week: 1, generationId: "ex-2", options: ExplainOptions(), observer: GenEventStream()
        )
        #expect(try await service.deleteGenerated(course: "DEMO205") == 1)
        #expect(try await saved(service, "DEMO205", week: nil).isEmpty)
        #expect(try await saved(service, week: nil) == ["ex-1"])
        _ = try await explain(service, "ex-3", week: 3)
        #expect(try await service.deleteGenerated(course: nil) == 2)
        #expect(try await saved(service, week: nil).isEmpty)
    }
}
