// This Week's words and dates in English and Simplified Chinese (spec §3.1 copy table, §6.4,
// §6.5, §7.1). Fixed locales (en_US, zh-Hans_CN) and the Toronto test clock, so the results
// don't depend on the machine's region.

import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Testing

@Suite("This Week text")
struct ThisWeekTextTests {
    let en = PlainText(ThisWeekText(
        l10n: L10n(locale: Locale(identifier: "en_US"), table: .app), calendar: TestClock.calendar, now: TestClock.now
    ))
    let zh = PlainText(ThisWeekText(
        l10n: L10n(locale: Locale(identifier: "zh-Hans_CN"), table: .app), calendar: TestClock.calendar, now: TestClock.now
    ))

    private func due(_ title: String, day: Int, hour: Int, minute: Int = 0, course: String? = "DEMO205", kind: EventKind = .assignmentDue) -> Deadline {
        let when = TestClock.at(day, hour, minute)
        let isClass = kind == .classEvent
        return Deadline(
            event: Event(
                id: title, sourceId: "ical:test", courseId: nil, kind: kind, title: title,
                startsAt: isClass ? when : nil, endsAt: nil, dueAt: isClass ? nil : when,
                url: nil, updatedAt: TestClock.now, courseHint: nil
            ),
            courseCode: course,
            courseName: nil
        )
    }

    @Test("today's title: \"Friday, September 25\" / \"9月25日 星期五\"")
    func todayTitle() {
        #expect(en.todayTitle == "Friday, September 25")
        #expect(zh.todayTitle == "9月25日 星期五")
    }

    @Test("Next up's due line and countdown follow the copy table")
    func nextUp() {
        let tonight = TestClock.at(0, 23, 59)
        #expect(en.due(tonight) == "Due today 11:59 PM")
        #expect(zh.due(tonight) == "今天 23:59 截止")
        #expect(en.due(TestClock.at(1, 9)) == "Due tomorrow 9:00 AM")
        #expect(zh.due(TestClock.at(1, 9)) == "明天 9:00 截止")

        #expect(en.countdown(Countdown(from: TestClock.now, to: TestClock.now.addingTimeInterval(58 * 60))) == "in 58 min")
        #expect(zh.countdown(Countdown(from: TestClock.now, to: TestClock.now.addingTimeInterval(58 * 60))) == "还有 58 分钟")
        let later = Countdown(from: TestClock.now, to: TestClock.now.addingTimeInterval(5 * 3600 + 12 * 60))
        #expect(en.countdown(later) == "in 5 h 12 min")
        #expect(zh.countdown(later) == "还有 5 小时 12 分钟")
        #expect(en.countdown(Countdown(from: TestClock.now, to: TestClock.now.addingTimeInterval(3 * 3600))) == "in 3 h")
    }

    @Test("the summary is one sentence per count")
    func summary() {
        let deadlines = [
            due("A", day: 0, hour: 23), due("B", day: 1, hour: 9), due("C", day: 3, hour: 12),
            due("Lecture", day: 1, hour: 10, kind: .classEvent),
        ]
        let plan = StoredStudyPlan(
            id: 1, createdAt: TestClock.at(-2, 10),
            plan: StudyPlan(
                horizonStart: "2026-09-24", horizonEnd: "2026-10-07",
                items: ["one", "two"].map {
                    StudyPlanItem(date: "2026-09-25", courseId: nil, title: $0, description: nil, materialIds: [], minutes: nil, done: false)
                },
                notes: nil
            ),
            origin: .aiApp
        )
        let digest = ThisWeekDigest(deadlines: deadlines, plan: plan, now: TestClock.now, calendar: TestClock.calendar)
        #expect(en.summary(digest) == "3 deadlines in the next 7 days · 2 plan tasks today")
        #expect(zh.summary(digest) == "未来 7 天有 3 个截止日期 · 今天有 2 项学习任务")
        #expect(en.next7Detail(digest) == "3 deadlines · 1 class")
        #expect(zh.next7Detail(digest) == "3 个截止日期 · 1 节课")

        let quiet = ThisWeekDigest(deadlines: [], plan: nil, now: TestClock.now, calendar: TestClock.calendar)
        #expect(en.summary(quiet) == "Nothing due in the next 7 days")
        #expect(en.next7Detail(quiet) == nil)
        let one = ThisWeekDigest(deadlines: [due("A", day: 2, hour: 9)], plan: nil, now: TestClock.now, calendar: TestClock.calendar)
        #expect(en.summary(one) == "1 deadline in the next 7 days")
        // Deadlines that failed to load (S14) are not "nothing due".
        #expect(en.summary(quiet, deadlinesLoaded: false) == nil)
        #expect(en.summary(digest, deadlinesLoaded: false) == "2 plan tasks today")
    }

    @Test("day headings: a relative word and the date, else the weekday and the date")
    func days() {
        let today = TestClock.calendar.startOfDay(for: TestClock.now)
        let sunday = TestClock.at(2, 0)
        #expect(en.dayTitle(today) == "Today")
        #expect(en.dayDate(today) == "Fri, Sep 25")
        #expect(en.dayTitle(TestClock.at(1, 0)) == "Tomorrow")
        #expect(en.dayTitle(sunday) == "Sunday")
        #expect(en.dayDate(sunday) == "Sep 27")
        #expect(zh.dayTitle(today) == "今天")
        #expect(zh.dayDate(today) == "9月25日 周五")
        #expect(zh.dayTitle(sunday) == "星期日")
        #expect(zh.dayDate(sunday) == "9月27日")
    }

    @Test("\"Next:\" lines say when briefly")
    func next() {
        #expect(en.next(due("Quiz 3", day: 1, hour: 9)) == "Next: Quiz 3 · Tomorrow 9:00 AM")
        #expect(zh.next(due("Quiz 3", day: 1, hour: 9)) == "下一项：Quiz 3 · 明天 9:00")
        #expect(en.next(due("Quiz 4", day: 2, hour: 9)) == "Next: Quiz 4 · Sun 9:00 AM")
        #expect(en.shortWhen(TestClock.at(0, 23, 59)) == "Today 11:59 PM")
        // Beyond the week: the date, not the weekday.
        #expect(en.shortWhen(TestClock.at(12, 18)).hasPrefix("Oct 7"))
        #expect(zh.shortWhen(TestClock.at(12, 18)).hasPrefix("10月7日"))
    }

    @Test("VoiceOver hears whole deadline sentences, with the countdown for Next up")
    func spokenDeadlines() {
        let tonight = due("Problem Set 2", day: 0, hour: 23, minute: 59)
        let later = PlainText(ThisWeekText(l10n: en.l10n, calendar: en.calendar, now: TestClock.at(0, 23, 1)))
        let spoken = later.spokenDeadline(tonight, withCountdown: true)
        #expect(spoken.hasPrefix("Problem Set 2. DEMO205, Assignment. Due Friday, September 25"))
        #expect(spoken.hasSuffix(", in 58 minutes."))
        #expect(en.spokenDeadline(tonight).hasSuffix("11:59 PM."))
        #expect(en.spokenDeadline(due("Quiz", day: 1, hour: 9, course: nil)).hasPrefix("Quiz. Assignment. Due Saturday"))
        let lecture = en.spokenDeadline(due("Lecture 9", day: 1, hour: 10, kind: .classEvent))
        #expect(lecture.hasPrefix("Lecture 9. DEMO205, Class. Saturday"))
        #expect(!lecture.contains("Due"))
        #expect(zh.spokenDeadline(tonight).hasPrefix("Problem Set 2。DEMO205，作业。9月25日"))
    }

    @Test("plan meta, minutes and the spoken plan item")
    func plan() {
        let stored = StoredStudyPlan(
            id: 1, createdAt: TestClock.at(-2, 10),
            plan: StudyPlan(horizonStart: "2026-09-24", horizonEnd: "2026-10-07", items: [], notes: nil),
            origin: .aiApp
        )
        #expect(en.planMeta(stored) == "Made by your AI app 2 days ago · covers Sep 24 – Oct 7")
        #expect(zh.planMeta(stored) == "2天前由你的 AI 应用生成 · 覆盖 9月24日 至 10月7日")
        #expect(en.planMinutes(30) == "30 min")
        #expect(zh.planMinutes(30) == "30 分钟")
        // Calendar days: made Wednesday 20:15, read Friday 10:00 → "2 days ago".
        let wednesdayEvening = StoredStudyPlan(
            id: 2, createdAt: TestClock.at(-2, 20, 15),
            plan: StudyPlan(horizonStart: "2026-09-24", horizonEnd: "2026-10-07", items: [], notes: nil),
            origin: .aiApp
        )
        #expect(en.planMeta(wednesdayEvening).hasPrefix("Made by your AI app 2 days ago"))
        let thisMorning = StoredStudyPlan(
            id: 3, createdAt: TestClock.at(0, 7),
            plan: StudyPlan(horizonStart: "2026-09-24", horizonEnd: "2026-10-07", items: [], notes: nil),
            origin: .aiApp
        )
        #expect(en.planMeta(thisMorning).hasPrefix("Made by your AI app 3 hours ago"))

        func item(minutes: UInt32?, done: Bool) -> StudyPlanItem {
            StudyPlanItem(date: "2026-09-25", courseId: nil, title: "Skim Week 4 slides", description: nil, materialIds: [], minutes: minutes, done: done)
        }
        #expect(en.spokenPlanItem(item(minutes: 30, done: false), course: "DEMO101") == "Skim Week 4 slides, DEMO101, 30 minutes, Not done yet")
        #expect(en.spokenPlanItem(item(minutes: 1, done: true), course: nil) == "Skim Week 4 slides, 1 minute, Done")
        #expect(en.spokenPlanItem(item(minutes: nil, done: false), course: "DEMO101") == "Skim Week 4 slides, DEMO101, Not done yet")
        #expect(zh.spokenPlanItem(item(minutes: nil, done: true), course: nil) == "Skim Week 4 slides，已完成")
    }

    @Test("Contents: week column, policy line (never colour, words only) and the spoken row")
    func contents() {
        #expect(en.week(.week(4)) == "Week 4")
        #expect(zh.week(.week(4)) == "第 4 周")
        #expect(en.week(.unknown) == "—")
        #expect(en.week(.outsideTerm) == "—")
        #expect(en.spokenWeek(.outsideTerm) == "Outside term")

        #expect(en.policyLine(thisWeekSummary(code: "DEMO101")) == "Learning aid only · 12 of 14 readable")
        #expect(zh.policyLine(thisWeekSummary(code: "DEMO101")) == "仅限辅助学习 · 14 份中 12 份可读取")
        let noAI = thisWeekSummary(code: "DEMO310", policy: .prohibited, materials: .withheldByPolicy)
        #expect(en.policyLine(noAI) == "No AI · materials not shared")
        #expect(zh.policyLine(noAI) == "禁止使用 AI · 资料不共享")
        #expect(en.policyLine(thisWeekSummary(code: "X", policy: .unknown, counts: (0, 0))) == "AI policy not set · No materials yet")
        #expect(en.policyLine(thisWeekSummary(code: "X", materials: .turnedOff)) == "Learning aid only · AI access off")

        let next = due("Quiz 3", day: 1, hour: 9)
        let row = en.spokenContentsRow(thisWeekSummary(code: "DEMO101", name: "Intro to Demo Studies", next: next))
        #expect(row.hasPrefix("DEMO101, Intro to Demo Studies. Week 4. Next: Quiz 3 · Saturday, September 26"))
        #expect(row.hasSuffix("AI policy: Learning aid only. 12 of 14 materials readable by your AI app."))
        #expect(en.spokenContentsRow(noAI).contains("AI policy: No AI. Materials not shared (No AI course)."))
    }
}

/// ThisWeekText with ICU's narrow and no-break spaces ("11:59\u{202F}PM") read as plain spaces,
/// so expectations are written as they read.
struct PlainText {
    let text: ThisWeekText
    init(_ text: ThisWeekText) { self.text = text }

    var l10n: L10n { text.l10n }
    var calendar: Calendar { text.calendar }

    static func plain(_ value: String) -> String {
        value.replacingOccurrences(of: "\u{202F}", with: " ").replacingOccurrences(of: "\u{00A0}", with: " ")
            .replacingOccurrences(of: "\u{2060}", with: "")
    }

    var todayTitle: String { Self.plain(text.todayTitle) }
    func due(_ date: Date) -> String { Self.plain(text.due(date)) }
    func countdown(_ countdown: Countdown) -> String { Self.plain(text.countdown(countdown)) }
    func summary(_ digest: ThisWeekDigest, deadlinesLoaded: Bool = true) -> String? {
        text.summary(digest, deadlinesLoaded: deadlinesLoaded).map(Self.plain)
    }
    func next7Detail(_ digest: ThisWeekDigest) -> String? { text.next7Detail(digest).map(Self.plain) }
    func dayTitle(_ day: Date) -> String { Self.plain(text.dayTitle(day)) }
    func dayDate(_ day: Date) -> String { Self.plain(text.dayDate(day)) }
    func next(_ deadline: Deadline) -> String? { text.next(deadline).map(Self.plain) }
    func shortWhen(_ date: Date) -> String { Self.plain(text.shortWhen(date)) }
    func spokenDeadline(_ deadline: Deadline, withCountdown: Bool = false) -> String {
        Self.plain(text.spokenDeadline(deadline, withCountdown: withCountdown))
    }
    func planMeta(_ stored: StoredStudyPlan) -> String { Self.plain(text.planMeta(stored)) }
    func planMinutes(_ minutes: UInt32) -> String { Self.plain(text.planMinutes(minutes)) }
    func spokenPlanItem(_ item: StudyPlanItem, course: String?) -> String {
        Self.plain(text.spokenPlanItem(item, course: course))
    }
    func week(_ state: CourseWeekState) -> String { Self.plain(text.week(state)) }
    func spokenWeek(_ state: CourseWeekState) -> String { Self.plain(text.spokenWeek(state)) }
    func policyLine(_ summary: CourseSummary) -> String { Self.plain(text.policyLine(summary)) }
    func spokenContentsRow(_ summary: CourseSummary) -> String { Self.plain(text.spokenContentsRow(summary)) }
}
