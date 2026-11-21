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
        #expect(chained.ops[1].parents.contains(try first.ops[0].id()))
        // A computer that applied it drafts past it: nothing to carry.
        let ahead = FleetClientTests.applying(Self.next(4)).client
        let fresh = try await ahead.prepare([.hostRemove(Data(repeating: 5, count: 32))], as: signer, at: .now, after: first.ops)
        #expect(fresh.ops.count == 1 && fresh.ops[0].seq == 4)
    }

    @Test func leavingIsConfirmedByThePhoneNoLongerCounting() async throws {
        let signer = FleetClientTests.signer()
        let posted = Mutex(false)
        // Once its removal applies, the phone has no next operation there.
        let computer = Computer { method, _, _ in
            if method == "POST" {
                posted.withLock { $0 = true }
                return FleetClientTests.applied
            }
            return FleetClientTests.status(next: posted.withLock { $0 } ? nil : Self.next(3))
        }
        let leave: Fleet.Body = .remove(member: signer.member, alsoMembers: [], alsoHosts: [], reason: .left)
        try await computer.client.sign([leave], as: signer, at: .now)
        // Removing another member gives no such proof: the phone is still
        // counted, so a missing next sequence number is not taken for it.
        posted.withLock { $0 = false }
        let other: Fleet.Body = .remove(member: Data(repeating: 6, count: 32), alsoMembers: [], alsoHosts: [], reason: .lost)
        await #expect(throws: FleetClientError.unapplied) {
            try await computer.client.sign([other], as: signer, at: .now)
        }
    }

    @Test func aNewOperationSeesBothTheUnconfirmedAndTheComputersHistory() async throws {
        let signer = FleetClientTests.signer()
        let first = try await FleetClientTests.applying(Self.next(3)).client
            .prepare([.hostRemove(Data(repeating: 4, count: 32))], as: signer, at: .now)
        // The computer has not applied it, but knows newer history (another
        // member's admission, say): a removal must name both.
        let newer = String(repeating: "0a", count: 32)
        let behind = FleetClientTests.applying(API.FleetNext(fleet: Self.fleet, seq: 3, parents: [newer])).client
        let chained = try await behind.prepare(
            [.remove(member: Data(repeating: 6, count: 32), alsoMembers: [], alsoHosts: [], reason: .lost)],
            as: signer, at: .now, after: first.ops
        )
        let parents = Set(chained.ops[1].parents)
        #expect(parents == [try first.ops[0].id(), try #require(Fleet.data(hex: newer))])
        // Within the limit, the unconfirmed one always kept.
        let many = (0..<20).map { String(format: "%064x", $0 + 100) }
        let kept = FleetClient.parents(keeping: String(repeating: "ff", count: 32), and: many)
        #expect(kept.count == Fleet.maxParents)
        #expect(kept.contains(String(repeating: "ff", count: 32)))
        #expect(kept == kept.sorted())
    }

    @Test func aComputerBehindGetsTheNextOperationAfterTheLastConfirmedOne() async throws {
        let signer = FleetClientTests.signer()
        // A confirms the phone's operation, then goes away before B hears
        // of it.
        let a = FleetClientTests.applying(Self.next(3))
        let first = try await a.client.prepare([.hostRemove(Data(repeating: 4, count: 32))], as: signer, at: .now)
        try await a.client.post(first)
        let last = try #require(first.ops.last)
        // Nothing waits unconfirmed, but the last one signed is kept and
        // carried.
        let carried = FleetClient.carried(unconfirmed: [], last: last)
        #expect(carried == [last])
        // B, behind, still drafts at 3: the next operation follows the last
        // one (sequence plus one, it among the parents), and B gets it too.
        let b = FleetClientTests.applying(Self.next(3))
        let second = try await b.client.prepare(
            [.hostRemove(Data(repeating: 5, count: 32))], as: signer, at: .now, after: carried
        )
        #expect(second.ops.count == 2 && second.ops[0] == last)
        #expect(second.ops[1].seq == last.seq + 1)
        #expect(second.ops[1].parents.contains(try last.id()))
        try await b.client.post(second)
        // A computer that heard of it drafts past it: nothing carried.
        let current = FleetClientTests.applying(API.FleetNext(fleet: Self.fleet, seq: 4, parents: [try last.id().hexString]))
        let third = try await current.client.prepare(
            [.hostRemove(Data(repeating: 6, count: 32))], as: signer, at: .now, after: carried
        )
        #expect(third.ops.count == 1 && third.ops[0].seq == 4)
        // Unconfirmed ones, when any, are what is carried, not only the last.
        #expect(FleetClient.carried(unconfirmed: second.ops, last: last) == second.ops)
        #expect(FleetClient.carried(unconfirmed: [], last: nil).isEmpty)
    }

    @Test func nothingIsPostedUnlessItWasKept() async throws {
        let computer = FleetClientTests.applying(Self.next(3))
        let signed = try await computer.client.prepare([.hostRemove(Data(repeating: 4, count: 32))], as: FleetClientTests.signer(), at: .now)
        let before = computer.requests.withLock { $0.count }
        await #expect(throws: FleetClientError.notKept) {
            try await computer.client.post(signed, kept: false)
        }
        #expect(computer.requests.withLock { $0.count } == before)
        try await computer.client.post(signed, kept: true)
        #expect(computer.requests.withLock { $0 }.contains { $0.method == "POST" })
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
