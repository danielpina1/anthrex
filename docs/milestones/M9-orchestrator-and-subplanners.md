# Milestone 9: The orchestrator and sub-planners

> Written 2026-09-22 against `docs/superpowers/specs/2026-09-22-adaptive-orchestrator-design.md` ("the spec" below), which binds this milestone. It replaces the superseded brief `docs/milestones/M9-orchestrator-agent.md` (branch `docs/m9-brief-refresh`), from which three grounded parts are reused and adapted: the orchestrator's launch flags on both runtimes, the contract's structure, and the roster payload that `get_roster` returned (now part of `get_context`). Built on three briefs that are not yet code: `docs/milestones/M8a-orchestration-engine-core.md` ("M8a"), `docs/milestones/M8b-adaptation.md` ("M8b") and `docs/milestones/M8c-live-run-view.md` ("M8c"). Every name taken from them is listed under "Names taken from earlier briefs". Existing code was checked on `main` at `2cb7e3c` (milestones 1 to 6, protocol 5) and on branch `m6.5-conversation-view` at `68bb67e` (milestone 6.5, protocol 6).

## Header

| | |
|--|--|
| Status | `blocked` — becomes `ready` when milestones 8a, 8b and 8c are all `done`. |
| Depends on | Milestone 8a (engine, headless sessions, MCP crate, `fake-agent` headless modes, run harness), milestone 8b (profile, scouts, deciders, triage and the fast path, OTLP metering, history), milestone 8c (the run view and the plan gate in `C-b T`). |
| Spec sections | §3 (shape: information flows down through the orchestrator and sub-planners), §4 (roles; only the orchestrator is an interactive terminal; read-only launch; only the user's settings load), §4.2 (who the user can talk to; steering), §5.1 (the plan and large paths; `run promote`), §5.2 (kinds `research` and `review`), §5.3 steps 1, 2, 3, 6 and 7, §6 (`generated` and `protected` as planning rules), §7.1–§7.3 (the rubric from scout evidence, never minutes; interfaces first; split depth one; chain collapse), §8 (test mode rules the planner applies), §9 (routing; never different runtimes on overlapping `owns`), §10 (the orchestrator's reaction to rung 3, rung 4 and `blocked(question)`), §11.4 step 5 and 6 (split, rewrite; the orchestrator never approves), §12.1–§12.4 (plan edits from the orchestrator and sub-planners; the non-blocking plan gate; steering by typing), §13 item 3 (readers), §14 items 1, 2, 4 and 8 (scout once and share the map; prompt layout; the digest; OTLP for the orchestrator), §16.4 "Navigation" (Enter on the orchestrator focuses its window), §17 (restart), §19 (orchestrator and sub-planner tools), §21 (the M9 row and its scenarios), §22.3 (`planner_task_cap`). |
| Branch | `m9-orchestrator-and-subplanners` |
| Protocol version | **One above `PROTO_VERSION` on `main` at the moment M9 starts.** Read `crates/proto/src/lib.rs` on `main` that day, add one, and record the derivation under "Implementation notes". No acceptance criterion greps for a specific number. |

## Starting point

Real on `main` at `2cb7e3c` unless the row says otherwise. Line counts are today's; M8a, M8b and M8c change several of these files first, so re-count at the start and record the counts under "Implementation notes".

| Path | What is there, and what this milestone does with it |
|------|------------------------------------------------------|
| `crates/daemon/src/launch/mod.rs` (427) | `LaunchPlan { program, args, cwd, env }`; `LaunchContext { window_id, name, socket_path, shell, exe, claude_bin, codex_bin, codex_hook_source, codex_bypass_hook_trust, resume }`; `plan(spec, ctx)`, which for Claude builds `--name <name> --settings <json>`, then `--model <m>` when the spec names one, then `--resume <id>` or `-- <prompt>`; `hook_command`, `shell_quote`. M9 adds `LaunchContext.role`, `LaunchPlan.scrub_agent_env`, and the role flags (decisions 7–10). |
| `crates/daemon/src/launch/claude.rs` (71) | `HOOK_EVENTS` (10 events) and `settings(exe, window_id)`, tested byte-for-byte by `claude_settings_json_is_exact`. M9 adds `StopFailure` (decision 12). |
| `crates/daemon/src/launch/codex.rs` (353) | `args(spec, ctx)`: `-C <cwd>`, five `-c` UI/notify overrides, the hook block, then `-m <model>`, then `resume <id>` or `-- <prompt>`. The comment at the `-m` push records that codex-cli 0.155.0 accepts root options ahead of `resume`. `toml_string`. M9 inserts the role block before `-m` (decision 8). |
| `crates/daemon/src/hooks.rs` (313; 391 on the M6.5 branch) | `HookKind` has 11 variants (`SessionStart … SessionEnd, TurnComplete`); `parse` maps `hook_event_name` strings; `to_status_event` maps `Stop` to `StatusEvent::Stop`. M9 adds `StopFailure`. |
| `crates/daemon/src/status.rs` (473) | The PTY status machine; `StatusEvent::Stop` ends a turn. Unchanged except that a `StopFailure` hook now reaches it as `Stop`. |
| `crates/daemon/src/manager/create.rs` (559) | `create(spec, project, worktree, cols, rows)` in three phases; phase A is the private method `admit`, phase B the private free function `spawn_window(config, events, spec, roots, admitted)` that calls `launch::plan` with `resume: None`, phase C the private method `insert`, which builds `Entry { …, process: Process::Live(window), run: None }`. M9 makes `admit`, `spawn_window` and `insert` `pub(super)` and threads an optional role through them (decision 5). |
| `crates/daemon/src/manager/restart.rs` (556) | `restart(self: &Arc<Self>, id)`; `struct ForRelaunch { spec, name, session_id, cols, rows }`; `spawn_for_restart` calls `launch::plan` with `resume: info.session_id`. M9 adds `ForRelaunch.role` so a restart re-passes the orchestrator's flags (decision 11). |
| `crates/daemon/src/manager/entry.rs` (331) | `Entry.run: Option<serde_json::Value>` (line 106), carried verbatim since M6. M8a adds `Entry.headless`; M9 adds `Entry.role` and `Entry.last_client_input` (decisions 11, 39). |
| `crates/daemon/src/state/mod.rs` (203) | `WindowRecord.run: Option<serde_json::Value>` (line 131). M9 stores the orchestrator's `RoleLaunch` there for a `Pty` record (decision 11). |
| `crates/daemon/src/manager/mod.rs` (478) | `validate_name` (at most 64 graphemes, no control characters; `/` is allowed), `write_input`, `restart`. |
| `crates/daemon/src/window.rs` (371) | `Window::spawn(id, &plan, cols, rows, events)` builds a `portable_pty::CommandBuilder`, which starts from the daemon's own environment, then sets `plan.env`. M9 removes the agent session variables when `plan.scrub_agent_env` (decision 10). |
| `crates/daemon/src/server.rs` (556) | `ClientMsg::Input { window_id, bytes }` goes to `manager.write_input` (line 377). M9 records the time of client input for the orchestrator window (decision 39). |
| `crates/proto/src/codec.rs` | `MAX_FRAME` = 16 MiB. |
| `crates/cli/src/main.rs` (527) | Subcommands `Attach`, `Daemon`, `New`, `Ls`, `Tree`, `Kill`, `Rm`, `Rename`, `Restart`. `anthrex restart <window>` is how a user relaunches the orchestrator. |
| `crates/fake-agent/src/script.rs` (190) | M3's `Step` enum, including `ReadLine` and `Hook`. M8a adds the headless steps; M9 adds PTY-mode MCP and three steps (task M9.12). |
| `scripts/pty-smoke.py` (1572) | Far over 600 lines. M8a adds stage `11c` in `scripts/pty_smoke_run.py`, M8b stage `11d` in `scripts/pty_smoke_adapt.py`, M8c stage `11e` in `scripts/pty_smoke_run_view.py`. M9's stage `11f` lives in a new module. |

## Goal

`anthrex run start --goal "<text>"` no longer refuses a goal that needs a plan. A goal the triage decider labels `plan` or `large` — or any goal when no decider answers — starts a run in a new `planning` state and launches its **orchestrator**: a real, interactive `claude` or `codex` TUI in a PTY window, on the frontier tier at high effort, launched read-only, loading only the user's own settings, metered through OTLP, and holding six MCP tools. It is the only run agent the user can type to. It reads the run's context, spawns read-only **scouts** per area, and writes the plan with validated plan edits: sizes from scout evidence, interfaces first, hub tasks alone, chains collapsed where it judges a link need not be reviewed or merged on its own (guidance, not an engine check), nothing L, test modes by the rules, routes that never put two runtimes on overlapping paths. On a large goal it writes the interface tasks itself and starts one headless **sub-planner** per epic, each confined to its area, which submits its epic once and exits. Submitting opens the plan gate without blocking any tool call; the user approves, edits or rejects in the run view, and the orchestrator reads the verdict from `run_status`, a digest it long-polls for up to 50 seconds. While the run executes, the engine wakes an idle orchestrator with a short `[anthrex]` line when something needs it; it answers blocked questions, splits or rewrites mis-sized tasks, tells the user what only the user can fix, and turns whatever the user types into plan edits. It never approves a task and never merges. Research goals run as report-writing scout tasks and review goals as reviewer tasks against a named range, neither merging anything; a large run gets one integration review per epic. `anthrex run promote` now gives a fast-path run an orchestrator. When the run completes the orchestrator writes a summary; the user accepts or discards. Every behaviour of the engine and MCP side is exercised in CI by scripted `fake-agent`s; the model's own judgement is checked by a manual run.

## Scope

In:

- The orchestrator window: its launch on both runtimes (read-only, user settings only, OTLP metering, scrubbed session variables), persistence, restart with its flags, the `StopFailure` hook for its status, and the daemon's refusals for it.
- The orchestrator's tools (`get_context`, `spawn_scout`, `spawn_subplanner`, `edit_plan`, `run_status`, `task_result`) and the sub-planner's (`get_context`, `submit_epic`), in `crates/mcp` and the engine.
- `ORCHESTRATOR_CONTRACT` and `PLANNER_CONTRACT`, every prompt and message they need, and the worker prompt's scout extract.
- Plan rules only planners are held to: `planner_task_cap`, scout evidence in `scout_refs`, no budgets from a planner, sub-planner scope. Chain collapse is contract guidance for the planners' judgement, never an engine check (spec §7.3, §22 item 6).
- The plan path and the large path of spec §5.1 (replacing M8b's refusal), the `planning` run state, submitting a plan, and the non-blocking plan gate from the orchestrator's side, including holds on epics added after approval.
- Sub-planners (`AgentRole::Planner`): headless sessions per epic, re-planning with a fresh session, and `RunInfo.planners`.
- Run scouts per area on M8b's `ScoutService`, in reader slots, feeding briefs.
- Task kinds `research` and `review`, executed.
- The per-epic integration review of the large path.
- Wake-ups for an idle orchestrator, and the engine support for its reactions (rewrite restarts a mis-sized task; replacing a cancelled dependency).
- `run promote` performing the promotion M8b recorded.
- `finish` with the orchestrator's summary.
- The small TUI changes the new states need in M8c's view (task M9.15).
- `fake-agent`: PTY-mode MCP calls, PTY `read_message`, `mcp_until`, `capture_json`, `expect`, planner and run-scout scripts, and an MCP call log.

Out:

| Out | Owner |
|-----|-------|
| Adaptive concurrency (§13 item 9), threshold and budget refitting, routing proposals in `run stats` | M9.5 |
| Racing (§4.1 "Race") and the test-writer-then-implementer pattern (`pair = true`), with their `AgentRole` variants and node labels | M9.5 |
| `RunInfo.estimate_left_secs` and `bound_ratio_permille` (history-derived, M8c placeholders) | M9.5 |
| A TUI form for starting a goal run or editing the profile | Follow-up (the CLI starts goal runs; M8b's follow-up for the profile form stays open) |
| OTLP metering of a Codex orchestrator | Follow-up (decision 14) |
| Stub-then-fill, a merge queue wider than 1, a learned router | Deferred by the spec (§13, §15) |
| Orchestrators that spawn orchestrators; sub-planners that spawn anything | Never (spec §18, amendment §5.10 row) |

## Design decisions

Numbered and final. If one proves wrong or impossible, stop work on it, record the evidence under "Implementation notes", and continue with the tasks that do not depend on it.

### Structure

1. **Where the code lives.** In `crates/daemon/src/`:
   - `run/orch/`: `mod.rs` (model types), `contract.rs` (contracts, prompts, message texts), `tools.rs` (argument parsing into `OrchCall`), `rules.rs` (decision 23's plan rules), `digest.rs` (`run_status`), `context.rs` (`get_context`), `result.rs` (`task_result`), `launch.rs` (the orchestrator's `RoleLaunch` and the headless specs of sub-planners, research sessions, review-task reviewers and integration reviewers).
   - `run/engine/`: `orch.rs` (planning, submit, holds, promotion, the orchestrator window's lifecycle, wake notes), `planners.rs` (sub-planners and run scouts), `kinds.rs` (research and review tasks), `integration.rs` (per-epic integration review and completion).
   - `run/driver/`: `orch.rs` (tool routing, the `run_status` long-poll, `get_context` and `task_result`), `orch_ops.rs` (the new ops), `wake.rs` (pasting a wake-up into the orchestrator's PTY).
   - `run/git/summary.rs` (a task's commits and diffstat for `task_result`).
   - `launch/role.rs` (the role flags for a PTY run window).
   - `manager/role_window.rs` (`create_run_window`, client-input time).

   **Pure** (M8a decision 2's grep): `run/orch/**`, `run/engine/{orch,planners,kinds,integration}.rs`, `launch/role.rs`. **I/O**: `run/driver/{orch,orch_ops,wake}.rs`, `run/git/summary.rs`, `manager/role_window.rs`. *(M8a decision 2; AGENTS.md rules 2, 5 and 10.)*
2. **Protocol.** `PROTO_VERSION` becomes one above `main`'s value at the start (header). Every addition is listed in Interfaces "proto". New enum variants are appended **last** in their enum, and every new struct field is `#[serde(default)]`, so an M8a–M8c `run.json` and snapshot still deserialize. Every new message and every new variant gets a MessagePack round-trip test (AGENTS.md rule 4). Every client is updated in the same change: the TUI (task M9.15), the CLI (task M9.14) and `anthrex mcp` (task M9.11). *(AGENTS.md rule 4; spec §19 last line.)*
3. **Configuration.** New keys live in a new file, `crates/config/src/orchestrator_agent.rs`, read by one call from M8a's `orchestrator::read`, with M8a's problem format. `crates/config/src/lib.rs` is not touched. Keys, defaults and ranges are in Interfaces "config". `planner_task_cap` is the config key M8b's `run::triage::PLAN_SCALE_MAX` names as M9's; the triage prompt's "2 to 12 tasks" is rendered from it from now on. *(Spec §22.3; M8b Interfaces `run/triage.rs`.)*
4. **Words.** An **epic** is spec §2's planning-only grouping; its id is a `PlanTask.epic` value and a `PlannerInfo.epic`. A **sub-planner** is a headless session with `AgentRole::Planner`, one per epic at a time. The **orchestrator** is the one PTY window with `AgentRole::Orchestrator`. A **hold** is decision 28's approval wait on tasks added after the plan gate. *(Spec §2.)*

### The orchestrator window

5. **The one interactive run window.** The engine creates it with `WindowManager::create_run_window(spec, project, role)` (new, `manager/role_window.rs`; the name is `spec.name`), which runs `create`'s three phases (`admit`, `spawn_window`, `insert`, made `pub(super)` and given an `Option<RoleLaunch>`) with no worktree:
   - `WindowInfo.kind == Pty`, `WindowInfo.run == Some(RunRef { run_id, task_id: None, role: Orchestrator, session: 1 })`;
   - name `<h4>/orchestrator` (`<h4>` is the run id's four hex digits, as M8a decision 16 names run windows);
   - `cwd` is the run's `root`, the user's own checkout, which the orchestrator may read and cannot write (decisions 7, 8). It is never a task worktree.
   - No client message can create one: `ClientMsg::CreateWindow` has no field that sets a role.
   - It counts toward `max_windows`. *(Spec §3, §4 "Only the orchestrator is an interactive terminal", §11.6 "scout, planner: none; they read the main checkout".)*
6. **Choosing its runtime and model.** `RunRequest::StartGoal` gains `orchestrator: Option<OrchestratorChoice { runtime, model: Option<String> }>`, from `--orchestrator <runtime>[:<model>]`. Resolution, `run::orch::launch::resolve_orchestrator(choice, agent, default_runtime, roster) -> Result<Route, String>` (Interfaces):
   1. Runtime: the choice's, else `[orchestrator.agent] runtime`, else `orchestrator.default_runtime`.
   2. Model: the choice's when given (`codex:` with an empty model means Codex's configured default, `""`); it must be in the roster (`<runtime>:<model> is not in the roster`). Otherwise `[orchestrator.agent] model` when non-empty (same check). Otherwise the first roster entry of that runtime at `frontier`, else that runtime's highest-strength entry (with the built-in roster, Codex resolves to `""` at `standard`, and the report says `orchestrator below the frontier tier: codex (default) standard`).
   3. Effort: `[orchestrator.agent] effort`, default `high`.

   `run start --plan` never creates an orchestrator. *(Spec §4 table "frontier tier, high effort"; superseded M9 brief decision 1, adapted to strengths.)*
7. **Claude launch.** `launch::plan` for `Runtime::Claude` with `ctx.role == Some(role)` builds, in this order:
   - M3's `--name <name>` and `--settings <json>` (M3's hook settings, with decision 12's `StopFailure`);
   - the flags in `CLI_CAPS.claude_user_settings_only`, when `Some` (decision 9);
   - `--mcp-config {"mcpServers":{"anthrex":{"type":"stdio","command":"<exe>","args":[…headless::argv::mcp_args(&role.mcp, ctx.window_id, ctx.socket_path)…]}}}`;
   - `--allowedTools mcp__anthrex__get_context,mcp__anthrex__spawn_scout,mcp__anthrex__spawn_subplanner,mcp__anthrex__edit_plan,mcp__anthrex__run_status,mcp__anthrex__task_result,Read,Glob,Grep`;
   - `--disallowedTools Edit,Write,NotebookEdit,Bash,Agent`;
   - `--append-system-prompt <ORCHESTRATOR_CONTRACT>`;
   - `--effort <effort>` when `CLI_CAPS.claude_effort_flag` and M9.1 item 1 finds the flag accepted by the interactive CLI;
   - then M3's `--model <model>` when the route names one, then `--resume <id>` or `--` and the first prompt.

   `--mcp-config`, `--allowedTools` and `--disallowedTools` are variadic; the order guarantees each is followed by another flag, and `--` guarantees the prompt is never read as a tool name.
   - **Read-only by the runtime.** The orchestrator is launched with `Edit`, `Write`, `NotebookEdit` and `Bash` disallowed, not in plan mode: in the interactive TUI, plan mode injects its own instruction to present a plan and call `ExitPlanMode`, which prompts the user and fights the contract. `Agent` is disallowed because a sub-agent the orchestrator starts is a second path to tools; its reading is the scouts' job. Everything not allowed and not disallowed keeps Claude Code's default permission behaviour, which asks the user, who is present. If M9.1 item 2 finds that `--disallowedTools` does not stop a tool call, stop and use `--permission-mode plan` instead, recording the evidence.
   - `strict` MCP: `--strict-mcp-config` is part of `claude_user_settings_only` when M8a.1 found it; if that list is `None`, `--strict-mcp-config` is still passed when `claude --help` lists it (M9.1 item 1), so the orchestrator never loads the repository's `.mcp.json`.

   *(Spec §4 "Read-only roles are launched read-only by the runtime"; user decision: "Claude `--permission-mode plan` or disallowing Edit/Write/Bash"; superseded M9 brief decision 5.)*
8. **Codex launch.** `launch::codex::args` with `ctx.role == Some(role)` inserts, after M3's hook block and before `-m`:
   - `-c mcp_servers.anthrex.command=<toml exe>`, `-c mcp_servers.anthrex.args=<toml array of mcp_args>`, `-c mcp_servers.anthrex.tool_timeout_sec=120`, `-c mcp_servers.anthrex.default_tools_approval_mode="auto"`;
   - `-c developer_instructions=<toml ORCHESTRATOR_CONTRACT>`, `-c model_reasoning_effort=<toml effort>`;
   - the flags in `CLI_CAPS.codex_user_config_only`, when `Some`;
   - `-s read-only`, `-a on-request`.

   Then M3's `-m`, `resume <id>` or `-- <prompt>`. Every TOML string comes from `launch::codex::toml_string`. M9.1 item 4 verifies that `-s` and `-a` are accepted ahead of `resume`, as `-m` and `-c` are; if not, they move after `resume <id>`. *(Spec §4; superseded M9 brief decision 6.)*
9. **Only the user's settings load.** Both launches carry M8a decision 53's exclusion flags when the CLI has them. When `CLI_CAPS.claude_user_settings_only` is `None` (or Codex's `codex_user_config_only` is `None` while `codex_loads_project_config`), the project-settings check that M8b decision 22 already runs for `StartGoal` covers the orchestrator too: the run is refused without `--trust-project`, and with it the orchestrator starts and `Run.trusted_project` records the files. `run promote` repeats the check against the run's base commit (decision 29), honouring the `trust_project` the run started with. *(Spec §4 "Only the user's own settings load"; M8a decision 53.)*
10. **Environment.** `LaunchPlan` gains `scrub_agent_env: bool` (false for every window except run windows created by `create_run_window`). `Window::spawn` then removes every inherited variable matching M8a's `headless::session::SCRUB_PREFIXES` and `SCRUB_NAMES` before setting `plan.env`, so a daemon started from inside a Claude session does not make the orchestrator a nested session. `RoleLaunch.env` adds, after M3's four variables:
    - for Claude, `metering::orchestrator_env(addr, run_id)` when `<data_dir>/otlp.addr` exists (read by the driver when it builds the op), with `anthrex.role=orchestrator` in its resource attributes (M8b decision 30);
    - `MCP_TOOL_TIMEOUT=120000` for Claude, when M9.1 item 3 finds that Claude Code's default MCP tool timeout is below 120 s (the long-poll's 50 s plus `task_result`'s git reads must never be cut off).

    *(Spec §14.8, §17 "Scrubbed agent environment".)*
11. **Persistence, restart and what the daemon refuses.**
    - `launch::role::RoleLaunch` (Interfaces) is the orchestrator's whole role: its `RunRef`, `McpTarget`, contract, tool lists, effort and environment. It is kept in `Entry.role` and persisted in `WindowRecord.run` as `{"role_launch": <RoleLaunch>}` for a `Pty` record (M8a stores a `HeadlessSpec` there for a `Headless` record; the kind decides the parse). A value that fails to parse restores the window as a plain PTY window with a warning.
    - `manager::restart` copies `Entry.role` into `ForRelaunch.role` and passes it to `launch::plan`, so a restart — the user's `anthrex restart <id>`, or decision 13's — re-passes every role flag with `--resume <session id>` (Claude) or `resume <id>` (Codex). Resume restores none of them by itself (spec §17).
    - A client may `Subscribe`, send `Input`, `Resize`, `Rename` and `Restart` for the orchestrator window. `Kill` and `Remove` are refused while its run is not terminal, with `DaemonMsg::Error { request: "kill" | "remove", message: "window <id> is the orchestrator of run <run>; stop the run with anthrex run cancel, or restart the orchestrator with anthrex restart <id>" }`, by the same helper M8a's decision 49 added (`server/headless_guard.rs`). Once the run is accepted, discarded or failed, the window is an ordinary PTY window again: nothing is refused, and M8c already lists it as a plain window.
    - **After a daemon restart** the window is restored dormant, like every PTY window (M6). `anthrex run resume <run>` issues `OpKind::RestartOrchestrator` for a dormant orchestrator of a paused run, and also for a run in `awaiting_approval`, whose state it leaves unchanged. The first wake-up after a restart carries the note `the daemon restarted and your session was resumed` (decision 39).

    *(Spec §4.2 "Kill and remove are refused for run sessions", §17.)*
12. **The status gap.** `launch::claude::HOOK_EVENTS` gains `StopFailure` (11 events), `hooks::HookKind` gains `StopFailure`, `parse` maps `"StopFailure"` to it, and it maps to `StatusEvent::Stop`: a turn that ends on an API error is over, and the window goes idle instead of staying `Working`. This applies to every Claude PTY window, which is correct for all of them. `PreCompact` and `PostCompact` are not added: compaction runs inside a turn that still ends with `Stop` or `StopFailure`. If M9.1 item 5 finds a manual `/compact` that leaves the window `Working`, add both and map them to no status event, recording it. *(Spec §11.1 "`StopFailure` replaces `Stop` on API errors"; M8a Out table and follow-up.)*
13. **When the orchestrator is not there.** The engine never waits for it. If its window exits (the driver sees `Exited` in the manager's window list and sends `EventKind::OrchestratorWindow { live: false }`), the run keeps executing, wake-ups are suspended, and the run's attention list gets `the orchestrator (window <n>) exited; restart it with anthrex restart <n>`. Everything the orchestrator does is also available to the user: `anthrex run edit` for every plan edit, `anthrex run approve|reject` for the gate and holds, `retry`, `override`, `cancel`, `finish`. *(Spec §12.4 "The same edits are available without it as `anthrex run edit …`".)*
14. **Metering.** A Claude orchestrator is metered through M8b's OTLP receiver (decision 10), and its usage lands in `Run.orchestrator_usage` through M8b's `EventKind::OrchestratorUsage`. A Codex orchestrator is not metered: `RunInfo.usage.by_role["orchestrator"]` stays zero and the report says `orchestrator usage: not metered (codex)`. Recorded as a follow-up. *(Spec §14.8, which names Claude Code's OTLP export only.)*

### Tools

15. **Tool lists and the rules every tool follows.** `mcp::tools_for(Orchestrator)` is `get_context`, `spawn_scout`, `spawn_subplanner`, `edit_plan`, `run_status`, `task_result`, in that order; `tools_for(Planner)` is `get_context`, `submit_epic`. Schemas and texts are in Interfaces "MCP".
    - **Nothing blocks on the user.** Every tool answers within M8a's `TOOL_REPLY_TIMEOUT` (100 s) and Codex's `tool_timeout_sec` (120 s): `run_status` waits at most 50 s, `task_result` makes at most two git reads bounded by `DONE_CHECK_GIT_TIMEOUT` (10 s) each, and everything else answers from memory. The plan gate's verdict is read, never awaited (decision 30).
    - **Authorization.** `anthrex mcp` refuses a tool outside its role's list without reaching the daemon (M8a). The daemon then checks the calling window: an orchestrator call must come from the run's orchestrator window (`this window is not the orchestrator of run <id>`), a planner call from the live session of its epic's sub-planner (`this window is not the sub-planner of epic <e> of run <id>`).
    - **Routing.** `ToolCall` gains `epic: Option<String>`. `RunService::request(Tool)` sends `get_context`, `run_status` and `task_result` to the driver's read path (decision 16–18), and every other orchestrator or planner tool to the reducer as M8a's `EventKind::Tool`, which dispatches by role to `run/engine/orch.rs` and `run/engine/planners.rs`.
    - **Results** are one text content item holding JSON. A refusal is `ToolResult { ok: false }` with a JSON object `{"error": "<text>"}`, except a rejected plan edit, which is decision 19's error list.

    *(Spec §19; M8a decision 4 and Interfaces "MCP tools"; user decision on MCP timeouts.)*
16. **`run_status`, the long-poll.** Arguments `since` and `wait_secs` (0–50, default 0). It returns the digest (Interfaces "The digest").
    - **The revision it waits on** is `Run.digest_rev`, not M8a's `revision`. After every reducer step that changed a run, `run::orch::digest::fingerprint(&run)` (FNV-1a 64 over the digest's JSON with every counter removed: tool calls, tokens, spend, seconds, `now`, round activity) is compared with `Run.digest_fp`; only a different fingerprint bumps `digest_rev`. So a worker's tool-call counter never wakes a waiting orchestrator; a task changing state, a block, a verdict, a hold, a scout or planner finishing, or an edit does.
    - **Waiting.** With `wait_secs == 0`, or `since` absent, or `since != digest_rev`, it answers at once. Otherwise the driver waits on M8a's snapshot `watch` until the run's `RunInfo.digest_revision` differs from `since` or `wait_secs` elapses, and answers with the current digest either way. No engine lock is held while waiting; the run is cloned under the lock for the answer, then the lock is released before the digest is built.
    - **Read receipt.** After answering, the driver sends `EventKind::DigestRead { run_id, digest_revision }`; the reducer drops the pending wake notes up to that revision (decision 39), since the orchestrator has now seen them.
    - A run that becomes terminal while a wait is open ends the wait at once.

    *(Spec §19 "`run_status` (long-poll up to 50 s)", §14.4 "structured and small".)*
17. **`get_context`.** Returns Interfaces "The context": the run and its triage, who is asking (the orchestrator, or a sub-planner and its epic), `profile::summary` and the profile's glob lists and commands, the limits (`planner_task_cap`, `max_tasks`, slots, `max_bounces`, `max_scouts`, the S and M rubric text of spec §7.1), the roster with strengths, notes and `installed`, the run's scout reports, the epics and the plan so far.
    - **Installed** means the runtime's configured binary (M3's `claude_bin`, `codex_bin`) resolves to an executable file directly or on `PATH`; checked once per run on `spawn_blocking` when the run is built, kept in `Run.installed`.
    - **For a sub-planner**, reports are filtered to its epic's `scout_refs`, the reports whose scout area intersects its epic's area (M8a decision 11's intersection), and the onboarding report; the plan lists tasks with no epic and its own epic's tasks.
    - **Bounds.** Each report summary is cut to 8000 characters; `scouts: [ids]` limits the answer to those reports. When the JSON passes `CONTEXT_MAX_BYTES` (96 KiB), later reports' summaries are cut to 1000 characters, then their file lists dropped, and `omitted` says how many.

    *(Spec §19 "`get_context` (roster with strengths, profile, scout reports)"; superseded M9 brief decisions 24–25 for `installed`.)*
18. **`task_result`.** Everything about one task (Interfaces "The task result"): the task record, its done claim, its checks (with the M8b summary), proofs, reviews with findings, agent rounds, its research report, its history, and two git reads the driver makes on `spawn_blocking` in `root` against the task branch `anthrex/<run>/<task>`, which M8a keeps until accept or discard: `git log --format=%h%x1f%s -n 50 <start>..<branch>` and `git diff --stat=100 <start>...<branch>` (60 lines at most). A task with no start commit has neither. At most `TASK_RESULT_MAX_BYTES` (64 KiB). This is how the orchestrator knows a task's work without reading a checkout. *(Spec §3 "reads the run digest, never transcripts or checkouts", §19.)*
19. **`edit_plan`.** `{ edits, submit?, summary? }`.
    - **One batch.** `edits` are M8a's `PlanEdit`s, applied with `run::edits::apply_edits` under `EditScope::Run`, then checked with decision 23's rules with source `EditSource::Orchestrator`. Any error rejects the whole batch and changes nothing; the reply is `ToolResult { ok: false }` with `{"accepted": false, "errors": [{"task": "t2" | null, "field": "…", "rule": "…", "message": "…"}]}`, built from `PlanError`s in order.
    - **Accepted:** `{"accepted": true, "revision": <digest_rev>, "awaiting_approval": <bool>, "notes": [<task notes added by this batch>], "held": <hold id> | null}`. `awaiting_approval` is true while the run is in `awaiting_approval` or the batch's tasks landed in a hold that is waiting.
    - **`submit: true`** is decision 27.
    - **`summary`** (1–8000 characters) is stored as `Run.orchestrator.summary` and written at the top of `REPORT.md` under `## Summary from the orchestrator`; the last one wins. On a `complete` run, a call with `edits: []` and a `summary` is accepted; any other edit is refused with `edits are not accepted on a complete run; only a summary is`.
    - **It cannot approve.** `PlanEdit` has no approve or override operation, so `{"op": "override", …}` fails to parse (`invalid arguments: edits[0]: unknown variant \`override\``), and nothing in the orchestrator's tools reaches M8a's `Override`, `Approve` or `Accept`. The `finish` edit ends the run as M8a decision 37 says; it approves nothing.
    - Every accepted batch is appended to `Run.edit_log` with source `orchestrator` (decision 40).

    *(Spec §12.1, §11.4 step 6 "cannot approve a task", §5.3 step 7.)*
20. **`spawn_scout`.** `{ id, question, area, web? }` starts a run scout on M8b's `ScoutService`.
    - **Ids.** The given id must match `^[a-z0-9][a-z0-9-]{0,31}$`; the scout's full id is `<h4>-<id>`, which is what the reply, the digest, `get_context` and `scout_refs` use, so two runs' scouts never collide in `ScoutService` or in `<data_dir>/runs/<run>/scouts/`.
    - **Slots.** A run scout takes one of the run's reader slots while it runs (spec §13 item 3). The reducer queues it (`RunScout { state: Queued }`) and emits `OpKind::StartScout` when a slot is free; the reply is at once: `{"scout_id": "<full id>", "state": "queued" | "starting"}`.
    - **The session** is M8b decision 12's area scout: `ScoutSpec { id, kind: Area, run_id: Some(run), question, first_turn, cwd: root, project, web }` with `first_turn = scout_first_turn(run, id, area, question)` (Interfaces).
    - **Its end.** The driver awaits the `ScoutHandle`'s outcome and sends `EventKind::ScoutEnded { run_id, scout_id, outcome, usage }`. A report pushes its id into `Run.scout_reports` (M8b decision 19 then uses it for the size cross-check) and adds its usage to `Run.scout_usage`.
    - **Limits.** At most `max_scouts` per run (`run <id> already has <n> scouts, the most max_scouts allows`); a used id is refused (`scout <id> already exists in run <run>`).
    - **In the snapshot.** `RunInfo.scouts` is M8b's `ScoutService::run_scouts(run_id)`, M8b's `ScoutInfo` with M8b's `ScoutState` (`starting`, `working`, `reported`, `failed`), which is what M8c draws. A `Queued` run scout has no `ScoutInfo` until `StartScout` runs; it shows only in the digest, whose `scouts[].state` uses this milestone's `RunScoutState` labels (`queued`, `running`, `reported`, `failed`).
    - **After a daemon restart** a run scout is not resumed: on `Restore`, every `Queued` or `Running` run scout becomes `Failed { "the daemon restarted during this scout" }`, and M8a's reconcile kills a leftover process by its session id. Scouts write nothing but their report, so repeating one is cheap; the digest shows the failure and the orchestrator may spawn it again (M8b decision 11's reasoning, applied to run scouts).

    *(Spec §5.3 step 1, §14 item 1, §19; M8b "Produces for later milestones" rows for M9.)*
21. **`spawn_subplanner`.** `{ epic, title, area, brief, scout_refs? }`.
    - **New epic.** `epic` matches `^[a-z0-9][a-z0-9-]{0,15}$`. Each `area` glob passes `validate_glob` and is a literal path or `<literal>/**` (M8a decision 12's area form); the area must not intersect another epic's (`epic <e>: area: overlaps epic <f>'s area (<glob>)`). An `EpicRecord` is added and its planner queued for a reader slot (decision 31). The run's `path` becomes `Large`.
    - **Re-plan.** For an existing epic whose planner is `Finished` or `Failed`, it starts a **fresh** planner session for that epic (never the old session), with the new `brief` as the re-plan request, the epic's current tasks in its prompt, and an entry in `replans` (the brief's first 40 characters). A live planner refuses it: `epic <e> is being planned by its sub-planner; wait for it to finish`.
    - **When** allowed: in `planning`, `awaiting_approval` (which returns the run to `planning`, decision 27) and `running`. A new epic in a `running` run gets a hold (decision 28).
    - Reply at once: `{"epic": "<e>", "state": "queued" | "planning", "hold": <id> | null}`.

    *(Spec §5.1 "Large", §3 "one per epic … add tasks then exit", §7.3, §12.3 "any later edit that adds an epic".)*
22. **`submit_epic`.** `{ edits, note? }`, from the epic's live planner only.
    - **Scope.** Applied with `EditScope::Area { globs: epic.area }` (M8a decision 12 checks every `owns` glob is inside) and decision 23's rules with source `Planner { epic }`.
    - **Operations.** `add_task`, `add_dep`, `amend_task`, `split_task` and `cancel_task`, each only on tasks of its own epic (`add_dep`'s `dep` may name any task). `answer`, `pause`, `resume` and `finish` are refused: `op <op> is not available to a sub-planner`. An added task with no `epic` gets the planner's epic; another epic is refused (`task <id>: epic: a sub-planner adds tasks only to its own epic <e>`).
    - **Accepted** ends the planner: the reply is `Epic recorded. You are done; end your turn now.`, the planner becomes `Finished`, and its session is retired as M8a decision 52 retires a reviewer. `note` (1–2000 characters) is kept on the epic and shown in the digest, for what the planner needs from the orchestrator (for example an interface task it found missing).
    - **Rejected** returns decision 19's error list, counts `edits_rejected` and sets `last_rejection` to the first error's message. After `[orchestrator.planners] max_rejections` rejected submits the planner fails (`the sub-planner's epic was rejected <n> times`).
    - A second submit after acceptance is refused: `submit_epic was already accepted for epic <e>`.

    *(Spec §3, §12.1 "`owns` inside the epic's area for a sub-planner", §19.)*

### Plan rules

23. **Rules for planners' edits**, in `run::orch::rules::check(run: &Run, touched: &BTreeSet<String>, source: &EditSource) -> Vec<PlanError>`, run after M8a's validation on every batch from the orchestrator (`edit_plan`) or a sub-planner (`submit_epic`). Plan files and the user's `run edit` keep M8a's rules only: a user's own plan is theirs.
    1. **No budgets.** A touched task with `budget` set: `task <id>: budget: budgets come from the task's size; leave budget out (rule 7.1)`. Size from evidence, never time (spec §7).
    2. **Scout evidence.** A touched `code` or `docs` task must name at least one `scout_refs` entry when the run has any finished scout report (run scouts or the `onboarding` alias of M8b): `task <id>: scout_refs: name the scout reports this task's size rests on (rule 7.1)`. Every entry must be a finished report of this run or `onboarding`: `task <id>: scout_refs: <ref> is not a finished scout report of this run`. A run with no report at all gets the note `size not backed by a scout report` on the task instead.
    3. **The cap.** Tasks with no epic count toward the orchestrator; tasks of epic `e` toward `e`'s sub-planner. Unfinished and finished tasks both count; cancelled ones do not. Over `planner_task_cap`: `tasks: the orchestrator's plan has <n> tasks, more than planner_task_cap (<cap>); plan the rest through sub-planners (rule 5.1)`, or `tasks: epic <e> has <n> tasks, more than planner_task_cap (<cap>); split the epic (rule 5.1)`.
    4. *Removed 2026-09-22 (chain collapse is the planner's judgement; spec §7.3). The engine does not check chains; contract rule 13 and the planner's rule 4 carry it as guidance.*
    5. **Epics exist.** A task whose `epic` names no epic of the run: `task <id>: epic: <e> is not an epic of this run; create it with spawn_subplanner`. The orchestrator may add a task to an epic whose planner has finished (a fix task after an integration review); while the epic's planner is live, `task <id>: epic: epic <e> is being planned by its sub-planner`.

    Rule ids in `PlanError.rule`: `7.1.budget`, `7.1.evidence`, `5.1.cap`, `2.epic`. *(Spec §7.1, §7.3, §12.1, §22.3.)*
24. **Kinds `research` and `review` are executed.** M8a decision 6's refusal is removed (its test `research_and_review_kinds_are_deferred` is replaced by the ones below). In M8a's `run/validate.rs`, for every source:
    - A research or review task must have an empty `owns` (`task <id>: owns: research and review tasks change nothing; leave owns empty (rule 5.2)`); M8a's `owns_required` applies to `code` and `docs` only. With no `owns`, these tasks never take part in implicit dependencies, the hub rule, the source rule or the cross-runtime rule.
    - Their test mode is forced to `none` with the note `test mode none: research and review tasks change nothing (rule 8)`; a given `test_mode_reason` is kept, none is required.
    - `PlanTask` gains `review_target: Option<String>`. A review task requires it (`task <id>: review_target: required for a review task (rule 5.2)`); any other kind refuses it (`task <id>: review_target: only review tasks have a review target`). Its syntax is one revision, or `<a>..<b>`, each part matching `^[A-Za-z0-9._/@^~-]{1,200}$` and not starting with `-` (`task <id>: review_target: <t> is not a revision or a range <a>..<b>`). Whether it resolves is checked at dispatch (decision 36).
    - Size is still required (it sets the budget); the S and M rules about modules do not apply to an empty `owns`.

    *(Spec §5.2, §8 table "none for research and review tasks", §21.)*
25. **Editing blocked work.** Two additions to M8a decision 13, for every source:
    - **`amend_task` gains `deps: Option<Vec<String>>`**, allowed on `pending`, `queued` and `blocked` tasks, validated like `add_dep` (every id exists, no cycle, no cancelled dependency). A `blocked(dep_cancelled)` task whose new deps name no cancelled task returns to `pending`. This is how a planner replaces a cancelled dependency (spec §12.2 "every task depending on it becomes `blocked(dep_cancelled)` for the orchestrator to re-plan").
    - **Rewriting a mis-sized task restarts it.** An accepted `amend_task` that changes `brief`, `acceptance`, `size` or `route` of a task in `blocked(mis_sized)` re-enters it exactly as M8a decision 42's retry does (a fresh session at rung 2, `failures = 1`), once the batch is applied. The amended size may be `S` or `M` again: rung 3 raised it because the old task did not fit, and the rewrite is the planner's new claim; M8a's size rules then apply to the new values and can raise it again. A `blocked(human)`, `blocked(conflict)` or `blocked(environment)` task is never restarted by an edit — only by the user's `anthrex run retry`, because rung 4 is a spend ceiling the user must lift, and a conflict or a broken environment is not something a new brief fixes.

    *(Spec §10 rung 3 "the orchestrator … splits or rewrites it", rung 4, §11.4 step 5.)*

### Planning and the plan gate

26. **Goals on the plan and large paths start.** M8b decision 22 step 6 refused them; now `RunService::request(StartGoal)` builds a run for `TriageRoute::Plan` and `TriageRoute::Large`:
    - `run::orch::build_planned_run(goal, pre, ctx, triage, route, installed)` (Interfaces) calls M8a's `build_run` with a `Plan` whose task list is empty (M8a's validation accepts an empty `tasks`; if its merged code refuses one, the wrapper builds the `Run` through the same resolution and skips that one check, recorded under "Implementation notes"), then sets `state = Planning`, `path` from the route, `triage`, `orchestrator: Some(OrchestratorRecord)` with the route of decision 6, `yes` from `ctx.yes`, and `installed`.
    - `EventKind::Start` of a `Planning` run emits M8a's `CreateRunBranch` (the run branch and integration worktree exist from the start, M8a decision 14), then `OpKind::CreateOrchestrator`. No task, no pre-warm and no gate yet.
    - The reply is M8b's `RunReply::Triaged { triage, run_id: Some(id), message }` with `message = planned_message(&info, id, path)` (Interfaces).
    - `RunState::Planning` (label `planning`) is new and last in its enum. On `Restore` a `Planning` run becomes `Paused` with `paused_from = Planning`, like `Running` (it has live agents); `run resume` returns it to `Planning`, restarts the orchestrator (decision 11) and resumes live planner sessions (decision 32).
    - `run approve` of a `Planning` run is refused: `run <id> is still being planned; approve it when the orchestrator has submitted the plan`. `run reject` discards it (M8a decision 20).

    *(Spec §5.1 table rows "Plan" and "Large"; M8b decision 22, "Produces for later milestones" row `RunRequest::StartGoal`.)*
27. **Submitting the plan.** `edit_plan { submit: true }` (after its edits, in the same batch):
    - In `planning`: refused with `the plan has no tasks yet; add tasks before submitting` when no unfinished task exists, and with `sub-planner <e> is still planning; submit when every sub-planner has finished` while any planner is `Queued` or `Planning`. Otherwise, with the run's `yes`, the run goes to `running` with `approved_by = "--yes"` and `approved_at` set (M8c); without it, to `awaiting_approval`, and M8a decision 14's pre-warm starts. `OrchestratorRecord.plan_submitted = true`.
    - In `awaiting_approval`: accepted with no change of state (a resubmission after edits).
    - In `running` with a promotion hold in `Drafting` (decision 29): the hold becomes `Awaiting`.
    - Otherwise `submit` is ignored and the reply's `awaiting_approval` is false.
    - A `spawn_subplanner` while `awaiting_approval` returns the run to `planning` (`plan_submitted = false`); pre-warmed worktrees stay. Other edits in `awaiting_approval` are applied at once and seen by the user live in the run view (M8a decision 14 already allows edits while the gate is open).

    *(Spec §5.3 step 3, §12.3.)*
28. **Holds: approval for work added after the gate.** Spec §12.3 makes "any later edit that adds an epic" wait for approval without blocking a tool call or the running work.
    - `Task.hold: Option<String>` names a `HoldRecord` of the run. A task whose hold is not `Approved` is not runnable (added to M8a decision 41's runnable test) and is not pre-warmed.
    - **Epic holds.** A `spawn_subplanner` of a **new** epic on a `running` run creates hold `epic:<e>` in `Drafting`; every task its planner adds carries it; when its `submit_epic` is accepted the hold becomes `Awaiting`, or `Approved` with `decided_by = "--yes"` when the run was started with `--yes`.
    - **Verdicts.** `RunRequest::ApproveHold { run_id, hold }` makes it `Approved` (`decided_by = "user"`) and its tasks runnable. `RunRequest::RejectHold { run_id, hold }` makes it `Rejected` and cancels its tasks (none has started, so nothing is salvaged; dependents become `blocked(dep_cancelled)` as M8a decision 13 says). Both answer `RunReply::Done`, or `Refused` with `run <id> has no hold <hold>` or `hold <hold> is <state>`. `anthrex run approve <run> --hold <hold>` and `anthrex run reject <run> --hold <hold>` send them.
    - **Completion** waits while any hold is `Drafting` or `Awaiting` (decision 38).
    - The run view shows an `Awaiting` hold's tasks as not yet approved and lets the user approve or reject it (task M9.15).

    *(Spec §12.3 "any later edit that adds an epic … Non-blocking".)*
29. **`run promote` promotes.** M8b recorded the request; M9 acts on it. `EventKind::Promote` on a fast-path run that is not terminal and has no orchestrator:
    - runs decision 9's project-settings check (in the driver, before the event, on `spawn_blocking`, against the run's `base_sha`), resolves the orchestrator route (decision 6, `run promote <run> [--orchestrator …]`), sets `path = Plan`, `promote_requested_at = now`, `orchestrator = Some(..)` with `plan_submitted = false`, and emits `OpKind::CreateOrchestrator` with `promoted_first_prompt`;
    - replies `Done` with `promoted: run <id> now has an orchestrator; it starts in a moment (anthrex run status <id>)`;
    - leaves the fast-path task `t1` exactly as it is: it keeps running through its gates.
    - While `plan_submitted` is false, every task added to the run (by the orchestrator, or by a planner it starts) carries hold `promotion`, created `Drafting` on the first such task. `submit` makes it `Awaiting`; the user approves it with `anthrex run approve <run> --hold promotion`. After that, only new epics are held.
    - **Requests recorded before M9.** On `Restore`, a non-terminal fast-path run with `promote_requested_at` set and no orchestrator is promoted as above when it is next resumed or, if it is `running`, on the first `Tick`.
    - An already promoted run: `Done` with M8b's `run <id> was already marked for promotion at <hh:mm>`. A plan-file or planned run: M8b's `run <id> is not a fast-path run`.

    *(Spec §5.1 "`anthrex run promote` turns it into a planned run at any time"; M8b decision 25.)*
30. **The verdict reaches the orchestrator through `run_status`.** The digest's `gate` object (Interfaces) says `planning`, `awaiting_approval`, `approved` (with who and when), and each hold's state. An approval, a rejection of a hold, and every user edit made while the gate is open or after it (`run edit`, the run view's `e` and `d`) change the digest, so a waiting `run_status` returns, and each also adds a wake note (decision 39). A rejected initial plan discards the run (M8a decision 14): the digest's `run.state` becomes `discarded`, every later mutating tool answers M8a's `run <id> is discarded`, and the orchestrator window becomes a plain window (decision 11). *(Spec §12.3 "the verdict arrives through `run_status`", replacing conversation-view decision 12.)*

### Sub-planners

31. **Sub-planner sessions** are M8a headless sessions (`WindowManager::create_headless`), built by `run::orch::launch::planner_spec(run, epic, session) -> HeadlessSpec`:
    - **Route.** `[orchestrator.planners] runtime` (default: the orchestrator's runtime), strength `frontier`, effort `high`, resolved with M8b's `scout::spec::route`.
    - **Read-only**, like M8b's area scouts: `cwd` is `root`; `instructions` is `PLANNER_CONTRACT`; `allowed_tools` is `mcp__anthrex__get_context`, `mcp__anthrex__submit_epic`, `Read`, `Glob`, `Grep`; `claude_permission_mode = Some("plan")`, or M8a decision 24's reviewer fallback when M8a.1 found that plan mode blocks an allowed MCP call in `-p`; `codex_sandbox = "read-only"`; no sandbox block, no output filter.
    - `mcp = McpTarget { role: Planner, run_id, task_id: None, scout_id: None, epic: Some(e) }`; `mcp_args` appends `--epic <e>`; `run_ref = RunRef { run_id, task_id: None, role: Planner, session: n }`; window name `<h4>/plan-<e>.p<n>`.
    - **A reader slot** is held while the session is live. Spec §13 item 3 lists scouts, reviewers and deciders; a sub-planner is the same kind of read-only reader, and without a slot a large goal would start every planner at once. When a slot frees, M8b's order extends to: deciders, then reviewers (task and integration), then sub-planners, then run scouts, then research and review tasks.
    - **First turn:** `planner_prompt` (or `replan_prompt`), Interfaces. The prompt layout follows spec §14.2: the fixed contract as the system prompt, then the prompt, the request last.
    - They are unattended: the user watches them through the conversation view and cannot type to them (M8a decision 49's refusals apply unchanged). *(Spec §4 table row "Sub-planner", §4.2.)*
32. **Sub-planner lifecycle**, in `run/engine/planners.rs`, reusing M8a's per-round machinery (`AgentRound`: turns, the stall watchdog, deaths, rate limits, delivery) through a round owner of `Planner { epic }` next to M8a's task rounds:
    - A turn that ends without an accepted `submit_epic` sends `PLANNER_NUDGE` once; a second such turn fails the planner (`the sub-planner ended two turns without an accepted epic`).
    - A process death mid-turn resumes once (M8a decision 32's rule); a second, or a second stall, fails it.
    - At `[orchestrator.planners] max_tool_calls` tool calls it gets `planner_wrap_up` once; at 1.5 times that it is killed and failed (`the sub-planner used <n> tool calls without an accepted epic`). After `timeout_secs` it is killed and failed (`the sub-planner ran longer than <n> s`).
    - `max_rejections` rejected submits fail it (decision 22).
    - A failed planner's epic keeps any tasks already accepted from an earlier session; the orchestrator is woken (decision 39) and may start a fresh one.
    - **After a daemon restart**, a live planner session is resumed like a reviewer's (M8a decision 45), with `RESUME_PLANNER`.
    - **Finished planner nodes stay** (spec §16.2); its window is retired after acceptance (M8a decision 52's `RETIRE_AFTER`).

    *(Spec §3 "a sub-planner exits once its tasks are accepted; it never relays results", §12.1.)*
33. **`RunInfo.planners`** (M8c's placeholder) is filled by `run/snapshot.rs` from `Run.epics`, one `PlannerInfo` per epic: `epic`, `title`, `area`, `route`, `window_id` of the latest session, `state` (`Planning` while queued or live, `Finished`, `Failed`), `started_at` of the first session, `ended_at` of the last, `edits_accepted`, `edits_rejected`, `last_rejection`, `replans`. *(M8c "Consumes from later milestones".)*

### Scouts and briefs

34. **Scouts feed every brief in their area.** M8a's `worker_prompt` gains a scout extract between the profile summary and the brief (spec §14.2: contract, profile summary, scout extract, brief last): for each of the task's `scout_refs` that resolves to a report (M8b's `scout::report::resolve_ref`), `Scout report <id>:` then its summary cut to 4000 characters and `Files: <paths>`; at most 12 KiB in all, later reports cut first with `[anthrex] … cut …`. Reports are read by the driver when it builds the `CreateWindow` op and passed in `OpKind::CreateWindow.first_turn`, so the reducer stays pure; a report that cannot be read is left out with a log warning. The same extract goes into `planner_prompt` for the epic's `scout_refs`. *(Spec §5.3 step 1 "Their reports feed every planner and every worker brief in that area", §14 item 1.)*

### Research, review and integration

35. **Research tasks** are scout sessions run by the engine as tasks.
    - **Dispatch.** A runnable research task takes a reader slot (not a writer slot) and gets no worktree and no branch. Its session is `run::orch::launch::research_spec(run, task)`: M8b's area-scout launch (read-only, `SCOUT_CONTRACT`, `submit_scout_report`, `Read`, `Glob`, `Grep`, `WebFetch`, `WebSearch`), `cwd` = `root`, route = the task's resolved route, `mcp = McpTarget { role: Scout, run_id, task_id: Some(t), scout_id: None, epic: None }`, `run_ref = RunRef { role: Scout, task_id: Some(t), session: n }`, window name `<h4>/<t>.s<n>`. First turn: `research_prompt`.
    - **The report.** `submit_scout_report` from a task-bound scout window (a `ToolCall` with `role == Scout`, `task_id` set, `scout_id` unset) goes to the engine, not to `ScoutService`: M8b decision 15's routing gains that one branch, and `anthrex mcp --role scout` accepts `--task` in place of `--scout`. It is validated with M8b's `scout::report::validate(args, ScoutKind::Area)`, stored on the task (`Task.research`), and the task becomes `TaskState::Reported` (new, last in its enum, label `reported`, `is_finished` true). The session is retired.
    - **Failures.** A turn without a report gets M8b's `SCOUT_NUDGE` once; a second is `blocked(environment)` with `the research task ended two turns without a report`. Budgets, stalls and deaths follow M8a's worker rules.
    - **Dependents.** A dependency is satisfied by `merged` or `reported` (M8a decision 41's runnable test).
    - **Output.** `REPORT.md` gains `## Research` with each report, and the driver writes the combined text to `<data_dir>/runs/<run>/research.md` whenever it writes the report; `RunInfo.research_report` names it. `anthrex run accept` prints `research report: <path>`. A run that merged nothing is accepted as M8a decision 20 accepts any run: `git merge --no-ff` of a run branch equal to the base does nothing, and the branches are deleted.

    *(Spec §5.2 "research — scout tasks only. Each writes a report into the run's data directory. There is no task branch, no merge, and `run accept` shows the combined report".)*
36. **Review tasks** are M8a reviewer sessions run as tasks.
    - **Dispatch.** A runnable review task takes a reader slot. The engine first resolves its target with `OpKind::ResolveTarget { root, target }` (`git rev-parse --verify <rev>^{commit}` for each side through `run_git`; a single revision `r` means the range `merge-base(<base branch>, r)..r`) → `OpResult::Target { base, head }`, or `blocked(environment)` with `review target <t> does not resolve: <stderr tail>`. Then M8a's `PrepareReview { root, head_ref: head, base_ref: base, path: <wt>/runs/<run>/<t>.review }` and a reviewer session: `REVIEWER_CONTRACT`, `submit_review`, level by size (`S` → `small`, `M` → `medium`), route from the task (policy fills it as for any task), `RunRef { role: Reviewer, task_id: Some(t), session: n }`. First turn: `review_task_prompt`.
    - **The verdict.** The first accepted `submit_review` is stored in `Task.reviews` and the task becomes `Reported`, whatever the verdict: nothing is merged and nothing is sent back. Findings of every severity go to `REPORT.md` under `## Review findings`. A turn without a verdict follows M8a decision 35's nudge rule; two verdict-less rounds are `blocked(environment)`.

    *(Spec §5.2 "review — reviewer tasks against the named branch or range, producing findings. No merge.")*
37. **The per-epic integration review** (large path).
    - **When.** When every task of an epic is finished, at least one merged, no planner of the epic is live, and no integration round of the epic is running, the engine starts round `n + 1` if the last round is absent or older than the epic's last merge. The epic's merges are recorded as they happen (`EpicRecord.merges`: task id and merge commit), and `EpicRecord.base` is the run head just before its first merge.
    - **The session.** A reviewer at level `frontier` in a reader slot, route `roster::pick_reviewer(roster, &author, Frontier)` where `author` is the route of the epic's most recently merged task; review worktree `<wt>/runs/<run>/<e>.epic` detached at the run head (M8a's `PrepareReview`; a task id cannot contain `.`, so the path never collides); `McpTarget { role: Reviewer, task_id: None, epic: Some(e) }`; `RunRef { role: Reviewer, task_id: None, session: n }`; window name `<h4>/epic-<e>.r<n>`. First turn: `integration_review_prompt`.
    - **The verdict** is stored in `EpicRecord.integration`. `approve` (or only minor findings) sets the epic's integration state to `Approved`. A blocking verdict sets `Changes` and holds completion (decision 38) until the orchestrator adds a task to that epic (which, once merged, triggers the next round) or ends the run with the `finish` edit, which completes it with the findings in the report. After `max_bounces + 1` rounds no new round starts; the hold stays until `finish`. Nobody approves the epic by edit: review authority stays with reviewers (spec §11.4 step 6).
    - The plan path has no epics and no integration review; the final check on the run head is M8a decision 37's.

    *(Spec §5.3 step 6.)*
38. **Completion with an orchestrator.** M8a decision 37's condition gains, for a run with an orchestrator: no hold `Drafting` or `Awaiting`; no epic whose planner is queued or live; no run scout queued or running; no integration round running; no epic with integration state `Changes` (unless the `finish` edit was accepted); and `plan_submitted`. Blocked tasks keep the run running, as in M8a. On completion the orchestrator gets the wake note `the run is complete; write your summary with edit_plan summary`. *(Spec §5.3 steps 6–7.)*

### Reacting and waking

39. **Waking an idle orchestrator.** The digest is pulled (spec §14.4), but an interactive session whose turn has ended pulls nothing, so the engine wakes it.
    - **Notes.** Each of these adds one line to `OrchestratorRecord.notes` (at most 20; the oldest are replaced by `+<n> earlier changes`): a task becoming `blocked` (`<t> blocked (<reason>): <text, first 120 characters>`); a gate verdict or hold verdict (`the user approved the plan`, `the user approved hold <h>`, `the user rejected hold <h>`, `the user rejected the plan; run discarded`); a user edit (`the user edited the plan: <describe>`, M8c's `describe`); a planner finishing or failing (`sub-planner <e> finished with <n> tasks`, `sub-planner <e> failed: <reason>`); a run scout ending (`scout <id> reported`, `scout <id> failed: <reason>`); an integration verdict (`integration review of epic <e>: <approve | changes (<n> critical, <m> important)>`); the run halting (`the run halted: <reason>`); completion (decision 38); a restart (decision 11). The orchestrator's own edits add none.
    - **The reducer** emits `Effect::WakeOrchestrator { run_id, window_id, text, digest_revision }` whenever notes are pending, the orchestrator window is live, `wake_orchestrator` is on, and `digest_rev > last_wake_rev`. `text` is `wake_text(run_id, notes)`, clamped to 2 KiB.
    - **The driver** (`run/driver/wake.rs`) delivers it into the PTY only when the window's status is `Idle` or `Done` (never `Working`, and never `Attention`, which may be a permission prompt) and no client input reached the window for `wake_quiet_secs` (`WindowManager::last_client_input(id)`, set by the server on `ClientMsg::Input`). Delivery is the superseded M8 brief's paste: `ESC [ 200 ~`, the text with `\r\n` and `\n` turned into `\r` and any paste markers removed, `ESC [ 201 ~`, then after `SUBMIT_DELAY` (200 ms, a `tokio::time::sleep`) a lone `\r`, both through `WindowManager::write_input`, never under a lock. It then sends `EventKind::OrchestratorWoken { run_id, digest_revision }`, which clears the delivered notes and sets `last_wake_rev`. A pending wake is re-checked on every 1-second tick and on every window-list change; a newer `WakeOrchestrator` for the same run replaces an undelivered one.
    - **A read clears too.** `DigestRead` (decision 16) drops the notes up to the revision read, so an orchestrator that is already polling is never pasted at.
    - `[orchestrator] wake_orchestrator = false` turns wake-ups off; the notes still appear in the digest.

    *(Spec §10 "the orchestrator is told through the digest", §12.4; M8a decision 29 removed paste delivery for headless sessions, and this is the one PTY run window that needs it.)*
40. **What the engine records for the orchestrator's reactions.** `Run.edit_log` keeps the last 100 batches from every source (`user`, `orchestrator`, `planner:<e>`), accepted or rejected, with `describe`'s text and, for a rejection, the first error; the digest shows the last 10. `RunInfo.edit_log` carries the same for the CLI. The reactions themselves are the contract's (decision 41) and use only edits the engine already validates: `answer` for `blocked(question)` (M8a), `split_task` or a rewriting `amend_task` for `blocked(mis_sized)` (decision 25), `amend_task deps` or `cancel_task` for `blocked(dep_cancelled)`, and a sentence to the user for `blocked(human)`, `blocked(conflict)` and `blocked(environment)`. *(Spec §10, §11.4 steps 5–6.)*

### Contracts

41. **Contracts are fixed texts** (Interfaces, exact): `ORCHESTRATOR_CONTRACT` and `PLANNER_CONTRACT` in `run/orch/contract.rs`. They never vary within a session, so the cached prefix stays stable (spec §14.2); the run's facts go in the first prompt and in `get_context`. Contract tests assert every rule the spec asks the planner to follow is present (task M9.5). The worker, reviewer and scout contracts of M8a and M8b are unchanged. *(Spec §7, §8, §9, §6, §14.2.)*

## Interfaces

### `proto`

`crates/proto/src/run.rs` (M8a's), appended variants and fields:

```rust
pub enum AgentRole { Orchestrator, Worker, Reviewer, Scout, Planner }   // Scout is M8b's; "planner"
pub enum RunState  { /* M8a's eight */ Planning }                       // "planning"; label() "planning"; not terminal
pub enum TaskState { /* M8a's eleven */ Reported }                      // "reported"; label() "reported"; is_finished() true
pub struct PlanTask { /* M8a's fields */ #[serde(default)] pub review_target: Option<String> }   // decision 24
// PlanEdit::AmendTask gains, last:  #[serde(default)] deps: Option<Vec<String>>                 // decision 25
```

`crates/proto/src/orch.rs` (new; everything derives `Debug, Clone, PartialEq, Serialize, Deserialize`, plus what is noted):

```rust
#[derive(Eq)]
pub struct OrchestratorChoice { pub runtime: Runtime, #[serde(default)] pub model: Option<String> }  // decision 6
#[serde(tag = "kind", rename_all = "snake_case")] #[derive(Eq)]
pub enum HoldKind { Promotion, Epic { epic: String } }
#[serde(rename_all = "snake_case")] #[derive(Copy, Eq)]
pub enum HoldState { Drafting, Awaiting, Approved, Rejected }
#[derive(Eq)]
pub struct HoldInfo { pub id: String, pub kind: HoldKind, pub state: HoldState, pub tasks: Vec<String>,
                      pub created_at: u64, pub decided_at: Option<u64>, pub decided_by: Option<String> }
#[derive(Eq)]
pub struct OrchestratorInfo { pub route: Route, pub window_id: Option<u32>, pub live: bool, pub started_at: u64,
                              pub plan_submitted: bool, pub summary: Option<String>, pub notes: Vec<String>, pub wakes: u32 }
#[serde(rename_all = "snake_case")] #[derive(Copy, Eq, Default)]
pub enum IntegrationState { #[default] NotYet, Reviewing, Approved, Changes, Finished }   // Finished: closed by the finish edit
pub struct IntegrationInfo { pub epic: String, pub state: IntegrationState, pub base: Option<String>,
                             pub merges: Vec<String>, pub reviews: Vec<ReviewInfo> }
#[derive(Eq)]
pub struct EditLogInfo { pub at: u64, pub source: String, pub text: String, pub accepted: bool, pub error: Option<String> }
```

`crates/proto/src/run_info.rs`, new fields, each `#[serde(default)]`:

```rust
// RunInfo
pub orchestrator: Option<OrchestratorInfo>, pub holds: Vec<HoldInfo>, pub integration: Vec<IntegrationInfo>,
pub digest_revision: u64, pub edit_log: Vec<EditLogInfo>, pub research_report: Option<PathBuf>,
// RunInfo.planners: Vec<PlannerInfo> is M8c's placeholder, filled by this milestone (decision 33)
// PlannerInfo (M8c)
pub note: Option<String>,
// TaskInfo
pub hold: Option<String>, pub review_target: Option<String>, pub research_bytes: Option<u32>,
```

`crates/proto/src/run_wire.rs`:

```rust
pub struct ToolCall { /* M8a's and M8b's fields */ #[serde(default)] pub epic: Option<String> }
pub enum RunRequest {
    /* M8a's and M8b's */
    // StartGoal gains, last:  #[serde(default)] orchestrator: Option<OrchestratorChoice>
    // Promote gains, last:    #[serde(default)] orchestrator: Option<OrchestratorChoice>
    ApproveHold { run_id: String, hold: String },
    RejectHold { run_id: String, hold: String },
}
// ApproveHold answers with request label request::APPROVE, RejectHold with request::REJECT.
```

`lib.rs` re-exports the new types and bumps `PROTO_VERSION` (header).

### `config` (`crates/config/src/orchestrator_agent.rs`, new; one call from M8a's `orchestrator::read`)

```rust
pub struct AgentConfig {                      // [orchestrator.agent]
    pub runtime: Option<proto::Runtime>,      // None = orchestrator.default_runtime
    pub model: String,                        // "" = decision 6's resolution; must be in the roster at run start
    pub effort: proto::Effort,                // "high"
}
pub struct PlannerConfig {                    // [orchestrator.planners]
    pub runtime: Option<proto::Runtime>,      // None = the orchestrator's runtime
    pub strength: proto::Strength,            // "frontier"
    pub effort: proto::Effort,                // "high"
    pub max_tool_calls: u32,                  // 200, 20..=2000
    pub timeout_secs: u64,                    // 2400, 120..=14400
    pub max_rejections: u32,                  // 5, 1..=20
}
// config::Orchestrator gains:
pub planner_task_cap: u32,                    // [orchestrator] planner_task_cap = 12, 2..=50
pub max_scouts: u32,                          // 12, 1..=50 (per run)
pub wake_orchestrator: bool,                  // true
pub wake_quiet_secs: u64,                     // 5, 1..=120
pub agent: AgentConfig, pub planners: PlannerConfig,
pub(crate) fn read_agent(table: &toml::Table, problems: &mut Vec<Problem>)
    -> (u32, u32, bool, u64, AgentConfig, PlannerConfig);
```

Messages follow M8a's format, for example `orchestrator.planner_task_cap: must be between 2 and 50 (using 12)`, `orchestrator.agent.effort: must be low, medium or high (using high)`, and `unknown key, ignored` under the two new tables. A non-empty `agent.model` that is not in the merged roster for the resolved runtime is not a config problem; `run start --goal` refuses with decision 6's text.

`RunLimits` (M8a) gains `planner_task_cap: u32`, `max_scouts: u32`, `wake_orchestrator: bool`, `wake_quiet_secs: u64`, `planners: config::PlannerConfig`, each `#[serde(default)]` with the config defaults.

### `daemon`

```rust
// run/orch/mod.rs (pure). Everything derives Debug, Clone, PartialEq, Serialize, Deserialize.
pub enum EditSource { User, Orchestrator, Planner { epic: String } }   // label(): "user" | "orchestrator" | "planner:<e>"
pub struct OrchestratorRecord {
    pub route: Route, pub window_id: Option<u32>, pub launch_op: Option<OpId>, pub live: bool,
    pub started_at: u64, pub exited_at: Option<u64>, pub first_prompt: String,
    pub plan_submitted: bool, pub summary: Option<String>, pub finish_accepted: bool,
    pub notes: Vec<String>, pub last_wake_rev: u64, pub wakes: u32,
}
pub enum PlannerPhase { Queued, Planning, Finished, Failed { reason: String } }
pub struct EpicRecord {
    pub epic: String, pub title: String, pub area: Vec<String>, pub brief: String, pub scout_refs: Vec<String>,
    pub route: Route, pub phase: PlannerPhase, pub request: String,       // the live or last session's brief
    pub sessions: Vec<AgentRound>,                                          // one per planner session (M8a's round type)
    pub accepted_this_session: bool, pub nudged: bool, pub wrap_up_sent: bool,
    pub started_at: u64, pub ended_at: Option<u64>,
    pub edits_accepted: u32, pub edits_rejected: u32, pub last_rejection: Option<String>,
    pub replans: Vec<String>, pub note: Option<String>, pub hold: Option<String>,
    pub base: Option<String>, pub merges: Vec<(String, String)>,           // (task id, merge commit)
    pub integration: Vec<ReviewRecord>, pub integration_round: Option<AgentRound>,
    pub integration_state: IntegrationState,
}
pub struct HoldRecord { pub id: String, pub kind: HoldKind, pub state: HoldState, pub tasks: Vec<String>,
                        pub created_at: u64, pub decided_at: Option<u64>, pub decided_by: Option<String> }
pub enum RunScoutState { Queued, Running, Reported, Failed { reason: String } }
pub struct RunScout { pub id: String, pub question: String, pub area: Vec<String>, pub web: bool,
                      pub state: RunScoutState, pub queued_at: u64, pub started_at: Option<u64>,
                      pub ended_at: Option<u64>, pub window_id: Option<u32> }
pub struct EditLogEntry { pub at: u64, pub source: EditSource, pub text: String, pub accepted: bool, pub error: Option<String> }
pub fn build_planned_run(goal: String, pre: Preflight, ctx: BuildContext<'_>, triage: TriageInfo, route: Route,
                         installed: BTreeMap<String, bool>) -> Result<Run, Vec<PlanError>>;   // decision 26; path = triage.path, yes = ctx.yes
// run/model.rs (M8a) — Run gains, each #[serde(default)]:
//   orchestrator: Option<OrchestratorRecord>, epics: Vec<EpicRecord>, holds: Vec<HoldRecord>,
//   run_scouts: Vec<RunScout>, edit_log: Vec<EditLogEntry>, digest_rev: u64, digest_fp: u64,
//   installed: BTreeMap<String, bool>, yes: bool, research_report: Option<PathBuf>
// Task gains: hold: Option<String>, research: Option<ScoutReportArgs>, review_range: Option<(String, String)>

// run/orch/tools.rs (pure)
pub enum OrchCall {
    GetContext { scouts: Option<Vec<String>> },
    SpawnScout { id: String, question: String, area: Vec<String>, web: bool },
    SpawnSubplanner { epic: String, title: String, area: Vec<String>, brief: String, scout_refs: Vec<String> },
    EditPlan { edits: Vec<PlanEdit>, submit: bool, summary: Option<String> },
    RunStatus { since: Option<u64>, wait_secs: u64 },
    TaskResult { task_id: String },
    SubmitEpic { edits: Vec<PlanEdit>, note: Option<String> },
}
pub fn parse_call(role: AgentRole, tool: &str, args: &serde_json::Value) -> Result<OrchCall, String>;
    // Err: "invalid arguments: <field>: <problem>" (the MCP schema's limits, checked again) or
    //      "tool <tool> is not available to the <role> role"
pub const RUN_STATUS_MAX_WAIT: u64 = 50;

// run/orch/rules.rs (pure)
pub fn check(run: &Run, touched: &BTreeSet<String>, source: &EditSource) -> Vec<PlanError>;   // decision 23

// run/orch/digest.rs (pure)
pub const DIGEST_MAX_BYTES: usize = 48 * 1024;
pub fn digest(run: &Run, now: u64) -> serde_json::Value;       // Interfaces "The digest", trimmed to the cap
pub fn fingerprint(run: &Run) -> u64;                           // decision 16: counters excluded

// run/orch/context.rs (pure)
pub const CONTEXT_MAX_BYTES: usize = 96 * 1024;
pub enum Asker { Orchestrator, Planner { epic: String } }
pub struct ContextInputs<'a> { pub run: &'a Run, pub asker: Asker, pub profile: Option<&'a RepoProfile>,
                               pub reports: Vec<ScoutReport>, pub only: Option<Vec<String>> }   // reports read by the driver
pub fn context(inputs: &ContextInputs<'_>) -> serde_json::Value;

// run/orch/result.rs (pure)
pub const TASK_RESULT_MAX_BYTES: usize = 64 * 1024;
pub struct TaskGit { pub commits: Vec<(String, String)>, pub diffstat: String }
pub fn task_result(run: &Run, task: &Task, git: Option<&TaskGit>) -> serde_json::Value;

// run/orch/launch.rs (pure)
pub fn resolve_orchestrator(choice: Option<&OrchestratorChoice>, agent: &config::AgentConfig,
                            default_runtime: Runtime, roster: &[ModelEntry]) -> Result<Route, String>;   // decision 6
pub fn orchestrator_role(run: &Run, route: &Route) -> RoleLaunch;                   // env filled by the driver (decision 10)
pub fn orchestrator_window_spec(run: &Run, route: &Route, first_prompt: &str) -> WindowSpec;
    // name "<h4>/orchestrator", runtime, cwd = run.root, worktree_branch None, model (None when ""), initial_prompt
pub fn planner_spec(run: &Run, epic: &EpicRecord, session: u32) -> HeadlessSpec;   // decision 31
pub fn research_spec(run: &Run, task: &Task) -> HeadlessSpec;                       // decision 35
pub fn review_task_spec(run: &Run, task: &Task, route: &Route) -> HeadlessSpec;     // decision 36
pub fn integration_reviewer_spec(run: &Run, epic: &EpicRecord, route: &Route) -> HeadlessSpec;   // decision 37
pub fn scout_spec(run: &Run, scout: &RunScout, root: &Path, project: &Path) -> ScoutSpec;        // decision 20

// run/engine (additions)
pub enum EventKind { /* M8a's, M8b's */
    ApproveHold { reply: ReplyId, run_id: String, hold: String },
    RejectHold { reply: ReplyId, run_id: String, hold: String },
    ScoutEnded { run_id: String, scout_id: String, outcome: ScoutEnd, usage: TokenUsage },
    OrchestratorWindow { run_id: String, window_id: u32, live: bool },
    OrchestratorWoken { run_id: String, digest_revision: u64 },
    DigestRead { run_id: String, digest_revision: u64 },
}
// M8b's EventKind::Promote { reply, run_id } gains  route: Option<Route>, installed: BTreeMap<String, bool>  (None: refused before the event)
// M8a's EventKind::Start is reused for a Planning run (decision 26); M8a's EventKind::Tool carries orchestrator and planner calls.
pub enum ScoutEnd { Reported, Failed { reason: String } }
pub enum OpKind { /* M8a's, M8b's */
    CreateOrchestrator { spec: WindowSpec, role: RoleLaunch, project: PathBuf },   // → OpResult::Window { window_id }
    RestartOrchestrator { window_id: u32 },                                          // → OpResult::Restarted
    StartScout { spec: ScoutSpec },                                                  // → OpResult::ScoutStarted { window_id }
    ResolveTarget { root: PathBuf, target: String, base_branch: String },            // → OpResult::Target { base, head }
}
pub enum OpResult { /* M8a's, M8b's */ Restarted, ScoutStarted { window_id: u32 }, Target { base: String, head: String } }
pub enum Effect { /* M8a's */ WakeOrchestrator { run_id: String, window_id: u32, text: String, digest_revision: u64 } }
// run/engine/orch.rs, planners.rs, kinds.rs, integration.rs (pure) — entry points called from engine/mod.rs:
pub fn on_orchestrator_tool(run: &mut Run, window_id: u32, call: OrchCall, reply: ReplyId, now: u64) -> Vec<Effect>;
pub fn on_planner_tool(run: &mut Run, window_id: u32, epic: &str, call: OrchCall, reply: ReplyId, now: u64) -> Vec<Effect>;
pub fn on_task_scout_report(run: &mut Run, window_id: u32, task_id: &str, args: &serde_json::Value, reply: ReplyId, now: u64) -> Vec<Effect>;
pub fn on_epic_review(run: &mut Run, window_id: u32, epic: &str, args: &serde_json::Value, reply: ReplyId, now: u64) -> Vec<Effect>;
pub fn after_step(run: &mut Run, now: u64) -> Vec<Effect>;    // digest fingerprint, wake effect, completion extension
pub enum RoundOwner { Task(String), Planner(String), Integration(String) }   // decision 32: M8a's round lookup by window id returns one

// run/git/summary.rs (blocking; run_git; reads, no write queue)
pub fn task_summary(git: &OsStr, root: &Path, start: &str, branch: &str, timeout: Duration) -> Result<TaskGit, String>;
pub fn resolve_target(git: &OsStr, root: &Path, target: &str, base_branch: &str, timeout: Duration)
    -> Result<(String, String), String>;                                           // (base sha, head sha)

// run/driver/orch.rs
impl RunService {
    pub(crate) async fn orchestrator_read(&self, call: ToolCall) -> RunReply;   // get_context, run_status, task_result
}
// run/driver/wake.rs
pub const SUBMIT_DELAY: Duration = Duration::from_millis(200);
pub const WAKE_MAX_BYTES: usize = 2 * 1024;
pub fn encode_paste(text: &str) -> Vec<u8>;                                     // pure helper, tested here
pub async fn deliver_wake(manager: &WindowManager, window_id: u32, text: &str) -> anyhow::Result<()>;
// RunContext (M8a) gains: scouts: Arc<ScoutService>, profiles: Arc<ProfileService> (M8b), claude_bin, codex_bin (M8b)

// launch/role.rs (pure)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleLaunch {
    pub run_ref: RunRef, pub mcp: McpTarget, pub instructions: String, pub effort: Effort,
    pub claude_allowed_tools: Vec<String>, pub claude_disallowed_tools: Vec<String>,
    pub env: Vec<(String, String)>,
}
pub const ORCHESTRATOR_ALLOWED_TOOLS: &[&str] = &["mcp__anthrex__get_context", "mcp__anthrex__spawn_scout",
    "mcp__anthrex__spawn_subplanner", "mcp__anthrex__edit_plan", "mcp__anthrex__run_status",
    "mcp__anthrex__task_result", "Read", "Glob", "Grep"];
pub const ORCHESTRATOR_DISALLOWED_TOOLS: &[&str] = &["Edit", "Write", "NotebookEdit", "Bash", "Agent"];
pub fn claude_role_args(role: &RoleLaunch, ctx: &LaunchContext<'_>, caps: &CliCaps) -> Vec<String>;   // decision 7's middle block
pub fn codex_role_args(role: &RoleLaunch, ctx: &LaunchContext<'_>, caps: &CliCaps) -> Vec<String>;    // decision 8's block
// launch/mod.rs: LaunchContext gains  pub role: Option<&'a RoleLaunch>;  LaunchPlan gains  pub scrub_agent_env: bool
//                (every existing construction site passes role: None; plan() sets scrub_agent_env = role.is_some())
// launch/claude.rs: HOOK_EVENTS: [&str; 11], StopFailure inserted in alphabetical order after Stop
// hooks.rs: HookKind::StopFailure (last); parse "StopFailure"; to_status_event → Some(StatusEvent::Stop)

// manager/role_window.rs
impl WindowManager {
    pub async fn create_run_window(&self, spec: WindowSpec, project: PathBuf, role: RoleLaunch) -> anyhow::Result<WindowInfo>;
    pub fn note_client_input(&self, id: u32);                   // server.rs, on ClientMsg::Input; under the lock, no I/O
    pub fn last_client_input(&self, id: u32) -> Option<Instant>;
}
// manager/entry.rs: Entry gains  role: Option<RoleLaunch>, last_client_input: Option<Instant>;
//                   Entry::info sets WindowInfo.run from role.run_ref for a Pty entry
// manager/create.rs: admit, insert and spawn_window become pub(super); spawn_window and insert take role: Option<RoleLaunch>
// manager/restart.rs: ForRelaunch gains role: Option<RoleLaunch>
// manager/restore.rs + state_snapshot: WindowRecord.run = {"role_launch": RoleLaunch} for a Pty entry with a role
// headless/mod.rs: McpTarget gains #[serde(default)] epic: Option<String>;  headless/argv.rs: mcp_args appends --epic <e>
// window.rs: Window::spawn removes SCRUB_PREFIXES/SCRUB_NAMES variables when plan.scrub_agent_env
```

**Reconcile rows** added to M8a's table:

| `OpKind` | Reality checked | Replay | Otherwise |
|----------|-----------------|--------|-----------|
| `CreateOrchestrator` | a restored window whose persisted `RoleLaunch.run_ref` has this run and role `Orchestrator` | `Window { window_id }` (dormant; `run resume` restarts it) | `NotStarted` |
| `RestartOrchestrator`, `ResolveTarget` | none (idempotent) | — | `NotStarted` |
| `StartScout` | a live process whose command line holds the scout's session id (killed, M8a decision 28) | — | `NotStarted`; the run scout is marked failed on `Restore` (decision 20) |

### MCP (`crates/mcp`)

`McpOptions` gains `epic: Option<String>`. `anthrex mcp` accepts `--role orchestrator` and `--role planner`, and `--epic <e>` (required for `planner`, refused for `orchestrator`). `--role scout` accepts `--task <t>` in place of `--scout <id>` (decision 35). New schemas live in `crates/mcp/src/tools_orch.rs`. Every schema is a closed object at every level (`additionalProperties: false`).

| Role | Tool | Description | Properties (required in bold) |
|------|------|-------------|-------------------------------|
| orchestrator, planner | `get_context` | `Read the run's context: the repository profile, the models you can route to, the limits, scout reports, epics and the plan so far.` | `scouts` array ≤ 50 of string 1–48 |
| orchestrator | `spawn_scout` | `Start a read-only scout on one area with one question. Returns at once; its report appears in run_status and get_context.` | **`id`** string matching `^[a-z0-9][a-z0-9-]{0,31}$`; **`question`** string 1–2000; **`area`** array 1–20 of string 1–300; `web` boolean |
| orchestrator | `spawn_subplanner` | `Start a sub-planner for one epic with its own area, or a fresh one to re-plan an existing epic. Returns at once.` | **`epic`** string matching `^[a-z0-9][a-z0-9-]{0,15}$`; **`title`** string 1–80; **`area`** array 1–20 of string 1–300; **`brief`** string 1–8000; `scout_refs` array ≤ 20 of string 1–48 |
| orchestrator | `edit_plan` | `Apply plan edits as one batch. Set submit to open the plan gate. Add a summary for the user when the run is complete. Returns at once.` | **`edits`** array ≤ 60 of `plan_edit`; `submit` boolean; `summary` string 1–8000 |
| orchestrator | `run_status` | `Read the run digest. With since and wait_secs, wait up to wait_secs seconds (at most 50) for it to change.` | `since` integer ≥ 0; `wait_secs` integer 0–50 |
| orchestrator | `task_result` | `Read everything about one task: brief, commits, diff size, checks, proofs, reviews, agent rounds and any report.` | **`task_id`** string 1–16 |
| planner | `submit_epic` | `Submit your epic's tasks as one batch of plan edits. If it returns errors, fix them and call it again. When it is accepted you are done.` | **`edits`** array 1–60 of `plan_edit`; `note` string 1–2000 |

`plan_edit` is an object with **`op`** enum `add_task`, `split_task`, `cancel_task`, `amend_task`, `add_dep`, `answer`, `pause`, `resume`, `finish`, and optional `task` (`plan_task`), `task_id` string 1–16, `into` array 1–12 of `plan_task`, `brief` string 1–8000, `acceptance` array 1–20 of string 1–500, `route` (`route`), `test_mode` enum `tdd`, `check`, `none`, `test_mode_reason` string 1–300, `priority` integer, `size` enum `S`, `M`, `L`, `deps` array ≤ 20 of string 1–16, `dep` string 1–16, `text` string 1–8000. Which keys each op needs is M8a's `PlanEdit` serde shape, checked by the daemon (`invalid arguments: edits[<i>]: <serde error>`).

`plan_task` is an object with **`id`** string matching `^[a-z0-9][a-z0-9-]{0,15}$`; **`title`** string 1–120; `epic` string 1–16; `kind` enum `code`, `docs`, `research`, `review`; **`size`** enum `S`, `M`, `L`; `interface_change` boolean; `test_mode` enum; `test_mode_reason` string 1–300; **`owns`** array ≤ 20 of string 1–300; `deps` array ≤ 20 of string 1–16; `priority` integer; **`brief`** string 1–8000; **`acceptance`** array 1–20 of string 1–500; `test_to_write` string 1–300; `scout_refs` array ≤ 20 of string 1–48; `route` (`route`); `review_target` string 1–200. `budget` is deliberately absent (decision 23.1).

`route` is an object with `runtime` enum `claude`, `codex`; `model` string 0–100; `strength` enum `fast`, `standard`, `frontier`; `effort` enum `low`, `medium`, `high`.

Engine-side texts, each a `ToolResult { ok: false }` whose text is `{"error": "<text>"}`: M8a's `unknown run <id>`, `run <id> is paused; the user must resume it`, `run <id> is <state>`; `this window is not the orchestrator of run <id>`; `this window is not the sub-planner of epic <e> of run <id>`; `unknown task <id>`; `unknown epic <e>`; `scout <id> already exists in run <run>`; `run <id> already has <n> scouts, the most max_scouts allows`; `epic <e> is being planned by its sub-planner; wait for it to finish`; `epic <e>: area: overlaps epic <f>'s area (<glob>)`; `epic <e>: area: <glob> must be a literal path or end in /**`; `the plan has no tasks yet; add tasks before submitting`; `sub-planner <e> is still planning; submit when every sub-planner has finished`; `edits are not accepted on a complete run; only a summary is`; `op <op> is not available to a sub-planner`; `submit_epic was already accepted for epic <e>`; `invalid arguments: <field>: <problem>`. A rejected batch is decision 19's `{"accepted": false, "errors": […]}`. Successes are decisions 19–22's JSON, and `Epic recorded. You are done; end your turn now.` for `submit_epic`.

### The digest (`run_status`)

One JSON object, compact, at most `DIGEST_MAX_BYTES`:

```json
{
  "revision": 42, "now": 1790000000,
  "run": {"id": "add-gemini-3f9a", "goal": "Add Gemini runtime", "state": "running", "path": "large",
          "base": "main@1a2b3c4", "approved_by": "user", "complete": false, "halted_reason": null,
          "summary_written": false},
  "gate": {"state": "approved", "at": "11:02",
           "holds": [{"id": "epic:c", "state": "awaiting", "tasks": 4}]},
  "slots": {"writers": "2/3", "readers": "1/3"},
  "counts": {"total": 9, "merged": 5, "reported": 0, "working": 2, "checking": 0, "review": 1,
             "merging": 0, "blocked": 1, "waiting": 0, "held": 0, "cancelled": 0},
  "tasks": [{"id": "t2", "title": "map hook events", "epic": "a", "kind": "code", "size": "M", "hub": false,
             "state": "blocked", "hold": null, "block": {"reason": "question", "text": "…"},
             "rung": 1, "deps": ["t0"], "route": "codex (default) high", "review": "r1 changes (1 critical)",
             "last": "12:31 review r1 changes"}],
  "omitted_tasks": 0,
  "scouts": [{"id": "3f9a-daemon", "state": "reported", "question": "…", "failure": null}],
  "planners": [{"epic": "a", "state": "finished", "tasks": 3, "rejected": 1, "last_rejection": "…", "note": null}],
  "integration": [{"epic": "a", "state": "changes", "round": 1, "findings": "1 critical, 0 important"}],
  "attention": ["t2 blocked (question): …"],
  "notes": ["t2 blocked (question): …"],
  "edits": [{"at": "11:40", "source": "user", "text": "cancel t4", "accepted": true, "error": null}],
  "spend": {"tokens": 1800000, "tool_calls": 612}
}
```

- `gate.state`: `planning`, `awaiting_approval`, `approved` (with `at`, and `approved_by` in `run`), or `none` (a fast-path run before promotion). `holds` lists every hold not `Approved`, plus holds decided since the last `DigestRead`.
- A task's `block.text` and a scout's `question` are cut to 500 characters; `route` is `<runtime> <model or (default)> <effort>` as in `run status`; `review` is the last round with a verdict, or `null`; `last` is `history[0]` or `null`.
- `counts.held` counts tasks whose hold is not `Approved`.
- **Trimming** past the cap, in order: finished tasks are dropped oldest first and counted in `omitted_tasks`; `edits` is cut to 3; `block.text` to 200 characters; `notes` to 5.
- `fingerprint` hashes this object with `now`, `spend`, `slots` and every task's `last` removed; the history line changes with every event, while the state that matters is carried by `state`, `block`, `rung` and `review`.

### The context (`get_context`)

```json
{
  "run": {"id": "…", "goal": "…", "path": "large", "state": "planning", "root": "/…/repo", "base_branch": "main",
          "triage": {"kinds": ["code"], "scale": "large", "reason": "…"}},
  "you": {"role": "planner", "epic": {"epic": "a", "title": "daemon", "area": ["crates/daemon/**"], "brief": "…"}},
  "profile": {"summary": "<profile::summary text>", "modules": [], "hub": [], "source": [], "generated": [], "protected": [],
              "check": "…", "single_test": "…"},
  "limits": {"planner_task_cap": 12, "max_tasks": 50, "max_writers": 3, "max_readers": 3, "max_bounces": 2, "max_scouts": 12,
             "sizes": {"S": "one file, no interface change, a mechanical check exists, about 20 changed lines",
                       "M": "one to three files inside one module, a clear spec, a check exists, about 100 changed lines",
                       "L": "never executed: split it"}},
  "roster": [{"runtime": "claude", "model": "claude-opus-5", "strength": "frontier", "note": "…", "installed": true}],
  "scouts": [{"id": "3f9a-daemon", "state": "reported", "question": "…", "area": ["crates/daemon/**"],
              "summary": "…", "files": [{"path": "…", "why": "…"}], "modules": [], "interfaces": [], "risks": []}],
  "epics": [{"epic": "a", "title": "daemon", "area": ["crates/daemon/**"], "state": "finished", "tasks": 3}],
  "plan": [{"id": "t0", "title": "…", "epic": null, "kind": "code", "size": "M", "hub": true,
            "owns": ["crates/proto/**"], "deps": [], "state": "pending"}],
  "omitted": {"scouts": 0}
}
```

`you.epic` is `null` for the orchestrator. The onboarding report, when the stored profile has one, is listed in `scouts` with id `onboarding`.

### The task result (`task_result`)

```json
{
  "task": {"id", "title", "epic", "kind", "size", "hub", "test_mode", "test_mode_reason", "owns", "deps", "implicit_deps",
           "route", "review_route", "state", "block", "rung", "failures", "bounces", "stalls", "hold",
           "brief", "acceptance", "test_to_write", "scout_refs", "review_target", "notes"},
  "done": {"signal": "task_done", "summary": "…", "test": "…", "red": "a1b2c3d"} ,
  "commits": [{"sha": "a1b2c3d", "subject": "…"}], "diffstat": " 4 files changed, 212 insertions(+), 31 deletions(-)",
  "checks": [{"at": "12:20", "ok": true, "code": 0, "timed_out": false, "summary": "…"}],
  "proofs": [{"at": "12:10", "test": "…", "red": "…", "ok": true}],
  "reviews": [{"round": 1, "runtime": "claude", "verdict": "changes", "summary": "…",
               "findings": [{"severity": "critical", "file": "…", "line": 118, "input": null, "text": "…"}]}],
  "rounds": [{"role": "worker", "session": 1, "round": 1, "runtime": "codex", "model": "", "effort": "high",
              "turns": 14, "tool_calls": 41, "tokens": 180000, "ended": false}],
  "research": null,
  "history": ["12:31 review r1 changes"]
}
```

(`task` lists its keys only; every value is the task's field as JSON.) `done` is `null` without a claim; `commits` and `diffstat` are absent when the task has no start commit or the git read failed (then `git: "<error>"`). Past the cap, `checks` keep their last 3, `rounds` their last 5, and `research.summary` is cut to 16 000 characters.

### Contracts (exact)

Both constants contain no em dash, have no leading or trailing whitespace, and survive `launch::codex::toml_string` as a TOML string.

```text
ORCHESTRATOR_CONTRACT:
You are the orchestrator of an anthrex run. The user gave a goal. You scout the repository, plan the work as small tasks, and steer the run until it finishes. anthrex's engine does the rest: it runs each task in its own git worktree with a headless worker, proves tdd tests, runs the check, has a different agent review the work, and merges approved work into the run branch. Nothing reaches the user's base branch until the user accepts the run.

You are the only agent the user talks to. Workers, reviewers, scouts and sub-planners are headless: the user watches them but cannot type to them, and neither can you.

What you may and may not do
1. You never edit, create or delete files, never run shell commands, and never commit. You may read files in this checkout. You never read task worktrees; call task_result instead.
2. You never approve a task and never merge. Reviewers and the engine approve, the engine merges, and only the user can override a rejection or accept the run. None of your tools does either. Never ask a worker to.
3. Every change to the plan goes through edit_plan. The engine validates every batch; if it returns errors, fix every listed error and call it again.

How a run goes
4. Call get_context first: the repository profile, the models you can route to, the limits, and any scout reports.
5. Scout before you plan. Call spawn_scout once per area the goal touches, each with one concrete question. Wait for their reports with run_status, then read them with get_context.
6. Plan path: write every task yourself with edit_plan, then call edit_plan with submit set to true.
7. Large path, when the goal needs more than planner_task_cap tasks or several separate areas that each need several tasks: write the interface and hub tasks yourself first, then call spawn_subplanner once per epic, each with its own area that overlaps no other. Each sub-planner adds its epic's tasks and exits. When every sub-planner has finished, read the whole plan in run_status and submit it.
8. Submitting opens the plan gate unless the user started the run with --yes. edit_plan returns at once with awaiting_approval. The user approves, edits or rejects the plan in the run view, and you learn the verdict from run_status. A new epic added after approval waits for the user's approval the same way, while the rest of the run goes on.
9. After that, call run_status with since set to the last revision and wait_secs 50. It returns as soon as something you need to know changes. Messages that start with [anthrex] come from anthrex, not from the user; when one says the run changed, call run_status.

Sizing
10. Every task is S or M. S: one file, no interface change, a mechanical check exists, about 20 changed lines. M: one to three files inside one module, a clear spec, a check exists, about 100 changed lines. Anything larger is L, and L is never executed: split it.
11. Size from evidence, never from time. Name the scout reports a task's size rests on in its scout_refs. Never give minutes, hours or budgets; the engine sets budgets from the size.
12. Split interfaces first: an interface or hub change is its own task, first, and every task that uses it depends on it. Split one level only; a piece that is still L goes back to whoever planned it, never deeper.
13. A chain of tasks where each depends only on the previous one, and whose combined size is still M, costs a cold start, a check, a review and a merge per link with nothing running beside it. Prefer one task; split only when a step must be reviewed or merged on its own. This is your judgement; the engine does not check it.
14. Hub files are the profile's hub globs. A task that touches one is a hub task: it runs alone, is tdd, and is always reviewed. Keep hub tasks few and small.
15. At most planner_task_cap tasks per planner: yours, and each sub-planner's.

Ownership and files
16. Every code or docs task names the paths it owns as globs, as narrow as possible. Two tasks whose owns overlap never run at the same time, so overlap costs parallelism. A task that changes a file outside its owns is stopped.
17. Generated files, the profile's generated globs such as lock files, change only in a task that owns them. A task that really changes dependencies owns the lock file.
18. Protected files, the agent settings, hooks, MCP servers, CLAUDE.md and AGENTS.md, change only in a task whose owns names each file exactly, never through a wildcard. Plan that only when the goal asks for it.
19. Research and review tasks change nothing: leave their owns empty.

Test mode
20. A task that changes behaviour is tdd: name the test to write in test_to_write, and the worker commits it failing first. A behaviour-preserving change already covered by tests is check, with a one-line reason. Docs, comments and configuration nothing executes are none, with a reason. A code task that touches the profile's source globs is never none.

Routing
21. Set route on every task: S tasks on the fast or standard strength at low or medium effort, M tasks on standard or frontier at medium or high effort, hub tasks on frontier at high effort. Use only models get_context lists as installed.
22. Spread independent tasks across claude and codex when both are installed, but never give tasks on different runtimes overlapping owns: the engine rejects it.

Kinds
23. code and docs tasks go through every gate. research tasks investigate and report, with no branch and no merge. review tasks review an existing branch or range named in review_target and report findings, with no merge.

When something goes wrong
24. A task blocked with question: if the scout reports or the plan answer it, answer with an answer edit. Otherwise ask the user here, then answer with their words.
25. A task blocked as mis_sized: split it with split_task, or rewrite its brief, acceptance criteria and size with amend_task, which restarts it.
26. A task blocked as human, conflict or environment: tell the user what happened and what they can do, such as anthrex run retry, anthrex run override or anthrex run cancel. You cannot unblock it yourself. A task blocked by a cancelled dependency needs new deps from amend_task, or its own cancellation.
27. An integration review that asks for changes: add fix tasks to that epic, or finish the run with a finish edit and tell the user why.

Talking to the user
28. The user steers you by typing here, for example "skip X", "do Y first" or "use codex for Z". Turn each request into edit_plan edits, then say in one line what changed. If a request is unclear or would break a rule above, say so and ask.
29. After each run_status that changed something, write at most two lines here: what happened, and what you are waiting for.

Finishing
30. When run_status reports the run complete, call edit_plan with a summary for the user: what was done, what was not and why, every task that failed or is blocked, and what the user should check before accepting. The user accepts or discards the run; you never do.

PLANNER_CONTRACT:
You are a sub-planner in an anthrex run. The orchestrator gave you one epic: a goal for one area of the repository. You plan that epic as small tasks, submit them once, and stop. You never write code, and nobody can type to you.
1. You never edit, create or delete files, never run shell commands, and never commit. You may read files in this checkout.
2. Call get_context first: the profile, the models, the limits, the scout reports for your area, and the tasks already planned that you may depend on.
3. Every task you add owns paths only inside your area, and belongs to your epic.
4. Every task is S or M. S: one file, no interface change, a mechanical check exists, about 20 changed lines. M: one to three files inside one module, a clear spec, a check exists, about 100 changed lines. L is never executed: split it, interfaces first, one level only. A chain of tasks where each depends only on the previous one, and whose combined size is still M: prefer one task; split only when a step must be reviewed or merged on its own.
5. Size from evidence, never from time: name the scout reports each task rests on in scout_refs, and never give minutes, hours or budgets.
6. Depend on the orchestrator's interface and hub tasks where you use them. Never plan a change to a hub file. If your epic needs an interface or hub change that is not planned, say so in submit_epic's note.
7. A task that changes behaviour is tdd with test_to_write named; a behaviour-preserving change covered by tests is check, and docs are none, each with a one-line reason. Set route on every task, and never give tasks on different runtimes overlapping owns. Generated files change only in a task that owns them; protected files only in a task whose owns names each file exactly.
8. At most planner_task_cap tasks.
9. Call submit_epic once with every edit. If it returns errors, fix every listed error and call it again. When it is accepted, end your turn: you are done.
10. Messages that start with [anthrex] come from anthrex. Do what they say.
```

### Prompts and messages (exact)

| Name | Text |
|------|------|
| `orchestrator_first_prompt(run)` | `[anthrex] You are the orchestrator of run <id> in <root>.` / `Goal: <goal>` / `Path: <plan \| large> (triage: <kinds joined with ,>/<scale>, <decider \| fallback: <reason>>: <triage reason>)` / `Plan gate: <the user approves your submitted plan in the run view \| off: the run was started with --yes, so your submitted plan starts at once>` / `Start with get_context, then scout, then plan.` |
| `promoted_first_prompt(run)` | `[anthrex] You are the orchestrator of run <id> in <root>, promoted from the fast path at the user's request.` / `Goal: <goal>` / `Its one task so far: t1 <title> (<state label>). It keeps running.` / `Start with get_context and run_status. Plan what else the goal needs; tasks you add wait for the user's approval once you submit them.` |
| `planner_prompt(run, epic, extract)` | `[anthrex] Plan epic <e> "<title>" of run <id>.` / `Run goal: <goal>` / `Your area (every task you add must own paths only inside it):` / `- <glob>` per glob / `Tasks already planned that you may depend on:` / `- <id> <size> <title> (owns <globs joined with , >)` per task with no epic, or `- none` / `Task cap: at most <cap> tasks.` / decision 34's scout extract when non-empty / blank line / `What to plan:` / `<brief>` |
| `replan_prompt(run, epic, extract)` | `planner_prompt`'s text with its first line `[anthrex] Re-plan epic <e> "<title>" of run <id>.`, and before `What to plan:` the lines `The epic's current tasks:` / `- <id> <size> <state label> <title>` per task of the epic; `What to plan:` is followed by the new brief. |
| `PLANNER_NUDGE` | `[anthrex] Your turn ended without an accepted epic. Call submit_epic now with every edit, then stop.` |
| `planner_wrap_up(n)` | `[anthrex] You have used <n> tool calls. Stop reading and call submit_epic now with the tasks you have.` |
| `RESUME_PLANNER` | `[anthrex] The daemon restarted. Finish your epic and call submit_epic.` |
| `scout_first_turn(run, id, area, question)` | `[anthrex] Scout <id> for run <run id>.` / `Area: <globs joined with , >` / `Question: <question>` |
| `research_prompt(run, task)` | `[anthrex] Research task <id>: <title>` / `Run goal: <goal>` / `Answer this from the repository at <root>, and the web if you need it. Change nothing.` / `Your report must cover:` / `- <item>` per acceptance item / blank line / `<brief>` |
| `review_task_prompt(run, task, base, head)` | `[anthrex] Review task <id> "<title>" of <review_target>, level <small \| medium>.` / `Base: <sha7>` / `Head: <sha7>` / `Nothing will be merged; your findings go to the user.` / `Acceptance criteria:` / `- <item>` per item / blank line / `<brief>` |
| `integration_review_prompt(run, epic, round, base, head)` | `[anthrex] Integration review of epic <e> "<title>", round <n>, level frontier.` / `Run goal: <goal>` / `Area: <globs joined with , >` / `Base: <sha7>` / `Head: <sha7>` / `The epic's change is these merges; read each with git diff <merge>^1 <merge>:` / `- <sha7> <task id> <task title>` per merge / `Judge whether the epic's tasks together do what the epic asked, and whether they fit each other and the code around them.` / `Earlier findings to confirm fixed:` lines when `round > 1` / blank line / `<epic brief>` |
| `wake_text(run_id, notes)` | `[anthrex] Run <id> changed: <notes joined with "; ">. Call run_status for the details.` (clamped to `WAKE_MAX_BYTES` by M8a's `messages::clamp` rule) |
| `planned_message(info, id, path)` | `triage: <kinds>/<scale> (<decider \| fallback: <reason>>)` / `<plan \| large> path: run <id> is being planned by its orchestrator` / `talk to it with: anthrex, then C-b T and Enter on the run` / `watch with: anthrex run status <id>` |

Wake notes are decision 39's texts, verbatim.

### CLI

```
anthrex run start (--plan <file> | --goal <text>) [--orchestrator <runtime>[:<model>]] [--yes] [--trust-project]
anthrex run promote <run> [--orchestrator <runtime>[:<model>]]
anthrex run approve <run> [--hold <hold>]
anthrex run reject <run> [--hold <hold>] [--confirm <run-id>]
anthrex run status [<run>] [--json]
anthrex run accept <run> [--yes]
anthrex mcp --role <orchestrator|planner|scout|worker|reviewer> --run <run> [--task <task>] [--epic <epic>] [--scout <id>] --window <id> [--socket <path>]   (hidden)
```

- `--orchestrator` is refused with `--plan` (`--orchestrator applies to --goal runs`). Its value parses as `claude`, `codex`, `claude:<model>` or `codex:<model>` (`codex:` means the default model); anything else exits 1 with `--orchestrator: expected claude or codex, optionally :<model>`.
- `run start --goal` on the plan or large path prints the run id on stdout and `planned_message` on stderr, and exits 0.
- `run approve <run> --hold <h>` sends `ApproveHold`; `run reject <run> --hold <h>` sends `RejectHold` and needs no confirmation (it cancels only held tasks that never started). Without `--hold`, both keep M8a's meaning; on a `running` run with an awaiting hold, `run approve <run>` without `--hold` exits 1 with `run <id> has holds waiting for approval: <ids>; pass --hold <id>`.
- `run status` adds, per run: `planning` as a state; a line `  orchestrator: window <n>, <runtime> <model or (default)>, <live | exited>`; a line `  planners: <e> <state>[ (<n> tasks)], …` when there are epics; a line `  holds: <id> <state> (<n> tasks), …` when there are holds; a line `  summary: written` once the orchestrator wrote one. The task table's `STATE` shows `reported`, and a held task's state is followed by ` (held)`.
- `run accept` prints `research report: <path>` when `research_report` is set, and asks `nothing to merge; accept run <id> and remove its branches? [y/N]` instead of M8a's merge question when `run_head == base_sha`.

### File sizes this milestone must respect

AGENTS.md rule 8: about 600 lines. At the start, run `wc -l` on every file below and record the counts under "Implementation notes"; the budgets are growth over those counts.

| File | Today on `main` | Budget | Note |
|------|---:|---:|------|
| `crates/daemon/src/launch/mod.rs` | 427 | +25 | One `role` field, one `scrub_agent_env` field, one call to `role::claude_role_args`; every flag is built in `launch/role.rs`. |
| `crates/daemon/src/launch/codex.rs` | 353 | +6 | One call to `role::codex_role_args` before `-m`. |
| `crates/daemon/src/launch/claude.rs` | 71 | +3 | `StopFailure`. |
| `crates/daemon/src/hooks.rs` | 313 (391) | +8 | `StopFailure`. |
| `crates/daemon/src/manager/create.rs` | 559 | +15 | Visibility and the `role` parameter only; `create_run_window` is in `manager/role_window.rs`. |
| `crates/daemon/src/manager/restart.rs` | 556 | +8 | `ForRelaunch.role`. |
| `crates/daemon/src/manager/entry.rs` | 331 (+ M8a) | +15 | Two fields, `info`'s `run`. |
| `crates/daemon/src/manager/restore.rs` | 509 (+ M8a) | +20 | Parse and save `role_launch`. |
| `crates/daemon/src/window.rs` | 371 | +10 | The scrub. |
| `crates/daemon/src/server.rs` | 556 (+ M8a) | +3 | One `note_client_input` call; the refusals are in `server/headless_guard.rs` (M8a). |
| M8a's `run/engine/{mod,requests,dispatch,done,gates,merge,restore}.rs` | M8a | +30 each | Calls into the new `engine/{orch,planners,kinds,integration}.rs`. |
| M8a's `run/model.rs`, `run/validate.rs`, `run/edits.rs`, `run/contract.rs`, `run/snapshot.rs`, `run/report.rs`, `run/reconcile.rs` | M8a | +25, +40, +30, +30, +60, +60, +20 | New model types are in `run/orch/mod.rs`. |
| M8a's `run/driver.rs`, `run/driver/ops.rs` | M8a | +20, +20 | New code in `run/driver/{orch,orch_ops,wake}.rs`. |
| M8a's `crates/mcp/src/tools.rs`, `lib.rs` | M8a | +10, +15 | Schemas in `tools_orch.rs`. |
| M8a's `crates/proto/src/run.rs`, `run_info.rs`, `run_wire.rs` | M8a | +10, +25, +15 | New types in `orch.rs`. |
| M8a's `crates/config/src/orchestrator.rs` | M8a | +10 | One `read_agent` call and six fields. |
| M8a's `crates/cli/src/run_cmd.rs`, `run_cmd/status.rs` | M8a | +15, +40 | Bodies in `run_cmd/orch.rs`. |
| M8c's `crates/tui/src/app/runs.rs`, `tree/runs.rs`, `tree/run_rows.rs`, `inspector/run.rs`, `theme.rs`, `graph/run_text.rs` | M8c | +40, +10, +10, +25, +10, +10 | Task M9.15. |
| `crates/fake-agent/src/script.rs`, `main.rs` | 190, 319 (+ M8a) | +20, +10 | New steps' bodies in `src/orch_steps.rs`; PTY MCP in `src/mcp.rs` (M8a). |
| `scripts/pty-smoke.py` | 1572 | +5 | The stage lives in `scripts/pty_smoke_orch.py`. |

No new file may exceed 600 lines. `run/engine/orch.rs` is the one at risk: holds and promotion go to `run/engine/holds.rs` if it passes 450.

## Names taken from earlier briefs

Every name below is used exactly as the source defines it. If the merged code differs, the code wins: record the difference under "Implementation notes" in task M9.1 and use the merged name everywhere, without renaming anything in M8a, M8b or M8c.

| Name | Used for | Source |
|------|----------|--------|
| `run::engine::{step, EngineState, Event, EventKind::{Start, Approve, Reject, Edit, Retry, Override, Cancel, Resume, Finish, Tool, OpDone, Signal, Restore, Tick}, OpKind, OpResult, Effect, ReplyId}`, `OpId` | The reducer this milestone extends | M8a decision 2; Interfaces `run/engine/mod.rs` |
| `OpKind::{CreateRunBranch, CreateWindow { first_turn, … }, ResumeSession, PrepareReview}`, `OpResult::{Window, Review}`, `Effect::{Reply, Op, Deliver, KillWindow, RetireWindow, Persist, WriteReport, Publish}` | Planning start, sessions, reviews | M8a Interfaces `run/engine/mod.rs` |
| `build_run(plan, pre, ctx)`, `Preflight`, `BuildContext { …, yes }`, `resolve_task`, `validate_tasks`, `EditScope::{Run, Area { globs }}`, `apply_edits`, `EditConsequence`, `PlanError { task, field, rule, message }` | Planned runs, edit batches, rule errors | M8a decisions 12–14; Interfaces `run/plan.rs`, `run/validate.rs`, `run/edits.rs` |
| `proto::{PlanTask, PlanEdit, TaskKind, Size, TestMode, Route, Strength, Effort, ModelEntry, Runtime, AgentRole, RunRef { run_id, task_id, role, session }, RunState, TaskState, BlockReason}` | Plan edits, routes, run references | M8a decisions 6–13, 23, 31; Interfaces `proto` |
| `RunInfo`, `TaskInfo`, `RunsSnapshot`, `ReviewInfo`, `AgentRoundInfo`, `RunLimits` | Snapshot additions | M8a decision 47; Interfaces `run_info.rs` |
| `RunRequest`, `RunReply::{Done, Refused, Snapshot, Started, ToolResult}`, `ToolCall`, `request::{APPROVE, REJECT, EDIT}` | Wire additions | M8a Interfaces `run_wire.rs` |
| `Run.{revision, paused_from, trusted_project, base_sha, run_head, root, project}`, `Task.{reviews, history, failures, rung}`, `AgentRound`, `ReviewRecord`, `CheckRecord`, `ProofRecord` | Model the digest and `task_result` read | M8a decisions 31, 38, 43; Interfaces `run/model.rs` |
| `roster::{pick_reviewer, escalate}`, `run::globs::{validate_glob, intersects, names_literally}` | Integration reviewer route; area checks | M8a decisions 9, 11, 23 |
| `WORKER_CONTRACT`, `REVIEWER_CONTRACT`, `worker_prompt`, `REVIEW_NUDGE`, `RESUME_REVIEWER`, `messages::clamp`, `MESSAGE_MAX_BYTES` | Scout extract in `worker_prompt`; review tasks; wake text clamp | M8a decisions 29, 30; Interfaces `run/contract.rs`, `run/messages.rs` |
| `headless::{HeadlessSpec, McpTarget, CLI_CAPS, CliCaps, session::{SCRUB_PREFIXES, SCRUB_NAMES}, argv::mcp_args}`, `WindowManager::create_headless`, `RETIRE_AFTER`, `server/headless_guard.rs` | Sub-planner, research and reviewer sessions; env scrub; kill/remove refusals | M8a decisions 24–28, 49, 52, 53; Interfaces `headless/` |
| `CliCaps.{claude_user_settings_only, codex_user_config_only, codex_loads_project_config, claude_effort_flag}` | Orchestrator launch flags | M8a decision 53; M8a.1 |
| `TOOL_REPLY_TIMEOUT` (100 s), `DONE_CHECK_GIT_TIMEOUT` (10 s), `run_git` | Tool deadlines, git reads | M8a decisions 4, 18, 32 |
| `mcp::{tools_for, McpOptions}`, `anthrex mcp --role --run --task --window --socket` | Orchestrator and planner tools | M8a decision 4; Interfaces "MCP tools" |
| `RunService::request`, `RunContext`, the snapshot `watch`, `RunHarness::{new, script, plan, anthrex, start, subscribe, wait_run, restart_daemon, git}`, `RUN_WAIT`, `fake_agent_bin()` | Driver, long-poll, tests | M8a decision 47; Tasks "Shared test helpers" |
| `fake-agent` headless modes, per-role scripts `<role>-<task>-<n>.jsonl`, steps `mcp_call`, `read_message`, `end_turn`, `sh`, `capture`, `hook`, `exit`, `hang`; `FAKE_AGENT_ARGS_FILE`, `FAKE_AGENT_STDIN_FILE`, `DONE` | Scripted agents | M8a task M8a.20 |
| `scripts/pty_smoke_run.py`, stage `11c`; `scripts/pty_smoke_adapt.py`, stage `11d`; `scripts/pty_smoke_run_view.py`, stage `11e` | Smoke stage ordering | M8a task M8a.25, M8b task M8b.15, M8c task M8c.10 |
| `proto::{RepoProfile, TriageInfo { kinds, scale, path, reason, source, fallback_reason, at }, RunPath::{Fast, Plan, Large}, Scale, DeciderSource, DiffStats, ScoutInfo}`, `AgentRole::Scout` | Triage, path, scouts | M8b Interfaces `proto` |
| `RunRequest::{StartGoal, Promote}`, `RunReply::Triaged { triage, run_id, message }`, `EventKind::{Promote, OrchestratorUsage}`, `Run.{path, triage, promote_requested_at, scout_reports, scout_usage, orchestrator_usage}` | Goal start, promotion, metering | M8b decisions 22, 25, 29; Interfaces `run_wire.rs`, run/engine additions |
| `run::triage::{route, PLAN_SCALE_MAX}`, `TriageRoute::{Fast, Plan { reason }, Large { reason }}` | Replacing the refusal; the cap | M8b decisions 22, 23 |
| `scout::{ScoutService, ScoutHandle, spec::{ScoutSpec { id, kind, run_id, question, first_turn, cwd, project, web }, route, headless_spec, valid_id}, report::{ScoutReportArgs, validate, resolve_ref, report_path}, contract::{SCOUT_CONTRACT, SCOUT_NUDGE}}`, `ScoutKind::{Onboarding, Area}` | Run scouts, research tasks, extracts | M8b decisions 12–15, 19 |
| `McpTarget.scout_id`, `ToolCall.scout_id`, `--role scout --scout <id>` | Scout MCP plumbing extended with `--task` | M8b decision 15 |
| `profile::summary`, `ProfileService`, `Effective`, the project-settings check of `StartGoal` | `get_context`, trust checks | M8b decisions 5–10, 22 |
| `metering::orchestrator_env(addr, run_id)`, `<data_dir>/otlp.addr` | OTLP for the orchestrator | M8b decision 30 |
| `RunHarness::{with_deciders, decider, decider_calls, stored_profile, onboarding_report}` | Triage in end-to-end tests | M8b "Shared test helpers" |
| `PlannerInfo { epic, title, area, route, window_id, state, started_at, ended_at, edits_accepted, edits_rejected, last_rejection, replans }`, `RunInfo.planners` | Filled here | M8c Interfaces, "Consumes from later milestones" |
| `Run.approved_at`, `RunInfo.{approved_hhmm, plan_edits, plan_edits_since_approval}`, `describe(edits)` (and M8a's `TaskEvent`, which `Run.plan_edits` holds) | Gate state in the digest; wake notes for user edits | M8c decision 4; Interfaces |
| The run view's node tree, `C-b T`, run-view keys `a`, `x`, `e`, `d`, round labels, glyph table | Task M9.15 changes | M8c decisions 12–19 |

## Scenario map (spec §21, the M9 row)

| §21 scenario | Test | Task |
|--------------|------|------|
| A goal on the plan path: scouts, a plan, the gate approved through the run view, the verdict read by `run_status` | `e2e_plan_path_scouts_plans_and_reads_the_approval` | M9.16 |
| The plan gate rejected | `e2e_rejected_plan_discards_the_run_and_the_orchestrator_learns_it` | M9.16 |
| Steering by typing | `e2e_typed_steering_becomes_a_plan_edit` | M9.16 |
| A mis-sized task split by the orchestrator | `e2e_mis_sized_task_is_split_by_the_orchestrator` | M9.16 |
| A blocked question answered | `e2e_blocked_question_is_answered_after_a_wake` | M9.16 |
| `run promote` | `e2e_promote_starts_an_orchestrator_and_holds_its_tasks` | M9.16 |
| The orchestrator cannot approve or merge | `e2e_orchestrator_cannot_approve_merge_or_write` | M9.16 |
| A large goal with sub-planners | `e2e_large_path_two_subplanners_submit_their_epics` | M9.17 |
| A sub-planner edit outside its area rejected | `e2e_subplanner_edit_outside_its_area_is_rejected_then_fixed` | M9.17 |
| A new epic mid-run waits for approval | `e2e_new_epic_after_approval_is_held_until_approved` | M9.17 |
| Per-epic integration review | `e2e_integration_review_asks_for_changes_and_a_fix_task_closes_it` | M9.17 |
| A research goal | `e2e_research_goal_reports_without_merging` | M9.17 |
| A review goal | `e2e_review_goal_reviews_a_range_without_merging` | M9.17 |
| Daemon restart | `e2e_restart_resumes_the_orchestrator_with_its_role_flags` | M9.17 |
| OTLP metering of the orchestrator | `e2e_claude_orchestrator_gets_the_otlp_environment` | M9.17 |

## Tasks

Do them in this order. Each task's tests are written first and must fail before its change (AGENTS.md rule 6). One commit per task.

**Shared test helpers**, added in the task that first needs them:

- `RunHarness` (M9.16) gains:
  - `start_goal(goal, extra: &[&str]) -> String`: runs `anthrex run start --goal <goal> <extra…>` and returns the run id from stdout;
  - `orchestrator_window(run) -> u32`: waits with a deadline loop, up to `ORCH_WAIT`, for `RunInfo.orchestrator.window_id`;
  - `type_into(window_id, bytes)`: a real client connection sending `ClientMsg::Input`, as a user's keystrokes arrive;
  - `mcp_log() -> Vec<Value>`: the lines of `FAKE_AGENT_MCP_LOG` (`<tmp>/agent-io/mcp.jsonl`), which the harness now sets;
  - `wait_digest(run, pointer, value)`: polls `run status --json` until the JSON pointer equals the value.
- **`ORCH_WAIT` = 120 s**, derived in task M9.16 from the test configuration: `[orchestrator.planners] timeout_secs = 120` (the minimum) bounds a planner; a scripted orchestrator's longest wait is one `run_status` with `wait_secs = 50` plus one wake after `wake_quiet_secs = 1` and the 1-second tick. Scenario tests themselves wait with M8a's `RUN_WAIT` (300 s). Add the `ORCH_WAIT` row to `docs/timing-budgets.md` with that derivation.
- **Test configuration**: every M9 end-to-end test writes `[orchestrator] wake_quiet_secs = 1` and `[orchestrator.planners] timeout_secs = 120`, and uses M8b's `with_deciders("claude")` with a scripted triage answer, or no deciders for the fallback.

### M9.1 Verify external facts and reconcile names

**Files.** `docs/milestones/M9-orchestrator-and-subplanners.md` ("Implementation notes" only). No code. Run everything in a scratch repository under `/tmp`, never in this repository and never through `anthrex`.

**Checks.** Record the command, the CLI version and the observed output (trimmed) for each:

1. **Interactive Claude flags.** `claude --help` lists `--mcp-config`, `--allowedTools`, `--disallowedTools`, `--append-system-prompt`, `--strict-mcp-config`, `--effort`, `--name`, `--settings`, `--resume`. Start `claude` interactively (not `-p`) with decision 7's argv in a scratch repository and a stub MCP server (`anthrex mcp` against nothing is fine; the tool list is what matters): the TUI starts, `/mcp` lists `anthrex` connected, and the first prompt after `--` is submitted.
2. **`--disallowedTools` holds.** In that session, ask the model to write a file, run `ls`, and start a sub-agent. Each is refused without a permission prompt. If any runs, decision 7's fallback applies (`--permission-mode plan`); record which.
3. **MCP tool timeout.** Find Claude Code's default MCP tool-call timeout (documentation, or a stub tool that sleeps 70 s). If below 120 s, decision 10's `MCP_TOOL_TIMEOUT=120000` is set, and a 70 s sleep then completes.
4. **Codex flags.** `codex --help`: `-s read-only` and `-a on-request` are accepted ahead of `resume <id>`; `-c mcp_servers.anthrex.tool_timeout_sec=120` and `default_tools_approval_mode="auto"` are accepted keys; a read-only interactive Codex calls an MCP tool over the stub's Unix socket (the MCP server process is not sandboxed, but record whether the socket connect succeeds from inside `read-only`).
5. **`StopFailure` and `/compact`.** With `--settings` hooks logging every event, an interactive turn that fails on an API error fires `StopFailure` (use an invalid model name, or record that it cannot be triggered and cite the hook documentation). A manual `/compact` ends with `Stop` or leaves no turn open; if it leaves `Working`, decision 12's fallback applies.
6. **Bracketed paste.** In both TUIs, `ESC[200~` + a two-line text + `ESC[201~`, then 200 ms, then `\r`, submits the text as one message; without the delay, record whether the `\r` lands inside the paste.
7. **OTLP interactive.** An interactive Claude session with `metering::orchestrator_env`'s variables posts metrics to a local listener within its export interval.
8. **Names.** For every row of "Names taken from earlier briefs", confirm the merged code has it. Record every difference and the name used instead.
9. **Counts and version.** `wc -l` of every file in "File sizes this milestone must respect", and `PROTO_VERSION` on `main` plus one.

**Acceptance.** Each check has a recorded result. Any decision whose fallback was triggered names the check. No file other than this brief changed.

**Commit.** `docs: record the external facts and names milestone 9 builds on`

### M9.2 Protocol

**Files.** Create `crates/proto/src/orch.rs`. Modify `crates/proto/src/{lib.rs, run.rs, run_info.rs, run_wire.rs}`, and `crates/proto/tests/` (the round-trip suite M8a keeps). Update every exhaustive `match` on the extended enums in `crates/daemon`, `crates/tui` and `crates/cli` with the minimum arm (`Planning` shown as `planning`, `Reported` as `reported`, `Planner` as `planner`) so the workspace builds; behaviour comes in later tasks.

**Tests first.**
- `orch_types_round_trip`: one value of each new type and variant (every `HoldKind`, `HoldState`, `IntegrationState`) through MessagePack and JSON.
- `new_requests_round_trip`: `ApproveHold`, `RejectHold`, `StartGoal` with and without `orchestrator`, `Promote` with `orchestrator`.
- `old_run_info_still_decodes`: an M8c-shaped `RunInfo` (a JSON fixture written from M8c's type, checked in) decodes with every new field at its default.
- `appended_variants_keep_their_indices`: `AgentRole::Planner`, `RunState::Planning`, `TaskState::Reported` serialize after every older variant (MessagePack index check on the variant list).
- `proto_version_is_bumped`: `PROTO_VERSION` equals the value recorded in M9.1 item 9.
- `reported_is_finished_and_planning_is_not_terminal`.

**Acceptance.** The five AGENTS.md commands pass.

**Commit.** `feat(proto): add the orchestrator, sub-planner, hold and integration types`

### M9.3 Configuration

**Files.** Create `crates/config/src/orchestrator_agent.rs`. Modify M8a's `crates/config/src/orchestrator.rs` (one call, six fields), M8a's `RunLimits` construction in `run/plan.rs`, and M8b's `run/triage.rs` so the triage prompt's upper bound and `PLAN_SCALE_MAX`'s use read `planner_task_cap`.

**Earlier-brief change (M8b's `PLAN_SCALE_MAX`, defect 20).** Delete M8b's `run::triage::PLAN_SCALE_MAX` constant. Every place that read it reads the configured `planner_task_cap` instead: the driver passes it into the triage decider's input, and M8b's triage prompt text `plan when it needs 2 to 12 tasks` is rendered as `plan when it needs 2 to <planner_task_cap> tasks`, so with the default 12 the prompt is byte-identical to M8b's and M8b's `triage_prompt_is_exact_for_a_fixed_input` passes unchanged. `run::triage::route` and `TriageRoute` are unchanged.

**Tests first**, in `crates/config/tests/orchestrator_agent.rs`:
- `defaults_when_absent`: every key of Interfaces "config" at its default.
- `each_range_is_enforced_with_the_exact_message`: one case per numeric key at each bound and one past it.
- `effort_and_strength_and_runtime_parse`: every accepted spelling; a wrong one gives the message and the default.
- `unknown_keys_warn`: under `[orchestrator.agent]` and `[orchestrator.planners]`.
- In `crates/daemon/src/run/triage.rs` tests: `triage_prompt_uses_planner_task_cap` with a cap of 7 renders `2 to 7 tasks`.

**Acceptance.** `crates/config/src/lib.rs` unchanged. The five AGENTS.md commands pass.

**Commit.** `feat(config): add the orchestrator agent, planner and wake settings`

### M9.4 Plan rules for planners, research and review kinds

**Files.** Create `crates/daemon/src/run/orch/{mod.rs, rules.rs}` (model types of Interfaces used by the rules; the rest of `mod.rs` fills in M9.7). Modify M8a's `run/validate.rs` (decision 24; remove decision 6's refusal), `run/edits.rs` (decision 25's `amend_task deps`; a task added by a planner inherits its epic), and M8a's `research_and_review_kinds_are_deferred` test, which is deleted with a comment naming this task.

**Earlier-brief changes (M8a, defect 14).** In M8a's `run/validate.rs`:
1. The `owns_required` check (M8a decision 11, test `owns_required`) applies to `code` and `docs` tasks only. A `research` or `review` task must have an empty `owns` instead (decision 24's message). M8a's `owns_required` test keeps passing for a code task and gains a research case that is accepted with `owns = []`.
2. M8a decision 6's refusal of `kind = "research"` and `kind = "review"` is deleted, with its test `research_and_review_kinds_are_deferred` (replaced by the tests below; the deletion carries a comment naming this task).

**Tests first**, pure, in `run/orch/rules.rs` and `run/validate.rs`:
- `planner_budget_is_refused` / `plan_file_budget_is_still_accepted` (decision 23.1, and that `EditSource::User` and plan files keep M8a's rules).
- `code_task_without_scout_refs_is_refused_when_reports_exist`; `unknown_scout_ref_is_refused`; `no_reports_gives_a_note_not_an_error`; `onboarding_ref_is_accepted`.
- `orchestrator_cap_counts_tasks_without_an_epic`; `epic_cap_counts_the_epics_tasks`; `cancelled_tasks_do_not_count`; each with the exact message.
- `task_naming_an_unknown_epic_is_refused`; `task_for_a_live_planners_epic_from_the_orchestrator_is_refused`; `fix_task_for_a_finished_epic_is_accepted`.
- `research_task_with_owns_is_refused`; `review_task_needs_a_review_target`; `review_target_syntax` (a table: `main`, `a1b2c3d`, `main..feature/x`, `HEAD~3..HEAD` accepted; `-x`, `a..b..c`, `a b`, empty refused); `code_task_with_review_target_is_refused`; `research_and_review_force_test_mode_none_with_the_note`.
- `amend_deps_unblocks_dep_cancelled`; `amend_deps_rejects_a_cycle`; `amend_deps_rejects_a_cancelled_dep`.
- `rule_ids_are_the_documented_ones`: every `PlanError.rule` this file produces is in the decision-23 list.

**Acceptance.** `run/orch/rules.rs` is pure (decision 1's grep passes). The five AGENTS.md commands pass.

**Commit.** `feat(daemon): check planners' plan edits and accept research and review tasks`

### M9.5 Contracts and prompts

**Files.** Create `crates/daemon/src/run/orch/contract.rs`. Modify M8a's `run/contract.rs` (`worker_prompt`'s scout extract, decision 34, taking the extracts as an argument the driver fills).

**Tests first**, pure:
- `orchestrator_contract_is_exact` and `planner_contract_is_exact`: byte comparison with the texts in Interfaces.
- `orchestrator_contract_covers_every_planning_rule`: the contract contains each of these phrases, one assertion per phrase so a failure names it: `interface or hub change is its own task, first`, `Split one level only`, `combined size is still M, is one task`, `Size from evidence, never from time`, `Never give minutes`, `hub task: it runs alone, is tdd, and is always reviewed`, `L is never executed`, `planner_task_cap`, `Generated files`, `Protected files`, `never through a wildcard`, `is tdd: name the test to write`, `is never none`, `never give tasks on different runtimes overlapping owns`, `You never approve a task and never merge`, `never read task worktrees`, `run_status`, `wait_secs 50`, `awaiting_approval`, `mis_sized`, `question`, `summary for the user`.
- `planner_contract_covers_its_rules`: `only inside your area`, `L is never executed`, `interfaces first, one level only`, `never from time`, `Never plan a change to a hub file`, `submit_epic once`, `planner_task_cap`, `never give tasks on different runtimes overlapping owns`.
- `contracts_have_no_em_dash_and_survive_toml`: no `—`; `launch::codex::toml_string` round-trips each through the `toml` crate.
- `contracts_do_not_vary`: built twice for two different runs, byte-equal (the facts are in the prompts).
- One `*_prompt_is_exact` test per row of Interfaces "Prompts and messages", on a fixed run.
- `worker_prompt_places_the_scout_extract_before_the_brief`: order is contract-free prompt text, profile summary, extract, brief last; `extract_is_capped_at_12_kib_with_a_cut_marker`; `unreadable_report_is_left_out`.
- `wake_text_is_clamped`.

**Acceptance.** Every text in Interfaces is a `const` or a function in `run/orch/contract.rs`. The five AGENTS.md commands pass.

**Commit.** `feat(daemon): add the orchestrator and sub-planner contracts and prompts`

### M9.6 The digest, the context and the task result

**Files.** Create `crates/daemon/src/run/orch/{digest.rs, context.rs, result.rs, tools.rs}`, `crates/daemon/src/run/git/summary.rs`. Modify M8a's `run/model.rs` (`Run.digest_rev`, `digest_fp`; the other new fields arrive in M9.7 but may be added here), `run/snapshot.rs` (`RunInfo.digest_revision`).

**Tests first.**
- Pure, `digest.rs`: `digest_shape_matches_the_interface` (a fixed run rendered and compared as JSON with a checked-in fixture); `counter_changes_do_not_change_the_fingerprint` (tool calls, tokens, spend, `now`, a history line appended to a task whose state did not change); `state_block_verdict_hold_scout_planner_and_edit_changes_do` (one case each); `digest_is_capped_and_trims_in_order` (a 50-task run with 500-character blocks and 100 edits stays under `DIGEST_MAX_BYTES`, finished tasks dropped first with `omitted_tasks`); `gate_states` (planning, awaiting, approved with `approved_by`, none, a hold decided since the last read).
- Pure, `context.rs`: `context_for_the_orchestrator`; `context_for_a_planner_filters_reports_and_tasks`; `context_is_capped_at_96_kib_with_omitted_counts`; `only_listed_scouts_are_returned`; `installed_is_carried`.
- Pure, `result.rs`: `task_result_carries_every_field`; `task_without_start_has_no_git`; `git_error_is_reported`; `task_result_is_capped`.
- Pure, `tools.rs`: `parse_call_for_each_tool` (every schema property); `wait_secs_over_50_is_refused`; `tool_outside_the_role_is_refused`; `planner_cannot_call_edit_plan`; `override_op_fails_to_parse_with_the_documented_message`.
- Blocking, `summary.rs` against a temporary git repository: `task_summary_lists_commits_and_diffstat`; `resolve_target_of_a_range_and_of_a_single_revision`; `resolve_target_refuses_an_unknown_revision`; `summary_times_out` (a `git` stand-in script that sleeps, with a 1 s timeout). Every invocation passes `--no-optional-locks` and a scrubbed environment (AGENTS.md rule 11), asserted by reading the argv the stand-in logs.
- Engine: `digest_revision_moves_only_on_fingerprint_change` (a reducer step sequence).

**Acceptance.** The four `run/orch` files are pure. The five AGENTS.md commands pass.

**Commit.** `feat(daemon): build the run digest, the planning context and the task result`

### M9.7 Engine: planning, submit, holds and promotion

**Files.** Create `crates/daemon/src/run/engine/orch.rs` (and `holds.rs` if `orch.rs` passes 450 lines), `crates/daemon/src/run/orch/launch.rs` (`resolve_orchestrator`, `orchestrator_role`, `orchestrator_window_spec`). Modify M8a's `run/engine/{mod.rs, requests.rs, dispatch.rs, restore.rs}` (the runnable test's hold condition and the pre-warm skip), `run/model.rs`, `run/snapshot.rs`, `run/report.rs` (summary section), `run/reconcile.rs` (the rows of Interfaces), M8b's `run/engine` promote handling and its tests `promote_records_intent_once` (now: the first promote creates an orchestrator op, the second answers `already marked`) and `promote_refuses_plan_runs_and_terminal_runs` (unchanged texts, now also for a planned run).

**Earlier-brief changes (M8a, M8b).**
1. M8a decision 41's runnable test gains one condition: the task's `hold` is `None` or names a hold in state `Approved` (decision 28). M8a decision 14's pre-warm skips a task whose hold is not `Approved`.
2. M8b's promote handling (M8b decision 25) is replaced by decision 29. M8b's test `promote_records_intent_once` is rewritten: the first promote emits `OpKind::CreateOrchestrator` and the second answers `run <id> was already marked for promotion at <hh:mm>`. `promote_refuses_plan_runs_and_terminal_runs` keeps its texts and gains a planned-run case.

**Tests first**, pure reducer tests in `run/engine/orch.rs`:
- `planned_run_starts_in_planning_and_creates_branch_then_orchestrator` (effects in that order; no task, no pre-warm).
- `resolve_orchestrator_order` (choice, `[orchestrator.agent]`, default; unknown model refused with the exact text; Codex falls back to `""` at standard).
- `approve_while_planning_is_refused`; `reject_while_planning_discards`.
- `submit_with_no_tasks_is_refused`; `submit_while_a_planner_is_live_is_refused`; `submit_opens_the_gate`; `submit_with_yes_runs_at_once_and_records_approved_by`; `resubmit_in_awaiting_approval_changes_nothing`; `spawn_subplanner_in_awaiting_approval_returns_to_planning`.
- `edit_plan_is_one_batch` (one bad edit rejects all; the run is unchanged, including `digest_rev`); `edit_plan_reply_shapes` (accepted and rejected JSON exactly).
- `summary_on_a_complete_run_is_accepted_and_edits_are_not`; `summary_is_written_to_the_report`.
- `new_epic_on_a_running_run_is_held`; `held_tasks_are_not_runnable_or_prewarmed`; `approve_hold_releases_its_tasks`; `reject_hold_cancels_its_tasks_and_blocks_dependents`; `hold_verdict_errors` (unknown hold, already decided); `with_yes_an_epic_hold_is_approved_on_submit`.
- `promote_creates_an_orchestrator_and_leaves_t1_running`; `tasks_added_before_submit_after_promote_carry_the_promotion_hold`; `promotion_hold_is_awaiting_after_submit`; `pre_m9_promote_request_is_performed_on_resume_and_on_tick`; the M8b attention line `promotion requested at …` is removed on promotion.
- `restore_pauses_a_planning_run_and_resume_restarts_the_orchestrator`; `resume_of_awaiting_approval_restarts_a_dormant_orchestrator_without_changing_state`.
- `orchestrator_calls_from_another_window_are_refused` with the exact text.
- `planned_message_is_exact`.
- Reconcile: `create_orchestrator_is_replayed_from_a_restored_window`; `start_scout_is_not_replayed`.
- `rejected_plan_is_seen_by_run_status` (after `Reject`, the digest's `run.state` is `discarded`, and a later `edit_plan` answers `run <id> is discarded`).

**Acceptance.** The five AGENTS.md commands pass.

**Commit.** `feat(daemon): plan a goal with an orchestrator, and hold work added after the gate`

### M9.8 Engine: sub-planners and run scouts

**Files.** Create `crates/daemon/src/run/engine/planners.rs`. Modify `run/orch/launch.rs` (`planner_spec`, `scout_spec`), M8a's `run/engine/dispatch.rs` (reader-slot order of decision 31), the round lookup (`RoundOwner`, decision 32), `run/snapshot.rs` (`RunInfo.planners`, decision 33), M8a's `headless/{mod.rs, argv.rs}` (`McpTarget.epic`, `--epic`).

**Earlier-brief change (M8a's `McpTarget`, defect 18).** In M8a's `headless/mod.rs`, `McpTarget` gains `#[serde(default)] pub epic: Option<String>`, appended after M8b's `scout_id`; in `headless/argv.rs`, `mcp_args` appends `--epic <e>` when it is set, after every existing argument. M8a's and M8b's `mcp_args_for_a_worker` and `mcp_args_for_a_scout` pass unchanged, and a persisted `HeadlessSpec` without `epic` still loads.

**Tests first**, pure:
- `spawn_subplanner_validates_epic_and_area` (pattern, `/**` form, overlap with the exact text); `spawn_subplanner_sets_path_large`.
- `planner_waits_for_a_reader_slot_and_the_order_is_deciders_reviewers_planners_scouts_kinds`.
- `planner_spec_is_read_only_and_names_its_epic` (allowed tools, plan mode or fallback, `codex_sandbox`, `mcp_args` ends with `--epic <e>`, window name `<h4>/plan-<e>.p1`).
- `submit_epic_outside_the_area_is_rejected_with_the_area_error`; `submit_epic_refuses_answer_pause_resume_finish`; `submit_epic_for_another_epic_is_refused`; `accepted_submit_finishes_and_retires`; `second_submit_is_refused`; `rejections_count_and_fail_at_max_rejections`.
- `turn_without_submit_is_nudged_once_then_fails`; `tool_call_wrap_up_then_kill`; `timeout_fails`; `death_resumes_once`.
- `replan_starts_a_fresh_session_with_the_current_tasks`; `replan_of_a_live_planner_is_refused`.
- `failed_planner_keeps_accepted_tasks_and_wakes_the_orchestrator`.
- `spawn_scout_queues_in_a_reader_slot_and_replies_at_once`; `scout_id_is_prefixed_and_unique`; `max_scouts_is_enforced`; `scout_ended_records_the_report_and_usage`; `restore_fails_running_scouts_with_the_documented_reason`.
- `planners_snapshot_fields` (every `PlannerInfo` field).

**Acceptance.** The five AGENTS.md commands pass.

**Commit.** `feat(daemon): run sub-planners per epic and scouts per area`

### M9.9 Engine: research, review, integration, completion and wake notes

**Files.** Create `crates/daemon/src/run/engine/{kinds.rs, integration.rs}`. Modify `run/orch/launch.rs` (`research_spec`, `review_task_spec`, `integration_reviewer_spec`), M8a's runnable test (decision 35 dependents; the hold condition is M9.7's), completion (decision 38), `run/engine/done.rs` (decision 25's restart on rewrite), `run/report.rs` (`## Research`, `## Review findings`), M8b's scout routing branch for task-bound scouts (decision 35).

**Earlier-brief changes (M8a, M8b; defects 15, 19).**
1. M8a decision 41's runnable test: a **declared** dependency is satisfied when it is `merged` **or `reported`** (was: `merged`). Implicit dependencies are unchanged (research and review tasks have no `owns`, so they never create one).
2. M8b decision 15's tool routing gains one branch ahead of `ScoutService::tool`: a call with `role == Scout`, `task_id` set and `scout_id` unset goes to the engine (`on_task_scout_report`).

**Tests first**, pure:
- `research_task_takes_a_reader_slot_and_no_worktree`; `research_report_marks_the_task_reported`; `research_without_report_is_nudged_then_blocked`; `dependent_of_a_reported_task_runs`.
- `review_task_resolves_its_target_first`; `unresolvable_target_blocks_environment_with_the_text`; `review_verdict_reports_whatever_it_is`; `review_findings_go_to_the_report`.
- `integration_review_starts_when_the_epic_is_merged`; `integration_changes_holds_completion`; `fix_task_merge_starts_round_two`; `finish_closes_changes`; `no_round_after_max_bounces_plus_one`; `plan_path_has_no_integration_review`; `integration_reviewer_route_is_the_peer_at_frontier`.
- `completion_waits_for_holds_planners_scouts_integration_and_submit` (one case per condition).
- `rewriting_a_mis_sized_task_restarts_it_at_rung_2`; `editing_a_human_blocked_task_does_not_restart_it`.
- Wake notes: `each_note_source_adds_its_exact_line` (a table over decision 39's list); `orchestrators_own_edits_add_no_note`; `notes_are_capped_at_20_with_the_earlier_line`; `wake_effect_needs_live_window_setting_and_new_revision`; `digest_read_drops_notes_up_to_the_revision`; `woken_clears_delivered_notes`; `completion_note`.
- `edit_log_records_every_source_and_keeps_100`.

**Acceptance.** The five AGENTS.md commands pass.

**Commit.** `feat(daemon): execute research and review tasks, review each epic, and note what the orchestrator must see`

### M9.10 The orchestrator window

**Files.** Create `crates/daemon/src/launch/role.rs`, `crates/daemon/src/manager/role_window.rs`, `crates/daemon/tests/orchestrator_window.rs`. Modify `launch/{mod.rs, claude.rs, codex.rs}`, `hooks.rs`, `manager/{create.rs, restart.rs, entry.rs, restore.rs}`, `window.rs`, `server.rs`, M8a's `server/headless_guard.rs`.

**Tests first.**
- Pure, `launch/role.rs` and `launch/mod.rs`: `claude_orchestrator_argv_is_exact` (the whole argv for a fixed role, in decision 7's order, with `--` before the prompt; with `--resume` instead); `claude_argv_without_user_settings_only_caps` (no exclusion flags; `--strict-mcp-config` per decision 7); `codex_orchestrator_argv_is_exact` (decision 8's block before `-m`, then `resume <id>`); `plain_windows_are_unchanged` (M3's argv tests pass untouched, and `role: None` produces byte-identical argv); `role_env_order_and_otlp` (M3's four variables first, then OTLP, then `MCP_TOOL_TIMEOUT` when M9.1 set it).
- `claude_settings_json_is_exact` updated for 11 events; `stop_failure_parses_and_maps_to_stop`.
- Real PTY, in `crates/daemon/tests/orchestrator_window.rs`, with `fake-agent` as the runtime binary:
  - `run_window_is_pty_with_run_ref_and_counts_to_max_windows`;
  - `scrub_removes_agent_session_variables` (the daemon test process sets `CLAUDECODE=1` and `CLAUDE_CODE_ENTRYPOINT=x`; `fake-agent` writes its environment to a file; neither is present, `ANTHREX_WINDOW_ID` is);
  - `restart_repasses_role_flags_with_resume` (the args file after `restart` has `--mcp-config`, `--disallowedTools` and `--resume <id>`);
  - `role_survives_a_daemon_restart` (persisted `{"role_launch": …}`; after restore and restart, same flags); `unparseable_role_restores_a_plain_window_with_a_warning`;
  - `kill_and_remove_are_refused_while_the_run_is_live` (exact `DaemonMsg::Error`), `and_allowed_once_it_is_terminal`;
  - `client_input_time_is_recorded`;
  - `stop_failure_hook_makes_the_window_idle` (a script sends `UserPromptSubmit` then `StopFailure` through `anthrex hook`; status goes `Working` then `Idle`).
- `create_run_window_does_not_hold_the_manager_lock_across_spawn`: the existing M1 lock test pattern (a second `list` answers while a slow `spawn` is in progress), AGENTS.md rule 2.

**Acceptance.** `create.rs` and `restart.rs` grow within their budgets. The five AGENTS.md commands pass.

**Commit.** `feat(daemon): launch the orchestrator read-only in a PTY window, and keep its role across restarts`

### M9.11 MCP tools

**Files.** Create `crates/mcp/src/tools_orch.rs`. Modify M8a's `crates/mcp/src/{tools.rs, lib.rs}`, `crates/cli/src/main.rs` (hidden `mcp` flags `--epic`, `--task` for `scout`), M8a's `run/driver.rs` routing (decision 15), `run/driver/orch.rs` (new: `orchestrator_read`, the long-poll).

**Earlier-brief change (M8b's `--role scout`, defect 19).** M8b's `anthrex mcp` argument check for `--role scout`, which requires `--scout <id>`, becomes: exactly one of `--scout <id>` or `--task <t>`. Neither or both exits with a usage error naming the two flags. With `--task`, `McpOptions.task_id` is set and `scout_id` is `None`. M8b's scout MCP tests pass unchanged.

**Tests first.**
- `crates/mcp`: `tools_for_orchestrator_and_planner_are_exact` (names, order, descriptions); `every_schema_is_closed_at_every_level` (walks each schema); `schemas_match_the_interface_table` (checked-in JSON fixtures); `planner_needs_epic_and_orchestrator_refuses_it`; `scout_accepts_task_instead_of_scout`; `tool_outside_the_role_is_refused_without_a_daemon` (a socket path that does not exist).
- Driver, with a real daemon socket and `fake-agent` absent (calls made by a test MCP client, M8a's pattern):
  - `run_status_returns_at_once_without_since`;
  - `run_status_waits_until_the_digest_changes` (a call with `wait_secs = 20` returns within 5 s of a state change made by `run edit`, and its `revision` is higher);
  - `run_status_times_out_with_the_same_revision` (`wait_secs = 2`, answered after 2 s and before 4 s);
  - `a_counter_change_does_not_end_the_wait` (a worker tool-call event during the wait; the call still takes `wait_secs`);
  - `terminal_run_ends_the_wait`;
  - `long_poll_holds_no_engine_lock` (a second `run status` answers while a `run_status` waits);
  - `get_context_and_task_result_answer_from_the_driver`.

**Acceptance.** The five AGENTS.md commands pass.

**Commit.** `feat(mcp): serve the orchestrator's and sub-planners' tools, with a long-polled run digest`

### M9.12 `fake-agent`: an orchestrator in a PTY, planners and run scouts

**Files.** Create `crates/fake-agent/src/orch_steps.rs`, `crates/fake-agent/tests/orch_modes.rs`. Modify `crates/fake-agent/src/{main.rs, script.rs, mcp.rs, roles.rs}`.

**Change.**
1. **PTY mode with MCP.** In PTY mode (no `-p`, no `exec`), when the argv carries an MCP server (Claude `--mcp-config`, Codex `-c mcp_servers.anthrex.*`), the role and task are parsed as in headless mode, and `mcp_call` works exactly as there. `--role orchestrator` claims `orchestrator-run-<n>.jsonl`; with no such script, M3's `FAKE_AGENT_SCRIPT`.
2. **PTY `read_message {timeout_ms?, expect?}`.** Sends a `Stop` hook through `anthrex hook` (the turn ends), then reads stdin: a bracketed paste (`ESC[200~ … ESC[201~`) followed by `\r`, or typed bytes up to `\r`. It then sends `UserPromptSubmit` with the text as `prompt`, keeps it as `FAKE_AGENT_MESSAGE`, and continues. Exit codes as in M8a.
3. **Script names.** `--role planner --epic <e>` claims `planner-<e>-<n>.jsonl`; `--role reviewer --epic <e>` claims `reviewer-epic-<e>-<n>.jsonl`; `--role scout --scout <h4>-<id>` claims `scout-<id>-<n>.jsonl` (the run prefix stripped); `--role scout --task <t>` claims `scout-<t>-<n>.jsonl`.
4. **New steps.**
   - `mcp_until {tool, args, until: {pointer, equals}, timeout_ms}`: repeats `mcp_call` until the JSON pointer into the result equals the value; exits 4 at the timeout.
   - `capture_json {name, pointer}`: `{{name}}` in later arguments becomes the value at the pointer of `FAKE_AGENT_RESULT` (a string as is, anything else as JSON).
   - `expect {pointer, equals}` on `FAKE_AGENT_RESULT`: exits 3 when it differs.
   - `expect_error_contains {text}` on the last `mcp_call` with `expect_error`.
5. **`FAKE_AGENT_MCP_LOG`.** When set, every `mcp_call` appends `{"script","tool","args","ok","result"}` as one JSON line.

**Tests first**, in `orch_modes.rs`, with a stub MCP server built as in M8a:
- `pty_mode_parses_the_mcp_role_from_claude_and_codex_argv`;
- `pty_read_message_takes_a_bracketed_paste_and_typed_input` (both forms, through a real PTY; the hook log shows `Stop` then `UserPromptSubmit`);
- `script_names_for_planners_integration_reviewers_and_run_scouts`;
- `mcp_until_polls_until_the_pointer_matches`, `and_times_out`;
- `capture_json_substitutes`; `expect_fails_with_exit_3`;
- `mcp_log_records_each_call`.

**Acceptance.** M3's and M8a's `fake-agent` tests pass unchanged. The five AGENTS.md commands pass.

**Commit.** `feat(fake-agent): script an orchestrator in a PTY, sub-planners and run scouts`

### M9.13 Driver: ops and wake delivery

**Files.** Create `crates/daemon/src/run/driver/{orch_ops.rs, wake.rs}`. Modify M8a's `run/driver.rs`, `run/driver/ops.rs` (dispatch to `orch_ops`), `RunContext` (scouts, profiles, binaries), and the `StartGoal` path of M8b (decision 26 replaces step 6's refusal; decision 9's check).

**Earlier-brief change (M8a decision 29, defect 16).** M8a decision 29 removed paste delivery and delivers every engine message as a headless turn. That stays exactly as it is for every headless session. Paste delivery is added back in one place only, `run/driver/wake.rs`, for the orchestrator's PTY window (decision 39); nothing in M8a's outbox or its `Effect::Deliver` changes, and no headless window ever receives a paste. M8b decision 22 step 6's refusal of the plan and large paths is replaced by decision 26.

**Tests first.**
- Pure, `wake.rs`: `encode_paste_wraps_and_normalises` (`\r\n` and `\n` to `\r`, markers stripped, framed); `wake_is_clamped`.
- Driver tests with a real daemon and `fake-agent`:
  - `create_orchestrator_op_starts_the_window_and_reports_it`;
  - `restart_orchestrator_op_resumes_it`;
  - `start_scout_op_runs_a_scout_and_sends_scout_ended`;
  - `resolve_target_op_runs_git_off_the_worker_threads` (on `spawn_blocking`, with the timeout);
  - `wake_is_delivered_only_when_idle_and_quiet` (the window `Working`: nothing; `Idle` with input 0.2 s ago and `wake_quiet_secs = 1`: nothing; after 1 s: delivered, and the `fake-agent` stdin file shows the framed paste then `\r`);
  - `wake_is_not_delivered_on_attention`;
  - `a_newer_wake_replaces_an_undelivered_one`;
  - `wake_writes_happen_outside_every_lock` (the manager answers `list` while a wake's 200 ms delay is pending);
  - `start_goal_on_the_plan_path_builds_a_planned_run` (replaces M8b's refusal);
  - `project_settings_check_covers_the_orchestrator` (refused without `--trust-project` when the caps are `None`, with the test placeholder M8a uses);
  - `orchestrator_exit_adds_the_attention_line_and_suspends_wakes`.

**Acceptance.** No blocking call on a tokio worker thread and none under `daemon::lock` (review by `grep` of the two new files for `std::process`, `std::fs` and `.lock()`, each allowed only inside `spawn_blocking` closures or through `daemon::lock`). The five AGENTS.md commands pass.

**Commit.** `feat(daemon): drive the orchestrator's ops and wake it when the run needs it`

### M9.14 CLI

**Files.** Create `crates/cli/src/run_cmd/orch.rs`. Modify M8a's `crates/cli/src/run_cmd.rs`, `run_cmd/status.rs`, M8b's goal and promote commands, `crates/cli/tests/run_cli.rs` (M8a's CLI test file).

**Tests first**, in `run_cli.rs` against a real daemon:
- `orchestrator_flag_parses_and_refuses_bad_values` (exact message); `orchestrator_flag_is_refused_with_plan`.
- `start_goal_on_the_plan_path_prints_the_planned_message` (stderr exact, stdout the run id, exit 0).
- `approve_and_reject_hold`; `approve_without_hold_names_waiting_holds`.
- `status_shows_orchestrator_planners_holds_and_summary_lines` (exact lines); `status_json_carries_the_new_fields`.
- `accept_prints_the_research_report_and_the_nothing_to_merge_question`.
- `promote_with_orchestrator_choice`.

**Acceptance.** The five AGENTS.md commands pass.

**Commit.** `feat(cli): start planned goals, approve holds, and show the orchestrator in run status`

### M9.15 TUI

**Files.** Modify M8c's `crates/tui/src/app/runs.rs`, `tree/runs.rs`, `tree/run_rows.rs`, `inspector/run.rs`, `theme.rs`, `graph/run_text.rs`, and their tests.

**Change.** A `planning` run shows `planning` in its header and its orchestrator node; `Reported` tasks get `✓` in the finished colour and the label `reported`; a held task shows `○` and `held` after its stage; an awaiting hold appears in the run's attention list as `hold <id>: <n> tasks wait for approval` and `a` / `x` on that line send `ApproveHold` / `RejectHold` (with the M8c confirmation prompt for `x`); planner nodes use `PlannerInfo` with M8c's content text (`planner {epic} {title}  {merged}/{total}`) and planner glyphs, now filled (decision 33); a research session's round label is `research #n`; Enter on the orchestrator node focuses its PTY window (spec §16.4, M8c's handling of PTY run windows). The reducer stays pure (AGENTS.md rule 5): each key returns `Effect`s.

**Tests first.** `planning_run_header`; `reported_and_held_glyphs`; `hold_attention_line_and_keys_emit_the_requests`; `planner_nodes_from_planner_info`; `research_round_label`; `enter_on_the_orchestrator_focuses_its_window`; M8c's mockup tests updated only where a new field appears, each change listed in "Implementation notes".

**Acceptance.** `crates/tui/src/app.rs` and `ui/**` do no I/O. The five AGENTS.md commands pass.

**Commit.** `feat(tui): show planning runs, holds, planners and reported tasks in the run view`

### M9.16 End-to-end I: the plan path, steering and reactions

**Files.** Create `crates/cli/tests/run_e2e_orch.rs`. Modify `crates/cli/tests/support/run_harness.rs` (the helpers above), `docs/timing-budgets.md` (`ORCH_WAIT`), and M8b's `run_e2e_*` tests: `e2e_goal_needing_a_plan_is_refused_without_side_effects` becomes `e2e_goal_needing_a_plan_starts_a_planning_run`; `e2e_goal_without_deciders_takes_the_plan_path` now expects an orchestrator window instead of a refusal; `e2e_promote_records_intent_and_the_task_continues` now expects the orchestrator and the promotion hold.

**Tests first.** Every scenario uses scripted `fake-agent`s (orchestrator PTY scripts, headless workers and reviewers), and `mcp_log()` for what the orchestrator saw.
- `e2e_plan_path_scouts_plans_and_reads_the_approval`: triage `plan`. `orchestrator-run-1`: `get_context`; `spawn_scout {id: "core", …}`; `mcp_until run_status` until `/scouts/0/state == "reported"`; `edit_plan` with two S tasks naming `<h4>-core` in `scout_refs` and `submit: true`, `expect /awaiting_approval == true`; `mcp_until run_status` until `/gate/state == "approved"`; `mcp_until run_status` until `/run/complete == true`; `edit_plan {edits: [], summary: "…"}`. The test waits for `awaiting_approval`, sends `run approve`, waits for `complete`; asserts both tasks merged, `REPORT.md` starts with the summary section, and the orchestrator's `run_status` calls each returned within `wait_secs + 5` s.
- `e2e_rejected_plan_discards_the_run_and_the_orchestrator_learns_it`: after submit the test sends `run reject`; the script's `mcp_until` sees `/run/state == "discarded"`, then an `edit_plan` with `expect_error` containing `is discarded`; afterwards `anthrex rm <orchestrator window>` succeeds.
- `e2e_typed_steering_becomes_a_plan_edit`: after approval the script does `read_message {expect: "skip t2"}` then `edit_plan [cancel_task t2]`. The test `type_into`s `skip t2\r`; asserts `t2` cancelled with `edit_log` source `orchestrator`.
- `e2e_mis_sized_task_is_split_by_the_orchestrator`: `worker-t1-*` scripts fail twice so `t1` reaches rung 3; the orchestrator's `read_message {expect: "t1 blocked (mis_sized)"}` (the wake) then `split_task t1 into [t1a, t1b]`; both merge.
- `e2e_blocked_question_is_answered_after_a_wake`: the worker calls `task_blocked {reason: "question", …}` then `read_message {expect: "use tabs"}`; the orchestrator's `read_message {expect: "t1 blocked (question)"}` then `edit_plan [answer t1 "use tabs"]`; `t1` merges. The stdin file of the orchestrator shows the paste framing.
- `e2e_promote_starts_an_orchestrator_and_holds_its_tasks`: a fast-path run whose `t1` waits on `read_message`; `run promote`; an orchestrator window appears; its script adds `t2` and submits; `t2` shows hold `promotion` awaiting; `t1` keeps running and merges; `run approve --hold promotion` releases `t2`.
- `e2e_orchestrator_cannot_approve_merge_or_write`: the script calls `edit_plan` with `{"op": "override", …}` (error text asserted), `task_done` (refused by `anthrex mcp` for the role), and the fake runtime's argv shows `--disallowedTools Edit,Write,NotebookEdit,Bash,Agent`; the run branch has no commit the orchestrator could have made.
- `e2e_wake_does_not_collide_with_typing`: while the test types into the orchestrator every 300 ms for 3 s (`wake_quiet_secs = 1`), a task blocks; no paste reaches the window until 1 s after the typing stops (asserted from the stdin file's timestamps written by `fake-agent`).

**Acceptance.** All pass three times in a row (`cargo test -p anthrex --test run_e2e_orch -- --test-threads=1`, repeated). The five AGENTS.md commands pass.

**Commit.** `test: cover the plan path, the gate, steering and the orchestrator's reactions end to end`

### M9.17 End-to-end II: the large path, kinds, restart, metering and the smoke stage

**Files.** Create `crates/cli/tests/run_e2e_large.rs`, `scripts/pty_smoke_orch.py`. Modify `scripts/pty-smoke.py` (import and call), `docs/ROADMAP.md` (milestone 9 `done`), `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` ("From milestone 9", the follow-ups of this brief).

**Tests first.**
- `e2e_large_path_two_subplanners_submit_their_epics`: triage `large`. The orchestrator adds interface task `t0` (hub), `spawn_subplanner a` (area `src/a/**`) and `b` (`src/b/**`), waits with `mcp_until run_status` for `/planners/*/state == "finished"` (two pointers), then submits. `planner-a-1` and `planner-b-1` call `get_context`, then `submit_epic` with tasks depending on `t0`. The run completes; `RunInfo.planners` has both with `edits_accepted`; both planner windows were retired.
- `e2e_subplanner_edit_outside_its_area_is_rejected_then_fixed`: `planner-a-1` first submits a task owning `src/b/x.rs` (`expect_error_contains "src/b/x.rs"` and the area wording), then a corrected batch; the digest shows `rejected: 1` and `last_rejection`.
- `e2e_new_epic_after_approval_is_held_until_approved`: after approval the orchestrator spawns epic `c`; its tasks show hold `epic:c` awaiting; the other epics' tasks keep running; `run approve --hold epic:c` releases them; the orchestrator's `run_status` sees the hold `approved`.
- `e2e_integration_review_asks_for_changes_and_a_fix_task_closes_it`: `reviewer-epic-a-1` submits `changes` with one critical finding; the run is not complete; the orchestrator's wake reads `integration review of epic a: changes (1 critical, 0 important)`; it adds fix task `a9` in epic `a`; after `a9` merges, `reviewer-epic-a-2` approves; the run completes.
- `e2e_research_goal_reports_without_merging`: triage kinds `[research]`; the orchestrator adds one research task; `scout-t1-1` submits a report; `t1` is `reported`; `run accept` prints `research report: <path>` and asks the nothing-to-merge question; the base branch is unchanged.
- `e2e_review_goal_reviews_a_range_without_merging`: the repository has a branch `feature` two commits ahead; the orchestrator adds a review task with `review_target = "main..feature"`; the reviewer's prompt names both shas; its `changes` verdict still ends `reported`; the findings are in `REPORT.md`; nothing merged.
- `e2e_restart_resumes_the_orchestrator_with_its_role_flags`: during `running`, `restart_daemon`; the run is `paused`; `run resume`; the orchestrator's args file has a second line with `--resume` and every role flag; its first wake contains `the daemon restarted`.
- `e2e_claude_orchestrator_gets_the_otlp_environment`: with M8b's receiver running (`<data_dir>/otlp.addr` exists), the orchestrator's environment file has M8b's OTLP variables and `anthrex.role=orchestrator`; a Codex orchestrator's does not, and `run status --json` has no orchestrator usage.

**Smoke stage.** In `scripts/pty_smoke_orch.py`, `orch_stage(env, bin_path)`, called from `pty-smoke.py` after stages 11c, 11d and 11e (M8a, M8b, M8c), printing `== stage 11f: a goal is planned by a scripted orchestrator ==`. With the smoke's isolated daemon, `fake-agent` as both runtimes and M8b's scripted triage decider (`plan`): `run start --goal "add two files"`; open the TUI, `C-b T`, wait for the run's `planning` header, Enter on the orchestrator node and wait for its window; the scripted orchestrator adds two S tasks and submits; press `a` in the run view; wait for `complete`; type `done\r` into the orchestrator window (its script's `read_message {expect: "done"}` then writes the summary); `run accept --yes`; both files on `main`. Every wait is a deadline loop bounded by `RUN_WAIT`.

**Acceptance.** `python3 scripts/pty-smoke.py` passes with stage 11f. After it, `pgrep -fl "anthrex daemon"` and `pgrep -fl fake-agent` show nothing of yours. All five AGENTS.md commands pass. Milestone 9 is `done` in `docs/ROADMAP.md`.

**Commit.** `test: cover the large path, research and review goals, restart and metering end to end, and add a smoke stage`

## Verification

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

All five pass. Also:

- `grep -rn "std::process\|std::fs\|tokio::process\|\.lock()" crates/daemon/src/run/orch crates/daemon/src/run/engine/{orch,planners,kinds,integration}.rs crates/daemon/src/launch/role.rs` prints nothing (decision 1's purity).
- `grep -rn "\.lock()\.unwrap()" crates/daemon/src` prints nothing new (AGENTS.md rule 3).
- `grep -rn -- "--no-optional-locks" crates/daemon/src/run/git/summary.rs` shows one per git invocation (AGENTS.md rule 11).
- `wc -l` of every file in "File sizes this milestone must respect" is within its budget, and no new file passes 600 lines.
- `pgrep -fl "anthrex daemon"` shows nothing of yours.

## Manual check

With real `claude` and `codex` installed and logged in. Never run `anthrex` without the two variables (AGENTS.md rule 1).

```bash
export ANTHREX_SOCKET=/tmp/ax-m9/d.sock ANTHREX_DATA_DIR=/tmp/ax-m9/data
mkdir -p /tmp/ax-m9 && cargo new --vcs git /tmp/ax-m9/repo && cd /tmp/ax-m9/repo && git add -A && git commit -qm init
anthrex daemon start
anthrex profile detect && anthrex profile confirm        # M8b
anthrex run start --goal "Add a --json flag that prints the greeting as a JSON object, with a test and a README section"
anthrex                                                   # C-b T, Enter on the run's orchestrator
```

Check and record under "Implementation notes":

1. The orchestrator calls `get_context`, spawns at least one scout, and writes S or M tasks with `scout_refs`, test modes with reasons, and routes; no task has a budget; no L survives.
2. It submits, and waits in `run_status` rather than asking you to approve in chat. Approve with `a` in the run view; it notices within a minute.
3. Type `skip the docs task` into its window; it cancels that task with one line of explanation.
4. Ask it to write a file or run a command; it cannot.
5. When a task blocks (edit a worker's brief to be ambiguous if none does), you see a wake line and its reaction.
6. When the run completes, it writes a summary. `anthrex run accept <run>`.
7. Repeat once with `--orchestrator codex`, and once with a goal that the triage labels `large` (for example "Add a Gemini runtime alongside Claude and Codex") far enough to see two sub-planners submit.
8. `anthrex daemon stop` with the same variables, then `pgrep -fl "anthrex daemon"` shows nothing of yours.

```bash
anthrex daemon stop
```

## Review focus

Five failure modes the reviewer checks first, each with the test that covers it in its owning task:

1. **A wake pasted into the user's typing or into a permission prompt.** The paste lands only when the window is `Idle` or `Done` and quiet for `wake_quiet_secs`. Tests: `wake_is_delivered_only_when_idle_and_quiet`, `wake_is_not_delivered_on_attention` (M9.13), `e2e_wake_does_not_collide_with_typing` (M9.16).
2. **A long-poll that never waits, or never wakes.** If counters move the digest revision, every worker's tool call ends the orchestrator's wait and it spins; if the fingerprint misses a field, a blocked task never reaches it. Tests: `counter_changes_do_not_change_the_fingerprint`, `state_block_verdict_hold_scout_planner_and_edit_changes_do` (M9.6), `a_counter_change_does_not_end_the_wait` (M9.11).
3. **A restarted orchestrator without its role flags.** Resume restores none of them; a plain `claude --resume` would be a writable session with no MCP tools. Tests: `restart_repasses_role_flags_with_resume`, `role_survives_a_daemon_restart` (M9.10), `e2e_restart_resumes_the_orchestrator_with_its_role_flags` (M9.17).
4. **A sub-planner that loops or goes silent.** Repeated rejections, turns without a submit, tool-call runaways and hangs must all end in a failed planner the orchestrator is told about, never a run stuck in `planning`. Tests: `rejections_count_and_fail_at_max_rejections`, `turn_without_submit_is_nudged_once_then_fails`, `tool_call_wrap_up_then_kill`, `timeout_fails`, `failed_planner_keeps_accepted_tasks_and_wakes_the_orchestrator` (M9.8).
5. **Unbounded replies.** A 50-task run, long block texts and many scout reports must not push a tool reply past what the agent can take. Tests: `digest_is_capped_and_trims_in_order`, `context_is_capped_at_96_kib_with_omitted_counts`, `task_result_is_capped` (M9.6).

## Risks and gotchas

1. **Interactive TUIs are not stream-json.** The engine learns the orchestrator's state only from hooks (`Stop`, `StopFailure`, `UserPromptSubmit`) and M3's status machine. A turn that ends without either hook leaves the window `Working` and wake-ups wait; `StopFailure` (decision 12) closes the known gap, and M9.1 item 5 checks `/compact`.
2. **`--disallowedTools` is the read-only guarantee for Claude.** If a future Claude Code version lets a disallowed tool through, the orchestrator could write to the user's checkout. M9.1 item 2 checks it once; the manual check repeats it.
3. **Codex read-only and the MCP socket.** Codex's `read-only` sandbox may block the MCP server's socket connection on some versions; M9.1 item 4 records it. Without it, the Codex orchestrator has no tools.
4. **Bracketed paste and `\r` timing.** If `SUBMIT_DELAY` is too short, the `\r` lands inside the paste and the message is not submitted; the wake then sits in the input box. M9.1 item 6 measures it on both TUIs.
5. **Wake storms.** A busy run changes the digest often. One pending wake per run, delivered only on idle and quiet, and cleared by a `run_status` read, bounds this to at most one paste per orchestrator turn.
6. **Approval by habit.** Models may ask the user "shall I approve?" in chat. The contract says the gate is in the run view (rule 8), and no tool can approve; the manual check looks for it.
7. **The cap versus a real large goal.** A goal with 40 tasks needs at least three sub-planners at the default cap of 12. If the orchestrator writes 13 tasks itself, the cap rejects the batch with a message that names sub-planners.
8. **Stale M8b tests.** Three M8b end-to-end tests and two unit tests encode the refusal and the recorded-only promotion; they are rewritten in M9.7 and M9.16, and nothing else of M8b's suite may change.
9. **User settings only.** If `CLI_CAPS.claude_user_settings_only` is `None`, the orchestrator is refused on a repository with project settings unless the user passes `--trust-project`; this is intentional (decision 9) but surprising the first time.
10. **Socket paths.** Every new test directory is under `/tmp` (`tempfile::Builder::new().prefix("ax-orch").tempdir_in("/tmp")`).

## Follow-ups handled

| Follow-up | Where it was recorded | Handled by |
|-----------|----------------------|------------|
| `StopFailure` not mapped for PTY Claude windows | M8a Out table and follow-up | Decision 12, task M9.10 |
| Research and review kinds deferred | M8a decision 6 | Decisions 24, 35, 36, tasks M9.4, M9.9 |
| Goals on the plan and large paths refused | M8b decision 22 step 6 | Decision 26, task M9.13 |
| `run promote` only recorded | M8b decision 25 | Decision 29, task M9.7 |
| OTLP receiver waiting for the orchestrator | M8b decision 30 | Decisions 10, 14, task M9.17 |
| `RunInfo.planners` placeholder | M8c "Consumes from later milestones" | Decision 33, task M9.8 |
| Triage's "2 to 12 tasks" constant | M8b `PLAN_SCALE_MAX` | Decision 3, task M9.3 |

Recorded as new follow-ups in task M9.17: OTLP metering of a Codex orchestrator (decision 14); a TUI form for starting a goal run; wake delivery for a Codex orchestrator if M9.1 item 6 finds its paste handling unreliable.

## Spec and brief defects

Found while writing this brief. Each is resolved by the decision named. Items 1–10, 21 and 22 were then written into the spec, in the section each belongs to, marked "settled in the briefs, 2026-09-22"; items 11–20 are between briefs and resolved in them.

1. **§12.1 has no submit boundary.** It lets the orchestrator edit the plan freely but never says when the plan is "the plan" the gate shows. Resolved by `edit_plan { submit: true }` (decision 27).
2. **§12.3 has no state for planning, and no mechanism for approving a new epic mid-run.** "Non-blocking" plus "any later edit that adds an epic" waits for approval needs somewhere for the waiting tasks to be. Resolved by `RunState::Planning` (decision 26) and holds (decision 28).
3. **§5.2 and §18: research and review tasks have no terminal state and review has no target field.** "No merge" means `merged` never happens, but M8a's runnable test and completion need a finished state; a review needs a range. Resolved by `TaskState::Reported` and `PlanTask.review_target` (decisions 24, 35, 36).
4. **§5.3 step 6 does not say what a rejected integration review does.** Resolved by decision 37: completion waits for a fix task or `finish`; nobody approves by edit.
5. **§10 rung 3 raises a task to L, but §11.4 step 5 lets the orchestrator "rewrite" it.** A rewritten task whose size stays L could never run. Resolved by decision 25: the rewrite is a new size claim, re-checked by M8a's rules.
6. **§3 says the orchestrator never reads checkouts, but §4 gives it the repository to read.** Resolved by decisions 5 and 18: it reads the user's own checkout (its cwd) and never a task worktree; `task_result` replaces reading one.
7. **§14.4 makes the digest pull-only, but an interactive session whose turn has ended pulls nothing.** Resolved by wake-ups (decision 39).
8. **§4.2 refuses kill and remove for run sessions without saying how a dead orchestrator comes back.** Resolved by decision 11 (`anthrex restart`, and `RestartOrchestrator` on resume) and decision 13 (the run never waits for it).
9. **§14.8 meters only Claude Code's OTLP export.** A Codex orchestrator is unmetered (decision 14, follow-up).
10. **§13 item 3's readers list scouts, reviewers and deciders, not sub-planners.** Resolved by decision 31: planners take reader slots, in a stated order.
11. **Smoke stage numbers collided.** M8b's task M8b.15 and M8c's task M8c.10 both printed `== stage 11d: …`. Resolved in the briefs (2026-09-22): M8a `11c`, M8b `11d`, M8c `11e`, M9 `11f`, M9.5 `11g`.
12. **M8c's `ScoutInfo` and `ScoutState` differed from M8b's.** Resolved in the briefs (2026-09-22): M8c now uses M8b's types exactly (`ScoutState { Starting, Working, Reported, Failed }`, M8b's `ScoutInfo`). The snapshot carries them (decision 20); the digest keeps this milestone's `RunScoutState` labels.
13. **M8c's `TaskInfo.diffstat` versus M8b's `TaskInfo.diff`.** Resolved in the briefs (2026-09-22): M8c now uses M8b's `TaskInfo.diff: Option<DiffStats>`. `task_result` uses the git diffstat text (decision 18) and does not depend on it.
14. **M8a's `owns` rule assumed every task writes.** `owns_required` (M8a decision 11) would refuse research and review tasks, which change nothing. Resolved by decision 24; the change to M8a's code is a step of task M9.4.
15. **M8a's runnable test requires `merged` dependencies.** A task depending on a research task could never run. Resolved by decision 35 (`reported` satisfies a dependency); the change is a step of task M9.9, and the hold condition one of M9.7.
16. **M8a decision 29 removed paste delivery**, which the orchestrator window, the one PTY run window, still needs. Resolved by decision 39, reusing the superseded M8 brief's framing; the scope of the change is a step of task M9.13.
17. **`manager/create.rs`'s phases are private** (`admit`, `spawn_window`, `insert`), so a role window cannot reuse them without a visibility change. Resolved by decision 5 (`pub(super)`).
18. **`McpTarget` has no epic** (M8a Interfaces, M8b decision 15). Resolved by adding `epic` (Interfaces "daemon"), a step of task M9.8.
19. **M8b's `--role scout` requires `--scout`**, which a task-bound research scout does not have. Resolved by `--task` (decision 35), a step of task M9.11.
20. **M8b's `PLAN_SCALE_MAX` is a constant** that spec §22.3 makes a setting (`planner_task_cap`). Resolved by decision 3, a step of task M9.3.
21. **The spec forbids time-based sizing but gives sub-planners no budget.** Resolved by `[orchestrator.planners]` tool-call and wall-clock limits (decision 32), which bound the planner's session, not any task's size.
22. **§7.3's chain collapse is a planning rule with no check.** Settled 2026-09-22 (spec §22 item 6): it stays the planners' judgement, carried by contract rule 13 and the planner's rule 4; the engine never checks it (decision 23.4 removed).

## Produces for M9.5

| Item | For | Where |
|------|-----|-------|
| `AgentRole::Planner` and the planner round records (`EpicRecord.sessions`) | Planner usage in `run stats`, refitting planner routes | Decisions 31–33 |
| `TaskState::Reported` | History records of research and review tasks | Decision 35 |
| `Task.hold`, `HoldRecord` | Racing and pair tasks added by the orchestrator mid-run go through the same holds | Decision 28 |
| `EpicRecord` (area, merges, integration rounds) | Per-epic history, adaptive concurrency by area | Decision 37 |
| The digest and `digest_revision` | Adding `estimate_left_secs` and `bound_ratio_permille` to what the orchestrator sees | Decision 16 |
| `planner_task_cap`, `[orchestrator.agent]`, `[orchestrator.planners]` | Threshold refitting | Interfaces "config" |
| `ORCHESTRATOR_CONTRACT` rules 21–22 (routing) | The place M9.5's routing proposals change | Interfaces "Contracts" |
| `Run.edit_log` with sources | Measuring how often the orchestrator, planners and the user change a plan | Decision 40 |
| Reader-slot order (deciders, reviewers, planners, scouts, research and review) | Adaptive concurrency's reader side | Decision 31 |
| `OrchestratorRecord` (route, wakes) and orchestrator OTLP usage | Orchestrator cost in `run stats` | Decisions 10, 14 |

## Implementation notes

