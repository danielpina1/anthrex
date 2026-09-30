//! Milestone 9.1 task M9.1.22: stages end to end, through the real binary and a real
//! daemon on `/tmp` paths: stage branches and the propagate that flows a late stage-1
//! merge up, a propagate conflict resolved by the engine's `sync` task, and completion
//! running tier 3 on every stage in order. `fake-agent` plays every agent.

mod support;

use std::time::Duration;

use proto::{RunState, TaskOrigin, TaskState};
use serde_json::{Value, json};
use support::run_harness::{REQUEST_WAIT, RunHarness};
use support::run_plans::*;
use support::run_tiers::*;

fn harness() -> RunHarness {
    RunHarness::with_config("", "", &tier_repo_files())
}

fn green(h: &RunHarness, id: &str, steps: &[Value]) {
    h.script(&format!("worker-{id}-1"), steps);
    h.script(&format!("reviewer-{id}-1"), &[approve()]);
}

/// One `sh` step's share of a wait: 1500 polls of 0.2 s, inside `fake-agent`'s
/// `SH_TIMEOUT` (330 s).
const SH_SHARE: Duration = Duration::from_secs(300);

/// Worker steps that wait for `path`, with the turn open, for at least `within`: the
/// test's own bound on everything before it writes `path` (ruling C-26, 1). The wait is
/// as many `sh` steps as `within` needs. A wait that runs out leaves [`missed`]'s
/// marker, which the test asserts is absent, and an `expect` fails the worker (exit 3).
/// The exit alone is not loud enough: the engine resumes a dead worker's session, and
/// the resumed script waits again.
fn wait_for(path: &std::path::Path, within: Duration) -> Vec<Value> {
    let steps = within.as_secs().div_ceil(SH_SHARE.as_secs());
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

/// The marker a [`wait_for`] that ran out leaves beside `path`.
fn missed(path: &std::path::Path) -> std::path::PathBuf {
    path.with_extension("missed")
}

/// The run completed, or cannot: it stopped running (halted, for one), or a task
/// blocked (a worker whose wait failed, for one).
fn settled(r: &proto::RunInfo) -> bool {
    !matches!(
        r.state,
        RunState::Running | RunState::AwaitingApproval | RunState::Planning
    ) || r.tasks.iter().any(|t| t.state == TaskState::Blocked)
}

/// Whether task `id` has finished, merged or otherwise.
fn finished(r: &proto::RunInfo, id: &str) -> bool {
    t(r, id).state.is_finished()
}

fn head(h: &RunHarness, id: &str, branch: &str) -> String {
    h.git(&["rev-parse", &format!("anthrex/{id}/{branch}")])
}

/// Waits until task `task` of run `id` has merged, or the run can no longer get there.
fn wait_merged(h: &RunHarness, id: &str, task: &str) {
    let run = h.wait_run(
        id,
        |r| t(r, task).state == TaskState::Merged || settled(r),
        TIER_WAIT,
    );
    assert_eq!(t(&run, task).state, TaskState::Merged, "{task}");
}

#[test]
fn e2e_two_stages_build_their_branches_and_propagate() {
    let h = harness();
    let go = h.dir.path().join("go");
    green(&h, "t1", &[commit("mods/a/src.txt", "a2\n"), done("a")]);
    // t2 merges into stage 1 late: after stage 2's own task.
    // Everything before the test writes `go` is `wait_merged(t3)`: one `TIER_WAIT`,
    // plus a `REQUEST_WAIT` of slack for the write.
    let mut steps = wait_for(&go, TIER_WAIT + REQUEST_WAIT);
    steps.extend([commit("mods/b/src.txt", "b2\n"), done("b")]);
    green(&h, "t2", &steps);
    green(&h, "t3", &[commit("mods/c/src.txt", "c2\n"), done("c")]);
    let tasks = [
        task("t1", &["mods/a/src.txt"], ""),
        task("t2", &["mods/b/src.txt"], ""),
        task("t3", &["mods/c/src.txt"], "stage = 2"),
    ];
    let id = h.start(&tier_plan(&h, "", &tasks), true);
    wait_merged(&h, &id, "t3");
    // The premise: t2 merges into stage 1 only after stage 2 exists and holds t3.
    let run = h.run(&id).expect("the run");
    assert!(!finished(&run, "t2"), "t2 finished early: {}", report(&run));
    assert!(!missed(&go).exists(), "t2's wait ran out before `go`");
    std::fs::write(&go, "").unwrap();
    // t1 and t3 side by side, then t2 (`k = 2`).
    let run = h.wait_run(&id, settled, 2 * TIER_WAIT);
    assert!(
        run.tasks.iter().all(|t| t.state == TaskState::Merged),
        "{}",
        report(&run)
    );

    let (one, two) = (head(&h, &id, "stage-1"), head(&h, &id, "stage-2"));
    assert_eq!(
        head(&h, &id, "integration"),
        two,
        "integration is stage 2's alias"
    );
    // The last stage-1 merge (t2's) came into stage 2 as a two-parent propagate.
    let propagates = h.git(&[
        "log",
        "--format=%H %P%x09%s",
        &format!("anthrex/{id}/stage-2"),
    ]);
    let late = propagates
        .lines()
        .filter(|l| l.ends_with("\tanthrex: propagate stage-1 into stage-2"))
        .map(|l| l.split('\t').next().unwrap().split(' ').collect::<Vec<_>>())
        .find(|ids| ids.len() == 3 && ids[2] == one)
        .unwrap_or_else(|| panic!("no propagate of {one} into stage 2:\n{propagates}"));
    assert_eq!(late.len(), 3, "a commit and its two parents: {late:?}");
    let b = h.git(&["show", &format!("{two}:mods/b/src.txt")]);
    assert_eq!(b, "b2", "stage 2 holds t2's work");
    let stages: Vec<_> = run.stages.iter().map(|s| (s.n, s.head.clone())).collect();
    assert_eq!(stages, [(1, Some(one)), (2, Some(two))]);
}

#[test]
fn e2e_propagate_conflict_is_resolved_by_a_sync_task() {
    let h = harness();
    let go = h.dir.path().join("go");
    // A plan cannot make the two stages' tasks race for a file: same-runtime tasks
    // whose `owns` overlap are ordered, the later stage after the earlier (ruling
    // C-14 (b)). A task added to stage 1 once stage 2's has started is not, so t3,
    // added after t2 merged, changes the line t2 changed, and its merge into stage 1
    // conflicts when it is propagated. t1 holds the run open until then.
    // Everything before the test writes `go`: `wait_merged(t2)`, the edit request,
    // `wait_merged(t3)`, and the wait for fix1, plus a `REQUEST_WAIT` of slack.
    let mut steps = wait_for(&go, 3 * TIER_WAIT + 2 * REQUEST_WAIT);
    steps.extend([commit("docs/guide.md", "guide 2\n"), done("docs")]);
    green(&h, "t1", &steps);
    green(&h, "t2", &[commit("mods/a/src.txt", "a2\n"), done("a2")]);
    green(&h, "t3", &[commit("mods/a/src.txt", "a1\n"), done("a1")]);
    green(
        &h,
        "fix1",
        &[
            sh(
                "grep -q '^<<<<<<<' mods/a/src.txt && printf 'a1\\na2\\n' > mods/a/src.txt && git add mods/a/src.txt && git commit -qm 'resolve'",
            ),
            done("kept both"),
        ],
    );
    let tasks = [
        task("t1", &["docs/guide.md"], ""),
        task("t2", &["mods/a/src.txt"], "stage = 2"),
    ];
    let id = h.start(&tier_plan(&h, "", &tasks), true);
    wait_merged(&h, &id, "t2");
    let t3 = serde_json::json!({"op": "add_task", "task": {
        "id": "t3", "title": "Task t3", "brief": "Do t3.", "acceptance": ["t3 is done"],
        "owns": ["mods/a/src.txt"], "size": "S", "test_mode": "check",
        "test_mode_reason": "a text file",
    }});
    let reply = h.request(proto::RunRequest::Edit {
        run_id: id.clone(),
        edits: vec![serde_json::from_value(t3).unwrap()],
        submit: false,
    });
    assert!(matches!(reply, proto::RunReply::Done { .. }), "{reply:?}");
    wait_merged(&h, &id, "t3");
    // Fail fast: the conflict either adds the sync task or leaves an attention line.
    let run = h.wait_run(
        &id,
        |r| r.tasks.iter().any(|t| t.id == "fix1") || !r.attention.is_empty() || settled(r),
        TIER_WAIT,
    );
    assert!(
        run.tasks.iter().any(|t| t.id == "fix1"),
        "no sync task: {:?}",
        run.attention
    );
    // The premise: t1 still holds the run open.
    assert!(!finished(&run, "t1"), "t1 finished early: {}", report(&run));
    assert!(!missed(&go).exists(), "t1's wait ran out before `go`");
    std::fs::write(&go, "").unwrap();
    // The sync task, then t1 (`k = 2` after t3's merge).
    let run = h.wait_run(&id, settled, 2 * TIER_WAIT);
    assert_eq!(run.state, RunState::Complete, "{}", report(&run));

    let fix = run
        .tasks
        .iter()
        .find(|t| t.id == "fix1")
        .unwrap_or_else(|| panic!("no sync task: {}", report(&run)));
    assert_eq!(fix.state, TaskState::Merged);
    assert_eq!(fix.origin, TaskOrigin::Sync);
    assert_eq!(
        fix.fixes.as_deref(),
        Some("propagate of stage 1 into stage 2")
    );
    assert_eq!(fix.stage, 2);
    assert_eq!(fix.owns, ["mods/a/src.txt"]);
    let (one, two) = (head(&h, &id, "stage-1"), head(&h, &id, "stage-2"));
    assert_eq!(h.git(&["show", &format!("{two}:mods/a/src.txt")]), "a1\na2");
    // The propagate completed: stage 2 holds stage 1's head.
    h.git(&["merge-base", "--is-ancestor", &one, &two]);
}

#[test]
fn e2e_completion_waits_for_every_stage_green() {
    let h = harness();
    green(&h, "t1", &[commit("mods/a/src.txt", "a2\n"), done("a")]);
    // t2 depends on t1, so stage 2 is created once t1 has merged into stage 1: from
    // stage 1's head, not the run's base (ruling C-26, 5). t2's worker records what
    // its worktree holds of t1's file before it changes anything.
    let seen = h.dir.path().join("seen");
    green(
        &h,
        "t2",
        &[
            sh(&format!("cat mods/a/src.txt > '{}'", seen.display())),
            commit("mods/c/src.txt", "c2\n"),
            done("c"),
        ],
    );
    let tasks = [
        task("t1", &["mods/a/src.txt"], ""),
        task("t2", &["mods/c/src.txt"], "stage = 2\ndeps = [\"t1\"]"),
    ];
    let id = h.start(&tier_plan(&h, "", &tasks), true);
    // t1, then t2 (`k = 2`).
    let run = h.wait_run(&id, settled, 2 * TIER_WAIT);
    assert_eq!(run.state, RunState::Complete, "{}", report(&run));
    let (one, two) = (head(&h, &id, "stage-1"), head(&h, &id, "stage-2"));
    assert_ne!(one, two);

    // Stage 2 started from stage 1's head (t1's merge, stage 1's only one), so t2's
    // worktree held t1's work and no propagate was needed.
    assert_eq!(std::fs::read_to_string(&seen).unwrap(), "a2\n");
    let json = run_json(&run);
    let created: Vec<_> = json["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["created_from"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(created[1], one, "stage 2's base: {created:?}");
    let log = h.git(&["log", "--format=%s", &format!("anthrex/{id}/stage-2")]);
    assert!(!log.contains("anthrex: propagate"), "{log}");

    // Tier 3 ran on stage 1's head, then on stage 2's, in `.full`.
    let full: Vec<String> = runs_of(&tier_log(&h), "check.sh")
        .iter()
        .filter(|l| l["cwd"].as_str().is_some_and(|c| c.ends_with("/.full")))
        .map(|l| l["head"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(full, [one.clone(), two.clone()]);
    for stage in &run.stages {
        assert_eq!(stage.full.state, proto::FullState::Green, "{stage:?}");
        assert_eq!(stage.full.commit, stage.head, "{stage:?}");
    }
}
