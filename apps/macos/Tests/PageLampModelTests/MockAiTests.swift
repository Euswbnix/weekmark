// The mock's AI setup follows the facade's rules as the Tauri mock has them (and the numbers its
// tests pin): the key is checked and dropped, disclosures are acknowledged by version, a feature's
// model must be its backend's, the estimate blocks in the facade's order, usage per month, and
// "Remove all AI data".

import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Testing

private let openai = BackendRef.provider(providerId: "openai")

@Suite("Mock AI setup")
struct MockAiTests {
    func mock(_ scenario: MockScenario = .demo) -> MockService {
        MockService(scenario: scenario, timing: .instant, calendar: TestClock.calendar, now: { TestClock.now })
    }

    @Test("presets in the facade's order; nothing set up in the demo, and no ChatGPT plan")
    func presets() async throws {
        let service = mock()
        #expect(try await service.modelProviderPresets().map(\.id) == ["openai", "anthropic", "gemini", "openrouter", "ollama", "lm_studio", "custom"])
        let status = try await service.aiStatus()
        #expect(status.backends.isEmpty && status.providers.isEmpty)
        #expect(!status.chatgptPlanOffered)
        #expect(status.features.map(\.feature) == [.studyPlan, .weeklyExplanation, .weeklyNote, .courseCalendar])
        #expect(status.features.allSatisfy { $0.choice == nil })
        #expect(status.budget.monthlyMicroUsd == 5_000_000)
        #expect(try await service.estimateGeneration(request: .weeklyNote).wouldBlock == .noModelChosen)
    }

    @Test("adding a key keeps only its last 4 characters; the new backend asks for its disclosure")
    func addKey() async throws {
        let service = mock()
        let record = try await service.addModelProvider(preset: "openai", baseUrl: nil, apiKey: "  sk-live-secret-value-9f3A  ")
        #expect(record.providerId == "openai" && record.keyLast4 == "9f3A" && !record.onDevice)
        let status = try #require(try await service.aiStatus().backends.first)
        #expect(status.state == .needsDisclosure && status.disclosureAcknowledged == nil && status.kind == .apiKey)
        // The wrong version is refused; the current one turns it on.
        do {
            try await service.acknowledgeAiDisclosure(backend: openai, version: status.disclosure.version + 1)
            Issue.record("expected invalid")
        } catch {
            #expect(error.kind == .invalid)
        }
        try await service.acknowledgeAiDisclosure(backend: openai, version: status.disclosure.version)
        #expect(try await service.aiStatus().backends.first?.state == .ready)
        // Replacing the key keeps only the new last 4.
        #expect(try await service.updateModelProviderKey(providerId: "openai", apiKey: "sk-new-0000Wx9Y").keyLast4 == "Wx9Y")
    }

    @Test("keys and addresses the facade refuses, with its codes")
    func refusals() async throws {
        let service = mock()
        func failure(_ preset: String, _ url: String?, _ key: String?) async -> PageLampFailure? {
            do {
                _ = try await service.addModelProvider(preset: preset, baseUrl: url, apiKey: key)
                return nil
            } catch {
                return error
            }
        }
        #expect(await failure("no-such-preset", nil, "k")?.kind == .notFound)
        #expect(await failure("openai", nil, "   ")?.kind == .invalid)
        let rejected = await failure("openai", nil, "sk-bad-key")
        #expect(rejected?.kind == .model && rejected?.modelError == .authRejected)
        let coding = await failure("openai", nil, "sk-sp-0000")
        #expect(coding?.kind == .blocked && coding?.blocked == .codingPlanKey)
        #expect(await failure("custom", "http://llm.example.edu/v1", "k-1234")?.kind == .invalid)
        #expect(await failure("custom", nil, "k-1234")?.kind == .invalid)
        // A local server needs no key; a second one of the same preset is refused.
        let local = try await service.addModelProvider(preset: "ollama", baseUrl: nil, apiKey: "ignored")
        #expect(local.onDevice && local.keyLast4 == nil)
        #expect(await failure("ollama", nil, nil)?.kind == .invalid)
    }

    @Test("a feature's model must be one of its backend's; removing a provider clears its routing")
    func routing() async throws {
        let service = mock(.aiKey)
        do {
            try await service.setFeatureModel(feature: .weeklyNote, choice: ModelChoice(backend: openai, model: "nope", effort: .low))
            Issue.record("expected invalid")
        } catch {
            #expect(error.kind == .invalid)
        }
        try await service.setFeatureModel(feature: .weeklyNote, choice: ModelChoice(backend: openai, model: "gpt-6-astra", effort: .high))
        #expect(try await service.aiStatus().features.first { $0.feature == .weeklyNote }?.choice?.model == "gpt-6-astra")
        try await service.removeModelProvider(providerId: "openai")
        let status = try await service.aiStatus()
        #expect(status.backends.isEmpty && status.features.allSatisfy { $0.choice == nil })
    }

    @Test("the estimate: the facade's amounts, and its blocks in its order")
    func estimate() async throws {
        let key = mock(.aiKey)
        let explanation = try await key.estimateGeneration(request: .weeklyExplanation(course: "DEMO101", week: nil))
        #expect(explanation.microUsdUpper == 69_750 && explanation.reasoningAllowance == 2_000 && explanation.wouldBlock == nil)
        #expect(try await key.estimateGeneration(request: .weeklyNote).microUsdUpper == 900)
        #expect(try await key.estimateGeneration(request: .studyPlan(horizonDays: nil, courses: [])).microUsdUpper == 3_800)
        // The course's own rules block first, with no amount.
        let prohibited = try await key.estimateGeneration(request: .weeklyExplanation(course: "DEMO310", week: nil))
        #expect(prohibited.wouldBlock == .coursePolicyProhibited && prohibited.microUsdUpper == nil && prohibited.inputTokens == 0)
        #expect(try await key.estimateGeneration(request: .weeklyExplanation(course: "DEMO205", week: nil)).wouldBlock == .materialSharingNotAllowed)
        // Past the budget: blocked, amount kept (the run may go ahead anyway).
        let budget = try await mock(.aiBudget).estimateGeneration(request: .weeklyExplanation(course: "DEMO101", week: nil))
        #expect(budget.wouldBlock == .budgetReached && budget.microUsdUpper == 69_750)
        // No price until the student accepts that.
        let unpriced = mock(.aiUnpriced)
        let first = try await unpriced.estimateGeneration(request: .weeklyExplanation(course: "DEMO101", week: nil))
        #expect(first.wouldBlock == .priceUnknownNotAcknowledged && first.microUsdUpper == nil && !first.priceKnown && first.inputTokens == 45_000)
        try await unpriced.acknowledgeUnpricedModel(backend: openai, model: "gpt-6-preview-0929")
        #expect(try await unpriced.estimateGeneration(request: .weeklyExplanation(course: "DEMO101", week: nil)).wouldBlock == nil)
        // On this computer: free, and question (b) doesn't apply.
        let local = try await mock(.aiLocal).estimateGeneration(request: .weeklyExplanation(course: "DEMO205", week: 1))
        #expect(local.microUsdUpper == 0 && local.priceKnown && local.wouldBlock == nil)
        // A changed disclosure: the full estimate, blocked until it's read again.
        let changed = try await mock(.aiDisclosureChanged).estimateGeneration(request: .weeklyNote)
        #expect(changed.microUsdUpper == 10_500 && changed.wouldBlock == .disclosureNotAcknowledged)
    }

    @Test("an Ollama cloud model changes Ollama's disclosure (a new version to read)")
    func ollamaCloud() async throws {
        let service = mock(.aiLocal)
        let ollama = BackendRef.provider(providerId: "ollama")
        #expect(try await service.aiStatus().backends.first?.state == .ready)
        try await service.setFeatureModel(feature: .weeklyNote, choice: ModelChoice(backend: ollama, model: "gpt-oss:120b-cloud", effort: .lowest))
        let status = try #require(try await service.aiStatus().backends.first)
        #expect(status.disclosure.version == 3551 && status.disclosure.cost == .cloudViaLocal && !status.disclosure.onDevice)
        #expect(status.state == .needsDisclosure && status.problems == [.disclosureChanged])
    }

    @Test("an unreachable endpoint: listing models fails, Test reports the rate limit")
    func errors() async throws {
        let service = mock(.aiErrors)
        let backend = try #require(try await service.aiStatus().backends.first).backend
        do {
            _ = try await service.listModels(backend: backend)
            Issue.record("expected a model error")
        } catch {
            #expect(error.kind == .model && error.modelError == .network)
        }
        let probe = try await service.testModel(backend: backend, model: "demo-model-large")
        #expect(!probe.ok && probe.error == .rateLimited)
        let fine = try await mock(.aiKey).testModel(backend: openai, model: "gpt-6-luna")
        #expect(fine.ok && fine.structuredOutputTier == .nativeSchema)
    }

    @Test("usage per month, with this month's spending against the budget")
    func usage() async throws {
        let service = mock(.aiKey)
        let now = try await service.usageSummary(month: nil)
        #expect(now.month == "2026-09-01" && now.rows.count == 2 && now.totalMicroUsd == 2_000_000)
        #expect(now.budget.spentMicroUsd == 2_000_000 && now.modeA == nil)
        #expect(try await service.usageSummary(month: "2026-08-01").rows.count == 2)
        #expect(try await service.usageSummary(month: "2026-07-01").rows.isEmpty)
        do {
            _ = try await service.usageSummary(month: "someday")
            Issue.record("expected invalid")
        } catch {
            #expect(error.kind == .invalid)
        }
        #expect(try await mock(.aiBudget).aiStatus().budget.spentMicroUsd == 4_960_000)
    }

    @Test("Remove all AI data: keys, choices, usage and AI settings go, the budget resets, sharing answers stay")
    func removeAll() async throws {
        let service = mock(.aiKey)
        try await service.setCourseMaterialSharing(course: "DEMO101", answer: .allowed)
        try await service.setAiOutputLanguage(language: .course)
        #expect(try await service.setPrepareWeeklyNoteOnMonday(on: true).prepareOnMonday)
        let report = try await service.removeAllAiData()
        #expect(try await service.aiOutputLanguage() == .ui)
        #expect(try await !service.weeklyNoteSettings().prepareOnMonday)
        #expect(report.providersRemoved == 1 && report.usageRowsRemoved == 4)
        let status = try await service.aiStatus()
        #expect(status.backends.isEmpty && status.features.allSatisfy { $0.choice == nil })
        #expect(status.budget.monthlyMicroUsd == 5_000_000)
        #expect(try await service.usageSummary(month: nil).rows.isEmpty)
        // DEMO205 stays not shared (its answer wasn't removed): a cloud estimate still says so.
        _ = try await service.addModelProvider(preset: "openai", baseUrl: nil, apiKey: "sk-demo-1234")
        try await service.acknowledgeAiDisclosure(backend: openai, version: 3101)
        try await service.setFeatureModel(feature: .weeklyExplanation, choice: ModelChoice(backend: openai, model: "gpt-6-luna", effort: .lowest))
        #expect(try await service.estimateGeneration(request: .weeklyExplanation(course: "DEMO205", week: nil)).wouldBlock == .materialSharingNotAllowed)
    }

    @Test("the ChatGPT plan isn't offered: every way in refuses")
    func noChatGptPlan() async throws {
        let service = mock()
        #expect(try await service.codexStatus().chatgptPlanOffered == false)
        do {
            _ = try await service.setCodexSource(source: .managed)
            Issue.record("expected blocked")
        } catch {
            #expect(error.blocked == .backendDisabledInThisBuild)
        }
        do {
            _ = try await service.listModels(backend: .codex)
            Issue.record("expected blocked")
        } catch {
            #expect(error.blocked == .backendDisabledInThisBuild)
        }
    }
}
