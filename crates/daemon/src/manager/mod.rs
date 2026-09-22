//! Owns every window, applies status events, and broadcasts the window list.

mod config;
mod create;
mod entry;
mod remove;
mod restart;
mod restore;

pub use config::ManagerConfig;
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
use unicode_segmentation::UnicodeSegmentation;

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

/// Fix wave 6, Minor finding: whether `c` is one of the bidi text-direction override
/// characters — U+202A LRE through U+202E RLO, and U+2066 LRI through U+2069 PDI. Unicode
/// category `Cf` ("format"), not `Cc` ("control"), so `char::is_control()` alone (decision
/// 22 exactly as first written) does not catch it. U+202E RIGHT-TO-LEFT OVERRIDE is the
/// character behind "Trojan Source" spoofing: it reorders a name's *rendered* glyphs
/// without changing its bytes, confirmed live against the daemon (`anthrex rename 1
/// "bad\u{202e}name"` succeeded, and `anthrex ls` rendered the reordered glyphs). That
/// matters specifically because anthrex renders window names as labels distinguishing
/// several agents running side by side with different worktrees and permissions — a window
/// that renders as a different window is a security surface, not a cosmetic one.
///
/// This rejects exactly these nine bidi formatting characters, not the whole `Cf`
/// category: `Cf` also contains U+200D ZERO WIDTH JOINER, required to fuse a legitimate
/// multi-codepoint emoji sequence (a family emoji, `man+ZWJ+woman+ZWJ+girl+ZWJ+boy`) into
/// the single grapheme cluster decision 22's own 64-*grapheme* limit exists to count
/// correctly — rejecting the category would refuse exactly the input that limit was built
/// to accept.
fn is_bidi_override(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// The full set of characters decision 22 refuses in a name: the Unicode `Cc` control
/// category (`char::is_control()`) plus the bidi overrides above. One predicate shared by
/// [`validate_name`] and [`sanitize_name`] so the rule enforced on `create`/`rename` and
/// the rule repaired on `restore` can never drift apart.
fn is_disallowed_name_char(c: char) -> bool {
    c.is_control() || is_bidi_override(c)
}

/// Design decision 22: a window name is trimmed, then must be 1 to 64 characters with no
/// control characters (amended, fix wave 6: and no bidi override characters — see
/// [`is_bidi_override`]). `create` (`manager::create::admit`) and `rename` both call this —
/// one validator, not two copies that could drift — so a name the CLI or TUI cannot get
/// through creation can never be reached through a rename either.
///
/// The 64-character limit is counted in *grapheme clusters*
/// (`UnicodeSegmentation::graphemes`), not bytes and not `char`s: this is the same unit
/// `crates/tui` already uses everywhere it counts or truncates user-facing text (see
/// `tree.rs`, `dialog.rs`, `tree_input.rs`, `ui/tree_view.rs`, `graph/paint.rs`), because a
/// name is rendered in the TUI sidebar and tree, where a multi-codepoint glyph (an accented
/// letter typed as a base character plus a combining mark, a flag, a family emoji) must
/// count as the one character it is perceived and rendered as, not as however many Unicode
/// scalar values happen to encode it.
fn validate_name(name: &str) -> anyhow::Result<String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("name must not be empty");
    }
    if trimmed.graphemes(true).count() > 64 {
        anyhow::bail!("name must be at most 64 characters");
    }
    if trimmed.chars().any(is_disallowed_name_char) {
        anyhow::bail!("name must not contain control characters");
    }
    Ok(trimmed)
}

/// Fix wave 6, Major finding: repairs a name that fails [`validate_name`] instead of
/// refusing it, for `restore` (`manager::restore`) to apply to every record loaded from
/// `state.json` — a file a user can hand-edit, that another tool could write, and that
/// survives across daemon versions, none of which `validate_name` ever saw before this.
///
/// Ruling: sanitize, do not reject. Losing a user's window over a display string is the
/// exact trade this milestone has refused everywhere else (a corrupt *file* is moved aside,
/// never deleted; one bad *record* among good ones is skipped, never the whole load) — a
/// name is a label, not data the user cannot reconstruct, so it is repaired in place.
///
/// Each disallowed character ([`is_disallowed_name_char`] — the exact rule
/// [`validate_name`] enforces, so a sanitized name can never itself fail validation on the
/// next save) is replaced with `_` rather than stripped, so two differently-placed bad
/// characters cannot collapse two names into the same string by deleting the gap between
/// them (`"a\x1bb"` becomes `"a_b"`, not `"ab"`). The result is then truncated to 64
/// grapheme clusters, the same unit and limit `validate_name` counts by. An input that is
/// empty, trims to empty, or sanitizes to nothing usable falls back to `"window-<id>"`,
/// which is always inside the limit and free of disallowed characters.
///
/// Idempotent by construction: every character this function can produce (`_`, and
/// whatever safe characters survived from the input) is itself allowed, and the result is
/// never longer than the limit, so calling this again on its own output is always a no-op —
/// the round-trip stability decision 12's "one bad field must not cost the user their work"
/// promise needs, so a restored name does not change on every subsequent restart.
///
/// This does not resolve a collision with another window's name; the caller
/// (`manager::restore::restore`) does that, the same way it already disambiguates ids.
fn sanitize_name(id: u32, name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return format!("window-{id}");
    }
    let cleaned: String = trimmed
        .chars()
        .map(|c| if is_disallowed_name_char(c) { '_' } else { c })
        .collect();
    let truncated: String = cleaned.graphemes(true).take(64).collect();
    let truncated = truncated.trim();
    if truncated.is_empty() {
        format!("window-{id}")
    } else {
        truncated.to_string()
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
                orphaned_cleanups: Vec::new(),
                runs: Vec::new(),
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
            entries,
            cleanups,
            orphaned_cleanups,
            ..
        } = &mut *inner;
        cleanups.retain(|id, done| entries.contains_key(id) || !*done.borrow());
        orphaned_cleanups.retain(|done| !*done.borrow());
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
        self.kill_reporting_insert(id).map(|_inserted| ())
    }

    /// Same as [`kill`](Self::kill), but also reports whether this call actually
    /// inserted a `cleanups[id]` record, rather than finding one already there or
    /// finding no live child to escalate. `restart.rs`'s `Restarting` guard uses this to
    /// derive ownership of the record structurally instead of asserting it
    /// (final-gate finding F1) — see that module's own doc comment for why "did this
    /// call reach the kill line" and "does this attempt own the record" are not the
    /// same question.
    pub(super) fn kill_reporting_insert(self: &Arc<Self>, id: u32) -> anyhow::Result<bool> {
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
        let name = validate_name(&name)?;
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
            // Every live id's own cleanup, plus any `orphaned_cleanups` a prior restart
            // evicted from `cleanups`: those escalation threads still own a process group
            // this daemon spawned, and exiting before they finish would take them down
            // mid-HUP/TERM/KILL and leave a descendant behind (`entry.rs`'s
            // `orphaned_cleanups` doc comment).
            inner
                .cleanups
                .values()
                .cloned()
                .chain(inner.orphaned_cleanups.iter().cloned())
                .collect()
        };
        for mut done in pending {
            let _ = done.wait_for(|finished| *finished).await;
        }
    }
}
