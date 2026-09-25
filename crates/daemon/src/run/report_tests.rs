use proto::{
    AgentRole, DoneSignal, Finding, Route, Runtime, Severity, Strength, TokenUsage, Verdict,
};

use super::*;
use crate::run::model::{
    AgentRound, BaseMoved, CheckRecord, DoneClaim, FailedTurn, FallbackState, LogEntry,
    ProofRecord, ReviewRecord, StallState, TaskEvent,
};
use crate::run::test_support::{EXAMPLE_PLAN, run_ok, task};

fn base_run() -> Run {
    run_ok(EXAMPLE_PLAN)
}

/// A worker round with every field filled, its counters set from the caller.
fn round(role: AgentRole, session: u32, turns: u32, tool_calls: u32, denials: u32) -> AgentRound {
    AgentRound {
        role,
        session,
        round: 1,
        window_id: Some(7),
        route: Route {
            runtime: Runtime::Claude,
            model: "claude-sonnet-5".to_string(),
            strength: Strength::Standard,
            effort: proto::Effort::High,
        },
        launch_op: 1,
        session_id: Some("sess-1".to_string()),
        pid: Some(4242),
        ended: true,
        started_at: 1_000,
        ended_at: Some(1_500),
        turn_open: false,
        turns,
        turn_had_task_done: true,
        last_event: 1_500,
        tool_calls,
        rate_limited_until: None,
        in_retry_streak: false,
        open_subagents: Default::default(),
        denials,
        usage: TokenUsage {
            input: 10,
            output: 5,
            cache_read: 1,
            cache_write: 2,
        },
        deaths: 0,
        fallback: FallbackState::None,
        stall: StallState::Watching,
        failed_turn: FailedTurn::None,
        review_nudged: false,
        wrap_up_sent: false,
        retiring: false,
        delivery_failures: 0,
        delivery_retry_at: None,
        turn_denied: Vec::new(),
        excused_secs: 0,
        last_denial: None,
        fallback_waiting: false,
        carried: Vec::new(),
        failed_error: None,
        resume_op: None,
        count_op: None,
        count_failures: 0,
        count_retry_at: None,
        count_turn: 0,
        interrupted: false,
        relaunch: None,
        closed_pid: None,
        exited_pid: None,
    }
}

#[test]
fn format_utc_vectors() {
    assert_eq!(format_utc(0), "1970-01-01 00:00:00Z");
    assert_eq!(format_utc(951_782_400), "2000-02-29 00:00:00Z");
    assert_eq!(format_utc(1_789_123_456), "2026-09-11 10:44:16Z");
}

#[test]
fn report_has_every_section() {
    let mut run = base_run();
    run.approved_by = Some("user".to_string());
    {
        let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
        t.notes
            .push("size raised to M: no check command".to_string());
        t.proofs.push(ProofRecord {
            at: 1_600,
            test: "token::expires_after_one_hour".to_string(),
            red: "a".repeat(40),
            head: "b".repeat(40),
            red_failed: true,
            head_passed: true,
            matched: true,
            red_tail: "red tail".to_string(),
            head_tail: "head tail".to_string(),
        });
        t.checks.push(CheckRecord {
            at: 1_700,
            ok: true,
            code: Some(0),
            timed_out: false,
            tail: "running 3 tests\ntest result: ok".to_string(),
            secs: 12,
            on_candidate: false,
        });
        t.reviews.push(ReviewRecord {
            round: 1,
            route: t.route.clone(),
            base: "c".repeat(40),
            head: "d".repeat(40),
            verdict: Some(Verdict::Changes),
            summary: "needs a test for expiry".to_string(),
            findings: vec![Finding {
                severity: Severity::Important,
                file: Some("src/a.rs".to_string()),
                line: Some(12),
                input: None,
                text: "Missing expiry test".to_string(),
            }],
        });
        t.salvage_refs
            .push("refs/anthrex/salvage/add-password-reset-3f9a/t1/1".to_string());
        t.merged_without_approval = Some("user override: looked fine".to_string());
        t.history.push(TaskEvent {
            at: 1_800,
            text: "dispatched".to_string(),
        });
        t.merge_commit = Some("e".repeat(40));
        t.done = Some(DoneClaim {
            summary: "added the reset token model".to_string(),
            test: Some("token::expires_after_one_hour".to_string()),
            red: Some("a".repeat(40)),
            signal: DoneSignal::TaskDone,
        });
    }
    run.log.push(LogEntry {
        at: 1_900,
        text: "run started".to_string(),
    });

    let out = render(&run, 2_000);

    let markers = [
        "# anthrex run add-password-reset-3f9a",
        "Goal:",
        "State:",
        "Approved by:",
        "Base branch:",
        "Run branch:",
        "Check: cargo test --workspace",
        "Limits:",
        "## Tasks",
        "| t1 |",
        "## t1: Reset token model",
        "Notes:",
        "Route:",
        "Review route:",
        "Budget:",
        "Proof 1:",
        "Check 1:",
        "```",
        "Review round 1",
        "Important:",
        "Missing expiry test",
        "Salvage refs:",
        "merged without approval: user override: looked fine",
        "History:",
        "## Log",
        "run started",
    ];
    let mut from = 0usize;
    for marker in markers {
        let at = out[from..]
            .find(marker)
            .unwrap_or_else(|| panic!("missing {marker:?} after byte {from} in:\n{out}"));
        from += at + marker.len();
    }
}

#[test]
fn minor_findings_are_listed_even_when_approved() {
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.reviews.push(ReviewRecord {
        round: 1,
        route: t.route.clone(),
        base: "a".repeat(40),
        head: "b".repeat(40),
        verdict: Some(Verdict::Approve),
        summary: "looks good".to_string(),
        findings: vec![Finding {
            severity: Severity::Minor,
            file: Some("src/a.rs".to_string()),
            line: Some(3),
            input: None,
            text: "nit: rename this".to_string(),
        }],
    });
    let out = render(&run, 2_000);
    assert!(out.contains("verdict=approve"));
    assert!(out.contains("Minor:"));
    assert!(out.contains("nit: rename this"));
}

#[test]
fn turn_end_fallback_is_named() {
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.done = Some(DoneClaim {
        summary: "committed and stopped".to_string(),
        test: None,
        red: None,
        signal: DoneSignal::TurnEndFallback,
    });
    let out = render(&run, 2_000);
    assert!(out.contains("| turn-end fallback |"));
}

#[test]
fn halted_and_rebaselined_runs_say_so() {
    let mut run = base_run();
    run.state = RunState::Halted;
    run.halted_reason = Some("the run ref moved outside the engine".to_string());
    run.log.push(LogEntry {
        at: 1_000,
        text: "halted: the run ref moved outside the engine".to_string(),
    });
    run.log.push(LogEntry {
        at: 1_100,
        text: format!(
            "resumed with --rebaseline: base main at {}, run head {}",
            sha7(&"1".repeat(40)),
            sha7(&"2".repeat(40))
        ),
    });
    let out = render(&run, 2_000);
    assert!(out.contains("State: halted: the run ref moved outside the engine"));
    assert!(out.contains("resumed with --rebaseline"));
}

#[test]
fn base_moved_and_accept_conflict_are_reported() {
    let mut run = base_run();
    run.base_moved = Some(BaseMoved {
        from: "1".repeat(40),
        to: "2".repeat(40),
        commits: 3,
        seen_at: 1_500,
    });
    let files = vec!["src/a.rs".to_string(), "src/b.rs".to_string()];
    run.log.push(LogEntry {
        at: 1_600,
        text: crate::run::contract::accept_conflict_message(&run.id, &run.base_branch, 3, &files),
    });
    let out = render(&run, 2_000);
    let expected = format!(
        "base {} moved during the run: {}..{}, 3 commits, listed at accept",
        run.base_branch,
        sha7(&"1".repeat(40)),
        sha7(&"2".repeat(40)),
    );
    assert!(out.contains(&expected), "{out}");
    let log_pos = out.find("## Log").unwrap();
    let files_pos = out
        .find("accept conflicts with 3 commits")
        .expect("the aborted accept's files are logged");
    assert!(files_pos > log_pos, "the accept conflict must be in ## Log");
    assert!(out.contains("src/a.rs, src/b.rs"));
}

#[test]
fn usage_and_denials_are_reported() {
    let mut run = base_run();
    let t = run.tasks.iter_mut().find(|t| t.id() == "t1").unwrap();
    t.rounds.push(round(AgentRole::Worker, 1, 5, 12, 3));
    let out = render(&run, 2_000);
    assert!(out.contains("5 turns"));
    assert!(out.contains("12 tool calls"));
    // billable = input(10) + cache_write(2) + output(5) = 17
    assert!(out.contains("17 tokens"));
    assert!(out.contains("3 denials"));
}

#[test]
fn containment_is_reported_excluded_by_default() {
    let run = base_run();
    let out = render(&run, 2_000);
    assert!(out.contains("project settings: excluded"));
}

#[test]
fn containment_is_reported_trusted() {
    let mut run = base_run();
    run.trusted_project = vec![".claude/settings.json".to_string()];
    let out = render(&run, 2_000);
    assert!(out.contains("project settings trusted by --trust-project: .claude/settings.json"));
}

/// Ruling T23-C1: decision 53's Codex line, from the branch recorded at start; a run
/// from before the field records none and gets no line.
#[test]
fn containment_reports_the_codex_project_config_branch() {
    use crate::headless::argv::CodexProjectConfig;
    let mut run = base_run();
    for (branch, line) in [
        (
            CodexProjectConfig::NotLoaded,
            "codex project config: not loaded by this CLI\n",
        ),
        (
            CodexProjectConfig::Excluded,
            "codex project config: excluded\n",
        ),
        (
            CodexProjectConfig::Loaded,
            "codex project config: loaded (this Codex CLI cannot exclude it)\n",
        ),
    ] {
        run.codex_project_config = Some(branch);
        let out = render(&run, 2_000);
        assert!(out.contains(line), "{branch:?}: {out}");
        assert_eq!(out.matches("codex project config:").count(), 1, "{out}");
    }
    run.codex_project_config = None;
    assert!(!render(&run, 2_000).contains("codex project config"));

    // An older `run.json`, without the field, still loads.
    let mut json = serde_json::to_value(base_run()).unwrap();
    json.as_object_mut().unwrap().remove("codex_project_config");
    let old: Run = serde_json::from_value(json).unwrap();
    assert_eq!(old.codex_project_config, None);
}

#[test]
fn containment_is_reported_sandbox_off() {
    let mut run = base_run();
    run.limits.worker_sandbox = false;
    let out = render(&run, 2_000);
    assert!(out.contains("worker sandbox: off ([orchestrator] worker_sandbox = false)"));
    // Final fix batch F2 (review C, M4): what that opt-out also gives up.
    assert!(
        out.contains(
            "an unsandboxed worker can reach the daemon's socket, so nothing stops it approving its own task's review or accepting the run\n"
        ),
        "{out}"
    );
}

#[test]
fn containment_is_silent_on_sandbox_when_on() {
    let run = base_run();
    assert!(run.limits.worker_sandbox);
    let out = render(&run, 2_000);
    assert!(!out.contains("worker sandbox: off"));
}

#[test]
fn unverified_run_says_so() {
    let mut run = base_run();
    run.profile.check = None;
    run.unverified = true;
    let out = render(&run, 2_000);
    assert!(out.contains("no check command: this run is unverified"));
}

#[test]
fn task_lookup_helper_finds_t1() {
    let run = base_run();
    assert_eq!(task(&run, "t1").id(), "t1");
}

#[path = "report_tests_escaping.rs"]
mod escaping;
