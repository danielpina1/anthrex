[![CI](https://github.com/danielpina1/anthrex/actions/workflows/ci.yml/badge.svg)](https://github.com/danielpina1/anthrex/actions/workflows/ci.yml)

# anthrex

A terminal multiplexer for coding agents. Each window is a real `claude`, `codex` or shell session in its own PTY, owned by a background daemon so it survives closing the UI. A sidebar shows every agent with live status; the main pane shows the focused one at full size.

## Build

```bash
cargo build --release
# binary: target/release/anthrex
```

Requires Rust 1.92 or newer, macOS or Linux.

## Use

```bash
anthrex                                   # attach (starts the daemon if needed)
anthrex new --runtime claude --name api   # create an agent window from any shell
anthrex new --runtime codex --dir ~/repo
anthrex ls
anthrex kill api
anthrex rm api
anthrex daemon stop                       # stops the daemon and every agent
```

Inside the UI, the prefix key is `Ctrl-b`:

| Keys | Action |
|------|--------|
| `C-b j` / `C-b k` | next / previous agent |
| `C-b 1`..`9` | focus agent by number |
| `C-b c` | new shell window |
| `C-b t` | tree mode (j/k, Enter, Space, /) |
| `C-b T` | tree overview |
| `C-b <` / `C-b >` | sidebar width |
| `C-b x` | kill agent |
| `C-b X` | remove agent |
| `C-b s` | toggle sidebar |
| `C-b d` | detach (agents keep running) |
| `C-b Q` | stop daemon and all agents |
| `C-b C-b` | send a literal C-b |
| `C-b ?` | help |

Mouse: click a sidebar card to focus it; the wheel scrolls (forwarded to programs that use the mouse). Because mouse capture is on, use your terminal's shift-drag to select text.

## Git status

The bottom bar shows the branch, uncommitted-work count, divergence from upstream, and any in-progress merge or rebase for the focused agent's checkout. A filesystem watcher gives the fast path, so it moves within a fraction of a second of a commit; a 30-second safety poll of every registered checkout is the backstop, so the cost while a repository sits idle is one cheap `git status` probe every 30 seconds, not nothing. Set `ANTHREX_GIT=off` to disable the watcher and probes entirely. The project tree (`C-b t` / `C-b T`) is drawn with box-drawing connectors (`├─`, `└─`, `│`) at every level, to any depth, so a worker's own sub-agents stay legible.

## Test

```bash
cargo test --workspace        # unit and integration tests
python3 scripts/pty-smoke.py  # drives the real binary through a PTY end to end
```

The smoke script builds `target/debug/anthrex` if it is missing, and runs against an isolated socket and data directory under `/tmp` that it cleans up afterwards.

## Files

| What | macOS | Linux |
|------|-------|-------|
| socket | `$TMPDIR/anthrex-<uid>/daemon.sock` | `$XDG_RUNTIME_DIR/anthrex/daemon.sock` |
| log, pid | `~/Library/Application Support/anthrex/` | `~/.local/share/anthrex/` |

`ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` override these. `ANTHREX_LOG=debug` raises the daemon log level.

## Roadmap

Milestone 1 of 9 is done. `docs/ROADMAP.md` lists every milestone: agent status from hooks, a project tree of agents and their sub-agents, worktrees, persistence, split panes, and orchestration where a Claude or Codex orchestrator dispatches reviewed, merged tasks to both. Coding agents working on this repository start with `AGENTS.md`.
