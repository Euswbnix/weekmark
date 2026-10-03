// The words of Plan your study (the shared `plan.*` strings): the run's headline and stages, the
// draft's summary, days, minutes, warnings and what was left unscheduled.

import Foundation
import PageLampKit

public struct PlanText: Sendable {
    public let l10n: L10n
    public let calendar: Calendar

    public init(l10n: L10n, calendar: Calendar) {
        self.l10n = l10n
        self.calendar = calendar
    }

    // MARK: The run

    /// "Writing with OpenAI · gpt-6-luna" once the run has said which, else "Starting…".
    public func headline(_ progress: GenerationRun<GeneratedStudyPlan>.Progress) -> String {
        guard let backend = progress.backend, let model = progress.model else { return l10n("plan.running.starting") }
        return l10n("plan.running.withModel", ["backend": backend, "model": model])
    }

    /// The stage ("Placing tasks on your study days"), or "Writing your plan" before the first.
    public func stage(_ stage: GenStage?) -> String {
        stage.map { l10n("plan.running.stage.\(AiCodes.name($0))") } ?? l10n("plan.running.writingNow")
    }

    // MARK: The draft

    /// "12 tasks, Sep 25 to Oct 8".
    public func summary(_ plan: StudyPlan) -> String {
        l10n.plural("plan.draft.summary", count: plan.items.count, [
            "start": date(plan.horizonStart),
            "end": date(plan.horizonEnd),
        ])
    }

    /// "Sep 25" (a date in a sentence).
    public func date(_ iso: String) -> String {
        guard let day = IsoDate.date(from: iso, calendar: calendar) else { return iso }
        return day.formatted(style.month(.abbreviated).day())
    }

    /// "Fri, Sep 25" (a day in the table).
    public func day(_ iso: String) -> String {
        guard let day = IsoDate.date(from: iso, calendar: calendar) else { return iso }
        return day.formatted(style.weekday(.abbreviated).month(.abbreviated).day())
    }

    /// "45 min".
    public func minutes(_ minutes: UInt32) -> String {
        l10n.plural("plan.draft.minutes", count: Int(minutes))
    }

    /// "1 task was left out because it looked like an answer to graded work."
    public func warning(_ warning: PlanWarning) -> String {
        l10n.plural("plan.warnings.\(AiCodes.name(warning.code))", count: Int(warning.count))
    }

    /// "no time left before it's due".
    public func reason(_ reason: UnscheduledReason) -> String {
        l10n("plan.unscheduled.reason.\(AiCodes.name(reason))")
    }

    /// "Planned from structure only (no material text was shared): DEMO101 and DEMO205."
    public func structureOnly(_ courses: [String]) -> String? {
        guard !courses.isEmpty else { return nil }
        return l10n("plan.structureOnly", ["courses": courses.formatted(.list(type: .and).locale(l10n.locale))])
    }

    /// The draft's tasks by day, in date order (tasks keep their order within a day).
    public static func days(_ items: [StudyPlanItem]) -> [(date: String, items: [StudyPlanItem])] {
        var order: [String] = []
        var byDate: [String: [StudyPlanItem]] = [:]
        for item in items {
            if byDate[item.date] == nil { order.append(item.date) }
            byDate[item.date, default: []].append(item)
        }
        return order.sorted().map { ($0, byDate[$0] ?? []) }
    }

    private var style: Date.FormatStyle {
        Date.FormatStyle(locale: l10n.locale, calendar: calendar, timeZone: calendar.timeZone)
    }
}
