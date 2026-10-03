// The system's notification center as the reminders see it (model-access design §5.3, Delivery):
// the system one for live data inside the app bundle, a recording one for mock data, tests and
// snapshots, so synthetic reminders never reach Notification Center.

import Foundation

/// Whether PageLamp may show notifications (System Settings ▸ Notifications).
public enum NotificationPermission: Equatable, Sendable {
    /// Never asked: the first request shows the system's prompt.
    case notDetermined
    case denied
    case allowed
}

/// One notification handed to the system: a reminder's (its id is the reminder's) or the
/// one-off "Reminders are on".
public struct PlannedNotification: Equatable, Sendable {
    public let id: String
    public let title: String
    public let body: String
    /// When it fires (`ReminderTrigger`); nil = now.
    public let trigger: DateComponents?
    /// A deadline's course, opened when the student clicks the notification.
    public let courseId: String?

    public init(id: String, title: String, body: String, trigger: DateComponents?, courseId: String? = nil) {
        self.id = id
        self.title = title
        self.body = body
        self.trigger = trigger
        self.courseId = courseId
    }
}

/// What the reminders need from a notification center.
public protocol NotificationCenterClient: AnyObject, Sendable {
    func permission() async -> NotificationPermission
    /// Asks the system (its prompt the first time); true when notifications are allowed.
    func requestPermission() async -> Bool
    /// Identifiers of the requests waiting to fire.
    func pendingIds() async -> [String]
    /// Identifiers of the notifications still in Notification Center.
    func deliveredIds() async -> [String]
    /// Schedules (or, with the same id, replaces) a request.
    func add(_ notification: PlannedNotification) async throws
    func removePending(_ ids: [String]) async
}

/// A notification center in memory: nothing reaches the system. The mock's, the tests' and the
/// snapshots'. `fire(until:)` plays the system firing what is due.
public actor RecordingNotificationCenter: NotificationCenterClient {
    public private(set) var currentPermission: NotificationPermission
    /// What the permission request answers (the student's choice in the system prompt).
    public var answer: Bool
    public private(set) var pending: [String: PlannedNotification] = [:]
    public private(set) var delivered: [String] = []
    /// Every notification shown at once (trigger nil), in order.
    public private(set) var shownNow: [PlannedNotification] = []
    public private(set) var permissionRequests = 0

    public init(permission: NotificationPermission = .notDetermined, answer: Bool = true) {
        currentPermission = permission
        self.answer = answer
    }

    public func permission() async -> NotificationPermission { currentPermission }

    public func requestPermission() async -> Bool {
        permissionRequests += 1
        if currentPermission == .notDetermined { currentPermission = answer ? .allowed : .denied }
        return currentPermission == .allowed
    }

    public func pendingIds() async -> [String] { Array(pending.keys) }

    public func deliveredIds() async -> [String] { delivered }

    public func add(_ notification: PlannedNotification) async throws {
        guard currentPermission == .allowed else { throw NotificationsNotAllowed() }
        if notification.trigger == nil {
            shownNow.append(notification)
            delivered.append(notification.id)
        } else {
            pending[notification.id] = notification
        }
    }

    public func removePending(_ ids: [String]) async {
        for id in ids { pending[id] = nil }
    }

    // MARK: Test controls

    public func setPermission(_ permission: NotificationPermission) {
        currentPermission = permission
    }

    /// The system fires every pending request due by `date` (it moves to Notification Center).
    public func fire(until date: Date) {
        for (id, notification) in pending {
            guard let trigger = notification.trigger, let at = ReminderTrigger.instant(of: trigger), at <= date else { continue }
            pending[id] = nil
            delivered.append(id)
        }
    }

    /// The student cleared Notification Center.
    public func clearDelivered() {
        delivered = []
    }
}

/// The system refused a request: notifications aren't allowed.
public struct NotificationsNotAllowed: Error {}
