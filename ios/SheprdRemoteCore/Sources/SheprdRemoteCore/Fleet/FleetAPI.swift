import Foundation

/// The fleet calls a phone makes on a member computer's daemon
/// (`crates/sheprd-remote-control/src/web/fleet.rs`). Operations travel
/// as their canonical bytes in base64url; the JSON around them is a view,
/// never what is signed.
extension API {
    /// `GET /api/v2/fleet?member=ID`.
    public struct FleetStatus: Decodable, Equatable, Sendable {
        /// Nil while the computer is in no fleet.
        public let view: FleetView?
        public let heads: [String]
        /// What this phone signs its next operation on, once it is a member.
        public let next: FleetNext?
        /// Where this phone's join stands, when it asked here.
        public let join: FleetStanding?
        /// The hosts this phone linked a device key for, by id.
        public let links: [String]
        /// Where hosts take direct connections, by host id.
        public let reach: [FleetReach]
        /// How to reach each Sheprd host this phone linked through its
        /// relays, from the host's own signed record.
        public let routes: [FleetRoute]

        public init(
            view: FleetView?, heads: [String] = [], next: FleetNext? = nil,
            join: FleetStanding? = nil, links: [String] = [], reach: [FleetReach] = [],
            routes: [FleetRoute] = []
        ) {
            self.view = view
            self.heads = heads
            self.next = next
            self.join = join
            self.links = links
            self.reach = reach
            self.routes = routes
        }

        enum CodingKeys: String, CodingKey { case view, heads, next, join, links, reach, routes }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            view = try c.decodeIfPresent(FleetView.self, forKey: .view)
            heads = try c.decodeIfPresent([String].self, forKey: .heads) ?? []
            next = try c.decodeIfPresent(FleetNext.self, forKey: .next)
            join = try c.decodeIfPresent(FleetStanding.self, forKey: .join)
            links = try c.decodeIfPresent([String].self, forKey: .links) ?? []
            reach = try c.decodeIfPresent([FleetReach].self, forKey: .reach) ?? []
            routes = try c.decodeIfPresent([FleetRoute].self, forKey: .routes) ?? []
        }

        /// How this phone reaches `host` through its relays with `keys`, its
        /// fleet bundle's: nil unless the host is linked, its route is
        /// pinned to the host's own certificate authority, and it names
        /// relays.
        public func relayRecord(for host: String, keys: RelayKeyPair) -> RelayRecord? {
            guard let route = routes.first(where: { $0.host.lowercased() == host.lowercased() }),
                  route.fingerprint.lowercased() == host.lowercased()
            else { return nil }
            return RelayRecord.make(from: route.relay, keys: keys)
        }

        /// The direct addresses `host` publishes, as `https://` origins.
        public func origins(of host: String) -> [String] {
            (reach.first { $0.host == host }?.direct ?? [])
                .map { "https://\($0)" }
                .filter(Computer.isOrigin)
        }
    }

    /// The fleet as one member computer sees it.
    public struct FleetView: Decodable, Equatable, Sendable {
        public let fleet: String
        /// The computer's own member id.
        public let me: String
        public let members: [FleetMember]
        public let hosts: [FleetHost]
        /// Members removed, ever.
        public let removed: Int
        /// Operations waiting for their parents.
        public let pending: Int
    }

    public struct FleetMember: Decodable, Equatable, Identifiable, Sendable {
        public let id: String
        public let name: String
        public let kind: Kind
        public let admittedBy: String?

        public enum Kind: String, Decodable, Sendable { case computer, phone }

        enum CodingKeys: String, CodingKey {
            case id, name, kind
            case admittedBy = "admitted_by"
        }
    }

    public struct FleetHost: Decodable, Equatable, Identifiable, Sendable {
        public let id: String
        public let name: String
        public let kind: Kind
        public let user: String
        public let addedBy: String
        public let relays: [String]
        public let held: Bool

        /// What runs there: a computer member, a Sheprd host, or a plain
        /// host with the relay agent, which a phone never reaches.
        public enum Kind: String, Decodable, Sendable { case member, sheprd, agent }

        /// A host whose daemon a phone can control.
        public var runsSheprd: Bool { kind != .agent }

        enum CodingKeys: String, CodingKey {
            case id, name, kind, user, relays, held
            case addedBy = "added_by"
        }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            id = try c.decode(String.self, forKey: .id)
            name = try c.decode(String.self, forKey: .name)
            kind = try c.decode(Kind.self, forKey: .kind)
            user = try c.decode(String.self, forKey: .user)
            addedBy = try c.decode(String.self, forKey: .addedBy)
            relays = try c.decodeIfPresent([String].self, forKey: .relays) ?? []
            held = try c.decodeIfPresent(Bool.self, forKey: .held) ?? false
        }
    }

    public struct FleetNext: Decodable, Equatable, Sendable {
        public let fleet: String
        public let seq: UInt64
        public let parents: [String]

        public init(fleet: String, seq: UInt64, parents: [String]) {
            self.fleet = fleet
            self.seq = seq
            self.parents = parents
        }
    }

    public enum FleetStanding: String, Decodable, Sendable { case pending, member }

    /// A linked host's relays, as `GET /api/relay` answers on it.
    public struct FleetRoute: Decodable, Equatable, Sendable {
        public let host: String
        /// The SHA-256 of the host's certificate authority: its id, and
        /// what its record is signed by.
        public let fingerprint: String
        public let relay: RelayInfo
    }

    public struct FleetReach: Decodable, Equatable, Sendable {
        public let host: String
        public let direct: [String]
    }

    /// `POST /api/v2/fleet/sync`.
    public struct FleetSync: Encodable, Equatable, Sendable {
        public let heads: [String]
        public let ops: [String]
    }

    /// What a sync came to: the computer's heads, and what the phone lacks.
    public struct FleetApplied: Decodable, Equatable, Sendable {
        public let fleet: String
        public let heads: [String]
        public let ops: [String]
        public let refused: Int
        public let more: Bool

        enum CodingKeys: String, CodingKey { case fleet, heads, ops, refused, more }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            fleet = try c.decode(String.self, forKey: .fleet)
            heads = try c.decode([String].self, forKey: .heads)
            ops = try c.decodeIfPresent([String].self, forKey: .ops) ?? []
            refused = try c.decodeIfPresent(Int.self, forKey: .refused) ?? 0
            more = try c.decodeIfPresent(Bool.self, forKey: .more) ?? false
        }
    }

    /// `POST /api/v2/fleet/join`.
    public struct FleetJoinRequest: Encodable, Equatable, Sendable {
        public let bundle: String
        public let signature: String
    }

    public struct FleetJoined: Decodable, Equatable, Sendable {
        public let member: String
        public let state: FleetStanding
    }
}
