import Foundation
import SheprdRemoteCore
import UserNotifications

/// Opens a sealed alert (`SealedAlert`) with the key the app registered,
/// so APNs only ever carried "Sheprd" and ciphertext, then shows the chat's
/// title and text, groups it with the chat's other notifications, and
/// gives it the actions its question or finished turn offers.
///
/// A copy of an alert it already showed (the same sealed `event`: a push
/// delivered twice, or sent again another way) shows once (`AlertGate`).
/// With Apple's notification filtering entitlement, which a build marks by
/// `SheprdFiltersRepeats` in its Info.plist, the copy is dropped; without
/// it an extension must deliver something, so the copy takes the first
/// one's place silently, and the phone shows and buzzes once.
final class NotificationService: UNNotificationServiceExtension {
    private static let group = "group.so.pen.herdr.remote"
    /// One per extension process, which iOS may hand several pushes at once.
    private static let gate = AlertGate { event in
        SeenAlerts.firstSighting(event, at: Date(), in: UserDefaults(suiteName: group))
    }
    /// Whether this build has the filtering entitlement: set with it in
    /// `ios/project.yml`, as Apple grants it.
    private static let filtersRepeats = Bundle.main.object(forInfoDictionaryKey: "SheprdFiltersRepeats") as? String == "YES"

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        let deliver = UncheckedSendable(value: contentHandler)
        let request = UncheckedSendable(value: request)
        Task {
            let (content, first) = await Self.prepare(request.value)
            deliver.value(content)
            // Copies waiting on this one go on only once it reached iOS.
            if let first { await Self.gate.delivered(first) }
        }
    }

    /// What `request` shows, and the event it is the first copy of, which
    /// must be marked delivered once the content was handed to iOS.
    private static func prepare(_ request: UNNotificationRequest) async -> (UNNotificationContent, String?) {
        let content = (request.content.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
        let sealed = request.content.userInfo["sealed"] as? String
        // A computer's key first: what one opens is that computer's alert.
        // Only what none opens may be this phone's own test push.
        let opened = sealed.flatMap { try? NotifyKey.open($0) }
        if opened == nil, let sealed, let test = try? NotifyKey.openSelfTest(sealed) {
            // Its id is its event: each test is a decision of its own.
            return await admitted(selfTest(test), event: test.id)
        }
        guard let opened else {
            // Not from a paired computer, or one this phone forgot. Only
            // the cover is shown, whatever the push said, with none of its
            // actions: opening the app shows what is really waiting.
            return (cover(), nil)
        }
        let alert = opened.alert
        content.title = alert.title
        content.body = alert.body
        content.threadIdentifier = AlertKeys.thread(computer: opened.computer.hex, pane: alert.pane)
        // The count is this computer's; the badge is every computer's.
        if let badge = alert.badge {
            let total = BadgeCounts.update(badge, for: alert.computer, in: UserDefaults(suiteName: group))
            content.badge = NSNumber(value: total)
        }
        var info = content.userInfo
        info["sealed"] = nil
        if let test = alert.test {
            // A test from Settings: the app waits for this receipt, which a
            // repeat records too. It names no chat, so a tap only opens the app.
            PushTestReceipts.record(test, at: Date(), in: UserDefaults(suiteName: group))
            content.userInfo = info
            return await admitted(content, event: alert.event)
        }
        // The computer whose key opened it, which actions then reach.
        info[AlertKeys.computer] = opened.computer.hex
        info[AlertKeys.pane] = alert.pane
        if let permission = alert.permission { info[AlertKeys.permission] = NSNumber(value: permission.id.value) }
        if alert.kind.isTerminal { info[AlertKeys.terminal] = true }
        content.userInfo = info
        if alert.wizz {
            content.interruptionLevel = .timeSensitive
            if let sound = WizzSound.named() { content.sound = sound }
        }
        let presentation = alert.presentation
        if let category = presentation.category {
            content.categoryIdentifier = category
            await NotificationCategories.register(presentation)
        }
        return await admitted(content, event: alert.event)
    }

    /// `content` as the first copy of `event` shows it, or as a repeat
    /// does: dropped where the build may filter, else in the first one's
    /// place without a sound. An alert without an event, from an older
    /// computer, always shows.
    private static func admitted(_ content: UNNotificationContent, event: String?) async -> (UNNotificationContent, String?) {
        guard let event else { return (content, nil) }
        let content = (content.mutableCopy() as? UNMutableNotificationContent) ?? UNMutableNotificationContent()
        content.userInfo[AlertKeys.event] = event
        guard await gate.admit(event) == .repeated else { return (content, event) }
        if filtersRepeats { return (UNNotificationContent(), nil) }
        await removeShown(event)
        content.sound = nil
        content.interruptionLevel = .passive
        return (content, nil)
    }

    /// Removes the notifications already shown for `event`, which the copy
    /// arriving now replaces.
    private static func removeShown(_ event: String) async {
        let center = UNUserNotificationCenter.current()
        let shown = await center.deliveredNotifications()
        let ids = shown
            .filter { $0.request.content.userInfo[AlertKeys.event] as? String == event }
            .map(\.request.identifier)
        if !ids.isEmpty { center.removeDeliveredNotifications(withIdentifiers: ids) }
    }

    /// A test push this phone sent itself through the push relay: its
    /// receipt for Settings, and its text, built fresh like the cover, so
    /// it has no computer, chat, badge, or actions; a tap only opens the app.
    private static func selfTest(_ test: SelfTestPush.Opened) -> UNNotificationContent {
        PushTestReceipts.record(test.id, at: Date(), in: UserDefaults(suiteName: group))
        let content = UNMutableNotificationContent()
        content.title = test.title
        content.body = test.body
        content.threadIdentifier = SelfTestPush.pane
        // The payload asks for the default sound, as a computer's test does.
        content.sound = .default
        return content
    }

    /// What every sealed alert says on the way here, built fresh so nothing
    /// of the push survives: text, actions, badge, or data an action reads.
    private static func cover() -> UNNotificationContent {
        let content = UNMutableNotificationContent()
        content.title = "Sheprd"
        content.body = "New activity"
        return content
    }
}

/// Lets a value iOS hands the extension cross into the task that finishes
/// it; the handler is called once, after the content is prepared.
private struct UncheckedSendable<Value>: @unchecked Sendable {
    let value: Value
}

/// The buzz a wizz alert plays: the chat effect's own sound
/// (`ScreenEffect.wizz.cue`), written once where notification sounds are
/// looked for, the app group's `Library/Sounds`.
enum WizzSound {
    /// Named by version: a changed buzz is written afresh rather than read stale.
    static let file = "wizz-1.wav"

    static func named() -> UNNotificationSound? {
        guard let group = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: "group.so.pen.herdr.remote") else { return nil }
        let folder = group.appending(path: "Library/Sounds", directoryHint: .isDirectory)
        let url = folder.appending(path: file)
        if !FileManager.default.fileExists(atPath: url.path(percentEncoded: false)) {
            do {
                try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
                try ScreenEffect.wizz.cue().sound.wav().write(to: url, options: .atomic)
            } catch {
                return nil
            }
        }
        return UNNotificationSound(named: UNNotificationSoundName(file))
    }
}
