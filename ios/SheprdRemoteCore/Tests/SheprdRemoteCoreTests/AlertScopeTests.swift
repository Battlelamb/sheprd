import CryptoKit
import Foundation
import SheprdRemoteCore
import Testing

/// `Fixtures/notify/scope.json`, written by the daemon
/// (`apns/tests/per_host.rs`): the phone scopes its own test push's thread
/// and collapse id exactly as a computer scopes its alerts.
@Suite struct AlertScopeTests {
    struct Fixture: Decodable {
        struct Collapse: Decodable {
            let tag: String
            let id: String
        }

        let public_key: String
        let scope: String
        let thread: String
        let collapse_ids: [Collapse]
    }

    @Test func thePhoneScopesAsTheDaemonDoes() throws {
        let url = try #require(Bundle.module.url(forResource: "Fixtures/notify/scope", withExtension: "json"))
        let fixture = try JSONDecoder().decode(Fixture.self, from: Data(contentsOf: url))
        let key = try P256.KeyAgreement.PublicKey(x963Representation: #require(Base64URL.decode(fixture.public_key)))
        let scope = AlertScope.scope(key)
        #expect(scope == fixture.scope)
        #expect(AlertScope.thread(scope, pane: "test") == fixture.thread)
        #expect(fixture.collapse_ids.count == 2)
        for collapse in fixture.collapse_ids {
            #expect(AlertScope.collapseId(scope, tag: collapse.tag) == collapse.id)
            #expect(collapse.id.utf8.count <= 64)
        }
    }

    @Test func twoKeysNeverShareAScope() {
        let one = AlertScope.scope(P256.KeyAgreement.PrivateKey().publicKey)
        let two = AlertScope.scope(P256.KeyAgreement.PrivateKey().publicKey)
        #expect(one != two)
        #expect(one.count == 16)
    }
}
