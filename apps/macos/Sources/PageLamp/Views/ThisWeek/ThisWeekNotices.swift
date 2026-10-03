// This Week's notices and empty states (spec §3.9): S7 source problems as callouts under the
// band, and the whole-page empty states S3 (no sources), S4 (no courses) and the M1 part of S5
// (courses on their way while the first sync runs).

import SwiftUI
import PageLampKit
import PageLampModel

/// S7: one callout per source whose last sync failed, so stale courses are explained. A
/// rejected token or feed address gets its own words. The fix itself is the capsule's tinted
/// bubble (and, from M2, the Replace sheet); here "Open Sources & Sync" leads to the source,
/// scrolled into view and highlighted, never a second Replace button.
struct ThisWeekSourceProblems: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: PLSpace.s3) {
            ForEach(model.failingSources, id: \.id) { source in
                SourceProblemCallout(source: source)
            }
        }
    }
}

private struct SourceProblemCallout: View {
    let source: SourceRecord

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n

    var body: some View {
        let problem = SourceProblem(source: source) ?? .failed(.other)
        Callout(
            tone: problem.fix == nil ? .warning : .danger,
            symbol: problem.fix == nil ? nil : "key",
            title: title(problem),
            message: l10n(message(problem))
        ) {
            Button(l10n("mac.actions.openSourcesAndSync")) {
                model.showSource(source.id)
            }
            .linkButtonStyle()
        }
    }

    private func title(_ problem: SourceProblem) -> String {
        switch problem {
        case .expired(.replaceToken):
            l10n("courses.problems.tokenExpiredTitle", ["source": source.label])
        case .expired(.replaceFeed):
            l10n("courses.problems.feedRejectedTitle", ["source": source.label])
        case .failed(.other):
            // "other" has no useful reason to name ("…couldn't sync: Error").
            l10n("courses.problems.failedTitleGeneric", ["source": source.label])
        case .failed(let kind):
            l10n("courses.problems.failedTitle", ["source": source.label, "reason": l10n.sourceError(kind)])
        }
    }

    private func message(_ problem: SourceProblem) -> String {
        switch problem {
        case .expired(.replaceToken): "courses.problems.tokenExpired"
        case .expired(.replaceFeed): "courses.problems.feedRejected"
        case .failed: "courses.problems.failed"
        }
    }
}

/// S3 / S4 / first sync: the whole detail column explains why there's nothing to read yet.
struct ThisWeekEmptyPage: View {
    let state: ThisWeekPageState

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n

    var body: some View {
        VStack(spacing: 0) {
            if model.lastCrash != nil || !model.failingSources.isEmpty || hasReminderNotes {
                ReadingColumn {
                    if let crash = model.lastCrash {
                        CrashNotice(crash: crash)
                    }
                    if !model.failingSources.isEmpty {
                        ThisWeekSourceProblems()
                    }
                    RemindersCatchUp()
                    if state == .firstSync {
                        RemindMeQuestion()
                    }
                }
                .padding(.top, PLSpace.s6)
            }
            ContentUnavailableView {
                Label {
                    Text(title)
                } icon: {
                    Image(systemName: symbol)
                        .font(.system(size: glyphSize))
                        .symbolRenderingMode(.hierarchical)
                        .foregroundStyle(.tertiary)
                }
            } description: {
                Text(message)
                    .paragraphLineSpacing()
            } actions: {
                actions
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(minHeight: PLSize.windowMainMinHeight)
    }

    /// The catch-up card, or "Remind me" while the first sync runs.
    private var hasReminderNotes: Bool {
        guard let delivery = model.reminderDelivery else { return false }
        return !delivery.catchUp.isEmpty
            || (state == .firstSync && (delivery.consent == false || delivery.questionAnswer != nil))
    }

    /// The lamp stays unlit until there is something to light (spec §4.5); errors never glow.
    private var symbol: String {
        switch state {
        case .noCourses: "tray"
        case .noSources, .firstSync, .page: "lamp.desk"
        }
    }

    private var glyphSize: CGFloat {
        symbol == "lamp.desk" ? PLSize.glyphLamp : PLSize.glyphEmpty
    }

    private var title: String {
        switch state {
        case .firstSync: l10n("courses.empty.syncingTitle")
        case .noSources, .noCourses, .page: l10n("courses.empty.title")
        }
    }

    private var message: String {
        switch state {
        case .firstSync: l10n("courses.empty.syncingDescription")
        case .noCourses: l10n("courses.empty.descriptionHasSources")
        case .noSources, .page: l10n("courses.empty.description")
        }
    }

    /// S3: Add a Source… (primary). S4: Sync Now (primary) and Add Source…. The Add Source
    /// sheet and the welcome window (Set Up PageLamp…) are M2: in M1 both lead to Sources & Sync.
    @ViewBuilder private var actions: some View {
        switch state {
        case .noSources:
            Button(l10n("mac.actions.addASource")) {
                model.destination = .sources
            }
            .arbitratedButtonStyle(.pagePrimary)
        case .noCourses:
            HStack(spacing: PLSpace.s3) {
                Button(l10n("mac.actions.syncNow")) {
                    Task { await model.syncAll() }
                }
                .arbitratedButtonStyle(.pagePrimary)
                .disabled(!model.canSync)
                Button(l10n("mac.actions.addSource")) {
                    model.destination = .sources
                }
                .buttonStyle(.bordered)
            }
        case .firstSync, .page:
            EmptyView()
        }
    }
}

/// S6: the panic hook recorded a crash of the app or of the MCP server an AI app started. The
/// first callout under the band until dismissed (Dismiss clears the record in the core).
struct CrashNotice: View {
    let crash: CrashReport

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @State private var dismissing = false

    var body: some View {
        Callout(
            tone: .warning,
            title: crash.process == .mcp
                ? l10n("common.diagnostics.crash.titleMcp")
                : l10n("common.diagnostics.crash.titleApp"),
            message: l10n("common.diagnostics.crash.body", [
                "when": l10n.relative(crash.time, to: model.clock(), calendar: model.calendar),
            ])
        ) {
            VStack(alignment: .leading, spacing: PLSpace.s2) {
                HStack(spacing: PLSpace.s2) {
                    Button(l10n("mac.actions.copyDiagnosticReport")) {
                        Task { await model.showDiagnosticReport(in: .main) }
                    }
                    .buttonStyle(.bordered)
                    if BrandLinks.issues != nil {
                        Button(l10n("mac.actions.reportProblem")) { AppActions.open(BrandLinks.issues) }
                            .buttonStyle(.bordered)
                    }
                    Button(l10n("common.diagnostics.crash.dismiss")) {
                        dismissing = true
                        Task {
                            await model.dismissCrash()
                            dismissing = false
                        }
                    }
                    .linkButtonStyle()
                    .disabled(dismissing)
                }
                if let failure = model.crashDismissFailure {
                    Text(l10n("common.diagnostics.crash.dismissFailed") + l10n("common.punctuation.colon")
                        + failure.localizedDescription(in: l10n))
                        .font(PLType.callout.font)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }
}
