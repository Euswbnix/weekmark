// This Week beyond the day grouping: page state, fixes, Next up's countdown, the study plan by
// day, the Contents order and each course's week (spec §3.1, §3.9, §6.4, §6.5).

import Foundation
import PageLampKit
import PageLampModel
import Testing

@Suite("This Week sections")
struct ThisWeekSectionsTests {
    let now = TestClock.now
    let calendar = TestClock.calendar

    // MARK: Page state

    @Test("the page state follows courses, sources, syncing and errors (S3, S4, first sync)")
    func pageState() {
        #expect(ThisWeekPageState.resolve(courseCount: 3, sourceCount: 2, syncing: false, coursesFailed: false) == .page)
        #expect(ThisWeekPageState.resolve(courseCount: 0, sourceCount: 0, syncing: false, coursesFailed: false) == .noSources)
        #expect(ThisWeekPageState.resolve(courseCount: 0, sourceCount: 2, syncing: false, coursesFailed: false) == .noCourses)
        #expect(ThisWeekPageState.resolve(courseCount: 0, sourceCount: 2, syncing: true, coursesFailed: false) == .firstSync)
        // An error is not "no courses": the page stays and its Contents shows the error (S14).
        #expect(ThisWeekPageState.resolve(courseCount: 0, sourceCount: 2, syncing: false, coursesFailed: true) == .page)
    }

    @Test("the model's page state on mock data")
    @MainActor
    func modelPageState() async {
        let (demo, _) = makeModel(scenario: .demo)
        await demo.refresh()
        #expect(demo.thisWeekPageState == .page)
        // This Week has no tinted action of its own (spec §1.2).
        #expect(demo.thisWeekCandidates.isEmpty)

        let (empty, _) = makeModel(scenario: .empty)
        await empty.refresh()
        #expect(empty.thisWeekPageState == .noSources)
        #expect(empty.thisWeekCandidates == [.pagePrimary])
    }

    @Test("a rejected token offers no second fix button on This Week: the capsule's bubble is the fix")
    @MainActor
    func fixCandidates() async {
        let (expired, _) = makeModel(scenario: .expired)
        await expired.refresh()
        #expect(expired.thisWeekCandidates.isEmpty)
        #expect(expired.showsCapsuleFix)
        #expect(expired.primaryActionWinner(for: expired.thisWeekCandidates) == .capsuleFix)

        let (missing, _) = makeModel(scenario: .error)
        await missing.refresh()
        #expect(!missing.failingSources.isEmpty)
        #expect(missing.thisWeekCandidates.isEmpty)
        #expect(!missing.showsCapsuleFix)
        #expect(missing.primaryActionWinner(for: missing.thisWeekCandidates) == nil)
    }

    // MARK: Countdown

    @Test("the countdown rounds up to whole minutes and never says 0")
    func countdown() {
        #expect(Countdown(from: now, to: now.addingTimeInterval(58 * 60)).minutes == 58)
        #expect(Countdown(from: now, to: now.addingTimeInterval(57 * 60 + 1)).minutes == 58)
        #expect(Countdown(from: now, to: now.addingTimeInterval(30)).minutes == 1)
        #expect(Countdown(from: now, to: now).minutes == 1)
        let later = Countdown(from: now, to: now.addingTimeInterval(5 * 3600 + 12 * 60))
        #expect(later.hours == 5 && later.minutesPastHour == 12)
    }

    @Test("the header's next deadline skips classes and deadlines that already passed")
    func nextDeadline() {
        let digest = ThisWeekDigest(
            deadlines: [
                deadline("Passed this morning", .assignmentDue, day: 0, hour: 9),
                deadline("Lecture", .classEvent, day: 0, hour: 11),
                deadline("Quiz", .quizDue, day: 2, hour: 9),
            ],
            plan: nil, now: now, calendar: calendar
        )
        #expect(digest.nextDeadline(after: now)?.event.title == "Quiz")
        #expect(digest.nextDeadline(after: TestClock.at(0, 8))?.event.title == "Passed this morning")
        #expect(digest.nextDeadline(after: TestClock.at(3, 0)) == nil)
    }

    @Test("under 2 hours is the final stretch (only the glyph changes)")
    func finalStretch() {
        #expect(Countdown(from: now, to: now.addingTimeInterval(119 * 60)).isFinalStretch)
        #expect(!Countdown(from: now, to: now.addingTimeInterval(120 * 60)).isFinalStretch)
    }

    // MARK: Study plan

    private func plan(created: Date, horizon: (Int, Int), items: [(Int, String)]) -> StoredStudyPlan {
        func iso(_ days: Int) -> String {
            IsoDate.string(from: TestClock.at(days, 12), calendar: calendar)
        }
        return StoredStudyPlan(
            id: 1,
            createdAt: created,
            plan: StudyPlan(
                horizonStart: iso(horizon.0),
                horizonEnd: iso(horizon.1),
                items: items.map { days, title in
                    StudyPlanItem(
                        date: iso(days), courseId: nil, title: title, description: nil,
                        materialIds: [], minutes: nil, done: false
                    )
                },
                notes: nil
            )
        )
    }

    @Test("groups the plan by date, keeps plan order within a day, and puts today + 2 days in focus")
    func planDays() {
        let digest = StudyPlanDigest(
            plan(
                created: TestClock.at(-1, 9),
                horizon: (-1, 10),
                items: [(4, "Later"), (0, "Today A"), (-1, "Yesterday"), (2, "In two days"), (0, "Today B"), (3, "In three days")]
            ),
            now: now,
            calendar: calendar
        )
        #expect(digest.days.map { $0.entries.map(\.item.title) } == [
            ["Yesterday"], ["Today A", "Today B"], ["In two days"], ["In three days"], ["Later"],
        ])
        #expect(digest.focus.flatMap { $0.entries.map(\.item.title) } == ["Today A", "Today B", "In two days"])
        #expect(digest.others.flatMap { $0.entries.map(\.item.title) } == ["Yesterday", "In three days", "Later"])
        // Positions are the items' places in the saved plan (stable identities).
        #expect(digest.focus[0].entries.map(\.position) == [1, 4])
        #expect(digest.focus[0].day == calendar.startOfDay(for: now))
        #expect(!digest.isStale)
    }

    @Test("a plan is stale when its horizon has ended or it is more than 7 days old")
    func planStale() {
        let ended = StudyPlanDigest(plan(created: TestClock.at(-2, 9), horizon: (-5, -1), items: []), now: now, calendar: calendar)
        #expect(ended.isStale)
        let old = StudyPlanDigest(plan(created: TestClock.at(-8, 9), horizon: (-8, 10), items: []), now: now, calendar: calendar)
        #expect(old.isStale)
        let endsToday = StudyPlanDigest(plan(created: TestClock.at(-7, 11), horizon: (-7, 0), items: []), now: now, calendar: calendar)
        #expect(!endsToday.isStale)
    }

    // MARK: Contents

    @Test("Contents: hidden courses left out, active ones by code, past ones last")
    @MainActor
    func contentsOrder() async {
        let (model, _) = makeModel(scenario: .demo)
        await model.refresh()
        #expect(ThisWeekContents.courses(model.courses).map(ThisWeekContents.label) == ["DEMO101", "DEMO205", "DEMO310"])
        #expect(!ThisWeekContents.allHidden(model.courses))
        #expect(!ThisWeekContents.allHidden([]))

        let past = thisWeekSummary(code: "OLD100", active: false)
        let hidden = thisWeekSummary(code: "AAA100", hidden: true)
        let second = thisWeekSummary(code: "MAT137")
        let first = thisWeekSummary(code: "CSC108")
        #expect(ThisWeekContents.courses([past, hidden, second, first]).map(ThisWeekContents.label) == ["CSC108", "MAT137", "OLD100"])
        #expect(ThisWeekContents.allHidden([hidden]))
    }

    @Test("plan items name their course by id or code; unknown ids are not shown")
    @MainActor
    func planCourseLabel() async {
        let (model, _) = makeModel(scenario: .demo)
        await model.refresh()
        #expect(ThisWeekContents.planCourseLabel("folder:demo-courses/course/DEMO101", courses: model.courses) == "DEMO101")
        #expect(ThisWeekContents.planCourseLabel("DEMO205", courses: model.courses) == "DEMO205")
        #expect(ThisWeekContents.planCourseLabel("canvas:elsewhere/course/42", courses: model.courses) == nil)
        #expect(ThisWeekContents.planCourseLabel("PHY131", courses: model.courses) == "PHY131")
        #expect(ThisWeekContents.planCourseLabel(nil, courses: model.courses) == nil)
    }

    @Test("a course's week: current, unknown (S11) or outside the term (S11)")
    func weekState() {
        #expect(CourseWeekState(thisWeekTimeline(week: 4)) == .week(4))
        #expect(CourseWeekState(thisWeekTimeline(week: nil)) == .unknown)
        #expect(CourseWeekState(thisWeekTimeline(week: 13, outsideTerm: true)) == .outsideTerm)
    }
}

// MARK: - Fixtures

func thisWeekTimeline(week: UInt32?, outsideTerm: Bool = false) -> CourseTimeline {
    testTimeline(week: week, outsideTerm: outsideTerm)
}

func thisWeekSummary(
    code: String,
    name: String = "A Made-Up Course",
    policy: AiPolicy = .learningAid,
    materials: AiMaterialsState = .readable,
    counts: (indexed: UInt32, total: UInt32) = (12, 14),
    week: UInt32? = 4,
    active: Bool = true,
    hidden: Bool = false,
    next: Deadline? = nil
) -> CourseSummary {
    CourseSummary(
        course: Course(
            id: "folder:test/course/\(code)", sourceId: "folder:test", externalId: code, code: code, name: name,
            termStart: nil, termEnd: nil, termSource: .none, url: nil, aiPolicy: policy, aiPolicyNote: nil,
            aiAccess: materials != .turnedOff, materialSharing: .unanswered, hidden: hidden, enrollmentActive: active,
            updatedAt: TestClock.now
        ),
        aiMaterials: materials,
        timeline: thisWeekTimeline(week: week),
        lifecycle: testLifecycle(),
        counts: CourseCounts(modules: 0, materials: counts.total, indexedMaterials: counts.indexed, upcomingDeadlines: 0),
        nextDeadline: next,
        sourceLabel: "Course folder",
        lastSyncedAt: nil
    )
}
