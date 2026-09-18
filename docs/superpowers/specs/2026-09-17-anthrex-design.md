# anthrex — Design Spec

Date: 2026-09-17
Status: draft for review

## 1. Overview

anthrex is a terminal multiplexer built for running many coding agents at once. Each agent is a real interactive `claude` or `codex` TUI (or a plain shell) running in its own pseudo-terminal. A background daemon owns the terminals so agents survive closing or crashing the UI. A ratatui client shows a sidebar of agents with live status and the focused agent's terminal at full size, tmux-style.

### Goals

- Run and switch between many Claude Code and Codex sessions from one clean, beautiful terminal UI.
- See at a glance which agents are working, idle, waiting for approval, finished, or exited.
- Agents survive UI restarts and crashes (daemon model). Closing the UI detaches.
- Optional git worktree per agent so parallel agents do not clash on the same files.
- Fast: Rust, a single static binary, negligible overhead per agent.

### Non-goals (v1)

> Update 2026-09-18: split panes and automatic task decomposition are now goals, in milestones 7 to 9. See `docs/superpowers/specs/2026-09-18-anthrex-product-design.md` and `docs/ROADMAP.md`.

- Split panes showing several agents at once.
- Automatic task decomposition or planner agents.
- Scrollback carried across detach/attach (the client keeps scrollback only from the moment it attached).
- Multiple simultaneous clients at different sizes (last resize wins).
- Windows OS support. Remote daemons over TCP or SSH.

## 2. Architecture

### 2.1 Processes

```
 ┌──────────────┐  Unix socket   ┌──────────────────────────────────────────┐
 │ anthrex (TUI)│◄──────────────►│ anthrex daemon                           │
 │ ratatui      │ MessagePack    │  WindowManager ─┬─ Window 1: pty ─ claude │
 └──────────────┘                │                 ├─ Window 2: pty ─ codex  │
                                 │  StatusEngine   └─ Window 3: pty ─ zsh    │
 ┌──────────────┐                │  StateStore (state.json)                 │
 │ anthrex hook │───────────────►│  (hook events from claude / codex)       │
 └──────────────┘                └──────────────────────────────────────────┘
```

- `anthrex daemon`: one per user. Owns PTYs, child processes, status, the state file, and the Unix-socket server.
- `anthrex` (client): connects to the socket, auto-starts the daemon if it is not running, renders the UI. Detaching leaves everything running.
- `anthrex hook`: short-lived process invoked by Claude Code hooks and Codex notify. Forwards the event to the daemon and exits 0 no matter what.

### 2.2 Crates

One Cargo workspace, Rust edition 2024, MSRV 1.92. Crates live under `crates/`.

| Crate | Purpose | Depends on |
|-------|---------|------------|
| `proto` | Message enums, `WindowInfo`, `WindowSpec`, `Status`, `Runtime`, framing codec, path helpers, `PROTO_VERSION`. No I/O logic. | serde, rmp-serde, bytes, dirs |
| `daemon` | `Server`, `WindowManager`, `Window`, `launch` (per-runtime command builders), `status` (state machine), `worktree`, `state` (persistence), `hooks` (payload parsing). | proto, tokio, portable-pty, vt100, tracing |
| `tui` | `App`, `Connection`, `ui/{sidebar,terminal,statusbar,dialog,help,toast}`, `keymap`, `theme`, `config`. | proto, ratatui, crossterm, tui-term, vt100, tokio |
| `cli` | The `anthrex` binary and clap subcommands. | proto, daemon, tui, clap |

Each module has one job and is testable alone: `status` is a pure state machine with no I/O; `launch` builds argv and env without spawning; `worktree` shells out to git and is tested against a temp repo; `Connection` is the only place the client touches the socket.

### 2.3 Files and directories

| What | macOS | Linux |
|------|-------|-------|
| Socket | `$TMPDIR/anthrex-<uid>/daemon.sock` | `$XDG_RUNTIME_DIR/anthrex/daemon.sock`, fallback `/tmp/anthrex-<uid>/daemon.sock` |
| Data dir | `~/Library/Application Support/anthrex/` | `~/.local/share/anthrex/` |
| Config | `~/Library/Application Support/anthrex/config.toml` | `~/.config/anthrex/config.toml` |

The socket directory is created with mode 0700. The data dir holds `state.json`, `daemon.pid`, `daemon.log`, and `worktrees/`. Paths come from the `dirs` crate. All config keys are optional.

## 3. Daemon

### 3.1 Window model

```rust
struct Window {
    id: u32,                 // allocated before spawn, stable for the daemon's lifetime and persisted
    name: String,            // unique, user-visible
    runtime: Runtime,        // Claude | Codex | Shell
    cwd: PathBuf,            // where the child runs (the worktree path when a worktree is used)
    worktree: Option<Worktree>, // { repo_root, path, branch }
    model: Option<String>,
    initial_prompt: Option<String>,
    session_id: Option<String>, // Claude session id or Codex thread id, once known
    status: Status,
    tool: Option<String>,    // current tool name for Claude windows
    since: Instant,          // when status last changed
    last_output: Instant,    // last byte from the PTY
    exit: Option<ExitInfo>,  // { code: Option<i32>, reason: String }
    created_at: SystemTime,
}

enum Status { Starting, Working, Idle, Attention, Done, Exited }
```

### 3.2 Launchers

`launch` builds `(program, argv, env)` for each runtime. `<exe>` is the absolute path of the running anthrex binary from `std::env::current_exe()`, shell-quoted where it lands inside a shell command string. Environment is inherited plus `TERM=xterm-256color`, `COLORTERM=truecolor`, `ANTHREX_WINDOW_ID=<id>`, `ANTHREX_SOCKET=<path>`.

**Claude**

```
claude --name <name> --settings '<json>' [--model <model>] [--resume <session_id>] [<initial_prompt>]
```

where `<json>` is:

```json
{
  "hooks": {
    "SessionStart":      [{"matcher": "", "hooks": [{"type": "command", "command": "'<exe>' hook --window <id> --source claude"}]}],
    "UserPromptSubmit":  [ ...same... ],
    "PreToolUse":        [ ...same... ],
    "PostToolUse":       [ ...same... ],
    "PermissionRequest": [ ...same... ],
    "Notification":      [ ...same... ],
    "Stop":              [ ...same... ],
    "SessionEnd":        [ ...same... ]
  },
  "preferredNotifChannel": "terminal_bell"
}
```

Facts this relies on (verified against the Claude Code docs for 2.1.x): `--settings` accepts inline JSON and merges hooks with the user's own settings at the lowest precedence; hook commands receive a JSON object on stdin with `session_id`, `hook_event_name`, `cwd`, and event-specific fields such as `tool_name` and `notification_type`; a hook that exits 0 with no output changes nothing. `preferredNotifChannel` makes Claude ring BEL on notifications as a backup signal. The Claude workspace-trust prompt, if any, appears inside the window and the user answers it there.

**Codex**

```
codex -C <cwd> \
  -c 'notify=["<exe>","hook","--window","<id>","--source","codex-notify"]' \
  -c 'tui.terminal_title=["status"]' \
  -c 'tui.notifications=["approval-requested"]' \
  -c 'tui.notification_method="bel"' \
  -c 'tui.notification_condition="always"' \
  [-m <model>] [<initial_prompt>]
```

For resume the argv is the same global options followed by `resume <thread_id>`. The implementation must check which of `-m` and a prompt `codex resume` accepts in 0.135.0 and drop what it rejects.

Facts this relies on (verified in the Codex 0.135.0 source and docs): `notify` fires only on `agent-turn-complete`, is spawned with stdin at `/dev/null`, and passes the JSON payload as the last argv argument; the payload has `thread-id`, which is the id `codex resume` takes. `tui.terminal_title=["status"]` makes the TUI set the terminal title (OSC 0) to one of `Starting`, `Working`, `Thinking`, `Waiting`, `Ready`. `tui.notifications=["approval-requested"]` with method `bel` and condition `always` rings BEL whenever an exec, patch, or MCP approval is pending. Codex lifecycle hooks exist but need a per-definition trust step that shows a review prompt at startup, so v1 does not use them. The first-run directory trust prompt appears inside the window and the user answers it there.

**Shell**

```
$SHELL -l     (cwd = the window's cwd)
```

### 3.3 PTY I/O

- Spawn with `portable-pty` at the size the creating client reports.
- One blocking reader thread per window reads the master and sends `Bytes` chunks to an async task. That task feeds the daemon's `vt100::Parser` (no scrollback, sized to the PTY), then publishes the chunk on a `tokio::sync::broadcast` channel. After each chunk it checks the parser for a bell-count change and a title change and forwards those to the status engine, along with an output-activity event.
- Input from clients is handed to a per-window writer thread through a bounded channel; enqueueing never blocks, and a full queue is reported to the client as an error. A PTY master write can stall for seconds when the child is not reading its stdin, so no request path may perform one.
- A snapshot of a window is `\x1b[?1049h` when the screen is the alternate one, followed by the parser's `state_formatted()` (contents plus input modes).
- Resize calls `MasterPty::resize` and resizes the daemon's parser. Multiple subscribers: last resize wins.
- A blocking thread waits on the child; on exit it emits `ChildExited(code)`.
- Kill sends SIGTERM to the child pid, waits up to 3 s, then SIGKILL.
- A client that lags on the broadcast channel is sent a fresh `Snapshot` instead of the missed bytes.

### 3.4 Status engine

A pure state machine per window: `fn apply(&mut self, event: StatusEvent, runtime: Runtime) -> Option<Status>`.

Events: `Spawned`, `ClaudeHook { event, notification_type, tool_name, session_id }`, `CodexNotify { thread_id }`, `TitleChanged(String)`, `Bell`, `Output`, `Quiet` (3 s without output), `Focused`, `InputSent`, `ChildExited(code)`.

Transitions:

| Runtime | Event | New status / side effect |
|---------|-------|--------------------------|
| all | `Spawned` | Starting |
| all | `ChildExited` | Exited; record code |
| all | `Bell` | Attention |
| all | `Focused` | Done → Idle |
| all | `InputSent` | Attention → Working |
| Claude | `SessionStart` | record session_id; Idle (from Starting or any fallback-derived state) |
| Claude | `UserPromptSubmit` | Working |
| Claude | `PreToolUse` | Working; tool = tool_name |
| Claude | `PostToolUse` | tool = None |
| Claude | `PermissionRequest` | Attention |
| Claude | `Notification` permission_prompt | Attention |
| Claude | `Notification` idle_prompt | Idle |
| Claude | `Stop` | Done if not focused, else Idle; tool = None |
| Codex | `TitleChanged` Starting | Starting |
| Codex | `TitleChanged` Working / Thinking | Working |
| Codex | `TitleChanged` Waiting | Attention |
| Codex | `TitleChanged` Ready | Done if previous was Working and not focused, else Idle |
| Codex | `CodexNotify` | record thread_id; Done if not focused, else Idle |
| Shell | `Output` | Working |
| Shell | `Quiet` | Idle |

Fallback: for Claude and Codex windows that have not yet produced a single hook or recognized title (for example, hooks blocked in an untrusted workspace), the Shell rules for `Output` and `Quiet` apply. Once a hook or recognized title is seen, the fallback is disabled for that window. The daemon knows which window each client is viewing because `Subscribe` implies focus.

### 3.5 Worktrees

- Repo root: `git -C <dir> rev-parse --show-toplevel`; not a repo is an error shown in the dialog.
- Path: `<data_dir>/worktrees/<repo_basename>-<hash8(repo_root)>/<branch>` with `/` in the branch replaced by `-`.
- Create: `git worktree add <path> <branch>` if the branch exists, else `git worktree add -b <branch> <path>`.
- Remove: `git worktree remove <path>`; if git refuses because the tree is dirty, the client asks whether to force, then `--force`. The branch is never deleted.
- Git runs in `spawn_blocking` with a 30 s timeout. Failures return the git stderr to the client.

### 3.6 Persistence

`state.json` (version, next_id, windows) is written atomically (temp file + rename) on every change, debounced 100 ms. Each window record holds id, name, runtime, cwd, worktree, model, initial_prompt, session_id, created_at, and the last known status as a string. Screen contents are not persisted.

On daemon start with an existing state file, every window is listed as Exited with reason "daemon restarted". Restart re-launches with the same spec plus `--resume <session_id>` / `resume <thread_id>` when the id is known. Remove drops the record and optionally the worktree.

### 3.7 Lifecycle

- Start: create directories, bind the socket (an existing socket that refuses connections is removed as stale; one that accepts means another daemon is running and this one exits with an error), write `daemon.pid`, log to `daemon.log` via `tracing`.
- Auto-start: the client tries to connect; on failure it spawns `<exe> daemon --foreground` in a new session (`setsid`) with stdio redirected to `/dev/null`, then polls the socket every 50 ms for up to 3 s.
- Stop: `anthrex daemon stop` sends `Shutdown`; SIGTERM does the same. The daemon SIGTERMs every child, waits 3 s, SIGKILLs stragglers, writes state, exits.

## 4. Protocol

Framing: 4-byte big-endian length followed by a MessagePack body (`rmp-serde`, named fields). Maximum frame 16 MiB. `PROTO_VERSION` is a `u32` in `proto`; the handshake rejects mismatches with an `Error` and closes.

Client → daemon:

| Message | Fields |
|---------|--------|
| `Hello` | proto_version, client_kind (Tui, Cli, Hook) |
| `ListWindows` | — |
| `CreateWindow` | spec: { name, runtime, cwd, worktree_branch?, model?, initial_prompt? }, cols, rows |
| `Subscribe` | window_id, cols, rows |
| `Unsubscribe` | — |
| `Input` | window_id, bytes |
| `Resize` | window_id, cols, rows |
| `Kill` | window_id |
| `Remove` | window_id, remove_worktree, force |
| `Restart` | window_id |
| `Rename` | window_id, name |
| `HookEvent` | window_id, source (Claude, CodexNotify), payload (JSON value) |
| `Shutdown` | — |

Daemon → client:

| Message | Fields |
|---------|--------|
| `Welcome` | daemon_version, windows: Vec<WindowInfo> |
| `WindowsChanged` | windows: Vec<WindowInfo> (throttled to one per 50 ms) |
| `Created` | window_id (reply to CreateWindow) |
| `Snapshot` | window_id, cols, rows, bytes (`\x1b[?1049h` when the screen is the alternate one, then the parser's `state_formatted()`, which already carries the contents and the input modes) |
| `Output` | window_id, bytes |
| `Error` | request, message |
| `Bye` | reason |

A client has at most one subscription; `Subscribe` replaces the previous one, resizes the PTY, and is answered by `Snapshot` followed by live `Output`. The client resets its own parser before applying a snapshot. `WindowInfo` is the display projection of `Window`: id, name, runtime, cwd, worktree branch, status, tool, seconds since status change, seconds since last output, session_id presence, exit info.

## 5. CLI

| Command | Behaviour |
|---------|-----------|
| `anthrex [--dir <path>]` | Attach (auto-start daemon). `--dir` sets the default directory for new agents; defaults to the current directory. |
| `anthrex daemon [start\|stop\|status] [--foreground]` | Manage the daemon. `start` without `--foreground` detaches. |
| `anthrex new --runtime claude\|codex\|shell [--name] [--dir] [--worktree <branch>] [--model] [--prompt]` | Create a window and print its id. |
| `anthrex ls` | Table of windows with status. |
| `anthrex kill <id\|name>` / `anthrex rm <id\|name> [--worktree] [--force]` | Kill or remove. |
| `anthrex attach [id\|name]` | Attach, optionally focusing a window. |
| `anthrex hook --window <id> --source claude\|codex-notify [payload]` | Hidden. Payload from the positional argument if present, else stdin. Connects with a 1 s timeout, sends `HookEvent`, exits 0 always, prints nothing. |

## 6. TUI client

### 6.1 Layout

```
╭ anthrex ───────────────────╮╭ api-worker · claude · ~/repos/shop (feat/api) ────────────╮
│ ● api-worker               ││                                                            │
│   claude · working · 12s   ││   (focused agent's terminal, full size)                    │
│   Bash                     ││                                                            │
│ ◆ tests                    ││                                                            │
│   codex · attention · 1m   ││                                                            │
│ ○ shell                    ││                                                            │
│   zsh · idle · 5m          ││                                                            │
│                            ││                                                            │
│ 1 working · 1 attention    ││                                                            │
╰────────────────────────────╯╰────────────────────────────────────────────────────────────╯
 C-b ? help  C-b c new  C-b j/k switch  C-b d detach          ◆ tests needs approval
```

- Sidebar (default 30 columns, collapsible): header with daemon connection dot; one card per window with two lines: status glyph plus name, then runtime, status text, and elapsed time since the status changed; a third line with the current tool for Claude windows while working. The focused card gets an accent bar and bold name. Footer counts windows by status.
- Main: rounded block titled with window name, runtime, cwd, and branch; body is a `tui-term` `PseudoTerminal` widget rendering the client-side `vt100::Parser` (5000 lines of scrollback). The hardware cursor is placed at the parser's cursor when visible.
- Bottom bar: mode indicator (`PREFIX` while a prefix key is pending), key hints, and the current toast on the right.
- Resize: the client sends `Resize` whenever the main inner area changes, debounced 30 ms.

### 6.2 Input model

Passthrough by default: every key and paste goes to the focused PTY, encoded as the terminal would send it. The prefix key (default Ctrl-b, configurable) enters prefix mode for one keystroke. The table below is milestone 1's set with the milestone-6 change to rename and reconnect; section 10.3 of the product design spec lists every binding of milestones 1 to 9.

| Prefix + key | Action |
|--------------|--------|
| `j` / `k`, `n` / `p` | Next / previous window |
| `1`–`9` | Focus window by position |
| `c` | New-agent dialog |
| `x` | Kill focused window (confirm) |
| `X` | Remove focused window (confirm; worktree checkbox) |
| `R` | Restart focused window |
| `,` | Rename focused window (as in tmux) |
| `r` | Reconnect, only while disconnected |
| `s` | Toggle sidebar |
| `d` | Detach (quit client, daemon keeps running) |
| `Q` | Stop daemon and all agents (confirm) |
| `?` | Help overlay |
| Ctrl-b | Send a literal Ctrl-b to the PTY |
| Esc | Cancel prefix mode |

Mouse: clicking a sidebar card focuses it. Wheel events over the main area are forwarded to the PTY as SGR mouse reports when the parser reports that the application enabled mouse tracking; otherwise they scroll the client's scrollback view, and any keypress snaps back to the live screen.

### 6.3 Dialogs

- New agent: fields name (default `<runtime>-<n>`), runtime (Claude, Codex, Shell), directory (default from `--dir`, tilde expanded, must exist), worktree toggle plus branch name, initial prompt, model. Tab and Shift-Tab move, Enter submits, Esc cancels. Validation errors show inline. On success the new window is focused.
- Confirm: kill, remove (with "also remove worktree" checkbox and a follow-up force prompt if git refuses), stop daemon.
- Rename: single-line prompt with uniqueness check.
- Help: keymap overlay.
- Disconnected banner: shown when the socket drops, with `r` to reconnect or re-spawn the daemon.

### 6.4 Notifications

When a window that is not focused enters Attention or Done, the client shows a toast for 4 s ("tests needs approval", "api-worker finished") and rings the outer terminal's bell if enabled (config: `bell.attention = true`, `bell.done = false` by default).

### 6.5 Theme and config

Colours inherit the terminal background. One accent (default `#89b4fa`); status colours: working yellow with a braille spinner, attention peach, done green, idle gray, exited dim, starting gray. Glyphs are plain unicode (`●`, `○`, `◆`, `✓`, `✕`, `◌`) so no special font is needed. Rounded borders everywhere.

`config.toml` keys: `prefix` (default `"C-b"`), `sidebar_width`, `accent`, `bell.attention`, `bell.done`, `default_runtime`, `scrollback_lines`.

## 7. Error handling

- A missing or failing binary produces an Exited window whose screen shows the spawn error; nothing crashes.
- Hook delivery never blocks an agent: `anthrex hook` swallows every error and exits 0.
- Daemon death takes its children with it (the PTY master closes). The client shows the disconnected banner; on the next daemon start the state file lists the windows for restart-with-resume.
- Protocol version mismatch: the client explains and offers `anthrex daemon stop` then restart.
- Worktree failures surface git's stderr in the dialog and leave no half-created window.
- Every socket task, PTY task, and git call has a timeout or is cancellable; panics in one window's tasks are caught and turn that window Exited with the panic message rather than killing the daemon.

## 8. Testing

- `proto`: round-trip serialization for every message; framing rejects oversize frames.
- `daemon`:
  - `status` unit tests for every row of the transition table and the fallback rule.
  - `launch` unit tests asserting exact argv and settings JSON for each runtime, including quoting of the exe path and resume variants.
  - Integration tests spawning a real Shell window running `sh`, asserting the snapshot contains echoed text, input reaches the child, resize changes `stty size`, and exit codes are reported.
  - `worktree` tests against a temp git repo: create with new and existing branch, remove clean, refuse dirty, force.
  - `state` tests: atomic write, load, restart-with-resume argv.
  - Hook path test: run `anthrex hook` against a test daemon and assert the status change.
- `tui`: keymap state machine tests (prefix, literal prefix, escape); sidebar and dialog rendering through ratatui's `TestBackend` with snapshot assertions; scrollback view snapping.
- Manual smoke checklist: create a Claude window and a Codex window, watch status through a full turn including an approval, detach and reattach, restart the daemon and resume both.
- CI (GitHub Actions): `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` on macOS and Ubuntu.

## 9. Dependencies

| Crate | Version | Use |
|-------|---------|-----|
| ratatui | 0.30 | UI |
| crossterm | 0.29 | terminal backend and events |
| tui-term | 0.3 | PTY widget (targets ratatui 0.30, vt100 0.16) |
| vt100 | 0.16 | VT parsing on both sides |
| portable-pty | 0.9 | PTYs |
| tokio | 1 | async runtime, Unix sockets, channels |
| serde, rmp-serde, serde_json | current | protocol and hook payloads |
| clap | 4 | CLI |
| dirs | current | platform paths |
| tracing, tracing-subscriber, tracing-appender | current | daemon logging |
| libc | current | setsid, kill |
| anyhow, thiserror | current | errors |

## 10. Verified facts and known risks

Verified: the Claude hook event list and stdin payload shape; `--settings` merging; `--resume <id>` and `--name`; `preferredNotifChannel`; the Codex notify payload, argv delivery, and `thread-id`; Codex title status strings and BEL notification settings for 0.135.0; `codex resume <uuid>`; session rollout file naming; `tui-term` 0.3.4 compatibility with ratatui 0.30.

Risks and their handling:

1. Claude hooks can be disabled in untrusted workspaces. Handling: the output-activity fallback plus the BEL backup; the trust prompt itself is visible in the window.
2. Codex title strings are read from source and may change in later versions. Handling: the fallback rule applies until a recognized title is seen; unknown titles are ignored and logged.
3. Codex's directory trust prompt blocks a new directory until answered. Handling: it is answered in the window like any other prompt. A `-c projects.<path>.trust_level` override was not verified and is not used.
4. `codex resume` may not accept `-m` or a prompt. Handling: verified during implementation; unsupported options are dropped on resume only.
5. Two clients attached at different sizes will fight over the PTY size. Handling: documented v1 limitation, last resize wins.

## 11. Implementation order

1. `proto` and a daemon that runs Shell windows, plus a minimal client that attaches, renders one window, switches, and detaches.
2. Claude and Codex launchers, the hook command, and the status engine feeding the sidebar.
3. New-agent dialog and worktrees.
4. Persistence, restart-with-resume, toasts, help, rename, config file, theme polish.
5. CI, README, smoke checklist.
