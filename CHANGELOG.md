# Changelog

All notable changes to anthrex are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and anthrex uses
[Semantic Versioning](https://semver.org/). While the major version is 0, every
release is a pre-release and any release may change behaviour or the protocol.

## [Unreleased]

The protocol version is now 26 (22 for milestone 9.13's `OrchestratorInfo.untrusted`,
23 for milestone 9.14's build admission, 24 for milestone 9.15's lessons, 25 for
milestone 9.16's brakes: `RunRequest::Hold`, `RunReply::Held`, `RunsSnapshot.brakes`,
`RunInfo.handoff`, a routing decision's `failover`, the settings document's `brakes`;
26 for milestone 9.17's plan and review quality: a finding's `target`, the `brief`
task note kind, `RunInfo.plan_frame`, `PlannerInfo.frame`, `TaskInfo.replaces` and
`replaced_by`, `add_task`'s `replaces`, a task record's `replaces` and
`brief_findings`, and `HistoryStats.replaced`): a client needs a daemon of the same
release. Stop the old daemon (`anthrex daemon
stop`) after upgrading.

**Downgrading.** `run.json` is now format 7 (milestone 9.15: a run's frozen lessons,
its discard reason, a task's lessons shown and risk note, and the distill and
risk-note ops; milestone 9.16: a run's failovers and its context handoff; milestone
9.17: a run's plan frames, an epic's frame, the approved spec's Non-goals, a task's
`replaces`, a finding's target and the `brief` notes; milestone 9.19: each design
agent's raw token usage). A 0.2.0 daemon skips a run
this release wrote (`run format 7; this one reads up to 1`) and
leaves it as it is, as does any daemon that reads an older format. The hold lives in
`brakes.json` in the data directory, which older releases ignore: an older daemon
does not hold dispatch. Releases before
0.2.0 have no such guard and can drop this release's new run fields when they save
(the untrusted-read record, the orchestrator's retry count, the pinned gate files, a
pending override proposal and an override count's protected list): finish or discard
your runs before downgrading below 0.2.0.

### Added

- **Reviewers see what the run is for** (milestone 9.17). Every reviewer prompt now
  has the run goal as its second line (`Run goal: …`, the same line the worker gets),
  and, before the brief, the plan's out-of-scope list (`Out of scope for this run …`)
  or, in a design-flow run, the approved spec's Non-goals.
- **`brief` findings.** A reviewer that thinks the brief itself is wrong (wrong for the
  goal, out of scope, or missing something the goal needs) files a finding with
  `target: brief`. It never sends the worker back and never blocks the merge; it
  becomes a `brief` task note that wakes the orchestrator (`t1's reviewer says the
  brief is wrong: …`). A run without an orchestrator, such as a plan file, shows it to
  you as a sticky attention line in `anthrex run status` (`t1: the reviewer says the
  brief is wrong (1 notes); read them in the task's notes`). The inspector, the report
  (`(brief, from the reviewer)`) and history (`brief_findings`) show them apart from
  the code findings.
- **Decided for you · Out of scope.** The orchestrator's plan submit, and each
  sub-planner's `submit_epic`, now carry `decided` (each question the goal left open,
  with the answer chosen) and `out_of_scope` (what the run will not do); a submit
  without them is refused, with the reason. At the plan gate the review opens on
  the frame (`frame  2 decided for you · 1 out of scope · f shows them`, or `frame
  none stated` in the attention colour), and `f` flips the detail between the frame
  and the selected task. `anthrex run status` prints `decided for you: <question> →
  <answer>` and `out of scope: <item>` after the goal while the plan awaits you, and
  the run report has a `Plan frame` section. Plan files, your own `run edit --submit`
  and design-flow runs need no frame.
- **`replaces`.** An orchestrator's `add_task` can name the cancelled or blocked tasks
  it takes over (`replaces: ["t1"]`); a blocked one is cancelled (`cancelled: replaced
  by t2`), and `split_task` records it for its children. The inspector shows `replaces`
  and `replaced by`, the report `cancelled (replaced by t2)`, and the history record
  names what it replaces. `anthrex run stats` and the TUI's stats page judge the work
  by the replacement: replaced records leave the class rows (`replaced 1 (judged by
  their replacements)`).
- **Shared sequences.** The orchestrator's and sub-planners' contracts now say that a
  task taking the next number of a shared sequence (a migration number, a version
  constant, a registry entry) owns the file that holds it, so two such tasks never run
  at once.

- **A dispatch hold** (milestone 9.16). `C-b H` (with a confirm) or `anthrex hold on`
  stops every new agent session daemon-wide; sessions already running finish their
  work, are reviewed and merge. The status bar shows ` HELD `, an alert and every
  run's attention line say so, and new goals are refused with `dispatch is held
  daemon-wide · anthrex hold off`; queued goals wait. The hold survives a daemon
  restart (`brakes.json`, written atomically; a corrupt file holds). `C-b H` again or
  `anthrex hold off` releases it and says how many queued goals start; `anthrex hold
  status` shows the hold and the quota.
- **A subscription quota brake.** anthrex reads each CLI's own rate-limit readings,
  Claude's `rate_limit_event` and the `token_count` lines of Codex's session rollout
  under `$CODEX_HOME/sessions`, with no credentials and no vendor request. While a
  runtime's 5-hour window is at 90 % or its 7-day window at 95 %, no new session of
  that runtime starts until the window resets; in-flight work still finishes. A
  runtime with no recent reading is never braked, and the run log says so once.
  `[orchestrator.brakes]` (`quota_brake`, `quota_pause_5h_percent`,
  `quota_pause_7d_percent`, `quota_fresh_secs`) sets it, also from the Settings
  screen; `anthrex run status` prints the brake lines while something is held.
- **Cross-runtime failover.** A fresh session whose runtime is braked starts on its
  role row's "if it struggles" fallback when that is on the other runtime: `claude
  braked: t2 started on codex (fallback)`. A reviewer never gets its author's model,
  overlapping work never splits runtimes, and the hold stops failover too. A row whose
  fallback is on the same runtime waits instead.
- **The orchestrator's context handoff.** When a goal is continued on a chain whose
  orchestrator session is 80 % of its context window or 300,000 tokens full
  (`[orchestrator] handoff_at_percent`, `handoff_at_tokens`, `context_window_tokens`),
  a fresh orchestrator session starts with the summary and recent history, the old
  window is renamed `<h4>/orchestrator (handed off)`, and the run's inspector shows
  where it came from. A live orchestrator at 90 % gets an attention line.

- **Lessons** (milestone 9.15). anthrex now learns from its own runs in a repository.
  Each failed check or proof records a failure signature on the task's history record,
  and a signature seen in two runs within 14 days becomes a known failure. After a
  recovered, blocked or blocking-review task, or a failed or discarded run, a
  background distiller (one decider call, off the run, at most 20 a day per
  repository, on a redacted dossier) proposes lessons: episodic ones go on trial at
  once, procedural and semantic ones wait for your approval. A run freezes the active
  lessons at its start, and each worker's first turn gets the best few for its files
  under `Notes from earlier runs in this repository. …`, its reviewer the same under
  `Known pitfalls in this repository`; a lesson earns credit when the tasks shown it
  merge. A reviewer can also get a short risk note written from the brief alone. The
  new `[lessons]` table turns each part on or off and sizes it.
- **`anthrex lessons list | approve | reject | forget`.** List the repository's lessons
  with their status, layer and `green/outcomes` (`--all`, `--json`), approve or reject
  proposed ones, and forget a lesson or dismiss a known failure (`y/N` per id unless
  `--yes`).
- **The Lessons page and the knowledge graph page.** On the Profile screen (`C-b P`),
  `l` lists the lessons (`a` approve, `r` reject, `d` forget, `t` all) and its hint
  counts those waiting for review; `g` there draws them as a graph of module hubs and
  lesson dots, with selection by arrows, an inspect panel, search, zoom and pan.
- **A discard reason.** `anthrex run discard <run> --reason "<text>"`, and a reason
  field in the TUI's discard form, record why a run was thrown away on its history
  record and in its distill dossier.

- **Files that run outside the sandbox are protected.** CI, git-hook, cargo and
  toolchain files join the protected paths: `.github/workflows/**`,
  `.github/actions/**`, `.gitlab-ci.yml`, `.circleci/**`, `.buildkite/**`,
  `Jenkinsfile`, `azure-pipelines.yml`, `.githooks/**`, `.husky/**`,
  `.pre-commit-config.yaml`, `**/.cargo/**` (the directory itself too, so a `.cargo`
  committed as a symbolic link is caught), `**/rust-toolchain` and
  `**/rust-toolchain.toml` (21 built-in paths in all). A task
  whose branch changes one without its `owns` naming it exactly is bounced at
  `task_done` with a line that says why (`… runs outside the run's sandbox (CI, git
  hooks, cargo or toolchain config); this task may change it only if its owns names it
  exactly`). A task whose `owns` names a protected file is noted at the plan gate
  (`owns names protected <path>; the user approves this grant with the plan`).
- **The gate's own scripts are pinned.** At run start anthrex reads which tracked
  files the profile's commands name (`check`, `setup`, `single_test`, the tier
  commands; `make`, `just`, `tox` and `nox` add their own files) and protects them.
  Before every gate command it compares them in the checkout with the base commit; if
  the run changed one, the command never runs and the run halts for you (`the gate's
  script <path> differs from the base commit's; the run changed what measures it`).
  Only you can resume it. A task may change a gate script when its `owns` names it
  exactly and you approve that with the plan.
- **An orchestrator retry cap.** After the orchestrator has retried one task twice,
  that task's next block is yours (`· the orchestrator retried it 2 times; the user
  decides`); your own `run retry` gives it its two retries back.
- **Untrusted text is tracked.** Once the orchestrator has read text from outside the
  run (a pull-request comment, or a review or CI fix task's block, note or result),
  its `override` and `accept_red` become an `orchestrator asks` question to you (`do
  it` / `leave it`), and its `approve_hold` is refused, for the rest of the run and any
  next goal it continues. `anthrex run status` prints `untrusted text read: <source> at
  <HH:MM>; override, accept_red and approve_hold go to the user`, and the run view's
  inspector shows an `untrusted` row. A fix task's wake-up no longer carries its
  worker's text (`read it with run_status`).

### Changed

- **The orchestrator cannot merge or grant protected changes on its own.** Its
  `override` of a task whose branch changes a protected file the task's `owns` does
  not name is refused and the task's block becomes yours; it cannot approve a hold
  whose tasks name a protected file, nor add such a task outside a hold after the plan
  gate. Your own `run override` still goes through and lists the protected paths it
  changes; for a pinned gate script the reply adds that it `stays pinned until a task's
  owns names it (run edit)`, so the next gate command halts until you add a task whose
  `owns` names it.
- **`brief` findings leave the code-review counts** (milestone 9.17). A history
  record's `reviews_rejected` no longer counts a round whose only findings are about
  the brief, the orchestrator's digest shows them apart in a task's review text (`r1
  changes (1 brief)`), and an integration review whose findings are all about the
  brief approves its epic.
- **Tokens are billed tokens everywhere, and a run's total counts its design agents**
  (milestone 9.19). A brainstormer's and a document reviewer's tokens no longer count
  cache reads (input, cache writes and output, as every other role's), and the run's
  usage now credits them under `brainstormer` and `doc_reviewer`, so its total covers
  every role. `history.jsonl` lines are now written at `v` 6: from `v` 6 a `phase`
  line's agent tokens are billed tokens and a `run` line's usage includes the design
  roles; the design budget refit takes its token samples from `v` 6 lines only. A run
  designed before the upgrade keeps no split of its design agents' tokens, so its total
  leaves them out (an undercount, never a cache-read overcount). The run spend row
  reads `tokens 1.8M (all roles) · cache hits 71% of input · worker tool calls 412`
  (`tokens ≥… (tasks only)` from a daemon without run usage), `REPORT.md` and the
  inspector print tokens as `750k` and `1.2M`, and `run stats` leaves a merged task with
  no recorded tokens out of its token median (`-` when none are left).
- **Every number has one meaning, label and format** (milestone 9.19), written down in
  `docs/numbers.md`, which `?` help (a `numbers` group) and the README link to. The
  protocol stays 26; `run.json` is format 7 and `history.jsonl` lines are `v` 6 (both
  from the token change above).
- **Clock times name their zone.** The TUI and the CLI show local time with its offset
  (`14:30 +02:00`, `Thu 14:30 +02:00` within six days, else the date); text the daemon
  writes (attention lines, prompts, the proposal file, the `REFIT` column) stays UTC and
  says `UTC`. On Linux, a change of the system time zone mid-session is picked up only
  after a restart (DST changes are picked up at once). After the upgrade the
  orchestrator digest's `at` values read `2026-09-21 11:40 UTC`, not `11:40`
  (cosmetic: the times are the same).
- **One duration and one age format**: `42s`, `41m 12s`, `1h 05m`, `3d 04h`, floored;
  `just now` under 10 s, else `<duration> ago`. Fixed-width cells (the sidebar, graph
  boxes, the alerts list) show the largest unit only.
- **One progress count and one stage position.** `3/5 merged` leaves out cancelled,
  reported and replaced tasks everywhere, a run still being designed shows its design
  word instead of `0/0 merged`, and every position reads `stage 2 of 3`.
- **Unknown is not zero.** A missing value reads `unknown` (`-` in a table cell): a
  sub-agent with no end time, a check that never ran (`✗ never ran`), a doc gate still
  loading, an age before the first snapshot.
- **Context fill never reads `450%`**: `context 91% full (183k / 200k)`, `context over
  100% (450k / 200k window)`, or `context 183k (window unknown)`.
- **Ages freeze when the daemon link is lost.** Timers and ages stop counting, and the
  badge reads ` DISCONNECTED · as of <ago> `; the model picker's age is the catalog's own
  fetch age.
- **`anthrex hold status` tells the truth**: `(read 3m 10s ago)`, `(stale: read 1h 35m
  ago)` for a stale reading, the brake text the TUI shows for a braked runtime, and the
  hold's age (`on since Thu 14:30 +02:00 (2h 05m ago)`).
- **`anthrex tree` labels its numbers**: `#1  id 7` (the position `C-b <n>` uses and the
  id `ls`, `kill` and `attach` take), `claude 2 · codex 1 · shell 1`, and `running 3m
  10s`, `ran 12s` or `done -` for a sub-agent.
- **`anthrex run stats` labels its aggregates**: `MED LINES`, `MED CALLS`, `MED TOKENS`,
  `MED WORK` (a floored duration, was rounded minutes) and `BOUNCES (ALL)`, a footer
  saying what the medians are over, and `rounds: 3 ended · 1 run iterated`. The TUI's
  stats page uses the same words and shows the rounds line.

### Fixed

- **A host sleep no longer stalls an open turn or delivers into it.** Across a laptop
  sleep the wall clock jumped, so the stall watchdog interrupted an open worker turn
  and the outbox delivered a message mid-turn (against decision 42), and the minutes
  budget charged the sleep. The daemon now measures each sleep (wall time against the
  awake clock) and excuses it from an agent session's stall clock, its waits (rate
  limit, delivery and count retries, the interrupt's grace, a failed turn's continue)
  and its minutes budget, and from a design phase's budget, a design agent's session
  seconds and a stopped racer's exit wait; budgets count awake time. Orchestrator
  notices and the GitHub-facing delivery polls still run on wall time. Records of what
  happened stay wall times.

Backlog hardening (milestone 9.18), no new features. The protocol (26) and the run
format (6) are unchanged.

- **More secrets are redacted** from provider prompts and the Lessons page: Stripe
  `sk_live_` and `sk_test_` keys, AWS secret access keys (after their key name, or on
  the line of an `AKIA` or `ASIA` key id), `Basic` credentials, and URL passwords
  spelled `%3A%2F%2F…%40` or `:\/\/…@`.
- **`timeout 600 cargo test` waits for its build slot** like a bare `cargo test`, and so
  do `stdbuf` and `cargo llvm-cov`, `cargo miri`, `cargo hack` and `cargo insta test`.
- **A cached green is not reused after a `CARGO_PROFILE_*` change.** `CARGO_PROFILE_*`,
  `CARGO_TARGET_<triple>_RUSTFLAGS`, `CARGO_BUILD_RUSTFLAGS` and `CARGO_INCREMENTAL` are
  now part of the tier cache key. If you set any of them, your next tier run misses the
  cache once.
- **A task's own `[task.budget]` raises its size ceiling.** The budget ladder's next-size
  check now uses, per axis, the larger of the task's budget and its class's, so a task
  given more room is not stopped at its class's limit. Token totals no longer overflow.
- **A wheel scroll no longer holds the orchestrator's wake-up.** Mouse reports sent
  while a wake-up is pasted no longer count as typing, so the wake-up is submitted.
- **Dialogs wrap in display columns**, so a path with CJK characters or emoji no longer
  breaks the force-remove prompt's box.
- **A failed tool call shows as failed at once** in the conversation view, from Claude's
  `PostToolUseFailure` hook, instead of staying pending until the transcript is read.
- **Lessons requests never hang.** `anthrex lessons` (`list`, `graph`, `approve`,
  `reject`, `forget`) refuses a store or history that is not a regular file, such as a
  FIFO, instead of blocking on it, and gives up after 30 s on a file system that stops
  answering. A stuck `approve`, `reject` or `forget` no longer holds the store's lock,
  so later changes and the lesson distiller still write.
- **`lessons.jsonl` is compacted.** Once it reaches 1 MiB, the lines no reader needs
  (a closed lesson's day-old attempts, and records a later line replaces) are dropped.
  Only the newest three moved-aside `tuning.toml.bad-*` files are kept.

## [0.2.0] — 2026-10-11

The protocol version is now 21 (19 for model roles, 20 for orchestrator-first alerts, 21
for the simple repository profile): a client needs a daemon of the same release. Stop
the old daemon (`anthrex daemon stop`) after upgrading. The stabilization work (milestone
9.11, under "Fixed") leaves the protocol unchanged (21).

**Upgrading.** Runs in flight keep working across the upgrade. Task checkouts made by
0.1.x are accepted as they are; each one picks up your git identity at its next fresh
session (see "Task checkouts commit as you" below).

**Downgrading.** Every `run.json` this release writes records its format (`"format": 1`).
A run written by a newer anthrex (a higher format, or one that is not a whole number) is
skipped with `run <id> skipped: written by a newer anthrex (run format <n>; this one reads
up to 1); left as it is`, and its directory is never rewritten. Releases before 0.2.0
lack this guard: a 0.1.x daemon loads a 0.2.0 run and can drop the fields it does not
know when it saves it again. Finish or discard your runs before downgrading below 0.2.0.

### Added

- **A first goal sets the repository up.** A goal started in a repository with no
  profile no longer fails: it waits in a queue while anthrex reads the repository and
  checks its commands, and the Alerts box shows `setting up anthrex · reading the repo`,
  then `· checking commands (n/m)`, then one `review how anthrex will work here`. Enter
  opens the review card; **Use this** stores the profile and starts every goal that was
  waiting, in order. Goals wait however long the review takes, and a drained goal is
  triaged against the repository as it is when it starts. With `--yes`, the goal stores
  the profile itself as soon as every command checks out (a dropped command still waits
  for your review). A set-up that fails keeps the queue and offers **Retry**;
  discarding the proposal drops the waiting goals, and `anthrex profile status` and the
  Profile screen list what waits and what was dropped. `anthrex run start --goal` in
  such a repository exits with status 3 and prints nothing on stdout (on stderr:
  `queued: <goal> waits for the repository profile (anthrex profile); no run started
  yet`), so a script that expects a run id stops.
- **The review card.** `anthrex learned how to work in this repo` lists what anthrex
  will use, in plain words, each command with its check (`✓ 12s`, `✗ 3m10s`) and a
  `couldn't verify:` line for each command that did not pass; a later re-detection
  shows only the rows that changed, `old → new`. `⏎` **Use this**, `e` edit a row, `x`
  discard, `esc` later (the proposal, its alert and the waiting goals stay).
- **Row edits.** Editing a command on the Profile screen checks it once: on ✓ it is
  saved at once (`saved <label>`); on ✗ the row shows the value that failed
  (`old → tried ✗`) and why, with `o` for its output, `s` to save it anyway and `r` to
  revert. The same works on the review card, where the edit changes only the proposal
  until **Use this**; a failed one is named under the card, since **Use this** leaves
  it out. `anthrex profile status` prints the edit, its state and its reason. Edits
  stay refused while a run is live in the repository (`finish or cancel the run in this
  repo to change its profile`).
- **`anthrex profile use`** stores the ready proposal and starts any waiting goals;
  `anthrex profile confirm` still works as its alias.
- **`anthrex profile edit --anyway`** stores the edit once verification has run, even
  if a check fails (it implies `--yes`).
- **Orchestrator-first alerts.** While a run's orchestrator is alive, a blocked task, a
  halted run, a held or red stage and a delivery problem go to the orchestrator first,
  not to you. It resolves them itself with five new `edit_plan` ops, each with a reason
  you can read: `retry`, `override`, `resume_run`, `approve_hold` and `accept_red`.
  When it needs you, it asks with the new `ask_user` tool, which raises one
  `orchestrator asks: <question>` alert; answer with `1`-`9` in the Alerts view
  (`C-b a`), by typing in its window, or see it in `anthrex run status`. You are still
  alerted for approvals, acceptance, profile proposals, problems only you can fix (a
  full disk, a missing git identity, a logged-out CLI or `gh`, billing), and an orchestrator
  that has stalled or exited, which hands everything it held back to you. What the
  orchestrator resolved stays visible, quietly: `orchestrator handled N` in the run
  view's inspector (with each action and its reason), a footer line in the Alerts view,
  `anthrex run status` and the run log.
- **Model roles.** Every model anthrex launches comes from one table, `[models]`: a row
  per role (orchestrator, planner, the three implementer sizes, test writer, reviewer,
  research, the helpers, the brainstormers), each a model, an optional effort and an
  optional "if it struggles" fallback. `codex:default` runs Codex's own configured
  model. A repository can override rows in its own `models.toml`. Settings (`C-b S`)
  shows the table and picks models from the list the installed `claude` and `codex`
  report. Older configs keep choosing the same models: their keys are read as rows and
  removed only when you save, which keeps the previous file as `config.toml.bak`. See
  the README's "Models".

### Changed

- **The Profile screen is one plain page.** `C-b P` shows one status line (`Ready ·
  verified 2h ago`, `Needs review — anthrex has a proposal`, `Out of date — Cargo.toml
  changed · re-checking`, `Not set up — press d to set up`, and the rest), then three
  sections (How anthrex checks your work, Your repo, Delivery) with plain labels and
  each key dimmed beside it, and the rest folded under `Advanced ▸` (`a` opens it).
  `anthrex profile status` prints the same status line. `profile proposal ready`
  alerts now read `review how anthrex will work here`.
- **`onboarding.auto` means only re-detection.** A goal in a repository with no profile
  now sets it up whatever `[orchestrator.onboarding] auto` says; `auto` still decides
  whether a stale profile is detected again on its own.
- **Worker windows no longer ring.** Only an orchestrator window and the windows you
  started yourself ring the bell or show a toast when they need you; a run's workers,
  reviewers and scouts report to their orchestrator instead.
- **The orchestrator no longer routes tasks.** It gives each task a size, and the size
  picks the row. A route from the orchestrator, a sub-planner or a plan file is ignored,
  and the run log says `route model ignored: models come from the role table`.
- **Hub tasks' reviewers change model.** Every review runs the one `reviewer` row
  (built-in: Codex's own default) instead of picking a model by review level, so a hub
  task's reviewer is no longer Claude Opus.
- **A route naming only a runtime no longer selects a model.** `anthrex run edit` refuses
  one (`choose a model; runtime alone no longer selects one`).
- **Escalation raises effort first.** A task that struggles runs again at its model's
  next higher effort, then on its row's fallback, then stays; it no longer moves to a
  stronger model or the other runtime on its own. A race's second racer runs the row's
  fallback, or the same model when there is none.
- **A missing orchestrator CLI refuses the start** instead of switching to the other
  runtime: `install it, or choose another model for the orchestrator in C-b S, or another
  runtime with --orchestrator`. Any other role whose CLI is missing fails naming the role
  (`reviewer: codex not found; choose another model in C-b S`).
- **Model names are checked.** A model you name (a task's route, `--orchestrator`) must
  be 1 to 100 visible characters with no spaces and no leading `-`.
- **Research scouts run the `research` row** (built-in: Claude Haiku at low effort).
  `[orchestrator.tuning] escalate_above_percent` and a `tuning.toml`'s `[routes]` are no
  longer used, and `anthrex run stats` proposes no model routes.

### Removed

- **The Profile screen's tabs, its `s` store-on-pass toggle and its confirm page.** The
  screen has one page, the card replaces the confirm page, and an edit from the screen
  is always saved once its check passes. `c`, `p` and Tab do nothing there any more.

### Fixed

- **Codex workers can commit on Linux.** Every Codex session is now sandboxed through
  Codex permission profiles on Codex 0.160 and later: a Linux Codex worker gets its task
  git directory whole, with its configuration and the protected agent-config paths
  (`.git`, `.claude`, `.codex`, `.mcp.json`, `AGENTS.md`, `CLAUDE.md`) read-only. Older
  Codex keeps the legacy sandbox flags, and its workers still cannot commit on Linux.
  Do not downgrade anthrex with a run in flight: older daemons ignore the new read-only
  list.

Stabilization (milestone 9.11), no new features. The protocol is unchanged (21).

**Data loss**

- **The run list never outgrows a frame.** Clients get every live run and the 50 newest
  finished runs; a snapshot still too large for one frame drops its oldest finished runs
  first. Before, a long-lived daemon with many finished runs stopped sending runs to its
  clients at all. Older runs stay on disk, untouched (A1).
- **A worker's waiting messages survive its replacement.** A session replaced at rung 2,
  a task blocked at rung 3 or 4, the last race lane (the one adopted after the other ran
  out) and a retried task keep their queued messages for the next session, which gets
  each one once. A lane stopped while the other lane races on still drops its own (A2).
- **A failed resume keeps a message that was delivered but not yet confirmed** (A4).
- **A task checkout deleted by hand is salvaged.** Its commits are imported from its
  repository and kept under a salvage ref before the repository is removed. One window
  is left: if the daemon dies inside that removal, the restore can still remove the
  repository without salvaging it (A3).
- **The orchestrator can no longer cancel or split a task blocked for you**; it gets the
  refusal `retry` already gave (A5).
- **A queued goal that could not start raises an alert**, `could not start "<goal>":
  <reason>`; Enter opens the Profile screen (A7).
- **Reading the wake digest answers only the notes it held**, so a newer note still
  restarts the orchestrator's stall clock (A8).
- **Keys you type during a wake-up are not submitted with it.** If you type in the
  orchestrator window while a wake-up note is being pasted, the note is left in the
  input for you to submit (A9).
- **A worker's broken sandbox is named** in the task's next block, stall or failure
  reason, with the fix, as scouts already did; nothing new is blocked (A10).
- **A lagged session feed is caught up**, so a burst of output never loses a turn end or
  an exit the engine was waiting for (A12).
- **A run written by a newer anthrex is never rewritten** (A13; see "Downgrading" above).
- **A task checkout that holds a nested git repository with unsaved work is kept**, with
  `kept <path>: it holds a nested git repository (<sub>) whose work anthrex cannot
  salvage; remove it yourself when done`. Initialized submodules are checked the same
  way (A14).
- **Task checkouts commit as you.** They now commit with the identity your repository
  resolves, `includeIf` included. A run in flight across the upgrade keeps working; each
  of its checkouts picks the identity up at its next fresh session (A15).
- **A session's transcript is learned from any hook** after a missed `SessionStart`, so
  the conversation view still finds it (A16).

**Crashes and hangs**

- **A panic in the run engine costs one event, not the engine.** A request waiting on it
  is answered `the run engine failed while carrying out this request (a bug; see the
  daemon log); the run may not match what was saved`, and the engine keeps running. A
  step that panics twice is rolled back in memory (B1, B3).
- **`anthrex daemon stop` leaves no daemon process behind** when a blocking task is
  stuck: the process exits about 2 s after it releases its lock (B2).
- **Shutdown waits for a worktree removal under way** (B4).
- **Briefs are capped where the daemon checks them**: 1,000,000 characters per brief,
  4 MiB for a plan's briefs together (B5).
- **A leftover session whose process id only the journal held is stopped at restore.** A
  launch's process id is now journaled even while the daemon stops (B6, the cases that
  need no process scan; see the README's "What a crash can leave running").
- **Starting the client behind a stopping daemon works.** The client's daemon start
  waits 10 s (the daemon's own 5 s lock wait plus a 5 s allowance for loading), claims
  the start again while it waits, and spawns the daemon itself when it wins. A
  `<data>/daemon.spawning` marker, fresh for 5 s (`PRE_BIND_ALLOWANCE`), keeps two
  waiting clients from both spawning one (B7).
- **Token totals saturate instead of overflowing** (B8).
- **The force-remove dialog lists its safe keys first** (`n`, `k`, then `f`), so a
  clipped dialog never shows only the destructive one (B9).
- **Profile writes to one repository no longer wait for another's** (B11).
- **Git watches on Linux skip what anthrex ignores** (`objects/`, `lfs/`, ignored and
  denied directories): each kept directory is watched on its own, so a large repository
  under a low `max_user_watches` is less likely to fall back to the 30 s poll (B12).
- **`profile status` no longer shows a proposal ready before the client does**: the
  daemon notes it in memory before it writes it.

**Flaky tests** (each with a reproduction that failed before its fix where one could be
built; see the two exceptions noted below)

- The module-graph capture file never collides; two tier jobs in one instant no longer
  fall back to a full run (C1, E1).
- The restart-endpoint test reads the endpoint once it is written, not once the file
  exists (C3).
- The Codex question rig warms its stand-in and waits for the whole screen (C4). The
  warm-up matches the observed failures but could not be reproduced by injection.
- Git test bounds include the watcher's arming, which has no bound of its own (C6, C10).
- The persisted session id test waits for a record that holds one (C7).
- A zombie no longer counts as a live process in the headless-session tests (C8).
- The profile queue's restart test waits for the first scout to claim its script (C14).
- The wake first-turn rig warms its stand-in before any window starts. Its stress run
  passed 239 of 240 copies; the one failure, in `status_codex`, is unattributed.
- A Linux test that re-ran itself under the wrong name, and so ran nothing and passed,
  now runs its body.

**Security and isolation**

- **Checks do not see your tokens.** Checks, proofs, `setup` and tier steps no longer
  inherit cloud and registry tokens (`GITHUB_TOKEN`, `GH_TOKEN`, `NPM_TOKEN`, `AWS_*` and
  the rest listed in the README); `ANTHREX_CHECK_ENV_PASS=NAME,...` in the daemon's
  environment passes one through. A check that relied on one fails until you pass it
  (D3).
- **`AGENTS.override.md`, `CLAUDE.local.md` and `.agents/**` are protected** like the
  other agent instruction files (D6).
- **The conversation view drops hidden format characters** (D2), and **`run status`**
  drops them from reasons (D7).
- **The Linux worker grant refuses a symbolic link at a denied path** (`the checkout's
  git directory has a link at <name>; anthrex will not grant it`) instead of replacing
  it, and every daemon git call in a task checkout checks the checkout's pin again after
  the call (D8).
- **Restoring a checkout's permissions walks by open directory**, never through a path
  re-resolved after a step, so a swapped-in link is left alone (D9).
- Your own agent windows keep inheriting `CLAUDE_CODE_*`, `MCP_*` and `OTEL_*`; the
  README now says so (D1). `worker_sandbox = false` is documented as giving workers your
  full power, approving their own review included (D5).

**Terminal input and small fixes**

- **Attaching repaints a full-screen program** (`less`, `vim`): an attach at the window's
  own size nudges its size, so the program redraws.
- **The wheel follows the program**: the mouse encoding it asked for (SGR, X10, UTF-8),
  or arrow keys in a full-screen program without mouse reporting.
- The paste boundary policy is documented in the README ("Pasting").
- A dialog opened while the prefix is pending clears it, so the next key is not read as
  a command (E2).
- Saving settings keeps a `config.toml` link to a file not yet created (E4).
- The plan review no longer lists a task among its own dependencies (E25).

## [0.1.2] — 2026-10-07

The protocol version is now 18: a 0.1.2 client needs a 0.1.2 daemon. Stop the old
daemon (`anthrex daemon stop`) after upgrading.

### Added

- **Run titles.** A run started from a goal gets a short title and a readable id
  (`status-bar-usage-version-a415` rather than the goal's first 32 characters), written
  by a quick decider call with a 15-second bound. The run view, the runs tree, the
  inspector and alerts show the title; the inspector keeps the full goal. When the
  call fails or times out, the run is named from its goal as before.
- **Esc no longer interrupts a working orchestrator.** In the orchestrator window of a
  live run, a bare Esc is held back while the orchestrator is working, with a notice to
  use Ctrl-C. Esc still passes when it is idle or waiting on you.

### Fixed

- **Claude workers could not commit on Linux.** The worker sandbox granted exact git
  files, including lock files that do not exist yet, which Linux's sandbox cannot grant.
  On Linux a worker now gets its checkout's git directory whole, with the entries it
  must never change (`config`, `gitdir`, `refs/`, `hooks/` and others) denied. macOS is
  unchanged. Codex workers on Linux keep the old grant and are still affected; use
  Claude workers there for now.
- **Sandbox hardening found in review:** the daemon no longer stages files in a
  directory a worker can write, never renames over a denied entry, verifies a task
  checkout's `config` before every git call, waits for a merged task's worker to exit
  before removing its checkout, and runs its git with `rerere` and submodule recursion
  off.
- **A retired session's exit no longer kills its successor.** When a paired task's test
  writer was retired before its turn-end reached the engine, its exit counted as a
  crash: the engine resumed it, then declared the task stalled and killed the
  implementer that had just started. A retiring session's exit now simply ends it, for
  workers, reviewers and research sessions alike.
- **A headless agent that dies at startup now says why.** A scout's or run task's
  failure reason includes the agent's own error, such as Claude's sandbox needing
  `bubblewrap` and `socat`, or Ubuntu's AppArmor blocking it (with a pointer to the fix),
  instead of "ended two turns without a report".

## [0.1.1] — 2026-10-06

### Fixed

- **Claude Code 2.1.291 and later could not see anthrex's tools.** Claude Code now
  speaks MCP `2026-07-28` and ignores a tool list that lacks cache hints, which the
  MCP library anthrex used left out. Every scout, so every `anthrex profile detect`,
  failed with "the scout ended two turns without a report". anthrex now uses rmcp
  3.5.1, which adds the hints.

## [0.1.0] — 2026-10-06 — first alpha

The first public build of anthrex: a terminal multiplexer for coding agents.
Prebuilt binaries are for Linux (x86_64, aarch64) and macOS (Apple silicon, Intel);
see [docs/install.md](https://github.com/danielpina1/anthrex/blob/main/docs/install.md). The binaries are unsigned.

### Added

- **Multiplexer.** A background daemon owns one pseudo-terminal per agent and runs
  the real `claude` or `codex` app, or a shell, inside it. A terminal client shows
  every agent with the focused one at full size; detaching leaves the agents
  running. A `C-b` prefix keymap and the `anthrex` CLI (`new`, `ls`, `kill`, `rm`,
  `daemon`, ...) drive it.
- **Agent status.** Live working / needs-attention / done status for Claude Code
  and Codex sessions, and their sub-agents, from the agents' own hooks.
- **Project tree and graph.** Every project drawn as a tree of sessions and
  sub-agents to any depth, a graph overview, and a node inspector below it.
- **Git surface.** The focused checkout's branch, uncommitted work and divergence
  in the bottom bar, kept fresh by watching the filesystem.
- **Worktrees.** A new-agent dialog that can start an agent in its own git worktree.
- **Persistence.** Windows, names and sessions survive a daemon restart or reboot;
  agents resume their previous session; a config file and client reconnect.
- **Conversation view.** A structured view of an agent's turns, with folded tool
  calls and links to its sub-agents.
- **Orchestrator.** Give a goal to one orchestrator agent: it scouts the
  repository, plans and sizes the work, and dispatches each task to a headless
  Claude or Codex worker in its own worktree. Each task passes a test proof, the
  check and a review by a different agent, then merges through a queue that tests
  the merged result. A plan gate, alerts, a live run view (`C-b T`) and a run
  inspector let you steer it; nothing reaches the base branch until you accept it.
- **Adaptation.** A repository profile, scouts and deciders, a fast path for small
  goals, an output filter, metering and run history.
- **Tiered testing.** Affected-set test tiers, a test scheduler and result cache,
  flake handling, bisecting and staged checks.
- **Stacked-PR delivery.** Results delivered as stacked pull requests; CI failures
  and review comments become fix tasks. anthrex never merges a pull request itself.
- **Rounds and chains.** A goal editor, rounds that iterate on a complete run, and
  next goals on the same orchestrator.
- **Tuning.** History refits and proposals, model lists, a run estimate, adaptive
  concurrency, and race and test-writer patterns.
- **Design flow.** Brainstorm (two models, merged), spec (peer-reviewed, with
  numbered requirements) and plan (with engine-checked coverage), each approved by
  you before the next begins.
- **Delivery hardening.** Fix tasks for merged stages, unviewed merges checked with
  git, trimmed holds, and a CI bisect over the whole stack.
- **TUI.** A design kit across every screen, a daemon-computed action menu with
  forms and confirmation pages, and Profile and Settings screens with live reload.
- **MCP server.** `anthrex mcp` exposes anthrex to an agent over stdio.

### Known limitations

- Split panes are not in this release.
- macOS and Linux only. anthrex runs `claude` and `codex` as installed on your
  machine; it does not ship or install them.

[Unreleased]: https://github.com/danielpina1/anthrex/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/danielpina1/anthrex/compare/v0.1.2...v0.2.0
[0.1.2]: https://github.com/danielpina1/anthrex/releases/tag/v0.1.2
[0.1.1]: https://github.com/danielpina1/anthrex/releases/tag/v0.1.1
[0.1.0]: https://github.com/danielpina1/anthrex/releases/tag/v0.1.0
