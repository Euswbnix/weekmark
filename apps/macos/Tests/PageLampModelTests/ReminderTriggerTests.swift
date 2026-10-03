// Calendar triggers across the 2026-11-01 DST change in Toronto (model-access design §5.3; the
// M3 DoD's DST test, Swift half): a reminder fires at its wall-clock time in its own zone, never
// at an absolute date, whatever the Mac's own zone. Instants are resolved with a Gregorian
// calendar, so the results don't depend on the day the tests run; one real
// `UNCalendarNotificationTrigger` checks a change still years ahead (2030-11-03).

import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Testing
import UserNotifications

private let toronto = "America/Toronto"

/// "2026-11-02T14:00:00Z" as a Date.
private func utc(_ text: String) -> Date {
    (try? Date(text, strategy: .iso8601)) ?? .distantPast
}

/// A reminder as the facade gives it: `localTime` is `fireAt` on the wall clock of `zone`.
private func reminder(
    _ kind: ReminderKind = .weeklyDigest, fireAt: Date, localTime: String? = nil, zone: String = toronto,
    dueAt: Date? = nil
) -> Reminder {
    Reminder(
        id: "weekly_digest:test", kind: kind, localTime: localTime ?? wallClock(fireAt, zone: zone), timeZone: zone,
        fireAt: fireAt, title: nil, courseId: nil, courseCode: nil, courseName: nil, dueAt: dueAt,
        hoursBefore: nil, count: 0
    )
}

/// The facade's `local_text`: "YYYY-MM-DDTHH:MM" in `zone`.
private func wallClock(_ date: Date, zone: String) -> String {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: zone) ?? .gmt
    let p = calendar.dateComponents([.year, .month, .day, .hour, .minute], from: date)
    return String(format: "%04d-%02d-%02dT%02d:%02d", p.year ?? 0, p.month ?? 0, p.day ?? 0, p.hour ?? 0, p.minute ?? 0)
}

@Suite("Reminder triggers across a DST change")
struct ReminderTriggerTests {
    @Test("Monday 09:00 stays 09:00 in Toronto before and after the clocks go back on 2026-11-01")
    func digestKeepsItsWallClock() {
        for (local, instant) in [("2026-10-26T09:00", "2026-10-26T13:00:00Z"), ("2026-11-02T09:00", "2026-11-02T14:00:00Z")] {
            let components = ReminderTrigger.components(for: reminder(fireAt: utc(instant), localTime: local))
            #expect(components.timeZone?.identifier == toronto)
            #expect(components.hour == 9)
            #expect(components.minute == 0)
            #expect(ReminderTrigger.instant(of: components) == utc(instant))
        }
    }

    @Test("the trigger carries its zone: a calendar in UTC or Shanghai resolves it to the same instant")
    func independentOfTheMacsZone() throws {
        let components = ReminderTrigger.components(
            for: reminder(fireAt: utc("2026-11-02T14:00:00Z"), localTime: "2026-11-02T09:00")
        )
        for zone in ["UTC", "Asia/Shanghai", toronto] {
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = try #require(TimeZone(identifier: zone))
            var elsewhere = components
            elsewhere.calendar = calendar
            #expect(calendar.date(from: elsewhere) == utc("2026-11-02T14:00:00Z"), "resolved in \(zone)")
        }
    }

    @Test("a deadline's 48 h and 24 h reminders across the change fire exactly at fire_at")
    func deadlineRemindersAcrossTheChange() {
        // Due Monday 2026-11-02 23:59 EST; 48 h before is 00:59 EDT on Nov 1, 24 h before 23:59 EST.
        let due = utc("2026-11-03T04:59:00Z")
        for hours in [48.0, 24.0] {
            let fire = due.addingTimeInterval(-hours * 3600)
            let components = ReminderTrigger.components(for: reminder(.deadlineSoon, fireAt: fire, dueAt: due))
            #expect(components.timeZone?.identifier == toronto)
            #expect(ReminderTrigger.instant(of: components) == fire)
        }
        #expect(wallClock(due.addingTimeInterval(-48 * 3600), zone: toronto) == "2026-11-01T00:59")
        #expect(wallClock(due.addingTimeInterval(-24 * 3600), zone: toronto) == "2026-11-01T23:59")
    }

    @Test("the repeated hour: 01:30 on Nov 1 is Toronto's first 01:30, else UTC components of fire_at")
    func repeatedHour() {
        // 01:30 EDT (05:30Z) is the first occurrence: the zone's own wall clock names it.
        let first = ReminderTrigger.components(for: reminder(fireAt: utc("2026-11-01T05:30:00Z"), localTime: "2026-11-01T01:30"))
        #expect(first.timeZone?.identifier == toronto)
        #expect(ReminderTrigger.instant(of: first) == utc("2026-11-01T05:30:00Z"))
        // 01:30 EST (06:30Z) is the second: the system would fire an hour early, so UTC.
        let second = ReminderTrigger.components(for: reminder(fireAt: utc("2026-11-01T06:30:00Z"), localTime: "2026-11-01T01:30"))
        #expect(second.timeZone == .gmt)
        #expect(second.hour == 6)
        #expect(ReminderTrigger.instant(of: second) == utc("2026-11-01T06:30:00Z"))
    }

    @Test("a zone this Mac doesn't know, or a malformed wall clock, falls back to UTC components of fire_at")
    func unknownZoneAndMalformedTime() {
        let fire = utc("2026-11-02T14:00:00Z")
        for bad in [reminder(fireAt: fire, localTime: "2026-11-02T09:00", zone: "Mars/Olympus_Mons"),
                    reminder(fireAt: fire, localTime: "Monday 9am"),
                    reminder(fireAt: fire, localTime: "2026-13-02T09:00")] {
            let components = ReminderTrigger.components(for: bad)
            #expect(components.timeZone == .gmt)
            #expect(ReminderTrigger.instant(of: components) == fire)
        }
    }

    @Test("a deadline due at 23:59:59 keeps its seconds")
    func seconds() {
        let fire = utc("2026-11-03T04:59:59Z")
        let components = ReminderTrigger.components(for: reminder(.deadlineSoon, fireAt: fire))
        #expect(components.timeZone?.identifier == toronto)
        #expect(components.second == 59)
        #expect(ReminderTrigger.instant(of: components) == fire)
    }

    @Test("the system's own calendar trigger fires at 09:00 local on both sides of the 2030-11-03 change")
    func systemTrigger() {
        for (local, instant) in [("2030-10-28T09:00", "2030-10-28T13:00:00Z"), ("2030-11-04T09:00", "2030-11-04T14:00:00Z")] {
            let components = ReminderTrigger.components(for: reminder(fireAt: utc(instant), localTime: local))
            let trigger = UNCalendarNotificationTrigger(dateMatching: components, repeats: false)
            #expect(trigger.nextTriggerDate() == utc(instant), "\(local)")
        }
    }

    @Test("end to end on the mock: from 2026-10-28, Monday's digest fires at 09:00 Toronto = 14:00Z")
    func mockScheduleAcrossTheChange() async throws {
        let now = utc("2026-10-28T14:00:00Z")
        let mock = MockService(scenario: .demo, timing: .instant, calendar: TestClock.calendar, now: { now })
        let reminders = try await mock.reminders(from: now, to: now.addingTimeInterval(7 * 86_400))
        let digest = try #require(reminders.first { $0.kind == .weeklyDigest })
        #expect(digest.id == "weekly_digest:2026-11-02")
        #expect(digest.localTime == "2026-11-02T09:00")
        let components = ReminderTrigger.components(for: digest)
        #expect(components.timeZone?.identifier == toronto)
        #expect(ReminderTrigger.instant(of: components) == utc("2026-11-02T14:00:00Z"))
        // Every reminder of the week, deadlines included, fires exactly at its fire_at.
        #expect(reminders.contains { $0.kind == .deadlineSoon })
        for reminder in reminders {
            #expect(ReminderTrigger.instant(of: ReminderTrigger.components(for: reminder)) == reminder.fireAt, "\(reminder.id)")
        }
    }
}
