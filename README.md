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
| `C-b j` / `C-b k` | next / previous window |
| `C-b 1`..`9` | focus window by number |
| `C-b c` | new shell window |
| `C-b x` / `C-b X` | kill / remove the focused window |
| `C-b s` | toggle the sidebar |
| `C-b d` | detach (agents keep running) |
| `C-b Q` | stop the daemon and all agents |
| `C-b C-b` | send a literal Ctrl-b |
| `C-b ?` | help |

Mouse: click a sidebar card to focus it; the wheel scrolls (forwarded to programs that use the mouse). Because mouse capture is on, use your terminal's shift-drag to select text.

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

## Status

Milestone 1 of 5 (see `docs/superpowers/specs/`). Coming next: hook-driven agent status, the new-agent dialog with git worktrees, persistence across daemon restarts, CI.
