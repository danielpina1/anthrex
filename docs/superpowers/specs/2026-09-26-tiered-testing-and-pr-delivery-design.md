# anthrex design: tiered testing and stacked-PR delivery

Date: 2026-09-26. Status: **binding once merged to `main`** (requested by Daniel, 2026-09-26). Implemented by milestones **9.1** (tiered testing) and **9.2** (stacked-PR delivery), after milestone 9 and before 9.5. §12, orchestrator-to-worker messaging, is added to the scope of **milestone 9** itself.

Amends `2026-09-22-adaptive-orchestrator-design.md` (the adaptive spec). §11 below lists exactly what it replaces. Where the two disagree, this document wins.

## 1. What this is for

Two problems with the adaptive spec as written:

1. **Testing is the wall-clock bottleneck.** The profile has one `check`. It runs in every task worktree **and** on every merge candidate, and the merge queue is width 1. On this repository the full suite takes about 40 minutes, so a 6-task run spends about 4 hours in serial candidate checks and a 28-task run about 19 hours. When several agents test at once they also compete for the same cores. That makes every run slower and makes timing-sensitive tests fail, and those failures get blamed on correct work.
2. **Delivery is one local merge at the end.** `run accept` merges the whole run into the base branch in one `--no-ff` merge. The user reviews thousands of lines in a terminal, CI never sees the change before it lands, and nothing that happens after the run completes (a CI failure, a review comment) can reach the agents.

This design fixes both:

- **Tiered testing.** Each gate runs only what can be affected by the change. The full suite runs once per stage and in the background, and a red result is bisected to the task that caused it.
- **Stacked-PR delivery.** A run is delivered as a stack of small pull requests, one per stage. A red CI run or a review comment on a PR automatically becomes a fix task that goes through the normal gates. **anthrex never merges a pull request.** The user merges on GitHub.

### Non-goals

- anthrex merging, approving, or enabling auto-merge on any pull request. This includes the unapproved "M8d auto-merge / landing policy" proposal, which this document rejects.
- Webhooks or a hosted service. Everything is polled with the user's own `gh` login.
- Hosts other than GitHub. The design keeps the host behind one interface (§7.10), but only GitHub is implemented.
- Test impact analysis from coverage data. Selection here is by module graph; learning which tests catch which changes is M9.5 or later.

## 2. Terms

- **tier** — one level of testing, 0 to 3 (§3.2).
- **affected set** — the modules whose tests a change must run: modules with changed files plus every module that depends on them (§3.3).
- **full trigger** — a changed path that forces the full suite, such as a root manifest or a lock file.
- **slow test** — a test the profile marks as slow. It runs only in tier 3.
- **stage** — an ordered group of tasks delivered as one pull request (§6).
- **stage branch** — `anthrex/<run>/stage-<n>`, the integration branch for stage *n*.
- **stack** — the stage PRs of one run, each based on the previous stage's branch.
- **origin** — who created a task: `plan` (a planner), `ci` (a red CI run), `review` (a PR comment), `sync` (a base-branch update), or `bisect` (a red tier 3).

## 3. Tiered testing (milestone 9.1)

### 3.1 Profile additions

The repo profile (adaptive spec §6, stored by M8b) gains these keys. All are optional. **A profile that sets none of them behaves exactly as today**: every tier runs `check`.

```toml
# existing: check is now the FULL suite, run in tier 3 (and as the fallback for every tier)
check        = "cargo nextest run --workspace --profile full && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check"

# new
build_check  = "cargo build --workspace --all-targets && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check"
module_test  = "cargo nextest run -p {module} --profile fast"        # {module} = one module name; run once per affected module, or once with {modules}
module_tests = "cargo nextest run {modules:-p %} --profile fast"     # optional: one invocation for the whole affected set
module_graph = "cargo"             # built-in detector: "cargo" | "none"; or a command printing JSON {module: [deps...]}
module_names = "cargo"             # how a module glob maps to the name {module} expects: "cargo" (package name) | "dir"
full_triggers = ["Cargo.toml", "Cargo.lock", ".github/**", "rust-toolchain*", "**/build.rs", ".config/nextest.toml"]
slow_tests   = "test(/^e2e_/) | package(anthrex-cli) & binary(/run_e2e_/)"   # runner filter; empty = none
timing_tests = "test(/timing|elapsed|deadline/)"                     # runner filter for the exclusive timing group; empty = none
skip_markers = ["#[ignore", "#[cfg(any())]", ".skip(", "@pytest.mark.skip", "xit(", "t.Skip("]
test_paths   = ["**/tests/**", "**/*_test.*", "**/test_*.py"]        # files that are tests; inline Rust tests are covered by skip_markers
full_shards  = 4                   # tier 3 is split into this many shards when the runner supports it ({shard}/{shards})
```

- `module_graph = "cargo"` is computed by the **engine**, not by an agent: it runs `cargo metadata --format-version 1 --no-deps` on a blocking thread with a timeout (AGENTS.md rule 2), keeps only the workspace members' internal dependencies, and caches the result keyed by the hash of every `Cargo.toml` and `Cargo.lock`.
  - Other ecosystems use a command that prints `{"module": ["dep", ...]}`.
  - A graph that can't be computed (the command fails, times out, or prints invalid JSON) means **"unknown"**, and every tier falls back to `check`. The run report says so once.
- M8b's onboarding scout proposes these keys. The engine verifies each proposed command in a scratch worktree before proposing it, exactly as it does for `check`: `module_test` must pass for one real module, and `module_graph` must parse.
  - For Rust the scout proposes `cargo-nextest` when it's installed. It is never required.
- Changing any of these keys re-proposes the profile for the user's confirmation, like every other profile key. Profile changes stay human (adaptive spec §6).

### 3.2 The four tiers

| Tier | Runs when | Runs what | Who runs it |
|---|---|---|---|
| **0: worker loop** | whenever the worker wants, while coding | its own test (`single_test`) and its module's tests | the worker, told how in its prompt; never a gate |
| **1: task gate** | after `task_done`, replacing the adaptive spec's §11.3 check | test proof (unchanged, §8.1) → `build_check` → affected-set tests with `slow_tests` excluded | engine |
| **2: merge candidate** | each candidate in the merge queue, replacing §11.5 step 2's check | `build_check` + the affected-set tests of the merged tree, with `slow_tests` excluded; **skipped if the result cache already has this exact tree** (§3.5) | engine |
| **3: full suite** | (a) before a stage PR opens, (b) when the merge queue has been idle `full_idle_secs` (default 120) and the stage head has no green tier-3 result, (c) at run completion in local delivery mode | `check`, sharded into `full_shards` when possible | engine |

- **Tier 1's affected set** is computed from the task's own net change: `git diff --name-only <task start>...HEAD`.
- **Tier 2's affected set** is computed from the candidate's change against the stage head it merges into: `git diff --name-only <stage head> <candidate>`. Tier 1 of the same task is not reused, because the merge may pull in dependents that the task's own diff didn't touch.
- **Degradation.** Some profile keys may be missing:
  - No `build_check`: tiers 1 and 2 run `check`.
  - No `module_test` and no `module_tests`, or an unknown graph: the tier 1 and tier 2 test step runs `check`.
  - No `check`: the adaptive spec §6 degradation stands (reviews one size up, run marked unverified).
- **A full trigger, a hub file, or a changed path that matches no module** makes that tier run `check` instead of the affected-set tests. This is logged as `tier 1: full suite (full trigger: Cargo.lock)`.

### 3.3 The affected set

A pure function in the engine, with no I/O:

```
affected(changed_paths, profile, graph) -> Affected::Full(reason) | Affected::Modules(set)
```

1. If any path matches `full_triggers` or `hub` → `Full`.
2. Map each path to its module through the `modules` globs. A path matching no module → `Full` (`unowned path <p>`). Docs are the exception: a path that matches only files nothing executes (the profile's `source` globs don't match, and no `test_paths` match) is ignored. A docs-only change runs `build_check` and no tests.
3. If the graph is unknown → `Full`.
4. Take the closure over **reverse** dependencies: the changed modules plus every workspace module that depends on one of them, transitively.
5. If the closure contains every module → `Full` (`every module affected`), so the one full invocation is used instead of N per-module runs.

For anthrex, `crates/cli` depends on every crate, so almost every change includes `cli`. That is why `slow_tests` matters: the cli crate's e2e and PTY tests are marked slow and run only in tier 3. Without that, tier 1 would rarely be cheaper than tier 3.

### 3.4 The test scheduler

Every command the engine runs as a test tier goes through one scheduler in the daemon: `setup`, the test proof, `build_check`, module tests, and tier 3 shards.

- **Slots.** `test_slots` (default: logical cores − 2, minimum 1). Each run holds a number of slots while it runs, and gets its parallelism through the environment:
  - `CARGO_BUILD_JOBS=<slots>`;
  - `NEXTEST_TEST_THREADS=<slots>` and `RUST_TEST_THREADS=<slots>`;
  - `ANTHREX_TEST_SLOTS=<slots>` for runners that read neither.
- **Priority.** Tier 2 candidate first, then tier 1 gates, then tier 3 before a PR opens, then idle tier 3. Among equals: critical path first, then first come.
- **Sizing.** A job asks for `min(requested, free)`. Tier 2 requests all slots, tier 1 half, tier 3 all. A job never waits for slots that are only held by lower-priority work: tier 3 shards yield between shards.
- **Workers' own test runs** (tier 0) are not scheduled, because they happen inside the agent's sandbox. They are **capped** through the session environment the engine launches them with: `CARGO_BUILD_JOBS` and the thread variables set to `max(1, test_slots / max_writers)`.
- **The timing group.** Tests matching `timing_tests` run in their own invocation that holds **all** slots, so nothing else runs beside them. The rest of the tier runs with `timing_tests` excluded.
  - The scheduler also exports `ANTHREX_TEST_LOAD`: active slots divided by `test_slots`, one decimal place. Timing bounds may scale by it; `docs/timing-budgets.md` gains that rule in M9.1.
- **Isolation.** Every tier command gets, on top of the profile's `[env]`:
  - `TMPDIR=<worktree>/.anthrex-tmp` (created fresh, removed after);
  - `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under that `TMPDIR`, so a test that starts an anthrex daemon can never reach the user's (AGENTS.md hard rule 1);
  - `HOME` left unchanged unless the profile's `[env]` sets it.
- **Durability.** Scheduler state is not journaled. After a daemon restart, running test commands are gone and their ops are re-issued from the journal, exactly as M8a re-issues a `Check` op today.

### 3.5 The result cache

- **Key:** `(tree id, tier, command after substitution, affected set, profile hash, toolchain id)`.
  - The toolchain id is the output of `profile.toolchain_id` if set (for Rust: `rustc -Vv`), cached per run.
- **Value:** pass or fail, duration, flaky tests, the last 200 lines.
- **Where:** `<data dir>/repos/<repo key>/test-cache.jsonl`, kept for `test_cache_days` (default 14).
- **What hits the cache:** only green results. A red result is never reused, so a flaky red is always retried.
- The cache is what makes tier 2 free in the common case: a candidate whose stage head hasn't moved since the task branched has the task's own tree, which tier 1 already passed.

### 3.6 Flaky tests

- **Retry once.** A test step that fails is re-run once, only for its failing tests when the runner can filter to them (nextest or libtest filtered to the failing names), otherwise the whole step.
  - Passes on retry → the step is **green**, and each test that flipped is recorded as flaky in `history.jsonl` (`{"kind":"flaky","test":…,"tier":…,"run":…}`).
  - Fails again → a normal gate failure (rung 1 and up).
- **Quarantine.** A test recorded flaky in `flaky_quarantine_after` (default 3) runs within `flaky_window_days` (default 14) is **proposed** for `slow_tests` in `anthrex run stats`, together with an offered fix task. The user confirms; profile changes stay human.
- A flaky pass **never** costs a bounce. A flaky result in tier 3 never starts a bisect.

### 3.7 Red tier 3: bisect

A stage branch is a line of `--no-ff` merges, one per task, so a red tier 3 has an owner.

1. Take the first-parent merges on the stage branch since its last green tier 3, or since it was created.
2. Bisect them with the **failing tests only**, using `single_test` or `module_test` filtered to those names, never the full suite. This runs in the proof worktree through the scheduler.
3. **Culprit found** (its merge is the first red): the engine adds a fix task, `origin = bisect`, to the same stage.
   - Its `owns` are the culprit's `owns`, and its route is the culprit's route one rung up (adaptive spec §10 rung 2).
   - Its brief is a template containing the culprit task's title and brief, the failing test names, the ≤ 40-line decider summary, and `git show --stat` of the culprit merge.
   - Its `deps` is empty.
   - It needs no approval, because it stays inside an already-approved stage's `owns`.
4. **No culprit** (every step passes in isolation, or the first commit is already red): the tests fail only in combination or are environmental. The stage gets the attention line `tier 3 red, no single culprit: <tests>`, and the orchestrator is woken to plan a fix. After `bisect_fix_max` (default 2) bisect fix tasks on one stage, the next red goes to the user.

The culprit's own merged state is never reopened. Merged is terminal. A fix is always a new task.

### 3.8 Protecting existing tests

A diff can make tests pass by weakening them. The test proof only guards the *new* test.

- On `task_done`, the engine computes **test-weakening signals** from the task's net diff:
  - a deleted file matching `test_paths`;
  - an added line containing a `skip_markers` entry;
  - in files matching `test_paths` or containing `#[cfg(test)]`, a net loss of assertion lines (`assert`, `expect(`, `should`, per a built-in list per language).
- **A deleted test file** is a rung-1 bounce (`deleted test file <path>; restore it or own it exactly`) unless the task's `owns` names that path exactly, the same rule as protected files (adaptive spec §6).
- Every other signal is **not** a bounce. It goes into the reviewer's prompt as a block, `Test changes to justify:`, one line per signal with file and line. The reviewer must either accept each one with a reason or raise it as a critical finding. A review that doesn't mention a listed signal is incomplete and is re-run once.

## 4. The stage model

### 4.1 Stages in the plan

- Every task has `stage: u16`, default 1. A run with a single stage is today's run.
- **Validation** (added to adaptive spec §12.1):
  - a task may depend only on tasks in the **same or an earlier** stage (`task <id>: stage 2 cannot depend on t9 in stage 3`);
  - stage numbers are contiguous from 1;
  - every stage has at least one task.
- **Planner guidance** (orchestrator and sub-planner contracts, not an engine rule):
  - a stage targets `stage_target_lines` (default 300–800 changed lines);
  - a stage is a unit a person can review and a CI run can judge **on its own**, so it must leave the code building and passing (§4.3);
  - on the large path, one epic is usually one stage.
- The plan gate shows stages as groups. The user can move a task to another stage there, subject to the same validation.

### 4.2 Stage branches replace the single run branch

| Branch | Created from | Written by |
|---|---|---|
| `anthrex/<run>/stage-1` | the base commit | the engine's merge queue |
| `anthrex/<run>/stage-<n>` | stage *n−1*'s head when stage *n*'s first task becomes runnable | the engine's merge queue |
| `anthrex/<run>/<task>` | its stage branch, after its deps merged | its worker sessions |

- `anthrex/<run>/integration` is kept as an alias ref to the **last** stage's head, so the M8a/M8c code, the run view and `run accept` keep working unchanged in local mode.
- **Flowing changes up the stack.** When stage *k* advances after stage *k+1* exists (a fix task, a sync), the engine enqueues a **propagate** item into the merge queue: merge `stage-k` into `stage-(k+1)` with `--no-ff`, run tier 2 on it, then compare-and-swap. This repeats up the stack.
  - A propagate conflict has no worker to hand back to. The engine adds a `sync` fix task to stage *k+1* whose worktree already contains the conflicted merge; that task's worker resolves it, as in the §11.5 hand-back.
- The merge queue stays width 1 across the whole run. Each item names its target stage branch.
- The ref guard (adaptive spec §17) covers every stage branch.

### 4.3 Each stage must be green on its own

Stacked PRs are reviewed and CI-tested one at a time, so stage *n* must build and pass without stage *n+1*. Two consequences:

- **Interfaces change additively (expand, migrate, contract).**
  - The task that changes an interface **adds** the new form beside the old one.
  - Dependent tasks migrate callers.
  - A later task (usually a later stage) removes the old form.
  - This goes into the orchestrator and sub-planner contracts. The reviewer of an interface task is told `Check that the old interface still works: callers outside this task must still build.`
  - Tier 2 enforces it mechanically: a non-additive interface task's candidate fails `build_check`.
- **An `atomic` hub task** is the escape hatch for a change that can't be additive, e.g. a protocol version bump that must update every client together (AGENTS.md rule 4).
  - The planner sets `atomic = true` with a reason.
  - The task may span modules without being sized L (adaptive spec §7.2 rule 2 doesn't apply). It is still `hub`: it runs alone, uses `tdd`, and is reviewed at frontier strength.
  - At most one atomic task per stage.

## 5. Delivery modes

`[delivery] mode = "pr" | "local"`, per repository, stored with the profile.

- **Default:** `pr` when the repository has a GitHub remote and `gh auth status` succeeds at profile detection; otherwise `local`. The user confirms it with the profile.
- `local` is today's behaviour, with tiered testing: the run completes after the last stage's tier 3, and `run accept` merges the last stage branch into the base with `--no-ff`. Stages still exist, and just aren't pushed.
- `anthrex run start --delivery local|pr` overrides per run.

## 6. Stacked-PR delivery (milestone 9.2)

### 6.1 Preflight

At `run start` in `pr` mode, before the plan gate, on a blocking thread with timeouts. Each failure refuses the run with its own message:

- `gh` is installed and `gh auth status` succeeds for the remote's host.
- The remote (`[delivery] remote`, default `origin`) is a GitHub URL, and `gh repo view` succeeds.
- A dry-run push of a scratch ref succeeds: `git push --dry-run <remote> <base_sha>:refs/heads/anthrex/preflight-<run>`.
- The base branch exists on the remote.

### 6.2 Opening a stage PR

A stage is **ready** when all of these hold:

- every task in it is `merged` or `cancelled`;
- its merge-queue items are done;
- stage *n−1*'s PR is open or merged (never opened out of order);
- tier 3 is green on its head.

The engine then does three things.

**1. Push, never forcing.** `git push <remote> anthrex/<run>/stage-<n>:anthrex/<run>/stage-<n>`.
- If the remote ref exists and isn't an ancestor of the local head, the push is refused, nothing is forced, and the run halts with `remote stage branch was rewritten by someone else`.

**2. Create the PR.** `gh pr create --base <base or anthrex/<run>/stage-(n-1)> --head anthrex/<run>/stage-<n> --title … --body-file …`.
- The base is the base branch for stage 1 and the previous stage's branch otherwise.
- Not a draft.
- The title: `[anthrex r<id> <n>/<N>] <stage title>`.

**3. Write the PR body.** It is generated from the run snapshot and has five parts:
- the run goal, the stage title, and links to the previous and next PRs in the stack;
- **Look here first**, a list ranked by risk:
  - hub or atomic tasks;
  - tasks merged without approval (`override`);
  - tasks escalated to rung 2 or higher;
  - tasks whose diff raised test-weakening signals (§3.8);
  - files that match `protected`;
- a table with one row per task: id, title, size, test mode, the named test, review verdict and rounds, highest rung;
- test evidence: which tiers ran, tier 3 duration, and flaky tests seen;
- a footer that says the PR was opened by anthrex, that anthrex will push fixes for CI failures and review comments, and that **anthrex never merges it**.

The PR number and URL go into the run snapshot and the stage's inspector.

### 6.3 Watching open PRs

- **Poll, don't subscribe.** For each open stage PR, every `poll_secs` (default 60, backing off to 300 while nothing changes), one call runs on a blocking thread with a 30 s timeout: `gh pr view <n> --json state,mergedAt,mergeCommit,baseRefName,headRefOid,mergeable,reviewDecision,statusCheckRollup,reviews,comments` plus `gh api` for review threads with their resolved state.
- **Persist a watermark per PR:** the last seen check suite per head SHA, the last seen comment or review id, and the last known state. After a daemon restart, events are neither lost nor re-processed.
- **Rate limits.** A `gh` rate-limit error doubles `poll_secs` for that run until the next success.
- **The run's lifecycle changes.** A `pr`-mode run stays `delivering` after its last stage opens, until every stage PR is merged or closed. Only then is it `complete`.

### 6.4 A red CI run becomes a fix task

When the check suite for the PR's **current head SHA** concludes with at least one required or failing check:

1. **Fetch the logs.** `gh run view <id> --log-failed`, capped at `ci_log_max_bytes` (default 2 MB), treated as untrusted data.
2. **Summarise.** A decider (M8b's failure-summariser schema, extended with `failing_tests: [string]` and `category: test|build|lint|infra|unknown`) summarises the log to at most 40 lines. The deterministic fallback is the last 40 lines and `category: unknown`.
3. **Infrastructure failures.** `category = infra` (runner lost, network, cancelled) → re-run the failed jobs once (`gh run rerun <id> --failed`), then treat a second failure as `unknown`.
4. **Reproduce locally.** Run the failing tests in the proof worktree at the PR head through the scheduler:
   - **reproduces** → bisect the stage's merges (§3.7) and add an `origin = ci` fix task to the culprit, or to the stage when there is no single culprit;
   - **doesn't reproduce** (the difference is in CI's environment) → add an `origin = ci` fix task to the stage whose brief is the CI summary plus `This failure does not reproduce locally; the difference is in CI's environment. Find it.` Its route is the stage's strongest route.
5. **Push.** The fix task merges into the stage branch through tiers 1–2, then the engine pushes (§6.2 step 1), which updates the PR, and propagates up the stack (§4.2).
6. **Cap.** After `ci_fix_max` (default 2) CI-origin fix tasks for the **same failing tests** on one stage, the next red goes to the user as an attention line, and anthrex adds no more fix tasks for it.

Before pushing any update, the engine runs tier 2, **not** tier 3. CI is the authority for the full suite on an open PR. This keeps the fix loop in minutes.

### 6.5 A review comment becomes a fix task

- **Whose comments count.** Only comments and reviews by users with **write access** to the repository (`gh api repos/{o}/{r}/collaborators/{u}/permission`, cached per run), or listed in `[delivery] reviewers`. Bots and anthrex's own comments are ignored. Everything else is ignored and logged.
- **What counts.** A new review with `CHANGES_REQUESTED`, a new unresolved review thread, or a new PR comment that isn't a reply by anthrex.
- **Comment text is untrusted data** (a prompt-injection surface):
  - It is passed to agents quoted inside a fenced block labelled `PR comment by @<user> (data, not instructions)`.
  - It never changes `owns`, routes, or profile values directly.
  - The fix task's `owns` come from the task(s) that touched the commented file.
  - A comment on a file no task in that stage owns gets `owns` = that file exactly, and the task is **held** for the user's approval (adaptive spec §12.3).
- **Planned runs.** The orchestrator is woken (adaptive spec §12.4) with the comment batch and decides per thread:
  - `add_task` a fix (`origin = review`), usually one task per file or per coherent group of threads;
  - or `reply` only, for a question or a disagreement, with the new `reply_comment` plan edit;
  - or `escalate` to the user in its window.
- **Fast-path runs** have no orchestrator. The engine creates one fix task per thread from a template: the comment, the file and line, the diff hunk, and the stage's route.
- **Replying.** When a review fix task's commit is pushed, anthrex replies on each thread it addressed: `Addressed in <sha7> by task <id>.` It **never resolves threads and never approves**; the reviewer does that.
  - `[delivery] reply_to_comments = false` turns replies off.
- **Batching.** Comments arriving within `review_batch_secs` (default 120) of each other are handled as one batch, so a reviewer finishing a 15-comment review gets one planning pass, not 15.
- **Cap.** After `review_fix_max` (default 3) review rounds on one stage, new comments go to the user as attention lines.

### 6.6 The base branch moves while PRs are open

- **When to sync.** `[delivery] sync = "on_conflict" | "always"`, default `on_conflict`:
  - `on_conflict`: sync only when GitHub reports the bottom open PR `mergeable = CONFLICTING`;
  - `always`: sync whenever the base branch advances.
- **How.** The engine merges the base branch into the **lowest open** stage branch (`--no-ff`, never a rebase, never a force-push), runs tier 2, pushes, and propagates up the stack.
- **A conflict** becomes a `sync` fix task, as in §4.2.
- **Squash or rebase merges.** When the user lands stage *n* with a squash or rebase merge, stage *n+1*'s branch still contains stage *n*'s original commits.
  - The engine merges the new base into stage *n+1* before retargeting. Because the trees match, this is normally a clean no-op merge.
  - The PR body recommends merge commits for stacks.

### 6.7 Landing: the user merges

- **anthrex never:**
  - merges a PR;
  - approves a PR;
  - enables auto-merge;
  - pushes to the base branch;
  - force-pushes;
  - deletes a branch that a PR still uses.
- **On detecting stage *n* merged:**
  - retarget stage *n+1*'s PR to the base branch (`gh pr edit <n+1> --base <base>`), after the §6.6 squash handling;
  - record the merge commit;
  - delete the remote stage branch **only** if `[delivery] delete_merged_branches = true` (default false), since GitHub may already do it.
- **Stage *n* closed without merging:**
  - the stages above it pause;
  - the attention line reads `stage <n> PR closed without merging; resume, re-plan, or cancel the rest`;
  - the orchestrator is woken.
- **A run is `complete`** when every stage PR is merged or closed. `run accept` and `run discard` are refused in `pr` mode. `run cancel` closes nothing: it stops the agents and leaves open PRs for the user.

### 6.8 New and changed commands

| Command | Does |
|---|---|
| `anthrex run start --delivery pr\|local` | overrides the repository default |
| `anthrex run prs <run>` | lists stage PRs with state, CI status, open threads and fix tasks in flight |
| `anthrex run deliver <run> --stage <n>` | opens stage *n*'s PR now if it's ready (skips waiting for idle tier 3; still requires tier 3 green) |
| `anthrex run watch <run> --off\|--on` | pauses or resumes PR watching (e.g. during a long human review) |

### 6.9 Tools

- The orchestrator's `edit_plan` gains `reply_comment { pr, thread, body }` and the task fields `stage`, `atomic`.
- `run_status` gains a `delivery` block: per stage, the PR, CI state, unaddressed threads, and fix tasks.
- No worker, reviewer or scout tool changes. A fix task is an ordinary task.

### 6.10 The host interface

Every host operation is one trait, `trait CodeHost`, with:

- `preflight`
- `push`
- `open_pr`
- `view_pr`
- `failed_logs`
- `rerun_failed`
- `reply`
- `retarget`
- `permission`

`GhHost` implements it with the `gh` CLI. Tests use `FakeHost`, a scripted implementation driven like `fake-agent`, so no test ever talks to GitHub. Every `GhHost` call runs on `spawn_blocking` with a timeout, and never under the manager lock (AGENTS.md rules 2 and 10).

## 7. The run view and snapshot

- **Snapshot.** The run snapshot gains `stages: [{n, title, branch, head, tier3: {state, at, duration}, pr: {number, url, state, ci, open_threads}?}]`, and each task gains `stage` and `origin`.
- **Run view.** Tasks are grouped by stage: a **stage node** sits between the orchestrator (or planner) tier and its tasks. Its content row reads `<glyph> stage 2/3  #142  ci ✓  2 threads`.
- **Stage inspector:**
  - progress;
  - the PR line;
  - the CI line (per check, most recent head);
  - the threads line (open, addressed, replied);
  - tier 3 (state, duration, shards, flaky);
  - fix tasks by origin.
- **Task inspector.** Adds `origin` and, for fix tasks, `fixes: CI run 123 / thread by @user / bisect of t4`.
- **The orchestrator's `path` line** accounts for the time a stage spends waiting on human review separately. That time is excluded from the bound ratio.

## 8. Protocol, config and history

- `PROTO_VERSION` bumps once in 9.1 and once in 9.2, each re-derived from `main`. Every new message gets a round-trip test (AGENTS.md rule 4).
- **Config:**
  - `[testing]`: `test_slots`, `full_idle_secs`, `test_cache_days`, `flaky_quarantine_after`, `flaky_window_days`, `bisect_fix_max`.
  - `[delivery]`: `mode`, `remote`, `poll_secs`, `ci_log_max_bytes`, `ci_fix_max`, `review_fix_max`, `review_batch_secs`, `reviewers`, `reply_to_comments`, `sync`, `delete_merged_branches`, `stage_target_lines`.
- **`history.jsonl` gains:**
  - per tier run: tier, duration, affected set size, whether it was full and why, cache hit;
  - flaky records;
  - bisect records;
  - per stage: time to open, time in human review, CI rounds, review rounds, merged or closed.

## 9. Tests every milestone must cover

**9.1:**
- the affected set for a leaf crate, a crate with dependents, a hub file, a full trigger, an unowned path, and a docs-only change;
- an unknown graph falls back to `check`;
- tier 2 is skipped on a tree already in the cache;
- a red result is never cached;
- a flaky test passes on retry, is recorded, and costs no bounce;
- the third flake is proposed for quarantine;
- a red tier 3 is bisected to the right merge, and the bisect fix task gets the culprit's `owns` and the route one rung up;
- a red tier 3 with no single culprit → attention, then the user after `bisect_fix_max`;
- the timing group runs alone;
- slot caps appear in the environment of tier commands and of worker sessions;
- a test that starts a daemon gets an isolated socket;
- a deleted test file bounces;
- a skip marker reaches the reviewer prompt;
- a profile with none of the new keys behaves exactly as M8a (a regression test running M8a's e2e scenarios unchanged).

**9.2** (all through `FakeHost`):
- a stage dependency on a later stage is rejected;
- stage PRs open in order with the right bases;
- a push refuses to force;
- CI red, reproduced → bisect fix task → push → propagated up the stack;
- CI red, not reproduced → an environment fix task;
- an infrastructure failure is re-run once;
- the `ci_fix_max` cap;
- a review comment by a writer → orchestrator batch → fix task → reply;
- a comment by a non-writer is ignored;
- a comment containing injected instructions reaches the agent only as quoted data;
- a comment on an unowned file → held task;
- the base advances with a conflict → sync task;
- a squash-merged stage *n* → the next stage retargets cleanly;
- a PR closed without merging pauses the stages above it;
- a daemon restart resumes watching from the watermark with no duplicate fix tasks;
- `run accept` refused in `pr` mode;
- `local` mode unchanged;
- anthrex never calls merge, approve or auto-merge. `FakeHost` panics if asked.

**PTY smoke:** 9.1 stage `11h` and 9.2 stage `11i`, following the existing letter scheme. The 9.2 stage uses `FakeHost` through `ANTHREX_CODE_HOST=fake`.

## 10. Milestones

| Milestone | Scope | Depends on | Testable without a model? |
|---|---|---|---|
| **9 (addition)** | §12: the `message` and `refresh` plan edits, the worker tool `task_note`, `anthrex run message`, and their display | 8a, 8b, 8c (unchanged) | yes, with `fake-agent` |
| **9.1: tiered testing** | §3 and §4 (stages, stage branches, propagate, additive-interface contract text, atomic tasks), §5 `local` mode, the scheduler, the cache, flake handling, bisect, test-weakening signals, profile keys and their onboarding detection | 9 | yes, with `fake-agent` and temporary repos |
| **9.2: stacked-PR delivery** | §6 and §7, `CodeHost` + `GhHost` + `FakeHost`, the fix-task origins `ci`, `review`, `sync`, and the orchestrator's `reply_comment` | 9.1 | yes, with `FakeHost`; one manual run against a real scratch GitHub repository |

Both come before 9.5, whose tuning (thresholds, routing, racing) should be fitted on runs that already use tiers and stages.

**9.2's manual check:**
- a real run against a scratch GitHub repository the user owns, with two stages;
- the user pushes a commit that breaks CI on stage 1, and leaves one review comment;
- verify the CI fix, the review fix, the reply, and the retarget after the user merges stage 1 on GitHub;
- verify anthrex merged nothing.

## 11. What this supersedes

| Document and section | Status |
|---|---|
| Adaptive spec §5.3 step 6 (integration: check on the run head) | replaced by tier 3 per stage (§3.2) |
| Adaptive spec §5.3 step 7 (finish: accept merges into base) | kept for `local` mode; `pr` mode delivers by §6 |
| Adaptive spec §11.3 (check) | replaced by tier 1 (§3.2) |
| Adaptive spec §11.5 step 2 (check on the candidate) | replaced by tier 2 (§3.2); the rest of §11.5 is unchanged |
| Adaptive spec §11.6 (one run branch) | extended by stage branches (§4.2); `integration` is kept as an alias |
| Adaptive spec §7.3 "interfaces first" | amended: interfaces first **and additive**, with `atomic` as the exception (§4.3) |
| Adaptive spec §17 ("nothing reaches base until accept") | kept, generalised: nothing reaches base except by the user's own merge of a PR or `run accept` |
| M8a decision 37 (final check on the run head) | replaced by tier 3 at completion in `local` mode |
| The unapproved "M8d auto-merge / landing policy" proposal | rejected: anthrex never merges |
| Adaptive spec §12.1 (the plan-edit vocabulary) | extended by `message` and `refresh` (§12) |
| Adaptive spec §3 ("no agent summarises another agent's work for a third") | kept; §12.5 says how orchestrator messages stay within it |

## 12. The orchestrator messages workers (milestone 9)

The adaptive spec lets the orchestrator reach a running worker only through `answer` (to a question) and `amend_task` (a changed brief or acceptance criteria). That isn't enough when the plan changes around a task that is already running:

- an interface task merged, and running workers should use the new form;
- a sibling task was split or cancelled;
- the user said "don't touch the config loader";
- a scout found something relevant after the worker started.

Workers also have no way to report such a discovery without claiming to be blocked.

### 12.1 `message`: tell one or more workers something

A new plan edit, available to the orchestrator and to the user (`anthrex run message`), never to sub-planners:

```
message { to: [task_id] | "stage:<n>" | "running", text, kind: "info" | "change" | "stop_and_wait" }
```

- **Recipients.** `to` resolves when the edit is accepted:
  - explicit task ids;
  - every unfinished task in a stage;
  - or every task that currently has a live worker (`running`).
- **What each task state receives:**

| Task state | What happens |
|---|---|
| `working`, or `blocked(question)` | Queued in M8a's outbox (M8a decision 29) and delivered as the session's **next turn**, after its current turn ends: `[anthrex] Message from the orchestrator (<kind>): <text>`. A `blocked(question)` task stays blocked; only `answer` unblocks it. |
| not started yet (`pending`, `waiting`, `queued`, `preparing`) | Appended to the task's **notes**, a new list field shown in the worker prompt under `Notes from the orchestrator:` just before the brief. Its first session sees it. |
| `review`, `merge_queue`, `merged`, `cancelled`, `reported`, `blocked(other)` | Refused for that task with `task <id> is <state>; a message would not reach a worker`. The other recipients still get it; the reply lists who got it and who didn't. |

- **Notes survive sessions.** Every message delivered to a live worker is also appended to the task's notes. A fresh rung-2 session, a resumed session, or a rewritten task starts with all of them.
- **`kind`:**
  - **`info`**: context. The worker may carry on as it was.
  - **`change`**: the plan or code around the task changed. The worker's contract says it must acknowledge the change in its next `task_done` summary (`Changes applied: …`), and the reviewer is shown the message.
  - **`stop_and_wait`**: the worker must finish its current step, commit anything worth keeping, and end its turn without calling `task_done`. The task goes to `paused(message)` until a later `message`, `amend_task` or `resume` for that task. This is how the orchestrator stops a worker that is about to build on something that just changed, without cancelling its work.
    - The engine enforces it: a `task_done` from a `paused(message)` task is refused with `this task was asked to stop and wait; wait for the next message`.
- **Limits.**
  - `text` is at most 4000 characters.
  - At most `message_max_per_turn` (default 3) messages are queued per task per turn. They are delivered together as one turn, as the outbox already does.
  - A message can't change `owns`, route, size, test mode or acceptance criteria; those stay `amend_task`. The orchestrator's contract says: `A message informs; an amendment changes the task. If the task's scope changes, use amend_task.`
- **Urgency.** No message interrupts a turn in progress. Interrupting mid-tool-call is unreliable across runtimes, and a turn is bounded by the stall watchdog anyway. `stop_and_wait` is the fastest stop, and it takes effect at the end of the current turn.

### 12.2 `refresh`: bring merged changes into a running worker's worktree

```
refresh { task_id }
```

- **Why.** A worker branched from the run branch (a stage branch from 9.1 on) when it started. Later merges don't reach it until the merge queue, where they show up as conflicts or failing candidates. `refresh` brings them in early.
- **Allowed** on a task in `working` or `paused(message)` whose worktree has **no uncommitted changes**.
- **How.** At the next turn boundary, the engine merges the current head of the task's target branch into the task branch in its worktree (`--no-ff`, never a rebase), through the journaled op machinery.
- **Outcomes:**
  - **Clean merge** → the worker's next turn is `[anthrex] Your branch now includes the latest merged work (<n> commits: <short list>). Rebuild before you continue.`
  - **Conflict** → the markers stay in the worktree, and the worker's next turn lists the conflicted files and says `resolve them, commit, and continue`. This is the same mechanism as M8a's merge-queue hand-back, but it **doesn't** count as a merge conflict toward `blocked(conflict)`.
  - **Uncommitted changes in the worktree** → refused with `task <id> has uncommitted changes; send it a message asking it to commit first`.
- **Pairing.** The orchestrator's contract pairs `refresh` with `message kind=change` whenever it tells a worker about merged code, so the worker gets the code and the explanation in the same turn.
- **Diff accounting.** The task's own net diff, used for the spill check and tier 1's affected set, stays `git diff <run head>...HEAD` (three dots), so merged-in work never counts as the task's own changes.

### 12.3 `task_note`: workers report back without being blocked

A new worker tool:

```
task_note { kind: "discovery" | "risk" | "progress", text }
```

- **Where it goes.** Recorded on the task and surfaced in the orchestrator's `run_status` digest, as a new `notes` list of the most recent notes across tasks, cut to 400 characters each.
  - `discovery` and `risk` also wake the orchestrator (adaptive spec §12.4).
  - `progress` doesn't.
- **It never changes the task's state.** A worker that needs an answer before it can continue still uses `task_blocked { kind: question }`.
- **Limits.** At most `note_max_per_task` (default 10) notes per task. Beyond that the tool answers `note limit reached; put the rest in your task_done summary`.
- **Worker contract text:** `If you learn something that affects other tasks or the plan, such as another place that must change, a wrong assumption in the brief, or a risk, report it with task_note and keep working. Use task_blocked only when you cannot continue.`
- **Example.** A worker notes `discovery: the status enum is also matched in crates/cli/src/hook.rs, which no task owns`. The orchestrator can then `add_task` for that file and `message` the dependent workers, all without the worker stopping.

### 12.4 The user can message too

- `anthrex run message <run> <task|stage:<n>|running> [--kind info|change|stop_and_wait] <text>` produces the same edit with source `user`.
- `anthrex run refresh <run> <task>` does the same for `refresh`.
- This keeps the adaptive spec's rule that everything the orchestrator can do is also an `anthrex run` command.
- Workers stay headless, and the user still can't type into them. A message goes through the engine's outbox at a turn boundary, like every other engine message.

### 12.5 Keeping the information-flow rule

The adaptive spec §3 says no agent summarises another agent's work for a third. Messages keep to that rule in three ways:

- **The worker gets the code, not a paraphrase.** When one worker's merged code matters to another, the recipient gets it through `refresh`, plus the orchestrator's instruction about what changed. It doesn't get a secondhand summary of the other worker's transcript.
- **Notes arrive attributed.** A `task_note` reaches the orchestrator marked with the task that wrote it, and the orchestrator decides what to do with it.
- **The engine records every message** in the edit log with its source and recipients, and they appear in the run report.

### 12.6 Display

- **Conversation view (M6.5).** A delivered message appears as a user turn labelled `orchestrator` or `user`, not as a plain prompt.
- **Task inspector.** A `messages` row: the count, and the most recent message's kind and first line.
- **`paused(message)`** has its own glyph and colour in the run view (`‖`), and is listed in the run's attention line when it has lasted more than 10 minutes.
- **Notes.** `task_note` entries appear in the task's `history` row and in the run inspector's `attention` line when their kind is `discovery` or `risk`.

### 12.7 Tests (milestone 9)

- a `message` to a working task is delivered after the current turn ends, never during it;
- a `message` to a pending task appears in its first prompt's notes;
- a `message` to a task in review is refused for that task alone;
- a fresh rung-2 session receives every earlier note;
- `stop_and_wait` moves the task to `paused(message)`, a `task_done` from it is refused, and a later message resumes it;
- more than `message_max_per_turn` messages are refused;
- `refresh` merges cleanly and the next turn names the commits;
- `refresh` with a conflict leaves markers, doesn't count toward `blocked(conflict)`, and the task's net diff excludes the merged commits;
- `refresh` with uncommitted changes is refused;
- a `task_note` of kind `discovery` wakes the orchestrator and `progress` doesn't;
- the note limit is enforced;
- `anthrex run message` from the user produces the same edit with source `user`;
- every message round-trips through `proto` (AGENTS.md rule 4).
