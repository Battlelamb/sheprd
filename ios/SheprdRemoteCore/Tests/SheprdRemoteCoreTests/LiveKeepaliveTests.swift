import Testing
@testable import SheprdRemoteCore

/// A pong cannot overtake a large message on its way to the phone, so a
/// late one alone does not close the socket: the address must also fail
/// a call of its own.
struct LiveKeepaliveTests {
    @Test func aPongInTimeAsksNothingElse() async {
        var probed = false
        let finding = await LiveKeepalive.check(pong: { true }, probe: { probed = true; return true })
        #expect(finding == .answered)
        #expect(!probed)
    }

    @Test func aLatePongOnABusySocketKeepsIt() async {
        #expect(await LiveKeepalive.check(pong: { false }, probe: { true }) == .busy)
    }

    @Test func noPongAndNoAnswerIsADeadPath() async {
        #expect(await LiveKeepalive.check(pong: { false }, probe: { false }) == .dead)
    }
}
