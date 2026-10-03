// The services: MockService behaves like the facade where the UI can tell; LiveService maps the
// facade's errors (over a temp data folder with in-memory secrets, never the real one).

import Foundation
import PageLampKit
import PageLampModel
import Testing

@Suite("MockService")
struct MockServiceTests {
    func mock(_ scenario: MockScenario) -> MockService {
        MockService(scenario: scenario, timing: .instant, calendar: TestClock.calendar, now: { TestClock.now })
    }

    @Test("demo courses: weeks, policies, readable counts")
    func courses() async throws {
        let courses = try await mock(.demo).listCourses()
        let byCode = Dictionary(uniqueKeysWithValues: courses.map { ($0.course.code ?? "", $0) })
        #expect(byCode["DEMO101"]?.timeline.currentWeek == 4)
        #expect(byCode["DEMO101"]?.counts.materials == 13)
        #expect(byCode["DEMO101"]?.counts.indexedMaterials == 10)
        #expect(byCode["DEMO101"]?.nextDeadline?.event.title == "Problem Set 2")
        #expect(byCode["DEMO205"]?.sourceLabel == "Demo Canvas")
        // No AI: materials withheld, nothing counted as readable.
        #expect(byCode["DEMO310"]?.aiMaterials == .withheldByPolicy)
        #expect(byCode["DEMO310"]?.counts.indexedMaterials == 0)
        #expect(byCode["DEMO310"]?.timeline.currentWeek == nil)
        #expect(byCode["DEMO099"]?.course.hidden == true)
        #expect(byCode["DEMO099"]?.course.enrollmentActive == false)
    }

    @Test("course lookups by id or code; unknown ones are notFound")
    func lookups() async throws {
        let service = mock(.demo)
        let overview = try await service.courseOverview(course: "DEMO205")
        #expect(overview.downloadableFiles == 1)
        #expect(overview.currentModules.map(\.name) == ["Unit C: Rows"])
        let week = try await service.weekMaterials(course: "folder:demo-courses/course/DEMO101", week: 2)
        #expect(week.materials.count == 2)
        #expect(week.availableWeeks == [1, 2, 3, 4])
        do {
            _ = try await service.courseOverview(course: "NOPE")
            Issue.record("expected notFound")
        } catch {
            #expect(error.kind == .notFound)
        }
    }

    @Test("sync streams events per source; an expired token fails that source only")
    func syncEvents() async throws {
        let service = mock(.expired)
        let recorder = EventRecorder()
        let summary = try await service.syncAll(request: SyncRequest(), observer: recorder)
        #expect(!summary.ok)
        #expect(summary.results.map(\.ok) == [true, true, false])
        #expect(summary.results[2].errorKind == .authExpiredOrRevoked)
        let events = recorder.events
        let starts = events.compactMap { if case .sourceStarted(let id, _) = $0 { id } else { nil } }
        let ends = events.compactMap { if case .sourceFinished(let id, let ok, _, _) = $0 { "\(id):\(ok)" } else { nil } }
        #expect(starts == ["folder:demo-courses", "ical:demo-calendar", "canvas:canvas.demo.test"])
        #expect(ends == ["folder:demo-courses:true", "ical:demo-calendar:true", "canvas:canvas.demo.test:false"])
        #expect(events.contains { if case .warning = $0 { true } else { false } })
        #expect(events.contains { if case .progress(_, _, let current, let total, _, _) = $0 { current == total } else { false } })
    }

    @Test("busy while another process syncs; unknown sources are notFound")
    func busyAndNotFound() async throws {
        do {
            _ = try await mock(.busy).syncAll(request: SyncRequest(), observer: EventRecorder())
            Issue.record("expected busy")
        } catch {
            #expect(error.kind == .busy)
        }
        do {
            _ = try await mock(.demo).syncSource(sourceId: "nope", request: SyncRequest(), observer: EventRecorder())
            Issue.record("expected notFound")
        } catch {
            #expect(error.kind == .notFound)
        }
    }

    @Test("course lane: timeline, lifecycle summary, I'm still taking this, snoozes")
    func courseLane() async throws {
        let service = mock(.demo)
        #expect(try await service.courseTimeline(course: "DEMO101").currentWeek == 4)
        let kept = try await service.keepCourseCurrent(course: "DEMO101", until: "2027-01-31")
        #expect(kept.code == "DEMO101")
        var summary = try await service.lifecycleSummary()
        let courseCount = try await service.listCourses().count
        #expect(summary.courses.count == courseCount)
        let entry = summary.courses.first { $0.code == "DEMO101" }
        #expect(entry?.lifecycle.keptCurrentUntil == "2027-01-31")
        #expect(entry?.lifecycle.state == .current && entry?.lifecycle.confidence == .high)
        #expect(summary.suggested.isEmpty && !summary.showBanner)
        _ = try await service.clearKeepCourseCurrent(course: "DEMO101")
        let listed = try await service.listCourses().first { $0.course.code == "DEMO101" }
        #expect(listed?.lifecycle.keptCurrentUntil == nil)
        // Without a date: today + keepCurrentDays() (the mock has no term dates).
        _ = try await service.keepCourseCurrent(course: "DEMO205", until: nil)
        summary = try await service.lifecycleSummary()
        #expect(summary.courses.first { $0.code == "DEMO205" }?.lifecycle.keptCurrentUntil?.hasPrefix("2027-01-") == true)

        try await service.snoozeRemovalSuggestions(courses: ["DEMO099"], kind: .keep)
        try await service.clearRemovalSnooze(courses: ["DEMO099"])
        try await service.snoozeLifecycleBanner()
        #expect(try await service.lifecycleSummary().bannerSnoozedUntil == "2026-10-09")
        #expect(try await service.confirmCourseDates(course: "DEMO101").currentWeek == 4)
        do {
            _ = try await service.courseTimeline(course: "NOPE")
            Issue.record("expected notFound")
        } catch {
            #expect(error.kind == .notFound)
        }
    }

    @Test("updates: a fresh install checks after the disclosure; an upgrade shows What's new first")
    func updates() async throws {
        let service = mock(.demo)
        let now = TestClock.now
        #expect(try await service.effectiveUpdateChannel() == .beta, "0.1.0-mock is a pre-release")
        var tasks = try await service.startupTasks(now: now)
        #expect(tasks.whatsNew == nil && tasks.updatedFrom == nil && !tasks.updateCheckDue)
        try await service.acknowledgeUpdateDisclosure()
        #expect(try await service.startupTasks(now: now).updateCheckDue)

        let record = UpdateCheckRecord(at: now, channel: .beta, outcome: .upToDate)
        try await service.recordUpdateCheck(record: record)
        #expect(try await service.lastUpdateCheck() == record)
        #expect(try await !service.startupTasks(now: now).updateCheckDue)
        #expect(try await service.startupTasks(now: now.addingTimeInterval(24 * 3600)).updateCheckDue)

        await service.simulateUpgrade(from: "0.3.0-alpha.1")
        tasks = try await service.startupTasks(now: now.addingTimeInterval(24 * 3600))
        #expect(tasks.whatsNew?.topics == [.courseWeeks], "the Mac app's topics: no update check")
        #expect(tasks.updatedFrom == "0.3.0-alpha.1")
        #expect(!tasks.updateCheckDue, "not while What's new waits")
        try await service.acknowledgeWhatsNew()
        tasks = try await service.startupTasks(now: now.addingTimeInterval(24 * 3600))
        #expect(tasks.whatsNew == nil && tasks.updateCheckDue)

        // Acknowledging the Mac app's What's new never counts as the update disclosure.
        let fresh = mock(.demo)
        await fresh.simulateUpgrade(from: nil)
        try await fresh.acknowledgeWhatsNew()
        #expect(try await !fresh.startupTasks(now: now).updateCheckDue, "no disclosure")

        try await service.setUpdatePrefs(prefs: UpdatePrefs(autoCheck: false, channel: .stable))
        #expect(try await service.updatePrefs() == UpdatePrefs(autoCheck: false, channel: .stable))
        #expect(try await service.effectiveUpdateChannel() == .stable)
        #expect(try await !service.startupTasks(now: now.addingTimeInterval(48 * 3600)).updateCheckDue)
    }

    @Test("activity: nothing running here; another process's sync shows")
    func activity() async throws {
        let idle = try await mock(.demo).activity()
        #expect(idle.items.isEmpty && !idle.otherProcessSyncing)
        #expect(try await mock(.busy).activity().otherProcessSyncing)
    }

    @Test("empty scenario: no sources; Connect and diagnostics still answer")
    func emptyScenario() async throws {
        let service = mock(.empty)
        let status = try await service.status()
        #expect(status.sources.isEmpty)
        #expect(status.counts.courses == 0)
        #expect(try await service.latestStudyPlan() == nil)
        let configs = try await service.mcpClientConfigs(pagelampBinary: MockService.binaryPath)
        #expect(configs.map(\.client) == [.claudeDesktop, .claudeCode, .codex, .generic])
        #expect(configs[0].content.contains(MockService.binaryPath))
        #expect(try await service.doctor().mcpClients.claudeCode)
    }
}

@Suite("LiveService")
struct LiveServiceTests {
    @Test("every facade error maps to its failure kind")
    func errorMapping() {
        let cases: [(PageLampError, PageLampFailure.Kind)] = [
            (.Auth(message: "a"), .auth),
            (.Network(message: "n"), .network),
            (.Invalid(message: "i"), .invalid),
            (.NotFound(message: "f"), .notFound),
            (.Ambiguous(message: "m"), .ambiguous),
            (.Busy(message: "b"), .busy),
            (.Schema(message: "s"), .schema),
            (.Blocked(message: "k", reason: .courseHidden), .blocked),
            (.Model(message: "o", kind: .authRejected, retryAfterSecs: nil), .model),
            (.Cancelled(message: "c"), .cancelled),
            (.Internal(message: "x"), .internal),
            (.Panic(message: "p"), .panic),
        ]
        for (error, kind) in cases {
            let failure = PageLampFailure.from(error)
            #expect(failure.kind == kind)
            #expect(!failure.message.isEmpty)
        }
        #expect(PageLampFailure.from(CancellationError()).kind == .internal)
    }

    @Test("an AI failure keeps the facade's codes: the block reason, the model error, the wait")
    func aiErrorDetails() {
        let budget = PageLampFailure.from(PageLampError.Blocked(message: "b", reason: .budgetReached))
        #expect(budget.kind == .blocked && budget.blocked == .budgetReached)
        #expect(budget.modelError == nil && budget.retryAfterSecs == nil)
        // A refusal the facade didn't name keeps its kind, without a reason.
        let unnamed = PageLampFailure.from(PageLampError.Blocked(message: "u", reason: nil))
        #expect(unnamed.kind == .blocked && unnamed.blocked == nil)
        let limited = PageLampFailure.from(PageLampError.Model(message: "r", kind: .rateLimited, retryAfterSecs: 30))
        #expect(limited.kind == .model && limited.modelError == .rateLimited && limited.retryAfterSecs == 30)
        #expect(limited.blocked == nil)
        let bare = PageLampFailure.from(PageLampError.Model(message: "m", kind: nil, retryAfterSecs: nil))
        #expect(bare.kind == .model && bare.modelError == nil && bare.retryAfterSecs == nil)
        // Every other kind carries none of them.
        let network = PageLampFailure.from(PageLampError.Network(message: "n"))
        #expect(network.blocked == nil && network.modelError == nil && network.retryAfterSecs == nil)
    }

    @Test("over the real facade (temp folder, in-memory secrets)")
    func realFacade() async throws {
        let dir = URL(filePath: NSTemporaryDirectory(), directoryHint: .isDirectory)
            .appending(path: "PageLampModelTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: dir) }
        let core = try await PageLamp.openWithMemorySecrets(dataDir: dir.path(percentEncoded: false))
        let service: any PageLampService = LiveService(core: core)

        let status = try await service.status()
        #expect(!status.version.isEmpty)
        #expect(status.sources.isEmpty)
        #expect(try await service.listCourses().isEmpty)
        #expect(try await service.latestStudyPlan() == nil)
        do {
            _ = try await service.courseOverview(course: "NOPE")
            Issue.record("expected notFound")
        } catch {
            #expect(error.kind == .notFound)
        }
        do {
            _ = try await service.syncSource(sourceId: "folder:nope", request: SyncRequest(), observer: EventRecorder())
            Issue.record("expected notFound")
        } catch {
            #expect(error.kind == .notFound)
        }

        // The course lane and the update facade reach the core.
        let summary = try await service.lifecycleSummary()
        #expect(summary.courses.isEmpty && !summary.showBanner)
        do {
            _ = try await service.courseTimeline(course: "NOPE")
            Issue.record("expected notFound")
        } catch {
            #expect(error.kind == .notFound)
        }
        #expect(try await service.updatePrefs().autoCheck)
        let tasks = try await service.startupTasks(now: Date())
        #expect(tasks.whatsNew == nil, "a fresh data folder is a fresh install")
        try await service.acknowledgeUpdateDisclosure()
        #expect(try await service.startupTasks(now: Date()).updateCheckDue)
        #expect(try await service.lastUpdateCheck() == nil)
        let activity = try await service.activity()
        #expect(activity.items.isEmpty && !activity.otherProcessSyncing)
        #expect(notNowDays() == 14 && keepCurrentDays() == 120 && keepForever() == "9999-12-31")

        // The M1–M3 calls reach the core too.
        #expect(try await service.reminderSettings().deadlineSoon)
        #expect(try await service.removedCourses().isEmpty)
        #expect(try await service.aiStatus().providers.isEmpty)
        try await service.cancelGeneration(generationId: "never-ran")
        do {
            _ = try await service.courseCalendar(course: "NOPE")
            Issue.record("expected notFound")
        } catch {
            #expect(error.kind == .notFound)
        }
    }
}

/// `ForwardingService` forwards whatever a wrapper doesn't implement, so an override with a typo
/// or a slightly different signature would silently forward too. These call every override
/// through `any PageLampService` and check that the wrapper's own behaviour shows up.
@Suite("ForwardingService wrappers")
struct ForwardingWrapperTests {
    func mock() -> MockService {
        MockService(scenario: .demo, timing: .instant, calendar: TestClock.calendar, now: { TestClock.now })
    }

    /// `body` must fail with the fixture's own failure (not the base's answer).
    func expectFixtureFailure(_ name: String, _ body: () async throws -> Void) async {
        do {
            try await body()
            Issue.record("\(name) answered from the base service")
        } catch {
            let message = (error as? PageLampFailure)?.message ?? "\(error)"
            #expect(message.hasPrefix("fixture:"), "\(name): \(message)")
        }
    }

    @Test("every FixtureService override is the one called")
    func fixtureOverrides() async throws {
        let base = mock()
        let failing: any PageLampService = FixtureService(
            base: base, failing: [.courses, .deadlines, .studyPlan, .week, .overview, .clientConfigs, .clearCrash]
        )
        await expectFixtureFailure("listCourses") { _ = try await failing.listCourses() }
        await expectFixtureFailure("listDeadlines") { _ = try await failing.listDeadlines(course: nil, daysAhead: 7, daysBack: 0) }
        await expectFixtureFailure("latestStudyPlan") { _ = try await failing.latestStudyPlan() }
        await expectFixtureFailure("weekMaterials") { _ = try await failing.weekMaterials(course: "DEMO101", week: nil) }
        await expectFixtureFailure("courseOverview") { _ = try await failing.courseOverview(course: "DEMO101") }
        await expectFixtureFailure("mcpClientConfigs") { _ = try await failing.mcpClientConfigs(pagelampBinary: MockService.binaryPath) }
        await expectFixtureFailure("clearLastCrash") { try await failing.clearLastCrash() }
        let courseDeadlines: any PageLampService = FixtureService(base: base, failing: [.courseDeadlines])
        await expectFixtureFailure("listDeadlines(course:)") {
            _ = try await courseDeadlines.listDeadlines(course: "DEMO101", daysAhead: 7, daysBack: 0)
        }

        #expect(try await base.status().syncInProgress == false)
        let syncing: any PageLampService = FixtureService(base: base, syncInProgress: true)
        #expect(try await syncing.status().syncInProgress)

        #expect(try await base.courseTimeline(course: "DEMO101").currentWeek == 4)
        let outside: any PageLampService = FixtureService(base: base, outsideTerm: true)
        #expect(try await outside.courseTimeline(course: "DEMO101").currentWeek == nil)
        #expect(try await outside.confirmCourseDates(course: "DEMO101").currentWeek == nil)

        #expect(try await base.keepCourseCurrent(course: "DEMO101", until: nil).aiAccess)
        let aiOff: any PageLampService = FixtureService(base: base, aiAccessOff: true)
        #expect(try await !aiOff.keepCourseCurrent(course: "DEMO101", until: nil).aiAccess)
        #expect(try await !aiOff.clearKeepCourseCurrent(course: "DEMO101").aiAccess)
    }

    @Test("HeldService's holds apply through any PageLampService")
    func heldOverrides() async throws {
        let statusHold = CallHold()
        let configsHold = CallHold()
        await statusHold.arm(failing: PageLampFailure(kind: .network, message: "held status"))
        await configsHold.arm(failing: PageLampFailure(kind: .network, message: "held configs"))
        let service: any PageLampService = HeldService(base: mock(), statusHold: statusHold, configsHold: configsHold)

        let status = Task { try await service.status() }
        await statusHold.waitUntilHeld()
        await statusHold.release()
        #expect((await status.result.failure as? PageLampFailure)?.message == "held status")

        let configs = Task { try await service.mcpClientConfigs(pagelampBinary: MockService.binaryPath) }
        await configsHold.waitUntilHeld()
        await configsHold.release()
        #expect((await configs.result.failure as? PageLampFailure)?.message == "held configs")
    }
}

private extension Result {
    var failure: Failure? {
        if case .failure(let error) = self { error } else { nil }
    }
}

@Suite("MockService: M1–M3 calls")
struct MockFeatureTests {
    func mock(_ scenario: MockScenario = .demo) -> MockService {
        MockService(scenario: scenario, timing: .instant, calendar: TestClock.calendar, now: { TestClock.now })
    }

    @Test("removing a course lists it with 7 days to undo; restoring puts it back")
    func removal() async throws {
        let service = mock()
        let before = try await service.listCourses().count
        let preview = try await service.removalPreview(courses: ["DEMO205"])
        #expect(preview.items.map(\.code) == ["DEMO205"])
        let report = try await service.removeCourses(
            courses: ["DEMO205"],
            options: RemoveOptions(reason: .ended, keepDownloadedFiles: false, purgeNow: false, deletePreUpdateBackup: false)
        )
        #expect(report.removed.map(\.state) == [.pending] && report.removed.first?.purgeInDays == 7)
        #expect(try await service.listCourses().count == before - 1)
        let removed = try await service.removedCourses()
        #expect(removed.map(\.code) == ["DEMO205"])

        // Not due yet: a purge of every due course leaves it.
        #expect(try await service.purgeRemovedCourses(removedIds: nil, permanentIfNoTrash: false).purged.isEmpty)
        let restored = try await service.restoreCourse(removedId: removed[0].removedId)
        #expect(restored.restored)
        #expect(try await service.listCourses().count == before)
        #expect(try await service.removedCourses().isEmpty)

        // Purged at once: can't be restored, only forgotten.
        let gone = try await service.removeCourses(
            courses: ["DEMO205"],
            options: RemoveOptions(reason: nil, keepDownloadedFiles: false, purgeNow: true, deletePreUpdateBackup: false)
        )
        #expect(!(try await service.restoreCourse(removedId: gone.removed[0].removedId).restored))
        try await service.forgetRemovedCourse(removedId: gone.removed[0].removedId)
        #expect(try await service.removedCourses().isEmpty)
    }

    @Test("reminders: settings round-trip; the facade's kinds; due once, then shown")
    func reminders() async throws {
        let service = mock()
        var settings = try await service.reminderSettings()
        #expect(settings.deadlineSoon && settings.digestTime == "09:00")
        let week = try await service.reminders(from: TestClock.now, to: TestClock.now.addingTimeInterval(7 * 86_400))
        #expect(week.contains { $0.kind == .deadlineSoon } && week.contains { $0.kind == .weeklyDigest })
        #expect(week.filter { $0.kind == .deadlineSoon }.allSatisfy { [48, 24].contains($0.hoursBefore) })
        let later = week[0].fireAt.addingTimeInterval(60)
        let due = try await service.dueReminders(now: later)
        #expect(due.map(\.id).contains(week[0].id))
        try await service.markRemindersShown(ids: due.map(\.id))
        #expect(try await !service.dueReminders(now: later).contains { $0.id == week[0].id })

        settings = ReminderSettings(
            deadlineSoon: false, weeklyDigest: settings.weeklyDigest, digestDay: .friday, digestTime: "18:30",
            planToday: settings.planToday, planTodayTime: settings.planTodayTime, runInBackground: true
        )
        try await service.setReminderSettings(settings: settings)
        #expect(try await service.reminderSettings() == settings)
        // Deadline reminders off: only the week's digest, now on Friday 18:30.
        let digests = try await service.reminders(from: TestClock.now, to: TestClock.now.addingTimeInterval(7 * 86_400))
        #expect(digests.map(\.id) == ["weekly_digest:2026-09-25"])
        #expect(digests.first?.localTime == "2026-09-25T18:30")
        #expect(try await service.weeklyDigest().generatedAt == TestClock.now)
    }

    @Test("AI settings stick; generations are blocked without a model")
    func ai() async throws {
        let service = mock()
        try await service.setMonthlyBudget(microUsd: 2_000_000)
        #expect(try await service.aiStatus().budget.monthlyMicroUsd == 2_000_000)
        #expect(try await service.usageSummary(month: nil).month == "2026-09-01")
        try await service.setAiOutputLanguage(language: .course)
        #expect(try await service.aiOutputLanguage() == .course)
        try await service.setCourseMaterialSharing(course: "DEMO101", answer: .allowed)
        #expect(try await service.codexStatus().login.state == .signedOut)
        do {
            _ = try await service.explainWeek(
                course: "DEMO101", week: nil, generationId: "g1", options: ExplainOptions(), observer: GenEventStream()
            )
            Issue.record("expected blocked")
        } catch {
            #expect(error.kind == .blocked && error.blocked == .noModelChosen)
        }
        try await service.cancelGeneration(generationId: "g1")
        #expect(try await service.removeAllAiData().providersRemoved == 0)
        // The budget goes back to its default (US$5), as in the facade.
        #expect(try await service.aiStatus().budget.monthlyMicroUsd == 5_000_000)
    }

    @Test("the weekly note is blocked without a model; only API keys and local models prepare it")
    func weeklyNote() async throws {
        let service = mock()
        #expect(try await service.weeklyNotes().isEmpty)
        #expect(try await service.weeklyNoteSettings().prepareOnMondayAllowed == false)
        do {
            _ = try await service.setPrepareWeeklyNoteOnMonday(on: true)
            Issue.record("expected invalid")
        } catch {
            #expect(error.kind == .invalid)
        }
        // A model on this computer (the aiLocal scenario routes the note to Ollama) may prepare it.
        let local = mock(.aiLocal)
        let settings = try await local.setPrepareWeeklyNoteOnMonday(on: true)
        #expect(settings.prepareOnMonday && settings.prepareOnMondayAllowed)
        // The ChatGPT plan isn't offered: choosing it is refused.
        let plan = ModelChoice(backend: .codex, model: "plan-model", effort: .lowest)
        do {
            try await service.setFeatureModel(feature: .weeklyNote, choice: plan)
            Issue.record("expected blocked")
        } catch {
            #expect(error.kind == .blocked && error.blocked == .backendDisabledInThisBuild)
        }
        #expect(try await service.weeklyNoteSettings().prepareOnMondayAllowed == false)
        do {
            _ = try await service.writeWeeklyNote(generationId: "n1", options: WeeklyNoteOptions(), observer: GenEventStream())
            Issue.record("expected blocked")
        } catch {
            #expect(error.kind == .blocked && error.blocked == .noModelChosen)
        }
    }

    @Test("a study plan item can be checked off")
    func planItem() async throws {
        let service = mock()
        let plan = try #require(try await service.latestStudyPlan())
        let checked = try await service.setStudyPlanItemDone(planId: plan.id, itemIndex: 0, done: !plan.plan.items[0].done)
        #expect(checked.plan.items[0].done != plan.plan.items[0].done)
        #expect(try await service.latestStudyPlan()?.plan.items[0].done == checked.plan.items[0].done)
        do {
            _ = try await service.setStudyPlanItemDone(planId: plan.id + 1, itemIndex: 0, done: true)
            Issue.record("expected notFound")
        } catch {
            #expect(error.kind == .notFound)
        }
    }
}
