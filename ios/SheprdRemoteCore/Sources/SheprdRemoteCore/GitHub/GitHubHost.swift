import Foundation

extension Home {
    /// A GitHub account a computer lends this phone: `github_hosts` in its
    /// `home`.
    public struct GitHubAccount: Codable, Hashable, Sendable {
        public var host: String
        public var login: String

        public init(host: String, login: String) {
            self.host = host
            self.login = login
        }
    }

    /// Where a workspace's pull request lives: a fork with an `upstream`
    /// remote names the upstream repository.
    public struct GitHubRepository: Codable, Hashable, Sendable {
        public var host: String
        public var owner: String
        public var repo: String
        /// The branch its pull request's head is; absent on a detached HEAD.
        public var branch: String?
        /// The checkout's HEAD commit (40 or 64 hex digits), as the computer
        /// reports it; absent on an empty repository.
        public var head: String?
        /// Who owns the repository `branch` is pushed to, when that is not
        /// `owner`: a fork whose pull request targets the upstream named
        /// here. Absent from older computers, which read as `owner`.
        public var headOwner: String?

        enum CodingKeys: String, CodingKey {
            case host, owner, repo, branch, head
            case headOwner = "head_owner"
        }

        public init(host: String, owner: String, repo: String, branch: String? = nil, head: String? = nil, headOwner: String? = nil) {
            self.host = host
            self.owner = owner
            self.repo = repo
            self.branch = branch
            self.head = head
            self.headOwner = headOwner
        }

        /// Who owns the pull request's head, and its branch: the fork in
        /// `headOwner`, else this repository's owner, and `branch`.
        public var headRef: (owner: String, branch: String)? {
            guard let branch, !branch.isEmpty else { return nil }
            let fork = headOwner.flatMap { $0.isEmpty ? nil : $0 }
            return (fork ?? owner, branch)
        }
    }
}

/// A GitHub host and the addresses its APIs answer on: github.com, a
/// GHE.com subdomain, or a GitHub Enterprise Server.
public struct GitHubHost: Hashable, Sendable, Codable, CustomStringConvertible {
    /// The host's name in lowercase, as a computer reports it and as people
    /// browse it: `github.com`, `acme.ghe.com`, `github.example.com`.
    public let name: String
    /// Where REST paths such as `/repos/...` go.
    public let restBase: URL
    /// Where GraphQL queries go.
    public let graphQL: URL

    /// Accepts a bare host name, with a port for a server that uses one;
    /// nothing a URL could read as a scheme, user, path, or query.
    public init?(_ text: String) {
        var name = text.lowercased()
        if name.hasSuffix(".") { name.removeLast() }
        let allowed = Set("abcdefghijklmnopqrstuvwxyz0123456789.-:")
        guard !name.isEmpty, name.count <= 253, name.allSatisfy(allowed.contains),
              !name.hasPrefix("."), !name.hasPrefix("-"), !name.contains(".."),
              name.filter({ $0 == ":" }).count <= 1
        else { return nil }
        // GitHub's API host stands for github.com itself.
        if name == "api.github.com" || name == "www.github.com" { name = "github.com" }
        let rest: String
        let graphQL: String
        if name == "github.com" {
            (rest, graphQL) = ("https://api.github.com", "https://api.github.com/graphql")
        } else if let sub = Self.gheSubdomain(name) {
            (rest, graphQL) = ("https://api.\(sub).ghe.com", "https://api.\(sub).ghe.com/graphql")
        } else {
            (rest, graphQL) = ("https://\(name)/api/v3", "https://\(name)/api/graphql")
        }
        guard let restURL = URL(string: rest), restURL.host() != nil, let graphQLURL = URL(string: graphQL) else { return nil }
        self.name = name
        restBase = restURL
        self.graphQL = graphQLURL
    }

    public var description: String { name }

    /// The subdomain of a GHE.com host, `SUB.ghe.com`.
    private static func gheSubdomain(_ name: String) -> String? {
        guard name.hasSuffix(".ghe.com") else { return nil }
        let sub = name.dropLast(".ghe.com".count)
        return sub.isEmpty || sub.contains(".") || sub.contains(":") ? nil : String(sub)
    }

    /// Where people read pull requests and check runs.
    public var web: String { "https://\(name)" }

    public init(from decoder: Decoder) throws {
        let text = try decoder.singleValueContainer().decode(String.self)
        guard let host = GitHubHost(text) else {
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "not a GitHub host"))
        }
        self = host
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(name)
    }

    public static func == (lhs: Self, rhs: Self) -> Bool { lhs.name == rhs.name }
    public func hash(into hasher: inout Hasher) { hasher.combine(name) }
}
