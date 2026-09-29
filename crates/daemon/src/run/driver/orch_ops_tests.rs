//! Task M9.13: `ResolveTarget` runs git on `spawn_blocking` under one deadline, never
//! on the runtime's own threads; and the OTLP token's shape.

use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::*;

/// Each call of the stand-in `git` sleeps this long: under the run's per-command bound
/// (9 s), so a call alone succeeds, while a range's two calls pass the one 10 s
/// deadline.
const SLOW_GIT_SECS: u64 = 8;

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// On a current-thread runtime, a ticker task keeps running while the target is
/// resolved: a git call on that one thread would stop it. The slow range ends at the
/// one deadline with its text.
#[tokio::test(flavor = "current_thread")]
async fn resolve_target_op_runs_git_off_the_worker_threads() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "Test User"]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    std::fs::write(repo.join("a"), "a\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "one"]);
    let first = git(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(repo.join("b"), "b\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "two"]);
    let second = git(&repo, &["rev-parse", "HEAD"]);

    // A real git: a single revision is `merge-base(main, r)..r`.
    let real = std::ffi::OsString::from("git");
    let t = Duration::from_secs(5);
    let got = resolve_target(
        real.clone(),
        t,
        repo.clone(),
        "feature".into(),
        "main".into(),
    )
    .await;
    assert_eq!(
        got,
        OpResult::Target {
            base: first.clone(),
            head: second.clone()
        }
    );
    let got = resolve_target(real, t, repo.clone(), "nope".into(), "main".into()).await;
    let OpResult::Failed { message } = got else {
        panic!("{got:?}");
    };
    assert!(message.starts_with("nope: "), "{message}");

    // A slow git, with a ticker on the same (only) runtime thread.
    let slow = dir.path().join("slow-git.sh");
    std::fs::write(&slow, format!("#!/bin/sh\nsleep {SLOW_GIT_SECS}\n")).unwrap();
    std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
    let ticks = Arc::new(AtomicU64::new(0));
    let counter = ticks.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(100)).await;
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });
    let started = Instant::now();
    let got = resolve_target(
        slow.into_os_string(),
        Duration::from_secs(9),
        repo,
        format!("{first}..{second}"),
        "main".into(),
    )
    .await;
    let took = started.elapsed();
    ticker.abort();
    assert_eq!(
        got,
        OpResult::Failed {
            message: "git did not answer within 10 s".into()
        }
    );
    assert!(
        took >= DONE_CHECK_GIT_TIMEOUT && took < Duration::from_secs(2 * SLOW_GIT_SECS - 2),
        "{took:?}"
    );
    assert!(
        ticks.load(Ordering::SeqCst) >= 50,
        "the runtime thread was blocked"
    );
}

#[test]
fn otlp_tokens_are_32_lowercase_hex_and_fresh() {
    let a = fresh_token().unwrap();
    let b = fresh_token().unwrap();
    assert_eq!(a.len(), 32);
    assert!(
        a.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "{a}"
    );
    assert_ne!(a, b);
}
