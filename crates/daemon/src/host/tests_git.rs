//! Task M9.2.4: `push` and `fetch` against real git: a local bare repository as the
//! remote, `SystemRunner` running `git` (no `gh` anywhere: its path does not exist).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::*;
use crate::worktree::run_git;

const RUN: &str = "r1a2b";
const NO_GH: &str = "/nonexistent/anthrex-test/gh";

/// Test setup's git, through `worktree::run_git` like the code under test; identity and
/// signing are given per command, so the user's global config cannot change a commit.
fn git(dir: &Path, args: &[&str]) -> String {
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

fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["commit", "-q", "--allow-empty", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

struct Rig {
    _tmp: tempfile::TempDir,
    bare: PathBuf,
    work: PathBuf,
    repo: HostRepo,
    host: GhHost<SystemRunner>,
}

impl Rig {
    fn new() -> Rig {
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

    fn push(&self, stage: u16, sha: &str) -> Result<PushOutcome, HostError> {
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

    fn local(&self, branch: &str) -> String {
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
    assert_eq!(rig.push(1, &a), Ok(PushOutcome::Pushed), "a new branch");
    assert_eq!(rig.remote_head(1), a);
    assert_eq!(rig.push(1, &a), Ok(PushOutcome::UpToDate));
    assert_eq!(rig.push(1, &b), Ok(PushOutcome::Pushed), "a fast-forward");
    assert_eq!(rig.remote_head(1), b);

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
    // `git push` itself updates `refs/remotes/origin/*` (git's own behaviour); the fetch
    // below must add nothing there, nor a `FETCH_HEAD` (decision 13).
    let tracking = || {
        git(
            &rig.work,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/remotes",
            ],
        )
    };
    let tracking_before = tracking();
    let into = format!("refs/anthrex/{RUN}/remote/stage-1");
    let fetch = |expected: &str, also_integration: bool| {
        rig.host.fetch(&FetchReq {
            repo: rig.repo.clone(),
            branch: stage.clone(),
            into: into.clone(),
            adopt: Some(Adopt {
                local_ref: stage.clone(),
                expected_local: expected.to_string(),
                also_integration,
            }),
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
    assert_eq!(tracking(), tracking_before);
    assert!(!rig.work.join(".git/FETCH_HEAD").exists());

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
        rig.host.fetch(&FetchReq {
            repo: rig.repo.clone(),
            branch: "main".to_string(),
            into: format!("refs/anthrex/{RUN}/remote/base"),
            adopt: None,
        })
    };
    assert_eq!(base(), Ok(FetchOutcome::Missing));
    git(
        &rig.work,
        &["push", "-q", "origin", &format!("{a}:refs/heads/main")],
    );
    assert_eq!(base(), Ok(FetchOutcome::Fetched { sha: a.clone() }));
    assert_eq!(rig.local("main"), main_before);
    assert_ne!(main_before, a);
}
