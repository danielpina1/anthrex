# Keep going: a goal editor, rounds and next goals

Milestone **9.3**. Written 2026-10-02 from a brainstorm with the user, who approved the design in chat ("the 9.3 design is ok, write the spec"). It builds on the adaptive-orchestrator spec (`2026-09-22-adaptive-orchestrator-design.md`), the tiered-testing and PR-delivery spec (`2026-09-26-tiered-testing-and-pr-delivery-design.md`) and the TUI spec (`2026-10-01-tui-end-to-end-design.md`). Where they disagree on a matter this spec covers, this spec wins.

## 0. Intent

The user's words were: "the goal creation window should be bigger, and have a nano text field to insert the goal prompt. I want to be able, when I achieve a goal, to continue on the same orchestrator for other goals or iterate over current goals."

**The user's choices from the brainstorm:**

- **Iterating a goal** happens in the **same run**: a complete run reopens for another round, and the user accepts or discards once, at the end of all rounds.
- **A different goal** runs on the **same orchestrator session**, as a **new run**, one goal at a time per orchestrator. The orchestrator keeps its conversation.
- **The editor** is **built in and nano-like**. No external `$EDITOR`.

**What success looks like:**

- A user writes a long, multi-paragraph goal in a large editor, starts it with one key, and never fights the text field.
- When a run completes, the user can say "now also do X" (in the TUI, with the CLI, or in the orchestrator's chat), review the new round's plan at the gate, and accept the whole result once.
- After accepting, the user can give the same orchestrator a new goal. It starts a new run with its memory of the project and of what it just did.

**What does not change:**

- Only the orchestrator is an interactive PTY window. Everything else stays headless and watch-only.
- The engine stays deterministic. Every new transition starts from a user request, or from an orchestrator tool call the engine checks.
- Nothing reaches the user's branch until the user accepts.
- anthrex never merges or lands in pr mode.
- Headless agents load only the user's own settings.

**Order.** 9.3 comes after 9.2 and before 9.5. It builds on 9.2's delivery code for rounds in pr mode, and it takes protocol version **15** (9.2 takes 14). The 9.5 brief re-derives its number from the roadmap.

## 1. The goal editor

### 1.1 Size

The "start a goal" dialog stops being a 64-column box.

- **Width** is `min(cols − 4, 120)`. **Height** is `rows − 2`, where the status bar and one margin row are the only rows it leaves.
- **Below 60 columns or 16 rows**, the dialog keeps 9.0.6's compact layout (the goal field at its current size). The status bar says `widen the terminal for the editor`.
- **The text area** takes every row the dialog does not need for:
  - its title;
  - the option rows (runtime, model, orchestrator §3.3, trust, approve at once, unconfined checks, delivery);
  - one position row;
  - the footer.

  At 80×24 that is about 10 text rows. At 120×40 it is about 26.

### 1.2 Keys (nano-like)

| Key | In the text area |
|-----|------------------|
| any character | inserts it |
| Enter | inserts a new line |
| Backspace / Delete | deletes before / after the cursor |
| ← → ↑ ↓ | move by character and by drawn row; ↑ on the first row and ↓ on the last stay put |
| Home / End, Ctrl-A / Ctrl-E | start / end of the logical line |
| PgUp / PgDn | move by the visible rows less one |
| Ctrl-Home / Ctrl-End | start / end of the text |
| Ctrl-K | cuts the current logical line into the cut buffer; consecutive Ctrl-Ks append, as in nano |
| Ctrl-U | pastes the cut buffer at the cursor |
| paste (bracketed) | inserts the text, keeping its line breaks; control characters dropped (unchanged) |
| Tab | moves to the first option row |
| Ctrl-S | starts the goal from anywhere in the dialog |
| Esc | cancels (§1.4) |

**In the option rows:**

- Tab / Shift-Tab and ↑ / ↓ move between the rows.
- Space and ← / → change the value.
- Shift-Tab from the first row returns to the text.
- Enter starts the goal, as today.

**The footer**, one row, reads `^S start  Tab options  ^K cut  ^U paste  Esc cancel` and drops entries from the right when it is narrow. The **position row** reads `ln 12, col 4 · 1,284 / 16,384`. The count turns to the attention role once the text is above 90% of the cap.

### 1.3 Limits and text

The goal's cap stays 16,384 characters (`TEXT_MAX_CHARS`). Typing or pasting past the cap stops at the cap and shows `goal is at its 16,384-character limit` in the position row. Text is wrapped at the area's width for drawing only; the goal sent is exactly what was typed.

### 1.4 Cancel and drafts

**Esc on an empty goal** closes the dialog.

**Esc on a non-empty goal** opens a confirm page: `discard this goal text?`, with `y discard · any other key back`.

**A draft is kept** per project directory for the life of the TUI process. It is held in pure client state; nothing is written to disk.
- Closing the dialog keeps the draft, unless the confirm page discarded it.
- A successful start clears it.
- Reopening the dialog for that project restores it, with the cursor at its end.

### 1.5 One editor, three uses

The editor is one widget, an extended `TextArea` with a new `kit` renderer. It is used by:
- the goal dialog;
- the iterate dialog (§2.2);
- the next-goal dialog, which is the goal dialog with the orchestrator row set to continue (§3.3).

The task edit form's brief keeps its current field. Moving it to the editor is a follow-up.

## 2. Rounds: iterating a goal in the same run

### 2.1 What a round is

A run has one or more **rounds**. Round 1 is the goal the run started with, and every iteration adds a round:

```rust
pub struct Round {
    pub n: u32,                  // 1-based
    pub goal: String,            // the round's request, cleaned (safe_text::multi_line), ≤ 16,384 chars
    pub origin: RoundOrigin,     // User (TUI or CLI) | Orchestrator (edit_plan iterate)
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub outcome: Option<RoundOutcome>, // Completed | Rejected | Cancelled
    pub summary: Option<String>, // the orchestrator's summary for the round
    pub first_stage: u16,        // the round's stages are first_stage..=last stage
}
```

- **Existing state.** `Run.rounds` defaults to one round, rebuilt from `Run.goal` when it is loaded, so a run from before 9.3 loads unchanged.
- **Tasks and stages** each gain a `round: u32` (default 1).
- **Cap.** At most `ROUNDS_MAX = 20` rounds a run. The 21st iterate is refused with `run <h4> has had 20 rounds; accept or discard it and start a new goal`.

### 2.2 Starting a round

A round can be started three ways:
1. **The TUI.** The action menu on a run offers `iterate` when §2.3 allows it. It opens the editor titled `iterate run <h4> · round <n>`, with a prompt line `what should change or be added?`.
2. **The CLI.** `anthrex run iterate <id> [<text> | --file <path> | -]`. The text comes from the argument, a file, or stdin, and is capped at 16,384 characters.
3. **The orchestrator.** It may call `edit_plan` with `iterate: "<request>"` when the user asks for more work on this goal in its chat (contract rule 43).

Paths 1 and 2 send `RunRequest::Iterate { run, goal }`. Path 3 is `PlanEdit::Iterate { goal }`, checked by the engine like every edit.

### 2.3 When a run can be iterated

A run can be iterated when its current round is **settled**:
- **Local mode:** the run is `Complete`.
- **pr mode:**
  - the run is `Complete` (every PR landed); or
  - it is delivering with nothing in progress: every stage of the current round has an open PR, no task is running or queued, no fix task is pending, and no host op is held.

Every other state is refused, with the state named. For example, `run 3f9a is running; iterate it when it completes`.

A `Halted` run is refused with `run 3f9a is halted; resume or cancel it first`. Accepted, discarded and failed runs are refused too: a new goal is §3.

### 2.4 What happens

When the engine applies an iterate, it does the following.

1. It records the new round, and moves the run from `Complete` (or delivering) to `Planning`.
2. It wakes the orchestrator with this exact text, the request fenced as user input by `quote::fence`:
   `the user asks for round <n> of run <h4>: plan only the new work, in new stages after stage <last>, then submit. Their request:` followed by the fenced text.
3. The orchestrator plans with the usual `edit_plan` calls:
   - New tasks go into **new stages appended after the existing ones**. The engine refuses an `add_task` into a stage of an earlier round with `stage <s> belongs to round <r>, which is done; put new work in a new stage`.
   - Earlier rounds' tasks are read-only. An edit to one is refused the same way.
   - A new task may depend on an earlier round's task. That dependency is already satisfied.
4. On `submit`, the plan gate opens for the new round:
   - It is skipped only when the round's origin is `User` and the run was started with "approve at once".
   - **A round the orchestrator started always stops at the gate.** So an orchestrator can never iterate on its own without the user seeing it.
5. The gate shows only the new round's tasks, under the header `round <n> · <first line of the request>`. Approving runs them.
6. **Rejecting a round** does not fail the run. The round's tasks are cancelled, its outcome is `Rejected`, and the run returns to `Complete` (or delivering) with the earlier rounds intact.
7. The round's stages start from the run's current head:
   - **Local mode:** the integration branch.
   - **pr mode:** see §2.5.
8. Tiers, reviews, fixes and bisect run as usual.
9. When the round's work is done, the engine asks the orchestrator for the round's summary, and the run returns to `Complete` or delivering. That reuses milestone 9's summary step.

A **cancel** during a round (`run cancel`) cancels that round's open tasks and ends the round as `Cancelled`. The run returns to `Complete` with the earlier rounds intact. A task that fails in a round halts the run as today. The user resumes it or cancels the round.

**Accept and discard** cover every round at once, as they cover every stage today.

### 2.5 Rounds in pr mode

A round's stages become new stacked PRs, following 9.2's rules unchanged.
- A new stage's PR is based on the previous stage's branch while that PR is open, and on the base branch once it has landed.
- If every earlier PR has landed, the round's first stage is based on the fetched base.

9.2's retargeting, base sync and landing then apply unchanged.

### 2.6 Limits, history and stats

- **Limits.** Each round gets its own budget of tool calls and minutes, at the values the run started with. The run's totals are the sum across rounds.
- **History.** Each ended round writes a `round` history line: run, round, origin, outcome, tasks, merged, calls, minutes.
- **Stats.** Run stats count tasks as today. `anthrex run stats` adds `rounds: <total> · <iterated runs>`.

## 3. Next goals: the same orchestrator, a new run

### 3.1 The chain

The orchestrator of a run that ends (accepted or discarded) is no longer released as a plain window. It becomes the project's **idle orchestrator**: the head of a **chain** of runs that share one session. A chain is identified by `o-<h4 of its first run>`:

```rust
pub struct Chain {
    pub id: String,              // "o-3f9a"
    pub project: PathBuf,        // the user's checkout
    pub runs: Vec<String>,       // run ids in order; the last is the current one
    pub window_id: u32,          // the orchestrator's PTY window
    pub runtime: Runtime,        // fixed for the chain
    pub model: String,           // fixed for the chain
    pub state: ChainState,       // Active (its current run is not terminal) | Idle
}
```

- **Where chains live.** They are stored in the daemon's state next to the runs.
- **One chain per project at a time.** A project has at most one idle orchestrator. Starting a fresh goal with `‹ new ›` while one is idle leaves that idle chain alone.
- **Failed runs.** A run that **fails** ends its chain: the window is released as today, because a failure may have left the session in a bad state.

### 3.2 While idle

An idle orchestrator's tools are limited:
- `get_context` and `run_status` for its last run (read-only);
- the new `start_goal` (§3.3).

Every other call is refused with `run <h4> has ended; start a new goal with start_goal when the user gives you one`.

The window stays open. The sidebar shows it as `◌ orchestrator · idle · after <h4>`, in the muted role.

A chain ends in three ways:
- **The user closes the window** (`C-b x`). The chain ends, and nothing else is affected.
- **The daemon restarts.** An idle orchestrator is not relaunched (§3.6).
- **The window's process exits.** The chain ends.

### 3.3 Starting the next goal

The next goal can be started two ways:
1. **The goal dialog.** On a project with an idle orchestrator, the dialog's new orchestrator row reads `‹ continue o-3f9a (after 3f9a) ›`, and its other value is `‹ new ›`.
   - Continue is the default.
   - While continue is chosen, the runtime and model rows show the chain's values and cannot be changed.
   - Starting sends `StartGoal { continue_from: Some("<last run id>"), .. }`.
   - The CLI equivalent is `anthrex run start --goal "<text>" --continue <run>`.
2. **The orchestrator.** It calls `start_goal { goal }` when the user gives it a new goal in its chat (contract rule 45).
   - The new run's options (delivery, trust, unconfined checks) are inherited from the previous run.
   - Its plan always stops at the gate, so "approve at once" is never inherited.

**The daemon refuses** with an exact text when any of these is true:
- the named run is not the chain's last run;
- the chain is not idle;
- the project directory differs;
- the checkout is on a branch reserved for runs (the existing guard);
- M8a's preflight fails.

**The new run** gets a new id, and its own branches and plan gate. It is based on the branch the user has checked out, which holds the accepted work if the user accepted into it. The preflight line says `based on <branch> at <sha7>`.

The orchestrator is woken with this exact text, the goal fenced as user input:
`a new goal, run <h4> (your previous run <h4> was <accepted|discarded>):` followed by the fenced goal.
- In pr mode it adds `open pull requests from earlier runs: #12, #13`.

The window is renamed `<new h4>/orchestrator`. The chain records the new run, and its state becomes `Active`.

### 3.4 Which run the orchestrator's tools reach

The orchestrator's MCP target is fixed when its session launches. It names the first run. Two changes keep calls pointed at the right run:
- **The target names the chain.** `McpTarget` gains `chain: Option<String>`, set for orchestrators.
- **The daemon resolves the run.** An orchestrator-role call that carries a chain is served for the chain's **current** run, whatever run id the call names.
  - A call that names an earlier run of the chain is resolved forward.
  - A call that names a run outside the chain is refused.

The contract tells the orchestrator that `run_status` always describes its current run.

### 3.5 One goal at a time

A chain drives at most one live run.
- **A second goal while the chain is active** needs `‹ new ›`: a fresh orchestrator and a separate run. The dialog shows `o-3f9a is working on run <h4>; this goal gets a new orchestrator`.
- **`start_goal` while active** is refused with `run <h4> is still going; finish it before starting another goal`.

### 3.6 After a daemon restart

- **Active chains.** A chain whose current run is live is restored like a run's orchestrator today: dormant, then relaunched by `run resume` with a new session.
- **Idle chains.** They are not relaunched. Their window is gone, and the chain is marked ended.
- **Choosing continue on an ended chain.** The goal dialog still offers `‹ continue o-3f9a (fresh session) ›`. Starting it launches a new orchestrator session. The session's first prompt includes the previous run's summary, its outcome, and the last 10 history lines of the chain's runs. The new session joins the same chain id.

## 4. The orchestrator contract

**Rule 33 changes** to: "When run_status reports complete, write a summary with edit_plan summary. The user accepts or discards the run, or iterates it; you never accept or discard."

**New rules** are appended after 9.2's rule 42:
- **43.** When the user asks you in chat for more work on the goal of a complete run (or of a settled pr run), call `edit_plan` with `iterate` and a restatement of their request. The run stops at the plan gate for the user. Never iterate on your own initiative.
- **44.** In a round, plan only the new work. Earlier rounds' tasks are done and read-only, and new tasks go in new stages after the last one. A new task may depend on an earlier task.
- **45.** After the user accepts or discards your run, you stay as the project's orchestrator. When the user gives you a new goal in chat, call `start_goal` with it. Never start a goal on your own initiative, and only one goal at a time.
- **46.** For a new goal, `run_status` describes the new run. What you remember from earlier runs is context: plan from the new goal, and check facts against the repository.

The **MCP schema** gains:
- `edit_plan`'s `iterate` (string, ≤ 16,384 characters);
- the `start_goal` tool (`goal`, string, ≤ 16,384 characters).

Sub-planners are refused both, as they are refused `reply_comment` today.

## 5. Protocol 15

Every message and field below gets a round-trip test (hard rule 4). New fields use serde `default` and are skipped when empty, so a protocol-14 snapshot decodes:
- `RunRequest::Iterate { run: String, goal: String }`. The reply is `Done(text)`, or a refusal with its exact text.
- `RunRequest::StartGoal` gains `continue_from: Option<String>`.
- `PlanEdit::Iterate { goal: String }`.
- The orchestrator tool request `StartGoal { goal: String }`, beside the existing role-scoped calls.
- `RunInfo` gains:
  - `round: u32`;
  - `rounds: Vec<RoundInfo { n, goal_head, origin, outcome, summary_head }>`, where `goal_head` and `summary_head` are cleaned and cut to 200 characters;
  - `chain: Option<String>`.
- `TaskInfo` and `StageInfo` gain `round: u32`.
- The snapshot gains `idle_orchestrators: Vec<IdleOrchestrator { chain, project, after_run, outcome, runtime, model, window_id, fresh }>`, where `fresh: bool` marks a chain whose session ended (§3.6).
- `McpTarget` gains `chain: Option<String>`. It is an internal launch argument, not a wire message, but it is versioned with the MCP binary's arguments.

## 6. TUI

- **The goal dialog** is §1, plus the orchestrator row §3.3.
- **The iterate dialog** is §1.5. It is opened from the action menu's `iterate`, which the daemon lists only when §2.3 allows it.
- **The run view:**
  - The frame title adds `· round <n>` when a run has more than one round.
  - Stage rows of earlier rounds are drawn muted, under a separator row `round <r> · <goal head>`.
  - The run inspector lists the rounds, each with its outcome and summary head.
  - At 80×24 the compact list shows the separator rows in the same way.
- **The task panel's** DETAIL gains `round <r>` for a run with more than one round.
- **The plan review** header names the round: `plan · <title> · <h4> · round 2`. Its summary counts only the round's tasks.
- **The sidebar** shows the idle orchestrator row (§3.2). Enter on it focuses the window. `.` opens a menu with `new goal here` (the goal dialog, preset to continue) and `close` (the window, with a confirm page).
- **Alerts.** None are new. A round waiting at the gate raises 9.0.5's existing gate alert, titled `round <n> awaits approval`.
- **Design rules.** Everything follows 9.0.7's rules:
  - one accented frame;
  - theme roles with ASCII twins;
  - the hostile-text rule for every goal and summary text drawn;
  - the status bar under modals.

## 7. CLI

- `anthrex run iterate <id> [<text> | --file <path> | -]`.
- `anthrex run start --goal <text> --continue <run>`.
- `anthrex run status <id>` adds a `round <n> of <total>` line, and one line a round: `round 2 · user · completed · <summary head>`.
- `anthrex ls` shows an idle orchestrator as `o-3f9a  orchestrator  idle after 3f9a`.

## 8. Safety and determinism

- **User text is data.** A round's request and a next goal are user text. They reach the orchestrator only fenced (`quote::fence`) and capped at 16,384 characters, and they are cleaned by `safe_text::multi_line` before they are stored.
- **Every transition is an engine event.** Each one starts from a `RunRequest` the user sent, or from an orchestrator call the engine checks against §2.3 and §3.3. The engine never starts a round or a goal by itself.
- **Orchestrator-started work always stops at the gate.** A round or goal the orchestrator started waits there, whatever the run's options. This keeps an orchestrator from looping without the user.
- **The base never moves without the user.** Nothing reaches the base until the user accepts, and in pr mode until the user merges. A next goal reads the base only as its starting point.
- **A chain's session sees only its project.** Its working directory and tools stay the project's; tool calls are resolved to its own chain and refused outside it.

## 9. Testing

Every test uses fake-agent only, an isolated daemon (its own socket, data dir and `ANTHREX_CONFIG` under a temp dir), every `*_BIN` pinned, and deadline loops whose bounds are derived as `docs/timing-budgets.md` requires.

**Editor (TUI, pure):**
- key-by-key tests for every row of §1.2;
- the cut buffer's appending;
- the cap;
- the draft's keep, restore and clear;
- the Esc confirm page;
- render tests at 80×24 and 120×40, in colour and ASCII, through the audit (one accented frame);
- the compact fallback below 60×16;
- a hostile paste: visible bidi and ZWJ characters gone from the stored text, with a mutant shown red.

**Rounds (daemon engine):**
- §2.3's allowed and refused states, with exact texts;
- new tasks only in new stages, and edits to an earlier round refused;
- the gate always taken for an orchestrator-started round, and skipped for a user round only with "approve at once";
- a rejected round returns to Complete with round 1 intact;
- a cancelled round;
- `ROUNDS_MAX`;
- each round's budget;
- the round history line;
- a pre-9.3 `run.json` loads as one round.

**Chains (daemon):**
- an ended run's orchestrator becomes idle, and a failed run's does not;
- idle tool limits;
- `start_goal` and `continue_from`, with every refusal and its exact text;
- MCP resolution forward through a chain, and refusal outside it;
- one live run per chain;
- restart: an active chain relaunches, and an idle one is marked ended;
- continue on an ended chain launches a fresh session with the handoff prompt.

**End to end (real binary, fake-agent):**
- a local run with two rounds, then accept: the integration branch has both rounds' commits, there are two round history lines, and one accept;
- a next goal on the same chain after the accept: the same window id, a new run based on the accepted branch, and the orchestrator's transcript showing one session with both wake texts;
- a pr-mode round on FakeHost: the new stages' PRs are stacked on the open ones, and `forbidden.jsonl` is absent.

**Smoke stage `11j`**, "rounds and next goals":
1. start a goal through the editor (typing a multi-line goal and pressing Ctrl-S);
2. approve it and wait for complete;
3. iterate it from the menu, approve round 2, and wait for complete;
4. accept;
5. start the next goal with continue;
6. assert the same orchestrator window and the TUI's `round 2` title;
7. end with `anthrex daemon stop` under the same variables.

## 10. Out of scope

- One orchestrator driving several runs at once.
- Stacking a new run on another run's unaccepted branches.
- Undo, search and syntax colouring in the editor.
- Moving the task edit form's brief to the editor (a follow-up).
- An external `$EDITOR`.
- Relaunching an idle orchestrator after a restart (§3.6 gives a fresh session instead).

## 11. Risks

- **Long sessions.** A chain's session grows with every goal. The runtimes compact their own context, but a long chain can get slow or expensive. The sidebar row shows the chain's run count (`idle · after 3f9a · 4 runs`), and `‹ new ›` is always one keypress away. 9.5's tuning can add a suggested chain length from the history.
- **Ctrl-S.** It is XOFF on terminals with flow control. crossterm's raw mode clears `IXON`, so the TUI receives it. A smoke step pins that in a real PTY. Enter on the option rows stays as a second way to start.
- **Resolving tool calls by chain.** This changes which run an orchestrator call reaches. The forward resolution is pure and tested on its own, and a call naming a run outside the chain is refused, never redirected.
- **Rounds in pr mode while PRs are open.** These add stages above open PRs. They reuse 9.2's stacking and retarget rules, with no new host op. The end-to-end test pins the bases.
