// Synthetic demo data for the preview's mock mode and for tests, ported from the Tauri app's
// apps/desktop/src/api/mock/fixtures.ts. EVERYTHING here is made up (docs/ARCHITECTURE.md §3.7):
// no real courses, people, schools or tokens. Dates are relative to `now`, so the demo always
// looks live (a deadline in 2 days, week 4 of term, …).

import Foundation
import PageLampKit

/// Which state the mock starts in (the Tauri mock's `?scenario=`).
public enum MockScenario: String, CaseIterable, Codable, Sendable {
    /// Three sources, four courses (one hidden, one past), a study plan; everything fine.
    case demo
    /// No sources yet (S3); the first folder or Canvas sync "finds" the demo courses.
    case empty
    /// The demo, but the Canvas token has expired (S7): the preview's default.
    case expired
    /// The demo, but the course folder is missing.
    case error
    /// The demo while another process (the CLI) holds the sync lock (S17).
    case busy
    /// The demo after a crash of the MCP server (S6).
    case crashed
    /// AI (the Tauri mock's scenarios): an OpenAI key set up and acknowledged, a model per feature,
    /// this month's and last month's usage.
    case aiKey
    /// A model on this computer (Ollama).
    case aiLocal
    /// Like aiKey, but explanations use a model without a known price.
    case aiUnpriced
    /// Like aiKey, but this month's usage has almost reached the budget.
    case aiBudget
    /// An Anthropic key whose disclosure changed since it was acknowledged.
    case aiDisclosureChanged
    /// A custom endpoint that can't be reached (listing models fails; "Test" is rate-limited).
    case aiErrors
    /// Like aiKey, with "prepare it on Monday" on and every day a Monday (the Tauri mock's
    /// weekly-note-monday): Monday's note is prepared once.
    case weeklyNoteMonday

    /// What the preview app starts with: the demo plus a failing Canvas token, so the attention
    /// states are visible without setup.
    public static let preview: MockScenario = .expired
}

/// A source whose sync state the mock changes.
struct MockSource: Sendable {
    var id: String
    var kind: SourceKind
    var label: String
    var config: String
    var lastSyncedAt: Date?
    var lastError: String?
    var lastErrorKind: SourceErrorKind?

    var record: SourceRecord {
        SourceRecord(
            id: id, kind: kind, label: label, config: config, lastSyncedAt: lastSyncedAt,
            lastError: lastError, lastErrorKind: lastErrorKind
        )
    }
}

/// One course with everything the views need.
struct MockCourse: Sendable {
    var course: Course
    var timeline: CourseTimeline
    var modules: [Module]
    var materials: [MaterialView]
    var announcements: [MaterialView]
    var deadlines: [Deadline]
    /// "I'm still taking this" until this day.
    var keptCurrentUntil: String? = nil
}

/// The course calendar and lifecycle fields (M0.10) in neutral values: a course with a week is
/// teaching, one without is in an unknown phase (both active); the term is unresolved, there is
/// no calendar, and every course is in the Current group.
enum MockCalendar {
    static var unresolvedTerm: TermResolution {
        TermResolution(
            weekOneMonday: nil, teaching: [], breaks: [], examsEnd: nil, anchor: .noAnchor,
            anchorConfidence: .low, anchorOrigin: nil, aiLabel: nil, outerFrame: nil, notUsed: [],
            studentStart: nil, studentEnd: nil
        )
    }

    static func lifecycle(_ timeline: CourseTimeline, keptCurrentUntil: String? = nil) -> CourseLifecycle {
        // "I'm still taking this" makes a course Current with high confidence (lifecycle rule 1).
        let kept = keptCurrentUntil != nil
        return CourseLifecycle(
            state: kept || timeline.currentWeek != nil ? .current : .unknown, group: .current,
            confidence: kept ? .high : timeline.confidence, since: nil, startsOn: nil, lastActivity: nil,
            nextEvent: nil, evidenceItems: [], suggestRemoval: false, keptCurrentUntil: keptCurrentUntil,
            // The facade's rule: Current and Unknown courses are active (study plans cover them).
            isActive: true
        )
    }
}

struct MockDb: Sendable {
    var dataDir: String
    var sources: [MockSource]
    var courses: [MockCourse]
    var studyPlan: StoredStudyPlan?
    /// Simulates another process (the CLI) holding sync.lock.
    var externalSyncRunning: Bool
    /// What the panic hook recorded last time ("crashed" scenario).
    var lastCrash: CrashReport?
    var updates = MockUpdates()
    /// "Not now" / "Keep" on removal suggestions, by course id.
    var removalSnoozes: [String: String] = [:]
    var bannerSnoozedUntil: String?
    /// The M1–M3 state the mock keeps (AI settings, reminders, removed courses).
    var features = MockFeatures()
}

/// Update settings and the launch state behind `startupTasks` (a fresh install by default).
struct MockUpdates: Sendable {
    var prefs = UpdatePrefs(autoCheck: true, channel: nil)
    var disclosureSeen = false
    /// This launch followed an update (`MockService.simulateUpgrade`).
    var upgraded = false
    /// The version it updated from (nil: 0.1, which recorded none).
    var upgradedFrom: String?
    var whatsNewSeen = false
    var lastCheck: UpdateCheckRecord?
}

enum MockIds {
    static let folder = "folder:demo-courses"
    static let ical = "ical:demo-calendar"
    static let canvas = "canvas:canvas.demo.test"
}

/// Builds the mock database for one scenario at one moment.
struct MockFixtures {
    let now: Date
    let calendar: Calendar
    private var materialSeq = 0
    private var eventSeq = 0

    static let dataDir = "/Users/demo/Library/Application Support/dev.PageLamp.PageLamp"

    init(now: Date, calendar: Calendar) {
        self.now = now
        self.calendar = calendar
    }

    static func database(_ scenario: MockScenario, now: Date, calendar: Calendar) -> MockDb {
        var fixtures = MockFixtures(now: now, calendar: calendar)
        if scenario == .empty {
            return MockDb(
                dataDir: dataDir, sources: [], courses: [], studyPlan: nil,
                externalSyncRunning: false, lastCrash: nil
            )
        }
        let courses = fixtures.courses()
        return MockDb(
            dataDir: dataDir,
            sources: fixtures.sources(scenario),
            courses: courses,
            studyPlan: fixtures.studyPlan(courses),
            externalSyncRunning: scenario == .busy,
            lastCrash: scenario == .crashed ? fixtures.crash() : nil
        )
    }

    // MARK: Calendar arithmetic (calendar days, not "+ N × 24 h", so DST changes stay right)

    func at(_ days: Int, _ hour: Int = 12, _ minute: Int = 0) -> Date {
        let day = calendar.date(byAdding: .day, value: days, to: calendar.startOfDay(for: now)) ?? now
        return calendar.date(bySettingHour: hour, minute: minute, second: 0, of: day) ?? day
    }

    func dateOnly(_ days: Int) -> String {
        let day = calendar.date(byAdding: .day, value: days, to: calendar.startOfDay(for: now)) ?? now
        return IsoDate.string(from: day, calendar: calendar)
    }

    // MARK: Sources

    func sources(_ scenario: MockScenario) -> [MockSource] {
        let ok = now.addingTimeInterval(-2 * 60 * 60)
        var folder = MockSource(
            id: MockIds.folder, kind: .folder, label: "Course folder",
            config: #"{"path":"/Users/demo/Documents/Courses","term_start":"\#(dateOnly(-23))"}"#,
            lastSyncedAt: ok
        )
        if scenario == .error {
            folder.lastError = "Folder /Users/demo/Documents/Courses was not found."
            folder.lastErrorKind = .notFound
        }
        let ical = MockSource(
            id: MockIds.ical, kind: .ical, label: "Course calendar", config: "{}", lastSyncedAt: ok
        )
        var canvas = MockSource(
            id: MockIds.canvas, kind: .canvas, label: "Demo Canvas",
            config: #"{"base_url":"https://canvas.demo.test","account_name":"Demo Student"}"#,
            lastSyncedAt: ok
        )
        if scenario == .expired {
            canvas.lastSyncedAt = at(-9, 18)
            canvas.lastError = "Canvas rejected the access token (401). It may have expired or been revoked."
            canvas.lastErrorKind = .authExpiredOrRevoked
        }
        return [folder, ical, canvas]
    }

    // MARK: Courses

    private struct CourseSpec {
        var id: String
        var sourceId: String
        var code: String
        var name: String
        var policy: AiPolicy
        var policyNote: String?
        var hidden: Bool
        var aiAccess = true
        /// False = Canvas no longer lists the course as active (term over).
        var enrollmentActive = true
        var termStartDays: Int?
        var week: UInt32?
        var confidence: Confidence
        var evidence: [String]
        var url: String?
    }

    private func course(_ spec: CourseSpec) -> Course {
        Course(
            id: spec.id,
            sourceId: spec.sourceId,
            externalId: spec.code,
            code: spec.code,
            name: spec.name,
            termStart: spec.termStartDays.map { dateOnly($0) },
            termEnd: spec.termStartDays.map { dateOnly($0 + 12 * 7 + 4) },
            termSource: spec.termStartDays == nil ? .none : .synced,
            url: spec.url,
            aiPolicy: spec.policy,
            aiPolicyNote: spec.policyNote,
            aiAccess: spec.aiAccess,
            materialSharing: .unanswered,
            hidden: spec.hidden,
            enrollmentActive: spec.enrollmentActive,
            updatedAt: at(-1, 9)
        )
    }

    private func timeline(_ spec: CourseSpec, moduleIds: [String]) -> CourseTimeline {
        CourseTimeline(
            asOf: dateOnly(0),
            currentWeek: spec.week,
            confidence: spec.confidence,
            evidence: spec.evidence,
            currentModuleIds: moduleIds,
            outsideTerm: false,
            phase: spec.week == nil ? .unknown : .teaching,
            phaseConfidence: spec.confidence,
            startsOn: nil,
            defaultWeek: spec.week,
            breakAfterWeek: nil,
            lastTeachingWeek: nil,
            currentBreakKind: nil,
            notesWeek: nil,
            term: MockCalendar.unresolvedTerm,
            calendar: .noCalendar,
            evidenceItems: []
        )
    }

    private struct MaterialOptions {
        var status: TextStatus = .ok
        var chunks: UInt32 = 8
        var module: Module?
        var error: String?
        var url: String?
        /// A Canvas file that can't be downloaded on request.
        var blocked: DownloadBlock?
    }

    private mutating func material(
        _ courseId: String, _ title: String, _ kind: MaterialKind, week: UInt32?,
        published days: Int, _ options: MaterialOptions = MaterialOptions()
    ) -> MaterialView {
        materialSeq += 1
        return MaterialView(
            id: "\(courseId)/material/\(materialSeq)",
            courseId: courseId,
            title: title,
            kind: kind,
            moduleId: options.module?.id,
            moduleName: options.module?.name,
            weekHint: week,
            publishedAt: at(days, 9, 30),
            url: options.url ?? "https://canvas.demo.test/files/\(materialSeq)",
            textStatus: options.status,
            textError: options.error,
            downloadBlocked: options.blocked,
            chunkCount: options.status == .ok ? options.chunks : 0
        )
    }

    private mutating func deadline(
        _ course: Course, _ title: String, _ kind: EventKind, due days: Int,
        hour: Int = 23, minute: Int = 59
    ) -> Deadline {
        eventSeq += 1
        let when = at(days, hour, minute)
        let isDue = kind == .assignmentDue || kind == .quizDue || kind == .plannerItem
        return Deadline(
            event: Event(
                id: "\(MockIds.ical)/event/\(eventSeq)",
                sourceId: MockIds.ical,
                courseId: course.id,
                kind: kind,
                title: title,
                startsAt: isDue ? nil : when,
                endsAt: nil,
                dueAt: isDue ? when : nil,
                url: "https://canvas.demo.test/calendar#event-\(eventSeq)",
                updatedAt: at(-1, 8),
                courseHint: nil
            ),
            courseCode: course.code,
            courseName: course.name
        )
    }

    private func weekModules(_ courseId: String, _ names: [String], termStartDays: Int) -> [Module] {
        names.enumerated().map { index, name in
            Module(
                id: "\(courseId)/module/\(index + 1)",
                courseId: courseId,
                name: name,
                position: Int64(index + 1),
                unlockAt: at(termStartDays + index * 7, 8),
                weekHint: UInt32(index + 1)
            )
        }
    }

    mutating func courses() -> [MockCourse] {
        [demo101(), demo205(), demo310(), demo099()]
    }

    private mutating func demo101() -> MockCourse {
        let spec = CourseSpec(
            id: "\(MockIds.folder)/course/DEMO101",
            sourceId: MockIds.folder,
            code: "DEMO101",
            name: "Intro to Demo Studies",
            policy: .learningAid,
            policyNote: "Syllabus §5: AI tools may be used to review and understand material, not to write graded work.",
            hidden: false,
            termStartDays: -23,
            week: 4,
            confidence: .high,
            evidence: [
                "Module 'Week 4: Sampling and Surveys' unlocked \(dateOnly(-2))",
                "Term started \(dateOnly(-23)) (course folder settings) → week 4, which agrees",
                "Reading week is not modelled in v0.1",
            ],
            url: nil
        )
        let c = course(spec)
        let modules = weekModules(
            c.id,
            [
                "Week 1: What Is a Demo?",
                "Week 2: Placeholder Data",
                "Week 3: Measuring Nothing Carefully",
                "Week 4: Sampling and Surveys",
            ],
            termStartDays: -23
        )
        let (m1, m2, m3, m4) = (modules[0], modules[1], modules[2], modules[3])
        let materials = [
            material(c.id, "Week 1 slides — What Is a Demo?", .file, week: 1, published: -23, .init(chunks: 24, module: m1)),
            material(c.id, "Course syllabus", .syllabus, week: nil, published: -24, .init(chunks: 6)),
            material(c.id, "Week 2 slides — Placeholder Data", .file, week: 2, published: -16, .init(chunks: 28, module: m2)),
            material(c.id, "Reading: Chapter 2, Making Up Numbers Responsibly", .file, week: 2, published: -16, .init(chunks: 14, module: m2)),
            material(c.id, "Week 3 slides — Measuring Nothing Carefully", .file, week: 3, published: -9, .init(chunks: 31, module: m3)),
            material(c.id, "Lab 3 notebook", .file, week: 3, published: -9, .init(chunks: 12, module: m3)),
            material(c.id, "Week 3 lecture recording", .file, week: 3, published: -8, .init(status: .unsupported, module: m3)),
            material(c.id, "Week 4 slides — Sampling and Surveys", .file, week: 4, published: -2, .init(chunks: 32, module: m4)),
            // Looks like graded work: left out of an explanation unless the student includes it
            // (then it is the week's second readable material, so the mock's two-material limit
            // reads it, as in the Tauri mock).
            material(c.id, "Assignment 4 — Survey Simulation", .file, week: 4, published: -2, .init(chunks: 15, module: m4)),
            material(c.id, "Reading: Chapter 4, Who Gets Asked", .file, week: 4, published: -2, .init(chunks: 18, module: m4)),
            material(c.id, "Survey dataset (large archive)", .file, week: 4, published: -2, .init(status: .notDownloaded, module: m4)),
            material(c.id, "Week 4 practice questions", .page, week: 4, published: -1, .init(chunks: 3, module: m4)),
            material(
                c.id, "Scanned handout — sampling frames", .file, week: 4, published: -1,
                .init(status: .error, module: m4, error: "The PDF contains only images; no text could be extracted.")
            ),
        ]
        let announcements = [
            material(c.id, "Office hours move to Thursday this week", .announcement, week: nil, published: -1, .init(chunks: 1)),
            material(c.id, "Problem Set 2 is posted", .announcement, week: nil, published: -6, .init(chunks: 1)),
        ]
        let deadlines = [
            deadline(c, "Reading response 3", .assignmentDue, due: -3),
            deadline(c, "Problem Set 2", .assignmentDue, due: 2),
            deadline(c, "Quiz 3 — Sampling", .quizDue, due: 5, hour: 10, minute: 0),
            deadline(c, "Lecture 9", .classEvent, due: 1, hour: 10, minute: 0),
            deadline(c, "Midterm test", .exam, due: 12, hour: 18, minute: 0),
        ]
        return MockCourse(
            course: c, timeline: timeline(spec, moduleIds: [m4.id]), modules: modules,
            materials: materials, announcements: announcements, deadlines: deadlines
        )
    }

    private mutating func demo205() -> MockCourse {
        let spec = CourseSpec(
            id: "\(MockIds.canvas)/course/205",
            sourceId: MockIds.canvas,
            code: "DEMO205",
            name: "Foundations of Sample Data",
            policy: .unknown,
            policyNote: nil,
            hidden: false,
            termStartDays: -24,
            week: 4,
            confidence: .medium,
            evidence: ["Term started \(dateOnly(-24)) (from Canvas) → week 4"],
            url: "https://canvas.demo.test/courses/205"
        )
        let c = course(spec)
        let modules = weekModules(c.id, ["Unit A: Tables", "Unit B: Columns", "Unit C: Rows"], termStartDays: -24)
        let (ma, mb, mc) = (modules[0], modules[1], modules[2])
        let materials = [
            material(c.id, "Unit A notes", .page, week: 1, published: -24, .init(chunks: 9, module: ma)),
            material(c.id, "Unit B notes", .page, week: 2, published: -17, .init(chunks: 11, module: mb)),
            material(c.id, "Unit C notes", .page, week: 3, published: -10, .init(chunks: 10, module: mc)),
            // Canvas files stay "not downloaded" until the student asks (download_course_files).
            material(c.id, "Unit C worked examples", .file, week: 4, published: -3, .init(status: .notDownloaded, module: mc)),
            // Over the download limit: listed, but asking for a download won't help.
            material(
                c.id, "Unit C lecture recording", .file, week: 4, published: -3,
                .init(status: .notDownloaded, module: mc, blocked: .tooLarge)
            ),
            material(c.id, "Course website", .externalLink, week: nil, published: -24, .init(status: .unsupported)),
        ]
        let deadlines = [
            deadline(c, "Exercise set 3", .assignmentDue, due: 6),
            deadline(c, "Exercise set 4", .assignmentDue, due: 13),
        ]
        return MockCourse(
            course: c, timeline: timeline(spec, moduleIds: [mc.id]), modules: modules,
            materials: materials, announcements: [], deadlines: deadlines
        )
    }

    private mutating func demo310() -> MockCourse {
        let spec = CourseSpec(
            id: "\(MockIds.folder)/course/DEMO310",
            sourceId: MockIds.folder,
            code: "DEMO310",
            name: "Seminar in Example Analysis",
            policy: .prohibited,
            policyNote: "Syllabus p. 2: no generative AI for any part of this course.",
            hidden: false,
            termStartDays: nil,
            week: nil,
            confidence: .low,
            evidence: ["No term dates and no week-numbered folders — set the term start to fix this"],
            url: nil
        )
        let c = course(spec)
        let materials = [
            material(c.id, "Seminar reading list", .file, week: nil, published: -20, .init(chunks: 4)),
            material(c.id, "Discussion guide", .file, week: nil, published: -5, .init(chunks: 6)),
        ]
        return MockCourse(
            course: c, timeline: timeline(spec, moduleIds: []), modules: [], materials: materials,
            announcements: [], deadlines: [deadline(c, "Seminar presentation", .assignmentDue, due: 9, hour: 14, minute: 0)]
        )
    }

    private mutating func demo099() -> MockCourse {
        let spec = CourseSpec(
            id: "\(MockIds.canvas)/course/99",
            sourceId: MockIds.canvas,
            code: "DEMO099",
            name: "Orientation Placeholder",
            policy: .unknown,
            policyNote: nil,
            hidden: true,
            enrollmentActive: false,
            termStartDays: -30,
            week: 5,
            confidence: .medium,
            evidence: ["Term started \(dateOnly(-30)) (from Canvas) → week 5"],
            url: "https://canvas.demo.test/courses/99"
        )
        let c = course(spec)
        return MockCourse(
            course: c, timeline: timeline(spec, moduleIds: []), modules: [],
            materials: [material(c.id, "Welcome page", .page, week: 1, published: -30, .init(chunks: 2))],
            announcements: [], deadlines: []
        )
    }

    // MARK: Study plan (as if the student's AI app saved it via MCP `save_study_plan`)

    func studyPlan(_ courses: [MockCourse]) -> StoredStudyPlan {
        let c101 = courses[0].course.id
        let c205 = courses[1].course.id
        func item(_ days: Int, _ course: String, _ title: String, _ minutes: UInt32, done: Bool = false) -> StudyPlanItem {
            StudyPlanItem(
                date: dateOnly(days), courseId: course, title: title, description: nil,
                materialIds: [], minutes: minutes, done: done
            )
        }
        return StoredStudyPlan(
            id: 1,
            createdAt: at(-2, 20, 15),
            plan: StudyPlan(
                horizonStart: dateOnly(-1),
                horizonEnd: dateOnly(12),
                items: [
                    item(-1, c101, "Skim Week 4 slides", 30, done: true),
                    item(0, c101, "Read Chapter 4 and summarise sampling frames", 60),
                    item(0, c205, "Work through Unit C examples", 45),
                    item(1, c101, "Problem Set 2 — questions 1–3", 90),
                    item(2, c101, "Problem Set 2 — finish and check", 60),
                    item(4, c101, "Quiz 3 practice questions", 40),
                    item(6, c205, "Exercise set 3", 75),
                    item(8, c101, "Midterm review: weeks 1–2", 90),
                ],
                notes: "Front-load Problem Set 2, then shift to midterm review from next week."
            ),
            origin: .aiApp
        )
    }

    func crash() -> CrashReport {
        CrashReport(
            time: at(-1, 21, 14),
            version: "0.1.0-mock",
            process: .app,
            message: "called `Option::unwrap()` on a `None` value",
            location: "crates/pagelamp-app/src/sync.rs:212:31"
        )
    }

    // MARK: Connect your AI app

    static func mcpClientConfigs(binary: String) -> [McpClientConfig] {
        let launch = McpLaunch(command: binary, args: ["mcp"], env: [:], temporaryLocation: nil)
        return [
            McpClientConfig(
                client: .claudeDesktop,
                title: "Claude Desktop",
                installKind: .jsonSnippet,
                configPathHint: "~/Library/Application Support/Claude/claude_desktop_config.json",
                content: """
                {
                  "mcpServers": {
                    "pagelamp": {
                      "command": "\(binary)",
                      "args": [
                        "mcp"
                      ]
                    }
                  }
                }
                """,
                notes: [
                    "Works on every Claude plan, including Free.",
                    "On Team, Enterprise and Education plans an admin can turn extensions off.",
                    "Quit Claude Desktop before editing its config: it rewrites the file when it quits.",
                ],
                noteCodes: [.worksOnAllClaudePlans, .adminsMayDisableExtensions, .quitBeforeEditing],
                launch: launch
            ),
            McpClientConfig(
                client: .claudeCode,
                title: "Claude Code",
                installKind: .shellCommand,
                configPathHint: nil,
                content: "claude mcp add --scope user pagelamp -- '\(binary)' mcp",
                notes: [
                    "Claude Code needs a paid Claude plan (Pro or higher).",
                    "Run this once in a terminal; new Claude Code sessions then have the server.",
                ],
                noteCodes: [.needsPaidClaudePlan, .restartClientAfterChange],
                launch: launch
            ),
            McpClientConfig(
                client: .codex,
                title: "Codex / ChatGPT desktop (Work/Codex mode)",
                installKind: .tomlSnippet,
                configPathHint: "~/.codex/config.toml",
                content: "[mcp_servers.pagelamp]\ncommand = \"\(binary)\"\nargs = [\"mcp\"]\n",
                notes: [
                    "The ChatGPT desktop app (Work/Codex mode) reads the same ~/.codex/config.toml.",
                    "Documented for ChatGPT Plus and higher, and for Edu.",
                    "Support on the Free and Go plans isn't documented.",
                    "Restart the app after changing its config.",
                ],
                noteCodes: [
                    .codexConfigSharedWithChatgptDesktop, .codexPlusAndEduDocumented,
                    .freeGoUndocumented, .restartClientAfterChange,
                ],
                launch: launch
            ),
            McpClientConfig(
                client: .generic,
                title: "Other MCP clients",
                installKind: .jsonSnippet,
                configPathHint: nil,
                // A bare server definition (no "mcpServers" wrapper), like the backend's.
                content: """
                {
                  "command": "\(binary)",
                  "args": [
                    "mcp"
                  ]
                }
                """,
                notes: [
                    "Any MCP client that can launch a local (stdio) server can use PageLamp: adapt this command, arguments and environment to that client's config format.",
                ],
                noteCodes: [.genericStdioClient],
                launch: launch
            ),
        ]
    }
}

/// "YYYY-MM-DD" dates (the facade's `IsoDate`) in a given calendar.
public enum IsoDate {
    public static func string(from date: Date, calendar: Calendar) -> String {
        let parts = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0)
    }

    /// Midnight of that day in `calendar`, or nil for a malformed string.
    public static func date(from string: String, calendar: Calendar) -> Date? {
        let parts = string.split(separator: "-")
        guard parts.count == 3, let year = Int(parts[0]), let month = Int(parts[1]), let day = Int(parts[2]) else {
            return nil
        }
        return calendar.date(from: DateComponents(year: year, month: month, day: day))
    }
}
