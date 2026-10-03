// The words of a course's Explain section (the shared `explain.*` strings): the week choices and
// Generate, the run's headline, stage and materials count, the result's labels, what was left
// out and why, the history's times, and the text Copy puts on the clipboard (with the AI label).

import Foundation
import PageLampKit

public struct ExplainText: Sendable {
    public let l10n: L10n
    public let calendar: Calendar

    public init(l10n: L10n, calendar: Calendar) {
        self.l10n = l10n
        self.calendar = calendar
    }

    // MARK: The week

    /// "Week 4 (this week)" for the current week, else "Week 4".
    public func weekOption(_ week: UInt32, current: UInt32?) -> String {
        l10n(week == current ? "explain.thisWeek" : "explain.weekOption", ["week": l10n.number(week)])
    }

    /// Generate's title: "Explain Week 4", or "Explain the Recent Materials" without a week.
    public func generate(_ week: UInt32?) -> String {
        guard let week else { return l10n("mac.explain.generateRecent") }
        return l10n("mac.explain.generate", ["week": l10n.number(week)])
    }

    // MARK: The run

    /// "Writing with OpenAI · gpt-5.4-mini" once the run has said which, else "Starting…".
    public func headline(_ progress: GenerationRun<WeeklyExplanation>.Progress) -> String {
        guard let backend = progress.backend, let model = progress.model else { return l10n("explain.running.starting") }
        return l10n("explain.running.withModel", ["backend": backend, "model": model])
    }

    /// "Reading 2 materials".
    public func reading(_ count: UInt32) -> String {
        l10n.plural("explain.running.reading", count: Int(count))
    }

    /// The stage ("Checking the citations"), or "Writing the explanation" before the first.
    public func stage(_ stage: GenStage?) -> String {
        stage.map { l10n("explain.running.stage.\(AiCodes.name($0))") } ?? l10n("explain.running.writingNow")
    }

    // MARK: The result

    /// The result's VoiceOver name: "Explanation of week 4", or of the recent materials.
    public func regionLabel(_ week: UInt32?) -> String {
        guard let week else { return l10n("explain.result.regionLabelRecent") }
        return l10n("explain.result.regionLabel", ["week": l10n.number(week)])
    }

    /// A citation chip: "Week 4 slides, p. 2", or the title alone.
    public func citation(_ citation: Citation) -> String {
        guard let locator = citation.locator, !locator.isEmpty else { return citation.title }
        return l10n("explain.result.citation", ["title": citation.title, "locator": locator])
    }

    /// "looks like graded work".
    public func reason(_ reason: LeftOutReason) -> String {
        l10n("explain.result.leftOutReason.\(AiCodes.name(reason))")
    }

    /// "Not Graded Work? Include It and Write Again", or "… Include These 2 …" (not a plural key:
    /// Chinese has one form, and "it" needs no number).
    public func include(_ count: Int) -> String {
        count == 1
            ? l10n("mac.explain.includeOne")
            : l10n("mac.explain.includeSeveral", ["count": l10n.number(count)])
    }

    /// "2 citations the model gave didn't match your materials and were removed."
    public func droppedCitations(_ count: UInt32) -> String {
        l10n.plural("explain.result.droppedCitations", count: Int(count))
    }

    // MARK: The history

    /// "Fri, Sep 25 at 10:05 AM" (a saved explanation's time, as the system writes it).
    public func savedAt(_ date: Date) -> String {
        date.formatted(
            Date.FormatStyle(locale: l10n.locale, calendar: calendar, timeZone: calendar.timeZone)
                .weekday(.abbreviated).month(.abbreviated).day().hour().minute()
        )
    }

    /// The history row's button for VoiceOver: "Show the explanation from Fri, Sep 25 at 10:05 AM".
    public func showFrom(_ date: Date) -> String {
        l10n("mac.explain.showFrom", ["when": savedAt(date)])
    }

    // MARK: Copy

    /// What Copy puts on the clipboard (the Tauri app's text, word for word): each section's
    /// heading, each paragraph with its sources ("text [title, locator; title]"), the check
    /// questions numbered, then the AI label; blocks apart by a blank line. The paragraphs keep
    /// their Markdown as written.
    public func copyText(_ explanation: WeeklyExplanation) -> String {
        var blocks: [String] = []
        for section in explanation.sections {
            blocks.append(section.heading)
            for paragraph in section.paragraphs {
                let sources = paragraph.citations.map { citation in
                    if let locator = citation.locator, !locator.isEmpty { "\(citation.title), \(locator)" } else { citation.title }
                }
                blocks.append(sources.isEmpty ? paragraph.text : "\(paragraph.text) [\(sources.joined(separator: "; "))]")
            }
        }
        if !explanation.checkQuestions.isEmpty {
            blocks.append(
                explanation.checkQuestions.enumerated().map { "\($0.offset + 1). \($0.element)" }.joined(separator: "\n")
            )
        }
        blocks.append(l10n.aiLabel(explanation.meta, calendar: calendar))
        return blocks.joined(separator: "\n\n")
    }
}
