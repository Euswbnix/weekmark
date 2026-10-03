// A course's Explain section (design §5.2, §7; the Tauri app's Explain tab; M3, preview builds):
// the week to explain, "≈ $x" and the facade's block, Explain Week N; while it runs, the
// headline, the materials read, the stage and Stop; then the explanation (ExplanationView), with
// the stale banner and the one-time question about sharing the course's materials, and the
// earlier explanations of the week. A course the AI may not read says why instead.
//
// The run belongs to the course (CourseDetailModel.explain): it goes on while another section
// shows and stops when the student leaves the course. VoiceOver hears the stages and how a run
// ended (only while this section shows), never the explanation as it's written; a failure is
// read out and takes the focus.

import AppKit
import SwiftUI
import PageLampKit
import PageLampModel

struct CourseExplainSection: View {
    let summary: CourseSummary
    let detail: CourseDetailModel
    let timeline: CourseTimeline

    // The section's model is made by the course page (CourseDetailPage), which always exists:
    // an `.onAppear` here would hang on nothing while there's no model yet.
    var body: some View {
        if let explain = detail.explain {
            ExplainContent(explain: explain, summary: summary, detail: detail, timeline: timeline)
        }
    }
}

private struct ExplainContent: View {
    let explain: ExplainModel
    let summary: CourseSummary
    let detail: CourseDetailModel
    let timeline: CourseTimeline

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @Environment(\.drawsControlStandIns) private var standIns
    @AccessibilityFocusState private var focus: Focus?

    enum Focus: Hashable {
        case heading
        case stop
        case result
        case outcome
    }

    var body: some View {
        let week = explain.week(timeline)
        ScrollViewReader { proxy in
            VStack(alignment: .leading, spacing: PLSpace.s5) {
                heading
                if let block = ExplainModel.block(summary) {
                    blocked(block)
                } else {
                    weekChoice(week)
                    if case .running(let progress) = explain.run.phase {
                        ExplainProgress(progress: progress, text: text, stopFocus: $focus) {
                            Task { await explain.stop() }
                        }
                    } else {
                        generate(week)
                    }
                    outcome
                    if let shown = explain.shown(week: week) {
                        result(shown, week: week)
                            .id(Self.resultAnchor)
                    }
                    history(week)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .onChange(of: phaseName) { _, phase in
                switch phase {
                case "running": focus = .stop
                case "finished":
                    focus = .result
                    withAnimation { proxy.scrollTo(Self.resultAnchor, anchor: .top) }
                case "stopped", "failed": focus = .outcome
                default: break
                }
            }
            .onChange(of: explain.shownId) { _, id in
                guard id != nil else { return }
                focus = .result
                withAnimation { proxy.scrollTo(Self.resultAnchor, anchor: .top) }
            }
        }
        .task(id: week) { await explain.load(week: week) }
        // Include It and Write Again are priced for the explanation on screen.
        .onChange(of: "\(explain.shown(week: week)?.meta.generationId ?? "")|\(week.map(String.init) ?? "")", initial: true) {
            explain.prepare(for: explain.shown(week: week), week: week)
        }
        .onChange(of: announcement) { _, text in
            if let text { announce(text) }
        }
        // Back from Settings ▸ AI or the Tauri app: the setup, the budget or the saved list may
        // have changed.
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { _ in
            Task { await explain.refresh(week: week) }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            Task { await explain.refresh(week: week) }
        }
    }

    // MARK: - Heading and the blocked course

    private var heading: some View {
        VStack(alignment: .leading, spacing: PLSpace.s1) {
            Text(l10n("explain.title"))
                .font(PLType.title3.font)
                .accessibilityAddTraits(.isHeader)
                .accessibilityFocused($focus, equals: .heading)
            Text(l10n("explain.hint"))
                .font(PLType.callout.font)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .paragraphLineSpacing()
        }
    }

    @ViewBuilder
    private func blocked(_ block: ExplainModel.Block) -> some View {
        switch block {
        case .courseHidden:
            Text(l10n("explain.blocked.course_hidden"))
                .fixedSize(horizontal: false, vertical: true)
        case .withheldByPolicy:
            Callout(tone: .info, symbol: "hand.raised", title: l10n("explain.blocked.withheld_by_policy")) {
                Button(l10n("mac.actions.aiPolicy")) { detail.showInspector(.aiPolicy, in: model) }
                    .buttonStyle(.bordered)
            }
        case .turnedOff:
            Callout(tone: .info, symbol: "nosign", title: l10n("mac.explain.turnedOff")) {
                Button(l10n("mac.actions.courseMaterials")) { detail.showInspector(.courseMaterials, in: model) }
                    .buttonStyle(.bordered)
            }
        }
    }

    // MARK: - The week and Generate

    @ViewBuilder
    private func weekChoice(_ week: UInt32?) -> some View {
        let weeks = model.ui(for: summary.course.id).availableWeeks
        let defaultWeek = timeline.defaultWeek ?? timeline.currentWeek
        if weeks.isEmpty, defaultWeek == nil {
            Text(l10n("explain.weeksUnknown"))
                .font(PLType.callout.font)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        } else {
            let choices = ExplainModel.choices(timeline, weeks: weeks)
            let current: ExplainModel.WeekChoice = week.map { .week($0) } ?? .recent
            LabeledContent(l10n("explain.week")) {
                if standIns {
                    Text(title(current))
                        .padding(.horizontal, PLSpace.s2)
                        .padding(.vertical, 3)
                        .background(.fill.tertiary, in: .rect(cornerRadius: 5))
                } else {
                    Picker(l10n("explain.week"), selection: Binding(get: { current }, set: { explain.pick($0) })) {
                        ForEach(choices, id: \.self) { choice in
                            Text(title(choice)).tag(choice)
                        }
                    }
                    .labelsHidden()
                    .fixedSize()
                    .disabled(explain.run.isRunning)
                }
            }
            .fixedSize()
        }
    }

    private func title(_ choice: ExplainModel.WeekChoice) -> String {
        switch choice {
        case .week(let week): text.weekOption(week, current: timeline.currentWeek)
        case .recent: l10n("explain.recent")
        }
    }

    private func generate(_ week: UInt32?) -> some View {
        VStack(alignment: .leading, spacing: PLSpace.s2) {
            EstimateView(estimate: explain.estimate) { await explain.refresh(week: week) }
            Button(text.generate(week)) {
                Task { await explain.generate(week: week, uiLanguage: model.localization) }
            }
            .buttonStyle(.bordered)
            .disabled(!explain.estimate.canGenerate)
            .accessibilityHint(explain.estimate.spokenHint(l10n))
        }
    }

    // MARK: - How the run ended

    @ViewBuilder
    private var outcome: some View {
        switch explain.run.phase {
        case .stopped:
            Text(l10n("explain.result.stopped"))
                .accessibilityFocused($focus, equals: .outcome)
        case .failed(let failure):
            VStack(alignment: .leading, spacing: 2) {
                Label {
                    Text(l10n("explain.result.failed"))
                        .font(PLType.body.font.weight(.semibold))
                } icon: {
                    Image(systemName: "exclamationmark.triangle")
                        .accessibilityHidden(true)
                }
                .foregroundStyle(PLColor.danger)
                Text(l10n.aiError(failure))
                    .fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
            .accessibilityFocused($focus, equals: .outcome)
        default:
            EmptyView()
        }
    }

    // MARK: - The explanation

    private func result(_ shown: WeeklyExplanation, week: UInt32?) -> some View {
        VStack(alignment: .leading, spacing: PLSpace.s4) {
            if shown.stale {
                HStack(alignment: .firstTextBaseline, spacing: PLSpace.s3) {
                    Label {
                        Text(l10n("explain.result.stale"))
                            .fixedSize(horizontal: false, vertical: true)
                    } icon: {
                        Image(systemName: "clock")
                            .foregroundStyle(PLColor.warning)
                            .accessibilityHidden(true)
                    }
                    Spacer(minLength: PLSpace.s3)
                    let again = explain.againEstimate(for: shown)
                    Button(l10n("mac.explain.writeAgain")) {
                        Task { await explain.writeAgain(shown, week: week, uiLanguage: model.localization) }
                    }
                    .controlSize(.small)
                    .disabled(!again.canGenerate || explain.run.isRunning)
                    .accessibilityHint(again.spokenHint(l10n))
                }
                // Written with materials included: Write Again sends them again, at this price
                // (otherwise the week's "≈ $x" above is the one).
                if !explain.againIds(shown).isEmpty {
                    EstimateView(estimate: explain.againEstimate) { await explain.refresh(week: week) }
                }
            }
            ExplanationView(
                explanation: shown,
                text: text,
                deleting: explain.deleting,
                includeEstimate: explain.includeEstimate,
                running: explain.run.isRunning,
                onAcknowledged: { await explain.refresh(week: week) },
                onInclude: {
                    Task { await explain.includeAndWriteAgain(shown, week: week, uiLanguage: model.localization) }
                },
                onDelete: { delete(shown, week: week) }
            )
            if let failure = explain.deleteFailure {
                Text(l10n.sentences(l10n("explain.result.deleteFailed"), failure.localizedDescription(in: l10n)))
                    .foregroundStyle(PLColor.danger)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if explain.asksAboutSharing(shown) {
                ExplainSharingQuestion(explain: explain, explanation: shown, summary: summary)
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(text.regionLabel(shown.week))
        .accessibilityFocused($focus, equals: .result)
    }

    private func delete(_ explanation: WeeklyExplanation, week: UInt32?) {
        Task {
            if await explain.delete(explanation, week: week) {
                announce(l10n("explain.result.deleted"))
                // The next explanation, else the heading (the Tauri app's focus).
                focus = explain.shown(week: week) == nil ? .heading : .result
            } else if let failure = explain.deleteFailure {
                announce(l10n.sentences(l10n("explain.result.deleteFailed"), failure.localizedDescription(in: l10n)))
            }
        }
    }

    // MARK: - Earlier explanations

    @ViewBuilder
    private func history(_ week: UInt32?) -> some View {
        let list = explain.history
        if list.count > 1 {
            let shownId = explain.shown(week: week)?.meta.generationId
            VStack(alignment: .leading, spacing: PLSpace.s2) {
                Text(l10n("explain.history.title"))
                    .font(PLType.headline.font)
                    .accessibilityAddTraits(.isHeader)
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(list.enumerated()), id: \.element.meta.generationId) { index, explanation in
                        if index > 0 { Divider() }
                        HStack(alignment: .firstTextBaseline, spacing: PLSpace.s2) {
                            Text(text.savedAt(explanation.meta.createdAt))
                                .monospacedDigit()
                            Text(verbatim: "·")
                                .foregroundStyle(.secondary)
                                .accessibilityHidden(true)
                            Text(explanation.meta.model)
                                .foregroundStyle(.secondary)
                            Spacer(minLength: PLSpace.s3)
                            if explanation.meta.generationId == shownId {
                                Text(l10n("explain.history.showing"))
                                    .font(PLType.callout.font)
                                    .foregroundStyle(.secondary)
                            } else {
                                Button(l10n("explain.history.show")) { explain.show(explanation) }
                                    .controlSize(.small)
                                    .accessibilityLabel(text.showFrom(explanation.meta.createdAt))
                            }
                        }
                        .padding(.vertical, PLSpace.s2)
                    }
                }
            }
        }
    }

    // MARK: - What VoiceOver hears

    private var text: ExplainText {
        ExplainText(l10n: l10n, calendar: model.calendar)
    }

    private var phaseName: String {
        switch explain.run.phase {
        case .idle: "idle"
        case .running: "running"
        case .finished: "finished"
        case .stopped: "stopped"
        case .failed: "failed"
        }
    }

    /// The stages and how the run ended, never the explanation's text (design §7).
    private var announcement: String? {
        switch explain.run.phase {
        case .running(let progress): text.stage(progress.stage)
        case .finished: l10n("explain.result.done")
        case .stopped: l10n("explain.result.stopped")
        case .failed(let failure): l10n.sentences(l10n("explain.result.failed"), l10n.aiError(failure))
        case .idle: nil
        }
    }

    private func announce(_ text: String) {
        AccessibilityNotification.Announcement(text).post()
    }

    private static let resultAnchor = "explain-result"
}

/// "Writing with OpenAI · gpt-5.4-mini", the materials read, the stage (VoiceOver hears it once,
/// as an announcement), and Stop with what leaving does.
private struct ExplainProgress: View {
    let progress: GenerationRun<WeeklyExplanation>.Progress
    let text: ExplainText
    /// The button that started the run is gone: Stop takes the focus.
    let stopFocus: AccessibilityFocusState<ExplainContent.Focus?>.Binding
    let stop: () -> Void

    @Environment(\.l10n) private var l10n
    @Environment(\.drawsControlStandIns) private var standIns

    var body: some View {
        VStack(alignment: .leading, spacing: PLSpace.s2) {
            HStack(spacing: PLSpace.s2) {
                Group {
                    // Offscreen a spinner draws nothing.
                    if standIns {
                        Image(systemName: "ellipsis.circle")
                            .foregroundStyle(.secondary)
                    } else {
                        ProgressView()
                            .controlSize(.small)
                    }
                }
                .accessibilityHidden(true)
                Text(text.headline(progress))
                    .font(PLType.body.font.weight(.semibold))
            }
            if let materials = progress.materialsIncluded {
                Text(text.reading(materials))
                    .foregroundStyle(.secondary)
            }
            if progress.stage != nil {
                Text(text.stage(progress.stage))
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
            }
            HStack(alignment: .firstTextBaseline, spacing: PLSpace.s3) {
                Button(l10n(progress.stopping ? "explain.running.stopping" : "explain.running.stop"), action: stop)
                    .disabled(progress.stopping)
                    .accessibilityHint(l10n("mac.explain.leaveHint"))
                    .accessibilityFocused(stopFocus, equals: .stop)
                Text(l10n("mac.explain.leaveHint"))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}
