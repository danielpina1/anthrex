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

/// A commit on the run head by another task, with `files` written (`None` deletes):
/// its head.
fn on_run_head(rig: &Rig, files: &[(&str, Option<&str>)]) -> String {
    for (path, text) in files {
        match text {
            Some(text) => {
                let full = rig.root.join(path);
                std::fs::create_dir_all(full.parent().unwrap()).unwrap();
                std::fs::write(&full, text).unwrap();
                git(&rig.root, &["add", "--", path]);
            }
            None => {
                git(&rig.root, &["rm", "-q", "--", path]);
            }
        }
    }
    let who = ["-c", "user.name=Other", "-c", "user.email=other@test"];
    git(
        &rig.root,
        &[&who[..], &["commit", "-q", "-m", "other task"]].concat(),
    );
    git(&rig.root, &["rev-parse", "HEAD"])
}

const W2: &str = "#[test]\nfn w() {\n    assert!(one());\n    assert!(two());\n}\n";
const W1: &str = "#[test]\nfn w() {\n    assert!(one());\n}\n";

/// Ruling T16-8 (a): each read is limited by a pathspec, so the run head's signals
/// that a refresh merge brings into `red..head` cannot crowd the implementer's
/// weakening of the writer's test out of the cap, and `more` counts nothing of theirs.
#[tokio::test(flavor = "multi_thread")]
async fn run_head_signals_cannot_crowd_out_the_writers_test() {
    let names: Vec<String> = (0..21).map(|n| format!("tests/o{n:02}.rs")).collect();
    let text = "#[test]\nfn o() {\n    assert!(true);\n}\n";
    let mut base: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), text)).collect();
    base.push(("src/lib.rs", "pub fn f() {}\n"));
    let rig = Rig::new(&base);
    rig.commit(&[("tests/w.rs", Some(W2))]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    rig.commit(&[("tests/w.rs", Some(W1))]);
    let gone: Vec<(&str, Option<&str>)> = names.iter().map(|n| (n.as_str(), None)).collect();
    let run_head = on_run_head(&rig, &gone);
    merge_in(&rig, &run_head, "refresh");
    let result = rig.verify_at(&run_head, Some(from(&red))).await;
    let OpResult::DoneChecked { signals, .. } = result else {
        panic!("{result:?}")
    };
    let signals = signals.expect("signals");
    let loss = Signal::AssertionLoss {
        path: "tests/w.rs".into(),
        line: 4,
        removed: 1,
        added: 0,
    };
    assert_eq!((signals.list, signals.more), (vec![loss], 0));
}

/// Ruling T16-8 (b): each deleted test file has its own restore base, red for the
/// writer's test and the merge base for a test the run head brought.
#[tokio::test(flavor = "multi_thread")]
async fn each_deleted_test_file_has_its_own_restore_base() {
    let rig = base_rig();
    rig.commit(&[("tests/w.rs", Some(W1))]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    let run_head = on_run_head(&rig, &[("tests/x.rs", Some(W1))]);
    merge_in(&rig, &run_head, "refresh");
    rig.commit(&[("tests/w.rs", None), ("tests/x.rs", None)]);
    let result = rig.verify_at(&run_head, Some(from(&red))).await;
    let OpResult::DoneChecked { signals, .. } = result else {
        panic!("{result:?}")
    };
    let signals = signals.expect("signals");
    let want: std::collections::BTreeMap<String, String> = [
        ("tests/w.rs".to_string(), red.clone()),
        ("tests/x.rs".to_string(), run_head.clone()),
    ]
    .into();
    assert_eq!(signals.restore_from, want, "{:?}", signals.list);
    assert_eq!(signals.base, run_head, "the merge base");
}

/// Ruling T16-8 (c): a change the run head made to a writer's path, which the head
/// holds exactly as the run head does, is not the implementer's.
#[tokio::test(flavor = "multi_thread")]
async fn a_run_head_change_on_a_writers_path_is_not_the_implementers() {
    let rig = base_rig();
    // The writer adds its test and a line to the shared `tests/old.rs`.
    let shared = "#[test]\nfn o() {\n    assert!(a());\n    assert!(b());\n}\n// shared\n";
    rig.commit(&[("tests/w.rs", Some(W2)), ("tests/old.rs", Some(shared))]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    rig.commit(&[("tests/w.rs", Some(W1))]);
    // Another task makes the same addition and drops an assertion of its own.
    let theirs = "#[test]\nfn o() {\n    assert!(a());\n}\n// shared\n";
    let run_head = on_run_head(&rig, &[("tests/old.rs", Some(theirs))]);
    merge_in(&rig, &run_head, "refresh");
    let result = rig.verify_at(&run_head, Some(from(&red))).await;
    let OpResult::DoneChecked { signals, .. } = result else {
        panic!("{result:?}")
    };
    let loss = Signal::AssertionLoss {
        path: "tests/w.rs".into(),
        line: 4,
        removed: 1,
        added: 0,
    };
    assert_eq!(signals.expect("signals").list, vec![loss]);
}

/// Ruling T16-8 (d): the test writer's own claim reads as any worker's (the merge-base
/// read over `start..red`), so its weakening of an existing test is caught.
#[tokio::test(flavor = "multi_thread")]
async fn a_writers_claim_catches_its_weakening_of_an_existing_test() {
    let rig = base_rig();
    rig.commit(&[
        ("tests/w.rs", Some(W2)),
        (
            "tests/old.rs",
            Some("#[test]\nfn o() {\n    assert!(a());\n}\n"),
        ),
    ]);
    let found = signals(&rig.verify(Some(spec())).await, 1);
    let loss = Signal::AssertionLoss {
        path: "tests/old.rs".into(),
        line: 4,
        removed: 1,
        added: 0,
    };
    assert_eq!(found, vec![loss]);
}

/// Ruling T16-8 (c)'s bound: a writer's path the head holds as the run head does is
/// dropped only when the run head changed it. An implementer that reverts the writer's
/// assertion in a shared test, with no refresh, is still caught.
#[tokio::test(flavor = "multi_thread")]
async fn reverting_the_writers_change_to_a_shared_test_is_caught() {
    let rig = base_rig();
    let added = "#[test]\nfn o() {\n    assert!(a());\n    assert!(b());\n    assert!(c());\n}\n";
    rig.commit(&[("tests/w.rs", Some(W2)), ("tests/old.rs", Some(added))]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    let start = "#[test]\nfn o() {\n    assert!(a());\n    assert!(b());\n}\n";
    rig.commit(&[("tests/old.rs", Some(start))]);
    let found = signals(&rig.verify(Some(from(&red))).await, 2);
    let loss = Signal::AssertionLoss {
        path: "tests/old.rs".into(),
        line: 5,
        removed: 1,
        added: 0,
    };
    assert_eq!(found, vec![loss]);
}

/// Whether `git <args>` in `dir` succeeded (a merge that may conflict).
fn git_ok(dir: &std::path::Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
        .output()
        .unwrap()
        .status
        .success()
}

/// Ruling T16-10 (re-review 3's NI-1): the head equals the run head on a shared writer
/// path, but only because the implementer resolved a refresh conflict with `--theirs`,
/// dropping the writer's `fn extra`. The clean 3-way merge of (start, red, run head)
/// conflicts there, so the path is read over `red..head` and the loss is a W-signal.
#[tokio::test(flavor = "multi_thread")]
async fn a_theirs_resolution_dropping_the_writers_test_is_caught() {
    let api = "#[test]\nfn a() {\n    assert!(one());\n}\n";
    let rig = Rig::new(&[("src/lib.rs", "pub fn f() {}\n"), ("tests/api.rs", api)]);
    let extra = format!("{api}#[test]\nfn extra() {{\n    assert!(two());\n}}\n");
    rig.commit(&[("tests/api.rs", Some(&extra)), ("tests/w.rs", Some(W1))]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    let other = format!("{api}#[test]\nfn other() {{}}\n");
    let run_head = on_run_head(&rig, &[("tests/api.rs", Some(&other))]);
    let who = ["-c", "user.name=Agent", "-c", "user.email=a@test"];
    let merge = [
        &who[..],
        &["merge", "-q", "--no-ff", "-m", "refresh", &run_head],
    ]
    .concat();
    assert!(!git_ok(&rig.worktree, &merge), "the refresh conflicts");
    git(
        &rig.worktree,
        &["checkout", "--theirs", "--", "tests/api.rs"],
    );
    git(&rig.worktree, &["add", "--", "tests/api.rs"]);
    git(
        &rig.worktree,
        &[&who[..], &["commit", "-q", "--no-edit"]].concat(),
    );
    let result = rig.verify_at(&run_head, Some(from(&red))).await;
    let OpResult::DoneChecked { signals, .. } = result else {
        panic!("{result:?}")
    };
    let list = signals.expect("signals").list;
    assert!(
        list.iter().any(|s| matches!(
            s,
            Signal::AssertionLoss { path, removed: 1, .. } if path == "tests/api.rs"
        )),
        "{list:?}"
    );
}

/// Ruling T16-9 (2): over 256 writer paths, the writer-path read runs without a
/// pathspec and is filtered; the claim says how many paths there were.
#[tokio::test(flavor = "multi_thread")]
async fn over_256_writer_paths_are_read_without_a_pathspec() {
    let rig = base_rig();
    let names: Vec<String> = (0..257).map(|n| format!("data/f{n:03}.txt")).collect();
    let mut files: Vec<(&str, Option<&str>)> =
        names.iter().map(|n| (n.as_str(), Some("x\n"))).collect();
    files.push(("tests/w.rs", Some(W2)));
    rig.commit(&files);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    rig.commit(&[("tests/w.rs", Some(W1))]);
    let run_head = other_task(&rig);
    merge_in(&rig, &run_head, "refresh");
    let result = rig.verify_at(&run_head, Some(from(&red))).await;
    let OpResult::DoneChecked { signals, .. } = result else {
        panic!("{result:?}")
    };
    let signals = signals.expect("signals");
    let loss = Signal::AssertionLoss {
        path: "tests/w.rs".into(),
        line: 4,
        removed: 1,
        added: 0,
    };
    assert_eq!(
        (signals.list, signals.more),
        (vec![loss], 0),
        "nothing of other.rs"
    );
    assert_eq!(signals.unlimited, 258);
}

/// Re-review 3's out-of-scope item: the writer's paths' signals come first within
/// their rank, so twenty of the implementer's own signals never push a writer-test
/// loss into `more`.
#[tokio::test(flavor = "multi_thread")]
async fn the_writers_test_loss_is_listed_ahead_of_the_implementers_own() {
    let names: Vec<String> = (0..21).map(|n| format!("tests/o{n:02}.rs")).collect();
    let mut base: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), W2)).collect();
    base.push(("src/lib.rs", "pub fn f() {}\n"));
    let rig = Rig::new(&base);
    rig.commit(&[("tests/w.rs", Some(W2))]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    let mut weakened: Vec<(&str, Option<&str>)> =
        names.iter().map(|n| (n.as_str(), Some(W1))).collect();
    weakened.push(("tests/w.rs", Some(W1)));
    rig.commit(&weakened);
    let result = rig.verify(Some(from(&red))).await;
    let OpResult::DoneChecked { signals, .. } = result else {
        panic!("{result:?}")
    };
    let signals = signals.expect("signals");
    let loss = Signal::AssertionLoss {
        path: "tests/w.rs".into(),
        line: 4,
        removed: 1,
        added: 0,
    };
    assert_eq!(signals.list.first(), Some(&loss), "{:?}", signals.list);
    assert_eq!((signals.list.len(), signals.more), (20, 2));
}

/// Ruling T16-10: a git without `merge-tree --merge-base` (before 2.40; the run
/// engine's minimum is 2.38) gives no clean merge, so the writer's path is read over
/// `red..head` (the safe direction): (c)'s run-head change shows again.
#[tokio::test(flavor = "multi_thread")]
async fn without_merge_base_the_writers_path_is_kept() {
    let old =
        "case \" $* \" in *\" --merge-base=\"*) echo 'error: unknown option' >&2; exit 129;; esac";
    let rig = Rig::with_wrapper(
        &[
            ("src/lib.rs", "pub fn f() {}\n"),
            (
                "tests/old.rs",
                "#[test]\nfn o() {\n    assert!(a());\n    assert!(b());\n}\n",
            ),
        ],
        Some(old),
    );
    let shared = "#[test]\nfn o() {\n    assert!(a());\n    assert!(b());\n}\n// shared\n";
    rig.commit(&[("tests/w.rs", Some(W2)), ("tests/old.rs", Some(shared))]);
    let red = git(&rig.worktree, &["rev-parse", "HEAD"]);
    let theirs = "#[test]\nfn o() {\n    assert!(a());\n}\n// shared\n";
    let run_head = on_run_head(&rig, &[("tests/old.rs", Some(theirs))]);
    merge_in(&rig, &run_head, "refresh");
    let result = rig.verify_at(&run_head, Some(from(&red))).await;
    let OpResult::DoneChecked { signals, .. } = result else {
        panic!("{result:?}")
    };
    let list = signals.expect("signals").list;
    assert!(
        list.iter()
            .any(|s| matches!(s, Signal::AssertionLoss { path, .. } if path == "tests/old.rs")),
        "{list:?}"
    );
    assert!(
        rig.logged().iter().any(|l| l.contains("--merge-base=")),
        "tried"
    );
}
