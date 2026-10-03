// "Add an API Key…" / "Replace Key…" (the Tauri app's ApiKeyDialog): one form, not steps. The
// provider (presets that take a key) with its data policy, the address where the preset lets the
// student change it, and the key. The facade checks the key with a free call before keeping it
// in the keychain; this sheet only ever shows its last 4 characters afterwards.
//
// The key lives in the secure field's state only: it goes to the facade once on submit and the
// field is cleared right after (whatever the answer) and when the sheet closes. It is never kept
// in a model, UserDefaults, logs or snapshots. While the key is being checked the sheet stays:
// a check can't be called back, so closing it would still save the key out of sight.

import SwiftUI
import PageLampKit
import PageLampModel

package struct AiKeySheet: View {
    let ai: AiSettingsModel
    let mode: KeySheet
    /// Closed: the new provider when one was added (its disclosure comes next), else nil.
    let done: (ModelProviderRecord?) -> Void

    @Environment(\.l10n) private var l10n
    /// Offscreen renders can't draw text fields: the snapshot harness gets stand-ins.
    @Environment(\.drawsControlStandIns) private var standIns
    @State private var presetId: String?
    @State private var baseUrl = ""
    @State private var key = ""
    /// The last submit found no key.
    @State private var missing = false
    @FocusState private var keyFocused: Bool
    @AccessibilityFocusState private var titleFocused: Bool

    package init(ai: AiSettingsModel, mode: KeySheet, done: @escaping (ModelProviderRecord?) -> Void) {
        self.ai = ai
        self.mode = mode
        self.done = done
    }

    package var body: some View {
        VStack(alignment: .leading, spacing: PLSpace.s4) {
            VStack(alignment: .leading, spacing: PLSpace.s1) {
                Text(title)
                    .font(PLType.title2.font.weight(.semibold))
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityFocused($titleFocused)
                Text(l10n("ai.addKey.description"))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Form {
                if case .add = mode {
                    providerFields
                }
                keyField
            }
            .formStyle(.columns)
            if let failure = ai.keyFailure {
                FailureNote(failure: failure)
            }
            HStack(spacing: PLSpace.s2) {
                if ai.addingKey {
                    ProgressView()
                        .controlSize(.small)
                        .accessibilityHidden(true)
                    Text(l10n("ai.addKey.checking"))
                        .font(PLType.callout.font)
                        .foregroundStyle(.secondary)
                }
                Spacer()
                Button(l10n("common.actions.cancel"), action: cancel)
                    .keyboardShortcut(.cancelAction)
                    .disabled(ai.addingKey)
                Button(submitTitle) { Task { await submit() } }
                    .keyboardShortcut(.defaultAction)
                    .disabled(ai.addingKey || (isAdding && preset == nil))
            }
        }
        .padding(PLLayout.sheetInset)
        .frame(width: Self.width)
        .accessibilityElement(children: .contain)
        .onAppear { titleFocused = true }
        .onDisappear {
            key = ""
            ai.clearKeyFailure()
        }
        .onExitCommand(perform: cancel)
        .interactiveDismissDisabled(ai.addingKey)
    }

    // MARK: - Fields

    @ViewBuilder
    private var providerFields: some View {
        Picker(l10n("ai.addKey.provider"), selection: Binding(get: { preset?.id }, set: { presetId = $0 })) {
            ForEach(ai.keyPresets, id: \.id) { preset in
                Text(preset.label).tag(Optional(preset.id))
            }
        }
        if let preset {
            Text(l10n.dataPolicy(preset.dataPolicy))
                .font(PLType.callout.font)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityLabel(l10n.labelled(l10n("ai.policy.label"), l10n.dataPolicy(preset.dataPolicy)))
            if preset.baseUrlEditable {
                if standIns {
                    LabeledContent(l10n("ai.addKey.baseUrl")) {
                        FieldStandIn(text: baseUrl, prompt: preset.defaultBaseUrl ?? "https://")
                    }
                } else {
                    TextField(l10n("ai.addKey.baseUrl"), text: $baseUrl, prompt: Text(verbatim: preset.defaultBaseUrl ?? "https://"))
                        .autocorrectionDisabled()
                }
                Text(l10n("ai.addKey.baseUrlHint"))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private var keyField: some View {
        if standIns {
            // Always empty: a render never shows a key.
            LabeledContent(l10n("ai.addKey.key")) {
                FieldStandIn(text: "")
            }
        } else {
            SecureField(l10n("ai.addKey.key"), text: $key)
                .focused($keyFocused)
                .accessibilityHint(missing ? l10n("common.errors.invalid") : l10n("ai.addKey.keyHint"))
        }
        if missing {
            Text(l10n("common.errors.invalid"))
                .font(PLType.callout.font)
                .foregroundStyle(PLColor.danger)
        }
        Text(l10n("ai.addKey.keyHint"))
            .font(PLType.callout.font)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    // MARK: - Actions

    private func submit() async {
        guard !ai.addingKey else { return }
        let entered = key
        // The key leaves the field now: only the facade gets it.
        key = ""
        missing = entered.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        guard !missing else {
            keyFocused = true
            return
        }
        switch mode {
        case .add:
            guard let preset else { return }
            if let record = await ai.addProvider(preset: preset, baseUrl: baseUrl, key: entered) {
                done(record)
            } else {
                announceFailure()
            }
        case .replace(let provider):
            if await ai.replaceKey(providerId: provider.providerId, key: entered) {
                AccessibilityNotification.Announcement(l10n("ai.replaceKey.done")).post()
                done(nil)
            } else {
                announceFailure()
            }
        }
    }

    private func cancel() {
        guard !ai.addingKey else { return }
        key = ""
        done(nil)
    }

    /// The failure is read out, and the key field is ready for another try.
    private func announceFailure() {
        if let failure = ai.keyFailure {
            AccessibilityNotification.Announcement(FailureNote.headline(failure, l10n)).post()
        }
        keyFocused = true
    }

    // MARK: - Helpers

    private var isAdding: Bool {
        if case .add = mode { true } else { false }
    }

    /// The chosen preset; the first until the student picks one.
    private var preset: ProviderPreset? {
        ai.keyPresets.first { $0.id == presetId } ?? ai.keyPresets.first
    }

    private var title: String {
        switch mode {
        case .add: l10n("ai.addKey.title")
        case .replace(let provider): l10n("ai.replaceKey.title", ["name": provider.label])
        }
    }

    private var submitTitle: String {
        isAdding ? l10n("mac.ai.checkAndAdd") : l10n("mac.ai.checkAndReplace")
    }

    package static let width: CGFloat = 480
}

/// Why the key wasn't taken: a coding-plan key with the provider's own words (quoted, English),
/// else the failure by its code with the service's message as an English detail.
private struct FailureNote: View {
    let failure: PageLampFailure
    @Environment(\.l10n) private var l10n

    var body: some View {
        VStack(alignment: .leading, spacing: PLSpace.s1) {
            Label {
                Text(Self.headline(failure, l10n))
                    .fixedSize(horizontal: false, vertical: true)
            } icon: {
                Image(systemName: "exclamationmark.triangle")
                    .accessibilityHidden(true)
            }
            .foregroundStyle(PLColor.danger)
            let message = failure.message.trimmingCharacters(in: .whitespacesAndNewlines)
            if !message.isEmpty {
                if isCodingPlan {
                    Text(l10n("ai.addKey.vendorSays"))
                        .font(PLType.callout.font)
                }
                // The provider's own words, in English (read with an English voice).
                Text.english(message)
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.leading, isCodingPlan ? PLSpace.s3 : 0)
                    .overlay(alignment: .leading) {
                        if isCodingPlan {
                            Rectangle().fill(.separator).frame(width: 2)
                        }
                    }
                    .textSelection(.enabled)
            }
        }
        .accessibilityElement(children: .combine)
    }

    private var isCodingPlan: Bool {
        failure.blocked == .codingPlanKey
    }

    static func headline(_ failure: PageLampFailure, _ l10n: L10n) -> String {
        failure.blocked == .codingPlanKey ? l10n("ai.addKey.codingPlan") : l10n.aiError(failure)
    }
}
