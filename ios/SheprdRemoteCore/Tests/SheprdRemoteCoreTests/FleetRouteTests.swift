import CryptoKit
import Foundation
import Testing
@testable import SheprdRemoteCore

/// The relays a linked host is reached through: what `GET /api/v2/fleet`
/// says from the host's signed record, made into the relay record the
/// phone keeps with that computer, with its fleet bundle's keys.
struct FleetRouteTests {
    static let host = String(repeating: "04", count: 32)

    static func identity() -> String {
        let x25519 = Curve25519.KeyAgreement.PrivateKey().publicKey.rawRepresentation
        let mlkem = (try? MLKEM768.PrivateKey())?.publicKey.rawRepresentation ?? Data()
        return Base64URL.encode(x25519 + mlkem)
    }

    static func status(fingerprint: String = host, linked: Bool = true) throws -> API.FleetStatus {
        let object: [String: Any] = [
            "heads": [],
            "links": linked ? [host] : [],
            "routes": linked ? [[
                "host": host,
                "fingerprint": fingerprint,
                "relay": [
                    "identity": identity(),
                    "relays": ["https://relay.example.org"],
                    "route": Base64URL.encode(Data(repeating: 0x52, count: 32)),
                ],
            ]] : [],
        ]
        return try JSONDecoder().decode(API.FleetStatus.self, from: JSONSerialization.data(withJSONObject: object))
    }

    @Test func routesDecode() throws {
        let status = try Self.status()
        let route = try #require(status.routes.first)
        #expect(route.host == Self.host && route.fingerprint == Self.host)
        #expect(route.relay.relays == ["https://relay.example.org"])
        #expect(try JSONDecoder().decode(API.FleetStatus.self, from: Data(#"{"heads":[]}"#.utf8)).routes.isEmpty)
    }

    @Test func aLinkedHostsRecordCarriesTheBundlesKeys() throws {
        let keys = RelayKeyPair()
        let record = try #require(try Self.status().relayRecord(for: Self.host.uppercased(), keys: keys))
        #expect(record.dialKey == keys.dialKey && record.tunnelKey == keys.tunnelKey)
        #expect(record.route == Data(repeating: 0x52, count: 32))
        #expect(record.relays.map(\.description) == ["https://relay.example.org"])
    }

    @Test func aRouteNotPinnedToTheHostOrNotLinkedMakesNoRecord() throws {
        let other = String(repeating: "05", count: 32)
        #expect(try Self.status(fingerprint: other).relayRecord(for: Self.host, keys: RelayKeyPair()) == nil)
        #expect(try Self.status(linked: false).relayRecord(for: Self.host, keys: RelayKeyPair()) == nil)
        #expect(try Self.status().relayRecord(for: other, keys: RelayKeyPair()) == nil)
    }
}
