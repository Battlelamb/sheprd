import Foundation
import SheprdRemoteCore
import Testing

struct FleetHealthTests {
    @Test func routeKindClassifiesDirectAndRelay() {
        #expect(FleetHealth.routeKind(origin: "https://192.168.1.20:8443", throughRelay: false) == .local)
        #expect(FleetHealth.routeKind(origin: "https://m4max.local:8443", throughRelay: false) == .local)
        #expect(FleetHealth.routeKind(origin: "https://100.88.4.2:8443", throughRelay: false) == .tailscale)
        #expect(FleetHealth.routeKind(origin: "https://studio.tail123.ts.net:8443", throughRelay: false) == .tailscale)
        #expect(FleetHealth.routeKind(origin: "https://sheprd.example.com:8443", throughRelay: false) == .online)
        // Relay proxy binds on loopback; throughRelay must override origin inspection.
        #expect(FleetHealth.routeKind(origin: "https://127.0.0.1:54321", throughRelay: true) == .relay)
        #expect(FleetHealth.routeKind(origin: "https://100.88.4.2:8443", throughRelay: true) == .relay)
        #expect(FleetHealth.RouteKind.local.label == "Local")
        #expect(FleetHealth.RouteKind.tailscale.label == "Tailscale")
        #expect(FleetHealth.RouteKind.relay.label == "Relay")
        #expect(FleetHealth.RouteKind.online.label == "Online")
    }

    @Test func summaryCountsLiveOnlineWorkAndIgnoresOfflineCache() {
        let summary = FleetHealth.Summary(entries: [
            .init(status: .online(origin: "https://192.168.1.20:8443"), working: 2, waiting: 1),
            .init(status: .online(origin: "https://100.88.4.2:8443"), working: 4, waiting: 2),
            .init(status: .offline(reason: "timeout"), working: 9, waiting: 5),
            .init(status: .connecting, working: 3, waiting: 1),
            .init(status: .revoked, working: 1, waiting: 1),
            .init(status: .outdated, working: 2, waiting: 0),
        ])
        #expect(summary.online == 2)
        #expect(summary.offline == 4)
        #expect(summary.total == 6)
        #expect(summary.working == 6)
        #expect(summary.waiting == 3)
        #expect(summary.settingsValue == "2 of 6 online")
    }

    @Test func summarySettingsValueFormatsEdgeCases() {
        #expect(FleetHealth.Summary(entries: []).settingsValue == "0")
        let allOnline = FleetHealth.Summary(entries: [
            .init(status: .online(origin: "https://192.168.1.20:8443")),
            .init(status: .online(origin: "https://100.88.4.2:8443")),
        ])
        #expect(allOnline.settingsValue == "2 online")
        let allOffline = FleetHealth.Summary(entries: [
            .init(status: .offline(reason: "none")),
        ])
        #expect(allOffline.settingsValue == "0 of 1 online")
    }

    @Test func versionLabelNormalizesPrefix() {
        #expect(FleetHealth.versionLabel(nil) == nil)
        #expect(FleetHealth.versionLabel("") == nil)
        #expect(FleetHealth.versionLabel("   ") == nil)
        #expect(FleetHealth.versionLabel("v") == nil)
        #expect(FleetHealth.versionLabel("0.15.2") == "v0.15.2")
        #expect(FleetHealth.versionLabel("v0.15.2") == "v0.15.2")
        #expect(FleetHealth.versionLabel("V1.2.0") == "v1.2.0")
    }

    @Test func lastSeenAndOfflineDetail() {
        let now = Date(timeIntervalSince1970: 1_763_000_000)
        #expect(FleetHealth.lastSeen(now.addingTimeInterval(15), now: now) == "just now")
        #expect(FleetHealth.lastSeen(now.addingTimeInterval(-30), now: now) == "just now")
        #expect(FleetHealth.lastSeen(now.addingTimeInterval(-5 * 60), now: now) == "5m ago")
        #expect(FleetHealth.lastSeen(now.addingTimeInterval(-2 * 3600), now: now) == "2h ago")
        let old = now.addingTimeInterval(-5 * 86_400)
        let formattedOld = FleetHealth.lastSeen(old, now: now)
        #expect(!formattedOld.hasSuffix("ago"))
        #expect(formattedOld != "just now")

        #expect(FleetHealth.offlineDetail(lastSeen: now.addingTimeInterval(-2 * 3600), reach: "Tailscale · Relay", now: now)
            == "Last seen 2h ago · Tailscale · Relay")
        #expect(FleetHealth.offlineDetail(lastSeen: nil, reach: "Tailscale · Relay", now: now) == "Tailscale · Relay")
        #expect(FleetHealth.offlineDetail(lastSeen: now.addingTimeInterval(-60), reach: nil, now: now) == "Last seen 1m ago")
        #expect(FleetHealth.offlineDetail(lastSeen: nil, reach: nil, now: now) == nil)
    }
}
