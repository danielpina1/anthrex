//! Milestone 9.2, task M9.2.16: a red CI run on a stage PR end to end, through the real
//! binary and a real daemon on `/tmp` paths, against `FakeHost`'s scripted CI (`PrRig`):
//! reproduced and bisected to its culprit, not reproduced, an infrastructure failure
//! re-run once, and the `ci_fix_max` cap. `fake-agent` plays every agent and the
//! `ci_summary` decider; nothing here can reach GitHub or run a real `gh`, and every
//! `PrRig` drop asserts `forbidden.jsonl` is empty. Split from `run_e2e_pr.rs` (AGENTS.md
//! rule 8).

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use daemon::host::Conclusion;
use daemon::host::fake::CiRule;
use proto::{RunInfo, RunState, TaskInfo, TaskOrigin, TaskState};
use serde_json::{Value, json};
use support::run_harness::{REQUEST_WAIT, RunHarness};
use support::run_plans::*;
use support::run_pr::*;
use support::run_tiers::*;

/// `run start --plan <toml> --delivery pr --yes`; the run id.
fn start(h: &RunHarness, toml: &str) -> String {
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

fn scripts(h: &RunHarness, id: &str, steps: &[Value]) {
    h.script(&format!("worker-{id}-1"), steps);
    h.script(&format!("reviewer-{id}-1"), &[approve()]);
}

/// The run stopped going anywhere: it left `running`, or a task blocked.
fn settled(r: &RunInfo) -> bool {
    !matches!(r.state, RunState::Running | RunState::AwaitingApproval)
        || r.tasks.iter().any(|t| t.state == TaskState::Blocked)
}

fn ci_fixes(run: &RunInfo) -> Vec<&TaskInfo> {
    (run.tasks.iter())
        .filter(|t| t.origin == TaskOrigin::Ci)
        .collect()
}

/// Waits until the run has the fix task `fix`, or cannot get there; the run.
fn wait_fix(h: &RunHarness, id: &str, fix: &str, wait: Duration) -> RunInfo {
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
fn wait_fix_pushed(h: &RunHarness, rig: &PrRig, id: &str, fix: &str, number: u64) -> String {
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

/// Worker steps that wait for `path` for at least `within`, as `run_e2e_stages.rs`'s
/// do: `sh` steps of at most 300 s each (inside `fake-agent`'s 330 s `SH_TIMEOUT`), a
/// marker beside `path` when the wait ran out, and an `expect` that fails the worker.
fn wait_for(path: &Path, within: Duration) -> Vec<Value> {
    let steps = within.as_secs().div_ceil(300);
    let (p, m) = (path.display(), missed(path).display().to_string());
    let mut out: Vec<Value> = (0..steps)
        .map(|_| {
            sh(&format!(
                "for i in $(seq 1 1500); do [ -e '{p}' ] && break; sleep 0.2; done; \
                 if [ -e '{p}' ]; then echo '{{\"go\":true}}'; else touch '{m}'; echo '{{\"go\":false}}'; fi"
            ))
        })
        .collect();
    out.push(json!({"expect": {"pointer": "/go", "equals": true}}));
    out
}

fn missed(path: &Path) -> PathBuf {
    path.with_extension("missed")
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

/// Task `id`'s brief, from `run.json` (the snapshot leaves briefs out).
fn brief_of(run: &RunInfo, id: &str) -> String {
    let all = run_json(run);
    let task = (all["tasks"].as_array().into_iter().flatten())
        .find(|t| t["spec"]["id"] == id)
        .unwrap_or_else(|| panic!("{id} in run.json"));
    task["spec"]["brief"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The repository's `history.jsonl` lines of type `bisect`.
fn bisect_lines(h: &RunHarness) -> Vec<Value> {
    let path = h.repo_dir().join(daemon::run::engine::HISTORY_FILE);
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|l| l["type"] == "bisect")
        .collect()
}

/// Stage 1's scenario of both bisect-path tests: CI is red while `mods/b/CI_FAIL`
/// contains `bad`, which `t2` writes; the scripted `ci_summary` names `b::ci`; `fix1`
/// removes the marker.
fn marker_ci(h: &RunHarness, rig: &PrRig) {
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
fn ci_files() -> Vec<(&'static str, &'static str)> {
    let mut files = tier_repo_files();
    files.push(("one.sh", ONE_SH));
    files
}

#[test]
fn e2e_pr_ci_red_reproduced_bisects_fixes_pushes_and_propagates() {
    let (h, rig) = pr_harness_with("", &ci_files(), "", Some("claude"));
    marker_ci(&h, &rig);
    // Stage 2's task waits until stage 1's fix is on its PR, so stage 2's PR opens on a
    // head that already holds the fix (propagated) and its CI is never red.
    let go = h.dir.path().join("go");
    let within = PR_OPEN_WAIT
        .saturating_add(CI_FIX_WAIT)
        .saturating_add(FIX_PUSH_WAIT)
        .saturating_add(REQUEST_WAIT);
    let mut steps = wait_for(&go, within);
    steps.extend([commit("mods/c/src.txt", "c2\n"), done("c")]);
    scripts(&h, "t3", &steps);
    // Tiered, with `single_test` for the bisect, and no `check`: decision 36's range
    // starts after the stage's last green tier 3, and with a `check` that is the very
    // head the PR opened with (decision 19), so nothing would be left to bisect. Without
    // one the PR opens unverified and the range is every merge of the stage.
    let profile = tier_profile(&tier_log_path(&h), "")
        .replace("check = \"sh check.sh {filter:--filter %}\"\n", "")
        .replace(
            "single_test = \"sh test.sh --one {test}\"",
            "single_test = \"sh one.sh {test}\"",
        );
    assert!(
        profile.contains("sh one.sh {test}") && !profile.contains("check.sh"),
        "{profile}"
    );
    let tasks = [
        task("t1", &["mods/a/src.txt"], ""),
        task("t2", &["mods/b/CI_FAIL"], ""),
        task("t3", &["mods/c/src.txt"], "stage = 2"),
    ];
    let id = start(
        &h,
        &format!("goal = \"Add a\"\n\n{profile}\n{}", tasks.concat()),
    );

    let run = wait_fix(&h, &id, "fix1", PR_OPEN_WAIT.saturating_add(CI_FIX_WAIT));
    // Fix round 1 (m1): t3 is already at work on stage 2's line, so the line predates
    // the fix, and the fix can reach stage 2 only by a propagate.
    let t3 = t(&run, "t3").state;
    assert!(
        matches!(t3, TaskState::Working | TaskState::Merged),
        "t3 is {t3:?}"
    );
    let summaries: Vec<Value> = (h.decider_calls().into_iter())
        .filter(|c| c["kind"] == "ci_summary")
        .collect();
    assert_eq!(summaries.len(), 1, "{summaries:#?}");
    assert_eq!(
        rig.calls_of(&["run", "view"]).len(),
        1,
        "one failed log read"
    );
    let lines = until("the bisect history line", REQUEST_WAIT, || {
        let lines = bisect_lines(&h);
        (!lines.is_empty()).then_some(lines)
    });
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert_eq!(
        (
            &lines[0]["stage"],
            &lines[0]["culprit"],
            &lines[0]["fix_task"]
        ),
        (&json!(1), &json!("t2"), &json!("fix1")),
        "{}",
        lines[0]
    );
    let (fix, culprit) = (t(&run, "fix1"), t(&run, "t2"));
    let brief = brief_of(&run, "fix1");
    assert_eq!(fix.origin, TaskOrigin::Ci);
    assert_eq!(fix.owns, culprit.owns, "the culprit's owns, exactly");
    assert_eq!(fix.title, "Fix CI on stage 1: b::ci");
    for text in [
        "CI failed on stage 1's pull request, at ",
        "Category: test. It reproduces locally with: ",
        "Bisect found the merge of task t2 (Task t2) as the first red; its brief follows.",
        "  b::ci failed on the PR head\n",
        "Stage 1's pull request: https://github.com/fake/app/pull/1.",
    ] {
        assert!(brief.contains(text), "{text:?} in:\n{brief}");
    }

    // The fix merges, is pushed to PR 1, and CI goes green there.
    let merged = wait_fix_pushed(&h, &rig, &id, "fix1", 1);
    rig.wait_stage_within(&h, &id, 1, "/ci", &json!("green"), VIEW_WAIT);
    // Then stage 2: its PR opens on a head that holds the fix, and is green.
    assert!(!missed(&go).exists(), "t3's wait ran out before `go`");
    std::fs::write(&go, "").unwrap();
    rig.wait_stage(&h, &id, 2, "/state", &json!("open"));
    let two = rig.wait_pr(2, |_| true);
    assert!(rig.contains(&two.head_oid, &merged), "propagated: {two:#?}");
    rig.wait_stage_within(&h, &id, 2, "/ci", &json!("green"), VIEW_WAIT);
    let run = h.run(&id).unwrap();
    // Stage 2's PR opened only with its merge queue quiet, so the propagate is done.
    let propagated = format!("stage 2: propagated stage 1 at {}", &merged[..7]);
    let log: Vec<String> = (run_json(&run)["log"].as_array().into_iter().flatten())
        .filter_map(|l| l["text"].as_str().map(str::to_string))
        .collect();
    assert!(
        log.iter().any(|l| l.starts_with(&propagated)),
        "{propagated:?} in {log:#?}"
    );
    assert_eq!(ci_fixes(&run).len(), 1, "one CI fix task in all");
    assert_eq!(bisect_lines(&h).len(), 1, "one bisect only");
    assert_eq!(run.state, RunState::Running, "{:?}", run.halted_reason);
}

/// Fix round 1 (m3): the same red under a profile WITH a `check`. Tier 3 was green on
/// the head the PR opened with, so 9.1's bisect has no merge after it to probe, and the
/// red becomes a stage fix with no bisect line.
#[test]
fn e2e_pr_ci_red_reproduced_under_a_check_is_a_stage_fix_without_a_bisect() {
    let (h, rig) = pr_harness_with("", &ci_files(), "", Some("claude"));
    marker_ci(&h, &rig);
    let profile = tier_profile(&tier_log_path(&h), "").replace(
        "single_test = \"sh test.sh --one {test}\"",
        "single_test = \"sh one.sh {test}\"",
    );
    assert!(
        profile.contains("sh one.sh {test}") && profile.contains("check.sh"),
        "{profile}"
    );
    let tasks = [
        task("t1", &["mods/a/src.txt"], ""),
        task("t2", &["mods/b/CI_FAIL"], ""),
    ];
    let id = start(
        &h,
        &format!("goal = \"Add a\"\n\n{profile}\n{}", tasks.concat()),
    );
    let run = wait_fix(&h, &id, "fix1", PR_OPEN_WAIT.saturating_add(CI_FIX_WAIT));
    let head = rig.wait_pr(1, |_| true).head_oid;
    let h7 = &head[..7];
    let log: Vec<String> = (run_json(&run)["log"].as_array().into_iter().flatten())
        .filter_map(|l| l["text"].as_str().map(str::to_string))
        .collect();
    // RULING F1 (task M9.2.16): this is today's text; the final wave replaces it (the
    // head is on the stage's line, the range after its green tier 3 is empty), and
    // this pin moves with it.
    let not_bisected = format!(
        "stage 1: CI red at {h7} reproduces; not bisected: {h7} is not a merge recorded on the stage's line"
    );
    assert!(log.contains(&not_bisected), "{not_bisected:?} in {log:#?}");
    let (fix, brief) = (t(&run, "fix1"), brief_of(&run, "fix1"));
    assert_eq!(fix.origin, TaskOrigin::Ci);
    assert_eq!(
        fix.owns,
        ["mods/a/src.txt", "mods/b/CI_FAIL"],
        "the stage's owns, not a culprit's"
    );
    for text in [
        "Category: test. It reproduces locally with: ",
        "No single task's merge is the cause.\n",
    ] {
        assert!(brief.contains(text), "{text:?} in:\n{brief}");
    }
    assert!(bisect_lines(&h).is_empty(), "{:#?}", bisect_lines(&h));
    assert_eq!(ci_fixes(&run).len(), 1);
}

/// A one-task `pr` run whose PR's CI is `rules`, with `fix1` scripted to commit to the
/// stage's file; the run id.
fn one_task_run(h: &RunHarness, rig: &PrRig, rules: Vec<CiRule>) -> String {
    rig.ctl().set_ci(rules);
    green_scripts(&h.repo);
    scripts(h, "fix1", &[commit("a.txt", "a fixed\n"), done("fixed CI")]);
    start(h, &plan("", &[task("t1", &["a.txt"], "")]))
}

#[test]
fn e2e_pr_ci_red_not_reproduced_adds_an_environment_fix_task() {
    let (h, rig) = pr_harness("");
    let red_once = CiRule::new("test", Conclusion::Failure)
        .times(1)
        .log("connection reset by peer");
    let id = one_task_run(&h, &rig, vec![red_once]);
    let run = wait_fix(&h, &id, "fix1", PR_OPEN_WAIT.saturating_add(CI_FIX_WAIT));
    let (fix, brief) = (t(&run, "fix1"), brief_of(&run, "fix1"));
    assert_eq!(fix.origin, TaskOrigin::Ci);
    assert_eq!(fix.title, "Fix CI on stage 1: test");
    assert_eq!(fix.owns, ["a.txt"], "the stage's owns");
    // No decider: the fallback's summary, no test names, so the stage's tier-2 steps
    // reproduce it, green: TT's environment sentence.
    for text in [
        "Category: unknown. This failure does not reproduce locally; the difference is in CI's environment. Find it.\n",
        "No single task's merge is the cause.\n",
        "connection reset by peer",
    ] {
        assert!(brief.contains(text), "{text:?} in:\n{brief}");
    }
    assert!(rig.calls_of(&["run", "rerun"]).is_empty(), "not re-run");
    wait_fix_pushed(&h, &rig, &id, "fix1", 1);
    rig.wait_stage_within(&h, &id, 1, "/ci", &json!("green"), VIEW_WAIT);
    assert_eq!(ci_fixes(&h.run(&id).unwrap()).len(), 1);
}

#[test]
fn e2e_pr_ci_infra_failure_is_rerun_once() {
    let (h, rig) = pr_harness("");
    let cancelled = CiRule::new("build", Conclusion::Cancelled).times(1);
    let id = one_task_run(&h, &rig, vec![cancelled]);
    rig.wait_stage(&h, &id, 1, "/state", &json!("open"));
    let wait = VIEW_WAIT
        .saturating_add(RERUN_WAIT)
        .saturating_add(VIEW_WAIT);
    rig.wait_stage_within(&h, &id, 1, "/ci", &json!("green"), wait);
    let reruns = rig.calls_of(&["run", "rerun"]);
    assert_eq!(reruns.len(), 1, "{reruns:?}");
    assert!(reruns[0].contains(&"--failed".to_string()), "{reruns:?}");
    let run = h.run(&id).unwrap();
    assert!(ci_fixes(&run).is_empty(), "no fix task");
    assert!(
        rig.calls_of(&["run", "view"]).is_empty(),
        "no log read for infra"
    );
    assert_eq!(run.state, RunState::Running);
}

#[test]
fn e2e_pr_ci_fix_cap_hands_the_red_to_the_user() {
    let (h, rig) = pr_harness("ci_fix_max = 1\n");
    let always = CiRule::new("test", Conclusion::Failure).log("still failing");
    let id = one_task_run(&h, &rig, vec![always]);
    let line = "stage 1 CI still red on unknown after 1 fix tasks; over to you";
    let wait = PR_OPEN_WAIT
        .saturating_add(CI_FIX_WAIT)
        .saturating_add(FIX_PUSH_WAIT)
        .saturating_add(CI_FIX_WAIT);
    let run = h.wait_run(
        &id,
        |r| r.attention.iter().any(|a| a == line) || settled(r),
        wait,
    );
    assert!(
        run.attention.iter().any(|a| a == line),
        "{:?} {:?}",
        run.attention,
        run.halted_reason
    );
    let fixes = ci_fixes(&run);
    assert_eq!(fixes.len(), 1, "the cap is one fix task");
    assert_eq!(fixes[0].state, TaskState::Merged);
    let alerts = &run.delivery.as_ref().expect("a pr run").alerts;
    assert!(
        alerts.iter().any(|a| a.text == line),
        "the typed alert: {alerts:?}"
    );
    assert_eq!(run.state, RunState::Running, "handed over, not halted");
}
