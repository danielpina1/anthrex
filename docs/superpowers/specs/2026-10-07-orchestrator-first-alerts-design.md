# Orchestrator-first alerts: design

Date: 2026-10-07. Status: approved in conversation, awaiting spec review.

## 1. Goal

While a run's orchestrator is alive, everything the run's agents and engine need goes to the
orchestrator, which resolves it. The user is alerted only for approvals, acceptance, profile
proposals, problems only the user can fix, the orchestrator's own explicit questions, and the
orchestrator itself being stuck. With no live orchestrator, today's alerts are unchanged.

Success: during a run with a live orchestrator, the Alerts list holds no `Blocked`, `Halted`,
`Stage` or `Delivery` entry the orchestrator could act on; every question the orchestrator puts
to the user is one "orchestrator asks" alert; background workers never ring the bell.

## 2. Decisions (from the user)

| # | Decision |
|---|---|
| D1 | Questions and problems from subagents go to the orchestrator first. |
| D2 | Autonomy "Everything": the orchestrator may retry, override (merge without approval), resume, approve holds and accept red stages on its own. |
| D3 | The orchestrator escalates with an explicit `ask_user` tool; that is the user's only question channel during a run. |
| D4 | The user still sees approvals (plan, design-doc gates, holds the orchestrator did not approve), ready-to-accept, profile proposals, user-only problems, and orchestrator-stuck alerts. |
| D5 | Worker windows no longer toast or ring. |
| D6 | What the orchestrator resolved on its own stays visible, quietly. |

## 3. Today (summary)

Alert kinds are built on every draw by `alerts()` in `crates/tui/src/app/alerts.rs`
(`AlertKey::{Orchestrator, Gate, Hold, Blocked, Halted, Accept, Proposal, Stage, Delivery}`).
While an orchestrator lives, only `Question`, `MisSized` and `DepCancelled` blocks stay off the
user's list; `Human`, `Conflict` and `Environment` blocks, halts, tier-3 holds and delivery alerts
reach the user even though the engine also wakes the orchestrator with a note
(`run/engine/wake.rs`). The orchestrator can only `answer` question blocks; retry, override and
resume are user-only (`run/engine/actions/`). It has no way to raise an alert: a question in its
chat window produces none. Any background window turning `Attention` toasts and rings
(`tui/src/app/windows.rs`).

## 4. Design

### 4.1 Routing rule

A pure function `alert_route(alert, orchestrator_lives) -> Route { User, Orchestrator, Both }`
in `crates/tui/src/app/alerts_route.rs` decides each alert. With `orchestrator_lives == false`
every alert is `User` (today's behaviour). With a live orchestrator:

| Alert | Route |
|---|---|
| `Blocked` with reason `Question`, `MisSized`, `DepCancelled`, `Environment`, `Human`, `Conflict` | Orchestrator |
| `Blocked` whose block is user-only (§4.4) | User |
| `Halted` | Orchestrator, unless its reason is user-only |
| `Stage` (tier 3 held, propagate red, red) | Orchestrator |
| `Delivery` `CiHandedToUser`, `ReviewRoundsOverCap`, `HostOpHeld`, `PrClosedUnmerged` | Orchestrator |
| `Delivery` `GhLoggedOut` | User |
| `Gate` (plan, round, design doc) | User |
| `Hold` | User (the orchestrator may also approve it, §4.2) |
| `Accept`, `Proposal` | User |
| `Orchestrator` (permission prompt, held wake-up, start prompt) | User |
| `OrchestratorAsks` (new) | User |
| `OrchestratorStuck` (new: stalled past `stall_after_secs`, or dead with work pending) | User |

"Orchestrator lives" is today's `orchestrator_lives` test, extended: an orchestrator that is
stalled or dead does not count, so its alerts fall back to the user.

The engine already wakes the orchestrator for every block (`wake::note`). It additionally wakes
it for halts, stage holds/reds and delivery events that do not note it today, so every
Orchestrator-routed alert reaches it.

### 4.2 New orchestrator operations

`edit_plan` gains ops that call the same engine actions the user's do, with `actor:
Orchestrator` recorded:

| Op | Engine action | Today's user path |
|---|---|---|
| `retry { task, reason }` | `actions::retry` | `anthrex run retry`, alert menu |
| `override { task, reason }` | `actions::override_task` | `anthrex run override` |
| `resume { reason, stage? }` | run or stage resume | alert menu Resume |
| `approve_hold { hold, reason }` | approve hold | `anthrex run approve --hold` |
| `accept_red { stage, reason }` | accept a red tier-3 stage | stage menu |

Each requires a non-empty `reason`, is recorded in the journal and the run log as `orchestrator:
retried t3 — <reason>`, and appears in the run's "handled" list (§4.5). Each is refused with the
user path's own refusal when the engine refuses the user's (same preconditions). Plan and
design-doc gates are not orchestrator-approvable.

### 4.3 `ask_user`

New orchestrator MCP tool `ask_user { question: String, options: Vec<String>, context: String }`:

- Stores one pending question per run (a new one replaces an unanswered one, which is noted);
  the run's snapshot carries it.
- The TUI raises `AlertKey::OrchestratorAsks(run)`, priority 1, text `orchestrator asks:
  <question>`, detail `context` and numbered options.
- Enter focuses the orchestrator window. `1`–`9` on the alert, or typing in the window, answers:
  the choice is delivered to the orchestrator as a message (`the user chose: <option>`), and the
  pending question is cleared. Answering in the window also clears it (the next orchestrator turn
  start clears a question it was told about).
- The orchestrator's tool call returns immediately ("asked; the answer arrives as a message"),
  so it can keep working on other tasks.

### 4.4 User-only problems

A block or halt is user-only when its cause is one the orchestrator cannot fix: `gh` logged out,
a CLI missing or logged out, disk full, git identity missing, the repository's base branch moved
in a way that needs the user (existing `HostOpHeld` reasons that name credentials). The engine
tags these at the point it raises them (`BlockReason::Environment` gains a `user_only: bool`, set
by the raising sites listed in the plan; default `false`). They route to the user even with a
live orchestrator, tagged `you` in the list.

### 4.5 Quiet visibility

The run's row in the Alerts/run view shows `orchestrator handled N` when the orchestrator
resolved anything; expanding it lists each action with time, op, target and reason, from the
journal. No toast, no bell.

### 4.6 Bells and toasts

`windows.rs`: `Attention` toasts and the bell fire only for orchestrator windows and for windows
the user created (plain agent windows, shells). Worker, reviewer, scout, planner, decider and
brainstorm windows never toast or ring. `bell.done` is unchanged.

### 4.7 Contract

`run/orch/contract.rs`:
- Rule 24 becomes: answer questions from the plan, scouts or your judgement; ask the user with
  `ask_user` only for product decisions or what only they can fix.
- Rule 26 ("tell the user; you cannot unblock them") is replaced by: resolve blocks, halts, stage
  and delivery problems yourself with `retry`, `override`, `resume`, `approve_hold`,
  `accept_red`, `split_task`, `amend_task` or `cancel_task`; give a reason for each.
- New rule: never ask the user in chat; use `ask_user`.

### 4.8 Protocol

New `edit_plan` ops, the `ask_user` request, the pending question in the run snapshot, the
handled-actions list, and `user_only` on blocks are wire changes: one `PROTO_VERSION` bump,
shared with the model-roles milestone if they ship together (AGENTS.md rule 4), with round-trip
tests for each new message and op.

## 5. Error handling

- An orchestrator op the engine refuses returns the refusal text to the orchestrator, as edits do
  today.
- If the orchestrator dies or stalls with Orchestrator-routed alerts pending, they reappear in the
  user's list at once (routing re-evaluates every draw), plus `OrchestratorStuck`.
- An `ask_user` from a run that then completes or is cancelled is dropped with the run.

## 6. Testing

- `alert_route`: every alert kind × orchestrator alive/stalled/dead (table test).
- Engine: each new op succeeds where the user's action does and is refused where it is; journal
  and run-log lines carry the actor and reason.
- Wake: halts, stage events and delivery events now wake the orchestrator (one test each).
- `ask_user`: raises one alert; option keys deliver the message and clear it; a replacing question
  is noted; answering in the window clears it.
- `user_only`: each tagged raising site routes to the user with a live orchestrator.
- Bell/toast: worker windows silent; orchestrator and user windows ring.
- Contract text: the rule changes are present (existing contract tests pattern).
- Protocol round trips.
- End to end with the fake agent: a worker blocks with `environment`; with a live (fake)
  orchestrator no user alert appears, the orchestrator's scripted `retry` runs, the task
  completes, and `orchestrator handled 1` shows.

## 7. Delivery

One milestone brief, built after model roles (or alongside, sharing the protocol bump).
Execution: subagent-driven with TDD.
