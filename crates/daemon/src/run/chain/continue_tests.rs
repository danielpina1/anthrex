//! Milestone 9.3 task 6b: the pure rules a next goal leans on. Which chain a goal
//! continuing from a run joins, and each refusal (decision 22, KG §3.3, §3.5); D16's
//! resolution to the newest run carrying a chain that left the table, and its
//! `start_goal` refused (6a re-review); an idle chain whose orchestrator never launched
//! is ended (6a review m2); a run's own suffix is taken (6a re-review N1).

use std::collections::BTreeMap;

use proto::{AgentRole, RunState, Runtime, ToolCall};
use serde_json::json;

use super::*;
use crate::run::orch::test_support::{orchestrator, run_of};

const PROJECT: &str = "/tmp/anthrex-chain-tests/project";
const WINDOW: u32 = 40;

fn run(id: &str, state: RunState, created: u64, chain: Option<&str>) -> Run {
    let mut run = run_of(1);
    run.id = id.into();
    run.project = PROJECT.into();
    run.state = state;
    run.created_at = created;
    let mut record = orchestrator();
    record.window_id = Some(WINDOW);
    run.orch.orchestrator = Some(record);
    run.chain = chain.map(String::from);
    run
}

fn runs(list: Vec<Run>) -> BTreeMap<String, Run> {
    list.into_iter().map(|r| (r.id.clone(), r)).collect()
}

fn chain(id: &str, runs: &[&str], state: ChainState) -> Chain {
    Chain {
        id: id.into(),
        project: PROJECT.into(),
        runs: runs.iter().map(|r| r.to_string()).collect(),
        window_id: WINDOW,
        runtime: Runtime::Claude,
        model: "claude-opus-5".into(),
        state,
        ended: false,
    }
}

fn table(chains: Vec<Chain>) -> BTreeMap<String, Chain> {
    chains.into_iter().map(|c| (c.id.clone(), c)).collect()
}

fn call(run_id: &str, chain: &str, tool: &str) -> ToolCall {
    ToolCall {
        run_id: run_id.into(),
        task_id: None,
        role: AgentRole::Orchestrator,
        window_id: WINDOW,
        tool: tool.into(),
        args: json!({}),
        scout_id: None,
        epic: None,
        chain: Some(chain.into()),
    }
}

#[test]
fn a_goal_continues_only_from_an_idle_chains_last_run() {
    let all = runs(vec![
        run("goal-one-3f9a", RunState::Accepted, 1, Some("o-3f9a")),
        run("goal-two-4c1d", RunState::Discarded, 2, Some("o-3f9a")),
        run("busy-5e5e", RunState::Running, 3, Some("o-5e5e")),
        run("plain-6f6f", RunState::Accepted, 4, None),
        run("gone-7a7a", RunState::Accepted, 5, Some("o-7a7a")),
    ]);
    let chains = table(vec![
        chain(
            "o-3f9a",
            &["goal-one-3f9a", "goal-two-4c1d"],
            ChainState::Idle,
        ),
        chain("o-5e5e", &["busy-5e5e"], ChainState::Active),
    ]);
    let (joined, last) = continuable(&chains, &all, "goal-two-4c1d").unwrap();
    assert_eq!(
        (joined.id.as_str(), last.id.as_str()),
        ("o-3f9a", "goal-two-4c1d")
    );

    let refused = |after: &str| continuable(&chains, &all, after).map(|_| ()).unwrap_err();
    assert_eq!(
        refused("goal-one-3f9a"),
        "run 3f9a is not the last run of o-3f9a; continue from run 4c1d"
    );
    assert_eq!(
        refused("busy-5e5e"),
        "run 5e5e is still going; finish it before starting another goal"
    );
    // No chain, and a chain that left the table.
    for (after, h4) in [("plain-6f6f", "6f6f"), ("gone-7a7a", "7a7a")] {
        assert_eq!(
            refused(after),
            format!(
                "run {h4} has no orchestrator to continue; start a new goal without --continue"
            ),
            "{after}"
        );
    }
    assert_eq!(refused("nowhere-0000"), "unknown run nowhere-0000");
}

/// 6a re-review, D16: once a chain has left the table, a window of it reads the newest
/// run carrying it, whichever run its MCP target names; its `start_goal` is refused.
#[test]
fn a_chain_that_left_the_table_resolves_to_its_newest_run() {
    let all = runs(vec![
        run("goal-one-3f9a", RunState::Accepted, 1, Some("o-3f9a")),
        run("goal-two-4c1d", RunState::Failed, 5, Some("o-3f9a")),
    ]);
    let chains = BTreeMap::new();
    for tool in ["run_status", "get_context", "edit_plan"] {
        let named = call("goal-one-3f9a", "o-3f9a", tool);
        assert_eq!(
            resolve(&chains, &all, &named),
            Ok(Some("goal-two-4c1d".to_string())),
            "{tool}"
        );
    }
    let text = "o-3f9a has ended; this window cannot start a goal, and the user starts the next one with a new orchestrator";
    let start = call("goal-one-3f9a", "o-3f9a", "start_goal");
    assert_eq!(resolve(&chains, &all, &start), Err(text.to_string()));
    // A run outside it is still refused, start_goal included.
    let outside = "this window is the orchestrator of o-3f9a; run x-0000 is not one of its runs";
    for tool in ["run_status", "start_goal"] {
        let named = call("x-0000", "o-3f9a", tool);
        assert_eq!(resolve(&chains, &all, &named), Err(outside.to_string()));
    }
}

/// 6a review m2: an idle chain whose orchestrator never launched (window 0) is ended,
/// so the snapshot never publishes window 0 and a continue launches a fresh session.
#[test]
fn an_idle_chain_whose_orchestrator_never_launched_is_ended() {
    let mut never = chain("o-3f9a", &["goal-one-3f9a"], ChainState::Active);
    never.window_id = 0;
    let mut chains = table(vec![never]);
    make_idle(&mut chains, "o-3f9a");
    assert!(chains["o-3f9a"].ended);
    let all = runs(vec![run(
        "goal-one-3f9a",
        RunState::Accepted,
        1,
        Some("o-3f9a"),
    )]);
    let listed = idle_list(&chains, &all);
    assert_eq!(
        (listed[0].window_id, listed[0].fresh),
        (None, true),
        "{listed:?}"
    );
    // One that launched stays unended.
    let mut chains = table(vec![chain(
        "o-3f9a",
        &["goal-one-3f9a"],
        ChainState::Active,
    )]);
    make_idle(&mut chains, "o-3f9a");
    assert!(!chains["o-3f9a"].ended);
}

/// 6a re-review N1: a run's own suffix is taken, chained or not (a plain run may be
/// promoted later and start `o-<its suffix>`).
#[test]
fn a_runs_own_suffix_is_taken() {
    let all = runs(vec![run("plain-goal-3f9a", RunState::Running, 1, None)]);
    assert!(suffix_taken(&BTreeMap::new(), &all, "another-goal-3f9a"));
    assert!(!suffix_taken(&BTreeMap::new(), &all, "another-goal-4c1d"));
}

/// D17 at a restart: a delivered `pr` run's chain follows the rule for idle chains
/// (idle and ended, the project's newest only, by when the run completed: its last
/// `complete: ` entry, not a later one); a `pr` run still delivering, and a cancelled
/// one complete with its PRs open, keep theirs active.
#[test]
fn a_restart_treats_a_delivered_pr_runs_chain_as_idle() {
    use crate::run::model::LogEntry;
    let entry = |at: u64, text: &str| LogEntry {
        at,
        text: text.into(),
    };
    let pr = |mut r: Run, project: &str, log: Vec<LogEntry>| {
        r.delivery.mode = proto::DeliveryMode::Pr;
        r.project = project.into();
        r.log = log;
        r
    };
    let shared = "/tmp/anthrex-chain-tests/shared";
    // Completed at 3, a later entry at 10; the accepted run of its project ended at 5.
    let early = pr(
        run("goal-one-3f9a", RunState::Complete, 1, Some("o-3f9a")),
        shared,
        vec![
            entry(3, "complete: 1 merged, 0 cancelled"),
            entry(10, "a note"),
        ],
    );
    let mut accepted = run("goal-two-1b2c", RunState::Accepted, 2, Some("o-1b2c"));
    accepted.project = shared.into();
    accepted.log = vec![entry(5, "accepted: merged")];
    let alone = pr(
        run("goal-three-6a6a", RunState::Complete, 3, Some("o-6a6a")),
        "/tmp/anthrex-chain-tests/alone",
        vec![entry(4, "complete: 1 merged, 0 cancelled")],
    );
    let mut cancelled = pr(
        run("goal-four-4c1d", RunState::Complete, 4, Some("o-4c1d")),
        "/tmp/anthrex-chain-tests/other",
        Vec::new(),
    );
    cancelled.cancelled = true;
    let running = pr(
        run("goal-five-5e5e", RunState::Running, 5, Some("o-5e5e")),
        "/tmp/anthrex-chain-tests/third",
        Vec::new(),
    );
    let all = runs(vec![early, accepted, alone, cancelled, running]);
    let chains = rebuild(&all);
    let state = |id: &str| chains.get(id).map(|c| (c.state, c.ended));
    assert_eq!(state("o-1b2c"), Some((ChainState::Idle, true)));
    assert_eq!(state("o-3f9a"), None, "the older idle chain left the table");
    assert_eq!(state("o-6a6a"), Some((ChainState::Idle, true)));
    assert_eq!(state("o-4c1d"), Some((ChainState::Active, false)));
    assert_eq!(state("o-5e5e"), Some((ChainState::Active, false)));
}

/// D17: a delivered run's idle orchestrator may still call `edit_plan` (its summary,
/// decision 38, and an iterate, which makes the chain active again); an accepted run's
/// may not.
#[test]
fn a_delivered_runs_idle_orchestrator_may_still_edit_its_plan() {
    let idle = chain("o-3f9a", &["goal-one-3f9a"], ChainState::Idle);
    let mut delivered = run("goal-one-3f9a", RunState::Complete, 1, Some("o-3f9a"));
    delivered.delivery.mode = proto::DeliveryMode::Pr;
    assert_eq!(idle_refusal(&idle, &delivered, "edit_plan"), None);
    let refused = Some(
        "run 3f9a has ended; start a new goal with start_goal when the user gives you one"
            .to_string(),
    );
    assert_eq!(idle_refusal(&idle, &delivered, "task_result"), refused);
    let accepted = run("goal-one-3f9a", RunState::Accepted, 1, Some("o-3f9a"));
    assert_eq!(idle_refusal(&idle, &accepted, "edit_plan"), refused);
}
