// Settings ▸ AI ▸ Weekly note (M3, preview builds; the Tauri app's WeeklyNoteSettingsSection):
// "Prepare my weekly note when I open PageLamp on Monday". Offered while the note's model is a
// provider (an API key or a model on this computer; the facade decides), shown paused when it's on
// but the model no longer allows it (it can still be turned off), hidden otherwise. The hint is
// what Monday's note costs: on this computer (the chosen model's facts; without them, the backend's
// kind, as the Tauri app decides), no price, or "≈ $x" each Monday. The setting lives in the data
// folder, so the Tauri app shares it; only one app prepares the note (the facade records the try).

import SwiftUI
import PageLampKit
import PageLampModel

struct AiWeeklyNoteSection: View {
    let ai: AiSettingsModel

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n

    var body: some View {
        if let settings = ai.noteSettings, settings.prepareOnMonday || settings.prepareOnMondayAllowed {
            let hint = self.hint(settings)
            Section {
                Toggle(l10n("weeklyNote.settings.prepare"), isOn: Binding(
                    get: { settings.prepareOnMonday },
                    set: { on in
                        Task {
                            if !(await ai.setPrepareOnMonday(on)), let failure = ai.noteSettingFailure {
                                AccessibilityNotification.Announcement(
                                    l10n.sentences(l10n("weeklyNote.settings.saveFailed"), l10n.aiError(failure))
                                ).post()
                            }
                        }
                    }
                ))
                .disabled(ai.savingNoteSetting)
                .accessibilityHint(hint ?? "")
                if let failure = ai.noteSettingFailure {
                    Text(l10n.sentences(l10n("weeklyNote.settings.saveFailed"), l10n.aiError(failure)))
                        .foregroundStyle(PLColor.danger)
                        .fixedSize(horizontal: false, vertical: true)
                }
            } header: {
                Text(l10n("weeklyNote.settings.title"))
            } footer: {
                if let hint {
                    Text(hint)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    /// Paused, or what Monday's note costs with its model (`AiSettingsModel.noteCost`).
    private func hint(_ settings: WeeklyNoteSettings) -> String? {
        guard settings.prepareOnMondayAllowed else { return l10n("weeklyNote.settings.paused") }
        return ai.noteCost.flatMap(NoteText(l10n: l10n, calendar: model.calendar).mondayCost)
    }
}
