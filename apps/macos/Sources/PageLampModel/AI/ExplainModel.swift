// A course's Explain section (design §5.2, §7; the Tauri app's ExplainTab and useExplanation):
// the week to explain, "≈ $x" and the facade's block (for Generate, and for Include It and Write
// Again priced with the include each sends), the run with its stages and Stop, the
// explanation shown (the run's new one, or one the student picked from the history), Delete, and
// the one-time question about sharing the course's materials. It lives as long as the student
// is on the course (the section can be left and come back to while a run goes on); leaving the
// course stops a run in flight. The facade decides everything; this keeps the student's choices.

import Foundation
import Observation
import PageLampKit

@Observable @MainActor
public final class ExplainModel {
    /// The week picker's choice: a numbered week, or the recent materials (no week).
    public enum WeekChoice: Hashable, Sendable {
        case week(UInt32)
        case recent
    }

    /// Why this course can't be explained (the section shows why instead of Generate).
    public enum Block: String, Sendable {
        case courseHidden = "course_hidden"
        case turnedOff = "turned_off"
        case withheldByPolicy = "withheld_by_policy"
    }

    public let courseId: String
    public let run: GenerationRun<WeeklyExplanation>
    /// "≈ $x" for the week (Generate, and Write Again of an explanation written without include).
    public let estimate: CostEstimateModel
    /// "≈ $x" for Include It and Write Again, priced with the materials it sends
    /// (`prepare(for:week:)` sets it to the explanation on screen).
    public let includeEstimate: CostEstimateModel
    /// "≈ $x" for Write Again of an explanation written with include (the same include again).
    public let againEstimate: CostEstimateModel

    /// The numbered week the student picked; nil = the course's default week, or the recent
    /// materials while the course has none (like the Tauri app, "Recent materials" isn't kept:
    /// once the course knows its weeks, its default week shows).
    public private(set) var picked: WeekChoice?
    /// The saved explanations of the week on screen, newest first.
    public private(set) var saved: [WeeklyExplanation] = []
    /// The explanation picked in the history; nil = the run's new one, else the newest.
    public private(set) var shownId: String?
    /// Deleted here: gone at once, before the saved list is read again.
    public private(set) var deleted: Set<String> = []
    public private(set) var deleting = false
    public private(set) var deleteFailure: PageLampFailure?
    /// The explanation whose sharing question the student closed (not kept).
    public private(set) var reminderClosed: String?
    public private(set) var savingSharing = false
    public private(set) var sharingFailure: PageLampFailure?

    @ObservationIgnored private let service: any PageLampService
    @ObservationIgnored private var savedLoads = 0
    /// The include each run here sent, by its explanation (Write Again sends it again; Include It
    /// adds to it). Saved explanations from before don't say: none.
    @ObservationIgnored private var sentIncludes: [String: [String]] = [:]
    /// The week the section shows (`load(week:)`): a list read for another week is dropped.
    @ObservationIgnored private var shownWeek: UInt32??

    public init(
        courseId: String,
        service: any PageLampService,
        debounce: Duration = .milliseconds(300),
        newId: @escaping @Sendable () -> String = randomGenerationId
    ) {
        self.courseId = courseId
        self.service = service
        run = GenerationRun(service: service, newId: newId)
        estimate = CostEstimateModel(service: service, debounce: debounce)
        includeEstimate = CostEstimateModel(service: service, debounce: debounce)
        againEstimate = CostEstimateModel(service: service, debounce: debounce)
    }

    // MARK: - Which week

    /// Why the course can't be explained, from its summary (the Tauri app's rule): hidden first,
    /// then whether the AI may read its materials.
    public static func block(_ summary: CourseSummary) -> Block? {
        if summary.course.hidden { return .courseHidden }
        switch summary.aiMaterials {
        case .readable: return nil
        case .turnedOff: return .turnedOff
        case .withheldByPolicy: return .withheldByPolicy
        }
    }

    /// The week explained: the picked one, else the course's default week (its current teaching
    /// week, or the last one before a break), else nil (the recent materials).
    public func week(_ timeline: CourseTimeline) -> UInt32? {
        switch picked {
        case .week(let week)?: week
        case .recent?, nil: timeline.defaultWeek ?? timeline.currentWeek
        }
    }

    /// The picker's choices: "Recent materials" first when the course has no default week (a
    /// week with a default is explained by number), then the weeks with materials.
    public static func choices(_ timeline: CourseTimeline, weeks: [UInt32]) -> [WeekChoice] {
        let defaultWeek = timeline.defaultWeek ?? timeline.currentWeek
        var weeks = weeks
        if weeks.isEmpty, let defaultWeek { weeks = [defaultWeek] }
        return (defaultWeek == nil ? [.recent] : []) + weeks.map { .week($0) }
    }

    public func pick(_ choice: WeekChoice) {
        guard !run.isRunning else { return }
        picked = choice == .recent ? nil : choice
        shownId = nil
    }

    /// Estimates the week and reads its saved explanations (each time the section shows a week).
    public func load(week: UInt32?) async {
        shownWeek = .some(week)
        estimate.update(.weeklyExplanation(course: courseId, week: week))
        await loadSaved(week: week)
    }

    /// Back from Settings ▸ AI or another app: the setup or the saved list may have changed.
    public func refresh(week: UInt32?) async {
        await estimate.refresh()
        await includeEstimate.refresh()
        await againEstimate.refresh()
        await loadSaved(week: week)
    }

    private func loadSaved(week: UInt32?) async {
        savedLoads += 1
        let load = savedLoads
        let list: [WeeklyExplanation]
        do throws(PageLampFailure) {
            list = try await service.savedExplanations(course: courseId, week: week)
        } catch {
            // Like the Tauri app: none to show (the run and Generate still work).
            list = []
        }
        // The latest read, of the week on screen (a run or a delete that ends after the shown
        // week changed reads its own week).
        guard load == savedLoads, shownWeek == .some(week) else { return }
        // Without a week the facade answers every week's: keep the recent materials' own.
        saved = week == nil ? list.filter { $0.week == nil } : list
    }

    // MARK: - What's shown

    /// The saved explanations, without those deleted here.
    public var history: [WeeklyExplanation] {
        saved.filter { !deleted.contains($0.meta.generationId) }
    }

    /// The explanation shown for `week`: the one picked in the history, else the run's new one
    /// (when it is this week's), else the newest saved.
    public func shown(week: UInt32?) -> WeeklyExplanation? {
        let history = self.history
        if let shownId, let picked = history.first(where: { $0.meta.generationId == shownId }) {
            return picked
        }
        if case .finished(let fresh) = run.phase, !deleted.contains(fresh.meta.generationId), fresh.week == week {
            return fresh
        }
        return history.first
    }

    public func show(_ explanation: WeeklyExplanation) {
        shownId = explanation.meta.generationId
    }

    // MARK: - The run

    /// Explain week `week` (Generate), from the week's "≈ $x".
    public func generate(week: UInt32?, uiLanguage: String) async {
        await start(week: week, include: [], from: estimate, uiLanguage: uiLanguage)
    }

    /// Include It and Write Again: `explanation`'s include plus the left-out materials the facade
    /// brings back, from that request's own "≈ $x" (`includeEstimate`).
    public func includeAndWriteAgain(_ explanation: WeeklyExplanation, week: UInt32?, uiLanguage: String) async {
        let include = includeIds(explanation)
        guard !include.isEmpty else { return }
        await start(week: week, include: include, from: includeEstimate, uiLanguage: uiLanguage)
    }

    /// Write Again (a stale explanation): the same include as its run, from that request's
    /// "≈ $x" (the week's when it had none).
    public func writeAgain(_ explanation: WeeklyExplanation, week: UInt32?, uiLanguage: String) async {
        let include = againIds(explanation)
        await start(week: week, include: include, from: againEstimate(for: explanation), uiLanguage: uiLanguage)
    }

    /// What Include It sends: what `explanation`'s run included, then the left-out materials the
    /// facade would bring back; empty when there are none of those.
    public func includeIds(_ explanation: WeeklyExplanation) -> [String] {
        let more = explanation.leftOut.filter { $0.includable }.map { $0.materialId }
        guard !more.isEmpty else { return [] }
        let sent = againIds(explanation)
        return sent + more.filter { !sent.contains($0) }
    }

    /// What Write Again sends: the include of `explanation`'s run (one written here), else none.
    public func againIds(_ explanation: WeeklyExplanation) -> [String] {
        sentIncludes[explanation.meta.generationId] ?? []
    }

    /// The "≈ $x" Write Again runs from: its own with an include, else the week's.
    public func againEstimate(for explanation: WeeklyExplanation) -> CostEstimateModel {
        againIds(explanation).isEmpty ? estimate : againEstimate
    }

    /// Prices Include It and Write Again for the explanation on screen (none: nothing to price).
    public func prepare(for explanation: WeeklyExplanation?, week: UInt32?) {
        let include = explanation.map(includeIds) ?? []
        includeEstimate.update(include.isEmpty ? nil : .weeklyExplanation(course: courseId, week: week, include: include))
        let again = explanation.map(againIds) ?? []
        againEstimate.update(again.isEmpty ? nil : .weeklyExplanation(course: courseId, week: week, include: again))
    }

    /// A run from `source` ("≈ $x" for exactly this request): going over the budget is chosen
    /// again for each run, so every line's tick goes with it (a tick left on another line would
    /// ride on its next run, the budget still reached).
    private func start(week: UInt32?, include: [String], from source: CostEstimateModel, uiLanguage: String) async {
        guard source.canGenerate, !run.isRunning else { return }
        let options = ExplainOptions(include: include, uiLanguage: uiLanguage, overrideBudget: source.goesOverBudget)
        let courseId = self.courseId
        shownId = nil
        for line in [estimate, includeEstimate, againEstimate] {
            line.overrideBudget = false
        }
        await run.run { service, id, observer async throws(PageLampFailure) in
            try await service.explainWeek(course: courseId, week: week, generationId: id, options: options, observer: observer)
        }
        if case .finished(let fresh) = run.phase {
            sentIncludes[fresh.meta.generationId] = include
        }
        // Usage, the budget and the saved list changed, whatever the end (a stopped run is billed).
        await refresh(week: week)
    }

    public func stop() async {
        await run.stop()
    }

    /// Whether the run in flight goes to a model on this computer (nil until it says, or when
    /// nothing runs): the course page checks the course again once it's known.
    public var runOnDevice: Bool? {
        guard case .running(let progress) = run.phase else { return nil }
        return progress.onDevice
    }

    /// Stops a run the course no longer allows: hidden, AI off or withheld (changed from another
    /// app), or a cloud run once its materials may not be shared. The facade stops its own runs;
    /// this is the Tauri app's second guard, for changes made elsewhere.
    public func stopIfRefused(_ summary: CourseSummary) async {
        guard case .running(let progress) = run.phase, !progress.stopping else { return }
        let sharingRefused = summary.course.materialSharing == .notAllowed && progress.onDevice == false
        if Self.block(summary) != nil || sharingRefused {
            await run.stop()
        }
    }

    /// The student left the course: a run in flight stops (nothing keeps writing, or costing,
    /// out of sight).
    public func leave() {
        run.cancelInFlight()
    }

    // MARK: - Delete

    /// Deletes an explanation (asked first). Returns whether it's gone; `deleteFailure` says why
    /// not.
    public func delete(_ explanation: WeeklyExplanation, week: UInt32?) async -> Bool {
        guard !deleting else { return false }
        deleting = true
        deleteFailure = nil
        defer { deleting = false }
        let id = explanation.meta.generationId
        do throws(PageLampFailure) {
            try await service.deleteExplanation(generationId: id)
        } catch {
            deleteFailure = error
            return false
        }
        deleted.insert(id)
        shownId = nil
        await loadSaved(week: week)
        return true
    }

    // MARK: - Sharing the course's materials (question (b))

    /// Whether `explanation` asks the one-time question (the first cloud run of a course whose
    /// answer is missing or "not sure"), until the student closes it.
    public func asksAboutSharing(_ explanation: WeeklyExplanation) -> Bool {
        explanation.sharingReminder && reminderClosed != explanation.meta.generationId
    }

    public func dismissSharingQuestion(_ explanation: WeeklyExplanation) {
        reminderClosed = explanation.meta.generationId
    }

    /// Saves the student's answer; the question closes. The caller reads the course again (its
    /// answer changes what may be sent).
    public func answerSharing(_ answer: MaterialSharing, for explanation: WeeklyExplanation) async -> Bool {
        guard !savingSharing else { return false }
        savingSharing = true
        sharingFailure = nil
        defer { savingSharing = false }
        do throws(PageLampFailure) {
            try await service.setCourseMaterialSharing(course: courseId, answer: answer)
        } catch {
            sharingFailure = error
            return false
        }
        reminderClosed = explanation.meta.generationId
        // What may be sent changed: every line is priced again (Include It and Write Again too).
        await estimate.refresh()
        await includeEstimate.refresh()
        await againEstimate.refresh()
        return true
    }
}
