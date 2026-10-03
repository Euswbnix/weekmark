// The words of the weekly note (the shared `weeklyNote.*` strings): the run's headline and stage,
// the note's week, the focus list, what was left out, the history's times, and the text Copy puts
// on the clipboard (with the AI label), as the Tauri app's card has them.

import Foundation
import PageLampKit

public struct NoteText: Sendable {
    public let l10n: L10n
    public let calendar: Calendar

    public init(l10n: L10n, calendar: Calendar) {
        self.l10n = l10n
        self.calendar = calendar
    }

    // MARK: The run

    /// "Preparing Monday's note" for Monday's run; else "Writing with OpenAI · gpt-6-luna" once
    /// the run has said which, else "Starting…".
    public func headline(_ progress: GenerationRun<WeeklyNote>.Progress, automatic: Bool) -> String {
        if automatic { return l10n("weeklyNote.running.automatic") }
        guard let backend = progress.backend, let model = progress.model else { return l10n("weeklyNote.running.starting") }
        return l10n("weeklyNote.running.withModel", ["backend": backend, "model": model])
    }

    /// The stage ("Gathering your week"), or "Writing your note" before the first.
    public func stage(_ stage: GenStage?) -> String {
        stage.map { l10n("weeklyNote.running.stage.\(AiCodes.name($0))") } ?? l10n("weeklyNote.running.writingNow")
    }

    // MARK: The note

    /// "Sep 28, 2026" (the note's Monday).
    public func date(_ weekOf: String) -> String {
        guard let day = IsoDate.date(from: weekOf, calendar: calendar) else { return weekOf }
        return day.formatted(Date.FormatStyle(date: .abbreviated, time: .omitted, locale: l10n.locale, calendar: calendar, timeZone: calendar.timeZone))
    }

    /// "For the week of Sep 28, 2026", then " · Prepared on Monday" for Monday's note.
    public func description(_ note: WeeklyNote) -> String {
        let week = l10n("weeklyNote.weekOf", ["date": date(note.weekOf)])
        return note.automatic ? "\(week) · \(l10n("weeklyNote.automatic"))" : week
    }

    /// The note's VoiceOver name: "Weekly note for the week of Sep 28, 2026".
    public func regionLabel(_ note: WeeklyNote) -> String {
        l10n("weeklyNote.regionLabel", ["date": date(note.weekOf)])
    }

    /// "DEMO101: Get ahead on Quiz 3 before it's due.", or the text alone when its course isn't
    /// known here.
    public func focus(_ item: NoteFocus, course: (String) -> String?) -> String {
        guard let id = item.courseId, let name = course(id) else { return item.text }
        return "\(name): \(item.text)"
    }

    /// "1 focus item was left out because it looked like an answer to graded work."
    public func gradedLeftOut(_ count: UInt32) -> String {
        l10n.plural("weeklyNote.gradedLeftOut", count: Int(count))
    }

    /// Whether the note is for a week before `now`'s (its Monday is earlier than this Monday).
    public func isEarlierWeek(_ note: WeeklyNote, now: Date) -> Bool {
        let today = calendar.startOfDay(for: now)
        let sinceMonday = (calendar.component(.weekday, from: today) + 5) % 7
        guard let monday = calendar.date(byAdding: .day, value: -sinceMonday, to: today) else { return false }
        return note.weekOf < IsoDate.string(from: monday, calendar: calendar)
    }

    // MARK: The history

    /// "Mon, Sep 28 at 10:00 AM" (when a kept note was written, as the system writes it).
    public func savedAt(_ date: Date) -> String {
        date.formatted(
            Date.FormatStyle(locale: l10n.locale, calendar: calendar, timeZone: calendar.timeZone)
                .weekday(.abbreviated).month(.abbreviated).day().hour().minute()
        )
    }

    // MARK: Settings

    /// The Monday opt-in's cost line: "With Ollama · qwen3.5:9b, on this computer." (a model on
    /// this computer, as its facts say), "…, counted toward your monthly budget (no price for this
    /// model)." without a price, "… (what it costs depends on the week)." in a week with nothing
    /// to write about, else "…: ≈ $0.01 at most each Monday, …"; nil without an amount.
    public func mondayCost(_ cost: AiSettingsModel.NoteCost) -> String? {
        let names = ["backend": cost.backend, "model": cost.model]
        if cost.onDevice { return l10n("weeklyNote.settings.costLocal", names) }
        if !cost.priceKnown { return l10n("weeklyNote.settings.costUnpriced", names) }
        if cost.weekEmpty { return l10n("weeklyNote.settings.costWeekEmpty", names) }
        guard let upper = cost.upper else { return nil }
        return l10n("weeklyNote.settings.costKey", names.merging(["cost": l10n.estimateAmount(microUsd: upper)]) { $1 })
    }

    // MARK: Copy

    /// What Copy puts on the clipboard (the Tauri app's text): the note, the focus list under its
    /// title, numbered, with each item's course, then the AI label; blocks apart by a blank line,
    /// an empty focus list left out.
    public func copyText(_ note: WeeklyNote, course: (String) -> String?) -> String {
        var blocks = [note.text]
        if !note.focus.isEmpty {
            let items = note.focus.enumerated().map { "\($0.offset + 1). \(focus($0.element, course: course))" }
            blocks.append(([l10n("weeklyNote.focusTitle")] + items).joined(separator: "\n"))
        }
        blocks.append(l10n.aiLabel(note.meta, calendar: calendar))
        return blocks.filter { !$0.isEmpty }.joined(separator: "\n\n")
    }
}
