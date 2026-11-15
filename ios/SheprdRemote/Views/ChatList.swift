import SheprdRemoteCore
import SwiftUI

/// Every chat across computers in the order the person chose, a question
/// answered in place. Agents run in terminal tabs join them, so Home lists
/// every agent the computers run. Home shows it.
struct ChatList: View {
    @Environment(AppModel.self) private var model
    @AppStorage(HomeRow.sortKey) private var sort = AgentSort.priority
    /// What a swipe or long press asked to close, waiting for confirmation.
    @State private var closing: CloseTarget?

    var body: some View {
        let sections = HomeRow.sections(sort, chats: model.fleet.chatItems, agents: model.fleet.agentTerminals)
        List {
            UnpairedCards()
            ForEach(sections) { section in
                Section(section.title) { ForEach(section.items) { row($0) } }
            }
        }
        .refreshable { model.fleet.networkChanged() }
        .closeConfirmation($closing)
    }

    @ViewBuilder
    private func row(_ row: HomeRow) -> some View {
        // A section names the workspace when they are grouped by it.
        let grouped = sort == .workspace
        switch row {
        case let .chat(item):
            Group {
                if let question = item.question {
                    // The card draws its own background and border.
                    AnswerCard(item: item, question: question)
                        .listRowInsets(EdgeInsets(top: 4, leading: 0, bottom: 4, trailing: 0))
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                } else {
                    NavigationLink(value: Route.chat(item.key, title: item.chat.title)) {
                        ChatRowLabel(item: item, showsComputer: !grouped, showsWorkspace: !grouped)
                    }
                }
            }
            .contextMenu {
                Button("Close Chat", systemImage: "xmark.circle", role: .destructive) { closing = .chat(item.key, title: item.chat.title) }
            }
            .closeSwipe(.chat(item.key, title: item.chat.title), closing: $closing)
            .summarizeSwipe(item.key)
        case let .agent(item):
            if let route = item.route {
                NavigationLink(value: route) { AgentTerminalRowLabel(item: item, showsComputer: !grouped, showsWorkspace: !grouped) }
                    .closeSwipe(item.closeTarget, closing: $closing)
            }
        }
    }
}
