import Testing
@testable import SheprdGitHub

/// Reads of the pull request lists: joined while they ask the same, and
/// replaced once what they ask, or the account, changes.
@MainActor
struct LatestReadTests {
    /// What the reads did, in order.
    @MainActor
    final class Log {
        var started: [String] = []
        var kept: [String] = []
    }

    private static func read(_ name: String, _ log: Log) -> @MainActor @Sendable () async -> Void {
        {
            log.started.append(name)
            try? await Task.sleep(for: .milliseconds(100))
            if !Task.isCancelled { log.kept.append(name) }
        }
    }

    @Test func theSameKeyJoinsTheReadUnderWay() async {
        let reads = LatestRead<String>()
        let log = Log()
        async let first: Void = reads.run("open", Self.read("a", log))
        async let second: Void = reads.run("open", Self.read("b", log))
        _ = await (first, second)
        #expect(log.started == ["a"])
        #expect(log.kept == ["a"])
        #expect(!reads.isRunning)
    }

    @Test func anotherKeyReplacesTheReadUnderWay() async {
        let reads = LatestRead<String>()
        let log = Log()
        async let first: Void = reads.run("lent account", Self.read("a", log))
        await Task.yield()
        async let second: Void = reads.run("own account", Self.read("b", log))
        _ = await (first, second)
        #expect(log.started == ["a", "b"])
        // The replaced read's answer is never kept.
        #expect(log.kept == ["b"])
        #expect(!reads.isRunning)
    }

    @Test func aReplacedReadsCallerSeesTheNewOneRunning() async {
        let reads = LatestRead<Int>()
        let log = Log()
        let first = Task { @MainActor in
            await reads.run(1, Self.read("a", log))
            return reads.isRunning
        }
        await Task.yield()
        await reads.run(2, Self.read("b", log))
        let runningAfterFirst = await first.value
        #expect(runningAfterFirst == true)
        #expect(!reads.isRunning)
    }
}
