//! Milestone 9.2 task M9.2.9, decision 27 step 4: a CI red reproduced at the PR head
//! with 9.1's probe (only the failing tests, `build_check` for a build or lint red),
//! bisected on a tiered profile (a culprit's fix one rung up, `bisect_fix_max`
//! untouched), and otherwise a stage fix with the stage's strongest route, or TT's
//! environment fix when it does not reproduce. Probes are answered with
//! `Fixture::done`.

use proto::{CiCategory, Route, RunState, TaskOrigin};

use super::bisect::{TEST, merge_next, probe, with_orchestrator};
use super::control::resume;
use super::control_restore::restart;
use super::delivery_ci::{
    RUN_A, answer_logs, ci_decide, ci_fixes, ci_record, deciding, logged, red_view, summary,
    test_red, tier2,
};
use super::delivery_open::{answer, green, host_op, open_stage, pr_on};
use super::delivery_watch::{PR, fast, poll_with, view, watched};
use super::fixture::*;
use super::full::{attention, outcome, profile, tier};
use super::merge::{commit, doc_task, merge, pending, to_queue, window_of};
use super::wake_notes::notes;
use crate::host::{FetchOutcome, PrView};
use crate::run::contract::sha7;
use crate::run::delivery::CiPhase;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::delivery::DeliveryRequest;
use crate::run::engine::{EventKind, OpResult};
use crate::run::model::FixOf;
use crate::run::proof::proof_command;
use crate::run::roster::escalate;

const SINGLE: &str = "cargo test -- --exact {test}";

/// A `pr`-mode run of `ids` on the tiered profile, merged in order at `commit(1)`, …,
/// green on tier 3 at its head, with PR #7 open on it.
fn tiered_watched(ids: &[&str]) -> Fixture {
    let tasks: Vec<String> = ids.iter().map(|id| doc_task(id, "")).collect();
    let (mut fx, mut windows) = pr_on(&profile(), &tasks);
    fast(fx.run_mut());
    for (k, id) in ids.iter().enumerate() {
        merge_next(&mut fx, &mut windows, id, &commit(k as u32 + 1));
    }
    green(&mut fx, 1);
    open_stage(&mut fx, 1, PR);
    fx
}

/// A red on `head` summarised with `tests` and `category` by the decider.
fn red_summarised(fx: &mut Fixture, head: &str, tests: &[&str], category: CiCategory) {
    deciding(fx);
    poll_with(fx, red_view(head, test_red(RUN_A)));
    answer_logs(fx, "--- FAIL: a::works");
    let (op, _) = ci_decide(fx);
    fx.decided(op, summary(&["boom"], tests, category));
}

/// The one pending reproduction probe.
fn reproduction(fx: &Fixture) -> (crate::run::model::OpId, crate::run::tiers::TestAtSpec) {
    probe(fx)
}

fn red_probe(command: &str) -> OpResult {
    OpResult::TestAt {
        red: true,
        failing: vec![command.into()],
        tail: "FAILED".into(),
        show: None,
    }
}

#[test]
fn reproduce_runs_only_the_failing_tests_at_the_pr_head() {
    let mut fx = watched();
    let names: Vec<String> = (1..=12).map(|k| format!("a::t{k:02}")).collect();
    let mut tests: Vec<&str> = names.iter().map(String::as_str).collect();
    tests.insert(3, "a;rm -rf /");
    red_summarised(&mut fx, &commit(1), &tests, CiCategory::Test);
    let (op, spec) = reproduction(&fx);
    assert_eq!(spec.commit, commit(1), "at the PR head");
    assert_eq!(spec.dir, fx.run().full_path());
    assert_eq!(spec.setup, Some("make deps".into()));
    // `single_test` once per name, the first ten, never the whole suite.
    let want: Vec<String> = names[..10]
        .iter()
        .map(|t| proof_command(SINGLE, t))
        .collect();
    assert_eq!(spec.commands, want);
    assert!(!spec.commands.iter().any(|c| c == "cargo test"));
    assert_eq!(fx.run().full_op, Some(op), "it holds the one tier-3 slot");
    assert!(tier2(&fx).is_none());
    assert_eq!(ci_record(&fx).probe, Some(op));
    // A probe lost in a restart is issued again once the run runs.
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Paused);
    resume(&mut fx);
    fx.tick();
    let (again, spec) = reproduction(&fx);
    assert_ne!(again, op);
    assert_eq!(spec.commands, want);
    assert_eq!(fx.run().full_op, Some(again));
    // Red on an untiered profile: no bisect, a stage fix that names the command.
    let failing = proof_command(SINGLE, "a::t01");
    fx.done(again, red_probe(&failing));
    assert_eq!(fx.run().full_op, None);
    let fix = ci_fixes(&fx).pop().expect("a fix task");
    let brief = &fx.task(&fix).spec.brief;
    assert!(
        brief.contains(&format!(
            "Category: test. It reproduces locally with: {failing}.\n"
        )),
        "{brief}"
    );
    assert!(brief.contains("\nNo single task's merge is the cause.\n"));
    assert_eq!(fx.task(&fix).spec.title, "Fix CI on stage 1: a::t01");
}

#[test]
fn build_and_lint_reproduce_with_build_check() {
    for (tiered, category, want) in [
        (true, CiCategory::Build, "cargo build"),
        (true, CiCategory::Lint, "cargo build"),
        // No `build_check`: the profile's `check`.
        (false, CiCategory::Build, "cargo test"),
    ] {
        let mut fx = if tiered {
            tiered_watched(&["t1"])
        } else {
            watched()
        };
        let head = fx.run().stage_head(1).unwrap().to_string();
        red_summarised(&mut fx, &head, &[], category);
        let (_, spec) = reproduction(&fx);
        assert_eq!(spec.commands, [want], "{category:?}");
        assert_eq!(spec.commit, head);
    }
}

#[test]
fn reproduced_with_a_culprit_adds_a_ci_fix_with_the_culprits_owns_one_rung_up() {
    let mut fx = tiered_watched(&["t1", "t2", "t3", "t4"]);
    with_orchestrator(&mut fx);
    // Tier 3 was last green at m1 on the stage's line (the PR head is m4).
    fx.run_mut().stages[0].full.green_at = Some(commit(1));
    red_summarised(&mut fx, &commit(4), &[TEST], CiCategory::Test);
    let (op, spec) = reproduction(&fx);
    assert_eq!(spec.commands, [proof_command(SINGLE, TEST)]);
    fx.done(op, red_probe(&spec.commands[0]));
    // 9.1's bisect, marked as serving this CI red.
    let line = format!(
        "stage 1: CI red at {} reproduces; bisecting 3 merges",
        sha7(&commit(4))
    );
    assert!(logged(&fx, &line), "{:#?}", fx.run().log);
    let b = fx.run().stage(1).unwrap().bisect.clone().expect("a bisect");
    assert_eq!(b.ci, Some((1, TEST.to_string())));
    assert_eq!(ci_record(&fx).phase, CiPhase::Bisecting);
    super::bisect::answer(&mut fx, 3);
    // The culprit's fix: its owns exactly, its route one rung up, origin `ci`.
    let fixes = ci_fixes(&fx);
    assert_eq!(fixes.len(), 1, "{fixes:?}");
    let fix = fx.task(&fixes[0]).clone();
    let culprit = fx.task("t3").clone();
    assert_eq!(fix.origin, TaskOrigin::Ci);
    assert_eq!(fix.spec.owns, culprit.spec.owns);
    assert_eq!(fix.route, escalate(&fx.run().roster, &culprit.route));
    assert_eq!(
        fix.fixes,
        Some(FixOf::Ci {
            stage: 1,
            head: commit(4),
            ci_runs: vec![RUN_A],
            key: TEST.into(),
        })
    );
    let brief = &fix.spec.brief;
    assert!(brief.contains(&format!(
        "\nBisect found the merge of task t3 ({}) as the first red; its brief follows.\n",
        culprit.spec.title
    )));
    assert!(brief.contains(&format!("\nTask t3's brief:\n{}\n", culprit.spec.brief)));
    assert!(
        brief.contains("\ngit show --stat of its merge:\nshow 3\n"),
        "{brief}"
    );
    // `bisect_fix_max` is not charged; 9.1's end and wake note are replaced.
    let s = fx.run().stage(1).unwrap();
    assert_eq!(s.full.bisect_fixes, 0);
    assert!(s.bisect.is_none());
    let added = format!("stage 1 CI red (test): fix task {} added", fixes[0]);
    assert_eq!(notes(&fx), [added]);
    assert!(attention(&fx).is_empty(), "{:#?}", attention(&fx));
    let rec = ci_record(&fx);
    assert_eq!(
        (rec.phase, rec.fix_task),
        (CiPhase::Tasked, Some(fixes[0].clone()))
    );
}

/// The roster's weakest and strongest routes, at `effort`.
fn routes(fx: &Fixture) -> (Route, Route) {
    let roster = &fx.run().roster;
    let effort = fx.task("t1").route.effort;
    let route = |e: &proto::ModelEntry| Route {
        runtime: e.runtime,
        model: e.model.clone(),
        strength: e.strength,
        effort,
    };
    let weak = roster.iter().min_by_key(|e| e.strength).unwrap();
    let strong = roster.iter().max_by_key(|e| e.strength).unwrap();
    assert!(weak.strength < strong.strength, "{roster:#?}");
    (route(weak), route(strong))
}

/// A stage fix's checks: the union of the stage's owns and the strongest route.
fn assert_stage_fix(fx: &Fixture, owns: &[&str], route: &Route) -> String {
    let fix = ci_fixes(fx).pop().expect("a stage fix");
    let task = fx.task(&fix);
    assert_eq!(task.spec.owns, owns);
    assert_eq!(&task.route, route);
    assert!(
        task.spec
            .brief
            .contains("\nNo single task's merge is the cause.\n")
    );
    assert_eq!(fx.run().stage(1).unwrap().full.bisect_fixes, 0);
    fix
}

#[test]
fn reproduced_without_a_culprit_adds_a_stage_fix_with_the_strongest_route() {
    // An untiered profile: nothing is bisected.
    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", ""), doc_task("t2", "")]);
    fast(fx.run_mut());
    for (k, id) in ["t1", "t2"].iter().enumerate() {
        to_queue(&mut fx, id, window_of(&windows, id));
        merge(&mut fx, id, &commit(k as u32 + 1));
    }
    green(&mut fx, 1);
    open_stage(&mut fx, 1, PR);
    let (weak, strong) = routes(&fx);
    fx.task_mut("t1").route = weak;
    fx.task_mut("t2").route = strong.clone();
    // Deciders off: the fallback's `unknown`, reproduced with tier 2 at the PR head.
    poll_with(&mut fx, red_view(&commit(2), test_red(RUN_A)));
    answer_logs(&mut fx, "boom");
    let (op, spec) = tier2(&fx).expect("tier 2");
    assert_eq!(
        (spec.head.as_str(), spec.dir.clone()),
        (commit(2).as_str(), fx.run().full_path())
    );
    assert_eq!(
        spec.scratch.as_ref().map(|s| s.commit.clone()),
        Some(commit(2))
    );
    fx.done(op, tier(outcome(2, &[TEST])));
    let fix = assert_stage_fix(&fx, &["docs/t1/**", "docs/t2/**"], &strong);
    let brief = &fx.task(&fix).spec.brief;
    assert!(
        brief.contains("Category: unknown. It reproduces locally with: cargo test.\n"),
        "{brief}"
    );

    // A tiered profile whose bisect finds no single culprit (red at G already).
    let mut fx = tiered_watched(&["t1", "t2"]);
    fx.run_mut().stages[0].full.green_at = Some(commit(1));
    let (_, strong) = routes(&fx);
    fx.task_mut("t2").route = strong.clone();
    red_summarised(&mut fx, &commit(2), &[TEST], CiCategory::Test);
    let (op, spec) = reproduction(&fx);
    fx.done(op, red_probe(&spec.commands[0]));
    assert!(fx.run().stage(1).unwrap().bisect.is_some());
    super::bisect::answer(&mut fx, 0);
    assert_stage_fix(&fx, &["docs/t1/**", "docs/t2/**"], &strong);
    assert!(
        attention(&fx).is_empty(),
        "9.1's no-culprit line is replaced: {:#?}",
        attention(&fx)
    );

    // A user's commit adopted onto the stage (decision 24, ruling R-9): it is the floor,
    // so a red there blames no task.
    let mut fx = tiered_watched(&["t1"]);
    let route = fx.task("t1").route.clone();
    poll_with(&mut fx, PrView { ..view(&commit(9)) });
    let (op, fetch) = host_op(&fx);
    assert!(matches!(fetch, HostOp::Fetch { .. }), "{fetch:?}");
    answer(
        &mut fx,
        op,
        HostResult::Fetched(FetchOutcome::Adopted { sha: commit(9) }),
    );
    assert_eq!(fx.run().stage_head(1), Some(commit(9).as_str()));
    red_summarised(&mut fx, &commit(9), &[TEST], CiCategory::Test);
    let (op, spec) = reproduction(&fx);
    assert_eq!(spec.commit, commit(9));
    fx.done(op, red_probe(&spec.commands[0]));
    let line = format!(
        "stage 1: CI red at {0} reproduces; not bisected: red before the rebaselined head {0}; not bisected",
        sha7(&commit(9))
    );
    assert!(logged(&fx, &line), "{:#?}", fx.run().log);
    assert!(fx.run().stage(1).unwrap().bisect.is_none());
    assert_stage_fix(&fx, &["docs/t1/**"], &route);
}

#[test]
fn not_reproduced_adds_an_environment_fix_task_with_tts_sentence() {
    let mut fx = watched();
    with_orchestrator(&mut fx);
    let route = fx.task("t1").route.clone();
    poll_with(&mut fx, red_view(&commit(1), test_red(RUN_A)));
    answer_logs(&mut fx, "works on my machine");
    let (op, _) = tier2(&fx).expect("tier 2");
    fx.done(op, tier(outcome(2, &[])));
    let line = format!(
        "stage 1: CI red at {} does not reproduce locally",
        sha7(&commit(1))
    );
    assert!(logged(&fx, &line));
    let fix = assert_stage_fix(&fx, &["docs/t1/**"], &route);
    let brief = &fx.task(&fix).spec.brief;
    assert!(brief.contains(
        "\nCategory: unknown. This failure does not reproduce locally; the difference is in CI's environment. Find it.\n"
    ), "{brief}");
    // The fallback summary says where it came from.
    assert!(brief.contains("(a decider's summary of the log, fallback (deciders are off)):\n"));
    assert_eq!(
        notes(&fx),
        [format!("stage 1 CI red (unknown): fix task {fix} added")]
    );
    // A `run deliver` of an open PR changes nothing here; the record is tasked.
    let reply = fx.reply();
    let request = DeliveryRequest::Deliver {
        reply,
        run_id: RUN_ID.into(),
        stage: 1,
    };
    fx.next(EventKind::Delivery(request));
    assert_eq!(ci_record(&fx).phase, CiPhase::Tasked);
    assert!(pending(&fx, "TestAt", None).is_empty());
}
