use super::finish::base_matches;
use super::{RUN_REQUEST_TIMEOUT, request_timeout, resolve_run, status};
use daemon::run::git::ACCEPT_MERGE_TIMEOUT;
use proto::{FinishAction, RunInfo, RunRequest};
use std::time::Duration;

/// Ruling T23-I1: accept and discard outwait the daemon's own merge bound, whatever
/// it becomes; every other request keeps `RUN_REQUEST_TIMEOUT`.
#[test]
fn finish_requests_outwait_the_accept_merge() {
    for action in [FinishAction::Accept, FinishAction::Discard] {
        let finish = RunRequest::Finish {
            run_id: "r".into(),
            action,
            confirm: None,
        };
        assert!(
            request_timeout(&finish) >= ACCEPT_MERGE_TIMEOUT + Duration::from_secs(60),
            "{action:?}: {:?}",
            request_timeout(&finish)
        );
    }
    assert_eq!(request_timeout(&RunRequest::List), RUN_REQUEST_TIMEOUT);
    let approve = RunRequest::Approve { run_id: "r".into() };
    assert_eq!(request_timeout(&approve), RUN_REQUEST_TIMEOUT);
}

/// Ruling T23-I2: `--base` takes the listed head or any hex prefix of it of at least
/// seven characters.
#[test]
fn base_accepts_the_listed_head_or_a_prefix_of_it() {
    let to = "0a04f693cbc5ae9b25fd8a2f3c5ad94ae2252ecb";
    for given in [to, "0a04f69", "0a04f693cbc5", "0A04F69"] {
        assert!(base_matches(given, to), "{given}");
    }
    for given in [
        "0a04f6",
        "",
        "0a04f6x",
        "1a04f69",
        "0a04f693cbc5ae9b25fd8a2f3c5ad94ae2252ecb0",
    ] {
        assert!(!base_matches(given, to), "{given}");
    }
}

fn runs(ids: &[&str]) -> Vec<RunInfo> {
    ids.iter()
        .map(|id| RunInfo {
            run_id: id.to_string(),
            ..status::tests::example()
        })
        .collect()
}

#[test]
fn resolve_run_by_id_suffix_and_prefix() {
    let all = runs(&["add-reset-3f9a", "add-login-77b0", "fix-reset-3f9b"]);
    assert_eq!(
        resolve_run(&all, "add-reset-3f9a").unwrap(),
        "add-reset-3f9a"
    );
    assert_eq!(resolve_run(&all, "3f9a").unwrap(), "add-reset-3f9a");
    assert_eq!(resolve_run(&all, "add-l").unwrap(), "add-login-77b0");
    assert_eq!(resolve_run(&all, "fix").unwrap(), "fix-reset-3f9b");
    assert_eq!(
        resolve_run(&all, "add").unwrap_err(),
        "'add' matches more than one run: add-reset-3f9a, add-login-77b0"
    );
    assert_eq!(
        resolve_run(&all, "reset").unwrap_err(),
        "no run matches 'reset'"
    );
    assert_eq!(
        resolve_run(&all, "nope").unwrap_err(),
        "no run matches 'nope'"
    );
    // An exact id wins over a longer id it prefixes.
    let nested = runs(&["a-1", "a-1b"]);
    assert_eq!(resolve_run(&nested, "a-1").unwrap(), "a-1");
}

/// M9.14 review fixes, item 3: what the CLI prints of a daemon's text keeps its lines
/// and shows every other control character as a space.
#[test]
fn printed_daemon_text_has_no_control_characters() {
    assert_eq!(
        status::printable("no such task nope\x1b[2J\x07\r\nnot delivered: t\u{9b}2"),
        "no such task nope [2J \nnot delivered: t 2"
    );
    assert_eq!(status::printable("plain\ntext"), "plain\ntext");
    // Task M9.2.14 fix round 1 (m1): the bidi override, the zero-width joiner and the
    // byte-order mark are dropped, the line separator is a space.
    assert_eq!(
        status::printable("pr\u{202E}lmth.exe\u{200D}\u{FEFF} a\u{2028}b"),
        "prlmth.exe a b"
    );
}

/// Task M9.2.12 fix round 1, I2: `run start` (plan or goal) outwaits M8a's git preflight
/// plus pr mode's host preflight (the daemon's `PREFLIGHT_BOUND`), whatever it becomes,
/// so a stalled `gh` or push is the daemon's refusal, never the CLI's timeout.
#[test]
fn run_start_outwaits_both_preflights() {
    let preflights = RUN_REQUEST_TIMEOUT + daemon::host::PREFLIGHT_BOUND;
    let start = RunRequest::Start {
        plan_toml: String::new(),
        dir: "/r".into(),
        yes: false,
        trust_project: false,
        unconfined_checks: false,
        delivery: None,
        design: None,
    };
    assert!(
        request_timeout(&start) > preflights,
        "{:?}",
        request_timeout(&start)
    );
    assert_eq!(request_timeout(&start), super::RUN_START_TIMEOUT);
    // Milestone 9.5 ruling T9-3: the build's 30 s and the start's tuning bound.
    let tuning = daemon::run::driver::tuning::TUNING_START_BOUND;
    assert_eq!(
        super::RUN_START_TIMEOUT,
        preflights + Duration::from_secs(30) + tuning
    );
    let goal = RunRequest::StartGoal {
        goal: "g".into(),
        dir: "/r".into(),
        yes: false,
        trust_project: false,
        unconfined_checks: false,
        orchestrator: None,
        delivery: None,
        continue_from: None,
        design: None,
    };
    // The goal's own terms (the triage decider's 600 s and 30 s) on top.
    assert!(request_timeout(&goal) > preflights + Duration::from_secs(600 + 30));
}

/// Deferred from task 14: `run stats --json` and accept's research-report line go through
/// `printable` like every other text the CLI prints of the daemon's.
#[test]
fn stats_json_and_the_report_line_are_printable() {
    let hostile = "re\u{202E}po\u{200D}\x1b[2J";
    let stats = proto::HistoryStats {
        path: hostile.into(),
        task_records: 0,
        run_records: 0,
        rows: Vec::new(),
        decider_calls: 0,
        decider_fallbacks: 0,
        size_checked: 0,
        size_raised: 0,
        problems: vec![format!("bad line: {hostile}")],
        flaky_proposals: Vec::new(),
        window_days: 0,
        quarantine_after: 0,
        rounds: 0,
        iterated_runs: 0,
        tuning: None,
    };
    let json = super::adapt::stats_json(&stats).unwrap();
    // JSON escapes the control character itself; the bidi override and the ZWJ, which
    // JSON leaves raw, are dropped.
    assert!(json.contains("\"path\": \"repo\\u001b[2J\""), "{json}");
    assert!(!json.contains(['\u{202E}', '\u{200D}', '\x1b']), "{json}");
    assert!(json.contains('\n'), "the JSON keeps its lines");
    assert_eq!(
        super::finish::report_line(std::path::Path::new(&format!("/tmp/{hostile}.md"))),
        "research report: /tmp/repo [2J.md"
    );
}
