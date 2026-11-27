import SheprdRemoteCore
import SwiftUI

extension View {
    /// Opens a tapped `ScreenLink` on the computer `device`, the one whose
    /// chat shows it, and leaves every other link to the handler further
    /// out, so put it inside `browsesLinks`.
    func opensScreenLinks(on device: String) -> some View {
        modifier(ScreenLinkOpening(device: device))
    }
}

private struct ScreenLinkOpening: ViewModifier {
    let device: String
    @Environment(AppModel.self) private var model
    @Environment(\.openURL) private var outer

    func body(content: Content) -> some View {
        content.environment(\.openURL, OpenURLAction { url in
            guard let screen = ScreenLink.screen(url) else {
                outer(url)
                return .handled
            }
            // The person's own tap: no banner, and no need for "Let agents
            // open screens", which is about agents acting on their own.
            if !model.agents.open(screen, on: device, model: model) {
                model.message = "That is no longer on \(computerName)."
            }
            return .handled
        })
    }

    private var computerName: String {
        model.fleet.machines.first { $0.id == device }?.computer.name ?? "your computer"
    }
}
