// Settings ▸ AI (M3, preview builds) for the snapshot catalogue: nothing set up; an API key with
// its models chosen and a passing Test; a model on this computer; a model without a price; the
// budget nearly used; a disclosure that changed; a server that can't be reached (Test fails).
// Then the sheets on their own: adding a key, a coding-plan key refused, replacing a key, a first
// disclosure (age and free-tier confirmations) and one already read. Keys typed here are
// synthetic and go only to the mock; the key field is always empty in a render.

import SwiftUI
import PageLamp
import PageLampKit
import PageLampModel

enum AiSettingsSnapshots {
    static let pages: [SnapshotPage] = [
        settings("settings-ai-empty", scenario: .demo, prepare: nothing),
        settings("settings-ai-key", scenario: .aiKey, prepare: testStudyPlan),
        settings("settings-ai-local", scenario: .aiLocal, prepare: nothing),
        settings("settings-ai-unpriced", scenario: .aiUnpriced, prepare: nothing),
        settings("settings-ai-budget", scenario: .aiBudget, prepare: nothing),
        settings("settings-ai-disclosure-changed", scenario: .aiDisclosureChanged, prepare: nothing),
        settings("settings-ai-errors", scenario: .aiErrors, prepare: testStudyPlan),
        keySheet("ai-key-add", scenario: .demo, mode: addMode, prepare: nothing),
        keySheet("ai-key-coding-plan", scenario: .demo, mode: addMode, prepare: codingPlanKey),
        keySheet("ai-key-replace", scenario: .aiKey, mode: replaceMode, prepare: nothing),
        disclosure("ai-disclosure-first", scenario: .demo, prepare: addGeminiKey),
        disclosure("ai-disclosure-read", scenario: .aiKey, prepare: nothing),
    ]

    // MARK: - Preparations

    private static func nothing(_ ai: AiSettingsModel) async {}

    /// Test on study plans (its result line shows).
    private static func testStudyPlan(_ ai: AiSettingsModel) async {
        await ai.test(.studyPlan)
    }

    /// A coding-plan key (the mock refuses its prefix, quoting the vendor).
    private static func codingPlanKey(_ ai: AiSettingsModel) async {
        guard let preset = ai.keyPresets.first else { return }
        _ = await ai.addProvider(preset: preset, baseUrl: "", key: "sk-sp-demo-0000")
    }

    /// A Gemini key: its disclosure asks for the age and the free tier.
    private static func addGeminiKey(_ ai: AiSettingsModel) async {
        guard let preset = ai.keyPresets.first(where: { $0.id == "gemini" }) else { return }
        _ = await ai.addProvider(preset: preset, baseUrl: "", key: "demo-key-4821")
    }

    // MARK: - Sheet modes

    private static func addMode(_ ai: AiSettingsModel) -> KeySheet {
        .add
    }

    /// Replacing the first provider's key.
    private static func replaceMode(_ ai: AiSettingsModel) -> KeySheet {
        ai.status?.providers.first.map { .replace($0) } ?? .add
    }

    // MARK: - Pages

    /// The AI tab after `prepare`, loaded (`.task` never runs offscreen).
    private static func settings(
        _ name: String,
        scenario: MockScenario,
        prepare: @escaping @MainActor (AiSettingsModel) async -> Void
    ) -> SnapshotPage {
        SnapshotPage(name: name, width: WindowMetrics.settingsWidth, minHeight: 640, setup: SnapshotSetup(scenario: scenario)) { model in
            let ai = await loaded(model)
            await prepare(ai)
            return AnyView(SettingsTabPage(tab: .ai, title: model.l10n("mac.settings.tabs.ai"), ai: ai))
        }
    }

    private static func keySheet(
        _ name: String,
        scenario: MockScenario,
        mode: @escaping @MainActor (AiSettingsModel) -> KeySheet,
        prepare: @escaping @MainActor (AiSettingsModel) async -> Void
    ) -> SnapshotPage {
        SnapshotPage(name: name, width: AiKeySheet.width, minHeight: 0, setup: SnapshotSetup(scenario: scenario)) { model in
            let ai = await loaded(model)
            await prepare(ai)
            return AnyView(AiKeySheet(ai: ai, mode: mode(ai), done: ignore))
        }
    }

    /// The disclosure of the last provider (the one `prepare` added, or the scenario's).
    private static func disclosure(
        _ name: String,
        scenario: MockScenario,
        prepare: @escaping @MainActor (AiSettingsModel) async -> Void
    ) -> SnapshotPage {
        SnapshotPage(name: name, width: AiDisclosureSheet.width, minHeight: 0, setup: SnapshotSetup(scenario: scenario)) { model in
            let ai = await loaded(model)
            await prepare(ai)
            guard let backend = ai.providerBackends.last else { return AnyView(EmptyView()) }
            return AnyView(AiDisclosureSheet(ai: ai, backend: backend, done: close))
        }
    }

    private static func loaded(_ model: AppModel) async -> AiSettingsModel {
        let ai = AiSettingsModel(service: model.service, clock: model.clock, calendar: model.calendar)
        await ai.load()
        return ai
    }

    private static func ignore(_ record: ModelProviderRecord?) {}

    private static func close() {}
}
