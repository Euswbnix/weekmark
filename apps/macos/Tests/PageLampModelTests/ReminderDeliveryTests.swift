// Reminders handed to the notification center (model-access design §5.3, Delivery): nothing
// before "Remind me", one prompt and one "Reminders are on", the week ahead as calendar triggers
// (at most 64), what fired marked shown once, what was never handed over shown now, and the
// record of what the system holds kept honest. All on a notification center in memory.

import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Synchronization
import Testing

/// A clock the test moves.
private final class TestTime: Sendable {
    private let value: Mutex<Date>
    init(_ date: Date = TestClock.now) { value = Mutex(date) }
    var now: Date { value.withLock { $0 } }
    func set(_ date: Date) { value.withLock { $0 = date } }
}

/// The ids marked shown, in order.
private final class Marked: Sendable {
    private let ids = Mutex<[String]>([])
    var all: [String] { ids.withLock { $0 } }
    func add(_ more: [String]) { ids.withLock { $0 += more } }
}

/// The mock with extra reminders in the week and marking that fails for chosen ids.
private struct ReminderDouble: ForwardingService {
    let base: any PageLampService
    var extra: [Reminder] = []
    /// Ids whose marking fails with this kind.
    var failing: [String: PageLampFailure.Kind] = [:]
    let marked = Marked()

    func reminders(from: Date, to: Date) async throws(PageLampFailure) -> [Reminder] {
        let base = try await self.base.reminders(from: from, to: to)
        return (base + extra.filter { $0.fireAt >= from && $0.fireAt < to }).sorted { $0.fireAt < $1.fireAt }
    }

    func markRemindersShown(ids: [String]) async throws(PageLampFailure) {
        for id in ids {
            if let kind = failing[id] { throw PageLampFailure(kind: kind, message: "test") }
        }
        marked.add(ids)
        try await base.markRemindersShown(ids: ids)
    }
}

private func extraReminder(_ id: String, at date: Date) -> Reminder {
    var calendar = TestClock.calendar
    calendar.timeZone = TestClock.calendar.timeZone
    let p = calendar.dateComponents([.year, .month, .day, .hour, .minute], from: date)
    return Reminder(
        id: id, kind: .deadlineSoon,
        localTime: String(format: "%04d-%02d-%02dT%02d:%02d", p.year ?? 0, p.month ?? 0, p.day ?? 0, p.hour ?? 0, p.minute ?? 0),
        timeZone: "America/Toronto", fireAt: date, title: "Extra", courseId: nil, courseCode: "DEMO101",
        courseName: nil, dueAt: date.addingTimeInterval(86_400), hoursBefore: 24, count: nil
    )
}

@MainActor
private func delivery(
    _ service: any PageLampService, center: RecordingNotificationCenter,
    record: InMemorySettingsStore = InMemorySettingsStore(), time: TestTime = TestTime()
) -> ReminderDelivery {
    ReminderDelivery(
        service: service, center: center, record: record, clock: { time.now },
        l10n: { L10n(locale: Locale(identifier: "en_US"), table: .app) }
    )
}

private func mock(_ time: TestTime = TestTime()) -> MockService {
    MockService(scenario: .demo, timing: .instant, calendar: TestClock.calendar, now: { time.now })
}

@Suite("Reminder delivery") @MainActor
struct ReminderDeliveryTests {
    @Test("nothing reaches the system, and nothing is asked, until the student says Remind me")
    func nothingBeforeConsent() async {
        let center = RecordingNotificationCenter()
        let subject = delivery(mock(), center: center)
        subject.requestPass()
        await subject.settle()
        #expect(await center.pending.isEmpty)
        #expect(await center.shownNow.isEmpty)
        #expect(await center.permissionRequests == 0)
        #expect(subject.consent == false)
    }

    @Test("Remind me: one prompt, one Reminders are on, the week ahead as calendar triggers")
    func consent() async throws {
        let service = mock()
        let center = RecordingNotificationCenter()
        let record = InMemorySettingsStore()
        let subject = delivery(service, center: center, record: record)
        try await subject.setConsent(true)
        await subject.settle()
        #expect(await center.permissionRequests == 1)
        #expect(await center.shownNow.map(\.id) == [ReminderDelivery.remindersOnId])
        #expect(subject.permission == .allowed)
        let week = try await service.reminders(from: TestClock.now, to: TestClock.now.addingTimeInterval(ReminderDelivery.window))
        let pending = await center.pending
        #expect(Set(pending.keys) == Set(week.map(\.id)))
        for reminder in week {
            let trigger = try #require(pending[reminder.id]?.trigger)
            #expect(ReminderTrigger.instant(of: trigger) == reminder.fireAt)
        }
        #expect(Set(record.handedOverReminders.keys) == Set(week.map(\.id)))
        // Another pass: the same requests, no second prompt or "Reminders are on".
        subject.requestPass()
        await subject.settle()
        #expect(await center.permissionRequests == 1)
        #expect(await center.shownNow.count == 1)
        #expect(Set(await center.pending.keys) == Set(week.map(\.id)))
    }

    @Test("not allowed: nothing is scheduled, and what came due shows in the app at launch")
    func denied() async throws {
        let service = mock()
        let center = RecordingNotificationCenter(answer: false)
        let subject = delivery(service, center: center)
        try await subject.setConsent(true)
        await subject.settle()
        #expect(subject.permission == .denied)
        #expect(await center.pending.isEmpty)
        #expect(await center.shownNow.isEmpty)
        let due = [extraReminder("deadline_soon:24h:x@1", at: TestClock.now.addingTimeInterval(-3600))]
        await subject.launch(due: due)
        #expect(subject.catchUp.map(\.id) == due.map(\.id))
    }

    @Test("off at launch: the catch-up card; opening one or dismissing marks them shown")
    func catchUpCard() async throws {
        let time = TestTime()
        let service = ReminderDouble(base: mock(time))
        let subject = delivery(service, center: RecordingNotificationCenter(), time: time)
        let due = [
            extraReminder("deadline_soon:24h:a@1", at: TestClock.now.addingTimeInterval(-3600)),
            extraReminder("deadline_soon:24h:b@1", at: TestClock.now.addingTimeInterval(-1800)),
        ]
        await subject.launch(due: due)
        #expect(subject.catchUp.count == 2)
        _ = await subject.openCatchUp(due[0])
        #expect(subject.catchUp.map(\.id) == [due[1].id])
        await subject.dismissCatchUp()
        #expect(subject.catchUp.isEmpty)
        #expect(service.marked.all == due.map(\.id))
    }

    @Test("turning it off removes our requests and forgets them without marking them shown")
    func turnOff() async throws {
        let service = ReminderDouble(base: mock())
        let center = RecordingNotificationCenter()
        let record = InMemorySettingsStore()
        let subject = delivery(service, center: center, record: record)
        try await subject.setConsent(true)
        await subject.settle()
        #expect(!(await center.pending.isEmpty))
        try await subject.setConsent(false)
        await subject.settle()
        #expect(await center.pending.isEmpty)
        #expect(record.handedOverReminders.isEmpty)
        #expect(service.marked.all.isEmpty)
    }

    @Test("what fired while the app was closed is marked shown, never shown again, even if cleared")
    func firedWhileAway() async throws {
        let time = TestTime()
        let service = ReminderDouble(base: mock(time))
        let center = RecordingNotificationCenter()
        let record = InMemorySettingsStore()
        let subject = delivery(service, center: center, record: record, time: time)
        try await subject.setConsent(true)
        await subject.settle()
        // Two days later the system has fired what was due, and the student cleared it all.
        let later = TestClock.now.addingTimeInterval(2 * 86_400)
        let fired = record.handedOverReminders.filter { $0.value <= later }.map(\.key).sorted()
        #expect(!fired.isEmpty)
        await center.fire(until: later)
        await center.clearDelivered()
        time.set(later)
        subject.requestPass()
        await subject.settle()
        #expect(Set(service.marked.all) == Set(fired))
        // Only "Reminders are on" was ever shown by the app itself.
        #expect(await center.shownNow.map(\.id) == [ReminderDelivery.remindersOnId])
        #expect(record.handedOverReminders.keys.allSatisfy { !fired.contains($0) })
    }

    @Test("due but never handed over (a week away): shown now, once, and marked")
    func dueNeverHandedOver() async throws {
        let time = TestTime()
        let base = mock(time)
        let service = ReminderDouble(base: base)
        let center = RecordingNotificationCenter()
        let subject = delivery(service, center: center, time: time)
        try await subject.setConsent(true)
        await subject.settle()
        // The pending requests are lost (the app was away past the week; the record is fresh).
        let fresh = InMemorySettingsStore()
        await center.removePending(await center.pendingIds())
        subject.replace(service: service, center: center, record: fresh)
        let week = try await base.reminders(from: TestClock.now, to: TestClock.now.addingTimeInterval(ReminderDelivery.window))
        let deadline = try #require(week.first { $0.kind == .deadlineSoon })
        let after = deadline.fireAt.addingTimeInterval(60)
        time.set(after)
        let due = try await base.dueReminders(now: after)
        #expect(!due.isEmpty)
        subject.requestPass()
        await subject.settle()
        let shown = await center.shownNow.map(\.id)
        #expect(Set(due.map(\.id)).isSubset(of: Set(shown)))
        #expect(try await base.dueReminders(now: after).isEmpty)
        subject.requestPass()
        await subject.settle()
        #expect(await center.shownNow.count == shown.count)
    }

    @Test("a stale id never holds up the others; one that failed to save is marked next time, not shown again")
    func markingPerId() async throws {
        let time = TestTime()
        let past = TestClock.now.addingTimeInterval(-600)
        var service = ReminderDouble(base: mock(time))
        service.failing = ["deadline_soon:24h:stale@1": .invalid, "deadline_soon:24h:busy@1": .internal]
        let center = RecordingNotificationCenter(permission: .allowed)
        let record = InMemorySettingsStore()
        record.handedOverReminders = [
            "deadline_soon:24h:stale@1": past, "deadline_soon:24h:busy@1": past, "deadline_soon:24h:good@1": past,
        ]
        try await service.setReminderSettings(settings: try await service.reminderSettings().with { $0.runInBackground = true })
        let subject = delivery(service, center: center, record: record, time: time)
        subject.requestPass()
        await subject.settle()
        #expect(service.marked.all == ["deadline_soon:24h:good@1"])
        let kept = record.handedOverReminders
        #expect(kept["deadline_soon:24h:stale@1"] == nil)
        #expect(kept["deadline_soon:24h:good@1"] == nil)
        #expect(kept["deadline_soon:24h:busy@1"] == past)
    }

    @Test("at most 64 requests, the earliest; the rest follow on a later pass")
    func pendingLimit() async throws {
        let base = mock()
        let extra = (0..<70).map { extraReminder("deadline_soon:24h:e\($0)@1", at: TestClock.now.addingTimeInterval(Double($0 + 1) * 600)) }
        let service = ReminderDouble(base: base, extra: extra)
        let center = RecordingNotificationCenter()
        let record = InMemorySettingsStore()
        let subject = delivery(service, center: center, record: record)
        try await subject.setConsent(true)
        await subject.settle()
        let week = try await service.reminders(from: TestClock.now, to: TestClock.now.addingTimeInterval(ReminderDelivery.window))
        #expect(week.count > 64)
        #expect(Set(await center.pending.keys) == Set(week.prefix(64).map(\.id)))
        #expect(record.handedOverReminders.count <= 64)
    }

    @Test("a reminder no longer in the week (deadline reminders turned off) is removed from the system")
    func removed() async throws {
        let service = mock()
        let center = RecordingNotificationCenter()
        let subject = delivery(service, center: center)
        try await subject.setConsent(true)
        await subject.settle()
        #expect(await center.pending.keys.contains { $0.hasPrefix("deadline_soon:") })
        try await service.setReminderSettings(settings: try await service.reminderSettings().with { $0.deadlineSoon = false })
        subject.requestPass()
        await subject.settle()
        let pending = await center.pending.keys
        #expect(!pending.contains { $0.hasPrefix("deadline_soon:") })
        #expect(pending.contains { $0.hasPrefix("weekly_digest:") })
    }

    @Test("the model: mock data uses the in-memory center, never the system's; a refresh schedules")
    func modelUsesTheMockCenter() async throws {
        let usedLive = Mutex(false)
        let center = RecordingNotificationCenter()
        let service = mock()
        let model = AppModel(
            dataMode: .mock(.demo), strings: .app, settings: InMemorySettingsStore(language: .english),
            calendar: TestClock.calendar, clock: { TestClock.now }, notificationCenter: NotificationCenter(),
            service: service, reminders: true,
            reminderCenters: ReminderCenters(live: {
                usedLive.withLock { $0 = true }
                return RecordingNotificationCenter()
            }, mock: center)
        )
        let reminders = try #require(model.reminderDelivery)
        try await reminders.setConsent(true)
        await model.refresh()
        await reminders.settle()
        #expect(!(await center.pending.isEmpty))
        #expect(usedLive.withLock { $0 } == false)
    }
}
