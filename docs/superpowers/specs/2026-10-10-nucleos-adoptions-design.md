# Adoptions from nucleos: hardening, build economy, lessons and brakes

Date: 2026-10-10. Status: approved in conversation, section by section; briefs M9.12–M9.15 implement
it.

## 1. Origin and goal

The user wants to develop anthrex with anthrex. An in-depth read of nucleos
(github.com/zDud4s/nucleos, a Rust daemon that runs Claude Code and Codex unattended) compared each
of its subsystems against anthrex's `main` at protocol 20/21. Most of nucleos's machinery (its job
state machine, departments, git queue, lexical risk classifier, test selection, flake and bisect
handling) is already in anthrex in a stronger form, usually because nucleos works in one shared,
mostly unsandboxed checkout. What nucleos has and anthrex lacks is what weeks of unattended work
need:

- **closed holes in orchestrator autonomy** (9.9 granted "Everything") and in what reaches CI;
- **build cost** when many checkouts build one Rust workspace at once;
- **learning in words**, not only in refit numbers;
- **brakes** that stop new work without killing work in flight.

Success: anthrex can run goals on anthrex itself unattended for long stretches without (a) an agent
being able to change what judges it or what runs outside the sandbox, (b) every checkout building
dependencies cold, (c) repeating the same failure run after run, or (d) draining the user's
subscription window or one session's context.

### 1.1 Decisions from the user

| # | Decision |
|---|---|
| U1 | Four milestones, in order: 9.12 hardening, 9.13 build economy, 9.14 lessons, 9.15 brakes. One spec and four briefs, reviewed once; then build each with subagent-driven TDD. Milestone 9.11 (Stabilization, protocol 21, another stream of work) lands first; these start after it merges, so 9.12 is protocol 22, 9.13 is 23, 9.14 is 24, 9.15 is 25. |
| U2 | Orchestrator autonomy stays as 9.9 set it, except for the **retry cap** and the **untrusted-text downgrade**. Rejected: shadow mode per op class; a run or chain spending allowance (M9.3 D5 stands). |
| U3 | A task may change a gate script only when its `owns` names the file exactly; the user approves that with the plan, and that file is not pinned for that run. |
| U4 | The untrusted-text flag is sticky for the orchestrator session; there is no clear command. A fresh session starts clean. |
| U5 | `npm test` / `yarn test` do not pin `package.json`. |
| U6 | The target-directory pool is on by default when the stored profile's env sets `CARGO_TARGET_DIR` with `{worktree}`: `max_writers + 2` task dirs, 2 engine dirs, 20 GB cap per dir, idle dirs wiped after 7 days. |
| U7 | Build admission applies to headless workers only (an opt-in key for interactive windows, default off); the longest wait is 300 s, then the command runs anyway. |
| U8 | Per-module test cache keys, with tier 3 as the backstop. |
| U9 | Episodic lessons enter prompts on trial without approval; semantic and procedural lessons need approval. Proposals show on the Profile page and in the CLI only. No `lessons add`. Lessons stay per repository and reach workers and reviewers only. |
| U10 | The pre-review risk note runs for every reviewed task except review level `small`. |
| U11 | The quota brake is on by default at 90% (5 h) and 95% (7 d), fed only by what the CLIs already emit; the daemon never reads an OAuth token or the Keychain. |
| U12 | The global hold stops new sessions only; merges and delivery continue. |
| U13 | The orchestrator hands off to a fresh session at the next goal when its context reaches 80% of the window or 300,000 tokens, whichever comes first (0 disables). A Codex orchestrator hands off too when its rollout gives a fill. |
| U14 | A knowledge graph page in the TUI, Obsidian-style: lessons as dots around module hubs, with search, an inspector and removal (§4). |

## 2. Hardening (9.12)

**Finding.** The orchestrator's `override` skips the protected-file check entirely
(`engine/done.rs` `spill_exempt`), and the merge queue checks nothing, so since 9.9 an orchestrator
can merge a branch that changed `AGENTS.md`. The built-in protected list covers only agent
configuration, while in `pr` mode anthrex pushes worker branches to the same repository, where a
changed `pull_request` workflow runs with the repository's secrets.

- **Files that run later.** A second built-in protected list: CI configuration (GitHub workflows and
  actions, GitLab, CircleCI, Buildkite, Jenkins, Azure), git hook managers (`.githooks`, husky,
  pre-commit), `.cargo/config*` and `rust-toolchain*`. Dependency manifests and `build.rs` are not
  built in. A literal `owns` grant is approved with the plan. The orchestrator cannot override a
  protected change, cannot add a literal grant after the plan gate, and cannot approve a hold that
  contains one; each becomes user-only.
- **The gate's own scripts.** At run start every tracked file named as a word of the profile's gate
  commands (plus `Makefile`/`justfile`/`tox.ini`/`noxfile.py` for those tools) is pinned by blob id at
  the base commit. Before each gate command the checkout's copy is hashed; a mismatch, missing file
  or symlink refuses the gate and halts the run for the user. U3 applies.
- **Retry cap.** `orch_retries` per task, never cleared by retry or rung 2; at 2 the next block is
  user-only and a third orchestrator retry is refused. A user retry resets it.
- **Untrusted text.** The daemon (not the stateless MCP forwarder) records the first time the
  orchestrator reads quoted PR or CI text, or results from a task that addresses review comments.
  From then on (U4) an `override` or `accept_red` becomes an `ask_user` carrying the proposal, which
  the user's "do it" executes as the user; `approve_hold` is refused. The flag follows the session
  to the chain's next goal.

## 3. Build economy (9.13)

- **Target-directory pool.** Exclusive leases from checkout creation to removal, with affinity to
  the directory a checkout built last. On an owner change the workspace's own path packages are
  cleaned (`cargo clean -p` over every `source == null` package) so dependency artifacts stay warm;
  any failure wipes the directory. Task and engine directories never cross. No free directory
  means today's cold per-worktree directory; nothing waits.
- **Admission for agents' builds.** The worker sandbox cannot reach the daemon, so a PreToolUse hook
  (which runs outside it) blocks heavy `cargo` calls until the daemon's slot scheduler grants them in
  a new lowest class, and releases them on the matching PostToolUse or session end. It fails open
  after the wait. Codex has no blocking hook and keeps the thread caps of M9.1 decision 27. The
  milestone opens with a fact check of these hook behaviours against the installed Claude Code.
- **Per-module cache key.** Module test steps key on an inputs id over the files of the step's module
  closure plus every file in no module; the build step and tier 3 keep the tree id.

## 4. Lessons (9.14)

- **Failure signatures**, model-free: normalised gate output, at most three decisive lines,
  hashed and recorded on the task record. A signature in two or more runs within 14 days is a known
  failure; five clean runs over the same area or 90 days retire it. Computed on read.
- **Distiller.** After a closed set of outcomes, a daemon-wide background worker asks the helpers
  model once over a bounded, redacted dossier for 0–5 lessons. Duplicate titles reconfirm; rejected
  titles never return. Episodic lessons go on trial (U9); others wait for approval.
- **Selection.** A frozen, bounded, advisory block (4000 characters, 5 items) in worker and reviewer
  prompts, ranked by file overlap with `owns`, usefulness and recency. No lessons: prompts unchanged.
- **Credit.** Tasks record the lessons they were shown; merged-and-not-reverted counts green, a
  block counts against.
- **Discard reason** and a **pre-review risk note** written from the brief alone (U10).
- The store is an append-only log in the repository's data directory, written only by the daemon.
- **Knowledge graph page** (U14). An Obsidian-style view in the TUI, not the M4.6 overview: braille
  dots on a canvas with a deterministic force-directed layout computed purely in the client.
  Modules are hub dots, lessons small dots (known failures red, on-trial dim, proposed hollow);
  lines join a lesson to the modules its files touch, to the known failure it was distilled
  from, and to lessons sharing files (nothing in the store supersedes one lesson with another). Arrows and mouse select, `/` searches with `n`/`N`, Enter opens an inspector, `d`
  removes (a lesson is retired; a known failure's signature is dismissed until it changes), `a`/`r`
  approve or reject a proposal. Opened with `g` from the Profile screen's Lessons page; the daemon
  sends nodes and edges.

## 5. Brakes (9.15)

- **Context handoff.** Fill is read from the orchestrator's own transcript tail (the last turn's
  usage), off the lock and bounded. At the goal boundary, at or above the U13 threshold, the next
  goal starts in a fresh session seeded with the M9.3 handoff prompt; the link is recorded. Unknown
  fill keeps today's behaviour. Mid-run there is only a warning at 90%.
- **Quota brake.** Claude's stream `rate_limit_event` (fractions) and the Codex rollout's
  `rate_limits` (percentages), converted in one place. Above the thresholds, new dispatch for that
  runtime stops; in-flight work finishes. It releases at the window's reset or on a fresh reading
  under the threshold, never merely because readings stopped; with no reading it fails open with one
  notice.
- **Global hold.** `anthrex hold on|off|status` and `C-b H`, persisted, visible as a status-bar
  badge; it refuses new sessions, goals, continues and iterates and changes no run state.

## 6. Out of scope

nucleos's lexical classifier, departments and council, git queue, devtime, spec-to-code map,
embeddings, and its mail, calendar, voice and browser pillars. Secret redaction for PR bodies is a
follow-up under 9.2 delivery. Shadow route advice is a follow-up under 9.5 tuning.
