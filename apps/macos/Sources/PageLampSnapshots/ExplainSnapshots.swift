// A course's Explain section (M3, preview builds) for the snapshot catalogue: "≈ $x" and Explain
// Week 4; the explanation (with what wasn't read, Include, the sharing question and an earlier
// one); the run in progress; no model chosen; a course whose policy says no AI; AI turned off;
// and the minimum window. The section's model is made and loaded beforehand (`.task` and
// `.onAppear` can't be relied on offscreen).

import SwiftUI
import PageLamp
import PageLampKit
import PageLampModel

enum ExplainSnapshots {
    static let pages: [SnapshotPage] = [
        page("course-DEMO101-explain", code: "DEMO101", setup: SnapshotSetup(scenario: .aiKey), prepare: nothing),
        page("course-DEMO101-explain-result", code: "DEMO101", setup: SnapshotSetup(scenario: .aiKey), prepare: explainTwice),
        page("course-DEMO101-explain-running", code: "DEMO101", setup: SnapshotSetup(scenario: .aiKey, syncStep: .seconds(30)), prepare: startAndHold),
        page("course-DEMO101-explain-nomodel", code: "DEMO101", setup: SnapshotSetup(scenario: .demo), prepare: nothing),
        page("course-DEMO310-explain-noai", code: "DEMO310", setup: SnapshotSetup(scenario: .aiKey), prepare: nothing),
        page(
            "course-DEMO101-explain-off", code: "DEMO101",
            setup: SnapshotSetup(scenario: .aiKey, service: { mock, _, _ in FixtureService(base: mock, aiAccessOff: true) }),
            prepare: nothing
        ),
        page(
            "course-DEMO101-explain-narrow", code: "DEMO101", setup: SnapshotSetup(scenario: .aiKey), width: CourseSnapshots.narrowWidth,
            prepare: explainTwice
        ),
    ]

    // MARK: - Preparations

    private static func nothing(_ explain: ExplainModel, _ week: UInt32?, _ model: AppModel) async {}

    /// Two explanations of the week; the first (the course's first cloud run, so with the sharing
    /// question) is picked from the history over the newest.
    private static func explainTwice(_ explain: ExplainModel, _ week: UInt32?, _ model: AppModel) async {
        for _ in 0..<2 {
            await explain.generate(week: week, uiLanguage: model.localization)
            await explain.estimate.settle()
        }
        if let first = explain.history.last {
            explain.show(first)
        }
    }

    /// The run at its first stage (the mock waits there long after the render).
    private static func startAndHold(_ explain: ExplainModel, _ week: UInt32?, _ model: AppModel) async {
        Task { await explain.generate(week: week, uiLanguage: model.localization) }
        let clock = ContinuousClock()
        let deadline = clock.now + .seconds(5)
        while clock.now < deadline {
            if case .running(let progress) = explain.run.phase, progress.stage != nil { return }
            try? await Task.sleep(for: .milliseconds(5))
        }
    }

    // MARK: - Pages

    private static func page(
        _ name: String,
        code: String,
        setup: SnapshotSetup,
        width: CGFloat = CourseSnapshots.pageWidth,
        prepare: @escaping @MainActor (ExplainModel, UInt32?, AppModel) async -> Void
    ) -> SnapshotPage {
        SnapshotPage(name: name, width: width, setup: setup) { model in
            guard let summary = model.courses.first(where: { $0.course.code == code }) else { return AnyView(EmptyView()) }
            let ui = model.ui(for: summary.course.id)
            _ = try? await model.weekMaterials(for: summary.course.id)
            ui.section = .explain
            let detail = CourseDetailModel(courseId: summary.course.id)
            await detail.loadAll(using: model)
            let explain = detail.explainModel(using: model)
            let week = explain.week(detail.week.value?.timeline ?? summary.timeline)
            await explain.load(week: week)
            await explain.estimate.settle()
            await prepare(explain, week, model)
            // Include It and Write Again priced for the explanation shown (the view's onChange
            // comes too late for the render).
            explain.prepare(for: explain.shown(week: week), week: week)
            await explain.includeEstimate.settle()
            await explain.againEstimate.settle()
            return AnyView(CourseDetailPage(summary: summary, detail: detail))
        }
    }
}
