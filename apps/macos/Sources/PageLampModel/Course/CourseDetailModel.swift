// The course detail page's data (spec §2.8, §3.2; M1 read-only): the overview (recent
// announcements), the displayed week's materials and the course's deadlines, each loaded on its
// own so one failure never blanks the page (S14 section errors).
//
// The page's section and week live in `CourseUIState` (AppModel, kept for the session); this
// model lives as long as the page does.

import Foundation
import Observation
import PageLampKit

/// One part of a page that loads on its own.
public enum CourseLoadable<Value: Sendable & Equatable>: Equatable, Sendable {
    case loading
    case loaded(Value)
    case failed(PageLampFailure)

    public var value: Value? {
        if case .loaded(let value) = self { return value }
        return nil
    }

    public var failure: PageLampFailure? {
        if case .failed(let failure) = self { return failure }
        return nil
    }
}

/// The inspector's sections (spec §2.6), in order. Cross-links ("AI Policy…", "Set Term
/// Dates…") open the inspector scrolled to one of them.
public enum CourseInspectorSection: String, CaseIterable, Hashable, Sendable {
    case aiPolicy
    case courseMaterials
    case termDates
    case course
}

@Observable @MainActor
public final class CourseDetailModel {
    /// A request to show the inspector at a section. `serial` makes a repeated request for the
    /// same section observable (the student scrolled away and clicked again).
    public struct InspectorRequest: Equatable, Sendable {
        public let section: CourseInspectorSection
        public let serial: Int
    }

    public let courseId: String
    /// `course_overview`: recent announcements (the rest of the page uses the summary).
    public private(set) var overview: CourseLoadable<CourseOverview> = .loading
    /// `week_materials` of the displayed week. While another week loads, this keeps the previous
    /// one (shown dimmed) and `isLoadingWeek` is true.
    public private(set) var week: CourseLoadable<WeekMaterials> = .loading
    public private(set) var isLoadingWeek = false
    /// `list_deadlines(course, 14, 7)`, split into coming up and recently past.
    public private(set) var deadlines: CourseLoadable<CourseDeadlines> = .loading
    public private(set) var inspectorRequest: InspectorRequest?
    /// The Explain section (M3) while the student is on the course: kept across section switches
    /// (a run goes on), dropped when they leave.
    public private(set) var explain: ExplainModel?
    @ObservationIgnored private var explainService = -1

    @ObservationIgnored private var weekGeneration = 0
    @ObservationIgnored private var overviewGeneration = 0
    @ObservationIgnored private var deadlinesGeneration = 0

    public init(courseId: String) {
        self.courseId = courseId
    }

    /// Loads everything the page shows (snapshots and tests; the page loads parts on their own).
    public func loadAll(using model: AppModel) async {
        async let week: Void = loadWeek(using: model)
        async let rest: Void = loadOverviewAndDeadlines(using: model)
        _ = await (week, rest)
    }

    /// What depends on the course's data but not on the displayed week.
    public func loadOverviewAndDeadlines(using model: AppModel) async {
        async let overview: Void = loadOverview(using: model)
        async let deadlines: Void = loadDeadlines(using: model)
        _ = await (overview, deadlines)
    }

    /// Loads the week the course's `CourseUIState` selects (nil = the current week). A stale
    /// answer (the student stepped on meanwhile) is dropped.
    public func loadWeek(using model: AppModel) async {
        weekGeneration += 1
        let generation = weekGeneration
        // Only another week dims the one on screen. The page reloads each time it appears; dimming
        // for a reload of the same week re-rendered the page twice for nothing.
        let requested = model.ui(for: courseId).selectedWeek
        if week.value?.requestedWeek != requested || week.value == nil { isLoadingWeek = true }
        let result: CourseLoadable<WeekMaterials>
        do throws(PageLampFailure) {
            result = .loaded(try await model.weekMaterials(for: courseId))
        } catch {
            result = .failed(error)
        }
        guard generation == weekGeneration else { return }
        week = result
        isLoadingWeek = false
    }

    public func loadOverview(using model: AppModel) async {
        overviewGeneration += 1
        let generation = overviewGeneration
        let service = model.service
        let courseId = self.courseId
        let result: CourseLoadable<CourseOverview>
        do throws(PageLampFailure) {
            result = .loaded(try await service.courseOverview(course: courseId))
        } catch {
            result = .failed(error)
        }
        guard generation == overviewGeneration else { return }
        overview = result
    }

    public func loadDeadlines(using model: AppModel) async {
        deadlinesGeneration += 1
        let generation = deadlinesGeneration
        let service = model.service
        let courseId = self.courseId
        let result: CourseLoadable<CourseDeadlines>
        do throws(PageLampFailure) {
            let list = try await service.listDeadlines(
                course: courseId,
                daysAhead: CourseDeadlines.daysAhead,
                daysBack: CourseDeadlines.daysBack
            )
            result = .loaded(CourseDeadlines(list, now: model.clock()))
        } catch {
            result = .failed(error)
        }
        guard generation == deadlinesGeneration else { return }
        deadlines = result
    }

    /// The Explain section's model, made on first use (and again for a new service: Debug ▸
    /// Data Source, the live facade opening).
    public func explainModel(using model: AppModel) -> ExplainModel {
        if let explain, explainService == model.serviceGeneration { return explain }
        explain?.leave()
        let fresh = ExplainModel(courseId: courseId, service: model.service)
        explain = fresh
        explainService = model.serviceGeneration
        return fresh
    }

    /// The student left the course: a run in flight stops, and the section starts afresh next
    /// time (like the Tauri app's tab).
    public func leaveExplain() {
        explain?.leave()
        explain = nil
    }

    /// "AI Policy…", "Set Term Dates…", the AI status line: opens the inspector at `section`.
    public func showInspector(_ section: CourseInspectorSection, in model: AppModel) {
        model.inspectorShown = true
        inspectorRequest = InspectorRequest(section: section, serial: (inspectorRequest?.serial ?? 0) + 1)
    }
}

/// A course's deadlines as the Deadlines section shows them (spec §3.2, W3a): class meetings
/// are calendar events, not deadlines, so they are left out.
public struct CourseDeadlines: Equatable, Sendable {
    /// "Coming up": due in the next 14 days.
    public static let daysAhead: UInt32 = 14
    /// "Recently past": due in the last 7 days.
    public static let daysBack: UInt32 = 7

    /// Soonest first.
    public let upcoming: [Deadline]
    /// Most recent first.
    public let past: [Deadline]

    public init(_ deadlines: [Deadline], now: Date) {
        let dated = deadlines
            .filter { $0.event.kind != .classEvent }
            .compactMap { deadline in Self.when(deadline).map { (deadline, $0) } }
            .sorted { $0.1 < $1.1 }
        upcoming = dated.filter { $0.1 >= now }.map(\.0)
        past = dated.filter { $0.1 < now }.reversed().map(\.0)
    }

    /// When a deadline is due (or an event starts).
    public static func when(_ deadline: Deadline) -> Date? {
        deadline.event.dueAt ?? deadline.event.startsAt
    }
}
