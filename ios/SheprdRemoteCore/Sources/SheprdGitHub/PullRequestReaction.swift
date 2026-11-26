import Foundation

/// A GraphQL mutation that adds or removes an emoji reaction on a GitHub
/// subject (such as a pull request description) and reads back the updated
/// reaction groups. Identifiers travel as variables, never interpolated.
struct PullRequestReactionMutation: Sendable, Equatable {
    var subjectId: String
    var content: PullRequestDetail.ReactionContent
    var add: Bool

    var text: String {
        add ? Self.addText : Self.removeText
    }

    var body: Data {
        get throws {
            let variables: [String: Any] = ["subjectId": subjectId, "content": content.rawValue]
            return try JSONSerialization.data(withJSONObject: ["query": text, "variables": variables], options: [.sortedKeys])
        }
    }

    static let addText = """
    mutation($subjectId: ID!, $content: ReactionContent!) {
      addReaction(input: { subjectId: $subjectId, content: $content }) {
        subject { reactionGroups { content viewerHasReacted reactors { totalCount } } }
      }
    }
    """

    static let removeText = """
    mutation($subjectId: ID!, $content: ReactionContent!) {
      removeReaction(input: { subjectId: $subjectId, content: $content }) {
        subject { reactionGroups { content viewerHasReacted reactors { totalCount } } }
      }
    }
    """
}

/// Reading a `PullRequestReactionMutation`'s answer.
enum PullRequestReactionAnswer {
    static func parse(
        _ response: GitHubResponse,
        add: Bool,
        now: Date
    ) throws(GitHubError) -> [PullRequestDetail.ReactionGroup] {
        guard let answer = try? JSONDecoder().decode(Answer.self, from: response.body) else { throw .malformed }
        let payload = add ? answer.data?.addReaction : answer.data?.removeReaction
        guard let groups = payload?.subject?.reactionGroups else {
            let errors = answer.errors ?? []
            throw errors.isEmpty ? .malformed : PullRequestAnswer.failure(errors, response: response, now: now)
        }
        return PullRequestDetailAnswer.parseReactions(groups)
    }

    private struct Answer: Decodable {
        var data: DataField?
        var errors: [PullRequestAnswer.Answer.Failure]?

        struct DataField: Decodable {
            var addReaction: Payload?
            var removeReaction: Payload?
        }

        struct Payload: Decodable {
            var subject: Subject?
        }

        struct Subject: Decodable {
            var reactionGroups: [PullRequestDetailAnswer.ReactionGroupNode?]?
        }
    }
}

/// Why adding or removing a pull request reaction failed.
public enum PullRequestReactionError: Error, Equatable, Sendable, LocalizedError {
    case noAccount
    case forbidden
    case ssoRequired
    case rateLimited
    case failed

    public var errorDescription: String? {
        switch self {
        case .noAccount:
            "Sign in to GitHub or share a computer's sign-in to react."
        case .forbidden:
            "GitHub did not allow reacting with this sign-in."
        case .ssoRequired:
            "Authorize SSO for this organization on GitHub to react."
        case .rateLimited:
            "GitHub rate limit reached. Try again later."
        case .failed:
            "Could not update the reaction. Try again."
        }
    }

    init(_ error: GitHubError) {
        switch error {
        case .unauthorized: self = .noAccount
        case .forbidden: self = .forbidden
        case .ssoRequired: self = .ssoRequired
        case .rateLimited: self = .rateLimited
        case .notFound, .status, .malformed, .tooLarge, .location, .ambiguous: self = .failed
        }
    }
}
