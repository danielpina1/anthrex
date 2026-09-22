# Milestone 8c: The live run view

> Written 2026-09-22 against `docs/superpowers/specs/2026-09-22-adaptive-orchestrator-design.md` ("the spec" below), which is binding for this milestone; its §16 is this milestone's whole subject. Built on the M8a brief `docs/milestones/M8a-orchestration-engine-core.md` (its final, headless version) for every run type, wire message and window rule. Grounded in the client as it stands on `main` at `2cb7e3c` — `crates/tui/src/tree.rs`, `tree/`, `graph/`, `inspector.rs`, `inspector/`, `app/`, `tree_input.rs`, `mouse.rs` and `ui/` — and in the milestone 6.5 conversation view as its brief (branch `m6.5-conversation-view`, `docs/milestones/M6.5-conversation-view.md`) specifies it. The graph overview (milestone 4.6, `docs/superpowers/specs/2026-09-20-graph-overview-design.md`) and the node inspector (milestone 4.7, `docs/superpowers/specs/2026-09-21-node-inspector-design.md`) are extended, never duplicated: there is one graph widget and one inspector.

## Header

| | |
|--|--|
| Status | `blocked` — becomes `ready` when milestone 8a is `done`. |
| Depends on | Milestone 8a only (which itself needs 5, 6 and 6.5). Milestone 8b may land before or after this one; see "Consumes from later milestones". |
| Spec sections | §3 (information flows down through planners, up through the engine), §4 and §4.2 (only the orchestrator is an interactive PTY; every other agent is headless and watched, never addressed), §12.3 (the plan gate is shown and edited in the run view), §16.1–§16.5 in full, §21 (the M8c row). Milestone 4.6's spec §4.2–§4.6 and milestone 4.7's spec §3–§5, unchanged except where decisions below extend them. The conversation-view spec §6 and decision 11 (read-only). |
| Branch | `m8c-live-run-view` |
| Protocol version | **One above `PROTO_VERSION` on `main` at the moment this milestone starts.** Task M8c.1 adds snapshot fields (AGENTS.md rule 4). Read `crates/proto/src/lib.rs` on `main` that day, add one, and record the derivation under "Implementation notes". Milestone 8b also bumps the protocol; the two run one after the other (`docs/ROADMAP.md`), never at once. No acceptance criterion greps for a specific number. |

## Starting point

Names below are real on `main` at `2cb7e3c` unless the row says otherwise. If the merged code differs when this milestone starts, use the real names and record the mapping under "Implementation notes".

| From | What this milestone uses |
|------|--------------------------|
| M4 / M4.5 (`crates/tui/src/tree.rs`, 459 lines) | `NodeKey { Project(PathBuf), Window(u32), Subagent { window_id, id } }` with the comment `// Milestone 8 adds Run(..)`. `ProjectChild::Window` with `// Milestone 8 adds Run { .. }: run rows sit above plain windows and own their windows`. `RowKind<'a> { Project { root, name, status, counts, collapsed }, Window { info, position, has_subagents, collapsed }, Subagent { info } }` with `// Milestone 8 adds Run { .. }`. `Row<'a> { key, guides, depth, kind }`. `TreeState { collapsed, filter, selected, sidebar, overview, keep_finished_secs, .. }` with `toggle`, `select`, `move_selection`, `repair_selection`, `selected_index`, `prune(&[WindowInfo])`. `build(&[WindowInfo], &TreeState) -> Vec<Row>` (groups by `WindowInfo.project`, rolls status up by `urgency`, `expect`s every project to have a window). `agent_order(&[Row]) -> Vec<u32>`, `row_index`, `urgency`, `subagent_label`, `format_elapsed`. `tree/rows.rs`: `guide_prefix`, `visible_windows`, `SubagentWalk`, `emit_subagents(rows, walk, nodes, ancestors, depth, ancestor_matches)`. `tree/forest.rs`: `subagent_forest(&[SubagentInfo], keep_finished_secs)`. |
| M4.6 (`crates/tui/src/graph/`) | `graph::layout(&[Row]) -> Layout { nodes: Vec<PlacedNode { key, rect, depth }>, edges, size }`, `MIN_NODE_WIDTH` 12, `MAX_NODE_WIDTH` 30, `NODE_HEIGHT` 3, `TIER_GAP` 3, `ROW_GAP` 1, private `BORDERS_AND_PADDING` 4 and `GLYPH_COLUMNS` 2, `pub(crate) fn content_text(&Row) -> String`. `graph/paint.rs` (475 lines): `paint(&Layout, Rect, Pan, &[Row], &App) -> Vec<Line>`, `node_rows`, `interior_slots`, `glyph_and_color`, `border_style`, `is_selected`, the edge painter. `graph/viewport.rs`: `Pan { x, y }` with `clamped`, `revealing`; `GraphGeometry { area, pan }` with `node_at`. |
| M4.7 (`crates/tui/src/inspector.rs`, 300 lines; `inspector/panel.rs`, 344) | `INSPECTOR_HEIGHT` 8, `MIN_INTERIOR_FOR_PANEL` 14, `Field { label: &'static str, value, wrap }`, `Inspection { glyph: Span<'static>, name, fields }`, `inspect(&Row, &App) -> Inspection`, `panel::render(frame, &Inspection, area)` (rounded block, `Padding::horizontal(1)`, title row, column packing by `pack`, the wrapping field). |
| `crates/tui/src/ui/overview.rs` (236) | `areas(main, inspector_visible) -> (canvas, footer)`, `View { canvas, footer, layout, pan }`, `view`, `view_of`, `render` (block title `" tree overview "`), `footer_line`, `footer_parts`. |
| `crates/tui/src/tree_input.rs` (310) | `App::rows`, `set_graph_viewport`, `reveal_tree_anchor`, `reveal_graph_selection`, `enter_overview`, `enter_tree_navigation`, `exit_tree`, `run_tree_command`, `on_tree_key` (`j k h l`, arrows, `Enter`, `Space`, `i`, `/`, `Esc`), `activate_tree_node`, `select_tree_parent`, `select_first_visible_child`, `toggle_tree_node`, the filter keys. Several of these call `tree::build(&self.windows, &self.tree)` directly. |
| `crates/tui/src/app/` | `mod.rs` (586 lines; 592 after M6.5): `Effect { Send, Quit, Bell, Reconnect }`, `PendingAction { Kill, Restart, StopDaemon }`, `Modal { Confirm { message, action }, Help, NewAgent, Remove, ForceRemove, Notice, Rename }`, `TreeInput`, `App` (fields `windows`, `focused`, `tree`, `tree_input`, `overview`, `inspector_visible`, `graph_pan`, `graph_area`, `keymap`, `settings`, `modal`, `spinner_frame`, private `windows_received_at: Instant`, `toast`), `focus`, `on_daemon`, `on_key`, `run`, `on_paste`, `on_tick`, `age_secs`. `windows.rs`: `replace_windows`. `link.rs`: `on_reconnected`, `on_send_failed`, `retry_dropped_subscribe`. `lifecycle.rs`: `perform(PendingAction)`. `modal_keys.rs`: `on_modal_key`. |
| `crates/tui/src/mouse.rs` (212) | `on_click` (sidebar rows via `tree::build`; `click_graph` selects on a single click, `activate_tree_node` on a double click), `on_drag`, `on_scroll`. |
| `crates/tui/src/ui/` | `tree_view.rs`: `narrow_line`, `truncate`, `cut`, `counts_text`. `statusbar.rs` (342): the `TREE` badge and the navigate hint `j/k move  ⏎ focus  space fold  / filter  esc back`. `modal.rs`: `render` dispatch. `dialog.rs`: `LABEL_WIDTH` 11, `MARKER_WIDTH` 2. `crate::dialog`: `TextInput`, `apply_text_key`. `crate::theme`: `status_glyph`, `status_color`, `subagent_glyph`, `border`, `border_focused`, `muted`, `SPINNER`. |
| `scripts/` | `pty-smoke.py` (1572 lines, far over 600), `pty_tree_smoke.py` (the graph stages). M8a adds `scripts/pty_smoke_run.py` with `run_stage(env, bin_path)`, printing `== stage 11c: …`, called before `== stage 12: stop the daemon, verify status ==`. |
| M6.5 (by its brief) | See "Names taken from M6.5". |
| M8a (by its brief) | See "Names taken from M8a". |

## Goal

In `C-b T`, a run appears as one node under its project, above the project's plain windows. Selecting it and pressing `l` or Enter opens the **run view**: the same canvas and the same inspector, rooted at the run's orchestrator. Its tiers read left to right as the spec's information flow — scouts, sub-planners and the tasks the orchestrator planned itself; each sub-planner's tasks; each task's agent rounds (worker sessions, and every review round as its own node, with a worker sent back drawn again as `worker #1 r2`); their sub-agents. Every node carries a live glyph; the critical path has a bold border; selecting a task lights its dependencies and dependents and dims the rest; `f` filters to running work, blocked work or one runtime. The inspector below grows to twelve rows and shows a progress line and the facts behind it for the run, a sub-planner, a task, an agent round or a scout, from the pushed run snapshot and nothing else. Enter on the orchestrator focuses its PTY window, the one place the user types; Enter on any other agent opens its milestone 6.5 conversation, live and read-only — there is no terminal for a headless agent and nothing in the client ever offers input to one. Before a run starts, the run view is its plan gate: `a` approves, `x` rejects, `e` edits a task's route, brief, size and test mode, `d` removes a task, each sending milestone 8a's run requests.

## Scope

In:

- A small protocol addition (M8c.1): the snapshot fields the view reads that M8a does not carry and no later milestone owns, plus the `#[serde(default)]` placeholders for the fields later milestones fill.
- The client's subscription to `RunsSnapshot` and its handling of `RunReply`.
- `NodeKey::{Run, Planner, Scout, Task, AgentRound}` and the matching `RowKind` variants; runs in the project tree and the sidebar; the run-view rows.
- Node content rows, live glyphs, the critical-path border, the dependency highlight, finished-node dimming, the three filters.
- Opening, navigating and leaving the run view; Enter routing to the orchestrator's window or an agent's conversation; the mouse.
- The per-kind run inspector, `RUN_INSPECTOR_HEIGHT`, the one-field-per-row panel layout with a right-aligned title.
- The plan gate: approve, reject, edit, remove.
- A PTY smoke stage.

Out, each with its owner:

| Out | Owner |
|-----|-------|
| The scout snapshot type, the onboarding scout, the fast path, `RunInfo.usage` (usage by role, OTLP), `TaskInfo.diff` | M8b |
| Filling `RunInfo.scouts` with a run's area scouts | M9 |
| `estimate_left_secs`, `bound_ratio_permille` (history-derived estimates) | M9.5 |
| Creating the orchestrator's PTY window, `AgentRole::Planner`, filling `RunInfo.planners`, steering, `edit_plan`, adding tasks from the gate | M9 |
| Racer and test-writer rounds (`AgentRole` variants for them) and their node labels | M9.5 |
| Typing to, killing, restarting or removing any headless window | never — the spec §4.2 forbids it; the daemon refuses it (M8a decision 49) |
| Accept, discard, cancel, retry, override, resume from the TUI | not in the spec's §16; `anthrex run …` does them |
| Editing a running (approved) run's tasks from the TUI | M9 (steering through the orchestrator) |
| Dependency lines drawn across the tree | never — spec §16.3 decides against them |
| Opening a sub-agent's own conversation directly from its node | M6.5's descent (`Enter` on a spawn inside the parent's conversation) already does it |
| Split panes | M7 |

## Design decisions

Numbered and final. If one proves wrong or impossible, stop work on it, record the evidence under "Implementation notes", and continue with the tasks that do not depend on it.

### The data

1. **One pushed snapshot, no polling.** The client sends `ClientMsg::Run(RunRequest::Subscribe)` once per connection — right after `App::new` in `lib.rs`, and again from `on_reconnected` — and keeps the latest `RunReply::Snapshot` in `App.runs`. Every snapshot replaces the previous one whatever its revision: within one connection M8a's watch only ever moves forward, and after a daemon restart the global revision starts again, so a "newer revision only" rule would ignore the restarted daemon forever. A refused send of the subscription is retried by `on_tick`, like M6's dropped `Subscribe`. *(Spec §16.5; M8a decision 47.)*
2. **Time comes from the snapshot.** M8c.1 adds `RunsSnapshot.now` (the daemon's unix seconds at publication). An age is `runs.now − t + runs_received_at.elapsed()`, the pattern `App::age_secs` already uses for windows, so elapsed times tick on the client's clock between pushes, with no clock skew between client and daemon, and the pure view never reads the wall clock. *(Spec §16.5 "Elapsed times … tick on the client's clock".)*
3. **The view derives nothing the snapshot and the window list do not carry.** Where a mockup value has no source, the field is omitted, or the value is added to the snapshot by M8c.1 when it is cheap and exact, or it is a placeholder a later milestone fills. The inspector and every `ui/` file stay pure (AGENTS.md rule 5). *(Spec §16.4 last paragraph.)*
4. **M8c.1 adds, computed by M8a's pure `run/snapshot.rs` from the engine model:** `RunsSnapshot.now`; `RunInfo.approved_hhmm`, `plan_edits`, `plan_edits_since_approval`; `TaskInfo.brief`, `acceptance`; `AgentRoundInfo.rate_limited_since`, `sent_back_at`. The model gains `Run.approved_at`, `Run.plan_edits`, `Run.plan_edits_since_approval`, `AgentRound.rate_limited_since`, `AgentRound.sent_back_at`, each `#[serde(default)]` so existing `run.json` files load. These are the only daemon changes in this milestone.
5. **Later milestones' fields exist before they are filled.** When M8c starts and a field in "Consumes from later milestones" does not exist yet, M8c.1 adds it exactly as listed, `#[serde(default)]`, and nothing populates it; the view renders its absence. When it already exists (M8b or M9 landed first), M8c uses it and records any name difference under "Implementation notes".

### Where runs appear

6. **Runs shown in the tree are the non-terminal ones:** every `RunInfo` whose state is not `accepted`, `discarded` or `failed`. `anthrex run status` shows the rest.
7. **A run is one node under its project**, `NodeKey::Run(run_id)`, depth 1, above the project's plain windows, runs ordered by `created_at` then `run_id`. It is a **leaf in the project tree**: its windows are not listed as plain windows, and its tasks appear only in the run view. The project is `RunInfo.project`; a project that has a run and no window still gets its project row (today's `build` `expect`s a window — that invariant goes). *(Spec §16.1 first paragraph; `tree.rs`'s milestone-8 comments.)*
8. **Which windows a run owns.** A window whose `info.run` is `Some(r)` and whose `r.run_id` names a run shown in the tree (decision 6) is hidden from the plain window list. Every other window — including a headless window of a run the snapshot does not name (no snapshot yet, a run discarded or accepted, a retired reviewer that outlived its run) — is listed as a plain window, exactly as today.
9. **Project status and counts.** A project's rolled-up status is the most urgent (`tree::urgency`) of its plain windows' statuses and its shown runs' statuses, where a run maps to a status: `awaiting_approval`, `paused`, `halted` → `Attention`; `running` → `Attention` when any task is `blocked`, else `Working`; `complete` → `Done`. Runtime counts count plain windows only.
10. **The orchestrator's window and the number keys.** A run whose orchestrator window is listed (a window with `run == Some(RunRef { run_id, role: AgentRole::Orchestrator, .. })`) takes the next tree position, shown on its sidebar row, and `tree::agent_order` yields that window's id at that point, so `C-b <n>`, `C-b j`/`k` and `ensure_focus` reach the orchestrator and nothing else of the run. A run with no orchestrator window (every run until M9, and fast-path runs) has no position.

### The run view

11. **The run view is the overview with a different root.** `App.run_view: Option<RunView { run_id, filter }>`. While `overview` is on and `run_view` is `Some`, `App::nav_rows()` returns `tree::run_rows(..)` instead of `App::rows()`; the canvas, the painter, the inspector, the reveal, the selection keys and the mouse all read `nav_rows()`. The sidebar keeps showing `App::rows()`. The root's key is `NodeKey::Run(run_id)` — the same key as the project-tree node — so the selection needs no translation on the way in or out. *(Spec §16 "extended, not replaced".)*
12. **Tiers.** Root, depth 0: the run (`RowKind::Run`). Depth 1: the run's scouts, then its sub-planners and the tasks no sub-planner owns, interleaved as in decision 13. Depth 2: each sub-planner's tasks (a task belongs to the planner whose `epic` equals the task's `epic`; a task whose `epic` names no planner belongs to the root). Below a task, one tier deeper: its agent rounds. Below a round, a scout or a planner with a listed window: that window's sub-agents, by `tree::subagent_forest` and `emit_subagents`, exactly as under a plain window. The orchestrator window's own sub-agents are not drawn (its conversation shows them). *(Spec §16.1.)*
13. **Order among siblings.** Scouts by `started_at`, then `id`. Tasks under one parent by `wave`, then plan order (their index in `RunInfo.tasks`). A sub-planner sorts among the root's tasks as if it were its earliest task by `(wave, plan index)`; a planner with no task yet sorts after every root task, in `RunInfo.planners` order. Agent rounds by start time, a worker before a reviewer on a tie. *(Spec §16.2 "Order among siblings".)*
14. **Agent rounds.** Each `AgentRoundInfo` of role `Worker` becomes `1 + sent_back_at.len()` nodes: round `n` starts at `started_at` for `n = 1` and at `sent_back_at[n − 2]` after, and ends where the next starts; the last ends at `ended_at`. Every reviewer `AgentRoundInfo` is one node, its number the review round (`AgentRoundInfo.round`). If M8a already appends a new worker `AgentRoundInfo` per bounce with `round > 1`, `sent_back_at` stays empty and the entries map one to one; both shapes draw the same nodes. The session's counters (turns, tool calls, tokens) belong to its last display round. The window's sub-agents hang under the last display round only, so no node key repeats. *(Spec §16.1 "Review calls are nodes", "worker #1 r2", "A fresh session … is a new node (worker #2)".)*
15. **Node keys.** `NodeKey::Run(String)`, `Planner { run, epic }`, `Scout { run, id }`, `Task { run, id }`, `AgentRound { run, task, role: proto::AgentRole, session, round }` with `round` the display round. The spec's `AgentRound { task, role, session, round }` gains `run` because two runs can have the same task id. *(Spec §16.2.)*
16. **Finished nodes stay.** A round, scout or planner that has ended is drawn with `Modifier::DIM`, never removed, even after M8a retires its window (`RETIRE_AFTER`, 30 s). Its inspector then says `window closed`. *(Spec §16.1 "Finished agent nodes are drawn dim, never removed", §16.2 "Finished sub-planners".)*

### What the canvas shows

17. **One content row per box**, the text in Interfaces "Node content". Task text is pre-fitted to the widest box (decision 18) so its size, hub mark and dependency list survive truncation of the title. Agent rounds name their runtime as a word (`claude`, `codex`), not with M6.5's badge: a node's content is plain text measured by `graph::content_text` before it is painted, and Codex's badge `◇` is also the spec's check glyph. *(Spec §16.3 table.)*
18. **Task text fitting.** `TASK_TEXT_MAX = MAX_NODE_WIDTH − 4 − 2 = 24` columns (borders and padding, then the glyph and its space). With `tail = " {size}" + (" ◆" if hub) + ("  ⇠" + declared deps concatenated, if any)`: if `width(id) + 1 + width(tail) + 2 ≤ 24`, the text is `{id} {truncate(title, 24 − width(id) − 1 − width(tail))}{tail}`; otherwise it is `truncate("{id}{tail}", 24)`. `truncate` is `ui::tree_view::truncate`.
19. **Glyphs.** Task: from its state, per Interfaces "Glyphs"; `working` animates (the spinner) only while its live worker round's window is `Working`, which is "animated while output is flowing". Run: `◉` in its state's colour. Rounds: live → the spinner when the window is `Working`, `◆` when the window is `Attention` or the round is rate-limited, else `●`; a finished worker round `✓`, or `✗` when it is the last worker round of a `blocked` task; a reviewer `✓` (approved, or changes with only minor findings), `✗` (blocking), `●`/spinner (live, no verdict), `–` (ended with no verdict). Scouts and planners by their own state. Merged or approved `✓` is green, `✗` red, `⊘` and `◆` the attention colour. *(Spec §16.3 glyph list.)*
20. **Critical path, dependencies, dimming.** A task with `on_critical_path` has a bold border. While the selection is a task, its declared and implicit dependencies and its dependents (tasks whose `deps` or `implicit_deps` name it) get the focused border colour, and every other node gets `Modifier::DIM` on all its cells. No dependency line is drawn. *(Spec §16.3.)*
21. **Filters.** The text filter (`/`) works in the run view against each node's content text, keeping ancestors of a match and every descendant of a matching node, as the project tree does. `f` cycles `RunFilter::{All, Running, Blocked, Runtime(Claude), Runtime(Codex)}`, labelled `all`, `running`, `blocked`, `claude`, `codex`. `Running` keeps tasks in `preparing`, `working`, `proof`, `check`, `review` or `merge_queue` and live rounds and scouts; `Blocked` keeps `blocked` tasks; `Runtime(r)` keeps tasks whose route runtime is `r`, rounds and scouts whose route runtime is `r`. A kept node keeps its ancestors; a task kept by its own match keeps all its rounds, while a task kept only as an ancestor shows only the rounds that matched; the root is always kept. Opening or leaving the run view resets both filters. *(Spec §16.3 "Filters".)*

### Navigation

22. **Opening.** In the project overview, `l` or Enter on a `Run` node, or a double click on it, opens the run view on that run with the root selected and the pan at the origin. In the sidebar tree (`C-b t`), Enter or a click on a `Run` row turns the overview on and opens the run view the same way. In the project tree, `Space` on a `Run` row does nothing (it is a leaf). *(Spec §16.1.)*
23. **Leaving.** In the run view, `Esc`, or `h` with the root selected, closes the run view and returns to the project overview with the `Run` node selected. `Esc` in the project overview leaves tree mode as today. If the snapshot stops naming the run, or names it in a terminal state, the run view closes the same way and toasts `run <id> is gone` or `run <id> is <state text>`. *(Spec §16.1.)*
24. **Enter in the run view.** On the root: focus the orchestrator's window and leave tree mode, as Enter on a window does today; with no orchestrator window, toast `run <id> has no orchestrator window; Enter on an agent opens its conversation`. On an agent round, a scout or a planner: open its window's conversation (decision 25). On a task: open the conversation of its current round — the live one with the latest start, else the latest round whose window is listed; none → toast `<task> has no agent yet`. On a sub-agent: open its owning window's conversation (M6.5's descent goes further from there). *(Spec §16.4 "Navigation".)*
25. **Opening a conversation never focuses a window.** `App::open_conversation(window_id)` does what M6.5's `C-b m` does for the focused window — `keymap.set_conversation_mode(true)` and `conversation.open(window_id)` — for any listed window, leaving `focused`, `overview` and `run_view` as they were, so closing the conversation (M6.5's `Esc`/`q`) lands back in the run view on the same node. A window id that is not in `App.windows` toasts `window #<id> is not listed yet` for a live round and `<label> has finished and its window is gone` for an ended one. *(Spec §4.2, §16.4.)*
26. **Headless windows are never offered input.** Every activation path — Enter, a sidebar click, a canvas double click — on a `Window` node whose `kind` is `Headless` opens its conversation instead of focusing it. M8a decision 49's placeholder pane (reached by `C-b j`/`k`/`<n>` on a plain-listed headless window) stays as it is, and so do M8a.17's no-`Subscribe` and no-`Input` rules; this milestone adds no path that could send `ClientMsg::Input` for a headless window. *(User decision: only the orchestrator is interactive; spec §4.2.)*
27. **Conversation mode wins over tree mode.** In `Keymap::handle`, conversation mode is checked before tree mode, so an M6.5 conversation opened over the run view receives its own keys. If M6.5 merged with the opposite order, this milestone swaps the two checks.

### The inspector

28. **`RUN_INSPECTOR_HEIGHT` = 12** (a border, the title, nine rows, a border) and `MIN_INTERIOR_FOR_RUN_PANEL = RUN_INSPECTOR_HEIGHT + 6` = 18. While the run view is open, `overview::areas` gives the panel 12 rows when the interior has 18, else milestone 4.7's 8 rows when it has 14, else the single line. The height depends on the view, not on the node, so the canvas never jumps as the selection moves; a sub-agent in the run view gets the tall panel too. The project overview is unchanged (8), including for a `Run` node, which there shows its first five fields. *(Spec §16.4 "Height".)*
29. **Run inspections are laid out one field per row**, the label padded to `RUN_LABEL_WIDTH` = 10 columns, the value truncated with `…` to the rest; fields past the last row are dropped from the end; the one wrapping field (a scout's `question`) wraps under itself, indented 10, onto as many rows as it needs while leaving one row for each field after it. The title carries a right-aligned muted text; when the name, two spaces and the right text do not fit, the right text is dropped and the name truncated. Project, window and sub-agent inspections keep milestone 4.7's column packing unchanged. This follows the spec's mockups, which are single-column with a ten-column label, rather than its sentence "labelled fields packed in columns" (decision 35). *(Spec §16.4 mockups.)*
30. **Progress lines.** A bar of `█` then `░`, `filled = (width × done + total / 2) / total`: 18 wide for the run, 10 for a planner and for a task's budget. The run's and a planner's counts: `{merged}/{total} merged`, then each non-zero category in the order `working` (preparing, working), `checking` (proof, check), `review`, `merging` (merge_queue), `blocked`, `waiting` (pending, queued), joined with ` · `; cancelled tasks are left out of `total` and appended as `· {n} cancelled`. A task's budget bar uses the larger of its tool-call and minute fractions of `spent_session` against `budget`. *(Spec §16.4 mockups; the mockups' own bars do not match their counts, decision 35.)*
31. **Numbers.** Durations: `< 60` → `{s}s`, `< 3600` → `{m}m`, else `{h}h{mm}m` (`1h12m`, `2h05m`). Tokens: `< 1000` → `{n}`, `< 1 000 000` → `{n/1000}k`, else `{n/1 000 000}.{(n mod 1 000 000)/100 000}M`. Token totals are `TokenUsage::billable()`; the cache share is `cache_read × 100 / (input + cache_read + cache_write)`, rounded down, omitted when the denominator is 0. Commit ids show 7 characters.

### The plan gate

32. **The gate is the run view of a run in `awaiting_approval`.** There, and only there, `a` asks `Approve run <id>? <n> tasks start.` (`1 task starts.` for one) and on `y` sends `RunRequest::Approve`; `x` asks `Reject run <id>? Its branches and worktrees are removed; salvage refs are kept.` and on `y` sends `RunRequest::Reject`; `d` on a task asks `Remove <task> from run <id>'s plan?` and on `y` sends `RunRequest::Edit` with `PlanEdit::CancelTask`; `e` on a task opens the task edit form. Elsewhere those four keys toast `the plan gate is closed: run <id> is <state text>`; `e` or `d` on a non-task node toasts `select a task to edit or remove`. Adding a task is not offered. *(Spec §12.3; conversation-view decision 13.)*
33. **The edit form** edits route (runtime, model, strength, effort), size (`S` or `M`), test mode and its reason, and brief. It sends one `PlanEdit::AmendTask` carrying only what changed; nothing changed closes it with the toast `nothing changed`. Changing the runtime clears the model field. A mode other than `tdd` needs a non-blank reason (`a reason is required when test mode is check or none`), mirroring M8a decision 10; every other rule is the engine's, whose `Refused` message is shown inline in the form. The brief is one line; a newline in it is shown as `↵` and restored on submit; `Ctrl-J` inserts one. *(Spec §12.3 "may edit route, brief, size, test mode, and remove tasks".)*
34. **Replies.** `RunReply::Done { message, .. }` toasts `message` (and closes a submitting edit form when `request` is `run edit`); `Refused { request, message }` fills a submitting edit form's error when `request` is `run edit`, and toasts `message` otherwise; `Started`, `ConfirmNeeded` and `ToolResult` are ignored.

### Where the spec's text and its mockups disagree

35. **Resolved here, so nobody has to ask.** Each is also a finding for the spec's author.
    - §16.4 says the inspector "keeps milestone 4.7's layout rules (labelled fields packed in columns)", but every §16.4 mockup is one field per row with a ten-column label. The mockups win (decision 29).
    - The §16.4 progress bars do not match their own counts: the run's shows 11 of 18 cells for 5/9 (exactly 10), the planner's 6 of 10 for 1/3 (3.3). Decision 30's formula wins, so the exact tests show 10 and 3 cells.
    - The §16.1 diagram draws the root at the top of the canvas and `t6` (wave 0) after planner `A`; milestone 4.6's layout centres a parent on its children, and §16.2 orders by wave. The layout and decision 13 win; the diagram is illustrative.
    - The §16.1 diagram labels a scout `scout S1` and a planner `planner A dmn`, while the §16.3 table says a scout shows its question and a planner its area. The table wins for scouts; a planner shows its epic and title (its area is a glob list that does not fit a box) and the inspector shows the area.
    - §16.2's `AgentRound { task, role, session, round }` cannot tell two runs' `t1` apart; decision 15 adds `run`.
    - §16.3's glyph list has `✗ rejected or failed` for tasks, but no M8a task state is "failed" (a failed gate sends the task back to `working` or to `blocked`). `✗` is used for rounds and gate marks; a task that failed is `⊘` when blocked.
    - §16.4's worker mockup shows `editing crates/daemon/src/status.rs`, a tool's target, which no snapshot or window field carries; `doing` shows `last tool: <name>` (Risks 6).
    - §16.4's worker mockup counts `3 commits` per round; nothing in M8a or M8b counts commits per session, and counting them would put git calls behind every snapshot. `activity` leaves them out.
    - §16.4's "derives nothing that the snapshot does not carry" and §16.5's "elapsed times tick on the client's clock" need data M8a's snapshot lacks (the daemon's clock, approval time, edit log, rate-limit start, worker bounces, briefs for the edit form); M8c.1 adds exactly those.
    - §16.4's navigation assumes every run has an orchestrator window; until M9, and on the fast path, none has. Decision 24 says what Enter does then.
    - §16.4's task mockup writes `red a1b2c3` (6 characters); the project writes commit ids with 7 (`halted_reason`, `run status`), and so does this view.

## Interfaces

### `proto` (M8c.1; `crates/proto/src/run_info.rs`)

Added to M8a's types, every field `#[serde(default)]`:

```rust
pub struct RunsSnapshot { /* M8a: revision, runs */ pub now: u64 }
pub struct RunInfo {
    /* M8a fields */
    pub approved_hhmm: Option<String>,        // "11:02", when approved (by the user or --yes)
    pub plan_edits: Vec<String>,              // "<hh:mm> <description>", newest first, at most 10
    pub plan_edits_since_approval: u32,
    // placeholders for later milestones ("Consumes from later milestones"):
    pub scouts: Vec<ScoutInfo>,
    pub planners: Vec<PlannerInfo>,
    pub usage: Option<RunUsage>,
    pub estimate_left_secs: Option<u64>,
    pub bound_ratio_permille: Option<u32>,
}
pub struct TaskInfo { /* M8a fields */ pub brief: String, pub acceptance: Vec<String>,
                      pub diff: Option<DiffStats> }                                     // M8b's name and type
pub struct AgentRoundInfo { /* M8a fields */ pub rate_limited_since: Option<u64>, pub sent_back_at: Vec<u64> }
pub struct DiffStats { pub files: u32, pub hunks: u32, pub added: u32, pub removed: u32 } // Copy, Eq, Default (M8b decision 32's type)
pub struct RunUsage { pub total: TokenUsage, pub by_role: std::collections::BTreeMap<String, TokenUsage>,
                      pub decider_calls: u32, pub decider_fallbacks: u32 }                  // M8b decision 29's type
// M8b's scout snapshot types (M8b Interfaces `crates/proto/src/scout.rs`), exactly:
#[serde(rename_all = "snake_case")] pub enum ScoutKind { Onboarding, Area }                     // Copy, Eq
#[serde(rename_all = "snake_case")] pub enum ScoutState { Starting, Working, Reported, Failed }  // Copy, Eq
pub struct ScoutInfo { pub id: String, pub kind: ScoutKind, pub question: String, pub state: ScoutState,
                       pub failure: Option<String>, pub window_id: Option<u32>, pub route: Route,
                       pub started_at: u64, pub ended_at: Option<u64>, pub tool_calls: u32,
                       pub report_bytes: Option<u32>, pub files: Vec<String>, pub usage: TokenUsage }
#[serde(rename_all = "snake_case")] pub enum PlannerState { #[default] Planning, Finished, Failed } // Copy, Eq, Default
pub struct PlannerInfo { pub epic: String, pub title: String, pub area: Vec<String>, pub route: Route,
                         pub window_id: Option<u32>, pub state: PlannerState, pub started_at: u64,
                         pub ended_at: Option<u64>, pub edits_accepted: u32, pub edits_rejected: u32,
                         pub last_rejection: Option<String>, pub replans: Vec<String> }
```

`lib.rs` re-exports the new types and bumps `PROTO_VERSION` (header).

### `daemon` (M8c.1)

```rust
// run/model.rs — each #[serde(default)]
pub struct Run { /* M8a */ pub approved_at: Option<u64>, pub plan_edits: Vec<TaskEvent>, pub plan_edits_since_approval: u32 }
pub struct AgentRound { /* M8a */ pub rate_limited_since: Option<u64>, pub sent_back_at: Vec<u64> }
// run/edits.rs (pure)
pub fn describe(edits: &[PlanEdit]) -> String;
```

- `approved_at = Some(now)` where the engine moves a run out of `awaiting_approval` on `Approve`, and at `Start` with `yes`.
- On an accepted `Edit` batch: push `TaskEvent { at: now, text: describe(&edits) }` onto `plan_edits` (keep the last 50), and add 1 to `plan_edits_since_approval` when `approved_at` is set.
- `describe` joins, with `, `: `add <id>`, `split <id>`, `cancel <id>`, `amend <id>`, `dep <id> on <dep>`, `answer <id>`, `pause`, `resume`, `finish`.
- `rate_limited_since = Some(now)` when a round's `rate_limited_until` goes from `None` to `Some`, and `None` whenever `rate_limited_until` is cleared.
- Rung 1 (M8a decision 38) pushes `now` onto the live worker round's `sent_back_at` when it queues the failure text for the same session.
- `run/snapshot.rs` fills `now`, `approved_hhmm` and each `plan_edits` stamp with the same `<hh:mm>` helper it uses for `TaskInfo.history`, `plan_edits` newest first and at most 10, and copies `brief`, `acceptance` (from the task's `spec`), `rate_limited_since` and `sent_back_at`.

### `crates/tui/src/tree.rs` and `tree/`

```rust
pub enum NodeKey {
    Project(PathBuf), Window(u32), Subagent { window_id: u32, id: String },
    Run(String),
    Planner { run: String, epic: String },
    Scout { run: String, id: String },
    Task { run: String, id: String },
    AgentRound { run: String, task: String, role: proto::AgentRole, session: u32, round: u32 },
}
pub enum RowKind<'a> {
    Project { .. }, Window { .. }, Subagent { .. },            // unchanged
    Run { run: &'a RunInfo, orchestrator: Option<&'a WindowInfo>, position: Option<usize> },
    Planner { run: &'a RunInfo, planner: &'a PlannerInfo },
    Scout { run: &'a RunInfo, scout: &'a ScoutInfo, window: Option<&'a WindowInfo> },
    Task { run: &'a RunInfo, task: &'a TaskInfo },
    AgentRound { run: &'a RunInfo, task: &'a TaskInfo, round: DisplayRound<'a> },
}
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayRound<'a> {
    pub info: &'a AgentRoundInfo, pub number: u32, pub started_at: u64, pub ended_at: Option<u64>,
    pub last: bool,                       // the session's last display round (decision 14)
    pub window: Option<&'a WindowInfo>,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RunFilter { #[default] All, Running, Blocked, Runtime(proto::Runtime) }

pub fn build(windows: &[WindowInfo], state: &TreeState) -> Vec<Row<'_>>;          // = build_with_runs(windows, &[], state)
pub fn build_with_runs<'a>(windows: &'a [WindowInfo], runs: &'a [RunInfo], state: &TreeState) -> Vec<Row<'a>>;
pub fn run_rows<'a>(run: &'a RunInfo, windows: &'a [WindowInfo], state: &TreeState, filter: RunFilter) -> Vec<Row<'a>>;
pub fn shown_runs(runs: &[RunInfo]) -> impl Iterator<Item = &RunInfo>;              // decision 6
pub fn display_rounds<'a>(task: &'a TaskInfo, windows: &'a [WindowInfo]) -> Vec<DisplayRound<'a>>; // decision 14, start order
pub fn round_label(role: AgentRole, session: u32, number: u32) -> String;        // "worker #1", "worker #1 r2", "review #2"
impl TreeState { pub fn prune_runs(&mut self, runs: &[RunInfo]); }  // keeps run-view keys of shown runs only
```

`TreeState::toggle` returns `true` for every new key; `TreeState::prune` keeps every new key (run keys are `prune_runs`'s). `agent_order` yields a `Run` row's orchestrator id when it has one (decision 10). New files: `tree/runs.rs` (grouping, `shown_runs`, the run status of decision 9, hidden windows), `tree/run_rows.rs` (`run_rows`, ordering, `display_rounds`, filters).

### Node content (`crates/tui/src/graph/run_text.rs`, called from `graph::content_text`)

| Row | Text | Example |
|---|---|---|
| `Run`, orchestrator window listed | `orchestrator  {merged}/{total}` | `orchestrator  5/9` |
| `Run`, none | `run {last 4 chars of run_id}  {merged}/{total}` | `run 3f9a  0/2` |
| `Scout` | `{question}` | `where are hooks parsed?` |
| `Planner` | `planner {epic} {title}  {merged}/{total}` (`planner {epic}  …` when `title` is empty) | `planner A daemon  1/3` |
| `Task` | decision 18 | `t2 status S  ⇠t0`, `t0 proto M ◆`, `t7 map Gemini … M  ⇠t0t6` |
| `AgentRound` | `{round_label} {runtime}` | `worker #1 claude`, `worker #1 r2 codex`, `review #1 codex` |

`total` excludes cancelled tasks; a planner's tasks are those whose `epic` equals its `epic`.

### Glyphs (`crates/tui/src/theme.rs`)

```rust
pub fn task_glyph(state: TaskState, gate_open: bool, animating: bool, spinner_frame: usize) -> &'static str;
pub fn task_color(state: TaskState) -> Color;
pub fn run_color(state: RunState) -> Color;
pub const RUN_GLYPH: &str = "◉";
```

| Task state | Glyph | Colour |
|---|---|---|
| any, while the run is `awaiting_approval` (`gate_open`) | `○` | `status_color(Idle)` |
| `pending` | `◌` | `status_color(Starting)` |
| `queued` | `▫` | `status_color(Idle)` |
| `preparing` | `●` | `status_color(Working)` |
| `working` | spinner when `animating`, else `●` | `status_color(Working)` |
| `proof`, `check` | `◇` | `status_color(Working)` |
| `review` | `◐` | `status_color(Working)` |
| `merge_queue` | `▸` | `status_color(Working)` |
| `merged` | `✓` | `status_color(Done)` |
| `blocked` | `⊘` | `status_color(Attention)` |
| `cancelled` | `–` | `DIM` |

`run_color`: `awaiting_approval`, `paused`, `halted` → `status_color(Attention)`; `running` → `status_color(Working)`; `complete`, `accepted` → `status_color(Done)`; `discarded`, `failed` → `DIM`. Scouts (M8b's `ScoutState`): `starting`, `working` → round rules, `reported` → `✓`, `failed` → `✗` (`Color::Red`). Planners: `planning` → round rules, `finished` → `✓`, `failed` → `✗`.

### Overview, inspector, panel

```rust
// crates/tui/src/inspector.rs
pub const RUN_INSPECTOR_HEIGHT: u16 = 12;
pub const MIN_INTERIOR_FOR_RUN_PANEL: u16 = RUN_INSPECTOR_HEIGHT + 6;
pub const RUN_LABEL_WIDTH: usize = 10;
pub const RUN_PROGRESS_WIDTH: usize = 18;
pub const PROGRESS_WIDTH: usize = 10;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FieldLayout { #[default] Columns, Rows }
pub struct Inspection { pub glyph: Span<'static>, pub name: String, pub right: Option<String>,
                        pub fields: Vec<Field>, pub layout: FieldLayout }
// crates/tui/src/inspector/run.rs and inspector/run_task.rs (pure)
pub(crate) fn run_inspection(run: &RunInfo, orchestrator: Option<&WindowInfo>, app: &App) -> Inspection;
pub(crate) fn planner_inspection(run: &RunInfo, planner: &PlannerInfo, app: &App) -> Inspection;
pub(crate) fn scout_inspection(run: &RunInfo, scout: &ScoutInfo, window: Option<&WindowInfo>, app: &App) -> Inspection;
pub(crate) fn task_inspection(run: &RunInfo, task: &TaskInfo, app: &App) -> Inspection;
pub(crate) fn round_inspection(run: &RunInfo, task: &TaskInfo, round: &DisplayRound<'_>, app: &App) -> Inspection;
pub fn format_duration(secs: u64) -> String;   // decision 31
pub fn format_tokens(n: u64) -> String;        // decision 31
pub fn progress_bar(done: u64, total: u64, width: usize) -> String;   // decision 30

// crates/tui/src/ui/overview.rs
pub fn areas(main: Rect, inspector_visible: bool, run_view: bool) -> (Rect, Rect);  // decision 28
```

Every existing `Inspection { .. }` construction gains `right: None, layout: FieldLayout::Columns`. `inspector/panel.rs` dispatches on `layout`; the `Rows` renderer lives in `inspector/panel/rows.rs`.

### `App` (`crates/tui/src/app/runs.rs`, new)

`App` gains four fields, declared in `app/mod.rs`: `pub runs: proto::RunsSnapshot` (default: revision 0, no runs), `pub run_view: Option<RunView>`, `runs_received_at: Instant`, `run_subscribed: bool`.

```rust
pub struct RunView { pub run_id: String, pub filter: RunFilter }
impl App {
    pub fn run_subscription(&mut self) -> Effect;                 // Send(Run(Subscribe)); sets run_subscribed
    pub(crate) fn on_run_reply(&mut self, reply: RunReply) -> Vec<Effect>;
    pub fn run_age(&self, unix_secs: u64) -> u64;                 // decision 2
    pub fn nav_rows(&self) -> Vec<tree::Row<'_>>;                 // decision 11
    pub(crate) fn open_run_view(&mut self, run_id: String);
    pub(crate) fn close_run_view(&mut self);
    pub(crate) fn open_conversation(&mut self, window_id: u32) -> Vec<Effect>;   // decision 25
    pub(crate) fn on_run_view_key(&mut self, key: KeyEvent) -> Option<Vec<Effect>>; // a x e d f h; None = not handled
    pub(crate) fn on_edit_task_key(&mut self, key: KeyEvent) -> Vec<Effect>;
}
pub enum PendingAction { /* existing */ ApproveRun(String), RejectRun(String), RemoveTask { run_id: String, task_id: String } }
pub enum Modal { /* existing */ EditTask(crate::run_edit::TaskEditForm) }
```

### The task edit form (`crates/tui/src/run_edit.rs`, pure; `crates/tui/src/ui/run_edit.rs`, rendering)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField { Runtime, Model, Strength, Effort, Size, TestMode, Reason, Brief }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEditForm {
    pub run_id: String, pub task_id: String,
    pub runtime: Runtime, pub model: TextInput, pub strength: Option<Strength>, pub effort: Effort,
    pub size: Size, pub test_mode: TestMode, pub reason: TextInput, pub brief: TextInput,
    pub focus: EditField, pub error: Option<String>, pub submitting: bool,
    original: TaskInfoValues,               // what the task had when the form opened
}
pub enum EditOutcome { Stay, Cancel, Submit(Vec<PlanEdit>), Unchanged }
impl TaskEditForm {
    pub fn new(run_id: &str, task: &TaskInfo) -> Self;
    pub fn visible_fields(&self) -> Vec<EditField>;          // Reason only when test_mode != Tdd
    pub fn on_key(&mut self, key: KeyEvent) -> EditOutcome;
    pub fn on_paste(&mut self, text: &str);
    pub fn edits(&self) -> Result<Vec<PlanEdit>, (EditField, String)>;
}
```

Keys: `Tab`/`Down` next field, `BackTab`/`Up` previous; on `Runtime`, `Strength`, `Effort`, `Size`, `TestMode`: `Left`/`Right`/`Space` cycle (`claude ↔ codex`; `policy → fast → standard → frontier`; `low → medium → high`; `S ↔ M`; `tdd → check → none`); on text fields, `apply_text_key`; `Ctrl-J` in `Brief` inserts `↵`; `Enter` submits; `Esc` or `Ctrl-C` cancels. The box is 72 columns wide, titled ` edit <task> `, one row per visible field (`LABEL_WIDTH` 11, `MARKER_WIDTH` 2, `› ` on the focused row), choice values written `‹ value ›`, `Strength` `None` written `policy`, then a blank row, the error row when there is one, and the hint row `⏎ save  tab next  ←/→ change  esc cancel`.

### Status bar and overview title

- Run view, `TreeInput::Navigate`, gate open: `a approve  x reject  e edit  d remove  ⏎ open  f filter: <label>  esc back`.
- Run view, `Navigate`, otherwise: `j/k move  h/l tier  ⏎ open  space fold  f filter: <label>  / find  esc back`.
- The project overview's hints are unchanged.
- The overview block's title is ` run <run_id> ` in the run view, ` tree overview ` otherwise.

### Inspector contents, exact

Values below are the "Gemini fixture" (task M8c.7): run `r1`, goal `Add Gemini runtime`, `created_at = now − 4320`; tasks `t0 t1 t4 t6 t8` merged, `t3 t7` working, `t2` in review, `t5` blocked (`question`, `Gemini has no subagent-stop event`); `critical_path = [t0, t6, t2, t3]`; writers `3/3`, readers `1/3`; a live Codex round rate-limited since `now − 240`; token usage `input 200 000, cache_read 710 000, cache_write 90 000, output 1 510 000`; `spent_total.tool_calls` summing to 612; `estimate_left_secs = 2400`; `bound_ratio_permille = 1200`; `approved_hhmm = "11:02"`, `plan_edits = ["11:40 split t2", "11:20 amend t4"]`, `plan_edits_since_approval = 2`.

**Run (orchestrator).** Glyph `◉`; name `{run_id}  {goal}`; right `{state text} · {format_duration(run_age(created_at))}`, state text `awaiting approval`, `running`, `paused (from <paused_from>)`, `halted`, `complete`. Fields, in order, each omitted when its condition says so:

| Label | Value |
|---|---|
| `progress` | decision 30 at 18; `no tasks yet` when total is 0 |
| `path` | `critical path {ids joined " → "} · {n} tasks left` (`1 task left`) + ` · {p/1000}.{p%1000/100}× the bound` when `bound_ratio_permille` is `Some(p)`; omitted when `critical_path` is empty |
| `agents` | `workers {writers_busy}/{max_writers} · readers {readers_busy}/{max_readers}`, then for each runtime used by a task route or review route, Claude first: ` · {runtime} ok` or ` · {runtime} rate-limited {format_duration(age of the earliest rate_limited_since among its live rate-limited rounds)}` (`rate-limited` alone when no `since`) |
| `spend` | `tokens {format_tokens(t)}`, `t` being `usage.total.billable()` when `usage` is set (every role, the orchestrator included) and the sum of every round's `usage.billable()` otherwise, the cache share over the same usage, + ` (cache {p}%)` + ` · tool calls {Σ spent_total.tool_calls}` + ` · est. left ~{format_duration(e)}` when `estimate_left_secs` is `Some(e)` |
| `gate` | gate open: `awaiting approval · a approve · x reject · e edit · d remove`; approved (`approved_by` set): `plan approved` (`plan approved by --yes` when `approved_by` is `--yes`) + ` {approved_hhmm}` when known + ` · {n} plan edits since` (`1 plan edit since`) when `n > 0` + ` · last: {plan_edits[0] without its stamp}` when there is one; otherwise `plan not approved` |
| `attention` | `halted: {halted_reason}` when halted; else the first blocked task in plan order as `{id} blocked: {reason} — "{text}"` (reasons `mis-sized`, `human`, `conflict`, `dependency cancelled`, `question`, `environment`); else `attention[0]`; then ` · +{n} more` for the other blocked tasks and attention lines; omitted when there is nothing |

Rendered at 86 × 12 (`M8c.8`, `run_panel_matches_the_mockup`):

```
╭────────────────────────────────────────────────────────────────────────────────────╮
│ ◉ r1  Add Gemini runtime                                           running · 1h12m │
│ progress  ██████████░░░░░░░░  5/9 merged · 2 working · 1 review · 1 blocked        │
│ path      critical path t0 → t6 → t2 → t3 · 2 tasks left · 1.2× the bound          │
│ agents    workers 3/3 · readers 1/3 · claude ok · codex rate-limited 4m            │
│ spend     tokens 1.8M (cache 71%) · tool calls 612 · est. left ~40m                │
│ gate      plan approved 11:02 · 2 plan edits since · last: split t2                │
│ attention t5 blocked: question — "Gemini has no subagent-stop event"               │
│                                                                                    │
│                                                                                    │
│                                                                                    │
╰────────────────────────────────────────────────────────────────────────────────────╯
```

**Sub-planner.** Glyph per its state; name `planner {epic}  {title}`; right `planning · {age}` while planning, `finished · planned {n} tasks in {format_duration(ended − started)}` (`1 task`) when finished, `failed · {duration}` when failed. Fields: `progress` (decision 30 at 10, over its tasks); `area` (`area` joined ` · `); `edits` (`{a} accepted · {r} rejected` + ` ({last_rejection})` when set + ` · re-planned once ({replans joined ", "})` / `twice (…)` / `{n} times (…)` when `replans` is not empty). Rendered at 86 × 12 (`planner_panel_matches_the_mockup`), planner `A`/`daemon`, area `crates/daemon/**`, `crates/cli/src/hook.rs`, finished after 120 s, its three tasks merged, working and pending, edits `3`/`1`, `owns outside area`, replans `["t2 split"]`:

```
╭────────────────────────────────────────────────────────────────────────────────────╮
│ ✓ planner A  daemon                               finished · planned 3 tasks in 2m │
│ progress  ███░░░░░░░  1/3 merged · 1 working · 1 waiting                           │
│ area      crates/daemon/** · crates/cli/src/hook.rs                                │
│ edits     3 accepted · 1 rejected (owns outside area) · re-planned once (t2 split) │
│                                                                                    │
│                                                                                    │
│                                                                                    │
│                                                                                    │
│                                                                                    │
│                                                                                    │
╰────────────────────────────────────────────────────────────────────────────────────╯
```

**Task.** Glyph per decision 19; name `{id}  {title}`; right `{size} · {test_mode} · {stage}`, stage `planned` (gate open), `waiting`, `queued`, `preparing`, `working`, `test proof`, `check`, `review round {reviews.len()}`, `merge queue`, `merged`, `blocked: {reason}`, `cancelled`. Fields:

| Label | Value |
|---|---|
| `stages` | `done {d} → proof {p} → check {c} → review {r} → merge {m}`. `d`: `●` while `working`, `✓` when `done_signal` is set, else `·`. `p`: `–` unless `tdd`; `●` in `proof`; else `last_proof` `ok` → `✓`, not ok → `✗`, none → `·`. `c`: `–` when the run is `unverified`; `●` in `check`; else `last_check` likewise. `r`: `–` when `review_route` is `None`; `●` in `review`; else the last review with a verdict: blocking → `✗`, otherwise `✓`; none → `·`. `m`: `✓` merged, `●` in `merge_queue`, else `·`. |
| `route` | `{runtime} · {strength} · {effort} effort` + `  →  reviewer {runtime} · {strength}` when `review_route` is set |
| `deps` | `waits on {id glyph …}` over `deps` then `implicit_deps` (deduplicated; glyph `✓` when merged, else the task glyph) + ` · unblocks {ids joined ", "}` + ` · on critical path`, each part present when non-empty; omitted when all three are |
| `budget` | `{bar at 10} {tool_calls}/{budget.tool_calls} tool calls · {secs/60}/{budget.minutes} min · {format_tokens(tokens)} tokens`, from `spent_session`, `/{format_tokens(budget.tokens)}` after the tokens when a token budget is set |
| `tries` | `review {b.review}/{max_bounces} bounces · check {b.check}/{max_bounces}` + ` · proof {n}/{max}` and ` · merge {n}/{max}` when non-zero + ` · stalls {n}` when non-zero + ` · escalation step {rung}` when `rung > 0` |
| `diff` | `{files} files · +{added} −{removed}` when `diff` is set, then ` · test `{test}` red {red7} {proof mark}` when `test` and `red` are set (the mark `✓`/`✗` from `last_proof`, omitted when none); omitted when both are absent |
| `review` | the last review with a verdict: `r{round} {✓ or ✗} {counts}: {file name}:{line} "{text}"` with counts the non-zero severities `{n} critical, {n} important, {n} minor` and the most severe finding in full (`{input}` in place of `file:line` when it has no file); `r{round} ✓ no findings` when it has none; omitted when no review has a verdict |
| `history` | `history` joined ` · ` (newest first, as M8a gives it); omitted when empty |

Rendered at 86 × 12 (`task_panel_matches_the_mockup`), task `t2` of the Gemini fixture — `map Gemini hook events to status`, `M`, `tdd`, `review`, deps `t0 t6` merged, dependents `t3 t7`, on the critical path, route Codex standard high, reviewer Claude frontier, budget 150/60, `spent_session` 104 tool calls, 2280 s, 410 000 tokens, bounces review 1, rung 1, `max_bounces` 2, `diff` 4 files +212 −31, test `status::gemini_stop_marks_idle`, red `a1b2c3d9…`, proof ok, round 1 `changes` with one critical finding `crates/daemon/src/status.rs:118 "SubagentStop not paired"` and two minor, round 2 without a verdict, history `12:31 review r1 changes`, `12:20 check passed`, `12:02 started`:

```
╭────────────────────────────────────────────────────────────────────────────────────╮
│ ◐ t2  map Gemini hook events to status                    M · tdd · review round 2 │
│ stages    done ✓ → proof ✓ → check ✓ → review ● → merge ·                          │
│ route     codex · standard · high effort  →  reviewer claude · frontier            │
│ deps      waits on t0 ✓ t6 ✓ · unblocks t3, t7 · on critical path                  │
│ budget    ███████░░░ 104/150 tool calls · 38/60 min · 410k tokens                  │
│ tries     review 1/2 bounces · check 0/2 · escalation step 1                       │
│ diff      4 files · +212 −31 · test `status::gemini_stop_marks_idle` red a1b2c3d ✓ │
│ review    r1 ✗ 1 critical, 2 minor: status.rs:118 "SubagentStop not paired"        │
│ history   12:31 review r1 changes · 12:20 check passed · 12:02 started             │
│                                                                                    │
╰────────────────────────────────────────────────────────────────────────────────────╯
```

At 60 × 12 (`task_panel_drops_the_right_text_and_elides_values`):

```
╭──────────────────────────────────────────────────────────╮
│ ◐ t2  map Gemini hook events to status                   │
│ stages    done ✓ → proof ✓ → check ✓ → review ● → merge… │
│ route     codex · standard · high effort  →  reviewer c… │
│ deps      waits on t0 ✓ t6 ✓ · unblocks t3, t7 · on cri… │
│ budget    ███████░░░ 104/150 tool calls · 38/60 min · 4… │
│ tries     review 1/2 bounces · check 0/2 · escalation s… │
│ diff      4 files · +212 −31 · test `status::gemini_sto… │
│ review    r1 ✗ 1 critical, 2 minor: status.rs:118 "Suba… │
│ history   12:31 review r1 changes · 12:20 check passed … │
│                                                          │
╰──────────────────────────────────────────────────────────╯
```

**Agent round.** Glyph per decision 19; name `{round_label}  {runtime} · {strength} · {effort}`; right `{status} · {duration} · {task id}` where status is `rate-limited` when rate-limited, else the window's `Status::label()` while the round is live (`starting` when the window is not listed), `finished` once ended, and duration is the age since the display round's start while live, `ended − start` after. Worker fields: `doing` (live: `rate-limited · waiting out the runtime's retry` when rate-limited, `last tool: {tool}` when the window has one, `thinking` when a turn is open, `waiting for its next turn` otherwise, then ` · {n} sub-agents open` when `open_subagents > 0`; ended: `finished`); `activity` (on the session's last display round only: `turns {turns} · tool calls {tool_calls}` + ` · tokens {format_tokens(billable)}` + ` · {denials} denied` when non-zero); `fixing` (display rounds after the first of a session, or any round of session > 1: the latest gate failure at or before the round's start — the most severe critical or important finding of a blocking review, timed by its reviewer round's `ended_at`, as `{file name}:{line} {severity} — {text}`; a failed `last_check` as `check failed: {last line of summary}`; a failed `last_proof` as `test proof failed`; omitted when none is found); `session`. Reviewer fields: `judging` (the latest worker display round started before this round: `{label} · {runtime} · {strength}`); `strength` (`{own} vs author {author's}`); `verdict` (`reviewing`, `approve`, `changes (blocking)`, `changes (minor only, counts as approval)`, or `none — the round ended without one`); `findings` (`{counts} · {most severe as in fixing}`, `none` when empty); `session`. `session` is `no window yet`; `#{id} · window closed` when the id is not listed; else `#{id} · {headless or terminal} · worktree {task.branch} · Enter: conversation` for a worker, `#{id} · {headless or terminal} · read-only review worktree · Enter: conversation` for a reviewer.

Rendered at 86 × 12 (`worker_round_panel_matches_the_mockup`), `worker #1 r2` of task `t2` (session 1 started `now − 1560`, sent back at `now − 360`, window 7 `Headless`, `Working`, tool `apply_patch`, 14 turns, 41 tool calls, billable usage 180 000), spinner frame 0:

```
╭────────────────────────────────────────────────────────────────────────────────────╮
│ ⠋ worker #1 r2  codex · standard · high                          working · 6m · t2 │
│ doing     last tool: apply_patch                                                   │
│ activity  turns 14 · tool calls 41 · tokens 180k                                   │
│ fixing    status.rs:118 critical — SubagentStop not paired                         │
│ session   #7 · headless · worktree anthrex/r1/t2 · Enter: conversation             │
│                                                                                    │
│                                                                                    │
│                                                                                    │
│                                                                                    │
│                                                                                    │
╰────────────────────────────────────────────────────────────────────────────────────╯
```

And `review #1` of the same task (Claude `claude-opus-5` frontier high, 540 s, blocking, window 9 retired) (`reviewer_round_panel_matches`):

```
╭────────────────────────────────────────────────────────────────────────────────────╮
│ ✗ review #1  claude · frontier · high                           finished · 9m · t2 │
│ judging   worker #1 · codex · standard                                             │
│ strength  frontier vs author standard                                              │
│ verdict   changes (blocking)                                                       │
│ findings  1 critical, 2 minor · status.rs:118 critical — SubagentStop not paired   │
│ session   #9 · window closed                                                       │
│                                                                                    │
│                                                                                    │
│                                                                                    │
│                                                                                    │
╰────────────────────────────────────────────────────────────────────────────────────╯
```

**Scout.** Glyph per its state; name `scout {id}`; right `{starting|working|reported|failed} · {duration}`. Fields: `question` (the wrapping field), `state` (`starting`, `working`, `reported`, `failed: {failure}`, with ` · {tool}` while its window runs one), `took` (`format_duration`), `report` (`{report_bytes} bytes`, or `not yet`), `files` (joined `, `, omitted when empty), `session` (as a worker's, with `worktree` replaced by `main checkout, read-only`). Rendered at 60 × 12 (`scout_panel_wraps_the_question`), scout `S1`, reported after 180 s, report 1840 bytes, window 4 retired:

```
╭──────────────────────────────────────────────────────────╮
│ ✓ scout S1                                 reported · 3m │
│ question  where are Claude hook events parsed, and which │
│           of them fire in -p mode?                       │
│ state     reported                                       │
│ took      3m                                             │
│ report    1840 bytes                                     │
│ files     crates/daemon/src/hooks.rs, crates/daemon/src… │
│ session   #4 · window closed                             │
│                                                          │
│                                                          │
╰──────────────────────────────────────────────────────────╯
```

**Sub-agent.** Milestone 4.7's projection, unchanged, `layout: Columns`.

**The single line** (the inspector off, or a short terminal): `{glyph} {name}` (the name bold) then `  {right}` muted, for every run kind. The task above: `◐ t2  map Gemini hook events to status  M · tdd · review round 2`.

### Canvases, exact

The **gate fixture** (M8c.5): project `/r/demo` with one plain PTY shell window `1` `shell`, `Idle`, and run `add-reset-3f9a` in `awaiting_approval` with tasks `t1`, `t2` and no windows. The project overview at 34 × 7 (`project_overview_draws_the_run_node`):

```
               ╭─────────────────╮
             ┌─┤ ◉ run 3f9a  0/2 │
╭──────────╮ │ ╰─────────────────╯
│ ◆ demo   ├─┤                    
╰──────────╯ │ ╭─────────────────╮
             └─┤ ○ 1 shell       │
               ╰─────────────────╯
```

The **three-task fixture** (M8c.5): run `add-reset-3f9a`, `running`, orchestrator window `3` listed (`Pty`, `run == RunRef { role: Orchestrator, task_id: None, session: 1, .. }`); `t0 proto` M hub `merged`, wave 0, a finished Claude worker round (window 4, retired) and a finished Codex reviewer round 1 that approved (window 5, retired); `t1 spawn` M, deps `[t0]`, `working`, wave 1, on the critical path, a live Claude worker round on window 6 (`Headless`, `Idle`); `t2 status` S, deps `[t0]`, `queued`, wave 1. The run view at 73 × 15 (`run_view_draws_tiers_rounds_and_glyphs`):

```
                                                   ╭────────────────────╮
                                                 ┌─┤ ✓ worker #1 claude │
                          ╭────────────────────╮ │ ╰────────────────────╯
                        ┌─┤ ✓ t0 proto M ◆     ├─┤                       
                        │ ╰────────────────────╯ │ ╭────────────────────╮
                        │                        └─┤ ✓ review #1 codex  │
                        │                          ╰────────────────────╯
╭─────────────────────╮ │                                                
│ ◉ orchestrator  1/3 ├─┤ ╭────────────────────╮   ╭────────────────────╮
╰─────────────────────╯ ├─┤ ● t1 spawn M  ⇠t0  ├───┤ ● worker #1 claude │
                        │ ╰────────────────────╯   ╰────────────────────╯
                        │                                                
                        │ ╭────────────────────╮                         
                        └─┤ ▫ t2 status S  ⇠t0 │                         
                          ╰────────────────────╯                         
```

## File sizes

AGENTS.md rule 8 puts the limit at about 600 lines. Counts on `main` at `2cb7e3c`, with M6.5's and M8a's where they change them:

| File | Lines | Budget in this milestone |
|------|------:|------|
| `crates/tui/src/app/mod.rs` | 586 (≈ 592 after M6.5, plus M8a's arms) | **At most 14 lines**: four `App` fields and their initialisers, the `DaemonMsg::Run(reply) => self.on_run_reply(reply)` arm replacing M8a's ignore arm, one `Modal::EditTask` variant, one `on_paste` arm. Everything else in `app/runs.rs`. |
| `crates/tui/src/tree.rs` | 459 | ≤ 560: the enum variants, `build` delegating to `build_with_runs`, `agent_order`, `toggle`/`prune` arms. Grouping in `tree/runs.rs`, run rows in `tree/run_rows.rs`. |
| `crates/tui/src/graph/paint.rs` | 475 | ≤ 520: the calls into `graph/paint/style.rs` (new), which holds glyphs, borders, dimming and the highlight. |
| `crates/tui/src/graph/mod.rs` | 234 | `content_text` arms call `graph/run_text.rs` (new). |
| `crates/tui/src/inspector.rs` | 300 | ≤ 360: the constants, `FieldLayout`, `inspect`'s new arms. Projections in `inspector/run.rs` and `inspector/run_task.rs`, each ≤ 450. |
| `crates/tui/src/inspector/panel.rs` | 344 | ≤ 370: the dispatch; the rows renderer in `inspector/panel/rows.rs`. |
| `crates/tui/src/tree_input.rs` | 310 | ≤ 400: `nav_rows` replaces its direct `tree::build` calls; run-view keys go to `App::on_run_view_key` in `app/runs.rs`. |
| `crates/tui/src/ui/overview.rs` | 236 | ≤ 320. |
| `crates/tui/src/ui/statusbar.rs` | 342 | ≤ 380. |
| `crates/tui/src/keymap.rs` | 570 | Only decision 27's reorder, if needed: ≤ 4 lines. |
| `crates/tui/src/dialog.rs` | 545 | Untouched; the form is `run_edit.rs`. |
| `crates/tui/src/mouse.rs` | 212 | ≤ 250. |
| `crates/tui/src/app/modal_keys.rs`, `app/lifecycle.rs`, `ui/modal.rs` | 303, 49, 161 | One dispatch arm each (three arms in `lifecycle.rs`). |
| `crates/proto/src/run_info.rs` | M8a's | ≤ 600 with the additions. |
| `scripts/pty-smoke.py` | 1572 | ≤ 10 lines: import and call; the stage lives in `scripts/pty_smoke_run_view.py` (new). |

Every new file stays under 600 lines; test modules are separate files (`*_tests.rs` or `tests/` directories, following the existing `#[path]` pattern).

## Tasks

Shared test helpers, created in M8c.3 and extended as tasks need them:

- `crates/tui/src/tree/tests/run_fixtures.rs` (declared from `tree/tests.rs`, `pub(crate)` so `graph`, `inspector`, `app` and `ui` tests use it): `run(id, project, state) -> RunInfo` with every other field defaulted (`serde_json::from_value` of a minimal object, so a test never lists fields it does not care about); `task(id, title, size, state) -> TaskInfo`; `worker(session, window, runtime, started) -> AgentRoundInfo`; `reviewer(round, window, runtime, started) -> AgentRoundInfo`; `headless(id, name, project, run_ref) -> WindowInfo`; `snapshot(now, runs) -> RunsSnapshot`; `gate_fixture()`, `three_task_fixture()`, `gemini_fixture()` returning `(RunsSnapshot, Vec<WindowInfo>)` exactly as the Interfaces section describes them.
- `crates/tui/src/app_tests/runs.rs` (declared from `app/tests.rs` with `#[path]`): `app_with_runs(windows, snapshot) -> App` that delivers the snapshot through `on_daemon(DaemonMsg::Run(RunReply::Snapshot(..)))`, and `open_run_view(&mut App, run_id)` that does it with keys (`C-b T`, select, `l`).

### M8c.1 Snapshot fields the view reads

**Files.** Modify `crates/proto/src/run_info.rs`, `crates/proto/src/lib.rs` (re-exports, `PROTO_VERSION`), `crates/proto/src/run_tests.rs`, `crates/daemon/src/run/model.rs`, `crates/daemon/src/run/edits.rs`, `crates/daemon/src/run/snapshot.rs`, `crates/daemon/src/run/engine/requests.rs`, `engine/ladder.rs`, `engine/done.rs` (or wherever M8a's final code handles `Approve`, `Edit`, rung 1 and `ApiRetry`), and the engine tests under `crates/daemon/src/run/engine/tests/`.

**Tests first.**

- `run_tests.rs`: `snapshot_view_fields_round_trip` — a `RunsSnapshot` with `now`, `approved_hhmm`, two `plan_edits`, a task with `brief`, `acceptance` and a `DiffStats`, a round with `rate_limited_since` and two `sent_back_at` entries, one `ScoutInfo` and one `PlannerInfo`, survives MessagePack (`rmp_serde::to_vec_named`) inside `DaemonMsg::Run(RunReply::Snapshot(..))`. `snapshot_without_view_fields_defaults` — the JSON M8a's own round-trip test produces (no new keys) deserializes with `now == 0`, empty lists and every `Option` `None`. Update the `proto_version_is_*` test.
- `run/edits.rs`: `describe_names_every_edit_op` — one of each of the nine ops gives `add t9, split t2, cancel t3, amend t4, dep t4 on t2, answer t5, pause, resume, finish`.
- Engine unit tests with M8a's `Fixture`:
  - `approve_records_when_the_plan_was_approved`: `approved_at == Some(now)` after `Approve`; a `Start` with `yes` sets it to the start time.
  - `accepted_edits_are_logged_and_counted_after_approval`: an edit before approval logs one event and leaves the count 0; two after approval make it 2; a rejected batch logs nothing.
  - `a_retry_streak_records_its_start`: `ApiRetry` at 100 sets `rate_limited_since = Some(100)`; another at 130 keeps 100; the next non-retry event clears it.
  - `rung_one_marks_the_worker_round_sent_back`: a check failure at `now = 500` on a task at rung 0 leaves one worker round with `sent_back_at == [500]`, and a stall (rung 2) appends a new round instead.
  - `snapshot_carries_the_view_fields`: `snapshot(state, 777).now == 777`; the task's `brief` and `acceptance` equal the plan's; `plan_edits` is newest first and capped at 10 after 12 edits.

**Change.** Interfaces "proto" and "daemon". Add the placeholder types and fields of "Consumes from later milestones" only when absent (decision 5).

**Acceptance.** `cargo test -p anthrex-proto -p anthrex-daemon` passes. A `run.json` written by M8a's code (take one from `e2e_green_s_task_runs_to_merged`'s data directory, or build it with the fixture and `serde_json` before this change) loads unchanged — test `old_run_json_loads` in `engine/tests/`. The derivation of `PROTO_VERSION` is recorded under "Implementation notes".

**Commit.** `feat(proto): carry approval time, plan edits, briefs, rate-limit starts and bounces in the run snapshot`

### M8c.2 The client's copy of the snapshot

**Files.** Create `crates/tui/src/app/runs.rs`, `crates/tui/src/app_tests/runs.rs`. Modify `crates/tui/src/app/mod.rs` (fields, the `DaemonMsg::Run` arm), `crates/tui/src/app/link.rs` (`on_reconnected`, `on_send_failed`), `crates/tui/src/lib.rs` (send `run_subscription()` right after `App::new`), `crates/tui/src/app/tests.rs` (declare the test module).

**Tests first**, in `app_tests/runs.rs`:

- `the_first_effect_subscribes_to_runs`: `app.run_subscription()` is `Effect::Send(ClientMsg::Run(RunRequest::Subscribe))`.
- `a_snapshot_replaces_the_last_one_even_with_a_lower_revision`: revision 57, then revision 3 (a restarted daemon): `app.runs.revision == 3` and its runs are the second snapshot's.
- `reconnecting_resubscribes_to_runs`: `on_reconnected(windows)` returns exactly one `Send(Run(Subscribe))` beside M6's own `Subscribe`.
- `a_refused_run_subscription_is_retried_on_tick`: `on_send_failed(&ClientMsg::Run(RunRequest::Subscribe))`, then `on_tick()` returns the subscription again, once.
- `run_age_counts_from_the_daemons_clock`: a snapshot with `now = 5000`; `run_age(4000) == 1000` immediately after (the `Instant` part is below one second in a test).
- `done_replies_toast_and_refusals_toast`: `Done { request: "run approve", message: "run r1 approved" }` toasts that text; `Refused { request: "run reject", message: "no such run" }` toasts it; `Started`, `ConfirmNeeded` and `ToolResult` change nothing.

**Change.** Decisions 1, 2 and 34 (without the edit form, which M8c.9 adds).

**Acceptance.** Tests pass; `app/mod.rs` grew by no more than its budget.

**Commit.** `feat(tui): subscribe to the run snapshot and keep the latest copy`

### M8c.3 Runs in the project tree

**Files.** Modify `crates/tui/src/tree.rs`, `crates/tui/src/tree/rows.rs` (`visible_windows` takes the hidden set), `crates/tui/src/ui/tree_view.rs` (`narrow_line` arms), `crates/tui/src/ui/sidebar.rs`, `crates/tui/src/app/runs.rs` (`App::rows` now calls `build_with_runs`; `prune_runs` after each snapshot and each window list), `crates/tui/src/tree_input.rs`, `crates/tui/src/app/windows.rs` and `crates/tui/src/mouse.rs` (their `tree::build` calls become `self.rows()`; their exhaustive `NodeKey` matches gain arms: every new key activates through `activate_tree_node`). Create `crates/tui/src/tree/runs.rs`, `crates/tui/src/tree/tests/runs.rs`, `crates/tui/src/tree/tests/run_fixtures.rs`.

**Tests first**, in `tree/tests/runs.rs`, asserting row keys and depths in order:

- `a_run_is_one_node_above_the_plain_windows`: the gate fixture plus a second plain window gives `[Project(/r/demo), Run(add-reset-3f9a), Window(1), Window(2)]`, depths `[0, 1, 1, 1]`.
- `a_runs_windows_are_not_listed_as_plain_windows`: the three-task fixture's windows 3 and 6 are absent from `build_with_runs`; window 1 (plain) is present.
- `a_project_with_only_a_run_still_has_a_row`: a snapshot whose run's project has no window yields `[Project, Run]` and does not panic (today's `expect`).
- `terminal_runs_are_not_shown`: `accepted`, `discarded` and `failed` runs give no row; `complete` does.
- `windows_of_an_unknown_run_are_plain` (review focus 3 and 4): a headless window whose `run` names `gone-0000`, which the snapshot does not have, and a headless window of a `discarded` run, are both listed as plain windows under their project, in id order; with an empty snapshot (none received yet) every window is plain.
- `project_status_includes_its_runs`: a project with one `Idle` window and a `running` run with a blocked task rolls up to `Attention`; with no blocked task, to `Working`; runtime counts count the plain window only.
- `the_orchestrator_takes_a_position_and_the_number_keys_reach_it`: the three-task fixture plus plain window 1: `Run` row `position == Some(1)`, window 1 position 2, `agent_order == [3, 1]`.
- `a_run_without_an_orchestrator_has_no_position`: the gate fixture: `Run` row `position == None`, `agent_order == [1]`.
- `the_filter_matches_a_runs_goal_and_id`: filter `reset` keeps `[Project, Run]` and drops a window named `api`; filter `3f9a` the same.
- `prune_runs_drops_keys_of_runs_that_left`: collapsed `Task { run: "a", .. }` and `Run("a")` survive `prune_runs` while run `a` is shown and are gone after it is discarded; `prune(&windows)` keeps them.
- In `ui/tree_view_tests.rs`: `the_sidebar_line_of_a_run_names_its_goal_and_progress` — the rendered line contains `◉`, `Add password reset` and `0/2`, and the orchestrator run's line contains its position `1`.

**Change.** Decisions 6–10; the `NodeKey`/`RowKind` variants of Interfaces (`Planner`, `Scout`, `Task`, `AgentRound` rows are built from M8c.4 on; their match arms exist from here).

**Acceptance.** Every existing tree, sidebar and overview test passes unchanged (`build` still means "no runs"). `rg -n "tree::build\(" crates/tui/src --glob '!**/tests*'` prints only `tree.rs` itself.

**Commit.** `feat(tui): show each run as one node under its project and hide the windows it owns`

### M8c.4 The run-view rows

**Files.** Create `crates/tui/src/tree/run_rows.rs`, `crates/tui/src/tree/tests/run_rows.rs`. Modify `crates/tui/src/tree.rs` (module declaration, re-exports).

**Tests first**, each asserting `(key, depth)` in order:

- `tiers_follow_the_information_flow`: a run with scouts `S2` (started 20) and `S1` (started 10), planner `A` owning `t1`, `t2`, root tasks `t0` and `t6`, gives `Run; Scout S1; Scout S2; Task t0; Planner A; Task t1; Task t2; Task t6` at depths `0, 1, 1, 1, 1, 2, 2, 1` when `t0` is wave 0, `t1` wave 1, `t2` wave 1, `t6` wave 2 (planner `A` sorts as `t1`, `(1, index)`).
- `tasks_order_by_wave_then_plan_order`: tasks listed `t3 (wave 1), t1 (wave 0), t2 (wave 1)` come out `t1, t3, t2`.
- `a_planner_with_no_tasks_sorts_after_the_root_tasks`.
- `a_task_whose_epic_has_no_planner_hangs_from_the_root`.
- `worker_rounds_split_where_they_were_sent_back`: one worker session started 100 with `sent_back_at [300, 500]` and reviewer rounds 1 (started 200) and 2 (started 400) give `worker #1 (100–300), review #1, worker #1 r2 (300–500), review #2, worker #1 r3 (500–)`, the keys' `round` being `1, 1, 2, 2, 3`; only `r3` has `last == true`.
- `appended_worker_rounds_map_one_to_one`: two worker `AgentRoundInfo`s of session 1 with `round` 1 and 2 and empty `sent_back_at` give the same keys as the split above would.
- `a_fresh_session_is_worker_2`: sessions 1 and 2 give labels `worker #1` and `worker #2`.
- `sub_agents_hang_under_the_last_display_round_only`: window 6 with two sub-agents; they appear once, under `worker #1 r2`, at depth `task + 2`.
- `a_round_whose_window_is_not_listed_has_no_sub_agents_and_no_panic` (review focus 3): `window_id: Some(99)` with no window 99 still gives the round's row, with `window: None`.
- `filters_keep_ancestors_and_drop_the_rest`: `Running` keeps a `working` task and its live round and drops a `merged` task; `Blocked` keeps only the `blocked` task, its planner and the root; `Runtime(Codex)` keeps a Codex-routed task with all its rounds, and of a Claude-routed task keeps only its Codex reviewer round (with the task as its ancestor); the root survives every filter even when nothing else does.
- `the_text_filter_keeps_a_matching_task_with_all_its_rounds`.
- `a_collapsed_task_hides_its_rounds`.
- `two_hundred_tasks_build_in_order` (review focus 2): a run of 200 tasks, each with a worker and two reviewer rounds, builds 801 rows, the last row being the last task's second review, with no key repeated.

**Change.** Decisions 12–15 and 21.

**Acceptance.** Tests pass.

**Commit.** `feat(tui): build the run view's tiers, rounds and filters from the snapshot`

### M8c.5 Node content and live styling on the canvas

**Files.** Create `crates/tui/src/graph/run_text.rs`, `crates/tui/src/graph/paint/style.rs`, `crates/tui/src/graph/paint/tests/runs.rs`. Modify `crates/tui/src/graph/mod.rs` (`content_text` arms), `crates/tui/src/graph/paint.rs`, `crates/tui/src/theme.rs`, `crates/tui/src/ui/overview.rs` (`footer_parts` arms — the single line of Interfaces), `crates/tui/src/graph/paint/tests.rs` (declaration).

**Tests first.**

- `run_text` unit tests: `task_text_keeps_its_tail_when_the_title_is_long` — `t7`, `map Gemini hook events to status`, M, deps `[t0, t6]` gives exactly `t7 map Gemini … M  ⇠t0t6`; `task_text_with_a_hub_mark` gives `t0 proto M ◆`; `task_text_drops_the_title_before_the_deps` — `t1`, S, deps `d0` to `d9`, gives exactly `t1 S  ⇠d0d1d2d3d4d5d6d7…`; `round_text` gives `worker #1 r2 codex` and `review #1 codex`; `run_text` gives `orchestrator  1/3` with an orchestrator and `run 3f9a  0/2` without, and cancelled tasks are left out of the total.
- `project_overview_draws_the_run_node` and `run_view_draws_tiers_rounds_and_glyphs`: the painted lines equal Interfaces "Canvases, exact" character for character (spinner frame 0 throughout).
- `the_gate_draws_every_task_as_planned`: in `awaiting_approval`, a task whose state is `pending` is drawn `○`, not `◌`.
- `a_working_task_animates_only_while_its_worker_works`: `t1` with its window `Working` is drawn with `SPINNER[frame]`; with `Idle`, `●`.
- `critical_path_tasks_have_a_bold_border`: every border cell of `t1`'s box has `Modifier::BOLD`; `t2`'s have not.
- `selecting_a_task_lights_its_dependencies_and_dims_the_rest`: with `t1` selected in the three-task fixture, `t0`'s border cells use `border_focused(accent)`, `t2`'s cells all have `Modifier::DIM`, the root's too, and `t1`'s are reversed.
- `finished_agent_nodes_are_dim`: `t0`'s worker and reviewer boxes carry `Modifier::DIM`; `t1`'s live worker does not.
- `round_glyphs_follow_the_verdict`: a blocking reviewer `✗` (red), a minor-only `changes` `✓`, a live reviewer `●`, an ended one without a verdict `–`, a rate-limited live worker `◆`, the last worker round of a blocked task `✗`.
- `two_hundred_tasks_paint_without_overflow` (review focus 2): the 200-task run's layout has `layout.nodes.len() == 801` and, with its 600 leaves, `layout.size.1 == 600 * 4 - 1 == 2399`; painting a 120 × 40 viewport with the pan revealing the last task returns 40 lines of display width 120, and the last task's content text is on them.
- `a_row_that_disagrees_with_its_node_is_skipped` (follow-up handled): feeding `paint` a row list whose second key differs from the layout's second node paints nothing for that node and does not panic in release (`#[cfg(not(debug_assertions))]` is not needed: the check is a real `if`, not a `debug_assert`).

**Change.** Decisions 17–20; the Glyphs table; the painter's zip check becomes a real key comparison that skips a mismatched node.

**Acceptance.** Tests pass; milestone 4.6's painter tests pass unchanged.

**Commit.** `feat(tui): draw run nodes with live glyphs, the critical path and the dependency highlight`

### M8c.6 Opening, navigating and leaving the run view

**Files.** Modify `crates/tui/src/app/runs.rs` (`RunView`, `nav_rows`, `open_run_view`, `close_run_view`, `open_conversation`, `on_run_view_key`), `crates/tui/src/tree_input.rs` (`nav_rows` everywhere a layout or a selection move needs rows; `l`, `h`, `Enter`, `Esc`, `Space` rules; `activate_tree_node` arms), `crates/tui/src/mouse.rs` (`click_graph` reads `nav_rows`; sidebar clicks on `Run` rows and headless windows route through `activate_tree_node`), `crates/tui/src/ui/overview.rs` (`view`, `render` read `nav_rows`; the block title), `crates/tui/src/ui/statusbar.rs` (the two hints), `crates/tui/src/keymap.rs` (decision 27, only if needed). Tests in `crates/tui/src/app_tests/runs.rs`, `crates/tui/src/app_tests/overview/runs.rs` (new, declared from `app_tests/overview.rs`), `crates/tui/src/ui/statusbar_tests.rs`, `crates/tui/src/keymap.rs`'s tests.

**Tests first.**

- `l_on_a_run_node_opens_the_run_view` and `enter_on_a_run_node_opens_the_run_view`: `run_view == Some(RunView { run_id, filter: All })`, selection `Run(run_id)`, `graph_pan == Pan::default()`, `nav_rows()[0].key == Run(run_id)`, no effect returned.
- `enter_on_a_run_row_in_the_sidebar_tree_opens_the_overview_on_it`.
- `space_on_a_run_node_in_the_project_tree_does_nothing`: `tree.collapsed` stays empty.
- `h_at_the_root_and_esc_return_to_the_project_overview`: both close the run view with `Run(run_id)` selected and `overview` still on; a second `Esc` leaves tree mode.
- `h_below_the_root_selects_the_parent` and `j_k_walk_the_run_view_in_order` (the order asserted against `tree::run_rows`, not restated).
- `enter_on_the_root_focuses_the_orchestrator`: the three-task fixture returns exactly `[Send(Subscribe { window_id: 3, .. })]`, `focused == Some(3)`, tree mode left.
- `enter_on_the_root_without_an_orchestrator_toasts`: the gate fixture toasts `run add-reset-3f9a has no orchestrator window; Enter on an agent opens its conversation`, returns nothing.
- `enter_on_a_round_opens_its_conversation_without_focusing`: on `t1`'s worker returns exactly `[Send(SubscribeConversation { window_id: 6, agent_id: None, from_rev: None })]`; `focused` unchanged; `keymap.conversation_mode()`; `run_view` and `overview` unchanged. After M6.5's `q` closes the conversation, `nav_rows` is the run view and the selection is still the round.
- `enter_on_a_task_opens_its_live_rounds_conversation` and `enter_on_a_task_with_no_agent_toasts` (`t2 has no agent yet`).
- `enter_on_a_round_whose_window_is_not_listed_yet` (review focus 3): toasts `window #99 is not listed yet`; an ended round whose window was retired toasts `worker #1 has finished and its window is gone`. Neither sends anything.
- `enter_on_a_headless_window_in_the_plain_tree_opens_its_conversation` (review focus 5): a headless window of an unknown run, listed plain, selected in the sidebar tree and in the project overview: Enter returns exactly one `SubscribeConversation` for it and no `ClientMsg::Subscribe`; a sidebar click and a canvas double click do the same; `focused` is unchanged. A PTY window's Enter still focuses it.
- `no_run_view_path_sends_input`: drive every key of this milestone (`a x e d f h l j k i / Enter Space Esc` and printable letters) and every mouse gesture over the three-task fixture, and assert no effect is `Send(ClientMsg::Input { .. })`.
- `f_cycles_the_filters`: `All → Running → Blocked → Runtime(Claude) → Runtime(Codex) → All`; the selection is repaired into the filtered rows each time.
- `the_run_view_closes_when_its_run_leaves`: a snapshot without the run closes it and toasts `run add-reset-3f9a is gone`; one with it `discarded` toasts `run add-reset-3f9a is discarded`.
- `reveal_follows_the_selection_in_the_run_view` (review focus 2): in the 200-task run at 120 × 40, pressing `j` 800 times from the root leaves the last row's rectangle inside the viewport, with `graph_pan.y > 0`.
- `conversation_mode_wins_over_tree_mode` (keymap): with both modes on, `j` is `KeyAction::Conversation`.
- `statusbar_shows_the_run_view_hints`: exact strings of Interfaces for the gate and for a running run with filter `running`; the overview title `run add-reset-3f9a` is on screen and ` tree overview ` is not.

**Change.** Decisions 11, 22–27.

**Acceptance.** Tests pass; milestone 4.6 and 4.7's overview and mouse tests pass unchanged.

**Commit.** `feat(tui): open the run view from its node, walk it, and open an agent's conversation from it`

### M8c.7 The run inspector's projections

**Files.** Create `crates/tui/src/inspector/run.rs` (run, planner, scout, the shared formatting of decisions 30–31), `crates/tui/src/inspector/run_task.rs` (task, rounds), `crates/tui/src/inspector/run_tests.rs`. Modify `crates/tui/src/inspector.rs` (constants, `FieldLayout`, `Inspection.right`/`layout`, `inspect`'s arms), every existing `Inspection { .. }` construction.

**Tests first**, each asserting the exact `(label, value)` list, `name` and `right` (the Interfaces tables are the expected values):

- `run_fields_match_the_mockup` (Gemini fixture), `run_fields_at_the_gate` (gate fixture: `progress ░░░░░░░░░░░░░░░░░░  0/2 merged · 2 waiting`, `gate awaiting approval · a approve · x reject · e edit · d remove`, no `path`, no `attention`), `run_fields_when_halted` (`attention halted: refs/heads/main moved from 1a2b3c4 to 5d6e7f8`), `run_fields_with_one_edit_and_yes` (`plan approved by --yes 09:15 · 1 plan edit since · last: cancel t3`), `run_spend_without_cache_or_estimate` (`tokens 0 · tool calls 0`).
- `planner_fields_match_the_mockup`, `planner_while_planning` (`planning · 1m`).
- `task_fields_match_the_mockup`, `task_stage_marks` (one case per mark of the `stages` row: a check-mode task shows `proof –`; an unverified run `check –`; an unreviewed S task `review –`; `merge ●` in the queue), `task_deps_include_implicit_ones_once`, `task_fields_when_blocked` (`right` ends `blocked: mis-sized`), `task_without_diff_or_review_omits_them`, `task_budget_with_tokens` (`… · 410k/3.0M tokens`).
- `worker_round_fields_match_the_mockup`, `the_first_display_round_has_no_fixing_and_no_activity` (`worker #1` of `t2`: fields `doing finished`, `session …`), `fixing_names_a_failed_check` (`check failed: error[E0308]: mismatched types`), `a_rate_limited_round` (`right` starts `rate-limited ·`, `doing rate-limited · waiting out the runtime's retry`).
- `reviewer_round_fields_match_the_mockup`, `a_live_reviewer_is_reviewing`, `minor_only_changes_count_as_approval`.
- `scout_fields_match_the_mockup`.
- `format_duration_cases` (`0s`, `59s`, `1m`, `59m`, `1h00m`, `1h12m`, `26h05m`), `format_tokens_cases` (`999`, `1k`, `410k`, `999k`, `1.0M`, `1.8M`, `12.3M`), `progress_bar_cases` (5/9 at 18 → 10 filled; 1/3 at 10 → 3; 0/0 is not called — `no tasks yet`; 9/9 → all filled).
- `a_subagent_in_the_run_view_keeps_the_column_layout`.

**Change.** Interfaces "Inspector contents, exact" and decisions 29–31, projection half only.

**Acceptance.** Tests pass; milestone 4.7's projection tests pass with only the two new struct fields added to their expectations.

**Commit.** `feat(tui): project runs, planners, scouts, tasks and agent rounds into the inspector`

### M8c.8 The tall panel and its row layout

**Files.** Create `crates/tui/src/inspector/panel/rows.rs`, `crates/tui/src/inspector/panel/rows_tests.rs`, `crates/tui/src/app_tests/overview/run_inspector.rs`. Modify `crates/tui/src/inspector/panel.rs` (dispatch), `crates/tui/src/ui/overview.rs` (`areas` gains `run_view`; `View::shows_panel` compares against the height it was given), `crates/tui/src/tree_input.rs` (`set_graph_viewport` passes `run_view.is_some()`), `crates/tui/src/app_tests/overview/inspector.rs` (callers of `areas`).

**Tests first.**

- In `rows_tests.rs`, drawn into a `TestBackend` and compared cell by cell: `run_panel_matches_the_mockup`, `planner_panel_matches_the_mockup`, `task_panel_matches_the_mockup`, `task_panel_drops_the_right_text_and_elides_values`, `worker_round_panel_matches_the_mockup`, `reviewer_round_panel_matches`, `scout_panel_wraps_the_question` — each equal to its block in Interfaces. `run_panel_in_eight_rows_drops_from_the_end`: the Gemini run at 86 × 8 shows `progress` to `gate` and not `attention`. `the_wrapped_field_leaves_a_row_for_each_later_field`: a 200-character question at 60 × 12 wraps to four rows and every later field is still shown.
- In `app_tests/overview/run_inspector.rs`: `the_run_view_gets_the_tall_panel` (interior 30: footer height 12; the project overview at the same size: 8); `a_short_terminal_steps_down` (interior 17: 8; interior 13: 1); `i_toggles_the_tall_panel_too`; `no_panic_at_degenerate_sizes_in_the_run_view` (review focus 1) — milestone 4.7's guard loop over widths `[1, 2, 3, 4, 12, 40, 61]` and heights `0..=MIN_INTERIOR_FOR_RUN_PANEL + 4`, inspector on and off, run view open on the three-task fixture with `t1` selected, drawing, clicking, dragging and scrolling at three points, reopening the run view when a double click left it, asserting the canvas and footer heights sum to the interior and that a 12-row footer always leaves six canvas rows.
- `the_single_line_for_a_task`: inspector off, the footer line is exactly `◐ t2  map Gemini hook events to status  M · tdd · review round 2` (Gemini fixture, `t2` selected).

**Change.** Decisions 28 and 29, rendering half.

**Acceptance.** Tests pass; milestone 4.7's panel tests pass unchanged.

**Commit.** `feat(tui): give run nodes a twelve-row inspector laid out one field per row`

### M8c.9 The plan gate

**Files.** Create `crates/tui/src/run_edit.rs`, `crates/tui/src/run_edit_tests.rs`, `crates/tui/src/ui/run_edit.rs`, `crates/tui/src/ui/run_edit_tests.rs`, `crates/tui/src/app_tests/gate.rs`. Modify `crates/tui/src/app/runs.rs` (keys `a x e d`, `on_edit_task_key`, reply handling of the form), `crates/tui/src/app/mod.rs` (`Modal::EditTask`, the `on_paste` arm), `crates/tui/src/app/lifecycle.rs` (three `perform` arms), `crates/tui/src/app/modal_keys.rs` (one dispatch arm), `crates/tui/src/ui/modal.rs` (one render arm), `crates/tui/src/lib.rs` (`pub mod run_edit;`).

**Tests first.**

- `app_tests/gate.rs`:
  - `a_asks_then_approves`: `a` opens `Modal::Confirm { message: "Approve run add-reset-3f9a? 2 tasks start.", action: ApproveRun("add-reset-3f9a") }`; `y` returns exactly `[Send(Run(Approve { run_id: "add-reset-3f9a" }))]`; `n` returns nothing.
  - `x_asks_then_rejects`, `d_on_a_task_asks_then_removes` (`[Send(Run(Edit { run_id, edits: [CancelTask { task_id: "t2" }] }))]`), with the Interfaces messages.
  - `gate_keys_outside_the_gate_toast`: on a `running` run, each of `a x e d` toasts `the plan gate is closed: run add-reset-3f9a is running` and sends nothing.
  - `e_or_d_on_the_root_toasts`: `select a task to edit or remove`.
  - `the_edit_form_sends_only_what_changed`: open on `t1` (Claude, `claude-sonnet-5`, standard, medium, M, tdd, brief `Line one\nLine two`); change effort to high and size to S; `Enter` returns exactly `[Send(Run(Edit { run_id, edits: [AmendTask { task_id: "t1", route: Some(RouteSpec { runtime: Some(Claude), model: Some("claude-sonnet-5"), strength: Some(Standard), effort: Some(High) }), size: Some(S), brief: None, acceptance: None, test_mode: None, test_mode_reason: None, priority: None }] }))]` and marks the form `submitting`.
  - `changing_the_runtime_clears_the_model`: Claude → Codex empties the model field, and the sent `RouteSpec` has `model: None`.
  - `a_mode_other_than_tdd_needs_a_reason`: `check` with an empty reason keeps the form open with focus on `Reason` and the error; with reason `renames only` sends `test_mode: Some(Check), test_mode_reason: Some("renames only")`.
  - `the_brief_round_trips_its_newlines`: the field shows `Line one↵Line two`; `Ctrl-J` then `x` at the end sends `brief: Some("Line one\nLine two\nx")`.
  - `nothing_changed_closes_with_a_toast`.
  - `a_refused_edit_shows_inline_and_a_done_edit_closes`: `Refused { request: "run edit", message: "task t1: size: …" }` sets `form.error` and clears `submitting`; `Done { request: "run edit", .. }` closes the modal and toasts.
  - `a_paste_goes_to_the_focused_text_field`: newlines become `↵` in `Brief` and are dropped in `Model`.
- `run_edit_tests.rs`: `visible_fields_hide_the_reason_for_tdd`, `choices_cycle_both_ways`, `esc_and_ctrl_c_cancel`.
- `ui/run_edit_tests.rs`: `the_form_renders_its_fields` — at 80 × 24, the form's rows contain exactly `› runtime    ‹ claude ›`, `  size       ‹ M ›`, `  test mode  ‹ tdd ›`, `  brief      Line one↵Line two`, and the hint `⏎ save  tab next  ←/→ change  esc cancel`; the title ` edit t1 `.

**Change.** Decisions 32–34.

**Acceptance.** Tests pass. `rg -n "ClientMsg::Input" crates/tui/src/app/runs.rs crates/tui/src/run_edit.rs crates/tui/src/ui/run_edit.rs` prints nothing.

**Commit.** `feat(tui): approve, reject and edit a run's plan from the run view`

### M8c.10 The smoke stage and the milestone's paperwork

**Files.** Create `scripts/pty_smoke_run_view.py`. Modify `scripts/pty-smoke.py` (≤ 10 lines), `docs/ROADMAP.md` (status), `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`, this brief's "Implementation notes".

**Tests first.** The stage itself. `run_view_stage(repo_root, pty_proc, run_cmd, fail)`, called after M8a's stage 11c and after M8b's stage 11d when that has landed first, and printing `== stage 11e: the run view shows the plan gate, approves it, and opens a worker's conversation ==` (the run milestones' stage letters are fixed: M8a `11c`, M8b `11d`, M8c `11e`, M9 `11f`, M9.5 `11g`), using stage 11c's environment (the one daemon `pty-smoke.py` starts, with `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` pointing at `fake-agent`) and its helpers for the repository and the fake-agent scripts:

1. Create `/tmp/anthrex-smoke-view-<pid>` (a repository with an identity and one commit) and the plan `/tmp/anthrex-smoke-view-plan-<pid>.toml`: profile `check = "true"`; one S task `t1`, title `add a`, `test_mode = "check"`, reason `smoke`, owns `["a.txt"]`, route runtime `claude`. Write `<repo>/.git/fake-agent/worker-t1-1.jsonl` whose only step is `{"wait_ms": 600000}` (a turn that stays open, so the worker round is live).
2. `anthrex run start --plan … --dir <repo>` (no `--yes`); read the run id and its last four characters `h4`.
3. In the attached TUI: `C-b T`; `/`, type `h4`, `Enter`; `l` (the project's first child, the run); wait for `run <h4>  0/1`. `l` again; wait for ` run <id> ` and `t1 add a S` and the hint `a approve`.
4. `a`; wait for `Approve run <id>?`; `y`. Wait, up to M8a's `RUN_WAIT` from `scripts/pty_smoke_run.py` (dispatch is preflighted git work bounded by the same constants), for `worker #1 claude`.
5. `l`, `l` (to `t1`, then its worker round); `Enter`; wait, up to the M6.5 smoke stage's derived conversation bound, for the window name `<h4>/t1.w1` in the conversation's header.
6. `q`; wait for ` run <id> `. `Esc`; wait for ` tree overview `. `Esc`.
7. `anthrex run cancel <id>`, then `anthrex run discard <id> --confirm <id>`; in `finally`, remove both paths.

**Change.** The stage. Set milestone 8c to `done` in `docs/ROADMAP.md`. In the follow-ups file, record under a new "From milestone 8c" heading: the painter zip check handled (see "Follow-ups handled"); the `doing` field's missing tool target (below); and any field of "Consumes from later milestones" still unfilled, assigned to its owner.

**Acceptance.** `python3 scripts/pty-smoke.py` passes with stage 11e. After it, `pgrep -fl "anthrex daemon"` and `pgrep -fl fake-agent` show nothing of yours, and `/tmp/anthrex-smoke-view-*` is gone.

**Commit.** `test: add a smoke stage for the run view and its plan gate`

## Verification

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

Milestone-specific checks:

- `PROTO_VERSION` equals the header's derivation, recorded under "Implementation notes".
- `rg -n "std::fs|std::process|std::thread|tokio|SystemTime|Instant::now" crates/tui/src/inspector crates/tui/src/tree crates/tui/src/graph crates/tui/src/run_edit.rs crates/tui/src/ui/run_edit.rs crates/tui/src/inspector.rs crates/tui/src/tree.rs` prints nothing (`app/runs.rs` may read `Instant` for `runs_received_at`, as `app/mod.rs` already does).
- `rg -n "ClientMsg::Input" crates/tui/src` lists only the sites M8a.17 already guards.
- `wc -l` on every file in the file-size table and every new file: nothing over its budget.

## Manual check

With an isolated daemon, config and throwaway repository, and the real `claude` and `codex` installed:

```bash
export ANTHREX_SOCKET=/tmp/anthrex-m8c/daemon.sock ANTHREX_DATA_DIR=/tmp/anthrex-m8c/data ANTHREX_CONFIG=/tmp/anthrex-m8c/config.toml
mkdir -p /tmp/anthrex-m8c && cd /tmp/anthrex-m8c && git init -b main demo && cd demo \
  && printf 'hello\n' > README.md && git add . && git commit -m init
```

1. Write `/tmp/anthrex-m8c/plan.toml`: profile `check = "true"`; `t1` an S `check`-mode task (reason `manual`) owning `hello.sh`, Codex; `t2` an S `docs` task owning `README.md`, Claude, depending on `t1`. `anthrex run start --plan /tmp/anthrex-m8c/plan.toml --dir /tmp/anthrex-m8c/demo`.
2. `anthrex` to attach. `C-b T`: the project `demo` shows one `run <last four characters of the id>  0/2` node and no window of the run. Select it, press `l`: the run view shows `t1`, then `t2` with `⇠t1`, both `○`; the status bar shows the gate hints; the panel is twelve rows on a tall terminal.
3. `e` on `t2`: change effort, `Enter`; the toast says the edit was applied and the inspector's `route` changed. `d` on neither — `Esc` from a confirm leaves the plan as it is.
4. `a`, `y`. Watch `t1` go `▫ → ● → ◇ → ◐ → ▸ → ✓`, a `worker #1 codex` node appear and dim when it finishes, a `review #1 claude` node appear after it with its verdict glyph, then `t2` start. Resize the terminal below 18 and then below 14 rows: the panel steps to 8 rows, then one line, and the canvas stays.
5. Select the live worker and press Enter: its conversation opens, live; typing does nothing to the agent; `q` returns to the same node. Select the root and press Enter: the toast says the run has no orchestrator window.
6. `f` through each filter, `/` a task id; select `t2` and confirm `t1` is lit and everything else dimmed.
7. `Esc`, `Esc`. `anthrex run cancel <id>`, `anthrex run discard <id> --confirm <id>`. While the run's reviewer window lingers as `Exited` (up to 30 s), confirm it is listed as a plain window under `demo` and that Enter on it opens its conversation, not a terminal.
8. `anthrex daemon stop`; `pgrep -fl "anthrex daemon"` shows nothing of yours; remove `/tmp/anthrex-m8c`.

## Review focus

The five failure modes most likely to bite a user that the tests above would not exercise without being told to. Each has a named test in its owning task; the reviewer checks that each test exists and fails without the change it guards.

1. **Tiny terminals.** The tall panel adds a third height step; an off-by-one in `areas` or in the rows renderer panics or eats the canvas at the thresholds. Test: `no_panic_at_degenerate_sizes_in_the_run_view` (M8c.8), plus `a_short_terminal_steps_down`.
2. **200-task runs.** Rounds multiply leaves: 200 tasks with three rounds each is 600 leaves and a canvas of 2399 rows, where a `u16` overflow, a duplicate key or a reveal that cannot reach the bottom would show. Tests: `two_hundred_tasks_build_in_order` (M8c.4), `two_hundred_tasks_paint_without_overflow` (M8c.5), `reveal_follows_the_selection_in_the_run_view` (M8c.6).
3. **A snapshot that runs ahead of the window list.** The engine names a window id in the snapshot before `WindowsChanged` lists it, and the other way round at startup. Nothing may panic, no row may be lost, and Enter must say why it cannot open anything. Tests: `windows_of_an_unknown_run_are_plain` (M8c.3), `a_round_whose_window_is_not_listed_has_no_sub_agents_and_no_panic` (M8c.4), `enter_on_a_round_whose_window_is_not_listed_yet` (M8c.6).
4. **A discarded run's window.** After `discard` the run leaves the tree but a retired reviewer's window is still listed for up to `RETIRE_AFTER`; it must reappear as a plain window, not vanish, and the run view must close rather than show a stale run. Tests: `windows_of_an_unknown_run_are_plain` (M8c.3), `the_run_view_closes_when_its_run_leaves` (M8c.6).
5. **A headless window selected in the plain project tree.** The one place a headless window is still a plain row; Enter, a click or a double click there must open its conversation and never subscribe to a terminal or send input. Tests: `enter_on_a_headless_window_in_the_plain_tree_opens_its_conversation` and `no_run_view_path_sends_input` (M8c.6).

## Risks and gotchas

1. **Two row lists.** The sidebar reads `App::rows()` and the canvas `App::nav_rows()`. Any code that builds rows for a layout, a hit test or a selection move with the wrong one works in the project overview and breaks only in the run view. M8c.3's acceptance greps for stray `tree::build` calls; review every `rows()` call in `tree_input.rs`, `mouse.rs` and `ui/overview.rs` by hand.
2. **The selection key is shared.** `NodeKey::Run(id)` is both the project-tree node and the run-view root, which is what makes entry and exit free — and what makes `Space` on the project-tree node dangerous: folding it would collapse the run view's root. Decision 22 forbids it; `space_on_a_run_node_in_the_project_tree_does_nothing` guards it.
3. **Snapshot times are the daemon's.** Never subtract a snapshot time from the client's clock; always go through `run_age` (decision 2).
4. **Revisions reset.** Do not add a "newer revision only" guard (decision 1); a restarted daemon would be ignored until its revision passed the old one.
5. **Ambiguous-width glyphs.** `◉`, `◆`, `▫`, `◇`, `◐`, `▸`, `⊘` and `⇠` are one column in `unicode_width` and in most terminals, and milestone 4.6 already relies on `◆`. A CJK-locale terminal that draws them two wide misaligns borders exactly as `◆` does today; nothing new is at stake, but do not add a genuinely wide glyph.
6. **The `doing` field is thinner than the mockup.** The spec shows `editing crates/daemon/src/status.rs (last tool: apply_patch)`; neither the snapshot nor `WindowInfo` carries a tool's target. Do not reach into M6.5's conversation state for it (the view would then depend on a subscription the user may not have open); record the follow-up (M8c.10).
7. **M6.5's conversation for a retired window is gone.** `ConversationGone` arrives when the window is removed; decision 25's toast avoids asking for it in the first place.
8. **Fixture drift.** The exact-string tests encode the fixture's numbers. Build the three fixtures once in `run_fixtures.rs` and reuse them; a copy per test file is how two tests come to disagree about the same run.

## Follow-ups handled

- **From milestone 4.6's final review: "The painter's zip invariant is only `debug_assert`ed."** This milestone adds five row kinds, which raises the cost of a future divergence; M8c.5 makes the check a real comparison that skips a mismatched node, with `a_row_that_disagrees_with_its_node_is_skipped`. Mark the entry handled.

Not taken, and why: `TreeState::overview` removal and the sub-agent label reveal gap stay with milestone 7, which owns them.

## Names taken from M8a

Checked against the M8a brief on 2026-09-22. Check each against the merged code when this milestone starts, and record every difference under "Implementation notes".

| Name | What this brief assumes | M8a decision or task |
|---|---|---|
| `proto::RunsSnapshot { revision, runs }` | Pushed as `RunReply::Snapshot` on `Subscribe` and on every publish; M8c.1 adds `now` | Decision 47; Interfaces `run_info.rs`; M8a.2 |
| `proto::RunInfo` | `run_id`, `goal`, `project` (the same root as `WindowInfo.project` of its windows), `state`, `paused_from`, `halted_reason`, `approved_by` (`"user"` or `"--yes"`), `max_writers`, `max_readers`, `max_bounces`, `writers_busy`, `readers_busy`, `unverified`, `tasks` in plan order, `critical_path`, `attention`, `created_at` (unix seconds) | Interfaces `run_info.rs`; decisions 14, 16, 41 |
| `proto::TaskInfo` | `id`, `title`, `epic`, `size`, `hub`, `test_mode`, `deps`, `implicit_deps`, `route`, `review_route` (`None` when not reviewed), `budget`, `spent_session`, `spent_total`, `state`, `block`, `rung`, `bounces`, `stalls`, `branch`, `test`, `red`, `done_signal`, `rounds`, `reviews`, `last_check`, `last_proof`, `on_critical_path`, `wave`, `history` (newest first, `"<hh:mm> <text>"`, at most 10) | Interfaces; decisions 31, 35, 41 |
| `proto::AgentRoundInfo` | `role`, `session`, `round` (a reviewer's review round; a worker's 1 unless M8a appends per bounce), `window_id`, `route`, `started_at`, `ended_at`, `tool_calls`, `turn_open`, `turns`, `rate_limited`, `open_subagents`, `denials`, `usage` — one entry per worker session and one per review round | Decisions 35, 38, 52; Interfaces |
| `proto::ReviewInfo`, `Finding`, `Severity`, `Verdict` | `round` equals the reviewer round's `round`; `blocking` true exactly when a critical or important finding exists; `Severity` orders `Critical < Important < Minor` | Decision 35 |
| `proto::TokenUsage { input, output, cache_read, cache_write }`, `TokenUsage::billable()` | `input` excludes cache reads; `billable()` is decision 40's sum | Decision 40; Interfaces `daemon` |
| `proto::Spend { tool_calls, secs, tokens }`, `Budget { tool_calls, minutes, tokens: Option<u64> }` | `Spend.tokens` is billable tokens | Decision 40 |
| `proto::Route { runtime, model, strength, effort }`, `RouteSpec`, `Strength`, `Effort`, `Size`, `TestMode`, `TaskState` (11 values), `RunState` (8), `BlockReason` (6), `GateCounts`, `DoneSignal` | As listed in M8a's `run.rs`; `Size` serializes `"S"`/`"M"`/`"L"` | Decisions 8, 31; Interfaces `run.rs` |
| `proto::AgentRole { Orchestrator, Worker, Reviewer }` | `Copy, Eq, Hash` (it is part of a `NodeKey`) | Decision 3; Interfaces |
| `proto::RunRef { run_id, task_id, role, session }` on `WindowInfo.run` | Set on every run window; the orchestrator's has `role: Orchestrator`, `task_id: None` | Decision 49; Interfaces `types.rs` |
| `proto::WindowKind { Pty, Headless }` on `WindowInfo.kind` | `Pty` by default; every worker and reviewer window is `Headless` | Decision 49; M8a.17 |
| `RunRequest::{Subscribe, Approve { run_id }, Reject { run_id }, Edit { run_id, edits }}` | `Reject` needs no confirmation over the wire; `Edit` validates the whole batch | Decisions 13, 14, 20; Interfaces `run_wire.rs` |
| `RunReply::{Snapshot, Done { request, message }, Refused { request, message }, Started, ConfirmNeeded, ToolResult}`, `proto::request::{APPROVE, REJECT, EDIT}` | Labels `run approve`, `run reject`, `run edit` | Interfaces `run_wire.rs` |
| `PlanEdit::{AmendTask { task_id, brief, acceptance, route, test_mode, test_mode_reason, priority, size }, CancelTask { task_id }}` | Amending `route`, `size` or `test_mode` is allowed on a task of a run in `awaiting_approval` | Decision 13 |
| `ClientMsg::Run(RunRequest)`, `DaemonMsg::Run(RunReply)` | The TUI's ignore arm in `app/mod.rs` is the one M8c.2 replaces | Decision 3; M8a.2 |
| Window names `<h4>/<task>.w<session>`, `<h4>/<task>.r<round>` | Used by the smoke stage to find the conversation header | Decision 16 |
| `RETIRE_AFTER` (30 s) | A retired window is listed `Exited`, then removed | Decision 52 |
| The TUI placeholder for a focused headless window; `app/link.rs` sends no `Subscribe` for one; no key or mouse input is forwarded to one | Kept as it is; this milestone only adds routes to the conversation | Decision 49; M8a.17 |
| `run/model.rs` `Run`, `AgentRound`, `TaskEvent { at, text }`; `run/snapshot.rs` `snapshot(state, now)` and its `<hh:mm>` helper for `TaskInfo.history`; the engine's `Approve`, `Edit`, rung 1 and `ApiRetry` handling | Where M8c.1's fields are recorded and published | Decisions 38, 40, 47; Interfaces `daemon`; M8a.11, M8a.12, M8a.15 |
| `scripts/pty_smoke_run.py`, `run_stage`, stage `11c`, `RUN_WAIT` | Stage 11e follows 11c (and M8b's 11d when present) and reuses 11c's environment, helpers and bound | M8a.25 |

## Names taken from M6.5

| Name | What this brief assumes | M6.5 task |
|---|---|---|
| `App.conversation: crate::conversation::ConversationView`, `ConversationView::open(window_id) -> Vec<Effect>` | Opens the root conversation of any window and emits `Send(SubscribeConversation { window_id, agent_id: None, from_rev: None })` | M6.5.12 |
| `Keymap::set_conversation_mode(bool)`, `conversation_mode()`, `KeyAction::Conversation(KeyEvent)` | Conversation keys go to the view; `Esc`/`q` close it and clear the mode | M6.5.12 |
| `ui::draw` renders the conversation in `l.main` in preference to the overview | So a conversation opened over the run view covers it and closing it reveals it | M6.5 Interfaces "crates/tui" |
| The `C-b m` toggle's steps | `open_conversation` performs the same steps for a given window (or calls M6.5's helper if it factored one out) | M6.5.12 |

## Consumes from later milestones

Each is added by M8c.1 as written when absent (decision 5) and filled by its owner. The view renders its absence as stated.

| Field | Owner | When absent |
|---|---|---|
| `#[serde(default)] RunInfo.scouts: Vec<ScoutInfo>`; M8b's `ScoutInfo { id: String, kind: ScoutKind, question: String, state: ScoutState, failure: Option<String>, window_id: Option<u32>, route: Route, started_at: u64, ended_at: Option<u64>, tool_calls: u32, report_bytes: Option<u32>, files: Vec<String>, usage: TokenUsage }`; `ScoutKind { Onboarding, Area }`; `ScoutState { Starting, Working, Reported, Failed }` (snake_case) | M8b defines them (M8b Interfaces `scout.rs`; M8b owns the names, so M8c.1 adds them exactly as M8b declares them when M8b has not landed); M9 fills `RunInfo.scouts` with a run's area scouts (M8b's Out table). | No scout nodes |
| `#[serde(default)] RunInfo.usage: Option<RunUsage>`; `RunUsage { total: TokenUsage, by_role: BTreeMap<String, TokenUsage>, decider_calls: u32, decider_fallbacks: u32 }` | M8b (its decision 29: usage by role, the orchestrator's from OTLP) | `spend` sums the rounds' own usage, which leaves out deciders, scouts and the orchestrator |
| `#[serde(default)] TaskInfo.diff: Option<DiffStats>`; `DiffStats { files: u32, hunks: u32, added: u32, removed: u32 }` | M8b (its decision 32 measures a task's diff as `DiffStats` and publishes it as `TaskInfo.diff`, M8b Interfaces `run_info.rs`) | The `diff` field shows only the test part, or is omitted |
| `#[serde(default)] RunInfo.estimate_left_secs: Option<u64>` | M9.5 (history medians; M8b only records history) | No `est. left` |
| `#[serde(default)] RunInfo.bound_ratio_permille: Option<u32>` | M9.5 (history medians) | No `× the bound` |
| `#[serde(default)] RunInfo.planners: Vec<PlannerInfo>`; `PlannerInfo { epic: String, title: String, area: Vec<String>, route: Route, window_id: Option<u32>, state: PlannerState, started_at: u64, ended_at: Option<u64>, edits_accepted: u32, edits_rejected: u32, last_rejection: Option<String>, replans: Vec<String> }`; `PlannerState { Planning, Finished, Failed }` (snake_case, default `Planning`) | M9 | No planner nodes; every task hangs from the root |
| The orchestrator's PTY window with `WindowInfo.run == Some(RunRef { role: AgentRole::Orchestrator, task_id: None, .. })` | M9 | Root drawn `run <h4>`; Enter on it toasts |
| `AgentRole` variants for scouts and sub-planners, on their windows' `RunRef` | M8b (`Scout`), M9 (`Planner`) | Not read by this milestone: scouts and planners are linked by `window_id` |
| Racer and test-writer rounds | M9.5 | Not drawn |

## Implementation notes

