# anthrex Foundation: follow-ups carried into later milestones

The old "plan 2 to 5" numbering in the ledger lines below predates `docs/ROADMAP.md`. Old plan 2 (agent status) is now milestone 3, old plan 3 (worktrees) is milestone 5, old plan 4 (persistence) is milestone 6, and old plan 5 (CI) is milestone 2. The section "Assignment to milestones" at the end is the authoritative mapping.

Generated from the execution ledger of `2026-09-17-anthrex-foundation.md` before the scratch workspace was deleted. Every line is a finding that a reviewer raised and that was deliberately deferred, or a decision ("Ruling") the controller made during execution.

## Deferred and parked findings

- Task 2: minor (deferred): every_daemon_message_round_trips covers 3/13 ClientMsg and 1/8 DaemonMsg variants (plan-mandated test set)
- Task 2: minor (deferred): no test asserts serialized case for Status/ClientKind/HookSource rename_all (plan-mandated test set)
- Task 3: minor (deferred): redundant `use tokio::io::AsyncWriteExt` in codec tests
- Task 4: minor (deferred): Linux runtime_dir branch and its fallback untested on macOS host
- Task 5: minor (deferred): shell launch test does not assert env.len()==4; redundant test import
- Task 6: minor (deferred): no fn-level doc on status::next; _runtime unused until plan 2
- Task 7: minor (deferred): parser Mutex uses lock().unwrap() everywhere — a reader-thread panic would poison it (plan-mandated)
- Task 7: minor (deferred): Window has no Drop that signals the child; manager owns that (plan-mandated)
- Task 8: minor (deferred): shutdown() waits on all windows but only signals the ids captured at start (race with concurrent create)
- Task 8: minor (deferred): manager tests do not cover focus/Bell/InputSent wiring or empty rename (plan-mandated test set)
- Task 8: minor (deferred): create() holds the Inner mutex across Window::spawn (fork/exec + 2 threads)
- Task 9: minor (deferred): a non-Hello first frame is dropped silently without an Error (plan-mandated); HookEvent reply path untested
- Task 9: minor (deferred): ListWindows replies with the WindowsChanged variant (plan-mandated)
- Task 10: minor (deferred): TOCTOU between stale-socket probe and remove_file; blocking Command::spawn inside async ensure_daemon; 2 clippy warnings in server.rs (pre-existing, for final review)
- Task 11: minor (deferred): inconsistent "cannot reach the daemon" vs "no daemon is running" phrasing
- Task 12: minor (deferred): is_prefix uses strict modifier equality; 6 of 10 Command variants and End key unasserted in tests (plan-mandated test set)
- Task 13: minor (deferred): send() can suspend the UI loop if the daemon stops reading and the 256-slot queue fills (doc note at least)
- Task 13: minor (deferred): no regression test for the drop-closes-socket behaviour (would need public API)
- Task 14: minor (deferred): 2 clippy collapsible_if suggestions in app.rs (plan code) — sweep with the server.rs ones at final review
- Task 14: minor (deferred): scroll_to_live only on passthrough keys (ToggleSidebar leaves view scrolled); debounce test uses a real 35 ms sleep; stale pending_focus could override a valid focus
- Task 14: minor (deferred): pending_focus is a single last-write-wins slot shared by request_focus, Created and unsized focus()
- Task 15: minor (deferred): no scrolling or "+N more" indicator when cards overflow the sidebar (focused card can be off-screen); dirs::home_dir() per frame; confirm modal message not wrapped; footer counts only working/attention
- Task 16: minor (deferred): unconditional redraw every 100 ms tick
- Task 16: minor (deferred): ratatui::restore() runs twice on the panic path (idempotent); panic hook stays installed after run returns
- Final: minor (deferred, plan 4): a shutting-down daemon unlinks the socket path unconditionally after its kill grace and can delete a replacement daemon's socket — same family as the stale-socket TOCTOU; needs the lifetime lock file
- Final: parked — a dropped Subscribe strands the client (focus() mutates before send; I8 early-return blocks re-subscribe of the same id) — Ruling: real but needs a full 256-slot queue at that instant; defer to plan 4 with reconnect. Cost if wrong: user must detach/re-attach.
- Final: parked — per-window input queue bounded by chunk count (256) not bytes; one Input frame may be 16 MiB — Ruling: real, defer to plan 2 (cap bytes or split pastes). Cost if wrong: memory growth from a hostile/huge paste into a non-reading child.
- Final: parked — after a caught parser panic, a second Exited overwrites exit reason unpublished, child not signalled — Ruling: real, rare; defer to plan 2. Cost if wrong: stale exit reason, orphan child until removed.
- Final: parked — bind-then-chmod window on the socket in a shared sticky dir — Ruling: real, only with an explicit override into /tmp; defer to plan 4 (umask around bind). Cost if wrong: brief window where another local user could connect.
- Final: parked — C-b Q with a dropped Shutdown quits silently leaving the daemon running — Ruling: real, rare; defer to plan 4. Cost if wrong: user must run `anthrex daemon stop`.
- M4.5 final review: the git watcher registers recursively (`watch(root, RecursiveMode::Recursive)` in `crates/daemon/src/git/watch.rs`), and `DENY_COMPONENTS` filters *events*, not *registration*. On inotify that walks the tree at registration and takes one descriptor per directory, including every directory under `target/`, `node_modules/` and `.next/` — tens of thousands in a Rust checkout with a populated `target/`. On a host still at `fs.inotify.max_user_watches = 8192` the watch fails outright, and the root falls back silently to the 30-second poll with one warning; M5 multiplies the count by the number of agents, since each gets its own worktree. Distinct from the CPU risk the debounce and the circuit breaker already cover: neither sees an event that was never registered for. Not fixed in M4.5 — the fix is a non-recursive watch plus a pruning walk that applies the deny list at registration time, which is real work of its own. Was assigned to M6, which did not implement it (whole-branch-review m11, fix wave 12). **Status 2026-09-22, branch `m6-git-config`: deferred deliberately, with the list it needs already in place.** That branch added the `[git]` table the spec assigns to M6, so `git.ignore` now exists and extends `DENY_COMPONENTS` at *event* time (`watch::accepts`). The walk that would apply the same filter at *registration* time was declined there on the merits, not forgotten: it is a mitigation with no observed symptom, it is the largest piece of that milestone's remaining scope, and adding `ignore` to the event filter is complete and useful on its own — the walk consuming the same list later is a purely additive change to `watch::build`. **The descriptor risk is unchanged by that branch**: `crates/daemon/src/git/watch.rs` still calls `watcher.watch(root, RecursiveMode::Recursive)`, so a directory the filter rejects still costs a descriptor, and M5's one-worktree-per-agent still multiplies the count. No longer assigned to "whichever milestone next touches the git registry or the config crate's key list" — that milestone was `m6-git-config`, and it declined this consciously. It is unassigned, and stays real work of its own; not a fix-wave-sized change. Whoever picks it up: `watch::build` already takes the `ignore` slice and its doc comment names this item.
- M3 final review: crossterm 0.29 ends a paste event at the first literal `ESC[201~`; the probe input `ESC[200~aESC[201~b\rESC[201~` produced `Paste("a")`, `Key('b')`, then `Enter`. After `EventStream` splits the input, the intended clipboard boundary is unrecoverable. M3 sanitizes only text delivered as one paste event and makes no end-to-end clipboard guarantee. Define a reliable terminal-input policy in M7 without timing filters or silent changes to ordinary key semantics.

## Rulings made during execution

- Ruling: T15 footer assertion shortened to "2 agents · 1 working" — the 30-col sidebar cannot show the full summary; spec fixes sidebar width at 30 — costs nothing if wrong (test-only).
- Task 3: Ruling: reviewer flagged partial-header EOF (1-3 bytes then close) returning Ok(None) — plan-mandated code; accepted as a real defect, fix in round 1 (read first byte with read(), 0 => None, else read_exact the rest) — cost if wrong: none, strictly more precise error reporting.
- Task 3: Ruling: commit trailer said "Claude Haiku 4.5"; controller amended HEAD to the required "Claude Fable 5.1" trailer (metadata, not reviewed code) and force-pushed — cost if wrong: none.
- Task 9: Ruling: reviewer's Important "bounded out_tx can stall a stalled client's request loop, delaying Bye" (plan-mandated) — accepted in part: blocking a client that has stopped reading is correct backpressure and self-heals when the socket dies (writer fails, channel closes, loop exits); the only real gap is Bye on shutdown, fixed in round 1 by sending Bye with try_send (best effort). Cost if wrong: a stalled client sees a delayed disconnect, nothing else.
- Task 10: Ruling: reviewer's Important "unscoped chmod swallow weakens 0700" — accepted: chmod failure is tolerated ONLY when the directory is not owned by the current uid (warn in the log); a failure on a self-owned dir stays fatal; add a test using a root-owned parent (/tmp). Cost if wrong: an override into a shared dir starts with a warning instead of failing.
- Task 10: Ruling: reviewer's Important "cleanup skipped when serve() errs" (plan-mandated) — accepted: capture serve's result, always run manager.shutdown() + socket/pid removal, then return the result. Cost if wrong: none.
- Task 10: Ruling: reviewer's Minor "detached Child never reaped" is promoted into the fix round because Task 16 calls ensure_daemon from the long-lived TUI, where a daemon that exits first would linger as a zombie; fix = reap in a background thread. Cost if wrong: one idle thread.
- Task 11: Ruling: reviewer's Important "--dir canonicalized for every command" (plan-mandated) — accepted: resolve the directory only in the arms that need it (new, and attach later); daemon/ls/kill/rm must work from a deleted cwd. Cost if wrong: none.
- Task 11: Ruling: reviewer's Important "handshake duplicated three times" (plan-mandated) — accepted: daemon stop/status reuse CliClient (adds send + wait_close; status distinguishes not-running from incompatible); this also makes daemon_version read, so the allow(dead_code) goes. Cost if wrong: none.
- Task 12: Ruling: reviewer's Important "auto-repeat of the prefix key while pending is treated as a literal prefix" (plan-mandated; inert until keyboard-enhancement flags are enabled) — accepted: a KeyEventKind::Repeat of the prefix while pending keeps pending and returns AwaitPrefix. Cost if wrong: none.
- Task 13: Ruling: reviewer's Important "reader task + fd leak when Connection is dropped" (plan-mandated; inert for the one-connection client but real once plan 4 adds reconnect) — accepted: Connection keeps the reader JoinHandle and aborts it in Drop. Cost if wrong: none.
- Task 14: Ruling: reviewer's Important "focus() before the first size report sets focused but never emits Subscribe, and ensure_focus then short-circuits" (plan code) — accepted: with term_size unknown, focus() records pending_focus and returns without setting focused; ensure_focus resolves it on the first size report. Cost if wrong: none.
- Task 15: Ruling: reviewer's Important "sidebar hit_test maps spacer/footer rows to undrawn cards when the list overflows" (plan code) — accepted: render and hit_test share one list-area helper; hit_test only returns cards that are fully drawn. Cost if wrong: none.
- Task 16: Ruling: the plan's Step 4 interactive smoke checklist cannot be run by a headless implementer — replaced for the implementer by a scripted PTY-driven smoke (python pty); the visual checklist is handed to the user in the final message. Cost if wrong: a purely visual defect slips to the user's first run.
- Task 16: Ruling: reviewer's Important "a panic in the event loop leaves mouse capture + bracketed paste enabled" (plan code) — accepted: a Drop guard disables both and restores the terminal on every exit path including unwinding. Cost if wrong: none.
- Final: Ruling: reviewer disagreed with the Task 10 chmod ruling (tolerating any foreign-owned dir is the attack case) — reviewer is right; narrowed to root-owned sticky parents only, else fatal; socket chmod 0600. Cost if wrong: overrides into non-sticky foreign dirs now fail to start.
- Final: Ruling: C1 (blocked PTY write freezes daemon) was in the plan's own code and missed by the Task 8/9 rulings on backpressure, which only considered daemon->client; fixed with per-window writer thread + bounded queue + client try_send. Cost if wrong: input dropped when a program does not read stdin for a long time (reported to the user).
- Final: Ruling: minors M1 (`--` before prompt), M2 (clippy), M3 (commit smoke script) included in the single fix wave; all other minors stay deferred to plans 2-5 as the reviewer triaged (SIGHUP/killpg, lock file, sidebar overflow before plan 3, mouse encoding, paste sanitising, redraw coalescing, log rotation).

## From the whole-branch review: recommended for later plans

- Plan 2: kill by process group (SIGHUP then SIGTERM via killpg) so interactive shells die promptly and agents' subprocesses are not orphaned; SIGWINCH jiggle on attach so apps repaint; wheel scrolling for alternate-screen apps without mouse mode; check `mouse_protocol_encoding()` instead of assuming SGR; decide on kitty keyboard flags; strip `ESC[201~` from pasted text; cap the input queue by bytes.
- Before plan 3: sidebar overflow (scrolling or "+N more"), since many agents is the product.
- Plan 4: a lifetime lock file (flock) to close the stale-socket TOCTOU and the unconditional socket unlink at shutdown; end-to-end `lifecycle::run` start/stop test; reconnect built on `try_send`; umask around bind; log rotation; handshake read timeout.
- Process: review the plan's own code listings adversarially before execution; add concurrency tests for a non-reading child, a lagged receiver and an alternate-screen snapshot (now present) to every future plan that touches the data path.
- A human should run the visual checklist in plan 1's Task 16 Step 4 with real `claude` and `codex` windows before plan 2 starts.

## Assignment to milestones

Each open item above is closed by exactly one milestone. Its brief lists the item under "Follow-ups handled".

| Milestone | Items it closes |
|-----------|-----------------|
| M2 CI | Nothing from the ledger. CI enforces `-D warnings`, which keeps the clippy sweep from regressing. |
| M3 Agent status | Kill by process group: SIGHUP, then SIGTERM, then SIGKILL via `killpg`. Cap the per-window input queue by bytes (1 MiB) as well as chunks. After a caught parser panic: signal the child, keep the first exit reason, and publish. Strip bracketed-paste markers from text delivered as one paste event. `status::next` doc comment. The Task 2 test-coverage minors: every message variant round-trips, and the serialized case of `HookSource` is asserted. |
| M4 Project tree | Sidebar overflow: the tree scrolls to keep the selection visible. Hit-testing shares geometry with rendering. |
| M5 Worktrees | The Task 8 minor "create() holds the Inner mutex across Window::spawn": `create` runs `Window::spawn` off the lock. |
| M6 Persistence | The lifetime lock file, the unconditional socket unlink at shutdown, and the stale-socket TOCTOU. Umask around bind. Reconnect, including re-subscribe after a dropped Subscribe. `C-b Q` confirms that the shutdown was delivered before quitting. End-to-end `lifecycle::run` start and stop test. Log rotation. Handshake read timeout. **Not closed** (whole-branch-review m11, fix wave 12): register git watches non-recursively with a pruning walk, so a directory the deny list already rejects does not still cost a descriptor. Half of what this row used to claim landed later on branch `m6-git-config`: the `[git]` table, `ignore` included, exists and filters events. The pruning walk itself is deferred, unassigned, and detailed above; the descriptor risk is unchanged. M6's own actual scope is the eight items above it in this cell. |
| M7 Split panes (dropped 2026-10-10; the four terminal-input items moved to milestone 9.11) | SIGWINCH jiggle on attach so full-screen apps repaint. Wheel scrolling for alternate-screen apps without mouse mode. Check `mouse_protocol_encoding()` instead of assuming SGR. Define terminal-input policy for embedded bracketed-paste end markers before crossterm splits the intended clipboard boundary. |
| Not scheduled | Kitty keyboard protocol flags. Coalescing redraws. The remaining test-coverage minors from tasks 4, 5, 8 and 13. |

### M3 file organization observation for M4

- `crates/tui/src/app.rs` was already 814 lines before M3.12 and is now 880.
  When M4 changes client state for the project tree, consider a focused split
  of input normalization/tests from application transitions. M3 keeps its
  small pure paste helpers local and does not perform an unrelated refactor.

### M3 verification observations for M6

- In isolated real-Codex probes, `anthrex daemon stop` stopped the owned daemon,
  but the attached TUI process did not exit within the helper's three-second
  deadline. The helper killed and reaped only its own client PID. Reproduce and
  define the expected disconnected-client behavior alongside M6 reconnect and
  lifecycle tests; this observation does not establish the root cause.

### M4.6 observation for a later tree milestone

- `TreeState::overview` — the one-dimensional viewport the aligned-row overview
  scrolled with — is still maintained by `set_tree_viewports` and
  `reveal_tree_anchor`, but nothing reads it any more: M4.6.7 replaced that
  renderer with the graph, which pans through `App::graph_pan` instead.
  Removing it means changing `set_tree_viewports`'s signature, which every
  existing viewport test calls, so M4.6.7 left it in place rather than edit
  tests outside its brief. Delete the field, the `overview_rows` parameter and
  the anchor's second `reveal` together in the milestone that next touches
  tree state.

## From milestone 4.6's final review (2026-09-21)

- **A sub-agent label change escapes the reveal gate.** A tier's width comes from
  `content_text`, which for a sub-agent is `kind: label`. A label that changes while the
  agent runs resizes its tier without changing any row key, so the boxes move sideways
  and no reveal fires; the selection can drift partly off-screen until the next real
  change. It cannot mis-draw or panic — `view_of` re-clamps the pan. Tightening the gate
  to compare `(key, content_text)` would close it, at the cost of re-introducing some of
  the snap-back the gate was added to remove. Assigned to milestone 7, which already
  takes the `TreeState::overview` follow-up. (Milestone 7 was dropped on 2026-10-10; this item is unscheduled.)
- **`crates/tui/src/ui/mod.rs` is 599 lines against the 600 rule**, about 470 of them its
  test module. The next `ui` change has nowhere to land. Milestone 7 removes `main_inner`
  and restructures the layout, so it splits the file there. *Resolved before M8c (recorded
  by M8c.10):* `ui/mod.rs` was 99 lines on `a910e18` and is 100 at M8c.10.
- **The painter's zip invariant is only `debug_assert`ed.** `paint` pairs `layout.nodes`
  with the row list positionally. The invariant holds by construction today; in release a
  future divergence would paint labels onto the wrong boxes rather than dropping a node.
  *Handled in M8c.5:* the painter compares each row's key with its node's and skips a
  mismatched node, with every edge touching it
  (`graph/paint/tests/runs.rs::a_row_that_disagrees_with_its_node_is_skipped`).

## From milestone 5's whole-branch review (2026-09-21)

- **Decision 25's "beside the loop" half is unpinned for `Remove`.**
  `list_is_answered_while_a_create_is_running`
  (`crates/daemon/tests/server/worktree.rs`) proves a `ListWindows` is answered while a
  `CreateWindow` is still running, held open by a real `post-checkout` git hook that
  writes a marker and sleeps for a fixed few seconds — so the test can wait for the
  marker and know git is *provably* still inside `worktree add` before it asks whether
  the connection loop is free. There is no equivalent test for `Remove`, and a refactor
  that awaited `remove_with_worktree` inline in `handle_client` — reintroducing exactly
  the stall decision 25 exists to prevent — would pass the whole suite today.
  Investigated during the fix wave that added this entry: the reason isn't that nobody
  wrote the test, it's that the technique the create-side test uses does not carry over.
  `git worktree remove` fires no hook at all (checked against git 2.50.1: a repository
  with every plausibly-relevant hook name wired up to log its own invocation stayed
  silent across a `worktree remove`), so there is nothing to plant a sleep in the way
  `post-checkout` holds `worktree add` open. The daemon's own `git` invocation is not
  test-injectable either — `manager::git()` is a hardcoded `OsStr::new("git")`, unlike
  `daemon::project::detect_roots_with`'s injectable program, so an integration test
  cannot substitute a slow wrapper for the real removal's git calls without a new
  production seam. `kill_and_await_exit`, the one phase of a removal that touches no
  git, cannot be held open either: it waits on a real `SIGKILL`, which a child process
  cannot delay or catch. Closing this needs one of: a test-only override for the git
  program `WindowManager::remove_with_worktree` runs (mirroring
  `project::detect_roots_with`'s `_with` twin), or a fixture child that can be told to
  ignore its own reaping for a bounded window so `kill_and_await_exit`'s wait becomes
  the held seam instead. Either is a real production-code change, not a test-only one,
  so it is left for whichever milestone next touches `manager::remove` rather than
  folded into a fix wave scoped to minors. See
  `crates/daemon/tests/server/worktree.rs`'s
  `a_worktree_removal_runs_to_completion_after_its_client_disconnects` for the same
  held-seam gap on the *no-abort* guarantee (decisions 17 and 25 together), documented
  in place since that test at least has a weaker, poll-based substitute; this one has
  no test at all.

## From milestone 5's final review (2026-09-21)

- **The force-remove dialog clips on a short terminal, hiding every option but the
  destructive one.** `crates/tui/src/ui/dialog.rs:338` with `:282-293`. The box grew from 11
  rows to 13 — wave B's `wrap()` continuation gives a long path three lines, and wave C raised
  `FORCE_MAX_LINES` from 4 to 6 — while `render_box`/`centered` clip the bottom silently. On an
  11-row terminal the only visible choice is `f`, which deletes uncommitted work; `k` (keep the
  worktree) and `n` (cancel) are both off-screen. Measured with `TestBackend` at both `cdd3681`
  and `4c79d5d`.

  Not merged as a blocker because the clipping class is pre-existing (it bit at ≤10 rows before
  this milestone, ≤12 now), `Esc` still cancels, and a terminal that short is barely usable. But
  a dialog whose only visible option is the destructive one is the wrong failure mode for this
  particular screen. The real fix is in `render_box`/`centered` — clip predictably or scroll,
  rather than silently dropping the bottom — which is a general rendering change deserving its
  own review, not a patch at merge time.

  This is a genuine cross-wave interaction: neither wave B's change nor wave C's is wrong alone,
  and no per-wave review could have seen it.

- **`wrap` measures characters while the box is sized in display columns.**
  `crates/tui/src/ui/dialog.rs:55-112`. A CJK path renders an over-wide, misaligned box. No text
  meaning is lost. Pre-existing, and the same unit mismatch the `TextInput` cursor work fixed
  elsewhere in this milestone — worth closing with the same `unicode-width` treatment.

- **Nothing runs `--ignored`, so the 45-second create-budget test proves nothing in CI.**
  Milestone 5 replaced it with a ~2-second injectable-deadline version and kept the slow one
  `#[ignore]`d as the only full-scale proof. The final review's judgement: little real coverage
  was lost, because `CREATE_WORST_CASE`/`REMOVAL_WORST_CASE` are computed from the daemon's own
  constants so an always-run assertion catches a timeout regression — and the ignored test's
  unique coverage would not catch a new blocking term anyway, since `SLOW_CREATE_DELAYS` is a
  fixed list. Either add a slow-test CI job that runs `--ignored`, or delete the test. An
  ignored test that nobody schedules is documentation wearing a test's clothes.

- **Two blind spots in the dirty check that are outside the paused-operation family**, both
  pre-existing and both accepted knowingly. `--skip-worktree` hidden edits — git itself has the
  identical hole — and per-worktree reflog loss on a detached HEAD, which is narrow because the
  daemon always creates on a branch. The paused-operation family itself is now complete: the
  final review tested twelve scenarios against `sequencer` and `BISECT_LOG`, found no reachable
  false positive, and verified per-worktree scoping (a cherry-pick paused in the main checkout
  correctly leaves a linked worktree clean).

## From milestone 6.5's review (2026-09-21), left open by fix wave 4

- **`u32::MAX` is never handed out as a window id.** `crates/daemon/src/manager/restore.rs`
  (`next_id = next_id.max(id.saturating_add(1))`) together with
  `crates/daemon/src/manager/create.rs:191` (`id.checked_add(1)`): a restored record already
  holding id `u32::MAX - 1` makes `next_id` saturate to `u32::MAX`, and `create`'s
  `checked_add(1)` then refuses — "no window ids remain" — one id before the space is actually
  exhausted, because saturation cannot tell "the max id is taken" from "the max id is one below
  the ceiling". This is deliberate, not an oversight: `crates/daemon/src/state.rs` reasons the
  saturation through explicitly (saturating at `u32::MAX` so the next window-creation attempt
  notices the collision and fails loudly) and names the consuming-side check — which
  `manager/create.rs`'s `checked_add` guard supplies — as later work. Not worth fixing on its
  own: it costs exactly one id, once, only after `u32::MAX - 1` ids have already been handed out
  in one daemon lifetime (a restart reclaims the whole space), and closing the one-id gap would
  mean `next_id` growing past `u32::MAX` itself, which is not representable. Recorded here per
  the M6.5 review's Minor 2, rather than changed.

## From the M6 persistence flake investigation (2026-09-21), deliberately deferred

A wall-clock timing audit (distilled into `docs/timing-budgets.md`) fixed the sites with a
negative or thin margin and a cheap seam. Two more sites have a real margin problem but no
cheap fix, and are recorded here rather than patched with a wider number, which would either
do nothing (the first) or silently delete the property under test (the second).

- **`later_size_changes_are_debounced_into_a_resize`** (`crates/tui/src/app/tests.rs:137`,
  the test for `RESIZE_DEBOUNCE` = 30ms, `crates/tui/src/app/mod.rs:16` — updated from
  `app_tests.rs:125`/`app.rs:14`, whole-branch-review m11: commit `8e2abc2` on the M6
  branch deleted both of the original paths via task M6.9's `git mv`). The tightest bound in
  the workspace: `set_terminal_size` stamps `Instant::now()` and the very next statement
  asserts `on_tick()` still sees the debounce as unexpired. There is no I/O and no
  subprocess between the two lines — the entire budget is scheduler slack, so a 30ms
  deschedule between two adjacent statements fails it. **Never observed failing** in this
  investigation's runs, but margin analysis alone makes it the highest-risk site in the
  repo. The real fix is to stop measuring the wall clock at all: give `App` an injectable
  `now: fn() -> Instant` (or a small `Clock` trait, defaulting to `Instant::now`), the way
  `ManagerConfig` already injects `operation_timeout`/`cleanup_timeout`/`kill_grace` in the
  daemon crate, and have the test advance a fake clock explicitly instead of sleeping past a
  real debounce. That also de-risks `TOAST_TTL` and `DOUBLE_CLICK` testing later, so it is
  worth doing as one small piece of TUI work rather than folded into a test-only patch.
  Estimated 1–2 hours. Not fixed here because it is production-code surgery in a different
  crate from the rest of this work, not a test-file change.

- **The four `< 100ms` lock-latency assertions** — `a_program_that_ignores_stdin_never_blocks_write_input_or_list`
  in `crates/daemon/tests/manager.rs` (three call sites, currently around `:238`, `:325`,
  `:333` — updated from `:215`, `:302`, `:310`, whole-branch-review m11) and
  `a_slow_worktree_create_does_not_block_the_manager` in
  `crates/daemon/tests/manager_worktree/admission.rs` (currently around `:191`, plus a
  second `< 1s` bound around `:212` covering a PTY spawn). These are **load-bearing**: the
  property under test is that `list()`/`write_input()` never wait on the manager lock while
  a sibling operation is deliberately stalled (a full PTY write buffer, or a real
  `git worktree add` stuck inside a 2s `post-checkout` hook) — design requirement C1 and the
  lock-discipline rule in AGENTS.md hard rule 2. Widening the bound would not fix a flake
  here, it would quietly delete the requirement: a 500ms bound still "passes" if the code
  regressed to blocking on the lock for 400ms, which is exactly the bug this test exists to
  catch. **Not fixed here** because the honest fix is real instrumentation, not a wider
  number: give `WindowManager::list`/`write_input` a way to report whether they blocked on
  the contended mutex (e.g. a `try_lock`-first path, or a counter of contended acquisitions
  incremented only when the fast path missed) and assert on that instead of on wall-clock
  time. None of these four sites failed in this investigation's runs at load average ~200
  across twelve full-suite passes, so there is no evidence of live flakiness forcing the
  issue — but if CI ever shows one of them failing, the fix is "add the instrumentation,"
  not "raise the number." Estimated 4–6 hours, the most expensive item in the investigation
  and the lowest priority unless CI evidence changes that.

Both items are recorded here rather than assigned to a specific milestone number; pick them
up whenever TUI clock injection or manager lock instrumentation is next in scope.

## From fix wave 7's re-review response (2026-09-21), deliberately deferred

- **`remove_with_worktree`'s own git operations can still be running when `shutdown()`
  returns, and `lifecycle::run` moves on to the final state save and process exit right
  after.** Fix wave 7 added the missing `shutting_down` admission check to
  `WindowManager::begin_removal` (`crates/daemon/src/manager/remove.rs`), closing the
  straightforward gap where a removal issued *after* `shutdown()` had already returned still
  ran to completion. That fix is admission-only, deliberately — the same limit `create`'s
  own check has — and does not extend to a removal that was already admitted *before*
  `shutdown()` set the flag. Unlike `restart`'s Major 1, an in-flight `remove_with_worktree`
  cannot leave a *live, untracked process* behind: the window's entry stays in
  `inner.entries` for as long as the removal runs, and `shutdown`'s own unconditional scan
  of every entry still present calls `start_cleanup` on it regardless of the `removing`
  flag, the same path that already signals and waits for a live plain window. What is not
  covered is *filesystem and git-registry consistency*: `shutdown()` only waits for process
  cleanup (the `cleanups`/`orphaned_cleanups` receivers), not for a `remove_with_worktree`
  task to reach `finish_removal`. If the daemon process itself exits (after the final
  `state.json` save that follows `shutdown()`) while a `remove_with_worktree` detached task
  is still between its `unregister` and `worktree::remove`, or mid `git worktree remove`,
  the operation is cut off — the same corruption `server/requests.rs`'s own doc comment on
  `detach` already reasons about for a *client-dropped* future, just not for the daemon
  process's own exit. Closing this needs `shutdown()` (or `lifecycle::run`) to also wait for
  every in-flight `create`/`remove_with_worktree` detached task to finish, not only for
  process cleanup — a change to `detach`'s own contract, wider than an admission check, and
  out of a fix wave's scope. Not reproduced with a running test in this fix wave (it needs a
  slow or hook-stalled `git worktree remove` racing an actual process exit, not just
  `shutdown()` returning); recorded here from the code reading that found it.

## From the M6 whole-branch review (2026-09-22), deferred by fix wave 12

- **A generation counter on `Entry`, or a per-spawn tag on the events keyed by window
  id, to close the restart id-reuse staleness hazard for good.** `restart`'s own doc
  comment (`crates/daemon/src/manager/restart.rs`) already disclosed this for
  `WindowEvent::Output`/`Title`/`ParserPanicked` (fix wave 5 review, Minor 8): those
  come from the `pty-read-{id}` thread, a different sender than the `pty-wait-{id}`
  thread `child_alive`'s confirmation depends on, so a stale event already enqueued but
  not yet delivered at swap time can still land on the fresh `Entry` phase D swaps in
  under the same id. The whole-branch review found the same shape, undisclosed, in
  `ClientMsg::HookEvent`: `WindowManager::handle_hook` looks up `entries.get_mut(&id)`
  by id alone, and `anthrex hook` (`crates/cli/src/hook.rs`) is a separate, short-lived
  process a running agent spawns on its own — it can still be connecting or have
  already written its frame when this window's id gets killed and restarted out from
  under it, with no queue to drain at swap time the way `WindowEvent`'s `handle_event`
  has one (each hook connection is its own server task, unbuffered inside this crate).
  Fix wave 12 extended `restart.rs`'s own disclosure to cover this case rather than
  leaving it unmentioned, but did not implement a fix: closing either instance for real
  needs a generation counter on `Entry`, bumped on every `finish_restart` swap, plus a
  way for the stale sender to carry its own generation back — for `WindowEvent` that is
  free (the event already flows through an in-process channel `handle_event` could tag
  at send time); for `HookEvent` it needs a new field on the wire message itself, which
  is a protocol change (`PROTO_VERSION` bump, updating the TUI, the CLI client and
  `anthrex hook` together, per AGENTS.md hard rule 4) — real work with its own round-trip
  tests, not a point fix either instance's own fix wave had room for. Both instances are
  narrow in practice (the panic itself is rare for `WindowEvent`; `anthrex hook` is fast,
  single-digit-to-low-double-digit milliseconds per `docs/timing-budgets.md`'s idle
  table, for `HookEvent`) but real, not theoretical — a stale event or hook payload can
  relabel a freshly restarted window's status or session id. One counter closes both at
  once, which is the reason to do this as a single follow-up rather than two.

## From fix-wave-12-re-review (2026-09-22), left open by the M6 pre-PR fix pass

- **Decision 24's "another anthrex daemon is running..." message is built from two
  independent format literals.** `crates/daemon/src/lockfile.rs:172` and
  `crates/daemon/src/lifecycle.rs:209` each construct the same user-facing string
  separately, so a future wording change to one can silently drift from the other. Same
  class as m13, which fix wave 12 fixed the same way in `server/requests.rs`: a shared
  helper beside `holder_pid` would close it.
- **`crates/daemon/src/manager/restart.rs` and `state.rs` are now past AGENTS.md rule
  8's ~600-line guideline** (534→667 and 592→607 lines respectively, per
  fix-wave-12-re-review Minor 3), putting the milestone brief's own check 4
  (`wc -l crates/tui/src/app/*.rs crates/daemon/src/*.rs`) at three files over 600
  instead of the two the M6.12 write-up named. Both crossings came from comment prose,
  not new logic; a later pass should look at splitting `restart.rs`'s phase
  documentation out of the module doc comment, or moving `state.rs`'s persistence
  helpers into their own file, if either file grows again.
- **`stop_daemon` (`scripts/pty-smoke.py`)'s own `daemon stop` bound was left at 30s**
  while the nine `run_cmd`-wrapped call sites moved to `DAEMON_STOP_CMD_TIMEOUT` (40s).
  Not a defect on its own — 30s still exceeds the 19s real worst case — but it is the
  same scoping gap that produced the worktree-CLI timeouts fix (fix-wave-12-re-review
  Major 2): the sweep that raised the nine `run_cmd` sites never revisited this direct
  `subprocess.run` call outside `run_cmd`, which also has no `TimeoutExpired` handler of
  its own. Worth folding into `DAEMON_STOP_CMD_TIMEOUT` (or its own named constant) and
  the same `try`/`except` the fix pass added to `run_cmd`, the next time this file is
  touched for timing reasons.

## From milestone 6's final verification

- **`server_git.rs`'s intermittent failures are environmental to the checkout path, not a code
  defect.** They were first recorded here as one test that "passed in isolation immediately
  after"; that characterisation was wrong twice over. The failures are not confined to
  `two_windows_in_one_worktree_register_once` — `removing_the_last_window_unregisters_the_root`
  and `a_fresh_client_receives_every_known_root_after_welcome` fail the same way, always as
  `timed out: Elapsed(())` against `tests/support/mod.rs`'s 5 s `recv()` bound — and they do
  **not** pass reliably in isolation. Measured 2026-09-22, same commit, same idle machine,
  `cargo test -p anthrex-daemon --test server_git` run five times in each location:

  | checkout | passed | wall clock |
  |---|---|---|
  | `~/Desktop/repos/anthrex/.worktrees/m6-persistence` | 4/5 | 5.10 - 7.01 s (spread 1.9 s) |
  | `/private/tmp/anthrex-speedtest` | 5/5 | 3.07 - 3.11 s (spread 0.04 s) |

  The spread matters more than the pass count: a 0.04 s spread in `/private/tmp` against 1.9 s
  under `~/Desktop` means something on that tree (most likely macOS TCC or Spotlight indexing)
  injects multi-second stalls into the real `git` subprocesses these tests spawn, pushing them
  past a 5 s bound they otherwise clear in 3 s. An independent measurement during the launch-gate
  work found the same tree ~10x slower on these tests. CI runs under `/home/runner` and
  `/Users/runner` and has not reproduced it.

  Two consequences. **Timing numbers measured from a `~/Desktop` checkout are not comparable to
  CI's** — treat them as an upper bound only. And the 5 s `recv()` bound is still worth deriving
  from the constants it wraps rather than left as a literal, which would make these tests
  survive a slow host instead of merely usually beating it; that part is unfixed.
- **`cargo test --workspace` fail-fast truncates the result lines.** When a binary fails, later
  binaries never run and never report, so a run that hit the flake above produced 28-29
  `test result:` lines instead of 41. Two separate parties read those truncated counts as a
  missing-tests discrepancy and spent effort reconciling it. Anyone counting tests on this repo
  should pass `--no-fail-fast` and capture the exit code without a pipe in between.

## From the codex-probe handshake fix (2026-09-22), found by its own sweep

The fix itself (`launch::LaunchGate`) removed one instance of "a client deadline racing a
server-side startup step": `proto::HANDSHAKE_TIMEOUT` (5 s) against
`CODEX_PROBE_TIMEOUT` (5 s), with the probe sitting between `bind_socket` and
`server::serve`. Sweeping for the same *behaviour* rather than the same construct turned
up two more sites. Neither is caused by that fix; both are recorded here rather than
changed, because each one's fix ripples into bounds outside this change's scope.

- **`ENSURE_DAEMON_SOCKET_WAIT` (3 s) must outlast `LOCK_WAIT` (5 s), and does not.**
  `spawn::ensure_daemon` (`crates/tui/src/spawn.rs`) spawns a detached `anthrex daemon
  start --foreground` and then polls the socket for 3 s. That child's very first act
  (`lifecycle::run`) is `DaemonLock::acquire_or_yield(.., opts.lock_wait, ..)`, and
  `crates/cli/src/main.rs:356` passes `daemon::LOCK_WAIT` — five seconds of retrying
  another daemon's lock, entirely ahead of `bind_socket`. The `already_running` probe
  shortens that wait only when the *other* daemon is answering on the socket; a daemon
  that is in its own teardown (holding the lock through `manager.shutdown`, the persister
  await and the final state save, with its listener already dropped) is exactly the case
  where the probe says "no" and the full five seconds can be spent. The caller reports
  "the daemon did not start within 3 s" while its child comes up fine a moment later —
  the same two-independent-constants shape, with the client's being the smaller. Fixing it
  means deriving `ENSURE_DAEMON_SOCKET_WAIT` from `LOCK_WAIT`, which changes
  `daemon_start_does_not_wait_on_a_slow_codex_probe`'s assertion (it imports that constant
  deliberately, whole-branch-review m19) and every `pty-smoke.py` bound that includes an
  `ensure_daemon` term.
- **Nothing bounds the pre-bind startup work itself.** Decision 12 requires `state::load`
  and `config::load` to precede `bind_socket`, and `manager.restore` runs there too. None
  of the three has a deadline, so a large or slow-to-read `state.json` delays the bind by
  an unbounded amount — under the same 3 s client wait as above. Deliberate ordering, but
  worth an explicit budget (or a bounded read) before anybody relies on the 3 s.

Checked and found clean in the same sweep: `anthrex hook`'s `HOOK_DEADLINE` (1 s) over the
daemon's handshake — reachable only from a running agent, which only exists after a window
launch, which the gate itself orders after the probe; `daemon stop`'s `WAIT_RELEASED_CAP`
(10 s) over the teardown, to which the probe's cancel-and-await adds one 5 ms poll interval
rather than its remaining budget; `TestDaemon::start`'s own 3 s socket wait (every
`TestDaemon` gets its own data directory, so its lock is never contended); and the TUI's
create/remove form waits, whose `WORKTREE_FORM_TIMEOUT` (50 s) still clears the gate-widened
worst case (5 + 5 + 30 = 40 s).

## From the M6 spec-gap audit (2026-09-22), branch `m6-git-config`

Milestone 6 was implemented from `docs/milestones/M6-persistence.md` as it stood on
`main`. An unmerged refresh of that brief (`docs/m6-corrections`) had added scope to it
before implementation and was never merged, so the milestone was built from a stale brief.
The audit checked the refresh's claims against the specs — which bind — rather than
against the refresh. Closed on this branch:

- **The `[git]` table** (git-surface spec 3.7 line 140). Specified for M6, never
  implemented; `[git]` in `config.toml` produced one `unknown key, ignored` problem and
  nothing else. Now parsed, with the spec's own defaults, and threaded to the registry,
  the scheduler and the watcher's filter.
- **Restored windows were never watched.** Spec 3.3's registration rule and 3.5's
  "never blank" promise both held only for created windows: `register` was called from
  the `CreateWindow` handler alone. Every window restored from `state.json` came back
  with a blank git segment and stayed that way, because `Restart` did not register
  either — contradicting a comment in `manager/restore.rs` that claimed a restart was the
  repair. `server::register_restored_roots` now registers them at startup, once per
  window so the reference counting stays symmetric with `Remove`.
- **The state file did not record the watched root at all**, only the checkout anthrex
  created, so a window standing in someone else's checkout had nothing to re-register.
  `WindowRecord` now carries `worktree` (the root) and `managed` (the created checkout)
  separately, at `STATE_VERSION` 3, with version 1 and 2 files migrated on load.

Checked and found already correct, so nothing was built: **a reconnect preserves the
view** — the tree viewport and selection, collapse, filter, overview, milestone 4.7's
inspector panel and the graph pan all survive `on_reconnected`. The refresh presented
this as new scope; it has held since milestone 6, because `on_reconnected` goes through
`replace_windows`. It had no test, and now does
(`a_reconnect_leaves_the_view_where_the_user_left_it`).

Deferred from the same audit: the non-recursive watch and pruning walk, above.

Worth knowing for the next audit of this kind: every one of these was found by
constructing an input and running it — a config file with a `[git]` table, a `StateFile`
handed to `restore` before `serve`, an `App` with its view moved. Reading the same code
had already missed all three, twice, and the comment in `manager/restore.rs` is why:
it described a fallback that did not exist, and reading for plausibility believed it.

## From milestone 6.5's transcript capture (2026-09-22), task M6.5.7

- **Agents inherit the launcher's entire environment, including another agent's session
  markers.** Started from a shell inside Claude Code, the daemon inherited
  `CLAUDE_CODE_CHILD_SESSION`, and the `claude` it launched printed "Transcript saving is off
  — inherited CLAUDE_CODE_CHILD_SESSION marker": no transcript is ever written, so milestone
  6.5's conversation view can never enrich a window started that way. The same inheritance
  hands every agent roughly twenty other host variables, among them
  `CLAUDE_CODE_MESSAGING_SOCKET` and `CLAUDE_CODE_MESSAGING_TOKEN`, which connect it to the
  launching session. Reproduce: from a Claude Code terminal, `anthrex daemon start`, then
  `anthrex new --runtime claude --prompt hi`, and read the agent's status line. The fix is a
  deliberate environment policy at spawn time — which variables an agent window inherits
  and which it must not — and it is a product decision, not a one-line scrub: a user's own
  `ANTHROPIC_BASE_URL` or proxy settings may be exactly what they want passed through.
  Belongs with whichever milestone next touches `crates/daemon/src/window/spawn`
  (orchestration, milestone 8, launches agents unattended and needs the policy most).
- **The M6.5.7 brief's capture recipe does not work as written.** It finds the transcript
  with `grep transcript_path daemon.log`, but the daemon does not log raw hook payloads.
  Claude writes to `~/.claude/projects/<cwd with / and . replaced by ->/<session>.jsonl`,
  Codex to `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. Recorded in the brief's
  Implementation notes too.

## From milestone 6.5's enrichment task (2026-09-22), task M6.5.8

- **Repeated identical prompts defeat the alignment check when a hook is lost.** The
  enricher checks each transcript prompt's text against the hook-built prompt its
  ordinal maps to. With hook prompts `go, go` against transcript prompts `go, go, go`
  (the first hook lost), ordinal 0 lines up with the hook turn that is really prompt 1,
  so its prose is misattributed and nothing flags it. Inherent to text alignment; a fix
  needs a second key both sides share (a timestamp window, or a prompt id the runtime
  puts in both the hook payload and the transcript). Found by task M6.5.8's review.


## From milestone 6.5's reader and subscription task (2026-09-22), task M6.5.10

- **The transcript path is learned from `SessionStart` only.** `build::session_start` is the
  one place `Draft.transcript_path` is set, although Claude sends `transcript_path` on every
  hook. A window that missed its `SessionStart` (a hook dropped past `HOOK_DEADLINE`, or a
  session already running when the daemon restarted) degrades with `NoTranscriptPath`
  until the next session. Taking the path from any hook that carries one would close it.
- **A transcript read that truly hangs (review F3, left for later).** Every constructible
  path returns: `O_NONBLOCK` plus `fstat` refuses FIFOs and devices, and `read_more` stops
  at its own deadline between 64 KiB reads. Only a regular file on a hung filesystem (NFS,
  FUSE) can block one `read` forever. The review measured the result by mutation: one
  blocking-pool thread held, no growth per poll, the lock and other windows unaffected,
  but two defects. The stuck window's `degraded` stays `None`, and `#[tokio::main]`'s
  runtime drop waits for the blocked thread, so `anthrex daemon stop` hangs. The fixes are
  small: on an overrun, have `watch.rs` set `Unreadable`, waiting `2 * TRANSCRIPT_READ_TIMEOUT`
  rather than 1x (a healthy pass may legitimately end one chunk read past the inner
  deadline, which is the same constant). And have the foreground daemon
  `std::process::exit` once `daemon::run` has returned, or use `shutdown_timeout`. Not done
  in task M6.5.10's fix round 1: neither half can be made red-when-reverted without a
  seam that injects a blocking pass into `conversation::watch`, and that seam is larger
  than the fix.
- **Every hook and every reader pass clones the whole conversation under the manager lock
  (review F4).** `Entry::mutate` (`conversation/entry.rs`) builds
  `before: HashMap<u64, Turn>` from every turn before it runs the change, and
  `apply_transcript` calls `enrich` even for a pass with no records, so a subscribed idle
  window pays it four times a second. Measured at 500 turns: 142 KB of conversation, 15 µs
  to 100 µs per hook in release (158 µs to 764 µs in debug); 1.89 MB (near the default
  `max_bytes`), 18 µs to 189 µs in release (161 µs to 890 µs in debug). It grows linearly;
  at the config maximum (10,000 turns, 15 MiB) that extrapolates to about 1.5 ms per hook
  and per poll, under the lock. Cheap fixes: return early from `apply_transcript` when a
  pass has no records and nothing is parked, and diff only the turns the change can touch
  (the open turn and anything after the first changed index).
- **`crates/daemon/tests/server_restore_git.rs` was flaky (review F6); fixed.** Task
  M6.5.10's review saw `a_restart_keeps_the_restored_root_watched` and
  `a_restored_plain_window_keeps_its_worktree_and_is_watched` each fail once. PR #12 fixed
  it (a root's first probe no longer waits for its watcher to arm), and it is merged into
  this branch (`1a76965`); see "From the `server_restore_git.rs` flake fix" below. What
  still flakes is `server_git.rs`'s first-`Git` timeout, recorded there. Task M6.5.10's
  fix round 1 also saw one unattributed failure in a five-test daemon suite during a
  `cargo test -p anthrex-daemon` run that two reruns did not reproduce.
- **Two limits of the `/clear` fix (review F2).** A switch to a new session's file drops
  whatever the old file still had unread; a final drain pass of the old `Tail` before the
  switch would keep it. And a later shrink or replacement of the new session's file is a
  restart, whose `reset` is window-wide, so it drops the old session's enrichment too;
  scoping `enrich::reset` to turns at or after `session_base` would keep it.
- **A session opened at its end after a prompt degrades to `Misaligned` for the whole
  session (fix rounds 2 and 3, N1/N2, re-review 2 I1, re-review 3 m2).** Any switch whose
  source is not `startup` or `clear` (`resume`, `fork`, `compact` onto another session,
  unknown, or absent) is opened at its end when the reader first reaches it. If a prompt
  arrived between the `SessionStart` and that open, the prompt's own line may lie on
  either side of the measured end. The session is then `Misaligned` from its first
  ordinal: every later turn of that session gets no prose, not only the turns before the
  open. For Claude this happens when no viewer was subscribed at the resume, or when a
  scripted prompt beats the first poll. For Codex it is the normal case for every resume
  or fork, not an edge case: the 0.155.0 binary defers `SessionStart` to the first turn
  (`run_pending_session_start_hooks` in `core/src/hook_runtime.rs`), so `UserPromptSubmit`
  follows within milliseconds and the reader's first step is almost always after the
  prompt has already moved the count. Whether Codex's `SessionStart` can land *after* its
  own `UserPromptSubmit` — which would misattribute the old file's reply instead of only
  degrading (re-review 3, m1; guarded in fix round 4) — is still unverified; check both
  the deferral and the hook order in M6.5.14's manual step. It lasts until the next
  session switch. Aligning by the records' timestamps against the hooks' `at_unix_secs`
  would recover all of it.
- **Which `SessionStart` sources real runtimes send is unverified (re-review 2, M1;
  re-review 3, n1).** The rule reads from the start only for `startup` and `clear`. A
  runtime that omits `source` on a fresh session has that session opened at its end
  instead of from the start, and that end-open is `Misaligned`: not only when the viewer
  opens late, but even when a viewer subscribes before the session starts, because the
  file does not exist yet at the reader's first step, so no measure is taken before the
  prompt moves the count. This path is unreachable with real runtimes (Claude's
  `SessionStart` schema requires `source`; Codex's `CodexHook` schema does too, with the
  enum `startup | resume | clear | compact | fork`), so only the wording was wrong, not
  the behaviour. Codex's embedded schema lists `startup`, `resume`, `clear`, `compact` and
  `fork`. Check the values real Claude and Codex send, and whether a Codex fork's rollout
  copies the parent's history, in M6.5.14's manual step. fake-agent sends no `source`
  unless a script gives one; the fixtures now send `startup`.
- **`crates/cli/tests/persistence.rs`'s `session_id_learned_from_a_hook_is_saved` races the
  save debounce.** Seen once in three full workspace runs during task M6.5.10's fix round 1
  (`left: None, right: Some("fake-session-1")`; 8/8 in isolation). The loop asserts on the
  *first* `state.json` record it finds for the window, and a save from before the hook
  (the window's creation) satisfies "found" while still holding `session_id: None`; the
  save carrying the session lands up to `SAVE_DEBOUNCE` later. The loop should keep
  polling until the record's session id is `Some` (or the deadline passes), then compare.
  Milestone 6's test; not touched here.

## From the `server_restore_git.rs` flake fix (2026-09-22), deliberately deferred

The fix (`run_root` no longer makes a root's first probe wait for its watcher; see
`docs/timing-budgets.md`, "Fixed, from the `server_restore_git.rs` flake") changed only
`crates/daemon/src/git/` and that one test file. It turned up three things outside that
scope.

- **`server_git.rs`'s intermittent failures, recorded above as environmental to the
  checkout path, were probably this same mechanism.** Every wait there that failed was a
  first-`Git` wait behind the same watcher-first path, bounded by `recv()`'s 5 s. The
  earlier entry put the stall down to "the real `git` subprocesses", inferred from
  wall-clock spread and never measured step by step. The step-level measurement this fix
  made found `git` at ~25 ms and `Watcher::watch()` at 1.5–7.8 s, and found the stall
  just as easily in `/private/tmp`. The production fix was expected to remove that stall
  from `server_git.rs`'s first-`Git` waits too, but it did not remove every failure there:
  milestone 6.5's final review (2026-09-23) saw
  `server_git::two_windows_in_one_worktree_register_once` time out at `recv()`'s 5 s bound
  (`crates/daemon/tests/support/mod.rs:118`) in one of three full runs, after this fix was
  merged in. So `server_git`'s first-`Git` timeout still flakes. Its bounds are still
  literals, too: an 8 s
  `wait_for_created_and_git`, `recv()`'s 5 s, and the 1–2.5 s `assert_no_git_message`
  windows. They should be derived the way `server_restore_git.rs`'s now are (`PROBE_TIMEOUT`,
  `DETECT_TIMEOUT`, the configured `poll_secs`), and `wait_for_created_and_git` should
  stop going through `recv()`'s own 5 s panic.
- **`git_registry.rs`'s `a_real_write_triggers_a_probe` writes after the registration
  probe's publication**, so if the watcher is slow to arm, the write is now caught by the
  probe that follows arming rather than by a watcher event. Its deadline
  (`PROBE_TIMEOUT + DEBOUNCE + 10 s` = 15.3 s) covers every arm time measured so far, but
  it is not derived from anything that bounds arming. `server_restore_git.rs`'s
  `change_deadline` (`poll_secs` + 2 × `PROBE_TIMEOUT` + slack) is the derived form.
- **`git_probe.rs`'s `a_timeout_marks_the_state_stale` failed once** (`git_probe.rs:321`,
  "a timeout still returns the state parsed so far") in one of three full-workspace runs,
  which were run alongside a loop of `server_restore_git` on the same host. Not
  investigated. It is a timeout test, so check its bound against the timeout it injects
  before assuming load.

## From milestone 6.5's rendering task (2026-09-23), task M6.5.13's review M2

- **TUI-wide ASCII spinner.** In ASCII mode (decision A5) the conversation view draws only
  ASCII — its border, separators and footers included, since the final review's M4 — except
  `theme::SPINNER`'s braille frames for a Pending call. That spinner is the only non-ASCII
  thing left in the view. The rest of the TUI draws the spinner (and its own borders and
  `·` separators) unconditionally, so on a genuinely non-UTF-8 terminal they render as
  mojibake, and for a Pending call the spinner is the only state glyph. Fix it once for the
  whole TUI: an ASCII spinner (for example `| / - \`) chosen from the same A5 decision,
  which `UiSettings.badges.ascii` already carries. Milestone 6.5 or later.
- **The sidebar still draws box glyphs in ASCII mode** (milestone 6.5's manual check,
  2026-09-23). The same A5 decision should reach the sidebar's borders; fold it into the
  TUI-wide ASCII fix above rather than fixing the sidebar alone.

## From milestone 6.5's final review (2026-09-23), left open by its fix pass

- **A conversation revision means nothing across a daemon restart (review M1).** A restored
  window's conversation starts again at rev 0 and turn id 1, and
  `crates/daemon/src/manager/conversation.rs`'s `reply` sends a delta whenever its store
  holds the `from_rev` it is asked for. So a `from_rev` learned from one daemon instance is
  answered by the next with a delta against a different conversation; the review built
  `[user "old prompt A", assistant [Bash "echo new"]]`, a conversation that never existed,
  and decision A12's guard cannot catch it because the client's rev equals `from_rev`.
  Latent today: the TUI always sends `from_rev: None`, including when it resubscribes after
  a reconnect (the fix for review I1 chose `None` for exactly this reason). It becomes live
  the moment any client resubscribes with a rev, which decision 8 intends for milestones 8
  and 9. Fix before then: tag revisions with a daemon epoch (a start nonce on `Welcome`, or
  on the conversation), or state in `proto` that `from_rev` is valid only on the daemon
  instance that issued it.
- **A turn's prose always leads it (review N1).** Enrichment joins all of a turn's transcript
  prose into one leading `Text` block (brief task M6.5.8, rule 2), so a closing remark
  renders above the tool calls it followed. As briefed, but spec §6's mock interleaves prose
  and calls. Worth revisiting for milestone 8's run view.
- **Missing prose after a `max_turns` drop is not flagged (review N5).** When the cap drops a
  `User` turn before the reader has aligned its ordinal, that turn's reply gets no prose and
  `degraded` stays `None`. The brief accepts this ("missing prose is the accepted cost"),
  but it is timing-dependent: the review saw a reply's prose in one run and not in another.
  A `Misaligned`-style reason for "prose lost to the cap" would make it visible.
- **A sub-agent's `degraded` is the root file's reason (review M2's other half).**
  `ConversationSet::set_degraded` copies the root transcript's reason to every key, so a
  sub-agent level can show "transcript unreadable" about a file its own prose never came
  from. Since the fix pass every sub-agent level also shows the client-side footer
  "sub-agent transcript not read — timeline only", so nothing is silent; keeping a reason
  per key would make the copied line accurate.
- **`WindowManager::unsubscribe_conversation` ignores its `agent_id` (review N4).** The
  viewer count, and the transcript reader it gates, are per window by design (decision 9),
  so there is nothing per key for the parameter to act on. Honouring it would mean per-key
  viewer counts that nothing reads; not cheap for no behaviour. Either drop the parameter
  or give it a use when a per-key subscription needs one.
- **Six test files are over the 600-line rule (review M5).** `crates/cli/tests/hook_command.rs`
  (977), `crates/daemon/src/conversation/store_tests.rs` (924),
  `crates/proto/src/conversation_tests.rs` (893), `crates/config/src/lib_tests.rs` (741),
  `crates/fake-agent/tests/script.rs` (681) and
  `crates/daemon/src/conversation/build_tests.rs` (617). The production file the review
  named, `crates/config/src/lib.rs`, was split in the fix pass; these were left as they are.
  Split each by the concern its tests cover, as `lib_tests.rs` could follow `lib.rs`'s new
  `conversation.rs` and `git.rs`.

## From milestone 6.5's manual check (2026-09-23)

- **The title shows no model unless `--model` is given.** A window started without
  `--model` has no model to name. Learn it from the transcript instead: every Claude
  assistant record carries `message.model`. The transcript reader already parses those
  records, so it could report the model it sees.
- **Codex 0.155.0's tool calls land in a separate assistant turn from that turn's prose.**
  Seen in the conversation view with real Codex 0.155.0: one reply shows as two assistant
  turns, the calls in one and the prose in the other. Not investigated; start from how the
  Codex hooks open and close turns against `codex-0.155.0.jsonl`.

## From the M8a brief refresh (2026-09-23), for the user to decide

- **AGENTS.md's environment-lock rule differs from landed practice.** `AGENTS.md:70` says
  "Tests that change environment variables must hold a shared lock." The landed code does
  not do that: each env-mutating test lives alone in its own test binary
  (`crates/daemon/tests/worktree_env.rs`, `crates/daemon/tests/git_env.rs`), and the
  daemon crate has no shared env mutex. `worktree_env.rs:1–20` explains why: the hazard
  is a `set_var` racing a child-process spawn on another libtest thread, which reads the
  whole `environ` block without taking any lock the crate controls, so a lock around the
  mutation alone does not help. The M8a brief follows the landed practice
  (`run_git_env.rs`, `run_exec_env.rs`, `headless_env.rs`). AGENTS.md was left unchanged;
  the user decides whether to reword the rule to "put the test alone in its own test
  binary".

## From M8a.6's review (2026-09-23), for milestone 9

- **The sub-planner's `EditScope::Area` limits only `owns`.** Under `Area { globs }`,
  `apply_edits` still accepts `cancel_task` and `answer` on tasks outside the area, and
  the run-level `pause`, `resume` and `finish` (task-6 review F7). Decision 12 speaks
  only of `owns`, so this is within M8a's letter. M9's sub-planner scope must decide
  which edit kinds, and which tasks, a sub-planner may use. *Handled in M9.8:*
  `submit_epic` refuses `answer`, `pause`, `resume`, `finish`, `message` and `refresh`
  (`op <op> is not available to a sub-planner`), and `run/edits_orch.rs::planner_confinement`
  refuses `amend_task`, `cancel_task` and `add_dep` on a task of another epic or of none.
- **No `remove_dep` / `replace_dep` edit.** A `blocked(dep_cancelled)` task stays
  editable (fix round 1, F3), and it can gain a dependency on a replacement task. But its
  dependency on the cancelled task cannot be removed, so it never becomes runnable. Today
  the only repair is `split_task` of the blocked task (the children carry fresh deps, and
  its dependents are rewired to them). M9's re-planning should add `remove_dep` or
  `replace_dep`, or document split as the path.

## From M8a.7 (2026-09-23), for M8c's conversation view

- **A synthesised Codex tool call's one-line summary reads as JSON.** Decision 27's table
  makes a headless Codex call's `PostToolUse` carry `tool_response: {"output": text}`
  (or `{"error": text}`). M6.5's `build::post_tool_use` renders any object response as
  its compact JSON, so the summary line shows `{"output":"[task-b 9d0ec2a] add a\n…`
  (asserted in `a_codex_session_builds_a_real_conversation`). A real Claude hook's
  `{"stdout": …}` object already renders the same way on `main`, so this is not new to
  M8a. The full text is the enriched `detail`, which is right. M8c, which puts headless
  conversations in front of the user, should decide whether the summary should read an
  object's single text field (`output`, `error`, `stdout`) instead. *Handled in M8c.11:*
  `conversation/summary.rs::response_text` reads an object response's single non-empty
  text field (its first non-blank line) for the summary; the detail is unchanged.

## From M8a.7's fix round 1 (2026-09-23), for M8a.12, M8a.18, M9.5 and M8c

- **A Codex usage limit names when it resets; mark the runtime unavailable until
  then** (ruling T7-C2, for M8a.12's rate-limit handling and M9.5's adaptive
  concurrency). The captured `turn.failed` reads `You’ve hit your usage limit. Visit
  https://chatgpt.com/codex/settings/usage to purchase more credits or try again at Sep
  25th, 2026 11:33 AM.` (`crates/daemon/tests/fixtures/headless/codex-0.156.1-usage-limit.jsonl`).
  Today it parses as `Failed { RateLimit }`, and decision 32 retries every
  `rate_limit_retry_secs`, which can mean dozens of doomed turns over two days. Parse the
  `try again at <date>` time. Mark the Codex runtime unavailable until then: no dispatch
  or delivery to Codex sessions, and route new work to the peer runtime where the roster
  allows it. Do not retry.
  - **M8a.12 (2026-09-23) passed this on to M9.5, whole.** Decision 32 fixes the wait
    after a failed rate-limit turn at `rate_limit_retry_secs`, and no M8a.12 decision
    covers a per-runtime availability clock, routing around a runtime, or a parsed
    reset time. The reset time also has no time zone (`Sep 25th, 2026 11:33 AM`), so
    turning it into the reducer's unix seconds needs a rule nobody has set. M8a.12
    counts the failure as a rate-limit event (`Run.rate_limits["codex"]`) and waits as
    decision 32 says.
- **An interrupted turn stays `Running` in the conversation** (M8a.7 review M3, for M8a.18
  and M8c). An interrupted Codex turn ends with `ProcessExited` and no `turn.*` line.
  Claude fires no `Stop` hook for an interrupted turn, and with `hooks_fire` the
  synthesised `Stop` is dropped. The Assistant turn therefore stays `Running` with its
  call `Pending`, and the next prompt closes that call as `Denied`. The daemon knows the
  turn ended. `TurnEnded { Interrupted }`, and a Codex `ProcessExited` while a sent turn is
  open, could synthesise `Stop` even when `hooks_fire` is true. *Handled in M8c.11 and its
  review fixes:* a Claude `TurnEnded { Interrupted }` keeps its synthesised `Stop` under
  `hooks_fire` unless the turn's own `Stop` hook already came or a later prompt is open; a
  Codex `ProcessExited` with a sent turn open, and a Claude `ProcessExited` with a turn
  open (an engine kill: stall, cancel, `CancelLive`, give-up), synthesise `Stop` and close
  the turn (`headless/conversation_turn_end_tests.rs`).
- **`server_git`'s `two_windows_in_one_worktree_register_once` fails when built into
  `.worktrees/m8a-engine-core/target`** (found during M8a.7's fix round 1, not caused by
  it). Load average was about 20. In that target dir the binary takes 5.5–6.8 s and fails
  about 2 runs in 3 on the `Client::recv` 5 s timeout
  (`crates/daemon/tests/support/mod.rs:118`). It fails there even when built from the
  base commit `b5173fb`. The same code built into a target dir under the session
  scratchpad takes 3.0–3.6 s and passes every time, for both the base and the fix-round
  code. `git_registry`'s `a_commit_in_a_linked_worktree_triggers_a_probe` failed once in
  a full parallel run and then passed three times alone. Worth a look at what in a
  `target/` inside a git worktree slows the window-creation path (a file watcher or git
  probe walking `target/`?). Worth an owner in M8a.8's git work or a flake pass.

## From M8a.9's review (2026-09-23), for M8a (salvage)

- **A git repository nested in a task worktree is salvaged as a gitlink only, and its
  content is lost when the worktree is removed** (task-9 review m3, ruling T9-m3). This
  happens when a worker runs `git init` in `nested/`, commits, and leaves `p.txt`
  uncommitted. Decision 20's `git add -A` then records `160000 commit <sha> nested`. That
  commit is not in the run repository's object store, and `p.txt` is not saved at all.
  `git worktree remove --force` then deletes `nested/`. The salvage ref exists, but it
  does not hold the work.
  - A cheap guard: refuse the salvage, and so the removal, when the written tree has a
    gitlink that `HEAD` does not have. The error would name the path.
  - A fuller fix: salvage the nested repository into its own ref, or copy the directory
    aside.
  - Decision 20 prescribes exactly `add -A`, so this needs a ruling before code.

## From M8a.10's review (2026-09-23), for M8a (plan validation)

- **`test_passed` need not contain `{test}`.** `run/plan.rs` (the `profile.test_passed`
  check near line 229) only compiles the pattern. A profile with `test_passed = "test
  result: ok"` makes the proof's "the output shows the named test" half vacuous: any
  passing run matches. Proposed: require `{test}` in `test_passed` when `single_test`
  is set, as `single_test` itself must contain `{test}`, with the error `must contain
  {test}`. This is a plan-validation change (M8a.5's territory), ruled out of scope for
  M8a.10 (ruling T10-M5).

## From M8a.12's re-review 3 (2026-09-23), for M8a (ruling T12-R4, no code change)

- **A task blocked by the third failed `CountCommits` keeps its live session.**
  `fallback::count_failed` calls `block` without `kill_worker` (re-review 3, probe RE).
  This matches the delivery-failure block (ruling T12-A2), but `check_denials` kills
  before it blocks. A worker still running can keep editing a blocked task's worktree.
  Decide in M8a.13/M8a.15, with the unblock and resume work, whether to kill it or to
  keep the session for the resume.
  **Decided in M8a.15: kept.** A blocked task dispatches nothing and gets no mail, so
  the session idles; `run retry` kills it first (`ladder::kill_worker`) and starts a
  fresh one, `run override` sends the counted head to the merge queue, and a cancel
  kills it. A restart ends it with every other session. Killing at the block would
  lose nothing either, but would diverge from the delivery-failure block (T12-A2).
- **A failed resume drops an in-flight wrap-up.** `outbox::resumed`'s failure path now
  calls `ladder::supersede`, which removes the task's delivered-but-unconfirmed outbox
  messages. Before, a late `Delivered{ok:false}` queued them again into the fresh
  session's `append`. A wrap-up sent just before a mid-turn exit and a failed resume is
  therefore missing from the fresh prompt. Decide with M8a.13's resume rules whether
  `supersede` should keep them for `append`.

## From M8a.13 (2026-09-23), for M8a

- **Messages queued to a worker are dropped by rung 2** (M8a.12's `drop_queued` and
  `supersede`). The hand-over prompt carries every bounce text and the amended brief,
  but not an `answer` or an unread `budget_wrap_up` still in the outbox. An answer can
  only be queued to a `blocked(question)` or `working` task, and a `working` task
  reaching rung 2 with an undelivered answer is possible (a stall). Consider appending
  undelivered answers to the fresh session's `append`, as a failed resume does.
- **A Codex exit that arrives before the next process's `ProcessStarted`** (M8a.13 fix
  round 2). `signals::exited` now drops an exit whose pid is not the round's. But a
  delivery opens the next turn at once. If process N's exit comes after that delivery
  and before N+1's `ProcessStarted`, the round's pid is still N, so the exit is taken
  for a mid-turn death of the new turn. Closing this needs the round to remember that
  process N finished its turn (for example an `exiting_pid` set when a delivery opens
  a Codex turn over a live pid), or the executor must guarantee that `ProcessStarted`
  for N+1 comes before N's exit. M8a.22's executor should settle which one.
  **Done in M8a.22:** `AgentRound.closed_pid`, the first option.

## From M8a.14's fix round 3 (2026-09-24), for M8a

- **`verify_done`'s dirty count follows the submodule config.** Its `status
  --porcelain` has no `--ignore-submodules=none` (`worktree/dirty.rs` passes it). Under
  `diff.ignoreSubmodules=all` or a `.gitmodules` `ignore = all`, an uncommitted gitlink
  change or a dirty submodule does not count toward `dirty_tracked`. Nothing
  uncommitted is merged, so this is not a gate bypass. The claim is accepted with work
  left behind, though, which M8a.9's salvage then has to catch.

## From M8a.15's fix round 4 (2026-09-24), for M8a

- **The liveness oracle rejects a halted run's task in `proof` or `check`.**
  `gates_alive` (`run/engine/tests/liveness.rs`) wants the gate op in flight, but
  `start_gates` runs only while the run runs, so such a task rightly waits for the
  resume. `assert_alive` resumes a paused run before checking, and nothing does that
  for a halted one. The review branch already exempts a stopped run; `proof` and
  `check` should too, or the oracle should resume a halted run with a rebaseline.
  **Done in final fix batch F4 (T15-P1):** `proof` and `check` are exempt in a halted
  or paused run.

## From M8a.18 (2026-09-24), for M8a

- **`headless_sessions`'s `kill_sends_sigterm_to_the_whole_group_first` is flaky under
  load.** It failed 1 run in 40 at the base commit `bc19dca` (and about 1 in 16 on the
  M8a.18 branch) at load average 20 to 27, on `the group's SIGTERM reached <pid>`: the
  background `sleep` is still a zombie of the trapping leader 2 s after the `SIGTERM`.
  `alive()` is `kill(pid, 0)`, which succeeds for an unreaped zombie, and the leader
  reaps its child only between its own `sleep 0.05` loops. Checking the process state
  (`ps -o stat=` not `Z`) instead of `kill(pid, 0)` would test what the test means.


## From M8a.21's review (2026-09-24), for M8a

- **Codex's first turn needs an anthrex marker in its argv so reconcile can find it.**
  Decision 28's leftover check signals a recorded pid only when its argv holds the
  session id, and `codex exec`'s first turn has no thread id yet, so an orphaned first
  turn survives a daemon restart. Put an anthrex-owned marker on Codex's argv (for
  example the op's `session_uuid` as a `-c` config value) so `run/reconcile/sessions.rs`
  can verify a recorded pid (ruling on M8a.21 concern 5).
- **A replayed accept `Finished` did not run its clean-up.** Reconcile now reports it
  honestly (`accepted as <sha7>; clean-up did not run before the restart`, with every
  run branch in `kept_branches`), but the salvage, worktree removal and branch deletion
  are not re-run. M8a.22 should re-run them after the restore. **Done in M8a.22**
  (`driver/restore.rs`); the report still names the branches as kept.
- **Session uuids need a per-run random nonce.** `session_uuid(run_id, op)` is
  deterministic, so two runs with the same id share uuids (Claude's session store, and
  any id-based check). Mix a random nonce stored in `run.json` into it (M8a.22).
  **Done in M8a.22:** `Run.session_nonce`, `role_launch::session_uuid_of`.

## From M8a.22 (2026-09-24), for M8a

- **Decision 53 counts worker routes only.** Fix round 1 (ruling T22-I1) widened the
  check to reviewers and one escalation, and called that complete; it was not. **Closed
  in M8a.22's fix round 2** (ruling T22-I1b): decisions 50 and 53 check every runtime
  a run can reach (`run::reach`), at `run start` and on every plan edit.
- **The report's `codex project config:` line** (decision 53's three texts) is not
  written: `report::render` sees only the `Run`, which does not record the caps it
  started under. **Done in M8a.23** (ruling T23-C1): `Run.codex_project_config`.
- **The Codex first-turn marker** (above) is still open.
- **T8-RR2** (the `<run_head>...HEAD` range after `resume --rebaseline`) is still
  open: the driver passes `DiffSoFar`'s fields through unchanged.

## From M8a.24 (2026-09-24), for M8a

- **A green merge candidate leaves no check record.** `OpResult::Merged` carries no
  check result, so only a red candidate becomes a `CheckRecord` with `on_candidate:
  true` (M8a.14). The report and `TaskInfo.last_check` never show the candidate check
  a merged task passed, and `run override`'s "it still passes the candidate check"
  (decision 35) is visible only as the merge itself.
  `e2e_override_merges_without_approval_and_is_reported` proves the candidate check ran
  with a check that logs its working directory. Decide whether `Merged` should carry
  the check's record.
- **Engine deadlines are whole truncated seconds.** M8a.24 made the failed turn's
  continue wait at least `rate_limit_retry_secs` (`engine::clock::not_before`). The
  other engine timers keep `now + secs`, so each can fire up to a second early in real
  time: the stall clock (`stall_after_secs`), `INTERRUPT_GRACE`, the delivery retry
  and `ApiRetry`'s `rate_limited_until`. None of them is a promised lower bound that a
  test measures today. **Closed in M8a.24's fix round 1** (ruling T24-clock): all four
  go through `not_before` (the stall clock through `clock::stall_due`), each with an
  engine unit test at its boundary. The `CountCommits` retry (`count_retry_at`) keeps
  `now + DELIVERY_RETRY_SECS`: a retry of an engine op, not a promised wait.
- **`RunHarness` leaks a daemon whose start outlasts the CLI's 3 s.** Under load (seen
  once in M8a.24's fix round 1, seven tests at once), `anthrex daemon start` gave up
  after 3 s while the daemon came up at about 4 s. `start_daemon` then panicked,
  `Drop`'s `stop_daemon` returned early because no socket existed yet, and the temp
  dir's removal left the daemon running (stopped by hand through its own socket).
  `stop_daemon` should wait for, or stop through, the pid in `daemon.pid` on that path.
  **Closed in M8a.24's fix round 1:** the harness starts `anthrex daemon start
  --foreground` as its own child (`support/run_daemon.rs`), waits `DAEMON_START_WAIT`
  for the socket, and its `Drop` stops that child whatever the start came to (through
  `anthrex daemon stop` once the socket is up, else by killing the unreaped child).
  Test: `a_slow_starting_daemon_does_not_outlive_its_harness`.

## From M8a.25 (2026-09-24), for M8a

- **Handled (`fix-early-session-events`: `run/engine/early.rs` holds and replays them.)**
  **A session's signals and tool calls before its `CreateWindow` result are lost.** The
  engine finds a round by its `window_id`, which it learns from the `Window` result;
  the session's process starts inside that op, so everything it does before the op's
  `done` line is written and its `OpDone` stepped is dropped (`signals::on_signal`
  finds no round) or refused (`task_done` gets `this window is not the current worker
  of task <id>`). The first `ProcessStarted` was always lost this way; M8a.25 carries
  that pid in the `Window` result. The rest (an early `TurnEnded`, a quick first
  `task_done`, a Codex first turn that ends within milliseconds) needs an agent that
  acts within the few milliseconds of an fsync, so no real CLI is known to hit it; a
  300 ms hold on the `done` line (`ANTHREX_TEST_DELAY_DONE_MS` on a `CreateWindow`)
  reproduced it with `fake-agent`: the worker's turn stayed open with no claim.
  Options: buffer a window's signals until its round has it, and accept a worker tool
  call whose `RunRef` names the round whose launch is in flight.
  - **Its interaction with the recorded pid (M8a.25 fix round 1, review finding 1).**
    Recording that pid made the race worse at one point: a session whose process exited
    before its window was known had a round with the dead pid, and `kill_effect` waited
    for that pid's exit, already delivered and dropped, so the round never ended and
    rung 2 never started. `kill_effect` now synthesises the engine's exit whenever the
    window has no live process. A duplicate reaches an ended round, which ignores it,
    or a round resumed since, which drops it as a repeat of the exit it took (final fix
    batch F3, B-10; before F3 it ended that round or counted a death, T25-N1).
    `e2e_a_session_that_exits_before_its_window_is_known_still_escalates` reproduces the
    race with `ANTHREX_TEST_DELAY_WINDOW_MS`. The early events are now held and replayed
    (above), so that test's later sessions no longer wait out the hold, and session 1's
    exit reaches its round.
- **`fake-agent`'s `git_commit` step does not create a missing directory** (`a/flag`
  fails, and the process exits mid-turn). The M8a.25 tests commit into a new directory
  with `sh` instead.
- **The restart test's idle working worker.** `e2e_daemon_restart_pauses_and_resume_continues`
  covers a session idle at the restart only for a task blocked on a question (`t3`,
  M8a.25 fix round 1). A *working* task's session cannot be idle at a restart with
  `fake-agent` long enough to aim at: every turn end of a working worker without
  `task_done` runs decision 32's fallback at once, whose nudge turn then claims the
  task. So the queued `RESUME_WORKER` for an idle working round is covered by the
  engine unit tests of `restore.rs` only.

## From M8a's final fix batch F1 (2026-09-24), for the user and for M8a/M8b

- **For the user to decide: `AGENTS.override.md` and `CLAUDE.local.md` are not
  protected.** Codex reads `AGENTS.override.md` and Claude Code reads `CLAUDE.local.md`
  as instructions, like the five built-in protected paths. The user's rule names five
  paths verbatim (`.claude/**`, `.mcp.json`, `.codex/**`, `CLAUDE.md`, `AGENTS.md`), so
  F1 did not add them (final review A-M7, ruling). Adding them is a one-line change to
  `run::plan::BUILTIN_PROTECTED` plus the brief's decision 56.
- **Narrowed worker git grants are proven on macOS only.** `run_git_sandbox.rs` proves
  them under a real `sandbox-exec` profile. Since fix round 1 the grant names single
  files (the task's branch and its `.lock`, and in the worktree's git dir `HEAD`,
  `index`, `MERGE_MSG` and the like with their `.lock`s), several of which do not exist
  at launch. Seatbelt grants a missing path; on Linux, Claude Code's bubblewrap and
  Codex's Landlock were not run, and if either skips a missing path a worker cannot
  commit there (blocked, not unsafe). A repository using the reftable ref backend
  (`extensions.refStorage = reftable`) keeps refs under `reftable/`, which is not
  granted, so a worker there cannot commit either.
- **A nested repository's uncommitted contents are not salvaged.** Salvage leaves every
  gitlink out of `add -A`, and engine `status` does not look inside nested
  repositories (`--ignore-submodules=dirty`), so a worktree whose only change is inside
  a nested repository counts as clean and is removed without a salvage ref (fix round
  1, N1).
- **M4's worktree git calls run hooks.** `crate::worktree::ops` and `dirty` (the user's
  own `anthrex worktree` windows) pass `core.fsmonitor=false` but not
  `core.hooksPath=/dev/null`, so `worktree add` runs the repository's `post-checkout`.
  These are user-initiated and workers can no longer write the hooks directory; F1
  scoped `NO_HOOKS` to engine calls (review N7).
- **D-10 (not done in F1): a stale review worktree is replaced without salvage.**
  `run::git::prepare_review` removes an earlier round's worktree with `worktree remove
  --force`; a Claude reviewer with `Bash` could leave untracked files there. Salvage (or
  refuse) first, as `cleanup::remove_worktree` does.
- **D-12's remaining prune.** `forget_missing` (a registered engine worktree whose
  directory is gone) still runs a repository-wide `git worktree prune` (reconcile's
  `PrepareWorktree` row no longer does, fix round 1), which also
  forgets the user's own missing worktrees. Git has no per-path prune; removing the
  one administrative directory by hand would be the narrow fix.
- **D-3's second half.** A merge whose hook or grandchild holds its output past the
  deadline is now recognised as merged, but `subprocess::capture` still waits for EOF on
  both pipes after git exits. Ending the capture a short grace after the process exits
  would bound it.
- **A worker's `gc --auto` warns.** Since final fix batch F1b a worker's git writes
  objects only to its private object directory, where its `gc`, `repack` and
  `commit-graph write` work; its `gc --auto` still tries `pack-refs` in the common dir,
  which its sandbox denies, and warns (`error: Unable to create
  '<common>/packed-refs.lock': Operation not permitted`, exit 0; tested before F1b by
  F1 re-review 2, S5). The repeated `error:` line may lead a model to "fix" something.
  A fix is to launch workers with `gc.auto=0` and `maintenance.auto=false` through
  `GIT_CONFIG_PARAMETERS` in the headless spec's environment. **Resolved by final fix
  batch F1c:** each task checkout is its own repository whose engine-written config sets
  `gc.auto=0`, and its refs are its own.
- **A task branch made a symbolic ref halts the run with a misleading reason.** Since
  F1 fix round 4 (S1), every engine call refuses a task branch whose ref is a symbolic
  ref or link, and blocks the task. D-2's guard (`merge::work_on_base`) still counts the
  run's refs with `--glob=refs/heads/anthrex/<run>`, which follows such a ref. So a user
  commit on the base would be reported as "unaccepted run work", and the run halts.
  That is fail-safe, but the reason is wrong. The guard could list the run's refs with
  `for-each-ref --format='%(refname) %(symref)'` and name the tampered branch instead.
- **F1b: the user's own git fails while a worker has unimported commits.** Since
  final fix batch F1b a worker commits into its private object directory on a detached
  `HEAD` in `<common>/worktrees/<task>/`, and the engine imports the commit only at its
  next turn-end count, done check or hand-back. Until then the user's `git gc`, `git
  repack -a -d`, `git prune`, `git log --all` and `git fsck` in their own checkout fail
  with `fatal: bad object worktrees/<task>/HEAD` (reproduced with git 2.50.1): git walks
  every worktree's `HEAD` and index as roots. Nothing is lost or changed, and a
  background `gc --auto` only fails and logs. Fixes to weigh: make each task worktree a
  separate repository (its own git dir under the run's data directory, the common store
  as its alternate) so the user's repository never names the worker's objects; or have
  the engine import on every turn end, not only when the fallback counts. **Resolved by
  final fix batch F1c** (the first fix; `run_git_users_git.rs`).
- **F1b: a worker's unimported commits are lost if its worktree is deleted by hand.**
  `prepare_worktree` forgets a registered task worktree whose directory is gone and
  re-adds it at the task branch's tip; commits the worker made since the last import
  stay only as unreachable objects in `<data_dir>/runs/<run>/tasks/<task>/objects`.
  Importing from the old `worktrees/<task>/HEAD` before forgetting it would keep them.
  **Resolved by final fix batch F1c:** the checkout's repository lives in the data
  directory, so a checkout deleted by hand comes back from it with its `HEAD` imported
  first (`a_task_checkout_deleted_by_hand_comes_back_with_its_work`).
- **F1b: private object directories are never removed.** Each task's
  `<data_dir>/runs/<run>/tasks/<task>/objects` and `staging.git` stay with the run's
  data directory after accept or discard. **Resolved by final fix batch F1c:** removing a
  checkout removes its repository (private objects, engine files, per-task tmp).

## From M8a's final fix batch F1c (2026-09-25), for the user and for M8a/M8b

- **I1's residual: the CLIs resolve the grant again.** The engine now creates each
  granted directory as the canonical parent plus the leaf and refuses a link found
  there, and removes links at the other granted paths. Claude and Codex canonicalise
  the paths again when they build their sandbox profiles, after the engine's check; a
  process that swaps a granted path for a link in that window widens the grant. The
  engine cannot close that window from outside the CLIs; handing them already
  canonical paths under a directory no worker can write is the most it can do.
- **Standalone checkouts: LFS, submodules and partial clones.** A task checkout is its
  own repository with the user's object store as its alternate. Git LFS objects (in
  `<common>/lfs`), submodules' own repositories (`<common>/modules`) and a partial
  clone's promisor remote are not wired into it; a project that needs them sees
  missing files or fetches that fail. Symlinking `lfs/objects` read-only, or giving
  the checkout the promisor config, are the options to weigh.
- **Standalone checkouts: the worker sees no branches.** The checkout's repository has
  no refs but its detached `HEAD`; `git log main` or `git branch` in the worker's shell
  finds nothing. The prompts name commits by id, so no role depends on it, but a model
  may be confused. Mirroring the base and run refs read-only into the checkout's
  `packed-refs` at dispatch would give it names.
- **I2: no check confinement on Linux (bubblewrap).** `run/confine.rs` confines
  checks, proofs and `setup` with `sandbox-exec` on macOS only. Since F1c round 2 a
  Linux `run start` refuses unless the user passes `--unconfined-checks` or sets
  `[orchestrator] unconfined_checks = true`, and the run, `run status` and the report
  say so. A bubblewrap wrapper (read-only bind of `/`, writable binds of the checkout,
  its objects, its tmp and `cache_dirs`, `--die-with-parent`, the shell `exec`ed as
  the group leader) or landlock would let Linux runs confine instead; when `bwrap` is
  on `PATH` the refusal could offer it. Not implemented, by ruling.
- **I2: confined `setup` needs `cache_dirs`.** Since F1c round 2 `setup` is confined
  like checks. A `setup` that installs dependencies (`npm ci`, `pip install`) writes
  the package manager's cache under `$HOME` and fails until the profile lists it in
  `cache_dirs`. Network access is not restricted.
- **E2e tests read the report without waiting.** `run_e2e_settings.rs`'s
  `e2e_trust_project_is_accepted_and_reported` read an empty report once under load
  (load average 35) right after `complete`; F1c round 2 moved it to the deadline
  helper `report_with`. `run_e2e_basic.rs:67`, `run_e2e_finish.rs:113` and
  `run_e2e_settings.rs:183` still call `report(&run)` directly after `complete`.
  **Done in final fix batch F4:** `run_e2e_basic` waits with `report_with`;
  `run_e2e_finish` reads the text its deadline wait returned; `run_e2e_settings` had
  already moved into an `until` loop.
- **N4: a reviewer's allowed git commands could once write files.** A Claude reviewer
  now runs under a read-only seatbelt sandbox (empty `allowWrite`) and a Codex reviewer
  under `-s read-only`, so `git diff --output=<path>` cannot write. Claude Code's
  permission matcher may still allow the `--output` flag on `Bash(git diff:*)`; the
  sandbox is the real block. Scouts (M9, not built yet) must be launched the same way,
  a read-only OS sandbox, when they arrive.
- **N5: a checkout the user deleted by hand loses its unimported commits at run end.**
  `driver/cleanup.rs::remove_worktree` skips salvage when the checkout directory is
  gone, and `checkout::remove` then deletes the repository holding the worker's `HEAD`
  and private objects. Before F1c they survived in the private dir with a recovery
  recipe. Fix: when the path is gone but `repo.git_dir()/HEAD` names a commit, `sync_in`
  it (or write a salvage ref) before removal.
- **N7: `includeIf "gitdir:..."` stops matching for task checkouts (3a regression).** A
  task checkout's git dir is under anthrex's data dir, so a user who sets their work
  identity with `[includeIf "gitdir:~/work/"]` gets worker commits with the wrong
  identity, and those commits are merged into the base branch. Fix to weigh: read the
  user's `--global`-resolved `user.name`/`user.email` at run start and set them in the
  checkout's engine-written config, or add an `includeIf "gitdir:<data>/..."`-aware rule.
- **`git worktree list` no longer shows the run's task checkouts.** Only the
  integration checkout stays a linked worktree. `anthrex run status` is where the
  checkouts are listed.

## From M8a's final fix batch F2 (2026-09-25), for the user and for M8a/M8b

- **For the user: Codex marks anthrex's checkouts trusted in your `~/.codex/config.toml`,
  and anthrex cannot stop it.** On a `workspace-write` turn (every Codex worker), Codex
  writes `[projects."<project root>"] trust_level = "trusted"` into the user's own
  `~/.codex/config.toml` by itself (M8a.1 item 7a). `-c projects."<root>".trust_level=...`
  on the command line does not prevent it. Since F1c a task checkout is a standalone
  repository, so the root is presumably the checkout (`<worktrees>/runs/<run>/<task>`,
  manual check 4e, F2 (2)): one entry per Codex worker task, left behind after the run.
  They grant nothing to the user's own repository, but they accumulate, and a later
  directory at the same path would be trusted. Remove them by hand; anthrex does not
  edit the user's Codex config.
- **C-I1's residual: a leftover process can write `.codex` after the guard's check.**
  The guard runs just before each Codex spawn. A `setsid` child that outlives its turn
  could write `<checkout>/.codex/config.toml` between the check and Codex's read. What
  is closed, since F2 round 2:
  - a confined check, proof or `setup` (the F1d profile denies the protected paths);
  - a Claude worker's or reviewer's `Bash` (`sandbox.filesystem.denyWrite`). Seatbelt
    restrictions are inherited by every descendant, so an escaped child is still denied.

  What is open is a Codex worker's own children. The legacy `sandbox_workspace_write`
  has no deny list. codex-cli 0.156's `[permissions.<name>]` profiles have `deny`
  filesystem entries, but the binary says a profile cannot yet grant writes outside the
  workspace root, which F1c's grant needs (the task git dir and tmp). The binary may
  also protect `.codex` and `.agents` itself. Manual check 4e, F2 round 2 (6), has the
  probe. A route edit that puts a task whose `owns` names a `.codex` path on Codex could
  also be refused up front, since the guard blocks it anyway (the F2 review's item 4).
- **Cross-cutting: worker children that escape their session's process group survive
  the turn.** Evidence:
  - The session waiter kills only the leader's process group, in its `WNOWAIT` window
    (`headless/session.rs`, `signal_locked` → `killpg`).
  - Before the next turn, the manager waits only for the leader (`is_ended()` in
    `manager/headless_turns.rs`).
  - Any descendant that called `setsid`/`setpgid` (seatbelt forbids neither) is not
    signalled. `session.rs`'s `OUTPUT_GRACE` already allows for "a process that escaped
    the group" holding a pipe.
  - Whether Codex puts its own tool commands in separate process groups, which would
    make an ordinary `cmd &` survive too, is unverified.

  Consequences:
  - a sandboxed survivor keeps its sandbox, and so its write grant (the checkout, its
    objects, its tmp) into later turns and later engine operations on that checkout;
  - an unsandboxed one (`worker_sandbox = false`) keeps everything.

  Not changed in F2: kill and process-scan code is under the safety rule. A fix to
  weigh: at retire and at task end, find the survivors by a marker the daemon controls
  (a per-session environment token, or a sandbox extension) and signal them by exact
  pid, never by pattern. That fix needs its own review under that rule.
- **`.agents/` is not guarded.** Codex reads repository skills (`.agents/skills`) and a
  plugin marketplace (`.agents/plugins/marketplace.json`) from the project. The
  protected list is the user's five paths verbatim; whether to add `.agents/**` to it,
  and to the Codex guard, is the user's decision.
- **A task that changes `.codex/**` runs on Claude only.** The guard refuses every Codex
  session on a checkout whose `.codex` differs from the base, the task's own Codex
  reviewer included; such a task needs a Claude worker and a Claude reviewer (a route
  edit), and later Codex tasks on the run branch are refused as well.
- **Review C, M1: a failed Claude `headless_send` leaves its turn recorded.**
  `manager/headless_turns.rs` records the turn in the cursor, then enqueues; a full
  queue or closed stdin leaves a `pending_sent` entry no prompt hook will match, so a
  background `result` before the next delivery is classed as `Unprompted`. Fix:
  enqueue first, under the lock (`send_line` does not block), and record only on
  success. Not done in F2: testing it needs an enqueue that fails and a later one that
  succeeds.
- **Review C, M4: the tool gate knows a session only by its window id.** With
  `worker_sandbox = false`, a worker that reaches the socket can call
  `submit_review approve` for its own task. The report now says so. A per-session
  unguessable token in `anthrex mcp`'s argv, checked by the engine, would close it.
- **Review C, M7: a restored headless window whose `run` does not parse comes back as a
  PTY window**, so `headless_guard` no longer refuses `Restart`, and C-b R would start an
  interactive `claude` in the task checkout on the user's normal settings. Restoring it
  as a headless window with no spec (or a `Dormant` that refuses restart) would keep
  decision 49's refusals.
- **F2 round 2's costs.**
  - A profile can no longer set the shell and interpreter start-up variables
    (`PYTHONPATH`, `NODE_PATH`, `RUBYLIB`, `XDG_*`, `SHELL`, ...). A check sets them in its
    own command.
  - A Codex user who authenticates only through an exported `OPENAI_API_KEY` or
    `CODEX_API_KEY` must run `codex login --with-api-key` instead, since sessions drop
    the inherited keys.
  - A confined check or `setup` that writes `CLAUDE.md`, `AGENTS.md`, `.mcp.json`,
    `.claude/` or `.codex/` in its checkout now fails.
- **C-I4's cost: a profile cannot set `PATH`, `HOME`, a proxy or a CA bundle.** They come
  from the daemon's environment. A project whose checks need a tool on a
  repository-relative path must name it in the command (`./node_modules/.bin/x`).

## From M8a's final fix batch F3 (2026-09-25), for M8a/M8b

- **`max_writers` can be exceeded by a task that comes back to `working` (final review
  A-I2, cap half; recorded, not changed).** Decision 41 frees a writer slot at `review`
  and `merge_queue` and takes one "again while a handed-back task is `working`"; the
  return paths (rung 1 after a rejection or a red candidate, rung 2, a hand-back, an
  answer, `run retry`) take it whether or not one is free. Holding a returning task until
  a slot frees needs a wait state on each of those five paths; counting `review` and
  `merge_queue` against the cap at dispatch contradicts decision 41's "hold none". The
  over-subscription is bounded by the tasks in those states. F3 closed the hub half.
- **A blocked non-hub task can come back beside a running hub task (A-I2 residual).** A
  hub task now waits for every task in `review` or `merge_queue` (they come back on their
  own) and keeps the hub from its start until it finishes, but it does not wait for
  `blocked` tasks: an answer, a retry or an override returns one to `working` while the
  hub runs. Waiting for them could deadlock (a task held on the hub as a dependency, or
  one the user never unblocks).
- **A panic in an effect, not in `step`, still ends the event loop (B-I2 residual).**
  (F4, F3 review N3: preparing a step's effects, the snapshot included, is now guarded
  too, in the loop and at restore; a panic there costs that publish only.)
  `step` runs under `catch_unwind` (state put back, the request failed, the loop kept),
  and a run whose restore panics is left out. A panic in the driver's own effect code
  (`execute`) still ends the loop task with no log and leaves waiting requests hanging.
  A data-dependent panic in the scheduler, which runs over every run on every step, makes
  every step fail until the daemon restarts without that run; each is logged.
- **B-6 (not done: process-kill code).** `kill_leftovers` takes its candidates from
  `run.json`'s round pids only; a pid that reached only the journal's `done` line of a
  `CreateWindow` is not examined. The fix changes which pids the leftover-session killer
  examines, so under the process-kill safety rule it is left for a change reviewed line
  by line.
- **B-7 (not done: process-kill code).** Orphaned `setup`, `check` and proof shells of a
  dead daemon are neither found nor killed, and a re-issued op runs beside them. Finding
  them needs a recorded pgid and a kill at reconcile, which is process-kill code.
- **B-5's other half.** A restored run with a leftover session still costs up to 2 s of
  SIGTERM grace, serially, before the socket is bound (`kill_leftovers`, process-kill
  code). F3 removed the unconditional `run.json` rewrite and journal compaction of
  unchanged runs.
- **B-10 residuals.** (F4, F3 review N2: a second pid-0 synthetic exit is no longer
  taken for a repeat, so a second kill of a reopened round ends it.) A synthetic exit
  carries pid 0 when the round never recorded one;
  the real exit of that unrecorded process is then not recognised as a repeat. And a
  Claude process's last `TurnEnded` (its usage) that lands after its round ended is
  still ignored, so that spend is not counted.
- **The pre-`CreateWindow` event race is still open** (the From M8a.25 entry): F3 made
  the test that reproduces it wait on the condition (`run.json` records the session's
  window) instead of a 4 s sleep (T25-N2), and assert the race was reproduced
  (T25-N3). Buffering a window's signals until its round has it was not cheap.
- **A fresh session's hand-over diff is `start..HEAD` (A-I5's sibling).** The review
  diff is now the task's net change from its merge base with the run head; the rung-2
  hand-over prompt's `git diff --stat <start>..HEAD` and diff (decision 30) still include
  every hand-back's merged work.

## From M8a's final fix batch F4 (2026-09-25), for the user and for M8a/M8b

- **Claude sessions no longer deny nested `CLAUDE.md`/`AGENTS.md` writes (F2 re-review
  I2).** Published packages ship these files (recharts ships `AGENTS.md`), and Claude's
  `denyWrite` globs cannot leave `node_modules`, `.venv` or `vendor` out, so the
  `<checkout>/**/…` entries made `rm -rf node_modules` and pnpm/bun installs fail. A
  Claude session now denies only the root files, `.claude`, `.codex` and `.mcp.json`.
  Nested ones are judged by the done gate (any depth, any case, tracked changes), and
  Codex reads `AGENTS.md` only from the root down to its cwd, the root. The seatbelt
  profile of checks, proofs and `setup` keeps the nested deny outside dependency trees
  (`seatbelt::DEPENDENCY_DIRS`), but only against direct creation: a confined check can
  write `node_modules/d/CLAUDE.md` and `mv node_modules/d src/d`, leaving
  `src/d/CLAUDE.md` (a directory rename is checked on the directory, not its contents;
  F4 review M1). For nested files the done gate is the barrier. A worker (or its leftover child) can still plant a
  nested `CLAUDE.md` that a later Claude session of the same task reads when it works
  in that directory.
- **Letter case (F2 re-review I1) was not reproducible on macOS 26.2.** Seatbelt's
  `literal`, `subpath` and `regex` filters already matched `.CODEX`, `agents.md`,
  `sub/Claude.md` and `.MCP.JSON` on the default case-insensitive APFS volume (probed
  with `sandbox-exec` directly, F4 report). The confined profile now spells the names
  with bracket classes anyway; Claude's entries rely on the kernel's folding (manual
  check 4e, F4). A case-sensitive volume needs no folding.
- **F2 re-review M2 (optional): Claude's `Edit`/`Write` tools** could get the same
  protected list as `permissions.deny` `Edit(//<abs>)` entries. Not done: the rule
  syntax is unverified against the real CLI, and the C-I1 race involves leftover
  children, which the sandbox covers.
- **F2 re-review M4: other inherited secrets reach checks.** `GITHUB_TOKEN`, `GH_TOKEN`,
  `AWS_*`, `NPM_TOKEN` and the like are inherited by checks, proofs and `setup`; with
  the user's `confined_network`, worker-written test code can send them out. An
  allow-list for `engine_env` is the real fix; it needs the user to decide which
  variables a project's checks may see.
- **F4 review M2: `prepare_guarded`'s double panic stalls a run.** If `effects::prepare`
  panics and the publish-free retry panics too, nothing runs: the step's new
  `pending_ops` and bumped revision are in the in-memory state but never executed and
  never saved, so memory and disk diverge until the next restart, and a run waiting on
  such an op stalls. Needs a panic in `prepare`'s mapping code (very unlikely). Fix:
  on a double panic restore the pre-step runs, as `guarded_step` does
  (`crates/daemon/src/run/driver/guard.rs`, `prepare_guarded_with`).
- **F3 review N7: `guarded_step` clones the whole `EngineState` per event** under the
  engine lock (a second clone beside `step`'s own `before`). A performance cost, not a
  correctness one; `step` could hand its `before` back on the panic path.
- **F3 review N4: a note-only restore does not bump the run's revision.** F4 saves such
  a run (its `as_loaded` is now taken before the restore's notes), but
  `engine::restore` compares against the runs as passed in, which already hold the
  notes. Clients re-read snapshots after a daemon start, so nothing shows stale.
- **B-8 (final review B): retired headless windows lost at a crash stay listed.** A
  dormant `Exited` headless window of a run counts against `max_windows` until the run
  finishes, and decision 49 lets no one else remove it. Not fixed: it needs the
  window-removal path the restore already walks (`remove_stale_windows`) to know which
  of a live run's windows no round will resume.
- **E-M8: the e2e harness's fallback kill leaves the daemon's headless children
  running.** When `anthrex daemon stop` times out, `DaemonProcess::kill` SIGKILLs the
  daemon alone; its `fake-agent` sessions (own process groups) keep running up to their
  own timeouts. A leak of test processes, not a safety problem; any fix must stay
  scoped to recorded pids (process-kill safety rule).
- **T20-P1, T20-P2 (fake-agent signal handling, not changed: kill code).** A SIGTERM
  that lands while an interrupted `sh` is being killed (`kill_tree` running `ps`) only
  sets the flag and is acted on at the next `sh`; a signal between the child's spawn and
  its pid being stored is likewise late. `kill_tree` runs `ps` with no timeout. The
  fake's `shell()` joins its drain threads after `sh` exits with no bound, so a
  background job holding the pipe blocks the step (pre-existing).
- **T21-P1..P3 (reconcile's leftover-session killer, not changed: process-scan code).**
  `reconcile_with_orphan_parent` is plain `pub` (should be `#[doc(hidden)]` or
  test-gated); the `round.ended` filter, the uid check and the non-group `kill` branch
  have no negative test (an ended round's recorded orphan must survive); the note says
  "killed" even when the signal failed with something other than `ESRCH`. Each needs a
  change reviewed line by line under the safety rule.
- **T22-P5: `run start`'s decision-53 refusal does not say why** a Claude-only plan
  reaches Codex (escalation or review) or that a one-runtime roster avoids it.
- **T23 M2: `run accept`'s pre-check is a snapshot.** The run is checked `complete`
  before any question, and the daemon checks again at the merge; a run that changes in
  between is refused by the daemon, in its own words.
- **`crates/config/src/lib.rs` is 605 lines** (600 on `main`; M8a added the
  `orchestrator` hooks) and `lib_tests.rs` 742 (741 on `main`). Left for a config
  change that has a reason to split them.

## From M8b.1 (2026-09-25), for the user and for M8b

- **Scouts' Bash can write their working directory (M8b.9; ruled R-T1-1: the checkout root goes in the scout's `denyWrite`).** With
  an empty `sandbox.filesystem.allowWrite`, Claude Code 2.1.280 still lets sandboxed
  Bash write the session's working directory (`touch x` succeeded). Adding the checkout
  root to `denyWrite` denies it while `ls`, `git log` and the MCP call keep working
  (fixtures `claude-2.1.280-scout*.jsonl`). M8a's reviewers are covered by `dontAsk`
  and their scoped `Bash(git …)` rules, not by the sandbox; worth confirming for any
  future role that allows unscoped Bash.
- **A worker's Bash does not see M8a's per-task `TMPDIR` (M8a, recorded).** Claude
  Code's sandbox sets `TMPDIR` to its own `/tmp/claude-<uid>` inside the Bash tool,
  and that directory is writable to every sandboxed session, scouts and reviewers
  included. Decision 28's filter log is unaffected (absolute `--log-dir`); anything
  that relies on `$TMPDIR` inside a worker's commands, or on it being private per task,
  is not.
- **A rewritten Bash call shows its original command in the stream (M8b.6).** The
  assistant `tool_use` keeps the model's command; only the `tool_result` reflects the
  `PreToolUse` rewrite. Decision 37's `bash` step emits the final command.
- **Deciders could run with `--tools ""` (M8b.5, optional).** It leaves only
  `StructuredOutput` in `init.tools` (the decider otherwise sees `SendMessage`,
  `RemoteTrigger`, `Workflow`, `Skill` and more) and cut cache creation from 17 688 to
  10 763 tokens; the answer arrived unchanged. The user's `SessionStart` hooks still
  run. `--restricted` skips them but leaves 18 tools.
- **The triage decider can use all of `--max-turns 3`.** The model answered in text
  first, the CLI forced a `StructuredOutput` call, and `num_turns` was 3. M8b.7 should
  read the fenced text (decision 16's source 3) even when a turn ends early.

## From M8b.7 (2026-09-26), for the user and for M8b

- **An inherited `ANTHROPIC_BASE_URL` reaches every headless session, deciders
  included (resolved: user ruling 2026-09-29: scrub).** It is now in
  `config::reserved_env::API_CREDENTIALS`, so `credential_scrub_for` removes it wherever
  it removes the keys: every headless session, the orchestrator's PTY window and the
  engine's commands; a Claude `auth = "api_key"` session keeps it with its keys (M9
  brief, "User rulings 2026-09-29"). The original finding: `credential_scrub_for` removes `ANTHROPIC_API_KEY` and
  `ANTHROPIC_AUTH_TOKEN`, but not `ANTHROPIC_BASE_URL` (or any other `ANTHROPIC_*`
  endpoint variable). M8b.7's credential test recorded it in a decider's environment,
  inherited from the shell that ran the tests. A daemon started from such a shell sends
  every `claude -p` session, with the user's login, to that endpoint. Whether it should
  be scrubbed (like the keys) or kept (a proxy the user chose) is a policy question for
  the user; M8a's scrub list is the place.

## From M8b.8's review (2026-09-26), for the user and for M8b

- **filter-run's shell is not quite the Bash tool's shell (residue of review I1).**
  filter-run now runs `$SHELL -c` for bash and zsh, else `/bin/sh`, but Claude Code's
  Bash tool also sources its shell snapshot (the user's aliases, functions and options)
  before each command; a plain `$SHELL -c` does not, and zsh still reads `~/.zshenv`.
  A test command that relies on an alias or a snapshot-only option behaves differently
  once wrapped. On Linux, `/bin/sh` (dash) is used whenever `$SHELL` is another shell.
- **On a Bash-tool timeout a wrapped command shows nothing (review M3).** filter-run
  prints the view and the log path only after the command exits. A command the tool
  kills at its timeout (2 minutes by default) would have streamed partial output
  unwrapped; wrapped, the agent gets no output and no log path. A fix could print the
  log path first (to stderr) or stream a bounded head.
- **A background child that keeps the pipe open keeps filter-run waiting (review M6).**
  `(sleep 3 &); echo done` returns after 3.1 s. Same as the Bash tool itself; recorded.
- **A scoped `Bash(...)` allowlist entry and the rewrite are unverified (review M4).**
  Decision 28's safety premise is that the worker's `--allowedTools` already allows the
  rewritten command. That holds for M8a's default unscoped `Bash`, which M8b.1 probed.
  A user's scoped `worker_allowed_tools` entry such as `Bash(cargo test:*)` may not
  match `'<exe>' filter-run …` (or `cd <dir> && '<exe>' filter-run …`), so under
  `--permission-prompts none` the rewritten call could be denied. Probe it when real-CLI
  probes are next allowed, or skip the hook when the worker's Bash is scoped.

## From M8b.10's review (2026-09-26), for M8b and M8a

- **A user-immutable flag can still block a removal.** `checkout::remove` now gives the
  owner read, write and search access back on every real directory inside a checkout
  before removing it (review I2). It does not clear BSD file flags. A command that sets
  `chflags uchg` on a file in its checkout would make the removal fail each time. The
  failure is reported (the error names the salvage ref), not silent. Whether macOS's
  `(deny default)` profile lets a confined command set the flag is unverified: the
  grant is a `file-write*` subpath, which may include `file-write-flags`.
  `profile_verify_leftovers.rs`'s `a_removal_that_fails_after_a_salvage_names_the_ref`
  shows the failure path with a flag the test sets itself. A fix would clear `uchg`
  (`chflags(0)` on owner-owned entries, without following links) in
  `restore_owner_access`, or deny `file-write-flags` in the seatbelt profile.
- **`restore_owner_access` walks by path.** Each mode change is
  `fchmodat(AT_SYMLINK_NOFOLLOW)` after an `lstat`, so a link at the leaf is never
  followed. An intermediate directory swapped for a link between the walk's steps (by a
  process that escaped the killed group with `setsid`) could still redirect a later
  step, which would at most add owner `rwx` to a directory the user owns. An
  `openat`/`fchmodat`-relative walk would close it.

## From M8b.11 (2026-09-26), for M8b

- **Both M8b.11 findings first recorded here were fixed in M8b.11's review round:**
  the M8b.9 `{"hang": true}` fixtures, and the staleness check at daemon start.
- **A leftover detection checkout with no trusted project is kept, and only logged (M8b.11
  re-review C1).** Restore runs no git for it, so it is neither salvaged nor removed.
  `anthrex profile status` could list such leftovers, and a command could let the user
  name the project to salvage into, or discard them.
- **`ProfileService.writes` is one mutex for every project (review m4).** `reject` holds
  it across `discard_checkout`, which waits on the project's git write queue, so a reject
  behind a long write in one project delays proposal saves in every project. It is not
  a deadlock. Per-project mutexes, or dropping the guard before the discard, would fix it.

## From M8b.14's review (2026-09-27), for M8b

- **`run start --goal` replies "detection has started" even when detection failed at once
  (review m5).** When `ProfileService::detect_or_record`'s start fails immediately, for
  example on the project-settings refusal or the confinement refusal, the goal still
  answers `DETECTION_STARTED` (`crates/daemon/src/run/driver/adapt_goal.rs`, `no_profile`).
  The failure is visible only in `anthrex profile status`. The text is the brief's exact
  wording (decision 22 step 2), so it is not a defect. A follow-up could append the
  proposal's `Failed` reason when the restart failed.

## From M8b.15 (2026-09-27), for M8b and M9

- **`TokenUsage`'s `+=` saturates now; `engine/signals.rs`'s field sums do not.** The
  M8b.15 fix made `proto::TokenUsage`'s `AddAssign` saturate (`fix(proto): saturate
  TokenUsage addition`). `engine/signals.rs` still adds each field with a plain `u64 +=`
  (`total.input += usage.input`), which panics on overflow in a debug build and wraps in a
  release build. The numbers come from a session's own stream, so an overflow needs a
  session to report tokens near `u64::MAX`. Summing with `TokenUsage`'s `+=` there would
  cover it. M8b.16's history records should sum with `+=` too.
- **The OTLP receiver is a small HTTP/1.1 server with no `Expect: 100-continue` support.**
  Claude Code's exporter (OTel JS 0.208) does not send it (M8b.1 item 5). A client that
  does would wait `OTLP_READ_TIMEOUT` and be closed. M9 should check this when it
  launches the orchestrator window with `orchestrator_env`.

## From M8b.15's review (2026-09-27), for M9

- **Orchestrator usage is not authenticated.** The OTLP receiver listens on loopback with
  no credential, so any local process can post points for a live run and inflate its
  `orchestrator_usage` (a run id is visible to anyone who can read the run's files or
  guess it). The M8b.15 fix bounds the damage: only live runs are metered, the engine
  gets one coalesced total per run per drain, and a restart never lowers the stored
  usage. Nothing gates on usage, so inflation only makes the numbers shown untrue. M9
  could give each orchestrator window a per-run token (an `OTEL_EXPORTER_OTLP_HEADERS`
  bearer value) and drop points whose header does not match the run.
- **Size the OTLP connection cap from the number of concurrent runs.**
  `OTLP_MAX_CONNECTIONS` is 8. Each M9 orchestrator keeps one keep-alive connection and
  exports every second, so it never reaches the 10 s idle close. With 9 or more
  concurrent runs, the 9th orchestrator waits `OTLP_SLOT_WAIT` (1 s) for a slot and is
  closed, on every attempt. M9 should derive the cap from the maximum number of
  concurrent runs, with a margin, when it starts orchestrator windows. Any local process
  can also hold every slot by reconnecting every 10 s; a per-run token (above) would let
  the receiver close unauthenticated connections first.
- **The ledger's series cap is global.** `MAX_SERIES` (4096) bounds every cumulative
  series across live runs, so a local process posting many cumulative series for one live
  run can stop new cumulative series of another live run from counting until that run
  ends and is evicted. Claude Code exports deltas (M8b.1 item 5), which keep no series,
  so the orchestrator is not affected today. A per-run series cap would close it if M9
  meets a cumulative exporter.

## From M8b.16's review (2026-09-27), for M8b

- **Phase times are wall-clock, so they include daemon downtime, pauses and the plan-gate
  wait.** `run/phases.rs::set_state` adds `now - phase_since` to the phase of the state a
  task leaves. A daemon that is down, a paused run, and the `queued` time of a task
  re-asserted by `requeue` while the plan awaits approval (`engine/dispatch.rs`) all count.
  Spec §15 asks for wall-clock per phase, and the inflation only loosens M8b.17's budget
  refits, so M8b.16 keeps it (controller ruling m5). The fix: at restore
  (`engine/restore.rs`, next to `clock::stop_at_restore`), close each task's open phase
  at the task clock's `stopped` time and restart it at `now`; and leave the time a run
  spends paused, and a task's time before the plan is approved, out of its phases.

## From M8b.17's review (2026-09-27), for M8b and M9.5

- **A revert of a revert does not clear the reverted count.** *(Done in milestone 9.5, task M9.5.4a.)* If the user reverts a task
  merge (or a run's accept merge) and later reverts that revert, the re-apply's message
  names the first revert commit, which is no candidate, so `detect_reverts`
  (`run/history_io.rs`) records nothing and `run/stats.rs` keeps counting the task as
  reverted. Recording only, so M8b keeps it (controller ruling m3). It matters to M9.5,
  whose refit uses "merged, unreverted" tasks per class. The fix: treat a revert commit
  already recorded as a candidate too, and on a revert of it record a `reinstated` line
  (or drop the revert record's effect) keyed by the original `reverted` sha.

## From M8b.18's review (2026-09-27), for M8b

- **Wider prefix matching wraps more compound commands (review m3).** Since M8b.18 a
  filter prefix ending in a separator (`sh tests/`) matches the path after it, and
  M8b.19 cuts a derived prefix back to its last separator when `{test}` is glued to a
  word (`pytest tests/test_{test}.py` derives `pytest tests/`). Both widen what the
  hook wraps. A wrapped command runs in filter-run's own `$SHELL -c`, so a compound
  command that sets Bash-session state (`export X=1; pytest tests/…`, `cd sub && …`
  beyond the leading `cd` the matcher strips, `source venv/bin/activate && …`) loses
  that state for the agent's next call, and a trailing `&` leaves a background child
  holding filter-run's pipe (see M8b.8's review M6), so the call returns only when the
  child exits. A fix could skip the rewrite when the command contains `;`, `&&` after
  the matched part, `export`, `source` or a trailing `&`.
- **Empty per-task TMPDIRs pile up under `/tmp/ax-<uid>/` (review m5).** About 3000
  empty directories (`/tmp/ax-501/<16 hex>/`, from September 25 to 27) were left by e2e
  runs on this machine. Each is a task's short `TMPDIR` (`run/git/tmp.rs`'s `task_tmp`, via
  `role_launch::task_tmp_dir`); the task's checkout and repository directory are removed,
  but the empty short directory is not. The engine's task cleanup (or the harness's
  teardown) should remove it once the task's checkout is gone.

## From M8b's whole-branch review (2026-09-27), for M8b, M9 and M9.5

- **A TUI form for the profile (M9).** `anthrex profile edit` is the only editor.
- **The output filter for Codex workers (M9.5).** *(Done in milestone 9.5, task M9.5.19, by instruction only: a Codex worker's `developer_instructions` carry the filter note. The hook path waits for the Codex facts M9.5.1 left for the user to record; see "From milestone 9.5".)* Decision 28 wraps Claude workers' Bash through the PreToolUse hook; Codex has no equivalent hook yet.
- **Pruning `history.jsonl`.** It only grows; readers keep the last line per `record_id`.
- **Routing and threshold proposals in `run stats` (M9.5).** *(Done in milestone 9.5, tasks M9.5.8 and M9.5.11.)* Stats only reports today.
- **A leftover scout process after a daemon crash is not killed (decision 11).** Weigh it under the process-kill safety rule: kill only by an exact recorded pid, never by pattern.
- **`run::git::checkout::default_repo_dir`** (`<parent>/.anthrex/<name>`) is used only by tests' convenience wrappers and should not be reachable from daemon code (a guard or a rename).
- **The profile fingerprint walk checks a path and then opens it** (M8b.4), a check-then-open race; open with `O_NOFOLLOW` and check the opened handle instead.
- **Smoke stage 11d's `profile status` timeout** (`scripts/pty_smoke_adapt.py`) uses `RUN_REQUEST_TIMEOUT` (`crates/cli/src/profile_cmd.rs`) without a derivation row in `docs/timing-budgets.md`.
- **Derived filter prefixes.** `check` can still derive a prefix made only of punctuation (the alphanumeric guard covers only `single_test`), and a `single_test` whose `{test}` is glued inside its first word (`./run_{test}`) derives no prefix, so that command is never filtered.

## From M8b's whole-branch re-review (2026-09-27), for M8a's protected-file gate and M8b

- **Non-ASCII case folding in `ProtectedMatcher` (security, predates M8b).** `run/globs.rs`'s matcher folds case only for ASCII, but the default case-insensitive macOS volume also folds letters such as `ſ` (U+017F) onto `s`. So `owns = ["AGENTſ.md"]` passes the done gate and the file opens as `AGENTS.md` (`.mcp.jſon` likewise). Nothing reaches the base branch before accept, where the change shows, but under a lookalike name. M8b closes it for the fast path only (a non-ASCII `owns` entry leaves the fast path); the gate itself should compare Unicode-case-folded, normalised paths, or refuse non-ASCII protected-lookalike paths, for planned runs too.
- **`owned_protected` skips its checks when the protected list fails to compile** (`run/triage.rs`), where the done gate refuses. Unreachable today (the list is validated when the run is built); make it refuse.
- **`{` is not a glob character** in `run/globs.rs`, so `owns = ["{AGENTS.md,x}"]` is not routed to the plan path; the done gate still bounces the change, so the only effect is a failed fast run.
- **`AmendTask { size: L }` still applies to a fast-path run.** User-initiated, so not a bypass; consider refusing it like `add_task`.

## From M8b's Linux CI audit (2026-09-27), watch items

- **`profile_cli_refusals` "stopping" refusal is timing-dependent.** `crates/cli/tests/profile_cli_refusals.rs:~196` expects the "stopping" refusal right after `reject`, but `detect` runs preflight's git calls before it checks the job. If cleanup wins, the reply differs. It has passed on both CI platforms; if it ever flakes, hold the stopping state deterministically (for example a verification command that hangs until released).
- **ETXTBSY on freshly written stand-in scripts (Linux).** Tests write a script and the daemon executes it directly (`ANTHREX_DECIDER_BIN`, scout and hook stand-ins). A concurrent fork in another test thread can briefly inherit the write fd. A rename does not help, because ETXTBSY is per inode. If it appears, retry the spawn on ETXTBSY in the test harness, or exec through `/bin/sh <script>`. It appeared on ubuntu CI twice (PR #21: `an_interrupted_import_is_finished_by_the_next` and `pseudo_refs_planted_after_the_checks_steer_nothing`), both through `tests/support/run_git.rs::wrapper_git`, which now waits with a probe run until the script executes. The other stand-in writers (decider, scout, hook scripts) still lack it; apply the same `settle` when one flakes.

## From the Claude tool-search fix (2026-09-27), for M8a and M9

- **What happened.** A real Claude worker told the user that no `task_done` tool was exposed, "confirmed via ToolSearch", and the engine fell back to its turn-end signal.
- **The probes, against `claude` 2.1.280.** The anthrex MCP server connected, and `mcp__anthrex__task_done` and `mcp__anthrex__task_blocked` were in `system/init`'s tool list. Claude Code defers MCP tools behind `ToolSearch`, so the model has to search to load one, and a loose keyword query can miss it. The user's own settings load their plugins and skills into the worker (`--setting-sources user`), which crowds the search results. With `ENABLE_TOOL_SEARCH=false` in the environment, `ToolSearch` is absent and the MCP tools load directly (checked in `system/init`'s tool list). In an isolated rerun the worker happened to query `select:mcp__anthrex__task_done`, and that worked. So the failure depends on the model's query, which is why it is intermittent.
- **The fix.** `HeadlessHandle::spawn`, the one start of every headless session (workers, reviewers, area and onboarding scouts, deciders), sets `ENABLE_TOOL_SEARCH=false` last on a Claude session (`headless::session_vars`, `config::reserved_env::CLAUDE_TOOL_SEARCH`). Neither the daemon's inherited environment nor a spec's `env` can override it, and a profile's `env` may not set the name. The user's own PTY windows (`anthrex new --runtime claude`, the orchestrator) are unchanged. The worker, reviewer and scout contracts name each tool by its Claude id, for example `task_done (in Claude: mcp__anthrex__task_done)`. That wording stays correct for Codex, which names MCP tools differently. It deviates from the exact contract texts in the M8a and M8b briefs.
- **Not fixed: `packed-refs.lock` under the worker sandbox (owner M9).** In the same run, a Claude worker's commit printed `Unable to create …/tasks/<t>/git/packed-refs.lock: Operation not permitted`. The commit still succeeded. The sandbox was deliberately not widened: a writable `packed-refs` would let a worker forge refs in the task's repository. Whether git should be kept from packing refs there, or the file allowed under some other guard, needs a design decision. **Resolved by M9 task M9.13c (no git change):** M9.1 check 11 found the lock is taken by the commit itself, when it deletes the merge-state refs after moving `HEAD`, and no config key stops that, so the sandbox stays as it is and the warning stays. The worker contract's new line 12 says the warning is expected, the commit succeeded, and nothing needs fixing. `run_git_sandbox.rs::packed_refs_stays_read_only_to_a_worker` pins both halves: `packed-refs` and its lock are denied, and the commit exits 0.

- **Precedence, probed 2026-09-27 against `claude` 2.1.280.** With `ENABLE_TOOL_SEARCH=true` in the process environment and `"env": {"ENABLE_TOOL_SEARCH": "false"}` in `--settings`, `ToolSearch` stayed present. The process environment decides tool search, and a settings `env` block does not override it. So an `env` block in the user's own `~/.claude/settings.json` (loaded by `--setting-sources user`) cannot undo the daemon's pin, and repeating the pin in `--settings` would add nothing. Not verified further, because that would mean editing `~/.claude`.
- **Test hygiene (review note, pre-existing).** `crates/daemon/tests/headless_env.rs` removes its windows only after its assertions, so a failing assertion leaves the stand-in's `sleep 30` running, reparented to pid 1, until it exits on its own. The cheap fix is a drop guard that calls `m.remove(id)`, the daemon's own exact-pid kill.

## From the main-branch CI failures (2026-09-23), deliberately deferred

- **The main pane can switch to a new window while the new-agent form is still open and
  swallowing keys.** A client with nothing focused focuses the first window a
  `WindowsChanged` lists (`crates/tui/src/app/windows.rs`, the `ensure_focus` fallback).
  The daemon can send that `WindowsChanged` before the `Created` reply that closes the form
  (`crates/daemon/src/server/requests.rs`, `create`). Until `Created` arrives, the new
  window's title shows behind a form that is still `submitting`, and `NewAgentForm::on_key`
  drops every key but `Esc` and `Ctrl-C`. Ubuntu CI run 35792210390 lost the leading `e` of
  `echo smoke-$((40+2))` this way. `scripts/pty-smoke.py` now waits for the form to close
  as well as for the title. Delaying the `Created` reply by 1 s reproduces the failure every
  time; with the new wait the smoke passes. The client still behaves this way: the form is
  visible while it happens, so a user is unlikely to type into it. The remaining question
  is whether focus should wait for `Created` while a create is submitting. Belongs with the
  next TUI milestone that touches the form.
- **`git_registry.rs`'s `a_root_survives_until_every_registration_is_released` was flaky;
  fixed.** PR #12 changed `run_root` so that a root asks for one more probe when its watcher
  arms. The test read its probe count as a task count right after the first publication. On
  Linux, inotify arms fast enough that the extra probe could land before that read
  (`left: 2`, ubuntu CI run 35831045650, the first recorded occurrence). A forced ordering,
  with the watcher armed before the registration probe returns, failed 50 times out of 50.
  The test now uses a watcher that fails to arm, so only the poll probes. The other
  exact-count reads in that file come after a 120 s virtual settling window, so the extra
  probe cannot reach them.
- **`WindowEvent::Exited` is not ordered after a window's final output.** `Window::spawn`
  sends `Exited` from the `pty-wait` thread and feeds the screen from the `pty-read` thread,
  with nothing ordering the two (`crates/daemon/src/window.rs`). A child's last bytes can
  still be unread when its exit is reported; they reach the screen shortly afterwards.
  Ubuntu CI run 35832528546 failed `missing_binary_shows_error_in_window_and_exits_127` with
  `screen: ""` because it read the screen right after `Exited`; a 200 ms sleep in the reader
  reproduces that every time on macOS. The test now waits for the text with a deadline. The
  daemon keeps the window and its screen after exit, so no output is lost. Anything that
  should see the final screen at the moment of exit (an exit summary, a status rule that
  reads the last lines) would need the waiter to wait, bounded, for the reader to drain.

## From M8c.1's review (2026-09-27), for M8c and M9

- **The promotion time in the run view (M8c).** The snapshot's promotion attention line no longer carries a time (M8c.1 review I1; brief R17 had assumed the line carried it). The view's gate or attention rendering should show `local_hhmm(RunInfo.promote_requested_at)` beside it. *Handled in M8c.7:* the `attention` row shows the line as `promotion requested at <local hh:mm>; …`.
- **`run promote`'s repeat reply is in UTC (M9).** `engine/requests.rs::promote` answers a second request with `was already marked for promotion at <hh:mm>` through `model_adapt::hh_mm`, in UTC. It is a request reply, not the snapshot, so M8c.1 left it; either drop the time or let the CLI format `promote_requested_at` locally.

## From M8c.7 (2026-09-27), for proto

- **`TokenUsage::billable()` can overflow.** `proto/src/run_info.rs` adds `input + cache_write + output` with plain `+`, so a hostile or corrupt usage report (any field near `u64::MAX`) panics a debug build and wraps in release. `AddAssign` for the same type already saturates. The run inspector computes the same sum with `saturating_add` (`inspector/run_format.rs::billable`) rather than touch `proto`; `billable()` itself should saturate. Its other callers are all in the daemon, none in the CLI: `run/engine/signals.rs:181, 240` (a round's reported spend added to `spent_total.tokens`, the budget `check_budget` enforces; a hostile stream report reaches this first, so it is the most exposed), `run/engine/ladder.rs:351` (a round's `Spend`), `run/report_task.rs:162` (the report's per-round tokens) and `run/stats.rs:50-52` (the per-task totals). *(Corrected by M8c.7's review m5: this entry had named `run status`, which does not call it.)*

## From M8c.7's review (2026-09-27), for M8c

- **The TUI's text scrubbing lets bidi and format characters through.** `inspector/run_format.rs::clean` maps only `char::is_control` (Unicode category Cc) to a space. Format characters (Cf) such as U+202E RIGHT-TO-LEFT OVERRIDE, U+2066–U+2069 isolates and zero-width characters reach the terminal from daemon-supplied text (goals, titles, branches, findings, scout questions, history). There is no escape injection, but a terminal may reorder or hide the text around them. The fix belongs in one shared sanitiser used by the whole TUI (the inspector, the canvas, the conversation view), not in the inspector's `clean` alone.

## From milestone 8c (2026-09-27), recorded by M8c.10

- **Handled in milestone 9.0.5.** `TaskInfo.activity` now publishes the live round's
  latest action with its target, and the round panel's `doing` reads `now: <activity>`
  for that round (M9.0.5 decisions 2, 3 and 26).
  **The run inspector's `doing` field has no tool target** (brief Risks 6, for M9 or a
  later snapshot change). The spec's mockup shows `editing crates/daemon/src/status.rs
  (last tool: apply_patch)`; neither `AgentRoundInfo` nor `WindowInfo` carries a tool's
  target, only its name. The view does not read M6.5's conversation state for it, which
  would make the inspector depend on a subscription the user may not have open. Publishing
  the last tool call's target (a path, or a short argument) on the round or the window
  would fill it.
- **Fields of "Consumes from later milestones" still unfilled** (the view renders their
  absence as the brief states):
  - `RunInfo.estimate_left_secs` and `RunInfo.bound_ratio_permille`: **M9.5** *(Done in milestone 9.5, task M9.5.12.)* (history
    medians). Until then the view shows no `est. left` and no `× the bound`.
  - `RunInfo.planners` (`PlannerInfo`, `PlannerState`): **M9**. Until then every task
    hangs from the run's root.
  - The orchestrator's PTY window (`WindowInfo.run == Some(RunRef { role: Orchestrator,
    task_id: None, .. })`) and `AgentRole::Planner`: **M9**. Until then the root reads
    `run <h4>` and Enter on it toasts that the run has no orchestrator window.
  - `RunInfo.scouts` for a run's area scouts: **M9** (M8b declared the type and fills
    only the onboarding scout, which has no run).
  - Racer and test-writer rounds: **M9.5**. Not drawn until then. *(Done in milestone 9.5, task M9.5.20.)*
- **`run promote`'s repeat reply is in UTC** (M9): already filed under "From M8c.1's
  review" above; not repeated here.
- **`TokenUsage::billable()` can overflow** (proto): already filed under "From M8c.7"
  above.
- **Bidi and format characters pass the TUI's text scrubbing** (the client): already
  filed under "From M8c.7's review" above; still open, M8c did not take it.
- **Replies are matched by request name only** (M8c.9 review M7, for the milestone that
  next changes the run protocol). The protocol has no request id, so a `Refused { request:
  "run edit" }` for an earlier `d` (a `CancelTask`) that arrives while a later edit form
  is submitting fills that form's error and clears `submitting`; the form's own `Done`
  then only toasts, and the form stays open with the wrong error until Esc. It needs two
  replies to race at human speed. A request id echoed in `Done`/`Refused` would fix it and
  is a protocol change (a `PROTO_VERSION` bump).
- **`crates/daemon/src/headless/conversation.rs` is 452 lines**, over the 430 the M8c
  brief budgeted for M8c.11 (still under the 600 rule). The turn-end handling added by
  M8c.11's review fixes (the Claude `ProcessExited` arm, `hook_stopped`, `keep_stop`) is
  the natural piece to move into its own module the next time the file grows.
- **Every run's task briefs ride on every snapshot push** (whole-branch review M3, for
  **M9**). `crates/daemon/src/run/snapshot.rs:311-313` copies each task's `brief`,
  `acceptance` and `route_spec` into the snapshot for every run `EngineState.runs` holds,
  terminal runs included, and terminal runs are never removed from it. `snapshot()`
  publishes all of them on every structural change to every subscriber, though only the
  plan gate's edit form reads these fields. A push therefore grows without bound as run
  history accumulates, against `proto::MAX_FRAME` (16 MB); an oversized push fails
  `encode`. Not urgent at today's sizes. Suggested fix: publish `brief`, `acceptance` and
  `route_spec` only while the run is `awaiting_approval`, or prune terminal runs' task
  detail (or the runs themselves) from pushes.

- **A real Claude worker cannot see `mcp__anthrex__task_done` (found by Daniel's first real run, 2026-09-27; owner: next milestone, M9, before its e2e with real agents).** In a four-task run with real `claude` and `codex`, t1's Claude worker reported that no `task_done` tool was exposed ("confirmed via ToolSearch"), on its first session and again after the review bounce. The engine's turn-end fallback moved the task on (history `done (the turn-end fallback)`), so the run did not stall. In the same run the Codex workers' `task_done` and the Claude reviewer's `submit_review` both reached the engine. The worker's argv carries `--strict-mcp-config --mcp-config {anthrex: anthrex mcp --role worker …}` and `--allowedTools mcp__anthrex__task_done,…`, and its `anthrex mcp` process was running. The one difference from the working Claude reviewer is the worker sandbox (`sandbox.enabled`, `network.allowUnixSockets = []`, `allowAllUnixSockets = false`, `headless/argv.rs` `CLAUDE_SANDBOX_PINS`). The hypothesis to test first: the installed Claude Code runs stdio MCP servers inside the session sandbox, or defers MCP tools behind ToolSearch in a way `--allowedTools` does not surface. `forward.rs` connects to the daemon only per call, so `tools/list` needs no socket. Needs a probe with the real CLI (`claude --debug`, the M8a.1 way), never in tests; the fake agent cannot show it.

## From M9.10 (2026-09-28), for M9.5 (re-owned from M9.15 by M9's whole-branch review, 2026-09-29)

- **The TUI still refuses `C-b x` and `C-b X` for a placeholder headless window.** *(Done in milestone 9.5, task M9.5.6.)* Milestone 9 decision 11a restores an unparseable headless record as an exited headless window with no run, which the daemon lets a client kill and remove. `WindowInfo` does not tell it apart from a repository-level scout's window (both are `Headless` with `run: None`), so `app/headless.rs::headless_control_refusal` toasts the scout refusal and the user cannot remove it from the TUI; `anthrex rm <id>` works. A fix needs the snapshot to say so (a protocol field) or the daemon to answer the TUI's refusal case differently. *M9.15 (2026-09-29): still open; the brief's task text does not assign it and it needs that daemon change. The TUI offers no control the daemon refuses; it only over-refuses this one window, which `anthrex rm <id>` removes.*

## From M9.13's review (2026-09-29), for M9 — fixed

- **`run edit`'s and `edit_plan`'s project-settings refusal ignores the run's `--trust-project`.** `RunService::runtime_refusals` (`crates/daemon/src/run/driver/build.rs`) passes only the files the start recorded in `Run.trusted_project`, and its text (`edit_settings_refusal`) tells a user who did start with `--trust-project` to start a new run with it. M9.13's review fix persisted `Run.trust_project` and made `run promote` honour it; the edit path could read the same field. Left alone because it is M8a's ruling T22-I1b and the review named only promotion.
  - **And the promotion does not record what it trusted (M9.13 re-review).** A `run promote` let through by `trust_project` does not add the project-settings files its new orchestrator and sub-planners reach to `Run.trusted_project` (the check runs in the driver, before the engine's `Promote` event, which does not carry them). A later `run edit` that reaches the same runtime is then refused for files the promotion already accepted. Honouring `trust_project` in `runtime_refusals` (above) closes both; so would carrying the files on the `Promote` event.
  - *Fixed by M9's whole-branch review, item 4 (2026-09-29): `runtime_refusals` trusts every project-settings file for a run started with `--trust-project`, as `run promote` does; `an_edit_of_a_run_started_with_trust_project_is_not_refused` covers it. That closes both points above.*

## From M9.13a's re-review (2026-09-29), for M9.5 (re-owned from M9 by M9's whole-branch review)

- **A run head rewound below a refresh's target still shows the refreshed run files in the done check's spill diff and in `diff_so_far`.** `git::verify_done` computes the spill as `git diff --name-only <run_head>...<head>`, and `diff_so_far` as `<run_head>...HEAD`. After `resume --rebaseline` rewinds the run head below a run head that a refresh merged into the task, the merge base drops back, so the merged run files appear there: a spill outside `owns`, and more of the diff for the reviewer. The commit counts and `task_result` already leave that run work out, since the M9.13a re-review records every refresh's target (`TaskOrch.refresh_targets`). The same targets could serve as the diff base here: the newest one the head has. This was left alone because the re-review named only the counts and `task_result`, and the spill check is M8a's gate.

## From milestone 9 (2026-09-29)

- **Unconfirmed: a `run_status` read can leave one stale wake-up to be pasted later (M9.17 review).** `driver/orch.rs:333-338`'s `run_status` calls `Wakes::read`, then sends `DigestRead` asynchronously; a read that lands between a step's commit and that step's `queue_wake` may leave its wake-up waiting, pasted later for notes the read already showed, at the cost of one redundant orchestrator turn. Not reproduced.
- **Decision 16a's snapshot is bounded, not slimmed (controller ruling, M9's whole-branch review; for M9.5).** The brief's 256 KiB bound was accepted as measured: the snapshot pins what 16a removes, under 1.25 KiB a task (`SNAPSHOT_BOUND`). A terminal run still carries its tasks in full; slimming them is left to M9.5.
- **`OPENAI_BASE_URL` is not scrubbed from the environment a Codex session gets (M9's whole-branch review; resolved: user ruling 2026-09-29: scrub).** It is now in `config::reserved_env::OPENAI_CREDENTIALS`, so it is removed wherever the OpenAI and Codex keys are: every headless session, the orchestrator's PTY window and the engine's commands (M9 brief, "User rulings 2026-09-29", item 3). The original finding: the user's 2026-09-29 ruling named only `ANTHROPIC_BASE_URL`, which M9 scrubs. Whether Codex's base URL should be scrubbed the same way is the user's call.
- **Onboarding scouts and deciders have no installed check (M9.17 fix round 3; for M9.5).** *(Done in milestone 9.5, task M9.5.10b.)* Run scouts and sub-planners route over the runtimes the run's start found installed; the repository-level onboarding scout (`profile/service_run.rs`, `profile/service_start.rs`) and the deciders still route by the daemon-wide config. A Codex-only user with the default config gets a Claude onboarding scout and Claude deciders, which fail at launch (the deciders then fall back).

## From milestone 9.0.5 (2026-09-30), for M9.1 (tiered testing and flake handling)

- **Fixed in M9.0.5: `fake-agent/tests/orch_modes.rs` timed out at its first hook** ("the first
  Stop within 20s").
  - Root cause: every test wrote its own fresh `anthrex-wrap` script and execed it at its first
    hook. macOS assesses a newly written executable on its first exec (about 470 ms) and
    serialises those assessments. Measured: 4, 12 and 24 fresh files execed at once took up to
    1.6, 5.1 and 10.4 s, against 0.6 s for 12 or 24 execs of one file.
  - With twelve tests the worst case passed fake-agent's 5 s `STEP_TIMEOUT` for a hook, so
    fake-agent printed "step timed out after 5 seconds" and exited 1, and the test waited out
    its 20 s.
  - The `*_BIN` variables were not the cause; the earlier comparison was noise.
  - Fix, test support only: `orch_support::wrapper()` writes one shared stand-in per test
    process under `CARGO_TARGET_TMPDIR`, rewritten only when its text changes. Pinned by
    `every_orch_shares_one_wrapper`.
  - Repro: two copies of the test binary at once failed in 5 of 5 pairs before the fix and
    passed in 5 of 5 after, cold cache included. The binary now runs in about 1.5 s instead of
    6.5 s.
- **`run::driver::orch_ops::tests::resolve_target_op_runs_git_off_the_worker_threads` is load
  sensitive.** Its first, real `git rev-parse` has an 8 s timeout that expired once, under the
  full `cargo test --workspace` on a loaded machine (load average about 5 to 6). It then passed
  8 of 8 runs alone. It needs a measured budget per `docs/timing-budgets.md`.

## From milestone 9.0.5's whole-branch review (2026-09-30), for M9.5

- **The plan review has no indicator for new alerts.** *(Done in milestone 9.5, task M9.5.5b.)* While the review covers the screen, an alert
  that arrives shows nowhere: the status bar's `⚑ n` is drawn only when the sidebar is hidden, and
  the Alerts box sits under the review. Give the review's header or the status bar a count
  while it is open.

## From the user's first live orchestrator run (2026-09-29), for M9.1 and M9.5

- `edit_plan` was invisible to the orchestrator's first turn (M9.5): *(Done in milestone 9.5, task M9.5.5a.)* the call was refused by Claude Code as `No such tool available: mcp__anthrex__edit_plan` before reaching anthrex, and the model passed `edits` as a JSON string, so it never saw the schema. `/mcp` then listed the tool, and after the user reconnected, the same turn's plan submitted fine (run reached `awaiting_approval`). Cause: the anthrex server finished connecting after the session's tool list was built, around the folder-trust prompt. Fix direction: the orchestrator's first prompt should not be sent until the MCP server is connected (or the contract tells the orchestrator to call `get_context` first and retry once on a missing tool); pair with the trust-prompt item below.
- **The first orchestrator launch in a new folder stops at Claude's "Is this a project you trust?" prompt** *(Done in milestone 9.5, task M9.5.5b.)*, and the run view does not say so (M9.5). The run looks stuck until the user focuses the orchestrator window. Surface it as an attention line, the same way a held wake-up is surfaced.
- The plan gate does not show what is being approved (M9.5): *(Fixed by M9.0.5, found by the M9.5 refresh, 2026-10-03: the plan review's right pane shows `owns` and `done when` (`tui/src/app/plan_review.rs:205-209`), and the task inspector shows `owns` (`tui/src/inspector/run_task_sections.rs:173`) and `done when`. The edit form still edits only the brief.)* the TUI task inspector (`crates/tui/src/inspector/run_task.rs`) shows route, deps, budget and state, but not a task's `owns` or `acceptance`, and the edit form shows only the brief. Show `owns` and `acceptance` (and the brief's first lines) in the inspector at the gate.

## From M9.1.6 (2026-09-29), for any later milestone with `cargo nextest` and `go`

- **Replace the hand-written failing-test fixtures with captured output** (controller ruling C-2 on M9.1.1). `crates/daemon/src/run/tiers/fixtures/nextest.txt`, `go.txt`, `go-build-failed.txt`, `go-timeout.txt` and `go-testmain.txt` were written by hand from the tools' documented formats, because neither `cargo nextest` nor `go` was installed where the other fixtures were captured; each says so on its first line. On a machine with both, capture one passing, one failing and (for nextest) one timed-out test with stderr merged, as `libtest.txt` and `pytest.txt` were, and check `failing_names_from_libtest_nextest_pytest_and_go` still passes. The nextest filter syntax in M9.1's "Risks" stays unverified until then.

## From M9.1.10 (2026-09-30), for M9.1.21 or M9.5

- **A crashed rewrite of the tier caches leaves its temporary file behind.** *(Done in milestone 9.5, task M9.5.4a.)* `test-cache.jsonl` (M9.1.10) and `module-graph.json` (M9.1.7) are rewritten with `profile::store::write_atomic`, whose temporary file is `<file>.<pid>.<n>.tmp` in the repository's data directory. `profile::store::sweep_leftovers`, which `ProfileService::restore` runs at daemon start, removes only the profile store's own files' leftovers (`PROFILE_FILE`, `META_FILE`, `PROPOSAL_FILE`, `DETECTION_FILE`), so a daemon killed mid-rewrite leaves one stray file per crash. Harmless (nothing reads it), but it accumulates. Adding the two cache files to that list closes it.

## From M9.1.13 (2026-09-30), for M9.1.21 or M9.5

- **Two wall-clock tests have a 1 s margin against load.** *(Fixed, found by the M9.5 refresh, 2026-10-03: both tests now run on the `GitBudget` seam with the `driver/gated_git.rs` stand-in and assert only `>= DEADLINE`; `docs/timing-budgets.md`, "Fixed, from the M9 deadline-test flakes".)* `run/driver/orch_ops_tests.rs::resolve_target_op_runs_git_off_the_worker_threads` and `run/driver/orch_read_tests_tools.rs::task_result_answers_a_git_error_at_its_one_deadline` give a stand-in git that runs `sleep 8` a 9 s per-command bound (`SLOW_GIT_SECS = 8`, `git_timeout_secs = 9`) and expect `DONE_CHECK_GIT_TIMEOUT`'s 10 s deadline to answer first. When the rest of the daemon's library tests start many processes at the same moment, the shell and `sleep` start more than a second late, the 9 s bound fires first, and the answer is `… timed out after 8 s`. M9.1.11 saw the first one fail once; M9.1.13 saw both fail in every full run while a test of its own ran git through a shell wrapper (removed), and the first once after a rebuild. A bound derived per `docs/timing-budgets.md` (the per-command bound well above the stand-in's sleep, the deadline between them) would remove the race.

## From M9.1.18's review (2026-09-30), for M9.5

- **`run stats` still proposes a test that is already quarantined.** *(Done in milestone 9.5, task M9.5.4a.)* `stats::flaky_proposals` (decision 34) counts the `flaky` lines of `history.jsonl` and nothing else. `history_io::summarise` has no profile input, so a test the user has already added to `slow_tests` with `anthrex profile edit` keeps being proposed until its flaky lines age out of `flaky_window_days`, which is 14 days by default. Ruling C-23 accepted this as a limitation for 9.1. The fix is to give `summarise` the repository's stored profile (`slow_tests`) and drop the tests it already filters out. Matching a test name against a filter expression needs the runner's filter semantics (nextest `-E`, `--skip` patterns), which is why it was not done in 9.1.

## From M9.1.22 (2026-09-30), for M9.5

- **Stray step directories.** Every tier step gets a fresh `TMPDIR` (`<base>/s<op>-<step>`, M9.1 decision 28), which is removed after the step on the step's blocking thread. A step whose command started a daemon that escaped the process group (a test that daemonises, as `anthrex daemon start` does) can hold files in it past the removal, and a daemon killed mid-step never removes it. Nothing sweeps these directories. Fix: at daemon start, remove step directories under each repository's tmp base that are older than a day.
- **An `--ignored` job.** No tier runs ignored tests: tier 3's `check` is the profile's full suite as written, and no profile key names an ignored-tests command. A tier (or a tier-3 step) that runs them, from a new key such as `ignored_tests`, would give anthrex's own `#[ignore]`d tests a home (see the ignored-test note above, `:193-200`).
- **A hand-edited `run.json` with a gap in its stages** (stages 1 and 3 recorded, 2 missing) restores without complaint (ruling C-27, item 12; the whole-branch review's scenario s6c). What the run then does was only printed, never asserted. Decide whether a restore refuses such a run (halts it with a clear reason) or recreates the missing stage, and test that.

## From M9.1's final review (2026-09-30, ruling C-28), for M9.5

- **Three tier-3 states raise no alert.** *(Done in milestone 9.5, task M9.5.6.)* Milestone 9.0.5's decision 18 lists what the Alerts box shows, and none of these three appears in it:
  - A stage whose tier 3 is held after three executor failures (ruling C-18). It waits for `anthrex run resume`, and nothing but its attention line says so.
  - A red propagate (`StageInfo.propagate_red`).
  - A red tier 3 (`FullState::Red`), with or without a bisect fix task.
  Decision 18's priorities should gain these. The held stage needs a human, so it belongs with the halted run. The two reds belong with the blocked tasks, unless an orchestrator is live and will be woken.
- **The STATUS section's three stage labels read alike.** *(Fixed by M9.0.7 decision 15, found by the M9.5 refresh, 2026-10-03: the labels are `phase` (`tui/src/inspector/run_task_sections.rs:192`), `stage` reading `n of m` (`tui/src/inspector/run_stage.rs:192`) and `pipeline`.)* The inspector's STATUS shows `stage`, `stages` and `stage no.` (the last since the 9.0.5 merge renamed 9.1's duplicate row). Relabel them in M9.5 so each says what it counts, for example `stage state`, `stages in run` and `task's stage`, and update `run_stage_tests.rs`.
- **Two intermittent tests seen during C-28's verification, neither touched by it. Fixed.**
  - `crates/cli/tests/codex_version.rs::a_window_launch_waits_for_the_codex_probe_to_finish`: **fixed by `c44a805`.** The cause was macOS's first-exec assessment of the freshly written `codex` stub (~0.44 s, serialised across processes). The probe's budget started before the exec, so with the file's tests running together the 3 s stall sometimes ran out `CODEX_PROBE_TIMEOUT`. The probe was then killed before `version-end`, and the gate opened on schedule; the product was correct. The stub now runs once before the daemon starts. `a_window_launch…` also failed on `main`.
  - `crates/daemon/src/lockfile.rs::tests::try_claim_holds_the_lock_until_dropped`: **fixed by `9a0e41f`.** Close-on-exec was not the cause: `std::fs::OpenOptions` already sets `O_CLOEXEC`. The cause was the fork-before-exec window: a sibling test's `fork()` briefly holds a duplicate of the just-closed fd, and the `flock` with it. The zero-wait reacquire now runs in `crates/daemon/tests/lockfile.rs`, a process with no forking siblings.
  - The branch review found the same first-exec shape in `crates/cli/tests/decider_call.rs` ("the decider timed out after 5 s" in a full workspace run), `crates/daemon/tests/git_probe.rs` and `crates/daemon/tests/lifecycle_resume.rs`. All three were **fixed by `ac7b426`**; see `docs/timing-budgets.md`, "Fixed, from the decider-call flake".

## From the trial's headless-error fix (2026-10-01, rulings F-1 to F-4), for M9.5

- **Expose a round's failed turn in `AgentRoundInfo`.** *(Done in milestone 9.5, task M9.5.6.)* A round waiting out decision 32's continue has `failed_turn = WaitingContinue { at }` and `failed_error` in `run.json`, but the snapshot publishes neither: only a rate limit sets `rate_limited_until`, so the run view shows an `Other` failure's round as idle for `rate_limit_retry_secs`. Ruling F-2's history line is the stopgap. Add `failed_error: Option<String>` and `failed_until: Option<u64>` (`#[serde(default)]`) to `proto::AgentRoundInfo`, filled by `run/snapshot.rs`, and show them like a rate limit (the `◆` glyph and the inspector's STATUS). This is a protocol change: bump `PROTO_VERSION`, with a round-trip test.
- **`run retry` re-picks the route that just failed.** *(Done in milestone 9.5, task M9.5.10b.)* A task blocked on its environment by a reviewer whose roster entry cannot run (the trial's `codex`/`""` with a ChatGPT login refusing the configured model) is retried at rung 2, which re-resolves `review_route` through `pick_reviewer` and chooses the same entry again, so the retry repeats the failure unless the user edits the roster first. Consider excluding, for the retried task, the route whose session ended in a ruling F-1 client error (or any `blocked(environment)` from its own failed turn), falling back to `pick_reviewer`'s next candidate; record the skip in the routing history with a reason such as `failed in this task`, never as a failure label on an unchosen candidate.
- **Salvage refs for setup output alone.** In the trial every task, review and integration worktree was salvaged at cleanup (`refs/anthrex/salvage/<run>/<task>/<seq>` and `…/_full/1`), each salvage commit adding only `Cargo.lock`: the toy repository does not track it, and the profile's `setup = "cargo build"` writes it. M8a decision 20 requires exactly this (untracked files make a worktree dirty, only ignored ones are excluded, and accept keeps every salvage ref), so it is not a defect, but a report that lists salvage refs for a fully merged run reads as lost work. Options: diff the worktree's untracked files against what `setup` alone produces in a fresh worktree of the same commit and skip a salvage that holds nothing else; or have accept delete the salvage refs of merged tasks whose salvage tree differs from their merged head only by such files; either way the report should say why a ref exists.
- **Log why a wake-up waits.** *(Done in milestone 9.5, task M9.5.5b.)* In the trial the orchestrator was never woken (`wakes: 0`, `last_wake_rev: 0`) from plan approval (08:39) to completion (08:55), with notes queued throughout, and `state.json` recorded its window as `working` at both saves in that span (08:45, 09:02). The driver's gate (`run/driver/wake.rs::ready`: `Idle` or `Done`, no open `Attention`, no input for `wake_quiet_secs`) cannot be told apart from a window that never reached `Idle` without the window's status history or the orchestrator's transcript, neither of which the daemon log keeps. Log at `info`, once per pending wake-up and status change, why it waits (`window working`, `input 2s ago`, `attention open`), and the delivery itself; then a trial shows whether a real interactive Claude orchestrator's `Stop` hook reaches the window after its turn ends.

## From M9.0.6.14 (2026-10-01), for M9.0.7 — done

- **The Profile screen's destructive pages draw their action word plain.** Decision 5 puts a destructive dialog's action word in `Failed` as well as its title (`ui/action_menu.rs::verb_hints` does; the Settings discard page does since M9.0.6.14's fix round 1). `ui/profile_pages.rs`'s Reject page (`y reject`) and the Confirm page's `y store` (y-only) build their hints with plain `kit::hints_joined`; colour the key and word in `Failed` for Reject, with a render test.
  - *Done in M9.0.7 task 13b (decision 37): the Reject page draws `y reject` with the key and the word in `Failed` (`ui/profile_tests.rs::the_reject_page_is_destructive`); the Confirm page's `y store` keeps the plain grammar (not destructive). Since the final fix wave the five recolour copies share `kit::destructive`.*

## From M9.0.6.16 (2026-10-01), for M9.0.7 — done

- **The status bar shows the tree's hints under the ` MENU ` badge.** With the action menu open over the sidebar tree, smoke stage 11t renders ` MENU  j/k move  ⏎ focus  space fold  / filter  esc back`: the badge is the menu's (`ui/statusbar_modes.rs::badge`), but the hints are the tree mode's, and `⏎ focus`, `space fold` and `/ filter` do nothing while the menu is open (it draws its own `⏎ choose · j/k move · esc close`). 9.0.7's status-bar pass should show the menu's hints, or none, while `Modal::Action` is open, with a render test.
  - *Done in M9.0.7 task 7 (decision 31): under the menu the bar reads ` MENU  esc back` alone (`ui/statusbar_polish_tests.rs::the_menu_shows_only_esc`). M9.0.7's final fix wave (I1) extended the rule to every modal: ` HELP  esc close`, ` DIALOG  esc back`.*

## From M9.0.6's final review (2026-10-01), parked by the controller

For M9.0.7 and M9.2 (*M9.0.7 task 1 ruling: nothing to change in 9.0.7, which adds no run state and no refused kind; both rows stay for M9.2*):

- **The refusal twin waits `FINISH_WAIT` for every kind.** `cli/tests/run_e2e_orch/tui_flow.rs:249` sends each listed refusal with `h.tagged(id, request, FINISH_WAIT)` (900 s), though only a `Finish` can legally take that long; today the only listed refusals on that run are `Accept` and `Discard`, so it is exact. If more states (and so more refused kinds) are added, wait `request_wait(&request)`: `FINISH_WAIT` for a `Finish`, `REQUEST_WAIT` otherwise.
- **An accept can legally outlast `LONG_REPLY_TIMEOUT`.** `app/replies.rs::LONG_REPLY_TIMEOUT` is 660 s (decision 19: the largest `git_timeout_secs`, 600, plus 60 s), but an accept runs three git reads at `git_timeout_secs` before the merge under `ACCEPT_MERGE_TIMEOUT` (600 s, `daemon/src/run/git/accept.rs:21`), so at the largest settings the client can show `no reply from daemon` before the daemon's `Done`. Decision 19 is final; the late `Done` is still shown (decision 16). Recorded, not a fix.

For M9.2 (the next milestone that touches the daemon and the config writer):

- **A dangling `config.toml` symlink is replaced by a regular file.** `config/src/settings/save.rs:34` resolves the path with `std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())`; `canonicalize` fails on a link whose target does not exist, so the save renames a new regular file over the link instead of writing its target. Resolve with `std::fs::read_link` (relative to the link's directory) when `canonicalize` fails, with a test beside `save_tests.rs::a_symlinked_config_stays_a_link`.
- **A stalled `spawn_blocking` save can delay the daemon's exit.** A Settings `Put` writes on `spawn_blocking` and the request gives up after `SETTINGS_WRITE_TIMEOUT` (5 s, `daemon/src/live_config.rs:19`), but the blocking task itself keeps running, and dropping the tokio runtime waits for every blocking task (`cli/src/main.rs:168`'s `#[tokio::main]`), so a write stuck on a hung filesystem holds `anthrex daemon stop` open. Build the runtime by hand at the daemon's entry and end it with `Runtime::shutdown_timeout`, which covers every blocking operation (git probes and spawns too), not only this save.

## From M9.0.6's final fix-wave re-review and the user's 9.0.6 try-out (2026-10-01), for M9.0.7 — done, except the last row (re-filed)

- **`fail_form_brief` matches by run and task, not by request id.** `tui/src/app/action_forms.rs::fail_form_brief` fails the open answer form's brief when any `FormBrief` for that run and task expires, so an older brief's expiry (an earlier opening of the same form) shows `no reply from daemon` while the newer request is still in flight; the newer reply still fills it. Match `f.brief_id == Some(id)`.
  - *Done in M9.0.7 task 13b (decision 37): `fail_form_brief(id, …)` fails only the form whose `brief_id` is `id` (`app_tests/form_brief.rs::an_older_briefs_expiry_leaves_the_newer_loading`).*
- **An answer form opened while disconnected stays `loading…` until a reconnect.** `action_forms.rs::open_form` sends the brief only when `connected()`, leaving `Brief::Loading` with `brief_id: None`; `form_brief_reconnected` covers the reconnect, but nothing says why it is loading. Set `Brief::Failed(NOT_CONNECTED)` there.
  - *Done in M9.0.7 task 13b (decision 37; `app_tests/form_brief.rs::a_form_opened_disconnected_says_not_connected`). The menu's Enter already refuses while disconnected, so the fix is defensive.*
- **A late pre-detection proposal `Show` can briefly draw the old proposal.** After a detection leaves in-progress the screen re-fetches `Show{proposed: true}` and sets that side `Loading`; `profile_awaits` accepts any non-`Ready` side, so a reply to a Show sent before the detection (older than 30 s) lands first and draws the old rows until the re-fetch replies. Accept only the side's latest request id.
  - *Done in M9.0.7 task 13b (decision 37): each side keeps its latest `Show` id and drops any other reply, late or live (`app_tests/profile_screen_views.rs::a_late_pre_detection_show_is_dropped`).*
- **`scripts/pty-smoke.py:106` is 92 columns** inside a block rewrapped to 88.
  - *Done in M9.0.7 task 14: the block is rewrapped at 88 columns.*
- **`C-b g` on an empty session says `select a Git project to start a goal`.** With no window and no run (a fresh daemon), `app/goal.rs::goal_project` has nothing to take a project from, though the TUI was started inside a Git repo; the user had to open a shell window first. Fall back to the TUI's own start directory (`App.default_dir`) when it is a Git repository, for `C-b g` and `C-b P`, with a test.
  - *Done in M9.0.7 task 13b (decision 37): the fallback is `App.default_dir` with no Git check in the client (controller ruling: no I/O in `app/`); the daemon's `not a git repository` refusal is shown in the goal form and the Profile screen (`app_tests/polish/dialogs.rs::an_empty_session_uses_the_start_directory`, end to end in `crates/cli/tests/empty_session_tui.rs`).*
- *The Codex orchestrator's native-question row is re-filed under "From milestone 9.0.7 (2026-10-02), for M9.2 and M9.5" below (M9.0.7 task 1 ruling: out of 9.0.7's scope).*

## From the M9.0.7 brief (2026-10-01), for M9.2 or later

- **An age on every alert.** Spec §6.1 shows an age on each alert, but the snapshot holds no time at which a run entered `awaiting_approval`, a hold became awaiting, a run halted or completed (`RunInfo` carries `created_at` and `approved_at` only). M9.0.7 shows ages only for blocked tasks (their `blocked (` history entry), proposals (`updated_at`) and orchestrator alerts (the window's status age). Fix: a `RunInfo.state_since` (and `HoldInfo.since`) field set by the engine at each state change; a protocol bump.
- **Per-criterion acceptance verdicts.** Spec §6.2 ticks each acceptance criterion `✓ met`, `✗ unmet` or `◌ not judged`, but `ReviewInfo` carries one verdict and findings, and `TaskDetailInfo.acceptance` only the criteria's text. M9.0.7 ticks every criterion `✓` on an approving review and `◌` otherwise, never `✗`. Fix: a `criteria: Vec<bool>` (or verdict per criterion) on `submit_review`, carried into `ReviewInfo`; a protocol bump.

## From milestone 9.0.7 (2026-10-02), for M9.2 and M9.5

- **A Codex orchestrator's native question raises no alert.** *(Done in milestone 9.5, task M9.5.5b.)* In the try-out, the Codex orchestrator (`gpt-6.1-sol`) queued a question through Codex's own UI (`Queued follow-up inputs · ? 1 question · shift+← to answer`) while its window stayed `working`; `app/alerts.rs::orchestrator_text` alerts only on `Attention` (permission) or a quiet window with no signals (start prompt), so the user saw no alert and thought the run had frozen (it had reached `awaiting_approval`). Either turn off Codex's native user-question tool for the orchestrator session (so it asks through anthrex's own channel), or detect the queued-question footer as an orchestrator alert; decide in 9.0.7's task 1 against the Codex CLI in use, with a test against a captured screen.
  - *M9.0.7 task 1 ruling (brief Scope, "Out"): not a client fix. The client holds one screen, the focused window's (`App.parser`; the daemon streams only the subscribed window), so a TUI-side scrape of `? 1 question` would alert exactly when the user can already see the question. Both real fixes are the daemon's, outside 9.0.7's Global Constraint 1: launch the Codex orchestrator session with its user-question tool off, or detect the queued-question footer in the daemon's status machine and raise `Attention`, which `app/alerts.rs::orchestrator_text` already turns into an alert. For M9.2 (the next milestone that touches the daemon's orchestrator launch) or M9.5.*
- **A daemon-side cap on a task's brief (for M9.2).** The daemon checks a brief only for being blank (`daemon/src/run/validate.rs:288`). A brief of about 16 MB can make the next run snapshot, which carries it at the gate, exceed `proto::MAX_FRAME` (16 MiB), so the client stops receiving runs. The TUI's edit form now caps a brief at 1,000,000 characters (`run_edit::BRIEF_MAX_CHARS`, M9.0.7 final fix wave) and never cuts one silently, but a plan file, `run edit` or an orchestrator's `edit_plan` can still send a larger one. Refuse a brief past a bound in `validate.rs` (the client's 1,000,000 characters, or less), with a test that the largest accepted brief's gate snapshot fits one frame. No protocol change.
- **A modal opened while the prefix is pending keeps it pending (for M9.2).** `App::on_key` hands every key to an open modal before the keymap (`app/mod.rs:414-418`), and `keymap::Keymap.pending` is reset only by the keymap's own `handle`. If a daemon reply opens a modal (the force-removal follow-up) between `C-b` and the next key, that key goes to the modal and the prefix stays pending, so the first key after the modal closes is read as a prefix command (`d` would detach). M9.0.7's final fix wave put the modal's badge above ` PREFIX `, so the bar no longer says `PREFIX` under the modal, but the state is unchanged. Clear the pending prefix when a modal opens (or when `on_key` routes a key to one), with an app test.
- **`plan_review::all_deps` keeps a task's own id (for M9.2).** `app/plan_review.rs::all_deps` joins `deps` and `implicit_deps` without dropping the task's own id. The daemon refuses a self-dependency, so no live plan shows it, but the plan review, the compact list and the graph's boxes (all through `run_format::after_text`) would print `after t1` on `t1` from a hand-built snapshot. Filter `t.id` out, with a test.
- **The overview inspects the selection again for its single-line footer (for M9.5).** M9.0.7's final fix wave made the overview frame inspect the selected node once to size and draw its panel, but `ui/overview_footer.rs` (the single line shown when the panel is hidden) calls `inspector::inspect` again (`:105`), and `settle_graph_viewport` inspects on the reducer's side. Two or three inspections a frame are cheap at 10 frames a second; share one if a profile ever shows them.
- **`audit::stray_accent` checks colour only, and not ASCII (for M9.2's TUI work).** The M9.0.7 render audit's stray-accent rule (`ui/audit.rs::stray_accent`, test-only) takes any cell whose foreground is the accent and whose symbol is a Unicode box glyph as part of a frame, so an accented toast or hint that draws `─` would fail it falsely; and it leaves ASCII `+ - |` out entirely (accented hint keys like `C-b` use `-`), so an ASCII frame left accented under a dialog is invisible to it. Tell frames from text by their shape (a closed rectangle), or let the audit read the regions the renderers draw. Also untriggered: a frame whose bottom row a title fills corner to corner is not counted by `accented_frames` (no frame draws one).

## From M9.2.11's review (2026-10-02, fix round 1, m5), for M9.5

- **A merged stage's unfinished fix tasks keep running.** *(Done in milestone 9.7, task M9.7.4; end to end in task M9.7.7.)* `engine/delivery/land.rs::merged` doesn't cancel or flag a merged stage's unfinished fix tasks. A closed stage does both (`closed`).
  - *Below a delivering stage:* a fix task's later commits go up with the next stage, through 9.1's propagate.
  - *On the top stage:* they are never pushed. They raise no "not delivered" line either, because `unpushed` runs only at the moment of the merge.
  - The brief says nothing about this (M9.2.11's notes record it as a reading).
  - Fix direction:
    - Cancel a merged top stage's unfinished fix tasks, with a reason like `stage <n> PR merged`.
    - Or run `unpushed` again when such a task merges after its stage landed.

## From M9.2.17 (2026-10-02), for M9.5

- **An open PR's stage node reads `tier 3 ◌` once a fix moves its head.** *(Done in milestone 9.7, task M9.7.9; smoke stage 11i pins stage 1's `tier 3 ✓` since task M9.7.14.)* In `pr` mode tier 3 runs only before a stage PR opens (decision 19); an update pushed after that runs tier 2 only, and CI is the authority (decision 28). So after a CI or review fix merges into an open stage, the stage node's tier-3 mark shows "no result on this head" for as long as the PR is open, which reads like a tier-3 job that never started. Seen in smoke stage 11i: `◌ stage 1/2  tier 3 ◌  #1  ci ✓` while stage 2 (opened on its final head) reads `tier 3 ✓`. Fix direction: on a `pr`-mode stage with an open PR, draw the tier-3 mark of the head the PR opened with (or `–`), so `ci` carries the current head's verdict; a render test with both stages.

## From M9.2's final fix wave, engine (W1, 2026-10-02), for M9.5

- **`gate.holds` is never trimmed in the digest** *(Done in milestone 9.7, task M9.7.8.)* (task 13's final review, M9 code). The orchestrator digest's `gate.holds` list grows with every hold the run made, decided ones included, and `orch::digest_trim` never cuts it, so a long run with many fix holds could push the digest past `DIGEST_MAX_BYTES` with no step left that shrinks it. Fix direction: drop decided holds (approved, rejected, moot) oldest first before any unfinished task goes, with a size test on a run of a few hundred holds.
- **Widen the CI bisect to the stage's whole line** *(Done in milestone 9.7, task M9.7.10.)* (ruling F1). In `pr` mode a reproduced CI red on the head a PR opened with is never bisected, because decision 36's range starts after the last green tier 3, which is that head; the final fix wave only made the log line say so (`CI fails a test tier 3 passed on this head …`). Fix direction: for a CI red, bisect from the PR's base (or its last CI-green head) instead, so a culprit whose test tier 3 does not run (outside its set, or environment-dependent) can still be named.
- **A double user push merged before anthrex views it reads as not delivered** *(Done in milestone 9.7, tasks M9.7.5 and M9.7.6; end to end in task M9.7.7. Its final fix wave's FW-4 covers the common squash merge whose branch GitHub deleted: the check reads the PR's own `refs/pull/<n>/head` into the same private ref, so it answers "could not check" only when neither ref can be fetched; FW-3 stops another stage's merge commit from skipping the stage's fetch.)* (I-1's reading). `land::merged` judges a merge delivered when the host's merged head is the local stage head or the head an open-PR view confirmed. When the user pushes twice on GitHub and merges before anthrex has viewed the first push, the merged head is neither, and the engine (pure) cannot ask git whether it contains the local head, so the stage reads `… merged at <at7>, without <head7>; that work is not delivered` although it was. Fix direction: let the base fetch after a merge (which already counts the merge commit's parents) also answer whether the local stage head is an ancestor of the merged head, and judge delivery on that.
- **The `ci_summary` decider's intent carries the CI log** *(Done in milestone 9.7, task M9.7.11.)* (A4's reading). A4 keeps the log's text out of the `FailedLogs` done line, but the `Decide` op's kind holds `CiSummaryInput.log` (up to 48 KiB), so the op's journal intent line, and `run.json`'s pending op while it runs, carry it. Fix direction: give the decider request the log's path and let the driver read its tail (decision 18's original form), or journal the intent with the log cut and rebuild it from `CiRecord.text` on a replay.

## From M9.3.4b (2026-10-02), for M9.3.10a or M9.5

- **The action menu's `reject` and `cancel` effect lines describe the whole run during a later round.** *(Fixed by M9.3.10a, found by the M9.5 refresh, 2026-10-03: `engine/actions/effects.rs` gives an open later round's `reject` and `cancel` the round's own effect lines.)* `engine/actions/effects.rs::effect` says `reject: discard run <id>: its worktrees and anthrex/<id>/* branches go` and `cancel: stop … and cancel … unmerged tasks; the run then completes`, but on round 2 or later `run reject` cancels only the round's tasks and returns the run to `complete` (decision 12), and `run cancel` cancels only the round (decision 16). The requests do the right thing; only the menu's one-line preview is wrong. Fix direction: for `run.round() > 1`, `reject: drop round <n>'s <k> tasks; the earlier rounds are unchanged` and `cancel: stop … and cancel round <n>'s <k> unmerged tasks; the run then completes`, with an actions-matrix row each. **Done in M9.3.10a** (controller ruling) *(Confirmed fixed by the M9.5 refresh, 2026-10-03: `engine/actions/effects.rs:84,110`, pinned by `engine/tests/actions_rounds.rs`.)*: the round's lines, with `the earlier rounds are unchanged` in place of `the run then completes` (a `pr` run returns to delivering), pinned by `engine/tests/actions_rounds.rs`; the reject confirm page's details name the round too.

## From M9.3.4b's review (2026-10-02, fix round 1 rulings), for M9.5

- **Two limit texts name the run while counting the round** (m4, accepted). *(Done in milestone 9.5, task M9.5.6.)* `run <id> already has <n> scouts, the most max_scouts allows` (`engine/run_scouts.rs`) and `run window limit (<n>) reached` (`engine/dispatch.rs`) keep their M8/M9 wording, but since milestone 9.3 decision 14 they count only the current round's scouts and windows, so in round 2 or later the run may hold more than `<n>`. Fix direction: `round <r> of run <id> already has …` and `round <r>'s window limit (<n>) reached` when `run.round() > 1`, with the pinned tests updated.
- **A round ≥ 2 that never reaches `complete` writes no `round` line.** *(Done in milestone 9.5, task M9.5.4a.)* `goal_rounds_end::pass` ends a later round only when its run completes; a round whose run is halted and then discarded (or fails) ends without one, so `run stats`' rounds count misses it. Fix direction: end the open round (outcome `cancelled`, or a new `abandoned`) when the run becomes terminal, before the run's own history line.

## From M9.3.6b and ruling D17 (2026-10-03), for M9.5

- **A next goal while a `pr` run's PRs are still open.** D17 makes a `pr` run finished for its chain once it is complete with every PR landed (its chain idle, the next-goal wake saying `delivered`). While any of its PRs is open the run is delivering, never accepted (`engine/delivery/mod.rs` `finish_refusal`: anthrex never merges) and discardable only once complete, so its chain stays active and `continue_from` / `start_goal` are refused with KG §3.5's `still going` text; the user iterates the same run for more work meanwhile, or waits for every merge. Fix direction, if users need it: let a delivering `pr` run hand its orchestrator to a next goal (its chain idle while its PRs stay watched), which needs a wake for later PR events that does not reach the next run's session.

## From M9.3's final fix wave, daemon (W1, 2026-10-03), for M9.5

- **A later round's pre-warm builds its worktrees from the run's original base** *(Obsolete: M9.5 task M9.5.6, ruling T6-2, found that no later round's task is ever pre-warmed. From round 2 the run is `Multi`, a round's tasks sit only in its new stages, those stages are created only once the round runs (`stages::create_pass`), and `stages::ready_in_stage` keeps a task without its stage record `Pending`, which `prewarm` skips. Nothing to fix; see the brief's Implementation notes, Task M9.5.6.)* (review A, M6). During round n ≥ 2's gate, `engine/dispatch_launch.rs:82` (`prewarm`) prepares worktrees, `setup` included, from `base_sha`; dispatch then finds them stale and prepares them again from the stage head (`:126-130`). The result is correct, but every pre-warm in a later round is wasted and runs `setup` twice. Fix direction: pre-warm from `run_head` (or the task's `start_of`) for a round after the first.
- **An adopted session's usage is credited to a delivered previous run** *(Done in milestone 9.5, task M9.5.4b.)* (review B, M2; beside D9's metering gap in the section below). The adopted process keeps its launch environment (`anthrex.run=<launch run>` and that run's OTLP token; `engine/chains.rs` copies `otlp_token`). D9 says that usage is dropped because the launch run has ended, which holds for an accepted or discarded run but not for a D17 delivered one: it is `Complete`, not terminal, so it stays in `is_live` and in the token map, and every token the shared session spends while later runs of the chain work is posted under it (cumulative counters: the data is wrong, not just missing). Fix direction: `Metered::refresh_live` (`driver/usage.rs`) skips a run with `continued_by` set, and the next run is credited once D9 is fixed.
- **`anthrex tree` numbers can differ from the sidebar's** (review B, M7; carried ruling 159). The idle orchestrator's window is unnumbered in the sidebar (`numbered_order` skips it, `tui/src/tree.rs`), but `tree::build` without runs lists it as a plain window with a `position`, so every later position is one higher than its `C-b <n>`. The final fix wave documented `position` as tree order (`tree --json` and its help). Fix direction: once `tree` reads the runs snapshot, number its windows with `numbered_order`.

## From milestone 9.3's final fix wave, TUI and scripts (W2, 2026-10-03), for M9.5

- **Stage 11f's module doc says the goal form has no unconfined-checks toggle** (task M9.3.12's review, m1). `scripts/pty_smoke_orch.py:16-17` says that off macOS the goal starts through `run start --goal` because "the form has no such toggle"; the goal dialog has an `unconfined checks` row (`tui/src/run_goal.rs`, `GoalField::UnconfinedChecks`, sent as `unconfined_checks`). Fix direction: reword the doc; optionally start 11f through the dialog on every platform with the stage's config setting `[orchestrator] unconfined_checks = true` off macOS, as stage 11j does.

## From milestone 9.3 (2026-10-03, recorded by M9.3.12), for M9.5 or later

- **A continued run's orchestrator usage is not metered** *(Done in milestone 9.5, task M9.5.4b.)* (the brief's defect D9, out of its scope). The OTLP ledger keys a session's token usage by the run id in the session's own attributes (`daemon/src/run/driver/usage.rs`), set when the session was launched: the chain's first run. After a next goal adopts the window (decision 23), the session still reports under that first run, which has ended, so the new run's `spend` line and `run stats` miss everything its orchestrator spends; a fresh session (decision 24) is metered as any launch. Fix direction: key the usage of an orchestrator session by its chain and attribute each export to the chain's current run (the one the window serves, `engine/chains.rs`), or re-tag the session's ledger entry when the window is adopted; a test that a continued run's orchestrator tokens land on the new run.
- **The task edit form's brief in the editor** (KG §1.5, §10; the brief's Scope, "Out"). The goal and iterate dialogs use the nano-like editor (`TextArea::on_editor_key`, `ui/kit/editor.rs`); the task edit form's brief keeps its 9.0.7 multi-line field (`tui/src/run_edit.rs`: `TextArea::with_cap`, Ctrl-J a newline, Enter submits the form), without the editor's keys (Enter a newline, Ctrl-S to send, nano's cut and paste of lines, the position row). Fix direction: give the edit form's brief the same editor (`TextArea::on_editor_key`, `ui/kit/editor.rs`) with Ctrl-S sending the form, as the goal dialog does, and keep its 1,000,000-character cap (`run_edit::BRIEF_MAX_CHARS`).
- **An older binary rewriting a multi-round `run.json` drops its rounds** (task M9.3.3's review, m3). `Run.rounds`, `chain` and `request_wake` are `skip_serializing_if` (decision 2) and unknown to a 9.2 build, so a 9.2 daemon that restores and rewrites a run with rounds keeps only round 1's view of it. A downgrade is not a requirement; if one ever is, refuse to restore a run whose `run.json` carries rounds the build does not know, rather than rewrite it.
- **The compact goal dialog does not say it is compact on short terminals** (task M9.3.9b's concern 1, ruled). Below 18 rows the compact fallback covers the status bar, so `widen the terminal for the editor` (`ui/goal_editor.rs::WIDEN`, drawn in the status bar's muted text) shows only on narrow terminals of 18 rows or more. Fix direction: draw the hint inside the compact dialog (its hint row or a muted line) when the status bar is covered, with a render test at 80 × 15.

## From the M9.5 brief refresh (2026-10-03): a recommended milestone 9.6, delivery hardening

*(Became milestone 9.7, delivery hardening, which did every item below; each is marked.)* Not on the roadmap; the user decides. The refreshed M9.5 brief leaves six `pr`-mode delivery items out of its tuning scope (its "Out" table). They share one subsystem (`engine/delivery/`, `run/delivery/`, the host operations), one test rig (`PrRig` on `FakeHost`) and one kind of review, so they are better done together than one by one inside unrelated milestones:

- **A merged stage's unfinished fix tasks keep running** *(Done in milestone 9.7, task M9.7.4.)* ("From M9.2.11's review"). User-visible: on the top stage their work is never pushed and nothing says so.
- **An open PR's stage node reads `tier 3 ◌` once a fix moves its head** *(Done in milestone 9.7, task M9.7.9.)* ("From M9.2.17").
- **`gate.holds` is never trimmed in the digest** *(Done in milestone 9.7, task M9.7.8.)* ("From M9.2's final fix wave, engine"). Partly fixed: approved holds are dropped once read; rejected and moot holds stay, and `digest_trim` never cuts them.
- **Widen the CI bisect to the stage's whole line** *(Done in milestone 9.7, task M9.7.10.)* (same section).
- **A double user push merged before anthrex views it reads as not delivered** *(Done in milestone 9.7, tasks M9.7.5 and M9.7.6; a deleted branch through the PR's ref in its final fix wave, FW-4.)* (same section). User-visible.
- **The `ci_summary` decider's intent carries the CI log** *(Done in milestone 9.7, task M9.7.11.)* (same section).

Suggested order: the two user-visible bugs first (the merged stage's fix tasks; the double push), then the digest's holds, the CI bisect, the decider's intent and the stage node's mark. Each has its fix direction above.

## From M9.5.5b (2026-10-03), for the controller's ruling or later

- *Fixed in M9.5.5b's fix round 1 (ruling T5b-1): while the footer shows, `Attention` holds against every event but client input and exit.* **A Codex title can end a question's `Attention` while the footer stays.** Decision 40 raises `Attention` only on a change from no footer match to a match (`manager/mod.rs::tick`, `Entry.codex_question`), so input that answers it is not undone by the next tick. But a Codex orchestrator launched with no hooks (as the daemon launches it today, M9.5.5a's notes) takes its status from its titles, and `status::next` maps `Title(Working | Thinking)` to `Working` from any status, `Attention` included (`(Runtime::Codex, _, E::Title(CodexTitle::Working | CodexTitle::Thinking)) if !ctx.hooks_seen => Working`). In the try-out's case (a question queued while the window stayed working) one title change after the match would end the attention with the footer still on screen, and nothing raises it again until the footer leaves and comes back. Unverified against real Codex (whether it retitles during a turn with a question queued). Fix direction: keep the match as "raised, not yet answered" until client input (`InputSent`) or the footer's leaving, and re-raise on a tick that finds the footer still unanswered and the status no longer `Attention`; the five M9.5.5b tick assertions all still hold under it.

## From M9.5.17b's fix rounds 2 and 3 (2026-10-04, rulings T17b-3 and T17b-4), for later

- **An old daemon's session that survives a crash can outlive the restore, unstopped, in three cases.** Decision 28 already makes the restore stop a live round's leftover process when its pid is recorded and the process's argv carries the session's id. Three cases slip past it. Each affects every task, not only races, and each predates 17b:
  - **A pid that was never recorded.** This is a launch the restart lost: the early-signal hold keeps the pid out of `run.json` until the `Window` result.
  - **Codex's first process.** Its argv carries no session id, so decision 28 cannot match it.
  - **A descendant that left the process group** (`setsid`). The group's signals miss it. See also M9.5.17b's `setsid` and `ECHILD` notes.

  For race lanes, 17b keeps the lane's checkout and locks in all three cases (`AgentRound.orphaned`), unless a resume of the recorded session succeeds. A relaunch clears nothing (T17b-4). For other tasks, nothing keeps them.

  Fix direction: close these three gaps in decision 28's reconcile. Record a launch's pid before its `Window` result is journaled. Match Codex's first process by something other than argv. Track a session's descendants beyond its group. Do not rebuild the recorded-pid path decision 28 already has.

## From milestone 9.5 (2026-10-04, recorded by M9.5.22)

The brief's "Out" table and its "New follow-ups" list, and what the task reviews left to this file (`.superpowers/sdd/M9.5-tuning/progress.md`). The `pr`-mode delivery items (FU-F29 to F34) stay in the recommended milestone 9.6 above.

- **Stub-then-fill.** Deferred by spec §13's last paragraph until real runs show the need.
- **A merge queue wider than 1.** Same paragraph: a candidate would no longer reflect the stage head. Racing (decision 21) still crowns into a width-1 queue.
- **Coordinating writer caps across runs on one subscription.** Each run halves and recovers on its own rate-limit events (decision 16), so two runs on one Claude subscription can each keep two writers after both were limited. Fix direction: a daemon-wide cap per runtime that each run's cap is read under.
- **Rate-limit events from orchestrator, planner, scout and decider sessions** (ruling RC-1). Only task sessions lower a writer cap; the others are counted nowhere, and readers have no cap. Fix direction: count them per runtime into the same `RuntimeConcurrency`, and give `max_readers` the same halving.
- **The Codex usage limit's reset time** (ruling RC-2, FU-F1). Codex's usage-limit text names a reset time with no time zone, so the runtime is not marked unavailable until then; nobody has set the rule.
- **Phase accounting that excludes daemon downtime** (Risk 1, ruling RH-4). A refit's working minutes come from phase accounting, which counts time the daemon was down while a task was `working`; the per-(run, round) cap limits the damage. Fix direction: a pause-aware phase, as decision 15's pause accounting is for the estimate.
- **`tuning.toml.bad-*` is never pruned.** Each unparsable `tuning.toml` is moved aside (decision 12's start line) and kept forever. Fix direction: keep the newest few and remove the rest when the next one is moved.
- **The Codex output filter by hook.** Task M9.5.19 shipped the instruction path only, because the Codex hook facts of task M9.5.1 (items 4 to 6: whether `codex exec` runs a `PreToolUse` hook that can rewrite a shell command, its tool names and its command pointer) are still for the user to record. With them, add `CodexFilter::Hook`, `output_filter::codex_rewrite` and `anthrex filter-hook --runtime codex` as the brief's decision 28 describes, never with `--dangerously-bypass-hook-trust`.
- **A learned router; racing with more than two lanes or on one runtime; refitting thresholds or routes automatically.** Out by the spec (§15, §4.1), unchanged; listed so they are not mistaken for gaps.
- **A suggested chain length from history** (audit F243 item 24; KG §11; the brief's "Out" table). Needs chain statistics nobody records yet: how many goals a chain took and how each ended. Fix direction: record a chain line when a chain goes idle or ends, then propose a length in `run stats` as the refit proposes thresholds.
- **Delivery statistics** (audit F243 item 25; 9.2's "produces for 9.5"; the brief's "Out" table): statistics from `StageLine`, per-origin fix-task accounting, and `DeliveryLimits` against outcomes. Delivery hardening (the recommended 9.6 above) comes first; then a stats pass over the delivery lines.
- **Role statistics and their refit** (audit F243 item 26; M9's "produces for 9.5"; the brief's "Out" table): planner usage and a planner-route refit in `run stats`, orchestrator cost in `run stats`, message and refresh effectiveness, configurable planner bounds, and the integration-review exemption cap. Milestone 9.5 refits task classes only; these are a separate refit over role records.
- **`pr`-mode revert blindness** (ruling RH-2; the brief's Risk 9). A revert that lands through GitHub as a squash or rebase merge names no task merge commit, so `detect_reverts` never sees it and the refit can step a class down despite it. Bisect culprits and CI or review fix tasks still hold a class. Fix direction: map a GitHub revert to its task through the PR it reverts (the host's PR record names the stage branch and its commits), or record a revert the user reports.
- **A late `McpReady` from a superseded orchestrator session** (task M9.5.5a's fix round 1, m4; the brief's Risk 15). `McpReady` carries no session or launch, so the engine can only ignore a notice while a restart is in flight; a notice from the old session's server that arrives after the restart's op result still counts, and releases the new session's first turn before its own server is up. The paste still needs a hook signal, and the contract's retry line covers a missing tool. Fix direction: put the launch's identity (a generation number) in `anthrex mcp`'s argv and in the notice, and drop a notice from another launch; an argv and wire change.
- **Research (scout) sessions record no routing decision** (task 10b carry, a reading at the 9.5 brief's Task M9.5.10b notes). A scout's route is picked from the scout list but no `RoutingDecision` is written for it, so `run stats` and the routing audit cannot see scout routes. Fix direction: write the decision at the scout launch as for workers.
- **`merge-tree` calls read the repository's own attributes** (ruling T16-11's carry). The signal reads run with `--attr-source=<the empty tree>` where git accepts it (`git/signals.rs`, `NoAttributes`), so no `.gitattributes` changes what they see; the pair read's `clean_merge` (`git/signals_pair.rs`, ruling T16-10) and the engine's existing `merge-tree` calls (`git/merge.rs`, `git/merge_state.rs`, `git/handback.rs:233` and `:500`: the candidate merge, the merges built on it, and the hand-back merges) do not, so a repository's merge attributes apply to them. Not a regression: the existing calls always ran so. Fix direction: decide per call whether the repository's attributes should apply (a merge the user will see should honour them; a read for signals should not), and pass `--attr-source` where they should not.
- **History from before 9.5 lists engine-made fix tasks as plan tasks** (note; whole-branch review B, M1, accepted). Before task M9.5.21, `history::task_record` wrote `origin: Plan` for every task, so bisect, sync, CI and review fix tasks in an existing `history.jsonl` are not left out by ruling RH-1's filter, and RH-2's CI and review evidence cannot match them. No fix: new records carry their real origin, and the refit's window (`[orchestrator.tuning] window`, 200 samples by default) ages the old ones out. If it ever matters sooner, a record whose `task_id` is `fix<digits>` (the engine's reserved ids, `validate_stages.rs`) can be dropped from the samples.
- **The rename variant of a paired task's signals** (ruling T16-11, accepted). When the run head renames a test file the writer edited, the implementer's writer-path read (`git/signals_pair.rs`) follows neither name, so a weakening of the renamed file is not attributed to the writer's path. Not a regression and unlikely; the full proof still catches a loss of the named test, and both candidate fixes falsely bounce an implementer that kept the rename. Fix direction, if one is ever needed: read the run head's renames (`git diff -M --name-status <start>..<run_head>`) and map each writer path through them before both reads.
- **The task temporary directories under `/tmp/ax-501/<hash>`** (tasks M9.5.15 and M9.5.18, review m6). The daemon's short temp root (`task_tmp_dir`) gets one directory per task checkout, lanes included (`…/s9-3`), and nothing removes them when the checkout goes; the checkout and sandbox tests leave empty ones behind, as older tests already did. They are never the user's daemon paths. Fix direction: remove a checkout's temp directory with its checkout (`driver/cleanup.rs`), and have the tests' harness remove its own root.
- **`RUN_REQUEST_TIMEOUT` (180 s) is below a request's six git calls at the default 60 s `git_timeout_secs`** (task M9.5.9's re-review, pre-9.5). Every test harness sets `git_timeout_secs = 5`, so no test sees it; a user on the default can have `run start` or `run stats` time out on the CLI while the daemon still answers. Fix direction: derive the CLI's wait from the daemon's configured git timeout (send it in `Welcome`) or raise it to the call count times the largest allowed timeout. For 9.6.
- **Three `roster::escalate` callers apply neither the installed skip nor the failed-in-this-task skip** *(Done in milestone 9.7, task M9.7.13: the bisect's and the CI fix task through `escalate_for`, the reach forecast through `installed_roster`.)* (task M9.5.10a's fix round 4, pre-9.5): the bisect's fix task (`engine/bisect.rs:552`), the CI fix task (`delivery/fix.rs:312`) and the reach forecast (`reach.rs:78`). Workers and test writers use the skipping ladder (rulings T10a-3/5/6, T16-2). Fix direction: route all three through `route_pick_step`'s `escalate_skipping` over `open_roster`.
- **A stopped racer's child that left the process group can outlive its lane's removal** (task M9.5.17b's fix round 1; pre-existing kill behaviour, no kill change in 9.5). This is the live case, with no crash; the crash cases are filed once, above, "From M9.5.17b's fix rounds 2 and 3". Once the stopped racer's own `ProcessExited` arrives, `race_salvage::kept_because` answers that the racer has finished, so the lane is salvaged with `clear_locks: true` and its checkout is removed (`RemoveWorktree { keep_head: true, clear_locks: true, keep_path: false }`). `ProcessExited` means the leader was reaped and the group was sent `SIGKILL` (M9.5.17b's escalation reading); a descendant that called `setsid` is no longer in that group and can still be running, inside the checkout being removed, after its `index.lock` or `HEAD.lock` was cleared. The locks cleared are only that lane checkout's own (`clear_stale_locks` removes only `index.lock` and `HEAD.lock` in the lane checkout's own git directory, ruling RR-1), and the checkout is being removed, so the risk is confined to that doomed checkout: the survivor can at worst write into a directory that is going away, or fail there. Nothing outside it, and no engine operation, uses that index. Fix direction, if it is ever seen: track a session's descendants beyond its group (the last direction of the crash entry, "From M9.5.17b's fix rounds 2 and 3") and treat a live descendant like an open round, which keeps the checkout and its locks.
- **A hub's test writer stays on the task's runtime with the built-in roster** (found by M9.5.22). A hub task's default route is `frontier`, and the built-in roster has no Codex model at that strength, so `peer_route` finds none and the writer falls back to the task's own route (decision 25, as designed). A user who wants a Codex test writer for a hub names a Codex frontier model in `[[orchestrator.models]]` or a `standard` route on the task. Fix direction, if wanted: let the writer's route step down to the peer's strongest entry, which decision 25 does not say today.
- **Rung 4's ceiling floor uses the class budget, not the task's own `[task.budget]`** (task M9.5.8's re-review 2, out of its scope). Ruling T8-6 keeps `refit::ceiling` from going below the task's class's effective budget on any axis, but `engine/ladder_budget.rs::ceiling` passes the class's budget (`budget_s`, `budget_m` or `budget_hub`) as `own`, never a task's explicit budget from its plan. A task whose `[task.budget]` is above its class's rung-4 ceiling therefore gets a ceiling below its own budget, and rung 4 can raise it to less than it already had. Fix direction: pass `max(class budget, task.budget)` per axis as `own`, with a test of a task budget above the S ceiling.

## From M9.5's final fix wave (2026-10-04, `e0e1bd5e..34313dbd`), for later

What the fix wave's code half (`.superpowers/sdd/M9.5-tuning/fix-wave-W1a-report.md`, `fix-wave-W1b-report.md`) and its re-reviews left to this file. The fix wave fixed the "From milestone 9.5" entry about two `git.ignore` problems printing `(using entry dropped)` (`ec9b68f3`: the drops print `(entry skipped)`), so that entry is removed.

- **A failed task-level prepare leaves its branch and partial checkout until the run's discard or accept** (ruling FW-4's check, W1a round 3). `git::ensure_task_worktree` (`run/git/worktrees.rs`) creates `refs/heads/<branch>` before its later steps; when one of those fails, `driver/ops.rs` answers `Failed` with the branch at `from`, and the checkout's repository may be partial. The task is blocked with `Task.prepare_failed` set, which keeps it from racing. Discard and accept clean the run's branches, but a task cancelled in that state is not cleaned at cancel: `dispatch::remove_cancelled_worktrees` skips a task whose `worktree_live` is false. Fix direction: on a failed prepare, remove what the prepare made (the branch, if it is still at `from`, and the partial checkout), or let the cancelled-task clean-up also take a task with `prepare_failed`.
- **A run started before `ee754027` can carry a task whose prepare already failed with no flag** (W1a round 3's re-review, carry (a); accepted). `Task.prepare_failed` defaults to false, so such a task, after an upgrade, amended to `race = true` and retried, could still race, and its crown would answer `RefMoved` against the branch the failed prepare left. Accepted, because detecting it needs a git probe of the task branch at restore. Fix direction, if it is ever seen: at restore, mark a not-started task whose task branch already exists as `prepare_failed`.
- **A task returning to `Working` takes a writer slot without a cap check** (W1a round 3's re-review, carry (b); pre-existing, general scheduler behaviour). A return to `Working` through a fresh session (rung 2), `run retry`, a relaunch or a review rejection takes the runtime's writer slot without the dispatch's room check (`concurrency::has_room`), so a runtime whose adaptive cap was halved can run more writers than its cap until one ends. FW-4's `pair::implementer_room` therefore counts a handed task with a pending fresh session or `awaiting_deps` as live, to be safe. Fix direction: route every return to `Working` that starts a session through the dispatch's room check, waiting like a dispatch when there is none.
- **`engine/tests/race_fixes.rs` is at 588 lines** (W1b round 3). The next race test goes in a new file.
- **`run/delivery/tests.rs` is at 601 lines** *(Done in milestone 9.7, task M9.7.2: the four persistence tests moved, move-only, to `run/delivery/tests_persist.rs`.)* (the final docs step's measurement at `34313dbd`). It was 600 at `e0e1bd5e`, and FW-2 (b) added a line. Split it move-only before it takes more.
- **The CLI's `run_cmd/delivery.rs:363-374` runs `git.output()` with no timeout.** *(Done in milestone 9.7, task M9.7.12: `bare_under` waits at most `HOST_READ_TIMEOUT`.)* It is a client-side git call, so it is outside AGENTS.md rule 2 (which is about the daemon), but a stalled git would hang the command. Give it the same bounded wait the daemon's git calls use. (Final docs step, 2026-10-04.)

## From M9.6.8's review (2026-10-04, fix round 1, m7), for later

- **The brainstormer's and the document reviewer's `tokens` budgets are not enforced.** `[orchestrator.design.budget]` freezes `tokens` for both, but the scout machine they run on stops a session only past its tool calls or its minutes (`DesignAgentSpec.max_tool_calls`, `timeout_secs`). `DesignAgent.tokens` records what each session spent, after it ends. Fix direction: give the scout machine a token ceiling from the session's usage events, and fail the session past it with its own text, as for calls and minutes.

## From milestone 9.6 (2026-10-05, recorded by M9.6.21)

What the task reviews left for after 9.6 (`.superpowers/sdd/M9.6-design-flow/progress.md`, the lines marked "followup" or "after 9.6") and the brief's recorded risks. The design budgets' `tokens` entry stays above, under "From M9.6.8's review".

- **A manager API to refresh a window's stored prompt** (task M9.6.14, concern 1; review m2; `task-14-rereview-1.md` Minor 1). A Codex orchestrator window's fresh restart re-runs the argv it was launched with, so its first prompt can hold a where-line that is no longer true (the phase, the open gate, the approved documents), and the step that was the session's at launch (`Then follow rule 48.`, ruling T14-3), which a session past the questions must not follow. Today a where-note goes out on every fresh restart to correct it, and when the run is past the questions (answers recorded, no step to take) that note ends `The questions are already answered; do not ask them again.` (`contract_design::ANSWERED`, appended by `restart_note`; final fix wave FW-65, ruling T14-3's carry). Fix direction: let the manager replace a window's stored launch prompt, and have the engine refresh it whenever the where-line or the step changes, so a fresh restart starts from the current line and step and the note is no longer needed.
- **A failed second spec review drops the first review's findings** (task M9.6.10's review, m4, accepted as DF §4.3's literal behaviour). When review 2 fails, the gate says `not reviewed` and shows none of review 1's findings, even those the orchestrator answered `kept:`. Fix direction: keep the latest completed review's findings and disputes on the gate, with a line that the later review failed.
- **Codex `--ephemeral` against the real CLI** (rulings T8-4 and T8-7; for the user's manual check). Every Codex design turn passes `--ephemeral` and is never resumed: a Codex design agent that ends without submitting is relaunched fresh once, and a second end fails it. No test runs the real `codex`, so it is unchecked that `codex exec --ephemeral` saves no session file under `~/.codex/sessions`, and that it accepts the flag on the installed version. Check it in the manual check; if the flag is missing, the runtime is skipped as a brainstormer with `brainstormer on <runtime> skipped: its CLI cannot run without saving the session` and the pair falls back.
- **Keystrokes typed inside a paste's 200 ms gap are submitted with it** (found in task M9.6.19; decision 39's exposure from milestone 9). `driver/wake.rs`'s `ready()` does not re-check the window's input between pasting a wake note and sending its `\r`, so a line the user types in that gap is sent with the note. Fix direction: re-check the quiet time just before the `\r`, and hold the submit while the user is typing.
- **A wake note pasted twice** (ruling T20-1) and **brainstorm draft reads in a rethink** (ruling T20-2) were owned by the 9.6 final fix wave and are done there (FW-1, FW-30; the brief's "Final fix wave W1" and "W2" notes); listed so they are not filed twice.
- **A pure `coverage::check_tasks` for tests** (optional, task M9.6.19). `coverage::check` takes a whole `Run`, so the fixture test re-implements its rules (`missing_heading`, each task's `covers` against the requirements). Fix direction: split the task-level rules into a pure function over tasks and requirements that both call.
- **`run show --findings` on a review draft says `no findings yet` for three different cases** (the W2b re-review, after the final fix wave's W2b). For a spec or plan review draft, the CLI prints `=== no findings yet ===` alike when the review has not answered yet, when it answered with zero findings, and when it failed (`driver/design_io.rs` sends the review's findings with no other state; `cli/src/run_cmd/design_show.rs` prints the line whenever they are empty). Fix direction: carry the review's state with the draft's view (running, answered, failed with its reason), and print `=== findings: none ===` for an answered empty review and the failure's reason for a failed one.
- **The gate screen never names a version's author** (ruling W4-2, DF §6.1). The header is the brief's `<Kind> · run <id4> · v<n> of <m> · <reason>`, and `DocInfo::author` reaches the TUI but is only used to label drafts (`tui/src/app/doc_gate_info.rs` `round_drafts`). Fix direction: show the author on the version list, or after the reason, in milestone 9.0.7's muted role.
- **A `run_status` read can hide a hold approval forever (milestone 9, found at 9.6's merge gate).** *(Done in milestone 9.7, task M9.7.3: `DigestRead { at }`, with `at` taken before the clone less `READ_MARGIN_SECS`, ruling T3-1.)* `driver/orch.rs::run_status` builds the digest from a clone taken at `now`, then sends `OrchEvent::DigestRead`; the engine records `digest_read_at` with its own later `now` (`engine/wake.rs::digest_read`). A hold approved between the clone and the event (in an earlier second than the engine's `now`) was in no answer, and `digest.rs::gate` then hides it from every later read (`decided_at >= read_at`). Under load this failed `run_e2e_orch::promote::e2e_promote_starts_an_orchestrator_and_holds_its_tasks` once in the full suite (its `until /gate/holds/0/state = approved` never matched; alone it passes 3/3 in 10 s). The code predates 9.6 (unchanged since 228656c4). Fix direction: carry the clone's `now` in `DigestRead { at }` and record that, so an approval after the clone stays visible to the next read.
- **An older binary that loads a 9.6 `run.json` drops `design`** (the brief's Risks, recorded, not fixed). serde drops the unknown field, so the run loses its gate and phase, and a write-back loses them for good. Same class as 9.3's "An older binary rewriting a multi-round `run.json` drops its rounds" above; one fix (refusing, or preserving, unknown fields of a newer `run.json`) covers both.

## From milestone 9.7 (2026-10-06, recorded by M9.7.14)

What the task reviews deferred past 9.7 (`.superpowers/sdd/M9.7-delivery-hardening/progress.md`) and the brief's recorded risks, except what its final fix wave owns. Every pre-9.7 item the milestone fixed is marked *(Done in milestone 9.7, …)* where it was filed.

- **Duplicate `scripts` and `settled` helpers in the `pr` end-to-end tests** (task M9.7.2's review, Minor). Task M9.7.2 moved them, move-only, to `cli/tests/support/run_pr_ci.rs` (`pub fn scripts`, `pub fn settled`); private copies predating 9.7 remain in `cli/tests/run_e2e_pr_land.rs:26`/`:32` and `cli/tests/run_e2e_pr_review.rs:26`/`:44`. Fix direction: have both files use the `support::run_pr_ci` ones and delete the copies (a file that imports several support modules by glob, as `run_e2e_pr_ci.rs` does, must not gain a second free `scripts` or `settled`: task M9.7.2's review notes the clash would not compile).
- **`open_roster`'s overlap skip compares current routes, while plan rule 9 compares planned runtimes** (task M9.7.13's review, Important's note and Minor 1; pre-existing, shared with rung 2). `route_pick_step.rs::open_roster` skips a runtime that an overlapping unfinished task is *on* (`t.route.runtime`); rule 9 (`validate_graph.rs::planned_runtime`) refuses a fix whose *planned* runtime matches an overlapping task's planned one. When an overlapping task was moved off its planned runtime (rung 2, `run retry`), the two disagree: `escalate_for` can pick a route rule 9 then refuses (the caller's fallback to the culprit's own route catches one direction; in the mirror case both are refused, the bisect returns `Err` and `ci_culprit` falls to `RouteSpec::default()`). Fix direction: make `open_roster` read the overlapping task's planned runtime, as rule 9 does, with a test for each direction.
- **`sync::live` counts a declared stage with no PR, no tasks left and no skip as live** (task M9.7.4's review, Minor; flagged for 9.7's whole-branch review, recorded here in case that review leaves it). So a merged top stage below such an empty stage keeps its fix tasks running (decision 2's guard) and `unpushed` says the work "goes up with stage m". Consistent with DH §1.1's predicate, but the work may never go up if stage m never opens a PR. Fix direction: treat a declared stage with no unfinished task and no PR as not live in both callers, with a test of each.
- **An older binary that loads a 9.7 `run.json` drops `StageDelivery.undecided` and `undecided_fails`** (the brief's "Risks and gotchas", recorded, not fixed). serde drops the unknown fields, so a merged stage awaiting its `contains` check is re-read as decided and can complete with an unjudged merge. Same class as 9.3's and 9.6's older-binary entries above; one fix (refusing, or preserving, unknown fields of a newer `run.json`) covers all three.
- **Files near 600 lines after 9.7** (task M9.7.14's measurement at `cfac4b38`): `run/delivery/mod.rs` 594, `engine/tests/delivery_land.rs` 592, `host/tests.rs` 591, `driver/host_ops_tests.rs` 576, `engine/bisect.rs` 571 (all under `crates/daemon/src/`). The next change to each goes in a new module or test file, or splits it move-only first.

- **`bisect::merged_away` has lost its direct test** (the W2 re-review). The landing is processed before the bisect check in the same scheduler pass, so `a_bisect_open_when_its_top_stage_merges_adds_no_fix` now ends through `bisect_end::spent_pass`; `merged_away` is reached only when that pass is skipped (for example `Running if held`). Fix direction: a test that holds the scheduler pass so the culprit arrives first.
- **A CI bisect on a merged top stage runs its remaining probes** (the W2 re-review). It never adds a fix or wakes the orchestrator (`ci::gone` at its end), but it wastes probes and holds completion until it ends. Fix direction: check `ci::gone` in `bisect::pass` too.

- **Two tier tests from milestone 9.1 flake under load** (9.7's merge gate, 2026-10-06; code untouched by 9.7). `run::driver::tier::tests::cache::red_and_timed_out_steps_are_never_cached` failed 2 of 15 stress runs at `tier_tests_cache.rs:96` (`assert!(!red.ok)`: the step meant to be red came out green). `run::driver::tier::tests::retry::retry_reruns_the_whole_step_without_single_test_or_with_many_names` failed once in a full suite (index out of bounds, `tier_tests_retry.rs:88`). Both read rig state files that the fake step writes, so the suspicion is a write/read race in the rig. Fix direction: reproduce with the 15-run loop, then make the rig's state reads wait on a deadline or write atomically (rename).

## From milestone 9.7's final fix wave (2026-10-06)

What the final fix wave (`.superpowers/sdd/M9.7-delivery-hardening/final-fix-wave.md`, FW-1 to FW-24) did not change, with the reason.

- **The digest does not say what a stage's `full.commit` means** (review B m6; FW-22, filed rather than written into the contract in a fix wave). The TUI hides a tier-3 red known to be on another commit (rulings T9-1, T9-2), but the orchestrator's digest still serializes `full.state = "red"` beside a different `head`, and no contract rule says that `full.commit` names the head the verdict is about. An orchestrator can read a stale red and plan a needless fix (contract rule 37 wakes it only on a red on the head). Fix direction: one sentence in the orchestrator contract (`orch/contract.rs`) or the digest's own description, that `stages[].full.commit` is the commit the verdict ran on and a verdict on another commit than `head` is stale; a contract change bumps nothing on the wire but needs its contract test updated.
- **A tier-3 red on a closed stage still bisects and can add a fix task to it** (found by FW-1, which asked to mirror the closed case and found nothing to mirror). `full::red`, `bisect::culprit` and `fixes::add_fix` read no PR state, so a red on a stage whose PR was closed without merging bisects, and its culprit's fix task is added to the closed stage (`land::skips` then treats a task added to a skipped stage as reopening it). The merged case is handled by FW-1's `land_judge::spent`. Fix direction: decide whether a closed stage's red means anything (its stages above are paused until a reopen, decision 37); if not, treat a closed stage as `spent` for tier 3 until it is reopened, and re-arm on `land::reopened` as the CI path does (`ci_trigger::rearm`).
- **A cancel during the undecided window logs no verdict** (review A m3; accepted, no change). Decision 39 stops the delivery pass of a cancelled run, so a merge still undecided at `run cancel` never gets its not-delivered or unverified line, and `undecided` stays in the finished `run.json`. Recorded in the brief's final fix wave notes. Fix direction if it ever matters: on cancel, decide each undecided stage as `None` with ruling R1's "could not check" line.
- **The ROADMAP's protocol paragraph does not mention 9.7** (review B, docs cross-check; optional). One line like 9.0.7's: "Milestone 9.7 changes no message and stays at 17".
- **Two `candidate_done` arms move a stage head without the late judgment** (the W1 re-review's O2; ruling FW-O1 sent it here). The "no task awaits this op" arm and the "merged at … after it left the queue" arm (`daemon/src/run/engine/merge.rs`, `candidate_done`) log a landed merge but do not call `land_judge::task_merged`, so if a fix task's merge ever reached either on a stage whose PR merged, it would not be judged. Cancel-during-merge is deferred (ruling T14-I1), so no fix-task route into them is known. Fix direction: call `land_judge::task_merged(run, n, now)` in both arms (or route them through the normal arm's judgment), with an engine test that reaches each.
- **A stage above that stops being live without merging does not cancel a lower merged stage's fixes** (the W1 re-review's O3). `land_judge::cancel_fixes` runs only when a stage merges. If the stage above a merged stage is closed or skipped instead, the merged stage is now `spent`, but its kept fix tasks keep working until they merge (and are then judged not delivered by `task_merged`): wasted work, nothing dropped silently. Fix direction: run `cancel_fixes` for the stages below when a stage closes or is skipped (`land::closed`, `land::skips`), and decide what a reopen restores.

## From v0.1.0 on Ubuntu 26.04 (2026-10-06, branch `fix/headless-startup-stderr`)

`anthrex profile detect` failed within a second with "the scout ended two turns without a report"; the cause, Claude Code 2.1.291 refusing to start without bubblewrap and socat (`sandbox.failIfUnavailable`), was only on the session's stderr. The branch makes the driver emit `SessionEvent::StartupFailed` (the runtime, the exit and the last three stderr lines, with an install hint for the sandbox case) before `ProcessExited` when a process fails before its turn says anything; fails a scout at once on it, or on a turn failed on the missing sandbox; appends it to the run engine's exit reasons (`signals::exit_reason`, for workers, reviewers and research sessions); logs a session's first 20 stderr lines at `WARN`; and stops the driver adding a second sandbox-failed turn when stdout already ended one. A second Ubuntu 24.04+ failure (bubblewrap installed, but AppArmor's `bwrap-userns-restrict` makes every sandboxed command fail with `apply-seccomp: ... nested userns is capability-restricted`) is named, with Claude's sandboxing docs, at the end of a scout's failure reason. Left for later:

- **No sandbox preflight** (the request's goal 2, considered and not done). Checking `bwrap` and `socat` on `PATH` before a sandboxed Claude session would refuse every such session on a Linux host without them, CI's ubuntu runner included, where the e2e tests run fake-agent behind sandboxed specs; and it would still miss the AppArmor failure above, which has both tools installed. The startup failure now says what to install within a second, which is what the preflight was for. Fix direction if wanted: a daemon-start probe that runs one trivial command under Claude's sandbox (the peer session's suggestion) and logs the result at `WARN`, never gating launches.
- **The broken-sandbox note reaches scouts only.** A worker or reviewer whose sandboxed commands all fail with `apply-seccomp` keeps working (and stalls or fails its gates) without the engine naming the cause. Fix direction: map a failed `ToolResult` that `failure::sandbox_command_failure` recognises to an engine signal, and block the task `blocked(environment)` with that text, as `FailureKind::SandboxUnavailable` already does.
- **A decider that dies at startup still reports only its exit** (`decider/call.rs`, `exited(code, signal)`): `StartupFailed` arrives just before `ProcessExited` and could be appended the same way.
- **What real Claude 2.1.291 prints on stdout when its sandbox is missing is not recorded.** "Two turns without a report" fits a failed `result` on stdout plus the driver's own synthesized failed turn (the branch now adds only one); with stderr alone, the old code would have said "the scout's process exited without a report". Fix direction: record the stream on a Linux host without bubblewrap into `crates/daemon/tests/fixtures/headless/` and replay it in `headless_sessions.rs`.
- **Files at the line limit:** `scout/service.rs` is 601 lines and `run/engine/signals.rs` 593 after this branch; the next change to either goes in a new module.

## From the Linux worker-git fix (2026-10-06, branch `fix/linux-worker-git-grants`)

A Claude worker on Ubuntu 26.04 could not commit (reported with Claude Code 2.1.291; the sandbox code read for the fix is 2.1.292's, installed on the test host): its grant named exact files of its checkout's git directory, and Claude Code's bubblewrap sandbox skips a granted path that does not exist, so `index.lock` was never writable. The branch grants a Linux Claude worker the git directory whole (`run::git::GrantShape::WholeDir`), with its configuration denied through placeholders, and keeps macOS unchanged. Left for later:

- **Codex workers on Linux still get the exact files.** *(Resolved 2026-10-07, branch `fix/codex-linux-git`: Codex permission profiles can keep a path read-only inside a writable root, so a Linux Codex worker gets the git directory whole on Codex 0.160 and later; see `docs/superpowers/specs/2026-10-07-codex-linux-worker-git-design.md`.)** The Codex CLI on the Ubuntu 26.04 test host sandboxes with bubblewrap by default (its binary's strings: landlock is the opt-in `features.use_legacy_landlock`), so a granted lock file that does not exist yet is most likely never writable there either, and a Codex worker's commit fails the same way. It was not run here (tests never run a real Codex). The whole-directory fix cannot be applied: `sandbox_workspace_write.writable_roots` takes no denial, so a Codex worker would be able to rewrite the checkout's `config`, which every unsandboxed engine git call reads (filter drivers, include paths). Fix direction: check whether Codex's permission profiles (`--permission-profile`, `[permissions]`) can deny paths inside a writable root; if so, give Codex the same shape; if not, move the lock files out of reach (for instance a worker `GIT_INDEX_FILE` in its private directory), which changes what the engine imports.
- **Names git reads in a git directory that the Linux grant leaves writable.** Besides the denied entries, a Linux worker can now create any other name in its checkout's git directory. Two were closed by the branch's review fixes: `rr-cache/` was not harmless as first written here. Its existence turns rerere on, and git's `mkdir_in_gitdir` follows an `rr-cache` symbolic link, so an engine merge in the checkout would have written rerere's preimage and postimage wherever the link points, outside the sandbox. Every engine git call now passes `-c rerere.enabled=false` (`NO_HOOKS`, `WRITE_FLAGS`); no placeholder is made, since an existing `rr-cache` directory is what turns rerere on. `shallow` (which would cut the engine's history walks in the checkout short; an empty one marks the repository shallow, so no placeholder either) is now refused by `pinned::check` in a task checkout. Reviewed and left: `modules/` (a submodule's git directory, the same exposure as a nested repository in the worktree, which the engine already ignores), `worktrees/`, `sharedindex.*` (the worker already writes the index). Fix direction if any matters: deny it with a placeholder that git reads as nothing, or pass the matching `-c` override on engine calls.
- **A planted `commondir` still redirects refs, not config.** `GIT_COMMON_DIR` makes git read the config from the checkout's own git directory, but git's files ref backend reads `commondir` itself, so a daemon git call that started between `pinned::check`'s refusal and git's own read would resolve refs (and write a per-worktree ref) through the planted directory. The engine names commit ids in a checkout, never a branch, so no decision reads one; the window is a race. Fix direction: have the check open the git directory and pass the call a descriptor-relative path, or re-check after the call and discard its result.
- **A `shallow` file is refused by a check, not denied** (re-review m2). No placeholder can deny it (an empty `shallow` marks the repository shallow), so a Linux worker can create one in its checkout's git directory; `pinned::check` refuses the checkout before each daemon git call there, but a call that started between the check and git's own read would see a shallow repository, as with `commondir` above. Its effect is a history walk in the checkout cut short (the import's connectivity is still enforced by `index-pack --strict` in the user's repository). Fix direction: as for `commondir`, or `-c` override if git ever gains one for the shallow file.
- **The sweep unlinks a link at a denied path** (re-review N1, noticed while fixing it). Like a rename, an unlink of a directory entry on the host detaches any mount on it in a sandbox still running. The sweep runs before each grant and removes only symbolic links; a sandboxed worker cannot make one at a denied path (it is a mount point there), so only a link made outside any Linux session (or one sandbox-runtime itself bound `/dev/null` over) is affected. Fix direction: refuse the grant instead of sweeping when a denied path is a link.

## From M9.8.6 (2026-10-07, model discovery), for later

- **`codex_version::probe` duplicates `models::lines::LineReader`'s read loop** (preflight F30). The startup Codex version probe (`daemon/src/lifecycle/codex_version.rs`) sets its pipe `O_NONBLOCK` and polls it every 5 ms under a deadline, exactly as `daemon/src/models/lines.rs` now does for model discovery; M9.8.6 was limited to moving `ProbeChild` out of that file, so the loop was copied, not shared. Fix direction: have `codex_version::probe` read through `models::lines::LineReader` (or `models::version::cli_version`, which also parses with `launch::codex::parse_version`), keeping its own `report` and launch-gate handling.

## From M9.8.8 (2026-10-07, escalation along the role table), for M9.8.13

- **A fix task naming a row model outside `run.roster` is refused.** *(Done in M9.8.13: `Run.roster` is gone and `validate::resolve_route` takes a named model as given, no roster membership or strength checked; tests `fix_routes::a_bisect_fix_task_off_the_roster_takes_its_step_up`, `model_roles_tests_user::a_users_route_to_any_catalog_model_is_accepted`.)* `validate::resolve_route` (`daemon/src/run/validate.rs`, the `roster::find` arm) refuses a route that names a model the frozen roster does not list (`<model> is not in the roster for <runtime>`). Since M9.8.8 an engine fix task (`engine/bisect.rs`, `engine/delivery/fix.rs`) names the route `role_step::escalate` gives along the culprit's row, which can be any catalog model the user chose (say `codex:gpt-6-sol`); such a fix task is refused and falls back to the culprit's route, or for a CI fix to the row's route. Fix direction: when `Run.roster` goes (M9.8.13), drop the roster-membership check for a named model (the strength check with it), as 7b did for `--orchestrator`.

## From M9.8.9 (2026-10-08, the orchestrator sizes; it never routes), for M9.8.13 or later

- **`get_context`'s tool description still says "the models you can route to".** *(Done in M9.8.13: "the role table (which model each size runs on)" in `tools_orch.rs`, `orch_tools.json` and `tools_orch_tests.rs::GET_CONTEXT`.)* `mcp/src/tools_orch.rs` (the `get_context` tool) and its fixture `mcp/src/fixtures/orch_tools.json` keep milestone 9's text, while contract rule 4 now says "the role table (which model each size runs on)" and `get_context` answers `roles` (decision 34). The brief's file list names only `route()`. Fix direction: reword the description to match rule 4, in the code, the fixture and `tools_orch_tests.rs::GET_CONTEXT`.
- **`route_pick::context_routes` has no caller left.** *(Done in M9.8.13: deleted with `route_pick.rs`.)* `get_context` no longer reads `limits.route_lists`; the function stays with the rest of `route_pick.rs` until M9.8.13 deletes it.

## From M9.8.12 (2026-10-08, saving the role table), for later

- **A save writes the daemon's live table, not the file's (review M10).** A `[models.*]` row hand-edited in `config.toml` while the daemon runs is reverted by the next C-b S save, which sends the live table loaded at start (the limits always behaved so, decision 29). Fix: refuse when the file's owned values differ from the screen's `base` ("config.toml changed since the daemon read it; reopen C-b S").

## From M9.8.13 (2026-10-08, the roster pickers, the model lists and the route refit removed), for later

- **Done (M9.8.13 fix round 1): `cli/tests/models_migration.rs::daemon_start_logs_each_migrated_key` fails since M9.8.12's fix round 2 (N1).** Fixed: the fixture lists codex `gpt-6-sol` with its roster entry; the kept review list's note now says `none of its models can review <runtime>'s work` (review M4). Its fixture's `[orchestrator.routes.review]` names only `claude-opus-5-5`, on the default medium route's own runtime, so the reviewer never came from it: the list is kept and the daemon logs `config: [orchestrator.routes.review]: none of its models are known; [models.reviewer] uses codex:default until you choose one in C-b S` instead of the `… is replaced by [models.reviewer] …` note the test waits for (61 s timeout). `config/src/models/` is unchanged by M9.8.13; the test was not in M9.8.12's verification list. Fix: name a Codex roster model in the fixture's list (as `write_models_tests::a_list_with_one_known_model_is_migrated_and_removed` does), or expect the kept-list note.

## From M9.8 (collected at M9.8.15, 2026-10-08), for later

The gaps tasks M9.8.6 to M9.8.14 left on purpose, each detailed under its task in `docs/milestones/M9.8-model-roles.md`'s Implementation notes.

- **The manual check is outstanding** (the brief's "Manual check"): one real discovery per CLI on macOS and nexus1, with the raw replies compared to `crates/fake-agent/fixtures/`. Any difference becomes a fixture-and-parser follow-up.
- **`reach` counts the reviewer row's runtime unconditionally** (M9.8.7a): a Claude-only user on the built-in `codex:default` reviewer has Codex in reach. Fix direction: count it only when a task is reviewed by it.
- **The once-per-run same-model reviewer line** dedups by scanning a trimmed `run.log`, so it can repeat once trimmed away (M9.8.7a). Fix direction: a flag in `run.json`.
- **`build_models::fill` rewrites a finished run's `run.json` once at restore** (M9.8.7a).
- **Rule 9's refusal text can say "size research"** for a research task's row (M9.8.7a).
- **The orchestrator's PTY window does not name its role on a missing CLI** (M9.8.7b, M2): its spawn goes through `portable_pty`, whose missing-program error is not an `io::Error`. The start's runtime check refuses first, so this needs a CLI removed between the check and the spawn.
- **Discovery snapshots** (M9.8.6 fix round 1): `CliProbes.claude_auth` is read at daemon start, and a Codex upgraded while the daemon runs is served the old version's cache until `r` or a restart. Check for leftover descendants of a probed CLI during the manual check.
- **The goal form's `role table (…)` names the global orchestrator row**, not a repository override (M9.8.11 minor 6): the TUI holds no project table outside the open Settings screen. The task edit form's picker title stays `this task` (minor 7).
- **Files over 600 lines**: `config/src/settings/write_models_tests.rs` (701), `daemon/src/run/model_roles_tests.rs` (624), `config/src/models/migrate.rs` (612), `tui/src/app_tests/goal_editor.rs` (602); `tui/src/app/mod.rs` is at 598, so the next `App` field needs a split first (M9.8.11 minor 10).

## From milestone 9.8's final review (2026-10-09), deferred

- **M6. Files over roughly 600 lines.** `crates/config/src/settings/write_models_tests.rs` (701), `crates/daemon/src/headless/argv_tests.rs` (663), `crates/daemon/src/run/model_roles_tests.rs` (about 650 after the final fix wave), `crates/tui/src/app/action_forms.rs` (619, was 590), `crates/config/src/models/migrate.rs` (612), `crates/cli/tests/scout_service.rs` (602, was 592), `crates/tui/src/app_tests/goal_editor.rs` (602). Most are recorded above; `action_forms.rs` and `scout_service.rs` were not. Split each by responsibility.
- **M7. Discovery snapshots.** `CliProbes.claude_auth` is read once, in `RunService::new` (`crates/daemon/src/run/driver.rs:203-208`). A Codex upgraded while the daemon runs keeps its old cached catalog until `r` or a restart. Already recorded above; re-read the version and auth per refresh.
- **M8. `r` can return a non-forced result.** Pressed while a `refresh: false` refresh is in flight, `r` waits on the gate and returns that refresh's result (T6 deferred). Pressing `r` again works. Fix: the ticket check in the models service should skip the shortcut when `refresh` is true.
- **The I1 settings-save refusal says "restart the daemon or reopen C-b S", but reopening does not clear it.** Settings `Get` does not reload `config.toml`, so only a daemon restart clears the refusal. Either reload the file when `C-b S` opens, or drop "reopen C-b S" from the refusal text and the README. Found by the 9.8 final fix wave's re-review.
- **Migrated scout keys that land on Haiku at `low` still log "effort 'low' not offered by haiku" on every run.** The 9.8 gate fix made only the built-in research row effortless. Migration should drop an effort that the migrated model doesn't report.
- **Three identity minors from the 9.8 alias fix's review** (`.superpowers/sdd/m98-discovery-fix-review.md`):
  - `canonical_of` falls back to the literal id for refs the run never looked up, so `claude:opus` and `opus[1m]` compare as different models.
  - Picking `default` freezes the account's model when the run starts. The brief doesn't say this.
  - The picker's "as <label>" disagrees with `label()`'s rule for matches past the context tag.
- **The TUI and the daemon briefly disagree on model identity until the catalogs load** (9.8 gate-fix review, minor).

## From milestone 9.9 (2026-10-09, recorded by M9.9.11), for later

What the task reviews of milestone 9.9 parked, each detailed in `docs/milestones/M9.9-orchestrator-first-alerts.md`'s Implementation notes or the task reviews.

- **The manual check is outstanding** (the brief's "Manual check"): a real orchestrator retrying a real environment block, answering an `ask_user` with `2` from the Alerts view, and an orchestrator closed with `/exit` handing its alerts back. It needs the real `claude` and a person.
- **Orchestrator `resume_run` and the user's `run resume` are parallel call sites** over the same primitives, not one shared function (M9.9.3 minor). A later change to one can miss the other; the double-hold test pins today's agreement.
- **`accept_red` is allowed while a bisect or fix task on that head is in flight** (M9.9.3 M2, kept under autonomy "Everything"). The in-flight fix then finishes against an accepted red.
- **A digest read marks every wake note answered, even one newer than the read** (M9.9.6 M3, as OFA's text says). A stall can stay hidden until the next note.
- **`over_cap` notes once per thread**: N review threads over the cap give the orchestrator N notes (M9.9.4 minor, unpinned). `wake_routed.rs` also duplicates `full_fixes.rs`' three-failure setup.
- **`handled.rs` repeats `LOG_MAX` (500)** from `requests.rs`, and `orch_ops::intercept` matches `edits[0]` twice (M9.9.2 minors).
- **`user_only_task` trusts `BlockInfo.user_only`** as set where the block is raised; a block raised without the marker check is the orchestrator's (M9.9.5 minor).
- **Give `submit_epic` its own op list** (M9.9.10 minor): `plan_edit` is shared with `edit_plan`, so the sub-planner's schema advertises the five ops the daemon always refuses from a sub-planner (as it already did for `answer` and `pause`).
- **Files near the limit**: `cli/src/run_cmd/status_tests.rs` (598) and `daemon/src/run/engine/planners.rs` (589) after M9.9.1; the next test or arm goes in a new file (R11).
- **The module-graph temp file can collide** (seen in M9.9.11's full run, pre-existing): `daemon/src/profile/verify_tiers.rs:58` names it `anthrex-graph-<pid>-<nanos>.json` and creates it exclusively, so two threads of one process in the same nanosecond collide (`File exists`). The tier then falls back to a full run (`module graph unknown`). This flaked `tier::tests::cache::a_cached_build_runs_again_when_a_later_step_misses` once. Fix direction: add a process-wide atomic counter to the name, or retry with a new name on `AlreadyExists`.
- **The orchestrator can cancel and replace a user-only-blocked task** (final re-review, pre-existing power): `cancel_task t1` plus `add t1b` after the rewrite cap's user-only block is accepted. t1 is cancelled, which removes the user's alert, and t1b starts with `rewrite_restarts` 0. This is visible only in the plan-edit log and the handled rows. Fix direction: refuse the orchestrator's `cancel_task` on a task whose block is `user_only`, or carry the count to a replacement that names it.
- **A design phase's budget halt is recognised by its reason text** (final review I-1 fix): `design::budget_halted` matches the shared head and tail constants that `design::tick` builds the text from. A persisted tag on the halt would be sturdier than the text.
- **More daemon lib flakes under full-suite load on macOS** (milestone 9.9's gate, in code this branch does not touch; all passed on rerun): `graph::tests::a_graph_command_that_times_out_is_unknown`, `graph::tests_keys::a_manifest_that_matches_nothing_keys_on_the_tree`, `graph::tests_keys::a_manifest_in_a_symlinked_directory_keys_on_the_tree`, `tier::tests::cache::a_second_job_on_the_same_tree_runs_nothing`, `tier::tests::cache::red_and_timed_out_steps_are_never_cached`. They may share the temp-file collision above.

## From milestone 9.10 (2026-10-10, recorded by M9.10.12), for later

What milestone 9.10 parked, each detailed in `docs/milestones/M9.10-simple-profile.md`'s Implementation notes.

- **The manual check is outstanding** (the brief's "Manual check"): a first goal in a scratch clone with the real `claude`, the set-up alert's three texts, a failing row edit on the card (`checking…`, `✗`, `o`, `r`), **Use this** starting the goal, and the out-of-date card after a manifest change. It needs the real `claude` and a person.
- **A queued goal has no row in the project tree** (decision 34, the controller's ruling; spec defect 16): the Alerts box's set-up alert is its only view in the TUI. A tree row is a new node type across about ten files.
- **Stage 11l does not exercise a row edit** (M9.10.12): the smoke stage reaches Use this with the scout's proposal as it is. A card `e` on `check`, its `checking…` cell and its ✓ would cover decision 15 through the real client; `profile_cli_tui.rs::e2e_tui_row_edit_on_a_proposal_then_use_this` covers it through the screen's requests.
- **A failed edit that adds an environment variable has no row** (final review fix wave, client): the Profile screen lists `env.<NAME>` rows from the profile, so a ✗ edit of a new `env.<NAME>` shows only in the notice under the list (`your edit of <NAME> (<value>) failed its check; …`); `s` and `r`, which act on the selected ✗ row, cannot reach it. `anthrex profile edit env.<NAME> … --anyway` or `anthrex profile reject` still can. Listing the edited key as a row, or letting `s`/`r` act on the one row edit whatever is selected, would close it.
- **`anthrex profile status` says `Ready · verified <age> ago` beside a check saved failing** (final re-review N3): the Profile screen reads the stored side's verification and says `Ready · <label> saved failing its check · verified <age> ago` in the attention tone, but `ProfileStatus` carries no verification, so the CLI's line (`profile_words::status_line_tone`) cannot. A `ProfileStatus` field naming the stored profile's failing checks (a protocol change, every client updated) would let both say it.
- **A dropped goal raises no alert** (final review D-I2, the "better" fix): the Profile screen and `profile status` list the last dropped goals, and the screen polls for 30 s after **Use this**, but a user who closed the screen learns of a drop only there. The snapshot carrying recent drops for a while, one `could not start "<goal>": <reason>` alert each, needs a protocol field (a daemon change).

## From milestone 9.11 (2026-10-10, recorded by M9.11.39), for later

What milestone 9.11 (stabilization, released as 0.2.0) left open: what its brief deferred, what each stream's review parked, and the entries above that it closed. Each item is detailed in `docs/milestones/M9.11-stabilization.md` (its "Scope", "Checked and dropped", decision 21 and each "Stream <X> notes" section). The streams' own entries, written while they ran, are kept below under their stream's heading.

### Deferred by the brief

- **A11: a failed second spec review drops the first review's findings.** Dropped by the user's ruling (2026-10-10), as the spec says: DF §4.3 gives the gate `not reviewed: <reason>` and opens it without findings, and the code does exactly that (`run/engine/design_reviewer.rs:301-304`). Changing it means changing the spec first.
- **B10: every hook and every 250 ms reader pass clones the whole conversation under the manager lock** (line 528-538 of this file). About 1.5 ms per hook at the configured maximum, far under the 100 ms lock-latency asserts; a performance refactor, not a hang (category F).
- **B6's two remaining cases and D4** (decision 21, the user's ruling). After a crash the restore cannot stop (a) **Codex's first process**, whose argv carries no session id, so M8a decision 28's argv check cannot confirm it is the session's, or (b) **a descendant that left its session's process group with `setsid`**; while the daemon runs, (b) also keeps its sandbox grant into later turns (D4), a stopped racer's child outliving its lane's removal included. Why deferred: finding either needs the daemon to look for processes it did not record itself (an environment marker or a process-table scan), and the user ruled that the daemon keeps its stance of never inspecting processes it did not start. The README's "What a crash can leave running" says so. Decisions 19-20 closed only the case whose pid the daemon's own launch recorded.
- **E3: `RUN_REQUEST_TIMEOUT` (180 s, `cli/src/run_cmd.rs:29`) is shorter than six git calls at the default 60 s timeout** (line 1914), so the CLI can time out while the daemon still answers. Not one line: the right bound is six times the configured `git_timeout_secs` (up to 600 s each, `config/src/orchestrator.rs:277-283`), which the CLI does not read today, and a constant big enough is an hour. Note that the client restates the 180 s in `tui/src/app/alerts_dropped.rs` (`RUN_REQUEST_SECS`); `cli/src/run_cmd/adapt.rs`'s `the_clients_restated_run_request_timeout_is_the_clis` fails if the two drift.
- **E5-E24 (except E1, E2, E4, done) were not taken**: none is a one-line fix (protocol, engine semantics or several files), and E15 and E19 also need a ruling. Each stays open where this file already records it:
  - E5, restart id reuse: a stale `Output` or `Title` event, or an `anthrex hook` payload, can relabel a freshly restarted window; it needs a generation counter (lines 307-334; protocol).
  - E6, a conversation `from_rev` is meaningless across a daemon restart; latent while the TUI always sends `None` (lines 644-656).
  - E7, a failed Claude `headless_send` leaves its turn recorded, so the next result is classed `Unprompted` (lines 1180-1186).
  - E8, a task returning to `Working` takes a writer slot without a cap check, so `max_writers` can be exceeded (lines 1211-1218, 1926).
  - E9, a failed task prepare leaves its branch and a partial checkout until discard, and a task cancelled in that state is never cleaned (line 1924).
  - E10, two `candidate_done` arms move a stage head without the late landing judgment, and a stage above that closes does not cancel a lower stage's fixes (lines 1973-1974).
  - E11, a tier-3 red on a closed stage still bisects and adds a fix task; a CI bisect on a merged top stage runs its remaining probes (lines 1970, 1961).
  - E12, `sync::live` treats an empty declared stage as live, so a merged stage's fixes keep running (line 1956).
  - E13, `open_roster` and plan rule 9 disagree after a runtime move, so a bisect can return an error and fall back to the default route (line 1955).
  - E14, rung 4's ceiling floor uses the class budget, not the task's own budget (line 1918).
  - E15, `accept_red` is allowed while a fix on that head is in flight (line 2050); needs a ruling.
  - E16, a design budget halt is recognised by matching its reason text (line 2059).
  - E17, phase times include daemon downtime and pauses, which skews refits and estimates (lines 1485-1493, 1900).
  - E18, a Codex usage limit is retried every `rate_limit_retry_secs` until its stated reset, which can be dozens of doomed turns (lines 744-760, 1899).
  - E19, a settings refusal says "reopen C-b S" but reopening does not clear it; migrated Haiku `low` logs an effort warning on every run (lines 2036-2037); needs a ruling.
  - E20, `WindowEvent::Exited` is not ordered after the window's final output (lines 1584-1593).
  - E21, the main pane can switch to a new window while the new-agent form is still swallowing keys (lines 1562-1574).
  - E22, a late `McpReady` from a superseded orchestrator session releases the new session's first turn too early (line 1908; argv and wire).
  - E23, disk leaks: empty `/tmp/ax-<uid>/<hash>` task temp directories, stray tier step directories, `tuning.toml.bad-*` and an ever-growing `history.jsonl` (lines 1519-1524, 1737, 1901, 1913, 1530).
  - E24, `wrap` measures characters while dialog boxes are sized in display columns, so CJK paths draw misaligned boxes (lines 188-191).
- **The pre-bind allowance is an allowance, not a bound** (decision 31). The client's daemon start waits `daemon::LOCK_WAIT` (5 s) plus `tui::spawn::PRE_BIND_ALLOWANCE` (5 s); nothing bounds what the daemon does before it binds (loading its state and config, restoring its runs, whose reconcile makes git calls). A pre-bind phase longer than the allowance makes the client report `the daemon did not start within 10 s` although the daemon comes up later, and once the spawn marker (`<data>/daemon.spawning`, fresh for `PRE_BIND_ALLOWANCE`) has aged, a second waiting client spawns a second daemon, which yields to the first. A bound on the pre-bind work, or a daemon that binds before it restores, would close it.
- **Category F stays out** (the user's ruling R1): TUI features and polish, engine features, refactors (the files at or over 600 lines included: `run/git/mod.rs` at exactly 600, `run/engine/signals.rs` 598, `daemon/tests/git_registry.rs` 608) and the accepted `u32::MAX` id.
- **The manual checks are outstanding**: milestone 9.11's own ("Manual check": `less` and `vim` after a re-attach, typing during a wake-up, `daemon stop` leaving no process) and those of 9.8, 9.9 and 9.10 above. They need the real `claude` and `codex` and a person, before the `v0.2.0` tag.

### Closed by milestone 9.11

Entries above that this milestone fixed; each stays where it is, for its history.

- Milestone 9.9's "a digest read marks every wake note answered" (A8, decision 6), "the module-graph temp file can collide" (C1/E1, decision 37; the 9.9 gate's graph and tier flakes listed beside it are likely the same cause) and "the orchestrator can cancel and replace a user-only-blocked task" (A5, decision 5; `split_task` is refused too).
- Milestone 9.10's "a dropped goal raises no alert" (A7, decision 22): the alert is raised by the client, with no protocol change, for the client session that saw the goal leave; a client started later sees the drop only on the Profile screen and in `profile status`, as before.

### Stream D

- **`restore.rs`'s `restore_runs` keeps the stepped state when `prepare` panics twice** (M9.11.8): decision 8 restores the pre-step state in the event loop only. Close to unreachable; `restore.rs` is in no stream.
- **Hiding old runs can make an ambiguous run-id prefix resolve** (D10-M2): once runs past the 50 newest finished leave the list, a prefix that used to match two runs can match the one still listed. Low risk with anthrex's run ids.
- **`fit_frame` encodes every snapshot one more time** (D10-M3), CPU only; cache the size if it ever shows in a profile.
- **The live-only snapshot rule's `error` log line is not asserted** (D10-M4): the lib tests capture no logs.

### Stream E
What stream E's review left open, each detailed under "Stream E notes" in `docs/milestones/M9.11-stabilization.md`.

- **A message in flight to a session a rung kills is dropped** (review m5; needs a ruling): `ladder::kill_worker`'s `supersede` removes a task's delivered-but-unconfirmed outbox messages when rung 2, 3, 4 or a lane out stops its session, so a `Deliver` whose `Delivered { ok: false }` comes after the kill finds nothing to queue again, and its text never reaches the next session. Decision 4 carries in-flight texts only for a failed resume (`outbox::resumed`). The window is the `Deliver`'s round trip (normally milliseconds). Carrying them into the rung's fresh session (or keeping them, for a blocked task) would close it.
- **A long `SANDBOX_UNAVAILABLE` reason plus a sandbox note can lose the note's `USERNS_HINT`** when the reason is cut to its length limit (review m2's remainder, left as it is). Cut the reason before appending the note.

### Stream G

From the stream G review and its fix round 1.

- **A3's last window (review m16).** `run/reconcile/git.rs::remove_worktree` (gone-checkout branch, `:457-468`) removes a deleted checkout's repository without `salvage_gone` when the daemon died inside that `RemoveWorktree`. Safe fix: when the repository's `HEAD` file holds a commit id and the op's `salvage_ref` does not exist, return `Reconciled::NotStarted` instead of `remove_dir_all(repo)`; the re-emitted op is idempotent (`salvage_gone` replays or creates the ref, then removes). Needs both the user's hand deletion and a crash inside that op.
- **Identity resolved in the common git directory (m1).** `checkout::identity` runs `git config --get` in `<common>`, not in `root` as decision 15 says; they differ only for a linked-worktree root (`onbranch:` conditions, patterns matching `worktrees/<id>`). Passing `root` into `ensure` needs `run/git/worktrees.rs`.
- **The run's data directory must stay readable to every task session (m2).** git treats an unreadable include target as fatal, and each task config includes `<repo>/engine/identity`; a future `deny_read` covering it would break every git command in the checkout. Say so beside the `deny_read` builders (`headless/argv.rs`).
- **The window probe has no pin re-check after its call (m3).** `git/probe.rs:45-80` checks before only; display-only output, so harmless; add the after-check (return `None` on refusal) for parity with decision 16.
- **A planted-and-removed `commondir`/`shallow` during a call (m4).** `GIT_COMMON_DIR` and (fix round 1) an empty `GIT_SHALLOW_FILE` neutralise both for a standalone checkout; a linked checkout's call still relies on the before/after checks.
- **The grant's TOCTOU to the sandbox's bind (m7).** Between the grant's lstat refusal/placeholders and the sandbox's bind-mount at launch, only a leftover process holding an earlier grant of the same git directory can act; it cannot replace a denied entry (a mount point: `EBUSY`) and a `commondir` it creates is refused by `pinned::check`. Denial of service at most.
- **Ignored nested repositories are not candidates (m12).** `ls-files --others --exclude-standard` drops ignored untracked directories, so a nested repository under an ignored path (`vendor/`, a `.gitignore`d clone) goes with the checkout, as today's salvage keeps no ignored file. Listing `--others --ignored --exclude-standard --directory` entries that hold a `.git` would keep them.
- **The "kept" text's route (m13).** Decision 14 says "the run log says"; the text reaches the user as the op's error in the task history (`could not remove its worktree: kept …`). A run-log line would match the decision's wording.
- **The Linux watcher after an inotify overflow (m14b).** Directories created during a queue overflow (`need_rescan`) are never watched; re-plan from the root on a rescan event.
- **A rename's source forwarded before its target is watched (m15).** For `Modify(Name(Both))` inside the root, the vanished source path sets `accepted` and the event is forwarded before the adder watches the target (the adder forwards again afterwards; no state is lost). Skip forwarding when the only accepted non-directory path is a rename's source whose target went to the adder.
- **Reflog-only commits in a nested repository (m11, part).** The probe counts `HEAD`, every ref, and (fix round 1) each linked worktree's `HEAD`; a commit reachable only from a reflog is not counted.
- **Remote-tracking tips count as saved (fix round 1, I3).** In a nested repository or submodule, a commit equal to a `refs/remotes/*` tip, or to the gitlink the checkout records, is taken as kept elsewhere; a worker that forges a remote-tracking ref in a repository it made can only hide its own work.
- **`restore_owner_access` on a system whose `fchmodat` cannot honour `AT_SYMLINK_NOFOLLOW` (fix round 1, I5).** glibc before 2.32 (or a newer one without `/proc` on a kernel before 6.6) refuses to open up a directory locked with no owner access; the removal fails and the checkout is kept. A descriptor-only change (`O_PATH` plus `/proc/self/fd`) was not taken: the milestone's verification forbids `/proc/` in new daemon code, and where `fchmodat2`'s `AT_EMPTY_PATH` exists glibc already honours the flag.
- **The production Linux watcher has not run outside CI (I6).** nexus1 was unreachable from the implementer's machine on 2026-10-10 (`ssh: connect to host 192.168.0.199 port 22: Operation timed out`, and no reply to ping); the PR's ubuntu job is its first run. Stress `git::watch` and `git_registry*` there before a release that depends on it.

- **An empty nested repository blocks its checkout's removal** (stream G followup 3): today's salvage fails on a nested repository with no commit (`git add -A`: `'<dir>/' does not have a commit checked out`), so such a checkout can be neither salvaged nor removed without the user's help.
- **One `salvage_gone` with a `keep_head` flag (fix round 2, n7).** `salvage_gone_keeping` repeats `salvage_gone`'s `Pin` setup and runs `common_dir` and `head_file` a second time. Folding it into `salvage_gone` as a parameter would remove the duplication.
- **A nested `.git` that git does not take for a repository is not a candidate (fix round 2).** One example is a `HEAD` whose `ref:` lies outside `refs/`.
  - `ls-files --others` then recurses into the directory and skips `.git`, so only its working files are salvaged. Any commits in its object store go with the checkout.
  - Git itself cannot open such a repository either.
  - A walk for `.git` entries that keeps the checkout when any of them is unrecognised would close the gap.
- **Stream G's final re-review minors** (fix round 2, all non-blocking):
  - **An intermediate directory of a ref path can be swapped.** `O_NOFOLLOW` covers only the last component, so a worker-left process could swap a directory in the path for a link to `/dev`. With `O_NOCTTY|O_NONBLOCK` and the regular-file check, the effect is an open and a close of the device: nothing is read and no controlling terminal is taken. An `openat` walk from the nested `.git` descriptor would close it.
  - **No test pins the 10,000-entry cap of the refs and worktrees walk (n3), or that `restore_owner_access` changes a mode only when the owner lacks a permission (n4).**
  - **The legacy-config bytes test does not cover `quoted`'s escaping**, which is shared with the current config writer rather than frozen with the 0.1.x text; a change to `quoted` would change what the legacy check accepts.

### Stream P

- **The `Orphan` and `ps` test helpers are copied** (about 90 lines) from `daemon/tests/run_journal/sessions.rs` into `daemon/tests/reconcile_journal_pid.rs`. Move them into `daemon/tests/support/` (stream C's file during 9.11, so not done there).
- **No start-time identity check on a recorded or journaled pid** (decision 20 and M9.11.20 forbid a new check). The residual: a reused pid whose new owner is an orphan of the same uid carrying the same session UUID as a whole argv element. It is the same as for a `run.json` pid today.

### Stream T

- **Three client clocks are still read directly** (review M1, M2; files outside stream T): `app/toast.rs:42` stamps a toast with `Instant::now()` (its expiry reads the injected `now`), and `app/replies.rs`'s `expire_replies` and `app/link.rs`'s `check_stopping_timeout` read the real clock inside `on_tick_at`. Stamp with the tick's instant and pass `now` through, as decision 23 does for the rest.
- **An expired background `profile status` shows the generic no-reply toast** (review M8, raised by the re-review's N2): `app/replies.rs` does not count a dropped-goal Status as owned, and the dropped-goal wait re-asks every second or so, so a slow daemon can repeat that toast every few seconds. Raise its priority.
- **The dropped-goal wait re-asks on every snapshot** (N1): a goal that left the queue for a reason other than a start (removed, cancelled, a hand-edited queue) has neither a run nor a drop, so its project is asked about at most once a second for up to 1220 s times its position. The rate is the controller's ruling; a growing gap (doubling per ask, 1 s floor) would tighten it.
- **`the_wait_ends_silently_at_its_bound` does not show the wait alive one second before its bound** (N4): the `bound - 1` snapshot sees no ask only because the first ask is still in flight. `a_goal_behind_another_waits_two_budgets` pins the multiplier; only position 1's last second is not pinned.
- **Two queued goals with one text in one project, one started and one refused in the same snapshot** (M9): the run matches both, so the refusal is not asked about and raises no alert. Decision 22's own matching rule, kept.

### Stream S

- **`RETRY_WINDOW` (30 s) is nearly filled at worst-case timings** (S28-m2): `daemon stop`'s 19 s and `daemon start`'s 10.25 s now nearly use the whole reconnect window of stage 11 of `pty-smoke.py`.
- **No full-screen repaint stand-in test** (S26-m1, optional): `server_winch.rs` proves the nudge's `SIGWINCH` with a trap, not a program that redraws.
- **`cli/tests/support/mod.rs`'s `anthrex()` waits a literal 3 s `finish`.** It is not a start wait, so M9.11.28b left it; derive it if it ever fails.
- **`daemon/tests/server/worktree.rs`'s `a_create_runs_to_completion_after_its_client_disconnects` and `list_is_answered_while_a_create_is_running`** failed 3 of 4 runs under the macOS first-exec stall, on the base binary as often as on stream S's (`timed out waiting for git to reach the checkout hook`, 20 s). The stand-in's first exec is paid inside the bound; a warm-up, as M9.11.36 and .38b did, would remove it.

### Stream X

From the stream X review and the M9.11.33b review.

- **More tokens are still visible to checks** (X-M5). Decision 32's list is binding for 9.11, so it was not widened. `crates/config/src/reserved_env.rs`'s `CHECK_SECRET_NAMES` and `CHECK_SECRET_PREFIXES` still let checks, proofs, `setup` and tier steps inherit at least:
  - `SSH_AUTH_SOCK` (a check can push with the user's keys)
  - `CI_JOB_TOKEN`, `GITLAB_PRIVATE_TOKEN`, `HOMEBREW_GITHUB_API_TOKEN`, `VERCEL_TOKEN`, `SENTRY_AUTH_TOKEN`, `GOOGLE_API_KEY`
  - `KUBECONFIG`

  Widening the list needs a ruling, since each name can break a user's check (`ANTHREX_CHECK_ENV_PASS` passes one back). `SSH_AUTH_SOCK` in particular is a socket, not a secret value.
- **Token sums outside `TokenUsage::billable` still use plain addition** (X-M7). B8 made `billable()` saturate. These still use `+` or `+=`:
  - `crates/daemon/src/run/engine/design_agents.rs:297`
  - `crates/daemon/src/run/engine/design_reviewer.rs:244`
  - `crates/daemon/src/run/engine/design_spend.rs:94`
  - `crates/daemon/src/scout/machine.rs:318`

  A huge or hostile usage report can overflow them: a panic in a debug build, a wrap in a release one. Fix direction: `saturating_add`, as `billable()` does.
- **The profile service's `blocking` helper has no timeout** (33b review m4, pre-existing; `crates/daemon/src/profile/service.rs`'s `blocking`, which wraps `spawn_blocking`). A stalled file write or fsync holds the project's `writes` lock indefinitely, and hard rule 2 asks for a timeout. `save_if_current`'s failure branch adds a second untimed call. Fix direction: a bound on each call. It needs care, because the write can still land after the timeout fires, so memory must be noted from what the file holds afterwards.
- **A failed proposal save still reports `true`** (33b review m7, pre-existing; `profile/service_run.rs`'s `save_if_current`). The detection carries on to `after_ready` and unregisters while `proposal.json` stays at its last saved state (for example `Verifying`). `running()` then reports it in progress until a restart's `restore` fails it.
- **Write-then-note and delete-then-note paths keep a short reverse gap** (33b review m6). In `service_edit.rs`'s `save_over` and `verify_row_edit`, the file's `updated_at` can run ahead of the snapshot. In `store_ready` and `reject`, the snapshot can list a ready proposal for one write after the file shows it stored or gone. No reader acts on it today. It matters only if a client starts keying on `updated_at`.
- **A failed proposal write can move the ready and set-up generations twice** (33b review m2, not applied): a client can see a `Ready` alert appear and then be withdrawn, and a failed reload after it falls back to `None`. Restoring the exact earlier memory needs the table's previous entries kept across the write (more code in `service.rs`).
- **The skip text for a hand-edited `format` still says "written by a newer anthrex"** (33b review m5, not applied): a `null` or `"2"` format is skipped with decision 36's template, which names a newer anthrex though none wrote it.

### Stream C

- **Two unattributed stress failures.** Under 20 rounds of 12 parallel copies, `status_codex::a_screen_drawn_in_two_writes_is_scanned_whole` failed once (`(Working, false)` in `signalled` after 5 s, with copies running about ten times slower than alone); two early whole-file runs of `profile_queue.rs` failed without a log. Neither recurred in the logged runs since.
- **M9.11.38c's Linux re-exec has run only on CI** (nexus1 unreachable): `manager/lifecycle.rs`'s `shutdown_waits_for_cleanup_after_the_group_leader_exits` now asserts `running 1 test` in its child; the PR's ubuntu job is its first Linux run.

### The independent whole-PR review of #57 (2026-10-11)

Five minors from the merge review of milestone 9.11, none blocking:

- **A git-probe reaping test can skip its check** (`daemon/tests/git_probe.rs:376-386`). It returns early when no grandchild pid was recorded, so the stall variant never checks that the grandchild is reaped, and the no-stall variant skips the check when the kill wins the race. Record the pid before the race, or fail when it is missing.
- **A wheel scroll can leave an orchestrator wake unsubmitted** (`daemon/src/run/driver/wake.rs:121-127`, `tui/src/app/mouse.rs`). Wheel scrolls go through `write_client_input` as client input, so a scroll during the 200 ms `SUBMIT_DELAY` leaves the wake pasted but unsubmitted while `OrchestratorWoken` is recorded. Scrolls should not count as typing.
- **The nested probe's `read_dir` follows a link at the directory itself** (`daemon/src/run/git/salvage_nested.rs:280`, `:298`). That applies to the nested `.git/refs` and `.git/worktrees`, which contradicts the module's "never through a link" doc. Open each with `O_DIRECTORY|O_NOFOLLOW` and read through the descriptor.
- **A cancelled queued goal makes the client poll `profile status`** about once a second for up to `GOAL_START_BUDGET` (`tui/src/app/alerts_dropped.rs`). This is the same item as stream T's N1 above.
- **A worker can trigger `SandboxBroken` by printing the sandbox tool's error text** (`daemon/src/run/observe.rs`). The signal is read from tool output a worker controls. Its effect is a note in the next block or failure, not a change of power. Tie it to the tool result's exit status, or to a daemon-side probe.

### The machine, across every stream

- **macOS's first-exec stall still fails stand-in tests in bulk under load.** With XProtect busy, the first exec of a freshly written script took 11-48 s on the implementers' machine while other streams built; every test that execs a fresh stand-in inside its bound (git wrapper scripts, warm-up-less rigs) then failed together, 25 to 43 at a time, and passed once the load fell. Milestone 9.11 warmed the rigs it owned (M9.11.36, .38b, the CLI harness's `warm_anthrex`); `docs/timing-budgets.md` records the pattern and its earlier warm-up fixes. A shared warm-up helper for every stand-in writer would retire the pattern.

## From milestone 9.12 (2026-10-11, recorded by M9.12.4's fix round 1), for later

- **One file gate for the e2e tests.** `crates/cli/tests/run_e2e_stages.rs` (`wait_for`, `missed`), `run_e2e_pr_ci.rs`, `run_e2e_pr_merged.rs` and `run_e2e_merge.rs` (`gate`, `missed`) each carry a copy of the same scripted file gate (`sh` steps of 1500 x 0.2 s, a `.missed` marker, an `expect` on `/go`). Move it to `crates/cli/tests/support/run_plans.rs` once, and point the four files at it.
- **Two test files past hard rule 8's size.** `crates/cli/tests/hook_command.rs` (about 990 lines) and `crates/fake-agent/tests/script.rs` (about 690) grew past 600 before M9.12; split each by responsibility (M9.12.7's binary merge is a natural moment for the first).


### Recorded by M9.12.12 (2026-10-11)

- **`CARGO_BIN_EXE_*` and `CARGO_TARGET_TMPDIR`, for 9.14.** Both are compile-time absolute paths, like `CARGO_MANIFEST_DIR` ("Spec and brief defects" 4 of `docs/milestones/M9.12-dev-speed.md`). Under 9.14's shared target directories they point into the shared directory, where another checkout may have built the binary a test execs. M9.12.11's `testexec::checkout_dir` guards only `CARGO_MANIFEST_DIR`; 9.14 decides whether these need a guard of their own or a per-checkout binary path.
- **Transitive engine purity.** M9.12.10's rule checks only what `crates/daemon/src/run/engine/` names itself (decision 16, Scope "Out"). The functions it calls (`run::history`, `run::role_launch`, `run::env`, …) are pure today (`role_launch.rs:383`, `env.rs:10`), but nothing keeps them so. A check that follows the engine's calls into the rest of `run`, or moving those helpers under `run/engine/`, would close the gap.
- **`.tmp*` leak coverage.** `tempfile::tempdir()`'s default directories (`.tmp` plus random characters) carry no anthrex prefix, so the leak gate cannot attribute a leftover one to this repository's tests and does not look at them. Giving every test temporary directory an `ax-` prefix (one helper in each crate's test support) would put them all under the gate.
- **A pure `anthrex-run-core` crate** (the M9.12 research's item 4). The engine-purity rule is its cheap first step; moving `run/engine/` and the pure helpers it calls into a crate with no `tokio`, `std::fs` or `std::process` in its dependencies would make the compiler enforce what the rule checks by text.
- **`docs/MODULES.md` and a module-map test** (the M9.12 research). A one-page map of the crates and the daemon's top-level modules, with a test that fails when a module is added or removed without the map changing.
- **Two `#[allow(clippy::duplicate_mod)]`s in each merged binary.** `daemon/tests/it/` mounts `run_journal/fixture.rs` twice (`run_journal_format.rs`, `reconcile_journal_pid.rs`) and `cli/tests/it/` mounts a `common.rs` twice (`run_e2e_design_rounds.rs`, `run_e2e_large.rs`); M9.12.6 and M9.12.7 kept the copies because sharing one needed visibility changes, not a move. Mount each once and share it.
- **`full_profile()` exists twice.** `crates/cli/tests/it/tui_contract.rs` carries a verbatim copy of `crates/tui/src/profile_view_tests.rs`'s private fixture (M9.12.9). A change to one will not reach the other.
- **`daemon_start_lock.rs` in the merged CLI binary.** It holds a real `flock` in-process and releases it on a thread; in one binary more sibling tests fork, and a fork before exec can briefly hold a copy of that descriptor (the `lockfile.rs` mechanism, decision 14). Its waiter retries for seconds, so M9.12.7 did not keep it alone; if it ever flakes on a lock that should be free, that is the cause and the fix is a binary of its own.
- **Two old `cargo doc` warnings in the client.** `crates/tui/src/tree/task_rounds.rs:10` links the private `rank`; `crates/tui/src/ui/statusbar.rs:379` links an unresolved `git_spans` (seen by M9.12.9, unrelated to it).

### Recorded by M9.12.13 (2026-10-11)

- **A run's check directories outlive it, for users too.** The temporary directory of a check in the integration checkout, the tier-3 `.full` checkout or a stage's proof is `task_tmp(<run data>/tasks/<checkout name>)` (`run/driver/tier_step.rs::step_base`, `scheduled.rs`, `gate_ops.rs`), but only `run/git/checkout.rs::remove`, `run/reconcile/git.rs` and `profile/verify.rs` ever remove a task's directory, and the integration and `.full` checkouts are worktrees that never go through them. So every accepted or discarded run that ran a check leaves one or two empty `/private/tmp/ax-<uid>/<16 hex>` directories behind (the smoke's accepted runs showed it: their `tasks/integration` directories were left after `run accept`). M9.12.13 removed only the tests' and the smoke's copies (a test teardown, no product change, per its ruling). The product fix: remove the run's step bases when the run's checkouts are removed (accept, discard, the cleanup sweep). This is probably most of the 38,573 directories found on the development Mac.
- **`scripts/screenshots.py` has no task-temporary teardown.** Its scenes run their own daemons and remove their roots (`screenshots.py:352`) like the smoke's stages did before M9.12.13; it is not part of the merge gate, so the leak gate never sees it. Calling `smoke_task_tmp.remove_task_tmps_beneath(self.root)` before the `rmtree` would close it.
- **Every in-process rig that drops a live `RunService` can race its directory.** M9.12.13 gave the three rigs the gate caught (`orch_read_rig.rs`, `role_route_tests.rs`, `server_runs.rs`'s `rig_on`) a bounded `stop().await` that the rig's `Drop` enforces (for `server_runs.rs`'s `Stop` since its fix round 1). Other tests that spawn a run service and only cancel its token (`server_runs.rs`'s `a_disconnected_client_is_not_held_open_by_its_run_request` and `approval_holds_are_answered_by_the_engine`) have not leaked in the runs seen, but have the same shape.


### Recorded by milestone 9.12's final reviews (2026-10-11), for 9.13 and 9.14

- **For 9.14: two copies of `TestDir`** (final review M4). `crates/daemon/src/run/git/tmp.rs`'s `#[cfg(test)] TestDir` and `crates/daemon/tests/support/mod.rs`'s are near-verbatim and differ only in the lock helper; a fix to one (a new spelling, a new worktree checkout) will not reach the other. Share one.
- **For 9.14: the task-temporary cleanup walk exists three times** (second review). `run/git/tmp.rs::remove_task_tmps_beneath`, its copy behind `tests/support/mod.rs`'s `TestDir`, and `scripts/smoke_task_tmp.py`; only the hash is pinned across them (`the_smokes_pinned_name_is_this_hash`), not the spelling variants tried (as given, resolved, resolved without `/private`) or the `WORKTREE_CHECKOUTS` lists (`["integration", ".full"]` in `tmp.rs:101` and `smoke_task_tmp.py:27`). Pin both, or derive the Python side from the Rust one.
- **For 9.13: blocking file-system work inside async test teardowns** (final review M5). `remove_task_tmps_beneath` runs on a tokio worker in `orch_read_rig.rs`'s and `role_route_tests.rs`'s `stop()`, and `daemon/tests/it/agent_hooks.rs`'s `Agent::new` spawns its warm-up process synchronously. Test-only, so hard rule 2's production concern does not apply, but a slow `/tmp` walk stalls the test's other tasks; move each onto `spawn_blocking`.
- **For 9.13: the environment-mutation rule sees only test sources** (final review M7). The daemon library calls `libc::umask(0o077)` in `lifecycle::bind_socket` (`daemon/src/lifecycle.rs:98`, serialised only by its own `UMASK_LOCK`). No merged-binary test calls `bind_socket` in-process today (`lifecycle_resume.rs`, which runs `daemon::run`, is kept alone), but one that did would flip the process umask under its siblings and the rule would not see it. Extend the rule to the library functions the merged binaries call, or document the limit beside decision 14's rule.
- **For 9.13: PR #58 carried the 9.13 to 9.17 briefs and the nucleos spec** (final review M8). The branch was cut on the docs branch (ledger line 2), so they reach `main` with 9.12 rather than by a direct docs push. Nothing to undo; a brief correction found before 9.12 merges goes on this branch, after it on `main`.
- **For 9.13: the unmounted-file rule counts a disabled `mod` line as reachable** (re-review 1). `daemon/tests/it/source_rules/targets.rs`'s `mod_decls` ignores cfg attributes and macro bodies, so `#[cfg(any())] mod x;`, `#[cfg(not(test))] mod x;`, an off feature, or a `mod x;` in an uninvoked `macro_rules!` passes falsely; treat a non-platform `#[cfg(` on a `mod` as a hit, or document the limit.
- **For 9.13: the unmounted-file rule resolves a `mod b;` nested in an inline `mod a { … }` against the file's own directory** (PR #58 review). `targets.rs` reads only out-of-line declarations' names, so `mod a { mod b; }` in `x.rs` looks for `x/b.rs`, not `x/a/b.rs`; the right file is reported unmounted and the wrong one counted as reached.
- **For 9.13: the unmounted-file rule exempts any `fixtures` path component** (PR #58 review). `targets.rs:125` skips every file with a `fixtures` component anywhere under `tests/`, so `tests/it/fixtures/real_test.rs` is never checked; exempt only `tests/fixtures/` at the crate root.
- **For 9.13: the unmounted-file rule's crate list is hard-coded to daemon and cli** (PR #58 review). `every_integration_test_file_is_compiled` walks `["daemon", "cli"]`; a third crate that sets `autotests = false` would go unchecked. Derive the list from the manifests that set it.
- **Fixed in 9.12 (`a81929c8`): `code_only` does not recognise `br"…"`/`cr"…"` raw strings** (re-review 2). `daemon/tests/it/source_rules.rs:70-73`'s `!is_ident(b[i-1])` guard rejects the prefix, so after a `br"\"` the env rule and the manifest-dir rule miss a following `set_var` silently (the engine scan panics); accept `b`/`c` before `r` when the byte before is not an identifier byte.
- **For 9.13: the manifest-dir rule misses `env![…]` and `env!{…}`** (re-review 3). `source_rules.rs:426` strips only `(`; both other delimiters compile. None exists in the tree.
- **For 9.13: three scanner nits** (re-review 4 to 6). Code on the same line after a skipped `#[cfg(test)]` item is skipped too (`engine.rs:245`, `:249`; unreachable under `cargo fmt`); `opens_items` treats an `extern "C" fn` body as an item-holding body (`engine.rs:137`); the env rule does not follow a function-pointer binding (`let g = std::env::set_var::<&str, &str>; g(..)`).
- **For 9.13: the environment rule misses cross-file aliases** (PR #58 review; the function-pointer form is the line above). An alias is followed only within the file that declares it, so `pub(crate) use std::env::set_var as sv;` in one module and `sv(..)` in another passes.
- **For 9.13: the manifest-dir rule misses `concat!("CARGO_", "MANIFEST_DIR")`** (PR #58 review). `names_manifest_dir` wants the literal whole, so `env!(concat!(…))` is never seen.
- **For 9.13: the manifest-dir guard accepts any `*_checkout_dir(`** (PR #58 review). `unguarded_manifest_dirs` checks the text before `env!` with `ends_with("checkout_dir")`, so `my_checkout_dir(env!("CARGO_MANIFEST_DIR"))` passes; require a word boundary before `checkout_dir`.
- **For 9.13: `server_runs`' leak fix (`2d1655f6`) is containment, not the root cause** (PR #58 review). Each rig's worktrees root now sits beside its socket, but `Stop::stop` still does not await a goal's run build still in flight after the `Triaged` reply; the build can outlive the test inside the rig's own directory. Have `stop()` wait for that build.
- **Fixed by M9.14.11:** **For 9.14: `ManagerConfig::for_tests` still defaults to the shared `<temp>/anthrex-worktrees`** (M9.12.13's CI-only leak follow-up). Twice now (`delivery_tests_start.rs`, `server_runs.rs`) a test that starts a run reached it and left a checkout there; each rig was given its own root, but any new rig built on `for_tests` inherits the hazard. Default it to a root no test shares (or one that panics if used), so a forgotten override fails at once rather than on a slow CI machine.
- **Fixed by M9.14.11 (`RunService::wait_for_ops`, shared by all three rigs):** (9.14) Other rigs whose tests start ops — the role-route rig and `server_runs.rs`'s `Stop` — may share the read-path rig's unawaited-op race (`9d7e605e`: `stop()` returned while a refresh's hand-back git work still staged `.anthrex/<task>/engine`). Give each rig the same op-order-lock wait, or one shared helper.
- **For 9.13: two flakes seen on PR #58's macOS CI (2026-10-11), in code the PR did not change.**
  - Smoke stage 11k (`scripts/pty_smoke_design.py`) pressed `a` at the brainstorm gate while the screen still read "the document is not loaded; a approves it once it is", then timed out waiting for "Approve brainstorm v1? Spec starts next." The stage should wait for the loaded document (for example, its first heading in the left pane, or the absence of the not-loaded line) before it presses `a`. Job 114525367271; a rerun passed.
  - **Fixed by PR #59 (f8c51c75):** `run_e2e_orch::alerts::e2e_an_environment_block_is_the_orchestrators_and_it_retries` failed at `cli/tests/support/run_orch.rs:167` with "orchestrator-run-1 read [], not 1 messages, within 120s". The daemon logged its wake-up as delivered to window 1 (417 bytes), but fake-agent recorded no read. Seen once, in job 114518949316; it passed on rerun and locally. If it recurs, check whether the wake-up's PTY write reaches a fake-agent that has not yet started reading.
- **A private `hh_mm` duplicates `common::text::hh_mm`** (task 9 review M-5). `crates/daemon/src/run/orch/contract_messages.rs:131` has its own copy; call the shared one.
- **A daemon restart drops a pending "do it" that waits for its commit count** (task 9 review M-7). An override proposal the user approved, waiting for `CountCommits`, is lost on restart and the orchestrator is not told.
- **A fresh session's handoff seed passes on a summary a tainted session may have written** (decision 18). By design, since the taint ends with the session unless adopted; recorded for review.

### Recorded by milestone 9.13's pre-flight scan (2026-10-12), for later

- **Redirect and `tee` targets become pinned gate files** (pre-flight C27; 9.13 decision 8 is final). `gate_words` splits on `<`, `>` and `|` and keeps every word, so `tee out.txt` and `> report.txt` leave `out.txt` and `report.txt` as words. If such a file is tracked it is pinned, and since gate commands run in the checkout (`step.dir`), a gate command that writes it makes the next gate command refuse it and halt the run. Today's way out is to grant the file in a task's `owns` (U1) or not to track it. Fix direction, if the user rules for it: drop the word after `>`, `>>`, `2>` and `tee` in `crates/daemon/src/run/gate_words.rs`.
- **Files near the limit: `crates/daemon/src/run/git/mod.rs` is past 600** (pre-flight C12). It was 602 on `main` at `71eaf198` (9.12's `48efcba5` took it from 600), and M9.13.5's `gate_files: Vec::new()` in its `Preflight` literal makes 603. Splitting it by responsibility (the `Preflight` builder, `protected_files`/`tracked_at`, the `Git` wrapper's flags) is out of 9.13's scope; the 9.11 follow-ups already list it under category F.
- **Fixed in 9.13 (controller ruling before M9.13.6): a `.` word pins every top-level tracked file** (found by M9.13.5). `gate_words` now drops a bare `.` and strips a trailing `/`, so a directory word is a tree to `ls-tree` and pins nothing. Decision 8 keeps a bare `.` (it is not empty, not absolute, has no `..` component), and `git --literal-pathspecs ls-tree <base> -- .` lists the root tree's direct entries, so a check such as `find . -name '*.sh' | xargs sh` pins `Cargo.toml`, `README.md` and every other root-level blob. A task that changes one then needs it named exactly in `owns`. Safe, but noisy; dropping `.` (and other directory-only words) in `gate_words` would fix it.
- **`verify` compares working-tree bytes with the blob** (task 6 review M-4). A gate file under `.gitattributes` eol, filters or LFS checks out different bytes from its blob, so every gate command is refused and the run halts although nothing changed. Consider hashing through git's filters on a mismatch.
- **A worktree-prepare refusal does not set `worktree_live` or emit `WatchWorktree`** (task 6 review M-1). The `GateRefused` arm for `PrepareWorktree` should mirror `dispatch::worktree_done`'s created branch, or a task cancelled afterwards keeps its worktree.

## From milestone 9.13 (2026-10-12, recorded by M9.13.11), for later

- **A binary from before 9.11 can drop 9.13's run fields** (brief, Risks "Persistence downgrade"). `RUN_FORMAT` is 2, so a 0.2.0 daemon leaves a 9.13 run alone; a 0.1.x daemon has no format guard and drops `untrusted`, `orch_retries`, `gate_files`, `AskRecord.proposal`, `OverrideCount.proposal` and `CountCommits.{owns, protected}` when it saves. CHANGELOG says so; nothing more can be done in the old binaries.
- **The orchestrator's own `Read` of `run.json` does not taint** (brief, Out and Risks). `run.json` holds raw comment text, and the orchestrator's `Read` tool can open it. Fix direction: a PreToolUse hook on the orchestrator session that reports a read under the run's data directory as an untrusted read (`OrchEvent::UntrustedRead`).
- **`cd sub && ./run.sh` is not pinned** (NA §2 B risks). `gate_words` reads `run.sh` relative to the repository root, so a gate command that changes directory first pins nothing for that script. The user can add it to `protected`. Following `cd` in `gate_words` would close it.
- **`run resume --rebaseline` does not re-read the gate files** (brief, Out and Risks). A gate script changed on the base branch then halts the rebaselined run on its first gate command, and the user must grant it. Re-read `Run.gate_files` at the new base on a rebaseline.
- **`gmake` is not an implicit-file program** (decision 8). PR #60 review I3 added `Justfile` and `.justfile` to `just`'s names and pins the names a program would prefer as absent; a profile that calls make as `gmake` (or `make -f`, `make -C`) is pinned only by what its words name.
- **The user cannot amend a task's `owns`** (found by M9.13.4, met again by M9.13.11). `PlanEdit::AmendTask` has no `owns`, so the C25 way out after the user overrode a change to a pinned gate script is to add a task whose `owns` names the script (`anthrex run edit`), then override the blocked task again: a `run retry` sends it back to a worker whose `task_done` is still bounced, because its own `owns` does not name the script. An `owns` amendment for the user's `run edit` (decision 6's grant rule already covers a growing `owns`) would let the user grant the file on the task itself.
- **Files at the limit.** `crates/daemon/src/run/orch/mod.rs` and `crates/daemon/src/run/driver/orch.rs` are 599 lines each, `driver/build.rs` and `engine/planners.rs` 593, and `run/git/mod.rs` 603 (recorded above). The next change to any of them should split it first.
- **The driver's orchestrator-role check on `run_status` and `task_result` is unreachable** (M9.13.8). Only the orchestrator role can call either tool (`orch/tools.rs::parse_call`), so no test can fail on the check's removal. Keep it, or pin it with a unit test on the driver's helper if the tools ever open to another role.
- **The manual check is outstanding** (brief, "Manual check"). It runs the real CLIs and is the user's: a rewritten `scripts/check.sh` halting the run, and the untrusted line and the `orchestrator asks` alert after a real PR review comment.

### Recorded by PR #60's review fixes (2026-10-12), for later

- **The `attention` block line matters only for a digest trimmed past 48 KiB.** `snapshot::attention` writes `{id} blocked ({reason}): {text}` for every blocked task, a review or CI fix task's too. While the task's entry is in the digest its `block` taints (decision 19's second rule), so the line is never shown alone; only a digest trimmed past its 48 KiB budget, which drops task entries before the `attention` lines, could show a fix task's block text without its entry. Fix direction: write a fix task's attention line without its text, as its block wake is (D-25), or let `digest_source` read the line as it reads the note line.
- **The C25 override reply should say how to grant the file.** The user's override of a change to a pinned gate script replies `; {path} stays pinned until a task's owns names it (run edit)`. `run edit` cannot amend an existing task's `owns` (see above), so the reply should say to add a task whose `owns` names the file (`anthrex run edit`).
- **Committed symbolic links into protected directories, beyond ancestors.** PR #60 review I2 makes a changed path that is a symbolic link a protected change when it is an ancestor directory of a protected entry's literal leading path. A link elsewhere is an ordinary change, and two cases stay open: a link inside a protected directory pointing out of it (`.github/workflows/ci.yml` committed as a link to `x.yml` is caught already, as a changed protected path, but a link created at base and retargeted outside the run is not the gate's concern), and a protected entry whose leading component is a wildcard (`**/*.sh`, a user-added `*/conf/**`), which has no literal ancestor to guard. A general rule would refuse any committed symbolic link whose target resolves into, or whose path lies above, a protected match; it needs the link targets from the head tree (`git cat-file`), one more git call.
- **A tainted `override`'s question cannot name the protected paths.** Decision 21's question is built when the orchestrator's `override` arrives; the task's protected changes are only known after the user's `do it` runs the count (`OverrideCount`, `gates::override_counted`), whose reply and wake note list them. Naming them in the question needs a count before the ask, or the done gate's last `protected_changed` kept on the task.

## From milestone 9.14 (2026-10-12, recorded by task 6's review fixes), for later

- **A dependent task's worktree can build cold for its whole life** (M9.14.6 deviation 1, task-6 review M4). `remove_merged` emits a merged task's checkout removals in the same step that dispatches its dependent, so the dependent's `PrepareWorktree` leases while the predecessor's releases are still cleaning (a release frees only after its `cargo metadata` clean). With `[target_pool] task_dirs` tight it gets `Full`, falls back with one note, and C24 keeps it on `{worktree}/target` until its own release: the dependency chain loses the pool's main saving (U6). Fix direction: dispatch a dependent task only after its predecessor's releases have finished, or let a lease that finds the pool full wait for a release in progress instead of falling back for life. Either would also let `run_e2e_pool.rs::e2e_a_second_task_reuses_the_released_dir` (and M9.14.10's `e2e_two_tasks_on_one_pool_dir_the_second_runs_its_own_code`) pin "no pool note" and "t2's `.proof` builds in the directory t1's `.proof` released" outright; today the test gates only t2's `.proof` (task-6 review I1) and pins the reuse it can observe. Ordering alone does not give F8's reuse: in three runs of that test t2's worktree always fell back, t1's check had built in `task-2` (t1's `.proof`), and t2's `.proof` then chose by decision 5's least-recently-used rule between `task-1` (t1's worktree, which built nothing) and `task-2`, released in the same second or nearly so; a tie goes to `task-1`. So a dependent's check usually lands in the predecessor's worktree directory, not in the one holding the predecessor's build. Decision 5 could prefer, among free directories, the one most recently released by a `.proof` (or the larger one) for a `.proof` lease.
- **The conversation view ignores `PostToolUseFailure`** (M9.14.9). Since F4 the Claude hook settings register `PostToolUseFailure` (a failed `Bash` fires it instead of `PostToolUse`), for admission's release. `conversation/build.rs` treats it as a no-op, so a failed call's block stays open in the hook-built draft until the transcript reader catches up, exactly as before the event was registered. Its payload has `error`, `is_interrupt` and `duration_ms` and no `tool_response`: a later milestone could close the block as failed (`ok = false`, the error as its result line).
- **anthrex's own stored profile: `crates/fake-agent` belongs in `hub` and `full_triggers`** (M9.14.10, controller). The cli and daemon integration tests run the `fake-agent` binary (`CARGO_BIN_EXE_fake-agent`, `fake_agent_bin()`) without declaring it as a dependency, so the module graph does not know they read it. Under decision 26's inputs id a change only inside `crates/fake-agent` keeps the cli's and daemon's `module_test` keys, and a cached green can stand for tests that would now fail (M9.14.10 changed `fake-agent`'s `bash` step exactly so). Until the profile lists it, tier 3 is the only backstop.
- **More of cargo's environment belongs in the tier cache key** (M9.14.10, controller). Decision 29's `TOOLCHAIN_ENV_VARS` covers `RUSTFLAGS`, `RUSTDOCFLAGS`, `RUSTC_WRAPPER`, `CARGO_ENCODED_RUSTFLAGS` and `CARGO_BUILD_TARGET`, but not `CARGO_PROFILE_*` (debug, opt-level, overflow checks), `CARGO_BUILD_RUSTFLAGS`, `CARGO_TARGET_<TRIPLE>_RUSTFLAGS` or `CARGO_INCREMENTAL`. A step whose environment changes one of them keeps its key and can hit a result built differently. Add them (a prefix rule for `CARGO_PROFILE_` and `CARGO_TARGET_*_RUSTFLAGS`), keeping today's key when none is set.
- **A purity check for the pool's pure files** (M9.14.10, controller). The Verification grep keeps `run/target_pool/{book,book_file,clean}.rs` free of I/O only when someone runs it. Add it to the `source_rules` tests (as their other greps are), so a later change cannot add `std::fs`, `std::process`, `tokio::` or a clock there unnoticed.
- **Decision 5's same-second tie should prefer the directory that already holds build output** (M9.14.10, controller; task 7 concern 2). Two directories released in the same second tie on `last_used`, and a tie goes to the lowest `k`, which is usually the predecessor's worktree directory (which built nothing) rather than its `.proof`'s (which holds the build). Break the tie toward the directory last released `Clean` by a checkout that built (or the larger one); the item above on dependent dispatch has the evidence.
- **The heavy matcher's open false positives and fail-open misses** (M9.14.7, task 7 review M3 parked; decision 18). False positives: a heredoc body (`cat <<EOF` … `cargo test` … `EOF`) and a `#` comment (`ls # cargo test`) are split into segments and can count as heavy; harmless (an admitted light command waits for nothing it needs), but they can hold slots. Misses, all fail-open and recorded as advisory: `bash -c "cargo test"` and `sh -c`, `eval`, scripts and Makefiles that run cargo, a `$(…)` or backtick substitution, an alias or shell function, `xargs cargo …`, and `cargo` reached through a variable (`$CARGO test`).
- **`ADMIT_HOLD_MAX_DEFAULT_SECS` = 630 rests on an unconfirmed 600 s** (M9.14.7, C8 (a)). The Claude `Bash` tool's longest foreground `timeout` (600 000 ms) was not read from the installed binary: the narrower `strings` extraction was refused by the session's permission classifier. Someone with permission should run `strings <claude binary> | grep -o 'Optional timeout in milliseconds (max [0-9]*)'` and the `BASH_MAX_TIMEOUT_MS` default, and adjust the default (600 + `ADMIT_HOOK_SLACK_SECS`) if it differs.
- **`daemon/src/server.rs` is at its 595-line limit** (task 9 review Minor 5). The next task that touches it splits it first, for example by moving the `HookEvent` and `Admit` dispatch into `server/admit.rs`.
- **`AllBut` could be scoped by `agent_id`** (task 9 review Minor 3). The status hook forwards every tool's `PreToolUse`, a subagent's too, and holds are keyed by window only, so any tool call of a concurrent Task subagent releases the main agent's (or another subagent's) running `cargo` hold early. Fail-open by C8 (c), never a deadlock; keying holds by `(window, agent_id)` would narrow it.
- **`fake-agent` fires `PostToolUse` for a failed `bash` step too** (M9.14.10). The Interfaces define one `PostToolUse` per `bash` step, with its `exit_code`; the real CLI fires `PostToolUseFailure` instead for a failed `Bash` (F4). Both release a hold, so no test depends on it; a test of the failure path through `fake-agent` would want the real CLI's choice.
- **Brief follow-ups not yet taken** (the brief's "Follow-ups handled"): Codex admission once fact F6 is known; a TUI line for the pool's leases and for admission holds; aging of agent asks if starvation shows up.

### From the PR #61 review (2026-10-12, Master), for later

- **More heavy-matcher misses** (fail open). `timeout N cargo test` is the cheapest fix: add `timeout` to `COMMAND_WRAPPERS` and skip its duration argument. Also missed: `stdbuf`, `sudo`, `xargs cargo …`, the cargo plugins `llvm-cov`, `miri test`, `insta test` and `hack check`, `make test`, and `bash -c '…'`.
- **C11 gap: planted registry artifacts survive two release paths.** A task merged earlier in a run that is later discarded, and a cancelled task of an accepted run, both release with `clean = true`. A registry artifact planted in their directory survives.
- **Unconfined checks (Linux): `cargo clean -p` follows planted symlinks.** A symlinked `debug/deps`, `build` or `.fingerprint` points the clean out of the pool directory.
- **The `cargo metadata > holder/metadata.json` redirect follows a link.** The holder name is pid + sequence, so a race can plant a link there first.
- **First-lease race on a Dirty directory.** A `wipe_first: false` re-lease can take the directory mutex before the wiping lease. No engine path does this today.
- **A crash mid-release leaves the lease until the hourly sweep.** A `RemoveWorktree` replayed through `reconcile/mod.rs:211` has no pool release.
- **An unreadable or other-version `leases.json`** marks every directory Dirty with no owners, while a persisted worker spec still names its old `task-k`.
- **`CARGO_TARGET_TMPDIR`, `target/nextest` and similar persist across owners** of a pool directory.
- **A `spelled()` blocking failure falls back to the unresolved path**, which gives one pool two books. This happens only at shutdown.
- **A pool directory named like `llvm-cov-target/debug` is taken for a target-triple directory** and wiped on every release.

## From the flaky-test root-cause work (2026-10-13), for later

- **A restarted window's early events land on the old process's entry.** `create` and `create_run_window` now keep a window's events that arrive before phase C (`Inner::early_events`). `restart` has the same gap one layer down: `spawn_for_restart` (phase C, `manager/restart.rs`) starts the new process's reader and waiter, whose `Title` and `Exited` are applied to the entry while it still describes the old, exited process; `finish_restart` (phase D) then resets `state`, `exit` and `child_alive`, so a new process that fails at once, or a one-shot startup title, is lost and the restarted window stays `Starting`. Fix direction: open an `early_events` slot keyed by the restart attempt in `begin_restart`, route events to it while `entry.restarting` is set and the swap has not happened, and replay it after the swap in `finish_restart`. The old process's own late events must not go into it (`wait_for_exit` already orders them before phase C).
- **A restore that cannot read git drops an `Accept` for good.** M8a.21's reconcile rule ("a check that cannot read reality answers `NotStarted` with a note"; decision 44) assumes every dropped op is re-issued on resume. Nothing re-issues an `Accept`: when the 4 s git read in `reconcile::check` times out after a restart, the accept is dropped, the run stays `Complete`, and the only trace is a line in the run's own log. A merge that did land then has no clean-up, and a second `run accept` meets a moved base. Proposed: when a check cannot read reality for an `Accept` (or any op nothing re-issues), hold the run halted and retryable (as `hold_unreconciled` does) so resume re-issues it, and log a `tracing::warn!` beside each "could not check" note so the cause reaches `daemon.log`. It changes a recorded design rule, so it needs its own brief. Found by `run_e2e_finish::e2e_a_replayed_accept_reports_its_clean_up` timing out (a cold `git` stand-in after the restart, fixed test-side in 8b56b16f).
- **`run_e2e_large/restart.rs::e2e_claude_orchestrator_gets_the_otlp_environment`** passes a fresh wrapper as `ANTHREX_CODEX_BIN` with no warm-up; the startup Codex version probe (5 s) pays its macOS first exec. Probably harmless (the test drives claude); warm it if it ever flakes.
- **`crates/daemon/src/run/engine/planners.rs` is at 595 lines**; the next change there splits it first.
- **`requirements_stored` reads true after a failed read-back clears `approved_spec`** (PR #62 review, Master).
- **The early-event buffer has no explicit cap and coalesces only `Output` events** (PR #62 review). A window whose `insert` never comes is bounded today only by `Reservation`'s drop.
- **A refused submit or spawn may bump the run's revision**, which costs one extra `run.json` write (PR #62 review).
- **The design e2e harness now restarts the daemon once per test** to arm the read-back hold (PR #62 review). This costs time, not correctness. A per-run seam would avoid it.
- **The e2e waits depend on `requirements_stored`, a key the digest shows only during planning** (PR #62 review). A digest change that drops or renames it breaks the fixtures' waits.
- **A profile `setup` (or check) still running when its daemon stops outlives the daemon** (the rate-limit ordering flake, 2026-10-14). `scheduled::run` runs the command through `exec::run_isolated` on a blocking thread, which kills the group on its own timeout; when the daemon exits first, that thread is gone and nothing ends the group. Seen with a temporary `setup` that polled for a release file: after each failed test's daemon stop, its `/bin/sh` was reparented to pid 1 and ran on past the 10 s `check_timeout_secs`, until its own poll cap. A setup or check that hangs would stay forever. Proposed: track live step groups in the service and end them in the daemon's shutdown, as windows are.

- **Rig fixtures that change `runs.state` behind a live loop** (the chain long-poll flake, `chain_tests::two_runs`, fixed by `Rig::settled`). `idle`, `delivered` and the inline set-ups in `chain_tests.rs`, and the state edits in `orch_read_tests*.rs`, still change the engine state while the rig's loop may have a push of the older state in flight. None of them then long-polls on what it changed, so none can fail this way today; a new test that does must await `Rig::settled` after its change. `wait_digest` reading a run missing from a push as gone is correct only while every change to the runs is a loop step that publishes before the next one; worth a sentence in `orch.rs` if that ever changes.

### From the PR #63 reviews (2026-10-13), for later

- **A predictable staging name in a shared sticky directory** (Master). Another user can pre-create `.<name>.<pid>.bind` beside a socket in a shared sticky directory such as `/tmp`, which denies the start, just as a pre-created socket does today. Proposed: give the staging name a random suffix, or bind it in a private directory on the same filesystem.
- **A daemon killed between bind and link leaves its `.<name>.<pid>.bind` file** (Master; own review M3). Only a later daemon with the same pid removes it. Sweeping other daemons' staging names needs a liveness check first.
- **Hard links are now required in the socket directory** (Master; own review M4). A filesystem without them fails the bind with an error that names the path.
- **The staging name is a second name for the live socket for a few microseconds** (own review M2). The macOS sandbox's literal-path deny rule doesn't cover it.
- **`crates/daemon/src/lifecycle.rs` is at 598 lines.** The next change there should split the socket code out first.
- The long-path failure (a custom `ANTHREX_SOCKET` within about 14 bytes of `sun_path`'s limit) is **fixed in b5ca735b**: such a path binds directly at the real path.

## From milestone 9.15 (2026-10-13, recorded by M9.15.4 and its extra commits), for later

- **The dossier's redaction stops at the brief's scope** (M9.15.8). `lessons::redact` does not replace `Authorization: Basic <base64>` (or `token`/`digest` schemes), credential assignments such as `DATABASE_PASSWORD=…` or `"client_secret": "…"`, or a key body whose BEGIN and END markers were both cut away. Decide whether to widen it, and share it with PR-body redaction (9.2, NA §6).
  - *Also left (task 8's review, M2, M3, M5, M6; recorded by M9.15.9):* a URL password with an unencoded `/`, `?` or `#`, and a fully percent-encoded URL (`https%3A%2F%2Fuser%3Atok%40host`); an indented PEM body (YAML `key: |`) whose BEGIN was cut, and a token hard-wrapped across lines (its prefix-less remainder survives); `sk-`/`sk-ant-` after `-` or `_` of any length (the price of the `task-sk-ip` rule; *closed by the rulings after M9.15.13 and M9.15.14: every prefix now matches anywhere*); a token split by a combining mark or by a C0 control other than ESC (both survive in halves). Task 8's I1 (escapes) and I2 (an escape before a token) were fixed before M9.15.9, and I3 (signature headlines stored unredacted) right after it.
- **Look-alike dashes forge dossier markers** (task 8's review, M8). `dossier::break_markers` breaks runs of ASCII `-` only, so `—`, `‐` or `－` can draw a fake `——— END OF DOSSIER ———`. Outside decision 11's letter; belongs with the prompt-injection posture (decision 34).
- **Redaction is best-effort defence in depth, not a guarantee** (task 11's review, I1, recorded after the structural fix that followed M9.15.12; rules as of the controller's ruling after cc036413). `lessons::redact::scrub` narrows what reaches a provider prompt, `lessons.jsonl` and the Lessons page; it cannot prove that no secret does. The rules, each matching anywhere with no left boundary: the unambiguous prefixes (`gh?_`, `github_pat_`, `glpat-`, `sk-ant-`, `sk-proj-`, `sk-svcacct-`, `sk-admin-`, `xox?-`, `AKIA`) with a body of 4 or more characters of the prefix's alphabet (placeholders count; real escapes and hidden format characters are read through) holding an uppercase letter, a digit or a placeholder; `sk-` and `npm_` with a body of 20 or more holding a digit; `Bearer` and whitespace with a credential of 16 or more holding a digit. Known residual gaps: an ambiguous `sk-`, `npm_` or `Bearer` secret with no digit, or shorter than its minimum, including one a clip cut short or a placeholder split; an unambiguous token clipped to fewer than 4 body characters (or `AKIA` before lowercase), or to an all-lowercase body (`ghp_fake`; the price of keeping `laughs_count` a word); a NUL or other C0 control inside a token (one-lining spaces it, splitting the token into visible halves); `Bearer` with encoded whitespace (`Bearer%20…`, `Bearer+…`); percent encoding nested deeper than eight levels; a token hard-wrapped across lines or split by a combining mark; a token split by an escape spelling that `decode` does not decode (`ghp_abcd\e[31m…`, `ghp_abcd^[[31m…`: the body stops at `\` or `^`, so its remainder survives; task 15's review, M5); and the shapes outside decision 11's scope listed above. Accepted over-redaction: an identifier holding an unambiguous prefix and four more token characters, one of them an uppercase letter or a digit (`myghp_Count`, `task-proj-v2beta`); all-lowercase ones (`laughs_count`, `task-proj-name`, `desk-admin-panel`) are words since the ruling after cc036413. Because `decider/distill.rs::parse` redacts each file glob, a path holding such an identifier with a digit or capital (`crates/highs_2024/**`, `desk-admin-2FA1/`) loses its glob and the lesson matches no files (task 14's review, M1); decide whether globs should skip the clipped-token rule.
- **Daemon starts outside `TestDaemon` and `RunHarness` read no `[lessons]` default** (ruling C3, M9.15.4). `codex_version.rs`, `daemon_stop_stuck.rs`, `daemon_spawn_stderr.rs` and `daemon_start_lock.rs` start daemons through `isolated_command` with no config of their own (some read the user's), and run no goal; if one ever starts a run it should write `support::lessons::LESSONS_OFF` too.

### Recorded by M9.15.16 (2026-10-13), for later

- **PR-body redaction (milestone 9.2's delivery, NA §6).** 9.2's pull-request bodies carry task summaries and review text to a code host with no redaction. `lessons::redact::scrub` (M9.15.8) is the natural shared filter; decide whether PR bodies go through it, with its residual gaps (above) stated in the delivery docs.
- **The graph layout runs in the update function** (task 15's concern 1). Decision 44 places it in the `LessonsGraph` reply's update: about 60 ms in release (53–71 ms measured on the 300-node test graph, 359 ms in debug), once per graph reply, and decision 48 asks for the graph again after every action, so each action freezes input for that long once. Accepted for 9.15 (pinned to once per reply by `the_layout_runs_once_per_graph_reply`); a candidate to move to a worker thread whose result returns as an event.
- **Graph labels overlap on a dense canvas** (task 15's concern 2). The selected node is drawn last so its label stays whole, but hub and lesson labels can overwrite each other; a label-placement pass is not done.
- **The graph's render tests are property checks, not golden frames** (task 15's concern 3, a recorded deviation). A golden 80×24 and 160×48 frame would catch a regression the properties allow.
- **A fresh unmeasured lesson cleared `MIN_SCORE` with no file overlap** — *fixed* by the review's correction to decision 22 (M9.15 brief, "Review corrections"). A lesson with files is now selected only when one of its files intersects the task's `owns`; a lesson without files keeps decision 22's rule. Stage 11n now asserts that task t3 (`mods/beta/**`) is shown neither `mods/alpha` lesson.
- **A discarded run's merged tasks count green.** Decision 25 counts a `merged` task record green unless its merge was reverted; a task merged inside a run you then discarded never reached the base branch, yet it credits the lessons it was shown (the smoke stage's discarded run took both lessons to `2/2`). Decide whether a discarded run's task records count.
- **Entries queued while the deciders are off are never distilled** (ruling C29). The worker closes such an entry `gave_up` (`deciders are off`) at once, permanently, so turning the deciders on later does not revisit it. For 9.16 or later.
- **`run.json` holds up to 50 frozen lesson briefs** (ruling C30): about 50 KB in every save of a run with a full store. Not changed.
- **The graph panel's scroll bound is measured at the side panel's width** (task 15's review, M4). `panel_move` measures a lesson's lines with `ui::lessons_graph_panel::lesson_lines(…, PANEL_WIDTH − 2, PLAIN)` from app state: pure, but app state now depends on `ui`, and in the wider lower-half panel (80×24) the lesson has fewer lines, so `j` can push the row past the last drawn line; the render clamps it and the next few `k` presses seem to do nothing. Measure at the panel's drawn width (kept in state by the render's layout) or clamp on the drawn line count.
- **No linearity test exercises redaction's kept-body path** (task 15's review, M6). `the_unambiguous_body_check_is_linear` has no all-lowercase kept unit (`ghs_abcd `, or `ghs_` plus 20 lowercase letters), and `BODY_READ` does not count the scan in `unambiguous()`'s closure. The reviewer's timing probe found it linear (×4 input, ×3.7–×4.1 time); no test pins it.

### Recorded by the milestone 9.15 review (2026-10-13), for later

- **A redistilled lesson that a trial retired comes back `on_trial`** (task 3's review, M2). When a later distill call produces again a lesson that its trial retired, it comes back as `on_trial` instead of staying retired, so a lesson the evidence rejected returns to prompts for another trial. Decide whether a retired lesson's id (or content key) blocks its return, or needs the user's approval to come back.
- **Redaction residuals the review found** (in addition to the known gaps above; `lessons::redact::scrub` leaves each of these in place):
  - a JSON-escaped URL credential, `https:\/\/user:pass@host` (the `\/` spelling hides the `://`);
  - a percent-encoded URL, `https%3A%2F%2Fuser%3Atok%40host`;
  - clipped tokens whose surviving body is all lowercase or has no digit;
  - Stripe secret keys, `sk_live_…`: the `sk-` rules match a dash, not an underscore;
  - AWS secret access keys (40 characters of base64 with no prefix; only the `AKIA` access key id is matched);
  - `Authorization: Basic <base64>` credentials.
  Decide which to cover, and share the result with PR-body redaction (9.2, NA §6).
- **`lessons.jsonl` has no compaction.** Every `queued`, `attempt`, `distilled`, `gave_up`, outcome and status line is appended forever, and the start's read and fold grow with it. Once reading it takes longer than `LESSONS_START_BOUND` (5 s), every run starts with `lessons: none (the lessons store could not be read in time)`, silently. A compaction (fold, then rewrite the live state under the store's lock) or a rotation is needed before stores get that large.
- **The graph's profile fallback walks `module_dirs` with no time bound.** `run/driver/lessons_requests.rs`'s module source, with no module-graph cache, calls `profile::verify_tiers::module_dirs(checkout, &profile.modules)`, which lists the checkout's directories for the profile's module globs; it runs on the blocking pool but has no deadline, so a huge or slow (network) checkout holds the Graph request's reply. Bound it as the other filesystem walks are, falling back to `ModuleSource::None`.

### From the main-CI flakes on 0e87e0f1 (2026-10-13), for later

- **Other headless stand-ins may answer without reading stdin.** The lag rig's stand-in printed its turn and exited without reading its message, so a reap that beat the first send refused it (fixed in f24d5243). The stand-ins in `crates/daemon/tests/support/headless.rs` and the other headless rigs were not audited for the same defect.
- **No permanent seam keeps `daemon.log`'s non-blocking writer slow.** A future test that reads `daemon.log` once, right after an event, would pass locally and flake on CI. `RunHarness::wait_daemon_log` is the pattern; a source-rules check, or a debug-build seam on the appender, would catch a new one-shot read.

### From the rate-limit test's timeout on macOS CI (bac2aa9e, 2026-10-15), for later

- **Waits that can outlast nextest's 600 s fail silently and leak.** nextest terminates a test at 600 s with a signal: no unwinding, no `Drop`, so the harness's daemon (its own process group) and its sessions survive, and a wait whose deadline lies past 600 s never prints its trail. `run_e2e_tuning`'s rate-limit test now ends every wait by its `SCENARIO_WAIT` (400 s). Other tests still have waits that reach or sum past 600 s: `2 * RUN_WAIT` in the race, pair, patterns and safety tests, `THREE_GATES_WAIT` (900 s), `FINISH_WAIT` (900 s) and every scenario of sequential `RUN_WAIT`s. A harness-wide budget (each `wait_run` capped at a deadline counted from the harness's creation) would cover them all; it narrows many tests' waits, so it needs its own review.
- **`RunHarness::windows()` has no bound at all**, and `request_unanswered` and `RunWatcher::connect` wait for the `Welcome` outside their bounds (the watcher's 10 s ready guard aside). `request` now bounds its handshake (`request_within`); the others should too.
- **A profile `setup` or check still running when the daemon stops outlives it.** In a forced reproduction, a `setup` held in a poll loop kept running (parent pid 1) after `daemon stop` returned and the daemon exited, until its own loop ended. The daemon's shutdown ends session process groups, not in-flight `setup` and check commands. Process-ending code, so not changed here.

### From the ax-<uid> leak hunt (fix/ax-cache-leak-2, 2026-10-16)

- **`AX_TMP_TRACE` names each task temporary directory's creator.** It is test-only and off unless set (`crates/daemon/src/run/git/tmp_trace.rs`). When set, every process appends one line per creation to that file: the hash, the repository, the binary, `NEXTEST_TEST_NAME` and the pid. CI sets it for the suite, the smoke and the gate. `leak_gate.py check` prints the creator under each leaked `ax-<uid>/<hash>`. Locally: `AX_TMP_TRACE=/tmp/<yours>.tsv` on both the run and the check.
- **The CI leak's source is still unattributed.** It is `056b782e51c46167` on main 29afa208 (Linux). Instrumented local runs found no creator: six full suites (one with creation delays, one throttled below CI speed) and three smokes. A meet-in-the-middle inversion of the hash against every observed test and smoke path found no match either. That inversion could not cover `RunHarness` paths, whose temp directory and run id are both random. The leak stays unattributed until the tracer's first red CI run names its creator.
- **Two candidate causes are fixed in this branch, neither proven to be the CI leak's.**
  - `remove_checkout` left a listed checkout's tmp when `git worktree remove` or `prune` failed.
  - `daemon stop` returned when the lifetime lock was freed. That was before the 2 s `shutdown_timeout` grace, during which a blocking op can still create a tmp after "daemon stopped". `daemon stop` now waits for the exit lock instead.
- **Not changed: a daemon started while a previous one is still in that grace runs without the exit lock.** `hold_until_exit` returns false, and a stop of the new daemon then waits for its lifetime lock alone. It is logged to the new daemon's stderr.
- **`leak_gate.py selftest` can fail when other worktrees run tests concurrently.** Its final clean `check` sees their fresh entries. It is fine on CI, where nothing else runs.
- **#71's review minors (2026-10-16), for later.**
  - A duplicate `daemon start --foreground` that yields prints "an earlier daemon is still exiting" and briefly holds the exit lock.
  - A stop's poll can make a just-starting daemon run without the exit lock (it falls back to the old behaviour).
  - `daemon stop` now fails if exit takes more than 5 s after the lifetime lock is freed.
  - Stale "19 s" comments remain in `run_daemon.rs:27`, `scripts/pty-smoke.py` (about line 1638) and `scripts/pty_smoke_pr.py:73`; stop's worst case is now 24 s.
  - No test pins `exit_lock_path()`'s path.
  - The 500 ms `REAP_LIMIT` may flake under load.

## From milestone 9.16 (2026-10-15, recorded by M9.16.6), for later

- **Widen `run::reach` to the fallbacks of the `planner` and `research` rows and the document reviewer, so those sessions can fail over safely.** `reach.rs` collects every runtime a run may reach (workers' escalations, which include their rows' fallbacks, the orchestrator, the sub-planner and scout rows' own routes, the design agents), but not those three rows' fallbacks. Since task 6's review I1, `failover::candidate` takes a fallback only on a runtime in `reach::reachable_runtimes(run)`, the set `run start` checks for decision 50 (an API key) and decision 53 (project settings the CLI would load unasked, `--trust-project`); so in a run that reaches one runtime, a braked sub-planner, run scout or document reviewer whose row falls back to the other waits instead of failing over. Counting those fallbacks in `reach` would vet them at start (and widen every such run's start-time checks) and let them fail over. The pin to change then: `failover_readers::a_fallback_the_start_never_vetted_waits`.
- **A run scout's route is not stored on `RunScout`** (`run/orch/mod.rs` is at 599 lines, C8): a failed-over scout's route is read back from `Run.failovers`, which keeps the newest 32. Once M9.16.9 splits `orch/mod.rs`, a `RunScout.route` field would remove the lookup.

### Recorded by M9.16.11 (2026-10-16), for later

Each item was checked against the code at the M9.16.11 head; items the later tasks already closed (task 3's m3 rejection book entry, task 5's M2 stale test, task 6's M1 `launch_ready` test, task 7's M1 in-flight read, M3 brake-off harnesses and M4 `finish_late` test, task 10's I1 and m7) are left out.

- **`changed_since_loaded` should guard every settings-owned key at once, the limits and the brakes together** (task 4's review m1). `config/src/settings/save.rs::changed_since_loaded` compares only some of the keys the Settings screen owns, so a hand edit of a limit or a `[orchestrator.brakes]` key made while the daemon runs is silently overwritten by the next Settings save. One guard over every owned scalar, not a brakes-only patch.
- **Readings from project scouts and deciders** (the brief's "Follow-ups handled"; task 3's review m4). `decider/call.rs` drains a decider's or a profile scout's `Quota` and `ContextWindow` events and discards them, so only manager-fed sessions give a quota reading or teach a context window. Codex deciders and scouts run `--ephemeral`, so they have no rollout either.
- **The orchestrator does not learn of a hold or a braked runtime** (ruling C22). The brake lines are appended in snapshot assembly (`run_info`), not in `orch/digest.rs::attention`, so `run_status` shows the orchestrator the fill line but neither `dispatch is held daemon-wide` nor a quota or failover line; it may keep planning work that waits.
- **`anthrex hold` starts a daemon when none is running** (task 10's review m3). `hold_cmd::dispatch` calls `ensure_daemon`, as `lessons` and `profile` do, so `hold status` or `hold off` against a stopped daemon starts one (which restores and relaunches runs) instead of saying it is not running, as `run status` does; `hold on` then races restore's first dispatch. `status` could connect only.
- **`anthrex hold`'s refusal and timeout exits have no test** (task 10's review m2). A unit test of `request_of` over a `Refused` reply, or an e2e against a refusing daemon, would pin decision 34's "a refusal exits 1"; the timeout's bound is pinned, its exit is not.
- **Enter twice on the hold alert releases dispatch** (task 10's review m4): Enter opens the confirm and, the confirm not being destructive (decision 32), Enter confirms it. For the user to rule on.
- **A goal boundary during an ordinary mid-run read adopts and logs "fill unknown"** (task 10's review m5, task 9's I1). `fill_of` measures nothing while that window's live read is in flight; no goal-boundary test covers it.
- **`status::brakes_head` writes `"{}\n\n"` across two real line breaks** (task 10's review m1); an explicit `"{}\n\n"` is clearer.
- **`handoff_due(_, Some(0), p, _)` is `true` for any `p > 0`** (task 3's review m1; `transcript/fill.rs`). Unreachable today (every source filters a zero window); `window.filter(|w| *w > 0)` would make the pure function safe on its own.
- **A newer `rate_limits: null` / `info: null` line falls back to an older reading** (task 3's review m2): `codex_limits` and `codex_fill` take the last non-null line of the tail, which then gets the event's `now` as its time though it may be minutes older.
- **`reading_over_threshold_trips_until_resets_at` checks `verdict(1999)` after `verdict(2000)` dropped the trip** (task 4's review m2; `engine/tests/quota_book.rs:72-78`), so the 1999 assertion proves nothing about the boundary.
- **`QuotaBook::info` keeps a window's last `used` after its `resets_at`** (task 4's review m3), and `hold_cmd::quota_line` prints it as current (`7d 96%` after the week reset).
- **`pub mod quota_book` can return to crate-private** (task 4's review m5; `engine/mod.rs:107`).
- **`brake_holds_claude_dispatch_not_codex` asserts `race_decision == None` for a task that does not race** (task 5's review M3; `engine/tests/brakes.rs:113`): vacuous; `a_race_never_starts_a_braked_lane` is the real cover.
- **The unmeasured line can precede a launch that never happens** (task 5's review M5): `brakes::admits` logs `quota_blind_text` before a review task's `resolve`, which may fail. Cosmetic.
- **A test writer that fails over can land on the reviewer's model** (task 6's review M3, task 7's review M2). `reviewer_at_launch` skips test-writer rounds, so the tests' author's model can review them, at the failover or at a later re-pick (the worker's failover, rung 2, RL-1). Whether U17 covers the tests' author is for the controller.
- **Failover attention lines are not tied to the current trip** (task 6's review M4; `engine/failover.rs::attention`): a second trip hours later shows the first episode's lines again, merged tasks included.
- **A run scout's route lookup misroutes, not only misreports, once its record leaves the 32-entry ring** (task 6's review M5); the entry above on `RunScout.route` should say so.
- **The brainstormer hold tests have no hang guard** (task 6's review M2): a `start_next` that returned `true` on refusal would spin the test until nextest's stop.
- **A thread's search date is the daemon's first `Init`** (task 7's review M5): after a restart, a thread more than a day old is never found and stays unmeasured (fail open); the handoff's Codex fill shares the limit (C16).
- **`find_rollout` matches by suffix** (task 7's review M6; `transcript/rollout.rs:58`): an id that is a `-`-delimited tail of another would match that file. Parse the name instead.
- **`runtime_of` clones the whole `HeadlessSpec` under the manager lock for one field** (task 7's review M7; `driver/quota.rs:309`); a `headless_runtime(id)` accessor would copy less.
- **`reachable_runtimes` is recomputed per gated starter per pass while braked** (task 7's review M8); cache it per step if runs grow.
- **A window gone before its `Init` reached `observe` is cached as `None`** (task 7's review M9): one missed read at most.
- **`in_flight` is not cleared if the rollout reader panics** (task 8's review 2; `driver/quota.rs:224-225`): that window is never read again. A drop guard in the closure, as `handoff.rs`'s `InFlight` does.
- **The distill worker's route read has no timeout** (task 8's review 3; `lessons/service.rs:340`), against AGENTS.md rule 2.
- **`reviewer_at_launch_avoiding`'s last resort can return a route that failed in this task** (task 8's review 1; `run/model_roles.rs`), dropping RL-1's line; only one row layout reaches it.
- **The corrupt `brakes.json` is moved aside under a whole-second name** (task 8's review 5): two corrupt loads in one second overwrite the earlier copy.
- **Approving a triaged run under the hold launches its orchestrator** (task 8's review 6): as decision 19 specifies (`approve` stays allowed); for the user to confirm against U12.
- **The corrupt file's write-back can be lost, and the next start then fails open** (task 9's review 1): a move-aside rename that outlives its wait and then completes leaves no file and no write-back.
- **The save's wait after its claim is unbounded while the C10 mutex is held** (task 9's review 2), also inside `load`'s write-back.
- **A racing `hold off` can strand a profile staleness check until the next `hold off` or restart** (task 9's review 3; `service_restore.rs::auto_at_start`).
- **The handoff renames `chain.window_id`, not `run.handoff.from_window`** (task 9's review 4); a mismatch could fall back to the plain `Fresh` arm.
- **An unmeasured live check clears a shown fill line** (task 9's review 5): a transient read timeout after a 92 % reading removes the line until the next good read.
- **A Codex orchestrator is measured only through its `SessionStart` hook's `session_id`** (task 9's review 6): with Codex hooks off its fill is always unknown (fail open); decision 26 names the notify payload's `thread-id`.
- **`check_live` lists up to three date directories per running Codex orchestrator each minute** (task 9's review 8), with no cached path, unlike `quota.rs`.

### Recorded by PR #72's review (2026-10-16), for later

- **The handoff prompt carries the previous goal's summary into the clean session.** A handoff is `Join::Fresh`, so the taint is not carried (ruling C30, settled by NA §1.1's U4, "a fresh session starts clean"), but `handoff_prompt` gives the new session the same summary and history lines decision 24's fresh session receives, each line fenced as data (`contract_rounds.rs`). That is consistent with U4; if the summary of a tainted session should ever count as tainted input, this is where it enters.
- **Codex readings are throttled to one rollout read per 60 s daemon-wide** (`driver/quota.rs::ROLLOUT_READ_EVERY`). A Codex trip can be seen up to a minute (plus `QUOTA_READ_WAIT`) after the turn that crossed it, and with several Codex windows ending turns, a given window's reading can be skipped; a dispatch in that gap starts unbraked.
- **A local writer of `~/.codex/sessions` (the daemon's `CODEX_HOME`) can plant a reading.** Anything that can write a rollout file the daemon reads can trip the Codex brake or hold it off a trip. The effect is bounded by the window: a planted trip's `until` is clamped to `read_at + the window's length` (task 3's review I2), and a newer real reading below the threshold releases it.
- **Every Claude `rate_limit_event` bumps the engine's revision.** `EventKind::Quota` updates the book's `read_at`, which is part of the snapshot's `brakes` (`brakes::info`), so `brakes_moved` (`engine/mod.rs`, decision 14) is true and the step publishes a structural change even when the usage and the trips are unchanged: one snapshot push to every client per event. Comparing the shown info without `read_at` (or rounding it) would let an unchanged reading skip the bump.

## From milestone 9.17 (2026-10-16, recorded from the reviews of tasks 4 to 10 and their fix rounds), for later

- **The per-task report lists brief findings under Critical with no mark** (task 4's review M3; `report_task::findings_by_severity`, `report_task.rs:296`): a brief finding reads like a code defect there, with its `file` if the reviewer gave one; the `(brief, from the reviewer)` note line above it is the only clue. Mark it `brief`, as `finding_line` does.
- **The `brief finding: ` check on a task entry's `last` rarely fires** (task 4's review M5; `orch/untrusted.rs::note_source`): `review::submit` logs `review round N approved` / `asked for changes` (or `reported: …`) straight after `review_brief::record`, so `last` is almost never the brief line. Harmless; nothing should rely on it as the taint (the digest's `task_notes` arm is the one that holds).
- **The TUI notes row credits a brief note to the task** (task 4's review M6): it reads `02:45 brief from t1: review round 1 …`, the "from" ambiguity C12 fixed in the report; it should say the reviewer wrote it.

- **An accepted post-approval submit silently replaces the approved run frame** (task 5's review M4; `engine/orch.rs::edit_plan`'s `store_run` call). Decision 18 replaces a round's frame on every accepted submit carrying the keys, and orchestrator rule 6 (M9.17.9) asks for them on every submitting call, so a mid-run epic-round submit on a running run usually rewrites the frame the user approved. Reviewers then get the new `out_of_scope`, but decisions 19 and 20 show an epic hold only its epics' frames, so the user never sees the replacement at a gate. A possible ruling: store a run frame only when the submit acts or the run awaits approval.

- **A long epic label makes the frame detail wrap one character per row at very narrow widths** (task 6's review m2; `tui/src/app/plan_frame.rs:143-144`, `ui/kit/rows.rs::labelled_rows`): `labelled_rows` does not bound its label column, and an epic id may have 11 characters (`orch/tools.rs:161`), so `abcdefghijk out of scope` makes the label 26 columns wide and, in a detail narrower than about 28 columns, every character of every item wraps onto its own row. This predates milestone 9.17; cap the label column, or wrap the label too.
- **`replaces` writes into an earlier round's task** (task 7's review M3; `run/validate_rounds.rs::earlier_round`, `:36-70`): in round 2, `add_task x replaces ["r1task"]` with `r1task` cancelled in round 1 adds the history line `replaced by x` to a task of a round decision 15 makes read-only, because `earlier_round` ignores `replaces`. Only history changes, and the link is what decision 26's statistics want; rule on whether `replaces` may name an earlier round's task.
- **A replace whose cancel is deferred by an in-flight merge still stores the link** (task 7's review M4; `run/edits.rs::cancel_logged`, `:361-376`): the blocked task stays blocked (`cancel deferred: its merge is in flight`) while the new task already has `replaces`, so the report shows `blocked (replaced by x)`, and the later cancel logs `cancelled by a plan edit`, not decision 24's `cancelled: replaced by x`. It matches `cancel_task`'s deferral and needs a merge candidate in flight on a blocked task.
- **`replaces` skips the "epic being planned" rule** (task 7's implementer concern; rule `2.epic`, `epic_being_planned`): `split_task` refuses a task of an epic whose sub-planner is still planning, but `add_task … replaces` does not, so the orchestrator can replace a cancelled or blocked task of such an epic. This is parity with `cancel_task`, so it is consistent today, but asymmetric with `split_task`; decide whether `replaces` should check it.
- **At 80 columns the design gate's status bar drops `f frame`** (task 8's concern; `tui/src/ui/statusbar_modes.rs::design_review_hints`): the hint shares priority 5 with `e edit`, `d drop` and `b back`, and all four go together at 80 columns (`plan_review_frame_tests.rs::design_gate_hints_f_frame_on_the_task_list_only` uses 120 and 160). A UX call: whether `f` should outrank them.
- **Re-pointing a replaced task's dependents at its replacement** (milestone 9.17's Out table, decision 24): a blocked task cancelled by `replaces` leaves its dependents `blocked(dep_cancelled)`; today the orchestrator re-points them with `amend_task deps` (rule 26). An automatic re-point would save that edit.
- **The plan frame in the orchestrator's digest and in 9.16's handoff prompt** (milestone 9.17's Out table): `run_status` and `handoff_prompt` carry neither `decided` nor `out_of_scope`, so a handed-off or long-running orchestrator relies on its own context to remember what it told the user.
- **Hold a task on a critical `brief` finding, if reviewers misuse the target** (milestone 9.17's Risks and "Follow-ups handled"): decision 7 lets a task merge whatever its `brief` findings say. If the manual check shows real reviewers hiding code defects behind `target: brief`, hold the task for the orchestrator on a critical one; that changes decision 7.
- **Orchestrator rule 6 promises `out_of_scope` to every reviewer, but only task reviewers get it** (task 9's review M2; `run/orch/contract.rs:50`, the brief's verbatim text): only `reviewer_prompt` (`run/contract.rs:294`) includes `out_of_scope_block`; the prompts `engine/kinds.rs:186-197` chooses for review tasks and integration reviews (`review_task_prompt`, `orch/contract.rs:426`, and `integration_review_prompt`) carry no out-of-scope block. An orchestrator that lists "no schema migration" may then see an integration reviewer ask for one and add fix tasks under rule 27, believing the reviewer was told. Decisions 2–4 only named `reviewer_prompt`, so this is a brief defect: extend the block to those prompts, or narrow the rule's text.
- **Rule 6's design-flow sentence omits decision 16's exception** (task 9's review M3; `run/orch/contract.rs:50`, the brief's verbatim text): "In a design-flow run … both may be left out", but by decision 16 and ruling C6 a design run's round with the flow off goes 9.5's way and is refused with `PLAN_FRAME_MISSING` without both keys. It costs one refused submit and a retry, so it corrects itself; the sentence could add "unless the round's design flow is off".
- **A brief-only changes round is logged as "approved"** (PR #73's review; `run/review.rs:383`). The run log line for a review round whose only findings are `brief` says `approved`, though the reviewer asked for changes to the brief. Log it as `approved (brief findings only)` or the like.
- **A merge-in-flight cancel deferral could skip "replaced by" and hide a merge** (PR #73's review). 9.18's pre-flight traced it as unreachable on `636f0a40` (see M9.18's Pre-flight rulings). If a Blocked task can ever hold a `MergeCandidate`, `candidate_done` leaves it Blocked, a restart cancels it despite its merge, and stats hide it as replaced. A `candidate_in_flight` guard in `edits_replaces.rs` and `split` is the user's call.
- **An ignored submit on a running run can silently replace the round's frame** (PR #73's review; C6, decision 18). A submit the engine ignores on a running run still stores the keys as the round's frame. This is the same family as task 5's review M4 above.
- **`j` from the frame view skips one task** (PR #73's review). In the plan review, with the detail on the frame, `j` moves the selection by two. Pin it with a test from the frame view.
- **An `edit_plan` tool description line is over width** (PR #73's review). One line of `edit_plan`'s MCP description runs past the width its neighbours wrap at.
- **Minor brief findings wake the orchestrator** (PR #73's review). A `brief` finding of Minor severity queues an orchestrator wake-up like a Critical one. Consider batching Minor brief notes into the next digest.

## From milestone 9.18 (2026-10-16), for later

- **A crashed compaction's temporary file is never swept** (M9.18.3). `lessons::store::compact` rewrites `lessons.jsonl` with `profile::store::write_atomic`. A crash between creating the temporary file and the rename leaves `lessons.jsonl.<pid>.<n>.tmp` in the repository's data directory. The start sweeps (`sweep_leftovers`, the run restore's `sweep_named`) do not name `LESSONS_FILE`, so it stays. It is harmless but grows by one store's size per crash. The fix is to add `LESSONS_FILE` to a sweep that runs before the distill worker starts.
- **An encoded or JSON-escaped URL credential cut before its at-sign is not redacted** (M9.18.4's review, M1). `lessons/redact_more.rs::encoded_userinfo` needs the authority's `@` or `%40`, so `https%3A%2F%2Fuser%3Apass` at a clip's end, the same cut inside the `%40` (`…%3Apass%4`), and the JSON-escaped `https:\/\/user:pass` at a clip's end all stay. Decision 6 named only the `URL_USERINFO` form. The fix is `URL_CUT`'s rule (a password to the line's end, a bare port kept) for both spellings.
- **More spellings that leak past the lessons redactor** (M9.18.4's review, M2). A JSON `\/` for `/` (`https:\/\/admin:hunter2@host` is caught, but `\/` kept inside a secret, `wJalr…\/K7…`, splits the AWS run); mixed `https:%2F%2Fuser:pass@host` and `https%3A//user:pass@host`; double-escaped `https:\\\/\\\/…`; and a space or `=>` as the AWS separator: `aws configure set aws_secret_access_key <secret>` (the likeliest in an agent's session), `--aws-secret-access-key <secret>`, and `"secret_access_key" => "<secret>"`.
- **`scrub` is not idempotent on some inputs, before 9.18 too** (M9.18.4's review, M3). A fuzz found 19 inputs at 9.18.4's head (20 at its base) where `scrub(scrub(x)) != scrub(x)`. `strip_escapes` leaves a `%` beside hex digits, and the next `decode` reads it as `%9b` (the single-byte CSI) or `%0A`, which eats or adds characters. Example (the review's, with AWS's example key id written as `<akia-id>` here): `%\u001bZ9basic\t<akia-id>%2f` scrubs to `%9basic\t[redacted token]%2f`, and that to `sic\t[redacted token]%2f`. Nothing leaks, but `redact.rs`'s "idempotent on what it returns" is false. One fix is to make the strip leave no `%` that a later `decode` can join to the hex digits after the cut.
- **A clock that goes back can free the daily distill cap early** (M9.18.3's review, M5). `lessons::store::compact(path, now)` drops a closed entry's `attempt` once `now - at >= DAY_SECS`, with `serve`'s start time. If the wall clock then steps back by more than that slack, `capped` at the earlier time would have counted attempts the compaction already dropped, so the worker may call the decider more than `max_distills_per_day` times that day. Same class as ruling C20's clock note; it needs a wall-clock regression to reach.
- **A large store is re-read on every `serve` call** (M9.18.3's review, M7). A `lessons.jsonl` over `LESSONS_COMPACT_BYTES` that stays under the quarter rule (mostly `queued` dossiers) is read and parsed in full under the repository's `LessonLocks` entry on every `serve`, and `read` reads it again. It stalls that repository's `QueueDistill`, `Decide` and `Forget` writers for that long (no tokio worker blocks). Decision 4 as designed; if stores reach tens of MiB, memoise the last skip by size and mtime.
