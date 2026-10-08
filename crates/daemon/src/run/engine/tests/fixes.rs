//! Milestone 9.1 task M9.1.15: engine-made fix tasks (decisions 37 and 39): ids that
//! count up across origins from `Run.fix_seq`, and a fix task M8a's rules refuse.

use proto::{RouteSpec, Size, TaskOrigin, TaskState, TestMode};

use super::fixture::*;
use super::merge::{doc_task, start_on};
use crate::run::engine::fixes::{FixSpec, add_fix, next_fix_id};
use crate::run::model::FixOf;
use crate::run::model_roles::peer;

pub(super) fn spec(origin: TaskOrigin, owns: &str, route: RouteSpec) -> FixSpec {
    let fixes = match origin {
        TaskOrigin::Sync => FixOf::Propagate {
            from: 1,
            to: 2,
            head: BASE.into(),
        },
        _ => FixOf::Bisect {
            culprit: "t1".into(),
            stage: 1,
            tests: vec!["a::works".into()],
        },
    };
    FixSpec {
        origin,
        fixes,
        stage: 1,
        title: "Fix it".into(),
        brief: "Fix the thing.".into(),
        acceptance: vec!["it passes".into()],
        owns: vec![owns.into()],
        size: Size::S,
        epic: None,
        route,
        test_mode: TestMode::Check,
        test_mode_reason: Some("glue".into()),
        sync: None,
    }
}

#[test]
fn fix_ids_count_up_across_origins() {
    let (mut fx, _) = start_on(&super::full::profile(), &[doc_task("t1", "")]);
    assert_eq!(fx.run().fix_seq, 0);
    assert_eq!(next_fix_id(fx.run()), "fix1");
    let now = fx.now;
    let mut effects = Vec::new();
    let a = add_fix(
        fx.run_mut(),
        spec(TaskOrigin::Bisect, "docs/x/**", RouteSpec::default()),
        now,
        &mut effects,
    );
    assert_eq!(a.as_deref(), Ok("fix1"));
    let b = add_fix(
        fx.run_mut(),
        spec(TaskOrigin::Sync, "docs/y.md", RouteSpec::default()),
        now,
        &mut effects,
    );
    assert_eq!(b.as_deref(), Ok("fix2"));
    assert_eq!(fx.run().fix_seq, 3);
    assert_eq!(fx.task("fix1").origin, TaskOrigin::Bisect);
    assert_eq!(fx.task("fix2").origin, TaskOrigin::Sync);
    assert_eq!(fx.task("fix2").state, TaskState::Pending);
    assert_eq!(fx.task("fix2").branch, format!("anthrex/{RUN_ID}/fix2"));

    // A task an M9 run already holds under a `fix<n>` id is skipped.
    let mut old = fx.task("t1").clone();
    old.spec.id = "fix3".into();
    old.spec.owns = vec!["docs/old/**".into()];
    fx.run_mut().tasks.push(old);
    assert_eq!(next_fix_id(fx.run()), "fix4");
    let c = add_fix(
        fx.run_mut(),
        spec(TaskOrigin::Bisect, "docs/z/**", RouteSpec::default()),
        now,
        &mut effects,
    );
    assert_eq!(c.as_deref(), Ok("fix4"));
    assert_eq!(fx.run().fix_seq, 5);
}

#[test]
fn a_fix_task_the_plan_rules_refuse_changes_nothing() {
    let (mut fx, _) = start_on(&super::full::profile(), &[doc_task("t1", "")]);
    let t1 = fx.task("t1").clone();
    assert!(!t1.state.is_finished());
    // Rule 9: a peer runtime overlapping a running task's `owns`.
    let other = peer(t1.route.runtime);
    let Some(entry) = (config::default_roster().into_iter()).find(|m| m.runtime == other) else {
        panic!("the built-in roster has no {other} entry");
    };
    let route = RouteSpec {
        runtime: Some(other),
        model: Some(entry.model.clone()),
        ..RouteSpec::default()
    };
    let before = fx.run().clone();
    let now = fx.now;
    let mut effects = Vec::new();
    let refused = add_fix(
        fx.run_mut(),
        spec(TaskOrigin::Bisect, "docs/t1/**", route),
        now,
        &mut effects,
    );
    let message = refused.expect_err("rule 9 refuses it");
    assert!(message.contains("(rule 9)"), "{message}");
    assert_eq!(fx.run(), &before);
    assert!(effects.is_empty());
}

/// Milestone 9.2 decision 41: a 9.2 fix task's text reads in its history and the
/// run's log as 9.1's do (`a fix task for the <text>`).
#[test]
fn a_ci_fix_task_logs_its_text() {
    let (mut fx, _) = start_on(&super::full::profile(), &[doc_task("t1", "")]);
    let mut ci = spec(TaskOrigin::Ci, "docs/ci/**", RouteSpec::default());
    ci.fixes = FixOf::Ci {
        stage: 1,
        head: BASE.into(),
        ci_runs: vec![7, 8],
        key: "a::works".into(),
    };
    let now = fx.now;
    let id = add_fix(fx.run_mut(), ci, now, &mut Vec::new()).expect("added");
    assert_eq!(
        fx.task(&id).history.last().map(|e| e.text.as_str()),
        Some("added by the engine: a fix task for the CI runs 7, 8")
    );
    let line = format!("fix task {id} added for the CI runs 7, 8");
    assert!(fx.run().log.iter().any(|l| l.text == line), "{line}");
}
