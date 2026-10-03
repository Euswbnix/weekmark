// Bridges the progress of model runs, syllabus batches and the Codex install and sign-in from
// PageLamp's worker threads to any actor, like `SyncEventStream` does for syncs.

/// An observer that forwards every event into an `AsyncStream`. PageLamp calls it off the run
/// (a slow consumer never holds a generation up), and `onEvent` only buffers, so it never
/// blocks either. Stop a run with `cancelGeneration(generationId:)` (or `cancelCodexInstall` /
/// `cancelCodexLogin`), not by dropping the stream: events after `finish()` are discarded.
///
/// ```swift
/// let progress = GenEventStream()
/// Task { @MainActor in for await event in progress.events { apply(event) } }
/// defer { progress.finish() }
/// let explanation = try await pageLamp.explainWeek(
///     course: id, week: nil, generationId: runId, options: ExplainOptions(), observer: progress)
/// ```
public final class EventStream<Event: Sendable>: Sendable {
    /// Every event of the run, in order; ends after `finish()`.
    public let events: AsyncStream<Event>
    private let continuation: AsyncStream<Event>.Continuation

    public init() {
        (events, continuation) = AsyncStream.makeStream(of: Event.self, bufferingPolicy: .unbounded)
    }

    func yield(_ event: Event) {
        continuation.yield(event)
    }

    /// Ends `events`; call once the call has returned or thrown.
    public func finish() {
        continuation.finish()
    }
}

/// `explainWeek`, `generateStudyPlan`, `readCourseCalendar`.
public typealias GenEventStream = EventStream<GenEvent>
/// `readCourseCalendars`.
public typealias CalendarBatchEventStream = EventStream<CalendarBatchEvent>
/// `installCodex`.
public typealias CodexInstallEventStream = EventStream<RuntimeEvent>
/// `codexLogin`.
public typealias CodexLoginEventStream = EventStream<LoginEvent>

extension EventStream: GenObserver where Event == GenEvent {
    public func onEvent(event: GenEvent) { yield(event) }
}

extension EventStream: CalendarBatchObserver where Event == CalendarBatchEvent {
    public func onEvent(event: CalendarBatchEvent) { yield(event) }
}

extension EventStream: CodexInstallObserver where Event == RuntimeEvent {
    public func onEvent(event: RuntimeEvent) { yield(event) }
}

extension EventStream: CodexLoginObserver where Event == LoginEvent {
    public func onEvent(event: LoginEvent) { yield(event) }
}
