// The course section picker's control (spec §3.2.1; user decision 2026-09-27, final): a custom
// segmented control that reads as the system's 27 tabs control (track, thumb, labels, heights and
// spacing as measured on 27.2, `SegmentedLayout`), whose selected segment is the shared glass
// thumb (GlassThumb.swift). The thumb slides on every change: a click (a trackpad tap too), Space,
// VoiceOver's press, a menu command, a cross-link that opens a course at a section. It is a Core
// Animation spring started in the input's own handler and drawn by the render server, so the new
// section's build (~20 ms of main thread) never makes it stutter; no motion gate. The control's
// own input writes the selection on the next run-loop turn, so the slide is committed (and on
// screen) before that build starts.
//
// An `NSControl`, so AppKit keeps the system control's focus rules (a Tab stop only with Full
// Keyboard Access on; a click never takes focus, as with the system control; first mouse), draws
// its focus ring (`drawFocusRingMask`) and routes keys; the accessibility tree (a tab group of
// one tab button per section, as the system control reports it) is in GlassSegmentedAccessibility.swift,
// the behaviour rules in `SegmentedNavigation`. The track is SwiftUI, behind the control
// (`SegmentedTrack`); the narrow fallback stays the system pop-up menu (CourseSectionPicker).

import AppKit
import QuartzCore
import SwiftUI
import PageLampModel

/// The wide section picker. The control is the accessibility element; the one SwiftUI modifier on it
/// is the label (CourseSectionPicker), which SwiftUI's own node for the focused view reads.
struct GlassSegmentedPicker: NSViewRepresentable {
    var titles: [String]
    var selection: Int
    /// Where the thumb stands when the control is made (`CourseUIState.pickerSection`): a page built
    /// at another section (a cross-link) slides from there once, in its first frame.
    var start: Int
    var accessibilityLabel: String
    /// False under Reduce Motion: the thumb jumps.
    var animates: Bool
    var onSelect: (Int) -> Void

    func makeNSView(context: Context) -> GlassSegmentedControl {
        GlassSegmentedControl(titles: titles, start: start)
    }

    func updateNSView(_ control: GlassSegmentedControl, context: Context) {
        control.update(
            titles: titles, selection: selection, label: accessibilityLabel, animates: animates,
            changeAnimates: !context.transaction.disablesAnimations,
            enabled: context.environment.isEnabled, onSelect: onSelect
        )
    }

    /// Its own size whatever is proposed (the system control at `.fixedSize()`), so ViewThatFits
    /// switches to the pop-up menu at the same widths.
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: GlassSegmentedControl, context: Context) -> CGSize? {
        nsView.segmentLayout.size
    }
}

/// The glass thumb and the labels above it; pointer, keys and the focus ring.
final class GlassSegmentedControl: NSControl {
    /// Text widths as AppKit measures a segment's label.
    static func labelWidths(_ titles: [String]) -> [CGFloat] {
        let font = NSFont.systemFont(ofSize: SegmentedMetrics.fontSize)
        return titles.map { NSAttributedString(string: $0, attributes: [.font: font]).size().width }
    }

    private(set) var segmentLayout: SegmentedLayout
    private(set) var titles: [String]
    /// The selection as the control shows it: written at a commit, before the binding comes back.
    private(set) var selection: Int
    private(set) var groupLabel = ""
    private var navigation: SegmentedNavigation
    /// Reduce Motion off: every change slides.
    private var animates = true
    private var onSelect: (Int) -> Void = { _ in }
    private let mover = GlassThumbMover(
        view: GlassThumbView(shape: .rounded(SegmentedMetrics.thumbRadius), outline: false, ring: false)
    )
    private let labels = SegmentLabelsView()
    /// One VoiceOver element per segment, kept for the control's life (relabelled in place).
    private(set) var elements: [GlassSegmentElement] = []
    /// A page built for a cross-link: the segment the thumb slides to once the control is in its
    /// window (the page's first frame).
    private var goal: Int?
    private var hasBeenInWindow = false
    /// When the last slide was added (the performance probe: a read in that same display frame
    /// sees the new position before its animation is committed).
    private(set) var lastSlideTime: CFTimeInterval = 0
    /// A selection this control made and hasn't written to the binding yet (`commit`): until it is
    /// written, updates that still carry the old value leave the thumb alone.
    private var pendingCommit: Int?

    init(titles: [String], start: Int) {
        self.titles = titles
        segmentLayout = SegmentedLayout(labelWidths: Self.labelWidths(titles))
        let start = min(max(start, 0), max(titles.count - 1, 0))
        selection = start
        navigation = SegmentedNavigation(count: titles.count, selection: start)
        super.init(frame: NSRect(origin: .zero, size: segmentLayout.size))
        wantsLayer = true
        addSubview(mover.view)
        labels.frame = bounds
        labels.autoresizingMask = [.width, .height]
        addSubview(labels)
        labels.show(titles: titles, layout: segmentLayout)
        if !titles.isEmpty { mover.place(segmentLayout.thumbFrame(start)) }
        elements = titles.indices.map { GlassSegmentElement(control: self, index: $0) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    override var isFlipped: Bool { true }
    override var intrinsicContentSize: NSSize { segmentLayout.size }
    /// The first click in an inactive window selects, like the system control's.
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
    /// Every point inside takes the pointer (the thumb and the labels never do).
    override func hitTest(_ point: NSPoint) -> NSView? {
        bounds.contains(convert(point, from: superview)) ? self : nil
    }

    var keySegment: Int { navigation.keySegment }
    var isFirstResponder: Bool { window?.firstResponder === self }
    /// The thumb's layer (the performance probe reads it once per frame).
    var thumbLayer: CALayer? { mover.layer }

    /// Segment `index`'s centre in window coordinates (where the performance probe clicks).
    func segmentCenter(_ index: Int) -> NSPoint {
        let frame = segmentLayout.thumbFrame(index)
        return convert(NSPoint(x: frame.midX, y: frame.midY), to: nil)
    }

    /// `animates`: Reduce Motion is off. `changeAnimates`: this update's transaction allows
    /// animation (a selection change made with animations disabled jumps).
    func update(
        titles newTitles: [String], selection newSelection: Int, label: String, animates: Bool, changeAnimates: Bool,
        enabled: Bool, onSelect: @escaping (Int) -> Void
    ) {
        self.onSelect = onSelect
        self.animates = animates
        groupLabel = label
        if isEnabled != enabled {
            isEnabled = enabled
            if !enabled { handle(.pointerCancelled) }
        }
        if newTitles != titles { retitle(newTitles) }
        guard !titles.isEmpty else { return }
        let new = min(max(newSelection, 0), titles.count - 1)
        // The binding hasn't caught up with this control's own commit yet.
        if let pendingCommit, new != pendingCommit { return }
        guard new != selection else { return }
        // From outside (the menu, the Go menu, a cross-link): the thumb follows unless a press is
        // down. No notification: the system control posts none either (VoiceOver reads the value
        // when it next looks at the control).
        selection = new
        // A page built for a cross-link comes in a transaction that never animates (the page is
        // new); its thumb still slides once, in the page's first frame.
        handle(.selectionChanged(new), animated: animates && (changeAnimates || !hasBeenInWindow))
    }

    /// A live language switch: new widths, the thumb at the selection at once, the same elements.
    private func retitle(_ newTitles: [String]) {
        titles = newTitles
        segmentLayout = SegmentedLayout(labelWidths: Self.labelWidths(newTitles))
        selection = min(selection, max(newTitles.count - 1, 0))
        navigation = SegmentedNavigation(count: newTitles.count, selection: selection)
        labels.show(titles: newTitles, layout: segmentLayout)
        goal = nil
        if newTitles.isEmpty { mover.hide() } else { mover.show(segmentLayout.thumbFrame(selection), fadeIn: nil) }
        if elements.count != newTitles.count {
            elements = newTitles.indices.map { GlassSegmentElement(control: self, index: $0) }
        }
        invalidateIntrinsicContentSize()
        noteFocusRingMaskChanged()
    }

    // MARK: Window

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        NotificationCenter.default.removeObserver(self, name: NSWindow.didBecomeKeyNotification, object: nil)
        NotificationCenter.default.removeObserver(self, name: NSWindow.didResignKeyNotification, object: nil)
        guard let window else { return }
        for name in [NSWindow.didBecomeKeyNotification, NSWindow.didResignKeyNotification] {
            NotificationCenter.default.addObserver(self, selector: #selector(windowKeyChanged), name: name, object: window)
        }
        hasBeenInWindow = true
        PerfProbe.register(segmentedControl: self)  // does nothing unless the probe runs
        if let goal {
            self.goal = nil
            moveThumb(.segment(goal), animated: animates)
        }
    }

    /// The focus ring shows only in the key window.
    @objc private func windowKeyChanged() {
        noteFocusRingMaskChanged()
    }

    override func viewWillMove(toWindow newWindow: NSWindow?) {
        super.viewWillMove(toWindow: newWindow)
        // A press can't end outside a window.
        if newWindow == nil { handle(.pointerCancelled) }
    }

    // MARK: Pointer

    override func mouseDown(with event: NSEvent) {
        guard isEnabled else { return }
        // A press still down (its mouse-up got lost) ends first.
        if navigation.pressedSegment != nil { handle(.pointerCancelled) }
        handle(.pointerDown(x: x(of: event)))
    }

    override func mouseDragged(with event: NSEvent) {
        guard isEnabled else { return }
        handle(.pointerDragged(x: x(of: event)))
    }

    override func mouseUp(with event: NSEvent) {
        guard isEnabled else { return }
        handle(.pointerUp(x: x(of: event)))
    }

    private func x(of event: NSEvent) -> CGFloat {
        convert(event.locationInWindow, from: nil).x
    }

    // MARK: Keys and focus

    override func keyDown(with event: NSEvent) {
        let modified = !event.modifierFlags.intersection([.shift, .control, .option, .command]).isEmpty
        let key: SegmentedEvent = switch event.specialKey {
        case .leftArrow?: .left
        case .rightArrow?: .right
        case .downArrow?: .down
        default: event.charactersIgnoringModifiers == " " && !modified ? .space : .otherKey
        }
        // Return, ↑, Home, End, Page Up/Down, letters, Esc, a modified Space … go on, as from the
        // system control (measured: the same keys reach the window unhandled).
        guard isEnabled else {
            super.keyDown(with: event)
            return
        }
        switch handle(key) {
        case .ignored: super.keyDown(with: event)
        // Space on the selected segment: VoiceOver hears the focused segment again, as from the
        // system control (a Space that selects posts it in `commit`).
        case .handled where key == .space: if isFirstResponder { postKeySegmentFocus() }
        default: break
        }
    }

    /// AppKit itself tells VoiceOver about the focused segment (`accessibilityFocusedUIElement`)
    /// when the control becomes first responder; a post here would make it speak twice.
    override func becomeFirstResponder() -> Bool {
        guard super.becomeFirstResponder() else { return false }
        noteFocusRingMaskChanged()
        return true
    }

    override func resignFirstResponder() -> Bool {
        guard super.resignFirstResponder() else { return false }
        noteFocusRingMaskChanged()
        return true
    }

    /// AppKit's own focus ring (the focus colour, 3 pt outside, the zoom-in) around the key
    /// segment's thumb, as the system control draws it; none while the window isn't key (AppKit
    /// keeps a custom view's ring otherwise; `windowKeyChanged` asks again).
    override var focusRingMaskBounds: NSRect {
        titles.isEmpty || window?.isKeyWindow != true ? .zero : segmentLayout.thumbFrame(navigation.keySegment)
    }

    override func drawFocusRingMask() {
        guard !titles.isEmpty, window?.isKeyWindow == true else { return }
        let radius = SegmentedMetrics.thumbRadius
        NSBezierPath(roundedRect: segmentLayout.thumbFrame(navigation.keySegment), xRadius: radius, yRadius: radius).fill()
    }

    // MARK: Accessibility entry points (GlassSegmentElement)

    /// VoiceOver's press: selects and slides; focus stays where it is, and nothing is announced
    /// beyond the focused segment's own notification (`commit`), as from the system control.
    func accessibilityPress(segment index: Int) -> Bool {
        guard isEnabled else { return false }
        // The selected segment pressed on a focused control whose focus is already there: VoiceOver
        // hears it again, as from the system control (a press that moves the focus or selects
        // announces in `handle` / `commit`).
        if case .handled = handle(.accessibilityPress(index)), index == selection, isFirstResponder {
            postKeySegmentFocus()
        }
        return true
    }

    /// VoiceOver set its focus on a segment: it becomes the key segment of a focused control.
    func accessibilityFocus(segment index: Int) {
        guard isEnabled else { return }
        let wasFocused = isFirstResponder
        handle(.accessibilityFocus(index))
        if !wasFocused { window?.makeFirstResponder(self) }
    }

    /// A segment's VoiceOver frame in screen coordinates (AppKit's numbers, `SegmentedLayout`).
    func accessibilityScreenFrame(segment index: Int) -> NSRect {
        guard let window, index < segmentLayout.count else { return .zero }
        return window.convertToScreen(convert(segmentLayout.accessibilityFrame(index), to: nil))
    }

    // MARK: Behaviour

    /// `animated`: nil for input (slides unless Reduce Motion is on).
    @discardableResult
    private func handle(_ event: SegmentedEvent, animated: Bool? = nil) -> SegmentedOutcome {
        let keyBefore = navigation.keySegment
        let outcome = navigation.handle(event, in: segmentLayout, selection: selection)
        switch outcome {
        case .ignored, .handled, .keySegment: break
        case .thumb(let thumb): moveThumb(thumb, animated: animated ?? animates)
        case .commit(let index): commit(index)
        }
        if navigation.keySegment != keyBefore {
            noteFocusRingMaskChanged()
            if isFirstResponder { postKeySegmentFocus() }
        }
        return outcome
    }

    /// The thumb goes first, in this same turn; the selection is written to the binding on the next
    /// turn. The slide reaches the render server before the new section is built (~20 ms of main
    /// thread, in the update that write causes): with this turn's Core Animation commit, or at once
    /// when no transaction is open (VoiceOver's press; `GlassThumbMover.slide`). No forced flush: a
    /// flush here made back-to-back switches stall the content's crossfade.
    private func commit(_ index: Int) {
        moveThumb(.segment(index), animated: animates)
        selection = index
        pendingCommit = index
        let onSelect = onSelect
        DispatchQueue.main.async { [weak self] in
            if let self {
                // A later commit, made before this one was written, writes its own selection.
                guard self.pendingCommit == index else { return }
                self.pendingCommit = nil
            }
            onSelect(index)
        }
        // As from the system control: no value notification; a focused control re-announces its
        // focused segment.
        if isFirstResponder { postKeySegmentFocus() }
    }

    private func moveThumb(_ thumb: SegmentedOutcome.Thumb, animated: Bool) {
        guard !titles.isEmpty else { return }
        guard window != nil else {
            // Not on screen yet: a page built for a cross-link slides once it is (its first
            // frame). A control that has left its window just follows.
            if case .segment(let index) = thumb, animated, !hasBeenInWindow {
                goal = index
            } else {
                goal = nil
                mover.place(frame(for: thumb))
            }
            return
        }
        if animated {
            if mover.slide(to: frame(for: thumb), spring: PLMotion.sectionSpring) { lastSlideTime = CACurrentMediaTime() }
        } else {
            mover.place(frame(for: thumb))
        }
    }

    private func frame(for thumb: SegmentedOutcome.Thumb) -> NSRect {
        switch thumb {
        case .segment(let index): segmentLayout.thumbFrame(index)
        case .following(let x): segmentLayout.thumbFrame(followingX: x)
        }
    }

    private func postKeySegmentFocus() {
        guard elements.indices.contains(navigation.keySegment) else { return }
        NSAccessibility.post(element: elements[navigation.keySegment], notification: .focusedUIElementChanged)
    }
}

/// The segment titles above the glass: the system font, Regular, `labelColor` in every state
/// (selected, pressed, disabled, inactive), the ink itself with Increase Contrast.
final class SegmentLabelsView: NSView {
    private static let color = NSColor(name: nil) { appearance in
        switch appearance.bestMatch(from: [.aqua, .darkAqua, .accessibilityHighContrastAqua, .accessibilityHighContrastDarkAqua]) {
        case .accessibilityHighContrastAqua: NSColor.black.withAlphaComponent(SegmentedMetrics.labelOpacityIncreasedContrast)
        case .accessibilityHighContrastDarkAqua: NSColor.white.withAlphaComponent(SegmentedMetrics.labelOpacityIncreasedContrast)
        default: NSColor.labelColor
        }
    }

    private var titles: [String] = []
    private var segmentLayout = SegmentedLayout(labelWidths: [])

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        wantsLayer = true
        setAccessibilityElement(false)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { nil }

    override var isFlipped: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func show(titles: [String], layout: SegmentedLayout) {
        self.titles = titles
        segmentLayout = layout
        needsDisplay = true
    }

    /// Each title's line fragment at its label box's top-left: the baseline lands 13 pt below it
    /// (17 pt from the control's top), where the system control's SwiftUI labels have theirs
    /// (SwiftUI's 13 pt line is 16 tall with the baseline at 13; measured with upright
    /// `cacheDisplay` renders, ink identical within 0.01 pt).
    override func draw(_ dirtyRect: NSRect) {
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.systemFont(ofSize: SegmentedMetrics.fontSize), .foregroundColor: Self.color,
        ]
        for (index, title) in titles.enumerated() where index < segmentLayout.count {
            (title as NSString).draw(at: segmentLayout.labelFrame(index).origin, withAttributes: attributes)
        }
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        needsDisplay = true
    }
}
