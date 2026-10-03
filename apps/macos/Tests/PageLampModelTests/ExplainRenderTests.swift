// A course page opened at Explain makes the section's model itself: nothing is made beforehand
// here (the snapshot pages make it before rendering, so they can't show this).

import AppKit
import PageLamp
import PageLampKit
import PageLampModel
import PageLampSnapshots
import SwiftUI
import Testing

@Suite("Explain on the course page") @MainActor
struct ExplainRenderTests {
    @Test("opening a course at Explain makes the section's model, with nothing made beforehand")
    func madeOnShow() async throws {
        let page = try #require(SnapshotCatalog.page(named: "course-DEMO101-explain"))
        let model = await SnapshotRenderer.model(for: page, language: .english, calendar: TestClock.calendar, now: TestClock.now)
        let summary = try #require(model.courses.first { $0.course.code == "DEMO101" })
        model.ui(for: summary.course.id).section = .explain
        let detail = CourseDetailModel(courseId: summary.course.id)
        await detail.loadAll(using: model)
        #expect(detail.explain == nil)
        let renderer = ImageRenderer(
            content: CourseDetailPage(summary: summary, detail: detail)
                .frame(width: 900)
                .pageLampEnvironment(model)
                .environment(\.drawsControlStandIns, true)
        )
        #expect(renderer.nsImage != nil)
        #expect(await until { detail.explain != nil })
    }
}
