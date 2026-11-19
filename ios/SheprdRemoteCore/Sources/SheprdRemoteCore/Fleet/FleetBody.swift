import CryptoKit
import Foundation

/// What fleet operations say, laid out byte for byte as
/// `crates/sheprd-remote-control/src/control/fleet/body.rs` writes them.
/// Keys are P-256 (SEC1), signatures 64 bytes (`r` then `s`).
public enum Fleet {
    /// What every signed message starts with.
    public static let domain = Data("sheprd-fleet-v1".utf8)

    static let maxName = 64
    static let maxUser = 64
    static let maxSSHKey = 256
    static let maxCertificate = 8192
    static let maxField = 2048
    static let maxRelays = 16
    static let maxDirect = 8
    static let maxAddress = 128
    static let maxHostKeys = 4
    static let maxHostKey = 1024
    static let maxAlsoMembers = 16
    static let maxHosts = 256
    static let maxDeviceKey = 65

    public enum Kind: UInt8, Sendable { case computer = 1, phone = 2 }
    public enum Method: UInt8, Sendable { case bluetooth = 1, ssh = 2, qr = 3 }
    public enum Reason: UInt8, Sendable { case lost = 1, retired = 2, compromised = 3, left = 4 }
    public enum HostKind: UInt8, Sendable { case member = 1, sheprd = 2, agent = 3 }
    public enum Verified: UInt8, Sendable { case knownHosts = 1, bluetoothCode = 2, firstUseApproved = 3 }

    /// What a member publishes about itself.
    public struct Bundle: Equatable, Sendable {
        public var key: Data
        public var kind: Kind
        public var name: String
        public var dial: Data
        public var tunnel: Data
        public var ssh: String?
        public var host: HostIntro?

        /// The member it describes.
        public var id: Data { Data(SHA256.hash(data: key)) }

        /// What the joining member signs: that it holds this key, for this
        /// fleet.
        public func joinMessage(fleet: Data) throws -> Data {
            var writer = FleetWriter()
            writer.raw(Fleet.domain)
            writer.raw(Data("/join".utf8))
            writer.raw(fleet)
            try write(&writer)
            return writer.bytes
        }

        func write(_ writer: inout FleetWriter) throws {
            guard key.count == 33, dial.count == 33, tunnel.count == 32 else { throw FleetError.invalid }
            writer.raw(key)
            writer.u8(kind.rawValue)
            try writer.text(name)
            writer.raw(dial)
            writer.raw(tunnel)
            try writer.option(ssh) { try $0.text($1) }
            try writer.option(host) { try $1.write(&$0) }
        }

        static func read(_ reader: inout FleetReader) throws -> Bundle {
            Bundle(
                key: try reader.raw(33),
                kind: try enumValue(&reader),
                name: try reader.text(max: maxName),
                dial: try reader.raw(33),
                tunnel: try reader.raw(32),
                ssh: try reader.option { try $0.text(max: maxSSHKey) },
                host: try reader.option { try HostIntro.read(&$0) }
            )
        }
    }

    /// A Sheprd host's certificate authority.
    public struct CA: Equatable, Sendable {
        public var certificate: Data
        /// SEC1 uncompressed.
        public var key: Data
    }

    /// How a host was added.
    public struct HostIntro: Equatable, Sendable {
        public var kind: HostKind
        public var name: String
        public var user: String
        public var ca: CA?
        public var record: SelfRecord
        public var recordSignature: Data?
        public var verified: Verified

        /// Its id: the CA's fingerprint, or the hash of a plain host's first
        /// SSH host key.
        public var id: Data? {
            if let ca { return Data(SHA256.hash(data: ca.certificate)) }
            guard let first = record.sshHostKeys.first else { return nil }
            return Data(SHA256.hash(data: Fleet.domain + Data("/host".utf8) + Data(first.utf8)))
        }

        /// Whether the CA signed `record`.
        public func vouches(for record: SelfRecord, signature: Data?) -> Bool {
            switch (ca, signature) {
            case (nil, nil):
                return true
            case let (ca?, signature?):
                guard let key = try? P256.Signing.PublicKey(x963Representation: ca.key),
                      let signature = try? P256.Signing.ECDSASignature(rawRepresentation: signature),
                      let message = try? record.message()
                else { return false }
                return key.isValidSignature(signature, for: message)
            default:
                return false
            }
        }

        func write(_ writer: inout FleetWriter) throws {
            writer.u8(kind.rawValue)
            try writer.text(name)
            try writer.text(user)
            try writer.option(ca) { writer, ca in
                try writer.bytes(ca.certificate)
                writer.raw(ca.key)
            }
            try record.write(&writer)
            writer.option(recordSignature) { $0.raw($1) }
            writer.u8(verified.rawValue)
        }

        static func read(_ reader: inout FleetReader) throws -> HostIntro {
            HostIntro(
                kind: try enumValue(&reader),
                name: try reader.text(max: maxName),
                user: try reader.text(max: maxUser),
                ca: try reader.option { CA(certificate: try $0.bytes(max: maxCertificate), key: try $0.raw(65)) },
                record: try SelfRecord.read(&reader),
                recordSignature: try reader.option { try $0.raw(64) },
                verified: try enumValue(&reader)
            )
        }
    }

    /// Where a host is, in its own words.
    public struct SelfRecord: Equatable, Sendable {
        public var host: Data
        public var rev: UInt64
        public var route: String
        public var relays: [String]
        public var identity: String
        public var direct: [String]
        public var sshHostKeys: [String]
        public var signedMs: UInt64

        /// What the host's CA signs.
        public func message() throws -> Data {
            var writer = FleetWriter()
            writer.raw(Fleet.domain)
            writer.raw(Data("/self".utf8))
            try write(&writer)
            return writer.bytes
        }

        func write(_ writer: inout FleetWriter) throws {
            guard host.count == 32 else { throw FleetError.invalid }
            writer.raw(host)
            writer.u64(rev)
            try writer.text(route)
            try writer.texts(relays)
            try writer.text(identity)
            try writer.texts(direct)
            try writer.texts(sshHostKeys)
            writer.u64(signedMs)
        }

        static func read(_ reader: inout FleetReader) throws -> SelfRecord {
            SelfRecord(
                host: try reader.raw(32),
                rev: try reader.u64(),
                route: try reader.text(max: maxField),
                relays: try reader.texts(count: maxRelays, max: maxField),
                identity: try reader.text(max: maxField),
                direct: try reader.texts(count: maxDirect, max: maxAddress),
                sshHostKeys: try reader.texts(count: maxHostKeys, max: maxHostKey),
                signedMs: try reader.u64()
            )
        }
    }

    /// What an operation says.
    public enum Body: Equatable, Sendable {
        case found(Bundle)
        case admit(Bundle, method: Method, joinerSignature: Data)
        case remove(member: Data, alsoMembers: [Data], alsoHosts: [Data], reason: Reason)
        case rotate(Bundle)
        case hostAdd(HostIntro)
        case access(host: Data, member: Data, allow: Bool, rev: UInt64)
        case hostSeen(SelfRecord, signature: Data?)
        case hostRemove(Data)
        case link(host: Data, deviceKey: Data)
        case checkpoint(Data)

        var tag: UInt8 {
            switch self {
            case .found: 1
            case .admit: 2
            case .remove: 3
            case .rotate: 4
            case .hostAdd: 5
            case .access: 6
            case .hostSeen: 7
            case .hostRemove: 8
            case .link: 9
            case .checkpoint: 10
            }
        }

        func write(_ writer: inout FleetWriter) throws {
            writer.u8(tag)
            switch self {
            case let .found(bundle), let .rotate(bundle):
                try bundle.write(&writer)
            case let .admit(bundle, method, joinerSignature):
                try bundle.write(&writer)
                writer.u8(method.rawValue)
                writer.raw(joinerSignature)
            case let .remove(member, alsoMembers, alsoHosts, reason):
                guard alsoMembers.count <= maxAlsoMembers, alsoHosts.count <= maxHosts else { throw FleetError.bounds }
                writer.raw(member)
                try writer.count(alsoMembers.count)
                alsoMembers.forEach { writer.raw($0) }
                try writer.count(alsoHosts.count)
                alsoHosts.forEach { writer.raw($0) }
                writer.u8(reason.rawValue)
            case let .hostAdd(intro):
                try intro.write(&writer)
            case let .access(host, member, allow, rev):
                writer.raw(host)
                writer.raw(member)
                writer.bool(allow)
                writer.u64(rev)
            case let .hostSeen(record, signature):
                try record.write(&writer)
                writer.option(signature) { $0.raw($1) }
            case let .hostRemove(host):
                writer.raw(host)
            case let .link(host, deviceKey):
                writer.raw(host)
                try writer.bytes(deviceKey)
            case let .checkpoint(hash):
                writer.raw(hash)
            }
        }

        static func read(_ reader: inout FleetReader) throws -> Body {
            switch try reader.u8() {
            case 1: return .found(try Bundle.read(&reader))
            case 2:
                return .admit(try Bundle.read(&reader), method: try enumValue(&reader), joinerSignature: try reader.raw(64))
            case 3:
                let member = try reader.raw(32)
                let alsoMembers = try (0..<reader.count(max: maxAlsoMembers)).map { _ in try reader.raw(32) }
                let alsoHosts = try (0..<reader.count(max: maxHosts)).map { _ in try reader.raw(32) }
                return .remove(member: member, alsoMembers: alsoMembers, alsoHosts: alsoHosts, reason: try enumValue(&reader))
            case 4: return .rotate(try Bundle.read(&reader))
            case 5: return .hostAdd(try HostIntro.read(&reader))
            case 6:
                return .access(host: try reader.raw(32), member: try reader.raw(32), allow: try reader.bool(), rev: try reader.u64())
            case 7: return .hostSeen(try SelfRecord.read(&reader), signature: try reader.option { try $0.raw(64) })
            case 8: return .hostRemove(try reader.raw(32))
            case 9: return .link(host: try reader.raw(32), deviceKey: try reader.bytes(max: maxDeviceKey))
            case 10: return .checkpoint(try reader.raw(32))
            default: throw FleetError.encoding
            }
        }
    }

    static func enumValue<T: RawRepresentable>(_ reader: inout FleetReader) throws -> T where T.RawValue == UInt8 {
        guard let value = T(rawValue: try reader.u8()) else { throw FleetError.encoding }
        return value
    }
}
