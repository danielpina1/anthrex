//! Milestone 9.2, task M9.2.17: the base moving and the user landing stage PRs end to
//! end, through the real binary and a real daemon on `/tmp` paths, against `FakeHost`
//! (`PrRig`): a base conflict becoming a `sync` task, a squash merge retargeting the
//! next stage, a PR closed without merging pausing the stages above, a daemon restart
//! resuming from the watermark, and completion once every PR has landed. The user's
//! merges are `FakeGithubCtl`'s; anthrex never merges anything, and every `PrRig` drop
//! asserts `forbidden.jsonl` is empty.

mod support;

use daemon::host::Conclusion;
use daemon::host::fake::{CiRule, MergeMethodArg};
use proto::{PrState, RunInfo, RunState, TaskOrigin, TaskState};
use serde_json::{Value, json};
use support::orch_script::{
    add, edit_plan, plan_task, prompt, read as read_message, until as status_until,
};
use support::run_harness::{REQUEST_WAIT, RunHarness};
use support::run_orch::ORCH_WAIT;
use support::run_plans::*;
use support::run_pr::*;
use support::run_tiers::{runs_of, tier_log, tier_plan, tier_repo_files};

const ORCH: &str = "orchestrator-run-1";

fn scripts(h: &RunHarness, id: &str, steps: &[Value]) {
    h.script(&format!("worker-{id}-1"), steps);
    h.script(&format!("reviewer-{id}-1"), &[approve()]);
}

/// The run stopped going anywhere: it left `running`, or a task blocked.
fn settled(r: &RunInfo) -> bool {
    !matches!(r.state, RunState::Running | RunState::AwaitingApproval)
        || r.tasks.iter().any(|t| t.state == TaskState::Blocked)
}

fn stage_branch(id: &str, n: u16) -> String {
    format!("anthrex/{id}/stage-{n}")
}

/// A green two-stage fast-path `pr` run (`t1` on `a.txt` in stage 1, `t2` on `b.txt`
/// in stage 2), its profile `check = "true"` or `profile`'s lines, both PRs open and
/// green; the run id.
fn open_two(h: &RunHarness, rig: &PrRig, profile: &str) -> String {
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    scripts(h, "t1", &[commit("a.txt", "a\n"), done("a")]);
    scripts(h, "t2", &[commit("b.txt", "b\n"), done("b")]);
    let tasks = [
        task("t1", &["a.txt"], ""),
        task("t2", &["b.txt"], "stage = 2"),
    ];
    let toml = if profile.is_empty() {
        plan("", &tasks)
    } else {
        format!(
            "goal = \"Add a\"\n\n[profile]\n{profile}\n{}",
            tasks.concat()
        )
    };
    let id = pr_run(h, &toml);
    for n in [1, 2] {
        rig.wait_stage(h, &id, n, "/state", &json!("open"));
        rig.wait_stage(h, &id, n, "/ci", &json!("green"));
    }
    id
}

/// The parents of `commit` in the bare remote.
fn parents(rig: &PrRig, commit: &str) -> Vec<String> {
    let line = rig.bare_git(&["rev-list", "--parents", "-n", "1", commit]);
    line.split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

#[test]
fn e2e_pr_base_conflict_becomes_a_sync_task() {
    let (h, rig) = pr_harness("");
    // Every update of the bare repository's refs is logged from here on.
    rig.bare_git(&["config", "core.logAllRefUpdates", "always"]);
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    scripts(&h, "t1", &[commit("a.txt", "from the stage\n"), done("a")]);
    let resolve = sh(
        "grep -q '^<<<<<<<' a.txt && printf 'from the stage\\nfrom main\\n' > a.txt && git add a.txt && git commit -qm 'resolve the merge of main'",
    );
    scripts(&h, "fix1", &[resolve, done("kept both")]);
    let id = pr_run(&h, &plan("", &[task("t1", &["a.txt"], "")]));
    rig.wait_stage(&h, &id, 1, "/ci", &json!("green"));
    let old = rig.wait_pr(1, |_| true).head_oid;

    // The base advances on the remote with a change that conflicts with the stage's.
    let base = rig
        .ctl()
        .commit("main", "a.txt", "from main\n", "a user's change on main");
    let run = h.wait_run(
        &id,
        |r| r.tasks.iter().any(|t| t.id == "fix1") || settled(r),
        LAND_WAIT,
    );
    let fix = run.tasks.iter().find(|t| t.id == "fix1");
    let fix = fix.unwrap_or_else(|| panic!("no sync task: {:?}", run.attention));
    assert_eq!(fix.origin, TaskOrigin::Sync);
    assert_eq!(fix.owns, ["a.txt"], "the conflicted file, exactly");
    assert_eq!(fix.title, "Sync stage 1 with main");
    assert_eq!(
        fix.fixes.as_deref(),
        Some(format!("sync with main@{}", &base[..7]).as_str())
    );

    // The resolution merges into the stage and is pushed. The sync task's commit is the
    // merge of the base into the stage, with two parents, the old head and the base
    // (9.1's hand-back); the stage's new head is M8a's merge of it. Never a rebase:
    // the old head is still in the history, unchanged, and every push moved forward.
    let merged = until("the sync on the PR", FIX_PUSH_WAIT, || {
        let pr = rig.wait_pr(1, |_| true);
        (pr.head_oid != old && rig.contains(&pr.head_oid, &base)).then_some(pr.head_oid)
    });
    assert!(rig.contains(&merged, &old), "the old head is kept");
    let run = h.run(&id).unwrap();
    let fix = run.tasks.iter().find(|t| t.id == "fix1").unwrap();
    assert_eq!(fix.state, TaskState::Merged, "{}", report(&run));
    let resolution = fix.head.clone().expect("the sync task's head");
    assert_eq!(parents(&rig, &resolution), [old.clone(), base.clone()]);
    // The merge queue's merge of the sync task: the old head, then the resolution.
    assert_eq!(parents(&rig, &merged), [old.clone(), resolution]);
    assert_eq!(
        rig.bare_git(&["show", &format!("{merged}:a.txt")]),
        "from the stage\nfrom main"
    );
    rig.wait_stage_within(&h, &id, 1, "/ci", &json!("green"), VIEW_WAIT);
    // Every update of the remote stage branch moved it forward: nothing was rewritten.
    let branch = format!("refs/heads/{}", stage_branch(&id, 1));
    let reflog = rig.bare_git(&["reflog", "show", "--format=%H", &branch]);
    let updates: Vec<&str> = reflog.lines().collect();
    assert!(
        updates.len() >= 2,
        "the open and the sync's push: {updates:?}"
    );
    for pair in updates.windows(2) {
        assert!(
            rig.contains(pair[0], pair[1]),
            "a fast-forward: {updates:?}"
        );
    }
}

#[test]
fn e2e_pr_squash_merged_stage_retargets_the_next() {
    let (h, rig) = pr_harness("");
    let id = open_two(&h, &rig, "");
    let old2 = rig.wait_pr(2, |_| true).head_oid;
    let tree = rig.bare_git(&["rev-parse", &format!("{old2}^{{tree}}")]);

    // The user's squash, deleting the branch: GitHub retargets PR 2 to main itself.
    rig.ctl().merge(1, MergeMethodArg::Squash, true);
    let squash = rig.bare_git(&["rev-parse", "refs/heads/main"]);
    let two = until("stage 2 synced and pushed", LAND_WAIT, || {
        let pr = rig.wait_pr(2, |_| true);
        (pr.head_oid != old2 && rig.contains(&pr.head_oid, &squash)).then_some(pr)
    });
    assert_eq!(two.base, "main");
    assert!(rig.contains(&two.head_oid, &old2), "merged, not rebased");
    assert_eq!(
        rig.bare_git(&["rev-parse", &format!("{}^{{tree}}", two.head_oid)]),
        tree,
        "the sync changed no file"
    );
    assert_eq!(
        parents(&rig, &two.head_oid),
        [old2.clone(), squash.clone()],
        "a base sync's merge"
    );
    // anthrex's own retarget, a harmless repeat of GitHub's.
    until("the retarget", LAND_WAIT, || {
        let edits = rig.calls_of(&["pr", "edit", "2"]);
        edits
            .iter()
            .any(|a| a.windows(2).any(|w| w == ["--base", "main"]))
            .then_some(())
    });
    let lines = until("the stage line", REQUEST_WAIT, || {
        let lines = h.history_lines("stage");
        (!lines.is_empty()).then_some(lines)
    });
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert_eq!(
        (
            &lines[0]["stage"],
            &lines[0]["pr"],
            &lines[0]["outcome"],
            &lines[0]["merge_method"]
        ),
        (
            &json!(1),
            &json!(1),
            &json!("merged"),
            &json!("squash_or_rebase")
        ),
        "{}",
        lines[0]
    );
    let run = h.run(&id).unwrap();
    assert_eq!(run.state, RunState::Running, "{:?}", run.halted_reason);
    assert!(
        !rig.remote_refs()
            .contains(&format!("refs/heads/{}", stage_branch(&id, 1))),
        "GitHub deleted the merged branch"
    );
}

#[test]
fn e2e_pr_closed_without_merging_pauses_the_stages_above() {
    let (h, rig) = orch_pr_harness();
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    scripts(&h, "t1", &[commit("a.txt", "a\n"), done("a")]);
    scripts(&h, "t2", &[commit("b.txt", "b\n"), done("b")]);
    let closed = "stage 1 PR closed without merging; resume, re-plan, or cancel the rest";
    let steps = [
        prompt(),
        edit_plan(
            vec![
                add(plan_task("t1", &["a.txt"], json!({}))),
                add(plan_task("t2", &["b.txt"], json!({"stage": 2}))),
            ],
            json!({"submit": true}),
        ),
        status_until("/gate/state", json!("approved"), ORCH_WAIT),
        read_message(Some(closed)),
        read_message(None),
    ];
    h.script(ORCH, &steps);
    let id = pr_goal(&h, "add a and b");
    h.wait_run(&id, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);
    let approved = h.anthrex(&["run", "approve", &id]);
    assert!(
        approved.status.success(),
        "{}",
        String::from_utf8_lossy(&approved.stderr)
    );
    rig.wait_stage(&h, &id, 2, "/state", &json!("open"));

    rig.ctl().close(1);
    let run = h.wait_run(
        &id,
        |r| r.attention.iter().any(|a| a == closed) || settled(r),
        VIEW_WAIT,
    );
    assert!(
        run.attention.iter().any(|a| a == closed),
        "{:?}",
        run.attention
    );
    rig.wait_stage_within(&h, &id, 2, "/paused", &json!(true), VIEW_WAIT);
    // The wake line, word for word.
    h.wait_messages(ORCH, 1, REQUEST_WAIT);
    let text = h.read_messages(ORCH)[0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains(closed), "{text}");
    assert_eq!(rig.wait_pr(2, |_| true).state, PrState::Open, "left alone");
    assert!(
        rig.calls_of(&["pr", "reopen"]).is_empty(),
        "anthrex never reopens"
    );

    // The user reopens it on GitHub: the stages above resume.
    rig.ctl().reopen(1);
    rig.wait_stage_within(
        &h,
        &id,
        2,
        "/paused",
        &json!(false),
        VIEW_WAIT.saturating_add(VIEW_WAIT),
    );
    let run = h.wait_run(&id, |r| !r.attention.iter().any(|a| a == closed), VIEW_WAIT);
    assert_eq!(run.state, RunState::Running, "{:?}", run.halted_reason);
    rig.wait_stage(&h, &id, 1, "/state", &json!("open"));
}

#[test]
fn e2e_pr_restart_resumes_watching_without_duplicate_fix_tasks() {
    let (mut h, rig) = pr_harness("");
    let red_once = CiRule::new("test", Conclusion::Failure)
        .times(1)
        .log("connection reset by peer");
    rig.ctl().set_ci(vec![red_once]);
    scripts(&h, "t1", &[commit("a.txt", "a1\na2\n"), done("a")]);
    scripts(
        &h,
        "fix1",
        &[commit("a.txt", "a1 ci\na2\n"), done("fixed CI")],
    );
    scripts(
        &h,
        "fix2",
        &[commit("a.txt", "a1 ci\na2 review\n"), done("fixed review")],
    );
    let id = pr_run(&h, &plan("", &[task("t1", &["a.txt"], "")]));
    // The red CI is processed: fix1 (origin ci) is pushed and CI goes green.
    let run = h.wait_run(
        &id,
        |r| {
            r.tasks
                .iter()
                .any(|t| t.id == "fix1" && t.state == TaskState::Merged)
                || settled(r)
        },
        PR_OPEN_WAIT
            .saturating_add(CI_FIX_WAIT)
            .saturating_add(FIX_PUSH_WAIT),
    );
    let fix1 = run.tasks.iter().find(|t| t.id == "fix1");
    assert_eq!(
        fix1.map(|t| t.origin),
        Some(TaskOrigin::Ci),
        "{}",
        report(&run)
    );
    rig.wait_stage_within(&h, &id, 1, "/ci", &json!("green"), FIX_PUSH_WAIT);
    // A writer's comment is processed: fix2 (origin review) is pushed and replied on.
    let first = rig
        .ctl()
        .review_comment(1, "tester", "a.txt", 2, "line 2 too, please");
    let run = h.wait_run(
        &id,
        |r| {
            r.tasks
                .iter()
                .any(|t| t.id == "fix2" && t.state == TaskState::Merged)
                || settled(r)
        },
        COMMENT_WAIT.saturating_add(FIX_PUSH_WAIT),
    );
    let fix2 = run.tasks.iter().find(|t| t.id == "fix2");
    assert_eq!(
        fix2.map(|t| t.origin),
        Some(TaskOrigin::Review),
        "{}",
        report(&run)
    );
    let replied = |rig: &PrRig| {
        (rig.wait_pr(1, |_| true).replies.iter())
            .filter(|r| r.thread == Some(first))
            .count()
    };
    until(
        "the reply",
        FIX_PUSH_WAIT.saturating_add(REPLY_WAIT),
        || (replied(&rig) == 1).then_some(()),
    );
    let tasks: Vec<String> = run.tasks.iter().map(|t| t.id.clone()).collect();
    let log_before = log_lines(&h, &id);
    let batches = |log: &[String]| log.iter().filter(|l| l.contains("review batch")).count();
    let posts = rig.calls_of(&["api", "-X", "POST"]).len();
    let reruns = rig.calls_of(&["run", "rerun"]).len();
    let logs = rig.calls_of(&["run", "view"]).len();

    // The harness's own daemon, restarted through its own socket; the run comes back
    // paused, and `run resume` watches again from the watermark.
    h.restart_daemon(&[]);
    let run = h.wait_run(&id, |r| r.state == RunState::Paused, REQUEST_WAIT);
    assert_eq!(run.state, RunState::Paused);
    let repo = h.repo.display().to_string();
    let resumed = h.anthrex(&["run", "resume", &id, "--dir", &repo]);
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let views = rig.calls_of(&["pr", "view"]).len();
    until(
        "two more views",
        VIEW_WAIT.saturating_add(VIEW_WAIT),
        || (rig.calls_of(&["pr", "view"]).len() >= views + 2).then_some(()),
    );
    // No task, no reply, no re-run, no log read and no batch (the wake's source).
    let run = h.run(&id).unwrap();
    let after: Vec<String> = run.tasks.iter().map(|t| t.id.clone()).collect();
    assert_eq!(after, tasks, "no new task");
    assert_eq!(replied(&rig), 1, "one reply per thread");
    assert_eq!(
        rig.calls_of(&["api", "-X", "POST"]).len(),
        posts,
        "no new reply"
    );
    assert_eq!(rig.calls_of(&["run", "rerun"]).len(), reruns);
    assert_eq!(rig.calls_of(&["run", "view"]).len(), logs);
    assert_eq!(batches(&log_lines(&h, &id)), batches(&log_before));
    assert_eq!(run.state, RunState::Running, "{:?}", run.halted_reason);
}

/// Each stage's tier-3 record (`run.json`'s `stages[].full`).
fn tier3_records(h: &RunHarness, id: &str) -> Vec<Value> {
    let all = h.run_json(id);
    (all["stages"].as_array().into_iter().flatten())
        .map(|s| s["full"].clone())
        .collect()
}

#[test]
fn e2e_pr_run_completes_when_every_pr_has_landed() {
    // A tiered profile, so 9.1's completion would run tier 3 on any stage head without
    // a green one; its idle trigger is put out of reach (`full_idle_secs`), so only a
    // completion could start a tier 3 after the merges.
    let idle = "[testing]\nfull_idle_secs = 3600\n";
    let (h, rig) = pr_harness_with("", &tier_repo_files(), idle, None);
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    scripts(&h, "t1", &[commit("mods/a/src.txt", "a2\n"), done("a")]);
    scripts(&h, "t2", &[commit("mods/c/src.txt", "c2\n"), done("c")]);
    let tasks = [
        task("t1", &["mods/a/src.txt"], ""),
        task("t2", &["mods/c/src.txt"], "stage = 2"),
    ];
    let id = pr_run(&h, &tier_plan(&h, "", &tasks));
    for n in [1, 2] {
        rig.wait_stage(&h, &id, n, "/state", &json!("open"));
        rig.wait_stage(&h, &id, n, "/ci", &json!("green"));
    }
    // Tier 3's runs: `check.sh` in a stage's `.full` checkout.
    let full_runs = || {
        let log = tier_log(&h);
        (runs_of(&log, "check.sh").into_iter())
            .filter(|l| l["cwd"].as_str().is_some_and(|c| c.ends_with("/.full")))
            .count()
    };
    assert_eq!(
        full_runs(),
        2,
        "tier 3 once per stage, before its PR opened"
    );

    // The user merges stage 1 with a merge commit; stage 2 is synced (its head moves,
    // and has no tier 3), pushed and retargeted to main before the user merges it too.
    rig.ctl().merge(1, MergeMethodArg::Merge, false);
    let one = rig.bare_git(&["rev-parse", "refs/heads/main"]);
    until("PR 2 synced and retargeted", LAND_WAIT, || {
        let pr = rig.wait_pr(2, |_| true);
        (pr.base == "main" && rig.contains(&pr.head_oid, &one)).then_some(())
    });
    let before = (full_runs(), tier3_records(&h, &id));
    rig.ctl().merge(2, MergeMethodArg::Merge, false);
    let run = h.wait_run(
        &id,
        |r| r.state == RunState::Complete || settled(r),
        LAND_WAIT,
    );
    assert_eq!(run.state, RunState::Complete, "{}", report(&run));
    // No tier 3 after the second merge: no `check.sh` in `.full`, and no stage's tier-3
    // record moved (a job whose tree the result cache already knows runs no command).
    assert_eq!((full_runs(), tier3_records(&h, &id)), before);
    let lines = h.history_lines("stage");
    let got: Vec<(Value, Value, Value)> = lines
        .iter()
        .map(|l| {
            (
                l["stage"].clone(),
                l["outcome"].clone(),
                l["merge_method"].clone(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (json!(1), json!("merged"), json!("merge")),
            (json!(2), json!("merged"), json!("merge")),
        ],
        "{lines:#?}"
    );
    for n in [1, 2] {
        assert_eq!(rig.wait_pr(n, |_| true).state, PrState::Merged);
    }
}
