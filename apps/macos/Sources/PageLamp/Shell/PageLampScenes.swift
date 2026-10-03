// Scenes (spec §2.1): the main window, Settings and, in preview builds until it ships, the menu
// bar extra (M3; off until the student turns it on in Settings ▸ General). The welcome window (M2)
// comes later.

import AppKit
import SwiftUI
import PageLampModel

/// The app's scenes; `PageLampApp` (the executable) owns the model and the delegate.
public struct PageLampScenes: Scene {
    /// The main window's scene id (`openWindow(id:)`).
    static let mainWindowID = "main"

    /// Settings ▸ General's "Show PageLamp in the menu bar" (spec §2.1: off until chosen).
    static let showInMenuBarKey = "showInMenuBar"

    let model: AppModel
    @AppStorage(PageLampScenes.showInMenuBarKey) private var showInMenuBar = false

    public init(model: AppModel) {
        self.model = model
    }

    public var body: some Scene {
        Window(model.menuL10n("mac.app.name"), id: Self.mainWindowID) {
            RootView()
                .pageLampEnvironment(model)
        }
        .defaultSize(width: PLSize.windowMainWidth, height: PLSize.windowMainHeight)
        .windowResizability(.contentMinSize)
        .commands {
            SidebarCommands()
            InspectorCommands()
            ToolbarCommands()
            PageLampCommands(model: model)
            CourseMenuCommands(model: model)
            #if PAGELAMP_PREVIEW
            DebugCommands(model: model)
            #endif
        }

        Settings {
            SettingsView()
                .pageLampEnvironment(model)
        }

        #if PAGELAMP_PREVIEW
        MenuBarExtra(isInserted: $showInMenuBar) {
            MenuBarWeekView()
                .pageLampEnvironment(model)
        } label: {
            MenuBarLabel()
                .environment(model)
        }
        .menuBarExtraStyle(.window)
        #endif
    }
}

/// Last window closed → quit, unless PageLamp is in the menu bar (spec §2.2).
public final class PageLampAppDelegate: NSObject, NSApplicationDelegate {
    override public init() {
        super.init()
    }

    public func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        !UserDefaults.standard.bool(forKey: PageLampScenes.showInMenuBarKey)
    }
}
