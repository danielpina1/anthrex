//! M9.0.7.5: what an alert carries (decision 7): who it is for, its task, its one-line
//! text and its full detail. Pure reducer tests over `app::alerts`.

use super::super::alerts::every_app;
use super::super::runs::app_with_runs;
use crate::app::{AlertKey, AlertWho, alerts};
use crate::tree::alert_fixtures::{at, blocked};
use crate::tree::run_fixtures::{GOAL, snapshot, task};
use proto::{BlockReason, RunState, Size, TaskState};

fn texts(app: &crate::app::App) -> Vec<String> {
    alerts(app).into_iter().map(|alert| alert.text).collect()
}

#[test]
fn alert_texts_follow_decision_7() {
    // Every source: the orchestrator's three texts unchanged, the gate, the hold (M9.9.8:
    // c-held's `Human` block is its living orchestrator's, no longer listed), a
    // question without a reason, the halted reason's first
    // line, the complete run and the proposal.
    assert_eq!(
        texts(&every_app()),
        [
            "orchestrator asks for permission",
            "orchestrator waits at a start prompt",
            "orchestrator wake-up held",
            "plan awaits approval · 1 task",
            "hold epic:ui awaits approval · 1 task",
            "blocked: which db?",
            "run halted: disk full",
            "ready to accept · 2/2 merged",
            "profile proposal ready",
        ]
    );
    // A block with no text: `blocked` for a question, else the reason alone; a task
    // blocked with no block at all reads `blocked`.
    let mut r = at("r", RunState::Running, 1);
    let mut bare = task("t3", "bare", Size::S, TaskState::Blocked);
    bare.block = None;
    r.tasks = vec![
        blocked("t1", BlockReason::Question, " \n"),
        blocked("t2", BlockReason::Environment, ""),
        bare,
        blocked(
            "t4",
            BlockReason::Conflict,
            "\nmerge conflict in a.rs\nsecond",
        ),
    ];
    let app = app_with_runs(vec![], snapshot(1, vec![r]));
    assert_eq!(
        texts(&app),
        [
            "blocked",
            "blocked (environment)",
            "blocked",
            "blocked (conflict): merge conflict in a.rs",
        ]
    );
    // The task id is not in the text: it is the alert's `task`.
    let tasks: Vec<_> = alerts(&app).into_iter().map(|alert| alert.task).collect();
    assert_eq!(tasks, ["t1", "t2", "t3", "t4"].map(|t| Some(t.to_owned())));
    // A halted run with no reason; a gate of two tasks.
    let mut gate = at("g", RunState::AwaitingApproval, 2);
    gate.tasks = vec![
        task("t1", "a", Size::S, TaskState::Pending),
        task("t2", "b", Size::S, TaskState::Pending),
    ];
    let app = app_with_runs(
        vec![],
        snapshot(1, vec![at("h", RunState::Halted, 1), gate]),
    );
    assert_eq!(
        texts(&app),
        ["plan awaits approval · 2 tasks", "run halted"]
    );
}

#[test]
fn blocked_alerts_carry_the_task_and_the_full_detail() {
    let all = alerts(&every_app());
    // M9.9.8: with a living orchestrator the `Human` block is its own, so this run has
    // none.
    let mut r = at("c-held", RunState::Running, 3);
    r.tasks = vec![blocked(
        "t1",
        BlockReason::Human,
        "needs a key\nsecond line",
    )];
    let human_app = app_with_runs(vec![], snapshot(1, vec![r]));
    let human_all = alerts(&human_app);
    let human = human_all
        .iter()
        .find(|alert| {
            alert.key
                == AlertKey::Blocked {
                    run: "c-held".into(),
                    task: "t1".into(),
                }
        })
        .unwrap();
    assert_eq!(
        human.who,
        AlertWho::Run {
            goal: GOAL.into(),
            id: "c-held".into()
        }
    );
    assert_eq!(human.task.as_deref(), Some("t1"));
    assert_eq!(human.text, "blocked (human): needs a key");
    // The whole block text, every line: the Alerts view draws it.
    assert_eq!(human.detail, "needs a key\nsecond line");
    // No `blocked (` history entry in the fixture: no age.
    assert_eq!(human.age, None);
    // A halted run's detail is its whole reason; a run alert has no task.
    let halted = all
        .iter()
        .find(|alert| alert.key == AlertKey::Halted("e-halt".into()))
        .unwrap();
    assert_eq!(halted.task, None);
    assert_eq!(halted.detail, "disk full\nat /tmp");
    // Where there is nothing longer, the detail is the text; a proposal is for its
    // project, by its directory name, and aged by its `updated_at`.
    let gate = all
        .iter()
        .find(|alert| alert.key == AlertKey::Gate("b-gate".into()))
        .unwrap();
    assert_eq!(gate.detail, gate.text);
    let proposal = all.last().unwrap();
    assert_eq!(proposal.who, AlertWho::Project("shop".into()));
    // The snapshot's clock moves on from its arrival: a second's slack.
    assert!(
        matches!(proposal.age, Some(1_000..=1_001)),
        "{:?}",
        proposal.age
    );
}

/// Fix round 1 ruling (Review focus 4): an orchestrator's alert is aged by its
/// window's time in its status only where that status is the wait: asking for
/// permission, or quiet at a start prompt. A held wake-up has no age: the window's
/// status time is not when the wake was held.
#[test]
fn a_held_wake_up_has_no_age() {
    let ages: Vec<(AlertKey, Option<u64>)> = alerts(&every_app())
        .into_iter()
        .filter(|alert| matches!(alert.key, AlertKey::Orchestrator(_)))
        .map(|alert| (alert.key, alert.age))
        .collect();
    let key = |id: &str| AlertKey::Orchestrator(id.into());
    assert_eq!(
        ages,
        [
            (key("a-attn"), Some(0)),
            (key("b-gate"), Some(0)),
            (key("c-held"), None)
        ]
    );
}
