# Delivery hardening: design (milestone 9.7)

> Written 2026-10-05. The user chose scope B (the six `pr`-mode delivery followups plus four delivery-adjacent fixes), approach A (no protocol change), and approved sections 1 and 2 in conversation. Sections 3 and 4 follow the readings the user saw with the approach choice and were approved with "don't stop until everything is done". The code facts are read at `f072b0cd` (9.6's PR head, #34). This spec is "DH" below.

## 0. Goal and constraints

**Goal.** In `pr` mode, what anthrex says about a run's PRs matches what happened on the host, and no work is lost without a word. Four nearby defects that already bit or are about to bite are fixed alongside.

**Success.** Each item below has a test that fails before its change and passes after it, on the engine fixture or on the fake GitHub host (`PrRig` on `FakeHost`). The full suite, smoke, clippy and fmt pass.

**Constraints.**
- **No protocol change.** `PROTO_VERSION` stays 17. `HISTORY_VERSION` stays 5.
- **`run.json` and the journal stay readable both ways.** Every new field is `#[serde(default)]` and appended.
- **AGENTS.md rules 2, 10 and 11** hold for every new git call: it runs on the git queue, never under the manager lock, with `--no-optional-locks` and a scrubbed environment.
- **The engine stays pure.** The driver does the I/O.
- **anthrex never merges and never lands in `pr` mode.**
- Tests use `fake-agent`, `FakeHost` and temp dirs only.

**Out of scope.**
- Delivery statistics: a later pass, once real `pr` runs exist.
- A next goal while PRs are open.
- Revert mapping for squash and rebase merges.
- `RUN_REQUEST_TIMEOUT`.

They stay in the followups file.

## 1. Work lost or misreported after a merge (approved)

### 1.1 A merged stage's unfinished fix tasks

**Today.** `engine/delivery/land.rs::merged` touches no task. On the top stage, a fix task's later commits are never pushed and nothing says so.

**Design.**
- **When stage `n`'s PR merges and no live stage sits above it** (`(n+1..).find(sync::live)` is `None`), every unfinished task with `fixes` on stage `n` is cancelled through `complete::cancel_task`, with the reason `stage <n> PR merged`. This mirrors `closed`.
- **A stage with a live stage above it** keeps its fix tasks, because propagate carries their commits up.
- **A task whose merge is in flight** is deferred by `cancel_deferred`, as today. If it lands on a merged stage, the engine runs `unpushed` again for that stage, so the existing "not delivered" line appears. Nothing is dropped silently.

**Tests.**
- **Engine:** a merged top stage cancels its working fix task with the reason; a merged stage with a live stage above it keeps its fix task; a deferred fix task lands and the not-delivered line appears.
- **End to end (`PrRig`):** a top stage merges while its CI fix task works, and the task ends cancelled with the reason.

### 1.2 Two pushes merged before anthrex viewed the first

**Today.** `land::merged` judges delivery when the merged view arrives. It counts a merge as delivered only if the host's merged head is the local stage head or a head an open-PR view confirmed. Otherwise the stage reads `… merged at <at7>, without <head7>; that work is not delivered`, even when that work is in the merge.

**Design.**
- **When the merged head is neither of those heads,** the verdict is held: the stage records an undecided delivery.
- **The base fetch after the merge** (`sync::pass`, executed by `driver/host_ops.rs` on the git queue) gains two optional steps:
  1. Fetch the stage branch into `refs/anthrex/<run>/remote/stage-<n>`.
  2. Run `merge-base --is-ancestor <local stage head> <merged head>`, which is already allowed (`host/allow.rs`).
- **For a merge-commit merge,** the merged head is already local after the base fetch (`merge_commit^2`), so the branch fetch is skipped.
- **The answer,** `contains: Option<bool>`, is judged in `sync::fetched`:
  - `Some(true)`: delivered, with no line.
  - `Some(false)`: today's line, which is now true.
  - `None` (the branch is gone, or the fetch or check failed): today's line. anthrex never claims a delivery it could not verify.
- **A restart while undecided** runs the fetch again on restore.

**Tests.**
- **Engine:** each of the three answers; the restart.
- **End to end (`PrRig`):** two scripted pushes with `FakeGithubCtl::commit`, a merge before any view, and no not-delivered line.

## 2. What the orchestrator and the stage node show (approved)

### 2.1 Decided holds in the digest

**Today.** `orch/digest.rs::gate` shows every rejected and moot hold forever, and `digest_trim::trim` never touches `gate.holds`.

**Design.**
- **A new first trim step** drops decided holds (Approved, Rejected, Moot), oldest `decided_at` first. It never drops a Drafting or Awaiting hold.
- **The digest reports `omitted.holds`** beside `omitted.scouts`.
- **Nothing changes while the digest fits.**

**Tests.**
- 500 decided holds plus 2 awaiting fit within `DIGEST_MAX_BYTES`, keep both awaiting holds, and report the right count.
- The existing "a rejected hold stays after a read" test is unchanged.

### 2.2 A `run_status` read that hides a hold approval

**Today.** `driver/orch.rs::run_status` builds the digest from a clone taken at `now`, then sends `OrchEvent::DigestRead`. The engine records `digest_read_at` with its own later `now`. A hold approved between the two is in no answer and is hidden from every later read.

**Design.**
- `DigestRead` carries `at`, the clone's `now`, and `wake::digest_read` records it.
- `OrchEvent` is not serialized, so there is no format change.
- An approval in the clone's own second is shown once more, which is harmless.

**Tests.**
- An engine test approves a hold between the clone and the event, and the next read shows it. It fails today.
- `run_e2e_orch::promote` goes on the merge-gate stress list, 20 runs.

### 2.3 The tier-3 mark of an open PR's stage

**Today.** After a fix moves an open PR's head, `run/snapshot_stages.rs::full` reports `FullState::None` (◌). In `pr` mode tier 3 runs only before a PR opens.

**Design.**
- **For a `pr`-mode stage whose PR is open,** `full` reports the last tier-3 job's verdict, ✓ or ✗, instead of `None`. That job ran on the head the PR opened with, and CI carries later heads.
- **Local mode, and a `pr` stage before its PR opens,** keep the on-head rule.
- **The orchestrator's digest** reads the same `stage_infos`, so it changes the same way.
- **The client needs no change.** The stage inspector already shows the job's commit.

**Tests.**
- Snapshot tests: an open PR with a moved head reads ✓; before the PR opens it reads ◌; local mode reads ◌.
- Smoke 11i's stage 1 line expects `tier 3 ✓` where it pinned `tier 3 ◌`.

## 3. CI: the bisect and the summary's log

### 3.1 Widen the CI bisect to the stage's whole line

**Today.** `engine/bisect.rs::range` starts after `s.full.green_at` when that is on the stage's line. For a CI red, `engine/delivery/ci_repro.rs` returns early with `CI fails a test tier 3 passed on this head` when the red head is the green head. In `pr` mode, a CI red on the head a PR opened with is therefore never bisected.

**Design.**
- **For a CI red** (the `ci_repro` caller), the range ignores `green_at`. Its base is `s.floor`, else `s.created_from`, so the search covers the stage's whole line of merges.
- **The early return goes.** Its log line becomes the bisect's own start line.
- **Tier 3's own reds** keep today's range.
- **The usual assumption holds:** the base is green for the failing test. A base that also fails takes the bisect's existing "no culprit found" path.
- **The search is bounded** by the bisect's existing probe limits.

**Tests.**
- **Engine:** a CI red on the opened head bisects from the floor and names a culprit below `green_at`; a tier-3 red keeps its range.
- **End to end:** the existing `e2e_pr_ci_red_reproduced_bisects_fixes_pushes_and_propagates` still passes.

### 3.2 The CI summary's request no longer carries the log

**Today.** `CiSummaryInput.log` (up to 48 KiB) is serialized into the queued decider in `run.json`, the `Decide` op's journal intent, and the pending op, on top of `CiRecord.text`.

**Design.**
- **`log` is `#[serde(skip_serializing)]`** (it is already `#[serde(default)]`).
- **`deciders::dispatch` refills it** from the CI record whose `decider` is this request and whose phase is `Summarising`, just before it emits `Decide`. `CiRecord.text` is kept until the summary, so a restored or lost op re-queues with an empty log and is refilled the same way.
- **If no record holds text,** the decider runs with an empty log, which its prompt already handles.

**Tests.**
- The queued request, the journal intent and `run.json` hold no log text.
- The emitted `Decide` holds the full text.
- A restore re-queues and refills it.
- The existing A4 journal tests still pass.

## 4. Smaller fixes beside delivery

### 4.1 The CLI's unbounded git call

**Today.** `cli/src/run_cmd/delivery.rs::bare_under`, behind the hidden `anthrex fake-github create-repo`, runs `git … rev-parse` with `.output()` and no deadline.

**Design.**
- Run it through `daemon::subprocess::run` with the daemon's bound for a local git read, after `scrub_inherited_git` (which also removes the pathspec variables).
- A timeout prints `git did not answer within <n>s` and exits 1.

**Test.** A CLI test with a `git` stand-in that sleeps past the bound returns within it, with the message.

### 4.2 Fix tasks that ignore uninstalled and just-failed runtimes

**Today.** The bisect's fix task (`engine/bisect.rs::add_fix`) and the CI culprit's fix task (`engine/delivery/fix.rs`) call `roster::escalate` directly. Workers use `route_pick_step`'s `escalate_skipping` over `open_roster`.

**Design.**
- A `pub(crate)` helper, `escalate_for(run, culprit, current, Mover::Worker)`, applies both skips: the installed skip, and the culprit's failed routes.
- Both fix tasks route through it.
- **`reach.rs`'s forecast** takes the installed skip only, since it has no task to read failures from. This narrows the forecast to what can actually run.

**Tests.**
- With the escalated runtime not installed, both fix tasks route to the next installed one.
- With the culprit's route failed, they skip it.
- `reach` drops an uninstalled runtime.

### 4.3 Split `run/delivery/tests.rs` (601 lines)

**Change.** A move-only split. The shared fixture stays in `tests.rs`, and the four persistence tests move to `tests_persist.rs`.

**Check.** The test count is unchanged.

## 5. Testing, smoke and paperwork

- **Every item's tests come first** and fail before its change (AGENTS.md rule 6).
- **No new smoke stage.** 11i's expectation changes (§2.3), and 11i still drives `pr` delivery end to end.
- **The merge-gate stress list gains** `run_e2e_orch::promote` and the new `PrRig` tests.
- **ROADMAP:** 9.7 goes to `done` when finished; the protocol line stays at 17.
- **Followups:** each item is marked done with its task; anything new goes to the file.
