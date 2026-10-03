//! Milestone 9.3 task M9.3.11: rounds of a `pr` run end to end (KG §2.5, decision 13,
//! D15 and D17), through the real binary and a real daemon on temporary paths against
//! `FakeHost` (`PrRig`), with `fake-agent` as every agent; no real agent, no `gh`, no
//! GitHub. A round's stage PR stacks on the open one; once every earlier PR has landed
//! (the user's merge, simulated by `FakeGithubCtl`) the round's stage starts from the
//! fetched base; and a delivered run's orchestrator continues with the next goal.
//! anthrex never merges anything: every `PrRig` drop asserts `forbidden.jsonl` is empty.

mod support;

use std::time::Duration;

use daemon::host::Conclusion;
use daemon::host::fake::{CiRule, FakePr, MergeMethodArg};
use proto::{PrState, RunState};
use serde_json::{Value, json};
use support::orch_script::*;
use support::run_harness::{REQUEST_WAIT, RunHarness};
use support::run_orch::ORCH_WAIT;
use support::run_pr::*;
use support::run_rounds::*;

/// A round's stage PR after every earlier PR landed (`docs/timing-budgets.md`, "Recorded,
/// from M9.3.11"): the base fetch the iterate made due ([`FETCH_WAIT`]), the base sync
/// into the new stage, bounded as one task path with its tier commands
/// ([`support::run_tiers::TIER_WAIT`]), then the stage's own way to an open PR
/// ([`PR_OPEN_WAIT`]: its task, tier 3, push, open and view).
const ROUND_PR_WAIT: Duration = PR_OPEN_WAIT
    .saturating_add(FETCH_WAIT)
    .saturating_add(support::run_tiers::TIER_WAIT);

fn stage_branch(id: &str, n: u16) -> String {
    format!("anthrex/{id}/stage-{n}")
}

/// A planned `pr` run's harness with green CI and the green scripts of `t1` (`a.txt`)
/// and `t2` (`b.txt`); the orchestrator runs `steps`. The run, approved, with stage 1's
/// PR open and green.
fn started(steps: &[Value]) -> (RunHarness, PrRig, String) {
    let (h, rig) = orch_pr_harness();
    rig.ctl()
        .set_ci(vec![CiRule::new("build", Conclusion::Success)]);
    green(&h, "t1", "a.txt");
    green(&h, "t2", "b.txt");
    h.script(ORCH, steps);
    let id = pr_goal(&h, "add a");
    h.approve_round(&id, 1);
    rig.wait_stage(&h, &id, 1, "/state", &json!("open"));
    rig.wait_stage(&h, &id, 1, "/ci", &json!("green"));
    (h, rig, id)
}

/// Round 1 plans `t1`; after the user's iterate (the script polls for round 2, which
/// clears the notes) it reads the round wake and plans `t2` in stage 2.
fn round_steps(wait_for_round: Duration) -> Vec<Value> {
    vec![
        prompt(),
        edit_plan(vec![staged("t1", "a.txt", 1)], json!({"submit": true})),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        until("/run/round", json!(2), wait_for_round),
        read(Some("the user asks for round 2 of run ")),
        edit_plan(vec![staged("t2", "b.txt", 2)], json!({"submit": true})),
        marker(),
        read(None),
    ]
}

/// `run iterate <run>` with [`REQUEST`], accepted.
fn iterate(h: &RunHarness, id: &str) {
    let out = h.iterate(id, REQUEST, "");
    ok(&out);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("run {} round 2 started; its orchestrator plans it", h4(id))
    );
}

/// What a PR shows on GitHub that anthrex could change.
fn shown(pr: &FakePr) -> (String, String, String, String, String, PrState) {
    let p = pr.clone();
    (p.base, p.head, p.head_oid, p.title, p.body, p.state)
}

#[test]
fn e2e_pr_round_stacks_new_stage_prs_on_the_open_ones() {
    let steps = round_steps(PR_OPEN_WAIT.saturating_add(REQUEST_WAIT));
    let (h, rig, id) = started(&steps);
    let one = shown(&rig.wait_pr(1, |_| true));
    let edits = rig.calls_of(&["pr", "edit"]).len();

    iterate(&h, &id);
    h.approve_round(&id, 2);
    rig.wait_stage(&h, &id, 2, "/state", &json!("open"));
    let two = rig.wait_pr(2, |_| true);
    assert_eq!(two.base, stage_branch(&id, 1), "stacked on stage 1's PR");
    assert_eq!(two.head, stage_branch(&id, 2));
    assert!(rig.contains(&two.head_oid, &one.2), "stage 2 holds stage 1");
    // PR #1 is untouched: nothing anthrex could change on it moved, and no edit was
    // sent to any PR.
    assert_eq!(shown(&rig.wait_pr(1, |_| true)), one);
    assert_eq!(rig.calls_of(&["pr", "edit"]).len(), edits);
    let texts = texts(&h);
    assert_eq!(
        texts[0].lines().next(),
        Some(round_wake_head(&id, 2, 1).as_str())
    );
    let run = h.run(&id).unwrap();
    assert_eq!(run.state, RunState::Running, "{:?}", run.halted_reason);
    assert_eq!(run.rounds.len(), 2);
}

#[test]
fn e2e_pr_round_after_every_pr_landed_starts_from_the_fetched_base() {
    let wait = PR_OPEN_WAIT
        .saturating_add(LAND_WAIT)
        .saturating_add(REQUEST_WAIT);
    let (h, rig, id) = started(&round_steps(wait));
    let top = rig.wait_pr(1, |_| true).head_oid;

    // The user merges stage 1's PR on GitHub (the fake's merge; anthrex never merges):
    // every PR has landed, and the run is complete.
    rig.ctl().merge(1, MergeMethodArg::Merge, false);
    let done = h.wait_run(
        &id,
        |r| r.state == RunState::Complete || r.halted_reason.is_some(),
        LAND_WAIT,
    );
    assert_eq!(done.state, RunState::Complete, "{:?}", done.halted_reason);
    // The base moves again after completion: only a fetch made at the iterate sees it.
    let base = rig
        .ctl()
        .commit("main", "c.txt", "c\n", "a user's change on main");

    iterate(&h, &id);
    h.approve_round(&id, 2);
    rig.wait_stage_within(&h, &id, 2, "/state", &json!("open"), ROUND_PR_WAIT);
    let two = rig.wait_pr(2, |_| true);
    assert_eq!(two.base, "main", "nothing open below it");
    assert_eq!(two.head, stage_branch(&id, 2));

    // The fetch answered before the stage was created, from the top stage's head, and
    // the base sync merged the fetched base into it before any task merged.
    let log = log_lines(&h, &id);
    let at = |line: &str| {
        log.iter()
            .position(|l| l.starts_with(line))
            .unwrap_or_else(|| panic!("no log line {line:?}: {log:#?}"))
    };
    let due = at(&format!(
        "stage 2: main moved to {}; merging it into stage 2",
        &base[..7]
    ));
    let created = at(&format!(
        "stage 2: {} at {}",
        stage_branch(&id, 2),
        &top[..7]
    ));
    let synced = at(&format!("stage 2: merged main@{} (", &base[..7]));
    assert!(due < created && created < synced, "{log:#?}");
    let range = format!("{}..{}", top, two.head_oid);
    let firsts = rig.bare_git(&["rev-list", "--first-parent", &range]);
    let firsts: Vec<&str> = firsts.lines().collect();
    assert!(
        firsts.len() >= 2,
        "the base sync and t2's merge: {firsts:?}"
    );
    let sync = firsts.last().unwrap();
    let parents = rig.bare_git(&["rev-list", "--parents", "-n", "1", sync]);
    let parents: Vec<&str> = parents.split_whitespace().skip(1).collect();
    assert_eq!(
        parents,
        [top.as_str(), base.as_str()],
        "the base sync's merge"
    );

    // `integration` moved with the top stage: no "moved" halt.
    let run = h.run(&id).unwrap();
    assert_eq!(run.state, RunState::Running, "{:?}", run.halted_reason);
    assert_eq!(
        h.git(&["rev-parse", &format!("refs/heads/{}", run.run_branch)]),
        h.git(&["rev-parse", &format!("refs/heads/{}", stage_branch(&id, 2))])
    );
    let texts = texts(&h);
    assert_eq!(
        texts[0].lines().next(),
        Some(round_wake_head(&id, 2, 1).as_str())
    );
}

#[test]
fn e2e_a_delivered_pr_runs_orchestrator_continues_with_the_next_goal() {
    let steps = [
        prompt(),
        edit_plan(vec![staged("t1", "a.txt", 1)], json!({"submit": true})),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        until(
            "/run/state",
            json!("complete"),
            PR_OPEN_WAIT.saturating_add(LAND_WAIT),
        ),
        read(Some("a new goal, run ")),
        marker(),
        read(None),
    ];
    let (h, rig, id) = started(&steps);
    let window = h.orchestrator_window(&id);
    rig.ctl().merge(1, MergeMethodArg::Merge, false);
    h.wait_saw(ORCH, "/run/state", json!("complete"), LAND_WAIT);
    assert_eq!(h.run(&id).unwrap().state, RunState::Complete);

    let next = h.continue_goal("next", &id);
    h.wait_log(
        "the orchestrator to read the next-goal wake",
        |log| passed(log, ORCH) >= 1,
        CONTINUE_WAIT,
    );
    let texts = texts(&h);
    assert_eq!(
        texts,
        [format!(
            "{}\n{}",
            next_goal_head(&next, &id, "delivered"),
            fenced("next")
        )]
    );
    assert_eq!(h.orchestrator_window(&next), window);
    let chain = format!("o-{}", h4(&id));
    assert_eq!(h.run(&next).unwrap().chain.as_deref(), Some(chain.as_str()));
}
