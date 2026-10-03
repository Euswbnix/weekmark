// This Week's reminder notes (model-access design §5.3, Delivery; M3, preview builds until they
// ship):
// - the catch-up card: reminders were off (or not allowed) at launch, so what came due since
//   shows here instead of as notifications, with codes and titles only. Open or Dismiss marks
//   them shown; "Get These as Notifications" says yes to reminders;
// - "Remind me": asked while the first sync runs, so answering costs no extra step (the same
//   question is in Settings ▸ Reminders).

import SwiftUI
import PageLampKit
import PageLampModel

struct RemindersCatchUp: View {
    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n

    var body: some View {
        if let delivery = model.reminderDelivery, !delivery.catchUp.isEmpty {
            VStack(alignment: .leading, spacing: PLSpace.s3) {
                Label {
                    Text(l10n("reminders.catchUp.title"))
                        .font(PLType.headline.font)
                        .accessibilityAddTraits(.isHeader)
                } icon: {
                    Image(systemName: "bell")
                        .symbolRenderingMode(.hierarchical)
                        .foregroundStyle(.secondary)
                }
                ForEach(delivery.catchUp, id: \.id) { reminder in
                    row(reminder, delivery: delivery)
                }
                HStack(spacing: PLSpace.s2) {
                    Button(l10n("mac.reminders.catchUp.turnOn")) {
                        Task { try? await delivery.setConsent(true) }
                    }
                    .buttonStyle(.bordered)
                    Button(l10n("reminders.catchUp.dismiss")) {
                        Task { await delivery.dismissCatchUp() }
                    }
                    .linkButtonStyle()
                }
                .padding(.top, PLSpace.s1)
            }
            .padding(PLSpace.s4)
            .frame(maxWidth: .infinity, alignment: .leading)
            .calloutSurface()
            .accessibilityElement(children: .contain)
        }
    }

    private func row(_ reminder: Reminder, delivery: ReminderDelivery) -> some View {
        let text = ReminderText.notification(for: reminder, l10n: l10n)
        return HStack(alignment: .firstTextBaseline, spacing: PLSpace.s3) {
            VStack(alignment: .leading, spacing: PLSpace.s1) {
                Text(text.title)
                    .fixedSize(horizontal: false, vertical: true)
                if !text.body.isEmpty {
                    Text(text.body)
                        .font(PLType.callout.font)
                        .foregroundStyle(.secondary)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button(l10n("reminders.catchUp.open")) {
                Task {
                    let courseId = await delivery.openCatchUp(reminder)
                    if let courseId, model.course(id: courseId) != nil {
                        model.destination = .course(courseId)
                    }
                }
            }
            .buttonStyle(.bordered)
            .controlSize(.small)
        }
        // The row holds its title and Open together, so Open is read with its reminder.
        .accessibilityElement(children: .contain)
    }
}

/// "Remind me", asked while the first sync runs.
struct RemindMeQuestion: View {
    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @State private var failed = false

    var body: some View {
        if let delivery = model.reminderDelivery, delivery.consent == false || delivery.questionAnswer != nil {
            // The buttons give way to a line saying what happens next.
            Callout(tone: .info, symbol: "bell", title: l10n("reminders.remind.title"), message: message(delivery)) {
                if delivery.questionAnswer == nil {
                    Button(l10n("mac.reminders.remind.yes")) { choose(true, delivery) }
                        .buttonStyle(.bordered)
                    Button(l10n("mac.reminders.remind.no")) { choose(false, delivery) }
                        .buttonStyle(.bordered)
                }
            }
        }
    }

    private func message(_ delivery: ReminderDelivery) -> String {
        if failed { return l10n("mac.reminders.remind.saveFailed") }
        switch delivery.questionAnswer {
        case true?: return l10n("mac.reminders.remind.onDone")
        case false?: return l10n("mac.reminders.remind.offDone")
        case nil: return l10n("mac.reminders.remind.question")
        }
    }

    private func choose(_ yes: Bool, _ delivery: ReminderDelivery) {
        delivery.questionAnswer = yes
        Task {
            do throws(PageLampFailure) {
                try await delivery.setConsent(yes)
                failed = false
            } catch {
                // The buttons come back: the answer wasn't saved.
                failed = true
                delivery.questionAnswer = nil
            }
        }
    }
}
