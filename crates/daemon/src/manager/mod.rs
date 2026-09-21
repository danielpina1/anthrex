//! Owns every window, applies status events, and broadcasts the window list.

mod create;
mod entry;
mod remove;
mod restart;
mod restore;

pub use remove::{GitRoots, RemoveError};

use crate::hooks;
use crate::launch;
use crate::status::StatusEvent;
use crate::window::{Attachment, WindowEvent};
use crate::worktree;
use entry::{Entry, Inner};
use proto::{ExitInfo, HookSource, Status, WindowInfo};
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
    /// `runtimes.codex.bypass_hook_trust` (config decision 4), copied into every window's
    /// `LaunchContext`. A changed config needs a daemon restart to take effect.
    pub codex_bypass_hook_trust: bool,
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
    /// `crate::process::KILL_GRACE`, injected here for the same reason as
    /// `operation_timeout` above: production always gets the real value (`new`,
    /// `from_vars`), and `remove_with_worktree`'s `kill_and_await_exit` reads this field
    /// rather than the constant directly, so a test can widen it far past its own removal
    /// bound instead of racing the two. See `an_exited_window_is_removed_without_waiting`
    /// (`daemon/tests/manager_worktree/removal/ordering.rs`), which sets this to 60 s and
    /// asserts the removal finishes in under 10 s — a 6x separation between "waited" and
    /// "didn't wait" instead of the ~1x a shared 1 s bound gave it.
    pub kill_grace: Duration,
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
            codex_bypass_hook_trust: false,
            worktrees_root: std::env::temp_dir().join("anthrex-worktrees"),
            operation_timeout: worktree::OPERATION_TIMEOUT,
            cleanup_timeout: worktree::CLEANUP_TIMEOUT,
            kill_grace: crate::process::KILL_GRACE,
        }
    }

    /// Applies config decision 6's precedence, highest first: an `ANTHREX_*_BIN`
    /// environment variable, then `runtimes.*.command`, then the built-in default. Each
    /// runtime's command is resolved independently, so an override for one never affects
    /// the other.
    pub fn from_vars(
        socket_path: PathBuf,
        shell: String,
        exe: PathBuf,
        var: impl Fn(&str) -> Option<String>,
        runtimes: &config::Runtimes,
    ) -> Self {
        let nonempty = |key| var(key).filter(|value| !value.is_empty());
        Self {
            socket_path,
            shell,
            exe,
            claude_bin: nonempty("ANTHREX_CLAUDE_BIN")
                .unwrap_or_else(|| runtimes.claude.command.clone()),
            codex_bin: nonempty("ANTHREX_CODEX_BIN")
                .unwrap_or_else(|| runtimes.codex.command.clone()),
            codex_hook_source: None,
            codex_bypass_hook_trust: runtimes.codex_bypass_hook_trust,
            worktrees_root: std::env::temp_dir().join("anthrex-worktrees"),
            operation_timeout: worktree::OPERATION_TIMEOUT,
            cleanup_timeout: worktree::CLEANUP_TIMEOUT,
            kill_grace: crate::process::KILL_GRACE,
        }
    }

    pub fn from_env(
        socket_path: PathBuf,
        shell: String,
        runtimes: &config::Runtimes,
    ) -> anyhow::Result<Self> {
        let exe = std::env::current_exe()?;
        let mut config = Self::from_vars(
            socket_path,
            shell,
            exe,
            |key| std::env::var(key).ok(),
            runtimes,
        );
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
            &config::Config::default().runtimes,
        );
        assert_eq!(config.claude_bin, "/opt/agents/claude");
        assert_eq!(config.codex_bin, "/opt/agents/codex");

        let defaults = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |key| (key == "ANTHREX_CLAUDE_BIN").then(String::new),
            &config::Config::default().runtimes,
        );
        assert_eq!(defaults.claude_bin, "claude");
        assert_eq!(defaults.codex_bin, "codex");
    }

    /// Config decision 6's precedence, highest first: an `ANTHREX_*_BIN` environment
    /// variable, then `runtimes.*.command`, then the built-in default. An override for
    /// one runtime never leaks onto the other, and `codex_bypass_hook_trust` is copied
    /// straight from the config (decision 4).
    #[test]
    fn commands_resolve_env_over_config_over_default() {
        let runtimes = config::Runtimes {
            claude: config::RuntimeCommand {
                command: "/opt/config/claude".to_string(),
            },
            codex: config::RuntimeCommand {
                command: "/opt/config/codex".to_string(),
            },
            codex_bypass_hook_trust: true,
        };

        // Nothing set: config wins over the default.
        let from_config = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |_| None,
            &runtimes,
        );
        assert_eq!(from_config.claude_bin, "/opt/config/claude");
        assert_eq!(from_config.codex_bin, "/opt/config/codex");
        assert!(from_config.codex_bypass_hook_trust);

        // ANTHREX_CLAUDE_BIN wins over config, for Claude only.
        let vars = HashMap::from([("ANTHREX_CLAUDE_BIN", "/opt/env/claude")]);
        let mixed = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |key| vars.get(key).map(|value| (*value).to_string()),
            &runtimes,
        );
        assert_eq!(mixed.claude_bin, "/opt/env/claude");
        assert_eq!(mixed.codex_bin, "/opt/config/codex");

        // ANTHREX_CODEX_BIN wins over config, for Codex only — the mirror image of the
        // Claude-only case above. Coverage gap flagged by the previous task's review:
        // without this case, a `from_vars` that shared one fallback expression between
        // `claude_bin` and `codex_bin` (reading only `ANTHREX_CLAUDE_BIN`, say) would
        // still pass every test in this file, because nothing exercised "Codex overridden
        // while config also differs from default" on its own.
        let vars = HashMap::from([("ANTHREX_CODEX_BIN", "/opt/env/codex")]);
        let mixed_codex = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |key| vars.get(key).map(|value| (*value).to_string()),
            &runtimes,
        );
        assert_eq!(mixed_codex.claude_bin, "/opt/config/claude");
        assert_eq!(mixed_codex.codex_bin, "/opt/env/codex");

        // Nothing set, no config either: the built-in default.
        let defaults = ManagerConfig::from_vars(
            "/tmp/a.sock".into(),
            "/bin/zsh".into(),
            "/opt/anthrex/bin/anthrex".into(),
            |_| None,
            &config::Config::default().runtimes,
        );
        assert_eq!(defaults.claude_bin, "claude");
        assert_eq!(defaults.codex_bin, "codex");
        assert!(!defaults.codex_bypass_hook_trust);
    }
}
