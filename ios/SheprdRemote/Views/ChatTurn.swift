import SheprdRemoteCore
import SwiftUI

/// One turn of a conversation: the prompt as its heading, the steps folded
/// to one line, the answer, then the files it changed. Every turn reads the
/// same, so the conversation scrolls back through its history as a messages
/// app does.
struct ChatTurn: View {
    let entries: ArraySlice<Entry>

    /// A transcript's entries split where each prompt starts; whatever came
    /// before the first prompt is a turn of its own, without a heading.
    static func split(_ entries: [Entry]) -> [ArraySlice<Entry>] {
        var turns: [ArraySlice<Entry>] = []
        var start = entries.startIndex
        for index in entries.indices where index > start {
            if case .user = entries[index].kind {
                turns.append(entries[start ..< index])
                start = index
            }
        }
        if start < entries.endIndex { turns.append(entries[start...]) }
        return turns
    }

    var body: some View {
        let tools = entries.compactMap { if case let .tool(tool) = $0.kind { tool } else { nil } }
        let prompt: (text: String, rest: ArraySlice<Entry>)? = if case let .user(text, _) = entries.first?.kind {
            (text, entries.dropFirst())
        } else {
            nil
        }
        VStack(alignment: .leading, spacing: 16) {
            if let prompt {
                PromptHeading(text: prompt.text)
            }
            if !tools.isEmpty {
                DisclosureGroup {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(tools, id: \.callId) { ToolRow(tool: $0) }
                    }
                    .padding(.top, 6)
                } label: {
                    Label(Self.stepsLine(tools), systemImage: "list.bullet.indent")
                        .font(.subheadline).foregroundStyle(.secondary)
                }
                Divider()
            }
            ForEach(Array(prompt?.rest ?? entries)) { entry in
                if case .tool = entry.kind {} else { EntryView(entry: entry) }
            }
            ForEach(tools.flatMap(Self.diffs), id: \.path) { diff in
                DiffBlock(path: diff.path, old: diff.old, new: diff.new)
            }
        }
    }

    static func stepsLine(_ tools: [Tool]) -> String {
        let edited = Set(tools.filter { $0.kind == .edit }.flatMap(\.locations)).count
        let steps = "\(tools.count) step\(tools.count == 1 ? "" : "s")"
        return edited > 0 ? "\(steps) · \(edited) file\(edited == 1 ? "" : "s") changed" : steps
    }

    private static func diffs(_ tool: Tool) -> [(path: String, old: String?, new: String)] {
        tool.content.compactMap { if case let .diff(path, old, new) = $0 { (path, old, new) } else { nil } }
    }
}

/// A prompt as its turn's heading. A picture sent into a terminal reaches
/// the agent as a file path in the prompt; it shows as a picture mark, not
/// as the path.
private struct PromptHeading: View {
    let text: String

    var body: some View {
        let (words, pictures) = Self.pictures(in: text)
        HStack(alignment: .top, spacing: 10) {
            Rectangle().fill(.tint).frame(width: 3)
            VStack(alignment: .leading, spacing: 6) {
                if pictures > 0 {
                    Label(pictures == 1 ? "Picture" : "\(pictures) pictures", systemImage: "photo")
                        .font(.subheadline).foregroundStyle(.secondary)
                }
                if !words.isEmpty {
                    Text(words).font(.title3.weight(.semibold)).textSelection(.enabled)
                }
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private static let pictureTypes: Set<String> = ["gif", "heic", "jpeg", "jpg", "png", "webp"]

    /// The prompt without the absolute paths of pictures in it, and how many
    /// there were.
    static func pictures(in text: String) -> (String, Int) {
        var count = 0
        let kept = text.split(separator: " ", omittingEmptySubsequences: false).filter { word in
            let isPicture = word.hasPrefix("/")
                && !word.contains("\n")
                && pictureTypes.contains(word.split(separator: ".").last.map { $0.lowercased() } ?? "")
            if isPicture { count += 1 }
            return !isPicture
        }
        return (kept.joined(separator: " ").trimmingCharacters(in: .whitespacesAndNewlines), count)
    }
}
