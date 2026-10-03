// The menu bar extra's week (macos-shell §3.7 W10, §6.5; model-access design §5.3): what the
// facade's `weekly_digest()` says, laid out for a 340 pt menu. Structure only: titles, codes,
// weeks and counts, never material text.

import Foundation
import PageLampKit

public struct MenuBarWeek: Sendable {
    /// Deadlines shown (the digest's next 7 days, soonest first).
    public static let maxDeadlines = 5
    /// Plan items shown (today's).
    public static let maxPlanItems = 4

    public struct DueItem: Sendable, Identifiable {
        public var id: String { deadline.event.id }
        public let deadline: Deadline
        public let due: Date
        /// The course to open (nil for a feed event not linked to a course).
        public let courseId: String?
        public let courseCode: String?
    }

    /// The soonest deadlines, at most `maxDeadlines`.
    public let dueSoon: [DueItem]
    /// Deadlines in the next 7 days beyond `dueSoon`.
    public let moreDue: Int
    /// Today's plan items, at most `maxPlanItems`, and how many more there are.
    public let today: [StudyPlanItem]
    public let moreToday: Int
    /// Last week's plan: done of planned (nil without a saved plan or with nothing planned).
    public let lastWeek: (done: Int, planned: Int)?
    /// Active courses with their week.
    public let courses: [DigestCourse]
    /// How many deadlines are due within 24 hours (the lamp lights: spec §6.5).
    public let dueWithin24h: Int
    /// The AI-generated line of the plan the tasks come from (a plan PageLamp wrote), read
    /// with the digest; nil for the AI app's plan or when it couldn't be read.
    public internal(set) var aiLabel: AiLabel?

    /// - Parameter aiLabel: the latest plan's label, read together with `digest`.
    public init(digest: WeeklyDigest, now: Date, aiLabel: AiLabel? = nil) {
        self.aiLabel = aiLabel
        let all = digest.courses.flatMap { course in
            course.deadlines.compactMap { deadline -> DueItem? in
                guard let due = deadline.event.dueAt ?? deadline.event.startsAt else { return nil }
                return DueItem(deadline: deadline, due: due, courseId: course.courseId, courseCode: course.code ?? deadline.courseCode)
            }
        }
        .sorted { ($0.due, $0.id) < ($1.due, $1.id) }
        dueSoon = Array(all.prefix(Self.maxDeadlines))
        moreDue = max(0, all.count - Self.maxDeadlines)
        let items = digest.plan?.today ?? []
        today = Array(items.prefix(Self.maxPlanItems))
        moreToday = max(0, items.count - Self.maxPlanItems)
        if let plan = digest.plan, plan.lastWeekPlanned > 0 {
            lastWeek = (Int(plan.lastWeekDone), Int(plan.lastWeekPlanned))
        } else {
            lastWeek = nil
        }
        courses = digest.courses.filter { $0.active && $0.week != nil }
        dueWithin24h = all.filter { $0.due >= now && $0.due.timeIntervalSince(now) <= 24 * 3600 }.count
    }

    /// Nothing to show: no course in the digest and no plan today.
    public var isEmpty: Bool {
        dueSoon.isEmpty && today.isEmpty && courses.isEmpty && lastWeek == nil
    }
}
