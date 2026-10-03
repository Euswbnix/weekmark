// Settings ▸ Reminders (macos-shell §3.5; model-access design §5.3): the facade's reminder
// settings, each change saved at once. "Remind me with notifications" goes through
// `ReminderDelivery.setConsent` (the system's prompt, "Reminders are on"); the kinds, the digest's
// day and time and today's plan are read, changed and written back as a whole, so a change never
// puts back an older answer.

import Foundation
import Observation
import PageLampKit

@Observable @MainActor
public final class ReminderSettingsEditor {
    public private(set) var settings: ReminderSettings?
    public private(set) var loadFailed = false
    /// The last change couldn't be saved (the switch shows what is saved).
    public private(set) var saveFailed = false

    public init() {}

    public func load(_ service: any PageLampService) async {
        do throws(PageLampFailure) {
            settings = try await service.reminderSettings()
            loadFailed = false
        } catch {
            loadFailed = true
        }
    }

    /// Changes one field and saves; afterwards the week ahead is scheduled again.
    public func update(
        _ change: (inout ReminderSettingsDraft) -> Void,
        service: any PageLampService,
        delivery: ReminderDelivery?
    ) async {
        do throws(PageLampFailure) {
            let current = try await service.reminderSettings().with(change)
            try await service.setReminderSettings(settings: current)
            settings = current
            saveFailed = false
            delivery?.requestPass()
        } catch {
            saveFailed = true
            await load(service)
        }
    }

    /// "Remind me with notifications" on or off.
    public func setConsent(_ on: Bool, service: any PageLampService, delivery: ReminderDelivery) async {
        do throws(PageLampFailure) {
            try await delivery.setConsent(on)
            saveFailed = false
        } catch {
            saveFailed = true
        }
        await load(service)
    }

    // MARK: - Times ("09:00" ↔ a time picker's date)

    /// "09:00" as a date on `calendar`'s reference day (for a time picker); nil if malformed.
    public static func date(fromClock text: String, calendar: Calendar) -> Date? {
        let parts = text.split(separator: ":")
        guard parts.count == 2, let hour = Int(parts[0]), let minute = Int(parts[1]),
              (0...23).contains(hour), (0...59).contains(minute) else { return nil }
        return calendar.date(from: DateComponents(year: 2001, month: 1, day: 1, hour: hour, minute: minute))
    }

    /// A time picker's date as "HH:MM" on `calendar`.
    public static func clock(from date: Date, calendar: Calendar) -> String {
        let parts = calendar.dateComponents([.hour, .minute], from: date)
        return String(format: "%02d:%02d", parts.hour ?? 0, parts.minute ?? 0)
    }

    /// The facade's day for the calendar's weekday number (1 = Sunday), and back.
    public static let weekdays: [DayOfWeek] = [.sunday, .monday, .tuesday, .wednesday, .thursday, .friday, .saturday]

    public static func weekdayNumber(_ day: DayOfWeek) -> Int {
        (weekdays.firstIndex(of: day) ?? 1) + 1
    }
}
