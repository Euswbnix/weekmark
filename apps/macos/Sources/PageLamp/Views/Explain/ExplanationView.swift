// One explanation (the Tauri app's ExplanationView): the AI-generated line with Copy and
// Delete…, then the sections, each paragraph with its sources (a chip opens the material: its
// file on this computer, else its link); citations the facade removed; the check questions;
// what wasn't read and why, with "Include it and write again" for what only looks like graded
// work; and the course's "cite AI use" note. Copy puts the text with its sources and the label
// on the clipboard.

import AppKit
import SwiftUI
import PageLampKit
import PageLampModel

struct ExplanationView: View {
    let explanation: WeeklyExplanation
    let text: ExplainText
    let deleting: Bool
    /// "≈ $x" of Include It and Write Again, priced with what it sends (off without it, or while
    /// running).
    let includeEstimate: CostEstimateModel
    let running: Bool
    /// "Use It Anyway" on Include It's line worked: every line of the section reads again.
    let onAcknowledged: @MainActor () async -> Void
    let onInclude: () -> Void
    let onDelete: () -> Void

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @Environment(\.openURL) private var openURL
    @State private var confirmingDelete = false

    var body: some View {
        VStack(alignment: .leading, spacing: PLSpace.s4) {
            labelRow
            ForEach(Array(explanation.sections.enumerated()), id: \.offset) { _, section in
                VStack(alignment: .leading, spacing: PLSpace.s2) {
                    Text(section.heading)
                        .font(PLType.headline.font)
                        .accessibilityAddTraits(.isHeader)
                    ForEach(Array(section.paragraphs.enumerated()), id: \.offset) { _, paragraph in
                        VStack(alignment: .leading, spacing: PLSpace.s1) {
                            MarkdownText(paragraph.text)
                                .textSelection(.enabled)
                                .fixedSize(horizontal: false, vertical: true)
                                .paragraphLineSpacing()
                            if !paragraph.citations.isEmpty {
                                ChipFlow(spacing: PLSpace.s1) {
                                    ForEach(Array(paragraph.citations.enumerated()), id: \.offset) { _, citation in
                                        Button {
                                            Task { await open(citation) }
                                        } label: {
                                            // A long title gives way in the middle: the page stays.
                                            Label(text.citation(citation), systemImage: "doc.text")
                                                .lineLimit(1)
                                                .truncationMode(.middle)
                                        }
                                        .buttonStyle(.bordered)
                                        .controlSize(.small)
                                    }
                                }
                                .accessibilityElement(children: .contain)
                                .accessibilityLabel(l10n("explain.result.sourcesLabel"))
                            }
                        }
                    }
                }
            }
            if explanation.droppedCitations > 0 {
                Label {
                    Text(text.droppedCitations(explanation.droppedCitations))
                        .fixedSize(horizontal: false, vertical: true)
                } icon: {
                    Image(systemName: "exclamationmark.triangle")
                        .foregroundStyle(PLColor.warning)
                        .accessibilityHidden(true)
                }
                .font(PLType.callout.font)
            }
            if !explanation.checkQuestions.isEmpty {
                VStack(alignment: .leading, spacing: PLSpace.s1) {
                    Text(l10n("explain.result.questions"))
                        .font(PLType.headline.font)
                        .accessibilityAddTraits(.isHeader)
                    ForEach(Array(explanation.checkQuestions.enumerated()), id: \.offset) { index, question in
                        HStack(alignment: .firstTextBaseline, spacing: PLSpace.s2) {
                            Text(l10n.number(index + 1) + ".")
                                .monospacedDigit()
                                .foregroundStyle(.secondary)
                            MarkdownText(question)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }
            if !explanation.leftOut.isEmpty {
                leftOut
            }
            if explanation.citeAiUse {
                Text(l10n("explain.result.citeAiUse"))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .alert(l10n("explain.result.deleteTitle"), isPresented: $confirmingDelete) {
            Button(l10n("common.actions.cancel"), role: .cancel) {}
            Button(l10n("explain.result.deleteConfirm"), role: .destructive, action: onDelete)
        } message: {
            Text(l10n("explain.result.deleteBody"))
        }
    }

    // MARK: - The label, Copy and Delete

    /// The label with Copy and Delete… beside it; where the line doesn't fit, the buttons go
    /// under it (the label never wraps into a narrow column).
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

    /// The AI-generated line (Canvas §2E), above the explanation and in every copy.
    private var aiLabel: some View {
        Text(l10n.aiLabel(explanation.meta, calendar: model.calendar))
            .font(PLType.callout.font)
            .foregroundStyle(.secondary)
    }

    @ViewBuilder private var actions: some View {
        CopyButton(title: l10n("explain.result.copy"), text: text.copyText(explanation))
            .controlSize(.small)
        Button(l10n("explain.result.delete")) { confirmingDelete = true }
            .controlSize(.small)
            .disabled(deleting)
    }

    // MARK: - Not read this time

    private var leftOut: some View {
        let includable = explanation.leftOut.filter(\.includable)
        return VStack(alignment: .leading, spacing: PLSpace.s1) {
            Text(l10n("explain.result.leftOut"))
                .font(PLType.headline.font)
                .accessibilityAddTraits(.isHeader)
            ForEach(Array(explanation.leftOut.enumerated()), id: \.offset) { _, material in
                InlineText.joined([
                    Text(material.title),
                    // " (reason)" / "（原因）": the space is the language's.
                    Text(l10n("mac.explain.reason", ["reason": text.reason(material.reason)])).foregroundStyle(.secondary),
                ])
                .fixedSize(horizontal: false, vertical: true)
            }
            if !includable.isEmpty {
                VStack(alignment: .leading, spacing: PLSpace.s1) {
                    // "≈ $x" before a run: Include It is one.
                    EstimateView(estimate: includeEstimate, onAcknowledged: onAcknowledged)
                    Button(text.include(includable.count)) {
                        onInclude()
                    }
                    .controlSize(.small)
                    .disabled(!includeEstimate.canGenerate || running)
                    .accessibilityHint(l10n.sentences(l10n("explain.result.includeNote"), includeEstimate.spokenHint(l10n)))
                    Text(l10n("explain.result.includeNote"))
                        .font(PLType.callout.font)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityHidden(true)
                }
                .padding(.top, PLSpace.s1)
            }
        }
    }

    // MARK: - Opening a source

    /// The material's file on this computer (an app or script is shown in Finder, never run),
    /// else its link; neither: says so.
    private func open(_ citation: Citation) async {
        let path: String? = (try? await model.service.materialLocalFile(materialId: citation.materialId, purpose: .open)) ?? nil
        if let path {
            Links.open(.file(URL(filePath: path)), openURL: openURL)
        } else if let link = CourseLink(citation.url) {
            Links.open(link, openURL: openURL)
        } else {
            NSSound.beep()
            AccessibilityNotification.Announcement(l10n("explain.result.notOnComputer", ["title": citation.title])).post()
        }
    }
}

/// An explanation's Markdown subset (InlineMarkdown): strong in semibold, emphasis in medium (the
/// Mac's type rules have no italics), code in the monospaced face; everything else as written.
struct MarkdownText: View {
    let source: String

    init(_ source: String) {
        self.source = source
    }

    var body: some View {
        Text(attributed)
    }

    private var attributed: AttributedString {
        var result = AttributedString()
        for run in InlineMarkdown.runs(source) {
            var piece = AttributedString(run.text)
            switch run.style {
            case .plain: break
            case .strong: piece.font = PLType.body.font.weight(.semibold)
            case .emphasis: piece.font = PLType.body.font.weight(.medium)
            case .code: piece.font = PLType.body.font.monospaced()
            }
            result.append(piece)
        }
        return result
    }
}

/// Chips left to right, wrapping to the next line when the row is full; a chip wider than the
/// row is given the row's width (its title then truncates).
struct ChipFlow: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let rows = rows(subviews, width: proposal.width ?? .infinity)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(0, rows.count - 1))
        return CGSize(width: proposal.width ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in rows(subviews, width: bounds.width) {
            var x = bounds.minX
            for (index, size) in zip(row.indices, row.sizes) {
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + spacing
        }
    }

    private struct Row {
        var indices: [Int] = []
        var sizes: [CGSize] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    /// A chip's size: its one-line width, else (wider than the row) measured at the row's width.
    private func size(of subview: LayoutSubview, within width: CGFloat) -> CGSize {
        let ideal = subview.sizeThatFits(.unspecified)
        guard ideal.width > width, width.isFinite else { return ideal }
        let fitted = subview.sizeThatFits(ProposedViewSize(width: width, height: nil))
        return CGSize(width: min(fitted.width, width), height: fitted.height)
    }

    private func rows(_ subviews: Subviews, width: CGFloat) -> [Row] {
        var rows: [Row] = []
        var row = Row()
        for index in subviews.indices {
            let size = size(of: subviews[index], within: width)
            let needed = row.indices.isEmpty ? size.width : row.width + spacing + size.width
            if needed > width, !row.indices.isEmpty {
                rows.append(row)
                row = Row()
            }
            row.width = row.indices.isEmpty ? size.width : row.width + spacing + size.width
            row.height = max(row.height, size.height)
            row.indices.append(index)
            row.sizes.append(size)
        }
        if !row.indices.isEmpty { rows.append(row) }
        return rows
    }
}
