// Renders pages of the snapshot catalogue headless with ImageRenderer.
//
// Always (plain `swift test`): a small subset (This Week, a course page, Sources, the sidebar,
// component gallery) in two variants, into a temporary folder that is removed afterwards. In debug
// builds an unknown string key or a missing argument trips an assertion, so this also proves those
// pages' strings exist in both languages.
//
// The whole catalogue (every page, light/dark × en/zh-Hans) only when asked:
//
//     PAGELAMP_SNAPSHOT_DIR=/path/to/dir swift test --filter SnapshotRenderTests
//     PAGELAMP_SNAPSHOT_FILTER=this-week,course-DEMO101 …   (name prefixes, optional)
//
// (or `swift run PageLampSnapshots <dir>`). The model tests wait on events, not on time, so a
// render holding the main actor only slows them down.

import Foundation
import PageLampModel
import PageLampSnapshots
import Testing

@Suite("Snapshot renders", .serialized)
@MainActor
struct SnapshotRenderTests {
    nonisolated static let directory = ProcessInfo.processInfo.environment["PAGELAMP_SNAPSHOT_DIR"]
    nonisolated static let prefixes = ProcessInfo.processInfo.environment["PAGELAMP_SNAPSHOT_FILTER"]?
        .split(separator: ",").map(String.init) ?? []

    /// The pages every `swift test` renders: one per kind of screen.
    static let subset = ["this-week-default", "course-DEMO101-week", "sources-expired", "sidebar", "settings-ai-key", "components"]

    @Test("a few pages render in English light and Chinese dark")
    func renderSubset() async throws {
        let directory = URL(filePath: NSTemporaryDirectory(), directoryHint: .isDirectory)
            .appending(path: "PageLampSnapshotTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        defer { try? FileManager.default.removeItem(at: directory) }
        let variants = SnapshotRenderer.variants.filter { variant in
            (variant.language == .english && !variant.dark) || (variant.language == .simplifiedChinese && variant.dark)
        }
        #expect(variants.count == 2)
        for name in Self.subset {
            #expect(SnapshotCatalog.page(named: name) != nil, "\(name) is not in the catalogue")
        }
        let rendered = try await SnapshotRenderer.renderAll(
            to: directory, names: Self.subset, variants: variants, calendar: TestClock.calendar, now: TestClock.now, scale: 1
        )
        #expect(rendered.count == Self.subset.count * variants.count)
        try Self.check(rendered)
    }

    @Test("every page renders in both languages and appearances", .enabled(if: directory != nil))
    func renderAll() async throws {
        let directory = URL(filePath: try #require(Self.directory), directoryHint: .isDirectory)
        let rendered = try await SnapshotRenderer.renderAll(to: directory, prefixes: Self.prefixes)
        let pages = SnapshotCatalog.pages.filter { page in
            Self.prefixes.isEmpty || Self.prefixes.contains { page.name.hasPrefix($0) }
        }
        #expect(rendered.count == pages.count * SnapshotRenderer.variants.count)
        try Self.check(rendered)
        for image in rendered {
            print(image.file.path(percentEncoded: false))
        }
    }

    private static func check(_ rendered: [SnapshotRenderer.Rendered]) throws {
        for image in rendered {
            let size = try FileManager.default.attributesOfItem(atPath: image.file.path(percentEncoded: false))[.size] as? Int
            #expect((size ?? 0) > 5_000, "\(image.name) looks empty")
            #expect(image.width > 0 && image.height > 0)
        }
    }
}
