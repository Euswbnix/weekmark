// Weekly notes in the mock (M3; design §5.3), like the Tauri mock's weeklyNote.ts where it follows
// the facade: the note written from the courses' structure, the deadlines of the next 7 days and
// the plan (never a material's text), the AI gate (Monday's run never goes over the budget), the
// run's stages with Stop between them, the last 5 kept, and "prepare it when I open PageLamp on
// Monday": due on a Monday in the calendar's zone, with the opt-in on, a provider for the note
// (an API key or a model on this computer), no try yet that day and something to write about, the
// try recorded as the run starts so a failed, blocked or stopped one still uses up the Monday (a
// week with nothing to write about isn't due, so it keeps its try). Nothing to write about is
// blocked with `nothingToWrite`, which the estimate says first, after no model chosen. Like the
// facade: the courses keep their own AI state, a note is dated when its run starts, and removing
// AI data or a course's generated content drops its notes.

import Foundation
import PageLampKit

extension MockService {
    /// Notes kept (the facade keeps the latest 5 accepted).
    private static let notesKept = 5
    /// At most this many things to focus on.
    private static let focusItems = 3
    /// The pretend context and answer, in tokens.
    private static let noteInputTokens: UInt64 = 2_400
    private static let noteOutputTokens: UInt64 = 420
    /// What a cloud run costs, in micro-USD (nothing is counted on this computer).
    private static let noteCost: UInt64 = 600

    public func writeWeeklyNote(
        generationId: String, options: WeeklyNoteOptions, observer: any GenObserver
    ) async throws(PageLampFailure) -> WeeklyNote {
        await respond("writeWeeklyNote")
        guard !db.features.runs.running.contains(generationId) else {
            throw PageLampFailure(kind: .busy, message: "This note is being written.")
        }
        db.features.runs.running.insert(generationId)
        defer {
            db.features.runs.running.remove(generationId)
            db.features.runs.cancelled.remove(generationId)
        }
        // Monday's run: due now, and the try is recorded before anything else (no `await` since
        // the check, so another call can't slip in between).
        if options.automatic {
            let today = now()
            guard noteDue(at: today) else {
                throw PageLampFailure(kind: .invalid, message: "The weekly note isn't due to be prepared now.")
            }
            db.features.noteTriedOn = IsoDate.string(from: today, calendar: calendar)
        }
        return try await writeNote(generationId: generationId, options: options, observer: observer)
    }

    public func weeklyNotes() async throws(PageLampFailure) -> [WeeklyNote] {
        await respond("weeklyNotes")
        return db.features.runs.notes
    }

    public func deleteWeeklyNote(generationId: String) async throws(PageLampFailure) {
        await respond("deleteWeeklyNote")
        guard let index = db.features.runs.notes.firstIndex(where: { $0.meta.generationId == generationId }) else {
            throw PageLampFailure(kind: .notFound, message: "There is no weekly note \(generationId).")
        }
        db.features.runs.notes.remove(at: index)
    }

    public func weeklyNoteSettings() async throws(PageLampFailure) -> WeeklyNoteSettings {
        await respond("weeklyNoteSettings")
        return noteSettings()
    }

    public func setPrepareWeeklyNoteOnMonday(on: Bool) async throws(PageLampFailure) -> WeeklyNoteSettings {
        await respond("setPrepareWeeklyNoteOnMonday")
        if on && !noteSettings().prepareOnMondayAllowed {
            throw PageLampFailure(
                kind: .invalid,
                message: "Preparing the note on Monday needs an API key or a model on this computer for weekly notes."
            )
        }
        db.features.prepareNoteOnMonday = on
        return noteSettings()
    }

    // MARK: - Monday

    /// Only a provider may prepare it (an API key or a model on this computer, plan D27); the
    /// opt-in is kept when the model changes, and then shows as paused.
    private func noteSettings() -> WeeklyNoteSettings {
        var allowed = false
        if case .provider = db.features.ai.routing[.weeklyNote]?.backend {
            allowed = true
        }
        return WeeklyNoteSettings(prepareOnMonday: db.features.prepareNoteOnMonday, prepareOnMondayAllowed: allowed)
    }

    /// Whether Monday's note is due at `date` (`startupTasks(now:)` and Monday's run alike): a
    /// Monday in the calendar's zone (its wall clock: daylight saving doesn't move it), the opt-in
    /// on and allowed, no try yet that day, no note written that day (dated by its run's start), and
    /// something to write about.
    func noteDue(at date: Date) -> Bool {
        let settings = noteSettings()
        let day = IsoDate.string(from: date, calendar: calendar)
        let writtenToday = db.features.runs.notes.first.map {
            IsoDate.string(from: $0.meta.createdAt, calendar: calendar) == day
        } ?? false
        // The weekly-note-on-Monday scenario: every day is a Monday.
        let monday = scenario == .weeklyNoteMonday || calendar.component(.weekday, from: date) == 2
        return settings.prepareOnMonday && settings.prepareOnMondayAllowed && monday
            && db.features.noteTriedOn != day && !writtenToday && !noteWeek(at: date).isEmpty
    }

    /// What the note writes about at `date` (the facade's note context): the active courses that
    /// show, the next 7 days' deadlines and the plan's items of the last week, a hidden or removed
    /// course's left out.
    struct NoteWeek {
        var active: [MockCourse]
        var due: [Deadline]
        var planItems: [StudyPlanItem]
        /// Nothing at all: nothing to write about (`nothingToWrite`).
        var isEmpty: Bool { active.isEmpty && due.isEmpty && planItems.isEmpty }
    }

    func noteWeek(at date: Date) -> NoteWeek {
        let courses = db.courses.filter { !$0.course.hidden }
        let active = courses.filter {
            MockCalendar.lifecycle($0.timeline, keptCurrentUntil: $0.keptCurrentUntil).isActive
        }
        let today = IsoDate.string(from: date, calendar: calendar)
        let weekAgo = IsoDate.string(from: date.addingTimeInterval(-7 * 86_400), calendar: calendar)
        let leftOut = Set(db.courses.filter { $0.course.hidden }.map { $0.course.id } + db.features.removed.map { $0.course.course.id })
        let planItems = db.studyPlan?.plan.items.filter { item in
            item.date >= weekAgo && item.date <= today && !(item.courseId.map { leftOut.contains($0) } ?? false)
        } ?? []
        return NoteWeek(
            active: active, due: deadlines(in: courses, daysAhead: 7, daysBack: 0, at: date), planItems: planItems
        )
    }

    /// The Monday of `date`'s week, "YYYY-MM-DD" (whatever day the calendar's weeks start on).
    func noteWeekOf(_ date: Date) -> String {
        let day = calendar.startOfDay(for: date)
        let sinceMonday = (calendar.component(.weekday, from: day) + 5) % 7
        let monday = calendar.date(byAdding: .day, value: -sinceMonday, to: day) ?? day
        return IsoDate.string(from: monday, calendar: calendar)
    }

    // MARK: - A run

    private func writeNote(
        generationId: String, options: WeeklyNoteOptions, observer: any GenObserver
    ) async throws(PageLampFailure) -> WeeklyNote {
        guard db.features.ai.routing[.weeklyNote] != nil else {
            throw PageLampFailure(kind: .blocked, message: "Choose a model for weekly notes first.", blocked: .noModelChosen)
        }
        // The week from its structure (`noteWeek`). Nothing at all: nothing to write about,
        // blocked before anything is sent (the estimate said so first).
        let started = now()
        let week = noteWeek(at: started)
        guard !week.isEmpty else {
            throw PageLampFailure(
                kind: .blocked,
                message: "There is nothing to write about this week: no active course, no deadline in the next 7 days and no study plan item.",
                blocked: .nothingToWrite
            )
        }
        let active = week.active
        let due = week.due
        let run = try aiRun(.weeklyNote, overrideBudget: options.overrideBudget && !options.automatic)
        // The "model's" note: what's due (not the classes) and the first things to get ahead on.
        let dueWork = due.filter { $0.event.kind != .classEvent }

        let summary = ContextSummary(
            courses: active.map {
                ContextCourse(courseId: $0.course.id, state: Self.aiMaterials($0.course), textIncluded: false)
            },
            materialsIncluded: 0, materialsTrimmed: 0, leftOut: []
        )
        let usage = TokenUsage(
            inputTokens: Self.noteInputTokens, cachedInputTokens: 0, outputTokens: Self.noteOutputTokens, reasoningTokens: nil
        )
        observer.onEvent(event: .stage(stage: .buildingContext))
        try await noteStep(generationId, 0, observer)
        observer.onEvent(event: .context(summary: summary, inputTokens: Self.noteInputTokens))
        observer.onEvent(event: .started(
            generationId: generationId, backendLabel: run.backendLabel, model: run.model, onDevice: run.onDevice
        ))
        observer.onEvent(event: .stage(stage: .waitingForModel))
        try await noteStep(generationId, 1, observer)
        observer.onEvent(event: .usage(usage: usage))
        observer.onEvent(event: .stage(stage: .validating))
        try await noteStep(generationId, 2, observer)
        observer.onEvent(event: .finished(ok: true))

        let note = WeeklyNote(
            meta: GenerationMeta(
                generationId: generationId, feature: .weeklyNote, backendLabel: run.backendLabel, model: run.model,
                onDevice: run.onDevice, createdAt: started, usage: usage,
                estCostMicroUsd: run.onDevice ? nil : Self.noteCost, estimated: false, context: summary, promptVersion: 1
            ),
            weekOf: noteWeekOf(started),
            text: "This week you have \(dueWork.count) deadlines across \(active.count) active courses. Start with the "
                + "earliest one, and keep an hour for this week's readings. Your study plan is on track so far.",
            focus: dueWork.prefix(Self.focusItems).map {
                NoteFocus(text: "Get ahead on \($0.event.title) before it's due.", courseId: $0.event.courseId)
            },
            automatic: options.automatic,
            gradedWorkLeftOut: 0
        )
        db.features.runs.notes.insert(note, at: 0)
        db.features.runs.notes = Array(db.features.runs.notes.prefix(Self.notesKept))
        return note
    }

    /// A run's step, then Stop: a run asked to stop ends here, cancelled (nothing is kept; Monday's
    /// try stays used).
    private func noteStep(_ generationId: String, _ step: UInt32, _ observer: any GenObserver) async throws(PageLampFailure) {
        await generationStep(generationId, step)
        if db.features.runs.cancelled.contains(generationId) {
            observer.onEvent(event: .finished(ok: false))
            throw PageLampFailure(kind: .cancelled, message: "The weekly note was stopped.")
        }
    }
}
