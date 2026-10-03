// "AI usage" (the Tauri app's UsageSection): per month, from this computer's ledger, the runs,
// tokens and estimated cost of each model and feature (counts only, never what was sent), the
// total, and this month's budget. A grid rather than a table: offscreen renders draw it, and it
// fits the tab's width. Without the ChatGPT plan, its rows and weekly runs aren't shown.

import SwiftUI
import PageLampKit
import PageLampModel

struct AiUsageSection: View {
    let ai: AiSettingsModel

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n

    var body: some View {
        Section {
            Picker(l10n("ai.usage.month"), selection: Binding(
                get: { ai.usageMonth },
                set: { month in Task { await ai.chooseUsageMonth(month) } }
            )) {
                ForEach(ai.months, id: \.self) { month in
                    Text(monthName(month)).tag(month)
                }
            }
            if let failure = ai.usageFailure {
                Text(l10n.aiError(failure))
                    .foregroundStyle(PLColor.danger)
                    .fixedSize(horizontal: false, vertical: true)
            } else if let usage = ai.usage {
                let rows = shownRows(usage)
                if rows.isEmpty {
                    Text(l10n("ai.usage.empty", ["month": monthName(ai.usageMonth)]))
                        .foregroundStyle(.secondary)
                } else {
                    table(rows, total: usage.totalMicroUsd)
                    if rows.contains(where: \.estimated) {
                        Text(l10n("ai.usage.estimatedNote"))
                            .font(PLType.callout.font)
                            .foregroundStyle(.secondary)
                    }
                }
                if ai.usageMonth == ai.months.first {
                    budgetLine(usage)
                }
            }
        } header: {
            Text(l10n("ai.usage.title"))
        } footer: {
            Text(l10n("ai.usage.description"))
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: - The table

    private func table(_ rows: [UsageRow], total: UInt64) -> some View {
        Grid(alignment: .leading, horizontalSpacing: PLSpace.s3, verticalSpacing: PLSpace.s2) {
            GridRow {
                header("ai.usage.columns.model")
                header("ai.usage.columns.feature")
                header("ai.usage.columns.runs").gridColumnAlignment(.trailing)
                header("ai.usage.columns.tokens").gridColumnAlignment(.trailing)
                header("ai.usage.columns.cost").gridColumnAlignment(.trailing)
            }
            Divider()
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                GridRow(alignment: .firstTextBaseline) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(row.model)
                        Text(row.backendLabel)
                            .font(PLType.subheadline.font)
                            .foregroundStyle(.secondary)
                    }
                    Text(l10n.aiFeature(row.feature))
                    Text(l10n.number(row.runs))
                    Text(l10n("ai.usage.tokens", [
                        "input": l10n.compactTokens(row.inputTokens),
                        "output": l10n.compactTokens(row.outputTokens),
                    ]))
                    Text(cost(row))
                }
            }
            Divider()
            GridRow {
                Text(l10n("ai.usage.total"))
                    .font(PLType.body.font.weight(.semibold))
                    .gridCellColumns(4)
                Text(approximately(rows.contains(where: \.estimated), l10n.usd(microUsd: total)))
                    .font(PLType.body.font.weight(.semibold))
            }
        }
        // Modifiers go on the grid, not on a GridRow (VoiceOver reads the column names, then each
        // row cell by cell).
        .monospacedDigit()
        .accessibilityElement(children: .contain)
        .accessibilityLabel(l10n("ai.usage.caption", ["month": monthName(ai.usageMonth)]))
    }

    private func header(_ key: String) -> some View {
        Text(l10n(key))
            .font(PLType.callout.font)
            .foregroundStyle(.secondary)
            .accessibilityAddTraits(.isHeader)
    }

    private func cost(_ row: UsageRow) -> String {
        switch row.costBasis {
        case .freeOnDevice: l10n("ai.usage.free")
        case .unpriced: l10n("ai.usage.noPrice")
        case .plan: l10n("ai.usage.plan")
        case .priced: approximately(row.estimated, l10n.usd(microUsd: row.microUsd ?? 0))
        }
    }

    /// "≈ $1.60" for an estimate.
    private func approximately(_ estimated: Bool, _ amount: String) -> String {
        estimated ? "≈ \(amount)" : amount
    }

    // MARK: - This month's budget

    @ViewBuilder
    private func budgetLine(_ usage: UsageSummary) -> some View {
        if let saved = usage.budget.monthlyMicroUsd {
            Text(l10n("ai.budget.used", [
                "spent": l10n.usd(microUsd: usage.budget.spentMicroUsd),
                "budget": l10n.usd(microUsd: saved),
            ]))
            .font(PLType.callout.font)
        }
        if planOffered, let modeA = usage.modeA {
            Group {
                if let cap = modeA.weeklyCap {
                    Text(l10n("ai.usage.modeA", ["runs": l10n.number(modeA.runsThisWeek), "cap": l10n.number(cap)]))
                } else {
                    Text(l10n("ai.usage.modeANoCap", ["runs": l10n.number(modeA.runsThisWeek)]))
                }
            }
            .font(PLType.callout.font)
        }
    }

    // MARK: - Helpers

    private var planOffered: Bool {
        ai.status?.chatgptPlanOffered ?? false
    }

    /// Without the ChatGPT plan its rows aren't shown (the total stays the facade's).
    private func shownRows(_ usage: UsageSummary) -> [UsageRow] {
        planOffered ? usage.rows : usage.rows.filter { $0.costBasis != .plan }
    }

    /// "September 2026" / "2026年9月" for "2026-09-01".
    private func monthName(_ month: String) -> String {
        let parts = month.split(separator: "-").compactMap { Int($0) }
        guard parts.count == 3,
              let date = model.calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: 1))
        else { return month }
        var calendar = model.calendar
        calendar.locale = l10n.locale
        return date.formatted(
            Date.FormatStyle(locale: l10n.locale, calendar: calendar, timeZone: calendar.timeZone).year().month(.wide)
        )
    }
}
