import Foundation
import Testing
@testable import SheprdRemoteCore

/// A reply that arrived while its chat was not on screen plays as the chat
/// opens, once.
@Suite struct ChatEffectsCatchUpTests {
    private let now = Date(timeIntervalSinceReferenceDate: 1000)

    private func transcript(_ texts: [String]) -> Transcript {
        var transcript = Transcript(agent: "Claude")
        transcript.entries = texts.enumerated().map { index, text in
            Entry(id: EntryId(UInt64(index + 1)), kind: .agent(text: text))
        }
        return transcript
    }

    private func key(_ entry: UInt64, _ order: Int) -> ChatEffects.BubbleKey {
        .init(entry: EntryId(entry), order: order)
    }

    @Test func aChatOpenedForTheFirstTimePlaysItsLastReply() {
        var effects = ChatEffects()
        let shown = transcript([":::slam\nold\n:::", ":::slam\nDone\n:::\n::confetti"])
        effects.history(shown, metThrough: nil)
        #expect(effects.playing(at: now) == nil)
        let began = effects.catchUp(shown, now: now)
        #expect(began.contains(.screen(.confetti, text: "Done")))
        #expect(effects.playing(at: now)?.entry == EntryId(2))
        #expect(effects.bubbleSince(key(2, 0)) == now)
        // The reply before it is noted, not played.
        #expect(effects.bubbleSince(key(1, 0)) == nil)
        #expect(effects.metThrough == EntryId(2))
    }

    @Test func aReplyAlreadyMetDoesNotPlayAgain() {
        var effects = ChatEffects()
        let shown = transcript(["::confetti"])
        effects.history(shown, metThrough: EntryId(1))
        #expect(effects.catchUp(shown, now: now).isEmpty)
        #expect(effects.playing(at: now) == nil)
    }

    @Test func onlyTheLatestOfSeveralMissedRepliesPlays() {
        var effects = ChatEffects()
        effects.history(transcript(["earlier"]))
        let shown = transcript(["earlier", ":::slam\na\n:::\n::balloons", ":::slam\nb\n:::\n::confetti"])
        effects.history(shown, metThrough: effects.metThrough)
        let began = effects.catchUp(shown, now: now)
        #expect(effects.playing(at: now)?.entry == EntryId(3))
        #expect(!began.contains(.screen(.balloons, text: "a")))
        #expect(effects.bubbleSince(key(2, 0)) == nil)
        #expect(effects.bubbleSince(key(3, 0)) == now)
    }

    @Test func openingAgainPlaysNothing() {
        var effects = ChatEffects()
        let shown = transcript(["::confetti"])
        effects.history(shown, metThrough: nil)
        effects.catchUp(shown, now: now)
        let later = now.addingTimeInterval(ScreenEffect.confetti.length + 1)
        effects.expire(now: later)
        #expect(effects.catchUp(shown, now: later).isEmpty)
        #expect(effects.playing(at: later) == nil)
        // Nor after the chat is loaded again from what was kept.
        let met = effects.metThrough
        var reopened = ChatEffects()
        reopened.history(shown, metThrough: met)
        #expect(reopened.catchUp(shown, now: later).isEmpty)
    }

    @Test func aSnapshotKeepsRepliesAfterTheLastMetForCatchUp() {
        var effects = ChatEffects()
        effects.history(transcript(["earlier"]))
        let snapshot = transcript(["earlier", "::confetti"])
        effects.history(snapshot, metThrough: effects.metThrough)
        #expect(effects.metThrough == EntryId(1))
        #expect(effects.playing(at: now) == nil)
        #expect(!effects.catchUp(snapshot, now: now).isEmpty)
    }

    @Test func liveRepliesAdvanceWhatWasMet() {
        var effects = ChatEffects()
        effects.history(transcript(["earlier"]))
        let shown = transcript(["earlier", "::confetti"])
        effects.live(shown, changed: [EntryId(2)], now: now)
        #expect(effects.metThrough == EntryId(2))
        #expect(effects.catchUp(shown, now: now).isEmpty)
    }
}
