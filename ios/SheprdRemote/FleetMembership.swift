import Foundation
import LocalAuthentication
import Observation
import SheprdRemoteCore
import UIKit
import os

private let log = Logger(subsystem: "so.pen.herdr.remote", category: "fleet-membership")

/// This phone as a member of the person's fleet (`docs/relay-plan.md`,
/// Fleet, phase 5): joining through a paired computer, which a person then
/// admits in Sheprd there; reading the fleet from it; and signing what the
/// phone may sign, each with Face ID: removals of members and hosts, and
/// one `Link` per Sheprd host it adds, which those hosts admit to their
/// API only. The phone keeps no log of its own in this version: it signs
/// on what a member computer answers, and that computer checks every
/// operation as it checks any member's.
@MainActor
@Observable
final class FleetMembership {
    /// What the phone keeps about its membership, in the Keychain.
    struct Record: Codable, Equatable {
        var keyKind: Computer.KeyKind
        /// The Secure Enclave's blob for the fleet key.
        var keyData: Data
        /// The member id, hex.
        var member: String
        var fleet: String
        /// The computer it joined through, which it reads the fleet from.
        var through: String
        /// A person admitted it; until then the join waits on `through`.
        var admitted: Bool
        /// The private halves of the relay keys its bundle published (dial,
        /// then tunnel): every host it links lets these through its relays,
        /// and each linked computer's relay record carries them. Kept in the
        /// Keychain as other relay keys are.
        var relayDial: Data
        var relayTunnel: Data
        /// Device keys made for hosts whose `Link` may not have landed yet.
        var pendingLinks: [FleetPendingLink]?
    }

    private(set) var record: Record?
    /// The fleet as `record.through`, or the computer last asked, said it.
    private(set) var status: API.FleetStatus?
    /// The paired computers in a fleet, by device, with the fleet's id.
    private(set) var fleets: [String: String] = [:]
    private(set) var busy = false
    /// The last failure, to show until the next attempt.
    var problem: String?
    /// The offer card was closed this launch.
    var offerDismissed = false
    /// A computer of this phone's fleet says the phone is not a member:
    /// another member removed it, or that computer has not heard of its
    /// admission yet. The phone can then forget the fleet and join again.
    private(set) var notMember = false
    /// The computers on this phone.
    @ObservationIgnored var computers: () -> [Computer] = { [] }
    /// Keeps a host the phone linked itself to.
    @ObservationIgnored var onLinked: (Computer) -> Void = { _ in }
    @ObservationIgnored private var asked: [String: Date] = [:]
    @ObservationIgnored private var waiting: Task<Void, Never>?
    private let sessions: Sessions

    private static let account = "fleet"
    /// How often one computer is asked when its link comes online.
    private static let interval: TimeInterval = 60
    /// How long a join is followed: the daemon keeps it ten minutes.
    private static let patience: Duration = .seconds(10 * 60)

    init(sessions: Sessions) {
        self.sessions = sessions
        record = (try? Keychain.read(Self.account)).flatMap { $0 }
            .flatMap { try? JSONDecoder().decode(Record.self, from: $0) }
    }

    var admitted: Bool { record?.admitted == true }

    /// A paired computer in a fleet, while this phone is in none.
    var joinable: Computer? {
        guard record == nil else { return nil }
        return computers().first { $0.via == nil && fleets[$0.device] != nil }
    }

    /// The computer the phone reads the fleet from and posts to.
    var through: Computer? { candidates.first }

    /// The computers of this phone's fleet it may post to: the one it last
    /// reached first, then every other directly paired one in the fleet,
    /// so one that is offline or lost never blocks the rest.
    private var candidates: [Computer] {
        guard let record else { return [] }
        let computers = computers().filter { $0.via == nil }
        let saved = computers.filter { $0.device == record.through }
        let others = computers.filter { $0.device != record.through && fleets[$0.device] == record.fleet }
        return saved + others
    }

    /// Sheprd hosts in the fleet the phone may add: not linked yet, not
    /// held, and not a computer it already holds.
    var offers: [API.FleetHost] {
        guard admitted, !notMember, let status, let view = status.view else { return [] }
        let held = Set(computers().map(\.fingerprint.hex))
        return view.hosts.filter { host in
            host.runsSheprd && !host.held && !status.links.contains(host.id)
                && !held.contains(host.id.uppercased())
        }
    }

    /// Reads the fleet from `computer` when its link comes online, at
    /// most once a minute.
    func online(_ computer: Computer) {
        guard computer.via == nil else { return }
        if let last = asked[computer.device], Date.now.timeIntervalSince(last) < Self.interval { return }
        asked[computer.device] = .now
        Task { await check(computer) }
    }

    /// Reads the fleet from `computer`; whether a fresh answer about this
    /// phone's fleet came back. A failed read keeps the last status, which
    /// must not settle anything a post just changed.
    @discardableResult
    func check(_ computer: Computer) async -> Bool {
        do {
            let member = record.flatMap { FleetLog.data(hex: $0.member) }
            let status = try await client(computer).status(member: member)
            fleets[computer.device] = status.view?.fleet
            guard let record, status.view?.fleet == record.fleet else {
                if self.record == nil { self.status = status }
                return false
            }
            self.status = status
            route(status)
            settleLinks(status)
            notMember = record.admitted && status.lacks(member: record.member, in: record.fleet)
            if !record.admitted, status.join == .member {
                var admitted = record
                admitted.admitted = true
                admitted.through = computer.device
                save(admitted)
            }
            return true
        } catch {
            // A daemon from before the fleet answers 404 here.
            log.info("\(computer.name, privacy: .public) gave no fleet: \(error.localizedDescription, privacy: .public)")
            return false
        }
    }

    /// Asks to join `computer`'s fleet: makes this phone's fleet key and
    /// signs its bundle, which Face ID confirms; a person then admits it
    /// in Sheprd on that computer.
    func join(through computer: Computer, name: String) async {
        guard record == nil, let fleet = fleets[computer.device] else { return }
        busy = true
        problem = nil
        defer { busy = false }
        let key: FleetKey
        let bundle: FleetLog.Bundle
        do {
            key = try FleetKey.create()
            let relay = RelayKeyPair()
            bundle = try FleetLog.phoneBundle(key: key.publicKey, name: name, relay: relay)
            // Kept before the join is sent: if its answer is lost, the
            // daemon may hold a join for this key, and only this key can
            // become that member.
            save(Record(
                keyKind: key.kind, keyData: key.data, member: bundle.id.hexString, fleet: fleet,
                through: computer.device, admitted: false,
                relayDial: relay.dialKey, relayTunnel: relay.tunnelKey
            ))
        } catch {
            problem = Self.describe(error)
            return
        }
        guard record != nil else { return }
        do {
            let joined = try await client(computer).join(bundle, fleet: fleet, as: key.signer)
            if joined.state == .member, var admitted = record {
                admitted.admitted = true
                save(admitted)
            }
            follow(computer)
        } catch {
            log.error("fleet: join through \(computer.name, privacy: .public): \(error.localizedDescription, privacy: .public)")
            problem = Self.describe(error)
            if fleetRequestMayHaveLanded(status: Self.status(of: error)) {
                // No answer: the join may be waiting there. Keep the key and
                // follow it; Cancel forgets it.
                follow(computer)
            } else {
                forget()
            }
        }
    }

    /// Stops waiting for a join, and forgets its key.
    func cancelJoin() {
        guard record?.admitted == false else { return }
        waiting?.cancel()
        forget()
    }

    /// Forgets the fleet on this phone alone, once a computer of the fleet
    /// says the phone is no longer a member: nothing is signed, as a
    /// removed key may sign nothing, and joining again makes a new key.
    func forgetRemoved() {
        guard notMember else { return }
        forget()
    }

    /// Removes a member, this phone included, which then leaves.
    func remove(member: String, reason: FleetLog.Reason) async {
        guard let id = FleetLog.data(hex: member) else { return }
        let leaving = member == record?.member
        let posted = await sign("Remove this member from your fleet", [
            .remove(member: id, alsoMembers: [], alsoHosts: [], reason: leaving ? .left : reason),
        ])
        // Left once a computer took it, whatever failed before it.
        if leaving, case .taken = posted { forget() }
    }

    /// Removes a host from the whole fleet.
    func removeHost(_ host: String) async {
        guard let id = FleetLog.data(hex: host) else { return }
        await sign("Remove this host from your fleet", [.hostRemove(id)])
    }

    /// Signs one `Link` per host, with a new device key for each, so those
    /// hosts admit this phone to their API; keeps each as a computer
    /// reached through the one the phone joined through. The keys are kept
    /// before posting, so a reply that never arrives loses none of a link
    /// that landed: the next read of the fleet finishes them.
    func link(_ hosts: [API.FleetHost]) async {
        guard var record, record.admitted else { return }
        let reason = hosts.count == 1 ? "Add \(hosts[0].name)" : "Add \(hosts.count) computers"
        var made: [FleetPendingLink] = []
        do {
            for host in hosts where FleetLog.data(hex: host.id) != nil {
                let key = try DeviceKey.create()
                made.append(FleetPendingLink(
                    host: host.id, name: host.name, keyKind: key.kind, keyData: key.data,
                    publicKey: DeviceKeyEncoding.publicKey(key.publicKey), point: key.publicKey.x963Representation
                ))
            }
        } catch {
            problem = error.localizedDescription
            return
        }
        record.pendingLinks = FleetPendingLink.merge(record.pendingLinks ?? [], made)
        save(record)
        guard self.record == record else { return }
        let bodies: [FleetLog.Body] = made.compactMap { pending in
            FleetLog.data(hex: pending.host).map { .link(host: $0, deviceKey: pending.point) }
        }
        let posted = await sign(reason, bodies)
        guard let status else { return }
        // A link's key goes only when the link surely never landed: nothing
        // was posted, or the computer that took the post refused it and
        // its own fresh read does not show it. Missing from any other read
        // may only mean that computer has not caught up.
        switch posted {
        case .nothing, .refused(fresh: true):
            settleLinks(status, refused: made.map(\.host))
        case .taken, .refused(fresh: false), .unknown:
            settleLinks(status)
        }
    }

    /// Keeps as computers the pending links the fleet now holds, and drops
    /// those of `refused` hosts it does not (`FleetPendingLink.settle`).
    private func settleLinks(_ status: API.FleetStatus, refused: [String] = []) {
        guard var record, let pending = record.pendingLinks, !pending.isEmpty,
              let through = through
        else { return }
        let (done, waiting) = FleetPendingLink.settle(pending, linked: status.links, refused: refused)
        for link in done {
            guard let device = FleetLog.linkDevice(key: link.point), let fingerprint = Fingerprint(link.host) else { continue }
            let enrolled = HostState.Enrolled(
                device: device, computer: link.name, fingerprint: fingerprint,
                origins: status.origins(of: link.host)
            )
            var computer = Computer(host: enrolled, key: link.key, via: through.fingerprint)
            computer.relay = relayKeys.flatMap { status.relayRecord(for: link.host, keys: $0) }
            onLinked(computer)
        }
        record.pendingLinks = waiting.isEmpty ? nil : waiting
        if record != self.record { save(record) }
    }

    /// The relay keys of this phone's fleet bundle: the ones every host it
    /// linked lets through.
    private var relayKeys: RelayKeyPair? {
        record.map { RelayKeyPair(dialKey: $0.relayDial, tunnelKey: $0.relayTunnel) }
    }

    /// Gives each computer this phone linked the relays its host's own
    /// signed record names (`FleetRoute.verified`), when it has none or
    /// the record is newer than the one its relays came from: a host that
    /// changed relays is followed, and an older record served again is
    /// refused.
    private func route(_ status: API.FleetStatus) {
        guard let keys = relayKeys else { return }
        for computer in computers() where computer.via != nil && computer.device.hasPrefix("fleet-") {
            let host = computer.fingerprint.hex.lowercased()
            guard status.links.contains(host),
                  let relay = status.relayRecord(for: host, keys: keys, replacing: computer.relay)
            else { continue }
            var routed = computer
            routed.relay = relay
            onLinked(routed)
        }
    }

    // MARK: Private

    /// How signing and posting ended.
    private enum Posted {
        /// Nothing reached a computer.
        case nothing
        /// A computer took every operation.
        case taken
        /// The computer that took the post refused some; `fresh` says
        /// whether that same computer could be read after it.
        case refused(fresh: Bool)
        /// No computer answered after it was posted: it may have landed.
        case unknown
    }

    /// Signs `bodies` and posts them through the first computer of the
    /// fleet that answers. Answers how it ended for links that may have
    /// landed: `nil` when nothing was posted (Face ID refused, say).
    @discardableResult
    private func sign(_ reason: String, _ bodies: [FleetLog.Body]) async -> Posted {
        guard let record, record.admitted, !bodies.isEmpty else { return .nothing }
        let candidates = candidates
        guard !candidates.isEmpty else { return .nothing }
        busy = true
        problem = nil
        defer { busy = false }
        let key: FleetKey
        do {
            key = try await Self.open(record, reason: reason)
        } catch {
            problem = Self.describe(error)
            return .nothing
        }
        // Signed once, by the first computer that answers its fleet: a post
        // whose answer is lost goes to the next computer as the same bytes.
        var signed: FleetClient.Signed?
        let attempt = await FleetAttempt.first(candidates) { computer in
            let client = client(computer)
            let ops: FleetClient.Signed
            if let signed {
                ops = signed
            } else {
                ops = try await client.prepare(bodies, as: key.signer, at: .now)
                signed = ops
            }
            try await client.post(ops)
        } failed: { computer, error in
            log.error("fleet: \(computer.name, privacy: .public) did not take it: \(error.localizedDescription, privacy: .public)")
        }
        switch attempt {
        case let .done(computer):
            // A computer before it may have failed: that no longer matters.
            problem = nil
            if computer.device != record.through, var moved = self.record {
                moved.through = computer.device
                save(moved)
            }
            await check(computer)
            return .taken
        case let .refused(computer, count):
            problem = Self.describe(FleetClientError.refused(count))
            return .refused(fresh: await check(computer))
        case let .failed(error):
            problem = error.map(Self.describe)
            // Nothing signed means nothing posted; otherwise what was
            // posted may have landed.
            return signed == nil ? .nothing : .unknown
        }
    }

    /// Opens the fleet key with one Face ID for `reason`, so a batch of
    /// signatures asks once. The simulator's software key asks nothing.
    private static func open(_ record: Record, reason: String) async throws -> FleetKey {
        guard record.keyKind == .secureEnclave else { return try FleetKey(kind: record.keyKind, data: record.keyData) }
        let context = LAContext()
        context.localizedReason = reason
        try await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)
        return try FleetKey(kind: record.keyKind, data: record.keyData, context: context)
    }

    /// Follows a join until a person admits it, or the daemon forgets it.
    private func follow(_ computer: Computer) {
        waiting?.cancel()
        waiting = Task {
            let deadline = ContinuousClock.now + Self.patience
            while !Task.isCancelled, ContinuousClock.now < deadline, record?.admitted == false {
                try? await Task.sleep(for: .seconds(3))
                await check(computer)
            }
        }
    }

    private func client(_ computer: Computer) -> FleetClient {
        let sessions = sessions
        return FleetClient { method, path, body in
            let (client, token) = try await sessions.session(for: computer)
            return try await client.fleetCall(method, path, body: body, bearer: token)
        }
    }

    private func save(_ record: Record) {
        do {
            try Keychain.write(JSONEncoder().encode(record), account: Self.account)
            self.record = record
        } catch {
            problem = error.localizedDescription
        }
    }

    private func forget() {
        try? Keychain.delete(Self.account)
        record = nil
        status = nil
        notMember = false
    }

    /// The HTTP status a computer answered with, when it answered.
    private static func status(of error: Error) -> Int? {
        if case let RemoteError.status(code, _) = error { return code }
        return nil
    }

    private static func describe(_ error: Error) -> String {
        switch error {
        case FleetClientError.noFleet: "That computer is no longer in a fleet."
        case FleetClientError.notMember: "This \(UIDevice.current.model) is not a member of that fleet yet."
        case let FleetClientError.refused(count): "The fleet refused \(count == 1 ? "it" : "\(count) of them")."
        case RemoteError.status(429, _): "Too many devices are waiting to join. Try again in a few minutes."
        case RemoteError.status(404, _): "That computer's Sheprd does not know fleets yet. Update it first."
        default: error.localizedDescription
        }
    }
}
