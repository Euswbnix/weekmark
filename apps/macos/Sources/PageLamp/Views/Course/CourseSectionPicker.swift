// The course page's section picker (spec §3.2 "Section picker", §3.2.1): This Week · Deadlines ·
// Timeline, then Explain where it's on (M3, preview builds). Wide, a custom segmented control that reads as the system's 27 tabs control, with one
// glass thumb that slides on every change (GlassSegmentedControl, Chrome/) over a SwiftUI track;
// narrow, the system pop-up menu, so the picker never runs past the reading column (English at the
// minimum window with the inspector open: its segments are wider than Chinese ones).

import SwiftUI
import PageLampModel

struct CourseSectionPicker: View {
    let ui: CourseUIState
    @Environment(AppModel.self) private var model
    @Environment(\.l10n) private var l10n
    @Environment(\.drawsControlStandIns) private var standIns
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    /// Preview builds, to compare on device (Debug ▸ Course Pages Use the System Section Picker):
    /// the system control of before the glass thumb. Removed once the user has signed off.
    @AppStorage(DebugPreferences.systemSectionPicker) private var systemPicker = false

    var body: some View {
        ViewThatFits(in: .horizontal) {
            wide
            narrow
        }
        // Where the next page's thumb starts (wide control and menu alike).
        .onChange(of: ui.section, initial: true) { ui.pickerSection = $1 }
    }

    @ViewBuilder private var wide: some View {
        if standIns {
            SegmentedStandIn(titles: sections.map(\.title), selected: index(of: ui.section))
        } else if systemPicker {
            if #available(macOS 27, *) {
                picker.pickerStyle(.tabs).fixedSize()
            } else {
                picker.pickerStyle(.segmented).fixedSize()
            }
        } else {
            GlassSegmentedPicker(
                titles: sections.map(\.title),
                selection: index(of: ui.section),
                start: index(of: ui.pickerSection),
                accessibilityLabel: l10n("course.tabs.label"),
                animates: !reduceMotion
            ) { index in
                ui.section = sections[index].section
            }
            // The control is the tab group VoiceOver reads (GlassSegmentedAccessibility); this label
            // is for SwiftUI's own node of the focused view, announced when focus arrives (the system
            // Picker's label reaches it the same way; without it, an unlabelled "tab group").
            .accessibilityLabel(l10n("course.tabs.label"))
            .background { SegmentedTrack() }
        }
    }

    @ViewBuilder private var narrow: some View {
        if standIns {
            PopUpStandIn(titles: sections.map(\.title), selected: index(of: ui.section))
        } else {
            picker.pickerStyle(.menu).fixedSize()
        }
    }

    private var sections: [(section: CourseSection, title: String)] {
        CourseSection.shown(explain: model.aiExplain).map { section in
            switch section {
            case .week: (section, l10n("mac.course.sections.week"))
            case .deadlines: (section, l10n("course.tabs.deadlines"))
            case .timeline: (section, l10n("course.tabs.timeline"))
            case .explain: (section, l10n("course.tabs.explain"))
            }
        }
    }

    private func index(of section: CourseSection) -> Int {
        sections.firstIndex { $0.section == section } ?? 0
    }

    private var picker: some View {
        @Bindable var ui = ui
        return Picker(l10n("course.tabs.label"), selection: $ui.section) {
            ForEach(sections, id: \.section) { item in
                Text(item.title).tag(item.section)
            }
        }
        .labelsHidden()
    }
}

/// The track under the glass thumb, drawn as the system tabs control draws its own: two fills of
/// the ink (black in light, white in dark), 4.7 % (14.9 % with Increase Contrast) and a 3.0 % sheen,
/// radius 6; Show Borders adds a 1 pt outline (plus-darker in light, plus-lighter in dark).
struct SegmentedTrack: View {
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.colorSchemeContrast) private var contrast
    @Environment(\.accessibilityShowBorders) private var showBorders

    var body: some View {
        let ink: Color = colorScheme == .dark ? .white : .black
        let shape = RoundedRectangle(cornerRadius: SegmentedMetrics.trackRadius, style: .continuous)
        ZStack {
            shape.fill(ink.opacity(contrast == .increased ? SegmentedMetrics.trackFillIncreasedContrast : SegmentedMetrics.trackFill))
            shape.fill(ink.opacity(SegmentedMetrics.trackSheen))
            if showBorders {
                shape.strokeBorder(ink.opacity(SegmentedMetrics.borderOpacity), lineWidth: 1)
                    .blendMode(colorScheme == .dark ? .plusLighter : .plusDarker)
            }
        }
        .accessibilityHidden(true)
    }
}

extension EnvironmentValues {
    /// Snapshots only: ImageRenderer draws AppKit-backed controls (segmented, tab and pop-up
    /// pickers, the glass thumb) as placeholders, so those render as stand-ins of their size, and
    /// a snapshot shows which layout the page chose. The app never sets it.
    @Entry package var drawsControlStandIns = false
}

/// Offscreen stand-in for the glass section picker (snapshots): the real track, geometry and label
/// type (so ViewThatFits makes the app's choice) with a flat thumb (glass doesn't render offscreen).
private struct SegmentedStandIn: View {
    let titles: [String]
    let selected: Int

    var body: some View {
        let layout = SegmentedLayout(labelWidths: GlassSegmentedControl.labelWidths(titles))
        let thumb = layout.thumbFrame(selected)
        let thumbShape = RoundedRectangle(cornerRadius: SegmentedMetrics.thumbRadius, style: .continuous)
        ZStack(alignment: .topLeading) {
            SegmentedTrack()
            thumbShape
                .fill(.fill.quaternary)
                .overlay { thumbShape.strokeBorder(.separator, lineWidth: 1) }
                .frame(width: thumb.width, height: thumb.height)
                .offset(x: thumb.minX, y: thumb.minY)
            ForEach(Array(titles.enumerated()), id: \.offset) { index, title in
                let frame = layout.labelFrame(index)
                Text(title)
                    .font(.system(size: SegmentedMetrics.fontSize))
                    .lineLimit(1)
                    .fixedSize()
                    .frame(width: frame.width, height: frame.height)
                    .offset(x: frame.minX, y: frame.minY)
            }
        }
        .frame(width: layout.size.width, height: layout.size.height, alignment: .topLeading)
        .accessibilityHidden(true)
    }
}

/// Offscreen stand-in for a pop-up (menu) picker (snapshots): as wide as its widest title (like
/// NSPopUpButton), showing the selected one and the up/down chevrons.
private struct PopUpStandIn: View {
    let titles: [String]
    let selected: Int

    var body: some View {
        HStack(spacing: 18) {
            ZStack(alignment: .leading) {
                ForEach(Array(titles.enumerated()), id: \.offset) { index, title in
                    Text(title)
                        .lineLimit(1)
                        .opacity(index == selected ? 1 : 0)
                }
            }
            Image(systemName: "chevron.up.chevron.down")
                .imageScale(.small)
                .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 3)
        .background(.fill.tertiary, in: .rect(cornerRadius: 6))
        .fixedSize()
        .accessibilityHidden(true)
    }
}
