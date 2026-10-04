//! Milestone 9.1 task M9.1.16: `OpKind::VerifyDone`'s signals (decision 40) through the
//! driver, against a temporary repository and a real task worktree. A git wrapper (the
//! test's own script) logs every invocation, so the tests see which reads ran.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::{GitRoots, OpCtx, RunService};
use crate::run::engine::{OpKind, OpResult};
use crate::run::tiers::{Signal, SignalsSpec};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
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
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn write(dir: &Path, path: &str, text: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

/// The absolute path of the real git, for the wrapper.
fn real_git() -> String {
    let out = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let path = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(path.starts_with('/'), "{path}");
    path
}

/// A repository whose base holds `base` files, a task worktree started there, and a
/// service whose git is a wrapper logging each invocation's arguments to `git.log`.
struct Rig {
    _tmp: tempfile::TempDir,
    log: PathBuf,
    root: PathBuf,
    worktree: PathBuf,
    start: String,
    service: Arc<RunService>,
    ctx: OpCtx,
}

impl Rig {
    /// A rig on the real git: most tests need no log, and a wrapper doubles every
    /// process start, which loads the machine under the other tests' wall-clock bounds.
    fn new(base: &[(&str, &str)]) -> Rig {
        Rig::with_wrapper(base, None)
    }

    /// A rig whose git logs every invocation.
    fn logging(base: &[(&str, &str)]) -> Rig {
        Rig::with_wrapper(base, Some(""))
    }

    /// As [`Rig::new`], through the logging wrapper when `extra` is given, with those
    /// shell lines in it before it runs git.
    fn with_wrapper(base: &[(&str, &str)], extra: Option<&str>) -> Rig {
        let logs = extra.is_some();
        let extra = extra.unwrap_or_default();
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path().canonicalize().unwrap();
        let log = top.join("git.log");
        let wrapper = top.join("git-wrapper.sh");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n{extra}\nexec '{}' \"$@\"\n",
                log.display(),
                real_git()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        let root = top.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        for (path, text) in base {
            write(&root, path, text);
        }
        git(&root, &["add", "-A"]);
        git(
            &root,
            &[
                "-c",
                "user.name=Signal Test",
                "-c",
                "user.email=signal@test",
                "commit",
                "-q",
                "-m",
                "base",
            ],
        );
        let start = git(&root, &["rev-parse", "HEAD"]);
        let worktree = top.join("wt/runs/r1/t1");
        crate::run::git::prepare_worktree(
            std::ffi::OsStr::new(&real_git()),
            &root,
            "anthrex/r1/t1",
            &start,
            &worktree,
            Duration::from_secs(30),
        )
        .unwrap();
        let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
        let (manager, _events) = WindowManager::new(config);
        let mut run_ctx = crate::run::driver::RunContext::new(
            top.join("data"),
            manager.config(),
            config::Orchestrator::default(),
            Arc::new(NoRoots),
        );
        if !extra.is_empty() || logs {
            run_ctx.git = wrapper.into_os_string();
        }
        let service = RunService::new(manager, run_ctx);
        let ctx = OpCtx {
            run_id: "r1".into(),
            project: root.clone(),
            data_dir: top.join("data/runs/r1"),
            git_timeout: Duration::from_secs(30),
            check_timeout: Duration::from_secs(30),
            confine: None,
        };
        Rig {
            _tmp: tmp,
            log,
            root,
            worktree,
            start,
            service,
            ctx,
        }
    }

    /// The worker's commit in its worktree: `files` written (`None` deletes).
    fn commit(&self, files: &[(&str, Option<&str>)]) {
        for (path, text) in files {
            match text {
                Some(text) => write(&self.worktree, path, text),
                None => {
                    git(&self.worktree, &["rm", "-q", "--", path]);
                }
            }
        }
        git(&self.worktree, &["add", "-A"]);
        git(
            &self.worktree,
            &[
                "-c",
                "user.name=Worker",
                "-c",
                "user.email=worker@test",
                "commit",
                "-q",
                "-m",
                "work",
            ],
        );
    }

    async fn verify(&self, signals: Option<SignalsSpec>) -> OpResult {
        self.verify_at(&self.start.clone(), signals).await
    }

    /// [`Rig::verify`] with the op's run head given.
    async fn verify_at(&self, run_head: &str, signals: Option<SignalsSpec>) -> OpResult {
        let kind = OpKind::VerifyDone {
            worktree: self.worktree.clone(),
            start: self.start.clone(),
            run_head: run_head.to_string(),
            owns: vec!["**".into()],
            generated: Vec::new(),
            protected: Vec::new(),
            spill_exempt: false,
            red: None,
            resolution: None,
            not_own: Vec::new(),
            not_run: Vec::new(),
            signals,
            spill_base: None,
            sync: None,
        };
        std::fs::write(&self.log, "").unwrap();
        super::run(&self.service, &self.ctx, 7, kind).await
    }

    fn logged(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

fn spec() -> SignalsSpec {
    SignalsSpec {
        test_paths: vec!["tests/**".into(), "crates/*/tests/**".into()],
        skip_markers: vec!["#[ignore]".into()],
        red: None,
    }
}

/// The result's signals: `None` when it carries none.
fn signals_of(result: &OpResult) -> Option<(Vec<Signal>, u32)> {
    match result {
        OpResult::DoneChecked {
            signals, commits, ..
        } => {
            assert_eq!(*commits, 1, "{result:?}");
            signals.as_deref().map(|s| (s.list.clone(), s.more))
        }
        other => panic!("{other:?}"),
    }
}

/// Decision 6 (**pinning**): with no `SignalsSpec` (an untiered profile), no `-U0` diff
/// and no `git grep` runs, and the result carries no signal; with one, both run.
#[tokio::test(flavor = "multi_thread")]
async fn untiered_profile_computes_no_signals() {
    let rig = Rig::logging(&[
        ("tests/gone.rs", "#[test]\nfn a() { assert!(true); }\n"),
        ("src/lib.rs", "pub fn f() {}\n#[cfg(test)]\nmod t {}\n"),
    ]);
    rig.commit(&[
        ("tests/gone.rs", None),
        (
            "src/lib.rs",
            Some("pub fn f() {}\n#[ignore]\n#[cfg(test)]\nmod t {}\n"),
        ),
    ]);
    let result = rig.verify(None).await;
    assert_eq!(signals_of(&result), None);
    let log = rig.logged();
    assert!(!log.is_empty(), "the wrapper ran");
    assert!(
        !log.iter()
            .any(|l| l.contains("-U0") || l.contains(" grep ")),
        "{log:#?}"
    );
    // The control: asked for, both reads run through the same git.
    let result = rig.verify(Some(spec())).await;
    let (signals, _) = signals_of(&result).expect("signals");
    assert_eq!(
        signals[0],
        Signal::DeletedTestFile {
            path: "tests/gone.rs".into()
        }
    );
    let log = rig.logged();
    assert_eq!(
        log.iter().filter(|l| l.contains("-U0")).count(),
        1,
        "{log:#?}"
    );
    let u0 = log.iter().find(|l| l.contains("-U0")).unwrap();
    assert!(u0.contains("--no-optional-locks"), "{u0}");
    assert!(u0.contains(&format!("{}..", rig.start)), "{u0}");
    assert!(log.iter().any(|l| l.contains(" grep ")), "{log:#?}");
}

/// Decision 40 through the op: deleted test files first, then each file in diff order
/// with its skip markers and its assertion loss; 20 kept and the rest counted. The
/// `#[cfg(test)]` grep reads each path literally: as a plain pathspec, `:weird.rs`
/// would be read as the magic form of `weird.rs`, and its assertion loss missed.
#[tokio::test(flavor = "multi_thread")]
async fn signals_are_capped_and_ordered() {
    let asserts = "#[cfg(test)]\nmod t {\n    fn a() {\n        assert!(one());\n        assert!(two());\n    }\n}\n";
    let rig = Rig::new(&[
        ("crates/x/tests/t.rs", "#[test]\nfn a() {}\n"),
        (":weird.rs", asserts),
        ("tests/a.rs", "#[test]\nfn a() {}\n"),
        ("tests/b.rs", "#[test]\nfn b() {}\n"),
    ]);
    let skips: String = (0..17)
        .map(|n| format!("#[ignore]\nfn s{n}() {{}}\n"))
        .collect();
    let more: String = (0..3)
        .map(|n| format!("#[ignore]\nfn m{n}() {{}}\n"))
        .collect();
    let fewer = "#[cfg(test)]\nmod t {\n    fn a() {\n    }\n}\n";
    rig.commit(&[
        ("tests/a.rs", None),
        ("tests/b.rs", None),
        (
            "crates/x/tests/t.rs",
            Some(&format!("#[test]\nfn a() {{}}\n{skips}")),
        ),
        (":weird.rs", Some(fewer)),
        ("tests/z.rs", Some(&more)),
    ]);
    let result = rig.verify(Some(spec())).await;
    let (signals, rest) = signals_of(&result).expect("signals");
    // `:weird.rs` sorts first in the diff (`:` is before every letter).
    let mut want = vec![
        Signal::DeletedTestFile {
            path: "tests/a.rs".into(),
        },
        Signal::DeletedTestFile {
            path: "tests/b.rs".into(),
        },
        Signal::AssertionLoss {
            path: ":weird.rs".into(),
            line: 4,
            removed: 2,
            added: 0,
        },
    ];
    want.extend((0..17).map(|n| Signal::SkipMarker {
        path: "crates/x/tests/t.rs".into(),
        line: 3 + 2 * n,
        marker: "#[ignore]".into(),
    }));
    assert_eq!(signals, want);
    assert_eq!(rest, 3, "tests/z.rs's three markers are counted");
}

// Ruling C-20's cases, against the same rig.
#[path = "ops_signals_tests_c20.rs"]
mod c20;

// Milestone 9.5 ruling RP-2: a paired task's implementer's signals, from red.
#[path = "ops_signals_tests_pair.rs"]
mod pair;
