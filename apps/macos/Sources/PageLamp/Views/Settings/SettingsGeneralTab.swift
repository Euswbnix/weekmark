// Settings ▸ General (spec §3.5 W8a, M1): Language and Appearance. Content switches language
// live; menus and system dialogs follow AppleLanguages, hence Reopen Now. In preview builds until
// they ship: Show PageLamp in the menu bar and Open PageLamp at login (M3), independent of each
// other and of reminders. (Week starts on is M2.)

import AppKit
import SwiftUI
import PageLampModel

struct SettingsGeneralTab: View {
    /// The tab's height in the Settings window (preview builds have the menu bar section).
    #if PAGELAMP_PREVIEW
    static let height: CGFloat = 420
    #else
    static let height: CGFloat = 220
    #endif

    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @State private var reopening = false
    @AppStorage(PageLampScenes.showInMenuBarKey) private var showInMenuBar = false
    #if PAGELAMP_PREVIEW
    @State private var loginItem = LoginItem()
    #endif

    var body: some View {
        @Bindable var model = model
        SettingsForm {
            Section {
                Picker(l10n("settings.appearance.language"), selection: $model.language) {
                    ForEach(AppLanguage.allCases, id: \.self) { language in
                        Text(language.title(l10n: l10n, preferredLanguages: SystemLanguages.preferred))
                            .tag(language)
                    }
                }
                if model.menusNeedReopen {
                    HStack(alignment: .firstTextBaseline, spacing: PLSpace.s4) {
                        Text(l10n("mac.settings.language.reopenHint"))
                            .font(PLType.callout.font)
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Button(l10n("mac.actions.reopenNow")) {
                            reopening = true
                            model.applyLanguageToMenus()
                            Task {
                                await AppRelauncher.relaunch()
                                reopening = false
                            }
                        }
                        .disabled(reopening || !AppRelauncher.canRelaunch)
                    }
                }
                Picker(l10n("settings.appearance.title"), selection: $model.appearance) {
                    ForEach(AppAppearance.allCases, id: \.self) { appearance in
                        Text(appearance.title(l10n: l10n)).tag(appearance)
                    }
                }
                .pickerStyle(.segmented)
            }
            #if PAGELAMP_PREVIEW
            Section {
                Toggle(isOn: $showInMenuBar) {
                    Text(l10n("mac.reminders.general.menuBar"))
                    Text(l10n("mac.reminders.general.menuBarHint"))
                }
                loginRows
            }
            .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
                // The student may have allowed or removed it in System Settings meanwhile.
                loginItem.refresh()
            }
            #endif
        }
    }
}

#if PAGELAMP_PREVIEW
extension SettingsGeneralTab {
    /// "Open PageLamp at login" and what macOS says about it.
    @ViewBuilder fileprivate var loginRows: some View {
        Toggle(l10n("mac.reminders.general.login"), isOn: Binding(
            get: { loginItem.state == .on || loginItem.state == .needsApproval },
            set: { loginItem.set($0) }
        ))
        .disabled(loginItem.state == .unavailable)
        switch loginItem.state {
        case .needsApproval:
            HStack(alignment: .firstTextBaseline, spacing: PLSpace.s4) {
                Text(l10n("mac.reminders.general.loginApproval"))
                    .font(PLType.callout.font)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button(l10n("mac.reminders.general.openLoginItems")) { loginItem.openSystemSettings() }
            }
        case .unavailable:
            Text(l10n("mac.reminders.general.loginUnavailable"))
                .font(PLType.callout.font)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        case .on, .off:
            EmptyView()
        }
        if loginItem.failed {
            Text(l10n("mac.reminders.general.loginFailed"))
                .font(PLType.callout.font)
                .foregroundStyle(PLColor.danger)
        }
    }
}
#endif

/// The system's preferred languages, ignoring this app's own AppleLanguages override (set by
/// Reopen Now), so "System (…)" names what the system really prefers. Read-only.
enum SystemLanguages {
    static var preferred: [String] {
        let global = UserDefaults.standard.persistentDomain(forName: UserDefaults.globalDomain)
        return global?["AppleLanguages"] as? [String] ?? Locale.preferredLanguages
    }
}

/// Reopen Now (spec §3.5 [verify]): launches a new instance of this app bundle, then quits, so
/// the menus and system dialogs pick up the AppleLanguages that `applyLanguageToMenus()` set.
enum AppRelauncher {
    /// Only a real app bundle can be relaunched (not `swift run`).
    static var canRelaunch: Bool {
        Bundle.main.bundleURL.pathExtension == "app"
    }

    static func relaunch() async {
        guard canRelaunch else { return }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.createsNewApplicationInstance = true
        do {
            _ = try await NSWorkspace.shared.openApplication(at: Bundle.main.bundleURL, configuration: configuration)
            NSApp.terminate(nil)
        } catch {
            NSSound.beep()
        }
    }
}
