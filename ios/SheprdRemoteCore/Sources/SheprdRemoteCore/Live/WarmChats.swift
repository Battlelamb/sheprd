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

    /// When each chat left lingers until.
    private var left: [Key: Date] = [:]

    public init() {}

    /// The last screen showing `key` closed at `now`.
    public mutating func closed(_ key: Key, at now: Date) {
        left[key] = now.addingTimeInterval(Self.lingers)
    }

    /// The chats to follow at `now` among those `listed`: the ones left
    /// that still linger, while listed, and the busiest and most recently
    /// answered. A chat no longer listed, as on a forgotten computer, is
    /// not followed.
    public mutating func keys(_ listed: [Listed], at now: Date) -> Set<Key> {
        left = left.filter { $0.value > now }
        let known = Set(listed.map(\.key))
        left = left.filter { known.contains($0.key) }
        let picked = listed.sorted { a, b in
            if a.busy != b.busy { return a.busy }
            return (a.lastAnswer ?? .distantPast) > (b.lastAnswer ?? .distantPast)
        }
        .prefix(Self.listed)
        .map(\.key)
        return Set(picked).union(left.keys)
    }
}
