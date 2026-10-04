# Design flow: brainstorm, spec, plan

Milestone **9.6**. Written 2026-10-04 from a brainstorm with the user. The user approved the design section by section in chat: approach A, then sections 1–6, each answered "looks good". It builds on:
- the adaptive-orchestrator spec (`2026-09-22-adaptive-orchestrator-design.md`);
- the tiered-testing and PR-delivery spec (`2026-09-26-tiered-testing-and-pr-delivery-design.md`);
- the TUI spec (`2026-10-01-tui-end-to-end-design.md`);
- the keep-going spec (`2026-10-02-keep-going-design.md`);
- milestone 9.5 (tuning: routing lists, reader slots, adaptive caps, history refits).

Where they disagree on a matter this spec covers, this spec wins.

## 0. Intent

The user's words: "for goal execution I want to create the following flow, Brainstorm -> Spec -> Plan … I approve each part: the brainstorm complete report, the spec and plan." Then: "In brainstorm we can use 2 good models at the same time for the same goal and use both results … the best way to create an optimized yet great system." And: "I don't want to use superpowers itself, I want that system in anthrex."

**The user's choices from the brainstorm:**

- **Built into the engine** (approach A). The phases and gates are engine states, not prompt conventions. The alternatives were: prompts only; the documents as plan tasks; and a two-step "A-lite".
- **Three documents, three approvals.** The brainstorm report, then the spec, then the plan. Only the user opens a gate.
- **Two models brainstorm the same goal independently**, and one merged report uses both.
- **It works like a careful engineer's process.** Clarifying questions; 2–3 approaches with trade-offs; a sectioned spec with testable requirements; a plan of small tasks with tests first. All of it is anthrex's own phases, prompts, tools and checks. No agent invokes an outside skill.

**What success looks like:**

- A user starts a planned goal and answers a few questions. Some minutes later they read one report that shows where two strong models agreed and where they disagreed, and they pick an approach.
- The user reads a spec whose every requirement is numbered and testable. The disputes between the writer and an independent reviewer are shown beside it.
- The user approves a plan the engine has checked: every requirement is covered by a task. What they approved is exactly what runs.
- Workers and reviewers work against the requirements they cover, and the run report shows each requirement's outcome.

**What does not change:**

- Only the orchestrator is an interactive PTY window. Brainstormers and document reviewers are headless, read-only and watch-only.
- The engine stays deterministic. Every transition starts from a user request, or from an orchestrator tool call the engine checks.
- Nothing reaches the user's branch until the user accepts. anthrex never merges or lands in pr mode.
- Goals that are not planned (the fast path, research and review goals) run as before.

**Order.** 9.6 comes after 9.5 and takes protocol version **17**; 9.5 takes 16. Delivery hardening, the pr-mode follow-ups FU-F29–F34, moves to 9.7.

## 1. When the flow runs

| Goal | Design flow |
|---|---|
| Triage path **plan** or **large**, kind `code` or `docs` | on by default |
| **Fast** path (one S/M task) | off |
| kind `research` or `review` | off (their output is already a report) |
| `anthrex run promote` of a fast-path run | off (work has started) |
| a run started from a `--plan` file | off (the plan already exists) |

**Override per goal:**
- the goal dialog's new row `design: full / off`;
- `anthrex run start --design full|off`;
- the config default.

```toml
[orchestrator.design]
default           = "full"          # full | off, for goals the table above puts on
docs_dir          = "docs/anthrex"  # "" = never commit; documents stay in the data folder only
commit_brainstorm = false
max_questions     = 5
```

A run records its choice as `Run.design: Design { Full | Off }`, frozen at start like `RunLimits`.

## 2. Phases, states and gates

```
triage → scouts (once, unchanged)
       → brainstorming  → gate brainstorm
       → specifying     → gate spec
       → planning       → gate plan (today's plan gate)
       → running …
```

- **Run states.** `RunState` gains `Brainstorming` and `Specifying`; their labels are `brainstorming` and `specifying`. Today's `AwaitingApproval` stays the one waiting state.
- **The gate record.** `Run.gate: Option<Gate>` says which gate the run waits at:

  ```
  Gate {
    kind: GateKind (Brainstorm | Spec | Plan),
    version: u32,
    opened_at,
    revising: Option<String>,  // the user's note while the orchestrator revises
  }
  ```

  A run without the design flow uses `Gate { kind: Plan, .. }` at its plan gate, so there is one code path.
- **Questions.** On entering `brainstorming`, the orchestrator may ask the user up to `max_questions` short questions in its window, one at a time. The user answers or types `skip`. The orchestrator then calls `start_brainstorm { answers }` (§4). That call launches the brainstormers. Calling it is refused outside `brainstorming`, and refused while brainstormers are running.

### 2.1 Gate actions

Each action is available in the TUI (§6), the CLI (§7) and the Alerts box.

| Action | Gates | Effect |
|---|---|---|
| **approve** | all | The next phase starts. At the plan gate, the documents are committed (§5) and the run starts, as today. |
| **changes** `note` | all | The orchestrator revises. The gate stays open with `revising = Some(note)`, and the next version opens when the orchestrator submits it. |
| **edit** `text` | all | The user's text becomes version n+1, `edited by you`. It still needs approve. At the plan gate, this is today's per-task editing; `plan.md` itself is generated, not edited. |
| **rethink** `note` | brainstorm | Both brainstormers run again with the note and the previous report. Then the merge is redone. |
| **back** `note` | spec, plan | Reopens the previous gate with the note, as a changes request there. Later documents are kept as superseded versions. |
| **reject** | all | Discards the run (round 1), as rejecting a plan does today. 9.3's round rules apply to later rounds (§8.1). |

**Rules:**

- **Only the user acts on a gate.** No orchestrator tool approves, edits, rethinks, goes back or rejects.
- **`--yes` skips none of the three gates** in a design-flow run. The run log says so once, exactly: `design flow: --yes does not skip the brainstorm, spec or plan gates`.
- **An open gate and every document version survive a daemon restart.**
- **The orchestrator learns verdicts** through a wake note and `run_status`, as for today's plan gate. No tool call blocks on the user (adaptive-orchestrator §12.3).
- **The Alerts box** shows one row per waiting gate. Examples: `brainstorm v1 ready for review · run 3f9a`, and `spec v2 ready for review`.

### 2.2 Budgets

- **Brainstormers** have 40 tool calls and 15 minutes each by default. **Document reviewers** have 20 calls and 10 minutes. Both are configurable under `[orchestrator.design.budget]`, and 9.5's history refit applies to them as to task classes.
- **Over budget.** A brainstormer or reviewer that goes over is stopped and counts as failed (§3.5, §4.3).
- **Orchestrator phases.** Each orchestrator phase (`brainstorming` after the drafts are in, `specifying`, `planning`) has a wall-clock budget, `phase_minutes = 60` by default. Past it, the run halts with `design flow: the <phase> phase passed its <n> min budget`, and `run resume` continues it. Gate waits never count.

## 3. The brainstorm

### 3.1 Brainstormers

- **A new headless role, `Brainstormer`.** It is read-only: its sandbox and tools are a scout's (adaptive-orchestrator §19). It holds a reader slot, so `max_readers` and 9.5's adaptive caps apply.
- **Two launch, in parallel.** Routes come from `[orchestrator.routes.brainstorm]`, a 9.5 route list. Without a list, the defaults are the strongest installed model of each installed runtime: one Claude and one Codex.
- **With one runtime installed,** its strongest model runs twice, with two lenses:
  - brainstormer **A**: *"the smallest change that fully meets the goal"*;
  - brainstormer **B**: *"the most robust, long-lived design"*.
- **With two runtimes,** both get the same neutral prompt, so their drafts compare fairly.

### 3.2 Independence and inputs

**The input pack.** Both get the same pack:
- the goal;
- the user's answers;
- the scout reports;
- the repo profile;
- for a chained goal, the previous run's approved spec, marked as related earlier work (§8.2).

**They stay independent.** Neither ever sees the other's draft. They may read the repository to check their claims, and every constraint they cite carries a `file:line`.

### 3.3 The draft

Each brainstormer submits its draft with `submit_doc { kind: "brainstorm_draft", text }`. The draft is capped at 12 KiB, and the engine checks its sections:

```
## Understanding        — the goal restated; what success looks like
## Assumptions          — each marked
## Constraints found    — with file:line
## Approaches           — 2 or 3; each: how it works, files touched, trade-offs, risks, size S/M/L
## Recommendation       — one approach, and why
## Questions for you
```

**A refused draft** gets the exact reason, and the brainstormer may resubmit within its budget.

### 3.4 The merged report

When both drafts are in, or one draft and one failure (§3.5), the engine wakes the orchestrator: `both brainstorm drafts are in; read them with get_doc and submit the merged report`. The orchestrator reads them with `get_doc` and submits `submit_doc { kind: "brainstorm", text }`, capped at 32 KiB:

```
## Where they agree
## Where they disagree    — side by side: each draft's argument, then the orchestrator's judgment
## Approaches             — merged, de-duplicated, each tagged [claude], [codex] or [both]
                            (with one runtime: [A], [B] or [both])
## Recommendation         — one, naming a listed approach
## Questions for you
```

**What the engine checks:**
- every section is present;
- every approach heading carries a tag;
- the recommendation names a listed approach.

**The appendix.** The engine attaches both drafts as the report's appendix. The orchestrator never copies them in.

**The gate.** The report becomes version 1 of the brainstorm gate. **Questions for you** appear first at the gate (§6). The user's answers travel in the approve or changes note, and the spec phase receives them.

### 3.5 Failure

- **One brainstormer fails** (a launch failure, a crash, or over budget). The run continues with the other draft, and the merged report must begin with the line `single brainstorm: <runtime or lens> failed: <reason>`. The engine checks this line.
- **Both fail.** The run halts with `design flow: both brainstormers failed: <reason a>; <reason b>`, and `run resume` relaunches them.
- **Restart.** A brainstormer running at a daemon restart is relaunched fresh with the same pack.

## 4. The spec

### 4.1 Writing it

The orchestrator writes the spec in its own window. It works from the goal, the answers, the approved brainstorm report and the user's approval note, the scout reports and the repo profile. It submits with `submit_doc { kind: "spec", text, ready }`, capped at 64 KiB:

```
# <title>
## Goal and success criteria
## Non-goals
## Approach
## Design
## Requirements          — lines starting `R<n>` (R1, R2, …), each testable, each with its acceptance check
## Interfaces
## Errors and edge cases
## Testing
## Risks
## Open questions        — must be empty when `ready = true`
```

**The engine refuses a spec, with exact text, when:**
- a section is missing;
- a requirement number repeats or does not start at R1;
- `TBD` or `TODO` appears outside a code block;
- Open questions is non-empty with `ready = true`;
- it is over the cap.

**Requirements** are parsed from lines in the Requirements section that match `^R(\d+)\b`. A requirement's text runs to the next `R<n>` line or heading. The parsed list is stored with the version.

### 4.2 The spec review

- **Who reviews.** A `ready = false` submit dispatches a **document reviewer**: a new headless, read-only role, `DocReviewer`, that holds a reader slot. Its route is the orchestrator's peer: the other runtime at the same strength, via 9.5's `peer_route`. If no peer is installed, the orchestrator's own runtime is used, and the gate shows `reviewed by the same runtime`.
- **What it checks:**
  - placeholders;
  - contradictions;
  - ambiguous or untestable requirements;
  - scope creep beyond the approved approach;
  - brainstorm decisions the spec dropped;
  - requirements without an acceptance check.
- **How it reports.** It submits `submit_findings { findings: [{ id, severity: blocking|minor, where, text }] }`.
- **The orchestrator answers.** It is woken, revises, and submits again. A `ready = true` submit must answer every finding of the latest review with `responses: [{ id, answer: "fixed" | "kept: <reason>" }]`, or it is refused, naming the unanswered ids.
- **At most two reviews.** After the second, only `ready = true` is accepted.
- **When the gate opens.** A `ready = true` spec opens the spec gate. Findings answered `kept: …` are **disputed** and appear beside the spec (§6).
- **Revising.** A revision after the user asks for changes is submitted `ready = true` directly. It gets one fresh review only if the user's note asks for one: `--review` in the CLI, or a checkbox in the note editor.

### 4.3 Reviewer failure

A document reviewer that fails gives the gate a line `not reviewed: <reason>`, and the gate opens on `ready = true` without findings. The user decides.

## 5. The plan, and committing the documents

### 5.1 The plan is the task graph

- **The orchestrator builds the plan with `edit_plan`, as today.** `PlanTask` gains `covers: Vec<String>`, the spec requirement ids the task delivers. It is `#[serde(default)]`, empty for runs without the design flow.
- **Briefs follow a stricter shape in a design-flow run.** The engine checks for these headings in each task's brief:
  - files to create or change;
  - the tests to write first (names, and what each asserts);
  - the steps;
  - the acceptance criteria;
  - the command that verifies it.

**`edit_plan { submit: true }` in a design-flow run adds these checks** to today's validation:

- **Coverage.** Every requirement of the approved spec is in some task's `covers`, and every `covers` entry names a real requirement. A refusal is exact, for example `R4, R7 are covered by no task`, or `task t3 covers R12, which the spec does not have`.
- **Brief shape.** Every task's brief has the five headings above. A refusal names each task and the heading it is missing.

**The plan review.** A passing submit dispatches a document reviewer on the plan. It checks for:
- missing steps;
- wrong order;
- tasks too big for their size;
- tests that would not prove their requirement;
- `covers` claims the brief does not deliver.

It runs once. The orchestrator answers every finding in `edit_plan { submit: true, responses }`, and then the plan gate opens. Today's plan-gate edits (route, brief, size, test mode, removal) still work. Removing a task re-runs the coverage check, and approval is refused while a requirement is uncovered.

**`plan.md`.** The engine renders the graph as `plan.md` for the gate and the repository:
- the stages, then each task with its brief and `covers`;
- a requirement → task coverage table.

**Large path.** Each sub-planner receives only the spec sections its epic's requirements cite, plus Goal and Interfaces. Coverage is checked across all epics together.

### 5.2 Requirements in the work

- **Worker prompts.** A worker's first prompt includes, for its `covers`, each requirement's text plus the spec's Goal section. It does not include the whole spec.
- **Task reviewers.** The task reviewer's prompt includes the same requirements and asks for a verdict on each. A `request_changes` verdict may cite a requirement id.
- **REPORT.md.** The report gains `## Requirements`, a table of requirement, tasks and outcome. The outcome is `merged` when every covering task merged, otherwise the covering tasks' states.

### 5.3 Where the documents live

**The data folder holds every version, always.** It is the source of truth for the gates:

```
<data>/runs/<id>/design/
  brainstorm/draft-<route>.md …
  brainstorm-v<n>.md   spec-v<n>.md   plan-v<n>.md
  findings-spec-v<n>.json   findings-plan-v<n>.json
  versions.json        — per version: author (orchestrator | you), reason, time, parsed requirements
```

Versions are immutable. `REPORT.md` links to the approved ones.

**The repository gets the approved spec and plan.**
- **When.** On approval of the plan gate, the engine makes one commit, `docs: spec and plan for <goal head>`. The engine writes it, not an agent, so the committed text is exactly the approved text.
- **Where.**
  - local mode: on the run's branch, before the first task branches from it;
  - pr mode: on the bottom stage's branch.
- **Paths.** `<docs_dir>/specs/YYYY-MM-DD-<slug>.md` and `<docs_dir>/plans/YYYY-MM-DD-<slug>.md`. The slug comes from the spec's title.
- **What happens later.** Accept merges the documents with the code. Discard drops them.
- **Opting out.** With `docs_dir = ""`, nothing is committed.
- **The brainstorm report** is committed only when `commit_brainstorm = true`.
- **Git rules.** The commit goes through the project's git write queue, with `--no-optional-locks` and the scrubbed environment, never under the manager lock (AGENTS.md rules 2, 10, 11).

## 6. TUI

### 6.1 The gate screen

The gate screen opens from:
- an Alerts row;
- the run view's orchestrator node, which shows `⏸ <kind> v<n>`;
- the action menu.

**Layout.** At 100 or more columns, the document is on the left and a **Review** panel on the right. Narrower, the document takes the full width and the panel shrinks to one line, `<n> changes · <m> disputed findings (f)`.

**The Review panel** shows:
- changes from the previous version (a summary of added, removed and changed sections and requirements);
- disputed findings, each with the orchestrator's answer;
- at the brainstorm gate: the agree and disagree counts, and each approach's tag;
- at the plan gate: the coverage table.

**The header** says the version, its author and its reason, for example `Spec · run 3f9a · v2 of 2 · revised after your note: "split R4"`.

**Keys:**

| Key | Action | Confirm page |
|---|---|---|
| `y` | approve | `Approve <kind> v<n>? <next phase> starts next.` |
| `c` | changes. A note editor opens; at the brainstorm gate it is pre-filled with the report's questions. | no |
| `e` | edit in the 9.3 editor; Ctrl-S saves version n+1 | no |
| `r` | rethink (brainstorm) | yes |
| `b` | back (spec, plan) | yes |
| `x` | reject | yes, listing what is discarded |
| `d` | full-screen coloured diff against the previous version | — |
| `f` | all findings, with severities | — |
| `a` | the appendix: both drafts, side by side when wide (brainstorm) | — |
| ↑↓ PgUp PgDn | scroll | — |
| `q` / Esc | close | — |

**Other screen rules:**
- While `revising` is set, the screen shows `revising v<n+1>… (your note: "…")`, and only `x` and `q` work.
- The action menu `.` lists the same actions.
- **The plan gate** is today's plan review screen with a new **Plan doc** tab, showing `plan.md` and the coverage table.

### 6.2 Elsewhere

- **The goal dialog and the iterate dialog** gain a `design:` row:
  - goal dialog: `full / off`;
  - iterate dialog: `amend / full / off`.
- **The run view** shows the phase on the orchestrator node, and the brainstormers and document reviewers as agent nodes under it, with their routes.
- **The status bar** shows `⏸ <kind> v<n>` while a gate waits.

## 7. CLI

```
anthrex run start --design full|off …
anthrex run iterate --design amend|full|off …
anthrex run show <run> --doc brainstorm|spec|plan [--version N] [--diff] [--findings]
anthrex run approve <run> --gate brainstorm|spec|plan
anthrex run changes <run> --gate <kind> --note "<text>" [--review]
anthrex run edit-doc <run> --gate <kind> --file <path>     # the file's text becomes version n+1
anthrex run rethink <run> --note "<text>"
anthrex run back <run> --gate spec|plan --note "<text>"
anthrex run reject <run>                                    # unchanged
```

`run status` prints the phase and, while a gate waits, `waiting for you: <kind> v<n> (anthrex run show <run> --doc <kind>)`.

## 8. With rounds, chains and pr mode

### 8.1 Rounds (9.3)

A round's design flow is chosen in the iterate dialog or with `--design`:

| Value | Steps |
|---|---|
| **amend** (default) | Specifying, then the round's plan gate. The orchestrator submits a **spec amendment**, `submit_doc { kind: "spec", amend: true }`: new requirements continue the numbering, changed ones carry `(changed in round <k>)`. It is reviewed as in §4.2. |
| **full** | Brainstorming, then specifying (an amendment), then planning. |
| **off** | 9.3's round: straight to the round's plan gate. |

**Rules for rounds:**
- **Coverage.** The round's plan must cover the round's new and changed requirements. Unchanged requirements stay covered by earlier rounds.
- **Gates.** 9.3's rule holds: rejecting at any gate of round ≥ 2 drops only that round. A round the orchestrator starts always stops at every gate.
- **Committing.** On approval of the round's plan, the engine commits the amendment by appending `## Round <k> amendment` to the spec file, and commits the round's `plan.md`. In pr mode, that commit goes on the round's first new stage.

### 8.2 Next goals on the same orchestrator (9.3 chains)

- A next goal is a new goal. Its design flow defaults as in §1.
- The brainstormers' input pack includes the previous run's approved spec, marked `related earlier work`.
- A goal the orchestrator starts with `start_goal` stops at every gate (9.3's rule).

### 8.3 pr mode (9.2)

- The documents are committed as in §5.3.
- Each PR description gains `Covers R2, R5 (spec: <path>)`, built from its stage's tasks' `covers`.

### 8.4 Restart, lost window, history

- **Daemon restart.**
  - Open gates, versions and findings are restored.
  - Brainstormers and document reviewers running at the restart are relaunched fresh.
  - An orchestrator phase's clock excludes the downtime (9.5's pause bookkeeping).
- **Lost orchestrator window.** 9.3's handoff prompt for a fresh session includes the phase, the open gate and every approved document's path.
- **History.** `TaskRecord` is unchanged. A new `PhaseRecord` line per phase records:
  - the run id;
  - the phase;
  - its duration;
  - the brainstormers' and reviewers' calls and tokens;
  - the versions each gate needed;
  - the number of disputed findings.

  `HISTORY_VERSION` stays 5; older readers skip unknown lines. 9.5's refit uses these records for the design budgets.

## 9. The orchestrator contract

**New rules in the orchestrator's contract:**

- **The phases.** The brainstorming, specifying and planning phases, and what each tool does in each.
- **Questions.** Ask at most `max_questions` questions, one at a time, before `start_brainstorm`.
- **The merge.** Never copy a draft wholesale. Tag every approach, and always show disagreements.
- **The spec.** Use the template. Every requirement is testable and carries its acceptance check.
- **Plan coverage.** Every requirement gets a task, and every brief has the five headings.
- **Gates.** Never claim a gate is approved. Read verdicts from `run_status`.

**Tools.**

| Tool | Who may call it | When |
|---|---|---|
| `start_brainstorm { answers }` | orchestrator | in `brainstorming`, no brainstormers running |
| `submit_doc { kind, text, ready?, amend?, responses? }` | orchestrator (brainstorm, spec); brainstormer (brainstorm_draft) | in the matching phase |
| `get_doc { kind, version?, from? }` | orchestrator, document reviewer | any time in a design-flow run |
| `submit_findings { findings }` | document reviewer | while its review is open |
| `edit_plan { …, responses? }` | orchestrator | as today, plus `responses` at submit |

- Every tool is refused outside its phase or role, with exact text.
- 9.3's allowlist drift test covers the new tools.
- A Claude orchestrator's allowed-tools list gains them.

## 10. Protocol 17

**New fields.** All are serde-defaulted, and each has a round-trip test:

- **`RunInfo`:**
  - `phase`;
  - `gate: Option<GateInfo { kind, version, revising, disputed, changes_summary }>`;
  - `design: Design`.
- **`DocInfo`:** `{ kind, version, author, reason, bytes, requirements }`, listed in `RunInfo.docs`.
- **`TaskInfo.covers`.**
- **`AgentRole`:** gains `Brainstormer` and `DocReviewer`.
- **`RunRequest`:**
  - `GateApprove { kind }`;
  - `GateChanges { kind, note, review }`;
  - `GateEdit { kind, text }`;
  - `GateRethink { note }`;
  - `GateBack { kind, note }`;
  - `ShowDoc { kind, version, diff, findings }`.

  Each has a matching reply.
- **Start options.** `RunRequest::Start` and `Iterate` gain `design: Option<DesignChoice>`.

**Old snapshots** decode with the design flow absent, and older runs load with `design = Off`.

## 11. Safety and determinism

- **Brainstormers and document reviewers are read-only.** They use a scout's sandbox and tools. They never get a writable root.
- **They are headless** and never receive pasted input.
- **Every phase change comes from the user or a checked tool call.** The engine checks every submitted document mechanically, never by model judgment: sections, numbering, caps, coverage, brief shape and the response completeness.
- **Git.** The documents commit is the engine's, through the git write queue (§5.3).
- **Tests reach no real agent.** Every test pins every `*_BIN` to `fake-agent` or a nonexistent path. `fake-agent` gains `brainstormer` and `doc_reviewer` scripts.

## 12. Testing

**Engine and unit tests:**
- every state transition, including `back` and `rethink`;
- every refusal text;
- requirement parsing;
- the coverage check;
- the brief-shape check;
- version immutability;
- the gate surviving a restart;
- `--yes` not skipping a gate;
- one brainstormer failing;
- both failing;
- reviewer failure;
- the round amend flow;
- chain inputs;
- pr-mode commit placement.

**Proto.** Round trips for every new message, and old-snapshot decodes.

**End to end with `fake-agent`:**
- A full goal: questions, two drafts, the merge, gate 1, the spec with one review and one disputed finding, gate 2, the plan with a coverage refusal and then a pass, gate 3, the documents commit, the run completing, and REPORT.md's requirements table.
- `changes` at each gate.
- `back` from spec to brainstorm.
- `rethink`.
- A round with `amend`.
- pr mode: the commit on the bottom stage and the `Covers` line in the PR body.

**TUI.** The gate screen at 80×24 and 120×40, every key, the confirm pages, the diff view, the revising state, and the Plan doc tab.

**Smoke stage 11k.** A design-flow goal through all three gates in a real PTY, with fake agents, on its own daemon under `/tmp`.

## 13. Out of scope

- More than two brainstormers, or a debate round between them.
- An external `$EDITOR` for documents.
- Committing documents anywhere but the run's branch.
- The design flow for fast-path, research or review goals.
- Automatic approval of any gate.
- Editing `plan.md` as text: the plan is edited as tasks.

## 14. Risks

1. **Cost.** Two brainstormers plus reviewers add tokens to every planned goal. The budgets, the reader-slot caps, `design = off` and the history refit bound it. REPORT.md shows the design phases' spend separately.
2. **Template drift.** Models may ignore a template. The engine checks every section and refuses with exact reasons, so drift costs a resubmit, never a bad gate.
3. **Requirement churn across versions.** Renumbering after `changes` could break coverage references from an earlier plan. Requirements are parsed per version, and coverage is checked only against the approved spec.
4. **Coverage gaming.** A `covers` claim without matching work. The plan reviewer checks claims against briefs, and task reviewers judge per requirement.
5. **Long waits at gates.** A gate waits indefinitely and costs nothing while it waits. Alerts and the status bar keep it visible.
6. **Prompt size.** Workers get only their requirements, and sub-planners only their epic's sections. Document caps bound everything else.
