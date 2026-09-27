//! M8c.9 review I1 and M6: a snapshot closes a gate `Confirm` or an edit form that
//! could now only be refused, or would do something other than what it said.

use super::*;

/// The gate snapshot as `gate()` holds it, so a delivery changes only what a test says.
fn gate_snapshot() -> proto::RunsSnapshot {
    let (mut snap, _) = gate_fixture();
    snap.runs[0].tasks[0] = edit_fixture_task();
    snap
}

/// Opens `key`'s `Confirm` in a fresh gate, `t2` selected.
fn confirming(key: char) -> App {
    let mut app = gate();
    select(&mut app, task_key("t2"));
    assert!(tap(&mut app, KeyCode::Char(key)).is_empty());
    assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
    app
}

/// The modal is closed, `text` toasted, and a following `y` or Enter sends nothing.
fn closed_with(app: &mut App, text: &str) {
    assert_eq!(app.modal, None);
    assert_eq!(app.toast_text(), Some(text));
    for code in [KeyCode::Char('y'), KeyCode::Enter] {
        let effects = tap(app, code);
        assert!(
            !effects.iter().any(|e| matches!(e, Effect::Send(_))),
            "{code:?} sent {effects:?}"
        );
    }
}

const RUNNING: &str = "the plan gate is closed: run add-reset-3f9a is running";

#[test]
fn a_running_snapshot_closes_each_gate_confirm() {
    for key in ['a', 'x', 'd'] {
        let mut app = confirming(key);
        let mut snap = gate_snapshot();
        snap.runs[0].state = RunState::Running;
        deliver(&mut app, snap);
        closed_with(&mut app, RUNNING);
    }
}

#[test]
fn a_snapshot_with_no_runs_closes_each_gate_confirm() {
    for key in ['a', 'x', 'd'] {
        let mut app = confirming(key);
        deliver(&mut app, snapshot(10_001, vec![]));
        assert_eq!(app.run_view, None);
        closed_with(
            &mut app,
            "the plan gate is closed: run add-reset-3f9a is gone",
        );
    }
}

#[test]
fn a_remove_whose_task_is_gone_or_cancelled_closes() {
    let removed = |snap: &mut proto::RunsSnapshot| {
        snap.runs[0].tasks.remove(1);
    };
    let cancelled = |snap: &mut proto::RunsSnapshot| {
        snap.runs[0].tasks[1].state = TaskState::Cancelled;
    };
    for change in [&removed as &dyn Fn(&mut _), &cancelled] {
        let mut app = confirming('d');
        let mut snap = gate_snapshot();
        change(&mut snap);
        deliver(&mut app, snap);
        closed_with(&mut app, "t2 is no longer in run add-reset-3f9a's plan");
    }
}

#[test]
fn an_approve_whose_starting_count_changed_closes() {
    let added = |snap: &mut proto::RunsSnapshot| {
        let t3 = task("t3", "new", Size::S, TaskState::Pending);
        snap.runs[0].tasks.push(t3);
    };
    let cancelled = |snap: &mut proto::RunsSnapshot| {
        snap.runs[0].tasks[1].state = TaskState::Cancelled;
    };
    for change in [&added as &dyn Fn(&mut _), &cancelled] {
        let mut app = confirming('a');
        let mut snap = gate_snapshot();
        change(&mut snap);
        deliver(&mut app, snap);
        closed_with(&mut app, "run add-reset-3f9a's plan changed; press a again");
    }
}

#[test]
fn a_snapshot_that_keeps_the_gate_keeps_each_confirm() {
    for key in ['a', 'x', 'd'] {
        let mut app = confirming(key);
        let before = app.modal.clone();
        let mut snap = gate_snapshot();
        // Not a count or a task the confirm names: t1's brief changes.
        snap.runs[0].tasks[0].brief = "other".into();
        deliver(&mut app, snap);
        assert_eq!(app.modal, before, "{key}");
        assert_eq!(tap(&mut app, KeyCode::Char('y')).len(), 1, "{key}");
    }
}

/// M6: each of the five values the form opened from, changed elsewhere.
fn changes() -> Vec<fn(&mut proto::TaskInfo)> {
    vec![
        |t| t.route_spec.model = Some("claude-opus-9".into()),
        |t| t.size = Size::S,
        |t| t.test_mode = TestMode::Check,
        |t| t.test_mode_reason = Some("renames".into()),
        |t| t.brief = "Line one\nLine two\u{7}".into(),
    ]
}

#[test]
fn a_task_changed_elsewhere_closes_an_idle_form() {
    for (at, change) in changes().into_iter().enumerate() {
        let mut app = gate();
        open_form(&mut app, "t1");
        focus(&mut app, EditField::Effort);
        tap(&mut app, KeyCode::Right);
        let mut snap = gate_snapshot();
        change(&mut snap.runs[0].tasks[0]);
        deliver(&mut app, snap);
        assert_eq!(app.modal, None, "change {at}");
        closed_with(&mut app, "t1 changed in run add-reset-3f9a; press e again");
    }
}

#[test]
fn a_task_changed_while_submitting_keeps_the_form() {
    for (at, change) in changes().into_iter().enumerate() {
        let mut app = gate();
        submit_a_size_change(&mut app);
        let before = form(&app).clone();
        let mut snap = gate_snapshot();
        change(&mut snap.runs[0].tasks[0]);
        deliver(&mut app, snap);
        assert_eq!(
            form(&app),
            &before,
            "change {at}: our own edit causes these"
        );
    }
}
