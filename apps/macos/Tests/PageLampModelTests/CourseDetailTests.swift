// Course detail (spec §3.2, M1 read-only): the week line and its lamp, material statuses
// (rule 8), deadlines, notes, the tinted action, dates in both languages, and loading over the
// mock. Nothing here touches the real data folder, the keychain or the network.

import Foundation
import Observation
@testable import PageLamp
import PageLampKit
import PageLampModel
import Synchronization
import Testing

@MainActor
private func l10n(_ language: AppLanguage) -> L10n {
    L10n(
        locale: Locale(identifier: language == .simplifiedChinese ? "zh-Hans-CN" : "en-US"),
        table: .app
    )
}

private func material(_ status: TextStatus, blocked: DownloadBlock? = nil, url: String? = nil) -> MaterialView {
    MaterialView(
        id: "m", courseId: "c", title: "Week 4 slides", kind: .file, moduleId: nil, moduleName: nil,
        weekHint: 4, publishedAt: nil, url: url, textStatus: status, textError: nil,
        downloadBlocked: blocked, chunkCount: 0
    )
}

private func timeline(
    week: UInt32?, confidence: Confidence = .high, outside: Bool = false, phase: CoursePhase? = nil
) -> CourseTimeline {
    testTimeline(week: week, confidence: confidence, outsideTerm: outside, phase: phase)
}

@Suite("Course week line") @MainActor
struct CourseWeekLineTests {
    @Test("the current week is lit and says This week")
    func current() {
        let line = CourseWeekLine(section: .week, selectedWeek: nil, currentWeek: 4, outsideTerm: false)
        #expect(line.week == 4)
        #expect(line.tag == .thisWeek)
        #expect(line.lit)
        #expect(line.title(l10n(.english)) == "Week 4")
        #expect(line.spoken(l10n(.english)) == "Week 4, This week")
        #expect(line.spoken(l10n(.simplifiedChinese)) == "第 4 周，本周")
    }

    @Test("stepping away puts the lamp out and names the distance", arguments: [
        (UInt32(5), CourseWeekLine.Tag.nextWeek, "Next week", "下周"),
        (UInt32(3), .lastWeek, "Last week", "上周"),
        (UInt32(6), .inWeeks(2), "In 2 weeks", "2 周后"),
        (UInt32(1), .weeksAgo(3), "3 weeks ago", "3 周前"),
    ])
    func away(selected: UInt32, tag: CourseWeekLine.Tag, english: String, chinese: String) {
        let line = CourseWeekLine(section: .week, selectedWeek: selected, currentWeek: 4, outsideTerm: false)
        #expect(line.week == selected)
        #expect(line.tag == tag)
        #expect(!line.lit)
        #expect(line.tagText(l10n(.english)) == english)
        #expect(line.tagText(l10n(.simplifiedChinese)) == chinese)
    }

    @Test("Deadlines and Timeline always show the current week, and VoiceOver can't step it there")
    func otherSections() {
        for section in [CourseSection.deadlines, .timeline] {
            let line = CourseWeekLine(section: section, selectedWeek: 7, currentWeek: 4, outsideTerm: false)
            #expect(line.week == 4)
            #expect(line.lit)
            // Stepping would switch sections unasked: the line is adjustable only in This Week.
            #expect(!line.isAdjustable)
        }
        #expect(CourseWeekLine(section: .week, selectedWeek: 7, currentWeek: 4, outsideTerm: false).isAdjustable)
    }

    @Test("an unknown week has no pool and no tag (S11)")
    func unknown() {
        let line = CourseWeekLine(section: .week, selectedWeek: nil, currentWeek: nil, outsideTerm: false)
        #expect(line.week == nil)
        #expect(line.tag == nil)
        #expect(!line.lit)
        #expect(line.title(l10n(.english)) == "Week unknown")
        #expect(line.title(l10n(.simplifiedChinese)) == "周次未知")
    }

    @Test("outside the term the lamp stays out")
    func outsideTerm() {
        let withWeek = CourseWeekLine(section: .week, selectedWeek: nil, currentWeek: 18, outsideTerm: true)
        #expect(withWeek.tag == .outsideTerm)
        #expect(!withWeek.lit)
        #expect(withWeek.tagText(l10n(.english)) == "Outside term")
        let withoutWeek = CourseWeekLine(section: .week, selectedWeek: nil, currentWeek: nil, outsideTerm: true)
        #expect(withoutWeek.title(l10n(.english)) == "Outside term")
        #expect(!withoutWeek.lit)
    }
}

@Suite("Section picker's starting point") @MainActor
struct CoursePickerSectionTests {
    @Test("a new page's thumb starts at This Week until the picker has shown another section")
    func defaults() {
        let ui = CourseUIState()
        #expect(ui.pickerSection == .week)
        ui.section = .deadlines
        // Only the picker moves it (it follows `section` once it shows it).
        #expect(ui.pickerSection == .week)
    }

    @Test("stepping weeks and Current Week change the section, never where the thumb starts")
    func untouchedByWeekCommands() {
        let ui = CourseUIState()
        ui.update(availableWeeks: [1, 2, 3], currentWeek: 3)
        ui.pickerSection = .timeline
        ui.section = .timeline
        ui.step(by: -1)
        #expect(ui.section == .week)
        #expect(ui.pickerSection == .timeline)
        ui.section = .deadlines
        ui.showCurrentWeek()
        #expect(ui.section == .week)
        #expect(ui.pickerSection == .timeline)
    }
}

@Suite("Course material status") @MainActor
struct CourseMaterialStatusTests {
    @Test("readable courses show the extraction status")
    func readable() {
        #expect(CourseMaterialStatus(material: material(.ok), aiMaterials: .readable) == .readable)
        #expect(CourseMaterialStatus(material: material(.pending), aiMaterials: .readable) == .waiting)
        #expect(CourseMaterialStatus(material: material(.unsupported), aiMaterials: .readable) == .unreadable)
        #expect(CourseMaterialStatus(material: material(.error), aiMaterials: .readable) == .extractFailed)
        let blocked = CourseMaterialStatus(material: material(.notDownloaded, blocked: .tooLarge), aiMaterials: .readable)
        #expect(blocked == .notDownloaded(.tooLarge))
        #expect(blocked.blockKey == "common.downloadBlock.too_large")
        #expect(CourseMaterialStatus(material: material(.notDownloaded), aiMaterials: .readable).blockKey == nil)
    }

    @Test("No AI and turned-off courses say so on every row, whatever the extraction (rule 8)")
    func withheld() {
        for status in TextStatus.allCases {
            let noAI = CourseMaterialStatus(material: material(status), aiMaterials: .withheldByPolicy)
            #expect(noAI == .notShared)
            #expect(noAI.symbol == "hand.raised")
            #expect(CourseMaterialStatus(material: material(status), aiMaterials: .turnedOff) == .offForAI)
        }
    }

    @Test("only the glyph carries a tone; every key exists in both languages")
    func keys() {
        let all: [CourseMaterialStatus] = [
            .readable, .waiting, .notDownloaded(nil), .notDownloaded(.locked), .unreadable, .extractFailed,
            .notShared, .offForAI,
        ]
        for status in all {
            #expect(StringTable.app.keys.contains(status.shortKey))
            #expect(StringTable.app.keys.contains(status.spokenKey))
            if let block = status.blockKey { #expect(StringTable.app.keys.contains(block)) }
        }
        #expect(CourseMaterialStatus.readable.tone == .success)
        #expect(CourseMaterialStatus.extractFailed.tone == .warning)
        #expect(CourseMaterialStatus.notShared.tone == .neutral)
        #expect(l10n(.english)(CourseMaterialStatus.offForAI.shortKey) == "Off for AI")
        #expect(l10n(.simplifiedChinese)(CourseMaterialStatus.notShared.shortKey) == "不共享")
    }

    @Test("links: files open in their app, web addresses in the browser, nothing else")
    func links() {
        #expect(CourseLink("file:///Users/demo/Courses/DEMO101/week4.pdf") == .file(URL(filePath: "/Users/demo/Courses/DEMO101/week4.pdf")))
        #expect(CourseLink("https://canvas.demo.test/files/1") == .web(URL(string: "https://canvas.demo.test/files/1")!))
        #expect(CourseLink("javascript:alert(1)") == nil)
        #expect(CourseLink("") == nil)
        #expect(CourseLink(nil) == nil)
    }
}

@Suite("Course deadlines and notes") @MainActor
struct CourseDeadlinesTests {
    @Test("class meetings are left out; coming up soonest first, past most recent first")
    func split() {
        let deadlines = CourseDeadlines([
            deadline("Midterm", .exam, day: 12, hour: 18),
            deadline("Lecture 9", .classEvent, day: 1, hour: 10),
            deadline("Reading response 3", .assignmentDue, day: -3, hour: 23),
            deadline("Problem Set 2", .assignmentDue, day: 2, hour: 23),
            deadline("Reading response 2", .assignmentDue, day: -6, hour: 23),
            deadline("Earlier today", .quizDue, day: 0, hour: 9),
        ], now: TestClock.now)
        #expect(deadlines.upcoming.map(\.event.title) == ["Problem Set 2", "Midterm"])
        #expect(deadlines.past.map(\.event.title) == ["Earlier today", "Reading response 3", "Reading response 2"])
    }

    @Test("week notes by code; the empty week is left to the empty state")
    func weekNotes() {
        func week(_ kind: WeekNoteKind?, note: String?) -> WeekMaterials {
            WeekMaterials(
                course: Course(
                    id: "c", sourceId: "s", externalId: "c", code: "C", name: "C", termStart: nil, termEnd: nil,
                    termSource: .none, url: nil, aiPolicy: .unknown, aiPolicyNote: nil, aiAccess: true,
                    materialSharing: .unanswered, hidden: false, enrollmentActive: true, updatedAt: TestClock.now
                ),
                aiMaterials: .readable, week: nil, requestedWeek: nil, timeline: timeline(week: nil),
                modules: [], materials: [], availableWeeks: [], note: note, noteKind: kind
            )
        }
        #expect(CourseDetailModel.WeekNote(week(.currentWeekUnknown, note: "x"))
            == .known(key: "course.week.note.current_week_unknown", offersTermDates: true))
        #expect(CourseDetailModel.WeekNote(week(.outsideTerm, note: "x"))
            == .known(key: "course.week.note.outside_term", offersTermDates: true))
        #expect(CourseDetailModel.WeekNote(week(.noMaterialsThisWeek, note: "x")) == nil)
        // The exam period and a break have known dates: their notes offer no Set Term Dates… (M0.10).
        #expect(CourseDetailModel.WeekNote(week(.examPeriod, note: "x"))
            == .known(key: "course.week.note.exam_period", offersTermDates: false))
        #expect(CourseDetailModel.WeekNote(week(.break, note: "x"))
            == .known(key: "course.week.note.break", offersTermDates: false))
        #expect(CourseDetailModel.WeekNote(week(nil, note: "Something new")) == .backend("Something new"))
        #expect(CourseDetailModel.WeekNote(week(nil, note: nil)) == nil)
    }

    @Test("the source problem and the page's one tinted action (spec §3.0)")
    func arbiter() {
        func source(_ kind: SourceErrorKind?) -> SourceRecord {
            SourceRecord(id: "canvas:x", kind: .canvas, label: "Canvas", config: "{}", lastSyncedAt: nil, lastError: nil, lastErrorKind: kind)
        }
        let expired = source(.authExpiredOrRevoked)
        #expect(SourceProblem(source: expired) == .expired(.replaceToken))
        #expect(SourceProblem(source: source(.network)) == .failed(.network))
        #expect(SourceProblem(source: source(.other)) == .failed(.other))
        #expect(SourceProblem(source: source(nil)) == nil)

        // The fix outranks Set Term Dates…
        let both = CourseDetailModel.primaryActionCandidates(source: expired, timeline: timeline(week: nil), canReplaceSecrets: true)
        #expect(PrimaryActionArbiter.winner(both) == .fixSource("canvas:x"))
        // Without the Replace sheet (M1) the fix is no candidate: Set Term Dates… wins instead.
        let preview = CourseDetailModel.primaryActionCandidates(source: expired, timeline: timeline(week: nil), canReplaceSecrets: false)
        #expect(!preview.contains(.fixSource("canvas:x")))
        #expect(PrimaryActionArbiter.winner(preview) == .setTermDates)
        // A failure without a fix (can't connect) is no candidate.
        #expect(CourseDetailModel.primaryActionCandidates(source: source(.network), timeline: timeline(week: 4)).isEmpty)
        // Only an unknown phase tints Set Term Dates… (M0.10), with or without a week.
        for unknown in [timeline(week: nil), timeline(week: 4, confidence: .low, phase: .unknown)] {
            #expect(PrimaryActionArbiter.winner(CourseDetailModel.primaryActionCandidates(source: nil, timeline: unknown)) == .setTermDates)
        }
        // No current week is not enough: the exam period, a break, before the start and after the
        // end have none, and their dates are known.
        for known in [
            timeline(week: nil, phase: .examPeriod), timeline(week: nil, phase: .break),
            timeline(week: nil, outside: true, phase: .notStarted), timeline(week: nil, outside: true),
        ] {
            #expect(CourseDetailModel.primaryActionCandidates(source: nil, timeline: known).isEmpty)
        }
        // A confident current week and a healthy source: nothing is tinted.
        #expect(CourseDetailModel.primaryActionCandidates(source: nil, timeline: timeline(week: 4, confidence: .medium)).isEmpty)
        #expect(CourseDetailModel.primaryActionCandidates(source: source(.other), timeline: timeline(week: 4)).isEmpty)
    }
}

@Suite("Course dates") @MainActor
struct CourseDatesTests {
    @Test("deadline days: relative near today, else weekday and date, in each language")
    func days() {
        let calendar = TestClock.calendar
        let en = l10n(.english)
        let zh = l10n(.simplifiedChinese)
        #expect(CourseDates.day(TestClock.at(0, 23, 59), now: TestClock.now, calendar: calendar, l10n: en) == "Today")
        #expect(CourseDates.day(TestClock.at(1, 9), now: TestClock.now, calendar: calendar, l10n: en) == "Tomorrow")
        #expect(CourseDates.day(TestClock.at(-1, 9), now: TestClock.now, calendar: calendar, l10n: zh) == "昨天")
        #expect(CourseDates.day(TestClock.at(2, 23, 59), now: TestClock.now, calendar: calendar, l10n: en) == "Sun, Sep 27")
        #expect(CourseDates.day(TestClock.at(12, 18), now: TestClock.now, calendar: calendar, l10n: zh).contains("10月7日"))
    }

    @Test("times follow the locale's clock")
    func times() {
        let calendar = TestClock.calendar
        let english = CourseDates.time(TestClock.at(0, 23, 59), calendar: calendar, l10n: l10n(.english))
        #expect(english.contains("11:59") && english.contains("PM"))
        #expect(CourseDates.time(TestClock.at(0, 23, 59), calendar: calendar, l10n: l10n(.simplifiedChinese)) == "23:59")
    }

    @Test("publication days add the year only when it isn't this year; term dates parse ISO days")
    func published() {
        let calendar = TestClock.calendar
        let en = l10n(.english)
        #expect(CourseDates.published(TestClock.at(-3, 9), now: TestClock.now, calendar: calendar, l10n: en) == "Sep 22")
        let lastYear = calendar.date(byAdding: .year, value: -1, to: TestClock.now) ?? TestClock.now
        #expect(CourseDates.published(lastYear, now: TestClock.now, calendar: calendar, l10n: en).contains("2025"))
        #expect(CourseDates.term("2026-09-02", calendar: calendar, l10n: en) == "Sep 2, 2026")
        #expect(CourseDates.term("2026-09-02", calendar: calendar, l10n: l10n(.simplifiedChinese)) == "2026年9月2日")
        #expect(CourseDates.term("not a date", calendar: calendar, l10n: en) == nil)
        #expect(CourseDates.term(nil, calendar: calendar, l10n: en) == nil)
    }
}

@Suite("Course detail model") @MainActor
struct CourseDetailModelTests {
    private func demo101(_ model: AppModel) throws -> CourseSummary {
        try #require(model.courses.first { $0.course.code == "DEMO101" })
    }

    @Test("loads the overview, the current week and the deadlines")
    func loadAll() async throws {
        let (model, _) = makeModel(scenario: .demo)
        await model.refresh()
        let summary = try demo101(model)
        let detail = CourseDetailModel(courseId: summary.course.id)
        #expect(detail.week == .loading)
        await detail.loadAll(using: model)

        let week = try #require(detail.week.value)
        #expect(week.week == 4)
        #expect(week.materials.count == 6)
        #expect(!detail.isLoadingWeek)
        #expect(detail.overview.value?.recentAnnouncements.map(\.title)
            == ["Office hours move to Thursday this week", "Problem Set 2 is posted"])
        let deadlines = try #require(detail.deadlines.value)
        #expect(deadlines.upcoming.map(\.event.title) == ["Problem Set 2", "Quiz 3 — Sampling", "Midterm test"])
        #expect(deadlines.past.map(\.event.title) == ["Reading response 3"])
        // The page's knowledge of the course's weeks reaches the toolbar and Go menu.
        #expect(model.ui(for: summary.course.id).availableWeeks == [1, 2, 3, 4])
    }

    @Test("stepping to another week loads it")
    func stepWeek() async throws {
        let (model, _) = makeModel(scenario: .demo)
        await model.refresh()
        let summary = try demo101(model)
        let detail = CourseDetailModel(courseId: summary.course.id)
        await detail.loadWeek(using: model)
        let ui = model.ui(for: summary.course.id)
        ui.step(by: -1)
        ui.step(by: -1)
        #expect(ui.selectedWeek == 2)
        await detail.loadWeek(using: model)
        #expect(detail.week.value?.week == 2)
        #expect(detail.week.value?.materials.map(\.title) == [
            "Week 2 slides — Placeholder Data", "Reading: Chapter 2, Making Up Numbers Responsibly",
        ])
    }

    @Test("reloading the week on screen changes nothing: going back to a course renders it once")
    func quietReload() async throws {
        let (model, _) = makeModel(scenario: .demo)
        await model.refresh()
        let summary = try demo101(model)
        let detail = CourseDetailModel(courseId: summary.course.id)
        await detail.loadAll(using: model)
        let ui = model.ui(for: summary.course.id)
        let changed = Mutex<[String]>([])
        func track(_ name: String, _ read: @escaping () -> Void) {
            withObservationTracking(read) { changed.withLock { $0.append(name) } }
        }
        track("week") { _ = detail.week }
        track("isLoadingWeek") { _ = detail.isLoadingWeek }
        track("overview") { _ = detail.overview }
        track("deadlines") { _ = detail.deadlines }
        track("weeks") { _ = ui.availableWeeks; _ = ui.currentWeek }
        await detail.loadAll(using: model)
        #expect(changed.withLock { $0 } == [])

        // Stepping to another week still dims the week while it loads (the trackers are armed).
        ui.step(by: -1)
        await detail.loadWeek(using: model)
        #expect(changed.withLock { $0 }.contains("isLoadingWeek"))
        #expect(changed.withLock { $0 }.contains("week"))
        #expect(!detail.isLoadingWeek)
    }

    @Test("a failing part fails alone (S14)")
    func sectionErrors() async throws {
        let (model, mock) = makeModel(scenario: .demo)
        let failing = FixtureService(base: mock, failing: [.week, .courseDeadlines])
        let failingModel = AppModel(
            dataMode: .mock(.demo), strings: .app, settings: InMemorySettingsStore(language: .english),
            timing: AppModel.Timing(finishedCapsule: .zero, failedCapsule: .zero, mock: .instant),
            calendar: TestClock.calendar, clock: { TestClock.now }, service: failing
        )
        _ = model
        await failingModel.refresh()
        let summary = try demo101(failingModel)
        let detail = CourseDetailModel(courseId: summary.course.id)
        await detail.loadAll(using: failingModel)
        #expect(detail.week.failure?.kind == .internal)
        #expect(detail.deadlines.failure?.kind == .internal)
        #expect(detail.overview.value != nil)
    }

    @Test("AI Policy… and Set Term Dates… open the inspector at their section")
    func inspector() {
        let (model, _) = makeModel(scenario: .demo)
        let detail = CourseDetailModel(courseId: "c")
        #expect(!model.inspectorShown)
        detail.showInspector(.termDates, in: model)
        #expect(model.inspectorShown)
        #expect(detail.inspectorRequest?.section == .termDates)
        let first = detail.inspectorRequest
        detail.showInspector(.termDates, in: model)
        // Asking again scrolls again.
        #expect(detail.inspectorRequest != first)
    }

    @Test("the AI status line: policy, materials, past and hidden, never colour words")
    func statusLine() async throws {
        let (model, _) = makeModel(scenario: .demo)
        await model.refresh()
        let en = l10n(.english)
        func items(_ code: String) throws -> [String] {
            let summary = try #require(model.courses.first { $0.course.code == code })
            return CourseAIStatusLine.items(summary, en).map(\.text)
        }
        #expect(try items("DEMO101") == ["Learning aid only", "10 of 13 materials readable by your AI app"])
        #expect(try items("DEMO205") == ["AI policy not set · Set…", "3 of 6 materials readable by your AI app"])
        #expect(try items("DEMO310") == ["No AI", "Materials not shared (No AI course)"])
        #expect(try items("DEMO099") == [
            "AI policy not set · Set…", "1 of 1 material readable by your AI app", "Past course", "Hidden",
        ])
        let zh = try #require(model.courses.first { $0.course.code == "DEMO101" })
        #expect(CourseAIStatusLine.items(zh, l10n(.simplifiedChinese)).map(\.text)
            == ["仅限辅助学习", "共 13 份资料，AI 应用可读取 10 份"])
    }
}
