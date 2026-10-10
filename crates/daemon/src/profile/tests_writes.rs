//! Milestone 9.11 decision 35 (B11): profile writes are ordered per repository. A
//! `profile reject` whose checkout discard stalls in git under repository A holds A's
//! writes, and B's `profile confirm` does not wait for it. A real service on a scratch
//! data directory and real git repositories; the only stand-in is a `git` wrapper (the
//! service's `ctx.git`) that stalls one call under A until the test releases it. No
//! agent runs (Claude and Codex are paths that do not exist).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use proto::{ProfileReply, ProfileRequest, ProposalState};

use super::service::{ProfileContext, ProfileService};
use super::store;
use super::tests_ready::{record, repo, service};

/// How long B's confirm may take: one `git rev-parse` (`project_of`) and a few small
/// file writes on `spawn_blocking`; no agent. This is the CLI harness's `REQUEST_WAIT`
/// (`cli/tests/support/run_harness.rs`), as `tests_queue_service.rs` uses it; only a
/// failure waits this long. Before the fix B waits behind A's stalled git call, which
/// ends only at the Rig's `git_timeout_secs` (60 s, `config::Orchestrator::default`),
/// so the test fails then: B answers after A's reject has finished.
const REQUEST_WAIT: Duration = Duration::from_secs(75);
/// Between two looks.
const POLL: Duration = Duration::from_millis(20);

/// A `git` that, for the `rev-parse --git-common-dir` the discard of A's leftover
/// checkout runs under A's writes (`verify::pin_leftover`), touches `started` and waits
/// for `release`; every other call (`project_of`'s `--show-toplevel` included) is the
/// real git. The wait is the injected stall, not a synchronisation.
fn stalling_git(dir: &Path, a: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let (started, release) = (dir.join("started"), dir.join("release"));
    let script = format!(
        "#!/bin/sh\n\
         case \"$*\" in *--show-toplevel*) ;; *--git-common-dir*)\n\
           case \"$*|$PWD\" in *'{a}'*)\n\
             : > '{started}'\n\
             while [ ! -e '{release}' ]; do sleep 0.05; done ;;\n\
           esac ;;\n\
         esac\n\
         exec git \"$@\"\n",
        a = a.display(),
        started = started.display(),
        release = release.display(),
    );
    let git = testexec::write_executable(dir.join("git-stall"), script);
    (git, started, release)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_slow_reject_in_one_repository_never_delays_another() {
    let tmp = tempfile::tempdir().unwrap();
    let a = repo(tmp.path(), "a");
    let b = repo(tmp.path(), "b");
    let (git, started, release) = stalling_git(tmp.path(), &a);
    // The service as the daemon wires it, then the same parts with the stalling git.
    let wired = service(tmp.path());
    let ctx = &wired.ctx;
    let profiles = ProfileService::new(
        wired.scouts.clone(),
        wired.manager.clone(),
        ProfileContext {
            data_dir: ctx.data_dir.clone(),
            worktrees_root: ctx.worktrees_root.clone(),
            git: git.into_os_string(),
            orchestrator: ctx.orchestrator.clone(),
            git_queue: ctx.git_queue.clone(),
            cli_caps: ctx.cli_caps,
            daemon_socket: ctx.daemon_socket.clone(),
            scheduler: ctx.scheduler.clone(),
        },
    );

    // A: a constructed leftover detection checkout whose repository has a `HEAD`, so its
    // discard asks git for A's common directory (and stalls there). Its `HEAD` is not a
    // detached commit, so once released the discard refuses it as tampered: this test is
    // about the order of the two requests, not the discard's outcome.
    let (checkout, checkout_repo) = profiles.checkout(&a, super::ONBOARDING_CHECKOUT);
    std::fs::create_dir_all(&checkout).unwrap();
    let git_dir = crate::run::git::Repo::at(&checkout_repo).git_dir();
    std::fs::create_dir_all(&git_dir).unwrap();
    std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

    // B: a hand-written `Ready` proposal (its `updated_at` differs from any A value).
    let ready = record(&b, ProposalState::Ready, 1_790_000_123);
    let b_dir = super::repo_dir(&ctx.data_dir, &b);
    std::fs::create_dir_all(&b_dir).unwrap();
    store::save_proposal(&b_dir, &ready).unwrap();
    profiles.note_proposal(&b, Some(&ready));

    let reject = tokio::spawn({
        let (profiles, dir) = (profiles.clone(), a.clone());
        async move { profiles.request(ProfileRequest::Reject { dir }).await }
    });
    // A reaches the stall after `project_of` (the stand-in's first run: on macOS the
    // first exec of a freshly written script was measured at up to 10 s) and a few small
    // file operations; the same `REQUEST_WAIT` bounds it.
    let deadline = Instant::now() + REQUEST_WAIT;
    while !started.exists() {
        assert!(Instant::now() < deadline, "A's discard never reached git");
        assert!(!reject.is_finished(), "{:?}", reject.await);
        tokio::time::sleep(POLL).await;
    }

    let confirm = ProfileRequest::Confirm {
        dir: b.clone(),
        shown: None,
    };
    let reply = tokio::time::timeout(REQUEST_WAIT, profiles.request(confirm))
        .await
        .expect("B's confirm waited behind A's reject");
    assert!(matches!(reply, ProfileReply::Done { .. }), "{reply:?}");
    assert!(
        !reject.is_finished(),
        "A's reject ended before B answered, so B may have waited for it"
    );
    assert!(
        store::load_proposal(&b_dir).unwrap().is_none(),
        "B confirmed"
    );

    std::fs::write(&release, "").unwrap();
    // Released, A's reject ends (refused, see above) and B's answer did not wait for it.
    let rejected = tokio::time::timeout(REQUEST_WAIT, reject)
        .await
        .expect("A's reject ends once released")
        .unwrap();
    assert!(
        matches!(&rejected, ProfileReply::Refused { message } if message.contains("tampered")),
        "{rejected:?}"
    );
}
