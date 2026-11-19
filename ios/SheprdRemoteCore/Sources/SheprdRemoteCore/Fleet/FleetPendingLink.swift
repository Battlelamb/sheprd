import Foundation

/// A device key this phone made for one fleet host and is linking itself
/// to it with: kept in the Keychain from before the `Link` is posted, so a
/// reply that never arrives cannot lose the key of a link that landed.
public struct FleetPendingLink: Codable, Equatable, Sendable {
    /// The host's id, hex.
    public let host: String
    public let name: String
    public let keyKind: Computer.KeyKind
    /// The device key, as a pairing record keeps it.
    public let keyData: Data
    /// Its public half, as a claim encodes it.
    public let publicKey: String
    /// Its public half, SEC1 uncompressed, as the `Link` names it.
    public let point: Data

    public init(host: String, name: String, keyKind: Computer.KeyKind, keyData: Data, publicKey: String, point: Data) {
        self.host = host
        self.name = name
        self.keyKind = keyKind
        self.keyData = keyData
        self.publicKey = publicKey
        self.point = point
    }

    public var key: HostKey { HostKey(publicKey: publicKey, kind: keyKind, data: keyData) }

    /// How a post of links ended.
    public enum Outcome: Sendable {
        /// The computer answered: what it holds is all that landed.
        case answered
        /// No answer: links may have landed, so their keys stay until the
        /// fleet says.
        case unknown
    }

    /// How a post that a computer took ended, by whether the fleet could be
    /// read again after it: only a fresh read says which links landed, so
    /// without one the keys of all of them stay.
    public static func outcome(freshRead: Bool) -> Outcome {
        freshRead ? .answered : .unknown
    }

    /// Splits `pending` against the hosts the fleet says this phone linked:
    /// those linked are done, to keep as computers; the rest stay waiting,
    /// unless the computer answered, when they never landed and go.
    public static func settle(
        _ pending: [FleetPendingLink], linked: [String], outcome: Outcome
    ) -> (done: [FleetPendingLink], waiting: [FleetPendingLink]) {
        let linked = Set(linked.map { $0.lowercased() })
        let done = pending.filter { linked.contains($0.host.lowercased()) }
        let rest = pending.filter { !linked.contains($0.host.lowercased()) }
        return (done, outcome == .answered ? [] : rest)
    }

    /// `pending` with `fresh` in place of any for the same hosts: a key made
    /// again for a host replaces the one before, which never landed or
    /// whose link the new one supersedes.
    public static func merge(_ pending: [FleetPendingLink], _ fresh: [FleetPendingLink]) -> [FleetPendingLink] {
        let hosts = Set(fresh.map { $0.host.lowercased() })
        return pending.filter { !hosts.contains($0.host.lowercased()) } + fresh
    }
}
