// The course section picker's geometry (spec §3.2.1) without the views: where the track, the
// glass thumb, the labels and the VoiceOver segments of a custom segmented control sit, so it
// reads as the system's 27 tabs control (`NSSegmentedControl`, role `.tabs`, `.fillEqually`).
// Every number was measured on macOS 27.2; `SegmentedMetrics` is the one place for them.

import CoreGraphics
import Foundation

/// The system tabs control as measured on 27.2 (regular size, system 13 pt labels), in points
/// and opacities.
public enum SegmentedMetrics {
    /// The control (and track) height.
    public static let height: CGFloat = 24
    /// The track's corner radius (continuous curve).
    public static let trackRadius: CGFloat = 6
    /// The selected segment's thumb: 2 pt inside the track, 20 pt tall, radius 4 (continuous,
    /// concentric with the track's 6).
    public static let thumbInset: CGFloat = 2
    public static let thumbHeight: CGFloat = 20
    public static let thumbRadius: CGFloat = 4
    /// Reserved before every segment except the first (AppKit's separator slot; nothing is drawn).
    public static let separator: CGFloat = 1
    /// A plain AppKit segment is its text plus 24, rounded up to 0.5 pt …
    public static let labelPadding: CGFloat = 24
    /// … and the 27 tabs style adds 4 per segment.
    public static let tabsExtra: CGFloat = 4
    /// The label boxes: 16 pt tall, 4 pt below the top, centred on their thumb.
    public static let labelTop: CGFloat = 4
    public static let labelHeight: CGFloat = 16
    /// Labels: the system font, Regular, in every state (never bold when selected).
    public static let fontSize: CGFloat = 13
    /// The track: two fills of the ink (black in light, white in dark): 4.7 % (14.9 % with
    /// Increase Contrast) and a 3.0 % sheen above it.
    public static let trackFill: Double = 0.047
    public static let trackFillIncreasedContrast: Double = 0.149
    public static let trackSheen: Double = 0.030
    /// Show Borders: a 1 pt outline of the ink inside the track's edge (plus-darker in light,
    /// plus-lighter in dark); the thumb gets none.
    public static let borderOpacity: Double = 0.125
    /// Labels are `labelColor` (84.7 % ink); with Increase Contrast the ink itself.
    public static let labelOpacityIncreasedContrast: Double = 1.0
}

/// Where each part of an n-segment control sits, from its labels' text widths (measured like
/// AppKit: `NSAttributedString(title, systemFont 13).size().width`). Coordinates from the
/// control's top-left corner.
public struct SegmentedLayout: Equatable, Sendable {
    public let labelWidths: [CGFloat]
    /// Every segment's slot: as wide as the widest segment (AppKit's `.fillEqually`).
    public let slotWidth: CGFloat

    /// AppKit's width rule for `.tabs` (matches `intrinsicContentSize` on 16 label sets): each
    /// segment wants its text + 24 rounded up to 0.5 pt, + 4 for the tabs style, + 1 for the
    /// separator before it (not before the first); every slot takes the largest of these.
    public init(labelWidths: [CGFloat]) {
        self.labelWidths = labelWidths
        slotWidth = labelWidths.enumerated().map { index, width in
            (2 * (width + SegmentedMetrics.labelPadding) - 1e-6).rounded(.up) / 2
                + SegmentedMetrics.tabsExtra + (index > 0 ? SegmentedMetrics.separator : 0)
        }.max() ?? 0
    }

    public var count: Int { labelWidths.count }

    /// The control's size: 273 × 24 for the English course labels, 243 × 24 for the Chinese (364 /
    /// 324 with Explain).
    public var size: CGSize {
        CGSize(width: CGFloat(count) * slotWidth, height: SegmentedMetrics.height)
    }

    /// The glass thumb over segment `index`: its slot less the separator and a 2 pt inset
    /// (English 87 / 86 / 86 wide at x 2 / 94 / 185).
    public func thumbFrame(_ index: Int) -> CGRect {
        let separator = index > 0 ? SegmentedMetrics.separator : 0
        return CGRect(
            x: CGFloat(index) * slotWidth + separator + SegmentedMetrics.thumbInset,
            y: SegmentedMetrics.thumbInset,
            width: slotWidth - 2 * SegmentedMetrics.thumbInset - separator,
            height: SegmentedMetrics.thumbHeight
        )
    }

    /// Segment `index`'s label box: its text width rounded up, centred on the thumb (English
    /// x 14 / 107 / 202.5).
    public func labelFrame(_ index: Int) -> CGRect {
        let width = labelWidths[index].rounded(.up)
        return CGRect(
            x: thumbFrame(index).midX - width / 2, y: SegmentedMetrics.labelTop,
            width: width, height: SegmentedMetrics.labelHeight
        )
    }

    /// The segment under x; outside the control, the nearest one (a release there still counts).
    public func segment(atX x: CGFloat) -> Int {
        guard count > 0, slotWidth > 0 else { return 0 }
        return min(max(Int((x / slotWidth).rounded(.down)), 0), count - 1)
    }

    /// The thumb while the pointer drags: centred on x, as wide as the segment under it, and
    /// never past the first or last thumb.
    public func thumbFrame(followingX x: CGFloat) -> CGRect {
        guard count > 0 else { return .zero }
        var frame = thumbFrame(segment(atX: x))
        let lowest = thumbFrame(0).minX
        let highest = thumbFrame(count - 1).maxX - frame.width
        frame.origin.x = min(max(x - frame.width / 2, lowest), highest)
        return frame
    }

    /// Segment `index`'s VoiceOver frame: its slot, the control's full height, as VoiceOver reads the
    /// system control's segments (English 91 wide at x 0 / 91 / 182, Chinese 81; read out of process
    /// on 27.2).
    public func accessibilityFrame(_ index: Int) -> CGRect {
        CGRect(x: CGFloat(index) * slotWidth, y: 0, width: slotWidth, height: SegmentedMetrics.height)
    }
}
