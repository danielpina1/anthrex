//! Milestone 9.6 task M9.6.12: `CommitDesignDocs` through the driver's executor
//! (`ops::run` → `design_commit`), against temporary repositories with a
//! repository-local identity. The stored versions are read back with their recorded
//! sizes and SHA-256 and committed byte for byte under decision 24's paths, slug, date
//! and message; the whole commit is one write through the project's git queue, every
//! git call with `--no-optional-locks`, the protect flags and the scrubbed environment,
//! the driver's own index set explicitly. The git plumbing alone is
//! `daemon/tests/run_git_docs.rs`'s.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::manager::{ManagerConfig, WindowManager};
use crate::run::design::commit::{DocSource, DocsCommitSpec};
use crate::run::design::state::sha256_hex;
use crate::run::driver::{GitRoots, OpCtx, RunService};
use crate::run::engine::{OpKind, OpResult};
use crate::run::git;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

pub(super) const T: Duration = Duration::from_secs(30);
const RUN: &str = "r1";
const SPEC: &str = "# Password reset\n\n## Requirements\nR1 Tokens expire.\n";
const PLAN: &str = "# Plan: Reset passwords\n\n## Stage 1\n";
const REPORT: &str = "## Recommendation\nStored.\n\n## Appendix: the drafts\n### A\nx\n";

/// git in `dir` with neither the machine's configuration nor ours; stdout, trimmed.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
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

/// A repository with a repository-local identity at a base commit, the run branch and
/// its integration worktree, run `r1`'s data directory with its stored versions, and
/// the service (its git program `git`, a recording one when given).
pub(super) struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    integration: PathBuf,
    base: String,
    pub(super) service: Arc<RunService>,
    pub(super) ctx: OpCtx,
}

impl Rig {
    pub(super) fn new(program: Option<PathBuf>) -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path().canonicalize().unwrap();
        let root = top.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Run Tester"]);
        git(&root, &["config", "user.email", "run@tester.test"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        let integration = top.join(format!("wt/runs/{RUN}/integration"));
        let os = std::ffi::OsStr::new("git");
        git::create_run_branch(os, &root, &branch(), &base, &integration, T).unwrap();
        let data_dir = top.join(format!("data/runs/{RUN}"));
        std::fs::create_dir_all(data_dir.join("design")).unwrap();
        let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
        let (manager, _events) = WindowManager::new(config);
        let mut run_ctx = crate::run::driver::RunContext::new(
            top.join("data"),
            manager.config(),
            config::Orchestrator::default(),
            Arc::new(NoRoots),
        );
        if let Some(program) = program {
            run_ctx.git = program.into_os_string();
        }
        let service = RunService::new(manager, run_ctx);
        let ctx = OpCtx {
            run_id: RUN.into(),
            project: root.clone(),
            data_dir,
            git_timeout: T,
            check_timeout: T,
            confine: None,
        };
        Rig {
            _tmp: tmp,
            root,
            integration,
            base,
            service,
            ctx,
        }
    }

    /// Stores `text` as `name` in the design folder, as `WriteDoc` would: its source.
    fn stored(&self, folder: &str, name: &str, text: &str) -> DocSource {
        let path = self.ctx.data_dir.join("design").join(name);
        std::fs::write(&path, text).unwrap();
        DocSource {
            folder: folder.into(),
            what: name.trim_end_matches(".md").replace("-v", " v"),
            path,
            bytes: text.len() as u64,
            sha256: sha256_hex(text.as_bytes()),
        }
    }

    fn spec(&self, files: Vec<DocSource>) -> DocsCommitSpec {
        DocsCommitSpec {
            root: self.root.clone(),
            branch: branch(),
            expected_head: self.base.clone(),
            integration: self.integration.clone(),
            docs_dir: "docs/anthrex".into(),
            date: "2026-10-05".into(),
            run_id: RUN.into(),
            files,
            message: "docs: spec and plan for Reset passwords".into(),
        }
    }

    pub(super) fn spec_and_plan(&self) -> DocsCommitSpec {
        let spec = self.stored("specs", "spec-v2.md", SPEC);
        let plan = self.stored("plans", "plan-v1.md", PLAN);
        self.spec(vec![spec, plan])
    }

    fn run(&self, spec: DocsCommitSpec) -> OpResult {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(super::super::ops::run(
            &self.service,
            &self.ctx,
            1,
            OpKind::CommitDesignDocs(Box::new(spec)),
        ))
    }

    fn branch_head(&self) -> String {
        git(
            &self.root,
            &["rev-parse", &format!("refs/heads/{}", branch())],
        )
    }

    fn show(&self, commit: &str, path: &str) -> String {
        git(&self.root, &["show", &format!("{commit}:{path}")])
    }
}

fn branch() -> String {
    format!("anthrex/{RUN}/integration")
}

const SPEC_PATH: &str = "docs/anthrex/specs/2026-10-05-password-reset.md";
const PLAN_PATH: &str = "docs/anthrex/plans/2026-10-05-password-reset.md";

/// Decisions 23 and 24: the approved spec and plan, byte for byte, at
/// `<docs_dir>/specs/<date>-<slug>.md` and `<docs_dir>/plans/<date>-<slug>.md` (the
/// slug from the spec's title), in one commit on the run branch whose parent is the
/// run head, with decision 24's message; the integration worktree is on it.
#[test]
fn approval_commits_the_spec_and_plan_on_the_run_branch() {
    let rig = Rig::new(None);
    let result = rig.run(rig.spec_and_plan());
    let head = match result {
        OpResult::DocsCommitted { head, spec } => {
            assert_eq!(spec, SPEC_PATH);
            head
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(rig.branch_head(), head);
    let parents = git(&rig.root, &["rev-list", "--parents", "-n", "1", &head]);
    assert_eq!(parents, format!("{head} {}", rig.base));
    assert_eq!(rig.show(&head, SPEC_PATH), SPEC.trim_end());
    assert_eq!(rig.show(&head, PLAN_PATH), PLAN.trim_end());
    let message = git(&rig.root, &["log", "-1", "--format=%B", &head]);
    assert_eq!(message, "docs: spec and plan for Reset passwords");
    let on_disk = std::fs::read_to_string(rig.integration.join(PLAN_PATH)).unwrap();
    assert_eq!(on_disk, PLAN);
    assert!(
        !rig.ctx.data_dir.join("design/commit.index").exists(),
        "the driver's index is removed"
    );
}

/// `commit_brainstorm`: the report, its appendix of drafts included, at
/// `<docs_dir>/brainstorms/<date>-<slug>.md`.
#[test]
fn the_brainstorm_report_goes_to_brainstorms_with_its_appendix() {
    let rig = Rig::new(None);
    let mut spec = rig.spec_and_plan();
    spec.files
        .push(rig.stored("brainstorms", "brainstorm-v3.md", REPORT));
    let head = match rig.run(spec) {
        OpResult::DocsCommitted { head, .. } => head,
        other => panic!("{other:?}"),
    };
    let path = "docs/anthrex/brainstorms/2026-10-05-password-reset.md";
    assert_eq!(rig.show(&head, path), REPORT.trim_end());
}

/// The addendum: only the stored, approved bytes are committed. A file that changed
/// since it was written fails the commit, naming it; the branch does not move.
#[test]
fn a_stored_version_that_changed_is_not_committed() {
    let rig = Rig::new(None);
    let spec = rig.spec_and_plan();
    std::fs::write(&spec.files[1].path, "# Plan: edited behind our back\n").unwrap();
    match rig.run(spec) {
        OpResult::Failed { message } => {
            assert!(
                message.starts_with("the stored plan v1 could not be read back: "),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(rig.branch_head(), rig.base);
}

/// A spec whose title gives no slug is committed under the run id.
#[test]
fn a_title_with_no_slug_falls_back_to_the_run_id() {
    let rig = Rig::new(None);
    let spec = rig.stored("specs", "spec-v1.md", "# ???\n\nR1 x\n");
    let plan = rig.stored("plans", "plan-v1.md", PLAN);
    match rig.run(rig.spec(vec![spec, plan])) {
        OpResult::DocsCommitted { spec, .. } => {
            assert_eq!(spec, "docs/anthrex/specs/2026-10-05-r1.md");
        }
        other => panic!("{other:?}"),
    }
}

/// The addendum: a tracked symbolic link on the documents folder's way is its own
/// result, and nothing is written.
#[test]
fn a_symlinked_documents_folder_is_its_own_result() {
    let rig = Rig::new(None);
    std::os::unix::fs::symlink("a.txt", rig.root.join("docs")).unwrap();
    git(&rig.root, &["add", "docs"]);
    git(&rig.root, &["commit", "-q", "-m", "a link"]);
    let head = git(&rig.root, &["rev-parse", "HEAD"]);
    git(
        &rig.root,
        &["update-ref", &format!("refs/heads/{}", branch()), &head],
    );
    let mut spec = rig.spec_and_plan();
    spec.expected_head = head.clone();
    assert_eq!(
        rig.run(spec),
        OpResult::DocsThroughSymlink {
            path: "docs".into()
        }
    );
    assert_eq!(rig.branch_head(), head);
}
