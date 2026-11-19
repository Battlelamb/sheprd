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
    var through: Computer? {
        guard let record else { return nil }
        let computers = computers()
        return computers.first { $0.device == record.through }
            ?? computers.first { fleets[$0.device] == record.fleet }
    }

    /// Sheprd hosts in the fleet the phone may add: not linked yet, not
    /// held, and not a computer it already holds.
    var offers: [API.FleetHost] {
        guard admitted, let status, let view = status.view else { return [] }
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

    /// Reads the fleet from `computer`.
    func check(_ computer: Computer) async {
        do {
            let member = record.flatMap { FleetLog.data(hex: $0.member) }
            let status = try await client(computer).status(member: member)
            fleets[computer.device] = status.view?.fleet
            guard let record, status.view?.fleet == record.fleet else {
                if self.record == nil { self.status = status }
                return
            }
            self.status = status
            route(status)
            if !record.admitted, status.join == .member {
                var admitted = record
                admitted.admitted = true
                admitted.through = computer.device
                save(admitted)
            }
        } catch {
            // A daemon from before the fleet answers 404 here.
            log.info("\(computer.name, privacy: .public) gave no fleet: \(error.localizedDescription, privacy: .public)")
        }
    }

    /// Asks to join `computer`'s fleet: makes this phone's fleet key and
    /// signs its bundle, which Face ID confirms; a person then admits it
    /// in Sheprd on that computer.
    func join(through computer: Computer, name: String) async {
        guard record == nil, let fleet = fleets[computer.device] else { return }
        await run {
            let key = try FleetKey.create()
            let relay = RelayKeyPair()
            let bundle = try FleetLog.phoneBundle(key: key.publicKey, name: name, relay: relay)
            let joined = try await client(computer).join(bundle, fleet: fleet, as: key.signer)
            save(Record(
                keyKind: key.kind, keyData: key.data, member: joined.member, fleet: fleet,
                through: computer.device, admitted: joined.state == .member,
                relayDial: relay.dialKey, relayTunnel: relay.tunnelKey
            ))
            follow(computer)
        }
    }

    /// Stops waiting for a join, and forgets its key.
    func cancelJoin() {
        guard record?.admitted == false else { return }
        waiting?.cancel()
        forget()
    }

    /// Removes a member, this phone included, which then leaves.
    func remove(member: String, reason: FleetLog.Reason) async {
        guard let id = FleetLog.data(hex: member) else { return }
        let leaving = member == record?.member
        await sign("Remove this member from your fleet", [
            .remove(member: id, alsoMembers: [], alsoHosts: [], reason: leaving ? .left : reason),
        ])
        if leaving, problem == nil { forget() }
    }

    /// Removes a host from the whole fleet.
    func removeHost(_ host: String) async {
        guard let id = FleetLog.data(hex: host) else { return }
        await sign("Remove this host from your fleet", [.hostRemove(id)])
    }

    /// Signs one `Link` per host, with a new device key for each, so those
    /// hosts admit this phone to their API; keeps each as a computer
    /// reached through the one the phone joined through.
    func link(_ hosts: [API.FleetHost]) async {
        guard let through, let status else { return }
        let reason = hosts.count == 1 ? "Add \(hosts[0].name)" : "Add \(hosts.count) computers"
        // A new device key per host, as host links make them: the host
        // admits this key alone, and the phone signs in with it there.
        var made: [(host: API.FleetHost, id: Data, key: HostKey, point: Data)] = []
        do {
            for host in hosts {
                guard let id = FleetLog.data(hex: host.id) else { continue }
                let key = try DeviceKey.create()
                let kept = HostKey(publicKey: DeviceKeyEncoding.publicKey(key.publicKey), kind: key.kind, data: key.data)
                made.append((host, id, kept, key.publicKey.x963Representation))
            }
        } catch {
            problem = error.localizedDescription
            return
        }
        let bodies: [FleetLog.Body] = made.map { .link(host: $0.id, deviceKey: $0.point) }
        // Signing reads the fleet again, so `self.status` now carries the
        // linked hosts' relays.
        await sign(reason, bodies)
        guard problem == nil else { return }
        let linked = self.status ?? status
        for (host, _, key, point) in made {
            guard let device = FleetLog.linkDevice(key: point),
                  let fingerprint = Fingerprint(host.id)
            else { continue }
            let enrolled = HostState.Enrolled(
                device: device, computer: host.name, fingerprint: fingerprint,
                origins: linked.origins(of: host.id)
            )
            var computer = Computer(host: enrolled, key: key, via: through.fingerprint)
            computer.relay = relayKeys.flatMap { linked.relayRecord(for: host.id, keys: $0) }
            onLinked(computer)
        }
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

    private func sign(_ reason: String, _ bodies: [FleetLog.Body]) async {
        guard let record, record.admitted, let through, !bodies.isEmpty else { return }
        await run {
            let key = try await Self.open(record, reason: reason)
            try await client(through).sign(bodies, as: key.signer, at: .now)
            await check(through)
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

    private func run(_ action: () async throws -> Void) async {
        busy = true
        problem = nil
        defer { busy = false }
        do {
            try await action()
        } catch {
            log.error("fleet: \(error.localizedDescription, privacy: .public)")
            problem = Self.describe(error)
        }
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
