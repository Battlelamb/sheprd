import SwiftUI

/// With no computer paired, the first list says so and offers pairing one
/// or exploring the demo (`DemoFleet`); while the demo runs, a bar along
/// its bottom says the computers are made up and leads to pairing.
private struct DemoOffer: ViewModifier {
    @Environment(AppModel.self) private var model
    @Binding var pairing: Bool

    func body(content: Content) -> some View {
        content
            .overlay {
                if model.shownComputers.isEmpty {
                    ContentUnavailableView {
                        Label("A companion to Sheprd", systemImage: "desktopcomputer")
                    } description: {
                        // The app shows nothing on its own: say what else it needs.
                        Text("""
                        This app follows the agents Sheprd runs on your Mac, Linux, or Windows computer. \
                        Install Sheprd for free from [sheprd.org](https://sheprd.org), open Remote control \
                        > Pair device, then scan its code here.
                        """)
                    } actions: {
                        Button("Pair a Computer") { pairing = true }.buttonStyle(.glassProminent)
                        Button("Explore the Demo") {
                            withAnimation(.snappy) { model.startDemo() }
                        }
                        .buttonStyle(.glass)
                    }
                }
            }
            .safeAreaInset(edge: .bottom) {
                if model.fleet.demo, !Self.barHidden {
                    DemoBar(pairing: $pairing)
                        .transition(.move(edge: .bottom).combined(with: .opacity))
                }
            }
            .animation(.snappy, value: model.fleet.demo)
    }

    /// The store screenshots' `-hideDemoBar YES` (`tools/store_screenshots.sh`).
    private static var barHidden: Bool {
        #if DEBUG
        UserDefaults.standard.bool(forKey: "hideDemoBar")
        #else
        false
        #endif
    }
}

private struct DemoBar: View {
    @Environment(AppModel.self) private var model
    @Binding var pairing: Bool

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Demo").font(.subheadline.weight(.semibold))
                Text("Made-up computers. Nothing is sent.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Button("End") { model.endDemo() }
                .buttonStyle(.glass)
            Button("Pair") { pairing = true }
                .buttonStyle(.glassProminent)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .glassEffect(in: .rect(cornerRadius: 20))
        .padding(.horizontal, 12)
        .padding(.bottom, 6)
        .accessibilityElement(children: .contain)
    }
}

extension View {
    /// The empty list's pairing and demo offer, and the demo's bar.
    func demoOffer(pairing: Binding<Bool>) -> some View {
        modifier(DemoOffer(pairing: pairing))
    }
}
