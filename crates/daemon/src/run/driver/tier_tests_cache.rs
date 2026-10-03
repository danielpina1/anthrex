//! Milestone 9.1 task M9.1.10: the result cache in the tier executor (decisions 30 and
//! 31), on the rig of `tier_tests.rs`: what a job stores, and what a hit skips. The
//! scripts' log (`$TIER_LOG`) shows every command that ran.

use super::*;
use crate::run::test_cache::{CACHE_FILE, CacheLine, TestCache};
use crate::run::tiers::CacheCtx;

fn cache_ctx(toolchain: &str) -> CacheCtx {
    CacheCtx {
        profile_hash: "00000000000000aa".to_string(),
        toolchain: toolchain.to_string(),
    }
}

/// A tiered spec whose job reads and writes the cache.
fn cached(rig: &Rig) -> TierSpec {
    TierSpec {
        cache: Some(cache_ctx("none")),
        ..rig.spec(tiered(), None)
    }
}

/// The cache file's lines for `spec`'s repository.
fn stored(spec: &TierSpec) -> Vec<CacheLine> {
    std::fs::read_to_string(spec.repo_dir.join(CACHE_FILE))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn commands(lines: &[CacheLine]) -> Vec<String> {
    lines.iter().map(|l| l.key.command.clone()).collect()
}

const BUILD_CMD: &str = "sh build.sh";
const TEST_B: &str = "sh test.sh 'b' ";
const TEST_C: &str = "sh test.sh 'c' ";

#[tokio::test]
async fn a_second_job_on_the_same_tree_runs_nothing() {
    let rig = Rig::new();
    let cache = TestCache::new(14);
    let spec = cached(&rig);
    let first = rig.tier_with(1, &spec, &cache).await;
    assert!(first.ok, "{first:?}");
    assert!(first.steps.iter().all(|s| !s.cached));
    // The final fix wave: what ran, so a short store below says whether the steps were
    // planned (`graph_note`) and ran (`steps`), or ran and were not stored.
    assert_eq!(first.graph_note, None, "{first:#?}");
    let lines = stored(&spec);
    assert_eq!(commands(&lines), [BUILD_CMD, TEST_B, TEST_C], "{first:#?}");
    for line in &lines {
        assert_eq!(line.key.tree, first.tree);
        assert_eq!(line.key.profile_hash, "00000000000000aa");
        assert!(line.key.toolchain.starts_with("none path:"), "{line:?}");
        assert!(line.entry.ok);
        assert_eq!((line.entry.run.as_str(), line.entry.tier), ("r1", 1));
    }
    assert_eq!(lines[0].key.affected, "-", "a build step ignores the set");
    assert_eq!(lines[1].key.affected, "b,c");
    let ran = rig.log().len();
    assert_eq!(ran, 3);

    // Decision 31: every step looks itself up first; a hit is not run.
    let second = rig.tier_with(2, &spec, &cache).await;
    assert!(second.ok, "{second:?}");
    assert_eq!(second.steps.len(), 3);
    for step in &second.steps {
        assert!(step.cached && step.ok && !step.retried, "{step:?}");
        assert_eq!((step.secs, step.granted), (0, 0));
    }
    assert_eq!(second.secs, 0);
    assert_eq!(rig.log().len(), ran, "no command ran: {:#?}", rig.log());
    assert_eq!(stored(&spec).len(), 3, "a hit writes nothing");

    // A new daemon reads the file: still a hit.
    let third = rig.tier_with(3, &spec, &TestCache::new(14)).await;
    assert!(third.steps.iter().all(|s| s.cached));
    assert_eq!(rig.log().len(), ran);
}

#[tokio::test]
async fn red_and_timed_out_steps_are_never_cached() {
    // A red step (its by-name retry fails): the green build before it is stored, the
    // red step is not, and the next job runs both again (ruling C-13 (2): a build hit
    // counts only when every later step hits).
    let rig = Rig::new();
    let cache = TestCache::new(14);
    let spec = cached(&rig);
    rig.state("out-b-1", &libtest(&["tests::one"]));
    rig.state("code-b-1", "101");
    rig.state("one-fail", "");
    let red = rig.tier_with(1, &spec, &cache).await;
    assert!(!red.ok);
    assert_eq!(commands(&stored(&spec)), [BUILD_CMD]);
    let again = rig.tier_with(2, &spec, &cache).await;
    assert!(!again.steps[0].cached, "the build runs again: {again:?}");
    assert!(!again.steps[1].cached, "the red step runs again");
    assert_eq!(
        rig.log().iter().filter(|l| l.starts_with("build ")).count(),
        2,
        "{:#?}",
        rig.log()
    );
    assert_eq!(
        rig.log()
            .iter()
            .filter(|l| l.starts_with("test b "))
            .count(),
        2,
        "{:#?}",
        rig.log()
    );

    // Ruling C-7: the by-name retry passes but the whole step still fails. Red: not
    // stored.
    let rig = Rig::new();
    let spec = cached(&rig);
    let named = format!(
        "{}error: the lint step failed too\n",
        libtest(&["tests::a"])
    );
    for n in 1..=2 {
        rig.state(&format!("out-b-{n}"), &named);
        rig.state(&format!("code-b-{n}"), "1");
    }
    let outcome = rig.tier_with(1, &spec, &TestCache::new(14)).await;
    assert!(!outcome.ok);
    assert_eq!(commands(&stored(&spec)), [BUILD_CMD]);

    // A timed-out step.
    let rig = Rig::new();
    let mut spec = cached(&rig);
    spec.timeout_secs = 1;
    rig.state("sleep-b-1", "");
    let outcome = rig.tier_with(1, &spec, &TestCache::new(14)).await;
    assert!(outcome.steps.last().unwrap().timed_out);
    assert_eq!(commands(&stored(&spec)), [BUILD_CMD]);

    // A step that passed on its retry is green, and is stored with its flaky name; a
    // later hit reports no flake (it was recorded when it happened).
    let rig = Rig::new();
    let cache = TestCache::new(14);
    let spec = cached(&rig);
    rig.state("out-b-1", &libtest(&["tests::flaky_one"]));
    rig.state("code-b-1", "101");
    rig.state("out-b-2", "test result: ok. 2 passed\n");
    let outcome = rig.tier_with(1, &spec, &cache).await;
    assert!(outcome.ok, "{outcome:?}");
    let lines = stored(&spec);
    assert_eq!(commands(&lines), [BUILD_CMD, TEST_B, TEST_C]);
    assert_eq!(lines[1].entry.flaky, ["tests::flaky_one"]);
    assert!(lines[2].entry.flaky.is_empty());
    assert_eq!(
        lines[1].entry.tail, "test result: ok. 2 passed",
        "the green run's tail"
    );
    let hit = rig.tier_with(2, &spec, &cache).await;
    assert!(
        hit.steps[1].cached && hit.steps[1].flaky.is_empty(),
        "{hit:?}"
    );
}

#[tokio::test]
async fn tier1_and_tier2_share_a_key_for_the_same_tree_and_scope() {
    // Decision 30 / defect 3: a candidate on an unmoved stage head has the task's tree,
    // which tier 1 already passed.
    let rig = Rig::new();
    let cache = TestCache::new(14);
    let tier1 = cached(&rig);
    assert!(rig.tier_with(1, &tier1, &cache).await.ok);
    let ran = rig.log().len();
    let tier2 = TierSpec {
        tier: 2,
        scratch: None,
        priority: Priority::Candidate,
        ..tier1.clone()
    };
    let outcome = rig.tier_with(2, &tier2, &cache).await;
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(outcome.scope, Scope::Gate);
    assert_eq!(outcome.steps.len(), 3);
    assert!(outcome.steps.iter().all(|s| s.cached), "{outcome:?}");
    assert_eq!(rig.log().len(), ran, "tier 2 ran nothing");

    // Tier 3's scope is `full`: its key differs, so it runs.
    let tier3 = TierSpec {
        tier: 3,
        check: Some("sh check.sh".to_string()),
        ..tier1.clone()
    };
    let full = rig.tier_with(3, &tier3, &cache).await;
    assert!(full.steps.iter().all(|s| !s.cached), "{full:?}");
    assert!(rig.log().len() > ran);
    let lines = stored(&tier1);
    let last = lines.last().unwrap();
    assert_eq!(last.key.scope, Scope::Full);
    assert_eq!(last.key.affected, "full:full suite");
}

#[tokio::test]
async fn untiered_and_unknown_toolchain_jobs_neither_read_nor_write() {
    // Decision 30: an untiered profile never touches the cache, even with a context.
    let rig = Rig::new();
    let cache = TestCache::new(14);
    let untiered = TierSpec {
        cache: Some(cache_ctx("none")),
        ..rig.spec(TierProfile::default(), Some("sh check.sh"))
    };
    for op in 1..=2 {
        assert!(rig.tier_with(op, &untiered, &cache).await.ok);
    }
    assert_eq!(rig.log().len(), 2, "run both times");
    assert!(stored(&untiered).is_empty());

    // An "unknown" toolchain in the context.
    let rig = Rig::new();
    let spec = TierSpec {
        cache: Some(cache_ctx("unknown")),
        ..rig.spec(tiered(), None)
    };
    let cache = TestCache::new(14);
    rig.tier_with(1, &spec, &cache).await;
    rig.tier_with(2, &spec, &cache).await;
    assert_eq!(rig.log().len(), 6);
    assert!(stored(&spec).is_empty());

    // A job that runs `toolchain_id` itself (decision 11) keys on what it printed, or
    // stores nothing when it failed.
    let rig = Rig::new();
    let cache = TestCache::new(14);
    let spec = |toolchain_id: &str| TierSpec {
        toolchain: None,
        cache: Some(cache_ctx("pending")),
        profile: TierProfile {
            toolchain_id: Some(toolchain_id.to_string()),
            ..tiered()
        },
        ..rig.spec(tiered(), None)
    };
    let failing = spec("exit 3");
    let outcome = rig.tier_with(1, &failing, &cache).await;
    assert_eq!(outcome.toolchain.as_deref(), Some("unknown"));
    assert!(stored(&failing).is_empty());
    let known = spec("echo rustc 1.92.0");
    let outcome = rig.tier_with(2, &known, &cache).await;
    let id = outcome.toolchain.clone().unwrap();
    assert_ne!(id, "unknown");
    let lines = stored(&known);
    assert_eq!(lines.len(), 3);
    let with_path = format!("{id} path:");
    assert!(
        lines
            .iter()
            .all(|l| l.key.toolchain.starts_with(&with_path)),
        "{id}"
    );
}

/// The build lines and the `test <m>` lines of the log.
fn builds_and_tests(rig: &Rig) -> (usize, usize) {
    let log = rig.log();
    (
        log.iter().filter(|l| l.starts_with("build ")).count(),
        log.iter().filter(|l| l.starts_with("test ")).count(),
    )
}

#[tokio::test]
async fn a_cached_build_runs_again_when_a_later_step_misses() {
    // Ruling C-13 (2), amending decision 31: the steps are not independent; a test step
    // may need what the build step made, so a build hit counts only when every later
    // step of the job also hits.
    let rig = Rig::new();
    let cache = TestCache::new(14);
    let spec = cached(&rig);
    let first = rig.tier_with(1, &spec, &cache).await;
    assert!(first.ok, "{first:?}");
    // The final fix wave: the job's premise, said first, so a failure names its cause
    // (a graph the job could not read plans tier 1 as the build alone).
    assert_eq!(first.graph_note, None, "{first:#?}");
    assert_eq!(builds_and_tests(&rig), (1, 2), "{first:#?}");
    // The same build command; the test steps' commands (and so their keys) differ.
    let other = TierSpec {
        profile: TierProfile {
            module_test: Some("sh test.sh {module} {filter:--filter %} x".to_string()),
            ..tiered()
        },
        ..spec.clone()
    };
    let outcome = rig.tier_with(2, &other, &cache).await;
    assert!(outcome.ok, "{outcome:?}");
    assert!(outcome.steps.iter().all(|s| !s.cached), "{outcome:?}");
    assert_eq!(builds_and_tests(&rig), (2, 4), "{:#?}", rig.log());
    // Both are stored now, so a third job hits all three.
    let third = rig.tier_with(3, &other, &cache).await;
    assert!(third.steps.iter().all(|s| s.cached), "{third:?}");
    assert_eq!(builds_and_tests(&rig), (2, 4));
}

#[tokio::test]
async fn a_checkout_at_another_tree_stores_nothing() {
    // Ruling C-13 (1): the key's tree is the judged head's, but the steps ran in the
    // checkout; a result is stored only when that checkout is at the key's tree and
    // clean.
    let rig = Rig::new();
    let cache = TestCache::new(14);
    let mut spec = cached(&rig);
    if let Some(scratch) = spec.scratch.as_mut() {
        scratch.commit = rig.base.clone();
    }
    let outcome = rig.tier_with(1, &spec, &cache).await;
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(git(&rig.dir, &["rev-parse", "HEAD"]), rig.base);
    assert!(stored(&spec).is_empty(), "{:?}", commands(&stored(&spec)));

    // A checkout at the key's tree with a tracked file changed stores nothing either:
    // a step that edits a tracked file leaves it dirty.
    let rig = Rig::new();
    let spec = TierSpec {
        profile: TierProfile {
            build_check: Some("sh build.sh && echo dirty >> mods/a/lib.txt".to_string()),
            ..tiered()
        },
        ..cached(&rig)
    };
    let outcome = rig.tier_with(1, &spec, &TestCache::new(14)).await;
    assert!(outcome.ok, "{outcome:?}");
    assert!(stored(&spec).is_empty(), "{:?}", commands(&stored(&spec)));
}

#[tokio::test]
async fn a_changed_path_misses() {
    // Ruling C-13 (5): the effective PATH is part of the key's toolchain.
    let rig = Rig::new();
    let cache = TestCache::new(14);
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string());
    let with_path = |path: &str| {
        let mut spec = cached(&rig);
        spec.env.push(("PATH".to_string(), path.to_string()));
        spec
    };
    assert!(rig.tier_with(1, &with_path(&path), &cache).await.ok);
    let ran = rig.log().len();
    let longer = format!("{path}:/nonexistent/extra");
    let moved = rig.tier_with(2, &with_path(&longer), &cache).await;
    assert!(moved.steps.iter().all(|s| !s.cached), "{moved:?}");
    assert_eq!(rig.log().len(), ran * 2);
    let same = rig.tier_with(3, &with_path(&path), &cache).await;
    assert!(same.steps.iter().all(|s| s.cached), "{same:?}");
}
