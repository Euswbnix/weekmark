// End-to-end tests of the Rust facade through the generated UniFFI bindings.
//
// Safe to run anywhere: every test uses its own temp data dir, folder sources and in-memory
// secrets (`openWithMemorySecrets`), so nothing touches the keychain, the real data dir or the
// network; HOME is redirected so the AI-app config checks read a fake home (TestEnvironment).

import Dispatch
import Foundation
import PageLampKit
import Synchronization
import Testing

// MARK: - Environment and fixtures

/// Process-wide setup, done once before any test calls into Rust (every test starts with
/// `Sandbox()`, which reads `TestEnvironment.root`):
/// - HOME / CODEX_HOME point into a temp dir: `doctor` and `diagnosticReport` look for PageLamp
///   entries in AI-app configs under HOME, and `mcpLaunch` compares the data dir with the
///   platform one under HOME; neither may see the real user's files.
/// - PAGELAMP_HOME is cleared, and diagnostics log into the temp dir.
enum TestEnvironment {
    static let root: URL = {
        let root = URL(filePath: NSTemporaryDirectory(), directoryHint: .isDirectory)
            .appending(path: "PageLampKitTests-\(UUID().uuidString)", directoryHint: .isDirectory)
        let home = root.appending(path: "home", directoryHint: .isDirectory)
        do {
            try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true)
            // A Claude Code config with a PageLamp server entry, so `doctor` has one to find.
            try Data(#"{"mcpServers":{"pagelamp":{"command":"pagelamp","args":["mcp"]}}}"#.utf8)
                .write(to: home.appending(path: ".claude.json"))
        } catch {
            fatalError("could not create the test home: \(error)")
        }
        setenv("HOME", home.path(percentEncoded: false), 1)
        setenv("CODEX_HOME", home.appending(path: ".codex").path(percentEncoded: false), 1)
        unsetenv("PAGELAMP_HOME")
        do {
            try initDiagnostics(
                verbose: false,
                dataDir: root.appending(path: "diagnostics").path(percentEncoded: false)
            )
        } catch {
            fatalError("initDiagnostics failed: \(error)")
        }
        return root
    }()
}

/// A per-test temp directory with a data dir and a course folder.
struct Sandbox {
    let dir: URL

    init() throws {
        dir = TestEnvironment.root.appending(path: UUID().uuidString, directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    var dataDir: String { dir.appending(path: "data").path(percentEncoded: false) }
    var courses: String { dir.appending(path: "Courses").path(percentEncoded: false) }

    func remove() {
        try? FileManager.default.removeItem(at: dir)
    }

    func write(_ relativePath: String, _ text: String) throws {
        let file = dir.appending(path: relativePath)
        try FileManager.default.createDirectory(
            at: file.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try Data(text.utf8).write(to: file)
    }

    /// `Courses/` with two courses, week folders and `course.toml`s whose term started on last
    /// week's Monday (so the current week is 2: weeks run Monday to Sunday). Returns the term
    /// start, "YYYY-MM-DD".
    @discardableResult
    func makeCourseFolder() throws -> String {
        let calendar = Calendar.current
        let today = Date()
        // `.weekday` is 1 for Sunday, 2 for Monday, and so on.
        let daysSinceMonday = (calendar.component(.weekday, from: today) + 5) % 7
        let start = try #require(calendar.date(byAdding: .day, value: -(daysSinceMonday + 7), to: today))
        let parts = calendar.dateComponents([.year, .month, .day], from: start)
        let termStart = String(
            format: "%04ld-%02ld-%02ld", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0
        )
        try write(
            "Courses/DEMO101 Intro to Demo Studies/course.toml",
            "code = \"DEMO101\"\nname = \"Intro to Demo Studies\"\nterm_start = \(termStart)\n"
        )
        try write(
            "Courses/DEMO101 Intro to Demo Studies/Week 1/lecture-01.md",
            "# Lecture 1\n\nPhotosynthesis turns light into chemical energy.\n"
        )
        try write(
            "Courses/DEMO101 Intro to Demo Studies/Week 2/lecture-02.md",
            "# Lecture 2\n\nThe Calvin cycle fixes carbon dioxide.\n"
        )
        try write(
            "Courses/DEMO202 Advanced Demo Studies/course.toml",
            "code = \"DEMO202\"\nterm_start = \(termStart)\n"
        )
        try write(
            "Courses/DEMO202 Advanced Demo Studies/Week 1/notes.txt",
            "Eigenvalues of a symmetric matrix are real.\n"
        )
        return termStart
    }
}

/// The case of a `PageLampError`, without its message.
enum ErrorCase: Equatable {
    case auth, network, invalid, notFound, ambiguous, busy, schema, blocked, model, cancelled, `internal`, panic

    init(_ error: PageLampError) {
        switch error {
        case .Auth: self = .auth
        case .Network: self = .network
        case .Invalid: self = .invalid
        case .NotFound: self = .notFound
        case .Ambiguous: self = .ambiguous
        case .Busy: self = .busy
        case .Schema: self = .schema
        case .Blocked: self = .blocked
        case .Model: self = .model
        case .Cancelled: self = .cancelled
        case .Internal: self = .internal
        case .Panic: self = .panic
        }
    }
}

func expectError<T>(
    _ expected: ErrorCase,
    sourceLocation: SourceLocation = #_sourceLocation,
    _ body: () async throws -> T
) async {
    do {
        _ = try await body()
        Issue.record("expected PageLampError \(expected), but the call succeeded", sourceLocation: sourceLocation)
    } catch let error as PageLampError {
        #expect(ErrorCase(error) == expected, "\(error)", sourceLocation: sourceLocation)
    } catch {
        Issue.record("expected PageLampError \(expected), got \(error)", sourceLocation: sourceLocation)
    }
}

/// The name of the calling thread (Rust names its workers `pagelamp-rt`).
func currentThreadName() -> String {
    var buffer = [CChar](repeating: 0, count: 64)
    pthread_getname_np(pthread_self(), &buffer, buffer.count)
    return buffer.withUnsafeBufferPointer { String(cString: $0.baseAddress!) }
}

/// Records the thread of every callback and forwards the events to a stream.
final class RecordingObserver: SyncObserver {
    let stream = SyncEventStream()
    let threads = Mutex<[String]>([])

    func onEvent(event: SyncEvent) {
        let name = currentThreadName()
        threads.withLock { $0.append(name) }
        stream.onEvent(event: event)
    }
}

/// Holds a sync inside its first callback until released (the sync lock stays taken).
final class GateObserver: SyncObserver {
    let started = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    private let fired = Mutex(false)

    func onEvent(event: SyncEvent) {
        let first = fired.withLock { fired in
            defer { fired = true }
            return !fired
        }
        if first {
            started.signal()
            release.wait()
        }
    }

    /// Waits (off the cooperative pool) until the first callback runs.
    func waitUntilStarted() async -> Bool {
        await withCheckedContinuation { continuation in
            DispatchQueue.global().async { [started] in
                continuation.resume(returning: started.wait(timeout: .now() + 30) == .success)
            }
        }
    }
}

// MARK: - Tests

@Suite("PageLampKit: the Rust facade over UniFFI")
struct PageLampKitTests {
    @Test("A folder source syncs, and every read view, setting and diagnostic works")
    func folderSourceEndToEnd() async throws {
        let sandbox = try Sandbox()
        defer { sandbox.remove() }
        let termStart = try sandbox.makeCourseFolder()

        let lamp = try await PageLamp.openWithMemorySecrets(dataDir: sandbox.dataDir)
        #expect(try await lamp.dataDir() == sandbox.dataDir)
        #expect(try await lamp.dbPath().hasSuffix("/pagelamp.db"))

        // Sources
        let source = try await lamp.addFolderSource(path: sandbox.courses, termStart: nil, label: nil)
        #expect(source.kind == .folder)
        #expect(source.label == "Courses")
        #expect(source.lastSyncedAt == nil)
        let config = try JSONSerialization.jsonObject(with: Data(source.config.utf8)) as? [String: Any]
        #expect((config?["path"] as? String)?.hasSuffix("/Courses") == true)
        #expect(try await lamp.listSources() == [source])

        // Sync with progress
        let progress = SyncEventStream()
        let summary = try await lamp.syncAll(request: SyncRequest(), observer: progress)
        progress.finish()
        var events: [SyncEvent] = []
        for await event in progress.events {
            events.append(event)
        }
        #expect(summary.ok)
        #expect(summary.finishedAt >= summary.startedAt)
        let result = try #require(summary.results.first)
        #expect(summary.results.count == 1)
        #expect(result.sourceId == source.id)
        #expect(result.courses == 2)
        #expect(result.filesIndexed == 3)
        guard case .sourceStarted(let startedId, let label)? = events.first else {
            Issue.record("first event is not SourceStarted: \(events)")
            return
        }
        #expect(startedId == source.id)
        #expect(label == "Courses")
        guard case .sourceFinished(let finishedId, let ok, let error, let errorKind)? = events.last else {
            Issue.record("last event is not SourceFinished: \(events)")
            return
        }
        #expect(finishedId == source.id)
        #expect(ok)
        #expect(error == nil)
        #expect(errorKind == nil)
        #expect(events.contains { if case .progress = $0 { true } else { false } })

        // Read views
        let courses = try await lamp.listCourses()
        #expect(courses.map(\.course.code) == ["DEMO101", "DEMO202"])
        let demo101 = try #require(courses.first)
        #expect(demo101.course.name == "Intro to Demo Studies")
        #expect(demo101.course.termStart == termStart)
        #expect(demo101.course.termSource == .synced)
        #expect(demo101.timeline.currentWeek == 2)
        #expect(demo101.aiMaterials == .readable)
        #expect(demo101.counts.materials == 2)
        #expect(demo101.sourceLabel == "Courses")

        let overview = try await lamp.courseOverview(course: "DEMO101")
        #expect(overview.course.id == demo101.course.id)
        #expect(overview.timeline.currentWeek == 2)

        let week1 = try await lamp.weekMaterials(course: "DEMO101", week: 1)
        #expect(week1.week == 1)
        #expect(week1.requestedWeek == 1)
        #expect(week1.materials.count == 1)
        #expect(week1.materials.first?.textStatus == .ok)
        #expect(week1.availableWeeks == [1, 2])
        let current = try await lamp.weekMaterials(course: "DEMO101", week: nil)
        #expect(current.week == 2)
        #expect(current.materials.count == 1)

        #expect(try await lamp.listDeadlines(course: nil, daysAhead: 7, daysBack: 0).isEmpty)
        #expect(try await lamp.listDeadlines(course: "DEMO101", daysAhead: 14, daysBack: 7).isEmpty)
        let hits = try await lamp.search(query: "photosynthesis", course: nil, limit: 50)
        #expect(hits.count == 1)
        #expect(hits.first?.courseCode == "DEMO101")
        #expect(hits.first?.weekHint == 1)
        #expect(try await lamp.search(query: "eigenvalues", course: "DEMO101", limit: 50).isEmpty)
        #expect(try await lamp.latestStudyPlan() == nil)

        // Course settings
        try await lamp.setCoursePolicy(course: "DEMO202", policy: .prohibited, note: "No AI tools.")
        try await lamp.setCourseAiAccess(course: "DEMO101", allowed: false)
        try await lamp.setCourseHidden(course: "DEMO101", hidden: true)
        try await lamp.setCourseTerm(course: "DEMO202", start: "2026-01-05", end: "2026-04-10")
        let updated = try await lamp.listCourses()
        let hidden = try #require(updated.first { $0.course.code == "DEMO101" })
        let prohibited = try #require(updated.first { $0.course.code == "DEMO202" })
        #expect(hidden.course.hidden)
        #expect(!hidden.course.aiAccess)
        #expect(hidden.aiMaterials == .turnedOff)
        #expect(prohibited.course.aiPolicy == .prohibited)
        #expect(prohibited.course.aiPolicyNote == "No AI tools.")
        #expect(prohibited.aiMaterials == .withheldByPolicy)
        #expect(prohibited.course.termStart == "2026-01-05")
        #expect(prohibited.course.termEnd == "2026-04-10")
        #expect(prohibited.course.termSource == .user)
        // The student's own search skips hidden courses.
        #expect(try await lamp.search(query: "photosynthesis", course: nil, limit: 50).isEmpty)

        // Status
        let status = try await lamp.status()
        #expect(status.version == version())
        #expect(status.dataDir == sandbox.dataDir)
        #expect(status.counts.courses == 1)
        #expect(status.counts.hiddenCourses == 1)
        #expect(status.counts.indexedMaterials == 3)
        #expect(!status.syncInProgress)
        #expect(status.lastSyncedAt != nil)
        #expect(status.sources.map(\.id) == [source.id])

        // "Connect your AI app" with a binary path that does not exist
        let binary = sandbox.dir.appending(path: "PageLamp.app/Contents/MacOS/pagelamp")
            .path(percentEncoded: false)
        let launch = try await lamp.mcpLaunch(pagelampBinary: binary)
        #expect(launch.command == binary)
        #expect(launch.args == ["mcp"])
        #expect(launch.env == ["PAGELAMP_HOME": sandbox.dataDir])
        #expect(launch.temporaryLocation == nil)
        let configs = try await lamp.mcpClientConfigs(pagelampBinary: binary)
        #expect(configs.map(\.client) == [.claudeDesktop, .claudeCode, .codex, .generic])
        for config in configs {
            #expect(config.launch == launch)
            #expect(config.notes.count == config.noteCodes.count)
            #expect(config.noteCodes.contains(.customDataDir))
            #expect(!config.content.isEmpty)
        }

        // Diagnostics: memory secrets, so `doctor` probes no keychain; AI-app configs are read
        // from the fake HOME (which has a Claude Code entry and nothing else).
        let doctor = try await lamp.doctor()
        #expect(doctor.version == version())
        #expect(doctor.keychainAvailable)
        #expect(doctor.keychainError == nil)
        #expect(doctor.databaseError == nil)
        #expect(doctor.courses == 1)
        #expect(doctor.hiddenCourses == 1)
        #expect(doctor.sources.map(\.kind) == [.folder])
        #expect(doctor.mcpClients == McpClientPresence(claudeDesktop: false, claudeCode: true, codex: false))
        let report = try await lamp.diagnosticReport()
        #expect(report.hasPrefix("# PageLamp diagnostic report"))
        #expect(!report.contains("DEMO101"), "course codes are pseudonymised")
        #expect(!report.contains("Intro to Demo Studies"), "course names are pseudonymised")
        let logs = try await lamp.logsDir()
        #expect(FileManager.default.fileExists(atPath: logs))
        #expect(try await lamp.lastCrash() == nil)
        try await lamp.clearLastCrash()

        // Removing a folder source removes its courses, never the folder.
        try await lamp.removeSource(sourceId: source.id)
        #expect(try await lamp.listSources().isEmpty)
        #expect(try await lamp.listCourses().isEmpty)
        #expect(FileManager.default.fileExists(atPath: sandbox.courses))
    }

    @Test("Facade errors arrive as the matching PageLampError case")
    func errorsMapToCases() async throws {
        let sandbox = try Sandbox()
        defer { sandbox.remove() }
        try sandbox.makeCourseFolder()
        let lamp = try await PageLamp.openWithMemorySecrets(dataDir: sandbox.dataDir)
        let source = try await lamp.addFolderSource(path: sandbox.courses, termStart: "2026-09-08", label: "Demo")
        #expect(source.label == "Demo")
        _ = try await lamp.syncAll(request: SyncRequest(), observer: SyncEventStream())

        await expectError(.notFound) { try await lamp.courseOverview(course: "NOPE999") }
        await expectError(.notFound) { try await lamp.weekMaterials(course: "NOPE999", week: nil) }
        await expectError(.notFound) { try await lamp.setCourseHidden(course: "NOPE999", hidden: true) }
        await expectError(.notFound) { try await lamp.removeSource(sourceId: "folder:000000000000") }
        await expectError(.notFound) {
            try await lamp.syncSource(sourceId: "folder:000000000000", request: SyncRequest(), observer: SyncEventStream())
        }
        // "DEMO" is a code prefix of both courses.
        await expectError(.ambiguous) {
            try await lamp.setCoursePolicy(course: "DEMO", policy: .learningAid, note: nil)
        }
        await expectError(.invalid) {
            try await lamp.addFolderSource(path: sandbox.dir.appending(path: "missing").path(percentEncoded: false), termStart: nil, label: nil)
        }
        // A malformed date is rejected while crossing the boundary, as `invalid`.
        await expectError(.invalid) {
            try await lamp.setCourseTerm(course: "DEMO101", start: "2026-13-45", end: nil)
        }
        await expectError(.invalid) {
            try await lamp.addFolderSource(path: sandbox.courses, termStart: "8 Sep 2026", label: nil)
        }
        // Rejected before any network access.
        await expectError(.invalid) {
            try await lamp.addCanvasSource(baseUrl: "https://canvas.example.edu", token: "  ")
        }
        await expectError(.invalid) { try await lamp.addIcalSource(feedUrl: "", label: nil) }
        await expectError(.invalid) {
            try await lamp.downloadCourseFiles(course: "DEMO101", observer: SyncEventStream())
        }
        await expectError(.invalid) { try await lamp.updateSourceSecret(sourceId: source.id, secret: "x") }

        // A data dir that is a file cannot be opened.
        try sandbox.write("not-a-folder", "x")
        await expectError(.internal) {
            try await PageLamp.openWithMemorySecrets(dataDir: sandbox.dir.appending(path: "not-a-folder").path(percentEncoded: false))
        }
        // The single-source sync works for a known id.
        let single = try await lamp.syncSource(sourceId: source.id, request: SyncRequest(), observer: SyncEventStream())
        #expect(single.ok)
        #expect(single.kind == .folder)
    }

    @Test("A second sync while one runs is Busy, and so is removing a source")
    func overlappingSyncsAreBusy() async throws {
        let sandbox = try Sandbox()
        defer { sandbox.remove() }
        try sandbox.makeCourseFolder()
        let lamp = try await PageLamp.openWithMemorySecrets(dataDir: sandbox.dataDir)
        let source = try await lamp.addFolderSource(path: sandbox.courses, termStart: nil, label: nil)

        let gate = GateObserver()
        let first = Task.detached {
            try await lamp.syncAll(request: SyncRequest(), observer: gate)
        }
        let started = await gate.waitUntilStarted()
        #expect(started)
        guard started else {
            gate.release.signal()
            return
        }
        #expect(try await lamp.status().syncInProgress)
        await expectError(.busy) {
            try await lamp.syncAll(request: SyncRequest(), observer: SyncEventStream())
        }
        await expectError(.busy) {
            try await lamp.syncSource(sourceId: source.id, request: SyncRequest(), observer: SyncEventStream())
        }
        await expectError(.busy) { try await lamp.removeSource(sourceId: source.id) }

        gate.release.signal()
        let summary = try await first.value
        #expect(summary.ok)
        #expect(try await !lamp.status().syncInProgress)
    }

    @Test("Progress callbacks run on PageLamp's threads and reach the main actor through the stream")
    @MainActor
    func progressReachesTheMainActor() async throws {
        let sandbox = try Sandbox()
        defer { sandbox.remove() }
        try sandbox.makeCourseFolder()
        let lamp = try await PageLamp.openWithMemorySecrets(dataDir: sandbox.dataDir)
        _ = try await lamp.addFolderSource(path: sandbox.courses, termStart: nil, label: nil)

        let observer = RecordingObserver()
        let sync = Task.detached {
            try await lamp.syncAll(request: SyncRequest(), observer: observer)
        }
        var received = 0
        for await event in observer.stream.events {
            MainActor.assertIsolated()
            received += 1
            if case .sourceFinished = event { break }
        }
        let summary = try await sync.value
        observer.stream.finish()
        #expect(summary.ok)
        #expect(received >= 3)
        let threads = observer.threads.withLock { $0 }
        #expect(threads.count == received)
        #expect(threads.allSatisfy { $0 == "pagelamp-rt" }, "\(threads)")
    }

    @Test("Swift's SyncRequest() is the facade's default, and the version is set")
    func defaultsAndVersion() {
        _ = TestEnvironment.root
        #expect(SyncRequest() == defaultSyncRequest())
        #expect(SyncRequest().maxFileMb == 50)
        #expect(!version().isEmpty)
        #expect(FileManager.default.fileExists(
            atPath: TestEnvironment.root.appending(path: "diagnostics/logs").path(percentEncoded: false)
        ))
    }

    @Test("Progress streams keep the run's order and drop events after finish()")
    func progressStreams() async {
        let stream = GenEventStream()
        stream.onEvent(event: .stage(stage: .buildingContext))
        stream.onEvent(event: .stage(stage: .waitingForModel))
        stream.finish()
        // A consumer that stopped listening: later events go nowhere, and nothing blocks.
        stream.onEvent(event: .stage(stage: .validating))
        var seen: [GenEvent] = []
        for await event in stream.events { seen.append(event) }
        #expect(seen == [.stage(stage: .buildingContext), .stage(stage: .waitingForModel)])
        // Each stream is its call's observer.
        let _: any CalendarBatchObserver = CalendarBatchEventStream()
        let _: any CodexInstallObserver = CodexInstallEventStream()
        let _: any CodexLoginObserver = CodexLoginEventStream()
    }
}
