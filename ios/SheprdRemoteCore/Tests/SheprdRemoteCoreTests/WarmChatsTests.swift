import Foundation
import Testing
@testable import SheprdRemoteCore

struct WarmChatsTests {
    private let now = Date(timeIntervalSince1970: 1_000_000)
    private let plenty = ["mac": WarmChats<String>.viewsPerLink]

    private func listed(_ key: String, busy: Bool = false, answered: TimeInterval? = nil) -> WarmChats<String>.Listed {
        .init(key: key, busy: busy, lastAnswer: answered.map { now.addingTimeInterval(-$0) })
    }

    /// Every key on one computer, "mac", unless it starts with "other".
    private func computer(_ key: String) -> String {
        key.hasPrefix("other") ? "other" : "mac"
    }

    @Test func busyChatsComeFirstThenTheMostRecentlyAnswered() {
        var warm = WarmChats<String>()
        let many = (0..<WarmChats<String>.listed + 2).map { listed("old\($0)", answered: 1000 + Double($0)) }
        let keys = warm.keys(many + [listed("busy", busy: true), listed("recent", answered: 5)], at: now, room: plenty, in: computer)
        #expect(keys.count == WarmChats<String>.listed)
        #expect(keys.contains("busy"))
        #expect(keys.contains("recent"))
        #expect(!keys.contains("old\(WarmChats<String>.listed + 1)"))
    }

    @Test func aChatLeftLingersBeyondTheListedFew() {
        var warm = WarmChats<String>()
        let busy = (0..<WarmChats<String>.listed).map { listed("busy\($0)", busy: true) }
        warm.closed("quiet", at: now)
        let soon = now.addingTimeInterval(WarmChats<String>.lingers - 1)
        #expect(warm.keys(busy + [listed("quiet")], at: soon, room: plenty, in: computer).contains("quiet"))
    }

    @Test func aChatLeftStopsLingering() {
        var warm = WarmChats<String>()
        let busy = (0..<WarmChats<String>.listed).map { listed("busy\($0)", busy: true) }
        warm.closed("quiet", at: now)
        let later = now.addingTimeInterval(WarmChats<String>.lingers)
        #expect(!warm.keys(busy + [listed("quiet")], at: later, room: plenty, in: computer).contains("quiet"))
    }

    @Test func aChatNoLongerListedIsNotFollowed() {
        var warm = WarmChats<String>()
        warm.closed("gone", at: now)
        #expect(warm.keys([listed("other")], at: now, room: ["other": 1], in: computer) == ["other"])
    }

    @Test func eachComputerFollowsOnlyWhatItsSocketHasRoomFor() {
        var warm = WarmChats<String>()
        let busy = (0..<WarmChats<String>.listed).map { listed("busy\($0)", busy: true) }
        warm.closed("quiet", at: now)
        warm.closed("left", at: now.addingTimeInterval(1))
        let keys = warm.keys(busy + [listed("quiet"), listed("left"), listed("other1", busy: true)], at: now, room: ["mac": 2], in: computer)
        // Lingering first, most recently left first; nothing on a computer
        // without room, as one too old to follow chats in the background.
        #expect(keys == ["left", "quiet"])
    }

    @Test func aComputerWithoutRoomDoesNotUseUpThePicks() {
        var warm = WarmChats<String>()
        let full = (0..<WarmChats<String>.listed).map { listed("other\($0)", busy: true) }
        let keys = warm.keys(full + [listed("quiet", answered: 5)], at: now, room: ["mac": 1], in: computer)
        #expect(keys == ["quiet"])
    }

    @Test func roomKeepsOneViewForTheNextScreen() {
        #expect(WarmChats<String>.room(shown: 0) == WarmChats<String>.viewsPerLink - 2)
        #expect(WarmChats<String>.room(shown: WarmChats<String>.viewsPerLink - 2) == 0)
        #expect(WarmChats<String>.room(shown: WarmChats<String>.viewsPerLink) == 0)
    }
}
