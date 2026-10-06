//! Task M9.7.6 (DH §1.2, decision 6): the base fetch after a merge answers whether the
//! merge holds the stage's local head. Real git, a local bare repository as the remote,
//! the `tests_git.rs` rig; the user's side (commits on the stage branch, the merge) is a
//! second clone, so the merged head is never in the work repository before the fetch.

use std::path::{Path, PathBuf};

use super::allow::check;
use super::gh::run_ctx;
use super::tests_git::{NO_GH, RUN, Rig, commit, git};
use super::*;

/// The stage-1 branch, its private fetch ref, and the base's.
pub(super) fn stage_branch() -> String {
    format!("anthrex/{RUN}/stage-1")
}
pub(super) fn stage_into() -> String {
    format!("refs/anthrex/{RUN}/remote/stage-1")
}
pub(super) fn base_into() -> String {
    format!("refs/anthrex/{RUN}/remote/base")
}

/// The user's clone of the remote: where the commits on the stage branch and the merge
/// are made, and pushed from.
pub(super) fn user_clone(rig: &Rig) -> PathBuf {
    let user = rig.tmp.path().join("user");
    git(
        rig.tmp.path(),
        &["clone", "-q", &rig.bare.to_string_lossy(), "user"],
    );
    user
}

/// The common start: `main` at `a` on the remote, anthrex's stage branch pushed at `h2`
/// (the local head). Returns `(a, h2)`.
pub(super) fn start(rig: &Rig) -> (String, String) {
    let a = commit(&rig.work, "a");
    git(&rig.work, &["push", "-q", "origin", "main"]);
    git(&rig.work, &["checkout", "-q", "-b", &stage_branch()]);
    commit(&rig.work, "h1");
    let h2 = commit(&rig.work, "h2 (anthrex's fix)");
    rig.push(1, &h2).unwrap();
    git(&rig.work, &["checkout", "-q", "main"]);
    (a, h2)
}

/// On the user's side: two commits on top of the remote stage branch, pushed. Returns
/// the new tip `u2`.
pub(super) fn user_pushes_on_the_stage(user: &Path) -> String {
    let remote = format!("origin/{}", stage_branch());
    git(user, &["fetch", "-q", "origin"]);
    git(user, &["checkout", "-q", "-B", "s", &remote]);
    commit(user, "u1");
    let u2 = commit(user, "u2");
    git(
        user,
        &[
            "push",
            "-q",
            "origin",
            &format!("HEAD:refs/heads/{}", stage_branch()),
        ],
    );
    u2
}

/// The user's squash merge of branch `s` into `main`, pushed: one parent.
pub(super) fn squash(user: &Path) -> String {
    git(user, &["checkout", "-q", "-B", "main", "origin/main"]);
    git(user, &["merge", "-q", "--squash", "s"]);
    let s = commit(user, "squash of stage 1");
    git(user, &["push", "-q", "origin", "main"]);
    s
}

pub(super) fn ask(head: &str, merged: &str) -> Contains {
    Contains {
        stage: 1,
        branch: stage_branch(),
        into: stage_into(),
        head: head.to_string(),
        merged: merged.to_string(),
        pr: None,
    }
}

pub(super) fn base_fetch(rig: &Rig, parents_of: &str, contains: Contains) -> FetchReq {
    FetchReq {
        repo: rig.repo.clone(),
        run_id: RUN.to_string(),
        branch: "main".to_string(),
        into: base_into(),
        adopt: None,
        parents_of: Some(parents_of.to_string()),
        contains: Some(contains),
        deadline: None,
    }
}

pub(super) fn fetched(
    sha: &str,
    parents: u32,
    contains: Option<bool>,
) -> Result<FetchOutcome, HostError> {
    Ok(FetchOutcome::Fetched {
        sha: sha.to_string(),
        parents: Some(parents),
        contains,
    })
}

pub(super) fn has_ref(dir: &Path, refname: &str) -> bool {
    !git(dir, &["for-each-ref", "--format=%(refname)", refname]).is_empty()
}

#[test]
fn a_squash_merge_fetches_the_stage_branch_and_finds_the_head() {
    let rig = Rig::new();
    let (_, h2) = start(&rig);
    let user = user_clone(&rig);
    let u2 = user_pushes_on_the_stage(&user);
    let s = squash(&user);

    let answer = rig.fetch(&base_fetch(&rig, &s, ask(&h2, &u2)));
    assert_eq!(answer, fetched(&s, 1, Some(true)));
    assert_eq!(git(&rig.work, &["rev-parse", &stage_into()]), u2);
    assert_eq!(git(&rig.work, &["rev-parse", &base_into()]), s);
}

#[test]
fn a_merge_commit_skips_the_branch_fetch() {
    let rig = Rig::new();
    let (_, h2) = start(&rig);
    let user = user_clone(&rig);
    let u2 = user_pushes_on_the_stage(&user);
    git(&user, &["checkout", "-q", "-B", "main", "origin/main"]);
    git(
        &user,
        &["merge", "-q", "--no-ff", "-m", "merge stage 1", "s"],
    );
    let m = git(&user, &["rev-parse", "HEAD"]);
    git(&user, &["push", "-q", "origin", "main"]);

    let recorder = Recorder::new(&rig);
    let answer = recorder.host.fetch(&base_fetch(&rig, &m, ask(&h2, &u2)));
    assert_eq!(answer, fetched(&m, 2, Some(true)));
    let calls = recorder.calls();
    let fetches = calls.iter().filter(|argv| sub(argv) == "fetch");
    assert_eq!(fetches.count(), 1, "only the base fetch: m^2 came with it");
    assert!(!has_ref(&rig.work, &stage_into()));
}

#[test]
fn a_merged_head_without_the_local_head_is_false() {
    let rig = Rig::new();
    let (a, h2) = start(&rig);
    let user = user_clone(&rig);
    // The user rewrote the stage branch from the base, without anthrex's fix.
    git(&user, &["checkout", "-q", "-B", "s", &a]);
    let u2 = commit(&user, "u2 (rewritten)");
    git(
        &user,
        &[
            "push",
            "-q",
            "-f",
            "origin",
            &format!("HEAD:refs/heads/{}", stage_branch()),
        ],
    );
    let s = squash(&user);

    let answer = rig.fetch(&base_fetch(&rig, &s, ask(&h2, &u2)));
    assert_eq!(answer, fetched(&s, 1, Some(false)));
}

#[test]
fn a_deleted_stage_branch_is_none() {
    let rig = Rig::new();
    let (_, h2) = start(&rig);
    let user = user_clone(&rig);
    let u2 = user_pushes_on_the_stage(&user);
    let s = squash(&user);
    // The branch fetched, but the merged head is not in the repository: `merge-base`
    // fails with exit 128 and a message, which is unknown, never `false`.
    let unknown = "e".repeat(40);
    let answer = rig.fetch(&base_fetch(&rig, &s, ask(&h2, &unknown)));
    assert_eq!(answer, fetched(&s, 1, None));
    git(
        &user,
        &[
            "push",
            "-q",
            "origin",
            "--delete",
            &format!("refs/heads/{}", stage_branch()),
        ],
    );

    let answer = rig.fetch(&base_fetch(&rig, &s, ask(&h2, &u2)));
    assert_eq!(answer, fetched(&s, 1, None), "the base fetch still answers");
    assert_eq!(git(&rig.work, &["rev-parse", &base_into()]), s);
}

/// Controller addendum (ruling R1): a base fetch that fails is the op's failure, never
/// an answer, so the engine counts it as a failed check and asks again.
#[test]
fn a_failed_base_fetch_is_the_ops_failure() {
    let rig = Rig::new();
    let (_, h2) = start(&rig);
    let user = user_clone(&rig);
    let u2 = user_pushes_on_the_stage(&user);
    let s = squash(&user);
    std::fs::rename(&rig.bare, rig.tmp.path().join("gone.git")).unwrap();

    let answer = rig.host.fetch(&base_fetch(&rig, &s, ask(&h2, &u2)));
    assert!(answer.is_err(), "{answer:?}");
}

/// The stage fetch writes only `refs/anthrex/<run>/remote/stage-<n>` of the asked stage,
/// from that stage's branch: a question naming any other ref is not asked (no git runs
/// for it) and answers `None`.
#[test]
fn a_question_about_another_ref_runs_no_git() {
    let rig = Rig::new();
    let (_, h2) = start(&rig);
    let user = user_clone(&rig);
    let u2 = user_pushes_on_the_stage(&user);
    let s = squash(&user);
    let recorder = Recorder::new(&rig);
    let wrong = [
        Contains {
            into: base_into(),
            ..ask(&h2, &u2)
        },
        Contains {
            into: format!("refs/anthrex/{RUN}/remote/stage-2"),
            ..ask(&h2, &u2)
        },
        Contains {
            branch: "main".to_string(),
            ..ask(&h2, &u2)
        },
        Contains {
            stage: 2,
            ..ask(&h2, &u2)
        },
        Contains {
            head: "HEAD".to_string(),
            ..ask(&h2, &u2)
        },
    ];
    for c in wrong {
        let answer = recorder.host.fetch(&base_fetch(&rig, &s, c.clone()));
        assert_eq!(answer, fetched(&s, 1, None), "{c:?}");
        let calls = recorder.take();
        let subcommands: Vec<&str> = calls.iter().map(|argv| sub(argv)).collect();
        assert_eq!(subcommands, ["fetch", "rev-parse", "rev-list"], "{c:?}");
    }
    assert!(!has_ref(&rig.work, &stage_into()));
}

#[test]
fn every_call_is_allowed_and_scrubbed() {
    let rig = Rig::new();
    let (_, h2) = start(&rig);
    let user = user_clone(&rig);
    let u2 = user_pushes_on_the_stage(&user);
    let s = squash(&user);

    let recorder = Recorder::new(&rig);
    let answer = recorder.host.fetch(&base_fetch(&rig, &s, ask(&h2, &u2)));
    assert_eq!(answer, fetched(&s, 1, Some(true)));

    let calls = recorder.calls();
    let subcommands: Vec<&str> = calls.iter().map(|argv| sub(argv)).collect();
    assert_eq!(
        subcommands,
        ["fetch", "rev-parse", "rev-list", "fetch", "merge-base"]
    );
    let ctx = run_ctx(&rig.repo, Some(RUN));
    for argv in &calls {
        assert_eq!(argv[0], "-C", "{argv:?}");
        assert_eq!(argv[2], "--no-optional-locks", "{argv:?}");
        // `run_git` adds `-C <dir> --no-optional-locks` and `NO_FSMONITOR`; the rest is
        // `GhHost::git`'s, which the allow-list checks.
        assert_eq!(argv[3..5], crate::worktree::NO_FSMONITOR, "{argv:?}");
        assert_eq!(check(Program::Git, &argv[5..], &ctx), Ok(()), "{argv:?}");
    }
    let stage_fetch = &calls[3];
    assert!(
        stage_fetch.ends_with(&[
            "origin".to_string(),
            format!("+refs/heads/{}:{}", stage_branch(), stage_into())
        ]),
        "{stage_fetch:?}"
    );
    let env = std::fs::read_to_string(&recorder.env_log).unwrap();
    for scrubbed in [
        "GIT_DIR=",
        "GIT_WORK_TREE=",
        "GIT_COMMON_DIR=",
        "GIT_INDEX_FILE=",
        "GIT_PREFIX=",
    ] {
        assert!(
            !env.lines().any(|line| line.starts_with(scrubbed)),
            "{scrubbed} reached git"
        );
    }
    // That `GIT_DIR` set in the daemon's own environment has no effect is
    // `crates/daemon/tests/host_contains_env.rs`: it changes the process environment,
    // so it is alone in its own test binary (AGENTS.md, worktree_env.rs).
}

/// A recorded call's git subcommand: the first word after `-C <dir>
/// --no-optional-locks` and `GhHost::git`'s `-c <k=v>` pairs.
pub(super) fn sub(argv: &[String]) -> &str {
    let mut at = 3;
    while argv.get(at).map(String::as_str) == Some("-c") {
        at += 2;
    }
    argv.get(at).map_or("", String::as_str)
}

/// A `GhHost` whose `git` is a stand-in that records each call's argv (one argument per
/// line, then a separator) and its `GIT_*` environment, then runs the real git.
pub(super) struct Recorder {
    pub(super) host: GhHost<SystemRunner>,
    argv_log: PathBuf,
    env_log: PathBuf,
}

const END: &str = "--anthrex-test-end--";

impl Recorder {
    pub(super) fn new(rig: &Rig) -> Recorder {
        let dir = rig.tmp.path().join("recorder");
        std::fs::create_dir_all(&dir).unwrap();
        let (argv_log, env_log) = (dir.join("argv.log"), dir.join("env.log"));
        let script = dir.join("git");
        let body = format!(
            "for a in \"$@\"; do printf '%s\\n' \"$a\" >> '{argv}'; done\nprintf '%s\\n' '{END}' >> '{argv}'\nenv | grep '^GIT_' >> '{env}'\nexec git \"$@\"\n",
            argv = argv_log.display(),
            env = env_log.display(),
        );
        install_stand_in(&script, &body);
        assert!(!argv_log.exists(), "the warm-up ran only the guard");
        Recorder {
            host: GhHost::new(SystemRunner::new(NO_GH, &script), NO_GH, &script),
            argv_log,
            env_log,
        }
    }

    /// Every recorded call so far, `-C <dir> --no-optional-locks …` each.
    pub(super) fn calls(&self) -> Vec<Vec<String>> {
        let text = std::fs::read_to_string(&self.argv_log).unwrap_or_default();
        let mut calls = vec![Vec::new()];
        for line in text.lines() {
            if line == END {
                calls.push(Vec::new());
            } else {
                calls.last_mut().unwrap().push(line.to_string());
            }
        }
        calls.pop();
        calls
    }

    /// The recorded calls, and the log emptied.
    fn take(&self) -> Vec<Vec<String>> {
        let calls = self.calls();
        let _ = std::fs::remove_file(&self.argv_log);
        calls
    }
}

/// FW-12 (task 6's Minor): a question with no merge commit to count (`parents_of`
/// unknown) fetches the stage branch, as a squash does.
#[test]
fn a_question_without_parents_of_fetches_the_stage_branch() {
    let rig = Rig::new();
    let (_, h2) = start(&rig);
    let user = user_clone(&rig);
    let u2 = user_pushes_on_the_stage(&user);
    let s = squash(&user);
    let req = FetchReq {
        parents_of: None,
        ..base_fetch(&rig, &s, ask(&h2, &u2))
    };
    let recorder = Recorder::new(&rig);
    let answer = recorder.host.fetch(&req);
    let want = FetchOutcome::Fetched {
        sha: s,
        parents: None,
        contains: Some(true),
    };
    assert_eq!(answer, Ok(want));
    assert_eq!(git(&rig.work, &["rev-parse", &stage_into()]), u2);
    // The stage branch first, then one `merge-base`: nothing was skipped.
    let calls = recorder.calls();
    let subcommands: Vec<&str> = calls.iter().map(|argv| sub(argv)).collect();
    assert_eq!(subcommands, ["fetch", "rev-parse", "fetch", "merge-base"]);
}

/// FW-18: writes `body` (after a warm-up guard) as the executable `script`, under
/// another name first and renamed into place, then runs it once with the guard set,
/// retrying while a concurrent fork still holds a writable copy of its descriptor
/// (`ETXTBSY`); so the test's own exec is never the first, and never busy
/// (`driver/delivery_tests_start.rs::decider_stand_in`).
fn install_stand_in(script: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let staged = script.with_extension("new");
    let text = format!("#!/bin/sh\n[ -n \"$ANTHREX_TEST_WARM_UP\" ] && exit 0\n{body}");
    std::fs::write(&staged, text).unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, script).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut child = loop {
        match std::process::Command::new(script)
            .env("ANTHREX_TEST_WARM_UP", "1")
            .stdin(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => break child,
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the stand-in stayed busy: {e}"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => panic!("the stand-in did not start: {e}"),
        }
    };
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the stand-in's warm-up never exited"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(status.success(), "warm-up: {status:?}");
}
