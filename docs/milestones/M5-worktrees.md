# Milestone 5: New-agent dialog and git worktrees

## Header

| | |
|--|--|
| Status | `blocked` |
| Depends on | Milestone 4 |
| Spec sections | Core spec 3.5 (worktrees) and 6.3 (new-agent dialog; remove confirm with worktree checkbox and force follow-up). Product spec 6 items 1, 7 and 8 (layout under the data directory, kept compatible with milestone 8's run worktrees; create and remove rules). |
| Branch | `m5-worktrees` |
| Protocol version | Unchanged. Stays at 3, the value milestone 4 set. No message changes shape. (Product spec 10.1: only a milestone that changes a message shape bumps `PROTO_VERSION`, to one more than the value on `main`.) |

## Starting point

Milestones 3 and 4 are merged before this one starts. This brief uses the names of `docs/milestones/M3-agent-status.md` and `docs/milestones/M4-project-tree.md`:

- Hooks: `anthrex hook`, `ClientMsg::HookEvent` handled, the Claude and Codex launch flags in `crates/daemon/src/launch/`, process-group kill. `WindowManager::remove` sends SIGKILL to the child's process group at once (M3 decision 47).
- `WindowManager::new(config: ManagerConfig)`, where `ManagerConfig` holds `socket_path`, `shell`, `exe`, `claude_bin`, `codex_bin` and `codex_hook_source`.
- `WindowInfo.session_id: Option<String>` in place of `has_session`, plus `WindowInfo.model`, `WindowInfo.subagents` and `WindowInfo.project`.
- `proto::PROTO_VERSION == 3`.
- `crates/daemon/src/project.rs` with `detect_root` and `resolve_root`. The server's `CreateWindow` arm already runs in a spawned task: it awaits `project::resolve_root(spec.cwd)`, then calls `WindowManager::create(spec, project, cols, rows)` and sends `Created` or `Error { request: "create" }` through a clone of `out_tx` (M4 decisions 6 and 7).
- The project tree in `crates/tui/src/tree.rs`, drawn by `crates/tui/src/ui/tree_view.rs` (`narrow_line` for the sidebar, `wide_line` for the overview), tree mode (`C-b t`) in `crates/tui/src/tree_input.rs`, and the overview (`C-b T`). `App`'s tests live in `crates/tui/src/app_tests.rs`.
- `crates/fake-agent` and the `ANTHREX_CLAUDE_BIN` and `ANTHREX_CODEX_BIN` overrides.

Names in this brief that come from milestone 1 are real and checked against `main`. If the merged code of milestones 3 and 4 differs from the names above, use the real names and record the mapping in "Implementation notes".

## Goal

`C-b c` opens a form to create an agent: runtime, name, directory, an optional git worktree on a branch, model and first prompt. With the worktree ticked, the daemon creates a linked worktree under the data directory and starts the agent inside it, so parallel agents never edit the same checkout. The tree shows the branch next to worktree windows, grouped under their repository's project. Removing a window offers to remove its worktree too. If git refuses because the tree has changes, the user chooses to force, to keep the worktree, or to cancel. The CLI does the same with `anthrex new --worktree <branch>` and `anthrex rm --worktree [--force]`.

## Scope

In:

- A daemon `git` runner with a deadline and process-group kill, and a `worktree` module that creates, checks and removes worktrees.
- `WindowManager::create` runs every blocking step (git, directory checks, `Window::spawn`) off the manager lock.
- Worktree cleanup when window creation fails after the worktree was made.
- Removal of a window's worktree on request, with dirty-tree detection and a force path.
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

## Design decisions

### Git and worktree layout

1. **Git runner.** New module `crates/daemon/src/git.rs`. Every git call in the daemon goes through `git::run`. It spawns `git` with `-C <dir>`, stdin from `/dev/null`, stdout and stderr piped, in its own process group (`std::os::unix::process::CommandExt::process_group(0)`), and with the environment variables `LC_ALL=C`, `GIT_TERMINAL_PROMPT=0` and `GIT_OPTIONAL_LOCKS=0` added and `GIT_DIR`, `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE` and `GIT_PREFIX` removed, as milestone 4 does for project detection. Milestone 4's `project.rs` keeps its own git call: it has a 5-second timeout and an injectable program for its tests, and it never fails.
2. **Deadline.** Each worktree operation (one create, one removal) gets one deadline, `Instant::now() + git::GIT_TIMEOUT`, with `GIT_TIMEOUT = 30 s`, shared by all the git commands it runs. A command still running at the deadline is killed with `SIGKILL` to its process group (`libc::killpg`), which also kills git hooks, and returns `GitError::TimedOut`.
3. **Off the async runtime.** `git::run` and everything in `worktree` are blocking functions. The manager calls them only inside `tokio::task::spawn_blocking`. No git runs on a tokio worker or while `WindowManager`'s `inner` mutex is held.
4. **Repository root.** `worktree::repo_root(dir)` first runs `git rev-parse --show-toplevel` in `dir`. Failure means "not a git repository", which also covers bare repositories. It then returns the canonicalized parent of `git rev-parse --path-format=absolute --git-common-dir`, which is the main checkout even when `dir` is inside a linked worktree. When the common directory's last component is not `.git`, as inside a submodule, it returns the `--show-toplevel` result instead. This is the same root milestone 4 uses for projects (product spec 3 rule 2). `--path-format` needs git 2.31 or later.
5. **Repository worktree directory.** `<wt> = <data_dir>/worktrees/<repo_basename>-<hash8>`. `repo_basename` is the root's last path component with every character outside `[A-Za-z0-9._-]` replaced by `-`. `hash8` is the 32-bit FNV-1a hash of the root path's bytes (`OsStr::as_bytes`), written as 8 lowercase hex digits. Test vectors: `""` gives `811c9dc5`, `"a"` gives `e40c292c`, `"/tmp/repo"` gives `a3cbe2c8`. Milestone 8 reuses `<wt>` unchanged. Never use `std::hash::DefaultHasher`; its output is not stable across Rust releases.
6. **Worktree path.** `<wt>/<branch with every "/" replaced by "-">`. The resulting directory name `runs` is rejected because milestone 8 owns `<wt>/runs/`. If the path already exists, creation fails with `worktree path already exists: <path>`. This also catches `feat/x` against an existing `feat-x`.
7. **Branch rules**, checked by the daemon in this order, each with the exact message in the Interfaces section: not empty after trimming; no whitespace; does not start with `-`; does not start with `anthrex/` (reserved for runs); `git check-ref-format --branch <branch>` exits 0 and prints the branch unchanged. The client checks the first four itself for fast feedback.
8. **Branch in use.** Before `worktree add`, the daemon reads `git worktree list --porcelain`. If any entry has `branch refs/heads/<branch>`, creation fails with `branch '<branch>' is already checked out at <path>`. This covers the branch checked out in the main checkout. Verified against git 2.50.1: `git worktree add` refuses the same case with `'<branch>' is already used by worktree at '<path>'`, but the wording differs between git versions, so the daemon checks first.
9. **Create.** `worktree::create` checks in this order and stops at the first failure: branch syntax (decision 7, rules 1 to 4), repository root (decision 4), `check-ref-format`, the reserved `runs` directory name, branch in use (decision 8), path exists (decision 6). Then, if `git show-ref --verify --quiet refs/heads/<branch>` exits 0, run `git worktree add <path> <branch>`. Otherwise run `git worktree add -b <branch> <path>`, which starts the branch at `HEAD` of the directory the user chose, and remember `created_branch = true`. All of these run with `-C <the user's directory>`. `create_dir_all(<wt>)` runs first.
10. **Window directory.** The window's child runs in the worktree root, whatever subdirectory of the repository the user chose. The entry's `spec.cwd` is replaced by the worktree path before `launch::plan`, so `WindowInfo.cwd`, the PTY's cwd and Codex's `-C` all name the worktree.
11. **Dirty check.** `worktree::is_dirty(path)` runs `git status --porcelain --ignore-submodules=none` in the worktree and returns true when stdout is not empty. Verified against git 2.50.1: untracked files make it dirty and make a plain `git worktree remove` fail with `contains modified or untracked files, use --force to delete it`. Ignored files do not count and are deleted by a plain remove.
12. **Remove.** `worktree::remove(wt, force)`: if `wt.path` does not exist, run `git worktree prune` in `wt.repo_root` and succeed. Otherwise run `git worktree remove [--force] <path>` in `wt.repo_root`. When that fails without `--force` and `is_dirty` now returns true, return `WorktreeError::Dirty`; any other failure returns `WorktreeError::Git` with git's stderr. The branch is never deleted.
13. **Error text.** Git failures carry the last 20 lines of stderr, trimmed, at most 1000 characters, with `fatal: ` prefixes kept. The client shows the message as given.

### Creating windows

14. **Three phases.** `WindowManager::create` becomes `async`:
    - Phase A, under the lock: resolve the name (unchanged rule: trimmed, else `<runtime>-<id>`), reject it if an entry or a reservation already has it, allocate the id (`next_id += 1` now; an id lost to a failed create is not reused), insert the name into `reserved_names`, release the lock. A `Reservation` guard removes the name again in `Drop`, on every exit path.
    - Phase B, in one `spawn_blocking` call, no lock: canonicalize and check the directory; if a branch was given, `worktree::create`; build the `LaunchPlan`; `Window::spawn`. The project root arrives as `create`'s parameter, resolved by the server before the call (milestone 4); for a worktree window it is the same main checkout.
    - Phase C, under the lock: insert the entry with its `worktree`, drop the reservation, publish.
15. **Cleanup on failure.** If phase B fails after `worktree::create` succeeded, it calls `worktree::discard_new` before returning: `git worktree remove --force <path>`, then `git worktree prune`, then `git branch -D <branch>` only when `created_branch` is true, with a fresh 10-second deadline. The error returned to the client is the original one with `; the new worktree was removed` appended. If cleanup itself fails, log it at `warn` and append `; cleanup failed: <reason>` instead. Deleting a branch that this same create made moments ago at `HEAD` loses nothing, so this is not the "branch is never deleted" case, which is about removal.
16. **A create is never cancelled half-way.** Once started, phase B runs to completion on the blocking pool even if the caller's future is dropped, so the server must never drop or abort a create future. See decision 21.

### Removing windows

17. **Removal is offered, never automatic.** `ClientMsg::Remove { remove_worktree: false }` removes the window exactly as today and leaves any worktree on disk. Kill (`C-b x`, `anthrex kill`) never touches worktrees. The TUI checkbox starts unticked.
18. **Removal with the worktree** is a new `WindowManager::remove_with_worktree(id, force)`, `async`, in this order:
    1. Under the lock: the entry must exist, must have a worktree (else `window '<name>' has no worktree`), and must not already be removing (else `window '<name>' is already being removed`). Set `removing = true`, clone the `Worktree`, release the lock.
    2. Without `force`: `is_dirty` on the blocking pool. If dirty, clear `removing` and return `RemoveError::Dirty`. The agent keeps running.
    3. If the window is not Exited, signal it the way `remove` does: SIGKILL to the process group. Poll `list()` every 25 ms, for at most `KILL_GRACE`, until it is Exited. Never hold the lock while waiting.
    4. `worktree::remove(wt, force)` on the blocking pool. On `Dirty` (a file appeared between the check and the kill) or any other error, clear `removing` and return the error. The window stays listed, Exited, so the user can retry.
    5. Under the lock: remove the entry and publish.
19. **`force` without `remove_worktree`** is rejected with `--force only applies when removing the worktree`.

### Protocol and server

20. **Dirty-tree signal without a protocol change.** `DaemonMsg::Error.request` is already a free string. The daemon answers a worktree removal refused for changes with `request = "remove-dirty"` and the `WorktreeError::Dirty` text as `message`. Every other outcome uses the milestone-1 values: `Ack { request: "remove" }`, `Error { request: "remove" }`, `Created`, `Error { request: "create" }`. The strings become constants in `proto::messages::request`. No message changes shape, so `PROTO_VERSION` stays 3. An older client would only show the text as a toast.
21. **Long requests run beside the connection loop.** In `server::handle_client`, `CreateWindow` and `Remove` are each handled in their own `tokio::spawn`ed task, which sends its reply through a clone of `out_tx`. Milestone 4 already runs `CreateWindow` this way; this milestone adds `.await` on the now async `create` inside that task and does the same for `Remove`. The loop goes straight on to the next frame, so input to other windows and `ListWindows` are never stuck behind a 30-second git call. These tasks are never aborted (decision 16). Their reply send is best effort: if the client has gone, the result is only logged.

### Client

22. **`C-b c` opens the form.** `Command::NewWindow` keeps its name and binding but no longer sends `CreateWindow` directly. It sets `app.modal = Some(Modal::NewAgent(NewAgentForm::new(&app.form_defaults)))`.
23. **The form is a pure model.** New file `crates/tui/src/dialog.rs` holds `TextInput`, `NewAgentForm`, `RemoveConfirm` and their key handling. No I/O, no clock, no filesystem. It follows `AGENTS.md` rule 5 like `app.rs`.
24. **Fields, in order:** Runtime, Name, Directory, Worktree (checkbox), Branch (shown only when Worktree is ticked), Model and Prompt (hidden when Runtime is Shell). Focus starts on Runtime. Hidden fields keep their text but are neither focusable nor sent.
25. **Keys in the form:**

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
26. **Pastes** go to the focused text field with `\r\n`, `\r` and `\n` replaced by a space. While any other modal is open, a paste is dropped, never sent to the PTY.
27. **Defaults.** The form opens with the runtime, directory text and model of the last form that the daemon accepted in this client session. On the first open: Claude, the default directory shown with the home prefix as `~` (`ui::terminal::shorten_home`), and an empty model. Name, branch and prompt always start empty and the worktree unticked. The Name field shows the placeholder `automatic (<runtime>-N)` while empty.
28. **Client-side validation** on Enter. The first failure sets `form.error` and moves focus to the failing field:
    - Directory: empty after trimming gives `directory is required`. `~` and `~/...` expand against `App.home_dir`. `~user` gives `only ~ and ~/ are expanded`. `~` with no known home gives `cannot expand ~: home directory unknown`. A relative path is joined onto `App.default_dir`.
    - Name: if not empty after trimming, at most 64 characters, no control characters (`name must be at most 64 characters`, `name must not contain control characters`), and not the name of a window in `app.windows` (`a window named '<name>' already exists`, the daemon's own wording).
    - Branch, when ticked: the first four rules of decision 7, with the same messages.
    - Model and prompt: trimmed; empty means `None`.
29. **Filesystem checks happen in the daemon, not the client.** The directory's existence, "is a git repository", the branch being in use, `check-ref-format`, and every git failure come back as `DaemonMsg::Error { request: "create" }`. While the form is `submitting`, the app shows that message inline as `form.error` and clears `submitting`, and no toast is shown. Focus stays where it was.
30. **Submission.** A valid Enter sets `submitting = true` and returns `Effect::Send(ClientMsg::CreateWindow { spec, cols, rows })` with the current terminal size, as `Command::NewWindow` does today. `Created` closes the form, stores the form's runtime, directory text and model in `app.form_defaults`, and focuses the window through the existing `Created` path. `Esc` while submitting closes the form; the create still completes and `Created` still focuses the new window.
31. **Remove confirm.** `C-b X` opens `Modal::Remove(RemoveConfirm)`. The checkbox line appears only when the window's `branch` is `Some`. `Space` or `w` toggles it, `y` or `Enter` confirms, `n` or `Esc` cancels. Confirming sends `Remove { window_id, remove_worktree, force: false }`. When `remove_worktree` is true, the app records `pending_worktree_remove = Some(window_id)`.
32. **Force follow-up.** `Error { request: "remove-dirty" }` while `pending_worktree_remove` is `Some(id)` opens `Modal::ForceRemove { window_id: id, name, message }`. `f` sends `Remove { remove_worktree: true, force: true }` and keeps `pending_worktree_remove`. `k` sends `Remove { remove_worktree: false, force: false }`, which removes the window and keeps the worktree. `n` or `Esc` cancels; the window stays as it is. `Enter` does nothing here, so a destructive choice is never one reflexive key away. `Ack { request: "remove" }` or `Error { request: "remove" }` clears `pending_worktree_remove`; the error is shown as a toast.
33. **Branch display.** `WindowInfo.branch` is `Some` exactly for worktree windows, set from `Worktree.branch`. The tree's agent row shows it after the name as `[<branch>]` in the muted style. When space is short, the branch shrinks first, ending in `…`, and is dropped when fewer than 4 columns remain for it. The name keeps at least 8 columns. The overview (`C-b T`) shows the branch in full. The main pane title for a worktree window reads ` <name> · <runtime> · <shortened project root> (<branch>, worktree) `, using `WindowInfo.project`, instead of the long data-directory path.

### CLI

34. `anthrex new --worktree <branch>` sends the branch in `WindowSpec.worktree_branch`. `anthrex rm <target> --worktree [--force]` sends `Remove` with those flags; clap rejects `--force` without `--worktree`. Both wait up to `GIT_REQUEST_TIMEOUT = 45 s` for the reply: 30 s of git, 3 s of kill grace, and margin. Other commands keep 5 s.
35. `anthrex rm` of a worktree window without `--worktree` prints to stderr `kept worktree <cwd> on branch <branch>`. A `remove-dirty` refusal prints the daemon's message followed by `run 'anthrex rm <target> --worktree --force' to discard the changes, or 'anthrex rm <target>' to keep the worktree` and exits 1.
36. `anthrex ls` gains a `BRANCH` column between `STATUS` and `DIR`, with `-` for windows without a worktree.

## Interfaces

### `crates/daemon/src/git.rs` (new)

```rust
pub const GIT_TIMEOUT: Duration = Duration::from_secs(30);

pub struct GitOutput {
    pub status: std::process::ExitStatus,
    pub stdout: String,   // lossy UTF-8
    pub stderr: String,   // lossy UTF-8
}

impl GitOutput {
    pub fn success(&self) -> bool;
    /// Last 20 lines of stderr, trimmed, at most 1000 characters.
    pub fn stderr_tail(&self) -> String;
}

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git is not installed or not on PATH")]
    NotFound,
    #[error("git {args} timed out after {secs} s")]
    TimedOut { args: String, secs: u64 },
    #[error("could not run git: {0}")]
    Io(#[from] std::io::Error),
}

/// Runs `git -C <dir> <args...>` and waits until it exits or `deadline` passes.
/// Blocking: call it only from `spawn_blocking` or a dedicated thread.
pub fn run(dir: &Path, args: &[&str], deadline: Instant) -> Result<GitOutput, GitError>;
```

Implementation shape: spawn the child, remember its pid, hand the `Child` to a helper thread that calls `wait_with_output()` and sends the result over a `std::sync::mpsc` channel, then `recv_timeout(deadline - now)`. On timeout, `killpg(pid, SIGKILL)` and then receive the helper's result so the child is reaped. `ErrorKind::NotFound` on spawn maps to `GitError::NotFound`. Add `thiserror.workspace = true` to `crates/daemon/Cargo.toml`.

### `crates/daemon/src/worktree.rs` (new)

```rust
/// Directory name under `<wt>` that milestone 8 owns.
pub const RESERVED_DIR: &str = "runs";
/// Branch prefix that milestone 8 owns.
pub const RESERVED_BRANCH_PREFIX: &str = "anthrex/";
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub repo_root: PathBuf, // main checkout, canonical (decision 4)
    pub path: PathBuf,      // the linked worktree
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub worktree: Worktree,
    pub created_branch: bool,
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
    #[error(transparent)]
    Run(#[from] crate::git::GitError),
}

pub fn hash8(path: &Path) -> String;
pub fn branch_dir_name(branch: &str) -> String;                 // "/" -> "-"
pub fn repo_worktrees_dir(worktrees_root: &Path, repo_root: &Path) -> PathBuf; // <wt>
/// Decision 7, rules 1 to 4. Pure. `crates/tui/src/dialog.rs` has its own copy with the same messages.
pub fn check_branch_syntax(branch: &str) -> Result<(), WorktreeError>;
pub fn repo_root(dir: &Path, deadline: Instant) -> Result<PathBuf, WorktreeError>;
pub fn create(dir: &Path, branch: &str, worktrees_root: &Path, deadline: Instant) -> Result<Created, WorktreeError>;
pub fn is_dirty(path: &Path, deadline: Instant) -> Result<bool, WorktreeError>;
pub fn remove(wt: &Worktree, force: bool, deadline: Instant) -> Result<(), WorktreeError>;
/// Decision 15. Uses its own `CLEANUP_TIMEOUT` deadline.
pub fn discard_new(created: &Created) -> Result<(), WorktreeError>;
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
    // existing fields, plus:
    worktree: Option<Worktree>,
    removing: bool,
}

struct Inner {
    next_id: u32,
    entries: BTreeMap<u32, Entry>,
    reserved_names: BTreeSet<String>, // new: names of creates in phase B
}

pub struct ManagerConfig {
    // milestone 3's fields, plus:
    /// `<data_dir>/worktrees`. `ManagerConfig::new` and `from_env` set
    /// `std::env::temp_dir().join("anthrex-worktrees")`; `lifecycle::run` overrides it
    /// with `opts.data_dir.join("worktrees")`, and tests set a `TempDir`.
    pub worktrees_root: PathBuf,
}

impl WindowManager {
    /// Unchanged signature: `new(config: ManagerConfig)`.

    /// Now async; decision 14. `project` is milestone 4's parameter, resolved by the server.
    pub async fn create(&self, spec: WindowSpec, project: PathBuf, cols: u16, rows: u16) -> anyhow::Result<WindowInfo>;

    /// Unchanged: removes the window, keeps any worktree.
    pub fn remove(&self, id: u32) -> anyhow::Result<()>;

    /// New; decision 18.
    pub async fn remove_with_worktree(&self, id: u32, force: bool) -> Result<(), RemoveError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    #[error("{0}")]
    Dirty(WorktreeError),        // always WorktreeError::Dirty
    #[error("{0}")]
    Failed(#[from] anyhow::Error),
}
```

`Entry::info()` sets `branch: self.worktree.as_ref().map(|w| w.branch.clone())`. `rename` also rejects names in `reserved_names`. Delete `daemon::WORKTREE_UNSUPPORTED` from `crates/daemon/src/lib.rs` and the `spec.worktree_branch.is_some()` rejection in `create`. Add `pub mod git;` and `pub mod worktree;` to `lib.rs`.

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

The server uses these constants instead of the literals `"create"` and `"remove"`.

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
/// Client copy of daemon decision 7 rules 1 to 4, same messages.
pub fn check_branch_syntax(branch: &str) -> Result<(), String>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveConfirm {
    pub window_id: u32,
    pub name: String,
    pub branch: Option<String>,
    pub remove_worktree: bool,
}
```

Name limit constant: `pub const NAME_MAX_CHARS: usize = 64;`.

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

- Width `min(66, area.width - 2)`, labels 11 columns, focused label in the accent colour with `›`. The error line appears only when `error` is set, in the attention colour, wrapped to at most 3 lines. While `submitting`, the hint line reads `creating…` or, with the worktree ticked, `creating the worktree… (up to 30 s)`.
- The hardware cursor goes to the focused text field's cursor (`frame.set_cursor_position`). The modal draws after the terminal pane, so this overrides the PTY cursor.

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

`crates/cli/src/client.rs` gains `pub async fn request_with_timeout(&mut self, msg: ClientMsg, timeout: Duration) -> anyhow::Result<DaemonMsg>`; `request` calls it with 5 s. `crates/cli/src/main.rs` defines `const GIT_REQUEST_TIMEOUT: Duration = Duration::from_secs(45);`.

## Tasks

Shared test helper for M5.2 to M5.5: new file `crates/daemon/tests/common/mod.rs` with `TempRepo`. `TempRepo::new()` makes a `tempfile::TempDir`, runs `git init -b main`, sets repo-local `user.name`, `user.email`, `commit.gpgsign false` and `core.hooksPath .git/hooks` so the developer's global config cannot interfere, commits a `README`, and stores the canonical root. Methods: `git(&self, args) -> String` (panics with stderr on failure), `branch_exists(&str) -> bool`, `worktree_paths() -> Vec<PathBuf>` from `git worktree list --porcelain`, and `slow_post_checkout(secs)`, which writes an executable `.git/hooks/post-checkout` that touches `<root>/.hook-started` and then sleeps. Each test file that uses it declares `mod common;`.

### M5.1 Git runner and pure layout helpers

**Files.** Create `crates/daemon/src/git.rs`, `crates/daemon/src/worktree.rs` (pure parts only). Modify `crates/daemon/src/lib.rs`, `crates/daemon/Cargo.toml`.

**Tests first.** Unit tests inside the two modules.

- `git::tests::run_returns_stdout_and_status`: `run(tmp, &["--version"], now + 5 s)` succeeds and stdout starts with `git version`.
- `git::tests::run_in_a_non_repo_reports_failure_with_stderr`: `run(tempdir, &["rev-parse", "--show-toplevel"], ...)` returns `Ok` with `success() == false` and `stderr_tail()` containing `not a git repository`.
- `git::tests::a_timed_out_command_is_killed_with_its_process_group`: run `git -c 'alias.slow=!echo $$ > <tmp>/pid; exec sleep 30' slow` with a deadline of 1 s. Assert `GitError::TimedOut` within 2 s of the start. Then read the pid and assert, in a deadline loop of up to 2 s, that `libc::kill(pid, 0)` fails with `ESRCH`.
- `git::tests::stderr_tail_keeps_the_last_20_lines_and_1000_chars`: build a `GitOutput` with 50 numbered stderr lines; the tail starts at line 31. A single 5000-character line is cut to 1000.
- `worktree::tests::hash8_matches_fnv1a_vectors`: the three vectors from decision 5.
- `worktree::tests::branch_dir_name_replaces_slashes`: `feat/api/v2` gives `feat-api-v2`.
- `worktree::tests::repo_worktrees_dir_uses_sanitized_basename_and_hash`: root `/tmp/my repo` gives `<root>/worktrees/my-repo-<hash8("/tmp/my repo")>`.
- `worktree::tests::branch_syntax_rules_and_messages`: each row of the branch message table except the last two returns exactly its message; `feat/api` passes.

**Change.** Implement decisions 1, 2, 5, 6 (naming only) and 7 (syntax rules), and the `GitError`, `GitOutput`, `hash8`, `branch_dir_name`, `repo_worktrees_dir` and `check_branch_syntax` interfaces. Milestone 4's `project.rs` is not touched.

**Acceptance.** All new tests pass. `cargo tree -p anthrex-daemon` shows no new dependency besides `thiserror`.

### M5.2 Worktree create, remove, dirty check and cleanup

**Files.** Modify `crates/daemon/src/worktree.rs`. Create `crates/daemon/tests/common/mod.rs`, `crates/daemon/tests/worktree.rs`.

**Tests first.** All in `crates/daemon/tests/worktree.rs`, each with a fresh `TempRepo` and a `tempfile::TempDir` as `worktrees_root`, deadline `now + GIT_TIMEOUT` unless stated.

- `repo_root_is_the_main_checkout_even_from_a_linked_worktree`: `repo_root(repo.root)` and `repo_root(repo.root/sub)` both equal `repo.root`. After `git worktree add ../other -b other`, `repo_root(other)` also equals `repo.root`.
- `repo_root_of_a_plain_directory_is_not_a_repo`: a tempdir gives `NotARepo`, and its message starts with `not a git repository: `.
- `create_with_a_new_branch`: `create(root, "feat/new", ...)` returns `created_branch == true`, a path equal to `<wt>/feat-new` where `<wt> = repo_worktrees_dir(worktrees_root, root)`, the directory contains `README`, `branch_exists("feat/new")`, and `git -C <path> rev-parse --abbrev-ref HEAD` prints `feat/new`.
- `create_with_an_existing_branch`: `git branch existing` first; `create` returns `created_branch == false` and the worktree is on `existing`.
- `create_from_a_subdirectory_still_uses_the_repo`: `create(root/sub, "b", ...)` succeeds and the worktree path is the same as from `root`.
- `a_branch_checked_out_elsewhere_fails_cleanly`: create `taken` once, then again. The second returns `BranchInUse` naming the first worktree's path. Also `create(root, "main", ...)` returns `BranchInUse` naming `repo.root`. After both, `worktree_paths()` has exactly two entries (root and the first worktree), the first worktree still contains `README`, and `<wt>` contains only `taken`.
- `an_existing_path_is_refused`: create the directory `<wt>/feat-x` by hand; `create(root, "feat/x", ...)` returns `PathExists`, `branch_exists("feat/x")` is false, and `worktree_paths()` has one entry.
- `invalid_and_reserved_branches_are_refused_before_worktree_add`: `"bad name"`, `"-x"`, `"anthrex/r/t1"`, `"runs"` and `"a..b"` each give the matching message from the table. `worktree_paths()` still has one entry.
- `is_dirty_sees_modified_and_untracked_but_not_ignored_files`: fresh worktree is clean; a file matching a committed `.gitignore` leaves it clean; an untracked file makes it dirty; after deleting that, modifying `README` makes it dirty.
- `remove_clean_worktree_keeps_the_branch`: `remove(wt, false)` succeeds, the path is gone, `branch_exists` is still true.
- `remove_dirty_worktree_is_refused_then_forced`: add `untracked.txt`. `remove(wt, false)` returns `Dirty` with the path and the directory still exists. `remove(wt, true)` succeeds, the path is gone, the branch still exists.
- `remove_of_a_missing_path_prunes`: delete the worktree directory by hand; `remove(wt, false)` succeeds and `worktree_paths()` has one entry.
- `discard_new_deletes_only_a_branch_it_created`: after `create` of a new branch, `discard_new` removes the path and the branch. After `create` on a pre-existing branch, `discard_new` removes the path and keeps the branch.
- `a_timeout_during_create_cleans_up`: `slow_post_checkout(10)`, deadline `now + 1 s`. `create` returns `Run(TimedOut)` within 3 s. Afterwards `worktree_paths()` has one entry, the target directory does not exist, and the new branch does not exist.

**Change.** Implement decisions 4, 6, 7 (the `check-ref-format` and `runs` rules), 8, 9, 11, 12, 13 and 15 (`discard_new`). `create` calls `discard_new` itself when a git step after `worktree add` started fails or times out, including the timeout case above, before returning the original error.

**Acceptance.** All tests pass on macOS and Linux with the git on the machine. Record the git version in "Implementation notes".

### M5.3 Manager create off the lock, with worktrees

**Files.** Modify `crates/daemon/src/manager.rs`, `crates/daemon/src/lib.rs`, `crates/daemon/src/lifecycle.rs`, `crates/daemon/src/server.rs` (only `.await` on `create`), `crates/daemon/tests/manager.rs`, `crates/daemon/tests/server.rs`, `crates/tui/tests/connection.rs` and every other caller of `WindowManager::new` or `create`.

**Tests first.** In `crates/daemon/tests/manager.rs`. The `manager()` helper gains a `TempDir` for `ManagerConfig.worktrees_root` and returns it alongside the manager so it lives as long as the test. Existing tests switch to `m.create(spec, project, cols, rows).await`; a plain window's project is the `project::detect_root` of its directory or, where nothing depends on it, `std::env::temp_dir()`, as in milestone 4's tests. A worktree window's project is `repo.root`.

- Delete `a_spec_asking_for_a_worktree_is_rejected`.
- `a_worktree_window_runs_in_its_worktree`: `TempRepo`; spec with `cwd = repo.root`, `worktree_branch = Some("feat/wt")`. The returned info has `branch == Some("feat/wt")` and `cwd == <wt>/feat-wt`. Send `pwd\n` and wait until the snapshot contains `feat-wt`. The window without a branch still reports `branch == None`.
- `worktree_windows_group_under_their_repository_project`: create one plain window in `repo.root` and one worktree window. Both infos have the same `project`, equal to `repo.root`.
- `create_errors_come_back_before_any_window_exists`: a non-repo directory with a branch fails with `not a git repository`; a missing directory fails with `directory does not exist`; a branch checked out in the main checkout fails with `is already checked out at`. After each, `m.list()` is empty and `worktree_paths()` has one entry.
- `a_failed_spawn_removes_the_new_worktree`: a manager whose shell is `/nonexistent/anthrex-test-shell`. Create a Shell window with `worktree_branch = Some("doomed")`. The error ends with `the new worktree was removed`; `worktree_paths()` has one entry and `branch_exists("doomed")` is false. `Window::spawn` failures are still returned as errors after milestones 3 and 4, so this path exists.
- `a_slow_worktree_create_does_not_block_the_manager`: `slow_post_checkout(2)`. Start the worktree create in a `tokio::spawn`. Wait with a deadline for `.hook-started`. Then assert `m.list()` returns in under 100 ms, and a plain Shell `create` for another name completes in under 1 s. Finally the worktree create succeeds.
- `a_name_is_reserved_while_its_create_is_in_flight`: `slow_post_checkout(2)`; start a worktree create named `dup`; once `.hook-started` exists, a plain create named `dup` fails with `already exists`, and `m.rename(other_id, "dup")` fails too. After the first create finishes, it is listed as `dup`.

**Change.** Implement decisions 10, 14, 15 and 16 and the manager interface. Remove `WORKTREE_UNSUPPORTED` from `crates/daemon/src/lib.rs` and `manager.rs`, and remove the `if worktree.is_some() { anyhow::bail!(daemon::WORKTREE_UNSUPPORTED) }` block in `crates/cli/src/main.rs`, which would no longer compile. `lifecycle::run` sets `worktrees_root` to `opts.data_dir.join("worktrees")`. In `server.rs`, the `CreateWindow` task from milestone 4 now awaits `create`.

**Acceptance.** `grep -rn WORKTREE_UNSUPPORTED crates` finds nothing. All tests pass.

### M5.4 Removing a window with its worktree

**Files.** Modify `crates/daemon/src/manager.rs`, `crates/daemon/tests/manager.rs`.

**Tests first.** In `crates/daemon/tests/manager.rs`, each with a `TempRepo` and a worktree Shell window that has reached Working.

- `remove_with_worktree_removes_a_clean_tree_and_keeps_the_branch`: `remove_with_worktree(id, false)` succeeds; the window is not listed; the worktree directory is gone; the branch exists.
- `a_dirty_tree_is_refused_and_the_agent_keeps_running`: `touch dirty.txt` through `write_input`, then wait until the file exists. `remove_with_worktree(id, false)` returns `RemoveError::Dirty`, whose message contains the worktree path and `uncommitted or untracked`. The window is still listed and not Exited, and `write_input` still reaches it (send `echo still-here` and see it in the snapshot).
- `force_removes_a_dirty_tree`: same setup, then `remove_with_worktree(id, true)` succeeds; the window is gone and so is the directory, the branch is kept.
- `plain_remove_keeps_the_worktree`: `m.remove(id)` succeeds and the worktree directory and branch both still exist.
- `remove_with_worktree_rejects_windows_without_one_and_double_removal`: a plain window gives `has no worktree` and stays listed. For a clean worktree window, run two `remove_with_worktree(id, false)` calls at once with `tokio::join!`: exactly one succeeds, and the other fails with `already being removed` or, if the first had already finished, `no window with id`.
- `an_exited_window_is_removed_without_waiting`: `write_input(id, b"exit\n")`, wait for Exited, then `remove_with_worktree(id, false)` completes in under 1 s.

**Change.** Implement decisions 17, 18 and 19 at the manager level (`RemoveError`, `removing`). Decision 19's message is produced by the server in M5.5, because `remove` itself has no `force` argument.

**Acceptance.** All tests pass. No test takes longer than 5 s.

### M5.5 Server: requests in tasks, `remove-dirty`, request constants

**Files.** Modify `crates/proto/src/messages.rs`, `crates/daemon/src/server.rs`, `crates/daemon/tests/server.rs`.

**Tests first.**

- `proto::messages::tests::request_constants_are_stable`: the three constants equal `create`, `remove` and `remove-dirty`.
- In `crates/daemon/tests/server.rs` (the `start_daemon` helper gains a `worktrees_root` inside its `TempDir`):
  - `create_with_a_worktree_over_the_socket`: `CreateWindow` with a branch in a `TempRepo` returns `Created`; the next `WindowsChanged` lists the window with that branch.
  - `create_errors_use_the_create_request`: a non-repo directory with a branch returns `Error { request: "create", message }` with `not a git repository` in `message`.
  - `list_is_answered_while_a_create_is_running`: `slow_post_checkout(3)`. Send `CreateWindow` with a branch, wait for `.hook-started`, then send `ListWindows`. A `WindowsChanged` arrives within 500 ms, before `Created`. `Created` arrives later.
  - `a_dirty_worktree_removal_answers_remove_dirty_then_force_works`: create a worktree Shell window, make it dirty through `Input`, send `Remove { remove_worktree: true, force: false }`: the reply is `Error { request: "remove-dirty" }` with the path in `message`. Send it again with `force: true`: the reply is `Ack { request: "remove" }` and the window leaves the list.
  - `force_without_worktree_is_rejected`: `Remove { remove_worktree: false, force: true }` returns `Error { request: "remove", message: "--force only applies when removing the worktree" }` and the window stays.

**Change.** Implement decisions 20 and 21. `ClientMsg::Remove` dispatch: `force && !remove_worktree` is the error above; `!remove_worktree` calls `manager.remove(id)` inline as today; otherwise a task calls `remove_with_worktree` and maps `RemoveError::Dirty` to `REMOVE_DIRTY`. `CreateWindow` stays in milestone 4's task and uses the `CREATE` constant. Keep the rule that `Snapshot` is queued before live output; it is unaffected because subscriptions stay in the loop.

**Acceptance.** All tests pass, including every milestone-1 server test unchanged except for the helper.

### M5.6 CLI: `new --worktree`, `rm --worktree [--force]`, `ls` branch column

**Files.** Modify `crates/cli/src/main.rs`, `crates/cli/src/client.rs`.

**Tests first.** Unit tests in `crates/cli/src/main.rs` (`#[cfg(test)] mod tests`, using `Cli::try_parse_from`) and `crates/cli/src/client.rs`.

- `rm_force_requires_worktree`: `["anthrex", "rm", "x", "--force"]` fails to parse; `["anthrex", "rm", "x", "--worktree", "--force"]` parses with both flags set.
- `new_accepts_a_worktree_branch`: `["anthrex", "new", "--runtime", "shell", "--worktree", "feat/x"]` parses with `worktree == Some("feat/x")`.
- `table_has_a_branch_column` (replaces the assertions of `table_has_header_and_one_row_per_window`): the header is `ID NAME RUNTIME STATUS BRANCH DIR` with the existing padding; a window with `branch: Some("feat/x")` shows it; one without shows `-`.
- `dirty_hint_names_both_commands`: a new pure function `dirty_hint(target: &str) -> String` returns exactly the second sentence of decision 35.

**Change.** Implement decisions 34, 35 and 36 and `request_with_timeout`. Update the `--worktree` help text to `Create a git worktree on this branch and start the window in it`. `rm` matches `Error { request: "remove-dirty" }` to print the message and the hint and exit 1; `anyhow::bail!` of the combined text is enough.

**Acceptance.** Tests pass. The smoke stages in M5.10 exercise the CLI end to end.

### M5.7 The new-agent form model

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

**Change.** Implement decisions 23 to 28 in `dialog.rs` and the interface above, plus `RemoveConfirm` (data only; its keys live in `app.rs`).

**Acceptance.** Tests pass. `dialog.rs` has no `use` of `std::fs`, `std::process`, `tokio`, `dirs` or `std::time`.

### M5.8 App: open, submit and close the form

**Files.** Modify `crates/tui/src/app.rs`, `crates/tui/src/lib.rs`.

**Tests first.** In `crates/tui/src/app_tests.rs`, the `app.rs` test module milestone 4 moved out.

- Replace `new_window_creates_a_shell_in_the_default_dir_and_focuses_it_when_created` with `new_window_opens_the_form_and_submits_on_enter`: `C-b c` returns no effects and sets `Modal::NewAgent` with focus on Runtime. Press `3`, then Enter: exactly one `Send(CreateWindow { spec, cols: 80, rows: 24 })` with `runtime: Shell` and `cwd: /tmp`. The modal is still open with `submitting == true`. `Created { window_id: 9 }` closes the modal; the following `WindowsChanged` with window 9 subscribes to it, as before.
- `a_create_error_is_shown_inline_not_as_a_toast`: submit, then `Error { request: "create", message: "not a git repository: /x" }`. The form stays open, `form.error` holds the message, `submitting` is false, `toast_text()` is `None`.
- `an_unrelated_error_while_the_form_is_open_still_toasts`: `Error { request: "kill", ... }` with the form open and not submitting shows a toast and leaves `form.error` unset.
- `keys_and_pastes_go_to_the_form_not_the_pty`: with the form open, typing `j` or pasting text returns no `Input` effect. With `Modal::Help` open, a paste returns no effects.
- `the_form_remembers_the_last_accepted_values`: submit with Codex, dir `~/p` and model `m1`, receive `Created`, open again: runtime Codex, dir `~/p`, model `m1`, name, branch and prompt empty, worktree unticked. A submit answered by `Error` does not change the defaults.
- `escape_while_submitting_still_focuses_the_new_window`: submit, Esc closes the form, then `Created` and `WindowsChanged` focus the window.
- `first_open_shows_the_default_dir_with_tilde`: `App.default_dir = /home/me/code`, `home_dir = Some(/home/me)`: the Directory field text is `~/code`. Build this with the pure helper the form uses; `shorten_home` currently calls `dirs::home_dir()`, so add a pure variant `shorten_home_with(path, home: Option<&Path>)` in `ui::terminal` and make `shorten_home` call it.

**Change.** Implement decisions 22, 26, 27, 29 and 30. In `on_key`, take the modal out with `self.modal.take()`, handle the key, and put it back unless the handler closed it; the current `clone()` would drop in-place edits to the form. Route `on_paste` to the form or drop it while any modal is open. `tui::run` sets `app.home_dir = dirs::home_dir()`. In tree mode (milestone 4), an open modal takes keys before the tree does.

**Acceptance.** All app tests pass. `app.rs` still performs no I/O. If `app.rs` grows past about 600 lines, move modal key handling into a new `crates/tui/src/app/modal_keys.rs` or similar and record it.

### M5.9 App: remove confirm and force follow-up

**Files.** Modify `crates/tui/src/app.rs`.

**Tests first.** In `crates/tui/src/app_tests.rs`, the `app.rs` test module milestone 4 moved out. `wt_win(id, name, branch)` builds a window with `branch: Some(branch)`.

- `remove_of_a_plain_window_sends_a_plain_remove`: `C-b X` opens `Modal::Remove` with `branch: None`; Space changes nothing; `y` sends `Remove { window_id, remove_worktree: false, force: false }`.
- `remove_checkbox_is_offered_and_starts_unticked`: for a worktree window, Enter right away sends `remove_worktree: false`. Opening again, `w` ticks it (Space also toggles), then Enter sends `remove_worktree: true, force: false`.
- `a_dirty_refusal_opens_the_force_prompt`: after the ticked remove, `Error { request: "remove-dirty", message: "worktree /w has uncommitted or untracked changes" }` opens `ForceRemove` with that message and no toast. `Enter` does nothing. `f` sends `Remove { remove_worktree: true, force: true }`.
- `keep_removes_only_the_window`: from `ForceRemove`, `k` sends `Remove { remove_worktree: false, force: false }`.
- `cancel_leaves_everything`: from `ForceRemove`, Esc sends nothing and closes the modal.
- `remove_dirty_without_a_pending_removal_is_a_toast`: with no removal pending, `Error { request: "remove-dirty" }` only toasts.
- `ack_or_error_clears_the_pending_removal`: after `Ack { request: "remove" }`, a later `remove-dirty` only toasts. `Error { request: "remove", ... }` toasts and also clears it.
- Update `kill_asks_for_confirmation_first` only if `PendingAction` changes break it.

**Change.** Implement decisions 31 and 32. Remove `PendingAction::Remove`.

**Acceptance.** All app tests pass.

### M5.10 Rendering, tree branch, help, smoke stages

**Files.** Create `crates/tui/src/ui/dialog.rs`. Modify `crates/tui/src/ui/mod.rs`, `crates/tui/src/ui/modal.rs`, `crates/tui/src/ui/terminal.rs`, milestone 4's row renderer `crates/tui/src/ui/tree_view.rs` (`narrow_line` and `wide_line`), `scripts/pty-smoke.py`.

**Tests first.** `TestBackend` tests in `crates/tui/src/ui/mod.rs`'s test module, next to milestone 4's tree rendering tests.

- `new_agent_form_renders_fields_and_error`: a form with the worktree ticked, branch `feat/x` and error `not a git repository: /x`, rendered at 100 by 30, contains `new agent`, `Runtime`, `[claude]`, `Branch`, `feat/x`, the error text and `Enter create`. With Shell, `Model` and `Prompt` are absent. While submitting with the worktree ticked, the hint contains `creating the worktree`.
- `new_agent_form_places_the_cursor_in_the_focused_field`: with focus on Name and text `ab`, `terminal.get_cursor_position()` after the draw is on the Name row, 2 columns after the field start.
- `remove_dialog_shows_the_checkbox_only_for_worktree_windows`: a worktree window renders `[ ] also remove worktree feat/x` and `the branch is kept`; ticked renders `[x]`; a plain window renders neither.
- `force_prompt_lists_the_three_choices`: contains `force`, `keep the worktree` and `cancel`.
- `main_title_of_a_worktree_window_names_project_and_branch`: a window with `branch: Some("feat/x")` and `project: /tmp/shop` renders `(feat/x, worktree)` and `/tmp/shop` in the title, not its cwd.
- `tree_row_shows_the_branch_and_truncates_it_first`: a 34-column sidebar with a worktree window named `api` on branch `feat/very-long-branch-name` shows `api [feat/` and `…]`; a window named `a-rather-long-name` keeps at least 8 name columns. The overview shows the full branch.
- `tree_groups_worktree_windows_under_the_repository`: two windows with the same `project` and different cwds (one in the data directory) render under one project row with the count `sh 2`.
- `help_lists_new_agent`: the help overlay contains `C-b c` and `new agent`. The empty-state hint in `ui/terminal.rs` reads `No agents. Press C-b c to create one, or run `anthrex new`.` and `empty_state_and_hidden_sidebar` still passes.

**Change.** Implement the rendering in the Interfaces section and decision 33. `ui::modal::render` dispatches the three new variants to `ui::dialog`. Update `HELP` in `ui/modal.rs`: `("C-b c", "new agent")` and `("C-b X", "remove agent (and worktree)")`.

Smoke script (`scripts/pty-smoke.py`):

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
- `grep -rn "Command::new(\"git\")" crates/daemon/src` prints only `git.rs` (milestone 4's `project.rs` spawns its injectable program, not the literal).
- Search `manager.rs` for every `crate::lock(&self.inner)`: no guard may be alive across a call into `git::`, `worktree::`, `Window::spawn`, `spawn_blocking` or `.await`. Say in the pull request that you checked this.
- `PROTO_VERSION` is still 3.
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
5. In another terminal, `git -C <repo> worktree list` shows the worktree under `/tmp/anthrex-m5/data/worktrees/<repo>-<hash>/m5-check`.
6. Repeat 4 with Codex on branch `m5/codex`. Answer Codex's directory trust prompt. Check that Codex's `-C` points at the worktree: ask it `pwd`.
7. Open the form again: runtime, directory and model are those of the last accepted form.
8. Ask the Claude agent to create a file. `C-b X`, tick the box, Enter. The force prompt appears; Enter does nothing. Press `n`: the agent is still running and its screen intact.
9. `C-b X` again, tick, Enter, then `k`. The window is gone; the worktree directory and branch are still there. Remove it by hand with `git worktree remove --force`.
10. For the Codex window, `C-b X`, tick, Enter on a clean tree: the window and worktree go, `git branch` still lists `m5/codex`.
11. `anthrex new --runtime shell --worktree main --dir <repo>` fails with `branch 'main' is already checked out at <repo>`.
12. Paste a multi-line text into the Prompt field: it arrives on one line. Paste while the help overlay is open: nothing reaches the focused agent.
13. `./target/release/anthrex daemon stop`, then `pgrep -fl "anthrex daemon"` shows nothing. Remove `/tmp/anthrex-m5`.

## Risks and gotchas

1. **Holding the lock across git.** Milestone 1's worst bug was blocking work under the manager lock. Symptom: `a_slow_worktree_create_does_not_block_the_manager` fails, or `anthrex ls` hangs while a worktree is being created. Fix: every `crate::lock` guard in `create` and `remove_with_worktree` must be dropped in its own block before any `.await` or blocking call.
2. **Dropping a create or remove future.** Phase B runs on the blocking pool even if its future is dropped, so an aborted future leaves a worktree or a window nobody records. Symptom: stray directories under `worktrees/` after a client disconnects mid-create. Fix: the server's per-request tasks are never aborted, and the connection teardown does not abort them.
3. **Global git configuration in tests.** A developer's `core.hooksPath`, `commit.gpgsign` or `init.defaultBranch` can break temp-repo tests. `TempRepo` sets repo-local values that override them. If a test still fails only on one machine, compare `git config --show-origin --list` there.
4. **Git version differences.** `--path-format=absolute` needs git 2.31. Error wording differs between versions, which is why the daemon checks "in use" and "dirty" itself (decisions 8 and 12). Tests assert anthrex's messages, never git's. Record the local and CI git versions in "Implementation notes".
5. **Remote-only branches.** `git worktree add <path> <branch>` can create a tracking branch from a unique remote branch, but decision 9 treats a branch with no local ref as new, so it starts at `HEAD`. A user who expects the remote branch gets a fresh one. This is acceptable for this milestone; note it in the pull request.
6. **Spaces in the data directory.** On macOS the worktrees live under `~/Library/Application Support/anthrex/worktrees/`. Some build scripts break on paths with spaces. If a manual check hits this, record it as a follow-up for milestone 6 (a `worktrees_dir` config key); do not change the layout here.
7. **Restart and persistence (milestone 6).** `Entry.spec.worktree_branch` stays `Some(branch)` after creation. A future restart that re-runs `create` with that spec would try to create the worktree again and fail with "already checked out". Milestone 6 must restart from `Entry.worktree` instead. Leave a `// milestone 6:` comment at the `Entry.worktree` field.
8. **Agents writing during removal.** Between the dirty check and the kill, an agent can create a file. Then `git worktree remove` refuses after the agent is already dead, and the window stays listed as Exited. That is the designed outcome of decision 18 step 4; the user answers the force prompt again.
9. **Trust prompts in every new worktree.** Claude and Codex treat each worktree as a new directory and ask for trust once. This is expected and answered in the window (core spec 3.2).
10. **The form and tree mode compete for keys.** If `j` typed into the Name field moves the tree selection, the modal is not taking precedence in `on_key`. The modal check must come before tree mode.

## Follow-ups handled

From `docs/superpowers/plans/2026-09-17-anthrex-foundation-followups.md`, "Assignment to milestones", row M5: the Task 8 minor "create() holds the Inner mutex across Window::spawn". Decision 14 moves `Window::spawn` off the lock (task M5.3).

## Implementation notes

The implementer fills this section in during implementation: every deviation, surprise and decision, with evidence.
