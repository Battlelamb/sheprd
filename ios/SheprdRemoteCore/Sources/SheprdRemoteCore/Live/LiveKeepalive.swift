/// Whether a live socket's path still carries traffic.
public enum LiveKeepalive {
    /// What a keepalive round found.
    public enum Finding: Equatable, Sendable {
        /// The pong came in time.
        case answered
        /// The pong was late, but the address answered a call of its own:
        /// the socket is busy, such as with a large chat snapshot, which a
        /// pong cannot overtake on the wire.
        case busy
        /// Neither came back: the path stopped carrying traffic.
        case dead
    }

    /// Waits for `pong`, and only when it does not come, asks `probe`,
    /// which reaches the same address over a connection of its own.
    public static func check(
        pong: () async -> Bool,
        probe: () async -> Bool
    ) async -> Finding {
        if await pong() { return .answered }
        return await probe() ? .busy : .dead
    }
}
