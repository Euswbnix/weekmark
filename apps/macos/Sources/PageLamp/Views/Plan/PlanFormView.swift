// What to plan (design §5.1; the Tauri app's PlanForm): days from today, hours per week, days
// off, which courses (the active ones to start with) and a note. What's still missing and "≈ $x"
// sit with the buttons (PlanSheet), so they stay in view.

import SwiftUI
import PageLampKit
import PageLampModel

struct PlanFormView: View {
    @Bindable var plan: PlanModel

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    /// Offscreen renders can't draw text fields or button toggles: the harness gets stand-ins.
    @Environment(\.drawsControlStandIns) private var standIns

    var body: some View {
        VStack(alignment: .leading, spacing: PLSpace.s4) {
            HStack(alignment: .top, spacing: PLSpace.s6) {
                numberField("plan.form.horizon", hint: "plan.form.horizonHint", text: $plan.horizon, invalid: plan.horizonDays == nil)
                numberField("plan.form.hours", hint: "plan.form.hoursHint", text: $plan.hours, invalid: plan.hoursPerWeek == nil)
            }
            field("plan.form.daysOff", hint: "plan.form.daysOffHint") {
                HStack(spacing: PLSpace.s1) {
                    ForEach(weekdays, id: \.self) { day in
                        dayToggle(day)
                    }
                }
                .accessibilityElement(children: .contain)
                .accessibilityLabel(l10n("plan.form.daysOff"))
            }
            field("plan.form.courses", hint: "plan.form.coursesHint") {
                VStack(alignment: .leading, spacing: PLSpace.s1) {
                    ForEach(plan.courses, id: \.course.id) { summary in
                        Toggle(isOn: Binding(
                            get: { plan.isPicked(summary.course.id) },
                            set: { plan.setPicked(summary.course.id, $0) }
                        )) {
                            courseLabel(summary)
                        }
                        .toggleStyle(.checkbox)
                    }
                }
                .accessibilityElement(children: .contain)
                .accessibilityLabel(l10n("plan.form.courses"))
            }
            field("plan.form.note", hint: "plan.form.noteHint") {
                if standIns {
                    FieldStandIn(text: plan.note)
                } else {
                    TextField(l10n("plan.form.note"), text: Binding(
                        get: { plan.note },
                        set: { plan.note = PlanModel.capped($0, to: plan.noteMaxChars) }
                    ), axis: .vertical)
                    .lineLimit(2...4)
                    .labelsHidden()
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    // MARK: - Fields

    private func field<Content: View>(_ label: String, hint: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: PLSpace.s1) {
            Text(l10n(label))
                .font(PLType.body.font.weight(.semibold))
            content()
            Text(l10n(hint))
                .font(PLType.callout.font)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// A whole-number field; VoiceOver reads what's typed, then what's wrong with it (as the hint).
    private func numberField(_ label: String, hint: String, text: Binding<String>, invalid: Bool) -> some View {
        field(label, hint: hint) {
            Group {
                if standIns {
                    FieldStandIn(text: text.wrappedValue)
                } else {
                    TextField(l10n(label), text: text)
                        .labelsHidden()
                        .multilineTextAlignment(.trailing)
                        .monospacedDigit()
                }
            }
            .frame(width: Self.numberWidth)
            .overlay {
                if invalid {
                    RoundedRectangle(cornerRadius: 5).strokeBorder(PLColor.danger)
                }
            }
            .accessibilityHint(invalid ? l10n(label == "plan.form.horizon" ? "plan.form.invalidHorizon" : "plan.form.invalidHours") : l10n(hint))
        }
    }

    @ViewBuilder
    private func dayToggle(_ day: DayOfWeek) -> some View {
        let off = plan.daysOff.contains(day)
        let name = weekdayName(day, short: true)
        if standIns {
            Text(name)
                .padding(.horizontal, PLSpace.s2)
                .padding(.vertical, 3)
                .foregroundStyle(off ? AnyShapeStyle(.white) : AnyShapeStyle(.primary))
                .background(off ? AnyShapeStyle(.tint) : AnyShapeStyle(.fill.tertiary), in: .rect(cornerRadius: 5))
        } else {
            Toggle(name, isOn: Binding(get: { off }, set: { plan.setDayOff(day, $0) }))
                .toggleStyle(.button)
                .accessibilityLabel(weekdayName(day, short: false))
        }
    }

    private func courseLabel(_ summary: CourseSummary) -> Text {
        guard let code = summary.course.code, !code.isEmpty else { return Text(summary.course.name) }
        return InlineText.joined([
            Text(code).fontWeight(.semibold),
            Text(verbatim: " · ").foregroundStyle(.secondary),
            Text(summary.course.name).foregroundStyle(.secondary),
        ])
    }

    // MARK: - Weekdays

    /// The week's days from the calendar's first weekday.
    private var weekdays: [DayOfWeek] {
        let first = model.calendar.firstWeekday - 1
        let days = ReminderSettingsEditor.weekdays
        return Array(days[first...] + days[..<first])
    }

    /// "Mon" / "周一", or "Monday" / "星期一", in the app's language.
    private func weekdayName(_ day: DayOfWeek, short: Bool) -> String {
        var calendar = model.calendar
        calendar.locale = l10n.locale
        let index = ReminderSettingsEditor.weekdayNumber(day) - 1
        return short ? calendar.shortStandaloneWeekdaySymbols[index] : calendar.standaloneWeekdaySymbols[index]
    }

    private static let numberWidth: CGFloat = 72
}
