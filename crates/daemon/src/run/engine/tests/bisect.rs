//! Milestone 9.1 task M9.1.15: a red tier 3 bisected over the stage's merges with only
//! the failing tests (decisions 35 and 36), the culprit's fix task one rung up
//! (decision 37), no single culprit and the cap (decision 38), and a probe lost in a
//! restart (decision 29). Probes are answered with `Fixture::done`.

use proto::{RunState, TaskOrigin, TaskState, TestMode};

use super::control::resume;
use super::control_restore::restart;
use super::fixture::*;
use super::full::{attention, full_job, later, merge_tiered, outcome, profile, tier, verify_ok};
use super::merge::{commit, doc_task, pending, pending_one, start_on};
use super::wake_notes::notes;
use crate::run::contract::sha7;
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::model::{FixOf, OpId, StageMerge};
use crate::run::proof::proof_command;
use crate::run::roster::escalate;
use crate::run::tiers::TestAtSpec;

pub(super) const TEST: &str = "a::works";

/// Gives the run an orchestrator (so wake notes are kept), its notes empty and its plan
/// submitted.
pub(super) fn with_orchestrator(fx: &mut Fixture) {
    let mut orch = super::orch::launched(false).run().orch.orchestrator.clone();
    // Its plan was submitted, so the run may complete (milestone 9 decision 38).
    orch.as_mut().expect("an orchestrator").plan_submitted = true;
    fx.run_mut().orch.orchestrator = orch;
    super::wake_notes::clear(fx);
}

/// `ids` merged in order at `commit(1)`, `commit(2)`, …, each launched when its turn
/// comes; the windows.
pub(super) fn merged(ids: &[&str], extra: &str) -> Fixture {
    let tasks: Vec<String> = ids.iter().map(|id| doc_task(id, "")).collect();
    let profile = profile().replacen("\n[profile]", &format!("\n{extra}\n[profile]"), 1);
    let (mut fx, mut windows) = start_on(&profile, &tasks);
    for (k, id) in ids.iter().enumerate() {
        merge_next(&mut fx, &mut windows, id, &commit(k as u32 + 1));
    }
    fx
}

/// `id` (launched now if it has no window yet) merges at `at`. A worktree prepared
/// from an older head is prepared again from the new one first, so this launches until
/// the window comes.
pub(super) fn merge_next(fx: &mut Fixture, windows: &mut Vec<(String, u32)>, id: &str, at: &str) {
    for _ in 0..4 {
        if windows.iter().any(|(t, _)| t == id) {
            break;
        }
        windows.extend(fx.launch_all());
    }
    let window = windows.iter().find(|(t, _)| t == id).expect(id).1;
    merge_tiered(fx, id, window, at);
}

/// Completion's tier 3 on stage 1 comes back red on [`TEST`].
pub(super) fn red_full(fx: &mut Fixture) -> Vec<Effect> {
    verify_ok(fx);
    let (op, _) = full_job(fx);
    fx.done(op, tier(outcome(3, &[TEST])))
}

/// The one pending probe.
pub(super) fn probe(fx: &Fixture) -> (OpId, TestAtSpec) {
    match pending_one(fx, "TestAt", None) {
        (op, OpKind::TestAt(spec)) => (op, *spec),
        _ => unreachable!(),
    }
}

/// `commit(n)`'s `n`, 0 for the base.
fn index(commit: &str) -> u32 {
    if commit == BASE {
        return 0;
    }
    let digits: String = commit.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().expect(commit)
}

pub(super) fn probe_result(red: bool, at: &str) -> OpResult {
    OpResult::TestAt {
        red,
        failing: Vec::new(),
        tail: String::new(),
        show: (at != BASE).then(|| format!("show {}", index(at))),
    }
}

/// Answers every probe until none is pending: red at `commit(n)` for `n >= red_from`.
/// The commits probed, in order.
pub(super) fn answer(fx: &mut Fixture, red_from: u32) -> Vec<String> {
    let mut probed = Vec::new();
    while !pending(fx, "TestAt", None).is_empty() {
        let (op, spec) = probe(fx);
        let red = index(&spec.commit) >= red_from;
        fx.done(op, probe_result(red, &spec.commit));
        probed.push(spec.commit);
        assert!(probed.len() <= 12, "the bisect does not end: {probed:?}");
    }
    probed
}

fn logged(fx: &Fixture, text: &str) -> bool {
    fx.run().log.iter().any(|l| l.text == text)
}

/// Six merges, red from `m4` on, bisected to `t4`.
fn bisected() -> Fixture {
    let mut fx = merged(&["t1", "t2", "t3", "t4", "t5", "t6"], "");
    with_orchestrator(&mut fx);
    red_full(&mut fx);
    fx
}

#[test]
fn bisect_finds_the_first_red_merge() {
    let mut fx = bisected();
    assert!(logged(
        &fx,
        "stage 1: tier 3 red (a::works); bisecting 6 merges"
    ));
    let (op, spec) = probe(&fx);
    let dir = fx.run().full_path();
    assert_eq!(spec.dir, dir);
    assert_eq!(spec.commit, BASE, "G first: the stage's creation point");
    assert_eq!(spec.setup, Some("make deps".into()));
    assert_eq!(
        spec.commands,
        [proof_command("cargo test -- --exact {test}", TEST)]
    );
    assert_eq!(spec.timeout_secs, fx.run().profile.check_timeout_secs);
    assert_eq!(fx.run().full_op, Some(op));
    // While it runs: no tier 3, no completion guard, and the stage waits red.
    let effects = later(&mut fx, 10_000);
    assert!(ops_in(&effects, "Tier").is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "VerifyRefs").is_empty(), "{effects:#?}");
    assert_eq!(fx.run().stage(1).unwrap().full.red_at, Some(commit(6)));
    assert_eq!(fx.run().state, RunState::Running);

    let probed = answer(&mut fx, 4);
    let expected: Vec<String> = [BASE.to_string(), commit(6), commit(3), commit(4)].into();
    assert_eq!(probed, expected);
    assert!(
        probed.len() - 2 <= 3,
        "at most ceil(log2 6) probes after G and H"
    );
    assert!(logged(
        &fx,
        "stage 1: bisected to t4 in 4 probes; added fix task fix1"
    ));
    let fix = fx.task("fix1");
    assert_eq!(
        fix.fixes,
        Some(FixOf::Bisect {
            culprit: "t4".into(),
            stage: 1,
            tests: vec![TEST.into()],
        })
    );
    let stage = fx.run().stage(1).unwrap();
    assert_eq!(stage.bisect, None);
    assert_eq!(stage.full.bisect_fixes, 1);
    assert_eq!(fx.run().full_op, None);
}

#[test]
fn bisect_fix_task_takes_the_culprits_owns_and_the_route_one_rung_up() {
    let mut fx = bisected();
    answer(&mut fx, 4);
    let culprit = fx.task("t4").clone();
    assert_eq!(culprit.state, TaskState::Merged, "the culprit stays merged");
    let fix = fx.task("fix1").clone();
    assert_eq!(fix.origin, TaskOrigin::Bisect);
    assert_eq!(fix.spec.owns, culprit.spec.owns);
    assert_eq!(fix.spec.owns, ["docs/t4/**"]);
    let up = escalate(&fx.run().roster, &culprit.route);
    assert_ne!(up, culprit.route, "a rung up");
    assert_eq!(fix.route, up);
    assert_eq!(fix.test_mode, TestMode::Check);
    assert_eq!(
        fix.spec.test_mode_reason.as_deref(),
        Some("the failing tests already exist; they must pass")
    );
    assert_eq!(fix.spec.priority, 100);
    assert_eq!((fix.stage(), fix.size), (1, culprit.size));
    assert!(fix.spec.deps.is_empty());
    assert_eq!(fix.spec.epic, culprit.spec.epic);
    assert!(!fix.state.is_finished(), "{:?}", fix.state);
    assert_eq!(fix.spec.title, "Fix a::works after t4");
    assert_eq!(
        fix.spec.acceptance,
        ["a::works passes", "no test is weakened, skipped or deleted"]
    );
    let summary = crate::run::exec::summary("FAILED");
    let brief = format!(
        "[anthrex] Fix task fix1: the full test suite of stage 1 fails, and bisecting the stage's merges found that it started failing with the merge of task t4 (\"{title}\").\n\
         Failing tests: a::works\n\
         Make these tests pass without weakening them. t4's work stays merged; fix it here.\n\
         \n\
         Summary of the failure:\n\
         {summary}\n\
         \n\
         The merge that introduced it:\n\
         show 4\n\
         \n\
         The brief of t4:\n\
         {brief}",
        title = culprit.spec.title,
        brief = culprit.spec.brief,
    );
    assert_eq!(fix.spec.brief, brief);
    assert_eq!(fix.branch, format!("anthrex/{RUN_ID}/fix1"));
    assert_eq!(fix.worktree, super::dispatch::task_path("fix1"));
    assert_eq!(
        notes(&fx),
        ["stage 1 tier 3 red (a::works): bisected to t4; added fix task fix1"]
    );
    assert!(
        !attention(&fx).iter().any(|l| l.contains("tier 3")),
        "{:?}",
        attention(&fx)
    );
    // It runs like any task: the next pass dispatches it.
    fx.tick();
    assert!(
        fx.launch_all().iter().any(|(t, _)| t == "fix1"),
        "{:#?}",
        fx.task("fix1").state
    );
}

#[test]
fn red_at_the_base_is_not_bisected() {
    let mut fx = merged(&["t1", "t2"], "");
    with_orchestrator(&mut fx);
    red_full(&mut fx);
    let probed = answer(&mut fx, 0);
    assert_eq!(probed, [BASE.to_string()]);
    assert!(fx.run().task("fix1").is_none());
    let line = format!(
        "tier 3 red, no single culprit: a::works (stage 1: the failing tests already fail at {})",
        sha7(BASE)
    );
    assert!(attention(&fx).contains(&line), "{:?}", attention(&fx));
    assert_eq!(
        notes(&fx),
        ["stage 1 tier 3 red, no single culprit: a::works; plan a fix"]
    );
    let stage = fx.run().stage(1).unwrap();
    assert_eq!(
        (stage.bisect.clone(), stage.full.red_at.clone()),
        (None, Some(commit(2)))
    );
    // The run waits red: no guard, no tier 3.
    let effects = later(&mut fx, 10_000);
    assert!(ops_in(&effects, "VerifyRefs").is_empty() && ops_in(&effects, "Tier").is_empty());
    assert_eq!(fx.run().state, RunState::Running);
}

#[test]
fn no_culprit_wakes_the_orchestrator_with_the_attention_line() {
    // H green: red twice is not reproduced by the failing tests alone.
    let mut fx = merged(&["t1", "t2"], "");
    with_orchestrator(&mut fx);
    red_full(&mut fx);
    let probed = answer(&mut fx, 99);
    assert_eq!(probed, [BASE.to_string(), commit(2)]);
    let line = format!(
        "tier 3 red, no single culprit: a::works (stage 1: the failing tests pass alone at {}; they fail only with the whole suite)",
        sha7(&commit(2))
    );
    assert!(attention(&fx).contains(&line), "{:?}", attention(&fx));
    assert_eq!(
        notes(&fx),
        ["stage 1 tier 3 red, no single culprit: a::works; plan a fix"]
    );
    assert!(fx.run().task("fix1").is_none());
    // A probe's pass decides nothing by itself (ruling C-7): the stage stays red.
    assert_eq!(fx.run().stage(1).unwrap().full.red_at, Some(commit(2)));
    assert_eq!(fx.run().stage(1).unwrap().full.green_at, None);

    // The first red merge is a propagate: no single culprit.
    let mut fx = merged(&["t1", "t2"], "");
    with_orchestrator(&mut fx);
    let stage = &mut fx.run_mut().stages[0];
    stage.merges.push(StageMerge::Propagate {
        from: 1,
        commit: commit(3),
    });
    set_stage_head(fx.run_mut(), 1, &commit(3));
    fx.run_mut().last_green_candidate = Some(commit(3));
    red_full(&mut fx);
    let probed = answer(&mut fx, 3);
    assert_eq!(probed, [BASE.to_string(), commit(3), commit(1), commit(2)]);
    let line = "tier 3 red, no single culprit: a::works (stage 1: the first red merge is the propagate of stage 1)";
    assert!(
        attention(&fx).contains(&line.to_string()),
        "{:?}",
        attention(&fx)
    );
    assert_eq!(
        notes(&fx),
        ["stage 1 tier 3 red, no single culprit: a::works; plan a fix"]
    );
    assert!(fx.run().task("fix1").is_none());
}

#[test]
fn after_bisect_fix_max_the_next_red_goes_to_the_user() {
    let tasks = [doc_task("t1", ""), doc_task("t2", "")];
    let (mut fx, mut windows) = start_on(&profile(), &tasks);
    fx.run_mut().limits.testing.bisect_fix_max = 2;
    with_orchestrator(&mut fx);
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    merge_next(&mut fx, &mut windows, "t2", &commit(2));
    red_full(&mut fx);
    answer(&mut fx, 2);
    assert!(fx.run().task("fix1").is_some());
    merge_next(&mut fx, &mut windows, "fix1", &commit(3));
    red_full(&mut fx);
    let probed = answer(&mut fx, 2);
    assert_eq!(
        probed.first(),
        Some(&BASE.to_string()),
        "G is still the base"
    );
    assert!(fx.run().task("fix2").is_some());
    assert_eq!(fx.run().stage(1).unwrap().full.bisect_fixes, 2);
    merge_next(&mut fx, &mut windows, "fix2", &commit(4));
    super::wake_notes::clear(&mut fx);

    red_full(&mut fx);
    assert!(pending(&fx, "TestAt", None).is_empty(), "no bisect");
    assert!(fx.run().task("fix3").is_none());
    let line = "stage 1: tier 3 still red after 2 fix tasks: a::works; fix it with a task, or end the run with the finish edit";
    assert!(
        attention(&fx).contains(&line.to_string()),
        "{:?}",
        attention(&fx)
    );
    assert_eq!(notes(&fx), [line]);
    assert_eq!(fx.run().stage(1).unwrap().full.red_at, Some(commit(4)));
}

#[test]
fn bisect_survives_a_restart() {
    let mut fx = merged(&["t1", "t2", "t3"], "");
    red_full(&mut fx);
    let (op, spec) = probe(&fx);
    fx.done(op, probe_result(false, &spec.commit));
    let (lost, at_head) = probe(&fx);
    assert_eq!(at_head.commit, commit(3));
    // `run.json` holds the bisect.
    let text = serde_json::to_string(fx.run()).unwrap();
    let stored: crate::run::model::Run = serde_json::from_str(&text).unwrap();
    assert_eq!(stored.stages[0].bisect, fx.run().stages[0].bisect);
    *fx.run_mut() = stored;
    restart(&mut fx, Vec::new());
    let bisect = fx.run().stage(1).unwrap().bisect.clone().expect("kept");
    assert_eq!((bisect.probe, bisect.probes), (None, 1));
    assert_eq!(fx.run().full_op, None);
    let effects = resume(&mut fx);
    let probes = ops_in(&effects, "TestAt");
    assert_eq!(probes.len(), 1, "{effects:#?}");
    let (again, kind) = probes[0].clone();
    assert_ne!(again, lost);
    assert!(matches!(&kind, OpKind::TestAt(s) if s.commit == commit(3)));
    assert_eq!(fx.run().full_op, Some(again));
    let probed = answer(&mut fx, 2);
    assert_eq!(probed, [commit(3), commit(1), commit(2)]);
    assert_eq!(
        fx.task("fix1").fixes,
        Some(FixOf::Bisect {
            culprit: "t2".into(),
            stage: 1,
            tests: vec![TEST.into()],
        })
    );
}

// Ruling C-18 for probes, and the run ending during a bisect.
#[path = "bisect_ends.rs"]
mod ends;

// Controller ruling C-19 and the review's other findings on c21b745.
#[path = "bisect_review.rs"]
mod review;
