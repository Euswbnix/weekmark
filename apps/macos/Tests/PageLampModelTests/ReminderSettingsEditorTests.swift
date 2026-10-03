// Settings ▸ Reminders' editor: read, change, write back as a whole (never over a newer answer);
// the time picker's "09:00".

import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Testing

private func mock() -> MockService {
    MockService(scenario: .demo, timing: .instant, calendar: TestClock.calendar, now: { TestClock.now })
}

@Suite("Reminder settings editor") @MainActor
struct ReminderSettingsEditorTests {
    @Test("a change is read, applied and written back as a whole, never over a newer answer")
    func update() async throws {
        let service = mock()
        let editor = ReminderSettingsEditor()
        await editor.load(service)
        #expect(editor.settings?.digestTime == "09:00")
        // Meanwhile "Remind me" was answered elsewhere (the question on This Week).
        try await service.setReminderSettings(settings: try await service.reminderSettings().with { $0.runInBackground = true })
        await editor.update({ $0.digestTime = "18:30" }, service: service, delivery: nil)
        let saved = try await service.reminderSettings()
        #expect(saved.digestTime == "18:30")
        #expect(saved.runInBackground)
        #expect(editor.settings == saved)
        #expect(!editor.saveFailed)
    }

    @Test("a time the facade refuses isn't kept: the switch shows what is saved")
    func refused() async throws {
        let service = mock()
        let editor = ReminderSettingsEditor()
        await editor.load(service)
        await editor.update({ $0.digestTime = "25:00" }, service: service, delivery: nil)
        #expect(editor.saveFailed)
        #expect(editor.settings?.digestTime == "09:00")
    }

    @Test("the time picker's date and \"HH:MM\" round-trip; day numbers follow the calendar")
    func times() throws {
        let date = try #require(ReminderSettingsEditor.date(fromClock: "07:05", calendar: TestClock.calendar))
        #expect(ReminderSettingsEditor.clock(from: date, calendar: TestClock.calendar) == "07:05")
        #expect(ReminderSettingsEditor.date(fromClock: "7am", calendar: TestClock.calendar) == nil)
        #expect(ReminderSettingsEditor.weekdayNumber(.sunday) == 1)
        #expect(ReminderSettingsEditor.weekdayNumber(.monday) == 2)
    }
}
