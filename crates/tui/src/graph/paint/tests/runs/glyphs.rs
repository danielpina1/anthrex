//! M8c.5: the glyphs of agent rounds, scouts and planners (decision 19).

use super::*;
use crate::graph::paint::style::node_glyph;
use crate::tree::run_fixtures::{
    PROJECT, headless, planner, reviewer, run, run_ref, scout, worker,
};
use proto::{Finding, PlannerState, ReviewInfo, Runtime, ScoutState, Severity, Verdict};
use ratatui::style::Color;

const NOW: u64 = 10_000;

fn review(round: &AgentRoundInfo, verdict: Verdict, blocking: bool) -> ReviewInfo {
    let severity = if blocking {
        Severity::Important
    } else {
        Severity::Minor
    };
    ReviewInfo {
        round: round.round,
        route: round.route.clone(),
        verdict: Some(verdict),
        summary: String::new(),
        findings: vec![Finding {
            severity,
            file: None,
            line: None,
            input: None,
            text: "a finding".into(),
        }],
        blocking,
    }
}

/// The record the daemon pushes when a review round starts, before any verdict
/// (`engine/review.rs:162-170`, `:279-289`).
fn pending(round: &AgentRoundInfo) -> ReviewInfo {
    ReviewInfo {
        round: round.round,
        route: round.route.clone(),
        verdict: None,
        summary: String::new(),
        findings: Vec::new(),
        blocking: false,
    }
}

/// The glyph and its role's colour for the row with `key`.
fn glyph(app: &App, key: &NodeKey) -> (&'static str, Color) {
    let rows = view_rows(app);
    let row = rows
        .iter()
        .find(|row| &row.key == key)
        .unwrap_or_else(|| panic!("{key:?} is a row"));
    let (glyph, role) = node_glyph(row, app);
    (glyph, theme::fg(role))
}

fn verdict_app() -> App {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);

    let mut reviews = task("t1", "reviews", Size::S, TaskState::Review);
    reviews.rounds = vec![
        ended(worker(1, None, Runtime::Claude, 100), 150),
        ended(reviewer(1, None, Runtime::Codex, 200), 250),
        ended(reviewer(2, None, Runtime::Codex, 300), 350),
        ended(reviewer(3, None, Runtime::Codex, 400), 450),
        reviewer(4, None, Runtime::Codex, 500),
    ];
    reviews.reviews = vec![
        review(&reviews.rounds[1], Verdict::Changes, true),
        review(&reviews.rounds[2], Verdict::Changes, false),
        // Rounds 3 (ended with no verdict) and 4 (live) carry the record the daemon
        // pushes at their start, with `verdict: None`.
        pending(&reviews.rounds[3]),
        pending(&reviews.rounds[4]),
    ];

    let mut limited = task("t2", "limited", Size::S, TaskState::Working);
    let mut round = worker(1, None, Runtime::Claude, 100);
    round.rate_limited_until = Some(NOW + 60);
    limited.rounds = vec![round];

    let mut expired = task("t3", "expired", Size::S, TaskState::Working);
    let mut round = worker(1, None, Runtime::Claude, 100);
    round.rate_limited_until = Some(NOW - 1);
    // The snapshot's own flag, evaluated at publication, is never read.
    round.rate_limited = true;
    expired.rounds = vec![round];

    let mut blocked = task("t4", "blocked", Size::S, TaskState::Blocked);
    blocked.rounds = vec![ended(worker(1, None, Runtime::Claude, 100), 200)];

    // Two sessions, each sent back once: only session 2's last piece is the last
    // worker round of the task.
    let mut two = task("t5", "two sessions", Size::S, TaskState::Blocked);
    let mut first = ended(worker(1, None, Runtime::Claude, 100), 200);
    first.sent_back_at = vec![150];
    let mut second = ended(worker(2, None, Runtime::Claude, 300), 500);
    second.sent_back_at = vec![400];
    two.rounds = vec![first, second];

    let mut merged = task("t6", "merged", Size::S, TaskState::Merged);
    merged.rounds = vec![ended(worker(1, None, Runtime::Claude, 100), 200)];

    let mut attention = task("t7", "asks", Size::S, TaskState::Working);
    attention.rounds = vec![worker(1, Some(8), Runtime::Claude, 100)];

    info.tasks = vec![reviews, limited, expired, blocked, two, merged, attention];
    let mut window = headless(
        8,
        "3f9a/t7.w1",
        PROJECT,
        Some(run_ref(RUN_ID, Some("t7"), AgentRole::Worker, 1)),
    );
    window.status = proto::Status::Attention;
    app_of((snapshot(NOW, vec![info]), vec![window]))
}

#[test]
fn round_glyphs_follow_the_verdict() {
    let app = verdict_app();
    let green = theme::fg(theme::Role::Done);
    let attention = theme::fg(theme::Role::Attention);
    let live = theme::fg(theme::Role::Working);
    let reviewer_key = |round| round_key("t1", AgentRole::Reviewer, round, round);

    assert_eq!(glyph(&app, &reviewer_key(1)), ("✗", Color::Red), "blocking");
    assert_eq!(glyph(&app, &reviewer_key(2)), ("✓", green), "minor only");
    assert_eq!(
        glyph(&app, &reviewer_key(3)),
        ("–", theme::fg(theme::Role::Muted)),
        "no verdict"
    );
    assert_eq!(glyph(&app, &reviewer_key(4)), ("●", live), "live");
    assert_eq!(
        glyph(&app, &round_key("t1", AgentRole::Worker, 1, 1)),
        ("✓", green),
        "a finished worker of a task in review"
    );

    assert_eq!(
        glyph(&app, &round_key("t2", AgentRole::Worker, 1, 1)),
        ("⚑", attention),
        "rate-limited until after run_now()"
    );
    assert_eq!(
        glyph(&app, &round_key("t3", AgentRole::Worker, 1, 1)),
        ("●", live),
        "the limit has passed; the snapshot's flag is not read"
    );
    assert_eq!(
        glyph(&app, &round_key("t4", AgentRole::Worker, 1, 1)),
        ("✗", Color::Red),
        "the last worker round of a blocked task"
    );
    assert_eq!(
        glyph(&app, &round_key("t6", AgentRole::Worker, 1, 1)),
        ("✓", green)
    );
    assert_eq!(
        glyph(&app, &round_key("t7", AgentRole::Worker, 1, 1)),
        ("⚑", attention),
        "the window asks for attention"
    );
}

/// The M8c.4 review's advisory: `DisplayRound.last` alone is not "the last worker
/// round of the task" — session 1's final piece is `last` too.
#[test]
fn only_the_last_sessions_last_piece_of_a_blocked_task_is_a_cross() {
    let app = verdict_app();
    let green = theme::fg(theme::Role::Done);
    let worker_key = |session, round| round_key("t5", AgentRole::Worker, session, round);
    assert_eq!(glyph(&app, &worker_key(1, 1)), ("✓", green));
    assert_eq!(
        glyph(&app, &worker_key(1, 2)),
        ("✓", green),
        "session 1 is last"
    );
    assert_eq!(glyph(&app, &worker_key(2, 1)), ("✓", green));
    assert_eq!(glyph(&app, &worker_key(2, 2)), ("✗", Color::Red));
}

#[test]
fn scouts_and_planners_glyph_by_their_state() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let with_state = |id: &str, state, window, ended_at| {
        let mut one = scout(id, id, Runtime::Claude, 10);
        one.state = state;
        one.window_id = window;
        one.ended_at = ended_at;
        one
    };
    // Each half of "finished" alone: s3 is reported with no end time yet, s5 has
    // ended while its state still says working.
    info.scouts = vec![
        with_state("s1", ScoutState::Starting, None, None),
        with_state("s2", ScoutState::Working, Some(9), None),
        with_state("s3", ScoutState::Reported, None, None),
        with_state("s4", ScoutState::Failed, None, Some(20)),
        with_state("s5", ScoutState::Working, None, Some(20)),
    ];
    let planner_in = |epic: &str, state, window, ended_at| {
        let mut one = planner(epic, "p");
        one.state = state;
        one.window_id = window;
        one.ended_at = ended_at;
        one
    };
    // B is finished with no end time; E has ended while still planning.
    info.planners = vec![
        planner_in("A", PlannerState::Planning, Some(10), None),
        planner_in("B", PlannerState::Finished, None, None),
        planner_in("C", PlannerState::Failed, None, Some(20)),
        planner_in("D", PlannerState::Planning, None, None),
        planner_in("E", PlannerState::Planning, None, Some(20)),
    ];
    let mut working = headless(9, "scout", PROJECT, None);
    working.status = proto::Status::Working;
    let mut asking = headless(10, "planner", PROJECT, None);
    asking.status = proto::Status::Attention;
    let mut app = app_of((snapshot(NOW, vec![info]), vec![working, asking]));
    app.spinner_frame = 1;

    let green = theme::fg(theme::Role::Done);
    let live = theme::fg(theme::Role::Working);
    let attention = theme::fg(theme::Role::Attention);
    let scout_key = |id: &str| NodeKey::Scout {
        run: RUN_ID.into(),
        id: id.into(),
    };
    let planner_key = |epic: &str| NodeKey::Planner {
        run: RUN_ID.into(),
        epic: epic.into(),
    };
    assert_eq!(glyph(&app, &scout_key("s1")), ("●", live));
    assert_eq!(glyph(&app, &scout_key("s2")), (theme::SPINNER[1], live));
    assert_eq!(glyph(&app, &scout_key("s3")), ("✓", green));
    assert_eq!(glyph(&app, &scout_key("s4")), ("✗", Color::Red));
    assert_eq!(glyph(&app, &planner_key("A")), ("⚑", attention));
    assert_eq!(glyph(&app, &planner_key("B")), ("✓", green));
    assert_eq!(glyph(&app, &planner_key("C")), ("✗", Color::Red));
    assert_eq!(glyph(&app, &planner_key("D")), ("●", live));
    assert_eq!(
        glyph(&app, &scout_key("s5")),
        ("●", live),
        "ended, state working"
    );
    assert_eq!(
        glyph(&app, &planner_key("E")),
        ("●", live),
        "ended, planning"
    );

    // Finished scouts and planners are dim; live ones are not.
    let (layout, lines) = paint_view(&app);
    let dim = |key: &NodeKey| {
        let rect = rect_of(&layout, key);
        style_at(&lines, rect.x + 4, rect.y + 1)
            .add_modifier
            .contains(Modifier::DIM)
    };
    for key in [
        scout_key("s3"),
        scout_key("s4"),
        scout_key("s5"),
        planner_key("B"),
        planner_key("C"),
        planner_key("E"),
    ] {
        assert!(dim(&key), "{key:?} is dim");
    }
    for key in [
        scout_key("s1"),
        scout_key("s2"),
        planner_key("A"),
        planner_key("D"),
    ] {
        assert!(!dim(&key), "{key:?} is not dim");
    }
}

/// Decision 19: `working` animates only while its live *worker* round's window is
/// `Working`. A live reviewer on a `Working` window spins its own round node
/// ("●/spinner (live, no verdict)") but leaves the task at `●`.
#[test]
fn only_a_live_worker_round_animates_a_working_task() {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut reviewed = task("t1", "reviewed", Size::S, TaskState::Working);
    reviewed.rounds = vec![
        ended(worker(1, None, Runtime::Claude, 100), 150),
        reviewer(1, Some(8), Runtime::Codex, 200),
    ];
    reviewed.reviews = vec![pending(&reviewed.rounds[1])];
    let mut worked = task("t2", "worked", Size::S, TaskState::Working);
    worked.rounds = vec![worker(1, Some(9), Runtime::Claude, 100)];
    info.tasks = vec![reviewed, worked];
    let working = |id, name: &str, task_id, role| {
        let mut window = headless(
            id,
            name,
            PROJECT,
            Some(run_ref(RUN_ID, Some(task_id), role, 1)),
        );
        window.status = proto::Status::Working;
        window
    };
    let windows = vec![
        working(8, "3f9a/t1.r1", "t1", AgentRole::Reviewer),
        working(9, "3f9a/t2.w1", "t2", AgentRole::Worker),
    ];
    let mut app = app_of((snapshot(NOW, vec![info]), windows));
    app.spinner_frame = 1;
    let live = theme::fg(theme::Role::Working);

    assert_eq!(glyph(&app, &task_key("t1")), ("●", live), "a reviewer only");
    assert_eq!(
        glyph(&app, &round_key("t1", AgentRole::Reviewer, 1, 1)),
        (theme::SPINNER[1], live),
        "the live reviewer's own node spins"
    );
    assert_eq!(
        glyph(&app, &task_key("t2")),
        (theme::SPINNER[1], live),
        "a live worker"
    );
}
