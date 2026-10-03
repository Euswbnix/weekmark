// A reminder's words (model-access design §5.3): course codes and titles only, never material
// text. The same keys as the Tauri app's notifications (`reminders.notify.*`); times are shown in
// the reminder's own time zone.

import Foundation
import PageLampKit

/// One notification's text; `id` is the reminder's (the request's identifier).
public struct ReminderNotificationText: Equatable, Sendable {
    public let id: String
    public let title: String
    public let body: String

    public init(id: String, title: String, body: String) {
        self.id = id
        self.title = title
        self.body = body
    }
}

public enum ReminderText {
    /// `reminder` as a notification in `l10n`'s language.
    public static func notification(for reminder: Reminder, l10n: L10n) -> ReminderNotificationText {
        let id = reminder.id
        switch reminder.kind {
        case .deadlineSoon:
            let title = reminder.title ?? ""
            let code = reminder.courseCode ?? reminder.courseName
            return ReminderNotificationText(
                id: id,
                title: code.map { l10n("reminders.notify.deadlineTitle", ["code": $0, "title": title]) } ?? title,
                body: reminder.dueAt.map {
                    l10n("reminders.notify.deadlineBody", ["when": when($0, zone: reminder.timeZone, l10n: l10n)])
                } ?? ""
            )
        case .weeklyDigest:
            let count = Int(reminder.count ?? 0)
            return ReminderNotificationText(
                id: id,
                title: l10n("reminders.notify.digestTitle"),
                body: count > 0
                    ? l10n.plural("reminders.notify.digestBody", count: count)
                    : l10n("reminders.notify.digestBodyNone")
            )
        case .planToday:
            return ReminderNotificationText(
                id: id,
                title: l10n("reminders.notify.planTitle"),
                body: l10n.plural("reminders.notify.planBody", count: Int(reminder.count ?? 0))
            )
        }
    }

    /// The one notification after the student said yes to reminders.
    public static func remindersOn(l10n: L10n) -> (title: String, body: String) {
        (l10n("reminders.notify.onTitle"), l10n("reminders.notify.onBody"))
    }

    /// "Tuesday 11:59 PM" / "星期二 23:59" in `zone` (the Mac's own zone if it doesn't know that
    /// one). The weekday and the time are formatted apart, as the Tauri app does.
    public static func when(_ date: Date, zone: String, l10n: L10n) -> String {
        let timeZone = TimeZone(identifier: zone) ?? .current
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        let weekday = date.formatted(
            Date.FormatStyle(locale: l10n.locale, calendar: calendar, timeZone: timeZone).weekday(.wide)
        )
        let time = date.formatted(
            Date.FormatStyle(date: .omitted, time: .shortened, locale: l10n.locale, calendar: calendar, timeZone: timeZone)
        )
        return l10n("reminders.notify.when", ["weekday": weekday, "time": time])
    }
}
