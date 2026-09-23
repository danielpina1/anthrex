# anthrex design: the adaptive orchestrator

Date: 2026-09-22. Status: **draft for review — not binding until approved.** Implemented by milestones 8 and 9, re-cut in §21.

Amends `2026-09-20-git-surface-and-simple-orchestration-design.md` §5 and `2026-09-21-agent-conversation-view-design.md` §9. §18 lists exactly what it supersedes. Where it disagrees with either, and once approved, it wins.

The evidence behind each decision is in the two research reports of 2026-09-22 — *Coding agent orchestrator design* and *Hierarchical token efficient orchestrator* — summarised with sources in §20. Every decision below is marked **[E]** when a primary or well-sourced finding supports it, and **[J]** when it is a judgement call the evidence points toward but does not settle.

## 1. What this is for

One orchestrator that can be pointed at **any repository in any language**, at **any kind of work** — a feature, a bug fix, a refactor, a migration, docs, an investigation with no code output, a review of an existing branch — and at **any scale**, from a one-line fix to a multi-subsystem goal. It adapts to each on its own, and the user can steer it while it runs.

It optimises three things, in this order:

1. **Quality** — nothing reaches the base branch that has not passed its gates and the user's accept.
2. **Speed** — wall-clock as close as possible to the task graph's critical path.
3. **Tokens** — spend proportional to task size, with no agent re-reading what another already read.

Test-driven development is the main paradigm, **applied when a task changes behaviour** and not otherwise (§8).

Budget and autonomy are not adaptive dimensions: the user's own subscriptions, with the plan gate on by default and `--yes` to skip it.

### 1.1 What the evidence says, in five lines

- A **deterministic engine owning every state change** beats an LLM coordinator: error amplification 4.4x with central coordination against 17.2x with independent agents; every LLM-led system surveyed documents lagging status and early stops. [E]
- **Hierarchy buys planning capacity, not speed.** Wall-clock is bounded by the critical path plus check and review per task; sub-orchestrators add a hop to that path. The one multi-level success (Cursor) made *planning* recursive and kept execution flat. [E]
- **Writes stay single-threaded per file set.** Parallel readers (scouts, reviewers) are cheap and safe; parallel writers on the same files are not. Different agents on overlapping code conflict 41.7% of the time. [E]
- **LLMs rank task difficulty well but misjudge absolute size**, and always overestimate. Size must come from scout evidence checked by rules, then be recalibrated from real runs. [E]
- **Tokens go to re-reading.** Most spend is context re-sent every turn plus repeated exploration. Scouting once, stable prompt prefixes, filtered tool output and size-matched model and effort are the levers. [E]

## 2. Terms

- **goal** — what the user asks for, in words.
- **run** — one attempt at a goal: a run branch, a task graph, and the agents working on it.
- **epic** — a planning-only grouping of tasks for one area. Epics are never executed.
- **task** — the unit the engine runs. Always size S or M (§7).
- **area** — a set of paths one planner owns, written as `owns` globs. Usually one module.
- **module** — a crate, package or directory the repo profile declares as a unit (§6).
- **hub file** — a file many modules depend on (in anthrex, `crates/proto/**`). Declared in the profile.
- **gate** — a check a task must pass before merging: done, test proof, check, review, merge.
- **bounce** — a gate failure that sends a task back to its worker.
- **rung** — a step on the escalation ladder (§10).
- **plan edit** — any change to the task graph, from anyone (§12).
- **route** — `{runtime, model, effort}` for one agent.

## 3. Shape

```
                          USER
          (types in the orchestrator's window, or `anthrex run …`)
                           │ steer
                           ▼
   ┌─────────────── ORCHESTRATOR (1 per run) ──────────────────────┐
   │ THE ONLY INTERACTIVE PTY · frontier tier · read-only          │
   │ plans, re-plans, answers blocked workers, talks to the user   │
   │ reads the run digest, never transcripts or checkouts          │
   └──────┬──────────────────────────────┬─────────────────────────┘
          │ plan edits (MCP)             │ spawns, large goals only
          ▼                              ▼
   ┌────────────── ENGINE (daemon, no model) ──────────────────────┐  SUB-PLANNERS
   │ task graph + intent log · scheduler · gates · merge queue     │◄─ one per epic,
   │ size / test-mode rules · routing policy · budgets · ladder    │   read-only, add tasks
   │ repo profile · run history · reconciliation · live tree feed  │   then exit
   └──┬───────────┬────────────┬──────────────┬────────────────────┘
      ▼           ▼            ▼              ▼
   SCOUTS      WORKERS     REVIEWERS      DECIDERS
   └──────── all headless: no terminal, watch their event stream ────────┘
```

**Information flows down through the orchestrator and sub-planners, and up through the engine only.** A sub-planner exits once its tasks are accepted; it never relays results. Workers, reviewers and scouts report to the engine; the orchestrator reads the engine's digest. No agent summarises another agent's work for a third. [E — relayed summaries lose information; structured state does not]

Execution is two levels deep: the orchestrator, then flat leaves. Hierarchy exists only while planning. [E]

"Never checkouts" means task checkouts: the orchestrator's working directory is the user's own checkout of the repository, which it may read and never write, and it never reads a task worktree; `task_result` (§19) gives it a task's commits, diffstat, checks and reviews instead. *(settled in the briefs, 2026-09-22)*

## 4. Roles

| Role | Runs as | Route (default) | Writes? | Lifetime | Reports through |
|---|---|---|---|---|---|
| **Orchestrator** | **the one interactive PTY window** | frontier tier, high effort | no | whole run | plan edits; its own window to the user |
| **Sub-planner** | headless session | frontier tier, high effort | no | one epic | `submit_epic`, then exits |
| **Scout** | headless session | fast tier, low effort | no | one question | `submit_scout_report` (≤ ~2k tokens) |
| **Worker** | headless session, own worktree | by size (§9) | yes, its task branch only | one task (one session per rung) | commits + `task_done` / `task_blocked` |
| **Reviewer** | headless session, read-only worktree | other runtime, strength ≥ author | no | one review round | `submit_review` |
| **Decider** | one headless call, no session kept | fast tier, low effort | no | one call | a JSON answer validated against a schema |
| **Engine** | the daemon | — | only merges into the run branch | always | the run snapshot and the live tree feed |

**Only the orchestrator is an interactive terminal**: a real `claude` or `codex` TUI in a PTY, where the user types. **Every other agent runs headless** and has no terminal at all:

- **Claude:** one long-lived `claude -p --input-format stream-json --output-format stream-json` process per session. Follow-up messages are written to its stdin as user messages.
- **Codex:** `codex exec --json` for the first turn, then `codex exec resume <session> --json` for each follow-up.

The daemon reads each session's event stream and gets exact, structured signals where a terminal only gives guesses:
- the end of a turn (`result`, `turn.completed`);
- rate-limit retries (`system/api_retry` with `error: rate_limit`, `turn.failed`);
- permission denials;
- per-turn token usage.

That stream is also what the user watches, through the conversation view (§4.2). [E — the research found structured protocols more reliable than a scraped terminal, and every surveyed tool that needed reliable done signals used one]

Every headless agent is also **contained**:

- **Only the user's own settings load.** Headless Claude sessions load the user's settings, not the repository's, so a cloned repository's hooks and `.mcp.json` servers never run unprompted. If the CLI cannot exclude project settings, a run refuses to start in a repository that has them unless the user passes `anthrex run start --trust-project`.
- **Codex loads only the user's config too.** Whether `codex exec` reads a repository's own Codex config is verified in M8a's first task; if it does, the same rule as Claude applies: exclude it, or refuse the run unless `--trust-project`.
- **Workers run sandboxed.** Workers run with Claude Code's sandbox enabled and unsandboxed commands disallowed, so `Bash` is confined to the task worktree plus the shared git directory it must commit to. Anything outside is denied and reported in the stream. Network work belongs in the profile's `setup` step, which the engine runs before the worker starts, as Codex's `workspace-write` sandbox already requires. [E — the sandbox settings are documented; that they apply under `-p` is verified in M8a's first task]

Read-only roles are launched read-only by the runtime, not only by prompt: Claude with `--permission-mode plan` (which also closes the `Bash` loophole M9 risk 4 names), Codex with `-s read-only`. Scouts on research goals may also use web fetch and search. [E]

The interactive Claude orchestrator is the exception to plan mode: it is launched with `Edit`, `Write`, `NotebookEdit`, `Bash` and `Agent` disallowed (`--disallowedTools`), because in the interactive TUI plan mode injects its own present-a-plan-and-`ExitPlanMode` instruction, which prompts the user and fights the contract. If the CLI turns out not to enforce `--disallowedTools`, it falls back to `--permission-mode plan`. A Codex orchestrator runs `-s read-only`. *(settled in the briefs, 2026-09-22)*

**Sub-planner limits.** A sub-planner is bounded by `[orchestrator.planners]`: a tool-call limit (a wrap-up message, then killed at 1.5 times it), a wall-clock timeout, and a number of rejected `submit_epic` batches. Hitting any of them, or two turns without an accepted epic, fails the planner; its accepted tasks stay, and the orchestrator is told and may start a fresh one. These bound the planning session, not any task's size. *(settled in the briefs, 2026-09-22)*

**Deciders** are one-turn headless sessions on the same stream-json and `codex exec --json` machinery as every other agent, with a JSON schema, on the user's own login. They make the narrow, frequent judgements that would otherwise fill the orchestrator's context: triage (§5), a size cross-check (§7), classifying a `task_blocked` reason (§10), summarising a failed check to ≤ 40 lines. Every decider has a deterministic fallback, so a failed or unavailable decider degrades a decision and never blocks a run. [J]

### 4.1 Agents per task

A task has **exactly one writer at a time**, and may have many agents over its life. [E — writes stay single-threaded per file set]

| Pattern | Agents on the task | Worktrees | When |
|---|---|---|---|
| **Single worker** | one worker | the task worktree | default |
| **Escalated** | worker #1, then a fresh worker #2 on a higher effort or the peer runtime (§10) | the same task worktree; #2 continues the branch | rung 2 |
| **With review** | the worker, then one fresh reviewer per round | reviewer gets its own read-only worktree at the task head | every task (§9) |
| **Worker with sub-agents** | the worker plus its runtime's own sub-agents (Claude's Agent tool, Codex sub-agents) | the task worktree, shared | the worker's choice; the contract tells it to use sub-agents to read and explore, not to write |
| **Test writer, then implementer** | a test writer commits the failing test (the red commit), stops; a different worker makes it pass | the same task worktree, one after the other | hub tasks, opt-in per task (`pair = true`) [J] |
| **Race** | two workers on different runtimes, in parallel | one worktree and branch each: `…/<task>.a`, `…/<task>.b` | opt-in for critical-path tasks, default off (§13) [J] |

Two writers in one worktree at the same time is never allowed. A task that needs two writers at once is two tasks with disjoint `owns`.

### 4.2 Who the user can talk to

**The orchestrator is the only interactive window**, as in Claude Code, where sub-agents are watched but never addressed. Sub-planners, scouts, workers and reviewers are **headless**: they have no terminal to type into, so the user cannot talk to them by construction, not only by policy.

The user **sees** everything they do through the conversation view (milestone 6.5), fed live from each session's event stream: every message, tool call and tool result, and sub-agents. Kill and remove are refused for run sessions; only the engine starts, messages and stops them, and `anthrex run cancel` is the user's way to stop one.

- **Steering goes through the orchestrator**, or through `anthrex run …` commands, which produce the same plan edits (§12).
- **A question a worker cannot resolve** reaches the user through the orchestrator (§10). It never arrives as a prompt in the worker's window.
- **Workers must never wait on a prompt**, because nobody can answer one. Their launch flags leave no interactive permission prompt reachable. A headless session cannot show a prompt at all: Claude runs with `--permission-prompts none`, so anything outside its allowed tools is denied and reported in the stream, and Codex runs with approvals set to never, inside its sandbox. A task whose worker keeps hitting denials is `blocked(environment)` and reported to the orchestrator.
- **The fast path has no orchestrator.** The user steers it with `anthrex run` commands, or `run promote`s it to a planned run with an orchestrator.
- **A dead orchestrator window comes back by restart.** The engine never waits for the orchestrator: if its window exits, the run keeps executing, wake-ups are suspended, and the run's attention list says to `anthrex restart <window>`, which relaunches it with every role flag and `--resume`. After a daemon restart the window is restored dormant and `anthrex run resume` restarts it. While the run is live its window refuses kill and remove like every run session; once the run is terminal it is an ordinary window. Everything the orchestrator does is also available as an `anthrex run …` command. *(settled in the briefs, 2026-09-22)*

## 5. How a goal flows

### 5.1 Triage

A decider labels the goal with a **kind** and a **scale**:

- kind: `code`, `docs`, `research` (investigation, no code output), `review` (of an existing branch or PR). A goal may mix kinds.
- scale: `single` (one task), `plan` (up to `planner_task_cap` tasks), `large` (more, or two or more disjoint areas each needing several tasks).

The engine picks a path from the label. Without a decider, the path is **plan**.

| Path | When | What happens |
|---|---|---|
| **Fast** | `single`, the one task is S or M, touches no hub file | No orchestrator window. The decider's brief becomes one task; the engine runs it through the normal gates. There is no plan gate; the user still accepts or discards the result. `anthrex run promote` turns it into a planned run at any time. |
| **Plan** | `plan` | Scouts, then the orchestrator writes the plan, then the plan gate. |
| **Large** | `large` | Scouts, then the orchestrator writes epics and the interface tasks itself, one sub-planner per epic writes that epic's tasks, then the plan gate. |

The fast path exists because a planner, a plan gate and a scout for a one-line fix is pure overhead. [J]

### 5.2 Per kind

- **code** — the full pipeline below.
- **docs** — the full pipeline with test mode `none` (§8).
- **research** — scout tasks only. Each writes a report into the run's data directory. There is no task branch, no merge, and `run accept` shows the combined report.
- **review** — reviewer tasks against the named branch or range, producing findings. No merge.

Research and review tasks have an empty `owns` and test mode `none`. A review task names its target in `review_target` (one revision, or `<a>..<b>`), resolved when it is dispatched. Both run in reader slots and end in the finished state **`reported`** whatever their findings or verdict; a dependency on one is satisfied by `reported` as by `merged`. *(settled in the briefs, 2026-09-22)*

### 5.3 Execution, for code

1. **Scouts** run first, one per area the goal touches, in parallel. Their reports feed every planner and every worker brief in that area.
2. **Plan.** The orchestrator (and, on the large path, sub-planners) writes tasks. The engine validates each plan edit (§7, §8, §12).
3. **Plan gate** (§12.3), unless `--yes`. Worktrees are pre-created and the profile's `setup` run while the gate is open.
4. **Schedule** (§13). Hub tasks first and alone; then critical-path order.
5. **Gates**, per task (§11): done → test proof → check → review → merge queue.
6. **Integration.** When every task has merged, the check runs once more on the run head, and on the large path one integration review per epic reads that epic's combined diff. An integration review that asks for changes holds the run's completion until the orchestrator adds a fix task to that epic (whose merge starts the next round) or ends the run with the `finish` edit, which completes it with the findings in the report. Nobody approves an epic by edit, and after `max_bounces + 1` rounds no new round starts. *(settled in the briefs, 2026-09-22)*
7. **Finish.** The orchestrator writes a summary; the user accepts (a `--no-ff` merge into the base branch) or discards.

## 6. The repo profile: adapting to any repository

On the first run in a repository, an **onboarding scout** writes a profile. The user confirms it once in a form; it is then stored in anthrex's data directory keyed by the repository root. **Nothing is written into the repository**, and there is no repo-level profile file: a committed file would be one an agent could edit to weaken the checks it is judged by. The user never writes the profile by hand; they can re-run detection with `anthrex profile detect` and correct a value with `anthrex profile edit`.

```toml
languages = ["rust"]
modules   = ["crates/*"]                  # what counts as one module for sizing
hub       = ["crates/proto/**"]           # always sized ≥ M, run alone, TDD, reviewed
source    = ["crates/*/src/**"]           # behaviour lives here
generated = ["Cargo.lock"]                # rewritten by builds; touching one outside `owns` is a bounce, not a spill
protected = [".claude/**", ".mcp.json", ".codex/**", "**/CLAUDE.md", "**/AGENTS.md"]  # agent config and instructions
setup     = "cargo fetch"                 # run once per new worktree
check     = "cargo build --workspace --all-targets && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check"
single_test = "cargo test --workspace -- --exact {test}"   # needed for the test proof
test_passed = 'test {test} \.\.\. ok'     # regex that proves the named test ran and passed
output_filter = "failures-only"           # applied to test output by a hook (§14)
conventions = ["AGENTS.md", "CLAUDE.md"]
[env]
CARGO_TARGET_DIR = "{worktree}/target"    # never shared between worktrees
```

- The scout reads manifests, CI files, READMEs and agent instruction files. Unlike every other scout it may run commands: it works in a disposable worktree, sandboxed, with shell access but no file-editing tools. It proposes `setup`, `check` and `single_test`; the **engine** then runs each itself in a fresh scratch worktree, and only commands that pass are proposed to the user. The scout's claims are recorded, never trusted.
- The profile also records `manifests` (the files whose change triggers re-detection) and which commands produce test output for the filter (§14.3).
- The profile is re-proposed when any file named in `conventions`, or a manifest, changes hash.
- **Degradation is explicit.** No test runner: `single_test` is empty, TDD is impossible, code tasks default to test mode `check`, and every review moves up one size (§9). No check at all: every task is reviewed as M or stronger, and the run report says the run was unverified.
- `generated` lists files that builds rewrite on their own. A task that changes one outside its `owns` gets a normal gate failure at rung 1 naming the file (revert it, or the task must own it); it is not sent to rung 3 as a spill. The scout fills the list from the repo's lock files.
- `protected` lists files that configure or instruct future agents: Claude and Codex settings, hooks and MCP servers, and the `CLAUDE.md` / `AGENTS.md` instruction files. After a run is accepted they affect every later session, including the user's own, so a task may change one **only if its `owns` names that path exactly**, never through a wildcard; the change is then visible in the plan the user approves. Any other change to a protected file is a rung-1 gate failure naming the file, like a generated file. The scout fills the list with these defaults; the user can add more.
- The per-worktree environment is part of the profile because every local orchestrator surveyed had to add it, and a shared Cargo target produced 460 GB and false test failures in one fleet. [E]

## 7. Sizing: small, medium, large

Size comes from **scout evidence, checked by rules the engine computes itself**. The planner never outputs minutes. [E — LLM absolute estimates scored 6.6–36% against human buckets and always ran long; ordering was right]

### 7.1 The rubric

| Size | Evidence the planner must cite | Starting thresholds (config; refit from history, §15) |
|---|---|---|
| **S** | one file, no interface change, a mechanical check exists | ≲ 20 changed lines, 1–2 hunks |
| **M** | 1–3 files inside one module, a clear spec, a check exists | ≲ 100 lines |
| **L** | any of: files in more than one module plus an interface change; more than ~100 lines; an ambiguous spec; a new dependency | — never executed |

The multi-file cliff is the only threshold with strong evidence (55.6% of hard SWE-bench Verified tasks are multi-file against 3.1% of easy ones). The line counts are anchors to refit, not facts. [E for the cliff; J for the numbers]

### 7.2 Engine rules

The engine applies these to every task in every plan edit. They can only raise a size, never lower it.

1. `owns` spans more than one module → at least **M**.
2. `owns` spans more than one module **and** the planner flags an interface change → **L**.
3. `owns` touches a hub file → at least **M**, and the task gets `hub = true`: it runs alone in its wave (§13), test mode `tdd` if it is code, and it is always reviewed. A hub task is not L merely for touching the hub, because it often cannot be split further.
4. A plan edit that leaves any task at **L** is rejected with a structured error naming the task and the rule, telling the planner to split it.
5. A decider cross-checks each task's size against its scout evidence. On disagreement the engine takes the larger size and records the disagreement. Fast-path tasks skip this check: triage already sized them from the same evidence.

### 7.3 Splitting

- An L goal or epic is split **interfaces first**: the interface or hub change is its own task, first, and every task that uses it depends on it. [E — dependency-aware partitioning with hub isolation gave 2.10x wall-clock and +14% pass rate]
- Split depth is one: L → S/M. A leaf that is still L goes back to its planner, never down another level. [J]
- A **chain** of tasks where each depends only on the previous one, and whose combined size is still M, is usually better as one task: each extra link adds a worker cold start, a check, a review and a merge, and nothing runs beside it. [J, following the sequential-work penalty in the scaling study] Chain collapse is guidance the orchestrator and sub-planners apply, written into their contracts as a judgement: prefer one task, and split a chain only when a step must be reviewed or merged on its own. The engine never checks it: neither its plan validation (§7.2) nor the batches the orchestrator and sub-planners submit have a chain rule. *(settled 2026-09-22, §22 item 6)*

## 8. Test mode: TDD when the task needs it

Every task carries `test_mode` and, unless it is `tdd`, a one-line `test_mode_reason`.

| Mode | For | The worker is told |
|---|---|---|
| **`tdd`** | any change in behaviour: features, bug fixes, state machines, protocols | name the test, commit it failing, then make it pass |
| **`check`** | behaviour-preserving work already covered by tests: renames, mechanical refactors, dependency bumps | keep `check` green; no new test required |
| **`none`** | docs, comments, config that nothing executes, research and review tasks | nothing about tests |

**`tdd` is the default for `kind = code`**; the planner opts out with a reason. Engine rules:

1. A code task whose `owns` touches the profile's `source` globs cannot be `none`.
2. A hub task is always `tdd`.
3. `tdd` requires a non-empty `single_test` in the profile; without one the task becomes `check` and its review moves up one size (§6).

### 8.1 The test proof

For a `tdd` task, `task_done` must name `test` (the test's identifier) and `red` (the commit where the test was added and failed). The engine then, in a scratch worktree:

1. runs `single_test` at `red`: it must **fail**;
2. runs it at the task head: it must **pass**, and the output must match `test_passed`, proving the named test actually ran.

This works in any language, including ones like Rust where unit tests live inside the source file, because it asks for a red *commit* rather than trying to separate test files from code. It is the same fail-to-pass rule SWE-bench uses, and it is what the project's own rule 6 asks of every change. It costs two test runs and no tokens. [E for fail-to-pass as a verifier; J for the red-commit form]

A proof failure is a gate failure like any other (§10). The reviewer's first job on a `tdd` task is to look for tests weakened or made trivial to pass the proof, the typical way agents fake TDD. [E]

## 9. Routing and review, by size

The roster gains a coarse **strength** per model: `fast`, `standard`, `frontier`. The profile or `config.toml` maps each tier to concrete models per runtime, so the policy survives model renames. [E — needed both for reviewer strength and to resolve the M8/M9 tier contradiction]

| | **S** | **M** | hub task |
|---|---|---|---|
| Worker | fast or standard tier, effort low–medium | standard or frontier tier, effort medium–high | frontier tier, effort high |
| Review | cheap diff-only review at low effort, other runtime | fresh-session review, other runtime, strength ≥ author, up to `max_bounces` rounds | as M, strength frontier |
| Bounces per gate | `max_bounces` (default 2) | `max_bounces` | `max_bounces` |
| Budget (placeholders, §15) | ~40 tool calls, 15 min | ~150 tool calls, 60 min | as M |

- **Review modifiers.** No `check` in the profile, or test mode not `tdd` on a source change → review one size up. `review.small = "off"` in config skips S review for users who accept that trade; the default keeps it, honouring the roadmap's promise that every task is reviewed by a different agent. [J]
- **The reviewer gets** the brief, the acceptance criteria, the diff and the check output — not the author's identity or transcript. Every critical or important finding must name a file and line, or a failing input. [E]
- **Severities.** `submit_review { verdict, findings: [{severity: critical|important|minor, file, line, text}] }`. Only critical and important findings consume a bounce; minor ones go in the run report. [E — reviewers over-flag, and one nitpick must not spend a task's retries]
- **Each review round is a fresh reviewer session.** Re-reviewing in the same session adds nothing; fresh context does. [E]
- **Runtime spreading.** The planner may spread tasks across Claude and Codex, which also draws on two independent subscription windows, but **never gives different runtimes overlapping `owns`**; plan validation rejects it. [E — 41.7% vs 19.8% conflict rate]
- The planner sets the route on every task; policy fills whatever it leaves out. A model's estimate of its own cost correlates at most 0.39 with reality, so workers never choose their own escalation. [E]

## 10. Escalation ladder

The engine drives it; no model decides when to escalate. [E — weak models do not recognise when to escalate; recovery-first beats always-escalate at 35% of the cost]

| Rung | Fires on | Action |
|---|---|---|
| **1** | first failure of any gate | same session, with the failure: the decider's ≤ 40-line check summary, or the critical and important findings |
| **2** | second failure, a stall (§11), or budget exceeded once | **fresh** session at effort +1, or on the peer runtime at the same strength, given the brief, the diff so far and the failure record |
| **3** | third failure, budget exceeded twice, a diff that spills outside `owns`, or `task_blocked{kind: mis_sized}` | task becomes `blocked(mis_sized)`; its size is raised; the orchestrator is told through the digest and splits or rewrites it |
| **4** | the task's cumulative spend reaches the next size's ceiling, or the orchestrator cannot resolve it | task becomes `blocked(human)`; the orchestrator raises it with the user in its window |

Rung 2 is a fresh session because changing model or effort mid-session invalidates the prompt cache and steers less reliably. [E]

**Rewriting a task at rung 3.** Rung 3 raises the size because the task as written did not fit. A rewrite (`amend_task` changing the brief, acceptance, size or route of a `blocked(mis_sized)` task) is the planner's new size claim: it may set S or M again, the §7.2 rules apply to the new values and can raise it again, and an accepted rewrite restarts the task at rung 2 with a fresh session. A task blocked at rung 4, on a conflict or on its environment is never restarted by an edit, only by `anthrex run retry`. *(settled in the briefs, 2026-09-22)*

`task_blocked` carries `kind: question | mis_sized | environment` and a reason. A decider classifies free-text reasons the worker did not type. A `question` goes to the orchestrator, which answers with an `answer` plan edit when the scout reports or plan cover it, and asks the user otherwise. An `environment` block (missing tool, failing setup) goes straight to the user.

The user can always `anthrex run retry <task>` a failed or blocked task, which re-enters it at rung 2.

## 11. Gates

### 11.1 Done

- **Primary signal:** the worker calls `task_done { summary, test?, red? }`. The engine rejects it with an actionable message unless the branch has at least one commit, the tracked tree is clean, and (for `tdd`) `test` and `red` are given.
- **Turn ends are exact.** A headless turn ends with a `result` event (Claude) or `turn.completed` / `turn.failed` (Codex). A rate-limit retry (`system/api_retry` with `error: rate_limit`) is a distinct `rate_limited` state, never an ended turn. Sub-agents still running are tracked from the stream and from `SubagentStart`/`SubagentStop` hooks, which still fire in `-p` mode, and a turn is not treated as finished while any is open. [E — `StopFailure` replaces `Stop` on API errors; `Stop` can precede background subagents]
- **Fallback:** a turn that ends with at least one commit and no `task_done` gets one nudge message (a new turn). If that turn also ends without `task_done`, the task is treated as done. The run report records which path fired.
- **Stall watchdog:** no stream event for `stall_after` while a turn is open → the process is interrupted, one nudge → `Stalled` → rung 2. Deadlines are persisted, so one that expired while the daemon was down fires on restart. [E — Symphony and Gas Town both needed this]

### 11.2 Test proof — §8.1.

### 11.3 Check

`check` from the profile, in the task worktree, with a timeout (default 30 min). Output is filtered and the decider summary is what a bounce carries.

### 11.4 Review, and what happens when a reviewer rejects

Routing, strength and severities are in §9. The reviewer returns `approve` or `changes`, with findings of severity `critical`, `important` or `minor`.

1. **`approve`, or `changes` with only minor findings** → the task goes to the merge queue. Minor findings are kept in the run report and shown in the task's inspector; they never cost a bounce.
2. **First rejection** (any critical or important finding) → escalation step 1. The engine sends the **same worker session** a message with only the critical and important findings, each with its file and line. The worker fixes them, commits and calls `task_done` again.
3. **Every gate runs again from the start**: done, test proof, check, then a **fresh reviewer session** (round 2). It gets the brief, acceptance criteria, the whole diff, the check output, and round 1's findings to confirm each is fixed. It is a new session because re-reviewing in the same context adds nothing. [E]
4. **Second rejection** → escalation step 2. A **fresh worker session** starts in the same worktree, on the same branch, at one effort level higher or on the other runtime. It gets the brief, the diff so far and both rounds' findings.
5. **Third rejection** → escalation step 3. The task becomes `blocked(mis_sized)`, its size is raised, and the orchestrator decides from the run digest: split the task, rewrite its brief or acceptance criteria (the next review then judges against the new ones), or raise it with the user.
6. **Disagreement.** A worker that believes a finding is wrong calls `task_blocked { kind: question }` instead of fixing it. The orchestrator can clarify the brief or acceptance criteria, but **cannot approve a task**: review authority stays with reviewers and the engine. Only the user can override a rejection, with `anthrex run override <task> --reason …`, which merges the task and marks it in the run report as merged without approval.

`max_bounces` (default 2) caps steps 2 and 4 for the review gate; the ladder in §10 counts failures across all gates, so one task that fails check once and review twice also reaches step 3.

### 11.5 Merge queue

Width 1. For each approved task, in completion order:

1. `git merge-tree --write-tree <run> <task>` computes the merge in memory.
2. Clean → `commit-tree`, materialise the candidate in the integration worktree, run `check` there.
3. Green → `git update-ref <run-branch> <candidate> <old>`, a compare-and-swap that fails if anything else moved the branch.
4. Conflict → **one hand-back**: the engine merges the run tip into the task's worktree, leaves the markers, and sends the worker the file list. The resolved task re-enters the queue. A second conflict → `blocked(conflict)` → the orchestrator.
5. Red on the candidate → rung 1 for that task, with the failure.

Testing the merged result, not the branch, is unanimous merge-queue practice. [E] The hand-back is [J]. The engine itself still never resolves a conflict.

**Branch names:** run branch `anthrex/<run>/integration`, task branches `anthrex/<run>/<task-id>`. A branch cannot be both `anthrex/<run>` and a parent of `anthrex/<run>/<task>`; the M8 refresh found this defect in the amended spec. [E]

### 11.6 Worktrees, branches and merges

Every agent that needs files gets a worktree; only task branches are ever merged, and only by the engine.

| Worktree | Branch | Who writes | Merged? |
|---|---|---|---|
| integration | `anthrex/<run>/integration` (the run branch) | the engine only, via the merge queue | into base, only on the user's accept |
| task | `anthrex/<run>/<task>`, created from the run branch **after its dependencies merged** | its worker sessions, one at a time | into the run branch, through the merge queue |
| race | `anthrex/<run>/<task>.a` / `.b` | one racer each | never directly; the winner's head becomes the task branch |
| review | detached at the task head | nobody (read-only) | no |
| proof | detached, scratch | the engine, for the test proof | no |
| scout, planner | none; they read the main checkout read-only | nobody | no |

- **One merge per task.** However many sessions worked on a task, their commits are on one task branch, and that branch is merged once, `--no-ff`, so `git merge-base --is-ancestor` answers "was this task merged?".
- **Sessions hand over by branch.** A fresh session at rung 2 starts in the same worktree on the same branch, with the diff so far in its prompt.
- **A race ends in one branch.** The first racer to pass every gate wins: the engine points `anthrex/<run>/<task>` at its head (a compare-and-swap ref update), stops the other racer, salvages its worktree to `refs/anthrex/salvage/…`, and removes it. The winner then enters the merge queue like any task. The loser always gets a salvage ref, pointing at its head even when its worktree is clean, because `run accept` deletes its lane branch with every run branch. *(settled in the briefs, 2026-09-22)*
- **Dependents start from merged work.** A task whose dependencies have merged branches from the run branch as it is then, so it builds on reviewed, checked code. (Starting from an unmerged dependency is the deferred stub-then-fill.)
- **A conflict goes back to the task's worktree** (§11.5, step 4): the engine merges the run tip into the task branch there, and the task's worker resolves it.
- **Cleanup.** When a task merges, its worktree and any race or review worktrees are removed after a dirty check. The task branch is kept until the run is accepted or discarded, so the run report can show each task's commits.

## 12. Plan edits and steering

### 12.1 One vocabulary for every change

The initial plan, a sub-planner's epic, the user's steering and the orchestrator's re-planning are all **plan edits**, validated and logged by the engine the same way:

`add_task`, `split_task`, `cancel_task`, `amend_task` (brief, acceptance, route, test mode, priority), `add_dep`, `answer` (to a blocked task), `pause`, `resume`, `finish`.

Validation runs every rule in §7, §8 and §9, plus: no cycles, no dependency on a cancelled task, `owns` inside the epic's area for a sub-planner. A rejected edit returns a structured error the model can fix and resubmit.

The orchestrator sends its edits as one batch through `edit_plan`, all accepted or none. The plan becomes "the plan" the gate shows when a batch sets `submit: true`; until then the run is in the **`planning`** state, with no task scheduled. *(settled in the briefs, 2026-09-22)*

### 12.2 Edits to running work

- Not-yet-started tasks: applied at once.
- `amend_task` or `answer` on a running task: delivered to the worker as its next turn's message as soon as the current turn ends.
- `cancel_task` on a running task: the worker is stopped, its worktree is **salvaged** to `refs/anthrex/salvage/<run>/<task>` before removal, and every task depending on it becomes `blocked(dep_cancelled)` for the orchestrator to re-plan.

### 12.3 The plan gate

- The initial plan, and any later edit that adds an epic, waits for the user's approval unless `--yes`.
- **Non-blocking:** the orchestrator's edit returns `{ awaiting_approval: true }` at once, and the verdict arrives through `run_status`. A tool call that blocks for hours cannot coexist with the 100-second reply timeout of `anthrex mcp` or Codex's 120-second tool timeout. This supersedes conversation-view decision 12. [J]
- The user sees the plan **in the run view** (§16) and may edit route, brief, size, test mode, and remove tasks. Adding tasks is left to the orchestrator, as conversation-view decision 13 argued.
- An unanswered gate survives a restart (conversation-view decision 15, unchanged).
- **Opening the gate.** A planned run starts in `planning`; `edit_plan {submit: true}` moves it to `awaiting_approval` (or straight to `running` with `--yes`). Submitting is refused while the plan has no task or a sub-planner is still planning. A new epic started while the gate is open returns the run to `planning`. *(settled in the briefs, 2026-09-22)*
- **Holds.** Tasks added after approval that need it wait in a **hold**, a named approval that blocks only those tasks while the rest of the run goes on: `epic:<e>` for a new epic's tasks, and `promotion` for the tasks an orchestrator adds to a promoted fast-path run. The user decides with `anthrex run approve <run> --hold <hold>` or `anthrex run reject <run> --hold <hold>` (which cancels the held tasks), or in the run view; the orchestrator reads the verdict through `run_status`, and completion waits while a hold is undecided. *(settled in the briefs, 2026-09-22)*

### 12.4 Talking to the orchestrator

The orchestrator is the only interactive window, so **steering is typing into it**: "skip the permission-prompt work", "do the TUI part first", "use Codex for the parser". It turns what the user says into plan edits. The same edits are available without it as `anthrex run edit …` for scripts.

**Wake-ups.** The digest is pulled, but an interactive session whose turn has ended pulls nothing. So when something the orchestrator must see happens (a block, a verdict, a user edit, a sub-planner or scout ending, an integration verdict, a halt, completion), the engine pastes one short `[anthrex] Run <id> changed: …` line into its window, only while the window is idle and the user has not typed into it for `wake_quiet_secs` (default 5 s), never into a busy window or a permission prompt. A `run_status` read clears pending wake notes, so an orchestrator that is already polling is never pasted at. *(settled in the briefs, 2026-09-22)*

## 13. Scheduling and speed

The honest limit first: wall-clock ≈ max(critical path × (work + check + review + merge), total work ÷ writers). With typical plans the critical path dominates, so **more writers or more planning levels do not make runs faster; a shallower graph does.** [E, derived] The speed levers, strongest first:

1. **Graph shape, set by the planner.** Interfaces first; no overlapping `owns`; chains collapsed (§7.3). [E]
2. **Critical-path dispatch.** Among runnable tasks, the one with the longest remaining downstream path runs first, weighted by the history median for its size. [E — within 1.03–1.11x of the bound on synthetic graphs]
3. **Separate reader and writer slots.** `max_writers` (default 3, range 1–8) caps workers; scouts, reviewers and deciders inside a run use `max_readers` (default 3) and never take a worker's slot. Triage runs before a run exists and takes no slot. [J] Sub-planners, research tasks and review tasks are readers too; a freed reader slot goes to deciders, then reviewers, then sub-planners, then scouts, then research and review tasks. *(settled in the briefs, 2026-09-22)*
4. **Hub tasks alone.** A hub task runs in its own wave with no other writer. [E]
5. **Implicit dependencies from `owns`.** Two tasks whose `owns` intersect never run together; the later one in plan order waits. (Unchanged from the amendment §5.5.)
6. **Pre-warmed worktrees.** Created, with `setup` run, while the plan gate is open. [J]
7. **The fast path** removes planning entirely for small goals (§5.1). [J]
8. **Two runtimes, two windows.** Claude and Codex draw on separate subscription limits.
9. **Adaptive concurrency.** A rate-limit event halves that runtime's writer cap (never below 1); every `recover_after` minutes (default 10) without one adds one back, up to `max_writers`. [J, additive-increase / multiplicative-decrease] Each runtime has its own cap inside the run's single `max_writers`: a task takes a writer slot only when both allow it. A rate-limit event is a retry streak or a turn that fails on a rate limit, from any session of the run, readers included; several within `halve_hold_secs` halve once. *(settled in the briefs, 2026-09-22)*

Useful fan-out clusters at 3–5 writers; the plan gate and `run accept` usually bind before `max_writers` does. [E]

Racing (§4.1) is opt-in per task and off by default: it doubles the tokens of the raced task to cut its tail latency, and no coding-specific measurement of the gain exists. Deferred until real runs show the need: starting a dependent task from its interface task's head before it merges (stub-then-fill), and a merge queue wider than 1.

## 14. Token efficiency

Ordered by leverage. [E unless marked]

1. **Scout once, share the map.** Scout reports go into every relevant brief, so workers do not each re-explore.
2. **Stable prompt layout.** Worker prompt, in this order: the fixed role contract, the profile summary, the scout extract for its area, then the task brief last. Model and effort are pinned at spawn and never changed within a session. Claude's prompt cache is scoped to one directory, so each worktree pays one cold prefix; the layout keeps that prefix short and every later turn cached.
3. **Filtered tool output.** A `PreToolUse` hook, injected with `--settings` (hooks still run in `-p` mode), pipes test output through the profile's `output_filter`, cutting logs "from tens of thousands of tokens to hundreds". Bounce messages carry the decider's ≤ 40-line summary, not raw logs.
4. **The orchestrator reads a digest.** `run_status` returns task states, gate outcomes, blocked reasons, diffstats and budget use — structured and small. It never reads a transcript or a checkout.
5. **Deciders for narrow judgements**, so triage, size checks and failure summaries never enter the orchestrator's context. [J]
6. **Size-matched route and effort** (§9). Lower effort means fewer, terser tool calls, not only less thinking.
7. **Runaway stops.** Repeated identical tool calls (visible in `PreToolUse` payloads) or spend past 2x the size median → one "wrap up" message → rung 2. [J]
8. **Metering.** Every headless turn reports its usage in the stream: Claude's `result` event, Codex's `turn.completed`. That gives per-task input, output and cache tokens with no extra machinery. The orchestrator, the one PTY session, is metered through Claude Code's OTLP export with `anthrex.run` and `anthrex.role` resource attributes. The exact usage field names are pinned in M8a's first task; M8b adds the OTLP receiver, which has nothing to meter until M9 creates the orchestrator window. This supersedes the product design's premise that interactive spend cannot be seen (L504). A **Codex orchestrator is not metered**: the run report says so, and metering it is a follow-up. *(settled in the briefs, 2026-09-22)*

Budgets are enforced by count, never by prediction: token use on the same task varies up to 30x. [E]

## 15. Learning from run history

Every finished task appends one record to `history.jsonl` in the repository's data directory:

predicted size and route · actual files, hunks and lines changed · tool calls · tokens by type (where metered) · wall-clock per phase (queued, working, check, review, merge) · gate outcomes · review severities · bounces · highest rung · which done signal fired · and, appended later as a separate `revert` record, whether the user reverted it.

- **Actual size is measured by diff**, not tokens.
- After `adapt.min_samples` tasks (default 30) of a size, the engine computes new line thresholds and sets each budget to 2–3x the observed median.
- **Budgets refit automatically. Size thresholds and routing are proposed** in `anthrex run stats` and applied when the user confirms, because they change what the planner is allowed to do. [J]
- **Refit budgets come from per-task totals, while a budget counts one session.** A task's tool calls and working time are summed over its sessions, so an escalated task pulls the median up; that errs toward a looser budget, which only delays a wrap-up message, never sends correct work up the ladder. Token budgets are not refitted unless configured. A class needs `min_samples` merged, unreverted tasks, at most 10 per run, from the newest 200. Everything learned is frozen into a run when it starts. *(settled in the briefs, 2026-09-22)*
- **Line thresholds reach prompts only.** No engine rule counts lines (§7.2 uses modules, hubs and interface changes), so a new threshold changes what deciders and planners are told an S or M task is, nothing else. *(settled in the briefs, 2026-09-22)*
- A learned router is out of scope until there are hundreds of records; routers trained on single-turn chat have not been shown to transfer to coding agents. [E]

## 16. The live run view in `C-b T`

A run is watched in the **expanded graph overview that already exists** (`C-b T`, milestone 4.6): the left-to-right, node-and-edge canvas with the node inspector below it (milestone 4.7). It is extended, not replaced, and it is live: every task state, agent start, review verdict and merge redraws as it happens. It is also the plan gate's view before the run starts. [J — the canvas already lays out any tree left to right, and `crates/tui/src/tree.rs` already reserves `NodeKey::Run` and a run row for milestone 8]

### 16.1 Everything starts at the orchestrator node

In the project overview, a run appears as **one node: its orchestrator**, under the project, above the project's plain windows. Selecting it and pressing `l` or Enter **opens the run view**, a canvas whose root is the orchestrator. `h` at the root, or Esc, returns to the project overview.

The run view's tiers, left to right, follow the information flow of §3: who planned a node is its parent, and the agents doing a task's work are its children.

```
 tier 0          tier 1              tier 2                  tier 3                    tier 4
┌──────────────┐ ┌─────────────────┐
│◉ orchestrator├┬┤✓ scout S1       │
└──────────────┘│└─────────────────┘
                │┌─────────────────┐
                ├┤✓ scout S2       │
                │└─────────────────┘
                │┌─────────────────┐ ┌─────────────────────┐ ┌───────────────────┐
                ├┤✓ t0 proto  M ◆  ├┬┤✓ worker #1 claude   │ │                   │
                │└─────────────────┘│└─────────────────────┘ │                   │
                │                   │┌─────────────────────┐ │                   │
                │                   └┤✓ review #1 codex    │ │                   │
                │                    └─────────────────────┘ │                   │
                │┌─────────────────┐ ┌─────────────────────┐ ┌───────────────────┐
                ├┤✓ planner A dmn  ├┬┤● t1 spawn  M  ⇠t0   ├─┤● worker #1 claude ├─ ┌ sub-agent Explore
                │└─────────────────┘│└─────────────────────┘ └───────────────────┘
                │                   │┌─────────────────────┐ ┌───────────────────┐
                │                   └┤✗ t2 status M  ⇠t0t6 ├┬┤✓ worker #1 codex  │
                │                    └─────────────────────┘│└───────────────────┘
                │                                           │┌───────────────────┐
                │                                           ├┤✗ review #1 claude │
                │                                           │└───────────────────┘
                │                                           │┌───────────────────┐
                │                                           └┤● worker #1 r2     │
                │                                            └───────────────────┘
                │┌─────────────────┐
                └┤✓ t6 fake-agent  │
                 └─────────────────┘
```

- **Tier 1:** the orchestrator's scouts, its sub-planners, and the tasks it planned itself (interface and hub tasks).
- **Tier 2:** each sub-planner's tasks.
- **Next tier:** a task's agents, in the order they worked: worker sessions, test writer, reviewer rounds, racers.
- **Last tier:** their sub-agents, which the existing tree already tracks.

A run view is five tiers deep, plus however deep sub-agents nest.

**Review calls are nodes.** Each review round is its own node under the task, placed after the worker round it judged, with its verdict as its glyph (`✓` approved, `✗` changes, `●` reviewing). When a worker is sent back, its next round appears after that review (`worker #1 r2`), so the task's children read as the loop that actually happened: work, review, fix, review. A fresh session after an escalation is a new node (`worker #2`).

Finished agent nodes are drawn dim, never removed.

**Fast-path runs** have no orchestrator. Their root node is the run itself, with the single task under it.

### 16.2 Node kinds and ordering

- **New node keys:**
  - `NodeKey::Run`: the orchestrator node, in the project overview and as the run view's root.
  - `NodeKey::Planner`: a sub-planner.
  - `NodeKey::Scout`.
  - `NodeKey::Task`.
  - `NodeKey::AgentRound { run, task, role, session, round }` (the run id, because two runs can both have a task `t1`): one worker, test writer, reviewer or racer round. It points at its headless session, which the daemon registers as a window with no terminal, so its live status, sub-agents and conversation all come from what milestones 3 to 6.5 already provide.
- **Layout:** the existing overview layout is used unchanged, so a parent is centred on its children; the §16.1 diagram is schematic.
- **Order among siblings:** tasks under the same planner are ordered by **wave** (the longest dependency chain before them), then by plan order, so the canvas reads top to bottom roughly in the order work happens. Agent rounds are ordered by start time.
- **Finished sub-planners:** a sub-planner node stays after its session ends, as a finished grouping.

### 16.3 Live state on the canvas

Node boxes keep the overview's one content row. What changes is what that row carries:

| Node | Content row |
|---|---|
| orchestrator | `◉ orchestrator  5/9` (merged of total) |
| scout | status glyph, question |
| sub-planner | status glyph, its area, `3/4` merged of its tasks |
| task | lifecycle glyph, id and title, size, `◆` for hub, `⇠` dependencies |
| agent round | status glyph, role and number, runtime badge |

Lifecycle glyphs, in their status colours: `○` pending, `◌` waiting on dependencies, `▫` queued for a slot, `●` working (animated while output is flowing), `◇` test proof or check, `◐` in review, `▸` in merge queue, `✓` merged or approved, `✗` a rejecting review round or a failed agent round (a task itself never shows `✗`: a failed gate sends it back to working or to blocked), `⊘` blocked, `–` cancelled.

- **Critical path:** tasks on it have a bold border.
- **Dependencies:**
  - Selecting a task highlights the tasks it waits on and the tasks waiting on it, and dims the rest.
  - Dependency lines are not drawn across the tree, because every crossing edge would fight the tree's own connectors. The `⇠` list and the highlight carry them instead. [J]
- **Filters** (added to the existing one): running only, blocked only, one runtime.

### 16.4 The inspector: insight and progress for the selected node

The inspector keeps milestone 4.7's collapse on a short terminal and `i` to toggle, but run-view nodes show **one labelled field per row** with a 10-column label, as the mockups below do, not M4.7's packed columns.

- **Height:** it grows to `RUN_INSPECTOR_HEIGHT` = 12 rows for run-view nodes, when the terminal has room, because a task's progress does not fit in five field rows.
- **First row:** for the orchestrator, sub-planners and tasks, the row under the title is a **progress line**.
- **Tasks with a long history:** the last row lists the most recent events, newest first.

**Orchestrator (the run):**

```
◉ r1  Add Gemini runtime                                 running · 1h12m
progress  ██████████░░░░░░░░  5/9 merged · 2 working · 1 review · 1 blocked
path      critical path t0 → t6 → t2 → t3 · 2 tasks left · 1.2× the bound
agents    workers 3/3 · readers 1/3 · claude ok · codex rate-limited 4m
spend     tokens 1.8M (cache 71%) · tool calls 612 · est. left ~40m
gate      plan approved 11:02 · 2 plan edits since · last: split t2
attention t5 blocked: question — "Gemini has no subagent-stop event"
```

**Sub-planner:**

```
✓ planner A  daemon                       finished · planned 3 tasks in 2m
progress  ███░░░░░░░  1/3 merged · 1 working · 1 waiting
area      crates/daemon/** · crates/cli/src/hook.rs
edits     3 accepted · 1 rejected (owns outside area) · re-planned once (t2 split)
```

**Task:**

```
✗ t2  map Gemini hook events to status          M · tdd · review round 2
stages    done ✓ → proof ✓ → check ✓ → review ✗ → merge ·
route     codex · standard · high effort  →  reviewer claude · frontier
deps      waits on t0 ✓ t6 ✓ · unblocks t3, t7 · on critical path
budget    ███████░░░ 104/150 tool calls · 38/60 min · 410k tokens
tries     review 1/2 bounces · check 0/2 · escalation step 1
diff      4 files · +212 −31 · test `status::gemini_stop_marks_idle` red a1b2c3d ✓
review    r1 ✗ 1 critical, 2 minor: status.rs:118 "SubagentStop not paired"
history   12:31 review r1 changes · 12:20 check passed · 12:02 started
```

**Agent round (worker, test writer, reviewer or racer):**

```
● worker #1 r2  codex · standard · high           working · 6m · t2
doing     last tool: apply_patch
activity  turns 14 · tool calls 41 · 3 commits · tokens 180k
fixing    status.rs:118 critical — SubagentStop not paired
session   #7 · headless · worktree anthrex/r1/t2 · Enter: conversation
```

For a reviewer, the fields are the round, the author it is judging, its strength against the author's, the verdict, and its findings counted by severity with the most severe in full.

**Scout:** its question, its state, how long it has taken, the report's size, and the files it named.

**Sub-agent:** unchanged from milestone 4.7.

**The bound** in the run's `path` line is §13's limit with history medians standing in for work, check, review and merge: `bound = max(critical path of the class weights, Σ weights ÷ max_writers)` over the run's non-cancelled tasks, and the ratio is `(active time since approval, pauses excluded, + estimated time left) ÷ bound`. Both are absent until history supplies weights. *(settled in the briefs, 2026-09-22)*

Every value comes from the run snapshot (§16.5), which carries the daemon's clock, approval time, plan-edit count, rate-limit start and bounce times for exactly this purpose, and from the window data milestone 4.7 already reads. Before M9 there is no orchestrator window: Enter on the run's root shows a short notice instead. The inspector does no I/O and derives nothing that the snapshot does not carry, as AGENTS.md rule 5 requires.

**Navigation.** Enter on the orchestrator node focuses its window, the one place the user types. Enter on any other agent node opens its conversation view (milestone 6.5), which shows its messages, tool calls and results live; there is no terminal to show, because it is headless (§4.2). Enter on a task opens its current agent's conversation.

### 16.5 How it stays live

- **Push, not poll.** The daemon keeps one run snapshot with a **revision counter**. It pushes a change to subscribed clients whenever any task, gate, agent round or budget counter changes, the same pattern as conversation-view decision 8. Budget counters are coalesced to at most one push per second.
- **Client side.** The client rebuilds its rows when the snapshot changes. The overview's layout is already a pure function of the rows. Elapsed times and the working animation tick on the client's clock between pushes. Nothing polls.
- **Scripts.** `anthrex run status --json` exposes the same snapshot.

## 17. Durability and safety

- **Intent log.** Every side-effecting engine step writes `Intent{op_id, kind, params}` and fsyncs before acting, and `Done{op_id}` after. Every step is idempotent, keyed by run, task and attempt. [E]
- **Reconcile on start.** Before anything else, each unmatched intent is checked against reality — `git worktree list --porcelain`, refs, headless process liveness — and completed or rolled back. Non-terminal runs come back Paused, as today. [E]
- **Git safety.** Engine git writes go through one queue per repository with bounded retry on `*.lock` errors; live task worktrees are `git worktree lock`ed; sessions launch with jitter. Every git call keeps `--no-optional-locks` and the scrubbed environment of AGENTS.md rule 11. [E — 8 of 13 parallel agents once failed to commit on lock contention, and cleanup then deleted their work]
- **Nothing is deleted dirty.** Discard, cancel and cleanup salvage any dirty worktree to `refs/anthrex/salvage/…` first. [E]
- **The run branch is guarded; the base branch is watched.** At every merge step and before `Complete`, the engine checks both refs. The run branch is written only by the engine: if it differs from the recorded head, the run halts. The base branch may move while a run works, because people keep committing to it: if it only advanced (the recorded base commit is still its ancestor), the run keeps building on its recorded base, records what it saw and shows it as an attention line; at accept the user is shown every commit on the base since the run started, with authors, and must confirm them before the run branch is merged onto the current base with `--no-ff`. A conflict at accept aborts the merge cleanly, leaves the base untouched and tells the user; the run stays complete. If the base was rewritten (the recorded base commit is no longer its ancestor) or deleted, the run halts, because accept could no longer be a clean merge onto what the run was built from; `run resume --rebaseline` continues it. A `reference-transaction` hook rejecting writes to protected refs is an optional second layer; it can likely be bypassed, which is why the ref check is mandatory. [E] *(settled 2026-09-22, §22 item 5)*
- **Scrubbed agent environment.** Per-worktree variables from the profile; inherited `CLAUDE_CODE_*` session variables removed; `--settings`, `--mcp-config` and the role flags re-passed on every resume, because resume does not restore them. [E]
- **Headless sessions survive a daemon restart by resuming.** A session's id is persisted; after a restart the engine resumes it (`claude -p --resume <id>`, `codex exec resume <id>`) with a hand-over message rather than starting over.

## 18. What this supersedes

| Document and section | Status |
|---|---|
| Amendment §5.2 run and task | replaced by §4, §7–§9 and the task record below |
| Amendment §5.3 lifecycle ("finished" = idle + commit; one bounce per gate; `--no-ff` merge) | replaced by §10–§11 |
| Amendment §5.4 reviewer selection | replaced by §9 |
| Amendment §5.5 parallelism | extended by §13; the `owns` rule is kept |
| Amendment §5.6 finishing | kept, plus the integration step of §5.3 |
| Amendment §5.7 tools | replaced by §19 |
| Amendment §5.8 planning contract (TDD in every worker's prompt) | replaced by §8 |
| Amendment §5.9 routing | extended by §9: routing stays a task field; policy fills gaps |
| Amendment §5.10 cuts | revised: bounded re-planning is in (§10, §12); orchestrators spawning orchestrators stays out — sub-planners only plan; token accounting is in (§14); automatic conflict resolution stays out (the hand-back asks the worker) |
| Conversation view decision 12 (blocking `submit_plan`) | replaced by §12.3 |
| Conversation view decision 13 (three editable fields) | extended by §12.3 |
| Product design L504 (spend cannot be seen) | superseded by §14.8 |

### The task record

```rust
struct Task {
    id, title, epic: Option<EpicId>, kind: Kind,          // code | docs | research | review
    size: Size,                                           // S | M; L only inside the planner
    interface_change: bool,                               // the planner's flag for §7.2 rule 2
    hub: bool,
    test_mode: TestMode, test_mode_reason: Option<String>,
    owns: Vec<Glob>, deps: Vec<TaskId>, priority: i32,    // owns empty for research and review
    brief: String, acceptance: Vec<String>, test_to_write: Option<String>,
    scout_refs: Vec<ScoutReportId>,
    review_target: Option<String>,                        // review tasks only: a revision or <a>..<b> (§5.2)
    race: bool, pair: bool,                               // opt-in patterns of §4.1, off by default
    route: Route,                                         // runtime, model, strength, effort
    budget: Budget,                                       // tool calls, wall-clock, tokens
    hold: Option<HoldId>,                                 // set while it waits for approval after the gate (§12.3)
    state: TaskState, rung: u8, bounces: GateCounts, history: Vec<TaskEvent>,
}
```

`interface_change`, `review_target`, `hold`, `race` and `pair` were added in the briefs (M8a, M9, M9.5), 2026-09-22. `TaskState` gains `reported` (§5.2), and the run state gains `planning` (§12.3).

## 19. Tools per role

| Role | MCP tools |
|---|---|
| Orchestrator | `get_context` (roster with strengths, profile, scout reports), `spawn_scout`, `spawn_subplanner`, `edit_plan` (a list of plan edits), `run_status` (long-poll up to 50 s), `task_result` |
| Sub-planner | `get_context`, `submit_epic` (plan edits limited to its epic and area) |
| Scout | `submit_scout_report` |
| Worker | `task_done`, `task_blocked` |
| Reviewer | `submit_review` |

Deciders have no tools; they return schema-validated JSON. Every new message bumps `PROTO_VERSION` and gets a round-trip test (AGENTS.md rule 4).

Racers and test writers use the worker's tools. A test writer reports with `task_done` naming `test` and `red`, and `red` must be its last commit (the branch head); the engine then checks that the test fails at `red` before an implementer starts. A research task's scout reports with `submit_scout_report`. *(settled in the briefs, 2026-09-22)*

## 20. Evidence, in brief

- Central coordination vs independent agents: 4.4x vs 17.2x error amplification; +80.8% on parallel tasks, −39% to −70% on sequential ones — Google/MIT, arXiv 2512.08296.
- Hierarchy at coding scale: Cursor, "scaling agents" (recursive planners, flat workers, integrator agent abandoned); Claude Code agent teams forbid nesting; Devin one coordinator over ~10 children.
- Writes single-threaded, extra agents contribute intelligence — Cognition, "multi-agents working" (2026).
- Deterministic daemon, stall timeout, reconcile before dispatch — OpenAI Symphony SPEC. Explicit done, watchdog, gated merge queue — Gas Town.
- Hub-isolating partitioning: 2.10x wall-clock, +14% pass rate, −35% cost — Co-Coder, arXiv 2606.00953.
- Cross-agent merge conflicts 41.7% vs 19.8% — arXiv 2607.04697.
- Size estimates: 6.6–36% bucket accuracy, systematic overestimation, correct ordering — BRIDGE, arXiv 2602.07267. Multi-file cliff — SWE-bench Verified annotations.
- Scout-then-route at one fifth the cost per solve — arXiv 2608.04804. Recovery-first routing at 35% of the cost — arXiv 2607.19338 (abstract only).
- Fresh-context review beats same-session review; repetition adds nothing — arXiv 2603.12123. Reviewers over-flag — arXiv 2603.00539.
- Failure taxonomy: 44.2% specification and design, 12.4% premature completion — MAST, arXiv 2503.13657.
- Token spend: input and re-reading dominate; up to 30x variance — arXiv 2604.22750. Claude Code costs, prompt caching (cache per directory), hooks, monitoring (OTel) docs.
- Merge queues test the merged result — GitHub merge queue, Zuul; `git merge-tree --write-tree`, git ≥ 2.38.

## 21. Milestones

| Milestone | Scope | Testable without a model? |
|---|---|---|
| **M8a — engine core** | task graph, plan edits and validation, intent log and reconciliation, headless sessions, scheduler with reader/writer slots, gates (done, test proof, check, review, merge queue), escalation ladder, budgets, salvage and ref guards, run snapshot with revision counter | yes, with `fake-agent` |
| **M8b — adaptation plumbing** | repo profile and onboarding scout, the scout service, deciders with fallbacks, triage and the fast path (`run promote` recorded), output-filter hook for Claude workers, OTLP metering, `history.jsonl` and `run stats` | yes, with a scripted decider binary |
| **M8c — the live run view** | §16: the orchestrator-rooted run view in `C-b T`, agent-round and review nodes, the run inspector, the plan gate's approve, reject, edit and remove, reusing the graph overview, node inspector and conversation view | yes, client-only rendering tests |
| **M9 — orchestrator and sub-planners** | the orchestrator (the one interactive window) and its contract, steering and wake-ups, run scouts, sub-planners, the `planning` state, submitting and holds at the plan gate, `run promote` performed, research and review tasks, per-epic integration review | contract tests and scripted `fake-agent` orchestrators, plus a manual run |
| **M9.5 — tuning** | threshold and budget refitting, adaptive concurrency, the run estimate and bound, racing, the test-writer-then-implementer pattern, the output filter for Codex workers | yes, from recorded history and `fake-agent` |

M8a comes first because every other piece stands on it. M8b and M8c each need only M8a and may land in either order, one after the other, because both bump the protocol version. Each milestone adds one PTY smoke stage: M8a `11c`, M8b `11d`, M8c `11e`, M9 `11f`, M9.5 `11g`. *(M9.5's racing and pairing, and the stage letters, settled in the briefs, 2026-09-22.)*

`fake-agent` must learn the new worker, scout, reviewer and sub-planner tools and be able to produce a red commit, a weakened test, a stall, a rate-limit stop and a conflict. Deciders are replaced in tests by `ANTHREX_DECIDER_BIN`, like the existing `ANTHREX_CLAUDE_BIN`.

### Scenarios every milestone's tests must cover

A green S task on the fast path · a TDD task whose red commit does not fail (proof rejected) · a TDD task whose test passes without the named test running · a task that fails check once, then passes · a stall → rung 2 on the peer runtime · a mis-sized task → split by the orchestrator · a merge conflict → hand-back → resolved · a second conflict → blocked · a candidate merge that is red although both branches were green · a cancel of a running task with dirty work (salvaged) · the run ref moved behind the engine's back (run halts) · the base branch advanced during a run (the run continues; accept lists the new commits for confirmation) · the base branch rewritten during a run (run halts) · a daemon killed after each logged intent (reconcile lands in the same state) · a rate-limit event halving writers · a race whose loser is stopped and salvaged · a test writer's red commit handed to an implementer · a review rejected twice → fresh worker on the peer runtime → approved · a worker that disputes a finding (`task_blocked: question`) · a user override merged without approval, marked in the report · input, kill and terminal subscribe to a headless window refused by the daemon while engine messages still arrive · a rate-limit retry in the stream (rate_limited, not a finished turn) · repeated permission denials → blocked(environment) · a headless process that dies mid-turn → resumed · every session resumed after a daemon restart · a plan edit that leaves an L task (rejected) · a sub-planner edit outside its area (rejected) · Claude and Codex given overlapping `owns` (rejected).

## 22. Decisions for the author

1. ~~Headless agents and deciders on the user's login~~ — settled 2026-09-22: **yes**. They run on the user's subscription login by default (`[orchestrator.claude] auth = "login"`); `auth = "api_key"` stays available for the day `--bare` becomes the default (§23).
2. ~~S review default~~ — settled 2026-09-22: **on**. Small tasks get the cheap, diff-only, low-effort review of §9. The gates that actually catch most S defects are the test proof and the check; the review costs little and keeps the roadmap's promise that every task is reviewed by a different agent. `review.small = "off"` remains for users who accept the trade.
3. ~~`planner_task_cap`~~ — settled 2026-09-22: **12**. A plan of up to 12 tasks is one the orchestrator can hold in context and the user can actually read at the plan gate; beyond that the large path and sub-planners take over. No source measures the right number, so M9.5's history records plan sizes against outcomes and can propose a different value.
4. ~~Profile location~~ — settled 2026-09-22: data directory only, no repo-level file (§6).
5. ~~Base branch moving during a run~~ — settled 2026-09-22: continue, list the new base commits at accept for confirmation; a rewritten base still halts (§17).
6. ~~Chain collapse enforcement~~ — settled 2026-09-22: the orchestrator decides; no engine check (§7.3).

## 23. Risks

- **Codex workers in linked worktrees may not be able to commit.** A linked worktree's index, refs and objects live under the main repository's `.git`, outside what Codex's `workspace-write` sandbox makes writable. M8a adds that directory as a writable root and proves a commit works in its first task.
- **Headless Claude would run the repository's own hooks and MCP servers without a trust dialog** (`-p` without `--bare`). §4 contains it by loading only the user's settings, or refusing untrusted project settings without `--trust-project`.
- **A daemon restart ends every headless session**, because the daemon owns their pipes. They resume from their session ids, and a resumed Claude session may repeat its last tool call.

- **`--bare` may become the default for `claude -p`.** Anthropic's docs say it will. Bare mode never reads the subscription login, so on that day headless Claude sessions need `ANTHROPIC_API_KEY` or an `apiKeyHelper`. Mitigation: pass an explicit opt-out if the CLI offers one (checked in M8a's first task), and a config switch `[orchestrator.claude] auth = "login" | "api_key"`. Codex headless sessions are unaffected.
- **The stream-json input format is less documented than the output.** M8a's first task pins the exact user-message envelope against the installed CLI and a recorded fixture.

- **The engine grows.** This is several times the amendment's engine. The pure reducer and `fake-agent` scenarios are what keep it testable; if a milestone cannot test a transition without a model, the transition is wrong.
- **The test proof depends on the profile.** A wrong `single_test` or `test_passed` rejects correct work. The onboarding scout must prove both commands ran, and a proof failure message must show the command and its output.
- **Deciders are a second model surface.** Every one has a deterministic fallback; the run report says when a fallback decided.
- **Thresholds are placeholders.** Size lines, budgets, stall timings, `max_bounces` and `planner_task_cap` all come from judgement. The history log exists so the first few dozen runs replace them.
- **Review quality is still unmeasured.** Reverts after accept, recorded in history, are the only true signal.
- **Rate limits bind before CPU does.** On subscriptions, parallelism is limited by a shared rolling window; adaptive concurrency keeps runs moving but cannot make them fast when the window is exhausted.
