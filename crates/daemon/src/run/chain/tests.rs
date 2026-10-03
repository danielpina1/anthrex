//! Milestone 9.3 task 6a: the chain table's pure rules (decisions 19–21): resolution of
//! a chained call, the idle orchestrator's tool limits, the table rebuilt at a restart,
//! one idle orchestrator per project, and the snapshot's idle list.

use std::collections::BTreeMap;
use std::path::PathBuf;

use proto::{AgentRole, IdleOrchestrator, RunState, Runtime, ToolCall};
use serde_json::json;

use super::*;
use crate::run::model::{LogEntry, Run};
use crate::run::orch::test_support::{orchestrator, run_of};

const PROJECT: &str = "/tmp/anthrex-chain-tests/project";
const WINDOW: u32 = 40;

/// A run `id` of `project` in `state`, created at `created`, its orchestrator in
/// window `WINDOW`, in `chain`.
fn run(id: &str, project: &str, state: RunState, created: u64, chain: &str) -> Run {
    let mut run = run_of(1);
    run.id = id.into();
    run.project = project.into();
    run.state = state;
    run.created_at = created;
    run.log = vec![LogEntry {
        at: created + 10,
        text: "ended".into(),
    }];
    let mut record = orchestrator();
    record.window_id = Some(WINDOW);
    run.orch.orchestrator = Some(record);
    run.chain = Some(chain.into());
    run
}

fn runs(list: Vec<Run>) -> BTreeMap<String, Run> {
    list.into_iter().map(|r| (r.id.clone(), r)).collect()
}

/// A chain of `runs`, the last its current one.
fn chain(id: &str, project: &str, runs: &[&str], state: ChainState) -> Chain {
    Chain {
        id: id.into(),
        project: project.into(),
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

/// An orchestrator's `tool` call naming `run_id`, from a window of `chain`.
fn call(run_id: &str, chain: Option<&str>, tool: &str) -> ToolCall {
    ToolCall {
        run_id: run_id.into(),
        task_id: None,
        role: AgentRole::Orchestrator,
        window_id: WINDOW,
        tool: tool.into(),
        args: json!({}),
        scout_id: None,
        epic: None,
        chain: chain.map(String::from),
    }
}

#[test]
fn chain_ids_name_the_first_run() {
    let first = run("goal-one-3f9a", PROJECT, RunState::Running, 1, "x");
    assert_eq!(chain_id(&first), "o-3f9a");
}

#[test]
fn resolve_moves_an_earlier_run_forward() {
    let chains = table(vec![chain(
        "o-3f9a",
        PROJECT,
        &["goal-one-3f9a", "goal-two-4c1d"],
        ChainState::Active,
    )]);
    let earlier = call("goal-one-3f9a", Some("o-3f9a"), "run_status");
    assert_eq!(
        resolve(&chains, &BTreeMap::new(), &earlier),
        Ok(Some("goal-two-4c1d".to_string()))
    );
    // The current run resolves to itself.
    let current = call("goal-two-4c1d", Some("o-3f9a"), "edit_plan");
    assert_eq!(
        resolve(&chains, &BTreeMap::new(), &current),
        Ok(Some("goal-two-4c1d".to_string()))
    );
}

#[test]
fn resolve_refuses_a_run_outside_the_chain() {
    let chains = table(vec![
        chain("o-3f9a", PROJECT, &["goal-one-3f9a"], ChainState::Active),
        chain("o-77aa", PROJECT, &["other-77aa"], ChainState::Active),
    ]);
    let outside = call("other-77aa", Some("o-3f9a"), "run_status");
    let text = "this window is the orchestrator of o-3f9a; run other-77aa is not one of its runs";
    assert_eq!(
        resolve(&chains, &BTreeMap::new(), &outside),
        Err(text.to_string())
    );
    // Never redirected: the call keeps the run it named.
    assert_eq!(outside.run_id, "other-77aa");
    // An unknown chain is refused with the same text.
    let unknown = call("goal-one-3f9a", Some("o-0000"), "run_status");
    assert_eq!(
        resolve(&chains, &BTreeMap::new(), &unknown),
        Err(
            "this window is the orchestrator of o-0000; run goal-one-3f9a is not one of its runs"
                .to_string()
        )
    );
}

#[test]
fn resolve_leaves_unchained_calls_alone() {
    let chains = table(vec![chain(
        "o-3f9a",
        PROJECT,
        &["goal-one-3f9a", "goal-two-4c1d"],
        ChainState::Active,
    )]);
    // No chain on the call.
    assert_eq!(
        resolve(
            &chains,
            &BTreeMap::new(),
            &call("goal-one-3f9a", None, "run_status")
        ),
        Ok(None)
    );
    // A chain on a role other than the orchestrator's.
    for role in [AgentRole::Planner, AgentRole::Worker] {
        let mut other = call("goal-one-3f9a", Some("o-3f9a"), "get_context");
        other.role = role;
        assert_eq!(
            resolve(&chains, &BTreeMap::new(), &other),
            Ok(None),
            "{role:?}"
        );
    }
}

/// D16 (decision 20 amended): a chain no longer in the table (its run failed, or it
/// was the project's older idle chain) reaches its own runs, the newest of them (task
/// 6b's ruling; here its only one); a run that does not carry the chain is still
/// refused.
#[test]
fn resolve_lets_a_chain_that_left_the_table_reach_its_own_run() {
    let chains = table(vec![chain(
        "o-4c1d",
        PROJECT,
        &["goal-two-4c1d"],
        ChainState::Idle,
    )]);
    let mut stranger = run("stranger-5e5e", PROJECT, RunState::Running, 3, "x");
    stranger.chain = None;
    let all = runs(vec![
        run("failed-3f9a", PROJECT, RunState::Failed, 1, "o-3f9a"),
        run("other-6f6f", PROJECT, RunState::Accepted, 2, "o-6f6f"),
        stranger,
    ]);
    let own = call("failed-3f9a", Some("o-3f9a"), "run_status");
    assert_eq!(
        resolve(&chains, &all, &own),
        Ok(Some("failed-3f9a".to_string()))
    );
    let refused = |run_id: &str| {
        format!("this window is the orchestrator of o-3f9a; run {run_id} is not one of its runs")
    };
    // A run of another chain, a run of none, and an unknown run.
    for run_id in ["other-6f6f", "stranger-5e5e", "nowhere-0000"] {
        let named = call(run_id, Some("o-3f9a"), "run_status");
        assert_eq!(
            resolve(&chains, &all, &named),
            Err(refused(run_id)),
            "{run_id}"
        );
    }
    // A chain still in the table refuses a run outside it, whatever the run carries.
    let mut carries = run("carries-7a7a", PROJECT, RunState::Running, 4, "o-4c1d");
    carries.chain = Some("o-4c1d".into());
    let all = runs(vec![carries]);
    let named = call("carries-7a7a", Some("o-4c1d"), "run_status");
    assert_eq!(
        resolve(&chains, &all, &named),
        Err(
            "this window is the orchestrator of o-4c1d; run carries-7a7a is not one of its runs"
                .to_string()
        )
    );
}

#[test]
fn idle_allows_only_reads_and_start_goal() {
    let last = run("goal-one-3f9a", PROJECT, RunState::Accepted, 1, "o-3f9a");
    let idle = chain("o-3f9a", PROJECT, &["goal-one-3f9a"], ChainState::Idle);
    let text = "run 3f9a has ended; start a new goal with start_goal when the user gives you one";
    for tool in ["get_context", "run_status", "start_goal"] {
        assert_eq!(idle_refusal(&idle, &last, tool, &json!({})), None, "{tool}");
    }
    for tool in [
        "spawn_scout",
        "spawn_subplanner",
        "edit_plan",
        "task_result",
        "submit_epic",
        "anything_else",
    ] {
        assert_eq!(
            idle_refusal(&idle, &last, tool, &json!({})),
            Some(text.to_string()),
            "{tool}"
        );
    }
    // An active chain has no idle limits.
    let active = chain("o-3f9a", PROJECT, &["goal-one-3f9a"], ChainState::Active);
    for tool in ["edit_plan", "spawn_scout", "task_result"] {
        assert_eq!(
            idle_refusal(&active, &last, tool, &json!({})),
            None,
            "{tool}"
        );
    }
}

#[test]
fn rebuild_marks_idle_chains_ended_and_keeps_active_ones() {
    let other = "/tmp/anthrex-chain-tests/other";
    let all = runs(vec![
        // An active chain of two runs: its current run is the later one.
        run("goal-one-3f9a", PROJECT, RunState::Accepted, 1, "o-3f9a"),
        run("goal-two-4c1d", PROJECT, RunState::Running, 5, "o-3f9a"),
        // An idle chain of another project.
        run("idle-run-5e5e", other, RunState::Discarded, 2, "o-5e5e"),
        // A failed chain: dropped.
        run(
            "failed-6f6f",
            "/tmp/anthrex-chain-tests/third",
            RunState::Failed,
            3,
            "o-6f6f",
        ),
    ]);
    let mut unchained = run("plain-7a7a", PROJECT, RunState::Running, 4, "x");
    unchained.chain = None;
    let mut all = all;
    all.insert(unchained.id.clone(), unchained);

    let chains = rebuild(&all);
    assert_eq!(
        chains.keys().collect::<Vec<_>>(),
        ["o-3f9a", "o-5e5e"],
        "{chains:#?}"
    );
    let active = &chains["o-3f9a"];
    assert_eq!(active.state, ChainState::Active);
    assert!(!active.ended);
    assert_eq!(active.runs, ["goal-one-3f9a", "goal-two-4c1d"]);
    assert_eq!(active.current(), "goal-two-4c1d");
    assert_eq!(active.project, PathBuf::from(PROJECT));
    assert_eq!(
        (active.window_id, active.runtime, active.model.as_str()),
        (WINDOW, Runtime::Claude, "claude-opus-5")
    );
    let idle = &chains["o-5e5e"];
    assert_eq!(idle.state, ChainState::Idle);
    assert!(idle.ended, "an idle chain's window died with the daemon");
}

#[test]
fn one_idle_orchestrator_per_project() {
    let mut chains = table(vec![
        chain("o-1111", PROJECT, &["older-1111"], ChainState::Idle),
        chain("o-2222", PROJECT, &["active-2222"], ChainState::Active),
        chain("o-3333", PROJECT, &["newer-3333"], ChainState::Active),
        chain(
            "o-4444",
            "/tmp/anthrex-chain-tests/other",
            &["away-4444"],
            ChainState::Idle,
        ),
    ]);
    make_idle(&mut chains, "o-3333");
    assert_eq!(
        chains.keys().collect::<Vec<_>>(),
        ["o-2222", "o-3333", "o-4444"],
        "the older idle chain of the project is dropped"
    );
    assert_eq!(chains["o-2222"].state, ChainState::Active, "untouched");
    assert_eq!(chains["o-3333"].state, ChainState::Idle);
    assert_eq!(
        chains["o-4444"].state,
        ChainState::Idle,
        "another project's"
    );

    // At a restart, the project's newer idle chain is the one kept.
    let all = runs(vec![
        run("older-1111", PROJECT, RunState::Accepted, 1, "o-1111"),
        run("newer-3333", PROJECT, RunState::Discarded, 3, "o-3333"),
        run("active-2222", PROJECT, RunState::Running, 2, "o-2222"),
    ]);
    let rebuilt = rebuild(&all);
    assert_eq!(
        rebuilt.keys().collect::<Vec<_>>(),
        ["o-2222", "o-3333"],
        "{rebuilt:#?}"
    );
}

/// Fix round 1, m3: at a restart the project's newer idle chain is the one whose last
/// run was accepted or discarded later, whatever that run logged after its end.
#[test]
fn rebuild_keeps_the_chain_that_ended_last() {
    let entry = |at: u64, text: &str| LogEntry {
        at,
        text: text.into(),
    };
    let mut early = run("early-1111", PROJECT, RunState::Accepted, 1, "o-1111");
    early.log = vec![
        entry(100, "accepted: merged into main"),
        entry(500, "a note logged after the run ended"),
    ];
    let mut late = run("late-2222", PROJECT, RunState::Discarded, 2, "o-2222");
    late.log = vec![entry(300, "discarded: discarded")];
    let rebuilt = rebuild(&runs(vec![early, late]));
    assert_eq!(
        rebuilt.keys().collect::<Vec<_>>(),
        ["o-2222"],
        "{rebuilt:#?}"
    );
}

#[test]
fn idle_list_counts_runs() {
    let all = runs(vec![
        run("goal-one-3f9a", PROJECT, RunState::Accepted, 1, "o-3f9a"),
        run("goal-two-4c1d", PROJECT, RunState::Discarded, 5, "o-3f9a"),
        run(
            "busy-5e5e",
            "/tmp/anthrex-chain-tests/other",
            RunState::Running,
            2,
            "o-5e5e",
        ),
    ]);
    let mut chains = table(vec![
        chain(
            "o-3f9a",
            PROJECT,
            &["goal-one-3f9a", "goal-two-4c1d"],
            ChainState::Idle,
        ),
        chain(
            "o-5e5e",
            "/tmp/anthrex-chain-tests/other",
            &["busy-5e5e"],
            ChainState::Active,
        ),
    ]);
    let expected = IdleOrchestrator {
        chain: "o-3f9a".into(),
        project: PROJECT.into(),
        after_run: "goal-two-4c1d".into(),
        outcome: RunState::Discarded,
        runtime: Runtime::Claude,
        model: "claude-opus-5".into(),
        window_id: Some(WINDOW),
        fresh: false,
        runs: 2,
    };
    assert_eq!(idle_list(&chains, &all), vec![expected.clone()]);
    // Once its window is gone, a fresh session, and no window.
    chains.get_mut("o-3f9a").unwrap().ended = true;
    let fresh = IdleOrchestrator {
        window_id: None,
        fresh: true,
        ..expected
    };
    assert_eq!(idle_list(&chains, &all), vec![fresh]);
}

/// The final fix wave (review A, M2): a chain's runs are in the order their continues
/// linked them (`Run.continued_by`), whatever their clocks say. Here the clock stepped
/// back: each continued run was created before the one it continues. `rebuild` keeps
/// the chain active on its last run, and `newest` names that run.
#[test]
fn a_chains_order_follows_its_continue_links_not_its_clock() {
    let mut first = run("goal-one-3f9a", PROJECT, RunState::Accepted, 300, "o-3f9a");
    first.continued_by = Some("goal-two-4c1d".into());
    let mut second = run("goal-two-4c1d", PROJECT, RunState::Accepted, 299, "o-3f9a");
    second.continued_by = Some("goal-three-5e5e".into());
    let third = run(
        "goal-three-5e5e",
        PROJECT,
        RunState::Planning,
        298,
        "o-3f9a",
    );
    let all = runs(vec![first, second, third]);
    let chains = rebuild(&all);
    let chain = &chains["o-3f9a"];
    assert_eq!(
        chain.runs,
        ["goal-one-3f9a", "goal-two-4c1d", "goal-three-5e5e"]
    );
    assert_eq!(chain.state, ChainState::Active);
    assert!(!chain.ended);
    let newest_id = |except| newest(&all, "o-3f9a", except).map(|r| r.id.clone());
    assert_eq!(newest_id(None).as_deref(), Some("goal-three-5e5e"));
    assert_eq!(
        newest_id(Some("goal-three-5e5e")).as_deref(),
        Some("goal-two-4c1d")
    );
}
