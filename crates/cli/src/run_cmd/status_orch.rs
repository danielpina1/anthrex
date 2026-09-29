//! Milestone 9's part of `anthrex run status` (task M9.14): the orchestrator, planner,
//! hold and summary lines, and the task table's `paused(message)` and ` (held)`. Pure.

use proto::{BlockReason, HoldState, PlannerState, RunInfo, TaskInfo};

/// Milestone 9's lines: the orchestrator's window, route and liveness; each planner
/// with the tasks of its epic; each hold with its tasks; whether the summary is written.
pub(super) fn orchestrator_lines(run: &RunInfo) -> String {
    let mut out = String::new();
    if let Some(o) = &run.orchestrator {
        let window = o.window_id.map_or("-".to_string(), |w| w.to_string());
        let model = if o.route.model.is_empty() {
            "(default)"
        } else {
            o.route.model.as_str()
        };
        out.push_str(&format!(
            "  orchestrator: window {window}, {} {model}, {}\n",
            o.route.runtime.label(),
            if o.live { "live" } else { "exited" }
        ));
    }
    if !run.planners.is_empty() {
        let planners: Vec<String> = run
            .planners
            .iter()
            .map(|p| {
                let n = run
                    .tasks
                    .iter()
                    .filter(|t| t.epic.as_deref() == Some(p.epic.as_str()))
                    .count();
                let tasks = if n == 0 {
                    String::new()
                } else {
                    format!(" ({})", tasks_count(n))
                };
                format!("{} {}{tasks}", p.epic, planner_label(p.state))
            })
            .collect();
        out.push_str(&format!("  planners: {}\n", planners.join(", ")));
    }
    if !run.holds.is_empty() {
        let holds: Vec<String> = run
            .holds
            .iter()
            .map(|h| {
                let n = tasks_count(h.tasks.len());
                format!("{} {} ({n})", h.id, hold_label(h.state))
            })
            .collect();
        out.push_str(&format!("  holds: {}\n", holds.join(", ")));
    }
    if run
        .orchestrator
        .as_ref()
        .is_some_and(|o| o.summary.is_some())
    {
        out.push_str("  summary: written\n");
    }
    out
}

fn tasks_count(n: usize) -> String {
    format!("{n} task{}", if n == 1 { "" } else { "s" })
}

fn planner_label(state: PlannerState) -> &'static str {
    match state {
        PlannerState::Planning => "planning",
        PlannerState::Finished => "finished",
        PlannerState::Failed => "failed",
    }
}

fn hold_label(state: HoldState) -> &'static str {
    match state {
        HoldState::Drafting => "drafting",
        HoldState::Awaiting => "awaiting",
        HoldState::Approved => "approved",
        HoldState::Rejected => "rejected",
    }
}

/// The task table's `STATE`: `paused(message)` for a task stopped by `stop_and_wait`
/// (decision 42c), and ` (held)` after the state of a task whose hold is not approved
/// (decision 28).
pub(super) fn state_text(run: &RunInfo, task: &TaskInfo) -> String {
    let state = match &task.block {
        Some(block) if block.reason == BlockReason::MessagePause => "paused(message)",
        _ => task.state.label(),
    };
    let held = task.hold.as_ref().is_some_and(|id| {
        run.holds
            .iter()
            .any(|h| &h.id == id && h.state != HoldState::Approved)
    });
    if held {
        format!("{state} (held)")
    } else {
        state.to_string()
    }
}
