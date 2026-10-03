//! Milestone 9.3 task M9.3.8: `run iterate`, `run start --continue` and `run status`'s
//! round lines (KG §7, design decision 31).

use std::path::Path;
use std::time::Duration;

use clap::Parser;
use clap::error::ErrorKind;
use daemon::run::orch::contract_rounds::{
    REQUEST_TOO_LONG, cancelled, ended, halted, no_orchestrator, not_settled, rounds_max,
    superseded,
};
use proto::{
    GOAL_MAX_CHARS, OrchestratorChoice, RoundInfo, RoundOrigin, RoundOutcome, RunReply, RunRequest,
    Runtime,
};

use super::super::status::{printable, run_block, tests::example};
use super::super::{
    RUN_REQUEST_TIMEOUT, RunCommand, adapt, dispatch, print_outcome, request_timeout, resolve_run,
};

#[derive(Parser, Debug)]
struct Cli {
    #[command(subcommand)]
    command: RunCommand,
}

fn parse(args: &[&str]) -> Result<RunCommand, ErrorKind> {
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    Cli::try_parse_from(all)
        .map(|cli| cli.command)
        .map_err(|e| e.kind())
}

/// A socket no daemon listens on: a command that reaches it fails to connect.
const NO_DAEMON: &str = "/nonexistent/anthrex-test/rounds.sock";

/// What `dispatch` makes of `args` with no daemon to reach.
async fn dispatched(args: &[&str]) -> String {
    let command = parse(args).expect("parses");
    match dispatch(command, Path::new(NO_DAEMON), None).await {
        Ok(()) => panic!("{args:?} succeeded with no daemon"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn iterate_takes_exactly_one_source() {
    for (args, text, file) in [
        (
            vec!["iterate", "3f9a", "also add b"],
            Some("also add b"),
            None,
        ),
        (vec!["iterate", "3f9a", "-"], Some("-"), None),
        (
            vec!["iterate", "3f9a", "--file", "req.md"],
            None,
            Some("req.md"),
        ),
    ] {
        match parse(&args) {
            Ok(RunCommand::Iterate {
                run,
                text: t,
                file: f,
            }) => {
                assert_eq!(run, "3f9a", "{args:?}");
                assert_eq!(t.as_deref(), text, "{args:?}");
                assert_eq!(f.as_deref(), file.map(Path::new), "{args:?}");
            }
            other => panic!("{args:?}: {other:?}"),
        }
    }
    assert_eq!(
        parse(&["iterate", "3f9a"]).unwrap_err(),
        ErrorKind::MissingRequiredArgument
    );
    assert_eq!(
        parse(&["iterate", "3f9a", "more", "--file", "req.md"]).unwrap_err(),
        ErrorKind::ArgumentConflict
    );
    assert_eq!(
        parse(&["iterate", "3f9a", "-", "--file", "req.md"]).unwrap_err(),
        ErrorKind::ArgumentConflict
    );
    assert_eq!(
        parse(&["iterate"]).unwrap_err(),
        ErrorKind::MissingRequiredArgument
    );
}

/// The CLI refuses a request the daemon would refuse for its length, with the daemon's
/// text, before it connects: with no daemon, the refusal is the length's, not the
/// connection's. One character under the cap (counted in characters, not bytes) goes on
/// to connect.
#[tokio::test]
async fn iterate_refuses_a_long_text_before_connecting() {
    let long = "x".repeat(GOAL_MAX_CHARS + 1);
    assert_eq!(
        dispatched(&["iterate", "3f9a", &long]).await,
        REQUEST_TOO_LONG
    );
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("request.md");
    std::fs::write(&file, &long).unwrap();
    let path = file.display().to_string();
    assert_eq!(
        dispatched(&["iterate", "3f9a", "--file", &path]).await,
        REQUEST_TOO_LONG
    );

    let at_cap = "é".repeat(GOAL_MAX_CHARS);
    let error = dispatched(&["iterate", "3f9a", &at_cap]).await;
    assert!(error.starts_with("cannot reach the daemon at "), "{error}");
    std::fs::write(&file, &at_cap).unwrap();
    let error = dispatched(&["iterate", "3f9a", "--file", &path]).await;
    assert!(error.starts_with("cannot reach the daemon at "), "{error}");

    let missing = dir.path().join("missing.md").display().to_string();
    let error = dispatched(&["iterate", "3f9a", "--file", &missing]).await;
    assert!(
        error.starts_with(&format!("cannot read {missing}: ")),
        "{error}"
    );
}

#[test]
fn continue_requires_goal_and_conflicts_with_plan_and_orchestrator() {
    assert_eq!(
        parse(&["start", "--continue", "3f9a"]).unwrap_err(),
        ErrorKind::MissingRequiredArgument
    );
    assert_eq!(
        parse(&["start", "--plan", "p.toml", "--continue", "3f9a"]).unwrap_err(),
        ErrorKind::ArgumentConflict
    );
    assert_eq!(
        parse(&[
            "start",
            "--goal",
            "next",
            "--continue",
            "3f9a",
            "--orchestrator",
            "codex"
        ])
        .unwrap_err(),
        ErrorKind::ArgumentConflict
    );
    // Decisions 12 and 22.4: a user's continue takes its own run flags, `--yes` too.
    match parse(&[
        "start",
        "--goal",
        "next",
        "--continue",
        "3f9a",
        "--yes",
        "--delivery",
        "pr",
    ]) {
        Ok(RunCommand::Start {
            goal,
            continue_from,
            yes,
            delivery,
            orchestrator,
            ..
        }) => {
            assert_eq!(goal.as_deref(), Some("next"));
            assert_eq!(continue_from.as_deref(), Some("3f9a"));
            assert!(yes);
            assert_eq!(delivery.as_deref(), Some("pr"));
            assert_eq!(orchestrator, None);
        }
        other => panic!("{other:?}"),
    }
    match parse(&["start", "--goal", "next"]) {
        Ok(RunCommand::Start { continue_from, .. }) => assert_eq!(continue_from, None),
        other => panic!("{other:?}"),
    }
}

/// Decision 31: `--continue <run>` sends `StartGoal { continue_from: Some(<resolved run
/// id>) }` with no orchestrator, under the goal's reply bound; a goal without it sends
/// `None`.
#[test]
fn continue_sends_continue_from() {
    let runs: Vec<proto::RunInfo> = ["add-reset-3f9a", "add-login-77b0"]
        .iter()
        .map(|id| proto::RunInfo {
            run_id: id.to_string(),
            ..example()
        })
        .collect();
    let resolved = resolve_run(&runs, "3f9a").unwrap();
    let request = adapt::goal_request(
        "next".into(),
        "/r".into(),
        (true, true, false),
        (None, Some(proto::DeliveryMode::Pr)),
        Some(resolved),
    );
    assert_eq!(
        request,
        RunRequest::StartGoal {
            goal: "next".into(),
            dir: "/r".into(),
            yes: true,
            trust_project: true,
            unconfined_checks: false,
            orchestrator: None,
            delivery: Some(proto::DeliveryMode::Pr),
            continue_from: Some("add-reset-3f9a".into()),
        }
    );
    // The final fix wave (B-I1): a continue waits the daemon's own deadline, not
    // triage's (changed expectation).
    assert_eq!(request_timeout(&request), adapt::CONTINUE_REQUEST_TIMEOUT);

    let choice = OrchestratorChoice {
        runtime: Runtime::Codex,
        model: None,
    };
    let fresh = adapt::goal_request(
        "g".into(),
        "/r".into(),
        (false, false, false),
        (Some(choice.clone()), None),
        None,
    );
    assert_eq!(
        fresh,
        RunRequest::StartGoal {
            goal: "g".into(),
            dir: "/r".into(),
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            orchestrator: Some(choice),
            delivery: None,
            continue_from: None,
        }
    );
}

/// The final fix wave (B-I1): `--continue` outwaits the daemon's deadline on a
/// continued start (`CONTINUE_START_BOUND`), which has `run start`'s terms; the daemon
/// refuses past it, so the CLI never times out on a run the daemon then starts.
#[test]
fn a_continue_outwaits_the_daemons_continue_deadline() {
    let bound = daemon::run::chain::CONTINUE_START_BOUND;
    assert_eq!(bound, super::super::RUN_START_TIMEOUT, "run start's terms");
    assert!(adapt::CONTINUE_REQUEST_TIMEOUT > bound);
    assert_eq!(
        adapt::CONTINUE_REQUEST_TIMEOUT,
        bound + Duration::from_secs(30)
    );
}

/// The final fix wave (B-M6): `--continue` refuses a goal over the cap before it
/// connects, with the daemon's text; a goal at the cap, and any goal without
/// `--continue` (triage judges a new goal), pass.
#[test]
fn a_continue_caps_its_goal_before_connecting() {
    let over = "x".repeat(GOAL_MAX_CHARS + 1);
    let error = adapt::continue_checked(&over, true).unwrap_err();
    assert_eq!(
        error.to_string(),
        daemon::run::orch::contract_rounds::GOAL_TOO_LONG
    );
    assert!(adapt::continue_checked(&"é".repeat(GOAL_MAX_CHARS), true).is_ok());
    assert!(adapt::continue_checked(&over, false).is_ok());
}

/// Pinning: a round is one engine step (the brief's "Timing"), so `run iterate` waits as
/// every other run request does.
#[test]
fn request_timeout_of_iterate_is_the_run_request_timeout() {
    let iterate = RunRequest::Iterate {
        run: "add-reset-3f9a".into(),
        goal: "also add b".into(),
    };
    assert_eq!(request_timeout(&iterate), RUN_REQUEST_TIMEOUT);
}

/// Pinning: every refusal of decision 9 (and D17's), as the daemon words it, is the
/// command's error as it is, and `printable` (what `main` prints) leaves it unchanged.
#[test]
fn iterate_prints_the_daemons_refusals() {
    for text in [
        no_orchestrator("3f9a"),
        rounds_max("3f9a"),
        halted("3f9a"),
        ended("3f9a", "accepted"),
        cancelled("3f9a"),
        not_settled("3f9a", "running"),
        not_settled("3f9a", "planning"),
        superseded("3f9a"),
        REQUEST_TOO_LONG.to_string(),
    ] {
        let reply = RunReply::Refused {
            request: proto::run_wire::request::ITERATE.into(),
            message: text.clone(),
            request_id: None,
        };
        let error = print_outcome(reply).unwrap_err().to_string();
        assert_eq!(error, text);
        assert_eq!(printable(&error), text);
    }
}

fn info(
    n: u32,
    origin: RoundOrigin,
    outcome: Option<RoundOutcome>,
    summary: Option<&str>,
) -> RoundInfo {
    RoundInfo {
        n,
        goal_head: format!("goal of round {n}"),
        origin,
        outcome,
        summary_head: summary.map(str::to_string),
    }
}

/// The lines of `text` that `run status` prints for rounds.
fn round_lines(text: &str) -> Vec<&str> {
    text.lines().filter(|l| l.starts_with("  round ")).collect()
}

/// KG §7: a run with more than one round gets `round <n> of <total>` and one line a
/// round after its goal; a one-round run's block is unchanged (`status_single_golden.txt`
/// is pinned by `status_prints_stage_lines_for_a_multi_run_only`).
#[test]
fn status_shows_rounds_only_for_a_multi_round_run() {
    let one = example();
    assert!(one.rounds.is_empty());
    assert_eq!(round_lines(&run_block(&one, 0)), Vec::<&str>::new());
    // A run that has round 1's record (every run made since milestone 9.3) is still a
    // one-round run: no round lines.
    let mut first = example();
    first.rounds = vec![info(1, RoundOrigin::User, None, None)];
    assert_eq!(round_lines(&run_block(&first, 0)), Vec::<&str>::new());

    let mut three = example();
    three.round = 3;
    three.rounds = vec![
        info(
            1,
            RoundOrigin::User,
            Some(RoundOutcome::Completed),
            Some("Added the reset form"),
        ),
        info(
            2,
            RoundOrigin::Orchestrator,
            Some(RoundOutcome::Rejected),
            None,
        ),
        info(3, RoundOrigin::User, None, None),
    ];
    let text = run_block(&three, 0);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[1..6],
        [
            "  goal: Add password reset",
            "  round 3 of 3",
            "  round 1 · user · completed · Added the reset form",
            "  round 2 · orchestrator · rejected · -",
            "  round 3 · user · running · -",
        ]
    );
    // Nothing else of the block moved.
    let without: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.starts_with("  round "))
        .collect();
    assert_eq!(without, run_block(&one, 0).lines().collect::<Vec<_>>());

    let mut cancelled = three.clone();
    cancelled.round = 2;
    cancelled.rounds.truncate(2);
    cancelled.rounds[1].outcome = Some(RoundOutcome::Cancelled);
    assert_eq!(
        round_lines(&run_block(&cancelled, 0)),
        [
            "  round 2 of 2",
            "  round 1 · user · completed · Added the reset form",
            "  round 2 · orchestrator · cancelled · -",
        ]
    );
}

/// Decision 33's hostile-text rule for the CLI: a summary head with visible carriers (a
/// zero-width joiner, a bidi override), a line break and an escape stays on its one line,
/// without them, whether or not `printable` runs after. Mutant: the `one_line` call on
/// the summary head removed, red (the line break splits the line).
#[test]
fn status_round_lines_are_sanitised() {
    let mut run = example();
    run.round = 2;
    run.rounds = vec![
        info(1, RoundOrigin::User, Some(RoundOutcome::Completed), None),
        info(
            2,
            RoundOrigin::User,
            Some(RoundOutcome::Completed),
            Some("sum x\u{200D}y\u{202E}z\nw\x1b[2J"),
        ),
    ];
    let expected = [
        "  round 2 of 2",
        "  round 1 · user · completed · -",
        "  round 2 · user · completed · sum xyz w [2J",
    ];
    let text = run_block(&run, 0);
    assert_eq!(round_lines(&text), expected);
    assert_eq!(round_lines(&printable(&text)), expected);
}
