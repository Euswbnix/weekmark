// The mock's AI setup calls (Settings ▸ AI, the estimate, usage), on `MockAi`'s state, with the
// facade's rules as the Tauri mock has them: the key is checked and dropped, disclosures are
// acknowledged by version, a feature's model must be one of its backend's, and the estimate
// blocks in the facade's order.

import Foundation
import PageLampKit

extension MockService {
    // MARK: - Presets, providers, local servers

    public func modelProviderPresets() async throws(PageLampFailure) -> [ProviderPreset] {
        await respond("modelProviderPresets")
        return MockAiFixtures.presets
    }

    /// Like the facade: each server's preset, and the provider already added with that preset
    /// and address (a trailing "/" and localhost vs 127.0.0.1 don't count).
    public func detectLocalServers() async throws(PageLampFailure) -> [LocalServer] {
        await respond("detectLocalServers")
        func normal(_ url: String) -> String {
            var url = url.replacingOccurrences(of: "//localhost", with: "//127.0.0.1")
            while url.hasSuffix("/") { url.removeLast() }
            return url
        }
        return MockAiFixtures.localServers.map { server in
            let preset = MockAi.presetName(server.kind)
            let added = db.features.ai.providers.first {
                $0.preset == preset && normal($0.baseUrl) == normal(server.baseUrl)
            }
            return LocalServer(
                kind: server.kind, preset: preset, baseUrl: server.baseUrl, running: server.running,
                providerId: added?.providerId
            )
        }
    }

    public func aiStatus() async throws(PageLampFailure) -> AiStatus {
        await respond("aiStatus")
        return AiStatus(
            backends: db.features.ai.providers.map(backendStatus),
            providers: db.features.ai.providers,
            features: MockAiFixtures.features.map { FeatureRouting(feature: $0, choice: db.features.ai.routing[$0]) },
            budget: budgetStatus(),
            // Not in any build until OpenAI confirms in writing: no Codex backend.
            chatgptPlanOffered: false
        )
    }

    public func addModelProvider(preset presetId: String, baseUrl: String?, apiKey: String?) async throws(PageLampFailure) -> ModelProviderRecord {
        await respond("addModelProvider")
        guard let preset = MockAiFixtures.preset(presetId) else {
            throw PageLampFailure(kind: .notFound, message: "Unknown preset \"\(presetId)\".")
        }
        let url = try Self.validBaseUrl(preset, baseUrl)
        let key = preset.needsKey ? apiKey : nil
        try Self.checkKey(preset, url: url, key: key)
        let id = preset.id == "custom" ? "custom-" + MockAi.shortHash(url) : preset.id
        if db.features.ai.providers.contains(where: { $0.providerId == id }) {
            throw PageLampFailure(kind: .invalid, message: "\(preset.label) is already set up.")
        }
        // The key itself goes nowhere: only its last 4 characters are kept.
        let record = ModelProviderRecord(
            providerId: id, preset: preset.id,
            label: preset.id == "custom" ? (URL(string: url)?.host() ?? url) : preset.label,
            wire: preset.wire, baseUrl: url, keyLast4: key.map(MockAi.last4), onDevice: MockAi.isLoopback(url),
            createdAt: now()
        )
        db.features.ai.providers.append(record)
        return record
    }

    public func updateModelProviderKey(providerId: String, apiKey: String) async throws(PageLampFailure) -> ModelProviderRecord {
        await respond("updateModelProviderKey")
        let index = try providerIndex(providerId)
        let record = db.features.ai.providers[index]
        guard let preset = MockAiFixtures.preset(record.preset) else { throw Self.noProvider(providerId) }
        try Self.checkKey(preset, url: record.baseUrl, key: apiKey)
        let updated = ModelProviderRecord(
            providerId: record.providerId, preset: record.preset, label: record.label, wire: record.wire,
            baseUrl: record.baseUrl, keyLast4: MockAi.last4(apiKey), onDevice: record.onDevice,
            createdAt: record.createdAt
        )
        db.features.ai.providers[index] = updated
        return updated
    }

    public func removeModelProvider(providerId: String) async throws(PageLampFailure) {
        await respond("removeModelProvider")
        let index = try providerIndex(providerId)
        db.features.ai.providers.remove(at: index)
        db.features.ai.acknowledged["provider:\(providerId)"] = nil
        for (feature, choice) in db.features.ai.routing where MockAi.key(choice.backend) == "provider:\(providerId)" {
            db.features.ai.routing[feature] = nil
        }
    }

    // MARK: - Models

    public func listModels(backend: BackendRef) async throws(PageLampFailure) -> [ModelInfo] {
        await respond("listModels")
        let record = try provider(of: backend)
        if db.features.ai.unreachable {
            throw PageLampFailure(kind: .model, message: "Couldn't reach llm.demo.test.", modelError: .network)
        }
        return MockAiFixtures.models[record.preset] ?? []
    }

    public func testModel(backend: BackendRef, model: String) async throws(PageLampFailure) -> ProbeReport {
        await respond("testModel")
        let record = try provider(of: backend)
        let failed = { (error: ModelErrorKind) in
            ProbeReport(ok: false, latencyMs: 0, structuredOutputTier: nil, thinkingAlwaysOn: false, error: error)
        }
        if db.features.ai.unreachable { return failed(.rateLimited) }
        guard let info = (MockAiFixtures.models[record.preset] ?? []).first(where: { $0.id == model }) else {
            return failed(.modelNotFound)
        }
        return ProbeReport(
            ok: true, latencyMs: info.onDevice ? 2_300 : 900,
            structuredOutputTier: record.wire == .openaiChat ? .jsonObject : .nativeSchema,
            thinkingAlwaysOn: info.reasoningAlwaysOn, error: nil
        )
    }

    public func setFeatureModel(feature: AiFeature, choice: ModelChoice?) async throws(PageLampFailure) {
        await respond("setFeatureModel")
        if let choice {
            let record = try provider(of: choice.backend)
            guard (MockAiFixtures.models[record.preset] ?? []).contains(where: { $0.id == choice.model }) else {
                throw PageLampFailure(kind: .invalid, message: "No model \"\(choice.model)\".")
            }
        }
        db.features.ai.routing[feature] = choice
    }

    // MARK: - Acknowledgements and the budget

    public func acknowledgeAiDisclosure(backend: BackendRef, version: UInt32) async throws(PageLampFailure) {
        await respond("acknowledgeAiDisclosure")
        let record = try provider(of: backend)
        guard version == disclosure(of: record).version else {
            throw PageLampFailure(kind: .invalid, message: "The disclosure changed; read it again.")
        }
        db.features.ai.acknowledged[MockAi.key(backend)] = version
    }

    public func acknowledgeUnpricedModel(backend: BackendRef, model: String) async throws(PageLampFailure) {
        await respond("acknowledgeUnpricedModel")
        _ = try provider(of: backend)
        db.features.ai.unpricedAcks.insert("\(MockAi.key(backend))/\(model)")
    }

    public func setMonthlyBudget(microUsd: UInt64?) async throws(PageLampFailure) {
        await respond("setMonthlyBudget")
        db.features.ai.budget = microUsd
    }

    // MARK: - The estimate ("≈ $x" before Generate, and whether the run would be blocked)

    public func estimateGeneration(request: EstimateRequest) async throws(PageLampFailure) -> CostEstimate {
        await respond("estimateGeneration")
        return try estimate(request)
    }

    /// The facade's estimate: nothing chosen, the course's own rules, a week with nothing to read,
    /// a weekly note with nothing to write about and a study plan with no course to plan for
    /// block with no amount; the other blocks keep
    /// it (only an acknowledgement or the budget stops the run). An explanation's `include` is priced as its run sends it: only the
    /// graded-looking materials of that week it lifts and reads (an id naming nothing there, or a
    /// material read anyway, adds nothing).
    func estimate(_ request: EstimateRequest) throws(PageLampFailure) -> CostEstimate {
        let gateBlocked = { (reason: BlockReason) in
            CostEstimate(
                microUsdUpper: nil, inputTokens: 0, maxOutputTokens: 0, reasoningAllowance: 0,
                repairPossible: false, priceKnown: false, wouldBlock: reason
            )
        }
        guard let choice = db.features.ai.routing[request.aiFeature] else { return gateBlocked(.noModelChosen) }
        let record = try provider(of: choice.backend)
        // The note's and the plan's context come next in the facade, before their other blocks.
        if case .weeklyNote = request, noteWeek(at: now()).isEmpty { return gateBlocked(.nothingToWrite) }
        if case .studyPlan(_, let courses) = request, try planCourses(courses).isEmpty {
            return gateBlocked(.noCourseToPlan)
        }
        let info = (MockAiFixtures.models[record.preset] ?? []).first { $0.id == choice.model }
        let onDevice = info?.onDevice ?? false
        let courses: [String] = switch request {
        case .weeklyExplanation(let course, _, _): [course]
        case .courseCalendar(let courses): courses
        case .studyPlan, .weeklyNote: []
        }
        for course in courses {
            if let reason = try courseGate(course, onDevice: onDevice) { return gateBlocked(reason) }
        }
        // Like the facade's week context: a week with nothing to read is refused before a run.
        var lifted = 0
        if case .weeklyExplanation(let course, let week, let include) = request {
            let materials = explanationWeek(db.courses[try courseIndex(course)], week).materials
            let read = Self.explanationSelection(materials, include: include).read
            if read.isEmpty {
                return gateBlocked(.noReadableMaterials)
            }
            lifted = Self.liftedIncludes(read, include).count
        }
        let work = MockAiFixtures.workload(request, lifted: lifted)
        let status = backendStatus(record)
        let reasoning = max(MockAiFixtures.reasoning(choice.effort), info?.reasoningAlwaysOn == true ? 8_000 : 0)
        let repair = record.wire == .openaiChat
        var upper: UInt64?
        if onDevice {
            upper = 0
        } else if let price = MockAiFixtures.prices[choice.model], status.kind == .apiKey {
            let amount = work.input * price.input + (work.output + reasoning) * price.output
            upper = (amount * (repair ? 2 : 1) + 999_999) / 1_000_000
        }
        var block: BlockReason?
        if status.state != .ready {
            block = .disclosureNotAcknowledged
        } else if status.kind == .apiKey, upper == nil,
                  !db.features.ai.unpricedAcks.contains("\(MockAi.key(choice.backend))/\(choice.model)") {
            block = .priceUnknownNotAcknowledged
        } else if let upper, upper > 0, let budget = db.features.ai.budget, spentThisMonth() + upper > budget {
            block = .budgetReached
        }
        return CostEstimate(
            microUsdUpper: upper, inputTokens: work.input, maxOutputTokens: work.output,
            reasoningAllowance: reasoning, repairPossible: repair, priceKnown: upper != nil, wouldBlock: block
        )
    }

    /// The course's own rules (weekly explanations and course calendars only).
    private func courseGate(_ reference: String, onDevice: Bool) throws(PageLampFailure) -> BlockReason? {
        let course = db.courses[try courseIndex(reference)].course
        if course.hidden { return .courseHidden }
        if course.aiPolicy == .prohibited { return .coursePolicyProhibited }
        if !course.aiAccess { return .courseAiTurnedOff }
        let sharing = db.features.sharing[course.id] ?? course.materialSharing
        if !onDevice, sharing == .notAllowed { return .materialSharingNotAllowed }
        return nil
    }

    // MARK: - Usage

    public func usageSummary(month: String?) async throws(PageLampFailure) -> UsageSummary {
        await respond("usageSummary")
        let today = calendar.dateComponents([.year, .month], from: now())
        var ago = 0
        if let month {
            let parts = month.split(separator: "-").compactMap { Int($0) }
            guard parts.count >= 2, parts[0] > 0, (1...12).contains(parts[1]) else {
                throw PageLampFailure(kind: .invalid, message: "Expected a date like 2026-09-01.")
            }
            ago = ((today.year ?? 0) - parts[0]) * 12 + ((today.month ?? 1) - parts[1])
        }
        let first = calendar.date(from: DateComponents(year: today.year, month: (today.month ?? 1) - ago, day: 1)) ?? now()
        let rows = db.features.ai.usage.filter { $0.monthsAgo == ago }.map(\.row)
        let total = rows.filter { $0.costBasis == .priced }.compactMap(\.microUsd).reduce(0, +)
        return UsageSummary(
            month: IsoDate.string(from: first, calendar: calendar), rows: rows, totalMicroUsd: total,
            budget: budgetStatus(), modeA: nil
        )
    }

    public func removeAllAiData() async throws(PageLampFailure) -> RemoveAiDataReport {
        await respond("removeAllAiData")
        let report = RemoveAiDataReport(
            providersRemoved: UInt32(db.features.ai.providers.count),
            generationsRemoved: UInt32(db.features.runs.explanations.count + db.features.runs.notes.count),
            usageRowsRemoved: UInt32(db.features.ai.usage.count), backupRemoved: false
        )
        // Keys, choices, acknowledgements, the ledger, the explanations and the weekly notes go;
        // the budget goes back to its default, and so do the AI settings (the answers' language,
        // Monday's note and its try, the ChatGPT plan's weekly cap and Codex source). The reminders
        // about sharing materials show again, but the courses' answers stay (the facade's rule).
        db.features.ai = MockAi()
        db.features.outputLanguage = .ui
        db.features.prepareNoteOnMonday = false
        db.features.noteTriedOn = nil
        db.features.weeklyCap = nil
        db.features.codexSource = .managed
        db.features.runs.explanations = []
        db.features.runs.sharingReminded = []
        db.features.runs.notes = []
        return report
    }

    // MARK: - Helpers

    func budgetStatus() -> BudgetStatus {
        BudgetStatus(monthlyMicroUsd: db.features.ai.budget, spentMicroUsd: spentThisMonth(), warnAtPercent: 80)
    }

    private func spentThisMonth() -> UInt64 {
        db.features.ai.usage
            .filter { $0.monthsAgo == 0 && $0.row.costBasis == .priced }
            .compactMap(\.row.microUsd)
            .reduce(0, +)
    }

    /// A provider backend's state, from its key and the disclosure it acknowledged.
    private func backendStatus(_ record: ModelProviderRecord) -> AiBackendStatus {
        let key = "provider:\(record.providerId)"
        let facts = disclosure(of: record)
        let acked = db.features.ai.acknowledged[key]
        let needsKey = MockAiFixtures.preset(record.preset)?.needsKey ?? false
        var problems: [BackendProblem] = []
        if needsKey, record.keyLast4 == nil { problems.append(.keyMissing) }
        if let acked, acked != facts.version { problems.append(.disclosureChanged) }
        let state: BackendState = problems.contains(.keyMissing) ? .needsSetup : acked != facts.version ? .needsDisclosure : .ready
        return AiBackendStatus(
            backend: .provider(providerId: record.providerId), label: record.label,
            kind: record.onDevice ? .local : .apiKey, state: state, problems: problems, disclosure: facts,
            disclosureAcknowledged: acked
        )
    }

    /// The preset's facts; for Ollama, the cloud's once a feature runs one of its cloud models.
    private func disclosure(of record: ModelProviderRecord) -> DisclosureFacts {
        guard let base = MockAiFixtures.preset(record.preset)?.dataPolicy else {
            return MockAiFixtures.facts(0, record.label, terms: nil, training: .unknown, retention: .providerTerms, minAge: nil, cost: .apiBilling)
        }
        guard record.preset == "ollama" else { return base }
        let cloud = db.features.ai.routing.values.contains { choice in
            MockAi.key(choice.backend) == "provider:\(record.providerId)"
                && (MockAiFixtures.models["ollama"] ?? []).first { $0.id == choice.model }?.onDevice == false
        }
        return cloud ? MockAiFixtures.ollamaCloudFacts(base) : base
    }

    private func providerIndex(_ id: String) throws(PageLampFailure) -> Int {
        guard let index = db.features.ai.providers.firstIndex(where: { $0.providerId == id }) else {
            throw Self.noProvider(id)
        }
        return index
    }

    /// The facade's gate before a run: the estimate's block (only going over the budget can be
    /// overridden), then who the run goes to.
    func aiRun(_ request: EstimateRequest, overrideBudget: Bool) throws(PageLampFailure) -> MockAiRun {
        let estimate = try estimate(request)
        if let block = estimate.wouldBlock, !(block == .budgetReached && overrideBudget) {
            throw PageLampFailure(kind: .blocked, message: "The AI gate stopped this run.", blocked: block)
        }
        guard let choice = db.features.ai.routing[request.aiFeature] else {
            throw PageLampFailure(kind: .blocked, message: "No model chosen.", blocked: .noModelChosen)
        }
        let record = try provider(of: choice.backend)
        return MockAiRun(backendLabel: record.label, model: choice.model, onDevice: record.onDevice)
    }

    /// A provider backend's record; the other backends aren't in this build.
    private func provider(of backend: BackendRef) throws(PageLampFailure) -> ModelProviderRecord {
        guard case .provider(let id) = backend else {
            throw PageLampFailure(kind: .blocked, message: "Not available in this build.", blocked: .backendDisabledInThisBuild)
        }
        return db.features.ai.providers[try providerIndex(id)]
    }

    static func noProvider(_ id: String) -> PageLampFailure {
        PageLampFailure(kind: .notFound, message: "No model provider \"\(id)\".")
    }

    /// The preset's address, or the student's where it can be edited: https, or http on this computer.
    private static func validBaseUrl(_ preset: ProviderPreset, _ given: String?) throws(PageLampFailure) -> String {
        let trimmed = given?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let raw = preset.baseUrlEditable && !trimmed.isEmpty ? trimmed : preset.defaultBaseUrl ?? ""
        guard !raw.isEmpty else { throw PageLampFailure(kind: .invalid, message: "Enter the endpoint's address.") }
        guard let url = URL(string: raw), let scheme = url.scheme?.lowercased(), url.host() != nil else {
            throw PageLampFailure(kind: .invalid, message: "That is not a web address.")
        }
        guard scheme == "https" || (scheme == "http" && MockAi.isLoopback(raw)) else {
            throw PageLampFailure(kind: .invalid, message: "Use an https:// address (http only on this computer).")
        }
        var normalized = raw
        while normalized.hasSuffix("/") { normalized.removeLast() }
        return normalized
    }

    /// The facade's key check, with the mock's pretend answers from the provider.
    private static func checkKey(_ preset: ProviderPreset, url: String, key: String?) throws(PageLampFailure) {
        let trimmed = key?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let address = url.replacingOccurrences(of: "https://", with: "").replacingOccurrences(of: "http://", with: "")
        if MockAiFixtures.codingPlanHosts.contains(where: { address.hasPrefix($0) })
            || MockAiFixtures.codingPlanKeyPrefixes.contains(where: { trimmed.hasPrefix($0) }) {
            throw PageLampFailure(
                kind: .blocked, message: "Demo Vendor: “This plan's keys may only be used in the vendor's own coding tools.”",
                blocked: .codingPlanKey
            )
        }
        guard preset.needsKey else { return }
        guard !trimmed.isEmpty else { throw PageLampFailure(kind: .invalid, message: "Enter the API key.") }
        if trimmed.contains("bad") {
            throw PageLampFailure(kind: .model, message: "The provider rejected this key.", modelError: .authRejected)
        }
        if url.contains("offline") {
            throw PageLampFailure(kind: .model, message: "Couldn't reach the provider.", modelError: .network)
        }
    }
}

/// Who a mock run goes to.
struct MockAiRun: Sendable {
    let backendLabel: String
    let model: String
    let onDevice: Bool
}
