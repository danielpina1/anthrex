//! The orchestrator's PTY window (milestone 9 decisions 5, 10, 11 and 11a): the one
//! interactive run window. `create_run_window` runs `create`'s three phases with the
//! role's flags and no worktree; the run-live flag guards it against `kill` and
//! `remove`; client input time is what a wake-up waits out (decision 39). I/O only in
//! `create_run_window`'s phase B, on `spawn_blocking`, holding no lock (AGENTS.md rule 2).

use super::WindowManager;
use super::create::{Admitted, spawn_window};
use crate::launch::role::RoleLaunch;
use crate::project::DetectedRoots;
use proto::{RunRef, WindowInfo, WindowKind, WindowSpec};
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Instant;

/// Decision 5: the orchestrator's PTY until the first client `Resize`.
pub const RUN_WINDOW_SIZE: (u16, u16) = (200, 50);

/// Decision 11's refusal of `kill` and `remove` for a live run's orchestrator.
pub fn orchestrator_refusal(id: u32, run_id: &str) -> String {
    format!(
        "window {id} is the orchestrator of run {run_id}; stop the run with anthrex run cancel, or restart the orchestrator with anthrex restart {id}"
    )
}

/// M9.10 review (the controller's ruling): the refusal of `Restart` and `Input` for a
/// restored window whose role did not parse.
pub fn lost_role_refusal(id: u32, run_id: &str) -> String {
    format!(
        "window {id} was the orchestrator of run {run_id}, and its role could not be restored; it cannot be restarted or typed into. Remove it with anthrex rm {id}"
    )
}

/// The run a Pty record's unparseable role names (`"unknown"` when even that is
/// unreadable), or `None` when the record has no role or its role parses.
pub(super) fn lost_run(kind: WindowKind, run: Option<&serde_json::Value>) -> Option<String> {
    let role = run?
        .get("role_launch")
        .filter(|_| kind == WindowKind::Pty)?;
    if RoleLaunch::deserialize(role).is_ok() {
        return None;
    }
    let run_id = role.pointer("/run_ref/run_id").and_then(|v| v.as_str());
    Some(run_id.unwrap_or("unknown").to_string())
}

/// M9.10 review (the controller's ruling, over decision 11's plain-window fallback): the
/// run of an entry restored with a role that did not parse. Such a window never runs
/// again: without its role it would start as a plain agent, with none of the read-only
/// flags, no user-settings-only flags and no scrub.
pub(super) fn lost_role(entry: &super::entry::Entry) -> Option<String> {
    let kind = if entry.is_headless() {
        WindowKind::Headless
    } else {
        WindowKind::Pty
    };
    entry
        .role
        .is_none()
        .then(|| lost_run(kind, entry.run.as_ref()))?
}

/// Decision 11: a Pty record's `run` is `{"role_launch": <RoleLaunch>}`.
pub(super) fn role_record(role: &RoleLaunch) -> serde_json::Value {
    serde_json::json!({ "role_launch": role })
}

/// Decision 11: the role of a restored Pty record, or `None` (with a warning when its
/// `run` does not parse: the window comes back as a plain PTY window).
pub(super) fn restored_role(
    id: u32,
    kind: WindowKind,
    run: Option<&serde_json::Value>,
) -> Option<RoleLaunch> {
    #[derive(serde::Deserialize)]
    struct Record {
        role_launch: RoleLaunch,
    }
    if kind != WindowKind::Pty {
        return None;
    }
    match Record::deserialize(run?) {
        Ok(record) => Some(record.role_launch),
        Err(error) => {
            let run = lost_run(kind, run).unwrap_or_default();
            tracing::warn!(
                id,
                %error,
                "restore: window {id}'s role in run {run} does not parse; it is restored, and never restarted"
            );
            None
        }
    }
}

/// Decision 11a's placeholder: no run, no MCP target, no session to resume.
pub(super) fn placeholder_spec(
    runtime: proto::Runtime,
    cwd: &std::path::Path,
) -> crate::headless::HeadlessSpec {
    crate::headless::HeadlessSpec {
        runtime,
        model: String::new(),
        effort: proto::Effort::Medium,
        cwd: cwd.to_path_buf(),
        instructions: String::new(),
        mcp: None,
        allowed_tools: Vec::new(),
        claude_permission_mode: None,
        claude_disallowed_tools: Vec::new(),
        claude_sandbox: None,
        codex_sandbox: String::new(),
        codex_writable_roots: Vec::new(),
        env: Vec::new(),
        claude_auth: config::ClaudeAuth::Login,
        api_key_helper: None,
        run_ref: None,
        codex_config_guard: None,
        output_filter: None,
    }
}

impl WindowManager {
    /// Creates a run's orchestrator window: `create`'s phases A, B and C with the role's
    /// flags, no worktree, and a 200 × 50 PTY. Only the engine's driver calls this; no
    /// client message can set a role.
    pub async fn create_run_window(
        &self,
        spec: WindowSpec,
        project: PathBuf,
        role: RoleLaunch,
    ) -> anyhow::Result<WindowInfo> {
        anyhow::ensure!(
            spec.worktree_branch.is_none(),
            "a run window never has a worktree"
        );
        let roots = DetectedRoots {
            project,
            worktree: None,
            detection_failed: false,
        };
        self.config.launch_gate.wait().await;
        let (id, reservation) = self.admit(&spec, &roots.project)?;
        let name = reservation.name.clone();
        let config = self.config.clone();
        let events = self.events.clone();
        let phase_b_name = name.clone();
        let phase_b_roots = roots.clone();
        let (cols, rows) = RUN_WINDOW_SIZE;
        let spawned = tokio::task::spawn_blocking(move || {
            let admitted = Admitted {
                id,
                name: &phase_b_name,
                cols,
                rows,
            };
            spawn_window(&config, events, spec, &phase_b_roots, &admitted, Some(role))
        })
        .await
        .map_err(|error| anyhow::anyhow!("window creation failed: {error}"))??;
        self.insert(id, name, roots.project, None, spawned, reservation)
    }

    /// Decision 11: set by the driver while the window's run is live. Under the lock, no
    /// I/O.
    pub fn set_run_window_live(&self, id: u32, live: bool) {
        let mut inner = crate::lock(&self.inner);
        if let Some(entry) = inner.entries.get_mut(&id) {
            entry.run_live = live;
        }
    }

    /// Decision 11 for an ended run (M9.13a re-review, item 5): clears `id`'s live flag
    /// only while `id` is still that run's window. A window id a restart gave to
    /// another run's window keeps that run's flag. Under the lock, no I/O.
    pub fn end_run_window(&self, id: u32, run_id: &str) {
        let mut inner = crate::lock(&self.inner);
        if let Some(entry) = inner.entries.get_mut(&id)
            && entry
                .role
                .as_ref()
                .is_some_and(|role| role.run_ref.run_id == run_id)
        {
            entry.run_live = false;
        }
    }

    /// M9.13 review, item 1: replaces run window `id`'s role environment with
    /// `refresh(<its current one>)`, in the persisted record too, so the next restart
    /// launches with it. Under the lock, no I/O (`refresh` must be pure). `false` when
    /// `id` is not a window with a role.
    pub fn update_role_env(
        &self,
        id: u32,
        refresh: impl FnOnce(&[(String, String)]) -> Vec<(String, String)>,
    ) -> bool {
        let mut inner = crate::lock(&self.inner);
        let Some(entry) = inner.entries.get_mut(&id) else {
            return false;
        };
        let Some(role) = entry.role.as_mut() else {
            return false;
        };
        role.env = refresh(&role.env);
        entry.run = Some(role_record(role));
        true
    }

    /// Tests only: holds (or releases) `id`'s `restarting` flag as a restart in flight
    /// would, so the driver's exit check can be seen during one.
    #[cfg(test)]
    pub(crate) fn hold_restarting(&self, id: u32, restarting: bool) {
        let mut inner = crate::lock(&self.inner);
        if let Some(entry) = inner.entries.get_mut(&id) {
            entry.restarting = restarting;
        }
    }

    /// Whether a restart of `id` is under way (M9.13 review: its exit then is the
    /// restart's, not the program's). Under the lock, no I/O.
    pub fn is_restarting(&self, id: u32) -> bool {
        let inner = crate::lock(&self.inner);
        inner.entries.get(&id).is_some_and(|entry| entry.restarting)
    }

    /// `Some(run)` while `id` is a run window whose run is live: the guard's check.
    pub fn run_window_live(&self, id: u32) -> Option<RunRef> {
        let inner = crate::lock(&self.inner);
        let entry = inner.entries.get(&id)?;
        let role = entry.role.as_ref().filter(|_| entry.run_live)?;
        Some(role.run_ref.clone())
    }

    /// Records a client's `Input` for `id`. Under the lock, no I/O.
    pub fn note_client_input(&self, id: u32) {
        let mut inner = crate::lock(&self.inner);
        if let Some(entry) = inner.entries.get_mut(&id) {
            entry.last_client_input = Some(Instant::now());
        }
    }

    /// When a client's `Input` last reached `id`.
    pub fn last_client_input(&self, id: u32) -> Option<Instant> {
        let inner = crate::lock(&self.inner);
        inner.entries.get(&id)?.last_client_input
    }

    /// Whole-branch review, item 2: `id` entered `Attention` and its turn has not ended
    /// since (`Entry::attention_open`); false for a window that is gone.
    pub fn attention_open(&self, id: u32) -> bool {
        let inner = crate::lock(&self.inner);
        inner.entries.get(&id).is_some_and(|e| e.attention_open)
    }

    /// Whole-branch fix round 2, item 2: how long `id` has been `Idle` or `Done` with
    /// its `attention_open` set (a wake-up is held only by that), else `None`.
    pub fn held_at_prompt_for(&self, id: u32) -> Option<std::time::Duration> {
        let inner = crate::lock(&self.inner);
        let e = inner.entries.get(&id)?;
        let at_prompt = matches!(e.status, proto::Status::Idle | proto::Status::Done);
        (e.attention_open && at_prompt).then(|| e.since.elapsed())
    }

    /// A client's `Input`: noted, then written. The engine's own writes (a wake-up) go
    /// through `write_input` and are not client input.
    pub fn write_client_input(&self, id: u32, bytes: &[u8]) -> anyhow::Result<()> {
        self.note_client_input(id);
        self.write_input(id, bytes)
    }

    /// M9.10 review: `Some(run)` when `id` was restored with a role that did not parse;
    /// the guard refuses its `Restart` and `Input`.
    pub fn lost_role_run(&self, id: u32) -> Option<String> {
        let inner = crate::lock(&self.inner);
        lost_role(inner.entries.get(&id)?)
    }

    /// Decision 11a: a headless window restored from a record that did not parse, which
    /// `kill` and `remove` may reach.
    pub fn is_placeholder_headless(&self, id: u32) -> bool {
        let inner = crate::lock(&self.inner);
        inner.entries.get(&id).is_some_and(|entry| {
            matches!(&entry.process, super::entry::Process::Headless(window) if window.placeholder)
        })
    }
}
