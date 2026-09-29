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
        "no such task nope [2J  \nnot delivered: t 2"
    );
    assert_eq!(status::printable("plain\ntext"), "plain\ntext");
}
