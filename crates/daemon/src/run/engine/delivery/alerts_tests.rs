//! Ruling R-13: each `RunDelivery.alerts` key's kind, and that the attention lines and
//! the typed alerts are one list. The conditions themselves are driven in
//! `engine/tests/delivery_*.rs` (one assertion per kind beside its attention line).

use super::classify;
use proto::DeliveryAlertKind::*;

/// Every key the engine files in `RunDelivery.alerts`, and its kind:
/// - `auth` (`watch.rs`, decision 11): `gh` logged out;
/// - `<n>/ci/<check>` (`fix.rs::to_user`): CI handed to the user;
/// - `<n>/closed` (`land.rs::closed`): a PR closed without merging;
/// - `<n>/cap/<thread>` (`review.rs::over_cap`): review rounds over the cap;
/// - `<n|run>/<op>` (`failure_key`, `watch.rs::keeps_failing`), `<op>` one of
///   [`super::super::OP_NAMES`]: a held host op;
/// - attention lines only: `<n>/unlanded` (`land.rs`, the remedy is `run cancel`),
///   `<n>/sync` (`sync.rs`, a red base sync, the orchestrator's to fix),
///   `<n>/reply/<thread>` (`reply.rs`), `<n>/review/<thread>` (`review.rs`),
///   `<n>/page/<key>` (`view.rs`) and `<n>/missed` (`land.rs`, a fix that missed the
///   merge, the final fix wave's I-1);
/// - any other key, a future one included: none, never a guessed kind (ruling, task 15
///   fix round 2).
#[test]
fn every_key_has_its_kind_and_stage() {
    let mut cases = vec![
        ("auth".to_owned(), (Some(GhLoggedOut), None)),
        ("2/closed".to_owned(), (Some(PrClosedUnmerged), Some(2))),
        ("1/ci/test::a".to_owned(), (Some(CiHandedToUser), Some(1))),
        (
            "3/cap/7:c5".to_owned(),
            (Some(ReviewRoundsOverCap), Some(3)),
        ),
        ("run/permission".to_owned(), (Some(HostOpHeld), None)),
        // Attention lines only.
        ("2/unlanded".to_owned(), (None, None)),
        ("1/sync".to_owned(), (None, None)),
        ("1/reply/7:c5".to_owned(), (None, None)),
        ("1/review/7:c5".to_owned(), (None, None)),
        ("1/page/reviews".to_owned(), (None, None)),
        ("1/missed".to_owned(), (None, None)),
        // Unknown, a future key's shape included.
        ("1/rebase".to_owned(), (None, None)),
        ("run/sync".to_owned(), (None, None)),
        ("odd".to_owned(), (None, None)),
        ("x/push".to_owned(), (None, None)),
    ];
    for op in super::super::OP_NAMES {
        cases.push((format!("4/{op}"), (Some(HostOpHeld), Some(4))));
    }
    for (key, want) in cases {
        assert_eq!(classify(&key), want, "{key}");
    }
}

/// Each host op's place in `HostOp`, by a `match` the compiler checks: a new op is a
/// compile error here until `op_names_are_every_ops_name` lists it (the final fix
/// wave's deferred task-15 item).
fn place(op: &crate::run::delivery::ops::HostOp) -> usize {
    use crate::run::delivery::ops::HostOp::*;
    match op {
        Push { .. } => 0,
        Fetch { .. } => 1,
        OpenPr { .. } => 2,
        ViewPr { .. } => 3,
        FailedLogs { .. } => 4,
        RerunFailed { .. } => 5,
        Reply { .. } => 6,
        Retarget { .. } => 7,
        Permission { .. } => 8,
        DeleteBranch { .. } => 9,
    }
}

/// `OP_NAMES` is every `op_name`, one per host op: the list below holds one op of
/// each place [`place`] knows, and `OP_NAMES` has exactly that many names.
#[test]
fn op_names_are_every_ops_name() {
    use crate::host::ReplyTarget;
    use crate::run::delivery::ops::HostOp;
    let s = String::new;
    let ops = [
        HostOp::Push { stage: 1, sha: s() },
        HostOp::Fetch {
            stage: None,
            branch: s(),
            into: s(),
            adopt: None,
            parents_of: None,
        },
        HostOp::OpenPr {
            stage: 1,
            base: s(),
            head: s(),
            title: s(),
            body: s(),
        },
        HostOp::ViewPr {
            stage: 1,
            number: 1,
        },
        HostOp::FailedLogs {
            stage: 1,
            ci_run: 1,
            max_bytes: 1,
        },
        HostOp::RerunFailed {
            stage: 1,
            ci_run: 1,
        },
        HostOp::Reply {
            stage: 1,
            number: 1,
            thread: s(),
            target: ReplyTarget::Conversation,
            body: s(),
            marker: s(),
        },
        HostOp::Retarget {
            stage: 1,
            number: 1,
            base: s(),
        },
        HostOp::Permission { user: s() },
        HostOp::DeleteBranch { stage: 1 },
    ];
    let places: Vec<usize> = ops.iter().map(place).collect();
    assert_eq!(
        places,
        (0..super::super::OP_NAMES.len()).collect::<Vec<_>>()
    );
    let names: Vec<&str> = ops.iter().map(super::super::op_name).collect();
    assert_eq!(names, super::super::OP_NAMES);
}

/// The typed alert's text is one line with no hidden character, whatever the engine
/// filed (its text is built from the host's): this list's own guarantee, not only the
/// callers' cleaning.
#[test]
fn an_alerts_text_is_one_clean_line() {
    let mut run = crate::run::test_support::run_ok(crate::run::test_support::EXAMPLE_PLAN);
    run.delivery.mode = proto::DeliveryMode::Pr;
    run.delivery.alerts.insert(
        "1/closed".into(),
        "PR #7 was cl\u{200D}osed\u{202E}\nby the host\u{1b}[2J".into(),
    );
    let alerts = super::alerts(&run);
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].kind, PrClosedUnmerged);
    assert_eq!(alerts[0].text, "PR #7 was closed by the host [2J");
    // The attention line is the same line, as filed.
    assert_eq!(
        super::attention(&run),
        ["PR #7 was cl\u{200D}osed\u{202E}\nby the host\u{1b}[2J"]
    );
}
