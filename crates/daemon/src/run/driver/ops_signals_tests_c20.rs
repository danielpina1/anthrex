//! Milestone 9.1 ruling C-20: what a worker's checkout must not be able to hide from
//! decision 40's signals (attributes, colour), test code removed with its module or
//! file, a test file replaced by a symbolic link past a cut diff, a `-U0` read that
//! times out, and the base the restore command names. Real git throughout.

use std::time::Duration;

use super::{Rig, git, signals_of, spec, write};
use crate::run::git::{DiffLimits, done_signals_with};
use crate::run::tiers::{ClaimSignals, Signal};

const T: Duration = Duration::from_secs(30);

fn skip(path: &str, line: u32) -> Signal {
    Signal::SkipMarker {
        path: path.into(),
        line,
        marker: "#[ignore]".into(),
    }
}

/// `tests/t.rs` with two assertions, and `src/lib.rs` with a test module holding two.
fn base() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "tests/t.rs",
            "#[test]\nfn a() {\n    assert!(one());\n    assert!(two());\n}\n",
        ),
        (
            "src/lib.rs",
            "pub fn f() {}\n#[cfg(test)]\nmod t {\n    fn a() {\n        assert!(one());\n    }\n}\n",
        ),
    ]
}

/// The worker adds a skip marker to `tests/t.rs` and drops an assertion from each file.
fn weaken(rig: &Rig, also: &[(&str, Option<&str>)]) {
    let mut files: Vec<(&str, Option<&str>)> = vec![
        (
            "tests/t.rs",
            Some("#[test]\n#[ignore]\nfn a() {\n    assert!(one());\n}\n"),
        ),
        (
            "src/lib.rs",
            Some("pub fn f() {}\n#[cfg(test)]\nmod t {\n    fn a() {\n    }\n}\n"),
        ),
    ];
    files.extend_from_slice(also);
    rig.commit(&files);
}

fn weakened() -> Vec<Signal> {
    vec![
        Signal::AssertionLoss {
            path: "src/lib.rs".into(),
            line: 5,
            removed: 1,
            added: 0,
        },
        skip("tests/t.rs", 2),
        Signal::AssertionLoss {
            path: "tests/t.rs".into(),
            line: 4,
            removed: 1,
            added: 0,
        },
    ]
}

/// Ruling C-20 (1): neither a committed `.gitattributes` nor an untracked one can mark
/// the files binary and so hide their lines.
#[tokio::test(flavor = "multi_thread")]
async fn gitattributes_cannot_hide_signals() {
    // Committed with the work.
    let rig = Rig::new(&base());
    weaken(&rig, &[(".gitattributes", Some("*.rs -diff\n"))]);
    let (signals, _) = signals_of(&rig.verify(Some(spec())).await).expect("signals");
    assert_eq!(signals, weakened());

    // Untracked, beside the test file (and the global attributes file ignored too).
    let rig = Rig::logging(&base());
    weaken(&rig, &[]);
    write(&rig.worktree, "tests/.gitattributes", "* -diff\n");
    write(&rig.worktree, "src/.gitattributes", "* -diff\n");
    let result = rig.verify(Some(spec())).await;
    let crate::run::engine::OpResult::DoneChecked { signals, .. } = &result else {
        panic!("{result:?}")
    };
    assert_eq!(signals.as_ref().expect("signals").list, weakened());
    let log = rig.logged();
    let u0 = log.iter().find(|l| l.contains("-U0")).expect("a -U0 read");
    for flag in ["--text", "--no-ext-diff", "core.attributesFile=/dev/null"] {
        assert!(u0.contains(flag), "{flag}: {u0}");
    }
}

/// Ruling C-20 (2): `color.ui = always` in the repository's config colours no read.
#[tokio::test(flavor = "multi_thread")]
async fn colour_config_does_not_change_the_reads() {
    let rig = Rig::logging(&base());
    // The user's colour config reaches the task checkout through its config's include
    // of the repository's; the checkout's own config is the engine's, byte for byte
    // (third review of the Linux worker-git fix, m2), so it is not set there.
    git(&rig.root, &["config", "color.ui", "always"]);
    weaken(&rig, &[]);
    let (signals, _) = signals_of(&rig.verify(Some(spec())).await).expect("signals");
    assert_eq!(signals, weakened(), "src/lib.rs is found by the grep");
    let log = rig.logged();
    let grep = log.iter().find(|l| l.contains(" grep ")).expect("a grep");
    assert!(grep.contains("--no-color"), "{grep}");
}

/// Ruling C-20 (3): a test module removed, or the file holding one deleted, is
/// `TestCodeRemoved`, whatever `test_paths` says.
#[tokio::test(flavor = "multi_thread")]
async fn removing_a_test_module_or_its_file_is_a_signal() {
    let mut files = base();
    files.push((
        "src/other.rs",
        "#[cfg(test)]\nmod t {\n    fn b() { assert!(x()); }\n}\n",
    ));
    let rig = Rig::new(&files);
    rig.commit(&[
        ("src/lib.rs", Some("pub fn f() {}\n")),
        ("src/other.rs", None),
    ]);
    let (signals, _) = signals_of(&rig.verify(Some(spec())).await).expect("signals");
    assert_eq!(
        signals,
        vec![
            Signal::TestCodeRemoved {
                path: "src/lib.rs".into(),
                asserts_removed: 1
            },
            Signal::TestCodeRemoved {
                path: "src/other.rs".into(),
                asserts_removed: 1
            },
        ]
    );
}

/// Ruling C-20 (4, 8): a cut diff takes its deleted files from `--diff-filter=DT`, so
/// a test file replaced by a symbolic link still bounces, and says it was too large.
#[tokio::test(flavor = "multi_thread")]
async fn a_cut_diff_catches_a_test_file_replaced_by_a_link() {
    let rig = Rig::new(&base());
    std::fs::remove_file(rig.worktree.join("tests/t.rs")).unwrap();
    std::os::unix::fs::symlink("../src/lib.rs", rig.worktree.join("tests/t.rs")).unwrap();
    rig.commit(&[]);
    let head = git(&rig.worktree, &["rev-parse", "HEAD"]);
    // The claim's own check imports the worker's commit first, as `VerifyDone` does.
    rig.verify(None).await;
    let limits = DiffLimits {
        bytes: 64,
        timeout: T,
    };
    let found = done_signals_with(
        &rig.service.git(),
        &rig.worktree,
        (&rig.start, &head),
        &spec(),
        limits,
        T,
    )
    .unwrap();
    assert_eq!(
        found,
        ClaimSignals {
            list: vec![
                Signal::DeletedTestFile {
                    path: "tests/t.rs".into()
                },
                Signal::DiffTooLarge,
            ],
            more: 0,
            base: rig.start.clone(),
            restore_from: Default::default(),
            unlimited: 0,
        }
    );
}

/// Ruling C-20 (8): a `-U0` read that does not finish in time is treated as cut: the
/// deleted test file is still found, `DiffTooLarge` is added, and the claim goes on.
#[tokio::test(flavor = "multi_thread")]
async fn a_diff_that_times_out_is_too_large_and_keeps_its_deleted_files() {
    // The wrapper (the test's own script) stalls only the `-U0` read.
    let rig = Rig::with_wrapper(
        &base(),
        Some("case \" $* \" in *\" -U0 \"*) sleep 5 ;; esac"),
    );
    rig.commit(&[("tests/t.rs", None)]);
    let head = git(&rig.worktree, &["rev-parse", "HEAD"]);
    // The claim's own check imports the worker's commit first, as `VerifyDone` does.
    rig.verify(None).await;
    let limits = DiffLimits {
        bytes: crate::run::git::SIGNALS_DIFF_BYTES,
        timeout: Duration::from_millis(500),
    };
    let found = done_signals_with(
        &rig.service.git(),
        &rig.worktree,
        (&rig.start, &head),
        &spec(),
        limits,
        T,
    )
    .unwrap();
    assert_eq!(
        found.list,
        vec![
            Signal::DeletedTestFile {
                path: "tests/t.rs".into()
            },
            Signal::DiffTooLarge,
        ]
    );
}

/// Ruling C-20 (5): the base is the merge base the diff was read from: the run head
/// the task merged by a refresh, else its start.
#[tokio::test(flavor = "multi_thread")]
async fn the_base_is_the_merged_run_head_after_a_refresh() {
    let rig = Rig::new(&base());
    let id = ["-c", "user.name=Run", "-c", "user.email=run@test"];
    let commit = |dir: &std::path::Path, message: &str| {
        let mut args = id.to_vec();
        args.extend(["commit", "-q", "-am", message]);
        git(dir, &args);
        git(dir, &["rev-parse", "HEAD"])
    };
    // The run head moves on, in the user's repository.
    write(&rig.root, "README", "run\n");
    git(&rig.root, &["add", "README"]);
    let run_head = commit(&rig.root, "run");
    // A task branch from the start that merges that run head, then deletes a test.
    git(&rig.root, &["checkout", "-q", "-b", "task", &rig.start]);
    let mut merge = id.to_vec();
    merge.extend(["merge", "-q", "--no-edit", run_head.as_str()]);
    git(&rig.root, &merge);
    git(&rig.root, &["rm", "-q", "tests/t.rs"]);
    let head = commit(&rig.root, "task");
    let limits = DiffLimits {
        bytes: crate::run::git::SIGNALS_DIFF_BYTES,
        timeout: T,
    };
    let signals = |run_head: &str| {
        done_signals_with(
            &rig.service.git(),
            &rig.root,
            (run_head, &head),
            &spec(),
            limits,
            T,
        )
        .unwrap()
    };
    let refreshed = signals(&run_head);
    assert_eq!(refreshed.base, run_head);
    assert_eq!(
        refreshed.list,
        vec![Signal::DeletedTestFile {
            path: "tests/t.rs".into()
        }]
    );
    // A run head the task never merged: the merge base is its start.
    git(&rig.root, &["checkout", "-q", "main"]);
    write(&rig.root, "README", "later\n");
    let later = commit(&rig.root, "later");
    git(&rig.root, &["checkout", "-q", "-b", "plain", &rig.start]);
    git(&rig.root, &["rm", "-q", "tests/t.rs"]);
    let plain = commit(&rig.root, "plain");
    let found = done_signals_with(
        &rig.service.git(),
        &rig.root,
        (&later, &plain),
        &spec(),
        limits,
        T,
    )
    .unwrap();
    assert_eq!(found.base, rig.start);
}
