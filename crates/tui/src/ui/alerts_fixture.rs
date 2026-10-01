//! M9.0.7.5: the milestone's shared alert fixture, `three_runs` (§6.1's mockup): the
//! grown Alerts box's tests, task 6's Alerts view and the render audit draw it.
//! Test-only.

use crate::app::App;
use crate::settings::UiSettings;
use crate::tree::alert_fixtures::blocked;
use crate::tree::run_fixtures::{pty, run, snapshot, task};
use proto::{BlockReason, RunInfo, RunState, Size, Status, TaskEventInfo, TaskState, WindowInfo};
use std::time::Instant;

pub(crate) const REPO: &str = "/tmp/repo";
/// The snapshot's daemon clock.
pub(crate) const NOW: u64 = 1_000_000;
pub(crate) const QUESTION: &str = "which crate owns the formatting helper?";

pub(crate) fn named(id: &str, goal: &str, state: RunState, created_at: u64) -> RunInfo {
    let mut info = run(id, REPO, state);
    info.goal = goal.into();
    info.created_at = created_at;
    info
}

/// `t2` of `add-mul-0723`: blocked on a question, blocked `ago` seconds before `NOW`.
pub(crate) fn blocked_t2(ago: u64) -> proto::TaskInfo {
    let mut t2 = blocked("t2", BlockReason::Question, QUESTION);
    t2.title = "report_product in c".into();
    // Newest first, as the daemon sends it; an older block and a later event around it.
    t2.history = vec![
        event(NOW - ago + 1, "worker round 2 started"),
        event(NOW - ago, &format!("blocked (question): {QUESTION}")),
        event(NOW - ago - 600, "blocked (question): an older question"),
    ];
    t2
}

pub(crate) fn event(at: u64, text: &str) -> TaskEventInfo {
    TaskEventInfo {
        at,
        text: text.into(),
    }
}

/// The three runs of §6.1's mockup, in project `/tmp/repo`.
pub(crate) fn three_runs_snapshot() -> Vec<RunInfo> {
    let mut docs = named("docs-77aa", "Docs", RunState::AwaitingApproval, 1);
    docs.tasks = vec![
        task("t1", "write", Size::S, TaskState::Pending),
        task("t2", "proof", Size::S, TaskState::Pending),
    ];
    let mut mul = named("add-mul-0723", "Add mul()", RunState::Running, 2);
    mul.tasks = vec![
        task("t1", "add", Size::S, TaskState::Merged),
        blocked_t2(41),
    ];
    let mut ci = named("fix-ci-9b1e", "Fix CI", RunState::Complete, 3);
    ci.tasks = vec![task("t1", "fix", Size::S, TaskState::Merged)];
    vec![docs, mul, ci]
}

pub(crate) fn app_of(windows: Vec<WindowInfo>, runs: Vec<RunInfo>) -> App {
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    let _ = app.set_terminal_size(80, 24);
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snapshot(
        NOW, runs,
    ))));
    app.set_runs_received_at(Instant::now());
    app
}

/// The fixture: one shell and the three runs; three alerts (P2, P3, P4).
pub(crate) fn three_runs() -> App {
    app_of(
        vec![pty(1, "shell", REPO, Status::Idle)],
        three_runs_snapshot(),
    )
}
