import Foundation
import Testing
@testable import SheprdRemoteCore

struct WarmChatsTests {
    private let now = Date(timeIntervalSince1970: 1_000_000)

    private func listed(_ key: String, busy: Bool = false, answered: TimeInterval? = nil) -> WarmChats<String>.Listed {
        .init(key: key, busy: busy, lastAnswer: answered.map { now.addingTimeInterval(-$0) })
    }

    @Test func busyChatsComeFirstThenTheMostRecentlyAnswered() {
        var warm = WarmChats<String>()
        let many = (0..<WarmChats<String>.listed + 2).map { listed("old\($0)", answered: 1000 + Double($0)) }
        let keys = warm.keys(many + [listed("busy", busy: true), listed("recent", answered: 5)], at: now)
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
        #expect(warm.keys(busy + [listed("quiet")], at: soon).contains("quiet"))
    }

    @Test func aChatLeftStopsLingering() {
        var warm = WarmChats<String>()
        let busy = (0..<WarmChats<String>.listed).map { listed("busy\($0)", busy: true) }
        warm.closed("quiet", at: now)
        let later = now.addingTimeInterval(WarmChats<String>.lingers)
        #expect(!warm.keys(busy + [listed("quiet")], at: later).contains("quiet"))
    }

    @Test func aChatNoLongerListedIsNotFollowed() {
        var warm = WarmChats<String>()
        warm.closed("gone", at: now)
        #expect(warm.keys([listed("other")], at: now) == ["other"])
    }
}
