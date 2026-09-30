//! M9.0.5.8: the alerts, computed (decisions 17–19), and the Alerts box's focus
//! (decision 21). Pure reducer tests: each asserts the state and the returned effects.

use super::runs::{app_with_runs, deliver};
use super::*;
use crate::app::{AlertKey, alerts};
use crate::tree::orch_fixtures::{hold, orchestrator_info};
use crate::tree::run_fixtures::{pty, run, run_ref, snapshot, task, worker};
use proto::{
    AgentRole, BlockInfo, BlockReason, HoldState, ProposalAlertInfo, RunInfo, RunState,
    RunsSnapshot, Size, TaskState,
};

const PROJECT: &str = "/r/demo";

/// A PTY orchestrator window for `run_id`: Claude, `status`, `signals_seen` as given.
pub(super) fn orch_window(id: u32, run_id: &str, status: Status, signals_seen: bool) -> WindowInfo {
    let mut window = pty(id, &format!("orch-{run_id}"), PROJECT, status);
    window.runtime = Runtime::Claude;
    window.run = Some(run_ref(run_id, None, AgentRole::Orchestrator, 1));
    window.signals_seen = signals_seen;
    window
}

pub(super) fn at(id: &str, state: RunState, created_at: u64) -> RunInfo {
    let mut info = run(id, PROJECT, state);
    info.created_at = created_at;
    info
}

pub(super) fn blocked(id: &str, reason: BlockReason, text: &str) -> proto::TaskInfo {
    let mut t = task(id, "work", Size::S, TaskState::Blocked);
    t.block = Some(BlockInfo {
        reason,
        text: text.into(),
    });
    t
}

pub(super) fn with_orch(mut info: RunInfo, window: u32) -> RunInfo {
    info.orchestrator = Some(orchestrator_info(Some(window)));
    info
}

/// Every source at once (decision 18):
/// - `a-attn`: its orchestrator asks for permission;
/// - `b-gate`: its orchestrator waits at a start prompt, and its plan awaits approval
///   (two tasks, one cancelled);
/// - `c-held`: a held wake-up, a hold awaiting approval, a `Human` block, a `Question`
///   block with its orchestrator live (no alert), a paused task (no alert) and a
///   worker round that just ended a turn (no alert);
/// - `d-bare`: no orchestrator, a `Question` block;
/// - `e-halt`: halted, with a two-line reason;
/// - `f-done`: complete, two of three tasks merged, one cancelled;
/// - and a ready profile proposal for `/r/shop`.
pub(super) fn every_source() -> (RunsSnapshot, Vec<WindowInfo>) {
    let a = with_orch(at("a-attn", RunState::Running, 1), 11);

    let mut b = with_orch(at("b-gate", RunState::AwaitingApproval, 2), 12);
    b.tasks = vec![
        task("t1", "one", Size::S, TaskState::Pending),
        task("t2", "two", Size::S, TaskState::Cancelled),
    ];

    let mut c = with_orch(at("c-held", RunState::Running, 3), 13);
    if let Some(orch) = &mut c.orchestrator {
        orch.wake_held = true;
    }
    let mut done_turn = task("t4", "turn", Size::S, TaskState::Working);
    let mut round = worker(1, Some(20), Runtime::Claude, 0);
    round.turns = 1;
    round.turn_open = false;
    done_turn.rounds = vec![round];
    let mut held = task("t5", "ui", Size::S, TaskState::Pending);
    held.hold = Some("epic:ui".into());
    c.tasks = vec![
        blocked("t1", BlockReason::Human, "needs a key\nsecond line"),
        blocked("t2", BlockReason::Question, "which db?"),
        blocked("t3", BlockReason::MessagePause, "hold on"),
        done_turn,
        held,
    ];
    c.holds = vec![hold("epic:ui", HoldState::Awaiting, &["t5"])];

    let mut d = at("d-bare", RunState::Running, 4);
    d.tasks = vec![blocked("t1", BlockReason::Question, "which db?")];

    let mut e = at("e-halt", RunState::Halted, 5);
    e.halted_reason = Some("disk full\nat /tmp".into());

    let mut f = at("f-done", RunState::Complete, 6);
    f.tasks = vec![
        task("t1", "one", Size::S, TaskState::Merged),
        task("t2", "two", Size::S, TaskState::Merged),
        task("t3", "three", Size::S, TaskState::Cancelled),
    ];

    let mut snap = snapshot(10_000, vec![f, e, d, c, b, a]);
    snap.proposals = vec![ProposalAlertInfo {
        project: "/r/shop".into(),
        updated_at: 9_000,
    }];
    let windows = vec![
        pty(1, "shell", PROJECT, Status::Idle),
        orch_window(11, "a-attn", Status::Attention, true),
        orch_window(12, "b-gate", Status::Idle, false),
        orch_window(13, "c-held", Status::Working, true),
    ];
    (snap, windows)
}

pub(super) fn listed(app: &App) -> Vec<(u8, String, String)> {
    alerts(app)
        .into_iter()
        .map(|alert| (alert.priority, alert.label, alert.text))
        .collect()
}

pub(super) fn line(priority: u8, label: &str, text: &str) -> (u8, String, String) {
    (priority, label.into(), text.into())
}

pub(super) fn every_app() -> App {
    let (snap, windows) = every_source();
    app_with_runs(windows, snap)
}

#[test]
fn alerts_in_priority_order() {
    let app = every_app();
    assert_eq!(
        listed(&app),
        vec![
            line(1, "a-attn", "orchestrator asks for permission"),
            line(1, "b-gate", "orchestrator waits at a start prompt"),
            line(1, "c-held", "orchestrator wake-up held"),
            line(2, "b-gate", "plan awaits approval · 1 task"),
            line(2, "c-held", "hold epic:ui awaits approval · 1 task"),
            line(3, "c-held", "t1 blocked (human): needs a key"),
            line(3, "d-bare", "t1 blocked (question): which db?"),
            line(3, "e-halt", "run halted: disk full"),
            line(4, "f-done", "ready to accept · 2/2 merged"),
            line(4, "shop", "profile proposal ready"),
        ]
    );
    let keys: Vec<AlertKey> = alerts(&app).into_iter().map(|a| a.key).collect();
    assert_eq!(
        keys,
        vec![
            AlertKey::Orchestrator("a-attn".into()),
            AlertKey::Orchestrator("b-gate".into()),
            AlertKey::Orchestrator("c-held".into()),
            AlertKey::Gate("b-gate".into()),
            AlertKey::Hold {
                run: "c-held".into(),
                hold: "epic:ui".into()
            },
            AlertKey::Blocked {
                run: "c-held".into(),
                task: "t1".into()
            },
            AlertKey::Blocked {
                run: "d-bare".into(),
                task: "t1".into()
            },
            AlertKey::Halted("e-halt".into()),
            AlertKey::Accept("f-done".into()),
            AlertKey::Proposal("/r/shop".into()),
        ]
    );
}

/// One run with its orchestrator on window 11, shown as `status`/`signals_seen`.
fn p1(status: Status, signals_seen: bool, runtime: Runtime) -> Vec<(u8, String, String)> {
    let info = with_orch(at("r", RunState::Running, 1), 11);
    let mut window = orch_window(11, "r", status, signals_seen);
    window.runtime = runtime;
    listed(&app_with_runs(vec![window], snapshot(1, vec![info])))
}

#[test]
fn each_priority_rule() {
    use Status::*;
    let asks = vec![line(1, "r", "orchestrator asks for permission")];
    let prompt = vec![line(1, "r", "orchestrator waits at a start prompt")];
    // P1: the window's status and signals.
    assert_eq!(p1(Attention, true, Runtime::Claude), asks);
    assert_eq!(p1(Idle, false, Runtime::Claude), prompt);
    assert_eq!(p1(Done, false, Runtime::Codex), prompt);
    assert_eq!(p1(Attention, false, Runtime::Codex), prompt);
    assert_eq!(p1(Idle, true, Runtime::Claude), vec![]);
    assert_eq!(p1(Working, false, Runtime::Claude), vec![]);
    assert_eq!(p1(Idle, false, Runtime::Shell), vec![]);
    assert_eq!(p1(Attention, true, Runtime::Shell), asks);
    // First match wins: a held wake-up over a window asking for permission.
    let mut both = with_orch(at("r", RunState::Running, 1), 11);
    if let Some(orch) = &mut both.orchestrator {
        orch.wake_held = true;
    }
    let window = orch_window(11, "r", Attention, true);
    let app = app_with_runs(vec![window], snapshot(1, vec![both]));
    assert_eq!(
        listed(&app),
        vec![line(1, "r", "orchestrator wake-up held")]
    );
    // P1: not live, or its window not listed.
    let mut info = with_orch(at("r", RunState::Running, 1), 11);
    if let Some(orch) = &mut info.orchestrator {
        orch.live = false;
        orch.wake_held = true;
    }
    let window = orch_window(11, "r", Attention, true);
    let app = app_with_runs(vec![window], snapshot(1, vec![info.clone()]));
    assert_eq!(listed(&app), vec![]);
    let mut listed_elsewhere = with_orch(at("r", RunState::Running, 1), 99);
    if let Some(orch) = &mut listed_elsewhere.orchestrator {
        orch.wake_held = true;
    }
    let window = orch_window(11, "r", Attention, true);
    let app = app_with_runs(vec![window], snapshot(1, vec![listed_elsewhere]));
    assert_eq!(listed(&app), vec![]);

    // P2: several tasks; a hold drafting or decided is not an alert.
    let mut gate = at("r", RunState::AwaitingApproval, 1);
    gate.tasks = vec![
        task("t1", "a", Size::S, TaskState::Pending),
        task("t2", "b", Size::S, TaskState::Pending),
    ];
    let app = app_with_runs(vec![], snapshot(1, vec![gate]));
    assert_eq!(
        listed(&app),
        vec![line(2, "r", "plan awaits approval · 2 tasks")]
    );
    let mut holds = at("r", RunState::Running, 1);
    holds.holds = vec![
        hold("h1", HoldState::Drafting, &["t1"]),
        hold("h2", HoldState::Approved, &["t1"]),
        hold("h3", HoldState::Rejected, &["t1"]),
        hold("h4", HoldState::Awaiting, &["t1", "t2"]),
    ];
    let app = app_with_runs(vec![], snapshot(1, vec![holds]));
    assert_eq!(
        listed(&app),
        vec![line(2, "r", "hold h4 awaits approval · 2 tasks")]
    );

    // P3: while the orchestrator lives, only Human, Conflict and Environment.
    let mut live = with_orch(at("r", RunState::Running, 1), 11);
    live.tasks = vec![
        blocked("t1", BlockReason::Conflict, "merge conflict in a.rs"),
        blocked("t2", BlockReason::Environment, ""),
        blocked("t3", BlockReason::MisSized, "too big"),
        blocked("t4", BlockReason::DepCancelled, "t0 cancelled"),
        blocked("t5", BlockReason::Question, "which?"),
        blocked("t6", BlockReason::MessagePause, "wait"),
    ];
    let window = orch_window(11, "r", Working, true);
    let app = app_with_runs(vec![window], snapshot(1, vec![live.clone()]));
    assert_eq!(
        listed(&app),
        vec![
            line(3, "r", "t1 blocked (conflict): merge conflict in a.rs"),
            line(3, "r", "t2 blocked (environment)"),
        ]
    );
    // With no live orchestrator every reason but a pause.
    live.orchestrator = None;
    let app = app_with_runs(vec![], snapshot(1, vec![live]));
    let texts: Vec<String> = listed(&app).into_iter().map(|(_, _, t)| t).collect();
    assert_eq!(
        texts,
        [
            "t1 blocked (conflict): merge conflict in a.rs",
            "t2 blocked (environment)",
            "t3 blocked (mis-sized): too big",
            "t4 blocked (dependency cancelled): t0 cancelled",
            "t5 blocked (question): which?",
        ]
    );
    // A halted run with no reason.
    let app = app_with_runs(vec![], snapshot(1, vec![at("r", RunState::Halted, 1)]));
    assert_eq!(listed(&app), vec![line(3, "r", "run halted")]);

    // P4, and runs that ask nothing: planning, paused, running quietly, accepted.
    let mut complete = at("r", RunState::Complete, 1);
    complete.tasks = vec![task("t1", "a", Size::S, TaskState::Merged)];
    let app = app_with_runs(vec![], snapshot(1, vec![complete]));
    assert_eq!(
        listed(&app),
        vec![line(4, "r", "ready to accept · 1/1 merged")]
    );
    for state in [
        RunState::Planning,
        RunState::Paused,
        RunState::Running,
        RunState::Accepted,
        RunState::Discarded,
    ] {
        let app = app_with_runs(vec![], snapshot(1, vec![at("r", state, 1)]));
        assert_eq!(listed(&app), vec![], "{state:?}");
    }
}

#[test]
fn alert_labels_and_text_are_one_line() {
    let bad = crate::safe_text::tests::hostile_text();
    let mut gate = at(&format!("r{bad}"), RunState::Halted, 1);
    gate.halted_reason = Some(format!("{bad}\nsecond"));
    gate.tasks = vec![blocked(&format!("t{bad}"), BlockReason::Human, &bad)];
    let mut snap = snapshot(1, vec![gate]);
    snap.proposals = vec![ProposalAlertInfo {
        project: PathBuf::from(format!("/r/p{bad}")),
        updated_at: 1,
    }];
    let app = app_with_runs(vec![], snap);
    let all = alerts(&app);
    assert_eq!(all.len(), 3);
    for alert in all {
        for text in [&alert.label, &alert.text] {
            assert_eq!(
                crate::safe_text::tests::first_hostile(text),
                None,
                "{text:?}"
            );
        }
    }
}

#[test]
fn alerts_clear_when_resolved() {
    let mut app = every_app();
    assert!(
        alerts(&app)
            .iter()
            .any(|a| a.key == AlertKey::Gate("b-gate".into()))
    );
    let (mut snap, _) = every_source();
    let b = snap.runs.iter_mut().find(|r| r.run_id == "b-gate").unwrap();
    b.state = RunState::Running;
    let c = snap.runs.iter_mut().find(|r| r.run_id == "c-held").unwrap();
    c.holds[0].state = HoldState::Approved;
    c.tasks[0].state = TaskState::Working;
    deliver(&mut app, snap);
    let keys: Vec<AlertKey> = alerts(&app).into_iter().map(|a| a.key).collect();
    assert!(!keys.contains(&AlertKey::Gate("b-gate".into())), "{keys:?}");
    assert!(
        !keys.iter().any(|k| matches!(k, AlertKey::Hold { .. })),
        "{keys:?}"
    );
    assert!(
        !keys.contains(&AlertKey::Blocked {
            run: "c-held".into(),
            task: "t1".into()
        }),
        "{keys:?}"
    );
}

#[test]
fn alert_colours_by_priority() {
    use ratatui::style::Color;
    let colours: Vec<Color> = (1..=4).map(crate::theme::alert_color).collect();
    assert_eq!(
        colours,
        [Color::Red, Color::Yellow, Color::Magenta, Color::Green]
    );
}
