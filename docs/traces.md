# Traces

Open **Traces** in the activity bar, or run `sheprd ui panel traces`, to
browse this computer's Claude Code, Codex, Pi, and omp session history.
The page shows session totals, messages per UTC day, agent and date filters,
and a search over session titles, folders, branches, models, and computer names.
Select a session to read its prompts, replies, thinking, tool calls, and results.

The app indexes native JSONL session files on a background thread and follows
changes as agents write. It reads Claude's `~/.claude/projects`, Codex's
`~/.codex/sessions` and `~/.codex/archived_sessions`, Pi's
`~/.pi/agent/sessions`, and omp's `~/.omp/agent/sessions`.
The index is private to this user; the page currently reads only the local
computer. It does not upload or share sessions.

Event text and titles pass through the diagnostics redactor before storage.
Events are limited to 4 KB in the index and shortened to 600 bytes in the view;
the original files remain with the agent. The page reads up to 1,000 recent
sessions, draws up to 200 matching rows, and shows the first 400 events of a
selected session. Search filters session metadata, not the full transcript.
Totals cover the matching sessions' lifetime counts; the chart covers messages
in the selected date range and agent, independently of the text search.

## Host command line

The companion can maintain and query the same index without the GUI:

```sh
sheprd-acp traces index --json
sheprd-acp traces watch --json
sheprd-acp traces list --agent claude --limit 50 --json
sheprd-acp traces activity --days 30 --json
```

`index` reads one bounded batch per file; repeat it or use `watch` to catch up
with very large files. `list` and `activity` read the existing index.
Without `--json`, the commands print human-readable summaries.
`watch --json` emits a host record, changed session records, and quiet-rescan
heartbeats as JSON lines. It runs until stopped or its output reader closes.

## Native screenshots

The opt-in fixture uses synthetic data and the real native window, with the
shared test sandbox. With a desktop available, capture dark, light, narrow,
and session-detail views:

```sh
HERDR_TEST_TRACES_CAPTURE=/absolute/existing/directory/traces \
  cargo test --locked -p sheprd-gpui --features integration-test,mockup \
  --test live_gui native_traces -- --ignored --nocapture --test-threads=1
```

This exercises native rendering; it does not verify compositor-delivered
keyboard input or IME.

On Linux, captures use X11 and require Python's Pillow and python-xlib packages.
Unset `WAYLAND_DISPLAY` for the command and supply the desktop's `DISPLAY` and
`XAUTHORITY` if running over SSH. Only the fixture process's window is captured.
