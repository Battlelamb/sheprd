import SheprdRemoteCore
import SwiftUI

/// The relays a computer listens on, as its page lists them: what its
/// owner calls each one in `[remote] relays`, its description and tags,
/// and its address. Hidden while the computer listens on none.
struct ComputerRelaysSection: View {
    let record: RelayRecord

    var body: some View {
        Section {
            ForEach(record.relays, id: \.self) { relay in
                RelayRow(relay: relay, label: record.label(for: relay))
            }
        } header: {
            Text("Relays")
        } footer: {
            Text("When no address answers, this phone reaches the computer through the first relay that does. Relays see only encrypted traffic.")
        }
    }
}

private struct RelayRow: View {
    let relay: RelayEndpoint
    let label: RelayLabel?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let name = label?.name {
                Text(name).font(.body.weight(.medium))
                address.font(.caption.monospaced()).foregroundStyle(.secondary)
            } else {
                address.font(.body.monospaced())
            }
            if let description = label?.description {
                Text(description).font(.subheadline).foregroundStyle(.secondary)
            }
            if let tags = label?.tags, !tags.isEmpty {
                RelayTags(tags: tags)
            }
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .combine)
    }

    private var address: some View {
        Text(relay.host + (relay.port.map { ":\($0)" } ?? "") + relay.prefix)
            .textSelection(.enabled)
            .lineLimit(1)
            .truncationMode(.middle)
    }
}

/// A relay's tags as small capsules, wrapping onto more lines as needed.
private struct RelayTags: View {
    let tags: [String]

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 4) { capsules }
            VStack(alignment: .leading, spacing: 4) { capsules }
        }
    }

    private var capsules: some View {
        ForEach(tags, id: \.self) { tag in
            Text(tag)
                .font(.caption2.weight(.medium))
                .padding(.horizontal, 6).padding(.vertical, 2)
                .background(.tint.opacity(0.15), in: Capsule())
                .foregroundStyle(.tint)
                .lineLimit(1)
        }
    }
}
