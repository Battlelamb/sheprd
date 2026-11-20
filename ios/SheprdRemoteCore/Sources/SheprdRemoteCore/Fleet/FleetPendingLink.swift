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

    /// Splits `pending` against the hosts the fleet says this phone linked:
    /// those linked are done, to keep as computers. Of the rest, only those
    /// in `refused` go: hosts whose links were never posted, or that the
    /// computer that took the post refused, with a fresh read from that
    /// same computer not showing them. A link missing from any other read
    /// may only not have arrived there yet (a computer that lags holds it
    /// waiting for its parents), so its key stays.
    public static func settle(
        _ pending: [FleetPendingLink], linked: [String], refused: [String] = []
    ) -> (done: [FleetPendingLink], waiting: [FleetPendingLink]) {
        let linked = Set(linked.map { $0.lowercased() })
        let refused = Set(refused.map { $0.lowercased() })
        let done = pending.filter { linked.contains($0.host.lowercased()) }
        let waiting = pending.filter {
            !linked.contains($0.host.lowercased()) && !refused.contains($0.host.lowercased())
        }
        return (done, waiting)
    }

    /// `pending` with `fresh` in place of any for the same hosts: a key made
    /// again for a host replaces the one before, which never landed or
    /// whose link the new one supersedes.
    public static func merge(_ pending: [FleetPendingLink], _ fresh: [FleetPendingLink]) -> [FleetPendingLink] {
        let hosts = Set(fresh.map { $0.host.lowercased() })
        return pending.filter { !hosts.contains($0.host.lowercased()) } + fresh
    }
}
