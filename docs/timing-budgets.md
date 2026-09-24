# Timing budgets: what wall-clock test bounds actually cost

Distilled from a flake investigation run on milestone 6 (`m6-persistence`), which spent
roughly an hour measuring every wall-clock assertion in the workspace plus a contaminated
round that had to be redone. The investigation log itself lived under `.superpowers/`
(gitignored scratch, deleted once this branch's review passes) — this document is what a
future implementer needs from it, not the narrative.

Read this before writing a new `assert!(elapsed < ...)`, `sleep`-then-assert, or
deadline-poll test. The workspace has three dozen of these; most are fine, and the ones
that were not shared one defect shape.

## The defect shape: a bound at or below the code's own legal worst case

This is a defect independent of machine speed — it needs no slow CI runner to be wrong,
only for the code under test to use the budget it is legitimately entitled to. It is also
independent of language: the shape is "a bound at or below a constant it depends on,"
and that relationship does not stop existing at a process boundary. Six sites have had
this shape so far; five are fixed, one is intentional and correct as written. Every
sweep for this class before the final gate enumerated a narrower rule than "every wait
whose bound must exceed a daemon budget" — a specific helper's shape, a specific
keyword argument — and each narrower rule missed at least one site a later, by-behaviour
sweep found sitting in the same file the previous sweep had just edited.

| Test | Site | Bound (as found) | The code's own legal worst case | Status |
|---|---|---|---|---|
| `a_real_write_triggers_a_probe`, `a_commit_in_a_linked_worktree_triggers_a_probe` | `daemon/tests/git_registry.rs` (`wait_for_state`) | `secs(5)` | `PROBE_TIMEOUT` (5s) + `DEBOUNCE` (300ms) + watcher latency, on top | **Fixed** (earlier in this branch, before this work): the deadline is now `PROBE_TIMEOUT + WALL_CLOCK_SLACK` (write) and `PROBE_TIMEOUT + DEBOUNCE + WALL_CLOCK_SLACK` (linked-worktree commit) — derived from the same constants the code runs against, not a literal that can coincide with them again. |
| `without_a_daemon_it_exits_zero_silently_and_fast` | `cli/tests/hook_command.rs:22` (constant now `LIMIT`) | `< 1s` | `HOOK_DEADLINE` (1s), plus a real process spawn on top | **Fixed** (this work): widened to `LIMIT` (3s). `HOOK_DEADLINE` cannot be imported into the test (`anthrex` is a binary-only crate, no `lib.rs`), so the coupling is a comment, not a compiler-enforced constant — see "standing rules" below for why that is still the right shape of fix. |
| `stderr_is_bounded_and_does_not_block_the_child` | `daemon/tests/subprocess.rs:~101` | `< 10s` | injected timeout, `10s` | **Not a defect — left alone.** The bound *is* the injected timeout, and the assertion's whole content is "the child did not hit its own timeout." Zero margin here is the correct thing to say, not tightness to fix. |
| `run_cmd`'s calls wrapping `anthrex restart` and `anthrex daemon stop` | `scripts/pty-smoke.py:346` (`run_cmd`'s default `timeout`), used at 9 call sites (3 `restart`, 6 `daemon stop`) | `15s` (the default every one of those 9 call sites fell through to, unchanged) | `restart`: `HANDSHAKE_TIMEOUT` (5s) + `RESTART_REQUEST_TIMEOUT` (15s) = 20s. `daemon stop`: `HANDSHAKE_TIMEOUT` (5s) + `HUP_GRACE` (1s) + `KILL_GRACE` (3s) + `wait_released`'s own 10s cap = 19s | **Fixed** (whole-branch-review m16, fix wave 12): the third instance of this shape in the workspace, and the first found at a language boundary — every earlier sweep for it only ever read Rust constants, never the Python that wraps the binary they belong to. `restart`'s old bound was not merely below its own worst case, it was *numerically equal* to `RESTART_REQUEST_TIMEOUT` (15s), the exact coincidence standing rule 1 already named. Both call sites now pass an explicit `RESTART_CMD_TIMEOUT` / `DAEMON_STOP_CMD_TIMEOUT` (40s each), derived and commented the same way as the Rust-side constants they cannot literally import (same shape as `hook_command.rs`'s `LIMIT`, below — a comment-enforced coupling, not a compiler-enforced one, because these are two different binaries in two different languages). |
| `run_worktree_cli_stage`'s `anthrex new --worktree` (×2) and `anthrex rm --worktree` | `scripts/pty-smoke.py:548-551`, `:577-581`, `:585` (a bare `timeout=60` keyword argument at each site, not a call to a named helper) | `60s` at all three | `new --worktree`: `ENSURE_DAEMON_SOCKET_WAIT` (3s) + `HANDSHAKE_TIMEOUT` (5s) + `WORKTREE_REQUEST_TIMEOUT` (57s) = 65s. `rm --worktree`: `HANDSHAKE_TIMEOUT` (5s) + `WORKTREE_REQUEST_TIMEOUT` (57s) = 62s | **Fixed** (fix-wave-12-re-review Major 2): the fourth instance of this shape in the workspace, and the second found at the Rust/Python language boundary. The same fix wave that landed `RESTART_CMD_TIMEOUT` / `DAEMON_STOP_CMD_TIMEOUT` (the row above) added `HANDSHAKE_TIMEOUT` to those two paths in that very commit, but never re-checked these three worktree sites against the new total — its own sweep enumerated `run_cmd` call sites (the *construct*) rather than every wait whose bound must exceed a daemon budget (the *behaviour*), and these three sat as a bare `timeout=` keyword argument, not a `run_cmd`/`RESTART_CMD_TIMEOUT`-shaped call, so they matched neither grep. Fixed with a derived `WORKTREE_CMD_TIMEOUT` (90s) applied at all three sites; `run_cmd` was also given a `try`/`except subprocess.TimeoutExpired` so an exceeded bound reports through `fail()` and names the command, rather than killing the whole suite with a traceback that blames the script. |
| `run_worktree_form_stage`'s worktree-create wait, dirty-tree prompt wait, and forced-removal poll (the TUI half of the same feature, one function below the row above) | `scripts/pty-smoke.py`'s `wait_for_focused_window`/`wait_for` helper default (10s, ×3) and one hand-rolled `deadline = time.monotonic() + 10.0` | `10s` at all four sites | create wait: `DETECT_TIMEOUT` (5s) + `OPERATION_TIMEOUT` (30s) = 35s. Dirty-tree wait: `OPERATION_TIMEOUT` (30s). Forced-removal poll: `OPERATION_TIMEOUT` (30s) + `KILL_GRACE` (3s) = 33s | **Fixed** (final-gate finding F2a): the fifth instance, and the third at the language boundary. These waits drive the *same* daemon create/remove operations as the row above, just through the TUI's broadcast protocol, which has no client-side reply ceiling of its own to truncate the daemon's real budget — but they are `wait_for`/`wait_for_focused_window` calls and a bare `deadline =` loop, neither the `run_cmd` nor the `timeout=` shape either earlier sweep grepped for, so both missed them, including the sweep that produced the row above in the same file. Fixed with a shared `WORKTREE_FORM_TIMEOUT` (50s) covering all four sites. |
| `run_cmd`'s own default `timeout`, wrapping `anthrex new` at the three call sites that pass no explicit timeout | `scripts/pty-smoke.py`'s `run_cmd(args, expect_ok=True, timeout=15, ...)` | `15s` — **numerically equal** to `new`'s own worst case | `ensure_daemon` (`SPAWN_HANDOFF_GRACE` 0.25s + `ENSURE_DAEMON_SOCKET_WAIT` 3s = 3.25s) + `CliClient::connect` (`HANDSHAKE_TIMEOUT` 5s) + `request_with_timeout` (`CREATE_WINDOW_REPLY_TIMEOUT` 7s) = 15.25s | **Fixed** (final-gate finding F2b): the sixth instance — the exact "bound equals what it wraps" shape standing rule 1 exists to forbid, and sharper than a missed site: the fix wave that produced the two rows above **rewrote this default's own comment** and reaffirmed `new` as safe, reasoning from observed cost ("tens to low hundreds of ms") rather than from the legal worst case the code actually allows. Fixed by raising the default to 20s, with the 15.25s derivation now in the comment it replaces. |

### Open, from the codex-probe launch-gate fix (2026-09-22)

One row the fix that added `daemon::launch::LaunchGate` could not close itself, because
it lives in a file that change was asked not to touch:

| Test | Site | Bound (as found) | The code's own legal worst case | Status |
|---|---|---|---|---|
| `run_cmd`'s own default `timeout`, wrapping `anthrex new` at the three call sites that pass no explicit timeout | `scripts/pty-smoke.py`'s `run_cmd(args, expect_ok=True, timeout=20, ...)` | `20s` | `ensure_daemon` (3.25s) + `HANDSHAKE_TIMEOUT` (5s) + `CREATE_WINDOW_REPLY_TIMEOUT`, which the launch gate raised from 7s to **12s** (`LAUNCH_GATE_WAIT` 5 + `DETECT_TIMEOUT` 5 + `CREATE_REPLY_ALLOWANCE` 2) = **20.25s** | **Open.** The seventh instance of this shape, and the first produced by a change to a *daemon* constant rather than by a bound being chosen badly: `run_cmd`'s 20s was correctly derived against the old 15.25s worst case (finding F2b) and went negative when `WindowManager::create` gained a wait the CLI budget then had to cover. ~28s restores F2b's own ~38% margin. Unreachable in that script as written — its `ANTHREX_CODEX_BIN` is `fake-agent`, which answers `--version` in milliseconds, so the gate term is never actually paid — but standing rule 1 exists precisely to stop "observed cost" from being the argument, and F2b is the row where reasoning that way is recorded as the error. |

The same fix's own sweep found two production-side instances of the wider behaviour
("a client deadline that must outlast a server-side startup step") that are not test
bounds and so are recorded in
`docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` instead.

### Fixed, from the `server_restore_git.rs` flake (2026-09-22)

| Test | Site | Bound (as found) | The code's own legal worst case | Status |
|---|---|---|---|---|
| Every first-`Git` wait in `server_restore_git.rs` (four tests), plus its two post-change waits and its created-window control | `crates/daemon/tests/server_restore_git.rs` (`saw_git_within(.., secs(4))`, `secs(6)`, `secs(8)`, each also capped by `Client::recv()`'s own 5s panic) | `4s` / `6s` / `8s`, effectively `≤ 5s` | First `Git`: `PROBE_TIMEOUT` (5s), **plus the time to arm the root's watcher, which had no bound at all**, because `run_root` did not start the first probe until `watch::build` returned. After a change: `poll_secs` (30s) + 2 × `PROBE_TIMEOUT` once arming can be slow (see below). Created window: `DETECT_TIMEOUT` (5s) + `PROBE_TIMEOUT` | **Fixed**: the eighth instance, and the first where the missing term was not a constant at all. The flake was measured at about 1 run in 10, always on the **first run of a newly created executable**: notify's `watch()` (the FSEvents stream start) took 1.5–7.8s for every root in the process at once, while `git status` took ~25ms. The failure was the same in `/private/tmp` as under `~/Desktop`, so it is **not** the checkout-path effect recorded in the follow-ups. Fixed at the source: `run_root` now arms the watcher alongside the first probe and probes again once it is armed (`crates/daemon/tests/git_registry_arming.rs` gates an injected watcher to prove both). The test's waits are now derived from `PROBE_TIMEOUT`, `DETECT_TIMEOUT` and the configured `poll_secs`, and read frames directly instead of through `Client::recv()`. |

The lesson for the next sweep: an **unbounded step in front of a budget** makes that
budget meaningless, however carefully it was derived. `registration_probe_deadline` in
`git_registry.rs` was correctly `PROBE_TIMEOUT + slack`. It still measured the wrong
thing, because the probe it bounds could not start until an operation with no timeout
had finished. When deriving a bound, list every step between the trigger and the thing
awaited, not just the ones that have a named constant.

A related but distinct case, from `an_exited_window_is_removed_without_waiting`
(`daemon/tests/manager_worktree/removal/ordering.rs`): its old `< 1s` bound was not below
the code's legal worst case (the removal has no timeout of its own to butt up against),
but it shared a number with the real cost of the operation it measured — three real git
subprocesses that this investigation measured at 156–475ms depending on load (see below).
That is a **1.02–1.7x margin**, not a negative one, but it is thin enough that ordinary
contention flips it. Fixed by making the grace period injectable (`ManagerConfig::kill_grace`,
`crates/daemon/src/manager/mod.rs`) and setting it to 60s in the test with a `< 10s`
assertion — a 60x separation between "took the early return" and "waited out the grace",
immune to how fast git happens to run that day. See that file's doc comment for the full
reasoning.

### Recorded, from the run engine's end-to-end tests (M8a.22, 2026-09-24)

Every bound below is derived from the harness's own configuration
(`git_timeout_secs = 5`, `check_timeout_secs = 10`), not from observed cost.

| Test | Site | Bound | The code's own legal worst case | Status |
|---|---|---|---|---|
| Every `wait_run` in `crates/cli/tests/run_e2e_*.rs` | `RUN_WAIT` in `crates/cli/tests/support/run_harness.rs` | `300s` per task path | One task path (one session's work through every gate to its merge): at most 40 sequential engine git calls at `git_timeout_secs` (200s; `VerifyDone` takes the smaller of it and `DONE_CHECK_GIT_TIMEOUT`), at most 4 sequential check or proof runs at `check_timeout_secs` (40s), at most 20s of scripted `wait_ms` on the critical path: 260s. | **Recorded.** **The `k` rule:** a test waits `k * RUN_WAIT`, where `k` is the number of task paths its scenario runs one after another. A fresh session of the same task, or the part of a run after a daemon restart, is a path of its own. `k` is 1 unless the test names it (the milestone brief lists the M8a.24 tests with `k` of 2 or 3). `RUN_WAIT` has no timer term, so a test that waits out an engine timer on its critical path adds it by name: each `stall_after_secs`, each `rate_limit_retry_secs`, each failed interrupt's `INTERRUPT_GRACE`, and `RETIRE_AFTER` where it waits for a window to go (`e2e_green_s_task_runs_to_merged` waits `RETIRE_AFTER + 5s` for the reviewer window). |
| Every raw run request but `Finish` | `REQUEST_WAIT` in `run_harness.rs` | `60s` | `run start`: its preflight's six git calls plus the id draw's and the settings scan's three, at 5s each: 45s. Every other request is one engine step, whose inline effects are single fsynced file writes. | **Recorded.** |
| `Finish` (accept, discard) | `FINISH_WAIT` in `run_harness.rs` | `600s + RUN_WAIT` | The request's own reads (three git calls, 15s), then accept's merge under `ACCEPT_MERGE_TIMEOUT` (600s, which the driver never shortens), then its other git calls for a one-task run (at most 52 at 5s, 260s). | **Recorded.** Valid for one-task runs only, which is all the harness's tests accept. A test that accepts a larger run must derive its own. |
| `anthrex run accept` and `anthrex run discard` (the CLI's reply bound, not a test) | `FINISH_REQUEST_TIMEOUT` in `crates/cli/src/run_cmd.rs` | `ACCEPT_MERGE_TIMEOUT + 60s` (660s) | Accept's merge under the daemon's `ACCEPT_MERGE_TIMEOUT` (600s, never shortened: the user's signing runs inside it; their hooks no longer do, M8a final fix batch F1), plus the request's own git reads and the clean-up's first calls. Defined from the daemon's constant, not a copy; `finish_requests_outwait_the_accept_merge` pins it. Every other run request waits `RUN_REQUEST_TIMEOUT` (180s: `run start`'s preflight at the 60s default `git_timeout_secs`). | **Recorded** (ruling T23-I1). The harness's `FINISH_WAIT` (900s) exceeds it, so a test sees the CLI's own timeout before its own. |
| `a_disconnected_client_is_not_held_open_by_its_run_request` | `crates/daemon/tests/server_runs.rs` (`CLOSE_WITHIN`) | `5s` | The connection's teardown, which has no timeout of its own. The request's stand-in `git` sleeps `GIT_SLEEP_SECS` (20s). | **Recorded.** A separation test in the shape of `an_exited_window_is_removed_without_waiting`: 5s against a 20s sleep, a 4x margin between "closed at once" and "held by the request". |
| `a_stop_that_times_out_waits_for_the_loop_to_go` | `crates/daemon/src/run/driver.rs` (tests) | `stop_wait_ms = 200`; `1s` for the aborted loop; `10s` each for the first save and for `stop()` | The loop is held forever by the test's journal lock, so the 200ms acknowledgement wait always expires. Abort, then one save of one small run. | **Recorded.** The 1s bound applies after `stop()` has already awaited the loop's gate, so the task is finished or finishing. Before the fix it was stuck forever. |
| `a_stop_whose_loop_is_stuck_saving_still_saves_every_run` (M8a.22 fix round 2) | `crates/daemon/src/run/driver/tests.rs` | `stop_wait_ms = 200`; `10s` each for the first saves, the fallback's start, and `stop()`; `1s` for the aborted loop | Run `a`'s writes wait on the test's lock until the fallback has taken the loop's abort handle. After that, two saves of one small run each. | **Recorded.** Every wait is a deadline loop on a state the test observes. None is a separation. |
| `e2e_a_replayed_accept_reports_its_clean_up`, first snapshot after the restart (M8a.22 fix round 2) | `crates/cli/tests/run_e2e_finish.rs` (`CLEAN_UP_HELD`) | the stand-in `git` holds the first `worktree remove` for `3s` | One snapshot request (one engine lock, no git). `CLEAN_UP_HELD` stays under the harness's `git_timeout_secs = 5`, so the clean-up still succeeds. | **Recorded.** A separation test: one request of milliseconds against a 3s hold, which the clean-up cannot reach before the socket is bound unless the clean-up runs first. |
| `accept_whose_merge_landed_before_its_deadline_is_merged` and the rewritten `accept_merge_that_outlives_its_deadline_is_aborted` (M8a final fix batch F1) | `crates/daemon/tests/run_git_accept_safety.rs`, `run_git_accept.rs` | the merge's deadline `2s`; the stand-in git (or signing program) sleeps `30s` | No elapsed time is asserted: the 2s deadline is a parameter that must strike while the 30s sleep holds the merge, and a local git merge of one commit takes milliseconds. `e2e_a_failed_reattach_after_the_swap_still_merges_the_task` waits out the write queue's lock retries (6.2s) inside one `RUN_WAIT`. | **Recorded.** |
| `a_live_worker_racing_hand_backs_never_moves_the_base` (M8a final fix batch F1, fix round 5) | `crates/daemon/tests/run_git_live_race.rs` (`RACER_START`) | `30s` for the racer's first pass | One `/bin/sh` start-up and a handful of builtin `printf` writes: milliseconds. The racer has no timeout of its own; nothing else is waited for, and no elapsed time is asserted (the hand-backs that follow are bounded only by their own 30s git timeout, `T`). | **Recorded.** A deadline loop on a marker file the racer creates, polled every 5ms. |
| The run harness's daemon start (M8a.24 fix round 1) | `DAEMON_START_WAIT` in `crates/cli/tests/support/run_daemon.rs` | `60s` for the socket | Before binding, the daemon loads its state and config and restores its runs; a restored run's reconcile makes git calls at the harness's `git_timeout_secs = 5`, the class and count of `run start`'s preflight (at most nine, 45s). The fresh-executable watcher stall comes after binding. A daemon that exits early is seen at once, so only a hung start waits this out. It replaces the CLI's `ENSURE_DAEMON_SOCKET_WAIT` (3s), which a loaded machine outlasted. | **Recorded.** |
| The run harness's daemon exit (M8a.24 fix round 1) | `DAEMON_EXIT_WAIT` in `run_daemon.rs` | `30s`, then the owned child is killed | `anthrex daemon stop`'s own worst case (19s, the `run_cmd` row above). | **Recorded.** |
| `e2e_rate_limit_retry_is_not_a_stall_and_a_failed_turn_is_continued` (M8a.24), its `wait_run` | `crates/cli/tests/run_e2e_gates.rs` | `RUN_WAIT + rate_limit_retry_secs` (307s) | One task path (`RUN_WAIT`, whose 20s of scripted waits cover the script's 9s `wait_ms`), plus the one engine timer on its critical path: the failed turn's `rate_limit_retry_secs` (7s). The stall clock is never waited out: the test asserts it never fires. | **Recorded.** |
| The same test's "the continue came at least `rate_limit_retry_secs` after the failed turn" | `crates/cli/tests/run_e2e_gates.rs` | `>= 7s`, a **lower** bound | The engine's own promise (decision 32). Measured between two `perl` timestamps the fake agent takes inside its turns: before its retry event, and on reading the continue; the failed turn is the first plus the script's `wait_ms` (9s). Both stamps' own costs (a few ms of `sh` and `perl`) only add to the measured gap, so the bound can pass up to that much early; an engine that waits a whole second too little (the truncated-second bug this test found) or `stall_after_secs` (5s) instead fails it. | **Recorded.** A lower bound: load can only lengthen the gap. |
| The same test's window statuses (M8a.24 fix round 1, ruling T24-I2) | `crates/cli/tests/run_e2e_gates.rs` | the retry's `Attention` received before the failed turn; the status `3s` after the failed turn is `Attention` | The retry's `Attention` is pushed at the retry, 9s before the failure (a 9s margin). The failed turn's `Attention` holds until its continue, at least `rate_limit_retry_secs` (7s) later; the 3s sample leaves 3s for the push to arrive and 4s before the continue. | **Recorded.** |
| `api_retry_pauses_between_its_events_and_not_after_the_last` (M8a.24) | `crates/fake-agent/tests/headless_turns.rs` | `RUN` (20s) for the turn; `>= 400ms` for the step | The script's own delays: 400ms between two events, and a 60s delay after a last event that must not be paused. `RUN` separates "no pause" from "a 60s pause" by 3x; the 400ms is a lower bound, which load can only lengthen. | **Recorded.** |

### Recorded, from the end-to-end scenarios II and the run smoke stage (M8a.25, 2026-09-24)

| Test | Site | Bound | The code's own legal worst case | Status |
|---|---|---|---|---|
| `e2e_stall_escalates_to_a_fresh_session_on_the_peer_runtime`, its `wait_run` | `crates/cli/tests/run_e2e_merge.rs` | `2 * RUN_WAIT + 2 * stall_after_secs` (610s) | Two task paths (`k = 2`: session 1, then the fresh session 2), plus the two stalls waited out on the critical path (`stall_after_secs = 5` each). The interrupt ends `hang` at once (`fake-agent`), so no `INTERRUPT_GRACE` is spent. | **Recorded.** |
| The ladder and merge tests' other `wait_run`s | `run_e2e_merge.rs` | `k * RUN_WAIT` with the brief's `k`: 3 for `e2e_mis_sized_…` and `e2e_second_conflict_…`, 2 for `e2e_conflict_…` and `e2e_red_candidate_…` | The `RUN_WAIT` row's `k` rule. | **Recorded.** |
| The scripted gates (`wait_for_file`, `wait_for_merge_of`) | `run_e2e_merge.rs`, `run_e2e_safety.rs` (`sh` loops of 1500 × 0.2s) | `300s` (one `RUN_WAIT`) | The path the gate waits for is one task path at most (`t3`'s own, or the test's next step after a `wait_run` of at most one `RUN_WAIT`). Under `fake-agent`'s `SH_TIMEOUT` (330s). | **Recorded.** A gate, never a sleep: each loop exits as soon as its file or merge exists. |
| `e2e_crash_after_each_intent_kind_reconciles`: the wait for the crash | `crates/cli/tests/run_e2e_crash.rs` (`wait_dead`) | `RUN_WAIT` | The daemon crashes at the first intent of the kind, at most one task path in. | **Recorded.** |
| The same test's `wait_run` per kind | `run_e2e_crash.rs` | `2 * RUN_WAIT` | The part before the crash and the part after it are a path each (`k = 2`). | **Recorded.** |
| The same test's `ANTHREX_TEST_DELAY_DONE_MS` | `run_e2e_crash.rs` (`DELAY_DONE_MS`) | `300ms`, a **lower** bound on the hold | Not a wait: every op's `done` line (but a `CreateWindow`'s) is held this long, far over a `run.json` save plus an intent append (a few fsyncs, milliseconds), so an engine that acts on a result before its line is written reaches the next intent first. A loaded machine can only make the mutant's window wider, never a correct daemon fail. Adds about 0.3s per op to each path, inside `RUN_WAIT`'s 20s of scripted waits. | **Recorded.** |
| The same test's "the final `run.json`" | `run_e2e_crash.rs` | `30s` | A structural change (every pending op retired) is persisted at once; a counter-only one within `COUNTER_PERSIST_EVERY` (5s). | **Recorded.** |
| `e2e_headless_windows_refuse_client_control_while_the_engine_delivers`, each refusal | `crates/cli/tests/run_e2e_sessions.rs` | `10s` per `DaemonMsg::Error` | One server lookup under the manager lock (`server/headless_guard.rs`), no I/O. | **Recorded.** |
| `e2e_permission_denials_block_the_task_as_environment`, the worker window's exit | `run_e2e_sessions.rs` | `RUN_WAIT` | A kill of an owned group: `SIGHUP`, then `KILL_GRACE` (3s). | **Recorded.** |
| `e2e_a_session_that_exits_before_its_window_is_known_still_escalates` (fix round 1), its `wait_run` | `crates/cli/tests/run_e2e_sessions.rs` | `2 * RUN_WAIT + stall_after_secs + INTERRUPT_GRACE_SECS` (645s) | Two task paths (session 1, then the fresh session 2), plus the stall (`stall_after_secs = 10`) and the interrupt's grace (30s): the interrupt of a session with no process never ends its turn. | **Recorded.** |
| The same test's holds | `run_e2e_sessions.rs` (`WINDOW_HOLD_MS`, `AFTER_HOLD_MS`) | `2000ms` on every `CreateWindow`'s `done` line; `4000ms` of scripted wait before session 2's and the reviewer's tool calls | Not waits on the daemon: the hold only needs to outlast `fake-agent`'s start and exit (tens of ms); the later sessions wait twice the hold so their calls come after their windows are known, and under `stall_after_secs` (10s) so the wait is not a stall. Load can shrink the hold's margin over the start; a correct daemon passes either way (the race is then simply not reproduced). | **Recorded.** |
| Stage 11c's poll for `complete` | `scripts/pty_smoke_run.py` (`RUN_WAIT`) | `300s`, polled every 0.5s | One task path: the Rust `RUN_WAIT`'s derivation, coupled by comment across the language boundary. The smoke daemon runs the default `git_timeout_secs` (60s), so one path's legal worst case is larger than the harness's; the stage runs one quick green path, and a failure names the last state. | **Recorded.** A known gap, not a derivation: the smoke daemon's own config is the default one. |
| Stage 11c's `run start`, `run approve` and `run status` | `scripts/pty_smoke_run.py` (`RUN_CMD_TIMEOUT`) | `240s` | `ensure_daemon` (`SPAWN_HANDOFF_GRACE` 0.25s + `ENSURE_DAEMON_SOCKET_WAIT` 3s) + `HANDSHAKE_TIMEOUT` (5s) + `RUN_REQUEST_TIMEOUT` (180s) = 188.25s, above `run_cmd`'s default of 28s: the language-boundary shape of the `run_cmd` rows above, a seventh time. | **Recorded.** |
| Stage 11c's `run accept` | `scripts/pty_smoke_run.py` (`ACCEPT_CMD_TIMEOUT`) | `900s` | 3.25s + 5s + `FINISH_REQUEST_TIMEOUT` (660s) = 668.25s, above `RUN_CMD_TIMEOUT`. The brief said `RUN_CMD_TIMEOUT` for every run command; for accept that is below its legal worst case (deviation, recorded in the brief's notes). | **Recorded.** |

### Fixed, from the main-branch CI failures (2026-09-23)

| Test | Site | Bound (as found) | The code's own legal worst case | Status |
|---|---|---|---|---|
| `restart_resumes_claude_and_codex_sessions` | `crates/daemon/tests/lifecycle/restart_persistence.rs` (every `Client::recv()` after the first `Restart`, 5 s each) | `secs(5)` | `CODEX_PROBE_TIMEOUT` (5 s). The fixture `argv.sh` is also `runtimes.codex.command`, so `lifecycle::run` probed it with `--version`. It never exited, and the launch gate held the first restart for 4.90 s, measured. The test passed only because the probe's clock started a few milliseconds before `recv()`'s did. | **Fixed**: the fixture answers `--version` and exits, so nothing waits behind the probe (test time 5.14 s → 0.42 s). The test now asserts each restart is acked within `CODEX_PROBE_TIMEOUT / 2`. macOS CI run 35822907546 failed on it. |

The same rule applies to any fixture a test puts in `runtimes.codex.command` or
`ANTHREX_CODEX_BIN`: it has to answer `--version` promptly. `fake-agent` and
`pty-smoke.py`'s resume script already do.

## Measured primitive costs

Two independent measurement passes, both worth keeping because they were taken under
different conditions and neither alone tells the whole story.

**Loaded** — from the original investigation, at load average ~200 on a 14-core host (a
polluted-but-documented condition; see the investigation's own §0.1 for how that load got
there):

| Operation | min | median | max |
|---|---|---|---|
| `git status --porcelain` | 67 ms | 111 ms | 287 ms |
| `rev-parse` + `status` + `worktree remove` (the full removal sequence) | 156 ms | 180 ms | 222 ms |
| `/bin/sh -c 'echo x'` spawn | 31 ms | 56 ms | 90 ms |
| `anthrex` binary spawn | 33 ms | 39 ms | 85 ms |

**Idle** (labelled — measured for this work, single experiment on the host, load average
3–10 on 14 cores, zero `cargo`/`rustc`/`yes` processes, 30 samples per primitive unless
noted):

| Operation | min | median | mean | max |
|---|---|---|---|---|
| `git --no-optional-locks rev-parse --git-path ×7` (the paused-operation check) | 14.7 ms | 18.1 ms | 18.9 ms | 29.4 ms |
| `git --no-optional-locks status --porcelain --ignore-submodules=none` | 16.3 ms | 18.4 ms | 19.1 ms | 25.8 ms |
| `git --no-optional-locks worktree remove` (n=20) | 24.8 ms | 27.0 ms | 27.2 ms | 32.7 ms |
| `/bin/sh -c 'echo x'` spawn | 6.6 ms | 7.7 ms | 8.3 ms | 13.2 ms |
| `anthrex hook` spawn, no daemon listening (n=30) | 6.2 ms | 8.8 ms | 8.4 ms | 17.7 ms |
| `an_exited_window_is_removed_without_waiting`'s real removal path, isolated (own test binary, no siblings, n=30) | 186.8 ms | 197.8 ms | 206.5 ms | 263.6 ms |
| same, full `manager_worktree` binary running (21 tests, 14 threads, self-contention only, n=12) | 199.5 ms | 231.9 ms | 232.1 ms | 272.5 ms |
| same, plus 16 self-limiting `yes` spinners (load average climbed 10→21.5 during the run, n=12) | 288.0 ms | 343.3 ms | 354.0 ms | 475.3 ms |

**Arming a git watcher** (macOS, notify 8.2 FSEvents backend, measured 2026-09-22 inside
`server_restore_git.rs`): usually 28–68 ms per root. On the **first run of a newly
created executable** it sometimes takes **1.5–7.8 s**. The stall hits every root in the
process at once and sits inside `Watcher::watch()`. It showed up in 1/20, 2/25 and 4/25
of the fresh-copy batches measured, never in about 800 re-runs of an already-run binary,
and the same in `/private/tmp` as under `~/Desktop`. A fresh build is exactly what
`cargo test` runs first, so treat this step as unbounded and never let a budget wait
behind it.

A finding worth flagging for whoever next touches `crates/daemon/src/worktree/ops.rs`:
`worktree::remove` (called from `remove_with_worktree` step 4) re-runs `dirty_reason` —
i.e. the `rev-parse` + `status` pair — internally whenever `force` is false, even though
`remove_with_worktree` step 2 has already just run the identical check. So the "three
subprocesses" figure both measurement passes above use is actually five git invocations on
the non-force path (`rev-parse`, `status`, `rev-parse`, `status`, `worktree remove`), which
is consistent with the idle end-to-end cost (~190–270ms) being higher than the sum of the
individual primitive medians measured in isolation (~100ms) once `spawn_blocking`
dispatch, lock acquisitions and scheduling are added on top. This looks like intentional
time-of-check-to-time-of-use safety (the tree could go dirty between steps 2 and 4) rather
than a bug, so it is recorded here rather than changed — but it is real cost that the next
person writing a bound around this path should budget for.

## The idle-vs-urgent verdict for `an_exited_window_is_removed_without_waiting`

The investigation measured this test failing once (1/12, looped) at **1.017s against its
old 1s bound** — a 1.7% overshoot — and asked whether that meant the bound was merely
imprecise (hygiene) or genuinely about to fail on any slower CI runner (urgent). The idle
measurements above answer it: the real cost, even under **deliberately added contention**
(16 CPU spinners, load average over 20), tops out at 475ms — under half the old 1s bound,
and the idle number clusters around 200ms. **Verdict: merely wrong, not urgent.** The
1.017s outlier came from the test binary's own internal contention (this binary runs
`git worktree add` inside a stalled hook in three other tests, and every `worktree_agent`
helper spawns a real PTY), not from the underlying operation being slow in general. The fix
(injectable `kill_grace`, above) removes the shared-number coincidence regardless, so this
verdict is context for prioritisation, not a reason the fix was unnecessary — the old bound
would eventually have failed again given a slow enough CI runner, just not because it was
close to genuine danger today.

## Standing rules

1. **A bound must exceed the code's own legal worst case, derived from the constants the
   code actually runs against — never a literal that can coincide with them.** `secs(5)`
   next to a `PROBE_TIMEOUT` of `Duration::from_secs(5)` is not "tight," it is arithmetic
   equality with the thing it is supposed to bound. Write `PROBE_TIMEOUT + slack`, not the
   number you compute `PROBE_TIMEOUT` currently evaluates to.

   **This rule does not stop at a process or language boundary.** `scripts/pty-smoke.py`
   drives the real `anthrex` binary from Python, and its own `run_cmd` timeouts wrap the
   exact same CLI request/reply cycles the Rust-side constants above are named for —
   `RESTART_REQUEST_TIMEOUT`, `HANDSHAKE_TIMEOUT`, `KILL_GRACE` and the rest all still
   apply to how long that subprocess can legitimately take, whether the thing waiting on
   it is another `tokio::select!` or a Python `subprocess.run(timeout=...)`. A sweep that
   only greps Rust source for this defect shape will not find an instance that lives in a
   `.py` file; whole-branch-review m16 (`pty-smoke.py:346`) was missed by two prior sweeps
   for exactly that reason. When a Rust constant cannot be imported into the wrapper
   (different language, different binary), the coupling has to be a comment instead of a
   compiler-enforced reference — see the table's `hook_command.rs` row above, or
   `pty-smoke.py`'s own `RESTART_CMD_TIMEOUT`/`DAEMON_STOP_CMD_TIMEOUT` — but the rule
   itself, "derive it, don't pick it," is unchanged by which side of the boundary the
   bound lives on.

   **Sweep by behaviour, not by construct.** A sweep that greps for a named helper
   (`run_cmd`, `RESTART_CMD_TIMEOUT`-shaped call sites) finds every instance that
   happens to share that helper's shape and silently skips every instance that does the
   same thing a different way — a bare `subprocess.run(..., timeout=60)`, a plain
   `timeout=` keyword argument, a hand-rolled deadline loop. `run_worktree_cli_stage`'s
   three worktree-CLI sites (the table row above this one) are exactly that: the fix
   wave that added `HANDSHAKE_TIMEOUT` to `restart` and `daemon stop`'s bounds, in the
   very same commit, never re-derived these three because they were not `run_cmd` calls
   wrapping a named constant — they were a bare `timeout=60` at each site. The rule to
   sweep for is "every wait whose bound must exceed a daemon budget," not "every call to
   this particular wrapper" — the construct is an implementation detail of how a given
   site happens to be written today, and a future site can and will pick a different
   one.

2. **A test that fails on timing has a wrong assumption, not bad luck.** Every negative- or
   thin-margin site in this investigation had a comment or a name explaining what should
   happen; none had a comment justifying the specific number chosen. When a bound and a
   deadline it wraps turn out to be the same number, that is usually not a coincidence —
   trace where each one came from before treating a failure as noise.

3. **`#[ignore]` is deletion in this repo.** Nothing in `.github/workflows/ci.yml` or
   `scripts/` passes `--ignored` or `--include-ignored`, so an ignored test has not run in
   CI since it was marked and never will until a scheduler exists to run it deliberately.
   Recommending `#[ignore]` for a flaky test is recommending its deletion; say that
   explicitly rather than let the marker imply the test still runs somewhere.

4. **Check the instrument before believing the measurement.** Three separate measuring bugs
   produced confident wrong numbers during the investigation this document is built from: a
   package-name typo (`-p anthrex-cli` against a package actually named `anthrex`) that
   made cargo refuse to run and had the harness count every refusal as a test failure,
   producing an impossible 12/12 "failure" rate; a leaked set of load generators from an
   earlier phase that made an "unloaded" baseline anything but; and a measurement window
   that turned out to overlap a second agent's own before/after loops on the same host,
   silently corrupting both. A rate that looks too clean (100% or 0%) or a comparison that
   does not fall where the mechanism predicts is a reason to re-check the harness before
   trusting the numbers, not a reason to report them faster.

## What this document deliberately does not recommend

`crates/daemon/tests/lockfile.rs` isolates one test into its own binary to dodge a
*specific, proven* mechanism: a sibling's `fork()` transiently duplicating a just-closed
`flock` file descriptor. No other site in the investigation's inventory shares that
mechanism — everywhere else the interference is ordinary cost-vs-budget, not fd sharing.
**Do not generalise the lockfile isolation pattern to other flaky-looking tests.** It would
add build time and binaries without addressing the actual cause at those other sites.

Two categories of test were deliberately left untouched by this work and are tracked
separately in `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md` under
"From the M6 persistence flake investigation": the `App` 30ms debounce test (the tightest
bound in the repo, but never observed to fail, and fixing it means injecting a clock rather
than widening a number), and the four `100ms` lock-latency assertions in
`daemon/tests/manager.rs` and `manager_worktree/admission.rs` (load-bearing — they exist to
prove the manager lock is never held across a stalled git or PTY operation, so widening the
bound would silently delete the property rather than fix a flake).
