import Foundation
import SheprdRemoteCore
import UserNotifications

/// Takes back the alert copies `RetractedAlerts` holds: copies of alerts
/// the person had already dismissed or opened, which the extension could
/// only show again silently. The extension runs it as each push arrives
/// and a moment after showing one; the app at launch, in the foreground,
/// and on a background push.
enum AlertSweep {
    static func run() async {
        let center = UNUserNotificationCenter.current()
        let shown = await center.deliveredNotifications()
        var events: [String: String?] = [:]
        for note in shown {
            events[note.request.identifier] = note.request.content.userInfo[AlertKeys.event] as? String
        }
        let remove = RetractedAlerts.take(shown: events, at: Date(), in: UserDefaults(suiteName: NotifyKey.group))
        if !remove.isEmpty { center.removeDeliveredNotifications(withIdentifiers: remove) }
    }
}
