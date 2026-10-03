// The menu bar extra's week (spec §3.7 W10, §6.5) from the facade's digest.

import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Testing

private func mock(_ scenario: MockScenario = .demo) -> MockService {
    MockService(scenario: scenario, timing: .instant, calendar: TestClock.calendar, now: { TestClock.now })
}

@Suite("Menu bar week") @MainActor
struct MenuBarWeekTests {
    @Test("at most 5 deadlines, soonest first, the rest counted; the lamp lights for one within 24 h")
    func deadlines() async throws {
        let digest = try await mock().weeklyDigest()
        let week = MenuBarWeek(digest: digest, now: TestClock.now)
        let all = digest.courses.flatMap(\.deadlines).count
        #expect(week.dueSoon.count == min(all, MenuBarWeek.maxDeadlines))
        #expect(week.moreDue == max(0, all - MenuBarWeek.maxDeadlines))
        #expect(week.dueSoon.map(\.due) == week.dueSoon.map(\.due).sorted())
        let within = digest.courses.flatMap(\.deadlines).compactMap { $0.event.dueAt ?? $0.event.startsAt }
            .filter { $0 >= TestClock.now && $0.timeIntervalSince(TestClock.now) <= 86_400 }.count
        #expect(week.dueWithin24h == within)
    }

    @Test("today's plan (at most 4), last week's progress, the active courses with their week")
    func planAndCourses() async throws {
        let digest = try await mock().weeklyDigest()
        let week = MenuBarWeek(digest: digest, now: TestClock.now)
        #expect(week.today.count == min(digest.plan?.today.count ?? 0, MenuBarWeek.maxPlanItems))
        #expect(week.lastWeek?.done == 1)
        #expect(week.lastWeek?.planned == 1)
        #expect(!week.courses.isEmpty)
        #expect(week.courses.allSatisfy { $0.active && $0.week != nil })
    }

    @Test("no courses and no plan: empty")
    func empty() async throws {
        let week = MenuBarWeek(digest: try await mock(.empty).weeklyDigest(), now: TestClock.now)
        #expect(week.isEmpty)
    }

    @Test("the model reads the digest with each refresh where reminders are on, and only there")
    func modelLoadsTheDigest() async {
        let on = AppModel(
            dataMode: .mock(.demo), strings: .app, settings: InMemorySettingsStore(language: .english),
            calendar: TestClock.calendar, clock: { TestClock.now }, notificationCenter: NotificationCenter(),
            service: mock(), reminders: true,
            reminderCenters: ReminderCenters(live: { RecordingNotificationCenter() }, mock: RecordingNotificationCenter())
        )
        await on.refresh()
        #expect(on.menuBarWeek != nil)
        let off = AppModel(
            dataMode: .mock(.demo), strings: .app, settings: InMemorySettingsStore(language: .english),
            calendar: TestClock.calendar, clock: { TestClock.now }, notificationCenter: NotificationCenter(),
            service: mock()
        )
        await off.refresh()
        #expect(off.menuBarWeek == nil)
        #expect(off.reminderDelivery == nil)
    }
}
