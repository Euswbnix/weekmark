// The mock's AI setup, like the Tauri mock's (apps/desktop/src/api/mock/ai.ts and ai-fixtures.ts),
// which follow the facade: presets; providers whose key is checked and dropped (only its last 4
// characters are kept); models and their prices; local servers; disclosures acknowledged by
// version; the model per feature; the unpriced acknowledgement; the monthly budget; the cost
// estimate with the facade's blocks in the facade's order; and a usage ledger. The ChatGPT plan
// isn't offered (as in every build until OpenAI confirms in writing), so there is no Codex backend.

import Foundation
import PageLampKit

/// The AI setup the mock keeps.
struct MockAi: Sendable {
    var providers: [ModelProviderRecord] = []
    /// Acknowledged disclosure versions by backend key ("provider:<id>").
    var acknowledged: [String: UInt32] = [:]
    /// "<backend key>/<model>" pairs whose missing price the student accepted.
    var unpricedAcks: Set<String> = []
    var routing: [AiFeature: ModelChoice] = [:]
    var budget: UInt64? = MockAiFixtures.defaultBudget
    /// The ledger: each row with how many months ago it was.
    var usage: [MockUsage] = []
    /// `aiErrors`: listing models fails as if the provider were unreachable; "Test" is rate-limited.
    var unreachable = false

    /// The AI setup a scenario starts with (the Tauri mock's scenarios of the same names).
    static func start(_ scenario: MockScenario, now: Date, calendar: Calendar) -> MockAi {
        var ai = MockAi()
        let created = calendar.date(byAdding: .day, value: -20, to: now) ?? now
        func record(_ preset: String, id: String, url: String, key: String?) {
            let spec = MockAiFixtures.preset(preset)
            ai.providers.append(ModelProviderRecord(
                providerId: id, preset: preset,
                label: preset == "custom" ? (URL(string: url)?.host() ?? url) : spec?.label ?? preset,
                wire: spec?.wire ?? .openaiChat, baseUrl: url, keyLast4: key.map(MockAi.last4),
                onDevice: MockAi.isLoopback(url), createdAt: created
            ))
        }
        func route(_ id: String, _ model: String, effort: Effort = .lowest, only feature: AiFeature? = nil) {
            for each in feature.map({ [$0] }) ?? MockAiFixtures.features {
                ai.routing[each] = ModelChoice(backend: .provider(providerId: id), model: model, effort: effort)
            }
        }
        switch scenario {
        case .aiKey, .aiBudget, .aiUnpriced, .weeklyNoteMonday:
            record("openai", id: "openai", url: "https://api.openai.com/v1", key: "sk-demo-000000000000007Qx2")
            ai.acknowledged["provider:openai"] = 3101
            route("openai", "gpt-6-luna")
            route("openai", scenario == .aiUnpriced ? "gpt-6-preview-0929" : "gpt-5.4-mini", effort: .low, only: .weeklyExplanation)
            ai.usage = MockAiFixtures.usage(scenario)
        case .aiLocal:
            record("ollama", id: "ollama", url: "http://127.0.0.1:11434", key: nil)
            ai.acknowledged["provider:ollama"] = 3501
            route("ollama", "qwen3.5:9b")
            ai.usage = MockAiFixtures.usage(scenario)
        case .aiDisclosureChanged:
            record("anthropic", id: "anthropic", url: "https://api.anthropic.com", key: "sk-ant-demo-0000Hq8e")
            // Acknowledged the version before the current one: the disclosure changed.
            ai.acknowledged["provider:anthropic"] = 3200
            route("anthropic", "claude-haiku-4-5")
        case .aiErrors:
            let url = "https://llm.demo.test/v1"
            let id = "custom-" + MockAi.shortHash(url)
            record("custom", id: id, url: url, key: "demo-key-0000Zt3k")
            ai.acknowledged["provider:\(id)"] = 3701
            route(id, "demo-model-large")
            ai.unreachable = true
        default:
            break
        }
        return ai
    }

    // MARK: - Helpers (the Tauri mock's)

    /// The preset a local server is added as (the facade's `LocalServer.preset`).
    static func presetName(_ kind: LocalServerKind) -> String {
        switch kind {
        case .ollama: "ollama"
        case .lmStudio: "lm_studio"
        }
    }

    static func last4(_ key: String) -> String {
        String(key.trimmingCharacters(in: .whitespacesAndNewlines).suffix(4))
    }

    /// Localhost only: PageLamp treats a server on this computer as on-device.
    static func isLoopback(_ url: String) -> Bool {
        guard let host = URL(string: url)?.host() else { return false }
        return ["localhost", "127.0.0.1", "::1", "[::1]"].contains(host)
    }

    /// A short stable id for a custom endpoint (the Tauri mock's `shortHash`).
    static func shortHash(_ text: String) -> String {
        var hash: UInt32 = 0
        for scalar in text.unicodeScalars {
            hash = hash &* 31 &+ scalar.value
        }
        return String(String(hash, radix: 36).prefix(6))
    }

    static func key(_ backend: BackendRef) -> String {
        switch backend {
        case .provider(let providerId): "provider:\(providerId)"
        case .codex: "codex"
        case .claudeCode: "claude_code"
        }
    }
}

/// One ledger row and how many months ago it was.
struct MockUsage: Sendable {
    var monthsAgo: Int
    var row: UsageRow
}

/// The Tauri mock's AI fixtures (ai-fixtures.ts).
enum MockAiFixtures {
    /// US$5 a month (decision D18).
    static let defaultBudget: UInt64 = 5_000_000
    static let features: [AiFeature] = [.studyPlan, .weeklyExplanation, .weeklyNote, .courseCalendar]
    private static let light: [AiFeature] = [.studyPlan, .weeklyNote, .courseCalendar]

    /// Keys of a coding plan (only for the vendor's own tools): refused.
    static let codingPlanHosts = [
        "api.z.ai/api/coding", "coding-intl.dashscope.aliyuncs.com", "api.kimi.ai/coding", "api.kimi.com/coding",
    ]
    static let codingPlanKeyPrefixes = ["sk-sp-"]

    /// micro-USD per million tokens, input and output, by model id.
    static let prices: [String: (input: UInt64, output: UInt64)] = [
        "gpt-6-luna": (100_000, 400_000),
        "gpt-5.4-mini": (750_000, 4_500_000),
        "gpt-6-astra": (10_000_000, 40_000_000),
        "claude-haiku-4-5": (1_000_000, 5_000_000),
        "claude-sonnet-5": (2_000_000, 10_000_000),
        "claude-opus-5-5": (4_000_000, 20_000_000),
        "gemini-2.5-flash-lite": (100_000, 400_000),
        "gemini-3.8-flash": (750_000, 4_500_000),
        "openai/gpt-6-luna": (100_000, 400_000),
    ]

    static func facts(
        _ version: UInt32, _ recipient: String, terms: String?, training: TrainingFact, retention: RetentionFact,
        minAge: UInt8?, guardian: Bool = false, cost: CostKind, onDevice: Bool = false
    ) -> DisclosureFacts {
        DisclosureFacts(
            version: version, sends: [.structure, .materialText], recipient: Recipient(name: recipient, termsUrl: terms),
            training: training, retention: retention, adminVisibility: .no, minAge: minAge,
            guardianPermission: guardian, cost: cost, onDevice: onDevice, location: nil
        )
    }

    /// In the facade's order.
    static let presets: [ProviderPreset] = [
        ProviderPreset(
            id: "openai", label: "OpenAI", wire: .openaiResponses, defaultBaseUrl: "https://api.openai.com/v1",
            needsKey: true, baseUrlEditable: false, local: false,
            dataPolicy: facts(3101, "OpenAI", terms: "https://openai.demo.test/api-terms", training: .noTraining,
                              retention: .storedDays(days: 30), minAge: 13, guardian: true, cost: .apiBilling)
        ),
        ProviderPreset(
            id: "anthropic", label: "Anthropic", wire: .anthropicMessages, defaultBaseUrl: "https://api.anthropic.com",
            needsKey: true, baseUrlEditable: false, local: false,
            dataPolicy: facts(3201, "Anthropic", terms: "https://anthropic.demo.test/commercial-terms",
                              training: .noTraining, retention: .storedDays(days: 30), minAge: 18, cost: .apiBilling)
        ),
        ProviderPreset(
            id: "gemini", label: "Google Gemini", wire: .openaiChat,
            defaultBaseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
            needsKey: true, baseUrlEditable: false, local: false,
            dataPolicy: facts(3301, "Google", terms: "https://google.demo.test/gemini-api-terms",
                              training: .mayTrainFreeTier, retention: .providerTerms, minAge: 18, cost: .apiBilling)
        ),
        ProviderPreset(
            id: "openrouter", label: "OpenRouter", wire: .openaiChat, defaultBaseUrl: "https://openrouter.ai/api/v1",
            needsKey: true, baseUrlEditable: false, local: false,
            dataPolicy: facts(3401, "OpenRouter", terms: "https://openrouter.demo.test/terms", training: .noTraining,
                              retention: .providerTerms, minAge: nil, cost: .apiBilling)
        ),
        ProviderPreset(
            id: "ollama", label: "Ollama", wire: .ollamaNative, defaultBaseUrl: "http://127.0.0.1:11434",
            needsKey: false, baseUrlEditable: true, local: true,
            dataPolicy: facts(3501, "Ollama", terms: nil, training: .noTraining, retention: .onDevice, minAge: nil,
                              cost: .freeOnDevice, onDevice: true)
        ),
        ProviderPreset(
            id: "lm_studio", label: "LM Studio", wire: .openaiChat, defaultBaseUrl: "http://127.0.0.1:1234/v1",
            needsKey: false, baseUrlEditable: true, local: true,
            dataPolicy: facts(3601, "LM Studio", terms: nil, training: .noTraining, retention: .onDevice, minAge: nil,
                              cost: .freeOnDevice, onDevice: true)
        ),
        ProviderPreset(
            id: "custom", label: "Custom (OpenAI-compatible)", wire: .openaiChat, defaultBaseUrl: nil,
            needsKey: true, baseUrlEditable: true, local: false,
            dataPolicy: facts(3701, "Custom endpoint", terms: nil, training: .unknown, retention: .providerTerms,
                              minAge: nil, cost: .apiBilling)
        ),
    ]

    static func preset(_ id: String) -> ProviderPreset? {
        presets.first { $0.id == id }
    }

    /// Ollama's cloud models run on Ollama's servers: routing a feature to one changes the facts.
    static func ollamaCloudFacts(_ base: DisclosureFacts) -> DisclosureFacts {
        DisclosureFacts(
            version: base.version + 50, sends: base.sends,
            recipient: Recipient(name: "Ollama", termsUrl: "https://ollama.demo.test/cloud-terms"),
            training: base.training, retention: .providerTerms, adminVisibility: base.adminVisibility,
            minAge: base.minAge, guardianPermission: base.guardianPermission, cost: .cloudViaLocal,
            onDevice: false, location: base.location
        )
    }

    private static func model(
        _ id: String, context: UInt32 = 200_000, onDevice: Bool = false, cloud: Bool = false, priced: Bool? = nil,
        reasoning: Bool = false, suggested: [AiFeature] = []
    ) -> ModelInfo {
        ModelInfo(
            id: id, label: nil, onDevice: onDevice, runsInCloud: cloud, priceKnown: priced ?? (prices[id] != nil),
            contextWindow: context, reasoningAlwaysOn: reasoning, suggestedFor: suggested
        )
    }

    /// Models by preset.
    static let models: [String: [ModelInfo]] = [
        "openai": [
            model("gpt-6-luna", context: 400_000, suggested: light),
            model("gpt-5.4-mini", context: 400_000, suggested: [.weeklyExplanation]),
            model("gpt-6-astra", context: 1_000_000),
            model("gpt-6-preview-0929", context: 400_000),
        ],
        "anthropic": [
            model("claude-haiku-4-5", suggested: features),
            model("claude-sonnet-5"),
            model("claude-opus-5-5", reasoning: true),
        ],
        "gemini": [
            model("gemini-2.5-flash-lite", context: 1_000_000, suggested: light),
            model("gemini-3.8-flash", context: 1_000_000, suggested: [.weeklyExplanation]),
        ],
        "openrouter": [model("openai/gpt-6-luna", suggested: features)],
        "ollama": [
            model("qwen3.5:9b", context: 32_768, onDevice: true, priced: true, suggested: features),
            model("gemma4:12b", context: 32_768, onDevice: true, priced: true),
            model("gpt-oss:120b-cloud", context: 131_072, cloud: true, priced: false),
        ],
        "lm_studio": [model("qwen3.5-4b", context: 32_768, onDevice: true, priced: true, suggested: features)],
        "custom": [model("demo-model-large", context: 128_000)],
    ]

    /// What "Detect" finds on this computer.
    static let localServers: [LocalServer] = [
        LocalServer(kind: .ollama, baseUrl: "http://127.0.0.1:11434", running: true),
        LocalServer(kind: .lmStudio, baseUrl: "http://127.0.0.1:1234/v1", running: false),
    ]

    /// A ledger row as the Tauri mock builds it.
    private static func row(
        _ label: String, local: Bool = false, _ model: String, _ feature: AiFeature, runs: UInt32,
        microUsd: UInt64?, estimated: Bool = false
    ) -> UsageRow {
        let perRun: UInt64 = feature == .weeklyExplanation ? 42_000 : 7_500
        return UsageRow(
            backendLabel: label, model: model, feature: feature, runs: runs,
            inputTokens: UInt64(runs) * perRun, outputTokens: UInt64(runs) * 2_400, reasoningTokens: UInt64(runs) * 600,
            costBasis: microUsd == nil ? .unpriced : local ? .freeOnDevice : .priced, microUsd: microUsd,
            estimated: estimated
        )
    }

    static func usage(_ scenario: MockScenario) -> [MockUsage] {
        switch scenario {
        case .aiKey:
            [
                MockUsage(monthsAgo: 0, row: row("OpenAI", "gpt-6-luna", .studyPlan, runs: 6, microUsd: 400_000)),
                MockUsage(monthsAgo: 0, row: row("OpenAI", "gpt-5.4-mini", .weeklyExplanation, runs: 14, microUsd: 1_600_000, estimated: true)),
                MockUsage(monthsAgo: 1, row: row("OpenAI", "gpt-6-luna", .studyPlan, runs: 9, microUsd: 620_000)),
                MockUsage(monthsAgo: 1, row: row("OpenAI", "gpt-5.4-mini", .weeklyExplanation, runs: 21, microUsd: 2_480_000)),
            ]
        case .aiBudget:
            [
                MockUsage(monthsAgo: 0, row: row("OpenAI", "gpt-6-luna", .studyPlan, runs: 10, microUsd: 650_000)),
                MockUsage(monthsAgo: 0, row: row("OpenAI", "gpt-5.4-mini", .weeklyExplanation, runs: 38, microUsd: 4_310_000)),
            ]
        case .aiUnpriced:
            [
                MockUsage(monthsAgo: 0, row: row("OpenAI", "gpt-6-luna", .studyPlan, runs: 4, microUsd: 260_000)),
                MockUsage(monthsAgo: 0, row: row("OpenAI", "gpt-6-preview-0929", .weeklyExplanation, runs: 3, microUsd: nil)),
            ]
        case .aiLocal:
            [
                MockUsage(monthsAgo: 0, row: row("Ollama", local: true, "qwen3.5:9b", .weeklyExplanation, runs: 9, microUsd: 0)),
                MockUsage(monthsAgo: 0, row: row("Ollama", local: true, "qwen3.5:9b", .studyPlan, runs: 5, microUsd: 0)),
                MockUsage(monthsAgo: 0, row: row("Ollama", local: true, "gpt-oss:120b-cloud", .weeklyExplanation, runs: 2, microUsd: nil)),
            ]
        default:
            []
        }
    }

    /// The tokens a run of `request` sends and may get back (the Tauri mock's workload).
    /// - Parameter lifted: the included materials an explanation sends too (each one more
    ///   material's worth, like the Tauri mock's).
    static func workload(_ request: EstimateRequest, lifted: Int = 0) -> (input: UInt64, output: UInt64) {
        switch request {
        case .studyPlan(_, let courses): (6_000 + 1_500 * UInt64(courses.count), 8_000)
        case .weeklyExplanation: (45_000 + 15_000 * UInt64(lifted), 6_000)
        case .weeklyNote: (3_000, 1_500)
        case .courseCalendar(let courses): (15_000 * UInt64(max(1, courses.count)), 4_000)
        }
    }

    static func reasoning(_ effort: Effort) -> UInt64 {
        switch effort {
        case .lowest: 0
        case .low: 2_000
        case .medium: 8_000
        case .high: 24_000
        }
    }
}
