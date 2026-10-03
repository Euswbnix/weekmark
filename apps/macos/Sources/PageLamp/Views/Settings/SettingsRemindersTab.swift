// Settings ▸ Reminders (spec §3.5, M3; model-access design §5.3): "Remind me with notifications"
// (the Mac app's own answer; the system's prompt comes with it), the permission as macOS has it,
// then the kinds: deadlines coming up, your week (day and time) and today's study plan (time).
// Every change is saved at once. Preview builds only, until reminders ship.

import AppKit
import SwiftUI
import PageLampKit
import PageLampModel

struct SettingsRemindersTab: View {
    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    /// Offscreen renders can't draw a date picker: the snapshot harness gets the time as text.
    @Environment(\.drawsControlStandIns) private var standIns
    @State private var editor: ReminderSettingsEditor

    /// - Parameter editor: already loaded for the snapshot harness (`.task` never runs offscreen).
    init(editor: ReminderSettingsEditor = ReminderSettingsEditor()) {
        _editor = State(initialValue: editor)
    }

    var body: some View {
        SettingsForm {
            if let delivery = model.reminderDelivery {
                consentSection(delivery)
                if let settings = editor.settings {
                    kindsSection(settings)
                }
                if editor.saveFailed {
                    Text(l10n("reminders.settings.saveFailed"))
                        .foregroundStyle(PLColor.danger)
                        .accessibilityAddTraits(.isStaticText)
                }
            }
        }
        .task(id: model.dataMode) {
            await editor.load(model.service)
            await model.reminderDelivery?.refreshPermission()
        }
        // The student may have changed the permission in System Settings meanwhile.
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            Task { await model.reminderDelivery?.refreshPermission() }
        }
    }

    // MARK: - Remind me

    @ViewBuilder
    private func consentSection(_ delivery: ReminderDelivery) -> some View {
        Section {
            Toggle(isOn: Binding(
                get: { editor.settings?.runInBackground ?? false },
                set: { on in Task { await editor.setConsent(on, service: model.service, delivery: delivery) } }
            )) {
                Text(l10n("mac.reminders.settings.notify"))
                Text(l10n("mac.reminders.settings.notifyHint"))
            }
            .disabled(editor.settings == nil)
            if editor.settings?.runInBackground == true {
                switch delivery.permission {
                case .allowed:
                    EmptyView()
                case .notDetermined:
                    Text(l10n("mac.reminders.settings.notDetermined"))
                        .font(PLType.callout.font)
                        .foregroundStyle(.secondary)
                case .denied:
                    HStack(alignment: .firstTextBaseline, spacing: PLSpace.s4) {
                        Text(l10n("mac.reminders.settings.denied"))
                            .font(PLType.callout.font)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Button(l10n("mac.reminders.settings.openNotificationSettings")) {
                            AppActions.open(Self.notificationSettingsURL)
                        }
                    }
                }
            }
        } footer: {
            Text(l10n("reminders.settings.description"))
                .foregroundStyle(.secondary)
        }
    }

    // MARK: - The kinds

    @ViewBuilder
    private func kindsSection(_ settings: ReminderSettings) -> some View {
        Section {
            Toggle(isOn: field(\.deadlineSoon)) {
                Text(l10n("reminders.settings.deadlineSoon"))
                Text(l10n("reminders.settings.deadlineSoonHint"))
            }
            Toggle(isOn: field(\.weeklyDigest)) {
                Text(l10n("reminders.settings.weeklyDigest"))
                Text(l10n("reminders.settings.weeklyDigestHint"))
            }
            if settings.weeklyDigest {
                Picker(l10n("mac.reminders.settings.digestDay"), selection: field(\.digestDay)) {
                    ForEach(weekdaysInOrder, id: \.self) { day in
                        Text(weekdayName(day)).tag(day)
                    }
                }
                timePicker(l10n("mac.reminders.settings.digestTime"), \.digestTime)
            }
            Toggle(isOn: field(\.planToday)) {
                Text(l10n("reminders.settings.planToday"))
                Text(l10n("reminders.settings.planTodayHint"))
            }
            if settings.planToday {
                timePicker(l10n("mac.reminders.settings.planTime"), \.planTodayTime)
            }
        }
    }

    @ViewBuilder
    private func timePicker(_ label: String, _ path: WritableKeyPath<ReminderSettingsDraft, String>) -> some View {
        if standIns {
            LabeledContent(label) {
                Text(time(path).wrappedValue.formatted(
                    Date.FormatStyle(date: .omitted, time: .shortened, locale: l10n.locale, calendar: model.calendar, timeZone: model.calendar.timeZone)
                ))
                .padding(.horizontal, PLSpace.s2)
                .padding(.vertical, 2)
                .background(.fill.tertiary, in: .rect(cornerRadius: 5))
            }
        } else {
            DatePicker(label, selection: time(path), displayedComponents: .hourAndMinute)
        }
    }

    /// A field of the saved settings; setting it saves.
    private func field<Value>(_ path: WritableKeyPath<ReminderSettingsDraft, Value>) -> Binding<Value> where Value: Sendable {
        Binding(
            get: { ReminderSettingsDraft(editor.settings ?? Self.fallback)[keyPath: path] },
            set: { value in
                Task {
                    await editor.update({ $0[keyPath: path] = value }, service: model.service, delivery: model.reminderDelivery)
                }
            }
        )
    }

    /// A "09:00" field as a time picker's date, in the app's calendar.
    private func time(_ path: WritableKeyPath<ReminderSettingsDraft, String>) -> Binding<Date> {
        let calendar = model.calendar
        return Binding(
            get: {
                let text = ReminderSettingsDraft(editor.settings ?? Self.fallback)[keyPath: path]
                return ReminderSettingsEditor.date(fromClock: text, calendar: calendar) ?? Date(timeIntervalSinceReferenceDate: 0)
            },
            set: { date in
                let text = ReminderSettingsEditor.clock(from: date, calendar: calendar)
                Task {
                    await editor.update({ $0[keyPath: path] = text }, service: model.service, delivery: model.reminderDelivery)
                }
            }
        )
    }

    /// The week's days from the calendar's first weekday (Monday first where weeks start then).
    private var weekdaysInOrder: [DayOfWeek] {
        let first = model.calendar.firstWeekday - 1
        let days = ReminderSettingsEditor.weekdays
        return Array(days[first...] + days[..<first])
    }

    /// "Monday" / "星期一" in the app's language.
    private func weekdayName(_ day: DayOfWeek) -> String {
        var calendar = model.calendar
        calendar.locale = l10n.locale
        return calendar.standaloneWeekdaySymbols[ReminderSettingsEditor.weekdayNumber(day) - 1]
    }

    /// What the switches show before the settings are read.
    private static let fallback = ReminderSettings(
        deadlineSoon: true, weeklyDigest: true, digestDay: .monday, digestTime: "09:00",
        planToday: false, planTodayTime: "08:00", runInBackground: false
    )

    /// System Settings ▸ Notifications, at PageLamp when the system knows the app.
    private static var notificationSettingsURL: URL? {
        let base = "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
        guard let id = Bundle.main.bundleIdentifier else { return URL(string: base) }
        return URL(string: "\(base)?id=\(id)")
    }
}
