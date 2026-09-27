//! M8c.2: the client's copy of the run snapshot (decisions 1, 2 and 34).

use super::*;
use proto::{
    AgentRole, AgentRoundInfo, Effort, HistoryStats, ProfileReply, Route, RunInfo, RunReply,
    RunRequest, RunState, RunsSnapshot, Strength, TokenUsage,
};
use std::time::{Duration, Instant};

fn run_subscribe() -> Effect {
    Effect::Send(ClientMsg::Run(RunRequest::Subscribe))
}

fn run_subscribes(effects: &[Effect]) -> usize {
    effects.iter().filter(|e| **e == run_subscribe()).count()
}

fn run_info(id: &str) -> RunInfo {
    serde_json::from_value(serde_json::json!({
        "run_id": id, "goal": "g", "project": "/p", "root": "/p", "state": "running",
        "base_branch": "main", "base_sha": "", "run_branch": "", "run_head": "",
        "revision": 1, "max_writers": 1, "max_readers": 1, "max_bounces": 1,
        "writers_busy": 0, "readers_busy": 0, "unverified": false, "worker_sandbox": true,
        "trusted_project": [], "rate_limits": {}, "tasks": [], "critical_path": [],
        "attention": [], "report_path": "/p/report.md", "created_at": 0,
    }))
    .expect("a minimal RunInfo")
}

fn snapshot(revision: u64, now: u64, runs: Vec<RunInfo>) -> RunsSnapshot {
    RunsSnapshot {
        revision,
        runs,
        now,
    }
}

fn deliver(app: &mut App, snapshot: RunsSnapshot) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snapshot)))
}

/// `app_with_runs(windows, snapshot)`: the shared helper later tasks build on.
pub(super) fn app_with_runs(windows: Vec<WindowInfo>, snapshot: RunsSnapshot) -> App {
    let mut app = app_with(windows);
    let _ = app.run_subscription();
    deliver(&mut app, snapshot);
    app
}

fn round(until: Option<u64>, flag: bool) -> AgentRoundInfo {
    AgentRoundInfo {
        role: AgentRole::Worker,
        session: 1,
        round: 1,
        window_id: None,
        route: Route {
            runtime: Runtime::Claude,
            model: String::new(),
            strength: Strength::Standard,
            effort: Effort::Medium,
        },
        session_id: None,
        started_at: 0,
        ended_at: None,
        tool_calls: 0,
        last_event: 0,
        turn_open: false,
        turns: 0,
        rate_limited: flag,
        open_subagents: 0,
        denials: 0,
        usage: TokenUsage::default(),
        rate_limited_since: until.map(|_| 4900),
        rate_limited_until: until,
        sent_back_at: vec![],
    }
}

#[test]
fn the_first_effect_subscribes_to_runs() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    assert_eq!(app.run_subscription(), run_subscribe());
    assert!(app.run_subscribed);
}

#[test]
fn a_snapshot_replaces_the_last_one_even_with_a_lower_revision() {
    let mut app = app_with_runs(vec![], snapshot(57, 100, vec![run_info("old")]));
    assert_eq!(app.runs.revision, 57);
    // A restarted daemon starts its global revision again.
    assert!(deliver(&mut app, snapshot(3, 200, vec![run_info("new")])).is_empty());
    assert_eq!(app.runs.revision, 3);
    assert_eq!(app.runs.now, 200);
    let ids: Vec<_> = app.runs.runs.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(ids, vec!["new"]);
}

#[test]
fn reconnecting_resubscribes_to_runs() {
    let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
    let _ = app.run_subscription();
    app.on_link_lost("x");
    let effects = app.on_reconnected(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
    assert_eq!(run_subscribes(&effects), 1, "{effects:?}");
    let window_subscribes = effects
        .iter()
        .filter(|e| matches!(e, Effect::Send(ClientMsg::Subscribe { .. })))
        .count();
    assert_eq!(window_subscribes, 1, "{effects:?}");
    assert!(app.run_subscribed);
}

#[test]
fn a_refused_run_subscription_is_retried_on_tick() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    let _ = app.run_subscription();
    let msg = ClientMsg::Run(RunRequest::Subscribe);
    assert!(app.on_send_failed(&msg).is_empty());
    assert!(!app.run_subscribed);
    // A refused subscription is quiet, like a refused window `Subscribe`.
    assert_eq!(app.toast_text(), None);
    assert_eq!(app.on_tick(), vec![run_subscribe()]);
    assert!(app.run_subscribed);
    // Hostile: exactly one resend, however many ticks follow.
    assert!(app.on_tick().is_empty());
    assert!(app.on_tick().is_empty());
}

#[test]
fn a_run_subscription_is_not_retried_while_disconnected() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    let _ = app.run_subscription();
    app.on_link_lost("x");
    let _ = app.on_send_failed(&ClientMsg::Run(RunRequest::Subscribe));
    assert_eq!(run_subscribes(&app.on_tick()), 0);
    // The reconnect sends it, and the tick after has nothing left to retry.
    let effects = app.on_reconnected(vec![win(1, "a", Status::Idle)]);
    assert_eq!(run_subscribes(&effects), 1, "{effects:?}");
    assert_eq!(run_subscribes(&app.on_tick()), 0);
}

#[test]
fn a_snapshot_while_a_retry_is_pending_clears_the_retry() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    let _ = app.run_subscription();
    let _ = app.on_send_failed(&ClientMsg::Run(RunRequest::Subscribe));
    // An earlier send got through after all: the daemon is already pushing.
    deliver(&mut app, snapshot(1, 10, vec![]));
    assert!(app.run_subscribed);
    assert!(app.on_tick().is_empty());
}

#[test]
fn a_fresh_app_ticks_without_a_run_subscription() {
    // `lib.rs` sends the first subscription itself; a tick never invents one.
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    assert!(app.on_tick().is_empty());
}

#[test]
fn run_age_counts_from_the_daemons_clock() {
    let app = app_with_runs(vec![], snapshot(1, 5000, vec![]));
    assert_eq!(app.run_now(), 5000);
    assert_eq!(app.run_age(4000), 1000);
    // Hostile: a time after the daemon's clock saturates instead of underflowing.
    assert_eq!(app.run_age(6000), 0);
    assert_eq!(app.run_age(u64::MAX), 0);
}

#[test]
fn run_now_uses_the_time_the_snapshot_arrived() {
    let mut app = app_with_runs(vec![], snapshot(1, 5000, vec![]));
    app.set_runs_received_at(Instant::now().checked_sub(Duration::from_secs(40)).unwrap());
    assert_eq!(app.run_now(), 5040);
    assert_eq!(app.run_age(5000), 40);
    // A new snapshot restarts the client-side count.
    deliver(&mut app, snapshot(2, 6000, vec![]));
    assert_eq!(app.run_now(), 6000);
}

#[test]
fn rate_limited_only_until_its_end() {
    let mut app = app_with_runs(vec![], snapshot(1, 5000, vec![]));
    assert!(app.rate_limited(&round(Some(5030), false)));
    assert!(!app.rate_limited(&round(Some(5000), true)));
    assert!(!app.rate_limited(&round(Some(4990), true)));
    assert!(!app.rate_limited(&round(None, true)));
    // 40 seconds of client time later, the limit has run out.
    app.set_runs_received_at(Instant::now().checked_sub(Duration::from_secs(40)).unwrap());
    assert!(!app.rate_limited(&round(Some(5030), true)));
    assert!(app.rate_limited(&round(Some(5041), false)));
}

#[test]
fn done_replies_toast_and_refusals_toast() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    let reply = |app: &mut App, r: RunReply| app.on_daemon(DaemonMsg::Run(r));

    let done = RunReply::Done {
        request: "run approve".into(),
        message: "run r1 approved".into(),
    };
    assert!(reply(&mut app, done).is_empty());
    assert_eq!(app.toast_text(), Some("run r1 approved"));

    let refused = |message: &str| RunReply::Refused {
        request: "run reject".into(),
        message: message.into(),
    };
    assert!(reply(&mut app, refused("no such run")).is_empty());
    assert_eq!(app.toast_text(), Some("no such run"));

    let _ = reply(&mut app, refused("task t1: too small\ntask t2: unknown"));
    assert_eq!(app.toast_text(), Some("task t1: too small (+1 more)"));
    let _ = reply(&mut app, refused("a\nb\nc"));
    assert_eq!(app.toast_text(), Some("a (+2 more)"));
    let _ = reply(&mut app, refused("only line\n"));
    assert_eq!(app.toast_text(), Some("only line"));
}

#[test]
fn a_refusal_without_text_still_toasts() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    for message in ["", "\n", "\n\n\n", "  \n \r\n"] {
        app.toast = None;
        let _ = app.on_daemon(DaemonMsg::Run(RunReply::Refused {
            request: "run reject".into(),
            message: message.into(),
        }));
        assert_eq!(app.toast_text(), Some("run reject refused"), "{message:?}");
    }
    // Blank lines around the text neither lead nor count.
    let _ = app.on_daemon(DaemonMsg::Run(RunReply::Refused {
        request: "run edit".into(),
        message: "\n\nfirst\n\nsecond\n".into(),
    }));
    assert_eq!(app.toast_text(), Some("first (+1 more)"));
}

/// Review M3: trailing spaces on a refusal's first line are not shown before the count.
#[test]
fn a_refusals_first_line_loses_its_trailing_spaces() {
    assert_eq!(
        crate::app::runs::first_line_and_more("a  \t\nb"),
        Some("a (+1 more)".to_string())
    );
}

/// Review M1: a huge reply is capped before it reaches the status bar, which also
/// renders it without overflowing.
#[test]
fn a_huge_reply_toast_is_capped_and_renders() {
    use crate::app::runs::TOAST_MAX_CHARS;
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    let _ = app.on_daemon(DaemonMsg::Run(RunReply::Refused {
        request: "run edit".into(),
        message: format!("{}\nsecond", "日".repeat(70_000)),
    }));
    let expected = format!("{}… (+1 more)", "日".repeat(TOAST_MAX_CHARS));
    assert_eq!(app.toast_text(), Some(expected.as_str()));
    // Exactly at the cap: whole.
    let _ = app.on_daemon(DaemonMsg::Run(RunReply::Done {
        request: "run edit".into(),
        message: "x".repeat(TOAST_MAX_CHARS),
    }));
    assert_eq!(
        app.toast_text().map(|t| t.chars().count()),
        Some(TOAST_MAX_CHARS)
    );
    let _ = app.on_daemon(DaemonMsg::Run(RunReply::Done {
        request: "run edit".into(),
        message: "x".repeat(TOAST_MAX_CHARS + 1),
    }));
    let expected = format!("{}…", "x".repeat(TOAST_MAX_CHARS));
    assert_eq!(app.toast_text(), Some(expected.as_str()));
}

/// Review M4: a window `Subscribe` and the run subscription dropped together are both
/// sent again on the same tick.
#[test]
fn both_dropped_subscriptions_are_retried_on_one_tick() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    app.focus(1);
    let _ = app.run_subscription();
    let _ = app.on_send_failed(&ClientMsg::Subscribe {
        window_id: 1,
        cols: 80,
        rows: 24,
    });
    let _ = app.on_send_failed(&ClientMsg::Run(RunRequest::Subscribe));
    let effects = app.on_tick();
    assert_eq!(run_subscribes(&effects), 1, "{effects:?}");
    let window = effects
        .iter()
        .filter(|e| matches!(e, Effect::Send(ClientMsg::Subscribe { window_id: 1, .. })))
        .count();
    assert_eq!(window, 1, "{effects:?}");
}

#[test]
fn other_run_replies_change_nothing() {
    let mut app = app_with_runs(vec![win(1, "a", Status::Idle)], snapshot(9, 50, vec![]));
    app.toast = None;
    let triage = serde_json::from_value(serde_json::json!({
        "kinds": [], "scale": "single", "path": "fast", "reason": "r",
        "source": "fallback", "fallback_reason": null, "at": 0,
    }))
    .expect("a TriageInfo");
    let stats = HistoryStats {
        path: "/p".into(),
        task_records: 0,
        run_records: 0,
        rows: vec![],
        decider_calls: 0,
        decider_fallbacks: 0,
        size_checked: 0,
        size_raised: 0,
        problems: vec![],
    };
    let replies = vec![
        RunReply::Started {
            run_id: "r1".into(),
            state: RunState::AwaitingApproval,
        },
        RunReply::ConfirmNeeded {
            run_id: "r1".into(),
            prompt: "p".into(),
            base_moved: None,
        },
        RunReply::ToolResult {
            ok: false,
            text: "t".into(),
        },
        RunReply::Triaged {
            triage,
            run_id: None,
            message: "m".into(),
        },
        RunReply::Profile(Box::new(ProfileReply::Done {
            message: "m".into(),
        })),
        RunReply::Stats(stats),
    ];
    for reply in replies {
        let label = format!("{reply:?}");
        assert!(app.on_daemon(DaemonMsg::Run(reply)).is_empty(), "{label}");
        assert_eq!(app.toast_text(), None, "{label}");
        assert!(app.modal.is_none(), "{label}");
        assert_eq!(app.runs.revision, 9, "{label}");
        assert_eq!(app.run_now(), 50, "{label}");
        assert!(app.run_subscribed, "{label}");
    }
}

/// M8c.3, decision 10. The brief's single-project form cannot move at all: folding
/// `/r/demo` hides the orchestrator's row as well as window 1, and `focus_relative`
/// returns early on an empty visible order. So window 1 lives in a second, folded
/// project, `/r/alpha` (sorted first by its `Attention`), and the expanded order
/// `[1, 3, 2]` must come from `build_with_runs`: plain `build` lists window 3 after
/// window 2 (`[1, 2, 3, 6]`), and the folded `rows()` does not contain window 1 at all.
#[test]
fn focus_relative_reaches_the_orchestrator_even_when_folded() {
    use crate::tree::run_fixtures::{PROJECT, pty, three_task_fixture};
    let (snapshot, mut windows) = three_task_fixture();
    windows[0].project = "/r/alpha".into();
    windows[0].cwd = "/r/alpha".into();
    windows[0].status = Status::Attention;
    windows.push(pty(2, "api", PROJECT, Status::Idle));
    let mut app = app_with_runs(windows, snapshot);
    assert_eq!(tree::agent_order(&app.rows()), vec![1, 3, 2]);
    assert!(app.tree.toggle(&tree::NodeKey::Project("/r/alpha".into())));
    assert_eq!(tree::agent_order(&app.rows()), vec![3, 2]);

    let _ = app.focus(1);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(3), "the orchestrator follows window 1");

    let _ = app.focus(1);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(2), "window 2 precedes window 1, wrapping");
}

/// M8c.3: a snapshot prunes run keys and keeps a run-only project's fold.
#[test]
fn a_snapshot_prunes_the_keys_of_runs_that_left() {
    use crate::tree::run_fixtures::{gate_fixture, snapshot};
    let (snap, _) = gate_fixture();
    let mut app = app_with_runs(vec![], snap.clone());
    let run_key = tree::NodeKey::Run("add-reset-3f9a".into());
    let project = tree::NodeKey::Project("/r/demo".into());
    assert!(app.tree.toggle(&run_key));
    assert!(app.tree.toggle(&project));
    deliver(&mut app, snap);
    let _ = app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![] });
    assert!(app.tree.is_collapsed(&run_key));
    assert!(
        app.tree.is_collapsed(&project),
        "a shown run names the project"
    );

    deliver(&mut app, snapshot(20_000, vec![]));
    assert!(!app.tree.is_collapsed(&run_key));
    assert!(!app.tree.is_collapsed(&project));
}

/// Review I1: the selection repair in `replace_runs` (`app/runs.rs`). The selected
/// `Run` row leaves with its discarded run, and the selection moves to the row now
/// at its old index, the plain window after it.
#[test]
fn a_discarded_runs_selected_row_hands_the_selection_to_its_neighbour() {
    use crate::tree::run_fixtures::{RUN_ID, gate_fixture};
    let (mut snap, windows) = gate_fixture();
    let mut app = app_with_runs(windows, snap.clone());
    app.enter_tree();
    let run_key = tree::NodeKey::Run(RUN_ID.into());
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, run_key.clone());
    assert_eq!(app.tree.selected, Some(run_key));

    snap.runs[0].state = RunState::Discarded;
    deliver(&mut app, snap);
    assert_eq!(app.tree.selected, Some(tree::NodeKey::Window(1)));
    assert_eq!(
        tree::row_index(&app.rows(), &tree::NodeKey::Window(1)),
        Some(1)
    );
    assert_eq!(app.tree.selected_index(&app.rows()), Some(1));
}

/// Review I1: a tree that the snapshot empties clears the selection, with no panic.
#[test]
fn a_snapshot_that_empties_the_tree_clears_the_selection() {
    use crate::tree::run_fixtures::{RUN_ID, gate_fixture};
    let (mut snap, _) = gate_fixture();
    let mut app = app_with_runs(vec![], snap.clone());
    app.enter_tree();
    let run_key = tree::NodeKey::Run(RUN_ID.into());
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, run_key.clone());
    assert_eq!(app.tree.selected, Some(run_key));

    snap.runs[0].state = RunState::Discarded;
    deliver(&mut app, snap);
    assert!(app.rows().is_empty());
    assert_eq!(app.tree.selected, None);
    // An empty snapshot after that is still quiet.
    deliver(&mut app, snapshot(2, 10, vec![]));
    assert_eq!(app.tree.selected, None);
}

/// Review I1: the reveal in `replace_runs`. Runs arriving above the plain windows push
/// the selected last window below the sidebar; the snapshot scrolls it back in view.
#[test]
fn runs_arriving_above_the_selection_keep_it_in_the_sidebar() {
    use crate::tree::run_fixtures::{PROJECT, pty, run};
    let windows: Vec<_> = (1..=8)
        .map(|id| pty(id, &format!("w{id}"), PROJECT, Status::Idle))
        .collect();
    let mut app = app_with_runs(windows, snapshot(1, 10, vec![]));
    app.enter_tree();
    app.set_tree_viewports(4, 10);
    let last = tree::NodeKey::Window(8);
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, last.clone());
    app.reveal_tree_anchor();
    assert_eq!(app.tree.sidebar.top, 5, "row 8 is the last of four");

    let runs = ["a-0001", "b-0002", "c-0003"]
        .map(|id| run(id, PROJECT, RunState::Running))
        .to_vec();
    deliver(&mut app, snapshot(2, 10, runs));
    let index = app
        .tree
        .selected_index(&app.rows())
        .expect("still selected");
    assert_eq!(index, 11);
    assert_eq!(app.tree.selected, Some(last));
    assert_eq!(app.tree.sidebar.top, 8, "the selected row is revealed");
}
