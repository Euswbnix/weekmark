// Course detail (spec §3.2, M1 read-only): the header band in the lamp (lit for the current
// week), the source problem (S7), then This Week / Deadlines / Timeline (and Explain, M3); the read-only
// inspector column; the toolbar's ‹ › and This Week (Chrome/Toolbars.swift); the accessory bar.
// Editing, the week scrubber, downloads and the term strip are M2.

import SwiftUI
import PageLampKit
import PageLampModel

struct CourseDetailView: View {
    let courseId: String
    /// This course's model, from the window's store (DetailColumn), shared with the inspector.
    let detail: CourseDetailModel
    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @State private var width: CGFloat = PLSize.windowMainWidth

    init(courseId: String, detail: CourseDetailModel) {
        self.courseId = courseId
        self.detail = detail
    }

    var body: some View {
        @Bindable var bindable = model
        if let summary = model.course(id: courseId) {
            let ui = model.ui(for: courseId)
            // Only a web address opens as the course website (never a file or script URL).
            let website = Links.web(summary.course.url)
            ScrollView {
                CourseDetailPage(summary: summary, detail: detail)
                    .environment(\.detailColumnWidth, width)
            }
            // A new course starts at the top with fresh content; the inspector, its split view and
            // the window toolbar stay (rebuilding them on every course switch cost ~200 ms).
            .id(courseId)
            .scrollEdgeEffectStyle(.soft, for: .bottom)
            .accessoryBar()
            // .inspector hosts the page and the inspector in their own NSHostingViews, which ask
            // their content for a minimum size on every frame of a sidebar/inspector animation.
            .minimumSizeShield()
            .inspector(isPresented: $bindable.inspectorShown) {
                CourseInspector(summary: summary, detail: detail)
                    .minimumSizeShield()
                    .inspectorColumnWidth(min: PLSize.inspectorMin, ideal: PLSize.inspectorIdeal, max: PLSize.inspectorMax)
            }
            .onDetailWidthChange { width = $0 }
            .focusedSceneValue(\.courseCommands, CourseCommands(
                website: website,
                showAIPolicy: { detail.showInspector(.aiPolicy, in: model) },
                showTermDates: { detail.showInspector(.termDates, in: model) }
            ))
            .navigationTitle(summary.course.code ?? summary.course.name)
            // The overview and deadlines reload when the course's data changes (after a sync,
            // on activation); the week also when the student steps to another week.
            .task(id: summary) {
                await detail.loadOverviewAndDeadlines(using: model)
            }
            .task(id: WeekRequest(week: ui.selectedWeek, summary: summary)) {
                await detail.loadWeek(using: model)
            }
        } else {
            // S14: the course is gone (removed with its source, or not synced yet).
            EmptyState(
                symbol: "book.closed",
                title: l10n("course.notFound.title"),
                message: l10n("course.notFound.description")
            ) {
                Button(l10n("mac.actions.backToThisWeek")) { model.destination = .thisWeek }
                    .buttonStyle(.bordered)
            }
        }
    }

    private struct WeekRequest: Hashable {
        var week: UInt32?
        var summary: CourseSummary
    }
}

/// The page's document: rendered in the scroll view and by the snapshot harness.
package struct CourseDetailPage: View {
    let summary: CourseSummary
    let detail: CourseDetailModel

    @Environment(AppModel.self) private var model
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    package init(summary: CourseSummary, detail: CourseDetailModel) {
        self.summary = summary
        self.detail = detail
    }

    package var body: some View {
        let ui = model.ui(for: summary.course.id)
        // Explain shows only where it's on (else This Week, like the Tauri app's unknown tab).
        let section = ui.section.shown(explain: model.aiExplain)
        // The freshest timeline: the displayed week's, else the course list's.
        let timeline = detail.week.value?.timeline ?? summary.timeline
        let weekLine = CourseWeekLine(
            section: section,
            selectedWeek: ui.selectedWeek,
            currentWeek: timeline.currentWeek,
            outsideTerm: timeline.outsideTerm
        )
        let source = model.sources.first { $0.id == summary.course.sourceId }
        ReadingPage(spacing: PLLayout.sectionGapCourse) {
            LampBand(lit: weekLine.lit) {
                CourseHeader(summary: summary, detail: detail, weekLine: weekLine)
            }
        } content: {
            ReadingColumn(spacing: PLLayout.sectionGapCourse) {
                if let source, let problem = SourceProblem(source: source) {
                    CourseSourceAlert(source: source, problem: problem)
                }
                Group {
                    switch section {
                    case .week:
                        CourseWeekSection(summary: summary, detail: detail)
                    case .deadlines:
                        CourseDeadlinesSection(detail: detail)
                    case .timeline:
                        CourseTimelineSection(timeline: timeline, detail: detail)
                    case .explain:
                        CourseExplainSection(summary: summary, detail: detail, timeline: timeline)
                    }
                }
                .transition(.opacity)
                .id(section)
            }
            .animation(reduceMotion ? PLMotion.reduced : PLMotion.section, value: section)
        }
        .primaryActionCandidates(CourseDetailModel.primaryActionCandidates(source: source, timeline: timeline))
        // Explain's run belongs to the course, not to its section: it goes on while another
        // section shows, and stops when the student leaves the course (another course, another
        // page, the window closing).
        .onDisappear { detail.leaveExplain() }
        // Explain's model, made when the section shows (also on coming back to the course, whose
        // model left with it) and again for a new service.
        .onChange(of: section, initial: true) { _, section in
            makeExplain(section)
        }
        .onChange(of: model.serviceGeneration) {
            makeExplain(section)
        }
        // A course hidden, AI turned off or materials no longer shareable (from another app)
        // stops a run it no longer allows; so does a refused course once a run says it goes to
        // the cloud.
        .onChange(of: summary) { _, summary in
            Task { await detail.explain?.stopIfRefused(summary) }
        }
        .onChange(of: detail.explain?.runOnDevice) {
            Task { await detail.explain?.stopIfRefused(summary) }
        }
    }

    private func makeExplain(_ section: CourseSection) {
        guard section == .explain else { return }
        _ = detail.explainModel(using: model)
    }
}

/// S7, course header variant: the course's source failed its last sync.
struct CourseSourceAlert: View {
    let source: SourceRecord
    let problem: SourceProblem
    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n

    var body: some View {
        switch problem {
        case .expired(let fix):
            // The fix (M1: Sources & Sync scrolled to the source, like the capsule's fix bubble;
            // M2: the Replace sheet) is the arbiter's candidate (2); `.bordered` while the
            // capsule's tinted bubble shows the same fix.
            Callout(
                tone: .danger,
                symbol: "key",
                title: l10n("course.sourceAlert.expiredTitle"),
                message: SourceRow.canReplaceSecrets
                    ? l10n("mac.course.sourceAlert.expiredBody")
                    : l10n("mac.course.sourceAlert.expiredBodyPreview")
            ) {
                if SourceRow.canReplaceSecrets {
                    Button(l10n.fix(fix)) { model.fixSource(source.id) }
                        .arbitratedButtonStyle(.fixSource(source.id))
                } else {
                    // No Replace sheet yet (M2): lead to the source, like This Week does.
                    Button(l10n("mac.actions.openSourcesAndSync")) { model.fixSource(source.id) }
                        .buttonStyle(.bordered)
                }
            }
        case .failed(let kind):
            Callout(
                tone: .warning,
                // "other" has no useful reason to name.
                title: kind == .other
                    ? l10n("course.sourceAlert.failedTitleGeneric")
                    : l10n("course.sourceAlert.failedTitle", ["reason": l10n.sourceError(kind)]),
                message: l10n("mac.course.sourceAlert.failedBody")
            ) {
                Button(l10n("mac.actions.openSourcesAndSync")) { model.showSource(source.id) }
                    .buttonStyle(.bordered)
            }
        }
    }
}

/// The course detail models of the courses shown in this window, created on first use.
/// Not observable itself: each model is, and looking one up never changes what a view shows.
@MainActor
final class CourseDetailStore {
    private var models: [String: CourseDetailModel] = [:]

    func model(for courseId: String) -> CourseDetailModel {
        if let model = models[courseId] { return model }
        let model = CourseDetailModel(courseId: courseId)
        models[courseId] = model
        return model
    }
}
