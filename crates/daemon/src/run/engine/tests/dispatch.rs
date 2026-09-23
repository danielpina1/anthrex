//! M8a.11: start, the plan gate, dependencies and dispatch.

use std::path::PathBuf;

use proto::{AgentRole, BlockReason, PlanEdit, RunState, Runtime, TaskState};

use super::fixture::*;
use crate::headless::McpTarget;
use crate::run::contract::{WORKER_CONTRACT, worker_prompt};
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};
use crate::run::role_launch::session_uuid;
use crate::run::validate::EditScope;

fn int_path() -> PathBuf {
    PathBuf::from(format!("{WT}/runs/{RUN_ID}/integration"))
}

pub(super) fn task_path(id: &str) -> PathBuf {
    PathBuf::from(format!("{WT}/runs/{RUN_ID}/{id}"))
}

pub(super) fn replies(effects: &[Effect]) -> Vec<Result<String, String>> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Reply { result, .. } => Some(result.clone()),
            _ => None,
        })
        .collect()
}

pub(super) fn edit(fx: &mut Fixture, edits: Vec<PlanEdit>) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Edit {
        reply,
        run_id: RUN_ID.into(),
        edits,
        scope: EditScope::Run,
    })
}

#[test]
fn start_creates_the_run_branch_before_approval() {
    let plan = plan_with(
        &profile_with("max_writers = 2"),
        &[
            task("t1", "S", "a", ""),
            task("t2", "S", "b", ""),
            task("t3", "S", "c", ""),
            task("t4", "S", "d", "deps = [\"t1\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    let effects = fx.start(false);
    assert_eq!(replies(&effects), vec![Ok(RUN_ID.to_string())]);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    let branches = ops_in(&effects, "CreateRunBranch");
    assert_eq!(branches.len(), 1, "{effects:#?}");
    match &branches[0].1 {
        OpKind::CreateRunBranch {
            root,
            branch,
            base_sha,
            path,
            setup,
            ..
        } => {
            assert_eq!(root, &PathBuf::from("/tmp/x"));
            assert_eq!(branch, &format!("anthrex/{RUN_ID}/integration"));
            assert_eq!(base_sha, BASE);
            assert_eq!(path, &int_path());
            assert_eq!(setup.as_deref(), Some("make deps"));
        }
        other => panic!("{other:?}"),
    }
    assert!(ops_in(&effects, "PrepareWorktree").is_empty());

    let effects = fx.done(branches[0].0, OpResult::Worktree { head: BASE.into() });
    assert!(
        effects.contains(&Effect::WatchWorktree { root: int_path() }),
        "{effects:#?}"
    );
    // The pre-warm: root tasks only (t4 depends on t1), in dispatch order (t1 has a
    // dependent, so a longer critical path), at most max_writers of them.
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t1", "t2"]);
    for (_, kind) in ops_in(&effects, "PrepareWorktree") {
        let OpKind::PrepareWorktree {
            from, setup, env, ..
        } = &kind
        else {
            unreachable!()
        };
        assert_eq!(from, BASE);
        assert_eq!(setup.as_deref(), Some("make deps"));
        let target = format!("{}/target", task_path(&op_task(&kind)).display());
        assert_eq!(env, &vec![("TARGET".to_string(), target)]);
    }
    assert!(fx.ops("CreateWindow").is_empty());
    fx.complete_prepares();
    assert!(
        fx.ops("CreateWindow").is_empty(),
        "no session before approval"
    );
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert!(fx.task("t1").prewarmed && fx.task("t2").prewarmed);
    assert_eq!(fx.task("t1").state, TaskState::Queued);
    assert_eq!(
        fx.ops("PrepareWorktree").len(),
        2,
        "a pre-warm is done once"
    );
}

#[test]
fn approve_starts_dispatch_and_yes_skips_the_gate() {
    // Approval of a pre-warmed Claude task: straight to its window.
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(false);
    fx.complete_prepares();
    let prepares = fx.ops("PrepareWorktree").len();
    let effects = fx.approve();
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(fx.run().approved_by.as_deref(), Some("user"));
    assert_eq!(
        fx.ops("PrepareWorktree").len(),
        prepares,
        "pre-warmed: reused"
    );
    let windows = ops_in(&effects, "CreateWindow");
    assert_eq!(windows.len(), 1, "{effects:#?}");
    let (op, kind) = &windows[0];
    let OpKind::CreateWindow {
        name,
        spec,
        session_uuid: uuid,
        first_turn,
        project,
        worktree,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(name, &format!("{H4}/t1.w1"));
    assert_eq!(spec.cwd, task_path("t1"));
    assert_eq!(worktree, &task_path("t1"));
    assert_eq!(project, &PathBuf::from("/tmp/p"));
    assert!(matches!(
        &spec.mcp,
        Some(McpTarget { role: AgentRole::Worker, task_id: Some(t), run_id }) if t == "t1" && run_id == RUN_ID
    ));
    let mut tools = vec![
        "mcp__anthrex__task_done".to_string(),
        "mcp__anthrex__task_blocked".to_string(),
    ];
    tools.extend(config::Orchestrator::default().worker_allowed_tools);
    assert_eq!(spec.allowed_tools, tools);
    assert_eq!(spec.instructions, WORKER_CONTRACT);
    assert_eq!(spec.runtime, Runtime::Claude);
    assert_eq!(uuid, &Some(session_uuid(RUN_ID, *op)));
    assert_eq!(first_turn, &worker_prompt(fx.run(), fx.task("t1")));
    assert_eq!(fx.task("t1").start_commit.as_deref(), Some(BASE));
    let round = fx.task("t1").rounds.last().unwrap();
    assert!(round.turn_open, "the first turn is open from dispatch");
    assert_eq!(round.launch_op, *op);

    // --yes on a Codex task: no gate, a fresh worktree, then the window.
    let codex = "[task.route]\nruntime = \"codex\"";
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", codex)]));
    fx.ready(true);
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(fx.run().approved_by.as_deref(), Some("--yes"));
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    assert!(
        fx.ops("CreateWindow").is_empty(),
        "the worktree comes first"
    );
    let effects = fx.complete_prepares();
    let windows = ops_in(&effects, "CreateWindow");
    assert_eq!(windows.len(), 1);
    let OpKind::CreateWindow {
        spec,
        session_uuid: uuid,
        ..
    } = &windows[0].1
    else {
        unreachable!()
    };
    assert_eq!(spec.runtime, Runtime::Codex);
    assert_eq!(uuid, &None);
    let common = PathBuf::from("/tmp/p/.git");
    assert_eq!(spec.codex_writable_roots, vec![common]);
    assert_ne!(spec.codex_writable_roots[0], PathBuf::from("/tmp/x/.git"));
    assert_ne!(spec.codex_writable_roots[0], PathBuf::from("/tmp/p"));
    assert!(fx.task("t1").rounds.last().unwrap().turn_open);
}

#[test]
fn reject_discards() {
    let plan = plan_with(
        PROFILE,
        &[
            task("t1", "S", "a", ""),
            task("t2", "S", "b", "deps = [\"t1\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(false);
    fx.complete_prepares();
    let reply = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let discards = ops_in(&effects, "Discard");
    assert_eq!(discards.len(), 1, "{effects:#?}");
    let OpKind::Discard {
        root,
        worktrees,
        branch_prefix,
    } = &discards[0].1
    else {
        unreachable!()
    };
    assert_eq!(root, &PathBuf::from("/tmp/x"));
    assert_eq!(branch_prefix, &format!("anthrex/{RUN_ID}/"));
    let salvage = |t: &str| format!("refs/anthrex/salvage/{RUN_ID}/{t}/1");
    assert_eq!(
        worktrees,
        &vec![
            (task_path("t1"), salvage("t1")),
            (task_path("t2"), salvage("t2")),
            (int_path(), salvage("integration")),
        ]
    );
    fx.done(
        discards[0].0,
        OpResult::Finished {
            outcome: "discarded".into(),
        },
    );
    assert_eq!(fx.run().state, RunState::Discarded);

    // Reject is the plan gate's; a running run is refused.
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    assert!(replies(&effects)[0].is_err(), "{effects:#?}");
    assert!(ops_in(&effects, "Discard").is_empty());
}

#[test]
fn awaiting_approval_survives_restore() {
    let run = build(
        &plan_with(PROFILE, &[task("t1", "S", "a", "")]),
        &config::Orchestrator::default(),
        false,
    );
    assert_eq!(run.state, RunState::AwaitingApproval);
    let mut fx = Fixture::new("");
    fx.next(EventKind::Restore {
        runs: vec![run],
        replay: vec![],
    });
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert_eq!(fx.run().paused_from, None);
}

#[test]
fn dependents_wait_for_merged_and_branch_from_the_run_head() {
    let plan = plan_with(
        PROFILE,
        &[
            task("t1", "S", "a", ""),
            task("t2", "S", "b", "deps = [\"t1\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    fx.launch_all();
    assert_eq!(fx.task("t1").state, TaskState::Working);
    for state in [TaskState::Proof, TaskState::Review, TaskState::MergeQueue] {
        fx.force("t1", state);
        assert_eq!(
            tasks_of(&fx.log, "PrepareWorktree"),
            vec!["t1"],
            "{state:?}"
        );
        assert_eq!(fx.task("t2").state, TaskState::Pending);
    }
    let head = "c1".repeat(20);
    let effects = fx.merge("t1", &head);
    let prepares = ops_in(&effects, "PrepareWorktree");
    assert_eq!(prepares.len(), 1, "{effects:#?}");
    let OpKind::PrepareWorktree { from, branch, .. } = &prepares[0].1 else {
        unreachable!()
    };
    assert_eq!(from, &head);
    assert_eq!(from, &fx.run().run_head);
    assert_eq!(branch, &format!("anthrex/{RUN_ID}/t2"));
    fx.complete_prepares();
    assert_eq!(fx.task("t2").start_commit.as_deref(), Some(head.as_str()));
}

#[test]
fn implicit_owns_deps_serialize_by_plan_order() {
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/a/**\"]", ""),
            task_toml("t2", "S", "[\"crates/a/src/**\"]", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    assert_eq!(fx.task("t2").implicit_deps, vec!["t1"]);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    fx.launch_all();
    fx.force("t1", TaskState::Review);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1"]);
    fx.merge("t1", &"c1".repeat(20));
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1", "t2"]);
}

/// M8a.6 ruling N5: no task is dispatched, resumed or returned to `working` while any
/// dependency is unfinished; a started task resumes only after the run head is merged
/// into its worktree.
#[test]
fn a_stale_prewarmed_branch_is_repointed_at_dispatch() {
    // t2 is a hub task, so it cannot start while t1 holds a writer slot.
    let plan = plan_with(
        &profile_with("max_writers = 2"),
        &[
            task("t1", "M", "a", "priority = 1"),
            task("t2", "M", "proto", ""),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(false);
    assert_eq!(tasks_of(&fx.log, "PrepareWorktree"), vec!["t1", "t2"]);
    fx.complete_prepares();
    assert!(fx.task("t2").prewarmed);
    fx.approve();
    assert_eq!(tasks_of(&fx.log, "CreateWindow"), vec!["t1"]);
    fx.complete_windows();
    let head = "c1".repeat(20);
    let effects = fx.merge("t1", &head);
    let prepares = ops_in(&effects, "PrepareWorktree");
    assert_eq!(prepares.len(), 1, "{effects:#?}");
    let OpKind::PrepareWorktree {
        from, setup, path, ..
    } = &prepares[0].1
    else {
        unreachable!()
    };
    assert_eq!(path, &task_path("t2"));
    assert_eq!(from, &head, "re-pointed at the run head");
    assert_eq!(
        setup.as_deref(),
        Some("make deps"),
        "setup runs again after a re-point (ruling T8-I4)"
    );
    assert!(ops_in(&effects, "CreateWindow").is_empty());
    let effects = fx.complete_prepares();
    assert_eq!(tasks_of(&effects, "CreateWindow"), vec!["t2"]);
    assert_eq!(fx.task("t2").start_commit.as_deref(), Some(head.as_str()));
}

#[test]
fn a_setup_failure_blocks_the_task_as_environment_without_starting_it() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(false);
    let (op, _) = fx.op("PrepareWorktree");
    fx.done(
        op,
        OpResult::SetupFailed {
            output: "make: *** no rule".into(),
        },
    );
    let task = fx.task("t1");
    assert_eq!(task.state, TaskState::Blocked);
    let block = task.block.as_ref().unwrap();
    assert_eq!(block.reason, BlockReason::Environment);
    assert!(block.text.contains("make: *** no rule"), "{}", block.text);
    assert_eq!(
        task.start_commit, None,
        "a task blocked in setup has not started"
    );
    assert!(task.worktree_live && !task.prewarmed);
    fx.approve();
    assert!(fx.ops("CreateWindow").is_empty());
}

#[test]
fn session_uuids_are_valid_and_distinct() {
    let mut seen = std::collections::BTreeSet::new();
    for op in 0..1000u64 {
        let uuid = session_uuid(RUN_ID, op);
        assert_eq!(uuid.len(), 36, "{uuid}");
        for (i, c) in uuid.chars().enumerate() {
            if [8, 13, 18, 23].contains(&i) {
                assert_eq!(c, '-', "{uuid}");
            } else {
                assert!(c.is_ascii_hexdigit() && !c.is_ascii_uppercase(), "{uuid}");
            }
        }
        assert_eq!(&uuid[14..15], "4", "version nibble: {uuid}");
        assert!("89ab".contains(&uuid[19..20]), "variant: {uuid}");
        assert_eq!(session_uuid(RUN_ID, op), uuid, "deterministic");
        seen.insert(uuid);
    }
    assert_eq!(seen.len(), 1000);
    assert_ne!(session_uuid(RUN_ID, 1), session_uuid("other-run-0000", 1));
}
