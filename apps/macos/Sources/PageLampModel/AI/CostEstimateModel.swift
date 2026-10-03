// "≈ $x" before Generate (design §3.5, §7; the Tauri app's GenerateButton): the facade's
// upper-bound estimate for the pending request, and the block it reports before a run. Two blocks
// can be settled where they show: going over this month's budget (this run only) and a model
// without a price (asked once, remembered by the facade). Every other block keeps Generate off
// and says why. The facade gates the run again anyway.

import Foundation
import Observation
import PageLampKit

@Observable @MainActor
public final class CostEstimateModel {
    /// The request the estimate is for; nil while the form is incomplete.
    public private(set) var request: EstimateRequest?
    public private(set) var estimate: CostEstimate?
    public private(set) var failure: PageLampFailure?
    /// A fetch is pending or in flight (Generate waits for it).
    public private(set) var loading = false
    /// "Go over the budget this time": this run only.
    public var overrideBudget = false
    public private(set) var acknowledging = false
    public private(set) var acknowledgeFailure: PageLampFailure?
    /// The AI setup, for the line of a model without a price and its acknowledgement.
    public private(set) var status: AiStatus?

    @ObservationIgnored private let service: any PageLampService
    @ObservationIgnored private let debounce: Duration
    @ObservationIgnored private var pending: Task<Void, Never>?
    /// The request `estimate` is for (it may be the last one's while a new one loads).
    @ObservationIgnored private var estimated: EstimateRequest?

    /// - Parameter debounce: how long a changed request settles before it's estimated.
    public init(service: any PageLampService, debounce: Duration = .milliseconds(300)) {
        self.service = service
        self.debounce = debounce
    }

    // MARK: - The request

    /// A new request (the form changed). The same request isn't estimated again; the last
    /// estimate stays while the new one loads (and goes if the new one fails).
    public func update(_ request: EstimateRequest?) {
        guard request != self.request || (request != nil && estimate == nil && !loading) else { return }
        self.request = request
        overrideBudget = false
        failure = nil
        pending?.cancel()
        guard let request else {
            loading = false
            return
        }
        loading = true
        let debounce = self.debounce
        pending = Task { [weak self] in
            if debounce > .zero {
                try? await Task.sleep(for: debounce)
            }
            guard !Task.isCancelled else { return }
            await self?.load(request)
        }
    }

    /// Estimates the current request again now (after a run, or an acknowledgement).
    public func refresh() async {
        guard let request else { return }
        pending?.cancel()
        loading = true
        await load(request)
    }

    /// Waits for the pending estimate (tests, snapshots).
    public func settle() async {
        await pending?.value
    }

    private func load(_ request: EstimateRequest) async {
        let service = self.service
        do throws(PageLampFailure) {
            async let status = Self.read { () async throws(PageLampFailure) in try await service.aiStatus() }
            let estimate = try await service.estimateGeneration(request: request)
            guard request == self.request else { return }
            self.estimate = estimate
            estimated = request
            failure = nil
            // The tick is for going over this block; with the block gone (a budget raised in
            // Settings ▸ AI), it goes too.
            if estimate.wouldBlock != .budgetReached { overrideBudget = false }
            if case .success(let value) = await status { self.status = value }
        } catch {
            guard request == self.request else { return }
            failure = error
            // Never another request's amount or block beside Generate (a failed re-read of the
            // same request keeps what it had).
            if estimated != request {
                self.estimate = nil
                estimated = nil
            }
        }
        loading = false
    }

    // MARK: - The block

    public var block: BlockReason? { estimate?.wouldBlock }

    /// Blocked, unless it's the budget and the student chose to go over it this time.
    public var blocked: Bool {
        guard let block else { return false }
        return !(block == .budgetReached && overrideBudget)
    }

    /// What a run sends: going over the budget only when that's the block and the student
    /// chose to (the Tauri app's `overrideBudget: block === "budget_reached"`).
    public var goesOverBudget: Bool {
        overrideBudget && block == .budgetReached
    }

    /// Generate can run: a request, its estimate, no block left, nothing loading.
    public var canGenerate: Bool {
        request != nil && estimate != nil && !blocked && !loading
    }

    /// The cost line shows (hidden for the course and setup gates, which have no amount).
    public var showsCost: Bool {
        guard request != nil, estimate != nil else { return false }
        switch block {
        case nil, .disclosureNotAcknowledged?, .budgetReached?, .priceUnknownNotAcknowledged?, .weeklyRunCapReached?:
            return true
        default:
            return false
        }
    }

    /// The feature's chosen model, when it has one.
    public var choice: ModelChoice? {
        guard let feature = request?.aiFeature else { return nil }
        return status?.features.first { $0.feature == feature }?.choice
    }

    /// The kind of the chosen model's backend (what a line without a price says).
    public var backendKind: BackendKind? {
        guard let choice else { return nil }
        return status?.backends.first { $0.backend == choice.backend }?.kind
    }

    /// "Use it anyway": the chosen model runs without a price from now on (asked once).
    public func acknowledgeUnpriced() async {
        guard let choice, !acknowledging else { return }
        acknowledging = true
        acknowledgeFailure = nil
        defer { acknowledging = false }
        do throws(PageLampFailure) {
            try await service.acknowledgeUnpricedModel(backend: choice.backend, model: choice.model)
            await refresh()
        } catch {
            acknowledgeFailure = error
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

extension EstimateRequest {
    /// The feature the request estimates.
    public var aiFeature: AiFeature {
        switch self {
        case .studyPlan: .studyPlan
        case .weeklyExplanation: .weeklyExplanation
        case .weeklyNote: .weeklyNote
        case .courseCalendar: .courseCalendar
        }
    }
}
