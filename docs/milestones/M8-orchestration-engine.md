# Milestone 8: Orchestration engine: runs, tasks, worktrees, review and merge

## Header

| | |
|--|--|
| Status | `blocked` |
| Depends on | Milestones 5 and 6 |
| Spec sections | Product spec 8 in full (8.1 to 8.8, including the no-orchestrator column of the holds table in 8.1), 11.5 (worker and reviewer rows), 10.1 (the version-5 row), 10.4 (the `[orchestrator]` keys), 5.1 item 4 (run rows), 6 items 1 and 5 (run branches, `runs` in the state file), 9.5 (the built-in roster, the user-roster rules and reviewer choice), 9.7 (the limits the engine enforces), 12 (the `fake-agent` steps this milestone owns). |
| Branch | `m8-orchestration-engine` |
| Protocol version | 5. Rule (product spec 10.1): set `PROTO_VERSION` to one more than the value on `main` when you start; 5 assumes roadmap order, with milestone 7 merged. |

## Starting point

Milestones 3 to 7 are merged before this one starts, in roadmap order. This brief uses the names of their briefs (`docs/milestones/M3-agent-status.md` to `M7-split-panes.md`). If the merged code differs, use the real names and record the mapping in "Implementation notes".

| From | What this milestone uses |
|------|--------------------------|
| M3 | `anthrex hook`, hook-driven status (Claude `SessionStart` makes a window Idle, `Stop` makes it Done or Idle, Codex notify makes it Done). `ManagerConfig { socket_path, shell, exe, claude_bin, codex_bin, codex_hook_source }`; `ManagerConfig.exe` is the absolute path of the anthrex executable, called `<exe>` below. The launcher in `crates/daemon/src/launch/` (`mod.rs` with `LaunchContext` and `plan`, `claude.rs`, `codex.rs` with `toml_string`). Process-group kill. The per-window input queue capped by bytes. `crates/fake-agent` (package `anthrex-fake-agent`) with the ten steps of M3's script format, and the `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` overrides. The end-to-end harness `crates/cli/tests/support/mod.rs` with `fake_agent_bin()` and `TestDaemon`. |
| M4 | `WindowInfo.project`; `crates/daemon/src/project.rs` (`resolve_root`). The tree model in `crates/tui/src/tree.rs` with its extension point for run rows (M4 decision 26: `NodeKey`, `ProjectChild` and `RowKind` each carry a "Milestone 8 adds ..." comment, and no `match` on them has a `_` arm), its renderer `crates/tui/src/ui/tree_view.rs`, and tree-mode keys in `crates/tui/src/tree_input.rs`. `anthrex tree --json` in `crates/cli/src/tree_cmd.rs`, whose project objects gain a `runs` array here. |
| M5 | `crates/daemon/src/git.rs` with `git::run(dir, args, deadline)`, `GitOutput` and `GitError`. `crates/daemon/src/worktree.rs` with `repo_root`, `repo_worktrees_dir` (the `<wt>` directory, `<data_dir>/worktrees/<repo_basename>-<hash8>`), `RESERVED_DIR = "runs"` and `RESERVED_BRANCH_PREFIX = "anthrex/"`. `WindowManager::create(spec, project, cols, rows)` is `async`. `ManagerConfig.worktrees_root`. `proto::messages::request` constants. The daemon test helper `TempRepo` in `crates/daemon/tests/common/mod.rs`. |
| M6 | The `config` crate (`config::Config`, `config::parse`, `config::load`, `Problem`) and `ANTHREX_CONFIG`. It skips the `[orchestrator]` table; this milestone parses it. `daemon::state` with `StateFile { version: 2, next_id, windows, runs }`, `WindowRecord.run`, `load`, `save`, `spawn_persister`. `WindowManager::restore`, `state_snapshot` and `restart(self: &Arc<Self>, id)`, which relaunches with the saved session id. `LaunchContext.resume`. The lock file. The client state in `crates/tui/src/app/mod.rs` with tests in `crates/tui/src/app/tests.rs`. |
| M7 | Nothing. The engine must not depend on split panes. |

Names in this brief that come from milestone 1 are real and checked against `main`: `WindowManager`, `Entry`, `Window::write_input`, `launch::plan`, `LaunchPlan`, `LaunchContext`, `status::next`, `StatusEvent`, `server::serve`, `handle_client`, `CliClient`, `CliClient::request`, `resolve_dir`, `expect_ack`, `proto::ClientMsg`, `proto::DaemonMsg`, `proto::WindowInfo`, `proto::WindowSpec`, `proto::ClientKind`, `proto::Runtime`, `proto::Status`, `scripts/pty-smoke.py`. Milestone 6 moved `spawn::ensure_daemon` to `tui::spawn::ensure_daemon(exe, socket)`.

## Goal

A user writes a plan file with a goal and a list of tasks and runs `anthrex run start --plan plan.json`. The daemon validates it, waits for `anthrex run approve`, then does every mechanical step itself: it creates a run branch and a worktree per task, starts a Claude or Codex worker window in each, checks that the worker committed, runs the verify command, starts a reviewer window on the other runtime, sends findings back to the worker, merges approved work into the run branch, sends merge conflicts back to the worker, verifies and reviews the whole run, and waits for the user to merge, open a pull request, keep or discard. Workers and reviewers talk to the engine through the anthrex MCP server. The engine is fully testable with `fake-agent` and no model. Runs survive a daemon restart as Paused and continue with `anthrex run resume`. The tree shows each run with its tasks and their windows.

## Scope

In:

- The run engine inside the daemon: plan validation, run and task state machines, git layout, verify, review rounds, merge queue, conflicts, integration verify, final review, finishing, the run report.
- The anthrex MCP server (`anthrex mcp`) with the worker and reviewer tools, and protocol version 5.
- Launch flags for worker and reviewer windows on Claude and Codex.
- Message delivery into agent windows.
- The `[orchestrator]` config section keys from spec 9.7 and the roster from spec 9.5, with the built-in default roster and reviewer selection.
- `anthrex run start|approve|status|retry|skip|finish|cancel|resume`.
- Runs in the state file, Paused after a restart, resume.
- Run rows and task rows in the tree and in `anthrex tree`.
- `fake-agent` additions for MCP calls, per-role scripts, messages and shell steps.
- End-to-end tests and one smoke stage.

Out:

- The orchestrator role, its tools, events and `wait_for_events` (milestone 9). The `answer` tool: in this milestone a worker's `ask` shows as attention and the human answers by typing into the window.
- Routing advice and `list_models` (milestone 9). The engine only uses the roster for validation and reviewer choice.
- The plan view and finish view in the TUI (milestone 9). Everything is driven from the CLI.
- `C-b O` and the run form (milestone 9). Allowed runtimes per run (milestone 9).
- Rebasing. Deleting old run records from the state file (follow-up).
- The integrator role from spec section 2. No window has it in this milestone.

## Design decisions

### Structure

1. **Engine inside the daemon.** New module tree `crates/daemon/src/run/`. No new crate for the engine: it needs the window manager, the git runner and the state store, which all live in the daemon.
2. **Pure core, thin I/O shell.** These files are pure and do no I/O, no `tokio`, no `std::process`, no `std::fs`: `run/plan.rs` (parsing, validation, slug), `run/roster.rs` (tier lookup, reviewer choice), `run/model.rs` (persisted `Run` and `Task`), `run/engine/*.rs` (the state machine), `run/contract.rs` (role instructions and message texts), `run/messages.rs` (delivery gate and encoding), `run/report.rs` (Markdown rendering), `run/role_launch.rs` (launch argv for roles). These do I/O: `run/git.rs` (git operations), `run/verify.rs` (the verify command), `run/driver.rs` (`RunService`: executes actions, owns the socket-facing API, writes the report). The engine's inputs are events and its outputs are actions; `RunService` executes the actions and feeds results back as inputs. Its unit tests need no git and no PTY.
3. **Protocol version 5.** `proto::PROTO_VERSION` becomes one more than the value on `main` when this milestone starts: 5 after milestone 7's 4. If milestone 7 is not merged yet, take 4 and record it in "Implementation notes"; milestone 7 then takes the next number (product spec 10.1).
4. **MCP server crate.** New library crate `crates/mcp`, package `anthrex-mcp`, library name `mcp`. The `anthrex mcp` subcommand in `crates/cli` calls `mcp::serve_stdio`. This keeps `rmcp` out of the daemon and the TUI.
5. **`rmcp` pinned.** Workspace dependency `rmcp = { version = "=3.4.0", default-features = false, features = ["server", "transport-io"] }`. Verified on 2026-09-18 in a throwaway crate outside the repository with Rust 1.92.0: it builds, and a hand-written `ServerHandler` (no macros) answered `initialize` for protocol versions `2025-06-18` and `2025-03-26`, `tools/list`, and `tools/call` with `isError` true and false over stdio. The API used: `rmcp::ServerHandler` with `get_info(&self) -> ServerConfig`, `async fn list_tools(&self, Option<PaginatedRequestParams>, RequestContext<RoleServer>) -> Result<ListToolsResult, ErrorData>`, `async fn call_tool(&self, CallToolRequestParams, RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData>`; `CallToolResult::success(vec![ContentBlock::text(..)]).into()` and `CallToolResult::error(..)`; `Tool::new(name, description, Arc<JsonObject>)`; `ServerCapabilities::builder().enable_tools().build()`; `ServiceExt::serve(rmcp::transport::stdio()).await?.waiting().await`. `ServerConfig::default()` reports the server name `rmcp`; set `server_info.name = "anthrex"` and `server_info.version` to the crate version. Tools are built by hand from JSON Schemas, not with the `#[tool]` macros, because the tool list depends on the role.

### Runs and git

6. **Run id.** The run id is the slug. It is unique because of its random suffix. The CLI accepts the full id, the 4 hex digits at its end, or a unique prefix.
7. **Slug.** Lower-case the goal. Replace every character that is not ASCII alphanumeric with `-`. Collapse runs of `-` and trim `-` at both ends. Take the first 32 characters and trim a trailing `-` again. An empty result becomes `run`. Append `-` and 4 lowercase hex digits. The digits come from hashing the current `SystemTime` nanoseconds and the process id with `std::collections::hash_map::RandomState`; they are random, not stable, and nothing depends on reproducing them. If `refs/heads/anthrex/<slug>/integration` already exists, draw new digits, up to 5 times, then fail with `could not pick a free run id`.
8. **Where a run lives.** `root` is `git rev-parse --show-toplevel` of the `--dir` directory, canonicalized: the checkout the user started from, which may be a linked worktree. `project` is `worktree::repo_root` of the same directory (the main checkout, as milestone 4 groups projects). `<wt>` is `worktree::repo_worktrees_dir(<data_dir>/worktrees, project)`. The run directory is `<wt>/runs/<slug>/`. Paths, with `<task>` the task id and `<n>` the round:

   | Thing | Branch | Path |
   |-------|--------|------|
   | Base | branch checked out in `root`; its commit is `base_sha` | `root`, never modified until finish |
   | Run | `anthrex/<slug>/integration`, created at `base_sha` | `<wt>/runs/<slug>/integration` |
   | Task | `anthrex/<slug>/<task>`, created from the run branch head at first dispatch | `<wt>/runs/<slug>/<task>` |
   | Task review | detached at the task branch head | `<wt>/runs/<slug>/<task>-review-<n>` |
   | Final review | detached at the run branch head | `<wt>/runs/<slug>/final-review-<n>` |
   | Report | none | `<wt>/runs/<slug>/REPORT.md` |

9. **Start preflight.** In order, each failure an error for `run start`: `root` must resolve (`not a git repository: <dir>`); `HEAD` must be on a branch (`<root> is on a detached HEAD; check out a branch first`); `HEAD` must have a commit (`the repository has no commits yet`); `git status --porcelain --untracked-files=no` in `root` must print nothing (`the working tree at <root> has uncommitted changes; commit or stash them first`). Untracked files do not count as uncommitted changes. This is deliberately narrower than milestone 5's `is_dirty`.
10. **Git runner.** Every git command goes through milestone 5's `git::run` with its own deadline of `Instant::now() + RUN_GIT_TIMEOUT` (60 s) per command, on `spawn_blocking`. Stderr tails go into error messages. The run branch and task branches are created by the engine, not by `worktree::create`, because `anthrex/` is reserved there.
11. **Task worktree creation.** If the task branch does not exist: `git worktree add -b anthrex/<slug>/<task> <path> anthrex/<slug>/integration`, and the start commit is `git rev-parse anthrex/<slug>/integration` read just before. If the branch exists and `<path>` exists: reuse both. If the branch exists and `<path>` does not: `git worktree add <path> anthrex/<slug>/<task>`. A reused task keeps its first start commit.
12. **Merge.** In the run worktree, first `git rev-parse -q --verify MERGE_HEAD`; if it exists, `git merge --abort`. Then, if `git merge-base --is-ancestor <task-branch> HEAD` succeeds, the task is already merged and the merge commit is `HEAD`. Otherwise `git merge --no-ff --no-edit -m "anthrex: merge <task>: <title>" <task-branch>`. On failure, `git diff --name-only --diff-filter=U` lists conflicted files, then `git merge --abort`. A non-empty list is a conflict; an empty list is a plain failure with git's stderr. Merges into one run are serialized through a FIFO queue.
13. **Review worktree.** `head = git rev-parse <task-branch>` and `base = git merge-base <run-branch> <head>`, both read when the review starts. For the final review, `head = git rev-parse <run-branch>` and `base = base_sha`. Then `git worktree add --detach <path> <head>`. If `<path>` already exists, as after a resume, `git worktree remove --force <path>` first.
14. **Submission check.** On `report_done`, in the task worktree: `git rev-list --count <start>..HEAD` must be at least 1, and `git status --porcelain --untracked-files=no` must be empty. The answer is the tool call's result, not a typed message, because the worker is still in its turn and reads it at once. The task stays Running when the check fails.
15. **Verify.** `/bin/sh -c "{ <verify>\n} 2>&1"` in the directory, in its own process group, with stdin from `/dev/null`, `ANTHREX_WINDOW_ID` removed from the environment, and a timeout of 20 minutes (`VERIFY_TIMEOUT`). On timeout the whole group gets `SIGKILL`. A reader thread keeps the last 200 lines (`VERIFY_TAIL_LINES`) in a ring, each cut to 300 characters. Exit 0 is success.
16. **Finish.** Only from Ready, except `keep` and `discard`, which also work from Cancelled and Failed. Milestone 9 also accepts a run with an orchestrator that is in a hold (product spec 8.1).
    - `merge`: `root` must have the base branch checked out (`check out <base> in <root> first (currently <branch>)`) and pass the check from decision 9. Then `git merge --no-ff --no-edit -m "anthrex: merge run <slug>: <goal>" anthrex/<slug>/integration` in `root`. A conflict runs `git merge --abort` and fails with the file list; the run stays Ready. The spec's command lacks `--no-edit`; without it git opens an editor and the command times out.
    - `pr`: the remote is `git config --get branch.<base>.remote`, else `origin`. `git remote get-url <remote>` must succeed. `gh` must be an executable file in some `PATH` directory. Then `git push -u <remote> anthrex/<slug>/integration`, then `gh pr create --base <base> --head anthrex/<slug>/integration --title <goal> --body-file <REPORT.md>` in `root` with `GH_PROMPT_DISABLED=1` and a 60 s timeout. The last non-empty stdout line is the pull request URL.
    - `keep`: nothing in git.
    - `discard`: remove every run window, then `git worktree remove --force <path>` for every path in decision 8's table that exists, `git worktree prune`, then `git branch -D` for every branch from `git for-each-ref --format=%(refname:short) refs/heads/anthrex/<slug>/`, then delete `<wt>/runs/<slug>/`. Every step runs even when an earlier one failed; the errors are joined into one message.
    - `merge`, `pr` and `discard` need `confirm == Some(<run id>)` in the request. Without it the daemon replies `RunConfirmNeeded` with a prompt; for `pr` the prompt names the remote, its URL, the branch and the base branch.

### Tasks and the engine

17. **Plan format.** Spec 8.3's JSON, parsed with `serde_json` and `deny_unknown_fields`. `notes`, `verify`, `acceptance`, `depends_on`, `model` and `reviewer` are optional. `model: ""` is the same as `null`.
18. **Validation.** All errors are collected, each naming the task and field as `task <id>: <field>: <message>`, run-level ones as `<field>: <message>`, joined with newlines. Rules: `goal` not blank; `verify` not blank when present; at least one task; at most `max_tasks`; `id` matches `^[a-z0-9][a-z0-9-]{0,15}$`, is unique, is not `integration` or `final`, and does not end with `-review-<digits>` (these names collide with branches and paths in decision 8); `title` and `prompt` not blank; each acceptance item not blank; every `depends_on` entry names another task; no cycles (`depends_on: cycle t1 -> t2 -> t1`); `runtime` is `claude` or `codex`; the task's runtime and model and the explicit reviewer's runtime and model are in the roster (decision 20). `plan::validate` returns each error as a `PlanError { task: Option<String>, field: String, message: String }`, whose `Display` is the text above; milestone 9 hands the same fields to the orchestrator as JSON.
19. **Task states.** Spec 8.3. "Active" means a task in Running, Submitted, Verifying, Reviewing, ChangesRequested, Approved, Merging or Conflict. The engine dispatches Planned tasks whose dependencies are all Merged, in plan order, while fewer than `max_parallel` tasks are active. Ready is the state between "dependencies merged" and "dispatched".
20. **Roster membership.** `model: null` is valid when the runtime has any roster entry. A named model is valid when an entry has the same runtime and model. The tier of `(runtime, null)` is the tier of that runtime's entry whose model is empty, else `standard`.
21. **Reviewer choice** when a task has no `reviewer`. Let `t` be the author's tier. Take the first match of: (a) entries on the other runtime with tier at least `t`, lowest tier first, then roster order; (b) entries on the same runtime with a different model and tier at least `t`, same order; (c) entries on the other runtime, highest tier first; (d) the author itself. An entry with an empty model becomes `model: None`. The final reviewer is the first `frontier` entry in roster order, else the first entry of the highest tier present. The default roster's `claude-opus-5` author gets the Codex entry through rule (c).
22. **Rounds.** `round` counts reviews started, and names review worktrees. `rounds_used` counts "changes" decisions, from a review verdict or from a verify failure. When a decision would make `rounds_used` reach `max_review_rounds`, the task goes to NeedsHuman with `review rounds exhausted (<n>)` instead of ChangesRequested. With the default 3, the worker receives two findings messages, and the third failed round stops. `run retry` resets `rounds_used` to 0; `round` keeps counting.
23. **Findings rule.** A `changes` verdict without a critical or important finding counts as `approve`, and its minor findings stay in the report.
24. **Messages set states.** ChangesRequested and Conflict hold while their message waits in the outbox. The engine moves the task to Running when that message is delivered. If the worker window has exited when the message would be queued, the task goes to NeedsHuman with `worker window is gone`.
25. **Tool acceptance.** A call is accepted only when: the run exists and is not Paused or terminal; the tool belongs to the role (spec 8.5 table); for a worker, `task_id` is set and the task's `worker_window` equals the caller's window; for a reviewer with a task, the task's `reviewer_window` equals the caller's window; for a reviewer without a task, the run's final reviewer window equals the caller's window. `report_done` is accepted in Running and Blocked. `report_blocked` and `ask` in Running and Blocked. `submit_review` only in Reviewing (or FinalReview) and once per round. `get_task` and `get_review` in any state. Rejections are tool errors with these texts: `unknown run <id>`, `run <id> is paused; the user must resume it`, `run <id> is <state>`, `tool <tool> is not available to the <role> role`, `this window is not the worker of task <id>`, `this window is not the reviewer of task <id> round <n>`, `report_done is accepted only while the task is running or blocked (it is <state>)`, `a review for round <n> was already submitted`.
26. **Window exits.** A worker window exiting while its task is Running, Blocked, ChangesRequested or Conflict sends the task to NeedsHuman with `worker window exited (<reason>)`. A reviewer exiting before `submit_review` sends the task to NeedsHuman with `reviewer window exited before submit_review`. A final reviewer exiting before submitting adds run attention. Exits of windows the engine retired or killed are ignored.
27. **Retiring windows.** After `submit_review`, the reviewer window is retired: `RunService` kills it once its status is Idle, Done or Exited, or 30 s after the action (`RETIRE_AFTER`). After a task is Merged, its worker window is retired the same way. Windows stay listed as Exited until the user removes them or discards the run.
28. **Window limit.** Each worker or reviewer window created counts toward `max_windows`; restarts do not. A task that would exceed it goes to NeedsHuman with `run window limit (<n>) reached`.
29. **Window names and size.** `h4` is the slug's 4 hex digits. Worker: `<h4>/<task>`, and `<h4>/<task>-<k>` for the k-th worker window of that task, k from 2. Reviewer: `<h4>/<task>-r<n>`. Final reviewer: `<h4>/final-r<n>`. Run windows are created at 120 by 40 cells. A name clash fails the create, and the task goes to Failed with the manager's message.
30. **Run-level exceptions.** This is the no-orchestrator column of product spec 8.1's holds table; milestone 9 adds the other column for runs with an orchestrator, and keeps every rule here for runs without one. A failed integration verify keeps the run in Integrating with attention `run verify failed; fix it in <run worktree> and run: anthrex run retry <id> integration, or accept it with: anthrex run approve <id>`. `retry <id> integration` runs Integrating again: verify, then a new final review. `approve` in Integrating after a failed verify, or in FinalReview after the final reviewer exited, moves on to FinalReview or Ready. A final review with `changes` still moves to Ready, with attention `final review requested changes; see the report`. A run whose tasks are all Cancelled skips verify and final review and goes to Ready.
31. **Control operations.** `approve`: AwaitingApproval to Running, and the cases in decision 30. `retry <task>`: NeedsHuman, Blocked or Failed to Planned; Cancelled dependencies are removed from the task's `depends_on`, and the report logs it; if the worker window is still live the task is re-dispatched without a new window and receives the retry message; otherwise the worktree is reused (decision 11) and a new worker window starts. `skip <task>`: any state except Merged, Merging and Cancelled; kills its windows; the task becomes Cancelled; every direct and transitive dependant that is not Merged or Cancelled becomes NeedsHuman with `dependency <id> was skipped`. `cancel`: kills every live run window; every task that is not Merged becomes Cancelled; the run becomes Cancelled. Paused runs accept only `resume` and `cancel`: `run <id> is paused; resume it first`.
32. **Attention.** `RunInfo.attention` lists, in this order: run-level reasons (decision 30), then per task `<id> blocked: <reason>`, `<id> needs a human: <why>`, `<id> asks: <question>`, then `<id>: message waiting for more than 10 minutes`. A question clears when the same worker makes its next tool call or the task changes state.
33. **Start.** `--yes` or `orchestrator.auto_approve = true` start the run in Running; otherwise AwaitingApproval. Planning exists in the enum for milestone 9 and is never entered here. The run branch and worktree are created on entering Running, so a run cancelled before approval leaves nothing in git.
34. **Limits are frozen.** The run records `max_parallel`, `max_tasks`, `max_review_rounds`, `max_windows`, `worker_permission_mode` and `worker_codex_sandbox` from the config at start. Later config changes do not affect it.

### Roles, launch and messages

35. **MCP server arguments.** `<exe> mcp --role <worker|reviewer> --run <run-id> [--task <task-id>] --window <window-id> --socket <socket-path>`, as product spec 8.5 and 11.5 show. The server needs the caller's window for decision 25 and must not depend on which environment variables Claude or Codex pass to MCP servers. `--socket` overrides `ANTHREX_SOCKET`. Milestone 9 launches the orchestrator with the same arguments and role `orchestrator`.
36. **Claude role flags** (verified against `claude --help` of Claude Code 2.1.276 on 2026-09-18: `--mcp-config <configs...>`, `--allowedTools <tools...>`, `--append-system-prompt <prompt>`, `--permission-mode <mode>` exist). Inserted after milestone 3's `--settings` and before `--model`, `--resume` and `--`: `--mcp-config <json>` with `{"mcpServers":{"anthrex":{"type":"stdio","command":"<exe>","args":[...]}}}`, then `--allowedTools mcp__anthrex__*`, then `--append-system-prompt <contract>`, then for reviewers `--permission-mode plan`, for workers `--permission-mode <mode>` when the run's `worker_permission_mode` is not `default`. `--mcp-config` and `--allowedTools` are variadic, so each must be followed by another flag, as this order guarantees. The argv is passed to `exec` directly; the quotes in the spec are shell notation only.
37. **Codex role flags** (spec 11.5). After milestone 3's `-c` overrides and before `-m`, `resume` and `--`: `-c mcp_servers.anthrex.command=<toml exe>`, `-c mcp_servers.anthrex.args=<toml array>`, `-c mcp_servers.anthrex.tool_timeout_sec=120`, `-c mcp_servers.anthrex.default_tools_approval_mode="auto"`, `-c developer_instructions=<toml contract>`, then for reviewers `-s read-only -a on-request`, for workers `-s <sandbox>` only when `worker_codex_sandbox` is set. `<toml ...>` is a TOML basic string built by milestone 3's `launch::codex::toml_string` (decision 38). `codex --help` of codex-cli 0.155.0 says the value of `-c` is parsed as TOML and falls back to a literal string when that fails.
38. **TOML basic strings.** Every TOML string comes from milestone 3's `launch::codex::toml_string(s)`, which returns `"` + escaped `s` + `"`: backslash as `\\`, `"` as `\"`, newline `\n`, carriage return `\r`, tab `\t`, backspace `\b`, form feed `\f`, every other character below U+0020 and U+007F as `\uXXXX` with uppercase hex (milestone 3 decision 19). Everything else is copied. Do not add a second helper. A TOML array is `[` + the strings joined by `,` + `]`.
39. **Role instructions** are the constants `WORKER_CONTRACT` and `REVIEWER_CONTRACT` in `run/contract.rs`, with the exact text in the Interfaces section. They go to `--append-system-prompt` and `developer_instructions`, and they end the first prompt too.
40. **First prompts.** Worker, from `contract::worker_prompt`: `[anthrex] Task <id>: <title>`, then `Run goal: <goal>`, `Worktree: <path>`, `Branch: <branch>`, `Start commit: <sha>`, a blank line, the task prompt, a blank line, `Acceptance criteria:` with one `- ` line each (omitted when empty), `Run notes:` with the notes (omitted when absent), a blank line, the contract. A worker started by `retry` after an earlier attempt gets the extra line `An earlier attempt left commits on this branch. Read them first.` after `Start commit`. Reviewer, from `contract::reviewer_prompt`: `[anthrex] Review task <id> "<title>", round <n>. Call get_review, then review the change and call submit_review.` or, for the final review, `[anthrex] Final review of run <id>: <goal>. Call get_review, then review the whole run and call submit_review.`, then a blank line and the contract.
41. **Delivery.** Each run keeps an outbox of pending messages, persisted with the run. A message goes to its window only when the window's status is Idle or Done, and, if the engine already delivered to that window, only after the window has been seen in Starting, Working or Attention since then, or 60 s after the last delivery (`REDELIVER_AFTER`). This stops two messages landing in one prompt. Encoding: `ESC [ 200 ~`, then the text with `\r\n` and `\n` turned into `\r` and every `ESC [ 200 ~` and `ESC [ 201 ~` removed, then `ESC [ 201 ~`; 200 ms later (`SUBMIT_DELAY`, a `tokio::time::sleep`, never a thread sleep), a lone `\r`. Both writes go through `WindowManager::write_input`, which only enqueues on the per-window writer thread. Texts are cut to 32 KiB (`MESSAGE_MAX_BYTES`) by keeping the first 16 KiB and the last 15 KiB around the line `[... truncated ...]`. Every message starts with `[anthrex]`.
42. **The 10-minute rule.** A message that has waited 600 s (`MESSAGE_ATTENTION_AFTER`) marks its window as attention once, through the new `WindowManager::mark_attention(id)`, and adds the attention line from decision 32.
43. **Role launch is persisted.** A run window's `Entry` keeps its `RoleLaunch` (Interfaces). `WindowRecord.run` stores it, so milestone 6's restart relaunches the window with the same role flags. `Entry::info()` sets `WindowInfo.run` from it.
44. **Tool results.** `get_task` and `get_review` return pretty-printed JSON as text. The others return one sentence: `report_done`: `Accepted. Your work goes to verification and review. End your turn now; further instructions arrive in this terminal as [anthrex] messages.`; `report_blocked`: `Recorded. A human will look at this task. End your turn now.`; `ask`: `Recorded. A human will answer in this terminal. End your turn now and wait.`; `submit_review`: `Review recorded. You are done; end your turn now.`. Failures are tool results with `isError: true` and the error text, never JSON-RPC errors.
45. **MCP forwarding.** For each `tools/call`, the MCP server opens a fresh connection to the socket with a 2 s connect timeout, sends `Hello { client: ClientKind::Mcp }`, then `ToolCall`, waits up to 100 s (`TOOL_REPLY_TIMEOUT`) for `ToolResult`, skipping broadcasts, and closes. A fresh connection per call survives daemon restarts. Codex's `tool_timeout_sec` is 120 s, above both.

### Persistence and the tree

46. **State file.** `StateFile.runs` becomes `Vec<Run>`, the full engine model including the outbox and the event log. Each entry is deserialized on its own; a bad entry is skipped and logged at `warn`, as milestone 6 does for windows. The version stays 2: spec 6 item 5 puts `runs` in version 2.
47. **Paused.** On load, every run that is not Finished, Cancelled or Failed becomes Paused with `paused_from` set to its state (a run that was already Paused keeps its `paused_from`). In-flight operations are forgotten. Nothing is dispatched until `run resume`.
48. **Shutdown order.** `lifecycle::run` calls `RunService::stop()` before `manager.shutdown()`. After `stop`, the service ignores every input, so the killing of windows at shutdown cannot move tasks to NeedsHuman. The final save takes the runs as they were at `stop`.
49. **Resume.** The run returns to `paused_from`. For each task not Merged, Cancelled or Failed: restart its worker window if it has one, and its current-round reviewer window if the review was not submitted, through `WindowManager::restart` (only windows whose status is Exited). Queue `RESUME_WORKER` for each restarted worker and `RESUME_REVIEWER` for each restarted reviewer. Submitted becomes Running with the resume message. Verifying runs verify again. Approved and Merging go back into the merge queue; decision 12's ancestor check makes a repeated merge safe. Integrating runs verify again unless its attention was already set. FinalReview restarts the final reviewer when it had not submitted. A failed restart sends the task to NeedsHuman with `restart failed: <message>`.
50. **Run rows.** Spec 5.1 item 4 and 8.8. Under its project, above plain windows: the run row `◈ <goal> · <merged>/<total>`, indent 2. Then one task row per task in plan order, indent 4: state glyph, id, title, and the state label right-aligned and dimmed. Then the task's worker window and its current reviewer window as ordinary window rows, indent 6. The final reviewer window hangs under the run row at indent 4, after the tasks. Windows whose `WindowInfo.run` names a run shown here are not listed again as plain windows; windows of an unknown run are plain windows. A Finished or Cancelled run is shown only while at least one of its windows exists, and starts collapsed.
51. **Glyphs and status.** Task glyphs: Planned and Ready `○`; Running, Submitted, Verifying, Reviewing, ChangesRequested, Approved and Merging the working spinner; Conflict, Blocked and NeedsHuman `◆`; Merged `✓`; Cancelled and Failed `✕`. A run's status for urgency and the colour of `◈`: attention when `attention` is not empty or a task is Blocked or NeedsHuman; else working when a task is active or the run is Integrating or FinalReview; else done when Ready or Finished; else idle. The project row's status includes its runs.

## Interfaces

### `proto` (`crates/proto/src/run.rs`, new; re-exported from `lib.rs`)

```rust
pub const PROTO_VERSION: u32 = 5; // lib.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role { Orchestrator, Worker, Reviewer } // Orchestrator is used from milestone 9

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRef { pub run_id: String, pub task_id: Option<String>, pub role: Role }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind { Implement, Test, Refactor, Docs, Investigate, Fix }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRef { pub runtime: Runtime, #[serde(default)] pub model: Option<String> }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTask {
    pub id: String,
    pub title: String,
    pub kind: TaskKind,
    pub prompt: String,
    #[serde(default)] pub acceptance: Vec<String>,
    #[serde(default)] pub depends_on: Vec<String>,
    pub runtime: Runtime,
    #[serde(default)] pub model: Option<String>,
    #[serde(default)] pub reviewer: Option<ModelRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub goal: String,
    #[serde(default)] pub notes: Option<String>,
    #[serde(default)] pub verify: Option<String>,
    pub tasks: Vec<PlanTask>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState { Planning, AwaitingApproval, Running, Integrating, FinalReview, Ready, Finished, Cancelled, Failed, Paused }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState { Planned, Ready, Running, Submitted, Verifying, Reviewing, ChangesRequested, Approved, Merging, Merged, Conflict, Blocked, NeedsHuman, Cancelled, Failed }

impl RunState { pub fn label(self) -> &'static str; pub fn is_terminal(self) -> bool; } // "awaiting approval", "final review", ...
impl TaskState { pub fn label(self) -> &'static str; pub fn is_active(self) -> bool; }  // "changes requested", "needs human", ...

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity { Critical, Important, Minor }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding { pub severity: Severity, pub file: String, #[serde(default)] pub line: Option<u32>, pub summary: String }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict { Approve, Changes }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FinishAction { Merge, Pr, Keep, Discard }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub title: String,
    pub kind: TaskKind,
    pub state: TaskState,
    pub runtime: Runtime,
    pub model: Option<String>,
    pub reviewer: ModelRef,
    pub depends_on: Vec<String>,
    pub round: u32,
    pub rounds_used: u32,
    pub worker_window: Option<u32>,
    pub reviewer_window: Option<u32>,
    pub branch: String,
    pub merge_commit: Option<String>,
    pub note: Option<String>, // blocked reason, needs-human reason, pending question, or failure
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInfo {
    pub run_id: String,
    pub goal: String,
    pub project: PathBuf,
    pub root: PathBuf,
    pub state: RunState,
    pub paused_from: Option<RunState>,
    pub base_branch: String,
    pub base_sha: String,
    pub run_branch: String,
    pub report_path: PathBuf,
    pub verify: Option<String>,
    pub tasks: Vec<TaskInfo>,
    pub final_reviewer_window: Option<u32>,
    pub attention: Vec<String>,
    pub outcome: Option<String>, // "merged <sha>", "pull request <url>", "kept", "discarded"
    pub created_at: u64,         // Unix seconds
}
```

`WindowInfo` gains `pub run: Option<RunRef>` (serde default). `ClientKind` gains `Mcp`.

New `ClientMsg` variants:

| Variant | Fields | Reply |
|---------|--------|-------|
| `RunStart` | `plan_json: String`, `dir: PathBuf`, `yes: bool` | `RunCreated { run_id }` or `Error { request: "run start" }` |
| `RunApprove` | `run_id: String` | `Ack { request: "run approve" }` or `Error` |
| `RunRetry` | `run_id: String`, `task_id: String` (a task id or `integration`) | `Ack { request: "run retry" }` or `Error` |
| `RunSkip` | `run_id: String`, `task_id: String` | `Ack { request: "run skip" }` or `Error` |
| `RunFinish` | `run_id: String`, `action: FinishAction`, `confirm: Option<String>` | `RunConfirmNeeded { run_id, prompt }`, `RunFinished { run_id, message }` or `Error { request: "run finish" }` |
| `RunCancel` | `run_id: String` | `Ack { request: "run cancel" }` or `Error` |
| `RunResume` | `run_id: String` | `Ack { request: "run resume" }` or `Error` |
| `ToolCall` | `run_id: String`, `task_id: Option<String>`, `role: Role`, `window_id: u32`, `tool: String`, `args: serde_json::Value` | `ToolResult { ok, text }` |

New `DaemonMsg` variants: `RunsChanged { runs: Vec<RunInfo> }` (broadcast, like `WindowsChanged`), `RunCreated { run_id: String }`, `RunConfirmNeeded { run_id: String, prompt: String }`, `RunFinished { run_id: String, message: String }`, `ToolResult { ok: bool, text: String }`. `Welcome` gains `runs: Vec<RunInfo>`. `proto::messages::request` gains `RUN_START = "run start"`, `RUN_APPROVE`, `RUN_RETRY`, `RUN_SKIP`, `RUN_FINISH`, `RUN_CANCEL`, `RUN_RESUME` (the same words with a space) and `TOOL = "tool"`.

### `config` (milestone 6's crate, changed)

`Tier` and `ModelEntry` live in `crates/proto/src/run.rs`, because milestone 9 sends the roster to the client; the config crate re-exports them (`pub use proto::{Tier, ModelEntry};`).

```rust
// proto/src/run.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier { Fast, Standard, Frontier } // ordered: Fast < Standard < Frontier

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry { pub runtime: Runtime, pub model: String, pub tier: Tier, pub strengths: Vec<String> }

// config
pub struct Config { /* existing fields */ pub orchestrator: Orchestrator }

#[derive(Debug, Clone, PartialEq)]
pub struct Orchestrator {
    pub max_parallel: u32,                   // default 3, valid 1..=8
    pub max_tasks: u32,                      // default 12, valid 1..=50
    pub max_review_rounds: u32,              // default 3, valid 1..=10
    pub max_windows: u32,                    // default 20, valid 1..=100
    pub auto_approve: bool,                  // default false
    pub worker_permission_mode: String,      // default "default", must not be empty
    pub worker_codex_sandbox: Option<String>, // default None; "read-only" | "workspace-write" | "danger-full-access"
    pub builtin_models: bool,                // default true
    pub models: Vec<ModelEntry>,             // the merged roster; default: default_roster()
}

pub fn default_roster() -> Vec<ModelEntry>; // exactly the four entries of spec 9.5, in that order
```

Parsing rules: each key is validated on its own; an invalid value produces a `Problem` and the default. `[[orchestrator.models]]` entries need `runtime` (`claude` or `codex`), `model` (a string; empty only for `codex`, meaning "omit `-m`"), `tier` (`fast`, `standard`, `frontier`); `strengths` defaults to empty and holds at most 8 strings of at most 40 characters. The roster is built as product spec 9.5 says: start from `default_roster()`, or from nothing when `builtin_models = false`; a user entry with the same `(runtime, model)` as an entry already in the roster replaces it in place; any other entry is appended in file order. A bad entry is skipped with a problem whose key names its index, for example `orchestrator.models[2]: empty model is only allowed for codex`. A repeated `(runtime, model)` among the user's own entries keeps the first with the problem `duplicate model, ignored`. If the result is empty, the default roster is used with the problem `no valid models; using the built-in roster`. Unknown keys inside `[orchestrator]` produce `unknown key, ignored`. The daemon logs every problem at `warn` once, as milestone 6 does.

### `daemon`

```rust
// run/model.rs (new). Everything derives Debug, Clone, PartialEq, Serialize, Deserialize.
pub struct RunLimits { pub max_parallel: u32, pub max_tasks: u32, pub max_review_rounds: u32, pub max_windows: u32,
                       pub worker_permission_mode: String, pub worker_codex_sandbox: Option<String> }
pub struct VerifyRecord { pub at: u64, pub ok: bool, pub code: Option<i32>, pub timed_out: bool, pub tail: String, pub secs: u64 }
pub struct ReviewRecord { pub round: u32, pub reviewer: ModelRef, pub window_id: Option<u32>, pub base: String, pub head: String,
                          pub verdict: Option<Verdict>, pub effective: Option<Verdict>, pub summary: String, pub findings: Vec<Finding> }
pub struct PendingMessage { pub window_id: u32, pub text: String, pub queued_at: u64, pub flagged: bool, pub task_id: Option<String> }
pub struct LogEntry { pub at: u64, pub text: String } // at most 500 kept per run, oldest dropped
pub struct Task {
    pub spec: PlanTask,
    pub state: TaskState,
    pub reviewer: ModelRef,          // explicit or chosen (decision 21)
    pub reviewer_chosen: bool,
    pub branch: String,
    pub worktree: PathBuf,
    pub start_commit: Option<String>,
    pub worker_window: Option<u32>,
    pub worker_windows_made: u32,
    pub reviewer_window: Option<u32>,
    pub round: u32,
    pub rounds_used: u32,
    pub reviews: Vec<ReviewRecord>,
    pub verifies: Vec<VerifyRecord>,
    pub done_summary: Option<String>,
    pub merge_commit: Option<String>,
    pub note: Option<String>,
    pub question: Option<String>,
}
pub struct FinalReview { pub round: u32, pub window_id: Option<u32>, pub record: Option<ReviewRecord>, pub exited: bool }
pub struct Run {
    pub id: String, pub goal: String, pub notes: Option<String>, pub verify: Option<String>,
    pub root: PathBuf, pub project: PathBuf, pub wt_dir: PathBuf,
    pub base_branch: String, pub base_sha: String,
    pub state: RunState, pub paused_from: Option<RunState>,
    pub limits: RunLimits,
    pub tasks: Vec<Task>,
    pub windows_created: u32,
    pub run_verifies: Vec<VerifyRecord>,
    pub final_review: Option<FinalReview>,
    pub run_attention: Vec<String>,
    pub merge_queue: Vec<String>,     // task ids, FIFO
    pub outbox: Vec<PendingMessage>,
    pub outcome: Option<String>,
    pub log: Vec<LogEntry>,
    pub created_at: u64,
}
impl Run {
    pub fn run_branch(&self) -> String;          // anthrex/<id>/integration
    pub fn run_dir(&self) -> PathBuf;            // <wt_dir>/runs/<id>
    pub fn run_worktree(&self) -> PathBuf;       // <run_dir>/integration
    pub fn report_path(&self) -> PathBuf;        // <run_dir>/REPORT.md
    pub fn short(&self) -> &str;                 // the 4 hex digits
    pub fn info(&self) -> RunInfo;
}

// run/plan.rs (new)
pub fn parse(plan_json: &str) -> Result<Plan, String>;
pub struct PlanError { pub task: Option<String>, pub field: String, pub message: String } // Display: decision 18's text
pub fn validate(plan: &Plan, max_tasks: u32, roster: &[ModelEntry]) -> Result<(), Vec<PlanError>>;
pub fn slug(goal: &str, suffix: u16) -> String;
pub fn random_suffix() -> u16;
pub struct Preflight { pub root: PathBuf, pub project: PathBuf, pub base_branch: String, pub base_sha: String }
pub fn build_run(plan: Plan, pre: Preflight, id: String, wt_dir: PathBuf, cfg: &config::Orchestrator, now: u64) -> Run;

// run/roster.rs (new)
pub fn tier_of(roster: &[ModelEntry], m: &ModelRef) -> Option<Tier>; // None: not in the roster
pub fn pick_reviewer(roster: &[ModelEntry], author: &ModelRef) -> ModelRef;
pub fn pick_final_reviewer(roster: &[ModelEntry]) -> ModelRef;

// run/engine/mod.rs (new)
pub type OpId = u64;
pub type ReplyId = u64;
pub struct ToolCall { pub run_id: String, pub task_id: Option<String>, pub role: Role, pub window_id: u32, pub tool: String, pub args: serde_json::Value }
pub enum Input {
    Start { reply: ReplyId, run: Run, auto_approve: bool },
    Approve { reply: ReplyId, run_id: String },
    Retry { reply: ReplyId, run_id: String, task_id: String },
    Skip { reply: ReplyId, run_id: String, task_id: String },
    Cancel { reply: ReplyId, run_id: String },
    Resume { reply: ReplyId, run_id: String },
    Finish { reply: ReplyId, run_id: String, action: FinishAction },
    Tool { reply: ReplyId, call: ToolCall },
    OpDone { op: OpId, result: OpResult },
    WindowExited { window_id: u32, reason: String },
}
pub enum OpKind {
    CreateRunWorktree { root: PathBuf, branch: String, path: PathBuf, base_sha: String },
    CreateTaskWorktree { root: PathBuf, run_branch: String, branch: String, path: PathBuf },
    CreateWindow { spec: WindowSpec, role: RoleLaunch },
    RestartWindow { window_id: u32 },
    CheckSubmission { worktree: PathBuf, start_commit: String },
    Verify { dir: PathBuf, command: String },
    PrepareReview { root: PathBuf, base_ref: String, head_ref: String, path: PathBuf },
    Merge { run_worktree: PathBuf, branch: String, message: String },
    FinishMerge { root: PathBuf, base_branch: String, run_branch: String, message: String },
    FinishPr { root: PathBuf, base_branch: String, run_branch: String, title: String, body_file: PathBuf },
    Discard { root: PathBuf, worktrees: Vec<PathBuf>, branch_prefix: String, run_dir: PathBuf },
}
pub enum OpResult {
    Worktree { start: Option<String> },           // None when an existing worktree was reused
    Window { window_id: u32 },
    Submission { commits: u32, dirty: bool },
    Verify { ok: bool, code: Option<i32>, timed_out: bool, tail: String, secs: u64 },
    Review { base: String, head: String },
    Merged { commit: String },
    Conflict { files: Vec<String> },
    Finished { outcome: String },
    Failed { message: String },
}
pub enum Action {
    Reply { reply: ReplyId, result: Result<String, String> },
    Op { op: OpId, run_id: String, kind: OpKind },
    KillWindow { window_id: u32 },
    RetireWindow { window_id: u32 },
    RemoveWindow { window_id: u32 },
    WriteReport { run_id: String },
}
pub struct Delivery { pub window_id: u32, pub text: String }
pub struct Engine { /* runs: BTreeMap<String, Run>, pending ops, retired windows, exited windows, delivery gate */ }
impl Engine {
    pub fn new() -> Self;
    pub fn restore(&mut self, runs: Vec<Run>);                 // decision 47
    pub fn handle(&mut self, now: u64, input: Input) -> Vec<Action>;
    /// Decisions 41, 42 and 24. `status` gives each window's current status, None if unknown.
    pub fn deliverable(&mut self, now: u64, status: &dyn Fn(u32) -> Option<Status>) -> (Vec<Delivery>, Vec<u32>, Vec<Action>);
    pub fn runs(&self) -> impl Iterator<Item = &Run>;
    pub fn run(&self, id: &str) -> Option<&Run>;
    pub fn generation(&self) -> u64;                           // bumped by every mutation
}

// run/contract.rs (new)
pub const WORKER_CONTRACT: &str;   // text below
pub const REVIEWER_CONTRACT: &str; // text below
pub fn worker_prompt(run: &Run, task: &Task, earlier_attempt: bool) -> String;
pub fn reviewer_prompt(run: &Run, task: Option<&Task>, round: u32) -> String;
pub fn findings_message(round: u32, review: &ReviewRecord) -> String;
pub fn verify_failed_message(command: &str, v: &VerifyRecord) -> String;
pub fn conflict_message(run_branch: &str, files: &[String]) -> String;
pub fn retry_message(task_id: &str) -> String;
pub const RESUME_WORKER: &str;
pub const RESUME_REVIEWER: &str;
pub fn no_commits_error(start: &str) -> String;
pub const DIRTY_ERROR: &str;

// run/role_launch.rs (new). Also serialized into WindowRecord.run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleLaunch {
    pub run_ref: RunRef,
    pub instructions: String,
    pub claude_permission_mode: Option<String>, // "plan" for reviewers; the worker mode unless "default"
    pub codex_sandbox: Option<String>,          // "read-only" for reviewers; the worker sandbox if set
    pub codex_approval: Option<String>,         // "on-request" for reviewers
}
pub fn mcp_args(role: &RoleLaunch, window_id: u32, socket: &Path) -> Vec<String>;
pub fn claude_role_args(role: &RoleLaunch, exe: &Path, window_id: u32, socket: &Path) -> Vec<String>;
pub fn codex_role_args(role: &RoleLaunch, exe: &Path, window_id: u32, socket: &Path) -> Vec<String>;

// run/messages.rs (new)
pub const REDELIVER_AFTER: u64 = 60;           // seconds
pub const MESSAGE_ATTENTION_AFTER: u64 = 600;  // seconds
pub const MESSAGE_MAX_BYTES: usize = 32 * 1024;
pub const SUBMIT_DELAY: Duration = Duration::from_millis(200);
pub fn encode_paste(text: &str) -> Vec<u8>;    // markers included, CR not included
pub fn clamp(text: &str) -> String;
pub struct Gate { /* per window: last delivery time and whether it was busy since */ }
impl Gate { pub fn observe(&mut self, window_id: u32, status: Status); pub fn may_deliver(&self, window_id: u32, status: Status, now: u64) -> bool; pub fn delivered(&mut self, window_id: u32, now: u64); }

// run/git.rs (new). Blocking; call only from spawn_blocking.
pub const RUN_GIT_TIMEOUT: Duration = Duration::from_secs(60);
pub fn preflight(dir: &Path) -> Result<Preflight, String>;
pub fn branch_exists(root: &Path, branch: &str) -> Result<bool, String>;
pub fn create_run_worktree(root: &Path, branch: &str, path: &Path, base_sha: &str) -> Result<(), String>;
pub fn create_task_worktree(root: &Path, run_branch: &str, branch: &str, path: &Path) -> Result<Option<String>, String>;
pub fn check_submission(worktree: &Path, start: &str) -> Result<(u32, bool), String>;
pub fn prepare_review(root: &Path, base_ref: &str, head_ref: &str, path: &Path) -> Result<(String, String), String>;
pub enum MergeOutcome { Merged(String), Conflict(Vec<String>) }
pub fn merge(run_worktree: &Path, branch: &str, message: &str) -> Result<MergeOutcome, String>;
pub fn finish_merge(root: &Path, base_branch: &str, run_branch: &str, message: &str) -> Result<String, String>;
pub struct PrTarget { pub remote: String, pub url: String }
pub fn pr_target(root: &Path, base_branch: &str) -> Result<PrTarget, String>;
pub fn finish_pr(root: &Path, base_branch: &str, run_branch: &str, title: &str, body_file: &Path) -> Result<String, String>;
pub fn discard(root: &Path, worktrees: &[PathBuf], branch_prefix: &str, run_dir: &Path) -> Result<(), String>;

// run/verify.rs (new). Blocking.
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(20 * 60);
pub const VERIFY_TAIL_LINES: usize = 200;
pub fn run_verify(dir: &Path, command: &str, timeout: Duration) -> VerifyOutcome;
pub struct VerifyOutcome { pub ok: bool, pub code: Option<i32>, pub timed_out: bool, pub tail: String, pub secs: u64 }

// run/report.rs (new)
pub fn render(run: &Run) -> String;
pub fn format_utc(unix: u64) -> String; // "2026-09-11 10:44:16Z"

// run/driver.rs (new)
pub const RETIRE_AFTER: Duration = Duration::from_secs(30);
pub struct RunContext { pub worktrees_root: PathBuf, pub exe: PathBuf, pub socket_path: PathBuf, pub orchestrator: config::Orchestrator } // exe and worktrees_root from ManagerConfig
pub enum FinishReply { Confirm(String), Done(String) }
pub struct RunService { /* engine: Mutex<Engine>, manager, input channel, reply map, watches */ }
impl RunService {
    pub fn new(manager: Arc<WindowManager>, ctx: RunContext) -> Arc<Self>;
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> tokio::task::JoinHandle<()>;
    pub fn restore(&self, runs: Vec<Run>);
    pub fn records(&self) -> Vec<Run>;
    pub fn infos(&self) -> Vec<RunInfo>;
    pub fn watch(&self) -> tokio::sync::watch::Receiver<Vec<RunInfo>>;
    pub fn stop(&self);
    pub async fn start(&self, plan_json: String, dir: PathBuf, yes: bool) -> Result<String, String>;
    pub async fn approve(&self, run_id: String) -> Result<(), String>;
    pub async fn retry(&self, run_id: String, task_id: String) -> Result<(), String>;
    pub async fn skip(&self, run_id: String, task_id: String) -> Result<(), String>;
    pub async fn cancel(&self, run_id: String) -> Result<(), String>;
    pub async fn resume(&self, run_id: String) -> Result<(), String>;
    pub async fn finish(&self, run_id: String, action: FinishAction, confirm: Option<String>) -> Result<FinishReply, String>;
    pub async fn tool_call(&self, call: ToolCall) -> (bool, String);
}

// manager.rs (changed)
impl WindowManager {
    /// `project` is the run's `project`; `RunService` passes it, so no detection runs.
    pub async fn create_with_role(&self, spec: WindowSpec, project: PathBuf, role: RoleLaunch, cols: u16, rows: u16) -> anyhow::Result<WindowInfo>;
    pub fn mark_attention(&self, id: u32); // applies StatusEvent::EngineAttention
}
// status.rs (changed): StatusEvent::EngineAttention. Exited stays Exited; every other status becomes Attention.
// launch/mod.rs (changed): LaunchContext gains `pub role: Option<&'a RoleLaunch>`; launch/claude.rs and launch/codex.rs append the role args.
// state.rs (changed): StateFile.runs: Vec<Run>; WindowRecord.run: Option<RoleLaunch>.
// state.rs (changed): spawn_persister also takes `runs: Arc<RunService>` and saves on either watch.
// server.rs (changed): serve and handle_client also take `runs: Arc<RunService>`.
```

The contracts, exact text:

```text
WORKER_CONTRACT:
You are a worker in an anthrex orchestration run.
1. Work only inside your task worktree. Do not change files anywhere else.
2. Commit your changes on the task branch with clear messages. Never push, never switch branches, never rewrite commits that are already there.
3. Run the relevant tests before you finish.
4. When the task is complete and committed, call the anthrex tool report_done with a short summary.
5. If you cannot continue, call report_blocked with the reason. If you need a decision, call ask with your question, then end your turn and wait.
6. Messages that start with [anthrex] come from the orchestration engine. Follow them.

REVIEWER_CONTRACT:
You are a reviewer in an anthrex orchestration run.
1. Call the anthrex tool get_review first. It gives the task, its acceptance criteria, and the base and head commits.
2. Read the change with git diff <base>..<head> in this directory and check it against every acceptance criterion.
3. Run the tests if that helps you judge.
4. Do not edit, create or delete files, and do not commit.
5. Call submit_review once, with verdict approve or changes, a summary, and findings. Each finding has a severity (critical, important or minor), a file, an optional line and a summary. Use changes only when at least one finding is critical or important.
6. Messages that start with [anthrex] come from the orchestration engine.
```

Message texts, exact, with `<...>` filled in:

| Constant or function | Text |
|----------------------|------|
| `findings_message` | `[anthrex] Review round <n> requested changes: <summary>` then one line per finding `- [<severity>] <file>[:<line>] <summary>`, then `Address every critical and important finding, commit, then call report_done again.` |
| `verify_failed_message` | `[anthrex] The verify command failed in your worktree (<exit <code> \| timed out after 20 minutes>): <command>` then `Last lines of its output:` and the tail, then `Fix it, commit, then call report_done again.` |
| `conflict_message` | `[anthrex] Merging your branch conflicted in: <files, comma-separated>. Merge <run_branch> into your branch, resolve the conflicts, run the tests, commit, then call report_done.` |
| `retry_message` | `[anthrex] Task <id> was sent back to you. Call get_task, continue the work, and call report_done when it is committed.` |
| `RESUME_WORKER` | `[anthrex] The daemon restarted. Call get_task to reload your task, then continue. Call report_done when your work is committed.` |
| `RESUME_REVIEWER` | `[anthrex] The daemon restarted. Call get_review, finish your review, and call submit_review.` |
| `no_commits_error` | `No commits beyond the start commit <sha7>. Commit your work, then call report_done again.` |
| `DIRTY_ERROR` | `Your worktree has uncommitted changes to tracked files. Commit them, then call report_done again.` |

### MCP tools (`crates/mcp`, new)

```rust
pub struct McpOptions { pub role: proto::Role, pub run_id: String, pub task_id: Option<String>, pub window_id: u32, pub socket: PathBuf }
pub const TOOL_REPLY_TIMEOUT: Duration = Duration::from_secs(100);
pub fn tools_for(role: proto::Role) -> Vec<rmcp::model::Tool>;    // tools.rs
pub async fn forward(opts: &McpOptions, tool: &str, args: serde_json::Value) -> (bool, String); // forward.rs
pub async fn serve_on<R, W>(opts: McpOptions, reader: R, writer: W) -> anyhow::Result<()>
    where R: tokio::io::AsyncRead + Unpin + Send + 'static, W: tokio::io::AsyncWrite + Unpin + Send + 'static; // lib.rs
pub async fn serve_stdio(opts: McpOptions) -> anyhow::Result<()>; // lib.rs: serve_on with rmcp::transport::stdio()
```

Descriptions and input schemas. Every schema is `{"type":"object","additionalProperties":false, ...}`.

| Role | Tool | Description | Properties (required in bold) |
|------|------|-------------|-------------------------------|
| worker | `get_task` | `Get your task: prompt, acceptance criteria, run notes, worktree, branch and start commit.` | none |
| worker | `report_done` | `Report that the task is complete and committed.` | **`summary`**: string, 1 to 4000 characters |
| worker | `report_blocked` | `Report that you cannot continue, and why.` | **`reason`**: string, 1 to 4000 |
| worker | `ask` | `Ask the human a question. End your turn after calling it.` | **`question`**: string, 1 to 4000 |
| reviewer | `get_review` | `Get what to review: the task, acceptance criteria, base and head commits, and earlier rounds.` | none |
| reviewer | `submit_review` | `Submit your review verdict and findings.` | **`verdict`**: enum `approve`, `changes`; **`summary`**: string, 1 to 4000; **`findings`**: array, at most 50, of objects with **`severity`** enum `critical`, `important`, `minor`, **`file`** string of 1 to 500, `line` integer at least 1, **`summary`** string of 1 to 2000 |

The daemon validates the same limits again and returns `invalid arguments: <field>: <problem>`.

`get_task` result JSON: `{"run_id","goal","task":{"id","title","kind","prompt","acceptance"},"notes","worktree","branch","start_commit","round","state","previous_findings":[Finding...]}`. `previous_findings` are the findings of every round with an effective `changes` verdict.

`get_review` result JSON: `{"run_id","goal","task":{...} or null,"acceptance":[...],"base","head","worktree","round","verify":{"ok","tail"} or null,"previous_rounds":[{"round","verdict","summary","findings"}]}`. For the final review, `task` is null, `acceptance` lists every merged task's criteria prefixed with `<id>: `, and `previous_rounds` holds earlier final-review rounds.

### CLI

```
anthrex run start --plan <file> [--dir <dir>] [--yes]
anthrex run approve <run>
anthrex run status [<run>] [--json]
anthrex run retry <run> <task|integration>
anthrex run skip <run> <task>
anthrex run finish <run> merge|pr|keep|discard [--yes] [--confirm <run-id>]
anthrex run cancel <run>
anthrex run resume <run>
anthrex mcp --role worker|reviewer --run <run> [--task <task>] --window <id> [--socket <path>]   (hidden)
```

- `run start` reads the file, auto-starts the daemon, prints the run id on stdout, and, when the run awaits approval, `approve with: anthrex run approve <id>` on stderr. `--dir` is the existing global flag, resolved with `resolve_dir`.
- `run finish`: `merge` and `pr` print the daemon's confirmation prompt and ask `Proceed? [y/N]` unless `--yes`. `discard` prints the prompt and asks the user to type the run id, unless `--confirm <run-id>` is given. The CLI then repeats the request with `confirm` set to the run id.
- `run status` text, one block per run, newest first:

```
add-password-reset-3f9a  running  1/3 merged  base main@1a2b3c4
  goal: Add password reset
  report: /Users/me/Library/Application Support/anthrex/worktrees/shop-a3cbe2c8/runs/add-password-reset-3f9a/REPORT.md
  ID   STATE              WORKER                 REVIEWER               ROUNDS  WINDOWS
  t1   merged             codex (default)        claude claude-sonnet-5  1       4 5
  t2   reviewing          claude claude-sonnet-5 codex (default)         1       6 7
  t3   planned            codex (default)        claude claude-sonnet-5  0
  attention: t2 asks: which mail provider?
```

A Paused run shows `paused (from <state>)` as its state. `--json` prints `serde_json::to_string_pretty` of `Vec<RunInfo>`.
- Run requests use a 180 s reply timeout: new `CliClient::request_timeout(msg, Duration)`. `CliClient` gains `pub runs: Vec<RunInfo>` from `Welcome`, and `request` also skips `RunsChanged`.

## Tasks

Shared test helpers:

- Daemon-level git tests use milestone 5's `TempRepo` from `crates/daemon/tests/common/mod.rs`.
- End-to-end tests live in `crates/cli/tests/` and share a new module `crates/cli/tests/support/run_harness.rs`, declared from milestone 3's `crates/cli/tests/support/mod.rs`. `RunHarness::new()`: a temp dir from `tempfile::Builder::new().prefix("ax-run").tempdir_in("/tmp")`; a repo at `<tmp>/repo` with `git init -b main`, repo-local `user.name`, `user.email`, `commit.gpgsign false`, and one commit of `README`; the daemon started with `anthrex daemon start` and the environment `ANTHREX_SOCKET=<tmp>/d.sock`, `ANTHREX_DATA_DIR=<tmp>/data`, `ANTHREX_CONFIG=<tmp>/config.toml`, `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` both the `fake-agent` path, `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`. Methods: `config(&str)` writes the config file before the daemon starts; `script(name, lines)` writes `<repo>/.git/fake-agent/<name>.jsonl`; `plan(json) -> PathBuf` writes it outside the repo; `anthrex(args) -> Output` runs the CLI with the same environment; `client()` opens a raw socket client that keeps the latest `RunInfo`s from `Welcome` and `RunsChanged`; `wait_run(id, pred, timeout) -> RunInfo` waits on those with a deadline, printing the last `RunInfo` and the tail of `daemon.log` on timeout; `git(args) -> String` runs in the repo. `Drop` runs `anthrex daemon stop` and waits until the socket file is gone. `fake-agent` comes from milestone 3's `support::fake_agent_bin()`.

### M8.1 Verify the external flags

**Files.** Modify only this brief's "Implementation notes".

**Tests first.** None. This task records facts.

**Change.** Against the installed tools, record the command, the version and the relevant output for each:

1. `codex --version`, `codex --help` (`-c`, `-s`, `-a`, `-m` present), `codex mcp --help`, `codex features list`.
2. Whether the Codex configuration keys `mcp_servers.<name>.default_tools_approval_mode`, `mcp_servers.<name>.tool_timeout_sec` and `developer_instructions` exist in the installed version. Search the native Codex binary with `strings <binary> | grep -E 'default_tools_approval_mode|tool_timeout_sec|developer_instructions'`. Resolve the binary through the `codex` launcher script if it is one.
3. `claude --version` and `claude --help`: `--mcp-config`, `--allowedTools`, `--append-system-prompt`, `--permission-mode` present, and whether `--mcp-config` and `--allowedTools` are still variadic.
4. `gh --version`.

If a key or flag is missing, stop work on the item that uses it, as `AGENTS.md` says: record the evidence, leave that flag out of `role_launch.rs`, and continue. The behavioural check that `default_tools_approval_mode="auto"` suppresses the approval prompt needs a real Codex session; it is step 6 of the manual check.

**Acceptance.** "Implementation notes" has a dated "M8.1 external flags" entry with every item above.

### M8.2 Protocol version 5

**Files.** Create `crates/proto/src/run.rs`. Modify `crates/proto/src/lib.rs`, `crates/proto/src/messages.rs`, `crates/proto/src/types.rs`, every construction of `WindowInfo` and `DaemonMsg::Welcome` in `crates/daemon`, `crates/tui` and `crates/cli` (tests included), `crates/daemon/src/server.rs`, `crates/cli/src/client.rs`, `crates/tui/src/app/mod.rs`, milestone 3's `anthrex hook` client in `crates/cli/src/hook.rs`.

**Tests first.** In `crates/proto/src/run.rs` and `messages.rs`:

- `plan_parses_the_spec_example`: the JSON from spec 8.3 parses into `Plan` with one task, `runtime == Codex`, `model == None`, reviewer `claude`/`claude-sonnet-5`.
- `plan_rejects_unknown_fields`: adding `"priority": 1` to a task fails to parse, and the error names `priority`.
- `states_serialize_as_snake_case`: `TaskState::NeedsHuman` is `"needs_human"`, `RunState::AwaitingApproval` is `"awaiting_approval"`; `label()` gives `needs human` and `awaiting approval`.
- `is_active_matches_decision_19`: exactly the eight active states return true.
- `every_new_client_message_round_trips` and `every_new_daemon_message_round_trips`: each new variant, with a nested `RunInfo` holding one `TaskInfo` and a `ToolCall` whose `args` is `{"summary":"x","n":[1,2]}`, survives MessagePack.
- `window_info_run_defaults_to_none`: a `WindowInfo` serialized without `run` through JSON deserializes with `run == None`.

**Change.** Add the Interfaces types and messages. Set `PROTO_VERSION = 5`. Add `ClientKind::Mcp`. The server answers every new request with `Error { request, message: "runs are not available yet" }` until M8.14, and `ToolCall` with `ToolResult { ok: false, text: "runs are not available yet" }`. `Welcome` sends `runs: vec![]`. The TUI stores `runs` from `Welcome` and `RunsChanged` in a new `App.runs: Vec<RunInfo>` and renders nothing new yet.

**Acceptance.** Workspace builds and all tests pass. `grep -rn "PROTO_VERSION: u32 = 5" crates/proto` matches.

### M8.3 Orchestrator config and roster

**Files.** Modify the `config` crate (`crates/config/src/lib.rs`) and `crates/proto/src/run.rs` (`Tier`, `ModelEntry`). Create `crates/daemon/src/run/mod.rs`, `crates/daemon/src/run/roster.rs`. Modify `crates/daemon/src/lib.rs` (`pub mod run;`), `crates/daemon/Cargo.toml` (add `config`, `serde`, `serde_json` if absent).

**Tests first.**

- In the config crate: `orchestrator_defaults_when_absent` (all defaults, `models == default_roster()`, four entries in spec order); `orchestrator_keys_are_read` (a file with every key set to a non-default valid value); `bad_orchestrator_values_fall_back_with_problems` (`max_parallel = 0`, `max_parallel = 9`, `worker_codex_sandbox = "yolo"` each give one problem and the default); `invalid_model_entries_are_skipped_with_their_index` (runtime `perl`, tier `huge`, an empty Claude model, nine strengths, a 41-character strength: one problem each, naming `orchestrator.models[<i>]`); `user_models_extend_and_replace_builtins` (a `claude`/`claude-sonnet-5` entry with tier `frontier` replaces the built-in in place, a `codex`/`gpt-x` entry is appended: five entries); `builtin_models_false_drops_builtins` (only the user's entries remain); `duplicate_user_models_keep_the_first`; `no_valid_models_uses_the_default_roster` (with `builtin_models = false` and only bad entries); `unknown_orchestrator_key_is_reported`. In `proto`: `tier_orders_and_serializes_lowercase`. Update milestone 6's `every_key_is_read` fixture: its `[orchestrator] max_parallel = 3` now parses with no problems.
- In `roster.rs`: `tier_of_null_model_uses_the_empty_entry_or_standard` (codex null is `standard` from the empty entry; claude null is `standard`; `claude`/`gpt-x` is `None`); `reviewer_is_other_runtime_same_tier_or_higher` (default roster: codex null gets `claude-sonnet-5`; `claude-haiku-4-5` gets codex null; `claude-sonnet-5` gets codex null); `frontier_author_falls_back_to_other_runtime_highest_tier` (`claude-opus-5` gets codex null, rule (c)); `same_runtime_different_model_is_rule_b` (a roster with two claude entries only); `author_itself_is_the_last_resort` (a one-entry roster); `final_reviewer_is_the_first_frontier_entry` (default roster gives `claude-opus-5`; a roster without frontier gives its first standard entry).

**Change.** Implement `Tier`, `ModelEntry`, the `config` interface and parsing rules, and `roster.rs` per decisions 20 and 21. Remove milestone 6's silent skip of `[orchestrator]`. Milestone 9 builds on this roster and adds only `resolve_orchestrator` and the allowed-runtimes filter.

**Acceptance.** Tests pass. `roster.rs` imports nothing from `std::fs`, `std::process` or `tokio`.

### M8.4 Plan parsing, validation and slug

**Files.** Create `crates/daemon/src/run/plan.rs`, `crates/daemon/src/run/model.rs` (types and path helpers only; `info()` may be a stub returning the fields it can).

**Tests first.** In `plan.rs`:

- `valid_plan_passes`: the spec example plus a second task depending on `t1`.
- One test per rule in decision 18, each asserting the exact message: `duplicate_ids` (`task t1: id: duplicate id`), `bad_id_syntax` (`task T1: id: must match [a-z0-9][a-z0-9-]{0,15}`), `reserved_ids` (`integration`, `final`, `t1-review-2` each give `task <id>: id: reserved name`), `unknown_dependency` (`task t2: depends_on: unknown task 't9'`), `self_dependency_is_a_cycle` and `cycle_is_reported_once` (`depends_on: cycle t1 -> t2 -> t1`, reported once for the cycle, naming the ids in plan order starting from the first one in the cycle), `too_many_tasks` (13 tasks with `max_tasks` 12: `tasks: 13 tasks, at most 12 allowed`), `shell_runtime_rejected` (`task t1: runtime: must be claude or codex`), `model_outside_roster` (`task t1: model: claude model 'gpt-x' is not in the roster`), `reviewer_outside_roster` (`task t1: reviewer: ...` with the same wording), `blank_fields` (`goal: must not be empty`, `task t1: title: must not be empty`, `task t1: prompt: must not be empty`, `task t1: acceptance: item 2 must not be empty`, `verify: must not be empty`), `empty_model_string_is_null`.
- `all_errors_are_collected`: a plan with three distinct problems returns three messages.
- `slug_examples`: `slug("Add password reset", 0x3f9a) == "add-password-reset-3f9a"`; `slug("  Fix: the ÄPI!! ", 1) == "fix-the-pi-0001"`; `slug("!!!", 2) == "run-0002"`; a 60-character goal gives a slug whose part before the suffix is at most 32 characters and does not end with `-`.
- `build_run_resolves_reviewers_and_paths`: `build_run` with a preflight for `/tmp/r` and `wt_dir` `/tmp/wt` gives `state == Planning`, `tasks[0].branch == "anthrex/<id>/t1"`, `tasks[0].worktree == "/tmp/wt/runs/<id>/t1"`, `reviewer_chosen` true where no reviewer was given, and `limits` copied from the config.

**Change.** Implement decisions 7, 17, 18 and 20 and the `model.rs` types. `build_run` sets `state = RunState::Planning`; the engine sets the real start state.

**Acceptance.** Tests pass. `plan.rs` and `model.rs` import nothing from `std::fs`, `std::process` or `tokio`.

### M8.5 Run git operations and the verify runner

**Files.** Create `crates/daemon/src/run/git.rs`, `crates/daemon/src/run/verify.rs`, `crates/daemon/tests/run_git.rs`.

**Tests first.** In `crates/daemon/tests/run_git.rs`, each with a fresh `TempRepo`:

- `preflight_reports_branch_sha_and_roots`: `root` and `project` are the repo; `base_branch == "main"`; `base_sha` is `HEAD`.
- `preflight_refuses_a_dirty_tree_but_not_untracked_files`: modify `README`: the exact decision 9 message. Revert it and add an untracked file: `Ok`.
- `preflight_refuses_detached_head_and_non_repo`: the two messages.
- `run_and_task_worktrees_follow_the_layout`: create the run worktree at `base_sha`, commit a file in it, then create a task worktree. The task branch's parent commit is the run branch head, the returned start is that sha, and `git worktree list --porcelain` lists both paths.
- `task_worktree_is_reused`: calling `create_task_worktree` again returns `Ok(None)` and changes nothing; after `git worktree remove` of the path, it re-adds the existing branch and still returns `Ok(None)`.
- `submission_counts_commits_and_dirt`: 0 commits gives `(0, false)`; one commit gives `(1, false)`; a modified tracked file gives `dirty == true`.
- `review_worktree_is_detached_at_the_task_head`: returns `(base, head)` with `base` the merge-base; `git -C <path> rev-parse HEAD` equals `head`; `git -C <path> symbolic-ref -q HEAD` fails. Calling it again with the same path succeeds (decision 13's re-create).
- `merge_is_no_ff_and_idempotent`: merging a one-commit task branch gives a merge commit with two parents; merging again returns `Merged` with the same `HEAD`.
- `merge_conflict_is_detected_and_aborted`: two branches change the same line; the second merge returns `Conflict(vec!["same.txt"])`, and afterwards `MERGE_HEAD` does not exist and `git status --porcelain` is empty.
- `merge_recovers_from_a_leftover_merge_head`: leave a conflicted merge in progress by hand, then `merge` another clean branch: it aborts the leftover and merges.
- `finish_merge_requires_the_base_branch_and_a_clean_tree`: with another branch checked out, the decision 16 message; on `main`, it merges and `main` contains the run's file.
- `pr_target_prefers_the_branch_remote_then_origin`: no remote gives `no remote named origin`; with `origin` set to a bare repo in the temp dir, the target is `origin` and its URL.
- `discard_removes_worktrees_branches_and_the_run_dir`: after creating a run and two task worktrees, `discard` removes all three paths, every `anthrex/<slug>/` branch, and the run directory, and `main` is untouched.
- `verify_captures_the_last_200_lines_and_exit_code`: `seq 1 500; exit 3` gives `ok == false`, `code == Some(3)`, a tail of 200 lines starting with `301`.
- `verify_merges_stderr`: `echo out; echo err >&2` has both in the tail.
- `verify_timeout_kills_the_process_group`: command `sleep 30 & echo $! > <tmp>/bg; wait` with a 1 s timeout returns `timed_out == true` within 3 s, and the background `sleep` pid is gone within 2 s (`kill(pid, 0)` fails with `ESRCH`).

**Change.** Implement decisions 9 to 16. Every git call goes through `crate::git::run` with a fresh `RUN_GIT_TIMEOUT` deadline. `finish_pr` runs `gh` with `std::process::Command` in its own process group, stdin `/dev/null`, a 60 s timeout, and the same kill-on-timeout pattern as `git::run`. The `PATH` search for `gh` is a plain loop over `PATH` entries checking for an executable file.

**Acceptance.** Tests pass. `grep -rn 'Command::new("git")' crates/daemon/src` still prints only milestone 5's `git.rs`. No function in `run/git.rs` or `run/verify.rs` is `async`.

### M8.6 Engine I: run lifecycle, dispatch and worker tools

**Files.** Create `crates/daemon/src/run/engine/mod.rs`, `crates/daemon/src/run/engine/flow.rs`, `crates/daemon/src/run/engine/tools.rs`, `crates/daemon/src/run/contract.rs`, `crates/daemon/src/run/role_launch.rs` (the `RoleLaunch` struct only; the argv builders come in M8.10). Modify `crates/daemon/src/run/model.rs` (`Run::info`).

**Tests first.** Unit tests in `engine/mod.rs` under `#[cfg(test)] mod tests`, with a helper that builds a `Run` through `plan::build_run` from an inline plan, paths under `/tmp/x`, and feeds `Input`s with `now` values chosen by the test. Assertions match on the returned `Vec<Action>`.

- `start_without_yes_awaits_approval`: `Start` gives `Reply Ok(<id>)` and the run is AwaitingApproval with no `Op`.
- `start_with_auto_approve_creates_the_run_worktree`: `Start { auto_approve: true }` gives one `Op CreateRunWorktree` with branch `anthrex/<id>/integration` and the decision 8 path.
- `approve_moves_to_running_and_dispatches_after_the_run_worktree`: `Approve`, then `OpDone Worktree` for the run worktree: one `Op CreateTaskWorktree` per task with no dependencies, at most `max_parallel` of them, in plan order.
- `dependent_task_waits_for_merge`: with `t2` depending on `t1`, no `CreateTaskWorktree` for `t2` until `t1` is Merged (drive `t1` with scripted `OpDone`s to Merged), then exactly one.
- `window_created_after_worktree_with_role_and_prompt`: `OpDone Worktree { start: Some(sha) }` gives `Op CreateWindow` whose spec has name `<h4>/t1`, `cwd` the task worktree, the task's runtime and model, and an initial prompt equal to `contract::worker_prompt`; its `RoleLaunch` has role Worker, `task_id` `t1`, instructions `WORKER_CONTRACT`. `windows_created` becomes 1.
- `max_parallel_is_respected`: four independent tasks and `max_parallel` 2: two dispatched; completing one to Merged dispatches the third.
- `report_done_checks_commits_then_replies`: `Tool report_done` from the worker window gives `Op CheckSubmission` and no reply yet; `OpDone Submission { commits: 0, dirty: false }` gives `Reply Err(no_commits_error)` and the task stays Running; `{ commits: 2, dirty: true }` gives `Reply Err(DIRTY_ERROR)`; `{ commits: 2, dirty: false }` gives `Reply Ok(<decision 44 text>)`, and with no verify command the task goes to Reviewing with an `Op PrepareReview`.
- `tool_authorization`: one assertion per rejection text in decision 25, including a reviewer calling `report_done`, the worker window of `t1` calling as `t2`, a Paused run and a Cancelled run.
- `blocked_and_ask`: `report_blocked` makes the task Blocked with the note and attention `t1 blocked: <reason>`; `report_done` from Blocked is accepted; `ask` records the question in attention and the next tool call clears it.
- `get_task_returns_the_documented_fields`: parse the reply as JSON and check every key from the Interfaces section.
- `invalid_tool_arguments`: `report_done` with `{}` gives `invalid arguments: summary: required`; with a 5000-character summary gives `invalid arguments: summary: at most 4000 characters`.
- `window_limit_sends_the_task_to_needs_human`: `max_windows` 1 and two tasks: the second gets NeedsHuman with `run window limit (1) reached`.
- `contract_prompts`: `worker_prompt` starts with `[anthrex] Task t1: ` and ends with `WORKER_CONTRACT`; it omits `Run notes:` when there are none; with `earlier_attempt` it contains the extra line.
- `generation_bumps_on_every_mutation`.

**Change.** Implement the engine skeleton, decisions 19, 25, 28, 29, 33, 40 and 44, run lifecycle up to Reviewing, and `contract.rs` in full. `Engine` keeps pending ops in a `BTreeMap<OpId, PendingOp>` with the run id, task id and purpose; results for unknown ops or terminal runs are dropped. Every state change appends a `LogEntry` like `t1: running -> submitted` and emits `WriteReport`.

**Acceptance.** Tests pass. `crates/daemon/src/run/engine/` imports nothing from `std::fs`, `std::process`, `std::thread` or `tokio`. No file under `run/` exceeds 600 lines.

### M8.7 Engine II: verify, review, merge, conflict, integration and final review

**Files.** Modify `crates/daemon/src/run/engine/flow.rs`, `crates/daemon/src/run/engine/tools.rs`, `crates/daemon/src/run/engine/mod.rs`. Create `crates/daemon/src/run/messages.rs` (the `Gate`, `encode_paste` and `clamp`; M8.11 wires delivery).

**Tests first.** Unit tests in `engine/mod.rs`, continuing the M8.6 helper:

- `verify_runs_before_review`: with a verify command, accepted `report_done` gives `Op Verify` in the task worktree and state Verifying; `OpDone Verify { ok: true }` gives `PrepareReview` with `base_ref` the run branch, `head_ref` the task branch, path `<task>-review-1`.
- `verify_failure_requests_changes`: `ok: false` puts `verify_failed_message` in the outbox for the worker window, state ChangesRequested, `rounds_used` 1.
- `reviewer_window_uses_the_chosen_reviewer`: after `OpDone Review`, `Op CreateWindow` with name `<h4>/t1-r1`, cwd the review worktree, the task's reviewer runtime and model, `RoleLaunch` role Reviewer with `REVIEWER_CONTRACT`, `claude_permission_mode Some("plan")` for Claude, `codex_sandbox Some("read-only")` and `codex_approval Some("on-request")` for Codex, and an initial prompt equal to `reviewer_prompt`.
- `approve_queues_a_merge`: `submit_review approve` gives `Reply Ok`, `RetireWindow` for the reviewer, then `Op Merge` with message `anthrex: merge t1: <title>`.
- `changes_without_serious_findings_count_as_approve`: only minor findings: `effective == Approve`, merged.
- `changes_request_goes_back_to_the_worker`: an important finding: outbox has `findings_message`, state ChangesRequested; `deliverable` with the worker Idle returns that text and the task becomes Running.
- `max_rounds_goes_to_needs_human`: `max_review_rounds` 2: first changes delivered; second changes gives NeedsHuman `review rounds exhausted (2)` and no second message.
- `merges_are_serialized`: two tasks approved together: one `Op Merge`, the second only after the first's `OpDone Merged`.
- `merged_task_retires_the_worker_and_unblocks_dependants`.
- `conflict_goes_back_to_the_worker`: `OpDone Conflict { files: ["same.txt"] }` gives state Conflict and `conflict_message` in the outbox; after delivery, Running; the next accepted `report_done` goes through review round 2 and merges.
- `conflict_with_the_worker_gone_needs_a_human`: after `WindowExited` for the worker, a conflict gives NeedsHuman `worker window is gone`.
- `all_merged_moves_to_integrating_then_final_review_then_ready`: run verify `Op Verify` in the run worktree; ok gives `PrepareReview` with `base_ref == base_sha`, `head_ref` the run branch, path `final-review-1`; the final reviewer window is `<h4>/final-r1` with `task_id None` and the final reviewer from the roster; `submit_review approve` gives Ready.
- `run_verify_failure_waits_for_the_user`: attention text from decision 30, state Integrating; `Approve` moves to FinalReview.
- `final_review_changes_still_reach_ready_with_attention`.
- `all_cancelled_skips_to_ready`.
- `op_failure_fails_the_task_or_the_run`: `Failed` for `CreateTaskWorktree` gives task Failed with the message; for `CreateRunWorktree` gives run Failed.
- In `messages.rs`: `encode_paste_wraps_and_normalizes` (`"a\nb"` gives `ESC[200~a\rb ESC[201~`, embedded markers removed); `clamp_keeps_head_and_tail` (a 100 KiB text is at most 32 KiB, starts with the original start, ends with the original end, contains `[... truncated ...]`); `gate_waits_for_busy_or_60_seconds`.

**Change.** Implement decisions 12 (engine side), 13, 22, 23, 24, 27, 30 and 41's gate inside `Engine::deliverable`: it returns deliveries for windows the gate allows (at most one per window per call, oldest first), marks the delivered messages sent, moves ChangesRequested and Conflict tasks to Running, and returns the ids of windows whose oldest message just crossed 600 s (decision 42).

**Acceptance.** Tests pass. The purity grep from M8.6 still holds.

### M8.8 Engine III: control operations, window exits, pause and resume

**Files.** Create `crates/daemon/src/run/engine/control.rs`. Modify `crates/daemon/src/run/engine/mod.rs`.

**Tests first.** Unit tests in `engine/mod.rs`:

- `retry_reuses_a_live_worker`: a Blocked task whose worker window is live: `Retry` gives Planned then Running with `retry_message` in the outbox and no `CreateWindow`.
- `retry_after_the_worker_exited_creates_a_new_worker`: gives `CreateTaskWorktree` then `CreateWindow` named `<h4>/t1-2` whose prompt has the earlier-attempt line; `rounds_used` is 0 and `round` is unchanged.
- `retry_drops_cancelled_dependencies`: `t2` depends on skipped `t1`; retrying `t2` removes `t1` from its `depends_on`, logs it, and dispatches.
- `retry_integration_reruns_verify`.
- `retry_rejects_other_states`: retrying a Running task gives `task t1 is running; only needs human, blocked or failed tasks can be retried`.
- `skip_cancels_and_flags_dependants`: `t1 <- t2 <- t3`; skipping `t1` kills its windows, `t1` Cancelled, `t2` and `t3` NeedsHuman `dependency t1 was skipped`.
- `skip_refuses_merging_and_merged`.
- `cancel_kills_every_live_window`: `KillWindow` for each live worker and reviewer, tasks not Merged become Cancelled, run Cancelled; a later `OpDone` for that run is dropped.
- `worker_exit_while_running_needs_a_human`, `reviewer_exit_before_submit_needs_a_human`, `retired_window_exit_is_ignored`.
- `restore_pauses_non_terminal_runs`: a Running run and a Finished run: the first becomes Paused with `paused_from == Running`; the second is unchanged; a restored Paused run keeps its `paused_from`.
- `paused_run_rejects_everything_but_resume_and_cancel`.
- `resume_restarts_windows_and_queues_messages`: a paused run with one Running task (worker 4), one Reviewing task (worker 5, reviewer 6, not submitted), one Approved task: `Resume` gives `Op RestartWindow` for 4, 5 and 6, `RESUME_WORKER` queued for 4 and 5, `RESUME_REVIEWER` for 6, and `Op Merge` for the Approved task.
- `resume_rechecks_submitted_and_verifying`: Submitted becomes Running with the resume message; Verifying issues `Op Verify` again.
- `restart_failure_needs_a_human`.
- `finish_actions_by_state`: `Finish merge` in Running gives `run <id> is running; only a ready run can be merged`; in Ready gives `Op FinishMerge`, and `OpDone Finished` gives Finished with the outcome and `Reply Ok`; `keep` from Cancelled gives Finished `kept` with no op; `discard` gives `RemoveWindow` for every run window, then `Op Discard` with every decision 8 path, branch prefix `anthrex/<id>/`, and the run dir.

**Change.** Implement decisions 26, 30 (control side), 31, 47 and 49 and the finish state rules of decision 16. Confirmation checking is not in the engine; `RunService::finish` does it (M8.14).

**Acceptance.** Tests pass. The purity grep holds.

### M8.9 The run report

**Files.** Create `crates/daemon/src/run/report.rs`.

**Tests first.** Unit tests in `report.rs` with a `Run` built as in M8.6 and filled by hand:

- `format_utc_vectors`: `0` gives `1970-01-01 00:00:00Z`; `951782400` gives `2000-02-29 00:00:00Z`; `1789123456` gives `2026-09-11 10:44:16Z`.
- `report_has_every_section`: the output contains, in order, `# anthrex run <id>`, `Goal: <goal>`, the state line, `## Tasks` with one table row per task, `## t1: <title>`, `### Round 1: changes`, a finding line `- [important] src/a.rs:12 Missing expiry test`, `## Run verify`, `## Final review`, `## Log`.
- `report_marks_chosen_reviewers_and_effective_verdicts`: a chosen reviewer shows `(chosen by the engine)`; a changes verdict with only minors shows `changes, counted as approve`.
- `report_shows_verify_tails_in_fenced_blocks`.

**Change.** Implement `render` and `format_utc` (days-from-civil arithmetic, no new dependency). Layout: header block (goal, state, base branch and short sha, run branch, verify command or `none`, worker permission mode and Codex worker sandbox or `user default`, created time); `## Tasks` table with columns id, title, state, worker, reviewer, rounds, merge commit; one `## <id>: <title>` section per task with kind, worker and window ids, reviewer, dependencies, the worker's `report_done` summary, each verify with time, result and the tail in a fenced block, each review round with verdict, effective verdict, summary and findings, and the merge commit; `## Run verify`; `## Final review`; `## Log` with the last 200 log entries as `- <utc> <text>`. `RunService` writes it in M8.14.

**Acceptance.** Tests pass. `report.rs` is pure except that it is called by the driver.

### M8.10 Role launch flags and run windows

**Files.** Modify `crates/daemon/src/run/role_launch.rs`, `crates/daemon/src/launch/mod.rs`, `crates/daemon/src/launch/claude.rs`, `crates/daemon/src/launch/codex.rs`, `crates/daemon/src/manager.rs`, `crates/daemon/src/status.rs`, `crates/daemon/src/state.rs`.

**Tests first.**

- In `role_launch.rs`: `toml_string_round_trips_through_the_toml_crate` (for 20 strings including every control character, U+007F, quotes, backslashes and a whole contract, parse `x = <launch::codex::toml_string(s)>` with the `toml` crate from milestone 6 and get the original back; milestone 3 tests the exact escapes); `mcp_args_for_worker_and_final_reviewer` (exact vectors: `["mcp","--role","worker","--run","r-3f9a","--task","t1","--window","7","--socket","/tmp/a.sock"]`, and without `--task` for a final reviewer).
- In `launch/claude.rs` and `launch/codex.rs`: `claude_worker_role_flags` (exact argv: milestone 3's flags, then `--mcp-config`, the JSON (parse it and compare to the expected value), `--allowedTools`, `mcp__anthrex__*`, `--append-system-prompt`, the contract, then `--model`, `--`, the prompt); `claude_reviewer_uses_plan_mode`; `claude_worker_permission_mode_only_when_not_default`; `claude_variadic_flags_are_followed_by_a_flag` (for every combination of role, model and resume, the element after the `--mcp-config` value and after the `--allowedTools` value starts with `--`); `codex_worker_role_flags` (the five `-c` pairs from decision 37 in order, then `-m`, `--`, prompt); `codex_reviewer_is_read_only_on_request`; `codex_worker_sandbox_only_when_set`; `role_flags_survive_resume` (with `resume: Some(..)`, the role flags are present and the prompt is absent).
- In `status.rs`: `engine_attention_marks_attention_except_exited`.
- In `crates/daemon/tests/manager.rs`: `create_with_role_sets_window_info_run` (a Claude window whose command is `fake-agent` running a script that only waits: `WindowInfo.run == Some(RunRef { .. })`); `role_is_in_the_state_snapshot_and_survives_restore` (`state_snapshot().windows[0].run` equals the `RoleLaunch`; after `restore`, `WindowInfo.run` is set and `restart` relaunches with the role flags, checked with milestone 6's `argv.sh` pattern: the printed argv contains `--mcp-config`); `mark_attention_sets_attention`.

**Change.** Implement decisions 35 to 38 and 43. `launch::plan` appends `claude_role_args` or `codex_role_args` at the positions given, when `ctx.role` is `Some`. `create_with_role` shares `create`'s code path and stores the role on the `Entry`; `create` with a client-supplied spec never sets a role. `WindowRecord.run` becomes `Option<RoleLaunch>`; a record whose `run` fails to parse loads with `run: None` and a warning.

**Acceptance.** Tests pass. No `CreateWindow` from a client can produce a window with `WindowInfo.run` set.

### M8.11 Message delivery

**Files.** Modify `crates/daemon/src/run/messages.rs`. Create `crates/daemon/tests/run_messages.rs`.

**Tests first.** In `crates/daemon/tests/run_messages.rs`, with a real PTY:

- `delivery_waits_for_idle_and_uses_bracketed_paste`: create a Shell window running `sh -c 'stty -echo; cat -v'`. Queue a message while it is Working (right after spawn). Assert nothing is written until the window is Idle (the 3 s quiet rule). Then call the delivery function: within 3 s the screen shows `^[[200~[anthrex] hello^[[201~` and, on the next line, nothing else; the `\r` arrives at least 150 ms after the paste (read timestamps from the output subscription).
- `a_full_input_queue_is_reported_not_blocking`: a window whose child never reads (`stty raw; sleep 30`); deliver 300 messages of 8 KiB in a loop; every call returns within 50 ms, and the ones that do not fit return an error instead of blocking.

In `messages.rs` unit tests: `ten_minute_rule_flags_once` (a message queued at 0 and polled at 599 and 600 and 700 s flags exactly once, at 600).

**Change.** Add `pub async fn deliver(manager: &WindowManager, window_id: u32, text: &str) -> anyhow::Result<()>` to `messages.rs`: `write_input(encode_paste(clamp(text)))`, `tokio::time::sleep(SUBMIT_DELAY)`, `write_input(b"\r")`. It never holds a lock across the sleep. A failed write leaves the message in the outbox for the next poll.

**Acceptance.** Tests pass. `grep -n "thread::sleep" crates/daemon/src/run` prints nothing.

### M8.12 The MCP server

**Files.** Create `crates/mcp/Cargo.toml`, `crates/mcp/src/lib.rs`, `crates/mcp/src/tools.rs`, `crates/mcp/src/forward.rs`, `crates/mcp/tests/stdio.rs`. Modify the workspace `Cargo.toml` (member, `rmcp` and `mcp` workspace dependencies), `crates/cli/Cargo.toml`, `crates/cli/src/main.rs` (hidden `mcp` subcommand).

**Tests first.**

- In `tools.rs`: `worker_and_reviewer_tool_lists` (names exactly as the Interfaces table, in that order); `schemas_are_closed_objects` (every schema has `type: object` and `additionalProperties: false`); `submit_review_schema_limits` (the enum values and `maxItems: 50`).
- In `crates/mcp/tests/stdio.rs`, against a stub daemon: a `UnixListener` in a temp dir under `/tmp` that answers `Hello` with `Welcome` and each `ToolCall` with `ToolResult { ok: true, text: <the call echoed as JSON> }`, recording what it received. `crates/mcp` has no binary, so these tests drive the server through an in-memory `tokio::io::duplex` pair with `mcp::serve_on` (Interfaces):
  - `initialize_then_list_tools`: send `initialize` with protocol version `2025-06-18`, then `notifications/initialized`, then `tools/list`; the server name is `anthrex` and the tools match the role.
  - `tool_call_is_forwarded_with_role_run_task_and_window`: `tools/call report_done {"summary":"s"}` makes the stub record `ToolCall { run_id, task_id: Some("t1"), role: Worker, window_id: 7, tool: "report_done", args }`, and the MCP result text is the stub's text with `isError: false`.
  - `daemon_error_becomes_is_error`: the stub answers `ok: false`; the MCP result has `isError: true`.
  - `daemon_down_is_a_tool_error_not_a_crash`: no listener: the result has `isError: true` and text containing `cannot reach the anthrex daemon`, and the server keeps serving the next request.
- In `crates/cli/tests/mcp_cli.rs`: `mcp_subcommand_speaks_json_rpc_on_stdout_only`: spawn the real binary with `mcp --role reviewer --run r --window 1 --socket <nonexistent>`, write `initialize` and `tools/list` lines, read two JSON lines from stdout, and assert every stdout line parses as JSON.

**Change.** Implement decisions 4, 5, 35, 44 and 45 and the MCP interfaces. `serve_stdio` sets up no logging on stdout; any diagnostics go to stderr. Tool calls not in the role's list return `isError: true` with `unknown tool <name>` without contacting the daemon.

**Acceptance.** Tests pass. `cargo tree -p anthrex-mcp` shows `rmcp v3.4.0`. `cargo tree -p anthrex-daemon` and `cargo tree -p anthrex-tui` do not contain `rmcp`.

### M8.13 `fake-agent` additions

**Files.** Modify `crates/fake-agent/src/*` (the files milestone 3 created). Create `crates/fake-agent/tests/mcp_and_scripts.rs`.

**Change.** Milestone 3 owns the base script format and the steps `print`, `hook`, `notify`, `title`, `bell`, `wait_ms`, `read_line`, `git_commit` and `exit`; keep them unchanged. This milestone implements `mcp_call` and adds the items below. Product spec 12 lists every step with its owner; milestone 9 adds `mcp_wait`, `expect` and `FAKE_AGENT_LOG` on top of these.

1. **Role detection.** Parse the MCP server command and arguments from `--mcp-config <json>` (Claude style) or from the `-c mcp_servers.anthrex.command=...` and `-c mcp_servers.anthrex.args=[...]` pairs (Codex style; parse the TOML values with the `toml` crate). The role is the value after `--role`, the task the value after `--task`, or `run` when there is no `--task` (the final reviewer, and milestone 9's orchestrator).
2. **Per-role scripts.** When a role is known, look in `<git common dir>/fake-agent/` (from `git rev-parse --path-format=absolute --git-common-dir` in the cwd) for files `<role>-<task>-<n>.jsonl`. Take the one with the smallest `n` whose `<file>.claimed` does not exist, claiming it by creating `<file>.claimed` with `OpenOptions::create_new`. With no match, fall back to `FAKE_AGENT_SCRIPT` as before.
3. **`{"mcp_call": {"tool": ..., "args": {...}}}`.** Replace milestone 3's stub, which exits 3, and its test `mcp_call_is_not_supported_yet`. Spawn the MCP server command, send `initialize` (protocol version `2025-06-18`), `notifications/initialized`, then `tools/call`; print `[fake-agent] <tool> -> ok: <text>` or `[fake-agent] <tool> -> error: <text>`, and close the child's stdin. Store the text for the next `sh` step as `FAKE_AGENT_RESULT`.
4. **`{"read_message": {"timeout_ms": n, "expect": "text"}}`.** Put stdin in raw mode with `libc::tcsetattr` (clear `ICANON` and `ECHO`), read until `ESC [ 201 ~` followed by `\r`, restore the terminal, strip the markers, store the text as the current message, and print `[fake-agent] message: <first line, cut to 80 chars>`. Exit 3 when `expect` is not contained; exit 4 on timeout.
5. **`{"sh": "command"}`.** Run `/bin/sh -c command` in the cwd with `FAKE_AGENT_MESSAGE` and `FAKE_AGENT_RESULT` set, print its output, and print `[fake-agent] sh exited <code>` when the code is not 0. Continue either way.

**Tests first.** In `crates/fake-agent/tests/mcp_and_scripts.rs`:

- `claims_scripts_in_order`: two scripts `worker-t1-1.jsonl` and `worker-t1-2.jsonl` in a temp repo's `.git/fake-agent/`; running the binary twice with Claude-style `--mcp-config` naming `--role worker --task t1` prints the first script's marker, then the second's, and both `.claimed` files exist.
- `codex_style_config_is_understood`: the same with the Codex `-c` pairs and role reviewer, task absent: picks `reviewer-run-1.jsonl`.
- `mcp_call_talks_to_a_real_mcp_server`: point the config at the real `anthrex mcp` binary (locate it next to the `fake-agent` binary, building it once behind a `OnceLock` with `$CARGO build -p anthrex --bin anthrex` when absent) with `--socket` naming a stub daemon like M8.12's; the fake agent prints `report_done -> ok:` and the stub recorded the call.
- `read_message_parses_a_bracketed_paste`: run the binary in a PTY (`portable-pty` as a dev-dependency) with a `read_message` step and `expect: "hello"`; write `ESC[200~[anthrex] hello\rsecond ESC[201~` then `\r`; the screen shows `[fake-agent] message: [anthrex] hello` and the process continues to the next step.
- `sh_step_sees_the_message`.

**Acceptance.** Tests pass. Milestone 3's `fake-agent` tests still pass.

### M8.14 `RunService` and the server

**Files.** Create `crates/daemon/src/run/driver.rs`. Modify `crates/daemon/src/run/mod.rs`, `crates/daemon/src/server.rs`, `crates/daemon/src/lifecycle.rs`, `crates/daemon/tests/server.rs` (pass a `RunService`), `crates/cli/tests/support/mod.rs` (declare `run_harness`), create `crates/cli/tests/support/run_harness.rs`, `crates/cli/tests/run_e2e.rs`.

**Tests first.** In `crates/cli/tests/run_e2e.rs`, using the harness and raw socket requests (the CLI arrives in M8.15):

- `one_task_plan_runs_to_ready`: plan with one Claude task `t1`, reviewer Codex, no verify. Scripts: `worker-t1-1`: `SessionStart` hook, `UserPromptSubmit` hook, `git_commit a.txt`, `mcp_call report_done {"summary":"added a.txt"}`, `Stop` hook, `wait_ms 600000`. `reviewer-t1-1`: `mcp_call get_review {}`, `mcp_call submit_review {"verdict":"approve","summary":"ok","findings":[]}`, a notify step, `wait_ms 600000`. `reviewer-run-1`: the same with the final review. `RunStart { yes: true }`, then `wait_run` until Ready within 60 s. Assert: `t1` Merged with a merge commit; `git log --merges anthrex/<id>/integration` has one merge; `REPORT.md` exists and contains `## t1:`; the worker window's `WindowInfo.run` is `Some(RunRef { task_id: Some("t1"), role: Worker, .. })`; the reviewer windows are Exited (retired) within 35 s.
- `tool_call_from_another_window_is_rejected`: during a run, send `ToolCall` for `t1` with `window_id` of a plain shell window: `ToolResult { ok: false, text: "this window is not the worker of task t1" }`.
- `dirty_tree_refuses_to_start`: modify `README`, `RunStart` gives `Error { request: "run start" }` with the decision 9 message.
- `validation_errors_come_back_as_one_error`: a plan with two problems gives both lines.
- `finish_requires_confirmation`: at Ready, `RunFinish { action: Merge, confirm: None }` gives `RunConfirmNeeded`; with `confirm: Some(<id>)` gives `RunFinished` and `main` contains `a.txt`.

**Change.**

1. `RunService` owns `Mutex<Engine>` (taken with `crate::lock`), an `mpsc::UnboundedSender<Input>`, a reply map from `ReplyId` to `oneshot::Sender<Result<String, String>>`, a `watch` of `Vec<RunInfo>`, and a `watch` of the engine generation for the persister.
2. `spawn` starts three tasks: the input loop; a status watcher on `WindowManager::watch()` that, for run windows, feeds `WindowExited` on the transition to Exited and calls `Gate::observe` and the delivery poll; and a 1-second ticker that runs the delivery poll and the retire checks.
3. The input loop takes the engine lock only around `Engine::handle` and `Engine::deliverable`, then executes actions with the lock released: `Reply` completes the oneshot; `Op` runs the matching `run/git.rs` or `run/verify.rs` call on `spawn_blocking`, or `WindowManager::create_with_role` (with the run's `project`) or `restart`, and sends `OpDone`; `KillWindow` and `RemoveWindow` call the manager; `RetireWindow` records a deadline for the ticker; `WriteReport` renders under the lock and writes `REPORT.md` on `spawn_blocking` via a temp file and rename, at most once per 500 ms per run. Deliveries run `messages::deliver` on a spawned task per window, one at a time per window.
4. `start` runs `git::preflight` and plan validation on `spawn_blocking`, picks the slug (decision 7), builds the run, and sends `Start`. Validation errors are joined with `\n`.
5. `finish` checks `confirm` against the run id for `merge`, `pr` and `discard`; without it, it returns `FinishReply::Confirm(prompt)`: for `merge`, `merge anthrex/<id>/integration into <base> in <root>?`; for `pr`, `push anthrex/<id>/integration to <remote> (<url>) and open a pull request into <base>?` after `pr_target` on `spawn_blocking`; for `discard`, `remove every worktree and branch of run <id>? Type the run id to confirm.`. A wrong `confirm` is an error `confirmation does not match the run id`.
6. `handle_client` handles each run request and `ToolCall` on a `tokio::spawn`ed task that sends the reply through a clone of `out_tx`, so a long git operation never stalls the connection. It forwards `RunsChanged` like `WindowsChanged`. `Welcome` carries `runs.infos()`.
7. `lifecycle::run` creates the `RunService` after the manager with `RunContext { worktrees_root: data_dir.join("worktrees"), exe: manager_config.exe.clone(), socket_path, orchestrator: config.orchestrator.clone() }` and calls `stop()` before `manager.shutdown()` (decision 48).

**Acceptance.** Tests pass. Search `driver.rs` for every `crate::lock(`: no guard is alive across `.await`, `spawn_blocking`, a git call or a manager call. Say in the pull request that you checked this.

### M8.15 The `anthrex run` commands

**Files.** Create `crates/cli/src/run_cmd.rs`. Modify `crates/cli/src/main.rs`, `crates/cli/src/client.rs`. Create `crates/cli/tests/run_cli.rs`.

**Tests first.**

- In `run_cmd.rs` unit tests: `resolve_run_by_id_suffix_and_prefix` (exact id wins; `3f9a` matches `add-reset-3f9a`; an ambiguous prefix errors listing both ids; no match errors `no run matches '<x>'`); `status_text_matches_the_layout` (a fixed `RunInfo` renders exactly the Interfaces example's lines, paused runs as `paused (from running)`).
- In `crates/cli/tests/run_cli.rs` with the harness: `start_approve_status_finish_keep` (`run start` without `--yes` prints the id and the approve hint; `run status <id> --json` parses as `Vec<RunInfo>` with state `awaiting_approval`; `run approve <id>`; wait for Ready; `run finish <id> keep`; state `finished`, outcome `kept`); `finish_discard_needs_the_id` (`run finish <id> discard` with stdin `wrong\n` fails with `confirmation does not match the run id`; with `--confirm <id>` it succeeds, the worktree directories and `anthrex/<id>/` branches are gone); `retry_and_skip_errors_are_printed` (retrying a Merged task prints the decision text and exits 1).

**Change.** Implement the CLI section. Exit status 1 with the daemon's message on every `Error`.

**Acceptance.** Tests pass. `anthrex run --help` lists the eight subcommands; `anthrex --help` does not list `mcp`.

### M8.16 Persistence, Paused and resume

**Files.** Modify `crates/daemon/src/state.rs`, `crates/daemon/src/lifecycle.rs`, `crates/daemon/src/run/driver.rs`. Add to `crates/cli/tests/run_e2e.rs`.

**Tests first.**

- In `state.rs` unit tests: `runs_round_trip_through_the_state_file` (a `StateFile` with one `Run` holding a task, a review record and an outbox message saves and loads equal); `a_bad_run_entry_is_skipped` (one valid run and one `{"id": 5}`: one run loads, one warning).
- In `crates/cli/tests/run_e2e.rs`: `daemon_restart_pauses_and_resume_continues`. Worker script 1: `SessionStart`, `UserPromptSubmit`, `git_commit`, `Stop`, `wait_ms 600000` (it never reports). Worker script 2 (claimed on resume): `SessionStart`, `read_message {"expect":"The daemon restarted"}`, `UserPromptSubmit`, `mcp_call report_done`, `Stop`, `wait_ms 600000`. Reviewer scripts approve. Start with `--yes`; wait until `t1` is Running with a worker window; stop the daemon with `anthrex daemon stop`; start it again. `RunsChanged` shows the run Paused with `paused_from == Running`, and the worker window Exited. `RunApprove` is refused with the paused message. `RunResume` gives `Ack`; the run reaches Ready within 60 s with `t1` Merged. `state.json` then lists the run with state `ready`.

**Change.** Implement decisions 46 to 49. `lifecycle::run` loads runs with the state file and calls `RunService::restore` before binding the socket, so the first `Welcome` lists them. `spawn_persister` saves on either watch and fills `runs` from `RunService::records()`.

**Acceptance.** Tests pass. A state file written by milestone 6 (with `"runs": []` and `"run": null`) loads with no warning.

### M8.17 Run rows in the tree

**Files.** Modify `crates/tui/src/tree.rs`, `crates/tui/src/app/mod.rs`, `crates/tui/src/tree_input.rs`, milestone 4's renderer `crates/tui/src/ui/tree_view.rs`, and `crates/cli/src/tree_cmd.rs`.

**Tests first.** Unit tests in `tree.rs` with a fixture `example_run()`: project `/tmp/shop` with plain window 1, and run `add-reset-3f9a` (goal `Add password reset`) with `t1` Merged (worker 2 exited), `t2` Reviewing (worker 3, reviewer 4), `t3` Planned, plus final reviewer none.

- `run_rows_sit_above_plain_windows`: the rows are project, run, `t1`, window 2, `t2`, window 3, window 4, `t3`, window 1, with indents 0, 2, 4, 6, 4, 6, 6, 4, 2.
- `run_row_text`: `◈ Add password reset · 1/3`.
- `task_row_shows_glyph_id_title_and_state`: `t2` has the spinner glyph and label `reviewing`; `t1` has `✓` and `merged`.
- `run_windows_are_not_repeated_as_plain_windows`, `windows_of_an_unknown_run_are_plain`.
- `collapsing_a_run_hides_tasks_and_windows`: `toggle(NodeKey::Run(..))` returns true.
- `agent_order_includes_run_windows_in_visible_order`: `[2, 3, 4, 1]`.
- `run_attention_bubbles_to_the_project`: `t2` NeedsHuman makes the run and the project rows attention.
- `finished_run_without_windows_is_hidden`.
- `enter_on_a_task_row_focuses_its_worker`: in `tree_input.rs` tests, Enter on `t2` focuses window 3; on `t3`, which has no window, nothing happens.
- Rendering test in `crates/tui/src/ui/mod.rs`'s tests, next to milestone 4's, with `TestBackend` at 34 columns: the run row and the task rows appear, the state label is right-aligned.
- In `crates/cli/tests/tree.rs` or its unit tests: `tree_json_has_runs` (each project object has a `runs` array with `run_id`, `goal`, `state`, `merged`, `total`, `tasks` of `{id, title, state, worker_window, reviewer_window}`).

**Change.** Add `NodeKey::Run(String)`, `NodeKey::Task { run_id: String, task_id: String }`, `ProjectChild::Run { .. }`, `RowKind::Run { .. }` and `RowKind::Task { .. }` per decisions 50 and 51. `build` takes `runs: &[RunInfo]` as a new argument; every caller passes `&app.runs`. `TreeState::prune` also drops keys of runs that no longer exist. Filtering matches run goals and task titles too.

**Acceptance.** Tests pass. Milestone 4's checks still hold: no `_ =>` arm on `NodeKey`, `RowKind` or `ProjectChild`, and no I/O in `tree.rs`, `tree_input.rs`, `app/` or `ui/`.

### M8.18 End-to-end scenarios and the smoke stage

**Files.** Modify `crates/cli/tests/run_e2e.rs`, `scripts/pty-smoke.py`.

**Tests first.** In `crates/cli/tests/run_e2e.rs`. Workers are Claude-runtime fake agents and reviewers Codex-runtime ones unless stated. Each test waits at most 90 s.

- `two_tasks_with_a_dependency_merge_in_order`: `t2` depends on `t1`. `t2`'s worker script runs `sh` `test -f a.txt && echo seen > b.txt && git add b.txt && git commit -m b` before `report_done`, so without `t1`'s work it has no commit and never passes the submission check. Assert `t1: merging -> merged` comes before `t2: planned -> ready` in `REPORT.md`'s log, `b.txt` is on the run branch, and the run branch has two merge commits in the order `t1`, `t2`.
- `review_requests_changes_then_approves`: `reviewer-t1-1` submits `changes` with an important finding `a.txt` `needs a newline`; the worker's script continues with `read_message {"expect":"requested changes"}`, `UserPromptSubmit`, `git_commit`, `report_done`, `Stop`; `reviewer-t1-2` approves. Assert `round == 2`, `rounds_used == 1`, Merged, and `REPORT.md` contains `needs a newline`.
- `merge_conflict_goes_back_to_the_worker`: `t1` and `t2` independent, `max_parallel` 2, both write different content to `same.txt`; `t2`'s worker waits 2 s before `report_done` so `t1` merges first. `t2`'s worker then runs `read_message {"expect":"conflicted in: same.txt"}`, `UserPromptSubmit`, and `sh` that extracts the branch from `$FAKE_AGENT_MESSAGE` with `grep -o 'anthrex/[a-z0-9-]*/integration'`, runs `git merge` of it, writes the resolved `same.txt`, `git add`, `git commit --no-edit`, then `report_done`, `Stop`. Reviewer scripts approve rounds 1 and 2 of `t2`. Assert `t2` Merged, the run branch's `same.txt` holds the resolved text, and `REPORT.md` logs `t2: merging -> conflict`.
- `verify_failure_sends_the_tail`: `verify` is `if [ -f fixed.txt ]; then echo ok; else echo VERIFY-MARKER missing; exit 1; fi`. The worker commits `a.txt`, reports, then `read_message {"expect":"VERIFY-MARKER"}`, commits `fixed.txt`, reports. Assert Merged, `rounds_used == 1`, two verify records, and the run-level verify passed.
- `max_review_rounds_reaches_needs_human`: config `[orchestrator] max_review_rounds = 2`; both reviewer rounds request changes. Assert NeedsHuman with note `review rounds exhausted (2)`, attention contains `t1 needs a human`, and the worker received exactly one findings message (its script's second `read_message` times out: give it `timeout_ms` 10000 and assert the window exited with code 4).
- `cancel_kills_windows_and_keeps_worktrees`: cancel while `t1` is Running; within 10 s its worker window is Exited, the run and task are Cancelled, and the task worktree still exists.
- `finish_discard_after_cancel`: `run finish <id> discard --confirm <id>` after the cancel removes worktrees and branches, and `main` is unchanged.

In `scripts/pty-smoke.py`, a new stage after milestone 6's stage 11 and before the final daemon stop, `== stage 12: a one-task run reaches merged ==`:

1. Milestone 3's `ensure_binary` already builds `target/debug/fake-agent` and sets `ANTHREX_CLAUDE_BIN` in `ENV`. Add `ANTHREX_CODEX_BIN` pointing to the same binary, `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1` to `ENV` at the top of the script, so the daemon started by the first stage already has them. The per-role scripts below take precedence over the script's `FAKE_AGENT_SCRIPT`.
2. Create `/tmp/anthrex-smoke-run-<pid>` (milestone 5's stages own `/tmp/anthrex-smoke-repo-<pid>`) with `git init -b main`, repo-local identity and `commit.gpgsign false`, one commit. Write the worker, reviewer and final reviewer scripts of `one_task_plan_runs_to_ready` into its `.git/fake-agent/`, and the plan to `/tmp/anthrex-smoke-plan-<pid>.json`.
3. `anthrex run start --plan <file> --dir <repo> --yes`; poll `anthrex run status <id> --json` every 0.5 s for up to 60 s until the state is `ready`. Fail with the last JSON on timeout.
4. Attach in a PTY and `wait_for("◈")`, then detach.
5. `anthrex run finish <id> merge --yes`; assert `git -C <repo> log -1 --format=%s` starts with `anthrex: merge run` and `a.txt` exists in the repo.
6. Remove the repo and plan in the script's `finally` block.

**Change.** Only tests and the smoke script, plus fixes the scenarios uncover.

**Acceptance.** All five commands from `AGENTS.md` pass.

## Verification

Run the five commands from `AGENTS.md`:

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

Milestone-specific checks:

- `grep -rn "PROTO_VERSION: u32 = 5" crates/proto/src/lib.rs` matches.
- `rg -n "std::fs|std::process|std::thread|tokio" crates/daemon/src/run/engine crates/daemon/src/run/plan.rs crates/daemon/src/run/roster.rs crates/daemon/src/run/model.rs crates/daemon/src/run/contract.rs crates/daemon/src/run/report.rs crates/daemon/src/run/role_launch.rs` prints nothing.
- `cargo tree -p anthrex-daemon | grep rmcp` and `cargo tree -p anthrex-tui | grep rmcp` print nothing; `cargo tree -p anthrex-mcp | grep "rmcp v3.4.0"` matches.
- `wc -l crates/daemon/src/run/*.rs crates/daemon/src/run/engine/*.rs`: no file above 600 lines.
- No `crate::lock` guard in `driver.rs` or `manager.rs` is alive across `.await`, `spawn_blocking`, a git call or `Window::spawn`.
- After the tests and the smoke script, `pgrep -fl "anthrex daemon"` and `pgrep -fl fake-agent` show nothing of yours, and `/tmp/ax-run*`, `/tmp/anthrex-smoke-repo-*` and `/tmp/anthrex-smoke-run-*` are gone.

## Manual check

Use an isolated daemon and config throughout, and a throwaway repository:

```bash
export ANTHREX_SOCKET=/tmp/anthrex-m8/daemon.sock ANTHREX_DATA_DIR=/tmp/anthrex-m8/data ANTHREX_CONFIG=/tmp/anthrex-m8/config.toml
mkdir -p /tmp/anthrex-m8 && cd /tmp/anthrex-m8 && git init -b main demo && cd demo && echo '# demo' > README.md && git add . && git commit -m init
```

1. Write `/tmp/anthrex-m8/plan.json` with two tasks: `t1` Codex worker (reviewer chosen by the engine), writing a small shell script `hello.sh` with a test; `t2` Claude worker `claude-sonnet-5`, depending on `t1`, adding a README section. Verify command `sh hello.sh | grep -q hello`.
2. `anthrex run start --plan /tmp/anthrex-m8/plan.json`. It prints the id and the approve hint. `anthrex run status` shows `awaiting approval`. `anthrex run approve <id>`.
3. `anthrex` to attach. The tree shows `◈ <goal> · 0/2` under the `demo` project, task rows, and the Codex worker window under `t1`. Focus the worker: its first prompt is the task text followed by the worker contract.
4. In the Codex worker, confirm the anthrex tools are listed (`/mcp` or by asking it) and that calling `get_task` does not ask for approval. Record whether `default_tools_approval_mode="auto"` suppressed the prompt on codex-cli 0.155.0.
5. Watch `t1` go through verifying and reviewing. The Claude reviewer runs in plan mode and cannot edit. It calls `submit_review` without an approval prompt (`--allowedTools mcp__anthrex__*`).
6. Make a review request changes: tell the reviewer in its window to request a change with an important finding. Confirm the worker receives the `[anthrex] Review round 1 requested changes` message only when it is idle, that it is submitted as one prompt (not left in the input box), and that it shows the pasted block normally in both Claude and Codex.
7. While `t2` runs, `anthrex daemon stop`, then `anthrex daemon start`. `anthrex run status` shows `paused (from running)`. `anthrex run resume <id>`: the Claude worker resumes its session and receives the restart message.
8. When the run is Ready, open `REPORT.md` from the status output and check it reads well. `anthrex run finish <id> merge`: the prompt names the base branch; answer `y`; `git log --oneline -3` on `main` shows the run merge.
9. Pull request path, only in a throwaway GitHub repository you own: repeat with a one-task plan, then `anthrex run finish <id> pr`. The prompt names the remote, its URL, the branch and the base. Answer `y`; the printed URL opens a pull request whose body is the report.
10. `anthrex run cancel` on a fresh run, then `anthrex run finish <id> discard` and type the id: its worktrees and `anthrex/<id>/` branches are gone.
11. `anthrex daemon stop`. `pgrep -fl "anthrex daemon"` shows nothing of yours.

## Risks and gotchas

1. **Variadic Claude flags.** `--mcp-config` and `--allowedTools` take several values in Claude Code 2.1.276. A value followed directly by the prompt would swallow the prompt as another config. The order in decision 36 and the test `claude_variadic_flags_are_followed_by_a_flag` prevent this. If Claude reports `Invalid MCP configuration` naming the prompt text, this is the cause.
2. **Codex approval mode.** If `default_tools_approval_mode="auto"` does not suppress the prompt in the installed Codex, every tool call waits for approval and shows as attention. Record it in "Implementation notes"; the user can approve once per session. Do not add undocumented keys.
3. **Paste then Enter.** An agent TUI may treat a carriage return that arrives in the same read as the paste end as part of the paste, leaving the text in the input box. `SUBMIT_DELAY` (200 ms) separates them. If step 6 of the manual check shows unsubmitted text, raise it to 500 ms and record it.
4. **Shutdown storm.** Killing windows at daemon shutdown produces an exit event per window. Without decision 48 every task would be NeedsHuman after a restart. The test `daemon_restart_pauses_and_resume_continues` catches this.
5. **Merge identity and signing.** Merge commits need `user.name` and `user.email`. Tests set them repo-locally and isolate global config with `GIT_CONFIG_GLOBAL=/dev/null`. A user with `commit.gpgsign = true` and an interactive pinentry can make a merge hang until the 60 s timeout; the task then fails with git's stderr. Do not override the user's signing settings.
6. **Deleting branches that are checked out.** `git branch -D` fails for a branch checked out in a worktree. Decision 16's discard order removes worktrees first. If branches survive a discard, check the order.
7. **Stdout in `anthrex mcp`.** Any byte on stdout that is not a JSON-RPC message breaks the MCP session. No `println!`, no tracing subscriber on stdout. `mcp_subcommand_speaks_json_rpc_on_stdout_only` guards this.
8. **Engine lock and I/O.** `Engine::handle` is pure and fast. Never call git, the manager, or `.await` while holding the engine lock; decision 2 exists to make this easy. A frozen `anthrex ls` during a run means this rule was broken.
9. **Sockets and long temp paths.** macOS limits socket paths to 104 bytes. Test temp dirs are under `/tmp`, never `std::env::temp_dir()`, which is a long `/var/folders/...` path.
10. **`fake-agent` not built.** `cargo test -p anthrex` alone does not build it; milestone 3's `support::fake_agent_bin()` then panics with the command to run. CI and the documented commands use `--workspace`.
11. **rmcp API.** The API in decision 5 was checked for 3.4.0 only. Do not loosen the `=3.4.0` pin in this milestone. If a type name differs, check the crate source under `~/.cargo/registry/src/*/rmcp-3.4.0/` rather than guessing.
12. **Claude MCP tool timeout.** Claude's own MCP tool-call timeout is not stated in the spec. The daemon answers every tool call quickly except `report_done`, which waits for at most two git commands. If a Claude worker reports a tool timeout, record it.
13. **Reviewer in plan mode.** Claude's plan mode may ask before running tests. That shows as attention on the reviewer window, which is acceptable in this milestone.
14. **Messages to exited windows.** A window restarted by the user while its run is Paused still receives the resume message only after `run resume`. Tool calls to a Paused run fail with the paused message by design.

## Follow-ups handled

None. The "Assignment to milestones" table in `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` assigns no item to milestone 8. New follow-up to record there during implementation: pruning old terminal run records from the state file.

## Implementation notes

The implementer fills this section in during the milestone.
