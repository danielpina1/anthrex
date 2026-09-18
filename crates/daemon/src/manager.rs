//! Owns every window, applies status events, and broadcasts the window list.

use crate::launch::{self, LaunchContext};
use crate::status::{self, StatusEvent};
use crate::window::{Attachment, Window, WindowEvent};
use proto::{ExitInfo, Status, WindowInfo, WindowSpec};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// A Working window with no output for this long becomes Idle.
pub const QUIET_AFTER: Duration = Duration::from_secs(3);
/// Time between SIGTERM and SIGKILL.
pub const KILL_GRACE: Duration = Duration::from_secs(3);

struct Entry {
    id: u32,
    name: String,
    spec: WindowSpec,
    status: Status,
    tool: Option<String>,
    since: Instant,
    last_output: Instant,
    session_id: Option<String>,
    exit: Option<ExitInfo>,
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
            tool: self.tool.clone(),
            since_secs: self.since.elapsed().as_secs(),
            last_output_secs: self.last_output.elapsed().as_secs(),
            has_session: self.session_id.is_some(),
            exit: self.exit.clone(),
        }
    }

    /// Applies a status event; returns whether the status changed.
    fn apply(&mut self, event: StatusEvent) -> bool {
        let next = status::next(self.status, event, self.spec.runtime);
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
}

pub struct WindowManager {
    inner: Mutex<Inner>,
    changed: watch::Sender<Vec<WindowInfo>>,
    events: mpsc::UnboundedSender<(u32, WindowEvent)>,
    socket_path: PathBuf,
    shell: String,
}

impl WindowManager {
    /// Returns the manager and the event receiver the caller must pump into `handle_event`.
    pub fn new(
        socket_path: PathBuf,
        shell: String,
    ) -> (Arc<Self>, mpsc::UnboundedReceiver<(u32, WindowEvent)>) {
        let (events, events_rx) = mpsc::unbounded_channel();
        let (changed, _) = watch::channel(Vec::new());
        let manager = Arc::new(Self {
            inner: Mutex::new(Inner {
                next_id: 1,
                entries: BTreeMap::new(),
            }),
            changed,
            events,
            socket_path,
            shell,
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
                socket_path: &self.socket_path,
                shell: &self.shell,
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
            tool: None,
            since: now,
            last_output: now,
            session_id: None,
            exit: None,
            window,
        };
        let info = entry.info();
        inner.entries.insert(id, entry);
        tracing::info!(id, name = %info.name, runtime = %info.runtime, "window created");
        self.publish(&inner);
        Ok(info)
    }

    pub fn handle_event(&self, id: u32, event: WindowEvent) {
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
            WindowEvent::Exited { code, signal } => {
                let reason = match (&signal, code) {
                    (Some(sig), _) => format!("killed by {sig}"),
                    (None, Some(c)) => format!("exited with code {c}"),
                    (None, None) => "exited".to_string(),
                };
                tracing::info!(id, %reason, "window exited");
                entry.exit = Some(ExitInfo { code, reason });
                entry.apply(StatusEvent::Exited)
            }
        };
        if changed {
            self.publish(&inner);
        }
    }

    /// Called once a second by the daemon: Working windows that went quiet become Idle.
    pub fn tick(&self) {
        let mut inner = crate::lock(&self.inner);
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

    pub fn snapshot(&self, id: u32) -> anyhow::Result<(Vec<u8>, u16, u16)> {
        self.with_entry(id, |e| {
            let (cols, rows) = e.window.size();
            (e.window.snapshot(), cols, rows)
        })
    }

    /// A client started viewing this window.
    pub fn focus(&self, id: u32) {
        let mut inner = crate::lock(&self.inner);
        if let Some(entry) = inner.entries.get_mut(&id)
            && entry.apply(StatusEvent::Focused)
        {
            self.publish(&inner);
        }
    }

    fn signal(&self, id: u32, sig: i32) -> anyhow::Result<()> {
        let inner = crate::lock(&self.inner);
        let entry = inner
            .entries
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        if entry.status == Status::Exited {
            return Ok(());
        }
        entry.window.signal(sig)
    }

    /// SIGTERM now, SIGKILL after `KILL_GRACE` if the child is still alive.
    pub fn kill(self: &Arc<Self>, id: u32) -> anyhow::Result<()> {
        self.signal(id, libc::SIGTERM)?;
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            std::thread::sleep(KILL_GRACE);
            let _ = me.signal(id, libc::SIGKILL);
        });
        Ok(())
    }

    /// Kills immediately and forgets the window.
    pub fn remove(&self, id: u32) -> anyhow::Result<()> {
        let mut inner = crate::lock(&self.inner);
        let entry = inner
            .entries
            .remove(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        if entry.status != Status::Exited {
            let _ = entry.window.signal(libc::SIGKILL);
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

    /// SIGTERM every live window, wait up to `KILL_GRACE`, then SIGKILL the rest.
    pub async fn shutdown(&self) {
        let live: Vec<u32> = self
            .list()
            .into_iter()
            .filter(|w| w.status != Status::Exited)
            .map(|w| w.id)
            .collect();
        for id in &live {
            let _ = self.signal(*id, libc::SIGTERM);
        }
        let deadline = Instant::now() + KILL_GRACE;
        while Instant::now() < deadline {
            if self.list().iter().all(|w| w.status == Status::Exited) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        for id in &live {
            let _ = self.signal(*id, libc::SIGKILL);
        }
    }
}
