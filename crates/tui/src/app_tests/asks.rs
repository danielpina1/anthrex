//! Milestone 9.9.8 (OFA §4.1, §4.3, §5): a living orchestrator takes the alerts it can
//! act on, a stuck one gives them back, `ask_user` and "stuck" are alerts of their own,
//! and the user answers a question with `1`-`9` or Enter in the window. Reducer tests.

use super::alerts::{line, listed};
use super::runs::app_with_runs;
use super::*;
use crate::app::{AlertKey, alerts};
use crate::tree::alert_fixtures::{at, blocked, orch_window, with_orch};
use crate::tree::run_fixtures::snapshot;
use crate::tree::stage_fixtures::stage;
use proto::{
    AskInfo, BlockReason, ClientMsg, FullState, OrchestratorStuck, RunInfo, RunRequest, RunState,
};

const NOW: u64 = 10_000;

fn env_block(run: &mut RunInfo, user_only: bool) {
    let mut task = blocked("t1", BlockReason::Environment, "locked");
    task.block.as_mut().unwrap().user_only = user_only;
    run.tasks = vec![task];
}

fn ask(question: &str, options: &[&str]) -> AskInfo {
    AskInfo {
        id: 1,
        question: question.into(),
        options: options.iter().map(|o| (*o).into()).collect(),
        context: "the style guide is silent".into(),
        asked_at: NOW - 30,
    }
}

fn asking(options: &[&str]) -> RunInfo {
    let mut run = with_orch(at("r", RunState::Running, 1), 11);
    run.orchestrator.as_mut().unwrap().ask = Some(ask("tabs or spaces?", options));
    run
}

#[test]
fn a_live_orchestrator_takes_the_environment_block_and_a_stuck_one_gives_it_back() {
    let mut run = with_orch(at("r", RunState::Running, 1), 11);
    env_block(&mut run, false);
    let app = app_with_runs(vec![], snapshot(NOW, vec![run.clone()]));
    assert_eq!(listed(&app), vec![]);

    run.orchestrator.as_mut().unwrap().stuck =
        Some(OrchestratorStuck::Stalled { since: NOW - 600 });
    let app = app_with_runs(vec![], snapshot(NOW, vec![run]));
    assert_eq!(
        listed(&app),
        vec![
            line(
                1,
                "r",
                "orchestrator has not acted for 10 min; its alerts are yours"
            ),
            line(3, "r › t1", "blocked (environment): locked"),
        ]
    );
}

#[test]
fn a_dead_orchestrator_names_the_restart_command() {
    let mut run = with_orch(at("r", RunState::Running, 1), 11);
    let orch = run.orchestrator.as_mut().unwrap();
    orch.live = false;
    orch.stuck = Some(OrchestratorStuck::Dead { since: None });
    let app = app_with_runs(
        vec![orch_window(11, "r", Status::Idle, true)],
        snapshot(NOW, vec![run]),
    );
    assert_eq!(
        listed(&app),
        vec![line(
            1,
            "r",
            "orchestrator exited; its alerts are yours · anthrex restart orch-r restarts it"
        )]
    );
}

#[test]
fn a_user_only_block_is_listed_with_you() {
    let mut run = with_orch(at("r", RunState::Running, 1), 11);
    env_block(&mut run, true);
    let app = app_with_runs(vec![], snapshot(NOW, vec![run.clone()]));
    let all = alerts(&app);
    assert_eq!(all.len(), 1);
    assert!(all[0].you);

    run.orchestrator = None;
    let app = app_with_runs(vec![], snapshot(NOW, vec![run]));
    let all = alerts(&app);
    assert_eq!(all.len(), 1);
    assert!(!all[0].you);
}

#[test]
fn task_needs_you_follows_the_route() {
    let mut live = with_orch(at("r", RunState::Running, 1), 11);
    env_block(&mut live, false);
    let mut only = live.clone();
    env_block(&mut only, true);
    assert!(!crate::app::alerts::task_needs_you(&live, &live.tasks[0]));
    assert!(crate::app::alerts::task_needs_you(&only, &only.tasks[0]));
}

#[test]
fn an_ask_is_one_priority_one_alert_with_numbered_options() {
    let app = app_with_runs(vec![], snapshot(NOW, vec![asking(&["tabs", "spaces"])]));
    let all = alerts(&app);
    assert_eq!(all.len(), 1);
    let alert = &all[0];
    assert_eq!(alert.priority, 1);
    assert_eq!(alert.key, AlertKey::OrchestratorAsks("r".into()));
    assert_eq!(alert.text, "orchestrator asks: tabs or spaces?");
    assert_eq!(
        alert.detail,
        "the style guide is silent\n\n1. tabs\n2. spaces"
    );
    assert!(matches!(alert.age, Some(30..=31)), "{:?}", alert.age);
}

fn open_view(app: &mut App) {
    prefix(app);
    press(app, KeyCode::Char('a'), KeyModifiers::NONE);
    assert_eq!(
        app.alerts_focus.as_ref().and_then(|f| f.selected.clone()),
        Some(AlertKey::OrchestratorAsks("r".into()))
    );
}

#[test]
fn option_keys_answer_and_a_missing_option_toasts() {
    let mut app = app_with_runs(vec![], snapshot(NOW, vec![asking(&["tabs", "spaces"])]));
    open_view(&mut app);
    assert_eq!(
        press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Run(RunRequest::AnswerAsk {
            run_id: "r".into(),
            ask: 1,
            choice: Some(1),
        }))]
    );
    assert!(app.alerts_focus.is_some(), "the view stays open");
    assert!(press(&mut app, KeyCode::Char('5'), KeyModifiers::NONE).is_empty());
    assert_eq!(app.toast_text(), Some("no option 5"));
}

#[test]
fn enter_on_an_ask_or_a_stuck_alert_focuses_the_orchestrator() {
    let mut app = app_with_runs(
        vec![orch_window(11, "r", Status::Working, true)],
        snapshot(NOW, vec![asking(&["tabs"])]),
    );
    open_view(&mut app);
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.focused, Some(11));
    assert!(app.alerts_focus.is_none());

    let mut run = with_orch(at("r", RunState::Running, 1), 11);
    run.orchestrator.as_mut().unwrap().stuck =
        Some(OrchestratorStuck::Stalled { since: NOW - 700 });
    let mut app = app_with_runs(
        vec![
            crate::tree::run_fixtures::pty(2, "shell", "/r/demo", Status::Idle),
            orch_window(11, "r", Status::Working, true),
        ],
        snapshot(NOW, vec![run]),
    );
    app.focus(2);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
    assert_eq!(
        app.alerts_focus.as_ref().and_then(|f| f.selected.clone()),
        Some(AlertKey::OrchestratorStuck("r".into()))
    );
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.focused, Some(11));
}

#[test]
fn typing_enter_into_the_orchestrator_window_answers_in_the_window() {
    let shell = crate::tree::run_fixtures::pty(2, "shell", "/r/demo", Status::Idle);
    let mut app = app_with_runs(
        vec![shell, orch_window(11, "r", Status::Working, true)],
        snapshot(NOW, vec![asking(&["tabs"])]),
    );
    let input = |id: u32, bytes: &[u8]| {
        Effect::Send(ClientMsg::Input {
            window_id: id,
            bytes: bytes.to_vec(),
        })
    };
    app.focus(11);
    assert_eq!(
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE),
        vec![
            input(11, b"\r"),
            Effect::Send(ClientMsg::Run(RunRequest::AnswerAsk {
                run_id: "r".into(),
                ask: 1,
                choice: None
            })),
        ]
    );
    assert_eq!(
        press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE),
        vec![input(11, b"x")]
    );
    app.focus(2);
    assert_eq!(
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE),
        vec![input(2, b"\r")]
    );
}

#[test]
fn an_accepted_red_stage_is_no_alert() {
    let mut run = at("r", RunState::Running, 1);
    let mut s = stage(1, Some("aaaa"), 1, 1);
    s.full.state = FullState::Red;
    s.full.commit = Some("aaaa".into());
    run.stages = vec![s.clone()];
    let app = app_with_runs(vec![], snapshot(NOW, vec![run.clone()]));
    assert_eq!(listed(&app), vec![line(3, "r", "stage 1 tier 3 red")]);
    run.stages[0].full.accepted = true;
    let app = app_with_runs(vec![], snapshot(NOW, vec![run]));
    assert_eq!(listed(&app), vec![]);
}

#[test]
fn an_ask_with_hostile_text_draws_on_one_line() {
    let bad = crate::safe_text::tests::hostile_text();
    let mut run = with_orch(at("r", RunState::Running, 1), 11);
    run.orchestrator.as_mut().unwrap().ask = Some(ask(&format!("q{bad}"), &[&format!("o{bad}")]));
    let app = app_with_runs(vec![], snapshot(NOW, vec![run]));
    let all = alerts(&app);
    assert_eq!(crate::safe_text::tests::first_hostile(&all[0].text), None);
    // The detail is raw; the view sanitises it where it draws.
    assert!(all[0].detail.contains("1. o"));
}
