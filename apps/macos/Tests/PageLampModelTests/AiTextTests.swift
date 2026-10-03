// The AI screens' words and numbers: every facade code has its string in both languages, a
// failure is worded by its code (never by the backend's English), and money and token counts
// follow the Tauri app's rules.

import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Testing

private let en = L10n(locale: Locale(identifier: "en_US"), table: .app)
private let zh = L10n(locale: Locale(identifier: "zh-Hans_CN"), table: .app)

@Suite("AI text")
struct AiTextTests {
    @Test("every code the facade sends has its string")
    func codesHaveStrings() {
        let blocked: [BlockReason] = [
            .coursePolicyProhibited, .courseAiTurnedOff, .courseHidden, .noReadableMaterials,
            .materialSharingNotAllowed, .codingPlanKey, .disclosureNotAcknowledged, .noModelChosen,
            .budgetReached, .priceUnknownNotAcknowledged, .weeklyRunCapReached, .backendDisabledInThisBuild,
            .nothingToWrite, .noCourseToPlan,
        ]
        let modelErrors: [ModelErrorKind] = [
            .notSignedIn, .authRejected, .billingOrQuota, .usageLimit, .rateLimited, .overloaded, .invalidRequest,
            .modelNotFound, .contextTooLong, .refused, .contentFiltered, .network, .timeout, .badOutput,
            .runtimeMissing, .runtimeVerifyFailed, .runtimeOutdated, .unsupported,
        ]
        let kinds: [BackendKind] = [.apiKey, .local, .codex, .claudeCode]
        let states: [BackendState] = [.ready, .needsSetup, .needsDisclosure, .unavailable]
        let problems: [BackendProblem] = [.keyMissing, .serverNotRunning, .modelMissing, .disclosureChanged, .notSignedIn, .runtimeMissing]
        let features: [AiFeature] = [.studyPlan, .weeklyExplanation, .weeklyNote, .courseCalendar]
        let efforts: [Effort] = [.lowest, .low, .medium, .high]
        let training: [TrainingFact] = [.noTraining, .mayTrain(howToTurnOffUrl: nil), .mayTrainFreeTier, .unknown]
        let retention: [RetentionFact] = [.notStored, .providerTerms, .onDevice]
        let costs: [CostKind] = [.apiBilling, .planCredits, .freeOnDevice, .cloudViaLocal, .selfHosted]
        let sent: [SentData] = [.structure, .materialText]
        let servers: [LocalServerKind] = [.ollama, .lmStudio]

        var keys = blocked.map { "ai.blocked.\(AiCodes.name($0))" }
        keys += modelErrors.map { "ai.modelError.\(AiCodes.name($0))" }
        keys += kinds.map { "ai.backend.kind.\(AiCodes.name($0))" }
        keys += states.map { "ai.backend.state.\(AiCodes.name($0))" }
        keys += problems.map { "ai.backend.problem.\(AiCodes.name($0))" }
        keys += features.map { "ai.features.name.\(AiCodes.name($0))" }
        keys += efforts.map { "ai.features.effortName.\(AiCodes.name($0))" }
        keys += training.flatMap { ["ai.policy.training.\(AiCodes.name($0))", "ai.disclosure.training.\(AiCodes.name($0))"] }
        keys += retention.flatMap { ["ai.policy.retention.\(AiCodes.name($0))", "ai.disclosure.retention.\(AiCodes.name($0))"] }
        keys += costs.flatMap { ["ai.policy.cost.\(AiCodes.name($0))", "ai.disclosure.cost.\(AiCodes.name($0))"] }
        keys += sent.map { "ai.disclosure.sent.\(AiCodes.name($0))" }
        keys += servers.map { "ai.local.server.\(AiCodes.name($0))" }
        for key in keys {
            #expect(en.has(key), "\(key) is missing")
        }
        // Stored days are plural keys.
        #expect(AiCodes.name(RetentionFact.storedDays(days: 30)) == "stored_days")
        #expect(en.table.plural.contains("ai.policy.retention.stored_days"))
        #expect(en.table.plural.contains("ai.disclosure.retention.stored_days"))
        #expect(AiCodes.key(.provider(providerId: "openai")) == "provider:openai")
        #expect(AiCodes.key(.codex) == "codex" && AiCodes.key(.claudeCode) == "claude_code")
    }

    @Test("a failure is worded by its code, with the wait the service asked for")
    func failureText() {
        let budget = PageLampFailure(kind: .blocked, message: "over budget", blocked: .budgetReached)
        #expect(en.aiError(budget) == "This would go over this month's budget.")
        let nothing = PageLampFailure(kind: .blocked, message: "nothing", blocked: .nothingToWrite)
        #expect(en.aiError(nothing).hasPrefix("There's nothing to write about this week yet"))
        #expect(zh.aiError(nothing).hasPrefix("这周还没有可写的内容"))
        let noCourse = PageLampFailure(kind: .blocked, message: "no course", blocked: .noCourseToPlan)
        #expect(en.aiError(noCourse) == "There's no active course to plan for.")
        #expect(zh.aiError(noCourse) == "没有可以规划的进行中课程。")
        // A refusal without a reason, and a model failure without a kind: the failure's kind.
        let unnamed = PageLampFailure(kind: .blocked, message: "blocked")
        #expect(en.aiError(unnamed) == unnamed.localizedDescription(in: en))
        let bare = PageLampFailure(kind: .model, message: "model")
        #expect(en.aiError(bare) == bare.localizedDescription(in: en))
        let wait = PageLampFailure(kind: .model, message: "429", modelError: .rateLimited, retryAfterSecs: 30)
        #expect(en.aiError(wait) == "The provider is limiting requests. Try again in 30 s.")
        let noWait = PageLampFailure(kind: .model, message: "429", modelError: .rateLimited, retryAfterSecs: 0)
        #expect(en.aiError(noWait) == "The provider is limiting requests. Try again in a moment.")
        let network = PageLampFailure(kind: .network, message: "offline")
        #expect(en.aiError(network) == network.localizedDescription(in: en))
        // Never the backend's English.
        #expect(zh.aiError(budget) != budget.message)
        #expect(!en.aiError(budget).contains("over budget"))
    }

    @Test("dollars with two decimals, half a cent rounded up; compact token counts")
    func numbers() {
        #expect(en.usd(microUsd: 70_000) == "$0.07")
        #expect(en.usd(microUsd: 5_000_000) == "$5.00")
        #expect(en.usd(microUsd: 25_000) == "$0.03")
        #expect(en.usd(microUsd: 1_234_567) == "$1.23")
        #expect(zh.usd(microUsd: 2_000_000) == "US$2.00")
        #expect(en.compactTokens(900) == "900")
        #expect(en.compactTokens(45_000) == "45K")
        #expect(en.compactTokens(3_400_000) == "3.4M")
    }

    @Test("the budget field reads what the Tauri app reads")
    func money() {
        #expect(AiMoney.parse("5") == 5_000_000)
        #expect(AiMoney.parse(" 2.50 ") == 2_500_000)
        #expect(AiMoney.parse("$3") == 3_000_000)
        #expect(AiMoney.parse("us$3") == 3_000_000)
        #expect(AiMoney.parse("US$ 3") == 3_000_000 && AiMoney.parse("$ 5") == 5_000_000)
        #expect(AiMoney.parse("1,000") == 1_000_000_000)
        #expect(AiMoney.parse("5.") == 5_000_000)
        #expect(AiMoney.parse("0") == 0)
        for invalid in ["", "-1", "2.505", ".5", "$", "five", "１２", "5.5.5", "1 000"] {
            #expect(AiMoney.parse(invalid) == nil, "\(invalid) should be refused")
        }
        #expect(AiMoney.inputValue(nil) == "")
        #expect(AiMoney.inputValue(5_000_000) == "5.00")
        #expect(AiMoney.inputValue(2_505_000) == "2.51")
        #expect(AiMoney.inputValue(0) == "0.00")
    }

    @Test("a data-policy line from the facts: on this computer, only that it stays and is free")
    func dataPolicy() async throws {
        let service = MockService(scenario: .demo, timing: .instant, calendar: TestClock.calendar, now: { TestClock.now })
        let presets = try await service.modelProviderPresets()
        let ollama = try #require(presets.first { $0.id == "ollama" })
        #expect(en.dataPolicy(ollama.dataPolicy) == "Stays on this computer · free")
        let openai = try #require(presets.first { $0.id == "openai" })
        let line = en.dataPolicy(openai.dataPolicy)
        #expect(line.hasPrefix("Sent to OpenAI · "))
        #expect(line.contains(" · kept up to 30 days · 13+ with a guardian's permission · "))
        #expect(line.hasSuffix(" · billed to your API account"))
        #expect(zh.dataPolicy(openai.dataPolicy) != line)
    }
}
