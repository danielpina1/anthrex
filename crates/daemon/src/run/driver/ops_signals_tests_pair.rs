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

/// Ruling T16-1: a run-head merge after red is not the implementer's work. Another
/// task's deletion of a test file arrives with the merge and raises nothing; the
/// implementer's weakening of the writer's test before the merge is still read from
/// red, limited to the writer's paths.
#[tokio::test(flavor = "multi_thread")]
async fn a_merge_after_red_is_not_the_implementers_work() {
    let rig = Rig::new(&[
        ("src/lib.rs", "pub fn f() {}\n"),
        (
            "tests/other.rs",
            "#[test]\nfn o() {\n    assert!(true);\n}\n",
        ),
    ]);
    rig.commit(&[(
        "tests/w.rs",
        Some("#[test]\nfn w() {\n    assert!(one());\n    assert!(two());\n}\n"),
    )]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    // The implementer drops an assertion from the writer's test.
    rig.commit(&[(
        "tests/w.rs",
        Some("#[test]\nfn w() {\n    assert!(one());\n}\n"),
    )]);
    // Another task merged on the run head deletes its own test file; the engine
    // merges the run head into the implementer's checkout.
    git(&rig.root, &["rm", "-q", "tests/other.rs"]);
    git(
        &rig.root,
        &[
            "-c",
            "user.name=Other",
            "-c",
            "user.email=other@test",
            "commit",
            "-q",
            "-m",
            "other task",
        ],
    );
    let run_head = git(&rig.root, &["rev-parse", "HEAD"]);
    git(
        &rig.worktree,
        &[
            "-c",
            "user.name=Engine",
            "-c",
            "user.email=engine@test",
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "refresh",
            &run_head,
        ],
    );
    let found = match rig.verify(Some(from(&red))).await {
        OpResult::DoneChecked { signals, .. } => signals.map(|s| s.list).unwrap_or_default(),
        other => panic!("{other:?}"),
    };
    assert!(
        !found
            .iter()
            .any(|s| matches!(s, Signal::DeletedTestFile { path } if path == "tests/other.rs")),
        "the merged deletion is not the implementer's: {found:?}"
    );
    assert_eq!(
        found,
        vec![Signal::AssertionLoss {
            path: "tests/w.rs".into(),
            line: 4,
            removed: 1,
            added: 0,
        }],
        "the writer's test, read from red"
    );
}
