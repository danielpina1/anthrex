//! Milestone 9.5 ruling RP-2 through the driver: a paired task's implementer has its
//! test-weakening signals read from the red commit (`SignalsSpec.red`), so an
//! assertion it removes from the test writer's test is a signal. Measured from the
//! stage head, the test is new and nothing shows. Real git throughout.

use super::{Rig, git, spec};
use crate::run::engine::OpResult;
use crate::run::tiers::{Signal, SignalsSpec};

/// The claim's signals, from a check that counted `commits`.
fn signals(result: &OpResult, commits: u32) -> Vec<Signal> {
    match result {
        OpResult::DoneChecked {
            signals,
            commits: counted,
            ..
        } => {
            assert_eq!(*counted, commits, "{result:?}");
            signals
                .as_deref()
                .map(|s| s.list.clone())
                .unwrap_or_default()
        }
        other => panic!("{other:?}"),
    }
}

fn from(red: &str) -> SignalsSpec {
    SignalsSpec {
        red: Some(red.to_string()),
        ..spec()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_weakened_test_raises_a_signal_from_red_but_not_from_the_stage_head() {
    let rig = Rig::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    // The test writer's red commit: a new test with two assertions.
    rig.commit(&[(
        "tests/w.rs",
        Some("#[test]\nfn w() {\n    assert!(one());\n    assert!(two());\n}\n"),
    )]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    // The implementer drops one of them and implements the behaviour.
    rig.commit(&[
        (
            "tests/w.rs",
            Some("#[test]\nfn w() {\n    assert!(one());\n}\n"),
        ),
        (
            "src/lib.rs",
            Some("pub fn f() {}\npub fn one() -> bool { true }\n"),
        ),
    ]);
    let weakened = Signal::AssertionLoss {
        path: "tests/w.rs".into(),
        line: 4,
        removed: 1,
        added: 0,
    };
    let found = signals(&rig.verify(Some(from(&red))).await, 2);
    assert_eq!(found, vec![weakened], "from red");
    // The control: from the stage head the whole test is new, and no loss shows.
    let found = signals(&rig.verify(Some(spec())).await, 2);
    assert!(
        !found
            .iter()
            .any(|s| matches!(s, Signal::AssertionLoss { .. })),
        "{found:?}"
    );
}

/// Another task's commit on the run head: it deletes `tests/other.rs`. Its head.
fn other_task(rig: &Rig) -> String {
    git(&rig.root, &["rm", "-q", "tests/other.rs"]);
    let who = ["-c", "user.name=Other", "-c", "user.email=other@test"];
    git(
        &rig.root,
        &[&who[..], &["commit", "-q", "-m", "other task"]].concat(),
    );
    git(&rig.root, &["rev-parse", "HEAD"])
}

/// A `--no-ff` merge of `rev` into the worktree, as `name`.
fn merge_in(rig: &Rig, rev: &str, name: &str) {
    let user = format!("user.name={name}");
    let args = [
        "-c",
        &user,
        "-c",
        "user.email=m@test",
        "merge",
        "-q",
        "--no-ff",
        "-m",
        name,
        rev,
    ];
    git(&rig.worktree, &args);
}

/// The base, the writer's red commit, the implementer weakening the writer's test and
/// another test it owns: its red.
fn red_then_weakened(rig: &Rig) -> String {
    rig.commit(&[(
        "tests/w.rs",
        Some("#[test]\nfn w() {\n    assert!(one());\n    assert!(two());\n}\n"),
    )]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    rig.commit(&[
        (
            "tests/w.rs",
            Some("#[test]\nfn w() {\n    assert!(one());\n}\n"),
        ),
        (
            "tests/old.rs",
            Some("#[test]\nfn o() {\n    assert!(a());\n}\n"),
        ),
    ]);
    red
}

fn base_rig() -> Rig {
    Rig::new(&[
        ("src/lib.rs", "pub fn f() {}\n"),
        (
            "tests/other.rs",
            "#[test]\nfn o() {\n    assert!(true);\n}\n",
        ),
        (
            "tests/old.rs",
            "#[test]\nfn o() {\n    assert!(a());\n    assert!(b());\n}\n",
        ),
    ])
}

fn both_losses() -> Vec<Signal> {
    let loss = |path: &str| Signal::AssertionLoss {
        path: path.into(),
        line: 4,
        removed: 1,
        added: 0,
    };
    vec![loss("tests/w.rs"), loss("tests/old.rs")]
}

fn sorted(mut list: Vec<Signal>) -> Vec<Signal> {
    list.sort_by_key(|s| format!("{s:?}"));
    list
}

/// Rulings T16-1 and T16-7: a run-head merge after red is not the implementer's work.
/// Another task's deletion of a test file arrives with the engine's refresh merge and
/// raises nothing; the implementer's weakening before the merge is still caught, of the
/// writer's test (read from red) and of another test it owns (read from the merge base
/// with the run head).
#[tokio::test(flavor = "multi_thread")]
async fn a_weakening_before_a_refresh_merge_is_caught() {
    let rig = base_rig();
    let red = red_then_weakened(&rig);
    let run_head = other_task(&rig);
    merge_in(&rig, &run_head, "refresh");
    let result = rig.verify_at(&run_head, Some(from(&red))).await;
    let OpResult::DoneChecked { signals, .. } = result else {
        panic!("{result:?}")
    };
    let signals = signals.expect("signals");
    assert_eq!(
        sorted(signals.list),
        sorted(both_losses()),
        "nothing of other.rs"
    );
    // Ruling T16-7: the base is the merge base with the run head, never a merge.
    assert_eq!(signals.base, run_head);
}

/// Ruling T16-7: a merge the implementer makes itself moves no base, so its earlier
/// weakening still shows.
#[tokio::test(flavor = "multi_thread")]
async fn a_weakening_before_the_agents_own_merge_is_caught() {
    let rig = base_rig();
    let red = red_then_weakened(&rig);
    // An empty commit on a side line, merged `--no-ff` by the agent.
    let who = ["-c", "user.name=Agent", "-c", "user.email=a@test"];
    let side = ["commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "side"];
    let side = git(&rig.worktree, &[&who[..], &side[..]].concat());
    merge_in(&rig, &side, "agent");
    rig.commit(&[(
        "src/lib.rs",
        Some("pub fn f() {}\npub fn one() -> bool { true }\n"),
    )]);
    let start = git(&rig.root, &["rev-parse", "HEAD"]);
    let result = rig.verify_at(&start, Some(from(&red))).await;
    let OpResult::DoneChecked { signals, .. } = result else {
        panic!("{result:?}")
    };
    assert_eq!(
        sorted(signals.expect("signals").list),
        sorted(both_losses())
    );
}

/// Ruling T16-7 (N2): a writer's test the implementer deleted is restored from red,
/// where it exists, not from the merge base, where it never did.
#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_writers_test_is_restored_from_red() {
    let rig = base_rig();
    rig.commit(&[(
        "tests/w.rs",
        Some("#[test]\nfn w() {\n    assert!(one());\n}\n"),
    )]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    rig.commit(&[("tests/w.rs", None)]);
    let run_head = other_task(&rig);
    merge_in(&rig, &run_head, "refresh");
    let result = rig.verify_at(&run_head, Some(from(&red))).await;
    let OpResult::DoneChecked { signals, .. } = result else {
        panic!("{result:?}")
    };
    let signals = signals.expect("signals");
    let deleted = Signal::DeletedTestFile {
        path: "tests/w.rs".into(),
    };
    assert!(signals.list.contains(&deleted), "{:?}", signals.list);
    assert_eq!(signals.base, red);
}
