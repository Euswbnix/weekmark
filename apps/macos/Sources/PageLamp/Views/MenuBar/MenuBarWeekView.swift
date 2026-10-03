// The menu bar extra (spec §3.7 W10, M3; `.menuBarExtraStyle(.window)`, 340 pt): This Week at a
// glance from `weekly_digest()` only — due soon (≤ 5), today's plan (≤ 4) and last week's
// progress, the active courses' weeks — then Sync Now, Open PageLamp and Settings. A row opens the
// main window at its course. Nothing inside is glass. Its language follows the app's.

import AppKit
import SwiftUI
import PageLampKit
import PageLampModel

package struct MenuBarWeekView: View {
    /// The extra's width (spec §3.7).
    package static let width: CGFloat = 340

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @Environment(\.openWindow) private var openWindow

    package init() {}

    package var body: some View {
        let now = model.clock()
        let text = ThisWeekText(l10n: l10n, calendar: model.calendar, now: now)
        VStack(alignment: .leading, spacing: 0) {
            header(now: now)
            Divider()
            if let week = model.menuBarWeek, !model.courses.isEmpty || !week.isEmpty {
                content(week, text: text)
            } else if model.menuBarWeekFailure != nil {
                note(l10n("mac.menuBar.loadFailed")) {
                    Button(l10n("mac.actions.tryAgain")) { Task { await model.loadMenuBarWeek() } }
                }
            } else if model.menuBarWeek != nil {
                note(l10n("courses.empty.title")) {
                    Button(l10n("mac.actions.setUp")) { open(.sources) }
                }
            }
            Divider()
            footer
        }
        .frame(width: Self.width)
        .task { await model.loadMenuBarWeek() }
    }

    // MARK: - Parts

    private func header(now: Date) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(l10n("mac.nav.thisWeek"))
                .font(PLType.headline.font)
                .accessibilityAddTraits(.isHeader)
            Spacer()
            Text(range(from: now))
                .font(PLType.callout.font)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, PLSpace.s4)
        .padding(.vertical, PLSpace.s3)
    }

    @ViewBuilder
    private func content(_ week: MenuBarWeek, text: ThisWeekText) -> some View {
        section(l10n("mac.menuBar.dueSoon")) {
            if week.dueSoon.isEmpty {
                Text(l10n("reminders.notify.digestBodyNone"))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, PLSpace.s2)
            }
            ForEach(week.dueSoon) { item in
                MenuRow {
                    if let courseId = item.courseId { open(.course(courseId)) } else { open(.thisWeek) }
                } content: {
                    VStack(alignment: .leading, spacing: 1) {
                        Text(text.due(item.due))
                            .font(PLType.callout.font)
                            .foregroundStyle(.secondary)
                        Text(item.courseCode.map { "\(item.deadline.event.title) · \($0)" } ?? item.deadline.event.title)
                            .lineLimit(2)
                    }
                }
            }
            if week.moreDue > 0 {
                Text(l10n.plural("mac.menuBar.moreDue", count: week.moreDue))
                    .font(PLType.callout.font)
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, PLSpace.s2)
            }
        }
        if !week.today.isEmpty || week.lastWeek != nil {
            Divider()
            section(l10n("mac.menuBar.todayPlan")) {
                ForEach(Array(week.today.enumerated()), id: \.offset) { _, item in
                    HStack(alignment: .firstTextBaseline, spacing: PLSpace.s2) {
                        Image(systemName: item.done ? "checkmark.circle.fill" : "circle")
                            .foregroundStyle(.secondary)
                            .accessibilityHidden(true)
                        Text(item.title)
                            .lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        if let minutes = item.minutes {
                            Text(text.planMinutes(minutes))
                                .font(PLType.callout.font)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .padding(.horizontal, PLSpace.s2)
                }
                if week.moreToday > 0 {
                    Text(l10n.plural("mac.menuBar.moreToday", count: week.moreToday))
                        .font(PLType.callout.font)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, PLSpace.s2)
                }
                // Written by PageLamp: its tasks carry the AI-generated line (Canvas §2E).
                if !week.today.isEmpty, let label = week.aiLabel {
                    Text(l10n.aiLabel(label, calendar: model.calendar))
                        .font(PLType.callout.font)
                        .foregroundStyle(.secondary)
                        .lineLimit(2)
                        .padding(.horizontal, PLSpace.s2)
                }
                if let last = week.lastWeek {
                    Text(l10n("mac.menuBar.lastWeek", ["done": l10n.number(last.done), "planned": l10n.number(last.planned)]))
                        .font(PLType.callout.font)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, PLSpace.s2)
                }
            }
        }
        if !week.courses.isEmpty {
            Divider()
            LazyVGrid(columns: [GridItem(.flexible(), alignment: .leading), GridItem(.flexible(), alignment: .leading)],
                      alignment: .leading, spacing: PLSpace.s1) {
                ForEach(week.courses, id: \.courseId) { course in
                    MenuRow { open(.course(course.courseId)) } content: {
                        HStack(spacing: PLSpace.s2) {
                            Text(course.code ?? course.name).lineLimit(1)
                            if let number = course.week {
                                Text(l10n("mac.common.week.compact", ["week": l10n.number(Int(number))]))
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .padding(PLSpace.s2)
        }
    }

    private var footer: some View {
        HStack(spacing: PLSpace.s2) {
            Button(l10n("mac.actions.syncNow")) { Task { await model.syncAll() } }
                .disabled(!model.canSync)
            Spacer()
            SettingsLink { Text(l10n("mac.menuBar.settings")) }
            Button(l10n("mac.menuBar.open")) { open(nil) }
        }
        .buttonStyle(.bordered)
        .controlSize(.small)
        .padding(PLSpace.s3)
    }

    // MARK: - Helpers

    private func section<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: PLSpace.s1) {
            Text(title)
                .font(PLType.subheadline.font.weight(.semibold))
                .foregroundStyle(.secondary)
                .accessibilityAddTraits(.isHeader)
                .padding(.horizontal, PLSpace.s2)
            content()
        }
        .padding(PLSpace.s2)
    }

    private func note<Actions: View>(_ message: String, @ViewBuilder actions: () -> Actions) -> some View {
        VStack(alignment: .leading, spacing: PLSpace.s2) {
            Text(message)
            actions()
                .buttonStyle(.bordered)
                .controlSize(.small)
        }
        .padding(PLSpace.s4)
    }

    /// Opens the main window, at `destination` when given.
    private func open(_ destination: Destination?) {
        if let destination { model.destination = destination }
        NSApp.activate()
        openWindow(id: PageLampScenes.mainWindowID)
    }

    /// "Sep 26 – Oct 2": today and the next six days, like This Week's subtitle.
    private func range(from now: Date) -> String {
        let start = model.calendar.startOfDay(for: now)
        let end = model.calendar.date(byAdding: .day, value: ThisWeekDigest.days - 1, to: start) ?? start
        return (start ..< end).formatted(
            Date.IntervalFormatStyle(locale: l10n.locale, calendar: model.calendar, timeZone: model.calendar.timeZone)
                .month(.abbreviated).day()
        )
    }
}

/// A plain row with a hover fill (spec §3.7), the whole row a button.
private struct MenuRow<Content: View>: View {
    let action: () -> Void
    @ViewBuilder var content: Content
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            content
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, PLSpace.s2)
                .padding(.vertical, PLSpace.s1)
                .contentShape(.rect)
                .background(hovering ? AnyShapeStyle(.fill.quaternary) : AnyShapeStyle(.clear), in: .rect(cornerRadius: 6))
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}

/// The menu bar extra's label: the lamp, lit while something is due within 24 hours (spec §6.5).
package struct MenuBarLabel: View {
    @Environment(AppModel.self) private var model

    package init() {}

    package var body: some View {
        let due = model.menuBarWeek?.dueWithin24h ?? 0
        let l10n = model.menuL10n
        Image(systemName: due > 0 ? "lamp.desk.fill" : "lamp.desk")
            .accessibilityLabel(due > 0 ? l10n.plural("mac.menuBar.labelDue", count: due) : l10n("mac.menuBar.label"))
    }
}
