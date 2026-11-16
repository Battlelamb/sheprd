import Foundation
import Testing
@testable import SheprdRemoteCore

/// A fork's checkout: the computer names the upstream repository and the
/// checkout's commit, and the pull request's head belongs to the fork.
struct PullRequestForkTests {
    private let oid = "0123456789abcdef0123456789abcdef01234567"

    private func parse(_ name: String, headOid: String?) throws -> GitHubPullRequest? {
        try PullRequestAnswer.parse(
            GitHubFixtures.response(name),
            query: PullRequestQuery(owner: "upstream", repo: "app", branches: ["feature"]),
            headOwner: { _ in "upstream" }, headOid: { _ in headOid },
            host: GitHubFixtures.host, now: GitHubFixtures.start
        )["feature"]?.get()
    }

    @Test func theCheckoutsCommitFindsAForksPullRequest() throws {
        #expect(try parse("open_pr", headOid: oid)?.number == 42)
    }

    @Test func anotherOwnersHeadAtAnotherCommitIsNotTheBranchs() throws {
        #expect(throws: GitHubError.malformed) { try parse("open_pr", headOid: String(repeating: "f", count: 40)) }
        #expect(throws: GitHubError.malformed) { try parse("open_pr", headOid: nil) }
    }

    @Test func twoForksAtTheSameCommitStayAmbiguous() throws {
        #expect(throws: GitHubError.ambiguous) {
            try PullRequestAnswer.parse(
                GitHubFixtures.response("forks"),
                query: PullRequestQuery(owner: "acme", repo: "app", branches: ["main"]),
                headOwner: { _ in "nobody" }, headOid: { _ in oid },
                host: GitHubFixtures.host, now: GitHubFixtures.start
            )["main"]?.get()
        }
    }
}
