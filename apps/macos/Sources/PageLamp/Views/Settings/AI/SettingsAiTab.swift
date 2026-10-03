// Settings ▸ AI (model-access design §7; the Tauri app's Settings → AI models, output language and
// usage; preview builds until it ships): the student's models in priority order with their state,
// problems and data policy; "Add an API Key…"; models on this computer; which model does what,
// with effort and Test; the disclosure before a backend's first run; the monthly budget; the
// answers' language; Monday's weekly note (where the note is on); usage per month; Remove All AI
// Data. Everything comes from the facade
// (`AiSettingsModel`); the ChatGPT plan isn't offered, so it has no card here.

import AppKit
import SwiftUI
import PageLampKit
import PageLampModel

struct SettingsAiTab: View {
    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @State private var ai: AiSettingsModel?
    /// The service `ai` reads (`AppModel.serviceGeneration`).
    @State private var loadedFor: Int?
    /// The key sheet: nil, or adding a key, or replacing one provider's.
    @State private var keySheet: KeySheet?
    /// The backend whose disclosure is shown (by key).
    @State private var disclosureFor: DisclosureRequest?
    /// The disclosure to show once the key sheet has closed (one sheet at a time).
    @State private var pendingDisclosure: DisclosureRequest?
    @State private var removing: ModelProviderRecord?
    @State private var confirmingRemoveAll = false
    /// A backend's row takes VoiceOver's focus once its disclosure closes after it was added.
    @AccessibilityFocusState private var focusedBackend: String?

    /// The tab's height in the Settings window (the form scrolls).
    static let height: CGFloat = 640

    /// - Parameter ai: already loaded for the snapshot harness (`.task` never runs offscreen).
    init(ai: AiSettingsModel? = nil) {
        _ai = State(initialValue: ai)
    }

    var body: some View {
        SettingsForm {
            if let ai {
                modelsSection(ai)
                AiLocalServersSection(ai: ai, added: added)
                if let status = ai.status, !status.backends.isEmpty {
                    AiFeatureModelsSection(ai: ai)
                }
                if let budget = ai.status?.budget, ai.hasApiKey {
                    // A new saved amount refills the form.
                    AiBudgetSection(ai: ai, budget: budget)
                        .id(budget.monthlyMicroUsd.map { String($0) } ?? "none")
                }
                outputLanguageSection(ai)
                if model.weeklyNote != nil {
                    AiWeeklyNoteSection(ai: ai)
                }
                AiUsageSection(ai: ai)
                removeAllSection(ai)
            }
        }
        // Read again each time the tab shows (local servers are detected anew); a new service
        // (mock ↔ live, or the live facade once it has opened) is a new setup.
        .task(id: model.serviceGeneration) {
            let current: AiSettingsModel
            // A model handed in (the snapshot harness) reads the current service.
            if let ai, loadedFor == nil || loadedFor == model.serviceGeneration {
                current = ai
            } else {
                current = AiSettingsModel(service: model.service, clock: model.clock, calendar: model.calendar)
                ai = current
            }
            loadedFor = model.serviceGeneration
            await current.load()
        }
        .sheet(item: $keySheet, onDismiss: {
            disclosureFor = pendingDisclosure
            pendingDisclosure = nil
        }) { sheet in
            if let ai {
                AiKeySheet(ai: ai, mode: sheet) { record in
                    // Only this sheet, while it's still up (its disclosure follows its dismissal).
                    guard keySheet?.id == sheet.id else { return }
                    if let record {
                        announce(l10n("ai.addKey.added", ["name": record.label]))
                        pendingDisclosure = disclosure(of: record)
                    }
                    keySheet = nil
                }
                .pageLampEnvironment(model)
            }
        }
        // Only while its backend is in the status (a failed re-read would leave it empty).
        .sheet(item: Binding(
            get: { disclosureFor.flatMap { ai?.backend(key: $0.key) == nil ? nil : $0 } },
            set: { disclosureFor = $0 }
        )) { request in
            if let ai, let backend = ai.backend(key: request.key) {
                AiDisclosureSheet(ai: ai, backend: backend) {
                    disclosureFor = nil
                    if request.added { focusedBackend = request.key }
                }
                .pageLampEnvironment(model)
            }
        }
        .alert(
            l10n("ai.backend.removeTitle", ["name": removing?.label ?? ""]),
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            presenting: removing
        ) { provider in
            Button(l10n("common.actions.cancel"), role: .cancel) {}
            Button(l10n("ai.backend.remove"), role: .destructive) {
                Task {
                    if await ai?.removeProvider(provider.providerId) == true {
                        announce(l10n("ai.backend.removed", ["name": provider.label]))
                    }
                }
            }
        } message: { provider in
            Text(l10n("ai.backend.removeBody", ["name": provider.label]))
        }
        .alert(l10n("ai.removeAll.title"), isPresented: $confirmingRemoveAll) {
            Button(l10n("common.actions.cancel"), role: .cancel) {}
            Button(l10n("mac.ai.removeAllConfirm"), role: .destructive) {
                Task {
                    if await ai?.removeAll() == true { announce(l10n("ai.removeAll.done")) }
                }
            }
        } message: {
            // Signing out of ChatGPT is mentioned only when Codex is signed in.
            Text(l10n(ai?.codexSignedIn == true ? "ai.removeAll.bodyCodex" : "ai.removeAll.body"))
        }
    }

    // MARK: - AI models

    @ViewBuilder
    private func modelsSection(_ ai: AiSettingsModel) -> some View {
        Section {
            if let failure = ai.statusFailure {
                Text(l10n.sentences(l10n("ai.settings.loadFailed"), l10n.aiError(failure)))
                    .foregroundStyle(PLColor.danger)
            } else if ai.providerBackends.isEmpty {
                Text(l10n("ai.settings.empty"))
                    .foregroundStyle(.secondary)
            } else {
                ForEach(ai.providerBackends, id: \.backend) { backend in
                    BackendRowView(
                        backend: backend, provider: ai.provider(of: backend),
                        showDisclosure: { disclosureFor = DisclosureRequest(key: AiCodes.key(backend.backend), added: false) },
                        replaceKey: { provider in openKeySheet(.replace(provider)) },
                        remove: { provider in removing = provider }
                    )
                    .accessibilityFocused($focusedBackend, equals: AiCodes.key(backend.backend))
                }
                if let failure = ai.removeFailure {
                    Text(l10n.aiError(failure)).foregroundStyle(PLColor.danger)
                }
            }
            Button(l10n("mac.ai.addKey")) { openKeySheet(.add) }
        } header: {
            Text(l10n("ai.settings.title"))
        } footer: {
            Text(l10n("ai.settings.description"))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: - Output language

    @ViewBuilder
    private func outputLanguageSection(_ ai: AiSettingsModel) -> some View {
        Section {
            if let language = ai.outputLanguage {
                Picker(l10n("explain.language.title"), selection: Binding(
                    get: { language },
                    set: { value in Task { await ai.setOutputLanguage(value) } }
                )) {
                    Text(l10n("explain.language.ui")).tag(OutputLanguage.ui)
                    Text(l10n("explain.language.course")).tag(OutputLanguage.course)
                }
                .pickerStyle(.radioGroup)
                .labelsHidden()
            }
        } header: {
            Text(l10n("explain.language.title"))
        } footer: {
            Text(l10n("explain.language.description"))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: - Remove all AI data

    @ViewBuilder
    private func removeAllSection(_ ai: AiSettingsModel) -> some View {
        Section {
            HStack(alignment: .firstTextBaseline, spacing: PLSpace.s4) {
                Text(l10n("ai.removeAll.hint"))
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button(l10n("mac.ai.removeAll"), role: .destructive) { confirmingRemoveAll = true }
            }
            if let failure = ai.removeAllFailure {
                Text(l10n.aiError(failure)).foregroundStyle(PLColor.danger)
            }
        }
    }

    /// A new key sheet starts without the last one's failure.
    private func openKeySheet(_ sheet: KeySheet) {
        ai?.clearKeyFailure()
        keySheet = sheet
    }

    /// A local server was added: say so, then show its disclosure (turning it on).
    private func added(_ record: ModelProviderRecord) {
        announce(l10n("ai.addKey.added", ["name": record.label]))
        disclosureFor = disclosure(of: record)
    }

    private func disclosure(of record: ModelProviderRecord) -> DisclosureRequest {
        DisclosureRequest(key: AiCodes.key(.provider(providerId: record.providerId)), added: true)
    }

    private func announce(_ text: String) {
        AccessibilityNotification.Announcement(text).post()
    }
}

/// What the key sheet is for.
package enum KeySheet: Identifiable {
    case add
    case replace(ModelProviderRecord)

    package var id: String {
        switch self {
        case .add: "add"
        case .replace(let provider): "replace:\(provider.providerId)"
        }
    }
}

/// A backend whose disclosure to show: from its row, or right after it was added.
struct DisclosureRequest: Identifiable {
    let key: String
    let added: Bool
    var id: String { key }
}

// MARK: - One backend

/// A key or local model: its name, kind, state, problems, key ending, data policy, and actions.
private struct BackendRowView: View {
    let backend: AiBackendStatus
    let provider: ModelProviderRecord?
    let showDisclosure: () -> Void
    let replaceKey: (ModelProviderRecord) -> Void
    let remove: (ModelProviderRecord) -> Void

    @Environment(\.l10n) private var l10n

    var body: some View {
        VStack(alignment: .leading, spacing: PLSpace.s1) {
            HStack(alignment: .firstTextBaseline, spacing: PLSpace.s2) {
                Text(backend.label)
                    .font(PLType.headline.font)
                    .accessibilityAddTraits(.isHeader)
                Text(l10n("ai.backend.kind.\(AiCodes.name(backend.kind))"))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                Spacer()
                Label {
                    Text(l10n("ai.backend.state.\(AiCodes.name(backend.state))"))
                } icon: {
                    Image(systemName: stateSymbol)
                }
                .font(PLType.callout.font)
                .foregroundStyle(backend.state == .ready ? AnyShapeStyle(.secondary) : AnyShapeStyle(PLColor.warning))
            }
            if let last4 = provider?.keyLast4 {
                Text(l10n("ai.backend.keyEnds", ["last4": last4]))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
            }
            Text(l10n.dataPolicy(backend.disclosure))
                .font(PLType.callout.font)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityLabel(l10n.labelled(l10n("ai.policy.label"), l10n.dataPolicy(backend.disclosure)))
            ForEach(backend.problems, id: \.self) { problem in
                Text(l10n("ai.backend.problem.\(AiCodes.name(problem))"))
                    .font(PLType.callout.font)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: PLSpace.s2) {
                if backend.state == .needsDisclosure {
                    Button(l10n("mac.ai.turnOn"), action: showDisclosure)
                        .buttonStyle(.borderedProminent)
                } else {
                    Button(l10n("mac.ai.whatsShared"), action: showDisclosure)
                }
                if let provider, backend.kind == .apiKey {
                    Button(l10n("mac.ai.replaceKey")) { replaceKey(provider) }
                }
                if let provider {
                    Button(l10n("mac.ai.remove")) { remove(provider) }
                }
            }
            .controlSize(.small)
            .padding(.top, PLSpace.s1)
        }
        .accessibilityElement(children: .contain)
    }

    private var stateSymbol: String {
        switch backend.state {
        case .ready: "checkmark.circle"
        case .needsDisclosure: "circle.dashed"
        case .needsSetup, .unavailable: "exclamationmark.triangle"
        }
    }
}
