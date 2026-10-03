// Weekly explanations in the mock (M3; design §5.2), like the Tauri mock's explain.ts: the
// course's rules, which of the week's materials are read and which are left out and why, the AI
// gate, the run's stages (Stop between them), an explanation citing every paragraph, and the last
// 5 kept per course and week, newest first. Where the Tauri mock is silent it follows the facade:
// an external link and a material without chunks are left out too, "≈ $x" refuses a week with
// nothing to read (MockService+Ai), an included material is priced only where the run lifts it,
// and removing AI data drops the explanations.

import Foundation
import PageLampKit

extension MockService {
    /// Explanations kept per course and week (the facade keeps 5 accepted ones).
    private static let explanationsKept = 5
    /// How many readable materials fit an explanation; the others are over the length limit.
    private static let explanationMaterials = 2
    /// The pretend context: tokens per material read.
    private static let tokensPerMaterial: UInt64 = 6_400
    /// The pretend explanation's length, in tokens.
    private static let explanationOutputTokens: UInt64 = 1_100
    /// What a cloud run costs, in micro-USD (nothing is counted on this computer).
    private static let explanationCost: UInt64 = 3_100

    /// Words that mark a title as graded work (whole words, any case; the facade's).
    private static let assessmentWords = [
        "assignment", "homework", "hw", "problem set", "pset", "quiz", "exam", "midterm", "test", "lab report",
    ]
    /// Words that make a graded-looking title study material after all ("Midterm review").
    private static let studyWords = [
        "review", "practice", "solution", "solutions", "notes", "lecture", "slides", "preparation",
    ]

    public func explainWeek(
        course reference: String, week: UInt32?, generationId: String, options: ExplainOptions, observer: any GenObserver
    ) async throws(PageLampFailure) -> WeeklyExplanation {
        await respond("explainWeek")
        guard !db.features.runs.running.contains(generationId) else {
            throw PageLampFailure(kind: .busy, message: "This explanation is being written.")
        }
        db.features.runs.running.insert(generationId)
        defer {
            db.features.runs.running.remove(generationId)
            db.features.runs.cancelled.remove(generationId)
        }
        return try await writeExplanation(
            reference, week: week, generationId: generationId, options: options, observer: observer
        )
    }

    /// The kept explanations of a week (every week when nil), newest first; at the same moment,
    /// the one kept later first (the facade's `created_at DESC, rowid DESC`).
    public func savedExplanations(course reference: String, week: UInt32?) async throws(PageLampFailure) -> [WeeklyExplanation] {
        await respond("savedExplanations")
        let id = db.courses[try courseIndex(reference)].course.id
        return db.features.runs.explanations.enumerated()
            .filter { $0.element.courseId == id && (week == nil || $0.element.week == week) }
            .sorted { first, second in
                let (a, b) = (first.element.meta.createdAt, second.element.meta.createdAt)
                return a == b ? first.offset < second.offset : a > b
            }
            .map { $0.element }
    }

    public func deleteExplanation(generationId: String) async throws(PageLampFailure) {
        await respond("deleteExplanation")
        guard let index = db.features.runs.explanations.firstIndex(where: { $0.meta.generationId == generationId }) else {
            throw PageLampFailure(kind: .notFound, message: "There is no explanation \(generationId).")
        }
        db.features.runs.explanations.remove(at: index)
    }

    // MARK: - The week's materials (the run and "≈ $x" alike)

    /// The week an explanation covers and its materials, like the facade's `week_materials`: the
    /// week asked for, else the course's default week, else (no week) the recent materials.
    func explanationWeek(_ course: MockCourse, _ week: UInt32?) -> (week: UInt32?, materials: [MaterialView]) {
        if let shown = week ?? course.timeline.defaultWeek {
            return (shown, course.materials.filter { $0.weekHint == shown })
        }
        return (nil, Self.recentMaterials(course.materials, now: now()))
    }

    /// The materials published in the last 14 days, newest first, then by title.
    static func recentMaterials(_ materials: [MaterialView], now: Date) -> [MaterialView] {
        let since = now.addingTimeInterval(-14 * 86_400)
        return materials
            .filter { material in
                guard let published = material.publishedAt else { return false }
                return published >= since && published <= now
            }
            .sorted { first, second in
                let (a, b) = (first.publishedAt ?? .distantPast, second.publishedAt ?? .distantPast)
                return a == b ? first.title < second.title : a > b
            }
    }

    /// What an explanation sends of `include`: the graded-looking materials it lifts and reads
    /// (pagelamp-core's `week_context_including`). An id that names nothing read there, or a
    /// material read anyway, isn't lifted.
    static func liftedIncludes(_ read: [MaterialView], _ include: [String]) -> [String] {
        read.filter { include.contains($0.id) && looksLikeAssessment($0.title) }.map { $0.id }
    }

    /// What an explanation reads and leaves out, in the facade's order: an external link, then no
    /// readable text (whatever the title says), then what looks like graded work unless `include`
    /// names it. The first two readable materials are read; the rest are over the length limit,
    /// which `include` doesn't change. Only graded-looking work can be brought back (`includable`).
    static func explanationSelection(
        _ materials: [MaterialView], include: [String]
    ) -> (read: [MaterialView], leftOut: [LeftOutMaterial]) {
        var readable: [MaterialView] = []
        var leftOut: [LeftOutMaterial] = []
        for material in materials {
            let reason: LeftOutReason?
            if material.kind == .externalLink {
                reason = .externalLink
            } else if material.textStatus != .ok || material.chunkCount == 0 {
                reason = .noText
            } else if looksLikeAssessment(material.title) && !include.contains(material.id) {
                reason = .looksLikeAssessment
            } else {
                reason = nil
            }
            if let reason {
                leftOut.append(LeftOutMaterial(
                    materialId: material.id, title: material.title, reason: reason,
                    includable: reason == .looksLikeAssessment
                ))
            } else {
                readable.append(material)
            }
        }
        for material in readable.dropFirst(explanationMaterials) {
            leftOut.append(LeftOutMaterial(
                materialId: material.id, title: material.title, reason: .overBudget, includable: false
            ))
        }
        return (Array(readable.prefix(explanationMaterials)), leftOut)
    }

    /// A title with a graded-work word and no study word (whole words, any case; "hw3" counts as
    /// "hw"), like the facade's `looks_like_assessment`.
    static func looksLikeAssessment(_ title: String) -> Bool {
        let words = title.split { !($0.isLetter || $0.isNumber) }.map { $0.lowercased() }
        let joined = " " + words.joined(separator: " ") + " "
        func has(_ phrase: String) -> Bool {
            if joined.contains(" \(phrase) ") { return true }
            guard phrase == "hw" else { return false }
            return words.contains { word in
                let rest = word.dropFirst(2)
                return word.hasPrefix("hw") && !rest.isEmpty && rest.allSatisfy { $0.isASCII && $0.isNumber }
            }
        }
        return assessmentWords.contains(where: has) && !studyWords.contains(where: has)
    }

    // MARK: - A run

    private func writeExplanation(
        _ reference: String, week requested: UInt32?, generationId: String, options: ExplainOptions,
        observer: any GenObserver
    ) async throws(PageLampFailure) -> WeeklyExplanation {
        let course = db.courses[try courseIndex(reference)]
        let id = course.course.id
        // The course's rules, then what the week has to read: all before the gate and any event.
        if course.course.hidden {
            throw PageLampFailure(kind: .blocked, message: "The course is hidden.", blocked: .courseHidden)
        }
        let state = Self.aiMaterials(course.course)
        if state == .withheldByPolicy {
            throw PageLampFailure(
                kind: .blocked, message: "The course's AI policy withholds its materials.", blocked: .coursePolicyProhibited
            )
        }
        if state == .turnedOff {
            throw PageLampFailure(kind: .blocked, message: "AI access is off for this course.", blocked: .courseAiTurnedOff)
        }
        let (week, materials) = explanationWeek(course, requested)
        let (read, leftOut) = Self.explanationSelection(materials, include: options.include)
        guard !read.isEmpty else {
            throw PageLampFailure(kind: .blocked, message: "No readable materials this week.", blocked: .noReadableMaterials)
        }
        // Priced with what the run sends (the facade's run checks its own prompt).
        let run = try aiRun(
            .weeklyExplanation(course: id, week: week, include: Self.liftedIncludes(read, options.include)),
            overrideBudget: options.overrideBudget
        )

        let tokens = Self.tokensPerMaterial * UInt64(read.count)
        let summary = ContextSummary(
            courses: [ContextCourse(courseId: id, state: state, textIncluded: true)],
            materialsIncluded: UInt32(read.count), materialsTrimmed: 0, leftOut: leftOut
        )
        let usage = TokenUsage(
            inputTokens: tokens, cachedInputTokens: 0, outputTokens: Self.explanationOutputTokens, reasoningTokens: nil
        )
        observer.onEvent(event: .stage(stage: .buildingContext))
        try await explanationStep(generationId, 0, observer)
        observer.onEvent(event: .context(summary: summary, inputTokens: tokens))
        observer.onEvent(event: .started(
            generationId: generationId, backendLabel: run.backendLabel, model: run.model, onDevice: run.onDevice
        ))
        observer.onEvent(event: .stage(stage: .waitingForModel))
        try await explanationStep(generationId, 1, observer)
        observer.onEvent(event: .usage(usage: usage))
        observer.onEvent(event: .stage(stage: .validating))
        try await explanationStep(generationId, 2, observer)
        observer.onEvent(event: .finished(ok: true))

        // Question (b) once per course: a cloud model, and the student hasn't said yes or no.
        let sharing = db.features.sharing[id] ?? course.course.materialSharing
        let reminder = !run.onDevice && (sharing == .unanswered || sharing == .notSure)
            && !db.features.runs.sharingReminded.contains(id)
        if reminder { db.features.runs.sharingReminded.insert(id) }

        let explanation = WeeklyExplanation(
            meta: GenerationMeta(
                generationId: generationId, feature: .weeklyExplanation, backendLabel: run.backendLabel,
                model: run.model, onDevice: run.onDevice, createdAt: now(), usage: usage,
                estCostMicroUsd: run.onDevice ? nil : Self.explanationCost, estimated: false, context: summary,
                promptVersion: 1
            ),
            courseId: id,
            week: week,
            sections: read.enumerated().map { index, material in
                ExplanationSection(heading: material.title, paragraphs: [
                    ExplanationParagraph(
                        text: "The key idea of **\(material.title)** is how this week's topic builds on the last one.",
                        citations: [Self.citation(material, index + 1, locator: "p. 2")]
                    ),
                    ExplanationParagraph(
                        text: "Work through the example at the end before the next lecture.",
                        citations: [Self.citation(material, index + 1, locator: "p. 5")]
                    ),
                ])
            },
            checkQuestions: read.map { "What is the main point of \($0.title)?" },
            leftOut: leftOut,
            stale: false,
            sharingReminder: reminder,
            droppedCitations: 0,
            citeAiUse: course.course.aiPolicy == .allowedWithCitation
        )
        // Newest first; past the 5th of this course and week, the oldest go.
        db.features.runs.explanations.insert(explanation, at: 0)
        let sameWeek = db.features.runs.explanations.indices.filter {
            db.features.runs.explanations[$0].courseId == id && db.features.runs.explanations[$0].week == week
        }
        for index in sameWeek.dropFirst(Self.explanationsKept).reversed() {
            db.features.runs.explanations.remove(at: index)
        }
        return explanation
    }

    /// A run's step, then Stop: a run asked to stop ends here, cancelled.
    private func explanationStep(
        _ generationId: String, _ step: UInt32, _ observer: any GenObserver
    ) async throws(PageLampFailure) {
        await generationStep(generationId, step)
        if db.features.runs.cancelled.contains(generationId) {
            observer.onEvent(event: .finished(ok: false))
            throw PageLampFailure(kind: .cancelled, message: "The explanation was stopped.")
        }
    }

    /// The "model's" citation of the `number`th material read.
    private static func citation(_ material: MaterialView, _ number: Int, locator: String) -> Citation {
        Citation(handle: "M\(number)", materialId: material.id, title: material.title, locator: locator, url: material.url)
    }
}
