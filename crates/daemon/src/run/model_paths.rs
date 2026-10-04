//! `Run`'s lookups and path helpers, and the free [`task_branch`] and [`task_path`].
//! Split out of `model.rs` (milestone 9.5) to keep it under the 600-line rule; a pure
//! move. Pure (design decision 2).

use std::path::PathBuf;

use super::{Run, StageLayout, StageRecord, Task};

impl Run {
    /// Whether `self` and `other` store the same `run.json`: equal in everything but
    /// the fields never stored (`orchestrator_base`, M8b.15 re-review minor 1).
    pub fn same_on_disk(&self, other: &Run) -> bool {
        if self.orchestrator_base == other.orchestrator_base {
            return self == other;
        }
        let mut rebased = self.clone();
        rebased.orchestrator_base = other.orchestrator_base;
        rebased == *other
    }

    /// Milestone 9.5 decision 15: every second the run has been paused or halted by
    /// `at`, the span in progress included.
    pub fn paused_total(&self, at: u64) -> u64 {
        let open = self.paused_at.map_or(0, |p| at.saturating_sub(p));
        self.paused_secs.saturating_add(open)
    }

    /// `anthrex/<id>/integration`.
    pub fn run_branch(&self) -> String {
        format!("anthrex/{}/integration", self.id)
    }

    /// `<wt_dir>/runs/<id>/integration`.
    pub fn integration_path(&self) -> PathBuf {
        self.task_path("integration")
    }

    /// `<wt_dir>/runs/<id>/<task>`.
    pub fn task_path(&self, task: &str) -> PathBuf {
        task_path(&self.wt_dir, &self.id, task)
    }

    pub fn review_path(&self, task: &str) -> PathBuf {
        self.task_path(&format!("{task}.review"))
    }

    pub fn proof_path(&self, task: &str) -> PathBuf {
        self.task_path(&format!("{task}.proof"))
    }

    /// `<data_dir>/REPORT.md`.
    pub fn report_path(&self) -> PathBuf {
        self.data_dir.join("REPORT.md")
    }

    /// The run id's 4 hex digits.
    pub fn short(&self) -> &str {
        let cut = self.id.len().saturating_sub(4);
        self.id.get(cut..).unwrap_or(&self.id)
    }

    pub fn task(&self, id: &str) -> Option<&Task> {
        self.tasks.iter().find(|t| t.spec.id == id)
    }

    /// Milestone 9.1 decision 47: stage `n`'s record, when it has been created.
    pub fn stage(&self, n: u16) -> Option<&StageRecord> {
        self.stages.iter().find(|s| s.n == n)
    }

    /// Stage `n`'s head: a `Single` run's one branch is `integration`, so every stage
    /// of it is `run_head`; a `Multi` run's is its record's, `None` until it is created.
    pub fn stage_head(&self, n: u16) -> Option<&str> {
        match self.stage_layout {
            StageLayout::Single => Some(&self.run_head),
            StageLayout::Multi => self.stage(n).map(|s| s.head.as_str()),
        }
    }

    /// The head task-context work starts from, merges into and is measured against:
    /// its stage's head (decision 47), `run_head` while that stage is not created.
    pub fn head_for(&self, task: &Task) -> &str {
        self.stage_head(task.stage()).unwrap_or(&self.run_head)
    }

    /// `integration` for a `Single` run, `anthrex/<run>/stage-<n>` for a `Multi` one.
    pub fn stage_branch(&self, n: u16) -> String {
        match self.stage_layout {
            StageLayout::Single => self.run_branch(),
            StageLayout::Multi => task_branch(&self.id, &format!("stage-{n}")),
        }
    }

    /// `<wt_dir>/runs/<id>/.full`, the tier-3 checkout (decision 17).
    pub fn full_path(&self) -> PathBuf {
        self.task_path(".full")
    }
}

/// `anthrex/<run>/<task>`.
pub fn task_branch(run_id: &str, task: &str) -> String {
    format!("anthrex/{run_id}/{task}")
}

/// `<wt_dir>/runs/<run>/<task>`.
pub fn task_path(wt_dir: &std::path::Path, run_id: &str, task: &str) -> PathBuf {
    wt_dir.join("runs").join(run_id).join(task)
}
