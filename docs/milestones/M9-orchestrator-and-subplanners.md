# Milestone 9: The orchestrator and sub-planners

> Written 2026-09-22 against `docs/superpowers/specs/2026-09-22-adaptive-orchestrator-design.md` ("the spec" below), which binds this milestone. It replaces the superseded brief `docs/milestones/M9-orchestrator-agent.md` (branch `docs/m9-brief-refresh`), from which three grounded parts are reused and adapted: the orchestrator's launch flags on both runtimes, the contract's structure, and the roster payload that `get_roster` returned (now part of `get_context`). Scope added 2026-09-26 by `docs/superpowers/specs/2026-09-26-tiered-testing-and-pr-delivery-design.md` §12 ("TT §12" below), and allocated on `main` by commit `586aba2` (decisions 42–44). **Refreshed 2026-09-27 from `main`'s brief at `586aba2`** against the code as it ships: milestones 8a and 8b (PR #19), milestone 8c (branch `m8c-live-run-view` at `cc9dcb7`, PR #20, protocol 9) and the Claude tool-search fix (branch `fix-claude-tool-search` at `c103308`, which sets `ENABLE_TOOL_SEARCH=false` on every headless Claude session and names every anthrex tool by its Claude id in the contracts). Both are merged before M9 starts. Every `file:line` below is on `cc9dcb7`, relative to `crates/` unless the path says otherwise; files the tool-search fix changes are cited on `c103308`. Every name taken from M8a, M8b or M8c is listed under "Names taken from earlier briefs", as shipped. Every correction the refresh made is listed, with its evidence, under "Implementation notes", "Brief refresh (2026-09-27)". "M8a", "M8b" and "M8c" below are `docs/milestones/M8a-orchestration-engine-core.md`, `docs/milestones/M8b-adaptation.md` and `docs/milestones/M8c-live-run-view.md`.

The user's design rules, which bind every decision here and are never reinterpreted:

- Only the orchestrator is an interactive PTY window. Workers, reviewers, scouts, sub-planners and deciders are headless (`claude -p` stream-json, `codex exec --json`). The user watches them through the conversation view and can never type to them; the daemon refuses input, kill and terminal subscribe for them (`server/headless_guard.rs`), and the TUI does not offer those controls.
- The engine is deterministic. No model merges, approves a task, or writes to the base branch, and nothing reaches the base branch before the user accepts the run. The orchestrator's tools cannot approve or merge; a message never changes a task's `owns`, route, size, test mode or acceptance.
- Headless agents load only the user's own settings, never a repository's. Workers run sandboxed.
- Protected agent-config files (`.claude/**`, `.mcp.json`, `.codex/**`, `CLAUDE.md`, `AGENTS.md`) change only when a task's `owns` names them exactly.
- There is no repo-level profile file. The repo profile lives in anthrex's data directory (M8b).
- TDD only where it is needed. Tasks below say "Tests first" where a test drives the change; a test that pins behaviour already true is marked as a pinning test.

## Header

| | |
|--|--|
| Status | `ready`. Milestones 8a, 8b and 8c are `done` (8b merged as PR #19, 8c as PR #20), and the Claude tool-search fix (`fix-claude-tool-search`) is merged. If either is not on `main` on the day M9 starts, stop and record it under "Implementation notes". |
| Depends on | Milestone 8a (engine, headless sessions, MCP crate, `fake-agent` headless modes, run harness, the outbox and the hand-back), milestone 8b (profile, scouts and `ScoutService`'s machine, deciders, triage and the fast path, OTLP metering, history), milestone 8c (the run view and the plan gate in `C-b T`, `RunInfo.planners`, the edit log `Run.plan_edits`, the edit form), the tool-search fix (`config::reserved_env::CLAUDE_TOOL_SEARCH`, `headless::session_vars`). |
| Spec sections | §3 (shape: information flows down through the orchestrator and sub-planners), §4 (roles; only the orchestrator is an interactive terminal; read-only launch; only the user's settings load), §4.2 (who the user can talk to; steering), §5.1 (the plan and large paths; `run promote`), §5.2 (kinds `research` and `review`), §5.3 steps 1, 2, 3, 6 and 7, §6 (`generated` and `protected` as planning rules), §7.1–§7.3 (the rubric from scout evidence, never minutes; interfaces first; split depth one; chain collapse), §8 (test mode rules the planner applies), §9 (routing; never different runtimes on overlapping `owns`), §10 (the orchestrator's reaction to rung 3, rung 4 and `blocked(question)`), §11.4 step 5 and 6 (split, rewrite; the orchestrator never approves), §12.1–§12.4 (plan edits from the orchestrator and sub-planners; the non-blocking plan gate; steering by typing), §13 item 3 (readers), §14 items 1, 2, 4 and 8 (scout once and share the map; prompt layout; the digest; OTLP for the orchestrator), §15, last paragraph ("M9 also records choices for agents that do not belong to a task"), §16 introduction ("M9 also lets the user start that goal run from the TUI: `C-b g`"), §16.4 "Navigation" (Enter on the orchestrator focuses its window), §17 (restart), §19 (orchestrator and sub-planner tools), §21 (the M9 row and its scenarios), §22.3 (`planner_task_cap`). TT §12 in full (§12.1 `message` with its "Batch boundary", §12.2 `refresh`, §12.3 `task_note`, §12.4 the user's commands, §12.5 the information-flow rule, §12.6 display, §12.7 tests), as resolved by decisions 42 and 42a–42i. |
| Branch | `m9-orchestrator-and-subplanners` |
| Protocol version | **10.** Derivation: `pub const PROTO_VERSION: u32 = 9;` at `crates/proto/src/lib.rs:40` (M8c, PR #20), with the test `proto_version_is_nine` at `lib.rs:109`; 9 + 1 = 10. The test becomes `proto_version_is_ten`. TT §12's additions, the role-routing history types and the request ids (decision 2) ride on the same bump. If `main` is not at 9 on the day M9 starts, stop and record it under "Implementation notes" before touching the protocol. |

## Starting point

Real on `main` once PR #20 and the tool-search fix are merged. Line counts are `wc -l` on `cc9dcb7` (on `c103308` for the files the fix changes). M9.1 re-counts every file at the start and records the counts under "Implementation notes"; the file-size budgets are growth over those counts.

| Path | What is there, and what this milestone does with it |
|------|------------------------------------------------------|
| `crates/daemon/src/launch/mod.rs` (432) | `LaunchPlan { program, args, cwd, env }`; `LaunchContext { window_id, name, socket_path, shell, exe, claude_bin, codex_bin, codex_hook_source, codex_bypass_hook_trust, resume }`; `plan(spec, ctx)` (`:63`), whose `env` is M3's four variables (`TERM`, `COLORTERM`, `ANTHREX_WINDOW_ID`, `ANTHREX_SOCKET`), and which for Claude builds `--name <name> --settings <json>`, then `--model <m>` when the spec names one, then `--resume <id>` or `-- <prompt>`; `hook_command`, `shell_quote`. M9 adds `LaunchContext.role`, `LaunchPlan.scrub_agent_env`, `LaunchPlan.remove_env` and the role flags (decisions 7–10). |
| `crates/daemon/src/launch/claude.rs` (71) | `HOOK_EVENTS` (10 events) and `settings(exe, window_id)`, tested byte-for-byte by `claude_settings_json_is_exact`. M9 adds `StopFailure` (decision 12). |
| `crates/daemon/src/launch/codex.rs` (353) | `args(spec, ctx)`: `-C <cwd>`, five `-c` UI/notify overrides, the hook block, then `-m <model>`, then `resume <id>` or `-- <prompt>`. `toml_string`. M9 inserts the role block before `-m` (decision 8). |
| `crates/daemon/src/hooks.rs` (399) | `HookKind` has 11 variants; `parse` maps `hook_event_name`; `to_status_event` maps `Stop` to `StatusEvent::Stop`. M9 adds `StopFailure`. |
| `crates/daemon/src/status.rs` (473) | The PTY status machine; unchanged except that a `StopFailure` hook now reaches it as `Stop`. |
| `crates/daemon/src/manager/create.rs` (563) | `create(spec, project, worktree, cols, rows)`: `launch_gate.wait()` (`:141`), the private method `admit` (`:188`), the private free function `spawn_window` (`:377`) on `spawn_blocking`, the private method `insert` (`:245`). M9 makes the three `pub(super)` and threads an optional role through them (decision 5). |
| `crates/daemon/src/manager/restart.rs` (556) | `restart(self: &Arc<Self>, id)`; `struct ForRelaunch { spec, name, session_id, cols, rows }` (`:240`); `spawn_for_restart` (`:519`) calls `launch::plan` with `resume: info.session_id`. M9 adds `ForRelaunch.role` (decision 11). |
| `crates/daemon/src/manager/entry.rs` (387) | `Entry.run: Option<serde_json::Value>` (`:111`), carried verbatim since M6. A headless window is `Process::Headless(Box<HeadlessWindow>)`, not a field; `Entry::info` (`:124`) sets `WindowInfo.run` **only** from a headless spec (`:152`). M9 adds `Entry.role`, `Entry.run_live` and `Entry.last_client_input` (decisions 11, 39). |
| `crates/daemon/src/manager/restore.rs` (554) | `headless_spec(id, kind, run)` (`:333-352`) parses a `Headless` record's `run`; a value that does not parse loads **as an exited PTY record** with a warning (M8c Risk 9; decision 11a changes this). M9 also parses `{"role_launch": …}` for a Pty record. |
| `crates/daemon/src/manager/headless.rs` (526) | `control_refusal(id, run)` (`:215`), `headless_run(id)` (`:243`, `None` for any PTY), `create_headless` (`:260`). |
| `crates/daemon/src/state/mod.rs` (207) | `WindowRecord.run: Option<serde_json::Value>`. M9 stores the orchestrator's `RoleLaunch` there for a `Pty` record (decision 11). |
| `crates/daemon/src/manager/mod.rs` (515) | `validate_name`, `write_input`, `restart`. |
| `crates/daemon/src/window.rs` (371) | `Window::spawn(id, &plan, cols, rows, events)` builds a `portable_pty::CommandBuilder` from the daemon's own environment, then sets `plan.env`. M9 removes the agent session and credential variables when `plan.scrub_agent_env` (decision 10). |
| `crates/daemon/src/server.rs` (570) | The headless guard runs at `:272`; `ClientMsg::Input` goes to `manager.write_input` at `:349`. M9 records client input time there (decision 39). |
| `crates/daemon/src/server/headless_guard.rs` (35) | `refuse(manager, msg)` (`:17`) takes only `&WindowManager` and asks `headless_run(id)`; it refuses `Subscribe`, `Input`, `Kill`, `Remove`, `Restart` for headless windows. M9 adds the orchestrator's `Kill`/`Remove` refusal through the run-live flag (decision 11). |
| `crates/daemon/src/headless/` | `mod.rs` (291 on `c103308`): `HeadlessSpec` (`:28`), `McpTarget { role, run_id, task_id, #[serde(default)] scout_id }` (`:78`), `credential_scrub_for(runtime, auth)` (`:236`), and the fix's `session_vars(runtime, env)` (`:252`), which appends `ENABLE_TOOL_SEARCH=false` last for a Claude session; `argv.rs` (450): `CLI_CAPS` (`:100`; `claude_user_settings_only = Some(["--setting-sources","user","--strict-mcp-config"])` at `:108`, `codex_user_config_only = None`, `codex_loads_project_config = true`), `mcp_args` (`:238`, ends with `--window`, `--socket`; its `match target.role` has no `Planner` arm); `session.rs`: `SCRUB_PREFIXES`, `SCRUB_NAMES` (re-exports of `config::reserved_env::{SCRUBBED_PREFIXES, SCRUBBED_NAMES, SESSION_IDENTITY}`), `HeadlessHandle::spawn`, which applies `session_vars` to every headless session; `conversation.rs` (452, over M8c's 430 budget; untouched by M9, see the file-size table). |
| `crates/config/src/reserved_env.rs` (328 on `c103308`) | `CLAUDE_TOOL_SEARCH: (&str, &str) = ("ENABLE_TOOL_SEARCH", "false")`, reserved so a profile's `env` cannot set it. M9 applies it to the orchestrator window too (decision 10). |
| `crates/daemon/src/scout/` | M8b's scouts: `ScoutSpec { id, kind, run_id, question, first_turn, cwd, project, web, codex_config, base_sha, repo_paths }` (`spec.rs:23`), `valid_id` `^[a-z0-9][a-z0-9-]{0,47}$` (`spec.rs:166`), `SUBMIT_TOOL` (`spec.rs:19`), `headless_spec` (read-only launch, `spec.rs:85`); `machine.rs` (239): the pure `ScoutMachine`/`step` (nudge once, wrap-up at `max_tool_calls`, killed at 1.5×, timeout, `ReportAccepted`, `Stop`); `service.rs` (521): `ScoutService::{start (:103), tool (:371), stop, info, run_scouts (:502)}`, windows named `scout/<id>`. M9 runs run scouts and sub-planners on it (decisions 20, 31, 32). |
| `crates/daemon/src/run/` | M8a's engine with M8b's and M8c's additions. Near 600: `driver.rs` 591, `driver/requests.rs` 588, `driver/ops.rs` 584, `contract.rs` 584, `edits.rs` 577, `engine/dispatch.rs` 589, `engine/mod.rs` 569, `model.rs` 572, `role_launch.rs` 579 (on `c103308`), `engine/ladder.rs` 553, `engine/signals.rs` 547, `engine/review.rs` 544, `engine/requests.rs` 536, `validate.rs` 514, `engine/merge.rs` 518, `engine/done.rs` 507. Headroom: `engine/outbox.rs` 360, `engine/holds.rs` 255 (M8a ruling N5's dependency holds), `engine/ops.rs` 436, `snapshot.rs` 315, `report.rs` 255, `report_task.rs` 237, `reach.rs` 97, `driver/adapt.rs` 391, `driver/adapt_goal.rs` 283, `triage.rs` 367, `globs.rs` 341, `edit_log.rs` 83 (M8c), `model_rounds.rs` 239 (M8c), `history_io.rs` 395, `stats.rs` 234, `engine/history.rs` 135. |
| `crates/daemon/src/run/role_launch.rs` (579 on `c103308`) | `WORKER_MCP_TOOLS: [&str; 2] = ["mcp__anthrex__task_done", "mcp__anthrex__task_blocked"]` (`:19`), the worker's `--allowedTools` MCP part (`:197`); `REVIEWER_PERMISSION_MODE = "dontAsk"` (`:35`); the reviewer's empty-root `ClaudeSandbox` (`:288`). M9 adds `mcp__anthrex__task_note` (decision 42f). |
| `crates/daemon/src/run/edit_log.rs` (83) | M8c's `PlanEditRecord` (`:14`), `PLAN_EDITS_KEPT = 50` (`:20`), `DESCRIBE_MAX_CHARS = 300` (`:23`), `describe` (`:29`), the exhaustive `describe_one` (`:51`), `record(run, edits, now)` (`:67`), called once, for accepted batches, at `engine/requests.rs:352`. M9 extends it (decision 40). |
| `crates/proto/src/history.rs` (210) | `HISTORY_VERSION = 1` (`:17`), `HistoryLine { Task, Run, Revert }` (`:22`, tag `type`), `RoutingCandidate { route, skipped_reason }` (`:62`), `RoutingDecision` (`:86`). M9 adds `RoleRoutingDecision` and `HistoryLine::RoleRoute`, version 2 (decision 43). |
| `crates/daemon/src/decider/prompt.rs` (430) | `const TRIAGE_HEAD` holds the literal `2 to 12 tasks` (`:27`); its exact-text test is in `decider/tests_prompt.rs`. `run::triage::PLAN_SCALE_MAX` (`triage.rs:23`) is defined and unused. |
| `crates/daemon/src/metering/` | `otlp.rs::orchestrator_env(addr, run_id)` (`:188`, re-exported at `mod.rs:17`); `server.rs` (311): `OTLP_MAX_CONNECTIONS = 8` (`:45`), `trait UsageSink`; `<data_dir>/otlp.addr` holds `http://127.0.0.1:<port>`. |
| `crates/config/src/orchestrator.rs` (585) and `orchestrator/{adapt,profile,roster,unknown}.rs` | `config::Orchestrator`, `read(table, problems)`, `orchestrator::unknown` (the known-key list); `worker_permission_mode` defaults to `acceptEdits` (`:145`). `crates/config/src/lib.rs` is 608. The config crate has no serde. |
| `crates/mcp/src/` | `tools.rs` (258): `tools_for` (`Worker => [task_done, task_blocked]`, `Orchestrator => Vec::new()`), test `worker_tools_are_task_done_and_task_blocked` (`:136`); `tools_scout.rs` (M8b's per-role schema file); `lib.rs` (109): `McpOptions`, `TOOL_REPLY_TIMEOUT` (100 s). |
| `crates/proto/src/` | `run.rs` (453), `run_info.rs` (320), `run_wire.rs` (211; `RunRequest::StartGoal { goal, dir, yes, trust_project, unconfined_checks }` at `:79`, `request::START_GOAL = "run start --goal"` at `:207`), `messages.rs` (`ClientMsg::Run(RunRequest)` at `:80`, `DaemonMsg::Run(RunReply)` at `:166`), `planner.rs` (33, M8c), `lib.rs` (117), `types.rs` (`WindowInfo.project` at `:130`). |
| `crates/cli/src/` | `main.rs` (597, no budget); `mcp_cmd.rs` (65): `anthrex mcp`'s flags, `RoleArg { Worker, Reviewer, Orchestrator, Scout }`, `--run` required for worker, reviewer and orchestrator only; `run_cmd.rs` (501), `run_cmd/status.rs` (267). |
| `crates/tui/src/` (M8c) | `keymap.rs` (275; the prefix map at `:116-135` binds `j n k p 1-9 c x X s t T < > d Q , R r ? m`, not `g`), `app/mod.rs` (499; `Modal::EditTask` at `:77`, `App.run_view` at `:168`), `app/runs.rs` (524; `state_text` `:73`, `on_run_reply` `:161`, `nav_rows` `:251`, `open_run_view` `:264`, `on_run_view_key` `:338`, `on_edit_task_key` `:416`), `app/run_enter.rs` (172), `app/headless.rs` (38; `headless_control_refusal` `:24`), `tree/runs.rs` (198), `tree/run_rows.rs` (516; `round_label` `:32`), `inspector/run.rs` (349), `inspector/run_task.rs` (301), `inspector/run_round.rs` (228), `inspector/run_format.rs` (282; `local_hhmm` `:44`), `theme.rs` (132; `run_color` `:40`, `task_glyph` `:54`, `task_color` `:80`), `graph/run_text.rs` (98), `graph/paint/style.rs` (260), `run_edit.rs` (472), `ui/run_edit.rs` (130), `ui/modal.rs` (247), `ui/conversation.rs` (389), `conversation.rs` (595), `tree.rs` (554). |
| `crates/fake-agent/src/` | `script.rs` (432), `main.rs` (382), `mcp.rs` (198), `roles.rs` (343), `headless.rs` (576), `stream_claude.rs` (581). |
| `crates/cli/tests/support/` | `run_harness.rs` (542), `run_adapt.rs` (348, M8b's helpers). |
| `scripts/pty-smoke.py` (1799) | Imports `run_view_stage` at `:46` and calls `run_engine_stage` (`:1716`), `adapt_stage` (`:1717`) and `run_view_stage(PtyProc, BIN, run_cmd, fail)` (`:1718`), then `== stage 12` at `:1720`. `scripts/pty_smoke_run_view.py` (209): `run_view_stage(pty_proc, bin_path, run_cmd, fail)` (`:100`). `scripts/pty_smoke_adapt.py` (155) removes `DECIDER_DIR` in its `finally` (`:155`) and sets `GOAL_CMD_TIMEOUT = 900.0` (`:40`). M9's stage `11f` lives in a new module. |

## Goal

`anthrex run start --goal "<text>"` no longer refuses a goal that needs a plan. A goal the triage decider labels `plan` or `large` — or any goal when no decider answers — starts a run in a new `planning` state and launches its **orchestrator**: a real, interactive `claude` or `codex` TUI in a PTY window, on the frontier tier at high effort, launched read-only, loading only the user's own settings, metered through OTLP, and holding six MCP tools. It is the only run agent the user can type to. It reads the run's context, spawns read-only **scouts** per area, and writes the plan with validated plan edits: sizes from scout evidence, interfaces first, hub tasks alone, chains collapsed where it judges a link need not be reviewed or merged on its own (guidance, not an engine check), nothing L, test modes by the rules, routes that never put two runtimes on overlapping paths. On a large goal it writes the interface tasks itself and starts one headless **sub-planner** per epic, each confined to its area, which submits its epic once and exits. Submitting opens the plan gate without blocking any tool call; the user approves, edits or rejects in the run view, and the orchestrator reads the verdict from `run_status`, a digest it long-polls for up to 50 seconds. While the run executes, the engine wakes an idle orchestrator with a short `[anthrex]` line when something needs it; it answers blocked questions, splits or rewrites mis-sized tasks, tells the user what only the user can fix, and turns whatever the user types into plan edits. It can also **message** running workers (`info`, `change`, or `stop_and_wait`, which pauses a task at its next turn boundary) and **refresh** a worker's branch with the run's latest merged work, a merge, never a rebase; workers report discoveries and risks with **`task_note`** without blocking; the user has the same `anthrex run message` and `anthrex run refresh` (TT §12). It never approves a task and never merges. Research goals run as report-writing scout tasks and review goals as reviewer tasks against a named range, neither merging anything; a large run gets one integration review per epic. `anthrex run promote` now gives a fast-path run an orchestrator. When the run completes the orchestrator writes a summary; the user accepts or discards. The user can also start a goal from the TUI with `C-b g` on a selected Git project (decision 44). Every orchestrator, sub-planner, run scout and decider session, pre-run triage included, leaves a factual role-routing record in the repository's local history (decision 43). Every behaviour of the engine and MCP side is exercised in CI by scripted `fake-agent`s; the model's own judgement is checked by a manual run.

## Scope

In:

- The orchestrator window: its launch on both runtimes (read-only, user settings only, its anthrex tools loaded up front, OTLP metering with a per-run token, scrubbed session and credential variables), persistence, restart with its flags, the `StopFailure` hook for its status, and the daemon's and the TUI's refusals for it.
- The orchestrator's tools (`get_context`, `spawn_scout`, `spawn_subplanner`, `edit_plan`, `run_status`, `task_result`) and the sub-planner's (`get_context`, `submit_epic`), in `crates/mcp` and the engine, with their own run-state gate ahead of M8a's `Running`-only one.
- `ORCHESTRATOR_CONTRACT` and `PLANNER_CONTRACT`, every prompt and message they need, the worker prompt's scout extract, and the worker contract's `task_note` and message lines (decision 41).
- Plan rules only planners are held to: `planner_task_cap`, scout evidence in `scout_refs`, no budgets from a planner, sub-planner scope, reserved integration-review ids. Chain collapse is contract guidance for the planners' judgement, never an engine check (spec §7.3, §22 item 6).
- Protected lookalike paths (non-ASCII names that open as a protected file on a case-folding volume), refused for every run (decision 23a).
- The plan path and the large path of spec §5.1 (replacing M8b's refusal), the `planning` run state, submitting a plan, and the non-blocking plan gate from the orchestrator's side, including holds on epics added after approval.
- Sub-planners (`AgentRole::Planner`): headless sessions per epic on M8b's scout machine, re-planning with a fresh session, and `RunInfo.planners`.
- Run scouts per area on M8b's `ScoutService`, in reader slots, feeding briefs, and shown in `RunInfo.scouts`.
- Task kinds `research` and `review`, executed.
- The per-epic integration review of the large path, as an engine-made review task.
- Wake-ups for an idle orchestrator, and the engine support for its reactions (rewrite restarts a mis-sized task; replacing a cancelled dependency).
- The binding 2026-09-26 spec §12: orchestrator and user messages to workers, `refresh` of a worker's task branch, and worker `task_note` reports, with their state, history, CLI and view behavior.
- Dispatch-time routing history for the orchestrator, sub-planners, run scouts and deciders, including pre-run triage, so the future router has examples for every agent role.
- `run promote` performing the promotion M8b recorded.
- `finish` with the orchestrator's summary.
- The M8c run-view changes and a TUI form that starts a goal run for the selected Git project (task M9.15).
- The protocol bump's request ids, which close M8c's "replies are matched by request name only" (decision 2).
- A snapshot that carries plan text only where a client edits it (decision 16a).
- `fake-agent`: PTY-mode MCP calls, PTY `read_message`, `mcp_until`, `capture_json`, `expect`, planner and run-scout scripts, and an MCP call log.
- M8b.15's follow-ups for M9 (the orchestrator's OTLP wiring, a per-run OTLP token, a connection cap sized from live runs, the saturating usage sum in `engine/signals.rs`) and the M8c follow-ups M9 owns ("Follow-ups handled").

Out:

| Out | Owner |
|-----|-------|
| Adaptive concurrency (§13 item 9), threshold and budget refitting, routing proposals in `run stats` | M9.5 |
| Racing (§4.1 "Race") and the test-writer-then-implementer pattern (`pair = true`), with their `AgentRole` variants and node labels | M9.5 |
| `RunInfo.estimate_left_secs` and `bound_ratio_permille` (history-derived, M8c placeholders) | M9.5 |
| Delivering a `message` to `stage:<n>` (TT §12.1): stages arrive with milestone 9.1. M9 carries the wire form and refuses it at acceptance (decision 42b) | M9.1 |
| The `<task start>...HEAD` diff wording and the `sync` fix task (PR #18's C4, C5) | M9.1, M9.2 |
| A TUI form for editing the repository profile or creating a new Git repository | Follow-up (goal runs in a selected project are in M9, decision 44; the profile form stays open in the followups file) |
| OTLP metering of a Codex orchestrator | Follow-up (decision 14) |
| A global OTLP series cap per run | Follow-up: Claude exports deltas; M9.1 item 7 re-confirms it for interactive sessions |
| The run inspector's `doing` tool target (M8c Risks 6) | Follow-up: M9 changes no round telemetry, and decision 16a argues against adding per-round text to every push |
| Stub-then-fill, a merge queue wider than 1, a learned router | Deferred by the spec (§13, §15) |
| Orchestrators that spawn orchestrators; sub-planners that spawn anything | Never (spec §18, amendment §5.10 row) |

## Design decisions

Numbered and final. If one proves wrong or impossible, stop work on it, record the evidence under "Implementation notes", and continue with the tasks that do not depend on it. Decisions added by the 2026-09-27 refresh are lettered (11a, 14a, 14b, 16a, 20a, 23a, 42a–42i), so no number moves.

### Structure

1. **Where the code lives.** In `crates/daemon/src/`:
   - `run/orch/`: `mod.rs` (model types: `RunOrch`, `TaskOrch`, `OrchLimits`, `OrchestratorRecord`, `EpicRecord`, `GateHoldRecord`, `RunScout`, `TaskMessage`, `WorkerNote`, `RefreshState`), `contract.rs` (contracts, prompts, message texts, the scout extract), `tools.rs` (argument parsing into `OrchCall`), `rules.rs` (decision 23's plan rules), `digest.rs` (`run_status`), `context.rs` (`get_context`), `result.rs` (`task_result`), `launch.rs` (the orchestrator's `RoleLaunch` and the headless specs of sub-planners, research sessions and review-task reviewers), `roles.rs` (decision 43's role-routing records).
   - `run/edits_orch.rs` (decision 42's `message` and `refresh` edits and decision 25's `amend_task deps`; `run/edits.rs` delegates to it).
   - `run/engine/`: `orch.rs` (planning, submit, the orchestrator window's lifecycle, wake notes, the `OrchEvent` dispatch), `gate_holds.rs` (decision 28's approval holds; **not** `holds.rs`, which is M8a ruling N5's dependency hold), `promote.rs` (decision 29), `planners.rs` (sub-planners and run scouts), `kinds.rs` (research and review tasks, and decision 37's integration review tasks), `worker_messages.rs` (decision 42's delivery rules, `paused(message)`, `task_note`, and the pure half of `refresh`).
   - `run/driver/`: `orch.rs` (tool routing, the `run_status` long-poll, `get_context` and `task_result`), `orch_ops.rs` (the new ops, the run-live flag), `refresh.rs` (decision 42e's clean-tree check and the refresh's git reads), `wake.rs` (pasting a wake-up into the orchestrator's PTY).
   - `run/git/summary.rs` (a task's commits and diffstat for `task_result`; a review target's resolution).
   - `scout/planner.rs` (sub-planner sessions on M8b's scout machine).
   - `launch/role.rs` (the role flags for a PTY run window).
   - `manager/role_window.rs` (`create_run_window`, the run-live flag, client-input time).

   **Pure** (M8a decision 2's grep): `run/orch/**`, `run/edits_orch.rs`, `run/engine/{orch,gate_holds,promote,planners,kinds,worker_messages}.rs`, `launch/role.rs`. **I/O**: `run/driver/{orch,orch_ops,refresh,wake}.rs`, `run/git/summary.rs`, `scout/planner.rs`, `manager/role_window.rs`. *(M8a decision 2; AGENTS.md rules 2, 5 and 10. The daemon model's approval hold is `GateHold`/`Task.orch.gate_hold`, so it never reads as M8a's `awaiting_deps` hold; the wire and every user-facing text keep the spec's word "hold". `engine/integration.rs` does not exist (decision 37); the new files exist because the files they would otherwise grow are at 577–591 lines.)*
2. **Protocol.** `PROTO_VERSION` becomes **10** (header). Every addition is listed in Interfaces "proto": the orchestrator and hold types, TT §12's `PlanEdit::{Message, Refresh}`, `MessageTarget`, `MessageKind`, `BlockReason::MessagePause`, `TaskNoteKind`, `TaskNoteInfo`, the history's `RoleRoutingDecision`, and **request ids**: `ClientMsg::RunTagged { id: u64, request: RunRequest }` (appended last), whose replies echo `#[serde(default)] request_id: Option<u64>` on every `RunReply` variant that answers a request (all but `Snapshot`; M9.2 review fixes). A client that sends a tagged request matches its reply by id, never by request name; an untagged `ClientMsg::Run` is answered exactly as today (`request_id: None`), so the CLI is unchanged. This closes M8c's "replies are matched by request name only" (followups file, M8c.9 review M7): the TUI's edit form and goal form send tagged requests. New enum variants are appended **last** in their enum, and every new struct field is `#[serde(default)]`, so an M8a–M8c `run.json` and snapshot still deserialize. Every new message and every new variant gets a MessagePack round-trip test (AGENTS.md rule 4). Every client is updated in the same change: the TUI (task M9.15), the CLI (task M9.14) and `anthrex mcp` (task M9.11). *(AGENTS.md rule 4; spec §19 last line.)*
3. **Configuration.** New keys live in a new file, `crates/config/src/orchestrator/agent.rs`, a submodule of M8a's `orchestrator` beside M8b's `adapt.rs`. `config::Orchestrator` gains **one** field, `pub agent: orchestrator::agent::AgentSettings`, holding `planner_task_cap`, `max_scouts`, `wake_orchestrator`, `wake_quiet_secs`, `message_max_per_turn`, `note_max_per_task` (all top-level keys of `[orchestrator]`) and the `[orchestrator.agent]` and `[orchestrator.planners]` tables, read by one call from `orchestrator::read`, with M8a's problem format; `orchestrator/unknown.rs` gains the new keys. `crates/config/src/lib.rs` changes only in its existing `pub use orchestrator::{…}` line. Keys, defaults and ranges are in Interfaces "config". `planner_task_cap` replaces M8b's unused `run::triage::PLAN_SCALE_MAX`, and the triage prompt's "2 to 12 tasks" is rendered from it in `decider/prompt.rs`, where the literal lives. *(Spec §22.3; TT §12.1, §12.3 name `message_max_per_turn` and `note_max_per_task`. M8a split `orchestrator` into a directory and `orchestrator.rs` is 585 lines, so one field, not eight.)*
4. **Words.** An **epic** is spec §2's planning-only grouping; its id is a `PlanTask.epic` value and a `PlannerInfo.epic`. A **sub-planner** is a headless session with `AgentRole::Planner`, one per epic at a time. The **orchestrator** is the one PTY window with `AgentRole::Orchestrator`. A **hold** is decision 28's approval wait on tasks added after the plan gate (`GateHold` in the daemon's code); it is not M8a's dependency hold (`awaiting_deps`). A **message** is TT §12.1's plan edit; a **task note** is TT §12.3's `task_note` record (`Task.orch.worker_notes` in the model). A **role-routing record** is decision 43's `RoleRoutingDecision`. *(Spec §2; TT §12.)*

### The orchestrator window

5. **The one interactive run window.** The engine creates it with `WindowManager::create_run_window(spec, project, role)` (new, `manager/role_window.rs`; the name is `spec.name`), which runs `create`'s steps (`launch_gate.wait()`, then `admit`, `spawn_window` and `insert`, made `pub(super)` and given an `Option<RoleLaunch>`) with no worktree:
   - `WindowInfo.kind == Pty`, `WindowInfo.run == Some(RunRef { run_id, task_id: None, role: Orchestrator, session: 1 })`;
   - name `<h4>/orchestrator` (`<h4>` is `Run::short()`, as M8a names run windows);
   - `cwd` is the run's `root`, the user's own checkout, which the orchestrator may read and cannot write (decisions 7, 8). It is never a task worktree.
   - Its PTY is 200 × 50 until the first client `Resize` (no client asked for it).
   - No client message can create one: `ClientMsg::CreateWindow`'s `WindowSpec` has no field that sets a role.
   - It counts toward the run's `max_windows`: the engine increments `Run.windows_created` when `CreateOrchestrator` succeeds, as dispatch does for every session window (`engine/dispatch.rs:329, 346`).

   *(Spec §3, §4 "Only the orchestrator is an interactive terminal", §11.6 "scout, planner: none; they read the main checkout".)*
6. **Choosing its runtime and model.** `RunRequest::StartGoal` gains `orchestrator: Option<OrchestratorChoice { runtime, model: Option<String> }>`, from `--orchestrator <runtime>[:<model>]` or the TUI goal form (decision 44). Resolution, `run::orch::launch::resolve_orchestrator(choice, agent, default_runtime, roster) -> Result<Resolved, String>` (Interfaces), which also yields decision 43's candidate snapshot:
   1. Runtime: the choice's, else `[orchestrator.agent] runtime`, else `orchestrator.default_runtime`.
   2. Model: the choice's when given (`codex:` with an empty model means Codex's configured default, `""`); it must be in the roster (`<runtime>:<model> is not in the roster`). Otherwise `[orchestrator.agent] model` when non-empty (same check). Otherwise the first roster entry of that runtime at `frontier`, else that runtime's highest-strength entry (with the built-in roster, Codex resolves to `""` at `standard`, and the report says `orchestrator below the frontier tier: codex (default) standard`).
   3. Effort: `[orchestrator.agent] effort`, default `high`.

   `run start --plan` never creates an orchestrator. *(Spec §4 table "frontier tier, high effort"; superseded M9 brief decision 1, adapted to strengths.)*
7. **Claude launch.** `launch::plan` for `Runtime::Claude` with `ctx.role == Some(role)` builds, in this order:
   - M3's `--name <name>` and `--settings <json>` (M3's hook settings, with decision 12's `StopFailure`);
   - the flags in `CLI_CAPS.claude_user_settings_only` (`--setting-sources user --strict-mcp-config` as shipped, `headless/argv.rs:108`);
   - `--mcp-config {"mcpServers":{"anthrex":{"type":"stdio","command":"<exe>","args":[…headless::argv::mcp_args(&role.mcp, ctx.window_id, ctx.socket_path)…]}}}`;
   - `--allowedTools mcp__anthrex__get_context,mcp__anthrex__spawn_scout,mcp__anthrex__spawn_subplanner,mcp__anthrex__edit_plan,mcp__anthrex__run_status,mcp__anthrex__task_result,Read,Glob,Grep`;
   - `--disallowedTools Edit,Write,NotebookEdit,Bash,Agent`;
   - `--append-system-prompt <ORCHESTRATOR_CONTRACT>`;
   - `--effort <effort>` when `CLI_CAPS.claude_effort_flag` and M9.1 item 1 finds the flag accepted by the interactive CLI;
   - then M3's `--model <model>` when the route names one, then `--resume <id>` or `--` and the first prompt.

   `--mcp-config`, `--allowedTools` and `--disallowedTools` are variadic; the order guarantees each is followed by another flag, and `--` guarantees the prompt is never read as a tool name.
   - **Read-only by the runtime.** The orchestrator is launched with `Edit`, `Write`, `NotebookEdit` and `Bash` disallowed, not in plan mode: in the interactive TUI, plan mode injects its own instruction to present a plan and call `ExitPlanMode`, which prompts the user and fights the contract. `Agent` is disallowed because a sub-agent the orchestrator starts is a second path to tools; its reading is the scouts' job. Everything not allowed and not disallowed keeps Claude Code's default permission behaviour, which asks the user, who is present. If M9.1 item 2 finds that `--disallowedTools` does not stop a tool call, stop and use `--permission-mode plan` instead, recording the evidence.
   - **Strict MCP.** `--strict-mcp-config` comes from `claude_user_settings_only`, so the orchestrator never loads the repository's `.mcp.json`.
   - **Its tools up front.** The environment carries `ENABLE_TOOL_SEARCH=false` (decision 10), so the six anthrex tools are loaded directly, not deferred behind `ToolSearch`.

   *(Spec §4 "Read-only roles are launched read-only by the runtime"; user decision: "Claude `--permission-mode plan` or disallowing Edit/Write/Bash"; superseded M9 brief decision 5.)*
8. **Codex launch.** `launch::codex::args` with `ctx.role == Some(role)` inserts, after M3's hook block and before `-m`:
   - `-c mcp_servers.anthrex.command=<toml exe>`, `-c mcp_servers.anthrex.args=<toml array of mcp_args>`, `-c mcp_servers.anthrex.tool_timeout_sec=120`, `-c mcp_servers.anthrex.default_tools_approval_mode="auto"`;
   - `-c developer_instructions=<toml ORCHESTRATOR_CONTRACT>`, `-c model_reasoning_effort=<toml effort>`;
   - the flags in `CLI_CAPS.codex_user_config_only`, when `Some` (it is `None` as shipped);
   - `-s read-only`, `-a on-request`.

   Then M3's `-m`, `resume <id>` or `-- <prompt>`. Every TOML string comes from `launch::codex::toml_string`. M9.1 item 4 verifies that `-s` and `-a` are accepted ahead of `resume`, as `-m` and `-c` are; if not, they move after `resume <id>`. *(Spec §4; superseded M9 brief decision 6.)*
9. **Only the user's settings load.** Both launches carry M8a decision 53's exclusion flags when the CLI has them. The project-settings check that `RunService::build_plan` already runs (M8a decisions 50/53, per reachable runtime) covers the orchestrator's and the planners' runtimes too (decision 26). With the shipped caps (`codex_user_config_only = None`, `codex_loads_project_config = true`), a **Codex orchestrator in a repository with tracked `.codex` configuration is always refused without `--trust-project`**; with it the orchestrator starts and `Run.trusted_project` records the files. M8a's `codex_config_guard` is a `HeadlessSpec` field and has no PTY equivalent, so the PTY Codex orchestrator relies on `--trust-project` alone. `run promote` repeats the check against the run's base commit (decision 29), honouring the `trust_project` the run started with. *(Spec §4 "Only the user's own settings load"; M8a decision 53.)*
10. **Environment.** `LaunchPlan` gains `scrub_agent_env: bool` and `remove_env: Vec<String>` (false and empty for every window except run windows created by `create_run_window`). `Window::spawn` then removes, before setting `plan.env`:
    - every inherited variable matching M8a's `headless::session::SCRUB_PREFIXES` and `SCRUB_NAMES` (`CLAUDE_CODE_`, `CLAUDECODE`, …), so a daemon started from inside a Claude session does not make the orchestrator a nested session;
    - every name in M8a's credential scrub, `headless::credential_scrub_for(runtime, config.claude.auth)`, carried in `RoleLaunch.remove_env` by the driver, so with `auth = "login"` an inherited `ANTHROPIC_API_KEY` does not make the orchestrator bill the key, the same rule every headless session follows.

    Scrub first, then `plan.env`, because `SESSION_IDENTITY` names `ANTHREX_WINDOW_ID` and `ANTHREX_SOCKET`, which `plan.env` sets back. `RoleLaunch.env` adds, after M3's four variables:
    - for Claude, `metering::orchestrator_env(addr, run_id)` when `<data_dir>/otlp.addr` exists (read by the driver on `spawn_blocking` when it builds the op), with `anthrex.role=orchestrator` in its resource attributes (M8b decision 30), plus decision 14a's token header;
    - `MCP_TOOL_TIMEOUT=120000` for Claude, when M9.1 item 3 finds that Claude Code's default MCP tool timeout is below 120 s (the long-poll's 50 s plus `task_result`'s git reads must never be cut off);
    - for Claude, **`config::reserved_env::CLAUDE_TOOL_SEARCH` (`ENABLE_TOOL_SEARCH=false`) last**, exactly as `headless::session_vars` pins it for every headless Claude session: the orchestrator cannot plan without its anthrex tools, and a model's loose `ToolSearch` query can miss them (the tool-search fix; followups file, "From the Claude tool-search fix"). The process environment decides tool search, so a user's own `~/.claude/settings.json` `env` block cannot turn it back on. A Codex orchestrator does not get it. The user's own PTY windows (`anthrex new`) stay unchanged.

    *(Spec §14.8, §17 "Scrubbed agent environment"; the tool-search fix.)*
11. **Persistence, restart and what the daemon and the TUI refuse.**
    - `launch::role::RoleLaunch` (Interfaces) is the orchestrator's whole role: its `RunRef`, `McpTarget`, contract, tool lists, effort, environment and the names to remove. It is kept in `Entry.role` and persisted in `WindowRecord.run` as `{"role_launch": <RoleLaunch>}` for a `Pty` record (M8a stores a `HeadlessSpec` there for a `Headless` record; the kind decides the parse). A Pty value that fails to parse restores the window as a plain PTY window with a warning. `Entry::info` sets `WindowInfo.run` from `role.run_ref` for a Pty entry (today only headless specs set it, `entry.rs:152`).
    - `manager::restart` copies `Entry.role` into `ForRelaunch.role` and passes it to `launch::plan`, so a restart — the user's `anthrex restart <id>`, or decision 13's — re-passes every role flag and variable with `--resume <session id>` (Claude) or `resume <id>` (Codex). Resume restores none of them by itself (spec §17).
    - **The run-live flag.** `Entry.run_live: bool` (not persisted; false after a daemon restart until the driver sets it) is set by `WindowManager::set_run_window_live(id, live)`, under the lock, no I/O. The driver sets it after `CreateOrchestrator` or `RestartOrchestrator` succeeds and, after a daemon restart, during restore for every restored orchestrator window of a non-terminal run (so a dormant orchestrator of a paused run is still protected); it clears it when it publishes the run in a terminal state (accepted, discarded or failed). `server/headless_guard.rs::refuse` gains one check: `Kill` and `Remove` of a window with `role.is_some() && run_live` are refused with `DaemonMsg::Error { request: "kill" | "remove", message: "window <id> is the orchestrator of run <run>; stop the run with anthrex run cancel, or restart the orchestrator with anthrex restart <id>" }`. The guard takes only `&WindowManager`, and `headless_run` is `None` for any PTY (`manager/headless.rs:243`), so the flag is how it sees a live run.
    - **The TUI does not offer them either.** `app/headless.rs::headless_control_refusal` (`:24`) gains a second case: a focused window whose `run` is `RunRef { role: Orchestrator, .. }` of a run the client shows as not terminal. `C-b x` and `C-b X` then toast the refusal text above and open no dialog; `C-b R` stays allowed. M8c's watch-only rule (M8c R13) is that offering a control the daemon refuses is itself wrong.
    - A client may `Subscribe`, send `Input`, `Resize`, `Rename` and `Restart` for the orchestrator window. Once the run is terminal, the window is an ordinary PTY window again: nothing is refused, and M8c already lists it as a plain window.
    - **After a daemon restart** the window is restored dormant, like every PTY window (M6). `anthrex run resume <run>` issues `OpKind::RestartOrchestrator` for a dormant orchestrator of a paused run, and also for a run in `awaiting_approval`, whose state it leaves unchanged. The first wake-up after a restart carries the note `the daemon restarted and your session was resumed` (decision 39).

    *(Spec §4.2 "Kill and remove are refused for run sessions", §17.)*

11a. **A headless record never comes back as a PTY** (*added 2026-09-27 by the controller's ruling: M9 takes M8c's Risk 9*). Today `manager/restore.rs::headless_spec` (`:333-352`) restores a `Headless` record whose `run` does not parse as an **exited PTY window**, which a client could then subscribe to and type into, against the watch-only rule. From M9, such a record is restored as an **exited headless window**: `Process::Headless` with a placeholder `HeadlessSpec` that has no `run_ref` and no session to resume, so the headless guard refuses `Subscribe`, `Input` and `Restart` for it, `Kill` and `Remove` work as for any exited headless window, and the warning names the window and the parse error. Nothing is ever respawned from it.
12. **The status gap.** `launch::claude::HOOK_EVENTS` gains `StopFailure` (11 events), `hooks::HookKind` gains `StopFailure`, `parse` maps `"StopFailure"` to it, and it maps to `StatusEvent::Stop`: a turn that ends on an API error is over, and the window goes idle instead of staying `Working`. This applies to every Claude PTY window, which is correct for all of them. `PreCompact` and `PostCompact` are not added: compaction runs inside a turn that still ends with `Stop` or `StopFailure`. If M9.1 item 5 finds a manual `/compact` that leaves the window `Working`, add both and map them to no status event, recording it. *(Spec §11.1 "`StopFailure` replaces `Stop` on API errors"; M8a Out table and follow-up.)*
13. **When the orchestrator is not there.** The engine never waits for it. If its window exits (the driver sees `Exited` in the manager's window list and sends `OrchEvent::OrchestratorWindow { live: false }`), the run keeps executing, wake-ups are suspended, and the run's attention list gets `the orchestrator (window <n>) exited; restart it with anthrex restart <n>`. Everything the orchestrator does is also available to the user: `anthrex run edit` for every plan edit, `anthrex run message` and `anthrex run refresh` (TT §12.4), `anthrex run approve|reject` for the gate and holds, `retry`, `override`, `cancel`, `finish`. *(Spec §12.4 "The same edits are available without it as `anthrex run edit …`".)*
14. **Metering.** A Claude orchestrator is metered through M8b's OTLP receiver (M8b decision 30), and its usage lands in `Run.orchestrator_usage` through M8b's `EventKind::OrchestratorUsage`. A Codex orchestrator is not metered: `RunInfo.usage.by_role["orchestrator"]` stays zero and the report says `orchestrator usage: not metered (codex)`. Recorded as a follow-up. M9.1 item 7 records whether Claude's exporter sends `Expect: 100-continue`, which the receiver does not support; code is needed only if it does (followups file, "From M8b.15"). *(Spec §14.8, which names Claude Code's OTLP export only.)*

14a. **A per-run OTLP token** (*controller's ruling of 2026-09-27, from M8b.15's review follow-up "orchestrator usage is not authenticated"*). Any local process can post points to the loopback receiver today. So:
   - when the engine creates the run's `OrchestratorRecord`, the driver gives it `otlp_token`: 32 lowercase hex characters from the OS random source, persisted in `run.json` (0600), never in the snapshot or the digest;
   - `RoleLaunch.env` carries `OTEL_EXPORTER_OTLP_HEADERS=authorization=Bearer <token>` beside `orchestrator_env`'s variables;
   - `UsageSink` gains `fn token(&self, run_id: &str) -> Option<String>` (the live run's token, read from the engine snapshot the sink already watches), and the receiver drops every point whose `authorization` header is absent or does not equal its run's token, counting drops per run for the log;
   - a restart keeps the token (it is persisted, and decision 11's restart re-passes the same `RoleLaunch.env`).
14b. **The connection cap grows with live runs** (*controller's ruling of 2026-09-27, from M8b.15's review follow-up*). `OTLP_MAX_CONNECTIONS` (8, `metering/server.rs:45`) becomes `OTLP_BASE_CONNECTIONS = 8` plus the number of live runs with an orchestrator, read through `UsageSink` (a new `fn live_orchestrators(&self) -> usize`). The receiver's semaphore is resized when the sink's live generation changes. When every slot is taken, a connection that has not yet presented a valid token (decision 14a) is closed first, so a local process holding slots cannot starve a real orchestrator.

### Tools

15. **Tool lists and the rules every tool follows.** `mcp::tools_for(Orchestrator)` is `get_context`, `spawn_scout`, `spawn_subplanner`, `edit_plan`, `run_status`, `task_result`, in that order; `tools_for(Planner)` is `get_context`, `submit_epic`; `tools_for(Worker)` becomes `task_done`, `task_blocked`, `task_note` (decision 42f). Schemas and texts are in Interfaces "MCP".
    - **Nothing blocks on the user.** Every tool answers within M8a's `TOOL_REPLY_TIMEOUT` (100 s) and Codex's `tool_timeout_sec` (120 s): `run_status` waits at most 50 s, `task_result` makes at most two git reads bounded by `DONE_CHECK_GIT_TIMEOUT` (10 s) each, and everything else answers from memory. The plan gate's verdict is read, never awaited (decision 30).
    - **Authorization.** `anthrex mcp` refuses a tool outside its role's list without reaching the daemon (M8a). The daemon then checks the calling window: an orchestrator call must come from the run's orchestrator window (`this window is not the orchestrator of run <id>`), a planner call from the live session of its epic's sub-planner (`this window is not the sub-planner of epic <e> of run <id>`).
    - **Routing.** `ToolCall` gains `epic: Option<String>`. `RunService::tool` (`driver/adapt.rs:153`) gains the M9 branches ahead of M8b's scout branch, delegating to `driver/orch.rs`: `get_context`, `run_status` and `task_result` go to the driver's read path (decisions 16–18); a `submit_scout_report` from a task-bound scout window goes to the engine (decision 35); every other orchestrator or planner tool, and a worker's `task_note`, goes to the reducer as `EventKind::Orch(OrchEvent::Tool { reply, call, refusals })`, dispatched by role to `run/engine/orch.rs`, `run/engine/planners.rs` and `run/engine/worker_messages.rs`.
    - **Their own run-state gate.** M8a's `engine/done.rs` refuses every tool unless the run is `Running`, so M9's orchestrator and planner tools are dispatched **before** that gate and check their own: writes are accepted in `planning`, `awaiting_approval` and `running`, and on a `complete` run only an `edit_plan` whose edits are empty and which carries a `summary`; a `paused` run answers M8a's `run <id> is paused; the user must resume it`; any other state answers M8a's `run <id> is <state>`. Reads (`run_status`, `get_context`, `task_result`) answer in every state, terminal ones included. `task_note` follows decision 42f.
    - **Runtime refusals.** An `edit_plan` or `submit_epic` batch can widen the runtimes the run reaches, exactly as a `run edit` can; the driver computes the same `refusals` for it before sending the event (M8a ruling T22-I1b, `driver/requests.rs`), and the pure edit path applies them as it does for `EventKind::Edit`.
    - **Results** are one text content item holding JSON. A refusal is `ToolResult { ok: false }` with a JSON object `{"error": "<text>"}`, except a rejected plan edit, which is decision 19's error list.

    *(Spec §19; M8a decision 4 and Interfaces "MCP tools"; user decision on MCP timeouts.)*
16. **`run_status`, the long-poll.** Arguments `since` and `wait_secs` (0–50, default 0). It returns the digest (Interfaces "The digest").
    - **The revision it waits on** is `Run.orch.digest_rev`, not M8a's `revision`. After every reducer step that changed a run, `run::orch::digest::fingerprint(&run)` (FNV-1a 64 over the digest's JSON with every counter removed: tool calls, tokens, spend, seconds, `now`, round activity) is compared with `Run.orch.digest_fp`; only a different fingerprint bumps `digest_rev`. So a worker's tool-call counter never wakes a waiting orchestrator; a task changing state, a block, a verdict, a hold, a scout or planner finishing, a task note, a message or an edit does.
    - **Waiting.** With `wait_secs == 0`, or `since` absent, or `since != digest_rev`, it answers at once. Otherwise the driver waits on M8a's snapshot `watch` until the run's `RunInfo.digest_revision` differs from `since` or `wait_secs` elapses, and answers with the current digest either way. No engine lock is held while waiting; the run is cloned under the lock for the answer, then the lock is released before the digest is built.
    - **Read receipt.** After answering, the driver sends `OrchEvent::DigestRead { run_id, digest_revision }`; the reducer drops the pending wake notes up to that revision (decision 39), since the orchestrator has now seen them.
    - A run that becomes terminal while a wait is open ends the wait at once.

    *(Spec §19 "`run_status` (long-poll up to 50 s)", §14.4 "structured and small".)*

16a. **The snapshot carries plan text only where a client edits it** (*added 2026-09-27; the M8c whole-branch review's follow-up "every run's task briefs ride on every snapshot push", owned by M9*). `run/snapshot.rs` copies each task's `brief`, `acceptance` and `route_spec` into `TaskInfo` (`snapshot.rs:311-313`) for every run the engine holds, terminal runs included, on every push. M9 adds per-task messages and notes and per-run orchestrator, holds, integration and planners to the same push. So:
   - `TaskInfo.brief`, `acceptance` and `route_spec` are filled only while the run is `awaiting_approval` or the task's hold is `Awaiting`; otherwise they are empty (`String::new()`, `Vec::new()`, `RouteSpec::default()`). Their only reader is M8c's edit form (`tui/src/run_edit.rs`, `ui/run_edit.rs`), which opens only at the gate (M8c decision 32), so its stale-form check is unaffected.
   - `TaskInfo.task_notes` carries at most the task's last 10 notes; message texts are not in the snapshot at all (`message_count`, `last_message_kind`, `last_message_line`; decision 42d).
   - The digest, `task_result` and the report read the model, not the snapshot, so they keep full texts.
17. **`get_context`.** Returns Interfaces "The context": the run and its triage, who is asking (the orchestrator, or a sub-planner and its epic), `profile::summary` and the profile's glob lists and commands, the limits (`planner_task_cap`, `max_tasks`, slots, `max_bounces`, `max_scouts`, the S and M rubric text of spec §7.1), the roster with strengths, notes and `installed`, the run's scout reports, the epics and the plan so far.
    - **Installed** means the runtime's configured binary (M3's `claude_bin`, `codex_bin`, from `manager.config()`) resolves to an executable file directly or on `PATH`; checked once per run on `spawn_blocking` when the run is built, kept in `Run.orch.installed`.
    - **For a sub-planner**, reports are filtered to its epic's `scout_refs`, the reports whose scout area intersects its epic's area (M8a decision 11's intersection), and the onboarding report; the plan lists tasks with no epic and its own epic's tasks.
    - **Bounds.** Each report summary is cut to 8000 characters; `scouts: [ids]` limits the answer to those reports. When the JSON passes `CONTEXT_MAX_BYTES` (96 KiB), later reports' summaries are cut to 1000 characters, then their file lists dropped, and `omitted` says how many.

    *(Spec §19 "`get_context` (roster with strengths, profile, scout reports)"; superseded M9 brief decisions 24–25 for `installed`.)*
18. **`task_result`.** Everything about one task (Interfaces "The task result"): the task record, its done claim, its checks (with the M8b summary), proofs, reviews with findings, agent rounds, its research report, its messages and task notes, its history, and two git reads the driver makes on `spawn_blocking` in `root` against the task branch `anthrex/<run>/<task>`, which M8a keeps until accept or discard: `git log --format=%h%x1f%s -n 50 <start>..<branch>` and `git diff --stat=100 <start>...<branch>` (60 lines at most). A task with no start commit has neither. At most `TASK_RESULT_MAX_BYTES` (64 KiB). This is how the orchestrator knows a task's work without reading a checkout. *(Spec §3 "reads the run digest, never transcripts or checkouts", §19.)*
19. **`edit_plan`.** `{ edits, submit?, summary? }`.
    - **One batch.** `edits` are M8a's `PlanEdit`s, applied with `run::edits::apply_edits` under `EditScope::Run`, then checked with decision 23's rules with source `EditSource::Orchestrator`. Any error rejects the whole batch and changes nothing; the reply is `ToolResult { ok: false }` with `{"accepted": false, "errors": [{"task": "t2" | null, "field": "…", "rule": "…", "message": "…"}]}`, built from `PlanError`s in order.
    - **Message and refresh calls** (decision 42, TT §12.1 "Batch boundary"). A `message` or `refresh` edit is the only edit in its call: a call that mixes either with any other edit, or carries `submit` or `summary`, is refused before any effect with `message and refresh must be the only edit in their call`. A `refresh` whose checkout the driver's clean-tree check found dirty (decision 42e) is refused before the event.
    - **Accepted:** `{"accepted": true, "revision": <digest_rev>, "awaiting_approval": <bool>, "notes": [<validation notes added by this batch>], "held": <hold id> | null}`; for a call whose one edit is a `message`, also `"delivered": [<task ids>], "refused": [{"task": "<id>", "reason": "<text>"}]` (decision 42b). `awaiting_approval` is true while the run is in `awaiting_approval` or the batch's tasks landed in a hold that is waiting.
    - **`submit: true`** is decision 27.
    - **`summary`** (1–8000 characters) is stored as `Run.orch.orchestrator.summary` and written at the top of `REPORT.md` under `## Summary from the orchestrator`; the last one wins. On a `complete` run, a call with `edits: []` and a `summary` is accepted; any other edit is refused with `edits are not accepted on a complete run; only a summary is`.
    - **It cannot approve.** `PlanEdit` has no approve or override operation, so `{"op": "override", …}` fails to parse (`invalid arguments: edits[0]: unknown variant \`override\``), and nothing in the orchestrator's tools reaches M8a's `Override`, `Approve` or `Accept`. The `finish` edit ends the run as M8a decision 37 says (M8a records it in `Run.finish_edit`); it approves nothing.
    - Every batch, accepted or rejected, is recorded in the edit log with source `orchestrator` (decision 40).

    *(Spec §12.1, §11.4 step 6 "cannot approve a task", §5.3 step 7; TT §12.1.)*
20. **`spawn_scout`.** `{ id, question, area, web? }` starts a run scout on M8b's `ScoutService`.
    - **Ids.** The given id must match `^[a-z0-9][a-z0-9-]{0,31}$`; the scout's full id is `<h4>-<id>` (at most 37 characters, inside M8b's `valid_id`), which is what the reply, the digest, `get_context` and `scout_refs` use, so two runs' scouts never collide in `ScoutService` or in `<data_dir>/runs/<run>/scouts/`. Its window is M8b's `scout/<full id>`.
    - **Slots.** A run scout takes one of the run's reader slots while it runs (spec §13 item 3). The reducer queues it (`RunScout { state: Queued }`) and emits `OpKind::StartScout` when a slot is free; the reply is at once: `{"scout_id": "<full id>", "state": "queued" | "starting"}`.
    - **The session** is M8b decision 12's area scout: `ScoutSpec { id, kind: Area, run_id: Some(run), question, first_turn, cwd: root, project, web, codex_config, base_sha, repo_paths }` with `first_turn = scout_first_turn(run, id, area, question)` (Interfaces), `codex_config` the run's Codex guard entries, `base_sha` the run's, and `repo_paths` the run's `root` and git common dir. It reads the user's checkout (`root`); see decision 20a.
    - **Its end.** The driver awaits the `ScoutHandle`'s outcome and sends `OrchEvent::ScoutEnded { run_id, scout_id, outcome, usage }`. A report pushes its id into `Run.scout_reports` (M8b decision 19 then uses it for the size cross-check) and adds its usage to `Run.scout_usage`.
    - **Limits.** At most `max_scouts` per run (`run <id> already has <n> scouts, the most max_scouts allows`); a used id is refused (`scout <id> already exists in run <run>`).
    - **In the snapshot.** `RunInfo.scouts` is M8b's `ScoutService::run_scouts(run_id)` (`scout/service.rs:502`): M8b's `ScoutInfo` with M8b's `ScoutState` (`starting`, `working`, `reported`, `failed`), which is what M8c draws. The pure snapshot cannot call the service (it sets `scouts: Vec::new()`, `snapshot.rs:91`), so the driver overlays `run_scouts(run_id)` onto each `RunInfo` in its publish path, outside the manager's lock, keeping M8b's live counters. A `Queued` run scout has no `ScoutInfo` until `StartScout` runs; it shows only in the digest, whose `scouts[].state` uses this milestone's `RunScoutState` labels (`queued`, `running`, `reported`, `failed`).
    - **After a daemon restart** a run scout is not resumed: on `Restore`, every `Queued` or `Running` run scout becomes `Failed { "the daemon restarted during this scout" }`. **Nothing is killed**: the process-kill rule allows killing only an exact recorded pid, and reconcile's killer considers only `run.json`'s round pids. A leftover scout lost its stdin with the old daemon, runs under a read-only sandbox, ends with its turn, and its tool call reaches no scout (`unknown scout <id>`); M8b made the same choice. Scouts write nothing but their report, so repeating one is cheap; the digest shows the failure and the orchestrator may spawn it again (M8b decision 11's reasoning, applied to run scouts).

    *(Spec §5.3 step 1, §14 item 1, §19; M8b "Produces for later milestones" rows for M9.)*

20a. **Readers read the user's checkout** (*controller's ruling of 2026-09-27*). The orchestrator, run scouts, sub-planners and research tasks read `root`, the user's own checkout, which may be dirty or ahead of the run's `base_sha`. This is a known difference from the plan's base: a scout may describe a file the user has not committed. Their prompts name `base_sha` so a planner can tell, and workers still start from the run branch. Review-task reviewers read M8a's review checkout at the resolved head (decision 36).
21. **`spawn_subplanner`.** `{ epic, title, area, brief, scout_refs? }`.
    - **New epic.** `epic` matches `^[a-z0-9][a-z0-9-]{0,10}$` (at most 11 characters, so decision 37's `<epic>-int<n>` fits a 16-character task id). Each `area` glob passes `validate_glob` and is a literal path or `<literal>/**` (M8a decision 12's area form); the area must not intersect another epic's (`epic <e>: area: overlaps epic <f>'s area (<glob>)`). An `EpicRecord` is added and its planner queued for a reader slot (decision 31). The run's `path` becomes `Large`.
    - **Re-plan.** For an existing epic whose planner is `Finished` or `Failed`, it starts a **fresh** planner session for that epic (never the old session), with the new `brief` as the re-plan request, the epic's current tasks in its prompt, and an entry in `replans` (the brief's first 40 characters). A live planner refuses it: `epic <e> is being planned by its sub-planner; wait for it to finish`.
    - **When** allowed: in `planning`, `awaiting_approval` (which returns the run to `planning`, decision 27) and `running`. A new epic in a `running` run gets a hold (decision 28).
    - Reply at once: `{"epic": "<e>", "state": "queued" | "planning", "hold": <id> | null}`.

    *(Spec §5.1 "Large", §3 "one per epic … add tasks then exit", §7.3, §12.3 "any later edit that adds an epic".)*
22. **`submit_epic`.** `{ edits, note? }`, from the epic's live planner only.
    - **Scope.** Applied with `EditScope::Area { globs: epic.area }` (M8a decision 12 checks every `owns` glob is inside; `run/validate_graph.rs`) and decision 23's rules with source `Planner { epic }`.
    - **Operations.** `add_task`, `add_dep`, `amend_task`, `split_task` and `cancel_task`, each only on tasks of its own epic (`add_dep`'s `dep` may name any task). `answer`, `pause`, `resume`, `finish`, `message` and `refresh` are refused: `op <op> is not available to a sub-planner` (TT §12.1 "never to sub-planners"). An added task with no `epic` gets the planner's epic; another epic is refused (`task <id>: epic: a sub-planner adds tasks only to its own epic <e>`). A `cancel_task`, `amend_task` or `split_task` of another epic's task is refused (`task <id>: a sub-planner changes only its own epic's tasks`), which closes the M8a.6 follow-up that `EditScope::Area` limits only `owns`.
    - **Accepted** ends the planner: the reply is `Epic recorded. You are done; end your turn now.`, the planner becomes `Finished`, and the engine emits `Effect::PlannerAccepted { window_id }`, on which the driver calls `ScoutService::accept_planner(window_id)`, the scout machine's `ReportAccepted` (close stdin, kill after `INTERRUPT_GRACE`, remove after `RETIRE_AFTER`). `note` (1–2000 characters) is kept on the epic and shown in the digest, for what the planner needs from the orchestrator (for example an interface task it found missing).
    - **Rejected** returns decision 19's error list, counts `edits_rejected` and sets `last_rejection` to the first error's message. After `[orchestrator.planners] max_rejections` rejected submits the planner fails (`the sub-planner's epic was rejected <n> times`): the engine emits `Effect::StopPlanner { window_id, reason }`, and the machine's `Stop` kills the session.
    - A second submit after acceptance is refused: `submit_epic was already accepted for epic <e>`.

    *(Spec §3, §12.1 "`owns` inside the epic's area for a sub-planner", §19.)*

### Plan rules

23. **Rules for planners' edits**, in `run::orch::rules::check(run: &Run, touched: &BTreeSet<String>, source: &EditSource) -> Vec<PlanError>`, run after M8a's validation on every batch from the orchestrator (`edit_plan`) or a sub-planner (`submit_epic`). Plan files and the user's `run edit` keep M8a's rules only: a user's own plan is theirs.
    1. **No budgets.** A touched task with `budget` set: `task <id>: budget: budgets come from the task's size; leave budget out (rule 7.1)`. Size from evidence, never time (spec §7).
    2. **Scout evidence.** A touched `code` or `docs` task must name at least one `scout_refs` entry when the run has any finished scout report (run scouts or the `onboarding` alias of M8b): `task <id>: scout_refs: name the scout reports this task's size rests on (rule 7.1)`. Every entry must be a finished report of this run or `onboarding`: `task <id>: scout_refs: <ref> is not a finished scout report of this run`. A run with no report at all gets the note `size not backed by a scout report` on the task instead.
    3. **The cap.** Tasks with no epic count toward the orchestrator; tasks of epic `e` toward `e`'s sub-planner. Unfinished and finished tasks both count; cancelled ones and decision 37's engine-made integration review tasks do not. Over `planner_task_cap`: `tasks: the orchestrator's plan has <n> tasks, more than planner_task_cap (<cap>); plan the rest through sub-planners (rule 5.1)`, or `tasks: epic <e> has <n> tasks, more than planner_task_cap (<cap>); split the epic (rule 5.1)`.
    4. *Removed 2026-09-22 (chain collapse is the planner's judgement; spec §7.3). The engine does not check chains; contract rule 13 and the planner's rule 4 carry it as guidance.*
    5. **Epics exist.** A task whose `epic` names no epic of the run: `task <id>: epic: <e> is not an epic of this run; create it with spawn_subplanner`. The orchestrator may add a task to an epic whose planner has finished (a fix task after an integration review); while the epic's planner is live, `task <id>: epic: epic <e> is being planned by its sub-planner`.
    6. **Reserved ids.** A task id ending in `-int` followed by digits is reserved for decision 37's integration reviews: `task <id>: id: ids ending in -int<n> are reserved for integration reviews`.

    Rule ids in `PlanError.rule`: `7.1.budget`, `7.1.evidence`, `5.1.cap`, `2.epic`, `5.3.reserved`. *(Spec §7.1, §7.3, §12.1, §22.3.)*

23a. **Protected lookalikes, for every run** (*controller's rulings of 2026-09-27, rounds 1 and 2*; followups file, "Non-ASCII case folding in `ProtectedMatcher`"). M8a's matcher folds ASCII case only, but a case-insensitive macOS volume also folds letters such as `ſ` (U+017F) onto `s`, so `owns = ["AGENTſ.md"]` passed the done gate and opened as `AGENTS.md`. M8b closed it for the fast path only; M9 is the first milestone in which models write `owns` for planned runs. The workspace has no Unicode normalisation crate, so M9 refuses lookalikes rather than folding them (the strict rule, accepted in round 2: every changed path with a non-ASCII character needs an exact `owns` entry):
   - `run::globs::ProtectedMatcher::matches(path)` also returns true for any path with a non-ASCII character in any component, so the done gate treats such a changed file as protected: it passes only when a task's `owns` names that exact path, byte for byte (M8a's exact-naming rule).
   - `run::globs::may_cover_protected(entry, _)` returns true for any `owns` entry with a non-ASCII character, so M8a's validation requires such an entry to be a literal path, never a wildcard, whoever wrote it (plan file, the user's `run edit`, the orchestrator, a sub-planner).
   - M8b's fast-path rule (a non-ASCII `owns` entry leaves the fast path) is unchanged.
24. **Kinds `research` and `review` are executed.** M8a decision 6's refusal is removed (its test `research_and_review_kinds_are_deferred` is replaced by the ones below). In M8a's `run/validate.rs`, for every source:
    - A research or review task must have an empty `owns` (`task <id>: owns: research and review tasks change nothing; leave owns empty (rule 5.2)`); M8a's `owns_required` applies to `code` and `docs` only. With no `owns`, these tasks never take part in implicit dependencies, the hub rule, the source rule or the cross-runtime rule.
    - Their test mode is forced to `none` with the note `test mode none: research and review tasks change nothing (rule 8)`; a given `test_mode_reason` is kept, none is required.
    - `PlanTask` gains `review_target: Option<String>`. A review task requires it (`task <id>: review_target: required for a review task (rule 5.2)`); any other kind refuses it (`task <id>: review_target: only review tasks have a review target`). Its syntax is one revision, or `<a>..<b>`, each part matching `^[A-Za-z0-9._/@^~-]{1,200}$` and not starting with `-` (`task <id>: review_target: <t> is not a revision or a range <a>..<b>`). Whether it resolves is checked at dispatch (decision 36).
    - Size is still required (it sets the budget); the S and M rules about modules do not apply to an empty `owns`.

    *(Spec §5.2, §8 table "none for research and review tasks", §21.)*
25. **Editing blocked work.** Two additions to M8a decision 13, for every source, in `run/edits_orch.rs` (`run/edits.rs` is 577 lines and gains only the dispatch arms):
    - **`amend_task` gains `deps: Option<Vec<String>>`**, allowed on `pending`, `queued` and `blocked` tasks, validated like `add_dep` (every id exists, no cycle, no cancelled dependency). It replaces the task's whole dependency list, which closes the M8a.6 follow-up "no `remove_dep`/`replace_dep`". A `blocked(dep_cancelled)` task whose new deps name no cancelled task returns to `pending`. This is how a planner replaces a cancelled dependency (spec §12.2 "every task depending on it becomes `blocked(dep_cancelled)` for the orchestrator to re-plan").
    - **Rewriting a mis-sized task restarts it.** An accepted `amend_task` that changes `brief`, `acceptance`, `size` or `route` of a task in `blocked(mis_sized)` re-enters it exactly as M8a decision 42's retry does (a fresh session at rung 2, `failures = 1`), once the batch is applied. The amended size may be `S` or `M` again: rung 3 raised it because the old task did not fit, and the rewrite is the planner's new claim; M8a's size rules then apply to the new values and can raise it again. A `blocked(human)`, `blocked(conflict)` or `blocked(environment)` task is never restarted by an edit — only by the user's `anthrex run retry`, because rung 4 is a spend ceiling the user must lift, and a conflict or a broken environment is not something a new brief fixes. The restart lives in `run/engine/orch.rs` and calls M8a's retry entry; `engine/ladder.rs` does not change.

    *(Spec §10 rung 3 "the orchestrator … splits or rewrites it", rung 4, §11.4 step 5.)*

### Planning and the plan gate

26. **Goals on the plan and large paths start.** M8b decision 22 step 6 refused them (`driver/adapt_goal.rs::planned()`, `:244`, called at `:150, :160, :165`); now `RunService::request(StartGoal)` builds a run for `TriageRoute::Plan` and `TriageRoute::Large` at those three call sites:
    - **The project-settings check** of decision 9 runs first, for the orchestrator's runtime and `[orchestrator.planners] runtime`.
    - The driver builds the run through `RunService::build_plan` (`driver/requests.rs:245`), so every M8a start check runs, with a `Plan` whose task list is empty (M8a's validation accepts an empty `tasks`; if its merged code refuses one, that one check is skipped for a planned run, recorded under "Implementation notes"). The pure `run::orch::make_planned` (Interfaces) then sets `state = Planning`, `path` from the route, `triage`, `Run.orch.orchestrator = Some(OrchestratorRecord)` with the route of decision 6, `Run.orch.yes` from the request's `yes`, and `Run.orch.installed`. `BuildContext.yes` is passed **false**: it would set `approved_by = "--yes"` and start the run at `Start`, which is M8b's fast-path behaviour; a planned run's `yes` applies at submit (decision 27).
    - **Start-time reach checks.** `run::reach::reachable_runtimes` (`reach.rs:37`) derives runtimes from tasks only, so an empty plan would reach none and M8a decisions 50/53 would check nothing. It gains the orchestrator's route runtime and the planners' runtime when `Run.orch.orchestrator` is `Some`, so the runtime checks (binary present, user-settings-only caps, the Codex `.codex` tree) cover them at start, and every later edit batch's `refusals` (decision 15) keeps covering them.
    - `EventKind::Start` of a `Planning` run emits M8a's `CreateRunBranch` (the run branch and integration worktree exist from the start, M8a decision 14), then `OpKind::CreateOrchestrator`. No task, no pre-warm and no gate yet.
    - The reply is M8b's `RunReply::Triaged { triage, run_id: Some(id), message }` with `message = planned_message(&info, id, path)` (Interfaces). `run::triage::refused_message` (`triage.rs:356`) loses its last caller and is deleted.
    - `RunState::Planning` (label `planning`) is new and last in its enum. On `Restore` a `Planning` run becomes `Paused` with `paused_from = Planning`, like `Running` (it has live agents); `run resume` returns it to `Planning`, restarts the orchestrator (decision 11), and starts nothing for failed planners (decision 32).
    - M8a's `complete_pass` (`engine/complete.rs`) treats an empty task list as all finished, so decision 38's `plan_submitted` condition is what keeps a `Planning` run from completing; the test `planning_run_never_completes` pins it.
    - `run approve` of a `Planning` run is refused: `run <id> is still being planned; approve it when the orchestrator has submitted the plan`. `run reject` discards it (M8a decision 20).

    *(Spec §5.1 table rows "Plan" and "Large"; M8b decision 22, "Produces for later milestones" row `RunRequest::StartGoal`.)*
27. **Submitting the plan.** `edit_plan { submit: true }` (after its edits, in the same batch):
    - In `planning`: refused with `the plan has no tasks yet; add tasks before submitting` when no unfinished task exists, and with `sub-planner <e> is still planning; submit when every sub-planner has finished` while any planner is `Queued` or `Planning`. Otherwise, with the run's `yes`, the run goes to `running` with `approved_by = "--yes"` and M8c's `Run.approved_at` set (`model.rs:504`); without it, to `awaiting_approval`, and M8a decision 14's pre-warm starts. `OrchestratorRecord.plan_submitted = true`.
    - In `awaiting_approval`: accepted with no change of state (a resubmission after edits).
    - In `running` with a promotion hold in `Drafting` (decision 29): the hold becomes `Awaiting`.
    - Otherwise `submit` is ignored and the reply's `awaiting_approval` is false.
    - A `spawn_subplanner` while `awaiting_approval` returns the run to `planning` (`plan_submitted = false`); pre-warmed worktrees stay. Other edits in `awaiting_approval` are applied at once and seen by the user live in the run view (M8a decision 14 already allows edits while the gate is open).

    *(Spec §5.3 step 3, §12.3.)*
28. **Holds: approval for work added after the gate.** Spec §12.3 makes "any later edit that adds an epic" wait for approval without blocking a tool call or the running work.
    - `Task.orch.gate_hold: Option<String>` names a `GateHoldRecord` in `Run.orch.gate_holds`. A task whose hold is not `Approved` is not runnable (one condition added to M8a decision 41's runnable test in `engine/dispatch.rs`), is not pre-warmed, has no session, and so receives no outbox delivery and is never a `running` message recipient (decision 42b).
    - **Epic holds.** A `spawn_subplanner` of a **new** epic on a `running` run creates hold `epic:<e>` in `Drafting`; every task its planner adds carries it; when its `submit_epic` is accepted the hold becomes `Awaiting`, or `Approved` with `decided_by = "--yes"` when the run was started with `--yes`.
    - **Verdicts.** `RunRequest::ApproveHold { run_id, hold }` makes it `Approved` (`decided_by = "user"`) and its tasks runnable. `RunRequest::RejectHold { run_id, hold }` makes it `Rejected` and cancels its tasks (none has started, so nothing is salvaged; dependents become `blocked(dep_cancelled)` as M8a decision 13 says). Both answer `RunReply::Done`, or `Refused` with `run <id> has no hold <hold>` or `hold <hold> is <state>`. `anthrex run approve <run> --hold <hold>` and `anthrex run reject <run> --hold <hold>` send them.
    - **Completion** waits while any hold is `Drafting` or `Awaiting` (decision 38).
    - The run view shows an `Awaiting` hold's tasks as not yet approved and lets the user approve or reject it (task M9.15).

    *(Spec §12.3 "any later edit that adds an epic … Non-blocking".)*
29. **`run promote` promotes.** M8b recorded the request (`engine/requests.rs::promote`, `:174-210`); M9 acts on it, and the body moves to `run/engine/promote.rs`. `EventKind::Promote` on a fast-path run that is not terminal, not `complete`, and has no orchestrator:
    - runs decision 9's project-settings check (in the driver, before the event, on `spawn_blocking`, against the run's `base_sha`), resolves the orchestrator route (decision 6, `run promote <run> [--orchestrator …]`), sets `path = Plan`, `promote_requested_at = now`, `Run.orch.orchestrator = Some(..)` with `plan_submitted = false`, and emits `OpKind::CreateOrchestrator` with `promoted_first_prompt`;
    - replies `Done` with `promoted: run <id> now has an orchestrator; it starts in a moment (anthrex run status <id>)`, replacing M8b's "recorded: … milestone 9" text (`requests.rs:208`);
    - removes M8b's attention line `promotion requested; it takes effect when the orchestrator exists (milestone 9)` (`run/model_adapt.rs:25`). The TUI's `inspector/run.rs:231` matches the prefix `promotion requested` and adds the local time; that rewrite goes with the line (task M9.15);
    - leaves the fast-path task `t1` exactly as it is: it keeps running through its gates;
    - lifts M8b's fast-path refusal of `add_task`/`split_task` (`requests.rs:286-293`), because that refusal keys on `path == Fast`; M8b's test `a_fast_path_run_refuses_task_additions` still holds for an unpromoted run.
    - While `plan_submitted` is false, every task added to the run (by the orchestrator, or by a planner it starts) carries hold `promotion`, created `Drafting` on the first such task. `submit` makes it `Awaiting`; the user approves it with `anthrex run approve <run> --hold promotion`. After that, only new epics are held.
    - **Requests recorded before M9.** On `Restore`, a non-terminal fast-path run with `promote_requested_at` set and no orchestrator is promoted as above when it is next resumed or, if it is `running`, on the first `Tick`.
    - An already promoted run: `Done` with `run <id> was already marked for promotion`, **without** a time. M8b's text appended `at <hh:mm>` in UTC through `model_adapt::hh_mm` (`requests.rs:195-199`), which a reply the user reads locally should not; `hh_mm` (`model_adapt.rs:33`) loses its last caller and is deleted (M8c.1 review follow-up "`run promote`'s repeat reply is in UTC", owned by M9). A plan-file or planned run: M8b's `run <id> is not a fast-path run`. A terminal or `complete` run keeps M8b's `run <id> is <state>` (`requests.rs:187-193`).

    *(Spec §5.1 "`anthrex run promote` turns it into a planned run at any time"; M8b decision 25.)*
30. **The verdict reaches the orchestrator through `run_status`.** The digest's `gate` object (Interfaces) says `planning`, `awaiting_approval`, `approved` (with who and when), and each hold's state. An approval, a rejection of a hold, and every user edit made while the gate is open or after it (`run edit`, `run message`, `run refresh`, the run view's `e` and `d`) change the digest, so a waiting `run_status` returns, and each also adds a wake note (decision 39). A rejected initial plan discards the run (M8a decision 14): the digest's `run.state` becomes `discarded`, every later mutating tool answers M8a's `run <id> is discarded`, and the orchestrator window becomes a plain window (decision 11's run-live flag is cleared). *(Spec §12.3 "the verdict arrives through `run_status`", replacing conversation-view decision 12.)*

### Sub-planners

31. **Sub-planner sessions run on M8b's scout machine** (*controller's ruling of 2026-09-27*: M8a's per-round machinery is keyed by task index throughout — `engine/signals.rs::on_signal` searches `run.tasks[i].rounds`, the outbox addresses tasks, and fallback, ladder and stalls take `i` — so a `RoundOwner` for planners would rewrite the engine's hottest files far past every budget; M8b's `ScoutMachine` is exactly the lifecycle a planner needs).
    - **Where.** `scout/planner.rs` adds `ScoutService::start_planner(spec: PlannerSpec) -> anyhow::Result<ScoutHandle>`, `accept_planner(window_id)` and `stop_planner(window_id, reason)`, on the same table, machine and session driving as `start`. A planner entry is tagged in the table (`Scout.planner: Option<PlannerTag { run_id, epic, session }>`), so `run_scouts(run_id)` lists only scouts (planners are reported through `RunInfo.planners`, decision 33). Its internal id is `<h4>-plan-<e>-<n>` (inside `valid_id`); its window is named `<h4>/plan-<e>.p<n>`.
    - **The machine.** M8b's `scout::machine::step`, unchanged in its rules: nudge once after a turn without an accepted submit, fail on the second; wrap-up at `max_tool_calls`, killed at 1.5 times that; fail after `timeout_secs`; fail on process exit; `ReportAccepted` retires the session; `Stop` kills it. `ScoutLimits` gains `texts: MachineTexts { nudge, wrap_up: fn(u32) -> String, submit_tool, noun }`, so a planner gets `PLANNER_NUDGE`, `planner_wrap_up`, `mcp__anthrex__submit_epic` and failure texts naming "the sub-planner" (`the sub-planner ended two turns without an accepted epic`, `the sub-planner used <n> tool calls without an accepted epic`, `the sub-planner ran longer than <n> s`, `the sub-planner's process exited without an accepted epic (code <c>)`). The limits are `[orchestrator.planners] max_tool_calls` and `timeout_secs`.
    - **Its tools.** `get_context` goes to the driver's read path; `submit_epic` goes to the engine (decision 15's routing), which answers it and emits `Effect::PlannerAccepted` or `Effect::StopPlanner` (decision 22). The machine counts every tool use but the submit tool, as it does for scouts.
    - **Route.** `[orchestrator.planners] runtime` (default: the orchestrator's runtime), strength `frontier`, effort `high`, resolved with M8b's `scout::spec::route`; decision 43 records the choice.
    - **Read-only, launched as M8b's area scouts:** `cwd` is `root` (decision 20a); `instructions` is `PLANNER_CONTRACT`; `allowed_tools` is `mcp__anthrex__get_context`, `mcp__anthrex__submit_epic`, `Read`, `Glob`, `Grep`; `REVIEWER_PERMISSION_MODE` (`dontAsk`) with `REVIEWER_DISALLOWED_TOOLS`, an empty-root `ClaudeSandbox` with the protected denials, and Codex `read-only` with `codex_config_guard` when Codex loads project config; no output filter; the credential scrub and `ENABLE_TOOL_SEARCH=false` that `HeadlessHandle::spawn` gives every headless Claude session. *(Not `--permission-mode plan`, which M8a.1 found blocks an allowed MCP call under `-p`; F1c N4 requires read-only roles under a read-only OS sandbox.)*
    - `mcp = McpTarget { role: Planner, run_id, task_id: None, scout_id: None, epic: Some(e) }`; `mcp_args` gains a `Planner` arm and passes `--epic <e>` after `--scout` and before `--window`; `run_ref = RunRef { run_id, task_id: None, role: Planner, session: n }`.
    - **A reader slot** is held while the session is live. Spec §13 item 3 lists scouts, reviewers and deciders; a sub-planner is the same kind of read-only reader, and without a slot a large goal would start every planner at once. When a slot frees, M8b's order extends to: deciders, then reviewers (task reviews and decision 37's integration reviews), then sub-planners, then run scouts, then research and review tasks. The engine counts live planners and run scouts toward `readers_busy`.
    - **First turn:** `planner_prompt` (or `replan_prompt`), Interfaces. The prompt layout follows spec §14.2: the fixed contract as the system prompt, then the prompt, the request last.
    - They are unattended: the user watches them through the conversation view and cannot type to them (M8a decision 49's refusals apply unchanged). *(Spec §4 table row "Sub-planner", §4.2.)*
32. **Sub-planner lifecycle**, in `run/engine/planners.rs`, which keeps `EpicRecord` (phase, sessions, counts, rejections) and never owns the session's turns:
    - The reducer queues a planner (`PlannerPhase::Queued`) and emits `OpKind::StartPlanner { spec }` when a reader slot is free; the driver calls `ScoutService::start_planner`, answers `OpResult::PlannerStarted { window_id }`, then awaits the handle's outcome and sends `OrchEvent::PlannerEnded { run_id, epic, session, outcome, usage }`.
    - An accepted `submit_epic` finishes it (decision 22). A machine failure (`PlannerEnded` with `Failed { reason }`) fails it with that reason; so does `max_rejections` (decision 22). A failed planner's epic keeps any tasks already accepted from an earlier session; the orchestrator is woken (decision 39) and may start a fresh one with `spawn_subplanner`.
    - **After a daemon restart**, a live or queued planner is **not** resumed: on `Restore` it becomes `Failed { "the daemon restarted during this sub-planner" }`, like a run scout (decision 20), and the orchestrator's first wake names it. Nothing is killed (the process-kill rule; decision 20's reasoning). `OpKind::StartPlanner` reconciles as `NotStarted`. The scout machine has no resume, so there is no `RESUME_PLANNER` text.
    - Planner usage is added to `Run.orch.planner_usage` and to `RunInfo.usage.by_role["planner"]`.
    - **Finished planner nodes stay** (spec §16.2); the session window is removed after `RETIRE_AFTER`.

    *(Spec §3 "a sub-planner exits once its tasks are accepted; it never relays results", §12.1.)*
33. **`RunInfo.planners`** (M8c's placeholder, `run_info.rs:305`) is filled by `run/snapshot.rs` from `Run.orch.epics`, one `PlannerInfo` per epic: `epic`, `title`, `area`, `route`, `window_id` of the latest session, `state` (`Planning` while queued or live, `Finished`, `Failed`), `started_at` of the first session, `ended_at` of the last, `edits_accepted`, `edits_rejected`, `last_rejection`, `replans`; plus M9's `note`, the only new field, `#[serde(default)]` (M8c's fields carry no per-field default, `proto/src/planner.rs:20-33`; M8c's `RunInfo.planners` list itself is `#[serde(default)]`). *(M8c "Consumes from later milestones".)*

### Scouts and briefs

34. **Scouts feed every brief in their area.** M8a's `worker_prompt` gains a scout extract between the profile summary and the brief (spec §14.2: contract, profile summary, scout extract, brief last), followed by decision 42d's notes section: for each of the task's `scout_refs` that resolves to a report (M8b's `scout::report::resolve_ref`), `Scout report <id>:` then its summary cut to 4000 characters and `Files: <paths>`; at most 12 KiB in all, later reports cut first with `[anthrex] … cut …`. The extract is built by `run::orch::contract::scout_extract`, and `worker_prompt`, `handover_prompt` and their callers (dispatch, handover) take it as one new parameter, so `run/contract.rs` (584) grows by at most 8 lines. Reports are read by the driver when it builds the `CreateWindow` op and passed in `OpKind::CreateWindow.first_turn`, so the reducer stays pure; a report that cannot be read is left out with a log warning. The same extract goes into `planner_prompt` for the epic's `scout_refs`. *(Spec §5.3 step 1 "Their reports feed every planner and every worker brief in that area", §14 item 1.)*

### Research, review and integration

35. **Research tasks** are scout sessions run by the engine as tasks.
    - **Dispatch.** A runnable research task takes a reader slot (not a writer slot) and gets no worktree and no branch. Its session is `run::orch::launch::research_spec(run, task)`, launched as M8b's area scout (decision 31's read-only launch; `SCOUT_CONTRACT`, `submit_scout_report`, `Read`, `Glob`, `Grep`, `WebFetch`, `WebSearch`), `cwd` = `root`, route = the task's resolved route, `mcp = McpTarget { role: Scout, run_id, task_id: Some(t), scout_id: None, epic: None }`, `run_ref = RunRef { role: Scout, task_id: Some(t), session: n }`, window name `<h4>/<t>.s<n>`, through M8a's `OpKind::CreateWindow` with `worktree: root` (its `worktree` is a `PathBuf`). First turn: `research_prompt`.
    - **The report.** `submit_scout_report` from a task-bound scout window (a `ToolCall` with `role == Scout`, `task_id` set, `scout_id` unset) goes to the engine, not to `ScoutService`: `RunService::tool`'s scout branch (`driver/adapt.rs:153`) gains that one condition, and `anthrex mcp --role scout` takes exactly one of `--scout` and `--task`. It is validated with M8b's `scout::report::validate(args, ScoutKind::Area)`, stored on the task (`Task.orch.research`), and the task becomes `TaskState::Reported` (new, last in its enum, label `reported`, `is_finished` true). The session is retired.
    - **Failures.** A turn without a report gets M8b's `SCOUT_NUDGE` once; a second is `blocked(environment)` with `the research task ended two turns without a report`. Budgets, stalls and deaths follow M8a's worker rules, since the session is an M8a task round.
    - **Dependents.** A declared dependency is satisfied by `merged` or `reported` (M8a decision 41's runnable test).
    - **Output.** `REPORT.md` gains `## Research` with each report, and the driver writes the combined text to `<data_dir>/runs/<run>/research.md` whenever it writes the report; `RunInfo.research_report` names it. `anthrex run accept` prints `research report: <path>`. A run that merged nothing is accepted as M8a decision 20 accepts any run: `git merge --no-ff` of a run branch equal to the base does nothing, and the branches are deleted.
    - M8c's `round_label` (`tui/src/tree/run_rows.rs:32-40`) draws a task round of role `Scout` as `scout #{session}` (`:38`); M9.15 changes that arm to `research #{session}`.

    *(Spec §5.2 "research — scout tasks only. Each writes a report into the run's data directory. There is no task branch, no merge, and `run accept` shows the combined report".)*
36. **Review tasks** are M8a reviewer sessions run as tasks.
    - **Dispatch.** A runnable review task takes a reader slot. The engine first resolves its target with `OpKind::ResolveTarget { root, target, base_branch }` (`git rev-parse --verify <rev>^{commit}` for each side through `run_git`; a single revision `r` means the range `merge-base(<base branch>, r)..r`) → `OpResult::Target { base, head }`, or `blocked(environment)` with `review target <t> does not resolve: <stderr tail>`. Then M8a's `PrepareReview { root, head_ref: head, base_ref: base, path: <wt>/runs/<run>/<t>.review }`, whose `OpResult::Review { patch }` is clamped to M8a's `REVIEW_DIFF_MAX` (16 KiB): a range review of a large branch is cut there, and the prompt says so (the reviewer reads the rest with `git diff`). Then a reviewer session: `REVIEWER_CONTRACT`, `submit_review`, level by size (`S` → `small`, `M` → `medium`), route from the task (policy fills it as for any task), `RunRef { role: Reviewer, task_id: Some(t), session: n }`. First turn: `review_task_prompt`.
    - **The verdict.** The first accepted `submit_review` is stored in `Task.reviews` and the task becomes `Reported`, whatever the verdict: nothing is merged and nothing is sent back. Findings of every severity go to `REPORT.md` under `## Review findings`. A turn without a verdict follows M8a decision 35's nudge rule; two verdict-less rounds are `blocked(environment)`.

    *(Spec §5.2 "review — reviewer tasks against the named branch or range, producing findings. No merge.")*
37. **The per-epic integration review is an engine-made review task** (*controller's ruling of 2026-09-27*: a round owner of its own is ruled out for decision 31's reason; a review task reuses decision 36 with no new machinery).
    - **When.** When every task of an epic is finished, at least one merged, no planner of the epic is live, and no integration review task of the epic is unfinished, the engine adds round `n + 1` if the last round is absent or older than the epic's last merge. The epic's merges are recorded as they happen (`EpicRecord.merges`: task id and merge commit), and `EpicRecord.base` is the run head just before its first merge.
    - **The task.** The engine adds a `review` task directly (no validation source; decision 23 reserves its id shape): id `<e>-int<n>` (at most 16 characters, decision 21), title `integration review of epic <e>, round <n>`, `epic = e`, `kind = review`, `review_target = "<EpicRecord.base>..<run_head sha>"`, `size = M`, review level `frontier` (overriding decision 36's size rule), route `roster::pick_reviewer(roster, &author, Frontier)` where `author` is the route of the epic's most recently merged task, `Task.orch.integration_of = Some(e)`. It runs in a reader slot through decision 36's dispatch. Its first turn is `integration_review_prompt` instead of `review_task_prompt`.
    - **The verdict** is the task's `submit_review`. `approve` (or only minor findings) sets the epic's integration state to `Approved`. A blocking verdict sets `Changes` and holds completion (decision 38) until the orchestrator adds a task to that epic (which, once merged, triggers the next round) or ends the run with the `finish` edit (M8a's `Run.finish_edit`), which completes it with the findings in the report. After `max_bounces + 1` rounds no new round starts; the hold stays until `finish`. Nobody approves the epic by edit: review authority stays with reviewers (spec §11.4 step 6).
    - It does not count toward `planner_task_cap` (decision 23.3), and the orchestrator cannot amend, split or cancel it (`task <id> is an integration review; the engine owns it`).
    - The plan path has no epics and no integration review; the final check on the run head is M8a decision 37's.

    *(Spec §5.3 step 6.)*
38. **Completion with an orchestrator.** M8a decision 37's condition gains, for a run with an orchestrator: no hold `Drafting` or `Awaiting`; no epic whose planner is queued or live; no run scout queued or running; no integration review task unfinished; no epic with integration state `Changes` (unless the `finish` edit was accepted); and `plan_submitted`. Blocked tasks, `paused(message)` ones included, keep the run running, as in M8a. On completion the orchestrator gets the wake note `the run is complete; write your summary with edit_plan summary`. *(Spec §5.3 steps 6–7.)*

### Reacting and waking

39. **Waking an idle orchestrator.** The digest is pulled (spec §14.4), but an interactive session whose turn has ended pulls nothing, so the engine wakes it.
    - **Notes.** Each of these adds one line to `OrchestratorRecord.notes` (at most 20; the oldest are replaced by `+<n> earlier changes`): a task becoming `blocked` for any reason but `message_pause` (`<t> blocked (<reason>): <text, first 120 characters>`); a gate verdict or hold verdict (`the user approved the plan`, `the user approved hold <h>`, `the user rejected hold <h>`, `the user rejected the plan; run discarded`); a user edit (`the user edited the plan: <describe>`, M8c's `edit_log::describe`); a planner finishing or failing (`sub-planner <e> finished with <n> tasks`, `sub-planner <e> failed: <reason>`); a run scout ending (`scout <id> reported`, `scout <id> failed: <reason>`); an integration verdict (`integration review of epic <e>: <approve | changes (<n> critical, <m> important)>`); a worker's `discovery` or `risk` note (decision 42f); a refresh that failed (decision 42e); the run halting (`the run halted: <reason>`); completion (decision 38); a restart (decision 11). The orchestrator's own edits add none, and a `paused(message)` block adds none (the orchestrator or the user caused it).
    - **The reducer** emits `Effect::WakeOrchestrator { run_id, window_id, text, digest_revision }` whenever notes are pending, the orchestrator window is live, `wake_orchestrator` is on, and `digest_rev > last_wake_rev`. `text` is `wake_text(run_id, notes)`, clamped to 2 KiB.
    - **The driver** (`run/driver/wake.rs`) delivers it into the PTY only when the window's status is `Idle` or `Done` (never `Working`, and never `Attention`, which may be a permission prompt) and no client input reached the window for `wake_quiet_secs` (`WindowManager::last_client_input(id)`, set by the server on `ClientMsg::Input`, `server.rs:349`). Delivery is the superseded M8 brief's paste: `ESC [ 200 ~`, the text with `\r\n` and `\n` turned into `\r` and any paste markers removed, `ESC [ 201 ~`, then after `SUBMIT_DELAY` (200 ms, a `tokio::time::sleep`) a lone `\r`, both through `WindowManager::write_input` (the per-window writer thread), never under a lock. It then sends `OrchEvent::OrchestratorWoken { run_id, digest_revision }`, which clears the delivered notes and sets `last_wake_rev`. A pending wake is re-checked on every 1-second tick and on every window-list change; a newer `WakeOrchestrator` for the same run replaces an undelivered one.
    - **A read clears too.** `DigestRead` (decision 16) drops the notes up to the revision read, so an orchestrator that is already polling is never pasted at.
    - `wake_orchestrator = false` in `[orchestrator]` turns wake-ups off; the notes still appear in the digest.

    *(Spec §10 "the orchestrator is told through the digest", §12.4; TT §12.3 "`discovery` and `risk` also wake the orchestrator"; M8a decision 29 removed paste delivery for headless sessions, and this is the one PTY run window that needs it.)*
40. **The edit log is M8c's `Run.plan_edits`, extended** (one log, not a second `Run.edit_log`). M8c's `PlanEditRecord { at, text }` (`run/edit_log.rs:14`) gains, each `#[serde(default)]`: `source: String` (`user`, `orchestrator` or `planner:<e>`; default `user`), `accepted: bool` (default true), `error: Option<String>` (a rejected batch's first error), and `recipients: Vec<String>` (a `message`'s resolved task ids, TT §12.5). It stays capped at M8c's `PLAN_EDITS_KEPT` (50). `edit_log::record(run, edits, now)` (`:67`), today called only for accepted batches (`engine/requests.rs:352`), becomes `record(run, edits, now, source, outcome)`; `engine/orch.rs` and `engine/planners.rs` also call it for rejected orchestrator and planner batches. M8c's `plan_edits_since_approval` counts accepted batches only. `describe_one` (`:51`, an exhaustive match) gains `message <to> (<kind>)` and `refresh <id>`, within `DESCRIBE_MAX_CHARS` (300). `RunInfo.plan_edits` (`PlanEditInfo`, `run_info.rs:222`) gains the same four fields; the TUI shows accepted ones; the digest shows the last 10 (Interfaces). The reactions themselves are the contract's (decision 41) and use only edits the engine validates: `answer` for `blocked(question)` (M8a), `split_task` or a rewriting `amend_task` for `blocked(mis_sized)` (decision 25), `amend_task deps` or `cancel_task` for `blocked(dep_cancelled)`, `message` and `refresh` for running work (decision 42), and a sentence to the user for `blocked(human)`, `blocked(conflict)` and `blocked(environment)`. *(Spec §10, §11.4 steps 5–6; TT §12.5.)*

### Contracts

41. **Contracts are fixed texts** (Interfaces, exact): `ORCHESTRATOR_CONTRACT` and `PLANNER_CONTRACT` in `run/orch/contract.rs`. They never vary within a session, so the cached prefix stays stable (spec §14.2); the run's facts go in the first prompt and in `get_context`. Contract tests assert every rule the spec asks the planner to follow is present (task M9.5). Following the tool-search fix, each contract names every anthrex tool by its Claude id at its first mention, as `get_context (in Claude: mcp__anthrex__get_context)`, which stays correct for Codex. M8a's worker contract gains only the message and `task_note` rules of decision 42: `WORKER_CONTRACT` (`run/contract.rs`, as the fix left it) gains lines 10 and 11 (Interfaces "Contracts"), and M8a's exact-text test `contracts_round_trip_through_toml_string` (`run/contract_tests.rs:10`) is updated for 12 lines and the new last sentence. The reviewer sees `change` messages in its prompt (decision 42d), while its contract and the scout contract stay unchanged. *(Spec §7, §8, §9, §6, §14.2; 2026-09-26 spec §12.1, §12.3, decision 42.)*

### Additions from the 2026-09-26 amendment and the 2026-09-27 scope review

42. **Messages, refresh and notes are M9 work.** The binding tiered-testing and PR-delivery spec §12 defines recipient selection, state-dependent delivery, `stop_and_wait`, branch refresh, worker `task_note`, CLI commands, display and tests. Implement those rules without a second communication path: live-worker messages use M8a's turn-boundary outbox, a pending worker sees notes in its first prompt, and a refresh uses journaled Git operations (M8a's `OpKind::HandBack`). `Message` and `Refresh` each occupy a one-edit `edit_plan` call (and a one-edit `run edit` file or CLI request); mixing either with plan mutations, or with `submit` or `summary`, is refused before any effect with `message and refresh must be the only edit in their call`, preserving decision 19's atomic plan batches. A `Message` resolves recipients at acceptance and returns separate delivered and refused task ids, as §12 requires; one refused recipient does not roll back delivery to the others. M9.2 adds the wire types; M9.5 updates the contracts; M9.13a implements state, delivery and Git (the pure engine parts included); M9.14 adds the CLI; M9.15 adds the view; M9.16 tests the complete flow. No tool call interrupts a worker mid-turn. Decisions 42a–42i resolve §12 against the code. Two rules bind all of them: **a message never interrupts a turn**, and **a message never changes a task's `owns`, route, size, test mode or acceptance** — it carries no field that could, and those stay `amend_task`, `split_task` and `cancel_task`. *(2026-09-26 spec §12, §12.1 "Batch boundary".)*

42a. **`message`, a plan edit.**
   - *Shape:* `PlanEdit::Message { to: MessageTarget, text: String, kind: MessageKind }`, appended last. `MessageTarget` is `Tasks(Vec<String>)`, `Stage(u32)` or `Running`, written in JSON and TOML as a task-id array, the string `stage:<n>`, or the string `running` (a custom serde, so "`running` stands alone" holds by construction). `MessageKind { Info, Change, StopAndWait }`, snake_case: `info`, `change`, `stop_and_wait`.
   - *Who may use it:* the orchestrator, through `edit_plan`; the user, through `run edit --file` and `anthrex run message` (decision 42h). **Not** a sub-planner (decision 22).
   - *Limits:* `text` is 1–4000 characters; a `Tasks` list holds 1–20 ids.
   - *Kinds:* `info` is context, and the worker may carry on; `change` means the plan or the code around the task changed, the worker's contract says it must say how it applied the change in its next `task_done` summary (`Changes applied: …`), and the reviewer is shown the message (decision 42d); `stop_and_wait` is decision 42c.
42b. **Recipients, and what each task state gets.**
   - *Recipients* resolve when the edit is accepted. `Tasks` names task ids; duplicates are dropped. `Running` means every task with a live worker round (M8a's `edits.rs::has_live_worker`, `:147`), excluding tasks whose hold is not `Approved` (they have no session). `Stage(n)` is carried on the wire so milestone 9.1 needs no protocol bump, and is refused at acceptance with `stage recipients arrive with milestone 9.1` (the controller's ruling).
   - *Per task state* (the code's states; TT's "waiting" is `pending`, and its `blocked(other)` is the five non-question reasons):
     - `working`: queued in M8a's outbox (M8a decision 29) and delivered as the session's **next turn**, after the current one ends, as `[anthrex] Message from the <orchestrator|user> (<kind>): <text>`; also recorded in the task's messages (42d).
     - `blocked(question)`: queued and recorded. The outbox **holds** it until `answer` unblocks the task, and the answer and the message then go out as one turn. The task stays blocked either way; only `answer` unblocks it. *(Controller's ruling of 2026-09-27: held until the question is answered.)*
     - `pending`, `queued`, `preparing`: recorded only; the first session's prompt carries it (42d).
     - `proof`, `check` (which TT omits): queued and recorded. The outbox holds a worker's mail while its task is in a gate (M8a ruling T13-I3), so it is delivered only if a gate sends the task back to `working`; on success it stays in the record the reviewer sees. *(Controller's ruling.)*
     - `blocked(message_pause)`, i.e. `paused(message)` (42c): queued and recorded, and it may release the task (42c).
     - `review`, `merge_queue`, `merged`, `cancelled`, `reported`, and `blocked(mis_sized | human | conflict | dep_cancelled | environment)`: refused for that task only, with `task <id> is <state label>; a message would not reach a worker`, where `<state label>` is `run::edits::state_label` (`edits.rs:167`; for example `blocked(human)`).
   - *Reply:* every recorded or queued recipient is listed in `delivered`, a pending task included (its first prompt carries the message); every refused one in `refused` with its reason. A message whose every recipient is refused is refused as the call's error (`message: no recipient can take it: <reasons joined with "; ">`), with no effect; `run edit` and `run message` print one line per refused task after the reply.
   - *Rate limit:* at most `message_max_per_turn` (default 3) **undelivered** `message` texts per task, counted from `Task.orch.messages` (`delivered == false`), never from the outbox, which also carries engine texts. The next is refused for that task with `task <id> already has <n> messages waiting for its next turn`. With `message_max_per_turn = 0`, every recipient is refused with `messages to workers are turned off (orchestrator.message_max_per_turn = 0)`. Queued texts are joined into one turn by M8a's `messages::join_turn` (32 KiB clamp).
42c. **`paused(message)`: `stop_and_wait`.**
   - *Model:* `BlockReason::MessagePause`, appended last (serde `message_pause`); the task is shown and spoken of as `paused(message)`. *(A block reason, not a twelfth `TaskState`: it reuses every blocked path — no writer slot, completion waits, the fallback skipped, `cancel_task` — and avoids a second meaning of "paused", which `RunState::Paused` and the run-level `pause`/`resume` edits already use. The controller's ruling names the variant `MessagePause`, as on `main`.)*
   - *Entering it:* at acceptance, the task (which must be `working`, else 42b refuses it) becomes `blocked(message_pause)` with block text `asked to stop and wait: <text, first 120 characters>`. The stop text is queued, and `outbox::deliver` (`engine/outbox.rs:81`) gains one exception: a `blocked(message_pause)` task's mail is delivered once its current turn closes, so the worker hears "stop and wait". M8a's fallback runs only for `Working`, so the stop turn's end starts no `CountCommits` and no nudge.
   - *`task_done` while paused* is refused with `this task was asked to stop and wait; wait for the next message`, a new branch in `engine/done.rs`'s worker path ahead of its `Working`-only check (`:85`). A worker mid-turn that tries `task_done` hears it at once, which is the fastest stop; otherwise the stop takes effect at the end of the current turn (the stall watchdog fires only on silence, so a busy turn is bounded by its budget, not by the watchdog).
   - *Releasing it,* in TT §12.1's words, "a later `message`, `amend_task` or `resume` for that task": the next accepted `message` of kind `info` or `change` to that task, which sets it `working` and delivers as the next turn (a further `stop_and_wait` is refused with `task <id> is already paused(message)`); an `amend_task` of `brief` or `acceptance`, which sets it `working` and delivers M8a's `amend_message`; or the `resume` plan edit. The protocol has no per-task resume: `PlanEdit::Resume` is run-wide and accepted only on a paused run (`engine/requests.rs::pause_or_resume_fits`), so the `resume` of §12.1 is that edit (`{"op": "resume"}` through `edit_plan` or `run edit`), which, when it resumes the run, also releases every `paused(message)` task of the run. `anthrex run resume` after a daemon restart (M8a's `RunRequest::Resume`, not a plan edit) does not, so a pause survives a restart. `cancel_task` works as for any task. `answer` is refused (only `blocked(question)` is answerable).
   - *Refused while paused:* `run retry` and `run override` (`task <id> is paused(message); send it a message to resume it`), and the route, size and test-mode amends (`edits.rs::not_started` counts a `blocked(message_pause)` task as started).
   - *Scheduling and attention:* no writer slot while paused; it keeps the run from completing, as blocked tasks do. The run's attention line lists it only after it has lasted 600 s (`<t> paused(message) for <n> min`, computed in `snapshot.rs` from `now`). It adds **no** wake note.
42d. **The task's messages and task notes survive sessions.**
   - *Model* (the field `Task.notes` is M8a's validation notes, `model.rs:172`, and "hold" is M8a's dependency hold): `Task.orch: TaskOrch` holds `messages: Vec<TaskMessage { at, source, kind, text, delivered: bool }>` for every accepted message to the task, and `worker_notes: Vec<WorkerNote { at, kind: TaskNoteKind, text }>` for its `task_note`s (42f).
   - *Prompts:* `worker_prompt` and `handover_prompt` (rung 2 and every fresh session, a rewritten task's included) place `Notes from the orchestrator:` then `- <hh:mm> (<kind>, from <source>) <text>` for every recorded message, just before the brief (after decision 34's scout extract). A resumed session already has the delivered ones, and the undelivered ones follow as a turn.
   - *Reviewer:* `reviewer_prompt` lists the task's `change` messages under `Messages the worker received:`.
   - *No engine check* on the `Changes applied: …` summary (the controller's ruling): the contract carries it, and the reviewer judges it. The engine never interprets an acknowledgement.
   - *Snapshot* (`main`'s lighter fields, decision 16a): `TaskInfo.message_count: u32`, `last_message_kind: Option<MessageKind>`, `last_message_line: Option<String>` (the most recent message's first line, cut to 80 characters, for TT §12.6's inspector row), and `task_notes: Vec<TaskNoteInfo>` (the task's last 10).
42e. **`refresh`, a plan edit.**
   - *Shape:* `PlanEdit::Refresh { task_id }`, appended last. The orchestrator and the user may use it; sub-planners may not (decision 22).
   - *Allowed on* a `working` or `paused(message)` task that is not waiting on dependencies (`awaiting_deps`), not `resolving` a conflict, and has no gate op and no `HandBack` in flight. Otherwise `task <id> is <state label>; refresh needs a working or paused task`.
   - *Clean-tree check, twice* (the controller's ruling). **At acceptance:** the driver (`run/driver/refresh.rs`) runs `git status --porcelain -z` in the task's checkout before it sends the edit event, on `spawn_blocking`, bounded by `DONE_CHECK_GIT_TIMEOUT`, through M8a's `run_git` (`--no-optional-locks`, scrubbed environment, AGENTS.md rule 11); any tracked change, staged or not, refuses the call with `task <id> has uncommitted changes; send it a message asking it to commit first`. The pure edit then records `Task.orch.refresh = Some(RefreshState::Due)`. **At the turn boundary:** the executor re-checks before merging (below).
   - *Merge, never rebase.* At the next turn boundary (the worker round's turn closed, no delivery or resume in flight), the engine emits **M8a's `OpKind::HandBack { worktree, run_head: run.run_head, task_head }`** with a new `#[serde(default)] list_merged: bool`, set true for a refresh, and records `refresh = InFlight(op)`. The executor is M8a's `git::hand_back` (`git/handback.rs:56`) — `merge-tree --write-tree`, `commit-tree` with explicit parents, the CAS `update-ref` of `anthrex/<run>/<task>`, and the engine-written `MERGE_HEAD` on a conflict; no `git merge`, no rebase — which already refuses staged changes. With `list_merged` the executor first repeats the clean-tree check (a dirty tree answers `OpResult::Failed { message: "uncommitted changes" }`), and after the merge reads `git log --format=%h%x1f%s -n 20 <onto>..<run_head>` into `OpResult::HandedBack { #[serde(default)] merged: Vec<String> }`. While a refresh is `Due` or `InFlight`, `outbox::deliver` holds the task's mail, so the refresh result and a `change` message sent in the next call go out as **one** turn.
   - *Result routing:* one arm in `engine/mod.rs`, ahead of M8a's `HandBack` routing (`:516-519`), matches `task.orch.refresh == InFlight(op)` and calls `worker_messages::refreshed`:
     - **Clean:** queue `[anthrex] Your branch now includes the latest merged work (<n> commits: <sha7> <subject>, …). Rebuild before you continue.` and record the new merge commit in `Task.orch.refresh_merges`.
     - **Already up to date** (`head == onto`): no worker message; history line `refresh: nothing new`.
     - **Conflict:** set `resolving = true`, exactly as M8a ruling N5's `holds::handed_back` does (`engine/holds.rs:205`); record the merge commit; queue `[anthrex] Merging the latest run branch into your worktree conflicted in: <files>. Resolve them, commit, and continue.` `Task.conflicts`, which only the merge queue counts toward `blocked(conflict)` (`engine/merge.rs:255`), is **not** touched.
     - **Failed** (a dirty tree at the boundary included): no block; a history line `refresh skipped: <reason>`, the wake note `refresh of <t> skipped: <reason>`, and the edit-log entry's `error` set.
   - *Reconcile:* M8a's `HandBack` row (`reconcile/mod.rs:179`) replays it unchanged.
   - *Never the task's own work.* Refresh merges `Run.run_head` (the engine's recorded head, which equals the run branch under the ref guard), never the live ref, so every range M8a and M8b measure already excludes the merged commits: `verify_done`'s spill diff `<run_head>...<head>` and its commit count `<head> ^start ^run_head` (`git/done.rs`), `diff_so_far`'s `<run_head>...HEAD`, and M8b's `MeasureDiff` (`run/history_io.rs`, `engine/history.rs`). The one hole is the engine-made merge commit itself, which is `head ^start ^run_head`: so `engine/done.rs`'s "no commit since the task started" check (`:257`) and the fallback's `NO_COMMIT_NUDGE` decision exclude every commit in `Task.orch.refresh_merges`, and a refresh merge alone is never work. M8a ruling N5's mid-task hand-back has the same property and is left as it is.
42f. **`task_note`, a worker tool.**
   - *Shape:* `task_note { kind: discovery | risk | progress, text }`, `text` 1–4000 characters (TT §12.3 gives no limit, so `main`'s). `TaskNoteKind { Discovery, Risk, Progress }`. Description: `Report a discovery, risk or progress without blocking the task.`
   - *Allowed tool:* `run::role_launch::WORKER_MCP_TOOLS` (`role_launch.rs:19`) becomes `[&str; 3]` with `mcp__anthrex__task_note`, so a real Claude worker in `acceptEdits` mode (the config default) may call it; `mcp::tools_for(Worker)` lists it.
   - *Where it is accepted:* from the task's current live worker round in any unfinished state, `paused(message)` included. It never changes the task's state. It passes M8a's run gate like every worker tool (`running` only; a paused run answers M8a's paused text), then bypasses the worker path's `Working`-only check.
   - *Recording:* each note is appended to `Task.orch.worker_notes` and to the task's history (`note (<kind>): <text, first 120>`). The `note_max_per_task + 1`th answers `note limit reached; put the rest in your task_done summary`; with `note_max_per_task = 0` every note answers it.
   - *Reply:* `Note recorded. Keep working.`
   - *Wakes:* `discovery` and `risk` add the wake note `<t> noted a <kind>: <text, first 120 characters>` (decision 39); `progress` does not.
   - *Digest:* `task_notes: [{task, kind, text (cut to 400), at}]`, the last 10 across tasks, newest first. It is not named `notes`, which the digest already uses for wake notes.
   - *Contract:* `WORKER_CONTRACT` gains TT §12.3's sentence (decision 41).
   - This resolves TT's own contradiction between §6.9 ("no new tool") and §12.3 for M9: §12.3 wins; §6.9 is milestone 9.2's.
42g. **Config.** `[orchestrator] message_max_per_turn = 3` (0..=20; zero turns messages off) and `[orchestrator] note_max_per_task = 10` (0..=100; zero turns notes off), in `AgentSettings` (decision 3). Both are frozen into the run's limits at run start (`RunLimits.orch`, Interfaces "config"), so a later config edit cannot change the rules of a live run.
42h. **CLI.** `anthrex run message <run> <task|stage:<n>|running> [--kind info|change|stop_and_wait] <text>` (default `info`) and `anthrex run refresh <run> <task>`. Both send the existing `RunRequest::Edit { run_id, edits: [<one edit>] }` with source `user` (label `run edit`), the same edit and journal path as the orchestrator: **no new `RunRequest`**. They never write into a headless agent's terminal. Their bodies are in `crates/cli/src/run_cmd/orch.rs`; `run_cmd.rs` takes only the two subcommand variants. A `stage:<n>` recipient parses, and the daemon refuses it (42b).
42i. **Display.**
   - The run view shows `paused(message)` with its own glyph `‖` and colour, and the run's attention line lists it after 600 s (42c) (task M9.15).
   - The task inspector gains a `messages` row: the count, and the most recent message's kind and first line (`message_count`, `last_message_kind`, `last_message_line`).
   - `discovery` and `risk` notes appear in the task's `history` and in the run's attention line (`<t> noted a <kind>: <first 80>`, the last 3), attributed to the task.
   - The report's per-task section (`report_task.rs`) gains `Messages:` and `Notes:`.
   - **Conversation view (TT §12.6):** a delivered message is a user turn whose text starts with `[anthrex] Message from the orchestrator (` or `[anthrex] Message from the user (`; the TUI recognises that prefix and labels the turn `orchestrator` or `user`, with **no protocol change** (the controller's ruling). The recogniser lives in a new file and is called from `ui/conversation.rs` (389), not `conversation.rs` (595).
   - The edit log records every message with its source and its resolved recipients (decision 40), keeping TT §12.5, and the report lists them.
43. **Role routing history complements M8b decision 33a.** For each orchestrator, sub-planner, run scout and decider session, including a decider that runs before a run exists, record a `RoleRoutingDecision` at dispatch. It has a stable `record_id`, time, optional run and task ids, role and session identity, trigger, source, policy version, role-specific input, chosen `Route`, selected index, and the **full ordered, resolved candidate snapshot** with skip reasons. The input includes the run path, goal capped like M8b's `RunRecord`, profile languages, and applicable epic, area or decider question kind; it excludes transcripts, credentials and raw tool output. The selected route is appended if absent from the candidate source. A skip reason says why the dispatch did not take a candidate (`not installed`, `not in the configured list`, `an earlier candidate was taken`); **an unchosen candidate is never labelled a failure**, and no record attributes the whole run's outcome to one role. A run-bound decision is kept in the persisted `Run.role_routing_decisions` (`#[serde(default)]`) before its session-start effect and is appended once as `HistoryLine::RoleRoute` when the session ends or recovery establishes its interruption. Pre-run triage appends the same record after its answer or fallback, even if no run is created. The record keeps the **factual session outcome** (`completed`, `failed`, `interrupted` or `fallback`, plus the role's existing accepted/rejected, report or submission status in `result`). Existing history lines and runs load unchanged. M9.2 adds the history schema and its round-trip tests; M9.13b implements capture and idempotent append; M9.16–17 cover real sessions. Milestone 9.5's configured role lists later supply the candidate snapshot, and record the `spread` rotation position.
    - **Capture points.** The orchestrator: `OpKind::CreateOrchestrator` and `RestartOrchestrator` (`run/engine/orch.rs`), one session id per launch and per restart, candidates from decision 6's resolution. A sub-planner: `OpKind::StartPlanner` (`run/engine/planners.rs`), a new session id for every re-plan or retry (a new `EpicRecord.sessions` entry). A run scout: `OpKind::StartScout`. A decider: M8b's deciders run in the driver (`RunService.adaptation.deciders`, outside the reducer), so a run-bound decider's record reaches the engine as `OrchEvent::RoleRoute { run_id, decision }` before the call and `OrchEvent::RoleRouteEnded { run_id, record_id, outcome, result }` after it. Pre-run triage (`driver/adapt_goal.rs`) has no run: the driver appends its record directly with M8b's `history_io::append_line`, on `spawn_blocking`, once the answer or fallback resolves.
    - **Completion.** The session-end event (`OrchestratorWindow { live: false }` or the run becoming terminal, `PlannerEnded`, `ScoutEnded`, `RoleRouteEnded`) sets `outcome` and `result` and emits M8b's `OpKind::AppendHistory` for that line; `Restore` marks every unfinished record `interrupted` and appends it. The append is idempotent by `record_id` (M8b's `history_io::contains_record`, and `read_history` keeps the last line of each `record_id`), so recovery never duplicates a record.
    - `record_id` is `<run id>/<role>/<session id>` for a run-bound record and `triage/<run-or-request id>/<n>` for pre-run triage; a session id is the one the session runs under (the orchestrator window's Claude session id, the scout machine's id, the decider call's id), never reused.
    - `run stats` ignores `RoleRoute` lines for its task-size and budget aggregates (`run/stats.rs`).
    *(Adaptive spec §15, last paragraph.)*
44. **Start a goal from the TUI.** `C-b g` opens a pure goal form for the selected Git project (or the focused window's project). It has a goal text area (`Ctrl-J` inserts a newline), optional orchestrator runtime and model, and a `trust_project` toggle that starts off; the plan gate remains on by default. `Enter` sends M8b's existing `RunRequest::StartGoal` with M9's optional orchestrator choice, `Esc` cancels, an error stays in the form, and success opens the run view on the new run. With no selected project, it says `select a Git project to start a goal` and sends nothing. It does not create a Git repository or edit its profile. M9.15 implements the form and M9.16–17 exercise it through the real client.
    - **Keys and state.** `Command::StartGoal` on `C-b g` (the prefix map, `tui/src/keymap.rs:116-135`, does not bind `g`); `Modal::StartGoal(crate::run_goal::GoalForm)` beside M8c's `Modal::EditTask` (`app/mod.rs:77`). The form's code lives in new `crates/tui/src/run_goal.rs` and its renderer in `crates/tui/src/ui/run_goal.rs`, not in `app/runs.rs` (524 lines).
    - **The project** is the selected `NodeKey::Project(root)`, or the selected run row's project, else the focused window's `WindowInfo.project` (`proto/src/types.rs:130`).
    - **The request** is `RunRequest::StartGoal { goal, dir, yes: false, trust_project, unconfined_checks: false, orchestrator }` (`proto/src/run_wire.rs:79`), sent tagged (decision 2). The form sends exactly what `anthrex run start --goal` sends; it has no `--unconfined-checks` toggle, so on Linux, where a run needs `--unconfined-checks` or `[orchestrator] unconfined_checks = true`, the form's start is refused with M8a's text unless the config sets it.
    - **Success** is `RunReply::Triaged { run_id: Some(id), .. }` for the tagged id, which M8c's `on_run_reply` ignores today (`app/runs.rs:193`). The reply can arrive before the snapshot that names the run, so the app keeps `pending_open: Option<String>` and opens the run view when a snapshot first names that run. A fast-path answer opens the same way. `run_id: None` (triage refused the goal) toasts the message and closes the form.
    - **Refusal** is `Refused { request: "run start --goal", request_id }` (`request::START_GOAL`, `run_wire.rs:207`), shown inline; the form keeps its input.
    - **While submitting**, only `Esc` acts, as in M8c's edit form. Triage can take minutes (smoke's `GOAL_CMD_TIMEOUT = 900.0`).

    *(M8c's pure form and effect pattern; adaptive spec §16 introduction.)*

## Interfaces

### `proto`

`crates/proto/src/run.rs` (M8a's), appended variants and fields:

```rust
pub enum AgentRole { Orchestrator, Worker, Reviewer, Scout, Planner, Decider }   // Scout is M8b's; "planner"; "decider" (M9.2 review ruling 2: named only by role-routing records, never given anthrex tools)
pub enum RunState  { /* M8a's eight */ Planning }                       // "planning"; label() "planning"; not terminal
pub enum TaskState { /* M8a's eleven */ Reported }                      // "reported"; label() "reported"; is_finished() true
pub enum BlockReason { /* M8a's six */ MessagePause }                   // "message_pause"; shown as paused(message) (decision 42c)
pub struct PlanTask { /* M8a's fields */ #[serde(default)] pub review_target: Option<String> }   // decision 24
// PlanEdit::AmendTask gains, last:  #[serde(default)] deps: Option<Vec<String>>                 // decision 25
pub enum PlanEdit { /* M8a's nine */
    Message { to: MessageTarget, text: String, kind: MessageKind },     // "message", decision 42a
    Refresh { task_id: String },                                         // "refresh", decision 42e
}
pub enum MessageTarget { Tasks(Vec<String>), Stage(u32), Running }       // custom serde: ["t1","t2"] | "stage:<n>" | "running"
#[serde(rename_all = "snake_case")] #[derive(Copy, Eq)]
pub enum MessageKind { Info, Change, StopAndWait }                       // "info" | "change" | "stop_and_wait"
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
#[derive(Eq)]
pub struct IntegrationInfo { pub epic: String, pub state: IntegrationState, pub base: Option<String>,
                             pub merges: Vec<String>, pub tasks: Vec<String> }            // tasks: the <e>-int<n> review tasks
#[serde(rename_all = "snake_case")] #[derive(Copy, Eq)]
pub enum TaskNoteKind { Discovery, Risk, Progress }                                          // decision 42f
#[derive(Eq)]
pub struct TaskNoteInfo { pub task_id: String, pub kind: TaskNoteKind, pub text: String, pub at: u64 }
```

`crates/proto/src/run_info.rs`, new fields, each `#[serde(default)]`:

```rust
// RunInfo
pub orchestrator: Option<OrchestratorInfo>, pub holds: Vec<HoldInfo>, pub integration: Vec<IntegrationInfo>,
pub digest_revision: u64, pub research_report: Option<PathBuf>,
// RunInfo.planners: Vec<PlannerInfo> (M8c, run_info.rs:305) is filled by this milestone (decision 33)
// RunInfo.scouts (M8b, :293) is overlaid by the driver from ScoutService::run_scouts (decision 20)
// PlannerInfo (M8c, proto/src/planner.rs): pub note: Option<String>
// PlanEditInfo (M8c, :222): pub source: String, pub accepted: bool (default true), pub error: Option<String>, pub recipients: Vec<String>
// TaskInfo
pub hold: Option<String>, pub review_target: Option<String>, pub research_bytes: Option<u32>,
pub message_count: u32, pub last_message_kind: Option<MessageKind>, pub last_message_line: Option<String>,
pub task_notes: Vec<TaskNoteInfo>,                  // the task's last 10 (decisions 16a, 42d)
// TaskInfo.brief, acceptance, route_spec (M8c) are filled only at the gate (decision 16a)
```

`crates/proto/src/run_wire.rs` and `messages.rs`:

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
// run message and run refresh send M8a's RunRequest::Edit with one edit (decision 42h): no new request.
// Every RunReply that answers a RunRequest gains, last:  (M9.2 review ruling 1)
//     #[serde(default)] request_id: Option<u64>          // decision 2: echoes a RunTagged id; None for ClientMsg::Run
//   Started, Done, Refused, ConfirmNeeded, ToolResult and Triaged carry it as their last field;
//   Profile and Stats become struct variants to carry it: Profile { reply: Box<ProfileReply>, request_id },
//   Stats { stats: HistoryStats, request_id }. Snapshot, which a subscription also pushes unasked, carries none.
//   RunReply::tagged(id) stamps every one of them.
// messages.rs: ClientMsg gains, last:  RunTagged { id: u64, request: RunRequest }
```

`lib.rs` re-exports the new types by name (never by glob) and sets `PROTO_VERSION` to 10, with a derivation paragraph in its doc comment in the M8a–M8c style.

`crates/proto/src/history.rs` (decision 43):

```rust
pub const HISTORY_VERSION: u32 = 2;                    // was 1 (:17); version-1 lines still decode
pub enum HistoryLine { Task(TaskRecord), Run(RunRecord), Revert(RevertRecord), RoleRoute(RoleRoutingDecision) }  // tag "role_route"
#[derive(Eq)]
pub struct RoleRoutingDecision {
    pub v: u32, pub record_id: String, pub at: u64,
    #[serde(default)] pub run_id: Option<String>, #[serde(default)] pub task_id: Option<String>,
    pub role: AgentRole, pub session_id: String,
    pub trigger: String,          // "start" | "restart" | "replan" | "retry" | "triage" | "size_check" | …
    pub source: String,           // "explicit_choice" | "agent_config" | "planner_config" | "roster_default" | "decider_config"
    pub policy_version: String,   // "m9-orchestrator-v1" | "m9-planner-v1" | "m9-scout-v1" | "m9-decider-v1"
    #[serde(default)] pub pick_policy: Option<String>,   // None until milestone 9.5's role lists apply
    pub input: RoleRoutingInput, pub chosen: Route, pub selected_index: u32,
    pub candidates: Vec<RoutingCandidate>,               // M8b's type, the full ordered snapshot; candidates[selected_index].route == chosen
    #[serde(default)] pub outcome: Option<RoleOutcome>,  // None until the session ends
    #[serde(default)] pub result: Option<String>,        // the role's accepted/rejected, report or submission status
}
#[derive(Eq)]
pub struct RoleRoutingInput { #[serde(default)] pub run_path: Option<RunPath>, #[serde(default)] pub goal: Option<String>,
                              #[serde(default)] pub languages: Vec<String>, #[serde(default)] pub epic: Option<String>,
                              #[serde(default)] pub area: Vec<String>, #[serde(default)] pub question_kind: Option<String> }
#[serde(rename_all = "snake_case")] #[derive(Copy, Eq)]
pub enum RoleOutcome { Completed, Failed, Interrupted, Fallback }
```

Only the completed or recovered record is appended to `<repo_dir>/history.jsonl`. Pre-run triage has no `run_id`. Existing `TaskRecord.routing_decisions` remain task-bound. Every exhaustive match on `HistoryLine` gains an arm: `daemon/src/run/history_io.rs:119-121` (the record id), `daemon/src/run/engine/history.rs:33-35`, `daemon/src/run/stats.rs:90-92` (ignored for the aggregates). M8b's `assert_eq!(HISTORY_VERSION, 1)` (`proto/src/adapt_tests.rs:208`) becomes 2.

### `config` (`crates/config/src/orchestrator/agent.rs`, new; one call from `orchestrator::read`)

```rust
pub struct AgentSettings {                    // config::Orchestrator gains exactly one field: pub agent: AgentSettings
    pub planner_task_cap: u32,                // [orchestrator] planner_task_cap = 12, 2..=50
    pub max_scouts: u32,                      // [orchestrator] max_scouts = 12, 1..=50 (per run)
    pub wake_orchestrator: bool,              // [orchestrator] wake_orchestrator = true
    pub wake_quiet_secs: u64,                 // [orchestrator] wake_quiet_secs = 5, 1..=120
    pub message_max_per_turn: u32,            // [orchestrator] message_max_per_turn = 3, 0..=20; zero turns messages off
    pub note_max_per_task: u32,               // [orchestrator] note_max_per_task = 10, 0..=100; zero turns notes off
    pub agent: AgentConfig,                   // [orchestrator.agent]
    pub planners: PlannerConfig,              // [orchestrator.planners]
}
pub struct AgentConfig {
    pub runtime: Option<proto::Runtime>,      // None = orchestrator.default_runtime
    pub model: String,                        // "" = decision 6's resolution; must be in the roster at run start
    pub effort: proto::Effort,                // "high"
}
pub struct PlannerConfig {
    pub runtime: Option<proto::Runtime>,      // None = the orchestrator's runtime
    pub strength: proto::Strength,            // "frontier"
    pub effort: proto::Effort,                // "high"
    pub max_tool_calls: u32,                  // 200, 20..=2000
    pub timeout_secs: u64,                    // 2400, 120..=14400
    pub max_rejections: u32,                  // 5, 1..=20
}
pub(crate) fn read(table: &toml::Table, problems: &mut Vec<Problem>) -> AgentSettings;
```

Messages follow M8a's format, for example `orchestrator.planner_task_cap: must be between 2 and 50 (using 12)`, `orchestrator.message_max_per_turn: must be between 0 and 20 (using 3)`, `orchestrator.agent.effort: must be low, medium or high (using high)`, and `unknown key, ignored` under the two new tables (`orchestrator/unknown.rs`). A non-empty `agent.model` that is not in the merged roster for the resolved runtime is not a config problem; `run start --goal` refuses with decision 6's text.

`RunLimits` (M8a, `run/model.rs:125`) gains one field, `#[serde(default)] pub orch: OrchLimits` (its type in `run/orch/mod.rs`), holding `planner_task_cap`, `max_scouts`, `wake_orchestrator`, `wake_quiet_secs`, `message_max_per_turn`, `note_max_per_task` and `planners: PlannerLimits { runtime, strength, effort, max_tool_calls, timeout_secs, max_rejections }` (proto types: the config crate has no serde, so `config::PlannerConfig` cannot be persisted), each defaulting to the config default. It is filled from `AgentSettings` when the run is built (`run/plan.rs`), so the message and note limits are frozen at run start and a later config edit cannot change the rules of a live run.

### `daemon`

```rust
// run/orch/mod.rs (pure). Everything derives Debug, Clone, PartialEq, Serialize, Deserialize, Default where noted.
pub enum EditSource { User, Orchestrator, Planner { epic: String } }   // label(): "user" | "orchestrator" | "planner:<e>"
#[derive(Default)]
pub struct RunOrch {                                // Run gains: #[serde(default)] pub orch: RunOrch
    pub orchestrator: Option<OrchestratorRecord>, pub epics: Vec<EpicRecord>, pub gate_holds: Vec<GateHoldRecord>,
    pub run_scouts: Vec<RunScout>, pub digest_rev: u64, pub digest_fp: u64,
    pub installed: BTreeMap<String, bool>, pub yes: bool, pub research_report: Option<PathBuf>,
    pub planner_usage: TokenUsage,
}
// run/model.rs: Run also gains  #[serde(default)] pub role_routing_decisions: Vec<RoleRoutingDecision>  (decision 43)
#[derive(Default)]
pub struct TaskOrch {                               // Task gains: #[serde(default)] pub orch: TaskOrch
    pub gate_hold: Option<String>, pub research: Option<ScoutReportArgs>, pub review_range: Option<(String, String)>,
    pub integration_of: Option<String>,             // decision 37: the epic an engine-made review task reviews
    pub messages: Vec<TaskMessage>, pub worker_notes: Vec<WorkerNote>,             // decision 42d (M9.13a)
    pub refresh: Option<RefreshState>, pub refresh_merges: Vec<String>,            // decision 42e (M9.13a)
}
pub struct OrchLimits { /* Interfaces "config" */ }
pub struct OrchestratorRecord {
    pub route: Route, pub window_id: Option<u32>, pub launch_op: Option<OpId>, pub live: bool,
    pub started_at: u64, pub exited_at: Option<u64>, pub first_prompt: String,
    pub plan_submitted: bool, pub summary: Option<String>,             // the finish edit is M8a's Run.finish_edit
    pub notes: Vec<String>, pub last_wake_rev: u64, pub wakes: u32,
    pub otlp_token: String,                                            // decision 14a; never in the snapshot or digest
    pub session: u32,                                                  // decision 43: 1 at launch, +1 per restart
}
pub enum PlannerPhase { Queued, Planning, Finished, Failed { reason: String } }
pub struct EpicRecord {
    pub epic: String, pub title: String, pub area: Vec<String>, pub brief: String, pub scout_refs: Vec<String>,
    pub route: Route, pub phase: PlannerPhase, pub request: String,       // the live or last session's brief
    pub sessions: Vec<PlannerSession>,                                     // one per planner session
    pub started_at: u64, pub ended_at: Option<u64>,
    pub edits_accepted: u32, pub edits_rejected: u32, pub last_rejection: Option<String>,
    pub replans: Vec<String>, pub note: Option<String>, pub gate_hold: Option<String>,
    pub base: Option<String>, pub merges: Vec<(String, String)>,           // (task id, merge commit)
    pub integration_state: IntegrationState, pub integration_rounds: u32,
}
pub struct PlannerSession { pub session: u32, pub window_id: Option<u32>, pub op: Option<OpId>,
                            pub started_at: u64, pub ended_at: Option<u64>, pub usage: TokenUsage }
pub struct GateHoldRecord { pub id: String, pub kind: HoldKind, pub state: HoldState, pub tasks: Vec<String>,
                            pub created_at: u64, pub decided_at: Option<u64>, pub decided_by: Option<String> }
pub enum RunScoutState { Queued, Running, Reported, Failed { reason: String } }
pub struct RunScout { pub id: String, pub question: String, pub area: Vec<String>, pub web: bool,
                      pub state: RunScoutState, pub queued_at: u64, pub started_at: Option<u64>,
                      pub ended_at: Option<u64>, pub window_id: Option<u32> }
pub struct TaskMessage { pub at: u64, pub source: EditSource, pub kind: MessageKind, pub text: String, pub delivered: bool }
pub struct WorkerNote { pub at: u64, pub kind: TaskNoteKind, pub text: String }
pub enum RefreshState { Due, InFlight(OpId) }
pub fn make_planned(run: &mut Run, triage: TriageInfo, route: Route, yes: bool, installed: BTreeMap<String, bool>);
    // decision 26: called by the driver on the Run that RunService::build_plan built (BuildContext.yes = false)
// run/model.rs: Run gains `orch` and `role_routing_decisions`, Task gains `orch`, RunLimits gains `orch` (with doc lines).

// run/orch/roles.rs (pure): decision 43
pub fn record(run: Option<&Run>, role: AgentRole, session_id: &str, trigger: &str, source: &str, policy: &str,
              input: RoleRoutingInput, candidates: Vec<RoutingCandidate>, chosen: &Route, now: u64) -> RoleRoutingDecision;
    // appends `chosen` to the candidates when absent; selected_index points at it
pub fn finish(decision: &mut RoleRoutingDecision, outcome: RoleOutcome, result: Option<String>);

// run/edits_orch.rs (pure; run/edits.rs's apply_edits delegates Message, Refresh and AmendTask.deps here)
pub(crate) fn one_edit_rule(edits: &[PlanEdit], submit: bool, summary: bool) -> Result<(), PlanError>;   // decision 42
pub(crate) fn apply_message(run: &mut Run, to: &MessageTarget, text: &str, kind: MessageKind, source: &EditSource, now: u64)
    -> Result<MessageOutcome, PlanError>;                           // decision 42b; MessageOutcome { delivered, refused }
pub(crate) fn apply_refresh(run: &mut Run, task_id: &str) -> Result<(), PlanError>;   // decision 42e (the pure half)
pub(crate) fn apply_amend_deps(run: &mut Run, task_id: &str, deps: &[String]) -> Result<(), PlanError>;   // decision 25
// EditConsequence (M8a) gains Message { task_id, text, kind } beside Deliver.

// run/orch/tools.rs (pure)
pub enum OrchCall {
    GetContext { scouts: Option<Vec<String>> },
    SpawnScout { id: String, question: String, area: Vec<String>, web: bool },
    SpawnSubplanner { epic: String, title: String, area: Vec<String>, brief: String, scout_refs: Vec<String> },
    EditPlan { edits: Vec<PlanEdit>, submit: bool, summary: Option<String> },
    RunStatus { since: Option<u64>, wait_secs: u64 },
    TaskResult { task_id: String },
    SubmitEpic { edits: Vec<PlanEdit>, note: Option<String> },
    TaskNote { kind: TaskNoteKind, text: String },                     // a worker's, decision 42f
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
pub struct Resolved { pub route: Route, pub source: String, pub candidates: Vec<RoutingCandidate> }   // decision 43's snapshot
pub fn resolve_orchestrator(choice: Option<&OrchestratorChoice>, agent: &config::AgentConfig,
                            default_runtime: Runtime, roster: &[ModelEntry]) -> Result<Resolved, String>;   // decision 6
pub fn orchestrator_role(run: &Run, route: &Route) -> RoleLaunch;                   // env and remove_env filled by the driver
pub fn orchestrator_window_spec(run: &Run, route: &Route, first_prompt: &str) -> WindowSpec;
    // name "<h4>/orchestrator", runtime, cwd = run.root, worktree_branch None, model (None when ""), initial_prompt
pub fn planner_spec(run: &Run, epic: &EpicRecord, session: u32) -> PlannerSpec;    // decision 31
pub fn research_spec(run: &Run, task: &Task) -> HeadlessSpec;                       // decision 35
pub fn review_task_spec(run: &Run, task: &Task, route: &Route) -> HeadlessSpec;     // decisions 36, 37
pub fn scout_spec(run: &Run, scout: &RunScout, root: &Path, project: &Path) -> ScoutSpec;        // decision 20

// run/orch/contract.rs (pure): the contracts and every text of Interfaces "Prompts and messages", plus
pub fn scout_extract(reports: &[(String, ScoutReport)]) -> String;                  // decision 34, at most 12 KiB
pub fn notes_section(messages: &[TaskMessage]) -> String;          // decision 42d

// run/engine (additions)
pub enum EventKind { /* M8a's, M8b's */ Orch(OrchEvent) }                            // one variant (engine/mod.rs)
pub enum OrchEvent {                                                                 // engine/orch.rs
    Tool { reply: ReplyId, call: ToolCall, refusals: Vec<(Runtime, String)> },       // decision 15
    ApproveHold { reply: ReplyId, run_id: String, hold: String },
    RejectHold { reply: ReplyId, run_id: String, hold: String },
    ScoutEnded { run_id: String, scout_id: String, outcome: ScoutEnd, usage: TokenUsage },
    PlannerEnded { run_id: String, epic: String, session: u32, outcome: ScoutEnd, usage: TokenUsage },
    OrchestratorWindow { run_id: String, window_id: u32, live: bool },
    OrchestratorWoken { run_id: String, digest_revision: u64 },
    DigestRead { run_id: String, digest_revision: u64 },
    RoleRoute { run_id: String, decision: RoleRoutingDecision },                     // decision 43, a run-bound decider
    RoleRouteEnded { run_id: String, record_id: String, outcome: RoleOutcome, result: Option<String> },
}
// M8b's EventKind::Promote { reply, run_id } gains  route: Option<Route>, installed: BTreeMap<String, bool>  (None: refused before the event)
// M8a's EventKind::Start is reused for a Planning run (decision 26); EventKind::Edit carries user message/refresh edits.
pub enum ScoutEnd { Reported, Failed { reason: String } }
pub enum OpKind { /* M8a's, M8b's */
    CreateOrchestrator { spec: WindowSpec, role: RoleLaunch, project: PathBuf },   // → OpResult::Window { window_id }
    RestartOrchestrator { window_id: u32 },                                          // → OpResult::Restarted
    StartScout { spec: ScoutSpec },                                                  // → OpResult::ScoutStarted { window_id }
    StartPlanner { spec: PlannerSpec },                                              // → OpResult::PlannerStarted { window_id }
    ResolveTarget { root: PathBuf, target: String, base_branch: String },            // → OpResult::Target { base, head }
}
// M8a's OpKind::HandBack gains  #[serde(default)] list_merged: bool;  OpResult::HandedBack gains  #[serde(default)] merged: Vec<String>
// M8b's OpKind::AppendHistory carries the HistoryLine::RoleRoute lines (decision 43)
pub enum OpResult { /* M8a's, M8b's */ Restarted, ScoutStarted { window_id: u32 }, PlannerStarted { window_id: u32 },
                    Target { base: String, head: String } }
pub enum Effect { /* M8a's */
    WakeOrchestrator { run_id: String, window_id: u32, text: String, digest_revision: u64 },
    PlannerAccepted { window_id: u32 },
    StopPlanner { window_id: u32, reason: String },
}
// run/engine/{orch,gate_holds,promote,planners,kinds,worker_messages}.rs (pure) — entry points called from engine/mod.rs:
pub fn on_orch_event(state: &mut EngineState, event: OrchEvent, now: u64) -> Vec<Effect>;
pub fn after_step(run: &mut Run, now: u64) -> Vec<Effect>;    // digest fingerprint, wake effect, completion extension
pub fn worker_messages::refreshed(run: &mut Run, i: usize, result: &OpResult, now: u64) -> Vec<Effect>;   // decision 42e

// run/git/summary.rs (blocking; run_git; reads, no write queue)
pub fn task_summary(git: &OsStr, root: &Path, start: &str, branch: &str, timeout: Duration) -> Result<TaskGit, String>;
pub fn resolve_target(git: &OsStr, root: &Path, target: &str, base_branch: &str, timeout: Duration)
    -> Result<(String, String), String>;                                           // (base sha, head sha)

// run/driver/refresh.rs (blocking work on spawn_blocking)
pub fn dirty(git: &OsStr, worktree: &Path, timeout: Duration) -> Result<bool, String>;   // decision 42e: tracked changes
pub fn merged_since(git: &OsStr, worktree: &Path, onto: &str, run_head: &str, timeout: Duration) -> Result<Vec<String>, String>;

// run/driver/orch.rs
impl RunService {
    pub(crate) async fn orch_tool(&self, call: ToolCall) -> RunReply;    // called from driver/adapt.rs::tool
}
// run/driver/wake.rs
pub const SUBMIT_DELAY: Duration = Duration::from_millis(200);
pub const WAKE_MAX_BYTES: usize = 2 * 1024;
pub fn encode_paste(text: &str) -> Vec<u8>;                                     // pure helper, tested here
pub async fn deliver_wake(manager: &WindowManager, window_id: u32, text: &str) -> anyhow::Result<()>;

// scout/planner.rs
pub struct PlannerSpec { pub run_id: String, pub epic: String, pub session: u32, pub headless: HeadlessSpec,
                         pub first_turn: String, pub project: PathBuf, pub cwd: PathBuf, pub route: Route }
impl ScoutService {
    pub async fn start_planner(self: &Arc<Self>, spec: PlannerSpec) -> anyhow::Result<ScoutHandle>;
    pub fn accept_planner(&self, window_id: u32);             // the machine's ReportAccepted
    pub fn stop_planner(&self, window_id: u32, reason: &str); // the machine's Stop, with the engine's reason
}
// scout/machine.rs: ScoutLimits gains  texts: MachineTexts { nudge, wrap_up, submit_tool, noun }  (scouts keep M8b's texts)

// launch/role.rs (pure)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoleLaunch {
    pub run_ref: RunRef, pub mcp: McpTarget, pub instructions: String, pub effort: Effort,
    pub claude_allowed_tools: Vec<String>, pub claude_disallowed_tools: Vec<String>,
    pub env: Vec<(String, String)>,                     // decision 10: OTLP, token, MCP_TOOL_TIMEOUT, then ENABLE_TOOL_SEARCH=false last (Claude)
    #[serde(default)] pub remove_env: Vec<String>,      // decision 10: credential_scrub_for(runtime, auth)
}
pub const ORCHESTRATOR_ALLOWED_TOOLS: &[&str] = &["mcp__anthrex__get_context", "mcp__anthrex__spawn_scout",
    "mcp__anthrex__spawn_subplanner", "mcp__anthrex__edit_plan", "mcp__anthrex__run_status",
    "mcp__anthrex__task_result", "Read", "Glob", "Grep"];
pub const ORCHESTRATOR_DISALLOWED_TOOLS: &[&str] = &["Edit", "Write", "NotebookEdit", "Bash", "Agent"];
pub fn claude_role_args(role: &RoleLaunch, ctx: &LaunchContext<'_>, caps: &CliCaps) -> Vec<String>;   // decision 7's middle block
pub fn codex_role_args(role: &RoleLaunch, ctx: &LaunchContext<'_>, caps: &CliCaps) -> Vec<String>;    // decision 8's block
// launch/mod.rs: LaunchContext gains  pub role: Option<&'a RoleLaunch>;  LaunchPlan gains  pub scrub_agent_env: bool, pub remove_env: Vec<String>
//                (every existing construction site passes role: None; plan() sets scrub_agent_env = role.is_some())
// launch/claude.rs: HOOK_EVENTS: [&str; 11], StopFailure inserted in alphabetical order after Stop
// hooks.rs: HookKind::StopFailure (last); parse "StopFailure"; to_status_event → Some(StatusEvent::Stop)

// manager/role_window.rs
impl WindowManager {
    pub async fn create_run_window(&self, spec: WindowSpec, project: PathBuf, role: RoleLaunch) -> anyhow::Result<WindowInfo>;
    pub fn set_run_window_live(&self, id: u32, live: bool);    // decision 11; under the lock, no I/O
    pub fn run_window_live(&self, id: u32) -> Option<RunRef>;  // Some while role.is_some() && run_live; read by the guard
    pub fn note_client_input(&self, id: u32);                   // server.rs, on ClientMsg::Input; under the lock, no I/O
    pub fn last_client_input(&self, id: u32) -> Option<Instant>;
}
// manager/entry.rs: Entry gains  role: Option<RoleLaunch>, run_live: bool, last_client_input: Option<Instant>;
//                   Entry::info sets WindowInfo.run from role.run_ref for a Pty entry
// manager/create.rs: admit, insert and spawn_window become pub(super); spawn_window and insert take role: Option<RoleLaunch>
// manager/restart.rs: ForRelaunch gains role: Option<RoleLaunch>
// manager/restore.rs + state snapshot: WindowRecord.run = {"role_launch": RoleLaunch} for a Pty entry with a role;
//                   headless_spec: an unparseable Headless record restores as an exited headless window (decision 11a)
// server/headless_guard.rs: Kill and Remove also refused when manager.run_window_live(id) is Some (decision 11)
// server/run_api.rs: a ClientMsg::RunTagged request is handled as ClientMsg::Run, and every reply but Snapshot carries request_id (decision 2; M9.2 review fixes). List, Subscribe and Unsubscribe are never sent tagged: they are answered by a Snapshot or not at all
// headless/mod.rs: McpTarget gains #[serde(default)] epic: Option<String>;
// headless/argv.rs: mcp_args gains the Planner arm and passes --epic <e> after --scout and before --window
// window.rs: Window::spawn removes SCRUB_PREFIXES/SCRUB_NAMES variables and plan.remove_env when plan.scrub_agent_env
// run/role_launch.rs: WORKER_MCP_TOOLS: [&str; 3] with "mcp__anthrex__task_note" (decision 42f)
// run/globs.rs: ProtectedMatcher::matches and may_cover_protected treat a non-ASCII path as protected (decision 23a)
// run/reach.rs: reachable_runtimes adds the orchestrator's and planners' runtimes (decision 26)
// run/snapshot.rs: decision 16a's gate-only plan text; the §12 TaskInfo fields; the paused attention line
// run/edit_log.rs: PlanEditRecord's four fields; record(run, edits, now, source, outcome); describe_one's two arms (decision 40)
// run/engine/signals.rs: add_usage's four field sums become TokenUsage's saturating += (followups file, "From M8b.15")
// metering/server.rs: UsageSink gains token(run_id) and live_orchestrators(); OTLP_BASE_CONNECTIONS (decisions 14a, 14b)
```

**Reconcile rows** added to M8a's table:

| `OpKind` | Reality checked | Replay | Otherwise |
|----------|-----------------|--------|-----------|
| `CreateOrchestrator` | a restored window whose persisted `RoleLaunch.run_ref` has this run and role `Orchestrator` | `Window { window_id }` (dormant; `run resume` restarts it) | `NotStarted` |
| `RestartOrchestrator`, `ResolveTarget` | none (idempotent) | — | `NotStarted` |
| `StartScout`, `StartPlanner` | none; **nothing is killed** (decision 20) | — | `NotStarted`; the run scout or planner is marked failed on `Restore` (decisions 20, 32) |
| `HandBack` with `list_merged` (refresh) | M8a's `HandBack` row, unchanged | M8a's | M8a's |
| `AppendHistory` of a `RoleRoute` line | M8b's row: `history_io::contains_record(record_id)` | done | appended again (idempotent) |

### MCP (`crates/mcp`)

`McpOptions` gains `epic: Option<String>`. `anthrex mcp` (flags in `crates/cli/src/mcp_cmd.rs`, not `main.rs`) accepts `--role orchestrator` and `--role planner`, and `--epic <e>` (required for `planner`, refused for every other role). `--run` is required for `planner` too. `--role scout` takes **exactly one** of `--scout <id>` and `--task <t>` (decision 35); neither or both is a usage error naming the two flags. New schemas live in `crates/mcp/src/tools_orch.rs`, beside M8b's `tools_scout.rs`. Every schema is a closed object at every level (`additionalProperties: false`).

| Role | Tool | Description | Properties (required in bold) |
|------|------|-------------|-------------------------------|
| orchestrator, planner | `get_context` | `Read the run's context: the repository profile, the models you can route to, the limits, scout reports, epics and the plan so far.` | `scouts` array ≤ 50 of string 1–48 |
| orchestrator | `spawn_scout` | `Start a read-only scout on one area with one question. Returns at once; its report appears in run_status and get_context.` | **`id`** string matching `^[a-z0-9][a-z0-9-]{0,31}$`; **`question`** string 1–2000; **`area`** array 1–20 of string 1–300; `web` boolean |
| orchestrator | `spawn_subplanner` | `Start a sub-planner for one epic with its own area, or a fresh one to re-plan an existing epic. Returns at once.` | **`epic`** string matching `^[a-z0-9][a-z0-9-]{0,10}$`; **`title`** string 1–80; **`area`** array 1–20 of string 1–300; **`brief`** string 1–8000; `scout_refs` array ≤ 20 of string 1–48 |
| orchestrator | `edit_plan` | `Apply plan edits as one batch. Set submit to open the plan gate. Add a summary for the user when the run is complete. Returns at once.` | **`edits`** array ≤ 60 of `plan_edit`; `submit` boolean; `summary` string 1–8000 |
| orchestrator | `run_status` | `Read the run digest. With since and wait_secs, wait up to wait_secs seconds (at most 50) for it to change.` | `since` integer ≥ 0; `wait_secs` integer 0–50 |
| orchestrator | `task_result` | `Read everything about one task: brief, commits, diff size, checks, proofs, reviews, agent rounds and any report.` | **`task_id`** string 1–16 |
| planner | `submit_epic` | `Submit your epic's tasks as one batch of plan edits. If it returns errors, fix them and call it again. When it is accepted you are done.` | **`edits`** array 1–60 of `plan_edit`; `note` string 1–2000 |
| worker | `task_note` | `Report a discovery, risk or progress without blocking the task.` | **`kind`** enum `discovery`, `risk`, `progress`; **`text`** string 1–4000 |

`plan_edit` is an object with **`op`** enum `add_task`, `split_task`, `cancel_task`, `amend_task`, `add_dep`, `answer`, `pause`, `resume`, `finish`, `message`, `refresh`, and optional `task` (`plan_task`), `task_id` string 1–16, `into` array 1–12 of `plan_task`, `brief` string 1–8000, `acceptance` array 1–20 of string 1–500, `route` (`route`), `test_mode` enum `tdd`, `check`, `none`, `test_mode_reason` string 1–300, `priority` integer, `size` enum `S`, `M`, `L`, `deps` array ≤ 20 of string 1–16, `dep` string 1–16, `text` string 1–8000, `to` (`oneOf`: an array 1–20 of string 1–16, or a string matching `^(running|stage:[0-9]{1,4})$`), `kind` enum `info`, `change`, `stop_and_wait`. Which keys each op needs is M8a's `PlanEdit` serde shape, checked by the daemon (`invalid arguments: edits[<i>]: <serde error>`); a `message`'s `text` is further limited to 4000 characters by the daemon (`message: text: at most 4000 characters`).

`plan_task` is an object with **`id`** string matching `^[a-z0-9][a-z0-9-]{0,15}$`; **`title`** string 1–120; `epic` string 1–11; `kind` enum `code`, `docs`, `research`, `review`; **`size`** enum `S`, `M`, `L`; `interface_change` boolean; `test_mode` enum; `test_mode_reason` string 1–300; **`owns`** array ≤ 20 of string 1–300; `deps` array ≤ 20 of string 1–16; `priority` integer; **`brief`** string 1–8000; **`acceptance`** array 1–20 of string 1–500; `test_to_write` string 1–300; `scout_refs` array ≤ 20 of string 1–48; `route` (`route`); `review_target` string 1–200. `budget` is deliberately absent (decision 23.1).

`route` is an object with `runtime` enum `claude`, `codex`; `model` string 0–100; `strength` enum `fast`, `standard`, `frontier`; `effort` enum `low`, `medium`, `high`.

`mcp::tools_for(Worker)` becomes `task_done`, `task_blocked`, `task_note`, so M8a's test `worker_tools_are_task_done_and_task_blocked` (`mcp/src/tools.rs:136`) becomes `worker_tools_are_task_done_task_blocked_and_task_note`.

Engine-side texts, each a `ToolResult { ok: false }` whose text is `{"error": "<text>"}`: M8a's `unknown run <id>`, `run <id> is paused; the user must resume it`, `run <id> is <state>`; `this window is not the orchestrator of run <id>`; `this window is not the sub-planner of epic <e> of run <id>`; `unknown task <id>`; `unknown epic <e>`; `scout <id> already exists in run <run>`; `run <id> already has <n> scouts, the most max_scouts allows`; `epic <e> is being planned by its sub-planner; wait for it to finish`; `epic <e>: area: overlaps epic <f>'s area (<glob>)`; `epic <e>: area: <glob> must be a literal path or end in /**`; `the plan has no tasks yet; add tasks before submitting`; `sub-planner <e> is still planning; submit when every sub-planner has finished`; `edits are not accepted on a complete run; only a summary is`; `op <op> is not available to a sub-planner`; `submit_epic was already accepted for epic <e>`; `task <id> is an integration review; the engine owns it`; `this task was asked to stop and wait; wait for the next message`; `note limit reached; put the rest in your task_done summary`; `invalid arguments: <field>: <problem>`. A rejected batch is decision 19's `{"accepted": false, "errors": […]}`, whose messages include decision 42's texts (`message and refresh must be the only edit in their call`, and those of decisions 42b, 42c, 42e). Successes are decisions 19–22's JSON, `Epic recorded. You are done; end your turn now.` for `submit_epic`, and `Note recorded. Keep working.` for `task_note`.

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
             "merging": 0, "blocked": 1, "paused": 0, "waiting": 0, "held": 0, "cancelled": 0},
  "tasks": [{"id": "t2", "title": "map hook events", "epic": "a", "kind": "code", "size": "M", "hub": false,
             "state": "blocked", "hold": null, "block": {"reason": "question", "text": "…"},
             "rung": 1, "deps": ["t0"], "route": "codex (default) high", "review": "r1 changes (1 critical)",
             "messages": {"count": 1, "undelivered": 0, "refresh": null},
             "last": "12:31 review r1 changes"}],
  "omitted_tasks": 0,
  "scouts": [{"id": "3f9a-daemon", "state": "reported", "question": "…", "failure": null}],
  "planners": [{"epic": "a", "state": "finished", "tasks": 3, "rejected": 1, "last_rejection": "…", "note": null}],
  "integration": [{"epic": "a", "state": "changes", "round": 1, "task": "a-int1", "findings": "1 critical, 0 important"}],
  "attention": ["t2 blocked (question): …"],
  "notes": ["t2 blocked (question): …"],
  "task_notes": [{"task": "t3", "kind": "discovery", "text": "…", "at": "12:29"}],
  "edits": [{"at": "11:40", "source": "user", "text": "cancel t4", "accepted": true, "error": null, "recipients": []}],
  "spend": {"tokens": 1800000, "tool_calls": 612}
}
```

- `gate.state`: `planning`, `awaiting_approval`, `approved` (with `at`, and `approved_by` in `run`), or `none` (a fast-path run before promotion). `holds` lists every hold not `Approved`, plus holds decided since the last `DigestRead`.
- A task's `block.text` and a scout's `question` are cut to 500 characters; `route` is `<runtime> <model or (default)> <effort>` as in `run status`; `review` is the last round with a verdict, or `null`; `last` is the newest history entry or `null`. A `paused(message)` task has `state: "blocked"` and `block.reason: "message_pause"`, and counts under `paused`, not `blocked`. `messages.refresh` is `null`, `"due"` or `"in_flight"`.
- `counts.held` counts tasks whose hold is not `Approved`.
- `task_notes`: the last 10 task notes across tasks, newest first, each `text` cut to 400 characters (decision 42f). Not named `notes`, which holds the wake notes.
- `edits` is the last 10 entries of M8c's `Run.plan_edits`, with decision 40's fields.
- **Trimming** past the cap, in order: finished tasks are dropped oldest first and counted in `omitted_tasks`; `edits` is cut to 3; `task_notes` to 3; `block.text` to 200 characters; `notes` to 5.
- `fingerprint` hashes this object with `now`, `spend`, `slots` and every task's `last` removed; the history line changes with every event, while the state that matters is carried by `state`, `block`, `rung`, `review` and `messages`. A new task note changes `task_notes`, so a `discovery` or `risk` note ends a waiting `run_status` (and so does a `progress` note, which adds no wake note).

### The context (`get_context`)

```json
{
  "run": {"id": "…", "goal": "…", "path": "large", "state": "planning", "root": "/…/repo", "base_branch": "main", "base_sha": "1a2b3c4…",
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

`you.epic` is `null` for the orchestrator. `run.base_sha` is there because readers read the user's checkout, not the base (decision 20a). The onboarding report, when the stored profile has one, is listed in `scouts` with id `onboarding`.

### The task result (`task_result`)

```json
{
  "task": {"id", "title", "epic", "kind", "size", "hub", "test_mode", "test_mode_reason", "owns", "deps", "implicit_deps",
           "route", "review_route", "state", "block", "rung", "failures", "bounces", "stalls", "hold",
           "brief", "acceptance", "test_to_write", "scout_refs", "review_target", "notes", "messages", "task_notes"},
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

(`task` lists its keys only; every value is the task's field as JSON.) `done` is `null` without a claim; `commits` and `diffstat` are absent when the task has no start commit or the git read failed (then `git: "<error>"`). Past the cap, `checks` keep their last 3, `rounds` their last 5, and `research.summary` is cut to 16 000 characters. `task.notes` is M8a's validation notes; `task.messages` and `task.task_notes` are decision 42d's records, read from the model (not the snapshot), with full texts.

### Contracts (exact)

Both constants contain no em dash, have no leading or trailing whitespace, and survive `launch::codex::toml_string` as a TOML string. Each names every anthrex tool by its Claude id at its first mention (decision 41, the tool-search fix's form).

```text
ORCHESTRATOR_CONTRACT:
You are the orchestrator of an anthrex run. The user gave a goal. You scout the repository, plan the work as small tasks, and steer the run until it finishes. anthrex's engine does the rest: it runs each task in its own git worktree with a headless worker, proves tdd tests, runs the check, has a different agent review the work, and merges approved work into the run branch. Nothing reaches the user's base branch until the user accepts the run.

You are the only agent the user talks to. Workers, reviewers, scouts and sub-planners are headless: the user watches them but cannot type to them, and neither can you.

What you may and may not do
1. You never edit, create or delete files, never run shell commands, and never commit. You may read files in this checkout. You never read task worktrees; call the anthrex tool task_result (in Claude: mcp__anthrex__task_result) instead.
2. You never approve a task and never merge. Reviewers and the engine approve, the engine merges, and only the user can override a rejection or accept the run. None of your tools does either. Never ask a worker to.
3. Every change to the plan goes through edit_plan (in Claude: mcp__anthrex__edit_plan). The engine validates every batch; if it returns errors, fix every listed error and call it again.

How a run goes
4. Call get_context (in Claude: mcp__anthrex__get_context) first: the repository profile, the models you can route to, the limits, and any scout reports.
5. Scout before you plan. Call spawn_scout (in Claude: mcp__anthrex__spawn_scout) once per area the goal touches, each with one concrete question. Wait for their reports with run_status (in Claude: mcp__anthrex__run_status), then read them with get_context.
6. Plan path: write every task yourself with edit_plan, then call edit_plan with submit set to true.
7. Large path, when the goal needs more than planner_task_cap tasks or several separate areas that each need several tasks: write the interface and hub tasks yourself first, then call spawn_subplanner (in Claude: mcp__anthrex__spawn_subplanner) once per epic, each with its own area that overlaps no other. Each sub-planner adds its epic's tasks and exits. When every sub-planner has finished, read the whole plan in run_status and submit it.
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

Messages to workers
28. A message informs; an amendment changes the task. If the task's scope changes, use amend_task: it changes a task's brief or acceptance criteria at any time, and its route, size or test mode before it starts; a change of owns is a cancel_task or split_task plus add_task. Send a message to task ids, or to running for every task with a live worker. info is context; change means the plan or the code around the task changed, and the worker will say how it applied it; stop_and_wait makes the worker finish its current step, commit, and wait until your next message to it. A message never interrupts a turn: it arrives when the worker's current turn ends. A message or refresh is always the only edit in its edit_plan call.
29. When merged work changes what a running worker builds on, call edit_plan with refresh for that task, then call edit_plan again with a change message to it. The worker gets the code and your explanation in one turn, because a message waits while its task's refresh is pending. refresh merges the run branch into the task's branch; it is refused while the task's worktree has uncommitted changes, so message the worker to commit first.
30. Workers report discoveries and risks with task_note. You see them in run_status as task_notes, and you are woken for them. Decide what to do: add a task, amend one, message the workers it affects, or nothing. Never pass one worker's summary to another; send the code with refresh and your own instruction.

Talking to the user
31. The user steers you by typing here, for example "skip X", "do Y first" or "use codex for Z". Turn each request into edit_plan edits, then say in one line what changed. If a request is unclear or would break a rule above, say so and ask.
32. After each run_status that changed something, write at most two lines here: what happened, and what you are waiting for.

Finishing
33. When run_status reports the run complete, call edit_plan with a summary for the user: what was done, what was not and why, every task that failed or is blocked, and what the user should check before accepting. The user accepts or discards the run; you never do.

PLANNER_CONTRACT:
You are a sub-planner in an anthrex run. The orchestrator gave you one epic: a goal for one area of the repository. You plan that epic as small tasks, submit them once, and stop. You never write code, and nobody can type to you.
1. You never edit, create or delete files, never run shell commands, and never commit. You may read files in this checkout.
2. Call the anthrex tool get_context (in Claude: mcp__anthrex__get_context) first: the profile, the models, the limits, the scout reports for your area, and the tasks already planned that you may depend on.
3. Every task you add owns paths only inside your area, and belongs to your epic.
4. Every task is S or M. S: one file, no interface change, a mechanical check exists, about 20 changed lines. M: one to three files inside one module, a clear spec, a check exists, about 100 changed lines. L is never executed: split it, interfaces first, one level only. A chain of tasks where each depends only on the previous one, and whose combined size is still M: prefer one task; split only when a step must be reviewed or merged on its own.
5. Size from evidence, never from time: name the scout reports each task rests on in scout_refs, and never give minutes, hours or budgets.
6. Depend on the orchestrator's interface and hub tasks where you use them. Never plan a change to a hub file. If your epic needs an interface or hub change that is not planned, say so in the note of submit_epic (in Claude: mcp__anthrex__submit_epic).
7. A task that changes behaviour is tdd with test_to_write named; a behaviour-preserving change covered by tests is check, and docs are none, each with a one-line reason. Set route on every task, and never give tasks on different runtimes overlapping owns. Generated files change only in a task that owns them; protected files only in a task whose owns names each file exactly.
8. At most planner_task_cap tasks.
9. Call submit_epic once with every edit. If it returns errors, fix every listed error and call it again. When it is accepted, end your turn: you are done.
10. Messages that start with [anthrex] come from anthrex. Do what they say.
```

`WORKER_CONTRACT` (M8a's, in `run/contract.rs` as the tool-search fix left it) gains two lines after its line 9, so it has 12 lines and ends with the second. Line 10 is TT §12.3's sentence, with the tool's Claude id added in the fix's form:

```text
10. If you learn something that affects other tasks or the plan, such as another place that must change, a wrong assumption in the brief, or a risk, report it with task_note (in Claude: mcp__anthrex__task_note) and keep working. Use task_blocked only when you cannot continue.
11. A message of kind change means the plan or the code around this task changed: in your next task_done summary, start with Changes applied: and say how you applied it. A message of kind stop_and_wait means finish your current step, commit anything worth keeping, and end your turn without calling task_done; wait for the next message.
```

### Prompts and messages (exact)

| Name | Text |
|------|------|
| `orchestrator_first_prompt(run)` | `[anthrex] You are the orchestrator of run <id> in <root>.` / `Goal: <goal>` / `Path: <plan \| large> (triage: <kinds joined with ,>/<scale>, <decider \| fallback: <reason>>: <triage reason>)` / `Plan gate: <the user approves your submitted plan in the run view \| off: the run was started with --yes, so your submitted plan starts at once>` / `Start with get_context, then scout, then plan.` |
| `promoted_first_prompt(run)` | `[anthrex] You are the orchestrator of run <id> in <root>, promoted from the fast path at the user's request.` / `Goal: <goal>` / `Its one task so far: t1 <title> (<state label>). It keeps running.` / `Start with get_context and run_status. Plan what else the goal needs; tasks you add wait for the user's approval once you submit them.` |
| `planner_prompt(run, epic, extract)` | `[anthrex] Plan epic <e> "<title>" of run <id>.` / `Run goal: <goal>` / `The run starts from <base branch>@<sha7>; you read the user's checkout at <root>, which may differ.` (decision 20a) / `Your area (every task you add must own paths only inside it):` / `- <glob>` per glob / `Tasks already planned that you may depend on:` / `- <id> <size> <title> (owns <globs joined with , >)` per task with no epic, or `- none` / `Task cap: at most <cap> tasks.` / decision 34's scout extract when non-empty / blank line / `What to plan:` / `<brief>` |
| `replan_prompt(run, epic, extract)` | `planner_prompt`'s text with its first line `[anthrex] Re-plan epic <e> "<title>" of run <id>.`, and before `What to plan:` the lines `The epic's current tasks:` / `- <id> <size> <state label> <title>` per task of the epic; `What to plan:` is followed by the new brief. |
| `PLANNER_NUDGE` | `[anthrex] Your turn ended without an accepted epic. Call submit_epic now with every edit, then stop.` |
| `planner_wrap_up(n)` | `[anthrex] You have used <n> tool calls. Stop reading and call submit_epic now with the tasks you have.` |
| `scout_first_turn(run, id, area, question)` | `[anthrex] Scout <id> for run <run id>.` / `Area: <globs joined with , >` / `Question: <question>` / `You read the user's checkout at <root>; the run starts from <base branch>@<sha7>, so a file the user has not committed may differ from what workers get.` (decision 20a) |
| `research_prompt(run, task)` | `[anthrex] Research task <id>: <title>` / `Run goal: <goal>` / `Answer this from the repository at <root>, and the web if you need it. Change nothing.` / `Your report must cover:` / `- <item>` per acceptance item / blank line / `<brief>` |
| `review_task_prompt(run, task, base, head)` | `[anthrex] Review task <id> "<title>" of <review_target>, level <small \| medium>.` / `Base: <sha7>` / `Head: <sha7>` / `Nothing will be merged; your findings go to the user.` / when the patch was clamped, `The diff below was cut at 16 KiB; read the rest with git diff <sha7>..<sha7>.` / `Acceptance criteria:` / `- <item>` per item / blank line / `<brief>` |
| `integration_review_prompt(run, epic, round, base, head)` | `[anthrex] Integration review of epic <e> "<title>", round <n>, level frontier.` / `Run goal: <goal>` / `Area: <globs joined with , >` / `Base: <sha7>` / `Head: <sha7>` / `The epic's change is these merges; read each with git diff <merge>^1 <merge>:` / `- <sha7> <task id> <task title>` per merge / `Judge whether the epic's tasks together do what the epic asked, and whether they fit each other and the code around them.` / `Earlier findings to confirm fixed:` lines when `round > 1` / blank line / `<epic brief>` |
| `wake_text(run_id, notes)` | `[anthrex] Run <id> changed: <notes joined with "; ">. Call run_status for the details.` (clamped to `WAKE_MAX_BYTES` by M8a's `messages::clamp` rule) |
| `planned_message(info, id, path)` | `triage: <kinds>/<scale> (<decider \| fallback: <reason>>)` / `<plan \| large> path: run <id> is being planned by its orchestrator` / `talk to it with: anthrex, then C-b T and Enter on the run` / `watch with: anthrex run status <id>` |
| `message_text(source, kind, text)` | `[anthrex] Message from the <orchestrator \| user> (<info \| change \| stop_and_wait>): <text>` (decision 42b; the TUI's §12.6 label keys on this prefix) |
| `notes_section(messages)` | `Notes from the orchestrator:` / `- <hh:mm> (<kind>, from <orchestrator \| user>) <text>` per recorded message, oldest first (decision 42d; `<hh:mm>` is UTC, as M8a's prompts write times) |
| `worker_messages_for_review(messages)` | `Messages the worker received:` / `- <hh:mm> (change) <text>` per `change` message (decision 42d, in `reviewer_prompt`) |
| `refresh_clean(n, list)` | `[anthrex] Your branch now includes the latest merged work (<n> commits: <sha7> <subject>, …). Rebuild before you continue.` (at most 10 listed, then `and <k> more`) |
| `refresh_conflict(files)` | `[anthrex] Merging the latest run branch into your worktree conflicted in: <files joined with , >. Resolve them, commit, and continue.` |
| `STOP_AND_WAIT_REFUSAL` | `this task was asked to stop and wait; wait for the next message` |
| `NOTE_RECORDED` / `NOTE_LIMIT` | `Note recorded. Keep working.` / `note limit reached; put the rest in your task_done summary` |
| `ONE_EDIT_RULE` | `message and refresh must be the only edit in their call` |

Wake notes are decision 39's texts, verbatim.

### CLI

```
anthrex run start (--plan <file> | --goal <text>) [--orchestrator <runtime>[:<model>]] [--yes] [--trust-project] [--unconfined-checks]
anthrex run promote <run> [--orchestrator <runtime>[:<model>]]
anthrex run approve <run> [--hold <hold>]
anthrex run reject <run> [--hold <hold>] [--confirm <run-id>]
anthrex run message <run> <task|stage:<n>|running> [--kind info|change|stop_and_wait] <text>
anthrex run refresh <run> <task>
anthrex run status [<run>] [--json]
anthrex run accept <run> [--yes]
anthrex mcp --role <orchestrator|planner|scout|worker|reviewer> --run <run> [--task <task>] [--epic <epic>] [--scout <id>] --window <id> [--socket <path>]   (hidden)
```

- `--orchestrator` is refused with `--plan` (`--orchestrator applies to --goal runs`). Its value parses as `claude`, `codex`, `claude:<model>` or `codex:<model>` (`codex:` means the default model); anything else exits 1 with `--orchestrator: expected claude or codex, optionally :<model>`.
- `run start --goal` on the plan or large path prints the run id on stdout and `planned_message` on stderr, and exits 0.
- `run approve <run> --hold <h>` sends `ApproveHold`; `run reject <run> --hold <h>` sends `RejectHold` and needs no confirmation (it cancels only held tasks that never started). Without `--hold`, both keep M8a's meaning; on a `running` run with an awaiting hold, `run approve <run>` without `--hold` exits 1 with `run <id> has holds waiting for approval: <ids>; pass --hold <id>`.
- `run message` and `run refresh` (decision 42h) send `RunRequest::Edit` with one `message` or `refresh` edit, use the same edit and journal path as the orchestrator, record source `user` in the edit log, and print M8a's `run edit` reply, then one line per refused recipient: `  <task>: <reason>`. `--kind` defaults to `info`; the text is the remaining arguments joined with one space. Their help names `info`, `change` and `stop_and_wait`. They never write into a headless agent's terminal.
- `run status` adds, per run: `planning` as a state; a line `  orchestrator: window <n>, <runtime> <model or (default)>, <live | exited>`; a line `  planners: <e> <state>[ (<n> tasks)], …` when there are epics; a line `  holds: <id> <state> (<n> tasks), …` when there are holds; a line `  summary: written` once the orchestrator wrote one. The task table's `STATE` shows `reported` and `paused(message)`, and a held task's state is followed by ` (held)`.
- `run accept` prints `research report: <path>` when `research_report` is set, and asks `nothing to merge; accept run <id> and remove its branches? [y/N]` instead of M8a's merge question when `run_head == base_sha`.
- `run promote`'s repeat reply is `run <id> was already marked for promotion`, with no time (decision 29).

### TUI goal form

`C-b g` is unused in M8c's prefix map and opens decision 44's form. The project root is taken from the selected project or the focused window's project; the form never guesses a different repository. The goal supports `Ctrl-J` for a newline, `Tab` changes fields, `Esc` cancels, and `Enter` sends `StartGoal` as a tagged request (decision 2). Optional runtime and model select the orchestrator; `trust_project` starts false and is an explicit toggle. An error leaves the filled form open. Success closes it and opens M8c's run view on the new run once a snapshot names it. This is a view over the existing goal request, not a second planning pipeline.

### File sizes this milestone must respect

AGENTS.md rule 8: about 600 lines. At the start (task M9.1 item 9), run `wc -l` on every file below and record the counts under "Implementation notes"; the budgets are growth over those counts. The "Today" column is `cc9dcb7` (on `c103308` where marked †), counted by the refresh.

| File | Today | Budget | Note |
|------|---:|---:|------|
| `crates/daemon/src/launch/mod.rs` | 432 | +25 | One `role` field, `scrub_agent_env`, `remove_env`, one call to `role::claude_role_args`; every flag is built in `launch/role.rs`. |
| `crates/daemon/src/launch/codex.rs` | 353 | +6 | One call to `role::codex_role_args` before `-m`. |
| `crates/daemon/src/launch/claude.rs` | 71 | +3 | `StopFailure`. |
| `crates/daemon/src/hooks.rs` | 399 | +8 | `StopFailure`. |
| `crates/daemon/src/manager/create.rs` | 563 | +15 | Visibility and the `role` parameter only; `create_run_window` is in `manager/role_window.rs`. |
| `crates/daemon/src/manager/restart.rs` | 556 | +8 | `ForRelaunch.role`. |
| `crates/daemon/src/manager/entry.rs` | 387 | +15 | Three fields, `info`'s `run`. |
| `crates/daemon/src/manager/restore.rs` | 554 | +25 | Parse and save `role_launch`; decision 11a's exited headless restore. |
| `crates/daemon/src/manager/headless.rs` | 526 | +5 | Decision 11a's placeholder spec, if it lands here. |
| `crates/daemon/src/window.rs` | 371 | +12 | The scrub and `remove_env`. |
| `crates/daemon/src/server.rs` | 570 | +3 | One `note_client_input` call; the refusals are in `server/headless_guard.rs` (35), the tagged-request handling in `server/run_api.rs`. |
| `crates/daemon/src/headless/argv.rs` | 450† | +8 | The `Planner` arm of `mcp_args`, `--epic`. |
| `crates/daemon/src/headless/mod.rs` | 291† | +4 | `McpTarget.epic`. |
| `crates/daemon/src/headless/conversation.rs` | 452 | +0 | Over M8c's 430 budget already. M9 does not touch it; if a task must, it first moves the turn-end handling into `headless/conversation_turn_end.rs` in its own commit. |
| `crates/daemon/src/scout/service.rs`, `scout/machine.rs`, `scout/spec.rs` | 521, 239, 174 | +40, +25, +10 | `start_planner`, `accept_planner`, `stop_planner` (bodies in `scout/planner.rs`); `MachineTexts`; nothing else. |
| `crates/daemon/src/run/engine/{mod,requests,dispatch,done,merge,outbox,restore,complete}.rs` | 569, 536, 589, 507, 518, 360, 422, 429 | +25, +20, +10, +20, +5, +30, +30, +30 | Calls into `engine/{orch,gate_holds,promote,planners,kinds,worker_messages}.rs`. `dispatch.rs` at 589 gets one call per new op kind at most; the reader-slot order lives in `engine/planners.rs`. `requests.rs` loses the promote body (decision 29). |
| `crates/daemon/src/run/{model,validate,edits,contract,snapshot,report,report_task,edit_log,reach}.rs` | 572, 514, 577, 584, 315, 255, 237, 83, 97 | +20, +25, +15, +10, +80, +50, +40, +40, +10 | New model types are in `run/orch/mod.rs`; `message`/`refresh`/`amend deps` in `run/edits_orch.rs`; `contract.rs` gains only the two worker contract lines and the extract and notes calls (texts in `run/orch/contract.rs`). |
| `crates/daemon/src/run/role_launch.rs` | 579† | +2 | `WORKER_MCP_TOOLS: [&str; 3]`. |
| `crates/daemon/src/run/reconcile/mod.rs` | 305 | +25 | The rows of Interfaces "Reconcile rows". |
| `crates/daemon/src/run/driver.rs`, `driver/ops.rs`, `driver/requests.rs`, `driver/adapt.rs`, `driver/adapt_goal.rs` | 591, 584, 588, 391, 283 | +8, +8, +8, +20, +30 | New code in `run/driver/{orch,orch_ops,refresh,wake}.rs`. |
| `crates/daemon/src/run/{history_io,stats,triage}.rs`, `run/engine/history.rs` | 395, 234, 367, 135 | +10, +3, +5, +3 | Decision 43's arms; `planner_task_cap` in the triage input. |
| `crates/daemon/src/metering/server.rs` | 311 | +40 | Decisions 14a and 14b. |
| `crates/daemon/src/decider/prompt.rs` | 430 | +5 | `planner_task_cap` in `TRIAGE_HEAD`. |
| `crates/mcp/src/tools.rs`, `lib.rs` | 258, 109 | +15, +15 | Schemas in `tools_orch.rs`. |
| `crates/proto/src/run.rs`, `run_info.rs`, `run_wire.rs`, `messages.rs`, `history.rs`, `planner.rs`, `lib.rs` | 453, 320, 211, —, 210, 33, 117 | +50, +40, +25, +5, +45, +3, +15 | New types in `orch.rs`; `MessageTarget`'s serde in `run.rs` (move it to `orch.rs` if `run.rs` would pass 500). |
| `crates/config/src/orchestrator.rs` | 585 | +8 | One `agent::read` call and one field; keys in `orchestrator/agent.rs`, known keys in `orchestrator/unknown.rs`. |
| `crates/cli/src/run_cmd.rs`, `run_cmd/status.rs`, `mcp_cmd.rs` | 501, 267, 65 | +20, +40, +25 | Bodies in `run_cmd/orch.rs`. |
| `crates/tui/src/app/runs.rs` | 524 | +40 | Holds' keys and `pending_open`; the goal form in `run_goal.rs`. |
| `crates/tui/src/{keymap,app/mod,theme,tree/runs,tree/run_rows,inspector/run,inspector/run_task,inspector/run_round,graph/run_text,graph/paint/style,ui/modal,app/headless}.rs` | 275, 499, 132, 198, 516, 349, 301, 228, 98, 260, 247, 38 | +3, +3, +15, +15, +15, +25, +25, +5, +10, +10, +5, +10 | Task M9.15. |
| `crates/tui/src/ui/conversation.rs` | 389 | +5 | One call to the label recogniser (decision 42i); `conversation.rs` (595) is not touched. |
| `crates/fake-agent/src/script.rs`, `main.rs`, `mcp.rs`, `roles.rs` | 432, 382, 198, 343 | +20, +10, +20, +25 | New steps' bodies in `src/orch_steps.rs`. `headless.rs` (576) and `stream_claude.rs` (581) are not touched. |
| `crates/cli/tests/support/run_harness.rs` | 542 | +10 | The new helpers live in `support/run_orch.rs`. |
| `scripts/pty-smoke.py` | 1799 | +3 | One import, one call; the stage lives in `scripts/pty_smoke_orch.py`. |

No new file may exceed 600 lines. `run/engine/orch.rs` is the one at risk: the holds are already in `gate_holds.rs` and promotion in `promote.rs`; if `orch.rs` still passes 450, wake notes move to `run/engine/wake_notes.rs`.

## Names taken from earlier briefs

Every name below is used exactly as the merged code defines it (checked by the refresh on `cc9dcb7`; M9.1 item 8 re-checks). If the code on the day M9 starts differs, the code wins: record the difference under "Implementation notes" in task M9.1 and use the merged name everywhere, without renaming anything in M8a, M8b or M8c.

| Name | Used for | Source |
|------|----------|--------|
| `run::engine::{step, EngineState, Event, EventKind::{Start, Approve, Reject, Edit, Retry, Override, Cancel, Resume, BaseAdvanced, Finish, Tool, Promote, OrchestratorUsage, OpDone, Signal, Delivered, Restore, Stop, Tick}, OpKind, OpResult, Effect, ReplyId}`, `OpId` | The reducer this milestone extends | M8a, M8b; `run/engine/mod.rs:93` |
| `OpKind::{CreateRunBranch, PrepareWorktree, CreateWindow, ResumeSession, PrepareReview, HandBack, AppendHistory, Decide}` (`run/engine/ops.rs`), `OpResult::{Window, Review, HandedBack, Failed, HistoryAppended}`, `Effect::{Reply, Op, Deliver, Interrupt, KillWindow, RetireWindow, RemoveWindow, Persist, WriteReport, Publish}` | Planning start, sessions, reviews, refresh, history | M8a, M8b |
| `build_run(plan, pre, ctx)` (`run/plan.rs:311`), `Preflight`, `BuildContext { …, yes }`, `resolve_task` (`validate.rs:72`), `validate_tasks` (`validate_graph.rs:43`), `EditScope::{Run, Area { globs }}`, `apply_edits` (`edits.rs:47`), `EditConsequence`, `PlanError { task, field, rule, message }`, `edits::{has_live_worker, state_label, not_started}` (private; `edits_orch.rs` gets `pub(super)` access) | Planned runs, edit batches, rule errors | M8a decisions 12–14 |
| `proto::{PlanTask, PlanEdit, TaskKind, Size, TestMode, Route, Strength, Effort, ModelEntry, Runtime, AgentRole, RunRef { run_id, task_id, role, session }, RunState, TaskState, BlockReason}` | Plan edits, routes, run references | M8a; `proto/src/run.rs` |
| `RunInfo`, `TaskInfo`, `RunsSnapshot`, `ReviewInfo`, `AgentRoundInfo`, `RunLimits` (`run/model.rs:125`) | Snapshot additions | M8a decision 47 |
| `RunRequest`, `RunReply::{Done, Refused, Snapshot, Started, ToolResult, Triaged}`, `ToolCall`, `request::{APPROVE, REJECT, EDIT, START_GOAL}` | Wire additions | M8a, M8b; `proto/src/run_wire.rs` |
| `Run.{revision, paused_from, trusted_project, base_sha, run_head, root, project, notes, outbox, windows_created, finish_edit, approved_at, plan_edits, plan_edits_since_approval}`, `Task.{reviews, history, failures, rung, notes, resolving, conflicts}`, `AgentRound`, `ReviewRecord`, `CheckRecord`, `ProofRecord` | Model the digest and `task_result` read | M8a, M8c; `run/model.rs` |
| `roster::{pick_reviewer, escalate}` (`run/roster.rs:100, :134`), `run::globs::{validate_glob, intersects, names_literally, ProtectedMatcher}` | Integration reviewer route; area checks; decision 23a | M8a decisions 9, 11, 23 |
| `WORKER_CONTRACT`, `REVIEWER_CONTRACT`, `worker_prompt` (`contract.rs:70`), `handover_prompt` (`:111`), `reviewer_prompt` (`:139`), `REVIEW_NUDGE`, `RESUME_REVIEWER`, `clamp_with`, `messages::{clamp, join_turn, MESSAGE_MAX_BYTES}` | Scout extract and notes in prompts; review tasks; wake text clamp | M8a decisions 29, 30; `run/contract.rs`, `run/messages.rs` |
| `headless::{HeadlessSpec, McpTarget, credential_scrub_for, session_vars, CLI_CAPS, CliCaps, session::{SCRUB_PREFIXES, SCRUB_NAMES}, argv::mcp_args}`, `WindowManager::create_headless`, `RETIRE_AFTER`, `server/headless_guard.rs::refuse` | Sub-planner, research and reviewer sessions; env scrub; kill/remove refusals | M8a decisions 24–28, 49, 52, 53; the tool-search fix |
| `config::reserved_env::CLAUDE_TOOL_SEARCH` | The orchestrator's tool search off (decision 10) | The tool-search fix |
| `CliCaps.{claude_user_settings_only, codex_user_config_only, codex_loads_project_config, claude_effort_flag}` | Orchestrator launch flags | M8a decision 53 |
| `TOOL_REPLY_TIMEOUT` (100 s, `mcp/src/lib.rs`), `DONE_CHECK_GIT_TIMEOUT` (10 s, `run/driver.rs:59`), `worktree::run_git` (`daemon/src/worktree.rs:234`) | Tool deadlines, git reads | M8a decisions 4, 18, 32 |
| `mcp::{tools_for, McpOptions}`, `anthrex mcp --role --run --task --scout --window --socket` (`cli/src/mcp_cmd.rs`) | Orchestrator and planner tools | M8a decision 4, M8b decision 15 |
| `RunService::request`, `RunContext`, the snapshot `watch`, `RunHarness::{new, script, plan, anthrex, start, subscribe, wait_run, restart_daemon, git, request}`, `RUN_WAIT`, `fake_agent_bin()` (`cli/tests/support/mod.rs:30`) | Driver, long-poll, tests | M8a "Shared test helpers" |
| `fake-agent` headless modes, per-role scripts `<git common dir>/fake-agent/<role>-<task>-<n>.jsonl` (`roles.rs`), steps `mcp_call`, `read_message`, `end_turn`, `sh`, `capture`, `hook`, `exit`, `hang`; `FAKE_AGENT_ARGS_FILE`, `FAKE_AGENT_STDIN_FILE`, `FAKE_AGENT_RESULT`, `FAKE_AGENT_MESSAGE` | Scripted agents | M8a task M8a.20 |
| `scripts/pty_smoke_run.py` (`run_engine_stage`), stage `11c`; `scripts/pty_smoke_adapt.py` (`adapt_stage`), stage `11d`; `scripts/pty_smoke_run_view.py` (`run_view_stage`), stage `11e` | Smoke stage ordering | M8a.25, M8b.15, M8c.10 |
| `proto::{RepoProfile, TriageInfo, RunPath::{Fast, Plan, Large}, Scale, DeciderSource, DiffStats, ScoutInfo, ScoutState}`, `AgentRole::Scout` | Triage, path, scouts | M8b Interfaces `proto` |
| `RunRequest::{StartGoal { goal, dir, yes, trust_project, unconfined_checks }, Promote}`, `RunReply::Triaged { triage, run_id, message }`, `Run.{path, triage, promote_requested_at, scout_reports, scout_usage, orchestrator_usage}` | Goal start, promotion, metering | M8b decisions 22, 25, 29 |
| `run::triage::{route, PLAN_SCALE_MAX}` (`triage.rs:23`, unused), `TriageRoute`, `decider::prompt::TRIAGE_HEAD` (`prompt.rs:27`) | Replacing the constant with the cap | M8b decisions 22, 23 |
| `driver/adapt_goal.rs::planned()` (`:244`) | The refusal decision 26 replaces | M8b decision 22 |
| `engine/requests.rs::promote` (`:174`), `model_adapt.rs::hh_mm` (`:33`) | Replaced by decision 29 | M8b decision 25 |
| `scout::{ScoutService::{start, tool, stop, info, run_scouts}, ScoutHandle, spec::{ScoutSpec, SUBMIT_TOOL, headless_spec, valid_id}, machine::{ScoutMachine, ScoutLimits}, report::{ScoutReportArgs, validate, resolve_ref, report_path, state_label}, contract::{SCOUT_CONTRACT, SCOUT_NUDGE}}`, `ScoutKind::{Onboarding, Area}` | Run scouts, sub-planners, research tasks, extracts | M8b decisions 12–15, 19 |
| `McpTarget.scout_id`, `ToolCall.scout_id`, `--role scout --scout <id>` | Scout MCP plumbing, extended with `--task` | M8b decision 15 |
| `profile::summary` (`profile/mod.rs:63`), `ProfileService`, `Effective`, the project-settings check of `StartGoal` | `get_context`, trust checks | M8b decisions 5–10, 22 |
| `metering::orchestrator_env(addr, run_id)`, `OTLP_MAX_CONNECTIONS`, `UsageSink`, `<data_dir>/otlp.addr` | OTLP for the orchestrator | M8b decision 30 |
| `proto::history::{HISTORY_VERSION, HistoryLine, RoutingCandidate, RoutingDecision}`, `run::history_io::{append_line, contains_record, read_history}`, `run::stats` | Decision 43 | M8b decision 33a |
| `RunHarness` M8b helpers: `with_deciders(mode, dir)`, `decider_calls`, `stored_profile`, `onboarding_report` (`cli/tests/support/run_adapt.rs`) | Triage in end-to-end tests | M8b "Shared test helpers" |
| `PlannerInfo { epic, title, area, route, window_id, state, started_at, ended_at, edits_accepted, edits_rejected, last_rejection, replans }`, `PlannerState { Planning, Finished, Failed }` (`proto/src/planner.rs`), `RunInfo.planners` (`run_info.rs:305`) | Filled here (a queued planner shows `Planning`) | M8c |
| `run::edit_log::{PlanEditRecord, PLAN_EDITS_KEPT, DESCRIBE_MAX_CHARS, describe, describe_one, record}`, `Run.approved_at`, `RunInfo.{approved_hhmm, plan_edits, plan_edits_since_approval}` | The edit log (decision 40); gate state in the digest | M8c decision 4 |
| The run view's node tree, `C-b T`, run-view keys, round labels (`tree/run_rows.rs::round_label`), glyphs (`theme.rs::task_glyph`), `Modal::EditTask`, `app/headless.rs::headless_control_refusal` | Task M9.15 changes | M8c decisions 12–19, 32 |

## Scenario map (spec §21, the M9 row; TT §12.7)

| Scenario | Test | Task |
|----------|------|------|
| A goal on the plan path: scouts, a plan, the gate approved through the run view, the verdict read by `run_status` | `e2e_plan_path_scouts_plans_and_reads_the_approval` | M9.16 |
| The plan gate rejected | `e2e_rejected_plan_discards_the_run_and_the_orchestrator_learns_it` | M9.16 |
| Steering by typing | `e2e_typed_steering_becomes_a_plan_edit` | M9.16 |
| A mis-sized task split by the orchestrator | `e2e_mis_sized_task_is_split_by_the_orchestrator` | M9.16 |
| A blocked question answered | `e2e_blocked_question_is_answered_after_a_wake` | M9.16 |
| `run promote` | `e2e_promote_starts_an_orchestrator_and_holds_its_tasks` | M9.16 |
| The orchestrator cannot approve or merge | `e2e_orchestrator_cannot_approve_merge_or_write` | M9.16 |
| TT §12.7: a message after an interface merge, with refresh, acknowledged; a task note; `stop_and_wait` | `e2e_message_refresh_and_task_note` | M9.16 |
| TT §12.7: messages are not delivered twice across a daemon restart | `e2e_messages_are_not_redelivered_after_a_restart` | M9.16 |
| Spec §16: a goal started from the TUI | `e2e_tui_goal_form_starts_the_plan_path` (smoke stage 11f on macOS; `goal_form_*` in M9.15) | M9.16, M9.17 |
| Spec §15: role routing records survive a restart | `e2e_role_routing_records_survive_restart` | M9.16 |
| A large goal with sub-planners | `e2e_large_path_two_subplanners_submit_their_epics` | M9.17 |
| A sub-planner edit outside its area rejected | `e2e_subplanner_edit_outside_its_area_is_rejected_then_fixed` | M9.17 |
| A new epic mid-run waits for approval | `e2e_new_epic_after_approval_is_held_until_approved` | M9.17 |
| Per-epic integration review | `e2e_integration_review_asks_for_changes_and_a_fix_task_closes_it` | M9.17 |
| A research goal | `e2e_research_goal_reports_without_merging` | M9.17 |
| A review goal | `e2e_review_goal_reviews_a_range_without_merging` | M9.17 |
| Daemon restart | `e2e_restart_resumes_the_orchestrator_with_its_role_flags` | M9.17 |
| OTLP metering of the orchestrator | `e2e_claude_orchestrator_gets_the_otlp_environment` | M9.17 |
| Spec §15: role history for the large path and pre-run triage | `e2e_role_history_for_large_and_triage_paths` | M9.17 |

## Tasks

Do them in this order: M9.1, M9.2, M9.3, M9.4, M9.5, M9.6, M9.7, M9.8, M9.9, M9.10, M9.11, M9.12, M9.13, M9.13a, M9.13b, M9.13c, M9.14, M9.15, M9.16, M9.17. Each task's tests are written first and must fail before its change (AGENTS.md rule 6), except those marked **pinning**, which pin behaviour that is already true and may pass at once. One commit per task.

**Shared test helpers**, added in the task that first needs them:

- `RunHarness` gains, in a new `crates/cli/tests/support/run_orch.rs` (M9.16; `run_harness.rs` is 542 lines):
  - `start_goal(goal, extra: &[&str]) -> String`: runs `anthrex run start --goal <goal> <extra…>` and returns the run id from stdout;
  - `orchestrator_window(run) -> u32`: waits with a deadline loop, up to `ORCH_WAIT`, for `RunInfo.orchestrator.window_id`;
  - `type_into(window_id, bytes)`: a real client connection sending `ClientMsg::Input`, as a user's keystrokes arrive;
  - `mcp_log() -> Vec<Value>`: the lines of `FAKE_AGENT_MCP_LOG` (`<tmp>/agent-io/mcp.jsonl`), which the harness now sets;
  - `wait_digest(run, pointer, value)`: polls `run status --json` until the JSON pointer equals the value;
  - `history_lines(kind) -> Vec<Value>`: the repository's `history.jsonl` lines of one `type` (decision 43).
- **`ORCH_WAIT` = 120 s**, derived in task M9.16 from the test configuration: `[orchestrator.planners] timeout_secs = 120` (the minimum) bounds a planner; a scripted orchestrator's longest wait is one `run_status` with `wait_secs = 50` plus one wake after `wake_quiet_secs = 1` and the 1-second tick. Scenario tests themselves wait with M8a's `RUN_WAIT`. Add the `ORCH_WAIT` row to `docs/timing-budgets.md` with that derivation.
- **Test configuration**: every M9 end-to-end test writes `[orchestrator] wake_quiet_secs = 1` and `[orchestrator.planners] timeout_secs = 120`, and uses M8b's `with_deciders("claude", dir)` with a scripted triage answer, or no deciders for the fallback.
- **Linux CI** (the CI runs clippy 1.98 on macOS and Linux): every new test that needs M8a's sandbox either runs where the sandbox is available or passes `--unconfined-checks` exactly as M8a's and M8b's Linux tests do; every shell stand-in reads its prompt (dash exits differently from zsh on a closed stdin); every repository a test creates sets its own `user.name` and `user.email`. Check with `cargo +1.98 clippy` for the host and for `--target x86_64-unknown-linux-gnu` before pushing.
- **Tests never reach real agents**: every fixture pins `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` (and every decider binary) to `fake-agent` or a nonexistent path; no test changes program selection. Kill tests scope to their own recorded pids.

### M9.1 Verify external facts and reconcile names

**Files.** `docs/milestones/M9-orchestrator-and-subplanners.md` ("Implementation notes" only). No code. Run everything in a scratch repository under `/tmp`, never in this repository and never through a live `anthrex` (AGENTS.md rule 1).

**Checks.** Record the command, the CLI version and the observed output (trimmed) for each:

1. **Interactive Claude flags.** `claude --help` lists `--mcp-config`, `--allowedTools`, `--disallowedTools`, `--append-system-prompt`, `--strict-mcp-config`, `--effort`, `--name`, `--settings`, `--resume`, `--setting-sources`. Start `claude` interactively (not `-p`) with decision 7's argv and `ENABLE_TOOL_SEARCH=false` in the environment in a scratch repository and a stub MCP server (the tool list is what matters): the TUI starts, `/mcp` lists `anthrex` connected with its six tools, the tools are callable in the first turn without a `ToolSearch` call, and the first prompt after `--` is submitted.
2. **`--disallowedTools` holds.** In that session, ask the model to write a file, run `ls`, and start a sub-agent. Each is refused without a permission prompt. If any runs, decision 7's fallback applies (`--permission-mode plan`); record which.
3. **MCP tool timeout.** Find Claude Code's default MCP tool-call timeout (documentation, or a stub tool that sleeps 70 s). If below 120 s, decision 10's `MCP_TOOL_TIMEOUT=120000` is set, and a 70 s sleep then completes.
4. **Codex flags.** `codex --help`: `-s read-only` and `-a on-request` are accepted ahead of `resume <id>`; `-c mcp_servers.anthrex.tool_timeout_sec=120` and `default_tools_approval_mode="auto"` are accepted keys; a read-only interactive Codex calls an MCP tool over the stub's Unix socket (record whether the socket connect succeeds from inside `read-only`).
5. **`StopFailure` and `/compact`.** With `--settings` hooks logging every event, an interactive turn that fails on an API error fires `StopFailure` (use an invalid model name, or record that it cannot be triggered and cite the hook documentation). A manual `/compact` ends with `Stop` or leaves no turn open; if it leaves `Working`, decision 12's fallback applies.
6. **Bracketed paste.** In both TUIs, `ESC[200~` + a two-line text + `ESC[201~`, then 200 ms, then `\r`, submits the text as one message; without the delay, record whether the `\r` lands inside the paste.
7. **OTLP interactive.** An interactive Claude session with `metering::orchestrator_env`'s variables and decision 14a's `OTEL_EXPORTER_OTLP_HEADERS` posts metrics to a local listener within its export interval, with the header present; record whether the exporter sends `Expect: 100-continue` (decision 14) and whether it exports deltas (the global series cap stays a follow-up if it does).
8. **Names.** For every row of "Names taken from earlier briefs", confirm the merged code has it. Record every difference and the name used instead.
9. **Counts and version.** `wc -l` of every file in "File sizes this milestone must respect", and `PROTO_VERSION` on `main`: expected `9` (`crates/proto/src/lib.rs:40`), so M9 makes 10. If it is not 9, stop (header).
10. **MCP tool visibility, once, against the real CLI** (*added 2026-09-27; the tool-search fix is merged, so this only re-verifies it*). One headless worker session with the argv `run/role_launch.rs` builds (including `--allowedTools` with the three `WORKER_MCP_TOOLS` of decision 42f) and `ENABLE_TOOL_SEARCH=false`, against the stub MCP server: the stream's `system/init` event lists `mcp__anthrex__task_done`, `mcp__anthrex__task_blocked` and `mcp__anthrex__task_note`, and the session calls `task_done` without a `ToolSearch` call. Record the CLI version and the `system/init` tools line. One probe; no fixture is added. If a tool is missing, stop and record it: decision 42f depends on it.
11. **`packed-refs.lock`** (*added 2026-09-27; followups file, "From the Claude tool-search fix"*). In a task checkout configured by `run/git/checkout.rs` (which already sets `gc.auto=0`), find which git command a sandboxed worker runs that tries to create `packed-refs.lock` in the common dir (for example `git commit` after many refs, `git maintenance`, `git pack-refs`), with `GIT_TRACE=1`. Record the command and the config key that stops it. Task M9.13c uses the result.

**Acceptance.** Each check has a recorded result. Any decision whose fallback was triggered names the check. No file other than this brief changed.

**Commit.** `docs: record the external facts and names milestone 9 builds on`

### M9.2 Protocol

**Files.** Create `crates/proto/src/orch.rs` and `crates/proto/src/orch_tests.rs`. Modify `crates/proto/src/{lib.rs, run.rs, run_info.rs, run_wire.rs, messages.rs, history.rs, planner.rs, adapt_tests.rs}` and the in-crate round-trip suites (`run_tests.rs`, `run_tests_view.rs`; proto has no `tests/` directory). Update every exhaustive `match` on the extended enums in `crates/daemon`, `crates/tui` and `crates/cli` with the minimum arm so the workspace builds; behaviour comes in later tasks. The known ones: `tui/src/app/runs.rs::state_text` (`:73`, `Planning` as `planning`), `tree/run_rows.rs::round_label` (`:32`, `Planner` as `planner #{session}`), `theme.rs::{run_color, task_glyph, task_color}` (`:40`, `:54`, `:80`), `graph/paint/style.rs`, `run/edit_log.rs::describe_one` (`:51`, `message` and `refresh` described as `message to <to>` and `refresh <t>`), `cli/src/tree_cmd.rs`, `run_cmd/status.rs`; `HistoryLine` in `run/history_io.rs:119-121`, `run/engine/history.rs:33-35`, `run/stats.rs:90-92`. The server handles `ClientMsg::RunTagged` as `ClientMsg::Run` and echoes the id (decision 2).

**Tests first.**
- `orch_types_round_trip`: one value of each new type and variant (every `HoldKind`, `HoldState`, `IntegrationState`, `TaskNoteKind`) through MessagePack and JSON.
- `new_requests_round_trip`: `ApproveHold`, `RejectHold`, `StartGoal` with and without `orchestrator`, `Promote` with `orchestrator`, `ClientMsg::RunTagged`.
- `request_id_round_trips`: `RunReply::Done` and `Refused` with `request_id: Some(7)`, and an M8c-shaped reply without the field decodes as `None`.
- `old_run_info_still_decodes`: an M8c-shaped `RunInfo` (a JSON fixture written from M8c's type, checked in) decodes with every new field at its default.
- `appended_variants_keep_their_indices`: `AgentRole::Planner`, `RunState::Planning`, `TaskState::Reported`, `BlockReason::MessagePause`, `PlanEdit::{Message, Refresh}` serialize after every older variant.
- `proto_version_is_ten` (renamed from `proto_version_is_nine`, `lib.rs:109`).
- `reported_is_finished_and_planning_is_not_terminal`.
- `message_and_refresh_edits_round_trip`: a task-id array, `stage:<n>` and `running` targets, every message kind, `Refresh`, and `BlockReason::MessagePause` survive MessagePack and JSON; `"stage:x"`, `"runnin"` and an empty array fail to parse.
- `role_routing_history_round_trip`: every new role, an absent run id for pre-run triage, candidate order and skip reasons, and a version-1 history line decode; `HISTORY_VERSION` is 2 (M8b's `adapt_tests.rs:208` assertion becomes 2); an old persisted run defaults `role_routing_decisions` to empty (in `crates/daemon`, against `run/engine/tests/m8b_run.json`).

**Acceptance.** The five AGENTS.md commands pass.

**Commit.** `feat(proto): add the orchestrator, sub-planner, hold, message and role-history types`

### M9.3 Configuration

**Files.** Create `crates/config/src/orchestrator/agent.rs` and `crates/config/src/orchestrator_tests_agent.rs` (the crate's tests live beside `orchestrator_tests_adapt.rs`). Modify `crates/config/src/orchestrator.rs` (one call, one field), `orchestrator/unknown.rs` (the known keys), `run/orch/mod.rs` (`OrchLimits`, `PlannerLimits`), M8a's `RunLimits` construction in `run/plan.rs`, `run/triage.rs` and `decider/prompt.rs`.

**Earlier-brief change (M8b's `PLAN_SCALE_MAX`, defect 20).** Delete `run::triage::PLAN_SCALE_MAX` (`triage.rs:23`, defined and unused). `decider/prompt.rs`'s `TRIAGE_HEAD` (`:27`) holds the literal `2 to 12 tasks`; it becomes a function of `planner_task_cap`, which the driver passes into the triage decider's input, so with the default 12 the prompt is byte-identical and M8b's `triage_prompt_is_exact_for_a_fixed_input` (`decider/tests_prompt.rs:92`) passes unchanged. `run::triage::route` and `TriageRoute` are unchanged.

**Tests first.**
- `defaults_when_absent`: every key of Interfaces "config" at its default, `message_max_per_turn` 3 and `note_max_per_task` 10 included.
- `each_range_is_enforced_with_the_exact_message`: one case per numeric key at each bound and one past it; zero is accepted for the two message and note keys.
- `effort_and_strength_and_runtime_parse`: every accepted spelling; a wrong one gives the message and the default.
- `unknown_keys_warn`: under `[orchestrator.agent]` and `[orchestrator.planners]`.
- In `run/plan_tests.rs`: `limits_are_frozen_at_run_start` (a run built with `message_max_per_turn = 1` keeps 1 after the config changes).
- In `decider/tests_prompt.rs`: `triage_prompt_uses_planner_task_cap` with a cap of 7 renders `2 to 7 tasks`.

**Acceptance.** `crates/config/src/lib.rs` unchanged. The five AGENTS.md commands pass.

**Commit.** `feat(config): add the orchestrator agent, planner, wake, message and note settings`

### M9.4 Plan rules for planners, research and review kinds

**Files.** Create `crates/daemon/src/run/orch/{mod.rs, rules.rs}` (model types used by the rules; the rest of `mod.rs` fills in M9.7) and `crates/daemon/src/run/edits_orch.rs` (decision 25's `amend_task deps`; `message` and `refresh` arrive in M9.13a). Modify `run/validate.rs` (decision 24; remove decision 6's refusal), `run/edits.rs` (the delegation; a task added by a planner inherits its epic), `run/globs.rs` (decision 23a), and M8a's `research_and_review_kinds_are_deferred` test (`validate_tests_fields.rs:179`), which is deleted with a comment naming this task.

**Earlier-brief changes (M8a, defect 14).** In `run/validate.rs`:
1. The `owns_required` check (M8a decision 11, test `owns_required` at `validate_tests_fields.rs:135`) applies to `code` and `docs` tasks only. A `research` or `review` task must have an empty `owns` instead (decision 24's message). `owns_required` keeps passing for a code task and gains a research case accepted with `owns = []`.
2. M8a decision 6's refusal of `kind = "research"` and `kind = "review"` is deleted, with its test.

**Tests first**, pure, in `run/orch/rules.rs` and `run/validate_tests_fields.rs`:
- `planner_budget_is_refused` / `plan_file_budget_is_still_accepted` (decision 23.1, and that `EditSource::User` and plan files keep M8a's rules).
- `code_task_without_scout_refs_is_refused_when_reports_exist`; `unknown_scout_ref_is_refused`; `no_reports_gives_a_note_not_an_error`; `onboarding_ref_is_accepted`.
- `orchestrator_cap_counts_tasks_without_an_epic`; `epic_cap_counts_the_epics_tasks`; `cancelled_and_integration_review_tasks_do_not_count`; each with the exact message.
- `task_naming_an_unknown_epic_is_refused`; `task_for_a_live_planners_epic_from_the_orchestrator_is_refused`; `fix_task_for_a_finished_epic_is_accepted`; `integration_review_ids_are_reserved`.
- `research_task_with_owns_is_refused`; `review_task_needs_a_review_target`; `review_target_syntax` (a table: `main`, `a1b2c3d`, `main..feature/x`, `HEAD~3..HEAD` accepted; `-x`, `a..b..c`, `a b`, empty refused); `code_task_with_review_target_is_refused`; `research_and_review_force_test_mode_none_with_the_note`.
- `amend_deps_unblocks_dep_cancelled`; `amend_deps_rejects_a_cycle`; `amend_deps_rejects_a_cancelled_dep`.
- In `run/globs_tests.rs`: `non_ascii_paths_are_protected` (decision 23a: a path with a non-ASCII byte is protected; an ASCII path keeps M8a's answer).
- `rule_ids_are_the_documented_ones`: every `PlanError.rule` this file produces is in the decision-23 list.

**Acceptance.** `run/orch/rules.rs` and `run/edits_orch.rs` are pure (decision 1's grep passes). The five AGENTS.md commands pass.

**Commit.** `feat(daemon): check planners' plan edits and accept research and review tasks`

### M9.5 Contracts and prompts

**Files.** Create `crates/daemon/src/run/orch/contract.rs` (and `run/orch/contract_tests.rs`). Modify `run/contract.rs` (`WORKER_CONTRACT`'s two lines; `worker_prompt`'s and `handover_prompt`'s scout extract and notes section, and `reviewer_prompt`'s messages section, each taking its text as an argument the engine fills), `run/contract_tests.rs`.

**Tests first**, pure:
- `orchestrator_contract_is_exact` and `planner_contract_is_exact`: byte comparison with the texts in Interfaces.
- `worker_contract_is_exact` (M8a's test, updated to 12 lines).
- `orchestrator_contract_covers_every_planning_rule`: the contract contains each of these phrases, one assertion per phrase so a failure names it: `interface or hub change is its own task, first`, `Split one level only`, `combined size is still M`, `Size from evidence, never from time`, `Never give minutes`, `hub task: it runs alone, is tdd, and is always reviewed`, `L is never executed`, `planner_task_cap`, `Generated files`, `Protected files`, `never through a wildcard`, `is tdd: name the test to write`, `is never none`, `never give tasks on different runtimes overlapping owns`, `You never approve a task and never merge`, `never read task worktrees`, `run_status`, `wait_secs 50`, `awaiting_approval`, `mis_sized`, `question`, `summary for the user`, `A message informs; an amendment changes the task`, `A message or refresh is always the only edit in its edit_plan call`, `refresh for that task, then call edit_plan again with a change message`, `task_note`.
- `planner_contract_covers_its_rules`: `only inside your area`, `L is never executed`, `interfaces first, one level only`, `never from time`, `Never plan a change to a hub file`, `Call submit_epic once`, `planner_task_cap`, `never give tasks on different runtimes overlapping owns`.
- `contracts_name_every_tool_by_its_claude_id`: at its first mention in each contract, every tool of that role is followed by `(in Claude: mcp__anthrex__<tool>)` (decision 41; the tool-search fix's form).
- `contracts_have_no_em_dash_and_survive_toml`: no `—`; `launch::codex::toml_string` round-trips each through the `toml` crate.
- `contracts_do_not_vary`: built twice for two different runs, byte-equal (the facts are in the prompts).
- One `*_prompt_is_exact` test per row of Interfaces "Prompts and messages", on a fixed run.
- `worker_prompt_places_the_scout_extract_before_the_brief`: order is prompt text, profile summary, extract, notes, brief last; `extract_is_capped_at_12_kib_with_a_cut_marker`; `unreadable_report_is_left_out`.
- `worker_prompt_puts_saved_messages_before_the_brief`; `handover_prompt_carries_the_saved_messages`; `reviewer_prompt_lists_change_messages`.
- `change_message_requires_acknowledgement_in_task_done`: a contract-text test (the worker contract's line 11 names `Changes applied:`); the engine never checks the summary (decision 42d).
- `stop_and_wait_text_is_exact`.
- `wake_text_is_clamped`.

**Acceptance.** Every text in Interfaces is a `const` or a function in `run/orch/contract.rs` (the worker contract's two lines stay in `run/contract.rs`). The five AGENTS.md commands pass.

**Commit.** `feat(daemon): add the orchestrator and sub-planner contracts, and the worker's note and message lines`

### M9.6 The digest, the context, the task result and the snapshot

**Files.** Create `crates/daemon/src/run/orch/{digest.rs, context.rs, result.rs, tools.rs}`, `crates/daemon/src/run/git/summary.rs`. Modify `run/model.rs` (`Run.orch` with `digest_rev`, `digest_fp`; the other new fields arrive in M9.7 but may be added here), `run/snapshot.rs` (`RunInfo.digest_revision`; decision 16a), the driver's publish path (decision 20's `RunInfo.scouts` overlay).

**Tests first.**
- Pure, `digest.rs`: `digest_shape_matches_the_interface` (a fixed run rendered and compared as JSON with a checked-in fixture); `counter_changes_do_not_change_the_fingerprint` (tool calls, tokens, spend, `now`, a history line appended to a task whose state did not change); `state_block_verdict_hold_scout_planner_note_message_and_edit_changes_do` (one case each); `digest_is_capped_and_trims_in_order` (a 50-task run with 500-character blocks and 100 edits stays under `DIGEST_MAX_BYTES`, finished tasks dropped first with `omitted_tasks`); `gate_states` (planning, awaiting, approved with `approved_by`, none, a hold decided since the last read); `message_pause_counts_as_paused`.
- Pure, `context.rs`: `context_for_the_orchestrator`; `context_for_a_planner_filters_reports_and_tasks`; `context_is_capped_at_96_kib_with_omitted_counts`; `only_listed_scouts_are_returned`; `installed_is_carried`; `context_names_the_base_sha`.
- Pure, `result.rs`: `task_result_carries_every_field`; `task_without_start_has_no_git`; `git_error_is_reported`; `task_result_is_capped`.
- Pure, `tools.rs`: `parse_call_for_each_tool` (every schema property, `task_note` included); `wait_secs_over_50_is_refused`; `tool_outside_the_role_is_refused`; `planner_cannot_call_edit_plan`; `override_op_fails_to_parse_with_the_documented_message`.
- Blocking, `summary.rs` against a temporary git repository: `task_summary_lists_commits_and_diffstat`; `resolve_target_of_a_range_and_of_a_single_revision`; `resolve_target_refuses_an_unknown_revision`; `summary_times_out` (a `git` stand-in script that sleeps, with a 1 s timeout). Every invocation goes through `worktree::run_git` with `--no-optional-locks` and a scrubbed environment (AGENTS.md rule 11), asserted by reading the argv and environment the stand-in logs.
- Snapshot (decision 16a), in `run/snapshot.rs`'s tests: `briefs_are_published_only_at_the_gate` (a running run's `TaskInfo.brief`, `acceptance` and `route_spec` are empty; an `awaiting_approval` run's and an awaiting hold's are filled); `a_snapshot_of_fifty_terminal_runs_stays_small` (50 complete runs of 20 tasks with 4 KiB briefs encode under 256 KiB); `task_notes_are_capped_at_ten_in_the_snapshot`.
- Driver: `run_scouts_appear_in_the_snapshot` (a run scout started through `ScoutService` shows in `RunInfo.scouts` with M8b's `ScoutState`).
- Engine: `digest_revision_moves_only_on_fingerprint_change` (a reducer step sequence).

**Acceptance.** The four `run/orch` files are pure. M8c's edit form tests pass unchanged. The five AGENTS.md commands pass.

**Commit.** `feat(daemon): build the run digest, the planning context and the task result, and trim the snapshot`

### M9.7 Engine: planning, submit, holds and promotion

**Files.** Create `crates/daemon/src/run/engine/{orch.rs, gate_holds.rs, promote.rs}`, `crates/daemon/src/run/orch/launch.rs` (`resolve_orchestrator`, `orchestrator_role`, `orchestrator_window_spec`), `crates/daemon/src/run/engine/tests/{orch.rs, gate_holds.rs, promote.rs}`. Modify `run/engine/{mod.rs, requests.rs, dispatch.rs, restore.rs, complete.rs}` (the runnable test's hold condition and the pre-warm skip), `run/model.rs`, `run/model_adapt.rs` (the attention text and `hh_mm`), `run/snapshot.rs`, `run/report.rs` (summary section), `run/reconcile/mod.rs` (the rows of Interfaces), `run/reach.rs` (decision 26), and M8b's tests in `run/engine/tests/fast_path.rs` and `run/driver/adapt_goal_tests.rs`.

**Earlier-brief changes (M8a, M8b).**
1. M8a decision 41's runnable test gains one condition: the task's `orch.gate_hold` is `None` or names a hold in state `Approved` (decision 28). M8a decision 14's pre-warm skips a task whose hold is not `Approved`.
2. M8b's promote handling (`engine/requests.rs::promote`, `:174-210`) moves to `engine/promote.rs` and is replaced by decision 29. `promote_records_intent_once` (`fast_path.rs:126`) is rewritten: the first promote emits `OpKind::CreateOrchestrator`, and the second answers `run <id> was already marked for promotion` (no time; M8b's `at 13:07` at `:168` goes). `model_adapt.rs::hh_mm` (`:33`), whose only caller was the repeat reply, is deleted. `promote_refuses_plan_runs_and_terminal_runs` (`:177`) keeps its texts and gains a planned-run case. The attention text `promotion requested; it takes effect when the orchestrator exists (milestone 9)` (`model_adapt.rs:25`) goes: a promoted run shows the orchestrator instead.
3. M8b's `a_fast_path_run_refuses_task_additions` (`fast_path.rs:294`) and the refusal text at `requests.rs:286-293` stay for a fast-path run that is not promoted; a promoted run accepts the orchestrator's additions into the promotion hold (decision 29).
4. `adapt_goal_tests.rs:125` (the plan-path refusal) is rewritten with decision 26's planned run.

**Tests first**, pure reducer tests in `run/engine/tests/`:
- `planned_run_starts_in_planning_and_creates_branch_then_orchestrator` (effects in that order; no task, no pre-warm).
- `resolve_orchestrator_order` (choice, `[orchestrator.agent]`, default; unknown model refused with the exact text; Codex falls back to `""` at standard); `resolve_orchestrator_keeps_the_candidate_snapshot` (decision 43).
- `approve_while_planning_is_refused`; `reject_while_planning_discards`.
- `submit_with_no_tasks_is_refused`; `submit_while_a_planner_is_live_is_refused`; `submit_opens_the_gate`; `submit_with_yes_runs_at_once_and_records_approved_by`; `resubmit_in_awaiting_approval_changes_nothing`; `spawn_subplanner_in_awaiting_approval_returns_to_planning`.
- `edit_plan_is_one_batch` (one bad edit rejects all; the run is unchanged, including `digest_rev`); `edit_plan_reply_shapes` (accepted and rejected JSON exactly).
- `summary_on_a_complete_run_is_accepted_and_edits_are_not`; `summary_is_written_to_the_report`.
- `new_epic_on_a_running_run_is_held`; `held_tasks_are_not_runnable_or_prewarmed`; `approve_hold_releases_its_tasks`; `reject_hold_cancels_its_tasks_and_blocks_dependents`; `hold_verdict_errors` (unknown hold, already decided); `with_yes_an_epic_hold_is_approved_on_submit`; `gate_holds_do_not_touch_dependency_holds` (M8a ruling N5's `engine/holds.rs` state is unchanged by an approval hold).
- `promote_creates_an_orchestrator_and_leaves_t1_running`; `tasks_added_before_submit_after_promote_carry_the_promotion_hold`; `promotion_hold_is_awaiting_after_submit`; `pre_m9_promote_request_is_performed_on_resume_and_on_tick`; `promote_repeat_reply_has_no_time`.
- `restore_pauses_a_planning_run_and_resume_restarts_the_orchestrator`; `resume_of_awaiting_approval_restarts_a_dormant_orchestrator_without_changing_state`.
- `orchestrator_calls_from_another_window_are_refused` with the exact text.
- `planned_message_is_exact`.
- `reachable_runtimes_include_the_orchestrator_and_planners` (in `run/reach_tests.rs`).
- Reconcile: `create_orchestrator_is_replayed_from_a_restored_window`; `start_scout_is_not_replayed`.
- `rejected_plan_is_seen_by_run_status` (after `Reject`, the digest's `run.state` is `discarded`, and a later `edit_plan` answers `run <id> is discarded`).

**Acceptance.** `engine/requests.rs` shrinks by the promote body. The five AGENTS.md commands pass.

**Commit.** `feat(daemon): plan a goal with an orchestrator, and hold work added after the gate`

### M9.8 Engine: sub-planners and run scouts

**Files.** Create `crates/daemon/src/run/engine/planners.rs`, `crates/daemon/src/scout/planner.rs`, `run/engine/tests/planners.rs`. Modify `run/orch/launch.rs` (`planner_spec`, `scout_spec`), `run/engine/dispatch.rs` (one call into `planners.rs`'s reader-slot order, decision 31), `scout/machine.rs` (`MachineTexts`), `scout/service.rs` (`start_planner`, `accept_planner`, `stop_planner`), `run/snapshot.rs` (`RunInfo.planners`, decision 33), `headless/{mod.rs, argv.rs}` (`McpTarget.epic`, `--epic`, the `Planner` arm).

**Earlier-brief change (M8a's `McpTarget`, defect 18).** In `headless/mod.rs`, `McpTarget` (`:78`) gains `#[serde(default)] pub epic: Option<String>`, appended after M8b's `scout_id`; in `headless/argv.rs`, `mcp_args` (`:238`) gains the `Planner` arm and appends `--epic <e>` after `--scout` and before `--window`. M8a's `mcp_args_for_a_worker` (`headless/argv_tests.rs:201`) and M8b's `mcp_args_for_a_scout` (`headless/argv_caps_tests.rs:327`) pass unchanged, and a persisted `HeadlessSpec` without `epic` still loads.

**Tests first**, pure unless noted:
- `spawn_subplanner_validates_epic_and_area` (pattern, `/**` form, overlap with the exact text); `spawn_subplanner_sets_path_large`.
- `planner_waits_for_a_reader_slot_and_the_order_is_deciders_reviewers_planners_scouts_kinds`.
- `planner_spec_is_read_only_and_names_its_epic` (allowed tools, the scout machine's read-only launch, `mcp_args` has `--epic <e>`, window name `<h4>/plan-<e>.p1`).
- `planner_machine_uses_its_own_texts` (in `scout/machine.rs`: `PLANNER_NUDGE`, `planner_wrap_up`, `submit_epic`; scouts keep M8b's texts).
- `planner_and_research_specs_carry_the_read_only_sandbox` (followups file, F1 N4).
- `submit_epic_outside_the_area_is_rejected_with_the_area_error`; `submit_epic_cannot_cancel_or_amend_another_epics_task` (followups file, M8a.6: `EditScope::Area` limits only `owns`); `submit_epic_refuses_answer_pause_resume_finish_message_refresh`; `submit_epic_for_another_epic_is_refused`; `accepted_submit_finishes_and_retires`; `second_submit_is_refused`; `rejections_count_and_fail_at_max_rejections`.
- `turn_without_submit_is_nudged_once_then_fails`; `tool_call_wrap_up_then_kill`; `timeout_fails`.
- `replan_starts_a_fresh_session_with_the_current_tasks`; `replan_of_a_live_planner_is_refused`.
- `failed_planner_keeps_accepted_tasks_and_wakes_the_orchestrator`.
- `spawn_scout_queues_in_a_reader_slot_and_replies_at_once`; `scout_id_is_prefixed_and_unique`; `max_scouts_is_enforced`; `scout_ended_records_the_report_and_usage`; `restore_fails_running_scouts_with_the_documented_reason`; `restore_kills_nothing` (no kill effect for a run scout or a planner).
- `planners_snapshot_fields` (every `PlannerInfo` field, a queued planner as `Planning`).
- Real, in `crates/cli/tests/scout_service.rs` (M8b's service suite): `a_planner_session_runs_on_the_scout_machine_and_is_accepted` with `fake-agent`.

**Acceptance.** The five AGENTS.md commands pass.

**Commit.** `feat(daemon): run sub-planners per epic and scouts per area`

### M9.9 Engine: research, review, integration, completion and wake notes

**Files.** Create `crates/daemon/src/run/engine/kinds.rs` and `run/engine/tests/kinds.rs`. Modify `run/orch/launch.rs` (`research_spec`, `review_task_spec`), the runnable test (decision 35 dependents; the hold condition is M9.7's), `run/engine/complete.rs` (decision 38), `run/engine/done.rs` (decision 25's restart on rewrite), `run/report.rs` (`## Research`, `## Review findings`), `run/edit_log.rs` (decision 40), M8b's scout tool routing for task-bound scouts (decision 35), `run/engine/signals.rs` (the saturating usage sum).

**Earlier-brief changes (M8a, M8b, M8c; defects 15, 19).**
1. M8a decision 41's runnable test: a **declared** dependency is satisfied when it is `merged` **or `reported`** (was: `merged`). Implicit dependencies are unchanged (research and review tasks have no `owns`, so they never create one).
2. M8b decision 15's tool routing (`run/driver/adapt.rs::tool`, `:153`) gains one branch ahead of `ScoutService::tool`: a call with `role == Scout`, `task_id` set and `scout_id` unset goes to the engine.
3. M8c's `edit_log::record(run, edits, now)` (`edit_log.rs:67`) becomes `record(run, edits, now, source, outcome)`; its one caller (`engine/requests.rs:352`) passes source `user` and outcome accepted. M8c's `edit_log_tests.rs` pass with the new arguments.
4. `engine/signals.rs::add_usage` (`:196`) sums with `TokenUsage`'s saturating `+=` (followups file, "From M8b.15").
5. **`Reported` is a finished state in the engine** (M9.2 review ruling 4). M9.2 added `TaskState::Reported` to the wire only; three engine places still treat it as unfinished, and this task, which produces `Reported`, fixes each:
   - `engine/complete.rs::complete_pass` (`:121-125`) counts only `merged` and `cancelled` as finished; a run whose remaining tasks are `reported` must complete. Test: `a_run_of_merged_and_reported_tasks_completes`.
   - `engine/outbox.rs` (`:101-106`) skips `blocked`, `merged` and `cancelled` tasks when delivering; a `reported` task must be skipped too, so no message is queued for a finished session. Test: `the_outbox_skips_a_reported_task`.
   - `run/history.rs::outcome` (`:44-54`) maps every other state, `reported` included, to `TaskOutcome::Unfinished`; a reported task's history record must carry an outcome that says it finished (an appended `TaskOutcome` variant is the natural one, inside PROTO 10 and `HISTORY_VERSION` 2). Test: `a_reported_task_is_recorded_as_finished`.

**Tests first**, pure:
- `research_task_takes_a_reader_slot_and_no_worktree`; `research_report_marks_the_task_reported`; `research_without_report_is_nudged_then_blocked`; `dependent_of_a_reported_task_runs`.
- `review_task_resolves_its_target_first`; `unresolvable_target_blocks_environment_with_the_text`; `review_verdict_reports_whatever_it_is`; `review_findings_go_to_the_report`.
- `integration_review_task_is_made_when_the_epic_is_merged` (id `<e>-int1`, kind review, engine-made); `integration_changes_holds_completion`; `fix_task_merge_makes_round_two`; `finish_closes_changes`; `no_round_after_max_bounces_plus_one`; `plan_path_has_no_integration_review`; `integration_reviewer_route_is_the_peer_at_frontier`; `integration_review_tasks_refuse_orchestrator_edits`.
- `completion_waits_for_holds_planners_scouts_integration_and_submit` (one case per condition).
- `rewriting_a_mis_sized_task_restarts_it_at_rung_2`; `editing_a_human_blocked_task_does_not_restart_it`.
- Wake notes: `each_note_source_adds_its_exact_line` (a table over decision 39's list); `orchestrators_own_edits_add_no_note`; `notes_are_capped_at_20_with_the_earlier_line`; `wake_effect_needs_live_window_setting_and_new_revision`; `digest_read_drops_notes_up_to_the_revision`; `woken_clears_delivered_notes`; `completion_note`.
- `edit_log_records_every_source_rejections_and_recipients` (M8c's cap of 50 kept).
- `usage_sum_saturates`.

**Acceptance.** The five AGENTS.md commands pass.

**Commit.** `feat(daemon): execute research and review tasks, review each epic, and note what the orchestrator must see`

### M9.10 The orchestrator window

**Files.** Create `crates/daemon/src/launch/role.rs`, `crates/daemon/src/manager/role_window.rs`, `crates/daemon/tests/orchestrator_window.rs`. Modify `launch/{mod.rs, claude.rs, codex.rs}`, `hooks.rs`, `manager/{create.rs, restart.rs, entry.rs, restore.rs, headless.rs}`, `window.rs`, `server.rs`, `server/headless_guard.rs`, `metering/server.rs` (decisions 14a, 14b), `crates/tui/src/app/headless.rs` (decision 11's TUI refusal) and its tests in `app_tests/headless.rs`.

**Tests first.**
- Pure, `launch/role.rs` and `launch/mod.rs`: `claude_orchestrator_argv_is_exact` (the whole argv for a fixed role, in decision 7's order, with `--` before the prompt; with `--resume` instead); `claude_argv_without_user_settings_only_caps` (no exclusion flags; `--strict-mcp-config` per decision 7); `codex_orchestrator_argv_is_exact` (decision 8's block before `-m`, then `resume <id>`); `plain_windows_are_unchanged` (M3's argv tests pass untouched, and `role: None` produces byte-identical argv); `role_env_order_and_otlp` (M3's four variables first, then OTLP and the token header, then `MCP_TOOL_TIMEOUT` when M9.1 set it, then `ENABLE_TOOL_SEARCH=false` last); `claude_orchestrator_env_turns_tool_search_off_and_codex_does_not`.
- `claude_settings_json_is_exact` (`launch/claude.rs:42`) updated for 11 events; `stop_failure_parses_and_maps_to_stop`.
- Real PTY, in `crates/daemon/tests/orchestrator_window.rs`, with `fake-agent` as the runtime binary:
  - `run_window_is_pty_with_run_ref_and_counts_to_max_windows`;
  - `scrub_removes_agent_session_and_credential_variables` (the test process sets `CLAUDECODE=1`, `CLAUDE_CODE_ENTRYPOINT=x` and `ANTHROPIC_API_KEY=x` with `auth = "login"`; `fake-agent` writes its environment to a file; none is present, `ANTHREX_WINDOW_ID` and `ENABLE_TOOL_SEARCH=false` are);
  - `restart_repasses_role_flags_with_resume` (the args file after `restart` has `--mcp-config`, `--disallowedTools` and `--resume <id>`);
  - `role_survives_a_daemon_restart` (persisted `{"role_launch": …}`; after restore and restart, same flags); `unparseable_role_restores_a_plain_window_with_a_warning`;
  - `unparseable_headless_record_restores_as_an_exited_headless_window` (decision 11a: `Subscribe`, `Input` and `Restart` refused; `Remove` works);
  - `kill_and_remove_are_refused_while_the_run_is_live` (exact `DaemonMsg::Error`), `and_allowed_once_it_is_terminal`;
  - `client_input_time_is_recorded`;
  - `stop_failure_hook_makes_the_window_idle` (a script sends `UserPromptSubmit` then `StopFailure` through `anthrex hook`; status goes `Working` then `Idle`).
- `create_run_window_does_not_hold_the_manager_lock_across_spawn`: the existing M1 lock test pattern (a second `list` answers while a slow `spawn` is in progress), AGENTS.md rule 2.
- In `crates/daemon/tests/otlp_server.rs`: `points_without_the_runs_token_are_dropped`; `connection_cap_grows_with_live_orchestrators`; `untokened_connections_are_closed_first_when_full`.
- TUI, pure: `kill_and_remove_of_a_live_orchestrator_open_nothing` (the toast is the refusal text; `C-b R` still opens its dialog).

**Acceptance.** `create.rs` and `restart.rs` grow within their budgets. The five AGENTS.md commands pass.

**Commit.** `feat(daemon): launch the orchestrator read-only in a PTY window, and keep its role across restarts`

### M9.11 MCP tools

**Files.** Create `crates/mcp/src/tools_orch.rs` and `crates/mcp/src/tools_orch_tests.rs`. Modify `crates/mcp/src/{tools.rs, lib.rs}`, `crates/cli/src/mcp_cmd.rs` (hidden `mcp` flags: roles `orchestrator` and `planner`, `--epic`, `--task` for `scout`) and `mcp_cmd_tests.rs`, `run/driver/adapt.rs::tool` (the routing of decision 15), `run/driver/orch.rs` (new: `orch_tool`, the long-poll).

**Earlier-brief change (M8b's `--role scout`, defect 19).** M8b's `anthrex mcp` argument check for `--role scout`, which requires `--scout <id>`, becomes: exactly one of `--scout <id>` or `--task <t>`. Neither or both exits with a usage error naming the two flags. With `--task`, `McpOptions.task_id` is set and `scout_id` is `None`. M8b's scout MCP tests pass unchanged.

**Tests first.**
- `crates/mcp`: `tools_for_orchestrator_and_planner_are_exact` (names, order, descriptions); `worker_tools_are_task_done_task_blocked_and_task_note` (renamed from `tools.rs:136`); `every_schema_is_closed_at_every_level` (walks each schema); `schemas_match_the_interface_table` (checked-in JSON fixtures); `planner_needs_epic_and_orchestrator_refuses_it`; `scout_accepts_task_instead_of_scout`; `tool_outside_the_role_is_refused_without_a_daemon` (a socket path that does not exist).
- Driver, with a real daemon socket and no agent (calls made by a test MCP client, M8a's pattern in `crates/mcp/tests/`):
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
2. **PTY `read_message {timeout_ms?, expect?}`.** Sends a `Stop` hook through `anthrex hook` (the turn ends), then reads stdin: a bracketed paste (`ESC[200~ … ESC[201~`) followed by `\r`, or typed bytes up to `\r`. It then sends `UserPromptSubmit` with the text as `prompt`, keeps it as `FAKE_AGENT_MESSAGE`, and continues. It appends each read, with a timestamp, to `FAKE_AGENT_STDIN_FILE`. Exit codes as in M8a.
3. **Script names.** `--role planner --epic <e>` claims `planner-<e>-<n>.jsonl`; `--role reviewer --task <e>-int<n>` claims M8a's `reviewer-<task>-<n>.jsonl` (the integration review is a task, decision 37); `--role scout --scout <h4>-<id>` claims `scout-<id>-<n>.jsonl` (the run prefix stripped); `--role scout --task <t>` claims `scout-<t>-<n>.jsonl`.
4. **New steps.**
   - `mcp_until {tool, args, until: {pointer, equals}, timeout_ms}`: repeats `mcp_call` until the JSON pointer into the result equals the value; exits 4 at the timeout.
   - `capture_json {name, pointer}`: `{{name}}` in later arguments becomes the value at the pointer of `FAKE_AGENT_RESULT` (a string as is, anything else as JSON).
   - `expect {pointer, equals}` on `FAKE_AGENT_RESULT`: exits 3 when it differs.
   - `expect_error_contains {text}` on the last `mcp_call` with `expect_error`.
5. **`FAKE_AGENT_MCP_LOG`.** When set, every `mcp_call` appends `{"script","tool","args","ok","result"}` as one JSON line.

**Tests first**, in `orch_modes.rs`, with the stub daemon of `tests/headless_support/stub_daemon.rs`:
- `pty_mode_parses_the_mcp_role_from_claude_and_codex_argv`;
- `pty_read_message_takes_a_bracketed_paste_and_typed_input` (both forms, through a real PTY; the hook log shows `Stop` then `UserPromptSubmit`);
- `script_names_for_planners_integration_reviewers_and_run_scouts`;
- `mcp_until_polls_until_the_pointer_matches`, `and_times_out`;
- `capture_json_substitutes`; `expect_fails_with_exit_3`;
- `mcp_log_records_each_call`.

**Acceptance.** M3's and M8a's `fake-agent` tests pass unchanged. The five AGENTS.md commands pass.

**Commit.** `feat(fake-agent): script an orchestrator in a PTY, sub-planners and run scouts`

### M9.13 Driver: ops and wake delivery

**Files.** Create `crates/daemon/src/run/driver/{orch_ops.rs, wake.rs}`. Modify `run/driver.rs` (`mod` lines only), `run/driver/ops.rs` (dispatch to `orch_ops`), `RunContext` (scouts, profiles, binaries), `run/driver/adapt_goal.rs` (decision 26 replaces `planned()` at `:244`, called at `:150, :160, :165`; decision 9's check), the driver's publish path (the run-live flag cleared on a terminal run) and restore path (set for a non-terminal run), the driver's `UsageSink` implementation (`token`, `live_orchestrators`; decisions 14a, 14b).

**Earlier-brief change (M8a decision 29, defect 16).** M8a decision 29 removed paste delivery and delivers every engine message as a headless turn. That stays exactly as it is for every headless session. Paste delivery is added back in one place only, `run/driver/wake.rs`, for the orchestrator's PTY window (decision 39); nothing in M8a's outbox or its `Effect::Deliver` changes, and no headless window ever receives a paste. M8b decision 22 step 6's refusal of the plan and large paths is replaced by decision 26.

**Tests first.**
- Pure, `wake.rs`: `encode_paste_wraps_and_normalises` (`\r\n` and `\n` to `\r`, markers stripped, framed); `wake_is_clamped`.
- Driver tests with a real daemon and `fake-agent`:
  - `create_orchestrator_op_starts_the_window_and_reports_it`; `create_orchestrator_op_counts_toward_max_windows`; `create_orchestrator_sets_the_run_live_flag_and_a_terminal_run_clears_it`; `restore_sets_the_run_live_flag_for_a_non_terminal_run`;
  - `restart_orchestrator_op_resumes_it`;
  - `start_scout_op_runs_a_scout_and_sends_scout_ended`; `start_planner_op_runs_a_planner_and_sends_planner_ended`;
  - `resolve_target_op_runs_git_off_the_worker_threads` (on `spawn_blocking`, with the timeout);
  - `wake_is_delivered_only_when_idle_and_quiet` (the window `Working`: nothing; `Idle` with input 0.2 s ago and `wake_quiet_secs = 1`: nothing; after 1 s: delivered, and the `fake-agent` stdin file shows the framed paste then `\r`);
  - `wake_is_not_delivered_on_attention`;
  - `a_newer_wake_replaces_an_undelivered_one`;
  - `wake_writes_happen_outside_every_lock` (the manager answers `list` while a wake's 200 ms delay is pending);
  - `start_goal_on_the_plan_path_builds_a_planned_run` (replaces M8b's refusal);
  - `project_settings_check_covers_the_orchestrator` (a Codex orchestrator in a repository with a tracked `.codex/config.toml` is refused without `--trust-project`, because `CLI_CAPS.codex_user_config_only` is `None`, and starts with it, the files recorded; a Claude orchestrator starts, because `claude_user_settings_only` is set);
  - `orchestrator_exit_adds_the_attention_line_and_suspends_wakes`;
  - `orchestrator_token_is_persisted_and_survives_a_restart` (decision 14a).

**Acceptance.** No blocking call on a tokio worker thread and none under `daemon::lock` (review by `grep` of `wake.rs` and `orch_ops.rs` for `std::process`, `std::fs` and `.lock()`, each allowed only inside `spawn_blocking` closures or through `daemon::lock`). The five AGENTS.md commands pass.

**Commit.** `feat(daemon): drive the orchestrator's ops and wake it when the run needs it`

### M9.13a Orchestrator-to-worker messages, refresh and worker notes

**Files.** Create `crates/daemon/src/run/engine/worker_messages.rs`, `crates/daemon/src/run/driver/refresh.rs`, `run/engine/tests/{worker_messages.rs, refresh.rs}`. Modify `run/edits_orch.rs` (`apply_message`, `apply_refresh`, `one_edit_rule`), `run/edits.rs` (the dispatch arms; `not_started` counts `blocked(message_pause)` as started), `run/engine/mod.rs` (one routing arm ahead of M8a's `HandBack` routing, `:516-519`), `run/engine/ops.rs` (`HandBack.list_merged`, `HandedBack.merged`), `run/engine/outbox.rs` (the paused-delivery exception at `deliver`, `:81`; the refresh hold), `run/engine/done.rs` (the paused `task_done` refusal ahead of `:85`; `task_note` dispatch; the refresh-merge exclusion at `:257`), `run/engine/fallback.rs` (the same exclusion for `NO_COMMIT_NUDGE`), `run/engine/requests.rs` (retry and override refused while paused; `PlanEdit::Resume` releases paused tasks), `run/git/handback.rs` (the dirty re-check and the log read, through `worktree::run_git`), `run/driver/requests.rs` and `run/driver/orch.rs` (call the clean-tree pre-check for `run edit` and `edit_plan`), `run/role_launch.rs` (`WORKER_MCP_TOOLS: [&str; 3]`), `crates/mcp/src/tools.rs` (`task_note` for `Worker`), `run/orch/digest.rs` (`task_notes`, `messages`), `run/snapshot.rs` (decision 42d's `TaskInfo` fields, the paused attention line), `run/report_task.rs` (`Messages:`, `Notes:`), `run/edit_log.rs` (recipients). The binding 2026-09-26 spec §12 controls every state transition, limit and text, as decisions 42–42i resolve it; M9.2's `PlanEdit::{Message, Refresh}`, `MessageTarget` and `BlockReason::MessagePause` are the wire.

**Tests first.** Pure reducer tests in `run/engine/tests/worker_messages.rs`:
- `message_to_a_working_task_waits_for_the_turn_end` (queued while the turn is open; one `Deliver` after `TurnEnded`, text exact).
- `message_to_a_pending_task_is_in_its_first_prompt`; `rung_2_and_resumed_sessions_get_every_earlier_message`.
- `message_to_a_review_task_is_refused_for_it_alone` (the other recipient gets it; the reply lists both under `delivered` and `refused`); `message_whose_every_recipient_is_refused_rejects_the_call`.
- `message_mixed_with_a_plan_edit_is_refused_without_effect` (and with `submit` or `summary`; the run, its `digest_rev` and its outbox unchanged; the text is `message and refresh must be the only edit in their call`).
- `message_to_blocked_question_waits_for_the_answer` (one turn carries the answer and the message; the task stays blocked until `answer`).
- `message_to_check_is_delivered_only_if_the_gate_bounces`.
- `running_resolves_to_live_worker_rounds_and_skips_held_tasks`; `stage_recipient_is_refused_until_9_1`.
- `stop_and_wait_pauses_refuses_task_done_and_a_message_resumes`; `brief_amend_resumes_a_paused_task`; `resume_plan_edit_releases_paused_tasks`; `answer_to_a_paused_task_is_refused`; `paused_task_refuses_retry_override_and_route_amend`; `paused_task_holds_no_writer_slot_and_blocks_completion`.
- `a_pause_survives_restore_and_run_resume` (after `Restore` and M8a's `RunRequest::Resume` the task is still `blocked(message_pause)`; decision 42c).
- `messages_over_the_per_turn_limit_are_refused` (counted from `Task.orch.messages`, not the outbox); `message_max_per_turn_zero_refuses_every_recipient`; `message_limits_are_frozen_at_start` (`RunLimits.orch`, not the live config).
- `message_from_a_subplanner_is_refused`.
- `task_note_is_recorded_and_changes_no_state`; `task_note_from_a_paused_task_is_accepted`; `notes_are_capped` and `note_max_per_task_zero_refuses_every_note`; `discovery_and_risk_notes_wake_and_progress_does_not`.
- `paused_attention_only_after_ten_minutes`; `paused_block_adds_no_wake_note`.
- `edit_log_records_message_recipients_and_source`.
- In `headless/argv_tests.rs` (`:32` and `:261` pin the two-tool list today) and `run/engine/tests/dispatch.rs` (`:156`): `worker_spec_allows_task_note` (the worker's `--allowedTools` has `mcp__anthrex__task_note`).

Pure reducer tests in `run/engine/tests/refresh.rs`: `refresh_waits_for_the_turn_boundary`; `refresh_holds_a_change_message_sent_in_the_next_call_for_one_turn` (refresh in one call, then the `change` message in a second; both texts in one turn); `refresh_up_to_date_sends_nothing`; `refresh_failure_is_a_wake_note_and_no_block`; `refresh_is_refused_on_a_resolving_or_awaiting_task`; `a_refresh_merge_alone_is_not_work` (the zero-commit check and the fallback's no-commit nudge ignore `refresh_merges`).

Real git, in `crates/daemon/tests/run_git_handback.rs` and a new `crates/daemon/tests/run_refresh.rs`, through M8a's hand-back helpers: `refresh_merges_cleanly_and_the_next_turn_names_the_commits`; `refresh_conflict_leaves_markers_sets_resolving_and_does_not_count_a_conflict`; `refresh_with_uncommitted_changes_is_refused` (at acceptance, by the driver's pre-check, with the exact text); `refresh_dirty_at_the_boundary_fails_without_a_block`; `net_diff_excludes_refreshed_commits` (a two-file fixture whose merged file is outside `owns`: the spill list, `DiffStats` and `count_commits` after a clean refresh count only the task's own file); `refresh_never_rebases` (the task branch's old head is an ancestor of the new one); `refresh_is_replayed_by_reconcile` (in `crates/daemon/tests/run_journal/git_handback.rs`, with M8a's abort-after-intent hook). Every git invocation passes `--no-optional-locks` and a scrubbed environment (AGENTS.md rule 11), asserted by reading the argv a stand-in logs.

**Acceptance.** `run/engine/worker_messages.rs` and `run/edits_orch.rs` are pure. No message path interrupts a turn (`grep -n "Effect::Interrupt" crates/daemon/src/run/engine/worker_messages.rs crates/daemon/src/run/edits_orch.rs` prints nothing); no git operation under the manager lock (the pre-check runs on `spawn_blocking` with `DONE_CHECK_GIT_TIMEOUT`); no direct input to a headless window. All five AGENTS.md commands pass.

**Commit.** `feat(daemon): deliver worker messages, refresh task branches and record notes`

### M9.13b Routing history for orchestrators, planners, scouts and deciders

**Files.** Create `crates/daemon/src/run/orch/roles.rs` and `run/engine/tests/role_history.rs`. Modify `run/model.rs` (`Run.role_routing_decisions`), `run/engine/{orch.rs, planners.rs, restore.rs}` (capture at `CreateOrchestrator`, `RestartOrchestrator`, `StartPlanner`, `StartScout`; completion on the session-end events; `interrupted` on `Restore`), `run/orch/launch.rs` (`Resolved.candidates`), `run/driver/adapt.rs` (run-bound deciders: `OrchEvent::RoleRoute` before the call, `RoleRouteEnded` after), `run/driver/adapt_goal.rs` (pre-run triage appends its record with `history_io::append_line` on `spawn_blocking`), `run/history_io.rs` (the `RoleRoute` arm of the record id and `contains_record`), `run/engine/history.rs` (the arm), `run/stats.rs` (ignores `RoleRoute`), `run/reconcile/mod.rs` (the `AppendHistory` row, unchanged behaviour). Use M9.2's `RoleRoutingDecision` and `HistoryLine::RoleRoute`. The history file remains `<repo_dir>/history.jsonl` under anthrex's data directory; neither the repository nor an agent transcript receives this record.

**Tests first.**
- Pure, `run/orch/roles.rs`: `record_appends_the_chosen_route_when_absent`; `explicit_choice_is_identifiable_as_explicit` (source `explicit_choice`, even when the route is also the default); `unchosen_candidates_have_no_failure_label` (their `skipped_reason` is one of `not installed`, `not in the configured list`, `an earlier candidate was taken`, never a failure word); `record_ids_are_stable`.
- Pure reducer, `role_history.rs`: `each_role_keeps_its_dispatch_snapshot_after_a_config_change` (orchestrator, planner, run scout, run-bound decider); `orchestrator_restart_gets_a_new_session_id`; `planner_retry_gets_a_new_session_id`; `scout_retry_gets_a_new_session_id`; `decider_fallback_is_recorded_as_fallback`; `session_end_appends_one_line_with_the_outcome`; `scout_failed_on_restore_is_recorded_interrupted`; `restore_marks_every_open_record_interrupted_once`; `a_run_never_attributes_its_outcome_to_one_role`.
- Real, against a temporary repository and data dir (`crates/daemon/tests/history_io.rs`): `role_route_append_is_idempotent_by_record_id`; `version_1_history_and_an_old_run_json_still_load`; `pre_run_triage_writes_a_record_even_when_no_run_is_created`.
- `run/stats_tests.rs`: `stats_ignores_role_route_lines`.

**Acceptance.** The history is enough to join each role choice to its factual session outcome without reading transcripts. No history write under the manager lock or on a tokio worker thread. All five AGENTS.md commands pass.

**Commit.** `feat(daemon): record routing choices and outcomes for non-task agents`

### M9.13c `packed-refs.lock` under the worker sandbox (added 2026-09-27)

*Needs a controller ruling before it starts* (see "Implementation notes", "Left open at the refresh"). The followups file ("From the Claude tool-search fix") records that a real Claude worker's commit printed `Unable to create …/tasks/<t>/git/packed-refs.lock: Operation not permitted`, that the commit succeeded, that the sandbox was deliberately **not** widened (a writable `packed-refs` would let a worker forge refs in the task's repository), and that M9 owns the design decision. Task checkouts are M8a F1c's own repositories, whose engine-written config already sets `gc.auto=0` (`run/git/checkout.rs`).

**Files.** `crates/daemon/src/run/git/checkout.rs` (the engine-written config keys only), `crates/daemon/tests/run_git_checkout.rs`, `crates/daemon/tests/run_git_sandbox.rs`.

**Change** (the proposed resolution, pending the ruling). Add to the engine-written task-checkout config the key M9.1 item 11 finds stops the command that takes the lock (candidates: `maintenance.auto=false`; `gc.packRefs=false`). The sandbox is not widened, and `packed-refs` stays read-only to a worker. If M9.1 item 11 finds the lock comes from a command no config key stops (for example a worker's own `git branch -d`), stop, record the evidence under "Implementation notes", and leave the warning as it is.

**Tests first.** `task_checkout_config_stops_ref_packing` (the key is present in the checkout's config, read with `git config --get`); in `run_git_sandbox.rs` on macOS: `a_worker_commit_under_the_sandbox_prints_no_packed_refs_error` (M9.1 item 11's reproducing command, under the real seatbelt profile, with stderr asserted empty of `packed-refs.lock`), and `packed_refs_stays_read_only_to_a_worker` (pinning).

**Acceptance.** The sandbox's writable roots are unchanged (`role_launch::worker_git_roots`'s test passes untouched). The five AGENTS.md commands pass.

**Commit.** `fix(daemon): keep a sandboxed worker's git from packing refs it cannot write`

### M9.14 CLI

**Files.** Create `crates/cli/src/run_cmd/orch.rs`. Modify `crates/cli/src/run_cmd.rs` (the subcommand variants), `run_cmd/status.rs` and `run_cmd/status_tests.rs`, `run_cmd/adapt.rs` (M8b's goal and promote commands: `--orchestrator`), `crates/cli/tests/run_cli.rs` (M8a's CLI test file).

**Tests first**, in `run_cli.rs` against a real daemon:
- `orchestrator_flag_parses_and_refuses_bad_values` (exact message); `orchestrator_flag_is_refused_with_plan`.
- `start_goal_on_the_plan_path_prints_the_planned_message` (stderr exact, stdout the run id, exit 0).
- `approve_and_reject_hold`; `approve_without_hold_names_waiting_holds`.
- `run_message_reaches_the_selected_task_and_refuses_a_stage` (a task-id recipient gets the message; `stage:1` is refused with `stage recipients arrive with milestone 9.1`); `run_message_running_targets_only_live_workers`; `run_message_kind_defaults_to_info`; `run_message_and_refresh_record_source_user` (the edit log's entry has source `user` and the recipients; a refused recipient's line is printed); `run_refresh_refuses_uncommitted_work` (the exact text).
- `message_and_refresh_help_names_the_kinds` (the help names `info`, `change` and `stop_and_wait`).
- `status_shows_orchestrator_planners_holds_summary_and_paused_lines` (exact lines); `status_json_carries_the_new_fields`.
- `accept_prints_the_research_report_and_the_nothing_to_merge_question`.
- `promote_with_orchestrator_choice`; `promote_twice_answers_without_a_time`.

**Acceptance.** `crates/cli/src/main.rs` unchanged. The five AGENTS.md commands pass.

**Commit.** `feat(cli): start planned goals, approve holds, message and refresh tasks, and show the orchestrator in run status`

### M9.15 TUI

**Files.** Create `crates/tui/src/run_goal.rs` (the pure goal form), `crates/tui/src/ui/run_goal.rs` (its renderer), `crates/tui/src/conversation_label.rs` (decision 42i's recogniser), and `crates/tui/src/app_tests/run_goal.rs`. Modify `keymap.rs` (`C-b g`, `Command::StartGoal`) and `keymap_tests.rs`, `app/mod.rs` (`Modal::StartGoal`), `ui/modal.rs` (one arm), `app/runs.rs` (the hold keys, `pending_open`, the tagged replies of decision 2 for the edit form), `app/run_enter.rs`, `app/headless.rs` (decision 11's refusal, if M9.10 did not already land it), `tree/runs.rs`, `tree/run_rows.rs`, `inspector/run.rs`, `inspector/run_task.rs`, `inspector/run_round.rs`, `theme.rs`, `graph/run_text.rs`, `graph/paint/style.rs`, `ui/conversation.rs` (one call), and their tests.

**Change.** A `planning` run shows `planning` in its header and its orchestrator node; `Reported` tasks get `✓` in the finished colour and the label `reported`; a held task shows `○` and `held` after its stage; a `paused(message)` task shows `‖` in its own colour and the stage text `paused (message)`, and the run's attention line lists it after 600 s; an awaiting gate hold appears in the run's attention list as `hold <id>: <n> tasks wait for approval`, and `a` / `x` send `ApproveHold` / `RejectHold` (with the M8c confirmation prompt for `x`) when the selected node is a held task, or the run's root while exactly one hold is awaiting (with several, the root's `a` toasts `select a held task to approve its hold`); planner nodes use `PlannerInfo` with M8c's content text (`planner {epic} {title}  {merged}/{total}`) and planner glyphs, now filled (decision 33); a research session's round label is `research #n`; the task inspector gains the `messages` row (count, latest kind and first line) and the task's `discovery`/`risk` notes with attribution (decision 42i); a delivered message's user turn in the conversation view is labelled `orchestrator` or `user` from its prefix, with no protocol change. Enter on the run's root already focuses the orchestrator's PTY window (M8c, `app_tests/overview/run_enter.rs:43`); M9 keeps it. `C-b g` opens decision 44's goal form; the M8c edit form and the goal form send tagged requests and match replies by id (decision 2). The reducer stays pure (AGENTS.md rule 5): each key returns `Effect`s.

**Tests first.** `planning_run_header`; `reported_held_and_paused_glyphs`; `hold_attention_line_and_keys_emit_the_requests` (held-task node; root with one awaiting hold; root with two toasts); `planner_nodes_from_planner_info`; `research_round_label`; `messages_row`; `message_pause_and_notes_render_with_attribution`; `delivered_message_turn_is_labelled_by_source` (and a user turn without the prefix is unchanged); `enter_on_the_root_focuses_the_orchestrator` (**pinning**, M8c's test, kept); `goal_form_requires_a_selected_project` (the toast, no effect); `goal_form_sends_start_goal_and_opens_the_run` (a tagged `StartGoal` with `yes: false`, `unconfined_checks: false`, the chosen runtime and model); `goal_form_opens_the_view_only_once_the_snapshot_names_the_run` (`Triaged` first, snapshot second); `goal_form_keeps_its_input_on_error`; `a_refusal_for_an_earlier_request_does_not_reach_the_form` (a `Refused` with another `request_id`); `kill_and_remove_of_a_live_orchestrator_open_nothing` (if not already in M9.10); M8c's mockup tests updated only where a new field appears, each change listed in "Implementation notes".

**Acceptance.** `crates/tui/src/app/`, `ui/**`, `run_goal.rs` and `conversation_label.rs` do no I/O. `conversation.rs` (595) is not touched. The five AGENTS.md commands pass.

**Commit.** `feat(tui): show planning runs, holds, planners, paused and reported tasks, label delivered messages, and start a goal with C-b g`

### M9.16 End-to-end I: the plan path, steering, reactions and messages

**Files.** Create `crates/cli/tests/run_e2e_orch.rs`, `crates/cli/tests/support/run_orch.rs`. Modify `crates/cli/tests/support/mod.rs` (one `mod` line), `docs/timing-budgets.md` (`ORCH_WAIT`), and M8b's tests that encode the refusal and the recorded-only promotion.

**Earlier-brief changes (M8b; the corrected list, Risk 8).** Five end-to-end tests, three unit tests and one exact-text test encode M8b's refusal or recorded-only promotion; each is rewritten here or in M9.7, and nothing else of M8b's suite changes:
- `crates/cli/tests/run_e2e_adapt.rs`: `e2e_goal_needing_a_plan_is_refused_without_side_effects` (`:145`) becomes `e2e_goal_needing_a_plan_starts_a_planning_run`; `e2e_goal_touching_a_hub_file_is_refused_without_side_effects` (`:164`) becomes `e2e_goal_touching_a_hub_file_takes_the_plan_path`; `e2e_goal_owning_a_protected_file_takes_the_plan_path` (`:177`) now expects a planning run instead of the refusal; `e2e_goal_without_deciders_takes_the_plan_path` (`:187`) now expects an orchestrator window; `e2e_promote_records_intent_and_the_task_continues` (`:271`) now expects the orchestrator and the promotion hold, and its repeat assertion (`:307`, `was already marked for promotion at`) loses the time.
- `crates/daemon/src/run/driver/adapt_goal_tests.rs:125` (the hub goal's refusal) keeps the hub reason in the triage text and now expects a planned run.
- `crates/daemon/src/run/triage_tests.rs`: `messages_are_exact` (`:302`) loses its two `refused_message` cases (`:317`, `:335`) with `refused_message` itself (`triage.rs:356`), which decision 26 leaves unused; `planned_message`'s exact text is M9.7's test.
- `engine/tests/fast_path.rs`'s two promote tests are M9.7's.

**Tests first.** Every scenario uses scripted `fake-agent`s (orchestrator PTY scripts, headless workers and reviewers), and `mcp_log()` for what the orchestrator saw.
- `e2e_plan_path_scouts_plans_and_reads_the_approval`: triage `plan`. `orchestrator-run-1`: `get_context`; `spawn_scout {id: "core", …}`; `mcp_until run_status` until `/scouts/0/state == "reported"`; `edit_plan` with two S tasks naming `<h4>-core` in `scout_refs` and `submit: true`, `expect /awaiting_approval == true`; `mcp_until run_status` until `/gate/state == "approved"`; `mcp_until run_status` until `/run/complete == true`; `edit_plan {edits: [], summary: "…"}`. The test waits for `awaiting_approval`, sends `run approve`, waits for `complete`; asserts both tasks merged, `REPORT.md` starts with the summary section, and the orchestrator's `run_status` calls each returned within `wait_secs + 5` s.
- `e2e_rejected_plan_discards_the_run_and_the_orchestrator_learns_it`: after submit the test sends `run reject`; the script's `mcp_until` sees `/run/state == "discarded"`, then an `edit_plan` with `expect_error` containing `is discarded`; afterwards `anthrex rm <orchestrator window>` succeeds.
- `e2e_typed_steering_becomes_a_plan_edit`: after approval the script does `read_message {expect: "skip t2"}` then `edit_plan [cancel_task t2]`. The test `type_into`s `skip t2\r`; asserts `t2` cancelled with an edit-log entry of source `orchestrator`.
- `e2e_mis_sized_task_is_split_by_the_orchestrator`: `worker-t1-*` scripts fail twice so `t1` reaches rung 3; the orchestrator's `read_message {expect: "t1 blocked (mis_sized)"}` (the wake) then `split_task t1 into [t1a, t1b]`; both merge.
- `e2e_blocked_question_is_answered_after_a_wake`: the worker calls `task_blocked {reason: "question", …}` then `read_message {expect: "use tabs"}`; the orchestrator's `read_message {expect: "t1 blocked (question)"}` then `edit_plan [answer t1 "use tabs"]`; `t1` merges. The stdin file of the orchestrator shows the paste framing.
- `e2e_promote_starts_an_orchestrator_and_holds_its_tasks`: a fast-path run whose `t1` waits on `read_message`; `run promote`; an orchestrator window appears; its script adds `t2` and submits; `t2` shows hold `promotion` awaiting; `t1` keeps running and merges; `run approve --hold promotion` releases `t2`.
- `e2e_orchestrator_cannot_approve_merge_or_write`: the script calls `edit_plan` with `{"op": "override", …}` (error text asserted), `task_done` (refused by `anthrex mcp` for the role), and the fake runtime's argv shows `--disallowedTools Edit,Write,NotebookEdit,Bash,Agent`; the run branch has no commit the orchestrator could have made.
- `e2e_wake_does_not_collide_with_typing`: while the test types into the orchestrator every 300 ms for 3 s (`wake_quiet_secs = 1`), a task blocks; no paste reaches the window until 1 s after the typing stops (asserted from the stdin file's timestamps written by `fake-agent`).
- `e2e_message_refresh_and_task_note` (TT §12.7): `t0` (an interface file) merges while `t1` works; the orchestrator calls `edit_plan` with `refresh t1`, then, in a **second** call, `message {to: ["t1"], kind: "change"}` (decision 42: each alone in its call); `t1`'s next turn has both texts and its checkout has `t0`'s file; its `task_done` summary starts `Changes applied:`; `t1` merges, and its measured diff (`TaskInfo.diff`) counts only its own file. `t2`'s worker calls `task_note {kind: "discovery", …}` and keeps working; the orchestrator's `read_message` gets a wake naming the note, and the digest's `task_notes` has it with `t2`'s attribution. `t3` gets `stop_and_wait`; it is `blocked(message_pause)`; its worker's `task_done` is refused with the exact text; a later `info` message resumes it and it merges. The event log shows no delivery while a turn was open.
- `e2e_messages_are_not_redelivered_after_a_restart` (TT §12.7): a message delivered to `t1`, then `restart_daemon` and `run resume`; the worker's stdin file has the message once; a paused task is still `paused(message)` after the resume (decision 42c).
- `e2e_user_run_message_is_recorded_with_source_user`: `anthrex run message <run> t1 --kind info "use lowercase"`; the worker gets `[anthrex] Message from the user (info): use lowercase`; the edit log's entry has source `user` and recipients `["t1"]`.
- `e2e_goal_form_request_matches_the_cli` (spec §16): a tagged `StartGoal` sent over a real socket as the goal form sends it (`yes: false`, `unconfined_checks` as the test platform needs, an orchestrator choice) starts the same planning run as `anthrex run start --goal`, and the reply echoes the id. The keystroke path (`C-b g`, typing, Enter, the run view opening) is smoke stage 11f's macOS branch (M9.17) and M9.15's pure tests.
- `e2e_role_routing_records_survive_restart` (decision 43): the orchestrator's and a run scout's records keep their dispatch-time candidate snapshots and distinct session ids in `history.jsonl` after `restart_daemon`; the scout that was running is recorded `interrupted`; M8b's task-history aggregates (`run stats`) are unchanged.

**Acceptance.** All pass three times in a row (`cargo test -p anthrex --test run_e2e_orch -- --test-threads=1`, repeated), on macOS and, in CI, on Linux. The five AGENTS.md commands pass.

**Commit.** `test: cover the plan path, the gate, steering, reactions and worker messages end to end`

### M9.17 End-to-end II: the large path, kinds, restart, metering and the smoke stage

**Files.** Create `crates/cli/tests/run_e2e_large.rs`, `scripts/pty_smoke_orch.py`. Modify `scripts/pty-smoke.py` (one import beside `:46-47`, one call after `:1718`), `docs/timing-budgets.md` (any new bound), `docs/ROADMAP.md` (milestone 9 `done`), `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` ("From milestone 9", the follow-ups of this brief, and the entries this milestone handled marked handled).

**Tests first.**
- `e2e_large_path_two_subplanners_submit_their_epics`: triage `large`. The orchestrator adds interface task `t0` (hub), `spawn_subplanner a` (area `src/a/**`) and `b` (`src/b/**`), waits with `mcp_until run_status` for `/planners/*/state == "finished"` (two pointers), then submits. `planner-a-1` and `planner-b-1` call `get_context`, then `submit_epic` with tasks depending on `t0`. The run completes; `RunInfo.planners` has both with `edits_accepted`; both planner windows were removed after `RETIRE_AFTER`.
- `e2e_subplanner_edit_outside_its_area_is_rejected_then_fixed`: `planner-a-1` first submits a task owning `src/b/x.rs` (`expect_error_contains "src/b/x.rs"` and the area wording), then a corrected batch; the digest shows `rejected: 1` and `last_rejection`.
- `e2e_new_epic_after_approval_is_held_until_approved`: after approval the orchestrator spawns epic `c`; its tasks show hold `epic:c` awaiting; the other epics' tasks keep running; `run approve --hold epic:c` releases them; the orchestrator's `run_status` sees the hold `approved`.
- `e2e_integration_review_asks_for_changes_and_a_fix_task_closes_it`: `reviewer-a-int1-1` (the engine-made review task `a-int1`, decision 37) submits `changes` with one critical finding; the run is not complete; the orchestrator's wake reads `integration review of epic a: changes (1 critical, 0 important)`; it adds fix task `a9` in epic `a`; after `a9` merges, `reviewer-a-int2-1` approves; the run completes.
- `e2e_research_goal_reports_without_merging`: triage kinds `[research]`; the orchestrator adds one research task; `scout-t1-1` submits a report; `t1` is `reported`; `run accept` prints `research report: <path>` and asks the nothing-to-merge question; the base branch is unchanged.
- `e2e_review_goal_reviews_a_range_without_merging`: the repository has a branch `feature` two commits ahead; the orchestrator adds a review task with `review_target = "main..feature"`; the reviewer's prompt names both shas; its `changes` verdict still ends `reported`; the findings are in `REPORT.md`; nothing merged.
- `e2e_restart_resumes_the_orchestrator_with_its_role_flags`: during `running`, `restart_daemon`; the run is `paused`; `run resume`; the orchestrator's args file has a second line with `--resume` and every role flag; its first wake contains `the daemon restarted`.
- `e2e_claude_orchestrator_gets_the_otlp_environment`: with M8b's receiver running (`<data_dir>/otlp.addr` exists), the orchestrator's environment file has M8b's OTLP variables, `anthrex.role=orchestrator`, the bearer header and `ENABLE_TOOL_SEARCH=false`; a Codex orchestrator's has none of them, and `run status --json` has no orchestrator usage.
- `e2e_role_history_for_large_and_triage_paths` (decision 43): the pre-run triage decider, the orchestrator, two sub-planners and a run scout each have one `role_route` record with its candidate snapshot and factual outcome; a direct triage test whose run creation then fails (a missing runtime binary) still has its triage record with no `run_id`; candidates not chosen carry no failure label.

**Smoke stage 11f.** `scripts/pty_smoke_orch.py::orch_stage(PtyProc, bin_path, run_cmd, fail)`, M8c's `run_view_stage` injection pattern (`pty_smoke_run_view.py:100`), called from `pty-smoke.py` as `orch_stage(PtyProc, BIN, run_cmd, fail)` right after `run_view_stage(PtyProc, BIN, run_cmd, fail)` (`:1718`) and before `== stage 12` (`:1720`); the import sits beside `from pty_smoke_adapt import DECIDER_DIR, adapt_stage` (`:46`). It prints `== stage 11f: a goal is planned by a scripted orchestrator ==`. It imports `_git`, `_write_script`, `RUN_WAIT` and `RUN_CMD_TIMEOUT` from `pty_smoke_run`, and `DECIDER_DIR` and `GOAL_CMD_TIMEOUT` (900 s, `pty_smoke_adapt.py:40`) from `pty_smoke_adapt`.

1. Setup: its own repository `/tmp/anthrex-smoke-orch-<pid>` with a local identity, `commit.gpgsign false` and one commit; a stored profile written as stage 11d writes it (`pty_smoke_adapt.py:101-117`), because a goal needs one (M8b decision 22 step 2). `os.makedirs(DECIDER_DIR, exist_ok=True)` first, because stage 11d removes the directory in its `finally` (`pty_smoke_adapt.py:155`); then `DECIDER_DIR/triage-1.json`, a JSON answer file that `fake-agent`'s decider mode claims (`crates/fake-agent/src/decider.rs`), answering `plan`. `_write_script(repo, "orchestrator-run-1", …)`: `get_context`, `edit_plan` with two S tasks and `submit: true`, `mcp_until run_status /gate/state == approved`, `mcp_until /run/complete == true`, `read_message {expect: "done"}`, `edit_plan {edits: [], summary}`; the worker and reviewer scripts for both tasks.
2. Start the goal (decision 44, controller's ruling D-9). **On macOS**, through the TUI: the stage's own `PtyProc([bin_path])`, select the repository's project node, `C-b g`, type `add two files`, `Enter`, and wait (up to `GOAL_CMD_TIMEOUT`) for the run view to open on the new run. **Off macOS**, where a run needs `--unconfined-checks` and the goal form has no such toggle: `run start --goal "add two files" --dir <repo> --unconfined-checks` with `GOAL_CMD_TIMEOUT`, then the stage's own `PtyProc`, `C-b T`, `/`, the run's `h4`, `Enter`. Either way, wait for the run's `planning` header.
3. Wait for `awaiting approval`, then `a`, then `y`.
4. Wait up to `RUN_WAIT` for `complete`.
5. Enter on the root (M8c), which focuses the orchestrator window. Type `done\r`.
6. Wait for the run's `summary: written` through `run status`.
7. `C-b d`; `run accept --yes`; both files on `main`.
8. `finally`: close the client, and remove the repository and `DECIDER_DIR`.

Every wait is a deadline loop (`RUN_WAIT`, `RUN_CMD_TIMEOUT`, `GOAL_CMD_TIMEOUT`); any new bound gets its row in `docs/timing-budgets.md`. TT §12 is covered by the end-to-end tests, not the smoke stage. The smoke daemon runs with CI's `ANTHREX_GIT=off`, like every stage.

**Acceptance.** `python3 scripts/pty-smoke.py` passes with stage 11f, on macOS and in CI on Linux. After it, `pgrep -fl "anthrex daemon"` and `pgrep -fl fake-agent` show nothing of yours, and `/tmp/anthrex-smoke-orch-*` is gone. All five AGENTS.md commands pass. Milestone 9 is `done` in `docs/ROADMAP.md`.

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

- `grep -rn "std::process\|std::fs\|tokio::process\|\.lock()" crates/daemon/src/run/orch crates/daemon/src/run/edits_orch.rs crates/daemon/src/run/engine/{orch,gate_holds,promote,planners,kinds,worker_messages}.rs crates/daemon/src/launch/role.rs` prints nothing (decision 1's purity).
- `grep -rn "\.lock()\.unwrap()" crates/daemon/src` prints nothing new (AGENTS.md rule 3).
- `grep -rn -- "--no-optional-locks" crates/daemon/src/run/git/summary.rs crates/daemon/src/run/driver/refresh.rs` shows one per git invocation, and the refresh's log read in `run/git/handback.rs` goes through `worktree::run_git` (AGENTS.md rule 11).
- `wc -l` of every file in "File sizes this milestone must respect" is within its budget, and no new file passes 600 lines.
- `cargo +1.98 clippy --workspace --all-targets -- -D warnings` and `cargo +1.98 clippy --workspace --all-targets --target x86_64-unknown-linux-gnu -- -D warnings` pass (CI runs clippy 1.98 on macOS and Linux; a macOS-only import or errno is invisible to the host target).
- Before merging, the full suite passes once with `GIT_CONFIG_GLOBAL=/dev/null` (no global identity, as on CI) and once with dash as `sh`, so a test that relies on a global git identity or on bash-only `sh` fails locally first. Per round, only the touched tests run; the full suite and the smoke run before the merge.
- `pgrep -fl "anthrex daemon"` and `pgrep -fl fake-agent` show nothing of yours.

## Manual check

With real `claude` and `codex` installed and logged in. Never run `anthrex` without the two variables (AGENTS.md rule 1).

```bash
export ANTHREX_SOCKET=/tmp/ax-m9/d.sock ANTHREX_DATA_DIR=/tmp/ax-m9/data
mkdir -p /tmp/ax-m9 && cargo new --vcs git /tmp/ax-m9/repo && cd /tmp/ax-m9/repo && git add -A && git commit -qm init
anthrex daemon start
anthrex profile detect && anthrex profile confirm        # M8b
anthrex run start --goal "Add a --json flag that prints the greeting as a JSON object, with a test and a README section"
anthrex                                                   # C-b T, Enter on the run's root (its orchestrator)
```

Check and record under "Implementation notes":

1. The orchestrator calls `get_context`, spawns at least one scout, and writes S or M tasks with `scout_refs`, test modes with reasons, and routes; no task has a budget; no L survives.
2. It submits, and waits in `run_status` rather than asking you to approve in chat. Approve with `a` in the run view; it notices within a minute.
3. Type `skip the docs task` into its window; it cancels that task with one line of explanation.
4. Ask it to write a file or run a command; it cannot.
5. When a task blocks (edit a worker's brief to be ambiguous if none does), you see a wake line and its reaction.
6. **Tool visibility.** In `anthrex run status <run> --json` and the task histories, no worker received M8a's turn-end fallback nudge (`DONE_NUDGE` or `NO_COMMIT_NUDGE`, `run/contract.rs:266-270`, visible in its conversation view): every real worker called `task_done` itself. The orchestrator's `/mcp` lists `anthrex` with its six tools, and its calls to `submit`/`run_status` reach the daemon (a sub-planner's `submit_epic` too, in step 9).
7. *TT §12:* while a task works, run `anthrex run message <run> <task> --kind stop_and_wait "hold on"`; the task shows `paused (message)` after its turn ends; then `anthrex run message <run> <task> "carry on"` resumes it. Ask the orchestrator to tell the running workers something; it uses `message`, not `amend_task`. If a task reports a `task_note`, the orchestrator reacts to it.
8. When the run completes, it writes a summary. `anthrex run accept <run>`.
9. Repeat once with `--orchestrator codex` (with `~/.codex/config.toml` hashed before and after), once with a goal that the triage labels `large` (for example "Add a Gemini runtime alongside Claude and Codex") far enough to see two sub-planners submit, and once started from the TUI with `C-b g` on the repository's project node.
10. `anthrex daemon stop` with the same variables, then `pgrep -fl "anthrex daemon"` shows nothing of yours.

```bash
anthrex daemon stop
```

## Review focus

Seven failure modes the reviewer checks first, each with the test that covers it in its owning task:

1. **A wake pasted into the user's typing or into a permission prompt.** The paste lands only when the window is `Idle` or `Done` and quiet for `wake_quiet_secs`. Tests: `wake_is_delivered_only_when_idle_and_quiet`, `wake_is_not_delivered_on_attention` (M9.13), `e2e_wake_does_not_collide_with_typing` (M9.16).
2. **A long-poll that never waits, or never wakes.** If counters move the digest revision, every worker's tool call ends the orchestrator's wait and it spins; if the fingerprint misses a field, a blocked task never reaches it. Tests: `counter_changes_do_not_change_the_fingerprint`, `state_block_verdict_hold_scout_planner_note_message_and_edit_changes_do` (M9.6), `a_counter_change_does_not_end_the_wait` (M9.11).
3. **A restarted orchestrator without its role flags.** Resume restores none of them; a plain `claude --resume` would be a writable session with no MCP tools. Tests: `restart_repasses_role_flags_with_resume`, `role_survives_a_daemon_restart` (M9.10), `e2e_restart_resumes_the_orchestrator_with_its_role_flags` (M9.17).
4. **A sub-planner that loops or goes silent.** Repeated rejections, turns without a submit, tool-call runaways and hangs must all end in a failed planner the orchestrator is told about, never a run stuck in `planning`. Tests: `rejections_count_and_fail_at_max_rejections`, `turn_without_submit_is_nudged_once_then_fails`, `tool_call_wrap_up_then_kill`, `timeout_fails`, `failed_planner_keeps_accepted_tasks_and_wakes_the_orchestrator` (M9.8).
5. **Unbounded replies and pushes.** A 50-task run, long block texts, many notes and many scout reports must not push a tool reply past what the agent can take, and fifty finished runs must not push their briefs to every client. Tests: `digest_is_capped_and_trims_in_order`, `context_is_capped_at_96_kib_with_omitted_counts`, `task_result_is_capped`, `a_snapshot_of_fifty_terminal_runs_stays_small` (M9.6).
6. **A message that interrupts, or changes, a task** (TT §12). A message must never cut a turn short and never carry scope; `stop_and_wait` must stop `task_done` at once and hold until a message, an amend or the run's `resume`. Tests: `message_to_a_working_task_waits_for_the_turn_end`, `message_mixed_with_a_plan_edit_is_refused_without_effect`, `stop_and_wait_pauses_refuses_task_done_and_a_message_resumes`, `paused_task_refuses_retry_override_and_route_amend` (M9.13a), `e2e_message_refresh_and_task_note` (M9.16).
7. **A refresh that rebases, merges a dirty tree, or counts as the task's work** (TT §12.2). Tests: `refresh_never_rebases`, `refresh_with_uncommitted_changes_is_refused`, `refresh_dirty_at_the_boundary_fails_without_a_block`, `net_diff_excludes_refreshed_commits`, `a_refresh_merge_alone_is_not_work`, `refresh_conflict_leaves_markers_sets_resolving_and_does_not_count_a_conflict` (M9.13a).

## Risks and gotchas

1. **Interactive TUIs are not stream-json.** The engine learns the orchestrator's state only from hooks (`Stop`, `StopFailure`, `UserPromptSubmit`) and M3's status machine. A turn that ends without either hook leaves the window `Working` and wake-ups wait; `StopFailure` (decision 12) closes the known gap, and M9.1 item 5 checks `/compact`.
2. **`--disallowedTools` is the read-only guarantee for Claude.** If a future Claude Code version lets a disallowed tool through, the orchestrator could write to the user's checkout. M9.1 item 2 checks it once; the manual check repeats it.
3. **Codex read-only and the MCP socket.** Codex's `read-only` sandbox may block the MCP server's socket connection on some versions; M9.1 item 4 records it. Without it, the Codex orchestrator has no tools.
4. **Bracketed paste and `\r` timing.** If `SUBMIT_DELAY` is too short, the `\r` lands inside the paste and the message is not submitted; the wake then sits in the input box. M9.1 item 6 measures it on both TUIs.
5. **Wake storms.** A busy run changes the digest often. One pending wake per run, delivered only on idle and quiet, and cleared by a `run_status` read, bounds this to at most one paste per orchestrator turn.
6. **Approval by habit.** Models may ask the user "shall I approve?" in chat. The contract says the gate is in the run view (rule 8), and no tool can approve; the manual check looks for it.
7. **The cap versus a real large goal.** A goal with 40 tasks needs at least three sub-planners at the default cap of 12. If the orchestrator writes 13 tasks itself, the cap rejects the batch with a message that names sub-planners.
8. **Stale M8b tests.** Five M8b end-to-end tests (`run_e2e_adapt.rs:145, :164, :177, :187, :271`, and `:307`'s repeat text), three unit tests (`engine/tests/fast_path.rs`' `promote_records_intent_once` and `promote_refuses_plan_runs_and_terminal_runs`; `driver/adapt_goal_tests.rs:125`), and the two `refused_message` cases of `triage_tests.rs::messages_are_exact` encode the refusal and the recorded-only promotion. They are rewritten in M9.7 and M9.16 (the list is in M9.16), and nothing else of M8b's suite may change.
9. **User settings only.** A Codex orchestrator in a repository with tracked `.codex` configuration is refused without `--trust-project`, because `CLI_CAPS.codex_user_config_only` is `None` (`headless/argv.rs:100`; decision 9); a Claude orchestrator is not, because `claude_user_settings_only` is set (`:108`). This is intentional but surprising the first time.
10. **Socket paths.** Every new test directory is under `/tmp` (`tempfile::Builder::new().prefix("ax-orch").tempdir_in("/tmp")`).
11. **Linux CI.** Ubuntu's `/bin/sh` is dash, so a stand-in script that uses bash syntax, or a test that expects bash's exit code for a missing command, fails only on CI. A stand-in that exits before reading its stdin makes the daemon's write fail with a broken pipe on Linux timing. CI sets no global git identity, so any repository that `preflight` reads needs a local identity. A script written and executed at once can hit `ETXTBSY` under parallel tests on Linux. See Verification.
12. **Readers see the user's checkout, not the base** (decision 20a). A scout or sub-planner may describe uncommitted or newer files than the run's base. The prompts name the base.
13. **Protected lookalikes are strict** (decision 23a). Every changed path with a non-ASCII character now needs an exact `owns` entry. A repository with many non-ASCII file names will see more bounces.
14. **Messages and the outbox's holds interact.** A message to a task in `check` or `proof` waits until a gate bounces it, and a message to `blocked(question)` waits for the answer; an orchestrator that expects immediate delivery will be surprised. The digest's `messages.undelivered` shows it, and the contract says a message arrives when the worker's turn ends.
15. **MCP tool visibility in Claude** (the tool-search fix). Claude Code defers MCP tools behind `ToolSearch`, and a loose query can miss them; a worker that cannot see `task_done` ends by the turn-end fallback. Every headless Claude session and the Claude orchestrator run with `ENABLE_TOOL_SEARCH=false` (decision 10), and M9.1 item 10 and manual check 6 re-verify it against the real CLI. A future Claude Code that ignores the variable would bring the failure back silently; the manual check is the guard.
16. **`packed-refs.lock` warnings** under the worker sandbox (task M9.13c). Harmless to the commit, but an `error:` line may lead a model to "fix" something. The sandbox is not widened to silence it.

## Follow-ups handled

| Follow-up | Where it was recorded | Handled by |
|-----------|----------------------|------------|
| `StopFailure` not mapped for PTY Claude windows | M8a Out table and follow-up | Decision 12, task M9.10 |
| Research and review kinds deferred | M8a decision 6 | Decisions 24, 35, 36, tasks M9.4, M9.9 |
| Goals on the plan and large paths refused | M8b decision 22 step 6 | Decision 26, task M9.13 |
| `run promote` only recorded; its repeat reply printed a UTC time next to local times | M8b decision 25; followups file, M8c | Decision 29, task M9.7 |
| OTLP receiver waiting for the orchestrator; `Expect: 100-continue` | M8b decision 30; followups file, "From M8b.15" | Decisions 10, 14, tasks M9.1 item 7, M9.13, M9.17 |
| Orchestrator usage is not authenticated | Followups file, "From M8b.15's review" | Decision 14a, tasks M9.10, M9.13 |
| Size the OTLP connection cap from concurrent runs | Followups file, "From M8b.15's review" | Decision 14b, task M9.10 |
| `engine/signals.rs` sums usage with plain `+=` | Followups file, "From M8b.15" | Task M9.9 |
| Non-ASCII case folding in `ProtectedMatcher` | Followups file, "From M8b's whole-branch re-review" | Decision 23a, task M9.4 |
| `EditScope::Area` limits only `owns` | Followups file, M8a.6 | Decision 22, task M9.8 (`submit_epic_cannot_cancel_or_amend_another_epics_task`) |
| No `remove_dep`/`replace_dep` | Followups file, M8a.6 | Decision 25 (`amend_task deps`), task M9.4 |
| Read-only roles under a read-only OS sandbox | Followups file, F1 N4 | Decisions 31, 35, task M9.8 (`planner_and_research_specs_carry_the_read_only_sandbox`) |
| `RunInfo.planners` placeholder | M8c "Consumes from later milestones" | Decision 33, task M9.8 |
| Triage's "2 to 12 tasks" constant | M8b `PLAN_SCALE_MAX` | Decision 3, task M9.3 |
| Every run's task briefs ride on every snapshot push | M8c whole-branch review | Decision 16a, task M9.6 |
| Replies matched by request name only | Followups file, M8c.9 review M7 | Decision 2 (request ids), tasks M9.2, M9.15 |
| A restored headless record whose `run` does not parse comes back as a PTY window | M8c Risk 9 | Decision 11a, task M9.10 |
| A real Claude worker could not see `task_done` | Followups file, "From the Claude tool-search fix" | Fixed on `fix-claude-tool-search`; decision 10 extends it to the orchestrator, decision 42f to `task_note`; M9.1 item 10 re-verifies |
| `packed-refs.lock` under the worker sandbox (owner M9) | Followups file, "From the Claude tool-search fix" | Task M9.13c (pending the controller's ruling) |
| `headless/conversation.rs` over its budget (452 of 430) | M8c file-size table | Not touched by M9; a task that must touch it moves the turn-end handling out first (file-size table) |

Not taken, and why:

- A leftover scout after a crash is not killed: kept as is for run scouts and planners (decisions 20, 32), because the process-kill rule allows only an exact recorded pid.
- The run inspector's `doing` tool target (M8c Risks 6): stays a follow-up (Out table).
- The profile TUI form: stays out (Out table).
- The global OTLP series cap: stays a follow-up (Out table).
- `RunInfo.estimate_left_secs` and `bound_ratio_permille`: M9.5's (Out table).

Recorded as new follow-ups in task M9.17: OTLP metering of a Codex orchestrator (decision 14); wake delivery for a Codex orchestrator if M9.1 item 6 finds its paste handling unreliable; Unicode case folding and normalisation for protected paths with a crate, replacing decision 23a's refusal; moving readers to a scratch checkout at `base_sha` if Risk 12 bites; an `--unconfined-checks` toggle for the goal form if Linux users need it (decision 44).

## Spec and brief defects

Found while writing this brief. Each is resolved by the decision named. Items 1–10, 21 and 22 were then written into the spec, in the section each belongs to, marked "settled in the briefs, 2026-09-22"; items 11–20 are between briefs and resolved in them. Items 23–31 come from TT §12 (PR #18's contradictions C1–C5 and C9, and three of its own) and are resolved here by the refresh of 2026-09-27; they are not yet written back into that spec.

1. **§12.1 has no submit boundary.** It lets the orchestrator edit the plan freely but never says when the plan is "the plan" the gate shows. Resolved by `edit_plan { submit: true }` (decision 27).
2. **§12.3 has no state for planning, and no mechanism for approving a new epic mid-run.** "Non-blocking" plus "any later edit that adds an epic" waits for approval needs somewhere for the waiting tasks to be. Resolved by `RunState::Planning` (decision 26) and holds (decision 28).
3. **§5.2 and §18: research and review tasks have no terminal state and review has no target field.** Resolved by `TaskState::Reported` and `PlanTask.review_target` (decisions 24, 35, 36).
4. **§5.3 step 6 does not say what a rejected integration review does.** Resolved by decision 37: completion waits for a fix task or `finish`; nobody approves by edit.
5. **§10 rung 3 raises a task to L, but §11.4 step 5 lets the orchestrator "rewrite" it.** Resolved by decision 25: the rewrite is a new size claim, re-checked by M8a's rules.
6. **§3 says the orchestrator never reads checkouts, but §4 gives it the repository to read.** Resolved by decisions 5, 18 and 20a: it reads the user's own checkout (its cwd) and never a task worktree; `task_result` replaces reading one.
7. **§14.4 makes the digest pull-only, but an interactive session whose turn has ended pulls nothing.** Resolved by wake-ups (decision 39).
8. **§4.2 refuses kill and remove for run sessions without saying how a dead orchestrator comes back.** Resolved by decision 11 (`anthrex restart`, and `RestartOrchestrator` on resume) and decision 13 (the run never waits for it).
9. **§14.8 meters only Claude Code's OTLP export.** A Codex orchestrator is unmetered (decision 14, follow-up).
10. **§13 item 3's readers list scouts, reviewers and deciders, not sub-planners.** Resolved by decision 31: planners take reader slots, in a stated order.
11. **Smoke stage numbers collided.** Resolved in the briefs (2026-09-22): M8a `11c`, M8b `11d`, M8c `11e`, M9 `11f`, M9.5 `11g`.
12. **M8c's `ScoutInfo` and `ScoutState` differed from M8b's.** Resolved in the briefs (2026-09-22): M8c uses M8b's types exactly. The snapshot carries them (decision 20); the digest keeps this milestone's `RunScoutState` labels.
13. **M8c's `TaskInfo.diffstat` versus M8b's `TaskInfo.diff`.** Resolved in the briefs (2026-09-22): M8c uses M8b's `TaskInfo.diff: Option<DiffStats>`. `task_result` uses the git diffstat text (decision 18) and does not depend on it.
14. **M8a's `owns` rule assumed every task writes.** Resolved by decision 24; the change to M8a's code is a step of task M9.4.
15. **M8a's runnable test requires `merged` dependencies.** Resolved by decision 35 (`reported` satisfies a dependency); the change is a step of task M9.9, and the hold condition one of M9.7.
16. **M8a decision 29 removed paste delivery**, which the orchestrator window, the one PTY run window, still needs. Resolved by decision 39; the scope of the change is a step of task M9.13.
17. **`manager/create.rs`'s phases are private** (`admit`, `spawn_window`, `insert`). Resolved by decision 5 (`pub(super)`).
18. **`McpTarget` has no epic.** Resolved by adding `epic` (Interfaces "daemon"), a step of task M9.8.
19. **M8b's `--role scout` requires `--scout`**, which a task-bound research scout does not have. Resolved by `--task` (decision 35), a step of task M9.11.
20. **M8b's `PLAN_SCALE_MAX` is a constant** that spec §22.3 makes a setting (`planner_task_cap`). Resolved by decision 3, a step of task M9.3. (The refresh found it defined and unused; the literal is `decider/prompt.rs:27`.)
21. **The spec forbids time-based sizing but gives sub-planners no budget.** Resolved by `[orchestrator.planners]` tool-call and wall-clock limits (decision 32), which bound the planner's session, not any task's size.
22. **§7.3's chain collapse is a planning rule with no check.** Settled 2026-09-22 (spec §22 item 6): it stays the planners' judgement; the engine never checks it (decision 23.4 removed).
23. **TT §12.1's per-state table does not use the code's states** (PR #18's C1): it has `waiting` and `blocked(other)`, omits `proof` and `check`, and delivers to `blocked(question)` at once, which M8a's outbox holds. Resolved by decision 42b (the controller's rulings: held until the answer; queued through a gate).
24. **TT §12.1 says scope changes use `amend_task`, which has no `owns`** (C2). Resolved by decision 42 and contract rule 28: a scope change is `cancel_task` or `split_task` plus `add_task`.
25. **TT §12.1 ends `paused(message)` with a per-task `resume`**, but `resume` is run-wide (C3). Resolved by decision 42c: the next `info`/`change` message, a brief/acceptance amend, or the run-wide `resume` plan edit; no per-task resume exists.
26. **TT §12.2 and L77 name different diff bases** (C4). M9 keeps `<run_head>...HEAD` (decision 42e); the `<task start>` wording is 9.1's.
27. **TT §12.2 says `git merge --no-ff` in the worktree** (C5), which M8a's hand-back never runs. Resolved by decision 42e: M8a's plumbing hand-back, which is a merge, never a rebase.
28. **TT §12.1 says the stall watchdog bounds a turn** (C9). It fires only on silence; a busy turn is bounded by its budget. Decision 42c and contract rule 28 promise no more than "when the current turn ends".
29. **TT §6.9 ("no new tool") contradicts §12.3's `task_note`.** For M9, §12.3 wins (decision 42f); §6.9 is 9.2's.
30. **TT §12.3's digest list is named `notes`,** which the digest already uses for wake notes, and `Task.notes` is M8a's validation notes. Resolved by `task_notes` and `Task.orch.worker_notes` (decisions 42d, 42f).
31. **TT §12.1's "Batch boundary" and decision 19's atomic batch** meet at a mixed call. Resolved by decision 42: a `message` or `refresh` is the only edit in its call, and a mixed call is refused before any effect; so the orchestrator refreshes and messages in two calls, which the refresh hold joins into one worker turn (decision 42e).

## Produces for M9.5

| Item | For | Where |
|------|-----|-------|
| `AgentRole::Planner` and the planner session records (`EpicRecord.sessions`, `Run.orch.planner_usage`) | Planner usage in `run stats`, refitting planner routes | Decisions 31–33 |
| `TaskState::Reported` | History records of research and review tasks | Decision 35 |
| `Task.orch.gate_hold`, `GateHoldRecord` | Racing and pair tasks added by the orchestrator mid-run go through the same gate holds | Decision 28 |
| `EpicRecord` (area, merges, integration rounds) and the `<e>-int<n>` review tasks | Per-epic history, adaptive concurrency by area | Decision 37 |
| The digest and `digest_revision` | Adding `estimate_left_secs` and `bound_ratio_permille` to what the orchestrator sees | Decision 16 |
| `planner_task_cap`, `[orchestrator.agent]`, `[orchestrator.planners]`, `message_max_per_turn`, `note_max_per_task` | Threshold refitting | Interfaces "config" |
| `ORCHESTRATOR_CONTRACT` rules 21–22 (routing) | The place M9.5's routing proposals change | Interfaces "Contracts" |
| M8c's `Run.plan_edits` with `source`, `accepted`, `error` and `recipients` | Measuring how often the orchestrator, planners and the user change a plan, and how often they message workers | Decision 40 |
| Reader-slot order (deciders, reviewers, planners, scouts, research and review) | Adaptive concurrency's reader side | Decision 31 |
| `OrchestratorRecord` (route, wakes) and orchestrator OTLP usage | Orchestrator cost in `run stats` | Decisions 10, 14, 14a |
| `Task.orch.{messages, worker_notes, refresh_merges}` | How often messages and refreshes change a task's outcome | Decisions 42a–42f |
| `RoleRoutingDecision` lines (`HistoryLine::RoleRoute`) with candidate snapshots and outcomes | Role lists for the orchestrator, planners, scouts and deciders; the `spread` rotation position | Decision 43 |

## Implementation notes

### Brief refresh (2026-09-27)

Refreshed from `origin/main:docs/milestones/M9-orchestrator-and-subplanners.md` at `586aba2` (1428 lines), keeping its structure, decision numbers and task numbers; new decisions are lettered and the one new task is M9.13c. Evidence is on `cc9dcb7` (M8c, PR #20) unless marked `c103308` (the tool-search fix). The controller's rulings of 2026-09-27 (D-1 to D-13, B-23, C-01 and the earlier worksheet rulings) are binding and are cited as "ruling". One line per correction.

**Header and starting point**

1. Status `ready`, with 8c (PR #20) and the tool-search fix named as merged prerequisites; a stop rule if either is missing.
2. Protocol 10 derived from `PROTO_VERSION = 9` (`proto/src/lib.rs:40`, test `proto_version_is_nine` `:109`); the test becomes `proto_version_is_ten`.
3. Starting-point line counts recounted: `launch/mod.rs` 427→432, `hooks.rs` 313→399, `manager/create.rs` 559→563, `entry.rs` →387, `restore.rs` 509→554, `server.rs` 556→570, `fake-agent/src/script.rs` 190→432, `main.rs` 319→382, `pty-smoke.py` 1572→1799.
4. `Entry.run` is a verbatim value (`manager/entry.rs:111`) and `Entry::info` sets `WindowInfo.run` only from a headless spec (`:152`); decision 11 sets it from the role.
5. `server/headless_guard.rs::refuse` (`:17`) takes only `&WindowManager`, and `headless_run` is `None` for any PTY (`manager/headless.rs:243`): decision 11's run-live flag is how the guard sees a live run.
6. `CLI_CAPS` is at `headless/argv.rs:100`; `claude_user_settings_only` is **set** (`:108`) and `codex_user_config_only` is `None`: main's Risk 9 had the runtimes backwards; corrected, and `project_settings_check_covers_the_orchestrator` now tests the Codex case.
7. `WORKER_MCP_TOOLS: [&str; 2]` (`run/role_launch.rs:19`, `c103308`) becomes 3 with `mcp__anthrex__task_note` (ruling B-23; decision 42f).
8. The tool-search fix's `ENABLE_TOOL_SEARCH=false` (`config/src/reserved_env.rs`, `CLAUDE_TOOL_SEARCH`; `headless/mod.rs:252` `session_vars`, `c103308`) is applied to the Claude orchestrator too (ruling C-01; decision 10); the user's own PTY windows stay unchanged.
9. `pty-smoke.py` calls `run_engine_stage`, `adapt_stage`, `run_view_stage` at `:1716-1718` and `== stage 12` at `:1720`; stage 11f is placed at `:1719`, its import beside `:46`.

**Paths and names (the code wins)**

10. Config file `crates/config/src/orchestrator_agent.rs` → `crates/config/src/orchestrator/agent.rs`, beside the existing `orchestrator/{adapt,profile,roster,unknown}.rs`.
11. Config tests `crates/config/tests/orchestrator_agent.rs` → `crates/config/src/orchestrator_tests_agent.rs` (the crate has no `tests/`; its suites are `orchestrator_tests*.rs`).
12. Proto tests `crates/proto/tests/` → the in-crate suites (`proto/src/run_tests.rs`, `run_tests_view.rs`, `adapt_tests.rs`) and a new `orch_tests.rs`; proto has no `tests/`.
13. `run/reconcile.rs` → `run/reconcile/mod.rs` (the `HandBack` row at `:179`).
14. `run_git` → `worktree::run_git` (`daemon/src/worktree.rs:234`).
15. `anthrex mcp`'s flags live in `crates/cli/src/mcp_cmd.rs` (`RoleArg`, `--run` required for worker, reviewer, orchestrator), not `main.rs`; M9.11 edits `mcp_cmd.rs`.
16. `mcp_args` (`headless/argv.rs:238`) has no `Planner` arm; M9.8 adds it. Its tests are `headless/argv_tests.rs:201` (`mcp_args_for_a_worker`) and `argv_caps_tests.rs:327` (`mcp_args_for_a_scout`).
17. `run/engine/holds.rs` already exists (M8a ruling N5's dependency holds, 255 lines); the approval holds are `engine/gate_holds.rs`, and the daemon's types are `GateHoldRecord`/`Task.orch.gate_hold`; the wire keeps `HoldKind`/`HoldState`/`HoldInfo` (decision 1).
18. `engine/integration.rs` and `RoundOwner` are dropped: `RoundOwner` is not in the code, sub-planners run on M8b's scout machine, and the integration review is an engine-made review task `<e>-int<n>` in `engine/kinds.rs` (rulings; decisions 31, 37).
19. Integration-review scripts `reviewer-epic-<e>-<n>` → M8a's `reviewer-<e>-int<n>-<n>` (the review is a task).
20. `Run.edit_log` → M8c's `Run.plan_edits` extended (`run/edit_log.rs:14`, `record` at `:67`, called once at `engine/requests.rs:352`); `record` gains `source` and `outcome` (ruling: one log; decision 40).
21. `PLAN_SCALE_MAX` is defined and unused (`run/triage.rs:23`); the literal `2 to 12 tasks` is `decider/prompt.rs:27`, its test `decider/tests_prompt.rs:92` (M9.3).
22. `research_and_review_kinds_are_deferred` is `run/validate_tests_fields.rs:179`; `owns_required` is `:135`.
23. `worker_tools_are_task_done_and_task_blocked` (`mcp/src/tools.rs:136`) is renamed `worker_tools_are_task_done_task_blocked_and_task_note`.
24. `claude_settings_json_is_exact` is inline in `launch/claude.rs:42`.
25. `RunInfo.scouts` is set to `Vec::new()` by the pure snapshot (`run/snapshot.rs:91`); the driver overlays `ScoutService::run_scouts` (`scout/service.rs:502`) (decision 20).
26. `PlannerState` has no `Queued` (`proto/src/planner.rs:11`); a queued planner shows `Planning`.
27. `HistoryLine` exhaustive matches that gain the `RoleRoute` arm: `run/history_io.rs:119-121`, `run/engine/history.rs:33-35`, `run/stats.rs:90-92`; `proto/src/adapt_tests.rs:208` asserts `HISTORY_VERSION == 1` and becomes 2.
28. A history append is `OpKind::AppendHistory` (`run/engine/mod.rs:533`, `engine/ops.rs:252`), not an `Effect`; decision 43 uses the op.
29. The config crate has no serde, so `RunLimits` cannot hold `config::PlannerConfig`; `RunLimits.orch: OrchLimits` (with `PlannerLimits`) is a daemon type of proto enums.
30. `EventKind`'s variants listed as shipped (`run/engine/mod.rs:93`: `BaseAdvanced`, `Delivered`, `Stop` included); `Effect` has `Interrupt`, `RemoveWindow`, `WatchWorktree`, `UnwatchWorktree`.
31. Names table rows gained file:line anchors for `build_run` (`plan.rs:311`), `resolve_task` (`validate.rs:72`), `validate_tasks` (`validate_graph.rs:43`), `apply_edits` (`edits.rs:47`), `worker_prompt`/`handover_prompt`/`reviewer_prompt` (`contract.rs:70/:111/:139`), `DONE_CHECK_GIT_TIMEOUT` (`run/driver.rs:59`), `fake_agent_bin` (`cli/tests/support/mod.rs:30`), `with_deciders` (`support/run_adapt.rs:76`).
32. Enter on the run's root already focuses the orchestrator (`tui/src/app_tests/overview/run_enter.rs:43`); main's `enter_on_the_orchestrator_focuses_its_window` becomes that pinning test, and the smoke uses Enter on the root.
33. `C-b g` is free (`tui/src/keymap.rs:116-135`); `Modal::EditTask` is at `app/mod.rs:77` (not `:79`).
34. The TUI's refusal of kill and remove for a live orchestrator goes in `app/headless.rs::headless_control_refusal` (`:24`) (decision 11; M8c R13).
35. OTLP tests go in the existing `crates/daemon/tests/otlp_server.rs`; `OTLP_MAX_CONNECTIONS = 8` is `metering/server.rs:45`.
36. The promote body is `engine/requests.rs:174-210`; the attention text is `run/model_adapt.rs:25`; `hh_mm` (`model_adapt.rs:33`) has one caller (`requests.rs:198`) and is deleted with the time in the repeat reply (M8c follow-up: UTC time beside local times; decision 29).
37. `a_fast_path_run_refuses_task_additions` (`engine/tests/fast_path.rs:294`) and its text (`requests.rs:286-293`) stay for an unpromoted fast-path run.
38. M8b's stale tests, corrected list: `run_e2e_adapt.rs:145, :164, :177, :187, :271` (and `:307`'s repeat text), `driver/adapt_goal_tests.rs:125`, `run/triage_tests.rs:302` with its `refused_message` cases `:317`, `:335` (`refused_message` at `triage.rs:356`), and `fast_path.rs:126, :177` (main said three end-to-end and two unit tests).
39. `WORKER_MCP_TOOLS` is pinned by `headless/argv_tests.rs:32, :261` and `run/engine/tests/dispatch.rs:156`; M9.13a updates them.
40. The smoke's `DECIDER_DIR` is removed by stage 11d's `finally` (`pty_smoke_adapt.py:155`), so stage 11f recreates it; `triage-1.json` is an answer file `fake-agent`'s decider mode claims (`crates/fake-agent/src/decider.rs:2`), not a stand-in script; `GOAL_CMD_TIMEOUT = 900.0` (`pty_smoke_adapt.py:40`).
41. `run/driver/adapt_goal.rs::planned()` is at `:244`, called at `:150, :160, :165` (decision 26); tool routing is `run/driver/adapt.rs:153` (decision 35).
42. File-size table recounted on `cc9dcb7`/`c103308`, with rows added for `headless/{argv,mod,conversation}.rs`, `scout/*`, `role_launch.rs`, `reconcile/mod.rs`, `history_io.rs`, `stats.rs`, `engine/history.rs`, `metering/server.rs`, `decider/prompt.rs`, `proto/src/history.rs`, `mcp_cmd.rs`, `keymap.rs`, `app/mod.rs`, `ui/modal.rs`, `run_harness.rs`; `headless/conversation.rs` (452, over M8c's 430) is not touched.

**Rulings applied**

43. D-1: `message` and `refresh` each alone in a one-edit call; the reply lists `delivered` and `refused` (`noted` folded into `delivered`) (decisions 19, 42, 42b); contract rule 29 now says refresh in one call, then the change message in a second; rule 28 says a message or refresh is always the only edit in its call.
44. D-2: `BlockReason::MessagePause`, serde `message_pause` (decision 42c).
45. D-3: `MessageTarget::Stage` kept on the wire and refused at acceptance until 9.1; main's M9.14 test `run_message_reaches_the_selected_task_or_stage` → `run_message_reaches_the_selected_task_and_refuses_a_stage`.
46. D-4: main's typed `MessageTarget` and CLI shape `<task|stage:<n>|running>`.
47. D-5: top-level `[orchestrator] message_max_per_turn` (3, 0..=20) and `note_max_per_task` (10, 0..=100), zero disabling, frozen at run start in `RunLimits.orch`.
48. D-6: `task_note` text 1–4000 (TT §12.3 silent); `message` text ≤ 4000 (TT §12.1).
49. D-7: main's lighter snapshot fields (`message_count`, `last_message_kind`, `task_notes: Vec<TaskNoteInfo>`, `TaskNoteKind`).
50. D-8: main's placement; the draft's M9.4a/M9.4b tests merged into M9.13a (pure engine parts inside it).
51. D-9: the goal form sends the same `StartGoal` as `run start --goal`; smoke 11f uses the form on macOS and the CLI on Linux (deviation recorded below).
52. D-10: `Run.role_routing_decisions` top-level.
53. D-11: `Changes applied:` is a contract-text test only; the engine never interprets it.
54. D-12: request ids in PROTO 10 (`ClientMsg::RunTagged`, `request_id` on every answering reply; decision 2; widened by the M9.2 review fixes).
55. D-13: TT §12.1's "`resume` for that task" followed literally: with no per-task resume, the run-wide `resume` plan edit releases paused tasks; `anthrex run resume` after a restart does not (decision 42c; flagged below).
56. C-01: the tool-search fix is merged; M9.1 item 10 only re-verifies MCP tool visibility, once, against the real CLI; no M9.1a.
57. B-23: `task_note` in `WORKER_MCP_TOOLS`.
58. Earlier rulings kept: sub-planners on the scout machine (31); integration review as an engine-made task (37); one edit log (40); TUI label recognition with no protocol change (42i); readers read the user's checkout (20a); scouts failed on restore, never killed (20, 32); the run-live flag (11); worker contract lines 10–11 (41); OTLP token (14a) and connection cap (14b); strict non-ASCII protected rule (23a); credential scrub on the orchestrator (10); M8c Risk 9 taken (11a); a message to `blocked(question)` held until the answer, to `proof`/`check` queued and delivered only if a gate bounces (42b).

**Added**

59. Decision 11a (a headless record never comes back as a PTY) and M9.10's test `unparseable_headless_record_restores_as_an_exited_headless_window`.
60. Decision 16a (plan text only at the gate) and M9.6's `briefs_are_published_only_at_the_gate`, `a_snapshot_of_fifty_terminal_runs_stays_small`.
61. Decision 10's `ENABLE_TOOL_SEARCH=false` for the Claude orchestrator and M9.10's `claude_orchestrator_env_turns_tool_search_off_and_codex_does_not`.
62. Contracts name every anthrex tool by its Claude id at first mention (the tool-search fix's form, `run/contract.rs` on `c103308`); M9.5's `contracts_name_every_tool_by_its_claude_id`.
63. M9.1 item 10 (tool visibility probe) and item 11 (`packed-refs.lock` source); task M9.13c.
64. The M8c follow-ups M9 owns in "Follow-ups handled": snapshot size, UTC promote reply, request ids, Risk 9, `packed-refs.lock`, `headless/conversation.rs` over budget; the `doing` tool target moved to Out.
65. The goal form's details (decision 44): `pending_open`, tagged refusal, submitting state.
66. Manual check 6 (tool visibility) and Risks 15, 16.
67. Spec and brief defects 23–31 (the draft's 23–30 renumbered to decisions 42x, and 31, the batch boundary).
68. Scenario rows for TT §12.7, spec §15 and §16; M9.16's `e2e_messages_are_not_redelivered_after_a_restart`, `e2e_goal_form_request_matches_the_cli`; M9.17's `e2e_role_history_for_large_and_triage_paths` with a failed-creation triage case.
69. "Linux CI" and "tests never reach real agents" rules in the shared test helpers, and the dash and no-global-identity runs in Verification.

### Status and protocol

- Status `ready` once 8c (PR #20) and the tool-search fix are on `main`. Protocol 10 (above).

### Deviations from the brief

- (none yet; the implementer records each here, with its evidence)
- Pre-recorded by the refresh (ruling D-9): the goal form has no `--unconfined-checks` toggle, so on Linux its start is refused with M8a's text unless `[orchestrator] unconfined_checks = true`; smoke stage 11f starts through the CLI there.

### Changes to earlier briefs

M8a: `owns_required` scope, research/review refusal removed, runnable test (`reported` deps, gate holds), `McpTarget.epic`, `mcp_args` planner arm, `LaunchContext.role`, `LaunchPlan.{scrub_agent_env, remove_env}`, `HookKind::StopFailure`, `HandBack.list_merged`, `WORKER_CONTRACT` lines 10–11, `WORKER_MCP_TOOLS` 3, outbox exceptions (paused delivery, refresh hold). M8b: triage refusal replaced, promote performs, `PLAN_SCALE_MAX` deleted, `--role scout --task`, `HISTORY_VERSION` 2, OTLP token and cap. M8c: `edit_log::record` signature, snapshot plan text only at the gate, `RunInfo.planners` filled, headless restore of an unparseable record. Each is a step of the task named in its decision.

### Left open at the refresh (need a controller ruling)

1. **`packed-refs.lock` (task M9.13c).** The followups file says it "needs a design decision". The refresh proposes an engine-written checkout config key (found by M9.1 item 11) with the sandbox unchanged; M9.13c must not start until that is ruled.
2. **`TaskInfo.last_message_line`.** TT §12.6 asks the inspector for the latest message's first line; ruling D-7 took main's lighter fields, which lack it. The refresh added one optional field (≤ 80 characters). Keep it, or show only count and kind?
3. **D-13's reading.** The refresh reads "`resume` for that task" as the run-wide `resume` plan edit (which releases paused tasks) and not `anthrex run resume` after a restart (so a pause survives a restart, as main's M9.13a says). Confirm.
4. **Mixed calls with `submit` or `summary`.** Decision 42 refuses a `message`/`refresh` call that also carries `submit` or `summary` (main's text says "plan mutations"). Confirm that `submit`/`summary` count.
5. **Model grouping.** The refresh keeps new run and task state under `Run.orch: RunOrch` and `Task.orch: TaskOrch` (the files at 572–591 lines cannot take flat fields), with `Run.role_routing_decisions` top-level per D-10, and `RunLimits.orch: OrchLimits` because the config crate has no serde. Main named flat fields in places.
6. **Names across the wire.** The wire keeps main's `HoldKind`/`HoldState`/`HoldInfo`/`TaskInfo.hold`; the daemon uses `GateHoldRecord`/`Task.orch.gate_hold` to avoid M8a's dependency holds. Confirm the split.

### Controller rulings on "Left open at the refresh" (2026-09-27)

1. **`packed-refs.lock` (M9.13c).** M9.1 item 11 identifies which git command takes the lock. If an engine-set key in the existing `GIT_CONFIG_PARAMETERS` pin stops it without widening the sandbox, for example by disabling the auto-pack that causes it, apply that key and test it. Otherwise leave the harmless warning, record it, and close M9.13c as "no change". The sandbox is never widened.
2. **`TaskInfo.last_message_line`:** kept. It is optional, at most 80 characters, cleaned of control characters, and `#[serde(default)]`.
3. **D-13:** your reading is confirmed. A task paused by a message is released by the run-wide `resume` plan edit or by a later message. `anthrex run resume` after a daemon restart does not release it.
4. **Mixing:** confirmed. `submit` and `summary` count as plan mutations, so a message or refresh call that carries either is refused before any effect.
5. **Model grouping:** accepted as written. The new state is under `Run.orch` and `Task.orch`, with `Run.role_routing_decisions` at the top level.
6. **Hold names:** accepted. The wire keeps `Hold*`, and the daemon uses `GateHoldRecord` and `Task.orch.gate_hold`.
7. **Goal form on Linux:** the deviation is accepted and recorded, as is the follow-up.

### The early-events fix (PR #22, merged 2026-09-28, before M9 started)

`main` at `8d440d7` also carries PR #22 (the M8a engine race). It holds a session's signals and worker/reviewer tool calls that arrive before its `CreateWindow` op's `Window` result, and replays them once the round has the window; see `crates/daemon/src/run/engine/early.rs`. It has these caps:
- `HOLD_CAP`: 256 events per window;
- `HOLD_WINDOWS_CAP`: 64 windows;
- `HOLD_LIMIT_SECS`: 30.

M9 consequences:
- `early::holds_call` knows only `task_done`/`task_blocked` (worker) and `submit_review` (reviewer). M9.13a's `task_note` and the planner's and run scout's submit tools are refused at once when they arrive before their window is known, as every early call was before #22. A task that adds a headless role's tool must decide whether it joins `holds_call` and add an `early_events` test for it. The default is to join, so that the role's first turn cannot be lost.
- The orchestrator is a PTY window. Its signals come from hooks after the TUI starts, and its window id is bound by the same `Window` result, so the hold covers them too.

Line numbers cited on `cc9dcb7` and `c103308` above may have shifted in files that #21 and #22 touched: `run/engine/{signals,done,mod}.rs`, `tests/support/run_git.rs`, `headless/`. Re-check them before citing.

### Task M9.2 (Protocol)

- **Version.** `PROTO_VERSION` 9 → 10, derived from `crates/proto/src/lib.rs` on this branch (`pub const PROTO_VERSION: u32 = 9;`, test `proto_version_is_nine`), as the header says. M9.1's item 9 was not yet recorded when M9.2 ran; the header's derivation was followed as written. `HISTORY_VERSION` 1 → 2.
- **`MessageTarget` and `MessageKind` live in `proto/src/orch.rs`**, not `run.rs`: with its custom serde `run.rs` would pass 500 lines (the file-size table allows the move). `MessageTarget` also implements `Display` (`t1,t2`, `stage:3`, `running`), which `edit_log::describe_one` uses: `message to t1,t2`, `refresh t4`. An empty task-id array, `stage:` with no digits or a non-digit, and any other string are refused by the deserializer; the 20-id cap is left to acceptance (M9.13a).
- **Reply helpers.** `RunReply::done(request, message)`, `RunReply::refused(request, message)` (both `request_id: None`) and `RunReply::tagged(id)` (stamps `Done`/`Refused`, leaves every other reply as it is) are new in `run_wire.rs`. The driver's literal replies became helper calls, which keeps `driver/requests.rs` at 576 lines (−12). The server's `ClientMsg::RunTagged { id, request }` goes to `RunApi::handle(request, Some(id))`, which tags the reply; `handle` now returns `Option<DaemonMsg>` (always `None`) so both arms in `server.rs` are one line each (server.rs −2).
- **Budgets passed** (all files stay far below 600): `run_wire.rs` +55 (budget +25; the helpers), `history.rs` +72 (+45; the brief's fields each need their own `#[serde(default)]` line), `run_info.rs` +47 (+40), `messages.rs` +7 (+5). `orch_tests.rs` would have been 614 lines, so the history test is in `orch_tests_history.rs`.
- **Minimum arms until later tasks** (behaviour comes with the named task):
  - daemon: `RunRequest::{ApproveHold, RejectHold}` answer `Refused` with `this run has no approval holds` under `request::APPROVE`/`REJECT` (M9.7); `StartGoal.orchestrator` and `Promote.orchestrator` are ignored (M9.7, M9.10); `PlanEdit::{Message, Refresh}` and an `amend_task` carrying `deps` are refused in the edit batch with `amend deps, message and refresh are not available yet` (M9.4, M9.13a); `BlockReason::MessagePause`'s edit label is `message_pause`; `TaskState::Reported` counts toward no phase; `AgentRole::Planner` is `planner` in the usage roll-up and in `mcp_args` (M9.8 adds `--epic`); `HistoryLine::RoleRoute` is keyed by its `record_id` and ignored by `run stats`.
  - daemon model: `Run.role_routing_decisions: Vec<proto::RoleRoutingDecision>` (`#[serde(default)]`) is added now, so the old-run test can hold it. `old_run_json_loads` strips it and each task's `spec.review_target` as new keys.
  - snapshot: every new `RunInfo`/`TaskInfo` field is at its default; `PlanEditInfo.source` is empty and `accepted` true (decision 40 fills them).
  - mcp: `tools_for(Planner)` is empty (M9.11), `role_name(Planner)` is `planner`.
  - TUI: `Planning` → `planning`, drawn as working; `Reported` → `reported`, `✓`, done colour, outside the progress categories; `MessagePause` → `paused(message)`; `Planner` → `planner #<session>`, drawn as a worker round, ranked after scouts. M9.15 owns the real display.
- **Tests.** `orch_types_round_trip`, `new_requests_round_trip` (with `ClientMsg::RunTagged` and M8b-shaped `StartGoal`/`Promote` without `orchestrator`), `request_id_round_trips`, `appended_variants_keep_their_indices`, `reported_is_finished_and_planning_is_not_terminal`, `message_and_refresh_edits_round_trip` (JSON, MessagePack and a TOML edit file) in `proto/src/orch_tests.rs`; `role_routing_history_round_trip` in `proto/src/orch_tests_history.rs`; `old_run_info_still_decodes` and `orchestrator_snapshot_fields_round_trip` in `proto/src/run_tests_view.rs`; `proto_version_is_ten` in `lib.rs`; the daemon half of the history test is `old_run_defaults_role_routing_decisions_to_empty` (`run/engine/tests/view_fields.rs`, against `m8b_run.json`); and `a_tagged_run_request_is_answered_with_its_id` in `daemon/tests/server_runs.rs` pins the server's echo over a real socket (not named by the brief). `appended_variants_keep_their_indices` decodes a MessagePack integer as each unit enum's variant index; `PlanEdit` is internally tagged by name, so its order is pinned by the `op` list serde reports.
- **Fixture.** `crates/proto/src/m8c_run_info.json` is `a_view_snapshot().runs[0]` serialized with milestone 8c's `RunInfo` before any change of this task.
- **Left open for M9.13b.** `RoleRoutingDecision.role` is an `AgentRole`, which has no decider variant; the history test uses a placeholder role for the pre-run triage record. M9.13b decides how a decider's record names its role (a new `AgentRole` variant would need its own protocol change). *Superseded by the review fixes below (ruling 2).*

### M9.2 review fixes

Controller rulings on the M9.2 review (commit `7042f12`), each done in one commit:

1. **Every run reply carries `request_id`.** `Started`, `Done`, `Refused`, `ConfirmNeeded`, `ToolResult` and `Triaged` gain `#[serde(default)] request_id: Option<u64>` as their last field. `Profile` and `Stats` were newtype variants, so they became struct variants to hold the field: `Profile { reply: Box<ProfileReply>, request_id }` and `Stats { stats: HistoryStats, request_id }`. Their milestone-8c shape no longer decodes, which the PROTO 10 handshake already enforces. **Deviation: `Snapshot` carries no id.** It stays a newtype. A snapshot is state, not an answer: a subscription pushes the same value unasked, and any snapshot answers `List` equally. `RunReply::tagged(id)` stamps every other variant, and `RunReply::request_id()` reads it back. New untagged constructors `RunReply::{tool_result, profile, stats}` join `done` and `refused`. Tests:
   - `every_run_reply_round_trips_its_request_id` (all eight answering variants, and `Snapshot` untouched), in the new `proto/src/orch_tests_replies.rs`;
   - `request_id_round_trips`, moved there and widened to every struct-shaped milestone-8c reply decoding as `None`;
   - `a_tagged_goal_start_is_triaged_with_its_id` in `daemon/tests/server_runs.rs`: a real socket, a repository with a stored profile, deciders off, so triage falls back to the plan path and answers `Triaged` with no run and no agent;
   - `a_tagged_run_request_is_answered_with_its_id`, which now shares that file's `tagged_rig` helper.

   The Interfaces entry says every reply carries it.
2. **`AgentRole::Decider`**, appended after `Planner`, inside PROTO 10 (`appended_variants_keep_their_indices` extended). It is serialized as `decider`. A decider has no rounds or tasks, and no arm folds it into a worker:
   - usage roll-up: `decider`;
   - TUI `round_label`: `decider`, ranked last;
   - round glyph: `–` dimmed;
   - round inspector: no fields;
   - `mcp::tools::tools_for(Decider)` is empty and `role_name` is `decider` (test `a_decider_has_no_tools`).

   `headless::argv::mcp_args` now returns `Option<Vec<String>>` and answers `None` for a decider, so a session that names one gets no anthrex MCP server on either runtime (test `mcp_args_refuses_a_decider`). This refusal needs no `unreachable!`. The history test's pre-run triage record uses `Decider`. **For M9.13b:** a triage record's `record_id` must be namespaced (the fixture uses `triage/<request>/<n>`) so it never collides with a task or run record's id, because `history_io.rs` deduplicates on `record_id`. M9.13b owns the format.
3. **`Reported` in the TUI progress line.** `inspector/run_format.rs::progress_text` counted a `Reported` task as `waiting` and in the total (`unwrap_or(5)`). It now skips it. Test: `a_reported_task_is_outside_the_progress_line`, red before with `1/4 merged · 1 working · 2 waiting`.
4. **`Reported` is not yet finished in the engine.** The places are `engine/complete.rs::complete_pass`, `engine/outbox.rs`'s delivery skip list, and `run/history.rs::outcome`. They are unchanged here and are written into M9.9's task text ("Earlier-brief changes", item 5) as obligations with their tests.
5. **`running` is a reserved task id.** `validate.rs::check_fields` refuses it with `running is reserved for the message target of every running task` (test `reserved_id_running`). `stage:` needs nothing, because the id pattern has no `:`. That is pinned by `a_task_id_cannot_look_like_a_stage_target`, a pinning test that passed at once.
6. **`anthrex mcp --role planner`.** `mcp_cmd.rs`'s `RoleArg` gains `Planner`, and `--run` is required for it as for the other run roles. There is no `Decider`. Tests:
   - `mcp_role_planner_parses_and_decider_does_not`;
   - the planner row added to `mcp_parses_the_daemons_headless_argv_and_is_hidden` and `run_is_required_except_for_scouts`.
7. **`RunApi::handle` returns `()`.** Both server arms are `{ run_api.handle(..); None }`. This supersedes the `Option<DaemonMsg>` note above.
8. **Placeholder refusals pinned** (pinning, passed at once):
   - `placeholder_edits_refuse_their_whole_batch` (`run/edits_tests_placeholders.rs`, through `apply_edits`, with a valid `cancel` in the batch too);
   - `placeholder_edits_leave_the_run_unchanged` (`engine/tests/dispatch_edits.rs`, through the engine's `run edit` path on a running run). It covers `Message`, `Refresh`, `AmendTask { deps: Some }`, `[Pause, Message]` and `[Finish, Refresh]`, and asserts the run is equal before and after;
   - `approval_holds_are_refused_until_the_engine_keeps_them` (`daemon/tests/server_runs.rs`), for `ApproveHold` and `RejectHold` under `request::APPROVE`/`REJECT` with the `NO_HOLDS` text.
9. **`PlanEditInfo.source` defaults to `user`** through `user_by_default`, as `accepted` does. `old_run_info_still_decodes` now expects `user`. The daemon's snapshot also fills `user` until M9.9 records sources. **Decision 40 wins over the M9.2 task text** for `describe_one`: a message is described as `message <to> (<kind>)`, for example `message t6,t7 (change)`, not `message to <to>` (`describe_names_every_edit_op` updated).

### M9.1 External facts and names (2026-09-28)

- **Checks 1–7 and 10 (real `claude`/`codex`): outstanding.** The run's auto-mode safety classifier refused to launch an agent running the real CLIs, and this was not worked around. The user has been asked. Until then, every decision these checks guard is built as written, with its fallback unexercised: 7, 8, 10, 12, 14, 14a and 42f. The PR lists them as outstanding.
- **Check 11 (`packed-refs.lock`), done 2026-09-28 by the controller with git 2.50.1 (Apple Git-155), no agent CLI involved.**
  - **Setup.** A checkout shaped like `run/git/checkout.rs::config`: its own git dir, `core.bare = false`, `core.worktree`, `core.logAllRefUpdates = false` and `gc.auto = 0`. A `sandbox-exec` profile denies only `file-write*` of `<gitdir>/packed-refs.lock`.
  - **What happens.** `git commit` prints `error: Unable to create '<gitdir>/packed-refs.lock': Operation not permitted` and exits 0. The commit is made.
  - **Which git command takes the lock.** `GIT_TRACE_REFS=1` shows the lock is taken by the commit process itself, not by a child:
    - the HEAD update transaction finishes (`finish: 0`);
    - git reads `CHERRY_PICK_HEAD` and `REVERT_HEAD`;
    - a second ref transaction, the post-commit removal of merge-state refs, fails in `transaction_prepare` with that error;
    - then git reads `MERGE_AUTOSTASH`.
  - **Why no config key stops it.** The files backend takes `packed-refs.lock` for any ref deletion, whether or not the ref or a `packed-refs` file exists. The same error appears:
    - with the branch loose;
    - with no `packed-refs` file at all;
    - on a detached HEAD;
    - with `-c maintenance.auto=false`, which stops the `git maintenance run --auto --quiet --detach` child that `git commit` also starts (seen in `GIT_TRACE2_EVENT`);
    - with `-c maintenance.pack-refs.enabled=false`;
    - with `gc.auto=0`.
  - **Result under controller ruling 1.** No engine-set key stops it without widening the sandbox, so the warning stays. It is harmless: the commit succeeds with exit 0. **M9.13c closes as "no change"**, and the sandbox is not widened. What M9.13c still owes is a line in the worker contract. That line says the `packed-refs.lock` warning after a commit is expected and needs no action, so a model does not try to "fix" it. That is the followups file's concern at F1 re-review 2, S5.

Checked on `origin/main` at `8d440d7`, read with `git show` / `git grep` (no worktree files).

- **Check 8 (names).** Differences from "Names taken from earlier briefs":
  - `Run.notes` does not exist. The only `notes` field in the model is `Task.notes: Vec<String>` (`crates/daemon/src/run/model.rs:172`); `Run` (`model.rs:365–510`) has no notes field. Every other listed `Run` and `Task` field is present (for example `Run.revision` at `model.rs:395`, `Run.plan_edits_since_approval` at `:510`, `Task.conflicts` at `:190`). A task that needs run-level notes must add the field; it is not inherited.
  - `RunInfo.approved_hhmm` does not exist. The field is `RunInfo.approved_at: Option<u64>` (Unix seconds) (`crates/proto/src/run_info.rs:297`). The hh:mm formatting is `model_adapt.rs::hh_mm` (`crates/daemon/src/run/model_adapt.rs:33`). `RunInfo.plan_edits` is `Vec<PlanEditInfo>`, at most 10 (`run_info.rs:300`), not `PlanEditRecord`.
  - `edit_log::describe_one` is private: `fn describe_one(edit: &PlanEdit) -> String` (`crates/daemon/src/run/edit_log.rs:51`). `describe` (`:29`), `record` (`:67`), `PlanEditRecord` (`:14`), `PLAN_EDITS_KEPT` (`:20`) and `DESCRIBE_MAX_CHARS` (`:23`) are `pub`.
  - "The snapshot `watch`" is not a `tokio::sync::watch`. Snapshots go out on a `broadcast::Sender<Arc<RunsSnapshot>>` field `pushes` (`crates/daemon/src/run/driver.rs:141`), read through `RunService::pushes()` (`driver.rs:351`) and sent by `fn publish` (`driver.rs:563`). `RunService` (`driver.rs:132`), `RunContext` (`driver.rs:66`) and `RunService::request` (`crates/daemon/src/run/driver/requests.rs:92`) are as named.
  - `with_deciders(mode, dir)` is a free function, not a `RunHarness` method: `pub fn with_deciders(mode: &str, dir: &Path) -> (String, Vec<(String, String)>)` (`crates/cli/tests/support/run_adapt.rs:76`). `decider_calls` (`:151`), `stored_profile` (`:163`) and `onboarding_report` (`:328`) are methods in `impl RunHarness` (`:102`).
  - `proto::Runtime` is defined in `crates/proto/src/types.rs:10`, not `proto/src/run.rs`. The other types in that row are in `run.rs` (for example `AgentRole` at `:21`, `RunRef { run_id, task_id, role, session }` at `:32`).
  - `decider::prompt::TRIAGE_HEAD` is a private `const` at `crates/daemon/src/decider/prompt.rs:24` (the brief says `:27`). `run::triage::{route (:116), PLAN_SCALE_MAX (:23), TriageRoute (:64)}` match. `PLAN_SCALE_MAX` is still unused: its only occurrence in `crates/` is its definition.
  - Minor, same names: `EventKind` is at `run/engine/mod.rs:98` and `step` at `:313` (the brief cites `:93`). `EditScope` is defined in `run/validate_graph.rs:18` and re-exported from `validate.rs:21`. `headless::session::{SCRUB_PREFIXES, SCRUB_NAMES}` are `pub use` aliases of `config::reserved_env::{SCRUBBED_PREFIXES, SCRUBBED_NAMES}` (`headless/session.rs:50–52`). `run/engine/mod.rs:55` already declares `mod holds;` (M8a's early holds, `HeldEvent`/`HeldWindow`), so the new `gate_holds.rs` must keep a distinct name.
  - All other 24 rows confirmed.

- **Check 9 (counts).** `git show origin/main:<path> | wc -l`, brief "Today" → actual:
  - `crates/daemon/src/launch/mod.rs` 432 → 432; `launch/codex.rs` 353 → 353; `launch/claude.rs` 71 → 71; `hooks.rs` 399 → 399
  - `manager/create.rs` 563 → 563; `manager/restart.rs` 556 → 556; `manager/entry.rs` 387 → 387; `manager/restore.rs` 554 → 554; `manager/headless.rs` 526 → 526
  - `window.rs` 371 → 371; `server.rs` 570 → 570; `server/headless_guard.rs` 35 → 35
  - `headless/argv.rs` 450 → 450; `headless/mod.rs` 291 → 291; `headless/conversation.rs` 452 → 452
  - `scout/service.rs` 521 → 521; `scout/machine.rs` 239 → 239; `scout/spec.rs` 174 → 174
  - `run/engine/mod.rs` **569 → 584 (+15)**; `engine/requests.rs` 536 → 536; `engine/dispatch.rs` 589 → 589; `engine/done.rs` **507 → 526 (+19)**; `engine/merge.rs` 518 → 518; `engine/outbox.rs` 360 → 360; `engine/restore.rs` 422 → 422; `engine/complete.rs` 429 → 429
  - `run/model.rs` 572 → 572; `validate.rs` 514 → 514; `edits.rs` 577 → 577; `contract.rs` 584 → 584; `snapshot.rs` 315 → 315; `report.rs` 255 → 255; `report_task.rs` 237 → 237; `edit_log.rs` 83 → 83; `reach.rs` 97 → 97
  - `run/role_launch.rs` 579 → 579; `run/reconcile/mod.rs` 305 → 305
  - `run/driver.rs` 591 → 591; `driver/ops.rs` 584 → 584; `driver/requests.rs` 588 → 588; `driver/adapt.rs` 391 → 391; `driver/adapt_goal.rs` 283 → 283
  - `run/history_io.rs` 395 → 395; `run/stats.rs` 234 → 234; `run/triage.rs` 367 → 367; `run/engine/history.rs` 135 → 135
  - `metering/server.rs` 311 → 311; `decider/prompt.rs` 430 → 430
  - `crates/mcp/src/tools.rs` 258 → 258; `mcp/src/lib.rs` 109 → 109
  - `crates/proto/src/run.rs` 453 → 453; `run_info.rs` 320 → 320; `run_wire.rs` 211 → 211; `messages.rs` **— → 549**; `history.rs` 210 → 210; `planner.rs` 33 → 33; `lib.rs` 117 → 117
  - `crates/config/src/orchestrator.rs` 585 → 585
  - `crates/cli/src/run_cmd.rs` 501 → 501; `run_cmd/status.rs` 267 → 267; `mcp_cmd.rs` 65 → 65
  - `crates/tui/src/app/runs.rs` 524 → 524; `keymap.rs` 275 → 275; `app/mod.rs` 499 → 499; `theme.rs` 132 → 132; `tree/runs.rs` 198 → 198; `tree/run_rows.rs` 516 → 516; `inspector/run.rs` 349 → 349; `inspector/run_task.rs` 301 → 301; `inspector/run_round.rs` 228 → 228; `graph/run_text.rs` 98 → 98; `graph/paint/style.rs` 260 → 260; `ui/modal.rs` 247 → 247; `app/headless.rs` 38 → 38
  - `crates/tui/src/ui/conversation.rs` 389 → 389; (not touched) `crates/tui/src/conversation.rs` 595 → 595
  - `crates/fake-agent/src/script.rs` 432 → 432; `main.rs` 382 → 382; `mcp.rs` 198 → 198; `roles.rs` 343 → 343; (not touched) `headless.rs` 576 → 576, `stream_claude.rs` 581 → 581
  - `crates/cli/tests/support/run_harness.rs` **542 → 554 (+12)**
  - `scripts/pty-smoke.py` 1799 → 1799
  - Budget consequence: `run/engine/mod.rs` at 584 plus its +25 is 609, past the 600-line limit. Either the budget drops to +16 or engine code moves out first. `done.rs` (526 + 20 = 546), `messages.rs` (549 + 5 = 554) and `run_harness.rs` (554 + 10 = 564) stay under 600.
  - `PROTO_VERSION` = **9** (`crates/proto/src/lib.rs:40`, `pub const PROTO_VERSION: u32 = 9;`; the test at `:110` asserts 9). As expected, M9 makes it 10.

- **Controller rulings on checks 8 and 9:**
  - Tasks use the names as shipped: `Task.notes` only, with run-level notes added in `model.rs` where a task needs them; `RunInfo.approved_at` (unix seconds), formatted client-side; `describe_one` becomes `pub(crate)` where needed; the `broadcast` `pushes` channel, whose long-poll resubscribes on `Lagged`; and the free function `with_deciders`.
  - `run/engine/mod.rs` is 584 lines. Any task that grows it must first move code out, for example the step's dispatch helpers into a new file, so it stays under 600. Its row's budget becomes +15.

### M9.2 re-review notes (2026-09-28)

- The re-review APPROVED 86ffacb.
- The brief's request-id text is widened to every answering reply: decision 2, the `run_api` Interfaces line and D-12.
- `List`, `Subscribe` and `Unsubscribe` must never be sent tagged. A `Snapshot` or nothing answers them, so a client matching by id would wait forever.
- Nothing pins the snapshot's `PlanEditInfo.source = "user"`. M9.9 replaces that line, and its test pins the recorded source.
- **For M9.13a:** a `run.json` from before M9 may already hold a task named `running`. It loads without re-validation, so there `MessageTarget::Running` and the task id collide. M9.13a resolves a `running` target by state, never by id, and states that in its tests.
