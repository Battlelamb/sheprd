import CryptoKit
import Foundation

/// What a sealed alert's thread and collapse id start with, as the daemon
/// makes them (`apns::alert::scope` in `sheprd-remote-control`): 16 hex
/// digits of a digest of the notify key the alert is sealed to, so two
/// computers' identical panes never collapse or group together, while
/// nothing in the clear names a computer. The phone's own test push uses
/// it with the key it seals to itself, so it looks like a computer's.
/// `Fixtures/notify/scope.json`, written by the daemon, checks both agree.
public enum AlertScope {
    static let info = Data("sheprd-alert-scope-v1".utf8)
    static let digits = 16
    /// The longest `apns-collapse-id` APNs takes, in bytes.
    static let maxCollapseId = 64

    /// The scope of alerts sealed to `key`.
    public static func scope(_ key: P256.KeyAgreement.PublicKey) -> String {
        let digest = SHA256.hash(data: info + key.x963Representation)
        return digest.prefix(digits / 2).map { String(format: "%02x", $0) }.joined()
    }

    /// The thread of `pane` in `scope`.
    public static func thread(_ scope: String, pane: String) -> String {
        "\(scope)/\(pane)"
    }

    /// The collapse id of `tag` in `scope`: the scope, a dash, and the
    /// tag, or a digest of the tag when that passes 64 bytes.
    public static func collapseId(_ scope: String, tag: String) -> String {
        let id = "\(scope)-\(tag)"
        if id.utf8.count <= maxCollapseId { return id }
        let digest = Base64URL.encode(Data(SHA256.hash(data: Data(tag.utf8))))
        return "\(scope)-\(digest.prefix(maxCollapseId - scope.utf8.count - 1))"
    }
}
