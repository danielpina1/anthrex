# Contributing to anthrex

Thanks for helping. This page covers the build, the tests, and how a change reaches `main`. Coding agents working on the repository also follow [`AGENTS.md`](AGENTS.md), which has the full rules.

## Build and test

You need Rust 1.92 or newer, `git`, and `python3` for the end-to-end smoke test. anthrex runs on macOS and Linux.

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

All five must pass before a pull request is ready. CI runs the same commands on Ubuntu and macOS with the latest stable Rust.

- `cargo test` spawns real shells in real pseudo-terminals, and the run-engine tests create temporary git repositories.
- The tests never start a real `claude` or `codex`. They run `crates/fake-agent`, a scripted stand-in, through `ANTHREX_CLAUDE_BIN`, `ANTHREX_CODEX_BIN` and `ANTHREX_DECIDER_BIN`. Keep it that way in every new test.
- `scripts/pty-smoke.py` drives the real binary through a pseudo-terminal and checks the rendered screen. It runs its own daemon on a socket and data directory under `/tmp`, and removes them afterwards.

## Trying a change by hand

Use an isolated daemon, so your experiments never touch the daemon and agents you use day to day:

```bash
export ANTHREX_SOCKET=/tmp/anthrex-dev/daemon.sock
export ANTHREX_DATA_DIR=/tmp/anthrex-dev/data
export ANTHREX_CONFIG=/tmp/anthrex-dev/config.toml
target/debug/anthrex
# ...
target/debug/anthrex daemon stop   # with the same variables
```

## How to work

- **Tests first.** Write a test that fails without your change and passes with it. Prefer real behaviour over mocks: a real PTY, a real shell, a real socket, a temporary git repository. Wait on conditions with a deadline loop; a fixed sleep must never be the only synchronisation. Before adding a wall-clock bound to a test, read [`docs/timing-budgets.md`](docs/timing-budgets.md).
- **Keep the client pure.** The client state in `crates/tui/src/app/` and the rendering in `crates/tui/src/ui/` perform no I/O: update functions return effects, and rendering takes `&App`.
- **Never block the daemon.** No blocking work under the manager lock or on a tokio worker thread. Git commands, process spawns and file I/O that can stall run on `spawn_blocking` or a dedicated thread, with a timeout.
- **Protocol changes bump `proto::PROTO_VERSION`**, update every client in the same change, and add a round-trip test for each new message.
- **Keep files focused.** A file past roughly 600 lines is doing too much; split it by responsibility.
- **Milestones.** Planned work is listed in [`docs/ROADMAP.md`](docs/ROADMAP.md), with one brief per milestone in [`docs/milestones/`](docs/milestones/) and the design specs in `docs/superpowers/specs/`.

## Commits and pull requests

- Use conventional commit subjects: `feat(daemon): ...`, `fix(tui): ...`, `test: ...`, `docs: ...`, `chore: ...`.
- One commit per task, or per coherent group of tasks, so a reviewer can follow the change step by step.
- Work on a branch and open a pull request into `main`. Never push to `main` or force-push a shared branch.
- The pull request description lists what changed, the verification output of the five commands above, and anything you deferred.

By contributing you agree that your contributions are licensed under the [MIT License](LICENSE).
