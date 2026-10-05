//! Milestone 9.7, task M9.7.7 (DH §1.1, §1.2): a merged stage's delivery end to end,
//! through the real binary and a real daemon on `/tmp` paths, against `FakeHost`
//! (`PrRig`): a merged top stage cancels its working CI fix task, and the user's commits
//! pushed on top of anthrex's last push and merged before anthrex viewed that push are
//! judged by git (`merge-base --is-ancestor`), not reported as undelivered. The user's
//! merges are `FakeGithubCtl`'s; anthrex never merges anything, and every `PrRig` drop
//! asserts `forbidden.jsonl` is empty. `fake-agent` plays every agent.

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use daemon::host::fake::{CiRule, MergeMethodArg};
use daemon::host::{Conclusion, HOST_READ_TIMEOUT, PUSH_TIMEOUT};
use proto::{RunInfo, RunState, TaskState};
use serde_json::{Value, json};
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use support::run_plans::*;
use support::run_pr::*;
use support::run_pr_ci::*;

/// A base fetch that asks `contains` (task M9.7.6's `host_ops::bound`, restated: it is
/// `pub(crate)`): the base fetch and the stage branch's fetch at `PUSH_TIMEOUT` each,
/// eight reads at `HOST_READ_TIMEOUT`, and the executor's 5 s margin (485 s).
const CONTAINS_FETCH_WAIT: Duration = PUSH_TIMEOUT
    .saturating_add(PUSH_TIMEOUT)
    .saturating_add(Duration::from_secs(8 * HOST_READ_TIMEOUT.as_secs() + 5));

/// A merge at the confirmed head to the run's completion: the view that sees it, then
/// the landing's base fetch (no `contains`: the merged head is the local one).
const MERGED_DONE_WAIT: Duration = VIEW_WAIT.saturating_add(FETCH_WAIT);

/// The words of the line that says a merged stage's work was not delivered
/// (`land_judge::unpushed`).
const NOT_DELIVERED: &str = "that work is not delivered";

fn stage_ref(id: &str) -> String {
    format!("refs/heads/anthrex/{id}/stage-1")
}

/// Worker steps that hold until `path` exists, for at least `within`
/// (`run_e2e_pr_ci.rs`'s technique): `sh` steps of at most 300 s each (inside
/// `fake-agent`'s 330 s `SH_TIMEOUT`), a marker beside `path` when the wait ran out, and
/// an `expect` that fails the worker then.
fn hold_until(path: &Path, within: Duration) -> Vec<Value> {
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

/// A one-task `pr` run whose PR's CI is red once ("not reproduced": an environment fix
/// task), with `fix1`'s worker held until `go` exists for `within`, then committing to
/// the stage's file; the run id once `fix1` exists.
fn red_once_with_a_held_fix(h: &RunHarness, rig: &PrRig, go: &Path, within: Duration) -> String {
    let red_once = CiRule::new("test", Conclusion::Failure)
        .times(1)
        .log("connection reset by peer");
    rig.ctl().set_ci(vec![red_once]);
    green_scripts(&h.repo);
    let mut steps = hold_until(go, within);
    steps.extend([commit("a.txt", "a fixed\n"), done("fixed CI")]);
    scripts(h, "fix1", &steps);
    let id = start(h, &plan("", &[task("t1", &["a.txt"], "")]));
    wait_fix(h, &id, "fix1", PR_OPEN_WAIT.saturating_add(CI_FIX_WAIT));
    id
}

/// Stage 1's PR record's `pushed_head` in `run.json`, now; `None` while the file is
/// missing or mid-write.
fn pushed_head(h: &RunHarness, id: &str) -> Option<String> {
    let path = h.data().join("runs").join(id).join("run.json");
    let all: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let head = &all["delivery"]["stages"][0]["pr"]["pushed_head"];
    head.as_str().map(str::to_string)
}

/// Task `id`'s history texts, from `run.json`.
fn history_of(run: &RunInfo, id: &str) -> Vec<String> {
    let all = run_json(run);
    let task = (all["tasks"].as_array().into_iter().flatten())
        .find(|t| t["spec"]["id"] == id)
        .unwrap_or_else(|| panic!("{id} in run.json"));
    (task["history"].as_array().into_iter().flatten())
        .filter_map(|e| e["text"].as_str().map(str::to_string))
        .collect()
}

/// The bare remote's reflog of stage 1's branch (newest first).
fn stage_reflog(rig: &PrRig, id: &str) -> Vec<String> {
    let reflog = rig.bare_git(&["reflog", "show", "--format=%H", &stage_ref(id)]);
    reflog.lines().map(str::to_string).collect()
}

#[test]
fn e2e_pr_a_merged_top_stage_cancels_its_working_ci_fix_task() {
    let (h, rig) = pr_harness("");
    // Every update of the bare repository's refs is logged from here on.
    rig.bare_git(&["config", "core.logAllRefUpdates", "always"]);
    let go = h.dir.path().join("go");
    // Held past everything up to the cancel, so only the cancel ends it.
    let within = RUN_WAIT
        .saturating_add(VIEW_WAIT)
        .saturating_add(REQUEST_WAIT);
    let id = red_once_with_a_held_fix(&h, &rig, &go, within);
    let run = h.wait_run(
        &id,
        |r| t(r, "fix1").state == TaskState::Working || settled(r),
        RUN_WAIT,
    );
    assert_eq!(
        t(&run, "fix1").state,
        TaskState::Working,
        "{}",
        report(&run)
    );
    let reflog = stage_reflog(&rig, &id);
    assert!(!reflog.is_empty(), "the open's push is logged");

    // The user merges the PR with a merge commit and keeps the branch.
    rig.ctl().merge(1, MergeMethodArg::Merge, false);
    let why = "cancelled: stage 1 PR merged";
    let run = h.wait_run(
        &id,
        |r| t(r, "fix1").state != TaskState::Working || settled(r),
        VIEW_WAIT,
    );
    assert_eq!(
        t(&run, "fix1").state,
        TaskState::Cancelled,
        "{}",
        report(&run)
    );
    // `run.json` is saved on a blocking thread after the step, so it may lag the
    // snapshot (fix round 1: one run in ten read it first); waited for, as the stage
    // line is, within `REQUEST_WAIT`.
    let deadline = Instant::now() + REQUEST_WAIT;
    loop {
        let history = history_of(&run, "fix1");
        if history.iter().any(|e| e == why) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{why:?} in {history:#?} within {REQUEST_WAIT:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // Release the worker's hold; its session is already gone with the cancel.
    std::fs::write(&go, "").unwrap();
    assert!(
        !missed(&go).exists(),
        "fix1's hold ran out before the merge"
    );

    let run = h.wait_run(
        &id,
        |r| r.state == RunState::Complete || settled(r),
        MERGED_DONE_WAIT,
    );
    assert_eq!(run.state, RunState::Complete, "{}", report(&run));
    // Nothing was pushed after the merge: the remote stage branch never moved again,
    // and there is no update push's line (`watch::pushed`; the open's push logs none,
    // and fix1 never reached a push). `gh` never pushes, so `calls_of(&["push"])` is
    // empty either way; the bare repository's reflog is what a push would change.
    let log = log_lines(&h, &id);
    assert!(!log.iter().any(|l| l.contains(": pushed ")), "{log:#?}");
    assert_eq!(stage_reflog(&rig, &id), reflog);
    assert!(
        !log_lines(&h, &id).iter().any(|l| l.contains(NOT_DELIVERED)),
        "{:#?}",
        log_lines(&h, &id)
    );
}

/// BR-5: anthrex pushes its CI fix (`H2`); before any view of `H2`, the user pushes two
/// commits on top of it and merges with `method`, keeping the branch. Views are held off
/// with the fake's rate limit from before the fix's push until after the merge, so no
/// view can confirm `H2` (asserted after the fact, from `run.json`).
fn user_pushes_merged_before_a_view(method: MergeMethodArg) {
    let (h, rig) = pr_harness("");
    let go = h.dir.path().join("go");
    let id = red_once_with_a_held_fix(&h, &rig, &go, RUN_WAIT);
    let opened = rig.wait_pr(1, |_| true).head_oid;

    // Every `gh` call fails from here until the merge; `git` (the push) is unaffected.
    rig.ctl().rate_limit(u32::MAX);
    std::fs::write(&go, "").unwrap();
    // The fix's push, seen on the remote as soon as it lands.
    let deadline = Instant::now() + FIX_PUSH_WAIT;
    let h2 = loop {
        let head = rig.bare_git(&["rev-parse", &stage_ref(&id)]);
        if head != opened {
            break head;
        }
        assert!(
            Instant::now() < deadline,
            "fix1 was not pushed within {FIX_PUSH_WAIT:?}: {}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    // The push's answer processed: from here the interval base is `poll_secs`
    // (`watch::succeeded`, then `watch::pushed`), which the backoff below counts from.
    // The answer may come up to the push op's own bound after the ref moved.
    let deadline = Instant::now() + PUSH_WAIT;
    while pushed_head(&h, &id).as_deref() != Some(h2.as_str()) {
        assert!(
            Instant::now() < deadline,
            "the push of {h2} was not answered within {PUSH_WAIT:?}: {}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let pushed_at = Instant::now();
    let branch = format!("anthrex/{id}/stage-1");
    rig.ctl()
        .commit(&branch, "user1.txt", "one\n", "a user's first commit");
    let u2 = rig
        .ctl()
        .commit(&branch, "user2.txt", "two\n", "a user's second commit");
    rig.ctl().merge(1, method, false);
    rig.ctl().rate_limit(0);
    let window = pushed_at.elapsed();
    assert!(!missed(&go).exists(), "fix1's hold ran out");
    assert!(
        rig.contains(&u2, &h2),
        "the user's commits are on top of H2"
    );

    // After the push's success the interval base is `poll_secs` (1 s, `succeeded`);
    // each rate-limited view doubles it and the next view comes one base later, so the
    // views failed in `window` leave a base of at most `window + 2` s; one engine tick
    // and one second of the clock's rounding on top: `window + 4` s, which
    // `2 * floor(window) + 5` always covers (`docs/timing-budgets.md`, M9.7.7).
    let backoff = Duration::from_secs(2 * window.as_secs() + 5);
    let wait = backoff
        .saturating_add(VIEW_WAIT)
        .saturating_add(CONTAINS_FETCH_WAIT);
    let undelivered = |r: &RunInfo| r.attention.iter().any(|a| a.contains(NOT_DELIVERED));
    let run = h.wait_run(
        &id,
        |r| r.state == RunState::Complete || undelivered(r) || settled(r),
        wait,
    );
    // The premise: no view confirmed H2 before the merge (or the test proves nothing).
    // The head the PR was merged at is the user's, so `land::merged` had to ask git.
    // `run.json` is saved after the step and may lag the snapshot: the merge's view is
    // waited for in it within `REQUEST_WAIT`.
    let pr = until("the merge's view in run.json", REQUEST_WAIT, || {
        let pr = run_json(&run)["delivery"]["stages"][0]["pr"].clone();
        (pr["watermark"]["head"] == json!(u2)).then_some(pr)
    });
    let log = log_lines(&h, &id);
    assert_eq!(pr["pushed_head"], json!(h2), "{pr:#}");
    assert_ne!(
        pr["confirmed"],
        json!(h2),
        "a view confirmed H2 before the merge, so this run proves nothing: {pr:#}"
    );
    // The snapshot first: `run.json`'s log may lag the snapshot the wait stopped on.
    assert!(
        !undelivered(&run) && !log.iter().any(|l| l.contains(NOT_DELIVERED)),
        "{NOT_DELIVERED:?} in {:#?} or {log:#?}",
        run.attention
    );
    assert_eq!(run.state, RunState::Complete, "{}", report(&run));
    let lines = until("the stage line", REQUEST_WAIT, || {
        let lines = h.history_lines("stage");
        (!lines.is_empty()).then_some(lines)
    });
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert_eq!(
        (&lines[0]["stage"], &lines[0]["outcome"]),
        (&json!(1), &json!("merged")),
        "{}",
        lines[0]
    );
}

#[test]
fn e2e_pr_user_pushes_merged_before_a_view_are_delivered_by_a_merge_commit() {
    user_pushes_merged_before_a_view(MergeMethodArg::Merge);
}

#[test]
fn e2e_pr_user_pushes_merged_before_a_view_are_delivered_by_a_squash() {
    user_pushes_merged_before_a_view(MergeMethodArg::Squash);
}
