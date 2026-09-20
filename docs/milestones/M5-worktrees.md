# Milestone 5: New-agent dialog and git worktrees

Refreshed on 2026-09-20 against milestones 2, 3, 4 and 4.5 as merged on `main`, from the audit in `.superpowers/m5-brief-audit.md`.

## Header

| | |
|--|--|
| Status | `ready` |
| Depends on | Milestone 4.5 |
| Spec sections | `docs/superpowers/specs/2026-09-20-git-surface-and-simple-orchestration-design.md` — the top layer, which wins where it disagrees with anything below: §2 (worktree root against project root), §3.3 (watching and the root's lifecycle), §3.4 (window metadata), §3.5 (protocol), §3.8 (failure modes) and §9 (risks). Product spec 6 items 1, 7 and 8 (layout under the data directory, kept compatible with milestone 8's run worktrees; create and remove rules). Core spec 3.5 (worktrees) and 6.3 (new-agent dialog; remove confirm with worktree checkbox and force follow-up). |
| Branch | `m5-worktrees` |
| Protocol version | Raised from 4 to 5. No message changes shape; see decision 24 for why the number moves anyway. |

## Starting point

Milestones 2, 3, 4 and 4.5 are merged before this one starts. This brief uses the names on `main` at commit `2d3ccf8`:

- Hooks: `anthrex hook`, `ClientMsg::HookEvent` handled, the Claude and Codex launch flags in `crates/daemon/src/launch/`, process-group kill. `WindowManager::remove` sends SIGKILL to the child's process group at once (M3 decision 47).
- `WindowManager::new(config: ManagerConfig)`, where `ManagerConfig` holds exactly `socket_path`, `shell`, `exe`, `claude_bin`, `codex_bin` and `codex_hook_source`, built by `new`, `from_vars` and `from_env`.
- `WindowInfo` carries `id`, `name`, `runtime`, `cwd`, `project`, `worktree: Option<PathBuf>` (new in milestone 4.5, between `project` and `branch`), `branch`, `status`, `tool`, `since_secs`, `last_output_secs`, `session_id`, `model`, `subagents` and `exit`. Every literal `WindowInfo` construction in the workspace already has the `worktree` field.
- `proto::PROTO_VERSION == 4`, pinned by `proto::tests::proto_version_is_four`. New types `Head`, `GitOperation` and `GitState`, and `DaemonMsg::Git { root, state }`, which `App::on_daemon` already handles.
- `crates/daemon/src/project.rs`: `DetectedRoots { project: PathBuf, worktree: Option<PathBuf> }`, `detect_roots(cwd)`, `detect_roots_with(git, cwd, timeout)`, `resolve_roots(cwd)`, `resolve_roots_with(git, cwd, timeout)`, `DETECT_TIMEOUT = 5 s`. It never fails: it falls back to the canonical `cwd` with `worktree: None`.
- `crates/daemon/src/subprocess.rs`: `run(command, max_output_bytes, timeout) -> Outcome`, the hardened spawn (scrubbed git environment, null stdin and stderr, piped stdout drained non-blockingly, own process group, child always terminated with `killpg(pid, SIGKILL)` and reaped). `Outcome` is `Failed | Complete | TimedOut | Truncated`. Two callers: `project` and `git::probe`.
- `crates/daemon/src/git/` is a directory module: `mod.rs` (`GitRegistry`, reference-counted per root, `enabled_from_env`), `parse.rs`, `probe.rs` (`PROBE_TIMEOUT = 5 s`, `resolve_git_dir`), `schedule.rs`, `watch.rs`. `ANTHREX_GIT=off` disables the subsystem.
- `server::serve(listener, manager, git_enabled: bool, shutdown)`. Its `CreateWindow` arm runs in a spawned task: it awaits `project::resolve_roots(spec.cwd)`, calls `manager.create(spec, roots.project, roots.worktree, cols, rows)` inside `tokio::task::spawn_blocking`, and on success calls `git_registry.register(info.worktree)` — after `create` has returned, never inside the closure (AGENTS.md hard rule 10). Its `Remove` arm captures the removed window's `worktree` from `manager.list()`, calls `manager.remove`, then `git_registry.unregister(&root)` unconditionally.
- `WindowManager::create(spec, project, worktree, cols, rows)` is synchronous, takes the manager lock for its whole body, and rejects `spec.worktree_branch.is_some()` with `daemon::WORKTREE_UNSUPPORTED`.
- The project tree in `crates/tui/src/tree.rs` with its submodule `tree/rows.rs`. `Row { key, guides: String, kind }`: `guides` is the exact box-drawing prefix, two columns per level, and both renderers (`ui/tree_view.rs`, `narrow_line` and `wide_line`) emit it before anything else. Tree mode (`C-b t`) in `crates/tui/src/tree_input.rs`, the overview (`C-b T`). `App`'s tests live in `crates/tui/src/app_tests.rs` with the submodules `app_tests/{git,overview,tree_interaction,tree_mode}.rs`.
- The daemon's integration tests share `crates/daemon/tests/support/mod.rs`: `start_daemon()`, `start_daemon_with_git(git_enabled)`, a `Client` that speaks the wire protocol, `shell_spec`, and a `git(dir, args)` helper that already pins `user.name`, `user.email`, `commit.gpgsign=false`, `init.defaultBranch=main` and `GIT_CONFIG_NOSYSTEM=1`.
- `crates/cli/src/client.rs` already has `request_with_timeout`, `REQUEST_TIMEOUT = 5 s`, `CREATE_REPLY_ALLOWANCE = 2 s` and `CREATE_WINDOW_REPLY_TIMEOUT = project::DETECT_TIMEOUT + CREATE_REPLY_ALLOWANCE`, pinned by `create_timeout_adds_allowance_without_changing_the_default`. `crates/cli/src/main.rs` already parses `new --worktree <branch>` and `rm <target> --worktree --force` and wires them into `WindowSpec.worktree_branch` and `ClientMsg::Remove`; only the rejection, the help text and clap's `requires` are missing.
- `crates/fake-agent` and the `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` overrides.

If the merged code differs from the names above, use the real names and record the mapping in "Implementation notes".

## Goal

`C-b c` opens a form to create an agent: runtime, name, directory, an optional git worktree on a branch, model and first prompt. With the worktree ticked, the daemon creates a linked worktree under the data directory and starts the agent inside it, so parallel agents never edit the same checkout. The tree shows the branch next to worktree windows, grouped under their repository's project, and the bottom bar shows that worktree's own git state, not the parent repository's. Removing a window offers to remove its worktree too. If git refuses because the tree has changes, the user chooses to force, to keep the worktree, or to cancel. The CLI does the same with `anthrex new --worktree <branch>` and `anthrex rm --worktree [--force]`.

## Scope

In:

- Stderr capture in `crates/daemon/src/subprocess.rs`, so a failed git command can say why it failed.
- A `worktree` module that creates, checks and removes worktrees, on top of that one runner.
- `WindowManager::create` runs every blocking step (git, directory checks, `Window::spawn`) off the manager lock.
- Worktree cleanup when window creation fails after the worktree was made.
- Removal of a window's worktree on request, with dirty-tree detection and a force path.
- Keeping milestone 4.5's git registry correct as worktrees appear and disappear: the new checkout is what gets watched, and it stops being watched exactly when it is deleted.
- The new-agent form as a pure model in the client, its rendering, and routing of keys and pastes to it.
- The remove-confirm dialog with the "also remove worktree" checkbox, and the force follow-up.
- The branch on worktree windows in the tree, the main title and `anthrex ls`.
- CLI: `new --worktree`, `rm --worktree [--force]`.
- Smoke-script stages for both paths.

Out:

- Persisting worktrees across daemon restarts, and restarting a window in its existing worktree. That is milestone 6. See risk 7.
- Run worktrees (`anthrex/<slug>/...`, `<wt>/runs/...`). That is milestone 8. This milestone only keeps the layout compatible.
- Deleting branches on removal. The branch is always kept.
- Rename (`C-b ,`) and the config file (`default_runtime`). Milestone 6.
- Tracking remote branches. A branch that exists only on a remote gets a new local branch from `HEAD`. See risk 5.
- Any change to how milestone 4.5 probes, watches or publishes git state. This milestone changes which root is registered and when, never the machinery behind it.
- The inotify descriptor cost of one recursive watch per agent worktree. See risk 11.

## Design decisions

These are final. If one proves wrong, stop on it and record the evidence under "Implementation notes"; do not quietly choose differently.

### Git and worktree layout

1. **There is no new git runner.** Every git command this milestone runs goes through `crate::subprocess::run`, which milestone 4.5 already built and which two callers already share: the scrubbed environment, the null stdin, the non-blocking stdout drain under a byte cap, the deadline, and the child that is always terminated with `killpg(pid, SIGKILL)` and reaped. Its one real gap is stderr, which it discards, and `Outcome::Failed` which merges a spawn failure with a non-zero exit. Git's stderr is the only place that says *why* a worktree could not be created, so `subprocess` gains `run_captured` (see the Interfaces block below), and `worktree` is its third caller. `run` itself, and its two existing callers, keep their exact behaviour. Every function in `worktree` takes the git program as its first parameter, `git: &OsStr`, exactly as `git::probe::probe` does, so tests can hand it a recording or a slow script; the manager passes `OsStr::new("git")`. The command is built with `LC_ALL=C` and `GIT_TERMINAL_PROMPT=0` set, so messages are stable and git never blocks on a credential prompt.
2. **Every git invocation this milestone makes passes `--no-optional-locks` and the scrubbed environment** (AGENTS.md hard rules 10 and 11), written immediately after `-C <dir>` and before the subcommand, as `git::probe` writes it. It is uniform rather than conditional so there is no list to remember and no reviewer judgement to make: it is load-bearing on the dirty check of decision 12, which runs `git status` inside a worktree with a live agent in it — the exact scenario rule 11 names — and harmless on `worktree add`, `worktree remove`, `worktree prune` and `branch -D`, which take their locks unconditionally. The scrubbed environment comes from `subprocess::run` itself. Milestone 4's `project::detect_roots` is not changed here: its `rev-parse` takes no optional lock, and it already runs through the same hardened runner.
3. **Deadline.** Each worktree operation (one create, one removal) gets one deadline, `Instant::now() + worktree::OPERATION_TIMEOUT`, with `OPERATION_TIMEOUT = 30 s`, shared by all the git commands it runs. `subprocess::run` takes a timeout rather than a deadline, so `worktree`'s private helper converts: the remaining time at each call, and a deadline already passed fails as a timeout without spawning anything. A command still running at the deadline is killed with `SIGKILL` to its process group by `subprocess::run`, which also kills git hooks, and returns `WorktreeError::TimedOut`.
4. **Off the async runtime.** Everything in `worktree` is a blocking function. The manager calls it only inside `tokio::task::spawn_blocking`. No git runs on a tokio worker or while `WindowManager`'s `inner` mutex is held.
5. **Roots come from `project::detect_roots`, not from a second implementation.** `worktree::create` resolves the user's directory with `project::detect_roots(dir)`, which already asks git once for `--git-common-dir` and `--show-toplevel`, canonicalizes both, handles the linked-worktree and submodule cases, and never fails. `roots.worktree.is_none()` means "not a git working tree", which also covers a bare repository, and fails with `WorktreeError::NotARepo`. `roots.project` is the main checkout and becomes `ManagedWorktree.repo_root`. The vocabulary is the amendment's (§2): **project root** for the main checkout that every linked worktree of a repository shares, **worktree root** for a single checkout's toplevel. This brief never says "repository root". Detection carries its own `DETECT_TIMEOUT`, outside the operation deadline of decision 3, so a create's worst case is one detection timeout plus the operation deadline.
6. **Repository worktree directory.** `<wt> = <data_dir>/worktrees/<project_basename>-<hash8>`. `project_basename` is the project root's last path component with every character outside `[A-Za-z0-9._-]` replaced by `-`. `hash8` is the 32-bit FNV-1a hash of the project root path's bytes (`OsStr::as_bytes`), written as 8 lowercase hex digits. Test vectors: `""` gives `811c9dc5`, `"a"` gives `e40c292c`, `"/tmp/repo"` gives `a3cbe2c8`. Milestone 8 reuses `<wt>` unchanged. Never use `std::hash::DefaultHasher`; its output is not stable across Rust releases. The daemon already depends on `sha2`, and this is deliberately not it: the name has to be short, stable across versions and readable in a path, which a truncated SHA-256 is not any better at and which no dependency is needed for. Do not "simplify" it later.
7. **Worktree path.** `<wt>/<branch with every "/" replaced by "-">`. The resulting directory name `runs` is rejected because milestone 8 owns `<wt>/runs/`. If the path already exists, creation fails with `worktree path already exists: <path>`. This also catches `feat/x` against an existing `feat-x`.
8. **Branch rules**, checked by the daemon in this order, each with the exact message in the Interfaces section: not empty after trimming; no whitespace; does not start with `-`; does not start with `anthrex/` (reserved for runs); `git check-ref-format --branch <branch>` exits 0 and prints the branch unchanged. The client checks the first four itself for fast feedback.
9. **Branch in use.** Before `worktree add`, the daemon reads `git worktree list --porcelain`. If any entry has `branch refs/heads/<branch>`, creation fails with `branch '<branch>' is already checked out at <path>`. This covers the branch checked out in the main checkout. Verified against git 2.50.1: `git worktree add` refuses the same case with `'<branch>' is already used by worktree at '<path>'`, but the wording differs between git versions, so the daemon checks first.
10. **Create.** `worktree::create` checks in this order and stops at the first failure: branch syntax (decision 8, rules 1 to 4), the roots (decision 5), `check-ref-format`, the reserved `runs` directory name, branch in use (decision 9), path exists (decision 7). Then, if `git show-ref --verify --quiet refs/heads/<branch>` exits 0, run `git worktree add <path> <branch>`. Otherwise run `git worktree add -b <branch> <path>`, which starts the branch at `HEAD` of the directory the user chose, and remember `created_branch = true`. All of these run with `-C <the user's directory>`. `create_dir_all(<wt>)` runs first.
11. **Window directory.** The window's child runs in the worktree root, whatever subdirectory of the repository the user chose. The entry's `spec.cwd` is replaced by the worktree path before `launch::plan`, so `WindowInfo.cwd`, the PTY's cwd and Codex's `-C` all name the worktree.
12. **Dirty check.** `worktree::is_dirty(git, path, deadline)` runs `git status --porcelain --ignore-submodules=none` in the worktree and returns true when stdout is not empty. Verified against git 2.50.1: untracked files make it dirty and make a plain `git worktree remove` fail with `contains modified or untracked files, use --force to delete it`. Ignored files do not count and are deleted by a plain remove.
13. **Remove.** `worktree::remove(git, wt, force, deadline)`: if `wt.path` does not exist, run `git worktree prune` in `wt.repo_root` and succeed. Otherwise run `git worktree remove [--force] <path>` in `wt.repo_root`. When that fails without `--force` and `is_dirty` now returns true, return `WorktreeError::Dirty`; any other failure returns `WorktreeError::Git` with git's stderr. The branch is never deleted.
14. **Error text.** Git failures carry the last 20 lines of stderr, trimmed, at most 1000 characters, with `fatal: ` prefixes kept. The client shows the message as given.

### Creating windows

15. **Three phases.** `WindowManager::create` becomes `async`:
    - Phase A, under the lock: resolve the name (unchanged rule: trimmed, else `<runtime>-<id>`), reject it if an entry or a reservation already has it, allocate the id (`next_id += 1` now; an id lost to a failed create is not reused), insert the name into `reserved_names`, release the lock. A `Reservation` guard removes the name again in `Drop`, on every exit path.
    - Phase B, in one `spawn_blocking` call, no lock: canonicalize and check the directory; if a branch was given, `worktree::create`; build the `LaunchPlan`; `Window::spawn`.
    - Phase C, under the lock: insert the entry with its `managed` worktree and its `worktree` root (decision 21), drop the reservation, publish.

    The `project` and `worktree` parameters arrive resolved by the server (milestone 4.5). For a worktree window `project` is already the right value — a linked worktree shares its project root with the main checkout — and `worktree` is replaced; see decision 21.
16. **Cleanup on failure.** If phase B fails after `worktree::create` succeeded, it calls `worktree::discard_new` before returning: `git worktree remove --force <path>`, then `git worktree prune`, then `git branch -D <branch>` only when `created_branch` is true, with a fresh `CLEANUP_TIMEOUT` deadline. The error returned to the client is the original one with `; the new worktree was removed` appended. If cleanup itself fails, log it at `warn` and append `; cleanup failed: <reason>` instead. Deleting a branch that this same create made moments ago at `HEAD` loses nothing, so this is not the "branch is never deleted" case, which is about removal. A create that fails registers nothing with `GitRegistry`, because the server registers only on `Ok(Ok(info))`; registration must not move earlier than that.
17. **A create is never cancelled half-way.** Once started, phase B runs to completion on the blocking pool even if the caller's future is dropped, so the server must never drop or abort a create future. See decision 25.

### Removing windows

18. **Removal is offered, never automatic.** `ClientMsg::Remove { remove_worktree: false }` removes the window exactly as today and leaves any worktree on disk. Kill (`C-b x`, `anthrex kill`) never touches worktrees. The TUI checkbox starts unticked.
19. **Removal with the worktree** is a new `WindowManager::remove_with_worktree(id, force, roots)`, `async`, in this order:
    1. Under the lock: the entry must exist, must have a managed worktree (else `window '<name>' has no worktree`), and must not already be removing (else `window '<name>' is already being removed`). Set `removing = true`, clone the `ManagedWorktree`, release the lock.
    2. Without `force`: `is_dirty` on the blocking pool. If dirty, clear `removing` and return `RemoveError::Dirty`. The agent keeps running and the root stays registered (decision 22).
    3. If the window is not Exited, signal it the way `remove` does: SIGKILL to the process group. Poll `list()` every 25 ms, for at most `KILL_GRACE`, until it is Exited. Never hold the lock while waiting.
    4. `roots.unregister(&wt.path)`, then `worktree::remove(git, wt, force, deadline)` on the blocking pool. On `Dirty` (a file appeared between the check and the kill) or any other error, `roots.register(wt.path.clone())` again, clear `removing` and return the error. The window stays listed, Exited, so the user can retry.
    5. Under the lock: remove the entry and publish.
20. **`force` without `remove_worktree`** is rejected with `--force only applies when removing the worktree`.

### Git state and the registry

Milestone 4.5 keys git state by worktree root and watches one root per registration, reference-counted inside `GitRegistry`. Worktrees appearing and disappearing under the daemon is exactly what that subsystem was built ahead of (`docs/ROADMAP.md`: "milestone 4.5 builds the watcher and the per-worktree git probe that milestone 5 needs the moment agents get their own checkouts"), so this milestone owns keeping it correct.

21. **A worktree window's `worktree` root is its own linked checkout.** The server resolves `roots.worktree` from `spec.cwd` *before* `create` runs, so for a worktree window that value names the directory the user chose, which is usually the main checkout. Phase C therefore overwrites it: `Entry.worktree = Some(managed.path.clone())`, so `WindowInfo.worktree` names the new linked worktree and the server's existing `git_registry.register(info.worktree)` registers the right root with no server change at all. Getting this wrong is not cosmetic: every worktree agent would show the parent repository's branch and dirty counts in the bottom bar, which is precisely the failure amendment §2 exists to prevent. `Entry.project` is untouched — a linked worktree shares its project root, so the tree still groups the window under the repository. This also satisfies §3.4: the value is computed once, before the window exists, and never changes.
22. **Unregistration follows the directory, not the window.** A root whose worktree has been deleted must not stay watched: the recursive watch would point at a deleted tree and every poll would probe a missing path, leaving a stale state published until something unregisters it (§3.8). A root whose worktree survived must not stop being watched: after a dirty refusal the agent is still running in it, and after a failed `worktree::remove` the directory is still there. The ordering of decision 19 step 4 is therefore the requirement: unregister immediately before `worktree::remove`, re-register if it fails. That ordering cannot be done from the server, which only sees the call's result, so `remove_with_worktree` takes a `&dyn GitRoots` — the two calls it needs, nothing more — which `GitRegistry` implements. The manager still knows nothing about git: `register` and `unregister` take the registry's own lock, spawn no git, and are called off the manager lock, so hard rule 10 holds. The plain `Remove` path is unchanged: the server keeps unregistering after `manager.remove` returns.
23. **Reference counting stays in `GitRegistry`.** Nothing in this milestone re-derives "is this the last window on this root" from a `manager.list()` snapshot. Milestone 4.5 removed exactly that pattern because it raced a concurrent create on the same root across two independent locks (`M4.5-git-and-tree.md`, "Root registration is reference-counted inside `GitRegistry`"), and `remove_with_worktree` is precisely where someone would put it back. `unregister` is called unconditionally, once per removed window; the registry's count decides whether anything stops. In practice this milestone makes most counts 1, one agent per worktree, but restart and attach paths can still put two windows on one root, so the count must not be assumed.

### Protocol and server

24. **Dirty-tree signal without a new message.** `DaemonMsg::Error.request` is already a free string. The daemon answers a worktree removal refused for changes with `request = "remove-dirty"` and the `WorktreeError::Dirty` text as `message`. Every other outcome uses the milestone-1 values: `Ack { request: "remove" }`, `Error { request: "remove" }`, `Created`, `Error { request: "create" }`. The strings become constants in `proto::messages::request`. No message changes shape. `PROTO_VERSION` nevertheless goes from 4 to 5, re-derived at implementation time from the value on `main` as amendment §3.5 requires: the `remove_worktree` and `force` flags have been on the wire since milestone 1 but have never been honoured, and a client built before this milestone answers a `remove-dirty` refusal with a toast and no way to force or keep — it would look to the user like the removal simply failed. Bumping the version makes the daemon refuse that client outright, with the mismatch message it already prints, instead of leaving it half-working.
25. **Long requests run beside the connection loop.** In `server::handle_client`, `CreateWindow` and `Remove` are each handled in their own `tokio::spawn`ed task, which sends its reply through a clone of `out_tx`. `CreateWindow` already has such a task; because `create` becomes `async` and does its own `spawn_blocking` internally, that task's existing `tokio::task::spawn_blocking` wrapper is *removed* rather than awaited through — two nested blocking hops would put phase A back on a blocking thread for no reason. The invariant the milestone-4.5 comment pins survives the change: `git_registry.register` is called after `create` returns, never from inside a closure that could still hold the manager lock. The loop goes straight on to the next frame, so input to other windows and `ListWindows` are never stuck behind a whole worktree operation. These tasks are never aborted (decision 17). Their reply send is best effort: if the client has gone, the result is only logged.

### Client

26. **`C-b c` opens the form.** `Command::NewWindow` keeps its name and binding but no longer sends `CreateWindow` directly. It sets `app.modal = Some(Modal::NewAgent(NewAgentForm::new(&app.form_defaults)))`.
27. **The form is a pure model.** New file `crates/tui/src/dialog.rs` holds `TextInput`, `NewAgentForm`, `RemoveConfirm` and their key handling. No I/O, no clock, no filesystem. It follows `AGENTS.md` rule 5 like `app.rs`.
28. **Fields, in order:** Runtime, Name, Directory, Worktree (checkbox), Branch (shown only when Worktree is ticked), Model and Prompt (hidden when Runtime is Shell). Focus starts on Runtime. Hidden fields keep their text but are neither focusable nor sent.
29. **Keys in the form:**

    | Key | Effect |
    |-----|--------|
    | `Tab`, `Down` | Next visible field, wrapping |
    | `Shift-Tab` (`BackTab`), `Up` | Previous visible field, wrapping |
    | `Enter` | Validate and submit, from any field |
    | `Esc`, `Ctrl-C` | Close the form; nothing is sent |
    | `Left` / `Right`, `Space` (Runtime) | Previous / next runtime; Space is next. Order Claude, Codex, Shell, wrapping |
    | `1` / `2` / `3` (Runtime) | Claude / Codex / Shell |
    | `Space` (Worktree) | Toggle |
    | Printable characters (text fields) | Insert at the cursor |
    | `Backspace`, `Delete`, `Left`, `Right`, `Home`, `End` (text fields) | Edit and move |
    | `Ctrl-U` (text fields) | Clear the field |

    The prefix key has no meaning inside the form. Every other key is ignored. While `submitting` is true, only `Esc` and `Ctrl-C` do anything.
30. **Pastes** go to the focused text field with `\r\n`, `\r` and `\n` replaced by a space. While any other modal is open, a paste is dropped, never sent to the PTY.
31. **Defaults.** The form opens with the runtime, directory text and model of the last form that the daemon accepted in this client session. On the first open: Claude, the default directory shown with the home prefix as `~` (`ui::terminal::shorten_home`), and an empty model. Name, branch and prompt always start empty and the worktree unticked. The Name field shows the placeholder `automatic (<runtime>-N)` while empty.
32. **Client-side validation** on Enter. The first failure sets `form.error` and moves focus to the failing field:
    - Directory: empty after trimming gives `directory is required`. `~` and `~/...` expand against `App.home_dir`. `~user` gives `only ~ and ~/ are expanded`. `~` with no known home gives `cannot expand ~: home directory unknown`. A relative path is joined onto `App.default_dir`.
    - Name: if not empty after trimming, at most 64 characters, no control characters (`name must be at most 64 characters`, `name must not contain control characters`), and not the name of a window in `app.windows` (`a window named '<name>' already exists`, the daemon's own wording).
    - Branch, when ticked: the first four rules of decision 8, with the same messages.
    - Model and prompt: trimmed; empty means `None`.
33. **Filesystem checks happen in the daemon, not the client.** The directory's existence, "is a git working tree", the branch being in use, `check-ref-format`, and every git failure come back as `DaemonMsg::Error { request: "create" }`. While the form is `submitting`, the app shows that message inline as `form.error` and clears `submitting`, and no toast is shown. Focus stays where it was.
34. **Submission.** A valid Enter sets `submitting = true` and returns `Effect::Send(ClientMsg::CreateWindow { spec, cols, rows })` with the current terminal size, as `Command::NewWindow` does today. `Created` closes the form, stores the form's runtime, directory text and model in `app.form_defaults`, and focuses the window through the existing `Created` path. `Esc` while submitting closes the form; the create still completes and `Created` still focuses the new window.
35. **Remove confirm.** `C-b X` opens `Modal::Remove(RemoveConfirm)`. The checkbox line appears only when the window's `branch` is `Some`. `Space` or `w` toggles it, `y` or `Enter` confirms, `n` or `Esc` cancels. Confirming sends `Remove { window_id, remove_worktree, force: false }`. When `remove_worktree` is true, the app records `pending_worktree_remove = Some(window_id)`.
36. **Force follow-up.** `Error { request: "remove-dirty" }` while `pending_worktree_remove` is `Some(id)` opens `Modal::ForceRemove { window_id: id, name, message }`. `f` sends `Remove { remove_worktree: true, force: true }` and keeps `pending_worktree_remove`. `k` sends `Remove { remove_worktree: false, force: false }`, which removes the window and keeps the worktree. `n` or `Esc` cancels; the window stays as it is. `Enter` does nothing here, so a destructive choice is never one reflexive key away. `Ack { request: "remove" }` or `Error { request: "remove" }` clears `pending_worktree_remove`; the error is shown as a toast.
37. **Branch display.** `WindowInfo.branch` is `Some` exactly for worktree windows: `Entry::info` already sets it from `spec.worktree_branch`, and it starts carrying a value the moment `create` stops rejecting that field. Milestone 4.5 left `branch` deliberately untouched for this. The tree's agent row shows it after the name as `[<branch>]` in the muted style. The budget for that is what is left after `row.guides`, which is not negotiable and costs two columns per level — twelve at depth six in a 32-column sidebar (amendment §4.1). Within what remains, the branch shrinks first, ending in `…`, and is dropped when fewer than 4 columns remain for it; the name keeps at least 8 columns. The overview (`C-b T`) shows the branch in full. The main pane title for a worktree window reads ` <name> · <runtime> · <shortened project root> (<branch>, worktree) `, using `WindowInfo.project`, instead of the long data-directory path; this replaces the rendering in `ui/terminal.rs`, which today formats `shorten_home(&w.cwd)` and a bare ` ({branch})`. `anthrex tree --json` already copies `info.branch` and needs no change.

### CLI

38. `anthrex new --worktree <branch>` sends the branch in `WindowSpec.worktree_branch`. `anthrex rm <target> --worktree [--force]` sends `Remove` with those flags; clap rejects `--force` without `--worktree`. A request that can run git waits `client::WORKTREE_REQUEST_TIMEOUT = 45 s` for its reply: the worktree operation deadline of decision 3, plus `KILL_GRACE`, plus margin. It is a third constant beside the existing `REQUEST_TIMEOUT` and `CREATE_WINDOW_REPLY_TIMEOUT`, which keep their values and their test: a `new` without `--worktree` still waits `CREATE_WINDOW_REPLY_TIMEOUT`, and every other command still waits `REQUEST_TIMEOUT`.
39. `anthrex rm` of a worktree window without `--worktree` prints to stderr `kept worktree <cwd> on branch <branch>`. A `remove-dirty` refusal prints the daemon's message followed by `run 'anthrex rm <target> --worktree --force' to discard the changes, or 'anthrex rm <target>' to keep the worktree` and exits 1.
40. `anthrex ls` gains a `BRANCH` column between `STATUS` and `DIR`, with `-` for windows without a worktree.

## Interfaces

### `crates/daemon/src/subprocess.rs` (changed)

```rust
/// The stdout `Outcome` plus the child's stderr, for callers that must tell the user
/// why a command failed.
pub struct Captured {
    pub outcome: Outcome,
    /// Lossy UTF-8, bounded by `max_stderr_bytes`. Empty when the child wrote none.
    pub stderr: String,
    /// `Some` only when the child never started. `ErrorKind::NotFound` means the program
    /// is not installed or not on PATH, which `Outcome::Failed` alone cannot tell apart
    /// from a non-zero exit.
    pub spawn_error: Option<std::io::ErrorKind>,
}

/// Unchanged, including nulling stderr.
pub fn run(command: &mut Command, max_output_bytes: usize, timeout: Duration) -> Outcome;

/// As `run`, but stderr is piped and drained under the same deadline, so a chatty
/// stderr can neither block the child nor outlive the timeout.
pub fn run_captured(
    command: &mut Command,
    max_output_bytes: usize,
    max_stderr_bytes: usize,
    timeout: Duration,
) -> Captured;
```

`Outcome` keeps its four variants and its meaning: `Complete` is "exited zero and stdout reached EOF", `Failed` covers both a spawn failure and a non-zero exit, and `spawn_error` is what separates them.

### `crates/daemon/src/worktree.rs` (new)

```rust
pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Directory name under `<wt>` that milestone 8 owns.
pub const RESERVED_DIR: &str = "runs";
/// Branch prefix that milestone 8 owns.
pub const RESERVED_BRANCH_PREFIX: &str = "anthrex/";
const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_STDERR_BYTES: usize = 8 * 1024;

/// One worktree this daemon created and owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedWorktree {
    pub repo_root: PathBuf, // the project root: the main checkout, canonical (decision 5)
    pub path: PathBuf,      // the linked worktree, and this window's worktree root
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub worktree: ManagedWorktree,
    pub created_branch: bool,
}

/// One git invocation's result. `success` means git ran and exited zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

impl GitOutput {
    /// Decision 14.
    pub fn stderr_tail(&self) -> String;
}

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("not a git repository: {}", dir.display())]
    NotARepo { dir: PathBuf },
    #[error("{0}")]
    InvalidBranch(String), // one of the branch messages below
    #[error("branch '{branch}' is already checked out at {}", path.display())]
    BranchInUse { branch: String, path: PathBuf },
    #[error("worktree path already exists: {}", path.display())]
    PathExists { path: PathBuf },
    #[error("worktree {} has uncommitted or untracked changes", path.display())]
    Dirty { path: PathBuf },
    #[error("git {action} failed: {stderr}")]
    Git { action: String, stderr: String }, // action e.g. "worktree add"
    #[error("git is not installed or not on PATH")]
    GitMissing,
    #[error("git {args} timed out after {secs} s")]
    TimedOut { args: String, secs: u64 },
}

pub fn hash8(path: &Path) -> String;
pub fn branch_dir_name(branch: &str) -> String;                 // "/" -> "-"
pub fn repo_worktrees_dir(worktrees_root: &Path, project_root: &Path) -> PathBuf; // <wt>
/// Decision 8, rules 1 to 4. Pure. `crates/tui/src/dialog.rs` has its own copy with the
/// same messages.
pub fn check_branch_syntax(branch: &str) -> Result<(), WorktreeError>;

pub fn create(git: &OsStr, dir: &Path, branch: &str, worktrees_root: &Path, deadline: Instant)
    -> Result<Created, WorktreeError>;
pub fn is_dirty(git: &OsStr, path: &Path, deadline: Instant) -> Result<bool, WorktreeError>;
pub fn remove(git: &OsStr, wt: &ManagedWorktree, force: bool, deadline: Instant)
    -> Result<(), WorktreeError>;
/// Decision 16. Uses its own `CLEANUP_TIMEOUT` deadline.
pub fn discard_new(git: &OsStr, created: &Created) -> Result<(), WorktreeError>;
```

Branch messages, exact:

| Rule | Message |
|------|---------|
| empty | `branch name is required` |
| whitespace | `branch name cannot contain spaces` |
| leading `-` | `branch name cannot start with '-'` |
| `anthrex/` prefix | `branches under anthrex/ are reserved for orchestration runs` |
| directory `runs` | `branch name 'runs' is reserved` |
| `check-ref-format` | `invalid branch name '<branch>'` |

`worktrees_root` is `<data_dir>/worktrees`. `lifecycle::run` sets `ManagerConfig.worktrees_root = opts.data_dir.join("worktrees")`.

### `crates/daemon/src/manager.rs` (changed)

```rust
struct Entry {
    // existing fields, including milestone 4.5's `worktree: Option<PathBuf>`, plus:
    /// The worktree this daemon created for the window, `None` for every other window.
    /// Not to be confused with `worktree`, which is the git worktree *root* the git
    /// registry watches (decision 21) and which milestone 4.5 owns.
    // milestone 6: restart re-attaches to this, not to `spec.worktree_branch` (risk 7).
    managed: Option<ManagedWorktree>,
    removing: bool,
}

struct Inner {
    next_id: u32,
    entries: BTreeMap<u32, Entry>,
    reserved_names: BTreeSet<String>, // new: names of creates in phase B
}

pub struct ManagerConfig {
    // milestone 3's six fields, plus:
    /// `<data_dir>/worktrees`. `ManagerConfig::new` and `from_env` set
    /// `std::env::temp_dir().join("anthrex-worktrees")`; `lifecycle::run` overrides it
    /// with `opts.data_dir.join("worktrees")`, and tests set a `TempDir`.
    pub worktrees_root: PathBuf,
}

/// The two calls `remove_with_worktree` needs from milestone 4.5's `GitRegistry`, and
/// nothing else, so the manager stays ignorant of git (decision 22). `GitRegistry`
/// implements it; tests use a recording stub.
pub trait GitRoots: Send + Sync {
    fn register(&self, root: PathBuf);
    fn unregister(&self, root: &Path);
}

impl WindowManager {
    /// Unchanged signature: `new(config: ManagerConfig)`.

    /// Now async; decision 15. `project` and `worktree` are milestone 4.5's parameters,
    /// resolved by the server before the call.
    pub async fn create(
        &self,
        spec: WindowSpec,
        project: PathBuf,
        worktree: Option<PathBuf>,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<WindowInfo>;

    /// Unchanged: removes the window, keeps any worktree.
    pub fn remove(&self, id: u32) -> anyhow::Result<()>;

    /// New; decisions 19 and 22.
    pub async fn remove_with_worktree(
        &self,
        id: u32,
        force: bool,
        roots: &dyn GitRoots,
    ) -> Result<(), RemoveError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    #[error("{0}")]
    Dirty(WorktreeError),        // always WorktreeError::Dirty
    #[error("{0}")]
    Failed(#[from] anyhow::Error),
}
```

`Entry::info()` is unchanged: `branch: self.spec.worktree_branch.clone()` is already what decision 37 wants once the rejection is gone, and it must not be made to read `managed`. `rename` also rejects names in `reserved_names`. Delete `daemon::WORKTREE_UNSUPPORTED` from `crates/daemon/src/lib.rs` and the `spec.worktree_branch.is_some()` rejection in `create`. Add `pub mod worktree;` to `lib.rs`; `pub mod git;` is already there.

### `crates/proto/src/messages.rs` (changed, no shape change)

```rust
/// Values of `DaemonMsg::Ack.request` and `DaemonMsg::Error.request` that clients act on.
pub mod request {
    pub const CREATE: &str = "create";
    pub const REMOVE: &str = "remove";
    /// A worktree removal refused because the tree has changes. `message` names the path.
    pub const REMOVE_DIRTY: &str = "remove-dirty";
}
```

The server uses these constants instead of the literals `"create"` and `"remove"`. `crates/proto/src/lib.rs` sets `PROTO_VERSION = 5` and its pin test becomes `proto_version_is_five`.

### `crates/tui/src/dialog.rs` (new)

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput { text: String, cursor: usize /* in chars */ }

impl TextInput {
    pub fn new(text: &str) -> Self;              // cursor at the end
    pub fn text(&self) -> &str;
    pub fn cursor(&self) -> usize;
    pub fn insert(&mut self, s: &str);
    pub fn backspace(&mut self);
    pub fn delete(&mut self);
    pub fn left(&mut self);
    pub fn right(&mut self);
    pub fn home(&mut self);
    pub fn end(&mut self);
    pub fn clear(&mut self);
    /// The part of the text that fits in `width` columns with the cursor visible,
    /// and the cursor's column inside that slice. Counts chars, not display width.
    pub fn visible(&self, width: u16) -> (String, u16);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField { Runtime, Name, Directory, Worktree, Branch, Model, Prompt }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormDefaults { pub runtime: Runtime, pub dir: String, pub model: String }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAgentForm {
    pub runtime: Runtime,
    pub name: TextInput,
    pub dir: TextInput,
    pub worktree: bool,
    pub branch: TextInput,
    pub model: TextInput,
    pub prompt: TextInput,
    pub focus: FormField,
    pub error: Option<String>,
    pub submitting: bool,
}

/// Everything validation needs from the app, borrowed.
pub struct FormContext<'a> {
    pub default_dir: &'a Path,
    pub home: Option<&'a Path>,
    pub existing_names: &'a [&'a str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormOutcome { Stay, Cancel, Submit(WindowSpec) }

impl NewAgentForm {
    pub fn new(defaults: &FormDefaults) -> Self;
    pub fn visible_fields(&self) -> Vec<FormField>;
    /// On a valid Enter this sets `submitting` and returns `Submit`.
    pub fn on_key(&mut self, key: KeyEvent, ctx: &FormContext<'_>) -> FormOutcome;
    pub fn on_paste(&mut self, text: &str);
    pub fn validate(&self, ctx: &FormContext<'_>) -> Result<WindowSpec, (FormField, String)>;
    pub fn defaults(&self) -> FormDefaults;
}

pub fn expand_dir(input: &str, default_dir: &Path, home: Option<&Path>) -> Result<PathBuf, String>;
/// Client copy of daemon decision 8 rules 1 to 4, same messages.
pub fn check_branch_syntax(branch: &str) -> Result<(), String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveConfirm {
    pub window_id: u32,
    pub name: String,
    pub branch: Option<String>,
    pub remove_worktree: bool,
}
```

Name limit constant: `pub const NAME_MAX_CHARS: usize`, holding decision 32's limit.

### `crates/tui/src/app.rs` (changed)

```rust
pub enum PendingAction { Kill(u32), StopDaemon }   // Remove(u32) moves to Modal::Remove

pub enum Modal {
    Confirm { message: String, action: PendingAction },
    Help,
    NewAgent(NewAgentForm),                              // new
    Remove(RemoveConfirm),                               // new
    ForceRemove { window_id: u32, name: String, message: String }, // new
}

pub struct App {
    // existing fields, plus:
    pub home_dir: Option<PathBuf>,          // set by `tui::run` from `dirs::home_dir()`
    pub form_defaults: FormDefaults,
    pending_worktree_remove: Option<u32>,
}
```

Modal key handling lives in `crates/tui/src/app/modal_keys.rs`, a submodule of `app.rs` (the same shape as `tree.rs` and `tree/rows.rs`); `app.rs` is at 596 lines before this milestone starts, so this split is not optional. See task M5.9.

### Rendering: `crates/tui/src/ui/dialog.rs` (new)

```
╭ new agent ───────────────────────────────────────────────────╮
│ › Runtime    [claude]  codex   shell                         │
│   Name       automatic (claude-N)                            │
│   Directory  ~/repos/shop                                    │
│   Worktree   [x] create a git worktree                       │
│   Branch     feat/api                                        │
│   Model      opus                                            │
│   Prompt     fix the flaky login test                        │
│                                                              │
│ ✕ not a git repository: /Users/me/notes                      │
│ Tab next · Shift-Tab back · Enter create · Esc cancel        │
╰──────────────────────────────────────────────────────────────╯
```

- Width `min(66, area.width - 2)`, labels 11 columns, focused label in the accent colour with `›`. The error line appears only when `error` is set, in the attention colour, wrapped to at most 3 lines. While `submitting`, the hint line reads `creating…` or, with the worktree ticked, `creating the worktree…`.
- The hardware cursor goes to the focused text field's cursor (`frame.set_cursor_position`). Nothing else will place it: `ui/terminal.rs` already suppresses the PTY cursor while `app.modal.is_some()`, so a form that does not set the cursor leaves it wherever the previous frame left it.

```
╭ remove ─────────────────────────────────────╮
│ Remove 'api-worker'? Its process is killed. │
│                                             │
│ [ ] also remove worktree feat/api           │
│     the branch is kept                      │
│                                             │
│ Space toggle · y / Enter remove · n / Esc   │
╰─────────────────────────────────────────────╯

╭ worktree has changes ───────────────────────────────╮
│ worktree /…/shop-5cc2e648/feat-api has uncommitted  │
│ or untracked changes                                │
│                                                     │
│ f  force: delete the worktree and those changes     │
│ k  keep the worktree, remove the window             │
│ n  cancel                                           │
╰─────────────────────────────────────────────────────╯
```

For a window without a worktree, `Modal::Remove` shows only the first line and the key hint without `Space toggle`.

### CLI

```
anthrex new --runtime claude|codex|shell [--name <n>] [--dir <path>] [--worktree <branch>] [--model <m>] [--prompt <p>]
anthrex rm <id|name> [--worktree [--force]]
anthrex ls        # columns: ID NAME RUNTIME STATUS BRANCH DIR
```

`crates/cli/src/client.rs` gains `pub const WORKTREE_REQUEST_TIMEOUT: Duration`, decision 38's value, beside `REQUEST_TIMEOUT` and `CREATE_WINDOW_REPLY_TIMEOUT`, which are unchanged. `request_with_timeout` already exists and is what the new call sites use.

## Tasks

Shared test helper for M5.3 to M5.6: `crates/daemon/tests/support/mod.rs` — the harness the daemon's integration tests already share — gains `TempRepo`. `TempRepo::new()` makes a `tempfile::TempDir`, runs `git init -b main` through the existing `git()` helper, which already pins `user.name`, `user.email`, `commit.gpgsign=false`, `init.defaultBranch=main` and `GIT_CONFIG_NOSYSTEM=1`, commits a `README`, and stores the canonical root. It also pins `core.hooksPath` to the repository's own `.git/hooks` — pinned, not disabled, because `slow_post_checkout` below depends on hooks running; the point is only that a developer's global `core.hooksPath` cannot reach in. Methods: `git(&self, args) -> String` (panics with stderr on failure), `branch_exists(&str) -> bool`, `worktree_paths() -> Vec<PathBuf>` from `git worktree list --porcelain`, and `slow_post_checkout(secs)`, which writes an executable `.git/hooks/post-checkout` that touches `<root>/.hook-started` and then sleeps.

`support/mod.rs` also takes over `manager()`, `spec()`, `create()` and `wait_until()` from `crates/daemon/tests/manager.rs`, with `manager()` returning a `TempDir` for `ManagerConfig.worktrees_root` alongside the manager so it lives as long as the test. Both `tests/manager.rs` and the new `tests/manager_worktree.rs` declare `mod support;` and use them.

Every test that waits for `.hook-started` waits in a deadline loop, which AGENTS.md rule 6 allows and which is not the wall-clock exception milestone 4.5's `a_real_write_triggers_a_probe` and `a_commit_in_a_linked_worktree_triggers_a_probe` were granted. Nothing here needs that exception.

### M5.1 Stderr capture in the shared runner

**Files.** Modify `crates/daemon/src/subprocess.rs`, `crates/daemon/tests/` (a new `subprocess.rs` test file).

**Tests first.** In `crates/daemon/tests/subprocess.rs`, against `/bin/sh -c` scripts, not against git.

- `run_captured_returns_stdout_and_stderr`: a script that writes to both and exits 0 gives `Outcome::Complete` with the stdout bytes and the stderr string, and `spawn_error: None`.
- `a_non_zero_exit_keeps_its_stderr`: a script that writes to stderr and exits 3 gives `Outcome::Failed` with the stderr text kept and `spawn_error: None`. This is the distinction `run` cannot make.
- `a_missing_program_reports_a_spawn_error`: a path that does not exist gives `Outcome::Failed` with `spawn_error: Some(ErrorKind::NotFound)`.
- `stderr_is_bounded_and_does_not_block_the_child`: a script that writes far more than `max_stderr_bytes` to stderr and then a line to stdout completes within the timeout, with stderr cut to the cap and the stdout line present. Without draining stderr the child would block on a full pipe.
- `run_is_unchanged`: the same three scripts through `run` give exactly the outcomes they give today, with stderr going nowhere.

**Change.** Add `Captured` and `run_captured` per the Interfaces block. `run` keeps its own signature, its `Stdio::null()` stderr and its existing behaviour; `project::detect_roots` and `git::probe::probe` are not touched and their tests must pass unchanged.

**Acceptance.** All new tests pass, every milestone-4.5 test in `crates/daemon/tests/project.rs` and `git_probe.rs` passes unchanged, and `grep -rn "killpg" crates/daemon/src` still prints only `process.rs` and `subprocess.rs`.

### M5.2 Worktree layout helpers and the git invocation

**Files.** Create `crates/daemon/src/worktree.rs` (pure parts and the private git helper only). Modify `crates/daemon/src/lib.rs`, `crates/daemon/Cargo.toml`.

**Tests first.** Unit tests inside the module.

- `worktree::tests::hash8_matches_fnv1a_vectors`: the three vectors from decision 6.
- `worktree::tests::branch_dir_name_replaces_slashes`: `feat/api/v2` gives `feat-api-v2`.
- `worktree::tests::repo_worktrees_dir_uses_sanitized_basename_and_hash`: project root `/tmp/my repo` gives `<worktrees_root>/my-repo-<hash8("/tmp/my repo")>`.
- `worktree::tests::branch_syntax_rules_and_messages`: each row of the branch message table except the last two returns exactly its message; `feat/api` passes.
- `worktree::tests::the_git_helper_passes_no_optional_locks_and_scrubs_the_environment`: with an argv-recording script as `git`, in the shape of `the_probe_passes_no_optional_locks` (`crates/daemon/tests/git_probe.rs`), the recorded invocation contains `--no-optional-locks` after `-C <dir>`, and none of the scrubbed `GIT_*` variables reach the child.
- `worktree::tests::a_passed_deadline_fails_without_spawning`: a deadline already in the past gives `WorktreeError::TimedOut` and the recording script is never run.
- `worktree::tests::a_missing_git_is_reported_as_such`: a program path that does not exist gives `WorktreeError::GitMissing`, not a `Git { .. }` failure.
- `worktree::tests::stderr_tail_keeps_the_last_lines_within_the_cap`: a `GitOutput` with 50 numbered stderr lines has a tail starting at line 31; a single 5000-character line is cut to decision 14's character limit.

**Change.** Implement decisions 1, 2, 3, 6, 7 (naming only), 8 (syntax rules) and 14, and the `GitOutput`, `WorktreeError`, `hash8`, `branch_dir_name`, `repo_worktrees_dir` and `check_branch_syntax` interfaces. The process-group kill and the reaping are `subprocess::run`'s and are already covered by `inherited_stdout_does_not_outlive_the_detection_deadline` (`crates/daemon/tests/project.rs`) and `a_timeout_marks_the_state_stale` (`crates/daemon/tests/git_probe.rs`); do not re-test them here.

**Acceptance.** All new tests pass. `cargo tree -p anthrex-daemon` shows no new dependency besides `thiserror`, which the workspace already has and the daemon does not yet.

### M5.3 Worktree create, remove, dirty check and cleanup

**Files.** Modify `crates/daemon/src/worktree.rs`, `crates/daemon/tests/support/mod.rs`. Create `crates/daemon/tests/worktree.rs`.

**Tests first.** All in `crates/daemon/tests/worktree.rs`, each with a fresh `TempRepo` and a `tempfile::TempDir` as `worktrees_root`, deadline `now + OPERATION_TIMEOUT` unless stated. Root resolution itself is milestone 4.5's and is tested there (`detect_roots_in_a_linked_worktree_splits_them`, `linked_worktree_maps_to_the_main_checkout`, `detect_roots_outside_a_repository_has_no_worktree`); the only thing to assert here is that `create` delegates to it and reports its own error.

- `a_directory_outside_a_repository_is_not_a_repo`: `create` in a plain tempdir returns `NotARepo` whose message starts with `not a git repository: `, and nothing is created.
- `create_with_a_new_branch`: `create(git, root, "feat/new", ...)` returns `created_branch == true`, a path equal to `<wt>/feat-new` where `<wt> = repo_worktrees_dir(worktrees_root, root)`, the directory contains `README`, `branch_exists("feat/new")`, and `git -C <path> rev-parse --abbrev-ref HEAD` prints `feat/new`.
- `create_with_an_existing_branch`: `git branch existing` first; `create` returns `created_branch == false` and the worktree is on `existing`.
- `create_from_a_subdirectory_still_uses_the_project_root`: `create(git, root/sub, "b", ...)` succeeds, the worktree path is the same as from `root`, and `ManagedWorktree.repo_root` is `repo.root`. Creating from inside an existing linked worktree gives the same `repo_root` too.
- `a_branch_checked_out_elsewhere_fails_cleanly`: create `taken` once, then again. The second returns `BranchInUse` naming the first worktree's path. Also `create(git, root, "main", ...)` returns `BranchInUse` naming `repo.root`. After both, `worktree_paths()` has exactly two entries (root and the first worktree), the first worktree still contains `README`, and `<wt>` contains only `taken`.
- `an_existing_path_is_refused`: create the directory `<wt>/feat-x` by hand; `create(git, root, "feat/x", ...)` returns `PathExists`, `branch_exists("feat/x")` is false, and `worktree_paths()` has one entry.
- `invalid_and_reserved_branches_are_refused_before_worktree_add`: `"bad name"`, `"-x"`, `"anthrex/r/t1"`, `"runs"` and `"a..b"` each give the matching message from the table. `worktree_paths()` still has one entry.
- `is_dirty_sees_modified_and_untracked_but_not_ignored_files`: fresh worktree is clean; a file matching a committed `.gitignore` leaves it clean; an untracked file makes it dirty; after deleting that, modifying `README` makes it dirty.
- `remove_clean_worktree_keeps_the_branch`: `remove(git, wt, false, ...)` succeeds, the path is gone, `branch_exists` is still true.
- `remove_dirty_worktree_is_refused_then_forced`: add `untracked.txt`. `remove(git, wt, false, ...)` returns `Dirty` with the path and the directory still exists. `remove(git, wt, true, ...)` succeeds, the path is gone, the branch still exists.
- `remove_of_a_missing_path_prunes`: delete the worktree directory by hand; `remove(git, wt, false, ...)` succeeds and `worktree_paths()` has one entry.
- `discard_new_deletes_only_a_branch_it_created`: after `create` of a new branch, `discard_new` removes the path and the branch. After `create` on a pre-existing branch, `discard_new` removes the path and keeps the branch.
- `every_invocation_of_a_real_create_passes_no_optional_locks`: run `create` with a wrapper script that appends its argv to a log and then execs the real git. The create succeeds, and every recorded line contains `--no-optional-locks` (decision 2).
- `a_timeout_during_create_cleans_up`: `slow_post_checkout(10)`, deadline `now + 1 s`. `create` returns `WorktreeError::TimedOut` within 3 s. Afterwards `worktree_paths()` has one entry, the target directory does not exist, and the new branch does not exist.

**Change.** Implement decisions 5, 7 (the reserved `runs` name and the path-exists rule), 8 (the `check-ref-format` rule), 9, 10, 12, 13 and 16 (`discard_new`). `create` calls `discard_new` itself when a git step after `worktree add` started fails or times out, including the timeout case above, before returning the original error.

**Acceptance.** All tests pass on macOS and Linux with the git on the machine. Record the git version in "Implementation notes".

### M5.4 Manager create off the lock, with worktrees

**Files.** Modify `crates/daemon/src/manager.rs` and create `crates/daemon/src/manager/create.rs`; modify `crates/daemon/src/lib.rs`, `crates/daemon/src/lifecycle.rs`, `crates/daemon/src/server.rs` (the `CreateWindow` task only), `crates/daemon/tests/support/mod.rs`, `crates/cli/src/main.rs` (the rejection only), and every caller of `WindowManager::create`: `crates/daemon/tests/manager.rs`, `crates/daemon/tests/server.rs` and the `support` harness itself. Create `crates/daemon/tests/manager_worktree.rs`. `crates/daemon/tests/server_git.rs` and `crates/tui/tests/connection.rs` build a `ManagerConfig` but never call `create`, and `ManagerConfig::new` keeps setting a default `worktrees_root`, so neither needs a change.

`manager.rs` is 531 lines before this task. Phase A to C, `Reservation` and `reserved_names` move into `crates/daemon/src/manager/create.rs`, a submodule of `manager.rs`; `manager.rs` keeps `ManagerConfig`, `Entry`, `Inner`, the accessors and the short operations. `crates/daemon/tests/manager.rs` is 592 lines before this task, so every test below goes in the new `crates/daemon/tests/manager_worktree.rs`.

**Tests first.** In `crates/daemon/tests/manager_worktree.rs`. Existing tests in `tests/manager.rs` switch to `m.create(spec, project, worktree, cols, rows).await`; a plain window's project is `project::detect_roots(dir).project` or, where nothing depends on it, `std::env::temp_dir()` with `worktree: None`. A worktree window's project is `repo.root`.

- Delete `a_spec_asking_for_a_worktree_is_rejected` from `tests/manager.rs`.
- `a_worktree_window_runs_in_its_worktree`: `TempRepo`; spec with `cwd = repo.root`, `worktree_branch = Some("feat/wt")`. The returned info has `branch == Some("feat/wt")` and `cwd == <wt>/feat-wt`. Send `pwd\n` and wait until the snapshot contains `feat-wt`. The window without a branch still reports `branch == None`.
- `a_worktree_window_reports_its_own_worktree_root`: the same create, called with `worktree = Some(repo.root)` as the server would pass it, returns `info.worktree == Some(<wt>/feat-wt)` — the new checkout, not the parent (decision 21) — while `info.project == repo.root`. A plain window created in `repo.root` reports `info.worktree == Some(repo.root)`.
- `worktree_windows_group_under_their_repository_project`: create one plain window in `repo.root` and one worktree window. Both infos have the same `project`, equal to `repo.root`.
- `create_errors_come_back_before_any_window_exists`: a non-repo directory with a branch fails with `not a git repository`; a missing directory fails with `directory does not exist`; a branch checked out in the main checkout fails with `is already checked out at`. After each, `m.list()` is empty and `worktree_paths()` has one entry.
- `a_failed_spawn_removes_the_new_worktree`: a manager whose shell is `/nonexistent/anthrex-test-shell`. Create a Shell window with `worktree_branch = Some("doomed")`. The error ends with `the new worktree was removed`; `worktree_paths()` has one entry and `branch_exists("doomed")` is false.
- `a_slow_worktree_create_does_not_block_the_manager`: `slow_post_checkout(2)`. Start the worktree create in a `tokio::spawn`. Wait with a deadline for `.hook-started`. Then assert `m.list()` returns in under 100 ms, and a plain Shell `create` for another name completes in under 1 s. Finally the worktree create succeeds.
- `a_name_is_reserved_while_its_create_is_in_flight`: `slow_post_checkout(2)`; start a worktree create named `dup`; once `.hook-started` exists, a plain create named `dup` fails with `already exists`, and `m.rename(other_id, "dup")` fails too. After the first create finishes, it is listed as `dup`.

**Change.** Implement decisions 11, 15, 16, 17 and 21 and the `create` half of the manager interface. Remove `WORKTREE_UNSUPPORTED` from `crates/daemon/src/lib.rs` and `manager.rs`, and remove the `if worktree.is_some() { anyhow::bail!(daemon::WORKTREE_UNSUPPORTED) }` block in `crates/cli/src/main.rs`, which would no longer compile. `lifecycle::run` sets `worktrees_root` to `opts.data_dir.join("worktrees")`. In `server.rs`, the `CreateWindow` task drops its `tokio::task::spawn_blocking` wrapper and awaits `create` directly (decision 25); its `git_registry.register(info.worktree)` call and the comment above it stay exactly as they are.

**Acceptance.** `grep -rn WORKTREE_UNSUPPORTED crates` finds nothing. All tests pass, including `crates/daemon/tests/server_git.rs` unchanged.

### M5.5 Removing a window with its worktree

**Files.** Modify `crates/daemon/src/manager.rs` and create `crates/daemon/src/manager/remove.rs`; modify `crates/daemon/src/git/mod.rs` (`impl GitRoots for GitRegistry`) and `crates/daemon/tests/manager_worktree.rs`.

**Tests first.** In `crates/daemon/tests/manager_worktree.rs`, each with a `TempRepo` and a worktree Shell window that has reached Working. `roots` is a recording stub implementing `GitRoots` that records every `register` and `unregister` in order.

- `remove_with_worktree_removes_a_clean_tree_and_keeps_the_branch`: `remove_with_worktree(id, false, &roots)` succeeds; the window is not listed; the worktree directory is gone; the branch exists. The stub recorded exactly one `unregister` of the worktree path and no `register`.
- `a_dirty_tree_is_refused_and_the_agent_keeps_running`: `touch dirty.txt` through `write_input`, then wait until the file exists. `remove_with_worktree(id, false, &roots)` returns `RemoveError::Dirty`, whose message contains the worktree path and `uncommitted or untracked`. The window is still listed and not Exited, `write_input` still reaches it (send `echo still-here` and see it in the snapshot), and the stub recorded nothing at all — a refused removal must not unwatch a worktree the user is still working in.
- `a_failed_removal_re_registers_the_root`: make `worktree::remove` fail after the kill (a file appears between the dirty check and the kill, as risk 8 describes). The call returns an error, the window stays listed and Exited, and the stub recorded `unregister` followed by `register` of the same path.
- `force_removes_a_dirty_tree`: same setup as the dirty test, then `remove_with_worktree(id, true, &roots)` succeeds; the window is gone and so is the directory, the branch is kept, and the stub recorded one `unregister`.
- `plain_remove_keeps_the_worktree`: `m.remove(id)` succeeds and the worktree directory and branch both still exist.
- `remove_with_worktree_rejects_windows_without_one_and_double_removal`: a plain window gives `has no worktree` and stays listed. For a clean worktree window, run two `remove_with_worktree(id, false, &roots)` calls at once with `tokio::join!`: exactly one succeeds, and the other fails with `already being removed` or, if the first had already finished, `no window with id`. The stub recorded exactly one `unregister`.
- `an_exited_window_is_removed_without_waiting`: `write_input(id, b"exit\n")`, wait for Exited, then `remove_with_worktree(id, false, &roots)` completes in under 1 s.

**Change.** Implement decisions 18, 19, 20 and 22 at the manager level (`RemoveError`, `removing`, the `GitRoots` trait and `impl GitRoots for GitRegistry`). Decision 20's message is produced by the server in M5.6, because `remove` itself has no `force` argument.

**Acceptance.** All tests pass. No test takes longer than 5 s.

### M5.6 Server: requests in tasks, `remove-dirty`, request constants, protocol 5

**Files.** Modify `crates/proto/src/lib.rs`, `crates/proto/src/messages.rs`, `crates/daemon/src/server.rs` and create `crates/daemon/src/server/requests.rs`; modify `crates/daemon/tests/server.rs`, `crates/daemon/tests/support/mod.rs`.

`server.rs` is 552 lines before this task. The `CreateWindow` and `Remove` request tasks, and the registry calls around them, move into `crates/daemon/src/server/requests.rs`, a submodule of `server.rs`; `server.rs` keeps `serve`, `handle_client`, the subscription handling and the short arms.

**Tests first.**

- `proto::tests::proto_version_is_five`: replaces `proto_version_is_four`.
- `proto::messages::tests::request_constants_are_stable`, in the module that already exists at the bottom of `messages.rs`: the three constants equal `create`, `remove` and `remove-dirty`.
- In `crates/daemon/tests/server.rs`, through the `support` harness, whose `start_daemon` gains a `worktrees_root` inside its `TempDir`:
  - `create_with_a_worktree_over_the_socket`: `CreateWindow` with a branch in a `TempRepo` returns `Created`; the next `WindowsChanged` lists the window with that branch and with `worktree` equal to the new linked worktree.
  - `create_errors_use_the_create_request`: a non-repo directory with a branch returns `Error { request: "create", message }` with `not a git repository` in `message`.
  - `list_is_answered_while_a_create_is_running`: `slow_post_checkout(3)`. Send `CreateWindow` with a branch, wait for `.hook-started`, then send `ListWindows`. A `WindowsChanged` arrives within 500 ms, before `Created`. `Created` arrives later.
  - `a_dirty_worktree_removal_answers_remove_dirty_then_force_works`: create a worktree Shell window, make it dirty through `Input`, send `Remove { remove_worktree: true, force: false }`: the reply is `Error { request: "remove-dirty" }` with the path in `message`. Send it again with `force: true`: the reply is `Ack { request: "remove" }` and the window leaves the list.
  - `force_without_worktree_is_rejected`: `Remove { remove_worktree: false, force: true }` returns `Error { request: "remove", message: "--force only applies when removing the worktree" }` and the window stays.

**Change.** Implement decisions 24 and 25. `ClientMsg::Remove` dispatch: `force && !remove_worktree` is the error above; `!remove_worktree` keeps today's inline path, including its existing unregister-after-`remove`; otherwise a task calls `remove_with_worktree(id, force, &*git_registry)` and maps `RemoveError::Dirty` to `REMOVE_DIRTY`. `CreateWindow` keeps its task and uses the `CREATE` constant. Keep the rule that `Snapshot` is queued before live output; it is unaffected because subscriptions stay in the loop. Raise `PROTO_VERSION` to 5; no client holds the number as a literal, so the constant and its pin test are the whole change (AGENTS.md rule 4: no message is new, so no new round-trip test is owed).

**Acceptance.** All tests pass, including every milestone-1 server test and every milestone-4.5 test in `crates/daemon/tests/server_git.rs`, unchanged except for the helper.

### M5.7 CLI: `new --worktree`, `rm --worktree [--force]`, `ls` branch column

**Files.** Modify `crates/cli/src/main.rs`, `crates/cli/src/client.rs`.

**Tests first.** Unit tests in `crates/cli/src/main.rs` (`#[cfg(test)] mod tests`, using `Cli::try_parse_from`) and in `crates/cli/src/client.rs`'s existing test module.

- `rm_force_requires_worktree`: `["anthrex", "rm", "x", "--force"]` fails to parse; `["anthrex", "rm", "x", "--worktree", "--force"]` parses with both flags set.
- `new_accepts_a_worktree_branch`: `["anthrex", "new", "--runtime", "shell", "--worktree", "feat/x"]` parses with `worktree == Some("feat/x")`.
- `table_has_a_branch_column`, replacing the header and row assertions of `table_has_header_and_one_row_per_window` (`crates/cli/src/client.rs`, which is where `format_table` and its test live): the header is `ID NAME RUNTIME STATUS BRANCH DIR` with the existing padding; a window with `branch: Some("feat/x")` shows it; one without shows `-`.
- `dirty_hint_names_both_commands`: a new pure function `dirty_hint(target: &str) -> String` returns exactly the second sentence of decision 39.
- `create_timeout_adds_allowance_without_changing_the_default` is untouched, and a new assertion pins `WORKTREE_REQUEST_TIMEOUT` as the longest of the three.

**Change.** Implement decisions 38, 39 and 40. Update the `--worktree` help text to `Create a git worktree on this branch and start the window in it`, and add clap's `requires` so `--force` needs `--worktree`. `new` with `--worktree` and `rm` with `--worktree` use `request_with_timeout(msg, client::WORKTREE_REQUEST_TIMEOUT)`; `new` without it keeps `CREATE_WINDOW_REPLY_TIMEOUT`. `rm` matches `Error { request: "remove-dirty" }` to print the message and the hint and exit 1; `anyhow::bail!` of the combined text is enough. `crates/cli/src/tree_cmd.rs` already copies `info.branch` into its output and needs no change.

**Acceptance.** Tests pass. The smoke stages in M5.11 exercise the CLI end to end.

### M5.8 The new-agent form model

**Files.** Create `crates/tui/src/dialog.rs`. Modify `crates/tui/src/lib.rs` (`pub mod dialog;`).

**Tests first.** Unit tests in `crates/tui/src/dialog.rs`. `ctx()` builds a `FormContext` with `default_dir = /work`, `home = /home/me`, `existing_names = ["api"]`.

- `text_input_edits_at_the_cursor`: insert `helo`, left, insert `l`, gives `hello` with cursor 4; Home, Delete gives `ello`; End, Backspace gives `ell`; multi-byte characters (`é`, `日`) move by one char each; `clear` empties.
- `text_input_visible_keeps_the_cursor_in_view`: a 30-char text with the cursor at the end and width 10 shows the last 9 chars and cursor column 9; with the cursor at 0 it shows the first 10 and column 0.
- `fields_follow_runtime_and_worktree`: defaults give `[Runtime, Name, Directory, Worktree, Model, Prompt]`; ticking the worktree inserts `Branch` after `Worktree`; Shell removes `Model` and `Prompt`.
- `tab_and_shift_tab_wrap_over_visible_fields`: from Runtime, BackTab goes to Prompt; with Shell, Tab from Worktree (unticked) wraps to Runtime; Up and Down behave like BackTab and Tab.
- `runtime_field_keys`: Right goes Claude to Codex to Shell to Claude; Left the reverse; Space is Right; `3` selects Shell. Letters on the Runtime field change nothing.
- `space_toggles_the_worktree_only_on_its_field`: Space on Worktree toggles; Space in Name inserts a space.
- `escape_and_ctrl_c_cancel`: both return `Cancel`.
- `enter_submits_a_full_spec`: Runtime Codex, name `  api-2 `, dir `~/repos/shop`, worktree ticked, branch `feat/x`, model `gpt-5-codex`, prompt `hi` gives `Submit(WindowSpec { name: Some("api-2"), runtime: Codex, cwd: /home/me/repos/shop, worktree_branch: Some("feat/x"), model: Some("gpt-5-codex"), initial_prompt: Some("hi") })` and `submitting == true`.
- `shell_drops_hidden_fields`: Shell with text left in Model and Prompt submits `model: None, initial_prompt: None`; an unticked worktree with branch text submits `worktree_branch: None`.
- `validation_errors_focus_the_field`: empty dir gives `directory is required` with focus Directory; `~bob/x` gives `only ~ and ~/ are expanded`; name `api` gives `a window named 'api' already exists` with focus Name; a 65-char name gives the length message; ticked worktree with empty branch gives `branch name is required` with focus Branch; `anthrex/x` gives the reserved message. None of these return `Submit` or set `submitting`.
- `expand_dir_cases`: `~` gives home; `~/a` gives `home/a`; `rel/x` gives `/work/rel/x`; `/abs` is unchanged; `~` with `home = None` gives the unknown-home message.
- `submitting_ignores_everything_but_cancel`: after a Submit, Tab and characters change nothing and return `Stay`; Esc returns `Cancel`.
- `paste_replaces_newlines_with_spaces`: pasting `a\r\nb\nc` into Prompt gives `a b c`; pasting on the Runtime field changes nothing.

**Change.** Implement decisions 27 to 32 in `dialog.rs` and the interface above, plus `RemoveConfirm` (data only; its keys live in `app/modal_keys.rs`).

**Acceptance.** Tests pass. `dialog.rs` has no `use` of `std::fs`, `std::process`, `tokio`, `dirs` or `std::time`.

### M5.9 App: open, submit and close the form

**Files.** Modify `crates/tui/src/app.rs`, create `crates/tui/src/app/modal_keys.rs`, modify `crates/tui/src/lib.rs`, `crates/tui/src/ui/terminal.rs`, `crates/tui/src/ui/modal.rs`. Create `crates/tui/src/app_tests/dialog.rs`.

`app.rs` is 596 lines before this task, so the split is part of it, not a contingency: modal key handling for every `Modal` variant moves to `crates/tui/src/app/modal_keys.rs`, a submodule of `app.rs`, and `app.rs` keeps the state, the effects and `on_daemon`. `app_tests.rs` is 534 lines and is already a hub of `#[path]` submodules, so the tests below go in `crates/tui/src/app_tests/dialog.rs`, declared from `app_tests.rs` like the four existing ones.

**Tests first.** In `crates/tui/src/app_tests/dialog.rs`.

- Replace `new_window_creates_a_shell_in_the_default_dir_and_focuses_it_when_created` (`crates/tui/src/app_tests.rs`) with `new_window_opens_the_form_and_submits_on_enter`: `C-b c` returns no effects and sets `Modal::NewAgent` with focus on Runtime. Press `3`, then Enter: exactly one `Send(CreateWindow { spec, cols: 80, rows: 24 })` with `runtime: Shell` and `cwd: /tmp`. The modal is still open with `submitting == true`. `Created { window_id: 9 }` closes the modal; the following `WindowsChanged` with window 9 subscribes to it, as before.
- `a_create_error_is_shown_inline_not_as_a_toast`: submit, then `Error { request: "create", message: "not a git repository: /x" }`. The form stays open, `form.error` holds the message, `submitting` is false, `toast_text()` is `None`.
- `an_unrelated_error_while_the_form_is_open_still_toasts`: `Error { request: "kill", ... }` with the form open and not submitting shows a toast and leaves `form.error` unset.
- `keys_and_pastes_go_to_the_form_not_the_pty`: with the form open, typing `j` or pasting text returns no `Input` effect. With `Modal::Help` open, a paste returns no effects.
- `the_form_remembers_the_last_accepted_values`: submit with Codex, dir `~/p` and model `m1`, receive `Created`, open again: runtime Codex, dir `~/p`, model `m1`, name, branch and prompt empty, worktree unticked. A submit answered by `Error` does not change the defaults.
- `escape_while_submitting_still_focuses_the_new_window`: submit, Esc closes the form, then `Created` and `WindowsChanged` focus the window.
- `first_open_shows_the_default_dir_with_tilde`: `App.default_dir = /home/me/code`, `home_dir = Some(/home/me)`: the Directory field text is `~/code`. Build this with the pure helper the form uses; `shorten_home` currently calls `dirs::home_dir()`, so add a pure variant `shorten_home_with(path, home: Option<&Path>)` in `ui::terminal` and make `shorten_home` call it.
- A `DaemonMsg::Git` arriving while the form is open still reaches the bottom bar unchanged, so milestone 4.5's `app_tests/git.rs` keeps passing without modification.

**Change.** Implement decisions 26, 30, 31, 33 and 34. In `on_key`, take the modal out with `self.modal.take()`, handle the key, and put it back unless the handler closed it; the current `clone()` would drop in-place edits to the form. Route `on_paste` to the form or drop it while any modal is open. `tui::run` sets `app.home_dir = dirs::home_dir()`. In tree mode (milestone 4), an open modal takes keys before the tree does. `ui::modal::render` matches exhaustively over `Modal`, so this task adds arms for the three new variants or the client stops compiling; a one-line placeholder for each is enough, and M5.11 replaces them with the real rendering.

**Acceptance.** All app tests pass. `app.rs` still performs no I/O, and both it and `app/modal_keys.rs` are well under 600 lines.

### M5.10 App: remove confirm and force follow-up

**Files.** Modify `crates/tui/src/app.rs`, `crates/tui/src/app/modal_keys.rs`. Create `crates/tui/src/app_tests/remove.rs`.

**Tests first.** In `crates/tui/src/app_tests/remove.rs`, declared from `app_tests.rs` like the others. `wt_win(id, name, branch)` builds a window with `branch: Some(branch)`.

- `remove_of_a_plain_window_sends_a_plain_remove`: `C-b X` opens `Modal::Remove` with `branch: None`; Space changes nothing; `y` sends `Remove { window_id, remove_worktree: false, force: false }`.
- `remove_checkbox_is_offered_and_starts_unticked`: for a worktree window, Enter right away sends `remove_worktree: false`. Opening again, `w` ticks it (Space also toggles), then Enter sends `remove_worktree: true, force: false`.
- `a_dirty_refusal_opens_the_force_prompt`: after the ticked remove, `Error { request: "remove-dirty", message: "worktree /w has uncommitted or untracked changes" }` opens `ForceRemove` with that message and no toast. `Enter` does nothing. `f` sends `Remove { remove_worktree: true, force: true }`.
- `keep_removes_only_the_window`: from `ForceRemove`, `k` sends `Remove { remove_worktree: false, force: false }`.
- `cancel_leaves_everything`: from `ForceRemove`, Esc sends nothing and closes the modal.
- `remove_dirty_without_a_pending_removal_is_a_toast`: with no removal pending, `Error { request: "remove-dirty" }` only toasts.
- `ack_or_error_clears_the_pending_removal`: after `Ack { request: "remove" }`, a later `remove-dirty` only toasts. `Error { request: "remove", ... }` toasts and also clears it.
- Update `kill_asks_for_confirmation_first` (`crates/tui/src/app_tests.rs`) only if removing `PendingAction::Remove` breaks it; it uses `PendingAction::Kill`, which survives.

**Change.** Implement decisions 35 and 36. Remove `PendingAction::Remove`.

**Acceptance.** All app tests pass.

### M5.11 Rendering, tree branch, help, smoke stages

**Files.** Create `crates/tui/src/ui/dialog.rs` and `crates/tui/src/ui/dialog_tests.rs`. Modify `crates/tui/src/ui/mod.rs`, `crates/tui/src/ui/modal.rs`, `crates/tui/src/ui/terminal.rs`, `crates/tui/src/ui/tree_view.rs` (`narrow_line` and `wide_line`), `crates/tui/src/ui/tree_view_tests.rs`, `scripts/pty-smoke.py`.

`ui/mod.rs` is 564 lines with its test module inline, so the new `TestBackend` tests go in `crates/tui/src/ui/dialog_tests.rs`, declared with `#[path]` from `ui/mod.rs` exactly as `statusbar_tests.rs` and `tree_view_tests.rs` are. The two tree-row tests below go in `tree_view_tests.rs`, beside milestone 4.5's guide tests.

**Tests first.**

- `new_agent_form_renders_fields_and_error`: a form with the worktree ticked, branch `feat/x` and error `not a git repository: /x`, rendered at 100 by 30, contains `new agent`, `Runtime`, `[claude]`, `Branch`, `feat/x`, the error text and `Enter create`. With Shell, `Model` and `Prompt` are absent. While submitting with the worktree ticked, the hint contains `creating the worktree`.
- `new_agent_form_places_the_cursor_in_the_focused_field`: with focus on Name and text `ab`, `terminal.get_cursor_position()` after the draw is on the Name row, 2 columns after the field start.
- `remove_dialog_shows_the_checkbox_only_for_worktree_windows`: a worktree window renders `[ ] also remove worktree feat/x` and `the branch is kept`; ticked renders `[x]`; a plain window renders neither.
- `force_prompt_lists_the_three_choices`: contains `force`, `keep the worktree` and `cancel`.
- `main_title_of_a_worktree_window_names_project_and_branch`: a window with `branch: Some("feat/x")` and `project: /tmp/shop` renders `(feat/x, worktree)` and `/tmp/shop` in the title, not its cwd. A window with `branch: None` still renders today's ` <name> · <runtime> · <shortened cwd> `.
- `tree_row_shows_the_branch_and_truncates_it_first`: a 34-column sidebar with a worktree window named `api` on branch `feat/very-long-branch-name` shows `api [feat/` and `…]`; a window named `a-rather-long-name` keeps at least 8 name columns. The overview shows the full branch.
- `a_deep_row_spends_its_guides_before_its_branch`: the same window at depth three loses branch columns, never guide columns; the guides are byte-for-byte what milestone 4.5 renders for that depth.
- `tree_groups_worktree_windows_under_the_repository`: two windows with the same `project` and different cwds (one in the data directory) render under one project row with the count `sh 2`.
- `help_lists_new_agent`: the help overlay contains `C-b c` and `new agent`. The empty-state hint in `ui/terminal.rs`, today `No agents. Press C-b c to open a shell here, or run \`anthrex new\`.`, becomes `No agents. Press C-b c to create one, or run \`anthrex new\`.`, and `empty_state_and_hidden_sidebar` (`crates/tui/src/ui/mod.rs`) is updated to the new string and still passes.

**Change.** Implement the rendering in the Interfaces section and decision 37. `ui::modal::render` dispatches the three new variants to `ui::dialog`. Update `HELP` in `ui/modal.rs`: `("C-b c", "new agent")` replaces `("C-b c", "new shell window")`, and `("C-b X", "remove agent (and worktree)")` replaces `("C-b X", "remove agent")`.

Smoke script (`scripts/pty-smoke.py`). The script exports `ANTHREX_GIT=off` for the daemon it starts, so both new stages run with milestone 4.5's git subsystem disabled. They must therefore assert only on `WindowInfo.branch`, the tree and the filesystem, never on a `Git` message or the bottom bar's git segment; nothing in this milestone depends on one.

- Add `new_shell(proc, expected)`: send `\x02c`, wait for `new agent`, send `3`, send `\r`, wait for `expected`. Use it everywhere the script sends `\x02c` today (stages 2, 3, 7 and 8).
- Add a module-level `make_repo()` that creates `/tmp/anthrex-smoke-repo-<pid>` with `git init -b main`, repo-local identity and `commit.gpgsign false`, and one commit. The final cleanup removes it.
- New stage `W1: worktree from the CLI`, before the stop stage:
  1. `anthrex new --runtime shell --name wt-cli --dir <repo> --worktree smoke/cli` exits 0.
  2. `anthrex ls` lists `wt-cli` with `smoke/cli`.
  3. `git -C <repo> worktree list --porcelain` contains `branch refs/heads/smoke/cli` and a path under `<DATA_DIR>/worktrees/`.
  4. The same `new` with `--name wt-dup` exits non-zero with `already checked out` on stderr.
  5. `anthrex rm wt-cli --worktree` exits 0; the worktree directory is gone; `git -C <repo> branch --list smoke/cli` still prints the branch.
- New stage `W2: form, dirty removal, force`:
  1. Attach. Send `\x02c`, `3`, Tab, `wt-form`, Tab, `\x15` (Ctrl-U), the repo path, Tab, Space, Tab, `smoke/form`, `\r`. Wait for `wt-form` and `smoke/form` on screen.
  2. Send `pwd\r`; wait for `smoke-form` in the output.
  3. Send `touch dirty.txt\r`, then wait until the file exists in the worktree.
  4. Send `\x02X`; wait for `also remove worktree`. Send a space, then `\r`. Wait for `uncommitted or untracked`.
  5. Send `f`. Poll `anthrex ls` with a 10-second deadline until `wt-form` is gone. Assert the worktree directory is gone and the branch `smoke/form` still exists.
  6. Detach and check the exit status is 0.

**Acceptance.** All tests pass. `python3 scripts/pty-smoke.py` passes with every old and new stage.

## Verification

Run the five commands from `AGENTS.md`:

```bash
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
python3 scripts/pty-smoke.py
```

Milestone-specific checks:

- `grep -rn "WORKTREE_UNSUPPORTED\|not implemented yet" crates` prints nothing.
- `grep -rn "killpg" crates/daemon/src` prints only `process.rs` (the window kill) and `subprocess.rs`. A third hit means a second runner was written instead of reusing the first (decision 1).
- `grep -rn "no-optional-locks" crates/daemon/src` prints `git/probe.rs` and `worktree.rs`, and every git invocation in `worktree.rs` is covered by `every_invocation_passes_no_optional_locks` (hard rules 10 and 11).
- Search `manager.rs`, `manager/create.rs` and `manager/remove.rs` for every `crate::lock(&self.inner)`: no guard may be alive across a call into `worktree::`, `Window::spawn`, `GitRoots::register`, `GitRoots::unregister`, `spawn_blocking` or `.await`. Say in the pull request that you checked this.
- `PROTO_VERSION` is 5 and `proto_version_is_five` pins it.
- No file this milestone touched is over 600 lines (hard rule 8): check `app.rs`, `manager.rs`, `server.rs`, `ui/mod.rs`, `app_tests.rs`, `tests/manager.rs` and `tests/support/mod.rs`.
- After the smoke script, `pgrep -fl "anthrex daemon"` shows nothing of yours and `/tmp/anthrex-smoke-repo-*` is gone.

## Manual check

Use an isolated daemon throughout:

```bash
export ANTHREX_SOCKET=/tmp/anthrex-m5/daemon.sock ANTHREX_DATA_DIR=/tmp/anthrex-m5/data
cargo build --release && ./target/release/anthrex --dir ~/some/real/repo
```

1. `C-b c`. The form opens on Runtime with Claude selected. Tab through every field and back with Shift-Tab. Switch to Shell and see Model and Prompt disappear; switch back and see their text again.
2. Tick Worktree, leave Branch empty, press Enter. `branch name is required` appears inline and the cursor sits in Branch.
3. Set Directory to a path that is not a repository and a branch `m5/check`. Enter shows `not a git repository: ...` inline after a moment, with no toast.
4. Fix the directory to a real repository. Runtime Claude, a short prompt. Enter shows `creating the worktree…`, then the window opens focused. Claude's workspace-trust prompt for the new directory appears in the window; answer it. The tree shows `[m5/check]` after the name, under the repository's project row. The title shows the project root and `(m5/check, worktree)`.
5. The bottom bar shows `m5/check` for that window, not the branch the parent repository is on. Commit something in the parent checkout in another terminal: the worktree window's bar does not change. Then commit inside the worktree: its bar updates within a second (decision 21).
6. In another terminal, `git -C <repo> worktree list` shows the worktree under `/tmp/anthrex-m5/data/worktrees/<repo>-<hash>/m5-check`.
7. Repeat 4 with Codex on branch `m5/codex`. Answer Codex's directory trust prompt. Check that Codex's `-C` points at the worktree: ask it `pwd`.
8. Open the form again: runtime, directory and model are those of the last accepted form.
9. Ask the Claude agent to create a file. `C-b X`, tick the box, Enter. The force prompt appears; Enter does nothing. Press `n`: the agent is still running, its screen intact, and its bottom bar still updating — the refusal did not unwatch the worktree (decision 22).
10. `C-b X` again, tick, Enter, then `k`. The window is gone; the worktree directory and branch are still there. Remove it by hand with `git worktree remove --force`.
11. For the Codex window, `C-b X`, tick, Enter on a clean tree: the window and worktree go, `git branch` still lists `m5/codex`.
12. `anthrex new --runtime shell --worktree main --dir <repo>` fails with `branch 'main' is already checked out at <repo>`.
13. Paste a multi-line text into the Prompt field: it arrives on one line. Paste while the help overlay is open: nothing reaches the focused agent.
14. `./target/release/anthrex daemon stop`, then `pgrep -fl "anthrex daemon"` shows nothing. Remove `/tmp/anthrex-m5`.

## Risks and gotchas

1. **Holding the lock across git.** Milestone 1's worst bug was blocking work under the manager lock. Symptom: `a_slow_worktree_create_does_not_block_the_manager` fails, or `anthrex ls` hangs while a worktree is being created. Fix: every `crate::lock` guard in `create` and `remove_with_worktree` must be dropped in its own block before any `.await` or blocking call.
2. **Dropping a create or remove future.** Phase B runs on the blocking pool even if its future is dropped, so an aborted future leaves a worktree or a window nobody records. Symptom: stray directories under `worktrees/` after a client disconnects mid-create. Fix: the server's per-request tasks are never aborted, and the connection teardown does not abort them.
3. **Global git configuration in tests.** A developer's `core.hooksPath`, `commit.gpgsign` or `init.defaultBranch` can break temp-repo tests. `TempRepo` sets repo-local values that override them, `core.hooksPath` pinned to the repository's own hooks rather than disabled so `slow_post_checkout` still fires. If a test still fails only on one machine, compare `git config --show-origin --list` there.
4. **Git version differences.** `--path-format=absolute`, which `project::detect_roots` uses, needs git 2.31. Error wording differs between versions, which is why the daemon checks "in use" and "dirty" itself (decisions 9 and 12). Tests assert anthrex's messages, never git's. Record the local and CI git versions in "Implementation notes".
5. **Remote-only branches.** `git worktree add <path> <branch>` can create a tracking branch from a unique remote branch, but decision 10 treats a branch with no local ref as new, so it starts at `HEAD`. A user who expects the remote branch gets a fresh one. This is acceptable for this milestone; note it in the pull request.
6. **Spaces in the data directory.** On macOS the worktrees live under `~/Library/Application Support/anthrex/worktrees/`. Some build scripts break on paths with spaces. If a manual check hits this, record it as a follow-up for milestone 6 (a `worktrees_dir` config key); do not change the layout here.
7. **Restart and persistence (milestone 6).** `Entry.spec.worktree_branch` stays `Some(branch)` after creation. A future restart that re-runs `create` with that spec would try to create the worktree again and fail with "already checked out". Milestone 6 must restart from `Entry.managed` instead. Leave a `// milestone 6:` comment at the `Entry.managed` field — not at `Entry.worktree`, which is milestone 4.5's git worktree root and means something else.
8. **Agents writing during removal.** Between the dirty check and the kill, an agent can create a file. Then `git worktree remove` refuses after the agent is already dead, and the window stays listed as Exited with its root re-registered. That is the designed outcome of decision 19 step 4; the user answers the force prompt again.
9. **Trust prompts in every new worktree.** Claude and Codex treat each worktree as a new directory and ask for trust once. This is expected and answered in the window (core spec 3.2).
10. **The form and tree mode compete for keys.** If `j` typed into the Name field moves the tree selection, the modal is not taking precedence in `on_key`. The modal check must come before tree mode.
11. **One recursive watch per agent worktree.** Milestone 4.5's watch costs one inotify descriptor per directory in the checkout, ignored directories included, and this milestone multiplies that by the number of agents (amendment §9; `M4.5-git-and-tree.md`'s implementation notes). Past a host's `fs.inotify.max_user_watches` the watch silently fails to arm and the root drops to the 30-second poll, with only milestone 4.5's "watcher unavailable" warning to say so. This is a known, accepted cost here: the fix is a non-recursive watch plus a pruning walk, which shares its filter with milestone 6's `[git]` `ignore` list and is recorded against milestone 6 in `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`. Do not attempt it inside this milestone (AGENTS.md hard rule 7).

## Follow-ups handled

From `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`, "Assignment to milestones", row M5: the Task 8 minor "create() holds the Inner mutex across Window::spawn". Decision 15 moves `Window::spawn` off the lock (task M5.4).

## Implementation notes

The implementer fills this section in during implementation: every deviation, surprise and decision, with evidence.
