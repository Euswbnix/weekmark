// The AI screens' words and numbers (model-access design §3.8, §7): the facade's codes turned
// into the shared i18next keys (`ai.*`, the Tauri app's), an AI failure explained by its code
// (never by the backend's English), US dollars from micro-dollars, compact token counts, and a
// backend's data-policy line. Swift maps codes; policy stays in the facade.

import Foundation
import PageLampKit

/// The facade's snake_case names of its codes (the keys' last part), by exhaustive switches.
public enum AiCodes {
    public static func name(_ reason: BlockReason) -> String {
        switch reason {
        case .coursePolicyProhibited: "course_policy_prohibited"
        case .courseAiTurnedOff: "course_ai_turned_off"
        case .courseHidden: "course_hidden"
        case .noReadableMaterials: "no_readable_materials"
        case .materialSharingNotAllowed: "material_sharing_not_allowed"
        case .codingPlanKey: "coding_plan_key"
        case .disclosureNotAcknowledged: "disclosure_not_acknowledged"
        case .noModelChosen: "no_model_chosen"
        case .budgetReached: "budget_reached"
        case .priceUnknownNotAcknowledged: "price_unknown_not_acknowledged"
        case .weeklyRunCapReached: "weekly_run_cap_reached"
        case .backendDisabledInThisBuild: "backend_disabled_in_this_build"
        case .nothingToWrite: "nothing_to_write"
        case .noCourseToPlan: "no_course_to_plan"
        }
    }

    public static func name(_ kind: ModelErrorKind) -> String {
        switch kind {
        case .notSignedIn: "not_signed_in"
        case .authRejected: "auth_rejected"
        case .billingOrQuota: "billing_or_quota"
        case .usageLimit: "usage_limit"
        case .rateLimited: "rate_limited"
        case .overloaded: "overloaded"
        case .invalidRequest: "invalid_request"
        case .modelNotFound: "model_not_found"
        case .contextTooLong: "context_too_long"
        case .refused: "refused"
        case .contentFiltered: "content_filtered"
        case .network: "network"
        case .timeout: "timeout"
        case .badOutput: "bad_output"
        case .runtimeMissing: "runtime_missing"
        case .runtimeVerifyFailed: "runtime_verify_failed"
        case .runtimeOutdated: "runtime_outdated"
        case .unsupported: "unsupported"
        }
    }

    public static func name(_ kind: BackendKind) -> String {
        switch kind {
        case .apiKey: "api_key"
        case .local: "local"
        case .codex: "codex"
        case .claudeCode: "claude_code"
        }
    }

    public static func name(_ state: BackendState) -> String {
        switch state {
        case .ready: "ready"
        case .needsSetup: "needs_setup"
        case .needsDisclosure: "needs_disclosure"
        case .unavailable: "unavailable"
        }
    }

    public static func name(_ problem: BackendProblem) -> String {
        switch problem {
        case .keyMissing: "key_missing"
        case .serverNotRunning: "server_not_running"
        case .modelMissing: "model_missing"
        case .disclosureChanged: "disclosure_changed"
        case .notSignedIn: "not_signed_in"
        case .runtimeMissing: "runtime_missing"
        }
    }

    public static func name(_ feature: AiFeature) -> String {
        switch feature {
        case .studyPlan: "study_plan"
        case .weeklyExplanation: "weekly_explanation"
        case .weeklyNote: "weekly_note"
        case .courseCalendar: "course_calendar"
        }
    }

    public static func name(_ effort: Effort) -> String {
        switch effort {
        case .lowest: "lowest"
        case .low: "low"
        case .medium: "medium"
        case .high: "high"
        }
    }

    public static func name(_ sent: SentData) -> String {
        switch sent {
        case .structure: "structure"
        case .materialText: "material_text"
        }
    }

    public static func name(_ cost: CostKind) -> String {
        switch cost {
        case .apiBilling: "api_billing"
        case .planCredits: "plan_credits"
        case .freeOnDevice: "free_on_device"
        case .cloudViaLocal: "cloud_via_local"
        case .selfHosted: "self_hosted"
        }
    }

    public static func name(_ training: TrainingFact) -> String {
        switch training {
        case .noTraining: "no_training"
        case .mayTrain: "may_train"
        case .mayTrainFreeTier: "may_train_free_tier"
        case .unknown: "unknown"
        }
    }

    /// `storedDays` is "stored_days", a plural key (pass its days as the count).
    public static func name(_ retention: RetentionFact) -> String {
        switch retention {
        case .notStored: "not_stored"
        case .storedDays: "stored_days"
        case .providerTerms: "provider_terms"
        case .onDevice: "on_device"
        }
    }

    public static func name(_ kind: LocalServerKind) -> String {
        switch kind {
        case .ollama: "ollama"
        case .lmStudio: "lm_studio"
        }
    }

    /// A run's stage: the key's last part in each feature's `running.stage.*`.
    public static func name(_ stage: GenStage) -> String {
        switch stage {
        case .buildingContext: "building_context"
        case .waitingForModel: "waiting_for_model"
        case .validating: "validating"
        case .repairing: "repairing"
        case .scheduling: "scheduling"
        }
    }

    public static func name(_ code: PlanWarningCode) -> String {
        switch code {
        case .gradedWorkLeftOut: "graded_work_left_out"
        case .unknownMaterialsDropped: "unknown_materials_dropped"
        }
    }

    public static func name(_ reason: UnscheduledReason) -> String {
        switch reason {
        case .outsideHorizon: "outside_horizon"
        case .noStudyDays: "no_study_days"
        case .noTimeBeforeLatest: "no_time_before_latest"
        case .tooManyItems: "too_many_items"
        }
    }

    /// Why a material was left out of an explanation: the key's last part in
    /// `explain.result.leftOutReason.*`.
    public static func name(_ reason: LeftOutReason) -> String {
        switch reason {
        case .looksLikeAssessment: "looks_like_assessment"
        case .externalLink: "external_link"
        case .noText: "no_text"
        case .overBudget: "over_budget"
        }
    }

    /// A backend's key, as the facade's routing and acknowledgements name it.
    public static func key(_ backend: BackendRef) -> String {
        switch backend {
        case .provider(let providerId): "provider:\(providerId)"
        case .codex: "codex"
        case .claudeCode: "claude_code"
        }
    }
}

extension L10n {
    /// A failed AI call in words, by its code (the Tauri app's `useAiErrorText`): why it was
    /// blocked, what the model's service said (with the wait when it asked for one), else the
    /// failure's kind.
    public func aiError(_ failure: PageLampFailure) -> String {
        if failure.kind == .blocked, let reason = failure.blocked {
            let key = "ai.blocked.\(AiCodes.name(reason))"
            if has(key) { return self(key) }
        }
        if failure.kind == .model, let kind = failure.modelError {
            if kind == .rateLimited, let seconds = failure.retryAfterSecs, seconds > 0 {
                return self("ai.modelError.rate_limited_after", ["seconds": number(seconds)])
            }
            let key = "ai.modelError.\(AiCodes.name(kind))"
            if has(key) { return self(key) }
        }
        return failure.localizedDescription(in: self)
    }

    /// A lead sentence and its reason ("Couldn't load the AI settings. The provider rejected the
    /// key."), joined with a space like the Tauri app.
    public func sentences(_ lead: String, _ reason: String) -> String {
        "\(lead) \(reason)"
    }

    /// "Data policy: Sent to OpenAI · …" (a line's name for VoiceOver, then the line).
    public func labelled(_ label: String, _ value: String) -> String {
        label + self("common.punctuation.colon") + value
    }

    /// "Study plans", "Weekly explanations", …
    public func aiFeature(_ feature: AiFeature) -> String {
        self("ai.features.name.\(AiCodes.name(feature))")
    }

    /// US dollars with two decimals in this locale, half a cent rounded up like `Intl` ("$0.07",
    /// "US$5.00").
    public func usd(microUsd: UInt64) -> String {
        (Decimal(microUsd) / 1_000_000).formatted(
            .currency(code: "USD").precision(.fractionLength(2)).rounded(rule: .toNearestOrAwayFromZero).locale(locale)
        )
    }

    /// "45K", "3.4M" (the usage table's and the estimate's token counts).
    public func compactTokens(_ count: UInt64) -> String {
        Int(count).formatted(.number.notation(.compactName).precision(.fractionLength(0...1)).locale(locale))
    }

    // MARK: The estimate before Generate

    /// The cost line: what it shows, and what VoiceOver reads ("Estimated cost: ≈ $0.07 at
    /// most"; the label goes only before an amount). Nil when there's nothing to say (a model
    /// without a price on a backend that has no line).
    public func estimateLine(_ estimate: CostEstimate, backendKind: BackendKind?) -> (text: String, spoken: String)? {
        if let upper = estimate.microUsdUpper {
            if upper == 0 {
                let free = self("ai.estimate.free")
                return (free, free)
            }
            let amount = estimateAmount(microUsd: upper)
            return (amount, sentences(self("ai.estimate.label"), amount))
        }
        let key: String? = switch backendKind {
        case .codex?: "ai.codex.costLine"
        case .local?: "ai.estimate.cloudNoPrice"
        case .apiKey?: "ai.estimate.noPrice"
        case .claudeCode?, nil: nil
        }
        return key.map { (self($0), self($0)) }
    }

    /// "≈ $0.07 at most" (rounded up to the cent), or "≈ less than $0.01".
    public func estimateAmount(microUsd: UInt64) -> String {
        guard microUsd >= 10_000 else {
            return self("ai.estimate.lessThan", ["amount": usd(microUsd: 10_000)])
        }
        let cents = (microUsd + 9_999) / 10_000
        return self("ai.estimate.upTo", ["amount": usd(microUsd: cents * 10_000)])
    }

    /// "Up to 45K tokens in and 2.5K out" (the output counts the reasoning allowance).
    public func estimateDetails(_ estimate: CostEstimate) -> String? {
        guard estimate.inputTokens > 0 else { return nil }
        return self("ai.estimate.details", [
            "input": compactTokens(estimate.inputTokens),
            "output": compactTokens(estimate.maxOutputTokens + estimate.reasoningAllowance),
        ])
    }

    // MARK: The AI label

    /// The label on every AI output and every copy (Canvas §2E): "AI-generated · OpenAI ·
    /// gpt-6-luna · Oct 1, 2026 · 43,600 tokens" (≈ when the count is estimated). The date is
    /// the calendar's (local) day.
    public func aiLabel(_ meta: GenerationMeta, calendar: Calendar) -> String {
        let tokens = number(meta.usage.inputTokens + meta.usage.outputTokens)
        return aiLabel(
            backend: meta.backendLabel, model: meta.model, createdAt: meta.createdAt, calendar: calendar,
            tokens: self(meta.estimated ? "ai.aiLabel.tokensApprox" : "ai.aiLabel.tokens", ["tokens": tokens])
        )
    }

    /// A saved output's label (no token count), e.g. an accepted study plan's.
    public func aiLabel(_ label: AiLabel, calendar: Calendar) -> String {
        aiLabel(backend: label.backendLabel, model: label.model, createdAt: label.createdAt, calendar: calendar, tokens: nil)
    }

    private func aiLabel(backend: String, model: String, createdAt: Date, calendar: Calendar, tokens: String?) -> String {
        let date = createdAt.formatted(
            Date.FormatStyle(date: .abbreviated, time: .omitted, locale: locale, calendar: calendar, timeZone: calendar.timeZone)
        )
        return ([self("ai.aiLabel.prefix"), backend, model, date] + (tokens.map { [$0] } ?? [])).joined(separator: " · ")
    }

    /// A backend's data policy in one line (design §2.5): where the data goes, training, how
    /// long it's kept, the age, and the cost; on this computer, only that it stays here and is free.
    public func dataPolicy(_ facts: DisclosureFacts) -> String {
        if facts.onDevice {
            return [self("ai.policy.staysHere"), self("ai.policy.cost.free_on_device")].joined(separator: " · ")
        }
        var parts = [
            self("ai.policy.sentTo", ["name": facts.recipient.name]),
            self("ai.policy.training.\(AiCodes.name(facts.training))"),
        ]
        if case .storedDays(let days) = facts.retention {
            parts.append(plural("ai.policy.retention.stored_days", count: Int(days)))
        } else {
            parts.append(self("ai.policy.retention.\(AiCodes.name(facts.retention))"))
        }
        if let age = facts.minAge, age > 0 {
            parts.append(self(facts.guardianPermission ? "ai.policy.ageGuardian" : "ai.policy.age", ["age": number(age)]))
        }
        parts.append(self("ai.policy.cost.\(AiCodes.name(facts.cost))"))
        return parts.joined(separator: " · ")
    }
}

/// Dollars as the student types them, and back (the Tauri app's `parseUsd` / `usdInputValue`).
public enum AiMoney {
    /// "5", "2.50", "$3", "US$ 3", "$ 5", "1,000", "5." → micro-dollars; nil for anything else
    /// (negative, more than two decimals, empty).
    public static func parse(_ text: String) -> UInt64? {
        var value = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if value.lowercased().hasPrefix("us$") {
            value.removeFirst(3)
        } else if value.hasPrefix("$") {
            value.removeFirst()
        }
        value = value.replacingOccurrences(of: ",", with: "").trimmingCharacters(in: .whitespacesAndNewlines)
        let parts = value.split(separator: ".", omittingEmptySubsequences: false)
        guard (1...2).contains(parts.count), let whole = parts.first, !whole.isEmpty,
              whole.allSatisfy({ $0.isASCII && $0.isNumber }), let dollars = UInt64(whole)
        else { return nil }
        var cents: UInt64 = 0
        if parts.count == 2 {
            let fraction = parts[1]
            guard fraction.count <= 2, fraction.allSatisfy({ $0.isASCII && $0.isNumber }) else { return nil }
            cents = UInt64(fraction.padding(toLength: 2, withPad: "0", startingAt: 0)) ?? 0
        }
        let (micro, overflow) = dollars.multipliedReportingOverflow(by: 1_000_000)
        guard !overflow else { return nil }
        let (total, overflowCents) = micro.addingReportingOverflow(cents * 10_000)
        return overflowCents ? nil : total
    }

    /// The budget field's text for a saved amount, rounded to the cent ("5.00"); empty without one.
    public static func inputValue(_ microUsd: UInt64?) -> String {
        guard let microUsd else { return "" }
        let cents = microUsd / 10_000 + (microUsd % 10_000 >= 5_000 ? 1 : 0)
        return String(format: "%llu.%02llu", cents / 100, cents % 100)
    }
}
