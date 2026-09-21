//! Owns every window, applies status events, and broadcasts the window list.

mod create;
mod remove;
mod restore;

pub use remove::{GitRoots, RemoveError};

use crate::agent_state::AgentState;
use crate::hooks;
use crate::launch;
use crate::status::{self, StatusContext, StatusEvent};
use crate::window::{Attachment, Window, WindowEvent};
use crate::worktree::{self, ManagedWorktree};
use bytes::Bytes;
use proto::{ExitInfo, HookSource, Status, WindowInfo, WindowSpec};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::sync::{broadcast, mpsc, watch};

#[derive(Debug, Clone)]
pub struct ManagerConfig {
    pub socket_path: PathBuf,
    pub shell: String,
    pub exe: PathBuf,
    pub claude_bin: String,
    pub codex_bin: String,
    pub codex_hook_source: Option<String>,
    /// `<data_dir>/worktrees`, the `worktrees_root` every window's linked worktree is
    /// laid out under. `new` and `from_vars` pick a temporary directory so a manager
    /// built without a data directory — every test that does not exercise worktrees —
    /// still has somewhere harmless to point; `lifecycle::run` overrides it.
    pub worktrees_root: PathBuf,
    /// `worktree::OPERATION_TIMEOUT`, injected here rather than read from the constant
    /// directly, exactly like `worktrees_root` above: production always gets the real
    /// value (`new`, `from_vars`), and a test that wants to drive `spawn_window`'s create
    /// path to its deadline without waiting out the real 30 s sets this field instead
    /// (fix wave C item 7 — the cheap version of
    /// `new_worktree_waits_out_the_daemons_whole_create_budget`).
    pub operation_timeout: Duration,
    /// `worktree::CLEANUP_TIMEOUT`, `operation_timeout`'s companion: the other term the
    /// create path's worst-case budget is built from, for the `git worktree add` failure
    /// or timeout that follows.
    pub cleanup_timeout: Duration,
}

impl ManagerConfig {
    pub fn new(socket_path: PathBuf, shell: String) -> Self {
        Self {
            socket_path,
            shell,
            exe: PathBuf::from("anthrex"),
            claude_bin: "claude".to_string(),
            codex_bin: "codex".to_string(),
            codex_hook_source: None,
            worktrees_root: std::env::temp_dir().join("anthrex-worktrees"),
            operation_timeout: worktree::OPERATION_TIMEOUT,
            cleanup_timeout: worktree::CLEANUP_TIMEOUT,
        }
    }

    pub fn from_vars(
        socket_path: PathBuf,
        shell: String,
        exe: PathBuf,
        var: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let nonempty = |key| var(key).filter(|value| !value.is_empty());
        Self {
            socket_path,
            shell,
            exe,
            claude_bin: nonempty("ANTHREX_CLAUDE_BIN").unwrap_or_else(|| "claude".to_string()),
            codex_bin: nonempty("ANTHREX_CODEX_BIN").unwrap_or_else(|| "codex".to_string()),
            codex_hook_source: None,
            worktrees_root: std::env::temp_dir().join("anthrex-worktrees"),
            operation_timeout: worktree::OPERATION_TIMEOUT,
            cleanup_timeout: worktree::CLEANUP_TIMEOUT,
        }
    }

    pub fn from_env(socket_path: PathBuf, shell: String) -> anyhow::Result<Self> {
        let exe = std::env::current_exe()?;
        let mut config = Self::from_vars(socket_path, shell, exe, |key| std::env::var(key).ok());
        config.codex_hook_source = launch::codex::default_hook_source();
        Ok(config)
    }
}

/// A Working window with no output for this long becomes Idle.
pub const QUIET_AFTER: Duration = Duration::from_secs(3);
pub use crate::process::{HUP_GRACE, KILL_GRACE};

/// The `ExitInfo.reason` a restored window carries until it is restarted (decision 14).
pub const DAEMON_RESTARTED: &str = "daemon restarted";

/// The git program every worktree operation this manager runs is spawned as (design
/// decision 1). `worktree` takes it as a parameter so its own tests can hand it a
/// recording or a slow script; the daemon has no reason to use anything but `git`.
///
/// One definition for both halves of the lifecycle: the `git` that made a worktree in
/// [`create`] and the `git` that removes it in [`remove`] must be the same program, or a
/// daemon could create a checkout it cannot unmake.
fn git() -> &'static std::ffi::OsStr {
    std::ffi::OsStr::new("git")
}

/// What actually runs behind a window (decision 14).
///
/// A restored window has no child, no PTY and no vt100 mirror — everything [`Window`]
/// owns — so it cannot simply hold a [`Window`] in some "not really running" state. This
/// enum is the alternative: [`Process::Live`] is every window `create` or (a later
/// milestone's) `restart` actually spawned, and [`Process::Dormant`] is every window
/// [`WindowManager::restore`] rebuilt from the state file. Routing `write_input`,
/// `resize`, `attach`, `snapshot`, `signal_group` and `pid` through this instead of
/// through `Window` directly is what lets every other call site treat a restored window
/// as an ordinary listed window rather than special-casing it.
enum Process {
    Live(Window),
    /// `output` is a capacity-1 sender nothing is ever sent on. Its only job is to give
    /// a subscriber a [`broadcast::Receiver`] that exists, so `attach` has something to
    /// hand back — and, once a later milestone's `restart` swaps this entry's `Process`
    /// for a `Live` one, dropping this sender is what closes that receiver, which is how
    /// `forward_output_from` (decision 21) learns to re-attach.
    Dormant {
        output: broadcast::Sender<Bytes>,
        cols: u16,
        rows: u16,
    },
}

struct Entry {
    id: u32,
    name: String,
    spec: WindowSpec,
    project: PathBuf,
    /// The git worktree *root* this window's git state is keyed on: milestone 4.5's
    /// field, which the registry watches. For a window this daemon made a worktree for
    /// it is that new checkout (design decision 21), which is why it is not the same
    /// question as `managed` below.
    worktree: Option<PathBuf>,
    /// The worktree this daemon created *for* this window, `None` for every other
    /// window. Not to be confused with `worktree`: that one answers "which checkout do
    /// we watch", this one answers "did we make it, and may we remove it".
    // milestone 6: restart re-attaches to this, not to `spec.worktree_branch` (risk 7).
    managed: Option<ManagedWorktree>,
    /// A `remove_with_worktree` is in flight for this window (design decision 19 step 1).
    /// The window stays listed and keeps running while it is set, because the removal can
    /// still be refused; what the flag stops is a *second* removal reaching git for the
    /// same checkout, which would have two `git worktree remove` calls and two
    /// `unregister`s for one directory.
    removing: bool,
    /// A restart (task M6.7, not this one) is in flight for this window. Set to `false`
    /// everywhere an `Entry` is built in this task and read by no code yet; added now,
    /// ahead of the task that reads and writes it, because the M6.5 brief calls for it
    /// explicitly so M6.7 does not have to touch every `Entry` literal again.
    #[allow(
        dead_code,
        reason = "read and written starting in task M6.7 (restart in the daemon)"
    )]
    restarting: bool,
    status: Status,
    state: AgentState,
    viewers: u32,
    since: Instant,
    last_output: Instant,
    /// When this window was first created, preserved verbatim across a restore (decision
    /// 14) so the state file's `created_at` never resets just because the daemon did.
    /// Distinct from `since`, which restarts at every status change including a restore.
    created_at: SystemTime,
    exit: Option<ExitInfo>,
    child_alive: bool,
    process: Process,
}

impl Entry {
    fn info(&self, now: Instant) -> WindowInfo {
        WindowInfo {
            id: self.id,
            name: self.name.clone(),
            runtime: self.spec.runtime,
            cwd: self.spec.cwd.clone(),
            project: self.project.clone(),
            worktree: self.worktree.clone(),
            branch: self.spec.worktree_branch.clone(),
            status: self.status,
            tool: self.state.tool.clone(),
            since_secs: self.since.elapsed().as_secs(),
            last_output_secs: self.last_output.elapsed().as_secs(),
            session_id: self.state.session_id.clone(),
            model: self.spec.model.clone(),
            subagents: self.state.subagents.infos(now),
            exit: self.exit.clone(),
        }
    }

    /// Applies a status event; returns whether the status changed.
    fn apply(&mut self, event: StatusEvent) -> bool {
        self.apply_with_context(event, self.state.context(self.viewers > 0))
    }

    fn apply_with_context(&mut self, event: StatusEvent, ctx: StatusContext) -> bool {
        let next = status::next(self.status, event, self.spec.runtime, ctx);
        if next == self.status {
            return false;
        }
        self.status = next;
        self.since = Instant::now();
        true
    }

    fn pid(&self) -> Option<u32> {
        match &self.process {
            Process::Live(window) => window.pid(),
            Process::Dormant { .. } => None,
        }
    }

    /// Decision 14's dormant behaviour: refuses with a message naming the restart paths.
    fn write_input(&self, bytes: &[u8]) -> anyhow::Result<()> {
        match &self.process {
            Process::Live(window) => window.write_input(bytes),
            Process::Dormant { .. } => anyhow::bail!(
                "window is not running; restart it with C-b R or anthrex restart {}",
                self.id
            ),
        }
    }

    /// Decision 14: a dormant window has no PTY to resize, so this just records the size
    /// for the placeholder snapshot `attach`/`snapshot` build from.
    fn resize(&mut self, cols: u16, rows: u16) -> anyhow::Result<()> {
        match &mut self.process {
            Process::Live(window) => window.resize(cols, rows),
            Process::Dormant {
                cols: c, rows: r, ..
            } => {
                *c = cols;
                *r = rows;
                Ok(())
            }
        }
    }

    fn size(&self) -> (u16, u16) {
        match &self.process {
            Process::Live(window) => window.size(),
            Process::Dormant { cols, rows, .. } => (*cols, *rows),
        }
    }

    fn attach(&self) -> Attachment {
        match &self.process {
            Process::Live(window) => window.attach(),
            Process::Dormant { output, cols, rows } => Attachment {
                output: output.subscribe(),
                snapshot: self.dormant_placeholder(),
                cols: *cols,
                rows: *rows,
            },
        }
    }

    fn snapshot(&self) -> Vec<u8> {
        match &self.process {
            Process::Live(window) => window.snapshot(),
            Process::Dormant { .. } => self.dormant_placeholder(),
        }
    }

    /// Decision 14's placeholder screen: cleared, then a fixed explanation, then the
    /// session id to resume when one is known.
    fn dormant_placeholder(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"\x1b[2J\x1b[H");
        out.extend_from_slice(b"[anthrex] this window stopped when the daemon restarted.\r\n");
        out.extend_from_slice(
            format!(
                "[anthrex] restart it with C-b R (default keys) or: anthrex restart {}\r\n",
                self.id
            )
            .as_bytes(),
        );
        if let Some(session_id) = &self.state.session_id {
            out.extend_from_slice(
                format!("[anthrex] the restart resumes session {session_id}.\r\n").as_bytes(),
            );
        }
        out
    }

    /// Decision 14: succeeds and does nothing for a dormant window — there is no process
    /// group to signal.
    fn signal_group(&self, sig: i32) -> anyhow::Result<()> {
        match &self.process {
            Process::Live(window) => window.signal_group(sig),
            Process::Dormant { .. } => Ok(()),
        }
    }
}

struct Inner {
    next_id: u32,
    shutting_down: bool,
    entries: BTreeMap<u32, Entry>,
    /// Names of creates that have been admitted but whose window does not exist yet
    /// (design decision 15, phase A). A name is taken from the moment a create is
    /// admitted, because phase B can sit in `git worktree add` for seconds and two
    /// creates racing on one name would otherwise both pass the duplicate check.
    reserved_names: BTreeSet<String>,
    /// Worktree directories that admitted creates are on their way to making, held for
    /// exactly as long as `reserved_names` holds their window's name.
    ///
    /// Without this, two creates with different names and the same branch in one
    /// repository both enter phase B and run git concurrently: the second one's
    /// pre-flight checks pass before the first's `worktree add` has registered anything,
    /// so it goes on to `worktree add` itself, fails with "already exists", and cleans up
    /// after what it thinks is its own half-made worktree — which is the first agent's
    /// live checkout, removed with `--force`, and its branch deleted with it. Running
    /// parallel agents on one repository is what this milestone is *for*, so that is the
    /// normal case, not an exotic one.
    reserved_worktrees: BTreeSet<PathBuf>,
    // Cleanup owns a group beyond the leader's exit and even after window removal.
    cleanups: BTreeMap<u32, watch::Receiver<bool>>,
}

impl Inner {
    fn start_cleanup(&mut self, id: u32) -> anyhow::Result<()> {
        if self.cleanups.contains_key(&id) {
            return Ok(());
        }
        let entry = self
            .entries
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        if entry.child_alive
            && let Some(pid) = entry.pid()
        {
            self.cleanups.insert(id, crate::process::escalate(pid)?);
        }
        Ok(())
    }
}

pub struct WindowManager {
    inner: Mutex<Inner>,
    changed: watch::Sender<Vec<WindowInfo>>,
    events: mpsc::UnboundedSender<(u32, WindowEvent)>,
    config: ManagerConfig,
}

impl WindowManager {
    /// Returns the manager and the event receiver the caller must pump into `handle_event`.
    pub fn new(config: ManagerConfig) -> (Arc<Self>, mpsc::UnboundedReceiver<(u32, WindowEvent)>) {
        let (events, events_rx) = mpsc::unbounded_channel();
        let (changed, _) = watch::channel(Vec::new());
        let manager = Arc::new(Self {
            inner: Mutex::new(Inner {
                next_id: 1,
                shutting_down: false,
                entries: BTreeMap::new(),
                reserved_names: BTreeSet::new(),
                reserved_worktrees: BTreeSet::new(),
                cleanups: BTreeMap::new(),
            }),
            changed,
            events,
            config,
        });
        (manager, events_rx)
    }

    pub fn watch(&self) -> watch::Receiver<Vec<WindowInfo>> {
        self.changed.subscribe()
    }

    pub fn list(&self) -> Vec<WindowInfo> {
        let now = Instant::now();
        crate::lock(&self.inner)
            .entries
            .values()
            .map(|entry| entry.info(now))
            .collect()
    }

    fn publish(&self, inner: &Inner) {
        let now = Instant::now();
        self.changed.send_replace(
            inner
                .entries
                .values()
                .map(|entry| entry.info(now))
                .collect(),
        );
    }

    pub fn handle_hook(
        &self,
        id: u32,
        source: HookSource,
        payload: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let mut inner = crate::lock(&self.inner);
        let entry = inner
            .entries
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        if !hooks::accepts(entry.spec.runtime, source) {
            tracing::debug!(id, ?source, "ignored hook source for runtime");
            return Ok(());
        }
        let Some(hook) = hooks::parse(source, payload) else {
            if tracing::enabled!(tracing::Level::DEBUG) {
                let mut payload = payload.to_string();
                let mut end = payload.len().min(2048);
                while !payload.is_char_boundary(end) {
                    end -= 1;
                }
                payload.truncate(end);
                tracing::debug!(id, ?source, %payload, "ignored unparseable hook");
            }
            return Ok(());
        };
        // SessionStart must see the flags from before this event is accepted.
        let ctx = entry.state.context(entry.viewers > 0);
        let now = Instant::now();
        let outcome = entry.state.on_hook(entry.spec.runtime, &hook, now);
        let status_changed = outcome
            .status_event
            .is_some_and(|event| entry.apply_with_context(event, ctx));
        if status_changed || outcome.changed {
            self.publish(&inner);
        }
        Ok(())
    }

    pub fn handle_event(&self, id: u32, event: WindowEvent) {
        let parser_panicked = matches!(&event, WindowEvent::ParserPanicked(_));
        let mut inner = crate::lock(&self.inner);
        let Some(entry) = inner.entries.get_mut(&id) else {
            return;
        };
        let now = Instant::now();
        let changed = match event {
            WindowEvent::Output => {
                entry.last_output = now;
                entry.apply(StatusEvent::Output)
            }
            WindowEvent::Bell => entry.apply(StatusEvent::Bell),
            WindowEvent::Title(title) => {
                let ctx = entry.state.context(entry.viewers > 0);
                entry
                    .state
                    .on_title(entry.spec.runtime, &title)
                    .is_some_and(|event| entry.apply_with_context(event, ctx))
            }
            WindowEvent::ParserPanicked(reason) => {
                entry.exit.get_or_insert_with(|| ExitInfo {
                    code: None,
                    reason: format!("screen parser panicked: {reason}"),
                });
                entry.apply(StatusEvent::Exited)
            }
            WindowEvent::Exited { code, signal } => {
                let reason = match (&signal, code) {
                    (Some(sig), _) => format!("killed by {sig}"),
                    (None, Some(c)) => format!("exited with code {c}"),
                    (None, None) => "exited".to_string(),
                };
                tracing::info!(id, %reason, "window exited");
                entry.child_alive = false;
                entry.exit.get_or_insert(ExitInfo { code, reason });
                let status_changed = entry.apply(StatusEvent::Exited);
                let subagents_changed = entry.state.subagents.child_exited(now);
                status_changed || subagents_changed
            }
        };
        if parser_panicked && let Err(error) = inner.start_cleanup(id) {
            tracing::error!(id, %error, "parser panic cleanup failed");
        }
        if changed {
            self.publish(&inner);
        }
    }

    /// Called once a second by the daemon: Working windows that went quiet become Idle.
    pub fn tick(&self) {
        let now = Instant::now();
        let mut inner = crate::lock(&self.inner);
        let Inner {
            entries, cleanups, ..
        } = &mut *inner;
        cleanups.retain(|id, done| entries.contains_key(id) || !*done.borrow());
        let mut changed = false;
        for entry in inner.entries.values_mut() {
            changed |= entry.state.subagents.prune(now);
            if entry.status == Status::Working
                && now.saturating_duration_since(entry.last_output) >= QUIET_AFTER
            {
                changed |= entry.apply(StatusEvent::Quiet);
            }
        }
        if changed {
            self.publish(&inner);
        }
    }

    fn with_entry<R>(&self, id: u32, f: impl FnOnce(&mut Entry) -> R) -> anyhow::Result<R> {
        let mut inner = crate::lock(&self.inner);
        let entry = inner
            .entries
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        Ok(f(entry))
    }

    /// Queues input for a window. `Window::write_input` only enqueues onto the window's
    /// writer thread, so holding `Inner` across it cannot stall the rest of the daemon
    /// behind a PTY that is not being read.
    pub fn write_input(&self, id: u32, bytes: &[u8]) -> anyhow::Result<()> {
        let mut inner = crate::lock(&self.inner);
        let entry = inner
            .entries
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        entry.write_input(bytes)?;
        let status_changed = entry.apply(StatusEvent::InputSent);
        let subagents_changed = entry.state.subagents.input_sent();
        if status_changed || subagents_changed {
            self.publish(&inner);
        }
        Ok(())
    }

    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.with_entry(id, |e| e.resize(cols.max(1), rows.max(1)))?
    }

    pub fn attach(&self, id: u32) -> anyhow::Result<Attachment> {
        self.with_entry(id, |e| e.attach())
    }

    pub fn child_pid(&self, id: u32) -> anyhow::Result<Option<u32>> {
        self.with_entry(id, |e| e.pid())
    }

    pub fn snapshot(&self, id: u32) -> anyhow::Result<(Vec<u8>, u16, u16)> {
        self.with_entry(id, |e| {
            let (cols, rows) = e.size();
            (e.snapshot(), cols, rows)
        })
    }

    /// A client started viewing this window.
    pub fn focus(&self, id: u32) {
        let mut inner = crate::lock(&self.inner);
        if let Some(entry) = inner.entries.get_mut(&id) {
            entry.viewers = entry.viewers.saturating_add(1);
            if entry.apply(StatusEvent::Focused) {
                self.publish(&inner);
            }
        }
    }

    /// A client stopped viewing this window.
    pub fn unfocus(&self, id: u32) {
        let mut inner = crate::lock(&self.inner);
        if let Some(entry) = inner.entries.get_mut(&id) {
            entry.viewers = entry.viewers.saturating_sub(1);
        }
    }

    /// SIGHUP now, SIGTERM after one second, SIGKILL after three seconds.
    pub fn kill(self: &Arc<Self>, id: u32) -> anyhow::Result<()> {
        let mut inner = crate::lock(&self.inner);
        anyhow::ensure!(inner.entries.contains_key(&id), "no window with id {id}");
        inner.start_cleanup(id)
    }

    /// Kills immediately and forgets the window, leaving any worktree on disk (design
    /// decision 18). [`WindowManager::remove_with_worktree`] is the other path.
    ///
    /// A window that `remove_with_worktree` has already admitted belongs to that removal
    /// until it finishes or gives up, so this refuses it rather than forgetting the entry
    /// out from under it. That is not only tidiness: the two paths each drop one reference
    /// to the window's git root — this one through the server, the other between its kill
    /// and its deletion — and design decision 23 is that one removed window is exactly one
    /// `unregister`. Letting both run would decrement a single registration twice, which is
    /// invisible at a count of one and, with two windows on a root, stops the survivor's
    /// watch with nothing on screen to explain why.
    pub fn remove(&self, id: u32) -> anyhow::Result<()> {
        let mut inner = crate::lock(&self.inner);
        let entry = inner
            .entries
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        if entry.removing {
            anyhow::bail!("window '{}' is already being removed", entry.name);
        }
        let entry = inner.entries.remove(&id).expect("looked up a line above");
        if entry.child_alive {
            let _ = entry.signal_group(libc::SIGKILL);
        }
        drop(entry);
        tracing::info!(id, "window removed");
        self.publish(&inner);
        Ok(())
    }

    pub fn rename(&self, id: u32, name: String) -> anyhow::Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("name must not be empty");
        }
        let mut inner = crate::lock(&self.inner);
        // A name a create is still holding is taken just as firmly as one a window has:
        // letting a rename win the race would leave two windows named the same the
        // moment that create reached phase C.
        if inner.entries.values().any(|e| e.id != id && e.name == name)
            || inner.reserved_names.contains(&name)
        {
            anyhow::bail!("a window named '{name}' already exists");
        }
        let entry = inner
            .entries
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        entry.name = name;
        self.publish(&inner);
        Ok(())
    }

    /// Run the same bounded group escalation for every live child concurrently.
    pub async fn shutdown(&self) {
        let pending: Vec<_> = {
            let mut inner = crate::lock(&self.inner);
            // Share create's admission lock: an accepted window is in this snapshot,
            // and a creation still resolving its project cannot spawn afterward.
            inner.shutting_down = true;
            let ids: Vec<_> = inner.entries.keys().copied().collect();
            for id in ids {
                if let Err(error) = inner.start_cleanup(id) {
                    tracing::error!(id, %error, "shutdown cleanup failed");
                }
            }
            inner.cleanups.values().cloned().collect()
        };
        for mut done in pending {
            let _ = done.wait_for(|finished| *finished).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn bin_overrides_come_from_the_environment_variables() {
        let vars = HashMap::from([
            ("ANTHREX_CLAUDE_BIN", "/opt/agents/claude"),
            ("ANTHREX_CODEX_BIN", "/opt/agents/codex"),
        ]);
        let config = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |key| vars.get(key).map(|value| (*value).to_string()),
        );
        assert_eq!(config.claude_bin, "/opt/agents/claude");
        assert_eq!(config.codex_bin, "/opt/agents/codex");

        let defaults = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |key| (key == "ANTHREX_CLAUDE_BIN").then(String::new),
        );
        assert_eq!(defaults.claude_bin, "claude");
        assert_eq!(defaults.codex_bin, "codex");
    }
}
