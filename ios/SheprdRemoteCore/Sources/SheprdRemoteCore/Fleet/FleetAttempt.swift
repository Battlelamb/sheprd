import Foundation

/// How posting to a fleet's computers, one after another, ended.
public enum FleetAttempt<Candidate: Sendable> {
    /// `Candidate` took it. Failures of the ones tried before no longer
    /// matter.
    case done(Candidate)
    /// `Candidate`'s fleet checked it and refused this many: every other
    /// computer of the fleet would refuse them too.
    case refused(Candidate, Int)
    /// None took it; the last failure, when any computer was tried.
    case failed(Error?)

    /// Tries `candidates` in order until one takes it, or the fleet
    /// refuses. `failed` hears each computer that did not answer. Runs on
    /// the caller's actor, so the closures may touch its state.
    public static func first(
        _ candidates: [Candidate],
        isolation: isolated (any Actor)? = #isolation,
        _ attempt: (Candidate) async throws -> Void,
        failed: (Candidate, Error) -> Void = { _, _ in }
    ) async -> FleetAttempt<Candidate> {
        var last: Error?
        for candidate in candidates {
            do {
                try await attempt(candidate)
                return .done(candidate)
            } catch FleetClientError.refused(let count) {
                return .refused(candidate, count)
            } catch {
                failed(candidate, error)
                last = error
            }
        }
        return .failed(last)
    }
}

/// Whether a request may have been taken though its answer said otherwise
/// or never came: `status` is the HTTP status answered, nil when none came
/// back. A refusal (4xx) means the computer did not take it; no answer, or
/// a failure on its side, may hide one that did, so what the request
/// carries (a join's keys, a link's) is kept until the fleet says.
public func fleetRequestMayHaveLanded(status: Int?) -> Bool {
    guard let status else { return true }
    return status >= 500
}
