// "Monthly budget for API keys" (the Tauri app's BudgetField; design D18: US$5 by default, a
// warning at 80%): the amount in dollars or No budget, and this month's spending. The facade
// checks each run against it; this only shows it and saves a new amount. Shown while an API key
// is set up. The parent gives it the saved amount as its identity, so a new amount refills it.

import SwiftUI
import PageLampKit
import PageLampModel

struct AiBudgetSection: View {
    let ai: AiSettingsModel
    let budget: BudgetStatus

    @Environment(\.l10n) private var l10n
    /// Offscreen renders can't draw a text field or a progress bar: the harness gets stand-ins.
    @Environment(\.drawsControlStandIns) private var standIns
    @State private var amount: String
    @State private var noLimit: Bool
    /// The last save found an amount it couldn't read.
    @State private var invalid = false

    init(ai: AiSettingsModel, budget: BudgetStatus) {
        self.ai = ai
        self.budget = budget
        _amount = State(initialValue: AiMoney.inputValue(budget.monthlyMicroUsd))
        _noLimit = State(initialValue: budget.monthlyMicroUsd == nil)
    }

    var body: some View {
        Section {
            LabeledContent(l10n("ai.budget.amount")) {
                HStack(alignment: .firstTextBaseline, spacing: PLSpace.s3) {
                    amountField
                        .frame(width: Self.fieldWidth)
                    Toggle(l10n("ai.budget.noLimit"), isOn: $noLimit)
                        .toggleStyle(.checkbox)
                    Button(l10n("mac.ai.saveBudget")) { Task { await save() } }
                        .disabled(ai.savingBudget)
                }
            }
            if invalid {
                Text(l10n("ai.budget.invalid"))
                    .foregroundStyle(PLColor.danger)
            }
            if let failure = ai.budgetFailure {
                Text(l10n.aiError(failure))
                    .foregroundStyle(PLColor.danger)
                    .fixedSize(horizontal: false, vertical: true)
            }
            spending
        } header: {
            Text(l10n("ai.budget.title"))
        } footer: {
            Text(l10n("ai.budget.hint"))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    @ViewBuilder
    private var spending: some View {
        let spent = l10n.usd(microUsd: budget.spentMicroUsd)
        VStack(alignment: .leading, spacing: PLSpace.s1) {
            if let saved = budget.monthlyMicroUsd {
                Text(l10n("ai.budget.used", ["spent": spent, "budget": l10n.usd(microUsd: saved)]))
                if saved > 0 {
                    let percent = min(100, Int((Double(budget.spentMicroUsd) / Double(saved) * 100).rounded()))
                    Group {
                        if standIns {
                            BarStandIn(fraction: Double(percent) / 100)
                        } else {
                            ProgressView(value: Double(percent), total: 100)
                        }
                    }
                    .accessibilityLabel(l10n("ai.budget.progress"))
                    .accessibilityValue((Double(percent) / 100).formatted(.percent.locale(l10n.locale)))
                    if percent >= Int(budget.warnAtPercent) {
                        Text(l10n("ai.budget.warn", ["percent": l10n.number(percent)]))
                            .font(PLType.body.font.weight(.semibold))
                    }
                }
            } else {
                Text(l10n("ai.budget.usedNoBudget", ["spent": spent]))
            }
        }
    }

    @ViewBuilder
    private var amountField: some View {
        if standIns {
            FieldStandIn(text: amount)
        } else {
            TextField(l10n("ai.budget.amount"), text: $amount)
                .labelsHidden()
                .disabled(noLimit)
                .onSubmit { Task { await save() } }
                .accessibilityHint(invalid ? l10n("ai.budget.invalid") : "")
        }
    }

    private func save() async {
        let micro: UInt64?
        if noLimit {
            micro = nil
        } else if let parsed = AiMoney.parse(amount) {
            micro = parsed
        } else {
            invalid = true
            AccessibilityNotification.Announcement(l10n("ai.budget.invalid")).post()
            return
        }
        invalid = false
        if await ai.saveBudget(microUsd: micro) {
            AccessibilityNotification.Announcement(l10n("ai.budget.saved")).post()
        }
    }

    private static let fieldWidth: CGFloat = 100
}
