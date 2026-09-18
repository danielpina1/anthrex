# AGENTS.md

Instructions for coding agents, Codex first, working in this repository. Read this file completely before touching code.

## What anthrex is

anthrex is a terminal multiplexer for coding agents, written in Rust. A background daemon owns one pseudo-terminal per agent and runs the real `claude` or `codex` app, or a shell, inside it. A ratatui client shows every agent with live status and the focused agent at full size. Detaching leaves the agents running.

## Where things are

| Path | What it holds |
|------|---------------|
| `crates/proto` | Wire types, length-prefixed MessagePack framing, socket and data paths. No I/O logic beyond framing. |
| `crates/daemon` | PTY windows, status state machine, window manager, Unix-socket server, daemon lifecycle. |
| `crates/tui` | The client: key encoding, prefix keymap, daemon connection, pure application state, rendering, event loop. |
| `crates/cli` | The `anthrex` binary and its subcommands. |
| `crates/fake-agent` | Scripted Claude/Codex stand-in for deterministic status integration and smoke tests. |
| `scripts/pty-smoke.py` | End-to-end smoke test that drives the real binary through a real PTY with an isolated daemon. |
| `docs/ROADMAP.md` | Every milestone, its status, and its dependencies. Start here. |
| `docs/milestones/` | One implementation brief per milestone. The brief is your requirements. |
| `docs/superpowers/specs/` | Design specs. They are the binding authority when a brief is unclear. |
| `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` | Findings deferred from milestone 1, mapped to later milestones. |

## How to pick up work

1. Open `docs/ROADMAP.md`. Take the lowest-numbered milestone whose status is `ready`.
2. Read its brief in `docs/milestones/` from top to bottom, then the spec sections it cites.
3. Create a branch named `m<N>-<slug>`, for example `m3-agent-status`.
4. Do the tasks in the order the brief lists them. Each task names its files, the tests to write first, and its acceptance criteria.
5. When every task and the milestone verification pass, set the milestone's status to `done` in `docs/ROADMAP.md`. Record every deviation from the brief in the brief's "Implementation notes" section. Then open a pull request into `main`. Do not merge it yourself.

The "Design decisions" in a brief are final. If one turns out to be wrong or impossible, do not quietly do something else. Stop work on that item. Write what you found, with the evidence, under "Implementation notes". Then continue with the tasks that do not depend on it.

## Commands

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

Status tests run `crates/fake-agent` through `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN`; never point them at real agent binaries.

All five must pass before a milestone is done. `cargo test` spawns real shells in real PTYs. Kill signals the process group with SIGHUP first, so an interactive shell exits at once.

## Hard rules

1. **Never touch the user's live daemon.** For every manual run, set `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` to paths under `/tmp`, and end with `anthrex daemon stop` using the same variables. Never run `anthrex` without those variables. Never signal a process you did not start. Before you finish, `pgrep -fl "anthrex daemon"` must show nothing of yours.
2. **No blocking work under the manager lock or on a tokio worker thread.** PTY writes go through the per-window writer thread. Git commands, process spawns and file I/O that can stall run on `tokio::task::spawn_blocking` or a dedicated thread, with a timeout. Milestone 1's worst bug was a blocking PTY write under the lock, which froze the whole daemon.
3. **Take mutexes with `daemon::lock(&m)`, never `.lock().unwrap()`.** It recovers from poisoning so one panicking window cannot take the daemon down.
4. **Protocol changes bump `proto::PROTO_VERSION`.** Update every client in the same change: the TUI, the CLI client, and `anthrex hook` once it exists. Add a round-trip test for every new message.
5. **The client's state stays pure.** `crates/tui/src/app.rs` and everything under `crates/tui/src/ui/` perform no I/O. Update functions return `Vec<Effect>`. Rendering takes `&App`.
6. **Tests come first and test real behaviour.** Prefer a real PTY, a real shell, a real socket, or a temporary git repository over a mock. Wait with a deadline loop; a fixed sleep must never be the only synchronisation. A new test must fail before your change and pass after it.
7. **Stay inside the brief.** If you notice something worth fixing outside it, add it to `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` under the milestone it belongs to, and move on.
8. **Keep files focused.** A file that grows past roughly 600 lines is doing too much; split it by responsibility, following the brief's file list.
9. **Never push to `main`, never force-push a shared branch, never merge a pull request.** Opening a pull request is the end of your job.

## Facts learned the hard way in milestone 1

- vt100 0.16: `Screen::state_formatted()` already contains `contents_formatted()` plus `input_mode_formatted()`. Neither encodes the alternate screen; a snapshot must prepend `ESC [ ? 1049 h` when `alternate_screen()` is true.
- tokio `broadcast`: after `RecvError::Lagged` the receiver resumes from the oldest retained message. To resynchronise, take a fresh `attach()` and replace the receiver.
- macOS: writing to a PTY master blocks after about 1 KB when the foreground program is not reading. Never write from a thread that anything else waits on.
- `JoinHandle::abort()` takes effect at the next yield point. Await the handle after aborting when ordering matters.
- Kill signals the process group with SIGHUP first, so an interactive shell exits at once.
- Edition 2024 makes `std::env::set_var` and `remove_var` unsafe. Tests that change environment variables must hold a shared lock.
- Claude Code accepts hooks through `claude --settings '<json>'`, merged under the user's own settings. Codex runs `notify` with the JSON payload as the last argument, not on stdin.

## Commits and pull requests

- Conventional commit subjects: `feat(daemon): ...`, `fix(tui): ...`, `test: ...`, `docs: ...`.
- One commit per task, or per coherent group of tasks, so a reviewer can follow the milestone task by task.
- The pull request description lists every task with its status, the verification output, and a pointer to the brief's "Implementation notes".
