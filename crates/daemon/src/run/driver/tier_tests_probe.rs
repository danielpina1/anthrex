//! Milestone 9.1 task M9.1.15: a bisect probe (decision 36) on the rig of
//! `tier_tests.rs`: red only when a command fails twice, and a merge's `show --stat`.

use super::*;

/// A `--no-ff` merge of a task branch onto `main` whose `probe.sh` always fails, as a
/// stage's merge of a task is; its commit.
fn red_merge(rig: &Rig) -> String {
    git(&rig.root, &["checkout", "-q", "-b", "t4"]);
    write(
        &rig.root.join("probe.sh"),
        "echo \"probe $*\" >> \"$TIER_LOG\"\nexit 1\n",
    );
    git(&rig.root, &["add", "probe.sh"]);
    git(&rig.root, &["commit", "-q", "-m", "t4's work"]);
    git(&rig.root, &["checkout", "-q", "main"]);
    git(
        &rig.root,
        &["merge", "-q", "--no-ff", "-m", "anthrex: merge t4", "t4"],
    );
    git(&rig.root, &["rev-parse", "HEAD"])
}

fn probe(rig: &Rig, commit: &str, commands: &[&str]) -> TestAtSpec {
    TestAtSpec {
        root: rig.root.clone(),
        dir: rig.dir.with_file_name(".full"),
        commit: commit.to_string(),
        setup: None,
        commands: commands.iter().map(|c| c.to_string()).collect(),
        timeout_secs: 60,
        env: rig.spec(tiered(), None).env,
    }
}

#[tokio::test]
async fn probe_is_red_only_when_it_fails_twice() {
    let rig = Rig::new();
    let merge = red_merge(&rig);
    let (sched, queue) = (TestScheduler::new(2), GitQueue::new());
    let ctx = rig.ctx();
    let program = OsStr::new("git");

    // Red twice: red, and the merge's stat against the stage's own line.
    let spec = probe(&rig, &merge, &["sh probe.sh x::a", "sh probe.sh x::b"]);
    let result = run_test_at(&ctx, &sched, &queue, program, 11, &spec).await;
    let OpResult::TestAt {
        red, failing, show, ..
    } = result
    else {
        panic!("{result:?}");
    };
    assert!(red);
    assert_eq!(failing, ["sh probe.sh x::a", "sh probe.sh x::b"]);
    assert_eq!(
        rig.log(),
        ["probe x::a", "probe x::a", "probe x::b", "probe x::b"],
        "each red command runs twice"
    );
    let short = git(&rig.root, &["rev-parse", "--short", &merge]);
    let show = show.expect("a merge has its stat");
    let mut lines = show.lines();
    assert_eq!(
        lines.next(),
        Some(format!("{short} anthrex: merge t4").as_str())
    );
    assert!(
        show.contains("probe.sh | 2 ++"),
        "the task's change, against the first parent: {show}"
    );
    assert!(!show.contains("lib.txt"), "{show}");

    // Red once, then green: the probe is green; a non-merge has no stat.
    let flaky = r#"n=$(cat "$TIER_STATE/p" 2>/dev/null || echo 0); echo $((n+1)) > "$TIER_STATE/p"; [ "$n" -ge 1 ]"#;
    let spec = probe(&rig, &rig.head, &[flaky]);
    let result = run_test_at(&ctx, &sched, &queue, program, 12, &spec).await;
    assert!(
        matches!(&result, OpResult::TestAt { red: false, failing, show: None, .. } if failing.is_empty()),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(rig.state.join("p")).unwrap().trim(),
        "2"
    );
}
