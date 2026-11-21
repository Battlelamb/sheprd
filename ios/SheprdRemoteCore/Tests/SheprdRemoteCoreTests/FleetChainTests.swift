import CryptoKit
import Foundation
import Testing
@testable import SheprdRemoteCore

/// The phone's own line of operations across refusals: a refused operation
/// is never followed, and nothing new is signed until a fresh read says
/// which of a refused post applied.
struct FleetChainTests {
    static let fleet = String(repeating: "09", count: 32)

    /// Operations signed one after another from `seq`, as a phone does.
    static func line(_ count: Int, from seq: UInt64, signer: Fleet.Signer) throws -> [Fleet.Op] {
        var on = API.FleetNext(fleet: fleet, seq: seq, parents: [String(repeating: "01", count: 32)])
        var ops: [Fleet.Op] = []
        for index in 0..<count {
            var op = try Fleet.draft(on: on, signer: signer.member, issuedMs: 1, body: .hostRemove(Data(repeating: UInt8(index + 4), count: 32)))
            try op.sign(using: signer.sign)
            ops.append(op)
            on = API.FleetNext(fleet: fleet, seq: op.seq + 1, parents: [try op.id().hexString])
        }
        return ops
    }

    @Test func aRefusedRetryIsNeverFollowed() throws {
        let signer = FleetClientTests.signer()
        let ops = try Self.line(3, from: 3, signer: signer)
        var chain = FleetChain()
        try chain.signed([ops[0]])
        chain.confirmed()
        // Sent again later, the next two: the fleet applies the first and
        // refuses the second.
        try chain.signed([ops[1], ops[2]])
        chain.refused()
        // Nothing is followed, or signed, until a fresh read says.
        #expect(chain.carried == nil)
        #expect(throws: FleetClientError.refusalUnresolved) { try chain.signed([ops[2]]) }
        // The read: the phone's next sequence number is past the applied one.
        chain.resolve(next: ops[1].seq + 1)
        #expect(chain.lastOp == ops[1])
        #expect(chain.carried == [ops[1]])
    }

    @Test func aRefusalWhoseReadFailedWaitsForOneThatWorks() async throws {
        let signer = FleetClientTests.signer()
        let ops = try Self.line(2, from: 3, signer: signer)
        var chain = FleetChain()
        try chain.signed([ops[0]])
        chain.confirmed()
        // The next is refused, and the read after it fails: nothing known.
        try chain.signed([ops[1]])
        chain.refused()
        chain.resolve(next: nil)
        #expect(chain.refusalPending && chain.carried == nil)
        // A later read: only the first applied. A later action, through a
        // computer behind it, follows the first, never the refused one.
        chain.resolve(next: ops[0].seq + 1)
        #expect(chain.lastOp == ops[0])
        let behind = FleetClientTests.applying(API.FleetNext(fleet: Self.fleet, seq: ops[0].seq, parents: [String(repeating: "01", count: 32)]))
        let after = try #require(chain.carried)
        let next = try await behind.client.prepare([.hostRemove(Data(repeating: 9, count: 32))], as: signer, at: .now, after: after)
        let newest = try #require(next.ops.last)
        #expect(newest.parents.contains(try ops[0].id()))
        #expect(!newest.parents.contains(try ops[1].id()))
        #expect(!next.ops.contains(ops[1]))
        #expect(newest.seq == ops[0].seq + 1)
    }

    @Test func aRefusalOfAllOfAPostFollowsWhatItFollowed() throws {
        let signer = FleetClientTests.signer()
        let ops = try Self.line(2, from: 3, signer: signer)
        var chain = FleetChain()
        try chain.signed([ops[0]])
        chain.confirmed()
        // Through a computer that had the first: the post holds only the
        // second, which is refused.
        try chain.signed([ops[1]])
        chain.refused()
        chain.resolve(next: ops[1].seq)
        #expect(chain.lastOp == ops[0])
        // Nothing of the phone's ever applied: nothing to follow.
        var fresh = FleetChain()
        try fresh.signed([ops[0]])
        fresh.refused()
        fresh.resolve(next: ops[0].seq)
        #expect(fresh.lastOp == nil && fresh.carried == [])
    }

    @Test func theLineIsKeptAcrossLaunches() throws {
        let signer = FleetClientTests.signer()
        let ops = try Self.line(2, from: 3, signer: signer)
        var chain = FleetChain()
        try chain.signed(ops)
        chain.refused()
        let decoded = try JSONDecoder().decode(FleetChain.self, from: JSONEncoder().encode(chain))
        #expect(decoded == chain && decoded.refusalPending)
    }
}
