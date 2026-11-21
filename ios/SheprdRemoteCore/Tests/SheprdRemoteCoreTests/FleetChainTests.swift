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
        chain.refused(by: "studio")
        // Nothing is followed, or signed, until a fresh read says.
        #expect(chain.carried == nil)
        #expect(throws: FleetClientError.refusalUnresolved) { try chain.signed([ops[2]]) }
        // The read: the phone's next sequence number is past the applied one.
        chain.resolve(next: ops[1].seq + 1, from: "studio")
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
        chain.refused(by: "studio")
        chain.resolve(next: nil, from: "studio")
        #expect(chain.refusalPending && chain.carried == nil)
        // A later read: only the first applied. A later action, through a
        // computer behind it, follows the first, never the refused one.
        chain.resolve(next: ops[0].seq + 1, from: "studio")
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
        chain.refused(by: "studio")
        chain.resolve(next: ops[1].seq, from: "studio")
        #expect(chain.lastOp == ops[0])
        // Nothing of the phone's ever applied: nothing to follow.
        var fresh = FleetChain()
        try fresh.signed([ops[0]])
        fresh.refused(by: "studio")
        fresh.resolve(next: ops[0].seq, from: "studio")
        #expect(fresh.lastOp == nil && fresh.carried == [])
    }

    @Test func onlyTheRefusingComputerSettlesARefusal() throws {
        let signer = FleetClientTests.signer()
        let ops = try Self.line(3, from: 3, signer: signer)
        var chain = FleetChain()
        try chain.signed([ops[0], ops[1]])
        chain.confirmed()
        try chain.signed([ops[2]])
        chain.refused(by: "studio")
        // A computer behind, which has not heard of the second yet, would
        // name an older operation: it settles nothing, and lowers nothing.
        chain.resolve(next: ops[1].seq, from: "laptop")
        #expect(chain.refusalPending && chain.carried == nil)
        // The computer that refused settles it.
        chain.resolve(next: ops[2].seq, from: "studio")
        #expect(!chain.refusalPending && chain.lastOp == ops[1])
        // Another computer past every refused operation also may: they all
        // applied there.
        var past = FleetChain()
        try past.signed([ops[0], ops[1], ops[2]])
        past.refused(by: "studio")
        past.resolve(next: ops[2].seq + 1, from: "laptop")
        #expect(!past.refusalPending && past.lastOp == ops[2])
    }

    /// A refusal as the version before kept it: no computer, no floor.
    static func legacy(_ chain: FleetChain) throws -> FleetChain {
        var object = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(chain)) as? [String: Any])
        var refusal = try #require(object["refusal"] as? [String: Any])
        refusal["computer"] = nil
        refusal["floor"] = nil
        object["refusal"] = refusal
        return try JSONDecoder().decode(FleetChain.self, from: JSONSerialization.data(withJSONObject: object))
    }

    @Test func aRefusalKeptByTheVersionBeforeSettlesFromAnyComputer() throws {
        let signer = FleetClientTests.signer()
        let ops = try Self.line(2, from: 3, signer: signer)
        var chain = FleetChain()
        try chain.signed([ops[0]])
        chain.confirmed()
        // The second is refused and never applies anywhere.
        try chain.signed([ops[1]])
        chain.refused(by: "studio")
        var old = try Self.legacy(chain)
        #expect(old.refusalPending)
        // Any computer's read settles it, as before: it follows what the
        // post followed, never the refused one.
        old.resolve(next: ops[1].seq, from: "laptop")
        #expect(!old.refusalPending && old.lastOp == ops[0])
        #expect(old.carried == [ops[0]])
    }

    @Test func aLaggingReadNeverTakesBackTheConfirmedFloor() throws {
        let signer = FleetClientTests.signer()
        let ops = try Self.line(2, from: 3, signer: signer)
        // A confirms the first, refuses the second, and goes away.
        var chain = FleetChain()
        try chain.signed([ops[0]])
        chain.confirmed()
        try chain.signed([ops[1]])
        chain.refused(by: "studio")
        // B has not caught up to the first: its read settles nothing.
        chain.resolve(next: ops[0].seq, from: "laptop")
        #expect(chain.refusalPending && chain.carried == nil)
        // Nor would it from a refusal naming no computer.
        var unnamed = try Self.legacyKeepingFloor(chain)
        unnamed.resolve(next: ops[0].seq, from: "laptop")
        #expect(unnamed.refusalPending)
        // One caught up to it, from the refusing computer: the first stays
        // followed, and the next operation is the second sequence number.
        chain.resolve(next: ops[1].seq, from: "studio")
        #expect(!chain.refusalPending && chain.lastOp == ops[0])
        unnamed.resolve(next: ops[1].seq, from: "laptop")
        #expect(!unnamed.refusalPending && unnamed.lastOp == ops[0])
    }

    /// A refusal naming no computer, its floor kept.
    static func legacyKeepingFloor(_ chain: FleetChain) throws -> FleetChain {
        var object = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(chain)) as? [String: Any])
        var refusal = try #require(object["refusal"] as? [String: Any])
        refusal["computer"] = nil
        object["refusal"] = refusal
        return try JSONDecoder().decode(FleetChain.self, from: JSONSerialization.data(withJSONObject: object))
    }

    @Test func aRefusalWhoseComputerIsGoneSettlesOnceAnotherIsPastIt() throws {
        let signer = FleetClientTests.signer()
        let ops = try Self.line(3, from: 3, signer: signer)
        var chain = FleetChain()
        try chain.signed([ops[0]])
        chain.confirmed()
        try chain.signed([ops[1], ops[2]])
        // Refused by a computer the phone no longer reaches: only others
        // read now.
        chain.refused(by: "gone")
        for next in [ops[0].seq, ops[1].seq, ops[2].seq] {
            chain.resolve(next: next, from: "laptop")
            #expect(chain.refusalPending)
        }
        chain.resolve(next: ops[2].seq + 1, from: "laptop")
        #expect(!chain.refusalPending && chain.lastOp == ops[2])
    }

    @Test func aLineKeptBeforeItWasOneValueCarriesOver() throws {
        let signer = FleetClientTests.signer()
        let ops = try Self.line(2, from: 3, signer: signer)
        let encoded = try ops.map { Base64URL.encode(try $0.encode()) }
        let chain = FleetChain(legacyLast: encoded[1], unconfirmed: [encoded[1]])
        #expect(chain.lastOp == ops[1])
        #expect(chain.unconfirmedOps == [ops[1]])
        #expect(chain.carried == [ops[1]])
        // Only the last one signed, all confirmed.
        let confirmed = FleetChain(legacyLast: encoded[0], unconfirmed: nil)
        #expect(confirmed.lastOp == ops[0] && confirmed.unconfirmedOps.isEmpty)
        // Unreadable bytes are dropped, not followed.
        let broken = FleetChain(legacyLast: "!", unconfirmed: ["AA"])
        #expect(broken.lastOp == nil && broken.unconfirmedOps.isEmpty)
    }

    @Test func theLineIsKeptAcrossLaunches() throws {
        let signer = FleetClientTests.signer()
        let ops = try Self.line(2, from: 3, signer: signer)
        var chain = FleetChain()
        try chain.signed(ops)
        chain.refused(by: "studio")
        let decoded = try JSONDecoder().decode(FleetChain.self, from: JSONEncoder().encode(chain))
        #expect(decoded == chain && decoded.refusalPending)
    }
}
