# anthrex design amendment: git surface, tree connectors, simplified orchestration

Date: 2026-09-20. Status: binding.

This document amends `docs/superpowers/specs/2026-09-18-anthrex-product-design.md`. Where the two disagree, this one wins. Section 8 lists exactly what it supersedes.

## 1. What changes

1. A new milestone **4.5**, run before milestone 5, delivering two things: git state for the focused agent in the bottom bar, and real box-drawing connectors in the project tree at every level.
2. Milestone **8** is cut down to a deterministic engine: a run is a list of tasks, each task is one worker agent in its own worktree, gated by a check command and a review by a second agent, merged into a run branch that never touches your base branch until you accept it.
3. Milestone **9** is cut down to the model that writes the plan and fills in each task's runtime and model. It does not review and it does not merge.

The order of the remaining milestones is 4.5, 5, 6, 7, 8, 9. Milestone numbers 5 to 9 keep their current names and brief filenames.

## 2. Terms

- **worktree root** — the top level of the git working tree that a directory sits in: `git rev-parse --show-toplevel`. Every linked worktree has its own.
- **project root** — as milestone 4 defines it: the parent of `git rev-parse --git-common-dir`. Every linked worktree of one repository shares it.
- **git state** — branch, dirty counts and divergence for one *worktree root*.
- **run** — one orchestration: a goal, a base branch, a run branch, and a list of tasks.
- **task** — one unit of work: one worker agent, one worktree, one branch, one review.

The tree groups by project root. Git state is keyed by worktree root. These differ the moment milestone 5 gives agents their own worktrees, and conflating them would show every worktree the parent repository's branch.

## 3. Git state (milestone 4.5)

### 3.1 Model

```rust
pub enum Head {
    Branch(String),
    Detached(String),  // short oid
    Unborn(String),    // branch name that does not exist yet
}

pub enum GitOperation { Merge, Rebase, CherryPick, Revert, Bisect }

pub struct GitState {
    pub head: Head,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub dirty: u32,       // tracked files with staged or unstaged changes
    pub untracked: u32,
    pub conflicts: u32,
    pub operation: Option<GitOperation>,
    pub stale: bool,      // the last probe failed or was truncated
}
```

`dirty` deliberately merges staged and unstaged counts. The bottom bar has no room for both, and the question it answers is "does this worktree have uncommitted work", not "what is in the index".

### 3.2 The probe

One command per refresh:

```
git -C <worktree-root> --no-optional-locks status --porcelain=v2 --branch --untracked-files=normal -z
```

- `--no-optional-locks` is mandatory. Without it the probe can take the index lock while an agent is running git in the same worktree.
- `-z` makes records NUL-terminated, so a path containing a newline cannot desynchronise the parser. Rename records carry two NUL-separated paths; the parser counts records, so this is harmless as long as it consumes the extra field.

Parsing:

| Record | Meaning |
|---|---|
| `# branch.oid <oid>` | `(initial)` means unborn |
| `# branch.head <name>` | `(detached)` means detached; use the short oid from `branch.oid` |
| `# branch.upstream <name>` | absent when there is no upstream |
| `# branch.ab +<a> -<b>` | ahead, behind; absent when there is no upstream |
| `1 <XY> …`, `2 <XY> …` | `dirty += 1` when `X != '.'` or `Y != '.'` |
| `u …` | `conflicts += 1` |
| `? …` | `untracked += 1` |

The in-progress operation is not in `status` output; it is a handful of `stat` calls against the worktree's own git dir (`--git-dir`, not the common dir — rebase and merge state are per worktree), tested in this order: `MERGE_HEAD` → Merge, `rebase-merge/` or `rebase-apply/` → Rebase, `CHERRY_PICK_HEAD` → CherryPick, `REVERT_HEAD` → Revert, `BISECT_LOG` → Bisect.

Hardening, identical to `daemon::project`: `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE` and `GIT_PREFIX` removed from the environment; stdin and stderr null; stdout piped and drained non-blocking; the child terminated and reaped on timeout. The probe runs on `tokio::task::spawn_blocking` and never under the manager lock.

Limits: 1 MiB of output and 5 seconds. On either limit the probe stops, keeps the counts parsed so far, and sets `stale`. A repository with fifty thousand untracked files must degrade, not wedge the daemon.

At most one probe per root is in flight. A refresh requested while one is running sets a pending bit; exactly one more probe runs when the current one returns.

### 3.3 Watching

One `notify` recommended watcher per registered worktree root: recursive over the worktree, plus non-recursive over that worktree's git dir.

Events are filtered before they count, by path only — no gitignore parsing:

- any path component equal to `target`, `node_modules`, `.venv`, `dist`, `build`, `.next`
- anything under `<gitdir>/objects` or `<gitdir>/lfs`
- any file name ending in `.lock`

An accepted event schedules a probe 300 ms later; further accepted events inside that window push the deadline out. Every root is also probed every 30 seconds regardless, and immediately when it is first registered.

**Circuit breaker.** If a root accepts more than 200 events in 10 seconds, log once and drop that root to poll-only for 60 seconds. This is the `cargo build` case: a build writes inside `target`, which is filtered, but a `git checkout` of a large tree is not.

**Watcher failure** (descriptor limits, an unsupported filesystem) is logged once per root and leaves that root on the 30-second poll.

**Lifecycle.** A root is registered when at least one window records it, and unregistered when the last such window is removed. Registration and unregistration happen off the manager lock.

### 3.4 Window metadata

`WindowInfo` gains `worktree: Option<PathBuf>`. `daemon::project::detect_root` already asks git for `--git-common-dir` and `--show-toplevel` and discards the second; it now returns both. `None` means the window's directory is not inside a git working tree.

Like `project`, `worktree` is computed once, before the window is created, and never changes afterwards. An agent that `cd`s elsewhere keeps the worktree it was launched in — the same rule milestone 4 set for `project`, and for the same reason: a row that moves under the user's cursor breaks selection.

### 3.5 Protocol

`PROTO_VERSION` goes from 3 to 4.

- `WindowInfo.worktree: Option<PathBuf>`.
- `DaemonMsg::Git { root: PathBuf, state: Option<GitState> }`. `None` means the root is not a git working tree or git could not be run. Broadcast to every attached client whenever a probe changes the state, and sent once for every known root immediately after the client's initial window list, so a fresh client is never blank.

There is no client-initiated refresh. The daemon decides when to probe.

Later milestone briefs name specific protocol numbers. Those numbers are now wrong. Every brief's protocol number is re-derived when that milestone is implemented; `docs/ROADMAP.md` records the current version.

### 3.6 The bottom bar

A segment for the focused window's worktree root, rendered right of the key hints and left of the toast. It is hidden when the focused window has no worktree, or when no state has arrived yet.

```
main ✓                     clean and in sync
main ●3 ?1 ⇡2⇣1            3 tracked changes, 1 untracked, ahead 2 behind 1
main ⚠2 rebase             2 conflicts, rebase in progress
@a1b2c3d ●1                detached
main (unborn)              branch has no commits yet
main ●3 (stale)            the last probe failed; showing the last good state
```

Colour carries the meaning: the head is bold, `✓` green, `●` and `?` muted, `⇡⇣` muted, `⚠` and the operation name red, `(stale)` muted. No branch glyph — `⎇` and powerline glyphs are not reliably present in the fonts people use.

**Truncation order**, as the terminal narrows: key hints drop from the right one at a time; then the git segment's parts drop right to left (operation, untracked, divergence, dirty); then the git segment drops entirely, leaving the head name; then it is hidden. The toast is never truncated by the git segment.

### 3.7 Configuration

Milestone 4.5 predates the configuration file, so for now there is one environment variable: `ANTHREX_GIT=off` disables the subsystem entirely — no watchers, no probes, no `Git` messages. The smoke script and CI set it, so that terminal snapshots stay deterministic regardless of the repository's state.

Milestone 6 adds a `[git]` table with `enabled`, `poll_secs`, `debounce_ms` and an `ignore` list appended to the built-in filter.

### 3.8 Failure modes

| Cause | Behaviour |
|---|---|
| git not installed | every probe fails, no git segment, one log line per root |
| very large repository | output capped, counts partial, `stale` set |
| event storm | filter, then debounce, then the circuit breaker |
| probe slower than the poll | coalescing keeps one probe in flight |
| worktree deleted under us | probe fails, state keeps `stale`, root unregisters with its last window |

## 4. Tree connectors (milestone 4.5)

### 4.1 Guides

`Row.indent: u16` is replaced by `Row.guides: String`, the exact prefix drawn before the row's own marker. It generalises the logic `crates/tui/src/tree.rs` already uses for sub-agents to every level of the tree:

- for each ancestor above the row's parent: `"│ "` when that ancestor has a later visible sibling, `"  "` when it does not
- for the row itself: `"├─"` when it has a later visible sibling, `"└─"` when it does not
- project rows are roots and have empty guides

Two columns per level. At depth six the guides cost twelve columns, which still leaves room for a name in the 32-column sidebar.

### 4.2 Depth is unbounded

Sub-agent nesting is three levels deep today because that is Claude Code's limit. Orchestration adds levels below a window. No part of the renderer, the geometry or the tests may assume a maximum depth.

### 4.3 Visible tree, not the full tree

Guides are computed from the tree **after** collapse and filtering. Hiding the last sibling of a group turns the `├─` above it into `└─`. This is the case most likely to be got wrong, and it has a test of its own.

### 4.4 Both views, one implementation

The sidebar and the full-screen overview use the same guide strings. The overview keeps its aligned name, model, status and elapsed columns; the guides live inside the name column and count against its width.

### 4.5 Unchanged

One row per node. `TreeGeometry`, hit-testing, selection, collapse, the filter, the viewport and the `C-b 1..9` numbering are untouched. A collapsed node still shows `▸`, now after its guides.

### 4.6 Tests

- exact guide strings for a three-level tree
- the trunk case: the last window of a project has sub-agents, so the rows inside that block begin with two spaces, not `│`
- guides change when a filter hides the last sibling
- guides change when a project is collapsed and re-expanded
- a six-level tree renders without panicking and without truncating the name column to nothing

## 5. Orchestration, simplified

### 5.1 The split

Milestone 8 is the mechanism and contains no model-driven decision, so every state transition is exercised with `fake-agent` in CI. Milestone 9 puts a model in front of it. A broken milestone 9 can only produce a bad plan; it cannot corrupt a repository.

### 5.2 Run and task

```rust
struct Run { id, goal, base_branch, run_branch, check: Option<String>, max_parallel: u8, tasks: Vec<Task> }
struct Task { id, title, brief, runtime, model: Option<String>, owns: Vec<String>, deps: Vec<TaskId>, reviewer: Option<Runtime>, state: TaskState }
```

`owns` is a list of path globs the task is expected to touch. `check` is a shell command run in each task's worktree.

Git layout, unchanged from the earlier design: run branch `anthrex/<run-slug>` off the base branch; each task gets branch and worktree `anthrex/<run-slug>/<task-id>` off the run branch.

### 5.3 Task lifecycle

```
pending → running → verify → review → merging → done
                  ↘ failed        ↘ blocked
```

- **running** — the worker agent runs in the task worktree with the task brief as its prompt.
- **finished** — the agent's status is idle *and* the task branch has at least one commit. Idle with no commit is `failed`, reason "no commit".
- **verify** — `sh -c <check>` in the task worktree, 30-minute timeout. When the run has no `check`, verification is skipped and the run report says so.
- **review** — a reviewer agent is given the task branch's diff. Its verdict arrives through one MCP tool, `submit_review`, as approve or request-changes with comments.
- **one bounce per gate** — a failed check, or a request-changes verdict, returns the task to `running` once with the failure text or the comments appended to its prompt. A second failure of the same gate is `failed` (check) or `blocked` (review).
- **merging** — `git merge --no-ff --no-edit` of the task branch into the run branch, one merge at a time across the whole run. A conflict aborts the merge, marks the task `blocked`, and leaves the rest of the run going. The engine never resolves a conflict.

### 5.4 Reviewer selection

Default: the runtime that is *not* the author's, when both are available; otherwise the same runtime with a different model; otherwise the same runtime and model. A plan may name a reviewer per task and that choice wins.

The reviewer gets its own worktree at the task branch head, `anthrex/<run-slug>/<task-id>-review`, so it cannot amend the work it is judging. Where the runtime supports it, the reviewer is launched read-only (Codex `-s read-only`).

### 5.5 Parallelism and conflict avoidance

`max_parallel` defaults to 3. A task becomes runnable when every task in its `deps` is `done`. In addition, two tasks whose `owns` globs intersect never run at the same time: the engine inserts an implicit dependency from the later task to the earlier one, by plan order, so the outcome is deterministic.

### 5.6 Finishing

A run is complete when no task can make progress. anthrex shows a report: every task with its state, check result, review verdict and commits. You then accept — `--no-ff` merge of the run branch into the base branch — or discard, which deletes the run branch and every task worktree. Neither happens automatically, and nothing reaches the base branch before you choose.

### 5.7 Tools (milestone 9)

The orchestrator agent gets an MCP server with exactly six tools: `get_roster`, `submit_plan`, `run_status`, `task_result`, `revise_task`, `finish_run`. The reviewer agent gets exactly one: `submit_review`. Workers get none; they are ordinary agents with a prompt.

The orchestrator cannot review, merge, or write to the base branch. Those are engine operations.

### 5.8 The planning contract

The orchestrator's prompt template requires every task to carry: a title, a brief of one paragraph, the `owns` globs, acceptance criteria, and the instruction **write the failing test first, then the implementation**. Test-driven development is part of every worker's prompt, not a suggestion.

### 5.9 Routing

`get_roster` returns the runtimes installed, the models available for each, and a one-line note about what each is good for. The orchestrator fills `runtime` and `model` on each task; leaving `model` empty uses that runtime's default.

Routing is a *field on the task*, which is the point. A separate router agent, or a heuristic, can fill that field later without the engine changing.

### 5.10 Cut from the earlier design

Removed for now, and recorded here so nobody implements them by accident: replanning mid-run beyond the single bounce; orchestrators that spawn orchestrators; token and cost accounting; per-task parallelism tuning; automatic conflict resolution; anything that writes to the base branch without the user accepting the run.

## 6. Summary of deltas

**Protocol.** Version 4: `WindowInfo.worktree`, `ServerMsg::Git`.

**Keys.** None added in milestone 4.5. The git segment is display-only.

**CLI.** None added in milestone 4.5.

**Environment.** `ANTHREX_GIT=off`.

## 7. Testing

- The git parser is tested against recorded `--porcelain=v2 -z` output, including unborn, detached, renames, conflicts and paths containing a newline.
- The probe and the watcher are tested against real temporary repositories created by the test, since CI has git. Each test uses its own `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR`.
- The circuit breaker and the coalescing rule are tested with an injected clock, not by sleeping.
- Tree guides are asserted as exact strings.
- The PTY smoke script runs with `ANTHREX_GIT=off` so its screen comparisons do not depend on this repository's working tree.
- Orchestration is exercised end to end with `fake-agent` and scripted worker output: a green task, a task that fails its check once and then passes, a task whose review requests changes, and a task that conflicts on merge.

## 8. What this supersedes

| In `2026-09-18-anthrex-product-design.md` | Status |
|---|---|
| §8.1 run lifecycle | replaced by §5.3 and §5.6 here |
| §8.3 tasks | replaced by §5.2, §5.3, §5.5 here |
| §8.5 roles and the MCP server | replaced by §5.4 and §5.7 here |
| §9.2 how the orchestrator works | replaced by §5.7 and §5.8 here |
| §9.5 model roster and routing | replaced by §5.9 here |
| §9.7 limits | replaced by §5.10 here |
| §10.1 protocol numbers | replaced by §3.5 here |
| everything else | unchanged and still binding |

## 9. Risks

- **The watcher is the risky part.** Filesystem watching is where this design can hurt a machine. The filter, the debounce and the circuit breaker exist for that, and `ANTHREX_GIT=off` is the escape hatch. If the breaker fires often in real use, poll-only becomes the default.
- **`--no-optional-locks` must not be forgotten.** A probe that takes the index lock while an agent runs git is a real failure, and an intermittent one.
- **Guides are computed from the visible tree.** Computing them from the full tree passes most tests and is wrong exactly when a filter is active.
- **One bounce may be too few.** It is deliberately strict: a task that cannot pass its own check twice is more likely to be badly specified than nearly right. If runs block often, the bounce count becomes a setting.
- **Review quality is unmeasured.** Cross-vendor review is a reasoned default, not a measured one. The run report records every verdict so it can be judged after some real runs.
