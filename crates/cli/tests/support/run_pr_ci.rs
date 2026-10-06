use std::time::Duration;

use daemon::host::Conclusion;
use daemon::host::fake::CiRule;
use proto::{RunInfo, RunState, TaskInfo, TaskOrigin, TaskState};
use serde_json::{Value, json};

use super::run_harness::RunHarness;
use super::run_plans::*;
use super::run_pr::*;
use super::run_tiers::*;

/// `run start --plan <toml> --delivery pr --yes`; the run id.
pub fn start(h: &RunHarness, toml: &str) -> String {
    let plan = h.plan(toml).display().to_string();
    let repo = h.repo.display().to_string();
    let args = [
        "run",
        "start",
        "--plan",
        &plan,
        "--dir",
        &repo,
        "--delivery",
        "pr",
        "--yes",
    ];
    let out = pr_start(h, &args);
    assert!(
        out.status.success(),
        "start failed: {}{}",
        String::from_utf8_lossy(&out.stderr),
        h.log_tail()
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn scripts(h: &RunHarness, id: &str, steps: &[Value]) {
    h.script(&format!("worker-{id}-1"), steps);
    h.script(&format!("reviewer-{id}-1"), &[approve()]);
}

/// The run stopped going anywhere: it left `running`, or a task blocked.
pub fn settled(r: &RunInfo) -> bool {
    !matches!(r.state, RunState::Running | RunState::AwaitingApproval)
        || r.tasks.iter().any(|t| t.state == TaskState::Blocked)
}

pub fn ci_fixes(run: &RunInfo) -> Vec<&TaskInfo> {
    (run.tasks.iter())
        .filter(|t| t.origin == TaskOrigin::Ci)
        .collect()
}

/// Waits until the run has the fix task `fix`, or cannot get there; the run.
pub fn wait_fix(h: &RunHarness, id: &str, fix: &str, wait: Duration) -> RunInfo {
    let run = h.wait_run(
        id,
        |r| r.tasks.iter().any(|t| t.id == fix) || settled(r),
        wait,
    );
    assert!(
        run.tasks.iter().any(|t| t.id == fix),
        "no {fix}: {:?} {:?}",
        run.attention,
        run.halted_reason
    );
    run
}

/// Waits until fix task `fix` has merged and PR `number`'s head contains its merge; the
/// merge commit.
pub fn wait_fix_pushed(h: &RunHarness, rig: &PrRig, id: &str, fix: &str, number: u64) -> String {
    let run = h.wait_run(
        id,
        |r| t(r, fix).state == TaskState::Merged || settled(r),
        FIX_PUSH_WAIT,
    );
    let merged = t(&run, fix).merge_commit.clone();
    let merged = merged.unwrap_or_else(|| panic!("{fix} did not merge: {}", report(&run)));
    until("the fix on the PR", FIX_PUSH_WAIT, || {
        (rig.ctl().prs().iter())
            .any(|p| p.number == number && rig.contains(&p.head_oid, &merged))
            .then_some(())
    });
    merged
}

/// The single test `<module>::ci` fails while `mods/<module>/CI_FAIL` exists; a module's
/// tests (tiers 1 and 2) never read the marker, so the stage is green locally and its CI
/// red reproduces only by name. Any other name is `test.sh --one`'s.
const ONE_SH: &str = r#"case "$1" in
  *::ci)
    m=${1%%::*}
    if [ -f "mods/$m/CI_FAIL" ]; then echo "test $1 ... FAILED"; exit 1; fi
    echo "PASS $1"; exit 0 ;;
esac
exec sh test.sh --one "$1"
"#;

/// Stage 1's scenario of both bisect-path tests: CI is red while `mods/b/CI_FAIL`
/// contains `bad`, which `t2` writes; the scripted `ci_summary` names `b::ci`; `fix1`
/// removes the marker.
pub fn marker_ci(h: &RunHarness, rig: &PrRig) {
    rig.ctl().set_ci(vec![
        CiRule::new("test", Conclusion::Failure)
            .when("mods/b/CI_FAIL", "bad")
            .failing(&["b::ci"])
            .log("running b::ci"),
    ]);
    h.decider(
        "ci_summary",
        1,
        json!({"answer": {"lines": ["b::ci failed on the PR head"], "failing_tests": ["b::ci"], "category": "test"}}),
    );
    scripts(h, "t1", &[commit("mods/a/src.txt", "a2\n"), done("a")]);
    scripts(h, "t2", &[commit("mods/b/CI_FAIL", "bad\n"), done("b")]);
    let fix = sh("git rm -q mods/b/CI_FAIL && git commit -qm 'drop the CI marker'");
    scripts(h, "fix1", &[fix, done("fixed b::ci")]);
}

/// The tiered repository's files, with `one.sh` for `single_test`.
pub fn ci_files() -> Vec<(&'static str, &'static str)> {
    let mut files = tier_repo_files();
    files.push(("one.sh", ONE_SH));
    files
}
