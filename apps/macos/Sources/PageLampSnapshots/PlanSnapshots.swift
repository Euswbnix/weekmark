// Plan your study (M3, preview builds) for the snapshot catalogue: the form with its "≈ $x";
// with no model chosen; going over this month's budget; the run in progress; the draft (a short
// plan, so some tasks aren't scheduled); and This Week with the plan PageLamp wrote. The sheet
// renders on its own at its width.

import SwiftUI
import PageLamp
import PageLampKit
import PageLampModel

enum PlanSnapshots {
    static let pages: [SnapshotPage] = [
        sheet("plan-form", scenario: .aiKey, prepare: nothing),
        sheet("plan-form-no-model", scenario: .demo, prepare: nothing),
        sheet("plan-form-over-budget", scenario: .aiBudget, prepare: nearlySpent),
        sheet("plan-running", scenario: .aiKey, syncStep: .seconds(30), prepare: startAndHold),
        sheet("plan-draft", scenario: .aiKey, prepare: writeShortPlan),
        SnapshotPage(
            name: "plan-this-week", width: SnapshotCatalog.detailWidth, minHeight: WindowMetrics.mainHeight,
            setup: SnapshotSetup(scenario: .aiKey), make: thisWeekWithPlan
        ),
    ]

    // MARK: - Preparations

    private static func nothing(_ plan: PlanModel, _ model: AppModel) async {}

    /// A budget this plan would go over ($4.96 of it is spent).
    private static func nearlySpent(_ plan: PlanModel, _ model: AppModel) async {
        try? await model.service.setMonthlyBudget(microUsd: 4_962_000)
        await plan.estimate.refresh()
    }

    /// The run at its first stage (the mock waits there long after the render).
    private static func startAndHold(_ plan: PlanModel, _ model: AppModel) async {
        Task { await plan.generate() }
        let clock = ContinuousClock()
        let deadline = clock.now + .seconds(5)
        while clock.now < deadline {
            if case .running(let progress) = plan.run.phase, progress.stage != nil { return }
            try? await Task.sleep(for: .milliseconds(5))
        }
    }

    /// Four days only: the table, and the tasks that don't fit listed as not in the plan.
    private static func writeShortPlan(_ plan: PlanModel, _ model: AppModel) async {
        plan.horizon = "4"
        plan.hours = "10"
        await plan.estimate.settle()
        await plan.generate()
        await plan.againEstimate.settle()
    }

    // MARK: - Pages

    private static func sheet(
        _ name: String,
        scenario: MockScenario,
        syncStep: Duration = .zero,
        prepare: @escaping @MainActor (PlanModel, AppModel) async -> Void
    ) -> SnapshotPage {
        SnapshotPage(
            name: name, width: PlanSheet.width, minHeight: 0,
            setup: SnapshotSetup(scenario: scenario, syncStep: syncStep)
        ) { model in
            let plan = PlanModel(service: model.service, courses: model.courses, debounce: .zero)
            await plan.estimate.settle()
            await prepare(plan, model)
            return AnyView(PlanSheet(plan: plan, done: ignore))
        }
    }

    /// This Week after a plan PageLamp wrote was used: "Made by PageLamp", its AI-generated line.
    private static func thisWeekWithPlan(_ model: AppModel) async -> AnyView {
        let plan = PlanModel(service: model.service, courses: model.courses, debounce: .zero)
        await plan.estimate.settle()
        await plan.generate()
        if let stored = await plan.accept() {
            await model.studyPlanSaved(stored)
        }
        return AnyView(ThisWeekPage())
    }

    private static func ignore(_ stored: StoredStudyPlan?) {}
}
