// Reminders as notifications on the Mac (model-access design §5.3, Delivery; macos-shell §3.5).
// The facade decides what is due, how old is too old and what was shown; this hands its answers
// to the system's notification center:
//
// - Nothing is scheduled and no permission is asked until the student says "Remind me" (here,
//   in the Mac app: its own answer). Then the system's prompt, and one "Reminders are on".
// - A pass (after each refresh, a settings change, a time zone change, a wake):
//   1. what the system showed since the last pass is marked shown: Notification Center's list,
//      and what was handed over and has fired since (the student may have cleared it);
//   2. off, or not allowed: our waiting requests go, and what was handed over is forgotten
//      without marking it (it never fired);
//   3. `due_reminders` (what was never handed over: the app wasn't opened for a week, or
//      reminders were just turned on) is shown now and marked;
//   4. `reminders(now, now + 7 days)`, the earliest 64 (the system's limit per app), become
//      calendar triggers (`ReminderTrigger`); requests no longer in that list go.
// - With reminders off or not allowed at launch, what came due shows in the app instead (the
//   catch-up card, from `startup_tasks`).
//
// The one thing kept here is which reminders were handed to the system and when they fire
// (`SettingsStore.handedOverReminders`, at most 64): what the system holds, not policy.

import Foundation
import Observation
import PageLampKit

@Observable @MainActor
public final class ReminderDelivery {
    /// How far ahead reminders are scheduled.
    public static let window: TimeInterval = 7 * 86_400
    /// The system keeps at most 64 waiting requests per app.
    public static let pendingLimit = 64
    /// The one notification after "Remind me" (not a reminder: never marked, never removed).
    public static let remindersOnId = "pagelamp.reminders-on"
    private static let reminderPrefixes = ["deadline_soon:", "weekly_digest:", "plan_today:"]

    /// Whether PageLamp may notify (System Settings ▸ Notifications), as of the last look.
    public private(set) var permission: NotificationPermission = .notDetermined
    /// The student's answer to "Remind me" in this app; nil until read.
    public private(set) var consent: Bool?
    /// Reminders were off (or not allowed) at launch: what came due since, until opened or
    /// dismissed (the catch-up card).
    public private(set) var catchUp: [Reminder] = []
    /// The answer given to "Remind me" on the first-sync page in this run (its line stays).
    public var questionAnswer: Bool?

    @ObservationIgnored private var service: any PageLampService
    @ObservationIgnored private var center: any NotificationCenterClient
    @ObservationIgnored private var record: any SettingsStore
    @ObservationIgnored private let clock: @Sendable () -> Date
    @ObservationIgnored private let l10n: @MainActor () -> L10n
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var running: Task<Void, Never>?
    @ObservationIgnored private var passRequested = false

    /// - Parameter record: where the handed-over reminders are kept (the app's settings for live
    ///   data; memory for mock data, so a mock pass never touches the live record).
    public init(
        service: any PageLampService,
        center: any NotificationCenterClient,
        record: any SettingsStore,
        clock: @escaping @Sendable () -> Date,
        l10n: @escaping @MainActor () -> L10n
    ) {
        self.service = service
        self.center = center
        self.record = record
        self.clock = clock
        self.l10n = l10n
    }

    /// Another data source (mock ↔ live): passes of the old one stop applying.
    public func replace(service: any PageLampService, center: any NotificationCenterClient, record: any SettingsStore) {
        generation += 1
        self.service = service
        self.center = center
        self.record = record
        permission = .notDetermined
        consent = nil
        catchUp = []
    }

    // MARK: - Launch and consent

    /// At launch, with `startup_tasks`' due reminders: shown in the app when reminders are off
    /// or not allowed; otherwise the first pass shows them as notifications.
    public func launch(due: [Reminder]) async {
        let generation = self.generation
        guard let state = await readState(), generation == self.generation else { return }
        consent = state.consent
        permission = state.permission
        catchUp = state.consent && state.permission == .allowed ? [] : due
    }

    /// "Remind me" (yes or no) in this app. Yes asks the system (its prompt the first time) and,
    /// when allowed, sends one "Reminders are on". Throws when the answer couldn't be saved.
    public func setConsent(_ on: Bool) async throws(PageLampFailure) {
        let generation = self.generation
        let service = self.service
        let settings = try await service.reminderSettings().with { $0.runInBackground = on }
        try await service.setReminderSettings(settings: settings)
        guard generation == self.generation else { return }
        consent = on
        if on {
            let center = self.center
            let allowed = await center.requestPermission()
            permission = await center.permission()
            if allowed {
                let text = ReminderText.remindersOn(l10n: l10n())
                try? await center.add(PlannedNotification(id: Self.remindersOnId, title: text.title, body: text.body, trigger: nil))
                // What the card listed is due and unshown: the pass shows it as notifications.
                catchUp = []
            }
        }
        requestPass()
    }

    /// Re-reads the permission (the student may have changed it in System Settings).
    public func refreshPermission() async {
        let generation = self.generation
        let permission = await center.permission()
        if generation == self.generation { self.permission = permission }
    }

    // MARK: - The catch-up card

    /// Opened one: it counts as shown. Returns its course, if any, to open.
    public func openCatchUp(_ reminder: Reminder) async -> String? {
        catchUp.removeAll { $0.id == reminder.id }
        _ = await markShown([reminder.id], service: service)
        return reminder.courseId
    }

    /// Dismissed the card: everything on it counts as shown.
    public func dismissCatchUp() async {
        let ids = catchUp.map(\.id)
        catchUp = []
        _ = await markShown(ids, service: service)
    }

    // MARK: - The system's callbacks

    /// A notification fired while the app was frontmost, or the student clicked one: shown.
    public func shown(id: String) async {
        guard Self.isReminderId(id) else { return }
        let settled = await markShown([id], service: service)
        record.handedOverReminders = record.handedOverReminders.filter { !settled.contains($0.key) }
    }

    // MARK: - Passes

    /// Asks for a pass. Passes never overlap: one asked for while another runs follows it.
    public func requestPass() {
        passRequested = true
        guard running == nil else { return }
        running = Task { [weak self] in
            while let self, self.passRequested {
                self.passRequested = false
                await self.pass()
            }
            self?.running = nil
        }
    }

    /// Waits until no pass runs or is asked for (tests, snapshots).
    public func settle() async {
        while let running { await running.value }
    }

    private func pass() async {
        let generation = self.generation
        let service = self.service
        let center = self.center
        let now = clock()
        guard let state = await readState(), generation == self.generation else { return }
        let (consent, permission) = (state.consent, state.permission)
        self.consent = consent
        self.permission = permission

        var handedOver = record.handedOverReminders
        let pending = Set(await center.pendingIds())
        if permission == .allowed {
            // 1. What the system showed since the last pass.
            let delivered = await center.deliveredIds().filter(Self.isReminderId)
            let fired = handedOver.filter { $0.value <= now && !pending.contains($0.key) }.map(\.key)
            let settled = await markShown(Array(Set(delivered + fired)).sorted(), service: service)
            handedOver = handedOver.filter { !settled.contains($0.key) }
        }
        guard generation == self.generation else { return }
        guard consent, permission == .allowed else {
            // 2. Off: nothing of ours waits in the system; what was handed over never fired.
            await center.removePending(pending.filter(Self.isReminderId).sorted())
            record.handedOverReminders = [:]
            return
        }

        // 3. Due and never handed over: now.
        let l10n = self.l10n()
        if let due = try? await service.dueReminders(now: now) {
            for reminder in due where handedOver[reminder.id] == nil {
                let text = ReminderText.notification(for: reminder, l10n: l10n)
                guard (try? await center.add(PlannedNotification(
                    id: reminder.id, title: text.title, body: text.body, trigger: nil, courseId: reminder.courseId
                ))) != nil else { break }
                if await markShown([reminder.id], service: service).isEmpty {
                    // Not marked (the store failed): the next pass marks it instead of showing it again.
                    handedOver[reminder.id] = reminder.fireAt
                }
            }
        }
        guard generation == self.generation else { return }

        // 4. The week ahead as calendar triggers.
        guard let window = try? await service.reminders(from: now, to: now.addingTimeInterval(Self.window)) else {
            record.handedOverReminders = Self.capped(handedOver, keeping: [])
            return
        }
        let planned = Array(window.prefix(Self.pendingLimit))
        let wanted = Set(planned.map(\.id))
        let gone = pending.filter { Self.isReminderId($0) && !wanted.contains($0) }
        await center.removePending(gone.sorted())
        for id in gone { handedOver[id] = nil }
        for reminder in planned {
            let text = ReminderText.notification(for: reminder, l10n: l10n)
            let notification = PlannedNotification(
                id: reminder.id, title: text.title, body: text.body,
                trigger: ReminderTrigger.components(for: reminder), courseId: reminder.courseId
            )
            guard (try? await center.add(notification)) != nil else { break }
            handedOver[reminder.id] = reminder.fireAt
        }
        guard generation == self.generation else { return }
        record.handedOverReminders = Self.capped(handedOver, keeping: wanted)
    }

    // MARK: - Helpers

    /// The student's answer and the system's permission; nil when the settings can't be read.
    private func readState() async -> (consent: Bool, permission: NotificationPermission)? {
        let service = self.service
        let center = self.center
        guard let settings = try? await service.reminderSettings() else { return nil }
        return (settings.runInBackground, await center.permission())
    }

    /// Marks each id shown on its own, so one the facade refuses (`Invalid`: a stale or malformed
    /// id) never holds up the others. Returns the ids that are settled: marked, or refused.
    private func markShown(_ ids: [String], service: any PageLampService) async -> Set<String> {
        var settled = Set<String>()
        for id in ids {
            do throws(PageLampFailure) {
                try await service.markRemindersShown(ids: [id])
                settled.insert(id)
            } catch {
                if error.kind == .invalid { settled.insert(id) }
            }
        }
        return settled
    }

    /// At most 64 entries: the ones just scheduled first, then the latest of the rest.
    private static func capped(_ record: [String: Date], keeping wanted: Set<String>) -> [String: Date] {
        guard record.count > pendingLimit else { return record }
        let ordered = record.sorted { a, b in
            let aw = wanted.contains(a.key), bw = wanted.contains(b.key)
            return aw != bw ? aw : a.value > b.value
        }
        return Dictionary(uniqueKeysWithValues: ordered.prefix(pendingLimit).map { ($0.key, $0.value) })
    }

    static func isReminderId(_ id: String) -> Bool {
        reminderPrefixes.contains { id.hasPrefix($0) }
    }
}
