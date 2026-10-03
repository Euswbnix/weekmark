// "Which model does what" (the Tauri app's FeatureModels): for each feature, in the facade's
// order, a model from the backends that are set up (grouped by backend, each marked where it runs
// and whether it has a price), an effort, and Test (one small real call). Notes say when the
// chosen model has no price, always thinks, or runs in the cloud.

import SwiftUI
import PageLampKit
import PageLampModel

struct AiFeatureModelsSection: View {
    let ai: AiSettingsModel

    @Environment(\.l10n) private var l10n

    var body: some View {
        Section {
            if ai.usableBackends.isEmpty {
                Text(l10n("ai.features.turnOnFirst"))
                    .foregroundStyle(.secondary)
            } else {
                ForEach(ai.status?.features ?? [], id: \.feature) { routing in
                    AiFeatureRow(ai: ai, feature: routing.feature)
                }
                ForEach(ai.usableBackends, id: \.backend) { backend in
                    if let failure = ai.modelFailures[AiCodes.key(backend.backend)] {
                        Text(l10n.sentences(l10n("ai.features.modelsFailed", ["name": backend.label]), l10n.aiError(failure)))
                            .foregroundStyle(PLColor.danger)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        } header: {
            Text(l10n("ai.features.title"))
        } footer: {
            Text(l10n("ai.features.hint"))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// One feature: its model, effort, Test, the notes and the last result.
private struct AiFeatureRow: View {
    let ai: AiSettingsModel
    let feature: AiFeature

    @Environment(\.l10n) private var l10n

    var body: some View {
        let name = l10n.aiFeature(feature)
        let choice = ai.choice(for: feature)
        VStack(alignment: .leading, spacing: PLSpace.s1) {
            LabeledContent(name) {
                VStack(alignment: .trailing, spacing: PLSpace.s1) {
                    Picker(l10n("ai.features.model", ["feature": name]), selection: modelSelection) {
                        Text(l10n("ai.features.notSet")).tag("")
                        ForEach(ai.usableBackends, id: \.backend) { backend in
                            Section(groupTitle(backend)) {
                                ForEach(options(backend, choice: choice), id: \.tag) { option in
                                    Text(option.title).tag(option.tag)
                                }
                            }
                        }
                    }
                    .labelsHidden()
                    .frame(width: Self.modelWidth)
                    HStack(spacing: PLSpace.s2) {
                        Picker(l10n("ai.features.effort", ["feature": name]), selection: effortSelection) {
                            ForEach(Self.efforts, id: \.self) { effort in
                                Text(l10n("ai.features.effortName.\(AiCodes.name(effort))")).tag(effort)
                            }
                        }
                        .labelsHidden()
                        .frame(width: Self.effortWidth)
                        .disabled(choice == nil)
                        Button(ai.testing.contains(feature) ? l10n("ai.features.testing") : l10n("ai.features.test")) {
                            Task {
                                await ai.test(feature)
                                if let result = resultLine {
                                    AccessibilityNotification.Announcement(result.text).post()
                                }
                            }
                        }
                        .disabled(choice == nil || ai.testing.contains(feature))
                        .accessibilityHint(name)
                    }
                }
            }
            ForEach(notes, id: \.self) { note in
                Text(note)
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let result = resultLine {
                Text(result.text)
                    .font(PLType.callout.font)
                    .foregroundStyle(result.failed ? AnyShapeStyle(PLColor.danger) : AnyShapeStyle(.secondary))
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    // MARK: - Choosing

    /// "<backend key>|<model id>", or "" for none (a backend key has no "|"; a model id may).
    private var modelSelection: Binding<String> {
        Binding(
            get: { ai.choice(for: feature).map { Self.tag(AiCodes.key($0.backend), $0.model) } ?? "" },
            set: { value in
                guard let bar = value.firstIndex(of: "|"),
                      let backend = ai.backend(key: String(value[..<bar]))
                else {
                    Task { await ai.setModel(feature, backend: nil, model: nil) }
                    return
                }
                let model = String(value[value.index(after: bar)...])
                Task { await ai.setModel(feature, backend: backend.backend, model: model) }
            }
        )
    }

    private var effortSelection: Binding<Effort> {
        Binding(
            get: { ai.choice(for: feature)?.effort ?? .lowest },
            set: { effort in Task { await ai.setEffort(feature, effort) } }
        )
    }

    private struct Option {
        let tag: String
        let title: String
    }

    /// A backend's models with their badges; the saved model too while it isn't listed (the list
    /// is loading, or the model was withdrawn), so the picker can show it.
    private func options(_ backend: AiBackendStatus, choice: ModelChoice?) -> [Option] {
        let key = AiCodes.key(backend.backend)
        var options = (ai.models[key] ?? []).map { model in
            Option(tag: Self.tag(key, model.id), title: title(model.id, badges: badges(model, backend: backend)))
        }
        if let choice, AiCodes.key(choice.backend) == key, !options.contains(where: { $0.tag == Self.tag(key, choice.model) }) {
            let badges = backend.kind == .local ? [l10n("ai.features.badge.onDevice")] : []
            options.append(Option(tag: Self.tag(key, choice.model), title: title(choice.model, badges: badges)))
        }
        return options
    }

    private func badges(_ model: ModelInfo, backend: AiBackendStatus) -> [String] {
        var badges: [String] = []
        if model.runsInCloud {
            badges.append(l10n("ai.features.badge.cloud"))
        } else if model.onDevice {
            badges.append(l10n("ai.features.badge.onDevice"))
        }
        if backend.kind == .apiKey, !model.priceKnown {
            badges.append(l10n("ai.features.badge.noPrice"))
        }
        if model.suggestedFor.contains(feature) {
            badges.append(l10n("ai.features.badge.suggested"))
        }
        return badges
    }

    /// "gpt-6-luna (suggested)".
    private func title(_ id: String, badges: [String]) -> String {
        badges.isEmpty ? id : "\(id) (\(badges.joined(separator: ", ")))"
    }

    private func groupTitle(_ backend: AiBackendStatus) -> String {
        guard backend.state == .needsDisclosure else { return backend.label }
        return "\(backend.label) · \(l10n("ai.backend.state.needs_disclosure"))"
    }

    // MARK: - Notes and result

    /// Only for a chosen model in its backend's loaded list.
    private var notes: [String] {
        guard let choice = ai.choice(for: feature), let model = ai.chosenModel(for: feature),
              let backend = ai.backend(key: AiCodes.key(choice.backend))
        else { return [] }
        var notes: [String] = []
        if backend.kind == .apiKey, !model.priceKnown {
            notes.append(l10n("ai.features.noPriceNote", ["model": model.id]))
        }
        if model.reasoningAlwaysOn {
            notes.append(l10n("ai.features.thinkingNote", ["model": model.id]))
        }
        if model.runsInCloud {
            notes.append(l10n("ai.features.cloudNote", ["model": model.id]))
        }
        return notes
    }

    private var resultLine: (text: String, failed: Bool)? {
        if let failure = ai.modelChoiceFailures[feature] {
            return (l10n.aiError(failure), true)
        }
        switch ai.tests[feature] {
        case .report(let report)?:
            if report.ok {
                let seconds = (Double(report.latencyMs) / 1000)
                    .formatted(.number.precision(.fractionLength(1)).locale(l10n.locale))
                return (l10n("ai.features.testOk", ["seconds": seconds]), false)
            }
            if let error = report.error {
                return (l10n("ai.modelError.\(AiCodes.name(error))"), true)
            }
            return nil
        case .failed(let failure)?:
            return (l10n.aiError(failure), true)
        case nil:
            return nil
        }
    }

    // MARK: - Constants

    private static func tag(_ backendKey: String, _ model: String) -> String {
        "\(backendKey)|\(model)"
    }

    private static let efforts: [Effort] = [.lowest, .low, .medium, .high]
    private static let modelWidth: CGFloat = 260
    private static let effortWidth: CGFloat = 110
}
