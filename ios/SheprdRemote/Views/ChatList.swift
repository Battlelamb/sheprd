import SheprdRemoteCore
import SwiftUI

/// Every chat across computers in the order the person chose, a question
/// answered in place. Agents run in terminal tabs join them, so Home lists
/// every agent the computers run. Home shows it.
///
/// Each section folds to its header as a computer does in the Workspaces
/// tab, a status section's header shows its colored dot, and a long list
/// gets an index on its edge, as Workspaces has.
struct ChatList: View {
    @Environment(AppModel.self) private var model
    @AppStorage(HomeRow.sortKey) private var sort = AgentSort.priority
    var sections: [AgentSort.Section<HomeRow>]
    /// What a swipe or long press asked to close, waiting for confirmation.
    @State private var closing: CloseTarget?

    /// What `CollapsedStore.home` keeps for `section`: a workspace and a
    /// status may share an id, so the order names it too.
    static func key(_ section: AgentSort.Section<HomeRow>, sort: AgentSort) -> String {
        "\(sort.rawValue)/\(section.id)"
    }

    /// Rows open at once before the list needs its index: about a
    /// screenful on a phone, as in Workspaces.
    private static let indexedFrom = 12

    var body: some View {
        let open = sections
            .filter { !model.collapsed.home.contains(Self.key($0, sort: sort)) }
            .reduce(0) { $0 + $1.items.count }
        List {
            UnpairedCards()
            ForEach(sections) { section in
                let key = Self.key(section, sort: sort)
                let folded = model.collapsed.home.contains(key)
                Section {
                    if !folded { ForEach(section.items) { row($0) } }
                } header: {
                    SectionHeader(title: section.title, count: section.items.count, status: section.status, folded: folded) {
                        withAnimation(.snappy) { model.collapsed.home.toggle(key) }
                    }
                }
                .sectionIndexLabel(section.title.first.map { String($0).uppercased() })
            }
        }
        .listSectionIndexVisibility(sections.count > 1 && open >= Self.indexedFrom ? .visible : .hidden)
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

/// A status as a filled round icon, idle included, which `StatusMark`
/// leaves off a row's icon.
private struct StatusDot: View {
    var status: Home.AgentStatus

    var body: some View {
        let symbol = switch status {
        case .blocked: "exclamationmark.circle.fill"
        case .working: "ellipsis.circle.fill"
        case .done: "checkmark.circle.fill"
        case .idle, .unknown: "moon.circle.fill"
        }
        Image(systemName: symbol).foregroundStyle(status.color)
    }
}

/// A section's title and count, folding it on a tap.
private struct SectionHeader: View {
    var title: String
    var count: Int
    /// The status its rows share, shown as a colored dot.
    var status: Home.AgentStatus?
    var folded: Bool
    var toggle: () -> Void

    var body: some View {
        Button(action: toggle) {
            HStack(spacing: 6) {
                Image(systemName: "chevron.right").font(.caption.bold())
                    .rotationEffect(.degrees(folded ? 0 : 90))
                if let status { StatusDot(status: status) }
                Text(title).lineLimit(1)
                Text(count, format: .number).foregroundStyle(.secondary).monospacedDigit()
                Spacer(minLength: 0)
            }
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(folded ? "Expand" : "Collapse") \(title), \(count)")
    }
}
