import SwiftUI

/// A capsule along the top saying what an agent on a computer showed or
/// opened (`AgentControl`): its note, or that it opened the screen now in
/// front, so the screen never changes unexplained. It goes by itself; a
/// tap sends it sooner. It never takes input beyond that tap.
struct AgentBanner: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        let banner = model.agents.banner
        Group {
            if let banner {
                content(banner)
                    .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(.spring(duration: 0.35), value: banner)
        .task(id: banner?.id) {
            guard let banner else { return }
            try? await Task.sleep(for: banner.shown)
            if !Task.isCancelled, model.agents.banner?.id == banner.id { model.agents.banner = nil }
        }
    }

    private func content(_ banner: AgentControl.Banner) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: banner.note ? "text.bubble" : "sparkles")
                .foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 2) {
                if banner.note {
                    Text("From \(banner.computer)'s agent")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                }
                Text(banner.text)
                    .font(.subheadline)
                    .lineLimit(banner.note ? 6 : 1)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .glassEffect(in: .rect(cornerRadius: 18))
        .padding(.horizontal, 16)
        .padding(.top, 4)
        .contentShape(.rect)
        .onTapGesture { model.agents.banner = nil }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityHint("Dismisses it")
    }
}
