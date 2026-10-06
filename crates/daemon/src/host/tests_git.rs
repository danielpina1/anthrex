//! Task M9.2.4: `push` and `fetch` against real git: a local bare repository as the
//! remote, `SystemRunner` running `git` (no `gh` anywhere: its path does not exist).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;
use crate::worktree::run_git;

pub(super) const RUN: &str = "r1a2b";
pub(super) const NO_GH: &str = "/nonexistent/anthrex-test/gh";

/// Test setup's git, through `worktree::run_git` like the code under test; identity and
/// signing are given per command, so the user's global config cannot change a commit.
pub(super) fn git(dir: &Path, args: &[&str]) -> String {
    let mut all = vec![
        "-c",
        "user.name=anthrex test",
        "-c",
        "user.email=test@anthrex.invalid",
        "-c",
        "commit.gpgSign=false",
        "-c",
        "core.hooksPath=/dev/null",
    ];
    all.extend_from_slice(args);
    let os: Vec<&OsStr> = all.iter().map(OsStr::new).collect();
    let out = run_git(
        OsStr::new("git"),
        dir,
        &os,
        Instant::now() + Duration::from_secs(30),
    )
    .unwrap();
    assert!(out.success, "git {args:?}: {}", out.stderr);
    out.stdout.trim().to_string()
}

pub(super) fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["commit", "-q", "--allow-empty", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

pub(super) struct Rig {
    pub(super) _tmp: tempfile::TempDir,
    pub(super) bare: PathBuf,
    pub(super) work: PathBuf,
    pub(super) repo: HostRepo,
    pub(super) host: GhHost<SystemRunner>,
}

impl Rig {
    pub(super) fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("remote.git");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        git(&bare, &["init", "-q", "--bare"]);
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["remote", "add", "origin", &bare.to_string_lossy()]);
        let repo = HostRepo {
            host: "github.com".to_string(),
            owner: "o".to_string(),
            name: "r".to_string(),
            remote: "origin".to_string(),
            root: work.clone(),
        };
        Rig {
            _tmp: tmp,
            bare,
            work,
            repo,
            host: GhHost::new(SystemRunner::new(NO_GH, "git"), NO_GH, "git"),
        }
    }

    pub(super) fn push(&self, stage: u16, sha: &str) -> Result<PushOutcome, HostError> {
        self.host.push(&PushReq {
            repo: self.repo.clone(),
            run_id: RUN.to_string(),
            stage,
            sha: sha.to_string(),
        })
    }

    fn remote_head(&self, stage: u16) -> String {
        let refname = format!("refs/heads/anthrex/{RUN}/stage-{stage}");
        git(&self.bare, &["rev-parse", "--verify", "--quiet", &refname])
    }

    /// As someone rewriting the remote would: `branch` on the remote set to `sha`.
    fn set_remote(&self, branch: &str, sha: &str) {
        git(
            &self.work,
            &[
                "push",
                "-q",
                "origin",
                &format!("{sha}:refs/heads/scratch-{sha}"),
            ],
        );
        git(
            &self.bare,
            &["update-ref", &format!("refs/heads/{branch}"), sha],
        );
        git(
            &self.bare,
            &["update-ref", "-d", &format!("refs/heads/scratch-{sha}")],
        );
    }

    /// Fix round 1 (I3): a fetch with every remote-tracking ref deleted first, then
    /// checked to have written none, nor a `FETCH_HEAD` (decision 13). Deleting first
    /// matters: `git push` writes the same tracking ref git's opportunistic update would.
    pub(super) fn fetch(&self, req: &FetchReq) -> Result<FetchOutcome, HostError> {
        let tracking = || {
            git(
                &self.work,
                &["for-each-ref", "--format=%(refname)", "refs/remotes"],
            )
        };
        for refname in tracking().lines() {
            git(&self.work, &["update-ref", "-d", refname]);
        }
        assert_eq!(tracking(), "");
        let fetched = self.host.fetch(req);
        assert_eq!(tracking(), "", "the fetch wrote a remote-tracking ref");
        assert!(!self.work.join(".git/FETCH_HEAD").exists());
        fetched
    }

    pub(super) fn local(&self, branch: &str) -> String {
        git(
            &self.work,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
    }
}

#[test]
fn push_to_a_rewritten_remote_is_rejected_never_forced() {
    let rig = Rig::new();
    let a = commit(&rig.work, "a");
    let b = commit(&rig.work, "b");
    // Ruling I1: the user's push.followTags and push.recurseSubmodules never apply.
    git(&rig.work, &["config", "push.followTags", "true"]);
    git(
        &rig.work,
        &["config", "push.recurseSubmodules", "on-demand"],
    );
    git(&rig.work, &["tag", "-a", "v1", "-m", "a release", &a]);
    git(&rig.work, &["tag", "-a", "v2", "-m", "another", &b]);
    assert_eq!(rig.push(1, &a), Ok(PushOutcome::Pushed), "a new branch");
    assert_eq!(rig.remote_head(1), a);
    assert_eq!(rig.push(1, &a), Ok(PushOutcome::UpToDate));
    assert_eq!(rig.push(1, &b), Ok(PushOutcome::Pushed), "a fast-forward");
    assert_eq!(rig.remote_head(1), b);
    let tags = git(
        &rig.bare,
        &["for-each-ref", "--format=%(refname)", "refs/tags"],
    );
    assert_eq!(tags, "", "no tag reaches the remote");

    // Someone rewrote the remote branch: a commit that is not a descendant of it is
    // rejected by git itself, and the remote ref does not move.
    git(&rig.work, &["checkout", "-q", "--detach", &a]);
    let other = commit(&rig.work, "other");
    assert_eq!(
        rig.push(1, &other),
        Ok(PushOutcome::Rejected {
            reason: "non-fast-forward".to_string()
        })
    );
    assert_eq!(rig.remote_head(1), b, "never forced");

    // Ruling R-11: a refusal by the remote (a hook, a protection rule) is told apart.
    let hook = rig.bare.join("hooks/pre-receive");
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho 'protected by policy' >&2\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        rig.push(2, &a),
        Ok(PushOutcome::Refused {
            reason: format!(
                "the remote refused the push of anthrex/{RUN}/stage-2: pre-receive hook declined"
            )
        })
    );
    std::fs::remove_file(&hook).unwrap();

    // A branch delete of a stage branch, and of one already gone, both succeed.
    let delete = DeleteBranchReq {
        repo: rig.repo.clone(),
        run_id: RUN.to_string(),
        stage: 1,
    };
    assert_eq!(rig.host.delete_branch(&delete), Ok(()));
    assert_eq!(rig.host.delete_branch(&delete), Ok(()));
    let refs = git(&rig.bare, &["for-each-ref", "--format=%(refname)"]);
    assert!(!refs.contains("stage-1"), "{refs}");
}

#[test]
fn fetch_adopts_only_a_descendant() {
    let rig = Rig::new();
    let a = commit(&rig.work, "a");
    let stage = format!("anthrex/{RUN}/stage-1");
    let integration = format!("anthrex/{RUN}/integration");
    git(&rig.work, &["branch", &stage, &a]);
    git(&rig.work, &["branch", &integration, &a]);
    let b = commit(&rig.work, "b (the user's commit on the stage branch)");
    rig.push(1, &b).unwrap();
    let into = format!("refs/anthrex/{RUN}/remote/stage-1");
    let fetch = |expected: &str, also_integration: bool| {
        rig.fetch(&FetchReq {
            repo: rig.repo.clone(),
            run_id: RUN.to_string(),
            branch: stage.clone(),
            into: into.clone(),
            adopt: Some(Adopt {
                local_ref: stage.clone(),
                expected_local: expected.to_string(),
                also_integration,
            }),
            parents_of: None,
            contains: None,
            deadline: None,
        })
    };

    // The remote head descends from the local one: adopted by compare-and-swap, with
    // `integration` when asked; the fetch lands in anthrex's own ref only.
    assert_eq!(
        fetch(&a, true),
        Ok(FetchOutcome::Adopted { sha: b.clone() })
    );
    assert_eq!(rig.local(&stage), b);
    assert_eq!(rig.local(&integration), b);
    assert_eq!(git(&rig.work, &["rev-parse", &into]), b);

    // Not a descendant (a force-push left another commit): nothing moves.
    git(&rig.work, &["checkout", "-q", "--detach", &a]);
    let rewritten = commit(&rig.work, "rewritten");
    rig.set_remote(&stage, &rewritten);
    assert_eq!(
        fetch(&b, false),
        Ok(FetchOutcome::NotDescendant {
            remote: rewritten.clone()
        })
    );
    assert_eq!(rig.local(&stage), b);

    // The local ref moved meanwhile: the swap is refused and names where it is.
    let c = {
        git(&rig.work, &["checkout", "-q", "--detach", &b]);
        commit(&rig.work, "c")
    };
    rig.set_remote(&stage, &c);
    git(
        &rig.work,
        &["update-ref", &format!("refs/heads/{stage}"), &a],
    );
    assert_eq!(
        fetch(&b, false),
        Ok(FetchOutcome::LocalMoved { local: a.clone() })
    );
    assert_eq!(rig.local(&stage), a);
    assert_eq!(
        rig.local(&integration),
        b,
        "integration did not move either"
    );

    // Without an adoption the fetch only reads; a missing branch is `Missing`. The local
    // base branch never moves.
    let main_before = rig.local("main");
    let base = || {
        rig.fetch(&FetchReq {
            repo: rig.repo.clone(),
            run_id: RUN.to_string(),
            branch: "main".to_string(),
            into: format!("refs/anthrex/{RUN}/remote/base"),
            adopt: None,
            parents_of: None,
            contains: None,
            deadline: None,
        })
    };
    assert_eq!(base(), Ok(FetchOutcome::Missing));
    git(
        &rig.work,
        &["push", "-q", "origin", &format!("{a}:refs/heads/main")],
    );
    let fetched = FetchOutcome::Fetched {
        sha: a.clone(),
        parents: None,
        contains: None,
    };
    assert_eq!(base(), Ok(fetched));
    assert_eq!(rig.local("main"), main_before);
    assert_ne!(main_before, a);
}

/// Task M9.2.11, ruling R-4: a base fetch asked for a merged PR's merge commit counts
/// its parents locally, after the fetch brought it: two for a merge commit, one for a
/// squash (or rebase); a commit the repository does not have counts nothing.
#[test]
fn a_base_fetch_counts_the_merge_commits_parents() {
    let rig = Rig::new();
    let a = commit(&rig.work, "a");
    git(&rig.work, &["checkout", "-q", "-b", "side"]);
    let side = commit(&rig.work, "side");
    git(&rig.work, &["checkout", "-q", "main"]);
    let b = commit(&rig.work, "b");
    git(&rig.work, &["merge", "-q", "--no-ff", "-m", "merge", &side]);
    let merge = git(&rig.work, &["rev-parse", "HEAD"]);
    let squash = commit(&rig.work, "squash of side");
    git(
        &rig.work,
        &["push", "-q", "origin", &format!("{squash}:refs/heads/main")],
    );
    let main_before = rig.local("main");
    let fetch = |oid: Option<&str>| {
        rig.fetch(&FetchReq {
            repo: rig.repo.clone(),
            run_id: RUN.to_string(),
            branch: "main".to_string(),
            into: format!("refs/anthrex/{RUN}/remote/base"),
            adopt: None,
            parents_of: oid.map(str::to_string),
            contains: None,
            deadline: None,
        })
    };
    let fetched = |parents| {
        Ok(FetchOutcome::Fetched {
            sha: squash.clone(),
            parents,
            contains: None,
        })
    };
    assert_eq!(fetch(Some(&merge)), fetched(Some(2)));
    assert_eq!(fetch(Some(&squash)), fetched(Some(1)));
    assert_eq!(fetch(Some(&b)), fetched(Some(1)));
    assert_eq!(fetch(Some(&"e".repeat(40))), fetched(None));
    assert_eq!(fetch(None), fetched(None));
    // Fix round 1 (m2): an oid that is not one fails nothing; git is not asked.
    assert_eq!(fetch(Some("not-an-object-id")), fetched(None));
    assert_eq!(fetch(Some("HEAD")), fetched(None));
    assert_eq!(rig.local("main"), main_before, "the local base never moves");
    assert_ne!(a, squash);
}

/// Fix round 1 (m3): the `rev-list` arm takes exactly one shape, read-only, with a full
/// object id; every other is refused before any process starts.
#[test]
fn the_rev_list_arm_takes_only_a_full_object_id_read_only() {
    use super::allow::{AllowCtx, check};
    let ctx = AllowCtx {
        run_id: Some(RUN),
        remote: "origin",
        base_branch: Some("main"),
        repo: Some("o/r"),
        pulls: &[],
    };
    let sha = "a560bea91b8cd58b3c0d78e5db98c7fdc8e5036b";
    let read = ["-c", "core.hooksPath=/dev/null"];
    let write = [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "commit.gpgSign=false",
        "-c",
        "core.logAllRefUpdates=false",
    ];
    let args = |flags: &[&str], rest: &[&str]| -> Vec<String> {
        flags.iter().chain(rest).map(|a| a.to_string()).collect()
    };
    let ok = args(&read, &["rev-list", "--parents", "-n", "1", sha]);
    assert_eq!(check(Program::Git, &ok, &ctx), Ok(()));
    for refused in [
        args(
            &read,
            &["rev-list", "--parents", "-n", "1", "refs/heads/main"],
        ),
        args(&read, &["rev-list", "--parents", "-n", "1", "main"]),
        args(&read, &["rev-list", "--parents", "-n", "2", sha]),
        args(&read, &["rev-list", "--parents", "-n", "1", "--all"]),
        args(&read, &["rev-list", "--all", "--parents", "-n", "1", sha]),
        args(&read, &["rev-list", "--parents", "-n", "1", &sha[..7]]),
        args(&read, &["rev-list", "--parents", "-n", "1", sha, sha]),
        args(&read, &["rev-list", "--parents", "-n", "1", sha, "--", "x"]),
        args(&write, &["rev-list", "--parents", "-n", "1", sha]),
        args(&[], &["rev-list", "--parents", "-n", "1", sha]),
    ] {
        assert!(
            matches!(
                check(Program::Git, &refused, &ctx),
                Err(HostError::Forbidden(_))
            ),
            "{refused:?} was not refused"
        );
    }
}

/// Fix wave A2 (review A, M2): an adoption checks its op's deadline just before the
/// compare-and-swap. With too little of the bound left for the swap's own timeout, it
/// answers `TimedOut` and moves nothing, so a swap never lands after the op's answer
/// told the engine it timed out.
#[test]
fn an_adoption_past_its_deadline_moves_nothing() {
    let rig = Rig::new();
    let a = commit(&rig.work, "a");
    let stage = format!("anthrex/{RUN}/stage-1");
    let integration = format!("anthrex/{RUN}/integration");
    git(&rig.work, &["branch", &stage, &a]);
    git(&rig.work, &["branch", &integration, &a]);
    let b = commit(&rig.work, "b (the user's commit on the stage branch)");
    rig.push(1, &b).unwrap();
    let fetch = |deadline: Option<Instant>| {
        rig.fetch(&FetchReq {
            repo: rig.repo.clone(),
            run_id: RUN.to_string(),
            branch: stage.clone(),
            into: format!("refs/anthrex/{RUN}/remote/stage-1"),
            adopt: Some(Adopt {
                local_ref: stage.clone(),
                expected_local: a.clone(),
                also_integration: true,
            }),
            parents_of: None,
            contains: None,
            deadline,
        })
    };

    // Less than the swap's own bound is left: refused, nothing moved.
    let close = Instant::now() + HOST_READ_TIMEOUT - Duration::from_secs(1);
    let Err(HostError::TimedOut(text)) = fetch(Some(close)) else {
        panic!("an adoption without room for its swap went ahead");
    };
    assert_eq!(
        text,
        format!("fetch ran out of its bound before adopting into {stage}; nothing was moved")
    );
    assert_eq!(rig.local(&stage), a);
    assert_eq!(rig.local(&integration), a);

    // With room left, the same adoption goes ahead.
    let roomy = Instant::now() + HOST_READ_TIMEOUT + Duration::from_secs(60);
    assert_eq!(
        fetch(Some(roomy)),
        Ok(FetchOutcome::Adopted { sha: b.clone() })
    );
    assert_eq!(rig.local(&stage), b);
}

/// Deferred from task 4: a user's `fetch.prune` and `fetch.pruneTags` never reach
/// anthrex's fetch, so it deletes neither their local tags nor anything under its own
/// private refs that the remote lacks (`--no-prune --no-prune-tags`).
#[test]
fn a_users_prune_config_prunes_nothing() {
    let rig = Rig::new();
    let a = commit(&rig.work, "a");
    git(&rig.work, &["push", "-q", "origin", "main"]);
    git(&rig.work, &["tag", "local-only", &a]);
    let gone = format!("refs/anthrex/{RUN}/remote/gone");
    git(&rig.work, &["update-ref", &gone, &a]);
    git(&rig.work, &["config", "fetch.prune", "true"]);
    git(&rig.work, &["config", "fetch.pruneTags", "true"]);
    git(&rig.work, &["config", "remote.origin.prune", "true"]);
    git(&rig.work, &["config", "remote.origin.pruneTags", "true"]);
    let fetched = rig.fetch(&FetchReq {
        repo: rig.repo.clone(),
        run_id: RUN.to_string(),
        branch: "main".to_string(),
        into: format!("refs/anthrex/{RUN}/remote/base"),
        adopt: None,
        parents_of: None,
        contains: None,
        deadline: None,
    });
    assert_eq!(
        fetched,
        Ok(FetchOutcome::Fetched {
            sha: a.clone(),
            parents: None,
            contains: None,
        })
    );
    assert_eq!(
        git(&rig.work, &["tag", "--list", "local-only"]),
        "local-only"
    );
    assert_eq!(git(&rig.work, &["rev-parse", "--verify", &gone]), a);
}
