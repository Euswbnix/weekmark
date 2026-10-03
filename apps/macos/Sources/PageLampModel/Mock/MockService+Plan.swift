// Study plans written by PageLamp in the mock (M3; design §5.1), like the Tauri mock's plan.ts: the
// request's limits, the AI gate, the run's stages (Stop between them), a draft laid out like the
// scheduler's (study days only, at most 4 h a day, the rest unscheduled), and accepting it.

import Foundation
import PageLampKit

extension MockService {
    public func generateStudyPlan(
        request: StudyPlanRequest, generationId: String, observer: any GenObserver
    ) async throws(PageLampFailure) -> GeneratedStudyPlan {
        await respond("generateStudyPlan")
        guard !db.features.runs.running.contains(generationId) else {
            throw PageLampFailure(kind: .busy, message: "This plan is being written.")
        }
        db.features.runs.running.insert(generationId)
        defer {
            db.features.runs.running.remove(generationId)
            db.features.runs.cancelled.remove(generationId)
        }
        return try await writePlan(request, generationId: generationId, observer: observer)
    }

    public func acceptStudyPlan(generationId: String) async throws(PageLampFailure) -> StoredStudyPlan {
        await respond("acceptStudyPlan")
        guard let draft = db.features.runs.planDrafts[generationId] else {
            throw PageLampFailure(kind: .notFound, message: "There is no study plan draft \(generationId).")
        }
        guard !db.features.runs.acceptedDrafts.contains(generationId) else {
            throw PageLampFailure(kind: .invalid, message: "This draft is already your plan.")
        }
        db.features.runs.acceptedDrafts.insert(generationId)
        let stored = StoredStudyPlan(
            id: (db.studyPlan?.id ?? 0) + 1, createdAt: now(), plan: draft.plan, origin: .pageLamp,
            generationId: generationId,
            // Kept with the plan (also after "Remove all AI data").
            aiLabel: AiLabel(
                backendLabel: draft.meta.backendLabel, model: draft.meta.model, createdAt: draft.meta.createdAt,
                onDevice: draft.meta.onDevice
            )
        )
        db.studyPlan = stored
        return stored
    }

    /// The courses a study plan covers (the facade's plan scope): without a list, every visible,
    /// active course; with one, the named courses that aren't hidden (a course that has ended is
    /// still planned; an unknown one isn't found). None: blocked with `noCourseToPlan`.
    func planCourses(_ wanted: [String]) throws(PageLampFailure) -> [MockCourse] {
        guard !wanted.isEmpty else {
            return db.courses.filter {
                !$0.course.hidden && MockCalendar.lifecycle($0.timeline, keptCurrentUntil: $0.keptCurrentUntil).isActive
            }
        }
        var chosen: [MockCourse] = []
        for reference in wanted {
            let course = db.courses[try courseIndex(reference)]
            if !course.course.hidden, !chosen.contains(where: { $0.course.id == course.course.id }) {
                chosen.append(course)
            }
        }
        return chosen
    }

    private func writePlan(
        _ request: StudyPlanRequest, generationId: String, observer: any GenObserver
    ) async throws(PageLampFailure) -> GeneratedStudyPlan {
        let limits = planLimits()
        let horizon = request.horizonDays ?? limits.defaultHorizonDays
        let hours = request.hoursPerWeek ?? limits.defaultHoursPerWeek
        let daysOff = Set(request.daysOff)
        guard (limits.minHorizonDays...limits.maxHorizonDays).contains(horizon) else {
            throw PageLampFailure(kind: .invalid, message: "The plan covers 1 to 56 days.")
        }
        guard (limits.minHoursPerWeek...limits.maxHoursPerWeek).contains(hours) else {
            throw PageLampFailure(kind: .invalid, message: "Study hours per week are 1 to 80.")
        }
        guard daysOff.count < 7 else {
            throw PageLampFailure(kind: .invalid, message: "Leave at least one study day.")
        }
        // The gate says `noCourseToPlan` (after `noModelChosen`) for an empty scope, as the facade does.
        let run = try aiRun(
            .studyPlan(horizonDays: horizon, courses: request.courses), overrideBudget: request.overrideBudget
        )
        let courses = try planCourses(request.courses)
        observer.onEvent(event: .started(
            generationId: generationId, backendLabel: run.backendLabel, model: run.model, onDevice: run.onDevice
        ))
        let stages: [GenStage] = [.buildingContext, .waitingForModel, .scheduling]
        for (index, stage) in stages.enumerated() {
            observer.onEvent(event: .stage(stage: stage))
            await generationStep(generationId, UInt32(index))
            if db.features.runs.cancelled.contains(generationId) {
                observer.onEvent(event: .finished(ok: false))
                throw PageLampFailure(kind: .cancelled, message: "The plan was stopped.")
            }
        }
        let usage = TokenUsage(inputTokens: 9_800, cachedInputTokens: 0, outputTokens: 1_600, reasoningTokens: nil)
        observer.onEvent(event: .usage(usage: usage))
        observer.onEvent(event: .finished(ok: true))

        let today = calendar.startOfDay(for: now())
        let days = (0..<Int(horizon)).compactMap { calendar.date(byAdding: .day, value: $0, to: today) }
        let studyDays = days.filter { !daysOff.contains(Self.dayOfWeek($0, calendar: calendar)) }
        let perDay = min(240, Int((Double(hours) * 60 / Double(7 - daysOff.count)).rounded()))

        // The "model's" tasks: each course's materials to read, then a review before its deadline.
        struct PlannedTask {
            let courseId: String
            let title: String
            let materialIds: [String]
            let minutes: Int
        }
        let tasks = courses.flatMap { course in
            course.materials.prefix(3).map {
                PlannedTask(courseId: course.course.id, title: "Read \($0.title)", materialIds: [$0.id], minutes: 45)
            } + [PlannedTask(courseId: course.course.id, title: "Review this week's notes", materialIds: [], minutes: 30)]
        }
        var items: [StudyPlanItem] = []
        var unscheduled: [UnscheduledTask] = []
        var used: [String: Int] = [:]
        var day = 0
        for task in tasks {
            while day < studyDays.count,
                  used[IsoDate.string(from: studyDays[day], calendar: calendar), default: 0] + task.minutes > perDay {
                day += 1
            }
            guard day < studyDays.count else {
                unscheduled.append(UnscheduledTask(
                    title: task.title, courseId: task.courseId,
                    reason: studyDays.isEmpty ? .noStudyDays : .noTimeBeforeLatest
                ))
                continue
            }
            let iso = IsoDate.string(from: studyDays[day], calendar: calendar)
            used[iso, default: 0] += task.minutes
            items.append(StudyPlanItem(
                date: iso, courseId: task.courseId, title: task.title, description: nil,
                materialIds: task.materialIds, minutes: UInt32(task.minutes), done: false
            ))
        }

        let context = courses.map { course in
            let state = Self.aiMaterials(course.course)
            return ContextCourse(courseId: course.course.id, state: state, textIncluded: state == .readable)
        }
        let included = courses.filter { Self.aiMaterials($0.course) == .readable }
            .reduce(0) { $0 + min(3, $1.materials.count) }
        let draft = GeneratedStudyPlan(
            meta: GenerationMeta(
                generationId: generationId, feature: .studyPlan, backendLabel: run.backendLabel, model: run.model,
                onDevice: run.onDevice, createdAt: now(), usage: usage,
                estCostMicroUsd: run.onDevice ? nil : 4_200, estimated: false,
                context: ContextSummary(
                    courses: context, materialsIncluded: UInt32(included), materialsTrimmed: 0, leftOut: []
                ),
                promptVersion: 1
            ),
            plan: StudyPlan(
                horizonStart: IsoDate.string(from: today, calendar: calendar),
                horizonEnd: IsoDate.string(from: days.last ?? today, calendar: calendar),
                items: items, notes: nil
            ),
            unscheduled: unscheduled,
            // The "model" also proposed doing an assignment itself: dropped, only counted.
            warnings: [PlanWarning(code: .gradedWorkLeftOut, count: 1)]
        )
        db.features.runs.planDrafts[generationId] = draft
        return draft
    }

    private static func dayOfWeek(_ date: Date, calendar: Calendar) -> DayOfWeek {
        let days: [DayOfWeek] = [.sunday, .monday, .tuesday, .wednesday, .thursday, .friday, .saturday]
        return days[calendar.component(.weekday, from: date) - 1]
    }
}
