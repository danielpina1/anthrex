//! Task M9.13: `ResolveTarget` runs git on `spawn_blocking` under one deadline, never
//! on the runtime's own threads; and the OTLP token's shape.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::*;
use crate::run::driver::gated_git::GatedGit;
use crate::run::driver::{DONE_CHECK_GIT_TIMEOUT, RunContext};

/// The held case's deadline for the op's git calls. Nothing else can end the op (each
/// call's own bound is [`NEVER`], and the stand-in holds its second call until the
/// test releases it), so this length is only the test's running time.
const DEADLINE: Duration = Duration::from_secs(1);
/// A per-call bound no test reaches.
const NEVER: Duration = Duration::from_secs(3600);
/// A hang guard on each wait; nothing asserts it.
const WAIT: Duration = Duration::from_secs(30);

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

/// The daemon reads with `DONE_CHECK_GIT_TIMEOUT` as both the deadline and each call's
/// cap: the seam the held tests use changes nothing outside them.
#[tokio::test]
async fn the_daemon_reads_git_with_the_done_check_budget() {
    let budget = GitBudget::DONE_CHECK;
    assert_eq!(budget.deadline, DONE_CHECK_GIT_TIMEOUT);
    assert_eq!(budget.each_cap, DONE_CHECK_GIT_TIMEOUT);
    assert_eq!(budget.each(Duration::from_secs(60)), DONE_CHECK_GIT_TIMEOUT);
    assert_eq!(budget.each(Duration::from_secs(5)), Duration::from_secs(5));
    let config =
        crate::manager::ManagerConfig::for_tests("/tmp/unused.sock".into(), "/bin/sh".into());
    let git = crate::server::GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let ctx = RunContext::new(
        "/tmp/unused".into(),
        &config,
        config::Orchestrator::default(),
        git.registry.clone(),
    );
    assert_eq!(ctx.read_git, GitBudget::DONE_CHECK);
}

/// On a current-thread runtime, a ticker task keeps running while git is held: a git
/// call on that one thread would stop it. The held range ends at the one deadline,
/// with its text: per-call bounds alone would never end it.
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

    // A real git, under the daemon's budget: a single revision is
    // `merge-base(main, r)..r`.
    let real = std::ffi::OsString::from("git");
    let t = Duration::from_secs(5);
    let budget = GitBudget::DONE_CHECK;
    let got = resolve_target(
        real.clone(),
        t,
        budget,
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
    let got = resolve_target(real, t, budget, repo.clone(), "nope".into(), "main".into()).await;
    let OpResult::Failed { message } = got else {
        panic!("{got:?}");
    };
    assert!(message.starts_with("nope: "), "{message}");

    // A held git: its first call (the range's base) answers, its second is held until
    // the test releases it. A ticker shares the (only) runtime thread.
    let gate = GatedGit::new(&dir.path().join("gate"), &first);
    let ticks = Arc::new(AtomicU64::new(0));
    let counter = ticks.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(10)).await;
            counter.fetch_add(1, Ordering::SeqCst);
        }
    });
    let held = GitBudget {
        deadline: DEADLINE,
        each_cap: NEVER,
    };
    let started = Instant::now();
    let op = tokio::spawn(resolve_target(
        gate.program(),
        NEVER,
        held,
        repo,
        format!("{first}..{second}"),
        "main".into(),
    ));
    // Git off the runtime thread, by observation: while the second call is held, the
    // ticker and this task both keep running on the one thread. Git on that thread
    // would stop both until the stand-in gave up (its own 60 s cap), and the call
    // would no longer be held.
    gate.wait_held(WAIT).await;
    let seen = ticks.load(Ordering::SeqCst);
    let deadline = Instant::now() + WAIT;
    while ticks.load(Ordering::SeqCst) < seen + 5 {
        assert!(Instant::now() < deadline, "the ticker stopped");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        gate.held(),
        "the runtime thread was blocked while git was held"
    );

    // The one deadline ends the op while its second call is still held.
    let got = tokio::time::timeout(WAIT, op)
        .await
        .expect("only the op's deadline can end it")
        .unwrap();
    let took = started.elapsed();
    ticker.abort();
    assert_eq!(
        got,
        OpResult::Failed {
            message: format!("git did not answer within {} s", DEADLINE.as_secs())
        }
    );
    assert!(took >= DEADLINE, "{took:?}");
    // The abandoned read is still held: nothing but the deadline answered the op.
    assert!(gate.held(), "the held call ended before its release");
    assert_eq!(gate.calls().len(), 2, "{:?}", gate.calls());
    gate.release();
    gate.wait_exited(WAIT).await;
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

/// M9.13 review, item 1: a restarted orchestrator's OTLP variables name the receiver
/// that is up now, with the run's token; stale ones are replaced, missing ones added,
/// and with no receiver none is kept. Every other variable is left as it was.
#[test]
fn a_restart_refreshes_the_otlp_variables() {
    let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
    let old = crate::launch::role::otlp_env("http://127.0.0.1:62405", "r1", "t0k");
    let mut env = vec![pair("ANTHREX_KEEP", "1")];
    env.extend(old.clone());
    env.push(pair("OTHER", "2"));
    let now = crate::launch::role::otlp_env("http://127.0.0.1:62406", "r1", "t0k");

    let got = refreshed_otlp_env(&env, Some(("http://127.0.0.1:62406", "r1", "t0k")));
    let mut want = vec![pair("ANTHREX_KEEP", "1"), pair("OTHER", "2")];
    want.extend(now.clone());
    assert_eq!(got, want);
    let endpoint = |env: &[(String, String)]| {
        env.iter()
            .filter(|(k, _)| k == "OTEL_EXPORTER_OTLP_ENDPOINT")
            .map(|(_, v)| v.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(endpoint(&got), vec!["http://127.0.0.1:62406".to_string()]);

    // Created while the receiver was down: added now.
    let bare = vec![pair("ANTHREX_KEEP", "1")];
    let got = refreshed_otlp_env(&bare, Some(("http://127.0.0.1:62406", "r1", "t0k")));
    let mut want = bare.clone();
    want.extend(now);
    assert_eq!(got, want);

    // No receiver now: nothing stale is kept.
    let got = refreshed_otlp_env(&env, None);
    assert_eq!(got, vec![pair("ANTHREX_KEEP", "1"), pair("OTHER", "2")]);
}
