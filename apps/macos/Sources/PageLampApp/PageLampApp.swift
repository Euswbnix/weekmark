// The PageLamp Preview app. Everything lives in the PageLamp library; this target only owns the
// app's model and delegate. The executable is "PageLampApp" (never "pagelamp": the bundled CLI
// is Contents/MacOS/pagelamp and the file system is case-insensitive).

import SwiftUI
import PageLamp
import PageLampModel

@main
struct PageLampApp: App {
    @NSApplicationDelegateAdaptor(PageLampAppDelegate.self) private var appDelegate
    /// Starts on mock data (the preview's default); Debug ▸ Data Source switches. Reminders,
    /// Settings ▸ AI, Plan with PageLamp and Explain (M3) are on in preview builds until they ship.
    #if PAGELAMP_PREVIEW
    @State private var model = AppModel(
        strings: .app, reminders: true, aiSettings: true, aiPlan: true, aiExplain: true, aiNote: true
    )
    #else
    @State private var model = AppModel(strings: .app)
    #endif

    var body: some Scene {
        PageLampScenes(model: model)
    }
}
