import SheprdRemoteCore
import SwiftUI

/// The repository or workspace the agent lists show, kept on this phone and
/// shared by its windows. Read it through `AppModel.agentScope`: the
/// workspace may have closed since.
@MainActor
@Observable
final class AgentScopeStore {
    private static let key = "agentScope"

    var scope: AgentScope? {
        didSet { UserDefaults.standard.set(scope.flatMap { try? JSONEncoder().encode($0) }, forKey: Self.key) }
    }

    init() {
        scope = UserDefaults.standard.data(forKey: Self.key).flatMap { try? JSONDecoder().decode(AgentScope.self, from: $0) }
    }
}

extension AppModel {
    /// The scope the agent lists show, while a workspace still falls in it.
    var agentScope: AgentScope? {
        AgentScope.active(agentScopeStore.scope, among: homes { _ in true })
    }

    /// Whether the agent lists show what runs in `workspace` on `computer`:
    /// a computer the tag filter leaves, in the chosen repository or
    /// workspace. Read once per list, not per row.
    func agentListFilter() -> (Computer, Home.Workspace) -> Bool {
        let shown = tagFilter.shown(among: computers)
        let scope = agentScope
        return { computer, workspace in
            shown.contains(computer.device) && scope?.contains(workspace, on: computer.device) != false
        }
    }

    /// The homes of the computers the tag filter leaves, for the scopes to
    /// offer.
    var shownHomes: [(device: String, home: Home)] {
        let shown = tagFilter.shown(among: computers)
        return homes { shown.contains($0) }
    }

    private func homes(_ include: (String) -> Bool) -> [(device: String, home: Home)] {
        fleet.machines.compactMap { machine in
            guard include(machine.id), let home = machine.home else { return nil }
            return (machine.id, home)
        }
    }
}

/// Chooses the repository, or one of its workspaces, whose agents Home and
/// the iPad sidebar list.
struct AgentScopeMenu: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let active = model.agentScope
        let choices = AgentScope.choices(among: model.shownHomes)
        Menu {
            option("All Repositories", scope: nil, active: active)
            Section {
                ForEach(choices) { choice in
                    let chosen = active == choice.scope || choice.places.contains { $0.scope == active }
                    Menu {
                        option("All of \(choice.name)", scope: choice.scope, active: active)
                        Section {
                            ForEach(choice.places) { place in
                                option(label(place, in: choice), scope: place.scope, active: active)
                            }
                        }
                    } label: {
                        Label(choice.name, systemImage: chosen ? "checkmark" : "folder")
                    }
                }
            }
        } label: {
            Image(systemName: active == nil ? "line.3.horizontal.decrease.circle" : "line.3.horizontal.decrease.circle.fill")
        }
        .accessibilityLabel("Filter agents by repository or workspace")
        .disabled(choices.isEmpty && active == nil)
    }

    private func option(_ title: String, scope: AgentScope?, active: AgentScope?) -> some View {
        Toggle(title, isOn: Binding(
            get: { active == scope },
            set: { on in withAnimation(.snappy) { model.agentScopeStore.scope = on ? scope : nil } }
        ))
    }

    /// The workspace's branch when it is the repository's own checkout, as
    /// the Workspaces tab shows it, and its computer when the repository is
    /// on more than one.
    private func label(_ place: AgentScope.Place, in choice: AgentScope.Choice) -> String {
        let workspace = place.workspace
        let name = workspace.label == choice.name ? workspace.branch ?? workspace.label : workspace.label
        guard Set(choice.places.map(\.device)).count > 1,
              let computer = model.computers.first(where: { $0.device == place.device }) else { return name }
        return "\(name) · \(model.displayName(computer))"
    }
}

/// The chosen repository or workspace above an agent list, a tap away from
/// showing them all again.
struct AgentScopeBar: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let scope = model.agentScope {
            Section {
                Button {
                    withAnimation(.snappy) { model.agentScopeStore.scope = nil }
                } label: {
                    TagPill(tag: title(scope), selected: true, symbol: "xmark", icon: icon(scope), neutral: true)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Show agents of every repository")
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 0, leading: 4, bottom: 0, trailing: 4))
            }
        }
    }

    private func title(_ scope: AgentScope) -> String {
        switch scope {
        case let .repository(name):
            return name
        case let .workspace(device, id):
            let workspace = model.fleet.machines.first { $0.id == device }?.home?.workspaces.first { $0.id == id }
            guard let workspace else { return id }
            return workspace.label == workspace.repositoryName ? workspace.label : "\(workspace.repositoryName) / \(workspace.label)"
        }
    }

    private func icon(_ scope: AgentScope) -> String {
        switch scope {
        case .repository: "folder"
        case .workspace: "square.stack.3d.up"
        }
    }
}
