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
| M7 Split panes | SIGWINCH jiggle on attach so full-screen apps repaint. Wheel scrolling for alternate-screen apps without mouse mode. Check `mouse_protocol_encoding()` instead of assuming SGR. Define terminal-input policy for embedded bracketed-paste end markers before crossterm splits the intended clipboard boundary. |
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
  takes the `TreeState::overview` follow-up.
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

- **A revert of a revert does not clear the reverted count.** If the user reverts a task
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
- **The output filter for Codex workers (M9.5).** Decision 28 wraps Claude workers' Bash through the PreToolUse hook; Codex has no equivalent hook yet.
- **Pruning `history.jsonl`.** It only grows; readers keep the last line per `record_id`.
- **Routing and threshold proposals in `run stats` (M9.5).** Stats only reports today.
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

- **The run inspector's `doing` field has no tool target** (brief Risks 6, for M9 or a
  later snapshot change). The spec's mockup shows `editing crates/daemon/src/status.rs
  (last tool: apply_patch)`; neither `AgentRoundInfo` nor `WindowInfo` carries a tool's
  target, only its name. The view does not read M6.5's conversation state for it, which
  would make the inspector depend on a subscription the user may not have open. Publishing
  the last tool call's target (a path, or a short argument) on the round or the window
  would fill it.
- **Fields of "Consumes from later milestones" still unfilled** (the view renders their
  absence as the brief states):
  - `RunInfo.estimate_left_secs` and `RunInfo.bound_ratio_permille`: **M9.5** (history
    medians). Until then the view shows no `est. left` and no `× the bound`.
  - `RunInfo.planners` (`PlannerInfo`, `PlannerState`): **M9**. Until then every task
    hangs from the run's root.
  - The orchestrator's PTY window (`WindowInfo.run == Some(RunRef { role: Orchestrator,
    task_id: None, .. })`) and `AgentRole::Planner`: **M9**. Until then the root reads
    `run <h4>` and Enter on it toasts that the run has no orchestrator window.
  - `RunInfo.scouts` for a run's area scouts: **M9** (M8b declared the type and fills
    only the onboarding scout, which has no run).
  - Racer and test-writer rounds: **M9.5**. Not drawn until then.
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

## From M9.10 (2026-09-28), for M9.15

- **The TUI still refuses `C-b x` and `C-b X` for a placeholder headless window.** Milestone 9 decision 11a restores an unparseable headless record as an exited headless window with no run, which the daemon lets a client kill and remove. `WindowInfo` does not tell it apart from a repository-level scout's window (both are `Headless` with `run: None`), so `app/headless.rs::headless_control_refusal` toasts the scout refusal and the user cannot remove it from the TUI; `anthrex rm <id>` works. A fix needs the snapshot to say so (a protocol field) or the daemon to answer the TUI's refusal case differently. *M9.15 (2026-09-29): still open; the brief's task text does not assign it and it needs that daemon change. The TUI offers no control the daemon refuses; it only over-refuses this one window, which `anthrex rm <id>` removes.*

## From M9.13's review (2026-09-29), for M9

- **`run edit`'s and `edit_plan`'s project-settings refusal ignores the run's `--trust-project`.** `RunService::runtime_refusals` (`crates/daemon/src/run/driver/build.rs`) passes only the files the start recorded in `Run.trusted_project`, and its text (`edit_settings_refusal`) tells a user who did start with `--trust-project` to start a new run with it. M9.13's review fix persisted `Run.trust_project` and made `run promote` honour it; the edit path could read the same field. Left alone because it is M8a's ruling T22-I1b and the review named only promotion.
  - **And the promotion does not record what it trusted (M9.13 re-review).** A `run promote` let through by `trust_project` does not add the project-settings files its new orchestrator and sub-planners reach to `Run.trusted_project` (the check runs in the driver, before the engine's `Promote` event, which does not carry them). A later `run edit` that reaches the same runtime is then refused for files the promotion already accepted. Honouring `trust_project` in `runtime_refusals` (above) closes both; so would carrying the files on the `Promote` event.

## From M9.13a's re-review (2026-09-29), for M9

- **A run head rewound below a refresh's target still shows the refreshed run files in the done check's spill diff and in `diff_so_far`.** `git::verify_done` computes the spill as `git diff --name-only <run_head>...<head>`, and `diff_so_far` as `<run_head>...HEAD`. After `resume --rebaseline` rewinds the run head below a run head that a refresh merged into the task, the merge base drops back, so the merged run files appear there: a spill outside `owns`, and more of the diff for the reviewer. The commit counts and `task_result` already leave that run work out, since the M9.13a re-review records every refresh's target (`TaskOrch.refresh_targets`). The same targets could serve as the diff base here: the newest one the head has. This was left alone because the re-review named only the counts and `task_result`, and the spill check is M8a's gate.

## From milestone 9 (2026-09-29)

- **Unconfirmed: a `run_status` read can leave one stale wake-up to be pasted later (M9.17 review).** `driver/orch.rs:333-338`'s `run_status` calls `Wakes::read`, then sends `DigestRead` asynchronously; a read that lands between a step's commit and that step's `queue_wake` may leave its wake-up waiting, pasted later for notes the read already showed, at the cost of one redundant orchestrator turn. Not reproduced.
