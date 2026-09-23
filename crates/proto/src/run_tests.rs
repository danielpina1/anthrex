use std::any::TypeId;
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;
use crate::conversation::Role;
use crate::messages::{ClientMsg, DaemonMsg};
use crate::run_info::{
    AgentRoundInfo, BaseMovedInfo, BlockInfo, ReviewInfo, RunInfo, RunsSnapshot, Spend, TaskInfo,
    TokenUsage,
};
use crate::run_wire;
use crate::run_wire::{RunReply, RunRequest, ToolCall};
use crate::types::Runtime;

/// Decision 7's plan-file example, restated verbatim from
/// `docs/milestones/M8a-orchestration-engine-core.md`. Every value differs from the
/// default it overrides, and no two same-typed limits share a value.
const BRIEF_PLAN: &str = r#"
goal = "Add password reset"
max_writers = 4
max_readers = 2
max_bounces = 3

[profile]
modules = ["crates/*"]
hub = ["crates/proto/**"]
source = ["crates/*/src/**"]
check = "cargo test --workspace"
check_timeout_secs = 1800
single_test = "cargo test --workspace -- --exact {test}"
test_passed = 'test {test} \.\.\. ok'
setup = "cargo fetch"
generated = ["Cargo.lock"]
protected = ["docs/agents/**"]
[profile.env]
CARGO_TARGET_DIR = "{worktree}/target"

[[task]]
id = "t1"
title = "Reset token model"
kind = "code"
size = "M"
interface_change = false
test_mode = "tdd"
test_mode_reason = ""
owns = ["crates/auth/src/token.rs"]
deps = []
priority = 0
brief = "..."
acceptance = ["..."]
test_to_write = "token::expires_after_one_hour"
epic = "auth"
scout_refs = []
[task.route]
runtime = "claude"
model = "claude-sonnet-5"
strength = "standard"
effort = "high"
[task.budget]
tool_calls = 120
minutes = 45
tokens = 3000000
"#;

#[test]
fn plan_parses_the_brief_example() {
    let plan: Plan = toml::from_str(BRIEF_PLAN).expect("the brief example must parse");
    assert_eq!(plan.tasks.len(), 1);
    let task = &plan.tasks[0];
    assert_eq!(task.size, Size::M);
    assert_eq!(task.route.strength, Some(Strength::Standard));
    assert_eq!(
        plan.profile.env.as_ref().unwrap()["CARGO_TARGET_DIR"],
        "{worktree}/target"
    );
}

#[test]
fn plan_rejects_unknown_fields_at_every_level() {
    let top_level = format!("{BRIEF_PLAN}\nbogus_top = 1\n");
    let err = toml::from_str::<Plan>(&top_level).unwrap_err();
    assert!(err.to_string().contains("bogus_top"), "{err}");

    let in_profile = BRIEF_PLAN.replacen("[profile]\n", "[profile]\nbogus_profile_key = 1\n", 1);
    let err = toml::from_str::<Plan>(&in_profile).unwrap_err();
    assert!(err.to_string().contains("bogus_profile_key"), "{err}");

    let in_task = BRIEF_PLAN.replacen("[[task]]\n", "[[task]]\nbogus_task_key = 1\n", 1);
    let err = toml::from_str::<Plan>(&in_task).unwrap_err();
    assert!(err.to_string().contains("bogus_task_key"), "{err}");

    let in_route = BRIEF_PLAN.replacen("[task.route]\n", "[task.route]\nbogus_route_key = 1\n", 1);
    let err = toml::from_str::<Plan>(&in_route).unwrap_err();
    assert!(err.to_string().contains("bogus_route_key"), "{err}");

    let in_budget = BRIEF_PLAN.replacen(
        "[task.budget]\n",
        "[task.budget]\nbogus_budget_key = 1\n",
        1,
    );
    let err = toml::from_str::<Plan>(&in_budget).unwrap_err();
    assert!(err.to_string().contains("bogus_budget_key"), "{err}");
}

#[test]
fn size_serializes_as_a_capital_letter() {
    assert_eq!(serde_json::to_string(&Size::S).unwrap(), "\"S\"");
    assert_eq!(serde_json::to_string(&Size::M).unwrap(), "\"M\"");
    assert_eq!(serde_json::to_string(&Size::L).unwrap(), "\"L\"");
}

#[test]
fn strength_and_effort_order() {
    assert!(Strength::Fast < Strength::Standard);
    assert!(Strength::Standard < Strength::Frontier);
    assert!(Effort::Low < Effort::Medium);
    assert!(Effort::Medium < Effort::High);
    assert_eq!(Effort::High.raised(), None);
    assert_eq!(Size::M.raised(), Size::L);
}

fn minimal_task(id: &str) -> PlanTask {
    PlanTask {
        id: id.into(),
        title: "A task".into(),
        epic: None,
        kind: TaskKind::Code,
        size: Size::S,
        interface_change: false,
        test_mode: Some(TestMode::Tdd),
        test_mode_reason: None,
        owns: vec!["crates/x/**".into()],
        deps: vec![],
        priority: 0,
        brief: "do the thing".into(),
        acceptance: vec!["it works".into()],
        test_to_write: None,
        scout_refs: vec![],
        route: RouteSpec::default(),
        budget: None,
    }
}

#[test]
fn plan_edits_parse_from_an_edit_file() {
    let edits = vec![
        PlanEdit::AddTask {
            task: minimal_task("t9"),
        },
        PlanEdit::SplitTask {
            task_id: "t1".into(),
            into: vec![minimal_task("t1a"), minimal_task("t1b")],
        },
        PlanEdit::CancelTask {
            task_id: "t2".into(),
        },
        PlanEdit::AmendTask {
            task_id: "t3".into(),
            brief: Some("a new brief".into()),
            acceptance: Some(vec!["new criterion".into()]),
            route: Some(RouteSpec {
                runtime: Some(Runtime::Codex),
                model: None,
                strength: Some(Strength::Frontier),
                effort: Some(Effort::High),
            }),
            test_mode: Some(TestMode::Check),
            test_mode_reason: Some("no single_test configured".into()),
            priority: Some(5),
            size: Some(Size::L),
        },
        PlanEdit::AddDep {
            task_id: "t4".into(),
            dep: "t3".into(),
        },
        PlanEdit::Answer {
            task_id: "t5".into(),
            text: "use the token module".into(),
        },
        PlanEdit::Pause,
        PlanEdit::Resume,
        PlanEdit::Finish,
    ];
    let file = EditFile { edits };
    let toml_string = toml::to_string(&file).expect("EditFile must serialize");
    let back: EditFile = toml::from_str(&toml_string).expect("EditFile must round trip");
    assert_eq!(back, file);

    let rejected = r#"
[[edit]]
op = "approve_task"
task_id = "t1"
"#;
    assert!(toml::from_str::<EditFile>(rejected).is_err());
}

#[test]
fn states_serialize_snake_case() {
    assert_eq!(
        serde_json::to_string(&TaskState::MergeQueue).unwrap(),
        "\"merge_queue\""
    );
    assert_eq!(
        serde_json::to_string(&RunState::AwaitingApproval).unwrap(),
        "\"awaiting_approval\""
    );
    assert_eq!(
        serde_json::to_string(&BlockReason::MisSized).unwrap(),
        "\"mis_sized\""
    );

    assert!(TaskState::Merged.is_finished());
    assert!(TaskState::Cancelled.is_finished());
    for other in [
        TaskState::Pending,
        TaskState::Queued,
        TaskState::Preparing,
        TaskState::Working,
        TaskState::Proof,
        TaskState::Check,
        TaskState::Review,
        TaskState::MergeQueue,
        TaskState::Blocked,
    ] {
        assert!(!other.is_finished(), "{other:?} must not be finished");
    }

    assert!(RunState::Accepted.is_terminal());
    assert!(RunState::Discarded.is_terminal());
    assert!(RunState::Failed.is_terminal());
    for other in [
        RunState::AwaitingApproval,
        RunState::Running,
        RunState::Paused,
        RunState::Halted,
        RunState::Complete,
    ] {
        assert!(!other.is_terminal(), "{other:?} must not be terminal");
    }
}

#[test]
fn agent_role_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&AgentRole::Orchestrator).unwrap(),
        "\"orchestrator\""
    );
    assert_eq!(
        serde_json::to_string(&AgentRole::Worker).unwrap(),
        "\"worker\""
    );
    assert_eq!(
        serde_json::to_string(&AgentRole::Reviewer).unwrap(),
        "\"reviewer\""
    );
}

#[test]
fn agent_role_does_not_shadow_the_conversation_role() {
    use crate::{AgentRole as RootAgentRole, Role as RootRole};
    assert_ne!(TypeId::of::<RootAgentRole>(), TypeId::of::<RootRole>());
    assert_eq!(
        serde_json::to_string(&AgentRole::Worker).unwrap(),
        "\"worker\""
    );
    // The M6.5 `Role` keeps its own wire form (lowercase), unaffected by AgentRole's
    // rename_all choice.
    assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
}

#[test]
fn token_usage_billable_excludes_cache_reads() {
    let usage = TokenUsage {
        input: 700,
        output: 600,
        cache_read: 5000,
        cache_write: 300,
    };
    assert_eq!(usage.billable(), 1600);
}

fn a_finding() -> Finding {
    Finding {
        severity: Severity::Important,
        file: Some("crates/auth/src/token.rs".into()),
        line: Some(42),
        input: Some("token::expires_after_one_hour".into()),
        text: "off by one on the expiry check".into(),
    }
}

fn a_route(runtime: Runtime, strength: Strength, effort: Effort, model: &str) -> Route {
    Route {
        runtime,
        model: model.into(),
        strength,
        effort,
    }
}

fn a_token_usage() -> TokenUsage {
    TokenUsage {
        input: 111,
        output: 222,
        cache_read: 4000,
        cache_write: 333,
    }
}

fn an_agent_round() -> AgentRoundInfo {
    AgentRoundInfo {
        role: AgentRole::Worker,
        session: 1,
        round: 2,
        window_id: Some(9),
        route: a_route(
            Runtime::Claude,
            Strength::Standard,
            Effort::High,
            "claude-sonnet-5",
        ),
        session_id: Some("claude-session-abc".into()),
        started_at: 1_700_000_100,
        ended_at: Some(1_700_000_400),
        tool_calls: 7,
        last_event: 1_700_000_390,
        turn_open: false,
        turns: 3,
        rate_limited: false,
        open_subagents: 1,
        denials: 2,
        usage: a_token_usage(),
    }
}

fn a_review() -> ReviewInfo {
    ReviewInfo {
        round: 1,
        route: a_route(Runtime::Codex, Strength::Frontier, Effort::Medium, ""),
        verdict: Some(Verdict::Changes),
        summary: "one blocking finding".into(),
        findings: vec![a_finding()],
        blocking: true,
    }
}

fn a_task_info() -> TaskInfo {
    TaskInfo {
        id: "t1".into(),
        title: "Reset token model".into(),
        epic: Some("auth".into()),
        kind: TaskKind::Code,
        size: Size::M,
        hub: false,
        test_mode: TestMode::Tdd,
        test_mode_reason: None,
        notes: vec!["size raised from S to M: owns spans 2 modules (rule 7.2.1)".into()],
        owns: vec!["crates/auth/src/token.rs".into()],
        deps: vec!["t0".into()],
        implicit_deps: vec!["t0a".into()],
        priority: 3,
        route: a_route(
            Runtime::Claude,
            Strength::Standard,
            Effort::High,
            "claude-sonnet-5",
        ),
        review_route: Some(a_route(
            Runtime::Codex,
            Strength::Frontier,
            Effort::Medium,
            "",
        )),
        budget: Budget {
            tool_calls: 120,
            minutes: 45,
            tokens: Some(3_000_000),
        },
        spent_session: Spend {
            tool_calls: 4,
            secs: 90,
            tokens: 1200,
        },
        spent_total: Spend {
            tool_calls: 11,
            secs: 400,
            tokens: 5600,
        },
        state: TaskState::Review,
        block: Some(BlockInfo {
            reason: BlockReason::Question,
            text: "waiting on an answer about the token TTL".into(),
        }),
        rung: 1,
        failures: 2,
        bounces: GateCounts {
            done: 1,
            proof: 0,
            check: 1,
            review: 2,
            merge: 0,
        },
        stalls: 1,
        budget_exceeded: 0,
        conflicts: 1,
        branch: "anthrex/run-a1b2/t1".into(),
        worktree: PathBuf::from("/tmp/wt/runs/run-a1b2/t1"),
        start_commit: Some("aaaa1111".into()),
        head: Some("bbbb2222".into()),
        test: Some("token::expires_after_one_hour".into()),
        red: Some("cccc3333".into()),
        done_signal: Some(DoneSignal::TaskDone),
        rounds: vec![an_agent_round()],
        reviews: vec![a_review()],
        last_check: None,
        last_proof: None,
        merge_commit: None,
        merged_without_approval: None,
        salvage_refs: vec!["refs/anthrex/salvage/run-a1b2/t1/1".into()],
        on_critical_path: true,
        wave: 2,
        history: vec!["09:14 review round 1 requested changes".into()],
    }
}

fn a_run_info() -> RunInfo {
    let mut rate_limits = BTreeMap::new();
    rate_limits.insert("claude".to_string(), 2u32);
    rate_limits.insert("codex".to_string(), 0u32);
    RunInfo {
        run_id: "run-a1b2".into(),
        goal: "Add password reset".into(),
        project: PathBuf::from("/tmp/p"),
        root: PathBuf::from("/tmp/x"),
        state: RunState::Running,
        paused_from: None,
        halted_reason: None,
        approved_by: Some("user".into()),
        base_branch: "main".into(),
        base_sha: "0000000000000000000000000000000000000a".into(),
        run_branch: "anthrex/run-a1b2/integration".into(),
        run_head: "1111111111111111111111111111111111111b".into(),
        base_moved: Some(BaseMovedInfo {
            from: "0000000000000000000000000000000000000a".into(),
            to: "2222222222222222222222222222222222222c".into(),
            commits: vec!["abc1234 alice: fix the thing".into()],
            total: 3,
        }),
        revision: 42,
        max_writers: 4,
        max_readers: 2,
        max_bounces: 3,
        writers_busy: 1,
        readers_busy: 0,
        unverified: false,
        worker_sandbox: true,
        trusted_project: vec![".codex/config.toml".into()],
        rate_limits,
        tasks: vec![a_task_info()],
        critical_path: vec!["t1".into()],
        attention: vec![
            "base main moved from 0000000 to 2222222 (3 new commits); accept will list them".into(),
        ],
        report_path: PathBuf::from("/tmp/data/runs/run-a1b2/REPORT.md"),
        outcome: None,
        created_at: 1_700_000_000,
    }
}

fn a_tool_call() -> ToolCall {
    ToolCall {
        run_id: "run-a1b2".into(),
        task_id: Some("t1".into()),
        role: AgentRole::Worker,
        window_id: 9,
        tool: "task_done".into(),
        args: serde_json::json!({"summary": "ok"}),
    }
}

#[test]
fn every_run_request_and_reply_round_trips() {
    let snapshot = RunsSnapshot {
        revision: 42,
        runs: vec![a_run_info()],
    };

    let requests = vec![
        RunRequest::Start {
            plan_toml: BRIEF_PLAN.into(),
            dir: PathBuf::from("/tmp/p"),
            yes: false,
            trust_project: true,
        },
        RunRequest::Approve {
            run_id: "run-a1b2".into(),
        },
        RunRequest::Reject {
            run_id: "run-a1b2".into(),
        },
        RunRequest::Edit {
            run_id: "run-a1b2".into(),
            edits: vec![PlanEdit::Pause],
        },
        RunRequest::Retry {
            run_id: "run-a1b2".into(),
            task_id: "t1".into(),
        },
        RunRequest::Override {
            run_id: "run-a1b2".into(),
            task_id: "t1".into(),
            reason: "accepted the spill by hand".into(),
        },
        RunRequest::Cancel {
            run_id: "run-a1b2".into(),
        },
        RunRequest::Resume {
            run_id: "run-a1b2".into(),
            rebaseline: true,
        },
        RunRequest::Finish {
            run_id: "run-a1b2".into(),
            action: FinishAction::Accept,
            confirm: Some("run-a1b2".into()),
        },
        RunRequest::List,
        RunRequest::Subscribe,
        RunRequest::Unsubscribe,
        RunRequest::Tool(a_tool_call()),
    ];
    for request in requests {
        let msg = ClientMsg::Run(request);
        let packed = rmp_serde::to_vec_named(&msg).unwrap();
        let back: ClientMsg = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(back, msg);
    }

    // A named field lands in the named field, not merely "the bytes came back the
    // same": `dir` and `task_id` are pinned explicitly, not only through `PartialEq`.
    let start = ClientMsg::Run(RunRequest::Start {
        plan_toml: BRIEF_PLAN.into(),
        dir: PathBuf::from("/tmp/p"),
        yes: false,
        trust_project: true,
    });
    let packed = rmp_serde::to_vec_named(&start).unwrap();
    let ClientMsg::Run(RunRequest::Start { dir, .. }) = rmp_serde::from_slice(&packed).unwrap()
    else {
        panic!("must decode back to RunRequest::Start");
    };
    assert_eq!(dir, PathBuf::from("/tmp/p"));

    let retry = ClientMsg::Run(RunRequest::Retry {
        run_id: "run-a1b2".into(),
        task_id: "t1".into(),
    });
    let packed = rmp_serde::to_vec_named(&retry).unwrap();
    let ClientMsg::Run(RunRequest::Retry { task_id, .. }) = rmp_serde::from_slice(&packed).unwrap()
    else {
        panic!("must decode back to RunRequest::Retry");
    };
    assert_eq!(task_id, "t1");

    let replies = vec![
        RunReply::Started {
            run_id: "run-a1b2".into(),
            state: RunState::AwaitingApproval,
        },
        RunReply::Done {
            request: run_wire::request::APPROVE.into(),
            message: "run approved".into(),
        },
        RunReply::Refused {
            request: run_wire::request::START.into(),
            message: "runs are not available yet".into(),
        },
        RunReply::ConfirmNeeded {
            run_id: "run-a1b2".into(),
            prompt: "accept onto the advanced base?".into(),
            base_moved: Some(BaseMovedInfo {
                from: "0000000000000000000000000000000000000a".into(),
                to: "2222222222222222222222222222222222222c".into(),
                commits: vec!["abc1234 alice: fix the thing".into()],
                total: 3,
            }),
        },
        RunReply::Snapshot(snapshot.clone()),
        RunReply::ToolResult {
            ok: true,
            text: "task marked done".into(),
        },
    ];
    for reply in replies {
        let msg = DaemonMsg::Run(reply);
        let packed = rmp_serde::to_vec_named(&msg).unwrap();
        let back: DaemonMsg = rmp_serde::from_slice(&packed).unwrap();
        assert_eq!(back, msg);
    }

    // Same discipline for the daemon side: `run_id` and the snapshot's nested
    // `TaskInfo::id` are checked by name after the round trip.
    let started = DaemonMsg::Run(RunReply::Started {
        run_id: "run-a1b2".into(),
        state: RunState::AwaitingApproval,
    });
    let packed = rmp_serde::to_vec_named(&started).unwrap();
    let DaemonMsg::Run(RunReply::Started { run_id, .. }) = rmp_serde::from_slice(&packed).unwrap()
    else {
        panic!("must decode back to RunReply::Started");
    };
    assert_eq!(run_id, "run-a1b2");

    let snapshot_msg = DaemonMsg::Run(RunReply::Snapshot(snapshot.clone()));
    let packed = rmp_serde::to_vec_named(&snapshot_msg).unwrap();
    let DaemonMsg::Run(RunReply::Snapshot(back_snapshot)) = rmp_serde::from_slice(&packed).unwrap()
    else {
        panic!("must decode back to RunReply::Snapshot");
    };
    assert_eq!(back_snapshot.runs[0].tasks[0].id, "t1");
}
