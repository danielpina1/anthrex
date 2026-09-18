# anthrex Foundation: follow-ups carried into plans 2 to 5

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
