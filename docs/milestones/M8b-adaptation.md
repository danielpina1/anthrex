# Milestone 8b: Adaptation plumbing

> Written 2026-09-22 against `docs/superpowers/specs/2026-09-22-adaptive-orchestrator-design.md` ("the spec" below), which binds this milestone, including its same-day amendment of §6 and §22.4: the repository profile lives only in anthrex's data directory, and nothing is ever written into the repository. Built on the milestone 8a brief, `docs/milestones/M8a-orchestration-engine-core.md` ("M8a" below), in its final headless form: only the orchestrator is an interactive PTY window, and every other agent — workers, reviewers, and the scouts and deciders this milestone adds — runs headless on M8a's session layer. Checked against `main` at `2cb7e3c` (milestones 1 to 6 merged, protocol 5) and branch `m6.5-conversation-view` at `89450b0` (milestone 6.5, protocol 6). Every name this brief takes from M8a, which is not yet code, is listed under "Names taken from M8a" (checked against the M8a brief on 2026-09-22); when M8a's merged code differs, use the merged name and record the mapping under "Implementation notes".

## Header

| | |
|--|--|
| Status | `blocked` — depends on milestone 8a, which is `blocked` itself. Becomes `ready` when milestone 8a is merged. |
| Depends on | Milestone 8a (the engine, headless sessions, the MCP crate, `fake-agent`'s headless modes, the run harness). Milestone 8c may run before or after this one, but not at the same time: both raise the protocol (roadmap, "Why this order"). |
| Spec sections | §4 (roles; the decider row; headless sessions; read-only launch), §4.2, §5.1 (triage, the fast path, `run promote`), §6 (the repo profile and the onboarding scout, as amended, `generated` and `protected` included), §7.1 and §7.2 rule 5 (the size cross-check), §10 (the ≤ 40-line check summary, classifying free-text `task_blocked` reasons), §11.3, §13 item 3 (deciders in reader slots), §14 items 1, 3, 5 and 8 (scout once, filtered output, deciders, OTLP for the orchestrator), §15 (recording only), §16.5 (new snapshot fields), §17 (scrubbed environment, nothing deleted dirty), §19 (scout tool; deciders have no tools), §21 (M8b row, the fast-path scenario), §22.1, §22.4, §23 (deciders are a second model surface; thresholds are placeholders). |
| Branch | `m8b-adaptation` |
| Protocol version | **One above `PROTO_VERSION` on `main` at the moment M8b starts** (8 if M8a merged at 7). Read `crates/proto/src/lib.rs` on `main` that day, add one, and record the derivation under "Implementation notes". No acceptance criterion greps for a specific number. |

## Starting point

Existing code this milestone reads or changes, verified on `main` at `2cb7e3c` and on `m6.5-conversation-view` at `89450b0`:

| Path | What is there |
|------|---------------|
| `crates/daemon/src/launch/claude.rs` (71 lines, both branches) | `HOOK_EVENTS` (10 events) and `settings(exe, window_id) -> serde_json::Value`, which puts **one** matcher group `{"hooks":[{"command":…,"type":"command"}],"matcher":""}` under each event. M8a's `headless::argv::claude_args` passes this object as `--settings`. |
| `crates/daemon/src/launch/mod.rs` | `pub fn shell_quote(s: &str) -> String` (line 42), `pub fn hook_command(exe, window_id, HookSource) -> String` (line 46). |
| `crates/daemon/src/worktree.rs` | `hash8(path) -> String` (FNV-1a 32-bit, line 119), `repo_worktrees_dir(worktrees_root, project_root) -> PathBuf` = `<root>/<sanitized basename>-<hash8>` (line 140), `RESERVED_DIR = "runs"` (line 48), `run_git` (line 230). |
| `crates/daemon/src/project.rs` | `detect_roots(cwd) -> DetectedRoots { project, worktree, detection_failed }`; `project` is shared by every linked worktree of one repository. |
| `crates/daemon/src/subprocess.rs` | `run_captured(command, max_output_bytes, max_stderr_bytes, timeout) -> Captured`, scrubbing the five `GIT_*` variables, own process group. |
| `crates/daemon/src/manager/config.rs` | `ManagerConfig::from_vars`: `ANTHREX_CLAUDE_BIN` / `ANTHREX_CODEX_BIN` (non-empty) win over `runtimes.*.command` (lines 90–106). `ANTHREX_DECIDER_BIN` follows the same pattern. |
| `crates/daemon/src/lib.rs` (59 / 61 lines) | `pub(crate) fn lock`, the module list. |
| `crates/proto/src/paths.rs` | `data_dir()` honours `ANTHREX_DATA_DIR`. |
| `crates/cli/src/main.rs` (527 lines, both branches) | `hook` is dispatched from raw `args_os()` **before** clap (line 151), so it starts fast. `filter-hook` and `filter-run` are dispatched the same way. |
| `crates/cli/src/hook.rs` (138 / 382 lines) | Silent, deadline-bounded forwarding (`HOOK_DEADLINE` = 1 s). `filter-hook` copies its discipline: always exit 0, never block. |
| `crates/cli/tests/support/mod.rs` (479 lines) | `tempdir()` under `/tmp` with prefix `ax-`, `fake_agent_bin()`, `isolated_command`, `TestDaemon`. Process-level tests of daemon library code that need `fake-agent` live in `crates/cli/tests/`, because `crates/daemon/tests/` has no `fake-agent` helper. |
| `crates/daemon/tests/support/mod.rs` | `TempRepo` (line 209), `git`, `git_output`. |
| `crates/fake-agent/src/runtime.rs` (195 lines) | `discover(args)` reads `--settings` and keeps only the **first** matcher group's first command per event (`entries.get(0)`). |
| `crates/fake-agent/src/script.rs` (190 / 213 lines) | `Step` enum; M6.5 adds `Transcript`. |
| `crates/config/src/lib.rs` (730 / 1033 lines) | Already over 600 lines. M8b adds nothing to it: M8a delegates `[orchestrator]` to `crates/config/src/orchestrator.rs`. |
| `docs/timing-budgets.md` | The rules for wall-clock test bounds. Every new bound below names the constants it is derived from. |
| `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` | Where follow-ups go. |

## Names taken from M8a

M8a is a brief, not code, when this brief is written. Every M8a name this brief relies on is below. Before starting, check each against the merged code; where one differs, use the merged name everywhere in this milestone and add a row to "Implementation notes" (`M8a name as written → merged name`). Nothing in this brief renames anything M8a defines; everything M8b adds to an M8a type is a new variant or a new `#[serde(default)]` field.

| Name | What this brief assumes it is | Taken from (M8a) |
|------|-------------------------------|------------------|
| `run::engine::step(state: EngineState, event: Event) -> (EngineState, Vec<Effect>)` | The pure reducer; reads no clock, does no I/O. | Decision 2; Interfaces `run/engine/mod.rs` |
| `EngineState { runs: BTreeMap<String, Run>, revision, stopped }` | The engine's whole state. | Interfaces `run/engine/mod.rs` |
| `Event { now, kind }`, `EventKind::{Start, Edit, Tool, OpDone, Signal, Restore, Tick, Resume}` | Reducer inputs; `Start { reply, run }` carries a built `Run`. | Interfaces `run/engine/mod.rs` |
| `Effect::{Reply, Op, Deliver, Interrupt, KillWindow, RetireWindow, RemoveWindow, Persist, WriteReport, Publish}` | Reducer outputs, executed by `RunService` in decision 43's order. | Interfaces; decision 43; task M8a.22 |
| `OpKind`, `OpResult`, `OpId`, `PendingOp { op, task_id, kind }` | Journaled side effects; `OpKind::{Check, MergeCandidate, ResumeSession, …}`, `OpResult::{Check, CandidateRed, Merged, Finished, …}`. | Interfaces; decisions 43–44 |
| `run::model::{Run, Task, Profile, RunLimits, CheckRecord, AgentRound, ReviewLevel, LogEntry}` | The persisted run model with the fields listed in M8a's Interfaces (`Run.project`, `Run.root`, `Run.profile`, `Run.limits`, `Run.log`, `Task.spec: PlanTask`, `Task.size`, `Task.state`, `Task.block`, `Task.rung`, `Task.rounds`, `Task.checks`, `Task.merge_commit`, `Task.start_commit`, `Task.head`, `Task.notes`, `Task.review_level`, `Task.review_route`, `Task.route`, `Task.budget`, `Task.spent_total`). | Interfaces `run/model.rs` |
| `run::plan::{parse_plan, resolve_profile, build_run, resolve_task, BuildContext, Preflight, PlanError}`, `run::validate::validate_tasks`, `EditScope` | Plan parsing and validation; `resolve_profile(plan, config)` is per key, plan over config. | Decisions 7–13; Interfaces |
| `run::roster::{find, first_at, peer, pick_reviewer}` | Roster policy by strength. | Decisions 23, 35, 39 |
| `run::globs::validate_glob(glob) -> Result<(), String>`, `proto::GateKind::{Done, Proof, Check, Review, Merge}`, `RunLimits.worker_sandbox` | Glob validation; gate names; whether Claude workers are sandboxed. | Decision 11; Interfaces `proto`, `run/model.rs`; decision 54 |
| `run::git::{preflight, salvage, remove_worktree, lock_worktree, read_ref, GitQueue}`, `crate::worktree::run_git` | Blocking git helpers taking `git: &OsStr` first and a timeout last; `GitQueue::write(repo, f)` serialises writes per repository. | Decisions 18, 20; Interfaces `run/git/` |
| `run::exec::{run_shell, summary, ShellOutcome, CHECK_TAIL_LINES, CHECK_SUMMARY_LINES}` | `/bin/sh -c` with timeout and a 200-line tail; `summary` = last 40 lines. | Decision 34; Interfaces `run/exec.rs` |
| `run::contract::{check_failed_message, candidate_red_message, worker_prompt, reviewer_prompt}` | Message texts; the first two quote "Last 40 lines:" and the summary. | Interfaces "Contracts and message texts" |
| `run::env::profile_env(profile, worktree)` | Profile env with `{worktree}` substituted. | Interfaces `run/env.rs` |
| `run::journal::{JournalLine, append, save_run}`, `run::reconcile::{reconcile, Reconciled::{Replay, NotStarted}}` | The intent journal and reconciliation per op kind. | Decisions 43–44 |
| `RunService::{new, restore, spawn, stop, snapshots, request}`, `RunContext { data_dir, worktrees_root, exe, socket_path, orchestrator, git_roots, git }` | The driver; `request(Start)` runs preflight and `build_run` before sending `Start`. | Task M8a.22; Interfaces `run/driver.rs` |
| `ANTHREX_TEST_ABORT_AFTER_INTENT=<op kind>[:<n>]` | Debug-build crash injection after the n-th intent line of a kind. | Decision 48 |
| `proto::{AgentRole::{Orchestrator, Worker, Reviewer}, RunRef, Strength, Effort, Size, TaskKind, TestMode, Route, RouteSpec, Budget, ProfileSpec, Plan, PlanTask, PlanEdit, ModelEntry, RunState, TaskState, BlockReason, GateCounts, Severity, Finding, DoneSignal::{TaskDone, TurnEndFallback}}` | Run types in `crates/proto/src/run.rs`. | Interfaces `proto` |
| `proto::{RunsSnapshot, RunInfo, TaskInfo, AgentRoundInfo, CheckInfo, BlockInfo, Spend, TokenUsage { input, output, cache_read, cache_write }}`, `TokenUsage::billable()` | The snapshot in `crates/proto/src/run_info.rs`; `AgentRoundInfo.usage` and `Spend.tokens` already meter stream usage per round and task. | Interfaces `run_info.rs`; decision 40 |
| `proto::{RunRequest, RunReply, ToolCall, request::*}` in `crates/proto/src/run_wire.rs`, nested in `ClientMsg::Run` / `DaemonMsg::Run` | Run wire messages. | Decision 3; Interfaces `run_wire.rs` |
| `proto::WindowKind::{Pty, Headless}`, `WindowInfo.kind`, `WindowInfo.run: Option<RunRef>` | Headless windows. | Decision 49 |
| `config::Orchestrator` with `read(table, problems)`, `config::{ClaudeAuth, ClaudeHeadless}`, `default_roster()` | The `[orchestrator]` table in `crates/config/src/orchestrator.rs`. | Interfaces `config`; decision 50 |
| `headless::{HeadlessSpec, McpTarget { role, run_id, task_id }, SessionArg::{New { uuid }, Resume { session_id }}, SessionEvent, TurnOutcome, FailureKind, HeadlessHandle, CliCaps, CLI_CAPS, InterruptMode}` | The session layer, "shared with M9's scouts, sub-planners and deciders". `HeadlessSpec.run_ref: Option<RunRef>`, `HeadlessSpec.mcp: Option<McpTarget>`. `HeadlessHandle::spawn(program, args, cwd, env, on_event)` scrubs `CLAUDE_CODE_*` and `CLAUDECODE`. | Decisions 24–28, 51–52; Interfaces `headless/` |
| `headless::argv::{claude_args, codex_args, mcp_args}`, `headless::claude_stream::{parse_line, user_message}`, `headless::codex_stream::parse_line` | Pure argv builders and stream parsers. | Interfaces `headless/argv.rs`, stream tables |
| `headless::argv::claude_settings(exe, window_id, sandbox: Option<&ClaudeSandbox>, caps) -> serde_json::Value`, `HeadlessSpec.claude_sandbox: Option<ClaudeSandbox { writable_roots }>`, `[orchestrator] worker_sandbox` | The `--settings` object: M3's hooks plus the sandbox block; `claude_args` calls it. | Decisions 24, 54 |
| `CLI_CAPS.claude_user_settings_only: Option<&[&str]>`, `run::git::project_settings(root, sha)`, `RunRequest::Start { …, trust_project }`, `Run.trusted_project`, `ANTHREX_TEST_NO_SETTING_SOURCES` | Headless Claude sessions load only the user's settings; when the CLI cannot exclude project settings, a repository that tracks them needs `--trust-project`. | Decision 53 |
| `ProfileSpec.generated: Option<Vec<String>>`, `generated_files_message(files)`, `GateCounts.done` | Files builds rewrite; outside `owns` they are a rung-1 bounce, not a spill. | Decision 55 |
| `ProfileSpec.protected: Option<Vec<String>>`, `BUILTIN_PROTECTED` (in `run/plan.rs`), `protected_file_message(files)`, `Preflight.protected_files`, `names_literally(owns, path)` | Files that configure or instruct future agents. The five built-ins always apply; every source only adds. A task may change a protected path only if its `owns` names it literally; otherwise it is a rung-1 gate failure of gate `done`. M8a enforces; M8b stores and proposes the extras. | Decision 56 |
| `CLI_CAPS.{codex_loads_project_config, codex_project_config_paths, codex_user_config_only}`, `ANTHREX_TEST_CODEX_PROJECT_CONFIG=load\|exclude` | Whether `codex exec` reads a repository's Codex config, which files it reads, and the flags that exclude them. If it loads them and they cannot be excluded, a repository tracking one needs `--trust-project`. | Decision 53 (the Codex paragraph); M8a.1 item 7a |
| `WindowManager::{create_headless, headless_send, headless_interrupt, headless_kill, signals}`, `WindowSignal { window_id, kind }`, `WindowSignalKind::{Session(SessionEvent), Hook { kind, agent_id }}`, and the removal `RunService` performs for `Effect::RemoveWindow` | Registering and driving headless windows; the event feed. | Decision 49; Interfaces `manager`; task M8a.22 step 3 |
| Decision 49's refusal helper in `crates/daemon/src/server/headless_guard.rs` and its text `window <id> is a headless session of run <run>; …` | Client control of headless windows is refused. | Decision 49; File sizes row `server.rs` |
| `crates/mcp`: `McpOptions { role, run_id, task_id, window_id, socket }`, `tools_for(role)`, `forward`, `TOOL_REPLY_TIMEOUT` (100 s); `anthrex mcp --role … --run … [--task …] --window … [--socket …]` | The MCP server crate and its hidden subcommand. | Decisions 4–5; Interfaces "MCP tools" |
| `fake-agent` headless modes: detection from `-p --input-format stream-json` (Claude) or `exec` (Codex); per-role scripts `<role>-<task>-<n>.jsonl` in `<git common dir>/fake-agent/`, claimed with `<file>.claimed`; steps `mcp_call`, `read_message`, `end_turn`, `sh`, `capture`, `usage`, `hang`; `FAKE_AGENT_ARGS_FILE`, `FAKE_AGENT_STDIN_FILE` | The scripted stand-in for both runtimes. | Task M8a.20 |
| `RunHarness` in `crates/cli/tests/support/run_harness.rs` (`new(config_toml)`, `script`, `plan`, `anthrex`, `start`, `subscribe`, `wait_run`, `restart_daemon`, `git`), `RUN_WAIT` = 300 s | The end-to-end harness. | Tasks, "Shared test helpers" |
| `engine/tests/fixture.rs`: `Fixture::new(plan_toml)`, `fx.send`, `fx.op`, `fx.done`, `fx.task`, `fx.signal` | The reducer test fixture. | Tasks, "Shared test helpers" |
| `crates/cli/src/run_cmd.rs`, `crates/cli/src/run_cmd/status.rs`, `RUN_REQUEST_TIMEOUT` = 180 s | The `anthrex run` subcommands. | Task M8a.23; CLI section |
| `run::report` (`REPORT.md`) | The run report, rewritten on `WriteReport`. | Task M8a.16 |
| `RunInfo.approved_by` values `"user"` and `"--yes"` | Who opened the plan gate. | Interfaces `run_info.rs` |
| Decision 32's `task_blocked { kind?, reason }` with missing `kind` read as `question`, reply `Blocked recorded (<kind>). Stop and wait for an answer.` | What M8b's classification refines. | Decision 32 |
| Decision 41's reader slots (`max_readers`, `RunInfo.readers_busy`), held by live reviewers | What deciders now also take. | Decision 41 |
| Decision 45's resume rules (tasks re-issue the op their state needs) | What re-issues a dropped decider op. | Decision 45 |
| `crates/daemon/tests/fixtures/headless/` and decision 51's shape test | Where recorded CLI fixtures live and how `fake-agent` output is checked against them. | Decision 51; task M8a.1 |

## Goal

The first time anthrex meets a repository, `anthrex profile detect` (or the first `anthrex run start --goal …` there) launches a headless **onboarding scout** in a disposable copy of the repository. It works out the languages, the modules, the hub, the source globs, a setup command, a check command, a single-test command with the regex that proves a test ran, and the per-worktree environment, running each command itself. anthrex runs every proposed command again in a fresh scratch worktree and proposes only those that passed. The user confirms the result once with `anthrex profile confirm`; it is stored in anthrex's data directory keyed by the repository root, never in the repository. It is re-detected when a manifest or convention file changes, and corrected with `anthrex profile edit`. Every run in that repository then uses it, over any profile in a plan file.

`anthrex run start --goal "<text>"` sends the goal to a **triage decider**: a one-shot headless call returning schema-validated JSON. A goal that one S or M task can do, touching no hub file, takes the **fast path**: one task, no plan gate, the normal gates, and the user's accept at the end. Any other goal is refused with the reason, because the planned path needs the orchestrator of milestone 9. `anthrex run promote` records the user's wish to promote a fast-path run, for milestone 9 to act on.

Three more deciders run inside the engine: a **size cross-check** of planned tasks against scout evidence, which can only raise a size; a **≤ 40-line check summary** that replaces M8a's raw 40-line tail in every check bounce; and a **classifier** for `task_blocked` reasons the worker gave no kind for. Every decider has a deterministic fallback, so a failed, slow or disabled decider degrades a decision and never blocks a run. In tests, `ANTHREX_DECIDER_BIN` replaces the decider binary with `fake-agent`.

A `PreToolUse` hook injected into every headless Claude worker rewrites test commands to run through `anthrex filter-run`, which keeps the full log on disk and shows the agent only the failures. The engine meters deciders, scouts and, through an OTLP receiver ready for milestone 9's orchestrator, the one PTY session. It appends one record per finished task to `history.jsonl`, and `anthrex run stats` summarises that history. No test needs a model.

## Scope

In:

- The repository profile: its type and file format, storage in the data directory, the precedence over a plan file's `[profile]`, staleness by content fingerprint, and `anthrex profile status|detect|show|confirm|reject|edit`.
- The onboarding scout: a headless session in a disposable worktree, command verification by the engine, proposals, confirmation, and re-detection.
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
| A learned router | Out of scope (spec §15) |

## Design decisions

Numbered and final. If one proves wrong or impossible, stop work on it, record the evidence under "Implementation notes", and continue with the tasks that do not depend on it.

### Structure

1. **Where the code lives.** All daemon code is in `crates/daemon/src/`:
   - `profile/`: `mod.rs` (types glue), `resolve.rs`, `proposal.rs`, `store.rs`, `verify.rs`, `service.rs`.
   - `scout/`: `mod.rs`, `contract.rs`, `spec.rs`, `report.rs`, `machine.rs`, `service.rs`.
   - `decider/`: `mod.rs` (types), `schema.rs`, `prompt.rs`, `parse.rs`, `fallback.rs`, `argv.rs`, `call.rs`.
   - `output_filter.rs`.
   - `metering/`: `mod.rs`, `otlp.rs`, `server.rs`.
   - In M8a's `run/`: `run/triage.rs`, `run/phases.rs`, `run/history.rs`, `run/stats.rs`, `run/history_io.rs`, and `run/engine/deciders.rs`.

   **Pure** (no `std::fs`, `std::process`, `std::thread`, `tokio`, `std::time::SystemTime`): `profile/{resolve,proposal}.rs`, `scout/{contract,spec,report,machine}.rs`, `decider/{mod,schema,prompt,parse,fallback,argv}.rs`, `output_filter.rs`, `metering/otlp.rs`, `run/{triage,phases,history,stats}.rs`, `run/engine/deciders.rs`. **I/O**: `profile/{store,verify,service}.rs`, `scout/service.rs`, `decider/call.rs`, `metering/server.rs`, `run/history_io.rs`. *(M8a decision 2's discipline; AGENTS.md rule 5's spirit on the daemon side.)*
2. **Protocol.** `PROTO_VERSION` becomes one above `main`'s value at the start (header). Every new request and reply is a new variant of M8a's `RunRequest` or `RunReply`, so `ClientMsg` and `DaemonMsg` gain nothing and the TUI's ignore arm for `DaemonMsg::Run` still covers everything. Every field added to an M8a struct is `#[serde(default)]`, and every new snapshot field is an `Option` or a `Vec`, so a milestone-8a `run.json` and a milestone-8a snapshot still deserialize. `proto::AgentRole` gains `Scout`. New proto modules: `crates/proto/src/profile.rs`, `scout.rs`, `adapt.rs` (triage, deciders, usage, promote), `history.rs`. Every new message gets a MessagePack round-trip test. *(AGENTS.md rule 4; spec §19 last line.)*
3. **Configuration.** New tables in a new file, `crates/config/src/orchestrator_adapt.rs`, read by one call from M8a's `orchestrator::read`. `crates/config/src/lib.rs` is not touched. Keys, defaults and ranges are in Interfaces. Invalid values are a `Problem` and keep the default, with M8a's message format. *(M8a decision 50's pattern; `lib.rs` is already over 600 lines.)*

### The repository profile

4. **Storage.** A repository's data directory is `repo_dir(data_dir, project) = worktree::repo_worktrees_dir(&data_dir.join("repos"), project)`, keyed by `project::detect_roots(dir).project`, so every linked worktree of a repository shares one. It holds:

   | File | Content |
   |------|---------|
   | `profile.toml` | The confirmed profile (`proto::RepoProfile`, the spec §6 format plus the keys in decision 5). |
   | `profile.meta.json` | `ProfileMeta`: when it was confirmed, from which proposal, the verification record, and the fingerprint (decision 7). |
   | `proposal.json` | `ProposalRecord`: the one pending proposal and its state (decision 8). |
   | `scouts/<scout id>.json` | Repository-level scout reports (decision 13). |
   | `history.jsonl` | Run history (decision 33). |

   Every write goes to a temp file, is `fsync`ed and renamed, and the directory is `fsync`ed. **Nothing is ever written into the repository**: there is no `.anthrex/` directory and no repository-level profile file, because an agent could edit such a file to weaken the checks it is judged by. *(Spec §6 as amended, §22.4.)*
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
   - `sample_test`: the name of one existing passing test, needed to re-verify `single_test`.
   - `check_timeout_secs`: M8a's key, so a slow check survives re-detection.
   - `manifests`: the manifest files the scout read, which decision 7 watches. Spec §6 names manifests but gives no key for them.
   - `filter_prefixes`: which commands count as test commands for the output filter (decision 28). Spec §14.3 filters "test output" but no profile key says which commands produce it.

   It parses with `deny_unknown_fields`. `RepoProfile::spec()` gives M8a's `ProfileSpec`: `modules`, `hub`, `source` and `generated` become `Some` only when non-empty, and `env` only when it has entries. `protected` is `Some(extras)` only when there are extras. M8a's resolution always unions `BUILTIN_PROTECTED` into the effective list, so an empty or absent value still protects every built-in. Every glob in `modules`, `hub`, `source`, `generated` and `protected` must pass M8a's `run::globs::validate_glob`.
6. **Precedence, exact.** `profile::resolve::run_profile(stored, plan, config)` picks one source for the whole profile:
   1. **A confirmed stored profile exists** for the run's `project`. It is the profile, in full: every `ProfileSpec` key comes from it, and a key it leaves unset stays unset. The degradations of spec §6 and M8a decisions 10, 34 and 35 then apply to that key. The plan file's `[profile]` table and `[orchestrator.profile]` are ignored entirely. For every key the plan's `[profile]` set, the run log gets `profile.<key> from the plan file is ignored: this repository has a stored profile (<repo_dir>/profile.toml)`. `RunInfo.profile_source = Some(Stored)`.
   2. **Otherwise**, M8a decision 7's rule applies unchanged: per key, the plan's `[profile]`, else `[orchestrator.profile]`, else empty. `profile_source = Some(Plan)` when any key came from either, else `Some(None)` (degraded mode, spec §6).

   **`protected` is the one exception to the whole-source rule**, because adding protection can only tighten. It merges, as M8a decision 56 defines: the effective list is `BUILTIN_PROTECTED` ∪ the stored extras ∪ the plan's `[profile] protected` ∪ `[orchestrator.profile] protected`, de-duplicated in that order. So `run_profile` puts into the chosen `ProfileSpec.protected` the stored extras plus the plan's and config's entries, and M8a adds the built-ins. No source can remove a built-in. A plan's `protected` entries are therefore not ignored, and get no "ignored" log line.

   The stored profile wins as a whole, not per key, because its gaps are deliberate. A stored profile with no `single_test` records that the repository has no single-test runner, and a plan must not quietly fill that in. A stored profile whose file does not parse **refuses the run**: `run start` answers `the stored profile at <path> does not parse: <error>; fix it with anthrex profile edit or re-detect it with anthrex profile detect`. It never falls back to the plan's profile. `output_filter` and `filter_prefixes` are copied onto the run for decision 28; they are empty when the source is not `Stored`. `RunService::request(Start)` applies this before calling M8a's `build_run`, by replacing `plan.profile` with the chosen `ProfileSpec`, so `build_run` itself is unchanged. `StartGoal` requires source 1 (decision 22). *(Spec §6 as amended; the coordinator's precedence: stored, else the plan's `[profile]`, else none.)*
7. **Staleness.** `ProfileMeta.fingerprint` maps each path in `conventions` and `manifests` (relative to the project root) to `fnv1a64(contents)` as 16 hex digits and the byte length, or `"missing"` for a missing file. Only the first 4 MiB of a file is hashed; its full length is still recorded. `profile::store::stale(project, meta) -> Vec<String>` lists every path whose fingerprint differs. It runs on `spawn_blocking` at every `run start`, at `profile status`, and at daemon start for every stored profile. A stale profile **is still used**; a changed file does not make it wrong. The run gets the attention line `the repository profile may be stale: <paths> changed since it was confirmed; run anthrex profile detect`. With `[orchestrator.onboarding] auto = true`, and no proposal pending or failed within the last hour, a re-detection starts (decision 8). *(Spec §6 "re-proposed when any file named in `conventions`, or a manifest, changes hash".)*
8. **Detection flow and proposal states.** `ProfileService` owns at most one proposal per repository, in `proposal.json`:
   `Preparing` → `Scouting` → `Verifying` → `Ready` → confirmed (the proposal is deleted and `profile.toml` written), or → `Failed { reason }` from any state.
   1. **Preparing.** A disposable worktree is created: `git worktree add --detach <wt>/runs/.onboarding HEAD` in the project root through `GitQueue::write`, then `git worktree lock`. `<wt>` is `repo_worktrees_dir(worktrees_root, project)`. The name starts with `.`, which a run id cannot, inside M5's reserved `runs` directory, which a user branch cannot claim.
   2. **Scouting.** The onboarding scout runs there (decisions 9 and 12). Its report arrives through `submit_scout_report`.
   3. **Verifying.** The scout's worktree is salvaged if dirty (M8a decision 20, ref `refs/anthrex/salvage/onboarding/<unix secs>`) and removed. Then `profile::verify` runs the proposed commands in a **fresh** scratch worktree `<wt>/runs/.profile-verify`, created and removed the same way (decision 9).
   4. **Ready.** `proposal.json` holds the proposed profile with only the commands that passed, the verification record, and each dropped command with its reason and output tail.

   Detection starts from `anthrex profile detect`, from decision 7's automatic re-detection, and from `run start --goal` in a repository with no stored profile when `onboarding.auto` is true. A second `detect` while one is in progress is refused: `detection is already running for <project> (state <state>); anthrex profile reject stops it`.
9. **The scout runs the commands; the engine re-runs them.** Spec §6 requires the onboarding scout to run `check` and `single_test` successfully before proposing them. Spec §4 makes scouts read-only. This brief resolves the conflict as follows (see "Spec defects"):
   - The **onboarding scout** is the one scout that runs commands, and it stays read-only with respect to the repository:
     - It works in a disposable detached worktree that is discarded afterwards, never in the user's checkout.
     - Its `Bash` runs under Claude Code's sandbox (M8a decision 54's block, with that worktree as the only writable place and no extra roots), or Codex's `workspace-write` sandbox.
     - It has no `Edit` or `Write` tool.
     - It reports, for each command, whether it ran it successfully (`setup_ran_ok`, `check_ran_ok`, `single_test_ran_ok`). The engine records these claims and does not trust them.
     - A command the sandbox stops, such as a `setup` that needs the network, can still pass the engine's own run below, which is unsandboxed, as M8a runs `setup`.
   - The **engine** then runs each proposed command in the fresh scratch worktree, with the proposed `env` (`{worktree}` substituted by `run::env::profile_env`) and M8a decision 26's scrubbed environment, through `run::exec::run_shell`, each bounded by `onboarding.verify_timeout_secs`:
     1. `setup`, if proposed. On failure it is dropped, and the check and single test still run.
     2. `check`. It is kept only if it exits 0.
     3. `single_test`, with `{test}` replaced by `launch::shell_quote(sample_test)`. It is kept, together with `test_passed` and `sample_test`, only if it exits 0 **and** its output matches `test_passed` with `{test}` replaced by `regex::escape(sample_test)`. A missing `sample_test` or `test_passed` drops all three with the reason `single_test needs sample_test and test_passed to be verified`.
   - **A command that did not pass here is not proposed.** It moves to `ProposalRecord.dropped` with its exit code, timeout flag and last 40 output lines, and `profile show --proposed` prints them. `test_passed` must contain `{test}` and compile as a regex, and `single_test` must contain `{test}`, or they are dropped with M8a decision 7's validation message.
   - `generated` and `protected` are glob lists, not commands. They are validated with `validate_glob` and proposed as the scout gave them, with built-in `protected` entries dropped (decision 5). They are never "verified" by running anything.
   - The scratch worktree is salvaged if dirty and removed. The user's checkout is never written.
   - Verification runs on `spawn_blocking`, never under a lock. *(Spec §6, §23 "The onboarding scout must prove both commands ran"; §17 "Nothing is deleted dirty".)*
10. **Confirm, reject, edit and show.** The user never writes the profile by hand (spec §6).
    - `anthrex profile confirm [--yes]` prints the `Ready` proposal (the `show` text of Interfaces, CLI) and asks `store this profile for <project>? [y/N]` unless `--yes`. It then writes `profile.toml` and `profile.meta.json` with the fingerprint computed now, and deletes `proposal.json`. It is refused unless the proposal is `Ready`.
    - `anthrex profile reject` kills a running scout (`headless_kill`), removes any detection worktree (salvaged), and deletes `proposal.json`.
    - `anthrex profile edit <key> <value>` and `anthrex profile edit --unset <key>` correct one value. The key is one of `RepoProfile`'s fields, or `env.<NAME>` for one environment entry. The value is parsed as a TOML value (`toml::from_str::<toml::Table>("v = <value>")`); if that fails it is taken as a string, so `anthrex profile edit check 'cargo test'` works. The edit is applied to the stored profile (refused when there is none: `no stored profile for <project>; run anthrex profile detect first`) and becomes a new proposal. If it changed `setup`, `check`, `check_timeout_secs`, `single_test`, `test_passed`, `sample_test` or `env`, the proposal goes to `Verifying` with all three commands re-run (decision 9); otherwise it goes straight to `Ready`, carrying the stored verification. `--yes` confirms it automatically once `Ready`, but only if nothing the edit touched was dropped. An edit is refused while another proposal is in progress.
    - `anthrex profile show [--proposed] [--json]` prints the stored profile, or the proposal, as TOML, followed by the verification.
    - `anthrex profile status [--json]` prints `ProfileStatus`.

    Every request answers at once. Verification and scouting run in the background, and progress is visible through `status`. *(Spec §6 "correct a value with `anthrex profile edit`", "re-run detection with `anthrex profile detect`".)*
11. **Detection across a daemon restart.** When the daemon starts, `ProfileService::restore` finds every `proposal.json` in `Preparing`, `Scouting` or `Verifying`. It marks each `Failed { reason: "the daemon restarted during detection; run anthrex profile detect" }`, then salvages and removes `<wt>/runs/.onboarding` and `<wt>/runs/.profile-verify` if present. M8a's reconcile kills a leftover scout process by its session id (M8a decision 28) as for any headless window. The scout is **not** resumed, unlike run sessions (spec §17): detection writes nothing but its own proposal and is cheap to repeat, and resuming it would need a journal for a flow that has no side effects to reconcile. `Ready` and `Failed` survive a restart unchanged.

### Scouts

12. **Scout sessions are M8a headless sessions.** `scout::spec::headless_spec(scout: &ScoutSpec, ctx: &ScoutContext) -> HeadlessSpec` builds the session, and `ScoutService` registers it with `WindowManager::create_headless`:
    - **Route.** `scout::spec::route(roster, runtime, strength, effort)`: the first roster entry of `runtime` at the lowest strength at or above `strength`, else the same on the peer runtime, else the first entry of `runtime`. `runtime` is `scouts.runtime`, else `orchestrator.default_runtime`; `strength` and `effort` are `scouts.strength` (`fast`) and `scouts.effort` (`low`). Spec §4 gives scouts the fast tier at low effort.
    - **Area scout** (`ScoutKind::Area`, M9's) — read-only:
      - `cwd` is the project root;
      - `instructions` is `SCOUT_CONTRACT`;
      - `allowed_tools` is `mcp__anthrex__submit_scout_report`, `Read`, `Glob`, `Grep`, plus `WebFetch` and `WebSearch` when `ScoutSpec.web`;
      - `claude_permission_mode = Some("plan")`, or M8a decision 24's reviewer fallback when M8a.1 found that plan mode blocks an allowed MCP call;
      - `codex_sandbox = "read-only"`.
    - **Onboarding scout** (`ScoutKind::Onboarding`):
      - `cwd` is the disposable worktree;
      - `instructions` is `ONBOARDING_CONTRACT`;
      - `allowed_tools` is `mcp__anthrex__submit_scout_report`, `Bash`, `Read`, `Glob`, `Grep`, with no `Edit` or `Write`;
      - `claude_permission_mode = Some("default")`: with `--permission-prompts none` (M8a decision 24), anything not allowed is denied, never prompted;
      - `claude_sandbox = Some(ClaudeSandbox { writable_roots: vec![] })` when `[orchestrator] worker_sandbox` is true (M8a decision 54), so `Bash` can write only in the disposable worktree. When `worker_sandbox` is false it is `None`, and `ProfileStatus` says `onboarding scout sandbox: off`;
      - `codex_sandbox = "workspace-write"` with no extra writable roots.

      The scout commits nothing. The engine's own run of each command (decision 9) is the one that counts, so a command either sandbox refused can still pass.
    - **Both:**
      - `mcp = Some(McpTarget { role: Scout, run_id, task_id: None, scout_id: Some(id) })`, with `run_id` empty for a repository-level scout;
      - `run_ref = Some(RunRef { run_id, task_id: None, role: Scout, session: 1 })` for a run scout, and `None` for the onboarding scout;
      - `env` empty;
      - `claude_auth` and `api_key_helper` from `[orchestrator.claude]`.
    - **Only the user's settings load** (M8a decision 53). `claude_args` already passes `CLI_CAPS.claude_user_settings_only` on every launch, so scouts get it without doing anything. When those flags are `None`, the project-settings check of decision 53 runs before a scout starts. It uses `run::git::project_settings(root, <HEAD sha>)` for the onboarding scout, whose worktree is checked out from `HEAD`, and the run's base commit for a run scout. If the check finds project settings, `anthrex profile detect` is refused with `this repository has project settings that headless Claude sessions would run without asking: <paths>; review them, then detect again with --trust-project`, and `--trust-project` accepts them. Automatic detection (decisions 7 and 22) is refused the same way; the message is stored as the proposal's `Failed` reason.
    - **Codex scouts** follow the same rule with M8a's Codex mechanism. `codex_args` passes `CLI_CAPS.codex_user_config_only` on every turn. When that is `None` and M8a.1 found that Codex loads a repository's config, a repository that tracks `.codex/` config needs `--trust-project` exactly as above, with the paths from M8a's project-settings check.
    - **Window name:** `scout/<scout id>`.
    - **Refusals.** Decision 49's refusals apply to scout windows as to any headless window. For one with `run == None`, the text becomes `window <id> is a headless scout session; only the daemon drives it. Use anthrex profile reject to stop it`. *(Spec §4 table row Scout, read-only launch.)*
13. **`submit_scout_report` and the report.** The tool's schema is in Interfaces. `scout::report::validate(args, kind) -> Result<ScoutReportArgs, String>` checks it again on the daemon side and rejects with `invalid arguments: <field>: <problem>`. `profile` is required for `Onboarding` and refused for `Area`. An accepted report becomes `proto::ScoutReport` and is written atomically:
    - repository-level scouts: `<repo_dir>/scouts/<scout id>.json`;
    - run scouts (M9): `<data_dir>/runs/<run>/scouts/<scout id>.json`.

    Scout ids match `^[a-z0-9][a-z0-9-]{0,47}$`, and an onboarding scout's id is `onboarding-<unix secs>`. `ProfileMeta.report` names the report a confirmed profile came from. In a task's `scout_refs`, the alias `onboarding` resolves to that report. The tool replies `Report recorded. You are done; end your turn now.` A second report is refused: `a report for scout <id> was already recorded`. So are a report from a window that is not that scout's (`this window is not scout <id>`) and a report for a finished scout (`scout <id> is <state>`). A summary of 8000 characters is about the spec's "≤ ~2k tokens". *(Spec §4 table, §14 item 1.)*
14. **The scout lifecycle** is a pure machine, `scout::machine::step(ScoutState, ScoutEvent) -> (ScoutState, Vec<ScoutEffect>)`, driven by `ScoutService`:
    - **Starting.** `Start` delivers the first turn: `ONBOARDING_FIRST_TURN`, or the question for an area scout.
    - **Turn ends.** A `TurnEnded` without an accepted report sends `SCOUT_NUDGE` once. A second one fails the scout: `the scout ended two turns without a report`.
    - **Process exit.** `ProcessExited` without a report fails it: `the scout's process exited without a report (code <c>)`.
    - **Tool budget.** Every `ToolUse` counts. At `scouts.max_tool_calls` the machine sends `SCOUT_WRAP_UP` once; at 1.5 times that it kills the scout and fails it: `the scout used <n> tool calls without a report`.
    - **Timeout.** `scouts.timeout_secs` after the start, the scout is killed and failed: `the scout ran longer than <n> s`.
    - **Report accepted.** The machine emits `Finish(Report)`, then `CloseStdin`. `KillAfter(INTERRUPT_GRACE)` and `RemoveAfter(RETIRE_AFTER)` follow, as M8a decision 52 retires a reviewer.

    Usage from every `TurnEnded` is summed into the report's `usage`. `ScoutService` subscribes to `WindowManager::signals()` and forwards only its own windows' events. It holds its table under `daemon::lock` and releases the lock before any manager call.
15. **MCP plumbing, all additive.** `headless::McpTarget`, `mcp::McpOptions` and `proto::ToolCall` each gain `#[serde(default)] scout_id: Option<String>`. `headless::argv::mcp_args` appends `--scout <id>` when it is set. `anthrex mcp` accepts `--role scout` and `--scout <id>`. `--run` becomes optional for role `scout` only (an empty string when absent); for the other roles its absence is still an error. `tools_for(Scout)` is `[submit_scout_report]`. `RunService::request(Tool)` routes a call whose `role == Scout` to `ScoutService::tool`, and every other call to the engine as before.

### Deciders

16. **The decider call** is `decider::call::decide(ctx: &DeciderContext, request: &DeciderRequest) -> Decision`, which is async. It uses M8a's `HeadlessHandle::spawn` and stream parsers, so a decider is a one-turn headless session with no window and no session kept.
    - **Program.** `ANTHREX_DECIDER_BIN`, if set and non-empty, read once at daemon start. Otherwise the resolved `claude_bin` or `codex_bin` from `ManagerConfig`, by `deciders.mode`.
    - **Working directory.** `<data_dir>/deciders/cwd`, an empty directory created at start. A decider gets everything it needs in its prompt, and running outside the repository keeps the repository's `.claude/settings.json` hooks and `.mcp.json` servers out of it (M8a risk 6).
    - **Claude argv** (`decider::argv::claude_decider_args`, pure):
      - `-p --input-format stream-json --output-format stream-json`, then `--verbose` when `CLI_CAPS.claude_verbose`;
      - the flags in `CLI_CAPS.claude_user_settings_only`, when `Some`: only the user's settings load (M8a decision 53). The empty working directory already has no project settings, and the flags keep that true if the CLI ever looks further up;
      - `--permission-prompts none`, or `--permission-mode dontAsk` when the caps say it is absent;
      - `--permission-mode plan`;
      - `--disallowedTools Bash,Edit,Write,NotebookEdit,Agent,WebFetch,WebSearch`;
      - `--max-turns 3`;
      - `--json-schema <schema JSON>`;
      - `--no-session-persistence`;
      - `--model <model>` when the route names one;
      - `--effort <e>` when `CLI_CAPS.claude_effort_flag`;
      - M8a decision 50's authentication flag. With `api_key` and a helper, `--settings {"apiKeyHelper":…}`.

      The prompt is one stream-json user message (`claude_stream::user_message(prompt, None)`) written to stdin, which is then closed.
    - **Codex argv** (`codex_decider_args`, pure): `exec --json --skip-git-repo-check --ephemeral`, then `CLI_CAPS.codex_user_config_only` when `Some` (the empty working directory has no `.codex/` either), then `-s read-only -c approval_policy="never" -c model_reasoning_effort=<toml e> --output-schema <schema file>`, then `-m <model>` when the route names one, then `--`, then the prompt as the last argument. Schema files are written once to `<data_dir>/deciders/schemas/<kind>-<fnv1a64 of the schema, 16 hex>.json`.
    - **Route.** `scout::spec::route` with `deciders.strength` (`fast`) and `deciders.effort` (`low`), on runtime `claude` or `codex` by mode.
    - **Answer**, taken from the session's events in this order:
      1. a `SessionEvent::StructuredOutput { value }`, which M8b adds to `claude_stream::parse_line` if M8b.1 finds the answer in `result.structured_output`, and which M8a's consumers treat as `Other`;
      2. otherwise a top-level `ToolUse` named `StructuredOutput`, taking its `input`, if M8b.1 finds that form;
      3. otherwise the last top-level `AssistantText`, trimmed, with one surrounding fenced code block removed, parsed as JSON.

      `usage` comes from `TurnEnded`.
    - **Bounds.** The call waits for `TurnEnded` or `ProcessExited`, bounded by `deciders.timeout_secs`; on timeout it calls `handle.kill(Duration::from_secs(2))`. At most 256 KiB of assistant text is kept.

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
    - **Slots.** A decider op holds one of the run's reader slots while in flight, beside live reviewers, and `RunInfo.readers_busy` counts both. Queued deciders are started before queued reviewers whenever a slot frees.
    - **Slot wait.** A decider still queued `deciders.slot_wait_secs` (default 30) after it was queued is answered on the next `Tick` by its fallback, with reason `no reader slot was free within <n> s`. A bounce can therefore wait at most that long for a slot.
    - **Mode off.** With mode `off`, the reducer applies the fallback inline, with reason `deciders are off`, and emits no op.
    - **Reconcile and resume.** Reconcile maps `Decide` to `NotStarted`. On resume (M8a decision 45), a task whose `pending_decider` op was dropped queues it again.
    - **Accounting.** Every `Decided` adds to `Run.decider_calls` and, for a fallback, `Run.decider_fallbacks`. Usage goes to the task's and the run's decider usage (decision 29), and a report line records the source.

    Triage is the one decider outside a run (decision 22), so it takes no slot. *(Spec §13 item 3 "scouts, reviewers and deciders use `max_readers`".)*
19. **The size cross-check, §7.2 rule 5.**
    - **When.** It runs on `Start` for every task, and on every accepted `Edit` for the tasks the batch added or amended (M8a decision 13's touched set).
    - **Evidence.** A task's evidence is the scout reports its `scout_refs` name (run scouts first, then the alias `onboarding`). A task with no `scout_refs` uses the onboarding report of the stored profile, if there is one. Tasks with no evidence get the note `size cross-check skipped: no scout evidence` and no call.
    - **One call per batch.** The evidenced tasks go into one `SizeCheck` request (at most 50 tasks). Each task's `size_check` becomes `Pending`, and a pending task is **not runnable** (M8a decision 41).
    - **Raises.** When the answer arrives, each task whose answered size is larger than its engine size is raised, since this rule, like the other §7.2 rules, can only raise:
      - **S → M.** `run::engine::deciders::apply_raise` sets `size = M`. It re-derives effort and budget by M8a decision 8 when the plan did not set them, and the review level and reviewer route by decision 35. It adds the note `size raised from S to M: decider cross-check (rule 7.2.5): <reason>`.
      - **To L.** The task becomes `blocked(mis_sized)` with the text `the size cross-check judged this task L: <reason>; split it (rule 7.2.5)`. No rung is counted.
      - **No raise.** A smaller or equal answer changes nothing.
    - **Recording.** Every answered task records `SizeCheckInfo { engine, decided, agreed, reason, source }`, and a disagreement is written to the report.
    - **Missing tasks.** A task missing from the answer keeps its size, with `source: Fallback`.
    - **Fast path.** The single task of a fast-path run is not cross-checked. The triage decider sized it from the same evidence a moment earlier, so asking again would ask the same model the same question. It gets `SizeCheckInfo { source: Fallback, reason: "sized by triage" }`. *(Spec §7.2 rule 5.)*
20. **The check summary, §10 and §11.3.**
    - **Task check.** When a task's check fails (`OpResult::Check { ok: false }`, M8a decision 34), the reducer records the `CheckRecord` and counts the failure on the ladder (M8a decision 38) as before. It then **defers the rung's action**: it stores `Task.pending_failure = Some(PendingFailure { gate: Check, rung, check_index })` and emits a `CheckSummary` decider.
    - **Candidate check.** A red merge candidate (`OpResult::CandidateRed`) does the same with gate `Merge`. The merge queue moves on at once.
    - **Resuming the action.** On `Decided`, the check record gets `summary` and `summary_source`. The deferred action then runs, with the message built from the summary: `check_failed_message` and `candidate_red_message` take the summary text in place of M8a's tail. While deferred, the task keeps its state (`Check`, or `MergeQueue` after a red candidate, outside the queue).
    - **Reviewers.** The reviewer prompt's `Last check (40 lines):` block uses the latest check's summary when one exists.
    - **Proof failures** keep M8a's command and 40-line tail, because spec §23 requires a proof failure message to show the command and its output. *(Spec §10 rung 1 "the decider's ≤ 40-line check summary", §11.3, §14.3 last sentence.)*
21. **Classifying a free-text `task_blocked`, §10.**
    - **Without a kind.** A `task_blocked` whose `kind` is absent makes the task `blocked(question)` at once, as in M8a. It adds `pending_classification = true` and emits a `BlockedReason` decider. The reply becomes `Blocked recorded (classifying). Stop and wait for an answer.`
    - **On `Decided`:**
      - `question` keeps `blocked(question)`;
      - `environment` changes the block reason to `environment`;
      - `mis_sized` applies rung 3 exactly as a typed `mis_sized` would (M8a decision 38).
    - **Recording.** `TaskInfo.block_source` records `Decider` or `Fallback`.
    - **A typed kind** is never reclassified; `block_source` stays `None`.
    - **Answered first.** An `answer` edit that arrives before the classification clears it and proceeds as M8a does. *(Spec §10 "A decider classifies free-text reasons the worker did not type".)*

### Triage and the fast path

22. **`anthrex run start --goal "<text>" [--trust-project]`** sends `RunRequest::StartGoal { goal, dir, yes, trust_project }`. `RunService::request` does everything below before any engine event, with no lock held across an await:
    1. **Preflight.** M8a's `run::git::preflight` runs on `spawn_blocking`, followed by M8a decision 53's project-settings check with `trust_project`, exactly as for `Start`. It runs only when `claude_user_settings_only` is `None` and the goal's run could have a Claude task, which is always true for a goal run, because triage has not yet chosen a runtime.
    2. **Profile.** `ProfileService::effective(project)` must return a stored profile (decision 6, source 1). Otherwise the request is refused with one of these, exactly, and nothing else happens:
       - `this repository has no stored profile; detection has started (anthrex profile status), then confirm it with anthrex profile confirm and start the goal again` — this also starts detection, when `onboarding.auto` is true and nothing is pending;
       - `this repository has no stored profile; run anthrex profile detect, then anthrex profile confirm` — when `onboarding.auto` is false;
       - `this repository has no stored profile; detection is <state> (anthrex profile status)` — while a proposal is in progress;
       - `this repository has no stored profile; a proposal is ready: anthrex profile show --proposed, then anthrex profile confirm` — when one is `Ready`;
       - the stored-profile parse error of decision 6.
    3. **Triage.** The input is the goal (cut to 4000 characters), `profile::summary(&profile)`, the onboarding report's summary and files if one exists, and the tracked file list from `git ls-files -z` through `run_git` (at most 1500 paths or 48 KiB, sorted, with `(<n> of <total>)`). The call is `decider::call::decide`; mode `off` gives the fallback at once. Triage takes no reader slot, because no run exists yet.
    4. **Route.** `run::triage::route(&answer, fast_path_enabled) -> TriageRoute` (decision 23).
    5. **Fast.** The task becomes a one-task `Plan` (`goal`, `tasks: [t1]`, the stored profile's spec) and goes through M8a's `build_run`, with `BuildContext.yes = true`.
       - If `build_run` fails, or the resolved task is `hub`, or its size is L, the route becomes `Plan` with the reason `the fast path does not apply: <first PlanError or "task t1 touches a hub file" or "task t1 is L">`.
       - Otherwise the run gets `path: Some(Fast)`, `triage: Some(..)` and `approved_by: Some("fast path")`, and `Event::Start` is sent. The reply is `RunReply::Triaged { triage, run_id: Some(id), message }`.
    6. **Plan or Large.** Nothing is created: no branch, no run directory, no window. The reply is `RunReply::Triaged { triage, run_id: None, message }`, with `run::triage::refused_message` (Interfaces).

    The CLI waits `GOAL_REQUEST_TIMEOUT` = 810 s, derived as `RUN_REQUEST_TIMEOUT` (180 s) + the largest configurable `deciders.timeout_secs` (600 s) + 30 s for `git ls-files` (its `run_git` bound). *(Spec §5.1.)*
23. **Triage routing** (`run::triage::route`, pure), in order:
    1. The answer's `source` is `Fallback`: route `Plan`, with reason `triage fell back (<fallback reason>); without a decider the path is plan`.
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
      - replies `Done` with `recorded: run <id> is marked for promotion to a planned run. Until the orchestrator exists (milestone 9) nothing else changes: the fast-path task continues and the run finishes as a fast-path run.`
    - Already promoted: `Done` with `run <id> was already marked for promotion at <hh:mm>`.
    - A run that is not fast-path: `Refused` with `run <id> is not a fast-path run`.
    - A terminal run: `Refused` with `run <id> is <state>`.

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
    - **Injection.** `HeadlessSpec` gains `#[serde(default)] output_filter: Option<FilterHook { mode, prefixes, log_dir }>`. `headless::argv::claude_args`, right after it calls M8a's `claude_settings`, calls `output_filter::add_hook(&mut settings, exe, spec.output_filter.as_ref())`, whose signature is unchanged. That adds a **second** matcher group: `"PreToolUse": [<M3's group unchanged>, {"matcher":"Bash","hooks":[{"type":"command","command":"<filter-hook command>"}]}]`. The sandbox block and the user-settings flags are untouched. `filter-run` executes inside the worker's sandbox, and it writes its log to `log_dir`, which is outside the worktree. So `log_dir` is added to the worker's `ClaudeSandbox.writable_roots` when the hook is set. Logs never go inside the worktree: they would show up as untracked files and fail M8a's clean-tree check in `task_done`. If M8b.1 finds the write refused even with the root added, stop work on the injection (the hook, `filter-run` and their tests still land), and record the evidence under "Implementation notes". M8a's `worker_spec` sets it for Claude workers when the run's profile source is `Stored`, its `output_filter` is not `none`, and there is at least one prefix. `log_dir` is `<data_dir>/runs/<run>/logs/<task>`. Reviewers, scouts, deciders and Codex sessions never get it. *(Spec §14.3 "A `PreToolUse` hook, injected with `--settings` (hooks still run in `-p` mode)".)*

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
    - **For milestone 9.** `metering::orchestrator_env(addr, run_id) -> Vec<(String, String)>` gives the variables the orchestrator window needs. Their exact names and values are fixed by M8b.1 (Interfaces lists the documented set). *(Spec §14.8 "The orchestrator, the one PTY session, is metered through Claude Code's OTLP export with `anthrex.run` and `anthrex.role` resource attributes".)*

### History

31. **Phase times.** `Task` gains `phase_since: u64` and `phases: PhaseSecs { queued, preparing, working, proof, check, review, merge, blocked }`. M8b introduces `run::phases::set_state(task: &mut Task, state: TaskState, now: u64)`. It adds `now - phase_since` to the field of the old state (`Pending` and the finished states count nowhere), sets `phase_since = now`, sets `max_rung = max(max_rung, rung)`, and assigns the state. Every assignment of `Task.state` in `run/engine/**` and `run/edits.rs` is replaced by it. The acceptance criterion greps that no other assignment remains. This extends spec §15's five phases with `preparing`, `proof` and `blocked`, because a task spends real time in each.
32. **Actual size by diff.** `OpKind::MeasureDiff { root, from, to, three_dot }` runs `git diff --numstat` and `git diff -U0` through `run_git` (reads, no queue), bounded by `git_timeout_secs`. It answers `OpResult::DiffMeasured(DiffStats { files, hunks, added, removed })`, where `hunks` counts the lines starting with `@@` and binary files count as files with 0 lines.
    - **A merged task:** `from` is the run head before its merge, `to` its merge commit, `three_dot = false`. That is exactly what the task contributed, and it stays correct after a hand-back.
    - **Cancelled or unfinished with commits:** `from` = the run head, `to` = the task head, `three_dot = true`.
    - **No commits:** no op, `diff = None`.

    Reconcile maps it to `NotStarted`. *(Spec §15 "Actual size is measured by diff, not tokens".)*
33. **`history.jsonl`.** One JSON line per record, at `<repo_dir>/history.jsonl` (decision 4).
    - **Types.** `proto::history::HistoryLine`, tagged `type`, with the variants `task`, `run` and `revert` (Interfaces). Every record carries `v: 1` and a `record_id`: `<run>/<task>` for a task, `<run>` for a run, `revert/<revert commit>` for a revert.
    - **As an op.** Appends are journaled ops: `OpKind::AppendHistory { path, record_id, line }` → `OpResult::HistoryAppended`. The driver appends one line and calls `sync_all`, on `spawn_blocking`.
    - **Reconcile.** It scans the file for `"record_id":"<id>"` and gives `Replay(HistoryAppended)` if it is found, `NotStarted` otherwise. Readers still keep the last line of each `record_id`, so a duplicate can never be counted twice.
    - **Task records.** A task record is appended when a task becomes `merged` (after `DiffMeasured`) or `cancelled`. When a run reaches `accepted`, `discarded` or `failed`, or completes with unfinished tasks, every task without a record gets one with outcome `unfinished`, `blocked` or `cancelled`. Then one `run` record is appended.
    - **Filled by the driver.** For a `run` record with outcome `accepted`, the driver fills in `accepted_commit` just before appending, by resolving `refs/heads/<base>` with `run::git::read_ref`. That is the one field the pure reducer cannot know.
    - **Task fields.** `Task.history_written: bool` is set when the op is emitted. *(Spec §15 list of fields.)*
34. **Revert detection.** `run::history_io::detect_reverts(git, root, base_branch, history_path, timeout)` runs on `spawn_blocking` at every `run start` (both kinds) and every `run stats`. It considers only `run` records with `accepted_commit`, and their tasks' `merge_commit`s, that are at most 90 days old and have no revert record yet. It reads `git log -n 2000 --format=%H%x1f%B%x1e <base_branch>` through `run_git`. Every commit whose message contains `This reverts commit <sha>` for one of those shas gets a `revert` record: `task_id = Some(..)` for a task's merge commit, and `None` for the run's accept merge, which means every task of that run. A failure only logs a warning. *(Spec §15 "whether the user later reverted it", §23 "Reverts after accept … are the only true signal".)*
35. **`anthrex run stats [--json]`** sends `RunRequest::Stats { dir }`.
    - **The rows.** The daemon reads the history (keeping the last line per `record_id`) and computes `run::stats::aggregate(&lines) -> HistoryStats`, which is pure. It has one row per class: `S`, `M` (non-hub), and `hub`.
    - **Each row:** the task count, merged count, median lines changed (`added + removed`), median tool calls, median billable tokens, median working minutes, total bounces, and reverted count.
    - **Medians** are taken over merged tasks only. With an even count, they are the lower middle value.
    - **The totals line:** decider calls and fallbacks, and how many tasks the size cross-check checked and raised.

    Nothing is proposed or refitted; that is M9.5. The text layout is in Interfaces. *(Spec §15; "recording only" for M8b.)*

### Test doubles

36. **`ANTHREX_DECIDER_BIN` and `fake-agent`'s decider mode.**
    - **Detection.** `fake-agent` is in decider mode when its argv has `--json-schema` (Claude shape) or `--output-schema` (Codex shape). This is checked before its other headless modes.
    - **Kind.** It reads the prompt: the stdin user message for Claude, the last argument for Codex. The kind is taken from the prompt's first line, `[anthrex decider] <kind> v1`.
    - **Scripts.** From `$FAKE_AGENT_DECIDER_DIR` it claims `<kind>-<n>.json` with the smallest `n` whose `.claimed` does not exist (`OpenOptions::create_new`, as in M8a.20). The file holds one of:
      - `{"answer": …}`: a structured answer in the fixture's form;
      - `{"text": "…"}`: raw assistant text;
      - `{"fail_turn": "…"}`: a failed turn;
      - `{"exit": <code>}`: exit before answering;
      - `{"hang": true}`: no output until killed.

      Any of them may also carry `"usage": {input, output, cache_read, cache_write}`.
    - **Records.** Every call appends `{"kind","argv","prompt"}` to `$FAKE_AGENT_DECIDER_DIR/calls.jsonl`.
    - **No script.** With no matching file, or no `FAKE_AGENT_DECIDER_DIR`, it writes `fake-agent: no scripted decider answer for <kind>` to stderr and exits 2. The caller then falls back.
    - **Shapes.** Its output uses only the shapes of M8b.1's decider fixtures, checked by M8a decision 51's shape test.
    - **Harness default.** `RunHarness` writes `[orchestrator.deciders] mode = "off"` and `[orchestrator.onboarding] auto = false` unless a test asks otherwise, so every M8a scenario runs exactly as before. *(Spec §21 "Deciders are replaced in tests by `ANTHREX_DECIDER_BIN`".)*
37. **Other `fake-agent` additions.**
    - **Scout scripts.** For role `scout`, the task part of a script name is the value after `--scout`, so the onboarding scout claims `scout-onboarding-<secs>-<n>.jsonl`. Because the id carries a timestamp, a script named with the prefix `scout-onboarding-<n>.jsonl` matches any onboarding id.
    - **The `bash {cmd}` step** simulates a Claude `Bash` tool call. It runs every `PreToolUse` matcher group from `--settings` whose matcher is empty or matches `Bash` as a regex, in order. Each group's command gets the payload `{"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":…},"session_id":…,"cwd":…}` on stdin, and the last `hookSpecificOutput.updatedInput.command` printed wins. The step then runs the final command with `/bin/sh -c` in the cwd. It emits the `tool_use` (with the final command) and `tool_result` pair, keeps the output as `FAKE_AGENT_RESULT`, and appends `{"original","ran","exit","output_lines"}` to `$FAKE_AGENT_BASH_LOG` when that is set.
    - **Hook discovery.** `runtime::discover` keeps every matcher group per event (`Runtime::groups(event)`). `Runtime::hook(event)` still returns the first group's first command, so milestone 3's behaviour is unchanged.

## Interfaces

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
pub struct RepoProfile {
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
#[derive(Eq)] pub struct ProfileVerification { pub at: u64, pub setup: Option<CommandCheck>,
                                               pub check: Option<CommandCheck>, pub single_test: Option<CommandCheck> }
#[derive(Eq)] pub struct DroppedCommand { pub key: String, pub command: String, pub reason: String, pub tail: String }
#[derive(Eq)] pub struct ProfileFindings { pub profile: RepoProfile, pub setup_ran_ok: Option<bool>,
                                           pub check_ran_ok: Option<bool>, pub single_test_ran_ok: Option<bool> }
#[serde(tag = "state")] #[derive(Eq)]
pub enum ProposalState { Preparing, Scouting, Verifying, Ready, Failed { reason: String } }
#[serde(tag = "origin")] #[derive(Eq)]
pub enum ProposalOrigin { Detect, Auto { stale: Vec<String> }, Goal, Edit { keys: Vec<String> } }
#[derive(Eq)] pub struct ProposalRecord {
    pub project: PathBuf, pub state: ProposalState, pub origin: ProposalOrigin,
    pub started_at: u64, pub updated_at: u64,
    pub scout_id: Option<String>, pub window_id: Option<u32>,
    pub profile: Option<RepoProfile>,                 // after verification: only commands that passed
    pub verification: Option<ProfileVerification>,
    pub dropped: Vec<DroppedCommand>,
    pub findings: Option<ProfileFindings>,            // the scout's raw proposal and claims
    pub trusted_project: Vec<String>,                 // M8a decision 53, when --trust-project was given
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
    pub proposal: Option<ProposalRecord>, pub scout: Option<ScoutInfo>, pub sandbox: bool,
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
    #[serde(default)] pub risks: Vec<String>, #[serde(default)] pub profile: Option<ProfileFindings>,
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
impl std::ops::AddAssign for TokenUsage;   // field-wise; added here if M8a has not
```

`crates/proto/src/run_info.rs` (M8a's), new fields, each `#[serde(default)]`:

```rust
// RunInfo
pub path: Option<RunPath>, pub triage: Option<TriageInfo>, pub promote_requested_at: Option<u64>,
pub profile_source: Option<ProfileSource>, pub usage: Option<RunUsage>, pub scouts: Vec<ScoutInfo>,
// TaskInfo
pub decider_usage: Option<TokenUsage>, pub size_check: Option<SizeCheckInfo>, pub diff: Option<DiffStats>,
pub phases: Option<PhaseSecs>, pub block_source: Option<DeciderSource>,
// CheckInfo
pub summary_source: Option<DeciderSource>,
```

`crates/proto/src/run_wire.rs` (M8a's), new variants and one field:

```rust
pub struct ToolCall { /* M8a's fields */ #[serde(default)] pub scout_id: Option<String> }
pub enum RunRequest { /* M8a's */
    StartGoal { goal: String, dir: PathBuf, yes: bool, trust_project: bool },
    Promote { run_id: String },
    Stats { dir: PathBuf },
    Profile(ProfileRequest),
}
pub enum ProfileRequest {
    Status { dir: PathBuf }, Detect { dir: PathBuf, trust_project: bool },
    Show { dir: PathBuf, proposed: bool }, Confirm { dir: PathBuf }, Reject { dir: PathBuf },
    Edit { dir: PathBuf, key: String, value: Option<String>, yes: bool },   // value None: --unset
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
pub mod request { /* M8a's */ pub const PROMOTE: &str = "run promote"; pub const STATS: &str = "run stats";
                  pub const PROFILE: &str = "profile"; }   // StartGoal answers with Triaged or M8a's START label
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

`lib.rs` re-exports every new public type and bumps `PROTO_VERSION` (header).

### `config` (`crates/config/src/orchestrator_adapt.rs`, new; one call from `orchestrator::read`)

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
pub(crate) fn read_adapt(table: &toml::Table, problems: &mut Vec<Problem>) -> (bool, Deciders, Scouts, Onboarding, Metering);
```

Messages follow M8a's format: `orchestrator.deciders.timeout_secs: must be between 5 and 600 (using 90)`, `orchestrator.deciders.mode: must be claude, codex or off (using claude)`, and `unknown key, ignored` for anything else under these tables.

### `daemon`

```rust
// profile/mod.rs
pub fn repo_dir(data_dir: &Path, project: &Path) -> PathBuf;     // decision 4
pub fn summary(profile: &RepoProfile) -> String;                  // text below
pub const ONBOARDING_WORKTREE: &str = ".onboarding";             // under <wt>/runs/
pub const VERIFY_WORKTREE: &str = ".profile-verify";

// profile/resolve.rs (pure)
pub struct ChosenProfile { pub spec: ProfileSpec, pub source: ProfileSource, pub output_filter: OutputFilter,
                           pub filter_prefixes: Vec<String>, pub notes: Vec<String> }
pub fn run_profile(stored: Option<&RepoProfile>, stored_path: &Path, plan: &ProfileSpec, config: &ProfileSpec) -> ChosenProfile;
pub fn derived_prefixes(profile: &RepoProfile) -> Vec<String>;  // decision 28

// profile/proposal.rs (pure)
pub fn validate(profile: &RepoProfile) -> Vec<String>;          // "<key>: <problem>" each; globs, {test}, regex, env keys
pub fn from_findings(findings: &ProfileFindings) -> RepoProfile; // the scout's profile, invalid keys removed
pub fn apply_verification(proposed: &RepoProfile, v: &ProfileVerification) -> (RepoProfile, Vec<DroppedCommand>);
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

// profile/verify.rs (blocking; the worktree writes go through GitQueue::write)
pub fn prepare_scratch(git: &OsStr, root: &Path, path: &Path, timeout: Duration) -> Result<(), String>;   // add --detach HEAD, lock
pub fn run_commands(dir: &Path, profile: &RepoProfile, timeout: Duration, now: u64) -> ProfileVerification;
pub fn discard_scratch(git: &OsStr, root: &Path, path: &Path, salvage_ref: &str, timeout: Duration) -> Result<Option<String>, String>;

// profile/service.rs
pub struct ProfileContext { pub data_dir: PathBuf, pub worktrees_root: PathBuf, pub git: OsString,
                            pub orchestrator: config::Orchestrator, pub git_queue: Arc<GitQueue> }
pub enum Effective { Stored { profile: RepoProfile, meta: ProfileMeta, path: PathBuf, stale: Vec<String> },
                     Unparseable { path: PathBuf, error: String },
                     Absent { proposal: Option<ProposalState> } }
pub struct ProfileService { /* per-project proposal table under daemon::lock, the scout service, the context */ }
impl ProfileService {
    pub fn new(scouts: Arc<ScoutService>, ctx: ProfileContext) -> Arc<Self>;
    pub async fn restore(self: &Arc<Self>);                                     // decision 11
    pub async fn effective(&self, project: &Path) -> Effective;
    pub async fn request(self: &Arc<Self>, request: ProfileRequest) -> ProfileReply;
    pub async fn start_detection(self: &Arc<Self>, root: &Path, project: &Path, origin: ProposalOrigin,
                                 trust_project: bool) -> Result<(), String>;    // Err: the refusal text
}

// scout/contract.rs (pure): SCOUT_CONTRACT, ONBOARDING_CONTRACT, SCOUT_NUDGE (texts below)
pub fn onboarding_first_turn(project: &Path, cwd: &Path, tracked: usize, top_level: &[String]) -> String;
pub fn scout_wrap_up(tool_calls: u32) -> String;

// scout/spec.rs (pure)
pub struct ScoutSpec { pub id: String, pub kind: ScoutKind, pub run_id: Option<String>, pub question: String,
                       pub first_turn: String, pub cwd: PathBuf, pub project: PathBuf, pub web: bool }
pub struct ScoutContext { pub exe: PathBuf, pub socket_path: PathBuf, pub roster: Vec<ModelEntry>,
                          pub default_runtime: Runtime, pub scouts: config::Scouts, pub claude: config::ClaudeHeadless,
                          pub worker_sandbox: bool, pub data_dir: PathBuf }
pub fn route(roster: &[ModelEntry], runtime: Runtime, strength: Strength, effort: Effort) -> Route;
pub fn headless_spec(scout: &ScoutSpec, ctx: &ScoutContext) -> HeadlessSpec;   // decision 12
pub fn valid_id(id: &str) -> bool;                                              // ^[a-z0-9][a-z0-9-]{0,47}$

// scout/report.rs (pure)
pub struct ScoutReportArgs { pub summary: String, pub files: Vec<ScoutFile>, pub modules: Vec<String>,
                             pub interfaces: Vec<String>, pub risks: Vec<String>, pub profile: Option<ProfileFindings> }
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

// decider/mod.rs (pure types; serialized in OpKind::Decide)
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
                            pub cwd: PathBuf, pub schema_dir: PathBuf, pub claude_auth: ClaudeAuth,
                            pub api_key_helper: Option<String>, pub caps: &'static CliCaps }
impl DeciderContext {
    pub fn new(cfg: &config::Orchestrator, claude_bin: &str, codex_bin: &str, decider_bin: Option<&Path>,
               data_dir: &Path, roster: &[ModelEntry]) -> Self;
}
// decider/schema.rs, prompt.rs, parse.rs, fallback.rs, argv.rs (pure)
pub fn schema(kind: DeciderKind) -> serde_json::Value;
pub fn render(request: &DeciderRequest) -> String;               // PROMPT_MAX_BYTES = 128 KiB
pub fn parse(kind: DeciderKind, value: &serde_json::Value) -> Result<DeciderAnswer, String>;
pub fn answer_from_events(events: &[SessionEvent]) -> Result<serde_json::Value, String>;   // decision 16's order
pub fn fallback(request: &DeciderRequest) -> DeciderAnswer;
pub fn fallback_decision(request: &DeciderRequest, reason: String) -> Decision;
pub fn claude_decider_args(ctx: &DeciderContext, schema: &serde_json::Value) -> Vec<String>;
pub fn codex_decider_args(ctx: &DeciderContext, schema_file: &Path, prompt: &str) -> Vec<String>;
pub fn schema_file_name(kind: DeciderKind, schema: &serde_json::Value) -> String;   // "<kind>-<16 hex>.json"
// decider/call.rs (I/O)
pub async fn decide(ctx: &DeciderContext, request: &DeciderRequest) -> Decision;
pub const ANSWER_MAX_BYTES: usize = 256 * 1024;

// output_filter.rs (pure)
pub const FAILURE_RE: &str = r"(?i)\b(fail(ed|ure|ures|s)?|error(s)?|panic(ked|s)?|assert(ion)?|expected|traceback|exception)\b";
pub const LINE_MAX_CHARS: usize = 500;
pub const HOOK_SETS_ALLOW: bool;                                  // from M8b.1
#[derive(Serialize, Deserialize)] pub struct FilterHook { pub mode: OutputFilter, pub prefixes: Vec<String>, pub log_dir: PathBuf }
pub fn apply(mode: OutputFilter, lines: &[String], exit_code: i32) -> Vec<String>;
pub fn matches(command: &str, prefixes: &[String]) -> bool;
pub fn wrap(exe: &Path, hook: &FilterHook, command: &str) -> String;
pub fn rewrite(payload: &serde_json::Value, exe: &Path, hook: &FilterHook) -> Option<serde_json::Value>;
pub fn hook_command(exe: &Path, hook: &FilterHook) -> String;     // the settings "command" string, shell-quoted
pub fn add_hook(settings: &mut serde_json::Value, exe: &Path, hook: Option<&FilterHook>);
// headless::HeadlessSpec gains  #[serde(default)] pub output_filter: Option<FilterHook>
// headless::McpTarget gains     #[serde(default)] pub scout_id: Option<String>
// headless::SessionEvent gains  StructuredOutput { value: serde_json::Value }   only if M8b.1 finds result.structured_output

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
// run/model.rs additions
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
// CheckRecord gains: summary: Option<String>, summary_source: Option<DeciderSource>
// RunLimits gains: decider_mode: DeciderMode, decider_slot_wait_secs: u64
// run/engine/deciders.rs (pure)
pub fn queue(run: &mut Run, task_ids: Vec<String>, request: DeciderRequest, now: u64) -> u64;   // decider id
pub fn dispatch(run: &mut Run, now: u64) -> Vec<Effect>;           // start queued deciders into free reader slots; slot-wait fallbacks
pub fn on_decided(run: &mut Run, decider_id: u64, decision: Decision, now: u64) -> Vec<Effect>;
pub fn apply_raise(run: &mut Run, task_id: &str, size: Size, reason: &str);   // decision 19

// RunService (additions)
impl RunService { pub fn orchestrator_usage(&self, run_id: String, usage: TokenUsage); }   // sends EventKind::OrchestratorUsage
// RunContext gains: decider_bin: Option<PathBuf>, claude_bin: String, codex_bin: String
```

**Reconcile rows** added to M8a's table (`run/reconcile.rs`):

| `OpKind` | Reality checked | Replay | Otherwise |
|----------|-----------------|--------|-----------|
| `Decide`, `MeasureDiff` | none | — | `NotStarted` |
| `AppendHistory` | `contains_record(path, record_id)` | found → `HistoryAppended` | `NotStarted` |

### MCP (`crates/mcp`, M8a's)

`McpOptions` gains `scout_id: Option<String>`. `tools_for(AgentRole::Scout)` returns one tool. The schema is a closed object at every level (`additionalProperties: false`):

| Role | Tool | Description | Properties (required in bold) |
|------|------|-------------|-------------------------------|
| scout | `submit_scout_report` | `Submit your findings. Call it once, then stop.` | **`summary`** string 1–8000; **`files`** array ≤ 60 of objects {**`path`** string 1–500, **`why`** string 1–300}; `modules` array ≤ 40 of string 1–200; `interfaces` array ≤ 40 of string 1–500; `risks` array ≤ 20 of string 1–500; `profile` object {`languages` array ≤ 10 of string 1–40; `modules`, `hub`, `source`, `generated`, `protected` arrays ≤ 40 of string 1–300; `setup`, `check` string 1–2000; `check_timeout_secs` integer 10–14400; `single_test` string 1–1000; `test_passed`, `sample_test` string 1–300; `output_filter` enum `failures-only`, `tail`, `none`; `filter_prefixes` array ≤ 10 of string 1–100; `conventions` array ≤ 20 and `manifests` array ≤ 50 of string 1–300; `env` object ≤ 20 properties matching `^[A-Za-z_][A-Za-z0-9_]*$` with string values 0–1000; `setup_ran_ok`, `check_ran_ok`, `single_test_ran_ok` boolean} |

Engine-side texts (`ToolResult`):
- **Refusals:** `unknown scout <id>`; `this window is not scout <id>`; `scout <id> is <state>`; `a report for scout <id> was already recorded`; `invalid arguments: <field>: <problem>`; `invalid arguments: profile: required for the onboarding scout`; `invalid arguments: profile: only the onboarding scout reports a profile`.
- **Success:** `Report recorded. You are done; end your turn now.`

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
A check command failed in a coding task's worktree. Summarise the failure for the agent who must fix it, in at most 40 lines. Keep failing test names, error messages, file:line locations and assertion values exactly as they appear. Leave out passing tests, progress output and anything repeated. Answer with one JSON object that matches the schema, and nothing else.

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
2. Do not edit, create or delete files, and do not commit.
3. Call the anthrex tool submit_scout_report exactly once: a summary of at most about 2000 tokens, the files that matter with one line each on why, and the modules, interfaces and risks you found. Then stop.
4. Report what you found, not what you guess. Say in the summary what you could not determine.

ONBOARDING_CONTRACT:
You are the onboarding scout of anthrex, a tool that runs coding agents on this repository. You work out how it is set up, built and tested. You never change the repository.
1. This directory is a disposable copy of the repository at its current commit. It is thrown away when you finish, and the user's checkout is never touched. Your commands run in a sandbox that can write only here.
2. Read manifests, lock files, CI configuration, READMEs and agent instruction files (AGENTS.md, CLAUDE.md and similar) before source code.
3. Find: the languages; what counts as one module (globs); hub paths that many modules depend on; where behaviour lives (source globs); generated files that builds rewrite on their own, taken from the lock files you find (for example Cargo.lock, package-lock.json, yarn.lock, pnpm-lock.yaml, poetry.lock, uv.lock, go.sum); protected files that configure or instruct coding agents: always .claude/**, .mcp.json, .codex/**, **/CLAUDE.md and **/AGENTS.md, plus any other agent configuration, hook, MCP server or instruction file you find (for example .cursor/**, .github/copilot-instructions.md, GEMINI.md); a setup command to run once in a fresh copy; one check command that builds, tests and lints everything CI checks; a command that runs one named test, with {test} where the name goes; a regular expression, with {test} where the name goes, that matches a line of that command's output only when that test ran and passed; the name of one existing test that passes; the manifests and convention files you relied on; environment variables every copy needs, with {worktree} for the copy's path.
4. Run the setup, the check and the single-test command with your sample test yourself, and set setup_ran_ok, check_ran_ok and single_test_ran_ok to what happened. If the sandbox stopped a command, for example because it needs the network, propose it anyway with its ran_ok false: anthrex runs every command again outside the sandbox and proposes only the ones that pass.
5. Do not edit files, commit, or install anything outside this directory.
6. Call the anthrex tool submit_scout_report exactly once, with a short summary, the files that matter, and the profile. Then stop.
```

| Name | Text |
|------|------|
| `onboarding_first_turn` | `[anthrex] Work out this repository's profile. Repository: <project>. Disposable copy: <cwd>. It tracks <n> files; the top-level entries are: <names, comma-separated, at most 60>.` |
| `SCOUT_NUDGE` | `[anthrex] Your turn ended without a report. Call submit_scout_report now with what you found, then stop.` |
| `scout_wrap_up` | `[anthrex] You have used <n> tool calls. Stop exploring and call submit_scout_report now with what you found.` |
| `profile::summary` | Lines `languages: <a, b>`, `modules: <globs>`, `hub: <globs>`, `source: <globs>`, `generated: <globs>`, `protected: built-in <5 globs> + <extras or "no extras">` (always printed), `setup: <command or none>`, `check: <command or "none (runs are unverified)">`, `single test: <command or "none (tdd is impossible; code tasks use check)">`, each list line omitted when empty. |
| `started_message` | `triage: <kinds joined with ,>/<scale> (<decider \| fallback: <reason>>)` / `fast path: one task, no plan gate` / `  t1  <size>  <mode>  <title>` / `watch with: anthrex run status <id>` |
| `refused_message` | `triage: <kinds>/<scale> (<decider \| fallback: <reason>>): <reason>` / `this goal needs a planned run, which arrives with the orchestrator (milestone 9). Write a plan file and run: anthrex run start --plan <file>` |

### CLI

```
anthrex run start (--plan <file> | --goal <text>) [--yes] [--trust-project]
anthrex run promote <run>
anthrex run stats [--json]
anthrex profile status [--json]
anthrex profile detect [--trust-project]
anthrex profile show [--proposed] [--json]
anthrex profile confirm [--yes]
anthrex profile reject
anthrex profile edit <key> <value> [--yes]
anthrex profile edit --unset <key> [--yes]
anthrex mcp --role scout --scout <id> [--run <run>] --window <id> [--socket <path>]      (hidden)
anthrex filter-run --mode <failures-only|tail|none> --log-dir <dir> -c <command>          (hidden, before clap)
anthrex filter-hook --mode <failures-only|tail|none> --log-dir <dir> [--prefix <p>]...   (hidden, before clap)
```

- **`run start`.** `--plan` and `--goal` are a required, mutually exclusive clap group.
  - `--goal` uses `GOAL_REQUEST_TIMEOUT` (decision 22).
  - On the fast path it prints the run id on stdout and `started_message` on stderr, and exits 0.
  - On a refusal it prints the message on stderr and exits 1.
- **`run promote`** exits 0 on `Done` and 1 on `Refused`.
- **`run stats`** prints `stats::render` or, with `--json`, `HistoryStats`. `--dir` picks the repository:

  ```
  history: <path>  (<n> task records, <m> runs)
  CLASS  TASKS  MERGED  LINES  TOOL CALLS  TOKENS  WORK MIN  BOUNCES  REVERTED
  S      20     19      14     22          180k    6         3        0
  M      15     13      71     96          1.1M    31        7        1
  hub    2      2       64     120         1.4M    40        1        0
  deciders: 42 calls, 3 fallbacks · size cross-check: 30 checked, 2 raised
  ```

  Every numeric column except `TASKS`, `MERGED`, `BOUNCES` and `REVERTED` is a median, and `-` when there are no merged tasks. Tokens are shown as `<n>`, `<n>k` (one decimal under 10k) or `<n>.<d>M`. A problem line `history: <n> lines skipped: <first problem>` follows when any line was skipped.
- **`profile`** requests use `RUN_REQUEST_TIMEOUT`.
  - `status` prints:

    ```
    profile: <project>
      stored: yes, confirmed <yyyy-mm-dd hh:mm> (<repo_dir>/profile.toml)      | stored: no
      stale: <paths> changed since it was confirmed                              (only when stale)
      detection: <state> since <hh:mm> (scout <id>, window <n>)                  | detection: failed: <reason> | detection: none
    ```

  - `show` prints `show_text`, which is the TOML, then a comment block:

    ```
    # verification <yyyy-mm-dd hh:mm>
    #   setup        ok     3s   cargo fetch
    #   check        ok   214s   cargo build --workspace …
    #   single_test  ok    12s   cargo test --workspace -- --exact {test}  (sample: store::tests::round_trip)
    # dropped
    #   check: exit 101 after 30s: cargo test --all
    #     <each of the last 40 lines>
    ```

  - `confirm` prints the same, then asks.
  - `edit` never waits for verification. It prints one of:
    - `proposed: <key> = <value>; verifying (anthrex profile status), then confirm with anthrex profile confirm`;
    - `proposed: <key> = <value>; confirm with anthrex profile confirm`, when nothing needs re-running;
    - with `--yes`, `proposed: <key> = <value>; it is stored as soon as verification passes (anthrex profile status)`. The daemon confirms it (`ProposalRecord.auto_confirm`).
- **`run status`** (M8a's) adds ` fast path` after the state for a fast-path run, and a line `  triage: <kinds>/<scale> (<source>)` under `goal:`.

### File sizes this milestone must respect

AGENTS.md rule 8 puts the limit at about 600 lines. At the start, run `wc -l` on every file below and record the counts under "Implementation notes". The budgets are growth over those counts.

| File | Budget | Note |
|------|-------:|------|
| `crates/config/src/lib.rs` | 0 | Already over (1033 after M6.5). Everything goes in `orchestrator_adapt.rs`. |
| `crates/config/src/orchestrator.rs` (M8a) | +12 | Five fields and one `read_adapt` call. |
| `crates/proto/src/run.rs`, `run_info.rs`, `run_wire.rs` (M8a) | +2, +20, +45 | New variants and fields only; new types live in `profile.rs`, `scout.rs`, `adapt.rs`, `history.rs`. |
| `crates/daemon/src/run/model.rs` (M8a) | +45 | New structs go in `run/model_adapt.rs`, which `model.rs` re-exports. |
| `run/engine/{gates,merge,done,requests,dispatch}.rs` (M8a) | +40 each | Decider logic is `engine/deciders.rs`; these files call into it. |
| `run/snapshot.rs`, `run/report.rs`, `run/reconcile.rs` (M8a) | +50, +40, +25 | |
| `run/driver.rs`, `run/driver/ops.rs` (M8a) | +20, +40 | New op execution and `StartGoal` go in `run/driver/adapt.rs`. |
| `crates/daemon/src/headless/argv.rs`, `claude_stream.rs` (M8a) | +15, +15 | `add_hook` call, `--scout`, `StructuredOutput`. |
| `crates/daemon/src/server/run_api.rs` (M8a) | +35 | Routing only. |
| `crates/daemon/src/lifecycle.rs` (419 + M8a) | +30 | Services, OTLP bind, restore order. |
| `crates/mcp/src/tools.rs` (M8a) | +20 | The scout schema goes in `tools_scout.rs`. |
| `crates/cli/src/main.rs` (527 + M8a's ≤ 25) | +20 | `Profile` variant, two pre-clap dispatches; bodies in `profile_cmd.rs`, `filter_run.rs`, `filter_hook.rs`. |
| `crates/cli/src/run_cmd.rs` (M8a) | +25 | `--goal`, `promote`, `stats` bodies in `run_cmd/adapt.rs`. |
| `crates/fake-agent/src/runtime.rs` (195), `script.rs` (213) | +40, +20 | Decider mode in `decider.rs`; the `bash` step in `bash.rs`. |
| `crates/cli/tests/support/run_harness.rs` (M8a) | +50 | |
| `scripts/pty-smoke.py` (1572) | +5 | The stage lives in `scripts/pty_smoke_adapt.py`. |

No new file may exceed 600 lines.

## Produces for later milestones

Exact names milestones 8c, 9 and 9.5 consume. Renaming any of them later is a cross-milestone change and must update those briefs.

| Consumer | Name | What it is |
|----------|------|------------|
| M8c | `RunInfo.path: Option<RunPath>` (`proto::RunPath { Fast, Plan, Large }`) | A fast-path run's root node is the run itself (spec §16.1). |
| M8c | `RunInfo.triage: Option<TriageInfo>`, `RunInfo.promote_requested_at: Option<u64>`, `RunInfo.profile_source: Option<ProfileSource>` | Inspector fields. |
| M8c | `RunInfo.usage: Option<RunUsage { total, by_role, decider_calls, decider_fallbacks }>` | The orchestrator inspector's `spend` line; `by_role` keys `worker`, `reviewer`, `scout`, `decider`, `orchestrator`. |
| M8c | `RunInfo.scouts: Vec<ScoutInfo>` (`proto::ScoutInfo`, `ScoutState`, `ScoutKind`) | Scout nodes and the scout inspector (spec §16.3–§16.4); empty until M9 spawns run scouts. |
| M8c | `TaskInfo.decider_usage`, `TaskInfo.size_check: Option<SizeCheckInfo>`, `TaskInfo.diff: Option<DiffStats>`, `TaskInfo.phases: Option<PhaseSecs>`, `TaskInfo.block_source`, `CheckInfo.summary_source` | Task inspector fields. Round usage is M8a's `AgentRoundInfo.usage`. |
| M8c | `proto::AgentRole::Scout` | A scout's headless window carries it in `WindowInfo.run` for run scouts. |
| M8a (at M8b's start) | `RepoProfile.protected` (extras only) → `ProfileSpec.protected` through `profile::resolve::run_profile` | The stored extras that M8a unions with `BUILTIN_PROTECTED` and enforces (decision 56). |
| M9 | `proto::RepoProfile`, `profile::repo_dir(data_dir, project)`, `<repo_dir>/profile.toml`, `ProfileService::effective(project) -> Effective`, `profile::summary(&RepoProfile)` | The profile for `get_context` and plan validation. |
| M9 | `proto::ScoutReport`, `ScoutFile`, `ScoutKind::Area`; run scout reports at `<data_dir>/runs/<run>/scouts/<id>.json`, repository ones at `<repo_dir>/scouts/<id>.json`; `scout::report::{report_path, resolve_ref}`; the `onboarding` alias in `scout_refs` | Scout output for planners and worker briefs. |
| M9 | `ScoutService::{start, tool, stop, info, run_scouts}`, `ScoutSpec`, `ScoutContext`, `ScoutOutcome`, `SCOUT_CONTRACT`, `scout::spec::{route, headless_spec}`, `Run.scout_reports`, `Run.scout_usage` | `spawn_scout` builds on these. It must push the report id into `Run.scout_reports` so the size cross-check sees it. |
| M9 | `decider::call::decide(&DeciderContext, &DeciderRequest) -> Decision`, `DeciderRequest`, `DeciderAnswer`, `Decision`, `DeciderKind`, `decider::fallback::fallback_decision`, `OpKind::Decide`, `run::engine::deciders::{queue, on_decided}` | The decider interface; new kinds are new variants. |
| M9 | `RunRequest::StartGoal`, `run::triage::{route, TriageRoute}` | M9 replaces the `Plan` and `Large` refusal with the orchestrator path. |
| M9 | `Run.promote_requested_at` | M9 performs the promotion the user asked for. |
| M9 | `metering::orchestrator_env(addr, run_id)`, `<data_dir>/otlp.addr`, `EventKind::OrchestratorUsage` | Metering the orchestrator window. |
| M9 | `headless::McpTarget.scout_id`, `ToolCall.scout_id`, `anthrex mcp --role scout --scout` | Scout MCP plumbing. |
| M9.5 | `proto::history::{HistoryLine, TaskRecord, RunRecord, RevertRecord, HistoryStats, StatsRow, HISTORY_VERSION}`, `<repo_dir>/history.jsonl` | The record type and path the refit reads. |
| M9.5 | `run::history_io::read_history(path)`, `run::stats::{aggregate, render}`, `anthrex run stats` | M9.5 adds proposals to `stats`. |
| M9.5 | `TaskRecord.phases`, `TaskRecord.diff`, `TaskRecord.max_rung`, `RunInfo.rate_limits` (M8a) | The inputs of threshold and budget refits and adaptive concurrency. |

## Tasks

Shared test conventions:

- **Reducer tests** use M8a's `engine/tests/fixture.rs`. M8b adds `fx.decided(decider_id, Decision)` and `fx.queued_deciders()`.
- **Process tests** that need `fake-agent` live in `crates/cli/tests/`, use `support::fake_agent_bin()`, and call daemon library code directly. They reuse `crates/daemon/tests/support/mod.rs`'s `TempRepo` through a `#[path]` module include.
- **End-to-end tests** use M8a's `RunHarness`. M8b adds these methods:
  - `with_deciders(mode)`, which writes `[orchestrator.deciders] mode`, `timeout_secs = 5` and `slot_wait_secs = 2`, and sets `ANTHREX_DECIDER_BIN = fake_agent_bin()` and `FAKE_AGENT_DECIDER_DIR = <tmp>/deciders`;
  - `decider(kind, n, value)`, which writes `<tmp>/deciders/<kind>-<n>.json`;
  - `decider_calls() -> Vec<Value>`;
  - `stored_profile(toml)`, which writes `<repo_dir>/profile.toml` and a `profile.meta.json` with the fingerprint of the files the TOML names, as `anthrex profile confirm` would;
  - `onboarding_report(json)`.
- **`PROFILE_WAIT` = 150 s.** It is derived from the test configuration's bounds: `scouts.timeout_secs = 60`, plus three verification commands at `onboarding.verify_timeout_secs = 10` (30 s), plus at most 8 engine git calls at `git_timeout_secs = 5` (40 s). That is 130 s. Add the row to `docs/timing-budgets.md`, with the decider rows of M8b.7.
- **No test sleeps to synchronise.** Every wait is a deadline loop.

### Scenario map (spec §21)

| §21 scenario | Test here |
|--------------|-----------|
| A green S task on the fast path | `e2e_green_s_task_on_the_fast_path` (M8b.15) |
| *(M8b)* a decider that fails never blocks a run | `e2e_check_bounce_falls_back_to_the_tail_when_the_decider_fails`, `a_hanging_decider_falls_back_at_its_timeout_and_leaves_no_process` |
| *(M8b)* the profile is proposed only from commands that ran | `e2e_detect_proposes_only_verified_commands` |

Every other §21 scenario is M8a's, and M8b must keep all of them green, with deciders off in the harness (decision 36).

### M8b.1 Verify the external facts and record the fixtures

**Files.** Create:
- `crates/daemon/tests/fixtures/deciders/claude-<version>-decider.jsonl` and `codex-<version>-decider.jsonl`;
- `crates/daemon/tests/fixtures/otlp/claude-<version>-metrics.json`;
- `crates/daemon/tests/fixtures/headless/claude-<version>-filter-hook.jsonl`;
- a `.meta.json` beside each: CLI version, date, exact command, and what was observed versus only documented.

Fill "Implementation notes".

**Tests first.** None. This task records facts. It is the only task that runs the real `claude` and `codex`, in `/tmp/anthrex-m8b1/`, on the implementer's own login. Personal paths become `/tmp/fixture`.

**Change.** Record the command, the version and the relevant output for each item below:

1. **A Claude decider call.** Pipe one stream-json user message into `claude -p --input-format stream-json --output-format stream-json [--verbose] <CLI_CAPS.claude_user_settings_only> --permission-prompts none --permission-mode plan --disallowedTools Bash,Edit,Write,NotebookEdit,Agent,WebFetch,WebSearch --max-turns 3 --json-schema '<the blocked_reason schema>' --no-session-persistence --model claude-haiku-4-5`, then close stdin. Record:
   - which flags exist;
   - where the JSON answer appears: `result.structured_output`, a `StructuredOutput` tool use, or assistant text. This sets `DECIDER_CAPS.answer_source` and whether `SessionEvent::StructuredOutput` is added;
   - whether a schema without every property in `required` is refused (`DECIDER_CAPS.strict_schemas`);
   - the result's `usage`;
   - whether the process exits after `result`.
2. **A Codex decider call.** `codex exec --json --skip-git-repo-check --ephemeral <CLI_CAPS.codex_user_config_only> -s read-only -c approval_policy="never" -c model_reasoning_effort="low" --output-schema <file> -- "<the blocked_reason prompt>"`. Record which flags exist, that the final `agent_message` text is the JSON, and `turn.completed.usage`.
3. **The filter hook under `-p`.** Use a scratch repository and decision 28's `--settings` with two `PreToolUse` groups: M3's recording command, and a `Bash` group whose script prints `updatedInput` with the command `echo rewritten`. Add `--allowedTools Bash` and `--permission-prompts none`, M8a decision 54's sandbox block, and a `log_dir` writable root. Ask for `echo original`. Record:
   - whether both groups fired;
   - whether the rewritten command ran (its tool result);
   - whether `permissionDecision: "allow"` was needed (`output_filter::HOOK_SETS_ALLOW`);
   - the payload's `tool_input` keys;
   - whether a write into `log_dir` from inside the sandboxed command succeeds.
4. **Scout launch.** Reuse M8a.1's reviewer finding for `--permission-mode plan` with an allowed MCP tool: it applies to area scouts. For the onboarding scout, run `--permission-mode default --allowedTools Bash,Read` with the sandbox block in a detached worktree. Record that `Bash` runs, and that a write outside the worktree is denied.
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

Also cross-reference M8a.1's record of stream usage field names (spec §14.8 asks the metering milestone to verify them, and M8a meters first). Add them here only if M8a.1 did not record them.

In `decider/argv.rs`, add `pub struct DeciderCaps { pub claude_json_schema: bool, pub claude_max_turns: bool, pub claude_no_session_persistence: bool, pub answer_source: AnswerSource, pub strict_schemas: bool, pub codex_output_schema: bool, pub codex_ephemeral: bool }`, `pub enum AnswerSource { ResultField, StructuredOutputTool, Text }` and `pub const DECIDER_CAPS: DeciderCaps`, set from items 1–2. A flag that does not exist is omitted from the argv.

A missing capability stops work on the item that needs it, as AGENTS.md says:
- `--json-schema` or `--output-schema` absent: that runtime's deciders rely on the prompt alone and `parse`, which still validates everything;
- the filter hook ignored: record it and skip the injection in M8b.8.

**Acceptance.** "Implementation notes" has a dated "M8b.1 external facts" entry covering all five items and every `DECIDER_CAPS` value. The four fixtures and their meta files exist.

**Commit.** `test(daemon): record the decider, filter-hook and OTLP fixtures and the verified CLI facts`

### M8b.2 Protocol: scout role, profile, scout, adaptation and history types, and the version bump

**Files.** Create `crates/proto/src/profile.rs`, `scout.rs`, `adapt.rs`, `history.rs`, `adapt_tests.rs`. Modify `crates/proto/src/lib.rs`, `run.rs`, `run_info.rs`, `run_wire.rs`, and every exhaustive `match` on `AgentRole`, `RunRequest` or `RunReply` in the workspace (the daemon answers the new requests with `Refused { request, message: "not available yet" }` until their tasks land).

**Tests first**, in `adapt_tests.rs`:
- `agent_role_scout_serializes_as_scout` (`"scout"`, under M8a's `snake_case`).
- `repo_profile_parses_the_spec_example`: spec §6's TOML block, plus `sample_test`, `manifests` and `filter_prefixes`, parses. `generated == ["Cargo.lock"]`, `output_filter == FailuresOnly`, and `env["CARGO_TARGET_DIR"] == "{worktree}/target"`.
- `repo_profile_rejects_unknown_keys`: `lint = "x"` fails, naming `lint`.
- `repo_profile_protected_defaults_to_no_extras`: a profile without `protected` parses to `protected == []`, and `spec().protected == None`.
- `repo_profile_spec_maps_every_shared_key`: `spec()` carries `modules`, `hub`, `source`, `generated`, `setup`, `check`, `check_timeout_secs`, `single_test`, `test_passed` and `env`, and an empty list gives `None`.
- `output_filter_kebab_case` (`"failures-only"`).
- `scout_report_round_trips_json_and_msgpack`, with a `ProfileFindings`.
- `history_lines_are_tagged`: a `TaskRecord` serialises with `"type":"task"`, and a line with an extra unknown field still deserialises.
- `new_snapshot_fields_default_when_absent`: M8a's `RunInfo` and `TaskInfo` JSON without the new fields gives `None` and empty values.
- `every_new_request_and_reply_round_trips`: `StartGoal`, `Promote`, `Stats`, each `ProfileRequest`, `Triaged`, each `ProfileReply` and `Stats(HistoryStats)`, wrapped in `ClientMsg::Run` / `DaemonMsg::Run`, survive `rmp_serde::to_vec_named`.
- `tool_call_scout_id_defaults_to_none`.
- Update `proto_version_is_*`.

**Change.** Add the Interfaces types.

**Acceptance.** Tests pass. The workspace builds. `PROTO_VERSION` equals the header's derivation, recorded in "Implementation notes".

**Commit.** `feat(proto): add the scout role and the profile, scout, triage, usage and history types`

### M8b.3 Configuration

**Files.** Create `crates/config/src/orchestrator_adapt.rs` and `orchestrator_adapt_tests.rs`. Modify `crates/config/src/orchestrator.rs`.

**Tests first:**
- `adapt_defaults_when_absent`: every default in Interfaces.
- `adapt_keys_are_read`.
- `adapt_out_of_range_values_fall_back`: `timeout_secs = 4`, `timeout_secs = 601`, `slot_wait_secs = 601`, `scouts.timeout_secs = 59`, `max_tool_calls = 9`, `verify_timeout_secs = 9`, `otlp_port = 70000`. Each gives one problem with the exact message and keeps the default.
- `decider_mode_values`: `claude`, `codex` and `off` are accepted; `gpt` is a problem.
- `scouts_runtime_shell_is_a_problem`.
- `unknown_adapt_keys_are_reported` (`orchestrator.deciders.model` gives `unknown key, ignored`).
- `fast_path_false_is_read`.

**Change.** Decision 3.

**Acceptance.** Tests pass. `crates/config/src/lib.rs` is unchanged (`git diff --stat` shows nothing for it).

**Commit.** `feat(config): add the deciders, scouts, onboarding, metering and fast_path settings`

### M8b.4 Profile store and precedence

**Files.** Create `crates/daemon/src/profile/mod.rs`, `profile/resolve.rs`, `profile/proposal.rs` (types and `validate` only), `profile/store.rs`, `crates/daemon/tests/profile_store.rs`. Modify `crates/daemon/src/lib.rs` (`pub mod profile;`), `run/driver.rs` or `run/driver/adapt.rs` (decision 6 in `request(Start)`), and `run/model.rs` (`Run.profile_source`, `output_filter`, `filter_prefixes`, `repo_dir`, `stale_profile`).

**Tests first:**
- In `resolve.rs`:
  - `stored_profile_replaces_the_plan_profile_entirely`. Stored sets `check = "a"` only; the plan sets `check = "b"` and `single_test = "t {test}"`. The chosen spec has `check == Some("a")` and `single_test == None`, source `Stored`, and two notes with the exact text naming `check` and `single_test`.
  - `without_a_stored_profile_m8a_rule_holds`: the plan's `check` over the config's `single_test`, source `Plan`.
  - `nothing_anywhere_is_source_none`.
  - `filter_fields_only_come_from_a_stored_profile`.
  - `builtins_always_apply_whatever_the_stored_list`. The stored `protected` is `[]` in one case and `[".cursor/**"]` in another, and a plan `[profile] protected = ["docs/agents/**"]` is present. Run the chosen spec through M8a's profile resolution (`build_run`'s resolved `Profile.protected`). In both cases the result contains every entry of `BUILTIN_PROTECTED`, plus `.cursor/**` when stored, plus `docs/agents/**`. There is no "ignored" note for `protected`.
  - `derived_prefixes_from_check_and_single_test`: check `cargo build --all && cargo test -q; cargo fmt --check` and `single_test` `cargo test -- --exact {test}` give `["cargo build", "cargo test", "cargo fmt"]`.
- In `proposal.rs`: `from_findings_keeps_only_extra_protected_entries`. A scout reporting `[".claude/**", "**/AGENTS.md", ".cursor/**", ".cursor/**"]` gives `[".cursor/**"]`.
- In `proposal.rs`: `edit_of_protected_refuses_builtins`. `edit protected '[".mcp.json"]'` is refused with the exact message. `edit protected '[".cursor/**", "GEMINI.md"]'` stores exactly those two, and `--unset protected` leaves no extras, with every built-in still in force (checked through `run_profile` plus M8a's resolution).
- In `proposal.rs`: `validate_reports_each_bad_key` (a glob with `..`, an absolute `protected` entry, `single_test` without `{test}`, `test_passed` `(` that does not compile, env key `1X`, `generated` absolute), each with its exact message.
- In `profile_store.rs`, on a `TempRepo`:
  - `repo_dir_is_shared_by_linked_worktrees` (the main checkout and a linked worktree give one `repo_dir`, `<data>/repos/<basename>-<hash8>`).
  - `save_and_load_round_trip_atomically`: a leftover `profile.toml.tmp` is ignored and removed.
  - `an_unparseable_profile_is_reported_with_its_path`.
  - `fingerprint_changes_with_content_length_and_absence`.
  - `stale_lists_exactly_the_changed_files`.
  - `nothing_is_written_inside_the_repository`: after save, `git status --porcelain --ignored` in the repo is empty.
- The decision 6 driver path is tested end to end in M8b.15 (`e2e_stored_profile_wins_over_the_plan_profile`, `e2e_a_corrupt_stored_profile_refuses_the_run`).

**Change.** Decisions 4–7 (the pure and store parts), and decision 6 in `request(Start)`. `Run.repo_dir` is set for every run. The stale attention line is added at start when `stale` is non-empty.

**Acceptance.** Tests pass. `profile/resolve.rs` and `profile/proposal.rs` are pure (the Verification grep).

**Commit.** `feat(daemon): store the repository profile in the data directory and let it supersede a plan's profile`

### M8b.5 Deciders I: schemas, prompts, parsing, fallbacks and argv

**Files.** Create `crates/daemon/src/decider/{mod,schema,prompt,parse,fallback,argv}.rs` and `decider/tests.rs`. Modify `crates/daemon/src/lib.rs`.

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
  - `check_summary_over_40_lines_or_300_chars_is_an_error`;
  - `blocked_reason_unknown_kind_is_an_error`.
- Answer extraction (`answer_from_events`):
  - `structured_output_event_wins`;
  - `structured_output_tool_use_is_read`;
  - `assistant_text_in_a_fence_is_parsed`;
  - `subagent_text_is_ignored`;
  - `no_answer_is_an_error`.
- Fallbacks:
  - `fallback_table`: each kind's deterministic answer, with `check_summary` equal to `run::exec::summary(tail)`;
  - `fallback_decision_carries_the_reason`.
- Argv:
  - `claude_decider_args_exact`, for two `DeciderCaps` variants, with the user-settings flags present and absent;
  - `codex_decider_args_exact`;
  - `schema_file_name_is_stable`.

**Change.** Decisions 16 and 17, the pure parts.

**Acceptance.** Tests pass. Every file in `decider/` except `call.rs` is pure.

**Commit.** `feat(daemon): add decider schemas, prompts, answer parsing and deterministic fallbacks`

### M8b.6 `fake-agent`: decider mode, scout scripts, the `bash` step, every hook group

**Files.** Create `crates/fake-agent/src/decider.rs`, `src/bash.rs`, `crates/fake-agent/tests/adapt_modes.rs`. Modify `crates/fake-agent/src/main.rs` (dispatch), `runtime.rs`, `script.rs`, `roles.rs` (M8a's).

**Tests first**, in `adapt_modes.rs`:
- `decider_mode_answers_in_the_claude_shape` and `decider_mode_answers_in_the_codex_shape`: the scripted answer is found by M8b.5's `answer_from_events` over the parsed output, with usage.
- `decider_output_conforms_to_the_fixtures` (M8a decision 51's key-set test against M8b.1's decider fixtures).
- `decider_calls_are_recorded` (kind, argv, prompt in `calls.jsonl`).
- `decider_without_a_script_exits_2`.
- `decider_hang_blocks_until_killed`.
- `decider_scripts_are_claimed_in_order`.
- `scout_scripts_are_claimed_by_scout_id`: `scout-onboarding-1.jsonl` serves `--scout onboarding-1695000000`.
- `bash_step_applies_updated_input`: a settings JSON with two `PreToolUse` groups, the second a script printing `updatedInput` with `echo rewritten`. `FAKE_AGENT_BASH_LOG` shows `original == "echo original"` and `ran == "echo rewritten"`, and the emitted `tool_use` input is the rewritten command.
- `bash_step_without_hooks_runs_the_command`.
- `every_hook_group_is_discovered_and_hook_keeps_the_first`.
- Milestone 3's and M8a's `fake-agent` tests still pass.

**Change.** Decisions 36 and 37.

**Acceptance.** Tests pass.

**Commit.** `feat(fake-agent): add a scripted decider mode, scout scripts, and a bash step that honours PreToolUse hooks`

### M8b.7 Deciders II: the call

**Files.** Create `crates/daemon/src/decider/call.rs`, `crates/cli/tests/decider_call.rs`. Modify `crates/daemon/src/manager/config.rs` (read `ANTHREX_DECIDER_BIN` in `from_vars`, a new field `decider_bin: Option<String>`), `run/driver.rs` (`RunContext.decider_bin`), and `crates/daemon/src/headless/claude_stream.rs` (only if M8b.1 found `result.structured_output`).

**Tests first**, in `decider_call.rs`, with real `fake-agent` processes. `DeciderContext.program` is `fake_agent_bin()` and the timeout is 5 s.
- `claude_mode_returns_the_scripted_answer_and_usage`.
- `codex_mode_returns_the_scripted_answer`.
- `the_prompt_goes_on_stdin_for_claude_and_last_for_codex`, from `calls.jsonl`.
- `decider_bin_wins_over_the_runtime_command` (`ManagerConfig::from_vars` with both set).
- `mode_off_never_spawns`: the program is a script that creates a marker file, and no marker appears.
- `a_hanging_decider_falls_back_at_its_timeout_and_leaves_no_process`. The call returns after at least 5 s and within `5 + 2 (kill grace) + 5 (slack) = 12 s`, with reason `the decider timed out after 5 s`. `pgrep -f <unique marker in the prompt>` then finds nothing within a 5 s deadline loop.
- `garbage_empty_and_oversized_answers_fall_back`: text `not json`, text `{}` (a schema error naming `kinds`), and 300 KiB of text (cut to `ANSWER_MAX_BYTES`, then not JSON). Each gives its exact reason.
- `a_failed_turn_and_an_early_exit_fall_back`: `fail_turn` and `exit 3`, each with its reason.
- `a_missing_program_falls_back_with_could_not_start`.

**Change.** Decision 16's I/O. The call never holds a lock, writes the schema file on `spawn_blocking`, and kills the process group on every early return.

**Acceptance.** Tests pass. Add rows to `docs/timing-budgets.md` for the 12 s bound (`timeout 5 s + kill grace 2 s + spawn slack 5 s`) and for `GOAL_REQUEST_TIMEOUT`.

**Commit.** `feat(daemon): call deciders as one-shot headless sessions with bounded time and deterministic fallbacks`

### M8b.8 The output filter

**Files.** Create `crates/daemon/src/output_filter.rs`, `crates/cli/src/filter_run.rs`, `crates/cli/src/filter_hook.rs`, `crates/cli/tests/filter_run.rs`, `crates/cli/tests/filter_hook.rs`. Modify `crates/cli/src/main.rs` (two pre-clap dispatches), `crates/daemon/src/headless/{mod.rs,argv.rs}` (`HeadlessSpec.output_filter`, the `add_hook` call), and `run/role_launch.rs` (M8a's `worker_spec` sets `output_filter` and, for Claude, adds `log_dir` to `ClaudeSandbox.writable_roots`).

**Tests first:**
- In `output_filter.rs`:
  - `failures_only_keeps_matches_with_context_and_the_tail`: 5000 lines with one `test foo ... FAILED` at line 2500 and the summary at the end give that line, its 5 followers, the last 20 lines and one omission marker per gap, 30 lines at most.
  - `failures_only_on_success_is_the_last_10_lines`.
  - `failures_only_without_matches_is_the_last_60`.
  - `at_most_100_matched_lines`.
  - `tail_is_60_lines`.
  - `long_lines_are_cut_to_500_chars`.
  - `matches_strips_cd_and_env_prefixes_and_respects_word_boundaries` (`cargo testing` does not match `cargo test`).
  - `rewrite_keeps_every_tool_input_key`.
  - `rewrite_never_double_wraps`.
  - `add_hook_appends_a_second_group` (the exact settings JSON; M3's group unchanged).
- In `filter_run.rs`, running the built binary:
  - `filter_run_preserves_exit_status_and_signals`: exit 3 gives 3; a child killed by `SIGTERM` gives 143.
  - `filter_run_writes_the_full_log_and_prints_the_filtered_view`: 5000 lines logged, at most 32 printed, and the last line names the log path and `5000 lines, exit 1`.
  - `filter_run_passes_stdin_through`.
  - `filter_run_still_runs_when_the_log_cannot_be_written` (log dir under a read-only directory).
- In `filter_hook.rs`:
  - `hook_wraps_a_matching_command` (exact stdout JSON).
  - `hook_is_silent_for_other_commands_and_tools`.
  - `wrapped_commands_behave_exactly_like_the_original`. For each of these commands, running the wrapped command through `/bin/sh -c` prints the same `filter-run` log contents and exit code as the original: `printf 'a b\n'`; `echo "it's"`; a two-line command with a heredoc; `false || echo ok`; `FOO=1 sh -c 'echo $FOO'`; `cd sub && ls`; a command with `ü` and a tab.
  - `hook_exits_zero_silently_on_garbage_and_oversized_input`.
  - `hook_finishes_within_its_deadline`: stdin held open, it exits within `FILTER_HOOK_DEADLINE + 2 s`. Add the row to `docs/timing-budgets.md`.
- In `headless/argv.rs` tests:
  - `claude_worker_settings_include_the_filter_hook`;
  - `reviewers_scouts_and_codex_get_no_filter_hook`;
  - `worker_spec_sets_the_hook_only_for_a_stored_profile_with_prefixes`.

**Change.** Decisions 26–28.

**Acceptance.** Tests pass. `anthrex --help` lists neither `filter-run` nor `filter-hook`.

**Commit.** `feat: filter test output for headless Claude workers through a PreToolUse hook and anthrex filter-run`

### M8b.9 Scout sessions and `submit_scout_report`

**Files.** Create `crates/daemon/src/scout/{mod,contract,spec,report,machine,service}.rs`, `scout/tests.rs`, `crates/mcp/src/tools_scout.rs`, `crates/cli/tests/scout_service.rs`. Modify:
- `crates/mcp/src/{lib.rs,tools.rs,forward.rs}` (`scout_id`, the scout role);
- `crates/cli/src/main.rs` (`anthrex mcp` flags);
- `crates/daemon/src/headless/{mod.rs,argv.rs}` (`McpTarget.scout_id`, `--scout`);
- `server/run_api.rs` (route scout tool calls) and M8a's `server/headless_guard.rs` (decision 12's refusal text for a run-less headless window).

**Tests first:**
- In `scout/tests.rs` (pure):
  - `area_scout_spec_is_read_only` (the exact `HeadlessSpec`: plan mode, read-only Codex sandbox, the four tools, no `claude_sandbox`);
  - `onboarding_scout_spec_is_sandboxed_in_its_worktree` (`Bash` allowed, no `Edit`/`Write`, `claude_sandbox` with no roots, `run_ref == None`, `workspace-write`);
  - `onboarding_scout_without_worker_sandbox_has_none`;
  - `codex_scouts_and_deciders_pass_the_user_config_only_flags`. With a test `CliCaps` whose `codex_user_config_only` is `Some(["--flag"])`, both the Codex scout argv (`codex_args`) and `codex_decider_args` contain it. With `None`, neither does;
  - `route_picks_the_lowest_strength_at_or_above` (default roster: Claude `fast` gives `claude-haiku-4-5`; Codex `fast` gives Codex `""`);
  - `valid_ids`;
  - `report_validation_cases`: a missing summary, 61 files, a profile on an area scout, a missing profile on onboarding, a bad env key, each with the exact message;
  - `machine_nudges_once_then_fails`;
  - `machine_wraps_up_then_kills_at_1_5x_the_tool_budget`;
  - `machine_times_out`;
  - `machine_exit_without_report_fails`;
  - `machine_report_closes_stdin_then_kills_and_removes`;
  - `machine_sums_usage`.
- In `crates/mcp`:
  - `scout_tool_is_submit_scout_report`;
  - `scout_schema_limits`;
  - `mcp_args_for_a_scout` (the exact vector `["mcp","--role","scout","--scout","onboarding-1","--window","7","--socket","/tmp/a.sock"]`);
  - `run_is_required_except_for_scouts`.
- In `scout_service.rs`, with a real `WindowManager` whose `claude_bin` is `fake-agent`, and a stub `submit_scout_report` routed through `ScoutService::tool`:
  - `scout_report_is_stored_and_the_session_retired`: the report file exists with the fields, the window is `Exited`, then removed after `RETIRE_AFTER`, within a deadline of `RETIRE_AFTER + INTERRUPT_GRACE + 5 s`;
  - `scout_without_a_report_is_nudged_then_failed`: the second turn's stdin line is `SCOUT_NUDGE`;
  - `a_report_from_another_window_is_refused`;
  - `a_second_report_is_refused`;
  - `a_run_less_scout_window_refuses_client_input_with_the_profile_hint` (a real socket `Input`).

**Change.** Decisions 12–15.

**Acceptance.** Tests pass. `ScoutService` never holds `daemon::lock` across a manager call or an await (state it in the pull request).

**Commit.** `feat: add headless scouts with submit_scout_report, stored reports and a pure lifecycle machine`

### M8b.10 Onboarding: detection, verification, confirmation and `anthrex profile`

**Files.** Create `crates/daemon/src/profile/{verify,service}.rs`, and complete `profile/proposal.rs`. Create `crates/cli/src/profile_cmd.rs`, `crates/cli/tests/profile_verify.rs`, `crates/cli/tests/profile_cli.rs`. Modify:
- `run/driver.rs` (construct `ScoutService` and `ProfileService`, and share the `GitQueue` as `Arc`);
- `server/run_api.rs` (`RunRequest::Profile`);
- `lifecycle.rs` (`ProfileService::restore` after `RunService::restore`, before the socket binds);
- `crates/cli/src/main.rs`.

**Tests first:**
- In `proposal.rs`:
  - `verification_drops_commands_that_did_not_pass`: setup failing, check passing, single test passing, gives setup dropped and the others kept;
  - `single_test_needs_sample_and_a_matching_line`: exit 0 without the line drops `single_test`, `test_passed` and `sample_test` with the reason;
  - `edit_value_parsing`: `check 'cargo test'` as a string, `modules '["crates/*"]'` as an array, `check_timeout_secs 600` as an integer, `env.RUST_LOG debug` as an env entry, `--unset single_test`, and `lint x` refused as `unknown key lint; one of <keys>`;
  - `edit_rejects_invalid_values_with_the_rule` (M8b.4's `validate` messages);
  - `edit_of_a_command_key_needs_reverification_and_of_modules_does_not`;
  - `show_text_is_exact` (golden).
- In `profile_verify.rs`, on a `TempRepo`:
  - `verification_never_touches_the_checkout_and_salvages_dirt`. `check` modifies a tracked file, creates an untracked one and exits 0. Afterwards the user's checkout is clean and unchanged (`git status --porcelain` empty, same `HEAD`), `<wt>/runs/.profile-verify` is gone, and a salvage ref under `refs/anthrex/salvage/onboarding/` holds the change.
  - `a_hanging_verification_command_times_out_and_is_dropped` (`sleep 60` with a 2 s timeout; `timed_out`, dropped).
  - `env_and_worktree_are_substituted` (`check` prints `$CARGO_TARGET_DIR`, which is under the scratch path).
  - `the_scrubbed_environment_applies` (`CLAUDE_CODE_X` set in the test process under the shared env lock is absent).
- In `profile_cli.rs`, with `RunHarness`, `onboarding.auto` true, `verify_timeout_secs = 10`, and a repository with `check.sh`, `tests/t_ok.sh` (prints `PASS t_ok`) and `Cargo.lock`:
  - `e2e_detect_proposes_only_verified_commands`. The scout script reports `check = "sh check.sh"`, `single_test = "sh tests/{test}.sh"`, `test_passed = "PASS {test}"`, `sample_test = "t_ok"`, `setup = "sh missing.sh"`, `generated = ["Cargo.lock"]` and `protected = [".cursor/**"]`. Within `PROFILE_WAIT` the status is `Ready`. `profile show --proposed` has `check`, `single_test`, `generated`, and `protected: built-in … + .cursor/**` (the stored file's `protected == [".cursor/**"]`), and lists `setup` under dropped with its exit code.
  - `e2e_confirm_stores_the_profile_outside_the_repository`: `profile.toml` is under `<data>/repos/`, `git status --porcelain --ignored` in the repo is empty, and there is no `.anthrex` path.
  - `e2e_reject_stops_a_running_scout` (a scout script that hangs; reject; the window is gone and the proposal is deleted).
  - `e2e_edit_goes_through_verification` (`edit check 'sh broken.sh' --yes`: the proposal ends `Ready` with `check` dropped and is **not** auto-confirmed; the stored `check` is unchanged).
  - `e2e_status_reports_stale_files_and_auto_detects` (change `Cargo.lock` after confirm: `stale == ["Cargo.lock"]`, and a new proposal starts).
  - `e2e_detection_in_progress_at_restart_is_failed_and_cleaned`.
  - `e2e_detect_refuses_codex_project_config_without_trust_project`: `scouts.runtime = "codex"`, `ANTHREX_TEST_CODEX_PROJECT_CONFIG=load` makes the daemon act as if Codex loads repository config and cannot exclude it (`codex_user_config_only = None`), and the repo tracks `.codex/config.toml`. Detect is refused naming it, and `--trust-project` starts detection.
  - `e2e_detect_refuses_project_settings_without_trust_project`: the daemon runs with `ANTHREX_TEST_NO_SETTING_SOURCES=1` and the repo tracks `.claude/settings.json` with `hooks`. Detect is refused with the exact text, and `--trust-project` starts detection.
  - `e2e_a_second_detect_is_refused_while_one_runs`.

**Change.** Decisions 8–11.

**Acceptance.** Tests pass. No code path writes under the repository root (review this by reading every `write_atomic` and `run_shell` caller, and state it in the pull request).

**Commit.** `feat: detect, verify and confirm the repository profile with a headless onboarding scout`

### M8b.11 Engine: deciders inside a run

**Files.** Create `run/engine/deciders.rs`, `run/model_adapt.rs`, `run/engine/tests/deciders.rs`. Modify `run/engine/{gates,merge,done,requests,dispatch,restore}.rs`, `run/model.rs`, `run/snapshot.rs`, `run/reconcile.rs`, `run/contract.rs` (`check_failed_message` and `candidate_red_message` take the summary text), `run/report.rs`, and `run/driver/adapt.rs` (execute `Decide`: resolve `evidence_refs` into `Evidence` by reading report files on `spawn_blocking`, then `decide`).

**Tests first**, in `engine/tests/deciders.rs`:
- `failed_check_defers_the_bounce_until_the_summary`: `Check { ok: false }` gives `failures == 1` and one `Decide` op, and no `Deliver`. `Decided` gives a `Deliver` whose text contains the summary lines and not the raw tail, with `summary_source == Decider`.
- `summary_fallback_uses_the_tail_and_is_marked`.
- `candidate_red_uses_the_summary_and_the_queue_moves_on`: the next queued task's `MergeCandidate` is emitted before `Decided`.
- `deciders_take_a_reader_slot_before_reviewers`: `max_readers = 1`, a review pending and a decider queued; the decider starts first and `readers_busy == 1`.
- `a_decider_waiting_past_slot_wait_falls_back_on_tick`.
- `mode_off_decides_inline_without_an_op`.
- `an_unclassified_block_is_classified`: three cases (`question` stays, `environment` changes the reason, `mis_sized` gives rung 3 with the size raised), plus the reply text `Blocked recorded (classifying). …`.
- `a_typed_kind_is_never_reclassified`.
- `an_answer_before_classification_wins`.
- `size_check_raises_s_to_m_and_rederives_route_budget_and_review` (effort `low` → `medium` when the plan set none; the budget M's; the review level `medium`; the note text).
- `a_planner_set_effort_survives_a_raise`.
- `size_check_l_blocks_as_mis_sized_without_a_rung`.
- `size_check_never_lowers_and_records_agreement`.
- `missing_ids_keep_their_size_as_fallback`.
- `a_pending_size_check_blocks_dispatch`.
- `tasks_without_evidence_skip_the_cross_check` (note text; no op).
- `an_edit_cross_checks_only_the_touched_tasks`.
- `a_restart_requeues_a_dropped_decider_op` (`Restore` with the `Decide` op `NotStarted`, then `Resume`: queued again).
- `decider_usage_is_counted_on_task_and_run`.
- `reviewer_prompt_uses_the_latest_summary`.

In `reconcile.rs` tests: `decide_and_measure_diff_are_not_started`.

**Change.** Decisions 18–21.

**Acceptance.** Tests pass. `run/engine/deciders.rs` is pure. Every M8a engine test still passes.

**Commit.** `feat(daemon): run size cross-checks, check summaries and block classification as decider ops in reader slots`

### M8b.12 Triage, the fast path and `run promote`

**Files.** Create `run/triage.rs`, `run/triage_tests.rs`, `crates/cli/src/run_cmd/adapt.rs`. Modify `run/driver/adapt.rs` (`StartGoal`), `run/engine/requests.rs` (`Promote`), `run/report.rs` (path line), `crates/cli/src/run_cmd.rs`, `run_cmd/status.rs`, and `server/run_api.rs`.

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
  - `status_shows_fast_path_and_triage`.

**Change.** Decisions 22–25.

**Acceptance.** Tests pass. `anthrex run --help` lists `promote` and `stats`.

**Commit.** `feat: triage goals with a decider and run single-task goals on the fast path`

### M8b.13 Metering: usage by role and the OTLP receiver

**Files.** Create `crates/daemon/src/metering/{mod,otlp,server}.rs`, `metering/otlp_tests.rs`, `crates/daemon/tests/otlp_server.rs`. Modify `run/snapshot.rs` (`RunInfo.usage`), `run/engine/mod.rs` (`OrchestratorUsage`), `run/driver.rs` (`orchestrator_usage`), and `lifecycle.rs` (bind, `otlp.addr`, shutdown removal).

**Tests first:**
- In `otlp_tests.rs`:
  - `parses_the_recorded_fixture`: M8b.1's body gives points for `r-fix`/`orchestrator`, with the recorded values;
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

**Acceptance.** Tests pass. `metering/otlp.rs` is pure. No `hyper` or other HTTP crate is added (`cargo tree -p anthrex-daemon` shows none).

**Commit.** `feat(daemon): meter deciders, scouts and the orchestrator's OTLP export into run usage by role`

### M8b.14 History: phases, diff measurement, `history.jsonl`, reverts and `run stats`

**Files.** Create `run/phases.rs`, `run/history.rs`, `run/stats.rs`, `run/history_io.rs`, `run/history_tests.rs`, `crates/daemon/tests/history_io.rs`. Modify every file under `run/engine/` and `run/edits.rs` that assigns `Task.state` (through `set_state`). Also modify `run/engine/merge.rs` and `requests.rs` (emit `MeasureDiff` and `AppendHistory`), `run/driver/adapt.rs`, `run/reconcile.rs`, `server/run_api.rs` (`Stats`), and `run_cmd/adapt.rs`.

**Tests first:**
- In `history_tests.rs`:
  - `set_state_accumulates_phase_times`: queued 10 s, preparing 5 s, working 100 s, check 20 s, review 30 s and merge 3 s from injected times; `Pending` counts nowhere;
  - `max_rung_is_tracked`;
  - `task_record_from_a_merged_task`: every field from a fixture task, with two review rounds (one with an `important` finding and one `minor`), one check failure, one generated-file bounce, and `done_signal == TurnEndFallback`;
  - `unfinished_tasks_get_records_when_the_run_ends`;
  - `run_record_fields`;
  - `stats_medians_by_class_over_merged_tasks` (the lower middle value for an even count; `-` without merged tasks; the exact `render` output);
  - `reverted_counts_join_task_and_run_reverts`.
- In `history_io.rs`, on a `TempRepo`:
  - `measure_diff_counts_files_hunks_and_lines` (two files, three hunks, a binary file);
  - `measure_diff_of_a_merge_commit_is_the_tasks_contribution_after_a_hand_back`;
  - `append_is_one_line_and_fsynced` (the code is checked in review; the test checks content);
  - `read_history_keeps_the_last_line_per_record_id_and_skips_a_torn_line`;
  - `history_append_is_reconciled_exactly_once`: a `Replay` when the line is present, `NotStarted` when absent, and after the replayed path exactly one line;
  - `detect_reverts_matches_merge_and_accept_commits` (`git revert --no-edit <task merge>` and `git revert -m 1 --no-edit <accept merge>` give two revert records, the second with `task_id == None`; running again adds none).
- In the engine tests:
  - `merged_task_measures_then_appends_once`;
  - `cancel_without_commits_appends_without_a_diff`;
  - `accept_appends_the_run_record_after_the_tasks`.

**Change.** Decisions 31–35.

**Acceptance.** Tests pass. `rg -n "\.state = TaskState::" crates/daemon/src/run` prints only `run/phases.rs`.

**Commit.** `feat: record every finished task to history.jsonl with phases, diff size, gates and reverts, and add anthrex run stats`

### M8b.15 End-to-end scenarios and the smoke stage

**Files.** Create `crates/cli/tests/run_e2e_adapt.rs`, `crates/cli/tests/run_e2e_adapt_profile.rs`, `scripts/pty_smoke_adapt.py`. Modify `scripts/pty-smoke.py` (≤ 5 lines) and `crates/cli/tests/support/run_harness.rs` (decision 36's defaults and the M8b helpers).

**Tests first.** All use `RunHarness`. A stored profile is written with `stored_profile` unless the test detects one: `check = "sh check.sh"`, `single_test = "sh tests/{test}.sh"`, `test_passed = "PASS {test}"`, `filter_prefixes = ["sh tests/"]`, `generated = ["Cargo.lock"]`, and the default `protected`.

- `e2e_green_s_task_on_the_fast_path`. `with_deciders(claude)`; `triage-1.json` answers `single` with one S `check`-mode task owning `a.txt`. The worker commits `a.txt` and calls `DONE`; the reviewer approves. Assert:
  - `run start --goal` prints the id and `fast path: one task, no plan gate`;
  - no snapshot ever shows `awaiting_approval`;
  - `path == Some(Fast)`, `approved_by == "fast path"`, and the run is complete;
  - `triage.source == Decider`;
  - `run accept --yes` succeeds;
  - `history.jsonl` has one task record with `path == fast` and `diff.files == 1`, and one run record `accepted` with `accepted_commit` equal to `main`'s head.
- `e2e_goal_needing_a_plan_is_refused_without_side_effects`. Triage answers `plan`: exit 1 with `refused_message`, no `refs/heads/anthrex/*`, no `<data>/runs/*`, and no window.
- `e2e_goal_without_deciders_takes_the_plan_path` (mode `off`: the reason names `deciders are off`).
- `e2e_goal_without_a_profile_starts_detection` (no stored profile, `onboarding.auto`: the exact refusal, and `profile status` shows detection).
- `e2e_promote_records_intent_and_the_task_continues`.
- `e2e_check_bounce_carries_the_decider_summary`. The worker's first commit breaks `check.sh`, and `check_summary-1.json` answers two lines. The worker's `read_message` expects the first summary line and does not see line 150 of the raw output. The task merges with `summary_source == Decider`.
- `e2e_check_bounce_falls_back_to_the_tail_when_the_decider_fails` (no `check_summary` script: the message has M8a's 40-line tail, `summary_source == Fallback`, and the run completes).
- `e2e_unclassified_block_is_classified_as_environment`.
- `e2e_size_cross_check_raises_a_task_and_reports_it`. `onboarding_report(..)` is written, and the plan has one S task and one without evidence. `size_check-1.json` answers M for the first. Assert its size is M, `size_check.agreed == false`, `REPORT.md` has the note, and the second task is skipped with the note.
- `e2e_stored_profile_wins_over_the_plan_profile` (a plan `[profile] check = "false"` is ignored, the log line is present, and the run completes).
- `e2e_a_corrupt_stored_profile_refuses_the_run` (the exact message with the path; no branch).
- `e2e_filter_hook_shrinks_test_output_for_a_claude_worker`. The worker script runs `bash {"cmd":"sh tests/noisy.sh"}`, where `noisy.sh` prints 5000 lines with one `FAILED` and exits 1. `FAKE_AGENT_BASH_LOG` shows the command wrapped, at most 32 output lines, the `FAILED` line, and exit 1. The log file has 5000 lines and lies under `<data>/runs/<id>/logs/t1/`.
- `e2e_generated_lock_file_bounces_at_rung_1` (a stored profile with `generated = ["Cargo.lock"]`; the worker changes `Cargo.lock` outside `owns`; M8a decision 55's rung-1 bounce, not rung 3; the task merges after reverting it).
- `e2e_protected_file_from_the_stored_profile_bounces_at_rung_1`. The worker edits `AGENTS.md` while its `owns` is `**`. M8a's protected-path rule fires from the **stored** profile's list, as a rung-1 bounce naming `AGENTS.md`. After the worker reverts the file, the task merges.
- `e2e_otlp_usage_reaches_the_run_snapshot`. Read `<data>/otlp.addr`, POST M8b.1's fixture with `anthrex.run` rewritten to the test's run id, once with `Content-Length` and once chunked. Within a deadline, `usage.by_role["orchestrator"]` equals the fixture's totals.
- `e2e_history_survives_a_crash_after_the_append_intent` (`ANTHREX_TEST_ABORT_AFTER_INTENT=AppendHistory`: restart, `run resume` if paused, complete; `read_history` gives exactly one task record per task, and the raw file has at most one line per `record_id` plus nothing torn).
- `e2e_revert_after_accept_is_recorded_and_counted` (accept, `git revert -m 1` on `main`, `run stats --json`: `reverted == 1` in the S row).

In `scripts/pty_smoke_adapt.py`, the function `adapt_stage(env, bin_path)` is called from `pty-smoke.py` after stage 11c (and before M8c's stage 11e when that has landed first) and prints `== stage 11d: a goal takes the fast path ==`. The run milestones' stage letters are fixed: M8a `11c`, M8b `11d`, M8c `11e`, M9 `11f`, M9.5 `11g`, called in that order whichever merges first. It:
1. creates `/tmp/anthrex-smoke-adapt-<pid>`, a repo with identity, `check.sh` and one commit;
2. writes the stored profile and the decider script under the smoke data dir, with `ANTHREX_DECIDER_BIN` set to `fake-agent`, plus the worker and reviewer scripts of `e2e_green_s_task_on_the_fast_path`;
3. runs `anthrex run start --goal "add a" --dir <repo>`;
4. polls `anthrex run status <id> --json` every 0.5 s up to `RUN_WAIT` until `complete`;
5. runs `anthrex run accept <id> --yes`;
6. asserts `a.txt` exists on `main`;
7. removes the paths in `finally`.

**Change.** Only tests, scripts and the fixes they uncover.

**Acceptance.** All five AGENTS.md commands pass.

**Commit.** `test: cover the fast path, onboarding, deciders, the output filter, metering and history end to end, and add a smoke stage`

## Verification

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

Milestone-specific checks:

- `PROTO_VERSION` equals the header's derivation, recorded under "Implementation notes".
- This prints nothing:

  ```bash
  rg -n "std::fs|std::process|std::thread|tokio|SystemTime" \
    crates/daemon/src/profile/{resolve,proposal}.rs \
    crates/daemon/src/scout/{contract,spec,report,machine}.rs \
    crates/daemon/src/decider/{mod,schema,prompt,parse,fallback,argv}.rs \
    crates/daemon/src/output_filter.rs crates/daemon/src/metering/otlp.rs \
    crates/daemon/src/run/{triage,phases,history,stats}.rs crates/daemon/src/run/engine/deciders.rs
  ```

- `rg -n "\.state = TaskState::" crates/daemon/src/run` prints only `run/phases.rs`.
- `rg -n "\.anthrex" crates/ scripts/` finds no path written into a repository.
- `git diff --stat main -- crates/config/src/lib.rs` is empty.
- `wc -l` on every file in the file-size table and every new file: nothing new above 600, and nothing existing grown past its budget.
- Every git call added by this milestone goes through `worktree::run_git` (`--no-optional-locks`, scrubbed environment; AGENTS.md rules 10–11), and none runs under `daemon::lock`.
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
   - `anthrex profile show --proposed` shows `check = "make check"` or equivalent, a `single_test` with `{test}`, a `test_passed` regex, and a verification block with `ok` for each command;
   - `ls -a demo` shows no `.anthrex`, and `git -C demo status --porcelain --ignored` is empty.
2. Run `anthrex profile confirm` and answer `y`. `/tmp/anthrex-m8b/data/repos/demo-*/profile.toml` exists.
3. Run `anthrex run start --goal "Make hello.sh print hello" --dir /tmp/anthrex-m8b/demo`. Check:
   - the triage line says `code/single (decider)` and `fast path`;
   - attaching with `anthrex` and opening the worker's conversation (`C-b m`) shows, for its test run, the filtered view ending in `[anthrex] full output: …`;
   - the run completes. Then run `anthrex run accept <id> --yes`.
4. Run `anthrex run start --goal "Rewrite this project as a Rust workspace with a CLI crate, a library crate and a test suite" --dir …`. It exits 1 with the plan-path message, and `git -C demo branch --list 'anthrex/*'` is empty.
5. `anthrex run stats --dir …` shows one S task, merged.
6. Edit `Makefile` and commit. `anthrex profile status` shows `stale: Makefile`, and detection starts again. Run `anthrex profile reject`.
7. Run `anthrex profile edit check 'sh tests/run.sh all' --yes`. Status goes to `Verifying`, then the value is stored.
8. `curl -s -X POST -H 'Content-Type: application/json' --data @<M8b.1 fixture, anthrex.run replaced by a run id> "$(cat /tmp/anthrex-m8b/data/otlp.addr)/v1/metrics"`, then `anthrex run status <id> --json` shows `usage.by_role.orchestrator`.
9. Run `anthrex daemon stop`. `pgrep -fl "anthrex daemon"` shows nothing of yours. Remove `/tmp/anthrex-m8b`.

## Review focus

These are the five input classes or failure modes most likely to bite a user that the task tests above would not exercise without being told to. Each has a test in its owning task; the reviewer checks those tests exist and fail without the fix.

1. **A decider that hangs, crashes, or answers garbage, nothing or megabytes.** A decider is a second model surface on every bounce. Without a hard bound it stalls runs; without process-group kills it leaks processes. Tests: `a_hanging_decider_falls_back_at_its_timeout_and_leaves_no_process` and `garbage_empty_and_oversized_answers_fall_back` (M8b.7).
2. **Test commands with shell syntax.** Quotes, heredocs, `&&`/`||`, env prefixes, `cd` prefixes, non-ASCII and non-zero exits all pass through the filter hook. A wrapper that re-quotes wrongly silently runs a different command, and one that loses the exit status makes a failing test look green to the worker. Tests: `wrapped_commands_behave_exactly_like_the_original` and `filter_run_preserves_exit_status_and_signals` (M8b.8).
3. **Verification commands with side effects.** A `check` that rewrites tracked files, creates untracked ones, or hangs. Verification must never touch the user's checkout, must never delete dirt unsalvaged, and must end. Tests: `verification_never_touches_the_checkout_and_salvages_dirt` and `a_hanging_verification_command_times_out_and_is_dropped` (M8b.10).
4. **A hand-edited or corrupt stored profile.** `profile.toml` is a plain file a user may edit. A parse error must refuse the run by path and never fall back to a plan's weaker profile, and `profile edit` must refuse a glob with `..`, a `single_test` without `{test}` and a regex that does not compile. Tests: `an_unparseable_profile_is_reported_with_its_path` (M8b.4), `e2e_a_corrupt_stored_profile_refuses_the_run` (M8b.15), and `edit_rejects_invalid_values_with_the_rule` (M8b.10).
5. **A crash around a history append.** Between an append and its `done` line, a replayed op must not double-count a task in `stats`, and a torn last line must not poison the file. Tests: `history_append_is_reconciled_exactly_once` (M8b.14) and `e2e_history_survives_a_crash_after_the_append_intent` (M8b.15).

## Risks and gotchas

1. **Deciders cost tokens on every bounce.** A check summary per failed check, a size check per plan edit, and a triage per goal. They run at the fast tier and low effort, one call each, and `mode = "off"` removes them. Meter them (decision 29) before judging their worth.
2. **Strict schemas.** Structured-output modes may reject a schema with optional properties; decision 17's schemas already make every property required and nullable. If M8b.1 finds a CLI rejects `anyOf` or `["string","null"]`, simplify that runtime's schema and let `parse` enforce the rest.
3. **Codex prompts are on the argv.** `ps` shows them, and the size is bounded by `ARG_MAX` (1 MiB on macOS), which `PROMPT_MAX_BYTES` (128 KiB) stays well inside. Claude prompts go on stdin.
4. **Onboarding runs the check twice**, once by the scout and once by the engine. On a large repository that doubles a long build. It is the price of "a command that did not run is not proposed" without trusting the scout's claim.
5. **The sandbox blocks the scout's network.** A `setup` like `cargo fetch` fails inside the scout's sandbox. The contract tells it to propose the command anyway, and the engine's unsandboxed run decides.
6. **Fingerprint false positives.** Any change to `Cargo.toml` makes the profile stale, even a version bump. That is cheap: the stale profile stays in use, and detection only proposes.
7. **The OTLP port is unauthenticated on loopback.** Any local process can post usage for a run id. Metering is informational and never gates anything; keep it that way.
8. **Wrapped commands and allow rules.** If `HOOK_SETS_ALLOW` must be true, the hook approves the rewritten `Bash` call. It rewrites only calls that start with a profile prefix, which the worker's `--allowedTools Bash` already allows, and the sandbox still confines them.
9. **The history file grows forever.** At a few KiB per task this is harmless for years; pruning is a follow-up.
10. **Revert detection sees only the base branch's first 2000 commits** and only `git revert`'s standard message. A manual revert goes unseen, and the stats undercount reverts.
11. **Stale M8a tests.** The harness defaults deciders to `off` and onboarding to manual (decision 36). If an M8a test starts failing with decider calls in its log, a harness default was lost.
12. **Built-in `protected` entries cannot be switched off from M8b.** The stored list holds extras only, `profile edit` refuses built-ins, and M8a always unions `BUILTIN_PROTECTED`. A user who truly needs a task to change `AGENTS.md` names it exactly in that task's `owns` (M8a decision 56).
13. **Socket and temp paths.** Every new test directory is under `/tmp` (`tempfile::Builder::new().prefix("ax-…").tempdir_in("/tmp")`), never `std::env::temp_dir()`.

## Follow-ups handled

M8a's "Out" table assigned these to M8b; each is handled here:

- Deciders and `ANTHREX_DECIDER_BIN`, triage, the fast path, the size cross-check, the decider check summary, and classification of free-text `task_blocked` reasons: decisions 16–25.
- The repo profile file, the onboarding scout, and profile re-proposal: decisions 4–11. There is no `.anthrex/profile.toml`, per the amended spec §6.
- The output-filter `PreToolUse` hook, OTLP metering of the orchestrator, `history.jsonl` and `anthrex run stats`: decisions 26–35.

New follow-ups to record in `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` during implementation:
- a TUI form for the profile (M9);
- the output filter for Codex workers (under M9.5);
- pruning `history.jsonl` and `<data_dir>/runs/*/logs/`;
- routing and threshold proposals in `run stats` (M9.5).

## Spec defects found while writing this brief

1. **§6 against §4, read-only scouts.** §6 says the onboarding scout "must run `check` and `single_test` once successfully before proposing them". §4's table gives scouts `Writes? no`, and "Read-only roles are launched read-only by the runtime … Claude with `--permission-mode plan` (which also closes the `Bash` loophole)" — a plan-mode scout cannot run a command. Decision 9 resolves it: only the onboarding scout runs commands, sandboxed in a disposable copy, and the engine re-runs them.
2. **§21 against §14.8, OTLP in M8b.** §21 gives "OTLP metering" to M8b, but §14.8 meters only "the orchestrator, the one PTY session", which milestone 9 creates. Decision 30 builds the receiver and the environment, tested by direct POSTs; wiring it into a window is M9's.
3. **§7.2 rule 5 against §5.1.** Rule 5 cross-checks "each task's size against its scout evidence", but the fast path has no scouts (§5.1: "a scout for a one-line fix is pure overhead"), and its single task is sized by the triage decider. Decision 19 skips the cross-check there.
4. **§13 item 3 against §5.1.** Item 3 puts deciders in `max_readers` slots, which are per run, but triage runs before any run exists. Decision 18 exempts triage.
5. **§4, the decider row.** "Deciders are one-shot headless calls … `claude -p --output-format json`" predates the headless amendment. Every other headless surface uses stream-json, and decision 16 uses stream-json on M8a's spawner so one parser serves all.
6. **§15, "whether the user later reverted it".** A record appended when the task finishes cannot know this. Decision 33 adds an append-only `revert` record, joined by readers.
7. **§6, "manifests" and "test output".** §6 re-proposes "when … a manifest … changes hash" but defines no manifest list. §14.3 filters "test output", but no profile key says which commands produce it. Decision 5 adds `manifests` and `filter_prefixes`.
8. **§14.8 against M8a decision 40.** §14.8 says usage field names "are verified in the first task of the milestone that implements metering". M8a meters stream usage and pins them in M8a.1; M8b.1 only cross-references them.

## Implementation notes

