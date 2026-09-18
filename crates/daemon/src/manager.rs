//! Owns every window, applies status events, and broadcasts the window list.

use crate::agent_state::AgentState;
use crate::hooks;
use crate::launch::{self, LaunchContext};
use crate::status::{self, StatusContext, StatusEvent};
use crate::window::{Attachment, Window, WindowEvent};
use proto::{ExitInfo, HookSource, Status, WindowInfo, WindowSpec};
use std::collections::BTreeMap;
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
    fn info(&self) -> WindowInfo {
        WindowInfo {
            id: self.id,
            name: self.name.clone(),
            runtime: self.spec.runtime,
            cwd: self.spec.cwd.clone(),
            branch: self.spec.worktree_branch.clone(),
            status: self.status,
            tool: self.state.tool.clone(),
            since_secs: self.since.elapsed().as_secs(),
            last_output_secs: self.last_output.elapsed().as_secs(),
            session_id: self.state.session_id.clone(),
            model: self.spec.model.clone(),
            subagents: vec![],
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
    entries: BTreeMap<u32, Entry>,
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
                entries: BTreeMap::new(),
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
        crate::lock(&self.inner)
            .entries
            .values()
            .map(Entry::info)
            .collect()
    }

    fn publish(&self, inner: &Inner) {
        self.changed
            .send_replace(inner.entries.values().map(Entry::info).collect());
    }

    pub fn create(&self, spec: WindowSpec, cols: u16, rows: u16) -> anyhow::Result<WindowInfo> {
        let mut inner = crate::lock(&self.inner);
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
        if inner.entries.values().any(|e| e.name == name) {
            anyhow::bail!("a window named '{name}' already exists");
        }
        if !spec.cwd.is_dir() {
            anyhow::bail!("directory does not exist: {}", spec.cwd.display());
        }
        if spec.worktree_branch.is_some() {
            // Accepting it would create no worktree while `WindowInfo.branch` showed the
            // branch in the title bar, so the user would believe the agent was isolated.
            anyhow::bail!("{}", crate::WORKTREE_UNSUPPORTED);
        }
        let plan = launch::plan(
            &spec,
            &LaunchContext {
                window_id: id,
                name: &name,
                socket_path: &self.config.socket_path,
                shell: &self.config.shell,
                exe: &self.config.exe,
                claude_bin: &self.config.claude_bin,
                codex_bin: &self.config.codex_bin,
                codex_hook_source: self.config.codex_hook_source.as_deref(),
            },
        );
        let window = Window::spawn(id, &plan, cols.max(1), rows.max(1), self.events.clone())?;
        inner.next_id += 1;
        let now = Instant::now();
        let entry = Entry {
            id,
            name,
            spec,
            status: Status::Starting,
            state: AgentState::default(),
            viewers: 0,
            since: now,
            last_output: now,
            exit: None,
            child_alive: true,
            window,
        };
        let info = entry.info();
        inner.entries.insert(id, entry);
        tracing::info!(id, name = %info.name, runtime = %info.runtime, "window created");
        self.publish(&inner);
        Ok(info)
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
        let outcome = entry
            .state
            .on_hook(entry.spec.runtime, &hook, Instant::now());
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
        let changed = match event {
            WindowEvent::Output => {
                entry.last_output = Instant::now();
                entry.apply(StatusEvent::Output)
            }
            WindowEvent::Bell => entry.apply(StatusEvent::Bell),
            WindowEvent::Title(title) => {
                tracing::debug!(id, %title, "window title");
                false
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
                entry.apply(StatusEvent::Exited)
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
        let mut inner = crate::lock(&self.inner);
        let Inner {
            entries, cleanups, ..
        } = &mut *inner;
        cleanups.retain(|id, done| entries.contains_key(id) || !*done.borrow());
        let mut changed = false;
        for entry in inner.entries.values_mut() {
            if entry.status == Status::Working && entry.last_output.elapsed() >= QUIET_AFTER {
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
        if entry.apply(StatusEvent::InputSent) {
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
        if inner.entries.values().any(|e| e.id != id && e.name == name) {
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
