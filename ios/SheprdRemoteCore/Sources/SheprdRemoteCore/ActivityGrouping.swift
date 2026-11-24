import Foundation

/// The phone chooses independently for each paired computer. Reading a
/// missing preference gives new and existing pairings one shared activity.
public enum ActivityGrouping {
    public static func key(for computer: Fingerprint) -> String {
        "multipleLiveActivities." + computer.hex
    }

    /// A start can arrive after this phone ended all its activities. Per-pane
    /// cards have no generation. New shared cards carry the reset UUID.
    public static func isRetiredHostCard(_ pane: String, since epoch: UUID?) -> Bool {
        guard let epoch, pane.hasPrefix("host-activity/") else { return false }
        return !pane.hasPrefix("host-activity/" + epoch.uuidString.lowercased() + "/")
    }

    public static func single(for computer: Fingerprint, in defaults: UserDefaults = .standard) -> Bool {
        !defaults.bool(forKey: key(for: computer))
    }
}
