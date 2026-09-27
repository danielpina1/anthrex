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
