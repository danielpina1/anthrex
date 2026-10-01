# TUI end to end, and a visual design for every screen

Milestones **9.0.6** (TUI end to end) and **9.0.7** (TUI polish). Written 2026-10-01 from a brainstorm with the user. Sections 1–3 were approved one by one, and Sections 4 and 5 and the two-milestone split were approved with "lets go. implement".

This spec builds on the milestone 9.0.5 brief (plan review, Alerts, task panel) and the adaptive-orchestrator spec. Where they disagree on a TUI matter, this spec wins.

## 0. Intent

The user's words were: "In the TUI I want to be able to work end to end with no commands in a second terminal. I should be able to view and approve all actions needed from the TUI. I want to be able to create and modify the profile via the TUI. I also want a settings page that allows me to select the models to use on runtimes in a clean and simple way."

They later added: "improve parts of the TUI that are weaker, make everything more beautiful but maintaining the context of the terminal design and usability… I want the alert box bigger, optimize the views… clean and simple but really informative and detailed."

**What success looks like.** A user can do each of the following from one TUI, with no second terminal:
- take a repository from no profile to a confirmed profile with tiers;
- set their models;
- start a goal;
- review and approve the plan;
- answer a blocked worker;
- steer, retry or override tasks;
- accept or discard the finished run.

Each step shows the details needed to validate it before it is committed. At 80×24, every screen still shows the state, the outcome and the next key.

**Unchanged design rules.** Only the orchestrator is an interactive PTY window. Workers, reviewers, scouts and sub-planners stay headless and watch-only. The engine stays deterministic: the TUI only sends requests the CLI could already send. Nothing reaches the base branch until the user accepts.

**User choices from the brainstorm:**
- 9.0.6 goes before the rest of M9.2.
- Settings cover models and the key run limits.
- Settings apply live to new runs.
- Models come from fixed shipped lists, plus "custom…".
- Every selection gets an action menu.
- The daemon computes which actions are allowed (approach A).

## 1. The action menu (9.0.6)

### 1.1 Daemon-computed actions

Every `RunInfo`, `StageInfo` and `TaskInfo` in the live snapshot gains this field:

```rust
#[serde(default)] pub actions: Vec<ActionInfo>,

pub struct ActionInfo {
    pub kind: ActionKind,          // closed enum, below
    pub label: String,             // "accept", "answer", …
    pub effect: String,            // one line: what happens, with exact names
    pub needs: ActionNeeds,        // Confirm | Input(InputKind) | Open
    pub destructive: bool,         // drawn in the failure colour; needs `y`
    pub refused_why: Option<String>, // Some = listed but greyed, with the reason
}
```

`ActionKind`, grouped by the node it belongs to. Each entry names the existing request the TUI sends:

| Node | Kind | Request sent | Needs |
|---|---|---|---|
| run | `ReviewPlan` | opens the 9.0.5 plan review | Open |
| run | `Approve` / `Reject` | `RunRequest::Approve` / `Reject` | Confirm (reject is destructive) |
| run | `Submit` | `RunRequest::Edit{edits: [], submit: true}` | Confirm |
| run | `ApproveHold` / `RejectHold` | `RunRequest::ApproveHold` / `RejectHold` | Confirm |
| run | `Pause` / `Unpause` | `Edit{[PlanEdit::Pause]}` / `Edit{[PlanEdit::Resume]}` | Confirm |
| run | `Resume` | `RunRequest::Resume{rebaseline}` (a halted run) | Input(Resume) |
| run | `Cancel` | `RunRequest::Cancel` | Confirm, destructive |
| run | `Promote` | `RunRequest::Promote{orchestrator}` | Input(Promote) |
| run | `Accept` / `Discard` | `RunRequest::Finish{action, confirm}` | Confirm (discard is destructive) |
| run | `Stats` | `RunRequest::Stats` | Open |
| stage | `MessageStage` | `Edit{[Message{to: stage:n}]}` | Input(Message) |
| task | `Answer` | `Edit{[PlanEdit::Answer]}` | Input(Answer) |
| task | `Message` | `Edit{[PlanEdit::Message{to: task}]}` | Input(Message) |
| task | `Refresh` | `Edit{[PlanEdit::Refresh]}` | Confirm |
| task | `Retry` | `RunRequest::Retry` | Confirm |
| task | `Override` | `RunRequest::Override{reason}` | Input(Reason) |
| task | `CancelTask` | `Edit{[PlanEdit::CancelTask]}` | Confirm, destructive |
| task | `OpenConversation` | opens the conversation view | Open |

**One pure function.** The list comes from one pure engine function, `actions::available(&Run, Node) -> Vec<ActionInfo>`. The request handlers call the same function's predicate, so the menu and the daemon cannot disagree:
- A request for an action that is not available is refused with the same `refused_why` text.
- An action listed with `refused_why` is one the user would expect to see but cannot take now. For example, `accept` on a run with a task still running reads "2 tasks still running".

**Effect lines name exact things.** For example:
- `accept: merge 3 tasks into main@59705dc`
- `retry: fresh session, rung reset to 0`
- `discard: remove 3 worktrees and 4 local anthrex/0723/* branches`

Effects never contain text a model wrote unless it has passed `safe_text`.

### 1.2 The menu

Pressing `.` or Enter on a selected run, stage or task (in the run view, the overview or the sidebar's run rows) opens a small menu:
- `j`/`k` move, Enter picks and Esc closes.
- Entries with `refused_why` are muted and show their reason, and Enter on them does nothing but toast the reason.
- When the connection is down the menu shows "not connected" and offers nothing. Requests are never queued.

### 1.3 Forms and confirmation

**Input forms:**
- **Answer** shows the blocked task's question, the task's title and its brief excerpt, with a multi-line text box.
- **Message** has a kind (`info`, `change`, `stop_and_wait`) and a multi-line text.
- **Override** needs a non-empty reason.
- **Resume** has a `rebaseline` toggle. When `base_moved` is known it shows an explanation: the old and new base, and that rebaselining re-runs tier 3 against the new base.
- **Promote** offers an orchestrator picker over the roster.

**Confirmation pages.** Every action that changes state ends on a confirmation page. It repeats the effect line and the details that matter:
- **Accept** shows, per stage, the commits, the tier-3 (full-suite) result, the base and the target. If the base moved it shows the daemon's `ConfirmNeeded` prompt with `base_moved` and asks the user to type the run's short id, which is the CLI's `--confirm`.
- **Discard and reject** say what is removed and that nothing on the base branch changes.
- **Cancel** says which workers are stopped.

**Replies.** The daemon's reply is shown as a toast. A refusal toast carries the daemon's own text and the error severity.

### 1.4 Entry points

- **From an alert.** Enter on an alert opens the menu with the matching action preselected:
  - ready to accept → Accept
  - blocked (question) → Answer
  - halted → Resume
  - plan awaits approval → ReviewPlan
  - hold awaits approval → ApproveHold
  - profile proposal ready → the Profile screen
- **The goal form (`C-b g`)** gains two toggles, "approve at once" (`yes`) and "unconfined checks". Its model choice becomes a picker over the roster's enabled models, with "custom…" last.
- **Run stats** is a read-only screen opened from the menu. It renders `RunReply::Stats` as a table of per-route success, timing and flaky proposals.

## 2. The Profile screen (9.0.6)

`C-b P` opens the current project's profile, where the project is the selected window's or run's repository. The screen has three tabs.

**Status** (the default tab) shows:
- whether a profile is stored, and when it was confirmed;
- whether it is stale;
- the proposal in progress, in one of these states: the scout running with a live elapsed time, verification running, ready, or failed with its reason.

It offers two actions:
- **Detect**, with "trust project" and "unconfined checks" toggles, and a note that a real agent will read the repository.
- **Reject proposal**.

While a detection runs the TUI polls `ProfileRequest::Status` once a second. It stops polling when the screen closes.

**Profile** shows the stored profile, or the proposal, grouped like this:

| Group | Keys |
|---|---|
| Commands | setup, check, single test, sample test |
| Tiers | build check, module test(s), module graph, module names, toolchain id, slow / timing tests, full shards |
| Paths | modules, source, hub, generated, protected, manifests, test paths |
| Environment | `env.*` |

Every verified command shows its check result: ✓/✗, exit code, seconds, and the last lines of output on Enter. Commands dropped because they failed verification are listed with their reason. A proposal shows its difference from the stored profile, marked `+`, `−` and `~`.

**Edit:**
- `e` on any key opens an editor fitted to its type. A command or string gets a text line, a list gets a one-per-line editor, and `env.*` gets name and value.
- `u` unsets the key.
- Saving sends `ProfileRequest::Edit`, which makes a proposal and verifies it. The screen shows the verification result, and the user then confirms with `c` (then `y`) or drops it.
- "Store once verification passes" is a toggle; it is the CLI's `--yes`.

**Confirm** shows the exact TOML that will be stored, the same text the CLI's `confirm` stores, and asks for `y`.

**Rules:**
- Editing is refused while a run is live in the project, and the screen shows the daemon's reason.
- Enter on the "profile proposal ready" alert opens this screen on the proposal.

The TUI stops dropping `RunReply::Profile` and its related replies. No new daemon request is needed for the profile flow.

## 3. The Settings screen (9.0.6)

`C-b S` opens Settings. (`C-b ,` is taken.)

**Models.** There is one table per runtime, Claude and Codex.
- Each row is a model from anthrex's shipped list with a strength (fast / standard / frontier) and an on/off toggle. A last row, "custom…", lets the user type a model name and pick its strength.
- The shipped lists live in the config crate as one `const` per runtime:
  - **Claude:** `claude-haiku-4-5` fast, `claude-sonnet-5` standard, `claude-opus-5-5` frontier.
  - **Codex:** `gpt-6.1-sol` frontier, `gpt-6-sol` standard, `gpt-6-astra` standard, `gpt-6-luna` fast, `gpt-5.6-sol` standard, `gpt-5.6-terra` standard, `gpt-5.6-luna` fast, and "Codex default" (model `""`, standard).
- Saving with no enabled model is refused inline.
- A warning, not a refusal, shows when a strength has a model on only one runtime, because the cross-runtime reviewer then cannot be chosen for that strength.
- A saved roster is written as `builtin_models = false` plus one `[[orchestrator.models]]` table per enabled row, so what the screen shows is exactly what is used.

**Orchestrator.** The default runtime and model, picked from the enabled models.

**Limits.** Each field shows its allowed range, and a value out of range is refused inline:
- S/M/L budgets as tool calls and minutes, with the 1.5× hard stop shown beside each as text;
- stall time;
- max writers and readers;
- max bounces.

**Saving.**
- `w` saves through the daemon. The daemon validates with the same rules `config.toml` gets at start, writes, and reloads in place.
- The screen then says "saved · new runs use these settings · runs in progress keep theirs".
- A refused save lists the daemon's problems and writes nothing.
- A value that comes from the built-in defaults, because the file doesn't set it, shows "(default)".

## 4. Architecture, errors, testing (9.0.6)

### 4.1 Protocol 13

9.0.6 raises `PROTO_VERSION` from 12 to 13, and M9.2 renumbers to 14 when it resumes. Every new field has `#[serde(default)]`, so 12-era `run.json` still loads.

- **Snapshot.** The `actions` field described in §1.1, plus `ActionInfo`, `ActionKind`, `ActionNeeds` and `InputKind`.
- **Settings.** A new `ClientMsg::Settings(SettingsRequest)` with `Get` and `Put { settings: SettingsDoc }`. Its reply is `ServerMsg::Settings(SettingsReply)`, which is one of:
  - `Current { doc, origin }`, where `origin` marks each value as `File` or `Default`;
  - `Saved { doc }`;
  - `Refused { problems: Vec<String> }`.

  `SettingsDoc` holds the roster, the orchestrator default and the limits from §3, and nothing else.
- **A round-trip test** covers every new message and field, as hard rule 4 requires. The CLI client and `anthrex hook` compile against the new version in the same change.

### 4.2 Config writer and live reload

- **The writer.** The config crate gains a writer that edits only the keys Settings owns: `[orchestrator]` `builtin_models` and `models`, the budgets, and the limits. It leaves every other line untouched, comments included.
  - It works line by line on the TOML text. `toml_edit` is acceptable if it is already in the dependency tree; otherwise it is added to the config crate only.
  - It writes a temporary file in the same directory, then renames it over the original.
  - A missing file is created.
- **The reload.** The daemon holds its loaded `Config` behind a lock separate from the manager lock. `Put` runs on `spawn_blocking`, outside the manager lock, with a timeout. It parses, validates, writes, re-reads and swaps.
  - New runs, goals and promotions read the swapped config.
  - Runs in progress keep their frozen roster and limits, which `Run.roster` and the budgets already freeze.
- **The path** is `ANTHREX_CONFIG` when it is set, otherwise the default config path.

### 4.3 TUI modules

These are new pure state modules, which return `Vec<Effect>` and do no I/O, and their renderers, which take `&App`:

| State | Render | What it holds |
|---|---|---|
| `app/actions.rs` | `ui/action_menu.rs` | menu, forms, confirm pages, alert preselection |
| `app/profile_screen.rs` | `ui/profile.rs` | Profile screen tabs, editors, polling |
| `app/settings.rs` | `ui/settings.rs` | Settings screen, inline validation |
| `app/stats.rs` | `ui/stats.rs` | stats table |

The TUI also changes in these ways:
- It handles `ConfirmNeeded` and `base_moved` instead of dropping them at `app/runs.rs`.
- Help gains every context's keys (§6.7).
- Every displayed string passes `safe_text`.
- Nothing overflows its pane.

### 4.4 Errors

- Every daemon refusal is shown with its exact text.
- A settings save that fails validation writes nothing.
- A profile edit refused during a live run shows why.
- A lost connection disables the action menu and the Settings `w` with "not connected".
- A request with no reply within its timeout toasts "no reply from daemon" and leaves state unchanged.

### 4.5 Testing

- **`available()`** is tested for every run, stage and task state, including halted, paused, gated, holds, pr-mode and complete runs. Each case also asserts that a request for an action outside the list is refused with the same reason.
- **The config writer** is tested on a round trip that preserves unrelated comments and sections, and on a refused save that leaves the file byte-identical.
- **Reload** is tested end to end: a running run keeps its limits while a new run gets the new roster.
- **TUI reducer and render tests** run at 80×24 and 120×40.
- **End-to-end scenarios** use fake-agent only, with every `*_BIN` pinned:
  - goal → plan → approve → answer a blocked question → accept, all through the requests the TUI sends;
  - a profile edit, then confirm;
  - a settings save, then a new run that uses the new roster.
- **`scripts/pty-smoke.py`** gains a stage that opens the action menu and Settings.
- **Every test points `ANTHREX_CONFIG` at a temporary file.** No test reads or writes the user's config.

### 4.6 Out of scope

These are follow-up candidates:
- editing plans at run time (add, split, deps) beyond answer and message;
- starting a run from a plan file in the TUI;
- model discovery.

## 5. The design kit (9.0.6) and principles (both milestones)

### 5.1 Principles (binding for every screen)

1. **"Needs you" has one look.** It has one colour role, attention, and one glyph, `⚑`, and is more prominent than anything else on screen. Working is never drawn in the attention colour.
2. **The accent means "your keys are here".** Each frame has exactly one accented border: the region that receives keystrokes. Titles, related nodes and the critical path use weight, not the accent.
3. **Colours are roles, not RGB values.**
   - The roles are `accent`, `attention`, `working`, `done`, `failed`, `muted` and `paused`.
   - They resolve to the 16 ANSI colours by default. Truecolor is used only when `COLORTERM` is `truecolor` or `24bit` and `[theme] truecolor` is not `off`; it can be `auto` (the default), `on` or `off`.
   - Anything the user would act on is never drawn only in `muted`.
4. **One glyph, one meaning.** Runtimes become text tags (`cl`, `cx`, `sh`). Every glyph has an ASCII twin, and `force_ascii` covers the whole TUI. The table:

   | Glyph | ASCII | Meaning |
   |---|---|---|
   | `✓` | `+` | passed / finished |
   | `✗` | `x` | failed (replaces `✕`) |
   | `◌` | `.` | not started |
   | `○` | `o` | idle / held |
   | `⚑` | `!` | needs you |
   | spinner | `-\|/` | working |
   | `▸` | `>` | collapsed only |
   | `▌` | `>` | selection bar |
   | `›` | `>` | sequence separator |
   | `⚠` | `!` | warning |

5. **One name per object.**
   - A run is always its goal cut to fit, plus `·` and its short id (the last 4 characters of its id), for example `Add mul() · 0723`.
   - A task is `t2 <title>`.
   - "Stage" means only a 9.1 delivery stage. The lifecycle is **phase** and the done → proof → check → review strip is **pipeline**.
6. **80×24 is the design target.** The state word, the outcome and the next key show without scrolling. Panels size to their content. Anything cut is marked (`↓ 7 more`, `… b`), and what is cut is chosen by priority.
7. **Outcome first, then evidence, then intent.**
8. **The keys you need are on screen.**
   - Every mode has a badge.
   - Hints are width-aware: they drop whole by priority and always keep `esc`.
   - `C-b` pending lists the next keys.
   - Help is grouped by context and scrolls.
9. **One dialog grammar.**
   - Labels are lower case. The choice widget is `‹ value ›`.
   - Hints read `⏎ action · tab next · esc cancel`.
   - Dialogs have one column of padding and are capped at 64 columns; text wraps at 60.
   - Destructive dialogs name what is lost, draw the title and action word in `failed`, and confirm only on `y`. Bare Enter never confirms them.
10. **Calm by default.** Borders are muted, and only real progress animates.

### 5.2 The kit (built first in 9.0.6, so the new screens use it)

- **`theme.rs`** has the roles. A function `role(Role) -> Style` resolves each role through the truecolor decision, and the glyph table has the ASCII twins.
- **`ui/kit/`** holds the shared widgets:
  - `hints(width, &[Hint]) -> Line`, which drops whole hints by priority and keeps `esc`;
  - `dialog_frame(title, destructive)`;
  - `choice("‹ v ›")`;
  - `text_area(rows)`;
  - `labelled rows` with an aligned label column and two spaces before the value;
  - `scroll_marks` (`↑`/`↓ n more`);
  - `run_name(goal, id, width)`.
- **The status bar** gets:
  - mode badges `RUN`, `PLAN`, `ALERTS`, `CHAT`, `PROFILE`, `SETTINGS` and `MENU`;
  - keys drawn in the accent and the words around them in muted;
  - the prefix list `C-b › a alerts · g goal · T run · P profile · S settings · ? help`;
  - toast severity (info, warn, error).

The new screens in §§1–3 are built only from the kit. 9.0.6 moves existing screens to the kit only where it touches them anyway (the goal form, confirm modals, the status bar). Everything else is 9.0.7.

## 6. Screen-by-screen polish (9.0.7, no protocol change)

These findings come from a render audit of every screen at 120×40 and 80×24, run on 2026-10-01 against main at `fe1a381`.

### 6.1 Alerts

**The sidebar box grows into the free space.** The agents list takes the rows it needs, and Alerts get the rest: at least 3 rows, up to half the sidebar. Each alert has two lines: who it is for (the run name, `› task`, and its age), then what happened, wrapped. The title reads `⚑ Alerts n` in `attention` when n > 0.

```
╭ ⚑ Alerts 3 ─────────────────────╮
│⚑ Add mul() · 0723          2m   │
│  plan awaits approval · 2 tasks │
│⚑ Add mul() · 0723 › t2     41s  │
│  blocked: which crate owns the  │
│  formatting helper?             │
│✓ Fix CI · 9b1e             now  │
│  complete · ready to accept     │
╰──────────────────── C-b a open ─╯
```

**`C-b a` opens a full Alerts view in the main pane.** It shows the list on the left and the full detail on the right: the full text, the run, the task, the agent and the age. It also shows the actions, with Enter on the preselected one (§1.4).

```
╭ ⚑ Alerts ─────────────────────────────────────────────────────────────────────╮
│ P1 ⚑ Add mul() · 0723 › t2   41s │ t2  report_product in crate c              │
│ P2 ⚑ Add mul() · 0723         2m │ phase    blocked · question                │
│ P4 ✓ Fix CI · 9b1e           now │ asked    Which crate owns the formatting   │
│                                  │          helper: b or c? The brief says c  │
│                                  │          but b already exports fmt_sum().  │
│                                  │ worker   cx gpt-6-sol · 12 calls · 4m      │
│                                  │ ⏎ answer   m message   o open task         │
╰──────────────────────────────────┴────────────────────────────────────────────╯
 ALERTS  j/k move  ⏎ answer  o open  esc back
```

**Colour.** Priorities no longer reuse status colours: P1–P3 are `attention` (P1 bold) and P4 is `done`.

### 6.2 Task panel: outcome first

- **The order is:**
  1. the title row with the state word, always shown;
  2. **OUTCOME**: the pipeline, the check, and the acceptance criteria ticked against the result (`✓` met, `✗` unmet, `◌` not yet judged);
  3. **EVIDENCE**: the diff stat and the review verdict;
  4. **INTENT**: the brief, one line, with `b` to expand;
  5. a footer: stage, route, size and test mode.
- At 80 columns the state word moves under the title rather than being dropped.
- `✓` and `✗` are coloured.
- The edit form's `stage` field keeps its name. The lifecycle field is renamed `phase`, and `stage no.` becomes `stage`.

```
│ ◐ t2 map Gemini hook events     in review · r2 │
│ OUTCOME                                        │
│  pipeline  done ✓ › proof ✓ › check ✓ › review │
│  check     ✓ passed  cargo test -p gemini 4.1s │
│  accept    ✓ Stop marks the window idle        │
│            ◌ SubagentStop pairs with Start     │
│ EVIDENCE   +142 −18 · 3 files · review r2 …    │
│ INTENT     Map Gemini CLI hook events… (b)     │
│ stage 1 of 2 · cx gpt-6-sol · M · tdd          │
╰──────────────────────────── ↓ PgDn · . act ────╯
```

### 6.3 Run view

- **The inspector panel takes only the rows its content needs.** This replaces the fixed 18-row `RUN_INSPECTOR_TALL_HEIGHT`, and the graph gets the rest.
- **Below 30 rows of interior height, the graph becomes a compact list**, one row per node with its state and its deps.
- **Stage nodes always show their tier-3 state.** A red, bisecting stage is drawn in `failed` with `✗ bisecting`, never as a working spinner. The stage panel wraps failing test names instead of cutting them.
- **Deps read `after t0, t6`.**
- **Selection is drawn with the reverse highlight plus the selection bar.** Related nodes are bold only.

```
╭ run · Add mul() · 0723 ─────────────────────────────── 2/3 merged · 14m ─╮
│ ✓ stage 1/2   tier 3 ✓ 38s                                               │
│   ✓ t1 add mul() to a                     S tdd    merged                │
│ ◐ stage 2/2   tier 3 ◌                                                   │
│▌  ◐ t2 report_product in c                M tdd    review r1   after t1  │
│   ◌ t3 docs for report_product            S none   waiting     after t2  │
╰──────────────────────────────────────────────────────────────────────────╯
```

### 6.4 Plan review

- **It gets a bordered frame and a summary header**, showing:
  - the number of tasks, epics and stages;
  - the sizes;
  - an estimated budget, the sum of the size budgets in tool calls;
  - the critical path;
  - `⚠` warnings for owns overlaps between tasks that can run at the same time;
  - implicit deps, marked `(implied)`.
- **The list has aligned columns** (title, route tag, size, test mode, stage, deps), so the deps survive at 80 columns.
- **Selection uses the reverse highlight across the task's rows.**
- **The detail pane uses the same labelled-row layout as the task panel.**

```
╭ plan · Add mul() · 0723 ─────────────────────────────────────────────────╮
│ 3 tasks · 2 stages · S+M+S · ~190 calls · critical t1 › t2 › t3          │
│ ⚠ t2 and t3 both own crates/c/src/lib.rs                                 │
├──────────────────────────────────────────────────────────────────────────┤
│▌t1 add mul() to a        cx sol   S tdd   stage 1                        │
│ t2 report_product in c   cl opus  M tdd   stage 2  after t1              │
│ t3 docs                  cl haiku S none  stage 2  after t2 (implied)    │
```

### 6.5 Sidebar and panes

- **Overflow** shows `↓ n more`, and the blank spacer row is removed.
- **Run rows keep their column alignment.** Progress reads `2/3 ✓`.
- **The pane title** is `<tag> <name> · <dir>`, so it no longer repeats `shell · shell`.
- **The conversation title** drops `rev N`. User turns use `›` and the label `you`, and zero durations are hidden.

### 6.6 Status bar

- **Every mode uses the width-aware hints from the kit.** `esc` is never cut.
- **`⚑ n C-b a`** shows whenever the sidebar is hidden.
- **The overview, the run view and the sidebar tree get distinct badges:** `TREE`, `RUN` and `OVERVIEW`.

### 6.7 Help

Help is a scrollable modal, grouped as follows:
- Global
- Sidebar
- Run view
- Plan review
- Alerts
- Conversation
- Action menu
- Profile
- Settings

It opens on the current context's group. `j`/`k`/PgUp/PgDn scroll it, and `esc` closes it.

### 6.8 Dialogs

All dialogs follow principle 9:
- the new-agent dialog, the rename and kill confirms, and the reject confirm;
- the goal field, which is 4 rows with a placeholder;
- the edit form's brief, which is a 4-row text area.

### 6.9 Testing (9.0.7)

**Render tests at 80×24 and 120×40** for every screen in §6. They assert:
- the state word and the `esc` hint are visible;
- exactly one accent-coloured border per frame;
- no actionable text only in `muted`;
- ASCII mode emits no non-ASCII glyph.

**A theme test** asserts that every role resolves to an ANSI colour when truecolor is off.

## 7. Delivery

| Milestone | Holds | Protocol |
|---|---|---|
| 9.0.6 | §§1–5: the kit, the action menu and forms, the Profile and Settings screens, stats, the settings protocol and live reload | 13 |
| 9.0.7 | §6: polish of every existing screen to the kit | 13 (unchanged) |

M9.2 resumes after 9.0.7 merges and renumbers to 14. Each milestone is one PR, and the user merges it.
