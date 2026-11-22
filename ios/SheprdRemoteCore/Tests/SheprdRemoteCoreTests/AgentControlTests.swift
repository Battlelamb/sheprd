import Foundation
import Testing
@testable import SheprdRemoteCore

/// The live socket's agent messages, as `control::phone` writes and reads
/// them on the computer.
struct AgentControlTests {
    private func event(_ json: String) throws -> LiveEvent {
        try JSONDecoder().decode(LiveEvent.self, from: Data(json.utf8))
    }

    private func object(_ request: LiveRequest) throws -> NSDictionary {
        let data = try JSONEncoder().encode(request)
        return try #require(try JSONSerialization.jsonObject(with: data) as? NSDictionary)
    }

    private func object(_ json: String) throws -> NSDictionary {
        try #require(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? NSDictionary)
    }

    @Test func decodesEachDrive() throws {
        #expect(try event(#"{"type":"drive","id":7,"command":"state"}"#) == .drive(id: 7, command: .state))
        #expect(try event(#"{"type":"drive","id":7,"command":"open","to":{"screen":"chat","pane":"w1-p2"}}"#)
            == .drive(id: 7, command: .open(.chat(pane: "w1-p2"))))
        #expect(try event(#"{"type":"drive","id":8,"command":"show","text":"Look"}"#) == .drive(id: 8, command: .show(text: "Look")))
        let screens: [(String, AgentScreen)] = [
            (#"{"screen":"home"}"#, .home),
            (#"{"screen":"workspaces"}"#, .workspaces),
            (#"{"screen":"pull_requests"}"#, .pullRequests),
            (#"{"screen":"settings"}"#, .settings),
            (#"{"screen":"new_workspace"}"#, .newWorkspace),
            (#"{"screen":"term","pane":"p"}"#, .term(pane: "p")),
            (#"{"screen":"workspace","workspace":"w1"}"#, .workspace("w1")),
            (#"{"screen":"changes","workspace":"w1"}"#, .changes(workspace: "w1")),
        ]
        for (json, screen) in screens {
            #expect(try event(#"{"type":"drive","id":1,"command":"open","to":\#(json)}"#) == .drive(id: 1, command: .open(screen)))
        }
    }

    /// A newer computer's command or screen is answered, refused, rather
    /// than dropped, so the computer does not wait for nothing.
    @Test func keepsTheIdOfWhatItDoesNotKnow() throws {
        #expect(try event(#"{"type":"drive","id":3,"command":"tap"}"#) == .drive(id: 3, command: .unknown))
        #expect(try event(#"{"type":"drive","id":4,"command":"open","to":{"screen":"camera"}}"#) == .drive(id: 4, command: .unknown))
    }

    @Test func encodesTheSwitchAndAnswersAsTheComputerReadsThem() throws {
        #expect(try object(.agentControl(allowed: true)) == object(#"{"type":"agent_control","allowed":true}"#))
        #expect(try object(.agentControl(allowed: false)) == object(#"{"type":"agent_control","allowed":false}"#))
        let done = LiveRequest.driven(id: 7, outcome: .done(AgentPhoneState(screen: .chat(pane: "w1-p2"))))
        #expect(try object(done) == object(#"{"type":"driven","id":7,"outcome":"done","state":{"screen":{"screen":"chat","pane":"w1-p2"}}}"#))
        let locked = LiveRequest.driven(id: 8, outcome: .done(AgentPhoneState(screen: nil, locked: true)))
        #expect(try object(locked) == object(#"{"type":"driven","id":8,"outcome":"done","state":{"locked":true}}"#))
        for (reason, name) in [(AgentRefusal.locked, "locked"), (.notFound, "not_found"), (.off, "off"), (.other, "other")] {
            #expect(try object(.driven(id: 9, outcome: .refused(reason)))
                == object(#"{"type":"driven","id":9,"outcome":"refused","reason":"\#(name)"}"#))
        }
    }

    /// Allowed is said again on every socket, as the computer forgets it
    /// with the socket; never said while the switch was never on.
    @Test func linkSaysAllowedAgainAfterAReconnect() async {
        let computer = FakeComputer()
        await computer.setPing("https://lan", mac)
        let link = ComputerLink(fingerprint: mac, origins: ["https://lan"], io: await computer.io())
        await link.setAgentControl(false)
        await link.watch(.home)
        await link.start()
        await until { await computer.sent.count == 1 }
        #expect(await computer.sent == [.subscribe(.home)])
        await link.setAgentControl(true)
        await until { await computer.sent.count == 2 }
        #expect(await computer.sent.last == .agentControl(allowed: true))
        await computer.drop()
        await link.networkChanged()
        await until { await computer.sent.count == 4 }
        #expect(await computer.sent.suffix(2) == [.subscribe(.home), .agentControl(allowed: true)])
        await link.setAgentControl(false)
        await until { await computer.sent.count == 5 }
        #expect(await computer.sent.last == .agentControl(allowed: false))
        await link.reply(id: 2, outcome: .refused(.off))
        await until { await computer.sent.count == 6 }
        #expect(await computer.sent.last == .driven(id: 2, outcome: .refused(.off)))
        await link.stop()
    }
}
