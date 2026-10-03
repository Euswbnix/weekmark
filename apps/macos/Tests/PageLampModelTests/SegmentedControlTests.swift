// The course section picker without its views (spec §3.2.1): geometry that matches the system's
// 27 tabs control (measured on macOS 27.2, and checked against AppKit itself where the OS has the
// tabs style), and the pointer, key and VoiceOver rules measured on that control.

import AppKit
import CoreGraphics
import Foundation
import PageLampModel
import Testing

/// Text widths of the course labels in the system font at 13 pt (AppKit's measure, 27.2).
private let english: [CGFloat] = [62.734, 59.662, 50.965]   // This Week, Deadlines, Timeline
private let chinese: [CGFloat] = [25.798, 51.595, 51.595]   // 本周, 截止日期, 教学进度
private let thisWeek: CGFloat = 62.734, timeline: CGFloat = 50.965, deadlinesZh: CGFloat = 51.595, thisWeekZh: CGFloat = 25.798

@Suite("Section picker geometry")
struct SegmentedLayoutTests {
    @Test("English: slots of 91, 273 × 24; thumbs 87 / 86 / 86 at 2 / 94 / 185; labels centred on them; VoiceOver frames are the slots")
    func englishLabels() {
        let layout = SegmentedLayout(labelWidths: english)
        #expect(layout.slotWidth == 91)
        #expect(layout.size == CGSize(width: 273, height: 24))
        #expect((0..<3).map(layout.thumbFrame) == [
            CGRect(x: 2, y: 2, width: 87, height: 20), CGRect(x: 94, y: 2, width: 86, height: 20), CGRect(x: 185, y: 2, width: 86, height: 20),
        ])
        #expect((0..<3).map(layout.labelFrame) == [
            CGRect(x: 14, y: 4, width: 63, height: 16), CGRect(x: 107, y: 4, width: 60, height: 16), CGRect(x: 202.5, y: 4, width: 51, height: 16),
        ])
        #expect((0..<3).map(layout.accessibilityFrame) == [
            CGRect(x: 0, y: 0, width: 91, height: 24), CGRect(x: 91, y: 0, width: 91, height: 24), CGRect(x: 182, y: 0, width: 91, height: 24),
        ])
    }

    @Test("Chinese: slots of 81, 243 × 24; thumbs 77 / 76 / 76 at 2 / 84 / 165")
    func chineseLabels() {
        let layout = SegmentedLayout(labelWidths: chinese)
        #expect(layout.slotWidth == 81)
        #expect(layout.size == CGSize(width: 243, height: 24))
        #expect((0..<3).map(layout.thumbFrame) == [
            CGRect(x: 2, y: 2, width: 77, height: 20), CGRect(x: 84, y: 2, width: 76, height: 20), CGRect(x: 165, y: 2, width: 76, height: 20),
        ])
        #expect((0..<3).map { layout.labelFrame($0).minX } == [27.5, 96, 177])
        #expect((0..<3).map(layout.accessibilityFrame) == [
            CGRect(x: 0, y: 0, width: 81, height: 24), CGRect(x: 81, y: 0, width: 81, height: 24), CGRect(x: 162, y: 0, width: 81, height: 24),
        ])
    }

    @Test("the separator slot counts before every segment but the first", arguments: [
        ([deadlinesZh, thisWeekZh, thisWeekZh], 80),   // the widest label first: no separator
        ([thisWeek, thisWeek, timeline], 92),          // the widest label second: + 1
        ([timeline, timeline, thisWeek], 92),          // the widest label last: + 1
    ] as [([CGFloat], CGFloat)])
    func separator(widths: [CGFloat], slot: CGFloat) {
        let layout = SegmentedLayout(labelWidths: widths)
        #expect(layout.slotWidth == slot)
        #expect(layout.size.width == 3 * slot)
    }

    @Test("x → segment: floor(x ÷ slot), clamped (a release outside still hits the nearest)")
    func hitTesting() {
        let layout = SegmentedLayout(labelWidths: english)
        #expect([-10, 0, 90.9, 91, 181.9, 182, 300].map(layout.segment(atX:)) == [0, 0, 0, 1, 1, 2, 2])
    }

    @Test("the dragged thumb is centred on the pointer, as wide as the segment under it, never past the ends")
    func following() {
        let layout = SegmentedLayout(labelWidths: english)
        #expect(layout.thumbFrame(followingX: 136.5) == CGRect(x: 93.5, y: 2, width: 86, height: 20))
        #expect(layout.thumbFrame(followingX: 0) == CGRect(x: 2, y: 2, width: 87, height: 20))
        #expect(layout.thumbFrame(followingX: -80) == CGRect(x: 2, y: 2, width: 87, height: 20))
        #expect(layout.thumbFrame(followingX: 273) == CGRect(x: 185, y: 2, width: 86, height: 20))
        #expect(layout.thumbFrame(followingX: 400) == CGRect(x: 185, y: 2, width: 86, height: 20))
    }

    @Test("Explain (M3) as a fourth segment keeps the slot: 364 × 24 in English, 324 × 24 in Chinese")
    @MainActor
    func fourSegments() {
        let font = NSFont.systemFont(ofSize: SegmentedMetrics.fontSize)
        func widths(_ labels: [String]) -> [CGFloat] {
            labels.map { NSAttributedString(string: $0, attributes: [.font: font]).size().width }
        }
        let english = SegmentedLayout(labelWidths: widths(["This Week", "Deadlines", "Timeline", "Explain"]))
        #expect(english.size.width == 364 && english.size.height == 24)
        #expect(english.thumbFrame(3) == CGRect(x: 276, y: 2, width: 86, height: 20))
        #expect(english.accessibilityFrame(3) == CGRect(x: 273, y: 0, width: 91, height: 24))
        let chinese = SegmentedLayout(labelWidths: widths(["本周", "截止日期", "教学进度", "讲解"]))
        #expect(chinese.size.width == 324 && chinese.size.height == 24)
        #expect(chinese.thumbFrame(3) == CGRect(x: 246, y: 2, width: 76, height: 20))
    }

    @Test("the width rule is AppKit's: NSSegmentedControl, .tabs, .fillEqually (macOS 27)", arguments: [
        ["This Week", "Deadlines", "Timeline"], ["本周", "截止日期", "教学进度"],
        ["截止日期", "本周", "本周"], ["This Week", "This Week", "Timeline"], ["Timeline", "Timeline", "This Week"],
        ["This Week", "Deadlines", "Timeline", "Explain"], ["本周", "截止日期", "教学进度", "讲解"],
    ])
    @MainActor
    func matchesAppKit(labels: [String]) {
        guard #available(macOS 27, *) else { return }
        let font = NSFont.systemFont(ofSize: SegmentedMetrics.fontSize)
        let widths = labels.map { NSAttributedString(string: $0, attributes: [.font: font]).size().width }
        let control = NSSegmentedControl(labels: labels, trackingMode: .selectOne, target: nil, action: nil)
        control.segmentDistribution = .fillEqually
        control.role = .tabs
        #expect(SegmentedLayout(labelWidths: widths).size.width == control.intrinsicContentSize.width)
        #expect(control.intrinsicContentSize.height == SegmentedMetrics.height)
    }
}

@Suite("Section picker keyboard, pointer and VoiceOver")
struct SegmentedNavigationTests {
    private let layout = SegmentedLayout(labelWidths: english)
    /// x at the middle of each segment.
    private let middle: [CGFloat] = [45.5, 137, 228]

    @Test("← / → move the key segment and wrap both ways; the selection stays")
    func arrows() {
        var nav = SegmentedNavigation(count: 3, selection: 0)
        #expect(nav.keySegment == 0)
        #expect(nav.handle(.right, in: layout, selection: 0) == .keySegment(1))
        #expect(nav.handle(.right, in: layout, selection: 0) == .keySegment(2))
        #expect(nav.handle(.right, in: layout, selection: 0) == .keySegment(0))
        #expect(nav.handle(.left, in: layout, selection: 0) == .keySegment(2))
        #expect(nav.handle(.left, in: layout, selection: 0) == .keySegment(1))
    }

    @Test("Space selects the key segment; on the selection it does nothing; ↓ is taken; other keys go on")
    func space() {
        var nav = SegmentedNavigation(count: 3, selection: 0)
        #expect(nav.handle(.space, in: layout, selection: 0) == .handled)
        _ = nav.handle(.right, in: layout, selection: 0)
        #expect(nav.handle(.space, in: layout, selection: 0) == .commit(1))
        #expect(nav.handle(.down, in: layout, selection: 1) == .handled)
        #expect(nav.handle(.otherKey, in: layout, selection: 1) == .ignored)
        #expect(nav.keySegment == 1)
    }

    @Test("a press moves the thumb and selects nothing; the release commits the segment under the pointer")
    func press() {
        var nav = SegmentedNavigation(count: 3, selection: 0)
        #expect(nav.handle(.pointerDown(x: middle[2]), in: layout, selection: 0) == .thumb(.segment(2)))
        #expect(nav.pressedSegment == 2)
        #expect(nav.handle(.pointerUp(x: middle[2]), in: layout, selection: 0) == .commit(2))
        #expect(nav.pressedSegment == nil)
        #expect(nav.keySegment == 2)
    }

    @Test("press, drag, release: the thumb follows the pointer and the segment under it is committed")
    func drag() {
        var nav = SegmentedNavigation(count: 3, selection: 0)
        _ = nav.handle(.pointerDown(x: middle[0]), in: layout, selection: 0)
        #expect(nav.handle(.pointerDragged(x: 120), in: layout, selection: 0) == .thumb(.following(x: 120)))
        #expect(nav.pressedSegment == 1)
        #expect(nav.handle(.pointerDragged(x: 200), in: layout, selection: 0) == .thumb(.following(x: 200)))
        #expect(nav.handle(.pointerUp(x: 200), in: layout, selection: 0) == .commit(2))
    }

    @Test("a release 80 pt past the control still commits the last segment (no cancel)")
    func releaseOutside() {
        var nav = SegmentedNavigation(count: 3, selection: 0)
        _ = nav.handle(.pointerDown(x: middle[0]), in: layout, selection: 0)
        _ = nav.handle(.pointerDragged(x: 273 + 80), in: layout, selection: 0)
        #expect(nav.handle(.pointerUp(x: 273 + 80), in: layout, selection: 0) == .commit(2))
    }

    @Test("a release on the selected segment only brings the thumb back")
    func releaseOnSelection() {
        var nav = SegmentedNavigation(count: 3, selection: 1)
        _ = nav.handle(.pointerDown(x: middle[2]), in: layout, selection: 1)
        _ = nav.handle(.pointerDragged(x: middle[1]), in: layout, selection: 1)
        #expect(nav.handle(.pointerUp(x: middle[1]), in: layout, selection: 1) == .thumb(.segment(1)))
    }

    @Test("a cancelled press puts the thumb back; a stray release afterwards does nothing")
    func cancel() {
        var nav = SegmentedNavigation(count: 3, selection: 0)
        _ = nav.handle(.pointerDown(x: middle[2]), in: layout, selection: 0)
        #expect(nav.handle(.pointerCancelled, in: layout, selection: 0) == .thumb(.segment(0)))
        #expect(nav.pressedSegment == nil)
        #expect(nav.handle(.pointerUp(x: middle[2]), in: layout, selection: 0) == .handled)
        #expect(nav.handle(.pointerCancelled, in: layout, selection: 0) == .handled)
    }

    @Test("a selection change during a press waits for the release (the press wins)")
    func changeDuringPress() {
        var nav = SegmentedNavigation(count: 3, selection: 0)
        _ = nav.handle(.pointerDown(x: middle[1]), in: layout, selection: 0)
        #expect(nav.handle(.selectionChanged(2), in: layout, selection: 2) == .handled)
        #expect(nav.handle(.pointerUp(x: middle[1]), in: layout, selection: 2) == .commit(1))
        #expect(nav.handle(.selectionChanged(0), in: layout, selection: 0) == .thumb(.segment(0)))
    }

    @Test("VoiceOver's press selects and moves the key segment; on the selection it only moves the focus; out of range, nothing")
    func accessibilityPress() {
        var nav = SegmentedNavigation(count: 3, selection: 0)
        #expect(nav.handle(.accessibilityPress(2), in: layout, selection: 0) == .commit(2))
        #expect(nav.keySegment == 2)
        // Already the key segment and the selection: nothing changes (the view re-announces it).
        #expect(nav.handle(.accessibilityPress(2), in: layout, selection: 2) == .handled)
        #expect(nav.handle(.accessibilityPress(3), in: layout, selection: 2) == .handled)
        // The selected segment while the focus ring is elsewhere: the focus moves to it, no commit.
        #expect(nav.handle(.left, in: layout, selection: 2) == .keySegment(1))
        #expect(nav.handle(.accessibilityPress(2), in: layout, selection: 2) == .keySegment(2))
        #expect(nav.keySegment == 2)
    }

    @Test("VoiceOver's focus on a segment makes it the key segment")
    func accessibilityFocus() {
        var nav = SegmentedNavigation(count: 3, selection: 0)
        #expect(nav.handle(.accessibilityFocus(1), in: layout, selection: 0) == .keySegment(1))
        #expect(nav.keySegment == 1)
        #expect(nav.handle(.accessibilityFocus(-1), in: layout, selection: 0) == .handled)
    }

    @Test("a selection change from outside moves the thumb and the key segment")
    func selectionChanged() {
        var nav = SegmentedNavigation(count: 3, selection: 0)
        _ = nav.handle(.right, in: layout, selection: 0)
        #expect(nav.handle(.selectionChanged(2), in: layout, selection: 2) == .thumb(.segment(2)))
        #expect(nav.keySegment == 2)
    }
}
