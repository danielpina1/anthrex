# Milestone 8c: The live run view

> Written 2026-09-22 against `docs/superpowers/specs/2026-09-22-adaptive-orchestrator-design.md` ("the spec" below), which is binding for this milestone; its §16 is this milestone's whole subject. Built on the M8a brief `docs/milestones/M8a-orchestration-engine-core.md` (its final, headless version) for every run type, wire message and window rule. The graph overview (milestone 4.6, `docs/superpowers/specs/2026-09-20-graph-overview-design.md`) and the node inspector (milestone 4.7, `docs/superpowers/specs/2026-09-21-node-inspector-design.md`) are extended, never duplicated: there is one graph widget and one inspector. **Refreshed 2026-09-27 against the code milestones 6.5, 8a and 8b shipped** (branch `m8b-adaptation` at `a910e18`, PR #19, merging unchanged; protocol 8). `a910e18` is `da54149` plus one Linux build fix that touches no file this brief names, so line numbers quoted from either commit agree. Every name below that this brief takes from M6.5, M8a or M8b is checked against that code and listed under "Names as shipped"; everything else is marked new. Every decision the refresh changed says so inline (*Refreshed 2026-09-27:*) and is recorded, with its reason, under "Implementation notes", "Refresh 2026-09-27 (post-M8b)".

The user's design rules, which bind every decision here and are never reinterpreted:

- Only the orchestrator is an interactive PTY window. Workers, reviewers, scouts and sub-planners are headless and watch-only. The user watches them through the conversation view and can never type to them; the daemon refuses input, kill and terminal subscribe for them (`server/headless_guard.rs`).
- TDD applies only where a task needs it. Tasks below say "Tests first" where a test drives the change; a test that pins behaviour already true on `main` is marked as a pinning test, not a red-first one.
- The engine is deterministic. This milestone adds only recorded facts to its model (times, edit records, bounce times), never a decision.
- Headless agents load only the user's own settings. Nothing here changes how any agent is launched.
- There is no repo-level profile file. Nothing here reads or writes one.
- The client's state stays pure (AGENTS.md hard rule 5): `app/`, `tree*`, `graph/`, `inspector*`, `run_edit.rs` and `ui/` perform no I/O and read no clock but `Instant` fields set on message receipt.

## Header

| | |
|--|--|
| Status | `ready` — milestones 8a and 8b are `done`. *(Refreshed 2026-09-27: was `blocked` on 8a.)* |
| Depends on | Milestones 8a and 8b, both merged (8a as PR #17, 8b as PR #19). *(Refreshed 2026-09-27: 8b landed first, so every M8b field this view reads already exists; see "Names as shipped".)* |
| Spec sections | §3 (information flows down through planners, up through the engine), §4 and §4.2 (only the orchestrator is an interactive PTY; every other agent is headless and watched, never addressed), §5.1 (a fast-path run has no plan gate and no orchestrator), §12.3 (the plan gate is shown and edited in the run view), §16.1–§16.5 in full, §21 (the M8c row). Milestone 4.6's spec §4.2–§4.6 and milestone 4.7's spec §3–§5, unchanged except where decisions below extend them. The conversation-view spec §6 and decision 11 (read-only). |
| Branch | `m8c-live-run-view` |
| Protocol version | **9.** Derivation: `pub const PROTO_VERSION: u32 = 8;` at `crates/proto/src/lib.rs:32` on `a910e18` (set by M8b task 2), and `docs/ROADMAP.md:16` already says "milestone 8c will use 9"; 8 + 1 = 9. The test `proto_version_is_eight` (`crates/proto/src/lib.rs:99`, not `run_tests.rs`) becomes `proto_version_is_nine`. Every M8c.1 field is `#[serde(default)]`, so an 8 peer would still decode a 9 snapshot; the bump follows the ROADMAP rule that each run milestone bumps, so an 8 client never shows a 9 daemon's view half-filled. If `main` is not at 8 on the day M8c starts, stop and record it under "Implementation notes" before touching the protocol. *(Refreshed 2026-09-27: was "one above main when this milestone starts".)* |

## Starting point

*Refreshed 2026-09-27.* Names below are real on `a910e18`; line counts are `wc -l` on that commit. The M6.5, M8a and M8b names this brief relies on are listed with their locations under "Names as shipped".

| From | What this milestone uses |
|------|--------------------------|
| M4 / M4.5 (`crates/tui/src/tree.rs`, 459 lines) | `NodeKey { Project(PathBuf), Window(u32), Subagent { window_id, id } }` with the comment `// Milestone 8 adds Run(..)`. `ProjectChild::Window` with `// Milestone 8 adds Run { .. }: run rows sit above plain windows and own their windows`. `RowKind<'a> { Project { root, name, status, counts, collapsed }, Window { info, position, has_subagents, collapsed }, Subagent { info } }` with `// Milestone 8 adds Run { .. }`. `Row<'a> { key, guides, depth, kind }`. `TreeState { collapsed, filter, selected, sidebar, overview, keep_finished_secs, .. }` with `toggle`, `select`, `move_selection`, `repair_selection`, `selected_index`, `prune(&[WindowInfo])`. `build(&[WindowInfo], &TreeState) -> Vec<Row>` (groups by `WindowInfo.project`, rolls status up by `urgency`, `expect`s every project to have a window). `agent_order(&[Row]) -> Vec<u32>`, `row_index`, `urgency`, `subagent_label`, `format_elapsed`. `tree/rows.rs`: `guide_prefix`, `visible_windows`, `SubagentWalk`, `emit_subagents(rows, walk, nodes, ancestors, depth, ancestor_matches)`. `tree/forest.rs`: `subagent_forest(&[SubagentInfo], keep_finished_secs)`. |
| M4.6 (`crates/tui/src/graph/`) | `graph::layout(&[Row]) -> Layout { nodes: Vec<PlacedNode { key, rect, depth }>, edges, size }`, `MIN_NODE_WIDTH` 12, `MAX_NODE_WIDTH` 30, `NODE_HEIGHT` 3, `TIER_GAP` 3, `ROW_GAP` 1, private `BORDERS_AND_PADDING` 4 and `GLYPH_COLUMNS` 2, `pub(crate) fn content_text(&Row) -> String`. `graph/paint.rs` (475 lines): `paint(&Layout, Rect, Pan, &[Row], &App) -> Vec<Line>`, `node_rows`, `interior_slots`, `glyph_and_color`, `border_style`, `is_selected`, the edge painter. `graph/viewport.rs`: `Pan { x, y }` with `clamped`, `revealing`; `GraphGeometry { area, pan }` with `node_at`. |
| M4.7 (`crates/tui/src/inspector.rs`, 300 lines; `inspector/panel.rs`, 344) | `INSPECTOR_HEIGHT` 8, `MIN_INTERIOR_FOR_PANEL` 14, `Field { label: &'static str, value, wrap }`, `Inspection { glyph: Span<'static>, name, fields }`, `inspect(&Row, &App) -> Inspection`, `panel::render(frame, &Inspection, area)` (rounded block, `Padding::horizontal(1)`, title row, column packing by `pack`, the wrapping field). |
| `crates/tui/src/ui/overview.rs` (236) | `areas(main, inspector_visible) -> (canvas, footer)`, `View { canvas, footer, layout, pan }`, `view`, `view_of`, `render` (block title `" tree overview "`), `footer_line`, `footer_parts`. |
| `crates/tui/src/tree_input.rs` (310) | `App::rows` (`:11`, `tree::build(&self.windows, &self.tree)`), `set_graph_viewport`, `reveal_tree_anchor`, `reveal_graph_selection`, `enter_overview`, `enter_tree_navigation`, `exit_tree`, `run_tree_command`, `on_tree_key` (`j k h l`, arrows, `Enter`, `Space`, `i`, `/`, `Esc`), `activate_tree_node`, `select_tree_parent`, `select_first_visible_child`, `toggle_tree_node`, the filter keys. `tree::build(` is called directly at `:12, 102, 209, 218, 237, 252, 287`. |
| `crates/tui/src/app/` | `mod.rs` (537): `Effect { Send, Quit, Bell, Reconnect }`, `PendingAction { Kill, Restart, StopDaemon }`, `Modal { Confirm { message, action }, Help, NewAgent, Remove, ForceRemove, Notice, Rename }`, `TreeInput`, `App` (fields `windows`, `focused`, `tree`, `tree_input`, `overview`, `inspector_visible`, `graph_pan`, `graph_area`, `keymap`, `settings`, `modal`, `spinner_frame`, private `windows_received_at: Instant`, `toast`, M6.5's `conversation`, `conversation_follow` and `utc_offset_secs: i64` (`:143`, set once in `lib.rs:79`)), `focus`, `focused_window`, `on_key`, `run`, `on_paste`, `on_tick`, `age_secs`. **`daemon.rs` (143) holds `on_daemon`**; its `DaemonMsg::Run(_) => vec![]` arm (`:139-140`) is the one M8c.2 replaces. `windows.rs` (167): `is_headless` (`:17`), `focused_pty` (`:24`, excludes headless), `focus_relative` (its expanded order builds with `tree::build(&self.windows, &TreeState::default())` at `:44`), `replace_windows` (`tree::build` again at `:130`). `conversation.rs` (109): `toggle_conversation` (`:12`), `on_conversation_gone` (`:37`), `follow_focus` (`:66`), `sync_conversation_mode` (`:105`). `link.rs` (268): `on_reconnected`, `on_send_failed`, `retry_dropped_subscribe`. `lifecycle.rs` (49): `perform(PendingAction)`, `C-b R`'s restart (an exited window restarts at once, a live one asks). `modal_keys.rs` (303): `on_modal_key`, `confirm_focused` (`C-b x`), `open_remove_confirm` (`C-b X`). |
| `crates/tui/src/mouse.rs` (226) | `on_click` (sidebar rows via `tree::build` at `:142, 179`; the sidebar's `Window \| Subagent` arm calls `self.focus(id)` directly at `:157`, not `activate_tree_node`; `click_graph` selects on a single click, `activate_tree_node` on a double click), `on_drag`, `on_scroll`. |
| `crates/tui/src/keymap.rs` (275), `keymap_tests.rs` (495) | `Keymap::handle` already checks `conversation_mode` before `tree_mode` (`keymap.rs:144-149`), and `ui::draw` draws the conversation before the overview (`ui/mod.rs:84-90`). |
| `crates/tui/src/ui/` | `tree_view.rs`: `narrow_line`, `truncate`, `cut`, `counts_text`. `statusbar.rs` (342): the `TREE` badge and the navigate hint `j/k move  ⏎ focus  space fold  / filter  esc back`. `modal.rs` (162): `render` dispatch. `dialog.rs`: private `LABEL_WIDTH` 11 (`:36`) and `MARKER_WIDTH` 2 (`:38`); M8c.9 makes them `pub(crate)`. `crate::dialog`: `TextInput`, `apply_text_key`. `crate::theme`: `status_glyph`, `status_color`, `subagent_glyph`, `border`, `border_focused`, `muted`, `SPINNER`. `ui/mod.rs` is 99 lines. |
| `crates/tui/Cargo.toml` | The TUI depends on the `daemon` crate (`:15`), so the pure `daemon::run::triage::{kinds_scale, source_label}` and `daemon::manager::control_refusal` are callable from the view without I/O. |
| `scripts/` | `pty-smoke.py` (1797 lines, far over 600), `pty_tree_smoke.py` (the graph stages; `run_project_tree_stage(REPO, PtyProc, run_cmd, fail)` at `pty-smoke.py:1471` is the injection pattern). M8a's `scripts/pty_smoke_run.py`: `run_engine_stage(run_cmd, fail)` (`:115`), `RUN_WAIT = 300.0` (`:23`), `RUN_CMD_TIMEOUT = 240.0` (`:30`), `_git`, `_write_script`, `--unconfined-checks` off macOS (`:133-138`). M8b's `scripts/pty_smoke_adapt.py`: `adapt_stage(run_cmd, fail)` (`:86`). `pty-smoke.py` calls them at `:1715` and `:1716`, then prints `== stage 12: stop the daemon, verify status ==` at `:1718`. No TUI is attached there: stage 11b's client detached at `:1709-1712`. `CONVERSATION_VIEW_TIMEOUT = 45.0` with its derivation at `pty-smoke.py:1090-1102`. |
| M6.5, M8a, M8b | See "Names as shipped". |

## Goal

In `C-b T`, a run appears as one node under its project, above the project's plain windows. Selecting it and pressing `l` or Enter opens the **run view**: the same canvas and the same inspector, rooted at the run's orchestrator. Its tiers read left to right as the spec's information flow — scouts, sub-planners and the tasks the orchestrator planned itself; each sub-planner's tasks; each task's agent rounds (worker sessions, and every review round as its own node, with a worker sent back drawn again as `worker #1 r2`); their sub-agents. Every node carries a live glyph; the critical path has a bold border; selecting a task lights its dependencies and dependents and dims the rest; `f` filters to running work, blocked work or one runtime. The inspector below grows to twelve rows and shows a progress line and the facts behind it for the run, a sub-planner, a task, an agent round or a scout, from the pushed run snapshot and nothing else. Enter on the orchestrator focuses its PTY window, the one place the user types; Enter on any other agent opens its milestone 6.5 conversation, live and read-only — there is no terminal for a headless agent and nothing in the client ever offers input to one. Before a run starts, the run view is its plan gate: `a` approves, `x` rejects, `e` edits a task's route, brief, size and test mode, `d` removes a task, each sending milestone 8a's run requests. A fast-path run (milestone 8b) has no gate: its root is the run itself with its single task under it.

## Scope

In:

- A small protocol addition (M8c.1): the snapshot fields the view reads that M8a does not carry and no later milestone owns, plus the `#[serde(default)]` placeholders for the fields later milestones fill.
- The client's subscription to `RunsSnapshot` and its handling of `RunReply`.
- `NodeKey::{Run, Planner, Scout, Task, AgentRound}` and the matching `RowKind` variants; runs in the project tree and the sidebar; the run-view rows.
- Node content rows, live glyphs, the critical-path border, the dependency highlight, finished-node dimming, the three filters.
- Opening, navigating and leaving the run view; Enter routing to the orchestrator's window or an agent's conversation; the mouse.
- The per-kind run inspector, `RUN_INSPECTOR_HEIGHT`, the one-field-per-row panel layout with a right-aligned title.
- The plan gate: approve, reject, edit, remove.
- No kill, remove or restart dialog for a focused headless window (M8c.6). *(Refreshed 2026-09-27: new; closes a watch-only gap.)*
- Two daemon-side conversation fixes filed for this milestone: a Codex tool call's summary line, and an interrupted turn that stays `Running` (M8c.11). *(Refreshed 2026-09-27: new.)*
- A PTY smoke stage.

Out, each with its owner:

| Out | Owner |
|-----|-------|
| The scout snapshot types, the onboarding scout, the fast path, `RunInfo.usage`, `TaskInfo.diff` | Landed in M8b; consumed here, not added (*Refreshed 2026-09-27*) |
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
| A restored headless window whose `run` does not parse coming back as a PTY window (daemon restore) | M9 (Risks 9) |
| M8b's `phases`, `decider_usage`, `profile_source`, `promote_requested_at` in the inspector | Not shown (decision 31a) |

## Design decisions

Numbered and final. If one proves wrong or impossible, stop work on it, record the evidence under "Implementation notes", and continue with the tasks that do not depend on it. Decisions the 2026-09-27 refresh changed carry a *Refreshed 2026-09-27* line; the full record is under "Implementation notes".

### The data

1. **One pushed snapshot, no polling.** The client sends `ClientMsg::Run(RunRequest::Subscribe)` once per connection — right after `App::new` in `lib.rs`, and again from `on_reconnected` — and keeps the latest `RunReply::Snapshot` in `App.runs`. Every snapshot replaces the previous one whatever its revision: within one connection M8a's watch only ever moves forward, and after a daemon restart the global revision starts again, so a "newer revision only" rule would ignore the restarted daemon forever. A refused send of the subscription is retried by `on_tick`, like M6's dropped `Subscribe`. *(Spec §16.5; M8a decision 47.)*
2. **Time comes from the snapshot.** M8c.1 adds `RunsSnapshot.now` (the daemon's unix seconds at publication). An age is `runs.now − t + runs_received_at.elapsed()`, the pattern `App::age_secs` already uses for windows, so elapsed times tick on the client's clock between pushes, with no clock skew between client and daemon, and the pure view never reads the wall clock. The daemon's time as the client sees it is `App::run_now() = runs.now + runs_received_at.elapsed()` (whole seconds), and `run_age(t)` is `run_now() − t`, saturating. A round counts as rate-limited in the view exactly while `rate_limited_until > run_now()` (`App::rate_limited(&AgentRoundInfo)`); the snapshot's own `rate_limited` flag, evaluated only at publication, is not read. *(Spec §16.5 "Elapsed times … tick on the client's clock". Refreshed 2026-09-27: `run_now` and the rate-limit rule.)*
3. **The view derives nothing the snapshot and the window list do not carry.** Where a mockup value has no source, the field is omitted, or the value is added to the snapshot by M8c.1 when it is cheap and exact, or it is a placeholder a later milestone fills. The inspector and every `ui/` file stay pure (AGENTS.md rule 5). *(Spec §16.4 last paragraph.)*
4. **M8c.1 adds, computed by M8a's pure `run/snapshot.rs` from the engine model:** `RunsSnapshot.now`; `RunInfo.approved_at: Option<u64>`, `plan_edits: Vec<PlanEditInfo { at: u64, text: String }>`, `plan_edits_since_approval`; `TaskInfo.brief`, `acceptance`, `route_spec: RouteSpec`; `TaskInfo.history` becomes `Vec<TaskEventInfo { at: u64, text: String }>` (was `Vec<String>` of daemon-formatted `"<hh:mm> <text>"`); `AgentRoundInfo.rate_limited_since`, `rate_limited_until`, `sent_back_at`. The model gains `Run.approved_at`, `Run.plan_edits: Vec<PlanEditRecord>`, `Run.plan_edits_since_approval` (in `run/model.rs`), and `AgentRound.rate_limited_since`, `AgentRound.sent_back_at` (in `run/model_rounds.rs`), each `#[serde(default)]` so existing `run.json` files load. Every time is raw unix seconds; the daemon formats none of them (decision 31 formats them on the client). How each is recorded:
   - `approved_at = Some(now)` in `engine/requests.rs::approve`, and in `requests.rs::start` for a run that starts already approved (the `--yes` branch and the fast-path branch).
   - An accepted `Edit` batch pushes one `PlanEditRecord { at: now, text: describe(&edits) }` (keep the last 50) and adds 1 to `plan_edits_since_approval` when `approved_at` is set. `PlanEditRecord` is its own struct, not M8a's `TaskEvent`, so M9 can add the edit's `source` with `#[serde(default)]` (tiered-testing spec §12.5). `describe` and the push-and-cap live in a new pure `run/edit_log.rs`, because `run/edits.rs` is 577 lines. `Run.log` already says "applied N plan edits" but mixes every event, so it is not reused.
   - `rate_limited_since` is kept by one helper, `AgentRound::set_rate_limited(until: Option<u64>, now: u64)`: `Some` when `rate_limited_until` goes from `None` to `Some`, unchanged while it stays `Some`, `None` when it is cleared. It is called at **every** site that sets or clears `rate_limited_until`: set at `engine/signals.rs:145` (`ApiRetry`), `:341` (a failed-turn rate limit) and `engine/review.rs:407`; cleared at `signals.rs:121`, `:507` and `review.rs:529`.
   - Rung 1 (`engine/ladder.rs::take_rung`'s last branch, `:197-212`, reached from `gate_failure` for every gate, review included) pushes `now` onto the live worker round's `sent_back_at` **whether or not** it queues the failure text: with `told` (M8a decision 55) the tool reply already carried it, and the session was still sent back.
   - `history` publishes each `TaskEvent`'s own `at` and `text` (newest first, at most 10, as today), and `rate_limited_until` copies the round's model value. Both are raw unix seconds, like every other time here, so the snapshot formats no time at all and `run/snapshot.rs::clock` loses its last caller and is removed.
   - `route_spec` is the plan's own `spec.route`, the unresolved `RouteSpec` whose `None`s mean "policy". The snapshot's `TaskInfo.route` is the resolved `Route`, which cannot show `policy`, and `PlanEdit::AmendTask.route` replaces the whole `spec.route` (`run/edits.rs:446-448`), so the edit form needs the spec to send back (decision 33).
   These are the only engine changes in this milestone; M8c.11's are conversation-building fixes outside the engine. *(Refreshed 2026-09-27: was `approved_hhmm`, `"<hh:mm> <text>"` plan-edit strings and history strings formatted in UTC by `snapshot.rs::clock`, no `rate_limited_until`, `Vec<TaskEvent>`, `describe` in `edits.rs`, one `ApiRetry` site, a rung 1 push only when the text is queued, and no `route_spec`. The raw times and `route_spec` are the refresh's rulings; see "Implementation notes".)*
5. **Later milestones' fields exist before they are filled.** M8b landed first, so `RunInfo.scouts`, `RunInfo.usage`, `TaskInfo.diff`, `ScoutInfo`, `ScoutKind`, `ScoutState`, `DiffStats` and `RunUsage` exist exactly as "Consumes from later milestones" lists them and are **not** re-added. M8c.1 adds only what is still absent — `RunInfo.planners` with `PlannerInfo`/`PlannerState`, `RunInfo.estimate_left_secs`, `RunInfo.bound_ratio_permille` — exactly as listed, `#[serde(default)]`, and nothing populates them; the view renders their absence. *(Refreshed 2026-09-27: was conditional on whether M8b had landed.)*

### Where runs appear

6. **Runs shown in the tree are the non-terminal ones:** every `RunInfo` whose `state.is_terminal()` is false (`RunState::is_terminal`, `proto/src/run.rs:398`: `accepted`, `discarded`, `failed`). `anthrex run status` shows the rest. *(Refreshed 2026-09-27: names the shipped helper.)*
7. **A run is one node under its project**, `NodeKey::Run(run_id)`, depth 1, above the project's plain windows, runs ordered by `created_at` then `run_id`. `RunsSnapshot.runs` arrives newest first (`snapshot.rs:23-28`), so the client sorts. It is a **leaf in the project tree**: its windows are not listed as plain windows, and its tasks appear only in the run view. The project is `RunInfo.project`; a project that has a run and no window still gets its project row (today's `build` `expect`s a window — that invariant goes), and `TreeState`'s pruning keeps that project's `Project(root)` key while a shown run names it (today's `prune`, `tree.rs:196-202`, drops a key whose root has no window). *(Spec §16.1 first paragraph; `tree.rs`'s milestone-8 comments. Refreshed 2026-09-27: the client-side sort and the pruning rule.)*
8. **Which windows a run owns.** A window whose `info.run` is `Some(r)` and whose `r.run_id` names a run shown in the tree (decision 6) is hidden from the plain window list. Every other window — including a headless window of a run the snapshot does not name (no snapshot yet, a run discarded or accepted, a retired reviewer that outlived its run) and a headless window with `run: None` (M8b's onboarding scout, `crates/daemon/src/scout/spec.rs:132-136` sets a `RunRef` only for area scouts) — is listed as a plain window, exactly as today. Area scouts (M9) carry `RunRef { role: Scout, task_id: None, .. }`; they are hidden while their run is shown and drawn only through `RunInfo.scouts`. *(Refreshed 2026-09-27: the run-less headless window.)*
9. **Project status and counts.** A project's rolled-up status is the most urgent (`tree::urgency`) of its plain windows' statuses and its shown runs' statuses, where a run maps to a status: `awaiting_approval`, `paused`, `halted` → `Attention`; `running` → `Attention` when any task is `blocked`, else `Working`; `complete` → `Done`. Runtime counts count plain windows only.
10. **The orchestrator's window and the number keys.** A run whose orchestrator window is listed (a window with `run == Some(RunRef { run_id, role: AgentRole::Orchestrator, .. })`) takes the next tree position, shown on its sidebar row, and `tree::agent_order` yields that window's id at that point, so `C-b <n>`, `C-b j`/`k` and `ensure_focus` reach the orchestrator and nothing else of the run. A run with no orchestrator window (every run until M9, and fast-path runs) has no position.

### The run view

11. **The run view is the overview with a different root.** `App.run_view: Option<RunView { run_id, filter }>`. While `overview` is on and `run_view` is `Some`, `App::nav_rows()` returns `tree::run_rows(..)` instead of `App::rows()`; the canvas, the painter, the inspector, the reveal, the selection keys and the mouse all read `nav_rows()`. The sidebar keeps showing `App::rows()`. The root's key is `NodeKey::Run(run_id)` — the same key as the project-tree node — so the selection needs no translation on the way in or out. *(Spec §16 "extended, not replaced".)*
12. **Tiers.** Root, depth 0: the run (`RowKind::Run`). Depth 1: the run's scouts, then its sub-planners and the tasks no sub-planner owns, interleaved as in decision 13. Depth 2: each sub-planner's tasks (a task belongs to the planner whose `epic` equals the task's `epic`; a task whose `epic` names no planner belongs to the root). Below a task, one tier deeper: its agent rounds. Below a round, a scout or a planner with a listed window: that window's sub-agents, by `tree::subagent_forest` and `emit_subagents`, exactly as under a plain window. The orchestrator window's own sub-agents are not drawn (its conversation shows them). *(Spec §16.1.)*
13. **Order among siblings.** Scouts by `started_at`, then `id`. Tasks under one parent by `wave`, then plan order (their index in `RunInfo.tasks`). A sub-planner sorts among the root's tasks as if it were its earliest task by `(wave, plan index)`; a planner with no task yet sorts after every root task, in `RunInfo.planners` order. Agent rounds by start time, a worker before a reviewer on a tie. *(Spec §16.2 "Order among siblings".)*
14. **Agent rounds.** Each `AgentRoundInfo` of role `Worker` becomes `1 + sent_back_at.len()` nodes: round `n` starts at `started_at` for `n = 1` and at `sent_back_at[n − 2]` after, and ends where the next starts; the last ends at `ended_at`. Worker display rounds are numbered from `sent_back_at` alone; a worker's `AgentRoundInfo.round` is ignored, because it equals its `session` (`engine/dispatch.rs:358-367`). Every reviewer `AgentRoundInfo` is one node, its number the review round (`AgentRoundInfo.round`, equal to its `session`, `engine/review.rs:141-150`). A round of role `Orchestrator` or `Scout` on a task is unreachable; if one appears it is one node labelled `orchestrator #{session}` or `scout #{session}`, so every match on `AgentRole` stays exhaustive. The session's counters (turns, tool calls, tokens) belong to its last display round. The window's sub-agents hang under the last display round only, so no node key repeats. Failures count per task (`count_failure`, `ladder.rs:146-159`): the first gate failure takes rung 1, the second a fresh session (rung 2), so today a session is sent back at most once and `r3` does not occur; the rule still draws any number of bounces. *(Spec §16.1 "Review calls are nodes", "worker #1 r2", "A fresh session … is a new node (worker #2)". Refreshed 2026-09-27: M8a never appends a worker `AgentRoundInfo` per bounce — rung 1 re-sends to the same round (`ladder.rs:197-212`), and a new worker entry is always a new session — so the "appended" shape and its test are gone; M8b added `AgentRole::Scout`.)*
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
23. **Leaving.** In the run view, `Esc`, or `h` with the root selected, closes the run view and returns to the project overview with the `Run` node selected. `Esc` in the project overview leaves tree mode as today. If the snapshot stops naming the run, or names it in a terminal state, the run view closes the same way and toasts `run <id> is gone` or `run <id> is <state text>`. The state text is this view's own mapping, not `RunState::label()` (which is snake_case): `awaiting approval`, `running`, `paused`, `halted`, `complete`, `accepted`, `discarded`, `failed`. *(Spec §16.1. Refreshed 2026-09-27: the state-text mapping.)*
24. **Enter in the run view.** On the root: focus the orchestrator's window and leave tree mode, as Enter on a window does today; with no orchestrator window, toast `run <id> has no orchestrator window; Enter on an agent opens its conversation`. That covers the fast path, whose root is the run itself (spec §16.1: "Fast-path runs have no orchestrator"). On an agent round, a scout or a planner: open its window's conversation (decision 25). On a task: open the conversation of its current round — the live one with the latest start, else the latest round whose window is listed; none → toast `<task> has no agent yet`. On a sub-agent: open its owning window's conversation (M6.5's descent goes further from there). *(Spec §16.4 "Navigation". Refreshed 2026-09-27: the fast path named.)*
25. **Opening a conversation never focuses a window.** `App::open_conversation(window_id)`, in `app/conversation.rs`, does what M6.5's `toggle_conversation` (`app/conversation.rs:12-24`) does when it opens: clear `conversation_follow`, `self.conversation.open(window_id)`, then `self.sync_conversation_mode()` — for any listed window, leaving `focused`, `overview` and `run_view` as they were, so closing the conversation (M6.5's `Esc`/`q`) lands back in the run view on the same node. A window id that is not in `App.windows` toasts `window #<id> is not listed yet` for a live round and `<label> has finished and its window is gone` for an ended one. M6.5's "the view follows focus" rule (`follow_focus`, `:66-87`) is kept as it is: if focus changes while such a conversation is open (`C-b n`, `C-b <n>`, or `replace_windows`' neighbour fallback), the view re-opens on the focused window, exactly as it does for any conversation, and closing it still lands in the run view. A retired worker's `ConversationGone` closes the view with M6.5's toast. `crates/tui/src/conversation.rs` (595 lines) is not touched. *(Spec §4.2, §16.4. Refreshed 2026-09-27: M6.5 shipped `sync_conversation_mode` and `follow_focus`; the old text set the keymap mode directly and did not say what happens on a focus change.)*
26. **Headless windows are never offered input or control.** Every activation path — Enter, a sidebar click, a canvas double click — on a `Window` node whose `kind` is `Headless` opens its conversation instead of focusing it. `C-b x` (kill), `C-b X` (remove) and `C-b R` (restart) on a focused headless window open no dialog and send nothing: they toast the daemon's own refusal text, `daemon::manager::control_refusal(id, run)` (`manager/headless.rs:215`: `window <id> is a headless session of run <run>; only the engine drives it. Use anthrex run cancel to stop it`, or the scout variant for a run-less window). Today they open `Confirm` or `Remove` (`modal_keys.rs:27-39, 64-74`; `lifecycle.rs:29-48`), and the daemon then refuses the send (`server/headless_guard.rs`). M8a decision 49's placeholder pane (reached by `C-b j`/`k`/`<n>` on a plain-listed headless window) stays as it is, and so do M8a.17's no-`Subscribe` and no-`Input` rules; this milestone adds no path that could send `ClientMsg::Input`, `Subscribe`, `Kill`, `Remove` or `Restart` for a headless window. *(User decision: only the orchestrator is interactive; spec §4.2. Refreshed 2026-09-27: the three control commands.)*
27. **Conversation mode wins over tree mode.** In `Keymap::handle`, conversation mode is checked before tree mode, so an M6.5 conversation opened over the run view receives its own keys. This is already true (`keymap.rs:144-149`); `keymap.rs` is not touched, and `conversation_mode_wins_over_tree_mode` pins it (a pinning test, green from the start). *(Refreshed 2026-09-27: M6.5 merged with this order.)*

### The inspector

28. **`RUN_INSPECTOR_HEIGHT` = 12** (a border, the title, nine rows, a border) and `MIN_INTERIOR_FOR_RUN_PANEL = RUN_INSPECTOR_HEIGHT + 6` = 18. While the run view is open, `overview::areas` gives the panel 12 rows when the interior has 18, else milestone 4.7's 8 rows when it has 14, else the single line. The height depends on the view, not on the node, so the canvas never jumps as the selection moves; a sub-agent in the run view gets the tall panel too. The project overview is unchanged (8), including for a `Run` node, which there shows its first five fields. *(Spec §16.4 "Height".)*
29. **Run inspections are laid out one field per row**, the label padded to `RUN_LABEL_WIDTH` = 10 columns, the value truncated with `…` to the rest; fields past the last row are dropped from the end; the one wrapping field (a scout's `question`) wraps under itself, indented 10, onto as many rows as it needs while leaving one row for each field after it. The title carries a right-aligned muted text; when the name, two spaces and the right text do not fit, the right text is dropped and the name truncated. Project, window and sub-agent inspections keep milestone 4.7's column packing unchanged. *(Spec §16.4, which now says run-view nodes show "one labelled field per row" with a 10-column label, as its mockups do. Refreshed 2026-09-27: the spec was amended, so this no longer overrides it; decision 35's first bullet is resolved.)*
30. **Progress lines.** A bar of `█` then `░`, `filled = (width × done + total / 2) / total`: 18 wide for the run, 10 for a planner and for a task's budget. The run's and a planner's counts: `{merged}/{total} merged`, then each non-zero category in the order `working` (preparing, working), `checking` (proof, check), `review`, `merging` (merge_queue), `blocked`, `waiting` (pending, queued), joined with ` · `; cancelled tasks are left out of `total` and appended as `· {n} cancelled`. A task's budget bar uses the larger of its tool-call and minute fractions of `spent_session` against `budget`. *(Spec §16.4 mockups; the mockups' own bars do not match their counts, decision 35.)*
31. **Numbers.** Durations: `< 60` → `{s}s`, `< 3600` → `{m}m`, else `{h}h{mm}m` (`1h12m`, `2h05m`). Tokens: `< 1000` → `{n}`, `< 1 000 000` → `{n/1000}k`, else `{n/1 000 000}.{(n mod 1 000 000)/100 000}M`. Token totals are `TokenUsage::billable()`; the cache share is `cache_read × 100 / (input + cache_read + cache_write)`, rounded down, omitted when the denominator is 0. Commit ids show 7 characters. Wall-clock stamps the snapshot carries as unix seconds (`approved_at`, each `PlanEditInfo.at`, each `TaskEventInfo.at` of a task's `history`) are shown as local `hh:mm` by the pure `inspector::local_hhmm(at, app.utc_offset_secs)`: `((at as i64 + offset).rem_euclid(86_400))`, then hours and minutes, two digits each. `App.utc_offset_secs` is the plain field M6.5 set once at startup (`lib.rs:79`), so no clock is read in the view. Every time the view shows is therefore local. *(Refreshed 2026-09-27: new paragraph; the daemon formatted `approved_hhmm`, the edit stamps and the task history in UTC.)*
31a. **What this view shows of M8b's fields.** *(Refreshed 2026-09-27: new decision.)* M8b's "Produces for later milestones" promised ten fields to this view; it shows these, and only these:
   - **Run, `gate` row, a fast-path run** (`RunInfo.path == Some(RunPath::Fast)`): `fast path · no plan gate`, then ` · triage: {kinds_scale(kinds, scale)} ({source_label(source)})` from `RunInfo.triage` when set, using the pure `daemon::run::triage::{kinds_scale, source_label}` that `anthrex run status` uses (`cli/src/run_cmd/status.rs:4`).
   - **Task, `fixing` row, a failed check**: `check failed: {first line of CheckInfo.decider_summary}` when that is set, else the last line of `summary`, as before.
   - **Task, `tries` row**: ` · size raised {engine}→{decided}` when `TaskInfo.size_check` is `Some` with `agreed == false` and `decided == Some(_)`.
   - **Task, stage text**: `blocked: {reason} (fallback)` when `block_source == Some(DeciderSource::Fallback)`.
   Not shown: `phases` (M9.5 uses them), `decider_usage` (already counted in `RunInfo.usage.by_role["decider"]`), `profile_source` and `promote_requested_at` (their lines already arrive in `RunInfo.attention`, `snapshot.rs:154-155`), `CheckInfo.summary_source`.

### The plan gate

32. **The gate is the run view of a run in `awaiting_approval`.** There, and only there, `a` asks `Approve run <id>? <n> tasks start.` (`1 task starts.` for one) and on `y` sends `RunRequest::Approve`; `x` asks `Reject run <id>? Its branches and worktrees are removed; salvage refs are kept.` and on `y` sends `RunRequest::Reject`; `d` on a task asks `Remove <task> from run <id>'s plan?` and on `y` sends `RunRequest::Edit` with `PlanEdit::CancelTask`; `e` on a task opens the task edit form. Elsewhere those four keys toast `the plan gate is closed: run <id> is <state text>`, or, on a fast-path run, `the plan gate is closed: run <id> is on the fast path`; `e` or `d` on a non-task node toasts `select a task to edit or remove`. Adding a task is not offered (and the engine refuses additions to a fast-path run). *(Spec §12.3; conversation-view decision 13. Refreshed 2026-09-27: a fast-path run starts `running` with `approved_by = "fast path"`, `run::triage::FAST_APPROVED_BY`, and has no gate.)*
33. **The edit form** edits route (runtime, model, strength, effort), size (`S` or `M`), test mode and its reason, and brief. The route fields open from `TaskInfo.route_spec` (decision 4), not the resolved `route`: a field whose spec value is `None` shows `policy`, followed by the resolved value from `TaskInfo.route` in muted text, so the user sees what the policy chose without pinning it. The form sends one `PlanEdit::AmendTask` carrying only what changed. For the route that means: when no route field changed, `route: None`; otherwise `route: Some(spec)` where `spec` is `route_spec` with only the fields the user changed replaced — every untouched field keeps its spec value, `None` included, because `AmendTask.route` replaces the whole spec route (`run/edits.rs:446-448`). Nothing changed closes the form with the toast `nothing changed`. Changing the runtime clears the model field (the spec's `model` becomes `None`). A mode other than `tdd` needs a non-blank reason (`a reason is required when test mode is check or none`), mirroring M8a decision 10; every other rule is the engine's, whose `Refused` message is shown inline in the form. The size opens from `TaskInfo.size`, which M8b's size cross-check may already have raised; the engine refuses an amend below that floor, inline. The brief is one line; a newline in it is shown as `↵` and restored on submit; `Ctrl-J` inserts one. *(Spec §12.3 "may edit route, brief, size, test mode, and remove tasks". Refreshed 2026-09-27: the snapshot carried only the resolved `Route`, so the form could neither show `policy` nor avoid pinning every resolved value into the plan.)*
34. **Replies.** `RunReply::Done { message, .. }` toasts `message` (and closes a submitting edit form when `request` is `run edit`); `Refused { request, message }` fills a submitting edit form's error when `request` is `run edit`, and toasts `message` otherwise. A `run edit` refusal can be several lines (`engine/requests.rs:296-298` joins the batch's errors with `\n`): the form's error row shows the first line, then ` (+{n} more)`, and a toast of a multi-line refusal shows the same. `Started`, `ConfirmNeeded`, `ToolResult` and M8b's `Triaged`, `Profile` and `Stats` are ignored. The success texts are the engine's: `run <id> approved`, `run <id> rejected; discarding it`, `applied <n> edit(s)` (`requests.rs:167, 255, 346-358`). *(Refreshed 2026-09-27: M8b's three variants; multi-line refusals.)*

### Where the spec's text and its mockups disagree

35. **Resolved here, so nobody has to ask.** Each is also a finding for the spec's author.
    - ~~§16.4 says the inspector "keeps milestone 4.7's layout rules (labelled fields packed in columns)", but every §16.4 mockup is one field per row.~~ Resolved in the spec: §16.4 now specifies one labelled field per row with a 10-column label (decision 29). *(Refreshed 2026-09-27.)*
    - The §16.4 progress bars do not match their own counts: the run's shows 11 of 18 cells for 5/9 (exactly 10), the planner's 6 of 10 for 1/3 (3.3). Decision 30's formula wins, so the exact tests show 10 and 3 cells.
    - The §16.1 diagram draws the root at the top of the canvas and `t6` (wave 0) after planner `A`; milestone 4.6's layout centres a parent on its children, and §16.2 orders by wave. The layout and decision 13 win; the diagram is illustrative.
    - The §16.1 diagram labels a scout `scout S1` and a planner `planner A dmn`, while the §16.3 table says a scout shows its question and a planner its area. The table wins for scouts; a planner shows its epic and title (its area is a glob list that does not fit a box) and the inspector shows the area.
    - §16.2's `AgentRound { task, role, session, round }` cannot tell two runs' `t1` apart; decision 15 adds `run`.
    - §16.3's glyph list has `✗ rejected or failed` for tasks, but no M8a task state is "failed" (a failed gate sends the task back to `working` or to `blocked`). `✗` is used for rounds and gate marks; a task that failed is `⊘` when blocked.
    - §16.4's worker mockup shows `editing crates/daemon/src/status.rs`, a tool's target, which no snapshot or window field carries; `doing` shows `last tool: <name>` (Risks 6).
    - §16.4's worker mockup counts `3 commits` per round; nothing in M8a or M8b counts commits per session, and counting them would put git calls behind every snapshot. `activity` leaves them out.
    - §16.4's "derives nothing that the snapshot does not carry" and §16.5's "elapsed times tick on the client's clock" need data M8a's snapshot lacks (the daemon's clock, approval time, edit log, rate-limit start and end, worker bounces, briefs and route specs for the edit form, raw history times); M8c.1 adds exactly those.
    - §16.4's navigation assumes every run has an orchestrator window; until M9, and on the fast path, none has. Decision 24 says what Enter does then.
    - §16.4's task mockup writes `red a1b2c3` (6 characters); the project writes commit ids with 7 (`halted_reason`, `run status`), and so does this view.

## Interfaces

### `proto` (M8c.1)

*Refreshed 2026-09-27.* Added to the shipped types, every field `#[serde(default)]`. `crates/proto/src/run_info.rs` (267 lines):

```rust
pub struct RunsSnapshot { /* revision, runs */ pub now: u64 }
pub struct RunInfo {
    /* shipped fields, M8b's `path`, `triage`, `usage`, `scouts` … included */
    pub approved_at: Option<u64>,             // unix seconds; when approved (user, --yes or the fast path)
    pub plan_edits: Vec<PlanEditInfo>,        // newest first, at most 10
    pub plan_edits_since_approval: u32,
    // placeholders for later milestones ("Consumes from later milestones"):
    pub planners: Vec<PlannerInfo>,           // M9
    pub estimate_left_secs: Option<u64>,      // M9.5
    pub bound_ratio_permille: Option<u32>,    // M9.5
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEditInfo { pub at: u64, pub text: String }   // text: `describe`'s, e.g. "split t2"
pub struct TaskInfo { /* shipped fields, M8b's `diff`, `size_check`, `block_source` … included */
                      pub brief: String, pub acceptance: Vec<String>,
                      pub route_spec: RouteSpec,              // the plan's own `spec.route`; `None`s mean policy
                      pub history: Vec<TaskEventInfo> }       // changed type: was Vec<String> ("<hh:mm> <text>", UTC)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskEventInfo { pub at: u64, pub text: String }  // newest first, at most 10
pub struct AgentRoundInfo { /* shipped fields */ pub rate_limited_since: Option<u64>,
                            pub rate_limited_until: Option<u64>, pub sent_back_at: Vec<u64> }
```

New `crates/proto/src/planner.rs`, beside M8b's `scout.rs`:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")] pub enum PlannerState { #[default] Planning, Finished, Failed }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerInfo { pub epic: String, pub title: String, pub area: Vec<String>, pub route: Route,
                         pub window_id: Option<u32>, pub state: PlannerState, pub started_at: u64,
                         pub ended_at: Option<u64>, pub edits_accepted: u32, pub edits_rejected: u32,
                         pub last_rejection: Option<String>, pub replans: Vec<String> }
```

Not added, because M8b shipped them exactly as this brief listed them: `RunInfo.scouts`, `RunInfo.usage` (always `Some` from an M8b daemon, `snapshot.rs:85`), `TaskInfo.diff`, `ScoutInfo`/`ScoutKind`/`ScoutState` (`proto/src/scout.rs:11-76`), `DiffStats` and `RunUsage` (`proto/src/adapt.rs:70-99`, not `run_info.rs`).

`TaskInfo.history` keeps its name and `#[serde(default)]`; only its element type changes, which the protocol bump covers. `anthrex run status --json` prints `RunInfo`, so its history entries become `{"at": …, "text": …}` objects; the smoke stages read only `state` and task `id`/`state` from it, and no CLI code reads `TaskInfo.history` (the report's history comes from the model, `report_task.rs:114`). The one proto fixture that builds a history string (`run_tests_fixtures.rs:155`) and `cli/src/run_cmd/status_tests.rs:98` are updated.

`lib.rs` re-exports `PlanEditInfo`, `TaskEventInfo`, `PlannerInfo` and `PlannerState` by name (never by glob) and sets `PROTO_VERSION` to 9, with a derivation paragraph in its doc comment in the M8a/M8b style (`lib.rs:19-31`).

### `daemon` (M8c.1)

*Refreshed 2026-09-27.*

```rust
// run/model.rs (563) — each #[serde(default)], one doc line each
pub struct Run { /* shipped */ pub approved_at: Option<u64>, pub plan_edits: Vec<PlanEditRecord>, pub plan_edits_since_approval: u32 }
// run/model_rounds.rs (219) — each #[serde(default)]
pub struct AgentRound { /* shipped */ pub rate_limited_since: Option<u64>, pub sent_back_at: Vec<u64> }
impl AgentRound { pub fn set_rate_limited(&mut self, until: Option<u64>, now: u64); } // sets rate_limited_until and keeps rate_limited_since
// run/edit_log.rs (new, pure)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEditRecord { pub at: u64, pub text: String }   // M9 may add `#[serde(default)] source`
pub const PLAN_EDITS_KEPT: usize = 50;
pub fn describe(edits: &[PlanEdit]) -> String;
pub fn record(run: &mut Run, edits: &[PlanEdit], now: u64);   // push, cap at PLAN_EDITS_KEPT, count after approval
```

- `approved_at = Some(now)` in `engine/requests.rs::approve` (`:136-168`) when it moves the run out of `awaiting_approval`, and in `requests.rs::start` (`:55-112`) in its fast-path branch and its `--yes` branch.
- `requests.rs::edit` (`:264-361`) calls `edit_log::record(run, &edits, now)` once, after `apply_edits` accepts the batch; a refused batch records nothing.
- `describe` joins, with `, `: `add <id>`, `split <id>`, `cancel <id>`, `amend <id>`, `dep <id> on <dep>`, `answer <id>`, `pause`, `resume`, `finish` — one per `PlanEdit` variant; the match is exhaustive, so a variant M9 adds fails to compile until it is described.
- Every assignment to `rate_limited_until` in the engine goes through `AgentRound::set_rate_limited`: `engine/signals.rs:121, 145, 341, 507` and `engine/review.rs:407, 529`. `rg -n "rate_limited_until =" crates/daemon/src/run/engine` then prints nothing.
- `engine/ladder.rs::take_rung`'s rung 1 branch pushes `now` onto the live worker round's `sent_back_at` (the `worker_round(task)` it already finds), before and independent of the `if !told` queueing.
- `run/snapshot.rs` fills `now` (the `now` it is already called with), `approved_at`, `plan_edits` (newest first, at most 10, `PlanEditInfo { at, text }` copied from the records) and `plan_edits_since_approval`, copies `brief`, `acceptance` and `route_spec` from the task's `spec`, builds `history` as `TaskEventInfo { at, text }` from the task's events (newest first, `HISTORY_SHOWN` = 10, as today), and copies `rate_limited_since`, `rate_limited_until` and `sent_back_at` from each round. It formats no time; the private `clock` (`:191-194`) is deleted.

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
pub fn round_label(role: AgentRole, session: u32, number: u32) -> String;        // "worker #1", "worker #1 r2", "review #2"; exhaustive over AgentRole
impl TreeState { pub fn prune_runs(&mut self, runs: &[RunInfo]); }  // keeps run-view keys of shown runs, and Project keys a shown run names
```

`TreeState::toggle` returns `true` for every new key; `TreeState::prune` keeps every new key (run keys are `prune_runs`'s), and a `Project(root)` key survives when either a window or a shown run names the root (decision 7). `shown_runs` sorts by `(created_at, run_id)`, since the snapshot arrives newest first. `agent_order` yields a `Run` row's orchestrator id when it has one (decision 10). New files: `tree/runs.rs` (grouping, `shown_runs`, the run status of decision 9, hidden windows), `tree/run_rows.rs` (`run_rows`, ordering, `display_rounds`, filters). *(Refreshed 2026-09-27: the sort, the `Project` key rule and the exhaustive label.)*

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

The new glyphs (`◉ ▫ ◇ ◐ ▸ ⊘ ⇠`) join the TUI-wide ASCII-mode follow-up (followups file, "the TUI-wide ASCII spinner" and "sidebar box glyphs in ASCII mode"): whoever fixes ASCII mode covers them too. This milestone does not add an ASCII mode. *(Refreshed 2026-09-27: note added.)*

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
pub fn local_hhmm(at: u64, utc_offset_secs: i64) -> String;   // decision 31
pub fn progress_bar(done: u64, total: u64, width: usize) -> String;   // decision 30

// crates/tui/src/ui/overview.rs
pub fn areas(main: Rect, inspector_visible: bool, run_view: bool) -> (Rect, Rect);  // decision 28
```

`Inspection` gains `impl Default` (`Span` is `Default`), so every existing `Inspection { .. }` construction — three in `inspector.rs` (`:67, 72, 77`) and two in `inspector/panel/tests.rs` (`:22, 159`, a 584-line file) — gains only `..Default::default()`, which means `right: None, layout: FieldLayout::Columns`. `inspector/panel.rs` dispatches on `layout`; the `Rows` renderer lives in `inspector/panel/rows.rs`. *(Refreshed 2026-09-27: `Default`, and `local_hhmm`.)*

### `App` (`crates/tui/src/app/runs.rs`, new)

`App` gains four fields, declared in `app/mod.rs`: `pub runs: proto::RunsSnapshot` (default: revision 0, no runs), `pub run_view: Option<RunView>`, `runs_received_at: Instant`, `run_subscribed: bool`. The `DaemonMsg::Run(reply) => self.on_run_reply(reply)` arm replaces M8a's ignore arm in **`app/daemon.rs:139-140`**. `App::rows` stays in `tree_input.rs` (`:11`) and calls `build_with_runs`. *(Refreshed 2026-09-27: `on_daemon` lives in `app/daemon.rs`; `open_conversation` lives in `app/conversation.rs`; the headless control guard is new.)*

```rust
pub struct RunView { pub run_id: String, pub filter: RunFilter }
impl App {
    pub fn run_subscription(&mut self) -> Effect;                 // Send(Run(Subscribe)); sets run_subscribed
    pub(crate) fn on_run_reply(&mut self, reply: RunReply) -> Vec<Effect>;   // exhaustive over RunReply
    pub fn run_now(&self) -> u64;                                 // decision 2: runs.now + whole seconds since receipt
    pub fn run_age(&self, unix_secs: u64) -> u64;                 // decision 2: run_now() − t, saturating
    pub fn rate_limited(&self, round: &AgentRoundInfo) -> bool;   // decision 2: rate_limited_until > run_now()
    pub fn nav_rows(&self) -> Vec<tree::Row<'_>>;                 // decision 11
    pub(crate) fn open_run_view(&mut self, run_id: String);
    pub(crate) fn close_run_view(&mut self);
    pub(crate) fn on_run_view_key(&mut self, key: KeyEvent) -> Option<Vec<Effect>>; // a x e d f h; None = not handled
    pub(crate) fn on_edit_task_key(&mut self, key: KeyEvent) -> Vec<Effect>;
}
// crates/tui/src/app/conversation.rs (109 lines)
impl App {
    pub(crate) fn open_conversation(&mut self, window_id: u32) -> Vec<Effect>;   // decision 25
}
// crates/tui/src/app/modal_keys.rs / lifecycle.rs
impl App {
    /// Decision 26: `Some(toast text)` when the focused window is headless, so `C-b x`,
    /// `C-b X` and `C-b R` toast it and open nothing; `None` otherwise.
    fn headless_control_refusal(&self) -> Option<String>;   // daemon::manager::control_refusal(id, run.as_ref())
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
    // route fields hold the plan's RouteSpec values: None is "policy" (decision 33)
    pub runtime: Option<Runtime>, pub model: TextInput, pub strength: Option<Strength>, pub effort: Option<Effort>,
    pub size: Size, pub test_mode: TestMode, pub reason: TextInput, pub brief: TextInput,
    pub focus: EditField, pub error: Option<String>, pub submitting: bool,
    resolved: Route,                        // TaskInfo.route, shown muted beside a `policy` value
    original: TaskInfoValues,               // route_spec, size, test_mode, brief when the form opened
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

Keys: `Tab`/`Down` next field, `BackTab`/`Up` previous; on `Runtime`, `Strength`, `Effort`, `Size`, `TestMode`: `Left`/`Right`/`Space` cycle (`policy → claude → codex`; `policy → fast → standard → frontier`; `policy → low → medium → high`; `S ↔ M`; `tdd → check → none`); on text fields, `apply_text_key`; `Ctrl-J` in `Brief` inserts `↵`; `Enter` submits; `Esc` or `Ctrl-C` cancels. The box is 72 columns wide, titled ` edit <task> `, one row per visible field (`LABEL_WIDTH` 11, `MARKER_WIDTH` 2 — made `pub(crate)` in `ui/dialog.rs` — `› ` on the focused row), choice values written `‹ value ›`, a `None` route value written `‹ policy ›` followed by two spaces and the resolved value in `theme::muted` (`‹ policy ›  standard`), an empty model written `policy` the same way, then a blank row, the error row when there is one (the first line of the refusal, then ` (+{n} more)`), and the hint row `⏎ save  tab next  ←/→ change  esc cancel`. *(Refreshed 2026-09-27: runtime and effort gain `policy`, and a `policy` value shows what the policy resolved.)*

### Status bar and overview title

- Run view, `TreeInput::Navigate`, gate open: `a approve  x reject  e edit  d remove  ⏎ open  f filter: <label>  esc back`.
- Run view, `Navigate`, otherwise: `j/k move  h/l tier  ⏎ open  space fold  f filter: <label>  / find  esc back`.
- The project overview's hints are unchanged.
- The overview block's title is ` run <run_id> ` in the run view, ` tree overview ` otherwise.

### Inspector contents, exact

Values below are the "Gemini fixture" (task M8c.7): run `r1`, goal `Add Gemini runtime`, `created_at = now − 4320`; tasks `t0 t1 t4 t6 t8` merged, `t3 t7` working, `t2` in review, `t5` blocked (`question`, `Gemini has no subagent-stop event`); `critical_path = [t0, t6, t2, t3]`; writers `3/3`, readers `1/3`; a live Codex round rate-limited since `now − 240` until `now + 60` (`rate_limited_since`, `rate_limited_until`); token usage `input 200 000, cache_read 710 000, cache_write 90 000, output 1 510 000`; `spent_total.tool_calls` summing to 612; `estimate_left_secs = 2400`; `bound_ratio_permille = 1200`; `approved_at = D + 39_720` (11:02 UTC, `D` a whole number of days in unix seconds), `plan_edits = [PlanEditInfo { at: D + 42_000, text: "split t2" }, PlanEditInfo { at: D + 40_800, text: "amend t4" }]`, `plan_edits_since_approval = 2`; the `App` the tests build has `utc_offset_secs = 0`. *(Refreshed 2026-09-27: raw times instead of `"11:02"` strings.)*

**Run (orchestrator).** Glyph `◉`; name `{run_id}  {goal}`; right `{state text} · {format_duration(run_age(created_at))}`, state text `awaiting approval`, `running`, `paused (from <paused_from>)`, `halted`, `complete`. Fields, in order, each omitted when its condition says so:

| Label | Value |
|---|---|
| `progress` | decision 30 at 18; `no tasks yet` when total is 0 |
| `path` | `critical path {ids joined " → "} · {n} tasks left` (`1 task left`) + ` · {p/1000}.{p%1000/100}× the bound` when `bound_ratio_permille` is `Some(p)`; omitted when `critical_path` is empty |
| `agents` | `workers {writers_busy}/{max_writers} · readers {readers_busy}/{max_readers}`, then for each runtime used by a task route or review route, Claude first: ` · {runtime} ok` or ` · {runtime} rate-limited {format_duration(age of the earliest rate_limited_since among its live rate-limited rounds)}` (`rate-limited` alone when no `since`) |
| `spend` | `tokens {format_tokens(t)}`, `t` being `usage.total.billable()` when `usage` is set (every role, the orchestrator included) and the sum of every round's `usage.billable()` otherwise, the cache share over the same usage, + ` (cache {p}%)` + ` · tool calls {Σ spent_total.tool_calls}` + ` · est. left ~{format_duration(e)}` when `estimate_left_secs` is `Some(e)` |
| `gate` | gate open: `awaiting approval · a approve · x reject · e edit · d remove`; a fast-path run (`path == Some(Fast)`): decision 31a's `fast path · no plan gate · triage: …`; approved (`approved_by` set): `plan approved` (`plan approved by --yes` when `approved_by` is `--yes`) + ` {local_hhmm(approved_at)}` when `approved_at` is set + ` · {n} plan edits since` (`1 plan edit since`) when `n > 0` + ` · last: {plan_edits[0].text}` when there is one; otherwise `plan not approved`. *(Refreshed 2026-09-27: the fast path; local time from a raw stamp.)* |
| `attention` | `halted: {halted_reason}` when halted; else the first blocked task in plan order as `{id} blocked: {reason} — "{text}"` (reasons `mis-sized`, `human`, `conflict`, `dependency cancelled`, `question`, `environment`); else the first `attention` line that is not a blocked-task line; then ` · +{n} more`, `n` counting the other blocked tasks plus the `attention` lines that are not blocked-task lines (`RunInfo.attention` already carries one `"{id} blocked ({reason}): {text}"` line per blocked task, `snapshot.rs:127-141`, which must not be counted twice); omitted when there is nothing. *(Refreshed 2026-09-27: the double count.)* |

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

**Task.** Glyph per decision 19; name `{id}  {title}`; right `{size} · {test_mode} · {stage}`, stage `planned` (gate open), `waiting`, `queued`, `preparing`, `working`, `test proof`, `check`, `review round {reviews.len()}`, `merge queue`, `merged`, `blocked: {reason}` (+ ` (fallback)` when `block_source == Some(Fallback)`, decision 31a), `cancelled`. Fields:

| Label | Value |
|---|---|
| `stages` | `done {d} → proof {p} → check {c} → review {r} → merge {m}`. `d`: `●` while `working`, `✓` when `done_signal` is set, else `·`. `p`: `–` unless `tdd`; `●` in `proof`; else `last_proof` `ok` → `✓`, not ok → `✗`, none → `·`. `c`: `–` when the run is `unverified`; `●` in `check`; else `last_check` likewise. `r`: `–` when `review_route` is `None`; `●` in `review`; else the last review with a verdict: blocking → `✗`, otherwise `✓`; none → `·`. `m`: `✓` merged, `●` in `merge_queue`, else `·`. |
| `route` | `{runtime} · {strength} · {effort} effort` + `  →  reviewer {runtime} · {strength}` when `review_route` is set |
| `deps` | `waits on {id glyph …}` over `deps` then `implicit_deps` (deduplicated; glyph `✓` when merged, else the task glyph) + ` · unblocks {ids joined ", "}` + ` · on critical path`, each part present when non-empty; omitted when all three are |
| `budget` | `{bar at 10} {tool_calls}/{budget.tool_calls} tool calls · {secs/60}/{budget.minutes} min · {format_tokens(tokens)} tokens`, from `spent_session`, `/{format_tokens(budget.tokens)}` after the tokens when a token budget is set |
| `tries` | `review {b.review}/{max_bounces} bounces · check {b.check}/{max_bounces}` + ` · proof {n}/{max}` and ` · merge {n}/{max}` when non-zero + ` · stalls {n}` when non-zero + ` · escalation step {rung}` when `rung > 0` + ` · size raised {engine}→{decided}` when `size_check` disagreed (decision 31a) |
| `diff` | `{files} files · +{added} −{removed}` when `diff` is set, then ` · test `{test}` red {red7} {proof mark}` when `test` and `red` are set (the mark `✓`/`✗` from `last_proof`, omitted when none); omitted when both are absent. `diff` is set only once the task has merged, or ended cancelled or unfinished with a recorded head, on a run with history on (`engine/history.rs:65-90`, M8b decision 32); while a task is live the row shows only the test part. The mockup's `t2` carries a `diff` because this is a pure rendering test. *(Refreshed 2026-09-27: when `diff` is set.)* |
| `review` | the last review with a verdict: `r{round} {✓ or ✗} {counts}: {file name}:{line} "{text}"` with counts the non-zero severities `{n} critical, {n} important, {n} minor` and the most severe finding in full (`{input}` in place of `file:line` when it has no file); `r{round} ✓ no findings` when it has none; omitted when no review has a verdict |
| `history` | each entry as `{local_hhmm(at)} {text}`, joined ` · ` (newest first, as the snapshot gives it); omitted when empty. *(Refreshed 2026-09-27: local time from raw `at`.)* |

Rendered at 86 × 12 (`task_panel_matches_the_mockup`), task `t2` of the Gemini fixture — `map Gemini hook events to status`, `M`, `tdd`, `review`, deps `t0 t6` merged, dependents `t3 t7`, on the critical path, route Codex standard high, reviewer Claude frontier, budget 150/60, `spent_session` 104 tool calls, 2280 s, 410 000 tokens, bounces review 1, rung 1, `max_bounces` 2, `diff` 4 files +212 −31, test `status::gemini_stop_marks_idle`, red `a1b2c3d9…`, proof ok, round 1 `changes` with one critical finding `crates/daemon/src/status.rs:118 "SubagentStop not paired"` and two minor, round 2 without a verdict, history `[{ at: D + 45_060, text: "review r1 changes" }, { at: D + 44_400, text: "check passed" }, { at: D + 43_320, text: "started" }]` (12:31, 12:20, 12:02 UTC; `utc_offset_secs = 0`):

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

**Agent round.** Glyph per decision 19; name `{round_label}  {runtime} · {strength} · {effort}`; right `{status} · {duration} · {task id}` where status is `rate-limited` when rate-limited, else the window's `Status::label()` while the round is live (`starting` when the window is not listed), `finished` once ended, and duration is the age since the display round's start while live, `ended − start` after. Worker fields: `doing` (live: `rate-limited · waiting out the runtime's retry` when rate-limited, `last tool: {tool}` when the window has one, `thinking` when a turn is open, `waiting for its next turn` otherwise, then ` · {n} sub-agents open` when `open_subagents > 0`; ended: `finished`); `activity` (on the session's last display round only: `turns {turns} · tool calls {tool_calls}` + ` · tokens {format_tokens(billable)}` + ` · {denials} denied` when non-zero); `fixing` (display rounds after the first of a session, or any round of session > 1: the latest gate failure at or before the round's start — the most severe critical or important finding of a blocking review, timed by its reviewer round's `ended_at`, as `{file name}:{line} {severity} — {text}`; a failed `last_check` as `check failed: {first line of decider_summary}` when M8b's decider summarised it, else `check failed: {last line of summary}` (decision 31a); a failed `last_proof` as `test proof failed`; omitted when none is found); `session`. Reviewer fields: `judging` (the latest worker display round started before this round: `{label} · {runtime} · {strength}`); `strength` (`{own} vs author {author's}`); `verdict` (`reviewing`, `approve`, `changes (blocking)`, `changes (minor only, counts as approval)`, or `none — the round ended without one`); `findings` (`{counts} · {most severe as in fixing}`, `none` when empty); `session`. `session` is `no window yet`; `#{id} · window closed` when the id is not listed; else `#{id} · {headless or terminal} · worktree {task.branch} · Enter: conversation` for a worker, `#{id} · {headless or terminal} · read-only review worktree · Enter: conversation` for a reviewer.

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

AGENTS.md rule 8 puts the limit at about 600 lines. *Refreshed 2026-09-27:* counts are `wc -l` on `a910e18`; every budget keeps its file under 600.

| File | Lines | Budget in this milestone |
|------|------:|------|
| `crates/tui/src/app/mod.rs` | 537 | ≤ 552: four `App` fields and their initialisers, `mod runs;`, one `Modal::EditTask` variant, one `on_paste` arm. Everything else in `app/runs.rs`. |
| `crates/tui/src/app/daemon.rs` | 143 | ≤ 150: the `DaemonMsg::Run(reply) => self.on_run_reply(reply)` arm replacing M8a's ignore arm (`:139-140`). |
| `crates/tui/src/app/conversation.rs` | 109 | ≤ 140: `open_conversation`. |
| `crates/tui/src/conversation.rs` | 595 | **Untouched**: `open_conversation` uses its public API only. |
| `crates/tui/src/app/windows.rs` | 167 | ≤ 180: its two `tree::build` calls become `build_with_runs` (`:44` with `TreeState::default()`) and `self.rows()` (`:130`). |
| `crates/tui/src/app/modal_keys.rs`, `app/lifecycle.rs`, `ui/modal.rs` | 303, 49, 162 | `modal_keys.rs` ≤ 325 (the `EditTask` dispatch arm, the headless guard in `confirm_focused` and `open_remove_confirm`); `lifecycle.rs` ≤ 70 (three `perform` arms, the headless guard on restart); `ui/modal.rs` one render arm. |
| `crates/tui/src/app/tests.rs` | 533 | ≤ 540: `#[path]` declarations only. |
| `crates/tui/src/keymap.rs` | 275 | **Untouched** (decision 27 already holds). |
| `crates/tui/src/keymap_tests.rs` | 495 | ≤ 510: one pinning test. |
| `crates/tui/src/lib.rs` | 445 | ≤ 455: the subscription send after `App::new`, `pub mod run_edit;`. |
| `crates/tui/src/tree.rs` | 459 | ≤ 560: the enum variants, `build` delegating to `build_with_runs`, `agent_order`, `toggle`/`prune` arms. Grouping in `tree/runs.rs`, run rows in `tree/run_rows.rs`. |
| `crates/tui/src/tree/tests.rs` | 562 | ≤ 570: `mod` lines only. |
| `crates/tui/src/tree/tests/state.rs` | 581 | **Untouched**: `prune_runs` tests go in `tree/tests/runs.rs`. |
| `crates/tui/src/graph/paint.rs` | 475 | ≤ 520: the calls into `graph/paint/style.rs` (new), which holds glyphs, borders, dimming and the highlight. |
| `crates/tui/src/graph/mod.rs` | 234 | `content_text` arms call `graph/run_text.rs` (new). |
| `crates/tui/src/inspector.rs` | 300 | ≤ 360: the constants, `FieldLayout`, `impl Default for Inspection`, `inspect`'s new arms. Projections in `inspector/run.rs` and `inspector/run_task.rs`, each ≤ 450. |
| `crates/tui/src/inspector/tests.rs` | 561 | **Untouched**: new tests go in `inspector/run_tests.rs`. |
| `crates/tui/src/inspector/panel.rs` | 344 | ≤ 370: the dispatch; the rows renderer in `inspector/panel/rows.rs`. |
| `crates/tui/src/inspector/panel/tests.rs` | 584 | ≤ 590: the two `Inspection` literals gain `..Default::default()` and nothing else. |
| `crates/tui/src/tree_input.rs` | 310 | ≤ 400: `App::rows` calls `build_with_runs`; `nav_rows` replaces its direct `tree::build` calls; run-view keys go to `App::on_run_view_key` in `app/runs.rs`. |
| `crates/tui/src/ui/overview.rs` | 236 | ≤ 320. |
| `crates/tui/src/ui/statusbar.rs` | 342 | ≤ 380. |
| `crates/tui/src/ui/dialog.rs` | — | Two `const`s become `pub(crate)`; no line added. |
| `crates/tui/src/dialog.rs` | 545 | **Untouched**; the form is `run_edit.rs`. |
| `crates/tui/src/mouse.rs` | 226 | ≤ 260. |
| `crates/proto/src/run_info.rs` | 267 | ≤ 330: the new fields and `PlanEditInfo`. `PlannerInfo`/`PlannerState` go in `proto/src/planner.rs` (new). |
| `crates/proto/src/lib.rs` | 107 | ≤ 120: the derivation paragraph, the re-exports, the renamed version test. |
| `crates/proto/src/run_tests.rs` | 463 | ≤ 540; if the new tests would pass it, they go in a new `run_tests_view.rs`. |
| `crates/proto/src/adapt_tests.rs` | 576 | **Untouched**. |
| `crates/daemon/src/run/edits.rs` | 577 | **Untouched**: `describe` and the edit-log push go in `run/edit_log.rs` (new, pure). |
| `crates/daemon/src/run/model.rs` | 563 | ≤ 575: three `Run` fields, one doc line each. |
| `crates/daemon/src/run/model_rounds.rs` | 219 | ≤ 260: two `AgentRound` fields and `set_rate_limited`. |
| `crates/daemon/src/run/engine/requests.rs` | 531 | ≤ 540: one line each in `start` (two branches), `approve` and `edit`. |
| `crates/daemon/src/run/engine/ladder.rs` | 551 | ≤ 556: one push in the rung 1 branch. |
| `crates/daemon/src/run/engine/signals.rs` | 547 | ≤ 553: four sites call `set_rate_limited`. |
| `crates/daemon/src/run/engine/review.rs` | 544 | ≤ 548: two sites call `set_rate_limited`. |
| `crates/daemon/src/run/snapshot.rs` | 290 | ≤ 330. |
| `crates/daemon/src/conversation/build.rs` | 562 | ≤ 575 (M8c.11): the summary line reads through a helper in `conversation/summary.rs` (160). |
| `crates/daemon/src/conversation/build_tests.rs` | 617 | **Untouched** (already over 600): M8c.11's summary tests go in `conversation/summary_tests.rs` (275). |
| `crates/daemon/src/headless/conversation.rs` | 397 | ≤ 430 (M8c.11). Its tests go in `headless/conversation_tests.rs` (405). |
| `scripts/pty-smoke.py` | 1797 | ≤ 10 lines: one import and one call; the stage lives in `scripts/pty_smoke_run_view.py` (new). |

Every new file stays under 600 lines; test modules are separate files (`*_tests.rs` or `tests/` directories, following the existing `#[path]` pattern).

## Tasks

Shared test helpers, created in M8c.3 and extended as tasks need them:

- `crates/tui/src/tree/tests/run_fixtures.rs` (declared from `tree/tests.rs`, `pub(crate)` so `graph`, `inspector`, `app` and `ui` tests use it): `run(id, project, state) -> RunInfo` with every other field defaulted (`serde_json::from_value` of a minimal object, so a test never lists fields it does not care about); `task(id, title, size, state) -> TaskInfo`; `worker(session, window, runtime, started) -> AgentRoundInfo`; `reviewer(round, window, runtime, started) -> AgentRoundInfo`; `headless(id, name, project, run_ref) -> WindowInfo`; `snapshot(now, runs) -> RunsSnapshot`; `gate_fixture()`, `three_task_fixture()`, `gemini_fixture()` returning `(RunsSnapshot, Vec<WindowInfo>)` exactly as the Interfaces section describes them.
- `crates/tui/src/app_tests/runs.rs` (declared from `app/tests.rs` with `#[path]`, as `app/tests.rs:8-42` declares the others): `app_with_runs(windows, snapshot) -> App` that delivers the snapshot through `on_daemon(DaemonMsg::Run(RunReply::Snapshot(..)))`, and `open_run_view(&mut App, run_id)` that does it with keys (`C-b T`, select, `l`).
- *Refreshed 2026-09-27:* `crates/tui/src/app_tests/headless.rs` already has `headless(id, name, status)`, `subscribes(effects, id)` and `inputs(effects)` (`:8-33`); M8c.6 makes them `pub(super)` and reuses them rather than writing copies.

### M8c.1 Snapshot fields the view reads

*Refreshed 2026-09-27: the field list follows decision 4 as refreshed; the files are the ones that shipped.*

**Files.** Modify `crates/proto/src/run_info.rs`, `crates/proto/src/lib.rs` (re-exports, `PROTO_VERSION` 9, the version test), `crates/proto/src/run_tests.rs` (or a new `run_tests_view.rs`), `crates/daemon/src/run/model.rs` (`Run`'s three fields), `crates/daemon/src/run/model_rounds.rs` (`AgentRound`'s two fields, `set_rate_limited`), `crates/daemon/src/run/snapshot.rs` (and `clock` removed), `crates/proto/src/run_tests_fixtures.rs` and `crates/cli/src/run_cmd/status_tests.rs` (the history literal's new type), `crates/daemon/src/run/mod.rs` (`mod edit_log;`), `crates/daemon/src/run/engine/requests.rs` (`start`, `approve`, `edit`), `engine/ladder.rs` (rung 1), `engine/signals.rs` and `engine/review.rs` (every `rate_limited_until` assignment), and the engine tests under `crates/daemon/src/run/engine/tests/`. Create `crates/proto/src/planner.rs`, `crates/daemon/src/run/edit_log.rs` and its tests `crates/daemon/src/run/edit_log_tests.rs`. `run/edits.rs` and `engine/done.rs` are not touched.

**Tests first.**

- `crates/proto/src/lib.rs`: `proto_version_is_eight` becomes `proto_version_is_nine`.
- `run_tests.rs`: `snapshot_view_fields_round_trip` — a `RunsSnapshot` with `now`, `approved_at`, two `PlanEditInfo`s, a task with `brief`, `acceptance` and a `route_spec` with `strength: None`, a task with two `TaskEventInfo` history entries, a round with `rate_limited_since`, `rate_limited_until` and two `sent_back_at` entries, and one `PlannerInfo`, survives MessagePack (`rmp_serde::to_vec_named`) inside `DaemonMsg::Run(RunReply::Snapshot(..))`. `snapshot_without_view_fields_defaults` — the JSON M8b's own round-trip test produces (no new keys) deserializes with `now == 0`, empty lists, `route_spec == RouteSpec::default()` and every `Option` `None`.
- `run/edit_log_tests.rs`: `describe_names_every_edit_op` — one of each of the nine `PlanEdit` variants gives `add t9, split t2, cancel t3, amend t4, dep t4 on t2, answer t5, pause, resume, finish`. `record_keeps_the_last_fifty`.
- Engine unit tests with M8a's `Fixture` (`engine/tests/fixture.rs:80`):
  - `approve_records_when_the_plan_was_approved`: `approved_at == Some(now)` after `Approve`; a `Start` with `yes` sets it to the start time; a fast-path `Start` (`run.path = Some(Fast)`) does too.
  - `accepted_edits_are_logged_and_counted_after_approval`: an edit before approval logs one record and leaves the count 0; two after approval make it 2; a refused batch logs nothing.
  - `a_retry_streak_records_its_start`: `ApiRetry` at 100 sets `rate_limited_since = Some(100)`; another at 130 keeps 100; the next event that clears `rate_limited_until` (`signals.rs:121`) clears it. A failed-turn rate limit (`signals.rs:341`) at 200 sets it to 200, and a reviewer's (`review.rs:407`) likewise.
  - `rung_one_marks_the_worker_round_sent_back`: a check failure at `now = 500` on a task at rung 0 leaves one worker round with `sent_back_at == [500]`, and the second failure (rung 2) appends a new round (session 2) instead. A second case with `told` set (the failure already carried in the tool reply, M8a decision 55) also records `[500]` and queues no text.
  - `snapshot_carries_the_view_fields`: `snapshot(state, 777).now == 777`; the task's `brief`, `acceptance` and `route_spec` equal the plan's (a plan route with no strength gives `route_spec.strength == None` while `route.strength` is resolved); `plan_edits` is newest first and capped at 10 after 12 edits; `history` is `TaskEventInfo`s with the model's raw `at`s, newest first, at most 10; a round rate-limited until 900 publishes `rate_limited_until == Some(900)` and `None` after it clears; no field of the snapshot holds a formatted time.

**Change.** Interfaces "proto" and "daemon". Add only the placeholder types and fields decision 5 lists as absent.

**Acceptance.** `cargo test -p anthrex-proto -p anthrex-daemon` passes. `rg -n "rate_limited_until =" crates/daemon/src/run/engine` prints nothing (every assignment goes through `set_rate_limited`). A `run.json` written by M8b's code (build it with the fixture and `serde_json` before this change and keep it as a fixture file) loads unchanged — test `old_run_json_loads` in `engine/tests/`. The derivation of `PROTO_VERSION` (8 + 1 = 9) is recorded under "Implementation notes".

**Commit.** `feat(proto): carry approval time, plan edits, briefs, route specs, rate-limit starts and bounces in the run snapshot`

### M8c.2 The client's copy of the snapshot

**Files.** Create `crates/tui/src/app/runs.rs`, `crates/tui/src/app_tests/runs.rs`. Modify `crates/tui/src/app/mod.rs` (fields, `mod runs;`), `crates/tui/src/app/daemon.rs` (the `DaemonMsg::Run` arm, `:139-140`), `crates/tui/src/app/link.rs` (`on_reconnected`, `on_send_failed`), `crates/tui/src/lib.rs` (send `run_subscription()` right after `App::new`), `crates/tui/src/app/tests.rs` (declare the test module).

**Tests first**, in `app_tests/runs.rs`:

- `the_first_effect_subscribes_to_runs`: `app.run_subscription()` is `Effect::Send(ClientMsg::Run(RunRequest::Subscribe))`.
- `a_snapshot_replaces_the_last_one_even_with_a_lower_revision`: revision 57, then revision 3 (a restarted daemon): `app.runs.revision == 3` and its runs are the second snapshot's.
- `reconnecting_resubscribes_to_runs`: `on_reconnected(windows)` returns exactly one `Send(Run(Subscribe))` beside M6's own `Subscribe`.
- `a_refused_run_subscription_is_retried_on_tick`: `on_send_failed(&ClientMsg::Run(RunRequest::Subscribe))`, then `on_tick()` returns the subscription again, once.
- `run_age_counts_from_the_daemons_clock`: a snapshot with `now = 5000`; `run_now() == 5000` and `run_age(4000) == 1000` immediately after (the `Instant` part is below one second in a test).
- `rate_limited_only_until_its_end` (*Refreshed 2026-09-27*): with `now = 5000`, a round with `rate_limited_until = Some(5030)` is `rate_limited`, one with `Some(5000)` or `Some(4990)` is not, and one with `None` is not even when the snapshot's own `rate_limited` flag is `true`. The client-side passage of time is covered by constructing the `App` with `runs_received_at` set back 40 s (a test-only setter, no sleep): the `Some(5030)` round is then no longer `rate_limited`.
- `done_replies_toast_and_refusals_toast`: `Done { request: "run approve", message: "run r1 approved" }` toasts that text; `Refused { request: "run reject", message: "no such run" }` toasts it; a two-line `Refused` toasts its first line and ` (+1 more)`; `Started`, `ConfirmNeeded`, `ToolResult`, `Triaged`, `Profile` and `Stats` change nothing.

**Change.** Decisions 1, 2 and 34 (without the edit form, which M8c.9 adds). *(Refreshed 2026-09-27: the arm is in `app/daemon.rs`; M8b's three reply variants are ignored.)*

**Acceptance.** Tests pass; `app/mod.rs` and `app/daemon.rs` grew by no more than their budgets.

**Commit.** `feat(tui): subscribe to the run snapshot and keep the latest copy`


**M8c.2 review fixes.** A reply's text reaches a toast capped at 300 characters plus `…` (`app/runs.rs::TOAST_MAX_CHARS`), for `Done` and for a refusal's first line: a 65 535-column refusal overflowed `ui/statusbar.rs`'s `u16` width sum (a debug panic). That sum is now computed in `usize` (`toast_columns`), for both of its uses. The startup `run_subscription()` in `lib.rs` has no unit test (it needs a real terminal); the M8c.10 smoke stage is its check.

### M8c.3 Runs in the project tree

**Files.** Modify `crates/tui/src/tree.rs`, `crates/tui/src/tree/rows.rs` (`visible_windows` takes the hidden set), `crates/tui/src/ui/tree_view.rs` (`narrow_line` arms), `crates/tui/src/ui/sidebar.rs`, `crates/tui/src/app/runs.rs` (`prune_runs` after each snapshot and each window list), `crates/tui/src/tree_input.rs` (`App::rows` at `:11` now calls `build_with_runs`; its other `tree::build` calls, `:102, 209, 218, 237, 252, 287`, become `self.rows()`), `crates/tui/src/app/windows.rs` (`:44`, `focus_relative`'s expanded order, becomes `tree::build_with_runs(&self.windows, &self.runs.runs, &TreeState::default())` — not `self.rows()`, which would honour the user's folds; `:130` becomes `self.rows()`) and `crates/tui/src/mouse.rs` (`:142, 179` become `self.rows()`; their exhaustive `NodeKey` matches gain arms: every new key activates through `activate_tree_node`), `crates/tui/src/tree/tests.rs` (three `mod` lines). Create `crates/tui/src/tree/runs.rs`, `crates/tui/src/tree/tests/runs.rs`, `crates/tui/src/tree/tests/run_fixtures.rs`. *(Refreshed 2026-09-27: `App::rows` lives in `tree_input.rs`, and every `tree::build` caller is named.)*

**Tests first**, in `tree/tests/runs.rs`, asserting row keys and depths in order:

- `a_run_is_one_node_above_the_plain_windows`: the gate fixture plus a second plain window gives `[Project(/r/demo), Run(add-reset-3f9a), Window(1), Window(2)]`, depths `[0, 1, 1, 1]`.
- `a_runs_windows_are_not_listed_as_plain_windows`: the three-task fixture's windows 3 and 6 are absent from `build_with_runs`; window 1 (plain) is present.
- `a_project_with_only_a_run_still_has_a_row`: a snapshot whose run's project has no window yields `[Project, Run]` and does not panic (today's `expect`).
- `terminal_runs_are_not_shown`: `accepted`, `discarded` and `failed` runs give no row; `complete` does.
- `windows_of_an_unknown_run_are_plain` (review focus 3 and 4): a headless window whose `run` names `gone-0000`, which the snapshot does not have, a headless window of a `discarded` run, and a headless window with `run: None` (an M8b onboarding scout), are all listed as plain windows under their project, in id order; with an empty snapshot (none received yet) every window is plain.
- `runs_are_ordered_oldest_first`: a snapshot listing runs newest first, as the daemon sends it, gives `Run` rows by `created_at`, then `run_id`.
- `project_status_includes_its_runs`: a project with one `Idle` window and a `running` run with a blocked task rolls up to `Attention`; with no blocked task, to `Working`; runtime counts count the plain window only.
- `the_orchestrator_takes_a_position_and_the_number_keys_reach_it`: the three-task fixture plus plain window 1: `Run` row `position == Some(1)`, window 1 position 2, `agent_order == [3, 1]`.
- `a_run_without_an_orchestrator_has_no_position`: the gate fixture: `Run` row `position == None`, `agent_order == [1]`.
- `the_filter_matches_a_runs_goal_and_id`: filter `reset` keeps `[Project, Run]` and drops a window named `api`; filter `3f9a` the same.
- `prune_runs_drops_keys_of_runs_that_left`: collapsed `Task { run: "a", .. }` and `Run("a")` survive `prune_runs` while run `a` is shown and are gone after it is discarded; `prune(&windows)` keeps them. A collapsed `Project(root)` whose root has no window but a shown run survives both `prune` and `prune_runs`, and goes once the run leaves.
- `focus_relative_reaches_the_orchestrator_even_when_folded` (in `app_tests/runs.rs`): with the project folded, `C-b j` from window 1 still reaches orchestrator window 3, because `focus_relative` builds the expanded order.
- In `ui/tree_view_tests.rs`: `the_sidebar_line_of_a_run_names_its_goal_and_progress` — the rendered line contains `◉`, `Add password reset` and `0/2`, and the orchestrator run's line contains its position `1`.

**Change.** Decisions 6–10; the `NodeKey`/`RowKind` variants of Interfaces (`Planner`, `Scout`, `Task`, `AgentRound` rows are built from M8c.4 on; their match arms exist from here).

**Acceptance.** Every existing tree, sidebar and overview test passes unchanged (`build` still means "no runs"). `rg -n "tree::build\(" crates/tui/src --glob '!**/tests*' --glob '!**/*_tests.rs'` prints only `tree.rs` itself.

**Commit.** `feat(tui): show each run as one node under its project and hide the windows it owns`

### M8c.4 The run-view rows

**Files.** Create `crates/tui/src/tree/run_rows.rs`, `crates/tui/src/tree/tests/run_rows.rs`. Modify `crates/tui/src/tree.rs` (module declaration, re-exports).

**Tests first**, each asserting `(key, depth)` in order:

- `tiers_follow_the_information_flow`: a run with scouts `S2` (started 20) and `S1` (started 10), planner `A` owning `t1`, `t2`, root tasks `t0` and `t6`, gives `Run; Scout S1; Scout S2; Task t0; Planner A; Task t1; Task t2; Task t6` at depths `0, 1, 1, 1, 1, 2, 2, 1` when `t0` is wave 0, `t1` wave 1, `t2` wave 1, `t6` wave 2 (planner `A` sorts as `t1`, `(1, index)`).
- `tasks_order_by_wave_then_plan_order`: tasks listed `t3 (wave 1), t1 (wave 0), t2 (wave 1)` come out `t1, t3, t2`.
- `a_planner_with_no_tasks_sorts_after_the_root_tasks`.
- `a_task_whose_epic_has_no_planner_hangs_from_the_root`.
- `worker_rounds_split_where_they_were_sent_back`: one worker session started 100 with `sent_back_at [300, 500]` and reviewer rounds 1 (started 200) and 2 (started 400) give `worker #1 (100–300), review #1, worker #1 r2 (300–500), review #2, worker #1 r3 (500–)`, the keys' `round` being `1, 1, 2, 2, 3`; only `r3` has `last == true`. Two bounces of one session cannot happen today (decision 14); the case is synthetic and pins the rule, not the engine.
- `a_fresh_session_is_worker_2`: sessions 1 and 2 (their `round` 1 and 2, equal to `session`, as `dispatch.rs:367` sets it) with empty `sent_back_at` give labels `worker #1` and `worker #2`, never `worker #1 r2`.
- `an_unexpected_role_on_a_task_is_labelled_not_dropped`: a task round of role `Scout`, session 1, gives one row labelled `scout #1`.
- `sub_agents_hang_under_the_last_display_round_only`: window 6 with two sub-agents; they appear once, under `worker #1 r2`, at depth `task + 2`.
- `a_round_whose_window_is_not_listed_has_no_sub_agents_and_no_panic` (review focus 3): `window_id: Some(99)` with no window 99 still gives the round's row, with `window: None`.
- `filters_keep_ancestors_and_drop_the_rest`: `Running` keeps a `working` task and its live round and drops a `merged` task; `Blocked` keeps only the `blocked` task, its planner and the root; `Runtime(Codex)` keeps a Codex-routed task with all its rounds, and of a Claude-routed task keeps only its Codex reviewer round (with the task as its ancestor); the root survives every filter even when nothing else does.
- `the_text_filter_keeps_a_matching_task_with_all_its_rounds`.
- `a_collapsed_task_hides_its_rounds`.
- `two_hundred_tasks_build_in_order` (review focus 2): a run of 200 tasks, each with a worker and two reviewer rounds, builds 801 rows, the last row being the last task's second review, with no key repeated.

**Change.** Decisions 12–15 and 21. *(Refreshed 2026-09-27: `appended_worker_rounds_map_one_to_one` is removed, because M8a never appends a worker round per bounce; decision 14.)*

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
- `round_glyphs_follow_the_verdict`: a blocking reviewer `✗` (red), a minor-only `changes` `✓`, a live reviewer `●`, an ended one without a verdict `–`, a rate-limited live worker `◆` (`rate_limited_until` after `run_now()`), a live worker whose `rate_limited_until` has passed `●`, the last worker round of a blocked task `✗`.
- `two_hundred_tasks_paint_without_overflow` (review focus 2): the 200-task run's layout has `layout.nodes.len() == 801` and, with its 600 leaves, `layout.size.1 == 600 * 4 - 1 == 2399`; painting a 120 × 40 viewport with the pan revealing the last task returns 40 lines of display width 120, and the last task's content text is on them.
- `a_row_that_disagrees_with_its_node_is_skipped` (follow-up handled): feeding `paint` a row list whose second key differs from the layout's second node paints nothing for that node and does not panic in release (`#[cfg(not(debug_assertions))]` is not needed: the check is a real `if`, not a `debug_assert`).

**Change.** Decisions 17–20; the Glyphs table; the painter's zip check becomes a real key comparison that skips a mismatched node.

**Acceptance.** Tests pass; milestone 4.6's painter tests pass unchanged.

**Commit.** `feat(tui): draw run nodes with live glyphs, the critical path and the dependency highlight`

### M8c.6 Opening, navigating and leaving the run view

**Files.** Modify `crates/tui/src/app/runs.rs` (`RunView`, `nav_rows`, `open_run_view`, `close_run_view`, `on_run_view_key`), `crates/tui/src/app/conversation.rs` (`open_conversation`), `crates/tui/src/app/modal_keys.rs` (`confirm_focused` and `open_remove_confirm` check `headless_control_refusal` first) and `crates/tui/src/app/lifecycle.rs` (`C-b R` checks it first), `crates/tui/src/tree_input.rs` (`nav_rows` everywhere a layout or a selection move needs rows; `l`, `h`, `Enter`, `Esc`, `Space` rules; `activate_tree_node` arms), `crates/tui/src/mouse.rs` (`click_graph` reads `nav_rows`; the sidebar click's `Window | Subagent` arm, which calls `self.focus(id)` directly at `:157`, and `Run` rows route through `activate_tree_node`), `crates/tui/src/ui/overview.rs` (`view`, `render` read `nav_rows`; the block title), `crates/tui/src/ui/statusbar.rs` (the two hints). `crates/tui/src/keymap.rs` is not touched (decision 27). Tests in `crates/tui/src/app_tests/runs.rs`, `crates/tui/src/app_tests/overview/runs.rs` (new, declared from `app_tests/overview.rs`), `crates/tui/src/app_tests/headless.rs` (its helpers become `pub(super)`; the control-dialog test), `crates/tui/src/ui/statusbar_tests.rs`, `crates/tui/src/keymap_tests.rs`. *(Refreshed 2026-09-27: `open_conversation`'s home, the headless control guard, the sidebar arm at `mouse.rs:157`, and no keymap change.)*

**Tests first.**

- `l_on_a_run_node_opens_the_run_view` and `enter_on_a_run_node_opens_the_run_view`: `run_view == Some(RunView { run_id, filter: All })`, selection `Run(run_id)`, `graph_pan == Pan::default()`, `nav_rows()[0].key == Run(run_id)`, no effect returned.
- `enter_on_a_run_row_in_the_sidebar_tree_opens_the_overview_on_it`.
- `space_on_a_run_node_in_the_project_tree_does_nothing`: `tree.collapsed` stays empty.
- `h_at_the_root_and_esc_return_to_the_project_overview`: both close the run view with `Run(run_id)` selected and `overview` still on; a second `Esc` leaves tree mode.
- `h_below_the_root_selects_the_parent` and `j_k_walk_the_run_view_in_order` (the order asserted against `tree::run_rows`, not restated).
- `enter_on_the_root_focuses_the_orchestrator`: the three-task fixture returns exactly `[Send(Subscribe { window_id: 3, .. })]`, `focused == Some(3)`, tree mode left.
- `enter_on_the_root_without_an_orchestrator_toasts`: the gate fixture toasts `run add-reset-3f9a has no orchestrator window; Enter on an agent opens its conversation`, returns nothing.
- `enter_on_a_round_opens_its_conversation_without_focusing`: on `t1`'s worker returns exactly `[Send(SubscribeConversation { window_id: 6, agent_id: None, from_rev: None })]`; `focused` unchanged; `conversation_follow == None`; `keymap.conversation_mode()`; `run_view` and `overview` unchanged. After M6.5's `q` closes the conversation, `nav_rows` is the run view and the selection is still the round.
- `a_run_view_conversation_follows_focus_like_any_other` (decision 25): with window 6's conversation open from the run view and PTY window 1 focused, `C-b <n>` focusing a second PTY window re-opens the view on the focused window, as M6.5's `follow_focus` does; closing it with `q` lands in the run view with the round still selected.
- `enter_on_a_task_opens_its_live_rounds_conversation` and `enter_on_a_task_with_no_agent_toasts` (`t2 has no agent yet`).
- `enter_on_a_round_whose_window_is_not_listed_yet` (review focus 3): toasts `window #99 is not listed yet`; an ended round whose window was retired toasts `worker #1 has finished and its window is gone`. Neither sends anything.
- `enter_on_a_headless_window_in_the_plain_tree_opens_its_conversation` (review focus 5): a headless window of an unknown run, listed plain, selected in the sidebar tree and in the project overview: Enter returns exactly one `SubscribeConversation` for it and no `ClientMsg::Subscribe`; a sidebar click and a canvas double click do the same; `focused` is unchanged. A PTY window's Enter still focuses it.
- `no_run_view_path_sends_input`: drive every key of this milestone (`a x e d f h l j k i / Enter Space Esc` and printable letters) and every mouse gesture over the three-task fixture, and assert no effect is `Send(ClientMsg::Input { .. })`, and no `Subscribe`, `Kill`, `Remove` or `Restart` names a headless window id (reuse `app_tests/headless.rs`'s `subscribes` and `inputs`).
- `control_commands_on_a_headless_window_open_nothing` (decision 26), in `app_tests/headless.rs`: with headless window 6 (run `add-reset-3f9a`) focused through `C-b <n>`, each of `C-b x`, `C-b X` and `C-b R` (the window live, then `Exited`) leaves `modal == None`, returns no effect, and toasts `window 6 is a headless session of run add-reset-3f9a; only the engine drives it. Use anthrex run cancel to stop it`; a headless window with `run: None` toasts the scout variant. A PTY window still gets its dialogs.
- `f_cycles_the_filters`: `All → Running → Blocked → Runtime(Claude) → Runtime(Codex) → All`; the selection is repaired into the filtered rows each time.
- `the_run_view_closes_when_its_run_leaves`: a snapshot without the run closes it and toasts `run add-reset-3f9a is gone`; one with it `discarded` toasts `run add-reset-3f9a is discarded`.
- `reveal_follows_the_selection_in_the_run_view` (review focus 2): in the 200-task run at 120 × 40, pressing `j` 800 times from the root leaves the last row's rectangle inside the viewport, with `graph_pan.y > 0`.
- `conversation_mode_wins_over_tree_mode` (`keymap_tests.rs`): with both modes on, `j` is `KeyAction::Conversation`. A pinning test: it passes before this milestone's change (`keymap.rs:144-149`), so it is not red-first.
- `statusbar_shows_the_run_view_hints`: exact strings of Interfaces for the gate and for a running run with filter `running`; the overview title `run add-reset-3f9a` is on screen and ` tree overview ` is not.

**Change.** Decisions 11, 22–27.

**Acceptance.** Tests pass; milestone 4.6 and 4.7's overview and mouse tests pass unchanged; M6.5's conversation and follow tests (`app_tests/conversation.rs`, `conversation_follow.rs`) pass unchanged.

**Commit.** `feat(tui): open the run view from its node, walk it, and open an agent's conversation from it`

### M8c.7 The run inspector's projections

**Files.** Create `crates/tui/src/inspector/run.rs` (run, planner, scout, the shared formatting of decisions 30–31), `crates/tui/src/inspector/run_task.rs` (task, rounds), `crates/tui/src/inspector/run_tests.rs`. Modify `crates/tui/src/inspector.rs` (constants, `FieldLayout`, `Inspection.right`/`layout`, `impl Default for Inspection`, `local_hhmm`, `inspect`'s arms), and every existing `Inspection { .. }` construction (`..Default::default()`). *(Refreshed 2026-09-27: `Default` keeps `inspector/panel/tests.rs`, 584 lines, to two one-line changes.)*

**Tests first**, each asserting the exact `(label, value)` list, `name` and `right` (the Interfaces tables are the expected values):

- `run_fields_match_the_mockup` (Gemini fixture), `run_fields_at_the_gate` (gate fixture: `progress ░░░░░░░░░░░░░░░░░░  0/2 merged · 2 waiting`, `gate awaiting approval · a approve · x reject · e edit · d remove`, no `path`, no `attention`), `run_fields_when_halted` (`attention halted: refs/heads/anthrex/<run>/integration moved from 1a2b3c4 to 5d6e7f8`; a moved base alone no longer halts, M8a decision 21), `run_fields_when_the_base_advanced` (a running run with `base_moved` set and no blocked task: `attention base main moved from 1a2b3c4 to 5d6e7f8 (2 new commits); accept will list them`), `run_fields_with_one_edit_and_yes` (`approved_at` 07:15 UTC, `utc_offset_secs = 7200`: `plan approved by --yes 09:15 · 1 plan edit since · last: cancel t3`), `run_spend_without_cache_or_estimate` (`tokens 0 · tool calls 0`).
- *Refreshed 2026-09-27:* `run_fields_on_the_fast_path` (`path = Some(Fast)`, `approved_by = "fast path"`, triage kinds `[code]`, scale `single`, source `decider`: `gate fast path · no plan gate · triage: code/single (decider)`), `attention_counts_each_thing_once` (two blocked tasks and a moved base, `RunInfo.attention` holding the snapshot's three lines: `attention t5 blocked: question — "…" · +2 more`), `local_hhmm_cases` (`39_720` at offset 0 → `11:02`; at `3600` → `12:02`; at `-43_200` → `23:02`; at `50_400` → `01:02`).
- `planner_fields_match_the_mockup`, `planner_while_planning` (`planning · 1m`).
- `task_fields_match_the_mockup`, `task_stage_marks` (one case per mark of the `stages` row: a check-mode task shows `proof –`; an unverified run `check –`; an unreviewed S task `review –`; `merge ●` in the queue), `task_deps_include_implicit_ones_once`, `task_fields_when_blocked` (`right` ends `blocked: mis-sized`), `task_without_diff_or_review_omits_them`, `task_budget_with_tokens` (`… · 410k/3.0M tokens`), and for decision 31a: `task_size_raised_by_the_cross_check` (`size_check` `engine S`, `decided Some(M)`, `agreed false`: `tries` ends ` · size raised S→M`), `task_blocked_by_the_fallback` (`right` ends `blocked: question (fallback)`).
- `worker_round_fields_match_the_mockup`, `the_first_display_round_has_no_fixing_and_no_activity` (`worker #1` of `t2`: fields `doing finished`, `session …`), `fixing_names_a_failed_check` (`check failed: error[E0308]: mismatched types`, the last line of `summary`), `fixing_prefers_the_decider_summary` (`decider_summary` `"type mismatch in status.rs:118\n…"`: `check failed: type mismatch in status.rs:118`), `a_rate_limited_round` (`rate_limited_until` after `run_now()`: `right` starts `rate-limited ·`, `doing rate-limited · waiting out the runtime's retry`), `an_expired_rate_limit_is_not_shown` (`rate_limited_until` before `run_now()`, snapshot flag still `true`: `right` starts with the window's status, and the run's `agents` row says `codex ok`), `task_history_is_local_time` (the `t2` history at `utc_offset_secs = 3600`: `history 13:31 review r1 changes · 13:20 check passed · 13:02 started`).
- `reviewer_round_fields_match_the_mockup`, `a_live_reviewer_is_reviewing`, `minor_only_changes_count_as_approval`.
- `scout_fields_match_the_mockup`.
- `format_duration_cases` (`0s`, `59s`, `1m`, `59m`, `1h00m`, `1h12m`, `26h05m`), `format_tokens_cases` (`999`, `1k`, `410k`, `999k`, `1.0M`, `1.8M`, `12.3M`), `progress_bar_cases` (5/9 at 18 → 10 filled; 1/3 at 10 → 3; 0/0 is not called — `no tasks yet`; 9/9 → all filled).
- `a_subagent_in_the_run_view_keeps_the_column_layout`.

**Change.** Interfaces "Inspector contents, exact" and decisions 29–31a, projection half only.

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

**Files.** Create `crates/tui/src/run_edit.rs`, `crates/tui/src/run_edit_tests.rs`, `crates/tui/src/ui/run_edit.rs`, `crates/tui/src/ui/run_edit_tests.rs`, `crates/tui/src/app_tests/gate.rs`. Modify `crates/tui/src/app/runs.rs` (keys `a x e d`, `on_edit_task_key`, reply handling of the form), `crates/tui/src/app/mod.rs` (`Modal::EditTask`, the `on_paste` arm), `crates/tui/src/app/lifecycle.rs` (three `perform` arms), `crates/tui/src/app/modal_keys.rs` (one dispatch arm), `crates/tui/src/ui/modal.rs` (one render arm), `crates/tui/src/ui/dialog.rs` (`LABEL_WIDTH`, `MARKER_WIDTH` become `pub(crate)`), `crates/tui/src/lib.rs` (`pub mod run_edit;`). The approve/reject/edit request labels are `proto::run_wire::request::{APPROVE, REJECT, EDIT}` (`run_wire.rs:193-197`; not re-exported at the root, because `proto::messages::request` exists). *(Refreshed 2026-09-27: `dialog.rs`'s constants were private; the labels' module path.)*

**Tests first.**

- `app_tests/gate.rs`:
  - `a_asks_then_approves`: `a` opens `Modal::Confirm { message: "Approve run add-reset-3f9a? 2 tasks start.", action: ApproveRun("add-reset-3f9a") }`; `y` returns exactly `[Send(Run(Approve { run_id: "add-reset-3f9a" }))]`; `n` returns nothing.
  - `x_asks_then_rejects`, `d_on_a_task_asks_then_removes` (`[Send(Run(Edit { run_id, edits: [CancelTask { task_id: "t2" }] }))]`), with the Interfaces messages.
  - `gate_keys_outside_the_gate_toast`: on a `running` run, each of `a x e d` toasts `the plan gate is closed: run add-reset-3f9a is running` and sends nothing; on a fast-path run, `the plan gate is closed: run <id> is on the fast path`.
  - `e_or_d_on_the_root_toasts`: `select a task to edit or remove`.
  - `the_edit_form_sends_only_what_changed` (*Refreshed 2026-09-27*, decision 33): open on `t1`, whose `route_spec` is `RouteSpec { runtime: Some(Claude), model: None, strength: None, effort: Some(Medium) }` and whose resolved `route` is Claude, `claude-sonnet-5`, standard, medium; M, tdd, brief `Line one\nLine two`. The form shows strength `‹ policy ›  standard` and model `policy  claude-sonnet-5`. Change effort to high and size to S; `Enter` returns exactly `[Send(Run(Edit { run_id, edits: [AmendTask { task_id: "t1", route: Some(RouteSpec { runtime: Some(Claude), model: None, strength: None, effort: Some(High) }), size: Some(S), brief: None, acceptance: None, test_mode: None, test_mode_reason: None, priority: None }] }))]` — the policy's model and strength are not pinned — and marks the form `submitting`. Changing only the size sends `route: None`.
  - `changing_the_runtime_clears_the_model`: on a task whose `route_spec` is `{ runtime: Some(Claude), model: Some("claude-sonnet-5"), strength: Some(Standard), effort: None }`, Claude → Codex empties the model field, and the sent `RouteSpec` is `{ runtime: Some(Codex), model: None, strength: Some(Standard), effort: None }`.
  - `policy_is_a_choice`: cycling runtime from `claude` reaches `policy`, and submitting sends `runtime: None`.
  - `a_mode_other_than_tdd_needs_a_reason`: `check` with an empty reason keeps the form open with focus on `Reason` and the error; with reason `renames only` sends `test_mode: Some(Check), test_mode_reason: Some("renames only")`.
  - `the_brief_round_trips_its_newlines`: the field shows `Line one↵Line two`; `Ctrl-J` then `x` at the end sends `brief: Some("Line one\nLine two\nx")`.
  - `nothing_changed_closes_with_a_toast`.
  - `a_refused_edit_shows_inline_and_a_done_edit_closes`: `Refused { request: "run edit", message: "task t1: size: …" }` sets `form.error` and clears `submitting`; a refusal of three lines shows its first line then ` (+2 more)`; `Done { request: "run edit", .. }` closes the modal and toasts.
  - `a_paste_goes_to_the_focused_text_field`: newlines become `↵` in `Brief` and are dropped in `Model`.
- `run_edit_tests.rs`: `visible_fields_hide_the_reason_for_tdd`, `choices_cycle_both_ways`, `esc_and_ctrl_c_cancel`.
- `ui/run_edit_tests.rs`: `the_form_renders_its_fields` — at 80 × 24, the form's rows contain exactly `› runtime    ‹ claude ›`, `  strength   ‹ policy ›  standard` (the resolved value muted; *Refreshed 2026-09-27*), `  size       ‹ M ›`, `  test mode  ‹ tdd ›`, `  brief      Line one↵Line two`, and the hint `⏎ save  tab next  ←/→ change  esc cancel`; the title ` edit t1 `.

**Change.** Decisions 32–34.

**Acceptance.** Tests pass. `rg -n "ClientMsg::Input" crates/tui/src/app/runs.rs crates/tui/src/run_edit.rs crates/tui/src/ui/run_edit.rs` prints nothing.

**Commit.** `feat(tui): approve, reject and edit a run's plan from the run view`

### M8c.11 Headless conversation polish (daemon)

*Refreshed 2026-09-27: new task.* It takes the two follow-ups filed for this milestone ("From M8a.7 (2026-09-23), for M8c's conversation view" and "From M8a.7's fix round 1, … for M8a.18, M9.5 and M8c", followups file `:721-731` and `:751-756`), both still open on `a910e18`. It is numbered 11 because the refresh added it, but it is **done before M8c.10**, which closes the milestone; it is listed here in the order to do it. It is not folded into an existing task: M8c.1 is the engine's model and snapshot, and every other task is client code, while these two are fixes to the daemon's conversation building (`crates/daemon/src/conversation/` and `crates/daemon/src/headless/conversation.rs`) that no other task touches. They belong to this milestone because the run view is what puts headless conversations in front of the user: every worker, reviewer and scout is watched only through them.

**Files.** Modify `crates/daemon/src/conversation/summary.rs` (a pure `response_text(&Value) -> Option<&str>`), `crates/daemon/src/conversation/build.rs` (`post_tool_use`'s summary line, `:238-242`), `crates/daemon/src/conversation/summary_tests.rs`, `crates/daemon/src/headless/conversation.rs`, `crates/daemon/src/headless/conversation_tests.rs`. `conversation/build_tests.rs` (617 lines) is not touched.

**Tests first.**

- `summary_tests.rs`: `an_object_response_summarises_its_text_field` — `{"output": "[task-b 9d0ec2a] add a\nmore"}` gives `Some("[task-b 9d0ec2a] add a\nmore")`; `{"error": "…"}` and `{"stdout": "…", "stderr": ""}` give their text; an object with two non-empty text fields among `output`, `error`, `stdout`, or with none, or whose field is not a string, gives `None`, so the summary falls back to today's compact JSON.
- `headless/conversation_session_tests.rs`'s `a_codex_session_builds_a_real_conversation` currently asserts the JSON summary line; it changes to assert `[task-b 9d0ec2a] add a` — the one existing expectation this task changes. The enriched `detail` (the full result text) is unchanged.
- `conversation_tests.rs`: `an_interrupted_turn_ends_even_when_hooks_fire` — with `hooks_fire == true`, a Claude `TurnEnded { outcome: Interrupted, .. }` for an open sent turn yields a synthesised `Stop`, so the assistant turn is no longer `Running` and its open call is not left `Pending`; `a_codex_process_exit_ends_the_open_turn` — a Codex `ProcessExited` while a sent turn is open does the same. A turn already ended by its own `Stop` hook gets no second one.

**Change.** In `post_tool_use`, the summary's first line comes from `summary::response_text(value)` when it returns `Some`, else from `render_response` as today; the stored result text is unchanged. In `headless/conversation.rs`: a Claude `TurnEnded { outcome: TurnOutcome::Interrupted, .. }` keeps its synthesised `Stop` even with `hooks_fire` (today the `if hooks_fire { input.hooks.clear() }` rule, `:206-208`, drops it, and Claude fires no `Stop` hook for an interrupted turn); a Codex `ProcessExited` while a sent turn is open synthesises a `Stop` (an interrupted Codex turn ends with the process and no `turn.*` line, so nothing else closes it). The fix only closes a turn the daemon knows has ended; it synthesises nothing for a turn a hook already closed.

**Acceptance.** `cargo test -p anthrex-daemon conversation` passes. Both follow-up entries are marked handled by M8c.10.

**Commit.** `fix(daemon): summarise a tool's text response and end an interrupted headless turn`

### M8c.10 The smoke stage and the milestone's paperwork

**Files.** Create `scripts/pty_smoke_run_view.py`. Modify `scripts/pty-smoke.py` (≤ 10 lines: one import, one call), `docs/timing-budgets.md` (one row), `docs/ROADMAP.md` (status), `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`, this brief's "Implementation notes".

**Tests first.** The stage itself. *Refreshed 2026-09-27:* `run_view_stage(PtyProc, bin_path, run_cmd, fail)`, following the injection pattern of `run_project_tree_stage(REPO, PtyProc, run_cmd, fail)` (`pty-smoke.py:1471`), called as `run_view_stage(PtyProc, BIN, run_cmd, fail)` right after `adapt_stage(run_cmd, fail)` (`pty-smoke.py:1716`) and before `== stage 12` (`:1718`). It prints `== stage 11e: the run view shows the plan gate, approves it, and opens a worker's conversation ==` (the run milestones' stage letters are fixed: M8a `11c`, M8b `11d`, M8c `11e`, M9 `11f`, M9.5 `11g`, M9.1 `11h`, M9.2 `11i`). It uses the one daemon `pty-smoke.py` starts (with `ANTHREX_CLAUDE_BIN`, `ANTHREX_CODEX_BIN` and `ANTHREX_DECIDER_BIN` pointing at `fake-agent`), and imports `_git`, `_write_script`, `RUN_WAIT` and `RUN_CMD_TIMEOUT` from `pty_smoke_run`, as `pty_smoke_adapt.py:18` does. No TUI is attached at that point (stage 11b's client detached at `:1709-1712`), so the stage starts its own client, `PtyProc([bin_path])`, detaches it with `C-b d` and closes it in `finally`. The conversation bound is `RUN_VIEW_CONVERSATION_TIMEOUT = 45.0`, defined in the module with the same derivation as `CONVERSATION_VIEW_TIMEOUT` (`pty-smoke.py:1090-1102`; a hyphenated script cannot be imported) and a row in `docs/timing-budgets.md`.

1. Create `/tmp/anthrex-smoke-view-<pid>` (a repository with an identity, `commit.gpgsign false` and one commit, as 11c makes it) and the plan `/tmp/anthrex-smoke-view-plan-<pid>.toml`: profile `check = "true"`; one S task `t1`, title `add a`, `test_mode = "check"`, reason `smoke`, owns `["a.txt"]`, route runtime `claude`. `_write_script(repo, "worker-t1-1", [{"wait_ms": 600000}])` (a turn that stays open, so the worker round is live).
2. `anthrex run start --plan … --dir <repo>` (no `--yes`; `--unconfined-checks` off macOS, as 11c adds it), with `RUN_CMD_TIMEOUT`; read the run id and its last four characters `h4`.
3. In the stage's client: `C-b T`; `/`, type `h4`, `Enter`; `l` (the project's first child, the run); wait for `run <h4>  0/1`. `l` again; wait for ` run <id> ` and `t1 add a S` and the hint `a approve`.
4. `a`; wait for `Approve run <id>?`; `y`. Wait, up to `RUN_WAIT`, for `worker #1 claude`.
5. `l`, `l` (to `t1`, then its worker round); `Enter`; wait, up to `RUN_VIEW_CONVERSATION_TIMEOUT`, for the window name `<h4>/t1.w1` in the conversation's header.
6. `q`; wait for ` run <id> `. `Esc`; wait for ` tree overview `. `Esc`. `C-b d`; the client exits 0.
7. `anthrex run cancel <id>`, then `anthrex run discard <id> --confirm <id>`; in `finally`, close the client and remove both paths.

Other runs in the daemon do not disturb it: 11c's run is accepted (hidden), and 11d's fast-path run, if still `complete`, is filtered out by `/h4`.

**Change.** The stage. Set milestone 8c to `done` in `docs/ROADMAP.md`. In the follow-ups file, mark handled the painter zip check (M8c.5) and the two M8a.7 entries (M8c.11), mark the "`ui/mod.rs` is 599 lines" entry resolved (99 lines), and record under a new "From milestone 8c" heading: the `doing` field's missing tool target (Risks 6); and any field of "Consumes from later milestones" still unfilled, assigned to its owner.

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

- `PROTO_VERSION` is 9, and its derivation is recorded under "Implementation notes". *(Refreshed 2026-09-27.)*
- `rg -n "std::fs|std::process|std::thread|tokio|SystemTime|Instant::now" crates/tui/src/inspector crates/tui/src/tree crates/tui/src/graph crates/tui/src/ui crates/tui/src/run_edit.rs crates/tui/src/inspector.rs crates/tui/src/tree.rs` prints nothing (`app/runs.rs` may read `Instant` for `runs_received_at`, as `app/mod.rs` already does for `windows_received_at`). *(Refreshed 2026-09-27: `crates/tui/src/ui` added, which hard rule 5 names; it is clean on `a910e18`.)*
- `rg -n "ClientMsg::Input" crates/tui/src` lists only the sites M8a.17 already guards, all behind `focused_pty()`: keys (`app/mod.rs:383-389`), paste (`:467-483`) and the mouse wheel (`mouse.rs:83-92`), at whatever lines they move to.
- `rg -n "rate_limited_until =" crates/daemon/src/run/engine` prints nothing (M8c.1).
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
7. `Esc`, `Esc`. `anthrex run cancel <id>`, `anthrex run discard <id> --confirm <id>`. While the run's reviewer window lingers as `Exited` (up to 30 s), confirm it is listed as a plain window under `demo` and that Enter on it opens its conversation, not a terminal. Focus it with `C-b <n>` and press `C-b x`, `C-b X` and `C-b R`: each toasts that the engine drives it, and no dialog opens. *(Refreshed 2026-09-27.)*
8. `anthrex daemon stop`; `pgrep -fl "anthrex daemon"` shows nothing of yours; remove `/tmp/anthrex-m8c`.

## Review focus

The five failure modes most likely to bite a user that the tests above would not exercise without being told to. Each has a named test in its owning task; the reviewer checks that each test exists and fails without the change it guards.

1. **Tiny terminals.** The tall panel adds a third height step; an off-by-one in `areas` or in the rows renderer panics or eats the canvas at the thresholds. Test: `no_panic_at_degenerate_sizes_in_the_run_view` (M8c.8), plus `a_short_terminal_steps_down`.
2. **200-task runs.** Rounds multiply leaves: 200 tasks with three rounds each is 600 leaves and a canvas of 2399 rows, where a `u16` overflow, a duplicate key or a reveal that cannot reach the bottom would show. Tests: `two_hundred_tasks_build_in_order` (M8c.4), `two_hundred_tasks_paint_without_overflow` (M8c.5), `reveal_follows_the_selection_in_the_run_view` (M8c.6).
3. **A snapshot that runs ahead of the window list.** The engine names a window id in the snapshot before `WindowsChanged` lists it, and the other way round at startup. Nothing may panic, no row may be lost, and Enter must say why it cannot open anything. Tests: `windows_of_an_unknown_run_are_plain` (M8c.3), `a_round_whose_window_is_not_listed_has_no_sub_agents_and_no_panic` (M8c.4), `enter_on_a_round_whose_window_is_not_listed_yet` (M8c.6).
4. **A discarded run's window.** After `discard` the run leaves the tree but a retired reviewer's window is still listed for up to `RETIRE_AFTER`; it must reappear as a plain window, not vanish, and the run view must close rather than show a stale run. Tests: `windows_of_an_unknown_run_are_plain` (M8c.3), `the_run_view_closes_when_its_run_leaves` (M8c.6).
5. **A headless window selected in the plain project tree.** The one place a headless window is still a plain row (and a focusable one, through `C-b <n>`); Enter, a click or a double click there must open its conversation and never subscribe to a terminal or send input, and `C-b x`/`X`/`R` on it must open no dialog. Tests: `enter_on_a_headless_window_in_the_plain_tree_opens_its_conversation`, `no_run_view_path_sends_input` and `control_commands_on_a_headless_window_open_nothing` (M8c.6). *(Refreshed 2026-09-27: the control commands.)*

## Risks and gotchas

1. **Two row lists.** The sidebar reads `App::rows()` and the canvas `App::nav_rows()`. Any code that builds rows for a layout, a hit test or a selection move with the wrong one works in the project overview and breaks only in the run view. M8c.3's acceptance greps for stray `tree::build` calls; review every `rows()` call in `tree_input.rs`, `mouse.rs` and `ui/overview.rs` by hand.
2. **The selection key is shared.** `NodeKey::Run(id)` is both the project-tree node and the run-view root, which is what makes entry and exit free — and what makes `Space` on the project-tree node dangerous: folding it would collapse the run view's root. Decision 22 forbids it; `space_on_a_run_node_in_the_project_tree_does_nothing` guards it.
3. **Snapshot times are the daemon's.** Never subtract a snapshot time from the client's clock; always go through `run_age` (decision 2). Formatting a snapshot stamp as local wall-clock time (`local_hhmm`, decision 31) is not an age and reads no clock; it is the one other use of a snapshot time. Comparing `rate_limited_until` goes through `run_now()`, never the client's clock. *(Refreshed 2026-09-27.)*
4. **Revisions reset.** Do not add a "newer revision only" guard (decision 1); a restarted daemon would be ignored until its revision passed the old one.
5. **Ambiguous-width glyphs.** `◉`, `◆`, `▫`, `◇`, `◐`, `▸`, `⊘` and `⇠` are one column in `unicode_width` and in most terminals, and milestone 4.6 already relies on `◆`. A CJK-locale terminal that draws them two wide misaligns borders exactly as `◆` does today; nothing new is at stake, but do not add a genuinely wide glyph.
6. **The `doing` field is thinner than the mockup.** The spec shows `editing crates/daemon/src/status.rs (last tool: apply_patch)`; neither the snapshot nor `WindowInfo` carries a tool's target. Do not reach into M6.5's conversation state for it (the view would then depend on a subscription the user may not have open); record the follow-up (M8c.10).
7. **M6.5's conversation for a retired window is gone.** `ConversationGone` arrives when the window is removed; decision 25's toast avoids asking for it in the first place.
8. **Fixture drift.** The exact-string tests encode the fixture's numbers. Build the three fixtures once in `run_fixtures.rs` and reuse them; a copy per test file is how two tests come to disagree about the same run.
9. **A restored headless window can come back as a PTY window** (daemon side, not fixed here). Followups file, "Review C, M7" (`:1175-1179`): a restored headless window whose `run` does not parse is restored as a PTY window, so `headless_guard` no longer refuses `Restart`, and `C-b R` would start an interactive `claude` in the task checkout on the user's normal settings. The client cannot tell such a window from a real PTY window (its `kind` is `Pty`), so decision 26's client guard does not cover it. It is daemon restore code (M8a's, owned next by M9); this milestone only records it. *(Refreshed 2026-09-27.)*

## Follow-ups handled

- **From milestone 4.6's final review: "The painter's zip invariant is only `debug_assert`ed."** Still open on `a910e18` (`graph/paint.rs:42-43`). This milestone adds five row kinds, which raises the cost of a future divergence; M8c.5 makes the check a real comparison that skips a mismatched node, with `a_row_that_disagrees_with_its_node_is_skipped`. Mark the entry handled.
- *Refreshed 2026-09-27:* **From M8a.7, for M8c's conversation view: "A synthesised Codex tool call's one-line summary reads as JSON"** (followups file `:721-731`). M8c.11 reads an object response's single text field for the summary line. Mark it handled.
- *Refreshed 2026-09-27:* **From M8a.7's fix round 1, for M8a.18, M9.5 and M8c: "An interrupted turn stays `Running` in the conversation"** (`:751-756`; M8a.18's notes do not mark it handled). M8c.11 closes the turn for a Claude `TurnEnded { Interrupted }` with hooks firing and for a Codex `ProcessExited` with a sent turn open. Mark it handled. A stall watchdog's interrupt is common, and the run view is where a user will watch it.

Not taken, and why:

- `TreeState::overview` removal and the sub-agent label reveal gap stay with milestone 7, which owns them.
- *Refreshed 2026-09-27:* "A turn's prose always leads it … worth revisiting for milestone 8's run view" (`:652-656`): M6.5's enrichment, not this view.
- "Review C, M7: a restored headless window whose `run` does not parse comes back as a PTY window" (`:1175-1179`): daemon restore code, M9's; recorded in Risks 9.
- "The main pane can switch to a new window while the new-agent form is still open" (`:1527-1539`): it belongs with the next milestone that touches that form; this one adds a different form.
- The TUI-wide ASCII spinner and the sidebar box glyphs in ASCII mode (`:626-636`): not taken; the Glyphs section notes that the fix must cover this milestone's new glyphs too.
- "`crates/tui/src/ui/mod.rs` is 599 lines" (`:122-124`): already resolved (99 lines on `a910e18`); M8c.10 marks it so.

## Names as shipped

*Refreshed 2026-09-27:* this section replaces "Names taken from M8a" and "Names taken from M6.5", which were written against those milestones' briefs. Every row is checked against `a910e18`; where the shipped name differs from what the 2026-09-22 brief assumed, the row says so, and "Implementation notes" lists the change.

### From M8a and M8b

| Name | What it is, as shipped | Where |
|---|---|---|
| `proto::RunsSnapshot { revision, runs }` | Pushed as `RunReply::Snapshot` on `Subscribe` and on every publish; the revision restarts per daemon, and `forward` sends only increasing revisions within one connection. **`runs` are sorted newest first.** M8c.1 adds `now`. | `run_info.rs`; `run/snapshot.rs:22-28`; `server/run_api.rs:67-95` |
| `proto::RunInfo` | `run_id`, `goal`, `project`, `root`, `revision`, `state`, `paused_from`, `halted_reason`, `base_branch`, `base_sha`, `run_branch`, `run_head`, `base_moved`, `approved_by` (`"user"`, `"--yes"` or **`"fast path"`**, `run::triage::FAST_APPROVED_BY`), `max_writers`, `max_readers`, `max_bounces`, `writers_busy`, `readers_busy`, `unverified`, `worker_sandbox`, `unconfined_checks`, `trusted_project`, `rate_limits`, `report_path`, `outcome`, `tasks` in plan order, `critical_path`, `attention` (one `"{id} blocked ({reason}): {text}"` line per blocked task first, then base-moved, final-check, stale-profile and promotion lines, `snapshot.rs:127-157`), `created_at`; M8b's `path: Option<RunPath>`, `triage: Option<TriageInfo>`, `promote_requested_at`, `profile_source`, `usage: Option<RunUsage>` (always `Some` from an M8b daemon), `scouts: Vec<ScoutInfo>`. | `run_info.rs:208-260` |
| `proto::TaskInfo` | As the 2026-09-22 brief listed (`history: Vec<String>` newest first, `"<hh:mm> <text>"` in **UTC**, at most 10 — M8c.1 changes it to `Vec<TaskEventInfo>`), plus M8b's `decider_usage`, `size_check: Option<SizeCheckInfo { engine, decided, agreed, reason, source }>`, `diff: Option<DiffStats>`, `phases`, `block_source: Option<DeciderSource>`. `route` is the resolved `Route`. | `run_info.rs:134-191` |
| `proto::AgentRoundInfo` | `role`, `session`, `round` (**a worker's equals its `session`**; a reviewer's equals its review round and its `session`), `window_id`, `route`, `session_id`, `started_at`, `ended_at`, `tool_calls`, `last_event`, `turn_open`, `turns`, `rate_limited` (evaluated at publication; **no `rate_limited_until`** — M8c.1 adds it), `open_subagents`, `denials`, `usage`. One entry per worker session and one per review round; rung 1 never appends one. | `run_info.rs:112-130`; `engine/dispatch.rs:358-367`; `engine/review.rs:141-150`; `engine/ladder.rs:197-212` |
| `proto::CheckInfo` | `summary` (the raw tail) and M8b's `decider_summary: Option<String>`, `summary_source`. | `run_info.rs:71-84` |
| `proto::ReviewInfo`, `Finding`, `Severity`, `Verdict`, `TokenUsage` (`billable()`), `Spend`, `Budget` | As the 2026-09-22 brief listed. | `run_info.rs`, `run.rs` |
| `proto::Route`, `RouteSpec` (`Default`, `deny_unknown_fields`, every field `Option`), `Strength`, `Effort`, `Size`, `TestMode`, `TaskState` (11), `RunState` (8, with `label()` in snake_case and `is_terminal()` for accepted/discarded/failed), `BlockReason`, `GateCounts`, `DoneSignal` | As listed. | `proto/src/run.rs` (`RouteSpec` `:88-101`, `RunState` `:275`, `is_terminal` `:398`) |
| `proto::AgentRole { Orchestrator, Worker, Reviewer, Scout }` | **M8b added `Scout`**; `Copy, Eq, Hash`. | `run.rs:18-27` |
| `proto::RunRef { run_id, task_id, role, session }` on `WindowInfo.run`; `WindowKind { Pty, Headless }` on `WindowInfo.kind` | Set on every run window; M8b's onboarding scout is `Headless` with `run: None`, an area scout (M9) has `RunRef { role: Scout, task_id: None }`. | `run.rs:32-37`; `types.rs:142-144`; `scout/spec.rs:132-136` |
| `proto::{RunPath, Scale, DeciderSource, TriageInfo, SizeCheckInfo, DiffStats, PhaseSecs, RunUsage}` | M8b's adaptation types. | `proto/src/adapt.rs` |
| `proto::{ScoutInfo, ScoutKind, ScoutState}` | Field for field as "Consumes from later milestones" lists them. | `proto/src/scout.rs:11-76` |
| `RunRequest::{Subscribe, Approve { run_id }, Reject { run_id }, Edit { run_id, edits }, …}` | As listed. | `run_wire.rs` |
| `RunReply::{Started, Done { request, message }, Refused { request, message }, ConfirmNeeded, Snapshot, ToolResult, Triaged, Profile(Box<ProfileReply>), Stats(HistoryStats)}` | **M8b added the last three.** A `run edit` refusal joins its errors with `\n`. | `run_wire.rs:132-166`; `engine/requests.rs:296-298` |
| `proto::run_wire::request::{APPROVE, REJECT, EDIT}` | Labels `run approve`, `run reject`, `run edit`. **Not** `proto::request` (that is `proto::messages::request`). | `run_wire.rs:193-197` |
| `PlanEdit::{AddTask, SplitTask, CancelTask { task_id }, AmendTask { task_id, brief, acceptance, route: Option<RouteSpec>, test_mode, test_mode_reason, priority, size }, AddDep, Answer, Pause, Resume, Finish}` | `AmendTask.route` **replaces the task's whole `spec.route`**. | `run.rs:214-253`; `run/edits.rs:446-448` |
| `ClientMsg::Run(RunRequest)`, `DaemonMsg::Run(RunReply)` | The TUI's ignore arm is in **`app/daemon.rs:139-140`**. | |
| Window names `<h4>/<task>.w<session>`, `<h4>/<task>.r<round>`; `Run::short()` (last four characters) | Used by the smoke stage. | `dispatch.rs:316`; `review.rs:143`; `model.rs:545` |
| `run::driver::RETIRE_AFTER` (30 s) | A retired window is listed `Exited`, then removed. | |
| `server/headless_guard.rs::refuse`; `manager::control_refusal(id, run)` | Refuses `Subscribe`, `Input`, `Kill`, `Remove` and `Restart` for a headless window, before dispatch (`server.rs:272`); `control_refusal`'s text is what decision 26 toasts. | `headless_guard.rs:17-35`; `manager/headless.rs:215` |
| The TUI's headless placeholder; `focused_pty()` | No `Subscribe`, `Input` or `Resize` for a headless window. | `app/windows.rs:17-26` |
| `run::model::Run` (`model.rs:365`); `run::model_rounds::{AgentRound, TaskEvent { at, text }}` (`model_rounds.rs:45, 216`, re-exported by `model.rs:23-25`); `Run.log: Vec<LogEntry>` | Where M8c.1's fields go. | |
| `run/snapshot.rs::{snapshot(state, now), clock}` | `clock` is private and formats **UTC** `hh:mm` (`:191-194`); its only caller is `TaskInfo.history`, so M8c.1 removes it. | |
| `engine/requests.rs::{start, approve, edit}`; `engine/ladder.rs::{take_rung, worker_round, count_failure}`; `rate_limited_until` sites `engine/signals.rs:121, 145, 341, 507`, `engine/review.rs:407, 529` | Where M8c.1 records its facts. `engine/done.rs` is not involved. | |
| `run::triage::{kinds_scale, source_label, FAST_APPROVED_BY}` | Pure; `anthrex run status` uses the first two (`cli/src/run_cmd/status.rs:4`). | `triage.rs:29, 88, 96` |
| M8a's `Fixture`; `e2e_green_s_task_runs_to_merged` | `engine/tests/fixture.rs:80`; `crates/cli/tests/run_e2e_basic.rs`. | |
| `scripts/pty_smoke_run.py::{run_engine_stage(run_cmd, fail), _git, _write_script, RUN_WAIT, RUN_CMD_TIMEOUT}`; `scripts/pty_smoke_adapt.py::adapt_stage(run_cmd, fail)` | Stages 11c and 11d, called at `pty-smoke.py:1715-1716`. **Not** `run_stage(env, bin_path)`. | |

### From M6.5

| Name | What it is, as shipped | Where |
|---|---|---|
| `App.conversation: crate::conversation::ConversationView`; `ConversationView::{open(window_id) -> Vec<Effect>, is_open, close}` | `open` emits `Send(SubscribeConversation { window_id, agent_id: None, from_rev: None })`. | `conversation.rs:49-80` (595 lines) |
| `App::{toggle_conversation, sync_conversation_mode, follow_focus, on_conversation_gone}`, `App.conversation_follow` | `C-b m`'s steps are: clear `conversation_follow`, `conversation.open(id)`, `sync_conversation_mode()` (which calls `keymap.set_conversation_mode(conversation.is_open())`). `follow_focus` re-opens the view on the focused window when focus changes. | `app/conversation.rs:12-24, 37-87, 105` |
| `Keymap::{set_conversation_mode, conversation_mode}`, `KeyAction::Conversation(KeyEvent)` | Conversation mode is checked before tree mode. | `keymap.rs:44, 84-90, 144-149` |
| `ui::draw` | Draws the conversation in `l.main` in preference to the overview, so a conversation opened over the run view covers it and closing it reveals it. | `ui/mod.rs:84-90` |
| `App.utc_offset_secs: i64` | Read once at startup (`lib.rs:79`, `local_utc_offset_secs`); decision 31 formats local stamps with it. | `app/mod.rs:143` |

## Consumes from later milestones

*Refreshed 2026-09-27:* the first three rows landed in M8b exactly as written and are consumed, not added; M8c.1 adds the `estimate_left_secs`, `bound_ratio_permille` and `planners` rows as written (decision 5), and their owners fill them. The view renders absence as stated.

| Field | Owner | When absent |
|---|---|---|
| `#[serde(default)] RunInfo.scouts: Vec<ScoutInfo>`; M8b's `ScoutInfo { id: String, kind: ScoutKind, question: String, state: ScoutState, failure: Option<String>, window_id: Option<u32>, route: Route, started_at: u64, ended_at: Option<u64>, tool_calls: u32, report_bytes: Option<u32>, files: Vec<String>, usage: TokenUsage }`; `ScoutKind { Onboarding, Area }`; `ScoutState { Starting, Working, Reported, Failed }` (snake_case) | **Landed in M8b** (`proto/src/scout.rs`); M8b defines them (M8b Interfaces `scout.rs`; M8b owns the names, so M8c.1 adds them exactly as M8b declares them when M8b has not landed); M9 fills `RunInfo.scouts` with a run's area scouts (M8b's Out table). | No scout nodes |
| `#[serde(default)] RunInfo.usage: Option<RunUsage>`; `RunUsage { total: TokenUsage, by_role: BTreeMap<String, TokenUsage>, decider_calls: u32, decider_fallbacks: u32 }` | **Landed in M8b** (`proto/src/adapt.rs`; always `Some` from an M8b daemon); M8b (its decision 29: usage by role, the orchestrator's from OTLP) | `spend` sums the rounds' own usage, which leaves out deciders, scouts and the orchestrator |
| `#[serde(default)] TaskInfo.diff: Option<DiffStats>`; `DiffStats { files: u32, hunks: u32, added: u32, removed: u32 }` | **Landed in M8b** (`proto/src/adapt.rs`; set only once a task merged or ended with a head); M8b (its decision 32 measures a task's diff as `DiffStats` and publishes it as `TaskInfo.diff`, M8b Interfaces `run_info.rs`) | The `diff` field shows only the test part, or is omitted |
| `#[serde(default)] RunInfo.estimate_left_secs: Option<u64>` | M9.5 (history medians; M8b only records history) | No `est. left` |
| `#[serde(default)] RunInfo.bound_ratio_permille: Option<u32>` | M9.5 (history medians) | No `× the bound` |
| `#[serde(default)] RunInfo.planners: Vec<PlannerInfo>`; `PlannerInfo { epic: String, title: String, area: Vec<String>, route: Route, window_id: Option<u32>, state: PlannerState, started_at: u64, ended_at: Option<u64>, edits_accepted: u32, edits_rejected: u32, last_rejection: Option<String>, replans: Vec<String> }`; `PlannerState { Planning, Finished, Failed }` (snake_case, default `Planning`) | M9 | No planner nodes; every task hangs from the root |
| The orchestrator's PTY window with `WindowInfo.run == Some(RunRef { role: AgentRole::Orchestrator, task_id: None, .. })` | M9 | Root drawn `run <h4>`; Enter on it toasts |
| `AgentRole` variants for scouts and sub-planners, on their windows' `RunRef` | M8b landed `Scout`; M9 adds `Planner` | Not read by this milestone: scouts and planners are linked by `window_id` |
| Racer and test-writer rounds | M9.5 | Not drawn |

## Implementation notes

### Refresh 2026-09-27 (post-M8b)

Refreshed against branch `m8b-adaptation` at `a910e18` (PR #19, merging unchanged). `a910e18` is `da54149`, the commit the refresh worksheet read, plus one Linux build fix (`crates/daemon/src/run/git/checkout.rs`, `crates/cli/tests/profile_verify_leftovers.rs`) that touches no file this brief names, so every line number holds on both. The copy of this brief on `main` and on that branch were byte-identical, as was the spec. Every changed decision keeps its number; the reason for each change is M6.5's, M8a's or M8b's shipped code, or a ruling made at the refresh.

**Status and protocol**

- Status `blocked` → `ready`, and `docs/ROADMAP.md` row 8c → `ready` (done when PR #19 merges): M8a and M8b are `done`.
- Protocol "one above `PROTO_VERSION` on main when M8c starts" → **9**: `PROTO_VERSION = 8` at `crates/proto/src/lib.rs:32`; the test `proto_version_is_eight` is in `lib.rs:99`, not `run_tests.rs`, and becomes `proto_version_is_nine`. The new fields are all `#[serde(default)]`, so an 8 peer would still decode them; the bump follows the ROADMAP rule that 8b and 8c each bump, so an 8 client never shows a 9 daemon's view half-filled.

**Rulings made at the refresh**

- **Time stamps.** The snapshot publishes raw unix times — `RunInfo.approved_at` and `PlanEditInfo.at` on each plan edit — and the client formats them as local `hh:mm` with `App.utc_offset_secs`, inside the pure inspector code (`local_hhmm`). The old `approved_hhmm` and `"<hh:mm> <text>"` strings would have been UTC (`run/snapshot.rs::clock`, `:191-194`). *Amended the same day by the coordinator:* the ruling extends to `TaskInfo.history`, which now carries each entry's raw `at` as `TaskEventInfo { at, text }` and is formatted with `local_hhmm` too, so every time in the view is local and `snapshot.rs::clock` is removed.
- **Rate limits.** *Ruled the same day by the coordinator:* the snapshot also publishes `AgentRoundInfo.rate_limited_until` as a raw unix time, and the client shows a round as rate-limited only while `rate_limited_until > run_now()` (decision 2), using the daemon time it already has. The snapshot's `rate_limited` flag, evaluated at publication and re-sent only on a revision bump, could show an expired limit until the next publish; the view no longer reads it.
- **Smoke bound.** *Accepted by the coordinator:* `RUN_VIEW_CONVERSATION_TIMEOUT = 45.0` in `scripts/pty_smoke_run_view.py`, with the derivation of `CONVERSATION_VIEW_TIMEOUT` and a `docs/timing-budgets.md` row.
- **Edit-form routes.** M8c.1 publishes the plan's own route spec as `TaskInfo.route_spec`, so the form shows `policy` for an unset field (with the resolved value muted beside it), and a route edit sends only what the user changed: `route_spec` with the changed fields replaced, never every resolved value pinned (decisions 4 and 33).

**Decisions changed, and why**

- R1 (decision 4). The field list follows the two rulings (`approved_at`, `PlanEditInfo`, `route_spec`). `Run.plan_edits` is a new `PlanEditRecord`, not M8a's `TaskEvent`, so M9 can add a `source` with `#[serde(default)]` (tiered-testing spec §12.5); `describe` and the push-and-cap live in a new pure `run/edit_log.rs`, because `run/edits.rs` is 577 lines. `Run.log` already notes "applied N plan edits" but mixes every event, so it is not reused.
- R2 (decision 4). `sent_back_at` is pushed in `take_rung`'s rung 1 branch whether or not the text is queued: with `told` (M8a decision 55) the tool reply already carried the failure (`engine/ladder.rs:208-211`).
- R3 (decision 4). `rate_limited_since` is kept by one `AgentRound::set_rate_limited` helper called at every site that sets or clears `rate_limited_until` — `engine/signals.rs:121, 145, 341, 507` and `engine/review.rs:407, 529` — not only at `ApiRetry`.
- R4 (decision 4). `approved_at` is also set on a fast-path start (`requests.rs::start`'s fast branch), beside `--yes`.
- R5 (decision 5). M8b landed `RunInfo.scouts`, `RunInfo.usage`, `TaskInfo.diff`, `ScoutInfo`/`ScoutKind`/`ScoutState`, `DiffStats` and `RunUsage` exactly as this brief listed them (the last two in `proto/src/adapt.rs`); M8c.1 adds only `planners`/`PlannerInfo`/`PlannerState` (in a new `proto/src/planner.rs`), `estimate_left_secs` and `bound_ratio_permille`.
- R6 (decision 6). "Not accepted, discarded or failed" is `!RunState::is_terminal()`.
- R7 (decision 7). Runs arrive newest first (`snapshot.rs:23-28`), so the client sorts. `TreeState::prune` (`tree.rs:196-202`) dropped a `Project(root)` key with no window; a project with only a run now keeps its fold state.
- R8 (decision 8). An M8b onboarding scout's window is headless with `run: None` (`scout/spec.rs:132-136`); it is a plain window, and the fixtures cover it.
- R9 (decision 14). A worker's `AgentRoundInfo.round` equals its `session` (`engine/dispatch.rs:367`), and rung 1 re-sends to the same round (`ladder.rs:197-212`); M8a never appends a worker round per bounce. Worker display rounds are numbered from `sent_back_at` alone, and the "appended" shape and `appended_worker_rounds_map_one_to_one` are removed (reading `round > 1` as a bounce would have drawn `worker #2` as `worker #1 r2`). Failures count per task (`ladder.rs:146-159`), so one session is sent back at most once today; `r3` in M8c.4's test is synthetic. M8b's `AgentRole::Scout` is labelled, not dropped, if it ever appears on a task.
- R10 (decision 23). `RunState::label()` is snake_case, so the view maps states to its own text.
- R11 (decision 24). The fast path has no orchestrator (spec §16.1), which the no-orchestrator toast already covers; named.
- R12 (decision 25). `open_conversation` does what M6.5's shipped `toggle_conversation` does — clear `conversation_follow`, `conversation.open`, `sync_conversation_mode()` — rather than setting the keymap mode directly, and lives in `app/conversation.rs`. M6.5's `follow_focus` (its review M3) re-opens the view on the focused window when focus changes; that is accepted as M6.5's settled rule and pinned by `a_run_view_conversation_follows_focus_like_any_other`. Stopping the follow for run-view conversations would touch `conversation.rs` (595 lines) and was not chosen.
- R13 (decision 26). `C-b x`, `C-b X` and `C-b R` opened `Confirm`/`Remove` dialogs (or, for an exited window, sent `Restart` at once) for a focused headless window, which the daemon then refused. They now toast `control_refusal`'s text and open nothing (M8c.6). Offering the control was itself against the watch-only rule, even though the daemon refused it.
- R14 (decision 27). Already true on main (`keymap.rs:144-149`); `keymap.rs` is untouched and the test is a pinning test, green from the start.
- R15 (decisions 29 and 35). Spec §16.4 now specifies one labelled field per row itself; decision 35's first bullet is resolved, not overridden.
- R16 (decision 31). Local `hh:mm` for raw stamps via `local_hhmm` and `App.utc_offset_secs` (ruling above).
- R17 (new decision 31a). M8b's "Produces for later milestones" promised ten fields to this view. Shown: `path` and `triage` (the fast-path `gate` row), `CheckInfo.decider_summary` (`fixing`), `size_check` (`tries`), `block_source` (the blocked stage text). Not shown, with reasons: `phases` (M9.5), `decider_usage` (in `usage.by_role`), `profile_source` and `promote_requested_at` (their lines already arrive in `RunInfo.attention`), `summary_source`.
- R18 (decision 32). A fast-path run starts `running` with `approved_by = "fast path"` and has no gate; the gate keys say so.
- R19 (decision 33). Ruling above: `route_spec`, `policy` for runtime, strength, effort and model, and only the changed fields replaced. `AmendTask.route` replaces the whole spec route (`run/edits.rs:446-448`), which is why the form must send the spec back rather than a partial one. The size may already have been raised by M8b's cross-check; the engine refuses an amend below it.
- R20 (decision 34). `RunReply` gained `Triaged`, `Profile` and `Stats` (M8b); `on_run_reply` ignores them. A `run edit` refusal can be several lines (`engine/requests.rs:296-298`); the form and toasts show the first line and ` (+{n} more)`.
- `attention` row. `RunInfo.attention` already holds one line per blocked task (`snapshot.rs:127-141`); `+{n} more` counts the other blocked tasks plus the non-blocked-task lines, so nothing is counted twice.
- `diff` row. `TaskInfo.diff` is measured when a task merges or ends with a recorded head (M8b decision 32), never while it is live; the mockup's `t2` is a rendering fixture.

**Stale names fixed** (old → shipped)

- `main` at `2cb7e3c` → `a910e18` (the M8b PR head, `da54149` + a Linux build fix).
- `app/mod.rs` (586, "592 after M6.5") holds `on_daemon` → `app/daemon.rs` holds it (the `DaemonMsg::Run` arm, `:139-140`); `app/mod.rs` is 537.
- `keymap.rs` 570 → 275 (tests in `keymap_tests.rs`, 495); `mouse.rs` 212 → 226; `ui/modal.rs` 161 → 162; `pty-smoke.py` 1572 → 1797.
- `App::rows` in `app/runs.rs` → in `tree_input.rs:11`; every `tree::build` caller is named (`tree_input.rs:12, 102, 209, 218, 237, 252, 287`; `mouse.rs:142, 179`; `app/windows.rs:44, 130`), and `focus_relative`'s expanded order (`app/windows.rs:44`) builds with `build_with_runs` and a default state, not `self.rows()`.
- The sidebar click's `Window | Subagent` arm calls `self.focus(id)` directly (`mouse.rs:157`) → routed through `activate_tree_node`.
- `open_conversation` sets `keymap.set_conversation_mode(true)` → `conversation.open` then `sync_conversation_mode()`, clearing `conversation_follow`, in `app/conversation.rs`.
- `ui/dialog.rs`'s `LABEL_WIDTH`/`MARKER_WIDTH` were private → `pub(crate)`.
- `proto::request::{APPROVE, REJECT, EDIT}` → `proto::run_wire::request::…`.
- `RunReply::{Snapshot, Done, Refused, Started, ConfirmNeeded, ToolResult}` → also `Triaged`, `Profile`, `Stats`.
- `AgentRole { Orchestrator, Worker, Reviewer }` → also `Scout`.
- `AgentRoundInfo.round` "a worker's 1 unless M8a appends per bounce" → a worker's equals its `session`; the snapshot has `session_id` and `last_event`, and no `rate_limited_until`.
- `RunInfo` listed M8a's first fields → also `root`, `revision`, `base_branch`, `base_sha`, `run_branch`, `run_head`, `worker_sandbox`, `unconfined_checks`, `trusted_project`, `rate_limits`, `report_path`, `outcome`, and M8b's `path`, `triage`, `promote_requested_at`, `profile_source`, `usage`, `scouts`.
- `approved_by` `user`/`--yes` → also `fast path` (`run::triage::FAST_APPROVED_BY`).
- `run/model.rs` `AgentRound`, `TaskEvent` → `run/model_rounds.rs` (`:45`, `:216`), re-exported by `model.rs`.
- "the `<hh:mm>` helper" → `run/snapshot.rs::clock`, private, UTC; removed by M8c.1, whose times are all raw.
- `TaskInfo.history: Vec<String>` → `Vec<TaskEventInfo { at, text }>` (M8c.1); `AgentRoundInfo` gains `rate_limited_until`.
- "engine's `Approve`, `Edit`, rung 1 and `ApiRetry` handling" in `requests.rs`, `ladder.rs`, `done.rs` → `requests.rs::{start, approve, edit}`, `ladder.rs::take_rung`, and the six `rate_limited_until` sites in `signals.rs` and `review.rs`; `engine/done.rs` is dropped from M8c.1's files.
- `describe` in `run/edits.rs` → `run/edit_log.rs`.
- `DiffStats`, `RunUsage` in `run_info.rs` → `proto/src/adapt.rs`; `PlannerInfo`/`PlannerState` go in a new `proto/src/planner.rs`.
- The version test in `run_tests.rs` → `lib.rs`.
- `scripts/pty_smoke_run.py::run_stage(env, bin_path)` → `run_engine_stage(run_cmd, fail)` (`pty-smoke.py:1715`); M8b's `adapt_stage(run_cmd, fail)` (`:1716`). Stage 11e is `run_view_stage(PtyProc, bin_path, run_cmd, fail)`, called after 11d; it starts its own client, since none is attached after stage 11b's detach, and defines its own conversation bound with the same derivation as `CONVERSATION_VIEW_TIMEOUT` (`pty-smoke.py:1102`) and a `docs/timing-budgets.md` row, since a hyphenated script cannot be imported. Its step 3 no longer says "the attached TUI".
- `app_tests/headless.rs` already has `headless`, `subscribes` and `inputs` (`:8-33`) → reused, made `pub(super)`.
- `Inspection { .. }` literals gain `..Default::default()` via a new `impl Default for Inspection`, so `inspector/panel/tests.rs` (584) stays under 600.
- The file-size table: every count taken on `a910e18`, with budgets that keep every file under 600 (`edits.rs` 577, `conversation.rs` 595, `tree/tests/state.rs` 581, `inspector/tests.rs` 561, `adapt_tests.rs` 576 and `build_tests.rs` 617 are untouched).
- The verification purity `rg` gains `crates/tui/src/ui`, which hard rule 5 names; it is clean on `a910e18`.

**Tasks added or changed**

- New **M8c.11 "Headless conversation polish (daemon)"**, done before M8c.10: the two open follow-ups filed for M8c (a Codex tool call's JSON-looking summary; an interrupted turn that stays `Running`). They are not folded into an existing task because every other task is either the engine's model and snapshot (M8c.1) or client code, and these are fixes to the daemon's conversation building that no other task touches. The summary fix's helper goes in `conversation/summary.rs` and its tests in `summary_tests.rs`, because `build_tests.rs` is already 617 lines.
- M8c.1: the ruled fields, `edit_log.rs`, `model_rounds.rs`, the six rate-limit sites, the unconditional rung 1 push; tests for `told`, the failed-turn and reviewer rate limits, the fast-path start, `route_spec`, raw history times and `rate_limited_until`; a `rg` acceptance for `rate_limited_until =`.
- M8c.2: the arm in `app/daemon.rs`; M8b's three reply variants; multi-line refusals.
- M8c.3: every `tree::build` caller named; `runs_are_ordered_oldest_first`; the run-less headless window; the `Project` key pruning; `focus_relative_reaches_the_orchestrator_even_when_folded`.
- M8c.4: `appended_worker_rounds_map_one_to_one` removed; `a_fresh_session_is_worker_2` states the `round == session` shape; `an_unexpected_role_on_a_task_is_labelled_not_dropped` added.
- M8c.6: `open_conversation` in `app/conversation.rs`; no keymap change (pinning test); the headless control guard and its test; `no_run_view_path_sends_input` also checks `Subscribe`, `Kill`, `Remove` and `Restart`; the follow-focus test.
- M8c.7: decision 31a's tests, the fast-path gate row, the attention count, `local_hhmm_cases`.
- M8c.9: the edit form opens from `route_spec` and sends only the changed fields; `policy_is_a_choice`; multi-line refusals; the fast-path gate toast.
- M8c.10: the stage's signature, its own client, its own bound; the follow-ups it records and marks handled.

**Follow-ups.** Taken: the painter's zip check (M8c.5, unchanged) and the two M8a.7 entries (M8c.11). Not taken, with reasons, under "Follow-ups handled". Filed by M8c.10: the `doing` field's tool target. (Raw history times and `rate_limited_until` were first filed as follow-ups and then taken into M8c.1 by the coordinator's rulings.)

**Left open at the refresh.**

- The daemon-side restored-headless-window gap stays M9's (Risks 9).
- `docs/ROADMAP.md` row 8c is set to `ready` by the coordinator when the branch is created, not by this refresh.

### M8c.1

- **Protocol.** `PROTO_VERSION` was 8 at `crates/proto/src/lib.rs:32` on `3034601` (set by M8b task 2); 8 + 1 = 9. `proto_version_is_eight` is now `proto_version_is_nine`; the derivation paragraph is in `lib.rs`'s doc comment.
- **Files beyond the task's list**, each only because a struct literal must name every field: `crates/proto/src/adapt_tests.rs` (listed as untouched; its one `RunsSnapshot` literal gains `now: 0`, 576 → 577 lines), `crates/daemon/src/run/plan.rs` (`build_run`'s `Run` literal, three fields), `crates/daemon/src/run/engine/dispatch.rs` (the worker `AgentRound` literal, two fields, 587 → 589), and the test literals in `run/edits_tests.rs`, `run/report_tests.rs` and `crates/daemon/tests/run_journal/fixture.rs`.
- **Tests' homes.** The proto tests are in a new `crates/proto/src/run_tests_view.rs`, declared from `run_tests.rs` (as its fixtures are), with one extra test, `planner_state_is_snake_case_and_defaults_to_planning`. The engine tests, `old_run_json_loads` included, are in a new `crates/daemon/src/run/engine/tests/view_fields.rs`. The M8b `run.json` fixture is `engine/tests/m8b_run.json`: written by a throwaway test with the engine `Fixture` and `serde_json::to_string_pretty` on `3034601` before any change (two tasks, one launched worker in an `ApiRetry` streak, history and log entries), then the throwaway test was deleted. It is 618 lines of JSON data, not code.
- **`pub mod edit_log`**, not `mod edit_log`: `PlanEditRecord` is the element type of the public `Run.plan_edits`, which a private module would make a `private_interfaces` warning.
- **`set_rate_limited` on a restored run.** A limit that stays set while `rate_limited_since` is `None` takes `now`, rather than staying `None`. That happens only for a round restored from an M8b `run.json` mid-limit; otherwise the rule is exactly decision 4's.
- **`TaskInfo.history`** had no `#[serde(default)]` on `main`; it has one now, as the Interfaces say. An M8b snapshot's string history does not decode as `TaskEventInfo` (the protocol bump covers it), so `snapshot_without_view_fields_defaults` drops that key with the new ones.
- **Test times.** The engine `Fixture` starts at 2 000, so the brief's example times (100, 130, 200, 500) are taken relative to the fixture's clock; `rate_limited_until = 900` and `snapshot(state, 777)` are literal. The `told` case uses the done gate (a protected file), the path that passes `told = true`; the rung 2 case drives the real kill, exit, `DiffSoFar` and `CreateWindow` to show session 2's new round.
- **Red evidence.** Every new test failed to compile before the change (the fields, types and `set_rate_limited` did not exist). After it, a mutation of each recorded fact was caught: dropping the rung 1 push, the fast-path `approved_at`, `edit_log::record` in `edit`, swapping `describe`'s `dep` words, resetting `rate_limited_since` on every retry, and publishing the plan edits oldest first each failed exactly the test that pins it.

#### M8c.1 review fixes

- **I1: a UTC time in the snapshot.** The brief contradicted itself. Decision 4 says the snapshot formats no time, and the refresh ruling says every time in the view is local. But R17 leaves `promote_requested_at` unshown because its line "already arrives in `RunInfo.attention`", and that line (`Run::promotion_line`, `model_adapt.rs`) was `promotion requested at <hh:mm>; …` in UTC, which M8b's `fast_path.rs` test pinned. Resolved by controller ruling: the attention line now carries no time (`promotion requested; it takes effect when the orchestrator exists (milestone 9)`), and a client formats the raw `RunInfo.promote_requested_at` in local time. For the view that is later tasks' work: R17's "not shown" no longer covers the time, so the gate/attention rendering should show `local_hhmm(promote_requested_at)` beside the line. `anthrex run status` showed the old time through its attention lines, so it now prints `  promotion: requested at <local hh:mm>` itself; `status::render` and `run_block` take the local UTC offset, read by `tui::local_utc_offset_secs` (now `pub`), so they stay pure. `hh_mm` keeps one caller, `run promote`'s reply to a repeated request (`was already marked for promotion at <hh:mm>`, UTC). That is a request reply, not the snapshot; it is filed as a follow-up. `a_promoted_run_publishes_no_clock_time` scans every string of a promoted fast-path run's snapshot for `\d\d:\d\d`.
- **M3: bounded edit-log text.** `edit_log::describe` cuts its text at `DESCRIBE_MAX_CHARS` (300) characters on a char boundary and appends `…`, and replaces control characters with spaces. It stops building once the cap is passed, so a 20,000-edit batch costs no more than a short one.

### M8c.2

- **`run_subscribed` starts `true`.** The flag is `false` only while a refused `Run(Subscribe)` waits for `on_tick`'s retry: `on_send_failed` clears it, `run_subscription` sets it, and so does every `RunReply::Snapshot` (a push proves the subscription is live, so a pending retry is dropped). Starting it `false` would make the first `on_tick` of every `App` built without `lib.rs` send a subscription, which the existing tests that assert an empty or exact tick (`app/tests.rs`, `app_tests/headless.rs`, `lifecycle.rs`, `reconnect.rs`) would all see, as would every other `App::new` outside `lib.rs`. `lib.rs` sends the real first subscription right after `App::new`, as decision 1 says. The retry runs only while connected; disconnected, `on_reconnected` sends it.
- **`run_view` is not added yet.** `RunView { run_id, filter: RunFilter }` needs `tree::RunFilter`, which M8c.3/M8c.4 create; the field arrives with the task that first reads it. `app/mod.rs` is 546 lines after this task (budget 552), leaving room for it, `Modal::EditTask` and the `on_paste` arm.
- **`on_tick`** calls one new `retry_dropped_subscribes()` (in `app/runs.rs`) that chains M6's `retry_dropped_subscribe` with the run retry, so `app/mod.rs` grows one line there.
- **Refusal text.** `app::runs::first_line_and_more` (`pub(crate)`, for M8c.9's form) skips blank lines, so a trailing or leading newline neither leads nor counts. A `Refused` whose message has no non-blank line toasts `<request> refused` rather than an empty toast. A refused `Run(Subscribe)` send is quiet, like a refused window `Subscribe`, instead of the generic "daemon is not responding".
- **Existing tests changed:** `app_tests/reconnect.rs`'s two exact-equality `on_reconnected` assertions now also expect the run subscription.
- **Test-only setter:** `App::set_runs_received_at(Instant)` (`#[cfg(test)]`). `app_tests/runs.rs` has `app_with_runs(windows, snapshot)` (`pub(super)`); `open_run_view` waits for M8c.6.
- The `lib.rs` startup send has no unit test (`tui::run` needs a real terminal); the smoke stage (M8c.10) exercises it.

### M8c.3

- **Existing tests that had to change.** Three test matches over `RowKind` were exhaustive (`tree/tests.rs` `positions_follow_visible_order` and `projects_sort_by_urgency_then_name`, `tree/tests/state.rs:381`), so the new variants did not compile there; each `RowKind::X { .. } | RowKind::Y { .. } => None` arm became `_ => None`, with no assertion changed and no line added (`state.rs` is listed as untouched). `crates/cli/src/tree_cmd.rs` (not in the task's file list) matches `RowKind` exhaustively twice; it calls `tree::build`, which lists plain windows only, so the new arms write nothing.
- **`tree::build` callers.** `self.rows()` borrows all of `App`, so the call sites that go on to mutate `self.tree` with the rows still in hand (`select`, `move_selection`, `repair_selection`) cannot use it; they call `tree::build_with_runs(&self.windows, &self.runs.runs, &self.tree)`, which borrows the fields apart. The acceptance `rg` prints no source file (`tree.rs` itself declares `pub fn build`, which the pattern does not match); it does print three unchanged test files under `app_tests/`, which its `tests*` glob does not exclude.
- **Hiding owned windows at grouping, not in `visible_windows`.** A window a shown run owns is left out when windows are grouped into projects (`tree/runs.rs::group_projects`), so it counts toward neither a project's status and runtime counts nor its existence, and it can match no filter; `tree/rows.rs` is unchanged. A project's shown runs are held in `ProjectGroup::runs`; `ProjectChild` stays `Window` only.
- **`Project` keys and `prune`.** `prune(&windows)` cannot see runs, so `prune_runs` records the roots shown runs name (`TreeState::run_roots`, private) and `prune` keeps a `Project(root)` key that a window or a recorded root names. `prune_runs` itself never drops a `Project` key; the `App` calls `prune_runs` then `prune` after every snapshot and every window list.
- **`focus_relative_reaches_the_orchestrator_even_when_folded`.** As written (one project, folded) it cannot pass: folding `/r/demo` hides the orchestrator's row with window 1, and `focus_relative` returns early on an empty visible order, as `next_from_a_hidden_focused_window_goes_forward` pins. The test puts window 1 in a second, folded project (`/r/alpha`), so the expanded order `[1, 3, 2]` must come from `build_with_runs`: `C-b j` from 1 reaches 3 (plain `build` would give 2) and `C-b k` reaches 2 (the folded `rows()` would give 3).
- **Fixtures.** `run_fixtures.rs` has `gate_fixture` and `three_task_fixture`, plus `pty`, `run_ref` and the `RUN_ID`/`PROJECT`/`GOAL` constants; the three-task fixture also lists the gate fixture's plain shell window 1 (the tests name it). `gemini_fixture` is M8c.7's first need and is left to it. The fixtures are reached crate-wide as `crate::tree::run_fixtures` (a `#[cfg(test)]` re-export in `tree.rs`).
- **Placeholders for later tasks.** A `Run` row is reachable in the project overview now, so it gets real text there: its canvas text is the Interfaces table's (`orchestrator  1/3`, `run 3f9a  0/2`), its glyph `theme::RUN_GLYPH` in `theme::run_color` (both brought forward from M8c.5, as specified), and its inspector and single line show its id, goal, state and progress until M8c.7 and M8c.5 replace them. `Planner`, `Scout`, `Task` and `AgentRound` rows are not built yet; their arms draw a neutral `○` and a short text. Enter or a click on a `Run` row does nothing until M8c.6. A run's sidebar line is `{guides}{▎ when its orchestrator is focused} ◉ {position} {goal}` with `{merged}/{total}` on the right; a blank goal shows the run id (`tree::run_title`).
- **Extra tests** beyond the brief: `build_still_means_no_runs`, `an_orchestrator_window_of_another_run_is_plain_and_positions_nothing`, `two_runs_in_one_project`, `a_run_sits_under_its_exact_project_not_a_prefix` (`/r/demo` vs `/r/demo2`), `a_non_ascii_goal_matches_the_filter_and_an_empty_goal_matches_its_id`, `a_collapsed_project_hides_its_runs`, `toggle_folds_every_new_key`, `a_run_with_an_empty_goal_shows_its_id`, and `a_snapshot_prunes_the_keys_of_runs_that_left` (`app_tests/runs.rs`). `the_filter_matches_a_runs_goal_and_id` also filters on `password`: the brief's `reset` is in the run id too, so it alone could not tell the goal match from the id match.
- **M8c.3 review fixes.** `replace_runs`' selection repair and reveal are pinned in `app_tests/runs.rs` (`a_discarded_runs_selected_row_hands_the_selection_to_its_neighbour`, `a_snapshot_that_empties_the_tree_clears_the_selection`, `runs_arriving_above_the_selection_keep_it_in_the_sidebar`; the reveal is observed through the sidebar viewport, the one it guarantees outside the overview). `shown_runs` drops a repeated run id, keeping the first copy in its `(created_at, run_id)` order (`a_run_listed_twice_is_shown_once`). The brought-forward text is pinned: cancelled tasks out of `run_progress`, `run_short`'s last four characters and the two spaces of `orchestrator  {m}/{t}` (`graph/tests.rs::a_run_nodes_text_is_the_interfaces_text`), `run_title` on a whitespace-only goal, and the sidebar `▎` for a focused orchestrator. The filter test also pins a project matched by name keeping its run. The test-only row builds in `app_tests/tree_interaction.rs`, `overview.rs`, `conversation_follow.rs` and `app/tests.rs` use `build_with_runs` with `app.runs.runs`, so a `select` of a `Run` key now works (`the_select_helper_reaches_a_run_row`). `tree.rs` did not grow.

### M8c.4

- **Files.** The logic is in `tree/run_rows.rs` (504 lines); `tree.rs` gained only `mod run_rows;` and one `pub use` line (552 → 554). `RunFilter` is defined in `run_rows.rs` and re-exported as `tree::RunFilter`. `tree/runs.rs`'s `orchestrator_of` became `pub(super)` so the run view's root carries the same orchestrator the project tree does. The tests would have been 770 lines in one file, so they are split: `tree/tests/run_rows.rs` (tiers, order, rounds, sub-agents; 461) and `tree/tests/run_rows/filters.rs` (filters, folds, the 200-task run, hostile snapshots; 337), the second declared from the first so `tree/tests.rs` gains one `mod` line (565 → 566). `run_fixtures.rs` gained `scout(..)` and `planner(..)`.
- **The root's `position` is `None`** in the run view: the number keys are the project tree's (decision 10), and nothing in the run view reads a position.
- **Both filters at once.** The text filter and `f` compose: a node is kept when it passes both itself — by its own match or an ancestor's descendant-keeping match — or when it leads to a node that does; the root is always kept. `the_text_filter_keeps_a_matching_task_with_all_its_rounds` pins `proto` + `codex`.
- **What `f` matches.** Tasks, agent rounds and scouts only, as decision 21 lists them; planners (and the root) are kept only as ancestors, including under `Runtime(r)` although a planner has a route. A "live" round for `Running` is a display round with no end, so the earlier pieces of a bounced worker session are not live. A live scout is `starting` or `working`.
- **Sub-agents and filters.** Sub-agents follow their node through `f` (shown when the node passes it). Under the text filter they behave as under a plain window: all of them below a match, only the matching branches otherwise, and a matching sub-agent keeps its round, task and planner as ancestors. A planner's sub-agents come after its tasks.
- **Folds while filtering.** As in the project tree, folds are ignored while either filter is on, so a match is never hidden by a fold; with no filter, a folded node (the root included) hides everything below it.
- **The text filter reads `graph::content_text`**, so it follows the node text M8c.5 writes. Until then an agent round's text is the placeholder `#{number}`, so `worker` or `codex` do not yet match a round by name; M8c.5's `{round_label} {runtime}` makes them match with no change here.
- **`last` on reviewer rounds.** Every reviewer (and unexpected-role) round is its session's only display round, so its `last` is `true`; "only `r3` has `last`" in the brief's test is read over the worker's pieces.
- **Hostile snapshots** (`hostile_snapshots_never_panic_and_keep_keys_unique`): `sent_back_at` is taken sorted, and a bounce earlier than the session's start (or than the previous bounce) starts its piece at that floor, so pieces never run backwards; duplicate bounces give zero-length pieces, each its own number. A repeated `(role, session, display round)` keeps its earliest copy; a repeated task id, planner epic or scout id keeps its first copy; a window's sub-agents are drawn once, under the first node in row order that names it, and never under the orchestrator's window (decision 12). A consequence: if that first node is filtered out, a later node naming the same window shows none of them. Deps naming unknown tasks, a reviewer that started before any worker, and a planner no task belongs to give ordinary rows. The case runs every filter with five texts and checks every key is unique.
- **Extra tests** beyond the brief: `scouts_order_by_start_then_id` (the brief's fixture has the same order by start and by id, so an id-only sort survived it), `a_worker_sorts_before_a_reviewer_on_a_tie`, `the_root_is_the_run_with_its_orchestrator` (also pins the guides), `a_scout_with_a_listed_window_shows_its_sub_agents`, `the_text_filter_reaches_sub_agents_through_their_round`.
- **Red and mutation evidence.** Every new test failed to compile before the change (`run_rows`, `RunFilter`, `display_rounds`, `round_label` did not exist). After it, 30 mutations of `run_rows.rs`, each restored from a backup, were each caught by the test that pins the behaviour: tier order, scout order, wave and plan order (each half), planner placement and empty-planner order, the orphan epic, the round split, `last` on every piece, numbering workers from `AgentRoundInfo.round`, dropping or mislabelling an unexpected role, sub-agents under every piece, a missing window's fallback, ancestor keeping, `f`'s and the text filter's descendant keeping, each `f` predicate (running scouts, running rounds, blocked, runtime scouts), folds ignored and folds honoured while filtering, task order by id text, the worker-first tie, and each hostile guard (round, task and window dedup, bounce sort, bounce clamp).
