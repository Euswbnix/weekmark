// `UNUserNotificationCenter` behind `NotificationCenterClient`. Only inside the app bundle: the
// center traps in a process without a bundle identifier (swift test, the snapshot tool).

import Foundation
import UserNotifications

public final class SystemNotificationCenter: NotificationCenterClient, @unchecked Sendable {
    /// The key of a deadline's course in a notification's `userInfo`.
    public static let courseIdKey = "courseId"

    // UNUserNotificationCenter is thread-safe; the class holds no other state.
    private let center: UNUserNotificationCenter

    /// Whether this process may use the system's notification center (it runs as an .app).
    public static var isAvailable: Bool {
        Bundle.main.bundleIdentifier != nil && Bundle.main.bundleURL.pathExtension == "app"
    }

    /// The system center; nil when `isAvailable` is false.
    public static func shared() -> SystemNotificationCenter? {
        isAvailable ? SystemNotificationCenter(center: .current()) : nil
    }

    /// Makes `delegate` the system center's (banners while frontmost, clicks).
    static func install(delegate: any UNUserNotificationCenterDelegate) {
        guard isAvailable else { return }
        UNUserNotificationCenter.current().delegate = delegate
    }

    private init(center: UNUserNotificationCenter) {
        self.center = center
    }

    public func permission() async -> NotificationPermission {
        switch await center.notificationSettings().authorizationStatus {
        case .notDetermined: .notDetermined
        case .denied: .denied
        default: .allowed
        }
    }

    public func requestPermission() async -> Bool {
        (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
    }

    public func pendingIds() async -> [String] {
        await center.pendingNotificationRequests().map(\.identifier)
    }

    public func deliveredIds() async -> [String] {
        await center.deliveredNotifications().map(\.request.identifier)
    }

    public func add(_ notification: PlannedNotification) async throws {
        let content = UNMutableNotificationContent()
        content.title = notification.title
        content.body = notification.body
        content.sound = .default
        content.threadIdentifier = "pagelamp.reminders"
        if let courseId = notification.courseId {
            content.userInfo = [Self.courseIdKey: courseId]
        }
        let trigger = notification.trigger.map { UNCalendarNotificationTrigger(dateMatching: $0, repeats: false) }
        try await center.add(UNNotificationRequest(identifier: notification.id, content: content, trigger: trigger))
    }

    public func removePending(_ ids: [String]) async {
        center.removePendingNotificationRequests(withIdentifiers: ids)
    }
}
