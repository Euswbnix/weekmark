// The one-time question after a course's first cloud run with its materials (design D37, the
// Tauri app's MaterialSharingReminder): may this course's materials be shared with AI services?
// Not an error: an info callout with the three answers and Dismiss. An answer is saved, read out,
// and the course is read again (it changes what may be sent).

import SwiftUI
import PageLampKit
import PageLampModel

struct ExplainSharingQuestion: View {
    let explain: ExplainModel
    let explanation: WeeklyExplanation
    let summary: CourseSummary

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @Environment(\.drawsControlStandIns) private var standIns

    var body: some View {
        Callout(
            tone: .info,
            symbol: "questionmark.circle",
            title: l10n("ai.sharing.reminder.title"),
            message: l10n("ai.sharing.reminder.body", [
                "course": summary.course.code ?? summary.course.name,
                "service": explanation.meta.backendLabel,
            ])
        ) {
            VStack(alignment: .leading, spacing: PLSpace.s2) {
                Text(l10n("ai.sharing.reminder.answer"))
                    .font(PLType.body.font.weight(.semibold))
                // The answers wrap in a narrow window rather than cut their titles short.
                ChipFlow(spacing: PLSpace.s2) {
                    answer(.allowed, "mac.ai.sharingAllowed")
                    answer(.notSure, "mac.ai.sharingNotSure")
                    answer(.notAllowed, "mac.ai.sharingNotAllowed")
                    dismiss
                }
                .controlSize(.small)
                .accessibilityElement(children: .contain)
                .accessibilityLabel(l10n("ai.sharing.reminder.answer"))
                if let failure = explain.sharingFailure {
                    Text(l10n.aiError(failure))
                        .foregroundStyle(PLColor.danger)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    @ViewBuilder private var dismiss: some View {
        if standIns {
            // Offscreen a borderless button draws nothing: its look, as text.
            Text(l10n("ai.sharing.reminder.dismiss"))
                .foregroundStyle(.tint)
                .padding(.vertical, 2)
        } else {
            Button(l10n("ai.sharing.reminder.dismiss")) { explain.dismissSharingQuestion(explanation) }
                .buttonStyle(.borderless)
        }
    }

    private func answer(_ answer: MaterialSharing, _ key: String) -> some View {
        Button(l10n(key)) {
            Task {
                if await explain.answerSharing(answer, for: explanation) {
                    AccessibilityNotification.Announcement(l10n("ai.sharing.saved")).post()
                    await model.refresh()
                } else if let failure = explain.sharingFailure {
                    AccessibilityNotification.Announcement(l10n.aiError(failure)).post()
                }
            }
        }
        .buttonStyle(.bordered)
        .disabled(explain.savingSharing)
    }
}
