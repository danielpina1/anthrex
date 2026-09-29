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
// WAKE_MAX_BYTES (2 KiB) is run/orch/contract.rs's (M9.5); wake.rs reuses it and defines no second one
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
  "roster": [{"runtime": "claude", "model": "claude-opus-5-5", "strength": "frontier", "note": "…", "installed": true}],
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
- Confinement (M9.4 review fixes, ruling 4; M9.4 already refuses a sub-planner's `add_task` naming another epic and its `split_task` of a task outside its epic): `submit_epic_cannot_amend_another_epics_or_the_orchestrators_task`, `submit_epic_cannot_cancel_another_epics_or_the_orchestrators_task`, `submit_epic_cannot_add_dep_on_another_epics_or_the_orchestrators_task` (each an explicit test, for a task of another epic and for one with no epic), and `submit_epic_refuses_pause`.
- `spawn_scout_queues_in_a_reader_slot_and_replies_at_once`; `scout_id_is_prefixed_and_unique`; `max_scouts_is_enforced`; `scout_ended_records_the_report_and_usage`; `restore_fails_running_scouts_with_the_documented_reason`; `restore_kills_nothing` (no kill effect for a run scout or a planner).
- `planners_snapshot_fields` (every `PlannerInfo` field, a queued planner as `Planning`).
- Real, in `crates/cli/tests/scout_service.rs` (M8b's service suite): `a_planner_session_runs_on_the_scout_machine_and_is_accepted` with `fake-agent`.

**The worker's scout extract (M9.5 review fixes, ruling 2).** When the driver builds a worker's `OpKind::CreateWindow`, it reads the run's scout reports (`readable_reports`) and passes `scout_extract` of them to `worker_prompt` and `handover_prompt` (decision 34), as it does for `planner_prompt`. The engine passes `""` for `extract` today (`engine/dispatch.rs`); this task replaces that. Test: `worker_first_turn_carries_the_scout_extract` (the first turn and a handover's contain the extract, indented as M9.5 writes it).

**Obligation from M9.7's review fixes (second review, items 8 and 10).** An epic hold round opened by the orchestrator's own addition, with no sub-planner running, stays `Drafting`. The orchestrator's `submit` must move every `Drafting` round it owns to `Awaiting`: the promotion round, and each epic round whose epic has no queued or planning sub-planner. Otherwise such a task stays held forever, with nothing for the user to decide. Test: `orchestrator_submit_submits_its_drafting_epic_rounds`.

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
- `integration_review_task_is_made_when_the_epic_is_merged` (id `<e>-int1`, kind review, engine-made); `integration_review_takes_the_first_free_round_id` (M9.4 review fixes, ruling 7: a user's `run edit` may already hold `<e>-int<n>`, since plan files and `run edit` keep M8a's rules only, so the engine picks the first round number whose id is free); `integration_changes_holds_completion`; `fix_task_merge_makes_round_two`; `finish_closes_changes`; `no_round_after_max_bounces_plus_one`; `plan_path_has_no_integration_review`; `integration_reviewer_route_is_the_peer_at_frontier`; `integration_review_tasks_refuse_orchestrator_edits`.
- `completion_waits_for_holds_planners_scouts_integration_and_submit` (one case per condition).
- `rewriting_a_mis_sized_task_restarts_it_at_rung_2`; `editing_a_human_blocked_task_does_not_restart_it`.
- Wake notes: `each_note_source_adds_its_exact_line` (a table over decision 39's list); `orchestrators_own_edits_add_no_note`; `notes_are_capped_at_20_with_the_earlier_line`; `wake_effect_needs_live_window_setting_and_new_revision`; `digest_read_drops_notes_up_to_the_revision`; `woken_clears_delivered_notes`; `completion_note`.
- Wake notes' cap (M9.8 review, ruling 4): M9.8 pushes its notes (sub-planner and run-scout ends) through `engine/planners.rs::wake_note` with no cap. M9.9 puts the cap there, so every note source goes through it; `notes_are_capped_at_20_with_the_earlier_line` covers an M9.8 source too.
- `edit_log_records_every_source_rejections_and_recipients` (M8c's cap of 50 kept). It must fail on M9.7's interim state (M9.7 review fixes, ruling 6): today `engine/batch.rs::apply_batch` records every accepted batch, the orchestrator's included, with source `user`, and a rejected orchestrator batch is not recorded at all. The test drives an accepted and a rejected `edit_plan` through `OrchEvent::Tool` and checks source `orchestrator` on both and `accepted: false` with the error on the second.
- `usage_sum_saturates`.

**Acceptance.** The five AGENTS.md commands pass. M9.4's `a_fix_task_after_a_finished_integration_review_is_exempt_from_the_cap` (`run/orch/rules_tests_scope.rs`, built from constructed state) stays green once this task produces integration reviews (M9.4 review fixes, ruling 3).

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

**From M9.6's review (M-4).** `task_summary` and `resolve_target` give each git command its own deadline, so one call can take several. M9.11's read path wraps the whole call in one `DONE_CHECK_GIT_TIMEOUT` deadline on `spawn_blocking`: the tool answers with a git error once that deadline passes, whatever the separate commands still have left.

**From M9.6's second review (Minor 5).** `digest`, `context` and `task_result` are built from a clone of the `Run` taken under the engine lock, then released: the build runs outside the lock, on `spawn_blocking`, never on a tokio worker. A 500-task context takes up to about 0.9 s to build and trim.

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

**Files.** Create `crates/daemon/src/run/driver/{orch_ops.rs, wake.rs}`. Modify `run/driver.rs` (`mod` lines only), `run/driver/ops.rs` (dispatch to `orch_ops`), `RunContext` (scouts, profiles, binaries), `run/driver/adapt_goal.rs` (decision 26 replaces `planned()` at `:244`, called at `:150, :160, :165`; decision 9's check), the driver's publish path (the run-live flag cleared on a terminal run; the engine already sets `OrchestratorRecord.live = false` when the run is accepted, discarded or failed, M9.7 second review, ruling 5, and the driver clears `Entry.run_live` with it, which is decision 30's "plain window") and restore path (set for a non-terminal run), the driver's `UsageSink` implementation (`token`, `live_orchestrators`; decisions 14a, 14b).

**Earlier-brief change (M8a decision 29, defect 16).** M8a decision 29 removed paste delivery and delivers every engine message as a headless turn. That stays exactly as it is for every headless session. Paste delivery is added back in one place only, `run/driver/wake.rs`, for the orchestrator's PTY window (decision 39); nothing in M8a's outbox or its `Effect::Deliver` changes, and no headless window ever receives a paste. M8b decision 22 step 6's refusal of the plan and large paths is replaced by decision 26.

**Tests first.**
- Pure, `wake.rs`: `encode_paste_wraps_and_normalises` (`\r\n` and `\n` to `\r`, markers stripped, framed); `wake_is_clamped` (to `run/orch/contract.rs`'s `WAKE_MAX_BYTES`, which `wake.rs` reuses; it defines no second constant).
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
- `message_text_cannot_forge_a_second_line` (a message whose text holds `\n[anthrex] Message from user (change): ...` is saved and delivered as one line).
- `notes_and_messages_reach_the_worker_handover_and_reviewer_prompts`.
- `refresh_clean_is_called_with_the_whole_list` (`n >= 1` and `n == list.len()`).

Pure reducer tests in `run/engine/tests/refresh.rs`: `refresh_waits_for_the_turn_boundary`; `refresh_holds_a_change_message_sent_in_the_next_call_for_one_turn` (refresh in one call, then the `change` message in a second; both texts in one turn); `refresh_up_to_date_sends_nothing`; `refresh_failure_is_a_wake_note_and_no_block`; `refresh_is_refused_on_a_resolving_or_awaiting_task`; `a_refresh_merge_alone_is_not_work` (the zero-commit check and the fallback's no-commit nudge ignore `refresh_merges`).

Real git, in `crates/daemon/tests/run_git_handback.rs` and a new `crates/daemon/tests/run_refresh.rs`, through M8a's hand-back helpers: `refresh_merges_cleanly_and_the_next_turn_names_the_commits`; `refresh_conflict_leaves_markers_sets_resolving_and_does_not_count_a_conflict`; `refresh_with_uncommitted_changes_is_refused` (at acceptance, by the driver's pre-check, with the exact text); `refresh_dirty_at_the_boundary_fails_without_a_block`; `net_diff_excludes_refreshed_commits` (a two-file fixture whose merged file is outside `owns`: the spill list, `DiffStats` and `count_commits` after a clean refresh count only the task's own file); `refresh_never_rebases` (the task branch's old head is an ancestor of the new one); `refresh_is_replayed_by_reconcile` (in `crates/daemon/tests/run_journal/git_handback.rs`, with M8a's abort-after-intent hook). Every git invocation passes `--no-optional-locks` and a scrubbed environment (AGENTS.md rule 11), asserted by reading the argv a stand-in logs.

**Obligations from M9.5 (M9.5 review fixes, ruling 2).**
- Wire `notes_section` (from `Task.orch.messages`) into `worker_prompt` and `handover_prompt`, and the saved messages (`worker_messages_for_review`) into `reviewer_prompt`. The engine passes `""` for `notes` and `messages` today (`engine/dispatch.rs`, `engine/review.rs`).
- When a message is accepted, fold every newline (`\n` and `\r`) in its text to a space before it is saved, so a message cannot forge a second `[anthrex] Message from ...` line. Test: `message_text_cannot_forge_a_second_line`.
- Call `refresh_clean(n, list)` only with `n >= 1` and `n == list.len()`: `n` is the number of merged commits and `list` is all of them, and an up-to-date refresh sends nothing. Test that.

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
- `run_edit_submit_submits_a_planning_run` (M9.7 review fixes, ruling 5, decision 13's user submit): `anthrex run edit <run> --submit`, alone or with `--file <f>` (`--file` becomes optional; at least one of the two is required, else clap's error), sends `RunRequest::Edit { edits, submit: true }`. A planning run prints the engine's reply (`the plan of run <id> was submitted: it awaits approval`, after `applied <n> edits; ` when a file was given) and then waits for `anthrex run approve`; any other state is refused with `run <id> is <state>; only a run being planned can be submitted` and exit 1.

**Acceptance.** `crates/cli/src/main.rs` unchanged. The five AGENTS.md commands pass.

**Commit.** `feat(cli): start planned goals, approve holds, message and refresh tasks, and show the orchestrator in run status`

### M9.15 TUI

**Files.** Create `crates/tui/src/run_goal.rs` (the pure goal form), `crates/tui/src/ui/run_goal.rs` (its renderer), `crates/tui/src/conversation_label.rs` (decision 42i's recogniser), and `crates/tui/src/app_tests/run_goal.rs`. Modify `keymap.rs` (`C-b g`, `Command::StartGoal`) and `keymap_tests.rs`, `app/mod.rs` (`Modal::StartGoal`), `ui/modal.rs` (one arm), `app/runs.rs` (the hold keys, `pending_open`, the tagged replies of decision 2 for the edit form), `app/run_enter.rs`, `app/headless.rs` (decision 11's refusal, if M9.10 did not already land it), `tree/runs.rs`, `tree/run_rows.rs`, `inspector/run.rs`, `inspector/run_task.rs`, `inspector/run_round.rs`, `theme.rs`, `graph/run_text.rs`, `graph/paint/style.rs`, `ui/conversation.rs` (one call), and their tests.

**Change.** A `planning` run shows `planning` in its header and its orchestrator node; `Reported` tasks get `✓` in the finished colour and the label `reported`; a held task shows `○` and `held` after its stage; a `paused(message)` task shows `‖` in its own colour and the stage text `paused (message)`, and the run's attention line lists it after 600 s; an awaiting gate hold appears in the run's attention list as `hold <id>: <n> tasks wait for approval`, and `a` / `x` send `ApproveHold` / `RejectHold` (with the M8c confirmation prompt for `x`) when the selected node is a held task, or the run's root while exactly one hold is awaiting (with several, the root's `a` toasts `select a held task to approve its hold`); planner nodes use `PlannerInfo` with M8c's content text (`planner {epic} {title}  {merged}/{total}`) and planner glyphs, now filled (decision 33); a research session's round label is `research #n`; the task inspector gains the `messages` row (count, latest kind and first line) and the task's `discovery`/`risk` notes with attribution (decision 42i); a delivered message's user turn in the conversation view is labelled `orchestrator` or `user` from its prefix, with no protocol change. Enter on the run's root already focuses the orchestrator's PTY window (M8c, `app_tests/overview/run_enter.rs:43`); M9 keeps it. `C-b g` opens decision 44's goal form; the M8c edit form and the goal form send tagged requests and match replies by id (decision 2). The reducer stays pure (AGENTS.md rule 5): each key returns `Effect`s.

**Tests first.** `planning_run_header`; `reported_held_and_paused_glyphs`; `hold_attention_line_and_keys_emit_the_requests` (held-task node; root with one awaiting hold; root with two toasts); `planner_nodes_from_planner_info`; `research_round_label`; `messages_row`; `message_pause_and_notes_render_with_attribution`; `delivered_message_turn_is_labelled_by_source` (and a user turn without the prefix is unchanged); `enter_on_the_root_focuses_the_orchestrator` (**pinning**, M8c's test, kept); `goal_form_requires_a_selected_project` (the toast, no effect); `goal_form_sends_start_goal_and_opens_the_run` (a tagged `StartGoal` with `yes: false`, `unconfined_checks: false`, the chosen runtime and model); `goal_form_opens_the_view_only_once_the_snapshot_names_the_run` (`Triaged` first, snapshot second); `goal_form_keeps_its_input_on_error`; `a_refusal_for_an_earlier_request_does_not_reach_the_form` (a `Refused` with another `request_id`); `kill_and_remove_of_a_live_orchestrator_open_nothing` (if not already in M9.10); M8c's mockup tests updated only where a new field appears, each change listed in "Implementation notes".

**The user's submit (M9.7 review fixes, ruling 5, decision 13).** On a `planning` run's root, `s` asks M8c's confirmation prompt `submit the plan of <run> yourself? (y/n)` and, on `y`, sends a tagged `RunRequest::Edit { edits: [], submit: true }`; its reply is toasted as M8c's edit form's is. On any other node or state `s` does nothing new (if `s` is already bound there, pick a free key and record it in "Implementation notes"). Test: `planning_root_submit_key_sends_the_user_submit` (the prompt, then the one effect; and no effect on a running run's root).

**From M9.6's review (M-6).** The snapshot's task-note texts (`TaskInfo.task_notes`) and `last_message_line` are agent-written. The TUI sanitises them when rendering: it strips control characters, U+2028 and U+2029, and the bidi controls (U+200E, U+200F, U+202A–U+202E, U+2066–U+2069). Test: `task_notes_and_message_lines_render_sanitised` (a note and a message line holding each of them render without any).

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
| `packed-refs.lock` under the worker sandbox (owner M9) | Followups file, "From the Claude tool-search fix" | Task M9.13c (closed as "no git change"; a worker-contract line) |
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

### Task M9.3 (Configuration)

- **Where the values come from.** Every default and range is the Interfaces "`config`" block as written. None of M9.3's keys depends on M9.1's outstanding real-CLI checks 1–7 and 10. `MCP_TOOL_TIMEOUT` (decision 10, check 3) is a launch variable of the orchestrator, not a config key, so it is left to M9.10, which builds it as the brief says.
- **Messages the brief did not spell out**, in M8a/M8b's form:
  - `orchestrator.agent.runtime: must be claude or codex (using orchestrator.default_runtime)`;
  - `orchestrator.planners.runtime: … (using the orchestrator's runtime)`;
  - `orchestrator.agent.model: expected a string (using unset)`, as `claude.api_key_helper` does;
  - `orchestrator.agent` or `orchestrator.planners` that is not a table: `expected a table (using table of defaults)`, as `deciders` does.
- **Files outside the task's list.**
  - `config/src/orchestrator/adapt.rs`: `read_strength`, `read_effort` and `sub_table` became `pub(super)` so `agent.rs` reuses them. Their behaviour is unchanged.
  - `daemon/src/decider/mod.rs`: `TriageInput` gains `planner_task_cap: u32`. `run/driver/adapt_goal.rs` fills it from `[orchestrator] planner_task_cap`, which is how "the driver passes it into the triage decider's input". The `TriageInput` literals in `decider/tests.rs`, `decider/tests_prompt.rs`, `run/triage_tests.rs` and `cli/tests/support/decider.rs` gain `planner_task_cap: 12`. The body of `triage_prompt_is_exact_for_a_fixed_input` is unchanged. Its `triage()` helper sets 12, and the test passes against the M8b golden.
  - `run/mod.rs`: `pub mod orch;`. `run/orch/mod.rs` is new and holds only `OrchLimits` and `PlannerLimits`. `OrchLimits::from_config(&config::Orchestrator)` builds them, and `Default` gives the config defaults, which a pre-M9 `run.json` gets through `#[serde(default)]`.
  - `run/engine/tests/view_fields.rs::old_run_json_loads` strips `limits.orch`, the new key, and asserts that it holds the defaults. This is the same treatment M9.2 gave its keys.
- **Types not named outside the config crate.** `lib.rs` must stay unchanged, so `AgentSettings`, `AgentConfig` and `PlannerConfig` are not re-exported. Other crates reach them only through `config::Orchestrator.agent`'s fields. That is why `OrchLimits::from_config` takes the whole `config::Orchestrator`. A later task that needs to name one of these types re-exports it in `lib.rs` as a recorded deviation.
- **`TRIAGE_HEAD`** holds `{cap}` and is rendered with `planner_task_cap`. `PLAN_SCALE_MAX` is deleted. `route` and `TriageRoute` are unchanged.
- **Tests.**
  - `defaults_when_absent`, `each_range_is_enforced_with_the_exact_message`, `effort_and_strength_and_runtime_parse` and `unknown_keys_warn` are in `config/src/orchestrator_tests_agent.rs`, included from `orchestrator_tests.rs` as `mod orch_agent`.
  - `limits_are_frozen_at_run_start` in `run/plan_tests.rs` also checks two more things: the limits survive a `run.json` round trip, and a run without `limits.orch` loads with the defaults.
  - `triage_prompt_uses_planner_task_cap` in `decider/tests_prompt.rs`.
  - All six failed to compile before the change: there was no `Orchestrator.agent`, no `run::orch`, no `RunLimits.orch`, and no `TriageInput.planner_task_cap`.

### Task M9.4 (Plan rules)

- **Before M9.4 (M9.3 review gap), commit `c7ead5c`.** `driver/adapt_goal.rs` builds the triage input in a pure helper, `triage_input(goal, profile, &config::Orchestrator)`, and `the_configured_planner_cap_reaches_triage` pins that a cap of 7 reaches it. It went red (`left: 12, right: 7`) with the helper hard-coding 12.
- **Where decision 23 runs.** `apply_edits` gains `source: &EditSource` (`run edit` in `engine/requests.rs` passes `User`; M9.7 passes the orchestrator's and planners'). After M8a's validation it calls `orch::rules::apply(&mut edited, &touched, source)`, so the rules see exactly the batch's added, split-in and amended tasks. `rules.rs` has `check` as the Interfaces entry gives it, plus `note_unbacked` (decision 23.2's note needs a `&mut Run`, which `check` does not take) and `apply` (both). `EditSource::User` gets neither: plan files and `run edit` keep M8a's rules.
- **Readings of decision 23:**
  - A "finished scout report" is an id in `Run.scout_reports` (M8b decision 19's list, which M9.8's run scouts fill), or `onboarding` while `Run.onboarding_report` is set, which is the same source as `engine/deciders_size.rs::refs_of`. Every `scout_refs` entry is checked on every kind. The "name at least one" rule and the `size not backed by a scout report` note apply to `code` and `docs` tasks only.
  - The cap (23.3) is checked only for the groups a touched task is in: the orchestrator's (no epic), or each touched epic. A group over the cap that the batch did not touch (a user's edits are unchecked) does not refuse an unrelated batch.
  - The reserved-id rule (23.6) is `-int` followed by one or more digits at the end, and it skips a task whose `orch.integration_of` is set (the engine's own review).
  - A sub-planner's added and split-in tasks are given its epic (`EditSource::own`), whatever epic the spec names. Amended tasks keep theirs.
  - Error order: per touched task in plan order, the reserved id, then the epic, the budget and the evidence; then the caps.
- **Model.** `run/orch/mod.rs` now holds `EditSource` (serde snake_case; `label()`), `RunOrch { epics }`, `TaskOrch { integration_of }`, and `EpicRecord`, `PlannerPhase` and `PlannerSession` in full. Only the fields the rules read exist yet; M9.7 and later tasks add the rest of decision 1's fields, each `#[serde(default)]`. They derive `Eq`, because `Run` and `Task` do. `EpicRecord::new` is test-only. `old_run_json_loads` strips the new `orch` keys (run and task) and checks that they are empty.
- **`amend_task deps` (decision 25).**
  - `apply_amend_deps` takes `now` (for `set_state`); this is a deviation from the Interfaces signature.
  - The state refusal is `amend_task`'s per-field form: `task <id> is <state>; deps can be amended only on pending, queued or blocked tasks`.
  - The list is deduplicated. Every new dependency counts as added by the batch, so a cancelled one is refused even if it was already in the list. Unknown ids and cycles are M8a's batch validation, as for `add_dep`.
  - A `blocked(dep_cancelled)` task returns to `pending`; a task blocked for any other reason stays blocked. The history line is `amended: deps`.
  - `Batch` and four of its fields became `pub(super)`, so `impl Batch` in `edits_orch.rs` holds `amend_deps` and the `message`/`refresh` placeholder. The placeholder text is now `message and refresh are not available yet`, and M9.2's pinning tests (`placeholder_edits_refuse_their_whole_batch`, `placeholder_edits_leave_the_run_unchanged`) no longer list `amend deps`.
- **Decision 24 in a new file, `run/validate_kinds.rs`** (not in the task's list). In `validate.rs` it would have reached 555 lines against a budget of 539; with the file, `validate.rs` is 512. The four field errors carry rule `5.2`. The test-mode note is added to every research and review task. `review_target` is split at its first `..`, so `a...b` passes the syntax check as `a` and `.b`, which is what the regex allows; dispatch (decision 36) then fails to resolve it. The tests are in `validate_tests_kinds.rs`, not `validate_tests_fields.rs`, which would have been 604 lines; the deleted M8a test is named in a comment in both.
- **Decision 23a: a finding.** No M8a validation of a planned run calls `may_cover_protected`; its only caller is the fast path's `triage::owned_protected`, which already refuses any non-ASCII entry. So the decision's "M8a's validation requires such an entry to be a literal path" does not follow from the change. What holds is the done gate: a changed non-ASCII path is `protected_changed` unless `owns` names it byte for byte. That covers a wildcard entry as well, because the wildcard never names the path. Both functions changed as the decision says, and no new validation rule was added. M8a's `engine_paths_with_spaces_and_unicode_work` (`daemon/tests/run_git_worktrees.rs`) now expects `src/ü file.rs` in `protected_changed`, not `outside_owns`.
- **File sizes.** `edits.rs` is 593 lines, one over its budget of 577 + 15; it was 588 when M9.4 started. `model.rs` is 584 (budget 592), `validate.rs` 512, `globs.rs` 349. The new files are `orch/rules.rs` 172, `edits_orch.rs` 84 and `validate_kinds.rs` 59.
- **Red before green.** These failed first, as assertions against the old behaviour:
  - the six `edits_tests_orch.rs` tests and the updated placeholder test;
  - `non_ascii_paths_are_protected`;
  - `owns_required`;
  - the five `validate_tests_kinds.rs` tests;
  - four rules tests, which went through the edit path or built research tasks.

  The other rules tests were written after `rules.rs`, so they were run red by stubbing out `check` and `note_unbacked` (restored from a copy): 10 of 14 then failed. The acceptance-only tests (`cancelled_and_integration_review_tasks_do_not_count` and `onboarding_ref_is_accepted`) then gained controls that fail under the stub.

### M9.4 review fixes

These controller rulings answer the M9.4 review of `10114ab`. They replace the M9.4 notes above where the two differ: the scope of the rules, the cap, `EditSource::own`, decision 23a's two functions, and the `a...b` reading.

1. **The non-ASCII rule applies at the done gate only** (Important). `ProtectedMatcher::matches` is back to M8a's answer. The non-ASCII rule is a new method, `ProtectedMatcher::guards_change(path)`, which returns `!path.is_ascii() || matches(path)`. Only `git::verify_done` calls it, when it decides `protected_changed`.
   - `git::protected_files`, `triage::owned_protected` and `validate::protected_notes` behave exactly as on `origin/main`. Before the fix, every tracked non-ASCII path became protected at run start and on the fast path, which contradicts decision 23a's "M8b's fast-path rule … is unchanged".
   - `may_cover_protected`'s non-ASCII branch is reverted too. The ruling puts the non-ASCII rule only at the done gate, and the branch was unreachable anyway: `owned_protected`, its only caller, refuses a non-ASCII entry first. This deviates from decision 23a's second bullet.
   - Test: `daemon/tests/run_git_non_ascii.rs::non_ascii_paths_are_protected_at_the_done_gate_only`. It uses a real repository with tracked `docs/café.md` and `locales/日本語.json` and the built-in protected list. `protected_files` lists only `AGENTS.md`, `owned_protected(["docs/**"])` is `None` and no protected note is added. At the done gate, a changed `src/ü file.rs` is `protected_changed` under `owns = ["src/**"]`, and passes when `owns` names it exactly. The test failed before the fix, because `protected_files` listed both non-ASCII files. `globs_tests.rs::non_ascii_paths_are_protected` now checks `guards_change` against `matches`.
   - **Decision 23a's open question** (controller's ruling): there is no new validation rule for non-ASCII `owns` entries. The done gate enforces it.
2. **The rules apply to what an edit changes** (Important). `rules::check(run, before, source)` and `rules::apply` / `note_unbacked` take the run before the batch instead of the `touched` set. This deviates from the Interfaces signature `check(run, touched, source)`. `apply_edits` passes its input run.
   - A task is *new* when its id is not in `before`: an added or split-in task. A task is *resized* when an `amend_task` changed its `spec.size`. An amend to the size a task already has is not a change.
   - The reserved-id (23.6) and epic (23.5) rules apply to new tasks. The budget (23.1) and evidence (23.2) rules and the unbacked note apply to new and resized tasks. An amend of `priority`, `deps`, `brief` or any other field alone triggers none of them. `add_dep` does not trigger them either.
   - **The cap** (23.3) now compares each group's count after the batch with its count before. A group is refused only when the count rose and is over the cap. A batch that leaves an over-cap group the same or smaller is never refused for it: a split into one task, or a cancel with an add. This replaces the "groups a touched task is in" reading.
   - Tests in `run/orch/rules_tests_scope.rs`, through `apply_edits`:
     - `amends_that_keep_the_size_are_not_ruled` (scenario A: an onboarded run, a task without `scout_refs`, the orchestrator amends `deps`, then `priority`, then `brief`);
     - `an_over_cap_group_refuses_only_a_batch_that_grows_it` (scenario B);
     - `a_size_amend_needs_evidence`.
   - The first two failed before the fix: the `deps` amend was refused for evidence, and the priority amend by the cap. `a_size_amend_needs_evidence` pins behaviour that already held; it guards that the narrower scope still covers a resize. The existing `rules_tests.rs` tests call `check` through a helper, `check_new`, that builds `before` by removing the tasks named as new.
3. **Decision 23.5 against the cap.** Once an epic has a finished integration review, the orchestrator may add a fix task to it past the cap. A finished review is a task with `orch.integration_of == Some(e)` in a finished state. Such an addition is exempt from the epic's `planner_task_cap`.
   - Test: `a_fix_task_after_a_finished_integration_review_is_exempt_from_the_cap`. It builds its state by hand: a `reported` review marked for the epic, with an unfinished-review control. It failed before the fix.
   - M9.9, which produces integration reviews, must keep this test green. M9.9's acceptance says so.
4. **A sub-planner naming another epic is refused, not rewritten** (decision 22). `EditSource::own` / `own_all` are gone. `Batch::add_owned`, `split_owned` and `owned` in `run/edits_orch.rs` enforce the rule:
   - An `add_task` or a split child naming another epic: `task <id>: epic: a sub-planner adds tasks only to its own epic <e>`. An absent `epic` is set to the planner's own.
   - A `split_task` of a task outside the planner's epic (another epic's, or one with no epic): `task <id>: epic: a sub-planner splits only tasks of its own epic <e>`. The wording is the implementer's, since the ruling gave none.
   - Both carry rule `2.epic`.
   - Tests: `a_planners_task_for_another_epic_is_refused` and `a_planners_split_outside_its_epic_is_refused` in `edits_tests_orch.rs`. Both failed before the fix. `a_planners_added_task_inherits_its_epic` now splits a task of the planner's own epic.
   - The other confinement checks stay M9.8's, as the brief assigns them. M9.8's task text now lists them as explicit tests: amend, cancel and `add_dep` on another epic's or the orchestrator's task, and `pause`.
5. **The reserved-id exemption is tested.** `rules_tests.rs::an_integration_review_keeps_its_reserved_id` builds a new `auth-int1` whose `orch.integration_of` is set. Removing the condition from `rules.rs` turned this test red, the only failure in `run::orch`; the file was then restored from a copy.
6. **`review_target`** (controller's ruling, beyond decision 24's per-part regex).
   - No part may start with `.`, and an empty part stays refused. So `a...b`, `.a..b` and `.main` are refused.
   - The whole target is at most 200 characters, matching the MCP schema (`REVIEW_TARGET_MAX` in `run/validate_kinds.rs`).
   - `review_target_syntax` gained `a..b` and a 200-character range, both accepted. It also gained `a...b`, `.a..b`, `a..`, `..b`, `.main` and a 201-character range, all refused. The old "two full-length parts are accepted" case (401 characters) is gone.
   - The test failed before the fix, on `a...b`.
7. **M9.9 obligation.** When M9.9 creates `<e>-int<n>`, it picks the first free round number, because a user's `run edit` may already hold that id. It is in M9.9's task text as `integration_review_takes_the_first_free_round_id`.

**File sizes.** `edits.rs` is 591 lines, under its budget of 592; the owned-edit logic moved to `edits_orch.rs`. The test file `orch/rules_tests_scope.rs` is new.

#### Second review

These controller rulings answer the second M9.4 review, of `2e2c8ca`. They replace review fix 3 above where the two differ.

1. **Only a reported integration review lifts an epic's cap** (Important). `rules::integration_reviewed` now requires `TaskState::Reported`, not `is_finished()`, which includes `Cancelled`. It reads `before`, the run before the batch, so a batch cannot cancel a pending review to exempt its own additions.
   - Tests in `run/orch/rules_tests_scope.rs`:
     - `cancelling_the_review_in_the_batch_does_not_lift_the_cap`: `[cancel auth-int1 (pending), add a3, add a4]` over the cap is refused;
     - `a_cancelled_review_does_not_lift_the_cap`: an already-cancelled review does not exempt; its control, a `reported` review in `before`, does.
   - Both failed before the fix: each batch was accepted, since a cancelled review counted as finished.
2. **The exemption stays unlimited**, as review fix 3 worded it. This is deliberate: once an epic's integration review has reported, the orchestrator may add any number of fix tasks to it past the cap. `a_cancelled_review_does_not_lift_the_cap`'s control adds two. M9.5's tuning may revisit this.
3. **The epic (23.5) and reserved-id (23.6) rules apply to new tasks only**, now pinned:
   - `resizing_a_task_of_a_live_planners_epic_is_accepted`: the orchestrator resizes a user's task in an epic a sub-planner is still planning, with evidence (`scout_refs = ["onboarding"]`);
   - `resizing_a_users_task_with_a_reserved_id_is_accepted`: the orchestrator resizes a user's own task named `x-int1`, with evidence.
   - These are pinning tests and passed before the change. Each goes red under the mutation that applies its rule to every changed task: dropping `new &&` from the reserved-id condition fails only the second, and replacing `new` with `true` in the epic rule fails only the first. `rules.rs` was restored from a copy after each.
4. **An unknown task is reported before any epic rule.** A sub-planner's `split_task` of an unknown id reports `task <id>: task_id: no such task`, not an epic error for its would-be children. `Batch::split_owned` checks existence first, through `Batch::find`, now `pub(super)` so `edits_orch.rs` can call it.
   - Test: `edits_tests_orch.rs::a_planners_split_of_an_unknown_task_reports_the_unknown_task`, which splits `ghost` into a child naming another epic. It failed before the fix, reporting only the child's epic error.

### Task M9.5 (Contracts and prompts)

- **The texts.** `ORCHESTRATOR_CONTRACT` and `PLANNER_CONTRACT` were copied from Interfaces "Contracts (exact)" by script, not by hand, as were the worker contract's lines 10 and 11. `orchestrator_contract_is_exact`, `planner_contract_is_exact` and `worker_contract_is_exact` compare them byte for byte with a second copy made the same way. Every row of "Prompts and messages" is a `const` or a function in `run/orch/contract.rs`.
- **The worker contract has 12 lines, and no `packed-refs.lock` line.** M9.1 check 11 assigns that line to task M9.13c ("What M9.13c still owes is a line in the worker contract"), and M9.5's `worker_contract_is_exact` fixes the contract at 12 lines. So M9.13c adds it, and updates this test to 13 lines. *(Done in M9.13c: the contract now has 13 lines.)*
- **Readings of the prompt table:**
  - **`review_task_prompt` takes `patch`**, which the table's signature leaves out. The table has a clamp line, "The diff below was cut at 16 KiB", and `REVIEWER_CONTRACT` says "The change's diff is in your prompt", but the table's lines never include the diff. So after the acceptance criteria come `Diff (<b>..<h>):` and the patch, clamped to `REVIEW_DIFF_MAX`, as in `reviewer_prompt`. The brief stays last. The clamp line appears when the clamped patch is shorter than the patch. An S task is reviewed at `small`, and any other size at `medium`.
  - **Both planner prompts end with `epic.request`**, the live or last session's brief. The engine sets it to the `spawn_subplanner` brief on each start. `integration_review_prompt` ends with `epic.brief`, which is what the epic asked for.
  - **`replan_prompt` places the epic's current tasks** directly before `What to plan:`, after the blank line. It writes `- none` for an epic with no tasks, as `planner_prompt` does for no planned tasks.
  - **`orchestrator_first_prompt`** writes `Path: <path>` with no parentheses when the run has no triage, and `fallback` alone when the fallback has no reason.
  - **Earlier findings in `integration_review_prompt`** are the critical and important findings of every task with `orch.integration_of == Some(e)`, in plan order.
  - **`message_text`** labels a `Planner` source `orchestrator`. A sub-planner never sends a message (decision 22), so this never happens.
  - **`refresh_clean`** is `refresh_clean(n, list: &[(sha, subject)])`. It names at most 10 commits, then `and <n - named> more`. It always writes `commits`, as the table does, even for one.
  - **`notes_section` and `worker_messages_for_review`** sort by `at`, stably. Each returns an empty string when there is nothing to show, and the prompts then leave the section out.
  - **`hh:mm`** comes from a UTC helper in `run/orch/contract.rs`, because M9.7 deletes `model_adapt::hh_mm`.
- **The scout extract (decision 34).**
  - Each report is written as `Scout report <id>:`, then the summary cut to 4000 characters, then `Files: <paths joined with , >`, or `Files: none` when there are no files. Reports are joined by one newline.
  - Past 12 KiB the joined text is cut at a character boundary, so the later reports go first. It then ends with `EXTRACT_CUT_MARKER`, `\n[anthrex] The later scout reports were cut here to fit.`, whose wording is the implementer's.
  - An unreadable report is dropped by a new pure helper, `readable_reports(Vec<(id, Result<ScoutReport, String>)>)`, which logs a `tracing::warn!` for each one it drops. `scout_extract` keeps the Interfaces signature.
- **Model (outside the file list).**
  - `run/orch/mod.rs` gains `TaskMessage` (Interfaces). The `TaskOrch.messages` field is left to M9.13a.
  - `RunOrch.yes` (Interfaces; M9.7's `make_planned` sets it) is added now, because the first prompt's plan-gate line reads it. `old_run_json_loads` still passes.
- **`WAKE_MAX_BYTES`** is defined in `run/orch/contract.rs`, because `wake_text` clamps with it. Interfaces place it in `run/driver/wake.rs`; M9.13 should use this one, not define a second.
- **`run/contract.rs`.**
  - Production code grew 27 lines, against a budget of 10. That is the two contract lines, one new parameter on `reviewer_prompt`, two each on `worker_prompt` and `handover_prompt` (rustfmt wraps `handover_prompt`'s signature), and the section loop. `engine/dispatch.rs` grew 1 line, to 590, because the first worker prompt is now a closure.
  - The file is still 557 lines, down from 584, because the diff-clamp unit tests moved from its inline `mod tests` to `run/contract_tests.rs`, unchanged.
  - `floor_boundary` became `pub(crate)` for the extract's cut.
- **Test files.** The prompt tests are in a second file, `run/orch/contract_tests_prompts.rs` (435 lines), beside `contract_tests.rs` (349). One file would have been over 700 lines. `wake_text_is_clamped` has an exact-text companion, `wake_text_is_exact_and_clamped`. `short_texts_are_exact`, `planned_message_is_exact`, `refresh_texts_are_exact`, `scout_extract_is_exact`, `notes_section_is_exact` and `worker_messages_for_review_is_exact` cover the table rows that have no test named in the task. `contracts_do_not_vary` checks that neither contract contains either run's id, goal or root, and that the two first prompts differ.
- **Left open.**
  - The engine's callers pass `""` for `worker_prompt`/`handover_prompt`'s `extract` and `notes` and for `reviewer_prompt`'s `messages` (`engine/dispatch.rs`, `engine/review.rs`). No task's text says who fills them.
    - Decision 34 says the driver reads the reports when it builds `OpKind::CreateWindow`, and puts the extract in `first_turn`. That fits M9.8, which feeds the same extract to `planner_prompt`.
    - The notes and messages come from `Task.orch.messages`, which is M9.13a's.
  - A controller ruling should assign both.
- **Outstanding real-CLI checks.** The contracts name each tool as `<tool> (in Claude: mcp__anthrex__<tool>)`, the tool-search fix's form, as the brief says. Whether a real Claude session sees those ids is M9.1 check 10, which is still outstanding. The wake text's paste delivery is check 6, and is M9.13's. No other M9.5 text depends on checks 1 to 7 or 10.
- **Red before green.** Every new test failed to compile before the change: there was no `run::orch::contract`, `TaskMessage` or `RunOrch.yes`, and the prompt functions had the old signatures. With the new code in place, removing lines 10 and 11 from `WORKER_CONTRACT` again failed four tests: `worker_contract_is_exact`, `contracts_round_trip_through_toml_string`, `change_message_requires_acknowledgement_in_task_done` and `contracts_name_every_tool_by_its_claude_id`. `contract.rs` was then restored from a copy.

### M9.5 review fixes

- **Ruling 1: a scout report stays inside its section.** A scout summary is repository-derived, untrusted text, and a summary holding `\n\nWhat to plan:\n...` or a line starting `[anthrex]` made a fake section in `planner_prompt` and `worker_prompt`.
  - `scout_extract` now indents every summary line, an empty one too, with two spaces, so no report line starts at column 0. Each `\n` and `\r` of a file path becomes a space.
  - **Beyond the ruling:** a lone `\r` and a `\r\n` in a summary also end a line, so each is indented like a `\n`. Without that, `ok\rWhat to plan:` would pass as one indented line.
  - The 4000-character summary cut happens before the indent, and the 12 KiB cap after it, as before.
  - Tests: `summary_cannot_open_a_section_of_the_prompt` (the review's summary, through `planner_prompt` and `worker_prompt`: no column-0 `What to plan:` or `[anthrex]` line from the report, and the real `What to plan:` exactly once at column 0) and `file_path_with_a_newline_stays_on_its_line`. `scout_extract_is_exact`, `extract_is_capped_at_12_kib_with_a_cut_marker`, `planner_prompt_is_exact` and `run/contract_tests.rs`'s `EXTRACT` were updated for the indent.
  - **File move.** The fix would have taken `run/orch/contract.rs` past 600 lines, so the extract (`EXTRACT_*`, `readable_reports`, `scout_extract`) moved to a new `run/orch/extract.rs` (77 lines), its tests to `run/orch/extract_tests.rs`. `run/orch/contract.rs` re-exports all five, so Interfaces' `run/orch/contract.rs` path and every caller still work. `contract.rs` is now 546 lines, `contract_tests_prompts.rs` 345 and `extract_tests.rs` 177.
  - Red before green: the four extract tests that check the new layout failed on the old code, each showing the summary at column 0 or the path's raw newline.
- **Ruling 2: obligations recorded in the task texts.** M9.8 wires the extract into the worker's first turn and handover. M9.13a wires the notes and saved messages into the worker, handover and reviewer prompts, folds newlines in a message's text, and calls `refresh_clean` only with `n >= 1` and `n == list.len()`. M9.13 reuses `WAKE_MAX_BYTES` from `run/orch/contract.rs` (Interfaces' `run/driver/wake.rs` entry and the `wake_is_clamped` text now say so). This supersedes the "Left open" item of "Task M9.5" above.

### Task M9.6 (The digest, the context, the task result and the snapshot)

- **Decision 16a's 256 KiB bound cannot hold: a finding.** Measured with `proto::encode` (named MessagePack, as the wire sends it), 50 complete runs of 20 tasks with 4 KiB briefs encode to 5 699 030 bytes before the change and 1 098 030 after. The remaining ~1.1 KiB a task is `TaskInfo`'s other fields (about 70 named keys, routes, budget, spend, phases, history), not plan text. Reaching 256 KiB would need terminal runs' tasks slimmed in the snapshot, which is a design decision of its own, so it was not attempted. `a_snapshot_of_fifty_terminal_runs_stays_small` pins what 16a removes instead: the same runs encode to exactly the same bytes with 4 KiB briefs as with 1-byte ones, and under 1.25 KiB a task (`SNAPSHOT_BOUND`). **Needs a controller ruling** if the 256 KiB number matters.
- **Model (`run/orch/mod.rs`), added now as the task allows.** `RunOrch` gains `orchestrator`, `gate_holds`, `run_scouts`, `digest_rev`, `digest_fp`, `installed`, `research_report`, `planner_usage`; `TaskOrch` gains `gate_hold`, `research`, `review_range`, `messages`, `worker_notes`, `refresh`, `refresh_merges`; the types `OrchestratorRecord`, `GateHoldRecord`, `RunScout`, `RunScoutState` (with `label()`), `WorkerNote` and `RefreshState` are as in Interfaces. Every field is `#[serde(default)]`. `run/model.rs` is unchanged: `Run.orch` already existed.
  - **Deviation: `RunOrch.digest_read_at: Option<u64>`**, not in Interfaces: when the last `DigestRead` arrived. The digest's `gate.holds` keeps an approved hold decided at or after it (all decided holds while it is `None`). **Obligation for the task that handles `OrchEvent::DigestRead` (M9.9's `digest_read_drops_notes_up_to_the_revision`): set it to the event's `now`.**
  - `scout::report::ScoutReportArgs` derives `Serialize`/`Deserialize`, so `TaskOrch.research` persists.
  - `edit_log::PlanEditRecord` gains decision 40's four fields now (`source` defaults to `user`, `accepted` to true), so the digest can show them; `record` still writes a user's accepted batch. Its signature and the snapshot's `PlanEditInfo` mapping stay M9.9's. `edit_log_tests.rs`'s one struct literal gained the fields.
- **The fingerprint (deviation).** It hashes the **untrimmed** digest (trimming depends on the size, which counters change). Beyond decision 16's removals (`revision`, `now`, `spend`, `slots`, each task's `last`), it also leaves out the wake `notes` and the approved holds kept only because they were decided since the last read: both change *because of* a read, and a read must not bump `digest_rev` (else an orchestrator is answered at once after every read). Every note comes with a state change the fingerprint already sees. Test: `a_read_does_not_change_the_fingerprint`; mutating either removal back in turns it red.
- **The digest's readings.**
  - `gate.state`: `planning` and `awaiting_approval` from the run state; `none` for a `Fast` run with no orchestrator, or a run with no `approved_at`; else `approved` with `at` (`hh:mm` UTC of `approved_at`).
  - `run.complete` is true for `complete` and `accepted`. `run.base` is `<base_branch>@<sha7>`.
  - `task.hold` is the task's hold id while that hold is not approved (`counts.held` counts the same). `review` lists every severity with findings: `r1 changes (1 critical, 2 important)`. `integration[].task` is the epic's latest `<e>-int<n>` task and `findings` its last verdict's `N critical, M important`, else `null`.
  - A finished task's age, for trimming, is its newest history entry.
  - Texts not in the Interfaces' cut list are bounded too: the goal at 2000 characters, a title at 120, and at 300 the attention lines, wake notes, `last`, `halted_reason`, a planner's `last_rejection` and `note`, and an edit's `error`. The block-text trim step (to 200) cuts the attention lines with it.
  - **Beyond the documented trim order**, so the cap always holds: unfinished tasks dropped from the end of the plan (counted in `omitted_tasks`), then every string cut to 120 characters, then `attention`, `scouts`, `planners`, `integration` and `notes` cut to 10. Test: `the_cap_holds_whatever_the_run_holds`.
- **Untrusted text (carry-forward rule).** All three answers are JSON that `serde_json` escapes, so no `\n` or `\r` reaches a model as a line break. `json::fold_all` also folds the three breaks JSON leaves raw (U+0085, U+2028, U+2029) into spaces, over the whole answer. Tests: `untrusted_text_stays_inside_its_json_string` in the digest, context and result suites, each with a text that tries to open an `[anthrex]` line and a `What to plan:` section. The first version failed the context test: a scout file's `why` carried a raw U+2028, which is why the fold runs over the whole answer.
- **The context's readings.** The onboarding report is listed first, as `onboarding`. Then every run scout in order, with its report's fields when the driver passed one (else `summary: null` and empty lists: a queued scout is listed). Then any other report. A planner's filter applies with `scouts: [ids]` too. `profile.summary` is `profile::summary` of the stored profile, `null` without one; the lists and commands are the run's resolved `Run.profile` (what the engine enforces). A runtime missing from `installed` (a run built before the check) is not installed. An epic's `state` is `planning` for a queued planner too (`PlannerState` has no queued). `omitted.scouts` counts the reports shortened (summary cut or file list dropped), in the order the decision gives; beyond it, so the cap holds: their modules, interfaces and risks, then strings cut to 300, then later reports left out (counted).
- **`task_result` (deviation in the signature).** `task_result(run, task, git: Option<&Result<TaskGit, String>>)`: Interfaces' `Option<&TaskGit>` cannot carry the git error that `git: "<error>"` needs. `None` is a task with no start commit. `run` is unused (kept for the Interfaces shape). `checks[].summary` is the decider's summary when there is one, else M8a's tail summary. `research` leaves out the report's `profile`. Beyond the documented trims: the last 20 history lines, the last 10 messages and task notes, the last 2 reviews, then strings cut to 2000, 500, then 200 characters. Test: `the_cap_holds_whatever_the_task_holds`.
- **`tools.rs`.** Problems use M8a's worker-tool wording (`<field>: required`, `must be 1 to N characters`, `must be a string`, `unknown field`, `at most N items`, `must match <pattern>`). A `message` edit's text over 4000 characters is refused in parsing, as `edits[<i>]: message: text: at most 4000 characters`. The `override` refusal is serde's whole text, `unknown variant \`override\`, expected one of …`; the test checks the documented prefix. An unknown tool gets the role refusal.
- **`run/git/summary.rs`.** Every read goes through `Git::read` (`NO_HOOKS`, `-C`, `--no-optional-locks`, the scrubbed environment). The log adds `--no-color`, the diff M8a's `DIFF_FLAGS`. The diffstat keeps its first 59 lines and its total line. `rev-parse` passes `--end-of-options`. A single revision's base is `git merge-base <base branch commit> <head>`. An unresolvable side answers `<rev>: <stderr tail>` (git's own text does not name it), for M9.9's `review target <t> does not resolve: …`. `task_summary` and `resolve_target` were also added to `tests/run_git_env.rs`'s one test, which plants `GIT_DIR` and `GIT_INDEX_FILE` (a pinning addition; it passed at once).
- **The snapshot.** Besides 16a's gate-only plan text, `TaskInfo.hold`, `message_count`, `last_message_kind`, `last_message_line` (first line, control characters dropped, 80 characters) and `task_notes` (the last 10, oldest first) are filled, and `RunInfo.digest_revision`. **Deviation: each snapshot task note's text is cut to 400 characters** (`SNAPSHOT_NOTE_MAX`): 16a bounds the count only, and 10 notes of 4000 characters on every task would ride on every push. The M9 helpers are in a new `run/snapshot_orch.rs`, so `snapshot.rs` is 364 lines (336 when the task began).
- **The reducer.** `engine/mod.rs::finish` calls `digest::note_change` for a run that was in the state before the step and changed structurally. A run new to the state (a restore, a start) is left as loaded: bumping it there rewrote every restored finished run's `run.json` at daemon start, which M8a's final review B-5 forbids (`restoring_an_unchanged_finished_run_writes_nothing` went red). A pre-M9 run's first change then bumps `digest_rev` once. `engine/mod.rs` is 590 lines (+6 of its +15).
- **The overlay.** `run/driver/orch.rs::with_scouts` sets each `RunInfo.scouts` from `ScoutService::run_scouts`, in `current()` (a `List` answer, a subscription's first snapshot) and `publish()` (every push), with the engine lock released. `driver.rs` is 592 lines. Its test, `run_scouts_appear_in_the_snapshot`, is in-crate (`run/driver/orch_tests.rs`). It uses a real manager, `profile::service::wire` and a `/bin/sh` stand-in for Claude that reads its stdin; Codex is a path that does not exist. It kills only the window it started.
- **Tests outside the named files:** `engine/tests/digest.rs`, `run/snapshot_tests.rs`, `run/orch/test_support.rs` (shared builders), `crates/daemon/tests/run_git_summary.rs` (the blocking suite), plus `note_change_bumps_the_revision_only_on_a_new_fingerprint`, `diffstat_keeps_at_most_60_lines_and_its_total`, `invalid_arguments_name_the_field_and_the_problem`, `file_lists_go_after_the_summaries`, `last_message_line_is_one_clean_line_of_at_most_80_characters`, `the_rest_of_a_task_is_still_published` and `the_digest_revision_is_published`. `old_run_json_loads` now compares a task's `orch` with `TaskOrch::default()`.
- **Red before green.** The context, result, tools, summary, snapshot, driver and engine tests were written first and failed against stubs that compiled with the Interfaces signatures. Two passed against their stubs: `task_without_start_has_no_git`, which asserts absent keys that an empty stub also lacks, and `the_rest_of_a_task_is_still_published`, which is pinning. The digest was written before its tests, so it was run red by stubbing `digest` and `fingerprint` (all 11 failed), and by two targeted mutations: keeping `last` in the fingerprint failed `counter_changes_do_not_change_the_fingerprint`; keeping `notes` and every decided hold failed `a_read_does_not_change_the_fingerprint` and `gate_states`. Making `plan_text_shown` always true failed both 16a tests. `digest.rs` and `snapshot.rs` were restored from copies each time.
- **For later tasks.** M9.7 fills `RunInfo.holds` and `orchestrator`. M9.9 fills `PlanEditRecord`'s fields through `record` and maps them into `PlanEditInfo`. M9.11's read path calls `task_summary` on `spawn_blocking` with `DONE_CHECK_GIT_TIMEOUT` only when the task has a start commit, and passes `None` otherwise.

### M9.6 review fixes

- **I-1: a restore moves the digest.** `engine/restore.rs` calls `digest::note_change(run)` inside its `if *run != original { run.revision += 1; }` block, so a restored run the restore changed (a running run paused) gets a fresh `digest_fp` and a higher `digest_rev`. An unchanged run is still left as loaded: `restoring_an_unchanged_finished_run_writes_nothing` stays green. Test: `engine/tests/digest.rs::a_restore_that_changes_the_run_moves_the_digest` (red before: `digest_fp` was the pre-restore fingerprint).
- **I-2: `task_result` always fits 64 KiB.** After the existing trims (the string cuts to 2000, 500 and 200 included), `result.rs` now:
  - cuts the kept reviews' findings to 20, 10, then 5 in all, the newest review's first;
  - then cuts each research list (`files`, `modules`, `interfaces`, `risks`, first entries kept) and the proofs (newest kept) to 20, 10, 5, then none;
  - finally shrinks every string to 100, 40, then 16 characters.
  - What the count trims drop is counted in a top-level `omitted` object (`findings`, `research`, `proofs`), present only once something was dropped.
  - **Deviation:** the findings trim stops at 5, not none, so the newest verdict keeps its reasons. The hard shrink still brings the answer under the cap: every other list is already bounded (history 20, messages and notes 10, reviews 2, commits 50, diffstat 60 lines).
  - `task.task_notes` is now written field by field (`at`, `kind`, `text`), so M-2's `seq` stays out of the answer.
  - Test: `the_cap_holds_at_the_schema_maxima` (3 reviews × 50 findings with every text at its MCP maximum, a full research report, 50 proofs, 50 messages, 100 notes, 500 history lines and a 4000-character git error, in ASCII, `é` and `語`). The answer is at most 64 KiB and parses back. Red before: 142 065 bytes in ASCII.
- **I-3: the digest trims text before it drops an unfinished task.** The documented order is kept up to `notes` → 5. Then:
  - `attention` loses the lines of tasks already dropped and is cut to 10;
  - every string is cut to 120 characters;
  - `scouts`, `planners`, `integration` and `notes` are cut to 10;
  - only then are unfinished tasks dropped from the end of the plan, and the attention lines of the tasks dropped there are removed with them.
  - **Deviation:** the ruling moved the attention cut and `shrink_strings` before the unfinished-task drop. The four list cuts moved too, because they also trim before a task goes. An attention line of a dropped task is removed rather than marked omitted. `omitted_tasks` counts the task.
  - Tests: `texts_are_shortened_before_an_unfinished_task_is_dropped` (50 blocked tasks, 500-character CJK reasons, default `max_tasks` 50: all 50 kept, texts at most 201 characters; red before: 27 omitted) and `attention_names_only_tasks_the_digest_shows` (200 blocked tasks; red before: every task was dropped, so `tasks` was empty).
  - `past_the_finished_tasks_edits_notes_and_blocks_are_cut` asserted each block text was exactly 201 characters, which relied on the old order dropping tasks instead. It now asserts at most 201 and that no task is omitted.
- **M-1: the context holds 96 KiB.** After the existing string cut to 300, and before later reports are left out, `context.rs` trims:
  - the plan: every `owns` cut to 3 globs (`omitted.owns`), then the finished tasks left out (`omitted.tasks`), then every `owns` emptied;
  - the profile's five lists emptied;
  - the epics' areas emptied, then later epics left out (`omitted.epics`);
  - `you.epic`'s area emptied and its brief cut to 1000 characters.
  - Then later reports are left out as before, then every string is cut to 120, 40 and 16 characters, and last, later plan tasks are left out (`omitted.tasks`).
  - The new `omitted` keys appear only when non-zero, so `omitted` is still `{"scouts": n}` otherwise.
  - **Choice:** the plan trims run before any report is dropped, so a big plan never costs the planners the onboarding report.
  - Test: `a_large_plan_stays_under_the_cap` (200 tasks × 20 owns of about 90 characters, half merged). The answer is under 96 KiB, the onboarding report is kept, and `plan` plus `omitted.tasks` is 200. Red before: 405 165 bytes.
- **M-2: a new note always moves the fingerprint.**
  - **Deviation (model):** notes have no run-wide order to break a tie with, so `WorkerNote` gains `seq: u64` (`#[serde(default)]`, 0 for an older note) and `RunOrch` gains `note_seq`. `run/orch/mod.rs::add_worker_note(run, task, note)` appends a note with the next `seq`.
  - The digest sorts task notes by `(at, seq)`, newest first. Notes with no `seq` keep the old stable plan order.
  - **Obligation for M9.13a:** every `task_note` is stored through `add_worker_note`.
  - Test: `a_new_note_in_the_same_second_moves_the_fingerprint` (the reviewer's case: 10 notes on `t3` at T, then a `risk` note on `t0` at T; the fingerprint moves and the risk note is `task_notes[0]`). Red before: equal fingerprints.
- **M-3: `run/git/summary.rs` hardening.** `task_summary` refuses a `start` or `branch` that begins with `-` (`<rev>: a revision cannot start with '-'`). It also passes `--end-of-options` before the range in `git log` and `git diff`.
  - Test: `tests/run_git_summary.rs::a_start_that_reads_as_an_option_is_refused`. `start = "--output=<path>"` returns an error, and so does a branch `-p`. No file is written under the target directory, whose parent directories the test creates so a leaked option would succeed. Red before: `Ok` with empty output.
  - `task_summary_lists_commits_and_diffstat` now expects `--end-of-options` in both argvs.
- **M-5: the scouts overlay runs once per tick.** `driver.rs::on_tick` publishes the engine's raw snapshot, and `publish` lays the scouts over it. Before, `current()` and then `publish` each applied it. This is structural, so there is no new test. `run_scouts_appear_in_the_snapshot` stays green, and `driver.rs` stays at 592 lines.
- **For later tasks:** M-4 is added to M9.11's text and M-6 to M9.15's.

#### Second review

- **Important 1: the tick published under the engine lock.** `driver.rs::on_tick` called `self.publish(snapshot(&crate::lock(&self.state), unix_now()))`, whose temporary guard lives to the end of the statement, so the engine lock was held through `with_scouts` (the scout table's lock) and `pushes.send`. The snapshot is now bound first, so the guard drops before `publish`, with a comment naming AGENTS.md rule 2.
  - Audit: `current()` already binds the snapshot in a `let` (the guard drops there; a comment now says so). `Ready::Publish` snapshots are taken in `effects::prepare` under the lock and published by `execute`, which runs after the lock is released. No other `publish(` or `with_scouts(` caller.
  - Test: `run/driver/orch_tests.rs::the_tick_publishes_outside_the_engine_lock`. It holds the scout table (a `#[cfg(test)]` `ScoutService::with_table_held`) on a thread, runs a tick with a publish due, waits until the tick has taken `publish_due`, then takes the engine lock on a blocking thread within 3 s; it then releases the table and expects the push. Red before (the engine lock was not free). The one gap is a false green, never a false red: the probe could take the lock in the few instructions between the tick reading `publish_due` and taking the engine lock.
- **Important 2: run-wide attention lines survive the cut.** The attention cut (new `digest_trim.rs::cut_attention`) puts the run-wide lines (a moved base, a failed final check, a stale profile, a promotion) ahead of the blocked tasks' `<id> blocked (` lines, then keeps 10: only blocked tasks' lines are cut to fit. A line counts as a task's only when its id is one of the run's tasks. Test: `run_wide_attention_lines_survive_the_cut` (50 blocked tasks, 500-character `語` reasons, `final_check_failed`; red before: the line was cut, `attention[0]` was `t0 blocked (question): …`).
- **Minor 1: a second shortening pass.** After the list cuts to 10, the scouts' and planners' strings are cut to 40 characters, then both lists to 3 entries, before an unfinished task is dropped. Test: `scout_and_planner_texts_are_shortened_before_a_task_is_dropped` (10 blocked tasks, 50 failed scouts and 50 planners, every text at its bound, in `\u{1}` and `𝄞`: all 10 tasks shown). Red before for `\u{1}` (6 tasks omitted). The `𝄞` case passes before and after: at 4 bytes a character the first pass already fits; it stays as a guard.
- **Minor 2: the context cuts strings before it drops a report.** After the plan trims, every string is cut to 120, then 40 characters; then later reports are left out with the onboarding report kept; then strings go to 16; only then is the onboarding report left out; last, later plan tasks. Test: `a_large_unfinished_plan_keeps_the_onboarding_report` (200 unfinished tasks, 120-character titles, 20 dependencies with 11-character ids; red before: every report dropped, `omitted.scouts` 4).
  - **Choice:** the ruling says the onboarding report goes last of all reports; it also goes after the cut to 16 characters, which the other reports do not wait for.
- **Minor 3: `edits[].recipients` is capped at 20** (`RECIPIENTS_SHOWN`) in the digest, with `recipients_omitted` present only when some were left out, so the Interfaces' shape and the fixture are unchanged. The fingerprint sees the capped list and the count. Test: `edit_recipients_are_capped_with_an_omitted_count` (10 edits × 500 recipients: under 48 KiB, 20 shown, 480 counted, no task omitted; red before: 6 tasks omitted).
- **Minor 4: the first round's fixes are pinned.** Each was run red under its mutation, and the source restored from a copy:
  - `the_cap_holds_at_the_schema_maxima` now asserts `omitted.findings > 0`, `omitted.research > 0` and `omitted.proofs > 0`, and that kept plus omitted is 160 research entries and 50 proofs. With the findings trim disabled it fails (`omitted` was `{"proofs":50,"research":160}`); with the research and proof truncation disabled it fails (`{"findings":95}`).
  - `attention_names_only_tasks_the_digest_shows` now uses 50 blocked tasks with 400 dependency ids each, so fewer than 10 tasks are shown and the attention cut's 10 lines must lose those of the tasks dropped after it; it asserts the blocked lines name exactly the shown tasks. Removing the final `drop_attention_of_dropped_tasks` fails it.
  - `a_start_that_reads_as_an_option_is_refused` asserts both errors contain `a revision cannot start with '-'`. With the check removed it fails: git's own `fatal: option '--output=…'` refusal no longer satisfies it.
- **Minor 5** is added to M9.11's text.
- **Files.** `digest.rs`'s trimming moved to `run/orch/digest_trim.rs` (a child module), and the trimming tests to `digest_tests_trim.rs`, so every file stays under 600 lines (`driver.rs` is 597).

### M9.1 real-CLI checks (2026-09-28, authorized by the user in chat)

- - **Checks 1–7 and 10 (real CLIs, 2026-09-28).** Run by a subagent, with the user's authorization of 2026-09-28 ("go ahead with the real CLI checks").
    - **Setup.**
      - **Versions.** `claude --version` gives `2.1.280 (Claude Code)`. `codex --version` gives `codex-cli 0.156.1`.
      - **Where they ran.** Every session ran in a scratch `git init` repository under the session scratchpad (`…/scratchpad/m9.1/repo{A,B,C,D,3,10}`). None ran in this repository or through an anthrex daemon.
      - **Stub MCP server.** `stub_mcp.py` is a Python stdio JSON-RPC server that handles `initialize`, `tools/list`, `tools/call` and `ping`. It has one role per check:
        - `orchestrator`: the six tools `get_context`, `spawn_scout`, `spawn_subplanner`, `edit_plan`, `run_status` and `task_result`;
        - `worker`: `task_done`, `task_blocked` and `task_note`;
        - `slow`: `slow_sleep`;
        - `codex`: `run_status`, which connects to a Unix-socket listener.
      - **Hooks.** A `--settings` hook logger recorded every event: the 10 of `HOOK_EVENTS`, plus `StopFailure`, `PreCompact` and `PostCompact`.
      - **Environment.** Each session ran in the caller's environment with these removed: `CLAUDE_CODE_*`, `CLAUDECODE`, `CLAUDE_PID`, `CLAUDE_EFFORT`, `CLAUDE_AGENT_SDK_VERSION`, `MCP_*` and the API-key variables. That emulates a daemon started from a terminal.
      - **Driver.** The interactive sessions were driven through a Python `pty` wrapper:
        - 180×50 terminal;
        - a hard limit of 4 minutes per session;
        - the child pid alone is killed at the end, with SIGTERM, then SIGKILL after 3 s.
      - **Models.** `--model haiku` and `--effort low` for Claude; `-m gpt-5.6-luna` for Codex.
    - **1. Interactive Claude flags: holds.**
      - **Help.** `claude --help` lists all ten flags: `--mcp-config <configs...>`, `--allowedTools`, `--disallowedTools`, `--append-system-prompt`, `--strict-mcp-config`, `--effort <level>` (low, medium, high, xhigh, max), `-n, --name`, `--settings <file-or-json>`, `-r, --resume [value]` and `--setting-sources <sources>`.
      - **Command.** Decision 7's argv, run interactively, with `ENABLE_TOOL_SEARCH=false`:
        ```
        claude --name orch-probe-A --settings '<hooks json>' --setting-sources user --strict-mcp-config --mcp-config '{"mcpServers":{"anthrex":{"type":"stdio","command":"python3","args":[stub_mcp.py,--role,orchestrator,…]}}}' --allowedTools mcp__anthrex__get_context,…,mcp__anthrex__task_result,Read,Glob,Grep --disallowedTools Edit,Write,NotebookEdit,Bash,Agent --append-system-prompt '<probe contract>' --effort low --model haiku -- 'Call the tool mcp__anthrex__get_context once, then reply with just the word READY.'
        ```
      - **Startup.** The TUI started. In a folder never trusted before, it first showed the workspace trust dialog, with "No, exit" as the default choice: Down, then Enter, accepts it. After that dialog, the prompt after `--` was submitted with no further input: `UserPromptSubmit` carried it verbatim.
      - **First turn.** The hook sequence was `SessionStart, UserPromptSubmit, PreToolUse/mcp__anthrex__get_context, PostToolUse/mcp__anthrex__get_context, Stop`, with `Stop` 7.2 s after start. There was no `ToolSearch` call; the stub logged one `tools/call get_context`.
      - **`/mcp`.** It shows `anthrex · ✔ connected · 6 tools`. The details screen says `Status: ✔ connected · Config location: Dynamically configured · Tools: 6 tools`. "View tools" lists `get_context, spawn_scout, spawn_subplanner, edit_plan, run_status, …`.
      - **`--effort`.** The interactive CLI accepts it: the status line shows `effort: low`. Decision 7's `--effort` condition holds.
      - **Fallback.** None.
    - **2. `--disallowedTools` holds, with a caveat.** Session B used the same argv (`--name orch-probe-B`, first prompt `Reply with just the word READY.`). It asked three times.
      - **Write.** The model replied `DENIED: No Write tool available…`, and `probe.txt` was not created.
        - First, though, it tried the **`Artifact`** tool, Claude Code's built-in page publisher, with `probe.txt`. That tool was available in the interactive session and ran with no permission prompt. It failed only on input validation: `Error: an Artifact's page must be .html …`.
        - No `PreToolUse` hook fired for that call.
      - **`ls`.** The model replied `DENIED: No Bash tool available in current session`.
      - **Sub-agent.** The model replied `DENIED: No Agent tool available…`. It saw only `TaskCreate`, `TaskGet`, `TaskList`, `TaskStop` and `TaskUpdate`, which track tasks.
      - **Prompts and hooks.** No permission prompt appeared for any of the three, and no `PermissionRequest` hook fired.
      - **Result.** `Edit`, `Write`, `NotebookEdit`, `Bash` and `Agent` are removed from the model's tool list, so decision 7's fallback (`--permission-mode plan`) does **not** trigger.
      - **Two facts contradict decision 7's rationale.** Its rationale says "everything not allowed and not disallowed keeps Claude Code's default permission behaviour, which asks the user".
        - **(a) Auto mode.** Every interactive session in this account started with `⏵⏵ auto mode on (shift+tab to cycle)` in its status line. The mode does not come from `~/.claude/settings.json`, which has no `permissions.defaultMode`; it is the CLI and account default. In auto mode, a tool that is neither allowed nor disallowed is decided by the classifier, and the user is not asked.
        - **(b) More write-capable tools.** The interactive tool set has more tools that write or act outside the checkout than the five disallowed. Examples are `Artifact` and, as the headless init in check 10 lists, `CronCreate`, `RemoteTrigger`, `PushNotification`, `SendMessage`, `Workflow` and `WebFetch`.
        - **Needs a controller ruling.** One option is an explicit `--permission-mode default`. Another is a longer `--disallowedTools`.
    - **3. MCP tool timeout: default ≥ 120 s, so `MCP_TOOL_TIMEOUT` is not needed.**
      - **Source.** In the 2.1.280 bundle, the per-call timeout is `server.timeout (≥1000) ?? env.MCP_TOOL_TIMEOUT ?? 1e8` ms: the default is 100,000,000 ms, about 27.8 h.
      - **Command.** `claude -p 'Call the tool mcp__anthrex__slow_sleep once with seconds=N …' --output-format stream-json --verbose --setting-sources user --strict-mcp-config --mcp-config '<slow stub>' --allowedTools mcp__anthrex__slow_sleep --model haiku --max-turns 3`, with `MCP_TOOL_TIMEOUT` unset.
      - **Observed.**
        - N=70: `tool_result at 75.1s: slept 70.0 seconds`, result `success`.
        - N=130: `tool_result at 137.4s: slept 130.0 seconds`, result `success`.
      - **Result.** The default is above 120 s, so decision 10's `MCP_TOOL_TIMEOUT=120000` condition is false and the variable is not set. Setting it would do no harm.
    - **4. Codex flags: partly verified. Interactive Codex was blocked by expired login.**
      - **Help.** `codex --help` lists `-c, --config`, `-m, --model`, `-s, --sandbox <SANDBOX_MODE>`, `-a, --ask-for-approval` and the `resume` subcommand. `codex resume --help` also lists `-s`, `-a` and `-m`.
      - **Order.** `codex -s read-only -a on-request -m gpt-5.6-luna resume --help` exits 0. `codex -s bogus -a on-request mcp list` fails with `invalid value 'bogus' for '--sandbox <SANDBOX_MODE>'`, so root `-s` and `-a` are parsed and validated ahead of a subcommand.
      - **Keys.** `codex -s read-only -a on-request -c mcp_servers.anthrex.command="python3" -c mcp_servers.anthrex.args=[…] -c mcp_servers.anthrex.tool_timeout_sec=120 -c mcp_servers.anthrex.default_tools_approval_mode="auto" mcp list --json` accepts the keys: it prints `"tool_timeout_sec": 120.0` for `anthrex`. `approve` is accepted too. A wrong value fails with `unknown variant 'bogusmode', expected one of 'auto', 'prompt', 'writes', 'approve'`.
        - Decision 8 says `"auto"`, while the shipped headless `codex_args` uses `"approve"`. Both are valid. What `auto` does in an interactive `-a on-request` session, auto-approve or prompt, could **not** be observed: see below.
      - **Interactive session.** Decision 8's argv, without M3's hook-trust bypass:
        ```
        codex -C repoD -c notify=[…] -c tui.* … -c projects."repoD".trust_level="trusted" -c mcp_servers.anthrex.… -c developer_instructions="…" -c model_reasoning_effort="low" -s read-only -a on-request -m gpt-5.6-luna -- '<prompt>'
        ```
        The TUI rendered its header, then exited with:
        ```
        Error: account/read failed during TUI bootstrap: account/read failed: workspace routing discovery unauthorized (401)
        ```
        Codex's own log (`~/.codex/logs_2.sqlite`, read only) gives the cause: `Failed to refresh token: Your access token could not be refreshed because your refresh token was revoked. Please log out and sign in again.` `codex login status` still prints `Logged in using ChatGPT`.
      - **Not verified.** `-s`/`-a` ahead of `resume <id>` at run time, the MCP call from a read-only interactive session, and whether its socket connect succeeds. The user must sign in to Codex again.
      - **Supplementary.** `codex sandbox -c 'sandbox_mode="read-only"' -- python3 sockprobe.py <sock>` ran the probe under Codex's own read-only Seatbelt policy.
        - It gives `connect FAILED: PermissionError [Errno 1] Operation not permitted`.
        - The same probe outside the sandbox gives `connect OK`.
        - So **if** Codex ran MCP servers inside its sandbox, the anthrex MCP socket would be refused under `read-only`. Codex is expected to spawn MCP servers outside the sandbox, but this was not observed.
      - **Fallback.** Decision 8's fallback (moving `-s`/`-a` after `resume <id>`) is not indicated: the parser takes them ahead of the subcommand. It is not proven at run time.
    - **5. `StopFailure` and `/compact`: both hold, and no fallback triggers.**
      - **`StopFailure`.** Session C used decision 7's argv with `--model claude-nonexistent-model-m91 -- 'Reply with just the word READY.'`. The hooks were `SessionStart(startup), UserPromptSubmit, StopFailure`, and there was **no `Stop`**. The payload was `{"hook_event_name":"StopFailure","error":"model_not_found","last_assistant_message":"There's an issue with the selected model (claude-nonexistent-model-m91)…"}`, with keys `cwd, effort, error, hook_event_name, last_assistant_message, prompt_id, scratchpad_dir, session_id, transcript_path`. That confirms decision 12: without `StopFailure` the window stays `Working`.
      - **Manual `/compact`.** Session B ran it from idle. The hooks were `PreCompact(trigger=manual), SubagentStop, SessionStart(source=compact), PostCompact(trigger=manual)`. There was no `UserPromptSubmit`, no `PreToolUse` and no `Stop`.
        - In anthrex's subscribed set, only `SubagentStop` (no status event) and `SessionStart` arrive. `SessionStart` does not change the status once signals have been seen (`status.rs`).
        - So `/compact` leaves no turn open and never sets `Working`. Decision 12's fallback (adding `PreCompact`/`PostCompact`) does not trigger.
    - **6. Bracketed paste: Claude holds. Codex was blocked by the login (check 4).**
      - **Claude, with the delay.** `ESC[200~Reply with just the word PASTEONE.\nThis is line two of the paste.ESC[201~`, then 200 ms, then `\r`, was submitted as one message: `UserPromptSubmit.prompt` held both lines joined by `\n`.
      - **Claude, without the delay.** Paste plus `\r` in a single write was also submitted as one message, both lines intact. The `\r` did **not** land inside the paste.
      - **Codex.** Not run. It needs a working login.
    - **7. OTLP from an interactive session: holds.**
      - **Command.** Session A's argv, with `metering::orchestrator_env`'s variables and decision 14a's header:
        - `CLAUDE_CODE_ENABLE_TELEMETRY=1`
        - `OTEL_METRICS_EXPORTER=otlp`
        - `OTEL_EXPORTER_OTLP_PROTOCOL=http/json`
        - `OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:<port>`
        - `OTEL_METRIC_EXPORT_INTERVAL=1000`
        - `OTEL_RESOURCE_ATTRIBUTES=anthrex.run=r-probe,anthrex.role=orchestrator`
        - `OTEL_EXPORTER_OTLP_HEADERS=authorization=Bearer <32 hex>`
      - **Listener.** A Python `http.server` on 127.0.0.1.
      - **Timing.** 8 `POST /v1/metrics`. The first came 0.6 s after `SessionStart`, well inside the 1 s interval.
      - **Headers.** `authorization: Bearer <token>` (exact match), `Content-Type: application/json`, `User-Agent: OTel-OTLP-Exporter-JavaScript/0.208.0` and `Connection: keep-alive`.
      - **`Expect: 100-continue`.** Never sent, and the listener's `handle_expect_100` was never called. Decision 14 needs no code.
      - **Resource.** `anthrex.run=r-probe`, `anthrex.role=orchestrator` and `service.name=claude-code`.
      - **Metrics.** `claude_code.session.count`, `claude_code.active_time.total`, `claude_code.token.usage` (`type=input|output|cacheRead…`, `model=claude-haiku-4-5-20251001`) and `claude_code.cost.usage`.
      - **Deltas.** Every sum has `aggregationTemporality: 1`, which is **DELTA**. The exporter sends deltas, so the global series cap stays a follow-up, as the brief says.
    - **10. MCP tool visibility against the real CLI: holds.**
      - **Command.** One headless worker session with `run/role_launch.rs::worker_spec`'s argv shape, `ENABLE_TOOL_SEARCH=false` and `TMPDIR` set to a scratch dir:
        ```
        claude -p --input-format stream-json --output-format stream-json --verbose --permission-prompts none --session-id <uuid> --setting-sources user --strict-mcp-config --settings '<hooks + sandbox block with CLAUDE_SANDBOX_PINS>' --mcp-config '<worker stub>' --allowedTools mcp__anthrex__task_done,mcp__anthrex__task_blocked,mcp__anthrex__task_note,Bash,Edit,Write,Read,Glob,Grep,Agent,TodoWrite --append-system-prompt '<WORKER_CONTRACT, verbatim>' --permission-mode acceptEdits --model haiku --effort low
        ```
        The first turn was a stream-json user message on stdin.
      - **`system/init`.** `claude_code_version: 2.1.280`; `mcp_servers: [{"name":"anthrex","status":"connected"}]`. It lists the anthrex tools `mcp__anthrex__task_blocked`, `mcp__anthrex__task_done` and `mcp__anthrex__task_note`. `ToolSearch` is absent.
      - **Full tool list.**
        ```
        Task, Bash, CronCreate, CronDelete, CronList, DesignSync, Edit, EnterWorktree, ExitWorktree, Glob, Grep, ListAgents, Monitor, NotebookEdit, PushNotification, Read, RemoteTrigger, ReportFindings, ScheduleWakeup, SendMessage, Skill, TaskCreate, TaskGet, TaskList, TaskStop, TaskUpdate, WebFetch, WebSearch, Workflow, Write, mcp__anthrex__task_blocked, mcp__anthrex__task_done, mcp__anthrex__task_note
        ```
        The sub-agent tool is named `Task` in the init list, and `--allowedTools`/`--disallowedTools` say `Agent`. Check 2 shows that disallowing `Agent` removes it.
      - **Tool calls.** The session made exactly one tool call, `mcp__anthrex__task_done` with `{"summary":"probe: nothing to do"}`, and no `ToolSearch` call. It ended with `result: success`. The hooks were `SessionStart, UserPromptSubmit, PreToolUse/task_done, PostToolUse/task_done, Stop, SessionEnd`.
      - **Result.** Decision 42f's premise holds.
    - **Fallbacks.** No decision's fallback triggered:
      - decision 7's `--permission-mode plan`: check 2 held;
      - decision 8's `-s`/`-a` after `resume`: the parser accepts them ahead of a subcommand, but this is not proven at run time;
      - decision 10's `MCP_TOOL_TIMEOUT`: the default is about 27.8 h;
      - decision 12's `PreCompact`/`PostCompact`: `/compact` leaves no turn open;
      - decisions 14 and 14a: no `Expect: 100-continue`, and the token header arrives;
      - decision 42f: holds.
    - **Outstanding.**
      - The Codex interactive part of checks 4 and 6 is blocked because the Codex ChatGPT refresh token was revoked. The user must sign in to Codex again.
      - Decision 7's premise that other tools "ask the user" is false under this account's default auto mode. That needs a controller ruling.
    - **Side effects.** The CLIs wrote their own normal state:
      - Claude session transcripts;
      - `~/.claude.json` trust entries for `repoA`, `repoB` and `repoC`, from accepting the trust dialog;
      - Codex's log database.
      
      No settings file was edited. No bypass flag was used.

- **Controller rulings on the real-CLI findings.** These bind M9.10, which adds the tests.
  1. **The orchestrator's permission mode.** On this account, an interactive `claude` starts in "auto mode", so a tool that is neither allowed nor disallowed goes to a classifier, not to the user. Decision 7 assumed the user is asked.
     - The orchestrator's argv adds an explicit `--permission-mode default` after `--disallowedTools`, so any unlisted tool prompts the user at the orchestrator's own window.
     - `--disallowedTools` also gains the outward-acting tools that check 2 and check 10 observed: `Artifact`, `CronCreate`, `CronDelete`, `RemoteTrigger`, `PushNotification`, `SendMessage`, `Workflow`, `WebFetch` and `WebSearch`. The orchestrator plans. It never publishes, schedules or messages, and research is the scouts' job.
     - The explicit mode is the guard. The list is best effort, since a CLI can add tools.
     - Headless roles are unaffected: every one already passes an explicit `--permission-mode` (`role_launch.rs:212`, `:276`; `decider/argv.rs:106`; `scout/spec.rs:114`).
  2. **`Task` and `Agent`.** `system/init` names the sub-agent tool `Task`, but disallowing `Agent` removed it (check 2). The argv keeps `Agent` and adds `Task` too, which costs nothing.
  3. **Codex tool approval (decision 8).** The orchestrator uses `default_tools_approval_mode="approve"`, the value the shipped headless `codex_args` already uses and that works in practice, not the brief's unobserved `"auto"`.
  4. **`MCP_TOOL_TIMEOUT` (decision 10) is dropped.** The CLI's default is about 27.8 h, and 70 s and 130 s tool calls completed without it (check 3). The launch environment does not set it.
  5. **The scrub gains the `MCP_` prefix** (`config::reserved_env::SCRUBBED_PREFIXES`). Every agent session the daemon launches drops inherited `MCP_*` variables, for example `MCP_CONNECTION_NONBLOCKING` from a daemon started inside Claude Code, and a profile may not set them. M9.10 owns this, with a reserved-env test.
  6. **Check 4 and the Codex half of check 6 are outstanding.** The Codex login on this machine was revoked (401 on workspace routing), and the user must sign in again. Until then decision 8 is built as written, amended by ruling 3. Observed for the sandbox: a Unix-socket connect from inside `codex sandbox` read-only fails with EPERM. So if Codex ran its MCP servers inside the sandbox, the orchestrator's anthrex tools would fail, but it is expected to run them outside. M9.10's manual check re-runs this after the login.

### M9.1 Codex re-run (2026-09-28, after the user signed in again)


- **Version.** `codex --version` gives `codex-cli 0.156.1`. `codex login status`: `Logged in using ChatGPT`. No 401 this time.
- **Where.** Scratch repository `…/scratchpad/m9.1/repoE` (`git init`). Driver `sessE.py` (PTY 180×50, hard limit 235 s, kills its own child pid only). Stub `stub_mcp.py --role codex` connects to `sock_listener.py` on `…/m9.1/d.sock`; for this run the stub also logs its `ppid` and any `*SANDBOX*` env var, and its `run_status` also tries to write `…/m9.1/stub-write-test.txt`. Logs: `…/m9.1/logs/{E1,E2,E3,R1,R2}-*`.
- **Command (E2, trimmed).**
  ```
  codex -C repoE -c notify=[python3,codex_notify.py,…] -c tui.terminal_title=["status"] -c tui.notifications=["approval-requested"] -c tui.notification_method="bel" -c tui.notification_condition="always" -c 'projects={"<repoE>"={trust_level="trusted"}}' -c mcp_servers.anthrex.command="python3" -c mcp_servers.anthrex.args=[stub_mcp.py,--role,codex,--log,…,--sock,d.sock] -c mcp_servers.anthrex.tool_timeout_sec=120 -c mcp_servers.anthrex.default_tools_approval_mode="approve" -c developer_instructions="…" -c model_reasoning_effort="low" -s read-only -a on-request -m gpt-5.6-luna -- 'Call the anthrex MCP tool run_status exactly once and reply with exactly the text it returned.'
  ```
- **Probe-harness finding (E1, not a decision-8 issue).** The earlier dotted form `-c projects."<path>".trust_level="trusted"` did **not** take effect: Codex showed its "Folder access / Trust this folder?" dialog. The scratch path contains a dot (`m9.1`), and Codex splits `-c` keys on every `.`, quotes notwithstanding. The inline-table form `-c 'projects={"<path>"={trust_level="trusted"}}'` works. The dialog was never answered (the session was killed), so nothing was saved to `~/.codex/config.toml` (mtime still Sep 27, no `repoE` entry). Anthrex does not pass a project-trust flag for interactive Codex, so this matters only if it ever does: any repository path with a `.` in it needs the inline-table form.

1. **Decision 8's flags are accepted at run time: holds.**
   - Fresh start with `-s read-only -a on-request -m … -- <prompt>` (E2) and resume with `-s read-only -a on-request -m … resume <id>` (R1, R2) both started. `/status` in the resumed session shows `Permissions: Read Only (Ask for approval)`, `Model: GPT-5.6-Luna (reasoning low …)`, `Session: 01a0e80c-0ce4-7440-9864-007591da1950`.
   - `tool_timeout_sec=120` and both approval modes were accepted with no warning. The only warnings (`f2`) were for the user's own configured MCP servers: `MCP client for 'pycharm' timed out after 30 seconds…` and `MCP startup incomplete (failed: pycharm, vercel)`.
   - **`"approve"` vs `"auto"`.**
     - `"approve"` (E2): the tool ran with **no prompt**. Screen: `• Called anthrex.run_status({})`, first turn complete 16.2 s after start.
     - `"auto"` (E3): Codex **prompted**, 15 s after start: `Allow the anthrex MCP server to run tool "run_status"?` with `1. Allow / 2. Allow for this session / 3. Always allow / 4. Cancel`. The turn waited until Enter (Allow) was sent; then the call ran and succeeded. So in an interactive `-a on-request` session `"auto"` asks the user for a tool without annotations; `"approve"` does not.
2. **A read-only interactive Codex calls an MCP tool: holds (under `"approve"`, no prompt).** Stub log: `tools/call run_status` → `call_done … "digest: all tasks idle. socket connect OK, reply=pong from listener; write OK"`. Notify `agent-turn-complete` carried the same text as `last-assistant-message`.
3. **Socket connect from the MCP server under `-s read-only`: SUCCEEDS.** Codex runs MCP servers **outside** its sandbox.
   - Stub `run_status`: `socket connect OK, reply=pong from listener`; listener log `{"got": "hello from stub\n"}` (E2 and E3).
   - The same stub also **wrote a file** in the scratch dir (`write OK`; `stub-write-test.txt` existed afterwards), which the read-only Seatbelt profile forbids (compare the earlier `codex sandbox` probe: `connect FAILED: PermissionError [Errno 1] Operation not permitted`).
   - The stub's start log shows `ppid` = the `codex` pid itself (19125 in E2, 19553 in E3) and no `*SANDBOX*` variable in its environment (`sandbox_env: {}`), so it was spawned directly by Codex, not through `sandbox-exec`.
4. **`resume <id>` with the flags ahead of it: holds.**
   - Id from E2's notify payload `thread-id` = `01a0e80c-0ce4-7440-9864-007591da1950`.
   - R1 (`… -s read-only -a on-request -m gpt-5.6-luna resume 01a0e80c-…`): the TUI restored the whole transcript (the MCP call and both pastes) and `/status` showed the same session id and Read Only permissions. The probe prompt typed ~4 s after start was lost (typed before the TUI accepted input) — a driver timing issue.
   - R2 (same argv, prompt typed after startup settled): the turn completed in 9.4 s; notify `input-messages` held all four user messages of the thread and the reply was `PASTETWO`, i.e. the resumed model had the earlier context. Same `thread-id`.
   - Decision 8's fallback (moving `-s`/`-a` after `resume <id>`) is not needed.
5. **Check 6, Codex half (bracketed paste): holds.**
   - With the 200 ms delay: `ESC[200~Reply with just the word PASTEONE.\nThis is line two of the paste.ESC[201~`, 200 ms, `\r` → one turn; notify `input-messages` entry `"Reply with just the word PASTEONE.\nThis is line two of the paste."`, reply `PASTEONE`.
   - Without the delay (paste and `\r` in one write): one turn, `"Reply with just the word PASTETWO.\nThis is line two of the second paste."`, reply `PASTETWO`. The `\r` did **not** land inside the paste; no extra Enter was needed.
- **Does decision 8 need changing?**
  - **Yes, one value:** `default_tools_approval_mode="auto"` must be `"approve"` (as controller ruling 3 already says, and as the headless `codex_args` ships). With `"auto"`, every orchestrator MCP call in the interactive session stops at an Allow prompt.
  - Everything else holds as written: `-s read-only -a on-request` ahead of `resume <id>` and ahead of `-- <prompt>`, `tool_timeout_sec=120`, and the anthrex MCP server's socket connect from a read-only session (Codex spawns MCP servers unsandboxed).
  - Side observation for decision 9 (not a decision-8 change): the user's own `~/.codex` MCP servers (here `pycharm`, `vercel`) also start inside the orchestrator session; a failing one adds a startup warning and up to its 30 s startup timeout, but did not delay the first turn here.
- **Pids.** Recorded in `…/m9.1/pids.txt`: codex 18827, 19125, 19553, 20436, 20958; listeners 18825, 19124, 19552, 20435, 20957. Stub MCP servers (from stub logs): 19156, 19652, 20464, 20983. `ps -p` on all of them, and on every earlier pid in `pids.txt`, prints only the header (exit 1).

- **Controller rulings on the Codex re-run** (bind M9.10):
  1. Check 4 holds:
     - `-s read-only -a on-request` is accepted ahead of `resume <id>`, and resume keeps the thread;
     - Codex runs MCP servers **outside** its sandbox, so the anthrex socket connect works from a read-only session;
     - the Codex half of check 6 holds.
     
     Decision 8 stands, amended only by ruling 3 (`default_tools_approval_mode="approve"`; `"auto"` prompts per call).
  2. **Trust.** anthrex passes no Codex trust flag today (`headless/argv.rs`). If M9.10 ever passes one, it uses the inline-table form `-c 'projects={"<path>"={trust_level="trusted"}}'`, because a dotted `-c` key splits on every `.` in the path. Otherwise the orchestrator's first start may show Codex's trust dialog in its own window, which the user answers. M9.10's manual check records which happens.
  3. **User MCP servers.** A Codex orchestrator also starts the user's own `~/.codex` MCP servers, a Claude one does not, since `--strict-mcp-config` applies there. That is the user's own configuration, which the design rules allow. They keep their own approval mode, and only `anthrex` is set to `approve`. Recorded as a difference between runtimes, not changed.

### Task M9.7 (Engine: planning, submit, holds and promotion)

- **Preparatory commit** (no behaviour change): `engine/mod.rs` (590 lines) lost its op-result routing, `op_done`, to a new `engine/results.rs`. `driver.rs` (597) is not touched by M9.7.
- **Files beyond the task's list**, each to keep a file under 600 lines or to give a later task its seam:
  - `engine/batch.rs`: M8a's `run edit` batch (`apply_batch`, with `Refused` and `Applied`), moved out of `engine/requests.rs` so the orchestrator's `edit_plan` goes through the same path with its own `EditSource`. With the promote body gone too, `requests.rs` is 377 lines (537 before).
  - `engine/orch_window.rs`: the orchestrator window's lifecycle (launch, its result, restart on resume, dormant on restore, a lost launch). `engine/orch.rs` is 388 lines, which leaves room for M9.8 and M9.9.
  - `launch/role.rs` is created now with only `RoleLaunch` and the two Claude tool lists, because `OpKind::CreateOrchestrator` carries a `RoleLaunch`. M9.10 adds the argv builders. The disallowed list already has the M9.1 real-CLI rulings 1 and 2 (`Task`, `Artifact`, `CronCreate`, `CronDelete`, `RemoteTrigger`, `PushNotification`, `SendMessage`, `Workflow`, `WebFetch`, `WebSearch`); `--permission-mode default` is M9.10's argv.
  - `run/driver/orch.rs` gains `hold_verdict` and `promote` (the driver's request arms, which kept `driver/requests.rs` at 580); `driver/guard.rs` gains the `Orch` event's reply; `driver/ops.rs` answers the four new ops `Failed { "<Op> is not available yet" }` until M9.13 executes them.
  - `scout/spec.rs`: `ScoutSpec` derives `Serialize`/`Deserialize`, because `OpKind::StartScout` is journaled.
  - `config`: `AgentConfig` is re-exported (`orchestrator.rs` and the existing `pub use orchestrator::{…}` line of `lib.rs`), as the M9.3 notes foresaw for `resolve_orchestrator`'s signature.
  - `run/reconcile/sessions.rs` (`restored_orchestrator`) and a test file `run/reconcile/orch_tests.rs`; test files `engine/tests/{orch_edit.rs, orch_restore.rs}` beside the listed `orch.rs`, which would have been 675 lines.
- **Deviation: `RunLimits.orch.agent`.** `OrchLimits` gains `agent: AgentLimits { runtime, model, effort }`, frozen from `[orchestrator.agent]` at run start (`#[serde(default)]`: a pre-M9 run gets the config defaults). M9.3's rule is that later tasks read `run.limits.orch`, never the live config, and the engine must resolve a promoted run's route from the run alone on resume and on the tick. `limits_are_frozen_at_run_start` checks the new field.
- **Deviation: `EventKind::Promote { reply, run_id, orchestrator: Option<OrchestratorChoice> }`**, not Interfaces' `route: Option<Route>, installed`. The engine resolves the route with `resolve_orchestrator` from the run's frozen agent limits, `default_runtime` and roster, the same way for the request, a resume and a tick. `Run.orch.installed` is not recomputed on promote: the binaries reach `RunContext` in M9.13. **Obligation for M9.13:** decision 9's project-settings check for `run promote` (against `base_sha`, honouring the run's `trust_project`) is driver work before the event and is not done here.
- **Ops.** Added `OpKind::{CreateOrchestrator { spec: Box<WindowSpec>, role: Box<RoleLaunch>, project }, RestartOrchestrator { window_id }, StartScout { spec: Box<ScoutSpec> }, ResolveTarget { root, target, base_branch }}` (boxed as `CreateWindow`'s spec is) and `OpResult::{Restarted, ScoutStarted { window_id }, Target { base, head }}`, with the reconcile rows of Interfaces. **`StartPlanner` and `PlannerStarted` are left to M9.8**, whose `scout/planner.rs` defines `PlannerSpec`.
- **`OrchEvent`** has `Tool`, `ApproveHold` and `RejectHold` only; M9.8, M9.9, M9.13 and M9.13b add their variants. A `Tool` from any role but the orchestrator, and the orchestrator's `spawn_scout` and read tools, answer `{"error": "tool <t> is not available yet"}` until their tasks.
- **The tool gate** (decision 15) runs in this order: the run, the role, the run state (writes in `planning`, `awaiting_approval`, `running` and, for `edit_plan` only, `complete`), the caller (`this window is not the orchestrator of run <id>`, also before the window exists), then `parse_call`. So a `complete` run answers `spawn_subplanner` with `run <id> is complete`.
- **`edit_plan`'s readings.**
  - The batch, `submit` and `summary` are one unit: the batch is applied to a copy, the submit checked on it, and nothing is kept if either refuses.
  - A rejected batch's `errors[].message` is `PlanError.message` without the `task <id>: <field>: ` prefix, since `task` and `field` are their own keys. A refusal that is not a `PlanError` (a `pause`/`resume` that does not fit, a runtime refusal, a submit refusal) is `{"error": "<text>"}`.
  - `notes` are `<task>: <note>`, every validation note the batch added, M8a's included.
  - `revision` is the digest revision after the scheduler's pass: the handler runs `dispatch::schedule` once before it answers, as a tick at the same time would, so the revision is the one `run_status` then reports (without it, adding a task reported 2 while the run was at 3).
  - `submit` on a promoted running run sets `plan_submitted` even when the orchestrator added nothing, so decision 38's `plan_submitted` condition (M9.9) cannot keep a promoted run from completing.
- **`spawn_subplanner`: the engine's state half.** A new epic is recorded with its planner `Queued` and the route of `run::orch::launch::planner_route` (decision 31's, through M8b's `scout::spec::route`); on a running run with a submitted plan it gets hold `epic:<e>`; an ended epic is queued again with an entry in `replans`; a live one is refused with decision 21's text; `awaiting_approval` returns to `planning`. The epic and area validation, `path = Large`, the reader slot and `StartPlanner` are M9.8's.
- **Holds' readings.** Only the orchestrator's and sub-planners' tasks are held; a user's own `run edit` addition never is. "After the gate" is `run.approved_at` set. Before a promoted run's orchestrator submits, every task it adds waits under `promotion`; afterwards a task of an epic whose hold is `drafting` or `awaiting` carries that hold. `gate_holds::submitted` is the call M9.8 makes when a `submit_epic` is accepted (approved at once with `--yes`); `with_yes_an_epic_hold_is_approved_on_submit` calls it directly. Reply texts, the implementer's: `hold <h> of run <id> approved: <n> task(s) may start` and `hold <h> of run <id> rejected: <n> task(s) cancelled`; a verdict on a terminal or complete run answers `run <id> is <state>`, and on a drafting hold `hold <h> is drafting`. The approval hold is one condition in `dispatch_writers` and one in `prewarm` (`gate_holds::released`); M8a ruling N5's `awaiting_deps` is untouched.
- **Promotion's readings.** Promotion sets `path = plan`, which lifts M8b's fast-path refusal of additions. A pre-M9 wish is also performed by a new `run promote` request, and keeps its recorded time. On the tick it is performed for a `running` run only; a restored run is paused, so it is performed by `run resume`.
- **Restore and resume.** A `planning` run is paused like a `running` one. The orchestrator is marked dormant after the replay, so a replayed `CreateOrchestrator` keeps its window. A lost `CreateOrchestrator` is launched again by `run resume`. `unpause` (so both `run resume` and the `resume` plan edit) restarts a dormant orchestrator with the next session and performs a pre-M9 promotion. `run resume` of a run in `awaiting_approval` or `planning` that is not paused restarts a dormant orchestrator and answers `run <id>: its orchestrator restarts`, else M8a's refusal.
- **A failed `CreateOrchestrator`** is logged (`the orchestrator could not start: <message>`); the attention line of decision 13 is M9.13's.
- **Report.** `## Summary from the orchestrator` follows the title, rendered with `plain_text_line`, so a summary line cannot open a heading. **Left open:** decision 6's report line `orchestrator below the frontier tier: …` is not written (no task names it; M9.13 or M9.14 may). *Resolved by the M9.7 review fixes, ruling 3.*
- **Not rewritten here: `driver/adapt_goal_tests.rs:125`.** The driver's planned-run build that replaces `planned()` is M9.13's (its file list and `start_goal_on_the_plan_path_builds_a_planned_run`), so that test and M8b's goal-refusal end-to-end tests stay as they are until then.
- **Tests beyond the list:** `a_lost_orchestrator_launch_is_made_again_on_resume`, `orchestrator_tools_cannot_approve` (the design rule: no `edit_plan` op or tool name approves the plan, a task or a hold, and a resubmit at the gate changes nothing). `planned_message_is_exact` already exists (M9.5) and is not duplicated. M9.2's pinning test became `approval_holds_are_answered_by_the_engine` (`daemon/tests/server_runs.rs`: the run loop is now started and the answer is `unknown run r1`); M8b's `e2e_promote_records_intent_and_the_task_continues` became `e2e_promote_performs_and_the_task_continues` (the orchestrator's op fails in the driver's placeholder until M9.13, and the fast-path task still merges).
- **Red before green.** The tests were written after the code in this task, so they were run red two ways: against the pre-change code none compiles (no `OrchEvent`, `make_planned`, `gate_holds`, `Promote.orchestrator` or `OpKind::CreateOrchestrator`); and with the change in place, one combined mutation (the hold condition removed from `dispatch.rs`, the `planning` restore reverted, the caller check disabled, the tick's promotion disabled, the planned start's launch removed) turned 29 of the 33 new engine tests red; the four others (`resolve_orchestrator_order`, `…_keeps_the_candidate_snapshot`, `promote_creates_…`, `promote_repeat_reply_has_no_time`) are the compile-only red. The sources were restored from copies.

### M9.7 review fixes

One commit, `fix(daemon): hold every task added after promotion or split from a held task`. Tests were written first and each was seen red for the reason given.

1. **The promotion hold keys on the promotion, not on `approved_at`** (decision 29). `gate_holds::promoted(run)` is `promote_requested_at` set and the orchestrator record present (a pre-M9 wish not yet performed has no orchestrator, so nothing is held for it). While `plan_submitted` is false, every task the orchestrator adds to a promoted run waits under `promotion`, whatever the gate state. `past_gate`, which still decides whether a new epic of a running run gets `epic:<e>` (`create_epic_hold`), is now `approved_at` set **or** promoted, so a promoted M8b run without `approved_at` gets epic holds after its submit too; this extension is the implementer's reading of "whatever the gate state". This supersedes the M9.7 note "After the gate is `run.approved_at` set". Test `a_promoted_run_with_no_approval_time_holds_its_additions` (red: `held` was `null`, and `t2` dispatched).
2. **Split children inherit their parent's hold.** `gate_holds::assign(run, edits, added, now)`: a task this batch split from a parent whose hold is not `Approved` gets that hold and joins its `tasks`, whether or not it names an epic; other additions follow the promotion and epic rules. The split parent, which the split cancels, leaves its hold: its `gate_hold` is cleared and it is removed from the hold's `tasks`, so a verdict counts only live work (the approve reply's count is `tasks.len()`). The dead `source: &EditSource` parameter (only the orchestrator's `edit_plan` calls `assign`; the equivalent mutant the reviewer found) is gone, and the doc comment says a user's `run edit` never comes here. `amend_task` keeps the `Task` value in place (`edits.rs::amend_task`, `reresolve`), so a held task keeps its hold. Tests `split_children_of_a_promotion_held_task_stay_held` and `split_children_of_an_epic_held_task_stay_held` (red: `held` was `null`, and the children were unheld), and **pinning** `amending_a_held_task_keeps_its_hold` (a brief amend and a re-resolving size amend; green at once).
3. **Decision 6's report line.** `report.rs::below_frontier` writes `orchestrator below the frontier tier: <runtime> (<model, or default when empty>) <strength>` after `Approved by:` when the orchestrator's route is not `frontier`. The model goes through `plain_text_line`. `report_task::strength_label` became `pub(super)` for it. Test `an_orchestrator_below_the_frontier_tier_is_reported` (the built-in roster's Codex route, `codex (default) standard`; none for the default Claude route; red: no such line).
4. **`run reject` of a restored planning run.** `requests::reject` accepts `paused` with `paused_from = planning` and discards it (decision 26). Test `reject_of_a_restored_planning_run_discards_it` (red: `run <id> is paused; reject applies only while its plan awaits approval`).
5. **The user's submit (decision 13).** `RunRequest::Edit` gains `#[serde(default)] submit: bool` (inside PROTO 10; test `edit_submit_round_trips_and_defaults_to_false`, which also decodes an M8c-shaped `Edit` without the field; its red was the compile failure, since the field was added with the plumbing before the test ran). `EventKind::Edit` carries it; the driver passes it through; the TUI's two `Edit` requests and the CLI's `run edit` send `false` until M9.15 and M9.14 add their surfaces (their task texts now name them). In the engine, `requests::submit_edit` admits `planning` only (`run <id> is <state>; only a run being planned can be submitted` otherwise), applies the batch and the submit to a copy as `edit_plan` does, and submits through `orch::submit_plan` (now `pub(super)`, with a `who` for its log line: `the user submitted the plan; awaiting approval`). The reply is `the plan of run <id> was submitted: it awaits approval` (or `approved by --yes, it runs`), after `applied <n> edit(s); ` when the batch had edits. An empty plan is refused with the orchestrator's text. Tests `a_user_submit_while_planning_opens_the_gate` (then `run approve` and the task dispatches) and `a_user_submit_is_refused_in_any_other_state` (`awaiting_approval`, `running`; the run unchanged) (red: the reply was `applied 1 edit` and the batch was applied).
6. **Interim edit-log state, until M9.9 (decision 40, item 3).** The orchestrator's `edit_plan` batches go through `engine/batch.rs::apply_batch`, which calls M8c's `edit_log::record(run, edits, now)`: an accepted orchestrator batch is logged with source `user` (M8c's record has no source; decision 40's default is `user`), and a rejected orchestrator batch is not logged at all. M9.9's `edit_log_records_every_source_rejections_and_recipients` now names both as what it must turn red and then fix.

#### Second review

One commit, `fix(daemon): hold a promoted run's additions until the user approves`. Each test was written first and seen red for the reason given.

1. **The promotion window ends with the user's verdict, not the orchestrator's submit** (decision 29). `gate_holds::assign` holds every task the orchestrator adds to a promoted run under `promotion` while no `promotion` round is `Approved` (`promotion_open`); an addition made while it is `Awaiting` joins its `tasks`. After the user approves it, decision 29's "only new epics are held" applies. `submit_plan`'s `running` arm calls `gate_holds::submit_promotion`, which creates the hold when the orchestrator added nothing: an orchestrator `submit` on a promoted run with no held additions leaves `promotion` `Awaiting` with an empty task list (approved at once with `--yes`), so the user's approval is still the event that ends the promotion window. A resubmit while the hold awaits changes nothing. A rejected hold does not end the window (ruling 7 below). This supersedes the first review's ruling 1 wording "while `plan_submitted` is false" and the M9.7 note "Before a promoted run's orchestrator submits". The existing `promotion_hold_is_awaiting_after_submit` asserted the defect ("after the submit, the orchestrator's additions are no longer held"); its `t3` now joins the hold. Tests `an_empty_submit_after_promote_holds_later_additions_until_approval` (scenario A; red: no hold existed after the empty submit, so `t2` was unheld and `held` was `null`) and `additions_after_submit_join_the_awaiting_promotion_hold` (scenario B, then an addition after the approval that is not held and dispatches; red: `held` was `null` for `t3`). The second sets `max_writers = 4` so a task left queued is one that was held.
2. **`past_gate`'s promotion arm is tested.** `a_promoted_m8b_run_holds_a_new_epic_after_submit`: a fast-path run with `approved_at = None`, promoted, submitted empty, the promotion approved, then `spawn_subplanner` of `mail` replies `hold: "epic:mail"`. Red first because ruling 1 did not exist (the approval was refused: no `promotion` hold); after the fix, the mutation of `past_gate` back to `approved_at.is_some()` turns it red (`hold` `null`), and only it.
3. **Verdict counts.** The approve reply counts only the hold's unfinished tasks, as the reject reply already did. A user's own `run edit` split (or cancel) of a held task releases that work: the user's edit is theirs, so the split children carry no hold and dispatch without the verdict, and the cancelled parent stays in the hold's `tasks` but is no longer counted. Tests `a_users_split_of_a_held_task_releases_it_and_the_approval_counts_none` (red: `1 task may start`) and `verdicts_count_only_unfinished_tasks` (one of two held tasks cancelled by the user; approve and reject each count 1; red: `2 tasks may start`).
4. **No submit while the run is finishing.** `orch::submit_plan`, which both the orchestrator's `edit_plan` and the user's `run edit --submit` go through, refuses first with `run <id> is being discarded` (or `being accepted`) while `dispatch::finishing_as` names a `Discard` or `Accept` in flight; the batch is on a copy, so nothing is kept. Test `a_submit_while_the_run_is_being_discarded_is_refused` (a planning run with a task, rejected, then both submits; red: the orchestrator's submit was accepted).
5. **A finished run's orchestrator is not live.** `orch_window::ended`, run on every run after every event (beside the scheduler in `engine::step`), sets `OrchestratorRecord.live = false` when the run is terminal, so a `Restarted` or `Window` result arriving after the end cannot set it again. **Deviation from the ruling's list:** "terminal" is `RunState::is_terminal` (accepted, discarded, failed), which is exactly decision 11's list for clearing the run-live flag. `complete` is excluded, because decision 38 wakes the orchestrator on completion to write its summary and the tool gate still takes `edit_plan summary` there; there is no `cancelled` run state (`run cancel` leads to `complete`, then accept or discard). Decision 30's "the orchestrator window becomes a plain window" is decision 11's run-live flag, which the driver clears when it publishes a terminal run; M9.13's file list already had it and now says it follows this engine flag. Test `a_discarded_runs_orchestrator_is_not_live` (the reviewer's sequence: a planning run restored, resumed, rejected, then the `Restarted` and the `Discard` results; red: `live` was true).
6. **`below_frontier` with `Strength::Fast`.** Test `a_fast_orchestrator_is_reported_below_the_frontier` (the orchestrator's route set to `fast`; the line ends `fast`). It passed at once, as a test of existing behaviour; the mutation `if route.strength != Standard { return }` turns it red, and only it.
7. **A rejected promotion round keeps later additions held** (controller ruling in the same round; it replaces the implementer's first reading that a rejection ends the window, which let the orchestrator's later work bypass the user's "no"). The window closes only when a `promotion` hold is `Approved`. After a rejection, the orchestrator's next addition opens a new round with the same rules: `Drafting` until the orchestrator submits, `Awaiting` after, additions join it, and an empty submit creates it `Awaiting` with no task. Rounds are `HoldKind::Promotion` holds with unique ids, `promotion`, then `promotion-2`, `promotion-3`, … (`gate_holds::promotion_round`; the next free number after the count of promotion rounds), because holds are keyed by id in `run approve|reject --hold`, the digest, the snapshot and the history. The user approves a round with `anthrex run approve <run> --hold promotion-2`. Tests `after_a_rejected_promotion_additions_open_a_new_round` (add `t2`, submit, reject; `t3` is held under `promotion-2`, not dispatched after a tick, nor after the round's submit, and dispatched on its approval; red: `held` was `null`) and `after_a_rejected_promotion_an_empty_submit_opens_an_awaiting_round` (red: no second round existed).
8. **Third review: an epic's work is held through the promotion window and after a rejection.**
   1. *An epic spawned inside the promotion window.* `submit_plan`'s `running` arm on a promoted run now refuses while a sub-planner is queued or planning, with the `planning` arm's text (`sub-planner <e> is still planning; submit when every sub-planner has finished`; one helper, `planners_finished`). A task of an epic spawned while the window is open joins the open promotion round, as any addition does: `create_epic_hold` gives such an epic no `epic:<e>` hold (so no empty epic hold waits for a verdict), and `spawn_subplanner` replies `hold: null` for it. Test `epic_spawned_before_submit_stays_held` (the planner simulated by setting its phase to `Finished`, then the orchestrator adds `t7`; red: the empty submit was accepted while `mail`'s planner was queued).
   2. *A rejected epic, re-planned.* Epic holds take the promotion's round scheme. An epic is held while it has a round and none is approved (`epic_open`); its tasks join its open round, and when its last round was rejected, the next re-plan (`replan_epic_hold`, called from `spawn_subplanner`'s re-plan branch) or the next addition to it opens a new round, recorded as the epic's `gate_hold`. An epic with no round (planned before the gate, or inside the promotion window) or with an approved round is not held, as before. Round ids are `epic:<e>`, then `epic:<e>.2`, `epic:<e>.3`: **the separator is `.`, not `-`**, because an epic id matches `^[a-z0-9][a-z0-9-]{0,10}$` (`orch/tools.rs::id`), so `epic:mail-2` would be the first hold of an epic named `mail-2`; no epic id holds a `.`. Promotion rounds keep `promotion-<n>`, which cannot collide with an `epic:` id. `ensure` is gone: holds are created by `create` from a free id (`fresh`), so no rejected or decided hold's id is ever handed back to join (a `debug_assert` guards it). Test `rejected_epic_replanned_stays_held` (an approved promotion; `mail` spawned; `t5` added under `epic:mail`, submitted, rejected; `mail` spawned again, replying `epic:mail.2`; `t6` held under it and dispatched only on its approval; red: the re-plan replied `hold: null`). **Left open:** an epic round opened by the orchestrator's own addition, with no sub-planner running, stays `Drafting` until a `submit_epic` (M9.8) submits it, as `epic:<e>` holds did before; the orchestrator's `submit` submits only the promotion round.
   3. *`submit_promotion`'s early return is pinned.* **Pinning** test `a_resubmit_while_the_promotion_round_awaits_submits_nothing`: a resubmit while the round awaits adds no second `the orchestrator submitted its additions` log line. Removing the early return turns it red, and only it.
   4. *Accepted reading, no change:* the orchestrator may amend a queued task the user added, its brief included. Decision 19 allows `amend_task` of a brief at any time, and an amend is not an addition, so it is not held.
9. **Final review: every spawned epic has its own hold round** (one rule replacing item 8.1's special case, which let an epic spawned while `promotion` awaited the user reply `hold: null` and run its tasks unheld once the promotion was approved).
   1. *The rule.* `create_epic_hold` gives every new epic of a run that is promoted or past its gate (`past_gate`) its own `epic:<e>` round, the promotion window included; item 8.1's `|| promotion_open` exemption and the old `plan_submitted` condition are gone (a planned run is past its gate only after its submit). In `assign`, a task that names one of the run's epics follows the epic's rule only (`names_an_epic`, then `epic_hold`) and never joins the promotion round; tasks with no epic join the open promotion round as before. So approving `promotion` never releases an epic's work. The `epic:<e>.2` re-plan scheme (item 8.2) stays. Updated test `epic_spawned_before_submit_stays_held` (the spawn now replies `epic:mail`; `t7` is held under it, stays held after the promotion is approved, and runs on its epic's approval). New test `epic_spawned_while_promotion_awaits_stays_held` (the reviewer's scenario; red: the spawn replied `hold: null`; red again under the mutation that restores the exemption).
   2. *The submit's planners-finished refusal* on a promoted `running` run is scoped to `promotion_open(run)`, not `promoted(run)`: once the promotion is approved, a submit with a live planner is accepted, as on a planned run, which keeps M9.8's orchestrator-submit obligation workable. **Reasoning for keeping it while the window is open:** under rule 1 it is no longer needed for safety (a live planner's tasks name its epic and wait for the epic's round, whatever happens to `promotion`); it is kept because it is harmless (the orchestrator submits again once its planners end) and it makes the promotion round the user approves hold the orchestrator's whole first plan rather than part of it. Test `after_the_promotion_is_approved_a_submit_with_a_live_planner_is_accepted` (red: refused with the planner text).
   3. *Pinned survivors.* `an_epic_planned_before_the_gate_is_not_held` (M1: an addition to an epic with no round, planned before the gate, is not held and opens no round; red under the mutation that drops the "a round was opened" condition from `epic_open`) and `an_addition_to_an_approved_epic_is_not_held` (M2, decision 29; red under the mutation that drops the "none approved" condition). Both passed at once as pins of existing behaviour. `epic_open` is now "the epic's record names a round (`EpicRecord.gate_hold`) and none of its rounds is approved".
   4. *Empty `Drafting` rounds: the engine drops them.* `gate_holds::drop_empty_rounds`, run on every run after every event beside the scheduler, removes an epic round that is `Drafting` with no task once its epic's sub-planner is no longer queued or planning (`Finished` without a `submit_epic`, or `Failed`), with the log line `hold <id> dropped: its sub-planner ended with no task`, so it cannot keep the run from completing (decision 38). The epic's record keeps naming the dropped round, so the epic stays held: its next re-plan or addition opens a round again (the id is free again, so it is `epic:<e>` when that was the one dropped). M9.8 needs no extra obligation beyond setting the planner's phase when it ends. Test `an_empty_drafting_round_of_an_ended_planner_is_dropped` (kept while the planner is queued, dropped once it failed, and a re-spawn is held again; red: the round stayed).
10. **Final review of item 9: a split child follows its epic's rounds; re-plans open a round.**
    1. *The order for every added task* (`gate_holds::assign`), which **supersedes the first review's ruling 2 wherever the two conflict**: if the task names an epic whose rounds are open (`epic_open`), it takes that epic's open round, opening a new one after a rejection; else it inherits its split parent's unreleased hold; else the promotion rule applies. So a split child that names an epic follows the epic, not its parent: before, a split of the promotion-held `t2` into `t2a {epic: web}` put `t2a` in `promotion`, even after the user had rejected `epic:web`, and approving the promotion released web's work. A child naming an epic that is not open (no round, or its latest round approved) still inherits its parent's hold, then falls to the promotion rule, as the ruling orders. Test `a_split_child_naming_a_rejected_epic_opens_its_next_round` (in the new `engine/tests/gate_holds_epics.rs`, since `tests/promote.rs` is at 587 lines: `t2a` is held under `epic:web.2`, its sibling `t2b` under `promotion`, and `t2a` stays held after the promotion is approved; red: `t2a` was under `promotion`).
    2. *`Drafting` rounds with no mover.* `drop_empty_rounds` treats a round whose tasks are all finished or cancelled (or gone) as empty, so a `Drafting` round whose tasks the user cancelled, with its planner ended, is dropped and cannot block completion (decision 38). Test `a_drafting_round_whose_tasks_were_all_cancelled_is_dropped` (red: the round stayed `Drafting`). The first review's `a_users_split_of_a_held_task_releases_it_and_the_approval_counts_none` now puts its round `Awaiting` before the user's split, since a `Drafting` round the split left with no live task is now dropped. The obligation `orchestrator_submit_submits_its_drafting_epic_rounds` (commit `ca60139`) had landed under "### M9.10"; it is moved into "### M9.8", just before its "Acceptance", and names items 8 and 10.
    3. *A re-plan always opens a round* (the controller's decision). `replan_epic_hold`: a `spawn_subplanner` re-plan of an epic on a run past its gate is new, unreviewed work, so the planner's tasks join the epic's undecided round or a new one, `epic:<e>.<n>`, also after an approved round. `epic_open` now reads the epic's **latest** round (the one its record names): the epic is held while that round is not approved, so the new round holds the re-plan's tasks, and once it is approved an addition runs again. **Readings:** a single orchestrator `add_task` naming an epic whose latest round is approved still runs, per decision 29 as written (pinned by item 9's `an_addition_to_an_approved_epic_is_not_held`); a re-plan past the gate of an epic planned before the gate (no round yet) also opens a round, since it is equally new work; and after a re-plan's round is rejected, the epic's next addition opens the next round, as after any rejection. Test `a_replan_of_an_approved_epic_opens_a_new_round` (approve `epic:mail`; re-plan `mail` with a new brief; the reply names `epic:mail.2`; `t3` is held until the user approves it; red: the reply's `hold` was `null`). Item 9's M1 and M2 mutations, rewritten for the new `epic_open`, still turn their pins red, and only them.

### Task M9.8 (Engine: sub-planners and run scouts)

- **No preparatory commit.** `driver.rs` (597) is not touched. The driver's new code is in `driver/orch.rs` (`fill_extract`, `filled`), `driver/ops.rs` and `driver/effects.rs`. `engine/dispatch.rs` is 598 lines after it gained the worker's extract slot. The new engine code is in `engine/planners.rs` and `engine/run_scouts.rs`, and `orch.rs`'s `spawn_subplanner` moved into `planners.rs`. The new tests are in new files, because `tests/promote.rs` is at 587 lines: `tests/planners.rs`, `tests/planners_confine.rs`, `tests/planners_holds.rs` and `tests/run_scouts.rs`. The real session test is `crates/cli/tests/scout_service_planner.rs`, because `scout_service.rs` is at 592 lines.
- **The scout machine's texts.** `MachineTexts` has a fifth field, `missing` ("a report" or "an accepted epic"). The machine's failure reasons are templated from `noun` and `missing`, so a scout's texts are unchanged. `ScoutLimits` carries its `texts`, and `scout_event` takes them.
- **`ScoutEvent::Halt { reason }`** is new: the engine's stop of a planner (`max_rejections`) kills the session with the engine's reason.
- **`ScoutOutcome::Accepted`** is new: a planner whose epic the engine accepted (`accept_planner` drives `ReportAccepted`) ends `Accepted`, not `Reported`, because it stores no report.
- **Sub-planners on the scout service.** `scout/planner.rs` holds `PlannerSpec`, `PlannerTag`, `planner_names` (`<h4>-plan-<e>-<n>`, `<h4>/plan-<e>.p<n>`), and `start_planner`, `accept_planner` and `stop_planner`. `ScoutService::start` now delegates to a shared `launch`. A planner's table entry carries its tag. `claim` refuses a planner entry, and `run_scouts` leaves planners out.
- **`PlannerSpec` fields.** Beyond the brief's fields, `PlannerSpec` has:
  - `max_tool_calls` and `timeout_secs`, because the service knows only `[orchestrator.scouts]`;
  - `extract: Option<ExtractSlot>`.
- **`McpTarget.epic`** (`#[serde(default)]`) is new. `mcp_args` passes `--epic <e>` for a planner. The parsing of `--epic` in `anthrex mcp` is M9.11's. `crates/cli` builds its targets with `epic: None` until then.
- **The scout extract (decision 34).**
  - `run/orch/extract.rs::ExtractSlot { refs, onboarding, at, sep }` records where the extract goes. `ExtractSlot::fill` inserts it.
  - `OpKind::CreateWindow` gains `#[serde(default)] extract: Option<ExtractSlot>`. `dispatch.rs` sets it for a worker, and a handover takes the same slot (`worker_slot`).
  - The driver's `CreateWindow` arm fills the first turn on `spawn_blocking` before the window is created (`driver/orch.rs::fill_extract`). The reports are read only through `resolve_ref`, so an id that is not valid is never read.
  - `run/contract.rs::worker_extract_at` and `run/orch/contract.rs::planner_extract_at` give the byte offset where the prompt builders put the extract. The prompts are built with an empty extract, then filled, and the result equals the prompt built with the extract. Test: `a_filled_slot_is_the_prompt_built_with_the_extract`.
  - **Obligation for M9.13:** `PlannerSpec.extract` is set by `planner_spec`, but `StartPlanner` is answered "not available yet" until M9.13 executes it. M9.13's execution must fill the planner's first turn through `fill_extract`, as `CreateWindow` does.
- **`PlannerSession.rejections`** (`#[serde(default)]`) counts a session's rejected `submit_epic` batches toward `max_rejections`. In the epic's record:
  - `edits_rejected` counts rejected batches;
  - `edits_accepted` counts the edits of the accepted batch.
- **Reader slots.** `schedule::readers_busy` counts planning epics and running run scouts. Planner and scout sessions do not add to `windows_created`, so they do not count toward `max_windows`. They are bounded by the reader slots, and scouts also by `max_scouts`. Review this reading. The order is deciders, reviewers, planners, then scouts. The "kinds" part of `planner_waits_for_a_reader_slot_and_the_order_is_deciders_reviewers_planners_scouts_kinds` (research) is M9.9's. That test covers the order up to scouts.
- **A re-plan keeps the epic's area and title.** The `spawn_subplanner` of an existing epic starts a fresh session and takes the new brief. Its `area` is not re-validated or replaced.
- **Holds (the binding rule).** An accepted `submit_epic` runs its batch through `gate_holds::assign`, then `gate_holds::submit_epic_round`:
  - On a promoted or gated run, the epic's round goes to `Awaiting`, or with `--yes` it is approved.
  - A submit that adds no live task leaves its round `Drafting`, for `drop_empty_rounds`, so the user is never asked to approve nothing.
  - The orchestrator's `submit` on a running run calls `gate_holds::submit_epic_rounds`. That submits every `Drafting` round with live work whose epic has no live planner (`orchestrator_submit_submits_its_drafting_epic_rounds`).
  - Test for the rule, including after a re-plan: `a_promoted_runs_planner_and_its_replan_wait_for_their_rounds`.
- **Wake notes** are pushed to `orchestrator.notes` without a cap. The cap and the wake itself are M9.9's.
- **Early events.**
  - **The planner's `submit_epic` joins the hold** (`early::holds_planner_call`): a submit from a window no session has yet is held while the epic's latest session is launching. It is answered after binding, or refused if the launch fails. Tests: `a_planners_submit_before_its_window_is_answered_after_binding` and `a_planners_held_submit_is_refused_when_its_launch_fails`.
  - **The run scout's tool does not join.** A scout's `submit_scout_report` is answered by `ScoutService`, which binds the scout's window itself. It never reaches the engine's `early::holds_call`.
- **Security.** `planner_spec` launches read-only:
  - `cwd` is the run's root;
  - Claude runs with `--permission-mode dontAsk`, the reviewer's disallowed tools, and a sandbox with no writable root that denies writes to the protected paths, the root, the project and the git common dir;
  - Codex runs with the reviewer's read-only sandbox and the run's config guard.
  Tool calls are authorized by the window and epic together: a call from another window, or for a failed session, is refused as "not the sub-planner of epic <e>". `planner_and_research_specs_carry_the_read_only_sandbox` tests the planner half. The research half is M9.9's.
- **Tests updated.**
  - `orch_edit`: `spawn_subplanner`'s reply state is now `planning` when a reader slot is free, and the epic's phase is `Planning`.
  - `usage`: the `planner` role key and planner usage.
  - `tests/dispatch.rs`: the `CreateWindow` pattern takes `..`.
- **Red evidence.** Each new behaviour was mutated from a `cp` backup, and the pinning tests turned red. The mutations:
  - confinement off;
  - `pause` allowed;
  - another epic's split allowed;
  - no `submit_epic_round`;
  - no `assign` for a planner batch;
  - no `submit_epic_rounds`;
  - the worker slot dropped;
  - the planner hold off;
  - the area-glob rule off;
  - planners not counted as readers;
  - `max_scouts` off;
  - a scout report not recorded;
  - the driver's fill skipped;
  - the scout texts used for a planner (the real session test).
  The tests of new types and functions failed to compile before them.

### M9.8 review fixes

- **Ruling 1: a re-plan never joins an `Awaiting` round.** `gate_holds::epic_round` joins an `Awaiting` round only while the epic has no queued or planning sub-planner; otherwise it joins the latest round only when that round is `Drafting`, and opens a new one if it is not.
  - A re-plan marks its epic `Queued` before `replan_epic_hold`. So a re-plan reuses a `Drafting` round, or opens `epic:<e>.<n>` after an `Awaiting` or an approved one.
  - The planner's `submit_epic` then puts its additions and split children into that `Drafting` session round, and submits it. Approving an earlier round releases none of them.
  - **Reading:** "an orchestrator addition naming an epic whose planner is queued or planning joins that planner's `Drafting` round" cannot arise. `orch/rules.rs` (M9.4, rule `2.epic`) refuses the orchestrator's new task naming a live epic, with `epic <e> is being planned by its sub-planner`, and `is_live` covers `Queued`. `epic_round` would still put such a task in the `Drafting` round. Since the second review (ruling 2 below), rule `2.epic` also refuses the orchestrator's `split_task` of such an epic's task, and an `add_dep` onto one. A split child naming no epic would otherwise inherit its parent's `Awaiting` hold. So an `Awaiting` round is never joined while its epic is live.
  - When no planner is live, an orchestrator addition still joins its epic's `Awaiting` round, as M9.7 ruled for the promotion round.
  - Tests in the new `engine/tests/planners_review.rs`:
    - `a_replan_never_joins_an_awaiting_round` (the reviewer's sequence);
    - `a_replans_split_children_wait_for_its_own_round`;
    - `a_replan_opens_a_round_past_an_orchestrator_round_awaiting_the_user`.
    Red before the fix: the re-plan's reply named `epic:mail`, not `epic:mail.2`. With the fix's `Awaiting` filter mutated back, all three turn red again.
- **Ruling 2: a sub-planner amends only its epic's unapproved tasks.** `planners.rs::released_amends` refuses an `amend_task` of the planner's own epic's task with `task <id>: a sub-planner amends only its epic's unapproved tasks` when either:
  - the task has started (`session > 0`, or a state other than `pending`, `queued` or `blocked`);
  - or it is released past the gate (`gate_holds::released_past_gate`: the run was approved or promoted, and the task has no hold or an approved one).
  Another epic's task keeps M9.8's `2.epic` refusal. The orchestrator's and the user's amends are unchanged (decision 19).
  - The "started" clause is defensive. A task starts only once released, so no reachable state pins it apart from the release clause; its mutation survives.
  - Test: `a_planner_amends_only_its_epics_unapproved_tasks`. Session 2's amend of an approved but not started `t2`, and of a `Working` `t2`, are refused, with no `Deliver` and an unchanged outbox. Its amend of a `t2` still held is accepted. Red: both amends were accepted.
  - `an_accepted_epic_with_no_new_task_asks_the_user_nothing` amended an approved `t2`, which ruling 2 now refuses. It now amends the still-held `t2`, and also asserts that `epic:mail` still awaits the user.
- **Ruling 3: stale sessions.** A planner tool call is refused with `this window is not the sub-planner of epic <e> of run <id>` in any of these cases:
  - the caller's window is not the latest session's;
  - the latest session has `ended_at`;
  - the epic is not `Planning` (a queued re-plan has no session yet, so its previous session's window would otherwise still match).
  The latest session's own second `submit_epic` still gets `submit_epic was already accepted for epic <e>`.
  - Tests: `a_stale_sessions_submit_is_refused_and_the_queued_replan_stays_queued`, which is the reviewer's case with `max_readers = 0`, plus a variant with the old session's `ended_at` cleared, which pins the phase clause on its own. Also `an_ended_sessions_call_is_refused`. Red: the stale submit was accepted.
- **Ruling 4: bounds.** These are engine constants in `planners.rs`; M9.5's tuning may make them configurable.
  - `MAX_REPLANS_PER_EPIC = 3`: refused with `epic <e> was already re-planned 3 times, the most one epic allows`.
  - `MAX_EPICS = 20`: refused with `run <id> already has 20 epics, the most one run allows`.
  Tests: `replans_per_epic_are_bounded` and `epics_per_run_are_bounded`, both red before the fix. M9.9's task text now says that the wake notes M9.8 adds need M9.9's cap. M9.9 already had the cap test `notes_are_capped_at_20_with_the_earlier_line`; it did not say that M9.8's `wake_note` is uncapped.

#### Second review

- **Ruling 1: a sub-planner changes only its own round's work.** This replaces the first review's `released_amends` with `planners.rs::outside_its_round`, and fixes the reviewer's findings 1 and 2. For a task of the planner's own epic that existed before the batch:
  - `amend_task`, `split_task` and `add_dep` (the task as the dependent) are allowed only when the task is unstarted and its hold is the session's round. That round is `gate_holds::session_round`: the epic's current round, past the gate, while `Drafting`, which is the round the session's additions join.
  - `cancel_task` is allowed when the task is neither started nor released (`released_past_gate`), since cancelling unapproved work only reduces it.
  - Anything else is refused with `task <id>: a sub-planner changes only its own round's tasks` (rule `2.epic`).
  - So a later session cannot rewrite, split or add to a task in an earlier `Awaiting` round, where the change would ride on that round's approval; it may only cancel such a task. Another epic's task keeps `planner_confinement`'s refusal, and the orchestrator's and the user's rights are unchanged.
  - Tests in the new `engine/tests/planners_rounds.rs`:
    - `a_planner_cannot_cancel_split_or_add_a_dep_to_released_or_started_work`: an approved `t2`, then a `Working` `t2`; all three ops are refused, and the plan is unchanged.
    - `a_later_session_only_cancels_an_earlier_awaiting_rounds_task`: amend, split and `add_dep` of `t2` in the earlier `Awaiting` round are refused; its cancel is accepted.
    - `a_session_changes_its_own_rounds_tasks`: amend, `add_dep` and split of a task in the session's own round, and an amend of a task the batch adds, are all accepted.
  - The first review's tests are adjusted:
    - `a_replans_split_children_wait_for_its_own_round` is removed. Its split of the earlier round's `t2` is now refused, and `a_later_session_only_cancels_an_earlier_awaiting_rounds_task` pins that.
    - `a_planner_amends_only_its_epics_unapproved_tasks` now expects the new message and no longer accepts an amend of the earlier round's `t2`.
    - `an_accepted_epic_with_no_new_task_asks_the_user_nothing` now cancels the still-held `t2` instead of amending it.
  - **Reading:** a session's own round can hold tasks from before the batch only when the re-plan joined a `Drafting` round that the orchestrator had filled (ruling 4). A fresh `epic:<e>.<n>` holds only what the session's batch adds, and those tasks are its own. The `gate_hold.is_some()` guard in the round comparison is defensive: past the gate, a live session's round is `None` only if its round is not `Drafting`, which ruling 1 of the first review prevents.
- **Ruling 2: the orchestrator's split of a live epic's task is refused.** `Batch::epic_being_planned` (in `edits_orch.rs`, called from `split_owned` and `add_dep`) refuses, for the orchestrator's batch, a `split_task` of, or an `add_dep` onto, a task whose epic has a queued or planning sub-planner. It uses rule `2.epic`'s text, `epic <e> is being planned by its sub-planner`. The brief's ruling-1 reading above is corrected to say so. Test: `the_orchestrator_cannot_split_or_add_a_dep_to_a_live_epics_task`, the reviewer's probe (`t2` awaits under `epic:mail`, `mail` is re-planned, and the orchestrator's split of `t2` into `t2x` is refused). Red before the fix: the split was accepted and `t2x` was held under `epic:mail`.
- **Ruling 3: the `past_gate` clauses are pinned.** `before_the_gate_a_replan_changes_any_of_its_epics_tasks`: before the gate, a re-plan's session amends, splits and cancels its epic's tasks. The reading holds with ruling 1: before the gate no round exists and nothing is approved, so any unstarted task of the epic may change. Both mutations turn this test red:
  - removing `!past_gate(run) ||` from the non-cancel clause;
  - removing `past_gate(run) &&` from `released_past_gate`, which the cancel clause uses.
  It passed before the fix, as a pin.
- **Ruling 4, recorded (no change):** a re-plan may join a `Drafting` round the orchestrator filled, since a re-plan opens a new round only when the latest one is not `Drafting`. The planner's `submit_epic` then submits that round, and the user approves the orchestrator's and the planner's tasks together, knowingly: the round lists them all. `a_session_changes_its_own_rounds_tasks` exercises this.
- **Red evidence.** Before the fixes, three tests were red (the release, earlier-round and orchestrator tests). I then mutated each fix from a `cp` backup:
  - the own-round check removed;
  - the round comparison loosened to "has a hold";
  - the cancel rule removed;
  - the orchestrator's live-epic check removed;
  - each of the two `past_gate` clauses removed.
  Each mutation turned its tests red.

### Task M9.9 (Engine: research, review, integration, completion and wake notes)

- **Files.**
  - Preparatory commit `09d1ec5` moves `new_round` out of `dispatch.rs` into `engine/rounds.rs` with no behaviour change, so `dispatch.rs` stays under 600 lines (about 550 after M9.9).
  - Research and review tasks are in `engine/kinds.rs`. Decision 37's integration review and decision 38's `may_complete` are in `engine/integration.rs`, re-exported from `kinds`, so neither file passes 600 lines.
  - Decision 39's notes, cap and wake-up are in the new `engine/wake.rs`. `planners::wake_note` is gone, and every note source goes through `wake::note`.
  - The `research.md` sections are in the new `run/report_orch.rs`.
  - The new tests are in new files: `engine/tests/kinds.rs`, `kinds_integration.rs`, `kinds_complete.rs` and `wake_notes.rs`. `done.rs` is 586 lines.
- **Fields beyond Interfaces**, both `#[serde(default)]`:
  - `EpicRecord.integration_reviewed`, which counts real integration rounds;
  - `OrchestratorRecord.note_revs`, each note's digest revision. It is parallel to `notes`. A note is `PENDING` until `engine::finish` settles it at the step's final revision. `DigestRead` and `OrchestratorWoken` drop notes up to their revision.
- **Integration ids.** The id is the first free `<e>-int<n>`, counting from the next round's number (M9.4 ruling 7: a user's `run edit` may hold one). The title uses that same number. `integration_rounds` counts the rounds that actually ran, so a skipped id does not use up a bounce.
- **Research failures (decision 35's "M8a's worker rules"), the implementer's reading:**
  - A rate-limited or otherwise failed turn counts as a turn with no report. There is no wait-and-continue.
  - An authentication, billing or sandbox failure blocks at once.
  - A stall blocks at once (`blocked(environment)`) instead of an interrupt and a nudge.
  - A first mid-turn death resumes with `RESEARCH_RESUME_AFTER_EXIT`, and a second one blocks.
  - Run budgets are not applied to research sessions.
- **Decision 25's restart** reuses `run retry`'s rung-2 path, `requests::rung2`, now shared. So it escalates the route even when the rewrite changed the route. An L task, and a task still awaiting its dependencies, is not restarted. Only a `blocked(mis_sized)` task whose brief, acceptance, size or route changed restarts. Human, conflict and environment blocks stay blocked (`editing_a_human_blocked_task_does_not_restart_it`).
- **Wake-note sources, readings.**
  - A user-edit note is added in any run state, not only while the gate is open or after it.
  - The restart note is added on every `RestartOrchestrator`.
  - Blocked-task notes come from comparing each run before and after the step. They skip `message_pause`, and they are suppressed while the step is the orchestrator's own tool call.
  - Notes for a worker's discovery or risk and for a failed refresh are M9.13a's.
  - Delivering the wake-up is M9.13's. `Effect::WakeOrchestrator` is emitted, and the driver ignores it until then.
- **ResolveTarget** is still answered "not available yet" by the driver until M9.13. End to end, a review task therefore blocks `environment` with that text. The engine tests drive the result directly.
- **Edit log (decision 40).** `edit_log::record` takes the source and an `EditOutcome`. A rejected batch is logged with its error, and an empty batch is not logged. Only accepted batches count toward `plan_edits_since_approval`.
  - Because the digest fingerprint includes the edits, a logged rejection moves `digest_rev`.
  - Two M9.7 `orch_edit` tests compared whole runs across a refused batch, so they now compare through `but_the_log()`.
- **Decision 38 and the promoted run.** `run_e2e_adapt::e2e_promote_performs_and_the_task_continues` waited for `complete`. A promoted run's orchestrator cannot submit until M9.13 launches it, so with decision 38 the run correctly keeps running. The test now waits for `t1` to merge, then asserts the run is `running` with `plan_submitted` false. M9.16/M9.17 should assert completion once the orchestrator runs.
- **Snapshot and report.**
  - `RunInfo.integration` and `research_report` are now filled.
  - The driver writes `research.md` beside `REPORT.md`, on the same blocking task.
  - `PlanEditInfo` maps the record's source, accepted flag, error and recipients.
- **Usage sums saturate** (`signals::add_usage`); an overflow used to panic.
- **Constructed state.** `the_outbox_skips_a_reported_task`, `a_run_of_merged_and_reported_tasks_completes` and `completion_waits_for_holds_planners_scouts_integration_and_submit` set their state directly rather than driving it through events.
- **Red evidence.**
  - Against `todo!` and no-op stubs, 34 of the 39 new tests failed. The 5 that passed are `plan_path_has_no_integration_review`, a pin, and existing edit-log tests.
  - Each new behaviour was then mutated from a `cp` backup, and each mutation turned its test red:
    - the mis-sized check in `rewritten`;
    - the outbox's `Reported` skip;
    - the `quiet` flag;
    - the early `Scout` hold;
    - the first free id;
    - the round cap;
    - `engine_owned`;
    - the `Changes` clause;
    - the finish edit closing `Changes`;
    - a `Reported` dependency satisfying its dependents.

### M9.9 review fixes

- **C1 and I4: the user can always end a run.**
  - Once the user has cancelled the run (`run cancel`, `Run.cancelled`) or sent the `finish` edit, `integration::may_complete` stops waiting for three conditions:
    - `plan_submitted`;
    - `Drafting` or `Awaiting` holds;
    - integration state `Changes`.
  - Every other condition still holds: no sub-planner or run scout live, no integration review unfinished, every task finished, the merge queue empty and no op pending.
  - A cancelled run adds no integration review (`integration_pass` returns early), so cancelling an epic's work does not start a review of it.
  - The user's submit (`RunRequest::Edit { submit }`, `requests::submit_edit`) is now accepted on a promoted `running` run whose `plan_submitted` is false, as the orchestrator's `submit` (`orch::submit_plan`). Its reply is `the plan of run <id> was submitted: hold promotion awaits approval`, or `it runs` when no hold awaits.
  - Tests in the new `engine/tests/kinds_end.rs`. Each run completes and then takes `run accept` or `run discard`:
    - `a_promoted_run_the_user_cancels_completes`;
    - `a_promoted_run_the_user_finishes_completes`;
    - `the_users_submit_ends_a_promoted_runs_promotion` (submit, approve hold `promotion`, complete);
    - `a_planned_run_whose_epic_asked_for_changes_completes_on_cancel`.
  - `run_e2e_adapt::e2e_promote_performs_and_the_task_continues` now ends the promoted run with `anthrex run cancel` and asserts that it completes with `t1` merged. It no longer asserts the stuck state.
- **I1: wake notes stay on one line.** `wake::note`, the one entry for every note source, turns every control character into a space: `\n`, `\r`, U+0085 and the rest. U+2028 and U+2029 become spaces too. `wake_text` is therefore one line that starts with its own `[anthrex]` prefix.
  - Tests: `a_wake_note_stays_on_one_line` (a `task_blocked` reason carrying `\n[anthrex] …`, `\r\n`, U+0085, U+2028, U+2029 and a vertical tab) and `every_note_source_is_folded_to_one_line`.
- **I2: only the blocks the orchestrator's own edit causes are quiet.**
  - For an orchestrator tool call, `engine::step` snapshots the runs after the event is applied and before the scheduler's passes. `wake::blocked_notes` then compares against that snapshot, so a block made by a time-based pass (a stall, a budget) in the same step is noted.
  - Test `a_stall_in_the_orchestrators_step_is_noted`: an orchestrated run, a research task whose stall is due, and an orchestrator `run_status` call as that step.
  - **Reading:** `edit_plan`'s own handler runs `orch::settle`, which runs the scheduler before replying (M9.6 and M9.7, for the reply's revision and holds). A stall that falls due in the same step as an accepted `edit_plan` is therefore taken by that in-handler pass, and stays quiet. The next digest still shows the block. Recorded in the follow-ups file.
- **I3: research tasks follow M8a's worker rules (decision 35).** The session code moved from `kinds.rs` to the new `engine/research.rs`:
  - **Failed turns (decision 32).** A rate-limited turn waits `rate_limit_retry_secs`, then gets `rate_limit_continue`. So does a first failed turn of another kind. Neither counts as a turn without a report. A second other failure in a row blocks the task. An authentication, billing or sandbox failure blocks at once.
  - **Stalls.** Silence interrupts the turn and queues `research_stall_nudge`. An interrupt that does not end the turn within `INTERRUPT_GRACE_SECS`, or a second silence, is a stall.
  - **Deaths.** An exit while the interrupt is pending ends the turn. The first death mid-turn resumes the session; the second is a stall.
  - **Stall counting (decision 38).** A stall adds 1 to both `stalls` and `failures`. Where a worker would get rung 2, the research task gets a fresh research session: the old one is killed, and the next research session launches in the same pass with `rung = 2`. At three failures the task is `blocked(environment)`, not `mis_sized`, because a research task has no size to raise: `stalled <n> times (<f> failures in all); last: <reason>`.
  - **Sessions with nothing to resume.** An exit before the session had an id, a failed resume, or a session that ended with no id gets a fresh session with no failure counted, as a worker's does.
  - **Budgets (decision 40)** use the research rounds' spend:
    - the soft wrap-up, `research_wrap_up`, goes once per session;
    - a hard breach gets a fresh session, and the second blocks the task (`exceeded its budget twice; last: …`);
    - the total over all research sessions reaching the next size's budget is rung 4's ceiling, `blocked(human)`.
  - This supersedes the M9.9 note "Research failures".
  - Tests in the new `engine/tests/kinds_research.rs`:
    - `a_rate_limited_research_turn_waits_then_continues`;
    - `a_research_stall_is_interrupted_and_nudged_before_it_counts`;
    - `a_third_research_failure_blocks_the_task`;
    - `a_second_research_death_in_a_round_is_a_stall`;
    - `research_budgets_apply`.
- **M1:** `run override` of a research or review task, integration reviews included, is refused with `override applies only to code and docs tasks` (`gates::OVERRIDE_KINDS`). Test `override_is_refused_for_research_and_review_tasks`.
- **M2:** decision 25 restarts a task at most `MAX_REWRITE_RESTARTS` (3) times, counted in the new `TaskOrch.rewrite_restarts` (under `#[serde(default)]`). The next rewrite still applies, and the task is `blocked(environment)` with `rewritten 3 times; the user decides`. `rewritten` and `restart_rewritten` moved from `done.rs` to `batch.rs`, next to their one caller. Test `a_task_is_restarted_by_rewrites_at_most_three_times`.
- **M3: integration ids stay valid.**
  - `integration::free_id` makes an integration review only while the first free `<e>-int<n>` is a valid task id (`validate::is_valid_id`, now `pub(crate)`, at most 16 characters).
  - Otherwise nothing is made, and the snapshot's attention gains `epic <e>: no free integration review id` (`integration::attention`, derived, so it clears when the run is cancelled or finished).
  - `review_first_turn` numbers the round from the id's `n` (`integration::round_of`), as the title does.
  - Tests `no_integration_review_is_made_without_a_valid_id` and `the_integration_prompt_numbers_the_round_from_its_id`.
- **M4: the edit log.**
  - A rejected batch's error is stored as one line of at most `ERROR_MAX_CHARS` (300) characters, `…` included.
  - When the 50-entry log is full, the oldest rejected entry is dropped before any accepted one.
  - Tests `a_rejected_batchs_error_is_capped` and `a_full_log_drops_rejected_batches_first`.
- **M5:** `run stats` leaves `reported` task records out of the rows, but still counts them in `task_records`. Test `reported_tasks_are_left_out_of_the_rows`.
- **M6: a digest read drops only what its answer held.**
  - Each note gets a seq: `OrchestratorRecord.note_seqs` and `last_note_seq`, replacing M9.9's `note_revs` and its `PENDING` sentinel.
  - `engine::notes_seq(run)` is the highest seq a digest answer or wake-up built from `run` includes. `OrchEvent::DigestRead` carries it (`notes_seq`), and so do `Effect::WakeOrchestrator` and `OrchEvent::OrchestratorWoken`, the same race. Only notes up to that seq are dropped.
  - These events are engine-internal and never on the wire, so PROTO stays 10 and no `serde(default)` is needed.
  - **Obligation for M9.11 and M9.13:** the driver sends `wake::notes_seq` of the snapshot it built the `run_status` answer from, and the effect's `notes_seq` with `OrchestratorWoken`.
  - Test `a_digest_read_keeps_a_note_added_after_its_answer`. Every note source today also changes the digest, so the test adds the late note directly (`wake::note`) with the revision unmoved.
- **Also:** `edits::requeue_waiting` treats a `reported` dependency as met, as `schedule::unfinished_deps` does. It has no test of its own: a dependent that is queued while a writer slot is free is dispatched in the same step.
- **Files.** The new files are `engine/research.rs` (562 lines), `kinds_end.rs`, `kinds_research.rs`, `kinds_limits.rs` and `wake_fixes.rs`. `kinds.rs` is now 239 lines, `done.rs` 528 and `edits.rs` 596.
- **Red evidence.** In this round the fixes were written before their tests. Each test was then shown red by reverting its fix alone from a `cp` backup, 17 reverts in all. Every revert turned its tests red, and each file was restored:
  - C1's `ended` clause: 3 tests;
  - the user's submit;
  - I1's fold: 2 tests;
  - I2's snapshot;
  - I3's five rules (rate-limit wait, stall ladder, death, budget, third failure);
  - M1;
  - M2;
  - M3's id check and round number;
  - M4's clip and eviction;
  - M5;
  - M6's seq filter.
- **Second round: cancelling a run stops its planners and scouts.**
  - `run cancel` and the `finish` edit halt every queued or live sub-planner and run scout (`planners::halt_all`, with `run_scouts::halt_all`), where before the run waited for them to end on their own, for up to `[orchestrator.planners] timeout_secs`.
  - A live planner gets `Effect::StopPlanner`. A running run scout gets the new `Effect::StopScout { scout_id, reason }`, which the driver turns into `ScoutService::halt`, driving the machine's `ScoutEvent::Halt` as `stop_planner` does.
  - A queued planner or scout never starts. Each record ends `failed` (the outcome vocabulary of decisions 20 and 32; there is no `interrupted`) with `the run was cancelled`, or `the finish edit ends the run`.
  - The halt runs at the cancel and on every running pass of a cancelled or finishing run (`complete::finish_pass`), so a scout or planner the orchestrator asks for afterwards is dropped too.
  - A launch still in flight at the halt is stopped when its window arrives (`planners::started`, `run_scouts::started`).
  - **Reading:** the records fail at the halt, so completion does not wait for the halted sessions' end events. It waits for their launch ops, like any pending op.
  - Tests in the new `engine/tests/kinds_cancel.rs`, each written first and red:
    - `cancel_halts_planners_and_scouts`: a live planner, a running scout and a queued one. It checks both halt effects and that the queued scout never starts; once the sessions end, the run completes.
    - `the_finish_edit_halts_planners_and_scouts`.
    - `a_launch_in_flight_at_cancel_is_stopped_when_it_starts`: written after its code, and shown red by reverting each of the two `started` branches.
  - This supersedes the follow-up recorded in the first round.
- **`run retry` of a research or review task was already that kind's own path.** The first round's follow-up said otherwise, and was wrong:
  - these tasks never have a `start_commit`, so `requests::rung2` puts them back to `queued`;
  - the reader dispatch then starts a fresh research session, or resolves the review target again and reviews it afresh;
  - no `DiffSoFar` or worktree is involved.
  - `rung2` now also names the case explicitly (`is_reader_task`).
  - Pinning tests `retrying_a_research_task_starts_a_fresh_research_session` and `retrying_a_review_task_reviews_it_again` passed before any change. They turn red when `rung2` is forced onto the worker path.

#### Second review

- **C-1: a cancelled or finishing run takes no new work.**
  - Once `run.cancelled` is set or the `finish` edit was accepted, `orch::tool` refuses these calls with `run <id> was cancelled`, or `run <id> is finishing`:
    - the orchestrator's `edit_plan` with edits or `submit`;
    - `spawn_scout` and `spawn_subplanner`;
    - a sub-planner's `submit_epic`.
  - The reads still answer, and so does a summary-only `edit_plan`, which the completion note asks for.
  - The user's `run edit` on a cancelled run refuses `add_task`, `split_task` and `submit` with the same text; cancelling and other removals still apply.
  - `complete::finish_pass` now cancels every unstarted task when the run was cancelled, not only after a `finish` edit.
  - `refs_verified` and `final_checked` complete the run only while every task is still finished; otherwise the next pass verifies again.
  - `finish_pass`'s halt of a cancelled run's planners and scouts is gone: `cancel` halts them, and new ones are refused.
  - Tests in the new `engine/tests/cancel_work.rs`, each written first and red (the refusals were accepted, the guard completed the run):
    - `a_cancelled_run_refuses_the_orchestrators_new_work`: the reviewer's `--yes` scenario, with no window made for `t2`;
    - `a_cancelled_promoted_run_refuses_additions_and_completes`;
    - `a_cancelled_run_refuses_new_scouts_and_planners` (it also checks a live planner's `submit_epic`);
    - `a_finishing_run_refuses_new_work`;
    - `the_users_edit_of_a_cancelled_run_only_removes_work`;
    - `the_ref_guard_does_not_complete_a_run_with_an_unfinished_task`.
  - M-a's halt: removing `finish_pass`'s `halt_all` turns `the_finish_edit_halts_planners_and_scouts` red, and removing `cancel`'s turns `cancel_halts_planners_and_scouts` red (both checked by mutation).
- **I-1: the wake text stays on one line after its clamp.** `wake_text` now clamps with the new `WAKE_CUT_MARKER`, `" [anthrex: the middle of this note was cut to fit] "`. `MESSAGE_CUT_MARKER` stays for messages, whose test splits on it.
  - Test `the_wake_text_stays_on_one_line_after_its_clamp`, red first: 20 long notes stay at most `WAKE_MAX_BYTES` with no newline, and a padded `task_blocked` reason adds none.
  - `wake_text_is_clamped` now counts the new marker.
- **M-b: I2's gap is closed.** The orchestrator's `edit_plan` (and with it `submit`), `spawn_scout` and `spawn_subplanner` settle through `orch::settle_quiet`. That keeps the run as the call left it, before the handler's scheduler pass, in the new transient `EngineState.quiet_base`, and `engine::step` compares block notes against it.
  - **Reading:** "only the tasks the batch touched" is the blocks the batch itself made, `dep_cancelled` among them, as M9.9's `orchestrators_own_edits_add_no_note` pins.
  - Test `a_stall_in_the_step_of_the_orchestrators_edit_is_noted`, red first.
  - The follow-up entry is removed.
- **M-c: `rewrite_restarts` counts only a model's rewrites.** It counts the orchestrator's and, as the implementer's reading, a sub-planner's, since the cap stops a model's loop. The user's own rewrite always restarts and is not counted, and the user's `run retry` resets the count.
  - `a_task_is_restarted_by_rewrites_at_most_three_times` now drives the orchestrator's rewrites.
  - Test `the_users_rewrite_and_retry_are_not_capped`, red first: the user's fourth rewrite was blocked.
- **M-d:** a research turn interrupted before its session had an id gets the stall nudge appended to the fresh research session's first turn. This is what a worker's rung 2 does; the new `TaskOrch.research_append` carries it and is cleared at the launch.
  - Test `a_research_turn_interrupted_before_its_id_gets_the_nudge_next`, red first.

### Task M9.10 (The orchestrator window)

- **Launch.** `launch/role.rs` builds decision 7's and 8's blocks (`claude_role_args`, `codex_role_args`), the environment (`launch_env`: the role's `env`, then `ENABLE_TOOL_SEARCH=false` last for Claude, through `headless::session_vars`), and `otlp_env(addr, run_id, token)`, the OTLP variables plus the token header, for the driver to put in `RoleLaunch.env` (M9.13). The M9.1 real-CLI rulings are pinned by exact-argv and env tests in `launch/role_tests.rs`:
  - ruling 1: `--permission-mode default` right after `--disallowedTools` (`claude_orchestrator_argv_is_exact`);
  - ruling 2: the disallowed list, with `Task` and the outward-acting tools (the same test; the list itself was M9.7's);
  - ruling 3: `default_tools_approval_mode="approve"` (`codex_orchestrator_argv_is_exact`);
  - ruling 4: no `MCP_*` variable (`role_env_order_and_otlp`);
  - ruling 5: `SCRUBBED_PREFIXES` gains `"MCP_"` and `RESERVED_PREFIXES` gains `MCP_` (`reserved_env::mcp_variables_are_scrubbed_and_reserved`; the PTY scrub in `scrub_removes_agent_session_and_credential_variables`);
  - ruling 6: `ENABLE_TOOL_SEARCH=false` last, Claude only (`claude_orchestrator_env_turns_tool_search_off_and_codex_does_not`);
  - ruling 7: no Codex trust flag (`codex_orchestrator_argv_is_exact` asserts no `trust_level`).
- **Deviation: `LaunchContext` also gains `caps: &CliCaps`.** Decision 7's user-settings-only flags come from the caps, and `launch::plan` took none. `create` and `restart` pass `ManagerConfig::cli_caps`, so the debug-build test overrides (`ANTHREX_TEST_NO_SETTING_SOURCES`) reach the orchestrator too. Plain windows never read it.
- **Reading of `claude_argv_without_user_settings_only_caps`.** With `claude_user_settings_only = None`, neither `--setting-sources user` nor `--strict-mcp-config` is passed: decision 7 takes both from the caps. The test pins that, and that `--effort` follows `claude_effort_flag`.
- **Manager.** `manager/role_window.rs` holds `create_run_window` (phases A, B and C through `create.rs`'s `admit`, `spawn_window` and `insert`, now `pub(super)`; a 200 × 50 PTY; a spec with a worktree branch is refused), the run-live flag, client input time, the restored role's parse, decision 11a's placeholder spec, and `orchestrator_refusal`. `Spawned` carries the role from phase B to phase C, so `insert` keeps seven arguments (clippy). `Entry.run` is set to `{"role_launch": …}` at insert, so `state_snapshot` persists it unchanged; restore parses it back for a `Pty` record and keeps the value verbatim either way.
- **Deviation: `WindowManager::write_client_input`.** `server.rs` is at its budget, so the `Input` arm calls `write_client_input` (note, then write) instead of adding a `note_client_input` call. The engine's own writes (a wake-up, M9.13) go through `write_input` and are not client input (`client_input_time_is_recorded` checks both).
- **Decision 11a.** The placeholder is `HeadlessWindow.placeholder = true` with a spec that has no run, no MCP target and no session. The guard lets `Kill` and `Remove` through for it (`WindowManager::is_placeholder_headless`) and refuses `Subscribe`, `Input` and `Restart` as for any headless window. M8a.17's `headless_windows_persist_and_restore_as_ended` asserted the old exited-PTY fallback and now asserts the placeholder. **Left open** (followups file, "From M9.10"): the TUI cannot tell a placeholder from a repository scout's window and still refuses `C-b x` and `C-b X` for it.
- **`StopFailure`.** Besides `hooks.rs` and `launch/claude.rs`, two exhaustive matches take the new variant: `conversation/build.rs` closes the open turn on it, as on `Stop`, and `agent_state.rs` clears the root's tool on it, as on `Stop`.
- **Metering (decisions 14a, 14b).** The token check is in `metering/token.rs` and the slot table in `metering/slots.rs` (child modules of `server.rs`, which stays at 345 lines). A point of a live run is kept only when the request's `Authorization` equals `Bearer <token>` (compared in constant time); a dropped run is logged once at `warn` and counted at `debug`; the answer is still `200`. A connection becomes tokened when one of its requests carried a valid token for some run. The cap is read again whenever `live_generation` changes. When every slot is taken, the waiting connection closes the oldest untokened one, once, then waits up to `OTLP_SLOT_WAIT` as before.
  - **Deviation: the run service's `token` and `live_orchestrators` are implemented here** (`driver/usage.rs`, refreshed with the live runs after each step): the trait methods are required, and the receiver cannot be tested through the daemon without them. The token is the orchestrator record's `otlp_token` when it is not empty; nothing fills it yet (M9.13), so until then no run is metered. `orchestrator_token_is_persisted_and_survives_a_restart` stays M9.13's.
  - **Test changes.** `otlp_server.rs`'s requests carry the test sink's token; `hold_every_slot` now holds tokened connections (with a worker point, which posts no total), since idle untokened ones are now closed to make room. M8b's `e2e_otlp_usage_reaches_the_run_snapshot` (`cli/tests/run_e2e_adapt_engine.rs`) became `e2e_otlp_points_without_the_runs_token_are_dropped`: a `--plan` run has no orchestrator and so no token. The metered path end to end is M9.16's. The new tests live in `tests/otlp_server/tokens.rs` (`otlp_server.rs` would have passed 600 lines); `OTLP_MAX_CONNECTIONS` is now `OTLP_BASE_CONNECTIONS`.
- **Deviation: stand-ins, not `fake-agent`.** The daemon's integration tests cannot locate the `fake-agent` binary (another crate), so `tests/orchestrator_window.rs` uses a `/bin/sh` stand-in that records its argv and environment, as M8a.17's headless tests do. `stop_failure_hook_makes_the_window_idle` sends the two hooks as the `ClientMsg::HookEvent` frames `anthrex hook` sends, over the daemon's socket, with a subscribed client (a viewed window's finished turn is `Idle`; unviewed, it is `Done`).
- **`create_run_window_does_not_hold_the_manager_lock_across_spawn`.** A PTY spawn cannot be slowed from a test, so the test holds the launch at the gate and checks that `list()` answers at once, before and after. Phase B is `create`'s own `spawn_window` on `spawn_blocking`, whose slow-git lock test (`a_slow_worktree_create_does_not_block_the_manager`) covers the shared path. The `max_windows` half of `run_window_is_pty_with_run_ref_and_counts_to_max_windows` is the engine's and was already tested by M9.7 (`engine/tests/orch.rs`).
- **File sizes.** Over their budgets: `window.rs` +18 (budget +12: the scrub helper), `hooks.rs` +15 (budget +8, of which 11 are the new unit test), `manager/headless.rs` +6 (budget +5). `create.rs` is at +15, `restore.rs` +13, `restart.rs` +4, `entry.rs` +9, `launch/mod.rs` +20, `launch/codex.rs` +5, `launch/claude.rs` +2, `metering/server.rs` +34, `server.rs` +0. The new `tests/orchestrator_window.rs` keeps the socket tests in `tests/orchestrator_window/socket.rs`.
- **Timing budgets.** Three rows added to `docs/timing-budgets.md` (M8b.15's table).
- **Red before green.** Seen failing for the stated reason before the code: every pure test in `launch/role_tests.rs` except `plain_windows_are_unchanged` (a pin), `claude_settings_json_is_exact`, `stop_failure_parses_and_maps_to_stop`, `mcp_variables_are_scrubbed_and_reserved`; in `orchestrator_window.rs`, the scrub, persistence, placeholder, run-live and input-time tests; in `otlp_server.rs`, the three new tests; the TUI test. `create_run_window` had to exist for the manager tests to run, so five of them passed at once. A combined mutation (restart without the role, no persisted role, no role at spawn, the run-live flag ignored, `WindowInfo.run` without the role) turned three of those five red (`run_window_is_pty_…`, `restart_repasses_…`, `and_allowed_once_it_is_terminal`); `unparseable_role_restores_a_plain_window_with_a_warning` and the lock test are pins. Reverting `StopFailure`'s mapping turned the socket `StopFailure` test red. Sources were restored from copies.

### M9.10 review fixes

One commit, `fix(daemon): an orchestrator window never restarts without its role`. Each test was written first and seen red for the reason given.

1. **Critical, the controller's ruling (it overrides decision 11's plain-window fallback): a role that does not parse never runs again as a plain agent.** Without its role, a restart would launch a plain `claude` or `codex` with none of the read-only flags, no `--setting-sources user`, no scrub, and `--resume` of the orchestrator's session.
   - The window is restored as a PTY window whose `Entry.role` is `None` and whose `run` still holds the unparseable `{"role_launch": …}` (`role_window::lost_role`, derived, no new field). It comes back with no session id. The warning names the run (`pointer /run_ref/run_id`, else `unknown`).
   - `WindowManager::restart` refuses it (in `begin_restart`, under the lock, so the engine's `RestartOrchestrator` cannot either), and the guard refuses a client's `Restart` and `Input` with `window <id> was the orchestrator of run <run>, and its role could not be restored; it cannot be restarted or typed into. Remove it with anthrex rm <id>` (`manager::lost_role_refusal`). `Kill` and `Remove` work.
   - Tests: `unparseable_role_restores_a_plain_window_with_a_warning` became `unparseable_role_restores_a_window_that_never_restarts` (red: the session id survived); `socket::a_window_whose_role_is_lost_refuses_restart_and_input` (red: the restart was acknowledged).
   - **`#[serde(default)]`** is added only where a missing value is harmless: `RoleLaunch.env` (`remove_env` had it), `RunRef.task_id`, `McpTarget.task_id` (`scout_id` and `epic` had it). The tool lists, the contract, the effort, the run and the MCP role are never defaulted: a record missing one must not restore as a less restricted orchestrator. `RunRef` is a wire type, and a deserialization default changes no encoding, so there is no protocol bump. Test: `a_role_record_missing_optional_fields_still_parses` (red: `env` and `task_id` were required), which also checks that a record missing a tool list, the contract or the effort does not parse.
2. **More tools disallowed:** `Monitor`, `EnterWorktree`, `ExitWorktree`, `ScheduleWakeup` and `DesignSync` (M9.1 check 10). `claude_orchestrator_argv_is_exact` and `claude_argv_without_user_settings_only_caps` pin them (red first).
3. **OTLP eviction grace.** `OTLP_EVICT_GRACE = 2 s`: a connection younger than it is never closed to make room, so a real orchestrator's new connection cannot be closed before its first POST marks it tokened. Test `a_young_untokened_connection_is_not_closed_to_make_room` (red: the newcomer was served); `untokened_connections_are_closed_first_when_full` now waits out the grace. Both rows are in `docs/timing-budgets.md`.
4. **The scope of `MCP_` (accepted, recorded).** `MCP_` is in the shared `SCRUBBED_PREFIXES`, so besides every agent session it also scrubs the engine's commands (checks, proofs, `run/exec.rs`), and a profile may not set an `MCP_*` variable. That is accepted: anthrex configures every MCP client it starts, and no check needs one.
5. **`StopFailure` in headless sessions.** `headless/conversation.rs::observe_hook` treats `StopFailure` as `Stop` (it sets `hook_stopped`), so no second `Stop` is synthesised for a turn it ended. The change is in the existing match arm, so `conversation.rs` does not grow. Test `a_turn_ended_by_stop_failure_gets_no_second_stop` (red: a second `Stop`).
6. **`OTEL_` is scrubbed from agent sessions.** A new list, `config::reserved_env::AGENT_SCRUBBED_PREFIXES = ["OTEL_"]`, is applied by the orchestrator window's scrub (`window.rs`) and by every headless session (`headless/session.rs`), not by engine commands, and it is not reserved: a profile may still set `OTEL_*` for a worker's own tooling, and a user's test command may rely on its own. anthrex's OTLP variables are set after the scrub. **Finding:** no headless role relies on an inherited `OTEL_*` variable; the only `OTEL_*` the daemon sets are the orchestrator's (`metering::orchestrator_env`, `launch::role::otlp_env`), and headless sessions are metered from their streams with `CLAUDE_CODE_*` (so Claude's telemetry switch) already scrubbed. Test: `scrub_removes_agent_session_and_credential_variables` now inherits `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT`, which must be gone, and checks the role's `OTEL_EXPORTER_OTLP_ENDPOINT` (red: the inherited one reached the window).
7. **Test hygiene.** `scrub_removes_agent_session_and_credential_variables` sets its variables through an `EnvVars` guard that saves them under the shared env lock and puts them back under it on drop; the lock is never held across an `await`. The lock test is left as it is (a pin).

### Task M9.11 (MCP tools)

- **Schemas.** `crates/mcp/src/tools_orch.rs` holds the orchestrator's six tools, the sub-planner's two and the worker's `task_note`, in decision 15's order, with the Interfaces table's texts and bounds. `tools_for(Worker)` is `task_done`, `task_blocked`, `task_note`. `schemas_match_the_interface_table` compares every schema with `crates/mcp/src/fixtures/orch_tools.json`, which was built from the table alone. `every_schema_is_closed_at_every_level` walks `properties`, `patternProperties`, `items` and `oneOf` branches for every role, and counts the objects per role. The old `every_schema_is_a_closed_object` stays and now counts `task_note`. `orchestrator_tools_are_empty` is removed. `no_role_gets_a_tool_it_must_not_have` (not named by the brief) pins each role's exact list and `allowed` for every tool of every role; a decider has none, and no tool name approves, accepts, merges or overrides.
- **`anthrex mcp`.** `McpOptions.epic` is new, and `forward` sends it as `ToolCall.epic`. `--epic` is parsed. **Deviation:** clap cannot state a per-role presence or conflict, so `McpArgs::into_options` now returns `Result<McpOptions, clap::Error>`, and `main.rs` exits with the error. That is a usage error, exit code 2, as clap's own are:
  - `--role planner requires --epic <e>`;
  - `--epic <e> is accepted only with --role planner`;
  - `--role scout takes exactly one of --scout <id> or --task <t>`.
  M8b's scout MCP tests pass. `mcp_parses_the_daemons_headless_argv_and_is_hidden` now also covers a planner with its epic and a research task's scout window, which has `--task` and no `--scout`.
- **Deviation: where the tests live.**
  - `planner_needs_epic_and_orchestrator_refuses_it` and `scout_accepts_task_instead_of_scout` are in `crates/cli/src/mcp_cmd_tests.rs`, since the flags are the CLI's. The brief lists them under `crates/mcp`.
  - `tool_outside_the_role_is_refused_without_a_daemon` is in `crates/mcp/tests/stdio.rs`. It also shows that a tool inside the role does reach the socket.
  - `a_planner_call_carries_its_epic` (stdio) and `a_role_flag_mismatch_is_a_usage_error` (`crates/cli/tests/mcp_cli.rs`, the built binary) are added.
  - The driver tests are in-crate: `run/driver/orch_read_tests.rs` (`run_status`), `orch_read_tests_tools.rs` (the other tools) and the shared rig `orch_read_rig.rs`. `crates/mcp` cannot depend on the daemon, and the orchestrator window's launch is M9.13's, so the run is put in the engine by hand with its orchestrator record's window set. Its `digest_fp` is settled first, so the first step bumps nothing. It is `running`, with one task working in a window that does not exist and one blocked, so the scheduler starts nothing.
  - The test MCP client is `mcp::forward`, the exact path `anthrex mcp` takes, over a real socket served by `server::serve`. So `crates/daemon` gains `mcp` as a **dev-dependency**. `rmcp` is compiled into the daemon's tests only, never into the daemon.
- **Routing (decision 15).** `driver/adapt.rs::tool` sends every orchestrator and planner call, and a worker's `task_note`, to `driver/orch.rs::orch_tool`, ahead of M8b's scout branch.
  - `get_context`, `run_status` and `task_result` go to the read path.
  - Everything else goes to the engine as `OrchEvent::Tool`. `task_note` is answered `tool task_note is not available yet` until M9.13a. `WORKER_MCP_TOOLS` stays at two until M9.13a too.
  - An `edit_plan` or `submit_epic` batch carries the runtime refusals of ruling T22-I1b. `run edit`'s computation became `RunService::runtime_refusals`, shared by both. It runs for a caller that passes decision 15's check, and, since the review fixes (finding 2), for a planner call the engine would hold (`early::holds_planner_call`). So an unbound window that names a launching planner's epic can make the daemon run the project-settings git reads; see the re-review fixes, item 4, for why that is accepted.
  - `driver/requests.rs` is 597 lines (588 before).
- **The caller check** for reads is the engine's. An orchestrator call must come from `orchestrator.window_id`. A planner call must come from its epic's latest live session, in `Planning`: `EpicRecord::is_live_caller`, now shared with `engine/planners.rs::tool`. So a sub-planner whose `submit_epic` was accepted can no longer read its context. The order is the run (`unknown run`), the caller, then `parse_call` (the role's list and the arguments). Reads answer in every run state, terminal ones included.
- **`run_status` (decision 16).**
  - With `since` and `wait_secs > 0`, the driver subscribes to the snapshot pushes, then looks the run up once under the lock. So a change published before the subscription is not missed.
  - It then waits on the pushes with a hard deadline. The wait ends when the run's `digest_revision` differs from `since`, when its state `is_terminal()` (accepted, discarded, failed), or when the run is gone. `complete` is not terminal, but reaching it changes the digest.
  - On `RecvError::Lagged` it subscribes again and looks again.
  - The answer is built from a clone taken under the lock, on `spawn_blocking`. Then `OrchEvent::DigestRead` carries that clone's `digest_rev` and `wake::notes_seq` (the M9.9 M6 obligation). A `since` other than the current revision, or `wait_secs` 0, answers at once.
- **`get_context` (decision 17).**
  - Its reads, on `spawn_blocking`, are:
    - the stored profile, through `profile::store::load` of `run.repo_dir`;
    - the onboarding report, when `run.onboarding_report` is a valid id;
    - each id in `run.scout_reports`.
  - Every ref is resolved only through `scout::report::resolve_ref`, and read through `read_report`'s guards. One that cannot be read is left out with a warning.
  - An empty `repo_dir` reads nothing.
  - **Not in the brief:** the reads and the build share one `CONTEXT_READ_TIMEOUT` (10 s).
- **`task_result` (decision 18).**
  - The git reads run only when the task has a `start_commit` (the M9.6 obligation), with `None` passed otherwise.
  - Both reads run in one `spawn_blocking` under one `DONE_CHECK_GIT_TIMEOUT` deadline (the M-4 obligation). Each git command's own bound is the smaller of the run's `git_timeout_secs` and 10 s.
  - Past the deadline the answer has `git: "git did not answer within 10 s"`, and the abandoned read ends on its own bound.
  - The build runs on `spawn_blocking` (the second-review Minor 5 obligation).
  - **Left for M9.13:** `resolve_target` is not on the read path. It is the `ResolveTarget` op's, which M9.13 executes, so the one-deadline rule for it is M9.13's.
- **Tests.**
  - mcp: `tools_for_orchestrator_and_planner_are_exact`, `worker_tools_are_task_done_task_blocked_and_task_note`, `every_schema_is_closed_at_every_level`, `schemas_match_the_interface_table`, `no_role_gets_a_tool_it_must_not_have`, `tool_outside_the_role_is_refused_without_a_daemon`, `a_planner_call_carries_its_epic`.
  - cli: `planner_needs_epic_and_orchestrator_refuses_it`, `scout_accepts_task_instead_of_scout`, `a_role_flag_mismatch_is_a_usage_error`.
  - daemon: the brief's seven driver tests. Also:
    - `a_lagged_wait_subscribes_again_and_keeps_waiting`, on one thread, so the lag is certain;
    - `writes_go_to_the_engine`;
    - `task_result_answers_a_git_error_at_its_one_deadline`, which uses a stand-in `git` that sleeps 8 s a call.
  - Their bounds are in `docs/timing-budgets.md` ("Recorded, from M9.11").
- **Red before green.**
  - mcp: the five lib tests failed on the empty orchestrator and planner lists and the two-tool worker list. The stdio tests failed to compile (no `McpOptions.epic`).
  - cli: the flag tests failed with `--epic` parsed and no checks (a planner without `--epic` was accepted, and a scout with neither flag too).
  - daemon: all seven driver tests failed before the routing. Each call reached M8a's worker gate: `tool run_status is not available to the orchestrator role`, and the waits answered at once.
  - Mutations, each restored from a `cp` backup:
    - `Lagged` treated as the end turned `a_lagged_wait_…` red;
    - dropping `is_terminal()` from the wait's end turned `terminal_run_ends_the_wait` red;
    - skipping the `DigestRead` turned `run_status_returns_at_once_without_since` red, since the note was never dropped;
    - bypassing the routing turned `writes_go_to_the_engine` red;
    - widening the one git deadline to 60 s turned the deadline test red, since it answered at 16 s with no error.

### M9.11 review fixes

One commit, `fix(daemon): early M9 calls wait for their launch, and the daemon enforces the tools' nested bounds`. Each test was written first and seen red for the reason given.

1. **A session's calls before its window is bound (Important).** The engine learns a sub-planner's window from its `StartPlanner` result and the orchestrator's from its `CreateOrchestrator` result. The session can call before either arrives.
   - The new `early::awaits_launch(state, call)` is the rule. The call comes from a window nothing has bound yet, and it is one of these:
     - a planner call naming an epic whose latest session's `StartPlanner` is in flight (M9.8's `planner_launching`);
     - an orchestrator call while the run's `CreateOrchestrator` op is pending, from any window but the record's own.
   - `holds_planner_call` is now "`submit_epic` and `awaits_launch`", so the engine's hold uses the same rule as the driver's wait. `engine::early` is `pub(crate)`, which leaves `engine/mod.rs` at 599 lines.
   - **Decision.** The driver's reads (both roles) and the orchestrator's writes wait while `awaits_launch` holds, polling the engine state every `LAUNCH_POLL` (50 ms). Each look takes the lock only for itself. The wait is bounded by the engine's own `HOLD_LIMIT_SECS` (30 s). The caller is then checked as before, so no check is weaker than the engine's: a failed launch is refused with `this window is not the sub-planner of epic …`.
   - A sub-planner's `submit_epic` is still held by the engine, not the driver.
   - The engine has no hold for the orchestrator's calls. Its tool gate would refuse an early call, so the driver's wait covers the orchestrator's writes too. The engine gains no second hold for the same case.
   - Tests:
     - `early_events::a_planners_read_before_its_window_waits_for_its_launch` and `the_orchestrators_calls_before_its_window_wait_for_its_launch` (red against a stub that answered `false`);
     - `driver/orch_read_tests_launch.rs::a_planners_first_read_waits_for_its_launch`, bound and failed (red: the read answered at once with the refusal);
     - `driver/orch_read_tests_launch.rs::an_orchestrators_first_calls_wait_for_its_launch`, a `run_status` and an `edit_plan` (red: both answered at once, refused).
2. **A held `submit_epic` carries its runtime refusals (Important).** `tool_refusals` computes the refusals when the caller passes decision 15's check, and also when `holds_planner_call` would hold the call. So the replayed batch carries them (ruling T22-I1b).
   - Test: `a_held_submit_carries_its_runtime_refusals`. The planning run has no task, and its orchestrator and planners run on Codex, so it reaches Codex only. Planners are at standard strength, since the roster's Codex has no frontier model. `cli_caps.claude_user_settings_only` is `None`, and the base commit tracks `.mcp.json`.
   - The early `submit_epic` adds a Claude task. It is held, and then refused on the `PlannerStarted` replay with the project-settings refusal.
   - Red before the fix: the replay answered `Epic recorded. You are done; end your turn now.`
3. **The daemon enforces the nested bounds (Minor).** The new `run/orch/tools_bounds.rs::check_edit` runs on each raw edit before serde, for `edit_plan` and `submit_epic`. It applies the Interfaces table's string and array bounds in `plan_edit`, `plan_task` (at the top and in `into`) and `route`: `brief` ≤ 8000, `title` ≤ 120, `owns` ≤ 20 of ≤ 300, `into` 1–12, `to` 1–20 of ≤ 16, `route.model` 0–100, and so on. The paths look like `edits[0]: into[1]: brief: must be 1 to 8000 characters`. Fields the schemas lack are refused (`task: hub: unknown field`).
   - **Readings.** Two cases are left to their existing, documented refusals:
     - an edit whose `op` is not the schema's goes to serde (`unknown variant \`override\``, decision 19; `orchestrator_tools_cannot_approve`);
     - `plan_task.budget` goes to rule 7.1's `budgets come from the task's size; leave budget out` (decision 23.1; `edit_plan_reply_shapes`).
   - Enums, integers and patterns stay serde's and the plan rules'. `tools_orch.rs`'s comment, that the daemon refuses what breaks the limits, is now true.
   - Tests in the new `run/orch/tools_tests_bounds.rs`:
     - `a_task_at_every_bound_is_accepted`, a pin;
     - `nested_strings_past_their_bound_are_refused`, whose first case is the reviewer's 200 000-character brief;
     - `nested_lists_past_their_bound_are_refused`;
     - `unknown_nested_fields_are_refused`.
     The last three were red: the long brief was accepted, and an unknown field reached serde.
4. **The long-poll's deadline spans resubscriptions (Minor).** Test `the_deadline_spans_lagged_resubscriptions`. A sender thread floods the snapshot channel for 12 s, and `run_status` with `wait_secs = 2` still answers in 2 to 5 s with the same revision. It passed against the code as it was; the mutation that sets the deadline inside the resubscribe loop turns it red (answered after 14 s).
5. **Sizes and lines.** `driver/requests.rs` (597) and `cli/src/main.rs` (600) are not touched. `mcp_cmd.rs`'s long doc line is wrapped, and so is one pre-existing long line in `early.rs`'s module doc. The rows for the new timing bounds are in `docs/timing-budgets.md`.

#### Re-review fixes

One commit, `fix(daemon): null is absent in tool bounds, and an early orchestrator write that outwaits its launch is refused`. Each test was written first and seen red, except where noted.

1. **`null` in an optional field is absent.** `tools_bounds.rs` skips a `null` value, as serde's `Option` reads it. A field that is not an `Option` still refuses it, with serde's own text: `"route": null` gives `invalid type: null`, as it did before the bounds existed.
   - Test `null_in_an_optional_field_is_absent`: `task.epic`, `task.test_to_write`, `route.model`, and `amend_task`'s `brief` and `acceptance` as `null` are accepted, while `brief: 7` is still refused.
   - Red: `epic: null` was refused with `must be a string`.
2. **The launch wait's limit is tested.** `orch_tool` is now `orch_tool_within(call, LAUNCH_WAIT)`, where `LAUNCH_WAIT` is `HOLD_LIMIT_SECS` (30 s) in production. `await_launch` takes the limit and returns whether the launch is over.
   - Test `a_launch_wait_ends_at_its_limit`: the orchestrator and a sub-planner are both launching, and neither resolves. Each one's `get_context`, at a 300 ms limit, is answered with its caller refusal.
   - It passed once the limit became a parameter. With the deadline check removed, it and item 5's test hang past their 10 s bound, so both go red.
3. **A task's `id` is the plan rules'.** The `id` bound is removed from `tools_bounds.rs`.
   - Tests `a_task_id_is_left_to_the_plan_rules` (parse) and `engine/tests/orch_edit.rs::a_long_task_id_gets_the_plan_rules_error`: a 17-character id gets decision 19's structured error, with `task`, `field: "id"` and `rule: "id"`.
   - Red: `task: id: must be 1 to 16 characters` pre-empted the plan rule.
4. **The brief's text about the refusals' reach is corrected** in the Task M9.11 notes above. An unbound window that names a launching planner's epic now triggers `runtime_refusals`, whose project-settings reads are git calls on `spawn_blocking` bounded by `git_timeout_secs`. This is accepted, not closed, for two reasons:
   - the refusals must exist before the engine holds the batch, and the engine alone decides at replay which session the window was;
   - the reads are bounded, read-only, and can only be triggered while a `StartPlanner` of that run is in flight.
5. **An orchestrator write still waiting at its limit is refused in the driver** with `the orchestrator's launch has not finished; call again once it has` (`ORCHESTRATOR_LAUNCH_PENDING`), and never sent to the engine. Otherwise a `Window` result reduced just before the event could let the batch through without the ruling T22-I1b refusals.
   - Test `an_orchestrator_write_whose_launch_never_finishes_is_refused`, at a 300 ms limit. The engine logged no batch.
   - Red: the call reached the engine and was answered `this window is not the orchestrator of run …`.

### Task M9.12 (`fake-agent`: an orchestrator in a PTY, planners and run scouts)

- **Argv from the daemon's own builder.** `tests/orch_modes.rs` builds the orchestrator's Claude and Codex argv with `daemon::launch::plan` and a `RoleLaunch` (the shape of M9.10's `claude_orchestrator_argv_is_exact` and `codex_orchestrator_argv_is_exact`), and the planners' and scouts' Codex argv with `daemon::headless::argv::codex_args`, so no argv is guessed. The `exe` the daemon names is a stand-in that logs `hook` calls and runs the real `anthrex mcp` for everything else; the daemon is M8a's stub.
- **Script keys (`roles::key`, used by both modes).** Orchestrator: `run` (`orchestrator-run-<n>`, as smoke stage 11f writes it). Planner: `--epic`. Scout: `--scout` without the `<h4>-` prefix when `--run` is given (the prefix is the run id's last 4 characters, `Run::short`), else `--task`. Everyone else: `--task`. A repository scout (no `--run`) keeps M8b's id and its timestamp fallback.
- **PTY `read_message`.** In PTY mode with an MCP server, the terminal is put in raw mode (no `ICANON`, `ECHO`, `ICRNL`; signals kept), as a real TUI does, so a paste of any length arrives whole and `\r` arrives as `\r`. A message ends at `\r` or `\n` outside a paste. The paste brackets are removed from the text.
  - The `FAKE_AGENT_STDIN_FILE` line per read is JSON: `{"at": <UTC ISO time>, "raw": <the bytes read, brackets and \r included>, "text": <the message>}`.
  - `Stop` goes through the Claude hook. A Codex orchestrator has no `Stop` hook on its argv (`codex_hook_source` is unset), so its turn end is its `notify` (`agent-turn-complete`), and it sends no `UserPromptSubmit`.
  - EOF exits 0, a timeout exits 4, and an `expect` mismatch exits 3, as in M8a.
  - The message is kept in the runner. It is not exported as an environment variable, because PTY mode runs no `sh` step.
- **Resume.** A PTY session's id is Claude's `--resume <id>` or Codex's `resume <id>`, else `fake-session-<ANTHREX_WINDOW_ID>`, the id its hooks report. A resumed session continues its claimed script from the saved position, as a headless one does. Nothing tests this yet; M9.17's restart scenario will.
- **Steps (my readings where the brief is silent).**
  - `mcp_until`: an error reply exits 3, as an unexpected error does for `mcp_call`. Calls are 200 ms apart. The timeout is checked after each call, so the last call may run past it. In a headless turn, an interrupt ends the wait.
  - `capture_json`: nothing at the pointer, or a result that is not JSON, exits 3.
  - `expect`: a result that is not JSON differs, so it exits 3.
  - `expect_error_contains`: a substring check on the last result.
  - All four also run in headless turns, through the `orch_steps::Host` trait.
- **`FAKE_AGENT_MCP_LOG`.** Each line's `result` is the reply's text (what `FAKE_AGENT_RESULT` holds), not parsed JSON. Every call of an `mcp_until` is logged. Each line is written with a single append.
- **Deviations: files outside the brief's list.**
  - `headless.rs` and `headless_steps.rs` changed: the headless role and key parsing moved into `roles::key` (headless.rs 576 → 571), and the new `Step` variants need arms in `headless_steps.rs`'s exhaustive match.
  - `Cargo.toml` gained `portable-pty` as a dev-dependency, for a real PTY.
  - `tests/headless_support/stub_daemon.rs` gained `StubDaemon::replies`, which scripts a sequence of replies, the last one repeating.
  - `tests/headless_support/mod.rs` gained `codex_argv_for`.
  - The PTY and argv helpers went to a new `tests/orch_support/mod.rs`, which keeps `orch_modes.rs` under 600 lines.
  - M9.8's `crates/cli/tests/scout_service_planner.rs` now writes `planner-mail-1`, the brief's new name, where it wrote `planner-1`.
- **Test name.** The brief's "`mcp_until_polls_until_the_pointer_matches`, `and_times_out`" became two tests, `mcp_until_polls_until_the_pointer_matches` and `mcp_until_times_out`.
- **Red before the change.**
  - The six PTY tests ran until `MCP_RUN` (150 s). With no MCP-aware PTY mode, the process found no script and waited for EOF.
  - `pty_read_message_…` never saw a `Stop`.
  - `script_names_…` got exit 0 for the planner and the research scout, which claimed no script, and 99 for the run scout, which claimed the unstripped `scout-7a2c-api-1`.
- **File sizes.**

  | File | Lines |
  |------|-------|
  | `orch_steps.rs` | 563 |
  | `script.rs` | 456 |
  | `main.rs` | 387 |
  | `roles.rs` | 370 |
  | `mcp.rs` | 217 |
  | `headless.rs` | 571 |
  | `headless_steps.rs` | 358 |
  | `tests/orch_modes.rs` | 361 |
  | `tests/orch_support/mod.rs` | 274 |

### M9.12 review fixes

These supersede the raw-mode and step readings above where they differ.

1. **Raw mode only while reading a message.** `read_message` puts the terminal in raw mode before its `Stop` hook and restores the saved settings before its `UserPromptSubmit`. A guard (`RawMode`) restores them on every way out, including a timeout or EOF. M3's `read_line` therefore reads a typed `\r` (as `\n`, through `ICRNL`) before and after a `read_message`, in the orchestrator's script and in the `FAKE_AGENT_SCRIPT` fallback.
   - Test: `read_line_works_around_read_message_in_pty_mode`.
   - Red: the first `read_line` never returned, and the test hit its 20 s bound.
2. **Restored on exit.** The saved settings come back after every `read_message` and on every return from it, so the process exits with the settings it started with. A signal that kills it mid-read cannot restore them, but the PTY is the window's own.
   - Covered by the same test: the `read_line` after the message reads `y\r`.
3. **`\r\n` is one Enter.** I chose to swallow the `\n` rather than skip empty messages outside a paste: skipping would also swallow a deliberate empty Enter, and would hide a script's mistake.
   - After a message that ends at `\r`, a `\n` already buffered is dropped at once. One that has not arrived yet is dropped if it is the next read's first byte.
   - The message's text has `\r\n` and `\r` as `\n`, as Claude reports a prompt. `raw` keeps the bytes, without the dropped `\n`.
   - Test: `crlf_is_one_enter_and_the_text_normalises_line_ends`. It covers both a buffered and a late `\n`, and a paste with `\r\n` and `\r`.
   - Red: the prompts were `["first", "", "a\r\nb\rc"]`.
4. **Three more fixes.**
   - **`expect_error_contains` needs an error.** It requires the last `mcp_call` (or `mcp_until` call) to have failed; `Vars` gains `#[serde(default)] last_error`, set by both modes' calls. Otherwise it exits 3.
     - Test: `expect_error_contains_needs_an_error_reply`.
     - Red: exit 0.
   - **`first_at`.** Each `FAKE_AGENT_STDIN_FILE` line gains `first_at`, the time its first byte was read, beside `at`, the time its last byte was read. A byte typed while no `read_message` is waiting sits in the terminal until the next read, so its `first_at` is that read's time.
     - Test: `stdin_file_records_when_a_message_started`, with 400 ms between two writes.
     - Red: `first_at` was missing.
   - **The `mcp_until` bound.** No call starts once the deadline has passed; the wait after a call is cut at the deadline. The step therefore ends at most one call's duration after `timeout_ms`, the call already in flight.
     - Unit test: `orch_steps_tests.rs::mcp_until_starts_no_call_after_its_deadline`, a host whose calls take 50 ms, with a 120 ms timeout.
     - Red: 2 calls. The old loop started a call at the deadline.
     - A stub-daemon version of this test depended on the real `anthrex mcp`'s start-up time and was not reliably red, so it was dropped for the unit test.
- **Files.** `orch_steps.rs`'s unit tests moved to `src/orch_steps_tests.rs`, with `#[path]` as elsewhere in the workspace, to keep it under 600 lines.

  | File | Lines |
  |------|-------|
  | `orch_steps.rs` | 525 |
  | `orch_steps_tests.rs` | 139 |
  | `roles.rs` | 373 |
  | `headless_steps.rs` | 359 |
  | `tests/orch_modes.rs` | 484 |

### Task M9.13 (Driver: ops and wake delivery)

Two commits: a preparatory move, `refactor(daemon): move run start's build and the driver's context into their own files` (d8c04e3; `driver/build.rs` takes `build_plan` and its helpers from `requests.rs`, `driver/context.rs` takes `RunContext` and `OpCtx` from `driver.rs`, no behaviour change), then the task.

- **Where things are.** `driver/orch_ops.rs` executes `CreateOrchestrator`, `RestartOrchestrator`, `StartScout`, `StartPlanner` and `ResolveTarget` (`ops.rs` only dispatches to it); `driver/wake.rs` holds decision 39's delivery and decision 13's watch of the orchestrator window. `effects.rs` turns `Effect::WakeOrchestrator` into `queue_wake`.
- **Deviation: `RunContext` is unchanged.** The brief adds the scout service, profile access and the agent binaries to it. They are already reachable: the scout service and profiles through the adaptation wiring (`RunService::adaptation`), the binaries through the manager's config (`RunContext::manager`). Adding them again would give two sources for each.
- **Deviation: the OTLP token (decision 14a).** A planned run's token is drawn at build (`/dev/urandom`, 16 bytes as 32 lowercase hex, on `spawn_blocking`). A run whose record has none (a promoted one) gets one from the driver at `CreateOrchestrator`, sent to the engine as a new `OrchEvent::OtlpToken`, which sets it only when empty and persists urgently. It is never in the snapshot. `otlp_env` is added to the Claude orchestrator's `RoleLaunch.env` once `otlp.addr` exists; `remove_env` is `credential_scrub_for(runtime, auth)`.
- **Deviation: decision 13 needs two engine inputs the brief does not name.** `OrchEvent::OrchestratorWindow { run_id, window_id, live }` reports that the window exited (or came back after one), and `OrchestratorRecord.start_error` (`#[serde(default)]`) keeps a failed `CreateOrchestrator`'s reason until a launch succeeds. The attention lines are `the orchestrator (window <n>) exited; restart it with anthrex restart <n>` and `the orchestrator could not start: <error>`; the second's wording is mine. Wakes are dropped while the orchestrator is not live and its notes accumulate, as the engine already did. Exit is detected from the manager's window list: a watch on it (`manager.watch()`) and the driver's 1 s tick both call `check_orchestrators`.
- **Wake delivery.** One pending wake per run; a newer one replaces it. Delivered only when the window is `Idle` or `Done` (never `Working` or `Attention`) and no client input reached it for `wake_quiet_secs`. The paste and the `\r` go through `write_input`; the 200 ms `SUBMIT_DELAY` is a sleep on a spawned task, with no lock held (`wake_writes_happen_outside_every_lock`). `OrchestratorWoken` carries the effect's `digest_revision` and `notes_seq`; a still-pending wake with no newer note is dropped once delivered.
- **Run-live flag (decision 11).** Set on a successful create and restart, and for every restored non-terminal run's orchestrator window after restore; cleared on the publish that shows the run terminal.
- **Scouts and planners.** The first turn of a planner is filled through `fill_extract`. The watcher on the session's outcome waits (`SETTLE_WAIT`, 30 s) until the start op is answered before it sends `ScoutEnded` or `PlannerEnded`, so an end never precedes its start.
- **Bug fixed from M9.8 and M9.11: the run directory was doubled.** `fill_extract` built its paths as `runs_dir(run.data_dir).join(id)` and `profile::repo_dir(run.data_dir, …)`, where `run.data_dir` is already the run's own directory, so no extract was ever filled. `context_reads` (M9.11) had the same error. Both now use the run's directory and `run.repo_dir`. `start_planner_op_runs_a_planner_and_sends_planner_ended` checks the filled extract. M9.11's read-test rig (`orch_read_rig.rs`) now makes its orchestrator record dormant (`live = false`): its window 1 does not exist, and a live record over a missing window is now reported as exited, which changed the digest those tests assert.
- **Decision 26: the plan path builds a planned run.** `planned()` is removed. `build_plan` takes a `Shape` (`PlanFile`, `Fast`, `Planned`), and `start_planned` builds an empty-task plan with the orchestrator record, sends `EventKind::Start` and answers `Triaged { run_id: Some(..) }` with `planned_message`. `StartGoal` passes `orchestrator` and `yes` through. `triage::refused_message` is deleted, and M8b's refusal tests in `cli/tests/run_e2e_adapt.rs` now assert the planned run.
- **Decision 9: project settings.** The planned start resolves the orchestrator from the run's limits before the reach checks, so its runtime is checked. `run promote` runs `promote_refusal` first: the promoted run's reach minus the current run's reach, checked against the settings the start trusted. The refusal text is mine: `promoting would start <who> sessions, and this repository has project settings they would run without asking: …; a run trusts only what its start checked, so review them and start a new run with --trust-project`. In practice it refuses only when the orchestrator's runtime is one the roster does not reach.
- **Test placement.** `resolve_target_op_runs_git_off_the_worker_threads` and `project_settings_check_covers_the_orchestrator` are in-crate (`driver/orch_ops_tests.rs`, `driver/adapt_goal_tests.rs`): they call the op and the build directly. `promote_repeats_the_project_settings_check` sits beside it and also sends `run promote` through `RunService::promote`.
- **Shared helpers, early.** `ORCH_WAIT` and a first `cli/tests/support/run_orch.rs` (planned run from a goal, orchestrator window, a client's typing, what `fake-agent` read) are created here because M9.13's end-to-end tests need them; M9.16 adds the rest. Timing rows are in `docs/timing-budgets.md` ("Recorded, from M9.13").
- **Safety incident: a daemon test launched the real `claude`.** `crates/daemon/tests/server_runs.rs`'s `tagged_rig` used `ManagerConfig::new`, whose `claude_bin` defaults to `claude`. Once the plan path built a planned run, `a_tagged_goal_start_is_triaged_with_its_id` started a real `claude` orchestrator in a PTY, during about five runs of that test binary before I noticed. A process check afterwards found no leaked `claude` of these tests. Every rig in the file now uses `pinned()` (nonexistent `claude`, `codex` and decider paths), and the test asserts the planned run. No other rig that starts a goal leaves the binaries unpinned (`tests/support/mod.rs` and the in-crate rigs either set them or never start a planned run or an orchestrator).
- **Red before green.** The engine tests (`engine/tests/orch_window_events.rs`) and the wake and ops tests failed to compile or failed before the code. Mutations on the finished code, restored from copies, each turned the named test red:
  - the quiet check dropped: `wake_is_delivered_only_when_idle_and_quiet` (the paste came under 1 s);
  - `Attention` accepted: `wake_is_not_delivered_on_attention`;
  - no replacement (`or_insert`): `a_newer_wake_replaces_an_undelivered_one` (two pastes);
  - no exit report: `orchestrator_exit_adds_the_attention_line_and_suspends_wakes`;
  - no run-live flag on create, and none cleared at the end: `create_orchestrator_sets_the_run_live_flag_and_a_terminal_run_clears_it` (each at its own assertion);
  - none on restore: `restore_sets_the_run_live_flag_for_a_non_terminal_run`;
  - the planner's first turn not filled: `start_planner_op_runs_a_planner_and_sends_planner_ended`;
  - `run promote` without the settings check: `promote_repeats_the_project_settings_check`.

### M9.13 review fixes

One commit, `fix(daemon): pin every test rig's agents, refresh a restarted orchestrator's OTLP endpoint, and steady its exit`. Each test was written first and seen red for the reason given; sources for the mutations were restored from copies.

0. **Every test rig pins the agent programs.** `ManagerConfig::for_tests(socket, shell)` is `new` with `claude_bin`, `codex_bin` and `decider_bin` set to paths under `/nonexistent/anthrex-test/`. Every test that built a `ManagerConfig` (37 calls in 26 files, across the daemon, the TUI and the CLI's tests) now starts from it. `ManagerConfig::new`'s defaults and all program selection are unchanged. A rig that needs a stand-in still sets it afterwards, and `decider/tests_context.rs` clears `decider_bin` to test the mode's own command. `crates/daemon/tests/test_rigs_pin_agents.rs` fails if any `.rs` file under `crates/` other than `manager/config.rs` calls `ManagerConfig::new(`; it was red with all 26 files listed. The CLI's `TestDaemon` now sets `ANTHREX_DECIDER_BIN` to a nonexistent path instead of inheriting it (a test may still override it in `configure`).
1. **Correction to decision 14a ("a restart re-passes the same `RoleLaunch.env`").** The receiver binds a new port with each daemon (`otlp_port = 0`), so a resumed orchestrator was unmetered and would have sent its bearer token to whatever held the old port. Before `RestartOrchestrator`, the driver now sets the role's OTLP variables again: the `otlp.addr` of now, with the same persisted token. When no receiver is up, it removes them. A run created while the receiver was down is metered after a restart that finds it up. The manager's `update_role_env` replaces `Entry.role.env` and the persisted record under the lock, with no I/O; the address file is read on `spawn_blocking` first. A Codex orchestrator keeps none. Tests:
   - `a_restarted_orchestrator_names_the_receiver_that_is_up_now` (e2e; red: the persisted role still named the first port after `run resume`);
   - `a_restart_refreshes_the_otlp_variables` (unit: stale replaced, missing added, none kept without a receiver; it did not compile before).
2. **`run promote` honours the run's `--trust-project`.** `Run.trust_project` (`#[serde(default)]`, false for a run from before) records the start's flag. `promote_refusal` passes the settings of a run that started with it. The refusal now says `this run started without --trust-project, so review them and start a new run with it`. Tests:
   - `promote_repeats_the_project_settings_check` gains the trusted case (red: refused);
   - `a_run_keeps_whether_it_started_with_trust_project` covers persistence and the old-run default.

   The same gap in the edit path's refusal (`runtime_refusals`, M8a's ruling T22-I1b) is in the follow-ups file.
3. **A restart is not an exit.** The driver now treats an orchestrator window as exited only in two cases: it is gone from the list, or it has been `Exited` for `EXIT_CONFIRM` (1 s, two ticks) and is not restarting (`WindowManager::is_restarting`). A new `OrchestratorRecord.launches` counter (`#[serde(default)]`) goes up with each successful launch and restart. `OrchEvent::OrchestratorWindow` carries the count the driver saw, and the engine drops a report made before the last restart. Tests:
   - `a_manual_restart_of_the_orchestrator_is_not_an_exit` (e2e: after `anthrex restart <n>`, no attention line, no log line, still live). It turned red only with both driver guards removed; either guard alone keeps it green.
   - `a_stale_exit_from_before_the_restart_changes_nothing` (engine; red with the `launches` check removed).
5. **Recorded: the plan path's decision 9 refusal comes after the triage call.** `make_planned` and the reach checks run once triage has answered `plan`, so a refused planned start still spends one decider call. It cannot come earlier: the orchestrator's runtime is known only once the start takes the plan path.

#### Re-review fixes

One commit, `fix(daemon): every restart of a restored orchestrator names this daemon's receiver, and every anthrex a test starts pins its agents`.

1. **Every restart path launches with the current OTLP values.** The review's repro was a client's `anthrex restart <n>` of an orchestrator restored after a daemon restart. `WindowManager::restart` read the restored `entry.role`, so the process got the last daemon's endpoint. Now `lifecycle::run` calls `RunService::refresh_orchestrator_otlp` once the receiver is bound and before `serve` takes a client. It passes the receiver's address from memory (`OtlpServer.addr`; `None` when metering is off or did not bind), so there is no file I/O, and at that point no client can restart anything. Every orchestrator window the manager restored gets its role's OTLP variables set for that address with its run's token, or removed. It does this under the engine's lock, then the manager's, one at a time. The refresh in the `RestartOrchestrator` op stays, for windows made later. It could not happen in the driver's restore: that runs before the receiver binds, and the `otlp.addr` found then may be the last daemon's.
   - Test: `a_manual_restart_after_a_daemon_restart_names_the_receiver_that_is_up_now` (e2e). After the daemon restart, its `claude` is a wrapper that writes the endpoint from its real environment, then execs `fake-agent`.
   - Red: the wrapper saw the old port (`63001` while the receiver was on `63003`).
2. **Every `anthrex` a CLI test starts pins the three agent variables.**
   - `support::pin_agents` sets `ANTHREX_CLAUDE_BIN`, `ANTHREX_CODEX_BIN` and `ANTHREX_DECIDER_BIN` to nonexistent paths. `isolated_command` applies it (`daemon_spawn_stderr.rs` and `codex_version.rs`'s daemons can no longer run the real `codex --version` probe). `filter_run.rs` and `filter_hook.rs` use it too. `mcp_cli.rs`, which has no `support` module, pins them in its own `anthrex()`. A test that needs a stand-in still sets its own value afterwards.
   - The guard gains `every_test_that_starts_anthrex_or_reads_variables_pins_the_agents`:
     - a test source that calls `ManagerConfig::from_vars(` must name the claude and codex variables;
     - one that builds an `anthrex` command (`Command::new(ANTHREX)`, `CARGO_BIN_EXE_anthrex`) must name all three, or use `pin_agents(`;
     - a line that starts a daemon (`"daemon", "start"`) must go through `isolated_command(` or the run harness's `self.command(`.
   - Red: it flagged `filter_run.rs`, `filter_hook.rs` (no decider) and `mcp_cli.rs` (none).
   - **Its limits:** the checks read source text a file at a time. A file that names the variables once passes even if one of its commands does not set them. A command built by a helper in another file is judged by that file. A daemon started by a client command (`ensure_daemon`) is covered only because `isolated_command` pins, and the text check cannot see that.
   - Production program selection is unchanged.
3. **One driver test per exit guard** (`driver/wake_exit_tests.rs`, a real shell window killed by the test, the engine's channel read directly):
   - `an_exit_is_reported_only_once_it_has_lasted`: not reported at first sight, reported after `EXIT_CONFIRM`.
   - `an_exit_during_a_restart_is_never_reported`: with `restarting` held (a test-only `WindowManager::hold_restarting`), never reported past `EXIT_CONFIRM`; after release, the count starts again.
   - Mutations: `EXIT_CONFIRM = 0` turns both red. Removing the `is_restarting` check turns the second red and leaves the first green.
4. **Recorded:** a promote let through by `trust_project` does not add the newly reached files to `Run.trusted_project`, so a later `run edit` refuses them. Added to the existing follow-ups entry.

### Task M9.13a (Orchestrator-to-worker messages, refresh and worker notes)

Two commits: `refactor(daemon): move the plan edits' task-state predicates into their own file` (`run/edits_state.rs`, so `edits.rs` had room), then `feat(daemon): deliver worker messages, refresh task branches and record notes`. Each new test was seen red for its reason by mutating the source and restoring it from a copy; the new test files did not compile against the code before.

**Choices the brief left open**

- **`task_note` joins the early hold** (`early::holds_call`). Test: `a_task_note_before_the_window_is_recorded_after_binding`, red with the entry removed.
- `WORKER_MCP_TOOLS` has three entries. Its pins in `headless/argv_tests.rs` and `engine/tests/dispatch.rs` are updated, plus a new test, `worker_spec_allows_task_note`.
- `PAUSE_RELEASED` (`orch/contract.rs`) is new text: `[anthrex] The run was resumed. You were asked to stop and wait; continue your task now.` It is queued for every paused task when the `resume` plan edit releases it. `anthrex run resume` after a daemon restart releases nothing, so a pause survives a restart (decision 42c; corrected in the review fixes below).
- `stop_and_wait` to a task that is not `working` is refused as `task <id> is <label>; stop_and_wait needs a working task`. A second `stop_and_wait` to a paused task is refused as `task <id> is already paused(message)`.
- An unknown id in a message's task list is refused per recipient (`no such task <id>`). The other recipients still get it. The reply names every refusal as `not delivered: <reason>`, and `edit_plan`'s JSON has `delivered` and `refused: [{task, reason}]`.
- A failed refresh sets `error` on its accepted `plan_edits` entry (`edit_log::set_error`). Alongside that it writes the history line `refresh skipped: <why>` and the wake note `refresh of <t> skipped: <why>`.
- `refresh_clean(n, list)` gets `n` from `git rev-list --count`, every merged commit; the list stays capped at 20 (corrected in the review fixes below, item 6).
- Worker notes go in the task report's existing `Notes:` list as `(<kind>, from the worker)`. Messages get their own `Messages:` section, with `not delivered yet` on undelivered ones.
- Discovery and risk notes show in the snapshot's attention list: the last three, as `<t> noted a <kind>: <first 80 chars>`. They do not show in the digest's attention, which would have changed the digest fixture; the digest gets them through the wake note. A paused task shows in the attention list only after 10 minutes (`PAUSED_ATTENTION_SECS`).

**Deviations from the brief's file list and interfaces**

- The refresh's result routing is in `engine/results.rs`, ahead of the hand-back arms, not in `engine/mod.rs`. That file is at 600 lines, and `results.rs` is where every op result is routed.
- `TaskMessage.outbox: Option<u64>` (`#[serde(default)]`) links a recorded message to its outbox entry. A fresh session's first turn carries every message in its prompt, so the linked outbox copies are dropped (`worker_messages::launched`) and never sent twice. A resume that fails keeps them out as well. Tested inside `rung_2…`: red with the `carries` filter removed.
- `MessageOutcome` has `queued` and `text` besides `delivered` and `refused`. `EditConsequence::Recipients(MessageOutcome)` carries it out of `apply_edits`.
- The paused-task refusal for `override` is in `engine/gates.rs`, which is not in the brief's list. The refusal for `retry` is in `engine/requests.rs`.
- `OpKind::HandBack.list_merged` and `OpResult::HandedBack.merged` both use `#[serde(default)]`, so a journal from before replays. Reconcile replays a refresh (`refresh_is_replayed_by_reconcile`, in `tests/run_journal/git_handback.rs`) at the reconcile level. It does not go through the abort hook, because a refresh has no abort point of its own; it is M8a's hand-back.
- **The clean-tree check at acceptance** is `driver/refresh.rs`. It runs only for a lone `[Refresh]` of a working or paused task, from `run edit` and from `edit_plan`. The worker's HEAD object lives in the task's private object directory, so a plain `git status` in the task worktree said `bad object HEAD`. The check therefore runs `sync_in` first, behind the run's `GitQueue::write`, on `spawn_blocking`, inside `DONE_CHECK_GIT_TIMEOUT`, never under the lock. The boundary check in `git::hand_back_listing` (`tracked_changes`) does the same. Only tracked changes count, not untracked files. Its tests are in-crate (`driver/refresh_tests.rs`, through the real socket rig) rather than a new integration binary.
- The environment check for the refresh's git calls is in `tests/run_git_env.rs`: `hand_back_listing` is added to that binary's one test. That binary is the only one allowed to change the process environment.
- The brief's `worker_messages.rs` tests are split in two. `engine/tests/worker_messages.rs` holds delivery, recipients and limits; `engine/tests/worker_messages_pause.rs` holds the pause and the notes. The M9.12-era placeholder tests (`edits_tests_placeholders.rs`) are gone: the placeholder in `dispatch_edits.rs` became `message_or_refresh_beside_another_edit_leaves_the_run_unchanged`.

**Open, recorded for review**

- **What counts as the task's own commits.** The fallback's commit count and `task_done`'s zero-commit check subtract `refresh_merges.len()`. A worker that resets its branch below a refresh merge would be under-counted. This is an edge case; the net diff (`verify_done`, `measure_diff`) is unaffected, because it measures against the run head.
- **A conflicted refresh records no merge in `refresh_merges`.** The worker makes that merge commit itself, so it counts as the worker's own. `Task.conflicts` is unchanged, because only the merge queue counts a conflict.

### M9.13a review fixes

One commit, `fix(daemon): keep research and review tasks out of worker messages, leave refreshed run work out of a task's commits and result, and free an ended run's orchestrator at once`. Each test was written first and seen red for the reason given; the sources for the mutations were restored from copies.

1. **Research and review tasks take no worker message and no refresh.** Before, a message to a working research task was reported as delivered, but no worker round ever took it. A `stop_and_wait` then paused the task for good, with its scout's `submit_scout_report` refused. A pending research task's message was recorded, but `research_prompt` never shows it.
   - The choice: refuse. Delivering through `kinds::mailbox_task` would reach a scout or reviewer whose contract has no message or pause rule, and a review task already finishes on its first verdict.
   - `takes` refuses a research or review task in any state with `task <id> is a <kind> task; a message would not reach a worker`.
   - `running` never names one (`edits_state::is_reader`).
   - A refresh of one is refused with `task <id> is a <kind> task; refresh needs a code or docs task`, since such a task has no branch.
   - Tests (`engine/tests/worker_messages_review.rs`), each red with its check removed:
     - `a_research_task_takes_no_message_in_any_state` is the reviewer's repro: `info`, `stop_and_wait`, `running` and refresh.
     - `a_pending_research_or_review_task_records_no_message`.
2. **`task_result`'s git reads leave out refreshed run work.** `git::task_summary_excluding` is `task_summary` given the task's recorded refresh merges.
   - The log keeps decision 18's command and adds `^<M^2>` for each merge `M`, then drops the merges themselves.
   - The diffstat is `<M^2>...<branch>` for the newest recorded merge the branch still has. Without one, it is `start...branch`, as before.
   - A merge `root` does not have is ignored. `task_summary` itself is unchanged.
   - Real-git test (`run_refresh.rs`): `task_result_leaves_out_refreshed_run_work`. The unfixed read lists the other file; the fixed one lists only the task's commits and files, including work made after the refresh. Red with the merge list ignored.
3. **The commit counts leave the refresh merges out in git.** They no longer subtract a number.
   - `CountCommits` and `VerifyDone` carry `not_own` (`#[serde(default)]`): the task's `refresh_merges`, each once (`worker_messages::not_own`).
   - `git::count_commits_excluding` and `git::verify_done_excluding` count `HEAD ^start ^run_head` less the commits named there that the branch still has. A worker that rebased onto the run head, or reset below the merge, keeps its real commit.
   - `own_commits` is gone. `refreshed` records a merge only once, and treats one it already has as `refresh: nothing new`.
   - Reconcile claims a refresh's clean merge only when `merge-tree` of its first parent and the run head is clean and gives exactly its tree. So a merge the worker committed after a conflicted refresh is never recorded as the refresh's; the refresh runs again and finds the branch up to date.
   - Real-git tests:
     - `the_counts_leave_a_recorded_refresh_merge_out`: a merge alone is 0, a merge named twice is left out once. Red with either filter removed.
     - `a_rebase_or_reset_after_a_refresh_keeps_the_real_commit`: red under the old subtract-the-count rule.
     - `a_merge_the_worker_committed_is_not_replayed_as_a_refresh` (`run_journal/git_handback.rs`): red with the tree check removed.
   - Engine tests:
     - `the_counts_leave_each_recorded_refresh_merge_out_once`: red with the de-duplication removed.
     - `a_replayed_refresh_merge_is_not_recorded_twice`: red with the `contains` check removed.
     - `a_refresh_merge_alone_is_not_work` (`refresh.rs`) is rewritten for counts that come from git.
4. **`run override` counts the same way.** Its `CountCommits` carries `not_own`, so a task whose only commit is a refresh merge counts 0 and is refused. Test: `override_counts_without_the_refresh_merges`, red with `not_own` left empty.
5. **Message and note texts are one line through `messages::one_line`.** This is the wake notes' rule (M9.9 I1), moved from `engine/wake.rs` to `run/messages.rs`. Every control character, including U+0085 and ESC, becomes a space, as do U+2028 and U+2029. A `task_note`'s text is folded when it is stored, so the report, the history, the prompts and the wake-up all carry one line. Test: `message_and_note_texts_are_one_line`, red with either fold reverted.
6. **A refresh's `n` is every merged commit.** `git::merged_log` also runs `git rev-list --count <onto>..<run head>` and returns `MergedCommits { lines, total }`. `OpResult::HandedBack` gains `merged_total` (`#[serde(default)]`), and reconcile fills it too. Tests:
   - `a_refresh_names_the_count_of_every_merged_commit` (engine: 57 commits, 20 listed).
   - `a_refresh_counts_every_merged_commit_and_lists_the_newest` (real git: 25 commits).
   - Each is red with the total replaced by the list's length.
7. **A paused task is told to wait.** For a `paused(message)` task, `refresh_clean_paused` and `refresh_conflict_paused` (`orch/contract.rs`) replace "Rebuild before you continue." and "Resolve them, commit, and continue." with texts that end `then wait for the next message: you were asked to stop and wait.`. Test: `a_paused_task_is_told_to_wait_after_a_refresh`, clean and conflicted, red with the paused check removed. The implementation note on `PAUSE_RELEASED` above is corrected: only the `resume` plan edit releases paused tasks.
8. **An ended run frees its orchestrator on the step that ended it.** The flag used to clear only in `publish`, after the step's saves. `run list` answers from engine state, so it could show the run `discarded` while the kill was still refused.
   - `RunService::release_ended_orchestrators` now runs under the engine lock right after each step, as `metered.refresh_live` does. It clears the manager's flag for every terminal run's orchestrator window, taking the manager's lock inside the engine's, the order `refresh_orchestrator_otlp` already uses; neither does I/O.
   - Test: `driver/live_flag_tests.rs::an_ended_run_frees_its_orchestrator_before_its_saves_finish`. It uses a real manager and a real PTY window whose `claude` is a sleeping stand-in; the test kills that window itself, and no stand-in was left running.
   - Red: without the call, the flag was still set after 10 s with the run's saves stalled.
   - The e2e `create_orchestrator_sets_the_run_live_flag_and_a_terminal_run_clears_it` passed 12 times alone and 10 times with its whole binary, with no retry.

#### Re-review fixes

One commit, `fix(daemon): record every refresh's run head so no merged run work counts as the task's, fold repository text in refresh texts, and free only an ended run's own window`. Each test was written first and seen red for the reason given; the sources for the mutations were restored from copies.

1–3. **Every refresh records the run head it merged.**
   - The field is `TaskOrch.refresh_targets` (`#[serde(default)]` through the struct's default, so older runs load with it empty).
   - Clean, conflicted or up to date, the target is recorded once, in `refreshed`, which now gets the op's `run_head`.
   - `git::RefreshedIn { merges, targets }` carries these records, and the git reads use them:
     - `task_summary_excluding`: the log adds `^<target>` for each recorded target `root` has, and still drops the clean merges themselves. The diffstat's base is the newest recorded target that is an ancestor of the branch, else `start`. So a conflicted refresh, which records no merge, and a rebase after a refresh both leave the run work out.
     - `count_commits_excluding` and `verify_done_excluding` (through `own_commits`): `<head> ^start ^run_head ^<each known target>`, less the recorded clean merges. So a run head that `resume --rebaseline` rewound cannot make merged run work count.
   - `CountCommits` and `VerifyDone` gain `not_run` (`#[serde(default)]`), the targets each once (`worker_messages::not_run`).
   - **Choice recorded:** the merge commit the worker makes to finish a conflicted refresh counts as the task's own. It holds the conflict resolution, and no run commit counts.
   - Real-git tests (`run_refresh.rs`):
     - `a_conflicted_refresh_leaves_run_work_out_of_task_result` is the reviewer's scenario. With no record, the log lists `run:` commits and `docs/other.md`; with the target, the log is `resolve` and `task: lib`, and the diffstat is `src/lib.rs` only. The count with the run head rewound is 2 (the task commit and its resolving merge). Red with the targets left out of the log.
     - `a_rebase_after_a_refresh_leaves_run_work_out_of_the_diffstat`: red with the diff base left at `start`.
     - `a_rewound_run_head_does_not_make_refreshed_run_work_count`: merges alone count 1, with the target 0, for both counts. Red with the targets skipped.
   - `task_result_leaves_out_refreshed_run_work` now records the target as the engine does.
   - Engine test: `a_refresh_records_the_run_head_it_merged_clean_or_conflicted` checks the record, once, on the fallback's count and `task_done`'s check. Red without the record, and red without the de-duplication.
   - **Open:** `verify_done`'s spill diff and `diff_so_far` still diff from the current run head (`<run_head>...<head>`). A run head rewound below a refresh's target would show the merged run files there. That is M8a's rebaseline path, which this re-review did not ask to change; it is added to the follow-ups file.
4. **Repository text in the refresh texts is one line.** `refresh_clean` and `refresh_clean_paused` fold each commit subject, and `refresh_conflict` and `refresh_conflict_paused` fold the file names, through `messages::one_line`. Test: `refresh_texts_fold_file_names_and_subjects`, with the reviewer's file name and subject plus U+2028, U+2029 and U+0085. No control character or separator is left, and only the text's own two `[anthrex]` markers remain. Red with either fold removed.
5. **Only an ended run's own window is freed.** `WindowManager::end_run_window(id, run_id)` clears the flag only while window `id`'s role names that run. Both `release_ended_orchestrators` and `clear_ended_orchestrators` use it, so a window id a restart gave to another run's window keeps that run's flag. Test: `an_ended_run_leaves_another_runs_window_with_its_old_id_alone`, with real windows for both runs; the ended run's own window is freed and the other is not. Red when the owner is not checked.
6. **The lock-order note is reworded** on `release_ended_orchestrators`. Item 8 of the first review is the first place that takes the manager's lock inside the engine's; `refresh_orchestrator_otlp` takes them one at a time. It is safe because no path takes them the other way: the manager never calls into the run service, and its lock is held only inside its own methods. Neither lock is held across I/O or an `await` there; `end_run_window` changes one flag.

### Task M9.13b (Routing history for orchestrators, planners, scouts and deciders)

One commit, `feat(daemon): record routing choices and outcomes for non-task agents`. No move commit was needed: the engine's record logic is a new child module of `engine/history.rs`, so `engine/mod.rs` (600) and `driver/ops.rs` (598) are unchanged.

**Choices the brief left open**

- **Deciders and pre-run triage.** M9.2's ruling 2 already added `AgentRole::Decider`, so no placeholder remains. Every decider record, pre-run triage included, has `role: decider`. Its `trigger` and `input.question_kind` are the request's kind (`triage`, `size_check`, `check_summary`, `blocked_reason`). Pre-run triage has `run_id: None`, even when a run is then created. Old lines load unchanged; nothing on the wire changed.
- **Record ids** are `<run id>/<role>/<session id>` (`roles::record_id`). Pre-run triage's is `triage/<unix nanos>/<n>`, where `<n>` is a per-daemon counter. A task's record is `<run>/<task>` (one `/`), so no record can take another's id.
- **Session ids:**
  - The orchestrator's is the n-th orchestrator session dispatched in the run, not `RunRef.session`. A relaunch after a failed launch keeps `RunRef.session` at 1 but is a new session here. A launch, a relaunch, `run resume`'s restart and the user's own `anthrex restart` after an exit each count.
  - A sub-planner's is `<epic>/<session>`.
  - A run scout's is its scout id.
  - A run-bound decider's is its `Decide` op id. A `Decide` lost in a restart is queued again and asked under a new op, so it gets a new record.
- **No session, no record.** Decider fallbacks that start no session write no record: deciders off, no reader slot, unreadable evidence, and triage's failed `ls-files`. Decision 43 records sessions, and a route that was never dispatched would read as one.
- **Outcomes:**
  - **Orchestrator:**
    - a failed launch or restart is `failed`;
    - a window exit while the run goes on was `completed`; *corrected by the review fixes (M-1)*;
    - live when the run ended is `completed`, `live until the run ended; …`;
    - a run that ended before its window started is `interrupted`.
    - No result names the run's outcome.
  - **Sub-planner** (a session stopped by `run cancel` or `finish`: see the review fixes, I-1):
    - `completed` (`epic accepted`) only when its `submit_epic` was accepted. The acceptance is noted on the open record, so a re-plan queued before the end event cannot lose it.
    - Otherwise `failed`, with the machine's reason or `ended without an accepted epic`.
    - Rejected submissions are counted as `; <n> submissions rejected`.
  - **Run scout:** `completed` (`reported`), or `failed` with its reason.
  - **Decider:** `completed` (`answered`), or `fallback` with its reason.
  - **Restore:** every open record is `interrupted`, `the daemon restarted during this session`, and keeps any status it had (for example `epic accepted; …`).
- **Triggers:**
  - orchestrator: `start`, `promote`, `retry` (a later launch) and `restart`;
  - planner: `start` and `replan`;
  - scout: `start`.
- **Candidate snapshots:**
  - **Orchestrator:** decision 6's `Resolved`, kept in the new `OrchestratorRecord.routing: RoleSnapshot { source, candidates }` (`#[serde(default)]`). Every launch and restart record is copied from it. A record from before this task has an empty source, recorded as `unrecorded`.
  - **Sub-planner and scout:** `roles::ladder_candidates`, M8b's strength-ladder order: the runtime's entries at or above the strength, lowest first, then the peer's.
  - **Decider:** ~~its one configured route~~ *corrected by the review fixes (I-2):* the mode's runtime's ladder from the scout service's roster, no peer. Milestone 9.5's role lists replace all of these.
- **A run scout's record is made by the driver**, as a decider's is. The scout service routes every scout from the daemon's `[orchestrator.scouts]` (`ScoutContext`), which the run does not freeze. So `start_scout` builds the record from that same context, under the engine lock (pure), and sends `OrchEvent::RoleRoute` before `ScoutService::start`. The engine finishes it on `ScoutEnded` or a failed `StartScout`. Its source is a new value, `scout_config` (the proto doc comment lists it).
- **Idempotence:** a finished record is never finished again. Each finish emits one `AppendHistory`, reconciled by record id. Pre-run triage uses the new `history_io::append_once` (`contains_record`, then `append_line`), on `spawn_blocking`, awaited before the goal's reply; a write failure is only logged. A run whose history is off keeps its records and appends none.

**Deviations from the brief's file list and interfaces**

- `make_planned(run, triage, resolved: Resolved, yes, installed)` takes decision 6's whole resolution, not its route. `promote.rs` sets `routing` the same way.
- `OrchEvent::RoleRoute { run_id, decision: Box<RoleRoutingDecision> }`: boxed, like the other large payloads.
- The record logic is `engine/role_routes.rs`, a `#[path]` child of `engine/history.rs`. The orchestrator's capture points are in `engine/orch_window.rs`, where its launch, restart and window events live, not in `engine/orch.rs`. `orch_window::{launch, launched, restarted, window_seen}` and `planners::ended` / `run_scouts::ended` gain `now` or `fx` parameters.
- Run-bound deciders: `driver/effects.rs` runs each op through `RunService::run_op` (`driver/adapt.rs`), which sends a `Decide` to `decide_as` with its op id. `driver/ops.rs` keeps its unrecorded `decide` arm, so the file is unchanged.
- `run/model.rs`, `run/reconcile/mod.rs`, and the `RoleRoute` arms of `history_io.rs`, `engine/history.rs` and `stats.rs` needed no change: M9.2 had added them.
- `pre_run_triage_writes_a_record_even_when_no_run_is_created` is in `tests/server_runs.rs`, not `tests/history_io.rs`, because it needs that file's daemon rig. `rig_with` was added to that rig with a decider mode and the shipped CLI caps.
  - The decider program is the pinned nonexistent path, so triage falls back.
  - A Codex orchestrator in a repository that tracks `.codex/config.toml` is then refused after triage.
  - The record is in `<data>/repos/…/history.jsonl`, and the repository stays clean.
- `scout_retry_gets_a_new_session_id`: decision 20 uses a scout id once per run, so a retry is a new scout. The test re-asks under a new id and checks distinct session and record ids.

**Red before green.** Only the pure `roles.rs` tests were written against no code: they do not compile without the module. The reducer and triage tests were written with the code and made red by mutation, each from a `cp` backup restored afterwards:
- `role_routes::open` as a no-op: all 9 reducer tests red.
- `interrupt_open` and the terminal pass disabled: 4 red (`orchestrator_restart_…`, `a_run_never_attributes_…`, `scout_failed_on_restore_…`, `restore_marks_every_open_record_…`).
- The `record_triage` call removed: `pre_run_triage_…` red (no line).

Some tests passed at once, because the behaviour came from M9.2 or M8b:
- `stats_ignores_role_route_lines`: M9.2's ignore arm;
- `version_1_history_and_an_old_run_json_still_load`: serde defaults;
- the reconcile half of `role_route_append_is_idempotent_by_record_id`: M8b's `contains_record` row.

**Open**

- The driver halves (the `RoleRoute` and `RoleRouteEnded` sends of `decide_as` and `start_scout`) have no driver-level test here. The reducer tests feed the events those paths build with the same pure builders. M9.16 and M9.17's end-to-end tests cover them with real sessions.

### M9.13b review fixes

One commit, `fix(daemon): …`, on top of `1b760c9`. Each test was written first and seen red for the reason given (a runtime failure on the old code, or a mutation of the new code restored from a `cp` copy where the old code had no seam).

- **I-1. Sessions anthrex stopped are `interrupted`.** `planners::halt_all` (`run cancel`, the `finish` edit) now finishes the records of the sessions it halts (*narrowed by the re-review, item 1*) `interrupted`, `stopped when the run ended`. The session's later end, `failed` with the halt's reason, finds the record finished and changes nothing. `a_run_never_attributes_its_outcome_to_one_role` now also cancels a running run with a live planner and scout. Red on the old code: both recorded `failed`, `the run was cancelled`.
- **I-2. A decider's full candidate snapshot.** Candidates come from `roles::decider_candidates(roster, route, deciders.strength)`:
  - the roster is the scout service's context (`adaptation.scouts.context().roster`);
  - it takes the mode's runtime's entries at or above the strength, lowest first, with no peer runtime;
  - the chosen one is marked, and the others get the neutral reasons.
  - It is used for pre-run triage and for run-bound deciders. The deviation above is corrected.
  - Tests: `decider_candidates_are_the_modes_runtime_ladder_only` (pure, both runtimes). The triage test in `tests/server_runs.rs` now checks the ordered ladder and each reason; red on the old code, which had one candidate.
- **M-1. What `completed` means for the orchestrator.** It had submitted its plan (what the session is for) when its window exited while the run went on, or it was live until the run ended. An exit before it submitted a plan is `failed`, `its window exited before it submitted a plan`. An exit after the plan is `completed`, `its window exited after it submitted the plan, before the run ended`. The Interfaces' outcome set has no "exited early", so the result says it.
  - Test: `engine/tests/role_history_ends.rs::an_orchestrator_exit_is_failed_before_its_plan_and_completed_after`, red on the old code (`completed`).
  - `orchestrator_restart_gets_a_new_session_id` now expects `failed` for its exit before a plan.
- **M-2. Saved before the session starts.** `OrchEvent::RoleRoute` gains `reply: ReplyId`; the engine answers `recorded`, or `unknown run <id>`.
  - The driver's new `RunService::keep_record` sends the event with `ask`, so it waits until the step that keeps the record was saved (`Persist` runs before `Reply`). No lock is held across the wait.
  - `start_scout` and the run-bound decider (`decide_as`) call it before the session starts.
  - Tests in `driver/role_route_tests.rs`, a real engine loop and `run.json` writer with no agent:
    - `a_scout_record_is_saved_before_its_session_starts` is on a one-thread runtime. The scout id is one the service refuses before any await, and `run.json` is read with no await after the op returns.
    - `a_decider_record_is_saved_before_its_call`: `run.json` writes are stalled (`RunWrites::slot`) while the decider is dispatched. The decider is a `/bin/sh` stand-in the test writes. It exits by itself after writing whether `run.json` held its record.
    - Red with `keep_record` mutated back to a plain `send`: `run.json has no …/scout/Bad`, and the stand-in wrote `absent`.
- **M-3. No record when the daemon's deciders are off.** `decide_as` opens a record only when `adaptation.deciders.mode` is not `off`; such a call starts no session. Test: `no_decider_record_when_the_deciders_are_off`, red with the check removed.
- **M-4. Task ids.** `decider_record` takes every task id of the call. `task_id` names the task when there is exactly one. A call about several, a batch size check, is recorded run-level with no task id rather than only the first. `RoleRoutingDecision` has one `task_id`, and the history format is left as M9.2 made it. Test: `a_decider_record_names_its_task_only_when_it_has_one` (red on the old code, which took the first; it no longer compiled against the old signature).
- **M-5.** Pinning tests, which passed at once:
  - `planner_and_scout_ladders_include_the_peer_runtime`;
  - `each_unchosen_candidate_gets_the_reason_for_its_place`: before the chosen one, `not in the configured list`; after it, `an earlier candidate was taken`; a caller's own reason is kept; an appended chosen route comes last.
  - (c), the scout's `RoleRoute` from the driver, is M-2's scout test.
- **M-6.** Pre-run triage's `append_once` runs under a 10 s `tokio::time::timeout` (`TRIAGE_WRITE_TIMEOUT`). On a timeout it logs and the goal goes on. No test: a stalled file system is not reproducible without a fault-injection seam the brief does not ask for.
- The proto doc comment for `RoleRoutingDecision.source` lists `scout_config` (and the triage record id format), confirmed.

#### Re-review fixes

One commit, `fix(daemon): …`, on top of `6a7c91b`. Each test was written first and seen red on `6a7c91b` for the reason given.

1. **Only the sessions `halt_all` halts are `interrupted`.** `role_routes::sessions_stopped`, which closed every open planner and scout record, is replaced by `session_stopped(run, (role, session))`. `planners::halt_all` calls it for a planning epic's latest session, and `run_scouts::halt_all` for each running scout: the sessions the halt stops. Any other open record keeps its own end:
   - an accepted planner whose process is still ending;
   - a planner the engine stopped at `max_rejections`.

   Tests in `engine/tests/role_history_ends.rs`, both red on `6a7c91b` (the record was `interrupted`):
   - `cancel_leaves_an_accepted_planner_to_its_own_end`: the record stays open through `run cancel`, then ends `completed`, `epic accepted`;
   - `cancel_leaves_a_planner_failed_at_max_rejections_to_its_own_end`: it ends `failed` with its own rejection reason.
2. **A scout stopped before its record is kept is not started.** The engine's `RoleRoute` (`role_routes::keep`) refuses a run scout's record when that scout is no longer running: `scout <id> was stopped when the run ended`, or `… is not running`. `start_scout` then returns `the scout was not started: …` without starting a session.
   - **Choice: nothing is recorded.** No session was dispatched, as with M-3's deciders off. A record would name a route that never ran.
   - Test: `driver/role_route_tests.rs::a_scout_stopped_before_its_record_is_kept_is_not_started`. Red on `6a7c91b`: the service was reached (`invalid scout id "Bad"`), and a record was kept.
3. **A record that could not be saved starts no session.** `effects::RunWrites.record_replies` maps each record reply's id to its run; `keep_record` registers it inside `ask`. When `execute` applies that reply, it becomes `Err("its record could not be saved")` if the same step's `run.json` save failed.
   - No new stall: `execute` already awaits the save before any reply of its step, and only the reply's content changes.
   - `start_scout` then fails its op, and the engine finishes the kept record `failed` through `StartScout`'s failure.
   - `decide_as` runs no decider: it answers the fallback (`the decider could not start: …`) and ends the record `failed`, `not started: its record could not be saved`.
   - Test: `a_record_that_could_not_be_saved_starts_no_session`. The run's data directory is put under a regular file; the scout op fails with the reason, the decider stand-in never runs (no mark file), and the decider record is `failed`. Red on `6a7c91b`: the scout reached the service.
4. **The orchestrator's result describes the run's plan, not the session.** `plan_submitted` is the run's, so the results are:
   - `its window exited before the run's plan was submitted`;
   - `its window exited after the run's plan had been submitted, before the run ended`;
   - `live until the run ended; the run's plan had been submitted` or `…; no plan had been submitted`.

   `an_orchestrator_exit_is_failed_before_its_plan_and_completed_after` and `a_run_never_attributes_its_outcome_to_one_role` now expect these texts; both were red on `6a7c91b`'s wording.

### Task M9.13c (`packed-refs.lock` under the worker sandbox)

One commit on top of `b7d08b4`. It closes the task as "no git change", following M9.1 check 11 and controller ruling 1.

1. **No git config change, and the sandbox is not widened.**
   - M9.1 check 11 found that `git commit` itself takes `packed-refs.lock` in the checkout's own git dir. After it moves `HEAD`, it deletes the merge-state refs, and the files backend locks `packed-refs` for any ref deletion. None of `gc.auto=0`, `maintenance.auto=false` or `maintenance.pack-refs.enabled=false` stops it.
   - So the brief's **Change** (add a key to `run/git/checkout.rs`'s config) does not apply. `checkout.rs` and `run_git_checkout.rs` are untouched.
   - `role_launch::worker_git_roots` and its test are untouched, and so is `worker_git_dirs`.
2. **Two of the brief's tests are not written.** No config key exists, so there is nothing for them to check:
   - `task_checkout_config_stops_ref_packing`, which would read a key with `git config --get`;
   - `a_worker_commit_under_the_sandbox_prints_no_packed_refs_error`. Under git 2.50.1 that stderr does contain the warning.

   Instead, the pinning test below also asserts that the commit exits 0 despite the warning.
3. **`run_git_sandbox.rs::packed_refs_stays_read_only_to_a_worker`** (pinning; macOS, skipped where `sandbox-exec` is missing).
   - **The profile.** The worker's grant as this file already models it: the task worktree plus `worker_git_dirs(worker_git_roots(..))`, under the file's `profile()`. The daemon has no builder for a worker's seatbelt profile. The agent CLI builds that profile from the daemon's writable roots, and `run/seatbelt.rs` is the confined check's profile, not the worker's. So "the daemon's own profile builder" means the grant functions here.
   - **What it checks.**
     - Writes to `packed-refs` and `packed-refs.lock` are denied, both in the checkout's own git dir and in the common dir.
     - A `git commit` under the grant exits 0 and makes a commit on the old tip.
     - Every stderr line that names `packed-refs` says `Operation not permitted`.
     - None of the four files changes.
   - **Evidence.** It passed at once, as a pinning test should. Its stderr was `error: Unable to create '<tmp>/tasks/t1/git/packed-refs.lock': Operation not permitted`, which reproduces check 11 under the real grant.
   - **Mutation.** With the checkout's git dir added to the writable set, the test fails with `wrote …/tasks/t1/git/packed-refs`. The file was restored from a copy afterwards.
   - `Setup::sandboxed` now calls a new `Setup::sandboxed_output`, which returns the whole output.
4. **The worker contract gains line 12** in `run/contract.rs`. It has 13 lines and ends with the new line:

   ```text
   12. After a commit, git may print Unable to create '.../packed-refs.lock': Operation not permitted. That is expected: the commit succeeded, and it needs no action. Do not try to fix it or change git settings.
   ```

   - The path is written as `...` in ASCII. `contracts_have_no_em_dash_and_survive_toml` still passes.
   - Updated tests: `worker_contract_is_exact` (`WORKER_EXPECTED`), and `contracts_round_trip_through_toml_string`, which now checks 13 lines and the new last sentence.
   - New test: `the_packed_refs_lock_warning_is_expected`.
   - All three were red before the contract change: the count, the exact text, and a missing line 12.
   - Interfaces "Contracts" and decision 41 still describe 12 lines. This note and the M9.5 note supersede them.
5. **Commit subject changed.** The brief's subject is `fix(daemon): keep a sandboxed worker's git from packing refs it cannot write`, but no git behaviour changes. The commit is `docs(daemon): tell a sandboxed worker the packed-refs.lock warning is expected`.
6. **The followups entry** ("From the Claude tool-search fix", "Not fixed: `packed-refs.lock`…") is marked resolved.

### Task M9.14 (CLI)

One commit, `feat(cli): start planned goals, approve holds, message and refresh tasks, and show the orchestrator in run status`. No move commit was needed: `main.rs` is untouched (600), the new command bodies are in `run_cmd/orch.rs` (its unit tests in `run_cmd/orch_tests.rs`), `run_cmd.rs` takes the variants and grows to 552, and `run edit`'s file reading moved into `orch.rs`. No protocol change: every command sends an existing `RunRequest`.

**Red before green.** Both new test binaries were built and run against the unchanged CLI. Every test failed except the two pinning ones (`start_goal_on_the_plan_path_prints_the_planned_message`, `promote_twice_answers_without_a_time`): the new subcommands and flags exited 2 (clap), `approve_without_hold_names_waiting_holds` got the daemon's `run <id> is running`, and the research run's accept asked M8a's merge question with no report line. `status_shows_orchestrator_planners_holds_summary_and_paused_lines` failed on the missing lines. The `orch_tests.rs` unit tests were written with the module they test, so their red was the missing module.

**Deviations and choices**

- **Test placement.** `run_cli.rs` is 480 lines, so the e2e tests are in two new binaries: `crates/cli/tests/run_cli_orch.rs` (`--orchestrator`, the planned start, holds, status JSON, the research accept, promote, `run edit --submit`) and `crates/cli/tests/run_cli_message.rs` (message, refresh, help). Every command goes through the harness (`h.anthrex`, pinned) or `isolated_command`.
- **`status_shows_orchestrator_planners_holds_summary_and_paused_lines` is a pure test** in `run_cmd/status_tests.rs`, with exact lines for planners, holds, summary, `reported`, `paused(message)` and ` (held)`, which no single real run produces at once. Against a real daemon, `status_json_carries_the_new_fields` checks the orchestrator line, the hold line and a held task's ` (held)` on a promoted run, and `run_message_kind_defaults_to_info` checks a `stop_and_wait` task's `paused(message)` row.
- **A refused recipient's line is the daemon's.** `run message` prints the `run edit` reply as the daemon wrote it. That reply already has one `not delivered: <reason>` line per refused recipient (M9.13a), and each reason names its task. So the CLI does not print its own `  <task>: <reason>`; the reply has no structured refusals to build one from.
- **`run message` takes a comma-separated task list** (`t1,t2`, `MessageTarget::Tasks`). With only one task per call, a refusal of one recipient beside a delivery to another could not happen from the CLI. `running` and `stage:<n>` stand alone. A `stage:` that is not a number is refused by the CLI, with `expected stage:<n>, not <to>`. `stage:<n>` is sent, and the daemon refuses it with `stage recipients arrive with milestone 9.1` (D-3).
- **`--orchestrator`** is parsed in the CLI for its runtime only (`claude`, `codex`, optionally `:<model>`; an empty model is the default). A model that is not in the roster is refused by the daemon, in its words: `<runtime>:<model> is not in the roster`. The test pins that refusal too. Both refusals of the flag come before any request, and the `--plan` one comes first.
- **`run approve <run>` without `--hold`** reads the snapshot. On a `running` run with an `awaiting` hold it exits 1 with `run <id> has holds waiting for approval: <ids>; pass --hold <id>` (with the literal placeholder `<id>`). Otherwise it sends M8a's `Approve`, as before. `run reject --hold` conflicts with `--confirm` (a clap error) and asks nothing.
- **`run edit`**: `--file` is optional, and a clap group requires `--file`, `--submit` or both. The reply is the engine's (`applied <n> edit; the plan of run <id> was submitted: it awaits approval`). `run message` and `run refresh` send `submit: false`.
- **Status lines**: the counts are plural-aware, so a hold with one task shows `(1 task)`. A planner's count is the run's tasks naming its epic, shown only when non-zero. An orchestrator with no window yet shows `window -`. The lines come after `report:` and `checks:` and before the task table, in this order: orchestrator, planners, holds, summary.
- **`run accept`** prints `research report: <path>` on stderr before any question, including with `--yes`, like the moved-base listing. It asks the nothing-to-merge question whenever `run_head == base_sha`.
- **Help**: `run message --help` names `info`, `change` and `stop_and_wait` (clap's possible values, each described) and the three recipient forms. `run refresh --help` names the follow-up `anthrex run message --kind change`.
- **Obligations checked**:
  - Decision 6's report line was already written by the M9.7 review fixes (ruling 3), so the CLI has nothing to do for it.
  - M9.13b has no CLI part: routing history is daemon-only.
  - The message-target, one-edit, 4000-character, rate-limit and refresh refusals reach the user verbatim through `print_outcome`. `run_refresh_refuses_uncommitted_work` and the stage test compare the CLI's stderr with the raw request's refusal.

**Tests.**
- Unit, `run_cmd/orch_tests.rs`:
  - `orchestrator_values_parse`
  - `orchestrator_applies_to_goals_only`
  - `message_targets_parse`
  - `message_text_is_the_words_joined_with_one_space`
  - `message_kind_defaults_to_info_and_names_three_kinds`
  - `edit_needs_a_file_or_submit`
  - `hold_flags_parse`
- Unit, `run_cmd/status_tests.rs`: `status_shows_orchestrator_planners_holds_summary_and_paused_lines`.
- E2e: every test the task names, under the names it gives, split across the two files above. Each binary passed three times in a row, with no retry.

### M9.14 review fixes

One commit, `fix(cli): take a message's text verbatim after its recipient, print daemon text without control characters, and split run_cmd and status to their budgets`. Each test was written first and seen red for the reason given.

1. **A message's text is every argument after its recipient.**
   - *The problem.* `run message` used to parse `--kind` anywhere on the line. So `t1 use --kind change next time` sent kind `change` with the text `use next time`, and `-x is broken` exited 2.
   - *First attempt.* `trailing_var_arg` plus `allow_hyphen_values` on the text alone still took a `--kind` that came *first* after the recipient as the flag. `t1 --kind stop_and_wait is not a flag` paused the task.
   - *The fix.* The recipient and the text are now one positional argument: `to_and_text`, with `num_args = 2..`, value names `TO TEXT`, `trailing_var_arg` and `allow_hyphen_values`. The var-arg starts at the recipient. Everything after it is text, verbatim, and only flags before the recipient are the command's.
   - *Help.* `--kind`'s help says to give it before `TO`. A missing text is clap's `TooFewValues`, exit 2.
   - *Tests.* `run_cli_message.rs`'s `run` helper now puts the global `--dir` before `run`. The two tests that gave `--kind` after the recipient now give it before.
   - *New tests and their reds:*
     - `run_message_text_keeps_flag_like_words` (e2e). It covers `use --kind change next time` (kind `info`, words verbatim), `--kind stop_and_wait is not a flag` as the first words (the task stays `working`), and `running -x is broken`. Red at first: `(Some(Change), Some("use next time"))`. Red again under the first attempt: `Blocked` with `MessagePause`.
     - The additions to `message_kind_defaults_to_info_and_names_three_kinds` (unit): red with `UnknownArgument` for `-x`.
2. **File budgets.**
   - `run_cmd.rs` is 552 → 482 (budget 521). Its unit tests moved, unchanged, to `run_cmd/tests.rs`.
   - `run_cmd/status.rs` is 371 → 280 (budget 307). `orchestrator_lines`, `state_text` and the count and label helpers moved, unchanged, to the new `run_cmd/status_orch.rs` (103).
3. **Daemon text is printed without control characters.**
   - *The problem.* A refusal can echo what the user typed, for example `no such task nope<ESC>[2J`.
   - *The fix.* `status::printable` shows every control character except the newline as a space. It keeps a multi-line reply's `not delivered:` lines. The CLI applies it to every `anthrex run` error it prints (`run_cmd::main`), to every `Done` message (`print_outcome`), and to triage's message on `run start --goal`. The CLI had no existing sanitiser for this: `status::one_line` also folds newlines.
   - *Tests:*
     - `a_refusal_prints_no_control_characters` (e2e; red: stderr carried `\u{1b}[2J\u{7}`);
     - `printed_daemon_text_has_no_control_characters` (unit: ESC, BEL, CR and C1 CSI become spaces, and newlines stay).

Verification: the cli bin's unit tests (55), `run_cli_message` (8, three runs), `run_cli_orch` (10), `run_cli` (12), the workspace build, `fmt --check`, and `cargo +1.98 clippy -D warnings` for the host and for `x86_64-unknown-linux-gnu`. All pass. No test process was left running.

- **Controller check, 2026-09-29 (M9.14 review follow-up): a Codex orchestrator's shell cannot reach the daemon socket.** The orchestrator's PTY keeps `ANTHREX_SOCKET` (decision 10), so a Codex orchestrator could in principle run `anthrex run approve --hold …` or `anthrex run accept` from its shell tool. Checked with codex-cli 0.156.1: `codex sandbox -- python3 -c "<AF_UNIX connect to a listener under /tmp>"` (the default read-only seatbelt, no model involved) fails with `PermissionError: [Errno 1] Operation not permitted`, and the listener sees no connection. The Claude orchestrator has `Bash` disallowed. Escalating out of the sandbox under `-a on-request` needs the user's approval in the PTY, which is a human action. So no model can approve a hold or accept a run through the CLI.

### Task M9.15 (TUI)

Two commits: `refactor(tui): move the plan gate's keys and replies into their own file` (a pure move; `app/runs.rs` was already 530 against a 524 + 40 budget), then the task's commit.

- **Where the code went (deviations from the file list).**
  - `app/runs.rs` keeps the tagged-reply routing, `tagged_request` and `pending_open` (411 lines). The plan gate's keys, the edit form's keys and the stale checks moved, unchanged, to `app/run_gate.rs` in the move commit. The hold keys, the `s` key and their stale checks are in the new `app/run_holds.rs`; the goal form's opening, keys and replies in the new `app/goal.rs`.
  - `inspector/run_orch.rs` (new) holds the orchestrator row and a task's `messages` and `notes` rows, so `inspector/run.rs` (+15) and `inspector/run_task.rs` (+29) stay near their budgets.
  - `crates/tui/src/safe_text.rs` (new) is the sanitiser (below). `tree/tests/orch_fixtures.rs` (new) holds M9's fixtures, since `run_fixtures.rs` is 571 lines.
  - `app/run_enter.rs`, `app/headless.rs` and `inspector/run_round.rs` were not changed. Enter on the root already focuses the orchestrator (the pinning test `enter_on_the_root_focuses_the_orchestrator` passes unchanged). Decision 11's kill and remove refusal had landed in M9.10 (`kill_and_remove_of_a_live_orchestrator_open_nothing`, `app_tests/headless.rs`). The research label is in `tree/run_rows.rs::round_label`, which the round inspector already reads.
- **Budgets passed** (every file stays under 600): `app/mod.rs` 499 → 520 (+3; a `Modal` variant, two `PendingAction` variants, two fields, the toast's sanitising, three `mod` lines); `theme.rs` +27 (+15; `task_look`); `tree/runs.rs` +30 (+15; `is_paused`, `awaiting_holds`, `task_held`); `ui/conversation.rs` +7 (+5; rustfmt spreads the `TurnHeader` pattern over three lines once it binds `turn_id`); `ui/modal.rs` +7 (+5; the help row and the confirm's sanitising). `inspector/run_task.rs` +29 (+25; the stage's `paused`/`held` words and the two new rows' calls; the rows themselves are in `inspector/run_orch.rs`). Within budget: `keymap.rs` +3, `tree/run_rows.rs` +1, `graph/run_text.rs` +6, `graph/paint/style.rs` −2, `inspector/run.rs` +15.
- **State labels and glyphs.**
  - A `planning` run: the root's canvas text is `orchestrator  planning` (or `run <h4>  planning`) instead of `merged/total`. The inspector's header is `planning · <age>`. Its `gate` row is `planning · s submits the plan yourself`. The glyph keeps the working colour. The status bar hint is `s submit  j/k move  ⏎ open  f filter: <f>  esc back`.
  - `theme::task_look(state, gate_open, held, paused, animating, frame)` is the one place a task's glyph and colour come from (the canvas and the `deps` row). A held task (its hold `drafting` or `awaiting`) is `○` in the idle colour, and its stage gets ` · held` (for example `S · tdd · queued · held`). A `paused(message)` task is `‖` in `theme::PAUSED_COLOR` (`#cba6f7`, no status colour), and its stage is `paused (message)`. `Reported` keeps M9.2's `✓`, done colour and `reported`.
  - `round_label(Scout, n, _)` is `research #n`. M8c's `an_unexpected_role_on_a_task_is_labelled_not_dropped` now expects `research #1`.
- **Attention.**
  - A `paused(message)` task is no longer a blocked task in the `attention` row: it appears only through the daemon's own line, `<t> paused(message) for <n> min`, after 600 s.
  - Each awaiting hold is an attention line: `hold <id>: 1 task waits for approval`, or `… <n> tasks wait for approval` for several (the singular is a deviation from the brief's one plural form). Hold lines come first among the non-blocked lines.
  - M8b's promotion-line rewrite (decision 29) is gone. Every line is shown as sent. M8c's `a_promotion_line_carries_the_local_request_time` became `an_attention_line_is_shown_as_the_daemon_sent_it`.
- **Additions not named by the brief.**
  - `tree::run_status`: a run with a hold awaiting approval rolls up as `Attention`, and a `paused(message)` task alone no longer does.
  - The run inspector gains an `orchestrator` row (`<runtime>[ <model>] · window #<n> | no window yet · live | exited · <n> wakes · plan submitted | not submitted`) and a `summary` row, both shown only when `RunInfo.orchestrator` is `Some`, so M8c's run mockups are unchanged.
  - The planner inspection gains `note` (when `PlannerInfo.note` is set) and `session` (as a scout's, `main checkout, read-only`). **M8c mockup changes, both for the new `session` field only:** `planner_fields_match_the_mockup` (`inspector/run_nodes_tests.rs`) and `planner_panel_matches_the_mockup`'s `PLANNER` panel (`inspector/panel/rows_tests.rs`) gain the row `session   no window yet`.
  - A task's `messages` row is `<count> · latest <info|change|stop and wait>: "<line>"`, shown from the first message. Its `notes` row lists the `discovery` and `risk` notes, newest first, as `<hh:mm> <kind> from <task>: <text>`, joined with ` · `. Both come before `history`.
- **Holds (decision 28).**
  - On a run past its gate, `a` on a held task whose hold is `awaiting` sends `ApproveHold` at once (untagged). `x` opens M8c's confirm, `Reject hold <id> of run <run>? Its <n> task(s) is/are cancelled.`, and sends `RejectHold` on `y`.
  - On the root, both keys act on the one awaiting hold. With several, they toast `select a held task to approve its hold` or `… to reject its hold`. With none, the gate's old toast stands.
  - A held task whose hold is not awaiting toasts `hold <id> is drafting; it can be approved once it awaits approval`.
  - A reject confirm closes with `hold <id> is <state>` once a snapshot shows the hold decided.
  - The status bar shows `a approve hold  x reject hold` while a hold awaits.
  - `a` sends without a confirm because the brief names the confirm for `x` only.
- **The user's submit (ruling 5).**
  - `s` is free in the run view.
  - On a planning run's root, `s` opens `submit the plan of <run> yourself? (y/n)`. On `y` it sends a tagged `Edit { edits: [], submit: true }`, whose reply is toasted.
  - On any other node or state, `s` falls through to the tree's keys as before.
  - The confirm closes with `run <id> is <state>; only a run being planned can be submitted` once the run leaves `planning`.
- **Request ids (decision 2).**
  - `App::tagged_request` numbers requests from 1 for the client's life.
  - The edit form, the goal form and the submit are sent as `ClientMsg::RunTagged`. The form keeps its `request_id`.
  - `Done` and `Refused` reach a form only when their `request_id` equals the form's. Every other reply is toasted, whatever request it names; `Triaged` reaches only the goal form.
  - A connection refusal frees a form only for its own id. A lost link or a reconnect frees both forms.
  - The `d` confirm's `CancelTask` edit, the gate's approve and reject, and the hold verdicts stay untagged, since no form waits on them.
  - **M8c test changes:** the edit-form tests in `app_tests/gate.rs`, `gate/replies.rs` and `gate/unsent.rs` expect `RunTagged` with ids 1, 2, 3 (new helpers `form_edit`, `form_edit_as`, `to`). `no_gate_path_sends_input` accepts `RunTagged` as well as `Run`. The new test `an_edit_reply_for_another_request_leaves_the_form_submitting` pins the id match.
- **The goal form (decision 44).**
  - `C-b g` (`Command::StartGoal`, in the help overlay as `start a goal`) takes the project from, in order:
    - the selected project row;
    - the project of the run that the selected node belongs to (the run's row, or any run-view node of it);
    - the focused window's project.
  - With no project, it toasts `select a Git project to start a goal`.
  - The form has four fields:
    - `goal`: `Ctrl-J` inserts `↵`, which is sent as `\n`; the goal is trimmed; pastes keep their lines;
    - `runtime`: `‹ configured ›` / `claude` / `codex`; changing it clears the model;
    - `model`: `default` when empty, which sends `model: None`;
    - `trust`: a `[ ]` toggle on Space or ←/→.
  - `Enter` refuses an empty goal (`type a goal first`) and a model without a runtime (`choose claude or codex for a model`).
  - `Enter` sends the tagged `StartGoal { yes: false, unconfined_checks: false, … }`. While submitting, only `Esc` and `Ctrl-C` act.
  - A matching `Triaged` closes the form and toasts its message. With a run id, it sets `pending_open`, which opens the run view as soon as a snapshot shows the run (at once, if one already does).
  - A matching `Refused` fills the error row with the first line and `(+n more)`. The form keeps its input.
  - The renderer is `ui/run_goal.rs`, titled ` start a goal in <project> `.
- **Sanitising (M-6).**
  - `safe_text::one_line` turns every `Cc` control character (which includes U+0085) and U+2028/U+2029 into a space, and drops U+200E, U+200F, U+202A–U+202E and U+2066–U+2069. The TUI had no shared sanitiser beyond `inspector/run_format.rs::clean`, which let `Cf` and the line separators through.
  - It is applied in these places:
    - every inspector text, through `clean`;
    - `graph::content_text`: every canvas and sidebar node's text, including planner and epic names, scout questions and task titles;
    - the sidebar's run title;
    - every toast (`App::toast`), since toasts quote hold ids and daemon text;
    - the confirm modal's message;
    - the goal form's title and error.
  - Tests: `task_notes_and_message_lines_render_sanitised` puts every character above into the goal, the attention line, a hold id, the orchestrator's summary and model, a planner's epic, title, note and rejection, and each task's title, epic, message line, note text, note attribution and block text. It checks every run-view row's canvas text and every inspection string. `a_toast_quoting_a_hold_id_is_sanitised` covers the toast; it was seen red with `App::toast` unsanitised, then restored from a copy.
- **Conversation label (decision 42i).** `conversation_label::user_turn_label` reads the user turn's first text block. It returns `orchestrator` for the prefix `[anthrex] Message from the orchestrator (`, `user` for `[anthrex] Message from the user (`, and otherwise `you`, M8c's label. `ui/conversation.rs` calls it from the `TurnHeader` arm; `conversation.rs` is not touched.
- **Left open.** The followups file's "From M9.10, for M9.15" item (the TUI refuses `C-b x`/`C-b X` for a decision 11a placeholder headless window, which the daemon would let a client remove) stays open. The brief's M9.15 text does not assign it, and the fix needs the snapshot to tell a placeholder from a repository scout's window (a `WindowInfo` field, so a daemon change) or a different daemon answer. The TUI still offers no kill, input or subscribe for any headless window, so it never offers a control the daemon refuses. The followups entry now says so.
- **Red before green.** The new tests were written first. With stub types in place so that they compiled (`Modal::StartGoal`, `Command::StartGoal`, `PendingAction::{RejectHold, SubmitPlan}`, `theme::PAUSED_COLOR`), 36 failed, each for the missing behaviour:
  - the edit form sent untagged `Run(Edit)`;
  - `t1 blocked: paused(message) — …` was the attention line where the hold line or nothing was expected;
  - `⊘` and `M · tdd · blocked: paused(message)` were drawn where `‖` and `paused (message)` were expected;
  - `plan not approved` was the gate row where `planning · …` was expected;
  - the round label was `scout #2`;
  - there was no `messages`, `notes`, `note` or `session` row;
  - the promotion line still carried its local time;
  - `C-b g` did nothing, and `s` opened no confirm;
  - `delivered_message_turn_is_labelled_by_source` found `you` on the orchestrator's turn;
  - hostile characters reached the inspection strings.

  `run_goal_tests.rs`, `safe_text`'s own tests and `ui/run_goal.rs`'s render test were written with their code.
- **Verification.** `cargo test -p anthrex-tui` passes: 750 lib tests plus 6 and 1 in the integration binaries. `test_rigs_pin_agents` passes (2). The workspace build, `cargo fmt --all --check`, and `cargo +1.98 clippy --workspace --all-targets -D warnings` for the host and for `x86_64-unknown-linux-gnu` all pass. No test starts a daemon, and no process was left running.

### M9.15 review fixes

One commit, `fix(tui): check sanitising against a literal list, strip hidden format characters, raise the roll-up for holds and listed pauses, and toast a goal reply whose form closed`. Each test was written first and seen red, and each of the reviewer's mutations was then applied from a copy, seen red, and restored.

1. **Sanitising is checked against a literal list, not its own predicate.**
   - `safe_text::tests` now spells the characters out:
     - `spaced()`: all of C0 (with ESC, TAB, LF and CR), DEL, all of C1 (with U+0085), U+2028 and U+2029;
     - `DROPPED`: U+202A–U+202E, U+2066–U+2069, U+200E, U+200F, U+061C, U+200B–U+200D and U+FEFF.
   - `every_listed_character_is_spaced_or_dropped` checks that `a<c>b` becomes `a b` or `ab` exactly, and that a tab and a newline become spaces.
   - `is_safe` is removed. Every sanitising test (`task_notes_and_message_lines_render_sanitised`, `a_toast_quoting_a_hold_id_is_sanitised`, and the two below) now asserts `first_hostile(text) == None` against those lists.
   - Mutations, each red: U+2028 left out of the breaks; U+0085 let through; the isolates narrowed to U+2066–U+2068.
   - Red before the fix: the new test, and the three existing sanitising tests, found U+061C.
2. **The roll-up.** New `tree/tests/run_status.rs`:
   - `a_hold_awaiting_approval_rolls_up_attention` passed at once, since the behaviour came with M9.15. It goes red when the holds clause is removed.
   - `a_fresh_pause_alone_does_not_roll_up_attention_but_a_listed_one_does` was red before item 7's change. It also goes red when `!is_paused` is removed, or when the attention clause is removed.
3. **A goal's reply after its form closed.** A tagged `Triaged` that no open form waits on is toasted. An untagged `Triaged` still changes nothing, because this client sends `StartGoal` only tagged; M8c's `other_run_replies_change_nothing` pins that. Test: `a_goal_reply_after_the_form_closed_is_toasted` (red: no toast).
4. **The confirm's and the sidebar's sanitising are tested.**
   - `ui/modal.rs`'s confirm body is now `confirm_body(message, width)`; `a_confirm_quoting_a_hold_id_is_sanitised` checks its spans.
   - `a_run_title_is_drawn_without_hostile_characters` (`ui/tree_view_tests.rs`) checks the sidebar's run line.
   - Both were red on U+061C before item 8. Each goes red when its `one_line` is removed.
5. **Budget.** `inspector/run_task.rs` +29 (budget +25) is now in M9.15's budget list.
6. **The gate wins at the gate.** `the_gate_keys_win_while_the_run_awaits_approval`: on an `awaiting_approval` run, `a` on a task that names an awaiting hold opens the run's approve confirm. It passed at once (pinning). It goes red when `on_hold_key`'s `AwaitingApproval` guard is removed.
7. **A long pause raises the roll-up (controller's decision).** `tree::run_status` treats any line in the daemon's `RunInfo.attention` as Attention for a `running` or `planning` run. A `paused(message)` task therefore raises it once the daemon lists it (after 600 s, decision 42c), and not before. The daemon's attention also carries the last three `discovery`/`risk` notes (decision 42i), so a run with such a note rolls up as Attention too. That follows from "any daemon attention line asks for the user".
8. **More hidden characters dropped.** `safe_text::is_bidi_control` became `is_hidden_format`. It also drops U+061C, U+200B–U+200D and U+FEFF; the goal form's paste cleaning uses it too. Dropping U+200D splits a ZWJ emoji into its parts when drawn, which is accepted.

Verification: `cargo test -p anthrex-tui` (756 lib tests, plus 6 and 1), the workspace build, `cargo fmt --all --check`, and `cargo +1.98 clippy --workspace --all-targets -D warnings` for the host and for `x86_64-unknown-linux-gnu`. All pass. No process was left running.

### User rulings 2026-09-29

1. **`ANTHROPIC_BASE_URL` is scrubbed from agent sessions** (followups, "From M8b.7"). It joins `ANTHROPIC_API_KEY` and `ANTHROPIC_AUTH_TOKEN` in `config::reserved_env::API_CREDENTIALS`, so it has exactly the keys' scope:
   - `headless::credential_scrub_for` removes it from every headless session (workers, reviewers, scouts, planners, deciders) and, through the driver's `RoleLaunch.remove_env`, from the orchestrator's PTY window;
   - `run::exec::engine_env` already removed the keys from checks, proofs and setup, so it removes the endpoint there too; git commands and the user's own PTY windows are untouched, as for the keys;
   - a Claude session with `auth = "api_key"` keeps it along with its keys, since `credential_scrub_for` skips the whole Anthropic list for that session. Interpretation: the ruling said "the same scrub list", and the keys' scope was matched.
   - Tests: `headless_env::the_environment_is_scrubbed` (Claude and Codex sessions), `orchestrator_window::scrub_removes_agent_session_and_credential_variables`, `run_exec_env::engine_commands_get_the_profile_env_and_lose_agent_variables`, and the two exact-list unit tests (`decider_credential_scrub_removes_every_api_credential`, `only_a_claude_api_key_session_keeps_the_api_credentials`). Each was red before the list changed: the variable was in the dumped environment, or the list lacked it.
2. **The built-in roster's frontier Claude model is `claude-opus-5-5`** (was `claude-opus-5`, M8a decision 23). `config::orchestrator::roster::default_roster` and its doc comment changed; `[[orchestrator.models]]` still replaces or extends the built-in list as before.
   - Red first: `config::orchestrator::tests::defaults_when_absent` pinned the new name and failed. After the roster changed, the daemon lib tests that read the default roster failed (8), and were updated: `engine::tests::orch::{resolve_orchestrator_order, resolve_orchestrator_keeps_the_candidate_snapshot, planned_run_starts_in_planning_and_creates_branch_then_orchestrator}`, `history::tests::routing_history_keeps_choice_time_candidates`, `orch::digest::tests::digest_shape_matches_the_interface` (its `digest_fixture.json` t0 route), `plan::tests::routes::policy_fills_routes_by_class`, `roster::tests::frontier_level_falls_back_to_the_same_runtime` (and the default-roster case in the escalation test), and `scout::tests::route_picks_the_lowest_strength_at_or_above`.
   - Left alone, because they use `claude-opus-5` as an arbitrary model string and not the default roster: hand-built rosters and routes (`roster_tests`'s `s(..)` roster, `orch/test_support.rs`, `orch/mod.rs`'s tests, `reach_tests`, `dispatch_edits`, `snapshot_tests`, `argv_tests`, the `m8b_run.json` fixture), the proto, TUI and CLI display and parsing tests, and the recorded Claude transcripts.
   - Docs: this brief's context example (the `roster` line) and the product spec's `--orchestrator claude:claude-opus-5-5` example changed. Finished milestones' briefs (M8a decision 23 among them) are records and were not edited; this note supersedes decision 23's model name.

### M9.16

Three commits: two product fixes the scenarios found, each test-first in its own `fix(daemon)` commit, then the task's commit.

**Product bugs found and fixed**

1. `eadc270` `fix(daemon): rewrite the report when the orchestrator writes its summary`. A `complete` run's `REPORT.md` was rewritten only at accept or discard, so decision 38's summary was missing from the report the user reads before deciding. `engine/orch.rs::write_summary` now emits `Effect::WriteReport`. Test `engine/tests/orch_edit.rs::a_summary_rewrites_the_report`, red first (no `WriteReport` among the effects); end to end, `e2e_plan_path_scouts_plans_and_reads_the_approval` was red with `the report has no "Both files were added…" within 10s`.
2. `3b0b650` `fix(daemon): never paste a wake-up the orchestrator already read through run_status`. Decision 39's "a read clears too" dropped the engine's notes on `DigestRead`, but the driver kept the `WakeOrchestrator` it had already queued and pasted it once the orchestrator's turn ended. A polling orchestrator (the approval seen through `run_status`) then got `[anthrex] Run … changed: the user approved the plan…` in place of the user's next line. The driver's `run_status` answer now drops the waiting wake-up it covered (`Wakes::read`, by notes seq), and `check_orchestrators` drops a wake-up whose run no longer holds notes (the race where the read's event is reduced after the wake-up was queued). Test `driver/wake_tests.rs::a_digest_read_drops_the_wake_up_it_covered`, red with `read` made a no-op; end to end, `e2e_typed_steering_becomes_a_plan_edit` was red with the read message `"[anthrex] Run add-the-files-5ad7 changed: the user approved the plan. Call run_status for the details.skip t2"`, and `e2e_mis_sized_task_is_split_by_the_orchestrator` and `e2e_blocked_question_is_answered_after_a_wake` hung until their deadlines (their orchestrators exited 3 on the stale paste). `run_e2e_orch_wake.rs` and `run_e2e_orch_ops.rs` still pass.

**Files and deviations**

- The scenarios are in `run_e2e_orch.rs` plus `run_e2e_orch/{common,gate,steer,promote,messages,records}.rs` (`#[path]` modules of the one test binary), since one file would pass 600 lines. `cargo test -p anthrex --test run_e2e_orch` runs them all; the test names are the brief's, under their module (`gate::e2e_plan_path_…`).
- The shared helpers: `support/run_orch.rs` (M9.13's, now 297 lines) gains `mcp_log`, `wait_log`, `status_json`, `wait_digest`, `history_lines`, `wait_orchestrator_idle` and `tagged` (a `ClientMsg::RunTagged` request and the reply naming its id). The brief's `start_goal` is M9.13's `start_goal_id`, since M8b's `start_goal` returns the raw `Output`. The script steps and plan-task JSON are in a new `support/orch_script.rs` (the one new `mod` line in `support/mod.rs`).
- `run_harness.rs` sets `FAKE_AGENT_MCP_LOG` to `<tmp>/agent-io/mcp.jsonl` for every harness (+2 lines, 556).
- **Deviation: `fake-agent`'s MCP log lines gain `ms`,** the call's duration (`mcp.rs::log` takes it; `orch_steps.rs` and `headless_steps.rs` measure it). The brief asks that each `run_status` return within `wait_secs + 5` s, which the log could not show. `orch_modes.rs::mcp_log_records_each_call` checks `ms` is present and compares the rest exactly (red with the field removed).
- **Reading: `run_status` from a script has no `since`.** `fake-agent` fills captures into strings only, and `since` is an integer, so every scripted `run_status` (`mcp_until`, `wait_secs = 5`) answers at once. The bound is still asserted from `ms`. The long-poll itself is M9.11's driver tests'.
- A `marker` step (`run_status` with `wait_secs = 0`, which no other step sends) after a script's expectations shows the script passed them: `fake-agent` exits 3 at a failed `expect`, and the MCP log then has no marker.

**Scenario readings**

- `e2e_plan_path…`: the scout id is captured from `/scouts/0/id` and put in both tasks' `scout_refs` (the run id is unknown when the script is written); the tasks' `spec.scout_refs` in `run.json` are asserted. "REPORT.md starts with the summary section" is read as: the report's first `## ` heading is `## Summary from the orchestrator`.
- `e2e_mis_sized…`: "fail twice so t1 reaches rung 3" is three failed done checks (no commit): `worker-t1-1`'s two (`rung 1`, then rung 2's fresh session) and `worker-t1-2`'s one (rung 3, `blocked(mis_sized)`). `run.json` shows `rung = 3`.
- `e2e_promote…`: `t1` waits on a file with its turn open, not on `read_message`. A headless `read_message` ends the turn, and M8a's turn-end fallback then sends `NO_COMMIT_NUDGE` (or, with a commit, claims `task_done`), so `t1` would not simply wait. The orchestrator's `edit_plan` reply has `held: "promotion"` and `awaiting_approval: true`; after the user's `run approve --hold promotion`, its `mcp_until` sees `/gate/holds/0/state == "approved"`; the run completes with both tasks merged.
- `e2e_orchestrator_cannot…`: the override error is serde's `unknown variant \`override\``; `task_done` is refused by `anthrex mcp` (`tool task_done is not available to the orchestrator role`); the plan stays `awaiting_approval` with `t1` `queued`; the run branch is at `base_sha`.
- `e2e_wake_does_not_collide_with_typing`: the test types `x` (no Enter) every 300 ms, so the typed bytes and the paste make one `read_message` line: its `first_at` is the first keystroke and its `at` (the paste's submitting `\r`) at least 1 s after the last one. `t1` is seen `blocked` before the last keystroke, and no line is read while typing.
- `e2e_message_refresh_and_task_note`: `max_writers = 4` so `t0`–`t3` all work at once; `t0` has `interface_change = true`. "The event log shows no delivery while a turn was open": there is no persisted delivery log (an outbox entry is removed once delivered), so it is asserted from `run.json`'s `Task.orch.messages[].delivered`, which stays `false` for a second of polls while the test holds the worker's turn open (a file only the test writes), and is `true` afterwards. The `task_note` reply is `Note recorded. Keep working.`; the digest's `task_notes[0]` is checked by the orchestrator's `expect` steps; t1's measured diff has 1 file.
- `e2e_messages_are_not_redelivered_after_a_restart`: the messages are the user's (`anthrex run message`); `t1`'s `note this` is delivered before the restart and read once in all; `t2` is `paused(message)` after `run resume`, and its stop text was read once; a later `info` message releases it.
- `e2e_goal_form_request_matches_the_cli`: one harness, a second triage answer; `run start --goal --orchestrator claude`, then the form's tagged `StartGoal` (`yes: false`, `trust_project: false`, `unconfined_checks` off on macOS, `orchestrator: claude`). The reply is `Triaged` with `request_id` 7 and `planned_message`; both runs are planning, on the plan path, with the same triage and orchestrator route, each with its own window.
- `e2e_role_routing_records_survive_restart`: a run scout whose script hangs. Before the restart both records are in `run.json` and none in `history.jsonl`; after it, both are appended `interrupted` with their `record_id`, `session_id`, `chosen` and `candidates` as kept. `run stats --json` is equal before and after the restart. `run resume` restarts the orchestrator under a new session (`trigger: restart`, the same candidates); after `run reject` its record is appended too. The discarded run adds one run record to `run stats`; `rows`, `task_records`, `problems` and `decider_calls` are unchanged.

**Earlier-brief changes (M8b), as found**

- Done by M9.13 already: `e2e_goal_touching_a_hub_file_takes_the_plan_path`, `e2e_goal_owning_a_protected_file_takes_the_plan_path`, `e2e_goal_without_deciders_takes_the_plan_path` (a planned run through the file's `planned` helper); `triage_tests::messages_are_exact` without `refused_message`, which is deleted; `adapt_goal_tests.rs`'s hub test (now `a_hub_goal_reports_the_hub_reason_before_the_runtime_checks`, the fast-path build's hub reason; the planned run from a hub goal is the end-to-end test). M9.7 rewrote `fast_path.rs`'s two promote tests and M9.7/M9.9 `e2e_promote_performs_and_the_task_continues` (its repeat reply has no time).
- Done here: M9.13 had removed `e2e_goal_needing_a_plan_is_refused_without_side_effects` rather than renaming it; `e2e_goal_needing_a_plan_starts_a_planning_run` is added back under the brief's name (a decider's `plan` answer starts a planned run with its orchestrator, one triage call). `e2e_promote_performs_and_the_task_continues` loses its stale comment (the orchestrator's launch no longer fails) and now waits for the orchestrator to be live in its window; the promotion hold on its additions is `e2e_promote_starts_an_orchestrator_and_holds_its_tasks`, since that test's unscripted orchestrator adds nothing.

**Red before green.** The two product fixes are above. The other ten scenarios cover behaviour M9.7–M9.15 built, and passed at their first run once written; they were checked for teeth by mutation, restored from a `cp` copy: with the driver's quiet check removed (`ready` ignoring the last client input), `e2e_wake_does_not_collide_with_typing` fails at once (`a paste while typing`).

**Timing.** `docs/timing-budgets.md` gains "Recorded, from M9.16": `ORCH_WAIT` with the brief's derivation, the scripts' and tests' `k * RUN_WAIT` waits, the `wait_secs + 5` s bound, the wake-beside-typing bounds, and the one-second negative check of undelivered messages.

**Verification.** `cargo test -p anthrex --test run_e2e_orch -- --test-threads=1` three times in a row: 13 passed each time (113–115 s). `run_e2e_adapt` (8), `run_e2e_orch_wake` (5), `run_e2e_orch_ops` (11), `anthrex-daemon --lib` (1768), `anthrex-fake-agent` (all binaries), `test_rigs_pin_agents` (2). The workspace build, `cargo fmt --all --check`, and `cargo +1.98 clippy --workspace --all-targets -D warnings` for the host and for `x86_64-unknown-linux-gnu`. No test process was left running.

#### M9.16 fix round

Two commits: `fix(daemon): a stale engine snapshot never drops a wake-up built from a newer note`, then `test: …` for the review's minor items.

1. **A stale snapshot dropped a fresh wake-up (Important).** `3b0b650` made `check_orchestrators` drop a waiting wake-up when its engine snapshot showed no notes. That function runs from the 1 s tick, the window watch and `queue_wake` at once, so a call whose snapshot was taken just after a `run_status` cleared the notes could reach its `retain` after another call had queued a wake-up for a new note, and drop it. The engine kept the note, `last_wake_rev` did not move, and the orchestrator was never woken (the reviewer reproduced 8 losses in 3000 cycles).
   - **Fix:** `Seen` carries the engine's `OrchestratorRecord::last_note_seq`. The retain, now `Wakes::keep_live`, drops a live orchestrator's wake-up for "no notes" only when `p.notes_seq <= s.last_note_seq`, that is, only when the snapshot had seen every note the wake-up was built from. A wake-up from a newer note stays, and the next check judges it on a fresh snapshot.
   - **Why this and not a re-check under the engine lock:** the snapshot is read under the engine lock once, and the pending table under its own lock, never both together (AGENTS.md rule 2 and the file's lock rule). A seq comparison needs no second lock and cannot be defeated by the order the two locks are taken in: a wake-up queued after the snapshot always carries a seq the snapshot has not seen, because the engine assigns note seqs before it emits the wake-up.
   - **Test:** `driver/wake_tests.rs::a_stale_snapshot_keeps_a_wake_up_built_from_a_newer_note`, a deterministic interleaving: a snapshot with no notes up to seq 3, then a wake-up from seq 4 queued, then that snapshot's pass. It was red first ("the new wake-up was dropped by a snapshot older than its note"). The test also covers a snapshot that saw the note (dropped), notes held (kept) and a window that is not live (dropped). No stress variant was added; the interleaving is exact.
2. **The typing bound** (`promote.rs`): `at` is the submitting `\r`, so the assertion is now `at >= last keystroke + 1000 ms + SUBMIT_DELAY (200 ms)`, less 1 ms for whole-millisecond stamps. The keystroke's time is taken just before it is sent, so it is never later than the daemon's own record of the input. `SUBMIT_DELAY` is private to the driver; the test names its 200 ms in a commented constant.
3. **A real long-poll from a script.** `fake-agent`'s `mcp::fill` turns a string that is exactly `{{#name}}` into the capture parsed as JSON, or the capture as a string when it is not JSON. Plain `{{name}}` is unchanged. The test is `mcp.rs::fill_puts_a_json_value_for_a_whole_hash_capture`, red first. `e2e_plan_path_scouts_plans_and_reads_the_approval` now captures `/revision` after the summary and calls `run_status {since: "{{#rev}}", wait_secs: 3}`. Nothing changes a complete run's digest, so the call waits out its 3 s (asserted `>= 3000` ms, and within `wait_secs + 5` s as every call). This supersedes the reading above that scripted calls cannot pass `since`. `docs/timing-budgets.md`'s M9.16 rows are updated.

Verification: `run_e2e_orch` three times in a row with `--test-threads=1`, `run_e2e_orch_wake`, `run_e2e_orch_ops`, the daemon's `run::driver` lib tests, `anthrex-fake-agent`, clippy for both targets and fmt; see the report.

**Fix round 2.** Two commits: `fix(daemon): a check drops only the wake-ups it saw before reading the engine` (`1dac98b`), then `test(fake-agent): wait for a role session's argv record as for its claim`.

1. **The same stale-snapshot loss, on `live` (Important).** `check_orchestrators`'s `live` check predates `3b0b650`. On `run resume` after a daemon restart, the relaunch adds the daemon-restart note while the orchestrator is not live. The window's restart changes the window list, so a check takes a snapshot with `live = false`. The engine then sees the window back, sets `live`, and queues the wake-up for that same note. The stale check's retain then dropped it, and decision 11's first wake-up never came. The note seq could not help, because the seq is equal.
   - **Fix:** each waiting wake-up gets a generation (`Wakes::insert`, an atomic counter). A check takes the generations first, then reads the engine, and `keep_live` judges only the wake-ups in that set. That is sound because the engine commits a step's state before the driver runs its effects: a wake-up queued before the generations were taken comes from a step the snapshot already reflects. Any later wake-up (a new generation, replacements included) is left for the next check, at most one tick later.
   - **Why this rather than the reviewer's two suggestions:**
     - Carrying `launches` on `Effect::WakeOrchestrator` would change `engine/mod.rs`, which is at 600 lines. It would also cover only `live`, not a window id bound since the snapshot, or a run missing from a snapshot taken while its launch was pending. Those are the same race.
     - Never dropping for "not live" would leave a terminal run's wake-up to be pasted into what is by then a plain window.
     - The generation covers every field the retain reads, needs no second lock (the pending table and the engine are never locked together), and keeps the first round's rules. The notes-seq guard is kept as a second check; the generation implies it.
   - **Test:** `driver/wake_tests.rs::a_stale_not_live_snapshot_keeps_a_wake_up_queued_after_it`. The interleaving: generations taken, a stale snapshot with `live = false` and the note seq equal, the wake-up queued, then the stale pass. It also covers a run missing from the snapshot, the drop on a snapshot read after the queue, and a newer wake-up replacing a judged one. It was red first ("the wake-up was dropped by a snapshot taken before it was queued"). `a_stale_snapshot_keeps_a_wake_up_built_from_a_newer_note` now passes the generations explicitly, taken after the insert, so it still tests the seq rule on its own.
2. **Flake in `adapt_edges.rs::role_and_resumed_sessions_do_not_read_stdin_before_starting` (Minor).** `roles.rs` writes the claim, then the argv record, which the claim names, so the record is not guaranteed at the instant the claim appears. The test now waits for the record with the same deadline loop it uses for the claim, `exists_before_stdin`, with nothing written to stdin yet. What it asserts about stdin is unchanged, and fake-agent is unchanged.

Verification: the daemon's `run::driver` lib tests (75), `run_e2e_orch` once with `--test-threads=1` (13), `run_e2e_orch_wake` (5), `run_e2e_orch_ops` (11), `adapt_edges` five times (6 each), clippy for both targets, and fmt.

**Fix round 3.** One commit, `fix(daemon): deliver only the wake-ups a check judged, and pin its read order`. Both changes are in `driver/wake.rs`; both tests were seen red first.

1. **Deliver only judged wake-ups.** Round 2's `keep_live` skipped a wake-up queued after the check took its generations, but the check's delivery pass still delivered every waiting wake-up. So a check whose engine snapshot showed a run already terminal, with its window made plain, could still paste a wake-up it never judged into that window.
   - **Fix:** the delivery pass is now `Wakes::deliverable(&judged)`, which returns only the wake-ups whose generation this check recorded. The take is now `Wakes::take`, which also requires the generation to be unchanged, so a wake-up replaced between the pass and the take is not taken in place of the new one.
   - **No extra delay:** `queue_wake`'s own check records the new generation before it reads the engine, so it judges the new wake-up at once.
   - **Test:** `a_check_delivers_only_the_wake_ups_it_judged`. An unjudged wake-up is not deliverable by that check, and is by the next. A replaced wake-up is not taken for the old one. It was red first: "delivered unjudged".
2. **The read order is pinned.** `Wakes.between_reads` is a `#[cfg(test)]` hook that runs between `check_orchestrators`' two reads.
   - **Test:** `a_check_reads_the_generations_before_the_engine`. The run's orchestrator is not live and holds the daemon-restart note (seq 7). The hook marks it live and queues the wake-up for seq 7, as the engine's relaunch does. After the check, the wake-up must still be waiting.
   - With the two reads swapped (the hook stays between them), it is red: "the wake-up queued during the check was dropped". This was run from a `cp` backup, which was restored.

Verification: the daemon's `run::driver` lib tests (77), `run_e2e_orch` once with `--test-threads=1` (13), `run_e2e_orch_wake` (5), clippy for both targets, and fmt.

### M9.17

One commit, `test: cover the large path, research and review goals, restart and metering end to end, and add a smoke stage`. No product change: every scenario passed against the code M9.1–M9.16 built, once its fixture was right. The `ROADMAP.md` row and the followups file's "From milestone 9" section are left to the controller after the whole-branch review, as instructed.

**Files and deviations**

- The nine scenarios are in `crates/cli/tests/run_e2e_large.rs` plus `run_e2e_large/{epics,large,kinds,restart,records}.rs` (`#[path]` modules of one test binary, as M9.16 split `run_e2e_orch`), each under 250 lines. M9.16's `run_e2e_orch/common.rs` is shared through `#[path]` (with `#[allow(dead_code)]`), not copied. `epics.rs` holds what the large path adds: triage answers of other scales and kinds, `spawn_subplanner` and `submit_epic` steps, the planner scripts (`planner-<e>-1`, `get_context` first), and the integration reviewers' scripts (`reviewer-<e>-int<n>-1`).
- **Fixture reading: the epics' directories exist in the base commit.** `fake-agent`'s `git_commit` writes its file but creates no directory, so a worker committing `src/a/one.rs` failed its turn (`write src/a/one.rs`) and `t0` went to rung 3. `epics::SRC_FILES` puts `src/api.rs` and `src/{a,b,c}/mod.rs` in the base commit. `fake-agent` is unchanged.
- **`t0` is the interface task, not a hub task.** The brief's "interface task `t0` (hub)": `t0` has `interface_change = true`. The stored profile (`STORED_PROFILE`) names no hub globs, and a real hub task is forced to `tdd` (M8a rule 8.2), which the scripted worker does not follow; `hub` stays false.
- **A planner is given the run scout's report through `scout_refs`.** In `e2e_role_history_for_large_and_triage_paths` the run has a finished scout report, so rule 7.1 makes the planners' tasks name it. A planner's `get_context` lists only its epic's `scout_refs` and the reports whose area meets its own (`context.rs::scouts`); the scout's area is `tests/**`. The orchestrator captures the scout id and passes it as each `spawn_subplanner`'s `scout_refs`; the planner captures it from its context into its tasks' `scout_refs`.
- **Deviation: the failed run creation is decision 9's refusal, not a missing binary.** The brief's direct triage test fails run creation with "a missing runtime binary". No start check looks for the binary: `RunService::check_runtimes` (`driver/build.rs`) checks decision 50's API key and decision 53's project settings only, and a Codex orchestrator whose `ANTHREX_CODEX_BIN` did not exist was started (run `add-the-files-7649` in the first attempt, which then only failed to launch). Decision 26's "(binary present, …)" describes a check the merged code does not have; adding one would change every run's start and is outside this task. The closest faithful failure after triage is decision 9's: the repository tracks `.codex/config.toml`, and a goal with `--orchestrator codex` and no `--trust-project` is refused after its triage answered. That goal leaves its triage record with `run_id: None`, and no run.
- **`e2e_claude_orchestrator_gets_the_otlp_environment` records the environment with a wrapper,** as M9.13's re-review test does: `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` are a script that writes `env` to `env-<ANTHREX_WINDOW_ID>.txt`, then execs `fake-agent`. The Claude orchestrator's variables equal `launch::role::otlp_env(<otlp.addr>, <run>, <run.json's token>)`, `ENABLE_TOOL_SEARCH=false` is set, and `ANTHROPIC_BASE_URL`, which the test daemon was started with, is absent (user ruling 2026-09-29), while the daemon's `FAKE_AGENT_MCP_LOG` is present, so the daemon's environment did reach the window. A second goal with `--orchestrator codex` has none of the seven OTLP variables and no `ANTHROPIC_BASE_URL`, and its `usage.by_role.orchestrator` is zero. The assertion messages print the variables' names only.
- **Found, not fixed: the report's `orchestrator usage: not metered (codex)` line.** Decision 14 says the report carries it; no code writes it, and no task's file list names it. The scenario asks only for the zero usage, which holds. Left for the controller.
- **Restart.** The resumed orchestrator's argv (its second `.args` line) has `--resume` and the same value for `--settings`, `--mcp-config`, `--allowedTools`, `--disallowedTools` and `--append-system-prompt`, and every other `--` flag of the first launch. Its first wake-up (the first line its `read_message` read after `run resume`) contains `the daemon restarted and your session was resumed`. The test waits for the orchestrator's turn to end and its session id before the restart.
- **Other readings.** `e2e_large_path…`: "`RunInfo.planners` has both with `edits_accepted`" is `edits_accepted == 1` (one batch of one edit each) and `edits_rejected == 0`; both epics' integration reviews (`a-int1`, `b-int1`) approve, and the planner windows are gone within `RETIRE_AFTER + REQUEST_WAIT`. `e2e_subplanner_edit…`: the refusal is `src/b/x.rs is outside the area src/a/**` (M8a's `EditScope::Area` text); the orchestrator's digest has `planners[0].rejected == 1` and `tasks == 1`, and `last_rejection` names the file; the rejected batch is in the edit log with source `planner:a`. `e2e_new_epic…`: epic `a` is planned before the gate and epic `c` after it; `c1` waits under `epic:c` with no session while `a1` works and merges; `spawn_subplanner`'s reply names the hold; the orchestrator's `run_status` sees `/gate/holds/0` `approved` with id `epic:c`. `e2e_integration_review…`: the wake-up the orchestrator reads contains `integration review of epic a: changes (1 critical, 0 important)`; it adds `a9` with `epic = "a"`, which gets no hold (epic `a` was planned before the gate, so it has no round); `a-int2` approves and the epic's integration is `approved` with tasks `a-int1`, `a-int2`. `e2e_research_goal…` and `e2e_review_goal…`: `run accept` answered `n` prints `research report: <path>` first (research) and the nothing-to-merge question, then `run accept --yes` accepts; the base branch (and `feature`) are unchanged. The review prompt names both shas as `sha7`, as `review_task_prompt` does.
- **Red before green.** The scenarios cover behaviour M9.7–M9.16 built and passed once their fixtures were right (the two fixture defects above were the only reds). Two were checked for teeth by mutation, each restored from a `cp` copy: with `ANTHROPIC_BASE_URL` dropped from `config::reserved_env::API_CREDENTIALS`, `e2e_claude_orchestrator_gets_the_otlp_environment` fails at the endpoint's absence; with `manager/restart.rs` passing `role: None` to `launch::plan`, `e2e_restart_resumes_the_orchestrator_with_its_role_flags` fails (the relaunched process has no role flags and claims no orchestrator script).

**Smoke stage 11f** (`scripts/pty_smoke_orch.py`, one import and one call in `pty-smoke.py`)

- It follows the brief's steps, with these readings:
  - **The project node.** A fresh repository is no project the TUI lists until something runs in it, so on macOS the stage creates a shell window `orch-smoke` in the repository (`anthrex new --runtime shell --dir <repo>`), then `C-b t`, `/orch-smoke`, Enter, `k` (to its project row), `C-b g`; the form's title must name the repository's canonical path. The shell and the orchestrator's window (a plain window once the run is accepted) are removed in the `finally`.
  - **The new run's id** is read from `run status --json` (the one unfinished run of the goal); the stage then waits for the run view's title.
  - **`planning` is transient.** The scripted orchestrator submits within moments of starting, so the stage waits for either the root's `orchestrator  planning` or the gate's status-bar hint (`a approve  x reject`), prints which it saw, and checks through `run status --json` that the run's `path` is `plan` with an orchestrator. In every local run it saw `orchestrator  planning`.
  - **Screens, not words.** "awaiting approval" is the status-bar hint `a approve  x reject`; "complete" is the run's state from `run status --json` and then the root's `orchestrator  2/2` on the canvas (the inspector's texts depend on its layout).
  - **Two waits the brief does not name,** added after two early local failures that could not be reproduced afterwards (once the run stayed `awaiting_approval` after `a`, `y`; once the summary never came): after `y` the stage waits for the run to leave `awaiting_approval`, and before typing `done\r` it waits (`anthrex ls --json`) until the orchestrator's window is `idle` or `done`, that is, until its `read_message` waits for the line. Every wait's failure prints the rendered screen. After the two failures, 12 runs of the stage alone passed in a row without these waits, and 6 more with them (plus the forced off-macOS run below); neither failure recurred, so their cause is not known. If one recurs in CI, the screen in its message is the evidence.
  - The orchestrator's script ends with a `read_message` that never comes, so its window stays live until the stage removes it.
- **Off macOS** (`run start --goal … --unconfined-checks`, then `C-b T`, `/`, the run's `h4`, Enter, `l`, `l`, as stage 11e opens a run) could not run on Linux here. It was run on macOS by forcing the branch from a scratch driver that loads `pty-smoke.py` and calls `orch_stage` against its isolated daemon: it passed.
- The smoke daemon is `pty-smoke.py`'s: `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under a `/tmp` `mkdtemp`, `ANTHREX_GIT=off`, `fake-agent` as `ANTHREX_CLAUDE_BIN`, `ANTHREX_CODEX_BIN` and `ANTHREX_DECIDER_BIN`, so no real agent can start.

**Timing.** `docs/timing-budgets.md` gains "Recorded, from M9.17": the scenarios' `k * RUN_WAIT` and `ORCH_WAIT` waits, the planner windows' `RETIRE_AFTER + REQUEST_WAIT`, and stage 11f's bounds.
