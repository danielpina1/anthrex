//! Milestone 9.1 task M9.1.15's review (ruling C-19): an open bisect fix task holds its
//! stage's idle tier 3 and bisect; a paused or halted run issues no probe; `G` is the
//! last green; the fix task falls back to the culprit's own route; `fix_seq` survives a
//! restart; completion holds while a probe backs off.

use proto::{PlanEdit, RunState};

use super::*;
use crate::run::engine::EventKind;
use crate::run::engine::fixes::next_fix_id;
use crate::run::test_support::task_toml;

/// The idle tier-3 jobs pending.
fn idle_jobs(fx: &Fixture) -> usize {
    pending(fx, "Tier", None).len()
}

#[test]
fn an_open_bisect_fix_task_holds_its_stages_idle_tier3_and_bisect() {
    // t1..t3 merged, red from t2, fixed by fix1; t4 still to merge.
    let ids = ["t1", "t2", "t3", "t4"];
    let tasks: Vec<String> = ids.iter().map(|id| doc_task(id, "")).collect();
    let (mut fx, mut windows) = start_on(&profile(), &tasks);
    with_orchestrator(&mut fx);
    for (k, id) in ids[..3].iter().enumerate() {
        merge_next(&mut fx, &mut windows, id, &commit(k as u32 + 1));
    }
    let since = fx.run().queue_idle_since.expect("idle");
    fx.send(since + 120, EventKind::Tick);
    let (op, spec) = full_job(&fx);
    assert_eq!(spec.head, commit(3));
    fx.done(op, tier(outcome(3, &[TEST])));
    answer(&mut fx, 2);
    assert_ne!(fx.task("fix1").state, TaskState::Merged);
    // fix1 not merged: the head moves to c7, and ticks pass.
    merge_next(&mut fx, &mut windows, "t4", &commit(7));
    assert_eq!(fx.run().stage_head(1), Some(commit(7).as_str()));
    // Past `full_idle_secs` (120) several times over.
    for _ in 0..4 {
        let effects = later(&mut fx, 150);
        assert!(ops_in(&effects, "Tier").is_empty(), "{effects:#?}");
        assert!(ops_in(&effects, "TestAt").is_empty(), "{effects:#?}");
    }
    assert_eq!(idle_jobs(&fx), 0);
    assert!(fx.run().task("fix2").is_none());
    assert_eq!(fx.run().stage(1).unwrap().full.bisect_fixes, 1);
    // Completion waits for fix1 too: no guard while it is unfinished.
    assert!(pending(&fx, "VerifyRefs", None).is_empty());

    // A red that comes in anyway (9.2's `request`) starts no bisect.
    let now = fx.now;
    let mut effects = Vec::new();
    assert!(crate::run::engine::full::request(
        fx.run_mut(),
        1,
        crate::run::engine::full::FullWhy::Deliver,
        now,
        &mut effects
    ));
    let (op, _) = full_job(&fx);
    fx.done(op, tier(outcome(3, &[TEST])));
    assert!(pending(&fx, "TestAt", None).is_empty(), "no second bisect");
    assert!(fx.run().task("fix2").is_none());
    assert_eq!(fx.run().stage(1).unwrap().full.red_at, Some(commit(7)));

    // fix1 merges: the next idle tier 3 runs.
    merge_next(&mut fx, &mut windows, "fix1", &commit(8));
    let since = fx.run().queue_idle_since.expect("idle");
    let effects = fx.send(since + 120, EventKind::Tick);
    let jobs: Vec<_> = ops_in(&effects, "Tier");
    assert_eq!(jobs.len(), 1, "{effects:#?}");
    assert!(matches!(&jobs[0].1, OpKind::Tier(s) if s.tier == 3 && s.head == commit(8)));
}

#[test]
fn a_paused_or_halted_run_issues_no_probe_until_it_runs() {
    for halted in [false, true] {
        let mut fx = merged(&["t1", "t2"], "");
        red_full(&mut fx);
        let (op, spec) = probe(&fx);
        if halted {
            fx.run_mut().state = RunState::Halted;
        } else {
            super::super::dispatch::edit(&mut fx, vec![PlanEdit::Pause]);
            assert_eq!(fx.run().state, RunState::Paused);
        }
        let effects = fx.done(op, probe_result(false, &spec.commit));
        assert!(ops_in(&effects, "TestAt").is_empty(), "{effects:#?}");
        let b = fx
            .run()
            .stage(1)
            .unwrap()
            .bisect
            .clone()
            .expect("bisecting");
        assert_eq!((b.probes, b.probe), (1, None), "the result is recorded");
        assert!(ops_in(&later(&mut fx, 1_000), "TestAt").is_empty());
        assert_eq!(fx.run().full_op, None);
        let effects = if halted {
            fx.run_mut().state = RunState::Running;
            fx.tick()
        } else {
            resume(&mut fx)
        };
        let probes = ops_in(&effects, "TestAt");
        assert_eq!(probes.len(), 1, "halted {halted}: {effects:#?}");
        assert!(matches!(&probes[0].1, OpKind::TestAt(s) if s.commit == commit(2)));
    }
}

#[test]
fn the_search_starts_at_the_last_green_tier3() {
    let mut fx = merged(&["t1", "t2", "t3", "t4", "t5", "t6"], "");
    fx.run_mut().stages[0].full.green_at = Some(commit(3));
    red_full(&mut fx);
    assert!(
        fx.run()
            .log
            .iter()
            .any(|l| l.text == "stage 1: tier 3 red (a::works); bisecting 3 merges")
    );
    let probed = answer(&mut fx, 5);
    assert_eq!(probed, [commit(3), commit(6), commit(4), commit(5)]);
    assert!(matches!(
        &fx.task("fix1").fixes,
        Some(FixOf::Bisect { culprit, .. }) if culprit == "t5"
    ));
}

#[test]
fn a_refused_escalation_falls_back_to_the_culprits_own_route() {
    // t3 overlaps t2's `owns` and stays open on t2's runtime, so a fix task on the peer
    // runtime breaks rule 9.
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", ""),
        task_toml(
            "t3",
            "S",
            "[\"docs/t2/more/**\"]",
            "test_mode = \"check\"\ntest_mode_reason = \"glue code\"",
        ),
    ];
    let (mut fx, mut windows) = start_on(&profile(), &tasks);
    let mut route = fx.task("t2").route.clone();
    route.effort = proto::Effort::High;
    fx.task_mut("t2").route = route.clone();
    let up = escalate(&fx.run().roster, &route);
    assert_ne!(
        up.runtime, route.runtime,
        "the escalation is the peer runtime"
    );
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    merge_next(&mut fx, &mut windows, "t2", &commit(2));
    assert!(!fx.task("t3").state.is_finished());
    let since = fx.run().queue_idle_since.expect("idle");
    fx.send(since + 120, EventKind::Tick);
    let (op, _) = full_job(&fx);
    fx.done(op, tier(outcome(3, &[TEST])));
    answer(&mut fx, 2);
    let fix = fx.task("fix1");
    assert_eq!(fix.route, route, "the culprit's own route");
    assert!(matches!(&fix.fixes, Some(FixOf::Bisect { culprit, .. }) if culprit == "t2"));
}

#[test]
fn fix_seq_survives_a_restart() {
    let mut fx = merged(&["t1", "t2"], "");
    fx.run_mut().fix_seq = 7;
    let text = serde_json::to_string(fx.run()).unwrap();
    *fx.run_mut() = serde_json::from_str(&text).unwrap();
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().fix_seq, 7);
    assert_eq!(next_fix_id(fx.run()), "fix7");
    resume(&mut fx);
    red_full(&mut fx);
    answer(&mut fx, 2);
    assert!(fx.run().task("fix7").is_some());
    assert!(fx.run().task("fix1").is_none());
    assert_eq!(fx.run().fix_seq, 8);
}

#[test]
fn completion_holds_while_a_probe_backs_off() {
    // t1 and t2 merged, idle tier 3 red on c2; t3 merges while the probe backs off, so
    // the stage head is not the red commit and every task is finished.
    let ids = ["t1", "t2", "t3"];
    let tasks: Vec<String> = ids.iter().map(|id| doc_task(id, "")).collect();
    let (mut fx, mut windows) = start_on(&profile(), &tasks);
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    merge_next(&mut fx, &mut windows, "t2", &commit(2));
    let since = fx.run().queue_idle_since.expect("idle");
    fx.send(since + 120, EventKind::Tick);
    let (op, _) = full_job(&fx);
    fx.done(op, tier(outcome(3, &[TEST])));
    let (op, _) = probe(&fx);
    fx.done(
        op,
        OpResult::Failed {
            message: "git: could not lock the index".into(),
        },
    );
    merge_next(&mut fx, &mut windows, "t3", &commit(3));
    assert!(fx.run().tasks.iter().all(|t| t.state.is_finished()));
    assert!(fx.run().pending_ops.is_empty());
    let effects = fx.send(fx.now + 10, EventKind::Tick);
    assert!(ops_in(&effects, "VerifyRefs").is_empty(), "{effects:#?}");
    assert!(pending(&fx, "VerifyRefs", None).is_empty());
}
