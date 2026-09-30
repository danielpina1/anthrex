//! Milestone 9.1 decision 6 and controller ruling 1 (task M9.1.13): an untiered profile
//! emits M8a's gate ops. The op sequence of M8a's gate fixture is compared with the one
//! recorded from M9's code (`m9_gate_ops.json`, recorded by this test's first run on the
//! commit before task M9.1.13, with `ANTHREX_RECORD_M9_GATE_OPS=1`).

use serde_json::{Value, json};

use super::fixture::*;
use super::gates::{accepted, check_result, only_op, tdd_args, working_with};
use super::merge::{
    candidate, commit, config, doc_task, merge, pending_one, start, to_queue, window_of,
};
use crate::run::engine::{Effect, EventKind, OpResult};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/run/engine/tests/m9_gate_ops.json"
);

/// The gate ops whose fields are compared; every other op is compared by its kind.
const GATE_OPS: [&str; 4] = ["Proof", "Check", "MergeCandidate", "VerifyRefs"];

/// Fields milestone 9.1 adds with `#[serde(default)]` (decision 6: left out of the
/// comparison).
const NEW_FIELDS: [&str; 1] = ["tier"];

fn ops_json(log: &[Effect]) -> Vec<Value> {
    log.iter()
        .filter_map(|e| match e {
            Effect::Op { kind, .. } => {
                let name = op_name(kind);
                if !GATE_OPS.contains(&name) {
                    return Some(json!({ "op": name }));
                }
                let mut value = serde_json::to_value(kind).expect("an op serializes");
                strip(&mut value);
                Some(json!({ "op": name, "kind": value }))
            }
            _ => None,
        })
        .collect()
}

fn strip(value: &mut Value) {
    if let Value::Object(map) = value {
        for field in NEW_FIELDS {
            map.remove(field);
        }
        for inner in map.values_mut() {
            strip(inner);
        }
    }
}

/// Three runs: a tdd task whose proof passes and check fails (rung 1), then whose
/// proof and check pass, up to its merge candidate; a check-mode task whose candidate
/// is red (rung 1); and a check-mode task that merges, whose run completes after a
/// rebaselined head's final check.
fn scenario() -> Vec<Value> {
    let (mut fx, window) = working_with(PROFILE, "", config());
    for check_ok in [false, true] {
        let effects = accepted(&mut fx, window, tdd_args());
        let (op, _) = only_op(&effects, "Proof");
        let passed = OpResult::Proof {
            red_failed: true,
            head_passed: true,
            matched: true,
            red_tail: "red".into(),
            head_tail: "head".into(),
        };
        let effects = fx.done(op, passed);
        let (op, _) = only_op(&effects, "Check");
        fx.done(op, check_result(check_ok));
        if !check_ok {
            fx.turn_completed(window);
        }
    }
    pending_one(&fx, "MergeCandidate", Some("t1"));
    let mut ops = ops_json(&fx.log);

    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = candidate(&fx, "t1");
    fx.done(
        op,
        OpResult::CandidateRed {
            code: Some(101),
            timed_out: false,
            tail: "red".into(),
            secs: 3,
            tier: None,
        },
    );
    assert_eq!(fx.task("t1").rung, 1);
    ops.extend(ops_json(&fx.log));

    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    // A moved run ref, rebaselined: the final check runs on the new head.
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(
        op,
        OpResult::RefMoved {
            reason: "moved".into(),
        },
    );
    let reply = fx.reply();
    let head = "5555555555555555555555555555555555555555";
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some((BASE.to_string(), head.to_string()).into()),
    });
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(op, OpResult::RefsOk);
    let (op, _) = pending_one(&fx, "Check", None);
    fx.done(op, check_result(true));
    assert_eq!(fx.run().state, proto::RunState::Complete);
    ops.extend(ops_json(&fx.log));
    ops
}

#[test]
fn untiered_profile_emits_m8a_ops() {
    let ops = scenario();
    if std::env::var_os("ANTHREX_RECORD_M9_GATE_OPS").is_some() {
        let text = serde_json::to_string_pretty(&ops).expect("ops serialize");
        crate::run::test_support::record_fixture(FIXTURE, &(text + "\n"));
        return;
    }
    let recorded: Vec<Value> =
        serde_json::from_str(include_str!("m9_gate_ops.json")).expect("the fixture is JSON");
    assert_eq!(ops, recorded);
}
