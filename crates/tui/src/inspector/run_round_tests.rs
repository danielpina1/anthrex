//! M8c.7: the agent-round projection (Interfaces "Inspector contents, exact": Agent
//! round) — workers, their `fixing` line, rate limits, and reviewers.

use super::run_task_tests::with_task;
use super::run_tests::{app_of, inspect_node, pairs, value};
use crate::app::App;
use crate::inspector::FieldLayout;
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{GEMINI_NOW, gemini_fixture, headless, run_ref};
use proto::{AgentRole, CheckInfo, TaskState};

const T2_SESSION: &str = "#7 · headless · worktree anthrex/r1/t2 · Enter: conversation";

fn round_key(task: &str, role: AgentRole, session: u32, round: u32) -> NodeKey {
    NodeKey::AgentRound {
        run: "r1".into(),
        task: task.into(),
        role,
        session,
        round,
    }
}

#[test]
fn worker_round_fields_match_the_mockup() {
    let app = app_of(gemini_fixture());
    let inspection = inspect_node(&app, &round_key("t2", AgentRole::Worker, 1, 2));
    assert_eq!(inspection.glyph.content, "⠋");
    assert_eq!(inspection.name, "worker #1 r2  codex · standard · high");
    assert_eq!(inspection.right.as_deref(), Some("working · 6m · t2"));
    assert_eq!(inspection.layout, FieldLayout::Rows);
    assert_eq!(
        pairs(&inspection),
        [
            ("doing", "last tool: apply_patch"),
            ("activity", "turns 14 · tool calls 41 · tokens 180k"),
            ("fixing", "status.rs:118 critical — SubagentStop not paired"),
            ("session", T2_SESSION),
        ]
    );
}

#[test]
fn the_first_display_round_has_no_fixing_and_no_activity() {
    let app = app_of(gemini_fixture());
    let inspection = inspect_node(&app, &round_key("t2", AgentRole::Worker, 1, 1));
    assert_eq!(inspection.glyph.content, "✓");
    assert_eq!(inspection.name, "worker #1  codex · standard · high");
    assert_eq!(inspection.right.as_deref(), Some("finished · 20m · t2"));
    assert_eq!(
        pairs(&inspection),
        [("doing", "finished"), ("session", T2_SESSION)]
    );
}

/// `t2` with no review failures, and a failed check at `now − 400`, before the second
/// display round starts at `now − 360`.
fn failed_check(summary: &str, decider: Option<&str>) -> App {
    with_task("t2", |task| {
        task.reviews.clear();
        task.last_check = Some(CheckInfo {
            at: GEMINI_NOW - 400,
            ok: false,
            code: Some(101),
            timed_out: false,
            secs: 30,
            summary: summary.into(),
            on_candidate: false,
            decider_summary: decider.map(str::to_owned),
            summary_source: None,
        });
    })
}

fn fixing(app: &App) -> Option<String> {
    let inspection = inspect_node(app, &round_key("t2", AgentRole::Worker, 1, 2));
    value(&inspection, "fixing").map(str::to_owned)
}

#[test]
fn fixing_names_a_failed_check() {
    let app = failed_check("compiling anthrex\nerror[E0308]: mismatched types\n", None);
    assert_eq!(
        fixing(&app).as_deref(),
        Some("check failed: error[E0308]: mismatched types")
    );
}

#[test]
fn fixing_prefers_the_decider_summary() {
    let app = failed_check(
        "compiling anthrex\nerror[E0308]: mismatched types",
        Some("type mismatch in status.rs:118\n…"),
    );
    assert_eq!(
        fixing(&app).as_deref(),
        Some("check failed: type mismatch in status.rs:118")
    );
    // A blank decider summary falls back to the raw tail.
    let app = failed_check("error: one", Some("  \n"));
    assert_eq!(fixing(&app).as_deref(), Some("check failed: error: one"));
}

#[test]
fn fixing_takes_the_latest_failure_before_the_round() {
    // The blocking review (ended at `now − 360`) is later than the check at `now − 400`.
    let app = with_task("t2", |task| {
        let check = task.last_check.as_mut().expect("check");
        check.ok = false;
        check.at = GEMINI_NOW - 400;
    });
    assert_eq!(
        fixing(&app).as_deref(),
        Some("status.rs:118 critical — SubagentStop not paired")
    );
    // A failure after the round started is not what it is fixing.
    let app = with_task("t2", |task| {
        task.reviews.clear();
        let proof = task.last_proof.as_mut().expect("proof");
        proof.ok = false;
        proof.at = GEMINI_NOW - 100;
    });
    assert_eq!(fixing(&app), None);
    let app = with_task("t2", |task| {
        task.reviews.clear();
        let proof = task.last_proof.as_mut().expect("proof");
        proof.ok = false;
        proof.at = GEMINI_NOW - 1000;
    });
    assert_eq!(fixing(&app).as_deref(), Some("test proof failed"));
}

#[test]
fn a_second_session_is_fixing_from_its_first_piece() {
    let app = with_task("t2", |task| {
        let mut second =
            crate::tree::run_fixtures::worker(2, None, proto::Runtime::Codex, GEMINI_NOW - 30);
        second.route = task.route.clone();
        task.rounds.push(second);
    });
    let inspection = inspect_node(&app, &round_key("t2", AgentRole::Worker, 2, 1));
    assert_eq!(
        value(&inspection, "fixing"),
        Some("status.rs:118 critical — SubagentStop not paired")
    );
    assert_eq!(value(&inspection, "session"), Some("no window yet"));
    assert_eq!(inspection.right.as_deref(), Some("starting · 30s · t2"));
}

#[test]
fn hostile_check_summaries_stay_one_bounded_line() {
    let long = format!("bad\u{7}\u{1b}[2J{}", "y".repeat(10_000));
    let app = failed_check("x", Some(&long));
    let text = fixing(&app).expect("fixing");
    assert!(!text.chars().any(char::is_control), "{text:?}");
    assert!(text.chars().count() <= 320, "{}", text.chars().count());
    assert!(text.starts_with("check failed: bad  [2Jyyy"), "{text}");
    // A summary with no lines at all.
    let app = failed_check("", None);
    assert_eq!(fixing(&app).as_deref(), Some("check failed"));
    let app = failed_check("\n\n  \n", Some(""));
    assert_eq!(fixing(&app).as_deref(), Some("check failed"));
}

/// `t7`'s Codex worker, rate-limited in the fixture, on a listed `Idle` window 11.
fn t7_on_window(change: impl FnOnce(&mut proto::AgentRoundInfo)) -> App {
    let (mut snapshot, mut windows) = gemini_fixture();
    let task = &mut snapshot.runs[0].tasks[7];
    task.rounds[0].window_id = Some(11);
    change(&mut task.rounds[0]);
    windows.push(headless(
        11,
        "r1/t7.w1",
        "/r/anthrex",
        Some(run_ref("r1", Some("t7"), AgentRole::Worker, 1)),
    ));
    app_of((snapshot, windows))
}

#[test]
fn a_rate_limited_round() {
    let app = t7_on_window(|_| {});
    let inspection = inspect_node(&app, &round_key("t7", AgentRole::Worker, 1, 1));
    assert_eq!(inspection.glyph.content, "◆");
    assert_eq!(inspection.right.as_deref(), Some("rate-limited · 15m · t7"));
    assert_eq!(
        pairs(&inspection),
        [
            ("doing", "rate-limited · waiting out the runtime's retry"),
            ("activity", "turns 0 · tool calls 0 · tokens 0"),
            (
                "session",
                "#11 · headless · worktree  · Enter: conversation"
            ),
        ]
    );
}

#[test]
fn an_expired_rate_limit_is_not_shown() {
    let app = t7_on_window(|round| {
        round.rate_limited = true;
        round.rate_limited_until = Some(GEMINI_NOW - 1);
        round.turn_open = true;
        round.open_subagents = 2;
        round.denials = 3;
    });
    let inspection = inspect_node(&app, &round_key("t7", AgentRole::Worker, 1, 1));
    assert_eq!(inspection.right.as_deref(), Some("idle · 15m · t7"));
    assert_eq!(
        value(&inspection, "doing"),
        Some("thinking · 2 sub-agents open")
    );
    assert_eq!(
        value(&inspection, "activity"),
        Some("turns 0 · tool calls 0 · tokens 0 · 3 denied")
    );
    let run = inspect_node(&app, &NodeKey::Run("r1".into()));
    assert_eq!(
        value(&run, "agents"),
        Some("workers 3/3 · readers 1/3 · claude ok · codex ok")
    );
    // Waiting between turns, and a limit with no recorded start.
    let app = t7_on_window(|round| round.rate_limited_until = None);
    let inspection = inspect_node(&app, &round_key("t7", AgentRole::Worker, 1, 1));
    assert_eq!(
        value(&inspection, "doing"),
        Some("waiting for its next turn")
    );
    let app = t7_on_window(|round| round.rate_limited_since = None);
    let run = inspect_node(&app, &NodeKey::Run("r1".into()));
    assert_eq!(
        value(&run, "agents"),
        Some("workers 3/3 · readers 1/3 · claude ok · codex rate-limited")
    );
}

#[test]
fn reviewer_round_fields_match_the_mockup() {
    let app = app_of(gemini_fixture());
    let inspection = inspect_node(&app, &round_key("t2", AgentRole::Reviewer, 1, 1));
    assert_eq!(inspection.glyph.content, "✗");
    assert_eq!(inspection.name, "review #1  claude · frontier · high");
    assert_eq!(inspection.right.as_deref(), Some("finished · 9m · t2"));
    assert_eq!(
        pairs(&inspection),
        [
            ("judging", "worker #1 · codex · standard"),
            ("strength", "frontier vs author standard"),
            ("verdict", "changes (blocking)"),
            (
                "findings",
                "1 critical, 2 minor · status.rs:118 critical — SubagentStop not paired"
            ),
            ("session", "#9 · window closed"),
        ]
    );
}

#[test]
fn a_live_reviewer_is_reviewing() {
    let app = app_of(gemini_fixture());
    let inspection = inspect_node(&app, &round_key("t2", AgentRole::Reviewer, 2, 2));
    assert_eq!(inspection.right.as_deref(), Some("starting · 1m · t2"));
    assert_eq!(
        pairs(&inspection),
        [
            ("judging", "worker #1 r2 · codex · standard"),
            ("strength", "frontier vs author standard"),
            ("verdict", "reviewing"),
            ("findings", "none"),
            ("session", "no window yet"),
        ]
    );
    // Listed as a terminal window, and ended without a verdict.
    let (mut snapshot, mut windows) = gemini_fixture();
    let round = &mut snapshot.runs[0].tasks[2].rounds[2];
    round.window_id = Some(12);
    round.ended_at = Some(GEMINI_NOW);
    let mut window = headless(12, "r1/t2.r2", "/r/anthrex", None);
    window.kind = proto::WindowKind::Pty;
    windows.push(window);
    let app = app_of((snapshot, windows));
    let inspection = inspect_node(&app, &round_key("t2", AgentRole::Reviewer, 2, 2));
    assert_eq!(inspection.glyph.content, "–");
    assert_eq!(inspection.right.as_deref(), Some("finished · 1m · t2"));
    assert_eq!(
        value(&inspection, "verdict"),
        Some("none — the round ended without one")
    );
    assert_eq!(
        value(&inspection, "session"),
        Some("#12 · terminal · read-only review worktree · Enter: conversation")
    );
}

#[test]
fn minor_only_changes_count_as_approval() {
    let app = with_task("t2", |task| {
        task.reviews[0].blocking = false;
        task.reviews[0].findings.remove(1);
    });
    let inspection = inspect_node(&app, &round_key("t2", AgentRole::Reviewer, 1, 1));
    assert_eq!(inspection.glyph.content, "✓");
    assert_eq!(
        value(&inspection, "verdict"),
        Some("changes (minor only, counts as approval)")
    );
    assert_eq!(
        value(&inspection, "findings"),
        Some("2 minor · hooks.rs:12 minor — naming")
    );
    let app = with_task("t2", |task| {
        task.reviews[0].verdict = Some(proto::Verdict::Approve);
        task.reviews[0].blocking = false;
        task.reviews[0].findings.clear();
    });
    let inspection = inspect_node(&app, &round_key("t2", AgentRole::Reviewer, 1, 1));
    assert_eq!(value(&inspection, "verdict"), Some("approve"));
    assert_eq!(value(&inspection, "findings"), Some("none"));
}

#[test]
fn only_the_latest_sessions_last_piece_of_a_blocked_task_is_a_cross() {
    let app = with_task("t2", |task| {
        task.state = TaskState::Blocked;
        let first = &mut task.rounds[0];
        first.ended_at = Some(GEMINI_NOW - 200);
        let mut second = first.clone();
        second.session = 2;
        second.round = 2;
        second.started_at = GEMINI_NOW - 150;
        second.sent_back_at = vec![GEMINI_NOW - 100];
        second.ended_at = Some(GEMINI_NOW - 50);
        task.rounds.push(second);
    });
    let glyph = |session, round| {
        let key = round_key("t2", AgentRole::Worker, session, round);
        inspect_node(&app, &key).glyph.content.into_owned()
    };
    assert_eq!(glyph(1, 2), "✓");
    assert_eq!(glyph(2, 1), "✓");
    assert_eq!(glyph(2, 2), "✗");
}

#[test]
fn a_failure_before_the_first_session_starts_is_not_what_it_fixes() {
    let app = with_task("t2", |task| {
        let proof = task.last_proof.as_mut().expect("proof");
        proof.ok = false;
        proof.at = GEMINI_NOW - 2000;
    });
    let inspection = inspect_node(&app, &round_key("t2", AgentRole::Worker, 1, 1));
    assert_eq!(value(&inspection, "fixing"), None);
}

#[test]
fn a_blocking_review_with_only_minor_findings_names_nothing() {
    // Hostile: the daemon sets `blocking` only for a non-minor finding.
    let app = with_task("t2", |task| {
        task.reviews[0].findings.remove(1);
    });
    assert_eq!(fixing(&app), None);
}

#[test]
fn an_ended_piece_of_a_rate_limited_session_is_finished() {
    let app = with_task("t2", |task| {
        task.rounds[0].rate_limited_since = Some(GEMINI_NOW - 10);
        task.rounds[0].rate_limited_until = Some(GEMINI_NOW + 60);
    });
    let first = inspect_node(&app, &round_key("t2", AgentRole::Worker, 1, 1));
    assert_eq!(first.right.as_deref(), Some("finished · 20m · t2"));
    assert_eq!(value(&first, "doing"), Some("finished"));
    let second = inspect_node(&app, &round_key("t2", AgentRole::Worker, 1, 2));
    assert_eq!(second.right.as_deref(), Some("rate-limited · 6m · t2"));
}

#[test]
fn the_agents_row_ages_the_earliest_live_limit() {
    let app = with_task("t2", |task| {
        // A later limit on a live Codex round, and an earlier one on an ended round.
        task.rounds[0].rate_limited_since = Some(GEMINI_NOW - 60);
        task.rounds[0].rate_limited_until = Some(GEMINI_NOW + 60);
        let mut ended = task.rounds[0].clone();
        ended.session = 3;
        ended.round = 3;
        ended.ended_at = Some(GEMINI_NOW - 5);
        ended.rate_limited_since = Some(GEMINI_NOW - 3000);
        task.rounds.push(ended);
    });
    let run = inspect_node(&app, &NodeKey::Run("r1".into()));
    assert_eq!(
        value(&run, "agents"),
        Some("workers 3/3 · readers 1/3 · claude ok · codex rate-limited 4m")
    );
}
