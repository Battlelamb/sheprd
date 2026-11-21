import Foundation

/// The phone's own line of fleet operations, as it keeps it between
/// actions, so that every operation it signs follows the one before,
/// whichever computer it posts through; two operations by one member
/// where neither follows the other look like its key in use in two places.
///
/// - `last` is the last operation signed that the fleet has not refused,
///   kept after a computer confirms it: new ones follow it.
/// - `unconfirmed` are operations no computer confirmed applying yet:
///   sent again until one does, and carried by new ones.
/// - After a refusal, nothing new is signed until a fresh read says which
///   of the refused post applied (`resolve`): a refused operation is void
///   everywhere, so it is never followed.
public struct FleetChain: Codable, Equatable, Sendable {
    /// Canonical bytes of each operation.
    private var last: Data?
    private var unconfirmed: [Data] = []
    /// What the unconfirmed operations follow when they do not carry it.
    private var base: Data?
    private var refusal: Refusal?

    private struct Refusal: Codable, Equatable, Sendable {
        /// What was posted, and what it followed.
        var ops: [Data]
        var base: Data?
    }

    public init() {}

    /// The operation new ones follow.
    public var lastOp: Fleet.Op? { last.flatMap { try? Fleet.Op.decode($0) } }

    public var unconfirmedOps: [Fleet.Op] { unconfirmed.compactMap { try? Fleet.Op.decode($0) } }

    /// Whether a refusal waits for a fresh read to say what applied.
    public var refusalPending: Bool { refusal != nil }

    /// What new operations follow and carry (`FleetClient.carried`); nil
    /// while a refusal is unresolved, when nothing may be signed.
    public var carried: [Fleet.Op]? {
        guard refusal == nil else { return nil }
        return FleetClient.carried(unconfirmed: unconfirmedOps, last: lastOp)
    }

    /// Keeps `ops`, just signed (and the carried ones before them), as
    /// unconfirmed, before they are posted.
    public mutating func signed(_ ops: [Fleet.Op]) throws {
        guard refusal == nil else { throw FleetClientError.refusalUnresolved }
        let encoded = try ops.map { try $0.encode() }
        guard let newest = encoded.last else { return }
        if let last, !encoded.contains(last) { base = last }
        unconfirmed = encoded
        self.last = newest
    }

    /// A computer applied the unconfirmed operations.
    public mutating func confirmed() {
        unconfirmed = []
        base = nil
    }

    /// The fleet refused some of the unconfirmed operations. Which applied
    /// is not known until a fresh read (`resolve`); until then nothing is
    /// followed, and nothing new is signed.
    public mutating func refused() {
        guard !unconfirmed.isEmpty || refusal != nil else { return }
        if refusal == nil {
            refusal = Refusal(ops: unconfirmed, base: base)
        }
        unconfirmed = []
        base = nil
        last = nil
    }

    /// Settles a refusal with a fresh read of the fleet: `next` is the
    /// phone's next sequence number there, past every operation of its that
    /// applied. The last of the refused post that applied, else what it
    /// followed, becomes what new operations follow. With no `next` (the
    /// read says nothing about the phone), the refusal stays.
    public mutating func resolve(next: UInt64?) {
        guard let refusal, let next else { return }
        let known = (refusal.ops + (refusal.base.map { [$0] } ?? [])).compactMap { data -> (UInt64, Data)? in
            (try? Fleet.Op.decode(data)).map { ($0.seq, data) }
        }
        last = known.filter { $0.0 < next }.max { $0.0 < $1.0 }?.1
        self.refusal = nil
    }
}
