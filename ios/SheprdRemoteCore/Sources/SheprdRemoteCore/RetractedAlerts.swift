import Foundation

/// Copies of alerts to take back. A copy of an alert the person already
/// dismissed or opened can only be shown again, silently: without Apple's
/// filtering entitlement the Notification Service Extension must deliver
/// something. It notes the copy here, and the extension's next run and the
/// app (launch, foreground, background push) remove it. A copy is removed
/// only while the notification under its identifier still carries its
/// event: a newer alert for the same chat reuses the identifier (the APNs
/// collapse id) and stays. Kept in the App Group's defaults, at most
/// `kept`, none older than `window`.
public struct RetractedAlerts: Equatable, Sendable {
    public static let kept = 32
    public static let window: TimeInterval = 2 * 24 * 60 * 60
    static let key = "retractedAlerts"

    struct Entry: Equatable, Sendable {
        let identifier: String
        let event: String
        let at: TimeInterval
    }

    private(set) var entries: [Entry] = []

    public init() {}

    /// Notes the copy shown as `identifier` for `event` at `date`.
    public mutating func add(_ identifier: String, event: String, at date: Date) {
        entries.removeAll { $0.identifier == identifier }
        entries.append(Entry(identifier: identifier, event: event, at: date.timeIntervalSince1970))
        if entries.count > Self.kept { entries.removeFirst(entries.count - Self.kept) }
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

    /// `add` in the App Group's `defaults`.
    public static func add(_ identifier: String, event: String, at date: Date, in defaults: UserDefaults?) {
        guard let defaults else { return }
        var retracted = load(from: defaults)
        retracted.add(identifier, event: event, at: date)
        retracted.save(to: defaults)
    }

    /// `take` in the App Group's `defaults`.
    public static func take(shown: [String: String?], at date: Date, in defaults: UserDefaults?) -> [String] {
        guard let defaults else { return [] }
        var retracted = load(from: defaults)
        guard !retracted.entries.isEmpty else { return [] }
        let remove = retracted.take(shown: shown, at: date)
        retracted.save(to: defaults)
        return remove
    }

    static func load(from defaults: UserDefaults) -> RetractedAlerts {
        var retracted = RetractedAlerts()
        let flat = defaults.stringArray(forKey: key) ?? []
        for index in stride(from: 0, to: flat.count - 2, by: 3) {
            guard let at = TimeInterval(flat[index + 2]) else { continue }
            retracted.entries.append(Entry(identifier: flat[index], event: flat[index + 1], at: at))
        }
        retracted.entries = Array(retracted.entries.suffix(kept))
        return retracted
    }

    func save(to defaults: UserDefaults) {
        defaults.set(entries.flatMap { [$0.identifier, $0.event, String($0.at)] }, forKey: Self.key)
    }
}
