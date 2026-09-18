# Milestone 9: Orchestrator agent: planning, model routing, review loop, plan and finish views

## Header

| Field | Value |
|-------|-------|
| Status | `blocked` |
| Depends on | Milestone 8 (orchestration engine) |
| Spec sections | Product design `docs/superpowers/specs/2026-09-18-anthrex-product-design.md`: section 9 in full (9.1 to 9.7); the orchestrator column of the holds table in 8.1; the orchestrator rows of section 11.5; the version-6 row of section 10.1; the `O` key in 10.3. It also touches 8.3 (plan validation), 8.7 (finishing from a hold), 8.8 (the orchestrator row under a run) and 5.2 (Enter on a run row). |
| Branch | `m9-orchestrator-agent` |
| Protocol version | 6 (`proto::PROTO_VERSION` goes from 5 to 6). Rule (product spec 10.1): set it to one more than the value on `main` when you start; 6 assumes roadmap order. |

## Goal

The user starts a run by giving a goal and choosing which model orchestrates it: any Claude or Codex model from the roster. The orchestrator is a real, interactive `claude` or `codex` window in the project root. It reads the code, writes a plan, and routes each task to a Claude or Codex worker on a model suited to the task. The engine from milestone 8 runs every task in its own git worktree, has a different agent review it, sends findings back to the worker, and merges approved work into the run branch. The orchestrator decides every exception: questions, blocked tasks, exhausted review rounds, unresolved conflicts, verify failures and the final review. The user approves, edits or rejects the plan in a plan view, and chooses how to finish in a finish view. Nothing reaches the base branch until the user says so.

The user's own words for this milestone, and where each part is delivered:

| The user asked for | Delivered by |
|--------------------|--------------|
| "I can choose the Claude or Codex model to be the orchestrator" | Decisions 1 to 4; the `C-b O` form (M9.10) and `anthrex run start "<goal>" --orchestrator <runtime>[:<model>]` (M9.9); launch on both runtimes (M9.7) |
| "the orchestrator will dispatch different models depending on the task, from both Claude and Codex" | The roster and routing guidance (decisions 20 to 25, M9.3); per-task `runtime` and `model` in `submit_plan` and `add_tasks`; `reassign`; the role contract's routing section; user overrides in the plan view (M9.11) |
| "Take into account git worktrees" | Every task still runs in its own worktree from milestone 8. The orchestrator itself runs read-only in the project root and never edits code (decisions 8 and 9). |
| "reviewing code" | Cross-runtime review by default (decision 23), the review loop events, `approve_anyway`, and the final review hold (decisions 13 to 16) |
| "merging code between agents" | Dependent tasks start from a run branch that already contains their dependencies' merged work (spec 8.2 rule 4); conflicts go back to the worker and, if unresolved, to the orchestrator as `needs_human`; the finish view merges into base or opens a pull request (M9.12) |

## Scope

### Starting point

Milestones 3 to 8 are merged before this one starts. This brief uses the names of their briefs, above all `docs/milestones/M8-orchestration-engine.md`. If the merged code differs, use the real names and record the mapping under "Implementation notes". Milestone 8 provides:

- The run engine in `crates/daemon/src/run/`: the pure state machine `run::engine::Engine` (`handle(now, Input) -> Vec<Action>`, `deliverable`, in `engine/mod.rs`, `flow.rs`, `tools.rs` and `control.rs`), the persisted model `run::model::{Run, Task, RunLimits, ...}`, and the I/O driver `run::driver::RunService`, which executes actions, answers requests and owns the engine lock (`crate::lock`).
- The run git layout of spec 8.2 in `run/git.rs`, including `preflight` and `pr_target`, and the verify runner in `run/verify.rs`.
- Plan parsing and validation in `run/plan.rs` (`parse`, `validate` returning `Vec<PlanError { task, field, message }>`, `slug`, `build_run`). The run id is the slug, a `String`.
- The roster: `proto::Tier`, `proto::ModelEntry`, `config::Orchestrator { models, builtin_models, max_parallel, ... }` with the built-in roster and the user-roster rules of spec 9.5 (extend, replace in place, `builtin_models = false`, invalid entries skipped with problems), and `run/roster.rs` with `tier_of`, `pick_reviewer` and `pick_final_reviewer`. `max_parallel` is valid from 1 to 8.
- `RunLimits { max_parallel, max_tasks, max_review_rounds, max_windows, worker_permission_mode, worker_codex_sandbox }`, snapshotted into each run at start.
- Message delivery in `run/messages.rs` (`deliver`, `Gate`, `encode_paste`, `clamp`) through each run's outbox: queued, delivered only when the window is Idle or Done, bracketed paste then a carriage return 200 ms later, `[anthrex]` prefix, attention after 10 minutes.
- The MCP server crate `crates/mcp` (`tools_for(role)`, `forward`, `serve_on`, `serve_stdio`) behind the hidden `anthrex mcp --role <role> --run <run-id> [--task <task-id>] --window <window-id> --socket <socket-path>`. It opens a fresh socket connection per tool call and sends `ClientMsg::ToolCall { run_id, task_id, role, window_id, tool, args }`; the daemon answers `DaemonMsg::ToolResult { ok, text }`. The server handles each `ToolCall` in its own spawned task, so calls already run concurrently. Tools outside the role's list return `unknown tool <name>` without reaching the daemon. Authorization errors are milestone 8's decision-25 texts.
- Role launch in `run/role_launch.rs`: `RoleLaunch { run_ref, instructions, claude_permission_mode, codex_sandbox, codex_approval }`, `mcp_args`, `claude_role_args`, `codex_role_args`, inserted by `crates/daemon/src/launch/` after milestone 3's flags. TOML strings come from milestone 3's `launch::codex::toml_string`. `WindowManager::create_with_role(spec, project, role, cols, rows)`.
- The contracts and message texts in `run/contract.rs`, and the run report `REPORT.md` in `run/report.rs`.
- `proto::run`: `Role { Orchestrator, Worker, Reviewer }`, `RunRef`, `TaskKind`, `ModelRef`, `PlanTask`, `Plan`, `RunState`, `TaskState`, `Severity`, `Finding`, `Verdict`, `FinishAction`, `TaskInfo`, `RunInfo`. `ClientMsg::{RunStart, RunApprove, RunRetry, RunSkip, RunFinish { run_id, action, confirm }, RunCancel, RunResume, ToolCall}`, `DaemonMsg::{RunsChanged, RunCreated, RunConfirmNeeded, RunFinished, ToolResult}`, `Welcome.runs`, protocol version 5.
- `anthrex run start --plan|approve|status|retry|skip|finish|cancel|resume` in `crates/cli/src/run_cmd.rs`.
- Run rows in `crates/tui/src/tree.rs` (`NodeKey::Run`, `NodeKey::Task`, `RowKind::Run`, `RowKind::Task`), and run persistence in state file version 2 with Paused runs on daemon restart.
- `fake-agent` per-role scripts `<git common dir>/fake-agent/<role>-<task>-<n>.jsonl` (`<task>` is `run` without `--task`, so the orchestrator's scripts are `orchestrator-run-<n>.jsonl`), and the steps `mcp_call`, `read_message` and `sh`. The end-to-end harness `crates/cli/tests/support/run_harness.rs` (`RunHarness`) on top of milestone 3's `crates/cli/tests/support/mod.rs` (`fake_agent_bin()`, `TestDaemon`).

From milestones 3 to 6: exact status from hooks, `WindowInfo.session_id` and `model`, `WindowInfo.project`, the project tree with tree mode (`crates/tui/src/tree_input.rs`), milestone 5's pure form model in `crates/tui/src/dialog.rs` (`TextInput`, `NewAgentForm`, `FormOutcome`), `config.toml` loading, `anthrex restart`, and the client state in `crates/tui/src/app/mod.rs` with tests in `crates/tui/src/app/tests.rs`. With milestone 7, the main area holds panes.

Existing milestone-1 names this brief also uses, which are real today: `proto::PROTO_VERSION`, `proto::ClientMsg`, `proto::DaemonMsg`, `proto::Runtime`, `proto::Status`, `proto::WindowSpec`, `daemon::launch::{plan, LaunchPlan, LaunchContext}`, `daemon::lock`, `tui::app::{App, Effect, Modal, PendingAction}`, `tui::keymap::{Command, Keymap}`, `tui::ui::modal::HELP`, `scripts/pty-smoke.py`.

### In

- The orchestrator role in the engine: which decisions move from the human CLI to the orchestrator, the per-run event queue, `wait_for_events` long-poll, the waking rule.
- The twelve orchestrator MCP tools of spec 9.3 with JSON Schemas and error cases.
- Launching the orchestrator window on Claude and on Codex, the role contract text, the first prompt.
- The roster: choosing the orchestrator from milestone 8's roster, and enforcing the run's allowed runtimes for workers and reviewers. Milestone 8 already parses the roster, including user entries and `builtin_models`.
- Starting an orchestrated run from the `C-b O` form and from `anthrex run start "<goal>" --orchestrator ...`.
- The plan view and the finish view in the main area.
- The limits of spec 9.7 applied to orchestrated runs.
- New CLI commands `anthrex run reject` and `anthrex run tell`.
- `fake-agent` extensions for scripted orchestrators, and end-to-end tests on both runtimes.

### Out

- Cost budgets and token accounting (spec 9.7: anthrex cannot see token spend).
- Editing a task's reviewer, acceptance criteria or dependencies from the plan view. Only runtime, model and prompt are editable (spec 9.6).
- Rebasing, squash merges, or any change to milestone 8's merge semantics.
- Changing which models Codex has. anthrex never guesses Codex model names (spec 9.5).
- Saving plan view layouts, and more than one orchestrator per run.

## Design decisions

### Choosing and launching the orchestrator

1. **Orchestrator choice.** A run's orchestrator is one roster entry `(runtime, model)`. It must be in the roster. `--orchestrator claude:claude-opus-5` picks that exact entry. `--orchestrator claude` or `--orchestrator codex` with no model picks that runtime's entry with the highest tier, first in roster order at that tier. With the default roster, `codex` resolves to the entry with `model = ""`, which launches Codex without `-m`. `codex:` with an empty model means the same entry.
2. **The orchestrator window** is created by the engine in the project root, the run's `root` (milestone 8 decision 8), through `WindowManager::create_with_role` with `RunRef { role: Orchestrator, task_id: None }`. Its name is `orch-<slug>`, cut to 32 characters. It counts toward `orchestrator.max_windows`.
3. **The orchestrator runtime is independent of the worker runtimes.** A Codex orchestrator may dispatch only Claude workers and the reverse.
4. **Workers allowed.** A run records `allowed_runtimes`, a non-empty subset of `{claude, codex}`, default both. Every allowed runtime must have at least one roster entry, or the start is refused with `no roster entry for codex`.
5. **Claude orchestrator launch** (Claude Code 2.1.x, flags from spec 8.5 and 9.1, re-verified in M9.1). Milestone 8's `claude_role_args` with role Orchestrator, at milestone 8's position (after milestone 3's `--name` and `--settings`, before `--model`, `--resume` and `--`), in this order: `--mcp-config <json>` with `{"mcpServers":{"anthrex":{"type":"stdio","command":"<exe>","args":["mcp","--role","orchestrator","--run","<run-id>","--window","<window-id>","--socket","<socket-path>"]}}}` (milestone 8's `mcp_args`, no `--task`), `--allowedTools mcp__anthrex__*,Read,Grep,Glob`, `--disallowedTools Edit,Write,NotebookEdit`, `--append-system-prompt <ORCHESTRATOR_CONTRACT>`, and no `--permission-mode`. Then milestone 3's `--model <model>` when the model is not empty, then `--` and the first prompt. Each is its own argv element; no shell quoting is involved. `RoleLaunch` gains `claude_allowed_tools: Vec<String>` (extra entries after `mcp__anthrex__*`, joined with `,`) and `claude_disallowed_tools: Vec<String>`, both `#[serde(default)]` and empty for workers and reviewers.
6. **Codex orchestrator launch** (Codex 0.135.0 per spec 11.5, re-verified in M9.1). Milestone 8's `codex_role_args` with role Orchestrator, after the spec 11.2 base flags and milestone 3's hook flags: `-c mcp_servers.anthrex.command="<exe>"`, `-c mcp_servers.anthrex.args=["mcp","--role","orchestrator","--run","<run-id>","--window","<window-id>","--socket","<socket-path>"]`, `-c mcp_servers.anthrex.tool_timeout_sec=120`, `-c mcp_servers.anthrex.default_tools_approval_mode="auto"`, `-c developer_instructions="<ORCHESTRATOR_CONTRACT escaped as a TOML basic string>"`, `-s read-only`, `-a on-request` (`RoleLaunch.codex_sandbox = Some("read-only")`, `codex_approval = Some("on-request")`), `-m <model>` only when the model is not empty, then `--` and the first prompt. Every TOML string comes from milestone 3's `launch::codex::toml_string`; do not add another helper.
7. **First prompt** of the orchestrator window, exactly:

   ```
   Goal: <goal>

   You are orchestrating anthrex run <slug> in <project root>. Call get_run and list_models, explore the repository, then call submit_plan.
   ```
8. **The orchestrator never edits code.** Claude enforces it with `--disallowedTools`, Codex with `-s read-only`. The engine adds no other check: the orchestrator's only write path is its tools.
9. **The orchestrator stays in the project root**, not in a worktree, so it sees the base branch as it was at run start. The contract tells it to read task work through `get_run`, not through the checkout.

### Decisions that move from the human CLI to the orchestrator

10. In an orchestrated run the engine turns every exception into an event for the orchestrator instead of waiting only for a human command:

    | Situation | Milestone 8 (plan file, human) | Milestone 9 (orchestrated run) |
    |-----------|--------------------------------|--------------------------------|
    | Plan | `--plan <file>` | Orchestrator `submit_plan`; the user approves, edits or rejects |
    | Plan approval | `anthrex run approve` | Unchanged: always the user, unless `auto_approve` or `--yes` |
    | Worker `ask` | Attention on the run | `question` event; orchestrator `answer` |
    | Blocked task | `run retry` / `run skip` | `blocked` event; `retry`, `reassign`, `cancel_task` or `add_tasks` |
    | Review rounds exhausted | NeedsHuman; `run retry` / `run skip` | `needs_human` event with `why = review_rounds_exhausted`; `approve_anyway`, `retry` or `cancel_task` |
    | Conflict the worker could not resolve, worker window gone, dependency cancelled, window limit | NeedsHuman | `needs_human` event; `retry`, `reassign` or `cancel_task` |
    | Run-branch verify failure | Milestone 8's behaviour | `verify { scope: run }` event; the run holds (decision 14) |
    | Final review | Automatic move to Ready | `final_review` event; the run holds (decision 14) |
    | Ready | Automatic | Only through `finish_run { summary }`, or the user's override (decision 16) |
    | Finish: merge, pr, keep, discard | The user | Unchanged: always the user |

11. **Human commands keep working on orchestrated runs.** `anthrex run retry`, `skip`, `cancel` and typing into any window work as in milestone 8. When the user acts on an orchestrated run through a run command or the plan view, the engine queues a `user` event that describes the action, for example `The user ran retry on t3.` or `The user edited the plan before approving: t2 model claude-sonnet-5 -> claude-opus-5; dropped t4.`
12. **Events exist only for runs with an orchestrator.** Plan-file runs from milestone 8 behave exactly as before; every change in this brief is conditional on `run.orchestrator.is_some()`.
13. **Plan rejection.** `RejectPlan { comment }` is valid only in AwaitingApproval. The comment is 1 to 2000 characters. The run returns to Planning, keeps the rejected plan so `get_run` shows it, and queues `plan_rejected { comment }`.
14. **Holds.** This is the orchestrator column of product spec 8.1's holds table; runs without an orchestrator keep milestone 8's decision 30 unchanged. In an orchestrated run the engine does not leave Integrating or FinalReview on its own when a decision is needed. If the run-branch verify fails, the run stays in Integrating with `awaiting_orchestrator = true` and queues `verify { scope: run }`. When the final reviewer submits, the run stays in FinalReview with `awaiting_orchestrator = true`, whatever the verdict, and queues `final_review`. From a hold, `add_tasks` moves the run back to Running and `finish_run` moves it to Ready. Milestone 8's `anthrex run retry <run> integration` and `anthrex run approve <run>` still work in the Integrating hold, as in milestone 8, and each queues a `user` event. A passing run-branch verify still moves to FinalReview automatically, as in milestone 8.
15. **Repeated final reviews.** After tasks added from a hold are merged, the run goes through Integrating and FinalReview again with a new final reviewer. Every final review counts toward `max_windows`; there is no separate limit.
16. **User override of Ready.** `anthrex run finish <run> <action>` and the finish view also accept a run that is holding (decision 14). The engine records `Finished by the user without an orchestrator summary.` in the report. This keeps a run finishable when the orchestrator is stuck.
17. **`finish_run` preconditions.** The run is holding (decision 14); every task is Merged or Cancelled. Otherwise it returns `wrong_state` and lists the tasks that are not finished. The summary is 1 to 8000 characters and is stored in the run, shown in the finish view, and written at the top of `REPORT.md` under `## Summary from the orchestrator`.
18. **Retry after exhausted rounds** resets the task's `rounds_used` to 0, as milestone 8's retry does. The next worker's first prompt includes the previous rounds' findings and the note, under the heading `Note from the orchestrator:`.
19. **Orchestrator window exit.** If the orchestrator window exits while the run is not terminal, the run row shows attention with `orchestrator exited`, the engine keeps doing mechanical work, and events keep queuing. `anthrex restart <window>` from milestone 6 relaunches it with its session resumed and the same role flags. After a restart, and after `anthrex run resume`, the engine queues `user { text: "anthrex restarted your window. Call get_run to re-read the run before continuing." }`.

### Roster and routing

20. **Roster config** is milestone 8's: `config::Orchestrator.models` holds the merged roster of `proto::ModelEntry { runtime, model, tier, strengths }`, and the entry rules of spec 9.5 (empty model only for Codex, at most 8 strengths of at most 40 characters) are enforced there.
21. **User entries extend the built-in roster**, as milestone 8 implements: a user entry with the same `(runtime, model)` replaces a built-in in place, others are appended, and `orchestrator.builtin_models = false` drops the built-ins. This milestone adds nothing to the parsing.
22. **Invalid entries** are skipped by milestone 8's parser with a config problem naming `orchestrator.models[<index>]`; an empty result falls back to the built-in roster.
23. **Default reviewer** is milestone 8's `pick_reviewer`, with a new `allowed: &[Runtime]` parameter that skips entries on runtimes the run does not allow. With both runtimes allowed it behaves exactly as in milestone 8. When the other runtime is not allowed, milestone 8's rule (b) applies: the same runtime, a different model, at the author's tier or higher; then (d), the author itself. `pick_final_reviewer` gets the same parameter. Milestone 8's callers pass both runtimes. A task whose `model` is null counts as the tier of that runtime's roster entry with the empty model, or `standard` if there is none (milestone 8 decision 20).
24. **The engine enforces only** roster membership and the run's allowed runtimes, for workers and reviewers, on `submit_plan`, `add_tasks`, `reassign` and plan edits. The routing guidance is advice in the contract.
25. **`list_models`** returns every roster entry with an extra `allowed` boolean, true when the entry's runtime is in the run's allowed runtimes.

### Events and `wait_for_events`

26. **Event queue.** Each orchestrated run has one FIFO queue of events, each with a run-wide sequence number `seq` starting at 1. The queue holds at most 1000 events. When full, the oldest event is dropped and the run's `dropped` counter grows; the next `wait_for_events` result reports it and resets it. The queue and the counter are saved with the run in the state file.
27. **Long-poll.** `wait_for_events { timeout_secs }` returns at once with up to 50 queued events, oldest first, when any are queued. Otherwise it waits until the first event arrives, then 250 ms more to batch a burst, or until `timeout_secs` elapses and returns an empty list. `timeout_secs` above 50 is clamped to 50; 0 polls without waiting.
28. **At-most-once, with this ack rule.** The waiter removes the batch from the queue and hands the `ToolResult` frame to its connection's outbound channel. If the hand-over fails because the connection is gone, the batch goes back to the front of the queue in its original order. Once the hand-over succeeds, the events count as delivered and are never sent again. If the MCP client cancels the call before a batch was taken, nothing is removed.
29. **One open poll per run.** A new `wait_for_events` while another is open for the same run makes the older one return at once with no events and `superseded: true`.
30. **Concurrent tool calls.** A long-poll must not stall other tool calls. Milestone 8 already opens one socket connection per call and handles each `ToolCall` in its own task. This milestone adds `call_id: u64` to `ToolCall` and `ToolResult`: `anthrex mcp` numbers its calls from 1, and the reply carries the same `call_id`. A new `ClientMsg::ToolCancel { call_id }` cancels an open call; `anthrex mcp` sends it on that call's connection when the MCP client sends `notifications/cancelled`. A connection that closes before its reply cancels its open call too.
31. **Event payloads**: exactly spec 9.3's list, as JSON objects with `seq` and `type`: `plan_rejected { comment }`, `task_state { task_id, from, to }`, `review { task_id, round, verdict, findings }`, `verify { scope: "task" | "run", task_id?, ok, tail }`, `question { task_id, question_id, text }`, `blocked { task_id, reason }`, `needs_human { task_id, why }`, `window_exited { task_id, role, code }`, `final_review { verdict, findings }`, `user { text }`. `tail` is the last 40 lines of the verify output, at most 4000 characters. `why` is one of `review_rounds_exhausted`, `worker_gone`, `dependency_cancelled`, `window_limit`, or milestone 8's other reasons in snake case. `window_exited` is queued for worker and reviewer windows only.

### Waking the orchestrator

32. **Waking rule.** The engine sends the orchestrator window `[anthrex] <n> events waiting; call wait_for_events.` through milestone 8's message delivery when all of these hold: the orchestrator window's status is Idle or Done; at least one event is queued; no `wait_for_events` call is open for the run; and no nudge was sent in the last 60 seconds. For `n = 1` the text reads `1 event waiting`. The rule is a pure function, checked on every status change of the orchestrator window and on a 5-second tick.
33. **The nudge interval** is 60 seconds. For tests only, the environment variable `ANTHREX_NUDGE_INTERVAL_MS`, read once at daemon start, overrides it.

### Limits

34. **Limits are snapshotted** into the run at start from `[orchestrator]` (spec 9.7), so editing the config never changes a running run. The form's and CLI's `max_parallel` overrides the config for that run; allowed values 1 to 8.
35. **Enforcement and messages.** `max_tasks`: `submit_plan` and `add_tasks` fail with `limit_reached` and `max_tasks is 12 for this run; the plan would have 13 tasks`. `max_review_rounds`: milestone 8's move to NeedsHuman, now also a `needs_human` event. `max_windows`: when a dispatch or review would create a window beyond the limit, the task goes to NeedsHuman with `why = window_limit`, and the event's text includes `max_windows is 20 for this run; 20 windows were created`. `max_parallel`: dispatch as in milestone 8. `auto_approve`: a valid `submit_plan` moves straight to Running.
36. **Size limits** checked by plan validation for orchestrated runs: `title` at most 80 characters, `prompt` at most 16000, at most 12 acceptance criteria of at most 300 characters each, `notes` at most 8000. Errors name the task and the field, like milestone 8's.

### Plan input rules

37. **`submit_plan`** is valid only in Planning. The plan has spec 8.3's shape; `goal` is optional and ignored, since the run keeps the user's goal: the engine sets it to the run's goal before milestone 8's `plan::parse` and `plan::validate`. If the user set a verify command, the plan's `verify` is ignored and the result says so in `notes`; if the user set none, the plan's `verify` is used.
38. **Validation errors are tool results**, not protocol errors: the call returns `isError: true` with `{"accepted": false, "errors": [{"task": "t2", "field": "depends_on", "message": "unknown task 't9'"}]}`, the fields of milestone 8's `PlanError` (`task` is null for run-level errors). The model reads them and calls `submit_plan` again. Milestone 8's `plan::validate` gains an `allowed: &[Runtime]` parameter for the run's allowed runtimes, with the error `runtime: codex is not allowed in this run`; milestone 8's callers pass both runtimes.
39. **`add_tasks`** is valid in Running and in either hold. New tasks are validated against the current plan: unique ids, dependencies on existing or new tasks, no cycles, `max_tasks` counting every task ever in the plan including Cancelled ones.

### User plan edits

40. **`EditPlanTask`** and **`DropPlanTask`** are valid only in AwaitingApproval. An edit may change `runtime`, `model` and `prompt`; the result is validated like `submit_plan`. A drop fails with `t3 depends on t4` when another task depends on it. Edits are summarised in one `user` event queued when the user approves (decision 11).

### Starting a run

41. **The default verify command** for a project is the `verify` of the most recently created run, of any state, whose project root equals this one and that is still in the state file. None if there is none. `--verify ""` or an empty form field means "no verify command".
42. **CLI.** `anthrex run start "<goal>" --orchestrator <runtime>[:<model>] [--workers claude,codex] [--verify <cmd>] [--parallel <n>] [--yes]`. Exactly one of the positional goal and `--plan` is required. `--orchestrator` is required with a goal and refused with `--plan`. On success it prints `run <slug> started; orchestrator is window <id>` and exits 0.

### Client views

43. **The `C-b O` form** has six fields in this order: goal (text, required, 1 to 2000 characters, `Alt+Enter` inserts a newline), directory (text, default the tree's selected project root if any, else `App.default_dir`), orchestrator (selector over roster entries), workers (selector: `claude + codex`, `claude`, `codex`), verify (text, default decision 41), max parallel (number 1 to 8, default the config's `max_parallel`). Tab and Shift-Tab move, Left and Right change a selector or number, Enter submits, Esc cancels, as in milestone 5's dialog. The orchestrator default is the first `frontier` entry in the roster, else the first entry.
44. **Opening the views.** In tree mode, `Enter` or a mouse click on a run row in AwaitingApproval or Planning opens the plan view; on a run in Ready or holding (decision 14) it opens the finish view; on any other run it toggles collapse as spec 5.2 says. Opening a view leaves tree mode so plain keys reach the view. `Esc` returns the main area to the terminal or panes.
45. **Plan view keys** (spec 9.6): `j`/`k` or arrows select a task; `a` approve; `e` edit the selected task; `d` drop the selected task after a `y/n` confirmation; `r` reject with a comment; `Esc` back. In Planning the view shows `The orchestrator is planning.` and `Enter` focuses the orchestrator window.
46. **Finish view keys**: `m` merge into base, `p` open a pull request, `k` keep, `d` discard, `Esc` back. Each opens a confirmation. `p` shows the exact remote, its URL and the branch before anything is pushed. `d` requires typing the run's slug. The view asks the daemon for a `FinishPreview` when it opens, and the preview and the real action call the same daemon function, so what the dialog shows is what runs.

## Interfaces

### Protocol version 6 (`crates/proto`)

Add to `crates/proto/src/run.rs`, milestone 8's run-types module. All types derive `Debug, Clone, PartialEq, Serialize, Deserialize`; enums also `Copy, Eq` where they have no data. `Tier` and `ModelEntry` already exist there (milestone 8).

```rust
pub const PROTO_VERSION: u32 = 6;

pub struct OrchestratorChoice {                       // new
    pub runtime: Runtime,
    pub model: Option<String>,                        // None = decision 1's resolution
}

pub struct OrchestratedRunSpec {                      // new
    pub goal: String,
    pub dir: PathBuf,
    pub orchestrator: OrchestratorChoice,
    pub workers: Vec<Runtime>,                        // non-empty subset of {Claude, Codex}
    pub verify: Option<String>,                       // None = project default; Some("") = none
    pub max_parallel: Option<u8>,                     // None = config; 1..=8
    pub auto_approve: bool,                           // --yes
}

pub struct OrchestratorInfo {                         // new; RunInfo.orchestrator
    pub runtime: Runtime,
    pub model: String,
    pub window_id: Option<u32>,
    pub pending_events: u32,
    pub polling: bool,                                // a wait_for_events call is open
    pub awaiting: bool,                               // decision 14 hold
    pub summary: Option<String>,                      // from finish_run
}

pub struct RunStats {                                 // new; RunInfo.stats
    pub merged: u16,
    pub cancelled: u16,
    pub approved_anyway: u16,
    pub review_rounds: u16,
    pub findings_critical: u16,
    pub findings_important: u16,
    pub findings_minor: u16,
    pub run_verify_ok: Option<bool>,                  // None = no verify or not run yet
}

pub struct FinishPreview {                            // new
    pub root: PathBuf,
    pub base_branch: String,
    pub run_branch: String,
    pub base_checked_out: bool,
    pub root_clean: bool,
    pub remote: Option<String>,
    pub remote_url: Option<String>,
    pub gh_available: bool,
    pub report_path: PathBuf,
}

pub struct RunDefaults {                              // new; the reply to GetRunDefaults
    pub project: PathBuf,
    pub roster: Vec<ModelEntry>,
    pub default_orchestrator: usize,                  // index into roster
    pub last_verify: Option<String>,
    pub max_parallel: u8,
}

#[serde(rename_all = "lowercase")]
pub enum VerifyScope { Task, Run }                    // new
```

`RunInfo` (milestone 8) gains `orchestrator: Option<OrchestratorInfo>`, `allowed_runtimes: Vec<Runtime>` and `stats: RunStats`, all `#[serde(default)]`. Its `run_id` is the slug. `TaskInfo` (milestone 8, which already has `title`, `runtime`, `model`, `reviewer: ModelRef` and `depends_on`) gains `prompt: String`, `acceptance: Vec<String>`, `approved_anyway: Option<String>` and `open_questions: u16`. `Role::Orchestrator` exists since milestone 8.

Changed `ClientMsg::ToolCall` and `DaemonMsg::ToolResult` (milestone 8): both gain `call_id: u64` (decision 30).

New `ClientMsg` variants. Error `request` values are new constants in `proto::messages::request`, in milestone 8's style:

```rust
StartOrchestratedRun { spec: OrchestratedRunSpec },                       // reply RunStarted | Error{request: RUN_START = "run start"}
GetRunDefaults { dir: PathBuf },                                          // reply RunDefaults | Error{request: RUN_DEFAULTS = "run defaults"}
EditPlanTask { run_id: String, task_id: String, runtime: Runtime, model: Option<String>, prompt: String }, // Ack | Error{request: RUN_EDIT = "run edit"}
DropPlanTask { run_id: String, task_id: String },                         // Ack | Error{request: RUN_DROP = "run drop"}
RejectPlan { run_id: String, comment: String },                           // Ack | Error{request: RUN_REJECT = "run reject"}
TellOrchestrator { run_id: String, text: String },                        // Ack | Error{request: RUN_TELL = "run tell"}
GetFinishPreview { run_id: String },                                      // reply FinishPreview | Error{request: RUN_PREVIEW = "run preview"}
ToolCancel { call_id: u64 },                                              // no reply
```

New `DaemonMsg` variants:

```rust
RunStarted { run_id: String, orchestrator_window: u32 },
RunDefaults { defaults: RunDefaults },
FinishPreview { run_id: String, preview: FinishPreview },
```

### Daemon (`crates/daemon/src/run/orchestrator/`, new module)

```rust
// events.rs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OrchestratorEvent {
    PlanRejected { comment: String },
    TaskState { task_id: String, from: TaskState, to: TaskState },
    Review { task_id: String, round: u32, verdict: Verdict, findings: Vec<Finding> },
    Verify { scope: VerifyScope, task_id: Option<String>, ok: bool, tail: String },
    Question { task_id: String, question_id: u32, text: String },
    Blocked { task_id: String, reason: String },
    NeedsHuman { task_id: String, why: String },
    WindowExited { task_id: String, role: Role, code: Option<i32> },
    FinalReview { verdict: Verdict, findings: Vec<Finding> },
    User { text: String },
}
pub struct EventQueue { /* VecDeque<(u64, OrchestratorEvent)>, next_seq, dropped */ }
impl EventQueue {
    pub const CAP: usize = 1000;
    pub const BATCH: usize = 50;
    pub fn push(&mut self, e: OrchestratorEvent) -> u64;
    pub fn len(&self) -> usize;
    pub fn take_batch(&mut self) -> EventBatch;        // up to BATCH, oldest first; takes `dropped`
    pub fn restore(&mut self, batch: EventBatch);      // puts it back at the front, same order
}
pub struct EventBatch { pub events: Vec<(u64, OrchestratorEvent)>, pub dropped: u32 }

// nudge.rs
pub fn nudge_due(status: Status, queued: usize, polling: bool,
                 last_nudge: Option<Instant>, now: Instant, interval: Duration) -> bool;
pub fn nudge_text(n: usize) -> String;

// contract.rs
pub const ORCHESTRATOR_CONTRACT: &str = "...";     // the text in section "Role contract" below
pub fn first_prompt(goal: &str, slug: &str, root: &Path) -> String;

// tools.rs
pub struct ToolDef { pub name: &'static str, pub description: &'static str, pub input_schema: serde_json::Value }
pub fn tool_defs() -> Vec<ToolDef>;                 // exactly 12, in spec 9.3 order
pub enum OrchestratorCall {
    GetRun, ListModels,
    SubmitPlan { plan: serde_json::Value },
    AddTasks { tasks: Vec<serde_json::Value> },
    Reassign { task_id: String, runtime: Runtime, model: Option<String> },
    Retry { task_id: String, note: Option<String> },
    CancelTask { task_id: String, reason: String },
    ApproveAnyway { task_id: String, reason: String },
    Answer { task_id: String, text: String },
    Send { task_id: String, text: String },
    WaitForEvents { timeout_secs: u64 },
    FinishRun { summary: String },
}
pub fn parse_call(tool: &str, args: serde_json::Value) -> Result<OrchestratorCall, ToolError>;
pub struct ToolError { pub code: ToolErrorCode, pub message: String }
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCode { InvalidArguments, UnknownTool, UnknownTask, WrongState, NotInRoster,
                         RuntimeNotAllowed, LimitReached, NoOpenQuestion, NoWorkerWindow }

// crates/daemon/src/run/roster.rs (milestone 8's module), additions:
pub fn resolve_orchestrator(choice: &OrchestratorChoice, roster: &[ModelEntry]) -> Result<ModelEntry, String>; // decision 1
pub fn pick_reviewer(roster: &[ModelEntry], author: &ModelRef, allowed: &[Runtime]) -> ModelRef;  // decision 23, new parameter
pub fn pick_final_reviewer(roster: &[ModelEntry], allowed: &[Runtime]) -> ModelRef;               // new parameter
// Roster membership stays milestone 8's `tier_of(roster, m).is_some()`.

// crates/daemon/src/run/model.rs (milestone 8), additions to `Run`, all #[serde(default)]:
//   orchestrator: Option<OrchestratorRecord { runtime, model, window_id }>, allowed_runtimes: Vec<Runtime>,
//   awaiting_orchestrator: bool, summary: Option<String>, events: EventQueue, last_nudge: Option<u64>.
// crates/daemon/src/run/engine (milestone 8): the engine pushes each OrchestratorEvent into `Run.events`
//   itself and emits the new `Action::EventsQueued { run_id }`, which `RunService` turns into a wake-up of
//   that run's `tokio::sync::Notify`.
```

`Role`, `TaskState`, `Verdict` and `Finding` are milestone 8's types; `VerifyScope` is new in this milestone (Protocol section).

### Orchestrator tools and schemas

Every tool returns one text content item holding JSON. Success: `isError: false`. Failure: `isError: true` with `{"error": {"code": "<ToolErrorCode>", "message": "..."}}`, except `submit_plan` and `add_tasks` validation failures, which return `{"accepted": false, "errors": [{task, field, message}]}` (decision 38). Unknown arguments are `invalid_arguments` (`deny_unknown_fields`). Authorization comes first and keeps milestone 8's decision-25 texts as plain tool errors: `anthrex mcp` refuses a tool outside its role's list with `unknown tool <name>`, and the daemon refuses a call from a window that is not the run's orchestrator window with `this window is not the orchestrator of run <id>`.

| Tool | Input schema (`type: object`, `additionalProperties: false`) | Result on success | Errors besides `invalid_arguments` |
|------|----------------------------------------|-------------------|--------|
| `get_run` | `{}` | The run: `run_id` (the slug), `goal, state, awaiting_orchestrator, base_branch, run_branch, verify, allowed_runtimes, limits, windows_created, summary`, and `tasks[]` with `id, title, kind, state, runtime, model, reviewer, depends_on, acceptance, prompt, rounds, findings[], worker_window, reviewer_window, open_questions[{question_id, text}], blocked_reason, needs_human_why, approved_anyway, merge_commit`, and `final_reviews[]` | none |
| `list_models` | `{}` | `{"models": [{runtime, model, tier, strengths, allowed}]}` | none |
| `submit_plan` | `{plan: object}` required; `plan` has `goal?: string` (ignored), `notes?: string, verify?: string, tasks: array (minItems 1)` of task objects with spec 8.3's fields | `{"accepted": true, "state": "awaiting_approval" | "running", "tasks": n, "notes": [..]}` | `wrong_state` (not Planning); validation (decision 38) |
| `add_tasks` | `{tasks: array (minItems 1) of task objects}` | `{"accepted": true, "added": [ids], "state": "running"}` | `wrong_state`; validation |
| `reassign` | `{task_id: string, runtime: "claude"\|"codex", model: string\|null}` all required | `{"task_id", "runtime", "model"}` | `unknown_task`; `wrong_state` for Running, Submitted, Verifying, Reviewing, ChangesRequested, Approved, Merging, Conflict, Merged, Cancelled; `not_in_roster`; `runtime_not_allowed` |
| `retry` | `{task_id: string, note?: string (maxLength 4000)}` | `{"task_id", "state": "ready"}` | `unknown_task`; `wrong_state` unless Blocked or NeedsHuman |
| `cancel_task` | `{task_id: string, reason: string (minLength 1)}` | `{"task_id", "state": "cancelled", "now_needs_human": [ids]}` | `unknown_task`; `wrong_state` if Merged or Cancelled |
| `approve_anyway` | `{task_id: string, reason: string (minLength 1)}` | `{"task_id", "state": "approved"}` | `unknown_task`; `wrong_state` unless NeedsHuman with `why = review_rounds_exhausted` |
| `answer` | `{task_id: string, text: string (minLength 1, maxLength 8000)}` | `{"task_id", "question_id", "delivered": "queued"}` | `unknown_task`; `no_open_question`; `no_worker_window` |
| `send` | `{task_id: string, text: string (minLength 1, maxLength 8000)}` | `{"task_id", "delivered": "queued"}` | `unknown_task`; `no_worker_window` |
| `wait_for_events` | `{timeout_secs: integer (minimum 0, maximum 50)}` required | `{"events": [...], "run_state", "awaiting_orchestrator", "remaining", "dropped", "superseded"}` | none; values above 50 are clamped |
| `finish_run` | `{summary: string (minLength 1, maxLength 8000)}` | `{"state": "ready"}` | `wrong_state` with the unfinished task ids |

The task object schema in `submit_plan` and `add_tasks`: required `id` (pattern `^[a-z0-9][a-z0-9-]{0,15}$`, milestone 8's rule), `title`, `kind` (enum of spec 8.3's six kinds), `prompt`, `acceptance` (array of strings), `runtime` (`claude` or `codex`); optional `depends_on` (array of strings, default empty), `model` (string or null), `reviewer` (`{runtime, model}` or null).

`answer` answers the oldest open question on the task and delivers `[anthrex] Answer from the orchestrator: <text>` to the worker window. `send` delivers `[anthrex] Message from the orchestrator: <text>`. Both go through the run's outbox, milestone 8's delivery (decision 41 of milestone 8). `reassign` on Blocked or NeedsHuman changes the assignment only; a `retry` must follow to dispatch.

### Role contract

`ORCHESTRATOR_CONTRACT` is exactly this text. It is product behaviour; change it only with a spec change.

```text
You are the orchestrator of an anthrex run. anthrex runs Claude Code and Codex agents side by side. The user gave you a goal. You turn it into a plan of tasks. anthrex runs each task in its own git worktree with a worker agent, has a different agent review the work, sends review findings back to the worker, and merges approved work into the run branch. You make the decisions; anthrex does the mechanical work. Nothing reaches the user's base branch until the user accepts the result.

Rules
1. You never edit code. All changes happen through tasks. You may read and search files. Do not run commands that change files, branches or commits.
2. Make tasks small and independent where possible. A task is one reviewable change with written acceptance criteria that a reviewer can check. Use depends_on only for real ordering needs. A task that depends on others starts from a run branch that already contains their merged work.
3. Choose a runtime and a model for every task from list_models, following the routing guidance below. Use only entries marked allowed. Explain each choice in one line at the top of the task's prompt, for example: "Routing: claude-haiku-4-5, fast tier, mechanical rename across five files."
4. Prefer cross-runtime review: a task written by Codex is reviewed by Claude, and the reverse. Leave reviewer empty to let anthrex choose that way, or set it yourself.
5. After submit_plan, loop on wait_for_events until the run is Ready. Call wait_for_events again as soon as you have handled a batch. Answer questions promptly. Do not approve work that failed review without saying why.
6. Keep the user informed: after each batch of events, write a two-line status in your own window. Line one says what happened. Line two says what you do next.

Routing guidance
- Mechanical edits and documentation go to the fast tier.
- Ordinary implementation, tests and refactors go to the standard tier.
- Cross-cutting design, subtle concurrency, debugging and final review go to the frontier tier.
- When both runtimes are allowed, spread independent tasks across both.
- Give each review to the other runtime, at the author's tier or higher.
- Use each model's listed strengths to break ties. The user may change any assignment before approving the plan.

Writing the plan
- Call submit_plan with {"plan": {"notes": "...", "tasks": [...]}}. Each task has id (1 to 16 characters from a-z, 0-9 and -), title, kind (implement, test, refactor, docs, investigate or fix), prompt, acceptance (a list of checkable criteria), depends_on, runtime, model (null for the runtime's default) and optionally reviewer {runtime, model}.
- Write every prompt for a worker who has never seen this conversation: what to change, where, why, and how to check it.
- If submit_plan returns errors, fix every listed error and call submit_plan again.
- get_run shows the limits for this run. Stay within max_tasks.

Events and what to do
- plan_rejected {comment}: the user rejected the plan. Revise it to address the comment and call submit_plan again.
- task_state {task_id, from, to}: progress. Usually no action. When a task is Merged, check that the rest of the plan still fits.
- review {task_id, round, verdict, findings}: anthrex has already sent requested changes to the worker. Act only if the findings show the task itself is wrong: then cancel_task, reassign, or add_tasks.
- verify {scope, task_id, ok, tail}: with scope task, anthrex has already sent the failure to the worker. With scope run, the merged run branch fails the verify command: add_tasks with a fix, or call finish_run and explain the failure in the summary.
- question {task_id, question_id, text}: a worker is asking. Reply with answer. If the repository and the goal do not settle it, answer with the safest assumption and mention it in your status.
- blocked {task_id, reason}: retry with a note, reassign then retry, cancel_task, or add_tasks for a missing prerequisite.
- needs_human {task_id, why}: when why is review_rounds_exhausted, read the findings with get_run, then retry with a note (the review rounds start again), approve_anyway with a reason, or cancel_task. For worker_gone, dependency_cancelled and window_limit, retry, reassign or cancel_task.
- window_exited {task_id, role, code}: a worker or reviewer window ended. Wait for the related needs_human event, or retry.
- final_review {verdict, findings}: the review of the whole run diff. For critical or important findings, add_tasks with fixes. Otherwise call finish_run.
- user {text}: a message from the user or from anthrex. Follow it.

Finishing
When every task is Merged or Cancelled and you have handled the final review, call finish_run with a summary for the user: what was done, what was not, every task approved without passing review and why, and what the user should check. The user then chooses to merge, open a pull request, keep or discard. You never merge, push or finish on the user's behalf.

Messages that start with [anthrex] come from anthrex, not from the user. "[anthrex] 3 events waiting; call wait_for_events." means exactly that.
```

### Configuration

This milestone adds no configuration key. Milestone 8 parses every `[orchestrator]` key (product spec 10.4), including `orchestrator.builtin_models` and `[[orchestrator.models]]`. What changes here is how two of them are applied:

| Key | Default | Change in this milestone |
|-----|---------|--------------------------|
| `orchestrator.max_parallel` | 3 (1 to 8) | The run form and `--parallel` override it for one run (decision 34) |
| `orchestrator.max_windows` | 20 | Also counts the orchestrator window and every final reviewer (decisions 2 and 15) |

Environment, test only: `ANTHREX_NUDGE_INTERVAL_MS` (decision 33).

### CLI

```
anthrex run start "<goal>" --orchestrator <runtime>[:<model>] [--workers claude,codex] [--verify <cmd>] [--parallel <n>] [--yes] [--dir <dir>]
anthrex run start --plan <file> [--dir <dir>] [--yes]            # milestone 8, unchanged
anthrex run reject <run> "<comment>"                             # new
anthrex run tell <run> "<text>"                                  # new: queues a user event
anthrex run status [<run>] [--json]                              # adds orchestrator runtime, model, window, pending events, hold
anthrex run finish <run> merge|pr|keep|discard [--yes] [--confirm <run-id>]   # milestone 8's flags; also accepted from a hold (decision 16)
```

### Client (`crates/tui`)

```rust
// keymap.rs: new Command::NewRun, bound to `O` after the prefix (product spec 10.3).
// app/mod.rs:
pub enum MainView { Terminal, Plan(PlanView), Finish(FinishView) }   // new; App.main_view
// MainView::Terminal draws milestone 7's panes (or the single terminal without them); milestone 4's
// overview still draws over it while App.overview is set.
// App.run_form: Option<NewRunForm>; App.run_defaults: Option<proto::RunDefaults>
// PendingAction gains: ApprovePlan(String), DropPlanTask(String, String), Finish(String, FinishAction)   // run ids are Strings
// Modal gains: TypeToConfirm { prompt: String, expected: String, input: dialog::TextInput, action: PendingAction },
//              Comment { title: String, input: dialog::TextInput, purpose: CommentPurpose }
// pub enum CommentPurpose { RejectPlan(String) }

// run_form.rs (new, pure; built on milestone 5's dialog::TextInput)
pub struct NewRunForm { /* fields of decision 43, focus index, error: Option<String>, submitting: bool */ }
pub enum RunFormOutcome { Stay, Cancel, Submit(OrchestratedRunSpec) }  // milestone 5's FormOutcome is for WindowSpec
impl NewRunForm {
    pub fn new(defaults: &proto::RunDefaults, dir: PathBuf) -> Self;
    pub fn on_key(&mut self, key: KeyEvent) -> RunFormOutcome;
    pub fn on_paste(&mut self, text: &str);
    pub fn set_error(&mut self, message: String);
}

// plan_view.rs (new, pure)
pub struct PlanView { pub run_id: String, pub selected: usize, pub edit: Option<TaskEditForm> }
pub struct PlanRow { pub n: usize, pub id: String, pub title: String, pub runtime: Runtime, pub model: String, pub deps: String }
pub fn plan_rows(run: &RunInfo) -> Vec<PlanRow>;          // model "" or None shows "(default)"; deps "-" when empty
impl PlanView { pub fn on_key(&mut self, run: &RunInfo, roster: &[ModelEntry], key: KeyEvent) -> PlanOutcome; }

// finish_view.rs (new, pure)
pub struct FinishView { pub run_id: String, pub preview: Option<FinishPreview> }
pub fn headline(run: &RunInfo) -> Vec<String>;
impl FinishView { pub fn on_key(&mut self, run: &RunInfo, key: KeyEvent) -> FinishOutcome; }

// ui/run_form.rs, ui/plan.rs, ui/finish.rs (new): render(frame, &App, area)
```

## Tasks

### M9.1 Check the starting point and verify external flags

**Files.** Modify: `docs/milestones/M9-orchestrator-agent.md` ("Implementation notes" only).

**Tests first.** None; this task produces facts that later tasks depend on.

**Change.**
1. Check the names in "Starting point" and "Interfaces" against the merged code of milestones 3 to 8. Record every name that differs, with its real name and file, in a table under "Implementation notes", and use the real names from then on. Nothing needs recording when they match.
2. With the installed `claude`, record `claude --version` and check in `claude --help` that `--mcp-config`, `--allowedTools`, `--disallowedTools`, `--append-system-prompt` and `--model` exist, and whether `--allowedTools` and `--disallowedTools` take a comma-separated string. Check how long Claude Code waits for an MCP tool call before timing it out, from its documentation for the installed version; a 50-second call must complete.
3. With the installed `codex`, record `codex --version` and check `codex --help` and `codex features list` for `-s read-only`, `-a on-request`, `-c developer_instructions`, and `mcp_servers.<name>.default_tools_approval_mode` and `tool_timeout_sec` (spec 11.1 requires this check in every milestone that launches Codex). Check that an MCP server started by a `-s read-only` Codex can connect to a Unix socket under `/tmp`: milestone 8's reviewer launch is the evidence if it passed its manual check.

**Acceptance.** "Implementation notes" contains the name table (or "no differences") and the flag results with versions. Any flag that does not work stops the dependent decision per `AGENTS.md`.

### M9.2 fake-agent: polling, assertions and a call log

**Files.** Modify: `crates/fake-agent/src/script.rs` (the `Step` enum and parsing) and `crates/fake-agent/src/main.rs` or the module where milestone 8 put `mcp_call`. Test: `crates/fake-agent/tests/mcp_and_scripts.rs` (milestone 8's file).

Milestone 3 owns the script format and its ten steps; milestone 8 owns per-role scripts (`<git common dir>/fake-agent/<role>-<task>-<n>.jsonl`, with `run` for a missing `--task`, so the orchestrator's scripts are `orchestrator-run-<n>.jsonl`), `mcp_call`, `read_message` and `sh`. Product spec 12 lists every step with its owner. This task adds only the three items below.

**Tests first.**
- `mcp_wait_repeats_until_a_matching_event`: against a stub stdio MCP server script (a small test helper that replies to `wait_for_events` with empty events twice, then a `question` event), assert the step returns after the third call and the log shows three calls.
- `expect_step_fails_the_script_on_mismatch`: `{"expect": {"is_error": false}}` after a call whose result was an error makes fake-agent exit with code 3 and print `expect failed`.
- `log_records_calls_and_messages`: with `FAKE_AGENT_LOG` set, an `mcp_call` and a `read_message` each append one JSON line.

**Change.**
1. Step `{"mcp_wait": {"tool": "wait_for_events", "args": {...}, "until": {"type": "question", "task_id": "t2"}, "deadline_ms": 60000}}`: repeat milestone 8's `mcp_call` until the result's `events` contains one whose fields match every key in `until`; on deadline exit 4. The last result becomes `FAKE_AGENT_RESULT`, as for `mcp_call`.
2. Step `{"expect": {"is_error": bool, "contains": "text"}}`: asserts on the last `mcp_call` or `mcp_wait` result; exit 3 on mismatch.
3. `FAKE_AGENT_LOG`: append one JSON line per MCP call `{"tool", "args", "is_error", "result", "at_ms"}` and per message read `{"read_message": text, "at_ms"}`, where `at_ms` is milliseconds since the Unix epoch.

**Acceptance.** The three tests pass; the fake-agent tests of milestones 3 and 8 pass.

### M9.3 Protocol version 6, roster and limits

**Files.** Modify: `crates/proto/src/lib.rs` (`PROTO_VERSION = 6`), `crates/proto/src/messages.rs`, `crates/proto/src/run.rs`, `crates/daemon/src/run/roster.rs` (milestone 8's), `crates/mcp/src/forward.rs` (`call_id`), the TUI and CLI message matches. Create: `crates/daemon/src/run/orchestrator/mod.rs`.

**Tests first.**
- In `crates/proto/src/messages.rs`: `every_v6_message_round_trips` builds one value of each new `ClientMsg` and `DaemonMsg` variant, and a `RunInfo` with `orchestrator` and `stats` set, and asserts MessagePack round-trip equality, including `ToolCall` and `ToolResult` with a `call_id`. Milestone 8 already tests `Tier`'s order and names.
- In `run/roster.rs`: `resolve_orchestrator_picks_highest_tier_first_in_order` for `claude` (gets `claude-opus-5`), `codex` and `codex:` (get the `""` entry), `claude:nope` (error `claude:nope is not in the roster`). `reviewer_choice_respects_allowed_runtimes`: with only `claude` allowed, a `claude-sonnet-5` author gets another Claude entry at its tier or higher, and with both allowed the result equals milestone 8's.
- Milestone 8 already tests the roster parsing (built-ins, extension, replacement, `builtin_models`, invalid entries) and the `[orchestrator]` limits, including `max_parallel` 1 to 8. Do not duplicate those tests.

**Change.** Add the types and messages of "Interfaces". Every client that matches on `DaemonMsg` handles the new variants; the TUI stores `RunDefaults` and `FinishPreview` for M9.10 and M9.12 and ignores them until then. Implement decisions 1 (resolution), 23 (the `allowed` parameter) and 34.

**Acceptance.** `cargo test --workspace` passes; a v5 client is refused at `Hello` with milestone 1's version-mismatch path.

### M9.4 Pure engine: orchestrated runs

**Files.** Modify: milestone 8's pure engine `crates/daemon/src/run/engine/` (`mod.rs`, `flow.rs`, `tools.rs`, `control.rs`), `crates/daemon/src/run/model.rs` and `crates/daemon/src/run/plan.rs`. Create `crates/daemon/src/run/orchestrator/events.rs` (`OrchestratorEvent`, `EventQueue`, pure). Tests go in the engine's existing test module in `engine/mod.rs`.

**Tests first.** Each builds a run with an orchestrator and a roster, applies inputs, and asserts the new state and the emitted effects and events.
- `orchestrated_run_starts_in_planning_and_waits_for_submit_plan`.
- `submit_plan_outside_planning_is_wrong_state`.
- `invalid_plan_returns_every_error_and_keeps_planning`: unknown dependency `t9`, a cycle `t1 -> t2 -> t1`, runtime `codex` when only `claude` is allowed, model `claude-foo`; assert four errors naming task and field.
- `valid_plan_moves_to_awaiting_approval_or_running_with_auto_approve`.
- `reject_returns_to_planning_and_queues_plan_rejected`.
- `edit_and_drop_in_awaiting_approval`: edit t2 to `claude-opus-5` succeeds; editing to a model outside the roster fails; dropping t1 while t3 depends on it fails with `t3 depends on t1`; approving queues one `user` event summarising both changes.
- `exceptions_become_events`: ask, report_blocked, rounds exhausted, worker window exit, dependency cancelled each queue the matching event of decision 31; `task_state` is queued for every task transition.
- `approve_anyway_only_after_exhausted_rounds`: wrong state for a Blocked task; after exhausted rounds it moves to Approved and records the reason.
- `retry_resets_rounds_and_carries_note_and_findings`.
- `reassign_is_refused_while_in_flight` for Running and Reviewing; allowed for Ready and NeedsHuman.
- `run_verify_failure_holds_in_integrating` and `final_review_holds_in_final_review`: no automatic Ready; `awaiting_orchestrator` true; `final_review` or `verify {scope: run}` queued.
- `add_tasks_from_a_hold_returns_to_running`; `add_tasks_beyond_max_tasks_is_limit_reached` with the exact message of decision 35, counting Cancelled tasks.
- `finish_run_needs_a_hold_and_finished_tasks`; `finish_run_from_hold_moves_to_ready_and_stores_summary`.
- `user_finish_is_accepted_from_a_hold` (decision 16).
- `window_limit_turns_dispatch_into_needs_human`: with `max_windows = 3` and the orchestrator counting as one, the third task's dispatch yields NeedsHuman `window_limit`.
- `plan_file_runs_are_unchanged`: a run without an orchestrator still goes FinalReview to Ready automatically and queues no events.
- `default_reviewer_respects_allowed_runtimes` (decision 23).

**Change.** Implement decisions 10 to 18, 23, 24, 35 to 40 in the pure engine. The engine pushes each `OrchestratorEvent` into `Run.events` and emits `Action::EventsQueued { run_id }` (Interfaces). Add `orchestrator`, `allowed_runtimes`, `awaiting_orchestrator`, `summary`, `events` and `last_nudge` to milestone 8's `Run` with `#[serde(default)]`, so milestone-8 state files load unchanged; `windows_created` and `RunLimits.max_tasks` already exist. New engine inputs: `StartOrchestrated`, `EditPlanTask`, `DropPlanTask`, `RejectPlan`, `Tell`, and the orchestrator tools through milestone 8's `Input::Tool`.

**Acceptance.** All listed tests pass; milestone 8's engine tests pass unchanged.

### M9.5 Event queue, long-poll, concurrent calls and the waking rule

**Files.** Create: `crates/daemon/src/run/orchestrator/nudge.rs`. Modify: `crates/daemon/src/run/orchestrator/events.rs`, `crates/daemon/src/run/driver.rs` (`RunService`), the server's `ToolCall` and new `ToolCancel` handling in `crates/daemon/src/server.rs`, and `crates/mcp/src/forward.rs` and `lib.rs` for `call_id` and `ToolCancel`.

**Tests first.**
- `events.rs`: `batches_are_oldest_first_and_at_most_50` (push 60, take 50 with seq 1 to 50, then 10). `restore_puts_a_batch_back_in_order`. `cap_drops_oldest_and_reports_dropped` (push 1005: 1000 remain, `dropped = 5` in the next batch, then 0). `queue_round_trips_through_json`.
- `nudge.rs`: `nudge_due` table test covering Idle and Done (due), Working (not), zero queued (not), polling (not), last nudge 59 s ago (not), 60 s ago (due), never (due). `nudge_text(1)` is `[anthrex] 1 event waiting; call wait_for_events.` and `nudge_text(3)` is `[anthrex] 3 events waiting; call wait_for_events.`
- Driver tests with a real daemon socket, in `crates/daemon/tests/orchestrator_events.rs`: `wait_returns_immediately_when_events_are_queued`; `wait_blocks_until_an_event_then_batches_a_burst` (queue three events 50 ms apart after 300 ms; one call returns all three); `wait_times_out_empty` (timeout 1 s, returns within 1.5 s); `timeout_above_50_is_clamped` (inspect the effective deadline through the waiter, not by waiting 50 s); `second_wait_supersedes_the_first`; `a_long_poll_does_not_block_get_run_on_the_same_connection` (an open 5-second wait, then `get_run` answers within 500 ms); `closed_connection_restores_the_batch` (drop the connection between take and hand-over by closing the socket; the next wait gets the same seq numbers); `cancel_leaves_events_queued`.

**Change.** Implement decisions 26 to 30, 32 and 33. `RunService` handles `wait_for_events` itself instead of sending it into `Engine::handle`: it holds no lock while waiting, waits on a `tokio::sync::Notify` per run (woken by `Action::EventsQueued`), and takes the batch from `Run.events` under the engine lock (`crate::lock`). The nudge check runs on orchestrator status changes, through milestone 8's status watcher, and on a 5-second tick, and queues the nudge in the run's outbox like any milestone-8 message. `OrchestratorInfo.pending_events` and `polling` are updated and published through `RunsChanged`, coalesced to at most one publish per 250 ms per run.

**Acceptance.** All tests pass; no test sleeps as its only synchronisation.

### M9.6 Orchestrator tools and the MCP role

**Files.** Create: `crates/daemon/src/run/orchestrator/tools.rs`. Modify: milestone 8's tool dispatch (`RunService::tool_call` in `run/driver.rs` and `run/engine/tools.rs`) to route role `orchestrator`, and milestone 8's decision-25 authorization to accept the run's orchestrator window; the `anthrex mcp` tool listing, `tools_for(Role::Orchestrator)` in `crates/mcp/src/tools.rs`. Test: `crates/cli/tests/mcp_orchestrator.rs`.

**Tests first.**
- In `tools.rs`: `tool_defs_are_the_twelve_spec_tools_in_order`. `every_schema_is_a_closed_object` (type `object`, `additionalProperties: false`, `required` lists exactly the non-optional fields of `OrchestratorCall`). `parse_call_rejects_unknown_fields_and_tools` (`get_run` with `{"x":1}` is `invalid_arguments`; `rm_rf` is `unknown_tool`). `wait_for_events_clamps_to_50`.
- In `crates/cli/tests/mcp_orchestrator.rs`, with an isolated daemon and a run whose orchestrator window runs a fake agent that only does `{"read_line": true}`: spawn `anthrex mcp --role orchestrator --run <id> --window <orchestrator window id> --socket <socket>` with piped stdio and speak JSON-RPC: `initialize`, then `tools/list` returns the twelve names; `tools/call get_run` returns the run with state `planning`; `tools/call answer` on `t1` before any plan returns `isError: true` with code `unknown_task`; the same server started with `--role worker --task t1` calling `submit_plan` gets `isError: true` with milestone 8's `unknown tool submit_plan`; started with `--role orchestrator` but the `--window` of another window, `get_run` gets `this window is not the orchestrator of run <id>`.
- `each_tool_error_case` in the same file: one assertion per error row of the tools table, driven by `ToolCall` messages on a raw socket against a run in the needed state.

**Change.** Implement the table in "Orchestrator tools and schemas". Tool results are JSON text built with `serde_json`. `get_run` includes everything the orchestrator needs so it never reads task worktrees.

**Acceptance.** Tests pass; `anthrex mcp --role orchestrator` lists exactly the twelve tools with their schemas.

### M9.7 Contract, launch on both runtimes, and starting a run

**Files.** Create: `crates/daemon/src/run/orchestrator/contract.rs`. Modify: milestone 8's role launch code (`crates/daemon/src/run/role_launch.rs`, and `crates/daemon/src/launch/claude.rs` and `codex.rs` for the orchestrator flags), `RunService`'s start path in `run/driver.rs`, the server for `StartOrchestratedRun` and `GetRunDefaults`.

**Tests first.**
- `contract.rs`: `contract_mentions_every_tool_and_event` (each of the twelve tool names and ten event types appears). `contract_has_no_em_dash_and_no_surrounding_whitespace` (no `\u{2014}`; equal to its own `trim()`). `contract_round_trips_as_toml_string`: parse `x = <launch::codex::toml_string(ORCHESTRATOR_CONTRACT)>` with the `toml` crate and assert equality. `first_prompt_is_exact` for goal `Add password reset`, slug `add-password-reset-1a2b`, root `/tmp/repo`.
- Launch tests next to milestone 8's role launch tests: `claude_orchestrator_argv_is_exact`: the args after milestone 3's hook arguments equal decision 5's list, with the `--mcp-config` JSON parsed and compared as a value, and `--` then the first prompt last. `claude_orchestrator_passes_model` (`--model claude-sonnet-5` present when the orchestrator entry is `claude-sonnet-5`). `codex_orchestrator_argv_is_exact` per decision 6, including `-s read-only -a on-request`. `codex_orchestrator_with_default_model_omits_m`. `orchestrator_restart_keeps_role_flags`: the resume variant of both launches still contains the MCP and role flags.
- Integration in `crates/cli/tests/orchestrator_start.rs`, with milestone 8's `RunHarness` and `FAKE_AGENT_LOG`: `start_creates_planning_run_and_orchestrator_window` with a fake agent as Claude and as Codex; assert `RunStarted`, the window's `run.role` is `Orchestrator`, its cwd is the project root, and the fake agent's log shows `get_run` succeeded. `start_is_refused_on_a_dirty_tree` (milestone 8's message). `start_rejects_unknown_orchestrator_model` (`claude:nope`). `get_run_defaults_returns_last_verify` after a previous run in the same project with `verify = "true"`.

**Change.** Implement decisions 1 to 9, 19, 41. The start handler resolves the orchestrator and workers, snapshots limits, runs milestone 8's `preflight` and slug choice, creates the run in Planning (milestone 8 creates the run branch only on entering Running), then creates the orchestrator window with `create_with_role`. If creating the window fails, the run goes to Failed with the error and no worktrees remain.

**Acceptance.** Tests pass. The printed argv of a real start, visible in the daemon log at debug level, matches decisions 5 and 6.

### M9.8 End-to-end orchestrated runs with fake agents

**Files.** Create: `crates/cli/tests/orchestrator_e2e.rs`. Modify: `crates/cli/tests/support/run_harness.rs` (milestone 8's `RunHarness` gains `env(key, value)`, applied before the daemon starts, like `config`). These tests live in the CLI crate because `env!("CARGO_BIN_EXE_anthrex")` is the real binary the MCP server command must point to.

Each test uses milestone 8's `RunHarness`: a temporary git repository with one commit, an isolated daemon with `fake-agent` as both runtimes, and per-window scripts written with `RunHarness::script(name, lines)` into `<repo>/.git/fake-agent/<role>-<task>-<n>.jsonl` (the orchestrator's are `orchestrator-run-<n>`, the final reviewer's `reviewer-run-<n>`). It adds `FAKE_AGENT_LOG` and a config with `max_review_rounds = 2`, starts the run with verify `true`, waits with deadline loops of at most 60 seconds, and stops the daemon at the end. Every script follows milestone 8's pattern: a `SessionStart` hook first, and `UserPromptSubmit` before and `Stop` after each turn, so the window goes Idle and queued messages can be delivered.

**Tests first.**
- `orchestrated_run_completes` run twice, as `claude_orchestrator` and `codex_orchestrator`. Scripts:
  - Orchestrator: `submit_plan` with t2 depending on unknown `t9`, `expect {is_error: true, contains: "t9"}`; `submit_plan` valid with t1 (codex worker, no reviewer), t2 (claude worker, reviewer codex), t3 (codex, depends on t1 and t2); `mcp_wait` for `question` on t2; `answer` t2 `use the blue one`; `mcp_wait` for `needs_human` on t2; `approve_anyway` t2 `findings are style only`; `mcp_wait` for `final_review`; `add_tasks` with t4 (claude, fix); `mcp_wait` for a second `final_review`; `finish_run` `All four tasks merged.`; `read_line`.
  - Worker t1: `git_commit`, `report_done`. Worker t2: `ask`, `read_message` (the answer), `git_commit`, `report_done`, then `read_message` (the round-1 findings), `git_commit`, `report_done`. With `max_review_rounds = 2`, the second `changes` verdict goes to NeedsHuman and sends no message. Worker t3 and t4: `git_commit`, `report_done`.
  - Reviewers: `reviewer-t1-1`, `reviewer-t3-1` and `reviewer-t4-1` `approve`; `reviewer-t2-1` and `reviewer-t2-2` `changes` with one important finding. Final reviewer `reviewer-run-1`: `changes` with an important finding; `reviewer-run-2`: `approve`.
  - The test sends `RunApprove { run_id }` after `RunsChanged` shows AwaitingApproval.
  - Assertions: run state Ready with summary `All four tasks merged.`; `git log --merges` on the run branch has four merges for t1 to t4; t3's branch start commit contains t1's and t2's files; the base branch head equals `base_sha`; t1's reviewer window runtime is Claude and t2's is Codex; worker windows used both runtimes; the t2 worker log shows a `read_message` containing `[anthrex] Answer from the orchestrator: use the blue one`; `REPORT.md` contains `findings are style only` and `## Summary from the orchestrator`; the orchestrator log's first `submit_plan` has `is_error: true`.
- `rejected_plan_comment_reaches_the_orchestrator`: the orchestrator submits a valid plan, then `mcp_wait` for `plan_rejected`, then `submit_plan` again. The test sends `RejectPlan { comment: "split t1 into two tasks" }`. Assert the log shows the event with that comment and the run is AwaitingApproval again with the second plan.
- `idle_orchestrator_is_nudged_once_per_interval`: `ANTHREX_NUDGE_INTERVAL_MS=1000`. The orchestrator submits a plan with `auto_approve`, runs the `Stop` hook so its window goes Idle, then `read_message` twice, with the `UserPromptSubmit` and `Stop` hooks around the first so the window is busy and Idle again between them. Events queue as workers progress. Assert the first `read_message` is the exact nudge text with the queued count, and that two nudges are at least 1000 ms apart by the log's `at_ms`.
- `worker_question_without_answer_stays_open`: `answer` on a task without an open question returns `no_open_question`.

**Change.** Fix whatever these tests expose. Keep scripts small: one JSON-lines file per window.

**Acceptance.** All four tests pass on macOS and Linux in CI, each within 60 seconds.

### M9.9 CLI: start with a goal, reject, tell, status

**Files.** Modify: milestone 8's CLI run module `crates/cli/src/run_cmd.rs`, `crates/cli/src/main.rs`.

**Tests first.** Unit tests next to the argument parsing: `orchestrator_arg_parses` for `claude`, `claude:claude-opus-5`, `codex`, `codex:` (model None), `codex:gpt-x`, and refuses `shell` and `gpt:x`. `workers_arg_parses_comma_lists` (`claude,codex`, `codex`; refuses empty and `shell`). `goal_and_plan_are_exclusive`; `orchestrator_required_with_goal`; `parallel_range_is_1_to_8`. Integration in `crates/cli/tests/orchestrator_start.rs`: `run_start_prints_slug_and_window`, `run_reject_and_tell_queue_events` (the orchestrator log shows `plan_rejected` and `user`), `run_status_json_has_orchestrator_fields`.

**Change.** Implement decision 42 and the CLI block of "Interfaces". `run start` without `--verify` sends `verify: None`; `--verify ""` sends `Some("")`.

**Acceptance.** Tests pass; `anthrex run --help` lists `reject` and `tell`.

### M9.10 Client: the new-run form

**Files.** Create: `crates/tui/src/run_form.rs`, `crates/tui/src/ui/run_form.rs`. Modify: `crates/tui/src/keymap.rs` (`Command::NewRun` on `O`), `crates/tui/src/app/mod.rs`, `crates/tui/src/app/tests.rs`, `crates/tui/src/ui/mod.rs`, `crates/tui/src/ui/modal.rs` (`HELP` gains `C-b O` `new orchestrated run`).

**Tests first.**
- `keymap.rs`: `prefix_then_capital_o_is_new_run`.
- `run_form.rs`: `defaults_fill_from_run_defaults` (first frontier entry, both workers, last verify, max parallel). `tab_cycles_six_fields_and_shift_tab_reverses`. `left_right_cycle_orchestrator_and_workers_and_clamp_parallel` (1 and 8 bounds). `alt_enter_adds_newline_to_goal_and_enter_submits`. `empty_goal_shows_inline_error_and_does_not_submit`. `submit_builds_the_exact_spec` (roster entry `codex ""` gives `OrchestratorChoice { runtime: Codex, model: Some("") }`; verify field empty gives `Some("")`). `esc_cancels`.
- `app/tests.rs`: `c_b_o_requests_defaults_then_opens_the_form` (effect `Send(GetRunDefaults { dir })`; the form opens on `RunDefaults`). `daemon_error_for_start_run_stays_in_the_form` (`Error { request: "run start" }` while the form is submitting sets the form error). `run_started_focuses_the_orchestrator_window`.
- `ui/run_form.rs`: `form_renders_fields_and_hint` on a `TestBackend` 80x24: title `new run`, the six labels, the orchestrator line `claude · claude-opus-5 · frontier`, and the hint `Tab next · ←/→ change · Enter start · Esc cancel`.

**Change.** Model the form on milestone 5's `dialog::NewAgentForm`: the same `dialog::TextInput` fields, focus handling and inline error line. Implement decision 43. The directory default comes from the tree selection's project when there is one.

**Acceptance.** Tests pass; `App` and the form stay free of I/O.

### M9.11 Client: the plan view and the tree

**Files.** Create: `crates/tui/src/plan_view.rs`, `crates/tui/src/ui/plan.rs`. Modify: `crates/tui/src/app/mod.rs` (`MainView`), `crates/tui/src/tree.rs` and `crates/tui/src/tree_input.rs` (Enter and click on run rows, the orchestrator child row; milestone 8 added the run rows), `crates/tui/src/ui/mod.rs` (draw the main view), `crates/tui/src/ui/modal.rs` (`Modal::Comment`).

**Tests first.**
- `plan_view.rs`: `rows_show_default_model_and_dash_for_no_deps`. `jk_move_selection_and_clamp`. `a_approves` (effect: `Send(RunApprove { run_id })`). `d_asks_then_drops` (`y` sends `DropPlanTask`). `e_opens_edit_form_limited_to_roster` (runtime cycles through allowed runtimes; model cycles through that runtime's roster entries; Enter sends `EditPlanTask`). `r_opens_comment_and_empty_comment_is_refused`; a non-empty comment sends `RejectPlan`. `esc_returns_to_terminal`.
- Tree tests: `enter_on_awaiting_run_opens_plan_view`, `enter_on_running_run_toggles_collapse`, `orchestrator_window_is_the_first_child_of_its_run`.
- `ui/plan.rs` on a `TestBackend` 80x16 with the spec 9.6 example run: the title line contains `plan · Add password reset · orchestrator claude-opus-5`; row `3  t3  Reset endpoints and rate limiting  codex    (default)` and deps `t1 t2`; the key line `a approve   e edit task   d drop task   r reject with comment   Esc back`. `plan_view_while_planning_says_so`: `The orchestrator is planning.`. `selected_task_detail_shows_reviewer_and_acceptance`: below the table, the selected task's reviewer, acceptance criteria and the first four prompt lines.

**Change.** Implement decisions 44 and 45 and the plan view drawing of spec 9.6, with `-` in place of an empty deps cell. The run row gains a suffix: `plan ready` in AwaitingApproval, `ready` in Ready, `deciding` while holding; a pending-events count shows as `·<n>` when above zero.

**Acceptance.** Tests pass; the plan view uses the full main area and ignores terminal input while shown.

### M9.12 Client: the finish view

**Files.** Create: `crates/tui/src/finish_view.rs`, `crates/tui/src/ui/finish.rs`. Modify: `crates/tui/src/app/mod.rs` (`PendingAction::Finish`, `Modal::TypeToConfirm`), `crates/tui/src/ui/modal.rs`, `crates/daemon/src/run/driver.rs` (`GetFinishPreview`).

**Tests first.**
- `finish_view.rs`: `opening_requests_a_preview`. `m_confirms_with_branch_and_root` (message `Merge anthrex/<slug>/integration into main in /tmp/repo?`; `y` sends `RunFinish { run_id, action: Merge, confirm: Some(run_id) }`). `m_is_refused_when_root_is_dirty_or_base_not_checked_out` (toast with the reason, no effect). `p_shows_remote_url_and_branch_before_pushing` (message `Push anthrex/<slug>/integration to origin (git@example:x.git) and open a pull request with gh?`). `p_without_gh_or_remote_toasts`. `k_confirms_keep`. `d_requires_typing_the_slug` (a wrong slug leaves Enter inert; the right slug sends `RunFinish { action: Discard, confirm: Some(run_id) }`).
- `ui/finish.rs` on `TestBackend` 80x20: title `finish · <goal> · ready`; the summary wrapped; `Tasks 4 merged · 0 cancelled · 1 approved anyway`; `Review rounds 6 · findings 0 critical · 3 important · 2 minor`; `Verify on run branch: passed`; the report path; keys `m merge into main   p open pull request   k keep   d discard   Esc back`, where `main` is the base branch. `holding_run_without_summary_says_the_orchestrator_is_deciding`.

**Change.** Implement decision 46 and decision 16's user override from a hold. The daemon's `GetFinishPreview` handler uses milestone 8's `run::git::pr_target` for the remote, the same function the `pr` finish uses, and milestone 8's preflight check for `root_clean`.

**Acceptance.** Tests pass; no finish action is sent without a confirmation.

### M9.13 Smoke stage, help text and documentation touch-ups

**Files.** Modify: `scripts/pty-smoke.py`, `crates/tui/src/ui/modal.rs` if not already done, `docs/ROADMAP.md` at the end per `AGENTS.md`.

**Tests first.** The new smoke stage itself.

**Change.** Add `== stage 13: plan view ==` after milestone 8's stage 12, before the final daemon stop. Create `/tmp/anthrex-smoke-orch-<pid>` as a git repository with one commit, and write into its `.git/fake-agent/orchestrator-run-1.jsonl` a script whose orchestrator submits a two-task plan and then `read_line`s. Milestone 3's smoke setup already sets `ANTHREX_CLAUDE_BIN` to `target/debug/fake-agent`. Run `anthrex run start "smoke goal" --orchestrator claude --dir <repo>`; attach; press `C-b t`, move to the run row, press `Enter`; wait for `a approve` and `(default)` or the model name on the reconstructed screen; press `r`, type `too big`, press `Enter`; wait for the run row to show the planning state; press `Esc`; detach; `anthrex run cancel` the run; remove the repository. Assert each wait succeeds.

**Acceptance.** `python3 scripts/pty-smoke.py` passes with the new stage.

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

1. `cargo test -p anthrex --test orchestrator_e2e` passes three times in a row; flakiness here means a missing deadline loop.
2. `grep -n "PROTO_VERSION: u32 = 6" crates/proto/src/lib.rs` matches.
3. `anthrex mcp --role orchestrator --run x` answers `tools/list` with exactly twelve tools, checked by `crates/cli/tests/mcp_orchestrator.rs`.
4. `pgrep -fl "anthrex daemon"` and `pgrep -fl fake-agent` show nothing after the test run.

## Manual check

Use a scratch toy repository and an isolated daemon. Every command below runs with these variables set.

```bash
export ANTHREX_SOCKET=/tmp/anthrex-m9.sock ANTHREX_DATA_DIR=/tmp/anthrex-m9-data
rm -rf /tmp/anthrex-m9-toy && mkdir /tmp/anthrex-m9-toy && cd /tmp/anthrex-m9-toy && git init -q
printf 'def add(a, b):\n    return a + b\n' > calc.py
printf 'import unittest\nfrom calc import add\n\nclass T(unittest.TestCase):\n    def test_add(self):\n        self.assertEqual(add(2, 3), 5)\n' > test_calc.py
printf '# Changelog\n\n' > CHANGELOG.md
git add -A && git commit -qm init
```

1. Run `anthrex --dir /tmp/anthrex-m9-toy`. Press `C-b O`. Check the form shows the roster, both workers selected, and an empty verify. Enter goal `Add subtract, multiply and divide to calc.py, each as its own task with tests. Every task must add one line at the top of CHANGELOG.md.`, choose `claude · claude-opus-5`, verify `python3 -m unittest -q`, max parallel 3. Press Enter. The orchestrator window is focused and starts exploring.
2. Watch it call `get_run`, `list_models` and `submit_plan`. Open the tree with `C-b t`, select the run row, press Enter. Check the plan view: tasks on both runtimes, a routing line at the top of each prompt, `-` for no deps.
3. Press `e` on one task and change its model; press Enter. Press `a`. Check the orchestrator later reports the edit from the `user` event.
4. Watch workers start in their own worktrees (`git worktree list` in the toy repo shows them). Check that each task's reviewer is on the other runtime.
5. The CHANGELOG line makes the second and third merges conflict. Check the worker receives the `[anthrex]` merge instruction, resolves the conflict, and goes through review again. If a worker cannot resolve it, check the orchestrator receives `needs_human` and acts.
6. In one worker window, type a question to trigger `ask` if none happens naturally, for example by telling the worker `ask the orchestrator whether divide by zero should raise`. Check the answer arrives as `[anthrex] Answer from the orchestrator: ...`.
7. Leave the orchestrator idle while events queue, for example by pressing Esc in its window during a turn. Check the nudge appears at most once a minute.
8. When the run is Ready, open the finish view. Check the summary, the counts, and `Verify on run branch: passed`. Press `m`, check the confirmation names `anthrex/<slug>/integration`, `main` and the toy repo, press `y`. Check `git log --oneline --graph` in the toy repo.
9. Start a second run with `anthrex run start "Add a README section describing calc.py" --orchestrator codex --workers claude,codex --dir /tmp/anthrex-m9-toy`. In the plan view press `r`, type `use a fast-tier model for docs`, Enter. Check the Codex orchestrator revises the plan. Approve it.
10. While the second run works, press `C-b` then detach with `d`, reattach with `anthrex`, and check the run row and counts. Run `anthrex run tell <slug> "keep the README under 20 lines"` and check the orchestrator reacts.
11. At Ready, open the finish view and press `p`. Check the dialog shows the remote and branch, or refuses with `no remote to push to` in the toy repo. Press `d`, type the slug, and check the worktrees and `anthrex/<slug>/*` branches are gone.
12. Check that neither orchestrator window ever edited a file: `git status` in the toy repo is clean apart from the merge in step 8.
13. Run `anthrex daemon stop`, then `pgrep -fl "anthrex daemon"` shows nothing of yours. Remove `/tmp/anthrex-m9-*`.

## Risks and gotchas

1. **Claude's variadic tool flags swallow the prompt.** `--allowedTools` and `--disallowedTools` may accept several values. Without `--` the first prompt would be read as a tool name. Recognise it: the orchestrator starts with no prompt, or the log shows a tool named like the goal. The `--` rule from milestone 1 prevents it; the launch test asserts `--` precedes the prompt.
2. **MCP call timeouts shorter than the long-poll.** If Claude Code or Codex times out a 50-second `wait_for_events`, the orchestrator sees an error every loop. Recognise it: the orchestrator window shows a timeout after about the client's limit. M9.1 checks both; Codex uses `tool_timeout_sec=120` from spec 11.5. If Claude's limit is lower, record it and lower the clamp in decision 27 to 10 seconds below it, noting it under "Implementation notes".
3. **Read-only Codex and the socket.** If Codex sandboxes the MCP server process, `anthrex mcp` cannot reach the daemon socket and every tool fails with a connection error. Recognise it in M9.1 or in the Codex manual step. Stop and record the evidence; do not switch the orchestrator to a writable sandbox.
4. **Lost events.** Delivery is at most once. If the MCP process dies after the batch is handed over, those events are gone. The contract's restart message tells the orchestrator to call `get_run`, which carries the full state. Do not try to make delivery exactly-once.
5. **Server deadlock from long-polls.** A waiter holding `daemon::lock` or blocking the connection's request loop freezes other calls. The test `a_long_poll_does_not_block_get_run_on_the_same_connection` catches it.
6. **Nudges while the orchestrator is mid-turn.** Status can flicker between Idle and Working. The nudge goes through milestone 8's delivery, which delivers only when the window is Idle or Done, and the 60-second interval bounds repeats.
7. **Large `RunsChanged` frames.** Prompts are in `TaskInfo`. Twelve tasks at 16000 characters is under 200 KB, well under `proto::MAX_FRAME`, but publishes are coalesced (M9.5) so a burst of events does not flood clients.
8. **TOML escaping of the contract.** Newlines, quotes and backslashes must be escaped in `developer_instructions`. The round-trip test with the `toml` crate catches mistakes.
9. **Model ignores the loop.** A real model may stop calling `wait_for_events`. The nudge handles idle windows; the user can type or use `anthrex run tell`; decision 16 lets the user finish anyway.
10. **Plan edits racing a new submission.** The user edits while the orchestrator cannot submit, because `submit_plan` is valid only in Planning and edits only in AwaitingApproval. A rejection moves back to Planning; any open edit form in the client closes when `RunsChanged` shows the new state.
11. **fake-agent script selection across launches.** Reviewers relaunch every round, and every launch claims the next `<role>-<task>-<n>.jsonl` (milestone 8). If a test hangs on a reviewer, check which `.claimed` files exist in `<repo>/.git/fake-agent/`; a launch with no unclaimed script falls back to `FAKE_AGENT_SCRIPT`.

## Follow-ups handled

None. The "Assignment to milestones" table in `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` assigns no items to milestone 9.

## Implementation notes

The implementer fills this section in during implementation.
