import Foundation
import Synchronization
import Testing
@testable import SheprdFleetLog
import SheprdRemoteCore

/// Syncing the phone's copy with computers of the fleet. Each fake computer
/// keeps its log with the same library, so it refuses what a daemon would.
struct FleetLogSyncTests {
    typealias Vector = FleetReplicaTests.Vector

    /// A computer that answers `POST /api/v2/fleet/sync` from its own log.
    final class Computer: Sendable {
        let replica: Mutex<FleetReplica>
        let posts = Mutex(0)
        let offline = Mutex(false)

        init(_ replica: FleetReplica) {
            self.replica = Mutex(replica)
        }

        var client: FleetClient {
            FleetClient { _, path, body in
                guard path == "/api/v2/fleet/sync" else { throw URLError(.badURL) }
                self.posts.withLock { $0 += 1 }
                let request = try JSONDecoder().decode(Posted.self, from: body ?? Data())
                return try self.replica.withLock { replica in
                    guard request.fleet == replica.fleet else { throw URLError(.badServerResponse) }
                    let now = Date(timeIntervalSince1970: 1_800_000_000)
                    replica = try replica.applying(request.ops, at: now)
                    // The answer to a post is lost on its way back.
                    if self.offline.withLock({ $0 }) { throw URLError(.networkConnectionLost) }
                    let missing = try replica.missing(for: request.heads, at: now)
                    let answer: [String: Any] = [
                        "fleet": replica.fleet, "heads": replica.status.heads, "ops": missing.ops,
                        "refused": replica.refused, "more": missing.more,
                    ]
                    return try JSONSerialization.data(withJSONObject: answer)
                }
            }
        }

        struct Posted: Decodable { let fleet: String; let heads: [String]; let ops: [String] }
    }

    @Test func aPhoneWithNoCopyStartsOneFromAnyComputer() async throws {
        let vector = try Vector()
        let now = vector.now
        let computer = Computer(try vector.replica())
        let outcome = try await FleetLogSync.run(fleet: vector.fleet, me: vector.me, log: nil, with: computer.client) { now }
        #expect(outcome.replica.status.heads == outcome.heads)
        #expect(outcome.replica.status.join == .member)
        #expect(outcome.refused == 0)
    }

    @Test func whatThePhoneSignedReachesAComputerThatNeverSawThePhonesFirst() async throws {
        let vector = try Vector()
        let now = vector.now
        let replica = try vector.replica()
        let (_, signed) = try replica.signing(
            replica.draft([.remove(member: vector.removedMember, reason: .lost)], issued: vector.now, at: vector.now),
            with: vector.sign, at: vector.now
        )
        // The computer that introduced the phone is gone; it lost the
        // answer to the one post that reached it.
        let introducer = Computer(try vector.replica())
        introducer.offline.withLock { $0 = true }
        await #expect(throws: URLError.self) {
            _ = try await FleetLogSync.run(fleet: vector.fleet, me: vector.me, log: signed.log, with: introducer.client) { now }
        }
        // Another computer of the fleet gets the very same bytes from the
        // phone's own copy: nothing is signed again.
        let other = Computer(try vector.replica())
        let outcome = try await FleetLogSync.run(fleet: vector.fleet, me: vector.me, log: signed.log, with: other.client) { now }
        #expect(outcome.refused == 0)
        #expect(outcome.heads == signed.status.heads)
        let held = other.replica.withLock { $0 }
        #expect(held.status.view?.members.contains { $0.id == vector.removedMember } == false)
    }

    @Test func aComputerOfAnotherFleetIsRefused() async throws {
        let vector = try Vector()
        let now = vector.now
        let stranger = FleetClient { _, _, _ in
            Data(#"{"fleet":"\#(String(repeating: "07", count: 32))","heads":[],"ops":[],"refused":0,"more":false}"#.utf8)
        }
        await #expect(throws: FleetLogError.otherFleet) {
            _ = try await FleetLogSync.run(fleet: vector.fleet, me: vector.me, log: nil, with: stranger) { now }
        }
    }

    @Test func aComputerThatHandsAnotherHistoryStartsNothing() async throws {
        // It names this fleet but sends history without its founding op.
        let vector = try Vector()
        let now = vector.now
        let ops = Array(vector.ops.dropFirst())
        let fleet = vector.fleet
        let liar = FleetClient { _, _, _ in
            let answer: [String: Any] = ["fleet": fleet, "heads": [], "ops": ops, "refused": 0, "more": false]
            return try JSONSerialization.data(withJSONObject: answer)
        }
        await #expect(throws: FleetLogError.unfounded) {
            _ = try await FleetLogSync.run(fleet: vector.fleet, me: vector.me, log: nil, with: liar) { now }
        }
    }
}
