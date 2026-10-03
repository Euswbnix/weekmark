// Menus and shortcuts (spec §2.7, M1 items). The standard menus (Edit, Window, the sidebar,
// inspector and toolbar toggles) come from the system and follow AppleLanguages; our items use
// `model.menuL10n` (the launch language) so the menu bar never mixes two languages. Commands
// whose result shows in the main window go through `AppModel.perform(_:openMainWindow:)`, which
// opens (or brings forward) the main window first: with only Settings open they still work.

import SwiftUI
import PageLampModel

struct PageLampCommands: Commands {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        let l10n = model.menuL10n

        // PageLamp ▸ About (the standard panel: version, tagline, licence, links)
        CommandGroup(replacing: .appInfo) {
            Button(l10n("mac.menu.about")) {
                Task { await AppActions.showAbout(model) }
            }
        }

        // File ▸ Sync Now ⌘R (Add Source… ⇧⌘N is M2)
        CommandGroup(replacing: .newItem) {
            Button(l10n("mac.actions.syncNow")) {
                Task { await model.syncAll() }
            }
            .keyboardShortcut("r")
            .disabled(!model.canSync)
        }

        // View ▸ This Week ⌘1 · Sources & Sync ⌘2 · Connect AI App ⌘3 (above the sidebar toggle)
        CommandGroup(before: .sidebar) {
            Button(l10n("mac.nav.thisWeek")) { perform(.show(.thisWeek)) }
                .keyboardShortcut("1")
            Button(l10n("mac.nav.sources")) { perform(.show(.sources)) }
                .keyboardShortcut("2")
            Button(l10n("mac.nav.connect")) { perform(.show(.connect)) }
                .keyboardShortcut("3")
            Divider()
        }

        // Go ▸ Previous Week ⌘[ · Next Week ⌘] · Current Week ⇧⌘T (⌘T is HIG-reserved)
        CommandMenu(l10n("mac.menu.go")) {
            Button(l10n("mac.actions.previousWeek")) { perform(.stepWeek(-1)) }
                .keyboardShortcut("[")
                .disabled(!model.canStepWeek(by: -1))
            Button(l10n("mac.actions.nextWeek")) { perform(.stepWeek(1)) }
                .keyboardShortcut("]")
                .disabled(!model.canStepWeek(by: 1))
            Button(l10n("mac.actions.currentWeek")) { perform(.currentWeek) }
                .keyboardShortcut("t", modifiers: [.command, .shift])
                .disabled(!model.canShowCurrentWeek)
        }

        // Help
        CommandGroup(replacing: .help) {
            Button(l10n("mac.menu.help")) { AppActions.open(BrandLinks.homepage) }
            Divider()
            Button(l10n("mac.actions.copyDiagnosticReport")) { perform(.diagnosticReport) }
            Button(l10n("mac.actions.reportProblem")) { AppActions.open(BrandLinks.issues) }
            Button(l10n("mac.actions.openLogsFolder")) {
                Task { await AppActions.openLogsFolder(model) }
            }
        }
    }

    private func perform(_ command: MainWindowCommand) {
        Task { await model.perform(command) { openWindow(id: PageLampScenes.mainWindowID) } }
    }
}

/// The Course menu (spec §2.7, M1 items) for the course on screen, from the page's focused
/// value; disabled when no course is shown. Download Files…, Let My AI App Read Materials and
/// Hide Course are edits (M2).
struct CourseMenuCommands: Commands {
    let model: AppModel
    @FocusedValue(\.courseCommands) private var course

    var body: some Commands {
        let l10n = model.menuL10n
        CommandMenu(l10n("mac.menu.course")) {
            Button(l10n("mac.actions.openCourseWebsite")) {
                AppActions.open(course?.website)
            }
            .disabled(course?.website == nil)
            Divider()
            Button(l10n("mac.actions.aiPolicy")) { course?.showAIPolicy() }
                .disabled(course == nil)
            Button(l10n("mac.actions.setTermDates")) { course?.showTermDates() }
                .disabled(course == nil)
        }
    }
}

#if PAGELAMP_PREVIEW
/// Debug (preview build only): switch mock scenarios or to live data, run mock syncs.
struct DebugCommands: Commands {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow
    @AppStorage(DebugPreferences.sidebarCapsuleLeads) private var capsuleLeads = false
    @AppStorage(DebugPreferences.systemSectionPicker) private var systemSectionPicker = false

    var body: some Commands {
        let l10n = model.menuL10n
        let isMock = if case .mock = model.dataMode { true } else { false }

        CommandMenu(l10n("mac.debug.menu")) {
            Menu(l10n("mac.debug.dataSource")) {
                Picker(l10n("mac.debug.mockData"), selection: scenario) {
                    ForEach(MockScenario.allCases, id: \.self) { scenario in
                        Text(Self.title(of: scenario, l10n: l10n)).tag(Optional(scenario))
                    }
                }
                .pickerStyle(.inline)
                Divider()
                // Shares data with the installed PageLamp: asks first (RootView's alert).
                Toggle(l10n("mac.debug.liveData"), isOn: live)
            }
            .disabled(model.isSyncing)
            Divider()
            Button(l10n("mac.debug.runMockSync")) {
                Task { await model.syncAll() }
            }
            .disabled(!isMock || !model.canSync)
            Button(l10n("mac.debug.runMockSyncRejected")) {
                Task { await model.runMockSyncWithRejectedToken() }
            }
            .disabled(!isMock || !model.canSync)
            Divider()
            Toggle(l10n("mac.debug.capsuleLeads"), isOn: $capsuleLeads)
            Toggle(l10n("mac.debug.systemSectionPicker"), isOn: $systemSectionPicker)
        }
    }

    private var scenario: Binding<MockScenario?> {
        Binding(
            get: { if case .mock(let scenario) = model.dataMode { scenario } else { nil } },
            set: { scenario in
                guard let scenario else { return }
                Task { await model.useMock(scenario) }
            }
        )
    }

    private var live: Binding<Bool> {
        Binding(
            get: { model.dataMode == .live },
            // The confirmation is the main window's alert: open it first (it may be closed).
            set: { live in
                guard live, model.dataMode != .live else { return }
                Task { await model.perform(.liveData) { openWindow(id: PageLampScenes.mainWindowID) } }
            }
        )
    }

    private static func title(of scenario: MockScenario, l10n: L10n) -> String {
        switch scenario {
        case .demo: l10n("mac.debug.scenario.demo")
        case .empty: l10n("mac.debug.scenario.empty")
        case .expired: l10n("mac.debug.scenario.expired")
        case .error: l10n("mac.debug.scenario.error")
        case .busy: l10n("mac.debug.scenario.busy")
        case .crashed: l10n("mac.debug.scenario.crashed")
        case .aiKey: l10n("mac.debug.scenario.aiKey")
        case .aiLocal: l10n("mac.debug.scenario.aiLocal")
        case .aiUnpriced: l10n("mac.debug.scenario.aiUnpriced")
        case .aiBudget: l10n("mac.debug.scenario.aiBudget")
        case .aiDisclosureChanged: l10n("mac.debug.scenario.aiDisclosureChanged")
        case .aiErrors: l10n("mac.debug.scenario.aiErrors")
        case .weeklyNoteMonday: l10n("mac.debug.scenario.weeklyNoteMonday")
        }
    }
}
#endif
