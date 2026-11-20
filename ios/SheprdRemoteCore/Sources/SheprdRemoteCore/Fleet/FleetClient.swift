import CryptoKit
import Foundation

extension Fleet {
    /// This phone's fleet key, as the app holds it: its public half and a
    /// way to sign. On a phone the key is in the Secure Enclave with user
    /// presence, so `sign` asks for Face ID or the passcode.
    public struct Signer: Sendable {
        /// SEC1 compressed.
        public let publicKey: Data
        /// ECDSA P-256 with SHA-256, `r || s`.
        public let sign: @Sendable (Data) throws -> Data

        public init(publicKey: Data, sign: @escaping @Sendable (Data) throws -> Data) {
            self.publicKey = publicKey
            self.sign = sign
        }

        /// The member id: the SHA-256 of the key.
        public var member: Data { Data(SHA256.hash(data: publicKey)) }
    }

    /// A phone's bundle: its fleet key and relay keys, and nothing else, as
    /// a phone gets the API only, never SSH.
    public static func phoneBundle(key: Data, name: String, relay: RelayKeyPair) throws -> Bundle {
        guard let dial = try? P256.Signing.PrivateKey(rawRepresentation: relay.dialKey),
              let tunnel = try? Curve25519.KeyAgreement.PrivateKey(rawRepresentation: relay.tunnelKey)
        else { throw FleetError.invalid }
        return Bundle(
            key: key, kind: .phone, name: Self.memberName(name),
            dial: dial.publicKey.compressedRepresentation, tunnel: tunnel.publicKey.rawRepresentation,
            ssh: nil, host: nil
        )
    }

    /// A name the daemon takes: no control characters, at most 64 bytes.
    static func memberName(_ name: String) -> String {
        var name = String(name.unicodeScalars.filter { !CharacterSet.controlCharacters.contains($0) })
        while name.utf8.count > maxName { name.removeLast() }
        return name.trimmingCharacters(in: .whitespaces).isEmpty ? "iPhone" : name
    }

    /// An operation for `signer` to sign on what a computer answered.
    public static func draft(on next: API.FleetNext, signer: Data, issuedMs: UInt64, body: Body) throws -> Op {
        guard let fleet = Data(hex: next.fleet) else { throw FleetError.encoding }
        let parents = try next.parents.map { text -> Data in
            guard let parent = Data(hex: text) else { throw FleetError.encoding }
            return parent
        }
        return Op(
            fleet: fleet, signer: signer, seq: next.seq, issuedMs: issuedMs,
            parents: parents.sorted { $0.lexicographicallyPrecedes($1) }, body: body, signature: Data()
        )
    }

    /// An id (member, host, fleet, or operation) from hex; nil for
    /// anything but 32 bytes.
    public static func data(hex: String) -> Data? {
        Data(hex: hex).flatMap { $0.count == 32 ? $0 : nil }
    }

    /// The device id a host gives the key a `Link` names: `fleet-` and the
    /// first 16 bytes of the SHA-256 of the key, uncompressed, in hex
    /// (`fleet::links::device_id`).
    public static func linkDevice(key: Data) -> String? {
        let point: P256.Signing.PublicKey? = switch key.count {
        case 65: try? P256.Signing.PublicKey(x963Representation: key)
        case 33: try? P256.Signing.PublicKey(compressedRepresentation: key)
        default: nil
        }
        guard let point else { return nil }
        return "fleet-" + Data(SHA256.hash(data: point.x963Representation).prefix(16)).hexString
    }
}

extension Fleet.Op {
    /// Signs it with `sign`, which must be the signer's key.
    public mutating func sign(using sign: (Data) throws -> Data) throws {
        signature = try sign(signedBytes())
        guard signature.count == 64 else { throw FleetError.signature }
    }
}

extension Data {
    /// Reads hex in either case; nil for anything else.
    init?(hex: String) {
        guard hex.count.isMultiple(of: 2) else { return nil }
        var data = Data(capacity: hex.count / 2)
        var index = hex.startIndex
        while index < hex.endIndex {
            let next = hex.index(index, offsetBy: 2)
            guard let byte = UInt8(hex[index..<next], radix: 16) else { return nil }
            data.append(byte)
            index = next
        }
        self = data
    }
}

/// Why a fleet call did not do what it was asked.
public enum FleetClientError: Error, Equatable, Sendable {
    /// The computer is in no fleet.
    case noFleet
    /// This phone is not a member there, or not yet.
    case notMember
    /// The computer refused operations this phone signed.
    case refused(Int)
    /// The computer took them without applying them: it waits for history
    /// it lacks. Another computer, or this one later, may apply them.
    case unapplied
}

/// The fleet calls of one member computer, over a transport the app
/// supplies (a pinned session with a bearer token). Every operation is
/// built on what the computer answered, signed by this phone, and sent
/// back: the computer checks it as it checks any member's.
public struct FleetClient: Sendable {
    /// Sends `body` (JSON) with `method` to `path`; answers the response
    /// body, throwing for any status but success.
    public typealias Send = @Sendable (_ method: String, _ path: String, _ body: Data?) async throws -> Data

    private let send: Send

    public init(send: @escaping Send) {
        self.send = send
    }

    /// Calls through the computer's live connection when it has one, asked
    /// afresh for every call (`live`), relay included; through `fallback`
    /// (signing in at its saved addresses) only while it has none. A
    /// saved address may be the network the phone left; the live link is
    /// the path that answers now.
    public static func preferring(live: @escaping @Sendable () async -> Send?, fallback: @escaping Send) -> FleetClient {
        FleetClient { method, path, body in
            let send = await live() ?? fallback
            return try await send(method, path, body)
        }
    }

    /// The fleet, and what `member` signs on next when it is one.
    public func status(member: Data?) async throws -> API.FleetStatus {
        let path = member.map { "/api/v2/fleet?member=\($0.hexString)" } ?? "/api/v2/fleet"
        return try JSONDecoder().decode(API.FleetStatus.self, from: await send("GET", path, nil))
    }

    /// Posts `ops`; answers the computer's heads and what the phone lacks.
    public func sync(_ ops: [Fleet.Op], heads: [String]) async throws -> API.FleetApplied {
        let body = API.FleetSync(heads: heads, ops: try ops.map { Base64URL.encode(try $0.encode()) })
        let data = try await send("POST", "/api/v2/fleet/sync", try JSONEncoder().encode(body))
        return try JSONDecoder().decode(API.FleetApplied.self, from: data)
    }

    /// Operations one sync may carry: the daemon refuses more.
    public static let maxSyncOps = 64

    /// Operations this phone signed, ready to post, and the heads of the
    /// computer they were drafted on.
    public struct Signed: Equatable, Sendable {
        public let ops: [Fleet.Op]
        public let heads: [String]

        public init(ops: [Fleet.Op], heads: [String]) {
            self.ops = ops
            self.heads = heads
        }
    }

    /// Heads one sync may name: the daemon refuses more. They only say
    /// what the phone knows, so fewer only make the answer longer.
    public static let maxSyncHeads = 64

    /// Signs one operation per body, each on the one before, on what this
    /// computer answers. Nothing is posted: a post whose answer is lost is
    /// sent again as these same bytes, to this computer or another, never
    /// signed afresh, as two different operations with one sequence number
    /// would look like this phone's key in use in two places.
    ///
    /// `unconfirmed` are operations this phone signed before whose
    /// application no computer confirmed yet. While this computer has not
    /// applied them, the new ones follow them (and travel with them), so
    /// they never share a sequence number.
    public func prepare(
        _ bodies: [Fleet.Body], as signer: Fleet.Signer, at now: Date, after unconfirmed: [Fleet.Op] = []
    ) async throws -> Signed {
        let status = try await status(member: signer.member)
        guard status.view != nil else { throw FleetClientError.noFleet }
        guard let next = status.next else { throw FleetClientError.notMember }
        let issued = UInt64(max(0, now.timeIntervalSince1970 * 1000))
        var on = next
        var ops: [Fleet.Op] = []
        if let last = unconfirmed.last, next.seq <= last.seq {
            ops = unconfirmed
            on = API.FleetNext(fleet: next.fleet, seq: last.seq + 1, parents: [try last.id().hexString])
        }
        for body in bodies {
            var op = try Fleet.draft(on: on, signer: signer.member, issuedMs: issued, body: body)
            try op.sign(using: signer.sign)
            ops.append(op)
            on = API.FleetNext(fleet: next.fleet, seq: op.seq + 1, parents: [try op.id().hexString])
        }
        guard !ops.isEmpty else { throw FleetError.invalid }
        return Signed(ops: ops, heads: status.heads)
    }

    /// Posts `signed` at most `maxSyncOps` at a time, each batch on the
    /// heads the last one left (at most `maxSyncHeads`), then reads the
    /// fleet back to confirm they applied: a computer that lacks their
    /// history takes them without a refusal and only holds them waiting,
    /// which `unapplied` reports. Throws when the computer refused any;
    /// batches before it stand. Posting the same operations again is
    /// harmless: a computer that holds one already takes it as known.
    @discardableResult
    public func post(_ signed: Signed) async throws -> API.FleetApplied {
        var heads = signed.heads
        var applied: API.FleetApplied?
        for start in stride(from: 0, to: signed.ops.count, by: Self.maxSyncOps) {
            let batch = Array(signed.ops[start..<min(start + Self.maxSyncOps, signed.ops.count)])
            let answer = try await sync(batch, heads: Array(heads.prefix(Self.maxSyncHeads)))
            if answer.refused > 0 { throw FleetClientError.refused(answer.refused) }
            heads = answer.heads
            applied = answer
        }
        guard let applied, let last = signed.ops.last else { throw FleetError.invalid }
        // The member's next sequence number moves past an operation only
        // once it applies (the daemon drafts on what it applied).
        let after = try await status(member: last.signer)
        guard let next = after.next, next.seq > last.seq else { throw FleetClientError.unapplied }
        return applied
    }

    /// Signs one operation per body and posts them (`prepare`, then
    /// `post`).
    @discardableResult
    public func sign(_ bodies: [Fleet.Body], as signer: Fleet.Signer, at now: Date) async throws -> API.FleetApplied {
        try await post(prepare(bodies, as: signer, at: now))
    }

    /// Asks to join the computer's fleet with `bundle`, signed with
    /// `signer` for `fleet`; a person admits it in Sheprd.
    public func join(_ bundle: Fleet.Bundle, fleet: String, as signer: Fleet.Signer) async throws -> API.FleetJoined {
        guard let id = Data(hex: fleet), id.count == 32 else { throw FleetError.encoding }
        let signature = try signer.sign(try bundle.joinMessage(fleet: id))
        var writer = FleetWriter()
        try bundle.write(&writer)
        let request = API.FleetJoinRequest(bundle: Base64URL.encode(writer.bytes), signature: Base64URL.encode(signature))
        let data = try await send("POST", "/api/v2/fleet/join", try JSONEncoder().encode(request))
        return try JSONDecoder().decode(API.FleetJoined.self, from: data)
    }
}
