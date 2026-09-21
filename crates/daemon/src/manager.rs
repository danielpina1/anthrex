//! Owns every window, applies status events, and broadcasts the window list.

mod create;

use crate::agent_state::AgentState;
use crate::hooks;
use crate::launch;
use crate::status::{self, StatusContext, StatusEvent};
use crate::window::{Attachment, Window, WindowEvent};
use crate::worktree::ManagedWorktree;
use proto::{ExitInfo, HookSource, Status, WindowInfo, WindowSpec};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

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
    status: Status,
    state: AgentState,
    viewers: u32,
    since: Instant,
    last_output: Instant,
    exit: Option<ExitInfo>,
    child_alive: bool,
    window: Window,
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
            && let Some(pid) = entry.window.pid()
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
        entry.window.write_input(bytes)?;
        let status_changed = entry.apply(StatusEvent::InputSent);
        let subagents_changed = entry.state.subagents.input_sent();
        if status_changed || subagents_changed {
            self.publish(&inner);
        }
        Ok(())
    }

    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> anyhow::Result<()> {
        self.with_entry(id, |e| e.window.resize(cols.max(1), rows.max(1)))?
    }

    pub fn attach(&self, id: u32) -> anyhow::Result<Attachment> {
        self.with_entry(id, |e| e.window.attach())
    }

    pub fn child_pid(&self, id: u32) -> anyhow::Result<Option<u32>> {
        self.with_entry(id, |e| e.window.pid())
    }

    pub fn snapshot(&self, id: u32) -> anyhow::Result<(Vec<u8>, u16, u16)> {
        self.with_entry(id, |e| {
            let (cols, rows) = e.window.size();
            (e.window.snapshot(), cols, rows)
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

    /// Kills immediately and forgets the window.
    pub fn remove(&self, id: u32) -> anyhow::Result<()> {
        let mut inner = crate::lock(&self.inner);
        let entry = inner
            .entries
            .remove(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        if entry.child_alive {
            let _ = entry.window.signal_group(libc::SIGKILL);
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
