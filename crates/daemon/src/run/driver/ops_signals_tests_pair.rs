//! Milestone 9.5 ruling RP-2 through the driver: a paired task's implementer has its
//! test-weakening signals read from the red commit (`SignalsSpec.from`), so an
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
        from: Some(red.to_string()),
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
