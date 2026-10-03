// Settings (spec §3.5, M1; Settings scene, 600 pt; TabView + Tab; Form(.grouped)): General
// (language, appearance), Data (folders, what's stored), Privacy (the disclosure and promises),
// Help (diagnostic report with a preview first, logs, report a problem, about). Settings has no
// lamp band (the lamp marks only "now") and no custom glass: the window is the system's.

import SwiftUI
import PageLampModel

/// The Settings tabs. Reminders and AI (M3) show only where they're on (preview builds, until
/// they ship).
package enum SettingsTab: String, CaseIterable, Sendable {
    case general
    case reminders
    case ai
    case data
    case privacy
    case help
}

extension EnvironmentValues {
    /// Snapshots render forms with `.columns`: ImageRenderer draws `.grouped` forms (a scroll
    /// view) blank. The app always uses `.grouped`.
    @Entry var settingsSnapshotLayout = false
}

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @State private var settings = SettingsModel()

    var body: some View {
        TabView {
            Tab(l10n("mac.settings.tabs.general"), systemImage: "gearshape") {
                SettingsGeneralTab()
                    .frame(width: PLSize.settingsWidth, height: SettingsGeneralTab.height)
            }
            if model.reminderDelivery != nil {
                Tab(l10n("mac.settings.tabs.reminders"), systemImage: "bell") {
                    SettingsRemindersTab()
                        .frame(width: PLSize.settingsWidth, height: 520)
                }
            }
            if model.aiSettings {
                Tab(l10n("mac.settings.tabs.ai"), systemImage: "sparkles") {
                    SettingsAiTab()
                        .frame(width: PLSize.settingsWidth, height: SettingsAiTab.height)
                }
            }
            Tab(l10n("mac.settings.tabs.data"), systemImage: "internaldrive") {
                SettingsDataTab(settings: settings)
                    .frame(width: PLSize.settingsWidth, height: 560)
            }
            Tab(l10n("mac.settings.tabs.privacy"), systemImage: "lock.shield") {
                SettingsPrivacyTab()
                    .frame(width: PLSize.settingsWidth, height: 440)
            }
            Tab(l10n("mac.settings.tabs.help"), systemImage: "questionmark.circle") {
                SettingsHelpTab(settings: settings)
                    .frame(width: PLSize.settingsWidth, height: 500)
            }
        }
        // The file reader lines (Help), and S2 with the version fallback: the core's health
        // report works without the database.
        .task(id: model.status == nil) {
            await settings.loadDoctor(model.service)
        }
        // Settings ▸ Help ▸ Copy Diagnostic Report… previews the report on this window.
        .diagnosticReportSheet(host: .settings)
        // Appearance applies even while the main window is closed.
        .appAppearance(model.appearance)
    }
}

/// A settings tab's form: `.grouped` in the app, `.columns` in snapshots.
struct SettingsForm<Content: View>: View {
    @ViewBuilder var content: Content
    @Environment(\.settingsSnapshotLayout) private var snapshot

    var body: some View {
        if snapshot {
            Form { content }
                .formStyle(SnapshotGroupedFormStyle())
        } else {
            Form { content }
                .formStyle(.grouped)
        }
    }
}

/// One tab rendered on its own, with its title (the snapshot harness).
package struct SettingsTabPage: View {
    let tab: SettingsTab
    let title: String
    let settings: SettingsModel?

    /// Settings ▸ Reminders' loaded settings (snapshots).
    let reminders: ReminderSettingsEditor?
    /// Settings ▸ AI, loaded (snapshots).
    let ai: AiSettingsModel?
    @State private var fallback = SettingsModel()

    package init(
        tab: SettingsTab,
        title: String,
        settings: SettingsModel? = nil,
        reminders: ReminderSettingsEditor? = nil,
        ai: AiSettingsModel? = nil
    ) {
        self.tab = tab
        self.title = title
        self.settings = settings
        self.reminders = reminders
        self.ai = ai
    }

    package var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title)
                .font(PLType.title2.font.weight(.semibold))
                .accessibilityAddTraits(.isHeader)
                .padding([.horizontal, .top], PLLayout.sheetInset)
            switch tab {
            case .general: SettingsGeneralTab()
            case .reminders: SettingsRemindersTab(editor: reminders ?? ReminderSettingsEditor())
            case .ai: SettingsAiTab(ai: ai)
            case .data: SettingsDataTab(settings: settings ?? fallback)
            case .privacy: SettingsPrivacyTab()
            case .help: SettingsHelpTab(settings: settings ?? fallback)
            }
        }
        .environment(\.settingsSnapshotLayout, true)
        .frame(maxWidth: .infinity, alignment: .topLeading)
    }
}

/// Offscreen stand-in for `.grouped` (which ImageRenderer draws blank): each section's header,
/// its rows on a rounded fill with hairlines, then its footer; label-left, value-right rows.
/// Only the snapshot harness uses it.
private struct SnapshotGroupedFormStyle: FormStyle {
    func makeBody(configuration: Configuration) -> some View {
        VStack(alignment: .leading, spacing: PLSpace.s5) {
            ForEach(sections: configuration.content) { section in
                VStack(alignment: .leading, spacing: PLSpace.s2) {
                    if !section.header.isEmpty {
                        section.header
                            .font(PLType.headline.font)
                            .padding(.horizontal, PLSpace.s2)
                    }
                    VStack(alignment: .leading, spacing: 0) {
                        Group(subviews: section.content) { rows in
                            ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                                if index > 0 {
                                    Divider()
                                }
                                row
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .padding(.horizontal, PLSpace.s3)
                                    .padding(.vertical, PLSpace.s2)
                            }
                        }
                    }
                    .background(.fill.quaternary, in: .rect(cornerRadius: PLRadius.row))
                    if !section.footer.isEmpty {
                        section.footer
                            .font(PLType.callout.font)
                            .padding(.horizontal, PLSpace.s2)
                    }
                }
            }
        }
        .labeledContentStyle(SnapshotLabeledContentStyle())
        .padding(PLLayout.sheetInset)
    }
}

private struct SnapshotLabeledContentStyle: LabeledContentStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: PLSpace.s4) {
            configuration.label
            Spacer(minLength: PLSpace.s4)
            configuration.content
        }
    }
}
