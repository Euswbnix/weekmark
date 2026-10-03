// Settings ▸ AI (model-access design §7; the Tauri app's Settings → AI models, output language
// and usage): what the facade says about the student's AI setup, and every change to it. The
// facade decides everything (states, problems, facts, prices, blocks); this only reads, asks and
// re-reads. An API key passes through `addProvider` / `replaceKey` to the facade (which checks it
// and keeps it in the keychain) and is never kept here: only its last 4 characters come back.

import Foundation
import Observation
import PageLampKit

@Observable @MainActor
public final class AiSettingsModel {
    /// A "Test" of a feature's model: the probe's report, or why the call failed.
    public enum TestOutcome: Sendable {
        case report(ProbeReport)
        case failed(PageLampFailure)
    }

    // MARK: Reads

    public private(set) var status: AiStatus?
    public private(set) var statusFailure: PageLampFailure?
    /// The presets that take a key ("Add an API key"), in the facade's order.
    public private(set) var keyPresets: [ProviderPreset] = []
    public private(set) var localServers: [LocalServer]?
    public private(set) var localFailure: PageLampFailure?
    public private(set) var detecting = false
    /// Each usable backend's models, by backend key.
    public private(set) var models: [String: [ModelInfo]] = [:]
    public private(set) var modelFailures: [String: PageLampFailure] = [:]
    public private(set) var outputLanguage: OutputLanguage?
    /// Codex is signed in to ChatGPT (Remove All then also signs it out, and says so).
    public private(set) var codexSignedIn = false
    public private(set) var usage: UsageSummary?
    public private(set) var usageFailure: PageLampFailure?
    /// The usage month shown ("YYYY-MM-01").
    public private(set) var usageMonth: String
    /// Monday's weekly note: the opt-in and whether the note's model allows it (nil until read).
    public private(set) var noteSettings: WeeklyNoteSettings?
    /// "≈ $x" of a note with its model (the opt-in's cost line).
    public private(set) var noteEstimate: CostEstimate?
    /// This week has nothing for the note to write about, so there is no "≈ $x" to show.
    public private(set) var noteWeekEmpty = false
    /// The student picked `usageMonth` (else it follows this month, also into a new one).
    @ObservationIgnored private var usageMonthPicked = false

    // MARK: Changes in flight and their failures

    public private(set) var addingKey = false
    public private(set) var keyFailure: PageLampFailure?
    /// The local server being added (its kind's name) and what went wrong with each.
    public private(set) var addingServer: LocalServerKind?
    public private(set) var serverFailures: [LocalServerKind: PageLampFailure] = [:]
    public private(set) var removeFailure: PageLampFailure?
    /// Why saving a feature's model or effort failed, by feature.
    public private(set) var modelChoiceFailures: [AiFeature: PageLampFailure] = [:]
    public private(set) var testing: Set<AiFeature> = []
    public private(set) var tests: [AiFeature: TestOutcome] = [:]
    public private(set) var acknowledging = false
    public private(set) var acknowledgeFailure: PageLampFailure?
    public private(set) var savingBudget = false
    public private(set) var budgetFailure: PageLampFailure?
    public private(set) var removeAllFailure: PageLampFailure?
    public private(set) var savingNoteSetting = false
    public private(set) var noteSettingFailure: PageLampFailure?

    /// Each feature's Test run: a new choice (or Remove All) makes a late result stale.
    @ObservationIgnored private var testRuns: [AiFeature: Int] = [:]
    @ObservationIgnored private let service: any PageLampService
    @ObservationIgnored private let clock: @Sendable () -> Date
    @ObservationIgnored private let calendar: Calendar

    public init(service: any PageLampService, clock: @escaping @Sendable () -> Date, calendar: Calendar) {
        self.service = service
        self.clock = clock
        self.calendar = calendar
        usageMonth = Self.months(from: clock(), calendar: calendar).first ?? ""
    }

    // MARK: - Loading

    /// Everything the tab shows. Each part fails on its own; failures of earlier changes go.
    public func load() async {
        removeFailure = nil
        serverFailures = [:]
        modelChoiceFailures = [:]
        acknowledgeFailure = nil
        budgetFailure = nil
        removeAllFailure = nil
        noteSettingFailure = nil
        // This month, unless the student picked one that's still listed (a new month moves on).
        let months = self.months
        if !usageMonthPicked || !months.contains(usageMonth) {
            usageMonthPicked = false
            usageMonth = months.first ?? usageMonth
        }
        let service = self.service
        async let presets = Self.read { () async throws(PageLampFailure) in try await service.modelProviderPresets() }
        async let language = Self.read { () async throws(PageLampFailure) in try await service.aiOutputLanguage() }
        async let codex = Self.read { () async throws(PageLampFailure) in try await service.codexStatus() }
        await refreshStatus()
        if case .success(let value) = await presets { keyPresets = value.filter(\.needsKey) }
        if case .success(let value) = await language { outputLanguage = value }
        // Unread or failed: the removal's text doesn't mention ChatGPT (the Tauri app's rule).
        if case .success(let value) = await codex { codexSignedIn = value.login.state != .signedOut }
        await detectLocalServers()
        await loadUsage(month: usageMonth)
    }

    /// Re-reads the AI status and the models of every usable backend, and Monday's note (a
    /// model change can allow or pause it, and changes its cost).
    public func refreshStatus() async {
        do throws(PageLampFailure) {
            let status = try await service.aiStatus()
            self.status = status
            statusFailure = nil
            await loadModels(for: status.backends.filter(Self.isUsable))
        } catch {
            statusFailure = error
        }
        await loadNoteSettings()
    }

    private func loadNoteSettings() async {
        let service = self.service
        async let settings = Self.read { () async throws(PageLampFailure) in try await service.weeklyNoteSettings() }
        async let estimate = Self.read { () async throws(PageLampFailure) in
            try await service.estimateGeneration(request: .weeklyNote)
        }
        // Unread: the section stays as it was (hidden before the first read, like the Tauri app).
        if case .success(let value) = await settings { noteSettings = value }
        switch await estimate {
        // A week with nothing to write about has no estimate: the model's own facts say the rest
        // (no amount, and no "no price" for a priced model).
        case .success(let value):
            noteWeekEmpty = value.wouldBlock == .nothingToWrite
            noteEstimate = noteWeekEmpty ? nil : value
        case .failure:
            noteWeekEmpty = false
            noteEstimate = nil
        }
    }

    /// What Monday's note runs on and what it costs (the opt-in's hint).
    public struct NoteCost: Equatable, Sendable {
        public var backend: String
        public var model: String
        /// On this computer, as the chosen model's facts say (else the backend's kind).
        public var onDevice: Bool
        public var priceKnown: Bool
        public var upper: UInt64?
        /// This week has nothing to write about: the line says so instead of an amount.
        public var weekEmpty: Bool

        public init(
            backend: String, model: String, onDevice: Bool, priceKnown: Bool, upper: UInt64?, weekEmpty: Bool = false
        ) {
            self.backend = backend
            self.model = model
            self.onDevice = onDevice
            self.priceKnown = priceKnown
            self.upper = upper
            self.weekEmpty = weekEmpty
        }
    }

    /// Monday's note's model and cost; nil without a model for the note. On this computer by the
    /// model's facts (a cloud model through Ollama isn't); without them, by the backend's kind, as
    /// the Tauri app decides. The price from "≈ $x" for a note, else the model's facts.
    public var noteCost: NoteCost? {
        guard let choice = choice(for: .weeklyNote), let backend = backend(key: AiCodes.key(choice.backend)) else {
            return nil
        }
        let facts = chosenModel(for: .weeklyNote)
        return NoteCost(
            backend: backend.label, model: choice.model, onDevice: facts?.onDevice ?? (backend.kind == .local),
            priceKnown: noteEstimate?.priceKnown ?? facts?.priceKnown ?? true, upper: noteEstimate?.microUsdUpper,
            weekEmpty: noteWeekEmpty
        )
    }

    /// "Prepare my weekly note when I open PageLamp on Monday". The facade refuses turning it on
    /// unless the note's model allows it; turning it off always works. False with
    /// `noteSettingFailure` set.
    public func setPrepareOnMonday(_ on: Bool) async -> Bool {
        guard !savingNoteSetting else { return false }
        savingNoteSetting = true
        noteSettingFailure = nil
        defer { savingNoteSetting = false }
        do throws(PageLampFailure) {
            noteSettings = try await service.setPrepareWeeklyNoteOnMonday(on: on)
            return true
        } catch {
            noteSettingFailure = error
            return false
        }
    }

    public func detectLocalServers() async {
        detecting = true
        defer { detecting = false }
        do throws(PageLampFailure) {
            localServers = try await service.detectLocalServers()
            localFailure = nil
        } catch {
            localFailure = error
        }
    }

    private func loadModels(for backends: [AiBackendStatus]) async {
        for backend in backends {
            let key = AiCodes.key(backend.backend)
            do throws(PageLampFailure) {
                models[key] = try await service.listModels(backend: backend.backend)
                modelFailures[key] = nil
            } catch {
                modelFailures[key] = error
            }
        }
    }

    // MARK: - Backends

    /// The backends a feature can use: ready, or waiting for their disclosure.
    public var usableBackends: [AiBackendStatus] {
        status?.backends.filter(Self.isUsable) ?? []
    }

    /// The backends listed as rows: API keys and local models (the ChatGPT plan isn't offered).
    public var providerBackends: [AiBackendStatus] {
        status?.backends.filter { if case .provider = $0.backend { true } else { false } } ?? []
    }

    public var hasApiKey: Bool {
        status?.backends.contains { $0.kind == .apiKey } ?? false
    }

    public func provider(of backend: AiBackendStatus) -> ModelProviderRecord? {
        guard case .provider(let id) = backend.backend else { return nil }
        return status?.providers.first { $0.providerId == id }
    }

    public func backend(key: String) -> AiBackendStatus? {
        status?.backends.first { AiCodes.key($0.backend) == key }
    }

    /// Adds a provider with its key. The key goes to the facade and nowhere else. Returns the new
    /// provider (its disclosure comes next), or nil with `keyFailure` set.
    public func addProvider(preset: ProviderPreset, baseUrl: String, key: String) async -> ModelProviderRecord? {
        guard !addingKey else { return nil }
        addingKey = true
        keyFailure = nil
        defer { addingKey = false }
        let address = baseUrl.trimmingCharacters(in: .whitespacesAndNewlines)
        do throws(PageLampFailure) {
            let record = try await service.addModelProvider(
                preset: preset.id,
                baseUrl: preset.baseUrlEditable && !address.isEmpty ? address : nil,
                apiKey: key.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            await refreshStatus()
            return record
        } catch {
            keyFailure = error
            return nil
        }
    }

    /// Replaces a provider's key (checked by the facade first). False with `keyFailure` set.
    public func replaceKey(providerId: String, key: String) async -> Bool {
        guard !addingKey else { return false }
        addingKey = true
        keyFailure = nil
        defer { addingKey = false }
        do throws(PageLampFailure) {
            _ = try await service.updateModelProviderKey(
                providerId: providerId, apiKey: key.trimmingCharacters(in: .whitespacesAndNewlines)
            )
            await refreshStatus()
            return true
        } catch {
            keyFailure = error
            return false
        }
    }

    /// The sheet closed: its failure goes with it.
    public func clearKeyFailure() {
        keyFailure = nil
    }

    public func removeProvider(_ providerId: String) async -> Bool {
        removeFailure = nil
        do throws(PageLampFailure) {
            try await service.removeModelProvider(providerId: providerId)
            await refreshStatus()
            // A local server's "Added" comes from the facade's detection.
            await detectLocalServers()
            return true
        } catch {
            removeFailure = error
            return false
        }
    }

    // MARK: - Local servers

    /// Whether a detected server is already a provider (the facade names it).
    public func isAdded(_ server: LocalServer) -> Bool {
        server.providerId != nil
    }

    /// Adds a running local server (nothing is downloaded). Returns the provider, or nil with its
    /// row's failure set.
    public func useLocalServer(_ server: LocalServer) async -> ModelProviderRecord? {
        guard addingServer == nil else { return nil }
        addingServer = server.kind
        serverFailures[server.kind] = nil
        defer { addingServer = nil }
        do throws(PageLampFailure) {
            let record = try await service.addModelProvider(preset: server.preset, baseUrl: server.baseUrl, apiKey: nil)
            await refreshStatus()
            await detectLocalServers()
            return record
        } catch {
            serverFailures[server.kind] = error
            return nil
        }
    }

    // MARK: - Models per feature

    public func choice(for feature: AiFeature) -> ModelChoice? {
        status?.features.first { $0.feature == feature }?.choice
    }

    /// The chosen model's facts, when its backend's list has it.
    public func chosenModel(for feature: AiFeature) -> ModelInfo? {
        guard let choice = choice(for: feature) else { return nil }
        return models[AiCodes.key(choice.backend)]?.first { $0.id == choice.model }
    }

    /// Chooses a model (keeping the effort), or none.
    public func setModel(_ feature: AiFeature, backend: BackendRef?, model: String?) async {
        let choice = backend.flatMap { backend in
            model.map { ModelChoice(backend: backend, model: $0, effort: self.choice(for: feature)?.effort ?? .lowest) }
        }
        await save(feature, choice)
    }

    public func setEffort(_ feature: AiFeature, _ effort: Effort) async {
        guard let current = choice(for: feature) else { return }
        await save(feature, ModelChoice(backend: current.backend, model: current.model, effort: effort))
    }

    private func save(_ feature: AiFeature, _ choice: ModelChoice?) async {
        modelChoiceFailures[feature] = nil
        forgetTest(feature)
        do throws(PageLampFailure) {
            try await service.setFeatureModel(feature: feature, choice: choice)
            await refreshStatus()
        } catch {
            modelChoiceFailures[feature] = error
        }
    }

    /// One small real call to the feature's model (one at a time per feature). A result that
    /// arrives after the model changed is dropped.
    public func test(_ feature: AiFeature) async {
        guard let choice = choice(for: feature), !testing.contains(feature) else { return }
        let run = (testRuns[feature] ?? 0) + 1
        testRuns[feature] = run
        testing.insert(feature)
        let outcome: TestOutcome
        do throws(PageLampFailure) {
            outcome = .report(try await service.testModel(backend: choice.backend, model: choice.model))
        } catch {
            outcome = .failed(error)
        }
        guard testRuns[feature] == run else { return }
        testing.remove(feature)
        tests[feature] = outcome
    }

    /// The feature's model changed (or everything went): its last Test and one in flight no
    /// longer apply.
    private func forgetTest(_ feature: AiFeature) {
        testRuns[feature, default: 0] += 1
        testing.remove(feature)
        tests[feature] = nil
    }

    // MARK: - Disclosures

    /// Records that the student read this version of the backend's facts. False with
    /// `acknowledgeFailure` set.
    public func acknowledge(_ backend: AiBackendStatus) async -> Bool {
        guard !acknowledging else { return false }
        acknowledging = true
        acknowledgeFailure = nil
        defer { acknowledging = false }
        do throws(PageLampFailure) {
            try await service.acknowledgeAiDisclosure(backend: backend.backend, version: backend.disclosure.version)
            await refreshStatus()
            return true
        } catch {
            acknowledgeFailure = error
            return false
        }
    }

    public func clearAcknowledgeFailure() {
        acknowledgeFailure = nil
    }

    // MARK: - Budget, usage, output language, removal

    /// The monthly budget for API keys (nil: no limit). False with `budgetFailure` set.
    public func saveBudget(microUsd: UInt64?) async -> Bool {
        savingBudget = true
        budgetFailure = nil
        defer { savingBudget = false }
        do throws(PageLampFailure) {
            try await service.setMonthlyBudget(microUsd: microUsd)
            await refreshStatus()
            if usageMonth == months.first { await loadUsage(month: usageMonth) }
            return true
        } catch {
            budgetFailure = error
            return false
        }
    }

    /// This month and the 12 before it, newest first ("YYYY-MM-01").
    public var months: [String] {
        Self.months(from: clock(), calendar: calendar)
    }

    /// The month picker: the student's month, kept until it's no longer listed.
    public func chooseUsageMonth(_ month: String) async {
        usageMonthPicked = true
        await loadUsage(month: month)
    }

    public func loadUsage(month: String) async {
        usageMonth = month
        do throws(PageLampFailure) {
            let summary = try await service.usageSummary(month: month)
            guard month == usageMonth else { return }
            usage = summary
            usageFailure = nil
        } catch {
            guard month == usageMonth else { return }
            usageFailure = error
        }
    }

    public func setOutputLanguage(_ language: OutputLanguage) async {
        do throws(PageLampFailure) {
            try await service.setAiOutputLanguage(language: language)
            outputLanguage = language
        } catch {
            // The choice stays as it was (the Tauri app shows nothing either).
        }
    }

    /// Removes every AI key, choice, acknowledgement and output. False with `removeAllFailure` set.
    public func removeAll() async -> Bool {
        removeAllFailure = nil
        do throws(PageLampFailure) {
            _ = try await service.removeAllAiData()
            models = [:]
            modelFailures = [:]
            for feature in Set(tests.keys).union(testing) { forgetTest(feature) }
            await load()
            return true
        } catch {
            removeAllFailure = error
            return false
        }
    }

    // MARK: - Helpers

    private static func isUsable(_ backend: AiBackendStatus) -> Bool {
        backend.state == .ready || backend.state == .needsDisclosure
    }

    static func months(from now: Date, calendar: Calendar) -> [String] {
        let start = calendar.date(from: calendar.dateComponents([.year, .month], from: now)) ?? now
        return (0...12).compactMap { ago in
            calendar.date(byAdding: .month, value: -ago, to: start).map { date in
                let parts = calendar.dateComponents([.year, .month], from: date)
                return String(format: "%04d-%02d-01", parts.year ?? 0, parts.month ?? 1)
            }
        }
    }

    private nonisolated static func read<T: Sendable>(
        _ call: @Sendable () async throws(PageLampFailure) -> T
    ) async -> Result<T, PageLampFailure> {
        do throws(PageLampFailure) {
            return .success(try await call())
        } catch {
            return .failure(error)
        }
    }
}
