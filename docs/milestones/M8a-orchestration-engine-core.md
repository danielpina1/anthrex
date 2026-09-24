# Milestone 8a: Orchestration engine core

> Written 2026-09-22 against `docs/superpowers/specs/2026-09-22-adaptive-orchestrator-design.md` (the adaptive-orchestrator spec, "the spec" below), which is binding for this milestone and wins wherever it amends an older document. Built on the refreshed milestone 8 brief (branch `docs/m8-brief-refresh`, `docs/milestones/M8-orchestration-engine.md`): its structure, its grounded file paths and every decision the spec does not change are reused here; everything the spec changes is changed. Checked against `main` at `2cb7e3c` (milestones 1 to 6 merged, protocol 5) and against branch `m6.5-conversation-view` at `9ec0a30` (milestone 6.5, in progress, protocol 6). The research report *Coding agent orchestrator design* (2026-09-22) lists, as its P0, the contradictions between the M8 and M9 brief refreshes; this brief resolves each in the spec's favour: strength tiers are restored (decision 23), review severities are restored (decision 35), the plan gate does not block a tool call (decision 14), the run branch is `anthrex/<run>/integration` (decision 16) and is created at `run start` (decision 14), and the protocol number is re-derived when work starts (decision 3).

**Refreshed 2026-09-23 against `main` at `6f22681`** (milestones 1 to 6.5 merged, protocol 6; M6.5 merged in PR #13). Every name this brief takes from milestones 1 to 6.5 was checked against that commit; each correction is listed under "Implementation notes", "Brief refresh against main (6f22681)", with its evidence. Where a sentence below still says "on `main` at `2cb7e3c`" or "on the M6.5 branch", the refresh section is what holds.

**Headless amendment (2026-09-22, same day).** The spec now makes the orchestrator the only interactive PTY window and runs every other agent headless (§4 "Only the orchestrator is an interactive terminal", §4.2). This brief follows it. Every worker and reviewer is a headless session: one long-lived `claude -p` stream-json process, or one `codex exec --json` process per turn. Each session is registered with the window manager as a window with no terminal (`proto::WindowKind::Headless`, decision 49). The following are gone: bracketed-paste delivery, the settle bit, the resume-dialog watch, the stuck-prompt timer, and idleness inferred from window status or PTY output. In their place: turn-based delivery (decision 29), stream signals (decision 27), a turn-end done fallback and a stream-silence stall watchdog (decision 32), budgets from stream tool use and usage (decision 40), and restart by resuming the persisted session with a hand-over message (decision 28). Decision numbers from 1 to 49 are unchanged in meaning where they are unchanged in text; 50 to 52 are new. A second amendment the same day follows the spec's §4 "Every headless agent is also contained" and §6 `generated`: headless Claude loads only the user's settings (decision 53), workers run in Claude Code's sandbox (decision 54), and a changed generated file outside `owns` is a rung-1 bounce, not a spill (decision 55). A third addition follows §4 "Codex loads only the user's config too" and §6 `protected`: decision 53 covers Codex's project config, and a change to an agent-config or instruction file is allowed only when `owns` names it exactly (decision 56).

## Header

| | |
|--|--|
| Status | `ready` — milestone 6 is `done`, and milestone 6.5 merged in PR #13 as `6f22681`. |
| Depends on | Milestone 6 (persistence, `config` crate, restart) and milestone 6.5 (merged in PR #13: it raised the protocol to 6 and added `proto::Role`, which this brief must not collide with). |
| Spec sections | The spec §2 (terms), §4 (roles, headless sessions, read-only launch, "Every headless agent is also contained"), §4.1 (one writer per task), §4.2 (headless run agents, kill and remove refused), §5.3 steps 3–7, §6 (profile values only, `generated` and `protected` included; the onboarding scout is M8b), §7.1–§7.3, §8, §8.1, §9, §10, §11.1–§11.6, §12.1–§12.3, §13 items 2–5, §14 items 2, 3 (hooks still run in `-p`; the filter itself is M8b), 4, 7 and 8 (per-turn usage from the stream; OTLP for the orchestrator is M8b), §16.2 (an agent round points at a headless window; the node kinds are M8c), §16.4 "Navigation" (the conversation view is how a run agent is watched; the TUI part here is a placeholder, decision 49), §16.5 (snapshot and revision only; views are M8c), §17 in full, §22.1 (headless sessions on the user's login), §23 first two risks (`--bare`, the stream-json input envelope), §18 (the task record), §19 (worker and reviewer tools), §21 (M8a row and the scenario list). From the refreshed M8 brief: its decisions 1, 2, 4, 5, 7, 9, 10, 15, 20, 24, 34 and 44 as amended below; its 37–38 (paste delivery) are replaced by decision 29. |
| Branch | `m8a-engine-core` |
| Protocol version | **7**: one above `PROTO_VERSION = 6` on `main` at `6f22681` (`crates/proto/src/lib.rs:19`, set by milestone 6.5). If `main` has moved when work starts, read `crates/proto/src/lib.rs` on `main` that day and use one above it instead. The derivation is recorded under "Implementation notes". No acceptance criterion greps for a specific number. |

## Starting point

Names below are real on `main` at `6f22681` (re-checked 2026-09-23; see "Brief refresh against main (6f22681)" under "Implementation notes"). If the merged code differs when this milestone starts, use the real names and record the mapping under "Implementation notes".

| From | What this milestone uses |
|------|--------------------------|
| M3 | `crates/daemon/src/launch/mod.rs`: `LaunchPlan { program, args, cwd, env }`, `LaunchContext { window_id, name, socket_path, shell, exe, claude_bin, codex_bin, codex_hook_source, codex_bypass_hook_trust, resume }`, `plan(spec, ctx)`, `hook_command`, `shell_quote`. `launch/claude.rs`: `HOOK_EVENTS` (10 events: `Notification`, `PermissionRequest`, `PostToolUse`, `PreToolUse`, `SessionEnd`, `SessionStart`, `Stop`, `SubagentStart`, `SubagentStop`, `UserPromptSubmit`) and `settings(exe, window_id)`. `launch/codex.rs`: `args`, `HOOK_EVENTS` (8), `toml_string`. `crates/daemon/src/status.rs`: `StatusEvent`, `StatusContext`, `next(current, event, runtime, ctx)`. `crates/daemon/src/hooks.rs`: `HookKind`, `ParsedHook`, `parse`, `accepts`. `crates/fake-agent`: `script::Step { Print, Hook, Notify, Title, Bell, WaitMs, ReadLine, GitCommit, Exit, McpCall, Transcript }` (`Transcript` is M6.5.9's) where `McpCall` prints `fake-agent: mcp_call arrives in milestone 8` and exits 3; `runtime::discover(args)`; `FAKE_AGENT_SCRIPT`, `FAKE_AGENT_ARGS_FILE` (one JSON argv, overwritten per process), `FAKE_AGENT_TRANSCRIPT` (M6.5.9). `crates/cli/tests/support/mod.rs`: `tempdir`, `fake_agent_bin`, `isolated_command`, `TestDaemon`, `Client`. `crates/daemon/src/manager/entry.rs`: `pub(super) enum Process { Live(Window), Dormant { output, cols, rows } }`, which already routes `write_input`, `resize`, `attach`, `snapshot`, `signal_group` and `pid` for a window with no PTY behind it; `Entry`. `WindowInfo { id, name, runtime, cwd, project, worktree, branch, status, tool, since_secs, last_output_secs, session_id, model, subagents, exit }`. `ClientMsg::{Subscribe, Input, Resize, Kill, Remove, Restart, …}`. `proto::HookSource { Claude, CodexNotify, CodexHook }`. |
| M4.5 | `crates/daemon/src/project.rs`: `detect_roots(cwd) -> DetectedRoots { project, worktree, detection_failed }`, `resolve_roots`. `crates/daemon/src/git/mod.rs`: `GitRegistry::{new(settings, publish), register, unregister, snapshot}`, `impl crate::manager::GitRoots for GitRegistry`. `DaemonMsg::Git { root, state }`. `ANTHREX_GIT=off`. |
| M5 | `crate::subprocess::run_captured(command, max_output_bytes, max_stderr_bytes, timeout) -> Captured { outcome, stderr, spawn_error }`, which scrubs `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE`, `GIT_PREFIX` and runs the child in its own process group. `crate::worktree::run_git(git: &OsStr, dir, args: &[&OsStr], deadline: Instant) -> Result<GitOutput, WorktreeError>`, which already passes `-C <dir> --no-optional-locks`, `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0`; `GitOutput { stdout, stderr, success }` with `stderr_tail()`; `OPERATION_TIMEOUT`, `CLEANUP_TIMEOUT`, `RESERVED_DIR = "runs"`, `RESERVED_BRANCH_PREFIX = "anthrex/"`, `repo_worktrees_dir(worktrees_root, project_root)`, `hash8`, `check_branch_syntax`. `crate::manager::GitRoots` (in `manager/remove.rs`: `register(PathBuf)`, `unregister(&Path)`). The manager is split into `manager/{mod,config,conversation,create,entry,remove,restart,restore}.rs` (`conversation.rs` is M6.5's); `WindowManager::create(&self, spec, project, worktree, cols, rows)` is `async`. `project::detect_roots_with(git: &OsStr, cwd, timeout)` is the injectable form of `detect_roots` (`project.rs:54`). `crates/daemon/tests/support/mod.rs`: `TempRepo` (with `failing_post_checkout`, `hook_marker`, `worktree_paths`), `git`, `git_output`. |
| M6 | `crates/config/src/lib.rs`: `Config { prefix, accent, bell, default_runtime, scrollback_lines, ui, panes, runtimes, git, conversation }` (`conversation` is M6.5's), `parse`, `load`, `Problem { key, message, default }`; `report_unknown_keys` skips `orchestrator` silently (`lib.rs:551`), and `lib_tests.rs`'s `every_key_is_read` relies on it with `[orchestrator] max_parallel = 3` (`lib_tests.rs:180`). M6.5 split `lib.rs` into `lib.rs`, `conversation.rs` and `git.rs`, each table read by a `pub(super) fn read_<table>` and checked by a `report_unknown_<table>`. `crates/daemon/src/state/`: `STATE_VERSION = 3`, `StateFile { version, next_id, windows, runs: Vec<serde_json::Value> }`, `WindowRecord { …, session_id: Option<String>, …, run: Option<serde_json::Value> }` (both carried verbatim, meaningless until this milestone), `save`, `load`, `spawn_persister`. `WindowManager::{restore, state_snapshot, restart(self: &Arc<Self>, id)}`; `Entry.run: Option<serde_json::Value>`. `LaunchContext.resume`. `launch::LaunchGate`. |
| M6.5 (merged, `6f22681`) | `PROTO_VERSION = 6` (`crates/proto/src/lib.rs:19`). `crates/proto/src/conversation.rs` with `Role { User, Assistant, System }`, `ToolResult`, `Turn`, `TurnState`, `Block`, `Conversation`, `DegradeReason`, `DropCause`, `NoticeKind`, `ToolState`, `TurnPatch`, **all re-exported at the crate root** (`lib.rs:35–38`) — so this milestone's agent role is `proto::AgentRole`, never `proto::Role`, and no new proto type may be named after any of them. `ClientMsg::{SubscribeConversation { window_id, agent_id, from_rev }, UnsubscribeConversation { window_id, agent_id }}` and `DaemonMsg::{ConversationSnapshot, ConversationDelta, ConversationGone}`, which the TUI handles (not ignores) in `App::on_daemon`, now in `crates/tui/src/app/daemon.rs` (an exhaustive match). `ParsedHook` gains seven fields (`transcript_path`, `tool_use_id`, `tool_response`, `tool_result_truncated`, `tool_result_stringified`, `prompt`, and `session_source`, M6.5.10's resume trigger; `hooks.rs:32–54`). **The conversation model's two inputs** are the ones the headless mapping (decision 27) feeds: `crate::conversation::ConversationSet::on_hook(runtime, &ParsedHook, spawn: Option<&SpawnOrigin>, now_unix_secs, now: Instant, caps)` (`conversation/store.rs:69`), which is authoritative and alone creates turns and tool calls, and `ConversationSet::enrich(&[transcript::Record], caps) -> Vec<Option<String>>` (`store.rs:314`), which adds prose and tool detail to the root conversation only. The manager drives a parsed hook into them through `WindowManager::conversation_hook(window_id, entry, &hook, now)` (`manager/conversation.rs:355`, `pub(super)`), which resolves the spawn origin and notifies subscribers. `transcript::Record` (`transcript/mod.rs:21`) is `UserText { session_id: Option<String>, ordinal: u32, text, human: bool }`, `AssistantText { session_id, ordinal, text }` or `ToolDetail { tool_use_id, input, detail, ok }`; `ordinal` is the 0-based index of the prompt a record belongs to (M6.5 ruling R1). `Entry` gains `conversations: ConversationSet`, `conversation_viewers` and `transcript: TranscriptSlot`. The manager's conversation methods are in `crates/daemon/src/manager/conversation.rs`: `subscribe_conversation` (which also spawns the transcript reader through `transcript::parser_for(runtime)` and `conversation::watch::spawn_reader`, and sets `DegradeReason::NoTranscriptPath` when the root has no path, `manager/conversation.rs:119–150`), `unsubscribe_conversation`, `conversation_changes`, `conversation_reply`, `conversation_snapshot(window_id, agent_id: Option<&str>)` and `conversation_delta`. The per-client subscription task is `server/conversation.rs`. Recorded transcript fixtures live in `crates/daemon/tests/fixtures/transcripts/` with `.meta.json` keys `runtime`, `cli_version`, `captured`, `redactions`, `note`. |

The signature this milestone changes, and the one it deliberately leaves alone, as they stand on `main`:

```rust
// crates/daemon/src/server.rs:40 — constructs its own GitRegistry today (server.rs:47).
// Changed by decision 22. lifecycle::run passes git::settings_from_env(loaded_config.git).
pub async fn serve(listener: UnixListener, manager: Arc<WindowManager>,
                   git: config::Git, shutdown: CancellationToken) -> anyhow::Result<()>;

// crates/daemon/src/manager/create.rs:111 — unchanged: headless windows never go through it.
pub async fn create(&self, spec: WindowSpec, project: PathBuf, worktree: Option<PathBuf>,
                    cols: u16, rows: u16) -> anyhow::Result<WindowInfo>;
```

Why run agents are headless, in terms of today's code: the PTY status machine (`status.rs`) gets idleness from hooks and output quiet. `StopFailure`, `PreCompact` and `PostCompact` are not in `launch::claude::HOOK_EVENTS`, and `hooks::parse` drops them. So a turn that ends on an API error leaves the window `Working` forever, and `Stop` reads as done while sub-agents still run. Headless sessions do not have this problem: every turn ends with an explicit `result` (Claude) or `turn.completed` / `turn.failed` (Codex) event, and rate-limit retries arrive as `system/api_retry`. The PTY status machine is therefore left exactly as it is. Its `StopFailure` gap matters again only for the orchestrator's PTY window, which is M9's.

## Goal

A user writes a plan file — a goal, an optional `[profile]`, and a list of tasks, each with a size, a test mode, the paths it owns, a brief and acceptance criteria — and runs `anthrex run start --plan plan.toml`. The daemon validates it against the spec's size, test-mode, runtime and graph rules and returns at once with a run waiting for approval; the run branch and the first worktrees are prepared while the user reads the plan. `anthrex run approve` starts it (`--yes` skips the gate). The engine schedules tasks by critical path into separate writer and reviewer slots. It runs one headless Claude or Codex worker session per task, in the task's own worktree, and moves each task through the gates: the worker's explicit `task_done` (or, when a turn ends with commits but no `task_done`, one nudge turn and then the fallback), the fail-to-pass test proof for TDD tasks, the profile's check, a fresh cross-runtime review with severities, and a merge queue that tests the merged result before a compare-and-swap onto the run branch. Failures climb a four-rung escalation ladder. Budgets count tool calls, minutes and, when a task sets a token budget, tokens. Turn ends, rate-limit retries, permission denials, tool use and token usage all come from each session's structured event stream, never from a terminal. The user watches any agent through milestone 6.5's conversation view, which is fed from the same stream; no run agent has a terminal to type into. Every side effect is journaled before it happens and reconciled after a crash, dirty work is never deleted, and the run ref is checked before every merge: a run ref that moved, or a base branch that was rewritten, halts the run, while a base branch that only advanced is recorded and listed for the user's confirmation at accept. `anthrex run status` and a pushed snapshot with a revision counter show everything. Nothing reaches the base branch until `anthrex run accept`. Every transition is exercised in CI with `fake-agent`; no test needs a model.

## Scope

In:

- The run engine in the daemon: the §18 task record, plan-file parsing, plan validation (§7.2, §8, §9, §12.1), plan edits (§12.1–§12.2), the plan gate engine side (§12.3), the scheduler (§13 items 2–5), the gates (§11.1–§11.5), the escalation ladder rungs 1–4 (§10), budgets by tool calls, wall-clock and optional tokens (§9, §14.7, §14.8), worktrees, branches, salvage and cleanup (§11.6, §17), the ref guard, the intent journal and reconciliation (§17), the run snapshot with a revision counter (§16.5), and the run report.
- Repo-profile values (`modules`, `hub`, `source`, `check`, `single_test`, `test_passed`, `setup`, `env`) from a `[profile]` table in the plan file, over `[orchestrator.profile]` in `config.toml`.
- The anthrex MCP server (`anthrex mcp`) with `task_done` and `task_blocked` for workers and `submit_review` for reviewers.
- Headless sessions for workers and reviewers on both runtimes:
  - Claude: `claude -p` in stream-json mode, one process per session.
  - Codex: `codex exec --json`, then `codex exec resume` for each later turn.
  - Read-only reviewers, a scrubbed environment, flags re-passed on every resume, and the `[orchestrator.claude] auth` switch.
  - Each session is registered as a headless window (`WindowKind::Headless`) and fed into milestone 6.5's conversation model.
- Stream parsers for both runtimes: turn start and end, rate-limit retries, permission denials, tool use, compaction and usage. They are tested against fixtures recorded from the installed CLIs. `SubagentStart`/`SubagentStop` hook pairing for Claude sessions (hooks still run in `-p`).
- `anthrex run start|status|approve|reject|edit|retry|override|cancel|resume|accept|discard`, and the hidden `anthrex mcp`.
- `fake-agent` additions: headless modes for both runtimes that read stream-json input and emit recorded-shape events, per-role scripts, working `mcp_call`, `read_message`, `sh`, `capture`, and argument templating.
- End-to-end tests for every scenario of §21 that belongs to this milestone, and the M8a-level half of the others (see "Scenario map" in the Tasks section), plus one smoke stage.

Out, each with the milestone that owns it:

| Out | Owner |
|-----|-------|
| Deciders and `ANTHREX_DECIDER_BIN`, triage, the fast path, the size cross-check (§7.2 rule 5), the decider check summary (a deterministic 40-line tail stands in), classification of free-text `task_blocked` reasons (a missing `kind` becomes `question`) | M8b |
| The repo profile file, `.anthrex/profile.toml`, the onboarding scout, profile re-proposal | M8b |
| The output-filter `PreToolUse` hook, OTLP metering of the orchestrator's PTY session, `history.jsonl`, `anthrex run stats` (per-turn usage of headless sessions and token budgets are in this milestone, decision 40) | M8b |
| The `C-b T` run view, `NodeKey::{Run, Planner, Scout, Task, AgentRound}`, the run inspector, run rows in the sidebar, Enter on an agent node opening its conversation (§16.4) | M8c |
| The orchestrator's PTY window and its hook gaps (`StopFailure`, `PreCompact`, `PostCompact` in `launch::claude::HOOK_EVENTS`, the PTY status machine's idleness) | M9 |
| The orchestrator window and contract, sub-planners, scouts, `get_context`, `edit_plan`, `spawn_scout`, `spawn_subplanner`, `run_status`, `task_result`, `submit_epic`, steering chat, the per-epic integration review of the large path, research and review task kinds | M9 |
| Adaptive concurrency (§13 item 9), threshold and budget refit, racing, the test-writer-then-implementer pattern | M9.5 |

Also out: a merge queue wider than 1, stub-then-fill, rebasing task branches, pruning old run records (follow-up), automatic conflict resolution (the engine never resolves a conflict; decision 36's hand-back asks the worker).

## Design decisions

Numbered and final. If one proves wrong or impossible, stop work on it, record the evidence under "Implementation notes", and continue with the tasks that do not depend on it.

### Structure

1. **Engine inside the daemon**, module tree `crates/daemon/src/run/`. No new crate: it needs the window manager, the git runner, the registry and the data directory, which all live in the daemon. *(Refreshed M8 decision 1.)*
2. **A pure reducer.** The engine is `run::engine::step(state: EngineState, event: Event) -> (EngineState, Vec<Effect>)`; `Event` carries the clock (`Event { now, kind }`), so the reducer reads no clock, spawns nothing and touches no file. These files are pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or `std::time::SystemTime`: `run/plan.rs`, `run/validate.rs`, `run/globs.rs`, `run/roster.rs`, `run/edits.rs`, `run/model.rs`, `run/engine/**`, `run/contract.rs`, `run/messages.rs`, `run/report.rs`, `run/role_launch.rs`, `run/snapshot.rs`, `run/env.rs`, and in the new `crates/daemon/src/headless/` module `argv.rs`, `claude_stream.rs`, `codex_stream.rs`, `conversation.rs` and `status.rs`. These do I/O: `run/git/**`, `run/exec.rs`, `run/proof.rs`, `run/journal.rs`, `run/reconcile.rs`, `run/driver.rs`, `run/driver/*.rs`, `headless/session.rs`. `headless/` sits outside `run/` because M9's scouts, sub-planners and deciders reuse it. `RunService` (in `driver.rs`) executes effects and feeds results back as events. *(Spec §23 first risk; refreshed M8 decision 2.)*
3. **Protocol.** `proto::PROTO_VERSION` becomes one above the value on `main` when this milestone starts (header). Every run message is nested in one new variant on each side, `ClientMsg::Run(RunRequest)` and `DaemonMsg::Run(RunReply)`, with the enums in a new `crates/proto/src/run_wire.rs`, so `messages.rs` (506 lines at `6f22681`) grows by two variants. The agent role enum is `proto::AgentRole` because M6.5 re-exports `proto::Role`; the model strength enum is `proto::Strength` because the spec's route is `{runtime, model, strength, effort}` (§9, §18). The refreshed M9 brief's `proto::Role` and `proto::Tier` map to these names. `types.rs` gains `proto::WindowKind { Pty, Headless }` on `WindowInfo.kind` (decision 49). `messages.rs` gains `HookSource::Stream`, which marks a hook the daemon synthesised from a session's stream (decision 27); `hooks::accepts` returns `false` for it, so a client that sends one over the socket is ignored. *(AGENTS.md rule 4; spec §19.)*
4. **MCP server crate.** New library crate `crates/mcp`, package `anthrex-mcp`, library name `mcp`; the hidden `anthrex mcp` subcommand in `crates/cli` calls `mcp::serve_stdio`. `rmcp` stays out of the daemon and the TUI. *(Refreshed M8 decision 4.)*
5. **`rmcp` pinned** at `=3.4.0` with `default-features = false, features = ["server", "transport-io"]`, a hand-written `ServerHandler` and hand-built tool schemas, exactly as the refreshed M8 brief's decision 5 verified on 2026-09-18; M8a.1 re-verifies the API. *(Refreshed M8 decision 5.)*
6. **Kinds executed in this milestone.** `TaskKind` has all four values of §5.1 (`code`, `docs`, `research`, `review`) so M9 needs no protocol change, but plan validation rejects `research` and `review` with `task <id>: kind: <kind> tasks are executed from milestone 9; use code or docs`. They need scouts and reviewer-only pipelines that are M9's. *(Spec §5.2, §21.)*

### Plans and validation

7. **The plan file is TOML**, parsed with the `toml` crate into `proto::Plan` with `deny_unknown_fields` at every level. Shape (every key not marked required is optional):

   ```toml
   goal = "Add password reset"           # required
   max_writers = 4                       # 1..=8, else config (default 3)
   max_readers = 2                       # 1..=8, else config (default 3)
   max_bounces = 3                       # 1..=5, else config (default 2)

   [profile]                             # each key overrides [orchestrator.profile] from config.toml
   modules = ["crates/*"]
   hub = ["crates/proto/**"]
   source = ["crates/*/src/**"]
   check = "cargo test --workspace"
   check_timeout_secs = 1800             # 10..=14400
   single_test = "cargo test --workspace -- --exact {test}"
   test_passed = 'test {test} \.\.\. ok'
   setup = "cargo fetch"
   generated = ["Cargo.lock"]              # rewritten by builds; outside owns it is a rung-1 bounce (decision 55)
   protected = ["docs/agents/**"]           # added to the built-in agent-config list (decision 56)
   [profile.env]
   CARGO_TARGET_DIR = "{worktree}/target"

   [[task]]
   id = "t1"                             # required, ^[a-z0-9][a-z0-9-]{0,15}$, not "integration"
   title = "Reset token model"           # required
   kind = "code"                         # code | docs; default code
   size = "M"                            # S | M | L; required
   interface_change = false              # the planner's flag for §7.2 rule 2; default false
   test_mode = "tdd"                     # tdd | check | none; default tdd for code, none for docs
   test_mode_reason = ""                 # required unless tdd
   owns = ["crates/auth/src/token.rs"]   # required, at least one glob
   deps = []
   priority = 0                          # i32, higher first among equals
   brief = "..."                         # required
   acceptance = ["..."]                  # at least one item
   test_to_write = "token::expires_after_one_hour"
   epic = "auth"                         # stored for M9, unused here
   scout_refs = []                       # stored for M9, unused here
   [task.route]                          # every key optional; policy fills the rest (decision 8)
   runtime = "claude"
   model = "claude-sonnet-5"
   strength = "standard"
   effort = "high"                       # policy's M default is medium; the planner's value wins
   [task.budget]                         # optional; default by size (decision 40)
   tool_calls = 120                      # the M default is 150
   minutes = 45                          # the M default is 60
   tokens = 3000000                      # optional; no default, enforced only when set
   ```

   Every value the example sets differs from the default it overrides, and no two same-typed limits share a value, so a test built from it can tell the plan's value from policy's and one limit from another (see "Brief refresh against main (6f22681)").

   Profile values are resolved per key: the plan's `[profile]` key when present, else `[orchestrator.profile]`, else empty (`check_timeout_secs` defaults to 1800, the spec's 30 minutes). `{worktree}` in an `env` value is replaced by the absolute path of the worktree the process runs in. *(Spec §6, §11.3, §18.)*
8. **Route and budget resolution** (policy fills whatever the planner left out; the planner's value always wins when valid). Runtime: the task's, else `orchestrator.default_runtime` (default `claude`). Strength and effort by class: S → `standard`, `low`; M → `standard`, `medium`; hub → `frontier`, `high`. Model: the task's, which must be in the roster for that runtime and then fixes the strength (a given `strength` that disagrees is an error); else the first roster entry for the runtime at the strength. Budget: the task's, else by class (decision 40). The resolved route is stored as `proto::Route { runtime, model, strength, effort }`, `model == ""` meaning "omit `-m`/`--model`". *(Spec §9 table and "policy fills whatever it leaves out".)*
9. **Size rules, §7.2**, applied to every task on every validation, only ever raising a size, each raise recorded as a note on the task (`size raised from S to M: owns spans 2 modules (rule 7.2.1)`):
   1. `owns` spans more than one module → at least M.
   2. `owns` spans more than one module and `interface_change = true` → L.
   3. `owns` touches a hub glob → at least M, and `hub = true`.
   4. A task left at L is an error: `task <id>: size: L tasks are never executed; split the task (rule 7.2.4)`.

   **Modules.** Each `profile.modules` pattern has a component count `k` (`crates/*` has 2). For each `owns` glob, take its literal prefix (decision 11); if it has at least `k` components and its first `k` match the pattern component-wise (a `*` component matches any one component), its module is those `k` components joined with `/`; if it has fewer than `k` components but is a component-prefix of the pattern's literal part, it spans more than one module; otherwise its module is `.` (outside every module). With no `modules` configured, every glob's module is `.`. A task spans more than one module when its globs yield two or more distinct modules or any glob spans more than one. Rule 5 (the decider's cross-check) is M8b. *(Spec §7.2; `interface_change` is not in §18's struct but rule 2 needs the flag, so it is a task field.)*
10. **Test-mode rules, §8.** A `code` task defaults to `tdd`, a `docs` task to `none`. Any non-`tdd` mode needs a non-blank `test_mode_reason` (`task <id>: test_mode_reason: required when test_mode is check or none`). A `code` task whose `owns` intersect `profile.source` cannot be `none` (`task <id>: test_mode: a code task whose owns touch the profile's source globs cannot be none (rule 8.1)`). A hub `code` task is forced to `tdd` (note `test mode forced to tdd: hub task (rule 8.2)`). A `tdd` task with an empty `profile.single_test` becomes `check` with its review raised one level (note `test mode check: the profile has no single_test (rule 8.3)`). *(Spec §8.)*
11. **`owns` semantics.** Two helpers in `run/globs.rs`, both pure:
    - **Intersection** (scheduling, hub, source and cross-runtime rules) is the refreshed M8 brief's conservative literal-prefix test: a glob's literal prefix is its path components before the first component containing `*`, `?` or `[`; two globs intersect when either prefix is a component-prefix of the other. `**/*.rs` intersects everything; two empty lists intersect nothing.
    - **Matching** (the spill check of decision 32) uses `globset` with `literal_separator(true)`. An `owns` entry with no glob metacharacter matches the path itself and every path below it, so `crates/auth` covers `crates/auth/src/lib.rs`. A trailing `/` is ignored.

    Absolute globs and globs with a `..` component are rejected. **Runtimes:** two tasks on different runtimes whose `owns` intersect are rejected on the later one (`task t2: owns: overlaps task t1's owns (crates/proto/**) and the two tasks run on different runtimes (claude, codex) (rule 9)`). *(Spec §9 "runtime spreading", §13 item 5; refreshed M8 decision 23.)*
12. **Graph rules, §12.1.** Every `deps` entry names a task; no cycles (`deps: cycle t1 -> t2 -> t1`, reported once, starting from the first id in plan order); no dependency on a cancelled task (`task t3: deps: t2 is cancelled`). For an edit made under `EditScope::Area { globs }` (M9's sub-planner scope, implemented and unit-tested here), each `owns` glob must lie inside the area: an area glob must be `<literal>/**` or a literal path, and an `owns` glob is inside when its literal prefix starts with the area's literal prefix (`task t4: owns: crates/tui/** is outside the area crates/daemon/**`). *(Spec §12.1.)*
13. **Plan edits, §12.1–§12.2.** One vocabulary, `proto::PlanEdit`: `add_task`, `split_task`, `cancel_task`, `amend_task` (brief, acceptance, route, test mode and reason, priority, size), `add_dep`, `answer`, `pause`, `resume`, `finish`. A batch is applied to a copy of the model and validated with every rule above over all tasks that are not `merged` or `cancelled`; any error rejects the whole batch with every error listed, and nothing changes. Per-state rules, each refusal naming the task and its state:
    - `add_dep`, `split_task`, and `amend_task` of `route`, `size` or `test_mode`: only on `pending`, `queued` or `blocked` tasks.
    - `amend_task` of `brief`, `acceptance` or `priority`: any unfinished task; on a task with a live worker the new brief and criteria are delivered as a message (decision 29).
    - `cancel_task`: any unfinished task; a live worker is killed, its worktree salvaged and removed (decision 20), and every task depending on it becomes `blocked(dep_cancelled)`.
    - `split_task { task_id, into }`: cancels `task_id` as above and inserts `into` at `task_id`'s position in plan order; every task that depended on `task_id` now depends on all of `into`.
    - `answer { task_id, text }`: on `blocked(question)` or `working`; delivers `[anthrex] Answer to your question: <text>` as the session's next turn and returns a `blocked(question)` task to `working` in the same session (decision 29 resumes an ended session to carry it).
    - `pause`, `resume`, `finish`: decisions 45 and 37.

    **L exemption.** Rule 7.2.4 applies to tasks the batch adds or amends; a task raised to L by rung 3 (decision 38) is exempt until an edit touches it, and `run retry` refuses an L task with `task <id> is L; split it first`. Without the exemption, every unrelated edit would be rejected after any rung 3. *(Spec §12.1, §12.2.)*
14. **The plan gate, §12.3, engine side.** `run start` validates, creates the run branch and the integration worktree **immediately** (not at approval), and returns at once with the run in `awaiting_approval`, unless `--yes`, which starts it and records `approved by --yes` in the report. While the gate is open the engine pre-warms: it prepares worktrees, with `setup` run, for up to `max_writers` tasks that have neither declared nor implicit dependencies, in dispatch order (decision 41), and starts no session. `anthrex run approve` moves the run to `running`; `anthrex run reject` discards it (decision 20's discard). `run edit` works while the gate is open; a pre-warmed worktree whose task is cancelled is removed. Nothing blocks a tool call or a client: the verdict is visible in the snapshot. `awaiting_approval` is persisted and **survives a restart unchanged** — it is not turned into `paused`, because nothing is running (conversation-view decision 15's intent). *(Spec §12.3, §5.3 step 3, §13 item 6.)*

### Runs and git

15. **Run id** is the slug of the refreshed M8 brief's decision 7 (lower-cased goal, non-alphanumerics to `-`, collapsed, trimmed, first 32 characters, `run` if empty, then `-` and 4 random lowercase hex digits), redrawn up to 5 times while `refs/heads/anthrex/<id>/integration` or `<data_dir>/runs/<id>` exists, then `could not pick a free run id`. The CLI accepts the full id, its 4 hex digits, or a unique prefix. *(Refreshed M8 decisions 6–7.)*
16. **Layout.** `root` and `project` come from one `project::detect_roots_with(git, dir, git_timeout)` (the injectable form of `detect_roots`, `project.rs:54`, so the recording git of M8a.8 sees it too): `root` is `DetectedRoots.worktree` (the checkout `dir` is in) and `project` is `DetectedRoots.project`. Preflight also records `git_common_dir` (`git rev-parse --path-format=absolute --git-common-dir` in `root`), which decisions 25 and 54 make writable; it is `<project>/.git` for an ordinary repository and never `<root>/.git` when `root` is a linked worktree. `<wt>` is `worktree::repo_worktrees_dir(<data_dir>/worktrees, project)`.

    | Thing | Branch | Path |
    |-------|--------|------|
    | Base | the branch checked out in `root` at start; commit `base_sha` | `root`, never written until accept |
    | Run (integration) | `anthrex/<run>/integration` at `base_sha` | `<wt>/runs/<run>/integration` |
    | Task | `anthrex/<run>/<task>` from the run head when dispatched | `<wt>/runs/<run>/<task>` |
    | Review | detached at the task head | `<wt>/runs/<run>/<task>.review` |
    | Proof | detached, scratch | `<wt>/runs/<run>/<task>.proof` |
    | Engine state | none | `<data_dir>/runs/<run>/run.json`, `journal.jsonl`, `REPORT.md` |

    A branch cannot be both `anthrex/<run>` and a parent of `anthrex/<run>/<task>` (git's directory/file ref conflict); `integration` resolves it and is why the task id `integration` is reserved. `.` cannot appear in a task id, so `<task>.review` and `<task>.proof` never collide with a task. Run sessions are registered as headless windows (decision 49) named `<h4>/<task>.w<session>` (workers) and `<h4>/<task>.r<round>` (reviewers), where `<h4>` is the run id's 4 hex digits. Every one counts toward `max_windows`, and a task that would exceed it is `blocked(environment)` with `run window limit (<n>) reached`. *(Spec §11.5 "Branch names", §11.6.)*
17. **Start preflight**, in order, each failure an error from `run start`: `root` resolves (`not a git repository: <dir>`); `git version` is at least 2.38 (`anthrex runs need git 2.38 or newer for merge-tree --write-tree (found <v>)`); `HEAD` is on a branch (`<root> is on a detached HEAD; check out a branch first`); `HEAD` has a commit (`the repository has no commits yet`); `git var GIT_COMMITTER_IDENT` succeeds (`git has no user.name/user.email configured for <root>`); `git status --porcelain --untracked-files=no` in `root` prints nothing (`the working tree at <root> has uncommitted changes; commit or stash them first`); and, only when decision 53 applies, no untrusted project settings. Preflight also lists the tracked files at `base_sha` that match `profile.protected` (decision 56), for the plan warnings. *(Refreshed M8 decision 9; §11.5 needs merge-tree.)*
18. **Git runner.** Every git command in `run/git/**` goes through `crate::worktree::run_git` with the program injected as `git: &OsStr` (so tests can pass a recording script), a deadline of `now + git_timeout` (`orchestrator.git_timeout_secs`, default 60), on `spawn_blocking` — AGENTS.md rules 10 and 11 hold by construction because `run_git` adds `--no-optional-locks` and `run_captured` scrubs the environment. Additionally:
    - Every engine **write** in an engine-owned worktree (worktree add/remove/lock/unlock, checkout, `commit-tree`, `update-ref`, the hand-back merge, salvage) passes `-c core.hooksPath=/dev/null -c commit.gpgSign=false` so a user's hooks or signing pinentry cannot hang a run. `run accept`, the only write into the user's own checkout, passes neither.
    - **One write queue per repository** (keyed by `project`): writes take the repository's `tokio::sync::Mutex` before `spawn_blocking`; reads (`rev-parse`, `status`, `diff`, `merge-tree`) do not. A write whose stderr contains `.lock': File exists` or `Unable to create` and `.lock` is retried up to 5 times after 200, 400, 800, 1600 and 3200 ms.
    - **Worktree lock.** Every task and integration worktree is `git worktree lock --reason "anthrex run <run>"` right after creation and `git worktree unlock`ed right before removal.
    - Session launches and resumes are jittered by `jitter_ms = 100 + (fnv1a(run, task, session) % 400)` — deterministic, so tests are unaffected. *(Spec §17 "Git safety".)*
19. **Task branches start from merged work.** A task's worktree is created when it is dispatched — after every dependency (declared and implicit) is `merged` — with `git worktree add -b anthrex/<run>/<task> <path> <run_head>`; `start_commit` is `run_head` at that moment. An existing branch and path are reused (resume); an existing branch without its path is re-added. Pre-warmed worktrees (decision 14) are created from `base_sha`; at dispatch, a pre-warmed branch that still has no commit of its own and whose start is no longer the run head is re-pointed with `git checkout -B anthrex/<run>/<task> <run_head>` in its worktree (setup is not re-run), so no task ever starts from a stale base. *(Spec §11.6 "dependents start from merged work".)*
20. **Salvage and cleanup.** Before any engine-owned worktree is removed — task cancel, task merged, review round done, proof done, run discard, run accept — the engine checks `git status --porcelain` (untracked included, ignored excluded). If dirty: `git add -A`, `git write-tree`, `git commit-tree <tree> -p HEAD -m "anthrex salvage <run>/<task>"`, `git update-ref refs/anthrex/salvage/<run>/<task>/<seq> <commit>` (`<seq>` counts from 1 per task), and the ref is recorded on the task and in the report. Then unlock and `git worktree remove --force`. A worktree is never deleted dirty without a salvage ref. Cleanup on merge removes the task, review and proof worktrees; the task branch is kept until accept or discard. **Discard** (and `run reject`): remove every run window, salvage-and-remove every worktree, `git worktree prune`, delete every `refs/heads/anthrex/<run>/*` branch, keep every `refs/anthrex/salvage/<run>/*` ref and the run's data directory, state `discarded`. **Accept**: the refreshed M8 brief's decision 14, restated here so this brief stands alone: the base branch must be checked out in `root` (`check out <base> in <root> first (currently <branch>)`), the tracked tree must be clean (decision 17's message), then `git merge --no-ff --no-edit -m "anthrex: accept run <run>: <goal>" anthrex/<run>/integration` in `root` (`--no-edit` is required, or git opens an editor and the command sits until its timeout), then salvage-and-remove every run worktree and delete the run's branches; state `accepted`. Both need `confirm == Some(<run id>)`, else `RunReply::ConfirmNeeded`. **A moved base at accept** (settled 2026-09-22, spec §22 item 5): before the merge, the driver reads `refs/heads/<base>` on `spawn_blocking`.
    - Equal to `base_sha`: as above.
    - Advanced (decision 21's classification): `ConfirmNeeded` carries `base_moved: Some(BaseMovedInfo { from, to, commits, total })`, where `commits` is `run::git::commits_since(root, base_sha, to, ACCEPT_LIST_MAX)` — `git log --format='%h %an: %s' <base_sha>..<to>` (the `--oneline` form with authors), newest first, at most `ACCEPT_LIST_MAX` = 50 lines — and `total` is the full count. For this case accept needs `confirm == Some("<run id>@<to>")` (the full sha of the base head that was listed), so a base that moves again between the prompt and the answer is listed again, never merged unseen. `Accept` then merges onto the current base head, with `expected_base = to`: a base head that differs from `expected_base` inside the op is `Failed { message: "the base branch moved again; run accept again" }`.
    - Rewritten (`base_sha` not an ancestor): accept is refused with `refs/heads/<base> was rewritten since the run started (<old7> is not an ancestor of <new7>); merge anthrex/<run>/integration by hand, or discard the run`.
    - **A conflict at accept** is possible only against an advanced base. `git merge` exits non-zero with unmerged paths; the op runs `git merge --abort`, so `root` and the base branch are exactly as they were, and returns `AcceptConflict { files }`. The run stays `complete`, its branches and worktrees are kept, and the reply is `Refused` with `accept conflicts with <n> commits on <base>: <files>; resolve by merging anthrex/<run>/integration into <base> yourself, or discard the run`. The report's log records it. The spec's `refs/anthrex/salvage/<run>/<task>` becomes `…/<task>/<seq>` because one task can be salvaged more than once, and a bare `…/<task>` ref would conflict with its children. *(Spec §11.6 "Cleanup", §12.2, §17.)*
21. **Ref guard.** The run records `base_sha` and `run_head` (updated on every CAS). Before every merge candidate (inside the op, before `merge-tree` and again before `update-ref`) and before `complete`, `run::git::guard_refs` reads both refs and classifies them (`RefCheck`, Interfaces):
    - **Run ref.** `refs/heads/anthrex/<run>/integration` must equal `run_head`; only the engine writes it. Otherwise the run becomes `halted` with `halted_reason` `refs/heads/anthrex/<run>/integration moved from <old7> to <new7>`: no dispatch, no merge, windows keep running.
    - **Base ref, advanced.** `refs/heads/<base>` differs from `base_sha` and `git merge-base --is-ancestor <base_sha> refs/heads/<base>` succeeds: someone committed on the base branch. This is **not** a halt. The run keeps building on its recorded `base_sha`; the op goes on, and the driver sends `Event::BaseAdvanced { run_id, to, commits }` (`commits` from `git rev-list --count <base_sha>..<to>`). The reducer sets `Run.base_moved = Some(BaseMoved { from: base_sha, to, commits, seen_at: now })` (a later, different `to` replaces `to`, `commits` and `seen_at`; the same `to` changes nothing, not even the revision), logs it, and the snapshot carries the attention line `base <base> moved from <from7> to <to7> (<commits> new commits); accept will list them`. Dispatch and merges continue. Decision 20's accept lists the new commits for confirmation.
    - **Base ref, rewritten.** `refs/heads/<base>` differs from `base_sha` and `base_sha` is **not** an ancestor of it (history rewritten, or the branch deleted): accept could no longer be a clean merge onto what the run was built from, so the run becomes `halted` with `halted_reason` `refs/heads/<base> was rewritten: <old7> is not an ancestor of <new7>` (or `refs/heads/<base> was deleted`).
    - `anthrex run resume <run> --rebaseline` records the current values of both refs (`base_sha` becomes the base head, `base_moved` is cleared) and returns the run to `running`; without `--rebaseline`, resume of a halted run is refused with the reason. The optional `reference-transaction` hook is not installed. *(Spec §17 "The run branch is guarded; the base branch is watched"; settled 2026-09-22, spec §22 item 5.)*
22. **Run worktrees are watched.** Task and integration worktrees are registered with `crate::manager::GitRoots::register` when created and unregistered before removal (never review or proof worktrees). `GitRegistry` construction moves from `server::serve` to `lifecycle::run` so `RunService` and the server share one (new `server::GitWiring`). With `ANTHREX_GIT=off` every call is a no-op and runs still work. *(Refreshed M8 decision 15.)*

### Agents and signals

23. **Roster with strength.** `proto::ModelEntry { runtime, model, strength, note }`. The built-in roster, in order: `claude`/`claude-haiku-4-5`/`fast`, `claude`/`claude-sonnet-5`/`standard`, `claude`/`claude-opus-5`/`frontier`, `codex`/`""`/`standard`. `model == ""` is allowed only for Codex and means "use Codex's configured default". `[[orchestrator.models]]` entries (`runtime`, `model`, `strength`, `note`) replace a built-in with the same `(runtime, model)` in place and append otherwise; `builtin_models = false` starts from nothing; an empty result falls back to the built-ins with a problem. Strength orders `fast < standard < frontier`. *(Spec §9 "strength"; research P1-4.)*
24. **Claude headless sessions.** One long-lived process per session. `headless::argv::claude_args` builds its argv, in this order:
    - `-p --input-format stream-json --output-format stream-json`, then `--verbose` when M8a.1 finds stream-json output requires it.
    - `--permission-prompts none`. If M8a.1 finds the flag absent, a worker's `--permission-mode` below becomes `dontAsk` (which denies anything not allowed) and a reviewer keeps `plan`.
    - `--session-id <uuid>` for a new session, or `--resume <session id>` for a resumed one.
    - The user-settings-only flags of decision 53 (`CLI_CAPS.claude_user_settings_only`), on every launch and every resume.
    - `--settings <json>`: `headless::argv::claude_settings`, which is M3's hook settings (`launch::claude::settings`, unchanged) plus, for a worker, decision 54's sandbox block. Hooks still run in `-p` (§14.3), and `SubagentStart` / `SubagentStop` are needed from them (decision 27).
    - `--mcp-config <json>` and `--allowedTools <list>`, then `--append-system-prompt <contract>`.
    - `--permission-mode <mode>`, then `--model <model>` when the route names one, then `--effort <low|medium|high>` when M8a.1 confirms the flag.
    - The session's authentication flag (decision 50).

    Per role:
    - **Worker:** `--allowedTools` is `mcp__anthrex__task_done`, `mcp__anthrex__task_blocked` and every `orchestrator.worker_allowed_tools` entry. The default is `Bash,Edit,Write,Read,Glob,Grep,Agent,TodoWrite`, because nothing can grant a permission at run time. `--permission-mode` is `orchestrator.worker_permission_mode`, default `acceptEdits`.
    - **Reviewer:** `--allowedTools mcp__anthrex__submit_review,Read,Glob,Grep,Bash(git diff:*),Bash(git log:*),Bash(git show:*)`, and `--permission-mode plan`. The three `Bash` patterns let it read history beyond the diff in its prompt (decision 35); every other command, a build or test run included, is denied, because nobody can answer a prompt.

    Other rules:
    - The prompt is never on the argv. The first turn is a stream-json user message written to stdin (decision 29).
    - `--mcp-config` is `{"mcpServers":{"anthrex":{"type":"stdio","command":"<exe>","args":[…headless::argv::mcp_args…]}}}`. `--mcp-config` and `--allowedTools` are variadic, so each is followed by another flag, which this order guarantees.
    - `<uuid>` is `role_launch::session_uuid(run_id, op_id)`: a version-4-shaped UUID built from two FNV-1a 64-bit hashes of the run id and the `CreateWindow` op id. It is deterministic, so the reducer stays pure, and a re-issued op gets a new id, so it never collides with a half-created session.
    - **If M8a.1 finds no `--effort` flag**, effort is recorded on the round and not passed. Rung 2 for a Claude worker already at `high` then goes straight to the peer runtime (decision 39).
    - **If M8a.1 finds that plan mode blocks the allowed `submit_review` call in `-p`**, reviewers use `--permission-mode dontAsk --disallowedTools Edit,Write,NotebookEdit` instead, with the same `--allowedTools` (under `dontAsk` a `Bash` command outside the three patterns is denied, and disallowing `Bash` outright would override them). Record which in "Implementation notes".

    *(Spec §4 "Only the orchestrator is an interactive terminal", read-only launch, §4.2, §19.)*
25. **Codex headless sessions.** One `codex exec` process per turn. `headless::argv::codex_args` builds it:
    - The first turn: `codex exec --json`.
    - Every later turn: `codex exec resume <session id> --json`.
    - Then, on both, in this order:
      - `-c mcp_servers.anthrex.command=<toml exe>`, `-c mcp_servers.anthrex.args=<toml array>`, `-c mcp_servers.anthrex.tool_timeout_sec=120`, `-c mcp_servers.anthrex.default_tools_approval_mode="approve"`. Not `"auto"`: against codex-cli 0.156.1, `"auto"` together with `approval_policy="never"` fails every call to the server with `MCP tool call requires approval, but approval policy is never`, under both `-s read-only` and `-s workspace-write`, and `"approve"` completes it (`crates/daemon/tests/fixtures/headless/codex-0.156.1-mcp-approval-auto.jsonl` and `-approve.jsonl`; ruling T7-C1, M8a.7 fix round 1). The key is scoped to the `anthrex` server, so it approves only anthrex's own tools.
      - `-c developer_instructions=<toml contract>`, `-c model_reasoning_effort=<toml effort>`, `-c approval_policy="never"`.
      - `-s <sandbox>`.
      - For a worker, `-c 'sandbox_workspace_write.writable_roots=[<toml git common dir>]'`.
      - `-m <model>` when the route names one, then `--`, then the turn's message as the last argument.
    - A worker's sandbox is `orchestrator.worker_codex_sandbox`, default `workspace-write`. A reviewer's is always `read-only`. Under `-s read-only` with `approval_policy="never"` a Codex reviewer can still run `git diff`, `git log` and `git show` (they only read; `git diff` skips its optional index refresh when it cannot write), but a build or test run fails on its first write, so `REVIEWER_CONTRACT` item 3 holds for both runtimes.
    - The writable root is needed because a linked worktree's index, refs and objects live under the main repository's git directory, outside the worktree. Without it, every commit fails in the sandbox.
    - The session id is the `thread_id` of the first turn's `thread.started` event.
    - M8a.1 verifies that `exec resume` accepts the same `-c`, `-s` and `-m` options. If `-s` is refused on resume, the sandbox goes through `-c sandbox_mode=…` instead. Every TOML string comes from `launch::codex::toml_string`.

    No hook configuration is passed. A Codex headless window gets its conversation and status from the stream alone (decision 27), so a hook that did fire in `exec` could not double-count. *(Spec §4, §4.2 "approvals set to never, inside its sandbox"; refreshed M8 decisions 33–34.)*
26. **Agent environment.** `headless::session::spawn` starts every session process with `tokio::process::Command`, set up as follows:
    - Its own process group and stdin, stdout and stderr all piped.
    - Every inherited variable starting with `CLAUDE_CODE_`, plus `CLAUDECODE`, removed with `env_remove`.
    - `ANTHREX_WINDOW_ID` and `ANTHREX_SOCKET` set to the session's own window and socket, exactly as `launch::plan` sets them for a PTY window (`launch/mod.rs`), because `anthrex hook` reads both.
    - The profile's `env` set, with `{worktree}` substituted.

    Engine commands (`setup`, `check`, proof runs) get the same removal and the same profile env, and also lose `ANTHREX_WINDOW_ID`: they belong to no window, and an inherited value (a daemon started from inside an anthrex window) would make an `anthrex hook` in a check report to the wrong window. PTY windows and `launch::LaunchPlan` are unchanged. The follow-up from M6.5 calls the general policy a product decision, and this milestone applies it only to the unattended sessions that need it most. *(Spec §17 "Scrubbed agent environment", §6 `[env]`.)*
27. **Stream signals, not terminal inference.** Each line of a session's stdout is parsed, purely, by `headless::claude_stream::parse_line` or `headless::codex_stream::parse_line` into zero or more `headless::SessionEvent`s. The Interfaces tables give the exact mapping; M8a.1's recorded fixtures are its test inputs. A line that does not parse, or has an unknown type, yields `SessionEvent::Unknown` and is kept in the window's last-10-lines ring for diagnosis; it never ends a turn.
    - **Driver events.** The session driver adds `ProcessExited { code, signal }` and `StderrLine`.
    - **Status.** `headless::status::next(state, &event)` updates the window, keeping these rules apart from the PTY status machine:
      - `Starting` until the first event.
      - `Working` while a turn is open.
      - `Attention` while a rate-limit retry is pending, or after a turn failed.
      - `Idle` between turns.
      - `Exited` once the session has ended.

      `WindowInfo.tool` is the latest top-level tool name; `WindowInfo.session_id` comes from `Init`.
    - **The engine feed.** The manager publishes `WindowManager::signals() -> broadcast::Receiver<WindowSignal>`. Every session event is sent as `WindowSignalKind::Session(event)`. For a headless window, every parsed hook of kind `SubagentStart` or `SubagentStop` is also sent as `WindowSignalKind::Hook`. `launch::claude::settings` is reused unchanged, so its 10 events are what fire; compaction comes from the stream's `compact_boundary`. `broadcast::Sender::send` never blocks, so it may be called under the manager lock.
    - **What each round tracks** from that feed:
      - `turn_open`, set by the reducer when it delivers a turn and by `TurnStarted`, and cleared by `TurnEnded`.
      - `last_event`, the time of any event.
      - `tool_calls`: every `ToolUse`, sub-agents' included.
      - `rate_limited_until`, set by `ApiRetry` from `now + delay_ms`, or by a turn that ends failed on a rate limit to the time its continue is due (decision 32), and cleared by any other event.
      - `open_subagents`, a set keyed by `agent_id`: Claude's `SubagentStart` inserts and `SubagentStop` removes. Codex reports none.
      - `denials`: every `PermissionDenied`, plus the count in a `TurnEnded`'s `permission_denials` that was not already seen.
      - `usage`, summed from each `TurnEnded`.
    - **Lagged or silent.** A lagged receiver logs a warning and continues. Counts may then be low, which budgets tolerate. Because no event ever arrives without a line of output, a silent stream is the stall signal (decision 32).
    - **The conversation mapping.** `headless::conversation::map(runtime, hooks_fire, &event, &mut StreamCursor) -> ConversationInput { hooks: Vec<ParsedHook>, records: Vec<transcript::Record> }` turns each event into milestone 6.5's two inputs. Under the same lock, the manager applies each hook through the existing `WindowManager::conversation_hook(window_id, entry, &hook, now)` (`manager/conversation.rs:355`, which resolves the sub-agent spawn origin and calls `ConversationSet::on_hook`) and the records with `ConversationSet::enrich`, then notifies subscribers with `notify_conversations` as M6.5's reader does.
      - Claude's real hooks still arrive through `anthrex hook`, and M6.5 builds the timeline from them as it does for any window. So for Claude, `hooks_fire` is true and `map` returns records only.
      - For Codex, and for Claude when M8a.1 finds that `UserPromptSubmit`, `PreToolUse` or `PostToolUse` do not fire in `-p`, `map` also synthesises the hooks, with `source: HookSource::Stream` and `session_source: None` (M6.5.10's resume trigger is for the transcript reader, which a headless window never runs).
      - A headless window never starts M6.5's transcript reader, because the stream carries everything the transcript would. On `main` the reader is started by `subscribe_conversation` (`manager/conversation.rs:141–150`, through `transcript::parser_for(runtime)`) for any Claude or Codex window, and the same function sets `DegradeReason::NoTranscriptPath` when the root has no `transcript_path` (`:136–140`). Both are skipped for a `Headless` window, or a headless Claude session whose `SessionStart` names a transcript would be read twice, and one whose hooks name none would show "timeline only" for a complete stream.
      - Real hooks for a headless window update its conversation and sub-agent rows, never its status.
      - `WindowManager::tick`'s quiet rule (`manager/mod.rs:314–318`: a `Working` window with no output for `QUIET_AFTER` becomes `Idle`) skips `Headless` windows: their status comes from `headless::status::next` alone.

    *(Spec §4 "exact, structured signals", §11.1 "Turn ends are exact", §14.3, §16.2; research P1-2.)*
28. **Session persistence and resume.**
    - **What is persisted.** A headless window's `Entry` keeps a `headless::HeadlessSpec` (runtime, route, contract, tools, sandbox, env, the `RunRef`), serialized into the existing opaque `WindowRecord.run` together with `kind`. `WindowRecord.session_id`, which already exists, holds the runtime's session id, and so does the engine's `AgentRound.session_id` in `run.json`, which is the one the engine trusts. A value that fails to parse loads as a PTY record with a warning and is marked `Exited`. No `CreateWindow` from a client can create a headless window.
    - **After a daemon restart.** Every session process is gone: its pipes were the daemon's.
      - Reconcile (decision 44) first makes sure: for every persisted session with a recorded pid that is still alive, if `ps -o command= -p <pid>` contains that session's id, it sends `SIGTERM` to the process group, waits 2 s, then sends `SIGKILL`. It never signals a pid whose command line lacks the id. Two processes on one session would interleave its transcript.
      - The restored window is `Exited`, with `Process::Headless(HeadlessHandle::ended())`.
    - **Resume.** `anthrex run resume` resumes each live round's session with a hand-over message (`RESUME_WORKER` or `RESUME_REVIEWER`), rather than starting it over. The op is `OpKind::ResumeSession`:
      - Claude: `claude_args` with `--resume <id>` and every flag re-passed (resume restores neither `--settings` nor `--mcp-config`), then the message on stdin.
      - Codex: `codex exec resume <id>` with the message as its argument.
      - If the resume fails — the process exits before `Init`, or its first turn fails with a "no such session" error (M8a.1 records the text) — the task gets a fresh session at the same rung with no failure counted, using decision 30's hand-over prompt.

    *(Spec §17 "Headless sessions survive a daemon restart by resuming", "re-passed on every resume".)*
29. **Turn-based message delivery.** Every engine message to an agent — a bounce, an answer, an amendment, a nudge, a hand-over, a resume — goes into the run's persisted outbox. The reducer emits `Effect::Deliver` for a window only when all of these hold:
    - its round's turn is closed;
    - it has no pending interrupt;
    - it is not waiting out a rate-limit continue (decision 32).

    **What a delivery sends.** Every queued message for that window, joined in queue order by a blank line, as **one new turn**:
    - Claude: one stream-json user-message line written to the session's stdin through its writer thread (`headless::claude_stream::user_message`, the envelope M8a.1 pins). It never blocks a tokio worker, because a PTY-style full pipe is possible here too.
    - Codex: a new `codex exec resume` process with the text as its last argument.

    **Rules.**
    - The reducer marks the turn open when it emits the delivery.
    - `Delivered { ok: false }` puts the messages back at the head of the outbox and retries at the next `Tick` 5 s later; three failures in a row block the task as `blocked(environment)` with `could not deliver to the agent: <error>`.
    - A message queued while a turn is open waits for its `TurnEnded`. That is how an `answer` or an amendment reaches a busy worker (§12.2).
    - A message for a round whose session has ended — a Claude process that exited between turns, or any session after a restart — is carried by `Op ResumeSession` instead of `Deliver` (decision 28). If that resume fails, the task gets a fresh session whose prompt ends with the message.
    - `MESSAGE_MAX_BYTES` (32 KiB) and its head-and-tail clamp and the `[anthrex]` prefix are kept.
    - Delivery is at least once: a crash between the send and `Event::Delivered` repeats the message.
    - Bracketed paste, `SUBMIT_DELAY`, the settle bit, `REDELIVER_AFTER` and `MESSAGE_ATTENTION_AFTER` are gone; there is no terminal to paste into.

    *(Spec §4 "Follow-up messages are written to its stdin as user messages", §12.2; replaces refreshed M8 decisions 37–38.)*
30. **Contracts and prompts.** `WORKER_CONTRACT` and `REVIEWER_CONTRACT` (exact text in Interfaces) are the system prompt; they never vary, so the cached prefix is stable (§14.2). Prompt order: the role contract (system prompt), then in the first turn's message a profile summary (check command, single-test command, test mode rules), then the task brief last. Model and effort are pinned at launch and never changed within a session. A **fresh session** (rung 2, or any resume that failed) gets `worker_prompt` plus `This is session <n> of this task.`, the reason, `git diff --stat <start>..HEAD`, the diff clamped to 16 KiB, and the failure record (every earlier bounce message's text). *(Spec §14.2, §10 rung 2, §11.6 "Sessions hand over by branch".)*

### Gates

31. **States.** `RunState { AwaitingApproval, Running, Paused, Halted, Complete, Accepted, Discarded, Failed }`; `Accepted`, `Discarded` and `Failed` are terminal; `Failed` is only for a run whose integration worktree could not be created. `TaskState { Pending, Queued, Preparing, Working, Proof, Check, Review, MergeQueue, Merged, Blocked, Cancelled }` with `block: Option<BlockInfo { reason, text }>` when `Blocked`, `BlockReason { MisSized, Human, Conflict, DepCancelled, Question, Environment }`. `Pending`: waiting on dependencies. `Queued`: runnable, waiting for a slot. `Preparing`: worktree and setup. `Working`: a worker session is live. `Proof`, `Check`, `Review`, `MergeQueue`: the gates. *(Spec §11, §16.3 glyph list.)*
32. **The done gate, §11.1.**
    - **`task_done { summary, test?, red? }`** from the task's current worker in `working`. The engine runs `VerifyDone` (each git call bounded by the smaller of `DONE_CHECK_GIT_TIMEOUT` = 10 s and `git_timeout_secs`, six calls at most, so the reply beats `TOOL_REPLY_TIMEOUT` = 100 s) and rejects, leaving the task `working` and counting nothing, with exactly one of: `task_done rejected: the branch has no commit since the task started; commit your work first`; `task_done rejected: the tracked tree has uncommitted changes (<n> files); commit or revert them first`; `task_done rejected: a merge is in progress; finish it with git commit first`; `task_done rejected: untracked files inside this task's owns are not committed: <files>`; `task_done rejected: this is a tdd task; name the test (test) and the commit where it was added and failed (red)`; `task_done rejected: red <sha> is not a commit on this task's branch after its start commit`. The untracked-inside-owns rule is added by this brief: a forgotten `git add` would otherwise pass the task-worktree check and fail only on the merged candidate. A diff that changes files outside `owns` — computed as `git diff --name-only <run_head>...HEAD` (three dots: the task's own net change, which stays correct after a hand-back merge brings the run head in) — is accepted as a signal and sends the task to rung 3 (`changed files outside owns: <files>`) — except paths that match `profile.protected` or `profile.generated`, which decisions 56 and 55 turn into rung-1 bounces; a task the user has overridden (decision 35) is exempt from all three. Accepted: `Done recorded. The engine is running the gates now; stop and wait. If anything fails you will get an [anthrex] message.`
    - **`task_blocked { kind?, reason }`**: `kind` is `question`, `mis_sized` or `environment`, default `question` (M8b's decider classifies later). `question` → `blocked(question)`; `mis_sized` → rung 3; `environment` → `blocked(environment)`. Reply `Blocked recorded (<kind>). Stop and wait for an answer.`
    - **Turn-end fallback.** A worker's turn can end (`TurnEnded` with outcome `Completed`) while its task is still `working` and no `task_done` was accepted in that turn. If sub-agents are still open, the check waits until the last `SubagentStop`, or until the next `TurnEnded`. Then the engine counts commits (`Op CountCommits`):
      - With at least one commit, it queues `DONE_NUDGE`, which becomes the next turn. If that turn also ends without `task_done`, the task proceeds exactly as if `task_done` had been called with no `test` or `red`. A `tdd` task then fails its proof, with the message that names what is missing.
      - With no commit, it queues `NO_COMMIT_NUDGE`. If that turn also ends with no commit and no `task_done`, it is a stall (below).

      The task records `done_signal = task_done | turn_end_fallback` for the report.
    - **Stall watchdog.** A round with an open turn that has no stream event for `stall_after_secs` (default 600, measured from `last_event`) is stalled. While `rate_limited_until` is in the future the clock is suspended: it restarts from `rate_limited_until`, because the CLI is waiting out its own retry.
      - **First stall.** The engine emits `Effect::Interrupt`: the control request M8a.1 pins for Claude, else `SIGINT`, and `SIGINT` for Codex. It queues `stall_nudge`, which becomes the next turn once the interrupted turn ends.
      - **If the interrupt does not end the turn** within `INTERRUPT_GRACE` (30 s), the session is killed.
      - **Second stall.** Another `stall_after_secs` of silence in a later turn of the same session, or a kill after a failed interrupt, is a stall: rung 2 (decision 38).
    - **Rate limits and failed turns.**
      - A rate-limit retry (`ApiRetry` with error `rate_limit`) is never a turn end and never a stall.
      - **Rate-limit events.** The run's `rate_limits[runtime]` counts rate-limit events: the start of a retry streak (a streak ends at the next non-retry event) is one, and a turn that ends failed with a rate-limit error is one, whether or not retries preceded it. The only exception is a failed turn that arrives while its round is still in a retry streak (`in_retry_streak`): the streak ran straight into the failure, so that is the same event and is not counted again. So a failed rate-limit turn with no retry before it, or after a streak that already ended, is counted, and M9.5's adaptive concurrency sees it.
      - A turn that **ends** failed with a rate-limit error (Claude's `result` with `is_error` and a rate-limit error, Codex's `turn.failed` whose message M8a.1 records) waits `rate_limit_retry_secs` (default 300), with the round's `rate_limited_until` set to the end of that wait, so the round reads as rate-limited (`AgentRoundInfo.rate_limited`) while it waits. Then `rate_limit_continue` is queued as a new turn. It is not a failure.
      - A turn that fails with `authentication_failed` or `billing_error` blocks the task as `blocked(environment)`, with the error.
      - Any other failed turn gets one `rate_limit_continue` after the same wait. A second non-rate-limit failed turn in a row blocks the task as `blocked(environment)`.
    - **Permission denials.** A worker never waits on a prompt, because none can be shown. `denials_before_block` (default 3) denials in one session block the task as `blocked(environment)` with `the agent was denied <n> times; last: <tool>: <reason>`, and the session is killed. Codex reports denials only if M8a.1 finds a structured marker in `exec --json`; otherwise its sandbox refusals surface as failed commands, and the stall and budget rules cover them.
    - **A process that dies.** A session process that exits without the engine killing it:
      - During an open turn, or for Codex before its turn's `turn.completed` / `turn.failed`: the first time in a round, the engine resumes the session (`ResumeSession` with `RESUME_AFTER_EXIT`). The second time in the same round, it is a stall.
      - A Claude process that exits between turns is marked ended. The next delivery resumes it.
      - A Codex process that exits 0 after its `turn.completed` is the normal end of a turn.
    - Deadlines are fields of the persisted round (unix seconds). *(Spec §11.1; §4.2 "A task whose worker keeps hitting denials is blocked(environment)"; research P1-1, P1-3.)*
33. **Test proof, §8.1.** For a `tdd` task after `task_done`: in the proof worktree (created on first use with `setup`), `git checkout --detach --force <red>` and `git clean -fd`, run `single_test` with `{test}` replaced by `launch::shell_quote(test)`; it must exit non-zero. Then `git checkout --detach --force <head>`, `git clean -fd`, run again; it must exit 0 **and** its output must match `test_passed` with `{test}` replaced by `regex::escape(test)`. Both runs use `check_timeout_secs`. A failure is a gate failure of `proof` (decision 38) with `proof_failed_message`, which quotes the command and the last 40 lines of the offending run. A red commit from an earlier session of the same task is valid (sessions hand over by branch). *(Spec §8.1, §23 second risk.)*
34. **Check, §11.3.** `/bin/sh -c "{ <check>\n} 2>&1"` in the task worktree, own process group, stdin `/dev/null`, environment per decision 26, timeout `check_timeout_secs`; on timeout the group gets `SIGKILL`. A reader keeps the last 200 lines (each cut to 300 characters, invalid UTF-8 replaced) in the `CheckRecord`; the bounce message carries the last 40 (`CHECK_SUMMARY_LINES`), deterministically — the decider summary is M8b. No `check` in the profile skips the gate, raises every review one level, and marks the run `unverified` in the report. *(Spec §11.3, §6 "Degradation".)*
35. **Review, §9 and §11.4.**
    - **Level.** `S` → `small`, `M` → `medium`, hub → `frontier`; raised one level (to at most `frontier`) when the profile has no `check`, or the task is not `tdd` while its `owns` touch `source`. `review.small = "off"` in `[orchestrator]` (the spec's key, §9; a TOML dotted key, so `[orchestrator.review] small = "off"` is the same setting) skips review for non-hub S tasks at level `small` only; the default is `"on"`.
    - **Reviewer route**, `roster::pick_reviewer(roster, author, level)`: required strength `fast` (small), the author's (medium), `frontier` (frontier); the other runtime's roster entry with the lowest strength at or above the requirement, first in roster order; else the same runtime's, preferring a model different from the author's; else the highest-strength entry of the same runtime. Effort: `low`, `medium`, `high` by level.
    - **Each round is a fresh reviewer session** in a fresh review worktree detached at the task head (`git worktree remove --force` of the previous round's path first). Its prompt has the brief, the acceptance criteria, `Base: <sha7>` and `Head: <sha7>`, the diff `git diff <base>..<head>` clamped to `REVIEW_DIFF_MAX` = 16 KiB (the spec's "the diff and the check output", §9, §11.4 step 3; `PrepareReview` computes it), the last check's 40-line summary, and every earlier round's critical and important findings to confirm fixed — never the author's runtime, model or transcript. For a `tdd` task the prompt says to look first for weakened or trivial tests. At level `small` it says to review the diff only.
    - **`submit_review { verdict, summary, findings }`**; findings have `severity` (`critical`, `important`, `minor`), `file?`, `line?`, `input?`, `text`. A critical or important finding must have `file` and `line`, or `input` (`invalid arguments: findings[<i>]: a critical or important finding needs file and line, or input`). `approve` with a critical or important finding is refused (`an approve verdict cannot carry critical or important findings; use changes`). **A rejection is any critical or important finding**; `changes` with only minor findings counts as approval. Minor findings go to the report.
    - **Rejection flow:** the reviewer session is retired; the gate failure goes to decision 38's ladder (rung 1 sends only the critical and important findings to the same worker session; rung 2 a fresh session). The worker fixes, commits and calls `task_done`, and every gate runs again from the start, then a new review round.
    - **A reviewer whose turn ends without `submit_review`** gets `REVIEW_NUDGE` as one more turn. If that turn also ends without a verdict, or the process dies twice (the same resume rule as decision 32), the round ends without a verdict. A new round starts at the same level, with no failure counted. A second verdict-less round in a row is `blocked(environment)` with `the reviewer stopped twice without a verdict`.
    - **Dispute:** a worker that believes a finding is wrong calls `task_blocked { kind: "question" }`; the user answers with `run edit` (`answer`), or overrides. Nothing but the user can approve.
    - **Override:** `anthrex run override <run> <task> --reason <text>` on a task in `review` or `blocked` with at least one commit sends it to the merge queue without review and exempts it from the spill check from then on (the user has accepted what it touched); it still passes the candidate check. The report marks it `merged without approval: <reason>`. *(Spec §9, §11.4.)*
36. **Merge queue, §11.5.** Width 1, FIFO in the order tasks reached it. One op, `MergeCandidate`, per attempt:
    1. Ref guard (decision 21).
    2. `git merge-tree --write-tree --name-only --no-messages <run_head> <task_head>`: exit 0 → the tree; exit 1 → conflict, the file list from the lines after the tree.
    3. Clean → `git commit-tree <tree> -p <run_head> -p <task_head> -m "anthrex: merge <task>: <title>"` → candidate; in the integration worktree `git checkout --detach --force <candidate>`, `git clean -fd`, and run `check` there (decision 34). No check → skip to 4.
    4. Green → ref guard again, then `git update-ref refs/heads/anthrex/<run>/integration <candidate> <run_head>` (a compare-and-swap), then `git checkout --force anthrex/<run>/integration`. The task is `merged`, `run_head` and `last_green_candidate` become the candidate, cleanup runs (decision 20), the worker session is retired.
    5. Red → `git checkout --force anthrex/<run>/integration` (back to the run head) and a gate failure of `merge` with `candidate_red_message`.
    6. Conflict, first time for this task → **hand-back**: `git merge --no-ff --no-edit <run_head>` in the task worktree (conflict markers left, `MERGE_HEAD` set), `conflict_message` with the file list to the worker, as its session's next turn (decision 29 resumes an ended session to carry it); the task returns to `working`; its next accepted `task_done` sends it **straight back to the merge queue** (the candidate check re-tests it). A hand-back merge that turns out clean needs no worker and re-queues at once. A conflict is not a gate failure. Second conflict → `blocked(conflict)`.

    The merge commit keeps task ancestry, so `git merge-base --is-ancestor <task> <run>` answers "was it merged". *(Spec §11.5; research P1-5, P3-1.)*
37. **Completion.** A run is complete when every task is `merged` or `cancelled`, the merge queue is empty and no op is pending. Before `complete`: the ref guard (a run ref that moved or a base that was rewritten halts; a base that only advanced is recorded in `base_moved` and completion goes on, decision 21), then, if `run_head` differs from `last_green_candidate` and from `base_sha`, `check` in the integration worktree (never true unless the run was rebaselined; a red result is an attention line `final check failed on the run head`, the run still completes). Blocked tasks keep the run `running` with attention lines — they wait for `retry`, `edit`, `override` or `cancel`. The `finish` edit cancels every task that has not reached `working`, lets live ones run to `merged` or `blocked`, then cancels (salvaging) the blocked ones and completes. `run cancel` kills every run session, salvages and cancels every unmerged task, and completes. *(Spec §5.3 step 6; the per-epic integration review is M9.)*

### Ladder, budgets, scheduler

38. **Escalation ladder, §10.** Each task counts `failures` (all gates), `bounces` per gate (`done` for decision 55's generated files, `proof`, `check`, `review`, `merge`), `stalls` and `budget_exceeded`. On a **gate failure**: `bounces[gate] += 1; failures += 1`; then rung 3 if `bounces[gate] > max_bounces` or `failures >= 3`; else rung 2 if `failures == 2`; else rung 1. On a **stall**: `stalls += 1; failures += 1`; rung 3 if `failures >= 3`, else rung 2. On a **hard budget breach**: `budget_exceeded += 1`; rung 3 if it is the second, else rung 2. **Diff outside `owns`** (non-generated paths only, decision 55) or **`task_blocked { kind: mis_sized }`** → rung 3. Actions:
    - rung 1: the failure text to the **same** session as its next turn (decision 29), task back to `working`.
    - rung 2: kill the session, start a **fresh** one in the same worktree and branch on `roster::escalate(route)` (decision 39), with decision 30's hand-over prompt; the session budget restarts, the task's cumulative spend does not.
    - rung 3: `blocked(mis_sized)`, the size raised one step (S→M, M→L), the text naming the cause; the worker is killed and the worktree kept (the orchestrator's reaction is M9; here the user retries, edits or cancels).
    - rung 4: when the task's cumulative spend reaches the **next size's ceiling** (S: the M budget; M or hub: `[orchestrator.budget.l]`, default 300 tool calls and 120 minutes), `blocked(human)`.

    `max_bounces` (default 2) is frozen on the run at start. *(Spec §10, §11.4 last paragraph.)*
39. **Rung-2 route**, `roster::escalate(roster, route)`: effort below `high` → same runtime and model, effort + 1; else the peer runtime's first entry at the same strength, effort `high`; else the same runtime's first entry one strength up, effort `high`; else the same route. *(Spec §10 rung 2.)*
40. **Budgets.** A task's `[task.budget]` wins over the defaults:
    - `[orchestrator.budget.s]`: 40 tool calls and 15 minutes.
    - `[orchestrator.budget.m]`: 150 tool calls and 60 minutes. Hub tasks use M's.

    **What is counted, per session:**
    - Tool calls: every `ToolUse` stream event (decision 27), sub-agents' included, on both runtimes. Codex's `command_execution`, `mcp_tool_call`, `file_change` and `web_search` items count. Codex budgets are no longer wall-clock only.
    - Wall-clock minutes since the session started. Time spent rate-limited is not subtracted.
    - Tokens: from each `TurnEnded`'s usage. `Usage::billable()` is uncached input plus cache writes plus output: Claude's `input_tokens + cache_creation_input_tokens + output_tokens`, Codex's `input_tokens - cached_input_tokens + output_tokens`. M8a.1 pins the field names, and whether Claude's `result.usage` is per turn or cumulative. The parser always yields per-turn values.

    **Token budgets are implemented here but off by default.** `Budget.tokens` is optional and absent from every default. §9 gives no token placeholder, and §14 warns that token use varies 30x, so an invented default would reject correct work. A plan or config that sets one gets it enforced. Usage arrives only at a turn's end, so a single turn can overshoot before the check runs. Tokens are always metered, shown in the snapshot and written to the report.

    **Soft**: reaching the budget on any axis sends `budget_wrap_up` once per session. **Hard**: 1.5 × the budget on any axis is a breach (decision 38). *(Spec §9 "Budget", §14.7 "Runaway stops", §14.8 "Metering", §18 `budget: tool calls, wall-clock, tokens`.)*
41. **Scheduler, §13 items 2–5.**
    - **Runnable**: `pending`, every declared dependency `merged`, and every **implicit** dependency `merged` or `cancelled`. Implicit dependencies are recomputed on every step: for two unfinished tasks whose `owns` intersect (decision 11), a task that has not started waits for one that has (any state from `preparing` on, or `blocked` with a worktree); when neither has started, the later in plan order waits for the earlier. So a task added or split in later never runs beside a started task it overlaps.
    - **Order**: by `critical_len` descending, then `priority` descending, then plan order. `critical_len(t) = weight(t) + max(critical_len(d))` over tasks that depend on `t` (declared and implicit), with weights S = 1, M = 3 (hub M = 3) until M9.5's history exists.
    - **Writer slots** (`max_writers`, default 3, 1–8): held from `preparing` through `check`, and again while a handed-back task is `working`. **Reader slots** (`max_readers`, default 3, 1–8): held by a live reviewer. A reviewer never takes a writer slot. The merge queue and its candidate check use neither.
    - **Hub alone**: a hub task starts only when no writer slot is held, and while it holds one nothing else starts.
    - The snapshot marks `on_critical_path` (the chain of maximal `critical_len` among unmerged tasks) and `wave` (the longest dependency chain before the task). *(Spec §13.)*
42. **Retry.** `anthrex run retry <run> <task>` on a `blocked` task (not L, not `dep_cancelled`) sets `failures = 1`, clears `bounces`, `budget_exceeded` and `conflicts`, sets rung 2 and starts a fresh session. *(Spec §10 last line.)*

### Durability

43. **Run persistence and the intent journal.** Each run has `<data_dir>/runs/<run>/run.json` (the whole `run::model::Run`, written to a temp file, `fsync`ed, renamed, directory `fsync`ed) and `journal.jsonl` (append, `fsync` per line) of `{"op":<id>,"intent":{…OpKind…}}` and `{"op":<id>,"done":{…OpResult…}}`. `RunService` handles each step's effects in this order: `Persist` (the new `run.json`), then for each `Op` its `intent` line, then the op itself; when the op returns, its `done` line, then the `OpDone` event. Counter-only changes (tool calls, activity) persist at most every 5 s. The journal is rewritten with only pending ops' intents when it passes 1 MiB. Runs do not live in `state.json`, whose persister is debounced and owned by the manager; `StateFile.runs` stays opaque and empty. *(Spec §17 "Intent log".)*
44. **Reconcile on start**, in `lifecycle::run` after `WindowManager::restore` and before the socket is bound: for each `run.json`, every op in `pending_ops` is resolved against `journal.jsonl` and reality — `done` present → replay its result; `intent` only → `run::reconcile` checks reality per kind (table in Interfaces) and yields either a result to replay or `NotStarted`; neither → `NotStarted`. `NotStarted` ops are dropped and the engine re-issues whatever the task's state needs on resume. Session processes are checked too, as decision 28 describes. Every op is idempotent (worktree creation reuses, salvage checks its ref, the merge candidate recognises an already-advanced run branch whose parents are the expected pair, accept recognises a base that already contains the run head). *(Spec §17 "Reconcile on start".)*
45. **Paused and resume.** On load, every run that is `running` becomes `paused` with `paused_from = running`. `halted`, `awaiting_approval`, `complete`, `paused` and terminal runs keep their state. A `halted` run (its run ref moved, or its base was rewritten, decision 21) has its sessions ended after the restart, and `resume --rebaseline` resumes them like any other resume. `base_moved` is part of `run.json` and survives a restart; a base that advanced while the daemon was down is seen at the next guard, never at load. The `pause` edit pauses a running run with its sessions alive. It stops dispatch, gates and deliveries; a turn already open runs to its end.

    `anthrex run resume <run>` (or the `resume` edit) returns the run to `paused_from`, then:
    - Second-stage deadlines that already expired fire first: an interrupted stall's grace, a sent `stall_nudge`'s silence, a pending rate-limit continue.
    - First-stage deadlines are re-armed from `now`, so downtime is not a stall.
    - Each unfinished task's live round whose session has ended gets `Op ResumeSession` with `RESUME_WORKER` or `RESUME_REVIEWER` (decision 28).
    - Tasks in `proof`, `check` or `merge_queue` re-issue their op, and `preparing` re-issues `PrepareWorktree`.
    - A failed resume starts a fresh session.

    *(Spec §11.1 persisted deadlines, §17; refreshed M8 decisions 43, 45.)*
46. **Shutdown order.** `lifecycle::run` calls `RunService::stop()` before `manager.shutdown()`; after `stop` the service ignores every event, so sessions killed at shutdown do not become failures (the manager kills each session's process group; a Claude process also exits on stdin EOF), and the last `run.json` is the one written at `stop`. *(Refreshed M8 decision 44.)*
47. **Snapshot, revision and push, §16.5.** Every run has `revision: u64` (starting at 1), bumped by any change to it; the engine keeps a global `revision` bumped with any run's. `RunService` keeps a `watch` of `proto::RunsSnapshot { revision, runs }`, published at once for structural changes and at most once per second for counter-only ones. A client sends `RunRequest::Subscribe` and receives `RunReply::Snapshot` immediately and on every publish until `Unsubscribe` or disconnect; `RunRequest::List` answers one snapshot. The TUI does not subscribe in this milestone. `anthrex run status --json` prints the same `RunsSnapshot`. *(Spec §16.5.)*
48. **Crash injection for tests.** In debug builds only (`cfg!(debug_assertions)`), `RunService::new` reads `ANTHREX_TEST_ABORT_AFTER_INTENT=<op kind>[:<n>]`; after `fsync`ing the n-th (default first) intent line of that kind, the daemon calls `std::process::abort()`. It is how `e2e_crash_after_each_intent_kind_reconciles` kills the daemon at an exact point without timing. *(Spec §21 "a daemon killed after each logged intent".)*

49. **Headless windows.** Each worker and reviewer session is registered with the window manager as a window with no terminal, so the machinery milestones 3 to 6.5 key by window id keeps working unchanged: the sidebar, status, sub-agent rows, `max_windows`, persistence and the conversation view.
    - **Types.**
      - `proto::WindowKind { Pty, Headless }` is a new field `WindowInfo.kind`, `#[serde(default)]` to `Pty`. `WindowRecord` gains the same field with the same default, so the state file needs no version bump.
      - In the daemon, `manager::entry::Process` gains a third variant, `Headless(headless::HeadlessHandle)`, beside `Live` and `Dormant`.
      - `WindowManager::create_headless` registers the window and spawns its first process. Like `create` and `restart` (`crates/daemon/src/launch/gate.rs:11–13`, `manager/create.rs:141`), it and `headless_resume` await `self.config.launch_gate.wait()` before spawning, so no agent process starts before the startup Codex version probe has finished (its result decides how Codex is launched); the wait happens outside the manager lock.
      - `apply_session_event` applies one parsed event under the lock: status, tool, session id, conversation inputs, then the feed send. No I/O happens under the lock.
      - `headless_send`, `headless_interrupt`, `headless_kill` and `headless_resume` act on the handle outside the lock.
    - **What the daemon refuses.** For a window whose kind is `Headless`:
      - `ClientMsg::Subscribe` is answered with `DaemonMsg::Error { request: "subscribe", message: "window <id> is a headless session; open its conversation with C-b m" }`. `Entry::attach` is never reached.
      - `Input`, `Kill`, `Remove` and `Restart` are refused with `DaemonMsg::Error { request: "input" | "kill" | "remove" | "restart", message: "window <id> is a headless session of run <run>; only the engine drives it. Use anthrex run cancel to stop it" }`.
      - `Resize` is accepted and ignored: no reply, exactly as for any successful `Resize` on `main` (`server.rs`, the `ClientMsg::Resize` arm replies only on error).

      There is no terminal to type into, so the refusal is by construction; the check keeps a client from reaching the handle. Every read works: list, rename, git state, and M6.5's conversation messages.
    - **The engine** acts through the `headless_*` methods directly; none of them is reachable from a client message.
    - **The orchestrator.** `proto::AgentRole` already has `Orchestrator` (Interfaces). M9 creates it as the one `Pty` run window; M8a creates none.
    - **The user's recourse** is `run retry`, `run edit`, `run override` and `run cancel`, and from M9 the orchestrator.
    - **In the TUI**, the smallest change that keeps a headless window from looking broken. No path sends `Subscribe` for a `Headless` window: on `main` there are three, `App::focus` (`crates/tui/src/app/mod.rs:344`), and `retry_dropped_subscribe` and `on_reconnected` in `crates/tui/src/app/link.rs` (`:101`, `:207`). The focused pane, drawn by `crates/tui/src/ui/terminal.rs`, shows `headless session · <runtime> · <status> · C-b m shows its conversation`, and forwards no keys (the prefix key and its commands still work). Opening the conversation from a run node is M8c's (§16.4).

    *(Spec §4.2, §16.2 "It points at its headless session, which the daemon registers as a window with no terminal".)*
50. **Claude authentication.** `[orchestrator.claude] auth = "login" | "api_key"`, default `login`.
    - **`login`** passes no `--bare`, so `claude -p` reads the user's subscription login (§22.1). If M8a.1 finds an explicit opt-out flag (for example `--no-bare`), it is passed too, so the day `--bare` becomes the default for `-p` (§23) changes nothing.
    - **`api_key`** passes `--bare`. `run start` then refuses a plan with any Claude task unless the daemon's environment has `ANTHROPIC_API_KEY`, or `orchestrator.claude.api_key_helper` is set, which is passed through `--settings` as `apiKeyHelper`. The refusal reads: `[orchestrator.claude] auth = "api_key" needs ANTHROPIC_API_KEY in the daemon's environment or orchestrator.claude.api_key_helper`.
    - M8a.1 records whether `--settings` hooks and `--mcp-config` still apply under `--bare`. If they do not, `api_key` is refused at config load with a problem and `login` is kept.
    - Codex sessions are unaffected. *(Spec §22.1, §23 first risk.)*
51. **Recorded fixtures are the contract for both stream formats.**
    - **Recording.** M8a.1 records real sessions from the installed CLIs into `crates/daemon/tests/fixtures/headless/`, following milestone 6.5's transcript fixtures in `crates/daemon/tests/fixtures/transcripts/`: a `.jsonl` file plus a `.meta.json` with M6.5's keys `runtime`, `cli_version`, `captured` (the date), `redactions` and `note`, and two more, `command` (the exact command) and `observed` (`true`, or `false` for a documented shape that could not be triggered). Personal paths are replaced by `/tmp/fixture`, and `redactions` lists what else was blanked.
    - **Parser tests.** The pure parsers are tested against those files.
    - **Shape tests.** `fake-agent`'s headless output is tested against the same files (the observed ones first; a type only the documented file has is checked against it): for every event type `fake-agent` emits, the fixture has an event of that type whose key set, recursively, contains every key `fake-agent` writes. A `fake-agent` that invents a field the real CLI does not send fails CI.
    - **Input envelope.** The user-message envelope is pinned by a fixture of what was written to the real CLI's stdin and accepted. *(Spec §23 second risk; the recurring fixture defects of earlier milestones.)*
52. **Session identity.**
    - **Recorded on every round:** the runtime's session id, the current process's pid (for Claude, the long-lived process; for Codex, the running turn's process), and the op id that launched it.
    - **One process per session.** A `CreateWindow` or `ResumeSession` never starts a second process on a session whose process is alive. The driver kills the old one first, because resuming one session from two processes interleaves its transcript.
    - **Retirement.** A retired reviewer or merged worker has its Claude process's stdin closed (EOF) and, after `INTERRUPT_GRACE`, its group killed. Its window stays listed as `Exited` for `RETIRE_AFTER` (30 s), so a watcher sees the last turn, and is then removed. *(Spec §17.)*

53. **Only the user's settings load.** Headless Claude sessions never run the repository's `.claude/settings.json` hooks or its `.mcp.json` servers (spec §4 "Only the user's own settings load", §23).
    - **When the CLI can exclude them.** M8a.1 finds the mechanism: the working assumption is `--setting-sources user` (user settings only, so project and local settings are skipped) plus `--strict-mcp-config` (only `--mcp-config` servers load, so `.mcp.json` is ignored). M8a.1 records the real flags in `CLI_CAPS.claude_user_settings_only`, and proves them with a recorded session in a scratch repository that has both a project hook and an `.mcp.json`: neither runs, and anthrex's own `--settings` hooks still fire (the hooks fixture is recorded under exactly these flags, which also sets `claude_hooks_fire_in_print`). `claude_args` then passes those flags on every launch and resume, and `--trust-project` is accepted with no effect (the report says `project settings: excluded`).
    - **When it cannot** (`claude_user_settings_only == None`), and only for a run with at least one Claude task: `run start` reads the base commit's tree with `run::git::project_settings(root, base_sha)`. It reports each tracked `.claude/settings.json` or `.claude/settings.local.json` with a non-empty `hooks` key, and each tracked `.mcp.json`. The tree is what matters, because every task worktree is checked out from it; an untracked file in `root` never reaches a worktree. If any is found, `run start` is refused with `this repository has project settings that headless Claude sessions would run without asking: <paths>; review them, then start again with --trust-project`. With `anthrex run start --trust-project`, the run starts, `Run.trusted_project` holds the paths, and the report says `project settings trusted by --trust-project: <paths>`.
    - For tests only, debug builds read `ANTHREX_TEST_NO_SETTING_SOURCES=1`, which makes the daemon act as if `claude_user_settings_only` were `None`. It is how the refusal is tested whatever the installed CLI can do.
    - **Codex loads only the user's config too.** M8a.1 item 7a records whether `codex exec` and `codex exec resume` read a repository's own Codex config (a project `.codex/config.toml`, project-scoped hooks, a trust level that enables either), in `CLI_CAPS.codex_loads_project_config: bool`, with the files it reads in `CLI_CAPS.codex_project_config_paths`. Three outcomes, three behaviours:
      - **It does not load project config:** nothing changes; the brief's Codex argv is as decision 25 says, and the report says `codex project config: not loaded by this CLI`.
      - **It loads it and can be told not to** (`CLI_CAPS.codex_user_config_only: Option<&'static [&'static str]>`, the flags or `-c` overrides M8a.1 finds): `codex_args` passes them on every first turn and every `exec resume`, and the report says `codex project config: excluded`.
      - **It loads it and cannot be told not to:** decision 53's refusal extends to Codex. For a run with at least one Codex task, `run::git::project_settings` also reports each tracked path in `codex_project_config_paths` (at least `.codex/config.toml` and `.codex/hooks.json`). The refusal message names them, and `--trust-project` accepts them the same way, recorded in `Run.trusted_project` and the report.

      Debug builds read `ANTHREX_TEST_CODEX_PROJECT_CONFIG=load` (act as if loaded with no exclusion) or `=exclude` (as if excluded by the placeholder flag `--anthrex-test-exclude-project-config`, which `fake-agent` accepts and records), so each branch is tested whatever the installed Codex does. `AGENTS.md` and `CLAUDE.md` are instructions both runtimes read by design, not config; decision 56 guards changes to them.

    *(Spec §4 "Only the user's own settings load", "Codex loads only the user's config too", §23 second risk.)*
54. **Workers run sandboxed.** Every Claude worker's `--settings` JSON enables Claude Code's sandbox with unsandboxed commands disallowed. So `Bash` runs confined, writable only in the task worktree (the session's cwd) and the repository's git common directory, which `git rev-parse --path-format=absolute --git-common-dir` gives. A commit in a linked worktree writes there, which is the same reason as decision 25's Codex writable root.
    - **The block** is built by `headless::argv::claude_settings` from `HeadlessSpec.claude_sandbox`. The working shape is `"sandbox": {"enabled": true, "allowUnsandboxedCommands": false, "filesystem": {"allowWrite": ["<git common dir>"]}}`. M8a.1 pins the real key names in `CLI_CAPS.claude_sandbox_keys` and records a proof under `-p` on the implementer's platform: a write outside the worktree is denied, and a commit inside it succeeds. Linux is recorded as a second proof when available, else "not verified".
    - **Denials.** A command the sandbox refuses is counted by decision 32's denial rule when it arrives as a `permission_denied` event or in `permission_denials`. If M8a.1 finds that it arrives only as a failed `Bash` result, it is not counted, and the stall and budget rules cover a worker that keeps trying; record which.
    - **Network.** The sandbox blocks network access by default. Anything that needs the network belongs in the profile's `setup`, which the engine runs unsandboxed before the worker starts, as Codex's `workspace-write` already requires.
    - **When the sandbox cannot start** (for example, bubblewrap is missing on Linux), the session fails with the text M8a.1 records. The engine blocks the task as `blocked(environment)` with `Claude Code's sandbox is unavailable here: <error>; set [orchestrator] worker_sandbox = false to run workers unsandboxed`.
    - **`[orchestrator] worker_sandbox = true`** is the default; `false` omits the block. Then the snapshot's `worker_sandbox` is `false` and the report says `worker sandbox: off ([orchestrator] worker_sandbox = false)`.
    - Reviewers stay as decision 24 has them after M8a.1: read-only through `--permission-mode dontAsk`, `--disallowedTools Edit,Write,NotebookEdit` and their allowed tools (plan mode blocks the reviewer's MCP call in `-p`), with no sandbox block.

    *(Spec §4 "Workers run sandboxed".)*
55. **Generated files bounce; they are not spills.** The profile gains `generated: Vec<String>` (plan `[profile]` over `[orchestrator.profile]`, per key like the rest; M8b's stored profile later fills it from lock files). `VerifyDone` splits the changed paths outside `owns` into `outside_owns` (not generated) and `generated_outside_owns` (matching `generated`, with decision 11's matching rules). Then:
    - Any non-generated path outside `owns` → rung 3, as before, whatever else changed.
    - Otherwise, generated paths outside `owns` → a gate failure of gate `done` (`bounces.done += 1; failures += 1`, decision 38's ladder). The `task_done` reply is `ToolResult { ok: false }` with `generated_files_message(files)`, which is the rung-1 message itself, so rung 1 queues nothing more; rung 2 or 3 act as for any gate. When the turn-end fallback made the claim, the same text is queued as the rung-1 message.
    - A task whose `owns` covers the file passes.
    - An overridden task is exempt, as from the spill check.
    - Paths decision 56 catches are removed before this split, so a protected file outside `owns` is a decision-56 bounce, not a spill.

    *(Spec §6 `generated`.)*
56. **Protected agent-config paths.** The profile gains `protected: Vec<String>`. The built-in list is always included: `.claude/**`, `.mcp.json`, `.codex/**`, `**/CLAUDE.md`, `**/AGENTS.md`. `[orchestrator.profile] protected` and the plan's `[profile] protected` **add** to it; neither can remove a built-in. This is the one profile key that merges instead of overriding.
    - **The rule.** A changed path (from `VerifyDone`'s `<run_head>...HEAD` diff) that matches `protected` (decision 11's matching) is allowed only if the task's `owns` contains that exact path **literally**. A literal entry has no glob metacharacter and equals the path after dropping a leading `./` and a trailing `/`. A wildcard never counts, and neither does a plain directory entry covering it (so `.claude` in `owns` does not allow `.claude/settings.json`).
    - **Otherwise** it is a gate failure of gate `done` at rung 1, by exactly decision 55's mechanism: `bounces.done += 1; failures += 1`; the `task_done` reply is `ToolResult { ok: false }` with `protected_file_message(files)`, one line per file: `<path> configures or instructs future agents; this task may change it only if its owns names it exactly`, then `Revert it and call task_done again, or ask for the plan to be amended.`
    - **Order.** `VerifyDone` checks `protected` first. If anything is caught, that is the failure, and nothing else outside `owns` is judged on this claim. Otherwise it goes on to decision 55's generated/spill split, with the protected paths removed.
    - **Overridden tasks are exempt**, as from the spill check, because the user accepted what the task touched.
    - **Plan validation warns; it does not reject.** `run start` (and every edit batch) computes, from the base tree, the tracked files that match `protected` (`Preflight.protected_files`, also kept as `Run.protected_files`). For each task, each of those files that the task's `owns` matches without naming it literally adds a note to the task, `owns <glob> covers protected <path>; name it exactly in owns if this task must change it (rule 6.protected)`. `run start` also prints each note as a warning line on stderr.
    - The warning looks only at existing tracked files, so a nested `AGENTS.md` that a task creates is still caught by the rule above at `task_done`, just without a warning in advance.

    *(Spec §6 `protected`.)*

## Interfaces

### `proto`

`crates/proto/src/run.rs` (new; everything derives `Debug, Clone, PartialEq, Serialize, Deserialize`, and `Eq`, `Copy`, `Hash`, `PartialOrd`, `Ord` where noted):

```rust
#[serde(rename_all = "snake_case")] #[derive(Copy, Eq, Hash)]
pub enum AgentRole { Orchestrator, Worker, Reviewer }        // Orchestrator used from M9
// snake_case, not lowercase: the three variants' wire strings ("orchestrator", "worker", "reviewer") are the
// same under both, and later multi-word variants (M9.5's TestWriter) then serialize as "test_writer".

#[derive(Eq)]
pub struct RunRef { pub run_id: String, pub task_id: Option<String>, pub role: AgentRole, pub session: u32 }

#[serde(rename_all = "lowercase")] #[derive(Copy, Eq, Hash, PartialOrd, Ord)]
pub enum Strength { Fast, Standard, Frontier }
#[serde(rename_all = "lowercase")] #[derive(Copy, Eq, Hash, PartialOrd, Ord)]
pub enum Effort { Low, Medium, High }
#[derive(Copy, Eq, Hash, PartialOrd, Ord)]
pub enum Size { S, M, L }                                    // serialized "S", "M", "L"
#[serde(rename_all = "lowercase")] #[derive(Copy, Eq)]
pub enum TaskKind { Code, Docs, Research, Review }
#[serde(rename_all = "lowercase")] #[derive(Copy, Eq)]
pub enum TestMode { Tdd, Check, None }

#[serde(deny_unknown_fields)] #[derive(Default, Eq)]
pub struct RouteSpec { #[serde(default)] pub runtime: Option<Runtime>, #[serde(default)] pub model: Option<String>,
                       #[serde(default)] pub strength: Option<Strength>, #[serde(default)] pub effort: Option<Effort> }
#[derive(Eq)]
pub struct Route { pub runtime: Runtime, pub model: String, pub strength: Strength, pub effort: Effort }
#[serde(deny_unknown_fields)] #[derive(Copy, Eq)]
pub struct Budget { pub tool_calls: u32, pub minutes: u32, #[serde(default)] pub tokens: Option<u64> }  // tokens: decision 40

#[serde(deny_unknown_fields)] #[derive(Default, Eq)]
pub struct ProfileSpec {
    #[serde(default)] pub modules: Option<Vec<String>>, #[serde(default)] pub hub: Option<Vec<String>>,
    #[serde(default)] pub source: Option<Vec<String>>, #[serde(default)] pub check: Option<String>,
    #[serde(default)] pub check_timeout_secs: Option<u64>, #[serde(default)] pub single_test: Option<String>,
    #[serde(default)] pub test_passed: Option<String>, #[serde(default)] pub setup: Option<String>,
    #[serde(default)] pub generated: Option<Vec<String>>,
    #[serde(default)] pub protected: Option<Vec<String>>,   // added to the built-ins, never replacing them
    #[serde(default)] pub env: Option<std::collections::BTreeMap<String, String>>,
}

#[serde(deny_unknown_fields)] #[derive(Eq)]
pub struct PlanTask {
    pub id: String, pub title: String,
    #[serde(default)] pub epic: Option<String>,
    #[serde(default = "default_kind")] pub kind: TaskKind,      // default_kind() == TaskKind::Code
    pub size: Size,
    #[serde(default)] pub interface_change: bool,
    #[serde(default)] pub test_mode: Option<TestMode>,
    #[serde(default)] pub test_mode_reason: Option<String>,
    pub owns: Vec<String>,
    #[serde(default)] pub deps: Vec<String>,
    #[serde(default)] pub priority: i32,
    pub brief: String,
    pub acceptance: Vec<String>,
    #[serde(default)] pub test_to_write: Option<String>,
    #[serde(default)] pub scout_refs: Vec<String>,
    #[serde(default)] pub route: RouteSpec,
    #[serde(default)] pub budget: Option<Budget>,
}

#[serde(deny_unknown_fields)] #[derive(Eq)]
pub struct Plan {
    pub goal: String,
    #[serde(default)] pub max_writers: Option<u8>,
    #[serde(default)] pub max_readers: Option<u8>,
    #[serde(default)] pub max_bounces: Option<u8>,
    #[serde(default)] pub profile: ProfileSpec,
    #[serde(rename = "task")] pub tasks: Vec<PlanTask>,
}

#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)] #[derive(Eq)]
pub enum PlanEdit {
    AddTask { task: PlanTask },
    SplitTask { task_id: String, into: Vec<PlanTask> },
    CancelTask { task_id: String },
    AmendTask { task_id: String,
                #[serde(default)] brief: Option<String>, #[serde(default)] acceptance: Option<Vec<String>>,
                #[serde(default)] route: Option<RouteSpec>, #[serde(default)] test_mode: Option<TestMode>,
                #[serde(default)] test_mode_reason: Option<String>, #[serde(default)] priority: Option<i32>,
                #[serde(default)] size: Option<Size> },
    AddDep { task_id: String, dep: String },
    Answer { task_id: String, text: String },
    Pause, Resume, Finish,
}
#[serde(deny_unknown_fields)] #[derive(Eq)]
pub struct EditFile { #[serde(rename = "edit")] pub edits: Vec<PlanEdit> }   // `anthrex run edit --file`

#[derive(Eq)]
pub struct ModelEntry { pub runtime: Runtime, pub model: String, pub strength: Strength, pub note: String }

#[serde(rename_all = "snake_case")] #[derive(Copy, Eq)]
pub enum RunState { AwaitingApproval, Running, Paused, Halted, Complete, Accepted, Discarded, Failed }
#[serde(rename_all = "snake_case")] #[derive(Copy, Eq)]
pub enum TaskState { Pending, Queued, Preparing, Working, Proof, Check, Review, MergeQueue, Merged, Blocked, Cancelled }
#[serde(rename_all = "snake_case")] #[derive(Copy, Eq)]
pub enum BlockReason { MisSized, Human, Conflict, DepCancelled, Question, Environment }
#[serde(rename_all = "snake_case")] #[derive(Copy, Eq)]
pub enum GateKind { Done, Proof, Check, Review, Merge }
#[derive(Copy, Eq, Default)]
pub struct GateCounts { pub done: u8, pub proof: u8, pub check: u8, pub review: u8, pub merge: u8 }   // done: decision 55
#[serde(rename_all = "lowercase")] #[derive(Copy, Eq)]
pub enum Verdict { Approve, Changes }
#[serde(rename_all = "lowercase")] #[derive(Copy, Eq, PartialOrd, Ord)]
pub enum Severity { Critical, Important, Minor }
#[derive(Eq)]
pub struct Finding { pub severity: Severity, #[serde(default)] pub file: Option<String>, #[serde(default)] pub line: Option<u32>,
                     #[serde(default)] pub input: Option<String>, pub text: String }
#[serde(rename_all = "snake_case")] #[derive(Copy, Eq)]
pub enum DoneSignal { TaskDone, TurnEndFallback }            // "task_done" | "turn_end_fallback"
#[serde(rename_all = "lowercase")] #[derive(Copy, Eq)]
pub enum FinishAction { Accept, Discard }

impl RunState  { pub fn label(self) -> &'static str; pub fn is_terminal(self) -> bool; }
impl TaskState { pub fn label(self) -> &'static str; pub fn is_finished(self) -> bool; } // merged | cancelled
impl Size      { pub fn raised(self) -> Size; }                                          // S→M, M→L, L→L
impl Effort    { pub fn raised(self) -> Option<Effort>; }                               // High → None
```

`crates/proto/src/run_info.rs` (new; the snapshot, every field `#[serde(default)]` where it is optional so M8c and M9 can add fields):

```rust
pub struct Spend { pub tool_calls: u32, pub secs: u64, pub tokens: u64 }  // Copy, Eq, Default
pub struct TokenUsage { pub input: u64, pub output: u64, pub cache_read: u64, pub cache_write: u64 } // Copy, Eq, Default
impl TokenUsage { pub fn billable(&self) -> u64; }   // decision 40: input + cache_write + output. Defined here, in
                                                     // proto: the daemon cannot add an inherent impl to a foreign type.
pub struct BlockInfo { pub reason: BlockReason, pub text: String }
pub struct CheckInfo { pub at: u64, pub ok: bool, pub code: Option<i32>, pub timed_out: bool,
                       pub secs: u64, pub summary: String, pub on_candidate: bool }
pub struct ProofInfo { pub at: u64, pub test: String, pub red: String, pub red_failed: bool,
                       pub head_passed: bool, pub matched: bool, pub ok: bool }
pub struct ReviewInfo { pub round: u32, pub route: Route, pub verdict: Option<Verdict>,
                        pub summary: String, pub findings: Vec<Finding>, pub blocking: bool }
pub struct AgentRoundInfo {
    pub role: AgentRole, pub session: u32, pub round: u32, pub window_id: Option<u32>, pub route: Route,
    pub session_id: Option<String>,                   // the runtime's own session id (decision 52)
    pub started_at: u64, pub ended_at: Option<u64>, pub tool_calls: u32, pub last_event: u64,
    pub turn_open: bool, pub turns: u32, pub rate_limited: bool, pub open_subagents: u32,
    pub denials: u32, pub usage: TokenUsage,
}
pub struct TaskInfo {
    pub id: String, pub title: String, pub epic: Option<String>, pub kind: TaskKind,
    pub size: Size, pub hub: bool, pub test_mode: TestMode, pub test_mode_reason: Option<String>,
    pub notes: Vec<String>,                           // decision 9/10 raises, forced modes
    pub owns: Vec<String>, pub deps: Vec<String>, pub implicit_deps: Vec<String>, pub priority: i32,
    pub route: Route, pub review_route: Option<Route>, pub budget: Budget,
    pub spent_session: Spend, pub spent_total: Spend,
    pub state: TaskState, pub block: Option<BlockInfo>,
    pub rung: u8, pub failures: u8, pub bounces: GateCounts, pub stalls: u8, pub budget_exceeded: u8, pub conflicts: u8,
    pub branch: String, pub worktree: PathBuf, pub start_commit: Option<String>, pub head: Option<String>,
    pub test: Option<String>, pub red: Option<String>, pub done_signal: Option<DoneSignal>,
    pub rounds: Vec<AgentRoundInfo>, pub reviews: Vec<ReviewInfo>,
    pub last_check: Option<CheckInfo>, pub last_proof: Option<ProofInfo>,
    pub merge_commit: Option<String>, pub merged_without_approval: Option<String>,
    pub salvage_refs: Vec<String>, pub on_critical_path: bool, pub wave: u32,
    pub history: Vec<String>,                          // last 10 events, newest first, "<hh:mm> <text>"
}
pub struct RunInfo {
    pub run_id: String, pub goal: String, pub project: PathBuf, pub root: PathBuf,
    pub state: RunState, pub paused_from: Option<RunState>, pub halted_reason: Option<String>,
    pub approved_by: Option<String>,                  // "user" | "--yes"
    pub base_branch: String, pub base_sha: String, pub run_branch: String, pub run_head: String,
    pub base_moved: Option<BaseMovedInfo>,            // decision 21: the base advanced; not a halt
    pub revision: u64, pub max_writers: u8, pub max_readers: u8, pub max_bounces: u8,
    pub writers_busy: u8, pub readers_busy: u8, pub unverified: bool,
    pub worker_sandbox: bool,                         // decision 54; false is reported
    pub trusted_project: Vec<String>,                 // decision 53: the project settings --trust-project accepted
    pub rate_limits: std::collections::BTreeMap<String, u32>, // per runtime label, for M9.5
    pub tasks: Vec<TaskInfo>, pub critical_path: Vec<String>, pub attention: Vec<String>,
    pub report_path: PathBuf, pub outcome: Option<String>, pub created_at: u64,
}
pub struct BaseMovedInfo {                            // decisions 20, 21
    pub from: String, pub to: String,                 // base_sha, and the base head last seen
    pub commits: Vec<String>,                         // "<sha7> <author>: <subject>", newest first, at most 50; empty in a snapshot, filled in ConfirmNeeded
    pub total: u32,                                   // every commit in from..to
}
pub struct RunsSnapshot { pub revision: u64, pub runs: Vec<RunInfo> }
```

`crates/proto/src/run_wire.rs` (new):

```rust
pub struct ToolCall { pub run_id: String, pub task_id: Option<String>, pub role: AgentRole,
                      pub window_id: u32, pub tool: String, pub args: serde_json::Value }
pub enum RunRequest {
    Start { plan_toml: String, dir: PathBuf, yes: bool, trust_project: bool },
    Approve { run_id: String }, Reject { run_id: String },
    Edit { run_id: String, edits: Vec<PlanEdit> },
    Retry { run_id: String, task_id: String },
    Override { run_id: String, task_id: String, reason: String },
    Cancel { run_id: String },
    Resume { run_id: String, rebaseline: bool },
    Finish { run_id: String, action: FinishAction, confirm: Option<String> },
    List, Subscribe, Unsubscribe,
    Tool(ToolCall),
}
pub enum RunReply {
    Started { run_id: String, state: RunState },
    Done { request: String, message: String },
    Refused { request: String, message: String },
    ConfirmNeeded { run_id: String, prompt: String, base_moved: Option<BaseMovedInfo> }, // Some: accept onto an advanced base; confirm "<run id>@<to>"
    Snapshot(RunsSnapshot),
    ToolResult { ok: bool, text: String },
}
pub mod request {   // the `request` labels a client matches on. Always spelled proto::run_wire::request:
                    // proto::messages::request (CREATE, REMOVE) already exists, so neither is re-exported at the root.
    pub const START: &str = "run start";     pub const APPROVE: &str = "run approve";
    pub const REJECT: &str = "run reject";   pub const EDIT: &str = "run edit";
    pub const RETRY: &str = "run retry";     pub const OVERRIDE: &str = "run override";
    pub const CANCEL: &str = "run cancel";   pub const RESUME: &str = "run resume";
    pub const FINISH: &str = "run finish";
}
```

`messages.rs`: `ClientMsg::Run(RunRequest)` and `DaemonMsg::Run(RunReply)`; `HookSource` gains `Stream` (serialized `"stream"`: the enum is `kebab-case`, decision 3). `types.rs`: `#[derive(Default)] pub enum WindowKind { #[default] Pty, Headless }` (`rename_all = "lowercase"`, `Copy`, `Eq`); `WindowInfo` gains `#[serde(default)] pub kind: WindowKind` and `#[serde(default)] pub run: Option<RunRef>`; `ClientKind` gains `Mcp`. `lib.rs` re-exports the new modules' public types by name (not by glob, so no new name can shadow an M6.5 re-export) and bumps `PROTO_VERSION` (header).

### `config` (`crates/config/src/orchestrator.rs`, new, beside M6.5's `conversation.rs` and `git.rs`; `lib.rs` gains `mod orchestrator;`, one `pub use`, the field, its default, one call and the `report_unknown_keys` arm)

```rust
pub struct Orchestrator {
    pub max_writers: u8,            // 3, 1..=8
    pub max_readers: u8,            // 3, 1..=8
    pub max_bounces: u8,            // 2, 1..=5
    pub max_tasks: u32,             // 50, 1..=200
    pub max_windows: u32,           // 60, 1..=500
    pub default_runtime: proto::Runtime,   // claude; claude | codex
    pub review_small: bool,         // review.small = "on" | "off" ([orchestrator.review] small); "on" (true) by default
    pub budget_s: proto::Budget,    // [orchestrator.budget.s] 40 / 15 / no tokens
    pub budget_m: proto::Budget,    // [orchestrator.budget.m] 150 / 60 / no tokens
    pub budget_l: proto::Budget,    // [orchestrator.budget.l] 300 / 120 / no tokens (rung-4 ceiling for M and hub)
    pub stall_after_secs: u64,      // 600, 5..=7200
    pub rate_limit_retry_secs: u64, // 300, 5..=3600
    pub denials_before_block: u32,  // 3, 1..=50
    pub git_timeout_secs: u64,      // 60, 5..=600
    pub worker_permission_mode: String,        // "acceptEdits"; default | acceptEdits | dontAsk | bypassPermissions
    pub worker_allowed_tools: Vec<String>,     // ["Bash","Edit","Write","Read","Glob","Grep","Agent","TodoWrite"]
    pub worker_codex_sandbox: String,          // "workspace-write"; read-only | workspace-write | danger-full-access
    pub worker_sandbox: bool,                  // true: Claude workers run in Claude Code's sandbox (decision 54)
    pub claude: ClaudeHeadless,     // [orchestrator.claude] (decision 50)
    pub builtin_models: bool,       // true
    pub models: Vec<proto::ModelEntry>,        // merged roster (decision 23)
    pub profile: proto::ProfileSpec,           // [orchestrator.profile], [orchestrator.profile.env]; `protected`
                                               // holds the user's additions only: the built-ins (decision 56) are
                                               // added by resolve_profile (M8a.5) in the daemon
}
impl Default for Orchestrator;
#[derive(Default)] pub enum ClaudeAuth { #[default] Login, ApiKey }      // "login" | "api_key"
pub struct ClaudeHeadless { pub auth: ClaudeAuth, pub api_key_helper: Option<String> }
pub fn default_roster() -> Vec<proto::ModelEntry>;
pub(crate) fn read(table: &toml::Table, problems: &mut Vec<Problem>) -> Orchestrator;
```

`Config` gains `pub orchestrator: Orchestrator`; `report_unknown_keys` (`lib.rs:547`) stops skipping `orchestrator` silently (its `| "orchestrator" => {}` arm, `lib.rs:551`, and the comment at `lib.rs:138` go) and delegates its nested keys to `orchestrator.rs`, the way M6.5 delegates `conversation` to `report_unknown_conversation`. Parsing rules: each key validated on its own, an invalid value is a `Problem` and keeps the default (`orchestrator.max_writers: must be between 1 and 8 (using 3)`); `review.small` is the string `"on"` or `"off"` in the nested `orchestrator.review` table, whose unknown keys are reported like the rest; a bad `[[orchestrator.models]]` entry is skipped with its index (`orchestrator.models[2]: empty model is only allowed for codex`); unknown keys `unknown key, ignored`.

### `daemon`

```rust
// run/model.rs (pure). Everything derives Debug, Clone, PartialEq, Serialize, Deserialize.
pub type OpId = u64;
pub struct Profile { pub modules: Vec<String>, pub hub: Vec<String>, pub source: Vec<String>,
                     pub check: Option<String>, pub check_timeout_secs: u64, pub single_test: Option<String>,
                     pub test_passed: Option<String>, pub setup: Option<String>, pub generated: Vec<String>,
                     pub protected: Vec<String>,                      // built-ins + config + plan (decision 56)
                     pub env: BTreeMap<String, String> }
pub struct RunLimits { pub max_writers: u8, pub max_readers: u8, pub max_bounces: u8, pub max_tasks: u32, pub max_windows: u32,
                       pub default_runtime: Runtime,
                       pub review_small: bool, pub budget_s: Budget, pub budget_m: Budget, pub budget_l: Budget,
                       pub stall_after_secs: u64, pub rate_limit_retry_secs: u64, pub denials_before_block: u32,
                       pub git_timeout_secs: u64, pub worker_permission_mode: String, pub worker_allowed_tools: Vec<String>,
                       pub worker_codex_sandbox: String, pub worker_sandbox: bool, pub claude_auth: ClaudeAuth }
pub enum ReviewLevel { Small, Medium, Frontier }     // created by M8a.4, which roster::pick_reviewer needs first
pub enum FallbackState { None, Counting, Nudged { had_commits: bool } }          // decision 32 turn-end fallback
pub enum StallState { Watching, Interrupted { deadline: u64 }, Nudged }          // decision 32 stall watchdog
pub enum FailedTurn { None, WaitingContinue { at: u64, rate_limit: bool }, ContinueSent { rate_limit: bool } }
pub struct AgentRound {
    pub role: AgentRole, pub session: u32, pub round: u32, pub window_id: Option<u32>, pub route: Route,
    pub launch_op: OpId, pub session_id: Option<String>, pub pid: Option<u32>, pub ended: bool,
    pub started_at: u64, pub ended_at: Option<u64>,
    pub turn_open: bool, pub turns: u32, pub turn_had_task_done: bool, pub last_event: u64,
    pub tool_calls: u32, pub rate_limited_until: Option<u64>, pub in_retry_streak: bool,
    pub open_subagents: BTreeSet<String>, pub denials: u32, pub usage: TokenUsage,
    pub deaths: u8, pub fallback: FallbackState, pub stall: StallState, pub failed_turn: FailedTurn,
    pub review_nudged: bool, pub wrap_up_sent: bool, pub retiring: bool, pub delivery_failures: u8,
}
pub struct CheckRecord { pub at: u64, pub ok: bool, pub code: Option<i32>, pub timed_out: bool,
                         pub tail: String, pub secs: u64, pub on_candidate: bool }
pub struct ProofRecord { pub at: u64, pub test: String, pub red: String, pub red_failed: bool,
                         pub head_passed: bool, pub matched: bool, pub red_tail: String, pub head_tail: String }
pub struct ReviewRecord { pub round: u32, pub route: Route, pub base: String, pub head: String,
                          pub verdict: Option<Verdict>, pub summary: String, pub findings: Vec<Finding> }
pub struct DoneClaim { pub summary: String, pub test: Option<String>, pub red: Option<String>, pub signal: DoneSignal }
pub struct TaskEvent { pub at: u64, pub text: String }
pub struct Task {
    pub spec: PlanTask, pub size: Size, pub hub: bool, pub test_mode: TestMode, pub notes: Vec<String>,
    pub review_level: Option<ReviewLevel>,          // None: not reviewed (review.small = "off")
    pub route: Route, pub review_route: Option<Route>, pub budget: Budget, pub implicit_deps: Vec<String>,
    pub state: TaskState, pub block: Option<BlockInfo>,
    pub rung: u8, pub failures: u8, pub bounces: GateCounts, pub stalls: u8, pub budget_exceeded: u8, pub conflicts: u8,
    pub session: u32, pub spent_total: Spend,
    pub branch: String, pub worktree: PathBuf, pub prewarmed: bool, pub start_commit: Option<String>, pub head: Option<String>,
    pub done: Option<DoneClaim>, pub rounds: Vec<AgentRound>, pub reviews: Vec<ReviewRecord>,
    pub checks: Vec<CheckRecord>, pub proofs: Vec<ProofRecord>, pub handed_back: bool,
    pub merge_commit: Option<String>, pub merged_without_approval: Option<String>,
    pub salvage_refs: Vec<String>, pub failure_log: Vec<String>, pub history: Vec<TaskEvent>,
}
pub struct Outgoing { pub id: u64, pub window_id: u32, pub task_id: String, pub text: String,
                      pub queued_at: u64, pub delivered_at: Option<u64> }
pub struct PendingOp { pub op: OpId, pub task_id: Option<String>, pub kind: OpKind }
pub struct LogEntry { pub at: u64, pub text: String }                 // at most 500 per run
pub struct BaseMoved { pub from: String, pub to: String, pub commits: u32, pub seen_at: u64 } // decision 21
pub struct Run {
    pub id: String, pub goal: String, pub root: PathBuf, pub project: PathBuf, pub git_common_dir: PathBuf,
    pub wt_dir: PathBuf, pub data_dir: PathBuf,
    pub base_branch: String, pub base_sha: String, pub run_head: String, pub last_green_candidate: Option<String>,
    pub base_moved: Option<BaseMoved>,                // decision 21; #[serde(default)]
    pub state: RunState, pub paused_from: Option<RunState>, pub halted_reason: Option<String>,
    pub approved_by: Option<String>, pub profile: Profile, pub limits: RunLimits, pub roster: Vec<ModelEntry>,
    pub tasks: Vec<Task>, pub merge_queue: Vec<String>, pub outbox: Vec<Outgoing>, pub next_message: u64,
    pub pending_ops: BTreeMap<OpId, PendingOp>, pub next_op: OpId, pub windows_created: u32,
    pub revision: u64, pub unverified: bool, pub final_check_failed: bool, pub trusted_project: Vec<String>,
    pub protected_files: Vec<String>,                 // tracked files at base_sha matching profile.protected (decision 56)
    pub rate_limits: BTreeMap<String, u32>, pub outcome: Option<String>, pub log: Vec<LogEntry>, pub created_at: u64,
}
impl Run {
    pub fn run_branch(&self) -> String;               // anthrex/<id>/integration
    pub fn integration_path(&self) -> PathBuf;        // <wt_dir>/runs/<id>/integration
    pub fn task_path(&self, task: &str) -> PathBuf;   // <wt_dir>/runs/<id>/<task>
    pub fn review_path(&self, task: &str) -> PathBuf; // …/<task>.review
    pub fn proof_path(&self, task: &str) -> PathBuf;  // …/<task>.proof
    pub fn report_path(&self) -> PathBuf;             // <data_dir>/REPORT.md
    pub fn short(&self) -> &str;                      // the 4 hex digits
    pub fn task(&self, id: &str) -> Option<&Task>;
}

// run/globs.rs (pure)
pub fn literal_prefix(glob: &str) -> Vec<&str>;
pub fn intersects(a: &str, b: &str) -> bool;
pub fn any_intersect(a: &[String], b: &[String]) -> bool;
pub fn modules_spanned(owns: &[String], modules: &[String]) -> ModuleSpan;   // enum ModuleSpan { One(String), Many }
pub fn inside_area(glob: &str, area: &[String]) -> bool;
pub struct OwnsMatcher;                               // globset-backed
impl OwnsMatcher { pub fn new(owns: &[String]) -> Result<Self, String>; pub fn matches(&self, path: &str) -> bool; }
pub fn validate_glob(glob: &str) -> Result<(), String>;   // not blank, not absolute, no `..`

// run/roster.rs (pure)
pub fn find(roster: &[ModelEntry], runtime: Runtime, model: &str) -> Option<&ModelEntry>;
pub fn first_at(roster: &[ModelEntry], runtime: Runtime, strength: Strength) -> Option<&ModelEntry>;
pub fn peer(runtime: Runtime) -> Runtime;                                    // claude <-> codex
pub fn pick_reviewer(roster: &[ModelEntry], author: &Route, level: ReviewLevel) -> Route;
pub fn escalate(roster: &[ModelEntry], route: &Route) -> Route;

// run/plan.rs and run/validate.rs (pure)
pub fn parse_plan(text: &str) -> Result<Plan, String>;          // TOML, error names line and key
pub fn parse_edits(text: &str) -> Result<Vec<PlanEdit>, String>;
pub struct PlanError { pub task: Option<String>, pub field: String, pub rule: String, pub message: String } // Display: "task <id>: <field>: <message>" or "<field>: <message>"
pub struct Preflight { pub root: PathBuf, pub project: PathBuf, pub git_common_dir: PathBuf,   // decision 16
                      pub base_branch: String, pub base_sha: String, pub protected_files: Vec<String> }
pub const BUILTIN_PROTECTED: &[&str] = &[".claude/**", ".mcp.json", ".codex/**", "**/CLAUDE.md", "**/AGENTS.md"];
pub fn names_literally(owns: &[String], path: &str) -> bool;   // run/globs.rs, decision 56
pub struct BuildContext<'a> { pub id: String, pub wt_dir: PathBuf, pub data_dir: PathBuf,
                              pub config: &'a config::Orchestrator, pub now: u64, pub yes: bool }
pub fn resolve_profile(plan: &ProfileSpec, config: &ProfileSpec) -> Profile;
pub fn build_run(plan: Plan, pre: Preflight, ctx: BuildContext<'_>) -> Result<Run, Vec<PlanError>>;
pub fn resolve_task(spec: PlanTask, profile: &Profile, limits: &RunLimits, roster: &[ModelEntry],
                    default_runtime: Runtime) -> Result<Task, Vec<PlanError>>;       // decisions 8-10
pub enum EditScope { Run, Area { globs: Vec<String> } }
pub fn validate_tasks(tasks: &[Task], touched: &BTreeSet<String>, scope: &EditScope,
                      max_tasks: u32, profile: &Profile) -> Vec<PlanError>;           // decisions 9-13
pub fn slug(goal: &str, suffix: u16) -> String;
pub fn random_suffix() -> u16;

// run/edits.rs (pure)
pub fn apply_edits(run: &Run, edits: &[PlanEdit], scope: &EditScope, now: u64)
    -> Result<(Run, Vec<EditConsequence>), Vec<PlanError>>;
pub enum EditConsequence { CancelLive { task_id: String }, Deliver { task_id: String, text: String },
                           Pause, Resume, Finish }

// run/engine/mod.rs (pure)
pub type ReplyId = u64;
pub struct EngineState { pub runs: BTreeMap<String, Run>, pub revision: u64, pub stopped: bool } // Default
pub struct Event { pub now: u64, pub kind: EventKind }
pub enum EventKind {
    Start { reply: ReplyId, run: Run },
    Approve { reply: ReplyId, run_id: String }, Reject { reply: ReplyId, run_id: String },
    Edit { reply: ReplyId, run_id: String, edits: Vec<PlanEdit>, scope: EditScope },
    Retry { reply: ReplyId, run_id: String, task_id: String },
    Override { reply: ReplyId, run_id: String, task_id: String, reason: String },
    Cancel { reply: ReplyId, run_id: String },
    Resume { reply: ReplyId, run_id: String, rebaseline: Option<(String, String)> }, // (base sha, run head) read by the driver
    BaseAdvanced { run_id: String, to: String, commits: u32 },          // decision 21: sent by the driver when a guard sees an advanced base
    Finish { reply: ReplyId, run_id: String, action: FinishAction },
    Tool { reply: ReplyId, call: ToolCall },
    OpDone { run_id: String, op: OpId, result: OpResult },
    Signal { window_id: u32, signal: AgentSignal },
    Delivered { run_id: String, message_ids: Vec<u64>, ok: bool, error: Option<String> },
    Restore { runs: Vec<Run>, replay: Vec<(String, OpId, OpResult)> },
    Stop,
    Tick,
}
pub enum AgentSignal {                                // the driver's translation of WindowSignal (decision 27)
    Init { session_id: String }, TurnStarted, ToolUse { name: String },
    TurnEnded { outcome: TurnOutcome, usage: Option<TokenUsage>, denials: u32 },
    ApiRetry { error: String, delay_ms: u64 }, PermissionDenied { tool: String, reason: String },
    SubagentStart { agent_id: String }, SubagentStop { agent_id: String },
    Activity,                                         // any other event: text, tool result, compaction, unknown
    ProcessExited { code: Option<i32>, killed_by_engine: bool, pid: u32 },
    ProcessStarted { pid: u32 },
}
pub use crate::headless::TurnOutcome;                 // Completed | Failed { error: String, kind: FailureKind } | Interrupted
pub enum OpKind {
    CreateRunBranch { root: PathBuf, branch: String, base_sha: String, path: PathBuf, setup: Option<String>, env: Vec<(String, String)> },
    PrepareWorktree { root: PathBuf, branch: String, from: String, path: PathBuf, setup: Option<String>, env: Vec<(String, String)> },
    CreateWindow { name: String, spec: HeadlessSpec, session_uuid: Option<String>, first_turn: String,
                   project: PathBuf, worktree: PathBuf, jitter_ms: u64 },   // registers a headless window, starts the session
    ResumeSession { window_id: u32, session_id: String, message: String, jitter_ms: u64 },
    VerifyDone { worktree: PathBuf, start: String, run_head: String, owns: Vec<String>, generated: Vec<String>, protected: Vec<String>, spill_exempt: bool, red: Option<String> },
    CountCommits { worktree: PathBuf, start: String },
    DiffSoFar { worktree: PathBuf, start: String },
    Proof { root: PathBuf, path: PathBuf, red: String, head: String,
            command: String,   // single_test with {test} already replaced by shell_quote(test)
            passed: String,    // test_passed with {test} already replaced by regex::escape(test)
            timeout_secs: u64, setup: Option<String>, env: Vec<(String, String)> },
    Check { dir: PathBuf, command: String, timeout_secs: u64, env: Vec<(String, String)> },
    PrepareReview { root: PathBuf, head_ref: String, base_ref: String, path: PathBuf },
    MergeCandidate { root: PathBuf, integration: PathBuf, run_branch: String, expected_run_head: String,
                     base_branch: String, expected_base: String, task_head: String, message: String,
                     check: Option<String>, timeout_secs: u64, env: Vec<(String, String)> },
    HandBack { worktree: PathBuf, run_head: String },
    RemoveWorktree { root: PathBuf, path: PathBuf, salvage_ref: String },
    VerifyRefs { root: PathBuf, base_branch: String, expected_base: String, run_branch: String, expected_run_head: String },
    Accept { root: PathBuf, base_branch: String, expected_base: String, run_branch: String, message: String, worktrees: Vec<(PathBuf, String)>, branch_prefix: String }, // expected_base: base_sha, or the confirmed advanced head (decision 20)
    Discard { root: PathBuf, worktrees: Vec<(PathBuf, String)>, branch_prefix: String },
}
pub enum OpResult {
    Worktree { head: String }, SetupFailed { output: String },
    Window { window_id: u32 }, Resumed, ResumeFailed { error: String },
    DoneChecked { commits: u32, dirty_tracked: u32, merge_in_progress: bool, untracked_in_owns: Vec<String>,
                  outside_owns: Vec<String>, generated_outside_owns: Vec<String>, protected_changed: Vec<String>, red_ok: Option<bool>, head: String },
    Commits { count: u32, head: String },
    Diff { stat: String, patch: String },
    Proof { red_failed: bool, head_passed: bool, matched: bool, red_tail: String, head_tail: String },
    Check { ok: bool, code: Option<i32>, timed_out: bool, tail: String, secs: u64 },
    Review { base: String, head: String, patch: String },   // patch: git diff base..head, clamped to REVIEW_DIFF_MAX
    Merged { commit: String }, Conflict { files: Vec<String> },
    CandidateRed { code: Option<i32>, timed_out: bool, tail: String, secs: u64 },
    RefMoved { reason: String },                    // run ref moved, or base rewritten (decision 21)
    AcceptConflict { files: Vec<String> },          // merge aborted; base untouched; run stays complete (decision 20)
    HandedBack { files: Vec<String> },
    Removed { salvage_ref: Option<String> },
    RefsOk,
    Finished { outcome: String },
    Failed { message: String },
}
pub enum Effect {
    Reply { reply: ReplyId, result: Result<String, String> },
    Op { run_id: String, op: OpId, kind: OpKind },
    Deliver { run_id: String, message_ids: Vec<u64>, window_id: u32, text: String },   // one new turn (decision 29)
    Interrupt { window_id: u32 },
    KillWindow { window_id: u32 }, RetireWindow { window_id: u32 }, RemoveWindow { window_id: u32 },
    WatchWorktree { root: PathBuf }, UnwatchWorktree { root: PathBuf },
    Persist { run_id: String, urgent: bool },
    WriteReport { run_id: String },
    Publish { structural: bool },
}
pub fn step(state: EngineState, event: Event) -> (EngineState, Vec<Effect>);
pub fn snapshot(state: &EngineState, now: u64) -> RunsSnapshot;   // run/snapshot.rs

// run/role_launch.rs (pure)
pub fn worker_spec(run: &Run, task: &Task) -> HeadlessSpec;                          // decisions 24-26
pub fn reviewer_spec(run: &Run, task: &Task, route: &Route) -> HeadlessSpec;
pub fn session_uuid(run_id: &str, op: OpId) -> String;                                // decision 24

// run/contract.rs (pure)
pub const WORKER_CONTRACT: &str; pub const REVIEWER_CONTRACT: &str;
pub fn worker_prompt(run: &Run, task: &Task) -> String;
pub fn handover_prompt(run: &Run, task: &Task, reason: &str, stat: &str, patch: &str) -> String;
pub fn reviewer_prompt(run: &Run, task: &Task, round: u32, base: &str, head: &str, patch: &str) -> String;
pub const REVIEW_DIFF_MAX: usize = 16 * 1024;   // head-and-tail clamp on a char boundary, as decision 30's handover diff
pub fn check_failed_message(command: &str, c: &CheckRecord) -> String;
pub fn proof_failed_message(command: &str, p: &ProofRecord, passed: &str) -> String;
pub fn review_changes_message(review: &ReviewRecord) -> String;
pub fn candidate_red_message(command: &str, c: &CheckRecord) -> String;
pub fn conflict_message(files: &[String]) -> String;
pub fn budget_wrap_up(spent: Spend, budget: Budget) -> String;
pub fn stall_nudge(minutes: u64) -> String;
pub fn rate_limit_continue(reason: &str) -> String;
pub fn answer_message(text: &str) -> String;
pub fn amend_message(task: &Task) -> String;
pub const DONE_NUDGE: &str; pub const NO_COMMIT_NUDGE: &str; pub const REVIEW_NUDGE: &str;
pub const RESUME_WORKER: &str; pub const RESUME_REVIEWER: &str; pub const RESUME_AFTER_EXIT: &str;
pub fn denied_text(n: u32, tool: &str, reason: &str) -> String;
pub fn generated_files_message(files: &[String]) -> String;       // decision 55
pub fn protected_file_message(files: &[String]) -> String;        // decision 56

// run/messages.rs (pure)
pub const MESSAGE_MAX_BYTES: usize = 32 * 1024;
pub const DELIVERY_RETRY_SECS: u64 = 5; pub const DELIVERY_MAX_FAILURES: u8 = 3;
pub fn clamp(text: &str) -> String;
pub fn join_turn(messages: &[&Outgoing]) -> String;   // queue order, blank line between, then clamp

// run/env.rs (pure)
pub fn profile_env(profile: &Profile, worktree: &Path) -> Vec<(String, String)>;   // {worktree} substituted

// run/git/ (blocking; call only from spawn_blocking). Every fn takes `git: &OsStr` first and `timeout: Duration` last.
pub fn preflight(git, dir, timeout) -> Result<Preflight, String>;                  // decision 17
pub fn project_settings(git, root, base_sha, claude: bool, codex_paths: Option<&[&str]>, timeout) -> Result<Vec<String>, String>; // decision 53: Claude's hooks/.mcp.json when `claude`; the Codex paths when Some
pub fn protected_files(git, root, base_sha, protected: &OwnsMatcher, timeout) -> Result<Vec<String>, String>;      // decision 56: git ls-tree -r --name-only
pub fn create_run_branch(git, root, branch, base_sha, path, timeout) -> Result<String, String>;
pub fn prepare_worktree(git, root, branch, from, path, timeout) -> Result<String, String>;   // HEAD sha; reuses; re-points a branch whose HEAD is an ancestor of `from`
pub fn lock_worktree(git, root, path, reason, timeout) -> Result<(), String>;
pub fn verify_done(git, worktree, start, run_head, owns: &[String], generated: &OwnsMatcher, protected: &OwnsMatcher, red: Option<&str>, timeout) -> Result<OpResult, String>; // spill diff is run_head...HEAD
pub fn count_commits(git, worktree, start, timeout) -> Result<(u32, String), String>;
pub fn diff_so_far(git, worktree, start, timeout) -> Result<(String, String), String>;
pub fn prepare_review(git, root, head_ref, base_ref, path, timeout) -> Result<(String, String, String), String>; // (base, head, clamped patch)
pub enum CandidateStep { Tree(String), Conflict(Vec<String>) }
pub fn merge_tree(git, root, run_head, task_head, timeout) -> Result<CandidateStep, String>;
pub fn commit_tree(git, root, tree, parents: &[&str], message, timeout) -> Result<String, String>;
pub fn materialize(git, integration, commit, timeout) -> Result<(), String>;     // checkout --detach --force; clean -fd
pub fn cas_update(git, root, branch, new, old, timeout) -> Result<bool, String>; // false: the ref was not <old>
pub fn reattach(git, integration, branch, timeout) -> Result<(), String>;
pub fn read_ref(git, root, refname, timeout) -> Result<Option<String>, String>;
pub enum RefCheck { Ok, BaseAdvanced { to: String, commits: u32 }, Halt { reason: String } } // decision 21
pub fn guard_refs(git, root, base_branch, base_sha, run_branch, run_head, timeout) -> Result<RefCheck, String>; // run ref first; then base: equal, ancestor (advanced) or not (rewritten/deleted → Halt)
pub fn commits_since(git, root, from, to, limit, timeout) -> Result<(Vec<String>, u32), String>; // decision 20: git log --format='%h %an: %s' from..to, at most `limit`, and the total
pub fn hand_back(git, worktree, run_head, timeout) -> Result<Vec<String>, String>; // conflicted files; empty = clean
pub fn salvage(git, worktree, reference, message, timeout) -> Result<Option<String>, String>;
pub fn remove_worktree(git, root, path, timeout) -> Result<(), String>;          // unlock, remove --force, prune
pub fn delete_branches(git, root, prefix, timeout) -> Result<(), String>;
pub fn accept(git, root, base_branch, expected_base, run_branch, message, timeout) -> Result<AcceptOutcome, String>;
pub enum AcceptOutcome { Merged { commit: String }, Conflict { files: Vec<String> } } // Conflict: `git merge --abort` already ran
pub struct GitQueue;                                   // run/git/queue.rs
impl GitQueue { pub fn new() -> Self; pub async fn write<T: Send + 'static>(&self, repo: &Path,
                    f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String>; }
pub const LOCK_RETRY_DELAYS_MS: [u64; 5] = [200, 400, 800, 1600, 3200];

// run/exec.rs (blocking)
pub const CHECK_TAIL_LINES: usize = 200; pub const CHECK_SUMMARY_LINES: usize = 40;
pub struct ShellOutcome { pub ok: bool, pub code: Option<i32>, pub timed_out: bool, pub tail: String, pub secs: u64 }
pub fn run_shell(dir: &Path, command: &str, env: &[(String, String)], timeout: Duration) -> ShellOutcome;
pub fn summary(tail: &str) -> String;                 // last CHECK_SUMMARY_LINES lines
// run/proof.rs (blocking)
pub struct ProofOp { pub root: PathBuf, pub path: PathBuf, pub red: String, pub head: String, pub command: String,
                     pub passed: String, pub timeout_secs: u64, pub setup: Option<String>, pub env: Vec<(String, String)> } // OpKind::Proof's fields
pub fn run_proof(git: &OsStr, op: &ProofOp, git_timeout: Duration) -> OpResult;

// run/journal.rs (blocking)
pub enum JournalLine { Intent { op: OpId, kind: OpKind }, Done { op: OpId, result: OpResult } }
pub fn save_run(run: &Run) -> std::io::Result<()>;    // temp, fsync, rename, fsync dir
pub fn append(dir: &Path, line: &JournalLine) -> std::io::Result<()>;  // append + fsync
pub fn load_all(data_dir: &Path) -> (Vec<(Run, Vec<JournalLine>)>, Vec<String>);  // runs, problems
pub fn compact(dir: &Path, pending: &BTreeMap<OpId, PendingOp>) -> std::io::Result<()>;
pub const COMPACT_AFTER_BYTES: u64 = 1 << 20;

// run/reconcile.rs (blocking)
pub enum Reconciled { Replay(OpResult), NotStarted }
pub fn reconcile(git: &OsStr, run: &Run, journal: &[JournalLine], windows: &[WindowInfo], timeout: Duration)
    -> Vec<(OpId, Reconciled)>;

// run/driver.rs
pub struct RunContext { pub data_dir: PathBuf, pub worktrees_root: PathBuf, pub exe: PathBuf, pub socket_path: PathBuf,
                        pub orchestrator: config::Orchestrator, pub git_roots: Arc<dyn GitRoots>, pub git: OsString }
pub struct RunService { /* Mutex<EngineState>, event channel, reply map, snapshot watch, GitQueue, retire deadlines */ }
impl RunService {
    pub fn new(manager: Arc<WindowManager>, ctx: RunContext) -> Arc<Self>;
    pub async fn restore(&self);                      // decision 44: load, reconcile, Event::Restore
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> tokio::task::JoinHandle<()>;
    pub fn stop(&self);
    pub fn snapshots(&self) -> tokio::sync::watch::Receiver<RunsSnapshot>;
    pub async fn request(&self, request: RunRequest) -> RunReply;   // every RunRequest but Subscribe/Unsubscribe
}
pub const RETIRE_AFTER: Duration = Duration::from_secs(30);
pub const INTERRUPT_GRACE: Duration = Duration::from_secs(30);
pub const DONE_CHECK_GIT_TIMEOUT: Duration = Duration::from_secs(10);

// manager (changed; new file manager/headless.rs holds these methods)
impl WindowManager {
    pub async fn create_headless(self: &Arc<Self>, name: String, spec: HeadlessSpec, session: SessionArg,
                                 first_turn: String, project: PathBuf, worktree: PathBuf) -> anyhow::Result<WindowInfo>;
    pub fn apply_session_event(&self, id: u32, event: &SessionEvent);   // under the lock: status, tool, session id,
                                                                        // conversation inputs (decision 27), feed send
    pub async fn headless_send(&self, id: u32, text: &str) -> anyhow::Result<()>;     // decision 29: one new turn
    pub fn headless_interrupt(&self, id: u32) -> anyhow::Result<()>;
    pub async fn headless_resume(self: &Arc<Self>, id: u32, session_id: &str, message: &str) -> anyhow::Result<()>;
    pub fn headless_kill(&self, id: u32) -> anyhow::Result<()>;                        // group SIGTERM, then SIGKILL
    pub fn signals(&self) -> broadcast::Receiver<WindowSignal>;
}
pub struct WindowSignal { pub window_id: u32, pub kind: WindowSignalKind }
pub enum WindowSignalKind { Session(SessionEvent), Hook { kind: HookKind, agent_id: Option<String> } }
pub const SIGNAL_CHANNEL_CAPACITY: usize = 4096;
// manager/entry.rs: Process::Headless(headless::HeadlessHandle); Entry.headless: Option<HeadlessSpec> (persisted in WindowRecord.run).
// manager/mod.rs: handle_hook leaves a Headless window's status alone (decision 27); the feed send for SubagentStart/Stop;
//                 tick's QUIET_AFTER rule skips Headless windows (decision 27).
// manager/conversation.rs: subscribe_conversation starts no transcript reader and sets no NoTranscriptPath for a Headless
//                 window (decision 27); apply_session_event reuses its conversation_hook and notify_conversations.
// state/mod.rs: WindowRecord gains #[serde(default)] kind: WindowKind.
// server.rs: decision 49's refusals (one call each into server/headless_guard.rs).
// server/git_wiring.rs (re-exported as server::GitWiring, with pump_git moved beside it): pub struct GitWiring { pub registry: Arc<GitRegistry>, pub publish_rx: mpsc::UnboundedReceiver<(PathBuf, Option<GitState>)> }
//            impl GitWiring { pub fn new(settings: config::Git) -> Self }
//            pub async fn serve(listener, manager, git: GitWiring, runs: Arc<RunService>, shutdown) -> anyhow::Result<()>

// crates/daemon/src/headless/mod.rs — the headless session layer, shared with M9's scouts, sub-planners and deciders
#[derive(Serialize, Deserialize)]
pub struct HeadlessSpec {
    pub runtime: Runtime, pub model: String, pub effort: Effort, pub cwd: PathBuf,
    pub instructions: String,                         // --append-system-prompt | developer_instructions
    pub mcp: Option<McpTarget>, pub allowed_tools: Vec<String>,
    pub claude_permission_mode: Option<String>,       // None: omit the flag
    pub claude_disallowed_tools: Vec<String>,          // --disallowedTools; a reviewer's Edit,Write,NotebookEdit (M8a.7)
    pub claude_sandbox: Option<ClaudeSandbox>,        // workers only, when worker_sandbox (decision 54)
    pub codex_sandbox: String, pub codex_writable_roots: Vec<PathBuf>,
    pub env: Vec<(String, String)>, pub claude_auth: ClaudeAuth, pub api_key_helper: Option<String>,
    pub run_ref: Option<RunRef>,                      // WindowInfo.run
}
#[derive(Serialize, Deserialize)]
pub struct ClaudeSandbox { pub writable_roots: Vec<PathBuf> }   // the worktree is writable by default; this adds the git common dir
#[derive(Serialize, Deserialize)]
pub struct McpTarget { pub role: AgentRole, pub run_id: String, pub task_id: Option<String> }
pub enum SessionArg { New { uuid: Option<String> }, Resume { session_id: String } }   // Codex New has no uuid
pub enum SessionEvent {
    Init { session_id: String, model: Option<String>, mcp_ok: Option<bool> },
    TurnStarted,
    UserText { text: String },                        // Claude echoes of what was sent, when the CLI replays them
    AssistantText { text: String, parent: Option<String> },
    ToolUse { id: String, name: String, input: serde_json::Value, parent: Option<String> },
    ToolResult { id: String, text: String, ok: bool, parent: Option<String> },
    ApiRetry { error: String, attempt: u32, delay_ms: u64 },
    PermissionDenied { tool: String, reason: String },
    Compacted,
    Other { kind: String },                           // recognised, nothing to act on (reasoning, thinking, rate-limit info)
    TurnEnded { outcome: TurnOutcome, usage: Option<TokenUsage>, denials: Vec<String> },
    Diagnostic { text: String },                      // a runtime's own error notice (Codex's `error` line); the driver logs it (M8a.7 fix round 1)
    Unknown { line: String },                         // first 300 characters
    StderrLine { line: String },                      // from the driver
    ProcessExited { code: Option<i32>, signal: Option<i32> },   // from the driver
}
pub enum TurnOutcome { Completed, Failed { error: String, kind: FailureKind }, Interrupted }
pub enum FailureKind { RateLimit, Authentication, Billing, SandboxUnavailable, Other }  // SandboxUnavailable: M8a.1 item 4b's text, in a failed result or in stderr before Init
// proto::TokenUsage::billable (decision 40) is defined in proto/src/run_info.rs, not here (orphan rule).
// headless/argv.rs (pure)
pub struct CliCaps { pub claude_verbose: bool, pub claude_permission_prompts: bool, pub claude_effort_flag: bool,
                     pub claude_non_bare_flag: Option<&'static str>, pub claude_interrupt: InterruptMode,
                     pub claude_hooks_fire_in_print: bool, pub codex_resume_takes_sandbox: bool,
                     pub claude_user_settings_only: Option<&'static [&'static str]>,   // the flags that exclude project settings (decision 53), None: impossible
                     pub claude_sandbox_keys: SandboxKeys,
                     pub codex_loads_project_config: bool,                              // decision 53, M8a.1 item 7a
                     pub codex_project_config_paths: &'static [&'static str],
                     pub codex_user_config_only: Option<&'static [&'static str]> }                            // decision 54's JSON keys, as M8a.1 finds them
pub struct SandboxKeys { pub enabled: &'static str, pub allow_unsandboxed: &'static str, pub write_allow: &'static str,
                         pub fail_if_unavailable: &'static str }   // M8a.1 item 4b; set by M8a.7
pub fn claude_settings(exe: &Path, window_id: u32, sandbox: Option<&ClaudeSandbox>, caps: &CliCaps) -> serde_json::Value; // launch::claude::settings plus the sandbox block
pub enum InterruptMode { ControlRequest, Sigint }
pub const CLI_CAPS: CliCaps;                           // every field set from M8a.1's findings
pub fn mcp_args(target: &McpTarget, window_id: u32, socket: &Path) -> Vec<String>;   // the `anthrex mcp` argv of the CLI section
pub fn claude_args(spec: &HeadlessSpec, session: &SessionArg, exe: &Path, window_id: u32, socket: &Path,
                   caps: &CliCaps) -> Vec<String>;
pub fn codex_args(spec: &HeadlessSpec, session: &SessionArg, message: &str, exe: &Path, window_id: u32,
                  socket: &Path, caps: &CliCaps) -> Vec<String>;
// headless/claude_stream.rs, headless/codex_stream.rs (pure)
pub fn parse_line(line: &str) -> Vec<SessionEvent>;  // codex_stream (stateless)
#[derive(Default)] pub struct ClaudeStream { /* the failed turn's pending category */ }
impl ClaudeStream { pub fn parse_line(&mut self, line: &str) -> Vec<SessionEvent>; }   // claude_stream (M8a.7: stateful)
pub fn user_message(text: &str, session_id: Option<&str>) -> String;   // claude_stream only: the stdin envelope, one line
pub fn interrupt_request(request_id: u64) -> String;                    // claude_stream only
// headless/conversation.rs (pure)
#[derive(Default)] pub struct StreamCursor { /* prompts sent, tool names by id, session id */ }
pub struct ConversationInput { pub hooks: Vec<ParsedHook>, pub records: Vec<crate::transcript::Record> }
pub fn map(runtime: Runtime, hooks_fire: bool, event: &SessionEvent, cursor: &mut StreamCursor) -> ConversationInput;
pub fn sent_turn(runtime: Runtime, hooks_fire: bool, text: &str, cursor: &mut StreamCursor) -> ConversationInput; // the turn the daemon starts
pub fn observe_hook(runtime: Runtime, hook: &ParsedHook, cursor: &mut StreamCursor) -> ConversationInput; // a real hook the manager applied (M8a.7 fix round 2)
// headless/status.rs (pure)
pub struct HeadlessStatus { pub status: Status, pub tool: Option<String>, pub turn_open: bool, pub rate_limited: bool,
                            pub before_retry: Status }   // the status a pending retry interrupted (M8a.7 fix round 1)
pub fn next(current: &HeadlessStatus, event: &SessionEvent) -> HeadlessStatus;
// headless/session.rs (I/O)
pub struct HeadlessHandle { /* runtime, current child (pid, group), stdin writer thread sender, reader task, ended */ }
impl HeadlessHandle {
    pub fn ended() -> Self;
    pub fn spawn(program: &OsStr, args: &[String], cwd: &Path, env: &[(String, String)],
                 on_event: impl Fn(SessionEvent) + Send + Sync + 'static) -> anyhow::Result<Self>;  // own process group
    pub fn send_line(&self, line: String) -> anyhow::Result<()>;   // queued to the writer thread; never blocks;
                                                                   // Err once WRITER_QUEUE_MAX lines are queued
    pub fn close_stdin(&self);
    pub fn interrupt(&self, mode: InterruptMode, request_id: u64) -> anyhow::Result<()>;
    pub fn kill(&self, grace: Duration);               // SIGTERM to the group, SIGKILL after grace, on a thread
    pub fn pid(&self) -> Option<u32>;
}
pub const STDOUT_LINE_MAX: usize = 4 * 1024 * 1024;    // longer lines are cut and parsed as Unknown
pub const WRITER_QUEUE_MAX: usize = 256;               // lines queued to a session's stdin writer thread
pub const SCRUB_PREFIXES: &[&str] = &["CLAUDE_CODE_"];
pub const SCRUB_NAMES: &[&str] = &["CLAUDECODE"];
```

**Stream to `SessionEvent`** (`headless/claude_stream.rs`, `headless/codex_stream.rs`). These are the working shapes from the CLI documentation (research notes, "Agent control interfaces"). M8a.1 pins each one against a recorded fixture, and a field it finds under another name is renamed here and in the parser, not worked around.

| Runtime | Line | Events |
|---------|------|--------|
| Claude | `{"type":"system","subtype":"init","session_id":…,"model":…,"mcp_servers":[{"name":…,"status":…}]}` | `Init { mcp_ok: Some(status of "anthrex" == "connected") }` |
| Claude | `{"type":"assistant","message":{"content":[…]},"parent_tool_use_id":…}` | per block: `text` → `AssistantText`; `tool_use {id,name,input}` → `ToolUse`; `thinking` → `Other` |
| Claude | `{"type":"user","message":{"content":[…]},"parent_tool_use_id":…}` | per block: `tool_result {tool_use_id,content,is_error}` → `ToolResult` (content string, or its text parts joined, cut to 4 KiB; `ok = !is_error`); `text` → `UserText` |
| Claude | `{"type":"system","subtype":"api_retry","attempt":…,"retry_delay_ms":…,"error":…}` | `ApiRetry` |
| Claude | `{"type":"system","subtype":"permission_denied",…}` | `PermissionDenied { tool, reason }` |
| Claude | `{"type":"system","subtype":"compact_boundary",…}` | `Compacted` |
| Claude | `{"type":"result","subtype":…,"is_error":…,"usage":{…},"permission_denials":[…]}` | `TurnEnded`: `Completed` when `subtype == "success"` and not `is_error`; `Interrupted` when M8a.1's interrupted marker is present; else `Failed { kind }` by the error category (`rate_limit`, `authentication_failed`, `billing_error`, else `Other`), which M8a.1 found on the synthetic `assistant` line before the `result` (its `"error"` key), so `ClaudeStream` carries it to the `result`; `SandboxUnavailable` when the text holds `sandbox required but unavailable` |
| Codex | `{"type":"thread.started","thread_id":…}` | `Init` |
| Codex | `{"type":"turn.started"}` | `TurnStarted` |
| Codex | `item.completed` with `item.type == "agent_message"` | `AssistantText` |
| Codex | `item.started` / `item.completed` with `command_execution {id, command, aggregated_output, exit_code}` | `ToolUse { name: "Bash", input: {"command": command} }` / `ToolResult { ok: exit_code == 0 }` |
| Codex | the same for `mcp_tool_call {id, server, tool, arguments, result}` | `ToolUse { name: "mcp__<server>__<tool>", input: arguments }` / `ToolResult` |
| Codex | `file_change {id, changes, status}` (completed only) | `ToolUse { name: "apply_patch", input: {"changes": changes} }` then `ToolResult { ok: status == "completed" }` |
| Codex | `web_search`, `reasoning`, `todo_list` items | `ToolUse { name: "WebSearch" }` for `web_search`; `Other` for the rest |
| Codex | `{"type":"turn.completed","usage":{"input_tokens","cached_input_tokens","output_tokens"}}` | `TurnEnded { Completed }` |
| Codex | `{"type":"turn.failed","error":{"message":…}}` | `TurnEnded { Failed { kind } }`, kind by the message patterns M8a.1 records |
| Codex | `{"type":"error","message":…}` | `Diagnostic { text: message }`, which the session driver logs (M8a.7 fix round 1) |
| both | anything else, or invalid JSON | `Unknown` |

**`SessionEvent` to milestone 6.5's conversation model** (`headless/conversation.rs`). Records always go to `ConversationSet::enrich`. Hooks are synthesised only when `hooks_fire` is false (every Codex session, and Claude only if M8a.1 finds its turn hooks do not fire in `-p`), with `source: HookSource::Stream`, `session_source: None` and `session_id` from `Init`. A synthesised `tool_response` is bounded exactly as `anthrex hook` bounds a real one (M6.5.2), setting `tool_result_truncated` and `tool_result_stringified`. On `main` that bounding is private to the CLI binary (`crates/cli/src/hook.rs`: `TOOL_RESULT_SUMMARY_MAX` at `:23`, `bound_tool_response` at `:111`, `bound_tool_response_value` at `:141`), which the daemon cannot call. M8a.7 therefore moves those three items, unchanged, into `proto::conversation` (pure; `crates/cli` and `crates/daemon` both depend on `proto`), `hook.rs` calls them from there, and their existing tests move with them; `hook.rs`'s static assertion against `config::CONVERSATION_MAX_RESULT_BYTES_MIN` stays where it is.

| Input | `transcript::Record` | Synthesised `ParsedHook` |
|-------|----------------------|--------------------------|
| the daemon starts a turn with text `T` (`sent_turn`) | `UserText { session_id, ordinal: k, text: T, human: true }`, where `k` is the number of turns sent in this session before this one (0-based, M6.5 ruling R1); `human: true` because the hook-built prompt is exactly `T`, so M6.5's exact-match alignment applies | `UserPromptSubmit { prompt: T }` |
| `Init` | — | `SessionStart` |
| `AssistantText`, no parent | `AssistantText { session_id, ordinal: k }`, `k` the latest sent turn's | — |
| `ToolUse`, no parent | `ToolDetail { tool_use_id: id, input: Some(input) }` | `PreToolUse { tool_name, tool_input, tool_use_id }` |
| `ToolResult`, no parent | `ToolDetail { tool_use_id: id, detail: Some(text), ok: Some(ok) }` | `PostToolUse { tool_name` (from the cursor)`, tool_use_id, tool_response: {"output": text}` or `{"error": text} }` |
| `ToolUse` / `ToolResult` with a parent | the same `ToolDetail` (it joins by id) | — (M6.5 builds sub-agent conversations from `SubagentStart`/`SubagentStop`) |
| `AssistantText` with a parent | — | — |
| `TurnEnded` | — | `Stop` |
| anything else | — | — |

**Reconcile per kind** (`run/reconcile.rs`, decision 44):

| `OpKind` | Reality checked | Replay | Otherwise |
|----------|-----------------|--------|-----------|
| `CreateRunBranch`, `PrepareWorktree` | `git worktree list --porcelain` lists the path on the branch | `Worktree { head }` if `setup` is `None`; else `NotStarted` (setup re-runs; it is idempotent) | `NotStarted`; a partial directory not in the list is removed and `git worktree prune` runs |
| `CreateWindow` | a restored headless window whose `RunRef` equals the round's; any live process with the session's id on its command line (killed, decision 28) | `Window { window_id }` (its session is ended; resume restarts it) | `NotStarted` |
| `ResumeSession` | a live process with the session id on its command line (killed) | — | `NotStarted` |
| `VerifyDone`, `CountCommits`, `DiffSoFar`, `Proof`, `Check`, `PrepareReview` | none | — | `NotStarted` |
| `MergeCandidate` | run branch head; the integration worktree's `HEAD` | head's parents are `(expected_run_head, task_head)` → `Merged { commit }`; head is `expected_run_head` → `NotStarted` after `reattach` | any other head → `RefMoved` |
| `HandBack` | `MERGE_HEAD` in the task worktree | present → `HandedBack { files: <conflicted> }`; already a merge commit of the run head → `HandedBack { files: [] }` | `NotStarted` |
| `RemoveWorktree` | the path; the salvage ref | path gone → `Removed { salvage_ref }` (the ref if it exists) | `NotStarted` |
| `Accept` | `merge-base --is-ancestor <run branch> <base>`; `MERGE_HEAD` in `root` | ancestor → `Finished { outcome: "accepted as <base head7>" }` | `MERGE_HEAD` equal to the run branch head (a crash inside a conflicted accept) → `git merge --abort`, then `NotStarted`; otherwise `NotStarted` |
| `Discard`, `VerifyRefs` | none (idempotent) | — | `NotStarted` |

### MCP tools (`crates/mcp`, new)

```rust
pub struct McpOptions { pub role: proto::AgentRole, pub run_id: String, pub task_id: Option<String>,
                        pub window_id: u32, pub socket: PathBuf }
pub const TOOL_REPLY_TIMEOUT: Duration = Duration::from_secs(100);
pub fn tools_for(role: proto::AgentRole) -> Vec<rmcp::model::Tool>;   // tools.rs
pub async fn forward(opts: &McpOptions, tool: &str, args: serde_json::Value) -> (bool, String);
pub async fn serve_on<R, W>(opts: McpOptions, reader: R, writer: W) -> anyhow::Result<()>
    where R: tokio::io::AsyncRead + Unpin + Send + 'static, W: tokio::io::AsyncWrite + Unpin + Send + 'static;
pub async fn serve_stdio(opts: McpOptions) -> anyhow::Result<()>;
```

Forwarding is the refreshed M8 brief's decision 41: a fresh socket connection per call (2 s connect timeout), `Hello { client: ClientKind::Mcp }`, `ClientMsg::Run(RunRequest::Tool(..))`, wait up to `TOOL_REPLY_TIMEOUT` for `RunReply::ToolResult`, skipping anything else; daemon down → `isError: true`, `cannot reach the anthrex daemon at <socket>: <error>`. A tool outside the role's list never reaches the daemon: `tool <tool> is not available to the <role> role`. `tools_for(Orchestrator)` is empty. Every schema is `{"type":"object","additionalProperties":false,…}`:

| Role | Tool | Description | Properties (required in bold) |
|------|------|-------------|-------------------------------|
| worker | `task_done` | `Tell the engine the task is complete and committed. For a tdd task, name the test and the red commit.` | **`summary`** string 1–4000; `test` string 1–300; `red` string matching `^[0-9a-f]{7,40}$` |
| worker | `task_blocked` | `Tell the engine you cannot continue, and why.` | `kind` enum `question`, `mis_sized`, `environment`; **`reason`** string 1–4000 |
| reviewer | `submit_review` | `Submit your verdict and findings for this review round. Call it once.` | **`verdict`** enum `approve`, `changes`; **`summary`** string 1–4000; **`findings`** array ≤ 50 of objects with **`severity`** enum `critical`, `important`, `minor`; `file` string 1–500; `line` integer ≥ 1; `input` string 1–2000; **`text`** string 1–2000 |

Engine-side acceptance texts (each a `ToolResult { ok: false }`): `unknown run <id>`; `run <id> is paused; the user must resume it`; `run <id> is <state>`; `this window is not the current worker of task <id>`; `this window is not the reviewer of task <id>`; `task_done is accepted only while the task is working (it is <state>)`; `submit_review is accepted only while the task is under review (it is <state>)`; `a review for round <n> was already submitted`; `invalid arguments: <field>: <problem>`; plus decision 32's and 35's texts. Successes: decision 32's texts and `Review recorded. You are done; end your turn now.`

### Contracts and message texts (exact)

```text
WORKER_CONTRACT:
You are a worker in an anthrex orchestration run.
1. Work only in this worktree and only in the paths this task owns. Changing files outside them stops the task.
2. Follow the test mode in your task prompt. For tdd: write the named test first, commit it while it fails (that commit is the red commit), then make it pass.
3. Commit your work on this branch with clear messages. Never push, switch branches, or rewrite commits already there. Commit new files: untracked files are not part of your work.
4. Use sub-agents to read and explore if you like; do all writing yourself.
5. When the task is complete and committed, call the anthrex tool task_done with a summary (and, for tdd, the test and the red commit). Then stop.
6. If you cannot continue, call task_blocked: kind question if you need an answer, mis_sized if the task is bigger than one task, environment if a tool or setup is broken. Then stop.
7. If you believe a review finding is wrong, do not fix it: call task_blocked with kind question and say why.
8. Messages that start with [anthrex] come from the orchestration engine. Do what they say, commit, and call task_done again.
9. Nobody can answer a permission prompt. If a tool is denied, work without it or call task_blocked with kind environment.

REVIEWER_CONTRACT:
You are a reviewer in an anthrex orchestration run.
1. This worktree is checked out at the change's head. Do not edit, create or delete files, and do not commit.
2. The change's diff is in your prompt. If it was clamped, or you need history, use git diff, git log or git show in this directory. Judge it against the brief and every acceptance criterion in your prompt.
3. Read the diff; do not run the build. The engine has already run the check, and its summary is in your prompt.
4. Call the anthrex tool submit_review exactly once, with verdict approve or changes, a summary, and findings.
5. Every finding has a severity. critical: wrong or unsafe, must not merge. important: must be fixed before merging. minor: worth noting, does not block. Each critical or important finding must name a file and line, or a failing input.
6. Use changes only when there is at least one critical or important finding. Earlier rounds' findings, if listed, must each be confirmed fixed.
```

| Name | Text |
|------|------|
| `check_failed_message` | `[anthrex] The check failed (<exit <code> \| timed out after <m> minutes>): <command>` / `Last 40 lines:` / the summary / `Fix it, commit, then call task_done again.` |
| `proof_failed_message` | `[anthrex] The test proof failed: <reason>` where reason is one of `at the red commit <red7> the test passed, so it does not fail without your change`, `at your head <head7> the test failed`, `the output did not show that <test> ran and passed (expected a line matching <regex>)`, `this is a tdd task and no test or red commit was named; call task_done with test and red` / `Command: <command>` / `Last 40 lines:` / tail / `Fix it, commit, then call task_done again.` |
| `review_changes_message` | `[anthrex] Review round <n> asked for changes. Fix every finding below, commit, then call task_done again.` then one line per critical or important finding: `- [<severity>] <file>:<line> <text>` or `- [<severity>] input <input>: <text>` |
| `candidate_red_message` | `[anthrex] Your branch merged cleanly into the run branch, but the check failed on the merged result (<exit …>): <command>` / `Last 40 lines:` / summary / `Fix it on your branch, commit, then call task_done again.` |
| `conflict_message` | `[anthrex] Your branch conflicts with the run branch. The run branch has been merged into your worktree with conflict markers left in:` / `- <file>` per file / `Resolve every conflict, commit the merge, then call task_done again.` |
| `DONE_NUDGE` | `[anthrex] Your turn ended with commits on your branch and no task_done. If the task is complete, call task_done now (for a tdd task, with test and red). If you are stuck, call task_blocked.` |
| `NO_COMMIT_NUDGE` | `[anthrex] Your turn ended and your branch has no commit yet. Continue the task and commit, or call task_blocked with the reason.` |
| `REVIEW_NUDGE` | `[anthrex] Your turn ended without a verdict. Call submit_review now, exactly once.` |
| `stall_nudge` | `[anthrex] Your last turn was interrupted after <n> minutes without any progress. Continue the task, or call task_blocked if you cannot.` |
| `budget_wrap_up` | `[anthrex] This task has used its budget (<calls>/<limit> tool calls, <m>/<limit> minutes). Wrap up now: commit what works and call task_done, or call task_blocked with kind mis_sized.` |
| `rate_limit_continue` | `[anthrex] Your last turn stopped on an API error (<reason>). Continue the task where you left off.` |
| `answer_message` | `[anthrex] Answer to your question: <text>` |
| `amend_message` | `[anthrex] The task was amended.` / `Brief: <brief>` / `Acceptance criteria:` / `- <item>` per item / `Continue with the amended task.` |
| `RESUME_WORKER` | `[anthrex] The daemon restarted. Re-read your task above, continue, commit, and call task_done when complete.` |
| `RESUME_REVIEWER` | `[anthrex] The daemon restarted. Finish your review and call submit_review.` |
| `RESUME_AFTER_EXIT` | `[anthrex] Your session's process stopped in the middle of a turn and has been resumed. Check the state of your worktree, continue, commit, and call task_done when complete.` |
| `denied_text` | `the agent was denied <n> times; last: <tool>: <reason>` |
| `generated_files_message` | `[anthrex] task_done rejected: you changed generated files outside this task's owns: <files>. Revert them (git checkout <start> -- <files>, then commit), or this task must own them. Then call task_done again.` |
| `protected_file_message` | `[anthrex] task_done rejected:` / per file: `<path> configures or instructs future agents; this task may change it only if its owns names it exactly` / `Revert it and call task_done again, or ask for the plan to be amended.` |

`worker_prompt`: `[anthrex] Task <id>: <title>`, `Run goal: <goal>`, `Worktree: <path>`, `Branch: <branch>`, `Start commit: <sha7>`, `Size: <S|M>` (`, hub` when hub), `Test mode: <mode>` and, for tdd, `Test to write: <test_to_write>` when set and `Single-test command: <single_test>`, `Check command: <check>` (omitted when none), blank line, `This task owns:` with `- <glob>` lines, `Acceptance criteria:` with `- <item>` lines, blank line, the brief last. `reviewer_prompt`: `[anthrex] Review task <id> "<title>", round <n>, level <small|medium|frontier>.`, `Base: <sha7>`, `Head: <sha7>`, `Test mode: <mode>` (tdd adds `Look first for tests that were weakened or made trivial to pass.`), `small` adds `Review the diff only.`, `Acceptance criteria:` lines, `Diff (<base7>..<head7>):` block with the clamped patch (ending `[diff clamped: <n> bytes omitted; read the rest with git diff <base7>..<head7>]` when it was cut), `Last check (40 lines):` block when a check ran, `Earlier findings to confirm fixed:` lines when round > 1, blank line, the brief last.

### CLI

```
anthrex run start --plan <file> [--yes] [--trust-project]
anthrex run status [<run>] [--json]
anthrex run approve <run>
anthrex run reject <run> [--confirm <run-id>]
anthrex run edit <run> --file <edits.toml>
anthrex run retry <run> <task>
anthrex run override <run> <task> --reason <text>
anthrex run cancel <run>
anthrex run resume <run> [--rebaseline]
anthrex run accept <run> [--yes] [--base <sha>]
anthrex run discard <run> [--confirm <run-id>]
anthrex mcp --role <worker|reviewer> --run <run> [--task <task>] --window <id> [--socket <path>]   (hidden)
```

- `--trust-project` sets `RunRequest::Start.trust_project` (decision 53).
- `--dir` is the existing global flag. `run start` auto-starts the daemon, prints the run id on stdout, and on stderr either `approve with: anthrex run approve <id>` followed by the plan table (`status` format), or `watch with: anthrex run status <id>` with `--yes`.
- Every run request uses `CliClient::request_with_timeout` with `RUN_REQUEST_TIMEOUT` = 180 s (preflight's six git calls at the 60 s default could exceed a shorter bound; `start` is the slowest request).
- `accept` asks `merge anthrex/<id>/integration into <base> in <root>? [y/N]` unless `--yes`. When the reply is a `ConfirmNeeded` with `base_moved` (decision 20), it prints `<base> moved since the run started (<from7>..<to7>, <total> commits):`, the listed commits one per line indented two spaces, `… and <total - 50> more` when capped, and asks `merge onto <base> at <to7> including these commits? [y/N]`; `--yes` does not answer this question, only `--base <to sha>` naming the listed head does (for scripts). A yes resends with `confirm = "<id>@<to>"`. An accept conflict prints the daemon's message and exits 1; `discard` and `reject` ask the user to type the run id unless `--confirm`. A wrong id: `confirmation does not match the run id`, exit 1.
- `status` text, one block per run, newest first:

```
add-reset-3f9a  running  2/4 merged  base main@1a2b3c4  writers 2/3  readers 1/3  rev 57
  goal: Add password reset
  report: /Users/me/Library/Application Support/anthrex/runs/add-reset-3f9a/REPORT.md
  ID   SIZE MODE   STATE          RUNG BOUNCES        ROUTE                          WINDOWS
  t1   M◆   tdd    merged         0    -              claude claude-opus-5 high      4 5
  t2   M    tdd    review         1    review 1       codex (default) medium         6 9
  t3   S    check  queued         0    -              claude claude-sonnet-5 low
  t4   S    none   blocked        3    check 3        claude claude-sonnet-5 low     7
  attention: t4 blocked (mis_sized): check failed 3 times
```

  Columns: id padded to 5, size to 5 (`◆` suffix for hub), mode to 7, state label to 15, rung to 5, bounces to 15 (`-`, or `<gate> <n>` joined with `, `), route to 31 (`<runtime> <model or (default)> <effort>`), then the live and past window ids. A paused run shows `paused (from running)`, a halted one `halted: <reason>` on its own line. `--json` prints `serde_json::to_string_pretty(&RunsSnapshot)`.

### File sizes this milestone must respect

AGENTS.md rule 8 puts the limit at about 600 lines. Counts on `main` at `6f22681` (re-counted 2026-09-23):

| File | Lines | Note |
|------|------:|------|
| `crates/config/src/lib.rs` | 600 | **At the limit.** M6.5 split it into `lib.rs`, `conversation.rs` (318) and `git.rs` (154). Add only `mod orchestrator;`, one `pub use`, the field, its default, the `orchestrator::read` call and the `report_unknown_keys` arm, at most 8 lines; everything else, `[orchestrator.claude]` included, in `orchestrator.rs`. |
| `crates/config/src/lib_tests.rs` | 741 | **Already over** (recorded in the follow-ups by M6.5's review). Only the `[orchestrator]` lines of `every_key_is_read` change; every new config test goes in `orchestrator_tests.rs`. |
| `crates/daemon/src/server.rs` | 605 | **Already over.** Net growth must be zero or less: decision 49's refusals are one helper call per message kind, the helper in `server/headless_guard.rs` (task M8a.17); the run requests in `server/run_api.rs` (task M8a.22); `GitWiring` replaces the registry construction and takes `pump_git` with it into `server/git_wiring.rs`, which frees more lines than the calls add. |
| `crates/daemon/src/manager/mod.rs` | 489 | `handle_hook`'s headless branch, `tick`'s headless skip and the feed send only; every headless method is in new `manager/headless.rs`. |
| `crates/daemon/src/manager/conversation.rs` | 445 | `subscribe_conversation`'s headless skip (decision 27) only. |
| `crates/daemon/src/manager/entry.rs` | 340 | The `Process::Headless` arm in each routing method (`write_input`, `resize`, `size`, `attach`, `snapshot`, `signal_group`, `pid`) and `Entry.headless`. |
| `crates/daemon/src/manager/restore.rs` | 512 | Parse `kind` and `run` into `Process::Headless(HeadlessHandle::ended())` and `Entry.headless`; at most 20 lines. |
| `crates/daemon/src/manager/create.rs`, `restart.rs` | 563, 556 | **Untouched.** Headless windows never go through `create` or `restart`. |
| `crates/daemon/src/state/mod.rs` | 203 | `WindowRecord.kind`. |
| `crates/tui/src/app/daemon.rs` | 141 | Add a `DaemonMsg::Run(_) => vec![]` arm to `App::on_daemon`'s exhaustive match (M6.5 moved `on_daemon` here from `app/mod.rs`). |
| `crates/tui/src/app/mod.rs` | 515 | One `is_headless` guard in `focus` (the `Subscribe` at `:344`) and at each of the two `ClientMsg::Input` sites (`:362`, `:458`); at most 10 lines. |
| `crates/tui/src/app/link.rs` | 268 | Skip `Subscribe` for a headless window in `retry_dropped_subscribe` and `on_reconnected` (decision 49). |
| `crates/tui/src/mouse.rs`, `crates/tui/src/ui/terminal.rs` | 226, 90 | The one `ClientMsg::Input` site in `mouse.rs` (`:89`); the placeholder text in `ui/terminal.rs`. |
| `crates/cli/src/client.rs` | 541 | Untouched; `run_cmd.rs` uses `request_with_timeout`. |
| `crates/cli/src/main.rs` | 582 | The `Run` and hidden `Mcp` variants and their dispatch only, with their clap argument types defined in `run_cmd.rs`; at most 15 lines here. |
| `crates/cli/src/hook.rs` | — | Loses `TOOL_RESULT_SUMMARY_MAX`, `bound_tool_response` and `bound_tool_response_value` to `proto::conversation` (decision 27's mapping table note); net shrink. |
| `crates/proto/src/messages.rs` | 506 | Two variants, `HookSource::Stream`, and their round-trip cases only (decision 3). |
| `crates/proto/src/conversation.rs` | 282 | Gains the moved tool-response bounding (about 90 lines). |
| `crates/proto/src/types.rs` | 384 | `WindowKind`, two `WindowInfo` fields, one `ClientKind` variant. |
| `crates/daemon/src/lifecycle.rs` | 420 | `GitWiring`, `RunService` construction, restore, stop. |
| `crates/daemon/src/status.rs`, `hooks.rs`, `launch/mod.rs`, `window.rs` | 473, 399, 427, 371 | **Untouched.** Headless status is `headless/status.rs`; the PTY status machine and launch path are M9's to amend for the orchestrator. `hooks::accepts` (`hooks.rs:57`) already returns `false` for a source it does not list. |
| `crates/fake-agent/src/main.rs` | 342 | Mode dispatch only; the headless modes go in new `headless.rs`, `stream_claude.rs`, `stream_codex.rs`, `roles.rs`, `mcp.rs`, and the step additions in `script.rs` (213). |
| `crates/fake-agent/tests/script.rs` | 681 | **Already over.** Only `mcp_call_is_not_supported_yet` is removed; the headless tests go in new `tests/headless_modes.rs`. |
| `scripts/pty-smoke.py` | 1770 | Already far over; the new stage is a new module `scripts/pty_smoke_run.py`, imported and called from `pty-smoke.py` in ≤ 10 lines, like `scripts/pty_tree_smoke.py`'s stages. |

No file under `crates/daemon/src/run/`, `crates/daemon/src/headless/`, `crates/mcp/src/` or `crates/proto/src/run*.rs` may exceed 600 lines. The engine is split by responsibility: `engine/mod.rs` (types, `step` dispatch), `engine/requests.rs` (start, approve, reject, edit, retry, override, cancel, resume, finish), `engine/dispatch.rs` (scheduler, slots, worktree and window launch), `engine/done.rs` (task_done, task_blocked, turn-end fallback, stall, rate limits, failed turns, denials, process exits, signals), `engine/gates.rs` (proof, check, review), `engine/merge.rs` (merge queue, hand-back, ref guard, completion), `engine/ladder.rs` (rungs, budgets, fresh sessions), `engine/outbox.rs` (turn-based delivery gate), `engine/restore.rs` (restore, pause, resume), with tests in `engine/tests/*.rs`.

## Tasks

Shared test helpers:

- **Engine unit tests** use `engine/tests/fixture.rs`: `Fixture::new(plan_toml)` builds a `Run` through `plan::build_run` with a `Preflight` whose three paths all differ (`root` `/tmp/x`, `project` `/tmp/p`, `git_common_dir` `/tmp/p/.git`, so a test cannot pass by reading one where another was meant) and `base_sha` `b0` × 20 hex, `wt_dir` `/tmp/wt`, `data_dir` `/tmp/data/runs/<id>`, and the default config; `fx.send(now, kind) -> Vec<Effect>` runs `step`; `fx.op(kind_name) -> (OpId, OpKind)` finds the latest matching `Effect::Op`; `fx.done(op, result)`; `fx.task(id)`; `fx.status(window, Status)` and `fx.signal(window, AgentSignal)`. Assertions match on returned effects and on the task and run fields.
- **Daemon git tests** use `crates/daemon/tests/support/mod.rs`'s `TempRepo`; a helper `recording_git(dir) -> PathBuf` writes a script that appends its argv and the `GIT_*` part of its environment to `<dir>/git.log` and execs the real `git`.
- **Tests that set a variable in the test process's own environment** (to prove a child does not inherit it) each live **alone in their own test binary**, following `crates/daemon/tests/worktree_env.rs` and `git_env.rs`. The daemon crate has no shared environment lock, and one would not help: in edition 2024 the hazard is `set_var` racing any process spawn on another libtest thread, which reads the whole environment block and takes no lock (`worktree_env.rs`'s module doc). This supersedes "under the shared environment lock" wherever a task below said it.
- **End-to-end tests** live in `crates/cli/tests/run_e2e_*.rs` and share `crates/cli/tests/support/run_harness.rs`, declared from `support/mod.rs`. `RunHarness::new(config_toml)`: a temp dir from `tempfile::Builder::new().prefix("ax-run").tempdir_in("/tmp")`; a repo at `<tmp>/repo` (`git init -b main`, repo-local `user.name`, `user.email`, `commit.gpgsign false`, one commit of `README`); the config written to `<tmp>/config.toml` with `[orchestrator] git_timeout_secs = 5` and `[orchestrator.profile] check_timeout_secs = 10`, plus the caller's additions (test plans never set `check_timeout_secs`); the daemon started with `anthrex daemon start` and `ANTHREX_SOCKET=<tmp>/d.sock`, `ANTHREX_DATA_DIR=<tmp>/data`, `ANTHREX_CONFIG`, `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` both `fake_agent_bin()`, `FAKE_AGENT_ARGS_FILE` and `FAKE_AGENT_STDIN_FILE` both `<tmp>/agent-io/` (a test reads `worker-t1-1.args`, `worker-t1-1.stdin`), `GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`, `ANTHREX_GIT=off` unless the test says otherwise. Methods: `script(name, steps: &[Value])` writes `<repo>/.git/fake-agent/<name>.jsonl`; `plan(toml) -> PathBuf` (outside the repo); `anthrex(args) -> Output`; `start(plan_toml, yes) -> String`; `subscribe() -> RunWatcher` (a raw socket client that sends `Subscribe` and keeps the latest `RunsSnapshot`); `wait_run(id, pred, RUN_WAIT) -> RunInfo` printing the last snapshot and the tail of `daemon.log` on timeout; `restart_daemon(extra_env)`; `git(args) -> String`. `Drop` runs `anthrex daemon stop` and waits for the socket to go. **`RUN_WAIT` = 300 s**, derived from the test configuration's own bounds, not observed cost: at most 40 sequential engine git calls on a task's path at `git_timeout_secs = 5` (200 s, `VerifyDone` included because it takes the smaller timeout), at most 4 sequential check or proof runs at `check_timeout_secs = 10` (40 s), and at most 20 s of scripted `wait_ms` on the critical path (an agent's idle wait for its next turn is not on it), together 260 s. That bound covers **one task path** (one session's work through every gate to its merge). A test waits `k * RUN_WAIT`, where `k` is the number of task paths its scenario runs one after another (a fresh session of the same task, or the part of a run after a daemon restart, is a path of its own); `k` is 1 unless listed: 2 for `e2e_two_rejections_…`, `e2e_stall_…`, `e2e_a_second_death_…`, `e2e_conflict_is_handed_back_…`, `e2e_red_candidate_…`, `e2e_moved_run_ref_…`, `e2e_base_rewritten_…`, `e2e_base_advanced_…` (per run), `e2e_daemon_restart_…` and each iteration of `e2e_crash_…`; 3 for `e2e_second_conflict_…` (`t2`, then `t3`, which waits for `t2` on `b/**`, then `t1`'s second candidate) and `e2e_mis_sized_…` (`t2`, then `t2a`/`t2b`, then `t3`). The earlier draft's flat `2 * RUN_WAIT` was below the legal worst case of both 3-path tests. A test whose scenario runs an engine timer on its critical path adds it to the bound by name, since `RUN_WAIT`'s derivation has no timer term: each `stall_after_secs` it waits out, each `rate_limit_retry_secs`, each `INTERRUPT_GRACE` a failed interrupt costs, and `RETIRE_AFTER` where it waits for a window to go. Add the `RUN_WAIT` row and the `k` rule to `docs/timing-budgets.md`.
- **Scripted agents.** `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` point at `fake-agent`, which picks its headless mode from its argv (M8a.20). A script's steps run inside the current turn, with these special cases:
  - `{"read_message":{"expect":…}}` ends the turn and waits for the next message: for Claude, the next stdin line; for Codex, the next `exec resume` process's argument, with the script position kept in a sidecar file.
  - `{"end_turn":{}}` ends the turn without waiting.
  - The end of the script ends the turn and leaves the agent idle (a Claude process waits for stdin; a Codex process exits 0).
  - A message that arrives after the script has ended gets an empty turn: an agent that ignores messages.

  `DONE` below abbreviates `{"mcp_call":{"tool":"task_done","args":{…}}}` at the end of a script.

### Scenario map (spec §21)

| §21 scenario | Test here | What is left to a later milestone |
|--------------|-----------|-----------------------------------|
| A green S task on the fast path | `e2e_green_s_task_runs_to_merged` (plan path, `--yes`) | the fast path itself — M8b |
| A TDD task whose red commit does not fail | `e2e_tdd_red_commit_that_passes_is_rejected` | — |
| A TDD task whose test passes without the named test running | `e2e_tdd_test_that_did_not_run_is_rejected` | — |
| A task that fails check once, then passes | `e2e_check_fails_once_then_passes` | — |
| A stall → rung 2 on the peer runtime | `e2e_stall_escalates_to_a_fresh_session_on_the_peer_runtime` | — |
| A mis-sized task → split by the orchestrator | `e2e_mis_sized_task_blocks_and_is_split_by_an_edit` (the user's `run edit` stands in for the orchestrator) | the orchestrator's `edit_plan` — M9 |
| A merge conflict → hand-back → resolved | `e2e_conflict_is_handed_back_and_resolved` | — |
| A second conflict → blocked | `e2e_second_conflict_blocks_the_task` | — |
| A candidate merge that is red although both branches were green | `e2e_red_candidate_goes_back_to_the_worker` | — |
| A cancel of a running task with dirty work (salvaged) | `e2e_cancel_of_a_dirty_running_task_is_salvaged` | — |
| The run ref moved behind the engine's back (run halts) | `e2e_moved_run_ref_halts_the_run` | — |
| The base branch advanced during the run (fast-forward: the run continues, accept lists the new commits for confirmation) | `e2e_base_advanced_during_run_continues_and_accept_lists_it` | — |
| The base branch rewritten during the run (run halts) | `e2e_base_rewritten_halts_the_run` | — |
| A daemon killed after each logged intent | `e2e_crash_after_each_intent_kind_reconciles` | — |
| A rate-limit event halving writers | `e2e_rate_limit_retry_is_not_a_stall_and_a_failed_turn_is_continued` (the event is seen, counted, never read as a turn end or a stall) | halving `max_writers` — M9.5 |
| A race whose loser is stopped and salvaged | — (salvage itself is covered by the cancel test) | racing — M9.5 |
| A test writer's red commit handed to an implementer | `proof_accepts_a_red_commit_from_an_earlier_session` (engine unit) and the rung-2 hand-over in `e2e_stall_…` | the test-writer pattern — M9.5 |
| A review rejected twice → fresh worker on the peer runtime → approved | `e2e_two_rejections_then_a_fresh_peer_worker_is_approved` | — |
| A worker that disputes a finding | `e2e_disputed_finding_blocks_as_a_question_and_the_answer_resumes_it` | the orchestrator answering — M9 |
| A user override merged without approval, marked in the report | `e2e_override_merges_without_approval_and_is_reported` | — |
| Input to a worker window refused by the daemon while engine messages still arrive | `e2e_headless_windows_refuse_client_control_while_the_engine_delivers` (headless form: `Subscribe`, `Input`, `Kill`, `Remove` and `Restart` refused, the engine's next turn still delivered) | — |
| A worker stuck on a prompt nobody can answer → blocked(environment) | `e2e_permission_denials_block_the_task_as_environment` (headless form: no prompt can be shown; `denials_before_block` denials block the task) | — |
| *(headless, beyond §21)* A process that dies mid-turn | `e2e_process_that_dies_mid_turn_is_resumed_once` | — |
| *(headless, beyond §21)* Resume after a daemon restart | `e2e_daemon_restart_pauses_and_resume_continues` | — |
| A plan edit that leaves an L task (rejected) | `e2e_plan_with_an_l_task_is_rejected`, `edit_leaving_an_l_task_is_rejected` | — |
| A sub-planner edit outside its area (rejected) | `edit_outside_its_area_is_rejected` (engine unit, `EditScope::Area`) | the sub-planner's `submit_epic` — M9 |
| Claude and Codex given overlapping `owns` (rejected) | `e2e_cross_runtime_overlapping_owns_are_rejected` | — |

### M8a.1 Verify the external tools and record the stream fixtures

**Files.** Create `crates/daemon/tests/fixtures/headless/` with `claude-<version>-stream.jsonl`, `claude-<version>-input.jsonl`, `claude-<version>-hooks.jsonl`, `claude-<version>-documented.jsonl` (item 3's shapes, `"observed": false`), `claude-<version>-project-settings.jsonl`, `claude-<version>-sandbox.jsonl`, `codex-<version>-exec.jsonl` (item 7's first turn, then the `-m no-such-model` run whose `turn.failed` M8a.7 reads), `codex-<version>-project-config.jsonl`, `codex-<version>-resume.jsonl`, and a `.meta.json` beside each with decision 51's keys (`runtime`, `cli_version`, `captured`, `redactions`, `note`, `command`, `observed`). Fill this brief's "Implementation notes".

**Tests first.** None: this task records facts and fixtures. It is the only task that runs the real `claude` and `codex`. It uses the implementer's own login and a few turns of tokens, in `/tmp/anthrex-m8a1/`, and nothing in CI ever does. Replace personal paths with `/tmp/fixture` before committing.

**Change.** Record each of the following with its command, the installed version and the relevant output. Together they set every field of `headless::argv::CLI_CAPS`.

1. **Claude flags.** From `claude --version` and `claude --help`:
   - `-p`, `--input-format stream-json`, `--output-format stream-json`, and whether stream-json output requires `--verbose` (`claude_verbose`).
   - `--permission-prompts` and its values (`claude_permission_prompts`).
   - `--session-id`, `--resume`, `--settings`, `--mcp-config`, and `--allowedTools` (one comma-separated value accepted).
   - `--append-system-prompt`, and `--permission-mode` with `acceptEdits`, `dontAsk` and `plan`.
   - `--effort` (`claude_effort_flag`), `--bare`, and any explicit opt-out of bare mode (`claude_non_bare_flag`, §23).
2. **A recorded Claude session.** In a scratch repository, start one process with decision 24's flags for a worker. The `--settings` hook command must append each payload to `claude-…-hooks.jsonl`, and `--allowedTools Bash,Read` omits `Write` on purpose. Then write three stream-json user messages, each after the previous `result`:
   1. "Run `ls` with Bash, then reply done".
   2. "Create x.txt with the Write tool", which must be denied.
   3. "Run `sleep 60` with Bash", interrupted after 5 s.

   Record stdout and every stdin line. Then answer:
   - The exact **user-message envelope** the CLI accepted, which fixes `claude_stream::user_message`.
   - Whether the process stays alive after `result` and exits on stdin EOF.
   - Whether the interrupt control request (`{"type":"control_request","request_id":…,"request":{"subtype":"interrupt"}}`) is accepted, and what it prints. If it is not accepted, what `SIGINT` does (`claude_interrupt`). What an interrupted turn's `result` looks like.
   - Whether `result.usage` is per turn or cumulative (compare the first two), and its field names.
   - The `permission_denied` event's shape and the result's `permission_denials`.
   - Which hooks fired: `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, and `SubagentStart` (ask for one Agent call) (`claude_hooks_fire_in_print`).
3. **Documented shapes that cannot be triggered on demand.** Copy these into `claude-<version>-documented.jsonl`, whose meta says `"observed": false` (a separate file, because one meta cannot mark some lines of a recording observed and others not), from the headless documentation for the installed version: `system/api_retry` (`error` values), `compact_boundary`, a `result` that failed on `rate_limit`, `authentication_failed` and `billing_error`.
4. **Claude resume.** `claude -p --resume <the recorded id>` with every flag re-passed accepts a fourth message. With a random UUID, record the exact "no such session" text (decision 28's failed-resume marker).
4a. **Project settings (decision 53).** In a scratch repository with a tracked `.claude/settings.json` holding a `PreToolUse` hook that writes a marker file, and a tracked `.mcp.json` naming a server whose command writes another marker:
   - Find the flags that exclude them under `-p`. Try `--setting-sources user` and `--strict-mcp-config` first, then whatever `claude --help` offers.
   - Record a session with decision 24's flags: neither marker appears; anthrex's `--settings` hooks still fire (they are in the hooks fixture); `system/init`'s `mcp_servers` lists only `anthrex`.
   - Record the same session without the flags: both markers appear. That shows the test would catch a regression.
   - Set `claude_user_settings_only` to the flags, or to `None` if no mechanism exists.
4b. **Worker sandbox (decision 54).** Find the sandbox settings keys in the settings documentation for the installed version: enable, unsandboxed commands disallowed, extra writable paths. Then, in a *linked* worktree, run a worker session with decision 54's `--settings` block and ask it to (1) `touch` a file one directory above the worktree, and (2) commit a file inside the worktree.
   - Record that (1) is refused, and how it appears in the stream: a `permission_denied` event, an entry in `permission_denials`, or only a failed `Bash` result.
   - Record that (2) succeeds.
   - Record the error text when the sandbox cannot start (for example, run once with `PATH` stripped of `bwrap` on Linux, if available).
   - Do this on macOS, and on Linux when one is available; otherwise record Linux as "not verified". Set `claude_sandbox_keys`.
5. **Claude authentication.** With `ANTHROPIC_API_KEY` unset, `-p` without `--bare` answers, which shows the login is used. With `--bare` it fails; record the text. If a key is available: under `--bare`, do `--settings` hooks and `--mcp-config` still apply (decision 50)? Otherwise record "not verified", and `api_key` stays refused at config load.
6. **Codex flags.** From `codex --version`, `codex exec --help` and `codex exec resume --help`: `--json`, `-s`, `-m`, `-c`, and whether `resume` accepts `-s`, `-c` and `-m` (`codex_resume_takes_sandbox`). Confirm these config keys exist in the native binary (`strings … | grep -E …`): `mcp_servers.<n>.default_tools_approval_mode`, `mcp_servers.<n>.tool_timeout_sec`, `developer_instructions`, `model_reasoning_effort`, `approval_policy`, `sandbox_workspace_write.writable_roots`.
7. **A recorded Codex session.** In a *linked* worktree of a scratch repository:
   - `codex exec --json -s workspace-write -c approval_policy="never" -c 'sandbox_workspace_write.writable_roots=["<git common dir>"]' -- "create a.txt and commit it"`. The commit must succeed. Without the writable root it must fail; record how the failure appears in the stream (decision 25, review focus 5).
   - `codex exec resume <thread_id> --json -- "append a line and commit"`.
   - Then:
     - Record both streams.
     - Record `turn.completed`'s usage field names.
     - Record `turn.failed`'s shape (`-m no-such-model`), and the rate-limit message pattern as documented.
     - Record what `SIGINT` to an `exec` in progress prints.
     - Record whether a write refused by `-s read-only` appears as a structured denial or only as a failed command.
7a. **Codex project config (decision 53).** In a scratch repository, commit a `.codex/config.toml` that sets something visible (for example `developer_instructions = "Always answer PROJECT-CONFIG-LOADED"`, or an MCP server whose command writes a marker file) and a `.codex/hooks.json` hook that writes a marker.
   - Record a `codex exec --json` turn and an `exec resume` turn with decision 25's flags into `codex-<version>-project-config.jsonl`: does either marker, or the instruction, appear?
   - Repeat after marking the directory trusted in the user's config, if Codex has trust levels, since trust may be what enables project config.
   - If it loads, look for a way to exclude it (a flag, or a `-c` override) in `codex exec --help` and the config reference, and prove it with the same recording.
   - Set `codex_loads_project_config`, `codex_project_config_paths` and `codex_user_config_only`, or record "not loaded" with the evidence.
8. **rmcp.** `cargo add rmcp@=3.4.0` in a throwaway crate outside the repository: decision 5's API names resolve.
9. **git.** `git --version` is at least 2.38. Record the output of `git merge-tree --write-tree --name-only --no-messages A B` for a clean pair and a conflicting pair in a scratch repository: the tree on line 1, the conflicted paths after it.

If a flag, key or event is missing, stop work on the item that uses it, as AGENTS.md says, record the evidence, and follow the fallback its decision names.

**Acceptance.** "Implementation notes" has a dated "M8a.1 external tools" entry covering every item, 4a, 4b and 7a included, and states every `CLI_CAPS` value with the item that set it. The nine fixtures and their meta files exist, and every `.meta.json` names the CLI version in `cli_version`.

**Commit.** `test(daemon): record the headless stream fixtures and the verified external tool facts`
### M8a.2 Protocol: run types, wire messages, the version bump

**Files.** Create `crates/proto/src/run.rs`, `run_info.rs`, `run_wire.rs`, `run_tests.rs`. Modify `crates/proto/src/lib.rs`, `messages.rs`, `types.rs`; every `WindowInfo { … }` construction in the workspace (39 sites in 22 files at `6f22681`, tests included; `rg -n "WindowInfo \{" crates`) gains `kind: WindowKind::Pty, run: None`; `crates/tui/src/app/daemon.rs` (a `DaemonMsg::Run(_) => vec![]` arm in `App::on_daemon`, whose match is exhaustive); `crates/daemon/src/server.rs` (one `ClientMsg::Run(_)` arm giving the temporary refusal below; its match is exhaustive too). `crates/cli/src/client.rs` needs no change: `request_with_timeout` (`client.rs:182`) returns any reply that is not a `WindowsChanged` or `Git` broadcast, with no exhaustive match on `DaemonMsg`.

**Tests first**, in `run_tests.rs` and `messages.rs`:

- `plan_parses_the_brief_example`: decision 7's example parses with `toml::from_str::<Plan>` (dev-dependency `toml`) into one task with `size == Size::M`, `route.strength == Some(Strength::Standard)` and `profile.env["CARGO_TARGET_DIR"] == "{worktree}/target"`.
- `plan_rejects_unknown_fields_at_every_level`: an unknown key at top level, in `[profile]`, in `[[task]]`, in `[task.route]` and in `[task.budget]` each fails and the error names the key.
- `size_serializes_as_a_capital_letter` (`"S"`, `"M"`, `"L"`); `strength_and_effort_order` (`Fast < Standard < Frontier`, `Low < Medium < High`, `Effort::High.raised() == None`, `Size::M.raised() == Size::L`).
- `plan_edits_parse_from_an_edit_file`: an `EditFile` with one of each of the nine ops round-trips through TOML; `op = "approve_task"` is rejected.
- `states_serialize_snake_case`: `TaskState::MergeQueue` is `"merge_queue"`, `RunState::AwaitingApproval` is `"awaiting_approval"`, `BlockReason::MisSized` is `"mis_sized"`; `TaskState::is_finished` is true exactly for `Merged` and `Cancelled`; `RunState::is_terminal` exactly for `Accepted`, `Discarded`, `Failed`.
- `every_run_request_and_reply_round_trips`: each `RunRequest` and `RunReply` variant, wrapped in `ClientMsg::Run` / `DaemonMsg::Run`, with a `RunsSnapshot` holding one `RunInfo` with one `TaskInfo` holding one `AgentRoundInfo` and one `ReviewInfo` with a `Finding`, and a `ToolCall` with `args` `{"summary":"ok"}`, survives MessagePack (`rmp_serde::to_vec_named`).
- `window_info_run_defaults_to_none`: JSON without `run` deserializes with `run == None`; `window_info_kind_defaults_to_pty`: JSON without `kind` gives `WindowKind::Pty`, and `Headless` round-trips as `"headless"`.
- `hook_source_stream_round_trips` (`HookSource::Stream` survives MessagePack).
- `agent_role_serializes_snake_case`: `Orchestrator`, `Worker` and `Reviewer` are `"orchestrator"`, `"worker"` and `"reviewer"`, the strings `anthrex mcp --role` and `fake-agent` use.
- `agent_role_does_not_shadow_the_conversation_role`: `use proto::{AgentRole, Role};` both resolve at the crate root, and the test asserts something, not only that it compiles: `TypeId::of::<AgentRole>() != TypeId::of::<Role>()`, and `AgentRole::Worker` serializes as `"worker"` while `Role::User` keeps its M6.5 wire form (it is the guard for decision 3).
- `token_usage_billable_excludes_cache_reads`: `TokenUsage { input: 700, output: 600, cache_read: 5000, cache_write: 300 }.billable() == 1600`. Four distinct values, and a `cache_read` large enough that counting it, or swapping it with `cache_write`, changes the result.
- Rename `proto_version_is_six` (`crates/proto/src/lib.rs:48`) to `proto_version_is_seven` and assert the header's number.

**Change.** Add the Interfaces types, `TokenUsage::billable` included. Until M8a.22, the daemon answers every `RunRequest` with `RunReply::Refused { request: "run", message: "runs are not available yet" }` (and `Tool` with `ToolResult { ok: false, text: "runs are not available yet" }`); the TUI ignores `DaemonMsg::Run`.

**Acceptance.** Workspace builds; all tests pass; `PROTO_VERSION` equals the header's derivation, recorded in "Implementation notes".

**Commit.** `feat(proto): add run, task, plan-edit and snapshot types and the run wire messages`

### M8a.3 The `[orchestrator]` config and the roster

**Files.** Create `crates/config/src/orchestrator.rs`, `crates/config/src/orchestrator_tests.rs`. Modify `crates/config/src/lib.rs` (field, call, unknown-key arm only).

**Tests first**, in `orchestrator_tests.rs`:

- `defaults_when_absent`: every default of the Interfaces block; `models == default_roster()` (four entries in decision 23's order with their strengths).
- `keys_are_read`: a config setting every key, including `[orchestrator.budget.s]`, `[[orchestrator.models]]` and `[orchestrator.profile.env]`, reads back exactly.
- `out_of_range_values_fall_back_with_problems`: `max_writers = 0`, `max_writers = 9`, `stall_after_secs = 1`, `git_timeout_secs = 4`, `worker_codex_sandbox = "yolo"`, `default_runtime = "shell"`: one problem each, default kept, the exact message for `max_writers`.
- `invalid_model_entries_are_skipped_with_their_index`: runtime `perl`, an empty Claude model, strength `huge`, an 81-character note.
- `user_models_extend_and_replace_builtins` (a `claude-sonnet-5` entry with strength `frontier` replaces the built-in in place); `builtin_models_false_drops_builtins`; `no_valid_models_uses_the_default_roster`.
- `unknown_orchestrator_keys_are_reported` (`orchestrator.max_parallel`, `orchestrator.budget.x.tool_calls` and `orchestrator.done_quiet_secs`, a key an earlier draft had, each `unknown key, ignored`).
- `review_small_is_read_as_on_or_off`: `review.small = "off"` under `[orchestrator]` and `[orchestrator.review] small = "off"` both give `review_small == false`; `"on"` gives `true`; `small = false` (a boolean) and `small = "no"` are each one problem, `orchestrator.review.small: must be "on" or "off" (using "on")`, and keep `true`; `[orchestrator.review] large = 1` is `unknown key, ignored`.
- `claude_auth_and_helper_are_read` (`auth = "api_key"` with `api_key_helper`; `auth = "token"` is a problem and keeps `login`); `worker_tools_and_sandbox` (`worker_allowed_tools` replaces the default list; an empty list is a problem and keeps the default; `worker_codex_sandbox = "yolo"` is a problem); `budget_tokens_is_optional` (absent by default, read when set, `0` is a problem); `worker_sandbox_defaults_to_true` (and `false` is read); `profile_generated_is_read` (`[orchestrator.profile] generated`); `profile_protected_is_read_as_additions` (config `protected = ["x/**"]` reads back as exactly `["x/**"]`: the config crate cannot see the daemon's `BUILTIN_PROTECTED`, so the five built-ins are added by `resolve_profile` (M8a.5, `run/plan.rs` or `run/validate.rs`) and tested there, M8a.5's `plan_protected_adds_to_config_and_builtins`).
- In `lib_tests.rs`: `every_key_is_read` (`lib_tests.rs:154`) feeds `[orchestrator] max_parallel = 3` today (`:180`), which only passes because `orchestrator` is skipped. Replace those two lines with a valid key (`[orchestrator]` `max_writers = 4`) and assert `config.orchestrator.max_writers == 4`; `max_parallel` becomes one of `unknown_orchestrator_keys_are_reported`'s cases. No other test in `lib_tests.rs` changes (the file is already over 600 lines).

**Change.** Implement per the Interfaces and decision 23.

**Acceptance.** Tests pass. `crates/config/src/lib.rs` (600 lines at `6f22681`) grows by at most 8 lines.

**Commit.** `feat(config): add the [orchestrator] table with budgets, timings, roster strengths, headless session settings and profile defaults`

### M8a.4 Globs and roster policy

**Files.** Create `crates/daemon/src/run/mod.rs`, `run/globs.rs`, `run/roster.rs`, and `run/model.rs` holding only `ReviewLevel` (which `roster::pick_reviewer` takes; M8a.5 adds the rest of the model). Modify `crates/daemon/src/lib.rs` (`pub mod run;`, which coexists with the existing `pub use lifecycle::{…, run}` re-export at `lib.rs:43`: a module and a function live in different namespaces, and `daemon::run(opts)` in `crates/cli/src/main.rs:379` keeps working), `crates/daemon/Cargo.toml` (`globset = "0.4"`, `regex = "1"`, `toml` as workspace dependencies; add `globset` and `regex` to the workspace table).

**Tests first.**

- In `globs.rs`: `literal_prefix_stops_at_the_first_wildcard` (`crates/proto/**` → `["crates","proto"]`; `**/*.rs` → `[]`; `src/a[0-9].rs` → `["src"]`); `intersection_is_prefix_containment` (the refreshed M8 brief's four cases, restated: `crates/proto/**` and `crates/proto/src/*.rs` intersect; `crates/proto/**` and `crates/tui/**` do not; `**/*.rs` intersects everything; two empty lists do not); `modules_spanned_cases` (with `modules = ["crates/*"]`: `crates/proto/**` → `One("crates/proto")`; `crates/proto/src/a.rs` and `crates/tui/**` → `Many`; `crates/**` → `Many`; `docs/x.md` → `One(".")`; no modules configured → `One(".")`); `owns_without_wildcard_covers_the_directory_below_it` (`crates/auth` matches `crates/auth/src/lib.rs` and not `crates/authz/lib.rs`; `crates/auth/` the same); `glob_matching_does_not_cross_separators` (`src/*.rs` does not match `src/a/b.rs`); `absolute_and_parent_globs_are_invalid`; `inside_area_cases` (`crates/daemon/src/**` inside `crates/daemon/**`; `crates/tui/**` not; `crates/daemon` exact inside `crates/daemon/**`); `names_literally_cases` (`AGENTS.md` in owns names `AGENTS.md`; `./AGENTS.md` too; `**`, `*.md`, `.claude/**` and the directory entry `.claude` name nothing; `docs/AGENTS.md` does not name `AGENTS.md`); `builtin_protected_matches_nested_instruction_files` (`**/AGENTS.md` matches `AGENTS.md` and `docs/AGENTS.md`; `.claude/**` matches `.claude/settings.json`; `.mcp.json` does not match `x/.mcp.json`).
- In `roster.rs`, on the default roster: `pick_reviewer_prefers_the_other_runtime_at_or_above_the_author` (a Codex `standard` author at level `medium` gets `claude-sonnet-5`, effort `medium`); `small_level_takes_the_cheapest_other_runtime` (a Claude author at `small` gets Codex `""`, effort `low` — Codex has no `fast` entry, so the lowest at or above `fast`); `frontier_level_falls_back_to_the_same_runtime` (a Codex author at `frontier` gets `claude-opus-5`; a Claude `claude-opus-5` author at `frontier` with no Codex frontier entry gets `claude-opus-5` itself as the last resort); `escalate_raises_effort_then_changes_runtime` (`medium` → `high` same model; `high` Claude standard → Codex `""` high; `high` Claude `claude-haiku-4-5` on a Claude-only roster → `claude-sonnet-5` `high` (no peer, one strength up); `high` Claude `claude-opus-5` on the default roster → unchanged (no Codex frontier entry, nothing above frontier)).

**Change.** Implement decisions 11, 23 (policy side), 35 (reviewer route) and 39.

**Acceptance.** Tests pass; both files are pure (decision 2's grep).

**Commit.** `feat(daemon): add owns glob rules and strength-aware roster policy for runs`

### M8a.5 Plan parsing, resolution and validation

**Files.** Create `run/plan.rs`, `run/validate.rs`, `run/env.rs`, `run/plan_tests.rs`, `run/validate_tests.rs`. Extend `run/model.rs` (M8a.4 created it with `ReviewLevel` only).

**Tests first.** Each rule test asserts the exact message and rule id.

- `the_brief_example_builds_a_run`: decision 7's plan through `build_run` gives one task with branch `anthrex/<id>/t1`, worktree `/tmp/wt/runs/<id>/t1`, `route == Route { runtime: Claude, model: "claude-sonnet-5", strength: Standard, effort: High }` (the plan's `high`, not policy's `medium`), `budget == Budget { tool_calls: 120, minutes: 45, tokens: Some(3000000) }` (not the M default 150/60), `review_level == Some(Medium)`, `review_route` from `pick_reviewer`, and `limits` from config except `max_writers == 4`, `max_readers == 2` and `max_bounces == 3`, the plan's values, each different from its config default (3, 3, 2) and from the other two.
- `policy_fills_routes_by_class` (S → `standard`/`low`, M → `standard`/`medium`, hub → `frontier`/`high`, model the first roster entry at that strength) and `a_given_model_fixes_the_strength` and `a_contradicting_strength_is_an_error`.
- `profile_keys_resolve_per_key`: plan `check` with config `single_test` gives both; plan `generated` overrides config `generated`; `check_timeout_secs` defaults to 1800; `profile_env` substitutes `{worktree}`. `generated_globs_are_validated` (absolute or `..` globs rejected like `owns`). `plan_protected_adds_to_config_and_builtins`. `owns_covering_a_protected_file_warns_without_rejecting`: with `Preflight.protected_files == ["AGENTS.md", ".claude/settings.json"]`, a task owning `**` gets two notes with the exact rule-6.protected text and the run still builds; a task owning `AGENTS.md` exactly gets no note for it; a task owning `.claude/**` gets a note for `.claude/settings.json`.
- Size rules: `two_modules_raise_s_to_m`, `two_modules_with_interface_change_is_l_and_rejected`, `hub_touch_raises_to_m_and_sets_hub`, `l_is_rejected`, and `raises_are_recorded_as_notes` (exact note text).
- Test-mode rules: `code_defaults_to_tdd_and_docs_to_none`, `non_tdd_needs_a_reason`, `none_on_source_is_rejected`, `hub_code_is_forced_to_tdd`, `tdd_without_single_test_becomes_check_and_raises_review`.
- Review level: `no_check_raises_every_review`, `check_mode_on_source_raises_review`, `review_small_off_skips_s_but_not_hub`.
- `cross_runtime_overlap_is_rejected_on_the_later_task`, `same_runtime_overlap_is_allowed_and_becomes_an_implicit_dep` (`implicit_deps == ["t1"]` on `t2` only).
- Graph and fields: `duplicate_ids`, `id_syntax`, `reserved_id_integration`, `unknown_dependency`, `cycle_is_reported_once`, `blank_fields`, `owns_required`, `absolute_owns`, `research_and_review_kinds_are_deferred`, `too_many_tasks`, `limit_ranges`, `budget_ranges`, `single_test_must_contain_the_placeholder`, `test_passed_must_compile_as_a_regex`, `env_key_syntax`.
- `all_errors_are_collected`: three problems, three `PlanError`s.
- `slug_examples` (the refreshed M8 brief's four cases, restated: `slug("Add password reset", 0x3f9a) == "add-password-reset-3f9a"`; `slug("  Fix: the ÄPI!! ", 1) == "fix-the-pi-0001"`; `slug("!!!", 2) == "run-0002"`; a 60-character goal gives a slug whose part before the suffix is at most 32 characters and does not end with `-`).

**Change.** Implement decisions 7–12 and 15's slug; `Run` and `Task` types per Interfaces.

**Acceptance.** Tests pass; the files are pure.

**Commit.** `feat(daemon): parse, resolve and validate run plans against the size, test-mode and runtime rules`

### M8a.6 Plan edits

**Files.** Create `run/edits.rs`, `run/edits_tests.rs`.

**Tests first.**

- `add_task_is_validated_like_a_plan_task`; `amend_route_on_a_working_task_is_refused` (message `task t1 is working; route can be amended only on pending, queued or blocked tasks`); `amend_brief_on_a_working_task_yields_a_deliver_consequence` (text equals `amend_message`).
- `cancel_of_a_pending_task_marks_dependents_dep_cancelled`; `cancel_of_a_working_task_yields_cancel_live`.
- `split_rewires_dependents_to_every_child`: `t3` depended on `t2`; after `split_task t2 into [t2a, t2b]`, `t3.spec.deps == ["t2a","t2b"]` and `t2` is `cancelled`.
- `add_dep_creating_a_cycle_is_rejected`; `dep_on_a_cancelled_task_is_rejected`.
- `answer_only_on_blocked_question_or_working`.
- `a_batch_is_atomic`: a batch of a valid `add_task` and an invalid `add_dep` changes nothing and returns one error.
- `edit_leaving_an_l_task_is_rejected`: `amend_task size = "L"`.
- `an_untouched_rung3_l_task_does_not_block_other_edits`: a task at L from rung 3 and an unrelated `answer` → accepted.
- `edit_outside_its_area_is_rejected`: under `EditScope::Area { globs: ["crates/daemon/**"] }`, `add_task` owning `crates/tui/**` gives decision 12's message.
- `pause_resume_finish_are_consequences`.

**Change.** Implement decision 13.

**Acceptance.** Tests pass; `edits.rs` is pure.

**Commit.** `feat(daemon): apply and validate plan edits atomically`

### M8a.7 Headless streams: parsers, status and the conversation mapping

**Files.** Create `crates/daemon/src/headless/mod.rs` (types), `headless/claude_stream.rs`, `headless/codex_stream.rs`, `headless/status.rs`, `headless/conversation.rs`, `headless/argv.rs` (`CLI_CAPS`, `mcp_args` and the two argv builders, decisions 24–25), and a test file beside each. Modify `crates/daemon/src/lib.rs` (`pub mod headless;`), `crates/proto/src/messages.rs` only if M8a.2 did not add `HookSource::Stream`, `crates/daemon/Cargo.toml` (dev-dependency `proptest`, added to the workspace table; nothing in the workspace uses it yet), and move `TOOL_RESULT_SUMMARY_MAX`, `bound_tool_response` and `bound_tool_response_value` from `crates/cli/src/hook.rs` into `crates/proto/src/conversation.rs` unchanged, with their tests (decision 27's mapping-table note).

**Tests first.** Every parser test reads M8a.1's fixtures from `crates/daemon/tests/fixtures/headless/` through `include_str!`, so a new fixture version is a new test, not an edit.

- `claude_stream_parses_every_fixture_line`: every line of `claude-…-stream.jsonl` yields at least one event, and none yields `Unknown` except lines whose type the meta file lists as unmodelled. The event sequence of the three recorded turns is `Init`, then per turn tool and text events, then `TurnEnded`. Turn 2 has a `PermissionDenied` and `denials == ["Write"]`. Turn 3 is `Interrupted`.
- `claude_usage_is_per_turn`: two consecutive `TurnEnded` usages are per turn even if the fixture's `result.usage` is cumulative. The parser keeps the previous total in the event when M8a.1 found it cumulative; otherwise it passes the value through.
- `claude_failed_results_are_classified`: each failing `result` in `claude-…-documented.jsonl` yields `Failed` with `RateLimit`, `Authentication`, `Billing`, or `Other` for an unknown category. `api_retry` yields `ApiRetry` with `delay_ms`.
- `claude_user_message_matches_the_recorded_envelope`: `user_message("hello", Some(id))` is byte-for-byte the first line of `claude-…-input.jsonl` with its text replaced, compared as JSON values. The same holds for `interrupt_request` when M8a.1 found the control request accepted.
- `codex_stream_parses_exec_and_resume`: `thread.started` gives `Init` with the thread id. `command_execution` gives a `ToolUse { name: "Bash" }` and `ToolResult` pair joined by id. `turn.completed` gives `TurnEnded { Completed, usage }` with the fixture's numbers. The `-m no-such-model` run's `turn.failed` in `codex-…-exec.jsonl` gives `Failed { Other }`.
- `a_tool_result_is_cut_on_a_char_boundary`: a Claude `tool_result` whose content is 3000 × `世` (9000 bytes) yields `ToolResult.text` of exactly 4095 bytes (1365 characters), because 4096 is not a multiple of 3 and the cut must back off to a character boundary; with 2-byte characters the back-off would never run (4096 is even), which is the fixture defect M6.5.2 recorded. An ASCII text of 5000 bytes is cut to exactly 4096. An `Unknown { line }` built from a 1000 × `世` line holds exactly its first 300 characters.
- `garbage_never_panics`: invalid JSON, a JSON array, a 5 MB line, an unknown `type`, and a `result` with no `usage` each yield `Unknown` or a best-effort event, never a panic. Also run as a `proptest` over arbitrary strings with 1000 cases.
- `headless_status_transitions`: `Starting` → `Working` on `Init`/`TurnStarted`. `ApiRetry` → `Attention` with `rate_limited`, and any later event clears it. `TurnEnded Completed` → `Idle`, and `Failed` → `Attention`. `ProcessExited` → `Exited`. `tool` follows the latest top-level `ToolUse`.
- `conversation_map_follows_the_table`: one case per row of the Interfaces mapping table, for both `hooks_fire` values. With `hooks_fire == true`, no hook is produced.
- `a_codex_session_builds_a_real_conversation`: feed `codex-…-exec.jsonl` and the daemon's sent turn through `map` and `sent_turn` into a real M6.5 `ConversationSet` (`on_hook` then `enrich`). The snapshot has one `User` turn with the prompt text and one `Assistant` turn whose `ToolCall` for the command has `input` and a result. Without the synthesised hooks, the same records produce no turn, which is M6.5 decision 1.
- `a_claude_session_enriches_hook_built_turns`: replay `claude-…-hooks.jsonl` through `on_hook` (as the real hook path would) and `claude-…-stream.jsonl` through `map`. The prose and tool details land, and no turn is duplicated.
- `argv_builders`: the exact argv for a Claude worker, a Claude reviewer, a Claude resume, a Codex worker's first turn, a Codex resume and a Codex reviewer, each with the flags in decision 24's or 25's order (the `--mcp-config` JSON compared as a value). Also `claude_variadic_flags_are_followed_by_a_flag` over every combination of role, model, effort flag and session argument. `api_key_auth_adds_bare` and `login_adds_the_non_bare_flag_only_when_caps_say_so`. `codex_worker_args_add_the_git_common_dir_as_writable` (review focus 5). `claude_launches_load_only_user_settings`: with `claude_user_settings_only == Some(flags)`, every Claude argv (worker, reviewer, first launch, resume) contains the flags exactly once, before `--settings`; with `None` it contains none of them. `codex_launches_exclude_project_config_when_caps_say_so`: with `codex_user_config_only == Some(flags)`, the first-turn and resume argv both carry the flags exactly once, before `--`; with `None` neither does. `worker_settings_json_enables_the_sandbox`: `claude_settings` for a worker is exactly the M3 hook JSON plus the sandbox block with `CLI_CAPS.claude_sandbox_keys`, unsandboxed commands disallowed and `writable_roots == [<git common dir>]` (compared as JSON values); a reviewer's has no sandbox key; `worker_sandbox = false` gives none either. `toml_string_round_trips_through_the_toml_crate` covers the refreshed M8 brief's 20 strings (restated: every control character, U+007F, quotes, backslashes, and a multi-line text; parse `x = <launch::codex::toml_string(s)>` with the `toml` crate and get the original back). The two contracts do not exist until M8a.11, which adds them to this test (`contracts_round_trip_through_toml_string`).
- `synthesised_tool_responses_are_bounded_like_the_hooks`: a Codex `command_execution` result of 3000 × `世` mapped with `hooks_fire == false` gives a `PostToolUse` whose `tool_response`, `tool_result_truncated` and `tool_result_stringified` equal what the moved bounding makes of the same `{"output": text}` value (the same input through both paths, so nothing depends on where the cut lands); `anthrex hook`'s own tests (`crates/cli/tests/hook_command.rs`) still pass after the move.
- `mcp_args_for_a_worker`: the exact vector `["mcp","--role","worker","--run","r-3f9a","--task","t1","--window","7","--socket","/tmp/a.sock"]`.

**Change.** Implement decisions 24 and 25 (argv), 27 (parsers, status, mapping), 51 (the fixture contract), 53 (the user-settings flags) and 54 (the sandbox settings block), and the Interfaces tables.

**Acceptance.** Tests pass, and every file in `headless/` except `session.rs` passes decision 2's purity grep.

**Commit.** `feat(daemon): parse Claude stream-json and Codex exec events into session events, status and conversation inputs`
### M8a.8 Run git operations I: preflight, branches, worktrees, the write queue

**Files.** Create `run/git/mod.rs`, `run/git/worktrees.rs`, `run/git/queue.rs`, `crates/daemon/tests/run_git.rs`, and `crates/daemon/tests/run_git_env.rs` (one test, alone in its binary; see "Shared test helpers").

**Tests first**, each with a fresh `TempRepo`:

- `preflight_reports_branch_sha_and_roots` (and `git_common_dir == <project>/.git`); `preflight_from_a_linked_worktree_keeps_root_and_project_apart` (`root` is the linked checkout, `project` and `git_common_dir` are the main repository's, and `git_common_dir != <root>/.git`); `preflight_refusals` (dirty tracked tree, detached `HEAD`, no commits, not a repository, missing identity: no repo identity, and the injected `git` is a wrapper script that exports `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1` before exec'ing git, never a change to the test process's own environment) with decision 17's exact messages; `preflight_ignores_untracked_files`.
- `run_branch_and_integration_worktree_are_created_and_locked`: `git worktree list --porcelain` shows the path on `anthrex/<id>/integration` with a `locked` line.
- `task_branch_starts_at_the_given_run_head_and_is_reused`: create from a sha, then a second call returns the same head and changes nothing; after `git worktree remove --force` it is re-added on the existing branch; called again with a newer `from` while the branch has no commit of its own it re-points the branch to `from`; with a commit of its own it leaves the branch alone.
- `a_task_branch_can_live_under_the_run_branch_name`: `anthrex/<id>/integration` and `anthrex/<id>/t1` coexist, and `git branch anthrex/<id>` fails (decision 16).
- `verify_done_reports_each_condition`: no commits; one commit; a dirty tracked file; `MERGE_HEAD` present; an untracked file inside `owns` and one outside (only the inside one reported); a committed file outside `owns`; a committed `Cargo.lock` outside `owns` with `generated = ["Cargo.lock"]` (reported in `generated_outside_owns`, not `outside_owns`), and the same with `Cargo.lock` in `owns` (reported in neither); a changed `AGENTS.md` with owns `**` (in `protected_changed`), with owns `AGENTS.md` (in nothing), a changed `.claude/settings.json` with owns `.claude/**` (in `protected_changed`), and a new `docs/AGENTS.md` (in `protected_changed`, and not also in `outside_owns`); after merging a newer run head into the task branch, the run head's own files are **not** reported as outside `owns`; a `red` that is the start commit, a `red` on another branch, a valid `red` (`red_ok` `Some(false)`, `Some(false)`, `Some(true)`).
- `count_commits_and_diff_so_far`.
- `project_settings_are_found_in_the_base_tree`: a `TempRepo` whose base commit tracks `.claude/settings.json` with a `hooks` key → listed; one whose settings have no `hooks` key → not listed; one tracking `.mcp.json` → listed; an untracked `.mcp.json` in `root` only → not listed; invalid JSON in a tracked settings file → listed (it cannot be shown to be hook-free); with `codex_paths = Some([".codex/config.toml", ".codex/hooks.json"])` a tracked `.codex/config.toml` → listed, and with `None` → not listed; with `claude == false`, Claude's files are not listed.
- `protected_files_lists_tracked_matches_only`: tracked `AGENTS.md`, `docs/AGENTS.md` and `.claude/settings.json` are listed; an untracked `CLAUDE.md` is not.
- `review_worktree_is_detached_at_the_task_head_and_replaced`, which also checks the returned patch: it equals `git diff <base>..<head>` for a small change, and for a change whose diff exceeds `REVIEW_DIFF_MAX` (a committed file of 6 000 lines `世世世世`, 13 bytes each) it is valid UTF-8 within 3 bytes of the limit and never above it.
- `every_run_git_call_passes_no_optional_locks_and_no_git_env`, in `run_git_env.rs` alone: every function in this task through `recording_git`, with `GIT_DIR` and `GIT_INDEX_FILE` set in the test process's environment (edition 2024 makes `set_var` unsafe, and only a binary with no other test can have no concurrent spawn; `worktree_env.rs` is the precedent): every logged argv has `-C <dir> --no-optional-locks`, none of the five variables reached the child, and every write carries `-c core.hooksPath=/dev/null -c commit.gpgSign=false`.
- `engine_writes_ignore_hooks_and_signing`: a repo with `commit.gpgsign = true`, `gpg.program = /bin/false` and a `post-checkout` hook that exits 1 (`TempRepo::failing_post_checkout`, `tests/support/mod.rs:310`): worktree creation and checkout still succeed.
- In `queue.rs` (tokio test): `writes_to_one_repo_are_serialized` (two writes that each hold for 200 ms never overlap, checked by timestamps recorded inside); `writes_to_two_repos_run_concurrently` (proved by a rendezvous, not by timestamps: each write's closure signals the other and waits for the other's signal with `recv_timeout(5 s)`, so a serialized queue fails with a timeout instead of passing or failing on scheduling); `lock_errors_are_retried_then_surface` (a closure that returns `Unable to create '/x/.git/index.lock': File exists` twice then succeeds is called three times; one that always fails is called six times and returns the last error).

**Change.** Implement decisions 17, 18, 19, 53 (`project_settings`), 55 and 56 (the split of `VerifyDone`'s changed paths, and `protected_files`).

**Acceptance.** Tests pass. No function in `run/git/` is `async` except `GitQueue::write`. Milestone 5's rule still holds: every `Command::new` that spawns git in `crates/daemon/src` also matches `no-optional-locks`.

**Commit.** `feat(daemon): add run preflight, branch and worktree operations behind a per-repository git write queue`

### M8a.9 Run git operations II: merge candidate, CAS, hand-back, salvage, finish

**Files.** Create `run/git/merge.rs`, `run/git/salvage.rs`; add to `crates/daemon/tests/run_git.rs` (split into `run_git_merge.rs` if it passes 600 lines).

**Tests first.**

- `merge_tree_returns_a_tree_or_the_conflicted_files`.
- `commit_tree_and_cas_advance_the_run_branch`: the candidate's parents are `(run_head, task_head)`; `cas_update` with the right `old` returns `true` and moves the branch; with a stale `old` returns `false` and moves nothing.
- `materialize_and_reattach_leave_the_integration_worktree_on_its_branch`: after `materialize(candidate)` `HEAD` is detached at the candidate; after `reattach` `symbolic-ref HEAD` is the run branch.
- `hand_back_leaves_markers_and_merge_head` (conflict: files returned, `MERGE_HEAD` exists, the file contains `<<<<<<<`); `hand_back_that_is_clean_commits_the_merge` (empty list, `HEAD` is a merge commit).
- `salvage_of_a_clean_worktree_writes_nothing`; `salvage_captures_tracked_and_untracked_changes_but_not_ignored_files` (the salvage commit's tree contains the modified file and a new file, not an ignored `target/x`); `remove_refuses_nothing_after_salvage` (a locked, dirty worktree is salvaged, unlocked and removed).
- `accept_requires_the_base_branch_and_a_clean_tree`; `accept_merges_no_ff`; `accept_onto_an_advanced_base_merges_no_ff` (a commit on `main` touching another file; the merge's parents are `(the advanced head, run head)`); `accept_conflict_with_an_advanced_base_aborts_and_leaves_base_untouched` (a commit on `main` changing the same line as the run: `Conflict { files }` names it, `refs/heads/main` and `git status --porcelain` in `root` are exactly as before, no `MERGE_HEAD`); `accept_refuses_when_the_base_moved_again` (`expected_base` stale → error, nothing merged); `delete_branches_removes_every_run_branch_but_keeps_salvage_refs`.
- `read_ref_reports_a_moved_branch`.
- `guard_refs_classifies_each_case`, against real `TempRepo` states in which the run branch has first advanced by one merge, so `run_head != base_sha` (with the two equal, a guard that reads the run ref where it means the base ref, or the reverse, would pass every case): both refs at their recorded values → `Ok`; a commit on `main` → `BaseAdvanced { to, commits: 1 }`; `main` reset to a sibling commit (`git reset --hard <base_sha>~` plus a new commit) → `Halt` with `was rewritten`; `main` deleted → `Halt` with `was deleted`; the run branch moved → `Halt` naming it, checked first even when the base also advanced.
- `commits_since_caps_and_counts`: 55 commits → 50 lines newest first, each `<sha7> <author>: <subject>`, total 55.

**Change.** Implement decisions 20, 21 (the reads) and 36 (the git steps).

**Acceptance.** Tests pass.

**Commit.** `feat(daemon): add merge-tree candidates, compare-and-swap updates, conflict hand-back and salvage refs`

### M8a.10 Shell execution: setup, check and the test proof

**Files.** Create `run/exec.rs`, `run/proof.rs`, `crates/daemon/tests/run_exec.rs`, and `crates/daemon/tests/run_exec_env.rs` (one test, alone in its binary).

**Tests first.**

- `check_captures_the_last_200_lines_and_exit_code` (`seq 1 500; exit 3`: `ok == false`, `code == Some(3)`, tail of 200 lines starting at `301`); `summary_is_the_last_40_lines`; `check_merges_stderr`.
- `check_timeout_kills_the_process_group`: `sleep 30 & echo $! > <tmp>/bg; wait` with a 1 s timeout returns `timed_out` within 3 s, and the background pid is gone within `KILL_GRACE` + 1 s (`kill(pid, 0)` gives `ESRCH`).
- `check_tail_survives_invalid_utf8_and_huge_lines` (a 1 MB ASCII line, a line of 1000 × `世`, and bytes `0xff 0xfe`: the tail is valid UTF-8, and each long line is **exactly** 300 characters, 300 bytes and 900 bytes respectively; an "at most 300" assertion would pass a cut that loses a whole character).
- `engine_commands_get_the_profile_env_and_lose_agent_variables`, in `run_exec_env.rs` alone: with `CLAUDE_CODE_CHILD_SESSION=1`, `CLAUDECODE=1`, `ANTHREX_WINDOW_ID=9` set in the test process, a command printing `env` shows none of them and shows `CARGO_TARGET_DIR=<dir>/target` from the profile.
- `run_proof` in a `TempRepo` with `single_test = "sh tests/{test}.sh"` and `test_passed = "PASS {test}"`: `proof_accepts_a_real_red_then_green` (red commit adds `tests/t_reset.sh` which greps `impl.txt` for `reset` and echoes `PASS t_reset`; head adds `impl.txt`); `proof_rejects_a_red_that_passes`; `proof_rejects_a_head_that_fails`; `proof_rejects_output_that_does_not_show_the_test` (the head script exits 0 and prints nothing); `proof_quotes_a_hostile_test_name` (test `a; touch pwned` leaves no `pwned` file anywhere in the proof worktree); `proof_runs_setup_once_per_new_worktree`.

**Change.** Implement decisions 26 (engine commands), 33 and 34.

**Acceptance.** Tests pass. `grep -n "thread::sleep" crates/daemon/src/run` prints nothing outside `exec.rs`'s poll loop.

**Commit.** `feat(daemon): run setup, check and the fail-to-pass test proof with bounded, scrubbed shells`

### M8a.11 Engine I: start, plan gate, scheduler and dispatch

**Files.** Create `run/engine/mod.rs`, `engine/requests.rs`, `engine/dispatch.rs`, `engine/outbox.rs`, `run/snapshot.rs`, `run/role_launch.rs` (`worker_spec`, `reviewer_spec`, `session_uuid`), `run/contract.rs` (contracts and prompts), `run/messages.rs` (decision 29's clamp and join; no earlier task creates it, and M8a.12's delivery tests need it), `engine/tests/fixture.rs`, `engine/tests/dispatch.rs`.

**Tests first**, in `engine/tests/dispatch.rs`:

- `start_creates_the_run_branch_before_approval`: `Start` without `yes` replies `Ok(<id>)`, state `awaiting_approval`, one `Op CreateRunBranch` with decision 16's branch and path, then after its `OpDone` a `WatchWorktree` for the integration path and `PrepareWorktree` for root tasks up to `max_writers` (the pre-warm), and no `CreateWindow`.
- `approve_starts_dispatch_and_yes_skips_the_gate`: after `Approve`, or with `yes`, a pre-warmed task gets `Op CreateWindow` with:
  - name `<h4>/t1.w1`, and a `HeadlessSpec` whose `cwd` is the task worktree;
  - `mcp == Some(McpTarget { role: Worker, task_id: Some("t1"), .. })`;
  - `allowed_tools` of `mcp__anthrex__task_done`, `mcp__anthrex__task_blocked`, then `worker_allowed_tools`;
  - `instructions == WORKER_CONTRACT` and `first_turn == worker_prompt`;
  - `session_uuid == Some(session_uuid(<id>, <op>))` for Claude and `None` for Codex;
  - for Codex, `codex_writable_roots == [<git_common_dir>]`, the fixture's `/tmp/p/.git`, which is neither `<root>/.git` nor `<project>` (decision 16).

  `approved_by` is `user` or `--yes`, and the round's `turn_open` is true from dispatch.
- `session_uuids_are_valid_and_distinct`: `session_uuid` output parses as a UUID with version nibble 4, and a thousand op ids give a thousand distinct values.
- `reject_discards`: `Reject` gives `Op Discard` with every worktree and prefix `anthrex/<id>/`.
- `awaiting_approval_survives_restore`: `Restore` of an `awaiting_approval` run leaves it `awaiting_approval`.
- `dependents_wait_for_merged_and_branch_from_the_run_head`: `t2` depends on `t1`; no `PrepareWorktree` for `t2` until `t1` is `merged`, then one with `from == run_head`.
- `implicit_owns_deps_serialize_by_plan_order`; `cancelled_implicit_dep_releases_the_later_task`; `a_started_task_is_always_the_one_waited_for` (an `add_task` or `split_task` placing an overlapping task earlier in plan order than a `working` task makes the new task wait, not the working one).
- `a_stale_prewarmed_branch_is_repointed_at_dispatch`: `t2` pre-warmed from `base_sha`, `t1` merges first, `t2`'s dispatch issues `PrepareWorktree` with `from == run_head` and the git layer re-points the untouched branch.
- `critical_path_orders_dispatch`: tasks `a` (S, no dependents), `b` (M) with dependent `c` (M), `max_writers` 1: `b` dispatches first; ties break by `priority`, then plan order.
- `writer_and_reader_slots_are_separate`: `max_writers` 1, `max_readers` 1: a task in `review` does not stop another from dispatching; a second reviewer waits for the reader slot.
- `hub_runs_alone`: a hub task waits while any writer slot is held and blocks every dispatch while it holds one.
- `window_limit_blocks_the_task_as_environment`.
- `snapshot_marks_critical_path_and_wave` and `revision_bumps_on_every_change_and_only_then` (every event that changes a run raises its `revision` and the global one; `Tick` with nothing due changes neither).
- In `run/messages.rs`: `clamp_keeps_head_and_tail_on_char_boundaries` (a 40 002-byte text of `世` is clamped to valid UTF-8 whose length is within one character (3 bytes) of `MESSAGE_MAX_BYTES` and never above it, with each half cut on a character boundary: a two-sided bound, because "at most `MESSAGE_MAX_BYTES`" alone would pass a clamp that drops half the budget; `MESSAGE_MAX_BYTES` (32 768) is not a multiple of 3, so a 2-byte fixture would never exercise the back-off); `join_turn_orders_and_separates` (three queued messages with distinct texts, joined in queue order by one blank line).
- `contracts_round_trip_through_toml_string`: `WORKER_CONTRACT` and `REVIEWER_CONTRACT` through M8a.7's `toml_string` round-trip.
- `worker_prompt_layout`: starts with `[anthrex] Task t1: `, ends with the brief, has `Test mode: tdd` and `Single-test command:` for tdd, `Check command:` only when the profile has one; `WORKER_CONTRACT` names `task_done` and `task_blocked`.

**Change.** Implement decisions 14, 19 (engine side), 41, 47 (engine side) and the prompts of 30.

**Acceptance.** Tests pass; `run/engine/` is pure.

**Commit.** `feat(daemon): add the run reducer with the plan gate, critical-path scheduler and dispatch`

### M8a.12 Engine II: the done gate, turns, stalls, denials and budgets

**Files.** Create `engine/done.rs`, `engine/ladder.rs` (budget and stall parts), `engine/tests/done.rs`, `engine/tests/turns.rs`. Modify `engine/mod.rs`, `engine/outbox.rs`.

**Tests first**, in `engine/tests/done.rs` and `engine/tests/turns.rs`. The fixture's `fx.signal(window, AgentSignal)` drives every case.

- `task_done_runs_verify_done_and_replies_after_it`: a `Tool` call from the task's worker window gives `Op VerifyDone` and no reply yet. `OpDone DoneChecked` with commits then gives `Reply Ok(<decision 32's accepted text>)`, and the state becomes `proof` (tdd), `check` (check mode with a check), `review` or `merge_queue`.
- `task_done_rejections_leave_the_task_working`: one case per rejection text of decision 32, with no failure counted.
- `task_done_with_files_outside_owns_goes_to_rung_3`: `blocked(mis_sized)`, size S→M, text `changed files outside owns: x.rs`.
- `a_generated_file_outside_owns_bounces_at_rung_1`: with `generated = ["Cargo.lock"]`, `DoneChecked` with `generated_outside_owns == ["Cargo.lock"]` and nothing else outside replies `ToolResult { ok: false }` with `generated_files_message`. The task stays `working` in the same session with `bounces.done == 1`, `failures == 1` and no other message queued; a second such claim is rung 2.
- `a_non_generated_path_outside_owns_still_goes_to_rung_3`: `outside_owns == ["x.rs"]` together with `generated_outside_owns == ["Cargo.lock"]` gives `blocked(mis_sized)`, not a bounce.
- `a_task_that_owns_the_generated_file_passes`: `owns` includes `Cargo.lock`, and nothing is outside, so the task proceeds to its next gate.
- `a_protected_file_changed_through_a_wildcard_bounces_at_rung_1`: owns `**`, `DoneChecked { protected_changed: ["AGENTS.md"] }` gives `ToolResult { ok: false }` with `protected_file_message` naming `AGENTS.md`, `bounces.done == 1`, the task still `working`.
- `a_protected_file_named_exactly_passes`: owns `["src/**", "AGENTS.md"]`, and nothing is caught, so the task proceeds.
- `a_directory_glob_over_claude_settings_still_bounces`: owns `.claude/**` and a changed `.claude/settings.json` is a bounce.
- `a_nested_agents_md_is_protected`: a changed `docs/AGENTS.md` under owns `docs/**` is a bounce.
- `protected_is_checked_before_the_spill_split`: `protected_changed == ["AGENTS.md"]` together with `outside_owns == ["x.rs"]` gives the rung-1 bounce, not rung 3.
- `an_overridden_task_is_exempt_from_protected`.
- `an_unavailable_sandbox_blocks_as_environment`: a `TurnEnded { Failed { SandboxUnavailable } }`, or a process exit before `Init` classified the same way, gives `blocked(environment)` with decision 54's text naming `worker_sandbox = false`, and no resume is attempted.
- `tool_authorization`: every acceptance text of the MCP section (another window, a reviewer calling `task_done`, a paused run, a terminal run, a task not `working`, bad arguments).
- `task_blocked_kinds`: `question` → `blocked(question)`; `mis_sized` → rung 3; `environment` → `blocked(environment)`; missing kind → `question`.
- `deliveries_wait_for_the_turn_to_end`: two messages queued while `turn_open` give no `Deliver`. `TurnEnded` gives exactly one `Deliver` whose text joins both in queue order, and `turn_open` is set again. `Delivered { ok: false }` puts both back at the head and retries after `DELIVERY_RETRY_SECS`. The third failure blocks the task as `blocked(environment)` with `could not deliver to the agent: <error>`.
- `turn_end_fallback_nudges_once_then_proceeds`: `TurnEnded Completed` with no `task_done` gives `Op CountCommits`. With 2 commits, `DONE_NUDGE` is delivered as the next turn. That turn's `TurnEnded` gives `Op VerifyDone` and, on success, `done_signal == TurnEndFallback`. A `task_done` accepted inside the nudge turn ends the fallback instead.
- `turn_end_without_commits_nudges_then_counts_as_a_stall`.
- `open_subagents_defer_the_fallback`: `SubagentStart{a1}`, then `TurnEnded`: no `CountCommits` until `SubagentStop{a1}`.
- `stall_interrupts_then_nudges_then_goes_to_rung_2`:
  - No event for `stall_after_secs` in an open turn gives `Effect::Interrupt`.
  - The interrupted turn's `TurnEnded { Interrupted }` delivers `stall_nudge(10)`.
  - Another `stall_after_secs` of silence gives `KillWindow` and a new session (rung 2, `stalls == 1`).
  - An interrupt that produces no turn end within `INTERRUPT_GRACE` gives `KillWindow` and rung 2 directly.
- `rate_limit_retry_suspends_the_stall_clock`: `ApiRetry { rate_limit, delay_ms: 900_000 }` followed by silence past `stall_after_secs` gives no interrupt until `rate_limited_until + stall_after_secs`. `rate_limits["claude"]` counts one per streak.
- `failed_turns`: a `Failed { RateLimit }` turn gives `rate_limit_continue` after exactly `rate_limit_retry_secs`, with no failure counted, and the round is rate-limited until then. `Authentication` and `Billing` give `blocked(environment)` at once. Two `Other` failures in a row give `blocked(environment)`.
- `a_failed_rate_limit_turn_is_a_rate_limit_event`: a `Failed { RateLimit }` turn with no `ApiRetry` in it gives `rate_limits["claude"] == 1`; an `ApiRetry { rate_limit }` streak that runs straight into a `Failed { RateLimit }` gives 1, not 2; a streak, then a `ToolUse` (the streak ends), then a `Failed { RateLimit }` gives 2.
- `denials_block_at_the_threshold`: with `denials_before_block = 3`, two `PermissionDenied` and a `TurnEnded` carrying one more distinct denial give `blocked(environment)` with `denied_text`, plus `KillWindow`. A denial already seen as an event is not counted again from `permission_denials`.
- `a_process_that_dies_mid_turn_is_resumed_once`: `ProcessExited` during an open turn gives `Op ResumeSession` with `RESUME_AFTER_EXIT`. A second one in the same round is a stall (rung 2).
- `a_claude_process_that_exits_between_turns_is_resumed_on_the_next_delivery`: the round is marked `ended`, and the next queued message gives `Op ResumeSession` carrying it instead of a `Deliver`.
- `a_codex_exit_after_turn_completed_is_normal`: `TurnEnded` then `ProcessExited { code: Some(0) }` changes nothing.
- `a_session_the_engine_killed_is_not_treated_as_an_exit`: `ProcessExited { killed_by_engine: true }` changes nothing.
- `budget_soft_then_hard`:
  - With a budget of 5 tool calls, the 5th `ToolUse` queues `budget_wrap_up` once, and the 6th queues nothing more.
  - 1.5 × 5 is 7.5, so the 7th is **not** a breach and the 8th is: rung 2, a fresh session, the session spend reset and the total kept. (A budget of 4 would make 1.5 × the budget a whole number, and a floor, ceiling or float comparison would all agree on it; 5 tells them apart.)
  - A second breach gives rung 3.
  - Minutes behave the same through `Tick`, with a budget of 5 minutes: nothing at 7 minutes, a breach at 8.
  - With `tokens: Some(1001)` (hard limit 1501.5): a `TurnEnded` with usage `{ input: 700, output: 600, cache_read: 5000, cache_write: 300 }` (1600 billable) is a breach; one with `{ input: 601, output: 600, cache_read: 5000, cache_write: 300 }` (1501 billable, and 6501 if `cache_read` were wrongly counted) is not. With `tokens: None` no usage ever is.
- `usage_is_summed_per_round_and_task`: two turns with usage `{ 11, 23, 37, 41 }` and `{ 101, 203, 307, 409 }` (`input, output, cache_read, cache_write`, eight distinct values, so a swapped or dropped field shows) add up field by field on the round, in the task's `spent_total.tokens` (billable: 11 + 41 + 23 + 101 + 409 + 203 = 788) and in the snapshot.
- `rung_4_blocks_on_the_next_size_ceiling`: an S task whose total reaches the M budget is `blocked(human)`.

**Change.** Implement decisions 27 (engine side), 29 (gate), 32, 38 (the stall, budget and rung 4 parts), 40, 54 (the unavailable-sandbox block), 55 and 56.

**Acceptance.** Tests pass, and purity holds.

**Commit.** `feat(daemon): add explicit completion, turn-based delivery, the turn-end fallback, stall watchdog, denials and budgets to the reducer`
### M8a.13 Engine III: proof, check and review

**Files.** Create `engine/gates.rs`, `engine/tests/gates.rs`. Modify `engine/ladder.rs` (gate failures, rungs 1–3).

**Tests first**, in `engine/tests/gates.rs`:

- `proof_passes_to_check`; `proof_failure_is_rung_1_with_the_message` (each reason of `proof_failed_message`); `proof_accepts_a_red_commit_from_an_earlier_session` (a red committed in session 1 and named in session 2's `task_done` produces `Op Proof` with that red).
- `turn_end_fallback_on_a_tdd_task_fails_the_proof_with_the_missing_names_message`.
- `check_failure_goes_up_the_ladder`: first → rung 1 with `check_failed_message` to the same window; second → rung 2 (`KillWindow`, `Op DiffSoFar`, then `CreateWindow` with `handover_prompt` and `escalate`d route, name `<h4>/t1.w2`); third → rung 3.
- `max_bounces_1_blocks_on_the_second_failure_of_a_gate`.
- `no_check_skips_the_gate_and_marks_the_run_unverified`.
- `review_round_uses_a_fresh_session_and_worktree`: `Op PrepareReview` at `<task>.review`, then a `CreateWindow` with:
  - name `<h4>/t1.r1` and the review route;
  - `mcp == Some(McpTarget { role: Reviewer, .. })` and `allowed_tools == ["mcp__anthrex__submit_review","Read","Glob","Grep","Bash(git diff:*)","Bash(git log:*)","Bash(git show:*)"]`;
  - for Claude, `claude_permission_mode == Some("plan")`; for Codex, `codex_sandbox == "read-only"` and no writable roots;
  - and no `WatchWorktree`.

  Round 2 gets `<h4>/t1.r2` and a prompt listing round 1's critical and important findings.
- `approve_and_minor_only_changes_both_go_to_the_merge_queue`, minor findings kept.
- `a_blocking_finding_is_a_review_failure`: rung 1 message lists only critical and important findings in decision 35's format.
- `finding_validation`: a critical finding with neither `file`+`line` nor `input` → `invalid arguments: findings[0]: …`; `approve` with an important finding → refused.
- `a_second_submit_in_the_same_round_is_refused`; `a_reviewer_turn_without_a_verdict_is_nudged_then_replaced` (`TurnEnded` without `submit_review` delivers `REVIEW_NUDGE`; a second verdict-less turn starts round 2 at the same level with no failure counted; a second verdict-less round in a row blocks as `environment`).
- `review_small_off_skips_s_review`; `override_skips_review_and_marks_the_task`.
- `the_reviewer_prompt_carries_the_clamped_diff`: `OpDone Review { patch }` with a distinct 200-byte patch puts it verbatim under `Diff (`; a 40 001-byte patch of `世` yields a block within 3 bytes of `REVIEW_DIFF_MAX` and never above it, cut on character boundaries, followed by the `[diff clamped: …]` line.
- `the_reviewer_prompt_never_names_the_author`: for every route, the prompt contains neither the author's runtime label nor its model.

**Change.** Implement decisions 33, 34 (engine side), 35 and 38 (gate failures).

**Acceptance.** Tests pass; purity holds.

**Commit.** `feat(daemon): add test proof, check and severity-aware review gates to the reducer`

### M8a.14 Engine IV: merge queue, hand-back, ref guard, completion

**Files.** Create `engine/merge.rs`, `engine/tests/merge.rs`.

**Tests first**, in `engine/tests/merge.rs`:

- `merges_are_one_at_a_time_in_arrival_order`.
- `merged_updates_run_head_cleans_up_and_retires_the_worker`: `OpDone Merged` sets `run_head`, `last_green_candidate`, `merge_commit`, emits `RemoveWorktree` for task, review and proof paths with salvage refs `refs/anthrex/salvage/<id>/t1/<n>`, `UnwatchWorktree` before it, and `RetireWindow` for the worker; dependents become runnable.
- `first_conflict_hands_back_then_requeues_after_task_done`: `Conflict` gives `Op HandBack`; `HandedBack{files}` queues `conflict_message` and returns the task to `working`; its next accepted `task_done` goes straight to `merge_queue` (no proof, check or review op).
- `a_clean_hand_back_requeues_without_the_worker`.
- `second_conflict_blocks_the_task_as_conflict`, and `conflicts` counts are not failures.
- `red_candidate_is_a_merge_failure`: `CandidateRed` gives rung 1 with `candidate_red_message`.
- `ref_moved_halts_the_run`: `RefMoved` (a moved run ref, or a rewritten base) sets `halted` with the reason; nothing dispatches or merges; `Resume` without rebaseline is refused with the reason; with `rebaseline` it records both values, clears `base_moved` and continues.
- `base_advanced_is_recorded_and_the_run_goes_on`: with one task already merged (so `run_head != base_sha`; while they are equal, `from == base_sha` cannot tell the base from the run head), `BaseAdvanced { to: X, commits: 2 }` during the next task's merge sets `base_moved` (`from == base_sha` and `from != run_head`, `to == X`, `seen_at == now`) and the attention line, bumps the revision and persists; the state stays `running`, the pending `MergeCandidate` completes with `Merged`, and the next queued task is dispatched; a second `BaseAdvanced` with the same `to` changes nothing (revision unchanged); one with a new `to` replaces it.
- `accept_conflict_keeps_the_run_complete`: `OpDone AcceptConflict { files }` replies `Refused` with the conflict message, leaves the run `complete` with its branches, and logs it.
- `completion_checks_refs_then_completes`: all merged → `Op VerifyRefs` → `RefsOk` → `complete`, `WriteReport`; a run head differing from `last_green_candidate` runs `Op Check` in the integration worktree first.
- `blocked_tasks_keep_the_run_running_with_attention`.
- `finish_cancels_unstarted_then_completes_when_live_ones_end`; `cancel_kills_salvages_and_completes`.

**Change.** Implement decisions 21 (engine side), 36 and 37.

**Acceptance.** Tests pass; purity holds.

**Commit.** `feat(daemon): add the tested merge queue, conflict hand-back, ref guard and completion to the reducer`

### M8a.15 Engine V: edits, retry, override, pause, restore and resume

**Files.** Create `engine/restore.rs`, `engine/tests/control.rs`. Modify `engine/requests.rs`.

**Tests first**, in `engine/tests/control.rs`:

- `edit_applies_and_delivers`: `Edit` with `amend_task brief` on a working task replies `Ok("applied 1 edit")` and queues `amend_message`; `cancel_task` on a working task gives `KillWindow`, then on `ProcessExited { killed_by_engine: true }` `RemoveWorktree` with a salvage ref, and dependents `blocked(dep_cancelled)`; a rejected batch replies `Err` with every error line.
- `answer_resumes_a_question`: `blocked(question)` → `working` with `answer_message` delivered as the same session's next turn. If that session has ended, `Op ResumeSession` carries the answer instead. If that resume fails (`ResumeFailed`), a fresh session's prompt ends with the answer.
- `retry_resets_counts_and_starts_a_fresh_session_at_rung_2`; `retry_refuses_l_and_dep_cancelled`.
- `override_only_from_review_or_blocked_with_commits`.
- `pause_edit_stops_dispatch_and_gates_but_keeps_windows`.
- `restore_pauses_running_runs_only`: `running` → `paused` (`paused_from == running`); `halted`, `awaiting_approval`, `complete`, `accepted` unchanged.
- `restore_replays_journaled_results`: a `Restore` with `replay` containing `(run, op, Merged{…})` for a pending op applies it.
- `a_paused_run_refuses_tools_and_most_requests`: `task_done` → `run <id> is paused; the user must resume it`; `Finish` refused; `Cancel` and `Resume` accepted.
- `resume_fires_only_second_stage_deadlines`: a round in `StallState::Interrupted` whose grace passed goes to rung 2 without resuming its old session. A `Watching` round is re-armed from `now` and gets `Op ResumeSession` with `RESUME_WORKER`.
- `resume_resumes_sessions_and_reissues_gate_ops`: four tasks, all after `Restore`:
  - a `working` task (worker 4, session `s-4`) gives `Op ResumeSession { window_id: 4, session_id: "s-4", message: RESUME_WORKER }`;
  - a `review` task (reviewer 6, no verdict) gives `ResumeSession` for 6 with `RESUME_REVIEWER`;
  - a `check` task gives `Op Check`;
  - a `merge_queue` task gives `Op MergeCandidate`.
- `a_failed_resume_starts_a_fresh_session`: `ResumeFailed` gives `CreateWindow` for session n+1 with `handover_prompt`, at the same rung, with no failure counted.
- `restore_marks_every_session_ended`: after `Restore`, every live round has `ended == true`, `turn_open == false` and `pid == None`.
- `stop_ignores_everything_after`.

**Change.** Implement decisions 13 (engine side), 28 (resume and the failed-resume reaction), 35's override, 42, 45 and 46 (engine side).

**Acceptance.** Tests pass; purity holds; no file under `run/engine/` exceeds 600 lines.

**Commit.** `feat(daemon): add plan edits, retry, override, pause and resume to the reducer`

### M8a.16 The run report

**Files.** Create `run/report.rs`.

**Tests first.**

- `format_utc_vectors` (the refreshed M8 brief's three vectors, restated: `0` gives `1970-01-01 00:00:00Z`; `951782400` gives `2000-02-29 00:00:00Z`; `1789123456` gives `2026-09-11 10:44:16Z`).
- `report_has_every_section` in order: `# anthrex run <id>`, `Goal:`, state, `approved by`, base and run branch, profile summary (check or `no check command: this run is unverified`), limits, `## Tasks` table (id, title, size, mode, state, rung, bounces, done signal, merge commit), then per task `## <id>: <title>` with notes, route, review route, budget and spend, each proof, each check (tail in a fenced block), each review round with verdict and findings grouped by severity, salvage refs, `merged without approval: <reason>` when set, the history; then `## Log`.
- `minor_findings_are_listed_even_when_approved`; `turn_end_fallback_is_named`; `halted_and_rebaselined_runs_say_so`; `base_moved_and_accept_conflict_are_reported` (`base <base> moved during the run: <from7>..<to7>, <n> commits, listed at accept`; an aborted accept's files in `## Log`); `usage_and_denials_are_reported` (each round's turns, tool calls, billable tokens and denials); `containment_is_reported` (`project settings: excluded`, or `project settings trusted by --trust-project: <paths>`; `worker sandbox: off ([orchestrator] worker_sandbox = false)` when off).

**Change.** Implement the report; `RunService` writes it on `WriteReport` (M8a.22) through a temp file and a rename, at most once per 500 ms per run.

**Acceptance.** Tests pass; `report.rs` is pure.

**Commit.** `feat(daemon): render the run report with gates, severities, rungs and salvage refs`

### M8a.17 Headless sessions: the process driver and headless windows

**Files.** Create `crates/daemon/src/headless/session.rs`, `crates/daemon/src/manager/headless.rs`, `crates/daemon/src/server/headless_guard.rs`, `crates/daemon/tests/headless_sessions.rs`, `crates/daemon/tests/headless_env.rs` (one test, alone in its binary), `crates/daemon/tests/headless_windows.rs`, `crates/tui/src/app_tests/headless.rs` (declared from `crates/tui/src/app/tests.rs` by `#[path]`, like its siblings). Modify `manager/entry.rs` (`Process::Headless`, `Entry.headless`), `manager/mod.rs` (`handle_hook`'s headless branch, `tick`'s headless skip, the feed), `manager/conversation.rs` (`subscribe_conversation`'s headless skip, decision 27), `manager/restore.rs`, `state/mod.rs` (`WindowRecord.kind`), `server.rs` (one guard call per refused message), the three `Subscribe` sites (`App::focus` in `crates/tui/src/app/mod.rs`, `retry_dropped_subscribe` and `on_reconnected` in `crates/tui/src/app/link.rs`), the key and mouse input paths (`crates/tui/src/app/mod.rs`'s two `Effect::Send(ClientMsg::Input { .. })` sites, `:362` and `:458`, and `crates/tui/src/mouse.rs`'s one, `:89`), and the pane renderer `crates/tui/src/ui/terminal.rs`.

**Tests first.** Every test uses real processes with real pipes. Until M8a.20, the "agent" is a `/bin/sh -c` program that prints M8a.1's fixture lines, or records its argv and stdin.

- In `headless_sessions.rs`:
  - `spawn_reads_events_in_order_and_reports_exit`: `sh -c 'cat <claude-…-stream.jsonl>; exit 3'` gives exactly the events `claude_stream::parse_line` gives for the fixture, then `ProcessExited { code: Some(3) }`.
  - `send_line_never_blocks_on_a_full_pipe`: a child that never reads, and 300 lines of 8 KiB sent from the test thread. Every call returns, and at least one returns `Err` (the queue holds `WRITER_QUEUE_MAX` = 256 lines and the pipe a few more, so the last of the 300 cannot all fit); no call has a per-call wall-clock bound, because an enqueue has no cost worth bounding. The loop runs on a helper thread the test joins with a 10 s deadline, which only a call that blocks on the pipe could reach.
  - `interrupt_by_sigint_reaches_the_child`: a child that installs a trap on `INT`, prints `ready`, then waits; the test sends the interrupt only after reading `ready` (before it, the default action would kill the child and the test would pass or fail on timing), then reads `interrupted`.
  - `kill_terminates_the_process_group`: a child that starts a background `sleep 30`. After `kill(1 s)`, both pids are gone within 2 s (`kill(pid, 0)` gives `ESRCH`).
  - `a_long_line_is_cut_and_parsed_as_unknown`.
  - `the_environment_is_scrubbed`, in `headless_env.rs` alone: with `CLAUDE_CODE_CHILD_SESSION=1`, `CLAUDECODE=1` and `ANTHREX_WINDOW_ID=9` set in the test process, the child's `env` shows neither `CLAUDE_*` variable. It shows `ANTHREX_WINDOW_ID` as the session's own window id, and `CARGO_TARGET_DIR=<dir>/target` from the profile.
- In `headless_windows.rs`, against a real manager:
  - `create_headless_registers_a_headless_window`: `WindowInfo.kind == Headless`, `run == Some(RunRef { role: Worker, .. })`, and `session_id` is set once `Init` arrives.
  - `session_events_drive_status_and_the_conversation`: the Codex fixture moves the window `Starting → Working → Idle`. `conversation_snapshot(id, None)` has the prompt turn and the tool call.
  - `real_hooks_do_not_change_a_headless_windows_status`: a `HookEvent` `PreToolUse` for the window leaves its status `Idle`. It still reaches the conversation, and a `SubagentStart` adds a sub-agent row and a feed `Hook` signal.
  - `headless_windows_persist_and_restore_as_ended`: after `state_snapshot` and `restore`, the window has `kind == Headless`, status `Exited`, and the same `HeadlessSpec` and `session_id`. An unparseable `run` value restores as an `Exited` PTY record with a warning.
  - `a_client_cannot_create_a_headless_window`: a `ClientMsg::CreateWindow` for each runtime yields a window with `kind == Pty` and `run == None`, and the test destructures `WindowSpec { name, runtime, cwd, worktree_branch, model, initial_prompt }` exhaustively, so a field added to it later fails to compile here and has to be looked at (an assertion, not only a claim about the type).
  - `a_headless_window_starts_no_transcript_reader`: `subscribe_conversation` on a headless Claude window whose `SessionStart` hook named a `transcript_path` answers with a snapshot, `conversation_reader_running(id)` stays false, and the snapshot's `degraded` is `None`, not `NoTranscriptPath`.
  - `create_headless_waits_for_the_launch_gate`: with a manager built on `LaunchGate::closed()`, `create_headless` has not spawned (no args file written) 200 ms after the call while `list()` still answers; after `open()` it spawns and returns. The negative half is a sleep-then-assert on purpose: it checks an absence, and the positive half proves the same path does spawn.
  - `tick_leaves_a_headless_windows_status_alone`: a headless window left `Working` (turn open, no stream event) for more than `QUIET_AFTER` is still `Working` after `tick()`, while a PTY window in the same state becomes `Idle`.
  - `a_headless_claude_prompt_hook_feeds_the_cursor` (M8a.7 fix round 2, ruling T7-N1): the window's `UserPromptSubmit` hooks arrive over the socket as for any window. Real hooks come from the stream fixture, plus an unprompted `<task-notification>` prompt between two engine turns, and a background turn run after the next turn was already sent. After the stream is applied, the snapshot has each prompt with its own reply and `degraded == None`.

- In `headless_windows.rs`, over a real socket: `client_control_of_a_headless_window_is_refused`. `Subscribe`, `Input`, `Kill`, `Remove` and `Restart` each get decision 49's exact `DaemonMsg::Error` and leave the process running. `Resize` gets no reply (a `ListWindows` sent after it is answered first) and changes nothing. `ListWindows` lists the window.
- In `crates/tui/src/app_tests/headless.rs`:
  - `focusing_a_headless_window_sends_no_subscribe`, and neither `retry_dropped_subscribe` nor `on_reconnected` sends one for it;
  - `keys_are_not_forwarded_to_a_headless_window`: no `Effect::Send(ClientMsg::Input { .. })` when the focused window is `Headless`, while a PTY window still gets its keys;
  - `mouse_input_is_not_forwarded_to_a_headless_window`;
  - `prefix_commands_still_work_on_a_headless_window`: `C-b m` opens the conversation;
  - the render test `the_headless_placeholder_names_the_conversation_key`, with the exact string of decision 49.

**Change.** Implement decisions 26 (the session environment), 27 (the manager side), 49 and 52 (the driver side).
- **Feed real prompt hooks to the cursor (ruling T7-N1).** For a headless window with `hooks_fire`, every real hook applied through `conversation_hook` is also passed, under the same lock and right after it, to `headless::conversation::observe_hook` with the window's `StreamCursor`. The returned records go to `ConversationSet::enrich` like `map`'s.
  - A Claude turn is then numbered by its prompt's hook and classified by its text, not by when its `Init` arrives. So an unprompted turn (a background sub-agent's notification) keeps every later reply on its own turn, whatever order Claude runs it in.
  - Keep one `StreamCursor` per window for its whole life, across a `--resume` process, so its counts keep matching the conversation's.

`manager/headless.rs` holds `create_headless`, `apply_session_event`, `signals`, and the `headless_*` methods that M8a.18 fills in. Only `apply_session_event`'s status, conversation and feed updates run under the manager lock; spawning, writing and killing never do (AGENTS.md rule 2).

**Acceptance.** Tests pass. `create.rs` and `restart.rs` are unchanged (`git diff --stat` shows neither). No `crate::lock` guard in `manager/headless.rs` is alive across `.await`, a spawn, a write or a kill.

**Commit.** `feat(daemon): run headless agent sessions as manager windows with no terminal`

### M8a.18 Turn delivery, interrupt and resume

**Files.** Modify `headless/session.rs` and `manager/headless.rs`. Create `crates/daemon/tests/headless_turns.rs`.

**Tests first**, in `headless_turns.rs`, with recording programs as `claude_bin` and `codex_bin` (a script that appends its argv to `args.log` and each stdin line to `stdin.log`, then prints a scripted fixture turn):
- `claude_send_writes_one_envelope_line`: `headless_send(id, "hi")` writes exactly one line to the running process's stdin, equal to `claude_stream::user_message("hi", Some(<session>))`.
- `codex_send_spawns_exec_resume_with_the_message_last`: the logged argv is decision 25's resume argv, ending in `--`, `hi`.
- `a_send_while_a_codex_turn_is_running_is_refused`: `a turn is already running for window <id>`. The reducer never does this (decision 29), so it is an error, not a queue.
- `claude_resume_kills_a_live_process_first`: while the first process is alive, `headless_resume` stops it (decision 52), then starts `--resume <id>` with every flag of the first argv present, and writes the message.
- `interrupt_follows_cli_caps`: `InterruptMode::ControlRequest` writes `claude_stream::interrupt_request`, and `Sigint` signals the process. Codex always gets `SIGINT`.
- `a_send_to_an_ended_session_is_an_error`: `session for window <id> has ended; resume it`.
- `resume_failure_is_reported`: a program that exits 1 before printing `Init` makes `headless_resume` return an error whose text the driver maps to `ResumeFailed`.
- `sent_turn_is_recorded_before_the_write` (M8a.7 fix round 2): the recording program prints its turn's first stream line only after it reads its input (Claude) or starts (Codex). The window's `StreamCursor` must have applied `conversation::sent_turn` for that message before the first line of the turn is applied.
  - For Codex, this puts the snapshot's tool call and prose on the new turn, not on the previous one.
  - Checked by a `headless_send` followed by a scripted turn whose prose must land on the new prompt's turn.

- `a_background_turns_result_does_not_end_the_delivered_turn` (M8a.7 re-review 2, carried): in the order where a background sub-agent's notification turn runs just as the daemon delivers its next message, the background turn's `result` must not mark the delivered turn ended, count toward its tool calls or budget, or trigger the next delivery. Attribute each `TurnEnded` to the turn the cursor classified (by prompt text, `conversation::observe_hook`), not simply to "the open delivered turn".
- `sent_turn_records_the_clamped_text` (M8a.7 m3/m4): `sent_turn` receives exactly the text written to the process after any clamping, so a clamped prompt is still recognised as the daemon's.
- `a_fed_window_starts_its_cursor_in_content_mode` and `a_late_prompt_hook_drops_only_its_own_turn` (M8a.7 m1/m2): for a Claude window whose hooks are fed, the cursor is content-based from its first `sent_turn` (no timing path), and prose is dropped only while `turns_ended >= prompts_seen`, so a prompt hook that arrives before the previous `result` never drops the previous turn's reply.

**Change.** Implement decision 29's I/O and decision 28's resume.
- **Apply `sent_turn` before writing or spawning** (M8a.7 fix round 2).
  - For every send and every resume message, the manager applies `headless::conversation::sent_turn` to the window's cursor, and enriches with its output, before the message is written to a Claude process's stdin or a `codex exec` process is spawned.
  - For Codex this is binding. `sent_turn` synthesises the prompt's `UserPromptSubmit` hook, and the turn's events would otherwise land on the previous turn.
  - For Claude, ruling T7-N1's content rule (M8a.17's hook feed) makes the order irrelevant. A prompt observed before its `sent_turn` is still classified as the daemon's. The same order is used anyway, for one code path.
- A Claude send is one `send_line` of `user_message(clamp(text))`.
- A Codex send spawns `codex_args(spec, Resume { session_id }, text, …)` after `jitter_ms`.
- A resume spawns the resume argv, then sends the message: on Claude's stdin, or as Codex's argument.
- Nothing is written from under a lock, and nothing blocks a tokio worker.

**Acceptance.** Tests pass. `rg -nw "write_input" crates/daemon/src/run crates/daemon/src/headless` prints nothing (`-w`: M8a.18 fix round 1).

**Commit.** `feat(daemon): deliver engine messages to headless sessions as turns, and interrupt and resume them`
### M8a.19 The MCP server

**Files.** Create `crates/mcp/Cargo.toml`, `crates/mcp/src/lib.rs`, `src/tools.rs`, `src/forward.rs`, `crates/mcp/tests/stdio.rs`, `crates/cli/tests/mcp_cli.rs`. Modify the workspace `Cargo.toml` (member, `rmcp` pin), `crates/cli/Cargo.toml`, `crates/cli/src/main.rs` (hidden `mcp`).

**Tests first.**

- In `tools.rs`: `worker_tools_are_task_done_and_task_blocked`; `reviewer_tool_is_submit_review`; `orchestrator_tools_are_empty`; `every_schema_is_a_closed_object`; `schema_limits` (the enums, `maxItems: 50`, the `red` pattern).
- In `crates/mcp/tests/stdio.rs`, against a stub daemon (a `UnixListener` under `/tmp` answering `Hello` with `Welcome` and each `Run(Tool)` with a scripted `ToolResult`, recording what it received), through `serve_on` over a `tokio::io::duplex` pair: `initialize_then_list_tools` (server name `anthrex`, protocol `2025-06-18`); `tool_call_is_forwarded_with_role_run_task_and_window`; `daemon_error_becomes_is_error`; `daemon_down_is_a_tool_error_not_a_crash`; `a_tool_outside_the_role_never_reaches_the_daemon`.
- In `crates/cli/tests/mcp_cli.rs`: `mcp_subcommand_speaks_json_rpc_on_stdout_only`.

**Change.** Implement decisions 4 and 5 and the MCP interface.

**Acceptance.** Tests pass. `cargo tree -p anthrex-mcp` shows `rmcp v3.4.0`; `cargo tree -p anthrex-daemon` and `-p anthrex-tui` contain no `rmcp`.

**Commit.** `feat(mcp): add the anthrex MCP server with the worker and reviewer tools`

### M8a.20 `fake-agent` headless modes

**Files.** Modify `crates/fake-agent/src/main.rs` (mode dispatch), `runtime.rs`, `script.rs`. Create `src/headless.rs`, `src/stream_claude.rs`, `src/stream_codex.rs`, `src/roles.rs`, `src/mcp.rs`, and `crates/fake-agent/tests/headless_modes.rs`.

**Change.** Milestone 3's PTY behaviour stays unchanged for any argv without `-p` or `exec`. Add:

1. **Mode detection.**
   - Claude headless: argv has `-p` and `--input-format stream-json`.
   - Codex headless: `exec` is the first argument, and `exec resume <id>` means resume. It accepts and ignores decision 53's test placeholder `--anthrex-test-exclude-project-config`, which the argv log records.
   - Role and task: parsed from the MCP server's arguments in `--mcp-config <json>` (Claude) or in the `-c mcp_servers.anthrex.command=` / `-c mcp_servers.anthrex.args=` pairs (Codex, parsed with `toml`). The role is the value after `--role`, the task the value after `--task`.
2. **Output shapes.** Output is only the shapes of M8a.1's fixtures (decision 51).
   - Claude: `system/init` first, with `session_id` from `--session-id` or `--resume` and `mcp_servers: [{"name":"anthrex","status":"connected"}]`. Then `assistant` and `user` lines for text and tool use, and a `result` at every turn end with `usage`.
   - Codex: `thread.started` (a fresh id, or the resumed one), then `turn.started`, `item.*` and `turn.completed` with `usage`.
   - Each `mcp_call` is echoed as a `tool_use` / `tool_result` pair, or as an `mcp_tool_call` item.
   - `FAKE_AGENT_ARGS_FILE` gets the argv, and new `FAKE_AGENT_STDIN_FILE` gets every stdin line. When either value is a directory, each claimed script gets its own `<script name>.args` or `<script name>.stdin` file there, with one line appended per process or per line, so a test can read one session's history.
3. **Input.** In Claude mode, each stdin line must parse as M8a.1's user-message envelope; anything else exits 5 with `fake-agent: bad input line`. That is how a wrong envelope fails a test. A control request with subtype `interrupt` ends the running step and emits the interrupted `result`. In Codex mode, the turn's message is the last argument.
4. **Per-role scripts.** In `<git common dir>/fake-agent/`, found with `git rev-parse --path-format=absolute --git-common-dir` in the cwd, take `<role>-<task>-<n>.jsonl` with the smallest `n` whose `<file>.claimed` does not exist, claimed with `OpenOptions::create_new`. With none, fall back to `FAKE_AGENT_SCRIPT`.
   - A resumed session continues its claimed script: the claim file holds the session id, and `<file>.pos` holds the next step's index. This is how a Codex session spans processes and a Claude session survives `--resume`.
5. **Steps**, added to M3's:
   - **`mcp_call {tool, args, expect_error?}`** replaces the stub and its test `mcp_call_is_not_supported_yet`. It spawns the configured server, then sends `initialize` (`2025-06-18`), `notifications/initialized` and `tools/call`. It keeps the text as `FAKE_AGENT_RESULT`. A tool error exits 3 unless `expect_error`, and success with `expect_error` also exits 3.
   - **`read_message {timeout_ms?, expect?}`** ends the turn and takes the next message. It exits 3 when `expect` is not contained and 4 on timeout; the message is kept as `FAKE_AGENT_MESSAGE`.
   - **`end_turn`** ends the turn without waiting.
   - **`sh {cmd}`**: runs `/bin/sh -c` in the cwd with `FAKE_AGENT_MESSAGE` and `FAKE_AGENT_RESULT` set, emitted as a `Bash` tool use; the step continues.
   - **`capture {name, sh}`**: `{{name}}` in any later `mcp_call` argument is replaced by the trimmed stdout.
   - **`api_retry {error, delay_ms, times}`**: emits `system/api_retry` `times` times, `delay_ms` apart (Claude only).
   - **`fail_turn {error}`**: ends the turn failed. Claude emits a `result` with `is_error` and the error category; Codex emits `turn.failed` with the recorded rate-limit message when `error` is `rate_limit`.
   - **`deny {tool, reason}`**: emits `permission_denied` (Claude only) and adds the denial to the turn's `permission_denials`.
   - **`usage {input, output, cache_read, cache_write}`**: sets the next turn end's usage.
   - **`hang`**: blocks with no output until killed or interrupted (the stall).
   - M3's **`exit`**: mid-turn, it is a process that dies without a turn end.
   - M3's `hook`: still sends a real hook through `anthrex hook`. Headless Claude scripts use it for `SubagentStart` / `SubagentStop`.
6. **End of script and empty turns.** The end of the script ends the turn. Claude then waits on stdin, and exits 0 at EOF; Codex exits 0. A message arriving after the script has ended gets an empty turn.

**Tests first**, in `headless_modes.rs`:
- `claude_mode_output_conforms_to_the_fixture` and `codex_mode_output_conforms_to_the_fixture`: decision 51's shape test over every event type each mode emits.
- `claude_mode_reads_the_recorded_envelope_and_rejects_others`.
- `a_codex_session_continues_its_script_across_processes`.
- `claims_scripts_in_order`.
- `mcp_call_talks_to_a_real_mcp_server`: the real `anthrex mcp`, built behind a `OnceLock` when absent, against a stub daemon.
- `mcp_call_expect_error`.
- `sh_step_sees_the_message`.
- `capture_fills_a_template`: a `capture` of `git rev-parse HEAD` appears as `red` in the recorded `task_done` arguments.
- `interrupt_ends_a_hang`.
- `api_retry_and_fail_turn_shapes`.
- `resume_argv_is_recorded`.

**Acceptance.** Tests pass, and milestone 3's `fake-agent` tests still pass.

**Commit.** `feat(fake-agent): add headless Claude and Codex modes with recorded-shape events, per-role scripts and real MCP calls`
### M8a.21 The intent journal and reconciliation

**Files.** Create `run/journal.rs`, `run/reconcile.rs`, `crates/daemon/tests/run_journal.rs`.

**Tests first.**

- `save_run_is_atomic_and_round_trips` (a `Run` with every nested record saves and loads equal; a leftover `run.json.tmp` from a simulated crash is ignored and removed).
- `journal_lines_round_trip_and_a_torn_last_line_is_dropped` (a final line without `\n` is ignored with a problem).
- `load_all_skips_a_bad_run_with_a_problem`.
- `compact_keeps_only_pending_intents`.
- Reconcile, per row of the Interfaces table, each against a real `TempRepo` state built to match: `reconcile_prepare_worktree_done_and_not_started` (including a partial directory not in the worktree list, which is removed); `reconcile_create_window_finds_the_restored_window`; `reconcile_kills_a_leftover_session_process_by_its_session_id` (a `sleep` started with the session id in its argv is killed; a `sleep` without it is left alone and cleaned up by the test); `reconcile_merge_candidate_already_advanced`; `reconcile_merge_candidate_not_advanced_reattaches_the_integration_worktree` (the integration worktree left detached at an unmerged candidate is back on its branch afterwards); `reconcile_merge_candidate_ref_moved`; `reconcile_hand_back_with_merge_head`; `reconcile_remove_worktree_gone_with_salvage_ref`; `reconcile_accept_already_merged`; `reconcile_done_line_replays_without_touching_git` (a `done` line with no reality check: the recording git logs nothing).

**Change.** Implement decisions 43 and 44 (the I/O parts).

**Acceptance.** Tests pass; every write in `journal.rs` is followed by `sync_all` (checked by reading the code and named in the pull request).

**Commit.** `feat(daemon): journal run intents with fsync and reconcile them against git and windows`

### M8a.22 `RunService`, the server and lifecycle

**Files.** Create `run/driver.rs`, `run/driver/ops.rs`, `run/driver/observe.rs`, `crates/daemon/src/server/run_api.rs`, `crates/daemon/src/server/git_wiring.rs`, `crates/cli/tests/support/run_harness.rs`, `crates/cli/tests/run_e2e_basic.rs`. Modify `run/mod.rs`, `server.rs`, `lifecycle.rs`, `crates/cli/tests/support/mod.rs`.

**Tests first**, in `run_e2e_basic.rs`, with raw socket requests (the CLI arrives in M8a.23):

- `e2e_green_s_task_runs_to_merged`: profile `check = "true"`, one S `check`-mode task (reason `smoke`), Claude worker, `yes`. `worker-t1-1`: `git_commit a.txt`, `usage` (the values below), `DONE {"summary":"added a"}`. `reviewer-t1-1`: `mcp_call submit_review {"verdict":"approve","summary":"ok","findings":[]}`. Assert within `RUN_WAIT`: run `complete`; `t1` `merged` with `done_signal == task_done`; `git log --merges anthrex/<id>/integration` has one merge with parents `(base_sha, t1 head)`; the task, review and proof worktrees are gone and the integration worktree remains; `REPORT.md` contains `## t1:`; the worker's `WindowInfo` has `kind == Headless` and `run == Some(RunRef { task_id: Some("t1"), role: Worker, session: 1, .. })`; the reviewer's role is `Reviewer` on the Codex runtime; `FAKE_AGENT_STDIN_FILE` of the worker holds the first turn as one stream-json envelope; the reviewer window is `Exited` and then gone from `ListWindows` within `RETIRE_AFTER` + 5 s; the worker round's `usage` equals the scripted usage, which is `{"input":11,"output":23,"cache_read":37,"cache_write":41}` (four distinct values, so a swapped field fails).
- `e2e_plan_gate_waits_and_approve_runs`: without `yes`, the run stays `awaiting_approval` with the integration worktree existing and no window; `Approve` runs it to `complete`.
- `e2e_plan_gate_survives_a_restart`: `awaiting_approval`, `restart_daemon()`, still `awaiting_approval`, then approve and complete.
- `e2e_subscribe_pushes_snapshots_with_rising_revisions`: every `Snapshot` received has a revision greater than the previous one, and the task's state sequence contains `queued` or `preparing`, `working`, `check`, `review`, `merge_queue`, `merged` in that order.
- `e2e_task_worktree_is_watched`: with `ANTHREX_GIT` unset, a `DaemonMsg::Git` arrives for the task worktree whose head is `Branch("anthrex/<id>/t1")`.
- `e2e_a_worker_is_watchable_through_its_conversation`: with milestone 6.5's `ClientMsg::SubscribeConversation { window_id: <worker>, agent_id: None, from_rev: None }`, the conversation shows the first turn's prompt text and the `mcp__anthrex__task_done` tool call.
- `e2e_dirty_tree_refuses_to_start` and `e2e_validation_errors_come_back_together`.
- `e2e_project_settings_are_refused_without_trust_project`: daemon started with `ANTHREX_TEST_NO_SETTING_SOURCES=1`. One repo whose base commit tracks `.claude/settings.json` with a `hooks` key, and one that tracks `.mcp.json`: each `Start` with a Claude task is `Refused` with decision 53's exact message naming the path, and no branch is created. A Codex-only plan in the same repo starts.
- `e2e_trust_project_is_accepted_and_reported`: the same repo with `trust_project: true` starts and completes. `RunInfo.trusted_project` names the file, and `REPORT.md` contains `project settings trusted by --trust-project: .claude/settings.json`.
- `e2e_codex_project_config_follows_cli_caps`, one case per branch of decision 53's Codex bullet:
  - With `ANTHREX_TEST_CODEX_PROJECT_CONFIG=load`, a repo tracking `.codex/config.toml` refuses a plan with a Codex task, naming the file; `--trust-project` starts it and reports it; a Claude-only plan is not refused for it.
  - With `=exclude`, the same plan starts, and the Codex worker's argv logs carry the placeholder flag on both the `exec` and the `exec resume` turns.
  - With the variable unset and the real `CLI_CAPS`, the behaviour matches whichever branch `CLI_CAPS` names, and the assertion is chosen from `CLI_CAPS` at run time.
- `e2e_protected_file_bounces_then_merges`: `t1` owns `src/**` and its worker commits `src/a.rs` and an edit to the base commit's `AGENTS.md`, then calls `task_done` (`expect_error`), reads `configures or instructs future agents` in the result, reverts `AGENTS.md` with `git checkout HEAD~1 -- AGENTS.md && git commit -qm revert`, and `DONE`. Assert `bounces.done == 1` and `merged`, and that `AGENTS.md` on the run branch is unchanged. A second task `t2` owning `AGENTS.md` exactly edits it and merges with no bounce.
- `e2e_protected_warning_is_printed_at_start`: `anthrex run start` on a plan whose task owns `**` in a repo tracking `AGENTS.md` prints the rule-6.protected warning on stderr and still starts.
- `e2e_claude_sessions_load_only_user_settings`: without the override, the assertion follows `CLI_CAPS` at run time and is never empty: with `claude_user_settings_only == Some(flags)` the worker's recorded argv contains each flag exactly once; with `None` it contains none of the flags M8a.1 tried, and a plan in a repository that tracks `.claude/settings.json` with hooks is refused (decision 53).
- `e2e_generated_file_bounces_then_merges`: the base commit tracks `Cargo.lock`, and the profile has `generated = ["Cargo.lock"]`. The worker commits `a.txt` (owned) together with a change to `Cargo.lock` (not owned), then calls `task_done`, whose result must fail (`expect_error`). It then runs `sh` `git checkout HEAD~1 -- Cargo.lock && git commit -qm "revert Cargo.lock"`, and `DONE`. Assert `bounces.done == 1`, `rung` stayed 1, and the task merges.

**Change.**

1. `RunService` owns `Mutex<EngineState>` (taken with `crate::lock`), an unbounded `mpsc` of events, a reply map from `ReplyId` to `oneshot::Sender`, the snapshot `watch`, the `GitQueue`, and retire deadlines.
2. `spawn` starts three tasks:
   - the event loop;
   - a signal forwarder on `WindowManager::signals()`, which maps each `WindowSignal` of a run's headless window to `Event::Signal`: `Init`, `TurnStarted`, `ToolUse`, `TurnEnded`, `ApiRetry`, `PermissionDenied` and `ProcessExited` one to one, `SubagentStart` and `SubagentStop` from hooks, and everything else to `Activity`, at most one per window per second;
   - a 1-second ticker feeding `Tick`, running retire checks, and flushing coalesced snapshots and counter-only persists.
3. The event loop takes the engine lock only around `step` and `snapshot`, then executes effects in decision 43's order with the lock released: `Persist` → `journal::save_run` on `spawn_blocking`, awaited; `Op` → journal intent (awaited), crash injection (decision 48), then the op on its own task (git through `GitQueue` for writes, `spawn_blocking` for everything blocking, `create_headless`/`headless_resume` for sessions after `jitter_ms`, with the Claude binary from `ManagerConfig`'s `claude_bin`/`codex_bin`, so `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` apply), whose completion appends `done` and sends `OpDone`; `Deliver` → `headless_send` on a per-window task, then `Delivered`; `Interrupt` → `headless_interrupt`; `KillWindow`, `RemoveWindow` → `headless_kill` and removal; `RetireWindow` → close stdin, then a deadline; `WatchWorktree`/`UnwatchWorktree` → `GitRoots`; `WriteReport` → the report; `Publish` → the watch.
4. `request(Start)` runs preflight on `spawn_blocking`, then decision 53's project-settings check when it applies, parses and builds the run (validation errors joined with `\n` into `Refused`), picks the id, then sends `Start`. `Finish` checks `confirm` first; for accept it reads `refs/heads/<base>` first and, when the base advanced, builds `ConfirmNeeded.base_moved` with `commits_since` and requires `confirm == "<run id>@<to>"` (decision 20). `Resume { rebaseline: true }` reads both refs before sending the event. A `MergeCandidate` or `VerifyRefs` op whose `guard_refs` returns `BaseAdvanced` sends `Event::BaseAdvanced` before its `OpDone` and carries on (decision 21).
5. `server.rs`: `GitWiring` (in `server/git_wiring.rs`) built in `lifecycle::run` from `git::settings_from_env(loaded_config.git)`, the value `serve` receives today; `serve` takes it and `Arc<RunService>`; `server/run_api.rs` answers each `RunRequest` on its own spawned task through a clone of the connection's outgoing channel, so a slow git operation never stalls the connection; `Subscribe` starts forwarding the snapshot watch to that connection until `Unsubscribe` or disconnect.
6. `lifecycle::run`: after `manager.restore`, construct `RunService`, `await restore()` (load, reconcile, `Event::Restore`) before binding the socket; spawn it; call `stop()` before `manager.shutdown()`.

**Acceptance.** Tests pass. Every `crate::lock(` in `run/driver*.rs` is released before any `.await`, `spawn_blocking`, git call or manager call — state this check in the pull request. `rg -n "GitRegistry::new" crates/daemon/src` prints only `server/git_wiring.rs`'s `GitWiring::new` and tests. `wc -l crates/daemon/src/server.rs` is at most 605.

**Commit.** Two commits. First, a pure move: `refactor(daemon): move GitWiring and pump_git into server/git_wiring.rs` (no behaviour change, every existing test passes unchanged). Then `feat(daemon): drive the run reducer from RunService and serve runs over the socket`.

### M8a.23 The `anthrex run` commands

**Files.** Create `crates/cli/src/run_cmd.rs`, `crates/cli/src/run_cmd/status.rs`, `crates/cli/tests/run_cli.rs`. Modify `crates/cli/src/main.rs`.

**Tests first.**

- Unit tests in `run_cmd.rs`: `resolve_run_by_id_suffix_and_prefix`; in `status.rs`: `status_text_matches_the_layout` (a fixed `RunsSnapshot` renders exactly the Interfaces example), `paused_and_halted_lines`, `base_moved_prompt_lists_the_commits` (a `ConfirmNeeded` with 53 commits prints 50 lines and `… and 3 more`), `bounces_column_text`.
- In `run_cli.rs` with the harness: `start_approve_status_accept` (`run start` prints the id and the approve hint; `run approve`; `run status <id> --json` parses as `RunsSnapshot`; wait for `complete`; `run accept <id> --yes`; state `accepted`, `main` has the work, every `anthrex/<id>/` branch is gone); `reject_needs_the_id`; `discard_keeps_salvage_refs`; `edit_from_a_file` (an edits file cancelling a pending task); `retry_and_override_reach_the_daemon`; `trust_project_flag_reaches_the_daemon`; `accept_of_a_running_run_is_refused` (exit 1, the message).

**Change.** Implement the CLI section. Exit 1 with the daemon's message on every `Refused`.

**Acceptance.** Tests pass. `anthrex run --help` lists the eleven subcommands; `anthrex --help` does not list `mcp`.

**Commit.** `feat(cli): add anthrex run start, status, approve, reject, edit, retry, override, cancel, resume, accept and discard`

### M8a.24 End-to-end scenarios I: gates and review

**Files.** Create `crates/cli/tests/run_e2e_gates.rs`, `crates/cli/tests/run_e2e_review.rs`.

**Tests first.** Profile for tdd tests: `single_test = "sh tests/{test}.sh"`, `test_passed = "PASS {test}"`, `check = "sh check.sh"` where the repo's `check.sh` exits 0 unless a file `broken` exists. Workers Claude, reviewers the engine's choice (Codex) unless stated.

- `e2e_tdd_red_commit_that_passes_is_rejected`: the red commit's test already passes; the worker then reads `The test proof failed: at the red commit`, commits a real red and green pair and `task_done`s again. Assert `bounces.proof == 1`, then `merged`.
- `e2e_tdd_test_that_did_not_run_is_rejected`: the head test exits 0 without printing `PASS t_reset`; the message names the regex.
- `e2e_check_fails_once_then_passes`: the first commit adds `broken`; `read_message {"expect":"The check failed"}`; the worker removes it and `task_done`s. Assert `bounces.check == 1`, two check records, `merged`.
- `e2e_two_rejections_then_a_fresh_peer_worker_is_approved`: worker route `high` effort; `reviewer-t1-1` and `-2` submit `changes` with one `important` finding each, with different files, lines and texts (`a.rs:3 "off by one"`, then `b.rs:9 "missing check"`), so round 2's prompt can be checked to list round 1's finding and not its own; `worker-t1-1` fixes once (rung 1) and is killed at rung 2; `worker-t1-2` (a Codex fake agent, claimed by the fresh session) commits and `task_done`s; `reviewer-t1-3` approves. Assert the second session's window runtime is Codex, `rounds` shows sessions 1 and 2, `failures == 2`, `merged`.
- `e2e_disputed_finding_blocks_as_a_question_and_the_answer_resumes_it`: the worker calls `task_blocked {"kind":"question","reason":"the finding is wrong"}` after the rejection; the run is `running` with `t1 blocked (question)` in `attention`; an `answer` edit is read by the worker (`read_message {"expect":"Answer to your question"}`), which `task_done`s; round 2 approves.
- `e2e_override_merges_without_approval_and_is_reported`: plan `max_bounces = 1`; both review rounds submit `changes` with one `critical` finding, each with its own file, line and text; the second rejection makes `bounces.review == 2 > 1`, so the task is `blocked(mis_sized)` at rung 3; `run override <id> t1 --reason trusted` sends it to the merge queue, it passes the candidate check and merges; `merged_without_approval == Some("trusted")` and `REPORT.md` contains `merged without approval: trusted`.
- `e2e_minor_findings_do_not_bounce`: `changes` with only `minor` findings merges at once and the report lists them.
- `e2e_rate_limit_retry_is_not_a_stall_and_a_failed_turn_is_continued`: config `stall_after_secs = 5`, `rate_limit_retry_secs = 7` (different values, so an engine that waits `stall_after_secs` for the continue fails the timing assertion below). The worker script:
  1. commits;
  2. `api_retry {"error":"rate_limit","delay_ms":8000,"times":1}`, then `wait_ms 9000` with no output;
  3. `fail_turn {"error":"rate_limit"}`;
  4. `read_message {"expect":"stopped on an API error"}`, then `DONE`.

  Assert:
  - no interrupt control request is in the worker's `FAKE_AGENT_STDIN_FILE`, because the 9 s silence was inside the suspended stall clock;
  - the window showed `Attention` while rate-limited;
  - the continue message arrived at least 7 s (`rate_limit_retry_secs`) after the failed turn;
  - `rate_limits["claude"] == 1`;
  - the run completes.
- `e2e_turn_end_fallback_completes_a_silent_worker`: the worker commits and its script ends without `task_done`. The engine's `DONE_NUDGE` gets an empty turn. The task proceeds with `done_signal == turn_end_fallback` and merges, and the worker's stdin file holds the nudge text.
- `e2e_plan_with_an_l_task_is_rejected` and `e2e_cross_runtime_overlapping_owns_are_rejected`: `RunRequest::Start` returns `Refused` with the rule's exact line; no branch is created.

**Change.** Only tests, `fake-agent` scripts, and the fixes they uncover.

**Acceptance.** Tests pass.

**Commit.** `test: cover the proof, check, review, dispute, override and done-signal scenarios end to end`

### M8a.25 End-to-end scenarios II: ladder, merge, safety, sessions, crashes, and the smoke stage

**Files.** Create `crates/cli/tests/run_e2e_merge.rs`, `crates/cli/tests/run_e2e_safety.rs`, `crates/cli/tests/run_e2e_crash.rs`, `crates/cli/tests/run_e2e_sessions.rs` (the restart, death, refusal and denial tests), `scripts/pty_smoke_run.py`. Modify `scripts/pty-smoke.py` (≤ 10 lines).

**Tests first.**

- `e2e_stall_escalates_to_a_fresh_session_on_the_peer_runtime`: `stall_after_secs = 5`; worker route Claude `standard` `high`. `worker-t1-1` commits a red test, then `hang`; the engine's interrupt ends it; `read_message {"expect":"interrupted after"}`; `hang` again. `worker-t1-2` (Codex) commits the green change and `task_done`s naming session 1's red commit (captured from `git log`). Assert: session 1's stdin file holds one interrupt control request (or the process saw `SIGINT`, per `CLI_CAPS`); `stalls == 1`; the second session's runtime is Codex and its prompt, the last argument in `FAKE_AGENT_ARGS_FILE`, contains `This is session 2 of this task.` and the diff stat; the task merges.
- `e2e_mis_sized_task_blocks_and_is_split_by_an_edit`: the worker calls `task_blocked {"kind":"mis_sized",…}`; `t2` is `blocked(mis_sized)` with size M; a `split_task t2 into [t2a, t2b]` edit with disjoint `owns`; both run and merge; `t3`, which depended on `t2`, runs after both.
- **How the conflict tests produce a conflict.** Under decision 41 two tasks whose `owns` intersect never run together, and under decision 32 a task cannot finish with a change outside its `owns`, so two legal tasks can never change the same path and never conflict. The one legal route to a real conflict is a task whose spill the user accepts with `run override` (decision 35 exempts an overridden task from the spill check). Both tests use it. Common setup: the initial commit contains `b/shared.txt` with the line `base`; profile `check = "true"`; `[orchestrator] review.small = "off"`; `max_writers = 3`; every task S, `check` mode (reason `test`), Claude.
- `e2e_conflict_is_handed_back_and_resolved`: `t1` owns `a/**`, `t2` owns `b/**`, independent. `worker-t1-1` commits `a/one.txt` and changes `b/shared.txt` to `from t1`, then `DONE`: `t1` is `blocked(mis_sized)` with `changed files outside owns: b/shared.txt`. `worker-t2-1` changes `b/shared.txt` to `from t2` and `DONE`; `t2` merges. The test then runs `run override <id> t1 --reason shared-ok`. The candidate conflicts on `b/shared.txt`. The engine hands back: rung 3 ended session 1, so decision 29 resumes it (`--resume <session 1's id>`) carrying the conflict message, and `worker-t1-1`'s script continues. After its first `DONE` it has `read_message {"expect":"conflicts with the run branch"}`, then `sh` `printf 'from t1 and t2\n' > b/shared.txt && git add b/shared.txt && git commit --no-edit`, then `DONE`. Assert `conflicts == 1`, the resume argv carries session 1's id, `t1` `merged` with `merged_without_approval == Some("shared-ok")`, `b/shared.txt` on the run branch reads `from t1 and t2`, and no proof, check or review op ran for `t1` between the hand-back and the merge (the report's history shows `hand-back` followed by `merge queue`).
- `e2e_second_conflict_blocks_the_task`: as above, plus `t3` owning `b/**` (so it waits for `t2` by decision 41) whose worker changes `b/shared.txt` to `from t3`. `worker-t1-1`, resumed, reads the hand-back message, then waits with `sh` `for i in $(seq 1 1500); do git log --all --format=%s | grep -q 'anthrex: merge t3' && exit 0; sleep 0.2; done; exit 1` until `t3` has merged (1500 × 0.2 s = 300 s = one `RUN_WAIT`, the legal worst case of `t3`'s own path; the earlier 600 iterations, 120 s, were below it), then resolves against its own merge and `DONE`s. The second candidate conflicts with `t3`'s change: `t1` is `blocked(conflict)`, `conflicts == 2`, and it received exactly one conflict message.
- `e2e_red_candidate_goes_back_to_the_worker`: `t1` owns `a/**` and adds `a/flag`; `t2` owns `b/**` and adds `b/need-no-flag`; the repo's `check.sh` fails when both `a/flag` and `b/need-no-flag` exist. Both independent, `max_writers = 2`; `t1` merges first (its candidate check passes); `t2`'s task check passes in its own worktree (no `a/flag` there), its candidate check fails; the worker reads `merged cleanly into the run branch, but the check failed on the merged result`, deletes `b/need-no-flag`, `task_done`s, and merges. Assert `bounces.merge == 1`.
- `e2e_cancel_of_a_dirty_running_task_is_salvaged`: the worker writes an uncommitted `a/wip.txt` with `sh`, then `hang`s; once `a/wip.txt` exists in the task worktree (a deadline loop, not a sleep), the `cancel_task t1` edit; assert `t1` `cancelled`, the worktree path gone, `refs/anthrex/salvage/<id>/t1/1` exists and its tree contains `a/wip.txt`.
- `e2e_moved_run_ref_halts_the_run`: `t1` merges; `t2`'s reviewer script first waits with `sh` (a deadline loop of at most one `RUN_WAIT`) until `<tmp>/release-review` exists, then approves. While it waits (a gate, not a "slow check": a slow check would race the test, and a blocked check would hit the harness's 10 s `check_timeout_secs`), the test runs `git update-ref refs/heads/anthrex/<id>/integration <some other commit>`, then creates the release file, so the ref has moved before `t2` reaches the merge queue; assert `halted` with `refs/heads/anthrex/<id>/integration moved from`, no merge happens; `run resume --rebaseline` continues it to `complete`.
- `e2e_base_advanced_during_run_continues_and_accept_lists_it`: two tasks; `t1`'s worker, after its first commit, waits with `sh` (a deadline loop of at most one `RUN_WAIT`) until `<tmp>/base-moved` exists; the test commits `other.txt` on `main` in the repo, then creates that file, so the commit lands while `t1` works by construction. Assert `t1` and `t2` both merge, the run is `complete` (never `halted`), `base_moved.to` is the new `main` head and `attention` holds the `base main moved` line. `run accept <id> --yes` alone does not merge: the CLI prints the commit with its author and asks. `run accept <id> --base <new head>` merges; `main`'s merge commit has parents `(new head, run head)` and `other.txt` is still there. Then a second run whose task and a commit on `main` change the same line: accept with `--base` exits 1 with `accept conflicts with`, `main` is unchanged, `root` has no `MERGE_HEAD`, and the run is still `complete`.
- `e2e_base_rewritten_halts_the_run`: while a task works (held the same way as above, on `<tmp>/base-moved`), the test runs `git update-ref refs/heads/main <a commit that is not a descendant of base_sha>`, then releases it; at its merge the run halts with `refs/heads/main was rewritten`; `run resume --rebaseline` continues it to `complete`.
- `e2e_crash_after_each_intent_kind_reconciles`: for each kind in `CreateRunBranch`, `PrepareWorktree`, `CreateWindow`, `VerifyDone`, `Check`, `PrepareReview`, `MergeCandidate`, `RemoveWorktree`: start the daemon with `ANTHREX_TEST_ABORT_AFTER_INTENT=<kind>`, start the green one-task run with `yes`, wait until the daemon process has exited (socket gone), restart it without the variable, `run resume` if the run is `paused`, and wait for `complete`. Assert, for every kind, the same end state as `e2e_green_s_task_runs_to_merged`: one merge on the run branch with the same tree, no task, review or proof worktree left, the integration worktree present and on its branch, no duplicate `anthrex/<id>/t1` branch, and `journal.jsonl` has no intent without a `done` for a completed op.
- `e2e_daemon_restart_pauses_and_resume_continues`: two independent tasks, `t1` on Claude and `t2` on Codex, with disjoint `owns`. Each worker commits, then `read_message {"expect":"The daemon restarted"}`, then `DONE`. The test waits until both rounds have a `session_id` and their turns have ended, then calls `restart_daemon()`.
  - After the restart: the run is `paused (from running)`, both windows are `kind == Headless` and `Exited`, and no process whose argv holds either session id is alive.
  - After `run resume`: `t1`'s new argv has `--resume <t1's session id>` and every other flag of its first argv; `t2`'s has `exec resume <t2's thread id>`, ending in `RESUME_WORKER`'s text.
  - Both tasks merge, and `failures == 0` on both.
- `e2e_process_that_dies_mid_turn_is_resumed_once`: `worker-t1-1` commits, then `exit 1` mid-turn with no `result`. The engine resumes the session; the script continues with `read_message {"expect":"stopped in the middle of a turn"}` and `DONE`. Assert the round's `deaths == 1`, `failures == 0`, the second argv carries `--resume`, and the task merges.
- `e2e_a_second_death_in_a_round_is_a_stall`: the same, but the resumed script exits mid-turn again. Assert `stalls == 1`, a fresh session 2 (`worker-t1-2`) finishes, and the task merges.
- `e2e_headless_windows_refuse_client_control_while_the_engine_delivers`: the worker calls `task_blocked {"kind":"question","reason":"which file?"}`, then `read_message {"expect":"Answer to your question: a.txt"}`, then commits and `DONE`s. While the task is `blocked(question)`, the test sends `Subscribe`, `Input`, `Kill`, `Remove` and `Restart` for the worker window over a raw client. Each gets decision 49's exact error, and the window is still listed with its process alive. Then `run edit` with an `answer` edit: the worker receives it as a turn, and the task merges.
- `e2e_permission_denials_block_the_task_as_environment`: `denials_before_block = 2`. The worker runs `deny {"tool":"Write","reason":"not allowed"}` twice, then `hang`s. Assert `t1` is `blocked(environment)` with `the agent was denied 2 times; last: Write: not allowed`, the attention line names it, and the worker window is `Exited`.

In `scripts/pty_smoke_run.py`, function `run_engine_stage(run_cmd, fail)`, taking `pty-smoke.py`'s own `run_cmd` and `fail` the way `scripts/pty_tree_smoke.py`'s stages do (so it inherits the script's isolated `ENV`: `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under `/tmp`, fake-agent as both runtimes, `ANTHREX_GIT=off`), called from `pty-smoke.py` after stage 11b and before `== stage 12: stop the daemon, verify status ==`, printing `== stage 11c: a one-task run is approved, merged and accepted ==`: create `/tmp/anthrex-smoke-run-<pid>` (a repo with identity and one commit), write the worker and reviewer scripts of `e2e_green_s_task_runs_to_merged` and the plan to `/tmp/anthrex-smoke-plan-<pid>.toml`; `anthrex run start --plan … --dir <repo>`; `anthrex run approve <id>`; poll `anthrex run status <id> --json` every 0.5 s up to `RUN_WAIT` (a named Python constant, 300, with the Rust derivation in its comment, as the script's other cross-language bounds are) until `complete`; `anthrex run accept <id> --yes`. Every `run_cmd` of a run command passes `timeout=RUN_CMD_TIMEOUT` (240 s): `run_cmd`'s default of 28 s is below `run start`'s legal worst case of `ENSURE_DAEMON_SOCKET_WAIT` + `SPAWN_HANDOFF_GRACE` (3.25 s) + `HANDSHAKE_TIMEOUT` (5 s) + `RUN_REQUEST_TIMEOUT` (180 s) = 188.25 s, the language-boundary shape `docs/timing-budgets.md` records six times; add the row there; assert `git log -1 --format=%s` starts with `anthrex: accept run` and `a.txt` exists; remove both paths in `finally`.

**Change.** Only tests, scripts and the fixes they uncover.

**Acceptance.** All five commands from AGENTS.md pass.

**Commit.** `test: cover the ladder, merge queue, salvage, ref guard, headless session and crash-reconcile scenarios end to end, and add a run smoke stage`

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
- `rg -n "std::fs|std::process|std::thread|tokio|SystemTime" crates/daemon/src/run/engine crates/daemon/src/run/{plan,validate,globs,roster,edits,model,contract,messages,report,role_launch,snapshot,env}.rs crates/daemon/src/headless/{mod,argv,claude_stream,codex_stream,conversation,status}.rs` prints nothing.
- `rg -n "write_input" crates/daemon/src/run crates/daemon/src/headless` prints nothing: no run agent is driven through a terminal.
- Every `.meta.json` under `crates/daemon/tests/fixtures/headless/` names the CLI version it was recorded with, and the shape-conformance tests of M8a.20 pass against those files.
- `cargo tree -p anthrex-daemon | grep rmcp` and `cargo tree -p anthrex-tui | grep rmcp` print nothing; `cargo tree -p anthrex-mcp | grep "rmcp v3.4.0"` matches.
- `wc -l` on every file in the file-size table and every file under `crates/daemon/src/run/`, `crates/mcp/src/`, `crates/proto/src/run*.rs`: nothing new above 600, nothing existing grown beyond its row.
- Every `Command::new` in `crates/daemon/src` that spawns git goes through `worktree::run_git` or matches `no-optional-locks`.
- No `crate::lock` guard in `run/driver*.rs` or `manager/*.rs` is alive across `.await`, `spawn_blocking`, a git call, `Window::spawn` or `HeadlessHandle::spawn`/`send_line`/`kill`.
- A run starts, completes and is accepted with `ANTHREX_GIT=off`.
- After the tests and the smoke script, `pgrep -fl "anthrex daemon"` and `pgrep -fl fake-agent` show nothing of yours (a headless `fake-agent` left behind means a session was not killed at shutdown), and `/tmp/ax-run*`, `/tmp/anthrex-smoke-run-*` and `/tmp/anthrex-smoke-plan-*` are gone.

## Manual check

Use an isolated daemon, config and throwaway repository throughout:

```bash
export ANTHREX_SOCKET=/tmp/anthrex-m8a/daemon.sock ANTHREX_DATA_DIR=/tmp/anthrex-m8a/data ANTHREX_CONFIG=/tmp/anthrex-m8a/config.toml
mkdir -p /tmp/anthrex-m8a && cd /tmp/anthrex-m8a && git init -b main demo && cd demo \
  && printf '#!/bin/sh\ngrep -q hello hello.sh 2>/dev/null && echo "PASS $1"\n' > t.sh && git add . && git commit -m init
```

1. Write `/tmp/anthrex-m8a/plan.toml`:
   - profile `check = "sh -n hello.sh"`, `single_test = "sh t.sh {test}"`, `test_passed = "PASS {test}"`;
   - `t1`: a Codex M tdd task owning `hello.sh`;
   - `t2`: a Claude S docs task owning `README.md`, depending on `t1`.

   Leave `ANTHROPIC_API_KEY` unset in the shell that starts the daemon.
2. `anthrex run start --plan /tmp/anthrex-m8a/plan.toml`:
   - the id and the plan table print, and the state is `awaiting_approval`;
   - `git worktree list` in `demo` shows the integration worktree and `t1`'s pre-warmed worktree;
   - no run window exists.

   Then `anthrex run approve <id>`.
3. `anthrex` to attach. `t1`'s worker is listed as a window. Focusing it shows the headless placeholder, and typing does nothing. `C-b m` opens its conversation, which shows the prompt, each command and the `task_done` call live. `ps -o command= -p <pid>` shows `codex exec --json … -s workspace-write -c sandbox_workspace_write.writable_roots=…`. The worker's commit succeeds inside the sandbox, which confirms decision 25's writable root.
4. Watch `anthrex run status` go `working → proof → check → review → merge_queue → merged` for `t1`, with the red commit named in the status JSON. The reviewer is a Claude session:
   - `ps` shows `claude -p --input-format stream-json … --permission-mode plan`, with no `--bare`;
   - it answers without an API key, which confirms the login is used (decision 50);
   - its `submit_review` call is **not** blocked by plan mode. If it is, apply decision 24's fallback and record it;
   - its conversation shows at least one allowed `git diff`, `git log` or `git show` call succeeding (ask for it in a scratch review if the reviewer made none), and a `cargo build` it is asked to run is denied.
4a. **Project settings.** Add a tracked `.claude/settings.json` with a `PreToolUse` hook that runs `touch /tmp/anthrex-m8a/project-hook-ran`, and a tracked `.mcp.json`, and commit them. Start a second run with a Claude task:
    - with `claude_user_settings_only` set, the run starts, `/tmp/anthrex-m8a/project-hook-ran` never appears, and the conversation shows anthrex's hooks at work (tool calls listed);
    - otherwise `run start` is refused with decision 53's message; `--trust-project` starts it, and the report names both files.

    Discard the run and remove the two files.
4b. **Worker sandbox.** Give `t2`'s brief a first step: "run `touch ../outside-sandbox` with Bash, then continue". The conversation shows the command refused. `ls /tmp/anthrex-m8a/` in the worktrees' parent shows no `outside-sandbox`. `t2` still commits `README.md` and merges. Record whether the refusal appeared as a stream denial and was counted (the snapshot's `denials`).
4c. **Protected files.** In a third run, give a task `owns = ["**"]` in a repo that tracks `AGENTS.md`, and a brief that asks it to append a line to `AGENTS.md`. Then:
    - `run start` prints the protected warning;
    - the worker's `task_done` is rejected with `AGENTS.md configures or instructs future agents…`, and after it reverts, the task merges;
    - `AGENTS.md` on the run branch is unchanged.
4d. **Codex project config.** If M8a.1 found that Codex loads project config, commit a `.codex/config.toml` and start a run with a Codex task. It is either excluded (the argv carries `codex_user_config_only`) or refused without `--trust-project`, as decision 53 says. Remove the file afterwards.
5. In the TUI, typing into a focused worker window does nothing, and the kill and remove commands on it show decision 49's refusal. The run carries on.
6. While `t2`'s worker runs, `anthrex daemon stop`:
   - `pgrep -fl "claude -p"` shows nothing of this run.
   - `anthrex daemon start`: `run status` shows `paused (from running)`.
   - `anthrex run resume <id>`: `ps` shows `claude -p … --resume <session id>` with `--settings` and `--mcp-config` re-passed, and the conversation view shows `RESUME_WORKER` as a new user turn.
7. While the run works, commit a one-line change to an unrelated file on `main` in the repository (`git commit` in `root`, as a user would). The run does not halt; `run status` shows the `base main moved` attention line. When the run is `complete`, read `REPORT.md`: gates, rounds, severities, done signals, tokens per round, and the base-moved line. `anthrex run accept <id>`: it lists your commit with your name and asks `merge onto main at <sha7> including these commits?`; answer `y`. `git log --oneline -3` on `main` shows the accept merge above your commit, and `git branch --list 'anthrex/*'` is empty.
8. Start a second run and let a worker write an uncommitted file. `anthrex run cancel <id>`: the session's process is gone, and `git for-each-ref refs/anthrex/salvage` lists the salvage ref. Then `anthrex run discard <id>` and type the id.
9. `anthrex daemon stop`. `pgrep -fl "anthrex daemon"`, `pgrep -fl "claude -p"` and `pgrep -fl "codex exec"` show nothing of yours. Remove `/tmp/anthrex-m8a`.

## Review focus

The five input classes or failure modes most likely to bite a user that the task tests above would not exercise without being told to. Each is given a test in its owning task; the reviewer checks those tests exist and fail without the fix.

1. **A real repository's hooks and signing.** A user with `commit.gpgsign = true` and a pinentry, or a `post-checkout` hook that runs a build, would hang or fail every engine checkout and merge candidate. Test: `engine_writes_ignore_hooks_and_signing` (M8a.8), plus `accept` deliberately not overriding them (`accept_merges_no_ff` runs with the repo's own hook present and asserts it ran).
2. **Hostile or odd test names and paths.** A worker controls the `test` string that is substituted into a shell command, and repository paths can contain spaces and non-ASCII. Tests: `proof_quotes_a_hostile_test_name` (M8a.10), and `engine_paths_with_spaces_and_unicode_work` (M8a.8: a `TempRepo` under `/tmp/ax run ü/` runs preflight, worktree creation, merge candidate and salvage).
3. **Forgotten `git add`.** A new file left untracked passes the check in the task worktree, where the file exists, and fails only on the merged candidate. That spends a bounce on a mistake the engine can name. Tests: `verify_done_reports_each_condition`'s untracked-inside-owns case (M8a.8), and `task_done_rejections_leave_the_task_working` (M8a.12).
4. **`owns` written as a plain directory.** Planners write `crates/auth` or `crates/auth/` as often as `crates/auth/**`. A literal globset match would flag every file under it as a spill and send correct work to rung 3. Test: `owns_without_wildcard_covers_the_directory_below_it` (M8a.4).
5. **A sandboxed worker in a linked worktree.** A linked worktree's index, refs and objects live under the main repository's `.git`, outside the worktree that both sandboxes make writable: Codex's `workspace-write`, and Claude Code's sandbox (decision 54). Without the git common dir as a writable root, every worker's `git commit` fails inside its sandbox. The task stalls or loops through rung 2, and no test with `fake-agent` would notice, because `fake-agent` has no sandbox. Tests: `codex_worker_args_add_the_git_common_dir_as_writable` and `worker_settings_json_enables_the_sandbox` (M8a.7), M8a.1 items 4b and 7's recorded proofs, and manual check steps 3 and 4b.

## Risks and gotchas

1. **Conflicts are nearly unreachable, by design.** Decision 41 never runs two tasks whose `owns` intersect, and decision 32 stops a task that changes a path outside its `owns`. So between legal tasks a merge conflict cannot happen; §11.5's hand-back fires only after a `run override` accepts a spill, which is how M8a.25 builds its two conflict tests. If a conflict ever appears without an override, one of those two rules has a hole. Check `implicit_deps` and the spill diff (decision 32 diffs `<run_head>...HEAD`, three dots) before suspecting git.
2. **The spill check versus generated files.** A build that regenerates a tracked lock file outside the task's `owns` would be a spill and go to rung 3. Decision 55 turns it into a rung-1 bounce instead, but only for files listed in `profile.generated`. A plan that leaves `generated` empty still sends a stray `Cargo.lock` to rung 3. The planner (and from M8b the onboarding scout) must list the repository's lock files, and a task that really changes dependencies must own the lock file. A bounce spent on a lock file still counts toward `max_bounces` and rung 2, so a worker that keeps regenerating it escalates.
3. **Plan mode and MCP in `-p`.** If Claude's plan mode refuses the allowed `submit_review` call, every Claude review stalls. Decision 24 names the fallback, and manual check step 4 decides.
4. **The stream-json input envelope is thinly documented (§23).** A wrong envelope makes Claude ignore or reject every message after the first. `claude_user_message_matches_the_recorded_envelope` and `fake-agent`'s refusal of any other envelope (M8a.20) are the guard. Re-record the fixture when the CLI's major version changes.
5. **`--bare` may become the default for `-p` (§23).** On that day, `auth = "login"` sessions fail to authenticate unless M8a.1 found an explicit opt-out flag. The failure is a turn that fails with `authentication_failed`, which decision 32 turns into `blocked(environment)` with the CLI's text, so it is loud, not silent.
6. **Project settings run unprompted in `-p`.** Without `--bare`, `claude -p` would run the repository's own `.claude/settings.json` hooks and `.mcp.json` servers with no trust dialog. Decision 53 excludes them with the CLI's own flags, or refuses the run without `--trust-project`.
   - **What remains.** The fallback reads only the base commit. A task that adds a `.claude/settings.json` or `.mcp.json` during the run would have it loaded by a later resume of a session in that worktree. The spill check stops this unless the task owns those paths.
   - A plugin or a user-level setting still loads. That is the user's own configuration, by design.
7. **Workers are unattended with `Bash` allowed.** `--permission-prompts none` denies anything not allowed, so the default `worker_allowed_tools` must include `Bash`, or no worker can run a test. Decision 54 confines that `Bash` to the worktree and the git common dir through Claude Code's sandbox, just as Codex's `workspace-write` does.
   - **What remains.** Reads are not confined, only writes and network. With `[orchestrator] worker_sandbox = false`, for a platform where the sandbox cannot start, a worker's shell is unconfined again. The report says so on every such run.
   - Whether a sandbox refusal is counted as a denial depends on how it appears in the stream (M8a.1 item 4b). If it is not counted, a worker that keeps retrying is caught only by the stall and budget rules.
8. **Codex sandbox network.** `workspace-write` blocks network access by default. A worker whose test needs a download fails, which is why `setup` (run by the engine, unsandboxed) is where fetching belongs.
9. **Background sub-agents keep `-p` open**, up to its 10-minute idle ceiling. `open_subagents` defers the turn-end fallback (decision 32), and the stall clock keeps running on stream events, which sub-agents produce.
10. **`SIGTERM` leaves a Claude turn unfinished**, and that is what shutdown sends. A resumed session may re-run the last tool call. Workers commit often and the gates re-check everything, so the cost is time, not correctness.
11. **Usage semantics.** If Claude's `result.usage` is cumulative and the parser treats it as per turn (or the reverse), token spend is wrong by a large factor. `claude_usage_is_per_turn` pins it against the fixture.
12. **Shutdown storm.** Without decision 46, killing sessions at shutdown would read as process deaths of working tasks. `e2e_daemon_restart_pauses_and_resume_continues` catches it.
13. **fsync cost.** `Persist` and intent lines `fsync` on every structural step. On a slow disk this adds tens of milliseconds per step, which is fine at the rate tasks move, but counter-only changes are never `fsync`ed more than once every 5 s.
14. **Watcher descriptors.** Each registered run worktree costs a recursive watcher (refreshed M8 risk 8), and large runs on Linux may degrade to polling.
15. **Socket path length.** Test temp dirs go under `/tmp`, never `std::env::temp_dir()`.
16. **`fake-agent` not built.** `cargo test -p anthrex` alone does not build it; use `--workspace`.
17. **Stdout in `anthrex mcp`.** Any non-JSON-RPC byte on stdout breaks the session: no `println!`, and no tracing subscriber on stdout.
18. **Protected paths and legitimate work.** A task that must edit `CLAUDE.md` or `.claude/` config must name each file exactly in `owns`. A planner that writes `docs/**` for a docs task that also touches `docs/AGENTS.md` gets a bounce the first time. The warning at `run start` (decision 56) shows this before approval, but only for files that already exist.
19. **Codex project config is unverified until M8a.1.** If Codex loads project config and the recording misses the mechanism (for example, trust levels that only apply in the interactive TUI), decision 53's Codex branch would be wrong in a way no `fake-agent` test can see. Manual check step 4d is the backstop.
20. **rmcp API drift.** Do not loosen `=3.4.0`. When a name differs, read the crate source under `~/.cargo/registry/src/*/rmcp-3.4.0/`.
21. **A busy base branch.** Committing to the base branch during a run no longer halts it (decision 21); the run builds on its recorded `base_sha` and the new base commits are listed at accept for confirmation. Two consequences: the run's check never ran against those commits, so the accept merge is the first time run and base meet — a semantic clash that merges cleanly is not caught by any gate (the list at accept is the user's chance to see it); and a textual clash surfaces only at accept, as an aborted merge the user resolves by hand. A rewritten or deleted base still halts.

## Follow-ups handled

- **From milestone 6.5's transcript capture (`docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`, "From milestone 6.5's transcript capture"): "Agents inherit the launcher's entire environment, including another agent's session markers."** Handled for headless run sessions and engine commands by decision 26 (`CLAUDE_CODE_*` and `CLAUDECODE` removed, profile env set). Not handled for PTY windows, which keep today's inheritance; update the follow-up entry to say so and leave it open for the general policy.

New follow-ups to record in `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` during implementation: move `crates/config/src/lib.rs`'s remaining top-level readers (`prefix`, `accent`, `bell`, `ui`, `panes`, `runtimes`) out of it, since M6.5 split off only `conversation` and `git` and this milestone leaves it just over 600 lines; split `crates/daemon/src/server.rs` (605 lines at `6f22681`) further if `server/git_wiring.rs` does not bring it under 600; prune old run directories under `<data_dir>/runs/`; the environment policy for PTY windows; the PTY status machine's `StopFailure`/compaction gap, now only the orchestrator's (M9); re-recording the headless fixtures on each CLI major version.

## Implementation notes

### Brief refresh against main (6f22681)

Refreshed 2026-09-23, before any M8a code, against `main` at `6f22681` (PR #13, milestones 1 to 6.5 merged). Each bullet gives the old text, the new text, and the evidence as `file:line` on that commit. Paths are relative to the repository root.

**Status and protocol**

- Status `blocked` → `ready`: M6.5 merged in PR #13 as `6f22681` (`git log -1 origin/main`); `docs/ROADMAP.md` row 8a is `ready`, and its stale "milestones 1 to 6 are merged" and "protocol version on `main` is 5" lines now say 1 to 6.5 and 6.
- Protocol version "one above main when M8a starts (7 if M6.5 has merged)" → **7**. Derivation: `pub const PROTO_VERSION: u32 = 6;` at `crates/proto/src/lib.rs:19`, set by M6.5; 6 + 1 = 7. The test to update is `proto_version_is_six` (`crates/proto/src/lib.rs:48`), renamed `proto_version_is_seven`.

**Names taken from milestones 3 to 6**

- `script::Step { …, McpCall }` → `{ …, McpCall, Transcript }`, and `FAKE_AGENT_TRANSCRIPT` exists: `crates/fake-agent/src/script.rs:29`, `crates/fake-agent/src/main.rs:148`.
- The manager split `manager/{mod,config,create,entry,remove,restart,restore}.rs` → adds `conversation.rs`: `crates/daemon/src/manager/mod.rs:3–9`.
- `project::detect_roots(dir)` in decision 16 → `project::detect_roots_with(git, dir, timeout)`, because every `run/git` function takes an injected `git`: `crates/daemon/src/project.rs:49,54`.
- `Config { …, git }` → `{ …, git, conversation }`: `crates/config/src/lib.rs:37`. The silent `orchestrator` skip is `lib.rs:551` (comment `lib.rs:138`), and `every_key_is_read` depends on it through `[orchestrator] max_parallel = 3` at `crates/config/src/lib_tests.rs:180`; the brief's "the test that `orchestrator` is silently skipped" is that block, not a separate test.
- "`crates/config/src/lib.rs` grows to 1033 lines … record a follow-up to split it" → M6.5 already split it (`lib.rs` 600, `conversation.rs` 318, `git.rs` 154; `crates/config/src/lib.rs:9–10`); M8a.3's budget is now at most 8 lines, and the follow-up asks for the rest of the split.
- "Two signatures this milestone changes" listed `WindowManager::create`, which the file table says is untouched → "the signature this milestone changes (`server::serve`, `crates/daemon/src/server.rs:40`) and the one it leaves alone (`create`, `crates/daemon/src/manager/create.rs:111`)".
- `Entry`'s routing list gains `size`, which also matches on `Process`: `crates/daemon/src/manager/entry.rs:194`.
- Decision 49 "`Resize` is answered `Ack`" → no reply: the `ClientMsg::Resize` arm replies only on error (`crates/daemon/src/server.rs:395–402`, `.err().map(…)`).
- Decision 26's scrubbed names and the claim that `run_git` scrubs `GIT_DIR`…`GIT_PREFIX` hold: `crates/daemon/src/subprocess.rs:375–381`, `crates/daemon/src/worktree.rs:230–251`.

**Names taken from milestone 6.5**

- M6.5 row "(branch, `8230d36`) … the manager wiring is not on the branch … use the merged names" → the merged names, with locations: `ConversationSet::on_hook(runtime, &ParsedHook, spawn: Option<&SpawnOrigin>, now_unix_secs, now: Instant, caps)` (`crates/daemon/src/conversation/store.rs:69`), `enrich(&[Record], Caps) -> Vec<Option<String>>` (`store.rs:314`), and the manager's methods in `crates/daemon/src/manager/conversation.rs` (`subscribe_conversation` `:119`, `conversation_snapshot` `:187`, `conversation_delta` `:203`, `conversation_reader_running` `:217`, `conversation_hook` `:355`).
- "`DaemonMsg::{ConversationSnapshot, …}`, ignored by the TUI in one match arm in `crates/tui/src/app/mod.rs`" → handled, not ignored, in `App::on_daemon`, which M6.5 moved to `crates/tui/src/app/daemon.rs` (`:9`, arms at `:106–134`; no `_` arm). M8a.2's "ignore arm" becomes a new `DaemonMsg::Run(_) => vec![]` arm there.
- "`ParsedHook` gains six fields" → seven: `session_source` too (`crates/daemon/src/hooks.rs:54`). Synthesised hooks set it to `None`.
- `transcript::Record::UserText { session_id, ordinal, text }` → `{ session_id: Option<String>, ordinal: u32, text, human: bool }` (`crates/daemon/src/transcript/mod.rs:25–31`). The mapping table's `UserText { ordinal: n }` / `AssistantText { ordinal: n - 1 }` (where `n` counted turns sent) was off by one against M6.5 ruling R1 (0-based prompt index, recorded in `docs/milestones/M6.5-conversation-view.md`, "Task M6.5.7, the `TranscriptParser` interface"): it is now `UserText { ordinal: k, human: true }` and `AssistantText { ordinal: k }`, `k` the 0-based index of the latest sent turn.
- Proto root re-exports: not only `Role` and `ToolResult` but `Block, Conversation, DegradeReason, DropCause, NoticeKind, Role, ToolResult, ToolState, Turn, TurnPatch, TurnState` (`crates/proto/src/lib.rs:35–38`). No new M8a proto name collides with them (checked name by name); `lib.rs` re-exports the new modules by name, not by glob.
- `messages.rs` "500 lines after M6.5" → 506 (`wc -l crates/proto/src/messages.rs`).
- Fixture metadata "CLI version, date, exact command" → M6.5's keys `runtime`, `cli_version`, `captured`, `redactions`, `note` (`crates/daemon/tests/fixtures/transcripts/claude-2.1.278.meta.json`), plus `command` and `observed`.
- "A headless window never starts M6.5's transcript reader" named no code: the reader is started by `subscribe_conversation` for any runtime with a parser, and the same call sets `NoTranscriptPath` when the root has no path (`crates/daemon/src/manager/conversation.rs:136–150`). Decision 27, the manager Interfaces note and M8a.17 now name the skip and test it (`a_headless_window_starts_no_transcript_reader`).
- Hooks for a headless window are applied through `WindowManager::conversation_hook` (`manager/conversation.rs:355`), which already resolves the spawn origin from `entry.state.subagents` and notifies subscribers; decision 27 now says so instead of calling `on_hook` directly.
- "A synthesised `tool_response` is bounded exactly as `anthrex hook` bounds a real one" named a function the daemon cannot call: `TOOL_RESULT_SUMMARY_MAX`, `bound_tool_response` and `bound_tool_response_value` are private to the CLI binary (`crates/cli/src/hook.rs:23,111,141`; `anthrex` has no library target). M8a.7 moves them unchanged into `proto::conversation` (Ruling Q5 below).
- `WindowManager::tick` turns a quiet `Working` window `Idle` (`crates/daemon/src/manager/mod.rs:314–318`), which would override a headless window's stream-driven status; decision 27 and M8a.17 add the skip and a test.
- TUI: "`app/link.rs` never sends `Subscribe`" → three sites: `App::focus` (`crates/tui/src/app/mod.rs:344`), `retry_dropped_subscribe` and `on_reconnected` (`crates/tui/src/app/link.rs:101,207`). The two `Input` sites (`app/mod.rs:362,458`) and the mouse one (`crates/tui/src/mouse.rs:89`) were right. "The pane renderer under `ui/`" → `crates/tui/src/ui/terminal.rs`. The TUI tests go in `crates/tui/src/app_tests/headless.rs` (the existing `#[path]` pattern, `crates/tui/src/app/tests.rs:5–8`), because `app/tests.rs` is 528 lines.
- M8a.2 "every `WindowInfo { … }` construction (38 sites)" → 39 sites in 22 files (`rg -n "WindowInfo \{" crates`). "`crates/cli/src/client.rs` only if a match on `DaemonMsg` is exhaustive there" → no change: `request_with_timeout` has no exhaustive match (`crates/cli/src/client.rs:182–205`). M8a.2 also has to touch `crates/daemon/src/server.rs`, whose `match msg` is exhaustive, for the temporary refusal; it was missing from the Files list.

**Landed conventions the brief contradicted**

- "under the crate's shared environment lock" (M8a.8, M8a.10, M8a.17) → each such test alone in its own test binary. The daemon crate has no shared environment lock (no `static` env mutex in `crates/daemon`), and the landed tests explain why a lock cannot work: `crates/daemon/tests/worktree_env.rs:1–20` and `git_env.rs:1–16` (a `set_var` races any spawn on another libtest thread). M8a.8's missing-identity case now uses a wrapper `git` script instead of `GIT_CONFIG_GLOBAL` in the test process. AGENTS.md's "Tests that change environment variables must hold a shared lock" says otherwise (Ruling Q11 below).
- `scripts/pty_smoke_run.py`'s `run_stage(env, bin_path)` → `run_engine_stage(run_cmd, fail)`, the injection pattern of `scripts/pty_tree_smoke.py:90,225,337,444`, called as at `scripts/pty-smoke.py:1447–1450`. The module name is kept (M9's brief cites it).
- The smoke stage's run commands through `run_cmd` would time out at its 28 s default (`scripts/pty-smoke.py:452`) against `run start`'s 188.25 s worst case; they now pass `RUN_CMD_TIMEOUT` (240 s).
- File-size table recounted on `6f22681`: `server.rs` 556 (565) → 605, already over, so its budget changed from "35 lines" to "net zero or less" with `GitWiring` and `pump_git` moved to `server/git_wiring.rs`; `config/src/lib.rs` 730 (1033) → 600; `config/src/lib_tests.rs` 741 (new row, over); `manager/mod.rs` 478 → 489; `manager/conversation.rs` 445 (new row); `entry.rs` 331 → 340; `restore.rs` 509 → 512; `create.rs` 559 → 563; `app/mod.rs` 586 (592) → 515 and its ignore arm moved to `app/daemon.rs` (141); `app/link.rs` 262 → 268; `cli/src/main.rs` 527 → 582, so its budget fell from 25 lines to 15; `messages.rs` 400 (500) → 506; `lifecycle.rs` 419 → 420; `hooks.rs` 313 (391) → 399; `fake-agent/src/main.rs` 319 (342) → 342, `script.rs` 190 → 213, `tests/script.rs` 681 (new row, over); `pty-smoke.py` 1572 → 1770.

**Things the brief required that cannot compile or cannot be reached**

- `impl TokenUsage { fn billable }` in `crates/daemon/src/headless/mod.rs`: `TokenUsage` is a proto type, and an inherent impl on a foreign type does not compile. Moved to `crates/proto/src/run_info.rs`, tested in M8a.2.
- M8a.3's `profile_protected_adds_to_the_builtins` expected the config crate to add `BUILTIN_PROTECTED`, which is a daemon constant; `crates/config` depends on `proto`, `toml` and `unicode-width` only (`crates/config/Cargo.toml:12–15`). The config reads the user's additions; `resolve_profile` (M8a.5) adds the built-ins, as decision 56 already implied. Test renamed `profile_protected_is_read_as_additions`.
- `roster::pick_reviewer(…, level: ReviewLevel)` is written in M8a.4, but `ReviewLevel` lived in `run/model.rs`, created in M8a.5. M8a.4 now creates `run/model.rs` with `ReviewLevel`.
- M8a.7's `toml_string_round_trips_through_the_toml_crate` covered "both contracts", which M8a.11 creates. That half moved to M8a.11.
- `run/messages.rs` (decision 29's `clamp`, `join_turn`, `DELIVERY_RETRY_SECS`) was in the purity list but in no task's Files. M8a.11 creates it, with tests.
- M8a.7's `garbage_never_panics` uses `proptest`, which is not a workspace dependency (`Cargo.toml`); M8a.7 adds it.
- The Codex `turn.failed` fixture and Claude's documented-only shapes had no file: the failed run is part of `codex-<version>-exec.jsonl`, the documented shapes get `claude-<version>-documented.jsonl` (nine fixtures, not eight).
- `codex_writable_roots == [<root>/.git]` is wrong whenever `root` is a linked worktree (its `.git` is a file; the common dir is the main repository's). `Preflight` and `Run` gain `git_common_dir` (decision 16), and the fixture's `root`, `project` and `git_common_dir` are three different paths.
- `send_line`'s queue bound (256) appeared only in a test; it is now `WRITER_QUEUE_MAX` in the Interfaces.
- The refreshed M8 brief lives on branch `docs/m8-brief-refresh`; the test vectors this brief cited from it (intersection cases, slug examples, `format_utc` vectors, the 20 TOML strings) and its accept preconditions are restated inline so the brief stands alone on `main`.

**Name collisions checked** (none needed a rename)

- New proto names against every root re-export of `crates/proto/src/lib.rs:34–43`: no collision. `RunReply::ToolResult` and `SessionEvent::ToolResult` are variants, not types, so they do not clash with `proto::ToolResult`.
- `proto::run_wire::request` against the existing `proto::messages::request` (`crates/proto/src/messages.rs:191`): kept apart by never re-exporting either at the root.
- `pub mod run;` in the daemon against `pub use lifecycle::{…, run}` (`crates/daemon/src/lib.rs:43`, called as `daemon::run` at `crates/cli/src/main.rs:379`): a module and a function occupy different namespaces, so both stay.
- `proto::Severity` against `crate::state::Severity` (`crates/daemon/src/state/mod.rs:171`), and `proto::Plan` against `crate::git::schedule::Plan` (`crates/daemon/src/git/schedule.rs:40`): run code spells the proto ones qualified or imports them under their proto path; neither daemon type is used by run code.
- The new `manager/headless.rs` module is named `headless` inside `manager`, so inside the manager an unqualified `headless::…` resolves to it, not to `crate::headless`: code there spells `crate::headless::HeadlessHandle`.
- `headless::status::next` against `crate::status::next`, and `SessionEvent::{UserText, AssistantText}` against `transcript::Record::{UserText, AssistantText}`: deliberate parallels, always path-qualified in `headless/conversation.rs`.

**Fixture and timing defects fixed** (the recurring kinds)

- Symmetric fixtures and fixtures equal to defaults: decision 7's example had `max_writers = max_readers = 3` and every limit, route field and budget equal to the default it overrides, so `the_brief_example_builds_a_run` could not show the plan's values win; now 4/2/3, effort `high`, budget 120/45. The engine fixture had `root == project`; now `/tmp/x`, `/tmp/p`, `/tmp/p/.git`. `guard_refs_classifies_each_case` and `base_advanced_is_recorded_and_the_run_goes_on` ran with `run_head == base_sha`, so a guard or reducer that read one where it meant the other passed; both now advance the run branch first. The rate-limit e2e test had `stall_after_secs = rate_limit_retry_secs = 5`; retry is now 7. Two-round review tests gave both rounds findings of unstated, possibly equal, text; now distinct. Usage fixtures now have four distinct fields with a non-zero `cache_read`.
- A repeated unit whose size divides the bound: `budget_soft_then_hard` used a budget of 4, where 1.5 × 4 is whole and floor, ceiling and float comparisons agree; now 5 (breach at the 8th, not the 7th), and the token case uses 1001. The UTF-8 cuts at 4096 bytes (tool results), 32 KiB (message clamp) and 300 characters (check tail, `Unknown`) now have tests with the 3-byte `世`, the fixture M6.5.2 had to adopt after a 2-byte one left its back-off loop dead.
- "At most N" assertions on a truncation: the check tail's "cut to 300 characters" is now "exactly 300 characters"; the message clamp is asserted from both sides.
- Tests that assert nothing: `agent_role_does_not_shadow_the_conversation_role` (compile-only) and `a_client_cannot_create_a_headless_window` ("has no field") now assert behaviour; `e2e_claude_sessions_load_only_user_settings` asserted nothing when `CLI_CAPS` had no flags and now asserts both branches.
- Sleep-then-assert and racing waits: `e2e_moved_run_ref_halts_the_run` relied on "a scripted slow check" (a race, and a blocked check would hit the harness's 10 s `check_timeout_secs`); it now holds `t2`'s reviewer on a release file. `e2e_base_advanced_…` and `e2e_base_rewritten_…` committed "while `t1` works" and now hold the worker on a file. `e2e_cancel_of_a_dirty_running_task_is_salvaged` cancels only once `a/wip.txt` exists. `interrupt_by_sigint_reaches_the_child` waits for `ready` before signalling. `send_line_never_blocks_on_a_full_pipe` had a per-call 50 ms wall-clock bound drawn from observed cost; now structural. `writes_to_two_repos_run_concurrently` used overlapping timestamps; now a rendezvous.
- Bounds below the legal worst case: `2 * RUN_WAIT` for `e2e_second_conflict_…` and `e2e_mis_sized_…`, which run three task paths in sequence → `k * RUN_WAIT` with `k` per test, plus named engine timers; the second-conflict script's 120 s wait for `t3` → 300 s; the smoke stage's `run_cmd` default → `RUN_CMD_TIMEOUT`.

**Rulings on the refresh's open questions** (coordinator, 2026-09-23)

- Ruling Q1: salvage refs stay `refs/anthrex/salvage/<run>/<task>/<seq>` (decision 20), a deliberate refinement of spec §12.2 and §17's `…/<run>/<task>`, why: a second salvage of the same task would otherwise either hit a ref directory/file conflict or overwrite the first salvage. **Spec amendment proposed to the user.**
- Ruling Q2: the small-review switch follows the spec's form, `review.small = "off"` (spec §9, §22.2), why: the spec is binding, and M9.5's brief will be refreshed against landed code anyway. The spec names no table; the key sits in `[orchestrator]` as the TOML dotted key `review.small` (equivalently `[orchestrator.review] small`), with string values `"on"` (default) and `"off"`. Changed: decision 35's level rule, the `Orchestrator` Interfaces comment and parsing rules, the `Task.review_level` comment, M8a.3's new test `review_small_is_read_as_on_or_off`, the test names `review_small_off_skips_s_but_not_hub` (M8a.5) and `review_small_off_skips_s_review` (M8a.12), and the conflict tests' common setup. The Rust field keeps the name `review_small: bool` (true = `"on"`); only the config form changed.
- Ruling Q3: decision 45 stands — `halted`, `awaiting_approval` and `complete` restore as they were — a refinement of spec §17's "non-terminal runs come back Paused", why: `complete` is not unfinished, so the spec's rule does not cover it, and restoring `awaiting_approval` or `halted` as `Paused` would lose the plan gate that spec §12.3 requires to survive a restart. **Spec amendment proposed to the user.**
- Ruling Q4: the reviewer gets the diff in its prompt and can only read git, why: as written a Claude reviewer (no `Bash`, no one to answer a prompt) could run neither `git diff` nor tests, contradicting `REVIEWER_CONTRACT`, and the spec gives the reviewer "the diff and the check output". Changed: `PrepareReview` returns the patch (`OpResult::Review { patch }`, `prepare_review`'s third value), clamped to the new `REVIEW_DIFF_MAX` = 16 KiB; `reviewer_prompt` gains a `Diff (<base7>..<head7>):` block (decision 35, Interfaces prompt text, M8a.9 and M8a.12 tests); the Claude reviewer's `--allowedTools` adds `Bash(git diff:*)`, `Bash(git log:*)`, `Bash(git show:*)` (decision 24, M8a.12's `review_round_uses_a_fresh_session_and_worktree`), and the plan-mode fallback no longer disallows `Bash`, which would override those patterns; contract item 2 points at the prompt's diff and item 3 is now "Read the diff; do not run the build." The Codex reviewer path was checked: under `-s read-only` with `approval_policy="never"` it can still run `git diff`, `git log` and `git show`, and any build fails on its first write, so item 3 holds for both runtimes (decision 25). Manual check step 4 adds the allowed-git and denied-build observation.
- Ruling Q5: yes — `TOOL_RESULT_SUMMARY_MAX`, `bound_tool_response` and `bound_tool_response_value` move unchanged into `proto::conversation` in M8a.7 and `anthrex hook` calls them from there, why: the daemon must bound a synthesised `tool_response` exactly as the CLI bounds a real one, and it cannot call into the `anthrex` binary. No protocol change.
- Ruling Q6: yes — `create_headless` and `headless_resume` await `ManagerConfig.launch_gate` before spawning (decision 27's manager side; M8a.17's new `create_headless_waits_for_the_launch_gate`), why: the gate exists so that no agent process starts before the startup Codex version probe finishes (`crates/daemon/src/launch/gate.rs:1–13`), and a headless Codex session is such a process. The wait is outside the manager lock.
- Ruling Q7: yes — engine commands (`setup`, `check`, proof runs) drop `ANTHREX_WINDOW_ID` (decision 26; M8a.10's `engine_commands_get_the_profile_env_and_lose_agent_variables` already asserts its absence), why: they belong to no window, and an inherited value would attribute an `anthrex hook` run inside a check to the wrong window. Headless sessions still get their own window's id.
- Ruling Q8: yes — nothing in this brief depends on branch `docs/m8-brief-refresh`; the vectors and preconditions it cited are inline, why: the brief must stand alone on `main`.
- Ruling Q9: `every_key_is_read`'s `[orchestrator] max_parallel = 3` becomes `max_writers = 4` (M8a.3), why: a valid key whose value differs from its default (3), so the assertion shows the value was read.
- Ruling Q10: yes — `GitWiring` and `pump_git` move into `server/git_wiring.rs` as a separate pure-move commit inside M8a.22 (its Commit line now names two commits), why: `server.rs` is already 605 lines, and a pure move reviewed on its own keeps M8a.22's behavioural diff readable.
- Ruling Q11: AGENTS.md is not changed; the brief keeps the landed practice (each env-mutating test alone in its own test binary), why: the landed tests show a lock cannot stop a `set_var` racing a spawn on another libtest thread (`crates/daemon/tests/worktree_env.rs:1–20`, `git_env.rs:1–16`). The disagreement with AGENTS.md's "must hold a shared lock" is recorded for the user in `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`.
- Ruling Q12: config `[orchestrator.profile] protected` holds additions only (M8a.3's `profile_protected_is_read_as_additions`), why: the built-ins (decision 56) cannot be removed by configuration, and the config crate cannot see `BUILTIN_PROTECTED`; `resolve_profile` (M8a.5) merges them.
- Ruling Q13: nine fixtures, including `claude-<version>-documented.jsonl` with `"observed": false` in its meta (M8a.1 Files, decision 51), why: one meta file cannot mark some lines of a recording observed and others not, and a documented-only shape must never pass for a capture.

### M8a.1 external tools (2026-09-23)

Verified on macOS (Darwin 25.2, arm64) against `claude` 2.1.278, `codex-cli` 0.155.0 and `git` 2.50.1 (Apple Git-155). Every `claude` and `codex` run used a scrubbed environment (`env -i` with only `HOME`, `PATH`, `USER`, `LOGNAME`, `SHELL`, `TERM`, `LANG`), because the capture ran inside a Claude Code session whose `CLAUDE_CODE_*` and `CLAUDECODE` variables change how a child `claude` behaves (transcript saving off, messaging the parent); decision 26 scrubs the same names for the same reason. No `ANTHROPIC_API_KEY` was set. All work ran in scratch repositories under `/tmp/anthrex-m8a1/`, with `haiku` and `--effort low` for Claude and Codex's configured default model. Fixtures are in `crates/daemon/tests/fixtures/headless/`; each `.meta.json` has the exact command (paths redacted) and a note. Linux was not available: every Linux-specific point below is **not verified**.

**Design decisions that did not hold, or changed**

- **`codex exec resume` does not accept `-s`** (item 6): `codex exec resume -s read-only --json <id> -- hi` → `error: unexpected argument '-s' found` (exit 2). `-c` and `-m` are accepted. Decision 25's own fallback applies: the sandbox goes through `-c sandbox_mode="<mode>"` on resume, and item 7's resume turn proves it confines (a `touch ../escape-resume.txt` was refused, the commit succeeded). `-s` placed before the subcommand (`codex exec -s read-only resume …`) also parses, but was not used.
- **Codex loads a repository's own config, and nothing on the command line excludes it** (item 7a). Decision 53's third branch applies: `run start` refuses a run with a Codex task in a repository whose base tree tracks `.codex/config.toml` or `.codex/hooks.json`, unless `--trust-project`. Evidence and the side effect are under item 7a.
- **Plan mode blocks the reviewer's allowed `submit_review` call in `-p`** (decision 24's conditional, checked although it is not a numbered item). With `--permission-mode plan --permission-prompts none --allowedTools mcp__anthrex__task_done,Read,Glob,Grep,Bash(git diff:*),Bash(git log:*),Bash(git show:*)`, `git log` ran but the allowed MCP call was denied (`system/permission_denied`, `decision_reason: "no approval surface in this session; permission request denied automatically"`). Decision 24's fallback, `--permission-mode dontAsk --disallowedTools Edit,Write,NotebookEdit` with the same `--allowedTools`, works: `git log` ran, the MCP call ran, `touch reviewer.txt` was denied (`permission_denied` with `decision_reason_type: "mode"`). **Reviewers use the `dontAsk` fallback.**
- **The failed-turn category is not on the `result` line** (item 3 and item 5). A failed API turn is an `assistant` line with `"model":"<synthetic>"`, `"error":"<category>"`, `"is_api_error_message":true` and the error text as its only text block, followed by a `result` with `subtype: "success"`, `is_error: true`, `terminal_reason: "api_error"`, `api_error_status`. The Interfaces row "`Failed { kind }` by the error category" cannot be read from the `result` alone: `claude_stream::parse_line` must turn the assistant line's `error` into an event (or the turn's category must be carried to `TurnEnded` by the caller). M8a.7 decides which; the mapping table row changes either way.
- **Decision 54's block needs `failIfUnavailable: true`** (item 4b). Without it, "when the sandbox can't start, Claude Code shows a warning and runs commands unsandboxed" (settings reference, `sandbox.enabled`); the binary's text is `⚠ Sandbox disabled: <reason>`. With it, Claude exits at startup with `Error: sandbox required but unavailable: <reason>` / `  sandbox.failIfUnavailable is set — refusing to start without a working sandbox.` (exit 1, before `system/init`). So `SandboxKeys` needs a fourth key, `fail_if_unavailable: "failIfUnavailable"`, and `worker_settings_json_enables_the_sandbox` should expect it; otherwise decision 54's "the session fails with the text M8a.1 records" never happens. Not applied here: M8a.7 owns `SandboxKeys`.
- **A sandbox refusal is not a counted denial** (item 4b): it arrives only as a failed `Bash` `tool_result` (`is_error: true`, `Exit code 1\ntouch: ../escape.txt: Operation not permitted`), with no `permission_denied` line and an empty `permission_denials`. Per decision 54 it is not counted; stall and budget rules cover a worker that keeps trying.
- **A Codex command that exits non-zero does not appear in `exec --json` at all** (item 7). Codex 0.155 runs commands through a code-mode `exec` tool; the rollout file shows the failing `touch ../escape.txt` and `git commit` calls with exit codes 1 and 128, but `--json` emitted `command_execution` items only for the commands that exited 0. A sandbox refusal under `-s workspace-write` or `-s read-only` is therefore visible only in the agent's final message, never as a structured denial or a failed item. Decision 40's Codex tool-call counts are low by the number of failed commands; decision 32's denial count sees none.

All other decisions checked here hold:

- `claude -p` answers on the subscription login without `ANTHROPIC_API_KEY` (item 5): it held.
- stream-json input is accepted, and the process stays alive between turns and exits on EOF (item 2): it held.
- The interrupt control request works (item 2): it held.
- Hooks fire under `-p` (item 2): they held.
- `--setting-sources user --strict-mcp-config` excludes project settings and `.mcp.json` (item 4a): it held.
- The sandbox works under `-p` on macOS (item 4b): it held.
- `codex exec resume` accepts `-c` and `-m` (item 6): it held.
- `rmcp =3.4.0` (item 8): it held.
- `git merge-tree --write-tree` (item 9): it held.

**`CLI_CAPS`**

| Field | Value | Set by |
|-------|-------|--------|
| `claude_verbose` | `true` | item 1: without `--verbose`, `claude -p --output-format stream-json …` prints `Error: When using --print, --output-format=stream-json requires --verbose` (exit 1) |
| `claude_permission_prompts` | `true` | item 1: `--permission-prompts <target>`, choices `host` (default) and `none` |
| `claude_effort_flag` | `true` | item 1: `--effort <level>` (`low, medium, high, xhigh, max`); `--effort low` was accepted on every item 2–4b launch |
| `claude_non_bare_flag` | `None` | item 1: `--help` offers only `--bare`; `strings` on the binary finds no `no-bare`/`non-bare` |
| `claude_interrupt` | `InterruptMode::ControlRequest` | item 2 |
| `claude_hooks_fire_in_print` | `true` | item 2 |
| `codex_resume_takes_sandbox` | `false` | item 6 (resume uses `-c sandbox_mode=…`, proven in item 7) |
| `claude_user_settings_only` | `Some(&["--setting-sources", "user", "--strict-mcp-config"])` | item 4a |
| `claude_sandbox_keys` | `SandboxKeys { enabled: "enabled", allow_unsandboxed: "allowUnsandboxedCommands", write_allow: "filesystem.allowWrite" }` under `"sandbox"`; `write_allow` is a nested path (`{"sandbox":{"filesystem":{"allowWrite":[…]}}}`); plus `failIfUnavailable` (above) | item 4b |
| `codex_loads_project_config` | `true` | item 7a |
| `codex_project_config_paths` | `&[".codex/config.toml", ".codex/hooks.json"]` | item 7a |
| `codex_user_config_only` | `None` | item 7a |

**Item 1, Claude flags** (`claude --version` → `2.1.278 (Claude Code)`). `-p/--print`, `--input-format stream-json`, `--output-format stream-json` (needs `--verbose`), `--permission-prompts host|none`, `--session-id <uuid>`, `-r/--resume [value]`, `--settings <file-or-json>`, `--mcp-config <configs...>` (variadic), `--allowedTools, --allowed-tools <tools...>` ("Comma or space-separated", so one comma-separated value is accepted, and it is variadic), `--append-system-prompt`, `--permission-mode` with choices `acceptEdits, auto, bypassPermissions, manual, dontAsk, plan`, `--effort`, `--bare`, `--setting-sources <sources>`, `--strict-mcp-config`, `--disallowedTools`. The help says `--bare` "Sets CLAUDE_CODE_SIMPLE=1" and that its auth "is strictly ANTHROPIC_API_KEY or apiKeyHelper via --settings". The headless docs repeat that `--bare` "will become the default for `-p` in a future release"; no opt-out exists yet.

**Item 2, the recorded session** (`claude-2.1.278-stream.jsonl`, `-input.jsonl`, `-hooks.jsonl`). One process with decision 24's worker flags, including the sandbox block, `--allowedTools Bash,Read`, `--permission-mode acceptEdits`. Three turns:
- **Envelope.** Accepted, one line each: `{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<T>"}]},"parent_tool_use_id":null,"session_id":"<id>"}`. `claude_stream::user_message` writes exactly that.
- **Lifetime.** The process stays alive after every `result`, checked 2 s after each, and exits within 0.6 s of stdin EOF. The exit code was 0 when the last turn succeeded and 1 when it had been interrupted.
- **`system/init` is emitted again at the start of every turn.** It is not only the first line. `Init` handling must be idempotent, and `headless::status::next` must not treat a later `Init` as `Starting`.
- **Turn 1, Agent call.** The Agent call's sub-agent lines carry `parent_tool_use_id`. `system/task_started`, `task_updated` and `task_notification` frame it.
  - The first attempt got a *backgrounded* Agent (not committed). The turn ended early with its own `result`, and when the sub-agent finished, Claude Code started a **second, unprompted turn**: `UserPromptSubmit` and `Stop` hooks fired, a new `init` was emitted, and a second `result` arrived with no user message sent.
  - The engine must accept a `TurnEnded` it did not open (M8a.12's turn-end fallback and `turn_open` logic).
  - The committed recording asked for a foreground call (`run_in_background false`).
- **Turn 2, `Write`.** As the brief worded it (`x.txt` in the working directory), it is **allowed** under `acceptEdits` even though `Write` is not in `--allowedTools`: a probe created the file. The recording asks for a path outside the working directory instead. That was denied:
  - The stream has `{"type":"system","subtype":"permission_denied","tool_name":"Write","tool_use_id":…,"decision_reason_type":"asyncAgent","decision_reason":"no approval surface in this session; permission request denied automatically","message":"Permission for this tool use was denied. …"}`.
  - The `result` has `"permission_denials":[{"tool_name":"Write","tool_use_id":…,"tool_input":{…}}]`.
  - Under `dontAsk`, `permission_denied` has no `decision_reason`, only `message`. `PermissionDenied.reason` should take `decision_reason`, else `message`.
- **Turn 3, interrupt.** `sleep 60` was blocked before any hook by Claude Code's own guard (`<tool_use_error>Blocked: standalone sleep 60. …`), so the recording runs `python3 -c "import time; time.sleep(60)"`.
  - The interrupt, `{"type":"control_request","request_id":"1","request":{"subtype":"interrupt"}}`, was sent 8 s in. The CLI replied `{"type":"control_response","response":{"subtype":"success","request_id":"1","response":{"still_queued":[]}}}`.
  - Then came `system/task_notification` `status: "stopped"`, the tool result `The user doesn't want to proceed with this tool use. …`, and a user text `[Request interrupted by user for tool use]`.
  - The `result` has `subtype: "error_during_execution"`, `is_error: true`, `terminal_reason: "aborted_tools"`, `errors: ["[ede_diagnostic] …"]`.
  - **Interrupted marker:** `subtype == "error_during_execution"` and `terminal_reason` ∈ {`aborted_tools`, `aborted_streaming`}. The documented `terminal_reason` values include both.
  - `system/init.capabilities` advertises `interrupt_receipt_v1` and `interrupt_cancel_queued_v1`. SIGINT was not needed.
  - The same control request sent after a turn had ended was also answered `success` with `still_queued: []`, so a late interrupt is harmless.
- **Usage.**
  - `result.usage` is **per turn**. Turns 1 to 3: input 26/18/10, output 477/284/215, `cache_read_input_tokens` 66953/50323/25616.
  - `total_cost_usd` and `modelUsage` are cumulative for the session (0.0306 → 0.0381 → 0.0422).
  - `usage` fields: `input_tokens`, `output_tokens`, `cache_read_input_tokens`, `cache_creation_input_tokens`, `cache_creation{ephemeral_1h_input_tokens, ephemeral_5m_input_tokens}`, `server_tool_use{web_search_requests, web_fetch_requests}`, `output_tokens_details`, `service_tier`, `inference_geo`, `iterations`, `speed`.
  - The docs say the same: "per-turn in streaming-input sessions".
- **Hooks.** All of the following fired in `-p`:
  - Turn 1: `SessionStart`, `UserPromptSubmit`, `PreToolUse`/`PostToolUse` for Bash, `PreToolUse`/`PostToolUse` for Agent, `SubagentStart`/`SubagentStop` (with `agent_id`, `agent_type`), and `Stop`.
  - Turn 2: `PermissionRequest` for the denied Write.
  - The interrupted turn fired `UserPromptSubmit` and `PreToolUse`, but no `PostToolUse` and no `Stop`.
  - `SessionEnd` fired on EOF.
- **Line types the Interfaces table has no row for** (the stream meta's `unmodelled` list): `system/hook_started` and `system/hook_response` (the user's own `SessionStart` hooks, which run under `--setting-sources user` by design), `system/thinking_tokens`, `system/task_started`, `system/task_updated`, `system/task_notification`, `system/background_tasks_changed`, `system/vcs_state_changed`, `rate_limit_event` and `control_response`. Assistant `thinking` blocks arrive with empty text and a signature.
- **MCP tools are deferred.** The model loads `mcp__anthrex__*` through `ToolSearch` before calling one, which worked without `ToolSearch` in `--allowedTools`.

**Item 3, documented shapes** (`claude-2.1.278-documented.jsonl`, `"observed": false`):
- `system/api_retry` fields, from the headless doc: `attempt`, `max_retries`, `retry_delay_ms`, `error_status`, optional `no_response`, `error`, `uuid`, `session_id`.
- `error` ∈ `authentication_failed, oauth_org_not_allowed, account_on_hold, billing_error, rate_limit, overloaded, invalid_request, model_not_found, server_error, max_output_tokens, cloud_credential_error, unknown`. The file has three.
- `compact_boundary` has `compact_metadata {trigger, pre_tokens}`.
- Failed turns follow the observed `authentication_failed` shape (item 5): `rate_limit` ("You've hit your session limit · resets 3:45pm", 429) and `billing_error` ("Credit balance is too low", 400). Those two texts come from the error reference.

**Item 4, resume.** `--resume <id>` with every flag re-passed accepted a fourth message:
- `system/init.session_id` is the same id, and the model answered from the earlier turns.
- **Failed-resume marker.** With a random UUID, the process exits 1 before any `init`.
  - stderr has `No conversation found with session ID: <id>`.
  - stdout has one `result` with `subtype: "error_during_execution"`, `is_error: true`, `num_turns: 0` and `errors: ["No conversation found with session ID: <id>"]`.

**Item 4a, project settings** (`claude-2.1.278-project-settings.jsonl`). The scratch repository tracks two files:
- `.claude/settings.json`, with a `PreToolUse` hook that touches a marker file.
- `.mcp.json`, with a server that writes another marker when it starts.

With `--setting-sources user --strict-mcp-config`, the result was:
- neither marker appeared;
- `system/init.mcp_servers` was `[{"name":"anthrex","status":"connected","source":"dynamic"}]`;
- anthrex's `--settings` hooks fired.

The same session without the two flags wrote **both** markers and listed `{"name":"projectserver","source":"project"}` in `mcp_servers`, so the regression test would catch a lost flag. That run is not committed, because its `init` lists the user's own MCP servers.

`--strict-mcp-config` also drops the user's own MCP servers, and only `anthrex` loads.

**Item 4b, sandbox** (`claude-2.1.278-sandbox.jsonl`). The keys come from the settings reference for 2.1.278 and appear in the binary:
- `sandbox.enabled`, default false;
- `sandbox.allowUnsandboxedCommands`, default true;
- `sandbox.filesystem.allowWrite`, an array of paths added to the writable set;
- `sandbox.failIfUnavailable`, default false;
- `sandbox.autoAllowBashIfSandboxed`, default true, which is why sandboxed `Bash` runs without a prompt even outside `--allowedTools`.

In a linked worktree (`git worktree add`) with `{"enabled":true,"allowUnsandboxedCommands":false,"filesystem":{"allowWrite":["<git common dir>"]}}`:
1. **`touch ../escape.txt`** (one directory above the worktree) was refused, as a failed `Bash` result only.
2. **`git add` plus `git commit`** inside the worktree succeeded. `system/vcs_state_changed {"kind":"commit"}` followed.

The same commit **also succeeds without `allowWrite`**. The settings reference says: "When the session's working directory is a linked git worktree …, the repository's common `.git` directory stays readable and writable to sandboxed commands". Keeping `allowWrite` is harmless and guards against a change.

The sandbox-cannot-start text could not be triggered on macOS:
- `enabledPlatforms: ["linux"]` in `--settings` was ignored.
- Stripping `/usr/bin` from `PATH` broke keychain login instead (`Not logged in`).

The texts above come from the binary and the docs. The SDK docs add that a session which cannot start its sandbox reports a `result` with `subtype: "error_during_execution"` and the reason in `errors`. Linux (bubblewrap): **not verified**.

**Item 5, authentication.**
- With `ANTHROPIC_API_KEY` unset, `claude -p --model haiku "reply with the single word hi"` printed `hi` (exit 0).
- The same with `--bare` printed `Not logged in · Please run /login` (exit 1). In stream-json that is:
  - the assistant `error: "authentication_failed"` pair of item 3;
  - `system/init.apiKeySource: "none"`.
- No API key was available, so whether `--settings` hooks and `--mcp-config` apply under `--bare` is **not verified**. Per decision 50, `auth = "api_key"` stays refused at config load.

**Item 6, Codex flags** (`codex-cli 0.155.0`).
- `codex exec` has `--json`, `-s/--sandbox read-only|workspace-write|danger-full-access`, `-m/--model`, `-c/--config`, `--ignore-user-config` (user config only), `--ignore-rules`, `--dangerously-bypass-hook-trust` and `--skip-git-repo-check`.
- `codex exec resume [SESSION_ID] [PROMPT]` has `-c`, `-m` and `--json`. It has no `-s` (above).
- **No-such-session text:** `Error: thread/resume: thread/resume failed: no rollout found for thread id <id> (code -32600)`.
- `strings` on the native binary (`/opt/homebrew/Caskroom/codex/0.155.0/bin/codex`) finds each of these: `default_tools_approval_mode` (28), `tool_timeout_sec` (6), `developer_instructions` (61), `model_reasoning_effort` (29), `approval_policy` (44), `sandbox_workspace_write` (21), `writable_roots` (28), `sandbox_mode` (41).
- The recorded runs passed every decision-25 `-c` key without a config error.

**Item 7, Codex session** (`codex-0.155.0-exec.jsonl`, `-resume.jsonl`).
- **`/tmp` is always writable under `workspace-write`.** The sandbox makes `/tmp` and `$TMPDIR` writable by default. In the first attempt, under `/tmp`, both the commit *without* the writable root and `touch ../escape.txt` succeeded, which proved nothing. The recorded runs add `-c sandbox_workspace_write.exclude_slash_tmp=true -c sandbox_workspace_write.exclude_tmpdir_env_var=true`.
- **With those keys, the writable root is needed.**
  - Without `writable_roots`, the commit failed: the agent reported "index.lock permission denied" and the rollout shows `git commit` exit 128, but nothing structured is in the stream (above).
  - With `writable_roots=["<git common dir>"]`, it succeeded (`[task-b 9d0ec2a] add a`).
- **Resume.** `exec resume <thread_id>` with the `-c` flags and `-c sandbox_mode="workspace-write"` appended a line and committed. `thread.started` repeats the same `thread_id`.
- **`turn.completed.usage`** has `input_tokens`, `cached_input_tokens`, `cache_write_input_tokens`, `output_tokens` and `reasoning_output_tokens`. That is two more fields than the Interfaces row.
- **`-m no-such-model`:**
  - `item.completed` of `item.type: "error"` (`Model metadata for \`no-such-model\` not found. …`), then `turn.started`;
  - `{"type":"error","message":"<json>"}`;
  - `{"type":"turn.failed","error":{"message":"{\"type\":\"error\",\"status\":400,\"error\":{\"type\":\"invalid_request_error\",\"message\":\"The 'no-such-model' model is not supported when using Codex with a ChatGPT account.\"}}"}}`;
  - exit 1.
  - The message is the API error as a JSON string. `FailureKind` can read its `status`, and `turn.failed` → `Other` for this one.
- **`item.type: "error"` items are notices, not failures.** The same type also carries "Skill descriptions were shortened …".
- **Rate-limit pattern.** Unobserved; from the binary's strings: `Usage limit reached. You've reached your usage limit.`, `exceeded retry limit, last status: <status>`, "rate limit", "429", "too many requests". Classify `RateLimit` when the message (or its JSON `status`) matches `429`, `rate limit`, `usage limit` or `too many requests`, case-insensitively.
- **SIGINT to a running `exec`.** Sent 14 s into a 60 s command, the process exited 1 about 1.4 s later.
  - No `turn.completed` or `turn.failed` was printed: the stream ends after `item.started`.
  - stderr had `ERROR codex_core::tools::router: error=write_stdin failed: Unknown process id <n>`.
  - No child was left behind.
  - A Codex interrupt is therefore a `ProcessExited` with no `TurnEnded`.
- **`-s read-only` write refusal.** `echo x > r.txt` was refused (`operation not permitted`). There was no structured denial and not even a failed item; only the agent's text reports it.

**Item 7a, Codex project config** (`codex-0.155.0-project-config.jsonl`). The scratch repository commits:
- `.codex/config.toml`, with `developer_instructions = "Always end every reply with the exact word PROJECT-CONFIG-LOADED"` and `[mcp_servers.projmcp]`, whose command writes a marker;
- `.codex/hooks.json`, with `SessionStart`, `PreToolUse` and `UserPromptSubmit` hooks that touch markers.

Findings:
- **Auto-trust.** The first `codex exec` in the repository **wrote `[projects."<repo root>"] trust_level = "trusted"` into the user's `~/.codex/config.toml` by itself**. It did the same for item 7's repository.
- **Project config loads.**
  - With decision 25's flags, the project MCP server started (marker written) on the first turn, and again on `exec resume`.
  - Without anthrex's `-c developer_instructions`, the reply was `ok PROJECT-CONFIG-LOADED`: the project's `developer_instructions` loaded.
  - `-c` overrides win over project values for the keys anthrex sets. Anything else in the project config, such as MCP servers, loads.
- **Project hooks did not fire.** Codex requires persisted hook trust; `--dangerously-bypass-hook-trust` would skip that.
- **Only a user-config trust entry excludes it.** With the user config's entry for the repository set to `trust_level = "untrusted"`, the project MCP server did **not** start. Trust is what gates project config.
- **No command-line exclusion.** `-c 'projects."<root>".trust_level="untrusted"'` on the command line did **not** stop it. `codex exec --help` has no project-config switch (`--ignore-user-config` ignores the *user* file).
- **The user's config was restored.** It was not copied before item 7, because the change was made by Codex, not by this task. The original was reconstructed by removing exactly the two blocks Codex added (`/private/tmp/anthrex-m8a1/s7` and `/s7a`), confirmed by `diff` to be the only differences. The file was restored and checked with `cmp`. The item 7a `untrusted` edit was made and undone inside that window.

**Item 7a follow-up: a per-run trust override does not work** (coordinator request, 2026-09-23). Question: can `-c 'projects."<abs repo path>".trust_level="untrusted"'` on each invocation stop both problems, the project config loading and the automatic trust write, without touching the user's config? **No.** `codex_user_config_only` stays `None` and decision 53's third branch stands.

Method:
- `~/.codex/config.toml` was copied to the scratchpad before the first run. It was diffed after every run, then restored and checked with `cmp` (identical).
- Five fresh scratch repositories (`r1`–`r5`) under `/tmp/anthrex-m8a1/`, each committing a `.codex/config.toml` with `developer_instructions = "Always end every reply with the exact word PROJECT-CONFIG-LOADED"` and an MCP server that appends to a per-repository marker file.
- Prompt `Reply with the single word ok.`, with `-c approval_policy="never" -c model_reasoning_effort="low"`, and no anthrex `developer_instructions`, so the project's would show.

| Run | Sandbox | Override key | Reply | Marker | Trust entry written |
|-----|---------|--------------|-------|--------|---------------------|
| r1 `exec` | `-s read-only` | `/tmp/anthrex-m8a1/r1` (the path as typed) | `ok` | no | no |
| r1 `exec resume <thread>` | `-c sandbox_mode="read-only"` | same | `ok` | no | no |
| r2 `exec` | `-s read-only` | `/private/tmp/anthrex-m8a1/r2` (realpath) | `ok` | no | no |
| r2 `exec resume <thread>` | `-c sandbox_mode="read-only"` | same | `ok` | no | no |
| **r5 `exec`, control** | `-s read-only` | **none** | `ok` | no | no |
| r4 `exec` | `-s workspace-write` | `/private/tmp/anthrex-m8a1/r4` | `ok PROJECT-CONFIG-LOADED` | yes | **yes** (`[projects."/private/tmp/anthrex-m8a1/r4"] trust_level = "trusted"`) |
| **r3 `exec`, control** | `-s workspace-write` | none | `ok PROJECT-CONFIG-LOADED` | yes | yes |
| r3 `exec`, after r3 was trusted | `-s workspace-write` | `/private/tmp/anthrex-m8a1/r3` | `ok PROJECT-CONFIG-LOADED` | yes | (already present) |
| r3 `exec`, after r3 was trusted | `-s read-only` | none | `ok PROJECT-CONFIG-LOADED` | yes | (already present) |

What the runs show:
- **(a) The override does not stop project config from loading.** The read-only runs were clean with or without it: control r5 behaves the same as r1 and r2. Under `workspace-write` (a worker's sandbox), r4 loaded the project config despite the override.
- **(b) The override does not stop the trust write.** Codex writes `trust_level = "trusted"` for the repository root on a `workspace-write` run whether or not the override is passed (r4). It writes nothing on a read-only run, with or without it.
- **(c) Resume behaves the same.** `exec resume` with the override and a read-only sandbox loaded nothing and wrote nothing, but so did the read-only first turn without it. There is no evidence the override did anything on resume.
- **Path form makes no difference.** The key as typed and the realpath behaved the same.
- **The earlier item 7a observation is refined.** The automatic trust write happens on a `workspace-write` run (item 7 and 7a used it), not on a `read-only` one. Once a repository is trusted, `read-only` runs load its project config as well (the last r3 row).
  - So a Codex reviewer (`-s read-only`) avoids project config only until a Codex worker (`workspace-write`) has run in that repository, which is the normal order within a run.
  - Decision 53's refusal therefore has to cover both roles.
- No fixture was added. The streams are ordinary `thread.started`/`agent_message`/`turn.completed` runs, and the evidence is the marker files and the config diffs, recorded above.

**Item 8, rmcp.** In a throwaway crate, `cargo add rmcp@=3.4.0 --no-default-features --features server,transport-io` built and served a hand-written `ServerHandler` over stdio. It answered `initialize`, `tools/list` and `tools/call` correctly. The names that resolve in 3.4.0 are:
- `rmcp::{ServerHandler, ServiceExt, ErrorData}`;
- `rmcp::model::{Tool, JsonObject, ListToolsResult, CallToolRequestParams, CallToolResult, CallToolResponse, ContentBlock, PaginatedRequestParams, ServerCapabilities, ServerConfig}`;
- `rmcp::service::{RequestContext, RoleServer}`;
- `rmcp::transport::stdio()`.

Names that differ from what one might expect:
- `ServerHandler::get_info` returns `ServerConfig` (an alias of `InitializeResult`).
- `call_tool` returns `Result<CallToolResponse, ErrorData>`, built with `CallToolResult::success(…).into()`.
- Content is `ContentBlock::text(…)`.
- `Tool::new(name, description, Arc<JsonObject>)`.
- Serving is `handler.serve((reader, writer)).await?.waiting().await`.

**Item 9, git.** `git version 2.50.1 (Apple Git-155)` (≥ 2.38). `git merge-tree --write-tree --name-only --no-messages A B`:
- **A clean pair** printed one line, the tree (`b4f775c9a1855fa148f96b044be3ef80f4dd44be`), and exited 0.
- **A conflicting pair** printed the tree on line 1 and `f.txt` on line 2, and exited 1. A file changed on only one side (`g.txt`) is not listed.

**Side effects on the user's machine**
- Codex's `trust_level` entries were restored as above.
- `~/.claude.json` was updated by `claude` itself (per-project state for the scratch directories), which the task allows.
- Both CLIs left their usual transcripts: Claude under `~/.claude/projects/-private-tmp-anthrex-m8a1-*`, Codex under `~/.codex/sessions/2026/09/23/`. They were left in place rather than deleted.

### M8a.3 the `[orchestrator]` config and the roster (2026-09-23)

Implemented as specified, with these choices made where the brief and the Interfaces
block left a detail open:

- **`orchestrator::read`'s signature takes the whole top-level table**, exactly as the
  Interfaces block gives it (`pub(crate) fn read(table: &toml::Table, problems: &mut
  Vec<Problem>) -> Orchestrator`), unlike `read_git`/`read_conversation`'s
  `(&toml::Table, &mut Config, &mut Vec<Problem>)` shape. This is required, not a
  choice: `review.small` is a dotted key under `[orchestrator]` (decision 35, Ruling
  Q2), and the `toml` crate folds `[orchestrator]\nreview.small = "off"` and
  `[orchestrator.review]\nsmall = "off"` into the identical nested table once parsed,
  so `read_review` only needs `orchestrator`'s own sub-table either way -- confirmed by
  `review_small_is_read_as_on_or_off`, which asserts both forms give the same result.
- **`crates/config/src/orchestrator.rs` (588 lines after the split below) is further
  split into `orchestrator/roster.rs`** (`[[orchestrator.models]]` parsing and
  `default_roster()`, decision 23) **and `orchestrator/profile.rs`**
  (`[orchestrator.profile]` and `[orchestrator.profile.env]`, decision 56), both
  declared as submodules of `orchestrator` (`crates/config/src/orchestrator/*.rs`)
  rather than as siblings declared from `lib.rs`, so `lib.rs`'s budget (at most 8
  lines) is untouched by the split. `orchestrator_tests.rs` (556 lines) is likewise
  split, with the roster tests in `orchestrator_tests_roster.rs`, declared as a
  submodule from the bottom of `orchestrator_tests.rs`.
- **`orchestrator.models[i].note`'s length limit (decision 23's `note` field) is not
  given a number anywhere in the brief** -- only task-3's test list requires "an
  81-character note" to be invalid. `MODEL_NOTE_MAX = 80` (`orchestrator/roster.rs`) is
  this task's own choice, not derived from a decision or spec section; a later
  milestone should confirm or replace it if the spec settles on a different bound.
- **Problem messages beyond the two the brief gives verbatim** (`orchestrator.max_writers:
  must be between 1 and 8 (using 3)`, and `orchestrator.review.small: must be "on" or
  "off" (using "on")`, both asserted exactly as quoted) are this task's own wording,
  following the style of the existing `git.rs`/`conversation.rs` messages; no other
  message text is pinned by the brief or the milestone doc.
- **`lib.rs` grew by 4 lines** (600 to 604), within the "at most 8" budget: `mod
  orchestrator;`, one `pub use` (`ClaudeAuth, ClaudeHeadless, Orchestrator,
  default_roster`), the `pub orchestrator: Orchestrator` field, its default, the
  `orchestrator::read` call, and the `report_unknown_keys` arm added five lines, offset
  by three lines removed with the old silent-skip arm and its explanatory comment.
- **`crates/config/src/lib_tests.rs`'s `every_key_is_read`** now sets `[orchestrator]
  max_writers = 4` (Ruling Q9) and asserts `config.orchestrator.max_writers == 4`; no
  other line in that test, or any other test in `lib_tests.rs`, changed.

Verification run 2026-09-23: `cargo build --workspace --all-targets`, `cargo test
--workspace` (all crates, 0 failures), `cargo clippy --workspace --all-targets -- -D
warnings` (clean) and `cargo fmt --all --check` (clean, after one `cargo fmt --all`
pass) all passed. `python3 scripts/pty-smoke.py` was also run to satisfy the standing
five-gate rule; this task touches no daemon or PTY behaviour, so it exercises no new
code path.

### M8a.4 globs and roster policy (2026-09-23)

Implemented as specified. `crates/daemon/src/run/{mod,model,globs,roster}.rs` created,
`lib.rs` gained `pub mod run;` (coexisting with the `run` function re-exported from
`lifecycle`, as the brief's note says: a module and a function are different
namespaces), and `Cargo.toml` / `crates/daemon/Cargo.toml` gained `globset`, `regex`
and `toml` as workspace dependencies (`regex` and `toml` are unused by this task; they
are here because later M8a tasks in `run/` need them and the brief asked for all three
now).

- **Test files are split out**, `globs_tests.rs` and `roster_tests.rs`, `#[path = …]`
  declared from the bottom of `globs.rs`/`roster.rs`, following `config/orchestrator.rs`'s
  and `worktree.rs`'s existing conventions in this repository, rather than an inline
  `mod tests`. Both source files stay well under the ~600-line guidance (214 and 155
  lines).
- **`run/model.rs` holds only `ReviewLevel`** as the brief and refresh note C41 specify;
  the rest of the pure model (`Profile`, `RunLimits`, `Task`, `Run`, …) is M8a.5's.
- **`pick_reviewer`'s "preferring a model different from the author's" fallback**
  (decision 35's middle case) is read as: among the same-runtime candidates at or above
  the required strength, first exclude any entry whose model equals the author's, and
  pick the lowest strength among what remains; only if nothing remains (every
  same-runtime candidate at the required strength *is* the author's own model) does the
  policy fall through to the third case, the same runtime's highest-strength entry
  regardless of model. This is what makes `frontier_level_falls_back_to_the_same_runtime`
  (a Claude `claude-opus-5` author with no Codex frontier entry) land on the third case
  and return `claude-opus-5` "as the last resort", per the test's own wording, rather
  than stopping at the second case with the same result for a different reason. Not
  pinned verbatim by the brief; this is this task's reading of "preferring", verified
  against all four `pick_reviewer`/`escalate` test cases the brief gives.
- **`roster::peer`** maps `Shell` to itself; the brief only defines Claude/Codex
  swapping, and no test exercises `Shell` (it is never an authored or reviewer runtime
  in this milestone), so this is a defensive default rather than a decision.
- **Decision 2's grep** (`grep -nE 'std::fs|std::process|std::thread|tokio|std::time::SystemTime'`
  against `globs.rs`, `roster.rs` and `model.rs`) matches only each file's own doc
  comment naming the forbidden APIs it avoids; no code path in any of the three files
  uses them.
- **TDD**: each of the 13 named tests was shown red first by temporarily replacing
  `globs.rs` and `roster.rs` with `unimplemented!()` stubs of the same public
  signatures and running `cargo test -p anthrex-daemon --lib run::`; all 13 panicked
  with "not implemented". The real implementations were then restored and every test
  passed.

Verification run 2026-09-23: `cargo build --workspace --all-targets`, `cargo test
--workspace` (all crates, 0 failures, including the 13 new `run::globs`/`run::roster`
tests), `cargo clippy --workspace --all-targets -- -D warnings` (clean, after
collapsing one nested `if let` in `roster::escalate`) and `cargo fmt --all --check`
(clean, after one `cargo fmt --all` pass) all passed. `python3 scripts/pty-smoke.py`
passed (`ALL SMOKE STAGES PASSED`); this task touches no daemon or PTY behaviour, so it
exercises no new code path.

### M8a.4 fix round 1 (2026-09-23)

Per the coordinator's rulings on `task-4-review.md`'s findings 1 and 2:

- **I1 (fix).** Added `intersection_compares_path_components_not_string_prefixes` to
  `globs_tests.rs`, covering finding 1's exact construction (`crates/auth/**` vs
  `crates/authz/**`, and `crates/auth` vs `crates/authz/x.rs`). Shown red by mutating
  `intersects` to join each literal prefix into a string and compare with
  `str::starts_with`; the real component-vector comparison was then restored and every
  `run::globs` test passed. No production code changed.
- **I2 (no code change, pinned).** Added
  `pick_reviewer_prefers_a_stronger_model_over_the_authors_own` to `roster_tests.rs`,
  finding 2's exact construction (a Claude-only roster spanning fast/standard/frontier,
  a `standard` author at `ReviewLevel::Medium`) asserting the frontier model, not the
  author's own. Shown red by mutating `pick_reviewer`'s same-runtime call to pass
  `None` instead of `Some(&author.model)` for `exclude_model` (which lands on the
  author's own `claude-sonnet-5` instead); the real call was restored and the test
  passed. Why this reading and not the softer one finding 2 raises: decision 35 gives
  the reviewer route three ranked cases — the other runtime, then the same runtime
  "preferring a model different from the author's", then the same runtime's
  highest-strength entry as a last resort — and reading "preferring" as a hard
  exclusion is what keeps those three cases distinct rather than letting the second
  case silently absorb the third's job whenever the author is on the roster's top
  model at the required strength.
- **Backslash (fix).** `validate_glob` now rejects any glob containing `\`, with the
  message `must not contain \`, per finding 4 (globset's escape semantics on the
  matching side disagree with the intersection side's plain string comparison for a
  backslash). Added `globs_with_a_backslash_are_invalid` to `globs_tests.rs`, shown red
  against the pre-fix `validate_glob` (no backslash rule), then green after adding the
  `glob.contains('\\')` check.
- **Left as they are**, per the ruling: the `names_literally` guard (finding 3) and
  `literal_prefix("")` (the nit).

Verification run 2026-09-23 (fix round 1): `cargo test -p anthrex-daemon --lib run::`
(16 passed, 0 failed — the 13 from the original task plus the three added here),
`cargo clippy --workspace --all-targets -- -D warnings` (clean) and `cargo fmt --all
--check` (clean) all passed. `git status --porcelain` was empty after each mutation was
restored via `git show HEAD:<path> > <path>`, and at the end of the round.

### M8a.5 plan parsing, resolution and validation (2026-09-23)

Implemented decisions 7–12, 15's slug and 56's plan warning in `run/plan.rs`,
`run/validate.rs`, `run/validate_graph.rs`, `run/env.rs` and the rest of `run/model.rs`.
Deviations and readings the brief left open:

- **`validate.rs` is split by rule family.** Task resolution (decisions 8–10, 35's level,
  56's warning) stays in `validate.rs`; the cross-task rules (count, ids, graph, L,
  runtime overlap, area) and `implicit_deps` are in `validate_graph.rs`, re-exported from
  `validate.rs` so the Interfaces paths hold. `validate_graph.rs` is pure and belongs in
  decision 2's purity list beside `validate.rs`. Tests: `plan_tests.rs` +
  `plan_tests_parse.rs`, `validate_tests.rs` + `validate_tests_fields.rs`, shared
  fixtures in `run/test_support.rs` (`#[cfg(test)]`).
- **Not yet in the model:** `PendingOp` and `Run.pending_ops`, because `PendingOp.kind`
  is the engine's `OpKind` (M8a.11). M8a.11 adds both.
- **`ClaudeAuth`** is mirrored in `run/model.rs` with serde (`From<config::ClaudeAuth>`):
  `RunLimits` is persisted, and the config crate has no serde dependency.
- **Rule ids.** The brief gives rule numbers only inside some messages. `PlanError.rule`
  is that number where the message cites one (`7.2.4`, `8.1`, `9`, `12.1`), `8` for the
  missing `test_mode_reason`, and otherwise a family: `fields` (blank goal, title,
  brief, acceptance item; missing acceptance or owns), `id` (syntax, `integration`,
  duplicates), `globs` (`owns`, `profile.generated`, `profile.protected` via
  `validate_glob`), `kind` (decision 6), `range` (plan limits,
  `profile.check_timeout_secs`, task budgets, `max_tasks`), `profile` (`single_test`,
  `test_passed`, env keys), `route` (decision 8).
- **Messages this task chose** (no text in the brief): `must be between <lo> and <hi>`
  (as config's); `budget.<axis>: must be at least 1`; `profile.single_test: must contain
  {test}`; `profile.test_passed: is not a valid regular expression: <regex error>` (the
  pattern is compiled with `{test}` replaced by an escaped stand-in, since `{test}` is a
  placeholder, not a repetition); `profile.env: key <k> must match
  [A-Za-z_][A-Za-z0-9_]*`; `id: must match ^[a-z0-9][a-z0-9-]{0,15}$`; `id: integration is
  reserved for the run branch`; `id: <id> is used by an earlier task` (on each later
  duplicate); `deps: <dep> is not a task`; `tasks: <n> tasks exceed max_tasks (<max>)`;
  `area: <glob> must be <literal>/** or a literal path`; `route.model: <m> is not in the
  roster for <runtime>`; `route.strength: <m> is <s> in the roster, not <given>`;
  `route: the roster has no <runtime> model at <strength> strength`; `route.runtime:
  must be claude or codex`; the 7.2.2 note `size raised from <a> to L: owns spans <n>
  modules and interface_change is set (rule 7.2.2)`; the 7.2.3 note `size raised from
  <a> to M: owns touch the hub globs (rule 7.2.3)`. When one glob spans several modules
  by itself, the 7.2.1 note says `owns spans more than one module` (no count exists).
- **A hub Codex task with the built-in roster is an error** (`route: the roster has no
  codex model at frontier strength`): decision 8 fills the model from "the first roster
  entry for the runtime at the strength", and the built-in Codex entry is `standard`.
  The planner must name `strength = "standard"` (or a model) for such a task.
  Worth revisiting when M9's planner routes hub work to Codex.
- **`test_mode_reason`** is required for every task whose mode is not `tdd`, a docs task
  left at its default `none` included (decision 10; fix round 1 removed an exemption the
  first version had). A hub code task forced to `tdd` (8.2) skips both the reason and the
  8.1 check.
- **One review raise, not two.** A task gets one level up when any of: no `check`, a
  non-`tdd` task touching `source`, or rule 8.3 turned it into `check`. An 8.3 task
  touching `source` meets two of these and is still raised once.
- **Decision 11's literal-prefix intersection** makes `source = ["crates/*/src/**"]`
  touch everything under `crates/` (its prefix is `crates`), so the fixtures for "owns
  outside source" use `scripts/…`. Behaviour as specified; noted because the fixtures
  first used `crates/b/build.rs` and one test passed for the wrong reason.
- **Rule scoping in `validate_tasks`:** every unfinished task is checked for ids,
  unknown deps, cycles and runtime overlap; the L rule, the cancelled-dependency rule
  and the area rule apply to `touched` tasks only (decision 13's exemption, and a
  cancel must not make every later edit fail on the dependents it blocked).
  `max_tasks` counts every task that is not cancelled. The `profile` parameter is
  unused (`_profile`): the profile-dependent rules run per task in `resolve_task`.
- **Implicit dependencies** (decision 41 at plan time) skip a pair where the earlier
  task can already reach the later one through declared deps and the implicit deps
  given to earlier tasks so far — plan order would otherwise deadlock them — and a pair
  the later task already declares. See fix round 1, F1.
- **`build_run`** leaves the run in `awaiting_approval`; `ctx.yes` sets `approved_by =
  Some("--yes")` and M8a.11's `Start` moves the run on. `BuildContext.data_dir` is the
  run's own directory (`<data_dir>/runs/<id>`), matching `Run::report_path`.
- **Bounds checked mechanically.** Limits: unit 1, bounds 1..=8, 1..=5 and 10..=14400;
  the unit divides every bound, and each edge is accepted and each value one past it
  rejected, one field at a time, with the three limits pairwise distinct in every
  fixture. Budgets: unit 1, bound "at least 1": 0 rejected, 1 accepted. `max_tasks`:
  unit 1 task, bound 3 (not the default 50): 3 build, 4 rejected. Id length: 16
  accepted, 17 rejected. Slug: after mapping only ASCII survives, so the 32-character
  cut is a 32-byte cut; tested with a 60-character goal whose cut lands mid-word
  (`refactor-the-scheduler-loop-to-i`), one whose 32nd character ends a word (32
  kept), and one whose 32nd character is `-` (trimmed to 31).
- **TDD.** All 50 new tests were shown red against `todo!()` stubs of the public
  signatures (`17 passed; 50 failed` — the 17 passing were M8a.4's). Mutations then
  showed `implicit_dep_never_contradicts_a_declared_one`,
  `tdd_without_single_test_becomes_check_and_raises_review` (after its fixture moved
  outside `crates/`), the literal-name skip of the protected warning, and the
  default-mode reason exemption each fail when their line is removed.
- **Decision 2's grep** over `plan.rs`, `validate.rs`, `validate_graph.rs`, `env.rs` and
  `model.rs` matches only each file's doc comment naming the forbidden APIs.
  `random_suffix` draws from `std::collections::hash_map::RandomState`, no clock.

### M8a.5 fix round 1 (2026-09-23)

Coordinator rulings on `task-5-review.md` findings 1–12 (13 needs no change):

- **F1 (fix).** `implicit_deps` checks reachability over declared deps plus the implicit
  deps it has already given to earlier tasks, so no implicit edge closes a cycle. The
  reviewer's case (t1 `crates/a/**` deps t3, t2 `crates/a/src/**`, t3
  `crates/a/src/y.rs`) now gives t2 `["t1"]` and t3 `[]`, and the order is t3, t1, t2.
  As a backstop, `build_run` runs `combined_cycles` (declared plus implicit, same
  `deps: cycle …` message, rule `12.1`) after filling implicit deps. Tests:
  `implicit_deps_never_close_a_cycle_through_other_implicit_deps` (through
  `build_run`) and `the_combined_graph_check_reports_an_implicit_cycle`. The backstop
  call in `build_run` is unreachable while `implicit_deps` is correct, so no test can
  make it fire; the function itself is tested.
- **F2 (fix).** The docs exemption is gone. Every task whose mode is not `tdd` needs
  `test_mode_reason` (decision 10). The docs fixtures carry reasons, and
  `non_tdd_needs_a_reason` has a docs-default case.
- **F3 (fix, `globs.rs`).** A glob with an empty literal prefix (`**`, `**/*.rs`) spans
  every configured module, because the empty sequence is a component-prefix of every
  pattern. So rule 7.2.1 raises it (`owns spans more than one module`). With no
  `modules` configured it is still `.`. Tests: `an_empty_literal_prefix_spans_every_module`
  (globs) and `a_glob_with_an_empty_literal_prefix_spans_every_module` (validation,
  with no hub).
- **F4 (fix).** `validate_glob` compiles each glob as `OwnsMatcher` would, and a failure
  is `<glob> is not a valid glob: <globset error kind>`, for example `crates/a/src/[x.rs
  is not a valid glob: unclosed character class; missing ']'`, with rule `globs` and
  field `owns`, `profile.generated` or `profile.protected`. Tests:
  `globs_that_do_not_compile_are_invalid` and `globs_that_do_not_compile_are_rejected`.
- **F5 (fix).** `validate_glob` rejects a `.` component, which covers a leading `./`
  (`must not contain .`), and an empty component (`must not contain //`). A trailing `/`
  is still allowed, because decision 11 ignores it. Nothing is normalised. Tests:
  `dot_and_empty_components_are_invalid` and `dot_components_in_owns_are_rejected`.
- **F6–F11 (pinning tests).** Each is shown to kill the reviewer's surviving mutation:
  - `two_modules_raise_s_to_m` now asserts effort, budget and review follow the raised
    size.
  - `unset_plan_limits_come_from_config` uses config writers 5, readers 6, bounces 4.
  - `yes_records_the_approval`.
  - A hub docs task in `hub_code_is_forced_to_tdd` stays `none` with no 8.2 note.
  - `rule_8_3_on_source_raises_review_once`.
  - Eight `validate_tasks` scoping tests: cross-runtime and implicit deps ignore
    finished tasks, a cancelled dependency of an untouched task is allowed, `max_tasks`
    skips cancelled tasks, the area rule applies to touched tasks only, a duplicate of a
    finished task is still a duplicate, a dependency resolves to the first task with its
    id, and a declared dependency is not repeated as implicit.
- **F12 (fix).** `build_run` sets `revision = 1` (decision 47) and `unverified =
  profile.check.is_none()` (decision 34). Test:
  `a_new_run_starts_at_revision_one_and_is_unverified_without_check`.

Red first: the 10 tests for F1–F5, F2 and F12 failed before their fixes (`78 passed; 10
failed`). The pinning tests passed against the existing code and were each shown red
by one mutation per surviving mutant, 17 mutations in all. After each mutation run,
the working files were restored by copying them back from a scratch snapshot of the
working tree. `git show HEAD:<path>` could not be used, because HEAD does not contain
the uncommitted fixes being tested.

### M8a.6 plan edits (2026-09-23)

Implemented decision 13 in `run/edits.rs` (`apply_edits`, `EditConsequence`), pure.
Deviations and readings the brief left open:

- **`run/contract.rs` is created here, not in M8a.11**, with only `answer_message` and
  `amend_message` (exact Interfaces texts), because
  `amend_brief_on_a_working_task_yields_a_deliver_consequence` compares against
  `amend_message`. M8a.11 adds the contracts, prompts and other messages to it.
- **Tests are split** into `edits_tests.rs` (fixtures; add, amend, cancel, split) and
  `edits_tests_rules.rs` (deps, answer, atomicity, L, area, pause/resume/finish), the
  second a `#[path]` child module of the first, as `validate_tests_fields.rs` is.
- **Same validation path as a plan.** Every added, split-in or amended task goes through
  `resolve_task_lenient` with the run's profile, limits and roster, then gets its branch,
  worktree and decision 56 notes as `build_run` gives them. The whole list then goes
  through `validate_tasks` with the batch's `touched` set and the caller's `EditScope`.
  `touched` is the added tasks, the split-in tasks, every amended task and every
  `add_dep` target. `answer`, `cancel_task`, and dependents rewired by a split are not
  touched.
- **Refusal errors.** A per-state refusal is a `PlanError` with rule `13`, `task` set,
  an **empty `field`**, and the whole sentence as `message`. `PlanError`'s `Display` now
  prints the message alone when `field` is empty, so the brief's
  `task t1 is working; route can be amended only on pending, queued or blocked tasks`
  is the displayed line. Blocked states are named `blocked(<reason>)`. The other
  refusal texts are this task's:
  - `task <id> is <state>; <field> can be amended only on pending, queued or blocked
    tasks`, one error per field. The restricted fields are `route`, `test_mode`,
    `test_mode_reason` and `size`. The reason goes with the mode, because decision 13
    lists "test mode and reason" as one amendable item.
  - `task <id> is <state>; only unfinished tasks can be amended`, once per edit.
  - `task <id> is <state>; only unfinished tasks can be cancelled`.
  - `task <id> is <state>; only pending, queued or blocked tasks can be split`.
  - `task <id> is <state>; dependencies can be added only on pending, queued or blocked
    tasks`.
  - `task <id> is <state>; only blocked(question) or working tasks can be answered`.
  - An unknown id gives `task <id>: task_id: no such task`, and a split with an empty
    `into` gives `task <id>: into: at least one task is required`.

  All edits are applied first, in order; validation runs after, so the error list is
  the refusals and resolution errors in edit order, then the cross-task errors.
- **Amend.** It sets the spec fields. When `route`, `test_mode`, `test_mode_reason` or
  `size` changed, the derived fields (size, hub, test mode, notes, review level, route,
  review route, budget) are re-resolved from the spec, never below the engine's raises
  (superseded in part by fix round 1, F1). Otherwise only the spec changes, so a rung-3 L size stays and the L rule
  fires once the task is touched (decision 13's exemption ends). `Deliver
  { amend_message }` goes out when `brief` or `acceptance` changed on a task with a live
  worker: its state is `working`, or it has an unended worker round. A priority change
  alone delivers nothing.
- **Cancel.** `CancelLive` is emitted for a task in `preparing`, `working`, `proof`,
  `check`, `review` or `merge_queue`, or with any unended round (for example a
  `blocked(question)` task whose session is still open). The task leaves `merge_queue`.
  Every unfinished task that **declares** it as a dependency becomes
  `blocked(dep_cancelled)` with the text `dependency <id> was cancelled` (this task's
  wording). Implicit dependencies are recomputed after every batch (fix round 1, F2). Removing the
  worktree of a cancelled task that is not live (a pre-warmed or blocked task) is the
  engine's work (M8a.11/decision 14), keyed on `cancelled` plus an existing worktree.
- **Split.** Dependents that are not finished have `task_id` replaced in place by the
  children, in order, without duplicates. Then the original is cancelled, which by now
  blocks nobody. The children go in right after the original, so the cancelled task
  stays in the list. Children do not inherit the original's deps; the planner gives
  them.
- **`add_dep`** on a `queued` task whose new dependency is not `merged` puts the task
  back to `pending`, because it is no longer runnable. Test:
  `add_dep_returns_a_queued_task_to_pending` (added beyond the brief's list).
- **`answer`** returns `blocked(question)` to `working` (block cleared) and emits
  `Deliver { answer_message(text) }`. Whether the session is still open or has to be
  resumed is decided by the engine (decision 29, M8a.12).
- **`pause`, `resume`, `finish`** only yield their consequences, in batch order, and
  leave the model unchanged. The run-state checks (for example pausing a run that is not
  running) belong to the engine (decisions 37 and 45).
- **History.** Each change adds a `TaskEvent` stamped with `now`: `added by a plan
  edit`, `cancelled by a plan edit`, `blocked: dependency <id> was cancelled`,
  `split into …`, `split from <id>`, `amended: <fields>`, `dependency on <dep> added`,
  `answered`. `revision` is not bumped here; decision 47's bump is the engine's.
- **TDD.** All 15 tests were written against `todo!()` stubs of `apply_edits`,
  `answer_message` and `amend_message`, and all 15 failed (`0 passed; 15 failed`, each
  panicking with "not yet implemented"). Mutations then showed that the behaviour each
  test pins really is pinned:
  - no rewire on split: `split_rewires…` fails;
  - `Display` without the empty-field case: 7 tests fail;
  - always re-resolving on amend: the rung-3 test fails;
  - `is_live` ignoring rounds: `cancel_of_a_working…` fails;
  - no queued→pending: `add_dep_returns…` fails;
  - `merge_queue` kept: `cancel_of_a_working…` fails;
  - answer not returning to `working`: `answer_only…` fails;
  - `add_dep` not touching: `dep_on_a_cancelled…` fails;
  - every task treated as touched: 4 tests fail, the rung-3 exemption test among them.

  After each run the files were restored from a scratch copy.
- **Decision 2's grep** over `edits.rs` and `contract.rs` matches only their doc
  comments.

### M8a.6 fix round 1 (2026-09-23)

Coordinator rulings on `task-6-review.md`, findings F1–F9:

- **F1 (fix, Critical).** An amend never lowers a task below an engine raise. `reresolve`
  resolves the **unamended** spec to find the plan's part. Anything the task holds
  beyond it belongs to the engine:
  - A size larger than the planned one (rung 3) is a floor (superseded by fix round 2,
    N1: the floor is now the recorded `Task.raised_size`, not inferred), and the amended spec is
    resolved at `max(spec size, floor)`. So an amend of route, `test_mode` or
    `test_mode_reason` alone on a rung-3 L task still meets rule 7.2.4. An explicit
    smaller `size` does not lower the floor either: a rung-3 L task can only be split
    (consistent with `run retry`'s `task <id> is L; split it first`).
  - A route different from the planned one (rung 2) is kept unless the amend names
    `route`. When the route is kept, `review_route` is re-picked from it.
  - Notes the planned resolution does not produce (the engine's) are kept after the
    newly resolved ones.
  - The stored `spec.size` stays what the amend says.

  Tests: `amend_route_on_a_rung3_l_task_is_rejected` (route, test_mode, reason, and
  size M), `a_rung3_raise_to_m_survives_a_test_mode_amend`,
  `an_escalated_route_survives_a_test_mode_amend`.
- **F2 (fix).** After every batch `apply_edits` recomputes `implicit_deps` for every
  task, then runs `combined_cycles` when nothing else failed, the same backstop
  `build_run` runs.
  - **The reviewer's case is accepted, not rejected.** `t1` owns `crates/a/**`, `t2`
    owns `crates/a/src/**`, and the batch is `add_dep t1 t2`. Recomputing with the
    reachability skip (task 5's F1) drops `t2`'s implicit wait on `t1`, because `t1`
    now declares `t2`. So `t1` waits for `t2`, and there is no cycle.
  - **The backstop can never fire through `apply_edits`, for the same reason it cannot
    in `build_run`.** An edit can therefore never leave a combined cycle behind.
  - **`implicit_deps` is now started-aware, per decision 41.** A task that has not
    started waits for an overlapping one that has, whatever their plan order. A task
    has started when it is in `preparing` through `merge_queue`, or `blocked` with a
    `start_commit`. Two unstarted tasks keep the plan-order rule; two started tasks wait
    for nothing. At plan time nothing has started, so `build_run` is unchanged.
  - **The recompute is required because split children are inserted mid-list.** With
    the plan-order rule alone, a `working` task would be made to wait for a split child
    placed before it.

  Tests: `edits_recompute_implicit_deps`,
  `an_added_task_waits_for_an_overlapping_earlier_task`,
  `a_split_child_before_a_working_task_waits_for_it`. M8a.11's scheduler should reuse
  `implicit_deps`.
- **F3 (fix).** The cancelled-dependency rule applies only to dependencies the batch
  adds, recorded as `(task, dep)` pairs:
  - `add_dep`'s pair;
  - every declared dep of an added task;
  - every declared dep of a split-in task.

  New entry point: `validate_tasks_with(tasks, touched, Some(&added_deps), …)`.
  `validate_tasks` is `validate_tasks_with(…, None, …)`, which keeps the old behaviour
  (every dep of every touched task) for `build_run` and task 5's tests. The L and area
  rules still apply to every touched task.

  Test: `a_dep_cancelled_task_stays_editable`. It covers a priority amend, `add_task t1b`
  plus `add_dep t2 t1b`, and a new dependency on the cancelled task, which is still
  refused.

  The lack of `remove_dep` (so a `dep_cancelled` task never becomes runnable except by
  splitting it) is in the followups file under milestone 9.
- **F4 (tests).** Pinning tests, each shown to kill its surviving mutant:
  - `cancel_is_live_in_every_live_state`: all six live states yield `CancelLive`;
    `pending`, `queued` and `blocked` yield nothing; cancel clears `block`.
  - `amend_delivers_only_to_an_open_worker_round`: a `blocked(question)` task with an
    open worker round gets `Deliver`; a `review` task with an open reviewer round does
    not.
  - `dependencies_are_never_duplicated`: covers both `add_dep` and the split dedupe.
  - `edits_write_task_history`: covers every history line.
- **F5 (carry note, addressed to M8a.11).** Cancelling a task with no live session
  emits **no consequence**. This covers a `blocked` task after rung 3 (decision 38 keeps
  its worktree, with real commits), a pre-warmed `pending` or `queued` task (decision
  14), and any cancelled task whose worktree exists. The engine must therefore look for
  `state == cancelled` with an existing worktree after every applied edit. It must then
  salvage the committed and uncommitted work (decision 20's
  `refs/anthrex/salvage/<run>/<task>/<seq>`) and remove the worktree. Suggested M8a.11
  test: `cancel_of_a_rung3_blocked_task_salvages_its_worktree`.
- **F6 (fix).** `dep_cancelled` wins over an existing block: it is the permanent
  condition, and a dependent of an unmerged task has never started. The dependent's
  earlier block is kept in its history line:
  `blocked: dependency <id> was cancelled (was blocked(<reason>): <text>)`. Test:
  `a_cancel_records_the_dependents_earlier_block`.
- **F7 (followup).** `EditScope::Area` restricts only `owns`. This is recorded in the
  followups file under milestone 9.
- **F8 (fix).** An `amend_task` with every field unset is refused with
  `task <id>: amend_task: nothing to amend` (rule `13`). Test:
  `an_empty_amend_is_refused`.
- **F9 (no change).** Split children go in right after the cancelled original, as the
  first M8a.6 notes say.

Red first: with the new tests added to `edits_tests_state.rs` and nothing else changed,
the run gave `19 passed; 9 failed`. The 9 were the F1 (3), F2 (3), F3, F6 and F8 tests.
The F4 pinning tests passed against the existing code; after the fixes, 14 single-line
mutations were run, and each one turned at least one test red:
- `has_live_worker` ignoring rounds;
- `has_live_worker` counting any role;
- `is_live` reduced to `working | merge_queue`;
- cancel keeping `block`;
- no split dedupe;
- no `add_dep` duplicate check;
- no `added` history;
- no `split from` history;
- no size floor;
- escalated route dropped;
- engine notes dropped;
- no implicit recompute;
- `implicit_deps` not started-aware;
- empty amend accepted.

Each mutation was restored with `git show HEAD:<path> > <path>` from a WIP commit, which
was folded into the fix commit afterwards.

### M8a.6 fix round 2 (2026-09-23)

Coordinator rulings on `task-6-rereview.md`:

- **N1 / F1 (fix, Critical).** The rung-3 floor is no longer inferred by comparing
  `Task.size` with what `spec.size` resolves to. That inference was lost after
  `[size=L, size=S]` in one batch or across two, because the first amend makes the spec
  agree with the raise. `model::Task` gains `raised_size: Option<Size>`
  (`#[serde(default)]`, `None` from `resolve_task`). **M8a.12's rung-3 action must set
  it** to the raised size. Every re-resolving amend resolves at
  `max(amended spec.size, raised_size)`, and `spec.size` keeps what the amend asked for.
  The route and note inference (planned versus held) is unchanged: an amend naming
  `route` replaces it, and a later amend then sees the named route as planned.

  Tests: `a_rung3_l_task_cannot_be_lowered_by_two_amends` covers:
  - one batch;
  - two batches, the first of which the L rule refuses, so nothing changes;
  - a task whose spec already says L.

  `a_rung3_m_task_cannot_be_lowered_by_two_amends` covers one batch and two applied
  batches. The rung-3 fixtures now set `raised_size`.
- **N2 (fix).** After the implicit recompute, `requeue_waiting` returns each `queued`
  task to `pending` when:
  - a declared dependency is not `merged`, or
  - an implicit dependency is neither `merged` nor `cancelled` (decision 41's release).

  This replaces `add_dep`'s own check. Tests:
  - `a_split_child_returns_an_overlapping_queued_task_to_pending` is the reviewer's split
    case, plus a queued task the edit leaves runnable, which stays queued.
  - `add_dep_returns_a_queued_task_to_pending` is kept.
- **N3 (pinning tests).** Each surviving mutant is now killed:
  - `review_route` is re-picked for a kept escalated route. `an_escalated_route_survives_a_test_mode_amend`
    now escalates to the Codex peer, as decision 39 does at effort high, so the reviewer
    for the kept route (Claude) differs from the planned one (Codex).
  - `new_tasks_on_a_cancelled_dependency_are_rejected`: `add_task` and split-child deps
    are recorded as added.
  - `a_blocked_task_with_a_start_commit_counts_as_started`: both directions of
    `has_started`'s blocked clause.
  - `two_started_tasks_never_wait_for_each_other`.
- **N4 (recorded, for M8a.11 and M9).** Two known cases, both harmless today:
  - **The implicit-dep reachability skip also follows declared edges through inactive
    (cancelled) tasks.** For example, `t4 -> t3 (cancelled) -> t5` suppresses the
    overlap wait between `t4` and `t5`. Today `t4` is then `blocked(dep_cancelled)` and
    can never run, so this does no harm. If M9 adds `remove_dep`, that edit must
    recompute implicit deps, which `apply_edits` already does after every batch.
  - **A started `blocked` task may declare, through `add_dep`, a dependency on an
    unstarted task it overlaps.** No implicit wait is then added in either direction,
    because that would be a deadlock. The unstarted task may run while the blocked
    task's worktree holds overlapping work: a merge-conflict risk, not a deadlock.
    M8a.11's merge queue handles the conflict.
    **Controller ruling after the second re-review (N5, Important): binding on M8a.11.**
    The re-review showed that "the merge queue handles it" is not enough on its own. An
    `answer`, a `run retry` or an accepted rung-3 rewrite could send the started task
    back to `working` while its new declared dependency is still running, and then two
    writers would work on overlapping `owns` at the same time. M8a.11 must therefore
    hold one invariant: **no task is dispatched, resumed or returned to `working` while
    any of its dependencies, declared or implicit, is unfinished.**
    - An `answer` to such a task is recorded and delivered only when its dependencies
      have merged.
    - When a started task resumes after a dependency merges, the run head is merged into
      its worktree first, by decision 36's hand-back path.
    - M8a.11 needs a reducer test for this: t1 is started and `blocked(question)`; t2 is
      unstarted and overlaps t1; the batch `[add_dep t1 t2, answer t1]` must leave t1
      waiting until t2 merges, then resume it with t2's changes in its worktree.
  - M8a.11 must also decide whether a pre-warmed task that blocks in `setup` has
    `start_commit` set (and so counts as started) or not.

Red first: with the new tests added and only the `raised_size` field plumbed in, the
run gave `31 passed; 3 failed`: the two two-amend tests and the queued split test. The
N3 pinning tests passed against the existing code. Each was then shown to kill its
mutant, restored with `git show HEAD:<path> > <path>` from a WIP commit that was folded
away afterwards:
- `review_route_resolved`;
- `add_task_deps_not_recorded`;
- `split_deps_not_recorded`;
- `blocked_not_started`;
- `blocked_always_started`;
- `both_started_plan_order`.

The fixes were checked the same way against their own mutants:
- the floor inferred again;
- no requeue;
- the requeue's implicit clause ignored.

### M8a.7 headless streams: parsers, status and the conversation mapping (2026-09-23)

Built `crates/daemon/src/headless/` (`mod.rs`, `argv.rs`, `claude_stream.rs`,
`codex_stream.rs`, `status.rs`, `conversation.rs`, each with its tests beside it, plus a
test-only `test_support.rs` holding the nine fixtures through `include_str!`). Moved
`TOOL_RESULT_SUMMARY_MAX`, `bound_tool_response` and `bound_tool_response_value` into
`proto::conversation` unchanged, with their private helpers; `truncate_to_char_boundary`
became `pub` so the parsers cut tool results with the same function. `hook.rs` calls them
from there and keeps its static assertion. `hook.rs` had no unit tests of its own; the
bounding's tests are `crates/cli/tests/hook_command.rs`, which drive the binary and all
still pass (20). `proptest = "1"` (1.11.0) is a workspace dependency and a daemon
dev-dependency. Every file in `headless/` passes decision 2's grep (no match at all).

**Where M8a.1's findings override the brief** (M8a.1 wins; each is tested):

- **A failed turn's category** is on the synthetic `assistant` line before the `result`
  (M8a.1's open question, "M8a.7 decides which"). Decided: the Claude parser is stateful.
  `claude_stream::ClaudeStream::parse_line(&mut self, line)` records an `assistant`
  line's `"error"` category and gives it to the next `result`; a `result` takes and
  clears it, so it never leaks into the next turn (`a_category_does_not_leak_into_the_next_turn`).
  `SessionEvent` is unchanged. `codex_stream::parse_line` stays a free function. The
  Interfaces `parse_line` line and the Claude `result` row now say so. M8a.17's driver
  keeps one `ClaudeStream` per Claude session.
- **The failed turn's text** is the `result` string, else the `errors` joined by `; `,
  else the subtype. A failed resume (`No conversation found with session ID: <id>`, item
  4) is `Failed { Other }` with that text. `sandbox required but unavailable` in the text
  gives `SandboxUnavailable` (item 4b).
- **Interrupted** is `subtype == "error_during_execution"` with `terminal_reason`
  `aborted_tools` or `aborted_streaming` (item 2).
- **`system/init` repeats every turn.** It yields `Init` and `TurnStarted`.
  `status::next` treats any `Init` as a turn opening (`Working`), never as `Starting`.
  `conversation::map` synthesises `SessionStart` only when the session id changes, so a
  Codex session does not get one per process either.
- **Unsolicited turns** (a background sub-agent finishing) are ordinary turns. Status
  opens and closes them. In the conversation, `StreamCursor` tracks whether the daemon's
  own latest turn is open. Top-level prose after its `TurnEnded` gets no
  `AssistantText` record, because it would otherwise be appended to the previous reply
  (`prose_of_a_turn_the_daemon_did_not_send_is_not_put_on_the_last_one`). This is the
  headless form of `450680e`'s fix on `main`.
- **Usage is per turn** (item 2), so `claude_usage_is_per_turn` asserts the pass-through
  values 26/477/66953/7175, 18/284/50323/526 and 10/215/25616/183.
- **`PermissionDenied.reason`** is `decision_reason`, else `message` (item 2, `dontAsk`).
- **The stream meta's `unmodelled` types** are recognised and yield `Other { kind }`
  (`system/<subtype>`, `rate_limit_event`, `control_response`), not `Unknown`. The test
  allows `Unknown` only for those types; `Other` keeps them out of the window's
  last-10-lines diagnostic ring, which is for lines nothing recognised.
- **Reviewers use `dontAsk`** (plan mode blocks the reviewer's MCP call in `-p`).
  `HeadlessSpec` gains `claude_disallowed_tools: Vec<String>`, emitted as
  `--disallowedTools <list>` right after `--allowedTools`. The brief placed it after
  `--permission-mode`, where it would be the last flag of a reviewer with no model and
  no effort flag; there every variadic flag is followed by a flag
  (`claude_variadic_flags_are_followed_by_a_flag`, over 48 argv). M8a.11's role
  launch fills the field with `Edit,Write,NotebookEdit` for a Claude reviewer.
- **The sandbox block needs `failIfUnavailable`** (item 4b). `SandboxKeys` gains
  `fail_if_unavailable: "failIfUnavailable"`, and the block is `{"enabled": true,
  "allowUnsandboxedCommands": false, "failIfUnavailable": true, "filesystem":
  {"allowWrite": [<git common dir>]}}`. `write_allow` is a dotted path into nested
  objects.
- **A sandbox refusal is not a denial** (item 4b): the sandbox fixture parses to a failed
  `ToolResult` and no `PermissionDenied`.
- **`codex exec resume` rejects `-s`** (item 6): `codex_resume_takes_sandbox: false`, and a
  resume passes `-c sandbox_mode="<mode>"` where the first turn passes `-s <mode>`.
- **Codex's default writable `/tmp` is kept.** M8a.1 excluded `/tmp` only so its
  recording proved something. The argv adds no `exclude_slash_tmp` or
  `exclude_tmpdir_env_var`, and a test asserts that.
- **Codex project config** (item 7a): `codex_loads_project_config: true`,
  `codex_project_config_paths: [".codex/config.toml", ".codex/hooks.json"]`,
  `codex_user_config_only: None`. `codex_args` inserts the flags right after `--json`
  when the caps name some; today it inserts none. The refusal itself is M8a.22's.
- **A Codex command that exits non-zero is not in `--json`**, and an interrupted Codex
  turn ends with no `turn.*` line (item 7). The parser cannot see either; the driver's
  `ProcessExited` covers the interrupt.
- **Codex `turn.completed.usage`**: `input_tokens` includes `cached_input_tokens` (Codex's
  own `non_cached_input` is the difference). So `TokenUsage.input = input_tokens -
  cached_input_tokens` and `cache_read = cached_input_tokens`; otherwise decision 40's
  `billable` would bill every cache read. `cache_write = cache_write_input_tokens`, and
  `reasoning_output_tokens` is part of `output_tokens`. The fixture turn is 12754 uncached
  plus 28416 cached input and 73 output.
- **Codex `turn.failed`** carries the API's JSON error as a string. The failure text is
  its inner `error.message` when it parses, else the raw message. `RateLimit` follows item
  7's rule: JSON `status == 429`, or the text contains `429`, `rate limit`, `usage limit`
  or `too many requests`, ignoring case. `item.type == "error"` items are notices
  (`Other { kind: "item/error" }`). A top-level `error` line is `Other { kind: "error" }`,
  and its first 300 characters are logged at `warn`.
- **`CLI_CAPS`** is M8a.1's table verbatim, plus `fail_if_unavailable`.

**Other choices the brief left open**

- `user_message(text, None)` omits `session_id`. Only the `Some` form is recorded; the
  engine always knows the id, because it passes `--session-id` or `--resume`.
- `interrupt_request` writes the recorded key order, with the id as a string.
  `user_message` also matches the recorded line byte for byte, not only as a JSON value.
- `Init.mcp_ok` is `None` when `mcp_servers` has no `anthrex` entry (a session launched
  without MCP), and `Some(status == "connected")` otherwise.
- Tool-result text is cut to `TOOL_RESULT_SUMMARY_MAX` (4096 bytes) for Codex too, as for
  Claude.
- The Codex item shapes M8a.1 could not trigger follow the Interfaces table and are
  tested from constructed lines: `mcp_tool_call` (result text parts joined; `error` or
  `status: "failed"` is a failed result), `file_change` (on completion only), and
  `web_search` (a `WebSearch` use and result on completion, so it is counted once whether
  or not a start line exists). `reasoning`, `todo_list` and unknown item types are
  `Other { kind: "item/<type>" }`.
- `status::next`: `Interrupted` ends a turn in `Idle`. A failed turn's `Attention` holds
  until the next `Init` or `TurnStarted`. `tool` is cleared when the turn ends. A
  sub-agent's `ToolUse` never sets it.
- `api_key_helper` goes into `--settings` only under `auth = "api_key"`, with `--bare`
  last. Under `login` a configured helper is not passed, since it could override the
  subscription login.
- `conversation::map` ignores its `runtime` argument: the table is the same for both, and
  `hooks_fire` carries the difference. `StreamCursor` drops its tool names at every
  `TurnEnded`, and holds at most 1024.
- **Codex item ids restart in every process** (`item_0`, `item_1`, … in each turn of the
  exec and resume fixtures), so one Codex session repeats ids across turns. M6.5's
  matching still works: `PostToolUse` matches within the open turn, and `ToolDetail` goes
  to the last call with the id. But no later task should key tool counts by id across
  turns; decision 27 already counts every `ToolUse`.
- M6.5 renders the table's `{"output": text}` object as its compact JSON in the tool
  call's one-line summary, as it does for Claude's own `{"stdout": …}`. This is recorded
  as a follow-up for M8c.

**TDD evidence.** Each module was first written as `todo!()` stubs of its public
signatures, then its tests. Every named test failed on the stub's panic: 10 Claude
parser, 4 Codex parser, 2 status, 5 conversation and 12 argv tests. Each then passed
once implemented. Two tests could not fail that way:
- `garbage_never_panics` and its two `proptest` cases were written after the parsers.
  Mutating `codex_stream` to `unwrap()` a missing `thread_id` and `usage` made the
  explicit test and the JSON-shaped property fail (minimal input `turn.completed`); the
  file was restored from a scratch copy.
- `toml_string_round_trips_through_the_toml_crate` tests M3's existing `toml_string`, so
  its 20-string half passes on `main`'s code. It failed first only through its tail,
  which parses `codex_args`' TOML values.

### M8a.7 fix round 1 (2026-09-23)

Review `.superpowers/sdd/M8a-orchestration-engine-core/task-7-review.md` (0 Critical,
3 Important, 10 Minor) and the controller's capture against codex-cli 0.156.1 (one
Critical). Each ruling, and what was done:

- **Ruling T7-C1 (Critical): the anthrex MCP server is `"approve"`, not `"auto"`.**
  - The controller's capture: `default_tools_approval_mode="auto"` with
    `approval_policy="never"` fails every MCP call with `MCP tool call requires approval,
    but approval policy is never`, in both sandboxes. `"approve"` completes it. As
    decision 25 stood, no Codex worker could ever call `task_done`.
  - `codex_args` now passes `-c mcp_servers.anthrex.default_tools_approval_mode="approve"`.
    Decision 25's text now says so, with the evidence.
  - Tests: `argv_builders`, and `the_anthrex_mcp_server_is_approved_under_never` (first
    turn and resume, worker and reviewer).
- **Ruling T7-I1: an unprompted Claude turn is counted.**
  - With `hooks_fire`, an `Init` that follows a `TurnEnded`, with no turn sent in
    between, is a turn Claude Code started by itself (a background sub-agent finishing).
    Its `UserPromptSubmit` hook builds a `User` turn for it, so `StreamCursor` now
    counts it as a prompt. Its prose still gets no record.
  - Without that count, the next sent prompt was checked against the unprompted turn.
    The view went `Misaligned` and every later reply was lost.
  - The session's first `Init` never counts (`after_turn_end` starts false).
  - Without hooks (Codex) nothing builds such a turn, so nothing is counted.
  - A requirement on M8a.17 and M8a.18: the driver must apply `sent_turn` before it
    writes the turn's message. Otherwise a fast `Init` could be taken for an unprompted
    turn.
  - Tests: `an_unprompted_claude_turn_keeps_later_turns_aligned` (the reviewer's three-turn
    case through a real `ConversationSet`: `degraded == None`, `reply two` on turn 5),
    and `only_an_init_after_a_turn_with_hooks_firing_counts_as_an_unprompted_turn`. Three
    mutants of the rule were killed.
- **Ruling T7-I2: `429` counts only as a token of its own**, or as the JSON `status`. It
  counts with no ASCII letter, digit or `_` on either side.
  - The ruling said "whole number". Digits alone let `a429` in a UUID segment through
    (the test caught it), so letters and `_` count as word characters too.
  - The phrases `rate limit`, `usage limit` and `too many requests` are unchanged.
  - Test: `a_429_inside_an_id_or_a_count_is_not_a_rate_limit`. `req_8429`, `142913`,
    a UUID and `4290` are `Other`; `HTTP 429`, `status 429.`, `429 Too Many`, `(429)` and
    a JSON `status: 429` are `RateLimit`.
- **Ruling T7-I3: real captures replace the documented Codex shapes.** Four codex-cli
  0.156.1 fixtures were added, each with a `.meta.json` (`observed: true`):
  - `codex-0.156.1-item-shapes.jsonl`, `-mcp-approval-auto.jsonl`,
    `-mcp-approval-approve.jsonl` and `-usage-limit.jsonl`.
  - `codex_0_156_captures_parse_every_line` parses every line of them, none `Unknown`.
  - `recorded_item_shapes_map_as_the_table_says` checks the mapping:
    - The failed `mcp_tool_call` is a failed `ToolResult` whose text is the error message.
    - The completed one's text is `probe says: hi`.
    - `file_change` keeps its absolute path and kind.
    - The `web_search` item names `"id"` twice (`item_3`, then `exec-<uuid>`).
      `serde_json::Value` keeps the last key, so the line parses, and the use and result
      pair up on `exec-<uuid>`, with its query as input.
  - `a_usage_limit_is_a_rate_limit`: the usage-limit `turn.failed` is `Failed { RateLimit }`.
  - The constructed-line test stays for the shapes the captures do not cover.
  - The usage-limit text names its reset time. A follow-up asks M8a.12/M9.5 to mark the
    runtime unavailable until then rather than retry every `rate_limit_retry_secs`
    (ruling T7-C2).
- **The minors, all fixed:**
  - **M1: a failure category comes only from the top-level line of the same turn.** It
    is taken only from an `assistant` line whose `parent_tool_use_id` is null, and it is
    cleared at `system/init`. Test: `a_failure_category_comes_only_from_the_top_level_line_of_the_same_turn`.
  - **M2: Codex tool ids are namespaced `t<k>:<id>`** in hooks and records, `k` the
    latest sent turn's ordinal. Test: `a_repeated_codex_item_id_stays_on_its_own_turn`.
    The table test and the Codex session test now expect namespaced ids.
  - **M3: interrupted turns.** Recorded as a follow-up for M8a.18/M8c. The table is
    unchanged.
  - **M4: a known line type in a shape that yields no event is `Unknown`.** Test:
    `a_known_type_in_an_unknown_shape_is_unknown`.
  - **M5: a retry gives back the status it interrupted.** `HeadlessStatus.before_retry`
    holds it. Bookkeeping events (`Other`, `Unknown`, `StderrLine`, `Diagnostic`) no
    longer end a retry.
    - This narrows the brief's "any later event clears it". Claude emits
      `rate_limit_event`, `thinking_tokens` and hook lines constantly, and they say
      nothing about whether the retry is over.
    - The engine's own `rate_limited_until` (decision 27) is M8a.12's and is not
      affected.
    - Test: `a_retry_restores_the_status_it_interrupted_and_ignores_bookkeeping_lines`.
  - **M6: the seven surviving mutants are killed** (`n`, `v`, `x`, `ae`, `ag`, `aj`,
    `ak`), each re-run against the tests. The tests are `an_aborted_stream_is_an_interrupt`,
    `mcp_ok_reads_the_anthrex_server_status`, `tool_result_text_parts_are_joined_by_newlines`,
    `a_failed_mcp_call_is_failed_even_when_its_status_says_completed`,
    `codex_usage_counts_cache_writes_inside_input_and_never_goes_negative` (distinct
    non-zero fields; more cached than input) and the recorded `web_search` query.
  - **M7: Codex `cache_write` is 0.** Decision 40 defines Codex billable as input minus
    cached plus output. Cache writes are part of `input_tokens`, as cached reads are, so
    counting `cache_write_input_tokens` again would bill them twice.
  - **M8: documentation.** The Interfaces `HeadlessSpec` has `claude_disallowed_tools`.
    `SessionEvent` has `Diagnostic` and `HeadlessStatus` has `before_retry`. Decision
    54's reviewer bullet now says `dontAsk`. The report's line count is corrected.
  - **M9: the parser no longer logs.** Codex's top-level `error` line yields
    `SessionEvent::Diagnostic { text }`, a new variant. M8a.17's driver logs it and keeps
    it in the window's last-lines ring. This supersedes the M8a.7 note that it is
    `Other { kind: "error" }` and logged from the parser. Status and the conversation
    ignore it.
  - **M10: the moved bounding functions have unit tests in `proto`**
    (`crates/proto/src/conversation_bound_tests.rs`): the 3-byte back-off (4095), the
    4-byte mixed case (4093), string, object and array bounding, and the top-level
    rewrite.
    - They test existing code, so they passed at once.
    - Changing the back-off to step two bytes made two of them fail.

### M8a.7 fix round 2 (2026-09-23)

Re-review `.superpowers/sdd/M8a-orchestration-engine-core/task-7-rereview.md`: every
item addressed, except that I1 was partial (cases D and E) and a new N1 remained.

- **Ruling T7-N1: a Claude turn is the daemon's or Claude Code's own by its content,
  not by timing.**
  - **What is observable.** The stream does not echo the prompt the daemon sends. M8a.1's
    recordings run without `--replay-user-messages`, and the only top-level user text in
    `claude-2.1.278-stream.jsonl` is `[Request interrupted by user for tool use]`. The
    `UserPromptSubmit` hook's prompt is observable, and it is applied at the right moment:
    - Claude runs command hooks before it acts on a prompt.
    - `anthrex hook` waits for the daemon's ack.
    - `server.rs` acks a `HookEvent` only after `manager.handle_hook` returns.
    - So a turn's prompt is applied before any of its prose is read.
  - **The new function.** `headless::conversation::observe_hook(runtime, &hook, &mut cursor)`
    takes each real hook the manager applied.
    - A `UserPromptSubmit` is numbered by the count of prompts observed. That is exactly
      the count of `User` turns M6.5 builds from the same hooks, so the two cannot
      disagree.
    - It is the daemon's (`human: true`) when it equals a text the daemon sent and no
      earlier prompt matched. Equality is judged by M6.5's `conversation::align`,
      Exact for a typed prompt. `align` is now `pub(crate)`.
    - Otherwise it is Claude Code's own (`human: false`).
    - It returns the `UserText` record, with the hook's own text.
  - **Content mode.** It starts at the first observed prompt.
    - `sent_turn` then only remembers the text for matching.
    - Prose goes to the latest observed prompt's turn, which now includes an unprompted
      turn's own prose. Nothing is dropped.
    - After a `TurnEnded`, prose that arrives before any new prompt is dropped: it belongs
      to a turn whose prompt hook was lost, and M6.5 has no turn for it.
    - Sent texts left unmatched by a later match are discarded, since their hooks were
      lost. A prompt observed before its `sent_turn` is claimed by that `sent_turn`.
  - **Fallback.** The fix round 1 timing rule is kept for a cursor that is never fed a
    prompt hook, the only case where no text is observable.
  - **Binding for M8a.17.** The manager feeds every real hook of a headless window to
    `observe_hook`, under the same lock, right after `conversation_hook`, and keeps one
    cursor per window across resumes. This is written into M8a.17's Tests-first and
    Change text, with the named test `a_headless_claude_prompt_hook_feeds_the_cursor`.
  - **Tests.** All run through a real `ConversationSet` with real hook payloads
    (`conversation_content_tests.rs`). Each asserts that every reply is on its own
    prompt's turn and `degraded == None`:
    - `the_three_turn_case_places_every_reply` (the bg reply now lands on its own turn);
    - `a_background_turn_that_runs_before_a_delivered_turn_keeps_both_aligned`, the N1
      race;
    - `an_unprompted_turn_in_the_middle_of_a_sent_turn_keeps_both_aligned`, case D;
    - `an_unprompted_turn_first_in_a_session_keeps_later_turns_aligned`, case E;
    - `a_prompt_applied_before_its_sent_turn_is_still_aligned`, case F;
    - `prose_of_a_turn_whose_prompt_hook_was_lost_is_dropped_not_misplaced`;
    - `a_sent_prompt_is_human_and_an_unprompted_one_is_not`;
    - `without_a_hook_feed_the_timing_rule_still_applies`, the fallback.
  - **A finding about the test harness.** `ConversationSet::enrich` drops records for a
    window that has no conversation yet, so every session in these tests starts with the
    recorded `SessionStart` hook, as a real one does. Round 1's session test put the
    prompt hook before `sent_turn` and so never saw this.
- **The ordering carry is bound into M8a.18's task text.** It has a named test,
  `sent_turn_is_recorded_before_the_write`, and a Change bullet. The order is binding for
  Codex, where `sent_turn` synthesises the prompt hook. For Claude the content rule makes
  the order irrelevant (case F), and M8a.18's text says so.
- **The surviving `_` mutant.** `http_429` and `HTTP429` are now asserted to be `Other`.
- **Mutation.** Six mutants of the new rule and the `_` exclusion were run:
  - Five were killed.
  - The sixth, the `hook_feed` guard on the timing counter, was equivalent: that counter
    is not read in content mode. The guard was removed as dead.
  - Two of the five (dropping stale sent texts, claiming a late `sent_turn`) first
    survived and were then killed by assertions added to
    `a_sent_prompt_is_human_and_an_unprompted_one_is_not`.

### M8a.8 run git operations I: preflight, branches, worktrees, the write queue (2026-09-23)

Built `run/git/mod.rs` (preflight, `project_settings`, `protected_files`, `verify_done`,
`count_commits`, `diff_so_far`, the shared `Git` runner), `run/git/worktrees.rs`
(`create_run_branch`, `prepare_worktree`, `lock_worktree`, `prepare_review`) and
`run/git/queue.rs` (`GitQueue`). Deviations, resolutions and invented text:

- **`GitQueue::write` takes `Fn`, not `FnOnce`.** The interface block has
  `f: impl FnOnce() -> Result<T, String> + Send + 'static`, but decision 18 retries a
  lock failure up to five times, and the brief's own test calls one closure three and
  six times. The bound is `F: Fn() -> Result<T, String> + Send + Sync + 'static`; the
  closure is shared through an `Arc`. The repository's `tokio::sync::Mutex` is held for
  the whole write, retries and their back-off included, so a retried write keeps its
  place ahead of later writes to the same repository.
- **`verify_done` returns `run::git::DoneChecked`, not `OpResult`.** `OpResult` does not
  exist until M8a.11. `DoneChecked` has exactly `OpResult::DoneChecked`'s fields, and
  M8a.11 wraps it. `spill_exempt` is not a parameter, as in the interface block. The
  engine applies the override exemption.
- **What "commits" and a valid `red` mean.** `commits` counts `git rev-list HEAD ^<start>
  ^<run_head>`: the task's own commits. After a hand-back merge, the run head's merged
  commits are not the task's, but the merge commit is. `red_ok` is `Some(true)` only
  when `red^{commit}` resolves to one of those commits. So the start commit, a commit on
  another branch, a merged run-head commit and an unresolvable name all give
  `Some(false)`. `verify_done` makes six git calls at most: `HEAD`, that list,
  `status --porcelain -z --untracked-files=all`, `MERGE_HEAD`, the spill diff, and
  `red`, only when one is given.
- **The spill diff passes `--no-renames`.** A file moved out of `owns` then reports both
  its old and new paths, not just the new one. Protected paths come first (decision 56):
  a protected path that `owns` does not name literally goes into `protected_changed` and
  into nothing else, and a protected path that `owns` does name goes into nothing. The
  rest split per decision 55.
- **`Preflight.protected_files` is left empty by `preflight`.** `preflight(git, dir,
  timeout)` has no profile, and decision 56's list is the resolved profile's (built-ins
  plus config plus plan). The driver (M8a.22) calls `protected_files` with that list's
  `OwnsMatcher` and fills the field. The same goes for decision 53's refusal: the driver
  calls `project_settings` only when that decision applies.
- **The identity check is `git -c user.useConfigOnly=true var GIT_COMMITTER_IDENT`.**
  Evidence (git 2.50.1, macOS): with `GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1`
  and no repository identity, plain `git var GIT_COMMITTER_IDENT` exits 0 with the
  identity git invented from the login name and host name. With `user.useConfigOnly=true` it
  exits 128 with `no email was given and auto-detection is disabled`. Without the
  override, decision 17's check would never fire on a machine whose host name contains a
  dot.
- **Invented message:** when `project::detect_roots_with` reports `detection_failed` (git
  could not be started, timed out, or printed nothing parseable), preflight says `could
  not tell whether <dir> is a git repository: git did not answer; try again`, not `not a
  git repository: <dir>`, following M5's rule that "not a repository" is said only on
  git's own negative answer.
- **The git version check** reads the first whitespace token that starts with a digit
  (`2.50.1` in `git version 2.50.1 (Apple Git-155)`) and compares major and minor.
  `(found <v>)` names that token, or the whole line when no token parses.
- **Worktrees are locked on creation with `git worktree add --lock --reason "anthrex run
  <run>"`, not by a separate `lock` afterwards.** Decision 18 asks for the lock "right
  after creation", and this closes even that gap. `<run>` is read from the branch
  (`anthrex/<run>/<task>`). A reused worktree that is not locked is locked with
  `lock_worktree`, which treats `is already locked` as success.
- **Reuse, re-add, re-point.**
  - An existing registered worktree on another branch is refused with the invented `worktree
    <path> is not on <branch> (git lists <refs/heads/… or a detached HEAD>)`.
  - A registered worktree whose directory is gone is unlocked (git never prunes a locked
    entry), `git worktree prune`d, then re-added.
  - Re-pointing (decision 19) happens when the branch head differs from `from` and is an
    ancestor of it (`merge-base --is-ancestor`, exit 1 with empty stderr = no). It runs
    `git checkout -q -B <branch> <from>` in the worktree.
  - `create_run_branch` shares this code without re-pointing.
- **The brief's test says "after `git worktree remove --force`".** A task worktree is
  locked, and git refuses a single `--force` on a locked worktree. The test uses `git
  worktree remove --force --force`.
- **`prepare_review`** resolves both refs with `rev-parse --verify <ref>^{commit}`. An
  unresolvable ref fails with the invented `<ref> is not a commit`. It replaces the earlier
  round's worktree: unlock if locked, then `worktree remove --force`, or prune if its
  directory is gone. Then it runs `worktree add --detach <path> <head>`.
- **The patch and `REVIEW_DIFF_MAX`.**
  - `REVIEW_DIFF_MAX`, `clamp_diff` and `DIFF_CUT_MARKER` are added to `run/contract.rs`
    now, where the Interfaces put `REVIEW_DIFF_MAX`, because the git layer clamps.
  - The clamp keeps a head and a tail on character boundaries with the invented marker
    line `[anthrex: the middle of this diff was cut to fit]` between them. The whole is
    at most `REVIEW_DIFF_MAX` and at most 3 bytes short: the tail takes whatever the
    head's cut left.
  - **M8a.13 note:** `reviewer_prompt` receives a patch that is already at most
    `REVIEW_DIFF_MAX`, so its own `[diff clamped: …]` line fires only for a longer
    patch. To say that the git layer cut the diff, look for `DIFF_CUT_MARKER`.
  - `diff_so_far`'s patch (decision 30) is clamped by the same function.
  - Both diffs pass `--no-ext-diff --no-color`, whatever the user's config says.
- **`worktree::run_git_with_cap` is new** (`crates/daemon/src/worktree.rs`, outside the
  brief's file list). `run_git`'s 256 KiB stdout cap would fail `ls-tree -r` on any
  sizeable repository, and would fail a review diff larger than 256 KiB, instead of
  clamping it. `run_git` now delegates to `run_git_with_cap` with its old cap. Every
  `run/git` command uses 64 MiB, so it is still one `Command::new`, still with
  `--no-optional-locks`.
- **`project_settings`** lists the candidates with `git ls-tree -r -z --name-only
  <base_sha> -- <paths>`, and reads each tracked Claude settings file with `git cat-file
  blob <base_sha>:<path>`. A settings file "has hooks" unless it parses and its `hooks`
  is absent, `null`, `{}` or `[]`; invalid JSON counts as hooks. The result is sorted.
- **Test layout.**
  - `tests/run_git.rs` (12 tests), `tests/run_git_done.rs` (`verify_done` and
    `count_commits_and_diff_so_far`, split to stay under 600 lines) and
    `tests/run_git_env.rs` (the one environment test).
  - Shared helpers are in `tests/support/run_git.rs`: `TempRepo` with a
    repository-local identity, `commit_file`, `worktree_block`. `tests/support/mod.rs`
    gains `recording_git` and `TempRepo::with_prefix`.
  - The project-settings fixture uses `git add -f`, because a global excludes file
    (this machine's does) may ignore `.claude/settings.local.json`.
- **`engine_paths_with_spaces_and_unicode_work`** (Risks item 2, assigned to M8a.8)
  covers only what exists so far: preflight, the run and task worktrees, `verify_done`
  on a non-ASCII path, and the review worktree, all under `/tmp/ax run ü …`. **M8a.9
  must extend it** with the merge candidate and salvage.
- **Cost.** A git spawn on this machine takes about 0.1 to 0.2 s of wall time.
  `verify_done_reports_each_condition` makes about 150 git calls and takes about 15 s.
  `lock_errors_are_retried_then_surface` takes about 6.2 s by construction (the sum of
  `LOCK_RETRY_DELAYS_MS`). Neither asserts a wall-clock bound.

### M8a.8 fix round 1 (2026-09-23)

Review `task-8-review.md` found 4 Important and 10 Minor issues; controller rulings T8-I1 to I4 and T8-minors. All are fixed, test first.

- **I1: a diff of any size is clamped, never failed (ruling T8-I1).**
  - `subprocess::run_captured_head_tail` (new, in `subprocess/head_tail.rs`) keeps the first and last N bytes of stdout, drains the rest, and reports the total. There is no over-cap outcome.
  - `worktree::run_git_head_tail` wraps it.
  - `run::git::diff` reads the first and last `REVIEW_DIFF_MAX` bytes. That is enough, because `clamp_diff` never reads more than half its budget from the head, or more than its whole budget from the tail. When the middle was dropped, each end is decoded on its own, and a character cut at the inner edge falls in the part the clamp throws away.
  - `run_captured` and the new function now share one body, `capture`, with a stdout sink. It waits with `poll(2)` instead of a fixed 10 ms sleep. Measured: the fixed sleep read a 70 MB diff at about 5 MB/s (13 s per diff). With `poll`, a pipe is read as fast as git writes, and the whole test takes 2.8 s. `run` is untouched.
  - Test `a_diff_larger_than_any_capture_cap_is_clamped_not_failed`: a 70 MB file of 700 000 lines is generated at test time, and both `prepare_review` and `diff_so_far` return a clamped patch. Before the fix: `Err("git diff … failed: git produced more output than expected")`.
- **I2: the clamp's character boundaries (ruling T8-I2).**
  - Unit tests in `run/contract.rs` shift the input by 0 to 3 bytes over bodies of 3-byte, 4-byte and mixed characters, with several limits. Each asserts the output is at most the limit and at most 3 bytes short, the marker appears once, the head is a prefix of the input and the tail a suffix.
  - The integration test `clamped_review_diffs_cut_on_character_boundaries_at_every_offset` shifts the file name by 0 to 5 bytes over `世𝄞世` lines. It checks the whole contract against `git diff` itself, requires that no replacement character appears, and asserts that at least one offset really cut inside a character.
  - Mutants M3 (head cut not floored) and M4 (tail cut floored) are each killed by both tests. Both tests pass on the unmutated code, which was already correct.
- **I3: `GitQueue::write` is cancellation-safe (ruling T8-I3).**
  - The repository's lock is taken with `lock_owned`, and the guard and the whole retry loop move into a `tokio::spawn`ed task that the caller awaits. A caller that stops waiting, through a timeout, a `select!` or shutdown, no longer releases the repository while its git process runs. The write finishes unobserved.
  - A caller that gives up while still waiting for the lock cancels nothing but its own place. Its write never starts.
  - Test `a_dropped_write_keeps_the_repository_until_it_finishes` uses a rendezvous: the second write records whether the first had finished when it started, then releases it. Before the fix it failed with "the second write ran while the dropped first one still held the repository".
- **I4: re-pointing a pre-warmed worktree (ruling T8-I4, a clarification of decision 19).**
  - A branch with no commit of its own (the unchanged condition) is re-pointed with `git checkout --force -B <branch> <from>`, then `git reset --hard <from>`, both with the write flags. Setup's edits to tracked files are dropped, and untracked build output is kept.
  - Deviation: the ruling's literal `checkout -B` fails before the reset can run. When the setup edit is to a file the run head also changed, git refuses with `Your local changes … would be overwritten by checkout`. `--force` is what lets the reset be reached. With `--force`, the reset is a no-op kept to match the ruling.
  - Tests:
    - `re_pointing_over_a_setup_edit_the_run_head_also_changed`. Before the fix, the checkout error above.
    - `re_pointing_drops_a_setup_edit_the_run_head_did_not_change`. Before the fix, `status` showed `M setup.cfg`: the leak.
- **Carry for M8a.11: re-run setup after a re-point.** After a re-point, the caller must run the profile's `setup` again, because the pre-warmed setup ran against the old tree. It recognises a re-point as `prepare_worktree` returning a `HEAD` different from the pre-warmed branch's start (`base_sha`).
- **Minor 5: protected matching is case-insensitive (ruling T8-minors).**
  - `run::globs::ProtectedMatcher` (new) builds decision 56's patterns with `globset`'s `case_insensitive(true)`, under the same rules as `OwnsMatcher` otherwise. `verify_done` and `protected_files` now take `&ProtectedMatcher`: an interface change, a distinct type so a caller cannot pass a case-sensitive matcher.
  - `names_literally` stays exact. `owns` naming `AGENTS.md` does not allow a new `agents.md`, which errs toward a bounce.
  - Tests: `agents.md` and `.Claude/settings.json` land in `protected_changed`; `lib/agents.md` and `notes/Claude.MD` are listed by `protected_files`.
- **Minor 6: no user diff config reaches an agent.**
  - Every diff, stat and name list passes `--no-color --no-ext-diff --no-textconv`, and every patch also passes `--src-prefix=a/ --dst-prefix=b/`. `--default-prefix` needs git 2.41, and runs need only 2.38.
  - There is no `git log` call in `run/git` yet. M8a.9's `commits_since` should pass `--no-color` too.
  - Test `diffs_ignore_the_users_colour_prefix_and_textconv_config` sets `color.ui=always`, `diff.noprefix=true` and a `.gitattributes`-selected textconv. Before the fix, the textconv ran and the header lost its prefix.
- **Minor 7: a working-tree rename counts once.** `parse_status` checks both status bytes for `R`/`C`, so ` R new\0old` skips its source path. Test: in `verify_done_edge_cases`, a `mv` plus `add -N` gives `dirty_tracked == 1`. Mutant M2 is killed.
- **Minor 8: the mutation survivors are tested.**
  - M1: the hand-back case now asserts `commits == 2`.
  - M5 and M6: a worktree deleted with `rm -rf` while locked is re-added.
  - M7 and M8: a reused unlocked worktree is locked again, and `lock_worktree` twice succeeds. These are in `a_task_worktree_deleted_by_hand_or_unlocked_is_restored_and_relocked`.
  - M10: renames into and out of `owns` are in `verify_done_edge_cases`.
  - Each mutant was applied, the tests were seen to fail, and the file was restored with `git show HEAD:<path>`. All seven are killed.
  - M17 (canonicalizing `git_common_dir`) is **equivalent**, recorded rather than tested. `git rev-parse --path-format=absolute --git-common-dir` already returns a resolved path, even when `.git` is a symlink to another directory: checked by hand on git 2.50.1.
- **Minor 9: a user branch shaped like the run.**
  - Before `worktree add -b`, each parent of the new branch name is checked. A user branch `anthrex/<run>` now gives the invented `branch anthrex/col1 exists, so git cannot create anthrex/col1/integration; rename or delete anthrex/col1`, not git's ref-lock error.
  - **Carry for M8a.11 and M8a.22:** decision 15's run-id redraw should also count `refs/heads/anthrex/<id>` as taken.
- **Minor 10: counts and hand-over after a hand-back (interface change).**
  - `count_commits` and `diff_so_far` gain a `run_head` parameter, after `start`:
    - `count_commits` counts `HEAD ^start ^run_head`, as `verify_done` does.
    - `diff_so_far` diffs `<run_head>...HEAD`: the start commit until a hand-back, then the merged run head.
  - **Carry for M8a.11:** `OpKind::CountCommits` and `OpKind::DiffSoFar` need `run_head`.
  - Test: `count_and_diff_so_far_leave_out_a_merged_run_head`. Before the fix, `(4, h)` where `(2, h)` was wanted.
- **Minor 11: `HEAD` off the task branch.**
  - `DoneChecked` gains `head_branch: Option<String>`, the short name of the branch `HEAD` is on, or `None` when detached. It is read in the same call as `HEAD`: `rev-parse HEAD --symbolic-full-name HEAD`. `verify_done` stays at six calls.
  - **Carry for M8a.11:** reject a claim whose `head_branch` is not the task's branch. Proposed text, invented: `task_done rejected: HEAD is not on <branch>; commit your work on <branch> and call task_done again`.
- **Minor 12: lock contention matches git's own message.**
  - Retry needs `.lock': File exists`, or `Unable to create '<file>.lock'` with the `.lock` inside the quoted file name. It no longer fires on `Unable to create` and `.lock` anywhere in the error, which a path echoed in the failure could supply.
  - Test `only_gits_own_lock_message_is_retried`. Before the fix, 6 calls where 1 was wanted.
- **Minor 13: bare repositories.**
  - Preflight in a bare repository says `anthrex runs need a working tree; <dir> is a bare repository` (invented). It is detected with `rev-parse --is-bare-repository`, only after M5's detection gave git's own negative answer.
  - A linked worktree of a bare repository still gets `project == root`. That is M5's `detect_roots` behaviour, unchanged, and `git_common_dir` is correct there.
- **Minor 14: the queue's key.** `GitQueue` is documented as keyed by the path as spelled, which is `Preflight.project` (canonical), and as never pruned (a handful of repositories). No blocking `canonicalize` runs on a tokio worker.
- **Implementer concern 3, carried for M8a.22.** The driver must call `protected_files` with the resolved profile's `ProtectedMatcher` before the plan warnings (decision 17). `preflight` leaves the list empty.
- **Test layout.**
  - `tests/run_git.rs` now holds preflight, settings and protected files (6 tests).
  - New `tests/run_git_worktrees.rs` has 9 tests: the run and task worktrees, the re-point cases, the parent-branch message, hooks and signing, and paths with spaces and Unicode.
  - New `tests/run_git_review.rs` has 4 tests.
  - `tests/run_git_done.rs` has 4 tests.
  - `tests/run_git_env.rs` has 1 test.
  - `tests/subprocess.rs` gains `head_tail_keeps_both_ends_of_a_large_output`. It was written after `run_captured_head_tail` (the I1 integration test was the failing one); it passed on first run, after one fixture fix: macOS `seq` prints `2e+06`, so it uses `awk`.
- **File splits.**
  - `run/git/mod.rs` reached 628 lines, so the done check moved to `run/git/done.rs`.
  - `subprocess.rs` reached 615, so the head-and-tail capture moved to `subprocess/head_tail.rs`.

### M8a.9 run git operations II: merge candidate, CAS, hand-back, salvage, finish (2026-09-23)

Built `run/git/merge.rs` (`merge_tree`, `commit_tree`, `materialize`, `cas_update`,
`reattach`, `read_ref`, `guard_refs`, `commits_since`, `hand_back`, and the types
`CandidateStep`, `RefCheck`, `AcceptOutcome`, plus `ACCEPT_LIST_MAX` = 50) and
`run/git/salvage.rs` (`salvage`, `remove_worktree`, `delete_branches`, `accept`).
Deviations, resolutions and invented text:

- **`merge-tree` passes `-z`, and is read with the head-keeping capture.**
  - Without `-z`, git quotes a non-ASCII conflicted path (`"\303\274 x.txt"`), so the
    file list handed to the worker would be wrong. With `-z` and `--no-messages` the
    output is `<tree>\0<file>\0…`.
  - `run_git`'s capped capture drops stdout on a non-zero exit, but a conflict is exit 1
    with its answer on stdout. The first implementation failed the conflict case with
    `git merge-tree … failed: ` and an empty stderr.
  - `merge_tree` therefore reads through task 8's `run_git_head_tail`, via the new
    `Git::read_head(dir, args, head_bytes)`, which keeps stdout on failure. It keeps
    the first 64 MiB. Past that, the last, possibly cut, name is dropped.
  - Exit 1 is not told apart from other failures by code, because `GitOutput` has no
    code. A failure with a tree on stdout is a conflict, and a failure without one is
    an error. A bad name prints only to stderr.
- **`salvage`'s and `accept`'s status checks keep no output.** Both use `read_head(…,
  0)` and test `total > 0`, so a worktree with a vast untracked tree is salvaged, not
  failed over the 64 MiB cap. `salvage` passes `--untracked-files=normal` explicitly,
  because a user's `status.showUntrackedFiles=no` would otherwise hide a lone new file
  and the worktree would be removed unsalvaged. A test covers it.
- **Salvage refs are never overwritten.**
  - `update-ref <ref> <commit> ""` fails if the ref exists.
  - A salvage whose ref already exists and holds the same tree returns `Some(ref)`:
    this is the replay of an op interrupted after its `update-ref`.
  - A salvage whose ref holds a different tree is refused with the invented `salvage ref
    <ref> already holds other work`. The caller should have taken the next `<seq>`.
  - `salvage` returns the ref name, not the commit, so it maps directly onto
    `OpResult::Removed { salvage_ref }`.
- **`remove_worktree` removes unconditionally.** It unlocks if locked, runs `worktree
  remove --force` (one `--force` is enough once unlocked), then `worktree prune`.
  Decision 20's "never deleted dirty without a salvage ref" is the op's ordering:
  `salvage`, then `remove_worktree`. A worktree git no longer lists, or whose directory
  is gone, is not an error, so the op can be replayed.
- **`cas_update` takes the short branch name** and writes `refs/heads/<branch>`. git's
  refusal wording varies by version, so on failure the ref is read back. `false` is
  returned when it is not at `old`, including when it is missing. If it is at `old`,
  the failure is an error.
- **`reattach` is `git checkout -q --force <branch> --`.** The `--` keeps a branch name
  from being read as a path. `materialize` is `checkout -q --detach --force <commit>`
  then `clean -q -fd`, and ignored build caches stay.
- **`guard_refs` invented text:** a deleted run branch halts with
  `refs/heads/anthrex/<run>/integration was deleted`, mirroring the base's `was
  deleted`. Decision 21 gives only the "moved from … to …" form. `<old7>`/`<new7>` are
  the first seven characters of the full sha, not `rev-parse --short`, which may be
  longer.
- **`commits_since`** runs `git log --no-color --no-show-signature --abbrev=7
  --format='%h %an: %s' --max-count=<limit> <from>..<to> --`, and gets the total from
  `rev-list --count`. `--no-show-signature` keeps a user's `log.showSignature=true` from
  running gpg and adding lines.
- **`accept`.**
  - The merge names `refs/heads/<run branch>`, so a tag of the same name cannot be
    merged instead.
  - Invented text: `check out <base> in <root> first (currently a detached HEAD)` when
    `root` is detached.
  - A failure with unmerged paths is aborted and returns `Conflict`. Any other failure
    is aborted too if `MERGE_HEAD` exists, then returned as git's error: for example,
    untracked files that would be overwritten.
  - The `expected_base` check and the merge are two commands. A user who commits in
    `root` between them gets that commit merged onto. This is not guarded: the window
    is milliseconds, and it is the user's own checkout.
  - `accept` passes no decision 18 flags, through the new `Git::user_write`.
- **`delete_branches`** lists `refs/heads/<prefix>/` with `for-each-ref`, and deletes
  each ref with `update-ref -d <ref> <listed sha>`, a CAS delete. A prefix that is
  empty after trimming `/` is refused with the invented `refusing to delete branches
  under an empty prefix`, because it would name every branch. Mutant M7, which drops
  the trailing `/` from the pattern, is equivalent: `for-each-ref` already matches whole
  path components, so `anthrex/db01` never matches `anthrex/db010/…`.
- **Writes and the queue.** These functions are blocking, as task 8's are.
  - The writes are `commit_tree`, `materialize`, `cas_update`, `reattach`, `hand_back`,
    `salvage`, `remove_worktree`, `delete_branches` and `accept`. The op executor
    (M8a.11/M8a.14) must run each through `GitQueue::write`.
  - The reads, which may bypass the queue, are `merge_tree`, `read_ref`, `guard_refs`
    and `commits_since`.
- **`worktrees.rs`'s `listed`, `forget_missing` and `is_ancestor` became `pub(super)`**
  for reuse; `LARGE_OUTPUT_BYTES` became `pub(crate)`.
- **Test layout.** The brief says to add to `tests/run_git.rs`, splitting into
  `run_git_merge.rs` past 600 lines. All of the new tests would take it well past 600,
  so they went into two new files:
  - `tests/run_git_merge.rs` (8 tests: merge-tree, commit-tree and CAS, materialize and
    reattach, both hand-backs, `read_ref`, `guard_refs`, `commits_since`);
  - `tests/run_git_finish.rs` (10 tests: three salvage, remove, five accept,
    `delete_branches`).
  - Extra tests beyond the brief: `salvage_of_a_conflicted_hand_back_keeps_the_markers`,
    and the `showUntrackedFiles=no` case in `salvage_of_a_clean_worktree_writes_nothing`.
  - `engine_paths_with_spaces_and_unicode_work` now also runs `merge_tree`,
    `commit_tree`, `materialize`, `cas_update`, `reattach`, `salvage` and
    `remove_worktree` under `/tmp/ax run ü …`. `merge_tree`'s own test has a
    conflicted non-ASCII file.

### M8a.9 fix round 1 (2026-09-23)

Review `task-9-review.md` found 2 Important and 6 Minor issues. Controller rulings
T9-I1, T9-I2 and T9-m1 to m6 apply. Each behaviour fix has a test that failed first.

- **I1: accept never merges over, or aborts, the user's own operation (ruling T9-I1).**
  - Before anything else, `accept` reads `rev-parse --absolute-git-dir`, which is `root`'s
    own git directory, including for a linked worktree. It refuses when any of these
    exists there: `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `rebase-merge/` or
    `rebase-apply/`.
  - The refusal is the invented `a <merge|cherry-pick|revert|rebase> is in progress in
    <root>; finish or abort it first`.
  - It runs first because a rebase detaches `HEAD`, and "a rebase is in progress" is more
    useful than "currently a detached HEAD".
  - Any `MERGE_HEAD` seen afterwards is accept's own, so accept only ever aborts its own
    merge.
  - Test `accept_refuses_while_a_merge_cherry_pick_or_rebase_is_in_progress`. It covers a
    `merge --no-commit -s ours` state with a clean porcelain status, a stopped
    cherry-pick, and a stopped rebase. Before the fix: `left: Err("git merge … failed:
    fatal: You have not concluded your merge (MERGE_HEAD exists)…")`. The old code then
    ran `merge --abort` and destroyed the user's pending merge.
- **I2: a merge that fails in any way, including a timeout, is aborted (ruling T9-I2).**
  - `ACCEPT_MERGE_TIMEOUT` = 600 s is accept's own deadline for its `git merge`, because
    the user's hooks and signing run inside it. Every other command in accept keeps
    `git_timeout`.
  - Test seam: `accept_with_merge_timeout(…, merge_timeout, timeout)`. `accept` calls it
    with the constant.
  - When the merge returns `Err` (timed out, or could not start), or fails, accept aborts
    any `MERGE_HEAD` with `merge --abort` under a fresh `git_timeout` deadline.
  - If the abort fails too, for example on a leftover `index.lock`, the error is the
    invented `<root> is mid-merge: accept's merge failed (<cause>) and could not be
    aborted (<abort error>); run git merge --abort in <root>`.
  - Test `accept_merge_that_outlives_its_deadline_is_aborted`. A `commit-msg` hook writes
    a marker and sleeps 30 s. A local `core.hooksPath` is set so a global one cannot hide
    it. The merge deadline is 2 s. Before the fix: `MERGE_HEAD` was left behind ("the
    half-done merge was aborted"). After it: no `MERGE_HEAD`, the original porcelain
    status, and `main` unchanged.
  - Git 2.50.1 does run `commit-msg` for a merge, which the marker shows. The runner kills
    the whole process group, so the hook dies with git and leaves no `index.lock`.
- **m1: the first parent is checked after a successful merge (ruling T9-m1).**
  - `accept` reads the run branch's head before merging. After success, `HEAD` must be a
    merge whose parents are `(expected_base, run head)`.
  - A merge whose second parent is the run head but whose first parent is not
    `expected_base` is undone with `git reset -q --keep HEAD^1`. That keeps the commit
    that landed on the base, and refuses to lose local changes. Accept then returns `the
    base branch moved again; run accept again`.
  - A `HEAD` that is not accept's merge means git found nothing to merge. It counts as
    accepted only when `HEAD == expected_base`, and is otherwise refused the same way,
    with nothing reset. This keeps `reset --keep` from ever undoing a commit that is not
    accept's.
  - No decision 18 flags are passed, and `reset` runs no hooks.
  - If the reset fails, the invented error names the merge commit `root` is on.
  - Test `accept_undoes_its_merge_when_the_base_moved_under_it`. A wrapper git (the new
    test helper `support::run_git::wrapper_git`) commits `sneak` on `main` just before
    the `--no-ff` merge runs. Before the fix: `left: Ok(Merged { … })`. After it: the
    error above, `main` at `sneak`, whose parent is `base`, and a clean tree.
  - The review's optional `ORIG_HEAD` pre-merge guard was not added. The ruling asked
    only for the first-parent check.
- **m2: `delete_branches` passes `--no-deref` and skips checked-out branches (ruling
  T9-m2).**
  - Each delete is `update-ref --no-deref -d <ref> <sha>`, so a symbolic ref under the
    prefix is removed itself, never its target.
  - Branches that `worktree list --porcelain -z` shows checked out in any worktree are
    skipped.
  - **Interface change:** the return type is now `Result<Vec<String>, String>`, the
    skipped branches' short names, for the report.
  - Test `delete_branches_skips_a_checked_out_branch_and_never_follows_a_symref`. Before
    the fix, `refs/heads/main` was deleted through `refs/heads/anthrex/sy01/link`.
- **m4: a refused salvage leaves the index untouched (ruling T9-m4).**
  - When the salvage ref already exists, `salvage` decides before `add -A` whether this
    is a replay: no `ls-files --others --exclude-standard`, and `git diff --quiet <ref>
    --` clean. It returns the ref if so and refuses otherwise, and in both cases touches
    no index.
  - A first salvage's `add -A` made every untracked file tracked, so a real replay
    matches.
  - Test `a_refused_salvage_leaves_the_index_untouched`. A hand-back conflict meets a
    `<seq>` ref that holds other work. Before the fix, `diff --diff-filter=U` was empty
    after the refusal ("the conflict is still unresolved").
- **m6: tests that kill the three surviving mutants (ruling T9-m6).** Each test passes
  on the current code and was seen failing under its mutant:
  - `hand_back_blocked_by_an_untracked_file_is_an_error` (H1);
  - `accept_merges_the_run_branch_not_a_tag_of_the_same_name` (A4), with a tag
    `anthrex/tg01/integration` on a decoy commit;
  - `salvage_never_overwrites_a_ref_created_under_it` (S1), where a wrapper git creates
    the ref just before salvage's `update-ref`.
  - C1, `cas_update` turning a ref-at-old failure into `Ok(false)`, needs lock contention
    to provoke and is left as the review accepted it.
- **m5, documented, no code change (ruling T9-m5).** `merge_tree`'s conflict list has two
  known limits:
  - Past 64 MiB of NUL-separated names, the rest is dropped silently, along with the last,
    possibly cut, name. The list the worker gets is then short.
  - A file name that is not UTF-8 is decoded lossily, with `U+FFFD`, so the worker is
    handed a name that does not exist.
  - Both are far-fetched. Every constructed case in the review parsed correctly.
- **m3, a follow-up, no code change (ruling T9-m3).** A git repository nested in a
  worktree is salvaged only as a gitlink, and its content is lost when the worktree is
  removed. This is recorded under "From M8a.9's review" in the foundation follow-ups
  file.
- **Test layout.** The accept tests moved from `run_git_finish.rs` to a new
  `tests/run_git_accept.rs` (9 tests), to keep both files under 600 lines.
  `run_git_finish.rs` now has 8 tests and `run_git_merge.rs` has 9.

**Later change, from M8a.11 fix round 2 (ruling T11-N1(a)).**

- **Later change, from M8a.14 fix round 1 (ruling T14-C1): `hand_back` returns
  `HandBack { onto, head, files }`** instead of the file list. `onto` is the worktree's
  `HEAD` before the merge (the task branch tip it merged onto), `head` the `HEAD` after
  it (the merge commit when clean, `onto` when conflicted), `files` the unmerged paths.
  The engine re-queues a hand-back only when `onto` is the claimed commit. New real-git
  test `hand_back_reports_the_tip_it_merged_onto` (a commit after the claim is the
  reported tip); the existing hand-back tests assert the new fields.
- **Later change, from M8a.14 fix round 2 (ruling T14-R2, N4): `onto` is read after
  the merge.** A clean merge reports `onto` as `HEAD^1` of the merge commit it made, so a
  commit that lands between a read of `HEAD` and the merge cannot be reported as the
  merged-onto tip. A conflicted merge reports `onto = head = HEAD`, read after it. Test
  `hand_back_reports_the_tip_it_actually_merged_onto` in the new
  `tests/run_git_handback.rs` (a wrapper `git` commits a "sneak" right before
  `merge --no-ff`).
- **New read `resolution_only(git, worktree, head, onto, run_head, files, timeout)`**
  (`git/resolution.rs`, ruling T14-R2 #3). True only when `head` is a merge commit
  whose parents are exactly `(onto, run_head)`, in that order (`rev-list --parents -n
  1`), and whose tree differs from git's own automatic merge of the two (`merge-tree
  --write-tree --name-only --no-messages -z onto run_head`, the first NUL field, conflict
  markers included) only in `files` (`diff --no-renames --name-only -z`). Tests
  `resolution_only_accepts_a_pure_conflict_resolution` and
  `resolution_only_refuses_a_claim_that_carries_more`: an extra feature commit after
  the merge, a merge that also changes a non-conflicted task file, a merge that undoes
  the run side's auto-merged file, a plain non-merge commit, the resolution squashed
  into a single-parent commit, and the resolution's tree on swapped parents. A
  `rev-list <head> ^onto ^run_head` check was written first and dropped: the parent
  check implies it (its mutant survived).
- **Later change, from M8a.14 fix round 3 (ruling T14-R3).**
  - `DIFF_FLAGS` gains `--ignore-submodules=none`, so neither `diff.ignoreSubmodules`
    nor a `.gitmodules` `ignore = all` hides a gitlink change from `verify_done`'s
    owns, protected and spill split, from `resolution_only`, or from any other run diff
    (stat, patch, salvage's `--quiet`, `unmerged`). Tests
    `verify_done_sees_gitlinks_that_config_ignores` (`run_git_done.rs`: a gitlink moved
    outside owns and one at a protected path under `.gitmodules` `ignore = all`, a new
    one under `diff.ignoreSubmodules=all`) and
    `resolution_only_sees_gitlinks_that_config_ignores` (`run_git_handback.rs`).
  - The shared scrub (`subprocess::scrub_git_env`) sets `GIT_NO_REPLACE_OBJECTS=1`: no
    engine git call reads `refs/replace` objects. The scrub is the one every git call
    goes through, and the setup, check and proof shells (`run/exec.rs`) inherit it too.
    Tests `resolution_only_ignores_replace_refs` (an evil merge behind a replace ref to
    the pure one stays refused, while plain git reads the replacement's tree) and
    `every_run_git_call_passes_no_optional_locks_and_no_git_env` (every recorded call
    has the variable).
  - `hand_back` reports `onto = HEAD^1` only when `HEAD^2` is the run head; otherwise
    (the run head already in the task's history, "Already up to date", no merge commit
    made) `onto = head = HEAD`. Test `hand_back_of_a_run_head_already_merged_reports_head`
    (a plain claim, and a claim that is itself a merge).
- **`hand_back` refuses a merge already in progress.** If `MERGE_HEAD` already exists in
  the task worktree (`rev-parse -q --verify MERGE_HEAD`), `hand_back` returns `Err("a
  merge is already in progress in <worktree>; finish it or run git merge --abort")` and
  does not run `git merge`.
- **Why.** Before this, git refused the second merge ("unmerged files"), and `unmerged`
  then reported the earlier merge's files. The result was `Ok(old files)`: it looked like
  a new conflict, and the newer run head was never merged. The engine now turns this
  error into `blocked(environment)`.
- **New write `abort_merge(git, worktree, timeout)`.** It runs `git merge --abort` with
  decision 18's flags, and does nothing when there is no `MERGE_HEAD`. The engine uses it
  to undo a conflicted hand-back while another dependency is unfinished.
- **Tests** (in `run_git_merge.rs`, which now has 11):
  - `hand_back_while_a_merge_is_in_progress_is_an_error`: probe q1's shape, with
    `MERGE_HEAD` still on the first run head.
  - `abort_merge_undoes_a_conflicted_hand_back`: `HEAD`, the file and a clean status are
    back, and a second call is a no-op.

### M8a.10 shell execution: setup, check and the test proof (2026-09-23)

Built `run/exec.rs` (`run_shell`, `summary`, `ShellOutcome`, `CHECK_TAIL_LINES`,
`CHECK_SUMMARY_LINES`, plus `LINE_MAX_CHARS` = 300 and `OUTPUT_GRACE` = 1 s) and
`run/proof.rs` (`ProofOp`, `run_proof`, `proof_command`, `proof_pattern`), and one git
helper, `run::git::prepare_scratch`, in `run/git/worktrees.rs`. Deviations, resolutions
and invented text:

- **A separate process loop, not `subprocess::capture` (the reuse the brief expected).**
  `exec.rs` borrows `capture`'s `poll(2)` wait (`wait_readable`), `set_nonblocking` and
  `scrub_git_env` (now `pub(crate)`), but runs its own loop, because `capture` cannot
  give it three things without changing what its git callers get:
  - the exit **code**: a non-zero exit is `Outcome::Failed` there;
  - a **group kill whenever the command finishes**: `capture` kills the group only while
    the leader is still unreaped, so a shell that exits while a background process holds
    the pipe runs to the timeout and leaves that process alive;
  - **one merged pipe** read line by line into a bounded tail, not two byte buffers.
- **stdout and stderr share one pipe** (`std::io::pipe`, stable since Rust 1.87). The
  `2>&1` of decision 34 is kept, but the shell's own complaints (command not found, a
  syntax error in `check`) are written before that redirection applies. With one pipe
  they land in the tail in order. Test: `check_merges_stderr` asserts exit 127 and the
  command's name in the tail.
- **The whole group is killed when the command ends, not only on timeout** (invented,
  within decision 26's "they belong to no window"). A check that starts a server in the
  background would otherwise leak it for every retry. The leader is watched with
  `waitid(WEXITED | WNOHANG | WNOWAIT)` and reaped only after `killpg`. While it is an
  unreaped zombie, its pid, which is the group id, cannot be reused, so the kill can
  reach no other process. Test: `a_check_that_exits_leaves_no_background_process_behind`
  (the mutant that kills only a live leader leaves the `sleep` running).
- **`OUTPUT_GRACE` (1 s, invented).** Once the shell has exited, or its group has been
  killed, output is read for at most this long more, then the pipe is abandoned. So a
  process that escaped the group with `setsid` and still holds the pipe cannot hold the
  engine command. A timed-out command returns within `timeout + OUTPUT_GRACE`, and any
  command within `timeout + 2 × OUTPUT_GRACE`, plus the kill and the reap. The brief's
  3 s bound in `check_timeout_kills_the_process_group` is `1 s + OUTPUT_GRACE + 1 s` of
  slack, and the test asserts `timeout + OUTPUT_GRACE < bound`
  (docs/timing-budgets.md rule 1).
- **Kill on timeout is an immediate `SIGKILL` to the group**, as decision 34 says, with
  no `SIGTERM` step. The brief's `KILL_GRACE + 1 s` is only the test's bound for the
  background pid to disappear.
- **The tail.** Lines split on `\n`, and a last unterminated line counts. Each line is
  decoded lossily and cut to exactly 300 characters. At most `300 × 4` bytes of a line
  are held while it is read (enough for 300 characters of any width), so a 1 MB line
  costs nothing. The tail is joined by `\n` with no trailing newline. `summary` is the
  last 40 `\n`-separated lines of it.
- **A command that could not start** (bad `dir`, no `/bin/sh`) gives `ok: false, code:
  None, timed_out: false` with the invented tail `could not run /bin/sh in <dir>:
  <error>`.
- **Matching is per line and unanchored** (ruling T10-M3). A `test_passed` using `(?s)`
  or spanning lines can never match. `PASS {test}` also matches `PASS t_reset_other`
  for the test `t_reset`, so profile authors should prefer delimited patterns (`test
  {test} \.\.\. ok`, or `^PASS {test}$`). One trailing `\r` is dropped from each line
  before it is matched and stored, so a `$`-anchored pattern works on a CRLF runner's
  output. Tests: `one_trailing_carriage_return_is_dropped`,
  `proof_matches_an_anchored_pattern_on_crlf_output`.
- **`test_passed` is matched per line, as each line is read, against up to 64 KiB of
  that line, not against the 200-line tail.** A test runner prints its result line and
  then more, like `cargo test`'s summary, and a line longer than 300 characters would
  be cut before the name. `exec::run_matching` (`pub(super)`) is `run_shell` with a
  pattern. Unit test: `a_match_beyond_the_cut_counts`.
- **`ProofOp` is the interface block's, and its `command` and `passed` arrive already
  substituted,** as `OpKind::Proof`'s comments say. The substitution is two pure
  functions in `proof.rs`, `proof_command` (`{test}` becomes `launch::shell_quote(test)`)
  and `proof_pattern` (`{test}` becomes `regex::escape(test)`), for M8a.13's engine to
  call when it emits `Op Proof`. The tests use them too, so
  `proof_quotes_a_hostile_test_name` covers the real substitution.
- **`run_proof` returns `Result<ProofRuns, ProofError>`, not `OpResult`,** which does
  not exist until M8a.11 (the precedent is M8a.8's `DoneChecked`). `ProofRuns` has
  exactly `OpResult::Proof`'s fields. `ProofError::SetupFailed { output }` maps to
  `OpResult::SetupFailed`. `ProofError::Failed(message)` is a git step that failed, or
  a `passed` that is not a regex (plan validation compiles it with a stand-in name,
  so this is unlikely but possible for an odd name). It maps to
  the engine's generic failure.
- **The proof worktree** (`prepare_scratch`) is `git worktree add --detach <path> <red>`
  with decision 18's write flags. It is reused whenever git lists it and its directory
  exists, and re-added (after prune) when the directory is gone. It is never locked and
  never watched (decision 22). `setup` runs, at `red`, with the proof's env and
  `timeout_secs`, whenever the worktree's own git directory has no `anthrex-setup-ok`
  marker; the marker is written once setup succeeds (fix round 1, ruling T10-M1b,
  superseding the first version's removal of the worktree after a failed setup).
- **A failed `setup` writes no marker**, so the next proof runs `setup` again in the
  same worktree. That also covers a daemon that died mid-setup. Tests:
  `a_failing_setup_is_reported_and_runs_again_next_time`,
  `a_missing_setup_marker_runs_setup_again`.
- **Profile authors: setup output must be gitignored to survive** (ruling T10-M4).
  Each proof run starts with `checkout --detach --force` and `clean -fd`, which removes
  every untracked file that is not ignored, setup's included. That is decision 33's
  order.
- **The head run is skipped when the red run did not fail.** The proof has already
  failed, and the head run could cost a whole `check_timeout_secs`. `head_passed` and
  `matched` are then `false` and `head_tail` is empty. `proof_failed_message` (M8a.13)
  must quote `red_tail` in that case.
- **A red run that times out is not `red_failed`** (a timeout is no evidence that the
  test fails). Invented text: a timed-out run's tail gains a last line `[anthrex: timed
  out after <secs> s]`, for `red_tail`, `head_tail` and a setup's `output`, so the
  quoted tail says why. Test: `a_red_run_that_times_out_is_no_evidence_of_failure`.
- **Queue (corrected in fix round 1, ruling T10-I2).** `run_proof` must **not** run
  inside one `GitQueue::write`: it interleaves git writes with up to three shell runs of
  `check_timeout_secs` each, so it would hold the repository's write lock for the
  whole proof, and a lock-contention retry would re-run the tests. `run_proof` takes a
  `git_write: &dyn Fn(GitStep) -> Result<(), String>` hook, and only its git steps go
  through it: the worktree (`prepare_scratch`), the checkout before setup, and the red
  and head checkouts (`materialize`). Each `GitStep` is an owned `Box<dyn Fn() ->
  Result<(), String> + Send + Sync + 'static>` and idempotent, so it can be moved into
  `GitQueue::write` and retried. The shell runs, the `rev-parse --absolute-git-dir` read
  and the marker write happen outside the hook. **Carry for M8a.13:** run `run_proof`
  on `spawn_blocking` with the hook `|step| handle.block_on(queue.write(&root,
  step))`. Tests pass `proof::direct`.
- **Carry for M8a.11 (setup after a re-point):** the setup that M8a.8's carry asks to
  re-run is `exec::run_shell(worktree, setup, &profile_env(profile, worktree),
  check_timeout)`. A failure there is `OpResult::SetupFailed { output: tail }`.
- **Grep acceptance.** `grep -n "thread::sleep" crates/daemon/src/run` prints one line,
  `run/git/queue.rs:116`. It is inside that file's `#[cfg(test)]` module (M8a.8's
  `writes_to_one_repo_are_serialized`, which simulates a slow git write), not production
  code, and it was left alone as outside this brief. `exec.rs` has no `thread::sleep`.
  Its waits are `poll(2)` through `subprocess::wait_readable`, which sleeps only when
  no pipe is open.
- **Test layout.** `tests/run_exec.rs` has 14 tests: the brief's 10, plus
  `a_check_that_exits_leaves_no_background_process_behind`,
  `a_failing_setup_is_reported_and_runs_again_next_time`,
  `a_red_run_that_times_out_is_no_evidence_of_failure`, and the second worktree in
  `proof_runs_setup_once_per_new_worktree`. `tests/run_exec_env.rs` has the one
  environment test. That test also checks that an unrelated `ANTHREX_*` variable is
  kept, and it filters `env` through `grep` so that a long environment cannot push a
  line out of the 200-line tail. Unit tests: 5 in `exec.rs`, 2 in `proof.rs`.

### M8a.10 fix round 1 (2026-09-23)

Review `task-10-review.md` found 2 Important and 6 Minor issues. Rulings T10-I1, T10-I2
and T10-M1 to M5 apply. The notes above are corrected in place (proof worktree, setup
marker, Queue, matching). Each behaviour fix has a test that failed first.

- **I1: the timeout holds under an output flood.** `drain` took reads until
  `WouldBlock`, so writers that refilled the pipe kept it past the deadline. It now
  checks its caller's limit before every read and returns after at most 16 reads of
  64 KiB, so the loop re-checks the leader and the deadline. After the group kill,
  `OUTPUT_GRACE` is a hard cap on reading.
  - Test `check_timeout_holds_under_an_output_flood` runs 8 × `yes ''` with a 1 s
    timeout, bounded by `timeout + OUTPUT_GRACE + SLACK` (2 s). Before the fix: `a 1 s
    timeout under a flood returned after 13.908067166s`.
  - Test `output_grace_is_a_hard_cap_for_a_writer_that_left_the_group` runs a `setsid`
    `yes`, bounded by `2 × OUTPUT_GRACE + SLACK`. Before the fix: `an escaped flooder
    held the command for 22.302259292s`.
- **I2: the proof's git steps go through a caller-supplied hook**, not the whole proof
  through the queue. See the corrected Queue bullet. Test
  `proof_runs_only_its_git_steps_through_the_write_hook`: a hook that flags "inside"
  on disk while a step runs. Setup and the test script check the flag, and the hook
  counts 4 steps in the first round (worktree, checkout for setup, red, head) and 3 in
  the second. Before the fix: `left: 0` (the hook was never called).
- **M1:** `waitid` failing with anything but `EINTR`, `ECHILD` included, is now
  `Leader::Gone`. The loop stops waiting and skips both `killpg` and the reap, because
  the pid may already be reused. Unit test `a_pid_that_is_not_our_child_is_gone` (pid
  1). The mutant that maps the error to `Exited` fails it.
- **M1b:** the `anthrex-setup-ok` marker (`proof::SETUP_MARKER`), in the directory that
  `rev-parse --absolute-git-dir` gives in the proof worktree. Test
  `a_missing_setup_marker_runs_setup_again`. Before the fix: `no anthrex-setup-ok in
  …/.git/worktrees/t1.proof`. `run::git::absolute_git_dir` is new.
  `remove_worktree` is no longer called by the proof.
- **M2:** the tail test gains 😀×400, which must come out as exactly 300 characters
  and 1200 bytes. It passes on the unchanged code. Mutant `TAIL_LINE_BYTES =
  LINE_MAX_CHARS * 3` now fails it with `left: 225 right: 300`.
- **M3:** one trailing `\r` is trimmed (see above). Before the fix: `left:
  "a\r\nb\r\r\nc\r"`, and `matched: false` on `^PASS t_reset$`.
- **M4:** documented (setup output must be gitignored).
- **M5:** recorded in the follow-ups file under "From M8a.10's review".

### M8a.11 engine I: start, plan gate, scheduler and dispatch (2026-09-23)

Built the pure reducer `run/engine/` and its neighbours. Decision 2's grep over every new
file matches only doc comments.

**Module layout (two files beyond the brief's list, split by responsibility):**

- `engine/mod.rs`: `EngineState`, `Event`, `EventKind`, `AgentSignal`, `Effect`, `step`,
  and the routing of `OpDone` and `Signal`.
- `engine/ops.rs` (new): `OpKind` and `OpResult`, re-exported from `engine`. `mod.rs`
  was 637 lines with them.
- `engine/requests.rs`: start, approve, reject, edit (first part) and restore (first part).
- `engine/schedule.rs` (new): runnability, `critical_len`, dispatch order, slots, the
  critical path and waves. These are pure functions of a run, shared by
  `dispatch.rs` and `snapshot.rs`.
- `engine/dispatch.rs`: the scheduler's actions, the op results it handles, the pre-warm,
  the N5 resume and the F5 clean-up.
- `engine/outbox.rs`: queue and delivery gate.
- `run/snapshot.rs`, `run/role_launch.rs` and `run/messages.rs`.
- `contract.rs` is extended, with its new tests in `contract_tests.rs`.
- Tests: `engine/tests/{fixture,dispatch,dispatch_edits,dispatch_slots}.rs`. The brief's
  single `dispatch.rs` would have been 655 lines.

**Name corrections against the brief and Interfaces:**

- `run/contract.rs` already existed (M8a.6, plus M8a.8's clamp), so it is extended, not
  created.
- `toml_string` is M3's `crate::launch::codex::toml_string`.
- `OpKind::CountCommits` and `OpKind::DiffSoFar` gain `run_head` (M8a.8 minor 10).
- `OpResult::DoneChecked` gains `head_branch: Option<String>` (M8a.8 minor 11).
- `EventKind::Start.run` is `Box<Run>`, and `OpKind::CreateWindow.spec` is
  `Box<HeadlessSpec>`, because of clippy's `large_enum_variant`. `Effect` allows that
  lint instead: boxing every `OpKind` (about 256 bytes) would buy nothing in a short-lived
  effect list.
- `HeadlessSpec` gains `Eq`, so `Run`, which now holds `PendingOp { kind: OpKind }`, keeps
  its `Eq`.
- `run::model` gains `From<ClaudeAuth> for config::ClaudeAuth`. The spec carries the config
  type, and `RunLimits` carries the serde mirror.
- The fixture has no `fx.status(window, Status)`, because no event carries a window
  status: the engine's input is `Signal`.
- The fixture's base is `b0` × 20, as the brief says. `run/test_support.rs` uses `b` × 40.

**Model additions** (each `#[serde(default)]`):

- `Run.pending_ops` and `PendingOp`, which M8a.5 deferred to here.
- `Task.worktree_live`: the worktree exists. It is set when a `PrepareWorktree` succeeds or
  its setup fails, and cleared by `Removed`.
- `Task.awaiting_deps`: the N5 hold.
- `RunLimits.api_key_helper`, filled from `[orchestrator.claude]`. A session spec is built
  from the run alone, and decision 50 passes the helper.

**Readings and choices:**

- **Revision (decision 47).** Each step compares every run with its state before the event.
  A changed run gets `revision += 1`, the global revision rises by one per changed run, and
  `Persist` is placed first and `Publish` last. A new run keeps the revision 1 that
  `build_run` gave it; the global revision still rises. For now every `Persist` is
  `urgent: true` and every `Publish` is `structural: true`. M8a.12 separates counter-only
  changes (decisions 43 and 47).
- **Pre-warm (decision 14).**
  - Only a task in `queued` with no declared and no implicit dependency is pre-warmed, and
    at most `max_writers` pre-warms are held at once (a pending one counts).
  - A pre-warmed task stays `queued`, holds no writer slot, and has not started.
  - `prewarmed` means "created from `base_sha` and setup succeeded".
- **Dispatch (decisions 19 and 41).**
  - The task goes to `preparing`, which takes the writer slot.
  - If the task is pre-warmed and `run_head == base_sha`, `CreateWindow` is sent directly.
  - Otherwise `PrepareWorktree { from: run_head, setup: Some(..) }` is sent first.
  - A pre-warm still in flight at dispatch continues the dispatch when it returns.
  - `start_commit` is set when `CreateWindow` is sent. The state becomes `working` when
    `Window` comes back.
- **Carry T8-I4 (setup after a re-point) and RR1.**
  - The re-pointing `PrepareWorktree` carries `setup: Some`.
  - **Executor contract for M8a.22:** run setup (M8a.10's
    `exec::run_shell(worktree, setup, &profile_env(..), check_timeout)`) after every
    `PrepareWorktree` whose `setup` is `Some`. A failure is `SetupFailed { output: tail }`.
  - The engine sends a `PrepareWorktree` from the run head only for a task with no
    `start_commit` (never started). **Carry for M8a.15:** a resume or retry of a started
    task must pass `from = start_commit` (a reuse), never the run head.
- **N4 (M8a.6).** A task blocked in setup, pre-warm or dispatch, has no `start_commit`. It
  does not count as started, because no agent has written in its worktree. It is
  `blocked(environment)` with `setup failed:\n<output>` (invented).
- **Rounds.**
  - A round is created when `CreateWindow` is sent, with `turn_open: true`, `turns: 1` and
    `round == session`.
  - A Claude round's `session_id` is the uuid.
  - Worker names are `<h4>/<task>.w<session>`, and reviewer names `<h4>/<task>.r<round>`.
- **Window limit (decision 16).** It is checked at every `CreateWindow`, worker or reviewer,
  against `windows_created`.
- **Hub alone.**
  - A hub task that holds a writer slot also blocks reviewer dispatch ("blocks every
    dispatch", read literally).
  - A hub task waiting for slots to empty does not stop later non-hub tasks from taking
    them, so it can wait long behind a stream of small tasks. The decision gives no
    priority rule; noted for M9.5.
- **Reader slots.**
  - A reader slot is held from `PrepareReview` until the reviewer round ends.
  - `PrepareReview` then `CreateWindow` with `reviewer_spec` and `reviewer_prompt` is
    implemented because the slot test needs a real reviewer. `ReviewRecord`s, verdicts and
    nudges are M8a.13's.
  - A task still in `review` whose round ended gets a new round (decision 35). M8a.13
    decides when a task leaves `review`.
  - `PrepareReview.base_ref` is the task's `start_commit`.
- **Reviewer permissions.** M8a.13's test text says `claude_permission_mode == Some("plan")`.
  M8a.1 and M8a.7 moved reviewers to `dontAsk` with `Edit,Write,NotebookEdit` disallowed,
  and `reviewer_spec` does that. **M8a.13's test must follow M8a.7.**
- **Invented values.**
  - `critical_len` weights L as 3, like M. An L task never runs; the decision gives S and M
    only.
  - The critical path is the first unfinished task in dispatch order, then repeatedly its
    unfinished dependent with the longest chain.
  - `wave` counts merged dependencies and skips cancelled ones.
- **Reject.**
  - It is accepted only in `awaiting_approval`. Otherwise the reply is `run <id> is <state>;
    reject applies only while its plan awaits approval`.
  - `Discard` lists every task path, then the integration path, each with its next salvage
    ref (`…/integration/1` for the integration worktree).
  - The reply is immediate, and the run is `discarded` on `Finished`.
  - While a `Discard` or `Accept` is pending, the scheduler starts nothing.
  - A `CreateRunBranch` that fails, or whose setup fails, makes the run `failed`, with the
    reason as `outcome`.
- **Edits (first part).**
  - `CancelLive` sends `KillWindow` for every live round and marks it `retiring`.
  - `Deliver` queues the message, or holds it under N5.
  - `pause`, `resume` and `finish` do nothing yet (M8a.15).
  - The reply is `applied <n> edit(s)`. A refused batch replies with every error line.
- **Carry T6-F5.** After every step, a `cancelled` task with `worktree_live`, no unended
  round and no op in flight gets `UnwatchWorktree` then `RemoveWorktree` with
  `refs/anthrex/salvage/<run>/<task>/<seq>`. `Removed` records the ref. This covers a
  pre-warmed task, a rung-3 blocked task and a killed live one (after its
  `ProcessExited { killed_by_engine: true }`).
- **Carry T6-N5 (binding invariant).** Superseded by fix round 1 (ruling
  T11-I1..I3), below: the hold is task state set when the dependency is added, not when
  the answer arrives. Named test: `add_dep_then_answer_waits_for_the_dependency_then_hands_back`.
- **Signals (minimal).**
  - `Init` sets `session_id`. `TurnStarted` and `TurnEnded` open and close the turn.
    `ToolUse` counts. `ProcessStarted` sets the pid.
  - `ProcessExited { killed_by_engine: true }` ends the round.
  - Everything else only stamps `last_event`. M8a.12 owns decision 32.
- **Outbox.** (The retry delay is fix round 1's.)
  - A delivery goes to the task's current worker round. `Outgoing.window_id` is the window
    at queue time.
  - A round that has ended gets no `Deliver`. M8a.12 adds `ResumeSession` for it, plus the
    failure retry and block. For now `Delivered { ok: false }` only re-queues the messages
    and closes the turn.
- **Requests not built yet.** Retry, override, cancel, resume, finish and tools reply
  `Err("<what> is not available yet")`. `BaseAdvanced` is ignored.
- **Contracts and prompts.**
  - `worker_prompt` follows the Interfaces text exactly.
  - `handover_prompt`'s layout after the worker prompt is invented: `This is session <n> of
    this task.`, `Why a new session: <reason>`, the stat, `Diff so far:` clamped to
    `REVIEW_DIFF_MAX`, and `Earlier failures:`.
  - `reviewer_prompt`'s `[diff clamped: <n> bytes omitted…]` counts the bytes left out of
    the head and the tail.
  - `clamp_diff` is generalised to `clamp_with(text, max, marker)`. `messages::clamp` uses
    the invented marker `[anthrex: the middle of this message was cut to fit]`.
  - `summary` and `CHECK_SUMMARY_LINES` moved to the pure `messages.rs` in fix round 1,
    and `exec.rs` re-exports them.
- **Carry T8 (run-id redraw).** `plan::run_id_taken(id, refs)` is a pure predicate:
  `refs/heads/anthrex/<id>`, or anything under `…/<id>/`, takes an id.
  **Carry for M8a.22:** call it with `git for-each-ref refs/heads/anthrex/` output, beside
  the `<data_dir>/runs/<id>` check.
- **Passed on to M8a.12: the carry T8 (a `task_done` made off the task's branch).**
  `task_done` is M8a.12's. `OpResult::DoneChecked` carries `head_branch` for it, and the
  proposed text stands.
- **Carries T9 and T10 (`GitQueue`).** The reducer only sends ops. `Effect::Op` carries
  `run_id`, from which the driver finds the run's `project`, the queue key.
- **Concern for M8a.22 (ordering).** A `Discard` sent while a pre-warm `PrepareWorktree` is
  still running must execute after it. Both are writes on one repository's `GitQueue`, so
  the driver must queue them in emission order.

**TDD evidence.**

- 24 tests were written against `todo!()` stubs and empty contracts, and all 24 failed:
  `138 passed; 24 failed`, every failure a stub panic or the empty contract's assertion.
  They are:
  - the brief's 20 named tests;
  - the N5 test and the suggested F5 test
    (`cancel_of_a_rung3_blocked_task_salvages_its_worktree`);
  - 2 of this task's own, `a_setup_failure_blocks_the_task_as_environment_without_starting_it`
    and `reviewer_spec_is_read_only`.
- `a_run_id_is_taken_by_its_own_branch_or_anything_under_it` failed on its stub, then
  passed.
- `cancel_of_a_working_task_kills_then_removes_after_the_exit` was written after the code.
  It is shown to pin by mutation M11 below.
- One fixture defect was found and fixed: the stand-in for an approving review now moves
  the task to `merge_queue`. Before, the re-review decision 35 requires correctly
  re-dispatched `t1`.
- **Mutations**, each restored from a WIP commit (since folded): 19 run, 17 killed.
  - Killed:
    - the N5 dependency check, the hold and the hand-back;
    - the direct launch of a stale pre-warm;
    - `setup: None`;
    - both hub rules;
    - unlimited readers;
    - priority ascending, and no critical length;
    - a bump for an unchanged run;
    - the window limit off by one;
    - removal while a session is live;
    - an unlimited pre-warm;
    - waves that skip merged dependencies;
    - a Codex uuid;
    - the uuid version nibble.
  - Equivalent in reachable states, so they survive:
    - The pre-warm's implicit-dependency guard. A task with an unfinished implicit
      dependency is already `pending`.
    - Treating a cancelled implicit dependency as unfinished. `apply_edits` recomputes
      implicit dependencies without cancelled tasks, so one is never left listed.

**Gates.**

- `cargo build --workspace --all-targets`, `cargo clippy --workspace --all-targets -- -D
  warnings` and `cargo fmt --all --check` are clean.
- `cargo test -p anthrex-daemon`: every binary passes except `git_registry`
  (`a_real_write_triggers_a_probe`, `a_commit_in_a_linked_worktree_triggers_a_probe`) and
  `server_git` (`two_windows_in_one_worktree_register_once`).
  - Those three time out even run alone, in this worktree.
  - The same binaries pass, at both the base `57b19c6` and this task's code, from a
    scratch worktree under `/private/tmp`.
  - So they depend on the checkout's location or load (load average 20–30), not on this
    change, which touches neither the registry nor the server.

### M8a.11 fix round 1 (2026-09-23)

Review `.superpowers/sdd/M8a-orchestration-engine-core/task-11-review.md` found 3 Important
and 8 Minor issues. Rulings T11-I1..I3 and T11-minors apply. Each fix has a test that failed
first.

**I1–I3: the N5 hold is task state (ruling T11-I1..I3).** The review found three sequences
in which a started task went back to `working` with an unfinished dependency, or before its
hand-back. The hold now lives in the new `engine/holds.rs`:

- **The hold.** `enforce_holds` runs first in every scheduler pass. A started task
  (`start_commit` set, not finished) that has an unfinished dependency gets
  `awaiting_deps = true` as soon as it gains one, whether or not an answer exists.
- **While held.**
  - The task is never `working`. When `answer` un-blocks it, the engine sets it back to
    `blocked(question)` with `answered; resumes once <deps> merged`, or, once the
    dependencies are done, `answered; resumes once the run head is merged into its
    worktree`.
  - Its messages wait in the outbox, because a blocked task takes no delivery.
- **The hand-back.** `resume_held` sends exactly one `HandBack` when all of these hold:
  - every dependency is finished;
  - the task is `blocked(question)`;
  - a message is waiting for it (it was answered);
  - no `HandBack` of its is in flight.

  An unanswered held task is not handed back until its answer arrives. So the answer's own
  step carries the hand-back, and the delivery follows it.
- **The result** (`handed_back`):
  - Conflicted files queue `conflict_message(files)`.
  - If a dependency is still unfinished (one added while the merge ran), the hold is kept
    and another hand-back follows when it finishes.
  - Otherwise the hold is cleared, the task returns to `working` with no block, and its
    queued messages (answers, then the conflict message) go out as one turn joined per
    decision 29.
  - A failed hand-back is `blocked(environment)`, which `resume_held` does not retry.
- **Minor 8.** A `dep_cancelled` block replaces the hold: `awaiting_deps` is cleared. The
  held message stays queued, since `dep_cancelled` is permanent until M9's `remove_dep`.
- **Tests** (`engine/tests/holds.rs`, the review's probes r1, r2, r3 and r6):
  - `an_answer_after_the_new_dependency_merged_hands_back_first`;
  - `a_dependency_added_while_the_hand_back_runs_keeps_the_hold`;
  - `an_answer_while_the_hand_back_runs_waits_for_it`, which also asserts no second
    hand-back over three ticks and the joined `A`, `B` and conflict turn;
  - `a_cancelled_dependency_replaces_the_hold`.
- `requests.rs`'s `hold_or_queue` is gone: an edit's message is simply queued.

**Minors (ruling T11-minors):**

1. **Hub starvation.** No change. It stays recorded for M9.5; see the M8a.11 notes.
2. **Pre-warm places.** Only a pending pre-warm, or a pre-warmed task that is still a
   `queued` root, holds a pre-warm place. Test:
   `a_prewarmed_task_that_gains_a_dependency_releases_its_prewarm_place`.
3. **Unpinned guards.**
   - `a_cancel_waits_for_the_worktree_op_in_flight`: no removal while the pre-warm runs,
     and no second `RemoveWorktree` while the first runs.
   - `a_dispatch_overtaken_by_a_merge_is_prepared_again`: the `from == run_head` re-check.
   - The in-flight `HandBack` guard is covered by the I3 test.
4. **Reject and approve while discarding.** While a `Discard` (or `Accept`) is in flight,
   `reject` and `approve` reply `run <id> is being discarded` (or `accepted`), and no
   second `Discard` is sent. Test: `a_run_being_discarded_refuses_reject_and_approve`.
5. **Restore bumps the revision.** A restore that pauses a running run bumps its revision.
   Test: `a_restore_that_changes_a_run_bumps_its_revision`, which also checks that an
   unchanged `awaiting_approval` run keeps revision 1.
6. **Counter-only changes and the delivery retry.**
   - A step whose only change to a run is a round's `last_event`, `tool_calls` or `usage`
     gives `Persist { urgent: false }` and `Publish { structural: false }`. Any other change
     is urgent and structural. `without_counters` compares the run with those fields
     zeroed. Test: `counter_only_changes_are_neither_urgent_nor_structural`.
   - A failed delivery is retried no earlier than `DELIVERY_RETRY_SECS` later, through
     `AgentRound.delivery_retry_at`, a new `#[serde(default)]` field. `delivery_failures`
     counts, and a success resets both. Test: `a_failed_delivery_waits_before_it_is_retried`.
     The block after `DELIVERY_MAX_FAILURES` stays M8a.12's.
7. **`summary` moved.** `summary` and `CHECK_SUMMARY_LINES` moved from `exec.rs` to the
   pure `messages.rs`, and `exec.rs` re-exports them. The existing `exec` tests still cover
   them.
8. **The hold and `dep_cancelled`.** See I1–I3 above.

**TDD and mutation evidence.**

- With the tests added and no fix: `166 passed; 9 failed`.
  - The two guard tests (`a_cancel_waits_…` and `a_dispatch_overtaken_…`) passed, since
    they pin correct code.
  - Sample failures:
    - `left: [Ok("run engine-test-3f9a rejected; discarding it")] right: [Err("run
      engine-test-3f9a is being discarded")]`;
    - pre-warm `left: [] right: ["t0"]`;
    - restore revision `left: 2 right: 3`;
    - `urgent: true` where `false` was wanted;
    - the I2 test `left: Working right: Blocked`.
- 15 mutants were run against the new code; 13 were killed.
  - `G7`, the removal's in-flight guard, first survived: the test's worktree did not exist
    yet. The test was extended with the no-second-removal ticks, and the mutant is now
    killed.
  - `G3`, which drops the dependency re-check in `handed_back`, survives. It is
    equivalent: `enforce_holds` takes the hold back in the same step, before any delivery.
    The re-check is kept, as the ruling requires, as the first layer.
- After the fixes: `175 passed; 0 failed` in `run::`.

**The three integration tests that failed in this worktree** (`git_registry` ×2,
`server_git` ×1, M8a.11's gates):

- They failed only with this worktree's own `target/` (25 GB).
- They passed from the same Desktop worktree with a fresh `CARGO_TARGET_DIR` inside it, and
  from `/private/tmp`. So neither the Desktop path nor TCC is the cause.
- After `cargo clean -p anthrex-daemon`, which removed 268 792 files and 67.4 GiB, mostly
  incremental caches, they pass in the normal `target/`.
- The cause is stale build state for the daemon package in this worktree's `target/`. The
  exact mechanism was not pinned down. The tests themselves are fine and were not changed.

**Gates.** `cargo build --workspace --all-targets`, clippy with `-D warnings`, `cargo fmt
--all --check`, and `cargo test -p anthrex-daemon` all pass: 34 test binaries, 0 failures,
621 unit tests. `engine/dispatch.rs` passed 600 lines, so the hold code moved to
`engine/holds.rs` (134 lines).

### M8a.11 fix round 2 (2026-09-23)

The re-review (`.superpowers/sdd/M8a-orchestration-engine-core/task-11-rereview-1.md`)
confirmed I1–I3 and the minors. It found N1 (Important), and N2 and N3 (Minor). Rulings
T11-N1..N3 apply. Every fix has a regression test, taken from the re-review's probes
(q1, q7, q2b), that failed first.

**N1: a conflicted hand-back, then another one (probe q1).**

- **The problem.** A hand-back conflicted while a newly added dependency was unfinished.
  The markers and `MERGE_HEAD` stayed in the worktree. The second hand-back then got
  `Ok(the same files)` back from real git, so the newer run head was never merged. The
  worker was also sent the conflict message twice.
- **(a) Git layer.** `hand_back` now fails when a merge is already in progress. See the
  M8a.9 notes, "Later change". The engine's existing `Failed` path blocks the task on its
  environment.
- **(b) Engine.**
  - A conflict that arrives while the task is still held, with a dependency still
    unfinished, is not handed to the worker. `handed_back` sends `OpKind::AbortMerge {
    worktree }` (its result is the new `OpResult::MergeAborted`), keeps the hold, and
    queues no conflict message.
  - `AbortMerge` is a git write like the others: `git::abort_merge` under the repo's
    `GitQueue`.
  - `resume_held` sends no `HandBack` while a `HandBack` or an `AbortMerge` of the task is
    in flight.
  - Once every dependency is finished, the next hand-back brings the conflict again, and
    that single result queues the one conflict message the worker sees.
  - An abort that fails leaves the worktree mid-merge: `merge_aborted` blocks the task on
    its environment, and no hand-back follows.
- **Tests.**
  - `a_conflict_while_a_dependency_is_unfinished_is_aborted`: one `AbortMerge` on t1's
    worktree, the hold kept, no conflict message, no `HandBack` while the abort runs. After
    `MergeAborted`, exactly one `HandBack`. Its conflict gives `working` and one turn,
    `[answer A, conflict]`, with exactly one conflict message in the outbox.
  - `a_failed_abort_blocks_the_task_on_its_environment`.
  - `an_abort_result_for_a_cancelled_task_is_ignored`.
  - The I2 test now also asserts that a clean result sends no `AbortMerge`.

**N2: an amendment does not answer a question (probe q7).**

- **The problem.** A held task with an unanswered question was amended. The amendment's
  message started a hand-back, and the task came back `working` with its question never
  answered.
- **The fix.**
  - New `Task.held_answered` (`#[serde(default)]`). `enforce_holds` sets it when it
    re-blocks an answered, held task, and clears it along with the hold.
  - The hand-back still runs, as the ruling says.
  - A clean or conflicted result then clears the hold. An answered task resumes. An
    unanswered one stays `blocked(question)` with its original question text; the
    engine never rewrites that text for an unanswered task.
  - Its queued messages, the amendment and any conflict message, wait in the outbox and
    go out joined with the eventual answer.
- **Test.** `an_amendment_to_an_unanswered_held_task_waits_for_the_answer`: back on
  `which table?`, nothing delivered. The answer later gives `working` and one turn,
  `[amendment, answer]`.
- **Doc correction.** Fix round 1's note says an unanswered held task is not handed back
  until its answer arrives. That is only true when no other message waits for it: an
  amendment also triggers the hand-back.

**N3: a conflict whose hold is gone (probe q2b).**

- **The fix.** `handed_back` no longer ignores a result because the task is not held.
  Only a finished task is ignored: a cancelled task gets no conflict message.
  - A conflict is always processed: it is aborted per N1(b) if the task is still held
    with an unfinished dependency, and otherwise queued as the conflict message. It goes
    out when the task next takes messages, for example once a `dep_cancelled` block is
    resolved.
  - A `Failed` result still counts only for a held task, so it never replaces a
    `dep_cancelled` block.
- **Tests.**
  - `a_conflict_is_kept_after_the_holding_dependency_is_cancelled`: one conflict message,
    no abort, and `dep_cancelled` kept.
  - `a_failed_hand_back_keeps_a_dep_cancelled_block`.
  - `a_conflict_for_a_cancelled_task_is_dropped`.

**TDD and mutation evidence.**

- With the tests added and no fix, both git tests failed, as did the four engine tests
  (q1, failed abort, q7, q2b), each at its first assertion.
- 12 mutants were run; 11 are killed.
  - M9 (a `Failed` result blocks an unheld task) and M10 (no finished-task guard in
    `handed_back`) first survived. The two tests above were added, and both are now
    killed.
  - M7, which drops the `held_answered` reset on `dep_cancelled`, survives. It is
    equivalent: `enforce_holds` skips a `dep_cancelled` task, so it is never held again
    with the stale bit. The reset is kept as hygiene.
- `engine/holds.rs` is 178 lines, and `engine/tests/holds.rs` is about 510 (split in
  fix round 3).

### M8a.11 fix round 3 (2026-09-23)

Re-review 2 (`.superpowers/sdd/M8a-orchestration-engine-core/task-11-rereview-2.md`)
approved N1–N3 and found two minors, M1 and M2. Both are fixed. Each has a regression
test, from the re-review's probes p4 and p6, that failed first.

**M1: a failed abort replaced `dep_cancelled` (probe p4).**

- **The problem.** An `AbortMerge` was in flight when the holding dependency was
  cancelled, and then the abort failed. `merge_aborted` overwrote the `dep_cancelled`
  block with `environment`. On the next pass, `enforce_holds` held the task on the
  cancelled dependency, which can never finish.
- **The fix.** A failed abort keeps a `dep_cancelled` block, as a failed `HandBack` does.
  The failure is appended to the block text and recorded in the history, so the leftover
  `MERGE_HEAD` stays visible.
- **Test.** `a_failed_abort_keeps_a_dep_cancelled_block`: the block is still
  `dep_cancelled`, its text names the failure, and the task is not held after a tick.

**M2: an untold conflict in an unanswered task's worktree (probe p6).**

- **The problem.** An unanswered held task was amended, and its hand-back conflicted once
  every dependency was done. Per N2, the task went back to its question, with the
  markers and `MERGE_HEAD` left in the worktree and the conflict message undelivered. A
  new dependency then held it again. The next hand-back hit `hand_back`'s N1(a) error,
  and the task ended up `blocked(environment)`.
- **The fix.**
  - When `enforce_holds` newly holds a task, it checks for an undelivered conflict
    message for that task (`contract::is_conflict_message`, which matches the message's
    fixed first line). If there is one, the worker was never told of that merge.
  - The engine then emits `AbortMerge` for it and drops the message, the same treatment
    N1(b) gives a conflict during a hold. The next hand-back brings the conflict again,
    once.
  - `enforce_holds` now takes `fx`.
  - A conflict the worker was told of stays on N1(a)'s error, as the re-review requires.
    Its message is either gone from the outbox (delivery acknowledged) or has
    `delivered_at` set (delivery in flight), so it is never taken for an untold one.
- **Test.** `an_untold_conflict_is_aborted_when_the_task_is_held_again`:
  - one `AbortMerge` and no conflict message once the task is held again;
  - after `MergeAborted` and t3's merge, one `HandBack`;
  - its conflict queues exactly one message, and the task is back on `which table?` with
    the amendment still waiting.

**Test layout.** `engine/tests/holds.rs` would have reached 602 lines. Rounds 2 and 3's
conflict tests moved to a new `engine/tests/holds_conflicts.rs` (357 lines), and
`holds.rs` is now 260 lines. Its helpers are `pub(super)`.

**Mutation evidence.**

- 6 mutants were run; 4 are killed: the `abort_untold_conflict` call, the message drop,
  the `dep_cancelled` guard, and the conflict-only filter.
- Two survive, and both are unreachable in the current code:
  - **R4**, the "no `HandBack` or `AbortMerge` in flight" guard. A task cannot become
    newly held while one of its own hand-back ops is in flight, because it stays held for
    the whole op.
  - **R5**, the `delivered_at.is_none()` filter. A delivery in flight means the task was
    taking messages, not blocked, so no edit could hold it again.
  - Both guards are kept as defence for M8a.12, when workers' own signals can block a
    task mid-delivery.


### M8a.12 engine II: the done gate, turns, stalls, denials and budgets (2026-09-23)

Decisions 27 (engine side), 29 (the gate), 32, 38 (stall, budget and rung-4 parts), 40,
54 (the unavailable-sandbox block), 55 and 56 are in the reducer. Decision 2's grep over
every new file matches only doc comments.

**Module layout (beyond the brief's list, split by responsibility):**

- `engine/done.rs`: `Event::Tool` (the MCP section's engine-side acceptance),
  `task_done` and `task_blocked`, `VerifyDone`'s verdict, and the turn-end fallback.
- `engine/tools.rs` (new): the worker tools' argument checks, which kept `done.rs` under
  600 lines.
- `engine/signals.rs` (new): `on_signal`, moved out of `mod.rs`, with every decision-32
  session rule, plus `watch`, the watchdog each scheduler pass runs.
- `engine/ladder.rs`: gate failures, stalls, hard breaches, rungs 1 to 4, budgets, and
  rung 2's fresh session (`start_fresh_sessions`, `fresh_diff`). It is `pub(crate)` so
  `snapshot.rs` shares `round_spend` and `total_spend`.
- `engine/outbox.rs` gains resume-on-delivery, the `DELIVERY_MAX_FAILURES` block, the
  failed turn's wait, and `ResumeSession`'s result.
- `engine/dispatch.rs`'s `launch_worker` is split so `launch_fresh` shares it. The
  session number is known before the first turn is built, so the hand-over prompt says
  `session <n>` correctly.
- Tests: `engine/tests/{done,done_tools,turns,turns_holds,budgets}.rs`. The brief's
  `done.rs` would have been 655 lines, and `turns.rs` plus the budgets well over 600.

**Model additions** (each `#[serde(default)]`):

- `AgentRound.turn_denials`, `last_denial`, `fallback_waiting`, `carried` and
  `failed_error`.
- `Task.claim: Option<PendingClaim { reply, claim: DoneClaim }>`, the claim whose
  `VerifyDone` is in flight.
- `Task.fresh_session: Option<FreshSession { reason, append }>`, a fresh session decided
  on and not yet started.

**Name corrections and interface readings:**

- `AgentSignal::TurnEnded.denials` stays a count (`u32`), as the Interfaces have it.
  *(Superseded by fix round 1, review m-4: it is now the tool names.)*
  The driver passes `permission_denials.len()`. Only the part not already seen as
  `PermissionDenied` events in that turn is added (`turn_denials`).
  - `denied_text`'s `last: <tool>: <reason>` comes from the latest `PermissionDenied`
    event.
  - A denial reported only in `permission_denials` has no reason in the stream. If one
    such denial reaches the threshold with no event before it, the text reads `a tool:
    reported in the turn's permission_denials` (invented). Claude sends both forms
    (M8a.1), so this is a fallback only.
- `INTERRUPT_GRACE` is the driver's `Duration`. The reducer uses
  `engine::INTERRUPT_GRACE_SECS = 30`, in the unix seconds it works in.
- **The unavailable sandbox before `Init`.** `AgentSignal` has no stderr, so the engine
  cannot classify a stderr line itself.
  - **Carry for M8a.22 (driver contract):** when the session's stderr holds the text
    M8a.1 records before any `Init`, send `TurnEnded { Failed { error, kind:
    SandboxUnavailable } }`, then the `ProcessExited`.
  - The engine then blocks the task on its environment with decision 54's text. The
    exit that follows is not resumed, because the task is no longer `working`. The test
    drives exactly that sequence with no `Init`.
- `Event::Delivered.error` is now read. It becomes the block text on the third failure.
- `generated_files_message` keeps the Interfaces' exact text, including the literal
  `<start>` in `git checkout <start> -- <files>`: the signature carries only the files.
  The worker prompt names the start commit.

**Readings and choices (invented where marked):**

- **The done gate.**
  - Rejection order: HEAD off the task's branch (carry T8, M8a.8's proposed text), no
    commit, dirty tracked files, a merge in progress, untracked files inside `owns`, the
    tdd rule, then a bad `red`.
  - The tdd rule applies only to an explicit `task_done`. A fallback claim goes on to its
    proof, which fails, as decision 32 says.
  - The engine re-filters `protected_changed` with `globs::names_literally` against
    `owns`. The git layer already does this, so it is defence in depth, and
    `a_protected_file_named_exactly_passes` feeds `AGENTS.md` in to pin it.
  - Exempt means overridden: `merged_without_approval.is_some()`, which is also sent as
    `VerifyDone.spill_exempt`.
  - A spill (rung 3), a protected-file bounce or a generated-file bounce each reply `Err`
    with its text. A rung-2 or rung-3 bounce of the `done` gate replies the same way.
  - The first gate is chosen in this order:
    - a handed-back task goes to `merge_queue` (decision 36), and `handed_back` is
      cleared;
    - tdd goes to `proof`;
    - a check in the profile sends any other test mode to `check`;
    - a reviewed task goes to `review`;
    - anything else goes to `merge_queue`, pushed onto `Run.merge_queue`.

    No gate op is sent: `proof`, `check` and the merge queue run in M8a.13 and M8a.14.
  - Invented replies:
    - `task_blocked is accepted only while the task is working (it is <state>)`, the
      `task_done` text with the tool's name;
    - `task_done is already being checked; wait for its reply`, when a second claim
      arrives while one is in flight;
    - `task_done could not be checked: <error>; call task_done again`, when
      `VerifyDone` fails;
    - the argument problems (`summary: required`, `must be 1 to 4000 characters`,
      `red: must be 7 to 40 lowercase hex digits`, `<key>: unknown field`,
      `arguments: must be an object`, `kind: must be one of …`);
    - `submit_review is not available yet` (M8a.13's);
    - an unknown tool gets the MCP section's `tool <tool> is not available to the
      <role> role`.
  - The fallback's claim has no reply, so its rejection is queued as `[anthrex] <text>`,
    and the fallback starts over at the next turn end (invented). Its bounce text is
    queued as the rung-1 message (decision 55).
- **`task_blocked`.** `question` and `environment` keep the session alive for an answer.
  `mis_sized` is rung 3 with `the worker reported the task mis-sized: <reason>`
  (invented).
- **The turn-end fallback.**
  - It runs only when all of these hold:
    - the task is `working`;
    - no `task_done` was accepted in that turn;
    - no claim is in flight;
    - the turn was not interrupted by the watchdog;
    - no `CountCommits` or `VerifyDone` is in flight.
  - A claim still in flight at the turn end skips the fallback for that turn.
  - `NO_COMMIT_NUDGE`'s turn is counted again. A commit then gives `DONE_NUDGE`, and
    none is a stall.
- **The ladder.**
  - Rung texts are invented:
    - rung 3 of a gate: `the <gate> gate failed <b> times (<f> failures in all); last:
      <first line>`;
    - rung 3 of stalls: `stalled <n> times …`;
    - rung 3 of budgets: `exceeded its budget twice; last: <what>`;
    - rung 4: `the task's total spend reached the next size's budget (<calls>/<limit>
      tool calls, <m>/<limit> minutes)`.
  - Rung 2, 3 and 4 each drop the task's undelivered messages: the killed session never
    reads them, and rung 2's hand-over prompt carries the failure record. Every gate
    failure's text joins `failure_log`.
  - Rung 2 kills the session, escalates the route, and records `fresh_session`. Once
    every worker round of the task has ended, the engine sends `DiffSoFar`, and its
    result launches the session with `handover_prompt`. That requires a `working` task
    with no hold (M8a.6 ruling N5). A `DiffSoFar` failure is named in the prompt's stat
    line.
  - Rung 4's ceiling is `budget_m` for a non-hub S task and `budget_l` otherwise,
    compared with the task's total: tool calls and tokens counted on the task, seconds
    summed over its worker rounds. It is checked before the session budget.
  - `Task.spent_total.secs` is not stored. The snapshot's `spent_total` is
    `ladder::total_spend`.
- **Budgets.**
  - Hard: `2 × spend >= 3 × budget`. The decision says 1.5 × the budget "is a breach",
    so reaching it counts. With a 5-minute budget, 7:29 is not a breach and 7:30 is; the
    test pins both.
  - Soft: `spend >= budget` on any axis. `budget_wrap_up` is sent once per session.
  - The minutes axis is checked at every scheduler pass of a running run.
- **Stalls.**
  - The clock starts at `max(last_event, rate_limited_until)`.
  - A delivery or resume sets `last_event` to its time, so a turn delivered long after
    the last event is not stalled at once (invented; `last_event` stays counter-only).
  - `stall_nudge(stall_after_secs / 60)` is queued with the interrupt. `StallState::Nudged`
    lasts for the rest of the session, so any later silence is the second stall.
- **Rate limits and failed turns.**
  - Any `ApiRetry` sets `rate_limited_until`. Only `error == "rate_limit"` starts or
    continues a streak.
  - A failed `RateLimit` turn sets `rate_limited_until` to its continue time. An `Other`
    failure does not, so the snapshot does not show it as rate-limited, but the outbox
    holds every message until its continue is sent (`FailedTurn::WaitingContinue`).
  - "In a row" means the turn after an `Other` failure's continue. A completed turn
    resets it.
  - `Authentication` and `Billing` block the task and leave the session alive.
    `SandboxUnavailable` blocks it and kills the session.
- **Process exits.**
  - A Codex exit between turns changes nothing.
  - A Claude worker's exit between turns, or any worker exit while the task is not
    `working` (a held task included), marks the round ended. The next delivery resumes
    it with `ResumeSession` carrying the joined messages (`AgentRound.carried`).
  - `Resumed` removes the carried messages from the outbox. `ResumeFailed` (or `Failed`)
    ends the round and gives a fresh session at the same rung, with no failure counted,
    whose prompt ends with those messages.
  - A mid-turn exit with no session id to resume gets the same fresh session.
  - A reviewer's unexpected exit only clears its pid, as in M8a.11: decision 35's resume
    rule is M8a.13's.

**Carries.**

- **T8 (`task_done` off the task's branch): done.** M8a.8's proposed text is used,
  tested for a branch and for a detached HEAD, and pinned by mutant M1.
- **T7-C2 (the usage-limit reset time): passed on to M9.5, whole.** Decision 32 fixes the
  wait at `rate_limit_retry_secs`. No M8a.12 decision gives a per-runtime availability
  clock or a routing rule, and the reset time in the fixture has no time zone. The
  follow-ups file's entry says so.
- **The two guards in `holds.rs::abort_untold_conflict`.**
  - **R5 (the `delivered_at.is_none()` filter) is reachable now.** A worker's
    `task_blocked` can block a task whose conflict message is still being delivered, and
    a blocked task can gain a dependency. `add_dep` on a `working` task is refused, which
    is why it was unreachable before. Test:
    `a_conflict_being_delivered_is_not_undone_when_the_task_is_held_again`, pinned by
    mutant M18.
  - **R4 (no `HandBack` or `AbortMerge` in flight) stays unreachable.** Such an op runs
    only while the task is held, and nothing M8a.12 adds clears `awaiting_deps`. Every
    new block applies only to a `working` task.
- **"Not available yet" stubs.** `task_done` and `task_blocked` are replaced.
  `submit_review` stays M8a.13's. Retry, override, cancel, resume and finish stay M8a.14
  and M8a.15's.
- **The N5 hold on every new path.**
  - A process exit never resumes a task that is not `working`.
  - A resume carrying a delivery goes through the outbox, which skips a blocked task.
  - A fresh session requires `working` and no hold.
  - Tests: `a_held_task_is_not_resumed_after_an_exit_or_by_a_delivery`, a real sequence,
    and `a_held_task_gets_no_fresh_session_until_its_hand_back`. The second sets
    `fresh_session` by hand: rung 2 needs a `working` task and a dependency needs a
    blocked one, so no sequence reaches both today. It pins the guard, as mutant M13
    shows.

**Carries for later tasks.**

- **M8a.13.**
  - The engine moves a task into `proof`, `check` and `review` but sends no gate op.
    The gates must start from those states.
  - `ladder::gate_failure(run, i, GateKind, text, told, now, fx)` is ready for the
    proof, check, review and merge failures. It returns the task to `working` at rung
    1, where the fallback and watchdog apply again.
  - A reviewer's exits and stalls are M8a.13's.
- **M8a.15.**
  - Restore must clear a `Task.claim` whose `VerifyDone` was dropped as `NotStarted`,
    and a `CountCommits` fallback (`FallbackState::Counting`). Otherwise the task
    refuses every later `task_done` as "already being checked".
  - Retry can reuse `fresh_session` and `start_fresh_sessions`.
- **M8a.22.** The driver contract above, and `Event::Delivered.error`.

**TDD evidence.**

- 35 tests were written first: the brief's 31, three N5/R5 tests in `turns_holds.rs`,
  and `counter_changes_from_tool_calls_stay_lazy`. The run gave `42 passed; 33 failed`,
  every failure for the missing behaviour.
  - Two new tests passed at red, because both pin behaviour M8a.11 already had and that
    the new code must keep:
    - `a_session_the_engine_killed_is_not_treated_as_an_exit` (mutant M27, treating the
      engine's kill as a death, kills it);
    - `counter_changes_from_tool_calls_stay_lazy` (mutant M26, leaving `spent_total` out
      of `without_counters`, kills it).
  - Some of the failure lines:
    - `left: Err("the task_done tool is not available yet") right: Err("unknown run
      nope")`;
    - `no reply yet: [Reply { … }]`;
    - usage `left: TokenUsage { input: 0, … } right: TokenUsage { input: 112, … }`;
    - rung 4 `left: Working right: Blocked`;
    - `effects.contains(&Effect::Interrupt { … })`;
    - rate limits `left: None right: Some(1)`.
- Four test defects were found and fixed, and each fix is in the test:
  - `add_dep` on a `working` task is refused, so the N5 and R5 tests now block t1 with
    `task_blocked` first. The rewritten tests were not re-run red, but mutants M13, M14,
    M18 and M20 show they pin the guards.
  - The review case owned `crates/a/**`, which touches `source`, and rule 8.1 refused
    `test_mode = "none"`. It now owns `docs/**`.
  - The stall and rate-limit tests ran into S's 15-minute budget, which breaches at
    22.5 minutes. They now use a 1000-minute budget.
  - The rung-2 effort assertion assumed `high`. It now asserts one step up.
- **Mutations**, each restored from a WIP commit (since folded): 27 run, 27 killed
  (M26 and M27 are above).
  - M24 (the delivery gate ignoring a pending continue) first survived: nothing was
    queued during the wait. `failed_turns` now queues a message during an `Other`
    failure's wait and expects it joined with the continue, and M24 is killed.
  - The killed mutants are:
    - M1, the branch check;
    - M2, the literal re-filter;
    - M3, the protected check before the spill split;
    - M4, the override exemption;
    - M5, a floored 1.5 ×;
    - M6, cache reads made billable;
    - M7, the wrap-up sent more than once;
    - M8, rung 4 off;
    - M9, denials not de-duplicated;
    - M10, a streak's failure counted twice;
    - M11, the stall clock ignoring rate limits;
    - M12, three deaths instead of two;
    - M13, a fresh session under the hold;
    - M14, no resume on delivery;
    - M15, no delivery block;
    - M16, a Codex exit that ends the round;
    - M17, no sub-agent deferral;
    - M18, R5;
    - M19, the tdd rule applied to the fallback;
    - M20, a held task resumed after an exit;
    - M21, delivery to a blocked task;
    - M22, the interrupt grace;
    - M23, `Other` failures that never block;
    - M24, the continue wait;
    - M25, a rung-1 text queued as well as replied.

**Gates.**

- `cargo build --workspace --all-targets`, clippy with `-D warnings` and `cargo fmt
  --all --check` are clean.
- `cargo test -p anthrex-daemon` passes: 34 binaries, 665 unit tests.
- One run under a load average of 28 failed once in `tests/window.rs`, which does not
  touch the engine. It passed alone, and on two full re-runs.

### M8a.12 fix round 1 (2026-09-23)

The review (`.superpowers/sdd/M8a-orchestration-engine-core/task-12-review.md`) found
4 Important and 7 Minor issues. Rulings T12-I1..I4, T12-minors and T12-m5 apply. Each
probe became a regression test in `engine/tests/turns_fixes.rs`, and each failed first.
The minors' tests, and the denial test moved out of `turns.rs` for size, are in the new
`engine/tests/turns_minors.rs`.

**I1: a claim belongs to its session (ruling T12-I1).**

- `PendingClaim.window_id` (`#[serde(default)]`) is the claiming session's window.
- **A result for another session is dropped.** `checked` drops a result, replying `Err`
  (`this session is being replaced; its task_done no longer applies`), when the claim's
  window is not the task's current session. The current session is the latest worker
  round, unless it is `retiring` (killed, or its resume failed). A Claude round that
  merely ended between turns is still the same session, and its claim stands.
- **Every session change clears the claim.** `done::drop_claim` answers the waiting
  tool call at once and clears the claim. It runs in these places:
  - `kill_worker` (rungs 3 and 4, the denial and sandbox blocks);
  - `rung2`, whose text says the session is being replaced by a fresh one;
  - every new round (`dispatch::launch`);
  - `worker_tool`, when a claim of an earlier session is still recorded.
- **Restore clears every claim and every `FallbackState::Counting`.** This settles the
  first pass's concern 1, which no longer needs to go to M8a.15.
- **The fallback guard changed.** It now waits only for `CountCommits` and a recorded
  claim. A `VerifyDone` left over from a replaced session no longer blocks it.
- Tests: `a_claim_from_a_replaced_session_is_dropped` (probe C, both orderings),
  `restore_clears_a_claim_and_a_count_in_flight` and
  `a_claim_follows_its_session_not_its_process`.

**I2: an exit during an interrupt ends the interrupted turn (ruling T12-I2).**

- A non-engine exit of a working task's worker, while its turn is open and its stall is
  `Interrupted`, ends the interrupted turn. The stall becomes `Nudged` and no death is
  counted, so the grace kill never comes.
- For Codex, whose interrupt is exactly such an exit (M8a.1), the round stays live, and
  `stall_nudge` is delivered: the driver runs it as `exec resume`.
- For Claude, the round ends, and the delivery resumes it carrying `stall_nudge`, not
  `RESUME_AFTER_EXIT`.
- Tests: `a_codex_interrupt_exit_ends_the_turn_and_gets_the_nudge` (probe A) and
  `a_claude_exit_on_the_interrupt_resumes_with_the_nudge`.

**I3: a failed resume is final (ruling T12-I3).**

- `resumed`'s failure sets `retiring`, so no delivery resumes the round again.
- While a fresh session is pending, `deliver` moves every undelivered message of the
  task into `FreshSession.append`, in order, after anything already there. The carried
  messages of a failed resume go the same way (`accumulate`), so nothing is overwritten.
- `fresh_diff` clears `fresh_session` only once the launch has happened. A launch the
  window limit refuses keeps it, and the messages it carries, for a retry.
- Tests: `a_failed_resume_is_final_and_messages_accumulate_for_the_fresh_session`
  (probe D: the prompt ends `DONE_NUDGE`, a blank line, `[anthrex] X`) and
  `a_refused_fresh_launch_keeps_its_messages`. The second was written after the code;
  mutant F15 pins it.

**I4: nothing is left waiting forever (ruling T12-I4).**

- **(a) A round that ended between turns keeps its failed turn's timer.** `watch`
  handles the timer for such a resumable round too, and the continue then goes out as a
  `ResumeSession`.
  - A Claude exit between turns while the fallback waits for sub-agents clears the
    sub-agents, which died with the process, and runs the fallback.
  - Tests: `an_exit_during_a_failed_turns_wait_keeps_its_continue` (probe B) and
    `an_exit_while_the_fallback_waits_for_subagents_runs_it`.
- **(b) Blocking a task clears its interrupt.** `dispatch::block` resets any
  `Interrupted` stall to `Watching` and drops the undelivered `stall_nudge`
  (`contract::is_stall_nudge`), so whatever unblocks the task is delivered.
  - `turn_ended` also applies `Interrupted → Nudged` before its early returns.
  - Test: `a_block_during_an_interrupt_clears_it_so_the_answer_delivers` (probe E).
- **A rejection after the turn ended runs the fallback** (`after_rejection`), because
  otherwise a working task would be left with nothing pending. This was found by the
  liveness check. Test: `a_late_rejection_runs_the_fallback`.
- **The liveness check.** `turns_fixes::assert_alive` runs after every probe sequence.
  For each working task it requires at least one of:
  - a live worker round with an open turn;
  - a message that can be delivered (the round is live and not interrupted, or
    resumable, or a fresh session is pending), or one in flight;
  - a failed turn's timer or a delivery retry;
  - an op in flight;
  - a kill in progress.

**Minors (ruling T12-minors).**

- **m-1.**
  - `rejections_come_before_the_spill_split`: a dirty or off-branch claim that also
    has spill and protected paths is a rejection, not rung 3 (reviewer mutant Mr).
  - `a_retiring_window_cannot_claim` (reviewer mutant Mo).
- **m-2.** `counted` resets the fallback and queues nothing while a claim is in flight.
  Test: `no_done_nudge_while_a_claim_is_in_flight` (probe F).
- **m-3.** `a_completed_turn_resets_the_two_other_failures_rule` (reviewer mutant Mi).
- **m-4. `AgentSignal::TurnEnded.denials` is now `Vec<String>`**, the result's
  `permission_denials[].tool_name`, as `headless::SessionEvent::TurnEnded` already
  carries it. This is a deviation from the Interfaces' `u32`.
  - `AgentRound.turn_denials` became `turn_denied: Vec<String>`, the tools seen as
    events in the turn.
  - The new denials are the listed tools left after removing each seen one once.
  - The last new one becomes `last_denial`, with the fixed reason
    `contract::DENIAL_LISTED_REASON` (`listed in the turn's permission_denials`),
    because the result carries no reason.
  - **Carry for M8a.22:** pass the names through.
  - Test: `denials_block_at_the_threshold`, now in `turns_minors.rs`, including probe G
    and an ordering case.
- **m-5 (ruling T12-m5).** `Task.spent_total`, and so rung 4, counts worker rounds only:
  the tool calls and tokens of reviewer rounds stay on those rounds. Test:
  `reviewer_spend_is_not_the_tasks`.
- **m-6.** The hard-limit comparisons use `saturating_mul`. Test:
  `a_huge_token_budget_does_not_overflow`, which overflowed in debug before.
- **m-7.** `gate_failure` returns the rung. A `done` bounce's reply is the message at
  rung 1, and at rung 2 `task_done rejected again: this session is being replaced by a
  fresh one; stop now` (invented). At rung 3 it is `task_done rejected: the task is
  blocked (mis_sized): <cause>; stop now` (invented). Test:
  `a_rung_2_bounce_reply_says_the_session_is_replaced`.

**TDD and mutation evidence.**

- **The red run.** The fix-round tests were written first. Only the `denials` type and
  the `DENIAL_LISTED_REASON` constant were changed beforehand, so the tests would
  compile. The run gave `77 passed; 13 failed`. Sample failures:
  - probe A: a `ResumeSession(RESUME_AFTER_EXIT)` was emitted;
  - probe B: `left: [] right: [rate_limit_continue]`;
  - probe C: `rung 2 clears the claim`;
  - probe D: `never again: [..ResumeSession..]`;
  - probe E: `left: [] right: [answer]`;
  - m-6: `attempt to multiply with overflow`;
  - m-5: `left: 1 right: 0`.
- **Three passed at red: m-1's two tests and m-3's.** They pin correct code, and the
  reviewer's surviving mutants Mr, Mo and Mi each now fail one of them.
- **Tests added after the first green run.** `a_claim_follows_its_session_not_its_process`,
  `a_late_rejection_runs_the_fallback` and `a_refused_fresh_launch_keeps_its_messages`
  were added once mutants F1, F5 and F16 survived, and they kill them.
- **The ordering case.** `denials_block_at_the_threshold` gained its ordering case once
  mutant F12, counting instead of matching tools, survived.
- **19 mutants were run; all are killed.** F1 to F16, Mi, Mo and Mr, including every
  new guard.

**Gates.**

- `cargo build --workspace --all-targets`, clippy with `-D warnings` and `cargo fmt
  --all --check` are clean.
- `cargo test -p anthrex-daemon` passes: 34 binaries, 683 unit tests.
- No file passes 600 lines. `tests/turns.rs` is 558 lines, `turns_fixes.rs` 425 and
  `turns_minors.rs` 275.

### M8a.12 fix round 2 (2026-09-23)

The re-review (`.superpowers/sdd/M8a-orchestration-engine-core/task-12-rereview-1.md`)
confirmed I-2, I-3 and every minor. It found N-1, N-2 and N-3 (Important) and N-4
(Minor). Rulings T12-N, T12-N2, T12-N4 and T12-later apply. The probes became regression
tests in the new `engine/tests/turns_ops.rs`, and each failed first.

**N-1 and N-3: every op result is matched to the op the engine awaits (ruling T12-N).**

- **What the engine awaits is recorded.** All the new fields are `#[serde(default)]`:
  - `PendingClaim.op` is the claim's `VerifyDone`.
  - `AgentRound.resume_op` is set by both `ResumeSession` sources: a delivery to an
    ended round, and a mid-turn exit.
  - `AgentRound.count_op` is the turn-end fallback's `CountCommits`.
- **The handlers take the op id.** `op_done` passes it to `done::checked`,
  `done::counted` and `outbox::resumed`.
  - `checked` settles a claim only with its own op's result.
  - `counted` counts only for the current worker round's awaited count.
  - `resumed` takes only the round that awaits that resume. It no longer takes the
    latest worker round.
  - Anything else is dropped without a reply, since the claim it belonged to was
    already answered by `drop_claim`.
- **Supersession ends the waiting.** `ladder::supersede` runs in `kill_worker` (rungs 2
  to 4, the denial and sandbox blocks) and at every new worker round
  (`dispatch::launch`). It clears every worker round's `resume_op` and `count_op`, and
  removes the task's in-flight messages from the outbox. Those are a `Deliver`'s and a
  resume's.
- **`Delivered` is matched through the outbox.** It has no op id; its message ids are
  its correlation. A superseded session's messages have left the outbox, so its
  `Delivered` finds no task and changes nothing.
  - An `AgentRound.delivering` field was tried and removed: `delivered` already finds
    its task through the outbox, so that field's guard was a surviving mutant.
- **The fallback's guard is now the round's `count_op`**, not any `CountCommits` of the
  task in flight. A replaced session's count no longer holds up the fresh session's
  fallback.
- **Restore clears `count_op`** along with `Counting`, so the next turn end counts
  again.
- **`DiffSoFar` is not session-scoped and was left as it is.** It belongs to the fresh
  session still to start. `fresh_due` and its op-in-flight guard already drop a result
  that is no longer due, and only one can be in flight per task. `CreateWindow` was
  already matched by `launch_op`.
- Tests:
  - `a_stale_verify_result_leaves_the_fresh_sessions_claim_pending` (probe P1);
  - `a_stale_rejection_is_not_the_fresh_sessions` (P1b);
  - `a_stale_resume_failure_leaves_the_fresh_session_live` (P2: one live session, no
    second fresh one, and the fresh window's tools are still answered);
  - `a_stale_resumed_leaves_the_fresh_resumes_messages` (P2b);
  - `a_stale_count_neither_blocks_nor_nudges_the_fresh_session`;
  - `a_stale_delivery_failure_leaves_the_fresh_session_alone`;
  - `results_between_the_kill_and_the_fresh_session_are_dropped`;
  - `a_dead_sessions_late_delivery_failure_leaves_the_fresh_session_alone`;
  - `one_count_at_a_time`, `a_failed_resume_after_a_mid_turn_exit_starts_a_fresh_session`
    and `a_restore_awaits_no_count`.

**N-2: a late verdict re-engages the worker (ruling T12-N2).**

- `done::reengage` replaces fix round 1's `after_rejection`. For an explicit claim, a
  rejection, a check that could not run, or a rung-1 bounce reaches the worker as its
  next turn when the claiming turn is no longer open. The text is prefixed
  `[anthrex] ` unless it already is.
- `deliver` sends that turn: a `Deliver` to a live round, or a `ResumeSession` to a
  Claude round whose process ended. Within the open turn, the tool reply alone
  suffices.
- **Deviation from fix round 1.** A rejection after the turn ended no longer runs the
  turn-end fallback. The re-engaged turn ends normally, and its fallback runs then.
  `turns_fixes::a_late_rejection_runs_the_fallback` became
  `a_late_rejection_is_the_next_turn`.
- Tests, one per reviewer case, each ending with `assert_alive`:
  - `a_rejection_after_the_process_exited_resumes_with_it` (case a);
  - `a_bounce_after_the_turn_is_the_next_turn` (case b, delivered exactly once);
  - `a_bounce_after_the_process_exited_resumes_with_it` (case c);
  - `a_failed_check_after_the_process_exited_resumes_with_it` (case d);
  - `a_rejection_in_the_open_turn_is_only_the_reply` pins the open-turn case.

**N-4: a rejected fallback claim sends only its rejection (ruling T12-N4).**

- The fallback's claim has no tool call. Its rejection or bounce text is queued once,
  and nothing else follows it: no second count and no `DONE_NUDGE`.
- Test: `a_rejected_fallback_claim_sends_only_the_rejection` (probe P3). The turn is
  exactly the rejection text.

**T12-later.**

- **The stall clock waits for a `task_done` check.** `watch` skips the stall check
  while the task has a claim in flight. The verdict restarts the clock
  (`last_event = now`) for the worker round. Test:
  `the_stall_clock_waits_for_a_task_done_check`.
- **A turn interrupted before its session has an id is rung 2.** Only a Codex turn can
  be, since Claude's id is set at launch. Its exit ends the round and calls
  `ladder::rung2`: the route escalates and no failure is counted. The fresh session's
  `append` is `stall_nudge`, so the hand-over prompt ends with the nudge.
  - The check is on the missing id, not on the runtime. A Claude round without an id
    would otherwise be ended with nothing able to resume it.
  - Test: `a_codex_interrupt_before_its_session_id_is_rung_two` (probe P4).
- **The unreachable denial fallback text is gone.** Every counted denial records itself
  in `last_denial`, so `check_denials` uses `unwrap_or_default`.
- **Carry for M8a.15:** `run retry` must clear a pending `fresh_session` (and its
  `append`), along with the rung it resets.

**TDD and mutation evidence.**

- **The red run.** The 14 probe tests were written first, and the run gave
  `1 passed; 13 failed`. Each failed on its key assertion:
  - P1 and P1b: the stale result replied to the fresh claim;
  - P2: the fresh round ended and retired;
  - P2b: `left: [] right: [2]`, the carried messages taken;
  - the stale count: no `CountCommits` for the fresh session;
  - the stale delivery: the fresh round's turn closed;
  - cases a, c and d: no `ResumeSession`;
  - case b: no delivery;
  - P3: a second `CountCommits`;
  - the stall pause: an `Interrupt` during the check;
  - P4: a `Deliver` to the Codex window.
- `a_rejection_in_the_open_turn_is_only_the_reply` passed at red. It pins behaviour
  that was already right.
- **The first mutation run: 29 mutants, 17 killed.** Of the 12 survivors, four went
  with design changes. G4, G15 and G17 guarded the removed `delivering` field. H11 was
  the runtime check, which became the id check. The run also showed that six guards had
  no test. They were `supersede` in `kill_worker` and in `launch`,
  `supersede` clearing `count_op`, the fallback's `count_op` guard, the mid-turn
  exit's `resume_op`, and restore's `count_op`. Five tests were added for them, each
  failing on its mutant:
  - `results_between_the_kill_and_the_fresh_session_are_dropped`;
  - `a_dead_sessions_late_delivery_failure_leaves_the_fresh_session_alone`;
  - `one_count_at_a_time`;
  - `a_failed_resume_after_a_mid_turn_exit_starts_a_fresh_session`;
  - `a_restore_awaits_no_count`.
- The N-4 and case b and c tests were tightened to exact texts. That kills a `told`
  bypass and a doubled `[anthrex]` prefix.
- **The second run: 26 mutants, 24 killed.** The two survivors are equivalent:
  - `resumed` clearing `resume_op`: the pending op is already removed, so no second
    result can come. It is kept so the field states what is awaited, for M8a.15's
    re-issue.
  - `supersede` clearing `carried` was removed as redundant: the outbox removal covers
    those messages, and a superseded round's `carried` is never read.

**Gates.**

- `cargo build --workspace --all-targets`, clippy with `-D warnings` and `cargo fmt
  --all --check` are clean.
- `cargo test -p anthrex-daemon` passes: 34 binaries, 702 unit tests.
- No file passes 600 lines. `dispatch.rs` is 589 lines, `done.rs` 551 and
  `tests/turns_ops.rs` 529.

### M8a.12 fix round 3 (2026-09-23)

The second re-review (`.superpowers/sdd/M8a-orchestration-engine-core/task-12-rereview-2.md`)
accepted N-1 to N-4, the "later" items and the four round-2 deviations. It found A1 and
A2 (Important) and O-1 and O-2 (Minor). Rulings T12-A1, T12-A2, T12-O1 and T12-O2
apply. The probes became regression tests in the new `engine/tests/turns_retries.rs`,
and each failed first.

**A1: a failed resume supersedes the round (ruling T12-A1).**

- On failure, `outbox::resumed` takes the carried messages and then calls
  `ladder::supersede` before it ends the round. The round's `count_op` is cleared, so
  its late `CountCommits` is dropped.
  - Before this fix, a late `Commits{0}` ran `ladder::stall`: a failure, a stall and
    rung 2 were counted, and `fresh_session` was overwritten, which dropped its
    `append`.
- Test: `a_count_after_a_failed_resume_is_dropped` (probe PA, with counts 0 and 2).

**A2: a failed `CountCommits` is retried (ruling T12-A2).**

- New `AgentRound` fields, both `#[serde(default)]`: `count_failures` and
  `count_retry_at`.
- A `Failed` count, for a working task with no claim, keeps the fallback state and
  retries `DELIVERY_RETRY_SECS` later. The `DELIVERY_MAX_FAILURES`th (third) failure in
  a row blocks the task as `environment`, with `could not count the task's commits:
  <error>` (invented). A successful count resets the failures.
- `signals::watch` sends the retry, through `fallback::retry_count`. It does so for a
  live round and for one that ended and can be resumed, so the variant where the
  process exited works too.
  - When a claim was made meanwhile, the retry is not sent.
- **The fallback sends no count while one waits to be retried.** A turn end in that
  window adds nothing.
- Restore clears `count_retry_at`. `turns_fixes::assert_alive` counts a count retry as
  a timer.
- Tests:
  - `a_failed_count_is_retried`, live and after an exit: exactly one retry, then
    `DONE_NUDGE` as a delivery or a resume;
  - `the_third_failed_count_blocks_the_task`, live and after an exit;
  - `a_successful_count_resets_the_failures`;
  - `a_claim_takes_over_from_a_count_retry`;
  - `a_turn_end_waits_for_the_count_retry`;
  - `a_restore_drops_a_count_retry`.

**O-1: activity inside the interrupt grace ends the grace (ruling T12-O1).**

- Any stream event from the round, or a `task_done` from it (in `worker_tool`), turns
  an `Interrupted` stall into `Nudged`.
  - The grace kill is cancelled.
  - The queued `stall_nudge` stays and is delivered at the turn's end.
  - The next silence is a stall, as after any nudge.
- `TurnEnded` is excluded. It makes the same change itself, and it must still see the
  interrupt, so that a completed interrupted turn gets the nudge and not the fallback.
- Tests:
  - `a_claim_in_the_grace_cancels_the_kill` (probe PE);
  - `activity_in_the_grace_cancels_the_kill`;
  - `the_interrupted_turns_end_gets_the_nudge_not_the_fallback`.

**O-2: a verdict during a later turn waits for that turn's end (ruling T12-O2).**

- `PendingClaim.turn` (`#[serde(default)]`) records the claiming round's `turns` at
  the claim.
- `reengage` leaves the reply alone only when the claiming turn itself is still open.
  If a later turn is open, the text is queued, and normal delivery sends it at that
  turn's end.
- Test: `a_verdict_during_a_later_turn_is_its_next_turn` (probe PD).
- **A known consequence.** That later turn's end also runs the fallback, since no claim
  is left. Its `CountCommits`, and the `DONE_NUDGE` that follows, come after the
  rejection turn.

**The file split.** `done.rs` grew to 608 lines. The turn-end fallback, its count
handling and its retry moved to the new `engine/fallback.rs` (152 lines), and `done.rs`
is now 470. `done::claim` became `pub(super)` for the fallback's own claim.

**TDD and mutation evidence.**

- **The red run.** The six probe tests were written first, and the run gave `0 passed;
  6 failed`:
  - PA: `left: (1, 1, 2)`, a failure, a stall and rung 2;
  - PB: `assert_alive` failed, with nothing pending;
  - the block test: no retry `CountCommits`;
  - PE and the activity test: a `KillWindow`;
  - PD: the rejection was not queued.
- **The first mutation run: 17 mutants, 11 killed.** Six guards had no test, and each
  now has one that fails on its mutant:
  - the failure reset (K7);
  - the claim takeover (K8);
  - restore (K10);
  - the timer clear (K11);
  - the turn end during a retry (K12);
  - `TurnEnded`'s exclusion (O1b).
- **`supersede` clearing `count_retry_at` was removed.** It was an equivalent mutant
  (K9): a superseded round is never the round `watch` looks at.
- **The final run: 16 mutants, all killed.** They are K1 to K8, K10 to K12, O1, O1b,
  O1c, O2 and O2b.

**Gates.**

- `cargo build --workspace --all-targets`, clippy with `-D warnings` and `cargo fmt
  --all --check` are clean.
- `cargo test -p anthrex-daemon` passes: 34 binaries, 1042 tests, of which 713 are
  unit tests.
- No file passes 600 lines. `dispatch.rs` is 591 lines, `signals.rs` 473, `done.rs` 470
  and `tests/turns_retries.rs` 325.

### M8a.12 fix round 4 (2026-09-23)

The third re-review (`.superpowers/sdd/M8a-orchestration-engine-core/task-12-rereview-3.md`)
approved A1, A2 and O-1, found O-2 partly done (N3-2) and one new Minor (N3-1), and
listed OS-1 out of scope. Ruling T12-R4 brings OS-1 in: a false stall kill escalates a
healthy worker. The probes became regression tests in the new
`engine/tests/turns_stale.rs`, and each failed first.

**OS-1: the fallback's count and claim belong to their turn (ruling T12-R4).**

- New `AgentRound.count_turn` (`#[serde(default)]`): the round's `turns` when the
  fallback sent its count. A retry keeps it. The fallback's own claim already records
  `PendingClaim.turn`.
- A result is stale once `turns` has moved past that number (a delivery, a resume or a
  self-started turn has begun). `fallback::counted`, `fallback::retry_count` and
  `done::checked` (for the fallback's claim, which has no reply) drop a stale result
  through `fallback::drop_stale`:
  - The fallback state goes back to where it was before the count: `Counting` becomes
    `None`, but `Nudged{..}` stays. A nudge the worker has read still counts, so a
    second empty count after `NO_COMMIT_NUDGE` is still a stall
    (`one_count_at_a_time`, updated).
  - **Liveness.** A turn still open runs the fallback at its end. If that end has
    already come and gone (it was skipped while the count or claim was out), the
    fallback runs at once. It does not when a message is queued, a failed turn waits
    for its continue, sub-agents hold the fallback, or the turn was interrupted: each
    of those opens, or already is, the turn whose end runs it.
- `fallback()` voids a retry left from an earlier turn, and counts for the turn that
  just ended.
- Two earlier tests encoded the old behaviour and were updated:
  - `one_count_at_a_time`: the late count is dropped and counted again. Its 0 is then
    the stall.
  - `a_turn_end_waits_for_the_count_retry`: the later turn end counts once, and the
    retry time sends nothing. It is still one count, not two.
- Tests:
  - `a_count_that_lands_in_a_later_turn_is_dropped` (probe RD, in flight);
  - `a_count_dropped_after_the_later_turn_ended_counts_again`;
  - `a_count_dropped_while_a_message_waits_counts_at_that_turns_end`;
  - `a_retried_count_is_not_sent_into_a_later_turn` (probe RD, retry);
  - `a_fallback_claim_answered_in_a_later_turn_is_dropped`;
  - `a_fallback_claim_dropped_after_the_later_turn_ended_counts_again`;
  - `the_count_after_a_late_rejection_waits_for_the_rejection_turn` (probe OD).
- **Consequence for O-2's known double message.** The count that the rejection's turn
  end used to get back mid-turn is now dropped. The rejection's own turn end counts
  afresh, so `DONE_NUDGE` no longer races the fallback's immediate claim.

**N3-1: an interrupted turn stays interrupted (ruling T12-R4).**

- New `AgentRound.interrupted` (`#[serde(default)]`), set by the watchdog's interrupt.
  It is taken at `TurnEnded`, at the exit that ends an interrupted turn, and at
  `end_round`. Activity inside the grace still turns `Interrupted` into `Nudged`, so
  there is no kill (T12-O1), but the turn's end still sends only `stall_nudge`, never
  the fallback.
- The exit path uses the same flag, so a Codex interrupt that ends with an exit after
  some activity is the turn's end, not a death resumed with `RESUME_AFTER_EXIT`.
- Tests: `an_interrupted_turn_with_activity_gets_the_nudge_not_the_fallback` (probe
  OB) and `a_codex_interrupt_exit_after_activity_is_the_turns_end`.

**N3-2: a turn Claude starts by itself is a turn (ruling T12-R4).**

- `TurnStarted` with no open turn increments `turns`. A delivered turn is already open,
  so it is not counted twice.
- Test: `a_verdict_during_a_self_started_turn_is_queued_for_its_end` (probe OC). It
  also checks that a second `TurnStarted` counts nothing.

**Out of scope, recorded in the follow-ups under M8a (no code change):** a task
blocked by the third failed count keeps its live session; a failed resume's `supersede`
drops an in-flight wrap-up; `exited` ignores `pid`.

**TDD and mutation evidence.**

- **The red run.** The nine probe tests were written first, and the run gave `0 passed;
  9 failed`:
  - RD in flight: `NO_COMMIT_NUDGE` queued mid-turn;
  - the count-again variants: no `CountCommits`;
  - RD retry: the retry's `CountCommits` went into the later turn;
  - the two fallback-claim tests: the rejection was queued into the later turn, or no
    count followed;
  - OD: `DONE_NUDGE` queued mid-turn;
  - OB: a `CountCommits` at the interrupted turn's end;
  - the Codex variant: a death resumed with `RESUME_AFTER_EXIT`;
  - OC: `turns` stayed at 1.
- The tenth test, for the queued-message guard, was added after the first mutation
  run. It was checked against HEAD's engine sources, where `NO_COMMIT_NUDGE` was queued
  beside the waiting message.
- **Mutation run: 12 mutants, all killed.** The first run left one alive, the
  queued-message guard in `drop_stale` (M3b), and the tenth test now kills it. Each
  mutant was applied and then undone by hand. The mutants covered:
  - the stale check in `counted`, `checked` and `retry_count`;
  - `drop_stale`'s re-run and its queued-message guard;
  - `fallback()`'s retry void;
  - recording `count_turn`;
  - `restart` keeping `Nudged`;
  - the `interrupted` flag in `turn_ended` and in `exited`;
  - the `TurnStarted` increment.

**Gates.** `cargo build --workspace --all-targets`, clippy with `-D warnings` and `cargo
fmt --all --check` are clean. `cargo test -p anthrex-daemon` passes: 34 binaries, 1052 tests, of which 723 are unit tests. No file passes 600
lines: `done.rs` is 478, `signals.rs` 484, `fallback.rs` 210 and
`tests/turns_stale.rs` 292.

### M8a.12 fix round 5 (2026-09-23)

Re-review 4 (`.superpowers/sdd/M8a-orchestration-engine-core/task-12-rereview-4.md`)
approved round 4. It found R4-1, an older false stall kill that the turn-number guard
does not see, and the fix follows ruling T12-R5.

**R4-1: no count while a message waits for delivery (ruling T12-R5).**

- The sequence:
  1. `NO_COMMIT_NUDGE` is queued, its delivery fails and it waits for its retry.
  2. Claude starts a turn by itself and ends it.
  3. `fallback()` saw `Nudged{false}` and counted again, although the nudge was never
     delivered.
  4. A 0 was a stall, and it killed the session.
- The fix: `fallback()` now returns while the task has an undelivered outbox message.
  It uses the same `queued` test as `drop_stale`, factored into `fallback::queued`.
  This covers every fallback state.
- Liveness: the queued message is itself the pending item, through its delivery or
  delivery retry. The turn it opens ends with the fallback.
- Test: `no_count_while_the_nudge_waits_for_its_delivery_retry` (probe PA).
  - The self-started turn's end sends no count, and nothing stalls.
  - At the retry `NO_COMMIT_NUDGE` is delivered, and that turn's empty count is then
    the stall.
  - Red before the fix: a `CountCommits` came at the self-started turn's end
    (`turns_stale.rs:148`).
- Mutant: removing the guard fails three tests:
  - `no_count_while_the_nudge_waits_for_its_delivery_retry`;
  - `a_count_dropped_while_a_message_waits_counts_at_that_turns_end`;
  - `the_count_after_a_late_rejection_waits_for_the_rejection_turn`.

**Tests adjusted to the guard:**

- `the_count_after_a_late_rejection_waits_for_the_rejection_turn`: the turn end that
  sends the rejection now sends no count at all (in round 4 it sent one, which was
  dropped as stale). The rejection's own turn end counts, and `DONE_NUDGE` follows.
- `turns_retries::a_count_after_a_failed_resume_is_dropped`: the 4-call budget's
  wrap-up used to be queued before the turn end, and it now holds that end's count
  back.
  - The test now queues its message after the count has gone out.
  - It still ends with a mid-turn exit, a failed resume and the late count.
  - The late count is now dropped both by `supersede` (T12-A1) and as stale (T12-R4).
- `turns_retries::a_turn_end_waits_for_the_count_retry` is renamed to
  `a_later_turn_end_voids_the_count_retry`.

**Carry for M8a.15.** Restore (`requests.rs`) clears neither `AgentRound.interrupted`
nor `StallState::Interrupted`, and both are persisted. That is harmless only as long as
restored sessions are ended. M8a.15's restore must clear both, or end restored sessions
through `signals::end_round`, which clears the flag.

**Gates.**

- `cargo build --workspace --all-targets`, clippy with `-D warnings` and `cargo fmt
  --all --check` are clean.
- `cargo test -p anthrex-daemon` passes: 34 binaries, 1053 tests, of which 724 are unit
  tests.
- `fallback.rs` is 218 lines and `tests/turns_stale.rs` 327.

### M8a.13 engine III: proof, check and review (2026-09-23)

Decisions 33, 34 (engine side), 35 and 38 (gate failures) are in the reducer. Decision
2's grep over every new file matches only doc comments.

**Module layout (beyond the brief's list, split by responsibility):**

- `engine/gates.rs`: the gate order (`next_gate`, `enter`), the proof and the check
  (their ops and results), and `run override` (below).
- `engine/review.rs` (new): reviewer dispatch and `PrepareReview`'s result (moved out
  of `dispatch.rs`, which was at 593 lines), `submit_review`, the reviewer's turn ends,
  exits, resume results and silence, and the verdict-less round.
- `engine/tools.rs` gains `parse_review`. `contract.rs` gains the gate messages.
- `ladder.rs` needed no new rung: `gate_failure` (M8a.12) already takes every gate. It
  gains the rung-1 clock restart and the T12-P2 recovery (below).
- Tests: `engine/tests/{gates,gates_review,gates_rounds}.rs`. The brief's single
  `gates.rs` would have been over 1 100 lines.

**Name corrections and additions** (each new field `#[serde(default)]`):

- **Reviewer permission mode (carry, M8a.7).** The brief's
  `claude_permission_mode == Some("plan")` is wrong: M8a.1 found that plan mode blocks
  the reviewer's allowed MCP call in `-p`, and M8a.7 moved reviewers to `dontAsk` with
  `Edit,Write,NotebookEdit` disallowed. `review_round_uses_a_fresh_session_and_worktree`
  asserts `Some("dontAsk")` and the disallowed tools.
- `ProofRecord.head`: `proof_failed_message` names `<head7>`, which the Interfaces'
  record did not carry.
- `Task.gate_op`: the `Proof`, `Check` or `PrepareReview` op the task awaits.
  `Task.review_misses`: verdict-less rounds in a row.
- New texts: `REVIEW_NUDGE`, `REVIEW_RECORDED` and `APPROVE_WITH_BLOCKING` (the brief's
  exact texts), `REVIEWER_STOPPED_TWICE` (exact), and `REVIEWER_RESUME_AFTER_EXIT`
  (invented: `[anthrex] Your session's process stopped in the middle of a turn and has
  been resumed. Finish your review and call submit_review, exactly once.`).

**Readings and choices (invented where marked):**

- **Gate order.** Proof (tdd), check (a profile check), review (a review level), then
  the merge queue; a handed-back task still goes straight to the merge queue. Proof
  and check ops are sent by the scheduler pass (`gates::start_gates`) for a task in
  that state that awaits nothing.
- **Correlation (carry, ruling T12-N).** Each gate result is taken only when its op id
  is the task's `gate_op` and the task is still in that gate. A task in no gate awaits
  nothing (`start_gates` clears `gate_op`), so a late result of an earlier visit is
  dropped even when the task has come back. A new `PrepareReview` waits for a stale
  one in flight (`holds_reader` already counted it), so one review worktree is
  prepared at a time. Reviewer windows are matched by `launch_op` (M8a.11), resumes by
  `resume_op`, and a reviewer window that arrives for a task no longer in `review`, or
  for a round already given up, is killed.
- **The proof (decision 33).** `command` is `proof_command(single_test, test)`,
  `passed` is `proof_pattern(test_passed, test)`, `path` the `<task>.proof` worktree,
  `env` the profile's with `{worktree}` that path, `setup` the profile's.
  - **No `test_passed` in the profile** (invented): the pattern is `{test}`, so the head
    run's output must show the test's escaped name on some line.
  - Passing needs all three: red failed, head passed, output matched. The first reason
    that fails is the message's; the red run's tail is quoted when the red run did not
    fail (carry from M8a.10: `head_tail` is empty then), else the head run's.
  - A claim with no `test` or `red` (only the turn-end fallback's can be: an explicit
    `task_done` without them is rejected at the done gate) fails the proof at once,
    with no op. Nothing ran, so the message leaves out its `Command:` and `Last 40
    lines:` lines (deviation from the table's layout): `[anthrex] The test proof
    failed: this is a tdd task and no test or red commit was named; call task_done with
    test and red` then `Fix it, commit, then call task_done again.` A `ProofRecord`
    with an empty test and red records it.
  - `SetupFailed` blocks the task on its environment with `setup failed in the proof
    worktree:\n<output>`; `Failed` with `could not run the test proof: <message>`
    (invented). Neither counts a failure.
  - **Executor contract (carry, T10-RR), stated on `OpKind::Proof`:** `run_proof` on
    `spawn_blocking` with the hook `|step| handle.block_on(queue.write(&project, step))`.
    `Handle::block_on` inside `spawn_blocking` needs the multi-thread runtime, which the
    daemon has (`#[tokio::main]` in `crates/cli/src/main.rs`, multi-thread by default);
    on a current-thread runtime it would deadlock.
- **The check (decision 34).** `Op Check { dir, command: check, timeout:
  check_timeout_secs, env, scratch }` (`dir` and `scratch`: fix round 1, below); the result becomes a `CheckRecord` (`on_candidate:
  false`). `check_failed_message`'s parenthesis is `exit <code>`, `timed out after <m>
  minutes` (`secs / 60`), or, for a run with no code that did not time out, `no exit
  code` (invented). A `Failed` result blocks on the environment (`could not run the
  check: <message>`, invented). No check in the profile skips the gate; the run is
  `unverified` from `build_run` (M8a.5).
- **Rung 1 restarts the stall clock** of the worker's round (invented): a turn left
  open while the gates ran would otherwise be interrupted at once. Test
  `a_rung_1_bounce_does_not_stall_a_turn_left_open`.
- **Review (decision 35).**
  - The reviewer is picked by `pick_reviewer` against the author's **current** route at
    every round, and stored in `review_route`. After rung 2 moved the author to the
    peer runtime, the build-time `review_route` would have put the reviewer on the
    author's runtime. Test `a_review_after_rung_2_is_picked_against_the_new_author`.
  - The round's `ReviewRecord` is created with the reviewer (base, head, route, no
    verdict), and `submit_review` fills it.
  - `submit_review`'s acceptance order: a reviewer round of the task with this window,
    then `a review for round <n> was already submitted`, then the live current reviewer
    (`this window is not the reviewer of task <id>`), then the task's state, then the
    arguments (`invalid arguments: findings[<i>]: …`, or `findings[<i>].<field>: …`,
    invented), then `approve` with a blocking finding.
  - A recorded verdict retires the reviewer (`RetireWindow`) and resets
    `review_misses`. Any critical or important finding is a gate failure of `review`
    with `review_changes_message` (only those findings); otherwise the task goes to the
    merge queue with the minor findings kept in the record. Every gate counts toward
    the same ladder (`failures_of_different_gates_share_the_ladder`).
  - **Verdict-less rounds.** A reviewer turn that ends without a verdict gets
    `REVIEW_NUDGE`; the next one ends the round: the reviewer is killed and a new round
    starts at the same level, no failure counted. The second such round in a row blocks
    the task as `environment` with `REVIEWER_STOPPED_TWICE`. A failed reviewer turn
    follows the worker's rules instead (fix round 1, ruling T13-I1, below).
  - **Exits.** A Claude reviewer whose process ends between turns is marked ended and
    resumed by the delivery of its nudge. A mid-turn death is resumed once with
    `REVIEWER_RESUME_AFTER_EXIT`; the second in the round, a death before a session
    id, or a failed resume ends the round without a verdict.
  - **Silence** (invented): a reviewer turn with no stream event for
    `stall_after_secs` ends the round without a verdict. Decision 32's interrupt and
    `stall_nudge` are the worker's; a reviewer already has its nudge.
  - **The reviewer's mail.** The nudge goes through the run's outbox (decision 29's
    gate, retries and block apply) addressed to `<task>.review`. Task ids cannot contain
    `.`, so no worker-side rule that matches messages by task id sees it. The mail is
    delivered only while the task is in `review` and is dropped when the round ends.
    A bug found by `a_claude_reviewer_that_exits_between_turns_is_resumed_with_its_nudge`
    (written after the code, red first): the resume op was recorded with the mailbox as
    its task, so its result was dropped. It now carries the task's id.
  - **Read-only (carry).** Codex reviewers: `read-only`, no writable roots; Claude
    reviewers: `dontAsk`, the write tools disallowed, no sandbox block; no
    `WatchWorktree` for a review worktree. The prompt never names the author's runtime
    or model (`the_reviewer_prompt_never_names_the_author`, over the whole built-in
    roster, the empty Codex model skipped).
- **Override (decision 35), the part this brief tests.** `run override` accepts a task
  in `review`, or `blocked` with an accepted claim's head, in a running or paused run:
  it stops a live reviewer, clears the block, sets `merged_without_approval` and sends
  the task to the merge queue. Reply (invented): `task <id> goes to the merge queue
  without review: <reason>`. A held task or a `dep_cancelled` one is refused (`task <id>
  waits for its dependencies; override it once they are merged`, invented), keeping N5.
  **Carry for M8a.15:** a blocked task whose commits no accepted claim recorded (a
  `head` of `None`) is refused here; M8a.15's
  `override_only_from_review_or_blocked_with_commits` must decide how to learn its
  commits (for example a `CountCommits`).

**Carries.**

- **T12-P2 (fixed).** `ladder::recover_sessionless`, on every scheduler pass: a
  `working`, unheld task whose latest worker round ended with no session id, not
  retiring, and with no fresh session pending, gets a fresh session at the same rung and
  route, no failure counted (`its session ended before it had an id to resume`). The
  outbox then moves the waiting messages into its prompt (ruling T12-I3). It covers the
  held Codex task of the carry and the same loss in a gate (a rung-1 message to a Codex
  session that died before `thread.started` while the task was in `proof`). Test
  `turns_holds::a_held_codex_task_that_lost_its_session_before_an_id_gets_a_fresh_one`:
  red before the fix at its `DiffSoFar` assertion (`a fresh session: [...]`, none sent).
- **T6-N5 on the new paths.** A task in `proof`, `check` or `review` cannot gain a
  dependency (`add_dep` needs `pending`, `queued` or `blocked`), and a held task is
  `blocked`, so it is never in a gate. A gate failure's rung 2 goes through
  `start_fresh_sessions`, whose `fresh_due` refuses a held task (pinned by M8a.12's
  `a_held_task_gets_no_fresh_session_until_its_hand_back`), and `recover_sessionless`
  skips a held task. Override refuses a held task. Review round 2 needs `review`, which
  a held task is never in.
- **A reviewer bounce's fresh session drops the old session's in-flight messages
  (M8a.12): confirmed for decisions 35 and 38.** Rung 2's hand-over prompt carries
  every bounce text (the failure record, decision 30) and the amended brief and
  criteria, which is all decision 38 names. An `answer` or wrap-up still queued is lost;
  recorded in the follow-ups under "From M8a.13".
- **Liveness.** `turns_fixes::assert_alive` now also checks the gate states
  (`assert_gates_alive`): a task in `proof` or `check` has an op in flight; one in
  `review` has an op in flight, a live reviewer turn, reviewer mail deliverable or in
  flight, or waits for a reader slot; one in `merge_queue` is queued (M8a.14 runs it).
  Every new test ends with it.
- **Carry for M8a.15.** Restore must clear `Task.gate_op` for an op dropped as
  `NotStarted`, or `start_gates` never re-issues it (decision 45's "tasks in proof,
  check or merge_queue re-issue their op").
- **Carry for M8a.22.** The proof executor (above), and the check's `scratch` (fix
  round 1, below).

**One earlier test changed.** `turns::turn_end_fallback_nudges_once_then_proceeds`
expected a fallback-claimed tdd task to stay in `proof`; the proof now fails at once, so
it asserts rung 1 with one proof record.

**TDD evidence.**

- The brief's 20 tests (as 24 functions, with `a_timed_out_check_says_so`,
  `a_gate_result_no_longer_awaited_is_dropped`,
  `a_gate_that_cannot_run_blocks_on_the_environment` and
  `a_rung_1_bounce_does_not_stall_a_turn_left_open`) were written against stubs (empty
  message texts, `submit_review is not available yet`, no gate op): `0 passed; 24
  failed`. Sample lines: `one Proof: [...]` (no gate op sent), `a_timed_out_check_says_so`
  against the empty stub, and the fallback test's `left: Proof right: Working`. The
  review tests failed first at the missing `Check` op upstream; the mutation run below
  shows each pins its own rule.
- The T12-P2 test failed first as above.
- `gates_rounds.rs`'s nine tests were written after the first green run, for guards
  the first mutation run found unpinned; the resume one found the mailbox bug above.
- **Mutations**, each applied and restored by a script from a WIP commit (since
  folded): 31 distinct mutants, 27 killed.
  - Killed: the proof's three flags (G3, G4 after its test case became
    red-passed-head-passed), the red tail (C1), the gate order's check and review
    (G5, G6), the fallback's failure (G7), the check's result (G8, G9), override's state
    rule, reviewer stop and mark (G10–G12), the rung-1 clock (L1, after the test also
    checked the bounce's own step), `recover_sessionless` and its no-escalation (L2,
    L3), `review_ready`'s op check (R1), the reviewer route (R2), already-submitted
    (R3), a given-up round's submit (R4), minor-only approval (R5), approve with a
    blocking finding (R6), the misses reset (R7), the nudge (R8), the block at two
    (R9), two deaths (R10), the silence watch (R11), the stale reviewer window (R12)
    and the verdict's mail drop (R14).
  - Equivalent in reachable states (kept as defence): G1 and G2, `gates::awaited`'s op
    check and `start_gates`' clear, for proof and check (a task leaves `proof` or
    `check` only through its own result or a cancel, so it never comes back with an op
    in flight; M8a.15's retry may make them reachable); R13, the mail's `review` guard
    (every path out of `review` drops the mail); R15, `review_ready`'s state check
    (`start_gates` clears `gate_op` outside a gate, so R1's check already drops it).

**Gates.**

- `cargo build --workspace --all-targets`, clippy with `-D warnings` and `cargo fmt
  --all --check` are clean.
- `cargo test -p anthrex-daemon --no-fail-fast` passes: 34 binaries, 1087 tests, of
  which 758 are unit tests. One earlier run failed once in `tests/git_registry.rs`
  (`a_commit_in_a_linked_worktree_triggers_a_probe`, the load-sensitive test M8a.11
  noted), which the engine does not touch; the full re-run passed.
- No file passes 600 lines: `review.rs` 423, `gates.rs` 354, `dispatch.rs` 511,
  `contract.rs` 531, `tests/gates.rs` 547, `tests/gates_review.rs` 582.

#### M8a.13 fix round 1

The review found 3 Important and 6 Minor issues. Each ruling's probe became a
regression test in `engine/tests/gates_fixes.rs`, run red first, and each ends with the
liveness check.

- **T13-I1: reviewers get the worker's failed-turn rules.** `review::turn_ended` takes
  the turn's outcome and the rate-limit streak. A `RateLimit` failure is not a
  verdict-less turn: the round waits `rate_limit_retry_secs` (`FailedTurn::WaitingContinue`,
  `rate_limited_until` set), is counted in `rate_limits` once per streak, and the
  watch then queues `rate_limit_continue(<error>)` to `<task>.review`.
  `Authentication`, `Billing` and `SandboxUnavailable` stop the reviewer and block the
  task as `environment` with the error's own text, no miss counted. `Other` waits
  and continues the same way, and blocks the task only when the previous failed
  turn was also `Other` with no completed turn between them (a completed turn resets
  `ContinueSent`). Tests `a_rate_limited_reviewer_waits_and_continues`,
  `a_reviewers_rate_limit_streak_counts_once`,
  `a_reviewer_auth_or_billing_failure_blocks_at_once`,
  `two_other_reviewer_failures_in_a_row_block`,
  `a_completed_reviewer_turn_resets_the_other_failure_count`.
- **T13-I2: a reviewer holds its reader slot until its process exits.**
  `schedule::holds_reader` counts every reviewer round that has not ended, including
  a retiring one, so the next round's `PrepareReview` waits for the old reviewer's
  exit. A retiring round ignores `TurnEnded`, so a Codex reviewer retired mid-turn
  keeps its turn open and its process exit ends the round. A Codex reviewer
  **between** turns has no process (one per turn), so giving it up (`review::give_up`,
  now shared by the verdict-less rule and `stop_reviewers`) ends its round at once
  with no `KillWindow` (deviation: three existing tests that expected a `KillWindow`
  for a Codex reviewer between turns now assert the ended round). Tests
  `the_next_review_round_waits_for_the_last_reviewers_exit` (a Claude reviewer, then a
  retired one) and `a_retired_codex_reviewer_ends_with_its_process`. Two existing tests
  gained the retired reviewer's exit before the next round.
- **T13-I3: the gates run on the claimed commit and worker mail is held in them.**
  `outbox` holds a worker's mail while its task is in `proof`, `check`, `review` or
  `merge_queue`. The mail goes out only once the task is `working` again, with the
  next bounce. The review worktree's `head_ref` is `task.head`, the claimed commit
  (the branch only when no claim recorded one). **Deviation from decision 34's "the
  task's worktree":** the check now runs in the `<task>.proof` scratch worktree,
  materialized at `task.head`. `OpKind::Check` gained
  `scratch: Option<ScratchAt { root, commit, setup }>`. Its doc states the executor
  contract for M8a.22. The executor goes through the same `git_write` hook as
  `run_proof`: it runs `prepare_scratch(root, dir, commit)`, then `setup` once per new
  worktree (with the `anthrex-setup-ok` marker), then `materialize(dir, commit)`, then
  the command. With `None`, the command runs in `dir` as it is, which is what M8a.14's
  final check in the integration worktree needs. A `SetupFailed` result blocks the
  task on its environment with `setup failed in the check worktree:\n<output>`
  (invented). Before this fix, `check_done` ignored that result, and the scheduler
  re-issued the check forever. Test
  `a_check_whose_setup_fails_blocks_on_the_environment`, red first: it showed a second
  `Check` op. A worker commit after `task_done` is not in the claim, so
  it does not enter the gates; the worker's next claim covers it. Tests
  `worker_mail_is_held_while_the_task_is_in_a_gate`,
  `worker_mail_is_held_in_the_merge_queue`, `the_gates_run_on_the_claimed_commit`.
- **Minors.**
  - m1: a tdd task whose profile has no `test_passed` gets a plan-time note
    (`validate::NO_TEST_PASSED_NOTE`, invented text citing rule 8.1). The shared test
    `PROFILE` in `test_support.rs` (and one inline profile in `plan_tests.rs`) now sets
    `test_passed`, so other tests' note lists are unchanged; the regex test replaces it
    instead of appending a duplicate key. Test
    `a_tdd_task_without_test_passed_gets_a_plan_note`.
  - m2: `override_refuses_a_held_task` (passed at red: the refusal already existed).
  - m3: rung 3 re-resolves `review_level`, `review_route` (against the task's current
    route) and `budget` for the raised size (`ladder::reresolve`, through
    `resolve_task_lenient`). Test `rung_3_re_resolves_review_and_budget_for_the_raised_size`.
  - m4: `review_misses` resets whenever the task is outside `review`. Test
    `the_verdictless_count_resets_when_the_task_leaves_review`.
  - m5: override does not stop the worker: its session keeps running (idle, its mail
    held by I3) until the merge or a later bounce. Only a live reviewer is stopped.
  - m6: `assert_gates_alive` accepts a `review` task that waits on a reader slot a hub
    task holds. Test `a_review_waiting_for_a_hub_is_alive`.

**Red first.** The red run had 11 failures: 10 of the first 12 new tests, and
`gates::proof_passes_to_check`, whose expected `Check` took the new form. Sample lines: the
`Check` op had `dir: t1, scratch: None` where `t1.proof` and `Some` were expected;
`no nudge` (a rate-limited reviewer was nudged); `t1 is in Review with nothing
pending`; rung 3's `review_level` was `None`, expected `Some(Medium)`; `(Blocked, 1)`
against `(Blocked, 0)` for the misses reset; the continue was never delivered.
`override_refuses_a_held_task` and `a_retired_codex_reviewer_ends_with_its_process`
passed at red and are kept as pins. The mutation run found five survivors. The three
tests added for them are `a_reviewers_rate_limit_streak_counts_once`,
`a_completed_reviewer_turn_resets_the_other_failure_count` and
`worker_mail_is_held_in_the_merge_queue`.

**Mutations.** 23 mutants of the fix; 21 killed (the `SetupFailed` arm's removal is
the red run of its test). Killed: the rate-limit branch, the
`Other` rule both ways, the reviewer stop on an environment failure, the rate-limit
count, the count on a streak, `rate_limited_until`, the continue's due time, the
`ContinueSent` reset, Codex's immediate end, the review `head_ref`, the retiring
reader slot, the mail hold, the mail hold in `merge_queue`, the check's commit, the
note both ways, `reresolve` and its route and budget, and the misses reset. Equivalent:
`scratch: None.or(Some(..))` (a no-op; the full `Check` kind is asserted), and a
`!round.retiring` guard in `signals::exited`. That guard was unreachable for the
reason given under T13-I2, so it was removed.

**Gates (fix round 1).** `cargo build --workspace --all-targets`, clippy with
`-D warnings` and `cargo fmt --all --check` are clean. `cargo test -p anthrex-daemon
--no-fail-fast`: 34 binaries, 1101 passed, 2 failed. The failures were
`tests/git_registry.rs`'s `a_commit_in_a_linked_worktree_triggers_a_probe` (the
load-sensitive test above) and `tests/server_git.rs`'s
`two_windows_in_one_worktree_register_once`, a socket wait in `support/mod.rs`. Both
binaries pass when re-run alone (10/10, 6/6), and the engine touches neither. Unit
tests: 328 of `run::` pass. No file passes 600 lines: `review.rs` 513,
`tests/gates_review.rs` 585, `tests/gates_fixes.rs` 471.

#### M8a.13 fix round 2

Re-review 1 approved I1–I3, the minors and the check-in-scratch deviation. It found
three minor issues (ruling T13-R2).

- **N1: a Codex reviewer given up after `turn.completed` but before its process
  exits.** Codex reports the turn's end before the process exits, so a closed turn
  does not mean there is no process. That is what fix round 1 assumed. `give_up` now
  ends a Codex round at once only when it has no pid. Otherwise it sends `KillWindow`,
  and the round keeps its reader slot until the exit. The `!round.retiring` guard in
  `signals::exited` is back: a natural exit of a retiring Codex round, arriving
  between turns, ends it. A Codex exit between turns now clears the round's pid,
  since the round has no process until the next one starts. Claude reviewers keep
  `KillWindow`: their process is long-lived, and a missing pid only means its start
  was not seen yet. Tests: `a_codex_reviewer_given_up_before_its_exit_holds_its_slot`
  (probe r1 inverted) and
  `a_codex_reviewer_between_processes_ends_at_once_when_stopped`. This replaces fix
  round 1's "a Codex reviewer between turns ends at once" and "the guard was removed".
- **Exits are matched by pid.** A `ProcessExited` whose pid is not the round's
  (`AgentRound.pid` set and different) is dropped before `exited`. A late exit of an
  earlier Codex process therefore no longer counts as a death of the current one,
  for a worker or a reviewer. With no pid recorded, the exit is taken as before.
  Tests: `a_workers_exit_from_another_process_is_dropped` and
  `a_reviewers_exit_from_another_process_is_dropped`. The follow-up entry is removed.
  One ordering is still open: an exit that arrives before the next process's
  `ProcessStarted` but after the delivery that opened its turn (new follow-up, for
  M8a.22).
- **N2.** `NO_TEST_PASSED_NOTE` moved above `resolve_task`'s doc comment.
- **N3: carry for M8a.14.** The merge takes `task.head`, the claimed commit that
  passed the gates, as `MergeCandidate`'s `task_head`. It never takes the tip of
  `anthrex/<run>/<task>`: a worker commit after the claim (a self-started turn, ruling
  T12-R4) would otherwise be merged without having passed the gates. When the tip
  differs from `task.head`, M8a.14 merges `task.head` and leaves the later commits to
  the worker's next claim, or bounces the task. It must not merge the tip.
- **Cost carry for M8a.22** (from the re-review): a check-mode or none-mode task now
  gets a scratch worktree and a cold first build for its check. Later checks are warm,
  because `clean -fd` keeps ignored build output.

**Red first.** The four new tests (`engine/tests/gates_exits.rs`) all failed against
`c2f5836`. `a_codex_reviewer_given_up_before_its_exit_holds_its_slot` had no
`KillWindow`. The two pid tests had `(deaths, ended, pid) = (1, true, None)` where
`(0, false, Some(..))` was expected, and the worker test had a `ResumeSession`. The
between-processes test had pid `Some(41)` where `None` was expected.

**Mutations.** 6 of 6 killed: `give_up` ignoring the pid, `give_up` for any runtime,
the retiring guard, the pid clear, the pid match, and dropping exits with no pid
recorded.

**Gates (fix round 2).** Build, clippy with `-D warnings` and `cargo fmt --all
--check` are clean. `cargo test -p anthrex-daemon --no-fail-fast`: 34 binaries, 1105
passed, 2 failed, the same two as in fix round 1:
`git_registry::a_commit_in_a_linked_worktree_triggers_a_probe` and
`server_git::two_windows_in_one_worktree_register_once`. The machine's load average
was about 20–24 (other sessions). With the worktree's default `target/`, `git_registry`
kept failing alone, taking 18 s. With a fresh `CARGO_TARGET_DIR` it passed in about
1.5 s, for this commit's code and for `c2f5836` alike. `server_git` passed alone on
its second try. The engine touches neither. `run::` unit tests: 332 of 332 pass.

### M8a.14 engine IV: merge queue, hand-back, ref guard, completion (2026-09-23)

Decisions 21 (engine side), 36 and 37 are in the reducer, with `run accept` and `run
discard` of a complete run (decision 20, engine side), which the brief's accept-conflict
test needs. Decision 2's grep over every new file matches nothing.

**Module layout (beyond the brief's list, split by responsibility):**

- `engine/merge.rs`: the queue (`start_merge`), `MergeCandidate`'s results, the merge
  queue's hand-back, the halt, `BaseAdvanced`, and `run resume --rebaseline`.
- `engine/complete.rs` (new): completion (`VerifyRefs`, the final check, `complete`),
  the `finish` edit, `run cancel`, and `run accept`/`discard` with their results.
  `requests::discarded` moved here as `finished`, shared by `run reject`.
- Tests: `engine/tests/{merge,merge_complete,merge_holds}.rs`. The brief's single
  `merge.rs` would have been over 1 100 lines.

**Name corrections and additions** (each new field `#[serde(default)]`):

- `Task.merge_op`: the `MergeCandidate`, or the merge queue's `HandBack`, the task
  awaits (ruling T12-N's correlation). An N5 `HandBack` never sets it, so `op_done`
  routes a `HandBack` result to `merge::handed_back` only when the task awaits that op,
  else to `holds::handed_back` as before.
- `Run.finish_edit` (the `finish` edit was applied) and `Run.finish_reply` (the
  `run accept`/`discard` request waiting for its op; `restore` clears it).
- `OpResult::HandedBack` gains `head: Option<String>`, the worktree's `HEAD` after the
  hand-back. A clean hand-back commits a merge on the task branch, so re-queuing the
  old `task.head` would conflict again and block the task; the clean head re-queues
  instead. Every existing test literal gained `head: None`.
- `OpResult::Finished` gains `kept_branches: Vec<String>` (carry T9): the names
  `delete_branches` skipped. They are logged (`branches kept because a worktree has
  them checked out: …`), so the report carries them.
- `dispatch::removed` takes the removed path: only the task's own worktree clears
  `worktree_live`, now that a merge also removes the review and proof worktrees.
- New texts in `contract.rs`: `candidate_red_message` (exact, Interfaces) and
  `accept_conflict_message` (exact, decision 20).
- Executor contracts are on the op docs (`MergeCandidate`, `HandBack`,
  `RemoveWorktree`, `VerifyRefs`, `Accept`, `Discard`): which steps are writes through
  `GitQueue::write`, and that **the executor must never wrap `accept` in a timeout
  shorter than `git::ACCEPT_MERGE_TIMEOUT`** (carry T9).

**Readings and choices (invented where marked):**

- **The queue.** Width 1 (any `MergeCandidate` pending), FIFO in `Run.merge_queue`.
  The head stays in the queue while its candidate runs; each pass first drops ids
  whose task is no longer in `merge_queue`. The candidate's `task_head` is `task.head`,
  the claimed commit (carry T13-R2 N3); `message` is `anthrex: merge <task>: <title>`;
  `check`, `timeout_secs` and `env` are the profile's, `env` with `{worktree}` the
  integration path.
- **`Merged`** sets `run_head` and `last_green_candidate` whatever became of the task
  (the run ref did move); a task cancelled while its merge ran stays cancelled and the
  merge is logged. For the awaited task: `merged`, `merge_commit`, the worker's live
  rounds retiring with `RetireWindow` (a Codex round with no process ends at once),
  its outbox mail dropped, `UnwatchWorktree` for the task worktree, then
  `RemoveWorktree` for the task, review and proof worktrees with consecutive salvage
  refs. `<seq>` starts after the highest recorded one (`merge::next_salvage_seq`), not
  after the count: only dirty worktrees record their ref.
- **Conflict.** The first gives `HandBack { worktree, run_head }`; the task stays in
  `merge_queue` state, out of the queue, until the result. Conflicts in it: the
  conflict message, `working`, `handed_back = true`, the worker round's stall clock
  restarted (as at rung 1). Clean: `task.head` becomes the result's `head`, and the task
  goes to the back of the queue (superseded by fix round 1, T14-C1: only when the merge
  was made onto the claimed commit). `Failed`: `blocked(environment)`, `could not merge the
  run head into its worktree: <message>`. The second conflict:
  `blocked(conflict)`, `its branch conflicts with the run branch again: <files>`
  (invented); the session is left alive, as for a question, for retry, override or
  cancel. Neither counts a failure.
- **Red candidate**: a `CheckRecord` with `on_candidate: true`, then
  `ladder::gate_failure(Merge, candidate_red_message)`.
- **`Failed` candidate** (invented): `blocked(environment)`, `could not merge: <message>`.
- **Halt.** `RefMoved` from a candidate or from `VerifyRefs` halts the run with the
  reason, whichever task's op found it. The awaited task stays at the queue's head. A
  `VerifyRefs` that fails also halts, with `could not verify the refs: <message>`
  (invented), so completion is never decided on refs that were not read (superseded by
  fix round 1, review m1: read again once, then a retryable halt).
- **Resume.** A halted run without `rebaseline` is refused: `run <id> is halted:
  <reason>; check the refs, then resume with --rebaseline` (invented). With it:
  `base_sha` and `run_head` take the driver's values, `base_moved` and
  `halted_reason` are cleared, the run is `running`; the reply is `run <id> resumed
  with --rebaseline: base <base> at <b7>, run head <h7>`. A paused run's resume stays
  M8a.15's (`run resume is not available yet`).
- **`BaseAdvanced`.** Ignored for a terminal run, for `to == base_sha`, and for the
  `to` already recorded (so no revision bump); otherwise `base_moved` is set (from
  `base_sha`) and logged. The snapshot's attention line already existed (M8a.11).
- **Completion.** Each pass of a running run: every task `merged` or `cancelled`, the
  queue empty, **no op pending, and no cancelled task with a session still ending**
  (else the `KillWindow`'s exit and the F5 removal would come after `VerifyRefs`).
  `RefsOk` in a running run: the final `Check` (task-less, `scratch: None`, in the
  integration worktree) when `run_head` is neither `last_green_candidate` nor
  `base_sha` and the profile has a check; else `complete`. The final check's result, or
  its failure to run, sets `final_check_failed`; the run completes either way.
  `complete` logs `complete: <m> merged, <c> cancelled` and emits `WriteReport`. A
  result that arrives while the run is not running is dropped, and the next running
  pass verifies again.
- **The `finish` edit.** It sets `finish_edit` and logs. Each running pass: every
  unfinished task with no `start_commit` is cancelled (`has not reached working`, read
  as never started, so a blocked-in-setup task counts); once no task is `preparing`,
  `working`, in a gate or in the merge queue, every `blocked` one is cancelled.
- **Cancel.** `run cancel` of a running, halted or paused run (a paused one becomes
  `running`, so it can complete; a halted one completes after `resume --rebaseline`,
  since completion runs the ref guard). Every unfinished task is cancelled through the
  shared `cancel_task`: `ladder::kill_worker` (claim answered, ops superseded),
  `review::stop_reviewers`, messages dropped, out of the queue. The F5 clean-up then
  salvages and removes each worktree after its session exits. Reply (invented): `run
  <id> cancelled; it completes once its sessions have ended`. Other states are refused
  with `run <id> is <state>`.
- **Accept and discard.** Only a `complete` run (`run <id> is <state>; <accept|discard>
  applies only to a complete run`, invented; cancel first). The op lists every task's
  own, review and proof worktree, then the integration worktree, each with its next
  salvage ref; the executor skips one that no longer exists (as `run reject`'s list
  already needed). `expected_base` is `base_moved.to` when the base advanced, else
  `base_sha`; the driver sends `BaseAdvanced` before `Finish` when its read finds a
  newer head (documented on `OpKind::Accept`). The reply waits for the result:
  `Finished` makes the run `accepted` or `discarded`, sends `RemoveWindow` for every run
  window and `WriteReport`, and replies `run <id> <accepted|discarded>: <outcome>`;
  `AcceptConflict` replies `Err` with decision 20's text, logs it and leaves the run
  `complete`; `Failed` logs and replies `Err`.

**Carries.**

- **T13-R2 N3 (merge `task.head`): done.** `a_worker_commit_after_task_done_is_not_merged`:
  a self-started worker turn while the task waits in the queue sends no count, and the
  candidate names the claimed head. The engine never learns a later tip.
- **T9 (skipped branches, `ACCEPT_MERGE_TIMEOUT`): done**, above.
- **Every git write through `GitQueue::write`**: stated on each op the executor runs.
- **T11-RR, the part in reach**: `a_conflict_blocked_task_that_gains_a_dependency_keeps_its_block`.
  A `blocked(conflict)` task that gains a dependency is held (`awaiting_deps`), but
  `resume_held` hands back only a `blocked(question)` task, so it keeps its own block
  when the dependency merges. **Carry for M8a.15:** a retry of such a held task must
  hand back first and must not turn it into `blocked(question)`.
- **T11-RR2**: override already refuses a `dep_cancelled` or held task (M8a.13); retry
  is M8a.15's. No path in M8a.14 lifts `dep_cancelled`.
- **T6-N5 on the hand-back path.** A task in `merge_queue` (candidate or hand-back in
  flight) cannot gain a dependency (`add_dep` needs pending, queued or blocked), and a
  clean hand-back re-queues without passing through `working`.
  `a_handed_back_task_held_by_a_new_dependency_is_handed_back_again_first` covers a
  handed-back task that blocks on a question and gains a dependency: the dependency's
  merge brings the N5 hand-back (not the queue's), and its next `task_done` still goes
  straight to the queue.
- **Liveness.** `assert_gates_alive` accepts a `merge_queue` task with an op in flight
  (candidate or hand-back) or queued; the new `assert_run_alive` requires a running run
  whose tasks are all finished to have an op pending or a cancelled task's session
  still ending, a running run with a queue to have a `MergeCandidate` in flight, and a
  halted run to have a reason. Every new test ends with `assert_alive`.
- **Carry for M8a.15.** Restore must re-issue a dropped `MergeCandidate` or `HandBack`
  (clear `Task.merge_op` for an op dropped as `NotStarted`, as for `gate_op`), and a
  dropped `VerifyRefs` or final `Check` is re-issued by the next running pass on its
  own (no op pending). `finish_reply` is already cleared.

**TDD evidence.**

- The brief's 12 tests, plus `a_conflict_blocked_task_that_gains_a_dependency_keeps_its_block`
  and `a_worker_commit_after_task_done_is_not_merged`, were written against stubs
  (routing wired, bodies empty, empty message texts): `330 passed; 17 failed` in
  `run::`. Sample lines: `one pending MergeCandidate for Some("t1"): []` (11 tests),
  `left: Queued` (the finish edit), cancel's `Err("run cancel is not available yet")` reply, and two existing
  tests, `gates_fixes::worker_mail_is_held_in_the_merge_queue` and
  `gates_exits::a_codex_reviewer_between_processes_ends_at_once_when_stopped`, failing
  at the extended liveness check (`a merge queue ["t1"] with no merge in flight`), which
  pass once the queue runs.
- Written after the first green run, each shown to pin by a mutant below:
  `salvage_numbers_follow_the_highest_recorded_ref`,
  `a_handed_back_task_held_by_a_new_dependency_is_handed_back_again_first`,
  `a_merged_task_leaves_no_mail_behind` (for surviving mutant M23), the paused case of
  `cancel_kills_salvages_and_completes`, and the path check in
  `merged_updates_run_head_cleans_up_and_retires_the_worker`.

**Mutations**, each applied and restored by a script from a WIP commit (since
folded): 25 run, 25 killed (M23 after its test was added). Killed: width 1 ignored
(M1), the branch merged instead of `task.head` (M2), the first conflict blocking (M3),
`RefMoved` not halting (M4), the same `to` re-recorded (M5), completion before a
killed session's exit (M6), no final check (M7), `finish` cancelling blocked tasks
while others are live (M8) or cancelling started ones (M25), no `RetireWindow` (M9),
the `merge_op` correlation (M10) and the `HandBack` routing (M17), which the N5 tests
catch, the clean hand-back's old head (M11), salvage numbers by count (M12), completion
with ops pending (M13), accept onto `base_sha` (M14), a paused cancel (M15),
`worktree_live` cleared by any removal (M16), a red candidate that is not a failure
(M18), a halted resume without `--rebaseline` (M19), `base_moved` kept by a rebaseline
(M20), no `handed_back` flag (M21), a moved ref at completion that completes (M22),
the merged task's mail kept (M23), no `UnwatchWorktree` (M24).

**Gates.** `cargo build --workspace --all-targets`, clippy with `-D warnings` and
`cargo fmt --all --check` are clean. `cargo test -p anthrex-daemon --no-fail-fast`:
34 binaries, 1125 passed, 0 failed (after M8a.13's `cargo clean -p anthrex-daemon`,
`git_registry` and `server_git` passed in the full run). No file passes 600 lines:
`merge.rs` 377, `complete.rs` 354, `dispatch.rs` 519, `contract.rs` 551, `model.rs`
538, `tests/merge.rs` 510, `tests/merge_complete.rs` 482, `tests/merge_holds.rs` 162.

#### M8a.14 fix round 1

The review (`task-14-review.md`) found 1 Critical, 4 Important and 5 Minor issues.
Rulings T14-C1, T14-I1..I4 and T14-minors apply. Each probe became a regression test,
red first, in `engine/tests/merge_fixes.rs` (10 tests) or `merge_holds.rs` (3 more),
and each ends with the liveness check.

- **T14-C1: a hand-back re-queues only a merge made onto the claimed commit.**
  - `OpKind::HandBack` gains `task_head` (`Task.head`, what the engine expects the
    merge onto). `OpResult::HandedBack` gains `onto`, the tip the executor actually
    merged onto. `git::hand_back` now reports it (M8a.9 notes, "Later change").
  - `onto == task.head`: clean re-queues the merged head; conflicted sets
    `handed_back`, so the resolution goes straight back to the queue (decision 36).
  - Anything else (a tip past the claim, or `onto` unknown): the task goes back to
    `working` with `handed_back = false`. Clean: `UNCLAIMED_COMMITS` (invented: `[anthrex]
    The run branch was merged into your worktree, but your branch has commits after
    your last task_done, and they have not passed the gates. Check the result, commit,
    then call task_done again.`). Conflicted: the conflict message. Either way its next
    claim passes every gate.
  - **Spec risk, recorded as the review asks:** a conflicted hand-back onto the claimed
    commit goes straight back to the queue after the worker's resolution, as decision
    36 says. The resolution claim may carry commits beyond the conflict resolution;
    only the candidate check re-tests them.
  - Test `a_hand_back_onto_a_post_claim_commit_goes_back_through_the_gates` (clean and
    conflicted). Red: a `MergeCandidate` with the smuggled head was sent.
- **T14-I1: a cancel during a merge is deferred.** `Task.cancel_deferred`. A
  `cancel_task` edit (`edits.rs`) or `run cancel` of a task whose `MergeCandidate` is in
  flight only marks it (`cancel deferred: its merge is in flight` in its history). A
  merge that lands makes it `merged` and logs `the cancel of <id> arrived too late: it
  merged`; any other result (conflict, red, failure, refs moved) applies the cancel,
  including `blocked(dep_cancelled)` on its dependents (`complete::cancel_now` now does
  what the edit does). `run cancel`'s reply adds `; <id>'s merge is in flight: it is
  cancelled only if that merge does not land` (invented); an edit's reply stays
  `applied <n> edit(s)`, and the task's history says it was deferred. Tests
  `a_cancel_during_a_merge_that_lands_is_too_late` (report `complete: 1 merged, 0
  cancelled`) and `a_cancel_edit_during_a_merge_applies_only_if_it_does_not_land`
  (merged, conflict, refs moved). Red: `left: Cancelled`, and a reply without the
  deferral.
- **T14-I2: no session starts unless the run is running.**
  - A `PrepareWorktree` result for a `preparing` task records `Task.ready_from` and
    launches nothing. The first running pass (`dispatch::launch_ready`) launches the
    worker from that commit, or re-points the worktree when the run head moved (a
    rebaseline).
  - A `PrepareReview` result records a history line and starts no reviewer; the
    running pass prepares the review again (idempotent, and its diff, which can be
    large, is not kept in `run.json`), then starts the reviewer. Deviation from "recorded"
    read literally: the worktree is recorded as prepared, the diff is not.
  - A `DiffSoFar` result starts no fresh session; `start_fresh_sessions` asks again
    once running. (Beyond the ruling's two; mutant N18 showed it unguarded.)
  - `assert_gates_alive` accepts a `review` task of a halted or paused run.
  - Tests `a_worktree_prepared_while_halted_starts_no_worker`,
    `a_review_prepared_while_halted_starts_no_reviewer`,
    `a_fresh_session_waits_for_the_run_to_run`. Red: a `CreateWindow` while halted.
- **T14-I3: no hand-back into a worktree whose told conflict is being resolved.**
  - `Task.resolving` is set when the queue's conflicted hand-back queues its conflict
    message, and cleared by the next accepted claim, by a cancel, and when
    `abort_untold_conflict` undoes an untold one.
  - `resume_held` of a `resolving` task sends no `HandBack` (`holds::relax`): the hold
    is lifted, an answered task goes back to `working` with its answer delivered (N5
    relaxed for the worker's own conflict turn), and `Task.handback_due` is set. An
    unanswered one goes back to its question.
  - At the next accepted `task_done`, before any gate, `merge::hand_back_due` sends
    `HandBack` for the new run head (`task_head` the new claim), with the task in
    `merge_queue` (out of the queue) and `gates_after_handback` set. Clean onto the claim:
    the merged head goes through every gate (`next_gate(None)`). Conflicted: the conflict
    message, and the resolution goes through the gates too.
  - The old test `a_handed_back_task_held_by_a_new_dependency_is_handed_back_again_first`
    fed a clean result real git could not give (the worktree was mid-merge); it is
    replaced by `a_told_conflict_is_not_handed_back_into_when_the_task_is_held`, whose
    conflict message is acknowledged (`Delivered`) before the hold. Guards pinned by
    `a_resolved_conflict_does_not_skip_a_later_hand_back` and
    `an_untold_conflict_undone_on_hold_is_handed_back_as_usual`.
- **T14-I4:** `accept_and_discard_apply_only_to_a_complete_run`: accept and discard on a
  running, halted or paused run reply `Err` and emit no op. It passed at red: it pins
  the existing guard, which the reviewer's M8 showed unpinned.
- **Minors.**
  - m1: a failed `VerifyRefs` is read again by the next pass (`Run.verify_failures`);
    the second failure in a row halts with `could not verify the refs: <message>` and
    `Run.halt_retryable`, and a plain `run resume` (no `--rebaseline`) resumes it
    without re-recording refs. Test `a_failed_ref_read_is_retried_then_halts_with_a_plain_resume`.
  - m2: `run cancel` sets `Run.cancelled`; a cancelled run that is halted, with every
    task finished and no op pending, can be discarded without a rebaseline. Accept stays
    complete-only. Covered in the I4 test.
  - m3: `an_advance_to_the_recorded_base_is_no_advance` (passed at red; kills M16). The
    reviewer's M10, M12, M14 (and M9) stay equivalent: re-run as R10, R12, R14, R9, all
    survive, because a task leaves `merge_queue` only through a cancel, which is now
    deferred for a candidate and clears `merge_op` for a hand-back, and no run is paused
    yet. **Carry for M8a.15:** pause and restore make them reachable; add correlation
    tests then.
  - m4: `remove_cancelled_worktrees` uses `next_salvage_seq`. Test
    `a_cancelled_worktree_takes_the_next_salvage_number`.
  - m5: `OpKind::Accept`'s doc: `Failed` means the base is untouched; a failure after
    `git::accept` merged is still `Finished`, with `accepted; clean-up failed: <error>`.

**Red first.** With the tests added and no engine fix: `350 passed; 9 failed` in `run::`.
Sample lines: `left: "refs/anthrex/salvage/engine-test-3f9a/t1/2"` (m4); a
`MergeCandidate` after the unclaimed clean hand-back (C1); `left: Cancelled` (I1); the
cancel reply `run engine-test-3f9a cancelled; it completes once its sessions have ended`
(I1); `left: Halted` (m1); a reviewer `CreateWindow` while halted (I2); an N5
`HandBack` into the mid-merge worktree (I3); discard of the cancelled halted run refused
(m2). The m3 and I4 tests passed at red (pins). The git test failed to compile against
the old `Vec<String>` return.

**Mutations** (script from a WIP commit, since folded): 23 new mutants (N1–N23), all
killed, N18 and N20–N23 after the three `merge_holds.rs` tests and two test extensions
were added for them. The reviewer's M8, M13 and M16 are now killed (N15, N12, N16).

**Gates.** Build, clippy with `-D warnings` and `cargo fmt --all --check` are clean.
`cargo test -p anthrex-daemon --no-fail-fast`: 34 binaries, 1138 passed, 0 failed. No
file passes 600 lines: `merge.rs` 458, `complete.rs` 418, `dispatch.rs` 553,
`edits.rs` 569, `model.rs` 573, `tests/merge_fixes.rs` 520, `tests/run_git_merge.rs`
598.

#### M8a.14 fix round 2

The re-review (`task-14-rereview-1.md`) found N1–N4 and two carried items (#3, #4).
Ruling T14-R2 applies. Each probe became a regression test, red first, in the new
`engine/tests/merge_fixes2.rs` (6 tests) or `tests/run_git_handback.rs` (3 real-git
tests); each engine test ends with the liveness check.

- **#3: straight to the queue only for the resolution and nothing more** (resolves the
  fix-round-1 spec risk).
  - A conflicted hand-back onto the claimed commit (queue's, not due) sets
    `handed_back` and `Task.resolution = ResolutionAt { onto, run_head, files }`; the
    `HandBack` op's `run_head` is passed through `op_done`. `resolution` is set exactly
    while `handed_back` is.
  - The next `VerifyDone` carries `resolution` (serde default `None`). The executor
    answers `DoneChecked.resolution_only` from `git::resolution_only` (M8a.9 notes);
    `None` when it was not asked.
  - `done::accept` goes straight to `merge_queue` only when `handed_back` and
    `resolution_only == Some(true)`; any other claim takes `next_gate(None)`. Both
    flags clear on every accept.
  - A due hand-back's conflict (`gates_after_handback`) never sets `handed_back`: even a
    pure resolution passes the gates there, since the claim before it never did
    (pinned by `a_told_conflict_is_not_handed_back_into_when_the_task_is_held`, now
    claiming with `resolution_only: Some(true)`).
  - Tests `a_resolution_skips_the_gates_only_when_it_is_only_the_resolution` and the
    extended `task_done_runs_verify_done_and_replies_after_it` (`Some(true)` →
    `merge_queue`; `Some(false)` and `None` → `proof`). Red: `resolution left: None`.
- **N1:** the N5 hand-back's conflict arm (`holds::handed_back`) sets `resolving`, so
  a later hold of a worker resolving it sends no second `HandBack` into the mid-merge
  worktree. Test `an_n5_told_conflict_is_not_handed_back_into`. Red: the `resolving`
  assertion.
- **N2:** the hand-back context ends (`ladder::end_hand_back` clears `handed_back` and
  `resolution`) in `abort_untold_conflict` and in `ladder::supersede`, which every
  replaced or stopped worker session goes through (`kill_worker` for rungs 2 and 3 and
  every other kill, `dispatch::launch` for every new session, a failed resume). The
  successor's claim passes every gate. Tests `an_undone_conflict_ends_the_straight_to_queue_pass`
  and `rung_2_during_a_resolution_ends_the_straight_to_queue_pass`. Red: `assertion
  failed: !fx.task("t1").handed_back`.
- **N3:** `done::accept` with `handback_due` only parks the task (`merge_queue`, out of
  the queue, `gates_after_handback`) and emits nothing. The running pass
  (`merge::start_due_hand_backs`, before `start_merge` in `dispatch::schedule`) sends
  the `HandBack` with the run head of that moment and clears `handback_due`. A claim
  accepted while halted waits for the resume, and a rebaseline's new run head is the
  one handed back. `assert_run_alive` accepts a `merge_queue` task with `handback_due`
  in a halted or paused run. Test `a_due_hand_back_waits_for_the_run_to_run`. Red: a
  `HandBack` emitted while halted.
- **N4:** `git::hand_back` reads `onto` as `HEAD^1` after a clean merge (M8a.9 notes).
- **#4:** a `cancel_task` edit deferred behind an in-flight merge adds `; <id>'s merge is
  in flight: it is cancelled only if that merge does not land` to `applied <n> edit(s)`
  (the same text as `run cancel`, `complete::deferred_note`). Only cancels newly
  deferred by this batch are named. Test `a_deferred_cancel_edit_says_so`. Red: `left:
  [Ok("applied 1 edit")]`.

**Carry for M8a.15.**
- Retry (`run retry`) clears `handed_back`, `resolution`, `resolving`,
  `handback_due` and `gates_after_handback` on the retried task.
- Restore honours `cancel_deferred` (a deferred cancel whose `MergeCandidate` did not
  survive the restart is applied) and due hand-backs (a `merge_queue` task with
  `handback_due` is sent again by the running pass; one with an in-flight `HandBack`
  that did not survive needs `merge_op` cleared and `handback_due` set again).
- The executor maps `VerifyDone.resolution` to `git::resolution_only` and fills
  `DoneChecked.resolution_only`.
- Mutants R9, R10, R12, R14 stay equivalent until pause and restore exist (see fix
  round 1, m3).

**Mutations** (scratchpad script, not committed): 17 new mutants over the new guards,
all killed: the `resolution_only == Some(true)` condition (ignored, `None` as true),
`VerifyDone.resolution` not sent, a wrong `run_head` in it, `handed_back` without the
claim check or after a due hand-back, N1's `resolving`, N2's two clears, N3's running
pass (removed, ignoring the task state, keeping the due flag), #4's note (removed,
repeated for an earlier deferral), the git parent check, the diff subset, and N4's
`HEAD^1`. Three written guards were redundant, their mutants surviving, and were
removed: a `handed_back` filter on `resolution` (the two are set together), a
`!files.is_empty()` term (the clean claimed case returns earlier), and a
`merge_op.is_none()` term in the running pass (the due flag is cleared on sending).
Fix round 1's 23 mutants were re-run: all still killed; R9, R10, R12, R14 still survive.

**Gates.** Build, clippy with `-D warnings` and `cargo fmt --all --check` are clean.
`cargo test -p anthrex-daemon --no-fail-fast`: 35 binaries, 1147 passed, 0 failed. No
file passes 600 lines: `merge.rs` 488, `done.rs` 492, `dispatch.rs` 554, `model.rs` 577,
`tests/merge_fixes2.rs` 365.

#### M8a.14 fix round 3

Re-review 2 (`task-14-rereview-2.md`) approved N1–N4 and #4. It found R2-1 (Important),
R2-2 (Minor) and a nit; ruling T14-R3 folds in two of its out-of-scope items. Every
test was red first.

- **R2-1: gitlinks.** `--ignore-submodules=none` joins the shared `DIFF_FLAGS` (M8a.9
  notes). Before this, a merge that also moved a gitlink passed `resolution_only` under
  `diff.ignoreSubmodules=all` or a `.gitmodules` `ignore = all`, and `verify_done` did
  not see a gitlink outside `owns` or at a protected path. Red: `assertion failed:
  !only(...)` and `left: []` against `right: ["vendor/lib"]`.
- **Replace refs.** `GIT_NO_REPLACE_OBJECTS=1` in the shared env scrub (M8a.9 notes).
  Red: the evil merge read as the pure one behind `refs/replace`.
- **R2-2:** `onto = HEAD^1` only when `HEAD^2` is the run head (M8a.9 notes). Red: `onto`
  was the claim's parent after an "Already up to date" hand-back.
- **Nit:** a `cancel_task` edit clears `handback_due` (`edits.rs`), as `run cancel`
  does. Test `a_cancel_edit_clears_a_due_hand_back`, ending with the liveness check.
  Red: `assertion failed: !fx.task("t1").handback_due`.
- **Executor contract (M8a.22).** `OpKind::VerifyDone`'s doc now says a
  `resolution_only` error counts as `Some(false)`: the gates run, and the error never
  fails the `DoneChecked`.

**Mutations:** 6 new mutants, all killed:
- dropping `--ignore-submodules=none`
- dropping `GIT_NO_REPLACE_OBJECTS`
- `onto = HEAD^1` always
- `HEAD^1` for any merge commit (killed by the claim-is-a-merge case)
- `onto = HEAD` always
- the edit keeping `handback_due`

**Scope note.** `verify_done`'s `status --porcelain` (the dirty count) still follows the
submodule config. An uncommitted gitlink change is never merged, so it is left out here
and recorded as a follow-up.

### M8a.15 engine V: edits, retry, override, pause, restore and resume (2026-09-24)

Decisions 13 (engine side), 28, 35's override, 42, 45 and 46 (engine side) in the
reducer. `run retry`, `run resume`, the `pause` and `resume` edits and the restore after
a daemon restart replace the M8a.11 `not_yet` stubs.

**Layout.**
- New: `engine/restore.rs`, which holds the restore, `run resume` for a paused run,
  `unpause` and the relaunch of lost launches.
- `run retry` is in `requests.rs`, and the override is in `gates.rs`.
- The tests are split by responsibility for size:
  - `tests/control.rs`: edits, answers, pause, a paused run's refusals, stop.
  - `tests/control_retry.rs`: retry and override.
  - `tests/control_restore.rs`: restore and resume.
  - `tests/control_resume.rs`: what a resume must not do, and what a restart must not
    lose.
  - `tests/liveness.rs`: the liveness check, moved out of `turns_fixes.rs`, which
    re-exports it.

**Name corrections and additions.**
- The brief lists three files. The change also touches:
  - `mod.rs` (routing);
  - `gates.rs` (the override);
  - `holds.rs` (a retried held task's hand-back);
  - `merge.rs` (the paused arm of `resume` moved to `restore.rs`);
  - `dispatch.rs` (the running pass relaunches);
  - `review.rs` (`owes_verdict` is `pub(super)`);
  - `schedule.rs` (`holds_reader`, below);
  - `ops.rs` (`OverrideCount`);
  - `contract.rs` (`RESUME_WORKER` and `RESUME_REVIEWER`, with the Interfaces texts);
  - `model.rs`, `plan.rs` and `validate.rs`.
- New persisted fields, each `#[serde(default)]`:
  - `AgentRound.relaunch`: a `CreateWindow` the restart lost.
  - `Task.override_count`: the count an override awaits, with the reply id and reason.
  - `Run.restored`: the time of the restart that ended the sessions.
  - `Run.paused_at`: when the run was paused.
- `resume_resumes_sessions_and_reissues_gate_ops` uses the fixture's own window ids and
  the session ids `s-tw` and `s-rw`. The brief's "worker 4, session `s-4`" and
  "reviewer 6" are illustrative.

**Invented texts.**
- Pause and resume edits: `run <id> is <state>; only a running run can be paused` and
  `run <id> is <state>; only a paused run can be resumed`. Either refusal rejects the
  whole batch.
- Retry refusals, in this order:
  - `run <id> is being <accepted|discarded>`;
  - `run <id> is <state>` (only a running or paused run takes a retry);
  - `unknown task <id>`;
  - `task <id> is <state>; retry applies only to a blocked task`;
  - `task <id> is blocked(dep_cancelled); retry cannot bring back a cancelled
    dependency`;
  - `task <id> is L; split it first` (decision 12's text);
  - `task <id> waits for its dependencies; retry it once they are merged`.
- Retry reply: `task <id> retried at rung 2: <how>`, where `<how>` is one of:
  - `it is dispatched again` (never started);
  - `the run head is merged into its worktree first` (held);
  - `a fresh session starts`.
- Retry history: `retried by the user at rung 2 (it was blocked(<label>): <text>)`. The
  fresh session's reason is `the user retried it (it was blocked(<label>): <text>)`.
- Override:
  - Refusal suffix: `override applies only to a task in review, or blocked with
    commits`.
  - `task <id> has no commits; <suffix>`.
  - `task <id>'s commits are being counted for an override; wait for its reply`.
  - `could not count task <id>'s commits: <message>`.
- Resume of a paused run: `run <id> resumed`, plus ` with --rebaseline: base <branch> at
  <sha7>, run head <sha7>` when rebaselined (the halted form's pattern).
- Log and history lines:
  - `restored after a daemon restart; paused`
  - `paused by a plan edit`
  - `resumed`
  - `launching the session the restart lost`
  - `its reviewer had no session to resume; a new round`
  - `an accept or discard did not finish before the restart; request it again`
  - `its cancel, after its merge did not survive the restart`

**Deviations and decisions.**
- **A paused run takes edits, retry and override.** Only a running run starts a
  session (ruling T14-I2). A retry while paused leaves its fresh session for the resume.
- **Restore.**
  - Every running run becomes `paused`.
  - Every session ends through `end_round`, which clears `interrupted`.
  - These are cleared: the claim, the override count, a `Counting` fallback, `count_op`,
    `count_retry_at`, `resume_op`, `carried`, `finish_reply` and every outbox
    `delivered_at`.
  - Each pending op the journal did not replay is dropped (decision 44), and its task
    gets what it needs instead:
    - `CreateRunBranch`, `PrepareWorktree`, `AbortMerge` and `RemoveWorktree` are
      re-emitted under a new id.
    - A lost `CreateWindow` is kept in `AgentRound.relaunch`. The first running pass
      re-issues it with a new op id and, for Claude, a new uuid. If its task no longer
      wants it, the round ends instead.
    - A lost `MergeCandidate` clears `merge_op`, as does a lost `HandBack`, which also
      sets `handback_due` again while the task is in `merge_queue`.
    - A lost `Proof`, `Check` or `PrepareReview` clears `gate_op`.
    - A lost `Accept` or `Discard` is logged.
  - After the replay, a deferred cancel whose candidate did not survive applies.
- **Resume messages.** `RESUME_WORKER` and `RESUME_REVIEWER` are sent only on the first
  resume after a restart that ended sessions (`Run.restored`). They go only to a working
  task's resumable worker with no fresh session pending, and to a reviewer that owes its
  verdict. A gated task's worker gets its next message later (ruling T13-I3). A reviewer
  with no session id is given up for a new round. A pause edit's resume sends none.
- **Stall clocks at resume.**
  - An `Interrupted` stage whose grace has passed goes to the stall ladder.
  - An `Interrupted` round that the restart ended becomes `Nudged`. Its `stall_nudge`
    is already queued and goes out with the resume.
  - `Watching` is re-armed from now.
- **Neither the downtime nor a pause counts as session time** (decision 40's minutes;
  superseded by the task clock in fix round 1):
  - A round the restart ended is charged up to its last sign of life.
  - A live round through a pause edit is charged up to its last event or the pause,
    whichever is later.
  - The mutation run found this. Before the fix, a 100-minute pause blocked the task at
    rung 4 on resume. Red: `left: (Blocked, 4)` against `right: (Working, 0)`, with
    `(0/150 tool calls, 100/60 minutes)`.
- **One reader per review task across a restart** (`schedule::holds_reader`). A
  reviewer round that has ended but is resumable, with no verdict yet, still holds its
  reader slot. Without this, the running pass prepared a second reviewer beside the one
  being resumed. Red: `one live reviewer: the resumed one`, with a `PrepareReview` in the
  resume's effects.
- **Retry** (decision 42):
  - Kills the old session and stops any reviewer.
  - Resets `failures = 1`, the bounces, `budget_exceeded` and `conflicts`.
  - Sets rung 2 on `roster::escalate`.
  - Replaces any pending fresh session (and its `append`) with its own.
  - Reuses the start commit and worktree (no `PrepareWorktree`).
  - `kill_worker`'s `supersede` ends the hand-back context (`handed_back`,
    `resolution`). The retry's own clear of these was redundant (mutant Q6) and was
    removed. The retry clears `gates_after_handback` itself.
  - **Deviation from carry T14-R2, confirmed by ruling T15-concern1, which amends
    the carry:** `resolving` and `handback_due` are kept.
    `resolving` describes the worktree, where a merge is still in progress, and the fresh
    session must finish it. `handback_due` is the queue's N5 obligation. Clearing either
    would merge a worktree with conflict markers or skip a hand-back.
  - A held task (`awaiting_deps`) keeps its own block. It is handed back first through
    `holds::resume_held`, whose condition now includes `held_answered &&
    fresh_session.is_some()`, and it never becomes `blocked(question)`.
- **Override of a blocked task with no accepted head** (carry M8a.13). A `CountCommits`
  from the start commit counts its branch, correlated by `Task.override_count.op`. At
  least one commit sends the counted tip to the merge queue; zero refuses. A fallback
  count can be in flight at the same time: rung 4 can block a task while its turn-end
  count is running. The correlation keeps the two apart.
- **A task blocked by the third failed count keeps its session** (followups item). The
  decision is recorded in the followups file.
- **The liveness check covers paused runs.** It clones the state, steps a `Resume` and
  checks the resumed run. Every new test ends with it. `a_restore_awaits_no_count`
  (`turns_ops.rs`) now resumes first, because a restore ends every session.

**TDD evidence.**
- Red run: 20 of 25 new tests failed. Sample lines:
  - `Err("run retry is not available yet")`;
  - `left: Running right: Paused` (pause edit);
  - `left: Complete right: Paused` (M12);
  - `left: MergeQueue right: Cancelled` (deferred cancel);
  - `left: [] right: [(2, "s-tw", RESUME_WORKER…), (5, "s-rw", RESUME_REVIEWER…)]`;
  - `one AbortMerge: []`.
- Five tests passed at red, pinning existing behaviour: `stop_ignores_everything_after`,
  `edit_applies_and_delivers`, `answer_resumes_a_question`,
  `restore_pauses_running_runs_only` and `a_hand_back_result_after_a_cancel_is_dropped`.
- The mutation-driven tests were each shown red on their mutant, and two of them on the
  real code: the pause budget and the second reviewer, above.

**Mutations** (scratchpad script from a WIP commit, since folded). 47 distinct mutants
in total: 40 in the first run, then S1, R22b, P1 and P2, then H2, G2 and R18 (retried
after their tests landed).
- First run: 23 of 40 killed. The survivors got tests, or an argued equivalence:
  - H2: `resume_held`'s retried clause, requiring `held_answered`. Killed by
    `a_refused_fresh_launch_that_gains_a_dependency_keeps_its_block`: the window limit
    leaves a fresh session on a blocked task.
  - R4: the outbox reset.
  - R6: the override count cleared.
  - R16: the Watching re-arm.
  - R17 and R19: `restored` gates the restart messages. R19 uses a Claude reviewer that
    ended between turns while waiting out a rate limit.
  - R18: `!fresh`. Needs a second live task, so the restart ended sessions.
  - R22 and R22b: relaunch's `wanted`, via a cancel edit while paused. `kill_sessions`
    retires only windowed rounds.
  - R23: the paused rebaseline clears `base_moved`.
  - R24: `owes_verdict` in `resume_reviewer`.
  - Q7: the run-state refusal.
  - G2: the override correlation.
- Final state: every mutant is killed except these four, argued equivalent:
  - **M9** (`cancel_now` keeps `merge_op`). The cancel edit never clears it for a
    hand-back either. A late result is dropped by the `merge_queue` state guard (M14,
    killed by `a_hand_back_result_after_a_cancel_is_dropped`), and restore's `lost()`
    sets `handback_due` only in `merge_queue`.
  - **M10** (the candidate's `awaits` filter). `op_done` drops any result whose op is
    not pending. A candidate's op and `merge_op` are cleared together (restore's
    `lost()`), and a cancel is deferred while the candidate is in flight.
  - **R7** (restore clears `resume_op`). The dropped op is not pending, so its result is
    dropped, and the next resume overwrites the field.
  - **R20** (a sessionless reviewer is set `retiring`). `holds_reader` needs a session
    id to hold the slot, so the new round starts either way, and nothing can reach the
    dead round.
- M8a.14's M12 and M14 are killed through the pause:
  `refs_verified_while_paused_do_not_complete_the_run` and
  `a_hand_back_result_after_a_cancel_is_dropped`.

**Carries in.**
- M8a.11: a retry of a started task reuses its start commit and worktree. Asserted: no
  `PrepareWorktree`, `DiffSoFar` from the start commit. A lost `PrepareWorktree` is
  re-emitted unchanged. N5 holds: a held task is handed back first.
- M8a.12: restore clears a claim and a `Counting` fallback
  (`restore_clears_a_claim_and_a_count_in_flight` kills R5).
- M8a.12 fix round 2: retry replaces a pending fresh session and its `append` (`no stale
  append`).
- M8a.12 fix round 5 (T12-RR4): `interrupted` is cleared by `end_round`, and
  `StallState::Interrupted` is settled at the resume.
- M8a.13:
  - Override of a blocked task without a head: counted (above).
  - Restore clears `gate_op`: `resume_resumes_sessions_and_reissues_gate_ops`.
  - Review m3 and m4 were already done in M8a.13.
- M8a.14:
  - T11-RR: retry of a held conflict-blocked task hands back first and is never a
    question (`a_retried_held_task_*`).
  - T11-RR2: retry and override both refuse `dep_cancelled`, so nothing lifts it past
    N5.
  - Restore re-issues a lost candidate or hand-back, and honours `cancel_deferred` and
    `handback_due` (`restore_honours_deferred_cancels_and_lost_hand_backs`).
  - A lost `AbortMerge` is re-emitted (`a_lost_abort_is_sent_again_at_the_restore`).
  - R9, R10, R12 and R14: see Mutations.
- M8a.14 fix round 2: retry clears the hand-back flags (the deviation above). The
  `resolution_only` executor is M8a.22's.
- Followups: the third failed count's session is decided.

**Carries out.**
- **M8a.21:** reconcile should report an `AbortMerge` whose worktree has no `MERGE_HEAD`
  as `MergeAborted`. The re-emitted abort must be idempotent.
- **M8a.22:** the driver builds `Restore`'s `replay` from the journal: the results of
  ops still pending in `run.json`. It sends `Restore` before any other event, and it
  fills `DoneChecked.resolution_only`.

#### M8a.15 fix round 1

The review (`task-15-review.md`) found 1 Critical, 3 Important and 4 Minor findings.
The rulings T15-C1, T15-I1, T15-I2, T15-I3, T15-concern1 and T15-minors are binding.
Each of the reviewer's probes became a regression test that failed first. The tests
are in `tests/control_fixes.rs` and `tests/control_clock.rs`; both split out for size.

**C-1 / T15-C1: a retry starts a new budget epoch.**
- `run retry` records `Task.epoch` (`clock::BudgetEpoch`): the fresh session's round
  index, and `spent_total`'s tool calls and tokens at the retry.
- Rung 4 weighs `clock::epoch_spend`: the worker rounds from that index, plus the tool
  calls and tokens since the retry.
- `spent_total` and the snapshot's total keep everything, for the report.
- Red: `a_retried_rung_4_task_gets_a_fresh_ceiling`, `left: (Blocked, 4)` against
  `right: (Working, 2)`. After the fix, the new epoch reaches rung 4 again at its own
  ceiling.

**I-2, I-3 / T15-I2, T15-I3: the task clock (new `engine/clock.rs`).** It replaces
M8a.15's separate shifts for the pause and the restore, and `Run.paused_at` is gone.
- **When the clock stops.** A task's clock stops (`Task.clock_stopped`) while the run is
  not `running`, or while the task is blocked (held included), in `proof`, `check` or
  `review`, or in `merge_queue`.
  - A session that is launching (`preparing`) keeps charging, because the launch is
    the worker's own. Without this the brief's pinned minute counts moved by the launch
    latency.
- **What the stop excuses.** When the clock restarts, each worker round is excused its
  overlap with the stop (`AgentRound.excused_secs`).
  - The latest round, if it ended but can still be resumed, is excused the whole stop,
    and its `ended_at` becomes now. Once resumed, it is charged from its start.
  - An open turn's `last_event` moves on by the stopped span, so silence before the stop
    still counts toward a stall and silence during it does not.
  - Decision 45's re-arm from `now` at a resume stays, and is stricter than the shift.
    `a_resume_re_arms_the_first_stall_stage_from_now` pins it (R16).
- **Restore.** `clock::stop_at_restore` stops a working task's clock at its session's
  last sign of life, so the downtime before the restore is excused as well.
- **Where it runs.** `clock::sync` runs at the start of every scheduler pass, so a task
  that works again is excused before the watchdog looks at it. It runs again at the
  end, so a stop made inside the pass (`enforce_holds` holding an answered task) is
  recorded at once.
- **Spend while stopped.** `ladder::round_spend` takes the open stop, so the snapshot's
  spend does not grow while the clock is stopped.
- **Red:** the halt (probe B), the late answer (Q), the retry after a long block (Q2)
  and `time_in_the_gates_is_not_charged` all showed `left: (Blocked, 4)`.
- **Scope.** A second-stage (`Interrupted`) deadline is not moved by a stop. Decision 45
  fires it at the resume, for a halt as for a pause.

**I-1 / T15-I1: a restore re-engages every working task.**
- `Run.restored` is set on every restore of a run that has had a session. Before, it was
  set only when a live session was ended.
- The resume then sends `RESUME_WORKER` to every working task's resumable worker, or
  lets `recover_sessionless` or the relaunch start one.
- Red: `left: []` against `right: [RESUME_WORKER]` (probe A); `left: 0 right: 1`
  (probe A3, the lost count timer). Liveness holds with no other live task.

**Minors (T15-minors).**
- **M-1: an overridden task's due hand-back goes first.** `merge::start_merge` starts no
  candidate for a queue head with `handback_due` set or a merge op in flight, so the
  task awaits one merge op at a time. The hand-back's clean result puts it back in the
  queue at the new head, and the candidate follows
  (`an_override_with_a_due_hand_back_hands_back_first`).
  - A hand-back into a worktree with a merge still in progress fails in the executor.
    The message tells the user to finish or abort that merge, and the task is
    `blocked(environment)`.
  - The first attempt also cleared `gates_after_handback` in `send_to_queue`. That line
    is unreachable: an override applies only in `review` or `blocked`, and every path
    that sets the flag leaves the task in `merge_queue` or takes the flag first. It
    was removed (mutant F13).
- **M-2: an override of a blocked task always counts its branch.** With an accepted
  claim, the claim merges only if it is still the branch's tip. Otherwise the reply is
  `task <id> has commits after its accepted claim; retry it to have them checked`
  (invented). A task in `review` merges its claimed head at once, as before.
  - Red: `left: 0 right: 1`; no `CountCommits`.
- **M-3: tests that kill V2, V5, V6, V15 and V16.**
  - V2: `an_override_count_that_comes_after_a_retry_is_refused`.
  - V5: `a_sub_agent_the_restart_killed_defers_nothing`.
  - V6: `a_restart_ends_a_deferred_fallback_s_wait`.
  - V15: `a_rate_limit_streak_ends_at_the_restart`.
  - V16: `a_claim_accepted_before_the_restart_leaves_no_mark_on_the_resumed_turn`,
    with the claim accepted mid-turn.
- **M-4:** `run cancel` clears `Run.restored`. `paused_at` no longer exists.

**Concern 1 (T15-concern1).** Retry keeps `resolving` and `handback_due`. This amends
T14-R2's carry (see M8a.15 above).

**Mutations** (scratchpad script from a WIP commit, since folded).
- 24 new mutants:
  - F1 to F19 over the clock, the epoch, the restore mark, the start-merge guard, the
    override tip check and the cancel;
  - the reviewer's V2, V5, V6, V15 and V16.
- All are killed except F13, which is unreachable and whose line was removed.
- Survivors of the first run got a test each:
  - F4 and F15 (the stall shift, and the sync before the watchdog):
    `a_block_answered_by_the_next_event_is_excused`, with the turn still open.
  - F5 (the restore's stop): `the_downtime_before_a_restore_is_not_charged`.
  - F16 (the closing sync): `a_hold_made_in_the_pass_stops_the_clock_at_once`.
  - F17 (the open stop in the spend): `a_stopped_clock_shows_no_growing_spend`.
  - F18 and F19 (the resumable-latest rule): `an_older_ended_session_keeps_its_end`
    (after a retry) and `an_ended_session_keeps_its_time_to_the_pause`.
  - V16: see M-3.
- M8a.15's R14 to R19 were re-run and are killed. R16 is killed by its new test.

#### M8a.15 fix round 2

Re-review 1 (`task-15-rereview-1.md`) confirmed C-1, I-1 to I-3 and the minors, and
accepted concerns 1 to 4. It found two clock bugs that let a worker run far past its
budget. Ruling T15-R2 is binding.

**The invariant.** No round is excused more time than it has lasted (`excused_secs <=
ended_at.unwrap_or(now) - started_at`), so no spend is negative and no excused time is
banked as credit.
- `clock::restart` caps `excused_secs` at the round's elapsed time, on top of its
  saturating arithmetic.
- The liveness helper checks the invariant (`liveness::assert_clock_sound`, called by
  `assert_alive`), so every engine test sequence checks it.

**N-1: a relaunch starts with no credit.** `restore::relaunch` resets `excused_secs` to
0 along with `started_at`. A launch lost at a restart gave its relaunched session the
restore-to-resume span as unmetered minutes.
- Red (probe P1c): `left: 7201` against `right: 0`.
- Test: `a_relaunched_session_gets_no_credit_from_its_lost_launch`. An active
  relaunched worker is now stopped by its 15-minute budget.

**N-2: each period is excused once.** `Task.clock_stopped` became `Task.clock:
TaskClock { stopped, restarted }`, which fits the model file's size limit.
`clock::stop_at_restore` stops the clock no earlier than the latest round's `ended_at`
or the clock's last restart (`restarted`). Before, a restore could reach back into a
stop already excused.
- Red (probe P2): `t1 round 0: excused 10916 of 7919 elapsed`.
- Tests: `a_restore_does_not_excuse_a_pause_twice`, and
  `a_restore_right_after_an_answer_stops_the_clock_at_the_answer`. The second
  covers a live round whose turn silence the answer shifted into the excused block;
  a resume re-arms the silence from `now`, so only an answer shows this case.

**Property-style test.** `no_sequence_excuses_more_than_elapsed` drives 300
pseudo-random sequences of 40 steps each and checks the invariant after every step. The
steps cover time, activity, turn starts and ends, exits, pauses, resumes, restarts,
blocks, answers, retries, and the launches, diffs, resumes and counts the steps wait on.
`the_probe_sequences_keep_the_clock_sound` replays the probes' own sequences with the
same check.
- Red: `t1 round 1: excused 3 of 0 elapsed` and `t1 round 0: excused 7201 of 1
  elapsed`.
- The tests are in `tests/control_clock_props.rs`.

**Mutations.** Five mutants:
- G1 (the relaunch reset): killed.
- G3 (`restarted` in the restore's stop): killed.
- G5 (`restarted` not recorded): killed.
- G2 (the round's `ended_at` in the restore's stop) survives, and is equivalent. An exit
  is a signal and moves `last_event`, and an `ended_at` set by a restart equals
  `restarted`.
- G4 (the cap) survives, and is unreachable while G2 and G3 hold. The combined
  G2+G3+G4 mutant is killed by `a_restore_does_not_excuse_a_pause_twice` and the
  property test.
- G2 and G4 stay as the ruling's explicit defences.

#### M8a.15 fix round 3

Re-review 2 (`task-15-rereview-2.md`) confirmed N-1 and N-2 and accepted G2, G4 and the
P1c note. It found N-3 (Important) and M-5 (Minor). Ruling T15-R3 refines T15-I3 and is
binding.

**N-3 / T15-R3: an open turn is charged and watched.** The task clock now also runs
while the latest worker round has a turn open, whatever the task's state (blocked,
held, in a gate) or the run's (paused, halted). This is `clock::open_turn`. Only spans
with no open turn are excused. Before, a worker could call `task_blocked` and then stream
for an hour uncharged and unwatched, and a turn open at a pause ran to its end for free.
- **`clock::watch_open_turns`.** It runs at the start of every scheduler pass and
  watches every open turn that `signals::watch` does not, meaning the task is not
  working or the run is not running.
  - A turn past its session's hard budget, or silent for `stall_after_secs`, is
    interrupted, once per turn (`AgentRound.interrupted`). The history line is `<why>
    while <state>; interrupting it` (invented).
  - The task keeps its state, and the ladder applies once it works again: the next
    `check_budget` sees the charged spend.
  - A working task's silence in a paused or halted run is the watchdog's first stage,
    as in `signals::watch`: `StallState::Interrupted`, with `stall_nudge` queued for the
    resume. Without the nudge the resumed worker had nothing pending, and the liveness
    check caught it.
  - A blocked or gated task gets no nudge, because its answer or its gate's result
    re-engages it.
- **Red:**
  - `a_turn_streaming_after_task_blocked_is_charged_and_breaches`: `charged 62`.
  - `a_silent_open_turn_of_a_blocked_task_is_interrupted`: `left: 0 right: 1`.
  - `a_pause_with_an_open_streaming_turn_is_charged`: failed.
- **Tests rewritten to the ruling.** These round-1 tests assumed an open turn is excused
  once its task stops:
  - `a_turn_open_through_the_gates_is_watched_then_excused`;
  - `a_block_is_excused_from_the_end_of_its_turn`;
  - `a_stopped_clock_shows_no_growing_spend` (the turn now ends first);
  - `a_resume_re_arms_the_first_stall_stage_from_now` (roomy budget, and a 30-minute
    pause under the next size's ceiling);
  - `a_paused_session_s_open_turn_is_watched_then_excused`;
  - `a_restore_right_after_a_resume_stops_the_clock_at_the_resume`, which replaces the
    answer variant: a nudged round between turns keeps its old last event through a
    resume, so G3 is still needed;
  - `an_older_ended_session_keeps_its_end` (the fresh session's first turn ends before
    the pause).
- **Removed as dead under the ruling.** Both were shown by their mutants (F4, F15)
  surviving every test:
  - The stall-silence shift at a clock restart. The stall watchdog watches only open
    turns, and an open turn keeps the clock running.
  - The `sync` at the start of the pass. The spend read anywhere subtracts the open stop
    (`round_spend`), so one `sync` at the end of the pass records every stop and
    restart.

**M-5: the charge covers the time worked.** `the_charge_covers_the_time_worked`
(`tests/control_open_turn.rs`) ports the reviewer's Q1 generator: 400 sequences of 60
pseudo-random steps.
- A work step's seconds count when the worker's session is live and either its task
  works in a running run or its turn is open. After every step the test asserts
  `total_spend >= worked`, plus `assert_clock_sound`.
- Red: `sequence 0: charged 4 < worked 728`.
- It also kills G1, G3 and F19.

**Observation (re-review 2, out of scope).** A restore may lower the shown spend by up
to one silence span. The stop sits at the session's last sign of life, which is the
accepted estimate from concern 3.

**Mutations.**
- The new guards are all killed:
  - H1: the open-turn term of `charging`.
  - H2: the `watch_open_turns` call.
  - H3: the budget branch.
  - H4: the stall branch.
  - H5: once per turn, killed by the added second tick.
  - H6: a working task's first stage, killed by liveness.
  - H7: skipping the tasks `signals::watch` covers.
- The earlier clock mutants were re-run and are killed: F1, F2, F5, F16 (the one
  remaining `sync`), F17, F18, F19, G1, G3 and R16.
- G2 is still equivalent, as re-review 2 accepted.

#### M8a.15 fix round 4

Re-review 3 (`task-15-rereview-3.md`) confirmed N-3 and M-5. It found N-4 (Important,
low likelihood). Ruling T15-R4 is binding.

**N-4 / T15-R4: the open-turn watchdog keeps T12-later.** `clock::watch_open_turns`
interrupted an open turn whose `task_done` check was in flight in a paused or halted
run. That aborted the pending tool call, spent the session's one nudge and left a stale
`stall_nudge` in the outbox. Its stall branch now skips a task with a claim in flight
(`task.claim.is_none()`), the same rule `signals::watch` applies to a running run. The
budget branch still fires during a claim, as `check_budget` does in `signals::watch`.
- **Red** (both at the no-interrupt assertion in `claim_outlasts_the_stall`):
  - `a_paused_run_s_stall_clock_waits_for_a_task_done_check`;
  - `a_halted_run_s_stall_clock_waits_for_a_task_done_check`. Its proof waits for the
    resume, so the test resumes with a rebaseline, checks that the `Proof` op starts,
    then runs the liveness assertion.
- **Mutation.** Removing the new condition fails both tests.

### M8a.16 the run report (2026-09-24)

**Layout.** `run/report.rs` holds `render`, `format_utc` (Howard Hinnant's
days-from-civil arithmetic, no new dependency), the header block, the tasks table and
the `## Log` section; `run/report_task.rs` (AGENTS.md's file-size rule) holds one
task's `## <id>: <title>` section, its budget-and-spend block (including per-round
usage and denials) and its findings-by-severity grouping. Both are pure: no `std::fs`,
`std::process`, `std::thread`, `tokio` or `std::time::SystemTime`; `render(run, now)`
takes the clock as a parameter, never reads it.

**Most of the brief's "reported" facts were already `Run.log` text.** Before writing a
single report-specific line, a grep of the engine confirmed that a kept branch
(`OpResult::Finished.kept_branches`, `complete.rs`), an aborted accept's files
(`accept_conflict_message`, also `complete.rs`), a halt (`merge.rs::halt`) and a
`--rebaseline` resume (`merge.rs::resume`) each already call `log(run, now, ..)` with
exactly the text the brief wants shown. `## Log` renders every `Run.log` entry as
`- <utc> <text>`, so `minor_findings_are_listed_even_when_approved`'s sibling tests
(`halted_and_rebaselined_runs_say_so`'s rebaseline half,
`base_moved_and_accept_conflict_are_reported`'s aborted-accept half) needed no
report-side special case — only a fixture that pushes the same `LogEntry` the engine
would have logged. The one line the brief pins to an *exact* wording the engine does
not already log is the base-moved header line (`base <base> moved during the run:
<from7>..<to7>, <n> commits, listed at accept`); `merge.rs::base_advanced` logs a
different sentence for the attention line, so `report.rs::header` builds this one
itself from `Run.base_moved`.

**Visibility bumps, not new logic.** `report_task.rs` needed two pieces of spend
arithmetic the engine already has: `engine::ladder::total_spend` (already
`pub(crate)`, no change) and `engine::clock::epoch_spend` (was `pub(super)`, visible
only inside `engine/`). Rather than duplicate the ladder's or the clock's math in a
"pure" file that is supposed to have none of its own, `epoch_spend` was raised to
`pub(crate)` and re-exported from `engine/mod.rs` as `pub(crate) use
clock::epoch_spend;` (`BudgetEpoch`/`TaskClock` were already re-exported the same way,
just `pub`). `contract::mode_label` and `contract::size_label` were similarly raised
from private to `pub(crate)` so the table and the per-task header reuse the one
existing size/mode vocabulary instead of a second copy of the match arms.

**Corrections against the brief's field names.**
- The brief's `report_has_every_section` lists "the history" once, after "merged
  without approval"; it also asks for "budget and spend" and "each round's turns, tool
  calls, billable tokens and denials" (from `usage_and_denials_are_reported`) as if
  they were adjacent concerns. The model has no `AgentRoundInfo`-shaped view inside
  `report.rs`'s reach — only `Task.rounds: Vec<AgentRound>` — so the per-round usage
  and denial line lives inside `budget_and_spend`, one bullet per round, rather than a
  separate subsection; nothing in the brief names a heading for it.
- "the task's total spend and its since-retry spend should both be clear if they
  differ" (this task's carry note) is `Task.spent_total` (via `ladder::total_spend`,
  which fills in `secs` from the live rounds) versus `clock::epoch_spend`, keyed off
  `Task.epoch: Option<BudgetEpoch>` (M8a.15's rung-4 "spend since the last retry", not
  named `since_retry` anywhere in the model). The report only prints the second line
  when `task.epoch.is_some()` **and** the two spends actually differ, so a task that
  has never been retried, or one whose retry landed at its very first round, gets one
  line, not two identical ones.
- `GateCounts` (`done`/`proof`/`check`/`review`/`merge`) has no existing rendering
  anywhere in the codebase to match (the TUI does not consume it yet). The `Tasks`
  table's `bounces` column is the sum of all five; the brief does not ask for the
  breakdown, and summing keeps the table narrow. (No per-task breakdown was added
  either, since nothing in the brief's ordered list asks for one.)
- `Severity` and `Verdict`'s serde renames (`lowercase`) were not reused via
  `serde_json::to_value` (the trick `run/snapshot.rs::attention` uses for
  `BlockReason`); `report_task.rs` writes its own small label matches instead, to keep
  `report_task.rs` free of a `serde_json` dependency for what is, in the end, three
  two-armed and one three-armed match statement.

**Tests.** 13 unit tests in `report_tests.rs`, built on `test_support::run_ok` and the
Interfaces example plan (`EXAMPLE_PLAN`), mutating the one resulting task by hand for
proofs, checks, reviews, rounds, salvage refs and history. Verified red-then-green by
hand: `render` was temporarily stubbed to return an empty string, which failed 10 of
the 13 tests for the expected reason (a missing substring, not a panic or a compile
error) — `format_utc_vectors`, `containment_is_silent_on_sandbox_when_on` and
`task_lookup_helper_finds_t1` correctly kept passing, since none of the three exercises
`render`'s body. The stub was then reverted and the suite re-run green.

**Gates.** `cargo build --workspace --all-targets`, `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo fmt --all --check` are clean. `cargo test -p
anthrex-daemon` is green except for a pre-existing, unrelated flake,
`git_registry::a_commit_in_a_linked_worktree_triggers_a_probe` (a filesystem-watcher
timing test): it fails intermittently under the full parallel suite and passes every
time run alone or with `--test-threads=1`. Nothing in this task touches git
registration, probing, or anything under `run/git/`.

### M8a.16 fix round 1 (2026-09-24)

The review (`task-16-review.md`) found the report had **no Markdown escaping
anywhere**, despite rendering text with no closed vocabulary: a plan's task title
(LLM-authored, C1), a check's raw tool output (C2), and a reviewer's summary and
findings plus the engine's own free-text notes and history lines (I1). Two gaps (M1,
M2) were also found in the test suite itself: `report_has_every_section` asserted a
section *label* ("Salvage refs:", "Route:") was present without ever asserting the
*value* it was supposed to carry, so a mutated or dropped value would have passed.

**Fix.** One new pure module, `run/report_escape.rs`, with the whole escaping layer:
- `escape_cell`: a table cell — flattens `\n`/`\r` to spaces, escapes `|` as `\|`. Used
  for the one field in the `## Tasks` table that is not from a closed vocabulary: the
  task title.
- `escape_heading`: a `##`-heading line — flattens newlines the same way, and escapes
  a genuine *leading* `#` (only the string's own first character, once newlines are
  gone) as `\#`. Used for the task title in `## <id>: <title>`.
- `continuation_indent`: a free-text line or list item's text — the first line is
  unchanged, every later line is indented 4 spaces. CommonMark never reads 4+ leading
  spaces as an ATX heading, and a line indented under a list item's marker (`"- "`,
  width 2) stays part of that item rather than becoming its own block — so one
  mechanism defuses both hazards the ruling named (an embedded newline starting a new
  block, and a leading `#` reading as a heading). Used for task notes, review
  findings' `text`, `merged without approval: <reason>`, and task history entries —
  every free-text field the review flagged (I1), plus a reviewer's round summary,
  which is the same kind of field (LLM-authored prose) even though the review did not
  name it explicitly.
- `fence_for`: the longest run of backticks in a check's tail, plus one, never fewer
  than 3 — exactly the brief's own stated fix for C2. `report_task.rs`'s check
  rendering now opens and closes with this fence instead of a fixed ` ``` `.

Not escaped, on purpose: `t.spec.id` (validated against `ID_PATTERN` in
`validate.rs`, so it cannot carry a `|`, a newline or a leading `#`), every enum
label (`size_label`, `mode_label`, `done_signal_label`, `verdict_label`, route
labels — a closed, hand-written vocabulary), every sha (`sha7`, hex-only), branch
and ref names (git's own naming rules already forbid the dangerous characters), and
`Run.log` text (already reviewed and accepted as-is in the original M8a.16 pass,
since it is the engine's own constructed sentences, not raw agent output — carried
forward here as a scope boundary, not re-litigated).

**Tests.** `report_escape.rs` gets 5 unit tests on the four pure functions in
isolation. `report_tests_escaping.rs` (declared from `report_tests.rs` the way
`plan_tests.rs` declares its own split-out siblings) adds:
- Four hostile-input tests, one per finding: a title with `|` and a newline (C1,
  table), a title starting with `#` and a newline (C1, heading), a check tail
  containing a ` ``` ` fence (C2), and notes/findings/history each carrying a newline
  followed by `# looks like a heading` (I1) — asserted with `assert_eq!(...count(),
  3, ...)` so a fix that escaped only one or two of the three fields would still fail.
  All four were run against the unescaped code first and failed for the stated
  reason (a missing escape, not a panic or a compile error) before the fix landed;
  two of my first drafts of these assertions were themselves wrong (a substring check
  that matched a valid 4-backtick fence's `` ``` `` prefix, and a bare "no `title
  across two lines`" check that didn't account for the same title appearing correctly
  in the task's own heading below the table) — caught by re-reading the failure output
  and fixed before considering the tests trustworthy.
- `salvage_ref_content_is_asserted_m1`: asserts the literal ref string, not just the
  `"Salvage refs:"` label.
- `route_and_review_route_content_is_asserted_m2`: asserts the exact `Route:`/`Review
  route:` line built from `t.route`/`t.review_route`'s actual runtime, model,
  strength and effort, not just the label.
- `round_usage_line_survives_alongside_escaping`: a guard that the escaping pass did
  not touch the per-round usage line, which carries only counters.

**Mutation-checked** by hand (mutate `report_escape.rs`, run `cargo test -p
anthrex-daemon --lib run::report`, confirm a failure, restore from the same file's
backup, confirm `git status` clean):
- `escape_cell` stops escaping `|` → caught (its own unit test and
  `title_with_pipe_and_newline_does_not_break_the_table`).
- `fence_for` hardcoded to always return `` ``` `` → caught (its own unit test and
  `check_tail_with_a_backtick_fence_uses_a_longer_fence`).
- `continuation_indent` made an identity function → caught (its own unit test and
  `notes_findings_and_history_with_newlines_and_hash_indent_their_continuation`).

**Gates.** `cargo build --workspace --all-targets`, `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo fmt --all --check` clean. `cargo test -p
anthrex-daemon --lib` is 912/912 green (899 before this fix round's 13 new tests,
plus the 5 in `report_escape.rs` and the 8 new/M1/M2 tests already counted in
`report_tests_escaping.rs`'s prior red run — final count 912, 0 failed). File sizes:
`report.rs` 211, `report_task.rs` 204, `report_escape.rs` 100, `report_tests.rs` 335,
`report_tests_escaping.rs` 175 lines — all comfortably under AGENTS.md's ~600-line
guidance.

### M8a.16 fix round 2 (2026-09-24)

Re-review 1 (`task-16-rereview-1.md`) confirmed C1, C2, M1 and M2 genuinely fixed, but
found I1 still exploitable for 3 of its 5 call sites, and three more fields never
routed through any escaping at all.

**Why I1 wasn't actually fixed.** `continuation_indent`'s 4-space indent is correct
for a *plain* line (`merged without approval`, a review summary — confirmed still
fine, unchanged). But Notes, review Findings and History all wrap it inside a `"- "`
list item. A list item only needs indentation to its *content column* (2, for `"- "`)
to keep a continuation line inside it at all; anything past that is what CommonMark's
block-start rules see, and they still recognise a heading, blockquote, list or fence
with up to 3 leading spaces. 4 total minus the 2 the list item itself consumes left 2
spare columns — not enough. Confirmed with a real parser
(`markdown-it-py` in the re-review; `pulldown-cmark` here), not by string-matching
whether a line happened to start with 4 spaces, which is exactly the check round 1's
own tests ran and which cannot tell "still inside a paragraph" apart from "still
inside a list item, but interruptible".

**Fix.** `report_escape.rs` gains two functions:
- `escape_block_start(line)`: escapes the character CommonMark (or its GFM table
  extension, once `Options::ENABLE_TABLES` is on — the report's own `## Tasks` table
  needs it) would read as a block start at the beginning of `line`, after up to 3
  leading spaces: `#`, `>`, `-`, `+`, `*`, `` ` ``, `~`, `=`, `|` get a backslash
  directly; an ordered marker (1-9 digits then `.` or `)`) gets its *delimiter*
  escaped instead, since CommonMark can only backslash-escape ASCII punctuation, not a
  digit (`1\.` is inert; `\1.` is not a recognised escape at all and renders as a
  literal backslash followed by `1.`, still an ordered marker to the parser). Block-
  start detection reads the source's raw characters before any inline processing, so a
  literal `\` immediately defeats every one of these checks regardless of what follows
  it — the same mechanism C1/C2 already relied on for headings and fences, generalised
  to every hazard T16-R2 names.
- `list_item_text(prefix, text)`: replaces the three list-item call sites'
  `continuation_indent`. `prefix` (already single-line — the caller's job) goes first
  on the physical line; every line of `text`, the first included, is escaped with
  `escape_block_start`; every line after the first is indented 2 spaces (the item's
  content column) so it stays part of the item.

Wired in: Notes and History (`list_item_text("", ..)` /
`list_item_text("<utc> ", ..)`), review Findings (`list_item_text(&where_, &f.text)`,
`where_` now built from `escape_cell(file)` rather than the file name raw — N3), and
`Run.log` (`list_item_text("<utc> ", &entry.text)` — the ruling's "include the log";
the original review's "engine's own sentences" scope boundary still holds for what the
*engine* logs, but the ruling now treats every list-item field the same way whatever
its provenance, which is the more defensible default). `Run.goal` (N1) goes through
the existing `continuation_indent` — it is a plain line (`Goal: <goal>`), not a list
item, so round 1's mechanism was always sufficient for it; it simply was never called.
`ProofRecord.test` (N2) goes through `escape_cell` — it sits *inline*, mid-format-
string (`"Proof N: test={test} red=..."`), not as its own line, so it needs the
table-cell treatment (flatten to one line) rather than a line-oriented one.

**A second, unrelated defect found while wiring this up.** Every list in the per-task
section (Notes, each review round's findings, the round-usage bullets in
`budget_and_spend`, History) was directly followed by a plain paragraph line
(`Route:`, the next review round's own `Review round N (...):` line, `Proof N: ...`)
with **no blank line** between them. CommonMark's lazy-continuation rule folds a
plain line straight into the preceding list item's last paragraph when nothing
separates them and the line does not itself look like a new block — so even entirely
benign multi-line report content merged the report's own trusted metadata lines into
the wrong list item, and confusingly attributed a genuinely new list item that
followed later (recognisable as such, "-" can interrupt a paragraph) to the *same*
list rather than starting a new one. Not a T16-R2 finding and not itself an injection
(nothing forged — see `notes_findings_and_history_do_not_forge_real_markdown_structure`
below, which caught it), but real breakage the round's own CommonMark-parser
verification would otherwise have papered over by asserting the wrong invariant
("exactly 3 more `List` starts" instead of "exactly 3 more list items, and nothing
forged"). Fixed by pushing a blank line after each of these four list blocks.

**Tests.** `report_escape.rs` gets 3 more unit tests: `escape_block_start` against
every named marker plus the two boundary cases (up to 3 leading spaces still count; a
mid-line marker does not), and two for `list_item_text`. `report_tests_escaping.rs`
adds a `pulldown-cmark` (new dev-only dependency, workspace-wide in
`[workspace.dependencies]`, `pulldown-cmark = "0.13"`; nothing already in the
workspace parses Markdown) verification: `node_counts` parses rendered output with
`Options::ENABLE_TABLES` and counts `Heading`/`List`/`Table`/`CodeBlock` start events.
`notes_findings_and_history_do_not_forge_real_markdown_structure` poisons Notes, one
Finding (`file` and `text`) and History with one line of every named hazard, and
asserts heading/table/fence counts are unchanged from a clean baseline and exactly 3
new list items appear (not, e.g., a nested list from an unescaped `-` inside an
item). `run_goal_with_hostile_lines_forges_nothing_n1` and
`log_entries_do_not_forge_markdown_structure` do the same for `Run.goal` and
`Run.log`. `proof_test_field_with_a_newline_does_not_split_its_line_n2` and
`finding_file_with_a_leading_hash_does_not_forge_a_heading_n3` are direct
(non-parser) checks, since both are about a single field staying on one line rather
than about block structure. All five were run against the round-1 code first and
failed for the stated reason (a genuine list-merge/heading/table count mismatch, not a
panic) before the fix landed — including reproducing re-review 1's exact I1 finding
(1 `List` start instead of the expected 3, from the still-open lazy continuation) and,
once `escape_block_start` existed but before `|` was added to its marker set, a
forged-table false negative the delimiter-row poison line caught on its own.

**Mutation-checked** by hand (mutate, run `cargo test -p anthrex-daemon --lib
run::report`, confirm failure, restore, confirm `git status` clean): `list_item_text`
stripped of its `escape_block_start` calls (leaves only the indent) → caught by 5
tests, including its own two unit tests and all three list-item hostile-input tests.
Removing `|` from `escape_block_start`'s marker set → caught by the log and
notes/findings/history tests (both poison a real GFM delimiter row). Removing the
ordered-marker branch entirely → caught by `escape_block_start`'s own unit test and
both list-item hostile-input tests (the `1.`/`42)` lines in `HOSTILE_LINES` turn into
real ordered lists once unescaped, inflating the list count).

**Gates.** `cargo build --workspace --all-targets`, `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo fmt --all --check` clean. `cargo test -p
anthrex-daemon --lib` is 919/919 green. `cargo test -p anthrex-daemon` (full,
including integration tests) reproduces only the same pre-existing, unrelated
`git_registry::a_commit_in_a_linked_worktree_triggers_a_probe` flake noted in the
original M8a.16 notes and fix round 1 (green alone or with `--test-threads=1`;
nothing in this round touches `run/git/`). File sizes: `report.rs` 214,
`report_task.rs` 215, `report_escape.rs` 203, `report_tests.rs` 335,
`report_tests_escaping.rs` 348 lines.

### M8a.16 fix round 3 (2026-09-24)

Re-review 2 (`task-16-rereview-2.md`) confirmed I1 fixed for every marker T16-R2
named, and N1/N2/N3 fixed, but — asked to hunt for constructions beyond the named
list — found three more: NB1, a review round summary's bare `===`/`---` first line
setext-promoting the trusted `Review round N (...):` line above it (`summary` was
run through `continuation_indent`, which only touches *continuation* lines — the
first line, with no static prefix in front of it the way `Goal: `/`merged without
approval: ` have, was never escaped at all); NB2, a leading tab bypassing
`escape_block_start`'s column-counting loop (it only advanced past literal space
characters, so a tab looked like "the 3-space budget is used up" at position 0 and
the loop exited before ever checking the character against the marker set); NB3, `<`
simply absent from the marker set, letting a note/finding/history/log line open a
real HTML block.

**Ruling T16-R3: stop listing constructs one at a time.** `escape_block_start`'s
approach — name a marker, add a character to the set — is exactly what produced NB1-3:
every fix so far handled the constructs re-review found, not the general case. The
ruling replaces it with `normalize_line`, one rule instead of a list: CommonMark lets
*any* ASCII punctuation character be backslash-escaped, and every block-start marker
CommonMark defines (heading, blockquote, bullet list, thematic break, fence, setext
underline, HTML block, table row) is itself punctuation-led — the ordered-list marker
is the only digit-led exception, kept as its own small rule since a digit cannot be
escaped, only its delimiter can (`1\.` is inert; `\1.` is not a recognised escape at
all).

`normalize_line(line)`: expand tabs to spaces (closes NB2 — indentation is now
counted, then discarded, not miscounted), strip all leading whitespace (a normalized
line's indentation always comes from its caller afterwards, so nothing of the
original survives to be re-counted wrong), then backslash-escape the first remaining
character if `char::is_ascii_punctuation()` (closes NB3 for free — `<` is
punctuation, so it needs no special case, unlike round 2's hand-maintained
`"#>-+*=~\`|"` set) or, for an ordered marker (1-9 digits then `.`/`)`), its
delimiter. A line empty after trimming stays empty, per the ruling.

`list_item_text` now calls `normalize_line` instead of the retired
`escape_block_start` (unchanged shape otherwise: first line included, continuation
lines indented 2). `continuation_indent` is retired too, in favour of
`plain_text_line`, which is the same "normalize every line, first included, indent
continuation 4 spaces" idea applied to a *plain* line rather than a list item's — this
is what closes NB1, since it's what `Run.goal`'s and `merged without approval`'s
first lines already got for free from their static prefixes, and what a review
summary's bare first line never got at all. `Goal: `/`merged without approval: `
still work exactly as before (their first line was already safe; normalizing it is a
harmless no-op, since it never starts with punctuation there — the prefix does).

**Tests.** `report_escape.rs`: `normalize_line`'s own unit test now enumerates every
NB1-3 marker (`<`, tab, mixed space+tab) alongside T16-R2's original set, plus a
"stays empty" case for a whitespace-only line; `plain_text_line` gets a dedicated
NB1 case (`plain_text_line("===\nrest")` → escaped, not just its continuation).
`report_tests_escaping.rs` adds direct `render()`-level reproductions of NB1
(a `ReviewRecord.summary` starting with `===`), NB2 (a note whose continuation starts
with a tab), and NB3 (a note opening `<div>…</div>`, with the parser's `Html`/
`InlineHtml` event count as the check, not a string search), then the ruling's
requested fuzz: `every_ascii_punctuation_character_at_line_start_forges_nothing`
iterates the full 32-character ASCII punctuation set × 3 leading-whitespace variants
(none, one space, one tab) = 96 renders, each planting the same hostile line as a
continuation of `Run.goal`, a note, a finding's `text` *and* `file`, a review
summary's own *first* line (NB1's exact shape), history, `Run.log`, `ProofRecord.test`
and inside a check's tail (asserted to stay fenced — the tail itself is never
normalized, only fenced, per the ruling). Every one of the four new/updated tests was
run against round-2 code first and failed for the stated reason — the fuzz test in
particular failed on its very first punctuation/whitespace combination it tried
(`#`/no leading whitespace), well before iterating the rest, confirming it is not a
vacuous check.

**Mutation-checked** by hand (mutate, run `cargo test -p anthrex-daemon --lib
run::report`, confirm failure, restore, confirm `git status` clean): removing tab
expansion (reverting to "only spaces count") → caught by 4 tests, including the fuzz
test and both NB2 reproductions. Excluding `<` specifically from the punctuation
check → caught by 3 tests, including the direct NB3 test (the fuzz test's generic
per-character sweep did not happen to catch this one on its own, since its assertion
set does not special-case `<`'s `Html` count the way the dedicated NB3 test does — a
useful reminder that a broad fuzz and a few sharp, hazard-specific assertions are
complementary, not substitutes for each other). Removing the ordered-marker branch
→ caught by 4 tests, including `normalize_line`'s own unit test and both list-item
hostile-input tests carried from round 2.

**Gates.** `cargo build --workspace --all-targets`, `cargo clippy --workspace
--all-targets -- -D warnings` and `cargo fmt --all --check` clean. `cargo test -p
anthrex-daemon --lib` is 926/926 green. `cargo test -p anthrex-daemon` (full)
reproduces only the same pre-existing, unrelated `git_registry` flake noted in every
prior round (green alone or with `--test-threads=1`). File sizes: `report.rs` 214,
`report_task.rs` 215, `report_escape.rs` 243, `report_tests.rs` 335,
`report_tests_escaping.rs` 510 lines — `report_tests_escaping.rs` is now the largest
file this task touches but still comfortably under AGENTS.md's ~600-line guidance.

### M8a.16 fix round 4 (2026-09-24)

Re-review 3 (`task-16-rereview-3.md`) confirmed NB1-NB3 fixed and found NB4: a bare
`\r` (no `\n`) is a line ending to CommonMark but not to `str::lines()`, so text after
it reached the parser as a column-0 line that `normalize_line` never saw and no
caller indented (`"note\r# forged"` forged a real `<h1>`). **Ruling T16-R4:** a new
`normalize_line_endings` in `report_escape.rs` rewrites `\r\n` and a lone `\r` to `\n`
before any line handling. Every untrusted multi-line field passes through it:
`plain_text_line` and `list_item_text` call it first, and the check tail now goes
through a new `fenced(tail)` helper (which replaces the inline fence code in
`report_task.rs`) that calls it before `fence_for`. The single-line fields
(`escape_cell`, `escape_heading`) already flattened `\r`. The check-tail path was not
exploitable at render level, because `fence_for` counts backtick runs across the whole
tail whatever its line endings are. It is normalized anyway, so every field uses one
definition of a line. **Accepted risk:** mid-line inline HTML and autolinks (for
example `<span>` or `<http://…>` inside a line, or `<div>` in a field flattened onto
one line such as `ProofRecord.test` or `Finding.file`) still parse as inline nodes.
Only line-start (block) constructs are neutralised, and that is enough for this
milestone's terminal-facing report.

**Tests.** The NB4 regression, `a_bare_carriage_return_forges_nothing_in_any_field_nb4`,
was written first and failed against `965eedf` with `Run.goal forged Markdown
structure (bare \r)` (`report_tests_fields.rs:151`, 5 headings against a 4-heading
baseline). It plants every block-start hazard separated only by bare `\r`, then by
`\r\n`, then by a mix of `\r` and `\n`, into each of the nine untrusted fields one at a
time. The punctuation fuzz moved with it into the new `report_tests_fields.rs`
(`report_tests_escaping.rs` goes from 510 to 421 lines). It now asserts per field:
one render per field × character × leading whitespace × shape (first line, and a
continuation after `\n`, `\r\n` and `\r`), so a failure names the field, the character,
the whitespace and the shape. `NodeCounts` now counts block `html` and `inline_html`
separately. Block HTML is asserted for every field. Inline HTML is exempt only for the
two one-line fields, per the accepted risk above. A new unit test covers
`normalize_line_endings`, `list_item_text`, `plain_text_line` and `fenced` with a bare
`\r`.

**Mutation-checked** by hand. Making `normalize_line_endings` a no-op was caught by 3
tests (the NB4 regression, the fuzz and the unit test). Dropping the normalization
from `fenced` alone was caught by the unit test only, which fits the render-level
finding above. Both mutations were restored from a backup.

**Gates.** build, clippy `-D warnings` and fmt `--check` are clean. `cargo test -p
anthrex-daemon` is green in full: 928 lib tests and every integration binary.

### M8a.17 headless sessions: the process driver and headless windows (2026-09-24)

Built `headless/session.rs` (the driver), `manager/headless.rs` (headless windows),
`server/headless_guard.rs` (decision 49's refusals), and the TUI side (`App::is_headless`,
`focused_pty`, `focus_headless`, `ui::terminal::headless_placeholder`). Tests:
`tests/headless_sessions.rs` (9), `tests/headless_env.rs` (1, alone),
`tests/headless_windows.rs` plus `headless_windows/{hooks,socket}.rs` (11, split by
seam for rule 8), `tui/src/app_tests/headless.rs` (6), and `tests/support/headless.rs`
(shared spec, `/bin/sh` stand-ins, manager). No protocol change: `WindowKind`, `RunRef`
and the version bump to 7 landed in M8a.2 with their round-trip tests.

**Name corrections and interface changes (each needed by a test or the acceptance):**

- **`HeadlessHandle::spawn(runtime, program, args, cwd, env, on_event)`**: `runtime`
  is added first (it chooses the parser, one `ClaudeStream` per process), and
  `on_event` is `Fn(u32, SessionEvent)`: the pid of the process each event came from.
- **`SessionEvent::ProcessStarted { pid }`** is a new driver event, always a process's
  first. `status::next` treats it as bookkeeping (no status change, no end of a retry);
  `conversation::map` ignores it.
- **`WindowSignal { window_id, pid: Option<u32>, kind }`**: `pid` is new (`None` for a
  hook), and `apply_session_event(id, pid, event)` takes it.
- **`Entry.headless` is not a field.** A field on `Entry` would have to be added to
  `create.rs`'s struct literal, which the acceptance forbids. The spec, the stream
  status, the window's one `StreamCursor` and its diagnostic ring live in
  `Process::Headless(Box<HeadlessWindow>)`, and `Entry::headless()` reads the spec.
- **`Entry::attach` returns `anyhow::Result`**, refusing a headless window with decision
  49's subscribe text; only `WindowManager::attach` calls it.
- **The server guard is one call before dispatch**, `headless_guard::refuse(&manager,
  &msg)`, not one per match arm: `if let` match guards are not stable Rust. It covers
  `Subscribe`, `Input`, `Kill`, both `Remove` forms and `Restart`; `Resize` reaches the
  manager, which accepts and ignores it for a headless window.
- **`std::process::Command`, not `tokio::process`** (decision 26 named tokio's). The
  driver is threads end to end, `spawn` is called on `spawn_blocking`, and the waiter
  reaps with `waitpid` itself, which a tokio `Child` would race.
- New public items: `session::{STDERR_LINE_MAX, OUTPUT_GRACE (re-exported from
  run::exec)}`, `HeadlessHandle::{spawned_pid, is_ended}`,
  `manager::{DIAGNOSTIC_LINES, SIGNAL_CHANNEL_CAPACITY, subscribe_refusal,
  control_refusal}`, `WindowManager::{headless_spec, headless_run}`.

**The driver (`headless/session.rs`).**

- Five threads per process: the stdin writer (a `sync_channel(WRITER_QUEUE_MAX)`, so
  `send_line` is a `try_send` and fails when full or closed; a line with `\n` is
  refused), a stdout and a stderr reader, a waiter, and one dispatcher that owns the
  parser and calls `on_event`. Everything a process says reaches the caller from that
  one thread, in one order.
- **The ordering contract (ruling T13-P1, driver side)** is in the module doc and on
  `run::engine::AgentSignal`, where the engine consumes it: every event carries its
  process's pid; `ProcessStarted` comes first; `ProcessExited` comes last, after every
  stdout and stderr line, or after `OUTPUT_GRACE` (1 s, `run::exec`'s rule) when an
  escaped process still holds a pipe (later lines are dropped). Tested by
  `spawn_reads_events_in_order_and_reports_exit` and, through the manager's feed,
  `every_session_event_reaches_the_feed_with_its_pid_in_order`.
- **The reducer side of T13-P1** (record the pid whose turn closed, so a Codex exit after
  its turn completed is never a death) is **not** done here: carried to M8a.22.
- **The waiter** blocks in `waitid(WEXITED | WNOWAIT)`, then, holding the process's
  `reaped` lock, sends `SIGKILL` to what is left of the group while the leader is still
  an unreaped zombie, reaps it, and marks it reaped. Every signal takes the same lock
  and is skipped once reaped, so no signal can reach a reused pid or group. `ECHILD`
  skips both the kill and the reap, as in `run::exec`.
- **`kill(grace)`** closes stdin, sends `SIGTERM` to the group, and returns; a thread
  waits on a condvar for the reap and sends `SIGKILL` to the group at `grace`.
  `kill_sends_sigterm_to_the_whole_group_first` was added because the waiter's cleanup
  kill made the brief's `kill_terminates_the_process_group` pass even with the
  `SIGTERM` sent to the leader alone (a surviving mutant).
- **`interrupt(Sigint)` signals the leader only**, not the group: the group also holds
  the session's `anthrex mcp` server, which must outlive a turn.
- **Lines.** stdout lines keep at most `STDOUT_LINE_MAX` bytes; a longer one is read to
  its end and reported as `Unknown` with its first 300 characters. stderr lines are cut
  to `STDERR_LINE_MAX` (4096 bytes, invented). Blank lines are skipped and one trailing
  `\r` is dropped. `Diagnostic` is logged at `warn` by the dispatcher.
- **The environment (decision 26).** Removed: every inherited `CLAUDE_CODE_*`,
  `CLAUDECODE`, `ANTHREX_WINDOW_ID`, `ANTHREX_SOCKET`, and AGENTS.md rule 11's five git
  variables (invented for sessions: an inherited `GIT_DIR` would point an agent's own
  git at another repository). Then the profile's env, then the window's own
  `ANTHREX_WINDOW_ID` and `ANTHREX_SOCKET` last, so a profile cannot redirect a
  session's hooks to another window. `the_environment_is_scrubbed` goes through
  `create_headless`, so the window id it checks is the real one.
- **M8a.12's carry (the sandbox before `Init`) is done here, in the driver.** A stderr
  line containing `sandbox required but unavailable` before any `Init` becomes
  `TurnEnded { Failed { error: <the trimmed stderr line>, kind: SandboxUnavailable },
  usage: None, denials: [] }`, delivered after every line and before `ProcessExited`.
  The same text after `Init` is only a `StderrLine`. Tests:
  `a_sandbox_failure_before_init_is_a_failed_turn_before_the_exit`,
  `a_sandbox_text_after_init_is_only_a_stderr_line`.

**Headless windows (`manager/headless.rs`).**

- **`create_headless`** validates the name, waits for the launch gate outside the lock,
  then under the lock (no I/O) spends an id, lists the window as `Starting` with an
  ended handle, and records the first turn in its cursor (`sent_turn`, before anything
  is written: M8a.7's binding). It then builds the argv and env, spawns on
  `spawn_blocking`, writes Claude's first user message through the writer queue, and
  installs the handle under the lock. `child_alive` is read from the handle (the waiter
  marks the reap before the exit event is delivered, so the order of the two cannot
  leave it wrong). A spawn that fails unlists the window; a shutdown in between kills
  the new process after the lock is dropped. No lock guard is alive across an
  `.await`, the spawn, the write or a kill.
- **The window is listed before the spawn** so that no event of the new process can
  arrive for a window that does not exist yet. The events' closure holds a `Weak` of the
  manager.
- **`apply_session_event`** (under the lock) keeps the diagnostic ring
  (`DIAGNOSTIC_LINES` = 10: `Unknown`, `StderrLine`, `Diagnostic`), applies
  `status::next`, copies status and tool to the entry, sets the session id from `Init`,
  maps the event to conversation inputs (hooks through `conversation_hook`, records
  through `enrich`, then notify), and sends the feed signal. It publishes the list only
  when a listed field changed.
- **Status follows the window's current process only** (invented): an event whose pid
  is not the installed handle's still reaches the feed and the conversation, but moves
  neither status nor `child_alive`. This is what M8a.18's kill-then-`--resume` needs.
- **A Codex process that exits after a turn ended in it keeps the window `Idle` or
  `Attention`**, not `Exited` (invented; `status::next` alone would say `Exited`;
  narrowed by fix round 1, ruling T17-I1: an exit before any turn ended in that
  process is `Exited` with its exit info). Codex
  runs one process per turn, so that exit is not the session's end. `exit` is set only
  when the status becomes `Exited`. Test: `session_events_drive_status_and_the_conversation`.
- **Real hooks** (`handle_hook`'s headless branch, `headless_hook`): `AgentState::on_hook`
  still runs (sub-agent rows, the session id) but its status event is dropped; the hook
  goes to `conversation_hook`; for Claude it then goes to `observe_hook` with the
  window's cursor and the records are enriched (ruling T7-N1); `SubagentStart` and
  `SubagentStop` go on the feed as `WindowSignalKind::Hook` with `pid: None`.
- **`tick`** skips headless windows; **`subscribe_conversation`** starts no reader and
  sets no `NoTranscriptPath` for them.
- **Persistence.** `Entry.run` holds the spec's JSON from creation, and
  `state_snapshot` writes `kind: headless` beside it. `WindowRecord.kind` is appended
  after `run` (`#[serde(default)]`, so no state version bump; `record_json_shape` and
  `the_two_worktree_fields_are_independent_on_the_wire` now end with `"kind":"pty"`).
  `restore` rebuilds a headless record as `Process::Headless` with an ended handle and
  status `Exited`; a `run` that does not parse as a `HeadlessSpec` loads as an exited
  PTY window with a `warn`, its `run` carried verbatim.
- **What M8a.18 fills in.** `headless_send` has its Claude path now, because
  `a_headless_claude_prompt_hook_feeds_the_cursor` needs a second engine turn:
  `sent_turn` under the lock, then `send_line(user_message(text, session_id))` after
  it. It does not clamp the text yet, and a Codex send is refused with `a Codex turn is
  a new exec resume process (M8a.18)`. `headless_resume` returns an error naming
  M8a.18. `headless_interrupt` (Claude by `CLI_CAPS.claude_interrupt`, Codex by
  `SIGINT`, request ids from a daemon-wide counter) and `headless_kill`
  (`config.kill_grace`) are complete but thin.

**The TUI.** `App::focus` on a headless window sets focus, resets the pane, and sends
`Unsubscribe` when a PTY subscription was active (so the old window stops streaming to
this client), never `Subscribe`. `retry_dropped_subscribe`, `on_reconnected` and the
debounced `Resize` read `focused_pty()`, as do both key paths, the paste path and the
mouse wheel's report. The pane draws `  headless session · <runtime> · <status> ·
<prefix> m shows its conversation` (the prefix from settings, so `C-b` by default).

**Carries.**

- **M8a.22 (T13-P1, reducer side):** record the pid whose turn closed; a later exit of
  that pid is normal, never a death.
- **M8a.22 (translation):** `WindowSignal` to `AgentSignal`: `ProcessStarted` and
  `ProcessExited` take their pid from `WindowSignal.pid`; `TurnEnded.denials` passes
  through as the tool names; `Hook { SubagentStart | SubagentStop }` map one to one.
  Decide whether `StderrLine` and `Unknown` count as `Activity` (they reset the stall
  clock; Claude writes hook-progress lines constantly).
- **M8a.22 (git registry):** `create_headless` sets `Entry.worktree` but no registry is
  reachable from the manager, so nothing registers a headless window's root, while
  `server::register_restored_roots` registers a restored one's, and an engine-side
  `remove` would not unregister it. The driver should own both halves.
- **M8a.18:** clamp the text in `headless_send`; Codex send and resume; the interrupt
  tests. `headless/session.rs` (583 lines) and `manager/headless.rs` (558) are near
  rule 8's limit and will need splitting when they grow. `server.rs` went from 618 to
  627 lines (the guard call and its module).

**TDD evidence.**

- `headless_sessions.rs` was written against a `todo!()` stub of `session.rs`: all 8
  failed with `not yet implemented` (`session.rs:27`). The ninth,
  `kill_sends_sigterm_to_the_whole_group_first`, was added after a mutant survived, and
  failed against that mutant (`the group's SIGTERM reached <pid>`).
- The manager and socket tests were written before `manager/headless.rs` existed and
  failed to compile. Once built, `client_control_of_a_headless_window_is_refused`
  failed before the guard existed, with `left: Error { request: "input", message:
  "window 1 is a headless session; only the engine drives it" }` (the manager's own
  refusal, not decision 49's text). Every other behaviour was then reverted one at a
  time, and a named test failed each time:
  - the `handle_hook` branch removed: `left: Working right: Idle`
    (`real_hooks…`), plus `left: Some(Misaligned)` (`…feeds_the_cursor`);
  - `observe_hook` not called: `left: Some(Misaligned) right: None`;
  - the `SubagentStart` feed send removed: `real_hooks…` times out waiting for it;
  - the `tick` skip removed: `left: Idle right: Working`;
  - the transcript skip removed: `a_headless_window_starts_no_transcript_reader`;
  - the Codex exit rule removed: `left: Exited right: Idle`;
  - the restore branch removed: `left: Pty right: Headless`;
  - the launch-gate wait removed: `create_headless_waits_for_the_launch_gate`;
  - the env scrub or the window id removed: `the_environment_is_scrubbed`;
  - the sandbox synthesis removed: the event list lacks the `TurnEnded`.
- TUI: 5 of the 6 tests failed before the change (for example `[Send(Subscribe {
  window_id: 2, … })]` and `Char('x'): [Send(Input { window_id: 2, bytes: [120] })]`).
  `prefix_commands_still_work_on_a_headless_window` passed before and after. It guards
  behaviour that already existed, which the change must keep.

**Timing.** Every wait is a deadline loop or a feed wait of 10 s (a hang guard, far
above milliseconds of real cost). The brief's `kill(1 s)` then "gone within 2 s" leaves
1 s over the grace; the `SIGTERM` test waits 2 s for a `SIGTERM` that is delivered at
once. There are two sleep-then-asserts, both on purpose: the launch-gate absence (200 ms,
as the brief says) and `QUIET_AFTER + 300 ms` before `tick`, as in `manager.rs`'s
existing quiet test.

### M8a.17 fix round 1 (2026-09-24)

Review `task-17-review.md`: 1 Critical, 1 Important and minor findings. The rulings are
T17-C1, T17-I1 and T17-minors in `progress.md`. Each behaviour fix has a test that
failed first. Each fix was then mutation-checked: reverting it fails its test.

- **C1: decision 26 is wrong for Codex. Codex's stdin must not stay piped open.**
  - Evidence (the reviewer ran the real binary, codex-cli 0.156.1, with an isolated
    `CODEX_HOME`):
    - `codex exec --json … "say hi"` with a piped stdin left open printed nothing for
      8 s.
    - Once the pipe closed, it printed `Reading additional input from stdin...`, then
      `thread.started` and `turn.started`.
    - With `</dev/null` it starts at once.
    - Its `--help` says a piped stdin is appended to the prompt as a `<stdin>` block,
      so it reads to EOF first.
  - As built, every Codex worker and reviewer would have hung until the stall timer.
  - **Correction to decision 26's "stdin, stdout and stderr all piped":** the driver
    (`HeadlessHandle::spawn`) closes a Codex process's stdin at once. No sender is
    kept, so the writer thread drops the pipe and Codex sees EOF. `send_line` on a
    Codex handle always fails with `stdin is closed`. Claude keeps its stdin open for
    stream-json input.
  - The rule lives in the driver, so M8a.18's `exec resume` path gets it through the
    same `spawn`. M8a.18 must not reopen stdin for Codex.
  - Test: `a_codex_session_that_reads_stdin_to_eof_still_runs_its_turn`. Its stand-in
    runs `cat >/dev/null` before printing anything. Before the fix it timed out waiting
    for the exit.
  - **Carry for M8a.20:** fake-agent's Codex headless mode reads stdin to EOF before it
    prints anything, as the real CLI does, so an end-to-end run shows the same hang if
    stdin is ever left open.
- **I1: a Codex exit is a death unless a turn ended in that process.**
  - `HeadlessWindow.turn_ended_in_process` is reset by `ProcessStarted` and set by
    `TurnEnded`, both for the current process only.
  - A process that dies before its turn (a bad flag, an auth error) now ends the window
    as `Exited`, with `exit` set (`exited with code 2`).
  - Test: `a_codex_process_that_dies_before_its_turn_ends_the_window`, the reviewer's
    probe (one stderr line, then `exit 2`). Before the fix: `left: Starting right:
    Exited`.
- **Minors.**
  - Tests now kill the two surviving mutants:
    - `an_event_of_another_process_does_not_move_the_window`: with `current = true`,
      `left: Exited right: Starting`.
    - `a_leader_that_exits_takes_its_background_children_with_it`: without the waiter's
      post-exit group `SIGKILL`, the background `sleep` outlives its leader.
  - `send_line` checks the reap before it queues, so a send after the exit fails at
    once. Test: `send_line_fails_once_the_process_has_exited`. It returned `Ok` before
    the fix.
  - `kill_sends_sigterm_to_the_whole_group_first` and the new background test hold a
    `KillGroupOnDrop` guard, so a failing assertion no longer leaves the TERM-trapping
    leader looping.
  - The two leaked shells (pids 52097 and 52276, parent PID 1, started 05:55) were
    that test's leader. They were left when a mutation run removed the grace `SIGKILL`
    during the first round. `ps` showed the test's exact command line, and they were
    killed.
- **Carries for M8a.22:**
  - Bring `server.rs` back to 618 lines or fewer. The milestone's file-size table
    requires net growth of zero, and M8a.22's `GitWiring` extraction is the offset.
  - Remove the restored headless windows of finished runs. A client's `Remove` is
    refused for them, so only the engine can remove them.
  - Register and unregister each headless window's worktree root with the git
    registry. Today a live one gets no git state, and a restored one is registered and
    never unregistered.
- **Carry for M8a.18:** the stale-pid rule (`current`) is now pinned by a direct test.
  M8a.18 must also test it end to end through kill-then-`--resume`.

### M8a.18 turn delivery, interrupt and resume (2026-09-24)

Built `manager/headless_turns.rs` (split out of `manager/headless.rs`: session process
start, `headless_send`, `headless_resume`, `headless_interrupt`, `headless_kill`),
`headless/session/pipes.rs` (split out of `headless/session.rs`: the writer, reader,
waiter and dispatcher threads), the content-mode changes to `headless/conversation.rs`,
and `crates/daemon/tests/headless_turns.rs` with `headless_turns/cursor.rs`.

**Delivery (decision 29).**

- Every send and resume clamps its text with `run::messages::clamp` first, and the
  clamped text is what `sent_turn` records, what goes on Claude's stdin and what is
  Codex's last argument (M8a.7 m3).
- `sent_turn` is applied, and its output enriched, under the lock before the write
  (Claude) or the spawn (Codex), on every path: send, resume and `create_headless`.
- **Claude send:** one `send_line(user_message(clamp(text), session_id))`. An ended
  process is `session for window <id> has ended; resume it`.
- **Codex send:** refused with `a turn is already running for window <id>` while the
  turn's process is alive and no turn has ended in it (or another send or resume is in
  flight: `HeadlessWindow.busy`). A window whose status is `Exited` (a process that died
  before its turn ended, T17-I1, or a restored window) is `…has ended; resume it`.
  Otherwise the manager waits until the previous turn's process has delivered its last
  event (it has already ended its turn and is only exiting), sleeps the launch jitter,
  records the turn, and spawns `codex_args(spec, Resume { session_id }, text, …)`
  through the same `spawn_process` as the first turn, so stdin is closed at once
  (T17-C1).
- **Jitter (invented source).** `Effect::Deliver` carries no `jitter_ms`, so a Codex
  send computes decision 18's `jitter_ms(run_id, task_id, run_ref.session)` from the
  spec's `RunRef` (0 without one). `headless_resume` takes the `ResumeSession` op's
  jitter as a `Duration` argument (its signature gained it).

**Resume (decisions 28 and 52).**

- A live process of the window is killed (`kill(kill_grace)`), and the resume waits,
  on `spawn_blocking`, until its dispatcher has delivered `ProcessExited`
  (`HeadlessHandle::wait_finished`, new). Only then is the next process spawned. So no
  event of the replaced process can arrive after the new one starts, and one session
  never runs in two processes. The wait is bounded by `kill_grace + OUTPUT_GRACE + 1 s`;
  past it the process is killed again and waited for once more; then the resume fails
  with `the previous process of window <id> did not stop; its session cannot run twice`.
- The window's handle is cleared before the spawn, so the new process's events are the
  window's from its `ProcessStarted` on (the stale-pid rule `current` still applies to
  anything else).
- Claude: `claude_args(spec, Resume { id }, …)`, the same builder as the first launch,
  so every flag is re-passed (the user-settings-only flags of decision 53, `--settings`
  with its sandbox block, `--mcp-config`, tools, mode, model, effort, auth). Then the
  message on stdin. Codex: `codex_args(…, Resume { id }, message, …)`.
- `headless_resume` returns once the process prints `Init`. An exit before `Init` is an
  error `could not resume session <id>: the process exited with code <n> before it
  started: <text>`, `<text>` being the failed `result`'s text or else the first stderr
  line (M8a.1's `No conversation found with session ID: <id>`). The driver (M8a.22)
  reports any error from it as `ResumeFailed`.
- **`RESUME_START_TIMEOUT` = 120 s (invented).** A process that prints nothing for that
  long is killed and the resume fails, so a hung start cannot hold its task forever.

**Interrupt.** Unchanged from M8a.17 (Claude by `CLI_CAPS.claude_interrupt`, Codex by
`SIGINT`), now tested through the manager.

**The cursor (M8a.7 re-review 2's minors).**

- **m1:** a headless Claude window's cursor starts as `StreamCursor::fed()`, in content
  mode, so its first `sent_turn` records no timing-based prompt. Test:
  `a_fed_window_starts_its_cursor_in_content_mode` (its lost-first-hook half).
- **m2:** `awaiting_prompt` is replaced by `open_prompts`, the observed prompts whose
  turns have not ended (with each one's `human` flag). Prose is dropped only when none
  is open, so a prompt hook applied before the previous `result` keeps both replies.
  This is the count rule the review proposed, kept as a queue so a lost hook cannot
  skew it for the rest of the session. Test: `a_late_prompt_hook_drops_only_its_own_turn`.
- **The background `result` (re-review 2, out of scope there, carried).** A `TurnEnded`
  closes the first open prompt that is the daemon's, with every older open prompt (an
  unprompted prompt taken into that turn mid-turn, case D). With no open daemon prompt,
  it closes the oldest open one, and when that is an unprompted turn and a sent text is
  still waiting for its prompt, `StreamCursor::ended_unprompted()` is true. The manager
  then leaves the window's status alone (the delivered turn is still open) and
  publishes the event as the new `WindowSignalKind::Unprompted(event)`, not
  `Session(event)`. M8a.22's translation maps it to activity, never to the delivered
  turn's `TurnEnded`; its usage is still spend. Test:
  `a_background_turns_result_does_not_end_the_delivered_turn`.
  - **Known limit.** When the next sent prompt's hook is applied before a background
    turn's `result` (the N1 race and m2's reordering at once), that `result` closes the
    delivered prompt instead. Both reorderings need scheduling under load.
  - `a_headless_claude_prompt_hook_feeds_the_cursor` (M8a.17) now expects its third
    turn's end as `Unprompted`: that is its N1 background turn.
- **m4:** the M8a.18 task text says a prompt observed before its `sent_turn` "is still
  classified as the daemon's". The code records it as `human: false` and places it
  correctly, which is what matters (the review's point); the brief text is left as is.

**Files (rule 8).** `session.rs` 583 → 399 lines plus `session/pipes.rs` 253;
`manager/headless.rs` 574 → 490 plus `manager/headless_turns.rs` 406.

**Acceptance.** `rg -n "write_input" crates/daemon/src/run crates/daemon/src/headless`
prints three lines, all the Codex usage field `cache_write_input_tokens` in
`codex_stream.rs` and its tests (M8a.7's, unchanged). `rg -nw write_input` over the same
paths prints nothing: no PTY write reaches the engine or the headless layer. No lock
guard in `manager/headless.rs` or `manager/headless_turns.rs` is alive across a spawn,
a write, a kill or an `.await`; `TurnClaim` takes the lock only in its `Drop`.

**TDD evidence.** The tests were written first, against stubs of the new API (the
`Unprompted` variant and the `jitter` argument, neither doing anything):

- 11 of 14 failed: the three resume tests and the stale-pid test with `resuming a
  headless session is not implemented yet (M8a.18)`; `resume_failure_is_reported` on its
  text assertion; the Codex send tests with `left: "a Codex turn is a new exec resume
  process (M8a.18)"`; `sent_turn_records_the_clamped_text` on the unclamped stdin line;
  `a_background_turns_result…` on `Session(TurnEnded)` where `Unprompted` was wanted;
  `a_late_prompt_hook…` with `left: [("first", "reply one"), ("second", "")]`.
- `a_fed_window…`'s first half (a background turn first) passed on the old code, so a
  lost-first-hook half was added; it failed with `left: [("second", "")]`.
- `claude_send_writes_one_envelope_line` and `interrupt_follows_cli_caps` passed before
  the change: M8a.17 had built the Claude send and the interrupt. They pin them.
- Mutations: recording a Codex turn after its spawn (with a 300 ms pause to force the
  race) fails `sent_turn_is_recorded_before_the_write` (`left: …("second", "")`).
  Resuming without waiting for the killed process first survived the stale-pid test;
  the test's first process now takes 0.3 s to die, and the mutant fails it.

**For M8a.22.** Because a Codex send waits for the previous process's exit, that exit
always reaches the feed before the next process's `ProcessStarted`, and after the
engine emitted the `Deliver`. This is the order the M8a.13 fix-round-2 follow-up names:
the reducer side of T13-P1 (remember the pid whose turn closed; its later exit is
normal) must cover it.

### M8a.18 fix round 1 (2026-09-24)

Review `task-18-review.md`: 2 Important, minors. Rulings T18-I1, T18-I2 and T18-minors
in `progress.md`. Every fix has a test that failed first, and each was mutation-checked:
reverting it fails its test.

- **I1: a kill or interrupt between the old process and the new one is no longer lost.**
  - While a send or resume holds the window (`busy`), `HeadlessWindow.cancel` holds a
    `watch` sender. `headless_kill` records `Cancel::Killed` there (and still signals
    whatever handle is installed). `headless_interrupt` records `Cancel::Interrupted`
    when no process is installed; once one is, it signals it as before.
  - The jitter wait ends early on a recorded cancel. `record_turn` refuses to record the
    turn, and `install` refuses to install the new process and kills it at once. The
    send or resume returns `window <id> was killed before its turn started` (or
    `interrupted`).
  - A kill while a resume waits for `Init` is returned the same way, not as
    `could not resume session …`, so the driver does not take it for a failed resume.
  - `ManagerConfig.headless_install_pause` (new, zero in production) holds a spawned
    process before its install, so a test can put a kill in that gap.
  - Tests: `a_kill_during_a_resume_gap_stops_the_resume` (the reviewer's probe),
    `a_kill_before_the_install_kills_the_new_process`,
    `an_interrupt_during_a_codex_send_gap_stops_the_turn` and
    `a_kill_during_the_init_wait_is_reported_as_the_kill`. Before the fix, the probe
    test's resume ran its full jitter and started the process, the interrupt returned
    `the session has ended`, and the Init-wait kill came back as a failed resume.
- **I2: a new process clears the cursor's open prompts.** `record_turn` (every Codex
  turn and every resume) calls `StreamCursor::process_replaced`. The turns still open in
  the replaced process can never end. Unmatched sent texts are kept. Test:
  `a_resume_after_a_killed_turn_keeps_background_results_unprompted` (the reviewer's
  probe). Before the fix, the background `result` came through as `Session(TurnEnded)`.
- **Minors.**
  - A message with a NUL byte is refused before its turn is recorded, with `a message
    for window <id> contains a NUL byte`. For send, the driver (M8a.22) treats any error
    as `Delivered { ok: false }`. Test:
    `a_message_with_a_nul_byte_is_refused_before_it_is_recorded`. A spawn that fails
    for any other reason after `record_turn` still leaves its `User` turn in the
    conversation (the cost of recording before the spawn).
  - The three surviving mutants are now killed:
    - The Codex busy check: `two_codex_sends_at_once_start_one_turn`.
    - The kill on a resume timeout: `a_resume_that_never_starts_is_killed_at_the_timeout`.
      The timeout is now `ManagerConfig.resume_start_timeout`: `RESUME_START_TIMEOUT`
      (120 s) in production, 500 ms in the test.
    - The Codex turn jitter: `a_codex_send_waits_out_the_launch_jitter`. The jitter is
      injected as `ManagerConfig.codex_turn_jitter` (`None` in production) at 1.5 s. The
      check is a lower bound on the send's own duration, never a race. The formula has
      a unit test (`a_codex_turns_jitter_is_decision_18s`).
  - **The acceptance grep should read `rg -nw write_input`.** The brief's `rg -n`
    matches the Codex usage field `cache_write_input_tokens`.
- **Carry for M8a.22:** a `ResumeFailed` for a round the engine has already killed,
  retired or cancelled is ignored. A kill during the `Init` wait now returns
  `…was killed…`. But a kill racing the process's own failure can still produce a
  resume error, and decision 28 must not start a fresh session for a round that is gone.
- **Files.** `manager/headless_turns.rs` is 534 lines and `manager/headless.rs` 516,
  both under the limit.

### M8a.19 the MCP server (2026-09-24)

- **Package name.** The CLI package is `anthrex`, not `anthrex-cli`. The task's
  `cargo test -p anthrex-cli` therefore runs as `cargo test -p anthrex`.
- **`--role orchestrator` parses.** The CLI line lists `<worker|reviewer>`. But
  `daemon::headless::argv::mcp_args` can emit `orchestrator`, so `anthrex mcp` accepts
  it and serves no tools (`tools_for(Orchestrator)` is empty). Test:
  `mcp_cmd::tests::mcp_parses_the_daemons_headless_argv_and_is_hidden`. It feeds the
  launcher's own argv, for all three roles, into the CLI parser.
- **The CLI arguments live in `crates/cli/src/mcp_cmd.rs`.** They were not added inline
  to `main.rs`, which would then have been 683 lines. `main.rs` is now 590.
- **A `DaemonMsg::Error` ends the wait for a reply.** The brief says to skip anything
  other than `RunReply::ToolResult`. But an `Error` is the daemon refusing this
  connection's request (for example a daemon without the run engine), and skipping it
  would hold the agent for the whole 100 s `TOOL_REPLY_TIMEOUT`. It becomes
  `isError: true` with the daemon's message. A handshake `Error` (for example a
  protocol mismatch) is reported as
  `cannot reach the anthrex daemon at <socket>: <message>`.
- **Error texts the brief does not fix.** These all come back as `isError: true`:
  - connect timeout: `cannot reach the anthrex daemon at <socket>: connect timed out after 2 s`;
  - handshake timeout: `…: no handshake within 5 s`;
  - reply timeout: `the anthrex daemon did not answer <tool> within 100 s`;
  - closed before the reply: `the anthrex daemon closed the connection before answering <tool>`.
- **Protocol version.** `get_info` advertises `2025-06-18` as the fallback. rmcp's
  default negotiation echoes any known version the client asks for, so a client asking
  for `2025-06-18` gets `2025-06-18`. The supported list is rmcp's default.
- **Arguments are not validated in the server.** The schemas tell the model the limits.
  The engine re-validates and answers `invalid arguments: …` (M8a.22). A missing
  `arguments` is forwarded as `{}`.
- **Flakes seen in the workspace run.** Both are already recorded in the follow-ups
  file, and neither crate depends on `anthrex-mcp`:
  - `git_registry::a_commit_in_a_linked_worktree_triggers_a_probe`;
  - `server_git::two_windows_in_one_worktree_register_once`, which failed 2 of 3 runs
    alone, as recorded for this worktree's `target/`.

### M8a.19 fix round 1 (2026-09-24)

- **I1: a refused handshake becomes a fixed text.** Any `DaemonMsg::Error` answering
  `Hello` now comes back to the agent as `the anthrex daemon speaks a different protocol
  version; the user must restart it` (`mcp::forward::VERSION_MISMATCH`).
  - Why: the daemon only refuses a `Hello` over a version mismatch, and its text
    (`server.rs`) says to run `anthrex daemon stop`. An agent with a shell might run it.
  - The daemon's full text goes to stderr only, as
    `anthrex mcp: the daemon refused the handshake: <text>`.
  - This replaces the first round's "reported with the cannot-reach prefix" for a
    handshake `Error`.
  - Timeouts, EOF and a non-`Welcome` reply keep the `cannot reach …` prefix. The
    non-`Welcome` detail is now just `unexpected handshake reply`, no longer a `Debug`
    dump of the message.
- **M1: only a labelled `Error` ends the wait.** After the handshake, only a
  `DaemonMsg::Error` whose `request` is `proto::run_wire::request::TOOL` (`"run tool"`,
  new) ends the wait for a reply. Any other `Error` is skipped, like a broadcast.
  - The daemon has no such label today: it answers every `RunRequest::Tool` with
    `RunReply::ToolResult`, refusals included (`server.rs`, until M8a.22).
  - So the constant is added for M8a.22 to use if it ever refuses a tool call with an
    `Error`. It is a label constant, not a wire change, so `PROTO_VERSION` stays 7.
  - A labelled `Error`'s message is passed to the agent as it is. It is the engine's
    own answer to this call and has the same standing as `ToolResult.text`, so M8a.22
    must word it for an agent.
- **M2: the stub counts raw connections.** The stub daemon now counts every raw
  `accept()`, before anything is read. The never-reaches-the-daemon test and the
  listing test assert that count is 0.
  - The stub and the JSON-RPC client moved to `crates/mcp/tests/support/mod.rs`, shared
    by `stdio.rs` and the new `daemon_errors.rs`.
- **M3: the next CLI subcommand goes in its own file.** `main.rs` is at 590 lines, so
  the next subcommand (for example `run` in M8a.23) goes in its own `*_cmd.rs`, like
  `mcp_cmd.rs` and `tree_cmd.rs`.

### M8a.20 `fake-agent` headless modes (2026-09-24)

- **Package name.** The crate is `anthrex-fake-agent`, so the task's tests run as
  `cargo test -p anthrex-fake-agent`.
- **Files beyond the brief's list, for the 600-line rule.**
  - `src/headless_steps.rs` is a child module of `headless.rs` holding the steps that
    run inside a turn (`run_step`, `mcp_call`, `sh`). `headless.rs` would otherwise have
    been 697 lines.
  - The two shape tests are in `tests/headless_shapes.rs`, not `headless_modes.rs`.
    Both files share `tests/headless_support/`: `mod.rs` holds the owned processes,
    the argv builders, the git repository and the stub daemon, and `shape.rs` holds
    decision 51's check. `headless_modes.rs` would otherwise have been 730 lines.
- **Turn rules the brief left open.**
  - A `read_message` at the script position takes the message that opens a turn,
    including the first one. So a script that starts with
    `{"read_message":{"expect":"…"}}` checks the first prompt.
  - `end_turn` ends the turn. In Claude mode, if the next step is not `read_message`,
    an unprompted turn (`system/init` with no message) starts at once, as M8a.1
    recorded Claude Code doing. In Codex mode the process exits 0, and the rest of the
    script runs on the next `exec resume` message. `end_turn` followed by
    `read_message` behaves the same on both runtimes.
  - After `fail_turn` or an interrupt, a Claude session waits for the next message
    before it goes on. A Codex process exits 1 after `turn.failed`, as the real CLI did
    (M8a.1 item 7).
  - An interrupt is answered with the recorded `control_response`. During a turn, a
    `result` with `subtype: "error_during_execution"` and `terminal_reason:
    "aborted_streaming"` follows. Between turns nothing else follows, since M8a.1 found
    a late interrupt harmless. `hang`, `wait_ms`, `api_retry`'s delays and `sh` (whose
    process group is killed) can be interrupted. `mcp_call` cannot.
  - A Claude `read_message` that times out exits 4. EOF while waiting exits 0.
- **Script state.**
  - The position is saved as the index of the next step *before* a step runs. So a
    process killed in `hang`, or ended by `exit`, resumes after that step. A
    `read_message` that ends a turn saves its own index instead, so the next process's
    message is checked by it.
  - `capture` values and `FAKE_AGENT_RESULT` persist in `<file>.vars` next to `.pos`,
    so a template or `sh` step still sees them in the next Codex process. The brief
    names only `.pos`.
  - A resume finds its claim by session id whatever the argv's role. A resume with no
    claim claims afresh, or falls back to `FAKE_AGENT_SCRIPT` from step 0.
- **Output details.**
  - `print` is an assistant text (Claude) or an `agent_message` item (Codex).
  - `title` and `bell` write nothing in a headless mode.
  - `read_line` is an error there (exit 1).
  - The new steps in the terminal mode print `fake-agent: <step> needs a headless mode`
    and exit 3. This covers `mcp_call`, because a terminal argv carries no MCP server.
  - `sh` on Claude is a `Bash` `tool_use` / `tool_result` pair. A non-zero exit gives
    `is_error: true` with the text `Exit code <n>\n<output>`. On Codex, a non-zero exit
    emits no item at all (M8a.1 item 7).
  - A failed Codex `mcp_call` is `status: "failed"` with `error: {message}` and
    `result: null`, as in the `-mcp-approval-auto` recording. A successful one has
    `result: {content, structured_content: null}`, as in `-approve`.
  - Codex `fail_turn` with an error other than `rate_limit` uses the error string as
    the message.
  - `deny` and `api_retry` write nothing on Codex.
  - A `hook` step's payload carries the session's id as `session_id` in a headless
    mode.
  - The default turn usage is 10 input and 5 output tokens.
  - Claude's `mcp_servers` is `[]` without `--mcp-config`.
- **Input.** A bad Claude stdin line exits 5 from the reader thread at once, with
  `fake-agent: bad input line` on stderr. An envelope's `session_id` is optional and is
  not compared with the session's. `FAKE_AGENT_STDIN_FILE` naming a file appends every
  line. `FAKE_AGENT_ARGS_FILE` naming a file is still overwritten per process, as in
  milestone 3. In the terminal mode, a directory there gets
  `<FAKE_AGENT_SCRIPT's stem, or fake-agent>.args`.
- **Decision 51's check, made precise.**
  - An event's type is `type`, plus `/subtype` or `/<item.type>`.
  - `input`, `arguments` and `tool_input` are opaque: only their presence is checked.
  - Every element of an array the fake writes must be contained in some element of
    the fixture's array.
  - Observed recordings are tried first, then the documented file. Only the documented
    file has the failed turn's synthetic `assistant` line (`error`,
    `is_api_error_message`), a type the observed files also have.
  - The stdin fixture and the hook payloads are not stream events and are skipped.
  - Mutants proved it: an invented `system/init` key and an invented
    `result.is_error` inside a Codex `mcp_tool_call` both failed the shape tests.
- **The daemon's parsers read the fake's output.** As a one-off check (not
  committed), a Claude session (text, `sh`, `deny`, `api_retry`, `fail_turn`, an
  interrupt) and a Codex turn went through `ClaudeStream::parse_line` and
  `codex_stream::parse_line`. No line was `Unknown`, and the events were as expected.
  That covers `Failed { RateLimit }` from the synthetic line, and `Interrupted`.
- **`anthrex` for `mcp_call_talks_to_a_real_mcp_server`.** The `anthrex` next to
  `fake-agent` is used when present, which `cargo test --workspace` guarantees.
  Otherwise it is built once behind a `OnceLock` into `<target>/fake-agent-anthrex`, a
  separate target directory, so the outer `cargo` cannot hold its lock. That build took
  about 30 s here.
- **Carry T17-C1.** Codex mode reads stdin to EOF before anything else it writes. Test:
  `codex_mode_reads_stdin_to_eof_before_it_starts`.
  - The test first waits for the argv log, which is written before stdin is read, and
    only then checks that there is no output for 500 ms.
  - Without that gate, the first run of a freshly built binary took about 600 ms to
    start. The check then passed with the read removed, which proved nothing.
  - With the gate, the same mutant fails.
- **Bounds.**
  - `MCP_CALL_TIMEOUT` is 120 s, Codex's own `tool_timeout_sec`, above `anthrex mcp`'s
    100 s reply timeout.
  - `sh` and `capture` are bounded at 120 s, and the `git rev-parse` at 5 s.
  - The tests wait 20 s for a process without an MCP call. For one with an MCP call
    they wait 150 s, which is `MCP_CALL_TIMEOUT` plus slack.

### M8a.20 fix round 1 (2026-09-24)

Review `.superpowers/sdd/M8a-orchestration-engine-core/task-20-review.md`, rulings T20-I1,
T20-I2, T20-I3 and T20-minors.

- **I1: `sh` and `capture` children never outlive the fake.**
  - The child now stays in the agent's process group, so the daemon's group kill
    reaches it. That includes a SIGKILL, which no handler can see.
  - On an interrupt or `SH_TIMEOUT`, the fake stops the child, lists its descendants
    with `ps -A -o pid= -o ppid=`, and SIGKILLs the whole tree. The group itself
    cannot be signalled without the fake.
  - In a headless mode, SIGINT, SIGTERM and SIGHUP handlers only touch atomics. With
    no `sh` running, the signal is re-raised with its default action at once. With an
    `sh` running, the `sh` loop sees the flag within `POLL` (10 ms), kills the tree,
    then re-raises.
  - This covers Codex's interrupt, which is SIGINT to the leader alone, and a
    background job inside `sh`, which ignores SIGINT even when the whole group gets it.
  - Test: `sh_children_die_with_the_agent`, with SIGTERM to the group, SIGKILL to the
    group, and SIGINT to the leader alone. In each case a background `sleep` must be
    gone within a deadline.
- **I2: decision 51's check runs both ways.**
  - Every recorded sample of an event type, observed and documented, is merged into
    one schema: the JSON types at each path, the keys allowed, and the keys every
    sample has.
  - Array elements are grouped by their `type` string.
  - A key some sample lacks is optional. Each key every sample has must be written.
  - `input`, `arguments`, `tool_input`, `modelUsage` and `by_type` hold free-form data
    (a tool's arguments; maps keyed by model or agent-type names). Only their JSON type
    is checked.
  - Numbers are one type, so integer and float are not told apart.
  - To pass it, Claude's lines now carry every key the recordings always have: the
    whole `system/init`, the assistant `message` envelope and `usage`, `caller` on
    `tool_use`, and on `result` the full `usage`, `modelUsage: {}`, `subagent_stats`,
    `fast_mode_*`, `queued_turn_count` and `result_index`.
  - `mcp_servers` entries now include `source: "dynamic"`, which every recording has.
    The brief's shape omitted it.
  - The reviewer's M5b, M6 and M7 mutants are now killed.
- **I3: the surviving mutants each have a test,** in the new
  `tests/headless_turns.rs`:
  - `a_read_message_timeout_exits_4`;
  - `end_turn_ends_the_turn`, on both runtimes;
  - `an_exit_mid_turn_writes_no_turn_end`, on both runtimes;
  - `an_interrupt_ends_an_sh_and_kills_it`;
  - `mcp_call_sends_initialized_between_initialize_and_the_call`, against an `sh`
    MCP server that logs what it reads;
  - `a_claude_expect_mismatch_exits_3`.

  Each was shown red by its mutant, because the code already behaved correctly.
- **m1.** `anthrex-daemon` is now a dev-dependency of `fake-agent`, with no cycle.
  The tests build their argv with `daemon::headless::argv::{claude_args, codex_args}`
  and `CLI_CAPS`. The placeholder flag is passed through
  `codex_user_config_only`. The spec's instructions contain `--`, `-p`, quotes and a
  newline.
- **m3.**
  - An `sh` or `capture` is now announced as a `Bash` `tool_use` before it runs.
  - An interrupt inside one writes the recorded rejection `tool_result`, then
    `[Request interrupted by user for tool use]`, then a `result` with
    `aborted_tools` and `stop_reason: "tool_use"`.
  - An interrupt outside a tool (`hang`, `wait_ms`, an `api_retry` delay) keeps
    `aborted_streaming`, with `stop_reason: "end_turn"`. That is unrecorded: the only
    recording has a string there, so `null` would fail I2's type check.
- **m4.** `.pos` and `.vars` are written to `<file>.tmp`, then renamed into place.
  Test: `roles::tests::state_files_are_replaced_by_a_rename`, which checks that the
  inode changes and that no temporary file is left.
- **m5.** A Claude `hang` checks for stdin's EOF every 100 ms and exits 0 when it
  sees it, as the real CLI exits at EOF. Test: `a_claude_hang_ends_at_stdin_eof`.
- **m6.** `SH_TIMEOUT` is now 330 s: the spec's `RUN_WAIT` (300 s) for M8a.24's longest
  scripted loop, plus slack. Test: `sh_timeout_outlasts_the_specs_run_wait`.
- **m8.** Claude accepts exactly one text block, as recorded. `session_id` stays
  optional, because `claude_stream::user_message(text, None)` omits it. Test:
  `refuses_every_other_shape` now includes two blocks.
- **Carries for M8a.24 (no code):**
  - **m2.** `mcp_call`, `hook` and `git_commit` cannot be interrupted. An interrupt
    during one stays queued: the step finishes, the turn ends `success`, and only then
    is the interrupt answered with a bare `control_response`. No M8a.24 scenario
    should interrupt during a tool call and expect `Interrupted`.
  - **m7.** On Claude, the steps after `end_turn` run at once, as an unprompted turn.
    On Codex they run only on the next `exec resume` message. So the same script
    behaves differently on each runtime unless `end_turn` is followed by
    `read_message`.
  - **m7, continued.** The fake fires no turn hooks (`UserPromptSubmit`, `Stop`) of
    its own. So a fake Claude session's conversation view gets no hook feed, whether
    a turn is prompted or not.

### M8a.21 the intent journal and reconciliation (2026-09-24)

- **Files.** `run/journal.rs`; `run/reconcile/` instead of `run/reconcile.rs`, split by
  seam for the 600-line rule: `mod.rs` (types, the journal lookup, the per-kind
  dispatch), `git.rs` (the git rows), `sessions.rs` (the `CreateWindow` row and decision
  28's leftover processes). The tests are `tests/run_journal.rs` plus
  `tests/run_journal/{fixture,git,sessions}.rs`.
- **Interface change: `reconcile` returns `Reconciliation { ops, notes }`**, not a bare
  `Vec<(OpId, Reconciled)>`. `ops` is that vector, in op order. `notes` says what
  reconcile killed, removed, pruned, re-attached or aborted, and what it could not
  read; M8a.22 should put them in the run's log. `Reconciliation::replay(run_id)` gives
  `Event::Restore`'s `replay` entries for the run.
- **Journal additions.** `JournalLine::op()`, `runs_dir`, `needs_compact(dir)` (the 1 MiB
  test), and the file-name constants. `JournalLine` is `#[serde(untagged)]` with the
  fields renamed, so the lines are exactly `{"op":1,"intent":{…}}` and
  `{"op":1,"done":{…}}`.
- **Deviation: `compact` also keeps the `done` line of a still-pending op.** Decision 43
  says "only pending ops' intents". A result appended between the op's return and the
  `Persist` that retires it would otherwise be lost to a compaction in that window, and
  reconcile would re-check reality instead of replaying it. The intents come from the
  `pending` map (the persisted `run.json`), not from the old journal.
- **Durability, as the acceptance asks.** Every write in `journal.rs` is followed by
  `sync_all`: `save_run` (`run.json.tmp` synced, renamed, the directory synced),
  `append` (one `write_all`, `sync_all`; the directory synced when the file is new),
  `compact` (temp, sync, rename, directory sync), and `ensure_dir` syncs the parents of
  a new run directory. `load_all` removes a leftover `run.json.tmp` or
  `journal.jsonl.tmp`, skips a run whose `run.json` is missing or bad with a problem,
  drops an unparseable line with a problem, and drops a torn last line (no `\n`) with a
  problem that says `torn`.
- **Reconcile rules the table left open.**
  - An op with a `done` line replays it without any git call; an op with no line at all
    is `NotStarted` without any git call; a line for an op no longer in `pending_ops` is
    ignored.
  - A check that cannot read reality (a git error or timeout) answers `NotStarted` with
    a note. Every op is idempotent, and a re-issued `MergeCandidate` over a run branch
    that did move is caught by its own ref guard.
  - `CreateRunBranch`/`PrepareWorktree`: listed on the branch **and** the directory
    exists → `Worktree { head }` (the porcelain `HEAD`), unless there is a `setup`. A
    path listed on another branch is `NotStarted` and left alone. Only an unlisted
    directory is removed, then `git worktree prune`.
  - `MergeCandidate`: a deleted run ref is `RefMoved { "<ref> was deleted" }`; a moved
    one is `RefMoved { "<ref> moved from <sha7> to <sha7>" }`, `guard_refs`'s texts. The
    integration worktree is re-attached in the `Merged` case too (a crash between the
    CAS and `reattach`), and only when it is not already on the run branch.
  - `HandBack`: `MERGE_HEAD` must be the run head and there must be conflicted files;
    otherwise `NotStarted` with a note. `head` and `onto` are filled as `git::hand_back`
    reports them (T14-C1): the conflicted case has both `HEAD`; the clean case (`HEAD^2`
    is the run head) has `head = HEAD`, `onto = HEAD^1`.
  - **Carry (ruling on task 15): `AbortMerge` with no `MERGE_HEAD` → `MergeAborted`**;
    with one → `NotStarted` (the reducer re-emits the idempotent abort). Test:
    `reconcile_abort_merge_without_merge_head_is_aborted`.
  - `RemoveWorktree`: a gone path still registered (the daemon died before the prune) is
    unlocked and pruned; `salvage_ref` is reported only if the ref exists.
  - `Accept`: the conflicted-accept abort is `git merge --abort` with decision 18's
    flags. `kept_branches` is empty in the replayed `Finished`.
  - `CreateWindow`: the window must be `Headless` with `run` equal to the spec's
    `RunRef` (the round's, if the spec has none); the highest id wins.
- **Leftover session processes (decision 28), made precise.** The candidates are the
  session ids the run still has live: every round not `ended` with a `session_id`, each
  pending `CreateWindow`'s `session_uuid`, each pending `ResumeSession`'s `session_id`
  (ids shorter than 8 characters or with whitespace are skipped). One
  `ps -ww -A -o pid= -o pgid= -o args=` finds every process whose command line contains
  one. That also finds a process whose pid was never recorded. A process without the
  id is never signalled, whatever pid a round recorded. `SIGTERM` goes to the group when
  the process leads its own group (and it is not the daemon's group), else to the pid;
  after `SESSION_KILL_GRACE` (2 s), a pid whose `ps -o args= -p` still carries the id
  gets `SIGKILL`, so a pid reused meanwhile is never signalled. Codex's first turn
  carries no id on its command line and cannot be found; its round has no `session_id`
  yet either.
- **Git layer.** `worktrees::Listed` gained `head` and is `pub(crate)`; `listed`,
  `forget_missing`, `is_ancestor`, `read`, `short`, `unmerged` and a new
  `reattach_in(Git, …)` (the body of `reattach`) are re-exported `pub(crate)` for
  reconcile. `OpKind::name()` was added (for notes, and for decision 48's crash
  injection in M8a.22).
- **Carry T8-RR2 is untouched.** Reconcile never computes a diff: `VerifyDone`,
  `DiffSoFar` and `CountCommits` are `NotStarted` with no reality check. The
  `<run_head>...HEAD` range after `resume --rebaseline` stays for M8a.22 or the final
  review.
- **Left to M8a.22.** Building `Restore`'s `replay` from `Reconciliation::replay`,
  sending `Restore` before any other event, compacting the journal after the restore,
  and filling `DoneChecked.resolution_only`.
- **Concerns for later tasks.**
  - A replayed `Accept` `Finished` skips the clean-up that follows the merge (salvage,
    worktree removal, branch deletion), so a crash between accept's merge and its
    clean-up leaves those worktrees and branches. The table asks only for `Finished`.
  - `session_uuid(run_id, op)` is deterministic. Two daemons with the same run id (a
    test daemon and another one) would share the uuid of the same op, and one's
    reconcile could kill the other's session. Run ids carry a random suffix, so this
    needs a collision.

### M8a.21 fix round 1 (2026-09-24)

The review (`task-21-review.md`) found 3 Important and 5 Minor findings. The rulings
T21-I1, T21-I2, T21-I3 and T21-minors are binding. Every change has a test that failed
first.

- **I1 / T21-I1: the leftover-session kill is decision 28's again.** The `ps -A` scan
  is gone. Only the recorded `pid` of a round that has not ended is examined, with the
  round's `session_id` (else its launching `CreateWindow`'s `session_uuid`). It is
  signalled only when it is alive (not a zombie), runs as the daemon's uid, has parent
  `ORPHAN_PARENT` (1: the old daemon is dead), and its argv, read element by element
  (`KERN_PROCARGS2` on macOS, `/proc/<pid>/cmdline` on Linux, none elsewhere), holds the
  id as a whole element or as `--session-id=<id>` / `--resume=<id>`. `killpg` is used
  only when pgid == pid (and not the daemon's group), else `kill`. After
  `SESSION_KILL_GRACE` a pid that still passes every check gets `SIGKILL`.
  - The orphan parent is injectable for tests only: `reconcile_with_orphan_parent`.
    `reconcile` passes `ORPHAN_PARENT`. On macOS the tests' orphans are adopted by
    launchd (pid 1), which the kill test asserts, so they run the production path.
  - Tests use only processes the test spawned: an orphaned `sleep` (made by
    `bash -c 'set -m; (exec -a <id> sleep 120) & echo $!'`) with a recorded pid is
    killed; the same with no recorded pid, a recorded orphan whose argv holds the id
    only inside `/x/<id>.jsonl`, and a recorded non-orphan (the test's own child) are all
    left alone. Each test's clean-up kills only its own process, after checking its
    argv. Unit tests cover `argv_carries` and the `KERN_PROCARGS2` parser.
  - Not found, by design: a session whose pid was never recorded, and Codex's first
    turn (no id in argv; followups file).
- **I2 / T21-I2: a torn tail is cut before the next append.** `append` checks the
  journal's last byte; if it is not `\n`, the file is cut back to the last `\n` and
  `sync_all`ed before the line is written. Test:
  `an_append_after_a_torn_line_survives_the_next_load` (the review's P1).
- **I3 / T21-I3.** `reconcile_accept_leaves_the_users_own_merge_alone` (kills M4) and
  `reconcile_hand_back_ignores_a_merge_of_something_else` (kills M5), in the new
  `tests/run_journal/git_guards.rs`.
- **m1: path guard.** Reconcile removes an unregistered directory only when it is
  strictly under `<wt_dir>/runs/<run>` with no `.` or `..` component; anything else is
  `NotStarted` with a note (`left <path> alone: it is outside the run's worktrees`).
  Test: `reconcile_never_removes_a_directory_outside_the_runs_worktrees`.
- **m2: a replayed accept reports its clean-up honestly**, per `OpKind::Accept`'s
  contract: `Finished { outcome: "accepted as <sha7>; clean-up did not run before the
  restart", kept_branches }`, where `kept_branches` lists every branch under
  `refs/heads/<branch_prefix>`. This supersedes the empty `kept_branches` above.
- **m3: `compact` returns the old journal's problems** (`io::Result<Vec<String>>`), for
  the driver to log. Test: `compact_reports_the_lines_it_cannot_read`.
- **m4.** `load_all_removes_a_leftover_journal_temp_file` (M1),
  `reconcile_prepare_worktree_whose_directory_is_gone_is_not_started` (M2),
  `reconcile_merge_candidate_with_swapped_parents_is_ref_moved` (M3).
- **m5: the accept abort in the user's checkout** now uses `Git::user_write`: the
  scrubbed environment and `--no-optional-locks`, without decision 18's engine flags,
  as `git::accept` treats that checkout. This supersedes "with decision 18's flags"
  above. Test: `the_accept_abort_in_the_users_checkout_has_no_engine_write_flags`.
- **Carries to M8a.22.**
  - Never compact between the `run.json` write and the intent lines.
  - Re-run accept's clean-up after a replayed `Finished`.
  - Put a random per-run nonce in session uuids.
  - The last two, and the Codex first-turn marker, are also in the followups file.

### M8a.22 `RunService`, the server and lifecycle (2026-09-24)

- **Commits.** The pure move `refactor(daemon): move GitWiring and pump_git into
  server/git_wiring.rs` (no behaviour change; `server.rs` 627 → 569 lines), then one
  `feat(daemon)` commit with everything else.
- **Files (deviation: split for rule 8).** The brief names `run/driver.rs`,
  `run/driver/ops.rs` and `run/driver/observe.rs`. The driver came to about 2 600 lines
  with its unit tests, so it is split by seam: `driver.rs` (types, the event loop, the tick, stop, the
  forwarder), `driver/effects.rs` (executing effects, persists, the journal, reports,
  compaction), `driver/ops.rs` (the op executors), `driver/merge.rs` (the merge
  candidate), `driver/cleanup.rs` (salvage and removal, accept, discard),
  `driver/requests.rs` (client requests; start, finish and resume read git first),
  `driver/restore.rs` (decision 44 on start), `driver/observe.rs` (the translation). The
  e2e tests are `crates/cli/tests/run_e2e_basic.rs` (11) and `run_e2e_settings.rs`
  (decision 53's 4, split off so neither file passes 600 lines), with the shared plan
  and script builders in `tests/support/run_plans.rs`. The manager's retirement is in
  the new `manager/headless_end.rs` (`headless.rs` and `headless_turns.rs` were near the
  limit).
- **Interface changes.**
  - `RunContext` gains `cli_caps: CliCaps` (the manager's, so decision 53's check reads
    the caps sessions are launched with, test overrides included) and a constructor
    `RunContext::new(data_dir, &ManagerConfig, orchestrator, git_roots)`.
    `RunService::for_manager(&manager, data_dir, git_roots)` builds one with the default
    `[orchestrator]` for a test daemon (the four `serve` callers in tests use it).
  - `RunService::stop` is `async` (it waits for the loop's last `run.json` writes);
    `restore` takes `self: &Arc<Self>`. New: `pushes()` (a broadcast of every published
    snapshot, so a subscriber sees every structural change, which a `watch` would
    coalesce) and `current()`.
  - `ManagerConfig.cli_caps` (default `CLI_CAPS`); `from_env` applies decision 53's
    debug-build overrides through the new pure
    `headless::argv::caps_with_test_overrides` (`ANTHREX_TEST_NO_SETTING_SOURCES=1`,
    `ANTHREX_TEST_CODEX_PROJECT_CONFIG=load|exclude`, the placeholder flag
    `TEST_EXCLUDE_PROJECT_CONFIG`). Every session's argv is built from it.
  - `WindowManager::{config, headless_retire, headless_pid}`.
  - Reducer: `AgentSignal::Spend { usage }`, `AgentRound.closed_pid`,
    `Run.session_nonce` (both `#[serde(default)]`), `role_launch::session_uuid_of`.
- **Event loop.** One task steps the reducer for every event (requests, op results,
  signals, the 1 s `Tick`) and executes that step's effects before the next event.
  Under the engine lock it only steps and clones what the effects need (an urgent
  `Persist`'s run, an op's project and timeouts, a structural `Publish`'s snapshot).
  Counter-only persists are flushed on the tick at most every 5 s; counter-only
  publishes on the next tick; reports at most every 500 ms per run. Journal compaction
  runs on the tick, between steps, so never between a `run.json` write and its intent
  lines.
- **Ops.** The intent is appended (fsynced) before the op task is spawned, and the
  task appends `done` before sending `OpDone`. Appends and compactions share one mutex,
  held only inside `spawn_blocking`. Every op of a run takes a shared per-run lock at
  emission; `Accept` and `Discard` take it exclusively, so they run after every op of
  their run emitted before them (M8a.8's pre-warm concern). After `stop`, an op's result
  is neither journaled nor sent: reconcile checks reality for it on the next start,
  rather than replaying a failure the shutdown caused.
- **Kills.** `KillWindow` records the pid the window last started (`headless_pid`); the
  loop marks that pid's exit `killed_by_engine`. A window with no live process whose
  round still has no exit coming (a Codex window between turns, a send waiting out its
  jitter) gets a synthetic `ProcessExited { killed_by_engine: true }` at once.
- **Retirement (decision 52).** `RetireWindow` → `headless_retire` (stdin closed; a
  window with no live process is `Exited` at once), a kill at `INTERRUPT_GRACE` if the
  process still runs, and the window's removal at `RETIRE_AFTER`. The manager's new
  `ending` flag makes a Codex exit after its turn end the window too, once the engine
  has retired or killed it (a Codex process between turns otherwise keeps `Idle`);
  `record_turn` clears it.
- **Translation.** One to one for `Init`, `TurnStarted`, `ToolUse`, `TurnEnded`,
  `ApiRetry`, `PermissionDenied`, `ProcessStarted`, `ProcessExited` (pid from
  `WindowSignal.pid`) and the sub-agent hooks. `Unprompted(TurnEnded)` is `Spend`, any
  other `Unprompted` activity. Text, tool results, compaction and `Other` are
  `Activity`, at most one per window per second. `StderrLine`, `Unknown` and
  `Diagnostic` are not activity (the M8a.17 carry's open question): Claude prints hook
  progress constantly, and a session that only writes to stderr is not progressing.
- **Start.** Parse, preflight, the protected files by the resolved profile's
  `ProtectedMatcher` (before `build_run`'s warnings), the id (5 draws against
  `refs/heads/anthrex/` via `run_id_taken`, `<data_dir>/runs/<id>` and the engine's
  runs), `build_run` (errors joined by `\n`), the nonce, decision 50's `api_key` check,
  then decision 53. The protected warning reaches `TaskInfo.notes`; printing it on
  stderr is `anthrex run start`'s (M8a.23).
- **Decision 53 counts worker routes only** ("a run with at least one Claude task"),
  as the brief's tests require (a Codex-only plan starts in a repository with Claude
  hooks, a Claude-only plan is not refused for Codex config). A task's reviewer runs on
  the other runtime, so under `CLI_CAPS` as recorded (Codex loads project config and
  cannot be told not to) a Claude task's Codex reviewer would load a tracked
  `.codex/config.toml` unasked. Recorded as a follow-up; the fix is to count review
  routes too and change the two test expectations.
- **Finish.** `Discard` needs `confirm == <id>`, else `ConfirmNeeded` with
  `discard run <id>: remove its worktrees and delete its branches?` (invented). Accept
  reads `refs/heads/<base>`: equal needs `<id>`; advanced needs `<id>@<to>` and sends
  `BaseAdvanced` before `Finish`, else `ConfirmNeeded` with `base_moved` from
  `commits_since`; rewritten is refused with decision 20's text. The accept prompt is
  M8a.23's (`merge anthrex/<id>/integration into <base> in <root>?`).
- **Restore.** `load_all`, `reconcile` per run on `spawn_blocking` (its notes go to the
  run's log as `restore: <note>`), `Event::Restore` stepped before any other event and
  its effects executed, then every journal compacted, the integration and live task
  worktrees of unfinished runs watched, restored headless windows of unknown or
  finished runs removed, and a replayed `Accept` `Finished` gets its clean-up re-run.
  The report of such a run still lists the branches reconcile's `Finished` named as
  kept, though the clean-up then deletes them.
- **Server.** `serve(listener, manager, git: GitWiring, runs, shutdown)`;
  `server/run_api.rs` answers each request on its own task; `Subscribe` sends the
  current snapshot, then every push with a higher revision, until `Unsubscribe` or the
  connection ends. `register_restored_roots` skips headless windows: the engine owns
  their roots (decision 22). `handle_client` drops its `RunApi` (which holds an
  `out_tx` clone) before awaiting the writer; without that a daemon's shutdown left
  every client connected (`reconnect_tests::drives_a_real_reconnect_twice` and
  `tui/tests/connection.rs` caught it). `server.rs` is 570 lines.
- **Lock check (acceptance).** Every `crate::lock(` in `run/driver*.rs` is a block or a
  single statement whose guard is dropped before the next `.await`, `spawn_blocking`,
  git call or manager call; the list is in the task report. The journal mutex is held
  only inside `spawn_blocking` closures, around the journal file I/O.
- **Carries.**
  - T13-P1 (the pid whose turn closed): **done**, `AgentRound.closed_pid`; test
    `the_exit_of_the_process_whose_turn_closed_is_normal`. A `Delivered` can still
    reach the engine before the next Codex process's `ProcessStarted`; the field makes
    that order harmless.
  - `Unprompted` to activity, never a turn end: **done**, as `Spend`/`Activity`.
  - Sandbox-unavailable `TurnEnded` before `ProcessExited`: **already done** in
    `headless/session/pipes.rs` (M8a.17); forwarded in order.
  - `resolution_only` in `DoneChecked` (an error counts as `Some(false)`): **done**.
  - Restore's replay from the journal, `Restore` first, notes in the log: **done**.
  - Setup after `PrepareWorktree` (and `CreateRunBranch`): **done**; `Discard` after a
    pre-warm: **done** (the per-run op lock).
  - `run_id_taken` with `for-each-ref refs/heads/anthrex/`: **done**.
  - Accept never under a shorter timeout; kept branches passed through: **done**.
  - `server.rs` at most 605 lines: **done** (570).
  - Restored headless windows of finished runs removed; the registry for headless
    windows: **done** (the engine watches run worktrees; the server skips headless
    windows).
  - A `ResumeFailed` for a round the engine already stopped is ignored: **already
    true** in the reducer; pinned by `a_resume_failure_for_a_killed_round_is_ignored`.
  - `AbortMerge`, `Proof` (`Handle::block_on` inside `spawn_blocking`), the scratch
    `Check`: **done**.
  - `protected_files` with the resolved profile's matcher before the warnings: **done**.
  - A kill during a Codex send's jitter leaves the window `Exited` (T18-N3): **done**;
    tests in `tests/headless_turns/ending.rs`.
  - Send errors, a NUL byte included, are `Delivered { ok: false, error }`: **done**.
  - Run-tool replies worded for an agent: the driver adds no text of its own to a tool
    answer; the engine's `Ok`/`Err` text is passed as `ToolResult`. It never refuses a
    tool call with a labelled `Error`.
  - Never compact between `run.json` and the intent lines: **done** (compaction on the
    tick). Accept clean-up after a replayed `Finished`: **done**. Per-run session
    nonce: **done** (`Run.session_nonce`, test
    `the_session_nonce_reaches_the_session_uuid`; a nonce of 0, a run from before, keeps
    its old uuids).
  - **Re-carried: the Codex first-turn marker.** Adding one changes Codex's argv and
    reconcile's session check, which this task's tests do not cover; still in the
    followups file.
  - **Re-carried: T8-RR2** (the `<run_head>...HEAD` range after `resume --rebaseline`).
    The driver passes the op's `start` and `run_head` through and computes no range, so
    the question stays with `git::diff_so_far` (final review).
  - **Re-carried to M8a.23: the report's `codex project config:` line** (decision 53's
    three texts). The report renders from `Run` alone and the run does not record the
    caps it started under; the CLI brief or the final review should decide whether
    `Run` records them.
  - Counter-only persists at most every 5 s, compaction past 1 MiB: **done**.
  - Codex stdin closed at spawn: unchanged.
- **Tests.** Red first: all 15 e2e tests failed on the stub's `runs are not available
  yet`; `the_exit_of_the_process_whose_turn_closed_is_normal` (the round died),
  `an_unprompted_turns_usage_is_spend_not_a_turn_end` (usage 0),
  `the_session_nonce_reaches_the_session_uuid` (equal uuids),
  `a_kill_during_a_codex_send_jitter_leaves_the_window_exited` and
  `a_retired_codex_window_between_turns_is_exited` (`Idle`, not `Exited`) failed before
  their changes. Pinning tests, green from the start:
  `a_resume_failure_for_a_killed_round_is_ignored`,
  `a_retired_claude_window_exits_on_eof`. Unit tests: `observe.rs` (4),
  `test_overrides_follow_decision_53`, `the_salvage_message_names_the_task`.

#### M8a.22 fix round 1 (2026-09-24)

Review `task-22-review.md`; rulings T22-C1, T22-I1 and T22-minors.

- **I1: decisions 53 and 50 cover every runtime a run launches.** `run start` now asks
  whether a run launches a Claude (or Codex) session at all (`driver/requests.rs`,
  `launches`), not only whether a worker runs it. Per task it counts:
  - the worker's route;
  - the reviewer's route (`review_route`, which `roster::pick_reviewer` puts on the peer
    runtime);
  - the rung-2 route that decision 39's `escalate` can move the worker to, and that
    route's reviewer.

  *(Corrected in fix round 2: this set was not complete. `run retry` escalates an
  already escalated route again, rung 3 gives an unreviewed task a reviewer, and a
  plan edit can add a task on either runtime; see "M8a.22 fix round 2".)* So a
  Codex-worker plan in a repository with Claude hooks is refused without
  `--trust-project`. Under the recorded `CLI_CAPS`, a Claude-worker plan in a repository
  that tracks `.codex/config.toml` is refused too (its reviewer is Codex).
  - `e2e_project_settings_are_refused_without_trust_project` and
    `e2e_codex_project_config_follows_cli_caps` now expect those refusals.
  - New tests: `e2e_a_codex_workers_claude_reviewer_needs_trust_project` and
    `e2e_api_key_auth_covers_a_claude_reviewer`.
  - The harness removes `ANTHROPIC_API_KEY` from the daemon's environment.
  - This supersedes the round's "decision 53 counts worker routes only" note, and its
    follow-up is closed.
- **Decision 20 is pinned.** `e2e_accept_onto_a_moved_base_needs_the_listed_head`
  (`run_e2e_finish.rs`) tries five confirmations on a moved base: none, the plain id, a
  wrong sha, the short sha, and another word. Each gets `ConfirmNeeded` with the listed
  commit, and the base is untouched. `<id>@<to>` then merges.
- **m1.** A run request's task holds a weak sender for the connection
  (`server/run_api.rs`). The request still runs to its end, but it no longer holds a
  closed connection's writer open. Test: `a_disconnected_client_is_not_held_open_by_its_run_request`
  (`crates/daemon/tests/server_runs.rs`).
- **m2 (deviation, recorded).** `Deliver` runs one task per effect, not one task per
  window as the brief says. Ordering argument:
  - The reducer never has two deliveries in flight to one window. `outbox::deliver`
    requires a closed turn, and a `Deliver` opens it (`turn_open`) in the same step.
  - The next `Deliver` to that window can come only after that turn's `TurnEnded`,
    which comes from the process the first delivery started. So the first send has
    returned by then.
  - The manager also refuses a second send in flight (`busy`, "a turn is already
    running").

  So a per-window queue would never hold more than one item.
- **m3.** A merge candidate puts the integration worktree back on the run branch on
  every way out once the candidate is materialized, errors included
  (`driver/merge.rs`). Test: `e2e_a_failed_merge_candidate_reattaches_the_integration_worktree`.
  In that test, the check (run in the integration worktree) leaves the base ref
  pointing at a missing object. The second guard then errors, the task is blocked with
  `could not merge`, and the worktree is on `anthrex/<id>/integration`.
- **m4.** `docs/timing-budgets.md` has rows for `RUN_WAIT` (with the `k` rule),
  `REQUEST_WAIT`, and the new `FINISH_WAIT`.
  - `FINISH_WAIT` is `600 s + RUN_WAIT`. `Finish` had used the 60 s `REQUEST_WAIT`,
    which is below an accept's legal worst case.
  - The table also covers the two new unit and integration bounds of this round.
- **m5.** A replayed accept's clean-up runs during the restore, before `Restore` is
  stepped. Its outcome and the branches it really kept replace reconcile's placeholder
  in the replayed `Finished`: `accepted as <sha7>; clean-up did not run before the
  restart; clean-up after the restart: <outcome>`. So the run's log and report say what
  happened. This supersedes the round's "the report still lists the branches as kept".
  Test: `e2e_a_replayed_accept_reports_its_clean_up`. It crashes the daemon after the
  `Accept` intent (decision 48), makes the merge by hand, then restarts.
- **m6.** The per-window records are pruned:
  - `observe`'s activity map keeps only windows active in the last second.
  - `Book.killed` loses a window's entry with the exit it marks, or when the window is
    removed. The retire removal also clears it.
  - Tests: `the_killed_record_goes_with_its_exit_or_its_window`, and the extended
    activity test.
- **m7.** When reconcile panics, the run's log says so
  (`restore: reconcile failed; the run's unfinished ops were not checked`), and an
  unfinished run is halted retryably, so `run resume` re-issues its work. Unit test:
  `an_unreconciled_run_is_held_and_says_so`. Reconcile cannot be made to panic from a
  test, so the helper is tested directly.
- **m8.** When `stop()` gets no acknowledgement within `stop_wait_ms` (30 s), it aborts
  the event loop and waits for the loop's gate (held by the loop for its whole life)
  before its own `stop_now`. Test: `a_stop_that_times_out_waits_for_the_loop_to_go`.
  - What remains: a blocking write the loop had already handed to a thread cannot be
    cancelled.
- **Carry to M8a.25: decision 43's ordering mutants.** M8a.25's crash tests must kill
  two mutants that survive every committed test today:
  - `OpDone` sent before the `done` line is appended;
  - every `Persist` made lazy.

  A graceful stop saves every run, so only a crash can show either.

#### M8a.22 fix round 2 (2026-09-24)

Re-review `task-22-rereview-1.md`; rulings T22-I1b, T22-N2, T22-N3 and T22-N4.

- **I1b: decisions 50 and 53 check every runtime a run can reach.** One pure function,
  `run::reach::reachable_runtimes`, gives the set. Per task:
  - the worker's route, and every route `escalate` reaches from it, applied again and
    again to a fixpoint (rung 2 and each `run retry` escalate once more);
  - for each of those routes, its reviewer (`pick_reviewer`) at every review level
    the task can have: its own, and the level rung 3's re-resolution gives it at each
    larger size, so an unreviewed `S` task counts the reviewer it gets once raised;
  - the task's current reviewer route.

  The roster's fallbacks (the peer runtime, else the same one) are those of
  `escalate` and `pick_reviewer` themselves.
  - `run start` checks the set.
  - `run edit` checks each runtime the run cannot reach yet
    (`driver/requests.rs`, `edit`), and passes each failing check's text to the
    engine. The engine (`engine/requests.rs`, `edit`) refuses an edit whose edited run
    reaches such a runtime, with that text, and applies nothing. Project settings
    already trusted at `run start` pass.
  - Unit tests (`reach_tests.rs`):
    - probe A;
    - repeated escalation alone;
    - the escalated route alone (mutant M-esc is killed here and by the
      repeated-escalation test);
    - rung 3's reviewer alone;
    - one-runtime rosters;
    - a peer entry no rung reaches.
  - Probe B is an engine test (`control_retry.rs`): a retried task lands on Codex, and
    Codex was in the set at the start.
  - The engine refusal is tested in `dispatch_edits.rs`.
  - E2E tests:
    - `e2e_a_claude_bound_plan_starts_and_an_edit_onto_codex_is_refused`;
    - `e2e_a_codex_bound_plan_starts_beside_claude_settings`.

    These are the brief's Claude-only and Codex-only cases. They hold only on rosters
    that keep the run on one runtime: with the built-in roster, every reviewed task
    reaches both runtimes.
- **N2 and the m8 residual.**
  - `stop()` returns early only once every run's last `run.json` has been written (a
    new `saved` flag, set after `stop_now`'s saves). So a loop aborted inside its own
    `stop_now` no longer leaves runs unsaved.
  - Every `run.json` write goes through `effects::RunWrites`. Writes are serialised per
    run, and each is numbered when requested, so a write a stuck or aborted loop left
    on a blocking thread can neither share `RUN_TMP` nor rename an older state over a
    newer one.
  - Tests: `an_older_run_json_write_never_lands_over_a_newer_one` and
    `a_stop_whose_loop_is_stuck_saving_still_saves_every_run`.
- **N3.** A replayed accept's clean-up no longer runs inside `restore()`, before the
  socket is bound.
  - Its op is held pending through `Restore` (the event's new `held` list).
  - `spawn` starts its clean-up on a task of its own. The task holds the run's ops
    exclusively and journals, then steps, the `Finished` with the clean-up's outcome.
  - A crash during the clean-up leaves the intent pending, so the next restart replays
    it again.
  - `e2e_a_replayed_accept_reports_its_clean_up` holds the first `worktree remove` for
    3 s and checks that the restarted daemon answers while the run is still `complete`.
- **N4.** That test's `until` reads a fresh snapshot on every poll.

### M8a.23 The `anthrex run` commands (2026-09-24)

- **Files.** As the brief lists, plus `crates/cli/src/run_cmd/status_tests.rs` (the
  status tests, split off so `status.rs` stays short). `main.rs` gains 4 lines (the
  `mod`, the `Run` variant and its dispatch); `crates/cli/Cargo.toml` gains `toml` for
  `run edit --file`. The harness gains `RunHarness::anthrex_input` (stdin for the
  prompts, bounded by `FINISH_WAIT`).
- **The status example's `2/4 merged` (deviation, recorded).** The Interfaces example's
  header says `2/4 merged` while its table shows one merged task (t1). The header's count
  is the table's (`TaskState::Merged`), so `status_text_matches_the_layout` renders the
  example exactly with `1/4` in the header, and checks that a second merged task makes
  it `2/4`.
- **Choices the CLI section leaves open.**
  - `<run>` is the full id, or a prefix or suffix matching exactly one run (an exact id
    wins); otherwise `no run matches '<run>'` or `'<run>' matches more than one run: …`.
  - A halted run's `halted: <reason>` line comes right after its header line.
  - The run-start warning is each task note carrying `(rule 6.protected)`, as
    `warning: <task>: <note>`, before the approve or watch hint. Other notes (raises) are
    not printed; the plan table does not show notes.
  - Prompts go to stderr and answers are read from stdin; end of input is "no".
  - `run accept` asks decision 20's first question (`merge anthrex/<id>/integration into
    <base> in <root>? [y/N]`, the daemon's `ConfirmNeeded` prompt) unless `--yes`, then,
    for a moved base, lists it and asks its own question unless `--base` equals the
    listed head. A different `--base` exits 1 with `--base <sha> is not the listed head
    <to>; not merged`; a "no" exits 1 with `not merged`. A base that moves again between
    the answer and the resend is listed and asked again.
  - The listing's `… and <n> more` line is indented two spaces like the commits, and
    counts `total − listed` (the client caps at 50 as well).
  - `run reject` asks `reject run <id>: remove its worktrees and delete its branches?`
    (invented; the daemon asks nothing for a reject). `run discard` prints the daemon's
    own `ConfirmNeeded` prompt. Both then read `type the run id to confirm: `.
  - `Done` messages print on stdout; every `Refused`, a wrong confirmation, a bad
    `--file` and an unknown run print on stderr and exit 1 (`std::process::exit(1)`, so
    no `Error:` prefix).
- **Ruling T23-C1: the report's `codex project config:` line.**
  `headless::argv::CodexProjectConfig { NotLoaded, Excluded, Loaded }` and
  `CliCaps::codex_project_config()`; `run start` records it on
  `Run.codex_project_config: Option<…>` (`#[serde(default)]`, so an older `run.json`
  loads with `None` and its report has no line). `REPORT.md` says `codex project
  config: not loaded by this CLI` or `codex project config: excluded`, decision 53's
  two texts. **Deviation:** decision 53 words no line for its third branch (Codex loads
  project config and cannot be told not to; the trust line covers the files), so the
  report says `codex project config: loaded by this CLI` (invented). It is the branch
  the real `CLI_CAPS` names. `model.rs` is 603 lines.
- **Tests.** Unit: `resolve_run_by_id_suffix_and_prefix`, `status_text_matches_the_layout`,
  `paused_and_halted_lines`, `base_moved_prompt_lists_the_commits`,
  `bounces_column_text`; `containment_reports_the_codex_project_config_branch` (report,
  old `run.json` included); `test_overrides_follow_decision_53` extended. E2E
  (`run_cli.rs`): the brief's seven, plus `help_lists_the_run_commands_and_hides_mcp`,
  `start_prints_the_protected_warning` (the M8a.22 carry),
  `accept_onto_a_moved_base_lists_it_and_needs_its_own_yes` and
  `accept_with_the_listed_base_merges`; `e2e_codex_project_config_follows_cli_caps`
  checks the report's line in each branch. `edit_from_a_file` cancels `t2` of a plan
  awaiting approval, where tasks are `queued`, not `pending`.

#### M8a.23 fix round 1 (2026-09-24)

Review `task-23-review.md`; rulings T23-I1, T23-I2, T23-I3 and T23-minors.

- **I1.** `run accept` and `run discard` wait `FINISH_REQUEST_TIMEOUT` =
  `daemon::run::git::ACCEPT_MERGE_TIMEOUT` + 60 s (660 s), built from the daemon's
  constant; every other run request keeps `RUN_REQUEST_TIMEOUT` (180 s). Test
  `finish_requests_outwait_the_accept_merge`; row in `docs/timing-budgets.md`.
- **I2.** `--base` takes the listed head or any hex prefix of it of at least 7
  characters (case-insensitive); the resend carries the full sha. Unit test
  `base_accepts_the_listed_head_or_a_prefix_of_it`; `accept_with_the_listed_base_merges`
  passes the 7-character form.
- **I3.** `reject <suffix> --confirm <id>` and `discard <suffix> --confirm <id>` succeed
  in `reject_needs_the_id` and `discard_keeps_salvage_refs` (the typed-id success is
  kept for reject, whose test now starts two runs). Mutant "every `--confirm` fails"
  is killed by both.
- **Minors.**
  - M1: `start_approve_status_accept` checks `run approve`'s stdout
    (`run <id> approved`); `reject_needs_the_id` runs `status <id> --json` with two runs
    and gets one. Both mutants (Done on stderr, no filter) are killed.
  - M2: `run accept` of a run that is not `complete` is refused before any question,
    in the daemon's wording (`run <id> is <state>; accept applies only to a complete
    run`), read from the snapshot the command resolved the run with.
  - M3: a moved base is listed before any question; a `--base` that names another head
    is refused right after the listing.
  - M4: a status cell as wide as its column or wider ends in one space
    (`long_cells_keep_a_space`); cells that fit keep the spec's columns exactly.
  - M5: when stdin is not a terminal, `stdin is not a terminal; pass --yes or --confirm`
    is printed once, before the first prompt; after end of input, or any answer read
    from a non-terminal, a newline ends the prompt's line, so the error is on its own
    line.
  - M6: the third branch's report line is now `codex project config: loaded (this Codex
    CLI cannot exclude it)` (still invented; supersedes `loaded by this CLI` above).
  - M7: the accept loop lists a moving base at most 5 times, then exits 1 with
    `<base> kept moving; not merged; run accept again`; the heading says `1 commit` in
    the singular; `model.rs` is back to 600 lines (three doc comments shortened).
- **Files.** The accept flow and the prompts moved to `crates/cli/src/run_cmd/finish.rs`
  (147 lines); `run_cmd.rs` is 465.
