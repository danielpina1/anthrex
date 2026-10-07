# Changelog

All notable changes to anthrex are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and anthrex uses
[Semantic Versioning](https://semver.org/). While the major version is 0, every
release is a pre-release and any release may change behaviour or the protocol.

## [Unreleased]

## [0.1.2] — 2026-10-07

The protocol version is now 18: a 0.1.2 client needs a 0.1.2 daemon. Stop the old
daemon (`anthrex daemon stop`) after upgrading.

### Added

- **Run titles.** A run started from a goal gets a short title and a readable id
  (`status-bar-usage-version-a415` rather than the goal's first 32 characters), written
  by a quick decider call with a 15-second bound. The run view, the runs tree, the
  inspector and alerts show the title; the inspector keeps the full goal. When the
  call fails or times out, the run is named from its goal as before.
- **Esc no longer interrupts a working orchestrator.** In the orchestrator window of a
  live run, a bare Esc is held back while the orchestrator is working, with a notice to
  use Ctrl-C. Esc still passes when it is idle or waiting on you.

### Fixed

- **Claude workers could not commit on Linux.** The worker sandbox granted exact git
  files, including lock files that do not exist yet, which Linux's sandbox cannot grant.
  On Linux a worker now gets its checkout's git directory whole, with the entries it
  must never change (`config`, `gitdir`, `refs/`, `hooks/` and others) denied. macOS is
  unchanged. Codex workers on Linux keep the old grant and are still affected; use
  Claude workers there for now.
- **Sandbox hardening found in review:** the daemon no longer stages files in a
  directory a worker can write, never renames over a denied entry, verifies a task
  checkout's `config` before every git call, waits for a merged task's worker to exit
  before removing its checkout, and runs its git with `rerere` and submodule recursion
  off.
- **A headless agent that dies at startup now says why.** A scout's or run task's
  failure reason includes the agent's own error, such as Claude's sandbox needing
  `bubblewrap` and `socat`, or Ubuntu's AppArmor blocking it (with a pointer to the fix),
  instead of "ended two turns without a report".

## [0.1.1] — 2026-10-06

### Fixed

- **Claude Code 2.1.291 and later could not see anthrex's tools.** Claude Code now
  speaks MCP `2026-07-28` and ignores a tool list that lacks cache hints, which the
  MCP library anthrex used left out. Every scout, so every `anthrex profile detect`,
  failed with "the scout ended two turns without a report". anthrex now uses rmcp
  3.5.1, which adds the hints.

## [0.1.0] — 2026-10-06 — first alpha

The first public build of anthrex: a terminal multiplexer for coding agents.
Prebuilt binaries are for Linux (x86_64, aarch64) and macOS (Apple silicon, Intel);
see [docs/install.md](https://github.com/danielpina1/anthrex/blob/main/docs/install.md). The binaries are unsigned.

### Added

- **Multiplexer.** A background daemon owns one pseudo-terminal per agent and runs
  the real `claude` or `codex` app, or a shell, inside it. A terminal client shows
  every agent with the focused one at full size; detaching leaves the agents
  running. A `C-b` prefix keymap and the `anthrex` CLI (`new`, `ls`, `kill`, `rm`,
  `daemon`, ...) drive it.
- **Agent status.** Live working / needs-attention / done status for Claude Code
  and Codex sessions, and their sub-agents, from the agents' own hooks.
- **Project tree and graph.** Every project drawn as a tree of sessions and
  sub-agents to any depth, a graph overview, and a node inspector below it.
- **Git surface.** The focused checkout's branch, uncommitted work and divergence
  in the bottom bar, kept fresh by watching the filesystem.
- **Worktrees.** A new-agent dialog that can start an agent in its own git worktree.
- **Persistence.** Windows, names and sessions survive a daemon restart or reboot;
  agents resume their previous session; a config file and client reconnect.
- **Conversation view.** A structured view of an agent's turns, with folded tool
  calls and links to its sub-agents.
- **Orchestrator.** Give a goal to one orchestrator agent: it scouts the
  repository, plans and sizes the work, and dispatches each task to a headless
  Claude or Codex worker in its own worktree. Each task passes a test proof, the
  check and a review by a different agent, then merges through a queue that tests
  the merged result. A plan gate, alerts, a live run view (`C-b T`) and a run
  inspector let you steer it; nothing reaches the base branch until you accept it.
- **Adaptation.** A repository profile, scouts and deciders, a fast path for small
  goals, an output filter, metering and run history.
- **Tiered testing.** Affected-set test tiers, a test scheduler and result cache,
  flake handling, bisecting and staged checks.
- **Stacked-PR delivery.** Results delivered as stacked pull requests; CI failures
  and review comments become fix tasks. anthrex never merges a pull request itself.
- **Rounds and chains.** A goal editor, rounds that iterate on a complete run, and
  next goals on the same orchestrator.
- **Tuning.** History refits and proposals, model lists, a run estimate, adaptive
  concurrency, and race and test-writer patterns.
- **Design flow.** Brainstorm (two models, merged), spec (peer-reviewed, with
  numbered requirements) and plan (with engine-checked coverage), each approved by
  you before the next begins.
- **Delivery hardening.** Fix tasks for merged stages, unviewed merges checked with
  git, trimmed holds, and a CI bisect over the whole stack.
- **TUI.** A design kit across every screen, a daemon-computed action menu with
  forms and confirmation pages, and Profile and Settings screens with live reload.
- **MCP server.** `anthrex mcp` exposes anthrex to an agent over stdio.

### Known limitations

- Split panes are not in this release.
- macOS and Linux only. anthrex runs `claude` and `codex` as installed on your
  machine; it does not ship or install them.

[Unreleased]: https://github.com/danielpina1/anthrex/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/danielpina1/anthrex/releases/tag/v0.1.2
[0.1.1]: https://github.com/danielpina1/anthrex/releases/tag/v0.1.1
[0.1.0]: https://github.com/danielpina1/anthrex/releases/tag/v0.1.0
