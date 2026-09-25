# Milestone 8b: Adaptation plumbing

> Written 2026-09-22 against `docs/superpowers/specs/2026-09-22-adaptive-orchestrator-design.md` ("the spec" below), which binds this milestone, including its same-day amendment of §6 and §22.4: the repository profile lives only in anthrex's data directory, and nothing is ever written into the repository. **Refreshed 2026-09-26 against the code milestone 8a shipped** (branch `m8a-engine-core` at `baa04e1`, PR #17, merging unchanged; protocol 7). Every name below that this brief takes from M8a is checked against that code and listed under "M8a names as shipped"; everything else is marked new. Every decision the refresh changed says so inline (*Refreshed:*) and is recorded, with its reason, under "Implementation notes", "Refresh 2026-09-26 (post-M8a)". "M8a" below is `docs/milestones/M8a-orchestration-engine-core.md`, whose "Summary of deviations (read first)" and final-fix-batch notes (F1 to F4) describe what shipped where it differs from M8a's own decisions.

The user's design rules, which bind every decision here:

- Only the orchestrator is an interactive PTY window. Workers, reviewers, scouts and sub-planners run headless (`claude -p` stream-json, `codex exec --json`). The user watches them through the conversation view and can never type to them; the daemon refuses input, kill and terminal subscribe for them.
- TDD only when the task needs it: `test_mode` `tdd | check | none`, `tdd` the default for code.
- The engine is deterministic. No model merges, approves a task, or writes to the base branch. Nothing reaches the base branch until the user accepts the run.
- Headless agents load only the user's own settings, never a repository's; workers run sandboxed; protected agent-config files (`.claude/**`, `.mcp.json`, `.codex/**`, `CLAUDE.md`, `AGENTS.md`) change only when a task's `owns` names them exactly.
- No repo-level profile file: the repo profile lives in anthrex's data directory.

## Header

| | |
|--|--|
| Status | `ready` — milestone 8a is done (PR #17). |
| Depends on | Milestone 8a (the engine, headless sessions, the MCP crate, `fake-agent`'s headless modes, the run harness, confined checks). Milestone 8c may run before or after this one, but not at the same time: both raise the protocol (roadmap, "Why this order"). |
| Spec sections | §4 (roles; the decider row; headless sessions; read-only launch; containment), §4.2, §5.1 (triage, the fast path, `run promote`), §6 (the repo profile and the onboarding scout, as amended, `generated` and `protected` included), §7.1 and §7.2 rule 5 (the size cross-check), §10 (the ≤ 40-line check summary, classifying free-text `task_blocked` reasons), §11.3, §13 item 3 (deciders in reader slots), §14 items 1, 3, 5 and 8 (scout once, filtered output, deciders, OTLP for the orchestrator), §15 (recording only), §16.5 (new snapshot fields), §17 (scrubbed environment, nothing deleted dirty), §19 (scout tool; deciders have no tools), §21 (M8b row, the fast-path scenario), §22.1, §22.4, §23 (deciders are a second model surface; thresholds are placeholders). |
| Branch | `m8b-adaptation` |
| Protocol version | **8.** Derivation: `pub const PROTO_VERSION: u32 = 7;` at `crates/proto/src/lib.rs:25` on `baa04e1` (set by M8a task 2), and `docs/ROADMAP.md` says main is 7 after M8a; 7 + 1 = 8. The test `proto_version_is_seven` (`crates/proto/src/lib.rs:69`) becomes `proto_version_is_eight`. If `main` is not at 7 on the day M8b starts, stop and record it under "Implementation notes" before touching the protocol. |

## Starting point

Existing code this milestone reads or changes, on `baa04e1`. Line counts are `wc -l` on that commit.

| Path | What is there |
|------|---------------|
| `crates/proto/src/lib.rs` (79) | `PROTO_VERSION = 7` (line 25); root re-exports **by name, never by glob** (C20 comment): `run::{AgentRole, …, ProfileSpec, …}`, `run_info::{…, TokenUsage}`, `run_wire::{RunReply, RunRequest, ToolCall}`. |
| `crates/proto/src/run.rs` (451) | `AgentRole { Orchestrator, Worker, Reviewer }` (`snake_case`), `RunRef`, `Strength`, `Effort`, `Size`, `TaskKind`, `TestMode`, `RouteSpec`, `Route`, `Budget`, `ProfileSpec` (`deny_unknown_fields`: `modules`, `hub`, `source`, `check`, `check_timeout_secs`, `single_test`, `test_passed`, `setup`, `generated`, `protected`, `env`; **no** `cache_dirs` or `confined_*`), `PlanTask`, `Plan`, `PlanEdit`, `ModelEntry`, `RunState`, `TaskState`, `BlockReason`, `GateKind`, `GateCounts`, `Finding`, `DoneSignal`, `FinishAction`. |
| `crates/proto/src/run_info.rs` (222) | `Spend`, `TokenUsage { input, output, cache_read, cache_write }` with `billable()`, `BlockInfo`, `CheckInfo { at, ok, code, timed_out, secs, summary: String, on_candidate }`, `ProofInfo`, `ReviewInfo`, `AgentRoundInfo { …, usage }`, `TaskInfo`, `BaseMovedInfo`, `RunInfo { …, readers_busy, unverified, worker_sandbox, unconfined_checks, trusted_project, rate_limits, … }`, `RunsSnapshot`. Its module doc: an `Option` tolerates absence; any other new field needs `#[serde(default)]`. |
| `crates/proto/src/run_wire.rs` (118) | `ToolCall { run_id, task_id, role, window_id, tool, args }`; `RunRequest::{Start { plan_toml, dir, yes, trust_project, unconfined_checks }, Approve, Reject, Edit, Retry, Override, Cancel, Resume, Finish, List, Subscribe, Unsubscribe, Tool}`; `RunReply::{Started, Done { request, message }, Refused { request, message }, ConfirmNeeded, Snapshot, ToolResult { ok, text }}`; `pub mod request` (labels; not re-exported). |
| `crates/config/src/orchestrator.rs` (570) and `orchestrator/{profile,roster,unknown}.rs` (351, 141, 90) | `config::Orchestrator` (every `[orchestrator]` field, including `unconfined_checks`, `cache_dirs`, `confined_network`, `confined_unix_sockets`, `confined_localhost_ports` — each a `BTreeMap` keyed by repository root, **the user's own config only**), `ClaudeHeadless { auth, api_key_helper }`, `read(table, problems)`, `orchestrator::unknown::report_unknown` (the known-key list). `[orchestrator.claude] auth = "api_key"` is refused at load (F2 C-I3, line 544). |
| `crates/config/src/lib.rs` (605) | Over 600 already (F4 follow-up). `pub use orchestrator::{ClaudeAuth, ClaudeHeadless, Orchestrator, default_roster};` (line 24). |
| `crates/config/src/reserved_env.rs` (313) | `reserved_env(key) -> Option<&'static str>` (line 176): the one list of variables a profile's `env` may not set (F2 C-I4 and round 2). `API_CREDENTIALS`, `OPENAI_CREDENTIALS`, `TASK_TMPDIR`. |
| `crates/daemon/src/run/` | The engine. Pure: `engine/**` (reducer), `model.rs` (430) + `model_rounds.rs` (213, re-exported), `plan.rs` (457), `validate.rs` (502), `edits.rs` (571), `role_launch.rs` (552), `contract.rs` (563), `messages.rs`, `env.rs`, `snapshot.rs` (230), `report.rs` (236), `report_task.rs` (216), `globs.rs`, `roster.rs`, `reach.rs`, `seatbelt.rs`. I/O: `driver.rs` (598) + `driver/{cleanup,effects,guard,merge,observe,ops,requests,restore}.rs`, `git/**`, `exec.rs` (555), `confine.rs` (345), `confine_cache.rs` (558), `proof.rs`, `journal.rs`, `reconcile/{mod,git,sessions}.rs`. |
| `crates/daemon/src/run/git/` | Task, review and proof checkouts are **standalone repositories** (F1c 3a, `git/checkout.rs` module doc): a checkout at `<wt>/runs/<run>/<name>` has its repository at `<data>/runs/<run>/tasks/<name>/` (`checkout_repo_dir`, `Repo { dir }`: `git/`, `engine/`), its objects alternate to the user's store, no refs, a detached `HEAD`. Only the per-run integration checkout is a linked worktree. `prepare_scratch_in(git, root, path, at, repo, timeout)` (worktrees.rs:501) makes a read-only standalone checkout; `remove_checkout(git, root, path, repo, timeout)` (salvage.rs:212) removes checkout and repository; `salvage(git, worktree, reference, message, timeout)` (salvage.rs:30) writes the salvage ref in the user's repository. `GitQueue::write(repo, f: Fn)` serialises writes per repository. `private_dir` and the no-follow `engine_child` spell every grant (F1c I1). |
| `crates/daemon/src/run/confine.rs`, `seatbelt.rs`, `exec.rs` | Checks, proofs and **`setup`** run confined on macOS under a `(deny default)` seatbelt profile (F1c, F1d): writes only to the checkout, its own object store, its per-task `TMPDIR` and the user's `cache_dirs`; no network unless the user's `confined_network`; no Unix socket or loopback port but the user's `confined_unix_sockets`/`confined_localhost_ports`; never the daemon socket; protected agent-config paths denied (F2 round 2, F4). `ConfineSpec` (public fields; `for_run(run)`, `for_checkout(dir)`), `confined(dir, command, env, timeout, confine)`, `start_refusal(worker_sandbox, available, allowed)`, `available()`, `CONFINEMENT_ENV`. `exec::run_matching` (line 165, `pub(super)`) runs a command and reports whether a line matched a pattern. Linux cannot confine: `run start` refuses unless `--unconfined-checks` or `[orchestrator] unconfined_checks = true`. |
| `crates/daemon/src/headless/` | `HeadlessSpec` (mod.rs:28: `runtime, model, effort, cwd, instructions, mcp: Option<McpTarget>, allowed_tools, claude_permission_mode, claude_disallowed_tools, claude_sandbox: Option<ClaudeSandbox { writable_roots, deny_write }>, codex_sandbox, codex_writable_roots, env, claude_auth, api_key_helper, run_ref, codex_config_guard`), `McpTarget { role, run_id, task_id }`, `SessionEvent`, `credential_scrub(spec)`; `session.rs`'s `HeadlessHandle::spawn(runtime, program, args, cwd, env, remove, on_event)` (blocking, line 123), `kill(grace)`, `close_stdin`, `wait_finished`; `argv.rs` (446): `CliCaps`, `CLI_CAPS` (`claude_user_settings_only = Some(["--setting-sources","user","--strict-mcp-config"])`, `codex_loads_project_config = true`, `codex_user_config_only = None`), `claude_settings(exe, window_id, sandbox, caps)`, `mcp_args`, `claude_args`, `codex_args`, `CODEX_SANDBOX_PINS`, `caps_with_test_overrides`; `claude_stream::ClaudeStream::parse_line` (stateful struct), `user_message`; `codex_stream::parse_line`; `codex_guard`. |
| `crates/daemon/src/manager/` | `WindowManager::create_headless(name, spec, session, first_turn, project, worktree)` (headless.rs:255), `headless_send`, `headless_retire`, `headless_kill`, `headless_interrupt`, `remove`, `signals()`, `headless_run`; `WindowSignal { window_id, pid, kind }`, `WindowSignalKind::{Session, Hook, Unprompted}`; `control_refusal(id, run: Option<&RunRef>)` (headless.rs:215, the text decision 49's guard sends); `ManagerConfig { exe, socket_path, claude_bin, codex_bin, cli_caps, worktrees_root, … }`, `from_vars` (config.rs:116). |
| `crates/daemon/src/server/run_api.rs` (96), `server/headless_guard.rs` (35) | Every run request but subscribe is answered by `RunService::request` on its own task. The guard refuses `Subscribe`, `Input`, `Kill`, `Remove`, `Restart` for any headless window. |
| `crates/daemon/src/lifecycle.rs` (439) | `RunService::new(manager, run_context)`, `runs.restore().await` (line 294) before `bind_socket` (line 297). |
| `crates/daemon/src/worktree.rs` (492), `project.rs` (200) | `hash8`, `repo_worktrees_dir(worktrees_root, project_root)` (line 144), `RESERVED_DIR = "runs"`, `run_git` (line 234, `--no-optional-locks`, scrubbed environment); `project::detect_roots_with(git, dir, timeout)`. |
| `crates/mcp/src/{lib,tools,forward}.rs` (106, 250, 148) | `McpOptions { role, run_id, task_id, window_id, socket }`, `tools_for`, `allowed`, `role_name`, `forward`, `TOOL_REPLY_TIMEOUT` = 100 s. |
| `crates/cli/src/main.rs` (594) | `hook` dispatched from raw `args_os()` before clap (line 159); `Command::Run`, `Command::Mcp`. |
| `crates/cli/src/mcp_cmd.rs` (121) | `anthrex mcp` (hidden): `McpArgs { role, run_id: String (--run, required), task_id, window_id, socket }`, `RoleArg { Worker, Reviewer, Orchestrator }`. |
| `crates/cli/src/run_cmd.rs` (472), `run_cmd/{status,finish}.rs` | `anthrex run …`; `RUN_REQUEST_TIMEOUT` = 180 s; `start --plan <file> [--yes] [--trust-project] [--unconfined-checks]`. |
| `crates/cli/src/hook.rs` (145) | Silent, deadline-bounded forwarding (`HOOK_DEADLINE` = 1 s). `filter-hook` copies its discipline. |
| `crates/fake-agent/src/` | `headless.rs` (553), `headless_steps.rs` (291), `roles.rs` (323: scripts `<git common dir>/fake-agent/<role>-<task>-<n>.jsonl`, claimed with `.claimed`; found through the checkout's alternate for standalone checkouts), `runtime.rs` (343: `discover` keeps only the first matcher group per event, `entries.get(0)`; `mcp_server`, `McpServer::flag`), `script.rs` (424: `Step`), `stream_claude.rs`, `stream_codex.rs`, `mcp.rs`, `main.rs` (342). Tests: `tests/headless_{modes,shapes,turns}.rs`, `tests/headless_support/{mod,shape,stub_daemon}.rs`. |
| `crates/cli/tests/support/` | `run_harness.rs` (525: `RunHarness`, `init_repo`, `git_in`, `script_in`, `RUN_WAIT` = 300 s, `REQUEST_WAIT` = 60 s), `run_plans.rs` (230: `plan`, `task`, `until`, `report_with`, …), `run_watcher.rs`, `run_daemon.rs`, `mod.rs` (`fake_agent_bin`, `tempdir`, `RunningCommand`). The harness writes `[orchestrator.cache_dirs]` for its repository and `unconfined_checks = true` off macOS. CLI tests do not include `crates/daemon/tests/support`; they build repositories with `init_repo`. |
| `crates/daemon/src/run/engine/tests/fixture.rs` (414) | `Fixture::new(plan_toml)`, `with_config`, `send(now, kind)`, `next`, `start`, `ready`, `approve`, `tick`, `done(op, result)`, `signal`, `op(name)`, `ops(name)`, `task`, `task_mut`, `run_mut`, `tool`, `tool_as`, `turn_ended`, `clean_check`, `merge`, `force`; `op_name`, `ops_in`. |
| `scripts/pty-smoke.py` (1793), `scripts/pty_smoke_run.py` (165) | Stage 11c is `run_engine_stage(run_cmd, fail)`, called at `pty-smoke.py:1712`; the smoke daemon inherits the script's `ENV` (`ANTHREX_DATA_DIR`, `ANTHREX_CLAUDE_BIN = fake-agent`, `ANTHREX_CONFIG`). |
| `docs/timing-budgets.md` | The rules for wall-clock bounds. Every new bound below names the constants it is derived from. |
| `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` | Where follow-ups go. |

## M8a names as shipped

Checked on `baa04e1`. Nothing in this brief renames anything M8a defines; everything M8b adds to an M8a type is a new variant or a new `#[serde(default)]` field, except the two visibility changes marked **vis**.

| Name | What it is |
|------|-----------|
| `run::engine::step(state, event) -> (EngineState, Vec<Effect>)`; `EngineState { runs, revision, stopped }`; `Event { now, kind }` | The pure reducer. |
| `EventKind::{Start { reply, run }, Approve, Reject, Edit { reply, run_id, edits, scope, refusals }, Retry, Override, Cancel, Resume, BaseAdvanced, Finish, Tool { reply, call }, OpDone { run_id, op, result }, Signal { window_id, signal }, Delivered, Restore { runs, replay, held }, Stop, Tick}` | Reducer inputs (`engine/mod.rs:79`). A request's answer is `Effect::Reply { reply, result: Result<String, String> }`. |
| `Effect::{Reply, Op { run_id, op, kind }, Deliver, Interrupt, KillWindow, RetireWindow, RemoveWindow, WatchWorktree, UnwatchWorktree, WriteReport, Persist, Publish}` | Reducer outputs. |
| `engine::ops::{OpKind, OpResult}` | `OpKind::{CreateRunBranch, PrepareWorktree, CreateWindow, ResumeSession, VerifyDone, CountCommits, DiffSoFar, Proof, Check, PrepareReview, MergeCandidate, HandBack, AbortMerge, RemoveWorktree, VerifyRefs, Accept, Discard}`; `OpResult::{…, Check { ok, code, timed_out, tail, secs }, CandidateRed { code, timed_out, tail, secs }, Merged { commit }, Finished { outcome, kept_branches }, Failed { message }}`. Both derive `Serialize, Deserialize, Eq`. |
| `run::model::{Run, Task, Profile, RunLimits, PendingOp, LogEntry, ReviewLevel, OpId}`; `model_rounds::{CheckRecord { at, ok, code, timed_out, tail, secs, on_candidate }, ProofRecord, ReviewRecord, AgentRound, …}` | `Run.{id, root, project, git_common_dir, wt_dir, data_dir, base_branch, base_sha, run_head, state, approved_by, profile, limits, roster, tasks, log, …}`; `Task.{spec, size, raised_size, hub, test_mode, notes, review_level, route, review_route, budget, state, block, rung, failures, bounces, stalls, budget_exceeded, conflicts, session, spent_total, worktree, head, merge_commit, rounds, reviews, checks, proofs, history, …}`. `Profile` has `cache_dirs`, `confined_network`, `confined_unix_sockets`, `confined_localhost_ports`, set **only** by `build_run` from the user's config keyed by root. |
| `run::plan::{parse_plan, resolve_profile(plan, config) -> Profile, build_run(plan, pre, ctx), BUILTIN_PROTECTED, DEFAULT_CHECK_TIMEOUT_SECS, Preflight { root, project, git_common_dir, base_branch, base_sha, protected_files }, BuildContext { id, wt_dir, data_dir, config, now, yes }, PlanError}` | `resolve_profile` is per key, plan over config; `protected` merges built-ins + config + plan. The private `for_repo(table, root)` (plan.rs ~285) finds a root-keyed config entry. **vis:** M8b makes `for_repo` `pub(crate)`. |
| `run::validate::{resolve_task, resolve_task_lenient}`, `run::globs::{validate_glob, names_literally}`, `run::roster::{find, first_at, peer, pick_reviewer, escalate}` | Task resolution, globs, roster policy. |
| `run::engine::ladder::reresolve(run, i)` (private, ladder.rs ~270) | Re-resolves review level, reviewer and budget for a raised size. **vis:** M8b makes it `pub(crate)`. |
| `run::git::{preflight, prepare_scratch_in, remove_checkout, salvage, read_ref, project_settings(git, root, base_sha, claude, codex_paths, timeout), codex_config_tree, checkout_repo_dir, Repo, task_tmp, private_dir, GitQueue}` | Blocking git helpers, `git: &OsStr` first and a timeout last. |
| `run::exec::{run_shell, run_confined, run_matching, ShellOutcome { ok, code, timed_out, tail, secs }, CHECK_TAIL_LINES, CHECK_SUMMARY_LINES, summary}` | **vis:** M8b makes `run_matching` `pub(crate)`. |
| `run::confine::{ConfineSpec, Confinement, confined, start_refusal, available, AVAILABLE}`, `run::proof::{proof_command(single_test, test), proof_pattern(test_passed, test)}`, `run::env::profile_env(&Profile, worktree)` | Confinement, the proof's command and pattern, the profile environment. |
| `run::role_launch::{worker_spec, reviewer_spec, protected_write_denials(cwd, owns), codex_config_guard(run, runtime), task_tmp_dir(data_dir, task), worker_git_roots, REVIEWER_TOOLS, REVIEWER_PERMISSION_MODE ("dontAsk"), REVIEWER_DISALLOWED_TOOLS, REVIEWER_CODEX_SANDBOX ("read-only")}` | Session specs. A Claude reviewer always runs under a read-only sandbox (`ClaudeSandbox { writable_roots: [], deny_write: … }`, F1c N4). |
| `run::contract::{check_failed_message(command, &CheckRecord), candidate_red_message(command, &CheckRecord), reviewer_prompt, blocked_recorded(kind)}` | Message texts. |
| `run::driver::{RunService, RunContext { data_dir, worktrees_root, orchestrator, git_roots, git, cli_caps }, RETIRE_AFTER (30 s), INTERRUPT_GRACE, unix_now}`; `RunService::{new, restore, spawn, stop, pushes, current, request}`; `driver/requests.rs::{request, start, build}`; `driver/ops.rs::run` | The driver. `RunContext` has no `exe` or `socket_path` (removed in F3 B-11): use `manager.config().exe` / `socket_path`. |
| `run::reconcile::{reconcile, Reconciled::{Replay, NotStarted}}`, `run::journal::{runs_dir, save_run, append, load_all}` | Reconciliation and the journal. Reconcile's leftover-session killer considers only `run.json`'s round pids (F3 B-6). |
| `ANTHREX_TEST_ABORT_AFTER_INTENT=<op kind>[:<n>]` | Debug-build crash injection. |
| `headless::…` | As in "Starting point". `SessionEvent::{Init, TurnStarted, UserText, AssistantText { text, parent }, ApiErrorText, ToolUse { id, name, input, parent }, ToolResult, ApiRetry, PermissionDenied, Compacted, Other, TurnEnded { outcome, usage, denials }, Diagnostic, Unknown, StderrLine, ProcessStarted, ProcessExited { code, signal }}`. |
| `manager::…`, `server::…`, `mcp::…` | As in "Starting point". |
| `config::Orchestrator` and `orchestrator::read`; `config::reserved_env::reserved_env` | As in "Starting point". |
| `fake-agent` headless modes; `RunHarness`; `Fixture` | As in "Starting point". |
| `RunInfo.approved_by` values `"user"` and `"--yes"` | Who opened the plan gate. |
| Decision 32's `task_blocked { kind?, reason }` (`engine/done.rs:199`, `engine/tools.rs`), missing `kind` read as `question`, reply `blocked_recorded(kind)` | What M8b's classification refines. |
| Decision 41's reader slots (`schedule::readers_busy`, `RunInfo.readers_busy`) | What deciders now also take. |

## Goal

The first time anthrex meets a repository, `anthrex profile detect` (or the first `anthrex run start --goal …` there) launches a headless, **read-only** **onboarding scout** in a disposable copy of the repository. It reads the repository and proposes the languages, the modules, the hub, the source globs, a setup command, a check command, a single-test command with the regex that proves a test ran, a sample test, and the per-checkout environment. anthrex then runs every proposed command itself in a fresh scratch checkout, **confined exactly as a run's `setup` and `check` are**, and proposes only those that passed. The user confirms the result once with `anthrex profile confirm`; it is stored in anthrex's data directory keyed by the repository, never in the repository. It is re-detected when a manifest or convention file changes, and corrected with `anthrex profile edit`. Every run in that repository then uses it, over any profile in a plan file. Nothing a model writes into the profile can widen what a confined command may reach: `cache_dirs` and the `confined_*` tables stay in the user's own config.

`anthrex run start --goal "<text>"` sends the goal to a **triage decider**: a one-shot headless call returning schema-validated JSON. A goal that one S or M task can do, touching no hub file, takes the **fast path**: one task, no plan gate, the normal gates, and the user's accept at the end. Any other goal is refused with the reason, because the planned path needs the orchestrator of milestone 9. `anthrex run promote` records the user's wish to promote a fast-path run, for milestone 9 to act on.

Three more deciders run inside the engine: a **size cross-check** of planned tasks against scout evidence, which can only raise a size; a **≤ 40-line check summary** that replaces M8a's raw 40-line tail in every check bounce; and a **classifier** for `task_blocked` reasons the worker gave no kind for. Every decider has a deterministic fallback, so a failed, slow or disabled decider degrades a decision and never blocks a run. In tests, `ANTHREX_DECIDER_BIN` replaces the decider binary with `fake-agent`.

A `PreToolUse` hook injected into every headless Claude worker rewrites test commands to run through `anthrex filter-run`, which keeps the full log in the task's own temporary directory and shows the agent only the failures. The engine meters deciders, scouts and, through an OTLP receiver ready for milestone 9's orchestrator, the one PTY session. It appends one record per finished task to `history.jsonl`, and `anthrex run stats` summarises that history. No test needs a model.

## Scope

In:

- The repository profile: its type and file format, storage in the data directory, the precedence over a plan file's `[profile]`, staleness by content fingerprint, and `anthrex profile status|detect|show|confirm|reject|edit`.
- The onboarding scout: a headless, read-only session in a disposable standalone checkout; confined command verification by the engine; proposals, confirmation, and re-detection.
- Scouts in general: `AgentRole::Scout`, the MCP tool `submit_scout_report`, the scout report type and its storage, `ScoutService`, which milestone 9's area scouts reuse, and the scout snapshot type.
- Deciders: the call on M8a's headless spawner, `[orchestrator.deciders]`, `ANTHREX_DECIDER_BIN`, four kinds (triage, size cross-check, check summary, blocked reason) with JSON schemas, prompts, validation and fallbacks, and reader-slot accounting.
- Triage, the fast path, `anthrex run start --goal`, and `anthrex run promote` as a recorded intent.
- The output filter: `anthrex filter-run`, the `anthrex filter-hook` `PreToolUse` hook, and its injection into headless Claude workers' `--settings`.
- Metering: run-level usage by role (deciders, scouts, and the orchestrator from OTLP), and the OTLP/HTTP-JSON receiver with the environment milestone 9 gives the orchestrator's window.
- Run history: per-phase times, diff measurement, `history.jsonl` appended as journaled ops, revert detection, and `anthrex run stats` (recorded aggregates, no proposals).
- `fake-agent`: a decider mode, scout scripts, a `bash` step that honours `PreToolUse` hooks, and discovery of every hook matcher group.

Out, each with the milestone that owns it:

| Out | Owner |
|-----|-------|
| The orchestrator window, its contract, the plan and large paths, `spawn_scout` and `get_context`, area scouts inside runs, sub-planners, research and review task kinds | M9 |
| Acting on a promotion (turning a fast-path run into a planned run) | M9 |
| Launching the orchestrator window with `metering::orchestrator_env`, and its `StopFailure`/`PreCompact` hook gaps | M9 |
| A profile form in the TUI (M8b's form is the CLI) | M9 (recorded as a follow-up) |
| Drawing scout nodes, the fast-path root node, usage and size-check fields in `C-b T` and the inspector | M8c |
| Threshold and budget refitting, routing proposals in `anthrex run stats`, adaptive concurrency, racing, the test-writer pattern | M9.5 |
| The output filter for Codex workers (Codex headless sessions get no hook configuration, M8a decision 25) | Follow-up, recorded under M9.5 |
| Killing a scout process left over from a daemon that died (process-kill code) | Follow-up, recorded (decision 11) |
| A learned router | Out of scope (spec §15) |

## Design decisions

Numbered and final. If one proves wrong or impossible, stop work on it, record the evidence under "Implementation notes", and continue with the tasks that do not depend on it. Decisions the 2026-09-26 refresh changed carry a *Refreshed* line; the full record is under "Implementation notes".

### Structure

1. **Where the code lives.** All daemon code is in `crates/daemon/src/`:
   - `profile/`: `mod.rs` (types glue), `resolve.rs`, `proposal.rs`, `store.rs`, `verify.rs`, `service.rs`.
   - `scout/`: `mod.rs`, `contract.rs`, `spec.rs`, `report.rs`, `machine.rs`, `service.rs`.
   - `decider/`: `mod.rs` (types), `schema.rs`, `prompt.rs`, `parse.rs`, `fallback.rs`, `argv.rs`, `call.rs`.
   - `output_filter.rs`.
   - `metering/`: `mod.rs`, `otlp.rs`, `server.rs`.
   - In M8a's `run/`: `run/triage.rs`, `run/phases.rs`, `run/history.rs`, `run/stats.rs`, `run/history_io.rs`, `run/model_adapt.rs` (re-exported by `model.rs`), `run/engine/deciders.rs`, and `run/driver/adapt.rs` (the new op executions and the new requests; `driver/ops.rs` and `driver/requests.rs` only dispatch into it).

   **Pure** (no `std::fs`, `std::process`, `std::thread`, `tokio`, `std::time::SystemTime`): `profile/{resolve,proposal}.rs`, `scout/{contract,spec,report,machine}.rs`, `decider/{mod,schema,prompt,parse,fallback,argv}.rs`, `output_filter.rs`, `metering/otlp.rs`, `run/{triage,phases,history,stats,model_adapt}.rs`, `run/engine/deciders.rs`. **I/O**: `profile/{store,verify,service}.rs`, `scout/service.rs`, `decider/call.rs`, `metering/server.rs`, `run/history_io.rs`, `run/driver/adapt.rs`. *(M8a decision 2's discipline; AGENTS.md rule 5's spirit on the daemon side.)*
2. **Protocol.** `PROTO_VERSION` becomes **8** (header). Every new request and reply is a new variant of M8a's `RunRequest` or `RunReply`, so `ClientMsg` and `DaemonMsg` gain nothing and the TUI's `DaemonMsg::Run(_) => vec![]` arm (`crates/tui/src/app/daemon.rs`) still covers everything. Every field added to an M8a struct is `#[serde(default)]`, and every new snapshot field is an `Option` or a `Vec`, so a milestone-8a `run.json` and a milestone-8a snapshot still deserialize. `proto::AgentRole` gains `Scout`. New proto modules: `crates/proto/src/profile.rs`, `scout.rs`, `adapt.rs` (triage, deciders, usage), `history.rs`, re-exported by name in `lib.rs` (never by glob; M8a's C20). Every new message gets a MessagePack round-trip test. *(AGENTS.md rule 4; spec §19 last line.)*
3. **Configuration.** New tables in a new file, `crates/config/src/orchestrator/adapt.rs`, a submodule of M8a's `orchestrator` beside `profile.rs`, `roster.rs` and `unknown.rs`, read by one call from `orchestrator::read`. `orchestrator/unknown.rs` gains the five new keys. `crates/config/src/lib.rs` changes only in its existing `pub use orchestrator::{…}` line, which gains the four new type names (no line added). Keys, defaults and ranges are in Interfaces. Invalid values are a `Problem` and keep the default, with M8a's message format. *(M8a decision 50's pattern; `lib.rs` is already over 600 lines. Refreshed: the file is `orchestrator/adapt.rs`, not `orchestrator_adapt.rs`, because M8a split `orchestrator` into a directory.)*

### The repository profile

4. **Storage.** A repository's data directory is `repo_dir(data_dir, project) = worktree::repo_worktrees_dir(&data_dir.join("repos"), project)`, keyed by `Preflight.project`, so every linked worktree of a repository shares one. It holds:

   | File | Content |
   |------|---------|
   | `profile.toml` | The confirmed profile (`proto::RepoProfile`, the spec §6 format plus the keys in decision 5). |
   | `profile.meta.json` | `ProfileMeta`: when it was confirmed, from which proposal, the verification record, and the fingerprint (decision 7). |
   | `proposal.json` | `ProposalRecord`: the one pending proposal and its state (decision 8). |
   | `scouts/<scout id>.json` | Repository-level scout reports (decision 13). |
   | `history.jsonl` | Run history (decision 33). |
   | `tasks/.onboarding/`, `tasks/.profile-verify/` | The repositories of the two disposable checkouts (decision 8), as `run::git::checkout_repo_dir(repo_dir, checkout)` names them; removed with their checkouts. |

   Every write goes to a temp file, is `fsync`ed and renamed, and the directory is `fsync`ed. **Nothing is ever written into the repository's files**: there is no `.anthrex/` directory and no repository-level profile file, because an agent could edit such a file to weaken the checks it is judged by. The only writes into the repository's `.git` are M8a's salvage refs (`refs/anthrex/salvage/onboarding/<secs>`), exactly as M8a writes its own. *(Spec §6 as amended, §22.4. Refreshed: the disposable checkouts are M8a's standalone checkouts, so their repositories live here.)*
5. **The profile type** is `proto::RepoProfile` (Interfaces). It has every key of spec §6, `generated` included (the files builds rewrite on their own, such as `Cargo.lock`, `package-lock.json`, `yarn.lock`, `pnpm-lock.yaml`, `poetry.lock`, `uv.lock`, `go.sum` and `Gemfile.lock`, filled by the onboarding scout from the lock files it finds and confirmed with the rest; M8a decision 55 bounces a change to one outside `owns` at rung 1).

   It also has `protected`, the files that configure or instruct future agents. **The stored list holds only extra entries on top of M8a's `BUILTIN_PROTECTED`** (`.claude/**`, `.mcp.json`, `.codex/**`, `**/CLAUDE.md`, `**/AGENTS.md`), which always apply (M8a decision 56):
   - The default is empty.
   - `protected = []` means "no extra entries". It never disables protection; nothing M8b stores can remove a built-in.
   - The onboarding scout reports the built-ins plus any other agent-configuration or instruction file it finds. `proposal::from_findings` drops the built-in entries from what it reports and keeps the rest, de-duplicated in order.
   - The user confirms the extras with the rest of the profile.
   - `anthrex profile edit protected …` adds or removes only non-built-in entries. A value that names a built-in is refused with `protected: <entry> is built in and always applies; list only extra entries`.
   - `profile show` and `profile::summary` print `protected: built-in <5 globs> + <extras or "no extras">`.
   - M8a enforces the union.

   It adds four keys of its own:
   - `sample_test`: the name of one existing passing test, needed to verify `single_test`.
   - `check_timeout_secs`: M8a's key, so a slow check survives re-detection.
   - `manifests`: the manifest files the scout read, which decision 7 watches. Spec §6 names manifests but gives no key for them.
   - `filter_prefixes`: which commands count as test commands for the output filter (decision 28). Spec §14.3 filters "test output" but no profile key says which commands produce it.

   **It holds no confinement setting.** `cache_dirs`, `confined_network`, `confined_unix_sockets` and `confined_localhost_ports` are the user's own `[orchestrator.*]` tables keyed by repository root (M8a F1c N3, F1d R4 and round 2 S1), and `RepoProfile` has no such key: it parses with `deny_unknown_fields`, so a `profile.toml` that names one does not parse (decision 6's refusal), and neither the scout's schema (Interfaces, MCP) nor `profile edit` can set one. A model-written profile therefore cannot widen what a confined command may write or reach.

   `env` obeys M8a's reserved list: `proposal::validate` refuses every key `config::reserved_env::reserved_env` names, with M8a's text `key <K> may not be set by a profile: <reason>`, and `build_run`'s own plan check refuses it again.

   `RepoProfile::spec()` gives M8a's `ProfileSpec`: `modules`, `hub`, `source` and `generated` become `Some` only when non-empty, and `env` only when it has entries. `protected` is `Some(extras)` only when there are extras. M8a's `resolve_profile` always unions `BUILTIN_PROTECTED` into the effective list, so an empty or absent value still protects every built-in. Every glob in `modules`, `hub`, `source`, `generated` and `protected` must pass M8a's `run::globs::validate_glob`. *(Refreshed: the confinement paragraph and the reserved-env rule are new, from M8a F1c N3, F1d, F2 C-I4.)*
6. **Precedence, exact.** `profile::resolve::run_profile(stored, stored_path, plan, config)` picks one source for the whole profile:
   1. **A confirmed stored profile exists** for the run's `project`. It is the profile, in full: every `ProfileSpec` key comes from it, and a key it leaves unset stays unset. The degradations of spec §6 and M8a decisions 10, 34 and 35 then apply to that key. The plan file's `[profile]` table and `[orchestrator.profile]` are ignored entirely. For every key the plan's `[profile]` set, the run log gets `profile.<key> from the plan file is ignored: this repository has a stored profile (<repo_dir>/profile.toml)`. `RunInfo.profile_source = Some(Stored)`.
   2. **Otherwise**, M8a decision 7's rule applies unchanged: per key, the plan's `[profile]`, else `[orchestrator.profile]`, else empty. `profile_source = Some(Plan)` when any key came from either, else `Some(None)` (degraded mode, spec §6).

   **`protected` is the one exception to the whole-source rule**, because adding protection can only tighten. It merges, as M8a decision 56 defines: the effective list is `BUILTIN_PROTECTED` ∪ the stored extras ∪ the plan's `[profile] protected` ∪ `[orchestrator.profile] protected`, de-duplicated in that order. A plan's `protected` entries are therefore not ignored, and get no "ignored" log line.

   **How it reaches M8a's `build_run`** (*Refreshed:* M8a's `resolve_profile` falls back to `[orchestrator.profile]` per key, so replacing `plan.profile` alone would let the config fill the stored profile's deliberate gaps). In `driver/requests.rs::build`, right after `git::preflight`, the driver calls `adapt::choose_profile(&mut plan, &mut config, &pre)`, which loads the stored profile (on `spawn_blocking`) and, for source 1:
   - sets `plan.profile = chosen.spec`, whose `protected` already holds the stored extras, the plan's and the config's entries;
   - sets the **cloned** config's `profile` to `ProfileSpec::default()`, so `resolve_profile` finds nothing else to fill in;
   - leaves the cloned config's `cache_dirs` and `confined_*` untouched, so `build_run` still takes confinement from the user's config for the repository root, never from the profile.

   The protected-files scan in `build` (decision 17 carry) then resolves the same chosen profile. `build_run` itself is unchanged.

   The stored profile wins as a whole, not per key, because its gaps are deliberate. A stored profile with no `single_test` records that the repository has no single-test runner, and a plan must not quietly fill that in. A stored profile whose file does not parse **refuses the run**: `run start` answers `the stored profile at <path> does not parse: <error>; fix it with anthrex profile edit or re-detect it with anthrex profile detect`. It never falls back to the plan's profile. `output_filter` and `filter_prefixes` are copied onto the run for decision 28; they are empty when the source is not `Stored`. `StartGoal` requires source 1 (decision 22). *(Spec §6 as amended; the coordinator's precedence: stored, else the plan's `[profile]`, else none.)*
7. **Staleness.** `ProfileMeta.fingerprint` maps each path in `conventions` and `manifests` (relative to the project root) to `fnv1a64(contents)` as 16 hex digits and the byte length, or `"missing"` for a missing file. Only the first 4 MiB of a file is hashed; its full length is still recorded. `profile::store::stale(project, meta) -> Vec<String>` lists every path whose fingerprint differs. It runs on `spawn_blocking` at every `run start`, at `profile status`, and at daemon start for every stored profile. A stale profile **is still used**; a changed file does not make it wrong. The run gets the attention line `the repository profile may be stale: <paths> changed since it was confirmed; run anthrex profile detect`. With `[orchestrator.onboarding] auto = true`, and no proposal pending or failed within the last hour, a re-detection starts (decision 8). *(Spec §6 "re-proposed when any file named in `conventions`, or a manifest, changes hash".)*
8. **Detection flow and proposal states.** `ProfileService` owns at most one proposal per repository, in `proposal.json`:
   `Preparing` → `Scouting` → `Verifying` → `Ready` → confirmed (the proposal is deleted and `profile.toml` written), or → `Failed { reason }` from any state.
   1. **Preparing.** `run::git::preflight` on the request's `dir` gives `root`, `project`, `git_common_dir` and `base_sha` (the checked-out branch's `HEAD`; a detached `HEAD` or an empty repository is refused with preflight's own text). A disposable **standalone checkout** (M8a F1c 3a) is made at `<wt>/runs/.onboarding`, detached at `base_sha`, with `run::git::prepare_scratch_in(git, root, path, base_sha, repo, timeout)` through `GitQueue::write(project, …)`, `repo = checkout_repo_dir(repo_dir, path)`. `<wt>` is `repo_worktrees_dir(worktrees_root, project)`. The name starts with `.`, which a run id cannot, inside M5's reserved `runs` directory. No `git worktree add` and no lock: the user's `.git` gains no `worktrees/` entry.
   2. **Scouting.** The onboarding scout runs there (decision 12). Its report arrives through `submit_scout_report`.
   3. **Verifying.** The scout's checkout is salvaged if dirty (`run::git::salvage`, ref `refs/anthrex/salvage/onboarding/<unix secs>`) and removed (`run::git::remove_checkout(git, root, path, Some(repo), timeout)`). Then `profile::verify` runs the proposed commands in a **fresh** standalone checkout `<wt>/runs/.profile-verify` at the same `base_sha`, made and removed the same way (decision 9).
   4. **Ready.** `proposal.json` holds the proposed profile with only the commands that passed, the verification record, and each dropped command with its reason and output tail.

   Detection starts from `anthrex profile detect`, from decision 7's automatic re-detection, and from `run start --goal` in a repository with no stored profile when `onboarding.auto` is true. A second `detect` while one is in progress is refused: `detection is already running for <project> (state <state>); anthrex profile reject stops it`. *(Refreshed: standalone checkouts and M8a's `salvage`/`remove_checkout` replace `git worktree add --detach`/`lock`.)*
9. **The scout reads; the engine runs the commands, confined.** Spec §6 requires `check` and `single_test` to have run successfully before they are proposed; spec §4 makes scouts read-only; M8a's follow-up (F1c N4) requires every scout to run under a read-only OS sandbox. *Refreshed:* the old decision let the onboarding scout run the commands in a writable disposable copy and the engine re-run them unsandboxed "as M8a runs `setup`". M8a now confines `setup`, checks and proofs, and requires read-only scouts, so:
   - The **onboarding scout** is read-only like every scout (decision 12): it reads, and may run commands that write nothing (its `Bash` runs in a sandbox with no writable path and no network). It does **not** claim to have run anything: the old `setup_ran_ok`, `check_ran_ok` and `single_test_ran_ok` claims are gone, since a read-only sandbox cannot build.
   - The **engine** runs each proposed command in the fresh scratch checkout, through `run::exec::run_matching` (made `pub(crate)`), each bounded by `onboarding.verify_timeout_secs`, with the proposed `env` (`{worktree}` substituted by `run::env::profile_env`) and M8a's engine environment (`engine_env`: agent and git variables and every API credential removed; every `ANTHREX_*` removed when confined), **under the same confinement a run in this repository would get**:
     - a `ConfineSpec` built by `profile::verify::confine_spec(config, repo_dir, pre, daemon_socket)`: `data_dir = repo_dir` (so `for_checkout` finds the checkout's repository at `<repo_dir>/tasks/.profile-verify` and anthrex's data directory two levels up), `common_dir = pre.git_common_dir`, and `cache_dirs`, `network`, `unix_sockets`, `localhost_ports` from the user's `[orchestrator.*]` tables for `pre.root` via `run::plan::for_repo` — never from the proposal;
     - present exactly when `worker_sandbox` is true and `confine::available()`. Where the platform cannot confine, detection is refused with `confine::start_refusal`'s text unless the request's `--unconfined-checks` or `[orchestrator] unconfined_checks = true` allows it, and `ProfileStatus.verify_confined` is `false`;
     - so a proposal that passes here passes in a run, and one that needs the network or a cache fails here as it would in a run.
   - The order:
     1. `setup`, if proposed. On failure it is dropped, and the check and single test still run.
     2. `check`. It is kept only if it exits 0.
     3. `single_test`, as `run::proof::proof_command(single_test, sample_test)`. It is kept, together with `test_passed` and `sample_test`, only if it exits 0 **and** a line of its output matches `run::proof::proof_pattern(test_passed, sample_test)`. A missing `sample_test` or `test_passed` drops all three with the reason `single_test needs sample_test and test_passed to be verified`.
   - **A command that did not pass is not proposed.** It moves to `ProposalRecord.dropped` with its exit code, timeout flag and last 40 output lines, and `profile show --proposed` prints them. When the verification ran confined, a dropped `setup` or `check` also carries the hint `it ran confined, as runs do: if it needs the network, a cache directory, a Unix socket or a localhost port, allow it for <root> in your config ([orchestrator.confined_network], [orchestrator.cache_dirs], [orchestrator.confined_unix_sockets], [orchestrator.confined_localhost_ports]), then run anthrex profile detect`. `test_passed` must contain `{test}` and compile as a regex, and `single_test` must contain `{test}`, or they are dropped with M8a decision 7's validation message (this picks up the M8a.10 follow-up "`test_passed` need not contain `{test}`" for stored profiles; plan validation is unchanged).
   - `generated` and `protected` are glob lists, not commands. They are validated with `validate_glob` and proposed as the scout gave them, with built-in `protected` entries dropped (decision 5). They are never "verified" by running anything.
   - The scratch checkout is salvaged if dirty and removed. The user's checkout is never written.
   - Verification runs on `spawn_blocking`, never under a lock; its git steps go through `GitQueue::write(project, …)`. *(Spec §6, §23 "The onboarding scout must prove both commands ran"; §17 "Nothing is deleted dirty".)*
10. **Confirm, reject, edit and show.** The user never writes the profile by hand (spec §6).
    - `anthrex profile confirm [--yes]` prints the `Ready` proposal (the `show` text of Interfaces, CLI) and asks `store this profile for <project>? [y/N]` unless `--yes`. It then writes `profile.toml` and `profile.meta.json` with the fingerprint computed now, and deletes `proposal.json`. It is refused unless the proposal is `Ready`.
    - `anthrex profile reject` kills a running scout (`WindowManager::headless_kill`), removes any detection checkout (salvaged first), and deletes `proposal.json`.
    - `anthrex profile edit <key> <value>` and `anthrex profile edit --unset <key>` correct one value. The key is one of `RepoProfile`'s fields, or `env.<NAME>` for one environment entry. The value is parsed as a TOML value (`toml::from_str::<toml::Table>("v = <value>")`); if that fails it is taken as a string, so `anthrex profile edit check 'cargo test'` works. The edit is applied to the stored profile (refused when there is none: `no stored profile for <project>; run anthrex profile detect first`) and becomes a new proposal. If it changed `setup`, `check`, `check_timeout_secs`, `single_test`, `test_passed`, `sample_test` or `env`, the proposal goes to `Verifying` with all three commands re-run (decision 9); otherwise it goes straight to `Ready`, carrying the stored verification. `--yes` confirms it automatically once `Ready`, but only if nothing the edit touched was dropped. An edit is refused while another proposal is in progress. A key that is not a `RepoProfile` field (`cache_dirs`, `confined_network`, …) is refused as `unknown key <key>; one of <keys>`.
    - `anthrex profile show [--proposed] [--json]` prints the stored profile, or the proposal, as TOML, followed by the verification.
    - `anthrex profile status [--json]` prints `ProfileStatus`.

    Every request answers at once. Verification and scouting run in the background, and progress is visible through `status`. *(Spec §6 "correct a value with `anthrex profile edit`", "re-run detection with `anthrex profile detect`".)*
11. **Detection across a daemon restart.** When the daemon starts, `ProfileService::restore` runs after `RunService::restore` and before the socket binds:
    - every `proposal.json` in `Preparing`, `Scouting` or `Verifying` is marked `Failed { reason: "the daemon restarted during detection; run anthrex profile detect" }`;
    - `<wt>/runs/.onboarding` and `<wt>/runs/.profile-verify`, if present, are salvaged and removed with their repositories;
    - every restored headless window whose `run` is `None` and whose spec's `mcp.role` is `Scout` is removed with `WindowManager::remove` (M8a's `remove_stale_windows` skips windows with no run, `driver/restore.rs:298`).

    *Refreshed:* the old decision said M8a's reconcile kills a leftover scout process by its session id. It does not: reconcile's killer considers only the pids in a run's `run.json` (F3 B-6), and a scout has none. M8b adds **no** process-kill code (the process-kill safety rule): a leftover scout lost its stdin with the old daemon, so a Claude scout ends with its turn and a Codex scout with its one turn; its sandbox is read-only and its network off, and its tool call reaches no scout (`unknown scout <id>`). Recorded as a follow-up. The scout is **not** resumed, unlike run sessions (spec §17): detection writes nothing but its own proposal and is cheap to repeat. `Ready` and `Failed` survive a restart unchanged.

### Scouts

12. **Scout sessions are M8a headless sessions, launched read-only.** `scout::spec::headless_spec(scout: &ScoutSpec, ctx: &ScoutContext) -> HeadlessSpec` builds the session, and `ScoutService` registers it with `WindowManager::create_headless(name, spec, SessionArg::New { uuid }, first_turn, project, worktree)`:
    - **Route.** `scout::spec::route(roster, runtime, strength, effort)`: the first roster entry of `runtime` at the lowest strength at or above `strength`, else the same on the peer runtime, else the first entry of `runtime`. `runtime` is `scouts.runtime`, else `orchestrator.default_runtime`; `strength` and `effort` are `scouts.strength` (`fast`) and `scouts.effort` (`low`). Spec §4 gives scouts the fast tier at low effort.
    - **Every scout, both kinds** (*Refreshed:* M8a.1 found `--permission-mode plan` blocks an allowed MCP call in `-p`, so M8a launches reviewers `dontAsk`; M8a F1c N4 requires scouts to run "the same way, a read-only OS sandbox"):
      - `claude_permission_mode = Some(REVIEWER_PERMISSION_MODE)` (`"dontAsk"`) and `claude_disallowed_tools = REVIEWER_DISALLOWED_TOOLS` (`Edit`, `Write`, `NotebookEdit`);
      - `claude_sandbox = Some(ClaudeSandbox { writable_roots: vec![], deny_write: protected_write_denials(&cwd, &[]) })` **always**, whatever `[orchestrator] worker_sandbox` says, as for a Claude reviewer: `Bash` can write nothing, and M8a's sandbox pins keep its network, Unix sockets and local binding off;
      - `codex_sandbox = "read-only"`, `codex_writable_roots` empty; `codex_args` adds `CODEX_SANDBOX_PINS`;
      - `codex_config_guard`: for a Codex scout when the caps' `codex_project_config()` is `Loaded`, the `.codex` entries of the commit the scout reads (`run::git::codex_config_tree`), else `None`;
      - `mcp = Some(McpTarget { role: Scout, run_id, task_id: None, scout_id: Some(id) })`, with `run_id` empty for a repository-level scout;
      - `env` empty (M8a's session scrub and `credential_scrub` apply as for every session);
      - `claude_auth` from `[orchestrator.claude]` (always `login`: `api_key` is refused at config load, F2 C-I3), `api_key_helper: None`.
    - **Area scout** (`ScoutKind::Area`, M9's):
      - `cwd` is `ScoutSpec.cwd` (M9 chooses it);
      - `instructions` is `SCOUT_CONTRACT`;
      - `allowed_tools` is `mcp__anthrex__submit_scout_report`, `Read`, `Glob`, `Grep`, plus `WebFetch` and `WebSearch` when `ScoutSpec.web`;
      - `run_ref = Some(RunRef { run_id, task_id: None, role: Scout, session: 1 })`.
    - **Onboarding scout** (`ScoutKind::Onboarding`):
      - `cwd` is the disposable checkout;
      - `instructions` is `ONBOARDING_CONTRACT`;
      - `allowed_tools` is `mcp__anthrex__submit_scout_report`, `Bash`, `Read`, `Glob`, `Grep`;
      - `run_ref = None`.
    - **Only the user's settings load** (M8a decision 53). `claude_args` passes `CLI_CAPS.claude_user_settings_only` on every launch, so scouts get it without doing anything. When the caps give `None` (in tests, `ANTHREX_TEST_NO_SETTING_SOURCES=1`), the project-settings check runs before a scout starts: `run::git::project_settings(git, root, base_sha, claude, codex_paths, timeout)` with `claude` true for a Claude scout and `codex_paths = Some(CLI_CAPS.codex_project_config_paths)` for a Codex scout whose caps say `Loaded`. If it finds paths, `anthrex profile detect` is refused with `this repository has project settings that headless sessions would run without asking: <paths>; review them, then detect again with --trust-project`, and `--trust-project` accepts them (they are recorded in `ProposalRecord.trusted_project`). Automatic detection (decisions 7 and 22) is refused the same way; the message is stored as the proposal's `Failed` reason. With the shipped `CLI_CAPS` a Claude scout never needs it, and a Codex scout does whenever the repository tracks `.codex/config.toml` or `.codex/hooks.json`.
    - **Window name:** `scout/<scout id>`.
    - **Refusals.** Decision 49's refusals apply to scout windows as to any headless window (`server/headless_guard.rs`). For one with `run == None`, `manager::headless::control_refusal` returns `window <id> is a headless scout session; only the daemon drives it. Use anthrex profile reject to stop it`. *(Spec §4 table row Scout, read-only launch; M8a F1c N4.)*
13. **`submit_scout_report` and the report.** The tool's schema is in Interfaces. `scout::report::validate(args, kind) -> Result<ScoutReportArgs, String>` checks it again on the daemon side and rejects with `invalid arguments: <field>: <problem>`. `profile` is required for `Onboarding` and refused for `Area`. An accepted report becomes `proto::ScoutReport` and is written atomically:
    - repository-level scouts: `<repo_dir>/scouts/<scout id>.json`;
    - run scouts (M9): `<data_dir>/runs/<run>/scouts/<scout id>.json`.

    Scout ids match `^[a-z0-9][a-z0-9-]{0,47}$`, and an onboarding scout's id is `onboarding-<unix secs>`. `ProfileMeta.report` names the report a confirmed profile came from. In a task's `scout_refs`, the alias `onboarding` resolves to that report. The tool replies `Report recorded. You are done; end your turn now.` A second report is refused: `a report for scout <id> was already recorded`. So are a report from a window that is not that scout's (`this window is not scout <id>`), a report for a finished scout (`scout <id> is <state>`), and one for a scout the daemon does not know (`unknown scout <id>`). A summary of 8000 characters is about the spec's "≤ ~2k tokens". *(Spec §4 table, §14 item 1.)*
14. **The scout lifecycle** is a pure machine, `scout::machine::step(ScoutMachine, ScoutEvent, &ScoutLimits) -> (ScoutMachine, Vec<ScoutEffect>)`, driven by `ScoutService`:
    - **Starting.** The first turn is delivered by `create_headless`: `onboarding_first_turn`, or the question for an area scout.
    - **Turn ends.** A `TurnEnded` without an accepted report sends `SCOUT_NUDGE` once (`headless_send`). A second one fails the scout: `the scout ended two turns without a report`.
    - **Process exit.** `ProcessExited` without a report fails it: `the scout's process exited without a report (code <c>)`.
    - **Tool budget.** Every `ToolUse` counts. At `scouts.max_tool_calls` the machine sends `scout_wrap_up` once; at 1.5 times that it kills the scout and fails it: `the scout used <n> tool calls without a report`.
    - **Timeout.** `scouts.timeout_secs` after the start, the scout is killed and failed: `the scout ran longer than <n> s`.
    - **Report accepted.** The machine emits `Finished(Ok)`, then `CloseStdin` (`headless_retire`). `KillAfter(INTERRUPT_GRACE)` and `RemoveAfter(RETIRE_AFTER)` follow (`headless_kill`, `remove`), as M8a decision 52 retires a reviewer.

    Usage from every `TurnEnded` is summed into the report's `usage`. `ScoutService` subscribes to `WindowManager::signals()` and forwards only its own windows' events (the engine ignores signals of windows no round has). It holds its table under `daemon::lock` and releases the lock before any manager call.
15. **MCP plumbing, all additive.** `headless::McpTarget`, `mcp::McpOptions` and `proto::ToolCall` each gain `#[serde(default)] scout_id: Option<String>` (a plain field on `McpOptions`). `headless::argv::mcp_args` writes role `scout`, omits `--run` when `run_id` is empty, and appends `--scout <id>` when it is set. `anthrex mcp` (`crates/cli/src/mcp_cmd.rs`) accepts `--role scout` and `--scout <id>`; `--run` becomes optional for role `scout` only (an empty string when absent), and for the other roles its absence is still an error. `mcp::tools::{tools_for, allowed, role_name}` gain the `Scout` arms: `tools_for(Scout)` is `[submit_scout_report]`. `mcp::forward` puts `scout_id` into the `ToolCall`. `RunService::request(Tool)` routes a call whose `role == Scout` to `ScoutService::tool`, and every other call to the engine as before.

### Deciders

16. **The decider call** is `decider::call::decide(ctx: &DeciderContext, request: &DeciderRequest) -> Decision`, which is async. It uses M8a's `HeadlessHandle::spawn` (blocking, so on `spawn_blocking`) with an `on_event` callback that feeds a channel; the parsers are the session layer's own. A decider is a one-turn headless session with no window and no session kept.
    - **Program.** `ANTHREX_DECIDER_BIN`, if set and non-empty, read once at daemon start by `ManagerConfig::from_vars` into a new `decider_bin: Option<String>`. Otherwise the resolved `claude_bin` or `codex_bin` from `ManagerConfig`, by `deciders.mode`.
    - **Working directory.** `<data_dir>/deciders/cwd`, an empty directory created at start. A decider gets everything it needs in its prompt, and running outside the repository keeps the repository's `.claude/settings.json` hooks, `.mcp.json` servers and `.codex/` config out of it.
    - **Environment.** No `env`; `remove` is `headless::credential_scrub_for(runtime, config::ClaudeAuth::Login)` (M8b splits M8a's `credential_scrub(spec)` into this and a one-line wrapper), so every API credential is removed and the user's login is used.
    - **Claude argv** (`decider::argv::claude_decider_args`, pure):
      - `-p --input-format stream-json --output-format stream-json`, then `--verbose` when `caps.claude_verbose`;
      - `--permission-prompts none` when `caps.claude_permission_prompts`;
      - the flags in `caps.claude_user_settings_only`, when `Some`: only the user's settings load, and `--strict-mcp-config` with no `--mcp-config` gives it no MCP server;
      - `--permission-mode dontAsk` (*Refreshed:* was `plan`; M8a.1 found plan mode misbehaves under `-p`, and every M8a headless role uses `dontAsk`);
      - `--disallowedTools Bash,Edit,Write,NotebookEdit,Agent,WebFetch,WebSearch,Read,Glob,Grep`;
      - `--max-turns 3` when `DECIDER_CAPS.claude_max_turns`;
      - `--json-schema <schema JSON>` when `DECIDER_CAPS.claude_json_schema`;
      - `--no-session-persistence` when `DECIDER_CAPS.claude_no_session_persistence`;
      - `--model <model>` when the route names one;
      - `--effort <e>` when `caps.claude_effort_flag`.

      No `--settings` (no hooks: a decider has no window), no `--bare`, no `apiKeyHelper` (*Refreshed:* `auth = "api_key"` cannot be configured since F2 C-I3). The prompt is one stream-json user message (`claude_stream::user_message(prompt, None)`) written to stdin, which is then closed.
    - **Codex argv** (`codex_decider_args`, pure): `exec --json --skip-git-repo-check`, `--ephemeral` when `DECIDER_CAPS.codex_ephemeral`, then `caps.codex_user_config_only` when `Some`, then `-s read-only`, `CODEX_SANDBOX_PINS` as `-c` pairs, `-c approval_policy="never" -c model_reasoning_effort=<toml e>`, `--output-schema <schema file>` when `DECIDER_CAPS.codex_output_schema`, then `-m <model>` when the route names one, then `--`, then the prompt as the last argument. Schema files are written once to `<data_dir>/deciders/schemas/<kind>-<fnv1a64 of the schema, 16 hex>.json`.
    - **Route.** `scout::spec::route` with `deciders.strength` (`fast`) and `deciders.effort` (`low`), on runtime `claude` or `codex` by mode.
    - **Answer**, taken from the session's events in this order:
      1. a `SessionEvent::StructuredOutput { value }`, which M8b adds to `ClaudeStream::parse_line` if M8b.1 finds the answer in `result.structured_output`, and which M8a's consumers treat as `Other`;
      2. otherwise a top-level `ToolUse` named `StructuredOutput`, taking its `input`, if M8b.1 finds that form;
      3. otherwise the last top-level `AssistantText` (`parent == None`), trimmed, with one surrounding fenced code block removed, parsed as JSON.

      `usage` comes from `TurnEnded`.
    - **Bounds.** The call waits for `TurnEnded` or `ProcessExited`, bounded by `deciders.timeout_secs`; on timeout it calls `handle.kill(Duration::from_secs(2))`, and on every early return it kills the process group the same way. At most 256 KiB of assistant text is kept.

      Every failure returns the fallback answer with `source: Fallback` and one of these reasons, exactly:
      - `deciders are off`
      - `the decider could not start: <error>`
      - `the decider timed out after <n> s`
      - `the decider exited before answering (code <c>)`
      - `the decider's turn failed: <error>`
      - `the decider's answer is not JSON: <error>`
      - `the decider's answer does not match the schema: <error>`
      - `no reader slot was free within <n> s` (decision 18)
      - `sized by triage` (decision 19)

    A decider never blocks a run, and a fallback is never an error. *(Spec §4 "Deciders", "Every decider has a deterministic fallback"; §23 "Deciders are a second model surface".)*
17. **Four decider kinds.** Schemas and exact prompts are in Interfaces. `decider::parse::parse(kind, &Value) -> Result<DeciderAnswer, String>` validates by hand against the same limits as the schema; no JSON-schema crate is added. `decider::fallback::fallback(&DeciderRequest) -> DeciderAnswer` gives the deterministic answer:

    | Kind | Asked | Fallback |
    |------|-------|----------|
    | `triage` | kinds, scale, and for `single` one task | kinds `[code]`, scale `plan` (spec §5.1: "Without a decider, the path is plan") |
    | `size_check` | a size and reason per task id | every task keeps the engine's size |
    | `check_summary` | 1–40 lines summarising a failed check | `run::exec::summary(tail)`, M8a's last 40 lines |
    | `blocked_reason` | `question`, `mis_sized` or `environment` for a free-text reason | `question` (M8a decision 32's default) |

    Each prompt is capped at 128 KiB. Inputs are cut in a fixed order with the marker `[anthrex] … cut …`, so the same input always gives the same prompt.
18. **Deciders inside a run are engine ops, and take reader slots.** The reducer never calls a decider. It emits `OpKind::Decide { request }`, and the driver answers `OpResult::Decided(Decision)`.
    - **Slots.** A decider op holds one of the run's reader slots while in flight, beside live reviewers, and `schedule::readers_busy` (so `RunInfo.readers_busy`) counts both. Queued deciders are started before queued reviewers whenever a slot frees.
    - **Slot wait.** A decider still queued `deciders.slot_wait_secs` (default 30) after it was queued is answered on the next `Tick` by its fallback, with reason `no reader slot was free within <n> s`. A bounce can therefore wait at most that long for a slot.
    - **Mode off.** With mode `off`, the reducer applies the fallback inline, with reason `deciders are off`, and emits no op.
    - **Reconcile and resume.** Reconcile maps `Decide` to `NotStarted`. On restore (M8a decision 45), a task whose `Decide` op was dropped queues it again (`engine/restore.rs`).
    - **Accounting.** Every `Decided` adds to `Run.decider_calls` and, for a fallback, `Run.decider_fallbacks`. Usage goes to the task's and the run's decider usage (decision 29), and a report line records the source.

    Triage is the one decider outside a run (decision 22), so it takes no slot. *(Spec §13 item 3 "scouts, reviewers and deciders use `max_readers`".)*
19. **The size cross-check, §7.2 rule 5.**
    - **When.** It runs on `Start` for every task, and on every accepted `Edit` for the tasks the batch added or amended (M8a decision 13's touched set).
    - **Evidence.** A task's evidence is the scout reports its `scout_refs` name (run scouts first, then the alias `onboarding`). A task with no `scout_refs` uses the onboarding report of the stored profile, if there is one. Tasks with no evidence get the note `size cross-check skipped: no scout evidence` and no call.
    - **One call per batch.** The evidenced tasks go into one `SizeCheck` request (at most 50 tasks). Each task's `size_check` becomes `Pending`, and a pending task is **not runnable** (`engine/schedule.rs`).
    - **Raises.** When the answer arrives, each task whose answered size is larger than its engine size is raised, since this rule, like the other §7.2 rules, can only raise:
      - **S → M.** `run::engine::deciders::apply_raise` sets `size = M` and `raised_size = Some(M)` (so an amend never lowers it, as for M8a's rung-3 raise), calls `ladder::reresolve` (made `pub(crate)`) for the review level, reviewer and a budget the plan did not set, and — the task has not been dispatched, since a pending check blocks dispatch — takes the re-resolved route's effort when the plan's `route.effort` is `None`. It adds the note `size raised from S to M: decider cross-check (rule 7.2.5): <reason>`.
      - **To L.** The task becomes `blocked(mis_sized)` with the text `the size cross-check judged this task L: <reason>; split it (rule 7.2.5)`. No rung is counted.
      - **No raise.** A smaller or equal answer changes nothing.
    - **Recording.** Every answered task records `SizeCheckInfo { engine, decided, agreed, reason, source }`, and a disagreement is written to the report.
    - **Missing tasks.** A task missing from the answer keeps its size, with `source: Fallback`.
    - **Fast path.** The single task of a fast-path run is not cross-checked. The triage decider sized it from the same evidence a moment earlier, so asking again would ask the same model the same question. It gets `SizeCheckInfo { source: Fallback, reason: "sized by triage" }`. *(Spec §7.2 rule 5.)*
20. **The check summary, §10 and §11.3.**
    - **Task check.** When a task's check fails (`OpResult::Check { ok: false }`, `engine/gates.rs`), the reducer records the `CheckRecord` and counts the failure on the ladder (M8a decision 38) as before. It then **defers the rung's action**: it stores `Task.pending_failure = Some(PendingFailure { gate: Check, rung, check_index, decider_id })` and queues a `CheckSummary` decider.
    - **Candidate check.** A red merge candidate (`OpResult::CandidateRed`, `engine/merge.rs`) does the same with gate `Merge`. The merge queue moves on at once.
    - **Resuming the action.** On `Decided`, the check record gets `summary` and `summary_source`. The deferred action then runs. `check_failed_message` and `candidate_red_message` (`run/contract.rs`) use the summary when the record has one: the line `Last 40 lines:` becomes `Summary of its output:` followed by the summary lines; with a fallback (or no summary) they are M8a's text exactly. While deferred, the task keeps its state (`Check`, or `MergeQueue` after a red candidate, outside the queue).
    - **Reviewers.** The reviewer prompt's `Last check (40 lines):` block uses the latest check's summary when one exists.
    - **Proof failures** keep M8a's command and 40-line tail, because spec §23 requires a proof failure message to show the command and its output. *(Spec §10 rung 1 "the decider's ≤ 40-line check summary", §11.3, §14.3 last sentence.)*
21. **Classifying a free-text `task_blocked`, §10.**
    - **Without a kind.** `engine/tools.rs` now reports whether `kind` was given. A `task_blocked` whose `kind` is absent makes the task `blocked(question)` at once, as in M8a. It adds `pending_classification = true` and queues a `BlockedReason` decider. The reply becomes `Blocked recorded (classifying). Stop and wait for an answer.`
    - **On `Decided`:**
      - `question` keeps `blocked(question)`;
      - `environment` changes the block reason to `environment`;
      - `mis_sized` applies rung 3 exactly as a typed `mis_sized` would (M8a decision 38).
    - **Recording.** `TaskInfo.block_source` records `Decider` or `Fallback`.
    - **A typed kind** is never reclassified; `block_source` stays `None`.
    - **Answered first.** An `answer` edit that arrives before the classification clears it and proceeds as M8a does. *(Spec §10 "A decider classifies free-text reasons the worker did not type".)*

### Triage and the fast path

22. **`anthrex run start --goal "<text>" [--yes] [--trust-project] [--unconfined-checks]`** sends `RunRequest::StartGoal { goal, dir, yes, trust_project, unconfined_checks }`. `RunService::request` does everything below before any engine event, with no lock held across an await (`driver/adapt.rs`):
    1. **Early refusals.** `confine::start_refusal` exactly as `build` applies it, then `run::git::preflight` on `spawn_blocking`.
    2. **Profile.** `ProfileService::effective(project)` must return a stored profile (decision 6, source 1). Otherwise the request is refused with one of these, exactly, and nothing else happens:
       - `this repository has no stored profile; detection has started (anthrex profile status), then confirm it with anthrex profile confirm and start the goal again` — this also starts detection, when `onboarding.auto` is true and nothing is pending (detection's own refusals, such as decision 12's project-settings check, are then stored as its `Failed` reason);
       - `this repository has no stored profile; run anthrex profile detect, then anthrex profile confirm` — when `onboarding.auto` is false;
       - `this repository has no stored profile; detection is <state> (anthrex profile status)` — while a proposal is in progress;
       - `this repository has no stored profile; a proposal is ready: anthrex profile show --proposed, then anthrex profile confirm` — when one is `Ready`;
       - the stored-profile parse error of decision 6.
    3. **Triage.** The input is the goal (cut to 4000 characters), `profile::summary(&profile)`, the onboarding report's summary and files if one exists, and the tracked file list from `git ls-files -z` through `worktree::run_git` in `pre.root` (at most 1500 paths or 48 KiB, sorted, with `(<n> of <total>)`). The call is `decider::call::decide`; mode `off` gives the fallback at once. Triage takes no reader slot, because no run exists yet.
    4. **Route.** `run::triage::route(&decision, fast_path_enabled) -> TriageRoute` (decision 23).
    5. **Fast.** The task becomes a one-task `Plan` (`goal`, `tasks: [t1]`, the stored profile's spec) and goes through **exactly M8a's start path**: `driver/requests.rs::build` is split into `build` (parses the TOML) and `build_plan(plan, dir, yes, trust_project, unconfined_checks)`, which runs everything `build` did — preflight, decision 6's choice, the protected files, the id, `build_run` with `BuildContext.yes = true`, `unconfined_checks`, the session nonce, the Codex branch and `.codex` tree, the runtime checks of decisions 50 and 53 with `trust_project`.
       - If `build_plan` fails with `build_run`'s errors, or the resolved task is `hub`, or its size is L, the route becomes `Plan` with the reason `the fast path does not apply: <first PlanError or "task t1 touches a hub file" or "task t1 is L">`. Any other refusal of `build_plan` (settings, confinement, a ref) is the request's refusal, as for `run start --plan`.
       - Otherwise the run gets `path: Some(Fast)`, `triage: Some(..)` and `approved_by: Some("fast path")`, and `Event::Start` is sent. The reply is `RunReply::Triaged { triage, run_id: Some(id), message }`.
    6. **Plan or Large.** Nothing is created: no branch, no run directory, no window. The reply is `RunReply::Triaged { triage, run_id: None, message }`, with `run::triage::refused_message` (Interfaces).

    The CLI waits `GOAL_REQUEST_TIMEOUT` = 810 s, derived as `RUN_REQUEST_TIMEOUT` (180 s) + the largest configurable `deciders.timeout_secs` (600 s) + 30 s for `git ls-files` (its bound is `git_timeout_secs`, at most 60 s by M8a's range, of which the default 60 s is covered by the 180 s term's own slack; the implementer records the exact sum in `docs/timing-budgets.md` from the landed call count). *(Spec §5.1. Refreshed: `unconfined_checks` and the shared `build_plan` path, so the fast path runs every M8a start check.)*
23. **Triage routing** (`run::triage::route`, pure), in order:
    1. The decision's `source` is `Fallback`: route `Plan`, with reason `triage fell back (<fallback reason>); without a decider the path is plan`.
    2. `fast_path` is false in config: `Plan`, `the fast path is disabled ([orchestrator] fast_path = false)`.
    3. Scale `large`: `Large`, with the decider's reason.
    4. Scale `plan`: `Plan`, with the decider's reason.
    5. Scale `single`, but `kinds` is not exactly `[code]` or `[docs]`: `Plan`, `a single-task goal of kinds <k> is not a fast-path goal`. Research and review goals need milestone 9's scouts and reviewer pipelines.
    6. Scale `single`: `Fast(PlanTask)`. The task is `id = "t1"`, the decider's title, brief, acceptance, owns, size, `interface_change`, `test_mode` and `test_mode_reason`, `test_to_write`, `kind` = the single kind, and every other field defaulted. M8a's validation, run next, decides hub, L and the test-mode rules.

    `TriageInfo` records kinds, scale, the chosen `RunPath`, the reason, the source and the fallback reason. *(Spec §5.1 table.)*
24. **A fast-path run** is an ordinary M8a run with one task and `path = Fast`. Its only differences:
    - it starts `running`, with no `awaiting_approval` state ever;
    - the report's header line reads `path: fast (triage: <kinds>/<scale>, <source>)`;
    - `anthrex run status` shows `fast path` after the state.

    Everything else holds exactly: the gates, review, the ladder, the merge queue, `run accept` and `run discard`. A fast-path task that reaches rung 3 is `blocked(mis_sized)` like any task, and the user retries, edits, cancels or promotes it. *(Spec §5.1 "There is no plan gate; the user still accepts or discards the result".)*
25. **`anthrex run promote <run>` in M8b** sends `RunRequest::Promote { run_id }`, which becomes `EventKind::Promote`. The reducer:
    - on a run with `path == Some(Fast)` that is not terminal and not yet promoted:
      - sets `Run.promote_requested_at = Some(now)`;
      - adds the log line `promotion to a planned run requested by the user`;
      - adds the attention line `promotion requested at <hh:mm>; it takes effect when the orchestrator exists (milestone 9)`;
      - persists and publishes;
      - replies `Ok` (`Done`) with `recorded: run <id> is marked for promotion to a planned run. Until the orchestrator exists (milestone 9) nothing else changes: the fast-path task continues and the run finishes as a fast-path run.`
    - Already promoted: `Ok` with `run <id> was already marked for promotion at <hh:mm>`.
    - A run that is not fast-path: `Err` (`Refused`) with `run <id> is not a fast-path run`.
    - A terminal run: `Err` with `run <id> is <state>`.

    Nothing else changes: no task, no scheduling, no window. Milestone 9 reads `promote_requested_at` and performs the promotion. *(Spec §5.1 "`anthrex run promote` turns it into a planned run at any time"; the orchestrator it needs is M9's.)*

### The output filter

26. **Filter modes**, `output_filter::apply(mode, lines: &[String], exit_code: i32) -> Vec<String>`, pure. Every kept line is cut to 500 characters.
    - `none`: every line.
    - `tail`: the last 60 lines.
    - `failures-only`:
      - **Exit 0:** the last 10 lines.
      - **Otherwise:** every line matching `FAILURE_RE` is kept with the 5 lines after it, overlapping ranges merged, and at most the first 100 lines kept that way. The last 20 lines are then added, and each gap between kept ranges becomes one line `[anthrex] … <n> lines omitted …`.
      - **No line matches:** the last 60 lines.

    `FAILURE_RE` is `(?i)\b(fail(ed|ure|ures|s)?|error(s)?|panic(ked|s)?|assert(ion)?|expected|traceback|exception)\b`.
27. **`anthrex filter-run --mode <m> --log-dir <dir> -c <command>`** is dispatched before clap, like `hook`. It:
    - runs `/bin/sh -c <command>` as its own child, with stdin inherited and stdout and stderr on one pipe;
    - writes every byte to `<dir>/<unix ms>-<pid>.log`, creating `<dir>`, and stops writing at 64 MiB with a final line `[anthrex] log cut at 64 MiB`;
    - keeps every line up to 20 000 and then the last 5 000;
    - prints `apply(mode, lines, code)` to stdout, then `[anthrex] full output: <log path> (<n> lines, exit <code>)`;
    - exits with the child's code, or 128 + the signal number if a signal killed it.

    A failure to create the log does not stop the command: it runs, unfiltered, with the same exit code, and `[anthrex] could not write the log: <error>` goes to stderr. filter-run never changes what the command does, only what the agent reads. *(Spec §14.3 "cutting logs from tens of thousands of tokens to hundreds".)*
28. **`anthrex filter-hook --mode <m> --log-dir <dir> [--prefix <p>]…`**, dispatched before clap, is a `PreToolUse` hook command.
    - **Input.** It reads the payload from stdin (at most 1 MiB) under `FILTER_HOOK_DEADLINE` = 1 s. It always exits 0 and never writes to stderr.
    - **When it rewrites.** Only when `tool_name == "Bash"` and `tool_input.command`, after stripping leading `cd <non-space> && ` segments and leading `NAME=value ` assignments, starts with one of the prefixes followed by the end or whitespace, and does not already contain ` filter-run `.
    - **What it prints then:**

      ```json
      {"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{…every original tool_input key…,"command":"'<exe>' filter-run --mode <m> --log-dir '<dir>' -c '<original command, shell_quote'd>'"}}}
      ```

      It adds `"permissionDecision":"allow"` only if M8b.1 finds `updatedInput` ignored without it (`output_filter::HOOK_SETS_ALLOW`). That is safe because the hook only rewrites a `Bash` call, which the worker's `--allowedTools` already allows. Otherwise it prints nothing.
    - **Prefixes.** `filter_prefixes` from the profile. When that is empty, they are derived:
      - the text of `single_test` before `{test}`, cut to its first two words;
      - the first two words of each segment of `check` split on `&&`, `||` and `;`;
      - de-duplicated, in order.
    - **Injection.** `HeadlessSpec` gains `#[serde(default)] output_filter: Option<FilterHook { mode, prefixes, log_dir }>`. `headless::argv::claude_args`, right after it calls M8a's `claude_settings`, calls `output_filter::add_hook(&mut settings, exe, spec.output_filter.as_ref())`, whose signature is unchanged. That adds a **second** matcher group: `"PreToolUse": [<M3's group unchanged>, {"matcher":"Bash","hooks":[{"type":"command","command":"<filter-hook command>"}]}]`. The sandbox block (its writable roots, `denyWrite` and pins) and the user-settings flags are untouched.
    - **Where the log goes** (*Refreshed:* the old decision added a new writable root `<data_dir>/runs/<run>/logs/<task>` to the worker's sandbox). `log_dir` is `<task TMPDIR>/anthrex-logs`, where the task `TMPDIR` is `role_launch::task_tmp_dir(&run.data_dir, task_id)` — the short per-task directory M8a F1d already grants the worker and sets as its `TMPDIR`. So the grant is unchanged (no new writable root, and no grant computed from a new path, M8a F1c I1), `filter-run` inside the sandbox can write there, the log is outside the checkout (it never appears as an untracked file or trips the clean-tree check of `task_done`), and it is removed with the task's checkout at accept or discard.
    - **Who gets it.** M8a's `worker_spec` sets it for Claude workers when the run's profile source is `Stored`, its `output_filter` is not `none`, and there is at least one prefix. Reviewers, scouts, deciders and Codex sessions never get it. *(Spec §14.3 "A `PreToolUse` hook, injected with `--settings` (hooks still run in `-p` mode)".)*

### Metering

29. **Usage by role.** M8a already meters every headless round from its stream (`AgentRoundInfo.usage`, `Spend.tokens`). M8b adds:
    - `Task.decider_usage` and `Run.decider_usage`, summed from each `Decided`;
    - `Run.triage_usage`;
    - `Run.scout_usage`, for run scouts (M9);
    - `Run.orchestrator_usage`, from OTLP (decision 30).

    `run/snapshot.rs` fills `RunInfo.usage = Some(RunUsage { total, by_role, decider_calls, decider_fallbacks })`, where `by_role` has the keys `worker`, `reviewer`, `scout`, `decider` (triage included) and `orchestrator`, each a `TokenUsage`, and `total` is their sum. `TaskInfo.decider_usage = Some(..)`. A repository-level scout's usage is stored in its report and in `ProfileStatus`. These are counters, coalesced to one push per second by M8a decision 47. *(Spec §14.8.)*
30. **The OTLP receiver**, ready for milestone 9's orchestrator (the only PTY run session, spec §14.8).
    - **Binding.** With `metering.otlp = true`, `lifecycle::run` binds `127.0.0.1:<metering.otlp_port>` (0 picks a free port) and writes `http://127.0.0.1:<port>` to `<data_dir>/otlp.addr`, which is removed at shutdown.
    - **Accepted requests.** `POST /v1/metrics` with `Content-Type: application/json`, a `Content-Length` or chunked body of at most 4 MiB, and headers of at most 16 KiB, all read within 10 s.
    - **Answers.** `200` with body `{}`; `404` for another path; `415` for any other content type, logged once per daemon; `413` or a closed connection for oversize or slow requests. Each connection is its own task, so one slow client never delays another.
    - **Parsing.** `metering::otlp::parse_metrics(body) -> Result<Vec<UsagePoint>, String>` reads every sum data point of the metric `claude_code.token.usage`: the attribute `type` (`input`, `output`, `cacheRead`, `cacheCreation`), `aggregationTemporality` (1 delta, 2 cumulative), `session.id`, `model`, and the resource attributes `anthrex.run` and `anthrex.role`. Points without `anthrex.run` are ignored.
    - **The ledger.** `OtlpLedger` adds delta points. For cumulative ones, per series `(session.id, model, type)`, it adds the increase over the last value, or the value itself when it went down (a reset). It keeps totals per `(run, role)`.
    - **Into the engine.** After each accepted request, the server sends `EventKind::OrchestratorUsage { run_id, usage }` with the ledger's new total for `(run, "orchestrator")`, and the reducer stores it as `Run.orchestrator_usage`. Any other role is kept in the ledger and not shown.
    - **Who can post.** Any local process can reach a loopback port, but M8a's confinement denies confined checks, proofs and `setup` every loopback port the user did not list (F1d round 2), and M8a's Claude sandbox pins deny sandboxed sessions local networking. Metering is informational and never gates anything; keep it that way.
    - **For milestone 9.** `metering::orchestrator_env(addr, run_id) -> Vec<(String, String)>` gives the variables the orchestrator window needs. Their exact names and values are fixed by M8b.1 (Interfaces lists the documented set). *(Spec §14.8 "The orchestrator, the one PTY session, is metered through Claude Code's OTLP export with `anthrex.run` and `anthrex.role` resource attributes".)*

### History

31. **Phase times.** `Task` gains `phase_since: u64` and `phases: PhaseSecs { queued, preparing, working, proof, check, review, merge, blocked }`. M8b introduces `run::phases::set_state(task: &mut Task, state: TaskState, now: u64)`. It adds `now - phase_since` to the field of the old state (`Pending` and the finished states count nowhere), sets `phase_since = now`, sets `max_rung = max(max_rung, rung)`, and assigns the state. Every assignment of a task's `state` outside tests is replaced by it: on `baa04e1` there are 21, in `run/edits.rs` (4), `engine/complete.rs` (2), `engine/dispatch.rs` (4, one `= next`), `engine/gates.rs` (1, `= state`), `engine/holds.rs` (3), `engine/ladder.rs` (2), `engine/merge.rs` (3) and `engine/requests.rs` (2). The acceptance criterion greps that no other assignment remains. This extends spec §15's five phases with `preparing`, `proof` and `blocked`, because a task spends real time in each.
32. **Actual size by diff.** `OpKind::MeasureDiff { root, from, to, three_dot }` runs `git diff --numstat` and `git diff -U0` in `root` (the user's checkout, `Run.root`) through `worktree::run_git` (reads, no queue), bounded by `git_timeout_secs`. It answers `OpResult::DiffMeasured(DiffStats { files, hunks, added, removed })`, where `hunks` counts the lines starting with `@@` and binary files count as files with 0 lines. Every commit named is in the user's object store: a merge commit, the run head, or `Task.head`, which M8a sets only from an imported claim (a worker's unimported commits are never named).
    - **A merged task:** `from` is the run head before its merge, `to` its merge commit, `three_dot = false`. That is exactly what the task contributed, and it stays correct after a hand-back.
    - **Cancelled or unfinished with a recorded head:** `from` = the run head, `to` = `Task.head`, `three_dot = true`.
    - **No recorded head:** no op, `diff = None`.

    Reconcile maps it to `NotStarted`. *(Spec §15 "Actual size is measured by diff, not tokens".)*
33. **`history.jsonl`.** One JSON line per record, at `<repo_dir>/history.jsonl` (decision 4).
    - **Types.** `proto::history::HistoryLine`, tagged `type`, with the variants `task`, `run` and `revert` (Interfaces). Every record carries `v: 1` and a `record_id`: `<run>/<task>` for a task, `<run>` for a run, `revert/<revert commit>` for a revert.
    - **As an op.** Appends are journaled ops: `OpKind::AppendHistory { path, record_id, line }` → `OpResult::HistoryAppended`. The driver appends one line and calls `sync_all`, on `spawn_blocking`.
    - **Reconcile.** It scans the file for `"record_id":"<id>"` and gives `Replay(HistoryAppended)` if it is found, `NotStarted` otherwise. Readers still keep the last line of each `record_id`, so a duplicate can never be counted twice.
    - **Task records.** A task record is appended when a task becomes `merged` (after `DiffMeasured`) or `cancelled`. When a run reaches `accepted`, `discarded` or `failed`, or completes with unfinished tasks, every task without a record gets one with outcome `unfinished`, `blocked` or `cancelled`. Then one `run` record is appended.
    - **Filled by the driver.** For a `run` record with outcome `accepted`, the driver fills in `accepted_commit` just before appending, by resolving `refs/heads/<base>` with `run::git::read_ref`. That is the one field the pure reducer cannot know.
    - **Task fields.** `Task.history_written: bool` is set when the op is emitted.
    - **History off.** `Run.repo_dir` empty (a run restored from M8a) means no history ops for that run. *(Spec §15 list of fields.)*
34. **Revert detection.** `run::history_io::detect_reverts(git, root, base_branch, history, now, timeout)` runs on `spawn_blocking` at every `run start` (both kinds) and every `run stats`. It considers only `run` records with `accepted_commit`, and their tasks' `merge_commit`s, that are at most 90 days old and have no revert record yet. It reads `git log -n 2000 --format=%H%x1f%B%x1e <base_branch>` through `worktree::run_git`. Every commit whose message contains `This reverts commit <sha>` for one of those shas gets a `revert` record, appended with `append_line`: `task_id = Some(..)` for a task's merge commit, and `None` for the run's accept merge, which means every task of that run. A failure only logs a warning. *(Spec §15 "whether the user later reverted it", §23 "Reverts after accept … are the only true signal".)*
35. **`anthrex run stats [--json]`** sends `RunRequest::Stats { dir }`.
    - **The rows.** The daemon reads the history (keeping the last line per `record_id`) and computes `run::stats::aggregate(&lines, path) -> HistoryStats`, which is pure. It has one row per class: `S`, `M` (non-hub), and `hub`.
    - **Each row:** the task count, merged count, median lines changed (`added + removed`), median tool calls, median billable tokens, median working minutes, total bounces, and reverted count.
    - **Medians** are taken over merged tasks only. With an even count, they are the lower middle value.
    - **The totals line:** decider calls and fallbacks, and how many tasks the size cross-check checked and raised.

    Nothing is proposed or refitted; that is M9.5. The text layout is in Interfaces. *(Spec §15; "recording only" for M8b.)*

### Test doubles

36. **`ANTHREX_DECIDER_BIN` and `fake-agent`'s decider mode.**
    - **Detection.** `fake-agent` is in decider mode when its prompt's first line is `[anthrex decider] <kind> v1`: for Claude, its first stdin user message (read by the Claude headless mode before anything else); for Codex, its last argument. The prompt, not an argv flag, decides, so the mode works whatever M8b.1 finds about `--json-schema` and `--output-schema`.
    - **Scripts.** From `$FAKE_AGENT_DECIDER_DIR` it claims `<kind>-<n>.json` with the smallest `n` whose `.claimed` does not exist (`OpenOptions::create_new`, as in M8a.20). The file holds one of:
      - `{"answer": …}`: a structured answer in the fixture's form;
      - `{"text": "…"}`: raw assistant text;
      - `{"fail_turn": "…"}`: a failed turn;
      - `{"exit": <code>}`: exit before answering;
      - `{"hang": true}`: no output until killed.

      Any of them may also carry `"usage": {input, output, cache_read, cache_write}`.
    - **Records.** Every call appends `{"kind","argv","prompt"}` to `$FAKE_AGENT_DECIDER_DIR/calls.jsonl`.
    - **No script.** With no matching file, or no `FAKE_AGENT_DECIDER_DIR`, it writes `fake-agent: no scripted decider answer for <kind>` to stderr and exits 2. The caller then falls back.
    - **Shapes.** Its output uses only the shapes of M8b.1's decider fixtures, checked by M8a decision 51's shape test (`crates/fake-agent/tests/headless_shapes.rs`, `headless_support/shape.rs`).
    - **Harness default.** `RunHarness` writes `[orchestrator.deciders] mode = "off"` and `[orchestrator.onboarding] auto = false` unless the test's `[orchestrator]` lines mention `deciders` or `onboarding`, so every M8a scenario runs exactly as before. *(Spec §21 "Deciders are replaced in tests by `ANTHREX_DECIDER_BIN`".)*
37. **Other `fake-agent` additions.**
    - **Scout scripts.** For role `scout`, the task part of a script name is the value after `--scout` in the MCP server's argv (`McpServer::flag("--scout")`), so the onboarding scout claims `scout-onboarding-<secs>-<n>.jsonl`. Because the id carries a timestamp, a script named `scout-onboarding-<n>.jsonl` matches any onboarding id. Scripts are found as M8a's are: `<git common dir>/fake-agent/`, through the checkout's alternate for a standalone checkout.
    - **The `bash {cmd}` step** simulates a Claude `Bash` tool call. It runs every `PreToolUse` matcher group from `--settings` whose matcher is empty or matches `Bash` as a regex, in order. Each group's command gets the payload `{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":…},"session_id":…,"cwd":…}` on stdin, and the last `hookSpecificOutput.updatedInput.command` printed wins. The step then runs the final command with `/bin/sh -c` in the cwd. It emits the `tool_use` (with the final command) and `tool_result` pair, keeps the output as `FAKE_AGENT_RESULT`, and appends `{"original","ran","exit","output_lines"}` to `$FAKE_AGENT_BASH_LOG` when that is set.
    - **Hook discovery.** `runtime::discover` keeps every matcher group per event (`Runtime::groups(event)`). `Runtime::hook(event)` still returns the first group's first command, so milestone 3's behaviour is unchanged.

## Interfaces

Everything new is marked by its file. "(M8a's)" marks an existing file or type that gains something.

### `proto`

Everything derives `Debug, Clone, PartialEq, Serialize, Deserialize`, plus `Eq`, `Copy`, `Default` where noted. Every enum is `rename_all = "snake_case"` unless noted.

`crates/proto/src/run.rs` (M8a's), one variant:

```rust
pub enum AgentRole { Orchestrator, Worker, Reviewer, Scout }   // "scout"; M9 adds its planner role
```

`crates/proto/src/profile.rs` (new):

```rust
#[serde(rename_all = "kebab-case")] #[derive(Copy, Eq, Default)]
pub enum OutputFilter { #[default] FailuresOnly, Tail, None }   // "failures-only" | "tail" | "none"

#[serde(deny_unknown_fields)] #[derive(Default, Eq)]
pub struct RepoProfile {                                // no cache_dirs, no confined_* (decision 5)
    #[serde(default)] pub languages: Vec<String>,
    #[serde(default)] pub modules: Vec<String>,
    #[serde(default)] pub hub: Vec<String>,
    #[serde(default)] pub source: Vec<String>,
    #[serde(default)] pub generated: Vec<String>,
    #[serde(default)] pub protected: Vec<String>,        // extras only; M8a's BUILTIN_PROTECTED always applies (decision 5)
    #[serde(default)] pub setup: Option<String>,
    #[serde(default)] pub check: Option<String>,
    #[serde(default)] pub check_timeout_secs: Option<u64>,
    #[serde(default)] pub single_test: Option<String>,
    #[serde(default)] pub test_passed: Option<String>,
    #[serde(default)] pub sample_test: Option<String>,
    #[serde(default)] pub output_filter: OutputFilter,
    #[serde(default)] pub filter_prefixes: Vec<String>,
    #[serde(default)] pub conventions: Vec<String>,
    #[serde(default)] pub manifests: Vec<String>,
    #[serde(default)] pub env: std::collections::BTreeMap<String, String>,
}
impl RepoProfile { pub fn spec(&self) -> ProfileSpec; }   // decision 5

#[derive(Copy, Eq)] pub enum ProfileSource { Stored, Plan, None }
#[derive(Eq)] pub struct CommandCheck { pub command: String, pub ok: bool, pub code: Option<i32>,
                                        pub timed_out: bool, pub secs: u64, pub tail: String }   // tail: last 40 lines
#[derive(Eq)] pub struct ProfileVerification { pub at: u64, pub confined: bool,
                                               pub setup: Option<CommandCheck>,
                                               pub check: Option<CommandCheck>, pub single_test: Option<CommandCheck> }
#[derive(Eq)] pub struct DroppedCommand { pub key: String, pub command: String, pub reason: String, pub tail: String }
#[serde(tag = "state")] #[derive(Eq)]
pub enum ProposalState { Preparing, Scouting, Verifying, Ready, Failed { reason: String } }
#[serde(tag = "origin")] #[derive(Eq)]
pub enum ProposalOrigin { Detect, Auto { stale: Vec<String> }, Goal, Edit { keys: Vec<String> } }
#[derive(Eq)] pub struct ProposalRecord {
    pub project: PathBuf, pub state: ProposalState, pub origin: ProposalOrigin,
    pub started_at: u64, pub updated_at: u64,
    pub base_sha: String,                             // the commit the scout read and the commands ran at
    pub scout_id: Option<String>, pub window_id: Option<u32>,
    pub profile: Option<RepoProfile>,                 // after verification: only commands that passed
    pub verification: Option<ProfileVerification>,
    pub dropped: Vec<DroppedCommand>,
    pub proposed: Option<RepoProfile>,                // the scout's raw proposal (or the edited profile)
    pub trusted_project: Vec<String>,                 // decision 12, when --trust-project was given
    pub unconfined_checks: bool,                      // decision 9, when --unconfined-checks was given
    pub auto_confirm: bool,                           // profile edit --yes
}
#[derive(Eq)] pub struct ProfileMeta {
    pub confirmed_at: u64, pub report: Option<String>,        // scout report id, or None after an edit-only change
    pub verification: Option<ProfileVerification>,
    pub fingerprint: std::collections::BTreeMap<String, String>,   // path -> "<16 hex>:<len>" | "missing"
    pub edited_keys: Vec<String>,
}
#[derive(Eq)] pub struct ProfileStatus {
    pub project: PathBuf, pub repo_dir: PathBuf, pub source: ProfileSource,   // Stored or None
    pub confirmed_at: Option<u64>, pub stale: Vec<String>, pub unparseable: Option<String>,
    pub proposal: Option<ProposalRecord>, pub scout: Option<ScoutInfo>,
    pub verify_confined: bool,                        // whether verification would run confined here (decision 9)
}
```

`crates/proto/src/scout.rs` (new):

```rust
#[derive(Copy, Eq)] pub enum ScoutKind { Onboarding, Area }
#[derive(Copy, Eq)] pub enum ScoutState { Starting, Working, Reported, Failed }
#[derive(Eq)] pub struct ScoutFile { pub path: String, pub why: String }
pub struct ScoutReport {
    pub id: String, pub kind: ScoutKind, pub run_id: Option<String>, pub question: String,
    pub summary: String, pub files: Vec<ScoutFile>,
    #[serde(default)] pub modules: Vec<String>, #[serde(default)] pub interfaces: Vec<String>,
    #[serde(default)] pub risks: Vec<String>, #[serde(default)] pub profile: Option<RepoProfile>,
    pub route: Route, pub window_id: Option<u32>, pub started_at: u64, pub finished_at: u64,
    pub tool_calls: u32, pub usage: TokenUsage,
}
pub struct ScoutInfo {
    pub id: String, pub kind: ScoutKind, pub question: String, pub state: ScoutState,
    pub failure: Option<String>, pub window_id: Option<u32>, pub route: Route,
    pub started_at: u64, pub ended_at: Option<u64>, pub tool_calls: u32,
    pub report_bytes: Option<u32>, pub files: Vec<String>, pub usage: TokenUsage,
}
```

`crates/proto/src/adapt.rs` (new):

```rust
#[derive(Copy, Eq)] pub enum RunPath { Fast, Plan, Large }
#[derive(Copy, Eq)] pub enum Scale { Single, Plan, Large }
#[derive(Copy, Eq)] pub enum DeciderSource { Decider, Fallback }
#[serde(rename_all = "lowercase")] #[derive(Copy, Eq, Default)]
pub enum DeciderMode { #[default] Claude, Codex, Off }
#[derive(Eq)] pub struct TriageInfo { pub kinds: Vec<TaskKind>, pub scale: Scale, pub path: RunPath, pub reason: String,
                                      pub source: DeciderSource, pub fallback_reason: Option<String>, pub at: u64 }
#[derive(Eq)] pub struct SizeCheckInfo { pub engine: Size, pub decided: Option<Size>, pub agreed: bool,
                                         pub reason: String, pub source: DeciderSource }
#[derive(Copy, Eq, Default)] pub struct DiffStats { pub files: u32, pub hunks: u32, pub added: u32, pub removed: u32 }
#[derive(Copy, Eq, Default)] pub struct PhaseSecs { pub queued: u64, pub preparing: u64, pub working: u64, pub proof: u64,
                                                    pub check: u64, pub review: u64, pub merge: u64, pub blocked: u64 }
#[derive(Eq, Default)] pub struct RunUsage { pub total: TokenUsage,
                                             pub by_role: std::collections::BTreeMap<String, TokenUsage>,  // worker, reviewer, scout, decider, orchestrator
                                             pub decider_calls: u32, pub decider_fallbacks: u32 }
```

`impl std::ops::AddAssign for TokenUsage` (field-wise) goes in `crates/proto/src/run_info.rs`, beside `billable` (M8a has none).

`crates/proto/src/run_info.rs` (M8a's), new fields, each `#[serde(default)]`:

```rust
// RunInfo
pub path: Option<RunPath>, pub triage: Option<TriageInfo>, pub promote_requested_at: Option<u64>,
pub profile_source: Option<ProfileSource>, pub usage: Option<RunUsage>, pub scouts: Vec<ScoutInfo>,
// TaskInfo
pub decider_usage: Option<TokenUsage>, pub size_check: Option<SizeCheckInfo>, pub diff: Option<DiffStats>,
pub phases: Option<PhaseSecs>, pub block_source: Option<DeciderSource>,
// CheckInfo (M8a's `summary: String` stays the last-40-lines text; the decider's summary goes in the new field)
pub decider_summary: Option<String>, pub summary_source: Option<DeciderSource>,
```

`crates/proto/src/run_wire.rs` (M8a's), new variants and one field:

```rust
pub struct ToolCall { /* M8a's fields */ #[serde(default)] pub scout_id: Option<String> }
pub enum RunRequest { /* M8a's */
    StartGoal { goal: String, dir: PathBuf, yes: bool, trust_project: bool, unconfined_checks: bool },
    Promote { run_id: String },
    Stats { dir: PathBuf },
    Profile(ProfileRequest),
}
pub enum ProfileRequest {
    Status { dir: PathBuf },
    Detect { dir: PathBuf, trust_project: bool, unconfined_checks: bool },
    Show { dir: PathBuf, proposed: bool }, Confirm { dir: PathBuf }, Reject { dir: PathBuf },
    Edit { dir: PathBuf, key: String, value: Option<String>, yes: bool, unconfined_checks: bool },   // value None: --unset
}
pub enum RunReply { /* M8a's */
    Triaged { triage: TriageInfo, run_id: Option<String>, message: String },
    Profile(ProfileReply),
    Stats(HistoryStats),
}
pub enum ProfileReply {
    Status(ProfileStatus),
    Shown { source: ProfileSource, toml: String, meta: Option<ProfileMeta>,
            verification: Option<ProfileVerification>, dropped: Vec<DroppedCommand> },
    Done { message: String },
    Refused { message: String },
}
pub mod request { /* M8a's */ pub const START_GOAL: &str = "run start --goal"; pub const PROMOTE: &str = "run promote";
                  pub const STATS: &str = "run stats"; pub const PROFILE: &str = "profile"; }
```

`crates/proto/src/history.rs` (new). History types never use `deny_unknown_fields`, so M9.5 can add fields:

```rust
pub const HISTORY_VERSION: u32 = 1;
#[serde(tag = "type")]
pub enum HistoryLine { Task(TaskRecord), Run(RunRecord), Revert(RevertRecord) }
#[derive(Copy, Eq)] pub enum TaskOutcome { Merged, MergedWithoutApproval, Cancelled, Blocked, Unfinished }
#[derive(Copy, Eq, Default)] pub struct GateTally { pub proofs: u32, pub proofs_failed: u32, pub checks: u32, pub checks_failed: u32,
                                                    pub review_rounds: u32, pub reviews_rejected: u32, pub candidates_red: u32,
                                                    pub generated_bounces: u32 }
#[derive(Copy, Eq, Default)] pub struct SeverityTally { pub critical: u32, pub important: u32, pub minor: u32 }
pub struct TaskRecord {
    pub v: u32, pub record_id: String, pub at: u64, pub run_id: String, pub task_id: String,
    pub path: Option<RunPath>, pub kind: TaskKind, pub hub: bool, pub test_mode: TestMode,
    pub planned_size: Size, pub final_size: Size, pub size_check: Option<SizeCheckInfo>,
    pub route: Route, pub review_routes: Vec<Route>,
    pub outcome: TaskOutcome, pub block: Option<BlockReason>,
    pub diff: Option<DiffStats>, pub tool_calls: u32,
    pub worker_usage: TokenUsage, pub reviewer_usage: TokenUsage, pub decider_usage: TokenUsage,
    pub phases: PhaseSecs, pub wall_secs: u64,
    pub gates: GateTally, pub severities: SeverityTally, pub bounces: GateCounts,
    pub failures: u8, pub stalls: u8, pub budget_exceeded: u8, pub conflicts: u8, pub max_rung: u8,
    pub sessions: u32, pub done_signal: Option<DoneSignal>, pub merge_commit: Option<String>,
}
pub struct RunRecord {
    pub v: u32, pub record_id: String, pub at: u64, pub run_id: String, pub goal: String,   // first 200 characters
    pub path: Option<RunPath>, pub triage: Option<TriageInfo>, pub profile_source: Option<ProfileSource>,
    pub outcome: String,                              // "accepted" | "discarded" | "failed" | "complete"
    pub base_branch: String, pub accepted_commit: Option<String>, pub tasks: u32, pub usage: Option<RunUsage>,
}
pub struct RevertRecord { pub v: u32, pub record_id: String, pub at: u64, pub run_id: String,
                          pub task_id: Option<String>, pub reverted: String, pub revert_commit: String }
pub struct StatsRow { pub class: String,              // "S" | "M" | "hub"
                      pub tasks: u32, pub merged: u32, pub median_lines: Option<u32>, pub median_tool_calls: Option<u32>,
                      pub median_tokens: Option<u64>, pub median_work_secs: Option<u64>, pub bounces: u32, pub reverted: u32 }
pub struct HistoryStats { pub path: PathBuf, pub task_records: u32, pub run_records: u32, pub rows: Vec<StatsRow>,
                          pub decider_calls: u32, pub decider_fallbacks: u32, pub size_checked: u32, pub size_raised: u32,
                          pub problems: Vec<String> }
```

`lib.rs` re-exports every new public type by name and bumps `PROTO_VERSION` to 8 (header). No new root name collides with an existing root re-export (checked name by name when the task lands, as M8a did).

### `config` (`crates/config/src/orchestrator/adapt.rs`, new; one call from `orchestrator::read`)

```rust
pub struct Deciders { pub mode: proto::DeciderMode,       // [orchestrator.deciders] mode = "claude" | "codex" | "off"; "claude"
                      pub timeout_secs: u64,              // 90, 5..=600
                      pub strength: proto::Strength,      // "fast"
                      pub effort: proto::Effort,          // "low"
                      pub slot_wait_secs: u64 }           // 30, 0..=600
pub struct Scouts { pub runtime: Option<proto::Runtime>,  // [orchestrator.scouts]; None = orchestrator.default_runtime; claude | codex
                    pub strength: proto::Strength,        // "fast"
                    pub effort: proto::Effort,            // "low"
                    pub timeout_secs: u64,                // 900, 60..=7200
                    pub max_tool_calls: u32 }             // 120, 10..=1000
pub struct Onboarding { pub auto: bool,                   // [orchestrator.onboarding] true
                        pub verify_timeout_secs: u64 }    // 1800, 10..=14400
pub struct Metering { pub otlp: bool,                     // [orchestrator.metering] true
                      pub otlp_port: u16 }                // 0 (any free port), 0..=65535
// config::Orchestrator gains:
pub fast_path: bool,                                      // [orchestrator] fast_path = true
pub deciders: Deciders, pub scouts: Scouts, pub onboarding: Onboarding, pub metering: Metering,
pub(crate) fn read_adapt(table: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>);
```

`lib.rs`'s `pub use orchestrator::{…}` line gains `Deciders, Metering, Onboarding, Scouts`. `orchestrator/unknown.rs` accepts `fast_path` and reports unknown keys under `deciders`, `scouts`, `onboarding` and `metering` with `report_unknown_nested`. Messages follow M8a's format: `orchestrator.deciders.timeout_secs: must be between 5 and 600 (using 90)`, `orchestrator.deciders.mode: must be claude, codex or off (using claude)`, and `unknown key, ignored` for anything else under these tables. None of these tables names a path or a confinement setting.

### `daemon`

```rust
// profile/mod.rs
pub fn repo_dir(data_dir: &Path, project: &Path) -> PathBuf;     // decision 4
pub fn summary(profile: &RepoProfile) -> String;                  // text below
pub const ONBOARDING_CHECKOUT: &str = ".onboarding";             // under <wt>/runs/
pub const VERIFY_CHECKOUT: &str = ".profile-verify";

// profile/resolve.rs (pure)
pub struct ChosenProfile { pub spec: ProfileSpec, pub source: ProfileSource, pub output_filter: OutputFilter,
                           pub filter_prefixes: Vec<String>, pub notes: Vec<String> }
pub fn run_profile(stored: Option<&RepoProfile>, stored_path: &Path, plan: &ProfileSpec, config: &ProfileSpec) -> ChosenProfile;
pub fn derived_prefixes(profile: &RepoProfile) -> Vec<String>;  // decision 28

// profile/proposal.rs (pure)
pub fn validate(profile: &RepoProfile) -> Vec<String>;          // "<key>: <problem>" each; globs, {test}, regex, env keys (reserved_env)
pub fn from_findings(proposed: &RepoProfile) -> RepoProfile;    // the scout's profile, invalid keys and built-in protected removed
pub fn apply_verification(proposed: &RepoProfile, v: &ProfileVerification, root: &Path) -> (RepoProfile, Vec<DroppedCommand>);
pub fn apply_edit(stored: &RepoProfile, key: &str, value: Option<&str>) -> Result<(RepoProfile, bool), String>; // bool: re-verify
pub const REVERIFY_KEYS: &[&str] = &["setup", "check", "check_timeout_secs", "single_test", "test_passed", "sample_test", "env"];
pub fn show_text(profile: &RepoProfile, verification: Option<&ProfileVerification>, dropped: &[DroppedCommand]) -> String;

// profile/store.rs (blocking)
pub enum Stored { Found { profile: RepoProfile, meta: ProfileMeta, path: PathBuf }, Unparseable { path: PathBuf, error: String }, Absent }
pub fn load(repo_dir: &Path) -> Stored;
pub fn save(repo_dir: &Path, profile: &RepoProfile, meta: &ProfileMeta) -> std::io::Result<()>;
pub fn load_proposal(repo_dir: &Path) -> Result<Option<ProposalRecord>, String>;
pub fn save_proposal(repo_dir: &Path, record: &ProposalRecord) -> std::io::Result<()>;
pub fn delete_proposal(repo_dir: &Path) -> std::io::Result<()>;
pub fn fingerprint(project: &Path, paths: &[String]) -> BTreeMap<String, String>;
pub fn stale(project: &Path, meta: &ProfileMeta) -> Vec<String>;
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()>;   // temp, fsync, rename, fsync dir
pub const FINGERPRINT_MAX_BYTES: u64 = 4 << 20;

// profile/verify.rs (blocking; its git writes go through GitQueue::write)
pub fn confine_spec(config: &config::Orchestrator, repo_dir: &Path, pre: &Preflight, daemon_socket: &Path)
    -> Option<run::confine::ConfineSpec>;                          // decision 9; None: unconfined
pub fn prepare(git: &OsStr, pre: &Preflight, path: &Path, repo: &Path, timeout: Duration) -> Result<(), String>;
                                                                   // run::git::prepare_scratch_in at pre.base_sha
pub fn run_commands(dir: &Path, profile: &RepoProfile, confine: Option<&run::confine::ConfineSpec>,
                    timeout: Duration, now: u64) -> ProfileVerification;
pub fn discard(git: &OsStr, root: &Path, path: &Path, repo: &Path, salvage_ref: &str, timeout: Duration)
    -> Result<Option<String>, String>;                             // run::git::salvage, then remove_checkout

// profile/service.rs
pub struct ProfileContext { pub data_dir: PathBuf, pub worktrees_root: PathBuf, pub git: OsString,
                            pub orchestrator: config::Orchestrator, pub git_queue: Arc<GitQueue>,
                            pub cli_caps: CliCaps, pub daemon_socket: PathBuf }
pub enum Effective { Stored { profile: RepoProfile, meta: ProfileMeta, path: PathBuf, stale: Vec<String> },
                     Unparseable { path: PathBuf, error: String },
                     Absent { proposal: Option<ProposalState> } }
pub struct ProfileService { /* per-project proposal table under daemon::lock, the scout service, the manager, the context */ }
impl ProfileService {
    pub fn new(scouts: Arc<ScoutService>, manager: Arc<WindowManager>, ctx: ProfileContext) -> Arc<Self>;
    pub async fn restore(self: &Arc<Self>);                                     // decision 11
    pub async fn effective(&self, project: &Path) -> Effective;
    pub async fn request(self: &Arc<Self>, request: ProfileRequest) -> ProfileReply;
    pub async fn start_detection(self: &Arc<Self>, pre: &Preflight, origin: ProposalOrigin,
                                 trust_project: bool, unconfined_checks: bool) -> Result<(), String>;   // Err: the refusal text
}

// scout/contract.rs (pure): SCOUT_CONTRACT, ONBOARDING_CONTRACT, SCOUT_NUDGE (texts below)
pub fn onboarding_first_turn(project: &Path, cwd: &Path, tracked: usize, top_level: &[String]) -> String;
pub fn scout_wrap_up(tool_calls: u32) -> String;

// scout/spec.rs (pure)
pub struct ScoutSpec { pub id: String, pub kind: ScoutKind, pub run_id: Option<String>, pub question: String,
                       pub first_turn: String, pub cwd: PathBuf, pub project: PathBuf, pub web: bool,
                       pub codex_config: Vec<headless::codex_guard::GuardEntry>, pub base_sha: String }
pub struct ScoutContext { pub roster: Vec<ModelEntry>, pub default_runtime: Runtime, pub scouts: config::Scouts,
                          pub claude: config::ClaudeHeadless, pub caps: CliCaps, pub data_dir: PathBuf }
pub fn route(roster: &[ModelEntry], runtime: Runtime, strength: Strength, effort: Effort) -> Route;
pub fn headless_spec(scout: &ScoutSpec, ctx: &ScoutContext) -> HeadlessSpec;   // decision 12
pub fn valid_id(id: &str) -> bool;                                              // ^[a-z0-9][a-z0-9-]{0,47}$

// scout/report.rs (pure)
pub struct ScoutReportArgs { pub summary: String, pub files: Vec<ScoutFile>, pub modules: Vec<String>,
                             pub interfaces: Vec<String>, pub risks: Vec<String>, pub profile: Option<RepoProfile> }
pub fn validate(args: &serde_json::Value, kind: ScoutKind) -> Result<ScoutReportArgs, String>;
pub fn report_path(repo_dir: &Path, run_dir: Option<&Path>, id: &str) -> PathBuf;
pub fn resolve_ref(reference: &str, run_dir: &Path, repo_dir: &Path, onboarding_report: Option<&str>) -> PathBuf;

// scout/machine.rs (pure)
pub struct ScoutLimits { pub timeout_secs: u64, pub max_tool_calls: u32 }
pub struct ScoutMachine { pub state: ScoutState, pub started_at: u64, pub turns_without_report: u8, pub tool_calls: u32,
                          pub wrap_up_sent: bool, pub usage: TokenUsage, pub failure: Option<String> }
pub enum ScoutEvent { Start { now: u64 }, TurnEnded { usage: Option<TokenUsage> }, ToolUse, Exited { code: Option<i32> },
                      ReportAccepted, Tick { now: u64 } }
pub enum ScoutEffect { Send(String), Kill, CloseStdin, KillAfter(Duration), RemoveAfter(Duration),
                       Finished(Result<(), String>) }
pub fn step(machine: ScoutMachine, event: ScoutEvent, limits: &ScoutLimits) -> (ScoutMachine, Vec<ScoutEffect>);

// scout/service.rs
pub enum ScoutOutcome { Report(ScoutReport), Failed { reason: String } }
pub struct ScoutHandle { pub id: String, pub window_id: u32, pub outcome: tokio::sync::oneshot::Receiver<ScoutOutcome> }
pub struct ScoutService { /* scout table under daemon::lock, manager, context */ }
impl ScoutService {
    pub fn new(manager: Arc<WindowManager>, ctx: ScoutContext) -> Arc<Self>;
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> tokio::task::JoinHandle<()>;  // signal listener + 1 s ticker
    pub async fn start(self: &Arc<Self>, spec: ScoutSpec) -> anyhow::Result<ScoutHandle>;
    pub async fn tool(&self, call: ToolCall) -> RunReply;          // RunReply::ToolResult
    pub fn stop(&self, id: &str);                                  // kill, fail with "stopped by the user"
    pub fn info(&self, id: &str) -> Option<ScoutInfo>;
    pub fn run_scouts(&self, run_id: &str) -> Vec<ScoutInfo>;      // for the snapshot; empty until M9
}

// decider/mod.rs (pure types; serialized in OpKind::Decide, so Serialize, Deserialize, Eq)
#[derive(Copy)] pub enum DeciderKind { Triage, SizeCheck, CheckSummary, BlockedReason }  // label(): "triage" | "size_check" | "check_summary" | "blocked_reason"
pub struct TriageInput { pub goal: String, pub profile_summary: String, pub report_summary: Option<String>,
                         pub report_files: Vec<String>, pub files: Vec<String>, pub files_total: u32 }
pub struct SizeCheckTask { pub id: String, pub title: String, pub brief: String, pub acceptance: Vec<String>,
                           pub owns: Vec<String>, pub deps: Vec<String>, pub size: Size, pub interface_change: bool, pub hub: bool }
pub struct Evidence { pub id: String, pub summary: String, pub files: Vec<String>, pub modules: Vec<String>, pub interfaces: Vec<String> }
pub struct SizeCheckInput { pub tasks: Vec<SizeCheckTask>, pub evidence_refs: Vec<String>, pub evidence: Vec<Evidence>,
                            pub modules: Vec<String>, pub hub: Vec<String> }   // evidence filled by the driver
pub struct CheckSummaryInput { pub task_id: String, pub command: String, pub code: Option<i32>, pub timed_out: bool, pub tail: String }
pub struct BlockedReasonInput { pub task_id: String, pub title: String, pub reason: String }
pub enum DeciderRequest { Triage(TriageInput), SizeCheck(SizeCheckInput), CheckSummary(CheckSummaryInput), BlockedReason(BlockedReasonInput) }
pub struct TriageTask { pub title: String, pub brief: String, pub acceptance: Vec<String>, pub owns: Vec<String>, pub size: Size,
                        pub interface_change: bool, pub test_mode: TestMode, pub test_mode_reason: Option<String>,
                        pub test_to_write: Option<String> }
pub struct TriageAnswer { pub kinds: Vec<TaskKind>, pub scale: Scale, pub reason: String, pub task: Option<TriageTask> }
pub struct SizeVerdict { pub id: String, pub size: Size, pub reason: String }
#[derive(Copy)] pub enum BlockKind { Question, MisSized, Environment }
pub enum DeciderAnswer { Triage(TriageAnswer), SizeCheck(Vec<SizeVerdict>), CheckSummary { lines: Vec<String> },
                         BlockedReason { kind: BlockKind, reason: String } }
pub struct Decision { pub kind: DeciderKind, pub answer: DeciderAnswer, pub source: DeciderSource,
                      pub fallback_reason: Option<String>, pub usage: Option<TokenUsage>, pub secs: u64 }
pub struct DeciderContext { pub mode: DeciderMode, pub program: OsString, pub route: Route, pub timeout: Duration,
                            pub cwd: PathBuf, pub schema_dir: PathBuf, pub caps: CliCaps }
impl DeciderContext {
    pub fn new(cfg: &config::Orchestrator, manager: &ManagerConfig, data_dir: &Path) -> Self;  // program: decider_bin, else claude_bin/codex_bin
}
// decider/schema.rs, prompt.rs, parse.rs, fallback.rs, argv.rs (pure)
pub fn schema(kind: DeciderKind) -> serde_json::Value;
pub fn render(request: &DeciderRequest) -> String;               // PROMPT_MAX_BYTES = 128 KiB
pub fn parse(kind: DeciderKind, value: &serde_json::Value) -> Result<DeciderAnswer, String>;
pub fn answer_from_events(events: &[SessionEvent]) -> Result<serde_json::Value, String>;   // decision 16's order
pub fn fallback(request: &DeciderRequest) -> DeciderAnswer;
pub fn fallback_decision(request: &DeciderRequest, reason: String) -> Decision;
pub struct DeciderCaps { pub claude_json_schema: bool, pub claude_max_turns: bool, pub claude_no_session_persistence: bool,
                         pub answer_source: AnswerSource, pub strict_schemas: bool, pub codex_output_schema: bool,
                         pub codex_ephemeral: bool }
pub enum AnswerSource { ResultField, StructuredOutputTool, Text }
pub const DECIDER_CAPS: DeciderCaps;                              // from M8b.1
pub fn claude_decider_args(ctx: &DeciderContext, dcaps: &DeciderCaps, schema: &serde_json::Value) -> Vec<String>;
pub fn codex_decider_args(ctx: &DeciderContext, dcaps: &DeciderCaps, schema_file: &Path, prompt: &str) -> Vec<String>;
pub fn schema_file_name(kind: DeciderKind, schema: &serde_json::Value) -> String;   // "<kind>-<16 hex>.json"
// decider/call.rs (I/O)
pub async fn decide(ctx: &DeciderContext, request: &DeciderRequest) -> Decision;
pub const ANSWER_MAX_BYTES: usize = 256 * 1024;

// headless (M8a's) additions
pub fn credential_scrub_for(runtime: Runtime, auth: config::ClaudeAuth) -> Vec<&'static str>;   // credential_scrub(spec) calls it
// HeadlessSpec gains  #[serde(default)] pub output_filter: Option<FilterHook>
// McpTarget gains     #[serde(default)] pub scout_id: Option<String>
// SessionEvent gains  StructuredOutput { value: serde_json::Value }   only if M8b.1 finds result.structured_output
// manager::headless::control_refusal(id, None) returns decision 12's scout text

// output_filter.rs (pure)
pub const FAILURE_RE: &str = r"(?i)\b(fail(ed|ure|ures|s)?|error(s)?|panic(ked|s)?|assert(ion)?|expected|traceback|exception)\b";
pub const LINE_MAX_CHARS: usize = 500;
pub const LOG_DIR_NAME: &str = "anthrex-logs";                    // under the task TMPDIR (decision 28)
pub const HOOK_SETS_ALLOW: bool;                                  // from M8b.1
#[derive(Serialize, Deserialize)] pub struct FilterHook { pub mode: OutputFilter, pub prefixes: Vec<String>, pub log_dir: PathBuf }
pub fn apply(mode: OutputFilter, lines: &[String], exit_code: i32) -> Vec<String>;
pub fn matches(command: &str, prefixes: &[String]) -> bool;
pub fn wrap(exe: &Path, hook: &FilterHook, command: &str) -> String;
pub fn rewrite(payload: &serde_json::Value, exe: &Path, hook: &FilterHook) -> Option<serde_json::Value>;
pub fn hook_command(exe: &Path, hook: &FilterHook) -> String;     // the settings "command" string, shell-quoted (launch::shell_quote)
pub fn add_hook(settings: &mut serde_json::Value, exe: &Path, hook: Option<&FilterHook>);

// metering/otlp.rs (pure)
#[derive(Copy)] pub enum UsageKind { Input, Output, CacheRead, CacheWrite }
pub struct UsagePoint { pub run_id: String, pub role: String, pub session_id: String, pub model: String,
                        pub kind: UsageKind, pub value: u64, pub cumulative: bool }
pub fn parse_metrics(body: &[u8]) -> Result<Vec<UsagePoint>, String>;
#[derive(Default)] pub struct OtlpLedger { /* last value per cumulative series, totals per (run, role) */ }
impl OtlpLedger { pub fn apply(&mut self, points: &[UsagePoint]) -> Vec<(String, String)>;   // (run, role) pairs touched
                  pub fn total(&self, run_id: &str, role: &str) -> TokenUsage; }
pub fn orchestrator_env(addr: &str, run_id: &str) -> Vec<(String, String)>;
// metering/server.rs (I/O)
pub const OTLP_MAX_BODY: usize = 4 << 20; pub const OTLP_MAX_HEADERS: usize = 16 << 10;
pub const OTLP_READ_TIMEOUT: Duration = Duration::from_secs(10);
pub struct OtlpServer { pub addr: std::net::SocketAddr }
pub async fn bind(port: u16, data_dir: &Path, sink: tokio::sync::mpsc::UnboundedSender<(String, TokenUsage)>,
                  shutdown: CancellationToken) -> std::io::Result<OtlpServer>;

// run/triage.rs (pure)
pub const PLAN_SCALE_MAX: usize = 12;                             // spec §22.3's planner_task_cap starting point; M9 owns the config key
pub enum TriageRoute { Fast(PlanTask), Plan { reason: String }, Large { reason: String } }
pub fn route(decision: &Decision, fast_path: bool) -> TriageRoute;
pub fn info(decision: &Decision, route: &TriageRoute, now: u64) -> TriageInfo;
pub fn fast_plan(goal: &str, task: PlanTask, profile: ProfileSpec) -> Plan;
pub fn started_message(info: &TriageInfo, run_id: &str, task: &Task) -> String;
pub fn refused_message(info: &TriageInfo) -> String;

// run/phases.rs (pure)
pub fn set_state(task: &mut Task, state: TaskState, now: u64);    // decision 31

// run/history.rs (pure)
pub fn task_record(run: &Run, task: &Task, outcome: TaskOutcome, now: u64) -> TaskRecord;
pub fn run_record(run: &Run, outcome: &str, now: u64) -> RunRecord;
pub fn task_record_id(run_id: &str, task_id: &str) -> String;     // "<run>/<task>"
// run/stats.rs (pure)
pub fn aggregate(lines: &[HistoryLine], path: &Path) -> HistoryStats;
pub fn render(stats: &HistoryStats) -> String;
// run/history_io.rs (blocking)
pub fn append_line(path: &Path, line: &HistoryLine) -> std::io::Result<()>;          // one line, sync_all
pub fn contains_record(path: &Path, record_id: &str) -> std::io::Result<bool>;
pub fn read_history(path: &Path) -> (Vec<HistoryLine>, Vec<String>);                // last line per record_id; problems
pub fn measure_diff(git: &OsStr, root: &Path, from: &str, to: &str, three_dot: bool, timeout: Duration) -> Result<DiffStats, String>;
pub fn detect_reverts(git: &OsStr, root: &Path, base_branch: &str, history: &[HistoryLine], now: u64,
                      timeout: Duration) -> Result<Vec<RevertRecord>, String>;

// run/engine (additions; all serialized types #[serde(default)] where they are fields)
pub enum OpKind { /* M8a's */
    Decide { decider_id: u64, task_ids: Vec<String>, request: DeciderRequest },
    MeasureDiff { root: PathBuf, from: String, to: String, three_dot: bool },
    AppendHistory { path: PathBuf, record_id: String, line: HistoryLine },
}
pub enum OpResult { /* M8a's */ Decided(Decision), DiffMeasured(DiffStats), HistoryAppended }
pub enum EventKind { /* M8a's */ Promote { reply: ReplyId, run_id: String }, OrchestratorUsage { run_id: String, usage: TokenUsage } }
// run/model_adapt.rs (pure; re-exported by model.rs)
pub struct QueuedDecider { pub decider_id: u64, pub task_ids: Vec<String>, pub request: DeciderRequest, pub queued_at: u64 }
pub enum SizeCheckState { Pending { decider_id: u64 }, Done(SizeCheckInfo) }
pub struct PendingFailure { pub gate: GateKind, pub rung: u8, pub check_index: usize, pub decider_id: u64 }
// Run gains: path: Option<RunPath>, triage: Option<TriageInfo>, promote_requested_at: Option<u64>,
//   profile_source: Option<ProfileSource>, output_filter: OutputFilter, filter_prefixes: Vec<String>,
//   repo_dir: PathBuf (empty: history off for this run), scout_reports: Vec<String>, onboarding_report: Option<String>,
//   decider_queue: Vec<QueuedDecider>, next_decider: u64, decider_calls: u32, decider_fallbacks: u32,
//   decider_usage: TokenUsage, triage_usage: TokenUsage, scout_usage: TokenUsage, orchestrator_usage: TokenUsage,
//   stale_profile: Vec<String>, run_record_written: bool
// Task gains: size_check: Option<SizeCheckState>, pending_failure: Option<PendingFailure>, pending_classification: bool,
//   block_source: Option<DeciderSource>, decider_usage: TokenUsage, phases: PhaseSecs, phase_since: u64, max_rung: u8,
//   diff: Option<DiffStats>, history_written: bool
// CheckRecord (model_rounds.rs) gains: summary: Option<String>, summary_source: Option<DeciderSource>
// RunLimits gains: decider_mode: DeciderMode, decider_slot_wait_secs: u64
// run/engine/deciders.rs (pure)
pub fn queue(run: &mut Run, task_ids: Vec<String>, request: DeciderRequest, now: u64) -> u64;   // decider id
pub fn dispatch(run: &mut Run, now: u64) -> Vec<Effect>;           // start queued deciders into free reader slots; slot-wait fallbacks
pub fn on_decided(run: &mut Run, decider_id: u64, decision: Decision, now: u64) -> Vec<Effect>;
pub fn apply_raise(run: &mut Run, task_id: &str, size: Size, reason: &str);   // decision 19

// run/driver (additions)
// driver/requests.rs: build(plan_toml, …) = parse_plan + build_plan(plan, dir, yes, trust_project, unconfined_checks)
// driver/adapt.rs: choose_profile, start_goal, promote, stats, profile, and the Decide / MeasureDiff / AppendHistory executions
pub struct Adaptation { pub profiles: Arc<ProfileService>, pub scouts: Arc<ScoutService>, pub deciders: DeciderContext }
impl RunService {
    pub fn git_queue(&self) -> Arc<GitQueue>;
    pub fn set_adaptation(&self, adaptation: Adaptation);         // once, from lifecycle; a OnceLock field
    pub fn orchestrator_usage(&self, run_id: String, usage: TokenUsage);   // sends EventKind::OrchestratorUsage
}
```

**Reconcile rows** added to M8a's table (`run/reconcile/mod.rs` and `reconcile/git.rs`):

| `OpKind` | Reality checked | Replay | Otherwise |
|----------|-----------------|--------|-----------|
| `Decide`, `MeasureDiff` | none | — | `NotStarted` |
| `AppendHistory` | `contains_record(path, record_id)` | found → `HistoryAppended` | `NotStarted` |

### MCP (`crates/mcp`, M8a's)

`McpOptions` gains `scout_id: Option<String>`. `tools_for(AgentRole::Scout)` returns one tool, built in the new `crates/mcp/src/tools_scout.rs`. The schema is a closed object at every level (`additionalProperties: false`), and it has **no** property for `cache_dirs` or any `confined_*` setting (decision 5):

| Role | Tool | Description | Properties (required in bold) |
|------|------|-------------|-------------------------------|
| scout | `submit_scout_report` | `Submit your findings. Call it once, then stop.` | **`summary`** string 1–8000; **`files`** array ≤ 60 of objects {**`path`** string 1–500, **`why`** string 1–300}; `modules` array ≤ 40 of string 1–200; `interfaces` array ≤ 40 of string 1–500; `risks` array ≤ 20 of string 1–500; `profile` object {`languages` array ≤ 10 of string 1–40; `modules`, `hub`, `source`, `generated`, `protected` arrays ≤ 40 of string 1–300; `setup`, `check` string 1–2000; `check_timeout_secs` integer 10–14400; `single_test` string 1–1000; `test_passed`, `sample_test` string 1–300; `output_filter` enum `failures-only`, `tail`, `none`; `filter_prefixes` array ≤ 10 of string 1–100; `conventions` array ≤ 20 and `manifests` array ≤ 50 of string 1–300; `env` object ≤ 20 properties matching `^[A-Za-z_][A-Za-z0-9_]*$` with string values 0–1000} |

Engine-side texts (`ToolResult`):
- **Refusals:** `unknown scout <id>`; `this window is not scout <id>`; `scout <id> is <state>`; `a report for scout <id> was already recorded`; `invalid arguments: <field>: <problem>`; `invalid arguments: profile: required for the onboarding scout`; `invalid arguments: profile: only the onboarding scout reports a profile`.
- **Success:** `Report recorded. You are done; end your turn now.`

A reserved `env` key in the scout's profile is not a tool refusal: `proposal::from_findings` drops it and `proposal::validate`'s message lands in `ProposalRecord.dropped` with the reason, so the user sees why.

### Decider schemas (exact)

Every property is required and optional values are nullable, so each schema also satisfies strict structured-output modes (M8b.1 records whether either CLI needs that).

```json
{"triage":{"type":"object","additionalProperties":false,"required":["kinds","scale","reason","task"],"properties":{
  "kinds":{"type":"array","minItems":1,"maxItems":4,"items":{"enum":["code","docs","research","review"]}},
  "scale":{"enum":["single","plan","large"]},
  "reason":{"type":"string","minLength":1,"maxLength":500},
  "task":{"anyOf":[{"type":"null"},{"type":"object","additionalProperties":false,
    "required":["title","brief","acceptance","owns","size","interface_change","test_mode","test_mode_reason","test_to_write"],
    "properties":{
      "title":{"type":"string","minLength":1,"maxLength":120},
      "brief":{"type":"string","minLength":1,"maxLength":4000},
      "acceptance":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"string","minLength":1,"maxLength":500}},
      "owns":{"type":"array","minItems":1,"maxItems":10,"items":{"type":"string","minLength":1,"maxLength":300}},
      "size":{"enum":["S","M"]},
      "interface_change":{"type":"boolean"},
      "test_mode":{"enum":["tdd","check","none"]},
      "test_mode_reason":{"type":["string","null"],"maxLength":300},
      "test_to_write":{"type":["string","null"],"maxLength":300}}}]}}},
 "size_check":{"type":"object","additionalProperties":false,"required":["tasks"],"properties":{
  "tasks":{"type":"array","maxItems":50,"items":{"type":"object","additionalProperties":false,"required":["id","size","reason"],
    "properties":{"id":{"type":"string","minLength":1,"maxLength":16},"size":{"enum":["S","M","L"]},
                  "reason":{"type":"string","minLength":1,"maxLength":500}}}}}},
 "check_summary":{"type":"object","additionalProperties":false,"required":["lines"],"properties":{
  "lines":{"type":"array","minItems":1,"maxItems":40,"items":{"type":"string","maxLength":300}}}},
 "blocked_reason":{"type":"object","additionalProperties":false,"required":["kind","reason"],"properties":{
  "kind":{"enum":["question","mis_sized","environment"]},"reason":{"type":"string","minLength":1,"maxLength":300}}}}
```

`schema(kind)` returns the value under the kind's label.

`parse` enforces every limit above, and additionally:
- **Triage:** `task` must be non-null when `scale` is `single`, and is ignored otherwise.
- **Size check:** ids not asked for are ignored; an asked id missing from the answer keeps its size (decision 19).

An error names the path, for example `tasks[2].size: expected S, M or L`.

### Decider prompts (exact; `<…>` are inputs, each section omitted when its input is empty)

```text
[anthrex decider] triage v1
You label a coding goal for an orchestration engine. Answer with one JSON object that matches the schema, and nothing else.
kinds: every kind the goal needs. code changes behaviour; docs changes documentation, comments or configuration nothing executes; research investigates and reports without changing code; review reviews an existing branch or commit range.
scale: single when one task of size S or M does the whole goal; plan when it needs 2 to 12 tasks; large when it needs more, or two or more separate areas that each need several tasks.
Size S: one file, no interface change, about 20 changed lines or fewer. Size M: 1 to 3 files inside one module, about 100 changed lines or fewer. Anything bigger is not single.
When scale is single, give task: a short title; a brief a worker can follow without asking anything; acceptance criteria; the paths it owns, as globs relative to the repository root and as narrow as possible; its size; whether it changes an interface other code uses; its test mode (tdd for any change in behaviour, check for behaviour-preserving work already covered by tests, none for docs) with a one-line reason unless tdd; and for tdd the name of the test to write. Otherwise task is null.

Goal:
<goal>

Repository profile:
<profile::summary>

Onboarding scout report:
<summary>
Files it named: <path, path, …>

Tracked files (<n> of <total>):
<one path per line>
```

```text
[anthrex decider] size_check v1
You check the size of planned coding tasks against what scouts found in the repository. Answer with one JSON object that matches the schema, and nothing else, with one entry per task id below.
Size S: one file, no interface change, a mechanical check exists, about 20 changed lines or fewer. Size M: 1 to 3 files inside one module, a clear spec, about 100 changed lines or fewer. Size L: files in more than one module plus an interface change, more than about 100 lines, an unclear spec, or a new dependency.
Judge each task from the evidence, not from its stated size. Give the size you believe and a one-sentence reason naming the evidence.

Modules: <modules>
Hub: <hub>

Tasks:
- id <id>, stated size <S|M>, hub <yes|no>, interface change <yes|no>
  title: <title>
  owns: <globs>
  depends on: <ids>
  acceptance: <item>; <item>
  brief: <brief>

Scout evidence:
## <report id>
<summary>
Files: <paths>
Modules: <modules>
Interfaces: <interfaces>
```

```text
[anthrex decider] check_summary v1
A check command failed in a coding task's checkout. Summarise the failure for the agent who must fix it, in at most 40 lines. Keep failing test names, error messages, file:line locations and assertion values exactly as they appear. Leave out passing tests, progress output and anything repeated. Answer with one JSON object that matches the schema, and nothing else.

Command: <command>
Result: <exit <code> | timed out>
Output (last <n> lines):
<tail>
```

```text
[anthrex decider] blocked_reason v1
A coding agent stopped its task and gave the reason below without saying what kind of block it is. Classify it. question: it needs an answer or a decision about the task. mis_sized: the task is bigger than one task, or needs changes outside the paths it owns. environment: a tool, command, permission, dependency or setup is broken or missing. Answer with one JSON object that matches the schema, and nothing else.

Task <id>: <title>
Reason:
<reason>
```

Cutting order when a prompt would pass `PROMPT_MAX_BYTES`:
- **Triage:** tracked files first (from the end), then the report's file list, then the report summary (to 8000 characters).
- **Size check:** evidence summaries (each to 4000 characters, then from the last report), then briefs (each to 2000 characters).
- **Check summary:** the tail from its start.

Each cut leaves `[anthrex] … cut …` in its place.

### Contracts and texts (exact)

```text
SCOUT_CONTRACT:
You are a scout in an anthrex orchestration run. You read and report; you never change anything.
1. Answer the question in your first message from what is in this directory. Read manifests, CI files, READMEs and agent instruction files before source code.
2. Do not edit, create or delete files, and do not commit. Your session cannot write anything.
3. Call the anthrex tool submit_scout_report exactly once: a summary of at most about 2000 tokens, the files that matter with one line each on why, and the modules, interfaces and risks you found. Then stop.
4. Report what you found, not what you guess. Say in the summary what you could not determine.

ONBOARDING_CONTRACT:
You are the onboarding scout of anthrex, a tool that runs coding agents on this repository. You work out how it is set up, built and tested. You never change anything.
1. This directory is a disposable copy of the repository at its current commit. Your session is read-only: nothing you run can write a file or reach the network. Commands that only read (listing files, printing a Makefile's targets, showing git history) work; builds and tests do not, and you should not try them.
2. Read manifests, lock files, CI configuration, READMEs and agent instruction files (AGENTS.md, CLAUDE.md and similar) before source code.
3. Find: the languages; what counts as one module (globs); hub paths that many modules depend on; where behaviour lives (source globs); generated files that builds rewrite on their own, taken from the lock files you find (for example Cargo.lock, package-lock.json, yarn.lock, pnpm-lock.yaml, poetry.lock, uv.lock, go.sum); protected files that configure or instruct coding agents: always .claude/**, .mcp.json, .codex/**, **/CLAUDE.md and **/AGENTS.md, plus any other agent configuration, hook, MCP server or instruction file you find (for example .cursor/**, .github/copilot-instructions.md, GEMINI.md); a setup command to run once in a fresh copy; one check command that builds, tests and lints everything CI checks; a command that runs one named test, with {test} where the name goes; a regular expression, with {test} where the name goes, that matches a line of that command's output only when that test ran and passed; the name of one existing test that passes; the manifests and convention files you relied on; environment variables every copy needs, with {worktree} for the copy's path.
4. Propose the commands CI would run. anthrex runs each of them itself afterwards, in a fresh copy and under the same restrictions its runs use, and proposes only the ones that pass.
5. Call the anthrex tool submit_scout_report exactly once, with a short summary, the files that matter, and the profile. Then stop.
```

| Name | Text |
|------|------|
| `onboarding_first_turn` | `[anthrex] Work out this repository's profile. Repository: <project>. Disposable copy: <cwd>. It tracks <n> files; the top-level entries are: <names, comma-separated, at most 60>.` |
| `SCOUT_NUDGE` | `[anthrex] Your turn ended without a report. Call submit_scout_report now with what you found, then stop.` |
| `scout_wrap_up` | `[anthrex] You have used <n> tool calls. Stop exploring and call submit_scout_report now with what you found.` |
| `profile::summary` | Lines `languages: <a, b>`, `modules: <globs>`, `hub: <globs>`, `source: <globs>`, `generated: <globs>`, `protected: built-in <5 globs> + <extras or "no extras">` (always printed), `setup: <command or none>`, `check: <command or "none (runs are unverified)">`, `single test: <command or "none (tdd is impossible; code tasks use check)">`, each list line omitted when empty. |
| `started_message` | `triage: <kinds joined with ,>/<scale> (<decider \| fallback: <reason>>)` / `fast path: one task, no plan gate` / `  t1  <size>  <mode>  <title>` / `watch with: anthrex run status <id>` |
| `refused_message` | `triage: <kinds>/<scale> (<decider \| fallback: <reason>>): <reason>` / `this goal needs a planned run, which arrives with the orchestrator (milestone 9). Write a plan file and run: anthrex run start --plan <file>` |
| `control_refusal` (no run) | `window <id> is a headless scout session; only the daemon drives it. Use anthrex profile reject to stop it` |
| `blocked_recorded` (unclassified) | `Blocked recorded (classifying). Stop and wait for an answer.` |
| `check_failed_message` (with a decider summary) | M8a's text with `Last 40 lines:\n<tail>` replaced by `Summary of its output:\n<summary lines>` |
| `candidate_red_message` (with a decider summary) | The same replacement in M8a's text |

### CLI

```
anthrex run start (--plan <file> | --goal <text>) [--yes] [--trust-project] [--unconfined-checks]
anthrex run promote <run>
anthrex run stats [--json]
anthrex profile status [--json]
anthrex profile detect [--trust-project] [--unconfined-checks]
anthrex profile show [--proposed] [--json]
anthrex profile confirm [--yes]
anthrex profile reject
anthrex profile edit <key> <value> [--yes] [--unconfined-checks]
anthrex profile edit --unset <key> [--yes] [--unconfined-checks]
anthrex mcp --role scout --scout <id> [--run <run>] --window <id> [--socket <path>]      (hidden)
anthrex filter-run --mode <failures-only|tail|none> --log-dir <dir> -c <command>          (hidden, before clap)
anthrex filter-hook --mode <failures-only|tail|none> --log-dir <dir> [--prefix <p>]...   (hidden, before clap)
```

Every command takes M8a's global `--dir` (default: the current directory).

- **`run start`.** `--plan` and `--goal` are a required, mutually exclusive clap group.
  - `--goal` uses `GOAL_REQUEST_TIMEOUT` (decision 22).
  - On the fast path it prints the run id on stdout and `started_message` on stderr, and exits 0.
  - On a refusal it prints the message on stderr and exits 1.
- **`run promote`** exits 0 on `Done` and 1 on `Refused`.
- **`run stats`** prints `stats::render` or, with `--json`, `HistoryStats`:

  ```
  history: <path>  (<n> task records, <m> runs)
  CLASS  TASKS  MERGED  LINES  TOOL CALLS  TOKENS  WORK MIN  BOUNCES  REVERTED
  S      20     19      14     22          180k    6         3        0
  M      15     13      71     96          1.1M    31        7        1
  hub    2      2       64     120         1.4M    40        1        0
  deciders: 42 calls, 3 fallbacks · size cross-check: 30 checked, 2 raised
  ```

  Every numeric column except `TASKS`, `MERGED`, `BOUNCES` and `REVERTED` is a median, and `-` when there are no merged tasks. Tokens are shown as `<n>`, `<n>k` (one decimal under 10k) or `<n>.<d>M`. A problem line `history: <n> lines skipped: <first problem>` follows when any line was skipped.
- **`profile`** requests use `RUN_REQUEST_TIMEOUT`. The subcommand lives in `crates/cli/src/profile_cmd.rs`.
  - `status` prints:

    ```
    profile: <project>
      stored: yes, confirmed <yyyy-mm-dd hh:mm> (<repo_dir>/profile.toml)      | stored: no
      stale: <paths> changed since it was confirmed                              (only when stale)
      detection: <state> since <hh:mm> (scout <id>, window <n>)                  | detection: failed: <reason> | detection: none
      verification: confined, as runs are                                        | verification: unconfined (<why>)
    ```

  - `show` prints `show_text`, which is the TOML, then a comment block:

    ```
    # verification <yyyy-mm-dd hh:mm> (confined)
    #   setup        ok     3s   cargo fetch
    #   check        ok   214s   cargo build --workspace …
    #   single_test  ok    12s   cargo test --workspace -- --exact {test}  (sample: store::tests::round_trip)
    # dropped
    #   check: exit 101 after 30s: cargo test --all
    #     <the confinement hint of decision 9, when it applies>
    #     <each of the last 40 lines>
    ```

  - `confirm` prints the same, then asks.
  - `edit` never waits for verification. It prints one of:
    - `proposed: <key> = <value>; verifying (anthrex profile status), then confirm with anthrex profile confirm`;
    - `proposed: <key> = <value>; confirm with anthrex profile confirm`, when nothing needs re-running;
    - with `--yes`, `proposed: <key> = <value>; it is stored as soon as verification passes (anthrex profile status)`. The daemon confirms it (`ProposalRecord.auto_confirm`).
- **`run status`** (`run_cmd/status.rs`) adds ` fast path` after the state for a fast-path run, and a line `  triage: <kinds>/<scale> (<source>)` under `goal:`.

### File sizes this milestone must respect

AGENTS.md rule 8 puts the limit at about 600 lines. Counts are `wc -l` on `baa04e1`; at the start, run `wc -l` again and record any difference under "Implementation notes". Budgets are growth over those counts. A file whose budget would carry it past 600 is split by responsibility first, in a pure-move commit of its own.

| File | Count | Budget | Note |
|------|------:|-------:|------|
| `crates/config/src/lib.rs` | 605 | 0 lines | Only the `pub use orchestrator::{…}` line changes. |
| `crates/config/src/orchestrator.rs` | 570 | +8 | Five fields, their defaults, one `adapt::read_adapt` call; everything else in `orchestrator/adapt.rs`. |
| `crates/config/src/orchestrator/unknown.rs` | 90 | +25 | |
| `crates/proto/src/run.rs`, `run_info.rs`, `run_wire.rs` | 451, 222, 118 | +2, +25, +50 | New variants and fields only; new types in `profile.rs`, `scout.rs`, `adapt.rs`, `history.rs`. |
| `crates/proto/src/lib.rs` | 79 | +25 | |
| `crates/daemon/src/run/model.rs`, `model_rounds.rs` | 430, 213 | +45, +6 | New structs in `run/model_adapt.rs`. |
| `run/engine/dispatch.rs` | 578 | +10 | Decider dispatch is `engine/deciders.rs`. |
| `run/engine/{gates,merge,done,requests,restore,schedule}.rs` | 467, 505, 492, 404, 384, 303 | +40 each | These call into `engine/deciders.rs` and `phases::set_state`. |
| `run/engine/mod.rs`, `ops.rs`, `ladder.rs` | 524, 395, 511 | +20, +20, +5 | |
| `run/edits.rs` | 571 | +5 | `set_state` replacements are line-neutral. |
| `run/snapshot.rs`, `run/report.rs`, `run/reconcile/mod.rs`, `reconcile/git.rs` | 230, 236, 227, 441 | +50, +40, +10, +25 | |
| `run/driver.rs` | 598 | +3 | `mod adapt;`, the `OnceLock<Adaptation>` field and its init. If that passes 600, move `Book` and `Retiring` into `driver/book.rs` first. |
| `run/driver/ops.rs`, `driver/requests.rs`, `driver/restore.rs` | 580, 561, 443 | +10, +25, +15 | Dispatch only; bodies in `driver/adapt.rs` (which may itself be split into `driver/adapt/{requests,ops}.rs`). |
| `run/role_launch.rs`, `run/contract.rs`, `run/exec.rs` | 552, 563, 555 | +15, +20, 0 | |
| `crates/daemon/src/headless/argv.rs`, `headless/mod.rs`, `claude_stream.rs` | 446, 255, 343 | +15, +20, +15 | `add_hook` call, `--scout`, `credential_scrub_for`, `StructuredOutput`. |
| `crates/daemon/src/manager/headless.rs` | 521 | +5 | The scout refusal text. |
| `crates/daemon/src/server/run_api.rs` | 96 | +10 | Routing only. |
| `crates/daemon/src/lifecycle.rs` | 439 | +35 | Services, OTLP bind, restore order. |
| `crates/mcp/src/tools.rs`, `lib.rs`, `forward.rs` | 250, 106, 148 | +15, +3, +5 | The scout schema in `tools_scout.rs`. |
| `crates/cli/src/main.rs` | 594 | +6 | `Profile` variant and arm, `mod` lines; the `hook`, `filter-run` and `filter-hook` pre-clap dispatch moves into a new `pre_clap.rs` (bodies in `filter_run.rs`, `filter_hook.rs`), `profile_cmd.rs`. |
| `crates/cli/src/mcp_cmd.rs`, `run_cmd.rs`, `run_cmd/status.rs` | 121, 472, 220 | +25, +25, +20 | `--goal`, `promote`, `stats` bodies in `run_cmd/adapt.rs`. |
| `crates/fake-agent/src/runtime.rs`, `script.rs`, `headless.rs`, `roles.rs`, `main.rs` | 343, 424, 553, 323, 342 | +40, +20, +20, +10, +10 | Decider mode in `decider.rs`; the `bash` step in `bash.rs`. |
| `crates/cli/tests/support/run_harness.rs` | 525 | +15 | Decision 36's defaults and `pub(super)` fields; the M8b helpers in `support/run_adapt.rs`. |
| `scripts/pty-smoke.py` | 1793 | +6 | The stage lives in `scripts/pty_smoke_adapt.py`. |

No new file may exceed 600 lines.

## Produces for later milestones

Exact names milestones 8c, 9 and 9.5 consume. Renaming any of them later is a cross-milestone change and must update those briefs.

| Consumer | Name | What it is |
|----------|------|-----------|
| M8c | `RunInfo.path: Option<RunPath>` (`proto::RunPath { Fast, Plan, Large }`) | A fast-path run's root node is the run itself (spec §16.1). |
| M8c | `RunInfo.triage: Option<TriageInfo>`, `RunInfo.promote_requested_at: Option<u64>`, `RunInfo.profile_source: Option<ProfileSource>` | Inspector fields. |
| M8c | `RunInfo.usage: Option<RunUsage { total, by_role, decider_calls, decider_fallbacks }>` | The orchestrator inspector's `spend` line; `by_role` keys `worker`, `reviewer`, `scout`, `decider`, `orchestrator`. |
| M8c | `RunInfo.scouts: Vec<ScoutInfo>` (`proto::ScoutInfo`, `ScoutState`, `ScoutKind`) | Scout nodes and the scout inspector (spec §16.3–§16.4); empty until M9 spawns run scouts. |
| M8c | `TaskInfo.decider_usage`, `TaskInfo.size_check: Option<SizeCheckInfo>`, `TaskInfo.diff: Option<DiffStats>`, `TaskInfo.phases: Option<PhaseSecs>`, `TaskInfo.block_source`, `CheckInfo.decider_summary`, `CheckInfo.summary_source` | Task inspector fields. Round usage is M8a's `AgentRoundInfo.usage`. |
| M8c | `proto::AgentRole::Scout` | A scout's headless window carries it in `WindowInfo.run` for run scouts. |
| M9 | `proto::RepoProfile`, `profile::repo_dir(data_dir, project)`, `<repo_dir>/profile.toml`, `ProfileService::effective(project) -> Effective`, `profile::summary(&RepoProfile)` | The profile for `get_context` and plan validation. |
| M9 | `proto::ScoutReport`, `ScoutFile`, `ScoutKind::Area`; run scout reports at `<data_dir>/runs/<run>/scouts/<id>.json`, repository ones at `<repo_dir>/scouts/<id>.json`; `scout::report::{report_path, resolve_ref}`; the `onboarding` alias in `scout_refs` | Scout output for planners and worker briefs. |
| M9 | `ScoutService::{start, tool, stop, info, run_scouts}`, `ScoutSpec`, `ScoutContext`, `ScoutOutcome`, `SCOUT_CONTRACT`, `scout::spec::{route, headless_spec}`, `Run.scout_reports`, `Run.scout_usage` | `spawn_scout` builds on these. Area scouts are launched read-only (decision 12). It must push the report id into `Run.scout_reports` so the size cross-check sees it. |
| M9 | `decider::call::decide(&DeciderContext, &DeciderRequest) -> Decision`, `DeciderRequest`, `DeciderAnswer`, `Decision`, `DeciderKind`, `decider::fallback::fallback_decision`, `OpKind::Decide`, `run::engine::deciders::{queue, on_decided}` | The decider interface; new kinds are new variants. |
| M9 | `RunRequest::StartGoal`, `run::triage::{route, TriageRoute}`, `driver/requests.rs::build_plan` | M9 replaces the `Plan` and `Large` refusal with the orchestrator path. |
| M9 | `Run.promote_requested_at` | M9 performs the promotion the user asked for. |
| M9 | `metering::orchestrator_env(addr, run_id)`, `<data_dir>/otlp.addr`, `EventKind::OrchestratorUsage` | Metering the orchestrator window. |
| M9 | `headless::McpTarget.scout_id`, `ToolCall.scout_id`, `anthrex mcp --role scout --scout` | Scout MCP plumbing. |
| M9.5 | `proto::history::{HistoryLine, TaskRecord, RunRecord, RevertRecord, HistoryStats, StatsRow, HISTORY_VERSION}`, `<repo_dir>/history.jsonl` | The record type and path the refit reads. |
| M9.5 | `run::history_io::read_history(path)`, `run::stats::{aggregate, render}`, `anthrex run stats` | M9.5 adds proposals to `stats`. |
| M9.5 | `TaskRecord.phases`, `TaskRecord.diff`, `TaskRecord.max_rung`, `RunInfo.rate_limits` (M8a) | The inputs of threshold and budget refits and adaptive concurrency. |

## Tasks

Shared test conventions:

- **Reducer tests** use M8a's `run/engine/tests/fixture.rs` (register each new test file in `engine/tests/mod.rs`). M8b adds `fx.decided(decider_id, Decision)` and `fx.queued_deciders()` to the fixture.
- **Process tests** that need `fake-agent` live in `crates/cli/tests/`, use `support::fake_agent_bin()`, and call daemon library code directly. They build repositories with `support::run_harness::init_repo` (CLI tests do not include `crates/daemon/tests/support`). Daemon-crate tests that need a repository but no `fake-agent` use `crates/daemon/tests/support/mod.rs`'s `TempRepo`.
- **End-to-end tests** use M8a's `RunHarness`. M8b adds these methods, in `crates/cli/tests/support/run_adapt.rs`:
  - `with_deciders(mode)`: returns the `[orchestrator]` lines `deciders.mode = "<mode>"`, `deciders.timeout_secs = 5`, `deciders.slot_wait_secs = 2`, and the environment `ANTHREX_DECIDER_BIN = fake_agent_bin()` and `FAKE_AGENT_DECIDER_DIR = <tmp>/deciders`, for `RunHarness::with_env`;
  - `decider(kind, n, value)`, which writes `<tmp>/deciders/<kind>-<n>.json`;
  - `decider_calls() -> Vec<Value>`;
  - `stored_profile(toml)`, which writes `<repo_dir>/profile.toml` and a `profile.meta.json` with the fingerprint of the files the TOML names, as `anthrex profile confirm` would (`repo_dir` from `anthrex profile status --json`);
  - `onboarding_report(json)`;
  - `start_goal(goal) -> Output`, waiting `GOAL_WAIT`.
- **`GOAL_WAIT` = 120 s**: `REQUEST_WAIT` (60 s, which covers `build`'s ten git calls at 5 s) + the harness's `deciders.timeout_secs` (5 s) + the kill grace (2 s) + `StartGoal`'s own preflight and `git ls-files` (seven calls at 5 s, 35 s) = 102 s, rounded up. Add the row to `docs/timing-budgets.md`.
- **`PROFILE_WAIT` = 300 s**: `scouts.timeout_secs = 60` + three verification commands at `onboarding.verify_timeout_secs = 10` (30 s) + at most 40 engine git calls at `git_timeout_secs = 5` (200 s: preflight, two checkouts made, two salvages, two removals) = 290 s. M8b.11 counts the calls as landed and records the row in `docs/timing-budgets.md`; more than 40 calls raises the bound.
- **No test sleeps to synchronise.** Every wait is a deadline loop.
- **Confinement in tests.** Profile verification is confined on macOS like any check, so a test check that must write outside its checkout writes under `RunHarness::cache_dir()`, which the harness grants in `[orchestrator.cache_dirs]`. Off macOS the harness already sets `unconfined_checks = true`.

### Scenario map (spec §21)

| §21 scenario | Test here |
|--------------|-----------|
| A green S task on the fast path | `e2e_green_s_task_on_the_fast_path` (M8b.18) |
| *(M8b)* a decider that fails never blocks a run | `e2e_check_bounce_falls_back_to_the_tail_when_the_decider_fails` (M8b.18), `a_hanging_decider_falls_back_at_its_timeout_and_leaves_no_process` (M8b.7) |
| *(M8b)* the profile is proposed only from commands that ran | `e2e_detect_proposes_only_verified_commands` (M8b.11) |
| *(M8b)* a model-written profile cannot widen confinement | `a_profile_with_a_confinement_key_does_not_parse` (M8b.4), `verification_uses_the_users_confinement_not_the_proposal` (M8b.10) |

Every other §21 scenario is M8a's, and M8b must keep all of them green, with deciders off in the harness (decision 36).

### M8b.1 Verify the external facts and record the fixtures

**Files.** Create:
- `crates/daemon/tests/fixtures/deciders/claude-<version>-decider.jsonl` and `codex-<version>-decider.jsonl`;
- `crates/daemon/tests/fixtures/otlp/claude-<version>-metrics.json`;
- `crates/daemon/tests/fixtures/headless/claude-<version>-filter-hook.jsonl` and `claude-<version>-scout.jsonl`;
- a `.meta.json` beside each, with M8a's keys (`runtime`, `cli_version`, `captured`, `redactions`, `note`, `command`, `observed`).

Fill "Implementation notes".

**Tests first.** None. This task records facts. It is the only task that runs the real `claude` and `codex`, in `/tmp/anthrex-m8b1/`, on the implementer's own login, with M8a.1's scrubbed environment (`env -i` with only `HOME`, `PATH`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `LANG`). Personal paths become `/tmp/fixture`. If a probe would write the user's `~/.claude` or `~/.codex` (as a Codex `workspace-write` turn writes a trust entry, M8a F2), do not run it: record the item as outstanding for the user, as M8a's manual check 4e is.

**Change.** Record the command, the version and the relevant output for each item below:

1. **A Claude decider call.** Pipe one stream-json user message into `claude -p --input-format stream-json --output-format stream-json --verbose --permission-prompts none --setting-sources user --strict-mcp-config --permission-mode dontAsk --disallowedTools Bash,Edit,Write,NotebookEdit,Agent,WebFetch,WebSearch,Read,Glob,Grep --max-turns 3 --json-schema '<the blocked_reason schema>' --no-session-persistence --model claude-haiku-4-5`, then close stdin. Record:
   - which flags exist;
   - where the JSON answer appears: `result.structured_output`, a `StructuredOutput` tool use, or assistant text. This sets `DECIDER_CAPS.answer_source` and whether `SessionEvent::StructuredOutput` is added;
   - whether a schema without every property in `required` is refused (`DECIDER_CAPS.strict_schemas`);
   - the result's `usage`;
   - whether the process exits after `result`.
2. **A Codex decider call.** `codex exec --json --skip-git-repo-check --ephemeral -s read-only -c sandbox_workspace_write.network_access=false -c approval_policy="never" -c model_reasoning_effort="low" --output-schema <file> -- "<the blocked_reason prompt>"` in an empty directory. Record which flags exist, that the final `agent_message` text is the JSON, `turn.completed.usage`, and whether the run wrote anything to `~/.codex/config.toml` (compare its hash before and after).
3. **The filter hook under `-p`.** Use a scratch repository and decision 28's `--settings` with two `PreToolUse` groups: M3's recording command, and a `Bash` group whose script prints `updatedInput` with the command `echo rewritten > "$TMPDIR/anthrex-logs/probe"`. Add `--allowedTools Bash` and `--permission-prompts none`, M8a's worker sandbox block exactly as `claude_settings` writes it (writable roots and pins), and `TMPDIR` set to a directory in its writable roots. Ask for `echo original`. Record:
   - whether both groups fired;
   - whether the rewritten command ran (its tool result, and the probe file);
   - whether `permissionDecision: "allow"` was needed (`output_filter::HOOK_SETS_ALLOW`);
   - the payload's `tool_input` keys.
4. **Scout launch.** In a standalone scratch checkout, run a Claude session with `--permission-mode dontAsk --allowedTools mcp__anthrex__submit_scout_report,Bash,Read,Glob,Grep --disallowedTools Edit,Write,NotebookEdit` and a sandbox block with an empty `allowWrite` and M8a's pins (a stub MCP server that records the call). Record that `Bash` runs `ls` and `git log -1`, that `touch x` and a write under `$TMPDIR` are denied, that `curl https://example.com` fails, and that the MCP call is allowed (M8a.1 recorded the reviewer's; this confirms it with `Bash` allowed too).
5. **OTLP.** Run `claude -p "reply ok"` with `CLAUDE_CODE_ENABLE_TELEMETRY=1 OTEL_METRICS_EXPORTER=otlp OTEL_EXPORTER_OTLP_PROTOCOL=http/json OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:<port> OTEL_METRIC_EXPORT_INTERVAL=1000 OTEL_RESOURCE_ATTRIBUTES=anthrex.run=r-fix,anthrex.role=orchestrator` against a local listener that writes each request's headers and body. Record:
   - the path;
   - the content type;
   - `Content-Length` or chunked;
   - any compression;
   - the metric name;
   - the data point attributes and their `type` values;
   - `aggregationTemporality`;
   - that the resource attributes arrive.

   This fixes `orchestrator_env`'s exact list: the documented set above, plus `OTEL_EXPORTER_OTLP_COMPRESSION=none` if compression is on by default.

Also cross-reference M8a.1's record of stream usage field names (M8a's implementation notes, "M8a.1 external tools"). Add them here only if M8a.1 did not record them.

In `decider/argv.rs`, add `DeciderCaps`, `AnswerSource` and `DECIDER_CAPS` (Interfaces), set from items 1–2. A flag that does not exist is omitted from the argv.

A missing capability stops work on the item that needs it, as AGENTS.md says:
- `--json-schema` or `--output-schema` absent: that runtime's deciders rely on the prompt alone and `parse`, which still validates everything;
- the filter hook ignored, or the rewritten command unable to write under `$TMPDIR`: record it and skip the injection in M8b.8 (the hook, `filter-run` and their tests still land);
- a scout's MCP call refused under `dontAsk`: record it and stop M8b.9's launch until a ruling.

**Acceptance.** "Implementation notes" has a dated "M8b.1 external facts" entry covering all five items and every `DECIDER_CAPS` value, with any item not run listed as outstanding. The fixtures that could be recorded and their meta files exist.

**Commit.** `test(daemon): record the decider, filter-hook, scout and OTLP fixtures and the verified CLI facts`

### M8b.2 Protocol: scout role, profile, scout, adaptation and history types, and version 8

**Files.** Create `crates/proto/src/profile.rs`, `scout.rs`, `adapt.rs`, `history.rs`, `adapt_tests.rs`. Modify `crates/proto/src/lib.rs`, `run.rs`, `run_info.rs`, `run_wire.rs`, and every exhaustive `match` on `AgentRole`, `RunRequest` or `RunReply` in the workspace: `crates/mcp/src/tools.rs` (`tools_for`, `role_name`), `crates/daemon/src/headless/argv.rs` (`mcp_args`), `crates/cli/src/mcp_cmd.rs`, `crates/daemon/src/run/driver/requests.rs` (`request`), and the CLI's reply handling in `run_cmd.rs`. Until their tasks land, the daemon answers each new request with `Refused { request, message: "not available yet" }`, and `mcp_args` writes role `scout`.

**Tests first**, in `adapt_tests.rs`:
- `agent_role_scout_serializes_as_scout` (`"scout"`, under M8a's `snake_case`).
- `repo_profile_parses_the_spec_example`: spec §6's TOML block (with `protected` reduced to one extra entry, `.cursor/**`), plus `sample_test`, `manifests` and `filter_prefixes`, parses. `generated == ["Cargo.lock"]`, `output_filter == FailuresOnly`, and `env["CARGO_TARGET_DIR"] == "{worktree}/target"`.
- `repo_profile_rejects_unknown_keys`: `lint = "x"` fails, naming `lint`.
- `repo_profile_has_no_confinement_keys`: each of `cache_dirs = ["/tmp/c"]`, `confined_network = true`, `confined_unix_sockets = ["/tmp/s"]`, `confined_localhost_ports = [5432]` fails to parse, naming the key.
- `repo_profile_protected_defaults_to_no_extras`: a profile without `protected` parses to `protected == []`, and `spec().protected == None`.
- `repo_profile_spec_maps_every_shared_key`: `spec()` carries `modules`, `hub`, `source`, `generated`, `setup`, `check`, `check_timeout_secs`, `single_test`, `test_passed` and `env`, and an empty list gives `None`.
- `output_filter_kebab_case` (`"failures-only"`).
- `scout_report_round_trips_json_and_msgpack`, with a profile.
- `history_lines_are_tagged`: a `TaskRecord` serialises with `"type":"task"`, and a line with an extra unknown field still deserialises.
- `new_snapshot_fields_default_when_absent`: M8a's `RunInfo`, `TaskInfo` and `CheckInfo` JSON (taken from M8a's `run_tests_fixtures.rs`) without the new fields gives `None` and empty values.
- `every_new_request_and_reply_round_trips`: `StartGoal`, `Promote`, `Stats`, each `ProfileRequest`, `Triaged`, each `ProfileReply` and `Stats(HistoryStats)`, wrapped in `ClientMsg::Run` / `DaemonMsg::Run`, survive `rmp_serde::to_vec_named`.
- `tool_call_scout_id_defaults_to_none`.
- `token_usage_add_assign_is_field_wise`.
- Rename `proto_version_is_seven` to `proto_version_is_eight`.

**Change.** Add the Interfaces types. Update `PROTO_VERSION`'s doc comment with the derivation.

**Acceptance.** Tests pass. The workspace builds. `PROTO_VERSION == 8`, derived in "Implementation notes".

**Commit.** `feat(proto): add the scout role and the profile, scout, triage, usage and history types (protocol 8)`

### M8b.3 Configuration

**Files.** Create `crates/config/src/orchestrator/adapt.rs` and `crates/config/src/orchestrator_tests_adapt.rs` (included from `orchestrator_tests.rs` by `#[path]`, as `orchestrator_tests_confine.rs` is). Modify `crates/config/src/orchestrator.rs`, `orchestrator/unknown.rs`, and the one `pub use orchestrator::{…}` line of `lib.rs`.

**Tests first:**
- `adapt_defaults_when_absent`: every default in Interfaces.
- `adapt_keys_are_read` (every key set to a value different from its default).
- `adapt_out_of_range_values_fall_back`: `deciders.timeout_secs = 4`, `= 601`, `slot_wait_secs = 601`, `scouts.timeout_secs = 59`, `max_tool_calls = 9`, `verify_timeout_secs = 9`, `otlp_port = 70000`. Each gives one problem with the exact message and keeps the default.
- `decider_mode_values`: `claude`, `codex` and `off` are accepted; `gpt` is a problem.
- `scouts_runtime_shell_is_a_problem`.
- `unknown_adapt_keys_are_reported` (`orchestrator.deciders.model` and `orchestrator.onboarding.cache_dirs` each give `unknown key, ignored`).
- `fast_path_false_is_read`.
- `the_dotted_and_table_forms_read_alike` (`deciders.mode = "off"` under `[orchestrator]` and `[orchestrator.deciders] mode = "off"`).

**Change.** Decision 3.

**Acceptance.** Tests pass. `git diff --stat baa04e1 -- crates/config/src/lib.rs` shows one line changed, none added.

**Commit.** `feat(config): add the deciders, scouts, onboarding, metering and fast_path settings`

### M8b.4 Profile store and precedence

**Files.** Create `crates/daemon/src/profile/mod.rs`, `profile/resolve.rs`, `profile/proposal.rs` (types and `validate`, `from_findings`, `apply_edit` only), `profile/store.rs`, `profile/tests.rs`, `crates/daemon/tests/profile_store.rs`. Create `crates/daemon/src/run/driver/adapt.rs` with `choose_profile`. Modify `crates/daemon/src/lib.rs` (`pub mod profile;`), `run/driver.rs` (`mod adapt;`), `run/driver/requests.rs` (split `build` into `build` + `build_plan`; call `adapt::choose_profile` after preflight; the protected-files scan uses the chosen profile), `run/plan.rs` (`for_repo` → `pub(crate)`), and `run/model.rs` + new `run/model_adapt.rs` (`Run.profile_source`, `output_filter`, `filter_prefixes`, `repo_dir`, `stale_profile`).

**Tests first:**
- In `profile/tests.rs` (pure):
  - `stored_profile_replaces_the_plan_profile_entirely`. Stored sets `check = "a"` only; the plan sets `check = "b"` and `single_test = "t {test}"`. The chosen spec has `check == Some("a")` and `single_test == None`, source `Stored`, and two notes with the exact text naming `check` and `single_test`.
  - `a_stored_profile_leaves_no_gap_for_the_config`: stored sets `check` only; `[orchestrator.profile]` sets `single_test` and `setup`. Through `choose_profile`'s config clone and M8a's `resolve_profile`, the resolved `Profile` has `single_test == None` and `setup == None`.
  - `without_a_stored_profile_m8a_rule_holds`: the plan's `check` over the config's `single_test`, source `Plan`.
  - `nothing_anywhere_is_source_none`.
  - `filter_fields_only_come_from_a_stored_profile`.
  - `builtins_always_apply_whatever_the_stored_list`. The stored `protected` is `[]` in one case and `[".cursor/**"]` in another, the plan has `[profile] protected = ["docs/agents/**"]` and the config `[orchestrator.profile] protected = ["GEMINI.md"]`. Through `run_profile`, `choose_profile`'s config clone and M8a's `resolve_profile`, the resolved `Profile.protected` contains every entry of `BUILTIN_PROTECTED`, plus `.cursor/**` when stored, plus `docs/agents/**` and `GEMINI.md`. There is no "ignored" note for `protected`.
  - `confinement_still_comes_from_the_users_config`: with a stored profile, `build_run` (through `choose_profile`) gives the run the config's `cache_dirs` and `confined_network` entry for its root, and a different root's entry is not taken.
  - `derived_prefixes_from_check_and_single_test`: check `cargo build --all && cargo test -q; cargo fmt --check` and `single_test` `cargo test -- --exact {test}` give `["cargo build", "cargo test", "cargo fmt"]`.
  - `from_findings_keeps_only_extra_protected_entries`: a scout reporting `[".claude/**", "**/AGENTS.md", ".cursor/**", ".cursor/**"]` gives `[".cursor/**"]`.
  - `from_findings_drops_reserved_env_keys`: `PATH`, `HOME`, `BASH_ENV`, `PYTHONPATH` and `ANTHREX_X` are dropped, `PYTHONUNBUFFERED` and `RUST_LOG` kept (M8a's `reserved_env`).
  - `edit_of_protected_refuses_builtins`. `edit protected '[".mcp.json"]'` is refused with the exact message. `edit protected '[".cursor/**", "GEMINI.md"]'` stores exactly those two, and `--unset protected` leaves no extras, with every built-in still in force (checked through `run_profile` plus M8a's resolution).
  - `edit_refuses_confinement_keys`: `edit cache_dirs '["/tmp"]'` and `edit confined_network true` are refused as `unknown key <key>; one of <keys>`.
  - `validate_reports_each_bad_key` (a glob with `..`, an absolute `protected` entry, `single_test` without `{test}`, `test_passed` without `{test}`, `test_passed` `(` that does not compile, env key `1X`, env key `PATH`, `generated` absolute), each with its exact message.
- In `crates/daemon/tests/profile_store.rs`, on a `TempRepo`:
  - `repo_dir_is_shared_by_linked_worktrees` (the main checkout and a linked worktree give one `repo_dir`, `<data>/repos/<basename>-<hash8>`).
  - `save_and_load_round_trip_atomically`: a leftover `profile.toml.tmp` is ignored and removed.
  - `an_unparseable_profile_is_reported_with_its_path`.
  - `a_profile_with_a_confinement_key_does_not_parse`: a hand-written `profile.toml` with `cache_dirs = ["/"]` loads as `Unparseable`, naming `cache_dirs`.
  - `fingerprint_changes_with_content_length_and_absence`.
  - `stale_lists_exactly_the_changed_files`.
  - `nothing_is_written_inside_the_repository`: after save, `git status --porcelain --ignored` in the repo is empty.
- The decision-6 driver path is tested end to end in M8b.19 (`e2e_stored_profile_wins_over_the_plan_profile`, `e2e_a_corrupt_stored_profile_refuses_the_run`).

**Change.** Decisions 4–7 (the pure and store parts), and decision 6 in `build`. `Run.repo_dir` is set for every run. The stale attention line is added at start when `stale` is non-empty.

**Acceptance.** Tests pass. `profile/resolve.rs` and `profile/proposal.rs` are pure (the Verification grep). Every M8a test still passes.

**Commit.** `feat(daemon): store the repository profile in the data directory and let it supersede a plan's profile`

### M8b.5 Deciders I: schemas, prompts, parsing, fallbacks and argv

**Files.** Create `crates/daemon/src/decider/{mod,schema,prompt,parse,fallback,argv}.rs` and `decider/tests.rs`. Modify `crates/daemon/src/lib.rs`, and `crates/daemon/src/headless/mod.rs` (`credential_scrub_for`).

**Tests first:**
- `every_schema_is_closed_and_strict`: every object has `additionalProperties: false`, and every property is listed in `required`.
- `schemas_match_the_brief`: each equals the Interfaces JSON.
- Prompts:
  - `prompts_start_with_the_kind_line`;
  - `triage_prompt_is_exact_for_a_fixed_input` (a golden string in the test);
  - `sections_with_empty_inputs_are_omitted`;
  - `oversized_inputs_are_cut_in_order_with_the_marker`: 5000 tracked paths are cut first and the prompt stays within `PROMPT_MAX_BYTES`;
  - `prompt_is_deterministic`.
- Parsing:
  - `parse_accepts_each_valid_answer`;
  - `triage_single_without_task_is_an_error`;
  - `triage_task_size_l_is_an_error`;
  - `size_check_ignores_unknown_ids_and_reports_bad_sizes` (the exact path message);
  - `check_summary_over_40_lines_or_300_chars_is_an_error` (300 characters of the 3-byte `世` accepted, 301 refused);
  - `blocked_reason_unknown_kind_is_an_error`.
- Answer extraction (`answer_from_events`):
  - `structured_output_event_wins`;
  - `structured_output_tool_use_is_read`;
  - `assistant_text_in_a_fence_is_parsed`;
  - `subagent_text_is_ignored` (`parent: Some(..)`);
  - `no_answer_is_an_error`.
- Fallbacks:
  - `fallback_table`: each kind's deterministic answer, with `check_summary` equal to `run::exec::summary(tail)`;
  - `fallback_decision_carries_the_reason`.
- Argv:
  - `claude_decider_args_exact`, for two `DeciderCaps` variants, with the caps' user-settings flags present and absent, and no `--settings`, `--bare` or `--mcp-config` in either;
  - `codex_decider_args_exact` (`-s read-only` and every `CODEX_SANDBOX_PINS` entry present);
  - `schema_file_name_is_stable`;
  - `decider_credential_scrub_removes_every_api_credential` (`credential_scrub_for(Claude, Login)` and `(Codex, Login)` each include `ANTHROPIC_API_KEY` and `OPENAI_API_KEY`; `credential_scrub(spec)` gives the same as before for M8a's specs).

**Change.** Decisions 16 and 17, the pure parts.

**Acceptance.** Tests pass. Every file in `decider/` except `call.rs` is pure.

**Commit.** `feat(daemon): add decider schemas, prompts, answer parsing and deterministic fallbacks`

### M8b.6 `fake-agent`: decider mode, scout scripts, the `bash` step, every hook group

**Files.** Create `crates/fake-agent/src/decider.rs`, `src/bash.rs`, `crates/fake-agent/tests/adapt_modes.rs`. Modify `crates/fake-agent/src/main.rs` (dispatch), `headless.rs` (the Claude first-message check), `runtime.rs`, `script.rs`, `roles.rs`, `headless_steps.rs`.

**Tests first**, in `adapt_modes.rs`:
- `decider_mode_answers_in_the_claude_shape` and `decider_mode_answers_in_the_codex_shape`: the scripted answer is found by M8b.5's `answer_from_events` over the output parsed with M8a's parsers, with usage.
- `decider_output_conforms_to_the_fixtures` (M8a decision 51's key-set test, `headless_support/shape.rs`, against M8b.1's decider fixtures; a fixture M8b.1 could not record is skipped with a printed reason).
- `decider_mode_is_chosen_by_the_prompt_not_the_flags` (no `--json-schema` in argv, prompt `[anthrex decider] triage v1`).
- `decider_calls_are_recorded` (kind, argv, prompt in `calls.jsonl`).
- `decider_without_a_script_exits_2`.
- `decider_hang_blocks_until_killed`.
- `decider_scripts_are_claimed_in_order`.
- `scout_scripts_are_claimed_by_scout_id`: `scout-onboarding-1.jsonl` serves `--scout onboarding-1695000000`, found from a standalone checkout through its alternate.
- `bash_step_applies_updated_input`: a settings JSON with two `PreToolUse` groups, the second a script printing `updatedInput` with `echo rewritten`. `FAKE_AGENT_BASH_LOG` shows `original == "echo original"` and `ran == "echo rewritten"`, and the emitted `tool_use` input is the rewritten command.
- `bash_step_without_hooks_runs_the_command`.
- `every_hook_group_is_discovered_and_hook_keeps_the_first`.
- Milestone 3's and M8a's `fake-agent` tests still pass.

**Change.** Decisions 36 and 37.

**Acceptance.** Tests pass.

**Commit.** `feat(fake-agent): add a scripted decider mode, scout scripts, and a bash step that honours PreToolUse hooks`

### M8b.7 Deciders II: the call

**Files.** Create `crates/daemon/src/decider/call.rs`, `crates/cli/tests/decider_call.rs`. Modify `crates/daemon/src/manager/config.rs` (`decider_bin: Option<String>` from `ANTHREX_DECIDER_BIN` in `from_vars`), and `crates/daemon/src/headless/claude_stream.rs` + `headless/mod.rs` (only if M8b.1 found `result.structured_output`).

**Tests first**, in `decider_call.rs`, with real `fake-agent` processes. `DeciderContext.program` is `fake_agent_bin()` and the timeout is 5 s.
- `claude_mode_returns_the_scripted_answer_and_usage`.
- `codex_mode_returns_the_scripted_answer`.
- `the_prompt_goes_on_stdin_for_claude_and_last_for_codex`, from `calls.jsonl`.
- `decider_bin_wins_over_the_runtime_command` (`ManagerConfig::from_vars` with both set).
- `mode_off_never_spawns`: the program is a script that creates a marker file, and no marker appears.
- `a_hanging_decider_falls_back_at_its_timeout_and_leaves_no_process`. The call returns after at least 5 s and within `5 + 2 (kill grace) + 5 (slack) = 12 s`, with reason `the decider timed out after 5 s`. `pgrep -f <unique marker in the prompt>` then finds nothing within a 5 s deadline loop (the test signals nothing; it only looks).
- `garbage_empty_and_oversized_answers_fall_back`: text `not json`, text `{}` (a schema error naming `kinds`), and 300 KiB of text (cut to `ANSWER_MAX_BYTES`, then not JSON). Each gives its exact reason.
- `a_failed_turn_and_an_early_exit_fall_back`: `fail_turn` and `exit 3`, each with its reason.
- `a_missing_program_falls_back_with_could_not_start`.
- `the_decider_sees_no_api_credentials`: with `ANTHROPIC_API_KEY` set in the test process's command for the spawn (never in the test's own environment), the decider's recorded environment lacks it (`fake-agent` records its env keys when `FAKE_AGENT_ENV_FILE` is set; add that to M8b.6 if absent).

**Change.** Decision 16's I/O. The call never holds a lock, spawns on `spawn_blocking`, writes the schema file on `spawn_blocking`, and kills the process group (`HeadlessHandle::kill`) on every early return.

**Acceptance.** Tests pass. Add rows to `docs/timing-budgets.md` for the 12 s bound (`timeout 5 s + kill grace 2 s + spawn slack 5 s`) and for `GOAL_REQUEST_TIMEOUT`.

**Commit.** `feat(daemon): call deciders as one-shot headless sessions with bounded time and deterministic fallbacks`

### M8b.8 The output filter

**Files.** Create `crates/daemon/src/output_filter.rs`, `crates/cli/src/pre_clap.rs`, `crates/cli/src/filter_run.rs`, `crates/cli/src/filter_hook.rs`, `crates/cli/tests/filter_run.rs`, `crates/cli/tests/filter_hook.rs`. Modify `crates/cli/src/main.rs` (the pre-clap dispatch moves to `pre_clap::dispatch`, which handles `hook`, `filter-run` and `filter-hook`), `crates/daemon/src/headless/{mod.rs,argv.rs}` (`HeadlessSpec.output_filter`, the `add_hook` call), and `run/role_launch.rs` (`worker_spec` sets `output_filter` with `log_dir = task_tmp_dir(..).join(LOG_DIR_NAME)`).

**Tests first:**
- In `output_filter.rs`:
  - `failures_only_keeps_matches_with_context_and_the_tail`: 5000 lines with one `test foo ... FAILED` at line 2500 and the summary at the end give that line, its 5 followers, the last 20 lines and one omission marker per gap, 28 lines exactly.
  - `failures_only_on_success_is_the_last_10_lines`.
  - `failures_only_without_matches_is_the_last_60`.
  - `at_most_100_matched_lines`.
  - `tail_is_60_lines`.
  - `long_lines_are_cut_to_500_chars` (with the 3-byte `世`).
  - `matches_strips_cd_and_env_prefixes_and_respects_word_boundaries` (`cargo testing` does not match `cargo test`).
  - `rewrite_keeps_every_tool_input_key`.
  - `rewrite_never_double_wraps`.
  - `add_hook_appends_a_second_group` (the exact settings JSON; M3's group, the sandbox block and its pins unchanged).
- In `filter_run.rs`, running the built binary:
  - `filter_run_preserves_exit_status_and_signals`: exit 3 gives 3; a child killed by `SIGTERM` gives 143 (the test kills only the child pid it spawned).
  - `filter_run_writes_the_full_log_and_prints_the_filtered_view`: 5000 lines logged, at most 32 printed, and the last line names the log path and `5000 lines, exit 1`.
  - `filter_run_passes_stdin_through`.
  - `filter_run_still_runs_when_the_log_cannot_be_written` (log dir under a read-only directory).
- In `filter_hook.rs`:
  - `hook_wraps_a_matching_command` (exact stdout JSON).
  - `hook_is_silent_for_other_commands_and_tools`.
  - `wrapped_commands_behave_exactly_like_the_original`. For each of these commands, running the wrapped command through `/bin/sh -c` prints the same `filter-run` log contents and exit code as the original: `printf 'a b\n'`; `echo "it's"`; a two-line command with a heredoc; `false || echo ok`; `FOO=1 sh -c 'echo $FOO'`; `cd sub && ls`; a command with `ü` and a tab.
  - `hook_exits_zero_silently_on_garbage_and_oversized_input`.
  - `hook_finishes_within_its_deadline`: stdin held open, it exits within `FILTER_HOOK_DEADLINE + 2 s`. Add the row to `docs/timing-budgets.md`.
- In `headless/argv_tests.rs` and `run/role_launch.rs`'s tests:
  - `claude_worker_settings_include_the_filter_hook`;
  - `reviewers_scouts_and_codex_get_no_filter_hook`;
  - `worker_spec_sets_the_hook_only_for_a_stored_profile_with_prefixes`;
  - `the_filter_hook_adds_no_writable_root`: the worker's `ClaudeSandbox.writable_roots` equal M8a's `worker_git_roots`, and `log_dir` is under the task's `TMPDIR`.

**Change.** Decisions 26–28.

**Acceptance.** Tests pass. `anthrex --help` lists neither `filter-run` nor `filter-hook`. `crates/cli/src/main.rs` is at most 600 lines.

**Commit.** `feat: filter test output for headless Claude workers through a PreToolUse hook and anthrex filter-run`

### M8b.9 Scout sessions and `submit_scout_report`

**Files.** Create `crates/daemon/src/scout/{mod,contract,spec,report,machine,service}.rs`, `scout/tests.rs`, `crates/mcp/src/tools_scout.rs`, `crates/cli/tests/scout_service.rs`. Modify:
- `crates/mcp/src/{lib.rs,tools.rs,forward.rs}` (`scout_id`, the scout role);
- `crates/cli/src/mcp_cmd.rs` (`--role scout`, `--scout`, `--run` optional for scouts);
- `crates/daemon/src/headless/{mod.rs,argv.rs}` (`McpTarget.scout_id`, `mcp_args`);
- `crates/daemon/src/manager/headless.rs` (`control_refusal` for `run == None`);
- `run/driver/requests.rs` (route `Tool` calls with role `Scout` to `ScoutService::tool` through the adaptation handle).

**Tests first:**
- In `scout/tests.rs` (pure):
  - `area_scout_spec_is_read_only` (the exact `HeadlessSpec`: `dontAsk`, the reviewer's disallowed tools, a `ClaudeSandbox` with no writable roots and the five protected denials, read-only Codex sandbox, the four tools, `run_ref` set);
  - `onboarding_scout_spec_is_read_only_too` (`Bash` allowed, no `Edit`/`Write`, the same empty-root `ClaudeSandbox`, `run_ref == None`, Codex `read-only`);
  - `the_scout_sandbox_does_not_depend_on_worker_sandbox` (the same spec with `worker_sandbox` false);
  - `scouts_pass_the_user_settings_flags_and_codex_pins`: through `claude_args` the argv has the caps' user-settings flags and the sandbox pins; through `codex_args` every `CODEX_SANDBOX_PINS` entry and `-s read-only`. With a test `CliCaps` whose `codex_user_config_only` is `Some(["--flag"])`, both the Codex scout argv and `codex_decider_args` contain it; with `None`, neither does;
  - `a_codex_scout_carries_the_codex_config_guard_when_codex_loads_project_config`;
  - `route_picks_the_lowest_strength_at_or_above` (default roster: Claude `fast` gives `claude-haiku-4-5`; Codex `fast` gives Codex `""`);
  - `valid_ids`;
  - `report_validation_cases`: a missing summary, 61 files, a profile on an area scout, a missing profile on onboarding, a bad env key, a `cache_dirs` key inside `profile` (refused: unknown property), each with the exact message;
  - `machine_nudges_once_then_fails`;
  - `machine_wraps_up_then_kills_at_1_5x_the_tool_budget` (budget 11: wrap-up at the 11th, kill at the 17th);
  - `machine_times_out`;
  - `machine_exit_without_report_fails`;
  - `machine_report_closes_stdin_then_kills_and_removes`;
  - `machine_sums_usage`.
- In `crates/mcp`:
  - `scout_tool_is_submit_scout_report`;
  - `scout_schema_limits` (and no property named `cache_dirs` or starting with `confined_` at any level);
  - `mcp_args_for_a_scout` (the exact vector `["mcp","--role","scout","--scout","onboarding-1","--window","7","--socket","/tmp/a.sock"]`);
  - `run_is_required_except_for_scouts` (in `mcp_cmd.rs`'s tests, with the daemon's argv round trip extended to a scout).
- In `crates/cli/tests/scout_service.rs`, with a real `WindowManager` whose `claude_bin` is `fake-agent`, and `submit_scout_report` routed through `ScoutService::tool`:
  - `scout_report_is_stored_and_the_session_retired`: the report file exists with the fields, the window is `Exited`, then removed after `RETIRE_AFTER`, within a deadline of `RETIRE_AFTER + INTERRUPT_GRACE + 5 s`;
  - `scout_without_a_report_is_nudged_then_failed`: the second turn's stdin line is `SCOUT_NUDGE`;
  - `a_report_from_another_window_is_refused`;
  - `a_second_report_is_refused`;
  - `the_scout_argv_is_read_only`: the recorded argv (`FAKE_AGENT_ARGS_FILE`) has `--permission-mode dontAsk`, `--disallowedTools Edit,Write,NotebookEdit` and a `--settings` sandbox block with an empty `allowWrite`;
  - `a_run_less_scout_window_refuses_client_input_with_the_profile_hint` (a real socket `Input`, `Kill` and `Subscribe`).

**Change.** Decisions 12–15.

**Acceptance.** Tests pass. `ScoutService` never holds `daemon::lock` across a manager call or an await (state it in the pull request).

**Commit.** `feat: add read-only headless scouts with submit_scout_report, stored reports and a pure lifecycle machine`

### M8b.10 Onboarding I: proposal rules and confined verification

**Files.** Create `crates/daemon/src/profile/verify.rs`, `crates/cli/tests/profile_verify.rs`; complete `profile/proposal.rs`. Modify `run/exec.rs` (`run_matching` → `pub(crate)`).

**Tests first:**
- In `profile/tests.rs`:
  - `verification_drops_commands_that_did_not_pass`: setup failing, check passing, single test passing, gives setup dropped and the others kept;
  - `single_test_needs_sample_and_a_matching_line`: exit 0 without the line drops `single_test`, `test_passed` and `sample_test` with the reason;
  - `a_confined_drop_carries_the_confinement_hint` (exact text, naming the root), and an unconfined one does not;
  - `edit_value_parsing`: `check 'cargo test'` as a string, `modules '["crates/*"]'` as an array, `check_timeout_secs 600` as an integer, `env.RUST_LOG debug` as an env entry, `--unset single_test`, and `lint x` refused as `unknown key lint; one of <keys>`;
  - `edit_rejects_invalid_values_with_the_rule` (M8b.4's `validate` messages);
  - `edit_of_a_command_key_needs_reverification_and_of_modules_does_not`;
  - `show_text_is_exact` (golden, confined and unconfined headers).
  - `confine_spec_takes_the_users_tables_for_the_root`: `confine_spec` with `worker_sandbox` true on a confining platform gives the config's `cache_dirs`, `confined_network`, `confined_unix_sockets` and `confined_localhost_ports` for `pre.root`, `data_dir == repo_dir`, and `None` with `worker_sandbox` false.
- In `crates/cli/tests/profile_verify.rs`, on a repository from `init_repo`, driving `profile::verify` directly:
  - `verification_never_touches_the_checkout_and_salvages_dirt`. `check` modifies a tracked file, creates an untracked one and exits 0. Afterwards the user's checkout is clean and unchanged (`git status --porcelain` empty, same `HEAD`), the user's `.git/worktrees` has no entry, `<wt>/runs/.profile-verify` and `<repo_dir>/tasks/.profile-verify` are gone, and a salvage ref under `refs/anthrex/salvage/onboarding/` holds the change.
  - `a_hanging_verification_command_times_out_and_is_dropped` (`sleep 60` with a 2 s timeout; `timed_out`, dropped).
  - `env_and_worktree_are_substituted` (`check` prints `$CARGO_TARGET_DIR`, which is under the scratch path).
  - `the_engine_environment_applies` (`check` prints `env`; no `CLAUDE_CODE_*`, `ANTHROPIC_API_KEY` or `ANTHREX_SOCKET` — each set on the test's `Command` for the daemon library call's process, which runs alone in its own test binary as M8a's env tests do).
  - `verification_uses_the_users_confinement_not_the_proposal` (macOS only; skipped elsewhere with a printed reason): a `check` that writes `$HOME/pwned-<pid>` and one that writes under a directory the test lists in `[orchestrator.cache_dirs]` for the root. The first is dropped and `$HOME/pwned-<pid>` does not exist; the second is kept. A proposed `env` cannot change this (there is no key for it).
  - `a_confined_setup_without_network_is_dropped_with_the_hint` (macOS only): `setup = "curl -s https://example.com"`; dropped, with the hint.

**Change.** Decision 9, and the pure parts of decision 10.

**Acceptance.** Tests pass. `profile/proposal.rs` is pure. No code path writes under the repository root (review every `write_atomic`, `run_matching` and git write caller, and state it in the pull request).

**Commit.** `feat(daemon): verify proposed profile commands in a fresh confined checkout and keep only those that pass`

### M8b.11 Onboarding II: the profile service, detection, restart and `anthrex profile`

**Files.** Create `crates/daemon/src/profile/service.rs`, `crates/cli/src/profile_cmd.rs`, `crates/cli/tests/profile_cli.rs`, `crates/cli/tests/support/run_adapt.rs` (the helpers of "Shared test conventions" this task needs). Modify:
- `run/driver.rs` and `run/driver/adapt.rs` (`Adaptation`, `set_adaptation`, `git_queue`; `RunRequest::Profile` → `ProfileService::request`);
- `run/driver/requests.rs` (the `Profile` arm);
- `lifecycle.rs` (construct `ScoutService` and `ProfileService` after `RunService::new`, `set_adaptation`, `ProfileService::restore` after `RunService::restore` and before the socket binds);
- `crates/cli/src/main.rs` (`Command::Profile`);
- `crates/cli/tests/support/run_harness.rs` (decision 36's defaults, `pub(super)` fields).

**Tests first:**
- In `profile_cli.rs`, with `RunHarness`, `onboarding.auto` true, `onboarding.verify_timeout_secs = 10`, and a repository with `check.sh`, `tests/t_ok.sh` (prints `PASS t_ok`) and `Cargo.lock`:
  - `e2e_detect_proposes_only_verified_commands`. The scout script reports `check = "sh check.sh"`, `single_test = "sh tests/{test}.sh"`, `test_passed = "PASS {test}"`, `sample_test = "t_ok"`, `setup = "sh missing.sh"`, `generated = ["Cargo.lock"]` and `protected = [".cursor/**"]`. Within `PROFILE_WAIT` the status is `Ready`. `profile show --proposed` has `check`, `single_test`, `generated`, and `protected: built-in … + .cursor/**`, lists `setup` under dropped with its exit code, and its verification header says `confined` on macOS.
  - `e2e_confirm_stores_the_profile_outside_the_repository`: `profile.toml` is under `<data>/repos/`, `git status --porcelain --ignored` in the repo is empty, there is no `.anthrex` path, and `git worktree list` shows only the user's checkout.
  - `e2e_reject_stops_a_running_scout` (a scout script that hangs; reject; the window is gone and the proposal is deleted).
  - `e2e_edit_goes_through_verification` (`edit check 'sh broken.sh' --yes`: the proposal ends `Ready` with `check` dropped and is **not** auto-confirmed; the stored `check` is unchanged).
  - `e2e_status_reports_stale_files_and_auto_detects` (change `Cargo.lock` after confirm: `stale == ["Cargo.lock"]`, and a new proposal starts).
  - `e2e_detection_in_progress_at_restart_is_failed_and_cleaned` (a scout that hangs; `restart_daemon`; the proposal is `Failed` with the exact reason, both checkouts and their repositories are gone, and no `scout/…` window is listed).
  - `e2e_detect_refuses_codex_project_config_without_trust_project`: `scouts.runtime = "codex"`, `ANTHREX_TEST_CODEX_PROJECT_CONFIG=load`, and the repo tracks `.codex/config.toml`. Detect is refused naming it, and `--trust-project` starts detection.
  - `e2e_detect_refuses_project_settings_without_trust_project`: the daemon runs with `ANTHREX_TEST_NO_SETTING_SOURCES=1` and the repo tracks `.claude/settings.json` with `hooks`. Detect is refused with the exact text, and `--trust-project` starts detection.
  - `e2e_a_second_detect_is_refused_while_one_runs`.
  - `e2e_detect_refuses_an_unconfinable_platform_without_the_flag`: the daemon runs with `ANTHREX_CHECK_CONFINEMENT=unavailable` and the test's config has no `unconfined_checks` (the harness adds it only off macOS; this test runs on macOS only). Detect is refused with `start_refusal`'s text; `--unconfined-checks` starts it, and `status` says `verification: unconfined`.

**Change.** Decisions 8, 10 and 11.

**Acceptance.** Tests pass. Record `PROFILE_WAIT`'s landed git-call count in `docs/timing-budgets.md`. `ProfileService` never holds `daemon::lock` across an await or a manager call.

**Commit.** `feat: detect, verify and confirm the repository profile with a read-only onboarding scout`

### M8b.12 Engine deciders I: the queue, reader slots, check summaries and block classification

**Files.** Create `run/engine/deciders.rs`, `run/engine/tests/deciders.rs`. Modify `run/engine/{mod,ops,gates,merge,done,tools,schedule,restore}.rs`, `run/model_adapt.rs`, `run/model_rounds.rs` (`CheckRecord.summary`, `summary_source`), `run/snapshot.rs`, `run/reconcile/mod.rs`, `run/contract.rs` (`check_failed_message`, `candidate_red_message`, `reviewer_prompt`, the classifying reply), `run/report.rs`, `run/plan.rs` (`RunLimits.decider_mode`, `decider_slot_wait_secs`), `run/driver/ops.rs` + `run/driver/adapt.rs` (execute `Decide`: `decide` with the adaptation's `DeciderContext`), and the fixture (`fx.decided`, `fx.queued_deciders`).

**Tests first**, in `engine/tests/deciders.rs`:
- `failed_check_defers_the_bounce_until_the_summary`: `Check { ok: false }` gives `failures == 1` and one `Decide` op, and no `Deliver`. `Decided` gives a `Deliver` whose text contains `Summary of its output:` and the summary lines and not the raw tail, with `summary_source == Decider`.
- `summary_fallback_uses_the_tail_and_is_marked` (M8a's exact text).
- `candidate_red_uses_the_summary_and_the_queue_moves_on`: the next queued task's `MergeCandidate` is emitted before `Decided`.
- `deciders_take_a_reader_slot_before_reviewers`: `max_readers = 1`, a review pending and a decider queued; the decider starts first and `readers_busy == 1`.
- `a_decider_waiting_past_slot_wait_falls_back_on_tick`.
- `mode_off_decides_inline_without_an_op`.
- `an_unclassified_block_is_classified`: three cases (`question` stays, `environment` changes the reason, `mis_sized` gives rung 3 with the size raised), plus the reply text `Blocked recorded (classifying). …`.
- `a_typed_kind_is_never_reclassified`.
- `an_answer_before_classification_wins`.
- `a_restart_requeues_a_dropped_decider_op` (`Restore` with the `Decide` op answered `NotStarted`, then the resume: queued again).
- `decider_usage_is_counted_on_task_and_run`.
- `reviewer_prompt_uses_the_latest_summary`.

In `reconcile` tests: `decide_is_not_started`.

**Change.** Decisions 18, 20 and 21.

**Acceptance.** Tests pass. `run/engine/deciders.rs` is pure. Every M8a engine test still passes (with `decider_mode` off in `Fixture::new`'s default config, so no M8a test sees a `Decide` op).

**Commit.** `feat(daemon): run check summaries and block classification as decider ops in reader slots`

### M8b.13 Engine deciders II: the size cross-check

**Files.** Modify `run/engine/deciders.rs`, `run/engine/{requests,dispatch,schedule}.rs` (Start and Edit issue the check; pending tasks are not runnable), `run/engine/ladder.rs` (`reresolve` → `pub(crate)`), `run/driver/adapt.rs` (resolve `evidence_refs` into `Evidence` by reading report files on `spawn_blocking` before `decide`), `run/report.rs`, `run/snapshot.rs`. Tests in `run/engine/tests/deciders_size.rs`.

**Tests first:**
- `size_check_raises_s_to_m_and_rederives_route_budget_and_review` (effort `low` → `medium` when the plan set none; the budget M's; the review level `medium`; `raised_size == Some(M)`; the note text).
- `a_planner_set_effort_survives_a_raise`.
- `an_amend_never_lowers_a_cross_check_raise`.
- `size_check_l_blocks_as_mis_sized_without_a_rung`.
- `size_check_never_lowers_and_records_agreement`.
- `missing_ids_keep_their_size_as_fallback`.
- `a_pending_size_check_blocks_dispatch`.
- `tasks_without_evidence_skip_the_cross_check` (note text; no op).
- `an_edit_cross_checks_only_the_touched_tasks`.
- `the_driver_resolves_evidence_from_report_files` (a driver-level test with two report files, one named by `scout_refs` and the `onboarding` alias).

**Change.** Decision 19.

**Acceptance.** Tests pass. Every M8a engine test still passes.

**Commit.** `feat(daemon): cross-check planned task sizes against scout evidence with a decider that can only raise`

### M8b.14 Triage, the fast path and `run promote`

**Files.** Create `run/triage.rs`, `run/triage_tests.rs`, `crates/cli/src/run_cmd/adapt.rs`. Modify `run/driver/adapt.rs` (`StartGoal`), `run/driver/requests.rs` (the arms), `run/engine/{mod,requests}.rs` (`Promote`), `run/report.rs` (path line), `run/snapshot.rs`, `crates/cli/src/run_cmd.rs`, `run_cmd/status.rs`.

**Tests first:**
- In `triage_tests.rs`:
  - `single_code_goal_takes_the_fast_path` (the `PlanTask` fields exactly);
  - `plan_and_large_scales_route_with_the_reason`;
  - `mixed_or_research_kinds_are_not_fast` (exact reasons);
  - `fast_path_disabled_routes_plan`;
  - `fallback_triage_routes_plan_with_the_reason`;
  - `fast_plan_builds_a_valid_run` (through M8a's `build_run` with a stored profile: one task, `approved_by` overwritten to `fast path`);
  - `a_hub_task_falls_back_to_plan` and `an_l_task_falls_back_to_plan` (the reasons);
  - `messages_are_exact` (`started_message`, `refused_message`).
- In the engine tests:
  - `a_fast_path_run_starts_running_without_a_gate`;
  - `promote_records_intent_once` (the second request replies `already marked`, and no task, op or window changes);
  - `promote_refuses_plan_runs_and_terminal_runs`;
  - `a_fast_path_task_skips_the_cross_check`.
- In `run_cmd` unit tests:
  - `goal_and_plan_are_mutually_exclusive_and_one_is_required`;
  - `goal_uses_the_goal_request_timeout`;
  - `status_shows_fast_path_and_triage`.

**Change.** Decisions 22–25.

**Acceptance.** Tests pass. `anthrex run --help` lists `promote` and `stats`.

**Commit.** `feat: triage goals with a decider and run single-task goals on the fast path`

### M8b.15 Metering: usage by role and the OTLP receiver

**Files.** Create `crates/daemon/src/metering/{mod,otlp,server}.rs`, `metering/otlp_tests.rs`, `crates/daemon/tests/otlp_server.rs`. Modify `run/snapshot.rs` (`RunInfo.usage`), `run/engine/mod.rs` (`OrchestratorUsage`), `run/driver/adapt.rs` (`orchestrator_usage`), and `lifecycle.rs` (bind, `otlp.addr`, shutdown removal).

**Tests first:**
- In `otlp_tests.rs`:
  - `parses_the_recorded_fixture`: M8b.1's body gives points for `r-fix`/`orchestrator`, with the recorded values (skipped with a printed reason if M8b.1 could not record it; then `parses_the_documented_shape` on a hand-written body with `"observed": false` in its meta covers the parser);
  - `delta_points_add`;
  - `cumulative_series_add_only_their_increase`;
  - `a_cumulative_reset_restarts_the_series`;
  - `points_without_anthrex_run_are_ignored`;
  - `other_metrics_are_ignored`;
  - `malformed_json_is_an_error`;
  - `orchestrator_env_is_exact`.
- In `otlp_server.rs` (real TCP on `127.0.0.1:0`):
  - `post_with_content_length_is_accepted_and_totals_reach_the_sink`;
  - `chunked_post_is_accepted`;
  - `wrong_path_is_404_and_protobuf_is_415`;
  - `an_oversized_body_is_refused`;
  - `a_slow_client_does_not_block_another`: one connection sends half a header and stalls, and a second full request is answered within 2 s;
  - `the_address_file_is_written_and_removed_at_shutdown`.
- In the snapshot tests: `run_usage_sums_roles` (worker and reviewer rounds from M8a's usage, decider, triage, scout and orchestrator into `by_role` and `total`).

**Change.** Decisions 29 and 30.

**Acceptance.** Tests pass. `metering/otlp.rs` is pure. No `hyper` or other HTTP crate is added (`cargo tree -p daemon` shows none).

**Commit.** `feat(daemon): meter deciders, scouts and the orchestrator's OTLP export into run usage by role`

### M8b.16 History I: phases, diff measurement and `history.jsonl`

**Files.** Create `run/phases.rs`, `run/history.rs`, `run/history_io.rs`, `run/history_tests.rs`, `crates/daemon/tests/history_io.rs`. Modify every file that assigns a task's state (decision 31's list, through `set_state`), `run/engine/{merge,complete,requests}.rs` (emit `MeasureDiff` and `AppendHistory`), `run/engine/ops.rs`, `run/driver/adapt.rs` (execute both; fill `accepted_commit`), `run/reconcile/{mod,git}.rs`.

**Tests first:**
- In `history_tests.rs`:
  - `set_state_accumulates_phase_times`: queued 10 s, preparing 5 s, working 100 s, check 20 s, review 30 s and merge 3 s from injected times; `Pending` counts nowhere;
  - `max_rung_is_tracked`;
  - `task_record_from_a_merged_task`: every field from a fixture task, with two review rounds (one with an `important` finding and one `minor`), one check failure, one generated-file bounce, and `done_signal == TurnEndFallback`;
  - `unfinished_tasks_get_records_when_the_run_ends`;
  - `run_record_fields`;
  - `a_run_restored_from_m8a_writes_no_history` (`repo_dir` empty).
- In `crates/daemon/tests/history_io.rs`, on a `TempRepo`:
  - `measure_diff_counts_files_hunks_and_lines` (two files, three hunks, a binary file);
  - `measure_diff_of_a_merge_commit_is_the_tasks_contribution_after_a_hand_back`;
  - `append_is_one_line_and_fsynced` (the code is checked in review; the test checks content);
  - `read_history_keeps_the_last_line_per_record_id_and_skips_a_torn_line`;
  - `history_append_is_reconciled_exactly_once`: a `Replay` when the line is present, `NotStarted` when absent, and after the replayed path exactly one line.
- In the engine tests:
  - `merged_task_measures_then_appends_once`;
  - `cancel_without_a_recorded_head_appends_without_a_diff`;
  - `accept_appends_the_run_record_after_the_tasks`.

**Change.** Decisions 31–33.

**Acceptance.** Tests pass. This prints only `crates/daemon/src/run/phases.rs` lines:

```bash
rg -n '\.state = ' crates/daemon/src/run --glob '!**/tests/**' --glob '!*_tests*.rs' --glob '!**/driver/tests.rs' | rg -v '\brun\.state = '
```

**Commit.** `feat(daemon): record every finished task to history.jsonl with phases, diff size and gate tallies`

### M8b.17 History II: reverts and `anthrex run stats`

**Files.** Create `run/stats.rs`, `run/stats_tests.rs`. Modify `run/history_io.rs` (`detect_reverts`), `run/driver/adapt.rs` (`Stats`; revert detection at both starts), `run/driver/requests.rs` (the arm), `crates/cli/src/run_cmd/adapt.rs`, `crates/daemon/tests/history_io.rs`.

**Tests first:**
- In `stats_tests.rs`:
  - `stats_medians_by_class_over_merged_tasks` (the lower middle value for an even count; `-` without merged tasks; the exact `render` output);
  - `reverted_counts_join_task_and_run_reverts`;
  - `a_skipped_line_is_reported_once`.
- In `history_io.rs`: `detect_reverts_matches_merge_and_accept_commits` (`git revert --no-edit <task merge>` and `git revert -m 1 --no-edit <accept merge>` give two revert records, the second with `task_id == None`; running again adds none; a commit older than 90 days is not considered).

**Change.** Decisions 34 and 35.

**Acceptance.** Tests pass.

**Commit.** `feat: detect reverts after accept and summarise run history with anthrex run stats`

### M8b.18 End-to-end scenarios I: the fast path, deciders, the output filter and metering

**Files.** Create `crates/cli/tests/run_e2e_adapt.rs`. Complete `crates/cli/tests/support/run_adapt.rs`.

**Tests first.** All use `RunHarness`. A stored profile is written with `stored_profile`: `check = "sh check.sh"`, `single_test = "sh tests/{test}.sh"`, `test_passed = "PASS {test}"`, `sample_test = "t_ok"`, `filter_prefixes = ["sh tests/"]`, `generated = ["Cargo.lock"]`, and the default `protected`.

- `e2e_green_s_task_on_the_fast_path`. `with_deciders(claude)`; `triage-1.json` answers `single` with one S `check`-mode task owning `a.txt`. The worker commits `a.txt` and calls `task_done`; the reviewer approves. Assert:
  - `run start --goal` prints the id and `fast path: one task, no plan gate`;
  - no snapshot ever shows `awaiting_approval`;
  - `path == Some(Fast)`, `approved_by == "fast path"`, and the run is complete;
  - `triage.source == Decider`;
  - `run accept --yes` succeeds.
- `e2e_goal_needing_a_plan_is_refused_without_side_effects`. Triage answers `plan`: exit 1 with `refused_message`, no `refs/heads/anthrex/*`, no `<data>/runs/*`, and no window.
- `e2e_goal_without_deciders_takes_the_plan_path` (mode `off`: the reason names `deciders are off`).
- `e2e_goal_without_a_profile_starts_detection` (no stored profile, `onboarding.auto`: the exact refusal, and `profile status` shows detection).
- `e2e_goal_runs_m8a_start_checks` (`ANTHREX_TEST_NO_SETTING_SOURCES=1` and a tracked `.claude/settings.json` with hooks: the fast path is refused with M8a's settings text, and `--trust-project` starts it).
- `e2e_promote_records_intent_and_the_task_continues`.
- `e2e_check_bounce_carries_the_decider_summary`. The worker's first commit breaks `check.sh`, and `check_summary-1.json` answers two lines. The worker's `read_message` expects the first summary line and does not see line 150 of the raw output. The task merges with `summary_source == Decider`.
- `e2e_check_bounce_falls_back_to_the_tail_when_the_decider_fails` (no `check_summary` script: the message has M8a's 40-line tail, `summary_source == Fallback`, and the run completes).
- `e2e_unclassified_block_is_classified_as_environment`.
- `e2e_size_cross_check_raises_a_task_and_reports_it`. `onboarding_report(..)` is written, and the plan has one S task and one without evidence. `size_check-1.json` answers M for the first. Assert its size is M, `size_check.agreed == false`, `REPORT.md` has the note, and the second task is skipped with the note.
- `e2e_filter_hook_shrinks_test_output_for_a_claude_worker`. The worker script runs `bash {"cmd":"sh tests/noisy.sh"}`, where `noisy.sh` prints 5000 lines with one `FAILED` and exits 1. `FAKE_AGENT_BASH_LOG` shows the command wrapped, at most 32 output lines, the `FAILED` line, and exit 1. The log file has 5000 lines and lies under the task's `TMPDIR` (`<tmp root>/…/anthrex-logs/`), not in the checkout, and the task's `task_done` is accepted (the checkout is clean).
- `e2e_otlp_usage_reaches_the_run_snapshot`. Read `<data>/otlp.addr`, POST M8b.1's fixture (or the documented body) with `anthrex.run` rewritten to the test's run id, once with `Content-Length` and once chunked. Within a deadline, `usage.by_role["orchestrator"]` equals the fixture's totals.

**Change.** Only tests and the fixes they uncover.

**Acceptance.** Tests pass.

**Commit.** `test: cover the fast path, deciders, the output filter and metering end to end`

### M8b.19 End-to-end scenarios II: the profile, history, and smoke stage 11d

**Files.** Create `crates/cli/tests/run_e2e_adapt_profile.rs`, `scripts/pty_smoke_adapt.py`. Modify `scripts/pty-smoke.py` (at most 6 lines: the import, `ENV["ANTHREX_DECIDER_BIN"] = FAKE_AGENT_BIN`, `ENV["FAKE_AGENT_DECIDER_DIR"]`, and the call).

**Tests first.** All use `RunHarness`, with the stored profile of M8b.18 unless the test says otherwise.

- `e2e_stored_profile_wins_over_the_plan_profile` (a plan `[profile] check = "false"` is ignored, the log line is present, and the run completes).
- `e2e_a_corrupt_stored_profile_refuses_the_run` (the exact message with the path; no branch).
- `e2e_a_stored_profile_cannot_widen_confinement`: a hand-written `profile.toml` adds `cache_dirs = ["/"]`; `run start` is refused with the parse message naming `cache_dirs`.
- `e2e_generated_lock_file_bounces_at_rung_1` (the worker changes `Cargo.lock` outside `owns`; M8a decision 55's rung-1 bounce, not rung 3; the task merges after reverting it).
- `e2e_protected_file_from_the_stored_profile_bounces_at_rung_1`. The stored profile's `protected = ["docs/agents.txt"]`; the worker, whose `owns` is `**`, edits `docs/agents.txt`. M8a's protected-path rule fires from the **stored** list, as a rung-1 bounce naming it. After the worker reverts the file, the task merges.
- `e2e_history_records_the_fast_path_run`: after `e2e_green_s_task_on_the_fast_path`'s flow and accept, `history.jsonl` has one task record with `path == fast` and `diff.files == 1`, and one run record `accepted` with `accepted_commit` equal to `main`'s head.
- `e2e_history_survives_a_crash_after_the_append_intent` (`ANTHREX_TEST_ABORT_AFTER_INTENT=AppendHistory`: restart with `unset_env`, `run resume` if paused, complete; `read_history` gives exactly one task record per task, and the raw file has at most one line per `record_id` plus nothing torn).
- `e2e_revert_after_accept_is_recorded_and_counted` (accept, `git revert -m 1` on `main`, `run stats --json`: `reverted == 1` in the S row).

In `scripts/pty_smoke_adapt.py`, the function `adapt_stage(run_cmd, fail)` follows `pty_smoke_run.py`'s injection pattern (it uses the caller's `run_cmd`, `fail` and isolated `ENV`, starts no daemon of its own, and has no `__main__`). `pty-smoke.py` calls it right after `run_engine_stage(run_cmd, fail)` (stage 11c), and before M8c's stage 11e when that has landed first. It prints `== stage 11d: a goal takes the fast path ==`. The run milestones' stage letters are fixed: M8a `11c`, M8b `11d`, M8c `11e`, M9 `11f`, M9.5 `11g`, called in that order whichever merges first. `ANTHREX_DECIDER_BIN` and `FAKE_AGENT_DECIDER_DIR` are set in the script's `ENV` at the top, beside `ANTHREX_CLAUDE_BIN`, so the smoke daemon inherits them from its start (they change nothing for earlier stages: stage 11c's check passes, so no decider runs; a decider with no script falls back). The stage:
1. creates `/tmp/anthrex-smoke-adapt-<pid>`, a repo with identity, `check.sh` and one commit;
2. runs `anthrex profile status --json --dir <repo>` to learn `repo_dir`, and writes the stored profile and an empty-fingerprint `profile.meta.json` there;
3. writes `triage-1.json` under `FAKE_AGENT_DECIDER_DIR`, and the worker and reviewer scripts of `e2e_green_s_task_on_the_fast_path` under `<repo>/.git/fake-agent/`;
4. runs `anthrex run start --goal "add a" --dir <repo>` with a timeout derived from `GOAL_REQUEST_TIMEOUT` as `pty_smoke_run.py` derives `RUN_CMD_TIMEOUT` (`ensure_daemon` 3.25 s + `HANDSHAKE_TIMEOUT` 5 s + 810 s, rounded up to 900 s), and checks stderr says `fast path`;
5. polls `anthrex run status <id> --json` every 0.5 s up to `RUN_WAIT` (300 s) until `complete`;
6. runs `anthrex run accept <id> --yes` with `ACCEPT_CMD_TIMEOUT`;
7. asserts `a.txt` exists on `main`;
8. removes the paths (the repo, the decider directory's files) in `finally`.

**Change.** Only tests, scripts and the fixes they uncover.

**Acceptance.** All five AGENTS.md commands pass.

**Commit.** `test: cover the profile and history end to end, and add smoke stage 11d for the fast path`

## Verification

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

Milestone-specific checks:

- `PROTO_VERSION == 8`, derived under "Implementation notes".
- This prints nothing:

  ```bash
  rg -n "std::fs|std::process|std::thread|tokio|SystemTime" \
    crates/daemon/src/profile/{resolve,proposal}.rs \
    crates/daemon/src/scout/{contract,spec,report,machine}.rs \
    crates/daemon/src/decider/{mod,schema,prompt,parse,fallback,argv}.rs \
    crates/daemon/src/output_filter.rs crates/daemon/src/metering/otlp.rs \
    crates/daemon/src/run/{triage,phases,history,stats,model_adapt}.rs crates/daemon/src/run/engine/deciders.rs
  ```

- M8b.16's `rg` prints only `run/phases.rs`.
- `rg -n "\.anthrex" crates/ scripts/` finds no path written into a repository (M8a's `checkout::default_repo_dir` is a test-only default; the daemon always names a repository in its data directory).
- `rg -n "\bcache_dirs\b|\bconfined_(network|unix_sockets|localhost_ports)\b" crates/proto/src/profile.rs crates/mcp/src/tools_scout.rs crates/daemon/src/profile/proposal.rs` finds only comments: no model-writable surface names a confinement setting.
- `git diff --stat baa04e1 -- crates/config/src/lib.rs` shows one changed line.
- `wc -l` on every file in the file-size table and every new file: nothing new above 600, and nothing existing grown past its budget.
- Every git call added by this milestone goes through `worktree::run_git` or `run::git`'s helpers (`--no-optional-locks`, scrubbed environment; AGENTS.md rules 10–11), every write through `GitQueue::write`, and none runs under `daemon::lock`.
- No process-kill code is added or changed outside `decider::call`'s use of `HeadlessHandle::kill` and `ScoutService`'s use of `headless_kill`, both on handles the daemon itself spawned.
- After the tests and the smoke script, nothing of yours shows in `pgrep -fl "anthrex daemon"` or `pgrep -fl fake-agent`, and `/tmp/ax-*`, `/tmp/anthrex-smoke-adapt-*` are gone.

## Manual check

This uses real Claude, on your own login, with an isolated daemon, config and repository throughout:

```bash
export ANTHREX_SOCKET=/tmp/anthrex-m8b/daemon.sock ANTHREX_DATA_DIR=/tmp/anthrex-m8b/data ANTHREX_CONFIG=/tmp/anthrex-m8b/config.toml
mkdir -p /tmp/anthrex-m8b && cd /tmp/anthrex-m8b && git init -b main demo && cd demo \
  && printf 'check:\n\tsh tests/run.sh all\ntest:\n\tsh tests/run.sh $(T)\n' > Makefile \
  && mkdir tests && printf '#!/bin/sh\n[ "$1" = all ] && { grep -q hello hello.sh && echo "PASS greets"; exit 0; }\ngrep -q hello hello.sh && echo "PASS $1"\n' > tests/run.sh \
  && printf 'echo hi\n' > hello.sh && git add . && git commit -m init
printf '[orchestrator.deciders]\nmode = "claude"\n' > /tmp/anthrex-m8b/config.toml
```

1. Run `anthrex profile detect --dir /tmp/anthrex-m8b/demo`, then `anthrex profile status` until `Ready`. Check:
   - `anthrex profile show --proposed` shows `check = "make check"` or equivalent, a `single_test` with `{test}`, a `test_passed` regex, and a verification block marked `confined` with `ok` for each command;
   - `ls -a demo` shows no `.anthrex`, `git -C demo status --porcelain --ignored` is empty, and `git -C demo worktree list` lists only `demo`;
   - the scout's conversation (attach with `anthrex`, open the `scout/onboarding-…` window's conversation) shows no write attempt succeeding.
2. Run `anthrex profile confirm` and answer `y`. `/tmp/anthrex-m8b/data/repos/demo-*/profile.toml` exists.
3. Run `anthrex run start --goal "Make hello.sh print hello" --dir /tmp/anthrex-m8b/demo`. Check:
   - the triage line says `code/single (decider)` and `fast path`;
   - opening the worker's conversation (`C-b m`) shows, for its test run, the filtered view ending in `[anthrex] full output: …`, a path under `/private/tmp/ax-<uid>/`;
   - the run completes. Then run `anthrex run accept <id> --yes`.
4. Run `anthrex run start --goal "Rewrite this project as a Rust workspace with a CLI crate, a library crate and a test suite" --dir …`. It exits 1 with the plan-path message, and `git -C demo branch --list 'anthrex/*'` is empty.
5. `anthrex run stats --dir …` shows one S task, merged.
6. Edit `Makefile` and commit. `anthrex profile status` shows `stale: Makefile`, and detection starts again. Run `anthrex profile reject`.
7. Run `anthrex profile edit check 'sh tests/run.sh all' --yes`. Status goes to `Verifying`, then the value is stored.
8. Run `anthrex profile edit setup 'curl -s https://example.com' --yes`. The proposal ends `Ready` with `setup` dropped and the confinement hint naming `[orchestrator.confined_network]`; nothing is stored. Run `anthrex profile reject`.
9. `curl -s -X POST -H 'Content-Type: application/json' --data @<M8b.1 fixture, anthrex.run replaced by a run id> "$(cat /tmp/anthrex-m8b/data/otlp.addr)/v1/metrics"`, then `anthrex run status <id> --json` shows `usage.by_role.orchestrator`.
10. Run `anthrex daemon stop`. `pgrep -fl "anthrex daemon"` shows nothing of yours. Remove `/tmp/anthrex-m8b`.

## Review focus

These are the input classes or failure modes most likely to bite a user that the task tests above would not exercise without being told to. Each has a test in its owning task; the reviewer checks those tests exist and fail without the fix.

1. **A decider that hangs, crashes, or answers garbage, nothing or megabytes.** A decider is a second model surface on every bounce. Without a hard bound it stalls runs; without process-group kills it leaks processes. Tests: `a_hanging_decider_falls_back_at_its_timeout_and_leaves_no_process` and `garbage_empty_and_oversized_answers_fall_back` (M8b.7).
2. **Test commands with shell syntax.** Quotes, heredocs, `&&`/`||`, env prefixes, `cd` prefixes, non-ASCII and non-zero exits all pass through the filter hook. A wrapper that re-quotes wrongly silently runs a different command, and one that loses the exit status makes a failing test look green to the worker. Tests: `wrapped_commands_behave_exactly_like_the_original` and `filter_run_preserves_exit_status_and_signals` (M8b.8).
3. **Verification commands with side effects.** A `check` that rewrites tracked files, creates untracked ones, writes outside its checkout, or hangs. Verification must never touch the user's checkout, must stay inside the user's confinement, must never delete dirt unsalvaged, and must end. Tests: `verification_never_touches_the_checkout_and_salvages_dirt`, `verification_uses_the_users_confinement_not_the_proposal` and `a_hanging_verification_command_times_out_and_is_dropped` (M8b.10).
4. **A hand-edited or model-written profile.** `profile.toml` is a plain file a user may edit, and the scout writes a proposal. A parse error must refuse the run by path and never fall back to a plan's weaker profile; a confinement key or a reserved env key must never reach a run; and `profile edit` must refuse a glob with `..`, a `single_test` without `{test}` and a regex that does not compile. Tests: `an_unparseable_profile_is_reported_with_its_path`, `a_profile_with_a_confinement_key_does_not_parse`, `from_findings_drops_reserved_env_keys` (M8b.4), `e2e_a_corrupt_stored_profile_refuses_the_run`, `e2e_a_stored_profile_cannot_widen_confinement` (M8b.19), and `edit_rejects_invalid_values_with_the_rule` (M8b.10).
5. **A crash around a history append.** Between an append and its `done` line, a replayed op must not double-count a task in `stats`, and a torn last line must not poison the file. Tests: `history_append_is_reconciled_exactly_once` (M8b.16) and `e2e_history_survives_a_crash_after_the_append_intent` (M8b.19).
6. **A scout that is not read-only.** Every scout's `HeadlessSpec` must carry the empty-root sandbox and `dontAsk` with the write tools disallowed, whatever `worker_sandbox` says. Tests: `area_scout_spec_is_read_only`, `the_scout_sandbox_does_not_depend_on_worker_sandbox` (M8b.9) and `the_scout_argv_is_read_only` (M8b.9).

## Risks and gotchas

1. **Deciders cost tokens on every bounce.** A check summary per failed check, a size check per plan edit, and a triage per goal. They run at the fast tier and low effort, one call each, and `mode = "off"` removes them. Meter them (decision 29) before judging their worth.
2. **Strict schemas.** Structured-output modes may reject a schema with optional properties; decision 17's schemas already make every property required and nullable. If M8b.1 finds a CLI rejects `anyOf` or `["string","null"]`, simplify that runtime's schema and let `parse` enforce the rest.
3. **Codex prompts are on the argv.** `ps` shows them, and the size is bounded by `ARG_MAX` (1 MiB on macOS), which `PROMPT_MAX_BYTES` (128 KiB) stays well inside. Claude prompts go on stdin.
4. **Confined verification fails where the network or a cache is needed.** A `setup` like `cargo fetch` or `npm ci` fails confined unless the user's config enables `confined_network` (and a cache directory) for the repository. That is deliberate: it fails the same way in every run, so a profile that verifies here works in runs. The dropped command's hint names the tables. Enabling `confined_network` is close to trusting the repository's checks with the user's readable credentials (M8a F1d round 2, S1); the hint must not encourage it beyond naming it.
5. **The onboarding scout cannot run a build.** It is read-only, so its proposal is a reading of manifests and CI files. The engine's run is the proof; a wrong proposal is dropped, not stored.
6. **Fingerprint false positives.** Any change to `Cargo.toml` makes the profile stale, even a version bump. That is cheap: the stale profile stays in use, and detection only proposes.
7. **The OTLP port is unauthenticated on loopback.** An unconfined local process can post usage for a run id; confined checks and sandboxed sessions cannot (decision 30). Metering is informational and never gates anything.
8. **Wrapped commands and allow rules.** If `HOOK_SETS_ALLOW` must be true, the hook approves the rewritten `Bash` call. It rewrites only calls that start with a profile prefix, which the worker's `--allowedTools Bash` already allows, and the sandbox still confines them.
9. **The history file grows forever.** At a few KiB per task this is harmless for years; pruning is a follow-up. Filter logs live in the task `TMPDIR` and go with the checkout.
10. **Revert detection sees only the base branch's first 2000 commits** and only `git revert`'s standard message. A manual revert goes unseen, and the stats undercount reverts.
11. **Stale M8a tests.** The harness defaults deciders to `off` and onboarding to manual (decision 36). If an M8a test starts failing with decider calls in its log, a harness default was lost.
12. **Built-in `protected` entries cannot be switched off from M8b.** The stored list holds extras only, `profile edit` refuses built-ins, and M8a always unions `BUILTIN_PROTECTED`. A user who truly needs a task to change `AGENTS.md` names it exactly in that task's `owns` (M8a decision 56).
13. **Socket and temp paths.** Every new test directory is under `/tmp` (`tempfile::Builder::new().prefix("ax-…").tempdir_in("/tmp")`), never `std::env::temp_dir()`.
14. **Leftover scouts after a daemon crash** are not killed (decision 11). They are read-only and end with their turn. Do not add a pattern-based kill to "clean them up".
15. **Environment-changing tests** run alone in their own test binary (M8a's practice, ruling Q11), not under a shared lock.

## Follow-ups handled

M8a's "Out" table assigned these to M8b; each is handled here:

- Deciders and `ANTHREX_DECIDER_BIN`, triage, the fast path, the size cross-check, the decider check summary, and classification of free-text `task_blocked` reasons: decisions 16–25.
- The repo profile file, the onboarding scout, and profile re-proposal: decisions 4–11. There is no `.anthrex/profile.toml`, per the amended spec §6.
- The output-filter `PreToolUse` hook, OTLP metering of the orchestrator, `history.jsonl` and `anthrex run stats`: decisions 26–35.

From M8a's follow-up ledger (`docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`, the "for M8a/M8b" sections):

- **F1c N4, "Scouts … must be launched the same way, a read-only OS sandbox":** decision 12; every scout gets the reviewer's empty-root sandbox and `dontAsk`, and Codex `read-only`.
- **F1c I2 cost, "confined `setup` needs `cache_dirs`":** verification runs confined like a run (decision 9), so a profile never verifies what a run cannot run, and a dropped command names the user-config tables.
- **F1c N3, F1d R4 and round 2 S1 (confinement settings are the user's own):** the profile has no such key, the scout's schema and `profile edit` cannot set one, and verification reads them only from the user's config (decisions 5 and 9).
- **F2 C-I4 and round 2 (the reserved environment list):** the stored profile's `env` is checked against `config::reserved_env` at proposal, at edit, and again by `build_run`.
- **M8a.10 review, "`test_passed` need not contain `{test}`":** required for stored profiles (decision 9). Plan-file validation is unchanged; the follow-up stays open for plan files.
- **F2, Codex trust entries in `~/.codex/config.toml`:** scouts and deciders run Codex `read-only`, which M8b.1 item 2 checks writes no trust entry.

Not handled here, and left open: F4's `crates/config/src/lib.rs` split (M8b adds no line to it), F3 B-10's late `TurnEnded` usage (engine signal code, not metering), and every process-kill item.

New follow-ups to record in the ledger during implementation:
- a TUI form for the profile (M9);
- the output filter for Codex workers (under M9.5);
- pruning `history.jsonl`;
- routing and threshold proposals in `run stats` (M9.5);
- a leftover scout process after a daemon crash is not killed (decision 11), to weigh under the process-kill safety rule;
- `run::git::checkout::default_repo_dir`'s `<parent>/.anthrex/<name>` default, used only by tests' convenience wrappers, should not be reachable from daemon code (a guard or a rename).

## Spec defects found while writing this brief

1. **§6 against §4, read-only scouts.** §6 says the onboarding scout "must run `check` and `single_test` once successfully before proposing them" and that it "may run commands … sandboxed, with shell access". §4's table gives scouts `Writes? no`, and M8a's follow-up requires a read-only OS sandbox for every scout. Decision 9 resolves it: the onboarding scout reads (its shell writes nothing), and the engine runs every proposed command, confined, in a fresh copy; only commands that pass are proposed. *(Refreshed 2026-09-26; the 2026-09-22 resolution let the scout write its copy.)*
2. **§4, "Claude with `--permission-mode plan`" for read-only roles.** M8a.1 found plan mode blocks an allowed MCP call under `-p`; M8a launches reviewers with `dontAsk` and the write tools disallowed, plus a read-only sandbox. Decisions 12 and 16 do the same for scouts and deciders.
3. **§21 against §14.8, OTLP in M8b.** §21 gives "OTLP metering" to M8b, but §14.8 meters only "the orchestrator, the one PTY session", which milestone 9 creates. Decision 30 builds the receiver and the environment, tested by direct POSTs; wiring it into a window is M9's.
4. **§7.2 rule 5 against §5.1.** Rule 5 cross-checks "each task's size against its scout evidence", but the fast path has no scouts (§5.1: "a scout for a one-line fix is pure overhead"), and its single task is sized by the triage decider. Decision 19 skips the cross-check there.
5. **§13 item 3 against §5.1.** Item 3 puts deciders in `max_readers` slots, which are per run, but triage runs before any run exists. Decision 18 exempts triage.
6. **§4, the decider row.** "Deciders are one-shot headless calls … `claude -p --output-format json`" predates the headless amendment. Every other headless surface uses stream-json, and decision 16 uses stream-json on M8a's spawner so one parser serves all.
7. **§15, "whether the user later reverted it".** A record appended when the task finishes cannot know this. Decision 33 adds an append-only `revert` record, joined by readers.
8. **§6, "manifests" and "test output".** §6 re-proposes "when … a manifest … changes hash" but defines no manifest list. §14.3 filters "test output", but no profile key says which commands produce it. Decision 5 adds `manifests` and `filter_prefixes`.
9. **§14.8 against M8a decision 40.** §14.8 says usage field names "are verified in the first task of the milestone that implements metering". M8a meters stream usage and pins them in M8a.1; M8b.1 only cross-references them.
10. **§6's example `protected` list and `env`.** The example lists the five built-ins in `protected` (M8b stores extras only, decision 5), and §17's "per-worktree variables from the profile" is narrowed by M8a's reserved list (F2 C-I4).

## Implementation notes

### Refresh 2026-09-26 (post-M8a)

Refreshed against branch `m8a-engine-core` at `baa04e1` (PR #17, merging unchanged). Paths and line numbers are on that commit. Every changed decision keeps its number; the reason for each change is M8a's shipped code or its recorded deviations.

**Status and protocol**

- Status `blocked` → `ready`, and `docs/ROADMAP.md` row 8b → `ready`: M8a is `done` (ROADMAP row 8a).
- Protocol "one above main when M8b starts (8 if M8a merged at 7)" → **8**: `PROTO_VERSION = 7` at `crates/proto/src/lib.rs:25`; test `proto_version_is_seven` (`lib.rs:69`) becomes `proto_version_is_eight`. Re-derived in the header and M8b.2.

**Decisions changed, and why**

- R1 (decision 3). The config file is `crates/config/src/orchestrator/adapt.rs`, not `orchestrator_adapt.rs`: M8a split `orchestrator` into a directory (`orchestrator/{profile,roster,unknown}.rs`), and `orchestrator.rs` is 570 lines. `lib.rs` (605) changes only in its `pub use` line, since `ScoutContext` names `config::Scouts`. `orchestrator/unknown.rs` owns the known-key list.
- R2 (decisions 4 and 8). The disposable onboarding and verification copies are M8a's **standalone checkouts** (`prepare_scratch_in`, `remove_checkout`, `salvage`; F1c 3a), not `git worktree add --detach` + `git worktree lock`. Reason: since F1c no task-like checkout is a linked worktree, so the user's `git gc`/`fsck`/`log --all` never walk it, and M8a's removal and salvage work on exactly this shape. Their repositories live at `<repo_dir>/tasks/.onboarding` and `.profile-verify` (`checkout_repo_dir`).
- R3 (decision 5). `RepoProfile` holds no `cache_dirs` or `confined_*` key, and its `env` obeys `config::reserved_env`. Reason: M8a F1c N3, F1d R4 and round 2 S1 make those settings the user's own config only, keyed by repository root, precisely so that nothing a model or a repository writes can widen a confined command; F2 C-I4 made the reserved-env list the single rule for profile env. With `deny_unknown_fields`, a profile naming one does not parse, so the run is refused (decision 6).
- R4 (decision 6). "Replace `plan.profile` with the chosen spec" was not enough: `resolve_profile` (`run/plan.rs:119`) falls back to `[orchestrator.profile]` per key, so the config would fill a stored profile's deliberate gaps. The driver now also clears the cloned config's `profile` for source 1 (`adapt::choose_profile`), and the protected-files scan in `build` uses the chosen profile. Confinement still comes from the config tables through `build_run`'s `for_repo`.
- R5 (decision 9). The old decision had the onboarding scout run the commands in a writable sandboxed copy, and the engine re-run them "unsandboxed, as M8a runs `setup`". Both premises changed: M8a F1c round 2 runs `setup` confined like checks and proofs, and F1c N4 requires every scout to run under a read-only OS sandbox. Now the scout only reads (its `*_ran_ok` claims are removed, so `ProfileFindings` is gone and the report carries a plain `RepoProfile`), and the engine verifies each command through `run_matching` under a `ConfineSpec` built from the user's config for the repository root — the confinement a run would get — with M8a's `start_refusal` where the platform cannot confine. `single_test` verification uses M8a's `proof_command`/`proof_pattern`. A dropped command carries a hint naming the user-config tables (picks up F1c I2's cost). `run::exec::run_matching` becomes `pub(crate)`; `run::plan::for_repo` becomes `pub(crate)`.
- R6 (decision 11). The old decision said M8a's reconcile kills a leftover scout by session id. It does not: `kill_leftovers` examines only `run.json` round pids (F3 B-6), and `remove_stale_windows` skips windows with no run (`driver/restore.rs:298`). `ProfileService::restore` now removes restored scout windows itself, and M8b adds no process-kill code (the process-kill safety rule); the leftover is read-only and ends with its turn. Recorded as a follow-up.
- R7 (decision 12). Scouts launch with `dontAsk` and the reviewer's disallowed tools, not `--permission-mode plan` (M8a.1: plan mode blocks an allowed MCP call under `-p`; `REVIEWER_PERMISSION_MODE`, `role_launch.rs:34`), and **every** scout gets the empty-root `ClaudeSandbox` with the protected denials, as a Claude reviewer does (F1c N4, `role_launch.rs` `reviewer_spec`), whatever `worker_sandbox` says; `ProfileStatus.sandbox` ("onboarding scout sandbox: off") is gone, replaced by `verify_confined`. Codex scouts get `read-only`, the F1d pins through `codex_args`, and `codex_config_guard` when Codex loads project config (F2 C-I1). The project-settings check takes M8a's real signature (`project_settings(git, root, base_sha, claude, codex_paths, timeout)`).
- R8 (decision 15). `anthrex mcp`'s flags live in `crates/cli/src/mcp_cmd.rs`, not `main.rs`; `mcp_args` omits `--run` for a run-less scout.
- R9 (decision 16). Deciders use `--permission-mode dontAsk` (not `plan`, R7's reason), take no `--settings` (no window, so no hooks), and never use `--bare` or `apiKeyHelper`: `auth = "api_key"` is refused at config load since F2 C-I3 (`orchestrator.rs:544`). `HeadlessHandle::spawn` is blocking and takes `remove` (F2's credential scrub), so the call runs it on `spawn_blocking` with `credential_scrub_for(runtime, Login)`. `DeciderContext` takes the manager's `CliCaps` value (test overrides included) instead of `&'static CliCaps`, and no `claude_auth`/`api_key_helper`. Codex deciders add `CODEX_SANDBOX_PINS`. `claude_stream` is the stateful `ClaudeStream::parse_line`, not a free function.
- R10 (decision 22). `StartGoal` gains `unconfined_checks`, and the fast path goes through the same `build_plan` M8a's `run start` uses (split out of `driver/requests.rs::build`), so it runs every M8a start check: confinement refusal, protected files, the runtime checks of decisions 50 and 53 for every reachable runtime (M8a.22 fix round 2 made them per runtime, not only Claude), the Codex branch and `.codex` tree.
- R11 (decision 28). The filter log goes to `<task TMPDIR>/anthrex-logs`, not a new writable root `<data_dir>/runs/<run>/logs/<task>`. Reason: M8a F1c I1 computes every grant from engine-made paths, and F1d gives each worker a short per-task `TMPDIR` (`role_launch::task_tmp_dir`) that is already in its grant and outside the checkout; adding a root would widen the worker's sandbox for no need. The log is removed with the checkout, so the "prune logs" follow-up is dropped. M8b.1 item 3 probes the write under `$TMPDIR` instead of a separate root.
- R12 (decision 32). `MeasureDiff` names only commits in the user's store (merge commits, the run head, `Task.head`, which M8a sets from imported claims); a worker's unimported commits live in its private store (F1b/F1c) and are never named. "Cancelled with commits" became "with a recorded head".
- R13 (decision 36). `fake-agent` detects decider mode from the prompt's first line rather than from `--json-schema`/`--output-schema`, so it works whichever flags M8b.1 finds. The harness helpers go in a new `support/run_adapt.rs`, because `run_harness.rs` is 525 lines.
- R14 (decision 31). The `set_state` sweep is sized from the code: 21 task-state assignments in 8 files (`= next` and `= state` included, which the old grep `\.state = TaskState::` missed); the acceptance grep now catches all of them.

**Stale names fixed** (old → shipped)

- `RunContext { …, exe, socket_path, … }` → no such fields (F3 B-11); `exe`/`socket_path` come from `ManagerConfig` through the manager.
- `RunService::snapshots` → `pushes()`/`current()` (F3 B-11).
- `run::git::{remove_worktree, lock_worktree}` for the onboarding copies → `remove_checkout` with the repository named; no lock.
- `profile/verify.rs::{prepare_scratch, discard_scratch}` (`worktree add --detach HEAD, lock`) → `prepare` (`prepare_scratch_in` at `pre.base_sha`) and `discard` (`salvage` + `remove_checkout`).
- `run::exec::run_shell` for verification → `run_matching` with a `Confinement`.
- `run/driver.rs` or `run/driver/ops.rs` as the home of new ops → `run/driver/adapt.rs` (driver.rs is 598 lines, ops.rs 580, requests.rs 561).
- `server/run_api.rs` routing new requests → `RunService::request` in `driver/requests.rs` (run_api hands every non-subscribe request to it).
- `crates/cli/tests/support/mod.rs`'s `TempRepo` via `#[path]` → `run_harness::init_repo` (CLI tests never included the daemon support module).
- `crates/daemon/src/launch/claude.rs` / `launch/mod.rs::hook_command` as the settings source → `headless::argv::claude_settings(exe, window_id, sandbox, caps)`, which builds on `launch::claude::settings`.
- `ClaudeSandbox { writable_roots }` → `{ writable_roots, deny_write }` (F2 round 2).
- `CheckInfo.summary_source` beside a new `summary` → `CheckInfo.summary: String` already exists (M8a's last-40 text), so the decider's text is the new `CheckInfo.decider_summary`.
- `anthrex run start` flags → also `--unconfined-checks` (F1c round 2).
- `scripts/pty_smoke_adapt.py::adapt_stage(env, bin_path)` → `adapt_stage(run_cmd, fail)`, M8a's injection pattern (`pty_smoke_run.py::run_engine_stage(run_cmd, fail)`, called at `pty-smoke.py:1712`).
- The file-size table: every count taken on `baa04e1`, with budgets that keep every file under 600 (`main.rs` 594 forces the new `pre_clap.rs`; `driver.rs` 598 allows only `mod adapt;` and one field).

**Tasks re-cut.** The old 15 tasks became 19, so each is one implementer's job: onboarding split into proposal/verification (M8b.10) and service/CLI (M8b.11); engine deciders into summaries/classification (M8b.12) and the size cross-check (M8b.13); history into records (M8b.16) and reverts/stats (M8b.17); the end-to-end task into two (M8b.18, M8b.19, which carries smoke stage 11d).

### M8b.1 external facts (2026-09-25)

Verified on macOS (Darwin 25.2, arm64) against `claude` 2.1.280 and `codex-cli` 0.156.1. Every probe used M8a.1's scrubbed environment (`env -i` with only `HOME`, `PATH`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `LANG`), no `ANTHROPIC_API_KEY` (the subscription login), scratch directories under `/tmp/anthrex-m8b1/`, `--no-session-persistence` and `claude-haiku-4-5`. Fixtures, each with a `.meta.json` holding the exact command (paths redacted to `/tmp/fixture`; ids, thinking signatures, account identity and the user's own hook and command payloads redacted):
- `crates/daemon/tests/fixtures/deciders/claude-2.1.280-decider.jsonl` (item 1, `blocked_reason`) and `claude-2.1.280-decider-triage.jsonl` (item 1, `triage`);
- `crates/daemon/tests/fixtures/headless/claude-2.1.280-filter-hook.jsonl` (item 3);
- `crates/daemon/tests/fixtures/headless/claude-2.1.280-scout.jsonl` and `claude-2.1.280-scout-deny-cwd.jsonl` (item 4);
- `crates/daemon/tests/fixtures/otlp/claude-2.1.280-metrics.json` (item 5).

**Controller rulings for this task.** Real `claude` only for items 1, 3, 4 and 5, one short prompt per probe; real `codex` not at all (a Codex exec can write trust entries into `~/.codex/config.toml`), so item 2 comes from `codex exec --help` and `codex --version` only. The sha256 of `~/.claude/settings.json` and `~/.codex/config.toml` was taken before the first probe and after each, and of `~/.claude/settings.local.json` too from the resumed probes on; none changed. The first run also watched `~/.claude.json` and stopped after item 1 when its hash changed; the controller then ruled that `~/.claude.json` is Claude Code's shared state file, rewritten continually by every running Claude Code session (the controller's and the user's own included), so a change to it is not evidence of a probe writing user configuration, and a headless `claude -p` touching it is normal Claude Code behaviour, not an anthrex concern. The probes resumed without it.

**Item 1, a Claude decider call: run** (three calls with decision 16's argv exactly, plus two for the tools question below). One stream-json user message on stdin, stdin then closed.
- **Flags.** All exist and were accepted (exit 0): `--json-schema`, `--no-session-persistence`, `--permission-prompts none`, `--setting-sources user`, `--strict-mcp-config`, `--permission-mode dontAsk`, `--disallowedTools`, `--model`, and `--max-turns`, which `claude --help` does not list (hidden) but the CLI accepts.
- **Where the answer appears: three places at once.**
  1. A top-level assistant `tool_use` named `StructuredOutput` (`parent_tool_use_id: null`) whose `input` is the answer object, followed by a `user` `tool_result` "Structured output provided successfully".
  2. `result.structured_output`, the same object.
  3. `result.result`, the same object as JSON text.

  So `DECIDER_CAPS.answer_source = ResultField`, and M8b.7 adds `SessionEvent::StructuredOutput { value }` from `result.structured_output` (decision 16's source 1). Source 2 (the `StructuredOutput` tool use) also matches both streams and stays as the fallback.
- **The triage call answered in text first.** With the exact triage schema and prompt, the model first wrote the answer as fenced JSON in an assistant text block; the CLI then injected a user text message, `[structured-output-enforce] You MUST call the StructuredOutput tool to complete this request. Call this tool now.`, and the model called `StructuredOutput`. `num_turns` was **3**, exactly `--max-turns 3` (the `blocked_reason` calls used 2). So the margin decision 16 leaves is zero for a model that answers in text first. What the CLI does when it hits `--max-turns` before `StructuredOutput` was not probed; decision 16's source 3 (the last top-level assistant text, fence removed) would still find the fenced answer in the stream, provided M8b.7 reads the events before treating a failed turn as final. Ruling R-T1-4: M8b.7 parses that fenced text even when the turn limit is hit; `--max-turns 3` is unchanged.
- **Strict schemas: not required.** A `blocked_reason` schema whose `required` lists only `kind` was accepted and answered (exit 0, `structured_output` present). The triage schema's `anyOf` with `{"type":"null"}` and its `["string","null"]` types were accepted too, so the risk note "Strict schemas" needs no simplification. `DECIDER_CAPS.strict_schemas = false`.
- **Usage** (`result.usage`, the same names M8a.1 recorded, plus `output_tokens_details.thinking_tokens` and `iterations[]`): for `blocked_reason`, input 10, output 357 (257 thinking), `cache_read_input_tokens` 0, `cache_creation_input_tokens` 17 688, `total_cost_usd` 0.0372. `modelUsage` is keyed by `claude-haiku-4-5`. No new usage field needs adding to M8a.1's list.
- **Exit.** Every call exited 0, 0.37 to 0.80 s after the `result` line, with stdin already closed.
- **What else loads into a decider.** Under `--setting-sources user` the user's own `SessionStart` hooks run (`system/hook_started`/`hook_response`) and `system/commands_changed` lines list the user's skills (the fixtures empty those arrays; see each meta's `redactions`); that context is why a one-line classification cost 17 688 cache-creation tokens. `system/init.tools` lists built-in tools `--disallowedTools` does not name (`CronCreate`, `SendMessage`, `RemoteTrigger`, `PushNotification`, `Workflow`, `Skill`, `ToolSearch`, `Task*` and others) beside `StructuredOutput`. Two cheap extra calls, same argv plus one flag:
  - `--tools ""`: `init.tools` is exactly `["StructuredOutput"]`, the answer arrives as before, and cache creation fell to 10 763 tokens. The user's `SessionStart` hooks still ran.
  - `--restricted`: no `SessionStart` hooks ran (it ignores user settings files), cache creation 9 313 tokens, but `init.tools` still listed 18 tools.

  Ruling R-T1-6: Claude deciders pass `--tools ""` (M8b.5); this task leaves decision 16's argv as it is.

**Item 2, a Codex decider call: not run (controller ruling).** From `codex --version` (`codex-cli 0.156.1`) and `codex exec --help`: `--json`, `--skip-git-repo-check`, `--ephemeral` ("Run without persisting session files to disk"), `-s/--sandbox read-only|workspace-write|danger-full-access`, `-c/--config <key=value>`, `--output-schema <FILE>` ("Path to a JSON Schema file describing the model's final response shape") and `-m/--model` all exist; `--ignore-user-config` also exists (M8a.1). OUTSTANDING for the user: that the final `agent_message` text is the JSON, `turn.completed.usage`, whether `--output-schema` is honoured at run time, and whether the run writes `~/.codex/config.toml`. The command below is copy-pasteable: it makes an empty directory, writes the `blocked_reason` schema and prompt exactly as the Claude probe used them (the schema and prompt in `claude-2.1.280-decider.meta.json`'s `command`), and hashes the config before and after.

```sh
mkdir -p /tmp/anthrex-m8b1-codex && cd /tmp/anthrex-m8b1-codex
cat > schema.json <<'SCHEMA'
{"type":"object","additionalProperties":false,"required":["kind","reason"],"properties":{"kind":{"enum":["question","mis_sized","environment"]},"reason":{"type":"string","minLength":1,"maxLength":300}}}
SCHEMA
cat > prompt.txt <<'PROMPT'
[anthrex decider] blocked_reason v1
A coding agent stopped its task and gave the reason below without saying what kind of block it is. Classify it. question: it needs an answer or a decision about the task. mis_sized: the task is bigger than one task, or needs changes outside the paths it owns. environment: a tool, command, permission, dependency or setup is broken or missing. Answer with one JSON object that matches the schema, and nothing else.

Task t1: Add a retry to the fetch helper
Reason:
cargo test fails before my change: the linker `cc` is not installed in this checkout's environment.
PROMPT
shasum -a 256 ~/.codex/config.toml
env -i HOME="$HOME" PATH="$PATH" USER="$USER" LOGNAME="$LOGNAME" SHELL="$SHELL" TERM="$TERM" LANG="$LANG" \
  codex exec --json --skip-git-repo-check --ephemeral -s read-only \
  -c sandbox_workspace_write.network_access=false -c approval_policy="never" -c model_reasoning_effort="low" \
  --output-schema schema.json -- "$(cat prompt.txt)" > codex-decider.jsonl 2> codex-decider.stderr
shasum -a 256 ~/.codex/config.toml
```

Codex's caps follow the controller's ruling that a flag `codex exec --help` lists is passed: `codex_output_schema = true` and `codex_ephemeral = true`. Their runtime behaviour is **unverified** (outstanding for the user, above); `parse` validates every answer whatever `--output-schema` does, and a refused schema only makes a Codex decider fall back, which never blocks a run. No `codex-<version>-decider.jsonl` fixture exists; M8b.6's shape test skips it with a printed reason.

**Item 3, the filter hook under `-p`: run.** A worker-shaped session: `--permission-mode acceptEdits` (M8a's default worker mode), `--allowedTools Bash`, `--permission-prompts none`, `--setting-sources user --strict-mcp-config`, M8a's sandbox block as `claude_settings` writes it (`enabled`, `allowUnsandboxedCommands false`, `failIfUnavailable true`, `filesystem.allowWrite [repo, task TMPDIR]`, `denyWrite` of the protected paths, every pin), and `TMPDIR` set to the task directory. `--settings` had two `PreToolUse` groups: a recording group in M3's place (matcher `""`, M8a.1's recording command; see the deviations below) and a `Bash` group whose script printed `{"hookSpecificOutput":{"hookEventName":"PreToolUse","updatedInput":{…every tool_input key…,"command":<rewritten>}}}` **without** `permissionDecision`. The prompt asked for `echo original`.
- **Where the hook facts come from.** "Both groups fired" and the payload keys are not in the stream (it has only `SessionStart` hook lines); they come from the probe's recording files. The recording group's file is committed as `claude-2.1.280-filter-hook-hooks.jsonl` (hook payloads, one per line: its `PreToolUse` has the original command, its `PostToolUse` the rewritten one). The rewrite group's own recording (one `PreToolUse` payload, identical to the recording group's before redaction) was not committed.
- **The rewrite hook**, `/tmp/fixture/bin/rewrite-hook.py` (scratch paths as run; `rewrite-cmd` held the rewritten command, and no `allow-flag` file or `M8B1_ALLOW` existed, so no `permissionDecision` was printed):

  ```python
  #!/usr/bin/env python3
  import json, sys, os
  payload = json.load(sys.stdin)
  open("/tmp/anthrex-m8b1/out/filter-hook-payloads.jsonl","a").write(json.dumps(payload)+"\n")
  if payload.get("tool_name") != "Bash": sys.exit(0)
  ti = dict(payload.get("tool_input", {}))
  ti["command"] = open("/tmp/anthrex-m8b1/rewrite-cmd").read().strip() if os.path.exists("/tmp/anthrex-m8b1/rewrite-cmd") else 'echo rewritten > "$TMPDIR/anthrex-logs/probe"'
  out = {"hookSpecificOutput": {"hookEventName": "PreToolUse", "updatedInput": ti}}
  if os.environ.get("M8B1_ALLOW") == "1" or os.path.exists("/tmp/anthrex-m8b1/allow-flag"):
      out["hookSpecificOutput"]["permissionDecision"] = "allow"
  print(json.dumps(out))
  ```

  The recorded run's rewritten command: `echo rewritten > /tmp/fixture/s3/tmp/anthrex-logs/probe; echo "tmpdir=$TMPDIR"`. The first run (no `rewrite-cmd`) used the brief's `echo rewritten > "$TMPDIR/anthrex-logs/probe"`.
- **Both groups fired**, in order, once per Bash call.
- **The rewritten command ran.** The tool result is the rewritten command's output, and the probe file was written. So `updatedInput` applies without `"permissionDecision":"allow"`: **`output_filter::HOOK_SETS_ALLOW = false`**.
- **The assistant `tool_use` keeps the original command.** The stream's `tool_use.input.command` is `echo original`; only the `tool_result` shows that the rewrite ran. Decision 37's `bash` step emits "the `tool_use` (with the final command)", which differs from the real CLI: Ruling R-T1-3: M8b.6's `bash` step emits the original command in `tool_use`.
- **`$TMPDIR` inside the Bash tool is not the process's `TMPDIR`.** The brief's command, `echo rewritten > "$TMPDIR/anthrex-logs/probe"`, failed with `no such file or directory: /tmp/claude-<uid>/anthrex-logs/probe`: Claude Code's sandbox sets `TMPDIR` to its own `/tmp/claude-<uid>` for Bash. A second run whose rewrite wrote to the task directory **by absolute path** (`<task TMPDIR>/anthrex-logs/probe`) succeeded, and printed `tmpdir=/tmp/claude-<uid>`. Decision 28 is unaffected, because `filter-hook` bakes the absolute `--log-dir` into the command and the task directory is in the worker's `allowWrite`; but anything that relies on a worker's Bash commands seeing M8a's per-task `TMPDIR` does not hold (follow-up).
- **Payload.** Keys: `cwd`, `hook_event_name`, `permission_mode`, `prompt_id`, `session_id`, `tool_input`, `tool_name`, `tool_use_id`, `transcript_path`; `tool_input` keys: `command`, `description`. `updatedInput` carrying every original key worked.
- Claude Code created an empty `.claude/.cc-writes/` directory in the working directory (items 3 and 4, not the deciders). git ignores empty directories, so `git status` stays clean.

**Item 4, the scout launch: run, and decision 12's read-only premise does not hold as specified.** A standalone checkout (`git clone --shared`), `--permission-mode dontAsk --allowedTools mcp__anthrex__submit_scout_report,Bash,Read,Glob,Grep --disallowedTools Edit,Write,NotebookEdit`, `--setting-sources user --strict-mcp-config`, a stub MCP server recording the call, `TMPDIR` set to a scratch directory, and the sandbox block with an **empty `allowWrite`**, the protected `denyWrite` entries and M8a's pins.
- `ls` and `git log -1` ran.
- **`touch x` in the working directory succeeded.** With an empty `allowWrite`, Claude Code's sandbox still lets Bash write its working directory.
- `touch "$TMPDIR/…"` succeeded, because `$TMPDIR` is Claude Code's own `/tmp/claude-<uid>` (item 3). A write to the process's `TMPDIR` by absolute path was denied (`Operation not permitted`).
- `curl https://example.com` failed: exit 56, `CONNECT tunnel failed, response 403` with `<sandbox_violations> deny network-outbound example.com:443 (host is not on the allow list)`.
- **The MCP call was allowed** under `dontAsk` with unscoped `Bash` allowed: the model loaded it through `ToolSearch` and the stub recorded `submit_scout_report` with its arguments. So M8b.9's launch is not stopped on that account.
- **A second run with the checkout root itself added to `denyWrite`** (`claude-2.1.280-scout-deny-cwd.jsonl`): `touch x` was denied, `ls`, `git log -1` and the MCP call still worked, the absolute-`TMPDIR` write and `curl` were still denied. `/tmp/claude-<uid>` stayed writable.

So the "empty-root `ClaudeSandbox`" M8a gives reviewers (F1c N4) and decision 12 gives scouts is **not read-only for Bash**: the working directory stays writable. A reviewer is covered by `dontAsk` and its scoped `Bash(git …)` allow rules (M8a.1 recorded `touch reviewer.txt` denied by mode), but a scout allows unscoped `Bash`, so the sandbox is its only barrier. Per AGENTS.md this was recorded, not worked around; the controller's ruling R-T1-1 takes the fix the probe shows works: add the checkout root to the scout's `denyWrite` (Claude Code's own `/tmp/claude-<uid>` stays writable either way; the scout's checkout is disposable and removed after salvage, decision 8).

**Item 5, OTLP: run.** `claude -p --no-session-persistence --model claude-haiku-4-5 "reply ok"` with the brief's six variables, against a listener in the probe's own process that wrote each request's headers and body and answered `200 {}`.
- **Two requests**, both `POST /v1/metrics`, `Content-Type: application/json`, a **`Content-Length`** body (1 601 and 6 798 bytes; not chunked), **no `Content-Encoding`** (no compression by default), `User-Agent: OTel-OTLP-Exporter-JavaScript/0.208.0`, `Connection: keep-alive`. The first carried only `claude_code.session.count`; the second `claude_code.cost.usage`, `claude_code.token.usage` and `claude_code.active_time.total`.
- **`claude_code.token.usage`**: a monotonic sum, `aggregationTemporality` **1 (delta)**, one data point per `type`: `input` (10), `output` (170), `cacheRead` (13 689), `cacheCreation` (12 503). Values are in **`asDouble`**, not `asInt`: M8b.15's `parse_metrics` must read `asDouble` (and should accept `asInt`).
- **Data-point attributes**: `type`, `model` (`claude-haiku-4-5`), `session.id`, `query_source` (`main`), `terminal.type` (`non-interactive`), the account identity (`user.id`, `user.email`, `user.account_uuid`, `user.account_id`, `organization.id`; redacted in the fixture), and **`anthrex.run` and `anthrex.role` copied from the resource**.
- **Resource attributes arrive**: `anthrex.run = r-fix`, `anthrex.role = orchestrator`, beside `host.arch`, `os.type`, `os.version`, `service.name = claude-code`, `service.version = 2.1.280`.
- **So `orchestrator_env`'s list is the documented set exactly**, without `OTEL_EXPORTER_OTLP_COMPRESSION`: `CLAUDE_CODE_ENABLE_TELEMETRY=1`, `OTEL_METRICS_EXPORTER=otlp`, `OTEL_EXPORTER_OTLP_PROTOCOL=http/json`, `OTEL_EXPORTER_OTLP_ENDPOINT=<addr>`, `OTEL_METRIC_EXPORT_INTERVAL=1000`, `OTEL_RESOURCE_ATTRIBUTES=anthrex.run=<run>,anthrex.role=orchestrator`.
- The export carries the user's account identity in every data point. anthrex's receiver is on loopback and M8b.15 keeps only the usage numbers; nothing should log a body.

**Stream usage names** (M8a.1 cross-reference). M8a.1 recorded `input_tokens`, `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens` and the rest for Claude; the decider calls show the same names. Codex's `turn.completed.usage` was recorded by M8a.1's Codex fixtures. Nothing to add.

**`DECIDER_CAPS`** (`crates/daemon/src/decider/argv.rs`)

| Field | Value | Set by |
|-------|-------|--------|
| `claude_json_schema` | `true` | item 1: accepted, and the answer arrived as structured output |
| `claude_max_turns` | `true` | item 1: hidden from `--help`, accepted; 2 turns (`blocked_reason`) or 3 (`triage`) used |
| `claude_no_session_persistence` | `true` | item 1: accepted |
| `answer_source` | `AnswerSource::ResultField` | item 1: `result.structured_output` (also a top-level `StructuredOutput` tool use and `result.result` text) |
| `strict_schemas` | `false` | item 1: a schema without every property in `required` was accepted; `anyOf`/nullable types too |
| `codex_output_schema` | `true` | item 2: the flag exists in `codex exec --help` (controller ruling); runtime behaviour unverified, outstanding |
| `codex_ephemeral` | `true` | item 2: the flag exists in `codex exec --help` |

`output_filter::HOOK_SETS_ALLOW` (M8b.8) is **`false`** (item 3).

**Deviations from the task text** (all in the meta files' `command`):
- Items 3 and 4 added `--effort low` (M8a's headless sessions pass `--effort` when `claude_effort_flag`; the cheapest setting).
- The recording group in items 3 and 4 was `sh -c '{ cat; echo; } >> <file>'`, M8a.1's recording command, not M3's `anthrex hook --window <n> --source claude` (no daemon ran). It sits in the same place (the first `PreToolUse` group, matcher `""`) and fires for the same events.
- Item 4's standalone checkout was `git clone --shared` (objects by alternate, like M8a's `prepare_scratch_in`), and its MCP server a stub that records `tools/call`, not `anthrex mcp`.
- Item 5's listener ran inside the probe's own Python process, so no background process existed to stop.
- Item 1 added two calls beyond the task (`--tools ""` and `--restricted`), at the controller's request.

**OUTSTANDING, for the user:** item 2 (the Codex call and the runtime behaviour of `--output-schema` and `--ephemeral`, the command above).

**Rulings on these findings** (the controller's R-T1-1 to R-T1-6, restated):
- R-T1-1 (M8b.9): a scout's Claude sandbox adds the checkout root, and every repository path it can see, to `denyWrite`, and a test asserts it. Reviewers are unchanged.
- R-T1-2: Claude's Bash sets `TMPDIR=/tmp/claude-<uid>`, shared by every sandboxed Claude session: a follow-up, not fixed in M8b.
- R-T1-3 (M8b.6): `fake-agent`'s `bash` step matches the real stream: `tool_use` shows the original command, `tool_result` the rewritten run.
- R-T1-4 (M8b.7): the fenced JSON in assistant or result text is parsed even when the turn limit is hit.
- R-T1-5 (M8b.15): the OTLP receiver never logs request bodies.
- R-T1-6 (M8b.5): Claude deciders pass `--tools ""`, keeping `--setting-sources user`.

### M8b.2 protocol (2026-09-25)

**`PROTO_VERSION` = 8.** `pub const PROTO_VERSION: u32 = 7;` stood at `crates/proto/src/lib.rs:25` before the change (set by M8a task 2), so main was at 7 as the header requires; 7 + 1 = 8. `proto_version_is_seven` became `proto_version_is_eight`. Every client reads the one constant (`tui::connection`, `anthrex hook`, `mcp::forward`, the CLI's run client), so none hard-codes a version.

**Deviations:**
- `RunReply::Profile` carries `Box<ProfileReply>`, not `ProfileReply`. Reason: `ProfileStatus` holds a `ProposalRecord` with two inline `RepoProfile`s (≥ 1536 bytes), which made `RunReply`, and `DaemonMsg` that every broadcast slot holds, trip `clippy::large_enum_variant`. A `Box` is invisible on the wire; the daemon writes `RunReply::Profile(Box::new(reply))`. `ProfileReply` itself carries `#[allow(clippy::large_enum_variant)]` (built once per request, and boxed where it is carried).
- `ScoutReport` and `ScoutInfo` also derive `Eq`: `RunInfo` (which derives `Eq`) holds `Vec<ScoutInfo>`, and `ProfileStatus` (`Eq`) holds `Option<ScoutInfo>`.
- The tests are split into `adapt_tests.rs` (494 lines) and its builders `adapt_tests_fixtures.rs`, to stay under 600 lines. `adapt_tests.rs` also loads M8a's `run_tests_fixtures.rs` a second time, as a private module, under `#[allow(clippy::duplicate_mod)]`; the M8a fixtures gained the new fields, unset.
- Budgets: `run_info.rs` grew +44 (222 → 266; budget +25) and `run_wire.rs` +89 (118 → 207; budget +50), because Interfaces puts every new field `#[serde(default)]` (one attribute line each) and places `ProfileRequest`/`ProfileReply` in `run_wire.rs`. Both files stay far under 600.
- Beyond the listed files, every constructor of `RunInfo`, `TaskInfo`, `CheckInfo` and `ToolCall` gained the new fields as `None`/empty: `run/snapshot.rs` (M8b's later tasks fill them), `cli/src/run_cmd/status_tests.rs`, `mcp/src/forward.rs` (`scout_id: None` until M8b.9), and the tests `engine/tests/{fixture,done_tools}.rs`, `mcp/tests/stdio.rs`, `fake-agent/tests/headless_modes.rs`.
- `crates/cli/src/run_cmd.rs` needed no change: its reply handling has no exhaustive `match` on `RunReply` (each site ends in `other => …`). The TUI's `DaemonMsg::Run(_)` arm and `anthrex hook` needed none either.
- `mcp::tools_for(Scout)` is empty and `role_name(Scout)` is `"scout"` until M8b.9 adds `submit_scout_report`; `anthrex mcp --role scout` parses. The daemon refuses `StartGoal`, `Promote`, `Stats` and `Profile(_)` with `Refused { request: <its label>, message: "not available yet" }` (test `every_milestone_8b_request_is_refused_until_its_task_lands`, `crates/daemon/tests/server_runs.rs`).
- `ProfileRequest` and `ProfileReply` carry no `#[serde(rename_all)]`, so their variants are PascalCase on the wire (`"Status"`, `"Confirm"`, …). This is by design: it matches M8a's `RunRequest` and `RunReply`, which carry them. (Task 2 review.)
- `proto::HISTORY_VERSION` is re-exported at the crate root with the other `history` names (added after the task 2 review; `adapt_tests.rs` imports it from the root).

### M8b.3 configuration (2026-09-26)

**Deviations:**
- `lib.rs`'s `pub use orchestrator::{…}` line is the only statement changed, but `git diff --stat baa04e1 -- crates/config/src/lib.rs` shows 4 insertions and 1 deletion, not one changed line: with the four new names the statement is 118 columns, and `cargo fmt --all --check` (required) wraps it over four lines. No item, `use` or `mod` was added; `lib.rs` goes 605 → 608 lines.
- Messages the brief does not spell out follow M8a's format: `orchestrator.scouts.runtime: must be claude or codex (using orchestrator.default_runtime)` (`None` means the default runtime), `…strength: must be fast, standard or frontier (using fast)`, `…effort: must be low, medium or high (using low)`, `orchestrator.metering.otlp_port: must be between 0 and 65535 (using 0)`, the M8a `expected a boolean` / `expected a table` messages for a wrong type. One extra test, `adapt_wrong_types_are_problems`, pins these.
- The four table readers and their known-key lists live in `orchestrator/adapt.rs`; `unknown.rs` imports the lists, so a key is known in one place. The types are defined in `adapt.rs` and re-exported by `orchestrator.rs` (`pub use adapt::{…}`), so `lib.rs` names them from `orchestrator` as the brief's line requires.
- No decider `tools` setting was added: R-T1-6 (Claude deciders pass `--tools ""`) is fixed argv in M8b.5, not configuration.

### M8b.4 profile store and precedence (2026-09-26)

**Deviations and choices the brief leaves open:**
- `derived_prefixes` takes `check`'s segments first, then `single_test`'s head. Decision 28 lists `single_test` first, but the task's exact expectation (`["cargo build", "cargo test", "cargo fmt"]` for `check = "cargo build --all && cargo test -q; cargo fmt --check"`, `single_test = "cargo test -- --exact {test}"`) holds only in that order; the test wins.
- `choose_profile`'s change to the plan and the cloned config is the pure `profile::resolve::apply_choice(&ChosenProfile, &mut ProfileSpec, &mut ProfileSpec)`, which `driver/adapt.rs::choose_profile` calls after loading. The pure tests "through `choose_profile`'s config clone" go through it and M8a's `resolve_profile`/`build_run`; `choose_profile` itself (I/O) is exercised end to end by M8b.19 as the brief says.
- `ProfileSource::Plan` when any `ProfileSpec` key, `protected` included, is set in the plan or `[orchestrator.profile]`; `None` only when both are empty.
- `from_findings` removes: list entries (`modules`, `hub`, `source`, `generated`, `protected`, `conventions`, `manifests`) that fail `validate_glob`; built-in and repeated `protected` entries; malformed or reserved `env` keys; an out-of-range `check_timeout_secs`. It leaves `single_test`/`test_passed` without `{test}` to verification, which drops them with M8a's message (decision 9).
- `validate` also holds `conventions` and `manifests` to `validate_glob`, and checks `check_timeout_secs` against M8a's 10–14400. Messages: `<key>: <entry> <validate_glob error>`, `check_timeout_secs: must be between 10 and 14400`, `single_test: must contain {test}`, `test_passed: must contain {test}`, `test_passed: is not a valid regular expression: <e>`, `env: key <K> must match [A-Za-z_][A-Za-z0-9_]*`, `env: key <K> may not be set by a profile: <reason>`.
- `fingerprint` never follows a symlink (review finding 2 fixed an earlier version that did, through `metadata` and `File::open`). A path with only plain relative components whose every directory is a real directory (`symlink_metadata`), naming a regular file opened with `O_NOFOLLOW`, is `"<16 hex>:<len>"` of its contents. A symlink is `"link:<16 hex>:<len>"` of its target's *name*, never its target's contents, so retargeting it still shows as stale. Anything else (absolute, `..`, under a symlinked directory, missing, a FIFO or device, unreadable) is `"missing"`. So a model-written `manifests` entry can neither read outside the repository nor block on a FIFO. The hash is plain FNV-1a 64 (not `role_launch::fnv1a`, which appends a separator).
- `store::load`: a missing `profile.meta.json` beside a `profile.toml` reads as a profile confirmed at 0 with no fingerprint (never stale); a meta file that does not parse is `Unparseable` naming the meta path, so `run start`'s refusal reads `the stored profile at <repo_dir>/profile.meta.json does not parse: …` (tests `a_missing_meta_reads_as_confirmed_at_zero_and_never_stale`, `a_corrupt_meta_is_unparseable_naming_the_meta_file`). `save` writes the meta first and `profile.toml` last (its rename is the commit point); two concurrent saves may pair one's meta with the other's profile, which at worst misstates `confirmed_at` or the fingerprint.
- Temp files (review finding 1): every `write_atomic` uses its own temp name, `<file>.<pid>.<counter>.tmp`, opened `create_new` with mode 0600, and removes it on failure; concurrent writes of one path therefore never share a temp file, and each rename installs one whole write. **Reads never unlink anything**: the first version's `load`/`load_proposal` removed `<file>.tmp` on every read and so failed a concurrent save's rename (the reviewer's probe: 750 of 750 saves failed). Leftovers of crashed writes are ignored by readers and removed only by `store::sweep_leftovers(repo_dir)`, which must run where no writer can: `ProfileService::restore` calls it at daemon start (M8b.11). Test `saves_succeed_while_other_threads_load` (two writers of the profile and the proposal, two readers) failed before the fix.
- `Stored` carries `#[allow(clippy::large_enum_variant)]` to keep the Interfaces' unboxed shape (as M8b.2 did for `ProfileReply`).
- `apply_edit`: the value is typed by `RepoProfile`'s deserializer (an error is `<key>: <toml message>`); `env.<NAME>` values are always strings; only problems of the edited key refuse the edit; the bool is true only when the key is in `REVERIFY_KEYS` and the profile changed. The unknown-key list is `proposal::EDIT_KEYS` (the fields, then `env.<NAME>`).
- `build` now parses the plan and calls `build_plan`, which starts with the confinement refusal; so a malformed plan on a host that cannot confine gets the parse error first (before, the refusal came first).
- The ignored-key notes go into `run.log` at build time (at the start's `now`), before the engine's `started` line. The stale attention line is `Run::stale_profile_line()` in `run/model_adapt.rs`, appended by `snapshot::attention`; `RunInfo.profile_source` is now filled.
- `run/model_adapt.rs` holds only that `impl Run` item for now, so `model.rs` declares the module without `pub use adapt::*` (an unused glob import fails clippy); the task that adds its first struct adds the re-export. Run's five new fields are on `Run` in `model.rs`, `#[serde(default)]` (430 → 449 lines); `build_run` sets them empty and the driver fills them.
- Also added now because Interfaces places them in `profile/mod.rs`: `profile::summary` (with a test) and `ONBOARDING_CHECKOUT`/`VERIFY_CHECKOUT`.
- Extra tests beyond the list: `edit_takes_a_bare_word_as_a_string_and_reports_reverification`, `summary_prints_the_built_ins_and_the_degradations`, `repo_dir_is_keyed_by_project`, `a_proposal_round_trips_and_is_deleted`, and `a_stale_profile_gets_the_attention_line` (written after `model_adapt.rs`, so it did not fail first). After review (finding 3), that test also checks the snapshot: `RunInfo.profile_source` and the stale line in `attention` (verified by mutation: dropping either wiring line fails it); `run/driver/adapt.rs` has `apply_choice_copies_every_field_and_logs_each_note`. After review, `saves_succeed_while_other_threads_load`, `fingerprint_never_follows_a_symlink_out_of_the_project` and the reworked `save_and_load_round_trip_atomically` failed before their fixes; the two meta tests pin behaviour that was already there.
- Pure-module doc comments are worded without the literal `std::fs`/`tokio`/… tokens, so the Verification purity grep prints nothing for these files.
- The Verification grep was `rg -n "cache_dirs|confined_" …`, which also matched `ProposalRecord.unconfined_checks` (M8b.2), a user flag and not a confinement setting. It is narrowed (controller ruling after review) to `\bcache_dirs\b|\bconfined_(network|unix_sockets|localhost_ports)\b`.
