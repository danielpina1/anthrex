# anthrex roadmap

anthrex is built in ten milestones. Milestones 1 to 4 are merged. Each later milestone has an implementation brief in `docs/milestones/`, written so that a coding agent such as Codex can implement it without further questions. `AGENTS.md` at the repository root holds the rules that apply to every milestone.

The design is layered, newest first:

1. `docs/superpowers/specs/2026-09-20-git-surface-and-simple-orchestration-design.md` — milestone 4.5, and the cut-down scope of milestones 8 and 9. Where it disagrees with the document below, it wins.
2. `docs/superpowers/specs/2026-09-18-anthrex-product-design.md` — milestones 2 to 9.
3. `docs/superpowers/specs/2026-09-17-anthrex-design.md` — milestone 1 and the parts of the core it still governs.

The protocol version currently on `main` is **3**. Milestone 4.5 raises it to 4. Protocol numbers written in the milestone 5 to 9 briefs predate this and are wrong; each is re-derived from this line when that milestone is implemented.

## What anthrex does when all milestones are done

- Runs many Claude Code and Codex agents side by side, each as the real app in its own terminal, owned by a background daemon so agents survive closing the UI.
- Shows every project as a tree — project, then each agent session, then that session's sub-agents, then theirs — drawn with real connectors to any depth. Every node shows live state: working, needs attention, done.
- Shows the branch, uncommitted work and divergence of the focused agent's checkout, kept fresh by watching the filesystem.
- Shows several agents at once in split panes.
- Creates agents from a form, each optionally in its own git worktree, and restarts them in their previous session after a reboot.
- Runs an orchestration: you pick the orchestrator's runtime and model and give it a goal. It plans tasks and dispatches each one to a Claude or Codex worker with a model suited to the task. Every task runs in its own worktree and is reviewed by a different agent before it is merged into a run branch. Nothing reaches your base branch until you accept the result.

## Milestones

| # | Milestone | Brief | Depends on | Status |
|---|-----------|-------|------------|--------|
| 1 | Foundation: daemon, PTY windows, sidebar, CLI | `docs/superpowers/plans/2026-09-17-anthrex-foundation.md` | none | `done` |
| 2 | Continuous integration | `docs/milestones/M2-ci.md` | 1 | `done` |
| 3 | Agent status and sub-agent tracking from hooks | `docs/milestones/M3-agent-status.md` | 2 | `done` |
| 4 | Project tree view | `docs/milestones/M4-project-tree.md` | 3 | `done` |
| 4.5 | Git status in the bottom bar, and tree connectors | `docs/milestones/M4.5-git-and-tree.md` | 4 | `ready` |
| 5 | New-agent dialog and git worktrees | `docs/milestones/M5-worktrees.md` | 4.5 | `blocked` |
| 6 | Persistence, resume, rename, config, reconnect | `docs/milestones/M6-persistence.md` | 3 | `blocked` |
| 7 | Split panes | `docs/milestones/M7-split-panes.md` | 4 | `blocked` |
| 8 | Orchestration engine: runs, tasks, worktrees, review and merge | `docs/milestones/M8-orchestration-engine.md` | 5, 6 | `blocked` |
| 9 | Orchestrator agent: planning, model routing, review loop, plan and finish views | `docs/milestones/M9-orchestrator-agent.md` | 8 | `blocked` |

Work the milestones in numerical order. Milestones 6 and 7 do not depend on each other or on milestone 5, so they may be swapped, but only one milestone should be in progress at a time: they all touch the protocol and the client state.

```mermaid
flowchart LR
  M1[1 Foundation] --> M2[2 CI]
  M2 --> M3[3 Agent status]
  M3 --> M4[4 Project tree]
  M4 --> M45[4.5 Git and tree connectors]
  M45 --> M5[5 Worktrees]
  M3 --> M6[6 Persistence]
  M4 --> M7[7 Split panes]
  M5 --> M8[8 Orchestration engine]
  M6 --> M8
  M8 --> M9[9 Orchestrator agent]
```

## Why this order

- **CI first.** Every later milestone is implemented by an agent. Automated checks on every pull request catch regressions before a human reviews them.
- **Git state before worktrees.** Milestone 4.5 builds the watcher and the per-worktree git probe that milestone 5 needs the moment agents get their own checkouts, and that milestone 8 needs again to decide whether a task is finished.
- **Status before the tree.** The tree is only as good as the state it shows. Milestone 3 makes status exact and records sub-agents; milestone 4 draws them.
- **Worktrees before orchestration.** Parallel workers must never edit the same checkout.
- **Persistence before orchestration.** An orchestration run can last hours; it must survive a daemon restart.
- **Engine before agent.** Milestone 8 makes runs, worktrees, reviews and merges work deterministically from a plan file, testable without any model. Milestone 9 then puts a model in charge of the decisions.

## Definition of done for every milestone

1. Every task in the brief is implemented, or its brief's "Implementation notes" explain precisely why not.
2. `cargo build --workspace --all-targets`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` and `python3 scripts/pty-smoke.py` pass locally and in CI.
3. The brief's manual check has been done by a human, or is listed as outstanding in the pull request.
4. Findings that were deferred are recorded in `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`.
5. This table shows the milestone as `done`, and the next milestone whose dependencies are now all done is set to `ready`.
