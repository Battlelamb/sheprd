import SheprdRemoteCore
import SwiftUI

/// The reply bar under a terminal agent's conversation. Its transcript is
/// read only, so what is typed here goes to the agent's own terminal:
/// pasted, then Enter, as typing it there would.
struct TerminalReply: View {
    @Environment(AppModel.self) private var model
    let key: PaneKey
    /// The agent's name, as the field's placeholder says it.
    let agent: String
    /// The agent waits on something only its terminal shows, such as a
    /// permission prompt.
    let waiting: Bool
    let terminal: Route?
    @State private var typing: TermTyping?
    @State private var text = ""

    var body: some View {
        VStack(spacing: 8) {
            if waiting, let terminal {
                HStack(spacing: 10) {
                    Image(systemName: "hand.raised.fill").foregroundStyle(Home.AgentStatus.blocked.color)
                    Text("\(agent) is waiting for you in the terminal").font(.subheadline.weight(.medium))
                    Spacer(minLength: 0)
                    RouteLink(terminal) { Text("Open").font(.subheadline.weight(.semibold)) }
                        .buttonStyle(.glass)
                }
                .padding(.leading, 14).padding(.trailing, 6).padding(.vertical, 6)
                .glassEffect(.regular, in: .capsule)
                .accessibilityElement(children: .combine)
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField("Message \(agent)", text: $text, axis: .vertical)
                    .lineLimit(1 ... 6)
                    .padding(.horizontal, 16).padding(.vertical, 11)
                    .glassEffect(.regular, in: .rect(cornerRadius: 22))
                    .submitLabel(.send)
                Button(action: send) {
                    Image(systemName: "arrow.up").font(.body.weight(.semibold)).frame(width: 30, height: 30)
                }
                .buttonStyle(.glassProminent)
                .buttonBorderShape(.circle)
                .disabled(trimmed.isEmpty)
                .accessibilityLabel("Send to the terminal")
            }
            if let problem = typing?.problem {
                Text(problem).font(.footnote).foregroundStyle(.red)
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 8)
        .onAppear { typing = TermTyping(key: key, fleet: model.fleet) }
    }

    private var trimmed: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    private func send() {
        guard let typing, !trimmed.isEmpty else { return }
        typing.paste(trimmed)
        typing.press("enter")
        text = ""
    }
}
