// The system notification center's delegate (model-access design §5.3): a reminder that fires
// while PageLamp is frontmost shows as a banner and counts as shown; one the student clicks
// counts as shown and opens its course (a deadline) or This Week.

import Foundation
@preconcurrency import UserNotifications

@MainActor
final class ReminderResponder: NSObject, UNUserNotificationCenterDelegate {
    private weak var model: AppModel?

    init(model: AppModel) {
        self.model = model
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        let id = notification.request.identifier
        await model?.reminderDelivery?.shown(id: id)
        return [.banner, .list, .sound]
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let id = response.notification.request.identifier
        let courseId = response.notification.request.content.userInfo[SystemNotificationCenter.courseIdKey] as? String
        await opened(id: id, courseId: courseId)
    }

    private func opened(id: String, courseId: String?) async {
        guard let model else { return }
        model.openFromReminder(courseId: courseId)
        await model.reminderDelivery?.shown(id: id)
    }
}
