# Recorded T3 Code frames

Server frames from a sandboxed T3 Code 0.1.7 server (`t3 serve` with a
private home, loopback only, telemetry off), driven by a fake ACP agent that
echoes each prompt and makes one read tool call. No real agent, account, or
user data is involved; tokens and tickets were never written here.

- `snapshots.json`: the thread list and one finished thread, as
  `orchestration.subscribeShell` (request 1) and
  `orchestration.subscribeThread` (request 2) first answer.
- `live-turn.json`: the same two subscriptions while a second message is
  dispatched (request 3): every event of that turn, then the thread list's
  updates.

Re-record them against the version `sheprd-t3` pins when it changes.

## `steer.json`

One session acting in a thread, recorded from T3 Code 0.1.9 (`t3 serve
--host 127.0.0.1 --no-browser`, a private `T3CODE_HOME` and `HOME` under
`/tmp`, `T3CODE_TELEMETRY_ENABLED=false`, a cleared environment). It is a
JSON array of every client `Request` and every server frame, in order, each
marked `"dir": "client"` or `"dir": "server"`; the client's `Ack`s are left
out, and its last frame is the `Interrupt` that ends the subscription.

The agent is a fake ACP stdio agent, registered in the sandbox's
`userdata/settings.json` as two `providerInstances` of the `acpRegistry`
driver, `fakeacp` and `fakeacp2`, each with `config: {"source": "local",
"commandPath": <python3>, "commandArgs": [<script>, <name>]}`. It echoes
each prompt, asks `session/request_permission` (`allow_once`,
`allow_always`, `reject_once`) for a prompt saying "permission", streams
for about ten seconds for one saying "slow", advertises the models `fast`
and `smart` as a `model` select config option, and supports `session/load`,
which T3 needs to reopen a session. The thread was started beforehand with
`t3 thread start --provider fakeacp --model fast` (hence its `mcp:` id and
first "hello" turn).

| Request | What |
| --- | --- |
| 1 | `server.getConfig`: `providers` intact; `keybindings` and `settings` removed, and the host's label, editors, and `remoteOpenTargets` replaced |
| 2 | `thread.runtime-mode.set` to `approval-required` (the CLI starts threads in `full-access`, which answers permissions itself) |
| 3 | `orchestration.subscribeThread`: the snapshot, then every event below |
| 4 | `message.dispatch` "please ask permission", until an `approval_request` turn item waits |
| 5 | `runtime-request.respond` with `decision: "accept"`, until the run completes |
| 6 | Request 4 again, same `commandId`: T3 answers request 4's `{sequence}` again and emits nothing |
| 7 | `runtime-request.respond` for an unknown request: a `Failure` exit |
| 8 | `message.dispatch` "go slow please" |
| 9 | `message.dispatch` with `queue_after_active` while it runs: a `queued` run |
| 10 | `run.interrupt` with `holdQueue: true`: the slow run ends `interrupted`, the queued one stays held |
| 11 | `thread.model-selection.set` to `fakeacp`/`smart` |
| 12 | `provider.switch` to `fakeacp2`/`fast`: the provider session is detached |

Only the layout was compacted to keep the file short: small values, the
repeated `capabilities` blocks, and the payloads of bookkeeping events
(provider sessions, threads and turns, run attempts, nodes, checkpoints)
sit on one line. Every value is as the server sent it, apart from
`getConfig`'s trimming above. No real agent, account, token, ticket,
pairing code, or user data is involved.
