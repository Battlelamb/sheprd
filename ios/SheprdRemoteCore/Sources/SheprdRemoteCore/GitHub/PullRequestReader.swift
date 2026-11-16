import Foundation

/// Who answered a read: GitHub itself, with a leased token, or a paired
/// computer, named so the screen can say "via NAME".
public enum ReadOrigin: Hashable, Sendable {
    case github
    case computer(String)
}

/// An answer and who gave it.
public struct Sourced<Value: Sendable>: Sendable {
    public var value: Value
    public var origin: ReadOrigin

    public init(_ value: Value, from origin: ReadOrigin) {
        self.value = value
        self.origin = origin
    }
}

/// The paired computers that may lend a token, and how to ask one.
public struct Lenders: Sendable {
    public var lenders: [GitHubLender]
    public var fetch: FetchLease

    public init(lenders: [GitHubLender], fetch: @escaping FetchLease) {
        self.lenders = lenders
        self.fetch = fetch
    }
}

/// Reads a workspace's pull request from GitHub when a computer lends a
/// token for its host, and otherwise, or when that read fails for any
/// reason (no lease, no access to the organization, SSO, the rate limit,
/// no network), asks the workspace's computer as before. Falling back is
/// quiet: only the answer's origin tells.
public actor PullRequestReader {
    private let leases: GitHubLeases
    private let api: GitHubAPI

    public init(leases: GitHubLeases, api: GitHubAPI) {
        self.leases = leases
        self.api = api
    }

    /// The app's: tokens in the Keychain, GitHub over the network.
    public static func live() -> PullRequestReader {
        PullRequestReader(
            leases: GitHubLeases(store: TokenStore(vault: Keychain.vault(service: TokenStore.service))),
            api: .live()
        )
    }

    public var tokens: GitHubLeases { leases }

    /// The checks of `workspace`'s pull request.
    public func checks(
        workspace: Home.Workspace,
        computer: String,
        source: Lenders,
        askComputer: @Sendable () async throws -> PullRequestChecks
    ) async throws -> Sourced<PullRequestChecks> {
        if let repository = workspace.github, let host = GitHubHost(repository.host), let head = repository.headRef {
            let direct = await attempt(host: host, source: source) { api, token in
                let found = try await api.pullRequests(
                    host: host, owner: repository.owner, repo: repository.repo,
                    branches: [head.branch], headOwner: { _ in head.owner }, headOid: { _ in repository.head }, token: token
                )
                guard let answer = found[head.branch] else { throw GitHubError.malformed }
                return try answer.get()?.checkList ?? PullRequestChecks(checks: [])
            }
            if let direct { return Sourced(direct, from: .github) }
        }
        return Sourced(try await askComputer(), from: .computer(computer))
    }

    /// The excerpt of a failing Actions job's log.
    public func checkLog(
        job: UInt64,
        workspace: Home.Workspace,
        computer: String,
        source: Lenders,
        askComputer: @Sendable () async throws -> CheckLogExcerpt
    ) async throws -> Sourced<CheckLogExcerpt> {
        if let repository = workspace.github, let host = GitHubHost(repository.host) {
            let direct = await attempt(host: host, source: source) { api, token in
                try await api.jobLog(host: host, owner: repository.owner, repo: repository.repo, job: job, token: token)
            }
            if let direct { return Sourced(CheckLogExcerpt(excerpt: direct), from: .github) }
        }
        return Sourced(try await askComputer(), from: .computer(computer))
    }

    /// The pull requests of `workspaces` that GitHub can be asked about
    /// directly, by workspace id: one query per repository for all their
    /// branches. Workspaces left out had no lease or failed, and keep the
    /// computer's badge.
    public func pullRequests(workspaces: [Home.Workspace], source: Lenders) async -> [String: GitHubPullRequest?] {
        struct Repository: Hashable {
            var host: GitHubHost
            var owner: String
            var repo: String
        }
        var groups: [Repository: [(id: String, head: (owner: String, branch: String), oid: String?)]] = [:]
        for workspace in workspaces {
            guard let github = workspace.github, let host = GitHubHost(github.host), let head = github.headRef else { continue }
            groups[Repository(host: host, owner: github.owner, repo: github.repo), default: []].append((workspace.id, head, github.head))
        }
        var found: [String: GitHubPullRequest?] = [:]
        for (repository, members) in groups {
            // A branch's head owner, for branches several forks share.
            let owners = Dictionary(members.map { ($0.head.branch, $0.head.owner) }) { first, _ in first }
            let oids = Dictionary(members.compactMap { member in member.oid.map { (member.head.branch, $0) } }) { first, _ in first }
            let answers = await attempt(host: repository.host, source: source) { api, token in
                try await api.pullRequests(
                    host: repository.host, owner: repository.owner, repo: repository.repo,
                    branches: members.map(\.head.branch), headOwner: { owners[$0] ?? repository.owner }, headOid: { oids[$0] }, token: token
                )
            }
            for member in members {
                if case let .success(pullRequest)? = answers?[member.head.branch] { found[member.id] = pullRequest }
            }
        }
        return found
    }

    /// `read` with a lease for `host`, once more with a new one when GitHub
    /// refuses the first (401); nil when there is none or the read fails,
    /// for the computer to answer instead.
    private func attempt<Value: Sendable>(
        host: GitHubHost,
        source: Lenders,
        _ read: @Sendable (GitHubAPI, GitHubToken) async throws -> Value
    ) async -> Value? {
        for _ in 0..<2 {
            guard !Task.isCancelled,
                  let lease = try? await leases.lease(for: host, lenders: source.lenders, fetch: source.fetch)
            else { return nil }
            do {
                return try await read(api, lease.token)
            } catch GitHubError.unauthorized {
                await leases.rejected(lease)
            } catch {
                return nil
            }
        }
        return nil
    }
}
