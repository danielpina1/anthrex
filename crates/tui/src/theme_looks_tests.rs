//! M9.0.7.3: every look resolves through a role (decision 3), "needs you" is one rule
//! (decision 4), and the ASCII twins of borders, guides, bars and punctuation
//! (decision 5).

use crate::theme::*;
use proto::{
    FullInfo, FullState, RunState, StageInfo, Status, SubagentInfo, SubagentState, TaskState,
};
use ratatui::style::{Color, Modifier};

const STATUSES: [Status; 6] = [
    Status::Starting,
    Status::Working,
    Status::Idle,
    Status::Attention,
    Status::Done,
    Status::Exited,
];

const TASK_STATES: [TaskState; 12] = [
    TaskState::Pending,
    TaskState::Queued,
    TaskState::Preparing,
    TaskState::Working,
    TaskState::Proof,
    TaskState::Check,
    TaskState::Review,
    TaskState::MergeQueue,
    TaskState::Merged,
    TaskState::Blocked,
    TaskState::Cancelled,
    TaskState::Reported,
];

const RUN_STATES: [RunState; 9] = [
    RunState::AwaitingApproval,
    RunState::Running,
    RunState::Paused,
    RunState::Halted,
    RunState::Complete,
    RunState::Accepted,
    RunState::Discarded,
    RunState::Failed,
    RunState::Planning,
];

#[test]
fn looks_follow_decision_3() {
    assert_eq!(
        status_look(Status::Attention, 0, false),
        ("⚑", Role::Attention)
    );
    assert_eq!(status_look(Status::Exited, 0, true), ("_", Role::Muted));
    let t = |state, needs_you| TaskLook {
        state,
        gate_open: false,
        held: false,
        paused: false,
        animating: false,
        needs_you,
    };
    assert_eq!(
        task_look(t(TaskState::MergeQueue, false), 0, false),
        ("»", Role::Working)
    );
    assert_eq!(
        task_look(t(TaskState::Blocked, true), 0, false),
        ("⚑", Role::Attention)
    );
    assert_eq!(
        task_look(t(TaskState::Blocked, false), 0, true),
        ("#", Role::Paused)
    );
    assert_eq!(run_look(RunState::Halted, false), ("⚑", Role::Attention));
    assert_eq!(run_look(RunState::Running, true), ("@", Role::Working));
}

/// §5.1 principle 1: progress is never drawn as "needs you".
#[test]
fn working_is_never_attention() {
    for state in [
        TaskState::Preparing,
        TaskState::Working,
        TaskState::Proof,
        TaskState::Check,
        TaskState::Review,
        TaskState::MergeQueue,
    ] {
        for animating in [false, true] {
            let look = TaskLook {
                state,
                gate_open: false,
                held: false,
                paused: false,
                animating,
                needs_you: false,
            };
            assert_ne!(task_look(look, 3, false).1, Role::Attention, "{state:?}");
        }
    }
    assert_eq!(status_look(Status::Working, 3, false).1, Role::Working);
    assert_eq!(run_look(RunState::Running, false).1, Role::Working);
    assert_eq!(run_look(RunState::Planning, false).1, Role::Working);
}

#[test]
fn fold_and_guides_are_ascii() {
    assert_eq!(
        fold("a · b … c → d ‹ e › ⏎", true),
        "a - b ... c -> d < e > enter"
    );
    assert_eq!(guides("│ ├─", true), "| |-");
    assert_eq!(bar(3, 2, true), "###..");
    assert_eq!(fold("a · b", false), "a · b");
}

#[test]
fn ascii_twins_cover_the_table_the_spinner_and_the_git_marks() {
    assert_eq!(
        ascii_twins("done ✓ → proof ✗ ◇ ◐ » ⊘ – ‖ ◉ ◆ ▎ ● ▫ ⚑ ⚠ ⠋ ✕ ⇡2⇣1"),
        "done + → proof x ~ % = # _ \" @ H | * : ! ! - x ^2v1"
    );
    assert_eq!(ascii_twins("plain → text"), "plain → text");
}

#[test]
fn every_fold_twin_is_decision_5s() {
    assert_eq!(fold("← − — ↑ ↓ ☐", true), "<- - - ^ v [ ]");
    assert_eq!(guides("│ ├─└─", true), "| |-`-");
    assert_eq!(guides("│ ├─└─", false), "│ ├─└─");
    assert_eq!(bar(2, 3, false), "██░░░");
    assert_eq!(border_set(true).top_left, "+");
    assert_eq!(border_set(false).top_left, "╭");
}

fn stage(head: Option<&str>, state: FullState) -> StageInfo {
    StageInfo {
        actions: Vec::new(),
        n: 1,
        branch: "anthrex/r/stage-1".into(),
        head: head.map(str::to_owned),
        tasks: 1,
        merged: 0,
        full: FullInfo {
            state,
            ..FullInfo::default()
        },
        fix_tasks: vec![],
        propagate_red: None,
    }
}

fn subagent(state: SubagentState, needs_permission: bool) -> SubagentInfo {
    SubagentInfo {
        id: "s1".into(),
        parent_id: None,
        kind: "task".into(),
        label: None,
        model: None,
        state,
        tool: None,
        started_secs: 0,
        ended_secs: None,
        needs_permission,
    }
}

/// Every `(glyph, role)` of every look, in colour and in ASCII, for every frame.
fn every_look(ascii: bool) -> Vec<(String, &'static str, Role)> {
    let mut out = Vec::new();
    for frame in 0..12 {
        for s in STATUSES {
            let (g, r) = status_look(s, frame, ascii);
            out.push((format!("{s:?}"), g, r));
        }
        for state in TASK_STATES {
            for bits in 0u8..32 {
                let look = TaskLook {
                    state,
                    gate_open: bits & 1 != 0,
                    held: bits & 2 != 0,
                    paused: bits & 4 != 0,
                    animating: bits & 8 != 0,
                    needs_you: bits & 16 != 0,
                };
                let (g, r) = task_look(look, frame, ascii);
                out.push((format!("{look:?}"), g, r));
            }
        }
        for state in [
            SubagentState::Running,
            SubagentState::Done,
            SubagentState::Failed,
        ] {
            for asks in [false, true] {
                let (g, r) = subagent_look(&subagent(state, asks), frame, ascii);
                out.push((format!("{state:?} {asks}"), g, r));
            }
        }
        for head in [None, Some("abc")] {
            for state in [
                FullState::None,
                FullState::Running,
                FullState::Green,
                FullState::Red,
                FullState::Bisecting,
            ] {
                let (g, r) = stage_look(&stage(head, state), frame, ascii);
                out.push((format!("{head:?} {state:?}"), g, r));
            }
        }
    }
    for state in RUN_STATES {
        let (g, r) = run_look(state, ascii);
        out.push((format!("{state:?}"), g, r));
    }
    out
}

/// §6.9's theme test for the new looks: with truecolor off every look is one of the
/// 16 ANSI colours, whatever the configured accent; in ASCII every glyph is ASCII.
#[test]
fn every_look_is_ansi_when_truecolor_is_off() {
    let p = Palette {
        accent: Color::Rgb(1, 2, 3),
        truecolor: false,
        ascii: false,
    };
    for ascii in [false, true] {
        for (what, g, r) in every_look(ascii) {
            let fg = role(r, p).fg.expect("a role has a foreground");
            assert!(
                !matches!(fg, Color::Rgb(..) | Color::Indexed(_)),
                "{what}: {r:?} is {fg:?}"
            );
            assert_eq!(g.is_ascii(), ascii, "{what}: {g:?}");
        }
    }
    for priority in 1..=4 {
        let fg = alert_style(priority, p).fg.expect("an alert has a colour");
        assert!(
            !matches!(fg, Color::Rgb(..) | Color::Indexed(_)),
            "P{priority}"
        );
        assert!(alert_glyph(priority, true).is_ascii());
    }
}

/// Decision 3's table, the rows `looks_follow_decision_3` leaves out.
#[test]
fn the_rest_of_decision_3s_table() {
    let t = |state| TaskLook {
        state,
        gate_open: false,
        held: false,
        paused: false,
        animating: false,
        needs_you: false,
    };
    let table = [
        (TaskState::Pending, "◌", Role::Muted),
        (TaskState::Queued, "▫", Role::Muted),
        (TaskState::Preparing, "●", Role::Working),
        (TaskState::Working, "●", Role::Working),
        (TaskState::Proof, "◇", Role::Working),
        (TaskState::Check, "◇", Role::Working),
        (TaskState::Review, "◐", Role::Working),
        (TaskState::Merged, "✓", Role::Done),
        (TaskState::Reported, "✓", Role::Done),
        (TaskState::Cancelled, "–", Role::Muted),
    ];
    for (state, g, r) in table {
        assert_eq!(task_look(t(state), 0, false), (g, r), "{state:?}");
    }
    let animated = TaskLook {
        animating: true,
        ..t(TaskState::Working)
    };
    assert_eq!(
        task_look(animated, 2, false),
        (spinner(2, false), Role::Working)
    );
    let gate = TaskLook {
        gate_open: true,
        ..t(TaskState::Blocked)
    };
    assert_eq!(task_look(gate, 0, false), ("○", Role::Muted));
    let paused = TaskLook {
        paused: true,
        needs_you: true,
        ..t(TaskState::Blocked)
    };
    assert_eq!(task_look(paused, 0, true), ("\"", Role::Paused));

    let statuses = [
        (Status::Starting, "◌", Role::Muted),
        (Status::Idle, "○", Role::Muted),
        (Status::Exited, "–", Role::Muted),
        (Status::Done, "✓", Role::Done),
    ];
    for (s, g, r) in statuses {
        assert_eq!(status_look(s, 0, false), (g, r), "{s:?}");
    }
    let runs = [
        (RunState::AwaitingApproval, "⚑", Role::Attention),
        (RunState::Planning, "◉", Role::Working),
        (RunState::Paused, "◉", Role::Paused),
        (RunState::Complete, "✓", Role::Done),
        (RunState::Accepted, "✓", Role::Done),
        (RunState::Discarded, "–", Role::Muted),
        (RunState::Failed, "✗", Role::Failed),
    ];
    for (s, g, r) in runs {
        assert_eq!(run_look(s, false), (g, r), "{s:?}");
    }
    let asks = subagent(SubagentState::Running, true);
    assert_eq!(subagent_look(&asks, 0, false), ("⚑", Role::Attention));
    let failed = subagent(SubagentState::Failed, false);
    assert_eq!(subagent_look(&failed, 0, true), ("x", Role::Failed));
    let done = subagent(SubagentState::Done, false);
    assert_eq!(subagent_look(&done, 0, false), ("✓", Role::Done));

    assert_eq!(
        stage_look(&stage(None, FullState::Green), 0, false),
        ("◌", Role::Muted)
    );
    assert_eq!(
        stage_look(&stage(Some("a"), FullState::None), 0, false),
        ("◌", Role::Muted)
    );
    assert_eq!(
        stage_look(&stage(Some("a"), FullState::Bisecting), 4, false),
        ("✗", Role::Failed)
    );
    assert_eq!(
        stage_look(&stage(Some("a"), FullState::Running), 4, true),
        (spinner(4, true), Role::Working)
    );

    let p = Palette {
        accent: DEFAULT_ACCENT,
        truecolor: false,
        ascii: false,
    };
    assert!(alert_style(1, p).add_modifier.contains(Modifier::BOLD));
    for priority in [2, 3] {
        let style = alert_style(priority, p);
        assert_eq!(style.fg, role(Role::Attention, p).fg);
        assert!(!style.add_modifier.contains(Modifier::BOLD), "P{priority}");
    }
    assert_eq!(alert_style(4, p).fg, role(Role::Done, p).fg);
    assert_eq!(
        [1, 2, 3, 4].map(|n| alert_glyph(n, false)),
        ["⚑", "⚑", "⚑", "✓"]
    );
}

/// Review focus 4, decision 4: a blocked task wears `⚑` exactly when it is an
/// alert. A `question` block under a live orchestrator is the orchestrator's to
/// answer (`⊘`); with no orchestrator, or a `human` block, it needs the user (`⚑`).
/// At the plan gate or under a hold awaiting approval the gate's or the hold's alert
/// covers it: no blocked alert, and the task is drawn as planned (`○`).
#[test]
fn a_blocked_task_flags_only_when_it_needs_you() {
    use crate::app::AlertKey;
    use crate::app::alerts::task_needs_you;
    use crate::tree::alert_fixtures::{at, blocked, with_orch};
    use crate::tree::orch_fixtures::hold;
    use proto::{BlockReason, HoldState};

    let question = blocked("t1", BlockReason::Question, "which table?");
    let human = blocked("t2", BlockReason::Human, "please decide");
    let mut live = with_orch(at("r-live", RunState::Running, 1), 11);
    live.tasks = vec![question.clone(), human.clone()];
    let mut bare = at("r-bare", RunState::Running, 2);
    bare.tasks = vec![question.clone()];
    let mut gate = at("r-gate", RunState::AwaitingApproval, 3);
    gate.tasks = vec![human.clone()];
    let mut held = at("r-held", RunState::Running, 4);
    let mut held_task = human;
    held_task.hold = Some("epic:ui".into());
    held.tasks = vec![held_task];
    held.holds = vec![hold("epic:ui", HoldState::Awaiting, &["t2"])];

    assert!(
        !task_needs_you(&live, &live.tasks[0]),
        "the orchestrator answers"
    );
    assert!(task_needs_you(&live, &live.tasks[1]), "a human block");
    assert!(task_needs_you(&bare, &bare.tasks[0]), "no orchestrator");
    assert!(!task_needs_you(&gate, &gate.tasks[0]), "the gate covers it");
    assert!(!task_needs_you(&held, &held.tasks[0]), "the hold covers it");

    // The drawn glyph and role, and whether the alerts list the task.
    let drawn = |run: &proto::RunInfo, id: &str| {
        let mut app = crate::app::App::new(Vec::new(), "/tmp".into(), Default::default());
        let _ = app.run_subscription();
        app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(
            crate::tree::run_fixtures::snapshot(10, vec![run.clone()]),
        )));
        let rows = crate::tree::run_rows(
            &app.runs.runs[0],
            &app.windows,
            &app.tree,
            crate::tree::RunFilter::All,
        );
        let row = rows
            .iter()
            .find(
                |row| matches!(&row.kind, crate::tree::RowKind::Task { task, .. } if task.id == id),
            )
            .expect("the task's row");
        let (glyph, role) = crate::graph::paint::style::node_glyph(row, &app);
        let key = AlertKey::Blocked {
            run: run.run_id.clone(),
            task: id.into(),
        };
        let alerted = crate::app::alerts(&app).iter().any(|a| a.key == key);
        (glyph, role, alerted)
    };
    assert_eq!(drawn(&live, "t1"), ("⊘", Role::Paused, false));
    assert_eq!(drawn(&live, "t2"), ("⚑", Role::Attention, true));
    assert_eq!(drawn(&bare, "t1"), ("⚑", Role::Attention, true));
    assert_eq!(drawn(&gate, "t2"), ("○", Role::Muted, false));
    assert_eq!(drawn(&held, "t2"), ("○", Role::Muted, false));
}
