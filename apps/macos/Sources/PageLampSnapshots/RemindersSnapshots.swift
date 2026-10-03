// Reminders and the menu bar extra (M3, preview builds) for the snapshot catalogue: Settings ▸
// Reminders off, on and not allowed; This Week's catch-up card (reminders off at launch, what came
// due since); the menu bar extra with the demo's week and empty. "Remind me" on the first-sync
// page is in `ThisWeekSnapshots` ("this-week-first-sync").

import SwiftUI
import PageLamp
import PageLampKit
import PageLampModel

enum RemindersSnapshots {
    static let pages: [SnapshotPage] = [
        settings("settings-reminders-off", setup: SnapshotSetup(), prepare: leaveOff),
        settings("settings-reminders-on", setup: SnapshotSetup(notificationPermission: .notDetermined, notificationAnswer: true), prepare: turnOnWithPlan),
        settings("settings-reminders-denied", setup: SnapshotSetup(notificationPermission: .notDetermined, notificationAnswer: false), prepare: turnOn),
        SnapshotPage(name: "this-week-reminders-catchup", width: SnapshotCatalog.detailWidth, minHeight: WindowMetrics.mainHeight, setup: SnapshotSetup(moment: tuesday), make: catchUp),
        SnapshotPage(name: "menubar-week", width: MenuBarWeekView.width, minHeight: 0, setup: SnapshotSetup(), make: menuBar),
        SnapshotPage(name: "menubar-empty", width: MenuBarWeekView.width, minHeight: 0, setup: SnapshotSetup(scenario: .empty), make: menuBar),
    ]

    // MARK: - Preparations

    private static func leaveOff(_ model: AppModel) async {}

    /// Yes, remind me (the system's prompt answers as the page's setup says).
    private static func turnOn(_ model: AppModel) async {
        try? await model.reminderDelivery?.setConsent(true)
    }

    /// Yes, remind me, and today's plan on too (its time shows).
    private static func turnOnWithPlan(_ model: AppModel) async {
        await turnOn(model)
        if let settings = try? await model.service.reminderSettings() {
            try? await model.service.setReminderSettings(settings: settings.with { $0.planToday = true })
        }
    }

    /// The Tuesday after the harness's Friday: Monday's digest came due and is still shown.
    private nonisolated static func tuesday(_ now: Date, _ calendar: Calendar) -> Date {
        calendar.date(byAdding: .day, value: 4, to: now) ?? now
    }

    /// Reminders off at launch: what came due since the last launch shows on This Week.
    private static func catchUp(_ model: AppModel) async -> AnyView {
        let due = (try? await model.service.dueReminders(now: model.clock())) ?? []
        await model.reminderDelivery?.launch(due: due)
        return AnyView(ThisWeekPage())
    }

    private static func menuBar(_ model: AppModel) async -> AnyView {
        await model.loadMenuBarWeek()
        return AnyView(MenuBarWeekView())
    }

    /// Settings ▸ Reminders after `prepare`, with its settings loaded (`.task` never runs offscreen).
    private static func settings(
        _ name: String,
        setup: SnapshotSetup,
        prepare: @escaping @MainActor (AppModel) async -> Void
    ) -> SnapshotPage {
        SnapshotPage(name: name, width: WindowMetrics.settingsWidth, minHeight: 520, setup: setup) { model in
            await prepare(model)
            await model.reminderDelivery?.settle()
            let editor = ReminderSettingsEditor()
            await editor.load(model.service)
            return AnyView(SettingsTabPage(
                tab: .reminders, title: model.l10n("mac.settings.tabs.reminders"), reminders: editor
            ))
        }
    }
}
