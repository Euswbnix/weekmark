// Headless snapshots of the content layer: every page of every screen in its states, rendered
// with ImageRenderer on mock data at a fixed moment, light/dark × en/zh-Hans. NavigationSplitView,
// the toolbar, the inspector column and glass don't render offscreen, so pages render without the
// window chrome (spec §7.4 still needs the on-device review).
//
//     swift run PageLampSnapshots <directory> [name-prefix …]
//     PAGELAMP_SNAPSHOT_DIR=<directory> swift test --filter SnapshotRenderTests
//
// This target is the catalogue and the PNG writer; the pages themselves are views of the
// PageLamp module, reached through `package` access, so none of this is linked into the app.
// A screen adds its states to `SnapshotCatalog.pages` (`ThisWeekSnapshots`, `CourseSnapshots`,
// `SetupSnapshots`, `SidebarSnapshots`, `WhatsNewSnapshots`, `RemindersSnapshots`): each page says how to set up its model (`SnapshotSetup`)
// and loads what it needs in `make`, since `.task` never runs offscreen.

import AppKit
import SwiftUI
import PageLamp
import PageLampKit
import PageLampModel

/// How a snapshot page's model is set up: the mock scenario, the moment, answers to replace.
struct SnapshotSetup: Sendable {
    var scenario: MockScenario = .demo
    /// The moment of the snapshot, from the harness's "now" (Friday 10:00).
    var moment: @Sendable (_ now: Date, _ calendar: Calendar) -> Date = { now, _ in now }
    /// Wraps the mock (usually in a `FixtureService`) for states the demo data doesn't have.
    var service: @Sendable (_ mock: MockService, _ now: Date, _ calendar: Calendar) -> any PageLampService = { mock, _, _ in mock }
    /// The mock's sync speed (a page that renders mid-sync needs one in flight).
    var syncStep: Duration = .zero
    /// The system's languages differ from the app's (menus follow the system: Reopen Now).
    var otherSystemLanguage = false
    /// The notification permission macOS has for PageLamp, and what its prompt answers.
    var notificationPermission: NotificationPermission = .notDetermined
    var notificationAnswer = true
}

/// One snapshot-able page.
public struct SnapshotPage {
    public let name: String
    let width: CGFloat
    let minHeight: CGFloat
    let setup: SnapshotSetup
    /// Loads what the page needs, then returns its document view.
    let make: @MainActor (AppModel) async -> AnyView

    init(
        name: String,
        width: CGFloat = SnapshotCatalog.detailWidth,
        minHeight: CGFloat = WindowMetrics.mainHeight,
        setup: SnapshotSetup = SnapshotSetup(),
        make: @escaping @MainActor (AppModel) async -> AnyView
    ) {
        self.name = name
        self.width = width
        self.minHeight = minHeight
        self.setup = setup
        self.make = make
    }
}

public enum SnapshotCatalog {
    /// The detail column at the main window's default size, minus the sidebar.
    static let detailWidth = WindowMetrics.mainWidth - WindowMetrics.sidebarIdeal

    /// Every page, by screen.
    public static let pages: [SnapshotPage] =
        ThisWeekSnapshots.pages + CourseSnapshots.pages + SetupSnapshots.pages + SidebarSnapshots.pages
        + WhatsNewSnapshots.pages + RemindersSnapshots.pages + AiSettingsSnapshots.pages + PlanSnapshots.pages + ExplainSnapshots.pages
        + WeeklyNoteSnapshots.pages + [
            SnapshotPage(name: "components") { _ in AnyView(ComponentGallery()) },
        ]

    /// The page called `name`, if any.
    public static func page(named name: String) -> SnapshotPage? {
        pages.first { $0.name == name }
    }
}

public enum SnapshotRenderer {
    public struct Variant: Sendable {
        public let language: AppLanguage
        public let dark: Bool

        public var suffix: String {
            "\(language == .simplifiedChinese ? "zh-Hans" : "en")-\(dark ? "dark" : "light")"
        }
    }

    public static let variants: [Variant] = [AppLanguage.english, .simplifiedChinese].flatMap { language in
        [false, true].map { Variant(language: language, dark: $0) }
    }

    /// The harness's moment: Friday 2026-09-25 10:00 in `calendar`.
    public static func defaultNow(in calendar: Calendar) -> Date {
        calendar.date(from: DateComponents(year: 2026, month: 9, day: 25, hour: 10)) ?? Date()
    }

    /// One rendered PNG.
    public struct Rendered: Sendable {
        public let name: String
        public let file: URL
        public let width: Int
        public let height: Int
    }

    /// Renders the pages whose names start with one of `prefixes` (all pages without), or
    /// exactly the pages called `names`, in `variants` into `directory` as
    /// `<page>-<lang>-<scheme>.png`. Mock data at a fixed moment, so reruns are comparable.
    public static func renderAll(
        to directory: URL,
        prefixes: [String] = [],
        names: [String]? = nil,
        variants: [Variant] = SnapshotRenderer.variants,
        calendar: Calendar = .current,
        now: Date? = nil,
        scale: CGFloat = 2
    ) async throws -> [Rendered] {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let now = now ?? defaultNow(in: calendar)
        let pages = SnapshotCatalog.pages.filter { page in
            if let names { return names.contains(page.name) }
            return prefixes.isEmpty || prefixes.contains { page.name.hasPrefix($0) }
        }
        var rendered: [Rendered] = []
        for variant in variants {
            for page in pages {
                let model = await model(for: page, language: variant.language, calendar: calendar, now: now)
                let content = await page.make(model)
                let url = directory.appending(path: "\(page.name)-\(variant.suffix).png")
                let size = try render(content, page: page, model: model, dark: variant.dark, scale: scale, to: url)
                rendered.append(Rendered(name: "\(page.name)-\(variant.suffix)", file: url, width: size.width, height: size.height))
                // A page may leave a sync running (rendered mid-sync): let it finish first.
                await finishSync(model)
                // Each render holds the main actor for a while: let other main-actor work (tests
                // running in parallel) through between renders.
                await Task.yield()
            }
        }
        return rendered
    }

    /// The model of `page` in `language`, loaded.
    public static func model(for page: SnapshotPage, language: AppLanguage, calendar: Calendar, now: Date) async -> AppModel {
        let setup = page.setup
        let moment = setup.moment(now, calendar)
        let languages = (language == .simplifiedChinese) != setup.otherSystemLanguage ? ["zh-Hans"] : ["en"]
        let mock = MockService(
            scenario: setup.scenario,
            timing: MockService.Timing(latency: .zero, syncStep: setup.syncStep),
            calendar: calendar,
            now: { moment }
        )
        let model = AppModel(
            dataMode: .mock(setup.scenario),
            strings: .app,
            settings: InMemorySettingsStore(language: language),
            // Capsule timers long enough never to fire during a render.
            timing: AppModel.Timing(finishedCapsule: .seconds(60), failedCapsule: .seconds(60), mock: .instant),
            calendar: calendar,
            clock: { moment },
            notificationCenter: NotificationCenter(),
            preferredLanguages: { languages },
            service: setup.service(mock, moment, calendar),
            // Reminders, the menu bar extra, Settings ▸ AI, Plan, Explain and the weekly note (M3) are on
            // in preview builds, which these are; notifications stay in memory.
            reminders: true,
            reminderCenters: ReminderCenters(
                live: { RecordingNotificationCenter() },
                mock: RecordingNotificationCenter(permission: setup.notificationPermission, answer: setup.notificationAnswer)
            ),
            aiSettings: true,
            aiPlan: true,
            aiExplain: true,
            aiNote: true
        )
        await model.refresh()
        // The pass the refresh asked for decides whether "Remind me" shows: let it finish.
        await model.reminderDelivery?.settle()
        // This Week's weekly note, read now (its `.task` comes too late for the render).
        if let note = model.weeklyNote {
            await note.load()
            await note.estimate.refresh()
        }
        return model
    }

    private static func finishSync(_ model: AppModel) async {
        let clock = ContinuousClock()
        let deadline = clock.now + .seconds(10)
        while model.isSyncing, clock.now < deadline {
            try? await Task.sleep(for: .milliseconds(5))
        }
    }

    private static func render(
        _ content: AnyView, page: SnapshotPage, model: AppModel, dark: Bool, scale: CGFloat, to url: URL
    ) throws -> (width: Int, height: Int) {
        // The environment goes outermost so the background resolves in the same appearance.
        let view = content
            .frame(width: page.width)
            .frame(minHeight: page.minHeight, alignment: .top)
            .background(Color(nsColor: .windowBackgroundColor))
            .pageLampEnvironment(model)
            .environment(\.colorScheme, dark ? .dark : .light)
            .environment(\.detailColumnWidth, page.width)
            // Native pickers draw as placeholders offscreen: plain stand-ins show the layout.
            .environment(\.drawsControlStandIns, true)
        let renderer = ImageRenderer(content: view)
        renderer.scale = scale
        // Propose the page width: with no proposal, a page measured at its ideal size can come
        // out a few points taller than it lays out (transparent strips above and below).
        renderer.proposedSize = ProposedViewSize(width: page.width, height: nil)
        // Dynamic NSColors (PLColor, system colours) resolve against the current appearance.
        var image: CGImage?
        let appearance = NSAppearance(named: dark ? .darkAqua : .aqua) ?? NSAppearance.currentDrawing()
        appearance.performAsCurrentDrawingAppearance {
            image = renderer.cgImage
        }
        guard let image,
              let png = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
        else {
            throw CocoaError(.fileWriteUnknown, userInfo: [NSFilePathErrorKey: url.path])
        }
        try png.write(to: url)
        return (image.width, image.height)
    }
}
