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
and that relationship does not stop existing at a process boundary. Four sites had this
shape; three are fixed, one is intentional and correct as written.

| Test | Site | Bound (as found) | The code's own legal worst case | Status |
|---|---|---|---|---|
| `a_real_write_triggers_a_probe`, `a_commit_in_a_linked_worktree_triggers_a_probe` | `daemon/tests/git_registry.rs` (`wait_for_state`) | `secs(5)` | `PROBE_TIMEOUT` (5s) + `DEBOUNCE` (300ms) + watcher latency, on top | **Fixed** (earlier in this branch, before this work): the deadline is now `PROBE_TIMEOUT + WALL_CLOCK_SLACK` (write) and `PROBE_TIMEOUT + DEBOUNCE + WALL_CLOCK_SLACK` (linked-worktree commit) — derived from the same constants the code runs against, not a literal that can coincide with them again. |
| `without_a_daemon_it_exits_zero_silently_and_fast` | `cli/tests/hook_command.rs:22` (constant now `LIMIT`) | `< 1s` | `HOOK_DEADLINE` (1s), plus a real process spawn on top | **Fixed** (this work): widened to `LIMIT` (3s). `HOOK_DEADLINE` cannot be imported into the test (`anthrex` is a binary-only crate, no `lib.rs`), so the coupling is a comment, not a compiler-enforced constant — see "standing rules" below for why that is still the right shape of fix. |
| `stderr_is_bounded_and_does_not_block_the_child` | `daemon/tests/subprocess.rs:~101` | `< 10s` | injected timeout, `10s` | **Not a defect — left alone.** The bound *is* the injected timeout, and the assertion's whole content is "the child did not hit its own timeout." Zero margin here is the correct thing to say, not tightness to fix. |
| `run_cmd`'s calls wrapping `anthrex restart` and `anthrex daemon stop` | `scripts/pty-smoke.py:346` (`run_cmd`'s default `timeout`), used at 9 call sites (3 `restart`, 6 `daemon stop`) | `15s` (the default every one of those 9 call sites fell through to, unchanged) | `restart`: `HANDSHAKE_TIMEOUT` (5s) + `RESTART_REQUEST_TIMEOUT` (15s) = 20s. `daemon stop`: `HANDSHAKE_TIMEOUT` (5s) + `HUP_GRACE` (1s) + `KILL_GRACE` (3s) + `wait_released`'s own 10s cap = 19s | **Fixed** (whole-branch-review m16, fix wave 12): the third instance of this shape in the workspace, and the first found at a language boundary — every earlier sweep for it only ever read Rust constants, never the Python that wraps the binary they belong to. `restart`'s old bound was not merely below its own worst case, it was *numerically equal* to `RESTART_REQUEST_TIMEOUT` (15s), the exact coincidence standing rule 1 already named. Both call sites now pass an explicit `RESTART_CMD_TIMEOUT` / `DAEMON_STOP_CMD_TIMEOUT` (40s each), derived and commented the same way as the Rust-side constants they cannot literally import (same shape as `hook_command.rs`'s `LIMIT`, below — a comment-enforced coupling, not a compiler-enforced one, because these are two different binaries in two different languages). |

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
