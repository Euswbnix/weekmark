// What's new on the Mac: which topics the sheet shows, loading once per launch after an update,
// acknowledging, and the cases where nothing may be shown or written.

import Foundation
import PageLamp
import PageLampKit
import PageLampModel
import Testing

/// The mock, with `startupTasks` answered by the test.
private struct StartupAnswer: ForwardingService {
    let base: any PageLampService
    let answer: Result<StartupTasks, PageLampFailure>

    func startupTasks(now: Date) async throws(PageLampFailure) -> StartupTasks {
        try answer.get()
    }
}

private func tasks(_ whatsNew: WhatsNew?) -> StartupTasks {
    StartupTasks(
        whatsNew: whatsNew, updateCheckDue: false, updatedFrom: whatsNew?.since,
        syncDue: SyncDue(unattended: false, attended: false)
    )
}

@Suite("What's new") @MainActor
struct WhatsNewTests {
    @Test("the Mac shows its own topics in the facade's order, never the Tauri app's update check")
    func catalog() {
        let all: (String) -> Bool = { _ in true }
        #expect(WhatsNewCatalog.items(for: [.updateCheck, .courseWeeks], has: all).map(\.topic) == [.courseWeeks])
        #expect(WhatsNewCatalog.items(for: [.updateCheck], has: all).isEmpty)
        // A topic whose strings this build lacks is left out.
        let noBody: (String) -> Bool = { !$0.hasSuffix(".body") }
        #expect(WhatsNewCatalog.items(for: [.courseWeeks], has: noBody).isEmpty)
    }

    @Test("every topic the Mac shows has its title and body in the string table")
    func strings() {
        let items = WhatsNewCatalog.items(for: [.updateCheck, .courseWeeks]) { _ in true }
        for item in items {
            #expect(StringTable.app.keys.contains(item.titleKey), "\(item.titleKey)")
            #expect(StringTable.app.keys.contains(item.bodyKey), "\(item.bodyKey)")
        }
    }

    @Test("a fresh install shows nothing and acknowledges nothing")
    func freshInstall() async {
        let (model, mock) = makeModel()
        await model.start()
        #expect(model.whatsNew == nil)
        #expect(await mock.callCount("acknowledgeWhatsNew") == 0)
    }

    @Test("after an update the sheet shows once; closing it counts as read")
    func afterAnUpdate() async throws {
        let (model, mock) = makeModel()
        await mock.simulateUpgrade(from: "0.3.0-alpha.1")
        await model.start()
        let whatsNew = try #require(model.whatsNew)
        #expect(whatsNew.since == "0.3.0-alpha.1")
        #expect(whatsNew.items.map(\.topic) == [.courseWeeks])
        // start() is the launch: calling it again (the root view's task) asks nothing new.
        await model.start()
        #expect(await mock.callCount("startupTasks") == 1)

        await model.acknowledgeWhatsNew()
        #expect(model.whatsNew == nil)
        #expect(await mock.callCount("acknowledgeWhatsNew") == 1)
        #expect(try await mock.startupTasks(now: TestClock.now).whatsNew == nil)
        #expect(await mock.callCount("acknowledgeUpdateDisclosure") == 0, "the Tauri app's disclosure")
    }

    @Test("with nothing it can show, it acknowledges at once instead of holding What's new")
    func nothingToShow() async {
        let offered = WhatsNew(since: "0.3.0-alpha.1", topics: [.updateCheck])
        let (model, mock) = makeModel { StartupAnswer(base: $0, answer: .success(tasks(offered))) }
        await model.start()
        #expect(model.whatsNew == nil)
        #expect(await mock.callCount("acknowledgeWhatsNew") == 1)
    }

    @Test("when the settings can't be read, it shows nothing and writes nothing")
    func unreadable() async {
        let failure = PageLampFailure(kind: .internal, message: "database is locked")
        let (model, mock) = makeModel { StartupAnswer(base: $0, answer: .failure(failure)) }
        await model.start()
        #expect(model.whatsNew == nil)
        #expect(await mock.callCount("acknowledgeWhatsNew") == 0)
    }

    @Test("switching the data source drops a sheet that belonged to the old one")
    func dataSourceSwitch() async {
        let (model, mock) = makeModel()
        await mock.simulateUpgrade(from: nil)
        await model.start()
        #expect(model.whatsNew != nil)
        #expect(model.whatsNew?.since == nil, "an update from 0.1 names no version")
        await model.useMock(.demo)
        #expect(model.whatsNew == nil)
    }
}
