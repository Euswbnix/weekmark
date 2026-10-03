// `ReminderSettings` to change: the generated record's fields are constants, so a change is made
// on a draft and turned back into the record (`settings.with { $0.planToday = true }`).

import Foundation
import PageLampKit

public struct ReminderSettingsDraft: Equatable, Sendable {
    public var deadlineSoon: Bool
    public var weeklyDigest: Bool
    public var digestDay: DayOfWeek
    public var digestTime: String
    public var planToday: Bool
    public var planTodayTime: String
    /// "Remind me" (in the Mac app, its own answer: the facade keeps it apart from the Tauri app's).
    public var runInBackground: Bool

    public init(_ settings: ReminderSettings) {
        deadlineSoon = settings.deadlineSoon
        weeklyDigest = settings.weeklyDigest
        digestDay = settings.digestDay
        digestTime = settings.digestTime
        planToday = settings.planToday
        planTodayTime = settings.planTodayTime
        runInBackground = settings.runInBackground
    }

    public var settings: ReminderSettings {
        ReminderSettings(
            deadlineSoon: deadlineSoon, weeklyDigest: weeklyDigest, digestDay: digestDay, digestTime: digestTime,
            planToday: planToday, planTodayTime: planTodayTime, runInBackground: runInBackground
        )
    }
}

extension ReminderSettings {
    /// These settings with `change` applied.
    public func with(_ change: (inout ReminderSettingsDraft) -> Void) -> ReminderSettings {
        var draft = ReminderSettingsDraft(self)
        change(&draft)
        return draft.settings
    }
}
