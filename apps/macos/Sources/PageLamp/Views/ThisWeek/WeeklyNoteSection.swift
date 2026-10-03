// The weekly note (M3, preview builds; design §5.3, §7; the Tauri app's WeeklyNoteCard), between
// Next 7 days and Your study plan: a few sentences on the week and three things to focus on, from
// the courses' structure and the plan (no material text is sent). The note shown (the run's new
// one, the newest kept, or one picked from the earlier notes) with its week, the AI-generated line,
// Copy and Delete…; "≈ $x" and Write My Weekly Note; the run's headline and stage with Stop; how it
// ended; and Monday's line when Monday's note wasn't prepared.
//
// The run belongs to the app (WeeklyNoteModel): leaving This Week doesn't stop it. VoiceOver hears
// the stages and how a run ended while the card shows, never the note as it's written; a click's
// run moves the focus (to Stop, then the note or how it ended), Monday's never does.

import AppKit
import SwiftUI
import PageLampKit
import PageLampModel

struct WeeklyNoteSection: View {
    let note: WeeklyNoteModel
    /// This Week's minute (its `TimelineView`): what's due within 7 days, and the day, move with it.
    let now: Date

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @Environment(\.drawsControlStandIns) private var standIns
    @AccessibilityFocusState private var focus: Focus?

    enum Focus: Hashable {
        case stop
        case note
        case outcome
        case write
    }

    var body: some View {
        VStack(alignment: .leading, spacing: PLLayout.titleToRule) {
            SectionHeader(title: l10n("weeklyNote.title"))
            VStack(alignment: .leading, spacing: PLSpace.s4) {
                if let shown = note.shown {
                    WeeklyNoteView(
                        note: shown, text: text, deleting: note.deleting,
                        onDelete: { Task { await delete(shown) } }
                    )
                    .accessibilityFocused($focus, equals: .note)
                } else if note.loaded {
                    Text(l10n("weeklyNote.empty"))
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let failure = note.deleteFailure {
                    Text(l10n.sentences(l10n("weeklyNote.deleteFailed"), l10n.aiError(failure)))
                        .foregroundStyle(PLColor.danger)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let problem = note.automaticProblem, !note.run.isRunning {
                    Text(l10n("weeklyNote.automaticProblem", ["reason": l10n.aiError(problem)]))
                        .font(PLType.callout.font)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                controls
                outcome
                history
            }
        }
        .task(id: model.serviceGeneration) {
            // Offscreen the harness has read it already (a read in flight would show as loading).
            guard !standIns else { return }
            await note.load()
        }
        // Back from Settings ▸ AI (a model, Remove All AI Data) or the Tauri app: read again.
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { _ in
            Task { await note.refresh() }
        }
        // The week changed (a sync brought courses, a course was hidden, the plan changed): "≈ $x"
        // and "nothing to write about" are read again. Only the estimate: it never changes these.
        .onChange(of: noteWeekKey) {
            guard !standIns else { return }
            Task { await note.estimate.refresh() }
        }
        .onChange(of: announcement) { _, text in
            if let text { AccessibilityNotification.Announcement(text).post() }
        }
        .onChange(of: phaseName) { _, phase in
            // Monday's run never moves the focus.
            guard !note.automatic else { return }
            switch phase {
            case "running": focus = .stop
            case "finished": focus = .note
            case "stopped", "failed": focus = .outcome
            default: break
            }
        }
        .onChange(of: note.estimate.failure) { _, failure in
            if let failure { AccessibilityNotification.Announcement(l10n.aiError(failure)).post() }
        }
    }

    /// What the note writes about at `now` (`WeeklyNoteModel.weekKey`).
    private var noteWeekKey: [String] {
        WeeklyNoteModel.weekKey(
            courses: model.courses, deadlines: model.upcomingDeadlines, plan: model.studyPlan, now: now,
            calendar: model.calendar
        )
    }

    // MARK: - Write, or the run

    @ViewBuilder
    private var controls: some View {
        if case .running(let progress) = note.run.phase {
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
                    Text(text.headline(progress, automatic: note.automatic))
                        .font(PLType.body.font.weight(.semibold))
                }
                if progress.stage != nil {
                    // VoiceOver hears the stage once, as an announcement.
                    Text(text.stage(progress.stage))
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(true)
                }
                HStack(alignment: .firstTextBaseline, spacing: PLSpace.s3) {
                    Button(l10n(progress.stopping ? "weeklyNote.running.stopping" : "weeklyNote.running.stop")) {
                        Task { await note.stop() }
                    }
                    .buttonStyle(.bordered)
                    .disabled(progress.stopping)
                    .accessibilityHint(l10n("weeklyNote.running.leaveHint"))
                    .accessibilityFocused($focus, equals: .stop)
                    Text(l10n("weeklyNote.running.leaveHint"))
                        .font(PLType.callout.font)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityHidden(true)
                }
            }
        } else {
            VStack(alignment: .leading, spacing: PLSpace.s3) {
                EstimateView(estimate: note.estimate)
                // This Week has no tinted action (spec §1.2): a plain push button.
                Button(l10n(note.shown == nil ? "mac.weeklyNote.write" : "mac.weeklyNote.writeAgain")) {
                    Task { await note.write(uiLanguage: model.localization) }
                }
                .buttonStyle(.bordered)
                .disabled(!note.estimate.canGenerate)
                .accessibilityHint(note.estimate.spokenHint(l10n))
                .accessibilityFocused($focus, equals: .write)
            }
        }
    }

    /// How the last run ended, when it didn't write a note.
    @ViewBuilder
    private var outcome: some View {
        switch note.run.phase {
        case .stopped:
            Text(l10n("weeklyNote.result.stopped"))
                .accessibilityFocused($focus, equals: .outcome)
        case .failed(let failure):
            FailureBlock(title: l10n("weeklyNote.result.failed"), reason: l10n.aiError(failure))
                .accessibilityFocused($focus, equals: .outcome)
        default:
            EmptyView()
        }
    }

    // MARK: - Earlier notes

    @ViewBuilder
    private var history: some View {
        let list = note.history
        if list.count > 1 {
            let shownId = note.shown?.meta.generationId
            VStack(alignment: .leading, spacing: PLSpace.s2) {
                Text(l10n("weeklyNote.history.title"))
                    .font(PLType.headline.font)
                    .accessibilityAddTraits(.isHeader)
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(list.enumerated()), id: \.element.meta.generationId) { index, item in
                        if index > 0 { Divider() }
                        HStack(alignment: .firstTextBaseline, spacing: PLSpace.s2) {
                            Text(text.savedAt(item.meta.createdAt))
                                .monospacedDigit()
                            Text(verbatim: "·")
                                .foregroundStyle(.secondary)
                                .accessibilityHidden(true)
                            Text(item.meta.model)
                                .foregroundStyle(.secondary)
                            Spacer(minLength: PLSpace.s3)
                            if item.meta.generationId == shownId {
                                Text(l10n("weeklyNote.history.showing"))
                                    .font(PLType.callout.font)
                                    .foregroundStyle(.secondary)
                            } else {
                                Button(l10n("weeklyNote.history.show")) {
                                    note.show(item)
                                    focus = .note
                                }
                                .controlSize(.small)
                                .accessibilityLabel(l10n("mac.weeklyNote.showFrom", ["when": text.savedAt(item.meta.createdAt)]))
                            }
                        }
                        .padding(.vertical, PLSpace.s2)
                    }
                }
            }
        }
    }

    // MARK: - Delete

    private func delete(_ shown: WeeklyNote) async {
        if await note.delete(shown) {
            AccessibilityNotification.Announcement(l10n("weeklyNote.deleted")).post()
            focus = note.shown == nil ? .write : .note
        } else if let failure = note.deleteFailure {
            AccessibilityNotification.Announcement(l10n.sentences(l10n("weeklyNote.deleteFailed"), l10n.aiError(failure))).post()
        }
    }

    // MARK: - What VoiceOver hears

    private var text: NoteText {
        NoteText(l10n: l10n, calendar: model.calendar)
    }

    private var phaseName: String {
        switch note.run.phase {
        case .idle: "idle"
        case .running: "running"
        case .finished: "finished"
        case .stopped: "stopped"
        case .failed: "failed"
        }
    }

    /// The stages and how the run ended, never the note's text (design §7).
    private var announcement: String? {
        switch note.run.phase {
        case .running(let progress): text.stage(progress.stage)
        case .finished: l10n("weeklyNote.result.done")
        case .stopped: l10n("weeklyNote.result.stopped")
        case .failed(let failure): l10n.sentences(l10n("weeklyNote.result.failed"), l10n.aiError(failure))
        case .idle: nil
        }
    }
}

/// One note: its week, the text, the focus list (each item's course opens it), what was left out,
/// an earlier week's line, then the AI-generated line with Copy and Delete….
private struct WeeklyNoteView: View {
    let note: WeeklyNote
    let text: NoteText
    let deleting: Bool
    let onDelete: () -> Void

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @State private var confirmingDelete = false

    var body: some View {
        VStack(alignment: .leading, spacing: PLSpace.s3) {
            Text(text.description(note))
                .font(PLType.callout.font)
                .foregroundStyle(.secondary)
            Text(note.text)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .paragraphLineSpacing()
            if !note.focus.isEmpty {
                VStack(alignment: .leading, spacing: PLSpace.s1) {
                    Text(l10n("weeklyNote.focusTitle"))
                        .font(PLType.headline.font)
                        .accessibilityAddTraits(.isHeader)
                    ForEach(Array(note.focus.enumerated()), id: \.offset) { index, item in
                        HStack(alignment: .firstTextBaseline, spacing: PLSpace.s2) {
                            Text(l10n.number(index + 1) + ".")
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                            focusItem(item)
                        }
                    }
                }
            }
            if note.gradedWorkLeftOut > 0 {
                Label {
                    Text(text.gradedLeftOut(note.gradedWorkLeftOut))
                        .fixedSize(horizontal: false, vertical: true)
                } icon: {
                    Image(systemName: "exclamationmark.triangle")
                        .foregroundStyle(PLColor.warning)
                        .accessibilityHidden(true)
                }
                .font(PLType.callout.font)
                .foregroundStyle(.secondary)
            }
            if text.isEarlierWeek(note, now: model.clock()) {
                Text(l10n("weeklyNote.earlierWeek"))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
            }
            labelRow
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(text.regionLabel(note))
        .alert(l10n("weeklyNote.deleteTitle"), isPresented: $confirmingDelete) {
            Button(l10n("common.actions.cancel"), role: .cancel) {}
            Button(l10n("weeklyNote.deleteConfirm"), role: .destructive, action: onDelete)
        } message: {
            Text(l10n("weeklyNote.deleteBody"))
        }
    }

    /// "DEMO101: Get ahead on …", the course a link to its page; the text alone without one.
    @ViewBuilder
    private func focusItem(_ item: NoteFocus) -> some View {
        if let id = item.courseId, let course = model.course(id: id) {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
                Button(course.course.code ?? course.course.name) {
                    ThisWeekNavigation.open(courseId: id, model: model)
                }
                .linkButtonStyle()
                Text(verbatim: ": " + item.text)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
        } else {
            Text(item.text)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// The label with Copy and Delete… beside it; where the line doesn't fit, the buttons go under it.
    private var labelRow: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: PLSpace.s3) {
                aiLabel
                    .fixedSize()
                    .frame(maxWidth: .infinity, alignment: .leading)
                actions
            }
            VStack(alignment: .leading, spacing: PLSpace.s2) {
                aiLabel
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: PLSpace.s2) { actions }
            }
        }
    }

    /// The AI-generated line (Canvas §2E), under the note and in every copy.
    private var aiLabel: some View {
        Text(l10n.aiLabel(note.meta, calendar: model.calendar))
            .font(PLType.callout.font)
            .foregroundStyle(.secondary)
    }

    @ViewBuilder private var actions: some View {
        CopyButton(title: l10n("weeklyNote.copy"), text: text.copyText(note) { id in
            model.course(id: id).map { $0.course.code ?? $0.course.name }
        })
        .controlSize(.small)
        Button(l10n("weeklyNote.delete")) { confirmingDelete = true }
            .controlSize(.small)
            .disabled(deleting)
    }
}
