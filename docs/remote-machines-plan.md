# Virtual Macs on other computers

**Plan; nothing here is built.** macOS runs at most two virtual Macs per
computer, so two agents testing in VMs fill this Mac. This plan lets an
agent use a free virtual Mac on another Mac the window already reaches,
with the same `sheprd machine` commands and the same claims.

## Where things stand

- A virtual Mac runs inside the Sheprd app that started it
  (`crates/sheprd-gpui/src/machine/vm.rs`, Virtualization.framework). Its
  picture (`VZVirtualMachineView`) and its vsock device, which Sheprd Agent
  answers on (`machine/agent.rs`), exist only in that process.
- `MAX_ACTIVE_MACHINES = 2` (`sheprd-control-protocol`, `protocol/tabs.rs`)
  and `vm::capacity` count only this app's machines and a live installer.
  VMs of other programs, such as the Tart CI runners on the Mac minis
  (`docs/self-hosted-runners.md`), take the same slots unseen; starting
  then fails in the framework.
- Claims are kept by the running app, per Herdr pane or core terminal
  (`control/server/tabs/machine_claims.rs`). A caller without either
  cannot claim.
- The app already runs another machine's Sheprd command line over its
  host connection and reads its `--json` answer: `remote_update/fetch.rs`
  (`script`, the exit-code table, bounded output, strict parsing).

## Decisions

| Question | Decision |
| --- | --- |
| Who runs a remote VM | That Mac's own Sheprd. Nothing here can run it: the framework, the view, and vsock are per process. |
| How this app reaches it | `sheprd machine ... --json` over the existing host connection (SSH, through relays when needed), as `remote_update` runs `sheprd update`. No new daemon, transport, or crate. |
| Why not the remote control API | Fleet members do not get the remote control API on each other's daemons (`docs/relay-plan.md`, What joining grants). SSH is the access the user already granted. |
| Who may borrow | Every Mac lends by default: `[machine] lend`, on unless set off (Settings > Virtual Macs). A Mac that runs VMs of other programs, such as Tart CI runners, never lends, whatever the setting: their VMs share its two slots (Hosts that never lend). |
| Who decides to borrow | The user, once per host. The first time placement would use another Mac, the app asks: "Virtual Macs here are full. Use a free slot on mini-2?" with **Use once**, **Always use mini-2**, and **Wait for a slot here**. An agent cannot answer it. |
| What is copied | Nothing by default: each Mac makes machines from its own images. When the only way is copying an image, the app measures the link first and asks, with the size and the time it would take (Copying an image). |
| Remote app not running | Skipped by placement; a named host answers exit 3, "Sheprd is not running on mini-2". Never launched over SSH: it needs the user's GUI login. |

## Commands

Every `machine` subcommand takes `--host HOST`, a host ID or label as
`host update` takes. Without it, nothing changes: this computer.

| Command | Change |
| --- | --- |
| `machine list --all` | Each lending host's capacity and machines, this computer first. A host that does not answer is listed with its reason, not left out. |
| `machine claim --any [--image IMAGE] [--wait S]` | Places the claim: an unreserved running machine here, then a stopped one with a free slot here, then the same on lending hosts, most free slots first. Prints the host and name it got. `--wait` polls every host every five seconds from this app, not one long SSH command. |
| `machine start NAME` | Boots without a tab, which is what a forwarded `open` does: nobody is at the other Mac. Locally, `open` keeps opening a tab. |
| `machine screenshot NAME -` | Writes the PNG to stdout, so a forwarded screenshot comes back in one exchange. The local side bounds it (64 MiB, as the agent's reply), checks it is a PNG of the size claimed, and writes PATH here. |

Claims, `exec`, `click`, `type`, `key`, `agent`, `release`, and `set`
forward unchanged apart from the owner below. Exit codes stay those of
`docs/cli.md`; a host that cannot be reached is exit 8.

## Asking before borrowing

Borrowing uses another computer's memory, CPU, and one of its two slots,
so it is confirmed in the app's own dialog, as other outward actions are
(`AGENTS.md`, Command Line). The dialog names the host, the machine, its
CPUs and memory, and how the host is reached (direct, LAN, relay).

- **Use once** places this claim there.
- **Always use mini-2** is kept per host (`[machine] borrow_from`), so
  later claims go there without asking. Settings > Virtual Macs lists them
  and removes them.
- **Wait for a slot here** turns the claim into `--wait` on this Mac.

Until the user answers, `claim --any` prints "Virtual Macs here are full;
asking to use mini-2" on stderr every five seconds, as `--wait` does, and
exits 6 if the dialog is dismissed. The skill tells the agent to relay that
line to the user, who may be in the chat rather than at the window.

## Hosts that never lend

A Mac whose slots other programs use cannot keep its promise: placement
would see a free slot that a CI job takes a minute later. So a host does
not lend, and `list --all` says why, when any of these holds:

- Another program's VM is running (Counting every VM on a host).
- Tart has VMs (`tart list` answers with any, or `~/.tart/vms` is not
  empty), even when none runs: a CI runner starts them on demand.
- `[machine] lend = false`.

The check runs on the lending Mac when asked, so a forwarded claim is
refused there (exit 6, "mini-2 runs Tart VMs and does not lend") even if
this app's cached answer was older.

## Owners across computers

A forwarded command arrives over SSH, with no pane. It carries a hidden
`--owner remote:COMPUTER:PANE` (or `:TERMINAL`), where `COMPUTER` is this
app's fleet or device ID. The lending app:

- Accepts `--owner` only from a caller with no pane or terminal and only
  while it lends, so a local agent cannot pose as another pane.
- Keeps it as its own `Owner` variant, which never equals a local one.
- Applies the same rules: 15 idle minutes, renewed by guest commands, only
  the owner releases.

SSH as that account already controls the app, so this grants nothing new.
A borrower that quits leaves its claims to expire; the VM keeps running.

## Counting every VM on a host

Placement trusts `capacity`, so it must count all VMs, not only Sheprd's.
Each running Virtualization.framework VM is one
`com.apple.Virtualization.VirtualMachine` XPC process (to be confirmed on
macOS 26). `capacity` reports `in_use` from those, plus Sheprd's own
starting machines and installer, with `others` naming how many belong to
other programs. A start the framework refuses for the limit becomes
`Error::Capacity`, so placement moves on rather than failing the claim.

## Watching a remote machine

A tab for a machine on another Mac, from the start. The real view cannot
cross processes, so the tab draws the guest's screen from Sheprd Agent and
sends input back through it, as `machine click`, `type`, and `key` do.

- One long-lived forwarded command, `machine watch NAME --owner ...`, on
  the lending Mac streams frames on stdout and reads input on stdin, framed
  as the agent's wire is (4-byte length, JSON or PNG). One SSH channel, not
  a command per frame.
- Sheprd Agent gains a `frame` request: the screen scaled to a width the
  watcher asks for, as JPEG, and nothing when it has not changed since the
  last frame. Kept in step with `macos/sheprd-machine-agent/main.swift`.
- The watcher asks for frames one at a time, so a slow link lowers the
  frame rate instead of queuing: at most one frame in flight, the latest
  one drawn. It starts at the tab's width and halves it while frames take
  longer than half a second.
- Watching does not need the claim; input does. A tab without the claim
  shows the screen read-only, and says who holds it.
- The tab says it is remote and how it is reached, and shows the frame
  rate when it falls below a few per second.

## Copying an image

Only when the user asks for a machine from an image the other Mac lacks.
A used image is 20–40 GB even sparse, so it is never copied without the
user's answer.

1. **Measure.** Before asking, send 16 MB through the same SSH channel and
   time it, after a 2 MB warm-up. Read how the host is reached: direct on
   the LAN, Tailscale (direct or DERP), or a Sheprd relay. Keep the result
   for ten minutes per host.
2. **Ask.** The dialog says the image, the bytes to send, the measured
   speed, and the time it would take: "Copy macos-26-agent (24 GB) to
   mini-2? About 3 minutes over your network." Over a relay, Tailscale's
   DERP, or an estimate beyond 30 minutes it warns instead, "About 4 hours
   over a relay", and makes **Wait for a slot here** the default button.
   Below 1 MB/s it refuses and says why.
3. **Copy.** Only the blocks the disk uses (it is sparse), compressed,
   with progress in the app and a checksum checked on arrival before the
   image is listed there. Cancel leaves nothing behind on either side.

The measuring and asking come before any byte of the image is sent;
nothing here retries on a slower path by itself.

## Code

| Piece | Where |
| --- | --- |
| `host: Option<String>` on machine requests, `claim --any`, `list --all`, `start`, screenshot to stdout, `--owner` | `sheprd-control-protocol` `protocol/tabs.rs`, `control/client/tabs*` |
| Forwarding: resolve the host, run the script, parse the answer, map exit codes | new `control/server/tabs/machine_remote.rs`, reusing `remote_update::fetch::script` (moved to a shared module, not copied) |
| Lending hosts and their last answer, read on demand and at most every few minutes | new `machine/hosts.rs`, off the UI thread |
| Placement order | pure function in `machine/placement.rs`, tested without hosts |
| Remote owners | `machine_claims.rs` `Owner` |
| All-VM count | `machine/vm.rs` `capacity` |
| `lend` and `borrow_from` settings, hosts that never lend | `machine/settings.rs`, new `machine/lend.rs`, Settings > Virtual Macs |
| The borrow and copy dialogs | new `machine/borrow.rs` |
| Remote watching tab and `machine watch` | new `machine/watch.rs`; `frame` in `machine/agent/wire.rs` and the Swift agent |
| Link measurement and image copy | new `machine/transfer.rs`, off the UI thread |

Tests: placement order and ties; owner spoofing refused from a pane and
while not lending; forwarded exit codes and bounds against a fake host
script; screenshot over stdout rejected when oversized or not a PNG;
capacity with other programs' VMs; a Tart host refusing a forwarded claim;
the borrow dialog's three answers and dismissal; the watcher keeping one
frame in flight and dropping stale ones; the copy refused below its speed
floor and cleaned up after cancel.

## Skill and docs

`skills/herdr-gpui-browser/SKILL.md`, "Sharing the two macOS VMs":
`machine claim --any`, and when both slots here are full, tell the user
which host it got ("No local VM slots; using `default` on mini-2"). Keep
"never bypass a busy claim". `docs/cli.md` gets the rows above.

## Phases

1. **Forwarding and watching.** `--host` on every subcommand, remote
   owners, `start`, screenshot over stdout, `lend` with the hosts that
   never lend, the borrow dialog, and the remote watching tab. An agent can
   name a host by hand, and the user can see what it does there.
2. **Placement.** All-VM counting, `list --all`, `claim --any [--wait]`,
   and the skill's new flow.
3. **Copying images.** Measurement, the copy dialog, and the copy.
