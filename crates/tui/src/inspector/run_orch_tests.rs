//! M9.15: milestone 9 in the run view's nodes and inspections — a planning run, held,
//! paused and reported tasks, planners from `PlannerInfo`, research rounds, a task's
//! messages and notes, and every agent-written text drawn without a control character.

use super::run_tests::{app_of, inspect_node, pairs, value};
use crate::app::App;
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::theme;
use crate::tree::orch_fixtures::{ORCH_NOW, held_fixture, hold, orch_fixture};
use crate::tree::run_fixtures::{PROJECT, RUN_ID, headless, planner, run_ref, task};
use crate::tree::{NodeKey, RunFilter, round_label, run_rows};
use proto::{
    AgentRole, HoldState, MessageKind, PlannerState, RunState, Size, Status, TaskKind,
    TaskNoteInfo, TaskNoteKind, TaskState,
};
use ratatui::style::Color;

fn run_key() -> NodeKey {
    NodeKey::Run(RUN_ID.into())
}

fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: RUN_ID.into(),
        id: id.into(),
    }
}

/// The canvas text of the run-view row `key` names.
fn node_text(app: &App, key: &NodeKey) -> String {
    let rows = run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All);
    let row = rows.iter().find(|row| &row.key == key).expect("a row");
    crate::graph::content_text(row)
}

/// A task's glyph, its colour, and the inspection's right-hand text.
fn look(app: &App, id: &str) -> (String, Option<Color>, String) {
    let inspection = inspect_node(app, &task_key(id));
    (
        inspection.glyph.content.to_string(),
        inspection.glyph.style.fg,
        inspection.right.unwrap_or_default(),
    )
}

#[test]
fn planning_run_header() {
    let (mut snapshot, windows) = orch_fixture(RunState::Planning);
    snapshot.runs[0].tasks.clear();
    snapshot.runs[0].critical_path.clear();
    let app = app_of((snapshot, windows));
    let inspection = inspect_node(&app, &run_key());
    assert_eq!(inspection.right.as_deref(), Some("planning · 10m"));
    assert_eq!(
        value(&inspection, "gate"),
        Some("planning · s submits the plan yourself")
    );
    assert_eq!(
        value(&inspection, "orchestrator"),
        Some("claude claude-opus-5 · window #3 · live · no wakes · plan not submitted")
    );
    assert_eq!(node_text(&app, &run_key()), "orchestrator  planning");
    assert_eq!(
        inspection.glyph.style.fg,
        Some(theme::fg(theme::Role::Working))
    );
}

#[test]
fn reported_held_and_paused_glyphs() {
    let (snapshot, windows) = held_fixture();
    let app = app_of((snapshot.clone(), windows.clone()));
    let paused = theme::fg(theme::Role::Paused);
    assert_eq!(
        look(&app, "t1"),
        ("‖".into(), Some(paused), "paused (message)".into())
    );
    let idle = theme::fg(theme::Role::Muted);
    assert_eq!(
        look(&app, "t2"),
        ("○".into(), Some(idle), "queued · held".into())
    );
    let done = theme::fg(theme::Role::Done);
    assert_eq!(
        look(&app, "t3"),
        ("✓".into(), Some(done), "reported".into())
    );
    // The paused colour is its own.
    for status in [
        Status::Starting,
        Status::Working,
        Status::Idle,
        Status::Attention,
        Status::Done,
    ] {
        assert_ne!(paused, theme::fg(theme::status_look(status, 0, false).1));
    }

    // Once the hold is approved, `t2` is an ordinary queued task again.
    let mut approved = snapshot;
    approved.runs[0].holds = vec![hold("epic:ui", HoldState::Approved, &["t2"])];
    let app = app_of((approved, windows));
    assert_eq!(
        look(&app, "t2"),
        (
            "▫".into(),
            Some(theme::fg(theme::Role::Muted)),
            "queued".into()
        )
    );
}

#[test]
fn a_paused_task_is_no_blocked_attention_line_until_the_daemon_lists_it() {
    let (mut snapshot, windows) = held_fixture();
    snapshot.runs[0].holds.clear();
    let app = app_of((snapshot.clone(), windows.clone()));
    assert_eq!(value(&inspect_node(&app, &run_key()), "attention"), None);
    snapshot.runs[0].attention = vec!["t1 paused(message) for 10 min".into()];
    let app = app_of((snapshot, windows));
    assert_eq!(
        value(&inspect_node(&app, &run_key()), "attention"),
        Some("t1 paused(message) for 10 min")
    );
}

#[test]
fn planner_nodes_from_planner_info() {
    let (mut snapshot, mut windows) = orch_fixture(RunState::Running);
    let run = &mut snapshot.runs[0];
    let mut a = planner("A", "daemon side");
    a.window_id = Some(8);
    a.started_at = ORCH_NOW - 120;
    a.area = vec!["crates/daemon".into()];
    a.edits_accepted = 2;
    a.note = Some("re-planned after t4 failed".into());
    let mut b = planner("B", "");
    b.state = PlannerState::Finished;
    b.ended_at = Some(ORCH_NOW - 30);
    let mut c = planner("C", "tui");
    c.state = PlannerState::Failed;
    c.ended_at = Some(ORCH_NOW - 10);
    run.planners = vec![a, b, c];
    let mut t4 = task("t4", "engine", Size::S, TaskState::Merged);
    t4.epic = Some("A".into());
    let mut t5 = task("t5", "driver", Size::S, TaskState::Working);
    t5.epic = Some("A".into());
    run.tasks.extend([t4, t5]);
    let mut window = headless(
        8,
        "3f9a/plan-A.p1",
        PROJECT,
        Some(run_ref(RUN_ID, None, AgentRole::Planner, 1)),
    );
    window.status = Status::Working;
    windows.push(window);
    let app = app_of((snapshot, windows));

    let key = |epic: &str| NodeKey::Planner {
        run: RUN_ID.into(),
        epic: epic.into(),
    };
    assert_eq!(node_text(&app, &key("A")), "planner A daemon side  1/2");
    assert_eq!(node_text(&app, &key("B")), "planner B  0/0");
    let a = inspect_node(&app, &key("A"));
    assert_eq!(
        a.glyph.content,
        theme::status_look(Status::Working, 0, false).0
    );
    assert_eq!(
        pairs(&a),
        [
            ("progress", "█████░░░░░  1/2 merged · 1 working"),
            ("area", "crates/daemon"),
            ("edits", "2 accepted · 0 rejected"),
            ("note", "re-planned after t4 failed"),
            (
                "session",
                "#8 · headless · main checkout, read-only · Enter: conversation"
            ),
        ]
    );
    assert_eq!(inspect_node(&app, &key("B")).glyph.content, "✓");
    let c = inspect_node(&app, &key("C"));
    assert_eq!(c.glyph.content, "✗");
    assert_eq!(value(&c, "session"), Some("no window yet"));
}

#[test]
fn research_round_label() {
    assert_eq!(round_label(AgentRole::Scout, 2, 1), "research #2");
    let (mut snapshot, windows) = held_fixture();
    let t3 = &mut snapshot.runs[0].tasks[3];
    assert_eq!(t3.kind, TaskKind::Research);
    let mut round = crate::tree::run_fixtures::worker(2, None, proto::Runtime::Codex, 9_000);
    round.role = AgentRole::Scout;
    round.ended_at = Some(9_500);
    t3.rounds = vec![round];
    let app = app_of((snapshot, windows));
    let key = NodeKey::AgentRound {
        run: RUN_ID.into(),
        task: "t3".into(),
        role: AgentRole::Scout,
        lane: None,
        session: 2,
        round: 2,
    };
    assert_eq!(node_text(&app, &key), "research #2 codex");
    assert!(
        inspect_node(&app, &key)
            .name
            .starts_with("research #2  codex"),
        "{}",
        inspect_node(&app, &key).name
    );
}

#[test]
fn messages_row() {
    let (mut snapshot, windows) = held_fixture();
    let t2 = &mut snapshot.runs[0].tasks[2];
    t2.message_count = 3;
    t2.last_message_kind = Some(MessageKind::Change);
    t2.last_message_line = Some("t4 now owns lib.rs; import from there".into());
    let app = app_of((snapshot, windows));
    assert_eq!(
        value(&inspect_node(&app, &task_key("t2")), "messages"),
        Some("3 · latest change: \"t4 now owns lib.rs; import from there\"")
    );
    // No message, no row.
    assert_eq!(
        value(&inspect_node(&app, &task_key("t0")), "messages"),
        None
    );
}

fn note(task: &str, kind: TaskNoteKind, text: &str, at: u64) -> TaskNoteInfo {
    TaskNoteInfo {
        task_id: task.into(),
        kind,
        text: text.into(),
        at,
    }
}

#[test]
fn message_pause_and_notes_render_with_attribution() {
    let (mut snapshot, windows) = held_fixture();
    let t1 = &mut snapshot.runs[0].tasks[1];
    t1.message_count = 1;
    t1.last_message_kind = Some(MessageKind::StopAndWait);
    t1.last_message_line = Some("hold on".into());
    // 9_900 s is 02:45 UTC; the snapshot sends them oldest first.
    t1.task_notes = vec![
        note("t1", TaskNoteKind::Discovery, "the API is shared", 9_840),
        note("t1", TaskNoteKind::Progress, "half done", 9_870),
        note("t1", TaskNoteKind::Risk, "migration may lock", 9_900),
    ];
    let app = app_of((snapshot, windows));
    let inspection = inspect_node(&app, &task_key("t1"));
    assert_eq!(inspection.right.as_deref(), Some("paused (message)"));
    assert_eq!(
        value(&inspection, "messages"),
        Some("1 · latest stop and wait: \"hold on\"")
    );
    assert_eq!(
        value(&inspection, "notes"),
        Some("02:45 risk from t1: migration may lock · 02:44 discovery from t1: the API is shared")
    );
}

#[test]
fn hold_attention_line_lists_every_awaiting_hold() {
    let (mut snapshot, windows) = held_fixture();
    let run = &mut snapshot.runs[0];
    run.holds
        .push(hold("promotion", HoldState::Awaiting, &["t0", "t1"]));
    run.holds
        .push(hold("epic:db", HoldState::Drafting, &["t9"]));
    run.attention = vec!["final check failed on the run head".into()];
    let app = app_of((snapshot, windows));
    assert_eq!(
        value(&inspect_node(&app, &run_key()), "attention"),
        Some("hold epic:ui: 1 task waits for approval · +2 more")
    );
}

#[test]
fn task_notes_and_message_lines_render_sanitised() {
    let bad = hostile_text();
    let (mut snapshot, windows) = held_fixture();
    let run = &mut snapshot.runs[0];
    run.goal = bad.clone();
    run.attention = vec![bad.clone()];
    run.holds = vec![hold(&format!("epic:{bad}"), HoldState::Awaiting, &["t2"])];
    run.tasks[2].hold = Some(format!("epic:{bad}"));
    let orchestrator = run.orchestrator.as_mut().expect("an orchestrator");
    orchestrator.summary = Some(bad.clone());
    orchestrator.route.model = bad.clone();
    let mut p = planner(&format!("E{bad}"), &bad);
    p.note = Some(bad.clone());
    p.last_rejection = Some(bad.clone());
    run.planners = vec![p];
    for t in &mut run.tasks {
        t.title = bad.clone();
        t.message_count = 1;
        t.last_message_kind = Some(MessageKind::Info);
        t.last_message_line = Some(bad.clone());
        t.task_notes = vec![note(&bad, TaskNoteKind::Risk, &bad, 9_900)];
        t.epic = Some(format!("E{bad}"));
    }
    run.tasks[1].block.as_mut().expect("paused").text = bad.clone();
    let app = app_of((snapshot, windows));
    let rows = run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All);
    assert!(rows.len() > 5, "{} rows", rows.len());
    for row in &rows {
        let text = crate::graph::content_text(row);
        assert_eq!(first_hostile(&text), None, "node {:?}: {text:?}", row.key);
        let inspection = crate::inspector::inspect(row, &app);
        let mut texts = vec![
            inspection.name.clone(),
            inspection.right.unwrap_or_default(),
        ];
        texts.extend(inspection.fields.into_iter().map(|field| field.value));
        for text in texts {
            assert_eq!(first_hostile(&text), None, "{:?}: {text:?}", row.key);
        }
    }
}
