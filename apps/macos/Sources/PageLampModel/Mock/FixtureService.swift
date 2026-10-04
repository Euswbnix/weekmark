// A service for snapshots and tests: answers like another service (normally the mock) with some
// answers replaced, so every state of a page (spec §3.9: S3, S4, the first sync, S11, S12 quiet
// empties, S14 section errors, Next up, AI access off, …) can be rendered on demand. The app never
// uses it; it touches no data folder, keychain or network of its own.

import Foundation
import PageLampKit

public struct FixtureService: ForwardingService {
    /// A call that can be made to fail (S14).
    public enum Part: Hashable, Sendable {
        /// `list_courses()`.
        case courses
        /// Every `list_deadlines(…)`.
        case deadlines
        /// `list_deadlines(course, …)` of one course (the course page's Deadlines section).
        case courseDeadlines
        case studyPlan
        /// `week_materials(…)`.
        case week
        /// `course_overview(…)`.
        case overview
        /// `mcp_client_configs(…)` (the Connect page).
        case clientConfigs
        /// `clear_last_crash()` (the crash notice's Dismiss).
        case clearCrash
    }

    /// What `latestStudyPlan()` answers.
    public enum Plan: Sendable {
        /// The base service's plan.
        case base
        /// No plan saved yet (S12).
        case none
        case stored(StoredStudyPlan)
    }

    public var base: any PageLampService
    /// Replaces the course list (e.g. `[]` for S4).
    public var courses: [CourseSummary]?
    /// Replaces the deadlines (e.g. `[]` for the quiet Next 7 days).
    public var deadlines: [Deadline]?
    /// Added to the deadlines (e.g. one due within the hour for Next up).
    public var extraDeadlines: [Deadline]
    public var plan: Plan
    /// Overrides `status().syncInProgress` (another process syncing, e.g. the first sync).
    public var syncInProgress: Bool?
    public var failing: Set<Part>
    /// Every course's AI access is off (the student's switch, not No AI).
    public var aiAccessOff: Bool
    /// Today is outside every course's term: no current week (S11).
    public var outsideTerm: Bool
    /// Weeks come back without modules or materials (S12).
    public var emptyWeek: Bool

    public init(
        base: any PageLampService,
        courses: [CourseSummary]? = nil,
        deadlines: [Deadline]? = nil,
        extraDeadlines: [Deadline] = [],
        plan: Plan = .base,
        syncInProgress: Bool? = nil,
        failing: Set<Part> = [],
        aiAccessOff: Bool = false,
        outsideTerm: Bool = false,
        emptyWeek: Bool = false
    ) {
        self.base = base
        self.courses = courses
        self.deadlines = deadlines
        self.extraDeadlines = extraDeadlines
        self.plan = plan
        self.syncInProgress = syncInProgress
        self.failing = failing
        self.aiAccessOff = aiAccessOff
        self.outsideTerm = outsideTerm
        self.emptyWeek = emptyWeek
    }

    private func fail(_ part: Part) throws(PageLampFailure) {
        if failing.contains(part) {
            throw PageLampFailure(kind: .internal, message: "fixture: \(part) failed")
        }
    }

    // MARK: - Tweaks

    private func course(_ course: Course) -> Course {
        guard aiAccessOff, course.aiPolicy != .prohibited else { return course }
        return Course(
            id: course.id, sourceId: course.sourceId, externalId: course.externalId, code: course.code,
            name: course.name, termStart: course.termStart, termEnd: course.termEnd, termSource: course.termSource,
            url: course.url, aiPolicy: course.aiPolicy, aiPolicyNote: course.aiPolicyNote, aiAccess: false,
            hidden: course.hidden, enrollmentActive: course.enrollmentActive, updatedAt: course.updatedAt
        )
    }

    private func aiMaterials(_ state: AiMaterialsState) -> AiMaterialsState {
        aiAccessOff && state == .readable ? .turnedOff : state
    }

    private func timeline(_ timeline: CourseTimeline) -> CourseTimeline {
        guard outsideTerm else { return timeline }
        return CourseTimeline(
            asOf: timeline.asOf, currentWeek: nil, confidence: .low,
            evidence: timeline.evidence + ["outside term: today is after the term ended"],
            currentModuleIds: [], outsideTerm: true,
            // Outside the term is the ended phase (or not started): no default week (M0.10).
            phase: .ended, phaseConfidence: .low, startsOn: timeline.startsOn, defaultWeek: nil,
            breakAfterWeek: nil, lastTeachingWeek: nil, currentBreakKind: nil,
            notesWeek: timeline.notesWeek, term: timeline.term, calendar: timeline.calendar,
            evidenceItems: timeline.evidenceItems
        )
    }

    private func summary(_ summary: CourseSummary) -> CourseSummary {
        guard aiAccessOff || outsideTerm else { return summary }
        let readable = aiMaterials(summary.aiMaterials) == .readable
        return CourseSummary(
            course: course(summary.course),
            aiMaterials: aiMaterials(summary.aiMaterials),
            timeline: timeline(summary.timeline),
            lifecycle: summary.lifecycle,
            counts: CourseCounts(
                modules: summary.counts.modules,
                materials: summary.counts.materials,
                indexedMaterials: readable ? summary.counts.indexedMaterials : 0,
                upcomingDeadlines: summary.counts.upcomingDeadlines
            ),
            nextDeadline: summary.nextDeadline,
            sourceLabel: summary.sourceLabel,
            lastSyncedAt: summary.lastSyncedAt,
            deadlinesSyncedAt: summary.deadlinesSyncedAt,
            structurePending: summary.structurePending
        )
    }

    // MARK: - PageLampService

    public func status() async throws(PageLampFailure) -> AppStatus {
        let status = try await base.status()
        guard let syncInProgress else { return status }
        return AppStatus(
            version: status.version, dataDir: status.dataDir, dbPath: status.dbPath, sources: status.sources,
            counts: status.counts, lastSyncedAt: status.lastSyncedAt, syncInProgress: syncInProgress,
            autoSync: status.autoSync, deadlinesSyncedAt: status.deadlinesSyncedAt
        )
    }

    public func listCourses() async throws(PageLampFailure) -> [CourseSummary] {
        try fail(.courses)
        if let courses { return courses.map(summary) }
        return try await base.listCourses().map(summary)
    }

    public func listDeadlines(course: String?, daysAhead: UInt32, daysBack: UInt32) async throws(PageLampFailure) -> [Deadline] {
        try fail(.deadlines)
        if course != nil { try fail(.courseDeadlines) }
        var list: [Deadline]
        if let deadlines {
            list = deadlines
        } else {
            list = try await base.listDeadlines(course: course, daysAhead: daysAhead, daysBack: daysBack)
        }
        list += extraDeadlines.filter { course == nil || $0.event.courseId == course }
        return list
    }

    public func latestStudyPlan() async throws(PageLampFailure) -> StoredStudyPlan? {
        try fail(.studyPlan)
        switch plan {
        case .base: return try await base.latestStudyPlan()
        case .none: return nil
        case .stored(let stored): return stored
        }
    }

    public func courseOverview(course reference: String) async throws(PageLampFailure) -> CourseOverview {
        try fail(.overview)
        let overview = try await base.courseOverview(course: reference)
        return CourseOverview(
            course: course(overview.course), aiMaterials: aiMaterials(overview.aiMaterials),
            timeline: timeline(overview.timeline), lifecycle: overview.lifecycle,
            currentModules: overview.currentModules,
            recentMaterials: overview.recentMaterials, upcomingDeadlines: overview.upcomingDeadlines,
            recentAnnouncements: overview.recentAnnouncements, sourceLabel: overview.sourceLabel,
            lastSyncedAt: overview.lastSyncedAt, downloadableFiles: overview.downloadableFiles,
            deadlinesSyncedAt: overview.deadlinesSyncedAt, structurePending: overview.structurePending
        )
    }

    public func weekMaterials(course reference: String, week: UInt32?) async throws(PageLampFailure) -> WeekMaterials {
        try fail(.week)
        let base = try await base.weekMaterials(course: reference, week: outsideTerm ? nil : week)
        let outside = outsideTerm && week == nil
        return WeekMaterials(
            course: course(base.course), aiMaterials: aiMaterials(base.aiMaterials),
            week: outside ? nil : base.week, requestedWeek: base.requestedWeek, timeline: timeline(base.timeline),
            modules: emptyWeek ? [] : base.modules,
            materials: emptyWeek ? [] : base.materials,
            availableWeeks: base.availableWeeks,
            note: outside ? "Today is outside the term dates." : (emptyWeek ? "No modules or materials." : base.note),
            noteKind: outside ? .outsideTerm : (emptyWeek ? .noMaterialsThisWeek : base.noteKind)
        )
    }

    public func courseTimeline(course: String) async throws(PageLampFailure) -> CourseTimeline {
        timeline(try await base.courseTimeline(course: course))
    }

    public func keepCourseCurrent(course: String, until: String?) async throws(PageLampFailure) -> Course {
        self.course(try await base.keepCourseCurrent(course: course, until: until))
    }

    public func clearKeepCourseCurrent(course: String) async throws(PageLampFailure) -> Course {
        self.course(try await base.clearKeepCourseCurrent(course: course))
    }

    public func confirmCourseDates(course: String) async throws(PageLampFailure) -> CourseTimeline {
        timeline(try await base.confirmCourseDates(course: course))
    }

    public func mcpClientConfigs(pagelampBinary: String) async throws(PageLampFailure) -> [McpClientConfig] {
        try fail(.clientConfigs)
        return try await base.mcpClientConfigs(pagelampBinary: pagelampBinary)
    }

    public func clearLastCrash() async throws(PageLampFailure) {
        try fail(.clearCrash)
        try await base.clearLastCrash()
    }
}
