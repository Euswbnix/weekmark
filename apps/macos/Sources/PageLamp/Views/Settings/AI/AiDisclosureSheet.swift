// A backend's disclosure before its first run (model-access design §7; the Tauri app's
// DisclosureDialog; all six Canvas §2E items): generative AI, what is sent and to whom, training
// and storage, who else can see it, the cost, the age, the limits, and who owns the materials.
// Only the facade's DisclosureFacts are shown, never folded. Turning it on records the facts'
// version: when they change, the student is asked again. Read from the top (the title takes
// VoiceOver's focus); it scrolls inside the sheet.

import SwiftUI
import PageLampKit
import PageLampModel

package struct AiDisclosureSheet: View {
    let ai: AiSettingsModel
    let backend: AiBackendStatus
    let done: () -> Void

    @Environment(\.l10n) private var l10n
    @Environment(\.drawsControlStandIns) private var standIns
    @Environment(\.openURL) private var openURL
    @State private var ageConfirmed = false
    @State private var freeTierConfirmed = false
    @AccessibilityFocusState private var titleFocused: Bool

    package init(ai: AiSettingsModel, backend: AiBackendStatus, done: @escaping () -> Void) {
        self.ai = ai
        self.backend = backend
        self.done = done
    }

    package var body: some View {
        VStack(alignment: .leading, spacing: PLSpace.s4) {
            VStack(alignment: .leading, spacing: PLSpace.s1) {
                Text(l10n("ai.disclosure.title", ["name": backend.label]))
                    .font(PLType.title2.font.weight(.semibold))
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityFocused($titleFocused)
                Text(l10n("ai.disclosure.intro"))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                if backend.problems.contains(.disclosureChanged) {
                    Text(l10n("ai.disclosure.changed"))
                        .font(PLType.callout.font.weight(.semibold))
                }
            }
            // Offscreen (snapshots) a scroll view draws nothing: the sections at full height.
            if standIns {
                sections
            } else {
                ScrollView {
                    sections.padding(.trailing, PLSpace.s3)
                }
                .frame(maxHeight: Self.sectionsMaxHeight)
                .scrollBounceBehavior(.basedOnSize)
            }
            if !acknowledged {
                confirmations
            }
            footer
        }
        .padding(PLLayout.sheetInset)
        .frame(width: Self.width)
        .onAppear { titleFocused = true }
        .onDisappear { ai.clearAcknowledgeFailure() }
        .onExitCommand(perform: done)
    }

    // MARK: - The facts

    private var sections: some View {
        VStack(alignment: .leading, spacing: PLSpace.s4) {
            item(l10n("ai.disclosure.gai.heading")) {
                paragraph(l10n("ai.disclosure.gai.body"))
            }
            item(l10n("ai.disclosure.sent.heading")) {
                ForEach(facts.sends, id: \.self) { sent in
                    HStack(alignment: .firstTextBaseline, spacing: PLSpace.s2) {
                        Text(verbatim: "•").accessibilityHidden(true)
                        paragraph(l10n("ai.disclosure.sent.\(AiCodes.name(sent))"))
                    }
                }
                paragraph(sentTo)
                if let terms = facts.recipient.termsUrl {
                    link(l10n("ai.disclosure.sent.terms", ["name": who]), terms)
                }
            }
            item(l10n("ai.disclosure.training.heading")) {
                paragraph(l10n("ai.disclosure.training.\(AiCodes.name(facts.training))", ["name": who]))
                if case .mayTrain(let howToTurnOffUrl) = facts.training, let howToTurnOffUrl {
                    link(l10n("ai.disclosure.training.turnOff"), howToTurnOffUrl)
                }
                paragraph(retention)
            }
            if facts.adminVisibility != .no {
                item(l10n("ai.disclosure.admin.heading")) {
                    paragraph(l10n(facts.adminVisibility == .yes ? "ai.disclosure.admin.yes" : "ai.disclosure.admin.unknown"))
                }
            }
            item(l10n("ai.disclosure.cost.heading")) {
                paragraph(l10n("ai.disclosure.cost.\(AiCodes.name(facts.cost))", ["name": who]))
            }
            if let age = minAge {
                item(l10n("ai.disclosure.age.heading")) {
                    paragraph(l10n(
                        facts.guardianPermission ? "ai.disclosure.age.guardian" : "ai.disclosure.age.body",
                        ["name": who, "age": l10n.number(age)]
                    ))
                }
            }
            item(l10n("ai.disclosure.limits.heading")) {
                paragraph(l10n("ai.disclosure.limits.body"))
            }
            item(l10n("ai.disclosure.ownership.heading")) {
                paragraph(l10n("ai.disclosure.ownership.body"))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func item<Content: View>(_ heading: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: PLSpace.s1) {
            Text(heading)
                .font(PLType.headline.font)
                .accessibilityAddTraits(.isHeader)
            content()
        }
    }

    private func paragraph(_ text: String) -> some View {
        Text(text).fixedSize(horizontal: false, vertical: true)
    }

    /// An http(s) link opens in the browser; anything else is plain text.
    @ViewBuilder
    private func link(_ title: String, _ address: String) -> some View {
        if let url = URL(string: address), ["http", "https"].contains(url.scheme?.lowercased() ?? "") {
            if standIns {
                // Offscreen a link button draws nothing: its look, as a label.
                Label(title, systemImage: "arrow.up.forward.square")
                    .foregroundStyle(.tint)
            } else {
                Button {
                    openURL(url)
                } label: {
                    Label(title, systemImage: "arrow.up.forward.square")
                }
                .buttonStyle(.link)
            }
        } else {
            paragraph(title)
        }
    }

    private var sentTo: String {
        if facts.onDevice { return l10n("ai.disclosure.sent.onDevice", ["name": who]) }
        if backend.kind == .codex { return l10n("ai.disclosure.sent.codex", ["name": who]) }
        return l10n("ai.disclosure.sent.cloud", ["name": who])
    }

    private var retention: String {
        if case .storedDays(let days) = facts.retention {
            return l10n.plural("ai.disclosure.retention.stored_days", count: Int(days), ["name": who])
        }
        return l10n("ai.disclosure.retention.\(AiCodes.name(facts.retention))", ["name": who])
    }

    // MARK: - Turning it on

    @ViewBuilder
    private var confirmations: some View {
        VStack(alignment: .leading, spacing: PLSpace.s2) {
            Divider()
            if minAge != nil {
                Toggle(l10n("ai.disclosure.age.confirm", ["name": who]), isOn: $ageConfirmed)
                    .toggleStyle(.checkbox)
            }
            if needsFreeTier {
                Toggle(isOn: $freeTierConfirmed) {
                    Text(l10n("ai.disclosure.freeTier.confirm", ["name": who]))
                        .fixedSize(horizontal: false, vertical: true)
                }
                .toggleStyle(.checkbox)
            }
            if !canAccept {
                Text(l10n("ai.disclosure.confirmFirst"))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
            }
            if let failure = ai.acknowledgeFailure {
                Text(l10n.aiError(failure))
                    .font(PLType.callout.font)
                    .foregroundStyle(PLColor.danger)
            }
        }
    }

    @ViewBuilder
    private var footer: some View {
        HStack(spacing: PLSpace.s2) {
            Spacer()
            if acknowledged {
                Button(l10n("ai.disclosure.close"), action: done)
                    .keyboardShortcut(.defaultAction)
            } else {
                Button(l10n("mac.ai.notNow"), action: done)
                    .keyboardShortcut(.cancelAction)
                Button(l10n("mac.ai.turnOnName", ["name": backend.label])) {
                    Task {
                        if await ai.acknowledge(backend) {
                            AccessibilityNotification.Announcement(
                                l10n("ai.disclosure.accepted", ["name": backend.label])
                            ).post()
                            done()
                        }
                    }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!canAccept || ai.acknowledging)
                .accessibilityHint(canAccept ? "" : l10n("ai.disclosure.confirmFirst"))
            }
        }
    }

    // MARK: - Rules (the Tauri app's)

    private var facts: DisclosureFacts { backend.disclosure }

    /// Whose terms apply (e.g. "OpenAI"); the title and buttons use the student's label.
    private var who: String { facts.recipient.name }

    private var acknowledged: Bool { backend.disclosureAcknowledged == facts.version }

    /// The age to confirm (none when the facts give 0 or nothing).
    private var minAge: UInt8? {
        guard let age = facts.minAge, age > 0 else { return nil }
        return age
    }

    private var needsFreeTier: Bool {
        if case .mayTrainFreeTier = facts.training { true } else { false }
    }

    private var canAccept: Bool {
        (minAge == nil || ageConfirmed) && (!needsFreeTier || freeTierConfirmed)
    }

    package static let width: CGFloat = 520
    private static let sectionsMaxHeight: CGFloat = 420
}
