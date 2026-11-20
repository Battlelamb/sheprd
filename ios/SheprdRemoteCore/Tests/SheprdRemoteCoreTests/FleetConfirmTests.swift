import CryptoKit
import Foundation
import Synchronization
import Testing
@testable import SheprdRemoteCore

/// A post counts once the fleet applied it, not once a computer took it:
/// a computer that lacks the history holds operations waiting. Until a
/// computer confirms them, new operations follow them, so no two share a
/// sequence number. And a post names a bounded number of heads.
struct FleetConfirmTests {
    typealias Computer = FleetClientTests.Computer

    static let fleet = String(repeating: "09", count: 32)

    static func next(_ seq: UInt64) -> API.FleetNext {
        API.FleetNext(fleet: fleet, seq: seq, parents: [String(repeating: "01", count: 32)])
    }

    @Test func aComputerThatOnlyHoldsThePostWaitingHasNotAppliedIt() async throws {
        // Takes it without a refusal, but its next sequence number for the
        // phone does not move: the operation waits for its parents there.
        let lagging = Computer { method, _, _ in
            method == "GET" ? FleetClientTests.status(next: Self.next(3)) : FleetClientTests.applied
        }
        await #expect(throws: FleetClientError.unapplied) {
            try await lagging.client.sign([.hostRemove(Data(repeating: 4, count: 32))], as: FleetClientTests.signer(), at: .now)
        }
        // One that applies it confirms it.
        let current = FleetClientTests.applying(Self.next(3))
        try await current.client.sign([.hostRemove(Data(repeating: 4, count: 32))], as: FleetClientTests.signer(), at: .now)
    }

    @Test func newOperationsFollowOnesNotYetConfirmed() async throws {
        let signer = FleetClientTests.signer()
        // Signed before, and confirmed by no computer.
        let first = try await FleetClientTests.applying(Self.next(3)).client
            .prepare([.hostRemove(Data(repeating: 4, count: 32))], as: signer, at: .now)
        // A computer that never applied it still drafts at 3.
        let behind = FleetClientTests.applying(Self.next(3)).client
        let chained = try await behind.prepare([.hostRemove(Data(repeating: 5, count: 32))], as: signer, at: .now, after: first.ops)
        #expect(chained.ops.count == 2)
        #expect(chained.ops[0] == first.ops[0])
        #expect(chained.ops[1].seq == 4)
        #expect(chained.ops[1].parents == [try first.ops[0].id()])
        // A computer that applied it drafts past it: nothing to carry.
        let ahead = FleetClientTests.applying(Self.next(4)).client
        let fresh = try await ahead.prepare([.hostRemove(Data(repeating: 5, count: 32))], as: signer, at: .now, after: first.ops)
        #expect(fresh.ops.count == 1 && fresh.ops[0].seq == 4)
    }

    @Test func aPostNamesABoundedNumberOfHeads() async throws {
        let heads = (0..<(FleetClient.maxSyncHeads + 6)).map { String(format: "%064x", $0) }
        let seen = Mutex<[String]>([])
        let computer = Computer { method, _, body in
            if method == "GET" {
                let posted = seen.withLock { !$0.isEmpty }
                let object: [String: Any] = [
                    "view": try #require(JSONSerialization.jsonObject(with: FleetClientTests.status(next: nil)) as? [String: Any])["view"] as Any,
                    "heads": heads,
                    "next": ["fleet": Self.fleet, "seq": posted ? 2 : 1, "parents": [heads[0]]],
                ]
                return try JSONSerialization.data(withJSONObject: object)
            }
            let request = try JSONDecoder().decode(FleetClientTests.Posted.self, from: body ?? Data())
            seen.withLock { $0 = request.heads }
            return Data(#"{"fleet":"09","heads":[],"refused":0}"#.utf8)
        }
        try await computer.client.sign([.hostRemove(Data(repeating: 4, count: 32))], as: FleetClientTests.signer(), at: .now)
        #expect(seen.withLock { $0 } == Array(heads.prefix(FleetClient.maxSyncHeads)))
    }
}
