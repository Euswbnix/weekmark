// The weekly note (M3, preview builds) for the snapshot catalogue, on This Week: none yet with
// "≈ $x"; two notes written (the newest shows, the other in Earlier notes); Monday's note prepared;
// Monday's run blocked by the budget (its one line); a run in progress; the narrowest window; and
// Settings ▸ AI with the opt-in paused. The pages are named weekly-note-* (the this-week-* pages
// are pinned by ThisWeekRenderTests).

import SwiftUI
import PageLamp
import PageLampKit
import PageLampModel

enum WeeklyNoteSnapshots {
    static let pages: [SnapshotPage] = [
        page("weekly-note-empty", scenario: .aiKey, prepare: nothing),
        page("weekly-note-written", scenario: .aiKey, prepare: writeTwice),
        page("weekly-note-monday", scenario: .weeklyNoteMonday, prepare: prepareMonday),
        page("weekly-note-monday-blocked", scenario: .aiKey, moment: monday, prepare: blockMonday),
        page("weekly-note-running", scenario: .aiKey, syncStep: .seconds(30), prepare: startAndHold),
        page("weekly-note-narrow", scenario: .aiKey, width: ThisWeekSnapshots.narrowWidth, prepare: writeTwice),
        SnapshotPage(
            name: "settings-ai-weekly-note-paused", width: WindowMetrics.settingsWidth, minHeight: 640,
            setup: SnapshotSetup(scenario: .aiKey), make: pausedSettings
        ),
    ]

    // MARK: - Preparations

    private static func nothing(_ note: WeeklyNoteModel, _ model: AppModel) async {}

    /// Two notes: the newest shows, the first is listed under Earlier notes.
    private static func writeTwice(_ note: WeeklyNoteModel, _ model: AppModel) async {
        for _ in 0..<2 {
            await note.write(uiLanguage: model.localization)
        }
    }

    /// The scenario's Monday: the read says it's due, and the note is prepared.
    private static func prepareMonday(_ note: WeeklyNoteModel, _ model: AppModel) async {
        await note.check()
        await wait { if case .finished = note.run.phase { true } else { false } }
    }

    /// Monday with the opt-in on and a budget a note would go over: Monday's run is blocked and
    /// leaves its line (Write offers going over the budget).
    private static func blockMonday(_ note: WeeklyNoteModel, _ model: AppModel) async {
        _ = try? await model.service.setPrepareWeeklyNoteOnMonday(on: true)
        try? await model.service.setMonthlyBudget(microUsd: 1)
        await note.estimate.refresh()
        await note.check()
        await wait { note.automaticProblem != nil }
    }

    /// A click's run at its first stage (the mock waits there long after the render).
    private static func startAndHold(_ note: WeeklyNoteModel, _ model: AppModel) async {
        Task { await note.write(uiLanguage: model.localization) }
        await wait { if case .running(let progress) = note.run.phase { progress.stage != nil } else { false } }
    }

    /// Up to 5 s for a state the mock reaches on its own.
    private static func wait(_ condition: @MainActor () -> Bool) async {
        let clock = ContinuousClock()
        let deadline = clock.now + .seconds(5)
        while clock.now < deadline, !condition() {
            try? await Task.sleep(for: .milliseconds(5))
        }
    }

    /// Monday, 3 days after the harness's Friday.
    private static let monday: @Sendable (Date, Calendar) -> Date = { now, calendar in
        calendar.date(byAdding: .day, value: 3, to: now) ?? now
    }

    /// The opt-in on, then the note's model taken away: shown paused (it can still be turned off).
    private static func pausedSettings(_ model: AppModel) async -> AnyView {
        _ = try? await model.service.setPrepareWeeklyNoteOnMonday(on: true)
        try? await model.service.setFeatureModel(feature: .weeklyNote, choice: nil)
        let ai = AiSettingsModel(service: model.service, clock: model.clock, calendar: model.calendar)
        await ai.load()
        return AnyView(SettingsTabPage(tab: .ai, title: model.l10n("mac.settings.tabs.ai"), ai: ai))
    }

    // MARK: - Pages

    private static func page(
        _ name: String,
        scenario: MockScenario,
        moment: @escaping @Sendable (Date, Calendar) -> Date = { now, _ in now },
        syncStep: Duration = .zero,
        width: CGFloat = SnapshotCatalog.detailWidth,
        prepare: @escaping @MainActor (WeeklyNoteModel, AppModel) async -> Void
    ) -> SnapshotPage {
        SnapshotPage(
            name: name, width: width, minHeight: WindowMetrics.mainHeight,
            setup: SnapshotSetup(scenario: scenario, moment: moment, syncStep: syncStep)
        ) { model in
            guard let note = model.weeklyNote else { return AnyView(EmptyView()) }
            await prepare(note, model)
            return AnyView(ThisWeekPage())
        }
    }
}
