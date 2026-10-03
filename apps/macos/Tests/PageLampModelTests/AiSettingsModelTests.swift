// Settings ▸ AI's model on the mock: it reads what the facade says and re-reads after each
// change; an API key goes to the facade and nowhere else (only its last 4 characters come back);
// failures are kept by what failed.

import Foundation
import Synchronization
import PageLamp
import PageLampKit
import PageLampModel
import Testing

/// A clock the test moves.
private final class MovableClock: Sendable {
    private let date: Mutex<Date>

    init(_ date: Date) {
        self.date = Mutex(date)
    }

    var now: Date { date.withLock { $0 } }

    func set(_ new: Date) {
        date.withLock { $0 = new }
    }
}

/// Holds `testModel` until the test lets it answer.
private actor TestHold {
    private var waiting: CheckedContinuation<Void, Never>?
    private var started: [CheckedContinuation<Void, Never>] = []
    private var isHeld = false

    func hold() async {
        isHeld = true
        for continuation in started { continuation.resume() }
        started = []
        await withCheckedContinuation { waiting = $0 }
    }

    /// Waits until a test call is being held.
    func held() async {
        if isHeld { return }
        await withCheckedContinuation { started.append($0) }
    }

    func release() {
        waiting?.resume()
        waiting = nil
    }
}

private struct HeldTests: ForwardingService {
    let base: any PageLampService
    let hold: TestHold

    func testModel(backend: BackendRef, model: String) async throws(PageLampFailure) -> ProbeReport {
        await hold.hold()
        return try await base.testModel(backend: backend, model: model)
    }
}

@Suite("AI settings model")
@MainActor
struct AiSettingsModelTests {
    func loaded(_ scenario: MockScenario) async -> (AiSettingsModel, MockService) {
        let service = MockService(scenario: scenario, timing: .instant, calendar: TestClock.calendar, now: { TestClock.now })
        let ai = AiSettingsModel(service: service, clock: { TestClock.now }, calendar: TestClock.calendar)
        await ai.load()
        return (ai, service)
    }

    @Test("nothing set up: the key presets, this computer's servers, the months and the usage")
    func loadsEmpty() async throws {
        let (ai, _) = await loaded(.demo)
        #expect(ai.statusFailure == nil)
        #expect(ai.providerBackends.isEmpty && ai.usableBackends.isEmpty && !ai.hasApiKey)
        #expect(ai.keyPresets.allSatisfy { $0.needsKey })
        #expect(ai.keyPresets.map(\.id).contains("openai") && !ai.keyPresets.map(\.id).contains("ollama"))
        let servers = try #require(ai.localServers)
        #expect(servers.map(\.kind) == [.ollama, .lmStudio])
        #expect(servers.map(\.running) == [true, false])
        #expect(ai.outputLanguage != nil)
        #expect(ai.months.count == 13)
        #expect(ai.months.first == "2026-09-01" && ai.months.last == "2025-09-01")
        #expect(ai.usageMonth == "2026-09-01" && ai.usage != nil)
        #expect(!ai.codexSignedIn)
    }

    @Test("adding a key: the key isn't kept, the backend asks for its disclosure, then is ready")
    func addKey() async throws {
        let (ai, _) = await loaded(.demo)
        let preset = try #require(ai.keyPresets.first { $0.id == "openai" })
        let key = "sk-demo-never-kept-7Qx2"
        let record = try #require(await ai.addProvider(preset: preset, baseUrl: "", key: key))
        #expect(record.keyLast4 == "7Qx2")
        // Nothing the model holds has the key in it.
        for child in Mirror(reflecting: ai).children {
            #expect(!String(describing: child.value).contains(key), "\(child.label ?? "?") holds the key")
        }
        let backend = try #require(ai.providerBackends.first)
        #expect(backend.state == .needsDisclosure && ai.hasApiKey)
        #expect(ai.provider(of: backend)?.keyLast4 == "7Qx2")
        #expect(ai.backend(key: "provider:openai")?.label == backend.label)
        #expect(await ai.acknowledge(backend))
        #expect(ai.providerBackends.first?.state == .ready)
        #expect(ai.models["provider:openai"]?.isEmpty == false)
        // Replacing it keeps only the new last 4.
        #expect(await ai.replaceKey(providerId: "openai", key: "sk-demo-new-Wx9Y"))
        #expect(ai.status?.providers.first?.keyLast4 == "Wx9Y")
    }

    @Test("a refused key: the failure by its code, and nothing added")
    func refusedKey() async throws {
        let (ai, _) = await loaded(.demo)
        let preset = try #require(ai.keyPresets.first { $0.id == "openai" })
        #expect(await ai.addProvider(preset: preset, baseUrl: "", key: "sk-bad-0000") == nil)
        #expect(ai.keyFailure?.modelError == .authRejected)
        #expect(await ai.addProvider(preset: preset, baseUrl: "", key: "sk-sp-demo-0000") == nil)
        #expect(ai.keyFailure?.blocked == .codingPlanKey)
        #expect(await ai.addProvider(preset: preset, baseUrl: "", key: "   ") == nil)
        #expect(ai.keyFailure?.kind == .invalid)
        #expect(ai.providerBackends.isEmpty)
        ai.clearKeyFailure()
        #expect(ai.keyFailure == nil)
    }

    @Test("a running local server is added as the facade's preset; detection then names its provider")
    func localServer() async throws {
        let (ai, _) = await loaded(.demo)
        let ollama = try #require(ai.localServers?.first { $0.kind == .ollama })
        #expect(!ai.isAdded(ollama) && ollama.preset == "ollama")
        let record = try #require(await ai.useLocalServer(ollama))
        #expect(record.onDevice && record.keyLast4 == nil && record.preset == "ollama")
        // The facade's detection now names the provider it was added as.
        let added = try #require(ai.localServers?.first { $0.kind == .ollama })
        #expect(ai.isAdded(added) && added.providerId == record.providerId)
        #expect(ai.localServers?.first { $0.kind == .lmStudio }?.providerId == nil)
        #expect(ai.providerBackends.first?.kind == .local)
        #expect(!ai.hasApiKey)
        #expect(await ai.removeProvider(record.providerId))
        #expect(ai.localServers?.allSatisfy { !ai.isAdded($0) } == true)
    }

    @Test("a feature's model and effort are saved and re-read; Test reports the probe")
    func featureModels() async throws {
        let (ai, _) = await loaded(.aiKey)
        let choice = try #require(ai.choice(for: .studyPlan))
        #expect(ai.chosenModel(for: .studyPlan)?.id == choice.model)
        await ai.setEffort(.studyPlan, .high)
        #expect(ai.choice(for: .studyPlan)?.effort == .high && ai.choice(for: .studyPlan)?.model == choice.model)
        await ai.test(.studyPlan)
        guard case .report(let report)? = ai.tests[.studyPlan] else {
            Issue.record("expected a probe report")
            return
        }
        #expect(report.ok && report.latencyMs > 0)
        // A new choice clears the last test; none unsets it and keeps no failure.
        await ai.setModel(.studyPlan, backend: nil, model: nil)
        #expect(ai.choice(for: .studyPlan) == nil && ai.tests[.studyPlan] == nil)
        #expect(ai.modelChoiceFailures[.studyPlan] == nil)
        // A model the backend doesn't offer is refused, for that feature only.
        await ai.setModel(.weeklyNote, backend: choice.backend, model: "no-such-model")
        #expect(ai.modelChoiceFailures[.weeklyNote] != nil && ai.modelChoiceFailures[.studyPlan] == nil)
    }

    @Test("a server that can't be reached: Test says why")
    func unreachable() async throws {
        let (ai, _) = await loaded(.aiErrors)
        await ai.test(.studyPlan)
        guard case .report(let report)? = ai.tests[.studyPlan] else {
            Issue.record("expected a probe report")
            return
        }
        #expect(!report.ok && report.error == .rateLimited)
    }

    @Test("the budget is saved or removed, and re-read")
    func budget() async throws {
        let (ai, _) = await loaded(.aiKey)
        #expect(await ai.saveBudget(microUsd: 2_500_000))
        #expect(ai.status?.budget.monthlyMicroUsd == 2_500_000)
        #expect(ai.usage?.budget.monthlyMicroUsd == 2_500_000)
        #expect(await ai.saveBudget(microUsd: nil))
        #expect(ai.status?.budget.monthlyMicroUsd == nil)
    }

    @Test("usage per month; the last month asked for wins")
    func usage() async throws {
        let (ai, _) = await loaded(.aiKey)
        let current = try #require(ai.usage)
        #expect(!current.rows.isEmpty)
        await ai.loadUsage(month: "2025-09-01")
        #expect(ai.usageMonth == "2025-09-01" && ai.usage?.rows.isEmpty == true)
    }

    @Test("removing a provider, then everything")
    func removal() async throws {
        let (ai, _) = await loaded(.aiKey)
        let provider = try #require(ai.status?.providers.first)
        #expect(await ai.removeProvider(provider.providerId))
        #expect(ai.status?.providers.contains { $0.providerId == provider.providerId } == false)
        #expect(await ai.removeProvider(provider.providerId) == false)
        #expect(ai.removeFailure?.kind == .notFound)

        let (all, _) = await loaded(.aiKey)
        #expect(await all.removeAll())
        #expect(all.status?.providers.isEmpty == true && all.providerBackends.isEmpty)
        #expect(all.status?.budget.monthlyMicroUsd == 5_000_000)
        #expect(all.tests.isEmpty && all.removeAllFailure == nil)
    }

    @Test("a Test result that arrives after the model changed is dropped; one Test at a time")
    func staleTest() async throws {
        let hold = TestHold()
        let service = HeldTests(
            base: MockService(scenario: .aiKey, timing: .instant, calendar: TestClock.calendar, now: { TestClock.now }),
            hold: hold
        )
        let ai = AiSettingsModel(service: service, clock: { TestClock.now }, calendar: TestClock.calendar)
        await ai.load()
        let choice = try #require(ai.choice(for: .studyPlan))
        let first = Task { await ai.test(.studyPlan) }
        await hold.held()
        #expect(ai.testing.contains(.studyPlan))
        // A second press while it runs does nothing.
        await ai.test(.studyPlan)
        await ai.setEffort(.studyPlan, .high)
        #expect(!ai.testing.contains(.studyPlan) && ai.tests[.studyPlan] == nil)
        await hold.release()
        await first.value
        #expect(ai.tests[.studyPlan] == nil && !ai.testing.contains(.studyPlan))
        #expect(ai.choice(for: .studyPlan)?.model == choice.model)
    }

    @Test("usage follows this month into a new one, unless the student picked a month still listed")
    func usageMonthMoves() async throws {
        let clock = MovableClock(TestClock.now)
        let service = MockService(scenario: .aiKey, timing: .instant, calendar: TestClock.calendar, now: { TestClock.now })
        let ai = AiSettingsModel(service: service, clock: { clock.now }, calendar: TestClock.calendar)
        await ai.load()
        #expect(ai.usageMonth == "2026-09-01")
        clock.set(try #require(TestClock.calendar.date(byAdding: .day, value: 7, to: TestClock.now)))
        await ai.load()
        #expect(ai.usageMonth == "2026-10-01" && ai.months.first == "2026-10-01")
        await ai.chooseUsageMonth("2026-08-01")
        await ai.load()
        #expect(ai.usageMonth == "2026-08-01")
        // A year on, August 2026 is no longer listed: back to this month.
        clock.set(try #require(TestClock.calendar.date(byAdding: .year, value: 1, to: TestClock.now)))
        await ai.load()
        #expect(ai.usageMonth == "2027-09-01")
    }

    @Test("reading everything again clears the failures of earlier changes")
    func failuresClear() async throws {
        let (ai, _) = await loaded(.aiKey)
        #expect(await ai.removeProvider("nope") == false && ai.removeFailure != nil)
        #expect(await ai.saveBudget(microUsd: nil))
        await ai.load()
        #expect(ai.removeFailure == nil && ai.budgetFailure == nil && ai.serverFailures.isEmpty)
    }

    @Test("a new service (Debug ▸ Data Source, the live facade opening) is announced to the screens")
    func serviceGeneration() async throws {
        let model = AppModel(
            dataMode: .mock(.demo), strings: .app, settings: InMemorySettingsStore(language: .english),
            timing: AppModel.Timing(finishedCapsule: .seconds(60), failedCapsule: .seconds(60), mock: .instant),
            calendar: TestClock.calendar, clock: { TestClock.now }, notificationCenter: NotificationCenter()
        )
        let before = model.serviceGeneration
        await model.useMock(.aiKey)
        #expect(model.serviceGeneration > before)
    }
}
