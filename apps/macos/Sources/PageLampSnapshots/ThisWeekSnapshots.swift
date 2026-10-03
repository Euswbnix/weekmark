// This Week in each of its states (spec §3.1, §3.9, §7.4) for the snapshot catalogue: S7 above a
// full week, S6 after a crash, Next up, S12 quiet empties, S14 section errors, a stale plan, S3,
// S4, the first sync and the minimum window. States the demo data lacks come from a `FixtureService` over the mock.

import SwiftUI
import PageLamp
import PageLampKit
import PageLampModel

enum ThisWeekSnapshots {
    /// The narrowest detail column: the minimum window beside the ideal sidebar.
    static let narrowWidth = WindowMetrics.mainMinWidth - WindowMetrics.sidebarIdeal

    /// The states, named `this-week-<state>`.
    static let pages: [SnapshotPage] = [
        // The preview's default: an expired Canvas token (S7) above a full week.
        page("default", SnapshotSetup(scenario: .preview)),
        // S6: the app crashed yesterday evening (the crash notice comes first).
        page("crashed", SnapshotSetup(scenario: .crashed)),
        // Next up, 58 minutes before the deadline (final stretch glyph).
        page("next-up", SnapshotSetup(
            moment: { now, calendar in at(now, calendar, days: 0, hour: 23, minute: 1) },
            service: { mock, now, calendar in
                FixtureService(base: mock, extraDeadlines: [dueTonight(now: now, calendar: calendar)])
            }
        )),
        // Next up in the morning: hours and minutes, plain clock glyph.
        page("next-up-morning", SnapshotSetup(service: { mock, now, calendar in
            FixtureService(base: mock, extraDeadlines: [dueTonight(now: now, calendar: calendar)])
        })),
        // S12: nothing due, no plan yet.
        page("quiet", SnapshotSetup(service: { mock, _, _ in FixtureService(base: mock, deadlines: [], plan: .none) })),
        // S14: every section failed to load.
        page("errors", SnapshotSetup(service: { mock, _, _ in
            FixtureService(base: mock, failing: [.courses, .deadlines, .studyPlan])
        })),
        // An old plan whose horizon has ended, with Show Full Plan open.
        page("stale-plan", SnapshotSetup(service: { mock, now, calendar in
            FixtureService(base: mock, plan: .stored(stalePlan(now: now, calendar: calendar)))
        }), planExpanded: true),
        // S3: no sources.
        page("no-sources", SnapshotSetup(scenario: .empty)),
        // S4 with a failing source (S7) above it.
        page("no-courses", SnapshotSetup(scenario: .preview, service: { mock, _, _ in FixtureService(base: mock, courses: []) })),
        // The first sync: courses on their way.
        page("first-sync", SnapshotSetup(service: { mock, _, _ in
            FixtureService(base: mock, courses: [], syncInProgress: true)
        })),
        // The minimum window (zh must wrap, never truncate).
        page("narrow", SnapshotSetup(scenario: .preview), width: narrowWidth),
    ]

    private static func page(
        _ state: String, _ setup: SnapshotSetup, planExpanded: Bool = false, width: CGFloat = SnapshotCatalog.detailWidth
    ) -> SnapshotPage {
        SnapshotPage(name: "this-week-\(state)", width: width, setup: setup) { _ in
            AnyView(ThisWeekPage(planExpanded: planExpanded))
        }
    }

    // MARK: - Fixture data (made up, like the mock)

    private nonisolated static func at(_ now: Date, _ calendar: Calendar, days: Int, hour: Int, minute: Int = 0) -> Date {
        let day = calendar.date(byAdding: .day, value: days, to: calendar.startOfDay(for: now)) ?? now
        return calendar.date(bySettingHour: hour, minute: minute, second: 0, of: day) ?? day
    }

    /// DEMO205's "Problem Set 2 — questions 1–3", due tonight at 23:59.
    private nonisolated static func dueTonight(now: Date, calendar: Calendar) -> Deadline {
        let due = at(now, calendar, days: 0, hour: 23, minute: 59)
        return Deadline(
            event: Event(
                id: "ical:demo-calendar/event/tonight",
                sourceId: "ical:demo-calendar",
                courseId: "canvas:canvas.demo.test/course/205",
                kind: .assignmentDue,
                title: "Problem Set 2 — questions 1–3",
                startsAt: nil,
                endsAt: nil,
                dueAt: due,
                url: "https://canvas.demo.test/calendar#event-tonight",
                updatedAt: now,
                courseHint: nil
            ),
            courseCode: "DEMO205",
            courseName: "Foundations of Sample Data"
        )
    }

    /// A plan made ten days ago whose horizon ended yesterday.
    private nonisolated static func stalePlan(now: Date, calendar: Calendar) -> StoredStudyPlan {
        func date(_ days: Int) -> String {
            PageLampModel.IsoDate.string(from: at(now, calendar, days: days, hour: 12), calendar: calendar)
        }
        func item(_ days: Int, _ title: String, _ minutes: UInt32?, done: Bool) -> StudyPlanItem {
            StudyPlanItem(
                date: date(days), courseId: "folder:demo-courses/course/DEMO101", title: title,
                description: nil, materialIds: [], minutes: minutes, done: done
            )
        }
        return StoredStudyPlan(
            id: 7,
            createdAt: at(now, calendar, days: -10, hour: 20),
            plan: StudyPlan(
                horizonStart: date(-10),
                horizonEnd: date(-1),
                items: [
                    item(-9, "Skim Week 3 slides", 30, done: true),
                    item(-6, "Lab 3 notebook, parts 1–2", 60, done: true),
                    item(-2, "Reading response 3", 45, done: false),
                    item(-1, "Review sampling vocabulary", nil, done: false),
                ],
                notes: "Catch up on the reading response before starting Week 4."
            ),
            origin: .aiApp
        )
    }
}
