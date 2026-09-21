//! `WindowManager::create`, in the three phases design decision 15 fixes.
//!
//! The whole point of the split is *where the blocking happens*. `git worktree add` can
//! take seconds — it checks a tree out and runs the repository's hooks — and so can
//! spawning a PTY. Neither may happen while the manager's `inner` mutex is held or on a
//! tokio worker thread (AGENTS.md hard rules 2 and 10), because everything else the
//! daemon does to any window goes through that one lock: a create that held it would
//! freeze every other agent, its input and its status, for as long as git took.
//!
//! So:
//!
//! - **Phase A**, under the lock and nowhere near a syscall that can stall: resolve the
//!   name, refuse a duplicate, spend the id, reserve the name. The lock is released
//!   before anything slow starts.
//! - **Phase B**, in one `spawn_blocking` call with no lock held at all: the directory
//!   check, `worktree::create`, `launch::plan` and `Window::spawn`. This is the only
//!   phase that can take seconds, and it holds nothing.
//! - **Phase C**, under the lock again: insert the entry, release the reservation and
//!   publish.
//!
//! Between A and C the window's name and, when it is getting one, the directory its
//! worktree will occupy exist only in `Inner::reserved_names` and
//! `Inner::reserved_worktrees`, both held by one [`Reservation`] guard that gives them
//! back on every exit path — an early return, a panic in phase B, or a caller that drops
//! the future.

use super::{Entry, Inner, ManagerConfig, WindowManager};
use crate::agent_state::AgentState;
use crate::launch::{self, LaunchContext};
use crate::window::{Window, WindowEvent};
use crate::worktree::{self, Created};
use proto::{Status, WindowInfo, WindowSpec};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::sync::mpsc;

/// The git program every worktree operation this manager runs is spawned as (design
/// decision 1). `worktree` takes it as a parameter so its own tests can hand it a
/// recording or a slow script; the daemon has no reason to use anything but `git`.
fn git() -> &'static OsStr {
    OsStr::new("git")
}

/// What phase B hands phase C: a live window, the spec as the child actually saw it
/// (design decision 11 replaces `cwd` for a worktree window), and the worktree that was
/// made for it, if any.
struct Spawned {
    spec: WindowSpec,
    window: Window,
    created: Option<Created>,
}

/// Holds a window's name against other creates and against `rename`, and the directory
/// its worktree will occupy against other creates, until the window exists.
///
/// The guard exists rather than a bare `insert`/`remove` pair because phase B has many
/// ways to end — an error from git, a panic in the blocking closure, a dropped future —
/// and a name or a path left reserved after any of them would be unusable until the
/// daemon restarted, with nothing in `list()` to explain why.
struct Reservation<'a> {
    manager: &'a WindowManager,
    name: String,
    /// The worktree directory this create will make, for a create that asked for one.
    worktree_path: Option<PathBuf>,
    held: bool,
}

impl Reservation<'_> {
    /// Gives both claims back under a lock the caller already holds, so there is no
    /// instant in which neither the reservation nor an entry holds the name.
    fn release(mut self, inner: &mut Inner) {
        Self::clear(inner, &self.name, self.worktree_path.as_deref());
        self.held = false;
    }

    fn clear(inner: &mut Inner, name: &str, worktree_path: Option<&Path>) {
        inner.reserved_names.remove(name);
        if let Some(path) = worktree_path {
            inner.reserved_worktrees.remove(path);
        }
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.held {
            let mut inner = crate::lock(&self.manager.inner);
            Self::clear(&mut inner, &self.name, self.worktree_path.as_deref());
        }
    }
}

impl WindowManager {
    /// Creates a window, running every step that can block off the manager lock.
    ///
    /// `project` and `worktree` are milestone 4.5's roots, resolved by the server from
    /// `spec.cwd` before this is called. For a window that gets its own worktree
    /// `project` is already right — a linked worktree shares the main checkout's project
    /// root — and `worktree` is replaced by the new checkout in phase C (design decision
    /// 21).
    ///
    /// Design decision 17: once phase B has started it always runs to completion.
    /// `spawn_blocking` cannot be cancelled, so a caller that drops this future still
    /// gets the worktree created and the child spawned; the server therefore must never
    /// abort a create task, or it would leak both.
    pub async fn create(
        &self,
        spec: WindowSpec,
        project: PathBuf,
        worktree: Option<PathBuf>,
        cols: u16,
        rows: u16,
    ) -> anyhow::Result<WindowInfo> {
        // Phase A: under the lock, and nothing here can block.
        let (id, reservation) = self.admit(&spec, &project)?;
        let name = reservation.name.clone();

        // Phase B: no lock, not on a tokio worker.
        let config = self.config.clone();
        let events = self.events.clone();
        let phase_b_name = name.clone();
        let spawned = tokio::task::spawn_blocking(move || {
            spawn_window(&config, events, spec, id, &phase_b_name, cols, rows)
        })
        .await
        .map_err(|error| anyhow::anyhow!("window creation failed: {error}"))??;

        // Phase C: under the lock again.
        self.insert(id, name, project, worktree, spawned, reservation)
    }

    /// Phase A. The id is spent whether or not the create goes on to succeed: one lost to
    /// a failed create is never reused, so two creates in flight can never be handed the
    /// same id no matter how long either spends in git.
    ///
    /// Two things are claimed here, and both for the same reason: phase B runs git
    /// without the lock, so anything two concurrent creates could collide on has to be
    /// settled before either of them starts. The name is one. The worktree directory is
    /// the other, and it is the dangerous one — see [`worktree_claim`].
    fn admit(&self, spec: &WindowSpec, project: &Path) -> anyhow::Result<(u32, Reservation<'_>)> {
        let claim = worktree_claim(&self.config.worktrees_root, project, spec);
        let mut inner = crate::lock(&self.inner);
        anyhow::ensure!(!inner.shutting_down, "daemon is shutting down");
        let id = inner.next_id;
        let name = match spec
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            Some(n) => n.to_string(),
            None => format!("{}-{id}", spec.runtime.label()),
        };
        if inner.entries.values().any(|e| e.name == name) || inner.reserved_names.contains(&name) {
            anyhow::bail!("a window named '{name}' already exists");
        }
        if let Some(path) = &claim
            && inner.reserved_worktrees.contains(path)
        {
            // Refused here rather than left to git, which would report it as a "path
            // already exists" from inside phase B — at which point this create would
            // believe the directory was its own to clean up.
            anyhow::bail!("a worktree at {} is already being created", path.display());
        }
        inner.next_id += 1;
        inner.reserved_names.insert(name.clone());
        if let Some(path) = &claim {
            inner.reserved_worktrees.insert(path.clone());
        }
        Ok((
            id,
            Reservation {
                manager: self,
                name,
                worktree_path: claim,
                held: true,
            },
        ))
    }

    /// Phase C.
    fn insert(
        &self,
        id: u32,
        name: String,
        project: PathBuf,
        worktree: Option<PathBuf>,
        spawned: Spawned,
        reservation: Reservation<'_>,
    ) -> anyhow::Result<WindowInfo> {
        let Spawned {
            spec,
            window,
            created,
        } = spawned;
        let mut inner = crate::lock(&self.inner);
        if inner.shutting_down {
            drop(inner);
            return Err(refuse_after_shutdown(window, created));
        }
        let now = Instant::now();
        let entry = Entry {
            id,
            name,
            spec,
            project,
            // Design decision 21: a worktree window's watched root is its *own* linked
            // checkout, not the directory the user pointed at. The server resolved
            // `worktree` from `spec.cwd` before this create ran, so for such a window it
            // names the main checkout; leaving it would show every worktree agent the
            // parent repository's branch and dirty counts in the bottom bar.
            worktree: created
                .as_ref()
                .map(|created| created.worktree.path.clone())
                .or(worktree),
            managed: created.map(|created| created.worktree),
            status: Status::Starting,
            state: AgentState::default(),
            viewers: 0,
            since: now,
            last_output: now,
            exit: None,
            child_alive: true,
            window,
        };
        let info = entry.info(now);
        tracing::info!(
            id,
            name = %info.name,
            runtime = %info.runtime,
            managed = ?entry.managed.as_ref().map(|wt| &wt.path),
            "window created"
        );
        inner.entries.insert(id, entry);
        reservation.release(&mut inner);
        self.publish(&inner);
        Ok(info)
    }
}

/// Phase A admitted this create, then `shutdown` ran while phase B was in git. The window
/// is in no snapshot `shutdown` took, so nothing else will ever kill it; inserting it now
/// would leave a live agent behind the daemon's exit.
///
/// The child is killed here. The worktree, if one was made, is deliberately *not*:
/// removing it would mean running git from a tokio worker (AGENTS.md hard rule 2), and an
/// orphaned directory is recoverable where a stray agent process is not. But nothing else
/// will ever name it — no `Entry` is inserted, so milestone 5.5's removal cannot reach it
/// and no `git worktree list` consumer will connect it to anthrex — so the path goes into
/// both the daemon log and the error the client sees, which is the only record there is.
fn refuse_after_shutdown(window: Window, created: Option<Created>) -> anyhow::Error {
    let _ = window.signal_group(libc::SIGKILL);
    let Some(created) = created else {
        return anyhow::anyhow!("daemon is shutting down");
    };
    tracing::warn!(
        path = ?created.worktree.path,
        branch = %created.worktree.branch,
        "shutdown refused a window whose worktree was already created; it is left on disk"
    );
    anyhow::anyhow!(
        "daemon is shutting down; the new worktree was left at {}",
        created.worktree.path.display()
    )
}

/// The directory `worktree::create` would make for this spec, or `None` when it asked for
/// no worktree. Pure — a hash and some string work, nothing that touches the disk — so it
/// is safe to compute under the manager lock, and it is deliberately the same arithmetic
/// `worktree::create` does later from the roots it resolves itself.
///
/// It is the *directory*, not the branch, because two different branches can want one
/// directory: `branch_dir_name` maps `/` to `-`, so `feat/x` and `feat-x` collide.
/// Decision 9's "branch already checked out" check never sees that pair — they are
/// genuinely different branches — so the directory is the only thing that catches it.
///
/// `project` is the root the server resolved from `spec.cwd`, which is what makes this
/// claim repository-wide: every checkout of one repository shares it, so two creates
/// pointed at different subdirectories of the same repository still collide here.
fn worktree_claim(worktrees_root: &Path, project: &Path, spec: &WindowSpec) -> Option<PathBuf> {
    let branch = spec.worktree_branch.as_deref()?;
    Some(
        worktree::repo_worktrees_dir(worktrees_root, project)
            .join(worktree::branch_dir_name(branch)),
    )
}

/// Phase B, the only phase that can take seconds. Blocking throughout, called only from
/// `spawn_blocking`, and holding no lock of any kind.
fn spawn_window(
    config: &ManagerConfig,
    events: mpsc::UnboundedSender<(u32, WindowEvent)>,
    mut spec: WindowSpec,
    id: u32,
    name: &str,
    cols: u16,
    rows: u16,
) -> anyhow::Result<Spawned> {
    // Before the worktree, so a typo in the directory costs nothing: `worktree::create`
    // would reach the same conclusion by way of `detect_roots`, but only after spending
    // the detection timeout on a path that is not there.
    if !spec.cwd.is_dir() {
        anyhow::bail!("directory does not exist: {}", spec.cwd.display());
    }

    let created = match spec.worktree_branch.as_deref() {
        Some(branch) => Some(worktree::create(
            git(),
            &spec.cwd,
            branch,
            &config.worktrees_root,
            Instant::now() + worktree::OPERATION_TIMEOUT,
        )?),
        None => None,
    };

    if let Some(created) = &created {
        // Design decision 11: the child runs in the worktree root, whatever subdirectory
        // of the repository the user chose. Replacing `spec.cwd` here, before
        // `launch::plan` sees it, is what makes `WindowInfo.cwd`, the PTY's cwd and
        // Codex's `-C` all name the worktree, from one assignment rather than three.
        spec.cwd = created.worktree.path.clone();
    }

    let plan = launch::plan(
        &spec,
        &LaunchContext {
            window_id: id,
            name,
            socket_path: &config.socket_path,
            shell: &config.shell,
            exe: &config.exe,
            claude_bin: &config.claude_bin,
            codex_bin: &config.codex_bin,
            codex_hook_source: config.codex_hook_source.as_deref(),
        },
    );

    let window = match Window::spawn(id, &plan, cols.max(1), rows.max(1), events) {
        Ok(window) => window,
        Err(error) => {
            return Err(match &created {
                Some(created) => discard(created, error),
                None => error,
            });
        }
    };

    Ok(Spawned {
        spec,
        window,
        created,
    })
}

/// Design decision 16: a phase B that failed after the worktree was already made undoes
/// it, and says which of the two things happened.
///
/// Both suffixes are written by `worktree::discard_and_describe`, which is also what
/// `worktree::create` uses for a `git worktree add` that fails or times out, so the two
/// paths into decision 16 cannot word the same outcome differently. See that function
/// for why the distinction matters to the user.
fn discard(created: &Created, error: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!("{}", worktree::discard_and_describe(git(), created, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::ManagedWorktree;

    /// The second of decision 16's two suffixes, which no integration test can reach:
    /// `; the new worktree was removed` needs a cleanup that works, and this one needs a
    /// cleanup that does not. The difference is what the user must do next — retry, or
    /// go and delete a checkout by hand — so a failed cleanup must never be reported in
    /// the words of a successful one.
    #[test]
    fn a_failed_cleanup_says_so_rather_than_claiming_the_worktree_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wt");
        std::fs::create_dir(&path).unwrap();
        let created = Created {
            worktree: ManagedWorktree {
                // Not a repository, so `git worktree remove --force` cannot succeed.
                repo_root: dir.path().to_path_buf(),
                path,
                branch: "feat/x".to_string(),
            },
            created_branch: true,
        };

        let message = discard(&created, anyhow::anyhow!("spawn failed")).to_string();

        assert!(
            message.starts_with("spawn failed; cleanup failed: "),
            "{message}"
        );
        assert!(
            !message.contains("the new worktree was removed"),
            "{message}"
        );
    }
}
