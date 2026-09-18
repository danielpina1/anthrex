# anthrex product design: milestones 2 to 9

Date: 2026-09-18
Status: approved direction; binding for milestones 2 to 9
Builds on: `docs/superpowers/specs/2026-09-17-anthrex-design.md` (the core design, called "the core spec" below). Where this document and the core spec disagree, this document wins for milestones 2 to 9.

## 1. What changes

The core spec delivers a daemon-backed multiplexer with a flat list of agents. This design adds:

- **Exact agent status and sub-agent tracking** from Claude Code and Codex hooks (milestone 3).
- **A project tree**: project, then agent session, then sub-agents, with live state on every node (milestone 4).
- **Worktrees and the new-agent form**, already specified in core spec sections 3.5 and 6.3 (milestone 5).
- **Persistence, resume, rename, configuration and reconnect**, already specified in core spec section 3.6 with the additions in section 6 below (milestone 6).
- **Split panes** (milestone 7).
- **Orchestration**: a user-chosen orchestrator model plans a goal into tasks and dispatches them to Claude and Codex workers on different models, with a worktree per task, cross-agent review, and merging into a run branch (milestones 8 and 9).

Two items the core spec listed as non-goals are now goals: split panes, and automatic task decomposition. Remote daemons, Windows support, and different sizes per client stay out of scope.

## 2. Terms

| Term | Meaning |
|------|---------|
| Window | One PTY and its child process, as in the core spec. Every agent session is a window. |
| Agent | A window whose runtime is Claude or Codex. |
| Session id | Claude's `session_id` or Codex's thread id, learned from hooks or notify. |
| Sub-agent | An agent that a Claude or Codex session spawns inside itself. It has no PTY of its own; it runs inside its parent's window. |
| Project | A directory tree that groups windows. Defined in section 3. |
| Run | One orchestration: a goal, a plan of tasks, and the windows working on them. |
| Task | One unit of work inside a run, done by one worker window in one worktree. |
| Role | What a window does inside a run: orchestrator, worker, reviewer or integrator. |

## 3. Project model (milestone 4)

1. The daemon computes a window's project root once, before it creates the window, on a blocking thread with a 5-second timeout and off the manager lock. The window is created with its final project, so `WindowInfo.project` never changes. The server runs the detection and the create in a spawned task, so the connection keeps serving other requests meanwhile.
2. If the window's directory is inside a git repository, the root is the parent directory of `git rev-parse --path-format=absolute --git-common-dir`. Linked worktrees of one repository therefore share the main checkout's root, so a worktree agent appears under its repository's project. Exception: when the common directory is not named `.git`, as inside a submodule, the root is `git rev-parse --show-toplevel` instead.
3. Otherwise, or if git fails or times out, the root is the window's canonical directory.
4. `WindowInfo.project` carries the root. The display name is the root's last path component. When two projects share a name, the client appends the parent directory's name, for example `api (work)` and `api (oss)`. If that still collides, the client shows the full root path.
5. Projects are not stored separately. A project exists while at least one window or run belongs to it.

## 4. Agent status and sub-agents (milestone 3)

This section completes core spec section 3.4, whose transition table stays authoritative for window status.

### 4.1 Hook delivery

- `anthrex hook --window <id> --source <claude|codex-notify|codex-hook>` reads the payload from its last argument if present, otherwise from stdin. It sends `HookEvent`, waits for the daemon's `Ack { request: "hook" }`, prints nothing, and always exits 0. The whole command, connect included, runs under one 1-second deadline. A hook must never block or break an agent.
- Waiting for the `Ack` keeps hooks in order: Claude runs hooks one after another, so the daemon applies `PreToolUse` before the matching `PostToolUse`.
- The command drops the top-level `tool_response` key before sending, because it can be megabytes and the daemon never reads it.
- The launcher passes `ANTHREX_WINDOW_ID` and `ANTHREX_SOCKET` to the child, as milestone 1 already does, and embeds the window id in every hook command line.

### 4.2 Claude Code (verified against the 2.1.x documentation)

- The launcher adds `--settings '<json>'` with command hooks for `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PermissionRequest`, `Notification`, `Stop`, `SubagentStart`, `SubagentStop` and `SessionEnd`, each running `'<exe>' hook --window <id> --source claude`, plus `"preferredNotifChannel": "terminal_bell"`. Settings passed this way merge under the user's own settings.
- Every payload carries `session_id` and `hook_event_name`. Events fired inside a sub-agent also carry `agent_id`, the id of that sub-agent.
- `SubagentStart` and `SubagentStop` carry `agent_id` and `agent_type`. The same `agent_id` appears in both, which pairs them.
- Sub-agents are launched by the `Agent` tool. Its `tool_input` has `subagent_type`, `prompt`, and optionally `model` and `name`. Sub-agents nest up to 3 levels by default.
- `Notification` carries `notification_type`, including `permission_prompt` and `idle_prompt`.
- Claude "agent teams" are separate sessions coordinated through files, enabled only by an experimental flag. They are out of scope; if they appear, they are not shown as children.

### 4.3 Codex

Codex status comes from three sources, in order of precedence: lifecycle hooks, the `notify` program on turn completion, and the terminal title that Codex sets to `Starting`, `Working`, `Thinking`, `Waiting` or `Ready`. Section 11 fixes the launch flags, hook trust handling and sub-agent events. Core spec section 3.4's Codex rows stay as written; when lifecycle hooks are available they add the Claude-style rows for `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `Stop` and the sub-agent events.

### 4.4 The sub-agent model

`WindowInfo.subagents: Vec<SubagentInfo>`, where:

```rust
pub struct SubagentInfo {
    pub id: String,                 // Claude agent_id, or the Codex equivalent
    pub parent_id: Option<String>,  // None = spawned by the session itself
    pub kind: String,               // Claude agent_type, e.g. "Explore"; Codex role if known; "agent" if absent
    pub label: Option<String>,      // Agent tool `name`, else the first line of `prompt`, max 60 chars
    pub model: Option<String>,      // Agent tool `model` if given
    pub state: SubagentState,       // Running | Done | Failed, serialized lowercase
    pub tool: Option<String>,       // last PreToolUse tool_name seen with this agent_id, cleared on PostToolUse
    pub started_secs: u64,          // seconds since it started
    pub ended_secs: Option<u64>,    // seconds since it ended
    pub needs_permission: bool,     // a PermissionRequest carried this agent_id and no later event cleared it
}
```

Rules:

1. `SubagentStart` inserts a Running entry. `SubagentStop` marks it Done. If the window's child exits while sub-agents are Running, they become Failed. A `SubagentStart` for an id already tracked, and a `SubagentStop` for an unknown id, are ignored.
2. **Parent attribution is best effort.** When a `PreToolUse` event for the `Agent` tool arrives, the daemon records a pending spawn: the caller's `agent_id` (absent means the session itself), the requested `subagent_type`, and the label and model from `tool_input`. The next `SubagentStart` in the same window with a matching `agent_type` takes the oldest matching pending spawn, first in, first out, and inherits its parent, label and model. A pending spawn with no known type matches any type. With no match, the parent is the session. Pending spawns are capped at 50 per window and dropped after 300 seconds.
3. Finished entries are kept for 300 seconds, then dropped. At most 50 entries are kept per window, oldest finished first. A label longer than 60 characters is cut to 59 characters plus `…`.
4. A sub-agent that needs permission shows as attention on its parent window, because the prompt appears in that window. When `PermissionRequest` carries its `agent_id`, that sub-agent's `needs_permission` is set, and the tree marks its row with `◆` (milestone 4). The next `PreToolUse`, `PostToolUse` or `SubagentStop` for that sub-agent clears it, and input sent to the window clears it on every sub-agent.

### 4.5 Status refinements (milestone 3)

These rules refine core spec 3.4's table. Milestone 3's brief holds the full table.

1. `WindowInfo.tool` is the session's own tool. Events that carry an `agent_id` update that sub-agent's `tool` instead.
2. `PreToolUse` does not clear Attention: with parallel sub-agents one can wait for approval while another calls tools. Attention is cleared by input, `UserPromptSubmit` or `Stop`.
3. Only focusing clears Done. Claude's `idle_prompt` and a Codex `Ready` title no longer turn Done into Idle.
4. With `preferredNotifChannel: "terminal_bell"`, Claude rings on every notification. A Claude window therefore ignores the bell once it has seen a hook; before the first hook, the bell stays the backup signal.
5. A title identical to the window's previous title is dropped.
6. Once lifecycle hooks run in a Codex window, the title only reports `Waiting` (Attention) and a `Ready` that ends a Working turn. `notify` then only ends a Working turn.
7. A Codex window's session id is written once: from the root session's `SessionStart` hook, else from the first notify `thread-id`. A notify whose `thread-id` differs from the known session id is a sub-agent's turn and is ignored. A `/new` typed inside a Codex window is not tracked.
8. A Claude window's session id is the `session_id` of the latest `SessionStart`.
9. A window counts as focused while at least one client is subscribed to it.

## 5. The project tree (milestone 4)

### 5.1 What it shows

The sidebar becomes a tree. For the user's example of one project with three Codex and four Claude sessions:

```
╭ agents ──────────────────────────╮
│ ▾ shop            ◆  cl 4 · cx 3 │
│   ⠹ 1 api-worker    cl opus  2m  │
│   │ ├ ⠹ Explore: map routes   Read│
│   │ └ ✓ tests: run unit suite    │
│   ◆ 2 billing       cx      41s  │
│   ○ 3 search        cx       5m  │
│   ⠹ 4 frontend      cl sonnet 1m │
│   │ └ ⠹ general: style pass Edit │
│   ✓ 5 docs          cl       3m  │
│   ○ 6 infra         cx      12m  │
│   ◌ 7 perf          cl       2s  │
│ ▸ blog            ○  cl 1        │
│                                  │
│ 8 agents · 3 working · 1 attention│
╰──────────────────────────────────╯
```

1. **Project rows** show a collapse marker (`▾` open, `▸` closed), the display name, the most urgent status among the project's windows and runs, and counts per runtime in the order `cl` Claude, `cx` Codex, `sh` shell. Urgency order: attention, working, starting, done, idle, exited.
2. **Agent rows** show the status glyph, the global position number used by `C-b 1..9`, the name, the runtime tag, the model when known, and the elapsed time since the status changed. A worktree window shows its branch as `[<branch>]` after the name (milestone 5); the branch shrinks first when space is short.
3. **Sub-agent rows** hang below their window with tree guides (`├`, `└`, `│`), nested by `parent_id`. They show a state glyph (spinner, `✓`, `✕`), the kind, the label, and the current tool right-aligned and dimmed. A sub-agent whose `needs_permission` is set shows `◆` in the attention colour in place of its state glyph.
4. **Run rows** (milestone 8) sit under their project above plain windows and group the run's windows as children (section 8.8).
5. Projects are ordered by most urgent status, then by name, then by root path. Windows inside a project keep creation order.
6. The sidebar is titled ` agents `, and ` agents · tree ` in tree mode. It scrolls to keep the selection, or outside tree mode the focused window, visible. Rendering and mouse hit-testing share one geometry function.

### 5.2 Interaction

| Keys | Action |
|------|--------|
| `C-b t` | Enter tree mode: plain keys now drive the tree until `Esc`; again to leave |
| `j` / `k` or arrows (tree mode) | Move the selection, without wrapping |
| `Enter` (tree mode) | Agent row: focus the window and leave tree mode. Sub-agent row: focus its parent window. Project row: toggle collapse. Task row (milestone 8): focus its worker. Run row: open the plan view in Planning or AwaitingApproval, the finish view in Ready or a hold (milestone 9), otherwise toggle collapse |
| `Space` (tree mode) | Toggle collapse of the selected project, run or agent |
| `/` (tree mode) | Filter by text over project, window and sub-agent names; `Enter` keeps the filter, `Esc` clears it |
| `Esc` (tree mode) | Leave tree mode; this clears the filter and closes the overview |
| `C-b T` | Full-screen overview in the main area: the same tree, wide, with sub-agent labels, models and durations; `Esc` or `C-b T` returns. The PTY is not resized |
| `C-b <` / `C-b >` | Narrow or widen the sidebar by 4 columns, between 24 and 60 |
| Mouse click | Agent row: focus. Sub-agent row: focus its parent. Project row: toggle collapse. Run row: as `Enter` |
| Mouse wheel over the sidebar | Scroll the tree by 3 rows |

`C-b j`, `C-b k` and `C-b 1..9` walk agent rows in the tree's visible order, skipping collapsed projects and filtered rows. While a filter is set, collapse state is ignored so every match is visible. A paste while navigating the tree is dropped; a paste while typing the filter goes to the filter. The default sidebar width becomes 34 columns. Section 10.3 lists every key binding.

### 5.3 CLI

`anthrex tree [--project <dir>] [--json]` prints the same tree as text, or as JSON with one object per project containing its windows and their sub-agents, nested through `children`. Milestone 8 adds a `runs` array to each project object.

## 6. Worktrees, persistence and configuration (milestones 5 and 6)

Core spec sections 3.5 (worktrees), 3.6 (persistence), 6.3 (dialogs) and 6.5 (config) stand, with these changes and additions:

1. **Worktree branches from runs** (milestone 8) are named `anthrex/<run-slug>/<task-id>`. Worktrees for plain windows keep the core spec's layout under the data directory.
2. **A lifetime lock file**, `daemon.lock` in the data directory, is held with `flock` for the daemon's whole life. A second daemon waits up to 5 seconds for the lock, then exits with an error before it touches the socket. The socket is unlinked at shutdown only while the lock is held and only if it is still the socket this daemon bound. This closes the stale-socket race and the case where a stopping daemon deleted its replacement's socket. `anthrex daemon stop` returns only once the lock is released.
3. **Bind with a restrictive umask** (`0o077`) so the socket is never briefly world-accessible.
4. **Reconnect.** When the connection drops, the client keeps the last screen, shows the disconnected badge and retries every 2 seconds for 30 seconds. On success it re-subscribes to its visible windows. Automatic retries never start a daemon. `C-b r` works only while disconnected: it retries at once, opens a new 30-second window, and may start the daemon.
5. **State file version 2** adds `project`, `session_id`, `model` and run membership to each window record, and a `runs` array (milestone 8). Version 1 files load leniently, with those fields empty. A corrupt file is moved aside and the daemon starts empty; a file with a newer version is moved aside too.
6. **Configuration.** Section 10.4 lists every configuration key and environment variable. `ANTHREX_CONFIG` overrides the config file's path. The core spec's top-level `sidebar_width` is accepted as an alias of `ui.sidebar_width`, with a warning. Parsing never fails: a bad value keeps its default and is reported once.
7. **Worktree directory.** The per-repository directory `<wt>` is `<data_dir>/worktrees/<repo_basename>-<hash8>`, where the repository root is the main checkout, found as in section 3 rule 2, not `--show-toplevel` as core spec 3.5 says. `hash8` is the 32-bit FNV-1a hash of the root path's bytes as 8 lowercase hex digits. The directory name `runs` under `<wt>` and the branch prefix `anthrex/` are reserved for milestone 8.
8. **Worktree create and remove** (milestone 5). A failed create removes the new worktree, and deletes the branch only when this create made it; "the branch is never deleted" applies to removal. Removal checks for a dirty tree first, then kills the window, then runs `git worktree remove`. A removal refused for changes is answered with `Error { request: "remove-dirty" }`, and the client offers `f` force, `k` keep the worktree and remove the window, or `n`/`Esc` cancel. `CreateWindow` and `Remove` run in their own tasks on the server and are never aborted half-way.
9. **Rename and restart** (milestone 6). Rename is `C-b ,`, as in tmux, and restart is `C-b R`. Names are trimmed, 1 to 64 characters, with no control characters. Restarting a live window kills it first. A restart resumes the saved session and never sends the initial prompt again; a restart without a session id is a fresh launch with the initial prompt. A restart runs in the window's saved directory and keeps its worktree record; it never creates a worktree again from `worktree_branch`.
10. **Log rotation.** `daemon.log` rotates at 10 MiB and at most 5 files are kept.

## 7. Split panes (milestone 7)

### 7.1 Model

- The client holds a layout tree: `Pane::Leaf { id: PaneId, window_id }` or `Pane::Split { axis: Row | Column, ratio: u8 (10..=90, percent for the first child), first, second }`. The `PaneId` lets focus stay on a pane when that pane switches to another window.
- At most `panes.max` leaves: default 6, at most 16, which is also the subscription limit per connection. A pane's inner area is at least 20 columns by 4 rows. When the terminal is too small for the layout, the focused pane is shown zoomed until it fits again.
- A window is shown in at most one pane per client. Each visible window has its own client-side parser and scrollback.
- The layout lives in the client and is lost on detach. Saving layouts is a follow-up.
- A split is refused with a toast, and no picker opens, when the pane limit is reached, when there is no room for another pane, or when every window is already shown. The picker lists only existing windows; it has no "new window" entry.
- `C-b w` on the only pane is refused with a toast.
- The Attention and Done toasts are skipped for every window visible in a pane, not only the focused one.

### 7.2 Keys

| Keys | Action |
|------|--------|
| `C-b \|` | Split the focused pane side by side; the new pane opens a picker of windows not already shown |
| `C-b -` | Split the focused pane top and bottom, with the same picker |
| `C-b o` | Focus the next pane |
| `C-b` then arrow | Focus the pane in that direction |
| `C-b` then Shift+arrow | Move the nearest divider on that side by 5 percent |
| `C-b z` | Zoom the focused pane; again to unzoom |
| `C-b w` | Close the focused pane; its window keeps running |

Switching windows with `C-b j`, `C-b k`, `C-b 1..9` or the tree shows the chosen window in the focused pane. If the window is already visible in another pane, focus moves to that pane instead. Section 10.3 lists every key binding.

### 7.3 Protocol

Protocol version 4 (milestone 7): a connection may hold up to 16 subscriptions. `Subscribe { window_id, cols, rows }` adds a subscription, or re-sends a snapshot for an existing one. A seventeenth is refused with `Error { request: "subscribe" }`. `Unsubscribe { window_id }` removes one and is always answered with `Ack { request: "unsubscribe" }`. Server-side, each subscription has its own forwarder task, and the milestone-1 rule "abort, then await, before re-attaching" applies per window. Every subscribed window counts as viewed. Two clients showing one window at different sizes still resolve by last resize wins.

When a client subscribes at the size a window already has and the window shows its alternate screen, the daemon resizes the PTY one row smaller and back, so the program receives SIGWINCH and repaints.

### 7.4 Rendering

Each pane has its own rounded block titled with the window's status glyph, name, runtime and model, and `[zoom]` while zoomed. The focused pane's border uses the accent colour. Keys and pastes go to the focused pane; clicking a pane focuses it. The mouse wheel over a pane focuses that pane first, then scrolls it: it sends wheel reports in the encoding the program asked for when mouse reporting is on, arrow keys when the program shows the alternate screen without mouse reporting, and otherwise scrolls the local scrollback. The sidebar marks every visible window with a bar, in the accent colour for the focused one.

## 8. Orchestration engine (milestone 8)

The engine turns a plan into merged work, deterministically. It runs inside the daemon. In milestone 8 the plan comes from a JSON file and a human drives the decisions from the CLI. In milestone 9 an orchestrator agent writes the plan and makes the decisions through the same interfaces. The engine drives every mechanical step itself; the orchestrator decides plans and exceptions.

### 8.1 Run lifecycle

```
Planning ─▶ AwaitingApproval ─▶ Running ─▶ Integrating ─▶ FinalReview ─▶ Ready ─▶ Finished
    │               │              │            │              │           │
    └───────────────┴──────────────┴────────────┴──────────────┴───────────┴─▶ Cancelled | Failed
```

- **Planning**: the plan is being written. In milestone 8 the plan arrives with the start request, so the run moves straight on.
- **AwaitingApproval**: the plan is valid and waits for the user. `--yes` or `orchestrator.auto_approve = true` skips this.
- **Running**: tasks are dispatched, reviewed and merged. The run branch and its worktree are created on entering Running, so a run cancelled before approval leaves nothing in git.
- **Integrating**: every task is merged or cancelled; the verify command runs on the run branch. A run whose tasks are all cancelled skips verify and the final review and goes to Ready.
- **FinalReview**: one reviewer reviews the whole run diff.
- **Ready**: waiting for the user to choose how to finish (section 8.7).
- **Finished**, **Cancelled**, **Failed**: terminal. Worktrees and branches stay until the user removes them.
- **Paused**: any non-terminal run after a daemon restart. `anthrex run resume` returns it to the state it was in (section 8.8).

**Holds.** Two points in the lifecycle need a decision: a failed run verify in Integrating, and a submitted final review in FinalReview. The same state machine handles them in two modes. A passing run verify moves to FinalReview in both.

| Situation | Run without an orchestrator (milestone 8) | Run with an orchestrator (milestone 9) |
|-----------|-------------------------------------------|----------------------------------------|
| Run verify fails | Stays in Integrating with attention. `anthrex run retry <run> integration` runs verify again, then a new final review. `anthrex run approve <run>` accepts the failure and moves to FinalReview. | Stays in Integrating with `awaiting_orchestrator` set, and queues `verify { scope: run }`. The milestone-8 commands still work. |
| Final review submitted | Moves to Ready on its own. A `changes` verdict adds the attention `final review requested changes; see the report`. | Stays in FinalReview with `awaiting_orchestrator` set, whatever the verdict, and queues `final_review`. |
| Leaving a hold | Not applicable | `add_tasks` moves the run back to Running; the added tasks are merged, then Integrating and FinalReview run again with a new final reviewer. `finish_run` moves it to Ready. The user can also finish a holding run directly with `anthrex run finish` or the finish view. |

### 8.2 Git layout

For a run with slug `<slug>` in a repository whose worktree directory is `<wt>`, which is `<data_dir>/worktrees/<repo_basename>-<hash8>`:

| Thing | Branch | Worktree path |
|-------|--------|---------------|
| Base | the branch checked out in the project root at run start; its commit is recorded as `base_sha` | the project root; never modified by the engine |
| Run branch | `anthrex/<slug>/integration`, created at `base_sha` | `<wt>/runs/<slug>/integration` |
| Task | `anthrex/<slug>/<task-id>`, created from the run branch's head when the task is dispatched | `<wt>/runs/<slug>/<task-id>` |
| Review | detached at the task branch's head when the review starts | `<wt>/runs/<slug>/<task-id>-review-<round>` |
| Final review | detached at the run branch's head | `<wt>/runs/<slug>/final-review-<round>` |

Rules:

1. The engine refuses to start a run when the project root is not a git repository, is on a detached HEAD, has no commit, or has uncommitted changes to tracked files. Untracked files do not count, here and for the finish merge.
2. The slug is the goal, lower-cased, non-alphanumerics replaced by `-`, runs of `-` collapsed and trimmed, cut to 32 characters, plus `-` and 4 random hex digits. The run id is the slug. The CLI accepts the full id, its 4 hex digits, or a unique prefix.
3. All git commands run on blocking threads with a 60-second timeout. Their stderr is kept for error messages.
4. A task that depends on others is dispatched only after all of them are merged, so its branch starts from a run branch that already contains their work.
5. Task ids `integration` and `final`, and ids ending in `-review-<digits>`, are reserved because they collide with the branches and paths above.

### 8.3 Tasks

A plan is a list of tasks:

```json
{
  "goal": "Add password reset",
  "notes": "optional context for every worker",
  "verify": "cargo test --workspace",
  "tasks": [
    {
      "id": "t1",
      "title": "Reset token model and storage",
      "kind": "implement",
      "prompt": "What to do, in full.",
      "acceptance": ["Tokens expire after 30 minutes", "Unit tests cover expiry"],
      "depends_on": [],
      "runtime": "codex",
      "model": null,
      "reviewer": { "runtime": "claude", "model": "claude-sonnet-5" }
    }
  ]
}
```

- `id`: 1 to 16 characters from `[a-z0-9-]`, not starting with `-` (`^[a-z0-9][a-z0-9-]{0,15}$`), unique within the run, and not reserved (section 8.2 rule 5).
- `kind`: one of `implement`, `test`, `refactor`, `docs`, `investigate`, `fix`.
- `runtime` and `model`: which agent does the task. `model: null` means that runtime's default. Both must be in the roster (section 9.5).
- `reviewer`: optional. When absent, the engine picks a reviewer from the roster on the other runtime, at the same tier or higher (section 9.5).
- Validation rejects: duplicate ids, unknown dependencies, cycles, more than `orchestrator.max_tasks` tasks (default 12), and runtime or model outside the roster. Every error names the task and the field.

Task states:

```
Planned ─▶ Ready ─▶ Running ─▶ Submitted ─▶ Verifying ─▶ Reviewing ─▶ Approved ─▶ Merging ─▶ Merged
                       ▲                        │             │                      │
                       └──── ChangesRequested ◀─┴─────────────┘                      ▼
                                                                                  Conflict
   any state ─▶ Blocked | NeedsHuman | Cancelled | Failed
```

1. **Ready**: every dependency is Merged. The engine dispatches Ready tasks in plan order while fewer than `orchestrator.max_parallel` workers (default 3) are Running.
2. **Running**: the engine creates the task worktree and a worker window in it, with the worker role (section 8.5). The first prompt is the task prompt, the acceptance criteria, the run notes, and the worker contract.
3. **Submitted**: the worker called `report_done`, from Running or Blocked. The engine checks that the task branch has at least one commit beyond its start point and no uncommitted changes to tracked files. The answer is the tool call's result, not a typed message, because the worker is still in its turn: if the check fails, the result tells the worker to commit and call `report_done` again, and the task stays Running.
4. **Verifying**: if the run has a verify command, the engine runs it in the task worktree with a 20-minute timeout. Failure sends the command's last 200 lines to the worker, and the task goes to ChangesRequested. A verify failure counts as a review round.
5. **Reviewing**: the engine creates the review worktree and a reviewer window in it (section 8.5). The reviewer calls `submit_review` with `approve` or `changes` and a list of findings.
6. **ChangesRequested**: the engine sends the findings to the worker window and the task returns to Running once the message is delivered. When a round would reach `orchestrator.max_review_rounds` (default 3), the task goes to NeedsHuman instead, so with the default the worker receives two findings messages. Retrying the task resets the count.
7. **Merging**: the engine merges the task branch into the run branch in the run worktree with `git merge --no-ff --no-edit -m "anthrex: merge <task-id>: <title>"`. Merges into one run are serialized.
8. **Conflict**: the merge failed. The engine runs `git merge --abort` and sends the worker "merge `anthrex/<slug>/integration` into your branch, resolve the conflicts, run the tests, commit, then call report_done". The task returns to Running and goes through review again. If the worker window no longer exists, the task goes to NeedsHuman.
9. **Blocked**: the worker called `report_blocked`. **NeedsHuman**: the engine cannot proceed without a decision. Both show as attention in the tree. In milestone 9 they also become events for the orchestrator.

Worker and reviewer windows are ordinary windows. The user can open, watch, type into, or take over any of them.

### 8.4 Sending messages to windows

The engine talks to a running agent by typing into its window. A message is queued per window and delivered only when the window's status is Idle or Done, so it never interrupts a turn. Delivery is the text inside bracketed-paste markers, then a carriage return. Each message starts with `[anthrex]` so the agent and the user can tell it apart from human input. A message that has waited more than 10 minutes marks the window as attention.

### 8.5 Roles, launch and the MCP server

Every window in a run gets the anthrex MCP server. It is a stdio server started as `<exe> mcp --role <role> --run <run-id> [--task <task-id>] --window <window-id> --socket <socket-path>`. `--window` names the calling window, which the engine checks, and `--socket` names the daemon's socket, so the server does not depend on the environment Claude or Codex pass to MCP servers. It forwards each tool call to the daemon over a fresh socket connection and returns the result. The Rust MCP SDK crate `rmcp`, pinned to `=3.4.0` with features `server` and `transport-io`, implements the protocol. It lives in its own crate, `crates/mcp`, so neither the daemon nor the client depends on it.

| Role | Runs in | Can edit code | Tools |
|------|---------|---------------|-------|
| worker | the task worktree | yes | `get_task`, `report_done { summary }`, `report_blocked { reason }`, `ask { question }` |
| reviewer | the review worktree | no | `get_review`, `submit_review { verdict, summary, findings[] }` |
| orchestrator (milestone 9) | the project root | no | section 9.3 |

- `get_task` returns the task, its acceptance criteria, the run notes, the worktree path, the branch, and the start commit.
- `get_review` returns the task, its acceptance criteria, the base and head commits of the change, and previous rounds' findings.
- A finding is `{ severity: "critical" | "important" | "minor", file, line?, summary }`. A verdict of `changes` needs at least one critical or important finding; otherwise the engine treats it as `approve` and keeps the minor findings in the run report.
- `ask` records a question. In milestone 8 it shows as attention on the run. In milestone 9 it goes to the orchestrator, whose `answer` tool sends the reply back to the worker window.
- The engine accepts a tool call only from the role, run, task and window the server was started for. Anything else returns an error. Errors are tool results with `isError: true`, never JSON-RPC errors.

Launch flags per runtime and role:

| | Claude Code 2.1.x | Codex 0.135.0 |
|--|-------------------|---------------|
| MCP server | `--mcp-config '<json>'` with `{"mcpServers":{"anthrex":{"type":"stdio","command":"<exe>","args":["mcp","--role","<role>","--run","<run-id>","--task","<task-id>","--window","<id>","--socket","<path>"]}}}`; the `--task` pair only for a window that has a task | section 11 |
| Pre-approve anthrex tools | `--allowedTools "mcp__anthrex__*"` | section 11 |
| Role instructions | `--append-system-prompt '<text>'` | section 11 |
| Worker permissions | the user's own defaults | the user's own defaults |
| Reviewer permissions | `--permission-mode plan` | section 11 |
| Model | `--model <id or alias>` | `-m <model>` |

Worker permissions stay at the user's own defaults, so workers ask before risky actions and those prompts show as attention in the tree. `orchestrator.worker_permission_mode` can relax this; it is written into the run so the user sees what was chosen.

The Claude role flags go after milestone 3's `--settings` and before `--model`, `--resume` and `--`. `--mcp-config` and `--allowedTools` are variadic, so each is followed by another flag, never by the prompt. A run window's role flags are saved with the window, so a restart relaunches it with the same flags.

### 8.6 The run report

The engine keeps a Markdown report at `<wt>/runs/<slug>/REPORT.md`, outside every worktree. It lists the goal, each task with its runtime, model, reviewer, review rounds, findings, verify results and merge commit, and the final review. The finish step and the plan view link to it.

### 8.7 Finishing a run

Only the user finishes a run. The engine never modifies the base branch and never pushes on its own. In the Ready state, or in a hold (section 8.1) of a run with an orchestrator, the user chooses one of the following. Keep and discard also work on Cancelled and Failed runs.

1. **Merge into base**: requires the base branch to be checked out in the project root with no uncommitted changes to tracked files. Runs `git merge --no-ff --no-edit -m "anthrex: merge run <slug>: <goal>" anthrex/<slug>/integration`. Without `--no-edit` git would open an editor.
2. **Open a pull request**: requires `gh` on the PATH. Pushes the run branch to the base branch's remote, else `origin`, and runs `gh pr create` with the run report as the body. The dialog shows the exact remote, its URL and the branch before it pushes.
3. **Keep**: leaves the branches and worktrees for the user.
4. **Discard**: after the user types the run's slug, removes the run's windows, worktrees and branches.

Merge, pull request and discard need a confirmation that names the run id. The daemon answers an unconfirmed request with the prompt to show.

### 8.8 Milestone 8 control surface

| Command | Effect |
|---------|--------|
| `anthrex run start --plan <file> [--dir <dir>] [--yes]` | Validates the plan and creates the run |
| `anthrex run approve <run>` | AwaitingApproval to Running; also accepts a failed run verify (section 8.1) |
| `anthrex run status [<run>] [--json]` | Run and task states |
| `anthrex run retry <run> <task>\|integration` | NeedsHuman, Blocked or Failed back to Ready, with the review rounds reset; `integration` runs the run verify again |
| `anthrex run skip <run> <task>` | Mark a task Cancelled; its dependants become NeedsHuman |
| `anthrex run finish <run> merge\|pr\|keep\|discard [--yes] [--confirm <run-id>]` | Section 8.7; `merge` and `pr` ask `y/N` unless `--yes`, `discard` asks for the run id unless `--confirm` |
| `anthrex run cancel <run>` | Kill the run's windows and mark it Cancelled |
| `anthrex run resume <run>` | After a daemon restart, relaunch a Paused run's windows with their sessions resumed |

Runs are saved in the state file with their plan, task states, window ids, session ids and pending messages. On daemon start, every run that was not terminal becomes Paused: its windows are listed as exited, as in core spec section 3.6, and nothing is dispatched until the user resumes it. A Paused run accepts only `resume` and `cancel`.

In the tree, a run row reads `◈ <goal> · <merged>/<total>`, with the orchestrator window as its first child (milestone 9), then one row per task with its state, and the task's worker and reviewer windows below it. A run's windows are not listed again as plain windows.

## 9. Orchestrator agent (milestone 9)

### 9.1 Choosing the orchestrator

The user starts a run with `C-b O`, or with `anthrex run start "<goal>" --orchestrator <runtime>[:<model>] [--workers claude,codex] [--verify <cmd>] [--parallel <n>] [--yes] [--dir <dir>]`. For example `--orchestrator claude:claude-opus-5` or `--orchestrator codex`. A runtime without a model picks that runtime's highest-tier roster entry, first in roster order. The form asks for:

- the goal;
- the directory, defaulting to the project selected in the tree, else the client's default directory;
- the orchestrator's runtime and model, defaulting to the first `frontier` roster entry;
- which runtimes may take tasks (Claude, Codex or both), defaulting to both;
- the verify command, defaulting to the verify command of the most recent run in this project; empty means none. A verify command the user gives here wins over the `verify` in the orchestrator's plan; without one, the plan's is used;
- `max_parallel`, 1 to 8, defaulting to `orchestrator.max_parallel`.

The orchestrator is an interactive window in the project root, with the orchestrator role. It counts toward `orchestrator.max_windows`. It can read the code and call its tools. It cannot edit files: Claude runs with `--disallowedTools "Edit,Write,NotebookEdit"`, and Codex with a read-only sandbox (section 11). Its first prompt is the goal. Its role instructions are the contract in section 9.4.

### 9.2 How the orchestrator works

1. It explores the repository and writes a plan, then calls `submit_plan`. Validation errors come back as the tool result, so it can correct them.
2. The run waits in AwaitingApproval. The plan view (section 9.6) lets the user approve, edit or reject with a comment. A rejection returns the run to Planning and comes back to the orchestrator as an event carrying the comment. The user's edits reach it as one `user` event when the user approves.
3. After approval the engine dispatches, verifies, reviews and merges as in section 8. The orchestrator waits in a loop on `wait_for_events`.
4. It decides every exception: blocked tasks, worker questions, review rounds exhausted, conflicts the worker could not resolve, verify failures on the run branch, and the final review's findings. The run holds in Integrating or FinalReview until it decides (section 8.1).
5. When every task is merged or cancelled and it has handled the final review, it calls `finish_run` with a summary for the user, and the run becomes Ready. The user then finishes the run as in section 8.7.

The user's own run commands and typing into any window keep working on an orchestrated run. Each run command the user gives is also queued for the orchestrator as a `user` event. If the orchestrator window exits, the run shows attention and events keep queuing; `anthrex restart` relaunches it with its session and role flags.

### 9.3 Orchestrator tools

| Tool | Effect |
|------|--------|
| `get_run` | The run, its plan, and every task's state, windows, rounds and findings |
| `list_models` | The roster (section 9.5) with tiers and strengths |
| `submit_plan { plan }` | Replace the plan while Planning; returns validation errors |
| `add_tasks { tasks }` | Add tasks while Running or in a hold; each must be valid against the current plan, and `max_tasks` counts every task ever in the plan |
| `reassign { task_id, runtime, model }` | Change who works on a task that is not Running |
| `retry { task_id, note? }` | Send a Blocked or NeedsHuman task back to Ready, optionally with a note for the next worker |
| `cancel_task { task_id, reason }` | Cancel a task that is not Merged |
| `approve_anyway { task_id, reason }` | Accept a task whose review rounds ran out; recorded in the report |
| `answer { task_id, text }` | Reply to a worker's `ask` |
| `send { task_id, text }` | Send any message to a task's worker |
| `wait_for_events { timeout_secs }` | Block up to `timeout_secs` (at most 50) and return up to 50 queued events, oldest first |
| `finish_run { summary }` | From a hold with every task Merged or Cancelled: declare the run Ready for the user |

Events: `plan_rejected { comment }`, `task_state { task_id, from, to }`, `review { task_id, round, verdict, findings }`, `verify { scope: task | run, task_id?, ok, tail }`, `question { task_id, question_id, text }`, `blocked { task_id, reason }`, `needs_human { task_id, why }`, `window_exited { task_id, role, code }`, `final_review { verdict, findings }`, `user { text }`. Each event carries a run-wide sequence number. The queue is saved with the run and holds at most 1000 events. Delivery is at most once. One `wait_for_events` call is open per run; a newer one supersedes it. Tool calls run concurrently, so a long poll never stalls another call.

**Waking the orchestrator.** When the orchestrator window is Idle or Done, events are queued, and no `wait_for_events` call is open, the engine sends it `[anthrex] <n> events waiting; call wait_for_events.` (`1 event waiting` for one) at most once a minute. The user can also type to the orchestrator directly, like any window, or queue a `user` event with `anthrex run tell`.

### 9.4 The role contract

The orchestrator's role instructions say, in substance, the following. The exact text is `ORCHESTRATOR_CONTRACT` in milestone 9's brief; changing it is a spec change.

1. You never edit code. All changes happen through tasks.
2. Make tasks small and independent where possible. A task is one reviewable change with written acceptance criteria. Use `depends_on` only for real ordering needs.
3. Choose a runtime and model per task from `list_models`, following section 9.5. Explain each choice in one line in the task's `prompt` header.
4. Prefer cross-runtime review: a task written by Codex is reviewed by Claude, and the reverse.
5. After `submit_plan`, loop on `wait_for_events` until the run is Ready. Answer questions promptly. Do not approve work that failed review without saying why.
6. Keep the user informed: after each event batch, write a two-line status in your own window.

Worker instructions state the task contract: work only in this worktree, commit with clear messages, run the tests, call `report_done` with a summary, or `report_blocked` or `ask` when stuck, and never push. Reviewer instructions: read the diff between the given commits, check it against the acceptance criteria, run the tests if useful, do not edit files, and call `submit_review`.

### 9.5 Model roster and routing

The roster lists every runtime and model a run may use:

```toml
[[orchestrator.models]]
runtime = "claude"
model = "claude-opus-5"
tier = "frontier"
strengths = ["architecture", "debugging", "review"]

[[orchestrator.models]]
runtime = "claude"
model = "claude-sonnet-5"
tier = "standard"
strengths = ["implementation", "tests", "refactor"]

[[orchestrator.models]]
runtime = "claude"
model = "claude-haiku-4-5"
tier = "fast"
strengths = ["docs", "mechanical edits"]

[[orchestrator.models]]
runtime = "codex"
model = ""          # empty = Codex's configured default model
tier = "standard"
strengths = ["implementation", "tests"]
```

- The built-in default roster is exactly the four entries above. Users add Codex models by id in their config; anthrex does not guess Codex model names.
- **User entries extend the built-in roster.** A user entry with the same runtime and model as a built-in entry replaces it in place; other entries are appended in file order. `orchestrator.builtin_models = false` drops the built-ins, so the roster is only the user's entries. An invalid entry (unknown runtime or tier, an empty Claude model, a duplicate among the user's entries, more than 8 strengths or one longer than 40 characters) is skipped with a warning naming its index. If no entry is left, the built-in roster is used. Milestone 8 implements these rules.
- Tiers are `fast`, `standard` and `frontier`, in that order. A task whose `model` is null has the tier of its runtime's entry with the empty model, else `standard`.
- **Default reviewer**, when a task names none: an entry on the other runtime at the author's tier or higher, lowest such tier first, then roster order. If there is none, or the other runtime is not allowed in the run, an entry on the same runtime with a different model at the author's tier or higher; then the other runtime at any tier, highest first; then the author itself. The final reviewer is the first `frontier` entry, else the first entry of the highest tier present.
- **Routing guidance** given to the orchestrator: mechanical edits and docs go to `fast`. Ordinary implementation, tests and refactors go to `standard`. Cross-cutting design, subtle concurrency, debugging and final review go to `frontier`. Spread independent tasks across both runtimes when both are allowed. Give each review to the other runtime at the author's tier or higher.
- The engine enforces only membership in the roster and the runtimes allowed for the run, for workers and reviewers. Every allowed runtime needs at least one roster entry. The guidance is advice to the orchestrator; the user can override any assignment in the plan view.

### 9.6 Plan view and finish view

When a run is Planning or AwaitingApproval, selecting it in the tree opens the plan view in the main area. While Planning, the view says `The orchestrator is planning.`

```
╭ plan · add password reset · orchestrator claude-opus-5 ─────────────────────╮
│  #  id  task                               runtime  model            deps   │
│  1  t1  Reset token model and storage      codex    (default)        -      │
│  2  t2  Email sender for reset links       claude   claude-sonnet-5  -      │
│  3  t3  Reset endpoints and rate limiting  codex    (default)        t1 t2  │
│  4  t4  Docs for the reset flow            claude   claude-haiku-4-5 t3     │
│                                                                              │
│  a approve   e edit task   d drop task   r reject with comment   Esc back    │
╰──────────────────────────────────────────────────────────────────────────────╯
```

Editing a task changes its runtime, model or prompt within the roster and the run's allowed runtimes. Dropping a task fails when others depend on it. The plan view shows `-` for a task without dependencies and `(default)` for an empty model. When the run is Ready or in a hold, the same area shows the finish view: the orchestrator's summary, the run report's headline numbers, and the four choices from section 8.7, each with a confirmation. The finish view asks the daemon for a preview first, built by the same function the finish action uses, so the dialog shows exactly what will run.

### 9.7 Limits

| Key | Default | Meaning |
|-----|---------|---------|
| `orchestrator.max_parallel` | 3 | Worker windows Running at once, 1 to 8; the form and `--parallel` override it per run |
| `orchestrator.max_tasks` | 12 | Tasks in one plan, including added and cancelled ones |
| `orchestrator.max_review_rounds` | 3 | Review rounds before NeedsHuman; a verify failure counts as a round |
| `orchestrator.max_windows` | 20 | All windows a run may create, including reviewers, final reviewers and the orchestrator; restarts do not count |
| `orchestrator.auto_approve` | false | Skip AwaitingApproval |
| `orchestrator.worker_permission_mode` | `"default"` | Claude permission mode for workers |
| `orchestrator.worker_codex_sandbox` | unset | Codex sandbox for workers; unset keeps the user's default |

A run copies these limits from the config when it starts, so a later config change never affects it. Section 10.4 gives the valid ranges. anthrex cannot see token spend inside interactive sessions, so it has no cost budget. The window and round limits bound the work instead.

## 10. Protocol, CLI, keys and configuration summary

### 10.1 Protocol

**Version rule.** A milestone that changes a message shape sets `proto::PROTO_VERSION` to one more than the value on `main` when it starts. The numbers below, and in the briefs, assume the roadmap order; if milestones merge in another order, each takes the next free number and records it in its implementation notes. A milestone that changes no shape keeps the value on `main`.

| Version | Milestone | Messages and types added or changed |
|---------|-----------|-------------------------------------|
| 1 | 1 | Initial |
| 1 | 2 | No change |
| 2 | 3 | `HookEvent` handled and answered with `Ack { request: "hook" }`. `HookSource` gains `CodexHook` (`"codex-hook"`). `WindowInfo` gains `session_id: Option<String>` (replacing `has_session`), `model: Option<String>` and `subagents: Vec<SubagentInfo>`. New `SubagentInfo` and `SubagentState` |
| 3 | 4 | `WindowInfo.project: PathBuf`, after `cwd` |
| 3 | 5 | No shape change. `proto::messages::request` constants `CREATE = "create"`, `REMOVE = "remove"`, `REMOVE_DIRTY = "remove-dirty"`; a worktree removal refused for changes is answered with `Error { request: "remove-dirty" }` |
| 3 | 6 | No shape change. `Restart` is answered with `Ack { request: "restart" }` or `Error { request: "restart" }`. `proto::HANDSHAKE_TIMEOUT` (5 s) bounds both sides of the handshake |
| 4 | 7 | `Unsubscribe { window_id }` replaces the unit variant, answered with `Ack { request: "unsubscribe" }`. Up to 16 subscriptions per connection; the limit is answered with `Error { request: "subscribe" }` |
| 5 | 8 | `WindowInfo.run: Option<RunRef { run_id, task_id, role }>`. `ClientKind::Mcp`. New types in `proto::run`: `Role`, `RunRef`, `TaskKind`, `ModelRef`, `PlanTask`, `Plan`, `RunState`, `TaskState`, `Severity`, `Finding`, `Verdict`, `FinishAction`, `TaskInfo`, `RunInfo`, `Tier`, `ModelEntry`. `ClientMsg`: `RunStart`, `RunApprove`, `RunRetry`, `RunSkip`, `RunFinish`, `RunCancel`, `RunResume`, `ToolCall`. `DaemonMsg`: `RunsChanged { runs }`, `RunCreated`, `RunConfirmNeeded`, `RunFinished`, `ToolResult`. `Welcome` gains `runs`. Request constants `RUN_START` to `RUN_RESUME` (`"run start"` and so on) and `TOOL` |
| 6 | 9 | `ToolCall` and `ToolResult` gain `call_id: u64`; new `ToolCancel { call_id }`. `ClientMsg`: `StartOrchestratedRun`, `GetRunDefaults`, `EditPlanTask`, `DropPlanTask`, `RejectPlan`, `TellOrchestrator`, `GetFinishPreview`. `DaemonMsg`: `RunStarted`, `RunDefaults`, `FinishPreview`. `RunInfo` gains `orchestrator`, `allowed_runtimes` and `stats`; `TaskInfo` gains `prompt`, `acceptance`, `approved_anyway` and `open_questions`. New types `OrchestratedRunSpec`, `OrchestratorChoice`, `OrchestratorInfo`, `RunStats`, `FinishPreview`, `RunDefaults`, `VerifyScope`. Request constants `"run edit"`, `"run drop"`, `"run reject"`, `"run tell"`, `"run defaults"`, `"run preview"`; `StartOrchestratedRun` errors use `"run start"` |

The briefs hold the exact fields. No two milestones define the same message.

### 10.2 CLI

CLI commands added after milestone 1: `anthrex hook` (3, hidden), `anthrex ls --json` (3), `anthrex tree` (4), `anthrex new --worktree <branch>` and `anthrex rm --worktree [--force]` (5), `anthrex rename` and `anthrex restart` (6), `anthrex run start|approve|status|retry|skip|finish|cancel|resume` (8), `anthrex mcp` (8, hidden), `anthrex run start "<goal>" --orchestrator ...`, `anthrex run reject` and `anthrex run tell` (9).

### 10.3 Keys

Every binding after the prefix (`C-b` by default, configurable with `prefix`). No two bindings share a key. `Char` keys match on the character only, so `o` and `O` are different keys and a Shift reported with `|`, `<`, `>`, `T`, `X`, `Q`, `R` or `O` does not matter. Arrows match on their modifiers.

| Keys | Action | Milestone |
|------|--------|-----------|
| `j`, `n` / `k`, `p` | Next / previous window, in the tree's visible order from milestone 4 | 1 |
| `1` to `9` | Focus the window at that position | 1 |
| `c` | New window; the new-agent form from milestone 5 | 1, 5 |
| `x` | Kill the focused window (confirm) | 1 |
| `X` | Remove the focused window (confirm; worktree checkbox from milestone 5) | 1, 5 |
| `s` | Toggle the sidebar | 1 |
| `d` | Detach | 1 |
| `Q` | Stop the daemon and all agents (confirm); quits once the daemon confirms (milestone 6) | 1 |
| `?` | Help | 1 |
| the prefix again | Send the prefix key to the program | 1 |
| `Esc` | Cancel prefix mode | 1 |
| `t` | Tree mode on or off | 4 |
| `T` | Tree overview on or off | 4 |
| `<` / `>` | Narrow / widen the sidebar by 4 columns | 4 |
| `,` | Rename the focused window | 6 |
| `R` | Restart the focused window; asks first when it is running | 6 |
| `r` | Reconnect; only while disconnected, otherwise a `connected` toast | 6 |
| `\|` | Split the focused pane side by side | 7 |
| `-` | Split the focused pane top and bottom | 7 |
| `o` | Focus the next pane | 7 |
| arrows | Focus the pane in that direction | 7 |
| Shift+arrows | Move the nearest divider on that side by 5 percent | 7 |
| `z` | Zoom the focused pane on or off | 7 |
| `w` | Close the focused pane; its window keeps running | 7 |
| `O` | New orchestrated run form | 9 |

Keys without the prefix apply only in a mode: tree mode (section 5.2, milestone 4), the new-agent form, the remove and force dialogs (milestone 5), the rename prompt (milestone 6), the pane picker (milestone 7), the run form, the plan view and the finish view (section 9.6, milestone 9). An open modal takes keys before tree mode. Outside a mode, every key without the prefix goes to the focused program, including Shift+arrows.

### 10.4 Configuration and environment

Configuration keys in `config.toml`. Milestone 6 adds the file and parses every key outside `[orchestrator]`; milestone 8 parses `[orchestrator]`. A value of the wrong type or out of range keeps its default and is reported once.

| Key | Default | Valid values | Milestone | Consumer |
|-----|---------|--------------|-----------|----------|
| `prefix` | `"C-b"` | `"C-"` and one lowercase letter except `h`, `i`, `j`, `m` | 6 | client keymap, help and hints |
| `accent` | `"#89b4fa"` | `#` and six hex digits | 6 | client theme |
| `bell.attention` | `true` | boolean | 6 | client: bell when a background window needs attention |
| `bell.done` | `false` | boolean | 6 | client: bell when a background window is done |
| `default_runtime` | `"shell"` | `"claude"`, `"codex"`, `"shell"` | 6 | the new-agent form's first runtime; `anthrex new` without `--runtime` |
| `scrollback_lines` | `5000` | 0 to 100000 | 6 | client parsers, one per visible window from milestone 7 |
| `ui.sidebar_width` | `34` | 24 to 60 | 6 | client: starting sidebar width (milestone 4 steps it with `C-b <` and `C-b >`). Top-level `sidebar_width` is an alias, with a warning |
| `ui.tree_keep_finished_secs` | `300` | 0 to 300 | 6 | client tree: hides finished sub-agent rows older than this |
| `panes.max` | `6` | 1 to 16 | 6 | client: the pane limit of milestone 7 |
| `runtimes.claude.command` | `"claude"` | a non-empty program name or path | 6 | daemon launcher |
| `runtimes.codex.command` | `"codex"` | a non-empty program name or path | 6 | daemon launcher and the Codex version probe |
| `runtimes.codex.bypass_hook_trust` | `false` | boolean | 6 | daemon launcher: pass `--dangerously-bypass-hook-trust` to Codex windows (section 11.3) |
| `orchestrator.max_parallel` | `3` | 1 to 8 | 8 | engine dispatch; default of the run form (milestone 9) |
| `orchestrator.max_tasks` | `12` | 1 to 50 | 8 | plan validation, `add_tasks` |
| `orchestrator.max_review_rounds` | `3` | 1 to 10 | 8 | engine review loop |
| `orchestrator.max_windows` | `20` | 1 to 100 | 8 | engine window creation |
| `orchestrator.auto_approve` | `false` | boolean | 8 | engine start |
| `orchestrator.worker_permission_mode` | `"default"` | a non-empty Claude permission mode | 8 | worker launch |
| `orchestrator.worker_codex_sandbox` | unset | `"read-only"`, `"workspace-write"`, `"danger-full-access"` | 8 | worker launch |
| `orchestrator.builtin_models` | `true` | boolean | 8 | roster (section 9.5) |
| `[[orchestrator.models]]` | the built-in roster | entries of `runtime`, `model`, `tier`, `strengths` (section 9.5) | 8 | roster, plan validation, reviewer choice, `list_models` |

Environment variables:

| Variable | Milestone | Read by | Meaning |
|----------|-----------|---------|---------|
| `ANTHREX_SOCKET` | 1 | daemon, client, CLI, `anthrex hook` | Socket path. The launcher also sets it in every child |
| `ANTHREX_DATA_DIR` | 1 | daemon, CLI | Data directory |
| `ANTHREX_LOG` | 1 | daemon | `tracing` filter for `daemon.log` |
| `ANTHREX_WINDOW_ID` | 1 | agents, `fake-agent` | Set by the launcher in every child |
| `ANTHREX_SMOKE_KEEP` | 1 | `scripts/pty-smoke.py` | Keep the smoke test's data directory |
| `ANTHREX_CLAUDE_BIN` | 3 | daemon | Program for Claude windows; wins over `runtimes.claude.command` |
| `ANTHREX_CODEX_BIN` | 3 | daemon | Program for Codex windows; wins over `runtimes.codex.command` |
| `FAKE_AGENT_SCRIPT` | 3 | `fake-agent` | Path of the default script (section 12) |
| `FAKE_AGENT_ARGS_FILE` | 3 | `fake-agent` | If set, `fake-agent` writes its argv there as JSON |
| `ANTHREX_CONFIG` | 6 | daemon, client, CLI | Path of `config.toml` |
| `FAKE_AGENT_MESSAGE`, `FAKE_AGENT_RESULT` | 8 | `sh` steps of `fake-agent` | The last message read and the last tool result |
| `FAKE_AGENT_LOG` | 9 | `fake-agent` | If set, one JSON line per MCP call and per message read |
| `ANTHREX_NUDGE_INTERVAL_MS` | 9 | daemon, tests only | Overrides the 60-second waking interval |

## 11. Codex specifics

### 11.1 Versions

Facts in this section were verified against the Codex source at tag `rust-v0.135.0` and the help output of the installed CLI, which is 0.155.0. Codex changes quickly. Every milestone that launches Codex starts with a task that checks the flags it uses against `codex --help`, `codex resume --help` and `codex features list` for the installed version, and records the result in the brief's implementation notes. The daemon runs `codex --version` once at startup on a blocking thread with a 5-second timeout and logs a warning below 0.135.0.

### 11.2 Launch flags for every Codex window

```
codex -C <cwd>
  -c 'notify=["<exe>","hook","--window","<id>","--source","codex-notify"]'
  -c 'tui.terminal_title=["status"]'
  -c 'tui.notifications=["approval-requested"]'
  -c 'tui.notification_method="bel"'
  -c 'tui.notification_condition="always"'
  [hook flags from 11.3]
  [role flags from 11.5]
  [-m <model>]
  [-- <initial prompt>]
```

- `notify` runs the program with the JSON payload as its last argument, not on stdin, only for `agent-turn-complete`. The payload has `thread-id`, `turn-id`, `cwd`, `input-messages` and `last-assistant-message`.
- The title reports `Starting`, `Working`, `Thinking`, `Waiting` or `Ready`.
- The bell rings when an approval is pending.
- `--full-auto` no longer exists; `-s workspace-write` replaces it. `-a/--ask-for-approval` takes `untrusted`, `on-failure`, `on-request` (the default) or `never`. `-s/--sandbox` takes `read-only`, `workspace-write` or `danger-full-access`.

### 11.3 Codex lifecycle hooks and trust

Codex 0.135.0 has command hooks for `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `PostToolUse`, `PreCompact`, `PostCompact`, `SubagentStart`, `SubagentStop` and `Stop`. They are enabled by default, but each hook definition must be trusted before it runs; an untrusted one causes a review prompt at startup.

- Trust is stored as `hooks.state."<key>".trusted_hash = "sha256:<hex>"`. The key is `<source-file-path>:<event_snake_label>:<group_index>:<handler_index>`. The hash is SHA-256 over the canonical, sorted-key JSON of `{event_name, matcher, hooks: [normalized command with timeout defaulting to 600]}`. Both are deterministic, so anthrex can compute them.
- Configuration overrides given with `-c` are merged as dotted paths into the configuration before parsing, so `-c` can supply both the hooks and their trust hashes.
- `--dangerously-bypass-hook-trust` runs every enabled hook regardless of trust, including the user's own untrusted hooks. anthrex never passes it by default.

The key and hash formats above are a reading of the source. Milestone 3 verifies them against the installed Codex and may change its test vectors.

**Decision.** Milestone 3 supplies anthrex's hooks and their trust hashes with `-c`. Its first Codex task verifies, against the installed version, which source path the key uses for `-c` overrides and that the startup review does not appear. If that cannot be made to work, Codex windows run without lifecycle hooks: status comes from notify, the title and the bell, and Codex sub-agent rows stay empty. `runtimes.codex.bypass_hook_trust = true` (milestone 6 config) lets a user opt into the bypass flag knowingly: the launcher then passes `--dangerously-bypass-hook-trust` and anthrex's hook flags, without the trust-hash flags.

### 11.4 Codex sub-agents

- Codex spawns sub-agents with its built-in `spawn_agent` tool, feature `multi_agent`, enabled by default. Nesting depth is `agents.max_depth`, default 1.
- `SubagentStart` and `SubagentStop` payloads carry `agent_id` and `agent_type`. `agent_id` equals the sub-agent's own thread id, and its `session_id` is that same id, so start and stop pair trivially. `SubagentStop` also carries `last_assistant_message`. **No payload names the parent.**
- `PreToolUse`, `PermissionRequest`, `PostToolUse` and `UserPromptSubmit` fired inside a sub-agent carry its `agent_id` and `agent_type`. The root session's `Stop` does not; a sub-agent's turn end is `SubagentStop`.
- Parent attribution uses the rule in section 4.4 item 2, with the `spawn_agent` tool in place of Claude's `Agent` tool. The field names of `spawn_agent`'s `tool_input` are verified in milestone 3; until then the label falls back to `agent_type`.
- Each Codex sub-agent also gets its own rollout file whose first line records `forked_from_id`, the parent thread id. anthrex does not read rollout files; they are an internal format.
- Because a Codex sub-agent's hooks report the sub-agent's own thread id as `session_id`, the daemon must keep the window's session id from `SessionStart` of the root session and never overwrite it with a sub-agent's.

### 11.5 Codex in orchestration roles

| Need | Codex flags |
|------|-------------|
| anthrex MCP server | `-c mcp_servers.anthrex.command="<exe>"`, `-c 'mcp_servers.anthrex.args=["mcp","--role","<role>","--run","<run>","--task","<task>","--window","<id>","--socket","<path>"]'` (the `--task` pair only for a window that has a task), `-c mcp_servers.anthrex.tool_timeout_sec=120` |
| Pre-approve anthrex tools | `-c mcp_servers.anthrex.default_tools_approval_mode="auto"`; milestone 8 verifies that this value suppresses the prompt in the installed version |
| Role instructions | `-c developer_instructions="<text>"`, with the text escaped as a TOML basic string by milestone 3's `launch::codex::toml_string` |
| Reviewer and orchestrator | `-s read-only -a on-request` |
| Worker | the user's own defaults; `orchestrator.worker_codex_sandbox = "workspace-write"` relaxes it |
| Model | `-m <model>`; the default model name is not in the source, so an empty roster model means "omit `-m`" |

`tool_timeout_sec` is set explicitly because the documented default of 60 seconds is too close to the 50-second `wait_for_events` long-poll.

The flags go after milestone 3's `-c` overrides and before `-m`, `resume` and `--`, in this order: the four `mcp_servers.anthrex` flags, `developer_instructions`, then the sandbox and approval flags. The orchestrator (milestone 9) uses the same flags with role `orchestrator` and no `--task`.

### 11.6 Resuming Codex sessions

`codex resume <thread-id>` resumes an interactive session. The thread id comes from `SessionStart` when hooks run, and otherwise from the first notify payload. Codex offers no way to preassign or learn the id at launch. Checked against codex-cli 0.155.0: `codex resume --help` lists `-C`, `-c`, `-m` and `[PROMPT]`, and root options before `resume` are accepted. anthrex keeps `-m` on resume and never passes a prompt on resume. Milestone 6 repeats the check against the installed version.

## 12. Testing with a fake agent

Real `claude` and `codex` need accounts and network, so automated tests never run them. From milestone 3 on:

1. The launcher reads the programs to run from `runtimes.claude.command` and `runtimes.codex.command` in the config (milestone 6), overridden by the environment variables `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN`. Defaults are `claude` and `codex`.
2. A test-only binary crate, `crates/fake-agent` (package `anthrex-fake-agent`), builds `fake-agent`. It accepts every flag anthrex passes to either runtime and ignores the ones it does not use. `--version` prints `codex-cli 0.155.0`. When `FAKE_AGENT_ARGS_FILE` is set, it first writes its argv there as JSON.
3. **Which script runs.** From milestone 8, when the argv carries an anthrex MCP server (Claude `--mcp-config` or Codex `-c mcp_servers.anthrex.*`), `fake-agent` reads the role and task from its arguments and takes the first unclaimed file `<git common dir>/fake-agent/<role>-<task>-<n>.jsonl`, smallest `n` first, where `<task>` is `run` when the server has no `--task` (the final reviewer and the orchestrator). It claims a file by creating `<file>.claimed`. Otherwise it reads the file named by `FAKE_AGENT_SCRIPT`.
4. **Steps.** The script is JSON lines, one step per line. After the last step, unless it was `exit`, `fake-agent` reads stdin until EOF, so the window stays alive like an interactive agent. Each step is defined once, by the milestone that owns it:

   | Step | Effect | Milestone |
   |------|--------|-----------|
   | `{"print": "text"}` | Writes the text to the terminal | 3 |
   | `{"hook": "PreToolUse", "payload": {...}}` | Runs the hook command from the injected Claude settings or Codex `-c hooks.*` flags, payload on stdin, with `hook_event_name`, `session_id` and `cwd` filled in when absent | 3 |
   | `{"notify": {...}}` | Runs the Codex notify program with the payload as its last argument, `type` and `thread-id` filled in when absent | 3 |
   | `{"title": "Working"}` | Sets the terminal title, terminated with ST so it never counts as a bell | 3 |
   | `{"bell": true}` | Rings the bell | 3 |
   | `{"wait_ms": 200}` | Sleeps | 3 |
   | `{"read_line": true}` | Waits for a line of input | 3 |
   | `{"git_commit": {"file": "a.txt", "content": "x", "message": "m"}}` | Writes, stages and commits in the current directory | 3 |
   | `{"exit": 0}` | Exits with that code | 3 |
   | `{"mcp_call": {"tool": "report_done", "args": {...}}}` | Calls a tool on the injected MCP server and prints the result. Milestone 3 parses it and exits 3 when it runs; milestone 8 implements it | 3, 8 |
   | `{"read_message": {"timeout_ms": 10000, "expect": "text"}}` | Reads one `[anthrex]` message delivered by bracketed paste and Enter; exits 3 when `expect` is not in it, 4 on timeout | 8 |
   | `{"sh": "command"}` | Runs `/bin/sh -c` in the current directory with `FAKE_AGENT_MESSAGE` and `FAKE_AGENT_RESULT` set | 8 |
   | `{"mcp_wait": {"tool": "wait_for_events", "args": {...}, "until": {...}, "deadline_ms": 60000}}` | Repeats the call until an event matches every key of `until`; exits 4 at the deadline | 9 |
   | `{"expect": {"is_error": false, "contains": "text"}}` | Asserts on the last call's result; exits 3 on mismatch | 9 |

   `FAKE_AGENT_LOG` (milestone 9) makes `fake-agent` append one JSON line per MCP call and per message read.
5. End-to-end tests live in `crates/cli/tests/`, where `CARGO_BIN_EXE_anthrex` is the real binary. Each starts its own real daemon with an isolated socket and data directory and `fake-agent` as both runtimes. `cargo test --workspace` builds `fake-agent`. The tree, orchestration and review tests drive the daemon with fake agents in temporary git repositories. Only the manual checks use real `claude` and `codex`.

## 13. Risks

1. **Hook payloads change between agent versions.** Parse defensively: unknown fields are ignored, missing ones degrade to the milestone-1 output-activity fallback. Log unparseable payloads at debug level.
2. **Codex hook trust and version drift.** The installed Codex is newer than the source that was studied. If anthrex's hooks cannot be pre-trusted, Codex sub-agent rows stay empty and status relies on notify, the title and the bell (section 11.3).
3. **Typing into agent windows.** Messages are delivered only when the window is Idle or Done, but an agent's TUI could still treat pasted text differently from typed text. The manual checks for milestones 8 and 9 cover both runtimes.
4. **Long runs.** A run can outlive a daemon restart. Milestone 6 persistence covers windows; milestone 8 extends it to runs and pauses them on restart. The user resumes them with `anthrex run resume`, which relaunches the windows with their sessions resumed.
5. **Merge semantics.** The engine merges with `--no-ff` so each task stays one visible merge on the run branch. Rebasing is out of scope.
