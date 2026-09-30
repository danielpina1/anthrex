//! Milestone 9.1 task M9.1.20: the stage inspector and a task's `stage`, `origin` and
//! `tier` rows (decision 55). Each test compares the exact `(label, value)` list.

use super::run_tests::{app_of, inspect_node, pairs, value};
use crate::inspector::FieldLayout;
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{RUN_ID, gemini_fixture};
use crate::tree::stage_fixtures::{staged_fixture, staged_gate_fixture};
use proto::{FullState, TierInfo};

fn stage_key(n: u16) -> NodeKey {
    NodeKey::Stage {
        run: RUN_ID.into(),
        n,
    }
}

fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: RUN_ID.into(),
        id: id.into(),
    }
}

#[test]
fn stage_inspector_shows_branch_head_tier3_and_bisect() {
    let app = app_of(staged_fixture());
    let inspection = inspect_node(&app, &stage_key(1));
    assert_eq!(inspection.name, "stage 1/2");
    assert_eq!(inspection.right.as_deref(), Some("1/3 merged"));
    assert_eq!(inspection.layout, FieldLayout::Rows);
    assert_eq!(
        pairs(&inspection),
        [
            ("branch", "anthrex/add-reset-3f9a/stage-1 at 1111111"),
            (
                "tier 3",
                "bisecting at 2222222 · 41m12s · 2 shards · flaky a::flaky · failing a::works"
            ),
            ("bisect", "running · 1 fix task"),
            ("fix tasks", "fix1 (bisect of t2)"),
        ]
    );

    // Stage 2 is not created: no head, no job, no bisect.
    let inspection = inspect_node(&app, &stage_key(2));
    assert_eq!(inspection.right.as_deref(), Some("0/1 merged"));
    assert_eq!(
        pairs(&inspection),
        [
            ("branch", "anthrex/add-reset-3f9a/stage-2 (not created)"),
            ("tier 3", "not run"),
        ]
    );
}

/// A bisect that gave up says why; a red propagate names the lower stage's head.
#[test]
fn stage_inspector_shows_a_bisect_note_and_a_red_propagate() {
    let (mut snap, windows) = staged_fixture();
    let one = &mut snap.runs[0].stages[0];
    one.full.state = FullState::Red;
    one.full.note = Some("no single culprit: t1, t2".into());
    snap.runs[0].stages[1].head = Some("3".repeat(40));
    snap.runs[0].stages[1].propagate_red = Some("1".repeat(40));
    let app = app_of((snap, windows));
    let inspection = inspect_node(&app, &stage_key(1));
    assert_eq!(
        value(&inspection, "bisect"),
        Some("1 fix task · no single culprit: t1, t2")
    );
    let inspection = inspect_node(&app, &stage_key(2));
    assert_eq!(
        value(&inspection, "propagate"),
        Some("stage 1 at 1111111 is red here")
    );
}

#[test]
fn task_inspector_shows_stage_origin_and_tier() {
    let (mut snap, windows) = staged_fixture();
    let fix = snap.runs[0]
        .tasks
        .iter_mut()
        .find(|t| t.id == "fix1")
        .unwrap();
    fix.tier = Some(TierInfo {
        tier: 1,
        affected: "3 modules".into(),
        steps: 3,
        cached: 1,
        ok: true,
        secs: 130,
        flaky: vec!["t_x".into()],
    });
    let app = app_of((snap, windows));
    let inspection = inspect_node(&app, &task_key("fix1"));
    assert_eq!(value(&inspection, "stage"), Some("1 of 2"));
    assert_eq!(
        value(&inspection, "origin"),
        Some("bisect · fixes: bisect of t2")
    );
    assert_eq!(
        value(&inspection, "tier"),
        Some("tier 1: 3 modules · 2m10s · 1 cached · flaky t_x")
    );
    let inspection = inspect_node(&app, &task_key("t3"));
    assert_eq!(value(&inspection, "stage"), Some("2 of 2"));
    assert_eq!(
        (value(&inspection, "origin"), value(&inspection, "tier")),
        (None, None)
    );
}

/// The plan gate's stages have no head yet, so neither reads as created (C-24).
#[test]
fn the_gates_stages_read_as_not_created() {
    let app = app_of(staged_gate_fixture());
    for n in [1, 2] {
        let inspection = inspect_node(&app, &stage_key(n));
        assert_eq!(
            value(&inspection, "branch"),
            Some(format!("anthrex/add-reset-3f9a/stage-{n} (not created)").as_str())
        );
    }
}

/// Pinning: a one-stage plan task has none of the new rows.
#[test]
fn a_plan_task_of_a_one_stage_run_has_no_stage_rows() {
    let app = app_of(gemini_fixture());
    let key = NodeKey::Task {
        run: "r1".into(),
        id: "t2".into(),
    };
    let inspection = inspect_node(&app, &key);
    for label in ["stage", "origin", "tier"] {
        assert_eq!(value(&inspection, label), None, "{label}");
    }
}
