// When a reminder's notification fires (model-access design §5.3, Delivery): a calendar trigger
// built from the reminder's wall-clock time in its IANA time zone, never an absolute date, so a
// "Monday 09:00" digest stays at 09:00 local across a DST change (Canada leaves DST on
// 2026-11-01) and doesn't depend on the Mac's own time zone.

import Foundation
import PageLampKit

public enum ReminderTrigger {
    /// The date components of `reminder`'s calendar trigger (`UNCalendarNotificationTrigger`,
    /// not repeating): its `local_time` ("YYYY-MM-DDTHH:MM") in its `time_zone`, with the seconds
    /// of `fire_at` (a deadline due at 23:59:59 reminds at :59 too).
    ///
    /// Two wall-clock times don't name `fire_at` exactly, and then the components are `fire_at`'s
    /// in UTC (still a calendar trigger, still exact):
    /// - a time in the hour the clock repeats when it goes back, when `fire_at` is its second
    ///   occurrence (a deadline's reminder 48 h before, say): the system would fire at the first;
    /// - a time zone this Mac doesn't know.
    public static func components(for reminder: Reminder) -> DateComponents {
        if let zone = TimeZone(identifier: reminder.timeZone),
           let local = wallClock(reminder.localTime) {
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = zone
            var components = local
            components.calendar = calendar
            components.timeZone = zone
            components.second = calendar.component(.second, from: reminder.fireAt)
            if let resolved = calendar.date(from: components), sameSecond(resolved, reminder.fireAt) {
                return components
            }
        }
        return utcComponents(of: reminder.fireAt)
    }

    /// The instant `components` names (what the system's calendar trigger fires at).
    public static func instant(of components: DateComponents) -> Date? {
        var calendar = components.calendar ?? Calendar(identifier: .gregorian)
        if let zone = components.timeZone { calendar.timeZone = zone }
        return calendar.date(from: components)
    }

    /// "2026-11-02T09:00" → year, month, day, hour, minute; nil for anything else.
    static func wallClock(_ text: String) -> DateComponents? {
        let parts = text.split(separator: "T", omittingEmptySubsequences: false)
        guard parts.count == 2 else { return nil }
        let date = parts[0].split(separator: "-", omittingEmptySubsequences: false).map { Int($0) }
        let time = parts[1].split(separator: ":", omittingEmptySubsequences: false).map { Int($0) }
        guard date.count == 3, time.count == 2,
              let year = date[0], let month = date[1], let day = date[2],
              let hour = time[0], let minute = time[1],
              (1...12).contains(month), (1...31).contains(day), (0...23).contains(hour), (0...59).contains(minute)
        else { return nil }
        return DateComponents(year: year, month: month, day: day, hour: hour, minute: minute)
    }

    private static func utcComponents(of date: Date) -> DateComponents {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .gmt
        var components = calendar.dateComponents([.year, .month, .day, .hour, .minute, .second], from: date)
        components.calendar = calendar
        components.timeZone = .gmt
        return components
    }

    private static func sameSecond(_ a: Date, _ b: Date) -> Bool {
        a.timeIntervalSince1970.rounded(.down) == b.timeIntervalSince1970.rounded(.down)
    }
}
