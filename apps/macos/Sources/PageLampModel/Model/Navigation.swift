// Navigation model (spec §2.4): a two-column split view with no back stack.

import Foundation
import Observation

/// What the detail column shows.
public enum Destination: Hashable, Codable, Sendable {
    case thisWeek
    /// A course by id.
    case course(String)
    case sources
    case connect
}

/// The sections of a course's detail page, in picker order (Explain last: M3, where the AI
/// screens are on).
public enum CourseSection: String, CaseIterable, Codable, Sendable {
    case week
    case deadlines
    case timeline
    case explain

    /// The sections the picker offers: Explain only where it's on.
    public static func shown(explain: Bool) -> [CourseSection] {
        explain ? allCases : allCases.filter { $0 != .explain }
    }

    /// The section to show: Explain falls back to This Week where it's off (like the Tauri app's
    /// unknown tab).
    public func shown(explain: Bool) -> CourseSection {
        self == .explain && !explain ? .week : self
    }
}

/// One course's section and week, kept for the session (a relaunch opens at *now*).
@Observable @MainActor
public final class CourseUIState {
    public var section: CourseSection = .week
    /// Where the section picker's thumb last stood; a new page's thumb starts here and slides to
    /// `section` (a cross-link that opens the course at Deadlines). Only the picker writes it.
    @ObservationIgnored public var pickerSection: CourseSection = .week
    /// The week the student stepped to; nil = *now* (the current week).
    public var selectedWeek: UInt32? {
        didSet {
            // Stepping back onto the current week is "now" again (the lamp lights up).
            if let selectedWeek, selectedWeek == currentWeek { self.selectedWeek = nil }
        }
    }

    /// From the last `weekMaterials` load (`available_weeks`, `timeline.current_week`).
    public private(set) var availableWeeks: [UInt32] = []
    public private(set) var currentWeek: UInt32?

    public init() {}

    /// The week on screen: the selected one, else the current one (nil = unknown).
    public var displayedWeek: UInt32? { selectedWeek ?? currentWeek }

    /// Whether the page shows *now* (the band is lit and "This Week" is hidden).
    public var showsCurrentWeek: Bool { selectedWeek == nil && currentWeek != nil }

    /// Whether the page is away from its default view: a week the student stepped to, away
    /// from the current week or from "Recent materials" when the current week is unknown.
    public var isAwayFromDefault: Bool { selectedWeek != nil }

    /// Remembers what the course's week data says (called after each `weekMaterials` load).
    public func update(availableWeeks: [UInt32], currentWeek: UInt32?) {
        self.availableWeeks = availableWeeks.sorted()
        self.currentWeek = currentWeek
        if let selected = selectedWeek, selected == currentWeek { selectedWeek = nil }
    }

    /// The week before the one on screen. From "Recent materials" (the current week is
    /// unknown) that is the last available week, like the Tauri app.
    public var previousWeek: UInt32? {
        guard let shown = displayedWeek else { return availableWeeks.last }
        return availableWeeks.last { $0 < shown }
    }

    public var nextWeek: UInt32? {
        guard let shown = displayedWeek else { return nil }
        return availableWeeks.first { $0 > shown }
    }

    /// Go ▸ Previous Week (⌘[) / Next Week (⌘]); nothing past the first or last week.
    public func step(by delta: Int) {
        let target = delta < 0 ? previousWeek : nextWeek
        guard let target else { return }
        section = .week
        selectedWeek = target
    }

    /// Go ▸ Current Week (⇧⌘T) and the toolbar's "This Week" (or "Show Recent Materials" when
    /// the current week is unknown).
    public func showCurrentWeek() {
        section = .week
        selectedWeek = nil
    }
}
