// This Week (home) (spec §3.1, M1): the lamp band with today's date, the summary and Next up;
// then the crash notice (S6), source problems (S7), Next 7 days, the weekly note (M3, preview
// builds), the study plan and the Contents of the student's courses. S3/S4 (and the first sync) replace the page with an empty state
// (spec §3.9). The day ribbon is M3.
//
// One tinted action per window, and This Week normally has none (spec §1.2): a failing source's
// fix is the capsule's tinted bubble (the S7 callouts only lead to Sources & Sync), and only the
// empty states' primary action is a candidate on the page (`AppModel.thisWeekCandidates`).

import SwiftUI
import PageLampKit
import PageLampModel

struct ThisWeekView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n

    var body: some View {
        // Every minute: Next up's countdown, the day's rollover and the rolling range. Minutes,
        // never seconds (spec §6.5); nothing animates.
        TimelineView(.everyMinute) { _ in
            let now = model.clock()
            Group {
                if model.thisWeekPageState == .page {
                    ScrollView {
                        ThisWeekPage(now: now)
                    }
                    .scrollEdgeEffectStyle(.soft, for: .bottom)
                } else {
                    ThisWeekPage(now: now)
                }
            }
            .navigationSubtitle(rollingRange(from: now))
        }
        .accessoryBar()
        .navigationTitle(l10n("mac.nav.thisWeek"))
    }

    /// "Sep 26 – Oct 2": today and the next six days (rolling, so titled so).
    private func rollingRange(from now: Date) -> String {
        let start = model.calendar.startOfDay(for: now)
        let end = model.calendar.date(byAdding: .day, value: ThisWeekDigest.days - 1, to: start) ?? start
        return (start ..< end).formatted(
            Date.IntervalFormatStyle(locale: l10n.locale, calendar: model.calendar, timeZone: model.calendar.timeZone)
                .month(.abbreviated).day()
        )
    }
}

/// The page's document: rendered in the scroll view and by the snapshot harness.
package struct ThisWeekPage: View {
    /// "Now" for everything on the page; nil = `model.clock()` (snapshots).
    var now: Date?
    /// Opens Show Full Plan (snapshots of the full plan).
    var planExpanded = false

    @Environment(AppModel.self) private var model

    package init(now: Date? = nil, planExpanded: Bool = false) {
        self.now = now
        self.planExpanded = planExpanded
    }

    package var body: some View {
        let state = model.thisWeekPageState
        Group {
            switch state {
            case .page:
                ThisWeekDocument(now: now ?? model.clock(), planExpanded: planExpanded)
            case .noSources, .noCourses, .firstSync:
                ThisWeekEmptyPage(state: state)
            }
        }
        .primaryActionCandidates(model.thisWeekCandidates)
    }
}

/// The reading page: lamp band, then the reading column.
private struct ThisWeekDocument: View {
    let now: Date
    let planExpanded: Bool

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n

    var body: some View {
        let text = ThisWeekText(l10n: l10n, calendar: model.calendar, now: now)
        let digest = ThisWeekDigest(
            deadlines: model.upcomingDeadlines, plan: model.studyPlan, now: now, calendar: model.calendar
        )
        ReadingPage {
            LampBand(lit: true) {
                ThisWeekHeader(digest: digest, text: text)
            }
        } content: {
            ReadingColumn {
                if let crash = model.lastCrash {
                    CrashNotice(crash: crash)
                }
                if !model.failingSources.isEmpty {
                    ThisWeekSourceProblems()
                }
                RemindersCatchUp()
                Next7DaysSection(digest: digest, text: text)
                if let note = model.weeklyNote {
                    WeeklyNoteSection(note: note, now: now)
                }
                StudyPlanSection(text: text, expanded: planExpanded)
                ContentsSection(text: text)
            }
        }
    }
}

/// The band's text: today's date (Large Title), the summary, and Next up or the next deadline.
private struct ThisWeekHeader: View {
    let digest: ThisWeekDigest
    let text: ThisWeekText

    @Environment(AppModel.self) private var model
    @Environment(\.locale) private var locale

    var body: some View {
        // zh: sans Semibold for the date (spec §4.3); Latin: Regular (serif is M3).
        PageHeader(
            title: text.todayTitle,
            subtitle: text.summary(digest, deadlinesLoaded: model.sectionErrors[.deadlines] == nil),
            titleWeight: locale.language.languageCode == .chinese ? .semibold : .regular
        )
        if let next = digest.nextUp, let due = ThisWeekDigest.time(of: next) {
            NextUpLine(deadline: next, due: due, text: text)
                .padding(.top, PLSpace.s2)
        } else if let next = digest.nextDeadline(after: text.now), let line = text.next(next) {
            Text(line)
                .font(PLType.callout.font)
                .monospacedDigit()
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// Next up (spec §6.5): only for a real deadline due within 24 hours. Minutes, never seconds;
/// no red, no pulse; under 2 h only the glyph changes. Never re-announced.
struct NextUpLine: View {
    let deadline: Deadline
    let due: Date
    let text: ThisWeekText

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n

    var body: some View {
        let countdown = Countdown(from: text.now, to: due)
        let course = ThisWeekNavigation.course(of: deadline, in: model)
        HStack(alignment: .lastTextBaseline, spacing: PLSpace.s4) {
            HStack(alignment: .firstTextBaseline, spacing: PLSpace.s2) {
                Image(systemName: countdown.isFinalStretch ? "clock.badge.exclamationmark" : "clock")
                    .symbolRenderingMode(.hierarchical)
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(true)
                Text(l10n("mac.thisWeek.nextUp"))
                    .font(PLType.headline.font)
                    .accessibilityAddTraits(.isHeader)
                VStack(alignment: .leading, spacing: PLSpace.s1) {
                    Text(titleLine)
                        .font(PLType.body.font)
                        .fixedSize(horizontal: false, vertical: true)
                    Text([text.due(due), text.countdown(countdown)].joined(separator: " · "))
                        .font(PLType.callout.font)
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(text.spokenDeadline(deadline, withCountdown: true))
            }
            Spacer(minLength: 0)
            if let course {
                Button(l10n("mac.actions.openCourse")) {
                    ThisWeekNavigation.open(courseId: course.course.id, section: .deadlines, model: model)
                }
                .buttonStyle(.bordered)
            }
        }
    }

    /// "Problem Set 2 — questions 1–3 · DEMO205".
    private var titleLine: String {
        [deadline.event.title, text.course(of: deadline)].compactMap(\.self).joined(separator: " · ")
    }
}
