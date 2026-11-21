import Foundation

/// Which chats the phone keeps following while no screen shows them, so
/// opening one shows its latest turn at once rather than after a round
/// trip to the computer: a chat just left, for a while, and the few the
/// lists show busiest or most recently answered.
public struct WarmChats<Key: Hashable & Sendable>: Sendable {
    /// One the lists offer: whether its agent works or waits on the
    /// person, and when it last answered.
    public struct Listed: Sendable {
        public let key: Key
        public let busy: Bool
        public let lastAnswer: Date?

        public init(key: Key, busy: Bool, lastAnswer: Date?) {
            self.key = key
            self.busy = busy
            self.lastAnswer = lastAnswer
        }
    }

    /// How long a chat left stays followed, for a quick look elsewhere.
    public static var lingers: TimeInterval { 60 }
    /// How many listed chats are followed; each streams its changes.
    public static var listed: Int { 4 }
    /// The views one live socket follows at most (`MAX_VIEWS` in the
    /// daemon's `native/live.rs`): home, the screens, and these.
    public static var viewsPerLink: Int { 8 }

    /// The background chats a computer has room for while `shown` views
    /// (chats and terminals) are on screens: what its socket follows
    /// besides home and those, less one kept for the next screen to open,
    /// so opening one is never refused for these.
    public static func room(shown: Int) -> Int {
        max(0, viewsPerLink - 1 - shown - 1)
    }

    /// When each chat left lingers until.
    private var left: [Key: Date] = [:]

    public init() {}

    /// The last screen showing `key` closed at `now`.
    public mutating func closed(_ key: Key, at now: Date) {
        left[key] = now.addingTimeInterval(Self.lingers)
    }

    /// The chats to follow at `now` among those `listed`: the ones left
    /// that still linger, most recently left first, then the busiest and
    /// most recently answered, each while its computer (`group`) has
    /// `room` for it. A chat no longer listed, as on a forgotten
    /// computer, is not followed.
    public mutating func keys<Group: Hashable>(
        _ listed: [Listed],
        at now: Date,
        room: [Group: Int],
        in group: (Key) -> Group
    ) -> Set<Key> {
        let known = Set(listed.map(\.key))
        left = left.filter { $0.value > now && known.contains($0.key) }
        let lingering = left.sorted { $0.value > $1.value }.map(\.key)
        let picked = listed.sorted { a, b in
            if a.busy != b.busy { return a.busy }
            return (a.lastAnswer ?? .distantPast) > (b.lastAnswer ?? .distantPast)
        }
        .prefix(Self.listed)
        .map(\.key)
        var room = room
        var kept = Set<Key>()
        for key in lingering + picked where !kept.contains(key) {
            let free = room[group(key), default: 0]
            guard free > 0 else { continue }
            room[group(key)] = free - 1
            kept.insert(key)
        }
        return kept
    }
}
