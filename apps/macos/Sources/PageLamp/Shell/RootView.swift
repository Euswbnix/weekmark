// The main window (spec §2.3–§2.4, §3.9 S1/S2): a two-column split view, no back stack.

import AppKit
import SwiftUI
import PageLampModel

public struct RootView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @Environment(\.openWindow) private var openWindow

    /// Restored per window (spec §2.4): the destination and whether the inspector is open.
    @SceneStorage("destination") private var storedDestination = ""
    @SceneStorage("inspectorShown") private var storedInspector = false
    /// The inspector opens by itself only on the first course visit, on a wide window.
    @AppStorage("inspector.firstCourseVisitDone") private var firstCourseVisitDone = false

    @State private var columns: NavigationSplitViewVisibility = .all
    @State private var width: CGFloat = PLSize.windowMainWidth
    @State private var restored = false

    public init() {}

    public var body: some View {
        @Bindable var model = model
        NavigationSplitView(columnVisibility: $columns) {
            SidebarList()
                .minimumSizeShield()
                .navigationSplitViewColumnWidth(min: PLSize.sidebarMin, ideal: PLSize.sidebarIdeal, max: PLSize.sidebarMax)
        } detail: {
            DetailColumn()
        }
        .frame(minWidth: PLSize.windowMainMinWidth, minHeight: PLSize.windowMainMinHeight)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .diagnosticReportSheet(host: .main)
        .whatsNewSheet()
        .alert(l10n("mac.debug.live.title"), isPresented: $model.confirmingLiveData) {
            Button(l10n("common.actions.cancel"), role: .cancel) {}
                .keyboardShortcut(.defaultAction)
            Button(l10n("mac.debug.live.confirm")) {
                Task { await model.useLive() }
            }
        } message: {
            Text(l10n("mac.debug.live.message"))
        }
        .task {
            restore()
            // A click on a reminder's notification opens this window, also after it was closed.
            let openWindow = self.openWindow
            model.openMainWindow = { openWindow(id: PageLampScenes.mainWindowID) }
            await model.start()
            await PerfProbe.runIfRequested(model: model)  // no-op unless PAGELAMP_PERF_PROBE is set
        }
        .onChange(of: model.phase, initial: true) { _, phase in
            // S2: the whole window explains the problem; the sidebar would only show nothing.
            if case .unavailable = phase {
                columns = .detailOnly
            } else if columns == .detailOnly {
                columns = .all
            }
        }
        .onChange(of: model.destination) { _, destination in
            guard restored else { return }
            storedDestination = Self.encode(destination)
            if case .course = destination, !firstCourseVisitDone {
                firstCourseVisitDone = true
                if width >= PLSize.inspectorOpenAt { model.inspectorShown = true }
            }
        }
        .onChange(of: model.inspectorShown) { _, shown in
            if restored { storedInspector = shown }
        }
        .appAppearance(model.appearance)
        .onReceive(NotificationCenter.default.publisher(for: PerfProbe.toggleSidebar)) { _ in  // performance probe only
            withAnimation { columns = columns == .detailOnly ? .all : .detailOnly }
        }
    }

    /// The first main window of the run restores where the student was; a reopened one (a menu
    /// command with the window closed) keeps the model's destination, which the command set.
    private func restore() {
        guard !restored else { return }
        if model.restoredWindowState {
            storedDestination = Self.encode(model.destination)
            storedInspector = model.inspectorShown
        } else {
            if let destination = Self.decode(storedDestination) { model.destination = destination }
            model.inspectorShown = storedInspector
            model.restoredWindowState = true
        }
        restored = true
    }

    private static func encode(_ destination: Destination) -> String {
        (try? JSONEncoder().encode(destination)).flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }

    private static func decode(_ text: String) -> Destination? {
        guard !text.isEmpty else { return nil }
        return try? JSONDecoder().decode(Destination.self, from: Data(text.utf8))
    }
}

/// The detail column for the current phase and destination. Its width reaches the pages as
/// `detailColumnWidth` (rows with a compact layout read it; the course page measures its own,
/// beside the inspector).
struct DetailColumn: View {
    @Environment(AppModel.self) private var model
    @State private var width: CGFloat?
    /// One detail model per course visited in this window (the course page and its inspector
    /// share them; going back to a course shows its data at once).
    @State private var details = CourseDetailStore()
    @State private var nextFrame = NextFrame()
    @AppStorage(DebugPreferences.sidebarCapsuleLeads) private var capsuleLeads = false

    var body: some View {
        Group {
            switch model.phase {
            case .loading:
                LoadingState()
            case .unavailable(let failure):
                BackendUnavailableView(failure: failure)
            case .ready:
                switch model.pageDestination {
                case .thisWeek: ThisWeekView()
                case .course(let id): CourseDetailView(courseId: id, detail: details.model(for: id))
                case .sources: SourcesView()
                case .connect: ConnectView()
                }
            }
        }
        .environment(\.detailColumnWidth, width)
        .onDetailWidthChange { width = $0 }
        .minimumSizeShield()
        .toolbar { WindowToolbar(compact: (width ?? .infinity) < WindowToolbar.compactBelow) }
        // The page and its toolbar follow the choice at once, or (Debug: the capsule leads) two
        // display frames later, once the capsule's slide is with the render server.
        .onChange(of: model.destination) { _, _ in
            if capsuleLeads {
                nextFrame.run(afterFrames: 2) { model.showDestinationPage() }
            } else {
                model.showDestinationPage()
            }
        }
        .onAppear { model.showDestinationPage() }
    }
}

/// Runs work a few display frames from now, after the current update has been committed. One
/// frame is not enough to separate two commits: the update cycle can flush the current changes
/// together with the next frame's. The latest request wins; it runs once.
@MainActor
final class NextFrame: NSObject {
    private var link: CADisplayLink?
    private var work: (() -> Void)?
    private var ticks = 0
    private var frames = 1

    func run(afterFrames frames: Int, _ work: @escaping () -> Void) {
        self.work = work
        self.frames = frames
        guard link == nil else { return }
        guard let screen = NSScreen.main else {
            DispatchQueue.main.async { self.fire() }
            return
        }
        let link = screen.displayLink(target: self, selector: #selector(tick(_:)))
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    @objc private func tick(_ link: CADisplayLink) {
        ticks += 1
        if ticks >= frames { fire() }
    }

    private func fire() {
        link?.invalidate()
        link = nil
        ticks = 0
        let work = self.work
        self.work = nil
        work?()
    }
}

/// Answers a minimum-size query (a zero proposal) without laying out the content.
///
/// The window asks its content for a minimum size on every layout pass, which is every frame
/// while the sidebar or the inspector animates. A page's scroll view answered it by laying out
/// all of its text at zero width (one word per line): about a third of the main thread during
/// those animations. The window's real minimum comes from RootView's explicit frame.
struct MinimumSizeShield: Layout {
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let content = subviews.first else { return .zero }
        if proposal.width == 0 || proposal.height == 0 {
            return CGSize(width: proposal.width ?? 0, height: proposal.height ?? 0)
        }
        return content.sizeThatFits(proposal)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(bounds.size))
    }
}

extension View {
    /// See `MinimumSizeShield`. Use it on every content root hosted in its own NSHostingView
    /// (the detail column, and both sides of an `.inspector`, which SwiftUI hosts separately).
    func minimumSizeShield() -> some View {
        MinimumSizeShield { self }
    }
}
