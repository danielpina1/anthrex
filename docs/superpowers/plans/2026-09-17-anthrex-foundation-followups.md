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
  and restructures the layout, so it splits the file there.
- **The painter's zip invariant is only `debug_assert`ed.** `paint` pairs `layout.nodes`
  with the row list positionally. The invariant holds by construction today; in release a
  future divergence would paint labels onto the wrong boxes rather than dropping a node.

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
  which edit kinds, and which tasks, a sub-planner may use.
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
  object's single text field (`output`, `error`, `stdout`) instead.

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
- **An interrupted turn stays `Running` in the conversation** (M8a.7 review M3, for M8a.18
  and M8c). An interrupted Codex turn ends with `ProcessExited` and no `turn.*` line.
  Claude fires no `Stop` hook for an interrupted turn, and with `hooks_fire` the
  synthesised `Stop` is dropped. The Assistant turn therefore stays `Running` with its
  call `Pending`, and the next prompt closes that call as `Denied`. The daemon knows the
  turn ended. `TurnEnded { Interrupted }`, and a Codex `ProcessExited` while a sent turn is
  open, could synthesise `Stop` even when `hooks_fire` is true.
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
