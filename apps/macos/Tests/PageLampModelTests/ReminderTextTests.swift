// A reminder's notification text (model-access design §5.3): codes and titles only, times in the
// reminder's own zone, the Tauri app's keys; and the mock's reminders, which follow the facade's
// schedule, due rules and digest.

import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Testing

private let en = L10n(locale: Locale(identifier: "en_US"), table: .app)
private let zh = L10n(locale: Locale(identifier: "zh-Hans_CN"), table: .app)

private func plain(_ value: String) -> String {
    value.replacingOccurrences(of: "\u{202F}", with: " ").replacingOccurrences(of: "\u{00A0}", with: " ")
}

private func deadlineReminder(code: String?, name: String? = nil, zone: String = "America/Toronto") -> Reminder {
    let due = TestClock.at(4, 23, 59) // Tuesday 2026-09-29 23:59 in Toronto
    return Reminder(
        id: "deadline_soon:24h:e1@1", kind: .deadlineSoon, localTime: "2026-09-28T23:59", timeZone: zone,
        fireAt: due.addingTimeInterval(-86_400), title: "Problem Set 2", courseId: "c1", courseCode: code,
        courseName: name, dueAt: due, hoursBefore: 24, count: nil
    )
}

private func countReminder(_ kind: ReminderKind, count: UInt32) -> Reminder {
    Reminder(
        id: kind == .weeklyDigest ? "weekly_digest:2026-09-28" : "plan_today:2026-09-28", kind: kind,
        localTime: "2026-09-28T09:00", timeZone: "America/Toronto", fireAt: TestClock.at(3, 9), title: nil,
        courseId: nil, courseCode: nil, courseName: nil, dueAt: nil, hoursBefore: nil, count: count
    )
}

@Suite("Reminder text")
struct ReminderTextTests {
    @Test("a deadline: the course code and title, and when it's due in the reminder's zone")
    func deadline() {
        let english = ReminderText.notification(for: deadlineReminder(code: "DEMO205"), l10n: en)
        #expect(english.id == "deadline_soon:24h:e1@1")
        #expect(english.title == "DEMO205: Problem Set 2")
        #expect(plain(english.body) == "Due Tuesday 11:59 PM")
        let chinese = ReminderText.notification(for: deadlineReminder(code: "DEMO205"), l10n: zh)
        #expect(chinese.title.contains("DEMO205"))
        #expect(plain(chinese.body).contains("星期二"))
        #expect(plain(chinese.body).contains("23:59"))
        // Shown in its own zone: in Shanghai that deadline is Wednesday morning.
        let shanghai = ReminderText.notification(for: deadlineReminder(code: "DEMO205", zone: "Asia/Shanghai"), l10n: en)
        #expect(plain(shanghai.body) == "Due Wednesday 11:59 AM")
    }

    @Test("without a code the course name stands in; without either, the title alone")
    func deadlineWithoutCode() {
        #expect(ReminderText.notification(for: deadlineReminder(code: nil, name: "Statistics"), l10n: en).title
            == "Statistics: Problem Set 2")
        #expect(ReminderText.notification(for: deadlineReminder(code: nil), l10n: en).title == "Problem Set 2")
    }

    @Test("the digest and today's plan count what's ahead")
    func counts() {
        #expect(ReminderText.notification(for: countReminder(.weeklyDigest, count: 3), l10n: en).body
            == "3 deadlines in the next 7 days")
        #expect(ReminderText.notification(for: countReminder(.weeklyDigest, count: 1), l10n: en).body
            == "1 deadline in the next 7 days")
        #expect(ReminderText.notification(for: countReminder(.weeklyDigest, count: 0), l10n: en).body
            == "No deadlines in the next 7 days")
        #expect(ReminderText.notification(for: countReminder(.planToday, count: 2), l10n: en).title == "Today's study plan")
        #expect(ReminderText.notification(for: countReminder(.planToday, count: 2), l10n: en).body == "2 things to do today")
        #expect(ReminderText.remindersOn(l10n: en).title == "Reminders are on")
    }
}

@Suite("Mock reminders (the facade's rules on the mock's data)")
struct MockReminderTests {
    private func mock(at now: Date = TestClock.now) -> MockService {
        MockService(scenario: .demo, timing: .instant, calendar: TestClock.calendar, now: { now })
    }

    @Test("48 h and 24 h before each deadline, the facade's ids and wall-clock times, soonest first")
    func schedule() async throws {
        let service = mock()
        let reminders = try await service.reminders(from: TestClock.now, to: TestClock.now.addingTimeInterval(7 * 86_400))
        let deadlines = reminders.filter { $0.kind == .deadlineSoon }
        #expect(!deadlines.isEmpty)
        for reminder in deadlines {
            #expect(reminder.id.hasPrefix("deadline_soon:\(reminder.hoursBefore ?? 0)h:"))
            #expect([48, 24].contains(reminder.hoursBefore))
            #expect(reminder.timeZone == "America/Toronto")
            #expect(reminder.localTime.count == 16) // "YYYY-MM-DDTHH:MM"
        }
        #expect(reminders.map(\.fireAt) == reminders.map(\.fireAt).sorted())
        // Monday's digest at 09:00, counting the week's deadlines.
        let digest = try #require(reminders.first { $0.kind == .weeklyDigest })
        #expect(digest.id == "weekly_digest:2026-09-28")
        #expect(digest.localTime == "2026-09-28T09:00")
        // Shown reminders are no longer scheduled.
        try await service.markRemindersShown(ids: [digest.id])
        let after = try await service.reminders(from: TestClock.now, to: TestClock.now.addingTimeInterval(7 * 86_400))
        #expect(!after.contains { $0.id == digest.id })
    }

    @Test("today's plan only when it's on and the day has open items")
    func planToday() async throws {
        let service = mock()
        try await service.setReminderSettings(settings: try await service.reminderSettings().with { $0.planToday = true })
        let reminders = try await service.reminders(from: TestClock.now, to: TestClock.now.addingTimeInterval(3 * 86_400))
        let plans = reminders.filter { $0.kind == .planToday }
        #expect(plans.map(\.id) == ["plan_today:2026-09-26", "plan_today:2026-09-27"])
        #expect(plans.first?.count == 1)
    }

    @Test("due: the latest reminder of a deadline only, a digest for 3 days, today's plan until midnight")
    func due() async throws {
        let service = mock()
        try await service.setReminderSettings(settings: try await service.reminderSettings().with { $0.planToday = true })
        let week = try await service.reminders(from: TestClock.now, to: TestClock.now.addingTimeInterval(7 * 86_400))
        let deadline = try #require(week.first { $0.kind == .deadlineSoon && $0.hoursBefore == 24 })
        // Just after its 24 h reminder: that one is due, its 48 h one isn't any more. (The same
        // mock throughout: its demo data is laid out from its own clock.)
        let due = try await service.dueReminders(now: deadline.fireAt.addingTimeInterval(60))
        #expect(due.contains { $0.id == deadline.id })
        #expect(!due.contains { $0.id == deadline.id.replacingOccurrences(of: "24h:", with: "48h:") })
        // Monday's digest is due until Thursday 09:00, not after.
        let digest = try #require(week.first { $0.kind == .weeklyDigest })
        let thursday = digest.fireAt.addingTimeInterval(3 * 86_400 - 60)
        #expect(try await service.dueReminders(now: thursday).contains { $0.id == digest.id })
        let tooLate = digest.fireAt.addingTimeInterval(3 * 86_400 + 60)
        #expect(!(try await service.dueReminders(now: tooLate).contains { $0.id == digest.id }))
    }

    @Test("marking shown rejects an id that isn't a reminder's, and records nothing then")
    func markShown() async throws {
        let service = mock()
        await #expect(throws: PageLampFailure.self) {
            try await service.markRemindersShown(ids: ["weekly_digest:2026-09-28", "not-a-reminder"])
        }
        let week = try await service.reminders(from: TestClock.now, to: TestClock.now.addingTimeInterval(7 * 86_400))
        #expect(week.contains { $0.id == "weekly_digest:2026-09-28" })
    }

    @Test("the digest: active courses with their week and materials, every visible course's deadlines, the plan")
    func digest() async throws {
        let digest = try await mock().weeklyDigest()
        #expect(!digest.courses.isEmpty)
        for course in digest.courses {
            if course.active {
                #expect(course.week != nil)
                #expect(course.materialTitles.count <= 5)
                #expect(Int(course.materialCount) >= course.materialTitles.count)
            } else {
                #expect(course.week == nil)
                #expect(!course.deadlines.isEmpty)
            }
            for deadline in course.deadlines {
                let due = try #require(deadline.event.dueAt ?? deadline.event.startsAt)
                #expect(due >= TestClock.now && due < TestClock.now.addingTimeInterval(7 * 86_400))
            }
        }
        let plan = try #require(digest.plan)
        #expect(plan.lastWeekPlanned == 1)
        #expect(plan.lastWeekDone == 1)
        #expect(plan.today.count == 2)
    }
}
