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
            tracing::warn!(
                id,
                %error,
                "restore: window {id}'s role does not parse; restored as a plain PTY window"
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

    /// A client's `Input`: noted, then written. The engine's own writes (a wake-up) go
    /// through `write_input` and are not client input.
    pub fn write_client_input(&self, id: u32, bytes: &[u8]) -> anyhow::Result<()> {
        self.note_client_input(id);
        self.write_input(id, bytes)
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
