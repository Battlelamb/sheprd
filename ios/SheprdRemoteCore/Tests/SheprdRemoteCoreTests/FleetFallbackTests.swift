import Foundation
import Synchronization
import Testing
@testable import SheprdRemoteCore

/// Posting through the fleet's computers one after another, keeping what
/// a lost answer carries, and keeping the relay keys a linked computer
/// already works with.
struct FleetFallbackTests {
    struct Offline: Error, Equatable { let name: String }

    @Test func aComputerThatTakesItEndsTheAttemptWhateverFailedBefore() async {
        var failed: [String] = []
        let attempt = await FleetAttempt.first(["laptop", "studio", "spare"]) { name in
            if name == "laptop" { throw Offline(name: name) }
        } failed: { name, _ in
            failed.append(name)
        }
        guard case let .done(name) = attempt else {
            Issue.record("not done: \(attempt)")
            return
        }
        #expect(name == "studio")
        #expect(failed == ["laptop"])
    }

    @Test func aRefusalStopsTheAttempt() async {
        var tried: [String] = []
        let attempt = await FleetAttempt.first(["laptop", "studio"]) { name in
            tried.append(name)
            throw FleetClientError.refused(2)
        }
        guard case let .refused(name, count) = attempt else {
            Issue.record("not refused: \(attempt)")
            return
        }
        #expect(name == "laptop" && count == 2 && tried == ["laptop"])
    }

    @Test func whenNoneAnswersTheLastFailureIsKept() async {
        let attempt = await FleetAttempt.first(["laptop", "studio"]) { name in
            throw Offline(name: name)
        }
        guard case let .failed(error) = attempt else {
            Issue.record("not failed: \(attempt)")
            return
        }
        #expect(error as? Offline == Offline(name: "studio"))
        let none = await FleetAttempt<String>.first([], { _ in })
        guard case .failed(nil) = none else {
            Issue.record("tried no one, yet did not fail")
            return
        }
    }

    @Test func aLostPostGoesToTheNextComputerAsTheSameBytes() async throws {
        typealias Computer = FleetClientTests.Computer
        let next = API.FleetNext(fleet: String(repeating: "09", count: 32), seq: 3, parents: [String(repeating: "01", count: 32)])
        // The first computer drafts and loses the answer to the post; the
        // second has not synced and would draft on older parents.
        let laptop = Computer { method, _, _ in
            if method == "GET" { return FleetClientTests.status(next: next) }
            throw Offline(name: "laptop")
        }
        let stale = API.FleetNext(fleet: next.fleet, seq: 1, parents: [String(repeating: "02", count: 32)])
        let studio = Computer { method, _, _ in
            method == "GET" ? FleetClientTests.status(next: stale) : FleetClientTests.applied
        }
        let signer = FleetClientTests.signer()
        var signed: FleetClient.Signed?
        let attempt = await FleetAttempt.first([laptop, studio]) { computer in
            let client = computer.client
            let ops: FleetClient.Signed
            if let signed {
                ops = signed
            } else {
                ops = try await client.prepare([.hostRemove(Data(repeating: 4, count: 32))], as: signer, at: .now)
                signed = ops
            }
            try await client.post(ops)
        }
        guard case .done = attempt else {
            Issue.record("not done: \(attempt)")
            return
        }
        let first = try #require(laptop.requests.withLock { $0.last?.body })
        let second = studio.requests.withLock { $0 }
        // Never signed again: no read, and the very bytes the first got.
        #expect(second.map(\.method) == ["POST"])
        let sent = try JSONDecoder().decode(FleetClientTests.Posted.self, from: first)
        let secondBody = try #require(second.first?.body)
        let resent = try JSONDecoder().decode(FleetClientTests.Posted.self, from: secondBody)
        #expect(resent.ops == sent.ops)
        let text = try #require(resent.ops.first)
        let op = try Fleet.Op.decode(try #require(Base64URL.decode(text)))
        #expect(op.seq == 3)
    }

    @Test func aLaggingComputersReadDropsNoKeys() {
        // A link taken by a computer that lags (it holds it waiting for its
        // parents) is missing from that computer's read, and from any
        // other that has not caught up: absence alone drops nothing.
        let pending = [FleetRecoveryTests.pending("aa"), FleetRecoveryTests.pending("bb")]
        let lagging = FleetPendingLink.settle(pending, linked: [:])
        #expect(lagging.done.isEmpty && lagging.waiting == pending)
        // Only an explicit refusal of a link drops it, and only that one.
        let refused = FleetPendingLink.settle(pending, linked: [:], refused: ["bb"])
        #expect(refused.waiting == [pending[0]])
    }

    @Test func fleetCallsGoThroughTheLiveConnectionWhenThereIsOne() async throws {
        typealias Computer = FleetClientTests.Computer
        // The live link (a relay, say) answers; the saved address is the
        // network the phone left.
        let live = Computer { _, _, _ in FleetClientTests.status(next: nil) }
        let saved = Computer { _, _, _ in throw Offline(name: "lan") }
        let online = Mutex(true)
        let client = FleetClient.preferring(
            live: { online.withLock { $0 } ? live.send : nil },
            fallback: saved.send
        )
        let status = try await client.status(member: nil)
        #expect(status.view != nil)
        #expect(live.requests.withLock { $0.count } == 1)
        #expect(saved.requests.withLock { $0.isEmpty })
        // With no live link (a computer just paired), the saved addresses.
        online.withLock { $0 = false }
        await #expect(throws: Offline(name: "lan")) { try await client.status(member: nil) }
        #expect(saved.requests.withLock { $0.count } == 1)
        #expect(live.requests.withLock { $0.count } == 1)
    }

    @Test func aLostOrFailedAnswerKeepsWhatTheRequestCarried() {
        // No answer, or the computer failed: the join may be waiting there.
        #expect(fleetRequestMayHaveLanded(status: nil))
        #expect(fleetRequestMayHaveLanded(status: 500))
        #expect(fleetRequestMayHaveLanded(status: 503))
        // Refused: nothing landed, the keys can go.
        for refused in [400, 401, 403, 404, 409, 429] {
            #expect(!fleetRequestMayHaveLanded(status: refused))
        }
    }

    @Test func aRefreshedRouteKeepsTheKeysAlreadyInUse() throws {
        let host = FleetRouteTests.Host()
        let id = host.id.hexString
        let bundle = RelayKeyPair()
        let first = try #require(FleetRouteTests.status([try host.route(host.record(rev: 1))]).relayRecord(for: id, keys: bundle))
        // The phone since sent keys of its own for this computer.
        let own = RelayKeyPair()
        var saved = first
        saved.dialKey = own.dialKey
        saved.tunnelKey = own.tunnelKey
        let refreshed = try #require(
            FleetRouteTests.status([try host.route(host.record(rev: 2))]).relayRecord(for: id, keys: bundle, replacing: saved)
        )
        #expect(refreshed.dialKey == own.dialKey && refreshed.tunnelKey == own.tunnelKey)
        #expect(refreshed.signedRev == 2)
    }
}
