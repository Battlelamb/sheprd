import Foundation
@testable import SheprdRemoteCore
import Testing

@Suite struct RetractedAlertsTests {
    private let start = Date(timeIntervalSince1970: 1_800_000_000)

    @Test func aShownCopyOfItsEventIsRemovedOnce() {
        var retracted = RetractedAlerts()
        retracted.add("chat-w1:p2", event: "e1", at: start)
        #expect(retracted.take(shown: ["chat-w1:p2": "e1"], at: start) == ["chat-w1:p2"])
        #expect(retracted.take(shown: ["chat-w1:p2": "e1"], at: start).isEmpty)
    }

    /// The identifier is the chat's collapse id, which a newer alert for
    /// the same chat reuses: that one stays.
    @Test func aNewerAlertUnderTheSameIdentifierStays() {
        var retracted = RetractedAlerts()
        retracted.add("chat-w1:p2", event: "e1", at: start)
        #expect(retracted.take(shown: ["chat-w1:p2": "e2"], at: start).isEmpty)
        #expect(retracted.entries.isEmpty, "forgotten, not kept to remove later")
        retracted.add("chat-w1:p3", event: "e3", at: start)
        #expect(retracted.take(shown: ["chat-w1:p3": nil], at: start).isEmpty)
    }

    @Test func aCopyNotShownYetWaitsUntilItIsOrExpires() {
        var retracted = RetractedAlerts()
        retracted.add("x", event: "e1", at: start)
        #expect(retracted.take(shown: [:], at: start).isEmpty)
        #expect(retracted.entries.count == 1)
        #expect(retracted.take(shown: ["x": "e1"], at: start.addingTimeInterval(1)) == ["x"])
        retracted.add("y", event: "e2", at: start)
        #expect(retracted.take(shown: [:], at: start.addingTimeInterval(RetractedAlerts.window + 1)).isEmpty)
        #expect(retracted.entries.isEmpty)
    }

    @Test func itKeepsOnlyTheNewestFew() {
        var retracted = RetractedAlerts()
        for index in 0...RetractedAlerts.kept {
            retracted.add("n\(index)", event: "e\(index)", at: start)
        }
        #expect(retracted.entries.count == RetractedAlerts.kept)
        #expect(retracted.entries.first?.identifier == "n1")
    }

    @Test func theAppGroupKeepsItForTheAppAndTheExtension() throws {
        let suite = "retracted-\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        RetractedAlerts.add("x", event: "e1", at: start, in: defaults)
        #expect(RetractedAlerts.take(shown: ["x": "e1"], at: start, in: defaults) == ["x"])
        #expect(RetractedAlerts.take(shown: ["x": "e1"], at: start, in: defaults).isEmpty)
        #expect(RetractedAlerts.take(shown: ["x": "e1"], at: start, in: nil).isEmpty)
    }
}
