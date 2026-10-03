// The section picker's accessibility (spec §3.2.1 "VoiceOver"): the tree the system tabs control
// reports on 27.2, rebuilt by hand. The control itself is the tab group ("Course sections"), its
// value the selected segment, no actions; each segment is an `AXRadioButton` / `AXTabButton`
// ("tab") whose description is its title and whose value is 1 or 0, with a settable focus (the
// key segment while the control is first responder) and one action, press. VoiceOver counts the
// children for "1 of 3" ("1 of 4" with Explain). Notifications as the system control posts them (GlassSegmentedControl):
// focused-element changes only, never a value change, no custom announcements.

import AppKit
import PageLampModel

extension GlassSegmentedControl {
    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .tabGroup }

    /// "Course sections", as VoiceOver reads the system control's group.
    override func accessibilityLabel() -> String? { groupLabel.isEmpty ? nil : groupLabel }

    override func accessibilityValue() -> Any? {
        elements.indices.contains(selection) ? elements[selection] : nil
    }

    override func accessibilityChildren() -> [Any]? { elements }

    /// Focus is on a segment, never on the group.
    override func isAccessibilityFocused() -> Bool { false }

    /// The key segment while the control is first responder.
    nonisolated override var accessibilityFocusedUIElement: Any? {
        MainActor.assumeIsolated { () -> GlassSegmentElement? in
            isFirstResponder && elements.indices.contains(keySegment) ? elements[keySegment] : nil
        } ?? super.accessibilityFocusedUIElement
    }

    /// The segment under the point (screen coordinates).
    nonisolated override func accessibilityHitTest(_ point: NSPoint) -> Any? {
        MainActor.assumeIsolated { () -> GlassSegmentElement? in
            guard let window else { return nil }
            let local = convert(window.convertPoint(fromScreen: point), from: nil)
            return elements.indices.first { segmentLayout.accessibilityFrame($0).contains(local) }.map { elements[$0] }
        } ?? self
    }

    /// No actions on the group (NSControl would offer press and friends), and nothing settable:
    /// VoiceOver focuses and presses the segments, as on the system control.
    override func isAccessibilitySelectorAllowed(_ selector: Selector) -> Bool {
        let name = NSStringFromSelector(selector)
        if name.hasPrefix("accessibilityPerform") || ["setAccessibilityValue:", "setAccessibilityFocused:"].contains(name) { return false }
        return super.isAccessibilitySelectorAllowed(selector)
    }

    /// The same for AppKit's legacy query, which VoiceOver's view of the page (SwiftUI's node for
    /// this view) reads and which doesn't consult `isAccessibilitySelectorAllowed`.
    nonisolated override func accessibilityIsAttributeSettable(_ attribute: NSAccessibility.Attribute) -> Bool { false }
}

/// One segment: a tab button of the picker's tab group. Accessibility calls arrive on the main
/// thread (the only place it is used, hence unchecked); the state is the control's (read through
/// `MainActor.assumeIsolated`).
nonisolated final class GlassSegmentElement: NSAccessibilityElement, @unchecked Sendable {
    private let index: Int
    private weak var control: GlassSegmentedControl?

    init(control: GlassSegmentedControl, index: Int) {
        self.index = index
        self.control = control
        super.init()
    }

    /// Reads the control on the main actor; nil once it is gone.
    private func read<T: Sendable>(_ body: @MainActor (GlassSegmentedControl, Int) -> T) -> T? {
        let (control, index) = (self.control, self.index)
        return MainActor.assumeIsolated { control.map { body($0, index) } }
    }

    override func accessibilityRole() -> NSAccessibility.Role? { .radioButton }
    override func accessibilitySubrole() -> NSAccessibility.Subrole? { .tabButtonSubrole }

    override func accessibilityRoleDescription() -> String? {
        NSAccessibility.Role.radioButton.description(with: .tabButtonSubrole)
    }

    /// The title is the description (AXDescription), not an AXTitle, as on the system control.
    override func accessibilityLabel() -> String? {
        read { control, index in index < control.titles.count ? control.titles[index] : nil } ?? nil
    }

    /// 1 on the selected segment, else 0 (no AXSelected).
    override func accessibilityValue() -> Any? {
        NSNumber(value: read { control, index in control.selection == index ? 1 : 0 } ?? 0)
    }

    override func isAccessibilityEnabled() -> Bool {
        read { control, _ in control.isEnabled } ?? false
    }

    override func isAccessibilityFocused() -> Bool {
        read { control, index in control.isFirstResponder && control.keySegment == index } ?? false
    }

    override func setAccessibilityFocused(_ focused: Bool) {
        guard focused else { return }
        _ = read { control, index in control.accessibilityFocus(segment: index) }
    }

    override func accessibilityParent() -> Any? { control }

    override func accessibilityFrame() -> NSRect {
        read { control, index in control.accessibilityScreenFrame(segment: index) } ?? .zero
    }

    override func accessibilityPerformPress() -> Bool {
        read { control, index in control.accessibilityPress(segment: index) } ?? false
    }

    /// Press is the one action; focus is the one settable attribute (not while the control is
    /// disabled).
    override func isAccessibilitySelectorAllowed(_ selector: Selector) -> Bool {
        switch NSStringFromSelector(selector) {
        case "accessibilityPerformPress": return true
        case "setAccessibilityFocused:": return isAccessibilityEnabled()
        case let name where name.hasPrefix("accessibilityPerform") || name.hasPrefix("setAccessibility"): return false
        default: return super.isAccessibilitySelectorAllowed(selector)
        }
    }

    /// The same for AppKit's legacy query (what VoiceOver reads).
    override func accessibilityIsAttributeSettable(_ attribute: NSAccessibility.Attribute) -> Bool {
        attribute == .focused && isAccessibilityEnabled()
    }
}
