import Foundation
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
