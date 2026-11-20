import Foundation
import SheprdRemoteCore

/// One sync of this phone's copy with one computer of the fleet, over
/// `POST /api/v2/fleet/sync`: the phone takes what the computer has and it
/// lacks, checking every operation itself, then hands over what the
/// computer lacks, its own signed operations among them. Any computer the
/// phone reaches will do, so the phone carries operations between
/// computers that cannot reach each other, and needs no one computer.
public enum FleetLogSync {
    /// Rounds one sync takes each way, at most.
    public static let maxRounds = 64

    public struct Outcome: Sendable {
        public let replica: FleetReplica
        /// The computer's heads after the sync.
        public let heads: [String]
        /// How many of this phone's operations the computer refused.
        public let refused: Int
    }

    /// Syncs `log` (nil: no copy yet, which then starts from the
    /// computer's history, from the operation that founded `fleet` only)
    /// with the computer `client` reaches.
    public static func run(
        fleet: String, me: String, log: Data?, with client: FleetClient,
        now: @Sendable () -> Date = { Date() }
    ) async throws -> Outcome {
        var replica = try log.map { try FleetReplica.open(fleet: fleet, me: me, log: $0, at: now()) }
        var heads = try await pull(fleet: fleet, me: me, into: &replica, from: client, now: now, rounds: 1)
        guard var replica else { throw FleetLogError.unfounded }
        var refused = 0
        for _ in 0..<maxRounds {
            let (ops, _) = try replica.missing(for: heads, at: now())
            guard !ops.isEmpty else { break }
            let answer = try await client.sync(ops, heads: own(replica), fleet: fleet)
            try check(answer, fleet: fleet)
            refused += answer.refused
            replica = try replica.applying(answer.ops, at: now())
            // Nothing it took moved its heads: it refused them, or holds
            // them waiting. Sending them again changes nothing.
            if answer.heads == heads { break }
            heads = answer.heads
        }
        var pulled: FleetReplica? = replica
        heads = try await pull(fleet: fleet, me: me, into: &pulled, from: client, now: now, rounds: maxRounds)
        return Outcome(replica: pulled ?? replica, heads: heads, refused: refused)
    }

    /// Takes what the computer has, `rounds` answers at most, stopping once
    /// it has no more or an answer brings nothing new. Answers its heads.
    private static func pull(
        fleet: String, me: String, into replica: inout FleetReplica?, from client: FleetClient,
        now: () -> Date, rounds: Int
    ) async throws -> [String] {
        var heads: [String] = []
        for _ in 0..<rounds {
            let answer = try await client.sync([], heads: replica.map(own) ?? [], fleet: fleet)
            try check(answer, fleet: fleet)
            heads = answer.heads
            let next = try replica.map { try $0.applying(answer.ops, at: now()) }
                ?? FleetReplica.apply(fleet: fleet, me: me, log: nil, ops: answer.ops, at: now())
            replica = next
            // A computer that does not know this phone's heads answers from
            // the start; what it sends again is no progress.
            guard answer.more, next.applied > 0 else { break }
        }
        return heads
    }

    private static func own(_ replica: FleetReplica) -> [String] {
        Array(replica.status.heads.prefix(FleetClient.maxSyncHeads))
    }

    private static func check(_ answer: API.FleetApplied, fleet: String) throws {
        guard answer.fleet.lowercased() == fleet.lowercased() else { throw FleetLogError.otherFleet }
    }
}
