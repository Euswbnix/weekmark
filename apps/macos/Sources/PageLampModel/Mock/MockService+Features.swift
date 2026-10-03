// The mock's M1–M3 calls (course calendars, removal, reminders, generations; AI setup is in
// MockService+Ai, study plans in MockService+Plan, weekly explanations in MockService+Explain,
// weekly notes in MockService+Note). Settings, removing and restoring courses and reminders behave
// like the facade on the mock's own data. Reading course calendars still ends `blocked` like the
// facade without a model until its screen comes to the Mac, and Codex isn't offered (the ChatGPT
// plan's build switch).

import Foundation
import PageLampKit

/// The M1–M3 state the mock keeps.
struct MockFeatures: Sendable {
    /// Settings ▸ AI: providers, acknowledgements, the model per feature, the budget, usage.
    var ai = MockAi()
    var sharing: [String: MaterialSharing] = [:]
    var outputLanguage: OutputLanguage = .ui
    var prepareNoteOnMonday = false
    /// The day Monday's note was last tried, "YYYY-MM-DD" (the facade's `ai.weekly_note_tried_on`).
    var noteTriedOn: String?
    var weeklyCap: UInt32?
    var codexSource: CodexSource = .managed
    var reminderSettings = ReminderSettings(
        deadlineSoon: true, weeklyDigest: true, digestDay: .monday, digestTime: "09:00",
        planToday: false, planTodayTime: "08:00", runInBackground: false
    )
    var shownReminders: Set<String> = []
    /// Removed courses, with the course to put back on restore.
    var removed: [MockRemoval] = []
    /// Model runs in flight, the ones asked to stop, the study plan drafts, the explanations, the
    /// courses already asked about sharing their materials, and the weekly notes.
    var runs = MockRuns()
}

struct MockRuns: Sendable {
    var running: Set<String> = []
    var cancelled: Set<String> = []
    var planDrafts: [String: GeneratedStudyPlan] = [:]
    var acceptedDrafts: Set<String> = []
    /// Kept explanations, newest first (the facade's `created_at DESC, rowid DESC`).
    var explanations: [WeeklyExplanation] = []
    /// Courses whose reminder about sharing materials was shown (the facade's `reminders_shown`).
    var sharingReminded: Set<String> = []
    /// Kept weekly notes, newest first.
    var notes: [WeeklyNote] = []
}

struct MockRemoval: Sendable {
    var record: RemovedCourse
    var course: MockCourse
}

extension MockService {
    static let noModel = PageLampFailure(kind: .blocked, message: "Set up a model first: the preview's mock data has none.")

    // MARK: - Course answers and generated content (the rest of AI setup: MockService+Ai)

    public func setCourseMaterialSharing(course reference: String, answer: MaterialSharing) async throws(PageLampFailure) {
        await respond("setCourseMaterialSharing")
        db.features.sharing[db.courses[try courseIndex(reference)].course.id] = answer
    }

    /// Drops the course's explanations and the weekly notes that cover it (all of them when nil);
    /// the answers about sharing stay.
    public func deleteGenerated(course reference: String?) async throws(PageLampFailure) -> UInt32 {
        await respond("deleteGenerated")
        let id: String?
        if let reference {
            id = db.courses[try courseIndex(reference)].course.id
        } else {
            id = nil
        }
        let before = db.features.runs.explanations.count + db.features.runs.notes.count
        db.features.runs.explanations.removeAll { id == nil || $0.courseId == id }
        db.features.runs.notes.removeAll { note in
            id == nil || note.meta.context.courses.contains { $0.courseId == id }
        }
        return UInt32(before - db.features.runs.explanations.count - db.features.runs.notes.count)
    }

    // MARK: - The ChatGPT plan through Codex (not offered: every way in refuses, clean-up works)

    /// The facade's refusal while the build doesn't offer the ChatGPT plan.
    static let chatgptPlanNotOffered = PageLampFailure(
        kind: .blocked,
        message: "The ChatGPT plan isn't available in this version of PageLamp. Use an API key or a model on this computer.",
        blocked: .backendDisabledInThisBuild
    )

    public func codexStatus() async throws(PageLampFailure) -> CodexStatus {
        await respond("codexStatus")
        return codex()
    }

    public func installCodex(installId: String, observer: any CodexInstallObserver) async throws(PageLampFailure) -> CodexStatus {
        await respond("installCodex")
        throw Self.chatgptPlanNotOffered
    }

    public func cancelCodexInstall(installId: String) async throws(PageLampFailure) {
        await respond("cancelCodexInstall")
    }

    public func removeCodex() async throws(PageLampFailure) {
        await respond("removeCodex")
    }

    public func codexLogin(method: CodexLoginMethod, observer: any CodexLoginObserver) async throws(PageLampFailure) -> CodexStatus {
        await respond("codexLogin")
        throw Self.chatgptPlanNotOffered
    }

    public func cancelCodexLogin() async throws(PageLampFailure) {
        await respond("cancelCodexLogin")
    }

    public func codexLogout() async throws(PageLampFailure) -> CodexStatus {
        await respond("codexLogout")
        return codex()
    }

    public func setModeAWeeklyCap(runs: UInt32?) async throws(PageLampFailure) {
        await respond("setModeAWeeklyCap")
        throw Self.chatgptPlanNotOffered
    }

    public func setCodexSource(source: CodexSource) async throws(PageLampFailure) -> CodexStatus {
        await respond("setCodexSource")
        throw Self.chatgptPlanNotOffered
    }

    private func codex() -> CodexStatus {
        CodexStatus(
            runtime: CodexRuntime(
                state: .notInstalled, source: db.features.codexSource, installedVersion: nil,
                pinnedVersion: "0.0.0-mock", downloadBytes: 0, untestedPlatform: false
            ),
            outdatedAction: .none,
            login: CodexLogin(state: .signedOut, planType: nil),
            execAvailable: nil,
            weeklyCap: db.features.weeklyCap,
            runsThisWeek: 0,
            systemCodex: nil
        )
    }

    // MARK: - The answers' language (explanations and the weekly note; the mock writes in English)

    public func aiOutputLanguage() async throws(PageLampFailure) -> OutputLanguage {
        await respond("aiOutputLanguage")
        return db.features.outputLanguage
    }

    public func setAiOutputLanguage(language: OutputLanguage) async throws(PageLampFailure) {
        await respond("setAiOutputLanguage")
        db.features.outputLanguage = language
    }

    public func setStudyPlanItemDone(planId: Int64, itemIndex: UInt32, done: Bool) async throws(PageLampFailure) -> StoredStudyPlan {
        await respond("setStudyPlanItemDone")
        guard let stored = db.studyPlan, stored.id == planId else {
            throw PageLampFailure(kind: .notFound, message: "No study plan with id \(planId)")
        }
        let index = Int(itemIndex)
        guard stored.plan.items.indices.contains(index) else {
            throw PageLampFailure(kind: .invalid, message: "The plan has no item \(itemIndex)")
        }
        var items = stored.plan.items
        let item = items[index]
        items[index] = StudyPlanItem(
            date: item.date, courseId: item.courseId, title: item.title, description: item.description,
            materialIds: item.materialIds, minutes: item.minutes, done: done
        )
        let updated = StoredStudyPlan(
            id: stored.id, createdAt: stored.createdAt,
            plan: StudyPlan(
                horizonStart: stored.plan.horizonStart, horizonEnd: stored.plan.horizonEnd, items: items,
                notes: stored.plan.notes
            ),
            origin: stored.origin, generationId: stored.generationId, aiLabel: stored.aiLabel
        )
        db.studyPlan = updated
        return updated
    }

    /// One Stop for every run: a run in flight stops at its next step.
    public func cancelGeneration(generationId: String) async throws(PageLampFailure) {
        await respond("cancelGeneration")
        if db.features.runs.running.contains(generationId) {
            db.features.runs.cancelled.insert(generationId)
        }
    }

    // MARK: - Course calendars (none read or proposed in the mock)

    public func courseCalendar(course reference: String) async throws(PageLampFailure) -> CourseCalendarView {
        await respond("courseCalendar")
        return calendarView(db.courses[try courseIndex(reference)])
    }

    public func calendarCandidates(course reference: String) async throws(PageLampFailure) -> [CalendarCandidate] {
        await respond("calendarCandidates")
        _ = try courseIndex(reference)
        return []
    }

    public func setCalendarSources(course reference: String, include: [String], exclude: [String]) async throws(PageLampFailure) -> [CalendarCandidate] {
        await respond("setCalendarSources")
        _ = try courseIndex(reference)
        return []
    }

    public func downloadMaterialFiles(
        course reference: String, materialIds: [String], observer: any SyncObserver
    ) async throws(PageLampFailure) -> SourceSyncResult {
        await respond("downloadMaterialFiles")
        _ = try courseIndex(reference)
        throw PageLampFailure(kind: .invalid, message: "The preview's mock has no files to download.")
    }

    public func scanCourseCalendar(course reference: String) async throws(PageLampFailure) -> CalendarProposal? {
        await respond("scanCourseCalendar")
        _ = try courseIndex(reference)
        return nil
    }

    public func setCourseDates(course reference: String, dates: CourseDatesInput?) async throws(PageLampFailure) -> CourseCalendarView {
        await respond("setCourseDates")
        return calendarView(db.courses[try courseIndex(reference)])
    }

    public func acceptCalendarProposal(proposalId: Int64, edits: CourseDatesInput?) async throws(PageLampFailure) -> CourseCalendarView {
        await respond("acceptCalendarProposal")
        throw Self.noProposal(proposalId)
    }

    public func acceptPassingProposals(proposalIds: [Int64]) async throws(PageLampFailure) -> [CourseCalendarView] {
        await respond("acceptPassingProposals")
        if let first = proposalIds.first { throw Self.noProposal(first) }
        return []
    }

    public func dismissCalendarProposal(proposalId: Int64) async throws(PageLampFailure) {
        await respond("dismissCalendarProposal")
        throw Self.noProposal(proposalId)
    }

    public func syllabusReadingOffers() async throws(PageLampFailure) -> [SyllabusOffer] {
        await respond("syllabusReadingOffers")
        return []
    }

    public func snoozeCalendarOffers() async throws(PageLampFailure) {
        await respond("snoozeCalendarOffers")
    }

    public func readCourseCalendar(
        course reference: String, generationId: String, options: ReadCalendarOptions, observer: any GenObserver
    ) async throws(PageLampFailure) -> CalendarProposal {
        await respond("readCourseCalendar")
        _ = try courseIndex(reference)
        throw Self.noModel
    }

    public func readCourseCalendars(
        courses: [String], batchId: String, options: ReadCalendarOptions, observer: any CalendarBatchObserver
    ) async throws(PageLampFailure) -> [CalendarRunOutcome] {
        await respond("readCourseCalendars")
        throw Self.noModel
    }

    private func calendarView(_ course: MockCourse) -> CourseCalendarView {
        CourseCalendarView(
            courseId: course.course.id, accepted: nil, proposals: [], status: course.timeline.calendar,
            candidates: [], blocked: nil
        )
    }

    private static func noProposal(_ id: Int64) -> PageLampFailure {
        PageLampFailure(kind: .notFound, message: "No calendar proposal \(id)")
    }

    // MARK: - Removing finished courses (7 days to undo, like the facade)

    public func removalPreview(courses: [String]) async throws(PageLampFailure) -> RemovalPreview {
        await respond("removalPreview")
        var items: [RemovalPreviewItem] = []
        for reference in courses {
            let course = db.courses[try courseIndex(reference)]
            let kind = db.sources.first { $0.id == course.course.sourceId }?.kind ?? .folder
            items.append(RemovalPreviewItem(
                courseId: course.course.id, code: course.course.code, name: course.course.name, sourceKind: kind,
                lifecycle: summary(course).lifecycle, materials: UInt32(course.materials.count),
                downloadedFiles: 0, downloadedBytes: 0, deadlines: UInt32(course.deadlines.count),
                generatedItems: 0, customSettings: false, ownFolderUntouched: kind == .folder,
                cannotSyncAgain: !course.course.enrollmentActive, lostAfterPurge: []
            ))
        }
        return RemovalPreview(items: items, backup: nil)
    }

    public func removeCourses(courses: [String], options: RemoveOptions) async throws(PageLampFailure) -> RemovalReport {
        await respond("removeCourses")
        var indices: [Int] = []
        for reference in courses { indices.append(try courseIndex(reference)) }
        let removedAt = now()
        let purgeAfter = options.purgeNow ? nil : removedAt.addingTimeInterval(7 * 86_400)
        var removed: [RemovedCourse] = []
        for index in indices.sorted(by: >) {
            let course = db.courses.remove(at: index)
            let kind = db.sources.first { $0.id == course.course.sourceId }?.kind ?? .folder
            let record = RemovedCourse(
                removedId: "removed-\(course.course.id)", sourceId: course.course.sourceId, sourceKind: kind,
                externalId: course.course.externalId, courseId: course.course.id, code: course.course.code,
                name: course.course.name, reason: options.reason ?? .other,
                state: options.purgeNow ? .purged : .pending, removedAt: removedAt, purgeAfter: purgeAfter,
                purgedAt: options.purgeNow ? removedAt : nil, purgeInDays: options.purgeNow ? nil : 7,
                keepFiles: options.keepDownloadedFiles, filesPending: false
            )
            db.features.removed.append(MockRemoval(record: record, course: course))
            removed.insert(record, at: 0)
        }
        return RemovalReport(removed: removed, purgedNow: options.purgeNow, backupDeleted: false, backupFailed: false)
    }

    public func removedCourses() async throws(PageLampFailure) -> [RemovedCourse] {
        await respond("removedCourses")
        return db.features.removed.map(\.record)
    }

    public func restoreCourse(removedId: String) async throws(PageLampFailure) -> RestoreOutcome {
        await respond("restoreCourse")
        guard let index = db.features.removed.firstIndex(where: { $0.record.removedId == removedId }) else {
            throw PageLampFailure(kind: .notFound, message: "No removed course \(removedId)")
        }
        let removal = db.features.removed[index]
        guard removal.record.state == .pending else {
            return RestoreOutcome(restored: false, courseId: nil, failure: .other)
        }
        db.features.removed.remove(at: index)
        db.courses.append(removal.course)
        return RestoreOutcome(restored: true, courseId: removal.course.course.id, failure: nil)
    }

    public func purgeRemovedCourses(removedIds: [String]?, permanentIfNoTrash: Bool) async throws(PageLampFailure) -> PurgeReport {
        await respond("purgeRemovedCourses")
        let at = now()
        var purged: [String] = []
        for index in db.features.removed.indices {
            let record = db.features.removed[index].record
            let chosen = removedIds.map { $0.contains(record.removedId) } ?? ((record.purgeAfter ?? .distantFuture) <= at)
            guard chosen, record.state == .pending else { continue }
            db.features.removed[index].record = RemovedCourse(
                removedId: record.removedId, sourceId: record.sourceId, sourceKind: record.sourceKind,
                externalId: record.externalId, courseId: record.courseId, code: record.code, name: record.name,
                reason: record.reason, state: .purged, removedAt: record.removedAt, purgeAfter: record.purgeAfter,
                purgedAt: at, purgeInDays: nil, keepFiles: record.keepFiles, filesPending: false
            )
            purged.append(record.removedId)
        }
        return PurgeReport(purged: purged, filesPending: [], backupDeleted: false, backupFailed: false)
    }

    public func forgetRemovedCourse(removedId: String) async throws(PageLampFailure) {
        await respond("forgetRemovedCourse")
        guard let index = db.features.removed.firstIndex(where: { $0.record.removedId == removedId }) else {
            throw PageLampFailure(kind: .notFound, message: "No removed course \(removedId)")
        }
        db.features.removed.remove(at: index)
    }

    // MARK: - Reminders and the weekly digest (like the facade's reminders.rs and digest.rs)

    public func weeklyDigest() async throws(PageLampFailure) -> WeeklyDigest {
        await respond("weeklyDigest")
        let t = now()
        let today = IsoDate.string(from: t, calendar: calendar)
        let horizon = t.addingTimeInterval(7 * 86_400)
        let courses = db.courses.filter { !$0.course.hidden }.compactMap { course -> DigestCourse? in
            let upcoming = course.deadlines
                .filter { Self.isDeadline($0) }
                .filter { (Self.due($0) ?? .distantPast) >= t && (Self.due($0) ?? .distantPast) < horizon }
                .sorted { (Self.due($0) ?? .distantPast) < (Self.due($1) ?? .distantPast) }
            let active = MockCalendar.lifecycle(course.timeline, keptCurrentUntil: course.keptCurrentUntil).state == .current
            if !active && upcoming.isEmpty { return nil }
            let week = active ? course.timeline.defaultWeek ?? course.timeline.currentWeek : nil
            let materials = week.map { shown in course.materials.filter { $0.weekHint == shown } } ?? []
            return DigestCourse(
                courseId: course.course.id, code: course.course.code, name: course.course.name, active: active,
                week: week, confidence: course.timeline.confidence, phase: course.timeline.phase,
                currentBreakKind: course.timeline.currentBreakKind, lastTeachingWeek: course.timeline.lastTeachingWeek,
                materialCount: UInt32(materials.count), materialTitles: materials.prefix(5).map(\.title),
                deadlines: upcoming
            )
        }
        let plan = db.studyPlan.map { stored -> DigestPlan in
            let weekAgo = isoDay(daysFromToday: -7)
            let lastWeek = stored.plan.items.filter { $0.date >= weekAgo && $0.date < today }
            return DigestPlan(
                lastWeekPlanned: UInt32(lastWeek.count),
                lastWeekDone: UInt32(lastWeek.filter(\.done).count),
                today: stored.plan.items.filter { $0.date == today }
            )
        }
        return WeeklyDigest(generatedAt: t, courses: courses, plan: plan)
    }

    public func reminderSettings() async throws(PageLampFailure) -> ReminderSettings {
        await respond("reminderSettings")
        return db.features.reminderSettings
    }

    public func setReminderSettings(settings: ReminderSettings) async throws(PageLampFailure) {
        await respond("setReminderSettings")
        for time in [settings.digestTime, settings.planTodayTime] where Self.clockTime(time) == nil {
            throw PageLampFailure(kind: .invalid, message: "\(time) must be a time like 09:00.")
        }
        db.features.reminderSettings = settings
    }

    public func reminders(from: Date, to: Date) async throws(PageLampFailure) -> [Reminder] {
        await respond("reminders")
        guard to >= from, to.timeIntervalSince(from) <= 62 * 86_400 else {
            throw PageLampFailure(kind: .invalid, message: "Ask for reminders over a window of at most 62 days.")
        }
        return schedule(from: from, to: to).filter { !db.features.shownReminders.contains($0.id) }
    }

    public func dueReminders(now date: Date) async throws(PageLampFailure) -> [Reminder] {
        await respond("dueReminders")
        let fired = schedule(from: date.addingTimeInterval(-3 * 86_400), to: date.addingTimeInterval(1))
            .filter { $0.fireAt <= date }
        // The latest reminder that fired for each deadline replaces the earlier one.
        var latest: [String: Date] = [:]
        for reminder in fired where reminder.kind == .deadlineSoon {
            latest[Self.deadlineKey(reminder.id)] = max(latest[Self.deadlineKey(reminder.id)] ?? .distantPast, reminder.fireAt)
        }
        return fired.filter { reminder in
            guard !db.features.shownReminders.contains(reminder.id) else { return false }
            switch reminder.kind {
            case .deadlineSoon:
                return (reminder.dueAt.map { date < $0 } ?? false) && latest[Self.deadlineKey(reminder.id)] == reminder.fireAt
            case .weeklyDigest:
                return date.timeIntervalSince(reminder.fireAt) < 3 * 86_400
            case .planToday:
                return calendar.isDate(reminder.fireAt, inSameDayAs: date)
            }
        }
    }

    public func markRemindersShown(ids: [String]) async throws(PageLampFailure) {
        await respond("markRemindersShown")
        if let id = ids.first(where: { id in !["deadline_soon:", "weekly_digest:", "plan_today:"].contains { id.hasPrefix($0) } }) {
            throw PageLampFailure(kind: .invalid, message: "'\(id)' is not a reminder id.")
        }
        db.features.shownReminders.formUnion(ids)
    }

    /// The reminders that fire in [`from`, `to`), soonest first, in the mock's time zone (the
    /// facade's `schedule`): 48 h and 24 h before each deadline of a visible course, the weekly
    /// digest on its day and time, and today's plan when it has open items.
    private func schedule(from: Date, to: Date) -> [Reminder] {
        let settings = db.features.reminderSettings
        let zone = calendar.timeZone.identifier
        let inWindow = { (date: Date) in from <= date && date < to }
        var out: [Reminder] = []
        let visible = db.courses.filter { !$0.course.hidden }
        if settings.deadlineSoon {
            for course in visible {
                for deadline in course.deadlines where Self.isDeadline(deadline) {
                    guard let due = Self.due(deadline) else { continue }
                    for hours in [48, 24] {
                        let fire = due.addingTimeInterval(-Double(hours) * 3600)
                        guard inWindow(fire) else { continue }
                        out.append(Reminder(
                            id: "deadline_soon:\(hours)h:\(deadline.event.id)@\(Int(due.timeIntervalSince1970))",
                            kind: .deadlineSoon, localTime: localText(fire), timeZone: zone, fireAt: fire,
                            title: deadline.event.title, courseId: course.course.id, courseCode: course.course.code,
                            courseName: course.course.name, dueAt: due, hoursBefore: UInt32(hours), count: nil
                        ))
                    }
                }
            }
        }
        let openItems = Dictionary(grouping: db.studyPlan?.plan.items.filter { !$0.done } ?? [], by: \.date)
        var day = calendar.date(byAdding: .day, value: -1, to: calendar.startOfDay(for: from)) ?? from
        let lastDay = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: to)) ?? to
        while day <= lastDay {
            let date = IsoDate.string(from: day, calendar: calendar)
            if settings.weeklyDigest, !visible.isEmpty, Self.weekday(of: day, calendar: calendar) == settings.digestDay,
               let fire = at(day, settings.digestTime), inWindow(fire) {
                let weekEnd = fire.addingTimeInterval(7 * 86_400)
                let count = visible.flatMap(\.deadlines).filter(Self.isDeadline).compactMap(Self.due)
                    .filter { $0 >= fire && $0 < weekEnd }.count
                out.append(simple(.weeklyDigest, date: date, fire: fire, zone: zone, count: count))
            }
            if settings.planToday, let items = openItems[date], let fire = at(day, settings.planTodayTime), inWindow(fire) {
                out.append(simple(.planToday, date: date, fire: fire, zone: zone, count: items.count))
            }
            day = calendar.date(byAdding: .day, value: 1, to: day) ?? lastDay.addingTimeInterval(1)
        }
        return out.sorted { ($0.fireAt, $0.id) < ($1.fireAt, $1.id) }
    }

    private func simple(_ kind: ReminderKind, date: String, fire: Date, zone: String, count: Int) -> Reminder {
        let name = kind == .weeklyDigest ? "weekly_digest" : "plan_today"
        return Reminder(
            id: "\(name):\(date)", kind: kind, localTime: localText(fire), timeZone: zone, fireAt: fire,
            title: nil, courseId: nil, courseCode: nil, courseName: nil, dueAt: nil, hoursBefore: nil,
            count: UInt32(count)
        )
    }

    /// `day` at the wall-clock `time` ("09:00") in the mock's calendar.
    private func at(_ day: Date, _ time: String) -> Date? {
        guard let (hour, minute) = Self.clockTime(time) else { return nil }
        return calendar.date(bySettingHour: hour, minute: minute, second: 0, of: day)
    }

    /// "YYYY-MM-DDTHH:MM" in the mock's calendar (the facade's `local_time`).
    private func localText(_ date: Date) -> String {
        let p = calendar.dateComponents([.year, .month, .day, .hour, .minute], from: date)
        return String(format: "%04d-%02d-%02dT%02d:%02d", p.year ?? 0, p.month ?? 0, p.day ?? 0, p.hour ?? 0, p.minute ?? 0)
    }

    private static func clockTime(_ text: String) -> (Int, Int)? {
        let parts = text.split(separator: ":")
        guard parts.count == 2, parts.allSatisfy({ $0.count == 2 }), let hour = Int(parts[0]), let minute = Int(parts[1]),
              (0...23).contains(hour), (0...59).contains(minute) else { return nil }
        return (hour, minute)
    }

    private static func weekday(of day: Date, calendar: Calendar) -> DayOfWeek {
        let days: [DayOfWeek] = [.sunday, .monday, .tuesday, .wednesday, .thursday, .friday, .saturday]
        return days[calendar.component(.weekday, from: day) - 1]
    }

    /// When a deadline is (the facade's `Event::when`: due, else starts).
    private static func due(_ deadline: Deadline) -> Date? {
        deadline.event.dueAt ?? deadline.event.startsAt
    }

    /// The facade's `is_deadline`: assignments, quizzes, exams and planner items.
    private static func isDeadline(_ deadline: Deadline) -> Bool {
        [.assignmentDue, .quizDue, .exam, .plannerItem].contains(deadline.event.kind)
    }

    /// "<event id>@<due>" of a deadline reminder's id: the same for its 48 h and 24 h reminders.
    private static func deadlineKey(_ id: String) -> String {
        guard let range = id.range(of: "h:") else { return id }
        return String(id[range.upperBound...])
    }

    // MARK: - Files and syncs

    public func materialLocalFile(materialId: String, purpose: LocalFileUse) async throws(PageLampFailure) -> String? {
        await respond("materialLocalFile")
        return nil
    }

    public func cancelSync() async throws(PageLampFailure) {
        await respond("cancelSync")
    }
}
