import Foundation

/// Copies of alerts to take back. A copy of an alert the person already
/// dismissed or opened can only be shown again, silently: without Apple's
/// filtering entitlement the Notification Service Extension must deliver
/// something. It notes the copy here, and the extension's next run and the
/// app (launch, activation, background push) remove it. A copy is removed
/// only while the notification under its identifier still carries its
/// event: a newer alert for the same chat reuses the identifier (the APNs
/// collapse id) and stays. At most `kept`, none older than `window`.
///
/// The app and its extension are separate processes that both change the
/// list, so it lives in a file in the App Group and every change is one
/// coordinated read, change, and write (`update`), never two.
public struct RetractedAlerts: Codable, Equatable, Sendable {
    public static let kept = 32
    public static let window: TimeInterval = 2 * 24 * 60 * 60
    /// The file's name in the App Group container.
    public static let fileName = "retracted-alerts.json"

    struct Entry: Codable, Equatable, Sendable {
        let identifier: String
        let event: String
        let at: TimeInterval
    }

    private(set) var entries: [Entry] = []

    public init() {}

    public var isEmpty: Bool { entries.isEmpty }

    /// Notes the copy shown as `identifier` for `event` at `date`.
    public mutating func add(_ identifier: String, event: String, at date: Date) {
        entries.removeAll { $0.identifier == identifier }
        entries.append(Entry(identifier: identifier, event: event, at: date.timeIntervalSince1970))
        if entries.count > Self.kept { entries.removeFirst(entries.count - Self.kept) }
    }

    /// Forgets any copy noted under `identifier`: another alert is about
    /// to be shown there, which no sweep may take back.
    public mutating func forget(_ identifier: String) {
        entries.removeAll { $0.identifier == identifier }
    }

    /// The identifiers to remove now, given what is shown (`identifier` to
    /// its event, if it has one) at `date`. A copy not shown yet waits for
    /// a later sweep; one whose identifier shows another event was replaced
    /// by a newer alert and is forgotten, as is one past `window`.
    public mutating func take(shown: [String: String?], at date: Date) -> [String] {
        let now = date.timeIntervalSince1970
        var remove: [String] = []
        entries.removeAll { entry in
            if now - entry.at > Self.window { return true }
            guard let event = shown[entry.identifier] else { return false }
            if event == entry.event { remove.append(entry.identifier) }
            return true
        }
        return remove
    }

    /// Reads the list in `file`, lets `change` change it, and writes it
    /// back, all while holding the file against every other coordinated
    /// reader and writer, in this process or the other. A file that is
    /// missing or unreadable reads as empty. Blocking: `change` may wait,
    /// briefly, for what iOS shows.
    @discardableResult
    public static func update<Result>(file: URL, _ change: (inout RetractedAlerts) -> Result) -> Result {
        var outcome: Result?
        var failure: NSError?
        NSFileCoordinator().coordinate(writingItemAt: file, options: .forMerging, error: &failure) { url in
            var list = (try? JSONDecoder().decode(Self.self, from: Data(contentsOf: url))) ?? Self()
            let before = list
            outcome = change(&list)
            if list != before, let data = try? JSONEncoder().encode(list) {
                try? data.write(to: url, options: .atomic)
            }
        }
        // Coordination fails only for a file it cannot reach at all; the
        // change then runs on an empty list that is not kept.
        if let outcome { return outcome }
        var list = Self()
        return change(&list)
    }
}
