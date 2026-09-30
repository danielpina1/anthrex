//! Milestone 9.1 task M9.1.13: a merge candidate's tier-2 job through
//! `driver/merge.rs::candidate`, against a temporary git repository of its own (its
//! own `user.name`/`user.email`), shell-script commands that log each run, and a real
//! scheduler and result cache. Whether the candidate was materialized is read from
//! the integration worktree's `HEAD` reflog. No daemon; every command runs unconfined
//! (`ctx.confine` is `None`).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use proto::ModuleNames;

use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::{GitRoots, OpCtx, RunService, merge};
use crate::run::engine::{OpKind, OpResult};
use crate::run::slots::Priority;
use crate::run::tiers::{CacheCtx, GraphSource, StepKind, TierProfile, TierSpec};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

const INTEGRATION: &str = "anthrex/r1/integration";

const BUILD: &str = "echo build >> \"$TIER_LOG\"\n";
const TEST: &str = r#"echo "test ${1:-} slots=$ANTHREX_TEST_SLOTS" >> "$TIER_LOG"
if [ -f "mods/${1:-}/FAIL" ]; then echo "test a::works ... FAILED"; exit 101; fi
exit 0
"#;
const CHECK: &str = r#"echo "check slots=$ANTHREX_TEST_SLOTS tmp=$TMPDIR sock=$ANTHREX_SOCKET" >> "$TIER_LOG"
exit 0
"#;
const GRAPH: &str = "echo '{\"a\":[],\"b\":[\"a\"],\"c\":[]}'\n";

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

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// `main` with modules `mods/a` and `mods/b` and the scripts, `integration` there with
/// its worktree, a task commit (`t1`) changing `mods/a`, and a service.
struct Rig {
    _tmp: tempfile::TempDir,
    top: PathBuf,
    root: PathBuf,
    integration: PathBuf,
    base: String,
    service: Arc<RunService>,
    ctx: OpCtx,
}

impl Rig {
    fn new(task_files: &[(&str, &str)]) -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path().canonicalize().unwrap();
        let root = top.join("repo");
        for (path, text) in [
            ("mods/a/lib.txt", "a\n"),
            ("mods/b/lib.txt", "b\n"),
            ("mods/c/lib.txt", "c\n"),
            ("build.sh", BUILD),
            ("test.sh", TEST),
            ("check.sh", CHECK),
            ("graph.sh", GRAPH),
        ] {
            write(&root.join(path), text);
        }
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Tier Test"]);
        git(&root, &["config", "user.email", "tier@test"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        git(&root, &["branch", INTEGRATION, &base]);
        git(&root, &["checkout", "-q", "-b", "anthrex/r1/t1"]);
        for (path, text) in task_files {
            write(&root.join(path), text);
        }
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "t1"]);
        git(&root, &["checkout", "-q", "main"]);
        let integration = top.join("wt/runs/r1/integration");
        let at = integration.to_string_lossy().to_string();
        git(&root, &["worktree", "add", "-q", &at, INTEGRATION]);
        let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
        let (manager, _events) = WindowManager::new(config);
        let testing = config::Testing {
            test_slots: Some(2),
            ..config::Testing::default()
        };
        let run_ctx = crate::run::driver::RunContext::new(
            top.join("data"),
            manager.config(),
            config::Orchestrator::default(),
            Arc::new(NoRoots),
        )
        .with_testing(testing);
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
            top,
            root,
            integration,
            base,
            service,
            ctx,
        }
    }

    fn head(&self) -> String {
        git(
            &self.root,
            &["rev-parse", &format!("refs/heads/{INTEGRATION}")],
        )
    }

    /// The tiered profile's tier-2 job, as the engine builds it (`head` is the
    /// executor's to fill).
    fn tier(&self) -> TierSpec {
        TierSpec {
            tier: 2,
            stage: 1,
            root: self.root.clone(),
            dir: self.integration.clone(),
            scratch: None,
            diff_base: self.head(),
            head: String::new(),
            profile: TierProfile {
                build_check: Some("sh build.sh".into()),
                module_test: Some("sh test.sh {module}".into()),
                module_graph: GraphSource::Command("sh graph.sh".into()),
                module_names: ModuleNames::Dir,
                ..TierProfile::default()
            },
            check: Some("sh check.sh".into()),
            hub: Vec::new(),
            source: vec!["mods/**".into()],
            modules: vec!["mods/*".into()],
            manifests: vec!["graph.sh".into()],
            single_test: None,
            timeout_secs: 30,
            env: self.env(),
            priority: Priority::Candidate,
            critical: false,
            cache: Some(CacheCtx {
                profile_hash: "0123456789abcdef".into(),
                toolchain: "none".into(),
            }),
            toolchain: None,
            repo_dir: self.top.join("data/repos/r"),
        }
    }

    fn env(&self) -> Vec<(String, String)> {
        let log = self.top.join("tier.log").display().to_string();
        vec![("TIER_LOG".into(), log)]
    }

    /// The task's candidate onto `integration`, guarded at its head.
    fn candidate(&self, tier: Option<TierSpec>, check: Option<&str>) -> OpKind {
        let head = self.head();
        OpKind::MergeCandidate {
            root: self.root.clone(),
            integration: self.integration.clone(),
            run_branch: INTEGRATION.into(),
            expected_run_head: head.clone(),
            base_branch: "main".into(),
            expected_base: self.base.clone(),
            task_head: git(&self.root, &["rev-parse", "refs/heads/anthrex/r1/t1"]),
            message: "anthrex: merge t1: t1".into(),
            check: check.map(str::to_string),
            timeout_secs: 30,
            env: self.env(),
            guarded: vec![(INTEGRATION.into(), head)],
            also_integration: false,
            tier: tier.map(Box::new),
        }
    }

    async fn run(&self, op: u64, kind: OpKind) -> OpResult {
        merge::candidate(&self.service, &self.ctx, op, kind)
            .await
            .unwrap()
    }

    fn lines(&self, file: &str) -> Vec<String> {
        std::fs::read_to_string(self.top.join(file))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// `integration` and its worktree back at the base, as before the first merge.
    fn undo_merge(&self) {
        git(&self.integration, &["reset", "-q", "--hard", &self.base]);
        assert_eq!(self.head(), self.base);
    }
}

/// How many times the integration worktree was detached at a commit (`materialize`'s
/// `checkout --detach`), from its `HEAD` reflog.
fn detaches(rig: &Rig) -> usize {
    git(
        &rig.integration,
        &["reflog", "show", "--format=%gs", "HEAD"],
    )
    .lines()
    .filter(|l| {
        l.starts_with("checkout: moving from ")
            && l.rsplit(' ')
                .next()
                .is_some_and(|to| to.len() == 40 && to.chars().all(|c| c.is_ascii_hexdigit()))
    })
    .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_candidate_on_the_same_tree_runs_nothing_and_is_not_materialized() {
    let rig = Rig::new(&[("mods/a/x.txt", "x\n")]);
    let first = rig.run(1, rig.candidate(Some(rig.tier()), None)).await;
    let OpResult::Merged {
        commit,
        tier: Some(outcome),
    } = first
    else {
        panic!("not merged with tier 2: {first:?}")
    };
    assert_eq!(rig.head(), commit);
    assert!(outcome.ok);
    let kinds: Vec<(StepKind, bool)> = outcome.steps.iter().map(|s| (s.kind, s.cached)).collect();
    let tests = (StepKind::Tests, false);
    assert_eq!(kinds, vec![(StepKind::Build, false), tests, tests]);
    // Decision 16's range: the candidate's change is `mods/a`, and `b` depends on it.
    let ran = rig.lines("tier.log");
    assert_eq!(
        ran,
        vec!["build", "test a slots=2", "test b slots=2"],
        "{ran:?}"
    );
    assert_eq!(detaches(&rig), 1, "materialized once");

    // The same candidate tree again: every step hits, nothing runs or materializes.
    rig.undo_merge();
    let second = rig.run(2, rig.candidate(Some(rig.tier()), None)).await;
    let OpResult::Merged {
        commit,
        tier: Some(outcome),
    } = second
    else {
        panic!("not merged with tier 2: {second:?}")
    };
    assert_eq!(rig.head(), commit);
    assert!(outcome.ok && outcome.steps.iter().all(|s| s.cached));
    assert_eq!(rig.lines("tier.log"), ran, "no command ran");
    assert_eq!(detaches(&rig), 1, "not materialized again");
    // The worktree is back on `integration`, clean, holding the merge.
    let status = git(&rig.integration, &["status", "--porcelain"]);
    assert!(status.is_empty(), "{status}");
    assert!(rig.integration.join("mods/a/x.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_candidate_that_changes_the_graph_input_is_materialized_and_run() {
    // `graph.sh` is the command graph's manifest: the pre-check cannot trust the graph
    // read at the stage head, so the job runs after `materialize`, every time.
    let changed = format!("{GRAPH}# changed\n");
    let rig = Rig::new(&[("mods/a/x.txt", "x\n"), ("graph.sh", &changed)]);
    let _ = rig.run(1, rig.candidate(Some(rig.tier()), None)).await;
    rig.undo_merge();
    let second = rig.run(2, rig.candidate(Some(rig.tier()), None)).await;
    assert!(matches!(second, OpResult::Merged { .. }), "{second:?}");
    assert_eq!(detaches(&rig), 2, "materialized again");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_red_tier2_is_candidate_red_with_its_failing_names_and_nothing_moves() {
    let rig = Rig::new(&[("mods/a/x.txt", "x\n"), ("mods/a/FAIL", "")]);
    let result = rig.run(1, rig.candidate(Some(rig.tier()), None)).await;
    let OpResult::CandidateRed {
        code,
        tail,
        tier: Some(outcome),
        ..
    } = result
    else {
        panic!("not red: {result:?}")
    };
    assert_eq!(code, Some(101));
    assert!(tail.contains("test a::works ... FAILED"), "{tail}");
    assert!(!outcome.ok);
    assert_eq!(
        outcome.steps.last().map(|s| s.failing.clone()),
        Some(vec!["a::works".to_string()])
    );
    assert_eq!(rig.head(), rig.base, "nothing merged");
    let on = git(&rig.integration, &["symbolic-ref", "-q", "HEAD"]);
    assert_eq!(on, format!("refs/heads/{INTEGRATION}"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_untiered_candidate_check_waits_for_all_slots_and_runs_isolated() {
    let rig = Rig::new(&[("mods/a/x.txt", "x\n")]);
    let result = rig.run(9, rig.candidate(None, Some("sh check.sh"))).await;
    assert!(
        matches!(result, OpResult::Merged { tier: None, .. }),
        "{result:?}"
    );
    let ran = rig.lines("tier.log");
    assert_eq!(ran.len(), 1, "{ran:?}");
    let line = &ran[0];
    // Decision 24: a candidate asks for every slot; decision 28: its own TMPDIR, with
    // the daemon's coordinates under it.
    assert!(line.starts_with("check slots=2 tmp="), "{line}");
    let tmp = line
        .split(" tmp=")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    assert!(tmp.ends_with("/s9-candidate"), "{line}");
    assert!(line.ends_with(&format!("sock={tmp}/d.sock")), "{line}");
    assert!(!Path::new(tmp).exists(), "the step directory is removed");
}
