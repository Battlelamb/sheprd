import Foundation

/// The phone chooses independently for each paired computer. Reading a
/// missing preference gives new and existing pairings one shared activity.
public enum ActivityGrouping {
    public static func key(for computer: Fingerprint) -> String {
        "multipleLiveActivities." + computer.hex
    }

    public static func single(for computer: Fingerprint, in defaults: UserDefaults = .standard) -> Bool {
        !defaults.bool(forKey: key(for: computer))
    }
}
