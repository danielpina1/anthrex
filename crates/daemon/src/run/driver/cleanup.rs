//! Decision 20's end of a worktree and of a run (M8a.22's executors): salvage then
//! removal, `run accept` with its clean-up (also re-run for an accept a restart
//! replayed), and `run discard`. Every step is a write through the run's `GitQueue`.

use std::path::PathBuf;
use std::sync::Arc;

use super::ops::failed;
use super::{OpCtx, RunService};
use crate::run::engine::{OpKind, OpResult};
use crate::run::git;

/// Decision 20's discard: every worktree salvaged and removed (a worktree is never
/// removed dirty without its salvage ref), then the run's branches deleted.
pub(super) async fn discard(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    root: PathBuf,
    worktrees: Vec<(PathBuf, String)>,
    branch_prefix: String,
) -> OpResult {
    for (path, reference) in &worktrees {
        service.unwatch_root(path);
        if let Err(error) =
            remove_worktree(service, ctx, root.clone(), path.clone(), reference.clone()).await
        {
            return failed(error);
        }
    }
    match service
        .write(ctx, move |g, t| {
            git::delete_branches(g, &root, &branch_prefix, t)
        })
        .await
    {
        Ok(kept_branches) => OpResult::Finished {
            outcome: "worktrees removed and branches deleted".to_string(),
            kept_branches,
        },
        Err(error) => failed(error),
    }
}

/// Decision 20's salvage, then the removal; `Some(ref)` when the worktree was dirty. A
/// worktree already gone has nothing to salvage.
pub(super) async fn remove_worktree(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    root: PathBuf,
    path: PathBuf,
    reference: String,
) -> Result<Option<String>, String> {
    remove_keeping(service, ctx, root, path, reference, false).await
}

/// [`remove_worktree`], salvaging a clean checkout at its head too when `keep_head`
/// (milestone 9.5 decision 22, a race lane's).
pub(super) async fn remove_keeping(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    root: PathBuf,
    path: PathBuf,
    reference: String,
    keep_head: bool,
) -> Result<Option<String>, String> {
    // Milestone 9.11 decision 14 (A14): a nested repository's work cannot be salvaged
    // into the user's repository, so a checkout holding some is kept, and the op's
    // failure says why (the task's history, `could not remove its worktree: kept …`).
    if path.exists() {
        let at = path.clone();
        let nested = service
            .write(ctx, move |g, t| git::nested_work(g, &at, t))
            .await?;
        if !nested.is_empty() {
            return Err(format!(
                "kept {}: it holds a nested git repository ({}) whose work anthrex cannot \
                 salvage; remove it yourself when done",
                path.display(),
                nested.join(", ")
            ));
        }
    }
    let salvaged = salvage_at(
        service,
        ctx,
        root.clone(),
        path.clone(),
        reference,
        keep_head,
    )
    .await?;
    // Final fix batch F1c: a standalone checkout goes with its repository (the
    // worker's private objects, the engine's own files, its temporary directory).
    let repo = git::checkout_repo_dir(&ctx.data_dir, &path);
    service
        .write(ctx, move |g, t| {
            git::remove_checkout(g, &root, &path, Some(&repo), t)
        })
        .await?;
    Ok(salvaged)
}

/// Decision 20's salvage alone, `keep_head` as [`remove_keeping`]'s: `Some(ref)` when
/// it made one. A worktree already gone is salvaged from its repository, in the run's
/// project ([`salvage_at`]).
pub(super) async fn salvage_in(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    path: PathBuf,
    reference: String,
    keep_head: bool,
) -> Result<Option<String>, String> {
    let project = ctx.project.clone();
    salvage_at(service, ctx, project, path, reference, keep_head).await
}

/// [`salvage_in`] in the user's repository at `root`. A task checkout the user deleted
/// still has its repository, whose `HEAD` and private objects hold the worker's
/// commits: they are kept under `reference` before anything removes it (milestone 9.11
/// decision 13, `git::salvage_gone`).
async fn salvage_at(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    root: PathBuf,
    path: PathBuf,
    reference: String,
    keep_head: bool,
) -> Result<Option<String>, String> {
    let message = salvage_message(&ctx.run_id, &reference);
    if !path.exists() {
        let repo = git::checkout_repo_dir(&ctx.data_dir, &path);
        // Fix round 1 (I2): `keep_head` as for a live checkout (a race lane's commits
        // must outlive its lane branch, which `delete_branches` removes).
        return service
            .write(ctx, move |g, t| match keep_head {
                true => git::salvage_gone_keeping(g, &root, &repo, &reference, &message, t),
                false => git::salvage_gone(g, &root, &repo, &reference, &message, t),
            })
            .await;
    }
    service
        .write(ctx, move |g, t| {
            git::salvage(g, &path, &reference, &message, keep_head, t)
        })
        .await
}

/// `anthrex salvage <run>/<task>`, the task read from the ref
/// `refs/anthrex/salvage/<run>/<task>/<seq>`.
fn salvage_message(run_id: &str, reference: &str) -> String {
    let prefix = format!("refs/anthrex/salvage/{run_id}/");
    let task = reference
        .strip_prefix(&prefix)
        .and_then(|rest| rest.rsplit_once('/'))
        .map_or("run", |(task, _seq)| task);
    format!("anthrex salvage {run_id}/{task}")
}

/// Decision 20's accept. `git::accept` runs its merge under its own 10-minute
/// `ACCEPT_MERGE_TIMEOUT` and is never wrapped in a shorter one (carry T9). Once it has
/// merged the run is accepted whatever follows: a clean-up failure is still `Finished`,
/// naming it.
pub(super) async fn accept(service: &Arc<RunService>, ctx: &OpCtx, kind: OpKind) -> OpResult {
    let OpKind::Accept {
        root,
        base_branch,
        expected_base,
        run_branch,
        expected_run_head,
        message,
        worktrees,
        branch_prefix,
    } = kind
    else {
        unreachable!("accept takes an Accept");
    };
    let r = root.clone();
    let merged = service
        .write(ctx, move |g, t| {
            git::accept(
                g,
                &r,
                &base_branch,
                &expected_base,
                &run_branch,
                &expected_run_head,
                &message,
                t,
            )
        })
        .await;
    match merged {
        Ok(git::AcceptOutcome::Merged { .. }) => {}
        Ok(git::AcceptOutcome::Conflict { files }) => return OpResult::AcceptConflict { files },
        Err(error) => return failed(error),
    }
    let (outcome, kept_branches) = clean_up(service, ctx, root, &worktrees, branch_prefix).await;
    OpResult::Finished {
        outcome,
        kept_branches,
    }
}

/// Accept's clean-up (and a replayed accept's, after a restart): salvage and remove
/// every run worktree, then delete the run's branches.
pub(super) async fn clean_up(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    root: PathBuf,
    worktrees: &[(PathBuf, String)],
    branch_prefix: String,
) -> (String, Vec<String>) {
    for (path, reference) in worktrees {
        service.unwatch_root(path);
        if let Err(error) =
            remove_worktree(service, ctx, root.clone(), path.clone(), reference.clone()).await
        {
            return (format!("accepted; clean-up failed: {error}"), Vec::new());
        }
    }
    match service
        .write(ctx, move |g, t| {
            git::delete_branches(g, &root, &branch_prefix, t)
        })
        .await
    {
        Ok(kept) => ("merged into the base branch".to_string(), kept),
        Err(error) => (format!("accepted; clean-up failed: {error}"), Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    use super::{remove_keeping, salvage_message};
    use crate::manager::{ManagerConfig, WindowManager};
    use crate::run::driver::{GitRoots, OpCtx, RunService};
    use crate::run::git;

    const T: Duration = Duration::from_secs(30);

    struct NoRoots;
    impl GitRoots for NoRoots {
        fn register(&self, _: PathBuf) {}
        fn unregister(&self, _: &Path) {}
    }

    /// git in `dir` with none of the machine's configuration; its stdout, trimmed.
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
            .env_remove("GIT_OBJECT_DIRECTORY")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    /// A repository with its own identity, task `t1`'s checkout of run `r1` (its own
    /// repository in the run's data directory, as `PrepareWorktree` makes it), the
    /// service and the op context.
    struct Rig {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        task: PathBuf,
        repo: PathBuf,
        service: Arc<RunService>,
        ctx: OpCtx,
    }

    impl Rig {
        fn new() -> Rig {
            let tmp = tempfile::tempdir().unwrap();
            let top = tmp.path().canonicalize().unwrap();
            let root = top.join("repo");
            std::fs::create_dir_all(&root).unwrap();
            git(&root, &["init", "-q", "-b", "main"]);
            git(&root, &["config", "user.name", "t"]);
            git(&root, &["config", "user.email", "t@t"]);
            std::fs::write(root.join("a.txt"), "a\n").unwrap();
            git(&root, &["add", "-A"]);
            git(&root, &["commit", "-q", "-m", "base"]);
            let base = git(&root, &["rev-parse", "HEAD"]);
            let data_dir = top.join("data/runs/r1");
            let task = top.join("wt/runs/r1/t1");
            let repo = git::checkout_repo_dir(&data_dir, &task);
            let g = std::ffi::OsStr::new("git");
            git::prepare_task_worktree(g, &root, "anthrex/r1/t1", &base, &task, &repo, T).unwrap();
            let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
            let (manager, _events) = WindowManager::new(config);
            let run_ctx = crate::run::driver::RunContext::new(
                top.join("data"),
                manager.config(),
                config::Orchestrator::default(),
                Arc::new(NoRoots),
            );
            let ctx = OpCtx {
                run_id: "r1".into(),
                project: root.clone(),
                data_dir,
                git_timeout: T,
                check_timeout: T,
                confine: None,
            };
            let service = RunService::new(manager, run_ctx);
            Rig {
                _tmp: tmp,
                root,
                task,
                repo,
                service,
                ctx,
            }
        }

        /// `remove_keeping` of the task's checkout, salvaging to `<run>/t1/1`.
        fn remove(&self) -> Result<Option<String>, String> {
            self.remove_with(false)
        }

        /// [`Rig::remove`] with `keep_head` (a race lane's removal).
        fn remove_with(&self, keep_head: bool) -> Result<Option<String>, String> {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(remove_keeping(
                &self.service,
                &self.ctx,
                self.root.clone(),
                self.task.clone(),
                "refs/anthrex/salvage/r1/t1/1".into(),
                keep_head,
            ))
        }
    }

    /// Milestone 9.11 decision 13 (A3), through the driver: a checkout the user deleted
    /// keeps its worker's commits under the salvage ref, then its repository goes.
    #[test]
    fn a_deleted_checkout_is_salvaged_before_its_repository_goes() {
        let rig = Rig::new();
        std::fs::write(rig.task.join("w.txt"), "work\n").unwrap();
        git(&rig.task, &["add", "-A"]);
        git(&rig.task, &["commit", "-q", "-m", "worker"]);
        let worker = git(&rig.task, &["rev-parse", "HEAD"]);
        std::fs::remove_dir_all(&rig.task).unwrap();

        let reference = "refs/anthrex/salvage/r1/t1/1";
        assert_eq!(rig.remove(), Ok(Some(reference.to_string())));
        assert_eq!(git(&rig.root, &["rev-parse", reference]), worker);
        assert_eq!(
            git(&rig.root, &["show", &format!("{reference}:w.txt")]),
            "work"
        );
        assert!(!rig.repo.exists(), "the checkout's repository was kept");
    }

    /// Fix round 1 (I3): a clean initialized submodule (its `.git` a file naming the
    /// checkout's own `modules/sub`, at a commit the user has) does not block removal.
    #[test]
    fn a_checkout_with_a_clean_initialized_submodule_is_removed() {
        let rig = Rig::new();
        let admin = std::path::PathBuf::from(git(&rig.task, &["rev-parse", "--absolute-git-dir"]));
        let sub = rig.task.join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(admin.join("modules")).unwrap();
        let separate = format!("--separate-git-dir={}", admin.join("modules/sub").display());
        git(&sub, &["init", "-q", &separate]);
        git(&sub, &["fetch", "-q", rig.root.to_str().unwrap(), "HEAD"]);
        git(&sub, &["checkout", "-q", "--detach", "FETCH_HEAD"]);
        let removed = rig.remove();
        assert!(removed.is_ok(), "{removed:?}");
        assert!(!rig.task.exists(), "the checkout was kept");
    }

    /// Fix round 1 (I2): a deleted race-lane checkout whose head the engine already
    /// recorded on its lane branch still gets its salvage ref there (`keep_head`, as a
    /// live lane's clean checkout does), so the commits outlive the branch.
    #[test]
    fn a_deleted_lane_checkout_keeps_its_head() {
        let rig = Rig::new();
        std::fs::write(rig.task.join("w.txt"), "lane\n").unwrap();
        git(&rig.task, &["add", "-A"]);
        git(&rig.task, &["commit", "-q", "-m", "lane work"]);
        let worker = git(&rig.task, &["rev-parse", "HEAD"]);
        assert_eq!(
            git::sync(std::ffi::OsStr::new("git"), &rig.task, T).unwrap(),
            worker
        );
        std::fs::remove_dir_all(&rig.task).unwrap();

        let reference = "refs/anthrex/salvage/r1/t1/1";
        assert_eq!(rig.remove_with(true), Ok(Some(reference.to_string())));
        assert_eq!(git(&rig.root, &["rev-parse", reference]), worker);
        // Then the run's branches go; the commits do not.
        git::delete_branches(std::ffi::OsStr::new("git"), &rig.root, "anthrex/r1", T).unwrap();
        assert_eq!(git(&rig.root, &["rev-parse", reference]), worker);
        assert!(!rig.repo.exists());
    }

    /// `git init` of `<task>/nested`, with its own identity.
    fn nested(rig: &Rig) -> PathBuf {
        let sub = rig.task.join("nested");
        std::fs::create_dir_all(&sub).unwrap();
        git(&sub, &["init", "-q"]);
        git(&sub, &["config", "user.name", "n"]);
        git(&sub, &["config", "user.email", "n@n"]);
        sub
    }

    /// Milestone 9.11 decision 14 (A14): a nested repository holding work the salvage
    /// cannot keep keeps its checkout, and the op says so.
    #[test]
    fn a_nested_repository_with_work_keeps_the_checkout() {
        let rig = Rig::new();
        let sub = nested(&rig);
        std::fs::write(sub.join("c.txt"), "c\n").unwrap();
        git(&sub, &["add", "-A"]);
        git(&sub, &["commit", "-q", "-m", "nested"]);
        std::fs::write(sub.join("p.txt"), "uncommitted\n").unwrap();

        let expected = format!(
            "kept {}: it holds a nested git repository (nested) whose work anthrex cannot \
             salvage; remove it yourself when done",
            rig.task.display()
        );
        assert_eq!(rig.remove(), Err(expected));
        assert!(sub.join("p.txt").is_file(), "the nested work was removed");
        assert!(rig.repo.exists(), "the checkout's repository was removed");
    }

    /// Pinning: an unchanged clone of the user's repository holds nothing to lose (its
    /// `HEAD` is the user's); the checkout goes, as before.
    #[test]
    fn a_clean_nested_repository_is_removed() {
        let rig = Rig::new();
        let clone = rig.task.join("nested");
        let (from, to) = (rig.root.to_str().unwrap(), clone.to_str().unwrap());
        git(&rig.task, &["clone", "-q", from, to]);
        let removed = rig.remove();
        assert!(removed.is_ok(), "{removed:?}");
        assert!(!rig.task.exists(), "the checkout was kept");
        assert!(!rig.repo.exists(), "the checkout's repository was kept");
    }

    #[test]
    fn the_salvage_message_names_the_task() {
        assert_eq!(
            salvage_message("r-1a2b", "refs/anthrex/salvage/r-1a2b/t1/2"),
            "anthrex salvage r-1a2b/t1"
        );
        assert_eq!(
            salvage_message("r-1a2b", "refs/anthrex/salvage/r-1a2b/integration/1"),
            "anthrex salvage r-1a2b/integration"
        );
    }
}
