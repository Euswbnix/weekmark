// The words and dates of the This Week page (spec §3.1 copy table, §6.4, §6.5, §7.1, §7.3), in the
// student's language. Dates and times come from `Date.FormatStyle` in the active locale and the
// model's calendar (24-hour where the locale uses it); sentences come from whole-sentence keys.

import Foundation
import PageLampKit

/// Formats This Week's text as of `now`.
public struct ThisWeekText: Sendable {
    public let l10n: L10n
    public let calendar: Calendar
    public let now: Date

    public init(l10n: L10n, calendar: Calendar, now: Date) {
        self.l10n = l10n
        self.calendar = calendar
        self.now = now
    }

    // MARK: - Dates and times

    private var style: Date.FormatStyle {
        Date.FormatStyle(locale: l10n.locale, calendar: calendar, timeZone: calendar.timeZone)
    }

    /// "11:59 PM" / "23:59".
    public func time(_ date: Date) -> String {
        date.formatted(
            Date.FormatStyle(date: .omitted, time: .shortened, locale: l10n.locale, calendar: calendar, timeZone: calendar.timeZone)
        )
    }

    /// The page title: "Friday, September 25" / "9月25日 星期五".
    public var todayTitle: String {
        now.formatted(style.weekday(.wide).month(.wide).day())
    }

    /// Calendar days from today to `date` (tomorrow = 1, yesterday = -1).
    public func dayOffset(_ date: Date) -> Int {
        ThisWeekDigest.dayOffset(of: date, from: now, calendar: calendar)
    }

    /// A day's heading: "Today", "Tomorrow", "Yesterday", else the weekday ("Sunday" / "星期日").
    public func dayTitle(_ day: Date) -> String {
        switch dayOffset(day) {
        case 0: l10n("common.time.today")
        case 1: l10n("common.time.tomorrow")
        case -1: l10n("common.time.yesterday")
        default: day.formatted(style.weekday(.wide))
        }
    }

    /// The date under a day's heading: "Fri, Sep 25" under a relative word, else "Sep 27"
    /// (the weekday is already the heading).
    public func dayDate(_ day: Date) -> String {
        if (-1 ... 1).contains(dayOffset(day)) {
            return day.formatted(style.weekday(.abbreviated).month(.abbreviated).day())
        }
        return day.formatted(style.month(.abbreviated).day())
    }

    /// "Sep 22" / "9月22日".
    public func shortDate(_ date: Date) -> String {
        date.formatted(style.month(.abbreviated).day())
    }

    /// When something happens, briefly: "Today 11:59 PM", "Tomorrow 9:00 AM", "Sat 9:00 AM"
    /// within the week, else "Oct 7, 6:00 PM".
    public func shortWhen(_ date: Date) -> String {
        let offset = dayOffset(date)
        switch offset {
        case 0: return l10n("mac.thisWeek.todayAt", ["time": time(date)])
        case 1: return l10n("mac.thisWeek.tomorrowAt", ["time": time(date)])
        case 2 ..< ThisWeekDigest.days: return date.formatted(style.weekday(.abbreviated).hour().minute())
        default: return date.formatted(style.month(.abbreviated).day().hour().minute())
        }
    }

    /// For VoiceOver: the full date and time ("Friday, September 25 at 11:59 PM").
    public func fullWhen(_ date: Date) -> String {
        date.formatted(style.weekday(.wide).month(.wide).day().hour().minute())
    }

    /// Next up's due line: "Due today 11:59 PM" / "今天 23:59 截止" (spec §3.1).
    public func due(_ date: Date) -> String {
        switch dayOffset(date) {
        case 0: l10n("mac.thisWeek.dueToday", ["time": time(date)])
        case 1: l10n("mac.thisWeek.dueTomorrow", ["time": time(date)])
        default: l10n("common.time.due", ["when": shortWhen(date)])
        }
    }

    /// The countdown: "in 58 min", "in 5 h 12 min", "in 3 h" / "还有 58 分钟" (spec §6.5).
    public func countdown(_ countdown: Countdown) -> String {
        if countdown.minutes < 60 {
            return l10n.plural("mac.countdown.minutes", count: countdown.minutes)
        }
        if countdown.minutesPastHour == 0 {
            return l10n.plural("mac.countdown.hours", count: countdown.hours)
        }
        return l10n(
            "mac.countdown.hoursMinutes",
            ["hours": l10n.number(countdown.hours), "minutes": l10n.number(countdown.minutesPastHour)]
        )
    }

    /// For VoiceOver: "in 58 minutes" / "58分钟后".
    public func spokenRemaining(until date: Date) -> String {
        now.formatted(relativeStyle(anchor: date))
    }

    /// "2 days ago" / "2天前": calendar days for an earlier day (a plan saved on Wednesday
    /// evening was made "2 days ago" on Friday morning), else hours or minutes.
    public func ago(_ date: Date) -> String {
        let days = dayOffset(date)
        if days < 0 {
            // Exactly `days` calendar days before now, so the style counts days, not hours.
            let anchor = calendar.date(byAdding: .day, value: days, to: now) ?? date
            return now.formatted(relativeStyle(anchor: anchor))
        }
        // Within the last minute (a plan just saved): "now", never "in 0 seconds".
        if now.timeIntervalSince(date) < 60 {
            return now.formatted(relativeStyle(anchor: now, presentation: .named))
        }
        return now.formatted(relativeStyle(anchor: min(date, now)))
    }

    /// `anchor` relative to the formatted date ("now"): numeric, full units, this locale and
    /// calendar. A value type (Foundation caches its formatter), so view bodies can call it.
    private func relativeStyle(
        anchor: Date, presentation: Date.AnchoredRelativeFormatStyle.Presentation = .numeric
    ) -> Date.AnchoredRelativeFormatStyle {
        Date.AnchoredRelativeFormatStyle(
            anchor: anchor,
            presentation: presentation,
            unitsStyle: .wide,
            locale: l10n.locale,
            calendar: calendar,
            capitalizationContext: .middleOfSentence
        )
    }

    // MARK: - Header and Next 7 days

    /// "3 deadlines in the next 7 days · 2 plan tasks today": one sentence per count. Without
    /// the deadlines (they failed to load, S14) it never claims "Nothing due"; nil if nothing
    /// is left to say.
    public func summary(_ digest: ThisWeekDigest, deadlinesLoaded: Bool = true) -> String? {
        var parts: [String] = []
        if deadlinesLoaded {
            parts.append(
                digest.dueCount > 0
                    ? l10n.plural("courses.thisWeek.summary", count: digest.dueCount)
                    : l10n("courses.thisWeek.nothingDue")
            )
        }
        if !digest.planToday.isEmpty {
            parts.append(l10n.plural("mac.thisWeek.planTasksToday", count: digest.planToday.count))
        }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// Next 7 days' trailing detail: "3 deadlines · 1 class" (zero counts left out).
    public func next7Detail(_ digest: ThisWeekDigest) -> String? {
        var parts: [String] = []
        if digest.dueCount > 0 { parts.append(l10n.plural("mac.thisWeek.deadlineCount", count: digest.dueCount)) }
        if digest.classCount > 0 { parts.append(l10n.plural("mac.thisWeek.classCount", count: digest.classCount)) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// A deadline's course: its code, else its name.
    public func course(of deadline: Deadline) -> String? {
        deadline.courseCode ?? deadline.courseName
    }

    /// "Next: Quiz 3 · Sat 9:00 AM" (`courses.card.next`).
    public func next(_ deadline: Deadline) -> String? {
        guard let when = ThisWeekDigest.time(of: deadline) else { return nil }
        // The day and time never split across lines (the title may wrap).
        return l10n("courses.card.next", ["title": deadline.event.title, "when": TextWrap.keepTogether(shortWhen(when))])
    }

    /// VoiceOver for a Next 7 days row or Next up (spec §7.1): "Problem Set 2. DEMO205,
    /// Assignment. Due Friday, September 25 at 11:59 PM, in 58 minutes." Classes aren't "due".
    public func spokenDeadline(_ deadline: Deadline, withCountdown: Bool = false) -> String {
        let event = deadline.event
        guard let date = ThisWeekDigest.time(of: deadline) else { return event.title }
        var arguments = ["title": event.title, "kind": l10n.eventKind(event.kind), "when": fullWhen(date)]
        let course = course(of: deadline)
        if let course { arguments["course"] = course }
        if event.kind == .classEvent {
            return l10n(course == nil ? "mac.thisWeek.a11y.eventRowNoCourse" : "mac.thisWeek.a11y.eventRow", arguments)
        }
        if withCountdown {
            arguments["remaining"] = spokenRemaining(until: date)
            return l10n(
                course == nil ? "mac.thisWeek.a11y.deadlineRowCountdownNoCourse" : "mac.a11y.deadlineRowCountdown",
                arguments
            )
        }
        return l10n(course == nil ? "mac.thisWeek.a11y.deadlineRowNoCourse" : "mac.a11y.deadlineRow", arguments)
    }

    // MARK: - Study plan

    /// "Made by your AI app 2 days ago · covers Sep 22 – Oct 5" (or "Made by PageLamp …").
    public func planMeta(_ stored: StoredStudyPlan) -> String {
        func date(_ iso: String) -> String {
            IsoDate.date(from: iso, calendar: calendar).map(shortDate) ?? iso
        }
        return l10n(stored.origin == .pageLamp ? "courses.plan.madeByPageLamp" : "courses.plan.madeBy", [
            "when": ago(stored.createdAt),
            "start": date(stored.plan.horizonStart),
            "end": date(stored.plan.horizonEnd),
        ])
    }

    /// "30 min" / "30 分钟".
    public func planMinutes(_ minutes: UInt32) -> String {
        l10n("courses.plan.minutes", ["count": l10n.number(minutes)])
    }

    /// VoiceOver for a plan task (static): "Skim Week 4 slides, DEMO101, 30 minutes, not done yet".
    public func spokenPlanItem(_ item: StudyPlanItem, course: String?) -> String {
        var arguments = [
            "title": item.title,
            "state": l10n(item.done ? "courses.plan.done" : "courses.plan.notDone"),
        ]
        if let course { arguments["course"] = course }
        if let minutes = item.minutes { arguments["duration"] = l10n.plural("mac.a11y.minutes", count: Int(minutes)) }
        let key = switch (course != nil, item.minutes != nil) {
        case (true, true): "mac.a11y.planItem"
        case (false, true): "mac.thisWeek.a11y.planItemNoCourse"
        case (true, false): "mac.thisWeek.a11y.planItemNoDuration"
        case (false, false): "mac.thisWeek.a11y.planItemTitleOnly"
        }
        return l10n(key, arguments)
    }

    // MARK: - Contents

    /// The Contents week column: "Week 4" / "第 4 周", else "—" (S11).
    public func week(_ state: CourseWeekState) -> String {
        switch state {
        case .week(let week): l10n("common.week.current", ["week": l10n.number(week)])
        case .unknown, .outsideTerm: l10n("mac.common.week.none")
        }
    }

    /// The week in words, for VoiceOver and hints: "Week 4", "Week unknown", "Outside term".
    public func spokenWeek(_ state: CourseWeekState) -> String {
        switch state {
        case .week(let week): l10n("common.week.current", ["week": l10n.number(week)])
        case .unknown: l10n("common.week.unknown")
        case .outsideTerm: l10n("common.week.outsideTerm")
        }
    }

    /// The Contents policy line: "Learning aid only · 12 of 14 readable", "No AI · materials not
    /// shared", "AI policy not set · 3 of 6 readable" (spec §6.4).
    public func policyLine(_ summary: CourseSummary) -> String {
        policyLineParts(summary).joined(separator: " · ")
    }

    /// The policy line's two items: the policy, then what the AI app may read.
    public func policyLineParts(_ summary: CourseSummary) -> [String] {
        let policyName = summary.course.aiPolicy == .unknown
            ? l10n("mac.thisWeek.contents.policyNotSet")
            : l10n.policy(summary.course.aiPolicy)
        let materials: String = switch summary.aiMaterials {
        case .withheldByPolicy:
            l10n("mac.thisWeek.contents.notShared")
        case .turnedOff:
            l10n("mac.thisWeek.contents.aiOff")
        case .readable:
            summary.counts.materials == 0
                ? l10n("common.aiMaterials.readableNone")
                : l10n("mac.course.readableShort", [
                    "indexed": l10n.number(summary.counts.indexedMaterials),
                    "count": l10n.number(summary.counts.materials),
                ])
        }
        return [policyName, materials]
    }

    /// The materials sentence for VoiceOver: "12 of 14 materials readable by your AI app".
    public func spokenMaterials(_ summary: CourseSummary) -> String {
        switch summary.aiMaterials {
        case .withheldByPolicy:
            l10n("common.aiMaterials.withheld_by_policy")
        case .turnedOff:
            l10n("common.aiMaterials.turned_off")
        case .readable:
            summary.counts.materials == 0
                ? l10n("common.aiMaterials.readableNone")
                : l10n.plural(
                    "common.aiMaterials.readable",
                    count: Int(summary.counts.materials),
                    ["indexed": l10n.number(summary.counts.indexedMaterials)]
                )
        }
    }

    /// A Contents row as one VoiceOver element (spec §7.1): "DEMO101, Intro to Demo Studies.
    /// Week 4. Next: Quiz 3 · Saturday, September 26 at 9:00 AM. AI policy: Learning aid only.
    /// 12 of 14 materials readable by your AI app."
    public func spokenContentsRow(_ summary: CourseSummary) -> String {
        var arguments = [
            "code": ThisWeekContents.label(summary),
            "name": summary.course.name,
            "week": spokenWeek(CourseWeekState(summary.timeline)),
            "policy": l10n.policy(summary.course.aiPolicy),
            "materials": spokenMaterials(summary),
        ]
        if let next = summary.nextDeadline, let when = ThisWeekDigest.time(of: next) {
            arguments["next"] = l10n("courses.card.next", ["title": next.event.title, "when": fullWhen(when)])
            return l10n("mac.a11y.contentsRow", arguments)
        }
        return l10n("mac.a11y.contentsRowNoDeadline", arguments)
    }
}
